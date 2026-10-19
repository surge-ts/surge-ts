use std::time::Instant;

use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{
    NumberLiteralType, ObjectProperty, ObjectType, PropertyMap, Type, TypeCopyReason, union_type,
};

use crate::context::CheckerContext;
use crate::infer::map_parsed_type;
use crate::program::{
    record_expression_infer, record_function_type_copy_from_expression_call_return_count,
    record_function_type_copy_from_expression_identifier_count,
    record_function_type_copy_from_expression_optional_call_return_count,
    record_object_type_clone_count, record_object_type_id_copy_count, record_program_timing,
    record_type_clone_count, record_union_type_clone_count,
    record_union_type_copy_from_expression_call_return_count,
    record_union_type_copy_from_expression_identifier_count,
    record_union_type_copy_from_expression_optional_call_return_count,
};
use crate::symbols::SymbolTable;

use super::InferredExpression;

mod access;
mod functions;
mod literals;
mod operators;

pub(crate) use access::*;
pub(crate) use functions::*;
pub(crate) use literals::*;
pub(crate) use operators::*;
/// The type of `left ?? right`. tsc expands a genuine `unknown` left operand to
/// `{} | null | undefined` before taking its non-nullable half
/// (`getAdjustedTypeWithFacts`), so `u ?? x` is `{} | x`, not `unknown`; the
/// degradation sentinel and placeholder parameters keep flowing through as-is,
/// because a modelling failure must not invent a type.
pub(crate) fn nullish_coalescing_result(left_ty: Type, right_ty: Type) -> Type {
    if left_ty == Type::Any || (left_ty.is_unknown() && left_ty != Type::GenuineUnknown) {
        return left_ty;
    }
    if matches!(left_ty, Type::Undefined | Type::Null) {
        return right_ty;
    }
    let non_nullable = non_nullable_unknown(surge_ts_types::remove_nullish(&left_ty));
    union_type(vec![non_nullable, right_ty])
}

fn non_nullable_unknown(ty: Type) -> Type {
    match ty {
        Type::GenuineUnknown => Type::Object(ObjectType::new(PropertyMap::default(), None)),
        Type::Union(union) if union.types().contains(&Type::GenuineUnknown) => union_type(
            union
                .types()
                .iter()
                .cloned()
                .map(non_nullable_unknown)
                .collect(),
        ),
        other => other,
    }
}

enum CopySource {
    Identifier,
    CallReturn,
    OptionalCallReturn,
}

/// A generic function read as a value keeps its written signature on the type,
/// as `typeof fn` does: Go's signature carries its type parameters wherever the
/// value flows, so a binding copied from it (`const h = g`) still infers them
/// at a call.
fn with_generic_declaration(ty: Type, symbol: &crate::symbols::SymbolInfo) -> Type {
    match (ty, symbol.function_signature.as_ref()) {
        (Type::Function(function), Some(signature))
            if function.declaration().is_none()
                && !signature.type_parameters.is_empty()
                && !signature.overloaded =>
        {
            let concrete: std::sync::Arc<crate::symbols::FunctionSignatureInfo> =
                std::sync::Arc::clone(signature);
            let declaration: std::sync::Arc<dyn std::any::Any + Send + Sync> = concrete;
            Type::Function(function.with_declaration(declaration))
        }
        (ty, _) => ty,
    }
}

fn clone_type_with_metrics(ty: &Type, source: CopySource) -> Type {
    record_type_clone_count();
    match ty {
        Type::Object(_) => record_object_type_clone_count(),
        Type::Union(_) => record_union_type_clone_count(),
        _ => {}
    }
    if matches!(ty, Type::Object(_)) {
        record_object_type_id_copy_count();
    }
    match (source, ty) {
        (CopySource::Identifier, Type::Function(_)) => {
            record_function_type_copy_from_expression_identifier_count();
        }
        (CopySource::Identifier, Type::Union(_)) => {
            record_union_type_copy_from_expression_identifier_count();
        }
        (CopySource::CallReturn, Type::Function(_)) => {
            record_function_type_copy_from_expression_call_return_count();
        }
        (CopySource::CallReturn, Type::Union(_)) => {
            record_union_type_copy_from_expression_call_return_count();
        }
        (CopySource::OptionalCallReturn, Type::Function(_)) => {
            record_function_type_copy_from_expression_optional_call_return_count();
        }
        (CopySource::OptionalCallReturn, Type::Union(_)) => {
            record_union_type_copy_from_expression_optional_call_return_count();
        }
        _ => {}
    }

    ty.clone()
}

