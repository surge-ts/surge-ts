use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedVariableDeclaration};
use surge_ts_types::{Type, TypeCopyReason, is_assignable_to};

use super::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type_anchored};
use super::expr::{evaluate_expression, source_display_name, widen_type};
use crate::context::{CheckerContext, convert_span};
use crate::infer::{InferredExpression, map_parsed_type};
use crate::symbols::{SymbolInfo, SymbolInfoHandle, SymbolKind, SymbolTable, map_symbol_kind};

pub(crate) struct VariableCheckOptions {
    pub(crate) report_duplicate_let_const: bool,
    pub(crate) check_initializer: bool,
    /// Whether the declaration is being checked for its own diagnostics, so a
    /// type literal written as its annotation gets `checkTypeLiteral`'s index
    /// constraint check. Off where only the binding's value is collected.
    pub(crate) check_type_literal: bool,
}

pub(crate) fn check_variable_declaration(
    variable: ParsedVariableDeclaration,
    ctx: &mut CheckerContext,
) {
    let mut symbols = std::mem::take(&mut ctx.symbols);

    check_variable_declaration_with_symbols(
        variable,
        &mut symbols,
        ctx,
        VariableCheckOptions {
            report_duplicate_let_const: true,
            check_initializer: true,
            check_type_literal: true,
        },
    );

    ctx.symbols = symbols;
}

pub(crate) fn check_variable_declaration_with_symbols(
    variable: ParsedVariableDeclaration,
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
    options: VariableCheckOptions,
) -> Option<SymbolInfoHandle> {
    let variable_name = variable.name.clone();
    let variable_name_span = variable.name_span;
    let tracks_duplicates = options.report_duplicate_let_const
        && !variable.is_declare
        && matches!(
            map_symbol_kind(variable.kind),
            SymbolKind::Let | SymbolKind::Const
        );
    let is_duplicate = tracks_duplicates && symbols.contains_let_or_const(&variable_name);
    let enum_probe = (variable.declared_type.is_none()
        && matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Let | surge_ts_syntax::ParsedVariableKind::Var))
    .then(|| variable.initializer.clone().filter(may_read_enum_member))
    .flatten();

    let boolean_initializer = match variable.initializer.as_ref() {
        Some(ParsedExpression::BooleanLiteral(value))
            if matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Let | surge_ts_syntax::ParsedVariableKind::Var) =>
        {
            Some(*value)
        }
        _ => None,
    };
    let merged_enum_members = variable
        .is_enum_object
        .then(|| {
            ctx.module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get_shared(&variable_name))
                .or_else(|| symbols.get_shared(&variable_name))
        })
        .flatten();
    let symbol = check_variable_declaration_against_symbols(variable, symbols, ctx, options)?;
    let symbol = match merged_enum_members {
        Some(merged) => with_merged_namespace_members(symbol, &merged),
        None => symbol,
    };
    crate::semantic::record_declaration_type(variable_name_span, &symbol.ty, ctx);

    if is_duplicate {
        if let Some(first_span) = symbols.take_declaration_span(&variable_name) {
            let diagnostic = Diagnostic::ts2451(&variable_name, ctx.file_name.clone())
                .with_span(convert_span(first_span));
            ctx.push(diagnostic);
        }
    } else if tracks_duplicates {
        if let Some(span) = variable_name_span {
            symbols.record_declaration_span(&variable_name, span);
        }
    }

    // `getAssignmentReducedType` of the declaration by its literal initializer,
    // as a function body's declaration narrows (`boolean` is `true | false`).
    let literal_narrowed = boolean_initializer
        .filter(|_| matches!(symbol.ty, Type::Boolean))
        .map(Type::BooleanLiteral);
    let narrowed = literal_narrowed.or_else(|| {
        enum_probe.and_then(|initializer| {
            enum_member_initializer_narrowing(&initializer, &symbol.ty, symbols, ctx)
        })
    });
    match narrowed {
        Some(narrowed) => {
            let _ = symbols.insert_narrowed(
                variable_name,
                SymbolInfo {
                    ty: narrowed,
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
                symbol.ty.clone(),
            );
        }
        None => {
            symbols.insert_handle(variable_name, Arc::clone(&symbol));
        }
    }
    Some(symbol)
}

/// An enum merges with a same-named namespace into one symbol whose value
/// carries the namespace's exports beside the members. The binding passes
/// already bound that merged value (`apply_merging_namespace_value_members`);
/// the enum's own declaration re-binds the object without them, so the members
/// the merged value has and the enum object lacks are carried over.
fn with_merged_namespace_members(symbol: SymbolInfoHandle, merged: &SymbolInfo) -> SymbolInfoHandle {
    let (Type::Object(object), Type::Object(merged_object)) = (&symbol.ty, &merged.ty) else {
        return symbol;
    };
    if merged_object
        .properties
        .iter()
        .all(|(name, _)| object.properties.contains_key(name))
    {
        return symbol;
    }
    let mut extended = object.clone();
    let mut properties = (*object.properties).clone();
    for (name, property) in merged_object.properties.iter() {
        properties
            .entry(name.clone())
            .or_insert_with(|| property.clone());
    }
    extended.properties = Arc::new(properties);
    extended.property_map_id = None;
    Arc::new(SymbolInfo {
        ty: Type::Object(extended),
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    })
}

/// Reports an initializer whose type is not assignable to the annotation it
/// initializes, at `target_span` — the declared name.
pub(crate) fn report_initializer_mismatch(
    inferred_initializer_type: &Type,
    declared_type: &Type,
    target_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    report_initializer_mismatch_of(inferred_initializer_type, declared_type, false, target_span, ctx);
}

/// [`report_initializer_mismatch`] for an initializer read from a binding
/// declared `unknown`: the written keyword is a type of its own, assignable
/// only to `unknown`, `any` and what admits `null` and `undefined` alike,
/// where an `unknown` surge inferred is a gap it must not judge.
pub(crate) fn report_declared_unknown_initializer(
    declared_type: &Type,
    target_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    report_initializer_mismatch_of(&Type::GenuineUnknown, declared_type, true, target_span, ctx);
}

/// An object, array or function never fits a target made only of primitives
/// and literals, whatever surge left unmodelled among its members (an
/// instantiated `ZodString` whose `def` it could not resolve is still no `1`).
pub(crate) fn object_never_fits_primitive(source: &Type, target: &Type) -> bool {
    let source = source.peeled();
    let is_object = match &source {
        // A branded primitive (`string & { __brand }`) is still a primitive.
        Type::Object(object) if object.is_intersection => object
            .intersection_operands
            .as_deref()
            .is_some_and(|operands| {
                operands
                    .iter()
                    .all(|operand| matches!(operand.peeled(), Type::Object(_) | Type::Array(_) | Type::Tuple(_)))
            }),
        Type::Object(_) => true,
        Type::Array(_) | Type::Tuple(_) | Type::Function(_) => true,
        // An interface or class reference too deep to expand here still names
        // an object type.
        Type::Reference(reference) => {
            !reference.is_unique_symbol() && reference.enum_owner.is_none() && reference.written_tuple().is_none()
        }
        _ => false,
    };
    if !is_object {
        return false;
    }
    let primitive = |ty: &Type| {
        matches!(
            ty,
            Type::String
                | Type::Number
                | Type::BigInt
                | Type::Boolean
                | Type::Symbol
                | Type::StringLiteral(_)
                | Type::NumberLiteral(_)
                | Type::BooleanLiteral(_)
                | Type::Undefined
                | Type::Null
                | Type::Void
                | Type::Never
        )
    };
    match target.peeled() {
        Type::Union(union) => union.types().iter().all(primitive),
        other => primitive(&other),
    }
}

fn report_initializer_mismatch_of(
    inferred_initializer_type: &Type,
    declared_type: &Type,
    declared_unknown: bool,
    target_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    let definite_mismatch = crate::checks::assign::definite_primitive_member_mismatch(
        inferred_initializer_type,
        declared_type,
    );
    let never_fits = object_never_fits_primitive(inferred_initializer_type, declared_type);
    let source_unmodelled = !declared_unknown
        && !never_fits
        && (inferred_initializer_type.is_unmodelled()
            || (crate::checks::call::as_source(|| type_contains_unknown(inferred_initializer_type))
                && !crate::checks::assign::definite_type_variable_mismatch(inferred_initializer_type, declared_type))
            || crate::checks::call::is_open_instantiation(inferred_initializer_type))
        && !(crate::checks::call::is_open_instantiation(inferred_initializer_type)
            && crate::checks::call::definitely_mismatches(inferred_initializer_type, declared_type));
    if source_unmodelled
        || (type_contains_unknown(declared_type) && !definite_mismatch)
        || (!never_fits && is_assignable_to(inferred_initializer_type, declared_type))
    {
        return;
    }
    report_assignability_failure(inferred_initializer_type, declared_type, target_span, ctx);
}

