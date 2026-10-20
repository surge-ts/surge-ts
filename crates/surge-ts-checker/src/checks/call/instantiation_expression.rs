//! tsc's `checkExpressionWithTypeArguments`: `f<T>` written without an
//! argument list — an instantiation expression, in a value or in a `typeof`
//! query.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedType, TextSpan as SyntaxTextSpan};
use surge_ts_types::{FunctionType, Type};

use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::{FunctionSignatureInfo, SymbolTable};

/// An instantiation expression in a value position: its type arguments are
/// checked for what they name, the expression is evaluated as written, and
/// its type is judged by [`instantiation_expression_diagnostic`]. The value
/// keeps its signatures uninstantiated.
pub(crate) fn evaluate_instantiation_expression(
    expression: &ParsedExpression,
    expression_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    type_arguments_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
    super::check_untyped_call_type_arguments(type_arguments, ctx);
    ctx.symbols = saved_symbols;
    let result = evaluate_expression(expression, expression_span, symbols, ctx);
    if let InferredExpression::Known(ty) = &result {
        if let ParsedExpression::Identifier { name, .. } = expression
            && class_value_lost_its_constructor(name, ty, ctx)
        {
            return result;
        }
        // A declared function's own declarations say what each of its
        // signatures takes; its value type folds the overloads together.
        let declared = match expression {
            ParsedExpression::Identifier { name, .. } => symbols.get(name).and_then(|symbol| {
                symbol
                    .function_signature
                    .clone()
                    .filter(|_| symbol.ty == *ty)
            }),
            _ => None,
        };
        if let Some(diagnostic) = instantiation_expression_diagnostic(
            ty,
            declared.as_deref(),
            type_arguments.len(),
            type_arguments_span,
            &ctx.file_name,
        ) {
            ctx.push(diagnostic);
        }
        report_unsatisfied_instantiation_arguments(
            ty,
            declared.as_ref(),
            type_arguments,
            type_arguments_span,
            symbols,
            ctx,
        );
        if let Some(instantiated) = instantiated_expression_signature(ty, declared.as_deref(), type_arguments, ctx) {
            return InferredExpression::Known(instantiated);
        }
    }
    result
}

/// tsc's `getInstantiationExpressionType` for a value of one generic
/// signature that takes the written type arguments: that signature
/// instantiated with them (`make<string>` is `(value: string) => string[]`).
fn instantiated_expression_signature(
    ty: &Type,
    declared: Option<&FunctionSignatureInfo>,
    type_arguments: &[ParsedType],
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Function(function) = ty else { return None };
    if type_arguments.is_empty() {
        return None;
    }
    if function.overloads().is_some() {
        return instantiated_expression_overloads(function, declared?, type_arguments, ctx);
    }
    let member;
    let signature = match declared {
        Some(signature) => signature,
        None => {
            let declaration = function.declaration()?;
            member = declaration.downcast_ref::<super::DeclaredMemberSignature>()?;
            if !member.outer_type_arguments.is_empty() {
                return None;
            }
            member.signature.as_ref()
        }
    };
    if signature.overloaded
        || signature.construct_signatures.is_some()
        || !takes_type_arguments(signature, type_arguments.len())
        || signature.parameter_types.len() != function.parameters().len()
    {
        return None;
    }
    Some(Type::Function(instantiate_signature_for_expression(function, signature, type_arguments, ctx)))
}

fn takes_type_arguments(signature: &FunctionSignatureInfo, count: usize) -> bool {
    let required = signature.type_parameters.iter().filter(|parameter| parameter.default_type.is_none()).count();
    (required..=signature.type_parameters.len()).contains(&count)
}

fn instantiate_signature_for_expression(
    function: &FunctionType,
    signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    ctx: &mut CheckerContext,
) -> FunctionType {
    let reported = ctx.diagnostics.len();
    let substitution = super::explicit_type_argument_substitution(signature, type_arguments, ctx);
    let instantiated =
        super::instantiate_function_type_with_substitution(function, signature, &substitution, ctx).into_owned();
    ctx.diagnostics.truncate(reported);
    let names = signature.parameter_names.iter().map(|name| name.as_deref().map(Into::into)).collect();
    instantiated.with_parameter_names(names)
}