/// A name or `this` read back into the type being declared, marked for the
/// API (see [`crate::semantic::mark_cycle_read`]).
pub(crate) fn marked_cycle_read(
    expression: &ParsedExpression,
    result: InferredExpression,
    ctx: &CheckerContext,
) -> InferredExpression {
    match (expression, result) {
        (ParsedExpression::Identifier { span, .. } | ParsedExpression::This { span }, InferredExpression::Known(ty)) => {
            InferredExpression::Known(crate::semantic::mark_cycle_read(*span, ty, ctx))
        }
        // An object literal's function member reads the literal through a
        // `this` this checker gives no type.
        (ParsedExpression::This { span }, InferredExpression::Unknown) => {
            match crate::semantic::mark_cycle_read(*span, Type::Unknown, ctx) {
                Type::Unknown => InferredExpression::Unknown,
                marked => InferredExpression::Known(marked),
            }
        }
        (_, result) => result,
    }
}

pub(crate) fn infer_expression(
    parsed_expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let result = crate::checks::function::settle_call_result(
        parsed_expression,
        infer_expression_unsettled(parsed_expression, symbols, ctx),
    );
    let result = marked_cycle_read(parsed_expression, result, ctx);
    if let InferredExpression::Known(ty) = &result {
        crate::semantic::record_expression_type(parsed_expression, ty, ctx);
    }
    result
}