/// The TS2322 family for a source known not to be assignable to `declared_type`,
/// rendered as an initializer mismatch is. For a source the checker built
/// itself, which the inference-gap guards above would misjudge.
pub(crate) fn report_assignability_failure(
    inferred_initializer_type: &Type,
    declared_type: &Type,
    target_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    let declared_type = &crate::checks::expr::reported_relation_target(
        inferred_initializer_type,
        declared_type,
    );
    let inferred_type_name = source_display_name(inferred_initializer_type, declared_type);
    let declared_type_name = declared_type.name();
    let (inferred_type_name, declared_type_name) = crate::checks::expr::disambiguated_pair(
        inferred_initializer_type,
        inferred_type_name,
        declared_type,
        declared_type_name,
        &ctx.file_name,
    );
    let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
        inferred_initializer_type,
        declared_type,
        &inferred_type_name,
        &declared_type_name,
        false,
        ctx.file_name.clone(),
    );
    ctx.push(match target_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// A write `f.x = …` tsc's binder declares on the function an annotated
/// `const f: T = () => …` initializes (`bindDeferredExpandoAssignment`,
/// `getInitializerSymbol`): the target, and the JSDoc type written on it.
#[derive(Debug, Clone)]
pub(crate) struct ExpandoInitializerMember {
    target: ParsedExpression,
    declared_type: Option<surge_ts_syntax::ParsedType>,
    value: ParsedExpression,
}

/// [`install_expando_initializer_members`] for a module's statements.
pub(crate) fn module_expando_initializer_members(
    statements: &[surge_ts_syntax::ParsedStatement],
    javascript: bool,
) -> std::collections::HashMap<usize, Vec<ExpandoInitializerMember>> {
    use surge_ts_syntax::{ParsedExportDeclaration, ParsedStatement};
    let declarations: Vec<&ParsedVariableDeclaration> = statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::VariableDeclaration(variable) => Some(variable.as_ref()),
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                    ParsedStatement::VariableDeclaration(variable) => Some(variable.as_ref()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .filter(|variable| is_expando_initializer(variable, javascript))
        .collect();
    if declarations.is_empty() {
        return std::collections::HashMap::new();
    }
    let mut writes: Vec<&surge_ts_syntax::ParsedMemberAssignment> = Vec::new();
    for statement in statements {
        match statement {
            ParsedStatement::MemberAssignment(assignment) => push_member_writes(assignment, &mut writes),
            ParsedStatement::If(if_statement) => {
                crate::modules::exports::collect_nested_member_assignments(&if_statement.then_body, &[], &mut writes);
                crate::modules::exports::collect_nested_member_assignments(&if_statement.else_body, &[], &mut writes);
            }
            ParsedStatement::Block(body) => {
                crate::modules::exports::collect_nested_member_assignments(body, &[], &mut writes)
            }
            _ => {}
        }
    }
    expando_members_by_declaration(&declarations, &writes)
}

/// The writes `body` makes to the annotated expando declarations it declares,
/// installed for its statements; returns the set to restore after them. tsc
/// looks a write's receiver up in the write's block and then its container
/// (`lookupEntity`), so writes in nested blocks count unless the block
/// declares the name itself.
pub(crate) fn install_expando_initializer_members(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    ctx: &mut CheckerContext,
) -> Option<Arc<std::collections::HashMap<usize, Vec<ExpandoInitializerMember>>>> {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    let declarations: Vec<&ParsedVariableDeclaration> = body
        .iter()
        .filter_map(|statement| match statement {
            Statement::VariableDeclaration(variable) => Some(variable.as_ref()),
            _ => None,
        })
        .filter(|variable| is_expando_initializer(variable, javascript))
        .collect();
    if declarations.is_empty() {
        return None;
    }
    let mut writes: Vec<&surge_ts_syntax::ParsedMemberAssignment> = Vec::new();
    for statement in body {
        match statement {
            Statement::MemberAssignment(assignment) => push_member_writes(assignment, &mut writes),
            Statement::If(if_statement) => {
                crate::modules::exports::collect_nested_member_assignments(&if_statement.then_body, &[], &mut writes);
                crate::modules::exports::collect_nested_member_assignments(&if_statement.else_body, &[], &mut writes);
            }
            Statement::Block(block) => {
                crate::modules::exports::collect_nested_member_assignments(block, &[], &mut writes)
            }
            _ => {}
        }
    }
    let members = expando_members_by_declaration(&declarations, &writes);
    if members.is_empty() {
        return None;
    }
    Some(std::mem::replace(&mut ctx.expando_initializer_members, Arc::new(members)))
}

/// `getInitializerSymbol`: a `const` (any variable in JavaScript) initialized
/// with a function expression or an arrow (`IsExpandoInitializer`). The binder
/// declares every write's member on it wherever the write sits, so the
/// variable has them from its declaration on.
fn is_expando_initializer(variable: &ParsedVariableDeclaration, javascript: bool) -> bool {
    variable.name_span.is_some()
        && !variable.from_binding_pattern
        && (javascript || matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Const))
        && matches!(variable.initializer, Some(ParsedExpression::ArrowFunction(_)))
}

/// `F.a = F.b = v` writes both members.
fn push_member_writes<'a>(
    assignment: &'a surge_ts_syntax::ParsedMemberAssignment,
    writes: &mut Vec<&'a surge_ts_syntax::ParsedMemberAssignment>,
) {
    let mut chained = Some(assignment);
    while let Some(assignment) = chained {
        writes.push(assignment);
        chained = match &assignment.value {
            ParsedExpression::MemberAssignment(inner) => Some(inner.as_ref()),
            _ => None,
        };
    }
}

fn expando_members_by_declaration(
    declarations: &[&ParsedVariableDeclaration],
    writes: &[&surge_ts_syntax::ParsedMemberAssignment],
) -> std::collections::HashMap<usize, Vec<ExpandoInitializerMember>> {
    let mut members = std::collections::HashMap::new();
    for declaration in declarations {
        let Some(name_span) = declaration.name_span else {
            continue;
        };
        let declared: Vec<ExpandoInitializerMember> = writes
            .iter()
            .filter(|write| {
                expando_write_receiver(write) == Some(declaration.name.as_str()) && !is_compound_write(write)
            })
            .map(|write| ExpandoInitializerMember {
                target: write.target.clone(),
                value: write.value.clone(),
                declared_type: match &write.value {
                    ParsedExpression::TypeAssertion {
                        ty,
                        annotation: true,
                        ..
                    } => Some(ty.clone()),
                    _ => None,
                },
            })
            .collect();
        if !declared.is_empty() {
            members.insert(name_span.start, declared);
        }
    }
    members
}

fn expando_write_receiver(write: &surge_ts_syntax::ParsedMemberAssignment) -> Option<&str> {
    match &write.target {
        ParsedExpression::PropertyAccess { object, .. } => match object.as_ref() {
            ParsedExpression::Identifier { name, .. } => Some(name.as_str()),
            _ => None,
        },
        ParsedExpression::IndexAccess { object_name, .. } => Some(object_name.as_str()),
        _ => None,
    }
}

/// `f.x op= v`: tsc binds only `=`. The parser folds the operator into the
/// value, whose left operand is then the target itself.
fn is_compound_write(write: &surge_ts_syntax::ParsedMemberAssignment) -> bool {
    let left_span = match &write.value {
        ParsedExpression::Logical { left_span, .. }
        | ParsedExpression::NullishCoalescing { left_span, .. }
        | ParsedExpression::Binary { left_span, .. } => *left_span,
        _ => return false,
    };
    left_span.is_some() && left_span == write.target_span
}

/// The type tsc gives the function an annotated expando declaration
/// initializes: `getTypeOfFuncClassEnumModule` resolves the function's
/// exports, the members its writes declare, beside its signature, and that is
/// what relates to the annotation. A member is typed by the JSDoc `@type` of
/// the first write that has one (`getWidenedTypeForAssignmentDeclaration`);
/// otherwise by its value, checked with the annotation's member as contextual
/// type (`getContextualTypeForAssignmentExpression`), which relates to that
/// member wherever the write itself does, so the member's type stands for it.
/// A member the annotation lacks is left out.
fn annotated_expando_initializer_type(
    name_start: usize,
    initializer_type: &Type,
    declared_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Function(function) = initializer_type else {
        return None;
    };
    let writes = ctx.expando_initializer_members.get(&name_start)?.clone();
    let mut named: Vec<(String, Option<&surge_ts_syntax::ParsedType>)> = Vec::new();
    for write in &writes {
        let name = match &write.target {
            ParsedExpression::PropertyAccess { property_name, .. } => property_name.clone(),
            ParsedExpression::IndexAccess { index, .. } => {
                match crate::modules::exports::element_access_member_name(index, symbols, ctx) {
                    Some(name) => name,
                    None => continue,
                }
            }
            _ => continue,
        };
        match named.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, written)) => {
                if written.is_none() {
                    *written = write.declared_type.as_ref();
                }
            }
            None => named.push((name, write.declared_type.as_ref())),
        }
    }
    let mut properties = surge_ts_types::PropertyMap::default();
    for (name, written) in named {
        let member_type = match written {
            Some(written) => {
                let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
                let resolved = map_parsed_type(written.clone(), ctx);
                ctx.symbols = saved_symbols;
                resolved
            }
            None => match declared_member_type(declared_type, &name) {
                Some(member_type) => member_type,
                None => continue,
            },
        };
        properties.insert(name.as_str().into(), surge_ts_types::ObjectProperty::required(member_type));
    }
    if properties.is_empty() {
        return None;
    }
    Some(Type::Object(
        crate::metrics::alloc_object_type(properties, None).with_call_signature(function.clone()),
    ))
}

/// The type of an unannotated expando declaration's function: its signature
/// with the members its writes declare (`getTypeOfFuncClassEnumModule`), each
/// typed by the first write's JSDoc `@type`, else by the union of the written
/// values (`getWidenedTypeForAssignmentDeclaration`).
fn unannotated_expando_initializer_type(
    name_start: usize,
    function: &surge_ts_types::FunctionType,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let writes = ctx.expando_initializer_members.get(&name_start)?.clone();
    let mut named: Vec<(String, Vec<&ExpandoInitializerMember>)> = Vec::new();
    for write in &writes {
        let name = match &write.target {
            ParsedExpression::PropertyAccess { property_name, .. } => property_name.clone(),
            ParsedExpression::IndexAccess { index, .. } => {
                match crate::modules::exports::element_access_member_name(index, symbols, ctx) {
                    Some(name) => name,
                    None => continue,
                }
            }
            _ => continue,
        };
        match named.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, members)) => members.push(write),
            None => named.push((name, vec![write])),
        }
    }
    let mut properties = surge_ts_types::PropertyMap::default();
    for (name, members) in named {
        let member_type = match members.iter().find_map(|member| member.declared_type.as_ref()) {
            Some(written) => {
                let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
                let resolved = map_parsed_type(written.clone(), ctx);
                ctx.symbols = saved_symbols;
                resolved
            }
            None => {
                let values: Option<Vec<Type>> = members
                    .iter()
                    .map(|member| {
                        crate::modules::exports::expando_member_type(
                            &crate::modules::exports::ExpandoValue::Assigned(&member.value),
                            symbols,
                            ctx,
                        )
                        .map(|(ty, _)| ty)
                    })
                    .collect();
                // `getWidenedType` of the declarations' union widens their
                // object literals together (`getWidenedTypeOfObjectLiteral`).
                match values {
                    Some(values) => normalize_object_literal_union(&surge_ts_types::union_type(values)),
                    None => continue,
                }
            }
        };
        properties.insert(name.as_str().into(), surge_ts_types::ObjectProperty::required(member_type));
    }
    if properties.is_empty() {
        return None;
    }
    Some(Type::Object(
        crate::metrics::alloc_object_type(properties, None).with_call_signature(function.clone()),
    ))
}