/// The overloads of a declared function that take the written type
/// arguments, each instantiated with them; the others are dropped.
fn instantiated_expression_overloads(
    function: &FunctionType,
    declared: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let mut members = Vec::new();
    function.push_overload_members(&mut members);
    let signatures: Vec<&FunctionSignatureInfo> =
        std::iter::once(declared).chain(declared.overload_alternatives.iter().map(|overload| overload.as_ref())).collect();
    if !declared.overloaded || declared.construct_signatures.is_some() || members.len() != signatures.len() {
        return None;
    }
    let mut instantiated = Vec::new();
    for (member, signature) in members.iter().zip(&signatures) {
        if signature.type_parameters.is_empty()
            || signature.parameter_types.len() != member.parameters().len()
            || !takes_type_arguments(signature, type_arguments.len())
        {
            continue;
        }
        instantiated.push(instantiate_signature_for_expression(member, signature, type_arguments, ctx));
    }
    // No signature takes the arguments (TS2635): tsc's result has none.
    if instantiated.is_empty() && signatures.iter().all(|signature| !takes_type_arguments(signature, type_arguments.len())) {
        return Some(Type::Object(surge_ts_types::ObjectType::new(surge_ts_types::PropertyMap::default(), None)));
    }
    let first = instantiated.first()?.clone();
    Some(Type::Function(if instantiated.len() > 1 { first.with_overloads(instantiated) } else { first }))
}

/// tsgo's `getInstantiationExpressionType`, for what it reports: TS2635 on
/// the expression's type when none of its signatures takes the written number
/// of type arguments, or, when some do, on the first union member that has
/// signatures of which none does. `declared` is the value's own function
/// declaration. A part whose signatures' type-parameter counts surge does not
/// know, or a type it did not model, leaves the expression unjudged.
pub(crate) fn instantiation_expression_diagnostic(
    ty: &Type,
    declared: Option<&FunctionSignatureInfo>,
    type_argument_count: usize,
    type_arguments_span: Option<SyntaxTextSpan>,
    file_name: &str,
) -> Option<Diagnostic> {
    // An empty list (`f<>`) is TS1099's, and every generic signature takes it.
    if type_argument_count == 0 {
        return None;
    }
    let mut scan = ApplicabilityScan::new(type_argument_count);
    match declared {
        Some(signature)
            if signature.construct_signatures.is_none() && matches!(ty, Type::Function(_)) =>
        {
            let mut arities = vec![super::declared_type_argument_arity(&signature.type_parameters)];
            if signature.overloaded {
                arities.extend(
                    signature
                        .overload_alternatives
                        .iter()
                        .map(|overload| super::declared_type_argument_arity(&overload.type_parameters)),
                );
            }
            let mut part = PartSignatures::default();
            scan.judge_arities(&arities, &mut part);
            scan.close(ty, part);
        }
        _ => scan.judge(ty),
    }
    let error_type = scan.error_type(ty)?;
    // tsgo spans the arguments themselves, inside the angle brackets.
    let span = type_arguments_span.map(|span| SyntaxTextSpan {
        start: span.start + 1,
        end: span.end.saturating_sub(1).max(span.start + 1),
    });
    Some(diagnostic_with_syntax_span(
        Diagnostic::ts2635(error_type.name(), file_name),
        span,
    ))
}