fn infer_expression_unsettled(
    parsed_expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    record_expression_infer();
    let _written_type_arguments = crate::checks::call::WrittenTypeArguments::enter(parsed_expression);
    let infer_start = Instant::now();
    let result = match parsed_expression {
        ParsedExpression::StringLiteral(value) => {
            InferredExpression::Known(Type::StringLiteral(value.clone()))
        }
        ParsedExpression::BigIntLiteral(_) => InferredExpression::Known(Type::BigInt),
        ParsedExpression::NumberLiteral(value) => {
            InferredExpression::Known(Type::NumberLiteral(NumberLiteralType {
                value: value.clone(),
            }))
        }
        ParsedExpression::BooleanLiteral(value) => {
            InferredExpression::Known(Type::BooleanLiteral(*value))
        }
        ParsedExpression::UndefinedLiteral => InferredExpression::Known(Type::Undefined),
        ParsedExpression::NullLiteral => InferredExpression::Known(Type::Null),
        ParsedExpression::Identifier { name, span } => {
            // The module-scope value table backs a binding declared later in the
            // file or block: a function body may legally reference it because the
            // body runs after the enclosing scope is fully evaluated. It is a
            // *fallback* on a miss — except when the only hit is an ambient
            // global, which such a binding shadows (zod's `const Node = z.union(…)`
            // read as the DOM `Node` constructor).
            let resolved = match symbols.get_handle(name) {
                // Only the fallback table's *own* entries shadow: they are the
                // enclosing block's later `const`/`let`, which the language says
                // wins over a global. Its parent layers are ordinary fallbacks.
                Some(handle)
                    if ctx
                        .ambient_global_symbols
                        .get_handle(name)
                        .is_some_and(|global| std::sync::Arc::ptr_eq(&handle, &global)) =>
                {
                    ctx.module_value_fallback
                        .as_ref()
                        .and_then(|fallback| fallback.get_own_shared(name))
                        .or(Some(handle))
                }
                Some(handle) => Some(handle),
                None => ctx
                    .module_value_fallback
                    .as_ref()
                    .and_then(|fallback| fallback.get_handle(name)),
            };
            resolved
                .map(|symbol| {
                    // tsc types a read it reports as used before being assigned
                    // as the declared type, not what a guard narrowed it to.
                    let ty = crate::flow::is_unassigned_read(*span, &ctx.file_name)
                        .then(|| symbols.declared_type(name))
                        .flatten()
                        .unwrap_or(&symbol.ty);
                    InferredExpression::Known(with_generic_declaration(
                        clone_type_with_metrics(ty, CopySource::Identifier),
                        &symbol,
                    ))
                })
                .unwrap_or_else(|| InferredExpression::UnresolvedIdentifier {
                    name: name.clone(),
                    span: *span,
                })
        }
        ParsedExpression::This { span } => {
            // The containers tsc answers without types — a function
            // declaration, a namespace, an enum — are reported by the grammar
            // pass under `noImplicitThis`. This path stays withheld: an
            // object-literal accessor under a
            // contextual type reaches this through a path that does not clear
            // the flag, so zod's `const def: core.$ZodObjectDef = { get shape()
            // { … this.shape … } }` was a false positive. Re-enable with the
            // `this` binding carried on the lowered arrow itself.
            // A `function` expression whose known contextual signature gives
            // `this` no type is the one case reported here.
            if ctx.contextual_this_missing
                && ctx.options.no_implicit_this
                && let Some(span) = span
            {
                let file_name = ctx.file_name.clone();
                ctx.push(
                    surge_ts_diagnostics::Diagnostic::ts2683(file_name)
                        .with_span(crate::context::convert_span(*span)),
                );
            }
            symbols
                .get("this")
                .or_else(|| {
                    // tsc's `tryGetThisTypeAt`: a script's top level (arrows
                    // looked through) owns `this`, which is `globalThis`.
                    let start = u32::try_from(span.as_ref()?.start).ok()?;
                    ctx.global_this_starts.contains(&start).then_some(())?;
                    symbols.get("globalThis")
                })
                .map(|symbol| {
                    InferredExpression::Known(clone_type_with_metrics(
                        &symbol.ty,
                        CopySource::Identifier,
                    ))
                })
                .or_else(|| {
                    // A module's top level owns `this` too, and it is `undefined`.
                    let start = u32::try_from(span.as_ref()?.start).ok()?;
                    ctx.module_this_starts
                        .contains(&start)
                        .then_some(InferredExpression::Known(Type::Undefined))
                })
                // Anywhere else outside a class body `this` has no instance type
                // here; stay conservative rather than emitting an
                // unresolved-identifier error.
                .unwrap_or(InferredExpression::Unknown)
        }
        ParsedExpression::ObjectLiteral { properties, span } => {
            InferredExpression::Known(infer_object_literal(properties, *span, symbols, ctx))
        }
        ParsedExpression::ArrayLiteral {
            elements,
            tuple_context,
            ..
        } => infer_array_literal(elements, *tuple_context, symbols, ctx),
        ParsedExpression::Unary {
            operator, operand, ..
        } => infer_unary_expression(*operator, operand, symbols, ctx),
        ParsedExpression::Update { operand, .. } => {
            crate::checks::expr::update_result_type(&infer_expression(operand, symbols, ctx))
        }
        ParsedExpression::ArrayPatternElement {
            source,
            index,
            rest,
            tuple_literal,
            read,
            ..
        } => {
            let element = match array_pattern_source_type(source, *tuple_literal, symbols, ctx) {
                InferredExpression::Known(source_type) => crate::checks::expr::array_pattern_element_type(
                    &source_type,
                    *index,
                    *rest,
                    ctx.options.strict_builtin_iterator_return,
                ),
                _ if *rest => Some(Type::Unknown),
                _ => None,
            };
            match element {
                Some(element) => InferredExpression::Known(element),
                None => infer_expression(read, symbols, ctx),
            }
        }
        ParsedExpression::EmptyBindingPattern { source, .. } => infer_expression(source, symbols, ctx),
        ParsedExpression::PatternExcessProperties(_) => InferredExpression::Unknown,
        ParsedExpression::ObjectRest {
            source, omitted, computed, ..
        } => match infer_expression(source, symbols, ctx) {
            // A source that is not an object type is TS2700 where it is
            // checked; the binding reads as the error type.
            InferredExpression::Known(ty)
                if crate::checks::function::rest_source_validity(&ty) == Some(false) =>
            {
                InferredExpression::Known(Type::ErrorType)
            }
            InferredExpression::Known(ty) if computed.is_empty() => InferredExpression::Known(
                crate::checks::function::object_rest_type(&ty, omitted),
            ),
            InferredExpression::Known(ty) => {
                match crate::checks::function::computed_rest_omission(computed, symbols, ctx) {
                    Some(omission) => InferredExpression::Known(
                        crate::checks::function::object_rest_type_omitting(&ty, omitted, Some(&omission)),
                    ),
                    None => InferredExpression::Unknown,
                }
            }
            other => other,
        },
        // tsc's comma operator: every operand is evaluated, the value is the
        // last one's.
        ParsedExpression::Sequence { expressions } => {
            let mut result = InferredExpression::Unknown;
            for (expression, _) in expressions {
                result = infer_expression(expression, symbols, ctx);
            }
            result
        }
        ParsedExpression::Yield { .. } => InferredExpression::Known(Type::Any),
        ParsedExpression::Await { operand, .. } => match infer_expression(operand, symbols, ctx) {
            InferredExpression::Known(ty) => {
                InferredExpression::Known(crate::checks::call::awaited_type_with_alias(&ty, ctx))
            }
            other => other,
        },
        ParsedExpression::Binary {
            operator,
            left,
            right,
            ..
        } => infer_binary_expression(*operator, left, right, symbols, ctx),
        ParsedExpression::Logical {
            operator,
            left,
            right,
            ..
        } => infer_logical_expression(*operator, left, right, symbols, ctx),
        ParsedExpression::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => infer_conditional_expression(condition, when_true, when_false, symbols, ctx),
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
            ..
        } => {
            let lacking_enum_object = if *is_bracketed {
                None
            } else {
                const_enum_object_lacks_member(object, property_name, symbols, ctx)
            };
            match lacking_enum_object {
                Some(object_type) => InferredExpression::MissingProperty {
                    property_name: property_name.clone(),
                    object_type,
                    span: *property_span,
                },
                None => {
                    let inferred = infer_property_access(object, object_span, property_name, property_span, symbols, ctx);
                    match &inferred {
                        InferredExpression::Known(read) => {
                            match crate::checks::function::narrowed_tuple_length_read(object, property_name, read, symbols, ctx) {
                                Some(narrowed) => InferredExpression::Known(narrowed),
                                None => inferred,
                            }
                        }
                        _ => inferred,
                    }
                }
            }
        }
        ParsedExpression::IndexAccess {
            object_name,
            object_span,
            index,
            index_span,
        } => infer_index_access(object_name, object_span, index, index_span, symbols, ctx),
        ParsedExpression::ElementAccess {
            object,
            object_span,
            index,
            index_span,
        } => infer_element_access(object, object_span, index, index_span, symbols, ctx),
        ParsedExpression::OptionalPropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            ..
        } => infer_optional_property_access(
            object,
            object_span,
            property_name,
            property_span,
            symbols,
            ctx,
        ),
        ParsedExpression::Assignment { value, .. } => infer_expression(value, symbols, ctx),
        ParsedExpression::MemberAssignment(assignment) => {
            infer_expression(&assignment.value, symbols, ctx)
        }
        ParsedExpression::NullishCoalescing { left, right, .. } => {
            let left_type = infer_expression(left, symbols, ctx);
            let right_type = match &left_type {
                InferredExpression::Known(known) => {
                    let stripped = surge_ts_types::remove_nullish(known);
                    match crate::checks::expr::empty_object_fallback_type(right, &stripped) {
                        Some(fallback) => InferredExpression::Known(fallback),
                        None => infer_expression(right, symbols, ctx),
                    }
                }
                _ => infer_expression(right, symbols, ctx),
            };

            match (left_type, right_type) {
                (InferredExpression::Known(left_ty), InferredExpression::Known(right_ty)) => {
                    InferredExpression::Known(nullish_coalescing_result(left_ty, right_ty))
                }
                _ => InferredExpression::Unknown,
            }
        }
        ParsedExpression::SatisfiesExpression { expression, .. } => {
            infer_expression(expression, symbols, ctx)
        }
        ParsedExpression::NonNullAssertion {
            expression,
            in_optional_chain,
            ..
        } => {
            let inferred = infer_expression(expression, symbols, ctx);
            match inferred {
                InferredExpression::Known(ty) => {
                    let filtered = surge_ts_types::remove_nullish(&ty);
                    if *in_optional_chain {
                        InferredExpression::Known(surge_ts_types::union_type(vec![
                            filtered,
                            Type::Undefined,
                        ]))
                    } else {
                        InferredExpression::Known(filtered)
                    }
                }
                InferredExpression::MissingProperty { .. } | InferredExpression::Unknown => {
                    if *in_optional_chain {
                        InferredExpression::Known(Type::Undefined)
                    } else {
                        inferred
                    }
                }
                other => other,
            }
        }
        ParsedExpression::ConstAssertion { expression, .. } => {
            literals::infer_const_expression(expression, symbols, ctx)
        }
        ParsedExpression::ArrowFunction(arrow_function) => InferredExpression::Known(
            Type::Function(infer_arrow_function(arrow_function.as_ref(), symbols, ctx)),
        ),
        // Resolved where it is written, as the checking pass resolves it, so a
        // `typeof` query in it sees the enclosing locals (`k as keyof typeof o`).
        ParsedExpression::TypeAssertion {
            expression: _, ty, ..
        } => {
            let local_symbols = symbols.clone_with_reason(TypeCopyReason::ExpressionInference);
            let saved_symbols = std::mem::replace(&mut ctx.symbols, local_symbols);
            let resolved = map_parsed_type(ty.clone(), ctx);
            ctx.symbols = saved_symbols;
            InferredExpression::Known(resolved)
        }
        ParsedExpression::Call {
            callee_name,
            callee_span,
            type_arguments,
            arguments,
            ..
        } => match symbols.get(callee_name) {
            Some(symbol) => match &symbol.ty {
                Type::Function(function_type) => {
                    let return_type =
                        crate::checks::call::instantiate_function_return_type_for_call(
                            function_type,
                            symbol.function_signature.as_deref(),
                            type_arguments,
                            *callee_span,
                            arguments,
                            symbols,
                            ctx,
                        );
                    InferredExpression::Known(return_type)
                }
                Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) | Type::Any => {
                    InferredExpression::Unknown
                }
                // A value typed by an interface or alias carrying a generic call
                // signature is a `Type::Reference` here, and its answer depends on
                // binding the call's type arguments — the same recovery the
                // property spelling runs.
                // Narrow on purpose: only a callee whose *declaration* carries a
                // written generic call signature takes this path. Instantiating
                // every other callable reference here replaced a clean `unknown`
                // — which the check phase then resolves properly — with a
                // half-bound answer, and cost trpc 213 true positives.
                callee_type
                    if crate::checks::call::written_call_signature_recovery_enabled()
                        && crate::checks::call::interface_call_signature_info(callee_type, ctx)
                            .is_some() =>
                {
                    let callee_type = callee_type.clone();
                    match crate::checks::call::callable_member_call_return_type(
                        &callee_type,
                        type_arguments,
                        *callee_span,
                        arguments,
                        symbols,
                        ctx,
                    ) {
                        Some(return_type) => InferredExpression::Known(return_type),
                        None => InferredExpression::Unknown,
                    }
                }
                // A type literal's lone generic call signature is a function type
                // spelled as an object: `{ <U extends T, T extends A>(): U }`.
                Type::Object(object)
                    if object.call_signature().is_some_and(|signature| {
                        signature.overloads().is_none() && !signature.type_parameter_names().is_empty()
                    }) =>
                {
                    let Some(function_type) = object.call_signature().cloned() else {
                        return InferredExpression::Unknown;
                    };
                    InferredExpression::Known(crate::checks::call::instantiate_function_return_type_for_call(
                        &function_type,
                        symbol.function_signature.as_deref(),
                        type_arguments,
                        *callee_span,
                        arguments,
                        symbols,
                        ctx,
                    ))
                }
                // A non-generic, single call signature has one answer, whatever
                // the arguments: `f("")[0]` reads its result like any other value.
                callee_type => match callable_return_without_inference(callee_type) {
                    Some(return_type) => InferredExpression::Known(return_type),
                    None => InferredExpression::Unknown,
                },
            },
            None => InferredExpression::Unknown,
        },
        ParsedExpression::New {
            callee,
            type_arguments,
            arguments,
            ..
        } => {
            let inferred = infer_new_expression(callee, type_arguments, arguments, symbols, ctx);
            if matches!(inferred, InferredExpression::Known(Type::Any)) {
                crate::checks::call::generic_class_instance_type(
                    callee,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                )
                .map(InferredExpression::Known)
                .unwrap_or(inferred)
            } else {
                inferred
            }
        }
        ParsedExpression::PropertyCall {
            object,
            object_span,
            property_name,
            property_span,
            type_arguments,
            arguments,
            ..
        } => infer_property_call(
            object,
            object_span,
            property_name,
            property_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ),
        ParsedExpression::OptionalPropertyCall {
            object,
            property_name,
            call_optional,
            ..
        } => {
            let object_type = match infer_expression(object, symbols, ctx) {
                InferredExpression::Known(ty) => ty,
                _ => return InferredExpression::Unknown,
            };

            let base_type = surge_ts_types::remove_nullish(&object_type);
            // The chain's `undefined` only when its receiver can be nullish
            // (tsc's `getOptionalExpressionType`).
            let short_circuits = optional_chain_can_short_circuit(&object_type);

            match base_type {
                Type::Any => InferredExpression::Known(Type::Any),
                _ => match base_type.get_property_access_type(property_name) {
                    Some(property_type) => {
                        let short_circuits = short_circuits || (*call_optional && optional_chain_can_short_circuit(&property_type));
                        let prop_base = surge_ts_types::remove_nullish(&property_type);
                        if let Type::Function(function_type) = prop_base {
                            let returned = clone_type_with_metrics(
                                function_type.return_type(),
                                CopySource::OptionalCallReturn,
                            );
                            InferredExpression::Known(if short_circuits {
                                union_type(vec![returned, Type::Undefined])
                            } else {
                                returned
                            })
                        } else {
                            InferredExpression::Unknown
                        }
                    }
                    None => InferredExpression::Unknown,
                },
            }
        }
        // An IIFE and friends: the callee is an arbitrary expression, so its type
        // is inferred rather than looked up by name. Unlike `OptionalCall` the
        // result is not widened with `undefined`.
        ParsedExpression::ExpressionCall { callee, .. } => {
            match infer_expression(callee, symbols, ctx) {
                InferredExpression::Known(Type::Function(function_type)) => {
                    InferredExpression::Known(clone_type_with_metrics(
                        function_type.return_type(),
                        CopySource::CallReturn,
                    ))
                }
                InferredExpression::Known(Type::Any) => InferredExpression::Known(Type::Any),
                _ => InferredExpression::Unknown,
            }
        }
        ParsedExpression::OptionalCall { callee, .. } => {
            let callee_type = match infer_expression(callee, symbols, ctx) {
                InferredExpression::Known(ty) => ty,
                _ => return InferredExpression::Unknown,
            };

            let base_type = surge_ts_types::remove_nullish(&callee_type);
            let short_circuits = optional_chain_can_short_circuit(&callee_type);

            match base_type {
                Type::Function(function_type) => {
                    let returned =
                        clone_type_with_metrics(function_type.return_type(), CopySource::CallReturn);
                    InferredExpression::Known(if short_circuits {
                        union_type(vec![returned, Type::Undefined])
                    } else {
                        returned
                    })
                }
                Type::Any => InferredExpression::Known(Type::Any),
                _ => InferredExpression::Unknown,
            }
        }
        ParsedExpression::OptionalIndexAccess {
            object,
            object_span,
            index,
            index_span,
        } => {
            let inferred = infer_optional_index_access(object, object_span, index, index_span, symbols, ctx);
            // A guard on this exact access (`isObj(xs?.[0]) ? xs?.[0] : …`)
            // recorded the narrowed read under the access's key.
            match (&inferred, access::narrowed_element_read(object, index, symbols)) {
                (InferredExpression::Known(_), Some(narrowed)) => InferredExpression::Known(narrowed),
                _ => inferred,
            }
        }
        ParsedExpression::JsxElement { .. } | ParsedExpression::JsxFragment { .. } => {
            let fragment = matches!(parsed_expression, ParsedExpression::JsxFragment { .. });
            InferredExpression::Known(
                crate::checks::jsx::jsx_expression_type(fragment, ctx)
                    .unwrap_or_else(jsx_element_type),
            )
        }
        ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        } => {
            // Walk the interpolations so their identifier reads are observed (e.g.
            // for TS6133 use-tracking).
            for expression in expressions {
                let _ = infer_expression(expression, symbols, ctx);
            }
            InferredExpression::Known(template_literal_type(expressions, quasis, symbols, ctx))
        }
        ParsedExpression::TemplateStringsArray { .. } => {
            match ctx.lookup_type_declaration("TemplateStringsArray") {
                Some(_) => InferredExpression::Known(crate::infer::map_parsed_type(
                    surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(
                        surge_ts_syntax::ParsedNamedType {
                            type_argument_spans: Vec::new(),
                            name: "TemplateStringsArray".to_string(),
                            span: None,
                            type_arguments: Vec::new(),
                        },
                    )),
                    ctx,
                )),
                None => InferredExpression::Unknown,
            }
        }
        ParsedExpression::RegExpLiteral => match ctx.lookup_type_declaration("RegExp") {
            Some(_) => InferredExpression::Known(crate::infer::map_parsed_type(
                surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(
                    surge_ts_syntax::ParsedNamedType {
                        type_argument_spans: Vec::new(),
                        name: "RegExp".to_string(),
                        span: None,
                        type_arguments: Vec::new(),
                    },
                )),
                ctx,
            )),
            None => InferredExpression::Unknown,
        },
        ParsedExpression::ClassExpression(class_expression) => InferredExpression::Known(
            crate::program::class_expression_type(class_expression, symbols, ctx),
        ),
        ParsedExpression::Instantiation { expression, .. } => infer_expression(expression, symbols, ctx),
        ParsedExpression::ImportCall { specifier, .. } => match specifier.as_ref() {
            ParsedExpression::StringLiteral(specifier) => {
                match crate::checks::expr::import_call_module_namespace(specifier, ctx) {
                    Some(namespace) => InferredExpression::Known(crate::checks::call::promise_of(&namespace, ctx)),
                    None => InferredExpression::Unknown,
                }
            }
            _ => InferredExpression::Unknown,
        },
        ParsedExpression::PrivateName(_) | ParsedExpression::Unknown => InferredExpression::Unknown,
    };
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.type_inference += infer_start.elapsed()
    });
    result
}