/// The type `ty` declares its property `name` with, optionality aside.
fn declared_member_type(ty: &Type, name: &str) -> Option<Type> {
    match ty {
        Type::Object(object) => object.get_property_type(name).cloned(),
        Type::Reference(reference) => declared_member_type(&reference.resolve(), name),
        _ => None,
    }
}

pub(crate) fn check_variable_declaration_against_symbols(
    variable: ParsedVariableDeclaration,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
    options: VariableCheckOptions,
) -> Option<SymbolInfoHandle> {
    // Resolve the declared type against the in-scope value symbols so a
    // `typeof <value>` annotation can see locals/globals. The variable checker
    // moves `ctx.symbols` out into `symbols`, so without this the typeof lookup
    // would run against an empty table and spuriously report TS2304.

    if options.check_initializer {
        check_computed_binding_key(&variable, symbols, ctx);
    }
    let variable_name = variable.name.clone();
    let variable_name_span = variable.name_span;
    let annotated = variable.declared_type.is_some();
    let has_initializer = variable.initializer.is_some();
    let redeclaration_candidate = !variable.is_declare;
    let auto_array = is_auto_array_candidate(&variable, ctx);
    let circular_initializer = is_circular_var_initializer(&variable, &ctx.file_name);
    // `this` in an instance member is the polymorphic `this` type, which surge
    // reads as the class itself.
    let polymorphic_this = |symbols: &SymbolTable| {
        symbols.get("this").is_some_and(|this| {
            matches!(this.ty, Type::Reference(_) | Type::Object(_)) && !this.ty.name().starts_with("typeof ")
        })
    };
    let initializer_is_this =
        matches!(variable.initializer, Some(ParsedExpression::This { .. })) && polymorphic_this(symbols);
    let annotated_this = matches!(
        &variable.declared_type,
        Some(surge_ts_syntax::ParsedType::Named(named)) if named.name == "this"
    ) && polymorphic_this(symbols);

    // A generic annotation is kept for call-site instantiation; a type-predicate
    // annotation is kept so `if (isFoo(x))` can narrow — neither is recoverable
    // from the resolved callable type alone.
    let declared_function_type = match &variable.declared_type {
        Some(surge_ts_syntax::ParsedType::Function(function_type))
            if !function_type.type_parameters.is_empty()
                || matches!(
                    function_type.return_type.as_ref(),
                    surge_ts_syntax::ParsedType::Predicate(_)
                ) =>
        {
            Some(function_type.clone())
        }
        Some(other) => lone_generic_call_signature(other),
        _ => None,
    };
    // The same through a non-generic alias, read in the alias's own file.
    let alias_call_signature = match &variable.declared_type {
        Some(surge_ts_syntax::ParsedType::Named(named))
            if named.type_arguments.is_empty() && !ctx.type_parameter_in_scope(&named.name) =>
        {
            match ctx.lookup_type_declaration(&named.name) {
                Some(crate::symbols::TypeDeclarationInfo::Alias(alias))
                    if alias.body.type_parameters.is_empty() && alias.enum_name.is_none() =>
                {
                    lone_generic_call_signature(&alias.body.ty).map(|signature| (signature, alias.file_name.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    };
    // An enum's object side carries its reverse mapping as a number index,
    // which tsc never relates to the members (an enum is no type literal).
    let is_enum_object = variable.is_enum_object;
    let declared_type = variable.declared_type.map(|declared_type| {
        let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
        let literal = match &declared_type {
            surge_ts_syntax::ParsedType::Object(literal) if options.check_type_literal && !is_enum_object => {
                Some(literal.clone())
            }
            _ => None,
        };
        let resolved = map_parsed_type(declared_type, ctx);
        ctx.symbols = saved_symbols;
        if let Some(literal) = literal {
            crate::program::check_type_literal_index_constraints(&literal, &resolved, ctx);
        }
        resolved
    });
    if options.check_initializer
        && let Some(annotated) = variable.annotated_pattern.as_deref()
    {
        let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
        let pattern_type = map_parsed_type(annotated.declared_type.clone(), ctx);
        ctx.symbols = saved_symbols;
        crate::checks::function::check_binding_pattern_defaults(
            &annotated.pattern,
            Some(&pattern_type),
            true,
            symbols,
            ctx,
        );
    }
    if options.check_initializer {
        for excess in &variable.pattern_excess_properties {
            crate::checks::function::report_pattern_excess_property(excess, None, ctx);
        }
    }

    let symbol_kind = map_symbol_kind(variable.kind);

    // A `declare const`/`declare let` is pre-registered as an ambient symbol
    // before this check runs, so the duplicate probe would always find the
    // declaration's own pre-registration and report a spurious redeclaration.
    // Ambient declarations do not conflict with themselves; skip the report.
    // A module-scoped `let`/`const` shadows an ambient global of the same name
    // (`let name = …` over the DOM `declare var name`) without conflict — tsc
    // only reports a redeclaration when both declarations share a scope. The
    // merged symbol table folds ambient globals in flat, so exclude a name that
    // is solely an ambient global here; a genuine same-scope redeclaration still
    // reports through the span-tracked path in `check_variable_declaration`.
    if options.report_duplicate_let_const
        && !variable.is_declare
        && matches!(symbol_kind, SymbolKind::Let | SymbolKind::Const)
        && symbols.contains_let_or_const(&variable.name)
        && ctx.ambient_global_symbols.get(&variable.name).is_none()
    {
        let diagnostic = Diagnostic::ts2451(&variable.name, ctx.file_name.clone());
        let diagnostic = match variable.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };

        ctx.push(diagnostic);
    }

    // Inside its own initializer an annotated binding already has its declared
    // type: tsc's `getTypeOfVariableOrParameterOrProperty` answers the
    // annotation whatever is being checked, so a deferred self-reference
    // (`const s: ZodType<T> = z.lazy(() => s)`) reads `ZodType<T>`. The caller
    // pre-binds the name to the sentinel for the initializer; swap in the
    // annotation resolved just above, so it is still resolved exactly once.
    let self_bound_symbols = declared_type
        .as_ref()
        .filter(|declared_type| !declared_type.is_unknown())
        .filter(|_| {
            options.check_initializer
                && variable.initializer.is_some()
                && symbols
                    .get(&variable_name)
                    .is_some_and(|symbol| matches!(symbol.ty, Type::Unknown))
        })
        .map(|declared_type| {
            let mut bound = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
            let _ = bound.insert(
                variable_name.clone(),
                SymbolInfo {
                    ty: declared_type.clone(),
                    kind: symbol_kind,
                    function_signature: None,
                },
            );
            bound
        });
    let initializer_symbols = self_bound_symbols.as_ref().unwrap_or(symbols);

    // tsc checks an array pattern's source once for iterability, and a
    // source without the protocol types every element `any`.
    let non_iterable_pattern_source = options.check_initializer
        && variable
            .array_pattern_span
            .zip(variable.initializer.as_ref().and_then(|initializer| {
                // A rest that is the pattern's only element is bound to the
                // source itself; an earlier element already checked it otherwise.
                array_pattern_source(initializer)
                    .or_else(|| (variable.array_rest_start == Some(0)).then(|| initializer.clone()))
            }))
            .is_some_and(|(pattern_span, source)| {
                let InferredExpression::Known(source_type) = crate::infer::infer_expression(&source, symbols, ctx)
                else {
                    return false;
                };
                if !crate::checks::expr::is_definitely_not_iterable(&source_type, true) {
                    crate::checks::expr::check_iteration_next_type(
                        &InferredExpression::Known(source_type),
                        Some(pattern_span),
                        &Type::Undefined,
                        crate::checks::expr::IterationSend::ArrayDestructuring,
                        false,
                        ctx,
                    );
                    return false;
                }
                let diagnostic = Diagnostic::ts2488(source_type.name(), ctx.file_name.clone())
                    .with_span(convert_span(pattern_span));
                ctx.push(diagnostic);
                true
            });

    let iterated_pattern_element = (options.check_initializer && !non_iterable_pattern_source)
        .then(|| {
            iterated_array_pattern_element(
                variable.array_pattern_span.is_some(),
                variable.array_rest_start.is_some(),
                variable.initializer.as_ref(),
                variable.initializer_span,
                symbols,
                ctx,
            )
        })
        .flatten();

    let mut allow_missing = Vec::new();
    if variable.from_binding_pattern
        && let Some(initializer) = &variable.initializer
    {
        defaulted_element_reads(initializer, &mut allow_missing);
    }
    let outer_allow_missing = std::mem::replace(&mut ctx.allow_missing_tuple_elements, allow_missing);
    // `const s: unique symbol = Symbol()` is what creates that unique symbol:
    // the call's `symbol` is this declaration's own type, not a mismatch.
    let initializer_target = declared_type.as_ref().filter(|declared_type| {
        !(matches!(
            declared_type,
            Type::Reference(reference)
                if reference.is_unique_symbol()
                    && reference.id.ends_with(&format!("\u{0}{}", variable.name))
        ) && variable
            .initializer
            .as_ref()
            .is_some_and(is_symbol_constructor_call))
    });
    let inferred_initializer = if non_iterable_pattern_source {
        InferredExpression::Known(Type::Any)
    } else if let Some(element) = iterated_pattern_element {
        InferredExpression::Known(element)
    } else if options.check_initializer {
        variable
            .initializer
            .as_ref()
            .map(|initializer| {
                if let Some(declared_type) = initializer_target {
                    evaluate_expression_with_expected_type_anchored(
                        initializer,
                        variable.initializer_span,
                        variable.name_span,
                        Some(declared_type),
                        ExpectedTypeDiagnostic::TypeNotAssignable,
                        initializer_symbols,
                        ctx,
                    )
                } else {
                    evaluate_expression(initializer, variable.initializer_span, symbols, ctx)
                }
            })
            .unwrap_or(InferredExpression::Unknown)
    } else {
        InferredExpression::Unknown
    };
    ctx.allow_missing_tuple_elements = outer_allow_missing;
    if options.check_initializer
        && let Some(await_using) = variable.declaration_list.as_ref().and_then(|list| list.disposal)
    {
        use crate::checks::function::body_statements::report_missing_global_type;
        if await_using {
            report_missing_global_type("AsyncDisposable", ctx);
        }
        report_missing_global_type("Disposable", ctx);
    }
    if options.check_initializer
        && let Some(await_using) = variable.declaration_list.as_ref().and_then(|list| list.disposal)
        && let InferredExpression::Known(initializer_type) = &inferred_initializer
    {
        report_undisposable_using_initializer(initializer_type, await_using, variable.initializer_span, ctx);
    }
    let mut inferred_symbol_type = match &inferred_initializer {
        InferredExpression::Known(inferred_initializer_type) => {
            if let Some(declared_type) = initializer_target {
                // A module scope records no declared types; its bindings hold
                // `GenuineUnknown` only when it is the real `unknown`.
                // A member or element read typed `unknown` reads a written
                // `unknown` too (or the `Record<K, unknown>` an `in` test
                // proves), as `GenuineUnknown` only ever is.
                let reads_declared_unknown = matches!(inferred_initializer_type, Type::GenuineUnknown)
                    && match variable.initializer.as_ref() {
                        Some(surge_ts_syntax::ParsedExpression::Identifier { name, .. }) => {
                            matches!(initializer_symbols.declared_type(name), Some(Type::GenuineUnknown))
                                || matches!(
                                    initializer_symbols.get(name).map(|symbol| &symbol.ty),
                                    Some(Type::GenuineUnknown)
                                )
                        }
                        Some(
                            surge_ts_syntax::ParsedExpression::PropertyAccess { .. }
                            | surge_ts_syntax::ParsedExpression::OptionalPropertyAccess { .. }
                            | surge_ts_syntax::ParsedExpression::ElementAccess { .. }
                            | surge_ts_syntax::ParsedExpression::IndexAccess { .. }
                            | surge_ts_syntax::ParsedExpression::OptionalIndexAccess { .. },
                        ) => true,
                        _ => false,
                    };
                if reads_declared_unknown {
                    report_declared_unknown_initializer(
                        declared_type,
                        variable.name_span.or(variable.initializer_span),
                        ctx,
                    );
                } else {
                    let expando_initializer_type = variable.name_span.and_then(|name_span| {
                        annotated_expando_initializer_type(
                            name_span.start,
                            inferred_initializer_type,
                            declared_type,
                            symbols,
                            ctx,
                        )
                    });
                    report_initializer_mismatch(
                        expando_initializer_type.as_ref().unwrap_or(inferred_initializer_type),
                        declared_type,
                        variable.name_span.or(variable.initializer_span),
                        ctx,
                    );
                }
            }

            // Only the check phase publishes an error-typed binding: during
            // analysis the initializer may be the error type because an import
            // it names is not bound yet, and an export typed from that would
            // reach every consumer.
            if declared_type.is_none() && matches!(inferred_initializer_type, Type::ErrorType) {
                Some(crate::infer::unresolved_name_error_type().unwrap_or(Type::Unknown))
            } else if declared_type.is_none()
                && (!inferred_initializer_type.is_unknown()
                    || matches!(inferred_initializer_type, Type::GenuineUnknown)
                    || inferred_initializer_type.is_type_variable())
                && let Some(initializer) = variable.initializer.as_ref()
            {
                let widened_call;
                let initializer_type = if matches!(symbol_kind, SymbolKind::Let | SymbolKind::Var)
                    && returns_body_inferred_type(initializer, symbols, ctx)
                {
                    widened_call = crate::checks::expr::widen_type(inferred_initializer_type);
                    &widened_call
                } else {
                    inferred_initializer_type
                };
                if reads_regular_literal(initializer, initializer_type, symbols) {
                    Some(initializer_type.clone())
                } else {
                    // `widenTypeInferredFromInitializer`: in JavaScript an
                    // empty array literal's type is `any[]` either way.
                    Some(widen_variable_declaration_initializer_type(
                        symbol_kind,
                        initializer,
                        initializer_type,
                        auto_array || surge_ts_syntax::is_javascript_file_name(&ctx.file_name),
                        ctx.options.no_implicit_any,
                    ))
                }
            } else {
                declared_type.clone().or(Some(Type::Unknown))
            }
        }
        // The binding of a failed lookup is tsc's error type too.
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. } => declared_type
            .clone()
            .or_else(|| inferred_initializer.clone().flowing_type())
            .or(Some(Type::Unknown)),
        InferredExpression::Unknown => declared_type.clone().or(Some(Type::Unknown)),
    };

    if declared_type.is_none() && variable.initializer.is_none() {
        inferred_symbol_type = Some(Type::Any);
    }
    // tsc's `getTypeOfVariableOrParameter` answers `any` (TS7022) when checking
    // the initializer of the symbol's first declaration needs its own type.
    let circular = circular_initializer
        && symbols
            .get_own(&variable_name)
            .is_none_or(|existing| existing.ty.is_unknown());
    if circular {
        inferred_symbol_type = Some(Type::Any);
        if let Some(name_span) = variable.name_span
            && !ctx.circular_var_starts.contains(&name_span.start)
        {
            ctx.circular_var_starts.push(name_span.start);
        }
        if options.check_initializer
            && ctx.options.no_implicit_any
            && let Some(name_span) = variable.name_span
        {
            ctx.push(Diagnostic::ts7022(&variable_name, ctx.file_name.clone()).with_span(convert_span(name_span)));
        }
    }
    if declared_type.is_none()
        && let Some(Type::Function(function)) = inferred_symbol_type.as_ref()
        && let Some(name_span) = variable.name_span
        && let Some(expando) = unannotated_expando_initializer_type(name_span.start, function, symbols, ctx)
    {
        inferred_symbol_type = Some(expando);
    }
    if let Some(start) = variable.array_rest_start
        && let Some(Type::Tuple(elements)) = inferred_symbol_type.as_ref().map(Type::peeled)
    {
        inferred_symbol_type = Some(Type::Tuple(elements.get(start..).unwrap_or_default().to_vec()));
    }

    // A generic arrow assigned to a binding (`const arrayToEnum = <T, U>(…) => …`,
    // the shape every namespace-scoped helper takes) is still a generic call
    // target: without its signature the call site cannot infer type arguments and
    // the result degrades to the sentinel. Only the un-annotated form carries it;
    // an explicit annotation already supplies the callable type.
    let function_signature = match (&declared_type, variable.initializer.as_ref()) {
        // A *type predicate* is carried the same way, generic or not: the guard
        // machinery reads `v is T` off the written signature, and an inferred
        // function type has nowhere to keep it — so `const isW = (v: W | X): v
        // is W => …` narrowed nothing at all, while the same predicate written
        // as a `function` declaration did.
        (None, Some(surge_ts_syntax::ParsedExpression::ArrowFunction(arrow)))
            if !arrow.type_parameters.is_empty()
                || matches!(arrow.return_type, Some(surge_ts_syntax::ParsedType::Predicate(_))) =>
        {
            Some(crate::checks::function::function_signature_info(
                &arrow.type_parameters,
                &arrow.parameters,
                arrow.return_type.as_ref(),
                &ctx.file_name,
            ))
        }
        _ => None,
    };
    // `const isStr = (x: string | number) => typeof x === "string"` implies
    // its predicate exactly as the `function` spelling does.
    let function_signature = function_signature.or_else(|| {
        let (None, Some(surge_ts_syntax::ParsedExpression::ArrowFunction(arrow))) =
            (&declared_type, variable.initializer.as_ref())
        else {
            return None;
        };
        if arrow.return_type.is_some() || arrow.is_async || arrow.is_generator {
            return None;
        }
        let Some(Type::Function(function_type)) = inferred_symbol_type.as_ref() else {
            return None;
        };
        let inferred = match &arrow.body {
            surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => {
                crate::checks::function::infer_predicate_from_body(
                    &arrow.parameters,
                    function_type.parameters(),
                    expression,
                    symbols,
                    ctx,
                )
            }
            surge_ts_syntax::ParsedArrowFunctionBody::Block(statements) => {
                crate::checks::function::infer_predicate_from_function_body(
                    &arrow.parameters,
                    function_type.parameters(),
                    statements,
                    symbols,
                    ctx,
                )
            }
        }?;
        Some(crate::checks::function::with_inferred_predicate(
            crate::checks::function::function_signature_info(
                &arrow.type_parameters,
                &arrow.parameters,
                None,
                &ctx.file_name,
            ),
            Some(inferred),
        ))
    });
    // An explicit *generic* function-type annotation supplies the callable
    // shape, but not the parsed return annotation a call with explicit type
    // arguments needs to re-resolve.
    let function_signature = function_signature.or_else(|| {
        declared_function_type.as_ref().map(|function_type| {
            crate::checks::function::function_type_signature_info(function_type, &ctx.file_name)
        })
    });
    let function_signature = function_signature.or_else(|| {
        alias_call_signature.as_ref().map(|(function_type, declaring_file)| {
            crate::checks::function::function_type_signature_info(function_type, declaring_file)
        })
    });
    // `const alias = identity` binds the very signature it reads, type
    // parameters and all: Go types the binding as its initializer's type.
    let function_signature = function_signature.or_else(|| {
        let (None, Some(surge_ts_syntax::ParsedExpression::Identifier { name, .. })) =
            (&declared_type, variable.initializer.as_ref())
        else {
            return None;
        };
        symbols.get(name)?.function_signature.clone()
    });

    // Go carries an unresolved import's error type through every binding that
    // reads from it, so `const q = trpc.post.all.useQuery()` is `any` there too
    // and a callback passed to a call on `q` has no contextual type. surge's
    // `Type::Any` cannot say which `any` it is, so the provenance is recorded
    // beside the binding instead — otherwise it stopped at the import and every
    // downstream call read as a chain surge merely failed to model.
    if declared_type.is_none()
        && let Some(initializer) = variable.initializer.as_ref()
    {
        if crate::checks::call::property::receiver_any_is_genuine(initializer, symbols, ctx) {
            ctx.genuine_any_bindings.insert(variable_name.clone());
        } else {
            ctx.genuine_any_bindings.remove(variable_name.as_str());
        }
    }

    let symbol = declared_type.or(inferred_symbol_type).map(|ty| {
        Arc::new(SymbolInfo {
            ty,
            kind: symbol_kind,
            function_signature,
        })
    })?;

    if matches!(symbol.kind, SymbolKind::Var)
        && symbols.get_own(&variable_name).is_none_or(|existing| !matches!(existing.kind, SymbolKind::Var))
    {
        if !annotated && has_initializer && !circular && matches!(symbol.ty, Type::Any) {
            ctx.inferred_any_vars.insert(variable_name.clone());
        } else {
            ctx.inferred_any_vars.remove(&variable_name);
        }
    }
    // The first declaration, which a hoisted `var` meets pre-bound to the
    // sentinel.
    if matches!(symbol.kind, SymbolKind::Var)
        && symbols
            .get_own(&variable_name)
            .is_none_or(|existing| !matches!(existing.kind, SymbolKind::Var) || existing.ty.is_unknown())
    {
        if initializer_is_this || annotated_this {
            ctx.this_initialized_vars.insert(variable_name.clone());
        } else {
            ctx.this_initialized_vars.remove(&variable_name);
        }
    }
    if redeclaration_candidate {
        report_redeclared_var_type(
            &variable_name,
            variable_name_span,
            &symbol,
            annotated,
            has_initializer,
            initializer_is_this || annotated_this,
            symbols,
            ctx,
        );
        // A `var` has one type, its first declaration's: a later declaration
        // is checked against it (TS2403) and never replaces it.
        if matches!(symbol.kind, SymbolKind::Var)
            && let Some(first) = symbols
                .get_own_handle(&variable_name)
                .filter(|existing| {
                    matches!(existing.kind, SymbolKind::Var) && !existing.ty.is_unknown()
                })
        {
            return Some(match symbols.declared_type(&variable_name) {
                Some(declared) if *declared != first.ty => Arc::new(SymbolInfo {
                    ty: declared.clone(),
                    kind: first.kind,
                    function_signature: first.function_signature.clone(),
                }),
                _ => first,
            });
        }
    }

    Some(symbol)
}

/// `getTypeOfDestructuredProperty` for a computed key: a binding read through
/// a `string` or `number` key from an object with no index signature for it
/// is TS2537, whatever `noImplicitAny` says. The lowering reads such a binding
/// as `source[key]`.
fn check_computed_binding_key(variable: &ParsedVariableDeclaration, symbols: &SymbolTable, ctx: &mut CheckerContext) {
    if !variable.from_binding_pattern {
        return;
    }
    if let Some(read) = variable.initializer.as_ref() {
        check_computed_pattern_key(read, symbols, ctx);
    }
}

/// [`check_computed_binding_key`] for a read lowered from a pattern, the
/// binding's or a destructuring assignment's.
pub(crate) fn check_computed_pattern_key(read: &ParsedExpression, symbols: &SymbolTable, ctx: &mut CheckerContext) {
    let ParsedExpression::ElementAccess { object, index, index_span, .. } = read else {
        return;
    };
    if !matches!(index.as_ref(), ParsedExpression::Identifier { .. } | ParsedExpression::PropertyAccess { .. }) {
        return;
    }
    let checkpoint = ctx.diagnostics().len();
    let object_type = crate::infer::infer_expression(object, symbols, ctx);
    let index_type = crate::infer::infer_expression(index, symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    let (InferredExpression::Known(object_type), InferredExpression::Known(index_type)) = (object_type, index_type) else {
        return;
    };
    let Type::Object(object) = object_type.peeled() else {
        return;
    };
    let unanswered = match index_type {
        Type::String => object.string_index_type.is_none(),
        Type::Number => object.string_index_type.is_none() && object.number_index_type.is_none(),
        _ => return,
    };
    if unanswered
        && !object.is_intersection
        && object.keyed_indexes.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
        && !object.synthetic_open_index
        && !crate::checks::function::type_contains_degradation(&object_type)
    {
        let diagnostic = Diagnostic::ts2537(object_type.name(), index_type.name(), ctx.file_name.clone());
        ctx.push(match index_span {
            Some(span) => diagnostic.with_span(convert_span(*span)),
            None => diagnostic,
        });
    }
}

/// TS2403: a `var` may be redeclared, but every declaration has to give it the
/// same type. `get_own` keeps this to the declaring scope, so a module `var`
/// that shadows a same-named ambient global is not a redeclaration.
///
/// tsc compares each later declaration's widened type
/// (`getWidenedTypeForVariableLikeDeclaration`: its annotation, its widened
/// initializer, or `any`) with the first's. An initializer surge infers as
/// `any` is not compared: its inference still answers `any` where tsc has a
/// type.
///
/// A declaration without an initializer has type `any` and must be compared.
/// Ambient declarations are pre-registered, so their own registration would
/// read as an earlier declaration.
fn report_redeclared_var_type(
    variable_name: &str,
    variable_name_span: Option<surge_ts_syntax::TextSpan>,
    symbol: &SymbolInfoHandle,
    annotated: bool,
    has_initializer: bool,
    declared_this: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if !matches!(symbol.kind, SymbolKind::Var)
        || (!annotated && has_initializer && matches!(symbol.ty, Type::Any))
    {
        return;
    }
    // The binding may carry a flow-narrowed type; the symbol's own is the
    // declared one recorded beside it.
    let Some(previous) = symbols
        .get_own(variable_name)
        .filter(|existing| matches!(existing.kind, SymbolKind::Var | SymbolKind::ForInNumericKey))
        .map(|existing| symbols.declared_type(variable_name).unwrap_or(&existing.ty).clone())
    else {
        return;
    };
    // A type with one of surge's holes in it (the sentinel, an unsubstituted
    // parameter) is not the type tsc compares. A type variable of the body
    // being checked is (`T`, `T[K]`, a generic mapped type), and is identical
    // only to itself.
    // A namespace function surge keeps only as a permissive signature
    // (`UnmodelledNamespaceFunction`) is one too: its written type is not it.
    let carries_hole = |ty: &Type| {
        let unmodelled_function = matches!(ty, Type::Function(function)
            if function.declaration().is_some_and(|declaration| {
                declaration.is::<crate::modules::UnmodelledNamespaceFunction>()
            }));
        unmodelled_function
            || (!ty.is_type_variable()
                && !matches!(ty, Type::GenuineUnknown)
                && (ty.is_unknown() || surge_ts_types::parameter_type_is_degraded(ty)))
    };
    // A hole further in only hides the position it stands at: a
    // self-referential `var h: new () => typeof h` is expanded only so far, and
    // the redeclaration reading it is cut one level sooner.
    let identical_but_for_holes = |previous: &Type, current: &Type| {
        (surge_ts_types::type_carries_hole_anywhere(previous) || surge_ts_types::type_carries_hole_anywhere(current))
            && surge_ts_types::types_identical_modulo_holes(previous, current)
    };
    // The polymorphic `this` against a declaration that is not `this` itself.
    let first_is_this = ctx.this_initialized_vars.contains(variable_name);
    if first_is_this != declared_this
        && !(matches!(previous, Type::Any) && ctx.inferred_any_vars.contains(variable_name))
        && !previous.is_degraded()
        && !symbol.ty.is_degraded()
    {
        let (previous_name, current_name) = if first_is_this {
            ("this".to_string(), symbol.ty.name())
        } else {
            (previous.name(), "this".to_string())
        };
        let diagnostic = Diagnostic::ts2403(variable_name, previous_name, current_name, ctx.file_name.clone());
        ctx.push(match variable_name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
        return;
    }
    if (matches!(previous, Type::Any) && ctx.inferred_any_vars.contains(variable_name))
        || carries_hole(&previous)
        || carries_hole(&symbol.ty)
        || surge_ts_types::is_type_identical_to(&previous, &symbol.ty)
        || identical_but_for_holes(&previous, &symbol.ty)
    {
        return;
    }

    let diagnostic = Diagnostic::ts2403(
        variable_name,
        previous.name(),
        symbol.ty.name(),
        ctx.file_name.clone(),
    );
    ctx.push(match variable_name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// An un-annotated TypeScript `var` whose initializer needs the variable's own
/// type outside a nested function. JavaScript's `var ns = ns || {}` is an
/// expando initializer instead.
pub(crate) fn is_circular_var_initializer(variable: &ParsedVariableDeclaration, file_name: &str) -> bool {
    variable.kind == surge_ts_syntax::ParsedVariableKind::Var
        && variable.declared_type.is_none()
        && !surge_ts_syntax::is_javascript_file_name(file_name)
        && variable
            .initializer
            .as_ref()
            .is_some_and(|initializer| reads_own_binding(initializer, &variable.name))
}

fn reads_own_binding(expression: &ParsedExpression, name: &str) -> bool {
    let head = |path: &str| path.split('.').next() == Some(name);
    let direct = match expression {
        ParsedExpression::Identifier { name: read, .. } => read == name,
        ParsedExpression::Call { callee_name, .. } => head(callee_name),
        ParsedExpression::IndexAccess { object_name, .. } => head(object_name),
        ParsedExpression::Assignment { target_name, .. } => target_name == name,
        _ => false,
    };
    let mut found = direct;
    if !found {
        expression.for_each_child(&mut |child| found |= reads_own_binding(child, name));
    }
    found
}

/// A declaration tsc gives a control-flow tracked `any[]` (`autoArrayType`):
/// an un-annotated, non-ambient, non-destructured variable initialized with
/// `[]`, under `noImplicitAny`.
pub(crate) fn is_auto_array_candidate(
    variable: &surge_ts_syntax::ParsedVariableDeclaration,
    ctx: &CheckerContext,
) -> bool {
    ctx.options.no_implicit_any
        && variable.declared_type.is_none()
        && !variable.is_declare
        && !variable.from_binding_pattern
        && matches!(
            &variable.initializer,
            Some(ParsedExpression::ArrayLiteral { elements, .. }) if elements.is_empty()
        )
}

/// tsc widens only a fresh literal. A bare literal read from a mutable binding
/// or parameter whose declared type names that very literal is the declared,
/// regular type (flow narrowing filters the declared union), so
/// `let a: "foo" = "foo"; let b = a` stays `"foo"`. A union keeps the regular
/// literal over a fresh duplicate (`removeRedundantLiteralTypes`), so either
/// operand of a logical or conditional expression supplying it is enough.
fn reads_regular_literal(expression: &ParsedExpression, ty: &Type, symbols: &SymbolTable) -> bool {
    let enum_base = match ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => None,
        Type::Reference(reference) if reference.enum_base.is_some() => reference.enum_base.as_deref(),
        _ => return false,
    };
    match expression {
        ParsedExpression::Identifier { name, .. } => {
            let Some(symbol) = symbols.get(name) else {
                return false;
            };
            if !matches!(symbol.kind, SymbolKind::Let | SymbolKind::Var | SymbolKind::Parameter) {
                return false;
            }
            // An enum member is a member of its enum's union.
            let names = |declared: &Type| declared == ty || enum_base.is_some_and(|base| declared == base);
            match symbols.declared_type(name).unwrap_or(&symbol.ty) {
                Type::Union(union) => union.types().iter().any(names),
                declared => names(declared),
            }
        }
        ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. } => {
            reads_regular_literal(left, ty, symbols) || reads_regular_literal(right, ty, symbols)
        }
        ParsedExpression::Conditional {
            when_true,
            when_false,
            ..
        } => reads_regular_literal(when_true, ty, symbols) || reads_regular_literal(when_false, ty, symbols),
        _ => false,
    }
}

/// [`widen_implicit_variable_initializer_type`] for a variable declaration,
/// which tsc auto-types only under `noImplicitAny` and when its initializer is
/// written `null` or `undefined` (`isNullOrUndefined`): a read that merely is
/// `undefined` (`t[2]` past a tuple's end) keeps that type.
pub(crate) fn widen_variable_declaration_initializer_type(
    symbol_kind: SymbolKind,
    initializer: &ParsedExpression,
    ty: &Type,
    auto_array: bool,
    no_implicit_any: bool,
) -> Type {
    if surge_ts_types::strict_null_checks()
        && matches!(symbol_kind, SymbolKind::Let | SymbolKind::Var)
        && matches!(ty, Type::Null | Type::Undefined)
        && (!no_implicit_any
            || !matches!(initializer, ParsedExpression::UndefinedLiteral | ParsedExpression::NullLiteral))
    {
        return ty.clone();
    }
    widen_implicit_variable_initializer_type(symbol_kind, initializer, ty, auto_array)
}

pub(crate) fn widen_implicit_variable_initializer_type(
    symbol_kind: SymbolKind,
    initializer: &ParsedExpression,
    ty: &Type,
    auto_array: bool,
) -> Type {
    // `getWidenedTypeForVariableLikeDeclaration`: under `noImplicitAny` an
    // empty array initializer makes an evolving array (`autoArrayType`, read
    // as `any[]` until pushes give it elements); otherwise it stays `never[]`.
    if matches!(initializer, ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty())
    {
        return if auto_array || !surge_ts_types::strict_null_checks() {
            Type::Array(Box::new(Type::Any))
        } else {
            ty.clone()
        };
    }
    // Without `strictNullChecks`, `undefined` and `null` are widening types:
    // a declaration initialized with one is `any` (`getWidenedType`).
    if !surge_ts_types::strict_null_checks() {
        match ty {
            Type::Undefined => return Type::Any,
            Type::Array(element) if **element == Type::Undefined => {
                return Type::Array(Box::new(Type::Any));
            }
            _ => {}
        }
    }
    // tsc widens only fresh literal types, and an assertion (`as const`,
    // `as "a"`, `<T>x`) yields its regular type, so `let s = "a" as const`
    // stays `"a"`.
    let is_assertion = matches!(
        initializer,
        ParsedExpression::ConstAssertion { .. } | ParsedExpression::TypeAssertion { .. }
    );
    // A mutable binding initialized to `null`/`undefined` is auto-typed: it
    // evolves with its assignments, which surge approximates as `any`.
    if matches!(symbol_kind, SymbolKind::Let | SymbolKind::Var)
        && matches!(ty, Type::Null | Type::Undefined)
    {
        return Type::Any;
    }
    let ty = &widen_nullable_type(ty);
    // A *bare* literal type is widened however it was reached: freshness
    // survives a `const` read (`const a = "x"; let b = a` is `string`), and
    // surge does not track it on the type itself. A union of literals is not
    // widened unless it was written here, which is what keeps
    // `let status = state.status` at its declared union.
    // A call written with type arguments returns their instantiation, whose
    // literals were written as types and are regular.
    let instantiated_by_written_arguments = matches!(
        initializer,
        ParsedExpression::Call { type_arguments, .. }
            | ParsedExpression::PropertyCall { type_arguments, .. }
            | ParsedExpression::New { type_arguments, .. }
            if !type_arguments.is_empty()
    );
    let widens_as_literal = !instantiated_by_written_arguments
        && match ty {
            Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => true,
            Type::Reference(reference) => reference.enum_base.is_some(),
            _ => false,
        };
    let widened = if matches!(symbol_kind, SymbolKind::Let | SymbolKind::Var)
        && !is_assertion
        && (widens_as_literal || initializer_type_is_fresh(initializer))
    {
        // tsc deep-widens `let`/`var` initializers, so object properties and
        // array/union members widen too (e.g. `let o = { a: 1 }` -> `{ a: number }`),
        // not just a top-level primitive literal.
        widen_fresh_object_literal(initializer, ty).unwrap_or_else(|| widen_type(ty))
    } else if matches!(initializer, ParsedExpression::ObjectLiteral { .. }) {
        widen_object_literal_members(initializer, ty)
    } else {
        ty.clone()
    };
    if writes_object_literal_union(initializer) {
        normalize_object_literal_union(&widened)
    } else {
        widened
    }
}

/// Whether the initializer writes object literals that widen together: the
/// branches of a conditional or logical expression, or the elements of an
/// array literal (read whole or through an index).
fn writes_object_literal_union(initializer: &ParsedExpression) -> bool {
    fn written(expression: &ParsedExpression) -> usize {
        match expression {
            ParsedExpression::ObjectLiteral { .. } => 1,
            ParsedExpression::Conditional {
                when_true,
                when_false,
                ..
            } => written(when_true) + written(when_false),
            ParsedExpression::Logical { left, right, .. }
            | ParsedExpression::NullishCoalescing { left, right, .. } => {
                written(left) + written(right)
            }
            ParsedExpression::ArrayLiteral { elements, .. } => elements
                .iter()
                .filter(|element| !element.spread)
                .map(|element| written(&element.expression))
                .sum(),
            ParsedExpression::ElementAccess { object, .. } => written(object),
            _ => 0,
        }
    }
    written(initializer) >= 2
}

/// tsc's `getWidenedTypeWithContext` restricted to object-literal
/// normalization: object literals widened together each gain the names their
/// siblings write as `name?: undefined`, so `(c ? { a: 1 } : { a: 1, b: "x" }).b`
/// reads `string | undefined` instead of being a missing property. A property
/// is widened in the child context whose siblings are the same property of the
/// sibling literals (`getSiblingsOfContext`), so nested literals normalize too.
/// The members keep the types they were written with: `type: "ok" as const`
/// stays the discriminant it is.
fn normalize_object_literal_union(ty: &Type) -> Type {
    normalize_with_context(ty, None)
}

fn is_written_object_literal(member: &Type) -> bool {
    matches!(member, Type::Object(object) if is_written_object_literal_object(object))
}

fn is_written_object_literal_object(object: &surge_ts_types::ObjectType) -> bool {
    object.alias_name.is_none()
        && !object.without_inferable_index
        && !object.synthetic_open_index
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
}

fn normalize_with_context(ty: &Type, siblings: Option<&[Type]>) -> Type {
    match ty {
        Type::Array(element) => Type::Array(Box::new(normalize_with_context(element, None))),
        Type::Union(union) => {
            let members = union.types();
            let siblings = siblings.unwrap_or(members);
            let normalized = members
                .iter()
                .map(|member| match member {
                    Type::Null | Type::Undefined => member.clone(),
                    _ => normalize_with_context(member, Some(siblings)),
                })
                .collect();
            surge_ts_types::union_type(normalized)
        }
        Type::Object(object) if is_written_object_literal(ty) => {
            let mut properties = (*object.properties).clone();
            for (name, property) in properties.iter_mut() {
                let child_siblings = siblings.map(|siblings| property_siblings(siblings, name));
                property.ty = normalize_with_context(&property.ty, child_siblings.as_deref());
            }
            if let Some(siblings) = siblings {
                for sibling in siblings {
                    let Type::Object(sibling) = sibling else { continue };
                    if !is_written_object_literal_object(sibling) {
                        continue;
                    }
                    for name in sibling.properties.keys() {
                        if !properties.contains_key(name) {
                            properties.insert(name.clone(), undefined_or_missing_property());
                        }
                    }
                }
            }
            Type::Object(crate::metrics::alloc_object_type(properties, None))
        }
        _ => ty.clone(),
    }
}

/// `getUndefinedProperty`: the member a normalized object literal gains for a
/// sibling's property, typed `undefinedOrMissingType`. Under
/// `exactOptionalPropertyTypes` that is tsc's missing type, which surge spells
/// as an optional `never` (a read adds the `undefined` back, and `"p" in o`
/// removes it again).
pub(crate) fn undefined_or_missing_property() -> surge_ts_types::ObjectProperty {
    let ty = if surge_ts_types::exact_optional_property_types() { Type::Never } else { Type::Undefined };
    surge_ts_types::ObjectProperty::optional(ty)
}

/// `getSiblingsOfContext` for a child context: the named property's type in
/// each sibling literal, unions distributed.
fn property_siblings(siblings: &[Type], name: &str) -> Vec<Type> {
    let mut result = Vec::new();
    for sibling in siblings {
        if !is_written_object_literal(sibling) {
            continue;
        }
        let Type::Object(object) = sibling else { continue };
        let Some(property) = object.properties.get(name) else { continue };
        match &property.ty {
            Type::Union(union) => result.extend(union.types().iter().cloned()),
            other => result.push(other.clone()),
        }
    }
    result
}

/// tsc's `getWidenedType` without `strictNullChecks`: `null` and `undefined`
/// widen to `any` wherever a declaration's type is inferred, members included.
pub(crate) fn widen_nullable_type(ty: &Type) -> Type {
    if surge_ts_types::strict_null_checks() {
        return ty.clone();
    }
    widen_nullish_members(ty)
}

/// tsc's `getWidenedType` without strictNullChecks: `null` and `undefined`
/// widen to `any` wherever an inferred type holds them, and a union absorbs
/// them (`getUnionType`), leaving `any` when nothing else is left.
fn widen_nullish_members(ty: &Type) -> Type {
    match ty {
        Type::Null | Type::Undefined => Type::Any,
        Type::Array(element) => Type::Array(Box::new(widen_nullish_members(element))),
        Type::Tuple(elements) => Type::Tuple(elements.iter().map(widen_nullish_members).collect()),
        Type::Union(union) if union.types().iter().any(contains_nullish) => {
            let rest: Vec<Type> = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Null | Type::Undefined))
                .map(widen_nullish_members)
                .collect();
            if rest.is_empty() {
                Type::Any
            } else {
                surge_ts_types::union_type(rest)
            }
        }
        Type::Object(object)
            if object.alias_name.is_none()
                && object.properties.values().any(|property| contains_nullish(&property.ty)) =>
        {
            let mut properties = (*object.properties).clone();
            for property in properties.values_mut() {
                property.ty = widen_nullish_members(&property.ty);
            }
            let mut widened = crate::metrics::alloc_object_type(
                properties,
                object.string_index_type.as_deref().cloned(),
            );
            if object.synthetic_open_index {
                widened = widened.with_open_index_marker();
            }
            if let Some(call_signature) = object.call_signature() {
                widened = widened.with_call_signature(call_signature.clone());
            }
            if let Some(construct_signature) = object.construct_signature() {
                widened = widened.with_construct_signature(construct_signature.clone());
            }
            Type::Object(widened)
        }
        _ => ty.clone(),
    }
}

fn contains_nullish(ty: &Type) -> bool {
    match ty {
        Type::Null | Type::Undefined => true,
        Type::Array(element) => contains_nullish(element),
        Type::Tuple(elements) => elements.iter().any(contains_nullish),
        Type::Union(union) => union.types().iter().any(contains_nullish),
        Type::Object(object) if object.alias_name.is_none() => {
            object.properties.values().any(|property| contains_nullish(&property.ty))
        }
        _ => false,
    }
}

/// Whether the initializer's type is *fresh* — written as a literal here — which
/// is the only kind tsc widens (`getWidenedLiteralType`). A literal type read
/// from somewhere else (a property of a union-typed object, a call's declared
/// return) is regular, so `let status = state.status` keeps the union instead
/// of widening to `string`.
/// tsc's `getAssignmentReducedType` at the declaration: a binding declared as
/// the enum its initializer's member widened to (`let x = E.A` is `E`) holds
/// that member until it is reassigned, so `x === E.B` has no overlap.
pub(crate) fn enum_member_initializer_narrowing(
    initializer: &ParsedExpression,
    declared: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Reference(declared_reference) = declared else {
        return None;
    };
    declared_reference.enum_owner.as_ref()?;
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(initializer, symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    let InferredExpression::Known(ty) = inferred else {
        return None;
    };
    let member_of_declared =
        |ty: &Type| matches!(ty, Type::Reference(reference) if reference.enum_base.as_deref() == Some(declared));
    let narrows = match &ty {
        Type::Union(union) => union.types().iter().all(member_of_declared),
        other => member_of_declared(other),
    };
    narrows.then_some(ty)
}

/// Whether an unannotated `let`/`var` initializer can read an enum member
/// ([`enum_member_initializer_narrowing`]), so it is worth keeping for it.
pub(crate) fn may_read_enum_member(initializer: &ParsedExpression) -> bool {
    match initializer {
        ParsedExpression::PropertyAccess { .. } => true,
        ParsedExpression::Conditional {
            when_true,
            when_false,
            ..
        } => may_read_enum_member(when_true) && may_read_enum_member(when_false),
        _ => false,
    }
}

/// A call through a function whose return was read from its body hands back
/// the fresh literal types its `return`s produced, which a mutable binding
/// widens (`let mode = pick()` is `string` where `pick` returns `"a" | "b"`);
/// a written return type's literals are regular and stay.
fn returns_body_inferred_type(
    initializer: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let callee = match initializer {
        ParsedExpression::Await { operand, .. } => {
            return returns_body_inferred_type(operand, symbols, ctx);
        }
        // A type written as a type argument is no fresh literal to widen.
        ParsedExpression::Call { type_arguments, .. } | ParsedExpression::PropertyCall { type_arguments, .. }
            if !type_arguments.is_empty() =>
        {
            return false;
        }
        ParsedExpression::Call { callee_name, .. } => symbols.get(callee_name).map(|symbol| symbol.ty.clone()),
        ParsedExpression::PropertyCall {
            object,
            object_span,
            property_name,
            property_span,
            ..
        } => match crate::infer::expression::infer_property_access(
            object,
            object_span,
            property_name,
            property_span,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(ty) => Some(ty),
            _ => None,
        },
        _ => None,
    };
    matches!(callee, Some(Type::Function(function))
        if crate::checks::function::is_body_inferred_return(function.return_type()))
}

fn initializer_type_is_fresh(initializer: &ParsedExpression) -> bool {
    match initializer {
        ParsedExpression::StringLiteral(_)
        | ParsedExpression::NumberLiteral(_)
        | ParsedExpression::BigIntLiteral(_)
        | ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::NullLiteral
        | ParsedExpression::UndefinedLiteral
        | ParsedExpression::ObjectLiteral { .. }
        | ParsedExpression::ArrayLiteral { .. }
        | ParsedExpression::TemplateLiteral { .. }
        | ParsedExpression::Unary { .. } => true,
        ParsedExpression::Conditional {
            when_true,
            when_false,
            ..
        } => initializer_type_is_fresh(when_true) && initializer_type_is_fresh(when_false),
        ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. } => {
            initializer_type_is_fresh(left) && initializer_type_is_fresh(right)
        }
        _ => false,
    }
}

/// `getWidenedTypeOfObjectLiteral` widens each member's fresh literal type;
/// a member a spread supplies has the type its source declares, which is not
/// fresh (`let s = { ...state }` keeps `state.status: "a" | "b"`). `None` for
/// a literal without spreads, which widens whole.
fn widen_fresh_object_literal(initializer: &ParsedExpression, ty: &Type) -> Option<Type> {
    let (ParsedExpression::ObjectLiteral { properties, .. }, Type::Object(object)) = (initializer, ty) else {
        return None;
    };
    let last_spread = properties.iter().rposition(|property| property.is_spread)?;
    let mut widened_properties = (*object.properties).clone();
    for (index, property) in properties.iter().enumerate() {
        if property.is_spread {
            continue;
        }
        let Some(member) = widened_properties.get_mut(property.name.as_str()) else {
            continue;
        };
        // Written before a spread, the member is the spread's when the spread
        // supplies that name; a literal still standing is the written one.
        let still_written = index > last_spread
            || matches!(
                (&property.value, &member.ty),
                (ParsedExpression::StringLiteral(_), Type::StringLiteral(_))
                    | (ParsedExpression::NumberLiteral(_), Type::NumberLiteral(_))
                    | (ParsedExpression::BooleanLiteral(_), Type::BooleanLiteral(_))
            );
        if still_written {
            member.ty = widen_type(&member.ty);
        }
    }
    let mut widened = object.clone();
    widened.properties = std::sync::Arc::new(widened_properties);
    Some(Type::Object(widened))
}

/// A property initializer is a mutable location, so tsc widens its fresh
/// literal type even under `const` (`checkExpressionForMutableLocation`):
/// `const o = { a: 1 }` is `{ a: number }`. Only members written as literals
/// are widened — surge does not track freshness, and an identifier or an
/// assertion may carry a regular literal type that tsc keeps.
fn widen_object_literal_members(initializer: &ParsedExpression, ty: &Type) -> Type {
    let (ParsedExpression::ObjectLiteral { properties, .. }, Type::Object(object)) =
        (initializer, ty)
    else {
        return ty.clone();
    };
    if object.alias_name.is_some() {
        return ty.clone();
    }
    let mut widened_properties = (*object.properties).clone();
    let mut changed = false;
    for property in properties {
        if property.is_spread || property.is_method || property.is_accessor {
            continue;
        }
        let Some(member) = widened_properties.get_mut(property.name.as_str()) else {
            continue;
        };
        let widened = match &property.value {
            ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::TemplateLiteral { .. } => widen_type(&member.ty),
            nested @ ParsedExpression::ObjectLiteral { .. } => {
                widen_object_literal_members(nested, &member.ty)
            }
            // An enum member read off its enum (`{ k: E.A }`) is a fresh
            // literal, as `getDeclaredTypeOfEnum` declares every member.
            ParsedExpression::PropertyAccess { .. }
                if matches!(&member.ty, Type::Reference(reference) if reference.enum_base.is_some()) =>
            {
                widen_type(&member.ty)
            }
            _ => continue,
        };
        if widened != member.ty {
            member.ty = widened;
            changed = true;
        }
    }
    if !changed {
        return ty.clone();
    }
    let mut rebuilt = crate::metrics::alloc_object_type(
        widened_properties,
        object.string_index_type.as_deref().cloned(),
    );
    if object.synthetic_open_index {
        rebuilt = rebuilt.with_open_index_marker();
    }
    if let Some(call_signature) = object.call_signature() {
        rebuilt = rebuilt.with_call_signature(call_signature.clone());
    }
    if let Some(construct_signature) = object.construct_signature() {
        rebuilt = rebuilt.with_construct_signature(construct_signature.clone());
    }
    Type::Object(rebuilt)
}

/// Whether `ty` carries the `unknown` *degradation sentinel* anywhere in its
/// shape. The genuine `unknown` keyword ([`Type::GenuineUnknown`]) is
/// deliberately NOT matched: a declared `{ input?: unknown }` member is a real,
/// checkable type in tsc, and suppressing the assignability check for it hides
/// genuine TS2322s. Only surge's could-not-model sentinel warrants no-cascade.
fn type_contains_unknown(ty: &Type) -> bool {
    match ty {
        Type::Unknown => true,
        Type::TypeParameter(parameter) => !crate::checks::assign::is_bound_type_parameter(parameter),
        Type::Array(element) => type_contains_unknown(element),
        Type::Tuple(elements) => elements.iter().any(type_contains_unknown),
        // A resolved generic member signature carries the sentinel where its
        // own type parameters (and a self-reference instantiated with them)
        // were erased; that is a bound name, not a gap — see the same arm in
        // `checks::function::body::contains_unknown`.
        Type::Function(function) if function.type_parameter_head().is_some() => false,
        Type::Function(function) => {
            !crate::checks::call::is_generic_signature(function)
                && crate::checks::assign::with_signature_type_parameters(function, || {
                    function.parameters().iter().any(type_contains_unknown)
                        || type_contains_unknown(function.return_type())
                })
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| type_contains_unknown(&property.ty))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(type_contains_unknown)
        }
        Type::Union(union) => union.types().iter().any(type_contains_unknown),
        _ => false,
    }
}

/// The source an array-pattern element reads from: the object of the
/// `source[index]` access the pattern lowers each element to, beneath a
/// default's `??`.
/// tsc's `getBindingElementTypeFromParentType` for an array pattern over a
/// source that is not array-like: an element binds the source's iterated type
/// (`checkIteratedTypeOrElementType`), a rest element an array of it, where
/// the lowered indexed read would find no index signature. Only a lib
/// collection or iterator reference is known to be such a source.
fn iterated_array_pattern_element(
    in_array_pattern: bool,
    is_rest: bool,
    initializer: Option<&ParsedExpression>,
    initializer_span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let initializer = initializer.filter(|_| in_array_pattern)?;
    let (source, default) = match initializer {
        _ if is_rest => (initializer.clone(), None),
        ParsedExpression::NullishCoalescing { left, right, right_span, .. } => {
            (array_pattern_source(left)?, Some((right.as_ref(), *right_span)))
        }
        _ => (array_pattern_source(initializer)?, None),
    };
    let InferredExpression::Known(source_type) = crate::infer::infer_expression(&source, symbols, ctx) else {
        return None;
    };
    // `checkArrayLiteralDestructuringElementAssignment`: a source that is not
    // array-like is read through its iteration protocol.
    let iterated = || {
        crate::checks::expr::iteration_types_of_iterable(
            &source_type,
            crate::checks::expr::IterationUse::sync(ctx.options.strict_builtin_iterator_return),
        )
        .map(|types| types.yield_type)
    };
    let element = match &source_type {
        Type::Reference(reference) => {
            crate::checks::function::iterable_reference_element_type(reference).or_else(iterated)?
        }
        Type::Object(_) => iterated()?,
        _ => return None,
    };
    if element.is_unmodelled() {
        return None;
    }
    let _ = evaluate_expression(&source, initializer_span, symbols, ctx);
    if is_rest {
        return Some(Type::Array(Box::new(element)));
    }
    let Some((default, default_span)) = default else {
        return Some(element);
    };
    match evaluate_expression(default, default_span.or(initializer_span), symbols, ctx) {
        InferredExpression::Known(default_type) if !default_type.is_unmodelled() => Some(
            surge_ts_types::union_type(vec![surge_ts_types::remove_undefined(&element), default_type]),
        ),
        _ => None,
    }
}

/// A binding lowers each default on its path as `read ?? default`, nested
/// patterns reading through it: the spans of those defaulted element reads.
fn defaulted_element_reads(initializer: &ParsedExpression, spans: &mut Vec<surge_ts_syntax::TextSpan>) {
    match initializer {
        ParsedExpression::NullishCoalescing { left, .. } => {
            if let ParsedExpression::ElementAccess { index_span: Some(span), .. }
            | ParsedExpression::IndexAccess { index_span: Some(span), .. } = left.as_ref()
            {
                spans.push(*span);
            }
            defaulted_element_reads(left, spans);
        }
        ParsedExpression::ElementAccess { object, .. } | ParsedExpression::PropertyAccess { object, .. } => {
            defaulted_element_reads(object, spans);
        }
        _ => {}
    }
}

fn array_pattern_source(initializer: &ParsedExpression) -> Option<ParsedExpression> {
    match initializer {
        ParsedExpression::NullishCoalescing { left, .. } => array_pattern_source(left),
        ParsedExpression::IndexAccess {
            object_name,
            object_span,
            index,
            ..
        } if matches!(index.as_ref(), ParsedExpression::NumberLiteral(_)) => {
            Some(ParsedExpression::Identifier {
                name: object_name.clone(),
                span: *object_span,
            })
        }
        ParsedExpression::ElementAccess { object, index, .. }
            if matches!(index.as_ref(), ParsedExpression::NumberLiteral(_)) =>
        {
            Some(object.as_ref().clone())
        }
        _ => None,
    }
}

fn is_symbol_constructor_call(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::Call { callee_name, .. } => callee_name == "Symbol",
        ParsedExpression::PropertyCall {
            object,
            property_name,
            ..
        } => {
            property_name == "for"
                && matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "Symbol")
        }
        _ => false,
    }
}