/// tsc's `checkTypeArguments` over the signatures an instantiation expression
/// applies to (`getInstantiatedSignatures`): for each one taking that many
/// type arguments, the first argument its constraint rejects is TS2344. A
/// type parameter written as an argument is related through its constraint
/// when that is primitive, the only constraints judged exactly enough here.
pub(crate) fn report_unsatisfied_instantiation_arguments(
    ty: &Type,
    declared: Option<&std::sync::Arc<FunctionSignatureInfo>>,
    type_arguments: &[ParsedType],
    type_arguments_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if type_arguments.is_empty() || !type_arguments.iter().all(|argument| is_plainly_written(argument, ctx)) {
        return;
    }
    let mut signatures = Vec::new();
    match declared {
        Some(signature) if signature.construct_signatures.is_none() && matches!(ty, Type::Function(_)) => {
            signatures.push(std::sync::Arc::clone(signature));
            if signature.overloaded {
                signatures.extend(signature.overload_alternatives.iter().cloned());
            }
        }
        _ => collect_declared_signatures(ty, &mut signatures),
    }
    let count = type_arguments.len();
    signatures.retain(|signature| {
        let (minimum, maximum) = super::declared_type_argument_arity(&signature.type_parameters);
        maximum > 0 && count >= minimum && count <= maximum
    });
    if signatures.is_empty() {
        return;
    }
    let mut displays = vec![None; count];
    let arguments: Vec<ParsedType> = type_arguments
        .iter()
        .zip(displays.iter_mut())
        .map(|(argument, display)| match primitive_constraint_of_type_parameter(argument, ctx) {
            Some((name, constraint)) => {
                *display = Some(name);
                constraint
            }
            None => argument.clone(),
        })
        .collect();
    let span = type_arguments_span.map(|span| SyntaxTextSpan {
        start: span.start + 1,
        end: span.start + 1,
    });
    for signature in signatures {
        if let Some((index, argument, constraint)) =
            super::unsatisfied_type_argument_at(&signature, &arguments, symbols, ctx)
        {
            let argument_display = displays[index]
                .clone()
                .unwrap_or_else(|| crate::checks::expr::source_display_name(&argument, &constraint));
            let span = match type_arguments.get(index) {
                Some(ParsedType::Named(named)) => named.span.or(span),
                _ => span,
            };
            ctx.push_utility_diagnostic_once(diagnostic_with_syntax_span(
                crate::checks::relation_chain::with_chain(
                    Diagnostic::ts2344(argument_display, constraint.name(), ctx.file_name.clone()),
                    &if displays[index].is_some() {
                        // A type parameter written as the argument relates
                        // through its constraint.
                        crate::checks::relation_chain::relation_chain(&argument, &constraint)
                    } else {
                        crate::checks::relation_chain::chain_beneath_head(&argument, &constraint)
                    },
                ),
                span,
            ));
        }
    }
}

/// An argument surge relates as exactly as tsc: a primitive, a literal, a
/// type parameter or a named interface or class, and unions, arrays and
/// tuples of them. One that computes is left alone: the type-test idiom
/// `Expect<Equal<A, B>>` rests on an identity surge does not evaluate exactly.
fn is_plainly_written(argument: &ParsedType, ctx: &CheckerContext) -> bool {
    match argument {
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Undefined
        | ParsedType::Null
        | ParsedType::Void
        | ParsedType::Never
        | ParsedType::UnknownKeyword
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_) => true,
        ParsedType::Union(members) | ParsedType::Tuple(members, ..) => {
            members.iter().all(|member| is_plainly_written(member, ctx))
        }
        ParsedType::Array(element) => is_plainly_written(element, ctx),
        ParsedType::Named(named) => {
            named.type_arguments.is_empty()
                && (ctx.type_parameter_in_scope(&named.name)
                    || ctx.type_parameter_has_constraint(&named.name)
                    || matches!(
                        ctx.lookup_type_declaration(&named.name),
                        Some(crate::symbols::TypeDeclarationInfo::Interface(_))
                    ))
        }
        _ => false,
    }
}

/// The written signatures behind a value's type: a declared function's, or
/// a type literal member's, which carry their type parameters as written.
fn collect_declared_signatures(ty: &Type, out: &mut Vec<std::sync::Arc<FunctionSignatureInfo>>) {
    let collect_group = |group: &FunctionType, out: &mut Vec<std::sync::Arc<FunctionSignatureInfo>>| {
        let mut members = Vec::new();
        group.push_overload_members(&mut members);
        for member in &members {
            let Some(declaration) = member.declaration_handle() else {
                continue;
            };
            if let Some(member) = declaration.downcast_ref::<super::DeclaredMemberSignature>() {
                if member.outer_type_arguments.is_empty() {
                    out.push(std::sync::Arc::clone(&member.signature));
                }
            } else if let Ok(signature) =
                std::sync::Arc::clone(declaration).downcast::<FunctionSignatureInfo>()
            {
                out.push(signature);
            }
        }
    };
    match ty {
        Type::Union(union) => {
            for member in union.types() {
                collect_declared_signatures(member, out);
            }
        }
        Type::Reference(_) => collect_declared_signatures(&ty.peeled(), out),
        Type::Function(function) => collect_group(function, out),
        Type::Object(object) if !object.synthetic_open_index && !object.is_intersection => {
            for group in [object.call_signature(), object.construct_signature()].into_iter().flatten() {
                collect_group(group, out);
            }
        }
        _ => {}
    }
}