/// The conservative type assigned to every JSX element and fragment. This is a
/// parser-safe stand-in for `JSX.Element`, tagged with the `Element` alias name
/// so it renders exactly as tsc does in assignability diagnostics (`Type
/// 'Element' is not assignable to type 'number'.`). It carries
/// `ReactElement<any, any>`'s members (`type`/`props`/`key`, all `any` — the
/// instantiation tsc gives JSX expressions), so a JSX value satisfies a
/// structurally-resolved `ReactElement`/`ReactNode` target instead of failing as
/// an empty object; it still fails targets that require any other member. It
/// does not resolve the `JSX` namespace.
pub(crate) fn jsx_element_type() -> Type {
    let mut properties = PropertyMap::default();
    properties.insert("type".into(), ObjectProperty::required(Type::Any));
    properties.insert("props".into(), ObjectProperty::required(Type::Any));
    properties.insert("key".into(), ObjectProperty::required(Type::Any));
    Type::Object(ObjectType::new(properties, None).with_alias_name("Element"))
}

pub(crate) fn tuple_index_value(index_type: &Type) -> Option<usize> {
    // tsc converts a numeric-like string literal key to its number, so `t["0"]`
    // selects the same element `t[0]` does.
    match index_type {
        Type::NumberLiteral(NumberLiteralType { value }) | Type::StringLiteral(value) => {
            value.parse::<usize>().ok()
        }
        _ => None,
    }
}

