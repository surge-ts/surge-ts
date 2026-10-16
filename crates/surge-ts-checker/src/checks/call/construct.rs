//! `new` on a generic class. Its value side is `any` (`generic_class_value_symbol`),
//! so the construct signatures tsc resolves such a call against ride on the
//! signature that value carries, and the arguments are checked against them
//! here.

use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedCallArgument, ParsedClassDeclaration, ParsedClassMember, ParsedExpression,
    ParsedNamedType, ParsedType, ParsedTypeParameter, TextSpan as SyntaxTextSpan,
};
use surge_ts_types::{FunctionType, Type, TypeCopyReason, is_assignable_to, with_type_copy_reason};

use super::{
    call_site_required_count, check_function_type_call, instantiate_function_type,
    overload_arity_fits, overload_group, parsed_type_mentions_any, type_arguments_start,
};
use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::{ConstructSignature, SymbolInfo, SymbolKind, SymbolTable};

/// tsc's construct signatures of a generic class (`resolveAnonymousTypeMembers`),
/// each written over the class's type parameters: its constructor
/// declarations, less the implementation that follows overloads, else
/// `getDefaultConstructSignatures` — one taking no arguments, or the
/// signatures of `base`, the value its `extends` clause names. Empty where
/// they cannot be written here, which leaves `new` unchecked.
pub(crate) fn generic_class_construct_signatures(
    class: &ParsedClassDeclaration,
    base: Option<&SymbolInfo>,
    declaring_file: &str,
) -> Vec<ConstructSignature> {
    let constructors: Vec<_> = class
        .members
        .iter()
        .filter_map(|member| match member {
            ParsedClassMember::Constructor(constructor) => Some(constructor),
            _ => None,
        })
        .collect();
    let ambient = class.is_declare || crate::modules::is_declaration_file_name(declaring_file);
    let declared = match constructors.split_last() {
        Some((_, overloads)) if !ambient && !overloads.is_empty() => overloads,
        _ => &constructors[..],
    };
    if !declared.is_empty() {
        return declared
            .iter()
            .map(|constructor| ConstructSignature {
                signature: crate::checks::function::function_signature_info(
                    &class.type_parameters,
                    &constructor.parameters,
                    None,
                    declaring_file,
                ),
                template: FunctionType::new(
                    vec![Type::Any; constructor.parameters.len()],
                    Type::Any,
                    constructor
                        .parameters
                        .last()
                        .is_some_and(|parameter| parameter.rest),
                    crate::checks::function::required_parameter_count(&constructor.parameters),
                ),
            })
            .collect();
    }
    if class.heritage_expression.is_some() {
        return Vec::new();
    }
    // A signature that writes none of its parameters keeps every one from the
    // template it is instantiated through.
    let unwritten = || {
        crate::checks::function::function_signature_info(
            &class.type_parameters,
            &[],
            None,
            declaring_file,
        )
    };
    let Some(base_reference) = class.extends.first() else {
        return vec![ConstructSignature {
            signature: unwritten(),
            template: FunctionType::new(Vec::new(), Type::Any, false, 0),
        }];
    };
    let Some(base) = base else {
        return Vec::new();
    };
    if let Some(base_signatures) = base
        .function_signature
        .as_ref()
        .and_then(|signature| signature.construct_signatures.as_ref())
    {
        return base_signatures
            .iter()
            .filter_map(|inherited| {
                inherited_generic_construct_signature(class, base_reference, inherited, declaring_file)
            })
            .collect();
    }
    let Type::Object(base_static) = base.ty.peeled() else {
        return Vec::new();
    };
    let Some(construct_signature) = base_static.construct_signature() else {
        return Vec::new();
    };
    let mut members = Vec::new();
    construct_signature.push_overload_members(&mut members);
    members
        .into_iter()
        .map(|template| ConstructSignature {
            signature: unwritten(),
            template,
        })
        .collect()
}

/// `getDefaultConstructSignatures` over a generic base: the base signature
/// instantiated with the `extends` clause's type arguments, with the class's
/// own type parameters for its type parameters. Written only where that
/// instantiation renames nothing — every base type parameter the signature's
/// parameters mention is passed the class's own parameter of the same name —
/// and the base is declared in the same file. Anything else re-resolves an
/// `extends` argument, or one of the class's constraints, in a scope that may
/// give its names another meaning.
fn inherited_generic_construct_signature(
    class: &ParsedClassDeclaration,
    base_reference: &ParsedNamedType,
    inherited: &ConstructSignature,
    declaring_file: &str,
) -> Option<ConstructSignature> {
    if inherited.signature.declaring_file.as_deref() != Some(declaring_file) {
        return None;
    }
    let base_parameters = &inherited.signature.type_parameters;
    let written = base_reference.type_arguments.len();
    let minimum = base_parameters
        .iter()
        .filter(|parameter| parameter.default_type.is_none())
        .count();
    // tsc drops a base signature whose type parameters the clause's argument
    // count does not fit.
    if written < minimum || written > base_parameters.len() {
        return None;
    }
    let passes_own_parameter = |index: usize, name: &str| {
        matches!(
            base_reference.type_arguments.get(index),
            Some(ParsedType::Named(argument))
                if argument.type_arguments.is_empty()
                    && argument.name == name
                    && class.type_parameters.iter().any(|parameter| parameter.name == name)
        )
    };
    let renamed: Vec<&str> = base_parameters
        .iter()
        .enumerate()
        .filter(|(index, parameter)| !passes_own_parameter(*index, &parameter.name))
        .map(|(_, parameter)| parameter.name.as_str())
        .collect();
    if !renamed.is_empty()
        && inherited
            .signature
            .parameter_types
            .iter()
            .flatten()
            .any(|parameter| parsed_type_mentions_any(parameter, &renamed))
    {
        return None;
    }
    let mut signature = (*inherited.signature).clone();
    signature.type_parameters = class.type_parameters.clone();
    Some(ConstructSignature {
        signature: Arc::new(signature),
        template: inherited.template.clone(),
    })
}