/// A bare type parameter in scope whose written constraint resolves to a
/// primitive: its name, and that constraint to relate in its place.
fn primitive_constraint_of_type_parameter(argument: &ParsedType, ctx: &mut CheckerContext) -> Option<(String, ParsedType)> {
    let ParsedType::Named(named) = argument else {
        return None;
    };
    if !named.type_arguments.is_empty() {
        return None;
    }
    let constraint = ctx.type_parameter_constraint(&named.name)?.clone();
    let checkpoint = ctx.diagnostics().len();
    let resolved = crate::infer::map_parsed_type(constraint.clone(), ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    is_primitive(&resolved).then(|| (named.name.clone(), constraint))
}

fn is_primitive(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::Never
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_primitive),
        _ => false,
    }
}

/// Whether `name` is a class whose value surge holds without the construct
/// signatures tsgo instantiates: a generic class merged with a namespace is
/// its namespace object alone, and lacking signatures there says nothing.
pub(crate) fn class_value_lost_its_constructor(name: &str, ty: &Type, ctx: &CheckerContext) -> bool {
    matches!(
        ctx.lookup_type_declaration(name),
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) if info.is_class_instance
    ) && !matches!(ty.peeled(), Type::Object(object) if object.construct_signature().is_some())
}

/// What `getInstantiationExpressionType` gathers over the parts of the
/// expression's type.
struct ApplicabilityScan {
    type_argument_count: usize,
    some_applicable: bool,
    non_applicable: Option<Type>,
    undecided: bool,
}

/// One `getInstantiatedType` call's own flags: whether its part has
/// signatures, and whether one of them takes the type arguments.
#[derive(Default)]
struct PartSignatures {
    any: bool,
    applicable: bool,
}

impl ApplicabilityScan {
    fn new(type_argument_count: usize) -> Self {
        Self {
            type_argument_count,
            some_applicable: false,
            non_applicable: None,
            undecided: false,
        }
    }

    /// `getInstantiatedType`: the whole type, or one member of a union.
    fn judge(&mut self, ty: &Type) {
        let mut part = PartSignatures::default();
        self.judge_part(ty, &mut part);
        self.close(ty, part);
    }

    fn close(&mut self, ty: &Type, part: PartSignatures) {
        self.some_applicable |= part.applicable;
        if part.any && !part.applicable && self.non_applicable.is_none() {
            self.non_applicable = Some(ty.clone());
        }
    }

    /// `getInstantiatedTypePart`.
    fn judge_part(&mut self, ty: &Type, part: &mut PartSignatures) {
        match ty {
            Type::Union(union) => {
                for member in union.types() {
                    self.judge(member);
                }
            }
            Type::Reference(_) => self.judge_part(&ty.peeled(), part),
            Type::Function(function) => self.judge_group(function, part),
            Type::Object(object) => {
                // An open shape is one surge could not enumerate, and a merged
                // intersection folds its operands' signatures into one without
                // keeping them apart.
                if object.synthetic_open_index
                    || (object.is_intersection
                        && (object.call_signature().is_some() || object.construct_signature().is_some()))
                {
                    self.undecided = true;
                    return;
                }
                for group in [object.call_signature(), object.construct_signature()]
                    .into_iter()
                    .flatten()
                {
                    self.judge_group(group, part);
                }
            }
            // surge's sentinel and its permissive `any`, the error type, a type
            // variable whose constraint is not at hand, and a `never` surge may
            // have narrowed to: none says which signatures tsgo's type has.
            Type::Unknown | Type::Any | Type::ErrorType | Type::TypeParameter(_) | Type::Never => {
                self.undecided = true;
            }
            _ => {}
        }
    }

    fn judge_group(&mut self, group: &FunctionType, part: &mut PartSignatures) {
        let mut signatures = Vec::new();
        group.push_overload_members(&mut signatures);
        let mut arities = Vec::with_capacity(signatures.len());
        for signature in &signatures {
            let Some(arity) = super::signature_type_argument_arity(signature, false) else {
                self.undecided = true;
                return;
            };
            arities.push(arity);
        }
        self.judge_arities(&arities, part);
    }

    /// `hasCorrectTypeArgumentArity`, over the generic signatures only.
    fn judge_arities(&self, arities: &[(usize, usize)], part: &mut PartSignatures) {
        part.any |= !arities.is_empty();
        part.applicable |= arities.iter().any(|&(minimum, maximum)| {
            maximum > 0 && self.type_argument_count >= minimum && self.type_argument_count <= maximum
        });
    }

    fn error_type(&self, ty: &Type) -> Option<Type> {
        if self.undecided {
            return None;
        }
        if self.some_applicable {
            self.non_applicable.clone()
        } else {
            Some(ty.clone())
        }
    }
}