/// tsc's `checkTemplateExpression`: a template whose interpolations all
/// evaluate to constants is the fresh string literal of its text; any other is
/// `string` (a template-literal *type* needs a contextual type surge does not
/// model separately from `string`). Of the numbers only integer literals are
/// evaluated — the ones whose JavaScript string conversion is certain.
pub(crate) fn template_literal_type(
    expressions: &[ParsedExpression],
    quasis: &[Option<String>],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    // tsc's constant evaluator (`evaluateTemplateExpression`, `evaluateEntity`):
    // a literal, a `const` holding one, an enum member, or a template of those.
    fn constant_text(
        expression: &ParsedExpression,
        symbols: &SymbolTable,
        ctx: &mut CheckerContext,
    ) -> Option<String> {
        let literal_text = |ty: &Type| match ty.peeled() {
            Type::StringLiteral(value) => Some(value),
            Type::NumberLiteral(number) => Some(number.value),
            _ => None,
        };
        match expression {
            ParsedExpression::StringLiteral(value) => Some(value.clone()),
            ParsedExpression::NumberLiteral(value)
                if value
                    .parse::<i64>()
                    .is_ok_and(|number| number.to_string() == *value) =>
            {
                Some(value.clone())
            }
            ParsedExpression::Identifier { name, .. } => {
                let symbol = symbols.get(name)?;
                matches!(symbol.kind, crate::symbols::SymbolKind::Const)
                    .then(|| literal_text(&symbol.ty))
                    .flatten()
            }
            ParsedExpression::PropertyAccess {
                object,
                property_name,
                ..
            } => literal_text(&access::enum_member_value_type(
                object,
                property_name,
                symbols,
                ctx,
            )?),
            ParsedExpression::TemplateLiteral {
                expressions,
                quasis,
                ..
            } => match template_literal_type(expressions, quasis, symbols, ctx) {
                Type::StringLiteral(value) => Some(value),
                _ => None,
            },
            _ => None,
        }
    }
    if quasis.len() != expressions.len() + 1 {
        return Type::String;
    }
    let mut text = String::new();
    for (index, quasi) in quasis.iter().enumerate() {
        let Some(quasi) = quasi else {
            return Type::String;
        };
        text.push_str(quasi);
        if let Some(expression) = expressions.get(index) {
            match constant_text(expression, symbols, ctx) {
                Some(value) => text.push_str(&value),
                None => return Type::String,
            }
        }
    }
    Type::StringLiteral(text)
}