/// tsc's `resolveNewExpression` → `resolveCall` against a generic class's
/// construct signatures ([`generic_class_construct_signatures`]). Candidates
/// the argument count rules out are dropped first; the rest are instantiated
/// with the written type arguments, else with what each infers from the
/// arguments, and the arguments checked against them — several candidates as
/// one overload group. The instance the call produces is built by name
/// (`generic_class_instance_type`).
pub(crate) fn check_generic_class_construct(
    callee: &ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let ParsedExpression::Identifier { name, .. } = callee else {
        return false;
    };
    let Some((kind, function_signature)) = super::class_value_signature(name, symbols, ctx) else {
        return false;
    };
    // A binding that can hold another value no longer denotes the class.
    if matches!(kind, SymbolKind::Parameter | SymbolKind::Let | SymbolKind::Var) {
        return false;
    }
    let Some(signatures) = function_signature
        .as_ref()
        .and_then(|signature| signature.construct_signatures.clone())
    else {
        return false;
    };
    let Some(first) = signatures.first() else {
        return false;
    };
    let type_parameters = first.signature.type_parameters.clone();
    // Every candidate takes the class's type parameters, so a count they do
    // not take is the TS2558 reported where the callee is resolved.
    let minimum_type_arguments = type_parameters
        .iter()
        .filter(|parameter| parameter.default_type.is_none())
        .count();
    if !type_arguments.is_empty()
        && (type_arguments.len() < minimum_type_arguments
            || type_arguments.len() > type_parameters.len())
    {
        return true;
    }

    let has_spread_argument = arguments.iter().any(|argument| argument.spread);
    let fitting: Vec<&ConstructSignature> = signatures
        .iter()
        .filter(|candidate| {
            has_spread_argument || overload_arity_fits(&candidate.template, arguments.len())
        })
        .collect();
    if fitting.is_empty() {
        // `getArgumentArityError` over every signature. A count inside their
        // combined range, which no single one takes (TS2575), is left alone.
        let minimum = signatures
            .iter()
            .map(|candidate| call_site_required_count(&candidate.template))
            .min()
            .unwrap_or(0);
        let maximum = signatures
            .iter()
            .map(|candidate| candidate.template.parameters().len())
            .max()
            .unwrap_or(0);
        let variadic = signatures.iter().any(|candidate| candidate.template.is_variadic());
        if arguments.len() < minimum || (!variadic && arguments.len() > maximum) {
            let templates = signatures.iter().map(|candidate| candidate.template.clone()).collect();
            if let Some(group) = overload_group(templates) {
                let _ = check_function_type_call(
                    &group,
                    callee_span,
                    call_span,
                    type_arguments,
                    arguments,
                    None,
                    symbols,
                    ctx,
                );
            }
        }
        return true;
    }

    if violates_type_parameter_constraint(&type_parameters, type_arguments, callee_span, ctx) {
        // `chooseOverload` rejects every candidate; the arguments are still
        // checked themselves, with no signature to type a callback by.
        ctx.degraded_expected_type_depth += 1;
        for argument in arguments {
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
        }
        ctx.degraded_expected_type_depth -= 1;
        return true;
    }

    let instantiated: Vec<FunctionType> = fitting
        .iter()
        .map(|candidate| {
            with_type_copy_reason(TypeCopyReason::CallResolution, || {
                instantiate_function_type(
                    &candidate.template,
                    Some(&*candidate.signature),
                    &[],
                    type_arguments,
                    callee_span,
                    arguments,
                    None,
                    symbols,
                    ctx,
                )
                .into_owned()
            })
        })
        .collect();
    let Some(signature) = overload_group(instantiated) else {
        return true;
    };
    let _ = with_type_copy_reason(TypeCopyReason::CallResolution, || {
        check_function_type_call(
            &signature,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        )
    });
    true
}

/// `checkTypeArguments`: a written type argument outside its parameter's
/// constraint rejects every candidate, and the call reports TS2344 on it and
/// relates no argument. Only the first type argument's position is known
/// (types carry no spans), so a later one's violation is not reported. A side
/// surge cannot resolve decides nothing.
pub(crate) fn violates_type_parameter_constraint(
    type_parameters: &[ParsedTypeParameter],
    type_arguments: &[ParsedType],
    callee_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    for (index, (argument, parameter)) in type_arguments.iter().zip(type_parameters).enumerate() {
        let Some(constraint) = &parameter.constraint else {
            continue;
        };
        let checkpoint = ctx.diagnostics().len();
        let argument_type = crate::infer::map_parsed_type(argument.clone(), ctx);
        let constraint_type = crate::infer::map_parsed_type(constraint.clone(), ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        if crate::checks::function::type_contains_degradation(&argument_type)
            || crate::checks::function::type_contains_degradation(&constraint_type)
            || is_assignable_to(&argument_type, &constraint_type)
        {
            continue;
        }
        if index == 0 {
            ctx.push(diagnostic_with_syntax_span(
                crate::checks::relation_chain::with_chain(
                    Diagnostic::ts2344(
                        crate::checks::expr::source_display_name(&argument_type, &constraint_type),
                        constraint_type.name(),
                        ctx.file_name.clone(),
                    ),
                    &crate::checks::relation_chain::chain_beneath_head(&argument_type, &constraint_type),
                ),
                type_arguments_start(callee_span),
            ));
        }
        return true;
    }
    false
}