/// A type literal whose one member is a generic call signature is a callable
/// like a generic function type: `{ <T>(x: T): T }`.
fn lone_generic_call_signature(
    ty: &surge_ts_syntax::ParsedType,
) -> Option<std::sync::Arc<surge_ts_syntax::ParsedFunctionType>> {
    let surge_ts_syntax::ParsedType::Object(object) = ty else {
        return None;
    };
    let signature = object.call_signature.as_deref()?;
    (object.call_signature_overloads.is_empty()
        && object.construct_signature.is_none()
        && !signature.type_parameters.is_empty())
    .then(|| std::sync::Arc::new(signature.clone()))
}

/// tsc's `checkVariableLikeDeclaration` for `using` / `await using`: the
/// initializer must be `Disposable` (or, awaited, `AsyncDisposable`), `null`
/// or `undefined`. An object source of `using` is elaborated against
/// `Disposable`, the union's one object member, so its missing member is the
/// report (TS2741). A type surge could not enumerate is left alone.
fn report_undisposable_using_initializer(
    initializer_type: &Type,
    await_using: bool,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    fn disposable(ty: &Type, await_using: bool) -> Option<bool> {
        // A named type may inherit the member from a lib base surge does not
        // enumerate (`ArrayIterator` through `IteratorObject`).
        if matches!(ty, Type::Reference(_)) {
            return None;
        }
        match ty.peeled() {
            Type::Null | Type::Undefined | Type::Any | Type::Never => Some(true),
            Type::Union(union) => {
                let mut all = true;
                for member in union.types() {
                    all &= disposable(member, await_using)?;
                }
                Some(all)
            }
            Type::Object(object) => {
                if object.synthetic_open_index
                    || object.string_index_type.is_some()
                    || object.is_intersection
                    || object.properties.values().any(|property| property.ty.is_unknown())
                {
                    return None;
                }
                Some(
                    object.get_property("[Symbol.dispose]").is_some()
                        || (await_using && object.get_property("[Symbol.asyncDispose]").is_some()),
                )
            }
            Type::String | Type::Number | Type::Boolean | Type::StringLiteral(_) | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => Some(false),
            _ => None,
        }
    }
    if disposable(initializer_type, await_using) != Some(false) {
        return;
    }
    let file_name = ctx.file_name.clone();
    let diagnostic = if await_using {
        Diagnostic::ts2851(file_name)
    } else if matches!(initializer_type.peeled(), Type::Object(_)) {
        // The union's one object member is what the relation is elaborated
        // against: the missing member is the report.
        Diagnostic::ts2741("[Symbol.dispose]", widen_type(initializer_type).name(), "Disposable", file_name)
    } else {
        Diagnostic::ts2850(file_name)
    };
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}