/// The type of an element access's argument. A template expression there is
/// in a template literal context (`isTemplateLiteralContext`), so it has a
/// template literal type over its spans' types rather than `string`
/// (`checkTemplateExpression`).
pub(crate) fn infer_element_access_argument(
    index: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let ParsedExpression::TemplateLiteral { expressions, quasis, .. } = index else {
        return infer_expression(index, symbols, ctx);
    };
    let evaluated = infer_expression(index, symbols, ctx);
    if matches!(evaluated, InferredExpression::Known(Type::StringLiteral(_))) {
        return evaluated;
    }
    match template_expression_pattern_type(expressions, quasis, symbols, ctx) {
        Some(pattern) => InferredExpression::Known(pattern),
        None => evaluated,
    }
}

fn callable_return_without_inference(callee_type: &Type) -> Option<Type> {
    let signature = match callee_type.peeled() {
        Type::Object(object) => object.call_signature()?.clone(),
        Type::Union(union) => crate::checks::call::union_call_signature(&union)?,
        _ => return None,
    };
    (signature.overloads().is_none() && signature.type_parameter_names().is_empty())
        .then(|| signature.return_type().clone())
}

/// The type of an array destructuring assignment's source. An array literal
/// the pattern contextually types as a tuple is one, of its elements' widened
/// types (`checkArrayLiteral` under a tuple-like contextual type).
pub(crate) fn array_pattern_source_type(
    source: &ParsedExpression,
    tuple_literal: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match source {
        ParsedExpression::ArrayLiteral { elements, .. } if tuple_literal && elements.iter().all(|element| !element.spread) => {
            let mut types = Vec::with_capacity(elements.len());
            for element in elements {
                match infer_expression(&element.expression, symbols, ctx) {
                    InferredExpression::Known(ty) => types.push(crate::checks::expr::widen_type(&ty)),
                    _ => return InferredExpression::Unknown,
                }
            }
            InferredExpression::Known(Type::tuple(types))
        }
        _ => infer_expression(source, symbols, ctx),
    }
}
