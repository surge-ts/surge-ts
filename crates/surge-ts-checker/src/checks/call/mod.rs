use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedCall, ParsedCallArgument, ParsedExpression, ParsedNamedType, ParsedType,
    TextSpan as SyntaxTextSpan,
};
use surge_ts_types::{
    FunctionType, Type, TypeCopyReason, UnionType, is_assignable_to, union_type,
    with_type_copy_reason,
};

use super::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type};
use crate::checks::expr::{evaluate_expression, source_display_name};
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::program::{
    DtsExpansionReason, record_call_resolution, record_program_timing, with_dts_expansion_reason,
};
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::{FunctionSignatureInfo, SymbolTable};

mod builtins;
mod bind_call_apply;
pub(crate) mod construct;
mod instantiate;
mod instantiation_expression;
pub(crate) mod property;

pub(crate) use builtins::*;
pub(crate) use bind_call_apply::{bind_call_apply_result, check_bind_call_apply};
pub(crate) use instantiate::*;
pub(crate) use instantiation_expression::*;
pub(crate) use property::*;
pub(crate) fn check_call(call: ParsedCall, ctx: &mut CheckerContext) {
    let symbols = ctx
        .symbols
        .clone_with_reason(TypeCopyReason::CallResolution);
    let _ = check_call_like(
        &call.callee_name,
        call.callee_span,
        call.span,
        &call.type_arguments,
        &call.arguments,
        &symbols,
        ctx,
    );
}

pub(crate) fn check_call_like(
    callee_name: &str,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    check_call_like_with_expected_type(
        callee_name,
        callee_span,
        call_span,
        type_arguments,
        arguments,
        None,
        symbols,
        ctx,
    )
}

/// `check_call_like` with the call's contextual type, so a type parameter that
/// occurs only in the return type (`const c: Ctor<MyZ> = make("x", (inst) => …)`)
/// can be inferred from it instead of degrading every callback parameter it types.
/// The callee binding degraded to `unknown`, but the arguments are still code:
/// a missing member or an unresolved name inside a callback body is reported by
/// tsc whatever the callee is. Implicit-any stays gated the way the
/// property-call path gates it: a binding that is `any` because its import
/// failed has no contextual type in tsc either, so its callbacks report their
/// parameters; a builder chain surge could not model does not.
fn evaluate_arguments_under_degraded_callee(
    callee_name: &str,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let genuine = symbols
        .get(callee_name)
        .is_some_and(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::ErrorImport))
        || ctx.genuine_any_bindings.contains(callee_name);
    let saved_depth = ctx.degraded_expected_type_depth;
    if genuine {
        ctx.degraded_expected_type_depth = 0;
    } else {
        ctx.degraded_expected_type_depth += 1;
    }
    for argument in arguments {
        let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
    }
    ctx.degraded_expected_type_depth = saved_depth;
}

/// `resolveUntypedCall` checks the type arguments an untyped call cannot
/// take, for what they name.
pub(super) fn check_untyped_call_type_arguments(type_arguments: &[ParsedType], ctx: &mut CheckerContext) {
    for type_argument in type_arguments {
        let _ = crate::infer::map_parsed_type(type_argument.clone(), ctx);
    }
}

pub(crate) fn check_call_like_with_expected_type(
    callee_name: &str,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_call_like_with_expected_type_unrecorded(callee_name, callee_span, call_span, type_arguments, arguments, expected_return_type, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

fn check_call_like_with_expected_type_unrecorded(
    callee_name: &str,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    record_call_resolution();
    let call_start = Instant::now();
    let _this_argument = CallThisArgument::set(Type::Void, call_span.or(callee_span));
    // A call may target a module-scope binding declared later in the file when it
    // sits inside a function body (the body runs after the module is evaluated),
    // so fall back to the module value table before reporting an unresolved name.
    let fallback_symbol = if symbols.get(callee_name).is_none() {
        ctx.module_value_fallback
            .as_ref()
            .and_then(|fallback| fallback.get(callee_name))
            .cloned()
    } else {
        None
    };
    let Some(symbol) = symbols.get(callee_name).or(fallback_symbol.as_ref()) else {
        crate::checks::expr::report_unresolved_value_name(
            callee_name,
            callee_span,
            crate::checks::expr::UnresolvedNameSite::Callee,
            symbols,
            ctx,
        );
        property::check_error_call_operands(type_arguments, arguments, symbols, ctx);
        return None;
    };

    if symbol.ty == Type::GenuineUnknown {
        let callee = ParsedExpression::Identifier {
            name: callee_name.to_string(),
            span: callee_span,
        };
        report_uncallable_unknown(&callee, callee_span, ctx);
    }
    // A binding that is plainly `undefined` here (a flow-typed `let` before
    // any assignment) is `checkNonNullType`'s invocation error, and the call
    // goes on as the error type.
    if symbol.ty == Type::Undefined {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2722(ctx.file_name.clone()),
            callee_span,
        ));
        // `resolveUntypedCall` still checks the written type arguments.
        check_untyped_call_type_arguments(type_arguments, ctx);
        evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
        return None;
    }
    // `resolveCallExpression` resolves a call on the error type as an untyped
    // call, whose result is the error type again.
    if matches!(symbol.ty, Type::ErrorType) {
        check_untyped_call_type_arguments(type_arguments, ctx);
        property::evaluate_arguments_on_error_type(arguments, symbols, ctx);
        return Some(Type::ErrorType);
    }
    // A type variable narrowed to `T & X` is called through its constraint's
    // signatures in tsc; surge does not model those, so it is called like the
    // bare variable.
    if symbol.ty.is_unknown() || surge_ts_types::type_variable::is_narrowed_type_variable(&symbol.ty) {
        evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
        return None;
    }

    // tsc's `isUntypedFunctionCall`: `Function` — the global interface or a
    // type deriving from it — is callable with any arguments though it
    // declares no call signature, and takes no type arguments (TS2347). It has
    // to be answered before the peel below turns it into a signature-less
    // object.
    if surge_ts_types::is_untyped_function_callee(&symbol.ty) {
        if !type_arguments.is_empty() {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2347(ctx.file_name.clone()),
                call_span.or(callee_span),
            ));
            check_untyped_call_type_arguments(type_arguments, ctx);
        }
        for argument in arguments {
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
        }
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.call_expression_checking += call_start.elapsed()
        });
        return Some(Type::Any);
    }

    // A callee typed by a named declaration (`declare var Number: NumberConstructor`)
    // is a nominal reference; peel it so its call/construct signature is visible.
    let callee_ty =
        with_dts_expansion_reason(DtsExpansionReason::CallResolution, || symbol.ty.peeled());
    if callee_ty.is_unknown() {
        evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
        return None;
    }

    // tsc's `resolveCall` rejects written type arguments an untyped callee
    // cannot take (TS2347) and a count outside the signature's range (TS2558),
    // anchored at the first type argument.
    if !type_arguments.is_empty() {
        if matches!(callee_ty, Type::Any) {
            // An import of a module that did not resolve is tsc's error type
            // (`resolveErrorCall`): its type arguments are checked, but the
            // call is not an untyped one.
            if !matches!(symbol.kind, crate::symbols::SymbolKind::ErrorImport) {
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts2347(ctx.file_name.clone()),
                    call_span.or(callee_span),
                ));
            }
            check_untyped_call_type_arguments(type_arguments, ctx);
            evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
            return Some(Type::Any);
        }
        // `chooseOverload` with every candidate's type parameters rejecting
        // the written type arguments: the last one is reported
        // (`candidateForTypeArgumentError`), at the first type argument.
        if let Some(signature) = symbol.function_signature.as_deref()
            && signature.overloaded
            && !signature.overload_alternatives.is_empty()
        {
            let candidates: Vec<&crate::symbols::FunctionSignatureInfo> = std::iter::once(signature)
                .chain(signature.overload_alternatives.iter().map(|alternative| &**alternative))
                .collect();
            let arity_fits = |candidate: &crate::symbols::FunctionSignatureInfo| {
                let minimum = candidate.type_parameters.iter().filter(|parameter| parameter.default_type.is_none()).count();
                type_arguments.len() >= minimum && type_arguments.len() <= candidate.type_parameters.len()
            };
            if candidates.iter().all(|candidate| arity_fits(candidate)) {
                let checkpoint = ctx.diagnostics().len();
                let mut last = None;
                let mut every_rejects = true;
                for candidate in &candidates {
                    match unsatisfied_type_argument(candidate, type_arguments, symbols, ctx) {
                        Some(pair) => last = Some(pair),
                        None => {
                            every_rejects = false;
                            break;
                        }
                    }
                }
                ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                if every_rejects && let Some((argument_type, constraint_type)) = last {
                    let first_argument = callee_span.map(|span| SyntaxTextSpan {
                        start: span.end + 1,
                        end: span.end + 1,
                    });
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2344(
                            source_display_name(&argument_type, &constraint_type),
                            constraint_type.name(),
                            ctx.file_name.clone(),
                        ),
                        first_argument,
                    ));
                    evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
                    return None;
                }
            }
        }
        if let Some(signature) = symbol.function_signature.as_deref()
            && !signature.overloaded
            && matches!(&callee_ty, Type::Function(function) if function.overloads().is_none())
        {
            let maximum = signature.type_parameters.len();
            let minimum = signature
                .type_parameters
                .iter()
                .filter(|parameter| parameter.default_type.is_none())
                .count();
            if type_arguments.len() < minimum || type_arguments.len() > maximum {
                let expected = if minimum == maximum {
                    maximum.to_string()
                } else {
                    format!("{minimum}-{maximum}")
                };
                let type_arguments_start = callee_span.map(|span| SyntaxTextSpan {
                    start: span.end + 1,
                    end: span.end + 1,
                });
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts2558(expected, type_arguments.len(), ctx.file_name.clone()),
                    type_arguments_start,
                ));
                evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
                return None;
            }
            // tsc's `chooseOverload` relates the type arguments only for a
            // candidate whose parameters fit the arguments; one that fails
            // them leaves the call unresolved (`resolveErrorCall`), so the value
            // arguments are not related to the rejected instantiation. Only the
            // first type argument's position is known (types carry no spans),
            // so the report sits there whichever argument failed.
            if let Type::Function(function) = &callee_ty
                && arguments_fit(function, arguments)
                && let Some((argument_type, constraint_type)) =
                    unsatisfied_type_argument(signature, type_arguments, symbols, ctx)
            {
                let first_argument = callee_span.map(|span| SyntaxTextSpan {
                    start: span.end + 1,
                    end: span.end + 1,
                });
                // `isRelatedTo` reports a weak-type failure as TS2559 in place of
                // the constraint message.
                let diagnostic = if surge_ts_types::has_no_common_properties(&argument_type, &constraint_type) {
                    Diagnostic::ts2559(
                        source_display_name(&argument_type, &constraint_type),
                        constraint_type.name(),
                        ctx.file_name.clone(),
                    )
                } else {
                    Diagnostic::ts2344(
                        source_display_name(&argument_type, &constraint_type),
                        constraint_type.name(),
                        ctx.file_name.clone(),
                    )
                };
                ctx.push(diagnostic_with_syntax_span(diagnostic, first_argument));
                evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
                return None;
            }
        }
    }

    let kept_signature_checked = symbol.function_signature.as_deref().is_some_and(|signature| {
        !signature.overloaded && matches!(&callee_ty, Type::Function(function) if function.overloads().is_none())
    });
    // A declared function's own declarations say how many type arguments it
    // takes; its value type may be a merge (`function f` + `namespace f`) or
    // an overload fold that no longer does.
    let arity_reported = !kept_signature_checked
        && match symbol.function_signature.as_deref() {
            Some(signature) => {
                let mut arities = vec![declared_type_argument_arity(&signature.type_parameters)];
                if signature.overloaded {
                    arities.extend(
                        signature
                            .overload_alternatives
                            .iter()
                            .map(|overload| declared_type_argument_arity(&overload.type_parameters)),
                    );
                }
                report_type_argument_arities(&arities, type_arguments, type_arguments_start(callee_span), ctx)
            }
            None => report_type_argument_arity(&callee_ty, type_arguments, type_arguments_start(callee_span), true, ctx),
        };
    if arity_reported {
        evaluate_arguments_under_degraded_callee(callee_name, arguments, symbols, ctx);
        return None;
    }

    // A generic call signature on an interface (`interface $Fetch { <T, R
    // extends ResponseType = "json">(…): … }`) was resolved with its type
    // parameters at their defaults; the written signature is read back so the
    // call infers them from its arguments instead.
    let kept_signature = symbol.function_signature.as_deref().map(|signature| {
        let candidates = overloads_fitting_call(signature, &callee_ty, arguments.len(), type_arguments.len());
        match (candidates.as_slice(), &callee_ty) {
            ([], _) => signature,
            ([only], _) => only,
            (several, Type::Function(fold)) => first_accepting_overload(
                several,
                fold,
                type_arguments,
                callee_span,
                arguments,
                expected_return_type,
                symbols,
                ctx,
            ),
            ([first, ..], _) => first,
        }
    });
    let generic_signature = kept_signature.filter(|signature| !signature.type_parameters.is_empty());
    // An overload group's written signature is its permissive fold, which
    // infers nothing a member could; `check_function_type_call` instantiates
    // each member as `chooseOverload` reaches it.
    let callee_overloaded = match &callee_ty {
        Type::Function(function) => function.overloads().is_some(),
        Type::Object(object) => object.call_signature().is_some_and(|signature| signature.overloads().is_some()),
        _ => false,
    };
    let written_signature = (generic_signature.is_none() && !callee_overloaded)
        .then(|| interface_call_signature_info(&symbol.ty, ctx))
        .flatten();
    let outer_type_arguments = written_signature
        .as_ref()
        .map(|written| written.outer_type_arguments.as_slice())
        .unwrap_or_default();
    let implementation = symbols.hidden_function_implementation(callee_name, &symbol.ty).cloned();
    let result = match &callee_ty {
        Type::Function(function_type) => {
            with_type_copy_reason(TypeCopyReason::CallResolution, || {
                let function_type = instantiate_function_type(
                    function_type,
                    generic_signature
                        .or(written_signature
                            .as_ref()
                            .map(|written| &*written.signature))
                        .or(kept_signature),
                    outer_type_arguments,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                );

                CALLEE_IMPLEMENTATION.with(|cell| *cell.borrow_mut() = implementation.clone());
                check_function_type_call(
                    function_type.as_ref(),
                    callee_span,
                    call_span,
                    type_arguments,
                    arguments,
                    None,
                    symbols,
                    ctx,
                )
            })
        }
        Type::Union(union) => check_callable_union_call(
            union,
            callee_span,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ),
        Type::Object(object_type) if object_type.call_signature().is_some() => {
            let call_signature = object_type.call_signature().unwrap();
            with_type_copy_reason(TypeCopyReason::CallResolution, || {
                // A function merged with its namespace (`drizzle` + `namespace
                // drizzle { mock }`) is an object carrying the call signature;
                // its collected generic signature still drives inference.
                let call_signature = instantiate_function_type(
                    call_signature,
                    generic_signature.or(written_signature
                        .as_ref()
                        .map(|written| &*written.signature)),
                    outer_type_arguments,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                );
                CALLEE_IMPLEMENTATION.with(|cell| *cell.borrow_mut() = implementation.clone());
                check_function_type_call(
                    call_signature.as_ref(),
                    callee_span,
                    call_span,
                    type_arguments,
                    arguments,
                    None,
                    symbols,
                    ctx,
                )
            })
        }
        // An `any`-typed callee is callable and yields `any`; still evaluate the
        // arguments so their own errors surface. (`new`-position handles this in
        // `check_new_like`; this is the symmetric call-position arm.)
        Type::Any => {
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            Some(Type::Any)
        }
        // tsc's `isUntypedFunctionCall`: no signatures, but a `Function`.
        ref other if surge_ts_types::is_untyped_function_callee(other) => {
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            Some(Type::Any)
        }
        ref other => {
            if std::env::var("SURGE_CALL_DEBUG").is_ok() {
                let shown = match other {
                    Type::Object(o) => format!(
                        "Object(call={}, ctor={}, props=[{}])",
                        o.call_signature().is_some(),
                        o.construct_signature().is_some(),
                        o.properties
                            .keys()
                            .take(6)
                            .map(|k| k.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    t => format!("{t:?}").chars().take(70).collect(),
                };
                eprintln!("[call] not callable: {shown} in {}", ctx.file_name);
            }
            // tsc's `resolveCallExpression`: a callee that can only be
            // constructed is TS2348, which names it and suggests `new`. A
            // tagged template's tag goes through `invocationError` instead.
            let is_tagged_template = arguments.first().is_some_and(|argument| {
                matches!(argument.expression, ParsedExpression::TemplateStringsArray { .. })
            });
            let diagnostic = if is_tagged_template {
                Diagnostic::ts2349(ctx.file_name.clone())
            } else {
                not_callable_diagnostic(&other, ctx)
            };
            ctx.push(diagnostic_with_syntax_span(diagnostic, callee_span));
            None
        }
    };

    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.call_expression_checking += call_start.elapsed()
    });
    result
}

/// A written generic call signature recovered from the declaration a value is
/// typed by, paired with the bindings for the declaration's *own* type
/// parameters. tRPC's `RouterBuilder<$Root>` carries
/// `<TIn extends CreateRouterOptions>(_: TIn): BuiltRouter<$Root, …>`, whose
/// annotation names both the signature's `TIn` (inferred from the call) and the
/// interface's `$Root` (fixed by the reference), so re-resolving it needs both.
pub(crate) struct WrittenCallSignature {
    pub(crate) signature: std::sync::Arc<crate::symbols::FunctionSignatureInfo>,
    pub(crate) outer_type_arguments: Vec<(String, Type)>,
}

/// The written call signature of the declaration a value is declared as — the
/// shape whose instantiation `check_call_like` otherwise never sees, because
/// `resolve_function_type` erases a signature's type parameters to the
/// degradation sentinel and keeps only their rendering. The declaration is read
/// from its declaring file's scope; a same-file declaration falls back to the
/// current lookup.
/// A/B switch for the written-call-signature recovery, so one binary measures
/// both sides on one tree. Measurement only; the recovery is on by default.
/// The written signature of a generic function type, attached to the resolved
/// handle at resolution time so a call through a member can re-instantiate it
/// (see `resolve_function_type`). `outer_type_arguments` are the bindings the
/// body was resolved under — the enclosing interface's own parameters — so a
/// signature that names them (`filter<S extends N>`) still resolves at the call.
/// `generic_shape` is the signature's written shape, when it has a
/// constrained type parameter (see `generic_signature_shape`).
pub(crate) struct DeclaredMemberSignature {
    pub(crate) signature: std::sync::Arc<crate::symbols::FunctionSignatureInfo>,
    pub(crate) outer_type_arguments: Vec<(String, Type)>,
    pub(crate) generic_shape: Option<std::sync::Arc<surge_ts_types::GenericSignatureShape>>,
}

/// A method read off `receiver` with its polymorphic `this` bound: a bare
/// `this` return is the receiver, and one that names `this` inside
/// (`ZodOptional<this>`) carries the receiver as `this` in its written
/// signature, re-resolved here when the method is not generic and at the call
/// otherwise.
pub(crate) fn bind_member_polymorphic_this(member: Type, receiver: &Type, ctx: &mut CheckerContext) -> Type {
    let member = surge_ts_types::bind_polymorphic_this_return(member, receiver);
    let Type::Function(function) = &member else {
        return member;
    };
    let mut members = Vec::new();
    function.push_overload_members(&mut members);
    if !members.iter().any(FunctionType::return_mentions_polymorphic_this) {
        return member;
    }
    let rebind = |function: &FunctionType, ctx: &mut CheckerContext| -> FunctionType {
        if !function.return_mentions_polymorphic_this() {
            return function.clone();
        }
        let Some(declared) = function
            .declaration()
            .and_then(|declaration| declaration.downcast_ref::<DeclaredMemberSignature>())
        else {
            return function.clone();
        };
        let this_name = surge_ts_syntax::POLYMORPHIC_THIS_TYPE_NAME;
        let mut outer_type_arguments: Vec<(String, Type)> = declared
            .outer_type_arguments
            .iter()
            .filter(|(name, _)| name != this_name)
            .cloned()
            .collect();
        outer_type_arguments.push((this_name.to_string(), receiver.clone()));
        let bound = std::sync::Arc::new(DeclaredMemberSignature {
            signature: declared.signature.clone(),
            outer_type_arguments: outer_type_arguments.clone(),
            generic_shape: declared.generic_shape.clone(),
        });
        let rebound = function.clone().with_declaration(bound.clone());
        if !declared.signature.type_parameters.is_empty() {
            return rebound;
        }
        let mut substitution = crate::infer::TypeParameterSubstitution::new();
        for (name, ty) in outer_type_arguments {
            substitution.insert(name, ty);
        }
        let signature = declared.signature.clone();
        instantiate_function_type_with_substitution(&rebound, &signature, &substitution, ctx)
            .into_owned()
            .with_declaration(bound)
            .with_return_mentioning_polymorphic_this(false)
    };
    let rebound = rebind(function, ctx);
    Type::Function(match function.overloads() {
        Some(overloads) => {
            let overloads = overloads.iter().map(|overload| rebind(overload, ctx)).collect();
            rebound.with_overloads(overloads)
        }
        None => rebound,
    })
}

/// The written shape a [`DeclaredMemberSignature`] carries, installed as the
/// relation's lookup (`surge_ts_types::install_generic_signature_shape_lookup`).
pub(crate) fn declared_generic_signature_shape(
    declaration: &(dyn std::any::Any + Send + Sync),
) -> Option<&surge_ts_types::GenericSignatureShape> {
    declaration.downcast_ref::<DeclaredMemberSignature>()?.generic_shape.as_deref()
}

impl DeclaredMemberSignature {
    /// `None` when an enclosing binding is still open (the body was resolved
    /// for the uninstantiated declaration, or under a placeholder): a call
    /// re-instantiating such a signature would read `Partial<C1>` with `C1`
    /// bare and report members tsc binds through the enclosing generic.
    pub(crate) fn capture(
        function_type: &surge_ts_syntax::ParsedFunctionType,
        substitution: &crate::infer::TypeParameterSubstitution,
        ctx: &CheckerContext,
    ) -> Option<Self> {
        let own: Vec<&str> = function_type
            .type_parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect();
        let merged = crate::infer::types::merged_type_parameter_substitution(ctx, substitution);
        let mut outer_type_arguments = Vec::new();
        let mut open: Vec<String> = Vec::new();
        for (name, ty) in merged.iter() {
            if own.contains(&name.as_ref()) {
                continue;
            }
            // A binding that merely degraded is still a binding: the signature's
            // own parameters infer independently of it (`query<$Output>` on a
            // builder whose `TContext` surge could not model), and whatever
            // does read it stays at the sentinel.
            // A type variable of the body being checked (`T` of the class whose
            // own method `this.idt(x)` calls) is a binding like any other,
            // even where the class's scope marks its parameter a placeholder.
            if !ty.is_type_variable() && (merged.is_placeholder(name) || matches!(ty, Type::TypeParameter(_))) {
                open.push(name.to_string());
                continue;
            }
            outer_type_arguments.push((name.to_string(), ty.clone()));
        }
        // A non-generic predicate that names none of the open bindings reads
        // the same however they end up bound (`#isNum(x: unknown): x is number`
        // on a generic class, called as `this.#isNum(v)` in its own body).
        if !open.is_empty() {
            let open: Vec<&str> = open.iter().map(String::as_str).collect();
            let ParsedType::Predicate(predicate) = function_type.return_type.as_ref() else {
                return None;
            };
            let mentions_open = predicate
                .ty
                .as_ref()
                .is_none_or(|ty| parsed_type_mentions_any(ty, &open))
                || function_type
                    .parameters
                    .iter()
                    .any(|parameter| parsed_type_mentions_any(&parameter.ty, &open));
            if !own.is_empty() || mentions_open {
                return None;
            }
        }
        let mut signature =
            (*crate::checks::function::function_type_signature_info(function_type, &ctx.file_name))
                .clone();
        signature.namespace_prefix = ctx
            .namespace_member_prefix_stack
            .last()
            .map(|prefix| std::sync::Arc::from(prefix.as_str()));
        Some(Self {
            signature: std::sync::Arc::new(signature),
            outer_type_arguments,
            generic_shape: None,
        })
    }
}

pub(crate) fn written_call_signature_recovery_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_WRITTEN_CALL_SIGNATURE").as_deref() != Ok("0"))
}

pub(crate) fn interface_call_signature_info(
    declared: &Type,
    ctx: &CheckerContext,
) -> Option<WrittenCallSignature> {
    written_call_signature_info(declared, ctx, true)
}

/// [`interface_call_signature_info`] without the generic requirement when
/// `require_generic` is false: a predicate guard reads the written signature of
/// a non-generic callable declaration too (`declare const assert: Chai.Assert`
/// declares `asserts` on its call signature, not on a function).
pub(crate) fn written_call_signature_info(
    declared: &Type,
    ctx: &CheckerContext,
    require_generic: bool,
) -> Option<WrittenCallSignature> {
    // A deferred value annotation (`let assert: typeof import("vitest")["assert"]`)
    // is a reference to the *annotation*, not to a declaration; what it resolves
    // to is the declared type this lookup wants.
    let mut declared = std::borrow::Cow::Borrowed(declared);
    for _ in 0..4 {
        let Type::Reference(reference) = declared.as_ref() else {
            break;
        };
        if !reference.id.contains("\0value-annotation\0") {
            break;
        }
        declared = std::borrow::Cow::Owned(reference.resolve());
    }
    let declared = declared.as_ref();
    if matches!(declared, Type::Reference(reference) if reference.id.contains("\0value-annotation\0"))
    {
        return None;
    }
    let legacy = !written_call_signature_recovery_enabled();
    let mut outer_type_arguments: Vec<Type> = Vec::new();
    let info = match declared {
        Type::Reference(reference) => {
            if legacy && !reference.arguments.is_empty() {
                return None;
            }
            outer_type_arguments = reference.arguments.to_vec();
            let id = reference.id.as_ref();
            let separator = id.rfind('\u{0}')?;
            let (declaring_file, declaration_name) = (&id[..separator], &id[separator + 1..]);
            // A global namespace member (`Chai.Assert`) is keyed by its
            // qualified name in the global table, not in its file's scope.
            ctx.module_scope_by_file
                .get(declaring_file)
                .and_then(|scope| scope.get(declaration_name))
                .or_else(|| ctx.lookup_type_declaration(declaration_name))
                .filter(|info| declaration_file_name(info) == declaring_file)
        }
        // A generic signature resolved from a type (`Test<number>`, a call's
        // instantiated return) carries its written signature and the bindings
        // it was resolved under. A predicate guard reads those bindings off the
        // declaration itself, so only the generic lookup answers here.
        Type::Function(function)
            if require_generic
                && let Some(member) = function
                    .declaration()
                    .and_then(|declaration| declaration.downcast_ref::<DeclaredMemberSignature>())
                && !member.signature.type_parameters.is_empty() =>
        {
            return Some(WrittenCallSignature {
                signature: member.signature.clone(),
                outer_type_arguments: member.outer_type_arguments.clone(),
            });
        }
        // An import binding carries the resolved object, named after its
        // interface; the declaration is whichever module scope declares that
        // name with a generic call signature.
        Type::Object(_) | Type::Function(_) => {
            let name = match declared {
                Type::Object(object) => object.alias_name.as_deref()?,
                Type::Function(function) => function.alias_name()?,
                _ => return None,
            };
            if name.contains(['<', ' ', '.']) {
                return None;
            }
            ctx.lookup_type_declaration(name).or_else(|| {
                ctx.module_scope_by_file
                    .values()
                    .filter_map(|scope| scope.get(name))
                    .find(|info| {
                        matches!(info, crate::symbols::TypeDeclarationInfo::Interface(info)
                            if info.body.call_signature.as_ref()
                                .is_some_and(|signature| !require_generic || !signature.type_parameters.is_empty()))
                    })
            })
        }
        _ => None,
    };
    let (parsed, declared_type_parameters, file_name) = match info? {
        crate::symbols::TypeDeclarationInfo::Interface(info) => {
            if legacy && !info.body.type_parameters.is_empty() {
                return None;
            }
            let parsed = info.body.call_signature.clone()?;
            (
                parsed,
                info.body.type_parameters.clone(),
                info.file_name.clone(),
            )
        }
        // `type Router = <T>(x: T) => T[]` publishes the same generic call
        // signature as the interface spelling and was refused outright, so a
        // value typed by it lost its type parameters on *every* path.
        crate::symbols::TypeDeclarationInfo::Alias(info) => {
            if legacy {
                return None;
            }
            let surge_ts_syntax::ParsedType::Function(parsed) = &info.body.ty else {
                return None;
            };
            (
                (**parsed).clone(),
                info.body.type_parameters.clone(),
                info.file_name.clone(),
            )
        }
    };
    if require_generic && parsed.type_parameters.is_empty() {
        return None;
    }
    // A declaration's own type parameters are bound by the reference that named
    // it; without them the annotation re-resolves `$Root` against nothing and
    // the instantiated return type degrades exactly as the uninstantiated one
    // did. An arity mismatch means the reference is not the one this declaration
    // describes, so nothing is bound rather than something wrong.
    let outer_type_arguments = if declared_type_parameters.len() == outer_type_arguments.len() {
        declared_type_parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .zip(outer_type_arguments)
            .collect()
    } else if declared_type_parameters.is_empty() {
        Vec::new()
    } else {
        return None;
    };
    Some(WrittenCallSignature {
        signature: crate::checks::function::function_type_signature_info(
            &parsed,
            file_name.as_ref(),
        ),
        outer_type_arguments,
    })
}

fn declaration_file_name(info: &crate::symbols::TypeDeclarationInfo) -> &str {
    match info {
        crate::symbols::TypeDeclarationInfo::Interface(info) => info.file_name.as_ref(),
        crate::symbols::TypeDeclarationInfo::Alias(info) => info.file_name.as_ref(),
    }
}

/// A call on a union callee, resolved against the union's own call signatures
/// (tsc's `resolveUnionTypeMembers` → `getUnionSignatures`). An unresolved
/// member already reported upstream suppresses the call cascade; a union with
/// no signatures — a member that has none, or members whose signatures do not
/// combine — is TS2349.
fn check_callable_union_call(
    union: &UnionType,
    callee_span: Option<SyntaxTextSpan>,
    callee_expression_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if union.types().iter().any(|ty| ty.is_unknown()) {
        return None;
    }

    // tsc's `resolveCallExpression` reads the callee through
    // `checkNonNullExpression`: a possibly-nullish callee is TS2721/2722/2723
    // on the whole callee, and the call proceeds on the rest.
    let null = union.types().iter().any(|ty| matches!(ty, Type::Null));
    let undefined = union.types().iter().any(|ty| matches!(ty, Type::Undefined));
    if null || undefined {
        let file_name = ctx.file_name.clone();
        let diagnostic = match (null, undefined) {
            (true, true) => Diagnostic::ts2723(file_name),
            (true, false) => Diagnostic::ts2721(file_name),
            _ => Diagnostic::ts2722(file_name),
        };
        ctx.push(diagnostic_with_syntax_span(
            diagnostic,
            callee_expression_span.or(callee_span),
        ));
        let defined = union_type(
            union
                .types()
                .iter()
                .filter(|ty| !matches!(ty, Type::Undefined | Type::Null))
                .cloned()
                .collect(),
        );
        return match &defined {
            Type::Union(defined) => check_callable_union_call(
                defined,
                callee_span,
                callee_expression_span,
                call_span,
                type_arguments,
                arguments,
                symbols,
                ctx,
            ),
            Type::Function(function_type) => check_function_type_call(
                function_type,
                callee_span,
                call_span,
                type_arguments,
                arguments,
                None,
                symbols,
                ctx,
            ),
            _ => None,
        };
    }

    let (signatures, combined) = union_signature_lists(union)
        .map(|lists| union_signatures(&lists))
        .unwrap_or_default();
    let Some(callee) = overload_group(signatures) else {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2349(ctx.file_name.clone()),
            callee_span,
        ));
        return None;
    };

    COMBINED_UNION_SIGNATURE.with(|flag| flag.set(combined));
    with_type_copy_reason(TypeCopyReason::CallResolution, || {
        check_function_type_call(
            &callee,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        )
    })
}

/// The call signature a union member contributes. A member is callable when it
/// *is* a function type or when it peels to an object carrying a call signature
/// — the shape a callable interface takes, which is how tRPC's `AnyProcedure`
/// (a union of three `Procedure<…>` interfaces) is written. Peeling is safe
/// here: this runs only when a union is actually being called.
fn union_member_call_signature(ty: &Type) -> Option<FunctionType> {
    match ty {
        Type::Function(function_type) => Some(function_type.clone()),
        _ => match ty.peeled() {
            Type::Function(function_type) => Some(function_type),
            Type::Object(object) => object.call_signature().cloned(),
            _ => None,
        },
    }
}

/// Each member's call signature list, or `None` when some member has none
/// (`getUnionSignatures` bails on the first empty list). The global `Function`
/// interface contributes tsc's `unknownSignature`.
fn union_signature_lists(union: &UnionType) -> Option<Vec<Vec<FunctionType>>> {
    union
        .types()
        .iter()
        .map(|ty| {
            if surge_ts_types::is_global_function_interface(ty) {
                return Some(vec![FunctionType::new(vec![], Type::ErrorType, false, 0)]);
            }
            let mut signatures = Vec::new();
            union_member_call_signature(ty)?.push_overload_members(&mut signatures);
            Some(signatures)
        })
        .collect()
}

/// A union type's call signatures as one callee (see [`overload_group`]), or
/// `None` when the union has none.
pub(crate) fn union_call_signature(union: &UnionType) -> Option<FunctionType> {
    overload_group(union_signatures(&union_signature_lists(union)?).0)
}

thread_local! {
    /// Set for the one `check_function_type_call` that checks a union's
    /// combined signature (`combineUnionOrIntersectionParameters`).
    static COMBINED_UNION_SIGNATURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set for the one `check_function_type_call` that checks a call to an
    /// overloaded function declaration: the implementation behind its overloads.
    static CALLEE_IMPLEMENTATION: std::cell::RefCell<Option<FunctionType>> = const { std::cell::RefCell::new(None) };
}

/// The signatures a call resolves against, as one callee: a lone signature,
/// or the overload group of several in order. `None` for no signatures.
fn overload_group(signatures: Vec<FunctionType>) -> Option<FunctionType> {
    let mut signatures = signatures.into_iter();
    let first = signatures.next()?;
    Some(signatures.fold(first, |group, signature| {
        crate::checks::function::merge_overload_group_signatures(&group, &signature)
    }))
}

/// checker.go `getUnionSignatures`: a signature present in every constituent
/// — identical, or taking a prefix of its parameters elsewhere — keeps its own
/// parameters and unions the return types. Only when none is, and at most one
/// constituent is overloaded, is each of that constituent's signatures combined
/// with the others' single one (`combineUnionOrIntersectionMemberSignatures`),
/// which the second value reports.
fn union_signatures(lists: &[Vec<FunctionType>]) -> (Vec<FunctionType>, bool) {
    let mut result: Vec<FunctionType> = Vec::new();
    let mut index_with_length_over_one = 0;
    let mut count_length_over_one = 0;
    for (index, list) in lists.iter().enumerate() {
        if list.is_empty() {
            return (Vec::new(), false);
        }
        if list.len() > 1 {
            index_with_length_over_one = index;
            count_length_over_one += 1;
        }
        for signature in list {
            if find_matching_signature(&result, signature, false, true).is_some() {
                continue;
            }
            if let Some(matches) = find_matching_signatures(lists, signature, index) {
                result.push(if matches.len() > 1 {
                    create_union_signature(signature, &matches)
                } else {
                    signature.clone()
                });
            }
        }
    }
    if !result.is_empty() || count_length_over_one > 1 {
        return (result, false);
    }
    // Overloads in several constituents would need the power set of their
    // signatures, whose order is not obvious; tsc makes no signature then.
    let mut results = lists[index_with_length_over_one].clone();
    let mut combined = false;
    for (index, list) in lists.iter().enumerate() {
        if index == index_with_length_over_one {
            continue;
        }
        let signature = &list[0];
        if !own_type_parameter_names(signature).is_empty()
            && results.iter().any(|result| {
                !own_type_parameter_names(result).is_empty()
                    && result.type_parameter_head() != signature.type_parameter_head()
            })
        {
            return (Vec::new(), false);
        }
        results = results
            .iter()
            .map(|result| combine_union_member_signatures(result, signature, combined))
            .collect();
        combined = true;
    }
    (results, true)
}

/// relater.go `findMatchingSignatures`: the signature matching `signature` in
/// every list, preferring an identical one to a partial one. A generic
/// signature must be matched exactly, and only from the first list.
fn find_matching_signatures(
    lists: &[Vec<FunctionType>],
    signature: &FunctionType,
    list_index: usize,
) -> Option<Vec<FunctionType>> {
    if !own_type_parameter_names(signature).is_empty() {
        if list_index > 0 {
            return None;
        }
        for list in &lists[1..] {
            find_matching_signature(list, signature, false, false)?;
        }
        return Some(vec![signature.clone()]);
    }
    let mut result: Vec<FunctionType> = Vec::new();
    for (index, list) in lists.iter().enumerate() {
        let matched = if index == list_index {
            signature
        } else {
            find_matching_signature(list, signature, false, true)
                .or_else(|| find_matching_signature(list, signature, true, true))?
        };
        if !result.contains(matched) {
            result.push(matched.clone());
        }
    }
    Some(result)
}

fn find_matching_signature<'a>(
    list: &'a [FunctionType],
    signature: &FunctionType,
    partial_match: bool,
    ignore_return_types: bool,
) -> Option<&'a FunctionType> {
    list.iter().find(|candidate| {
        compare_signatures_identical(candidate, signature, partial_match, ignore_return_types)
    })
}

/// relater.go `compareSignaturesIdentical`: a partial match relates the
/// target's parameter types to the source's by the subtype relation instead
/// of identity, over the target's positions only.
fn compare_signatures_identical(
    source: &FunctionType,
    target: &FunctionType,
    partial_match: bool,
    ignore_return_types: bool,
) -> bool {
    if source == target {
        return true;
    }
    if !is_matching_signature(source, target, partial_match) {
        return false;
    }
    let source_type_parameters = own_type_parameter_names(source);
    if source_type_parameters.len() != own_type_parameter_names(target).len()
        || !source_type_parameters.is_empty() && source.type_parameter_head() != target.type_parameter_head()
    {
        return false;
    }
    let compare = |source: &Type, target: &Type| {
        if partial_match {
            surge_ts_types::is_subtype_of(source, target)
        } else {
            surge_ts_types::is_type_identical_to(source, target)
        }
    };
    if let (Some(source_this), Some(target_this)) = (source.this_type(), target.this_type())
        && !compare(source_this, target_this)
    {
        return false;
    }
    (0..surge_ts_types::parameter_count(target)).all(|position| {
        compare(
            &surge_ts_types::type_at_position(target, position),
            &surge_ts_types::type_at_position(source, position),
        )
    }) && (ignore_return_types || compare(source.return_type(), target.return_type()))
}

/// `combineUnionThisParam`: the intersection of the `this` types the combined
/// signatures write.
fn combined_this_type<'a>(signatures: impl IntoIterator<Item = &'a FunctionType>) -> (Option<Type>, bool) {
    let mut this_types = Vec::new();
    let mut polymorphic = false;
    for signature in signatures {
        if let Some(this_type) = signature.this_type() {
            if !this_types.contains(this_type) {
                this_types.push(this_type.clone());
            }
            polymorphic |= signature.has_polymorphic_this();
        }
    }
    let this_type = match this_types.len() {
        0 => None,
        1 => this_types.pop(),
        _ => Some(crate::infer::types::merge_intersection_members(this_types)),
    };
    (this_type, polymorphic)
}

/// relater.go `isMatchingSignature`: the same number of required, optional
/// and rest parameters — or, partially, no more required ones.
fn is_matching_signature(source: &FunctionType, target: &FunctionType, partial_match: bool) -> bool {
    let source_minimum = surge_ts_types::min_argument_count(source);
    let target_minimum = surge_ts_types::min_argument_count(target);
    surge_ts_types::parameter_count(source) == surge_ts_types::parameter_count(target)
        && source_minimum == target_minimum
        && surge_ts_types::has_effective_rest_parameter(source)
            == surge_ts_types::has_effective_rest_parameter(target)
        || partial_match && source_minimum <= target_minimum
}

/// checker.go `createUnionSignature`: `signature`'s parameters, returning the
/// subtype-reduced union of what the matched signatures return.
fn create_union_signature(signature: &FunctionType, matches: &[FunctionType]) -> FunctionType {
    let return_type = surge_ts_types::subtype_reduced_union(
        matches
            .iter()
            .map(|matched| (matched.return_type().clone(), surge_ts_types::LiteralShape::Regular))
            .collect(),
    );
    let mut union_signature = FunctionType::new(
        signature.parameters().to_vec(),
        return_type,
        signature.is_variadic(),
        signature.required_parameter_count(),
    )
    .with_type_parameter_head(signature.type_parameter_head().map(str::to_string));
    if let Some(names) = signature.parameter_names() {
        union_signature = union_signature.with_parameter_names(names.to_vec());
    }
    let (this_type, polymorphic) = combined_this_type(matches);
    union_signature.with_this_type(this_type).with_polymorphic_this(polymorphic)
}

/// checker.go `combineUnionOrIntersectionMemberSignatures` for a union: the
/// combined parameters, the larger minimum, and the union of the returns.
/// `left_combined` says `left` is itself such a combination.
fn combine_union_member_signatures(left: &FunctionType, right: &FunctionType, left_combined: bool) -> FunctionType {
    let (parameters, names, has_rest) = combine_union_parameters(left, right, left_combined);
    let (this_type, polymorphic_this) = combined_this_type([left, right]);
    let return_type = surge_ts_types::subtype_reduced_union(vec![
        (left.return_type().clone(), surge_ts_types::LiteralShape::Regular),
        (right.return_type().clone(), surge_ts_types::LiteralShape::Regular),
    ]);
    FunctionType::new(
        parameters,
        return_type,
        has_rest,
        left.required_parameter_count().max(right.required_parameter_count()),
    )
    .with_type_parameter_head(
        left.type_parameter_head()
            .or(right.type_parameter_head())
            .map(str::to_string),
    )
    .with_parameter_names(names)
    .with_this_type(this_type)
    .with_polymorphic_this(polymorphic_this)
}

/// checker.go `combineUnionOrIntersectionParameters` for a union: position by
/// position the intersection of what the longer and the shorter signature take
/// there (a rest parameter's element past its fixed parameters, `unknown` past
/// the shorter one's end), a rest parameter when either has one, and an extra
/// `...args` when only the shorter does.
fn combine_union_parameters(
    left: &FunctionType,
    right: &FunctionType,
    left_combined: bool,
) -> (Vec<Type>, Vec<Option<std::sync::Arc<str>>>, bool) {
    combine_member_parameters(left, right, true, left_combined)
}

/// checker.go `getIntersectedSignatures`: the contextual signature an
/// intersection of callable types gives a function expression, under
/// `noImplicitAny` only — each position takes the union of what the members
/// take there. `None` when the members' type parameters differ.
pub(crate) fn intersected_contextual_signature(signatures: &[FunctionType]) -> Option<FunctionType> {
    let (first, rest) = signatures.split_first()?;
    let mut combined = first.clone();
    for signature in rest {
        if combined.type_parameter_head() != signature.type_parameter_head() {
            return None;
        }
        let (parameters, names, has_rest) = combine_member_parameters(&combined, signature, false, false);
        combined = FunctionType::new(
            parameters,
            combined.return_type().clone(),
            has_rest,
            combined.required_parameter_count().max(signature.required_parameter_count()),
        )
        .with_type_parameter_head(combined.type_parameter_head().map(str::to_string))
        .with_parameter_names(names);
    }
    Some(combined)
}

fn combine_member_parameters(
    left: &FunctionType,
    right: &FunctionType,
    is_union: bool,
    left_combined: bool,
) -> (Vec<Type>, Vec<Option<std::sync::Arc<str>>>, bool) {
    let left_count = surge_ts_types::parameter_count(left);
    let right_count = surge_ts_types::parameter_count(right);
    let (longest_count, longest, shorter, longest_combined, shorter_combined) = if left_count >= right_count {
        (left_count, left, right, left_combined, false)
    } else {
        (right_count, right, left, false, left_combined)
    };
    // A combination's parameters are synthetic and carry no optionality of
    // their own; only a declared signature's optional position adds
    // `undefined` (`getTypeOfParameter`).
    let type_at = |signature: &FunctionType, combined: bool, position: usize| {
        let fixed = signature.parameters().len() - usize::from(signature.is_variadic());
        if combined && position < fixed {
            signature.parameters().get(position).cloned()
        } else {
            surge_ts_types::try_type_at_position(signature, position)
        }
    };
    let either_has_rest = surge_ts_types::has_effective_rest_parameter(left)
        || surge_ts_types::has_effective_rest_parameter(right);
    let needs_extra_rest_element =
        either_has_rest && !surge_ts_types::has_effective_rest_parameter(longest);
    let mut parameters = Vec::with_capacity(longest_count + 1);
    let mut names = Vec::with_capacity(longest_count + 1);
    for position in 0..longest_count {
        let longest_type = type_at(longest, longest_combined, position).unwrap_or(Type::Any);
        let shorter_type = type_at(shorter, shorter_combined, position).unwrap_or(Type::GenuineUnknown);
        let combined = if is_union {
            crate::infer::types::merge_intersection_members(vec![longest_type, shorter_type])
        } else {
            union_type(vec![longest_type, shorter_type])
        };
        let is_rest = either_has_rest && !needs_extra_rest_element && position == longest_count - 1;
        parameters.push(if is_rest {
            Type::Array(Box::new(combined))
        } else {
            combined
        });
        let left_name = (position < left_count)
            .then(|| parameter_name_at_position(left, position))
            .flatten();
        let right_name = (position < right_count)
            .then(|| parameter_name_at_position(right, position))
            .flatten();
        let name = match (left_name, right_name) {
            (Some(left), Some(right)) if left == right => Some(left),
            (Some(_), Some(_)) => None,
            (left, right) => left.or(right),
        };
        names.push(name.or_else(|| Some(format!("arg{position}").into())));
    }
    if needs_extra_rest_element {
        parameters.push(Type::Array(Box::new(surge_ts_types::type_at_position(
            shorter,
            longest_count,
        ))));
        names.push(Some("args".into()));
    }
    (parameters, names, either_has_rest)
}

/// relater.go `getParameterNameAtPosition`: a position past the fixed
/// parameters is named by the rest parameter.
fn parameter_name_at_position(signature: &FunctionType, position: usize) -> Option<std::sync::Arc<str>> {
    let names = signature.parameter_names()?;
    let index = position.min(names.len().checked_sub(1)?);
    names.get(index).cloned().flatten()
}

/// Whether `new` targets a generic class's own static side
/// (`generic_class_value_symbol`), whose construct signature only yields
/// `C<any>`.
fn is_generic_class_value(callee: &ParsedExpression, ty: &Type, ctx: &CheckerContext) -> bool {
    let ParsedExpression::Identifier { name, .. } = callee else {
        return false;
    };
    let Type::Object(object) = ty else {
        return false;
    };
    object
        .alias_name
        .as_deref()
        .is_some_and(|alias| alias.starts_with("typeof "))
        && matches!(
            ctx.lookup_type_declaration(name),
            Some(crate::symbols::TypeDeclarationInfo::Interface(info))
                if !info.body.type_parameters.is_empty()
        )
}

pub(crate) fn check_new_like(
    callee: &ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_new_like_unrecorded(callee, callee_span, call_span, type_arguments, arguments, expected_type, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

fn check_new_like_unrecorded(
    callee: &ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // `resolveNewExpression`: an `any` callee is an untyped call, which takes
    // no type arguments (TS2347 on the expression). Parameters are left out:
    // an annotation surge cannot resolve (`C: typeof Imported`) also binds
    // them to `any`.
    if !type_arguments.is_empty()
        && let ParsedExpression::Identifier { name, .. } = callee
        && symbols.get(name).is_some_and(|symbol| {
            symbol.ty == Type::Any
                && matches!(symbol.kind, crate::symbols::SymbolKind::Var | crate::symbols::SymbolKind::Let)
        })
        && ctx.lookup_type_declaration(name).is_none()
    {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2347(ctx.file_name.clone()),
            call_span.or(callee_span),
        ));
    }
    // Written type arguments on `new C<…>()` must fit the class's type
    // parameters (TS2558). Only a class declared in a source file is checked:
    // a library constructor's generics need not match its instance interface.
    if !type_arguments.is_empty()
        && let ParsedExpression::Identifier { name, .. } = callee
        && let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) =
            ctx.lookup_type_declaration(name)
        && !crate::modules::is_declaration_file_name(&info.file_name)
        && symbols.get(name).is_some_and(|symbol| match &symbol.ty {
            Type::Any => true,
            Type::Object(object) => object.construct_signature().is_some(),
            _ => false,
        })
    {
        let parameters = &info.body.type_parameters;
        let maximum = parameters.len();
        let minimum = parameters
            .iter()
            .filter(|parameter| parameter.default_type.is_none())
            .count();
        if type_arguments.len() < minimum || type_arguments.len() > maximum {
            let expected = if minimum == maximum {
                maximum.to_string()
            } else {
                format!("{minimum}-{maximum}")
            };
            let type_arguments_start = callee_span.map(|span| SyntaxTextSpan {
                start: span.end + 1,
                end: span.end + 1,
            });
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2558(expected, type_arguments.len(), ctx.file_name.clone()),
                type_arguments_start,
            ));
        }
    }

    // tsc resolves the *value* and looks at its construct signatures. This
    // lookup is by name over the type table, so a binding that shadows the
    // class — zod's `partial(Class: SchemaClass<…>, …)` beside its own
    // `export abstract class Class` — otherwise reported every `new Class()`
    // in the function as an abstract instantiation. A binding that can hold
    // any value no longer denotes the declaration; a `const` does (that is
    // what a class declaration itself binds).
    let class_info = match callee {
        ParsedExpression::Identifier { name, .. }
            if !matches!(
                symbols.get(name).map(|symbol| symbol.kind),
                Some(
                    crate::symbols::SymbolKind::Parameter
                        | crate::symbols::SymbolKind::Let
                        | crate::symbols::SymbolKind::Var
                )
            ) =>
        {
            match ctx.lookup_type_declaration(name) {
                Some(crate::symbols::TypeDeclarationInfo::Interface(info))
                    if info.is_class_instance =>
                {
                    Some(info.clone())
                }
                _ => None,
            }
        }
        _ => None,
    };
    let mut resolution_stopped = false;
    if let Some(info) = &class_info {
        // `resolveNewExpression` checks the constructor's modifier first, then
        // whether the class is abstract; each stops the resolution. tsc
        // underlines the whole `new` expression for both.
        if let Some((declaring, accessibility)) =
            crate::checks::expr::constructor_accessibility_error(info, true, ctx)
        {
            let class_name = declaring
                .declared_name
                .as_deref()
                .unwrap_or(&declaring.name)
                .to_string();
            let diagnostic = match accessibility {
                surge_ts_syntax::ParsedMemberAccessibility::Private => {
                    Diagnostic::ts2673(class_name, ctx.file_name.clone())
                }
                surge_ts_syntax::ParsedMemberAccessibility::Protected => {
                    Diagnostic::ts2674(class_name, ctx.file_name.clone())
                }
            };
            ctx.push(diagnostic_with_syntax_span(diagnostic, call_span.or(callee_span)));
            resolution_stopped = true;
        } else if info.is_abstract_class {
            // An `abstract class` has a construct signature like any other
            // class — the instance type is still what `new` produces, and tsc
            // reports the instantiation without cascading.
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2511(ctx.file_name.clone()),
                call_span.or(callee_span),
            ));
            resolution_stopped = true;
        } else if !type_arguments.is_empty()
            && let Some((argument_type, constraint_type)) = unsatisfied_class_type_argument(
                info,
                callee,
                type_arguments,
                arguments,
                symbols,
                ctx,
            )
        {
            // `resolveCall` then relates the written type arguments; a failure
            // leaves the expression unresolved, its arguments unrelated.
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2344(
                    source_display_name(&argument_type, &constraint_type),
                    constraint_type.name(),
                    ctx.file_name.clone(),
                ),
                callee_span.map(|span| SyntaxTextSpan {
                    start: span.end + 1,
                    end: span.end + 1,
                }),
            ));
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            return None;
        }
    }

    // Physical-lib mode: `new Foo<Args>()` produces an instance of the `Foo`
    // interface (the instance interface shares the constructor's name, e.g.
    // `Map<K, V>`, `Date`, `URL`, `Response`). Prefer resolving the real
    // interface instance over the hardcoded builtin fast-path so that lib
    // methods and properties carry meaningful types. Gated to interfaces
    // declared in physical default-lib files, so generated/default mode keeps
    // the existing builtin behaviour.
    if let ParsedExpression::Identifier { name, .. } = callee {
        let physical_interface_arity = match ctx.lookup_type_declaration(name) {
            Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => {
                if crate::default_lib::is_physical_default_lib_file_name(&info.file_name) {
                    Some(info.body.type_parameters.len())
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(arity) = physical_interface_arity {
            // The constructor value (e.g. `Promise` typed `PromiseConstructor`)
            // carries the construct signature. Keep its full type so we can re-read
            // the declaring constructor-interface's generic construct signature.
            let constructor_type = match evaluate_expression(callee, callee_span, symbols, ctx) {
                InferredExpression::Known(ty) => Some(ty),
                _ => None,
            };
            let construct_signature = constructor_type.as_ref().and_then(|ty| match ty.peeled() {
                Type::Object(object) => object.construct_signature().cloned(),
                _ => None,
            });
            // A lib value with only call signatures (`Symbol`) is no
            // constructor, whatever interface shares its name.
            if construct_signature.is_none()
                && let Some(Type::Object(object)) = constructor_type.as_ref().map(Type::peeled)
                && let Some(call_signature) = object.call_signature()
            {
                return new_with_call_signature(
                    call_signature,
                    callee,
                    callee_span,
                    call_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                );
            }
            // `resolveCall` reports written type arguments no construct
            // signature takes (TS2558), and relates no argument to them.
            let type_arity_reported = !type_arguments.is_empty()
                && construct_signature.as_ref().is_some_and(|signature| {
                    report_type_argument_arity(
                        &Type::Function(signature.clone()),
                        type_arguments,
                        type_arguments_start(callee_span),
                        false,
                        ctx,
                    )
                });

            // Resolve the constructor's type arguments: explicit `new Promise<void>()`
            // first, else infer from a contextual expected type that is a reference
            // to this same interface (`const p: Promise<void> = new Promise(...)`).
            // These pin the generic construct signature so the executor's callback
            // parameters are typed (`resolve: (value: T | PromiseLike<T>) => void`
            // becomes `... void | PromiseLike<void> ...`, making `resolve()` valid).
            // Explicit type arguments name things visible at the CALL, including
            // function locals and parameters (`new Promise<typeof opts.input>`);
            // `ctx.symbols` is the file-level table and does not hold them, so
            // resolving against it reported the local as an unresolved name.
            let explicit_args: Option<Vec<Type>> = if !type_arguments.is_empty() {
                let saved_symbols = std::mem::replace(
                    &mut ctx.symbols,
                    symbols.clone_with_reason(TypeCopyReason::CallResolution),
                );
                let resolved = type_arguments
                    .iter()
                    .map(|argument| crate::infer::map_parsed_type(argument.clone(), ctx))
                    .collect();
                ctx.symbols = saved_symbols;
                Some(resolved)
            } else {
                None
            };
            let contextual_instance = if explicit_args.is_none() && arity > 0 {
                expected_type
                    .and_then(|expected| contextual_instance_reference(name, arity, expected))
            } else {
                None
            };
            let substitution_args = explicit_args.clone().or_else(|| {
                contextual_instance
                    .as_ref()
                    .map(|(_, arguments)| arguments.clone())
            });

            if type_arity_reported {
                ctx.degraded_expected_type_depth += 1;
                for argument in arguments {
                    let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                }
                ctx.degraded_expected_type_depth -= 1;
                return None;
            }

            // Check the arguments against the construct signature so callback
            // parameters get contextual types instead of collapsing to implicit
            // `any`. When no construct signature is reachable, evaluate bare.
            let resolved = if let Some(construct_signature) = construct_signature {
                let substituted = substitution_args.as_ref().and_then(|args| {
                    constructor_type.as_ref().and_then(|ctor_ty| {
                        substituted_construct_signature(ctor_ty, &construct_signature, args, ctx)
                    })
                });
                // `resolveCall` infers a generic construct signature's own type
                // parameters from the arguments before relating them, and over
                // overloads (`SetConstructor`'s `values?: readonly T[] | null`
                // and `iterable?: Iterable<T> | null`) takes the first member
                // the arguments fit.
                let inferred = if substituted.is_some() {
                    None
                } else if construct_signature.overloads().is_some() {
                    // A contextual type would keep the literals tsc infers
                    // under it (`ReadonlySet<"a" | "b"> = new Set(["a"])`),
                    // which the argument-only selection widens.
                    (type_arguments.is_empty() && !arguments.is_empty() && expected_type.is_none())
                        .then(|| select_overload_member_for_inferred_call(&construct_signature, arguments, symbols, ctx))
                        .flatten()
                } else {
                    (!own_type_parameter_names(&construct_signature).is_empty()).then(|| {
                        instantiate_overload_member(
                            &construct_signature,
                            type_arguments,
                            callee_span,
                            arguments,
                            None,
                            symbols,
                            ctx,
                        )
                    })
                };
                let effective_signature =
                    substituted.as_ref().or(inferred.as_ref()).unwrap_or(&construct_signature);
                instantiate::with_constructed_under_context(expected_type, || {
                    with_type_copy_reason(TypeCopyReason::CallResolution, || {
                        check_function_type_call(
                            effective_signature,
                            callee_span,
                            call_span,
                            type_arguments,
                            arguments,
                            None,
                            symbols,
                            ctx,
                        )
                    })
                })
            } else {
                for argument in arguments {
                    let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                }
                None
            };

            // The contextual expected reference IS the instance type when present.
            if let Some((instance, _)) = contextual_instance {
                return Some(instance);
            }

            // `resolveNewExpression` yields the return type of the construct
            // signature `resolveCall` picked: `new Int8Array(4)` is
            // `Int8Array<ArrayBuffer>`, not an instance rebuilt by name.
            if let Some(resolved) = resolved.filter(is_resolved_construct_instance) {
                return Some(resolved);
            }

            // When resolution did not produce the instance, build the instance
            // interface type (`Map<K, V>`) by name so lib methods carry meaningful
            // types. With explicit type arguments use them; otherwise default each
            // missing argument to `any` — a bare `Set<>` would trip the
            // generic-arity TS2314, while `Set<any>` stays assignable to whatever
            // the use site expects.
            let type_arguments = if type_arguments.is_empty() && arity > 0 {
                vec![ParsedType::Any; arity]
            } else {
                type_arguments.to_vec()
            };
            let named = ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: name.clone(),
                span: None,
                type_arguments,
            }));
            let saved_symbols = std::mem::replace(
                &mut ctx.symbols,
                symbols.clone_with_reason(TypeCopyReason::CallResolution),
            );
            let instance = crate::infer::map_parsed_type(named, ctx);
            ctx.symbols = saved_symbols;
            return Some(instance);
        }
    }

    // The fast path stands in for a lib constructor, so it needs the lib to
    // declare one: `new Map()` under `lib: ["es5"]` is an unresolved name.
    if let ParsedExpression::Identifier { name, .. } = callee
        && let Some(result_type) = surge_ts_types::Type::builtin_constructor_result_type(name)
        && (symbols.get(name).is_some()
            || ctx.symbols.get(name).is_some()
            || ctx.ambient_global_symbols.get(name).is_some())
    {
        for argument in arguments {
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
        }
        return Some(result_type);
    }

    let callee_result = evaluate_expression(callee, callee_span, symbols, ctx);
    let callee_type = match callee_result {
        // A constructor value is often a nominal reference (`declare const P:
        // PromiseConstructor`); peel it so its construct signature is visible.
        InferredExpression::Known(ty) => {
            // `resolveNewExpression` reads the target through `checkNonNullExpression`.
            let ty = ty.peeled();
            match crate::checks::expr::check_non_null_operand(callee, &ty, callee_span, ctx) {
                Some(non_null) => non_null.peeled(),
                None => ty,
            }
        }
        // The constructor target is unresolved (e.g. `new Missing(...)`). The
        // missing-name diagnostic is already reported; still evaluate the
        // arguments so their own errors surface, but do not cascade a result.
        _ => {
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            return None;
        }
    };
    let callee_type = if is_generic_class_value(callee, &callee_type, ctx) {
        Type::Any
    } else {
        callee_type
    };
    // `getNarrowableTypeForReference`: a callee a guard narrowed out of a type
    // variable's union constraint (`T & (new () => {})`) is what is left of it.
    let callee_type = match crate::checks::function::union_constrained_variable(&callee_type) {
        Some((_, narrowed)) if !matches!(callee_type, Type::TypeParameter(_)) => narrowed,
        _ => callee_type,
    };

    match callee_type {
        // A function type has a call signature and no construct signature (a
        // constructor type is an object carrying one), so tsc resolves the call
        // signature and types the `new` as `any`: TS7009 under `noImplicitAny`,
        // otherwise TS2350 unless the function returns `void`. A callable object
        // with no construct signature (a function merged with a namespace) is
        // resolved the same way (`resolveNewExpression`).
        Type::Function(function_type) => new_with_call_signature(
            &function_type,
            callee,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ),
        Type::Object(object)
            if object.construct_signature().is_none() && object.call_signature().is_some() =>
        {
            let function_type = object.call_signature().expect("call signature present").clone();
            new_with_call_signature(
                &function_type,
                callee,
                callee_span,
                call_span,
                type_arguments,
                arguments,
                symbols,
                ctx,
            )
        }
        // A class value (static side) carries a construct signature. Check the
        // constructor arguments against it and yield the instance type.
        Type::Object(object) if object.construct_signature().is_some() => {
            let construct_signature = object
                .construct_signature()
                .expect("construct signature present")
                .clone();
            // `resolveNewExpression`: an abstract construct signature cannot be
            // instantiated, whatever names the value. A stopped resolution is
            // `resolveErrorCall`: the arguments are checked and nothing else,
            // though tsgo 7.0.2 still reads the expression as the instance.
            // A class named directly was judged by its declaration above: the
            // value a duplicate class binds is the later declaration's, while
            // the symbol tsc reads is the first's.
            if !resolution_stopped
                && class_info.is_none()
                && construct_signature.construct_modifiers().is_abstract()
            {
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts2511(ctx.file_name.clone()),
                    call_span.or(callee_span),
                ));
                resolution_stopped = true;
            }
            if resolution_stopped {
                property::evaluate_arguments_on_error_type(arguments, symbols, ctx);
                return Some(construct_signature.return_type().clone());
            }
            if !type_arguments.is_empty()
                && report_type_argument_arity(
                    &Type::Function(construct_signature.clone()),
                    type_arguments,
                    type_arguments_start(callee_span),
                    false,
                    ctx,
                )
            {
                ctx.degraded_expected_type_depth += 1;
                for argument in arguments {
                    let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                }
                ctx.degraded_expected_type_depth -= 1;
                return None;
            }
            // A generic construct signature of a constructor type
            // (`new <T>(n: T)`) binds its type arguments before the arguments
            // are related: the written ones, or the ones its arguments infer.
            // Inferring is left to plain arguments: tsc infers a nested
            // `new`/call against the outer inference's partial mapper
            // (`new Proxy(target, new Handler())`) and a callback after the
            // rest (`new Promise((resolve) => …)`), which surge's per-argument
            // inference does not reproduce.
            let inferable = arguments
                .iter()
                .all(|argument| !holds_callback_or_nested_call(&argument.expression));
            let instantiated = (class_info.is_none()
                && (!type_arguments.is_empty() || inferable)
                && construct_signature.overloads().is_none()
                && !own_type_parameter_names(&construct_signature).is_empty())
            .then(|| {
                instantiate_overload_member(
                    &construct_signature,
                    type_arguments,
                    callee_span,
                    arguments,
                    None,
                    symbols,
                    ctx,
                )
            });
            check_function_type_call(
                instantiated.as_ref().unwrap_or(&construct_signature),
                callee_span,
                call_span,
                type_arguments,
                arguments,
                None,
                symbols,
                ctx,
            )
        }
        // `new (Custom ?? Default)(…)`: a union whose every member is
        // constructable is resolved against tsc's `getUnionSignatures` over the
        // members' construct signatures, as a call on a union is against their
        // call signatures.
        Type::Union(union)
            if union
                .types()
                .iter()
                .all(|member| construct_signature_of(member).is_some()) =>
        {
            let lists: Vec<Vec<FunctionType>> = union
                .types()
                .iter()
                .filter_map(construct_signature_of)
                .map(|signature| {
                    let mut members = Vec::new();
                    signature.push_overload_members(&mut members);
                    members
                })
                .collect();
            let (signatures, combined) = union_signatures(&lists);
            let Some(signature) = overload_group(signatures) else {
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts2351(ctx.file_name.clone()),
                    callee_span,
                ));
                return None;
            };
            COMBINED_UNION_SIGNATURE.with(|flag| flag.set(combined));
            with_type_copy_reason(TypeCopyReason::CallResolution, || {
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
            })
        }
        // A generic class's value side constructs `C<any>`, and an `any` callee
        // would open the whole instance. The class's *type* side is exact, so
        // build the instance by name the way an annotation does, inferring the
        // type arguments; the arguments are checked against the construct
        // signatures the value carries.
        Type::Any => instantiate::with_constructed_under_context(expected_type, || {
            // Any other `any` callee is an untyped call (`resolveUntypedCall`):
            // its arguments are still checked, with no contextual type.
            if !resolution_stopped
                && !construct::check_generic_class_construct(
                    callee,
                    callee_span,
                    call_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                )
            {
                property::evaluate_arguments_context_free(callee, arguments, symbols, ctx);
            }
            generic_class_instance_type(callee, type_arguments, arguments, symbols, ctx)
                .or(Some(Type::Any))
        }),
        // `resolveNewExpression` resolves an error-typed target as an error
        // call: the arguments are checked and nothing more is reported.
        Type::ErrorType => {
            property::evaluate_arguments_on_error_type(arguments, symbols, ctx);
            Some(Type::ErrorType)
        }
        // `checkNonNullType` has reported `unknown` under `strictNullChecks`;
        // without it `unknown` is simply not constructable.
        Type::GenuineUnknown => {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2351(ctx.file_name.clone()),
                callee_span,
            ));
            None
        }
        Type::Unknown | Type::TypeParameter(_) => None,
        // A generic class merged with a namespace (`class SQL` +
        // `namespace SQL { class Aliased }`) has the namespace object for a
        // value: the class half contributed the `any` above, which the merge
        // drops, so the members survive and the construct signature does not.
        // The class's *type* side is exact either way, so the instance is built
        // by name exactly as the `any` arm does — reporting here instead called
        // `new SQL(...)` unconstructable in the module that declares it.
        Type::Object(ref object) if object.construct_signature().is_none() => {
            match generic_class_instance_type(callee, type_arguments, arguments, symbols, ctx) {
                Some(instance) => Some(instance),
                None => {
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2351(ctx.file_name.clone()),
                        callee_span,
                    ));
                    None
                }
            }
        }
        _ => {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2351(ctx.file_name.clone()),
                callee_span,
            ));
            None
        }
    }
}

/// The instance type of `new C<Args>()` where `C` is a generic class. A type
/// argument the call does not write takes the declared default, else the
/// declared constraint, else `unknown` — the order tsc resolves them in, and the
/// reason `new TRPCBuilder()` against `<TContext extends object, TMeta extends
/// object>` is `TRPCBuilder<object, object>` rather than an error.
/// Whether a written type mentions any of `names`, used to detect a type
/// parameter default or constraint written in terms of its siblings. Unhandled
/// composite forms answer `true`: the caller treats a mention as "do not
/// synthesize", so an unknown shape stays on the conservative side.
pub(crate) fn parsed_type_mentions_any(ty: &ParsedType, names: &[&str]) -> bool {
    match ty {
        ParsedType::Named(named) => {
            names.contains(&named.name.as_str())
                || named
                    .type_arguments
                    .iter()
                    .any(|argument| parsed_type_mentions_any(argument, names))
        }
        ParsedType::Array(inner) | ParsedType::KeyOf(inner) => {
            parsed_type_mentions_any(inner, names)
        }
        ParsedType::Tuple(types) | ParsedType::Union(types) | ParsedType::Intersection(types) => {
            types
                .iter()
                .any(|member| parsed_type_mentions_any(member, names))
        }
        // The `object` keyword lowers to an empty object type; a written object
        // type mentions a name only through its members.
        ParsedType::Object(object) => {
            object.call_signature.is_some()
                || object.construct_signature.is_some()
                || object
                    .string_index_type
                    .as_ref()
                    .is_some_and(|index| parsed_type_mentions_any(index, names))
                || object
                    .properties
                    .iter()
                    .any(|property| parsed_type_mentions_any(&property.ty, names))
        }
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Undefined
        | ParsedType::Void
        | ParsedType::Any
        | ParsedType::Unknown
        | ParsedType::UnknownKeyword
        | ParsedType::Never
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_) => false,
        _ => true,
    }
}

pub(crate) fn generic_class_instance_type(
    callee: &ParsedExpression,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let ParsedExpression::Identifier { name, .. } = callee else {
        return None;
    };
    // A written argument names types visible at the CALL; a synthesized one is a
    // declared default or constraint and names types visible where the CLASS is
    // declared (Prisma's `ClientOptions`, socket.io's `DefaultEventMap`), which
    // the consumer's scope does not hold. The two need different scopes, so the
    // mixed case — some written, some synthesized — is left alone rather than
    // resolved under one of them and reported as an unresolved name.
    let synthesize = type_arguments.is_empty();
    let (declared, declaring_file) = {
        let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) =
            ctx.lookup_type_declaration(name)
        else {
            return None;
        };
        let declared = &info.body.type_parameters;
        if declared.is_empty() {
            return None;
        }
        if !synthesize && type_arguments.len() != declared.len() {
            return None;
        }
        (
            synthesize.then(|| declared.clone()).unwrap_or_default(),
            info.file_name.clone(),
        )
    };
    // What the constructor's arguments prove, which outranks a declared default:
    // `new MutationObserver(client, { mutationFn: (text: string) => … })` is
    // `TVariables = string`, not the `void` the declaration defaults to.
    let inferred = synthesize
        .then(|| infer_generic_class_type_arguments(name, &declared, arguments, symbols, ctx))
        .flatten()
        .unwrap_or_default();
    let use_declared_defaults = synthesize
        && declared.iter().all(|parameter| {
            parameter.default_type.is_some()
                && (inferred.get(&parameter.name).is_none()
                    || inferred.is_placeholder(&parameter.name))
        });
    let arguments: Vec<ParsedType> = if use_declared_defaults {
        Vec::new()
    } else if synthesize {
        let names: Vec<&str> = declared.iter().map(|p| p.name.as_str()).collect();
        let mut synthesized = Vec::with_capacity(declared.len());
        for parameter in declared.iter() {
            if let Some(inferred_type) = inferred.get(&parameter.name)
                && !inferred.is_placeholder(&parameter.name)
            {
                // Written back as the type it resolved to whenever the syntax can
                // say it, so the instance is built exactly as an explicit
                // `new C<string, …>()` is. Resolving a synthesized `Named(param)`
                // through the substitution binds the *argument* correctly but a
                // member's lazy alias reference (`subscribe(listener:
                // Listener<A, B, C>)`) re-reads its parsed arguments later, past
                // the substitution, and came out with the class's own parameters
                // unbound. A type the syntax cannot spell keeps the name route.
                synthesized.push(reify_type_argument(inferred_type).unwrap_or_else(|| {
                    ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                        type_argument_spans: Vec::new(),
                        name: parameter.name.clone(),
                        span: None,
                        type_arguments: Vec::new(),
                    }))
                }));
                continue;
            }
            let fallback = parameter
                .default_type
                .clone()
                .or_else(|| parameter.constraint.clone())
                .unwrap_or(ParsedType::UnknownKeyword);
            // A default may be written in terms of its *siblings*
            // (zod's `Output = objectOutputType<T, Catchall, UnknownKeys>`).
            // Those are type parameters, not names in the declaring file, so
            // resolving the default there reports them as unresolved. Binding
            // them means resolving the list left to right under a growing
            // substitution, which this path does not carry — so a class written
            // that way keeps the permissive value side it has today.
            if parsed_type_mentions_any(&fallback, &names) {
                return None;
            }
            synthesized.push(fallback);
        }
        synthesized
    } else {
        type_arguments.to_vec()
    };
    let named = ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.clone(),
        span: None,
        type_arguments: arguments,
    }));
    let saved_symbols = std::mem::replace(
        &mut ctx.symbols,
        symbols.clone_with_reason(TypeCopyReason::CallResolution),
    );
    let saved_file = synthesize.then(|| {
        let saved = ctx.file_name.clone();
        ctx.set_file_name(declaring_file.to_string());
        saved
    });
    let instance = crate::infer::map_parsed_type_with_substitution(named, ctx, &inferred);
    if let Some(saved) = saved_file {
        ctx.set_file_name(saved);
    }
    ctx.symbols = saved_symbols;
    (!instance.is_unknown()).then_some(instance)
}

/// The written form of a resolved type, for the cases the syntax spells
/// directly: primitives, literals, and arrays/tuples/unions of those. `None` for
/// anything nominal or structural, which has no context-free spelling here.
fn reify_type_argument(ty: &Type) -> Option<ParsedType> {
    Some(match ty {
        Type::String => ParsedType::String,
        Type::Number => ParsedType::Number,
        Type::Boolean => ParsedType::Boolean,
        Type::BigInt => ParsedType::BigInt,
        Type::Symbol => ParsedType::Symbol,
        Type::Undefined => ParsedType::Undefined,
        Type::Void => ParsedType::Void,
        Type::Any => ParsedType::Any,
        Type::Never => ParsedType::Never,
        Type::GenuineUnknown => ParsedType::UnknownKeyword,
        Type::StringLiteral(value) => ParsedType::StringLiteral(value.to_string()),
        Type::NumberLiteral(literal) => ParsedType::NumberLiteral(literal.value.to_string()),
        Type::BooleanLiteral(value) => ParsedType::BooleanLiteral(*value),
        Type::Array(element) => {
            ParsedType::Array(std::sync::Arc::new(reify_type_argument(element)?))
        }
        Type::Tuple(elements) => ParsedType::Tuple(std::sync::Arc::new(
            elements
                .iter()
                .map(reify_type_argument)
                .collect::<Option<Vec<_>>>()?,
        )),
        Type::Union(union) => ParsedType::Union(std::sync::Arc::new(
            union
                .types()
                .iter()
                .map(reify_type_argument)
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => return None,
    })
}

/// The kind and signature of the value `name` constructs, read as a name is: a
/// namespace body reaches its module's own declarations only through the
/// module value fallback.
pub(crate) fn class_value_signature(
    name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<(crate::symbols::SymbolKind, Option<std::sync::Arc<FunctionSignatureInfo>>)> {
    symbols
        .get(name)
        .or_else(|| ctx.module_value_fallback.as_ref().and_then(|fallback| fallback.get(name)))
        .map(|symbol| (symbol.kind, symbol.function_signature.clone()))
}

/// The class type arguments the constructor call infers, by the same machinery a
/// generic function call uses. The written constructor signature rides on the
/// class's value symbol; without one (no explicit constructor, or a class bound
/// before its signature was attached) nothing is inferred and the caller keeps
/// the declared defaults.
fn infer_generic_class_type_arguments(
    name: &str,
    declared: &[surge_ts_syntax::ParsedTypeParameter],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<crate::infer::TypeParameterSubstitution> {
    if arguments.is_empty() {
        return None;
    }
    let (_, signature) = class_value_signature(name, symbols, ctx)?;
    let mut signature = signature?;
    // A class with no constructor of its own takes its base's, carried under
    // its own type parameters (`generic_class_construct_signatures`).
    if signature.type_parameters.is_empty()
        && let Some([inherited]) = signature.construct_signatures.as_deref().map(Vec::as_slice)
    {
        signature = inherited.signature.clone();
    }
    if signature.type_parameters.len() != declared.len() {
        return None;
    }
    Some(infer_type_argument_substitution(
        &signature,
        arguments,
        &[],
        None,
        symbols,
        ctx,
    ))
}

fn new_with_call_signature(
    function_type: &surge_ts_types::FunctionType,
    callee: &ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // `resolveCall` over the call signatures: written type arguments no
    // signature takes are TS2558, and no argument is related to one then.
    if report_type_argument_arity(
        &Type::Function(function_type.clone()),
        type_arguments,
        type_arguments_start(callee_span),
        true,
        ctx,
    ) {
        ctx.degraded_expected_type_depth += 1;
        for argument in arguments {
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
        }
        ctx.degraded_expected_type_depth -= 1;
    } else {
        let _ = check_function_type_call(
            function_type,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        );
    }
    // A return inferred from the body is a lazy reference until read.
    let returned = function_type.return_type().peeled();
    // A function constructed inside its own unannotated body reads its return
    // type circularly, which tsc resolves to the error type.
    let circular = matches!(callee, ParsedExpression::Identifier { name, .. }
        if crate::checks::function::in_circular_body_of(name));
    let diagnostic = if ctx.options.no_implicit_any {
        Some(Diagnostic::ts7009(ctx.file_name.clone()))
    } else if returned != Type::Void
        // A return type surge has not inferred may well be `void`.
        && (!returned.is_unmodelled() || circular)
    {
        Some(Diagnostic::ts2350(ctx.file_name.clone()))
    } else {
        None
    };
    if let Some(diagnostic) = diagnostic {
        ctx.push(diagnostic_with_syntax_span(diagnostic, call_span.or(callee_span)));
    }
    Some(Type::Any)
}

/// One of tsc's effective call arguments (`getEffectiveCallArguments`): a
/// spread of a tuple type stands for one argument per element, and its rest
/// element for a variadic one.
enum EffectiveArgument {
    /// A written argument or a fixed tuple element.
    Plain(Type),
    /// An open tuple's rest element, spread: the array of its element type.
    Variadic(Type),
    /// A spread of a type that is not a tuple.
    Spread(Type),
}

/// An arrow invoked on the spot (`((table) => …)(f.field.table)`) is typed from
/// the call's own arguments, which is the contextual type it has no other way to
/// get (tsc's `getContextuallyTypedParameterType` for an IIFE): each parameter
/// takes the argument at its position, a rest parameter the ones from there on,
/// and a parameter no argument reaches `undefined`. Such a parameter is also
/// optional to the call (`isOptionalParameter`).
///
/// The arguments are typed with diagnostics dropped: they are checked for real
/// by the call below, and reporting here would double every one of them.
fn immediately_invoked_arrow_type(
    callee: &surge_ts_syntax::ParsedExpression,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<surge_ts_types::FunctionType> {
    let surge_ts_syntax::ParsedExpression::ArrowFunction(arrow) = callee else {
        return None;
    };
    if arrow.parameters.is_empty()
        || arrow
            .parameters
            .iter()
            .all(|parameter| parameter.declared_type.is_some())
    {
        return None;
    }

    let diagnostics_before = ctx.diagnostics().len();
    let mut effective = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let ty = match evaluate_expression(&argument.expression, argument.span, symbols, ctx) {
            InferredExpression::Known(ty) => ty,
            _ => Type::Unknown,
        };
        if !argument.spread {
            // tsc reads the widened literal type of a written argument:
            // `({ p = 14 }) => p` called with `{ p: 15 }` binds `p: number`.
            // `null` and `undefined` are no auto-typed binding there: under
            // `strictNullChecks` they stay themselves.
            let widened = if surge_ts_types::strict_null_checks() && matches!(ty, Type::Null | Type::Undefined) {
                ty
            } else {
                crate::checks::var::widen_implicit_variable_initializer_type(
                    crate::symbols::SymbolKind::Let,
                    &argument.expression,
                    &ty,
                    false,
                )
            };
            effective.push(EffectiveArgument::Plain(widened));
            continue;
        }
        match ty.peeled() {
            Type::Tuple(elements) => {
                effective.extend(elements.into_iter().map(EffectiveArgument::Plain));
            }
            Type::OpenTuple(open) => {
                effective.extend(open.leading.into_iter().map(EffectiveArgument::Plain));
                effective.push(EffectiveArgument::Variadic(Type::Array(open.rest)));
                effective.extend(open.trailing.into_iter().map(EffectiveArgument::Plain));
            }
            other => effective.push(EffectiveArgument::Spread(other)),
        }
    }
    ctx.truncate_diagnostics(diagnostics_before);
    // A spread of a generic body's own type variable (`...t` with `t: T`) is an
    // argument like any other; only a spread surge could not model stops here.
    // A plain argument it could not type gives its parameter the sentinel: the
    // parameter is typed from the argument, never implicitly `any`.
    if effective.iter().any(|argument| match argument {
        EffectiveArgument::Plain(_) => false,
        EffectiveArgument::Variadic(ty) | EffectiveArgument::Spread(ty) => ty.is_unmodelled(),
    }) {
        return None;
    }

    // `checkExpression` of an effective argument: a spread reads as its element.
    let argument_type = |argument: &EffectiveArgument| match argument {
        EffectiveArgument::Plain(ty) => ty.clone(),
        EffectiveArgument::Variadic(array) | EffectiveArgument::Spread(array) => {
            crate::checks::function::for_of_element_type(array)
        }
    };
    let parameter_count = arrow.parameters.len();
    let rest_index = arrow
        .parameters
        .last()
        .filter(|parameter| parameter.rest)
        .map(|_| parameter_count - 1);
    let mut expected_parameters = Vec::with_capacity(parameter_count);
    for (index, parameter) in arrow.parameters.iter().enumerate() {
        if Some(index) == rest_index {
            expected_parameters.push(spread_argument_type(&effective[index.min(effective.len())..]));
            continue;
        }
        let ty = match effective.get(index) {
            // A context-sensitive argument has no type to give the parameter
            // but the one it would take from that parameter: tsc's IIFE
            // parameter reads it context-free, and the argument then has no
            // contextual signature, its own parameters implicitly `any`.
            Some(_)
                if arguments
                    .get(index)
                    .is_some_and(|argument| !argument.spread && is_context_sensitive_argument(&argument.expression)) =>
            {
                Type::Any
            }
            Some(argument) => argument_type(argument),
            // No argument leaves the parameter to its initializer, whose
            // widened type it takes as a declaration would.
            None => match parameter.initializer.as_ref() {
                Some(initializer) => {
                    let inferred =
                        evaluate_expression(initializer, parameter.initializer_span, symbols, ctx);
                    ctx.truncate_diagnostics(diagnostics_before);
                    match inferred {
                        InferredExpression::Known(ty) => {
                            crate::checks::var::widen_implicit_variable_initializer_type(
                                crate::symbols::SymbolKind::Let,
                                initializer,
                                &ty,
                                false,
                            )
                        }
                        _ => Type::Unknown,
                    }
                }
                // `undefinedWideningType`, which widens to `any` without
                // `strictNullChecks`.
                None if ctx.options.strict_null_checks => Type::Undefined,
                None => Type::Any,
            },
        };
        expected_parameters.push(ty);
    }

    let expected_parameters_for_rest = rest_index.map(|index| expected_parameters[index].clone());
    let expected = crate::metrics::alloc_function_type(
        expected_parameters,
        Type::Unknown,
        rest_index.is_some(),
        effective.len().min(parameter_count),
    );
    let checked = crate::checks::function::check_arrow_function_expression_with_expected_type(
        (**arrow).clone(),
        Some(&expected),
        symbols,
        ctx,
    );
    // An unannotated parameter past the last argument is optional to the call.
    let required = arrow
        .parameters
        .iter()
        .enumerate()
        .filter(|(index, parameter)| {
            !parameter.optional
                && parameter.initializer.is_none()
                && !parameter.rest
                && (parameter.declared_type.is_some() || *index < effective.len())
        })
        .map(|(index, _)| index + 1)
        .max()
        .unwrap_or(0);
    // The rest parameter's slot holds what it collects, which the call below
    // matches its arguments against position by position.
    let mut parameters = checked.parameters().to_vec();
    let rest_collection = rest_index.and_then(|index| {
        let collection = expected_parameters_for_rest.clone()?;
        (index < parameters.len()).then(|| (index, collection))
    });
    if required >= checked.required_parameter_count() && rest_collection.is_none() {
        return Some(checked);
    }
    if let Some((index, collection)) = rest_collection {
        parameters[index] = collection;
    }
    Some(
        surge_ts_types::FunctionType::new(
            parameters,
            checked.return_type().clone(),
            checked.is_variadic(),
            required.min(checked.required_parameter_count()),
        )
        .with_parameter_names(crate::checks::function::written_binding_names(&arrow.parameters)),
    )
}

/// An argument a generic constructor's inference cannot read in isolation:
/// a function, a `new` or call, or a literal holding one.
fn holds_callback_or_nested_call(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::ArrowFunction(_)
        | ParsedExpression::New { .. }
        | ParsedExpression::Call { .. }
        | ParsedExpression::PropertyCall { .. }
        | ParsedExpression::OptionalPropertyCall { .. }
        | ParsedExpression::OptionalCall { .. }
        | ParsedExpression::ExpressionCall { .. } => true,
        ParsedExpression::ObjectLiteral { properties, .. } => properties.iter().any(|property| {
            property.is_method || property.is_accessor || holds_callback_or_nested_call(&property.value)
        }),
        ParsedExpression::ArrayLiteral { elements, .. } => {
            elements.iter().any(|element| holds_callback_or_nested_call(&element.expression))
        }
        _ => false,
    }
}

/// tsc's `isContextSensitive` for the shapes an IIFE argument takes: a
/// function with an unannotated parameter, or an object literal holding one.
pub(crate) fn is_context_sensitive_argument(expression: &surge_ts_syntax::ParsedExpression) -> bool {
    match expression {
        surge_ts_syntax::ParsedExpression::ArrowFunction(arrow) => {
            arrow.parameters.iter().any(|parameter| parameter.declared_type.is_none())
        }
        surge_ts_syntax::ParsedExpression::ObjectLiteral { properties, .. } => properties
            .iter()
            .any(|property| !property.is_spread && is_context_sensitive_argument(&property.value)),
        _ => false,
    }
}

/// tsc's `isOptionalParameter` for an IIFE whose parameters are not typed
/// from its arguments: an unannotated parameter no argument reaches is
/// optional to the call all the same.
fn immediately_invoked_optional_parameters(
    callee: &surge_ts_syntax::ParsedExpression,
    arguments: &[ParsedCallArgument],
    function_type: surge_ts_types::FunctionType,
) -> surge_ts_types::FunctionType {
    let surge_ts_syntax::ParsedExpression::ArrowFunction(arrow) = callee else {
        return function_type;
    };
    if arguments.iter().any(|argument| argument.spread) {
        return function_type;
    }
    let required = arrow
        .parameters
        .iter()
        .enumerate()
        .filter(|(index, parameter)| {
            !parameter.optional
                && parameter.initializer.is_none()
                && !parameter.rest
                && (parameter.declared_type.is_some() || *index < arguments.len())
        })
        .map(|(index, _)| index + 1)
        .max()
        .unwrap_or(0);
    if required >= function_type.required_parameter_count() {
        return function_type;
    }
    surge_ts_types::FunctionType::new(
        function_type.parameters().to_vec(),
        function_type.return_type().clone(),
        function_type.is_variadic(),
        required,
    )
    .with_parameter_names(crate::checks::function::written_binding_names(&arrow.parameters))
}

/// tsc's `getSpreadArgumentType` for the arguments from a rest parameter's
/// position on: a spread in the last position is the collection itself, and
/// otherwise the arguments make up a tuple.
fn spread_argument_type(arguments: &[EffectiveArgument]) -> Type {
    if let [EffectiveArgument::Variadic(array) | EffectiveArgument::Spread(array)] = arguments {
        return array.clone();
    }
    let mut leading = Vec::new();
    let mut rest = None;
    let mut trailing = Vec::new();
    for argument in arguments {
        let (element, variadic) = match argument {
            EffectiveArgument::Plain(ty) => (ty.clone(), false),
            EffectiveArgument::Variadic(array) | EffectiveArgument::Spread(array) => {
                (crate::checks::function::for_of_element_type(array), true)
            }
        };
        match (&rest, variadic) {
            (None, false) => leading.push(element),
            (None, true) => rest = Some(element),
            (Some(_), false) => trailing.push(element),
            // A second variadic run joins the first: tsc normalizes a tuple to
            // one rest element and every later element into it.
            (Some(existing), true) => {
                rest = Some(surge_ts_types::union_type(vec![existing.clone(), element]));
            }
        }
    }
    match rest {
        None => Type::Tuple(leading),
        Some(rest) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading,
            rest: Box::new(rest),
            trailing,
        }),
    }
}

/// Checks a call whose callee is an arbitrary expression (an IIFE, a call on a
/// call). The callee and the arguments are always evaluated so everything
/// written inside them is checked; the result is the callee's return type.
pub(crate) fn check_expression_call(
    callee: &surge_ts_syntax::ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_expression_call_unrecorded(callee, callee_span, call_span, type_arguments, arguments, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

fn check_expression_call_unrecorded(
    callee: &surge_ts_syntax::ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // tsc resolves `super(…)` against the base constructor's construct
    // signatures and types the call `void`. A base surge has no constructor
    // value for falls through to the degraded walk below.
    let generic_base = ctx.super_class_reference.as_ref().is_some_and(|base| match &ctx.super_constructor_type {
        Some(Type::Any) => true,
        Some(ty) => is_generic_class_value(
            &ParsedExpression::Identifier { name: base.name.clone(), span: None },
            &ty.peeled(),
            ctx,
        ),
        None => false,
    });
    if let ParsedExpression::Identifier { name, .. } = callee
        && name == "super"
        && !generic_base
        && let Some(construct_signature) = ctx
            .super_constructor_type
            .as_ref()
            .and_then(construct_signature_of)
    {
        let _ = check_function_type_call(
            &construct_signature,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        );
        return Some(Type::Void);
    }
    // A generic base's constructor is checked as `new Base<Args>(…)` with
    // the `extends` clause's type arguments
    // (`getInstantiatedConstructorsForTypeArguments`).
    if let ParsedExpression::Identifier { name, .. } = callee
        && name == "super"
        && generic_base
        && let Some(base) = ctx.super_class_reference.clone()
        && construct::check_generic_class_construct(
            &ParsedExpression::Identifier { name: base.name.clone(), span: None },
            callee_span,
            call_span,
            &base.type_arguments,
            arguments,
            symbols,
            ctx,
        )
    {
        return Some(Type::Void);
    }

    if callee.continues_optional_chain()
        && let Some((unmarked, root)) = unmarked_optional_chain(callee)
    {
        let result = check_expression_call(
            &unmarked,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        )?;
        let short_circuits = match crate::infer::infer_expression(root, symbols, ctx) {
            InferredExpression::Known(receiver) => {
                crate::checks::expr::receiver_nullability(&receiver).is_some()
            }
            _ => true,
        };
        return Some(if short_circuits {
            union_type(vec![result, Type::Undefined])
        } else {
            result
        });
    }

    let callee_result = match immediately_invoked_arrow_type(callee, arguments, symbols, ctx) {
        Some(function_type) => InferredExpression::Known(Type::Function(function_type)),
        None => match evaluate_expression(callee, callee_span, symbols, ctx) {
            InferredExpression::Known(Type::Function(function_type)) => InferredExpression::Known(Type::Function(
                immediately_invoked_optional_parameters(callee, arguments, function_type),
            )),
            other => other,
        },
    };

    let callee_type = match callee_result {
        InferredExpression::Known(ty) => ty.peeled(),
        _ => {
            ctx.degraded_expected_type_depth += 1;
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            ctx.degraded_expected_type_depth -= 1;
            return None;
        }
    };
    let callee_type = check_non_null_callee(callee, callee_type, callee_span, ctx);

    match callee_type {
        Type::Function(function_type) => check_function_type_call(
            &function_type,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        ),
        Type::Union(union) if union_signature_lists(&union).is_some() => check_callable_union_call(
            &union,
            callee_span,
            callee_span,
            call_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ),
        // A primitive's apparent type (`String`, `Number`) has no call
        // signature: `resolveCallExpression`'s TS2349 on the callee.
        Type::String
        | Type::StringLiteral(_)
        | Type::Number
        | Type::NumberLiteral(_)
        | Type::Boolean
        | Type::BooleanLiteral(_) => {
            ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2349(ctx.file_name.clone()), callee_span));
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            None
        }
        // A callee surge could not reduce to a signature still has one for tsc,
        // so the arguments are evaluated under a degraded expectation: their own
        // diagnostics surface, but an implicit-any report describing surge's
        // missing contextual type does not (`describe.each(cases)(name, cb)`).
        _ => {
            ctx.degraded_expected_type_depth += 1;
            for argument in arguments {
                let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            }
            ctx.degraded_expected_type_depth -= 1;
            None
        }
    }
}

/// tsc's `getOptionalExpressionType`: a call continuing an optional chain
/// (`o?.["m"]()`) invokes the member as read off the non-nullish receiver, and
/// the chain's short-circuit adds `undefined` to the call's result instead of
/// to the callee. The chain rewritten with a non-optional access on a
/// non-null receiver at its root reads the member that way; the root's
/// receiver says whether the chain can short-circuit.
fn unmarked_optional_chain(
    expression: &ParsedExpression,
) -> Option<(ParsedExpression, &ParsedExpression)> {
    let non_null = |object: &ParsedExpression, span: Option<SyntaxTextSpan>| {
        Box::new(ParsedExpression::NonNullAssertion {
            expression: Box::new(object.clone()),
            span,
            in_optional_chain: false,
        })
    };
    match expression {
        ParsedExpression::OptionalPropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
        } => Some((
            ParsedExpression::PropertyAccess {
                object: non_null(object, *object_span),
                object_span: *object_span,
                property_name: property_name.clone(),
                property_span: *property_span,
                is_bracketed: *is_bracketed,
                binding_element: false,
            },
            object,
        )),
        ParsedExpression::OptionalIndexAccess {
            object,
            object_span,
            index,
            index_span,
        } => Some((
            ParsedExpression::ElementAccess {
                object: non_null(object, *object_span),
                object_span: *object_span,
                index: index.clone(),
                index_span: *index_span,
            },
            object,
        )),
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
            binding_element,
        } if object.continues_optional_chain() => {
            let (inner, root) = unmarked_optional_chain(object)?;
            Some((
                ParsedExpression::PropertyAccess {
                    object: Box::new(inner),
                    object_span: *object_span,
                    property_name: property_name.clone(),
                    property_span: *property_span,
                    is_bracketed: *is_bracketed,
                    binding_element: *binding_element,
                },
                root,
            ))
        }
        ParsedExpression::ElementAccess {
            object,
            object_span,
            index,
            index_span,
        } if object.continues_optional_chain() => {
            let (inner, root) = unmarked_optional_chain(object)?;
            Some((
                ParsedExpression::ElementAccess {
                    object: Box::new(inner),
                    object_span: *object_span,
                    index: index.clone(),
                    index_span: *index_span,
                },
                root,
            ))
        }
        _ => None,
    }
}

/// `resolveCallExpression` reads the callee through `checkNonNullType` with
/// the invocation wording: a possibly-`null` callee (the `null` keyword
/// included) is TS2721, a possibly-`undefined` one TS2722 and one that may be
/// either TS2723, all anchored on the callee (its parentheses included). The
/// call goes on with the rest of the type, or `any` when nothing is left.
fn check_non_null_callee(
    callee: &ParsedExpression,
    callee_type: Type,
    callee_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> Type {
    if callee_type == Type::GenuineUnknown {
        report_uncallable_unknown(callee, callee_span, ctx);
        return Type::Unknown;
    }
    let is_null = matches!(callee, ParsedExpression::NullLiteral);
    let nullability = if is_null {
        Some((true, false))
    } else {
        crate::checks::expr::receiver_nullability(&callee_type)
    };
    let Some((null, undefined)) = nullability else {
        return callee_type;
    };
    let span = callee_span
        .and_then(|span| ctx.parenthesized_outer_span(span))
        .or(callee_span);
    let file_name = ctx.file_name.clone();
    let diagnostic = match (null, undefined) {
        (true, true) => Diagnostic::ts2723(file_name),
        (true, false) => Diagnostic::ts2721(file_name),
        _ => Diagnostic::ts2722(file_name),
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
    match callee_type {
        Type::Undefined | Type::Null => Type::Any,
        _ if is_null => Type::Any,
        _ => surge_ts_types::remove_nullish(&callee_type).peeled(),
    }
}

/// A call on `unknown`: `checkNonNullType`'s TS18046 under `strictNullChecks`,
/// otherwise the plain not-callable TS2349.
fn report_uncallable_unknown(
    callee: &ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if !crate::checks::expr::report_unknown_operand(callee, callee_span, ctx) {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2349(ctx.file_name.clone()),
            callee_span,
        ));
    }
}

/// A call that continues an optional chain adds the chain's `undefined` only
/// when the chain's receiver can be nullish (tsc's `getOptionalExpressionType`).
pub(crate) fn with_chain_undefined(returned: Type, receiver: &Type) -> Type {
    if crate::infer::expression::optional_chain_can_short_circuit(receiver) {
        union_type(vec![returned, Type::Undefined])
    } else {
        returned
    }
}

pub(crate) fn check_optional_call_like(
    callee: &surge_ts_syntax::ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_optional_call_like_unrecorded(callee, callee_span, call_span, type_arguments, arguments, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

fn check_optional_call_like_unrecorded(
    callee: &surge_ts_syntax::ParsedExpression,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let callee_result = evaluate_expression(callee, callee_span, symbols, ctx);

    let callee_type = match callee_result {
        InferredExpression::Known(ty) => ty,
        _ => return None,
    };

    let base_type = surge_ts_types::remove_nullish(&callee_type);

    // A callee typed by an interface or type literal with a call signature
    // (`declare const expectTypeOf: _ExpectTypeOf`) is as callable as a plain
    // function type, and it may arrive as a lazy reference to that shape.
    let callable = match &base_type {
        Type::Function(function_type) => Some(function_type.clone()),
        Type::Object(object) => object.call_signature().cloned(),
        Type::Reference(_) => match base_type.peeled() {
            Type::Function(function_type) => Some(function_type),
            Type::Object(object) => object.call_signature().cloned(),
            _ => None,
        },
        _ => None,
    };

    match base_type {
        Type::Any => {
            super::call::property::evaluate_arguments_context_free(callee, arguments, symbols, ctx);
            Some(Type::Any)
        }
        // Same rule as the property-call path: a callee surge could not model
        // does not make its arguments stop being code.
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
            super::call::property::evaluate_arguments_context_free(callee, arguments, symbols, ctx);
            None
        }
        _ if callable.is_some() => check_function_type_call(
            callable.as_ref().expect("checked above"),
            callee_span,
            call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        )
        .map(|ret| with_chain_undefined(ret, &callee_type)),
        _ => {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2349(ctx.file_name.clone()),
                callee_span,
            ));
            None
        }
    }
}

/// The element type a variadic rest parameter accepts per argument. A rest
/// parameter is declared as an array (`...inputs: string[]`); each supplied
/// argument is checked against the element (`string`). Non-array types (e.g. an
/// unresolved `any`) are kept as-is.
/// The span tsc anchors a too-many-arguments (TS2554) error on: the range from
/// the first excess argument through the last supplied argument. Returns `None`
/// when the relevant argument spans are unavailable so the caller can fall back
/// to the call/callee span.
/// tsc's `getEffectiveCallArguments` as far as the argument count goes: a
/// spread of an array literal or of a fixed tuple stands for one argument per
/// element. `None` when a spread supplies no fixed count — an array, a
/// tuple's variable part, or a type surge did not settle — which
/// `hasCorrectArity` judges by where the spread begins instead.
struct EffectiveSpreadArguments {
    count: usize,
    /// The written argument each effective position comes from.
    sources: Vec<usize>,
}

impl EffectiveSpreadArguments {
    fn source_of(&self, position: usize) -> Option<usize> {
        self.sources.get(position).copied()
    }
}

fn effective_spread_arguments(
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<EffectiveSpreadArguments> {
    let mut sources = Vec::with_capacity(arguments.len());
    for (index, argument) in arguments.iter().enumerate() {
        let count = if !argument.spread {
            1
        } else {
            match &argument.expression {
                ParsedExpression::ArrayLiteral { elements, .. }
                    if elements.iter().all(|element| !element.spread) =>
                {
                    elements.len()
                }
                expression => {
                    let diagnostics_before = ctx.diagnostics().len();
                    let spread = evaluate_expression(expression, argument.span, symbols, ctx);
                    ctx.truncate_diagnostics(diagnostics_before);
                    match spread {
                        InferredExpression::Known(ty) => match ty.peeled() {
                            Type::Tuple(elements)
                                if !elements.iter().any(type_contains_unknown) =>
                            {
                                elements.len()
                            }
                            _ => return None,
                        },
                        _ => return None,
                    }
                }
            }
        };
        sources.extend(std::iter::repeat_n(index, count));
    }
    Some(EffectiveSpreadArguments {
        count: sources.len(),
        sources,
    })
}

fn excess_argument_span(
    arguments: &[ParsedCallArgument],
    expected: usize,
) -> Option<SyntaxTextSpan> {
    let first = arguments.get(expected)?.span?;
    let last = arguments.last().and_then(|argument| argument.span);
    Some(SyntaxTextSpan {
        start: first.start,
        end: last.map(|span| span.end).unwrap_or(first.end),
    })
}

/// The construct signature a `new` target carries: a function type is its own
/// (surge models a class value's static side as an object), an object supplies
/// its declared one, and a reference peels to whichever it resolves to.
fn construct_signature_of(ty: &Type) -> Option<surge_ts_types::FunctionType> {
    match ty.peeled() {
        Type::Function(function_type) => Some(function_type),
        Type::Object(object) => object.construct_signature().cloned(),
        _ => None,
    }
}

/// The first argument `candidate` rejects beyond doubt. Only a primitive or
/// unit mismatch counts (`true` for a `string` / `number` slot): an object
/// argument's rejection may be a lib shape surge models partly (`File` against
/// `Blob`), and reporting it would describe surge rather than the call.
fn candidate_definitely_rejects(
    candidate: &FunctionType,
    argument_types: &[ArgumentShape],
) -> Option<usize> {
    argument_types
        .iter()
        .enumerate()
        .find_map(|(index, shape)| {
            let argument = shape.ty.as_ref()?;
            let parameter = effective_parameter_type(candidate, index)?;
            (!type_contains_unknown(argument)
                && !names_open_parameter(&parameter)
                && (crate::checks::assign::definite_unit_member_mismatch(argument, &parameter)
                    || crate::checks::assign::definite_primitive_member_mismatch(
                        argument, &parameter,
                    )
                    || (is_primitive_only(argument)
                        && is_primitive_only(&parameter.peeled())
                        && !is_assignable_to(argument, &parameter))))
            .then_some(index)
        })
}

/// A type made of primitives and their literals only, whose relations surge
/// decides without any lib shape.
fn is_primitive_only(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_primitive_only),
        _ => false,
    }
}

/// tsc's `reportCallResolutionErrors` re-checks the last candidate and reports
/// "No overload matches this call" where that candidate rejects its first
/// argument. `None` when no typed argument is rejected by it (a callback, whose
/// shape is a wildcard here, may be the one), which leaves the fold's verdict.
fn last_candidate_rejected_argument(
    candidate: &FunctionType,
    argument_types: &[ArgumentShape],
) -> Option<usize> {
    argument_types
        .iter()
        .enumerate()
        .find_map(|(index, shape)| {
            let argument = shape.ty.as_ref()?;
            // A generic candidate's own parameters are the sentinel here, which a
            // relation accepts; what rejects is the rest of the shape (an arity, a
            // parameter's type), exactly as the candidate itself would.
            let parameter = effective_parameter_type(candidate, index)?;
            (!is_assignable_to(argument, &parameter)).then_some(index)
        })
}

/// The argument counts of a signature whose rest parameter is a fixed tuple
/// (`getMinArgumentCount`, `getParameterCount`): its other parameters, all
/// required, then the tuple's required elements, up to all of them.
fn fixed_tuple_rest_arity(signature: &FunctionType) -> Option<(usize, usize)> {
    if !signature.is_variadic() || signature.overloads().is_some() {
        return None;
    }
    let (rest, leading) = signature.parameters().split_last()?;
    // `...args: readonly [A, B]` is the tuple behind the readonly wrapper.
    let rest = match rest {
        Type::Reference(reference) if &*reference.id == surge_ts_types::READONLY_REFERENCE_ID => {
            reference.arguments.first()?
        }
        other => other,
    };
    let (elements, min_length) = surge_ts_types::fixed_tuple_parts(rest)?;
    // An empty tuple is as often a conditional surge failed to match
    // (`Parameters<T> extends [any, ...infer R] ? R : never`).
    if elements.is_empty()
        || signature.required_parameter_count() != leading.len()
        || elements.iter().any(Type::is_degraded)
    {
        return None;
    }
    // A tuple built from parameters (`infer A` against `(expected?: any) =>
    // void`, vitest's `Promisify`) marks an optional element only by the
    // `undefined` it adds, which `any` and `unknown` hold already, so such a
    // trailing element may be optional. A written tuple records its length.
    let min_length = if matches!(rest, Type::Tuple(_)) {
        let may_be_optional = |element: &Type| {
            matches!(element, Type::Any | Type::GenuineUnknown | Type::Undefined)
                || element.is_unknown()
                || matches!(element, Type::Union(union) if union.types().contains(&Type::Undefined))
        };
        elements.len() - elements.iter().rev().take_while(|element| may_be_optional(element)).count()
    } else {
        min_length
    };
    Some((leading.len() + min_length, leading.len() + elements.len()))
}

/// The arity of a rest tuple inferred from a function literal's parameters
/// (`call<T extends unknown[]>(f: (...args: T) => void, ...args: T)` called
/// with `(x: number, y?: number) => …`): tsc's tuple keeps which elements are
/// optional, and so does the literal. `(required, total)` with the leading
/// parameters counted.
fn function_literal_rest_arity(signature: &FunctionType, arguments: &[ParsedCallArgument]) -> Option<(usize, usize)> {
    if !signature.is_variadic() || signature.overloads().is_some() {
        return None;
    }
    let (rest, leading) = signature.parameters().split_last()?;
    let Type::Tuple(elements) = rest else {
        return None;
    };
    leading.iter().enumerate().find_map(|(index, parameter)| {
        let Type::Function(callback) = parameter.peeled() else {
            return None;
        };
        if !callback.is_variadic() || callback.parameters() != std::slice::from_ref(rest) {
            return None;
        }
        let argument = arguments.get(index).filter(|argument| !argument.spread)?;
        let surge_ts_syntax::ParsedExpression::ArrowFunction(literal) = &argument.expression else {
            return None;
        };
        if literal.parameters.len() != elements.len() || literal.parameters.iter().any(|parameter| parameter.rest) {
            return None;
        }
        // A trailing parameter that accepts `void` may be left out too.
        let required = literal
            .parameters
            .iter()
            .zip(elements)
            .rposition(|(parameter, element)| {
                !parameter.optional && parameter.initializer.is_none() && !parameter_is_void_optional(element)
            })
            .map_or(0, |last| last + 1);
        Some((leading.len() + required, leading.len() + elements.len()))
    })
}

fn call_site_required_count(signature: &FunctionType) -> usize {
    let parameters = signature.parameters();
    let mut required = signature.required_parameter_count();
    while required > 0
        && (parameter_is_void_optional(&parameters[required - 1])
            || names_open_parameter(&parameters[required - 1]))
    {
        required -= 1;
    }
    required
}

/// tsc's `chooseOverload` candidate filter (`hasCorrectArity`,
/// `hasCorrectTypeArgumentArity`) over the signatures an overload group keeps:
/// the first declaration's, then its later overloads in declaration order,
/// which the folded callee lists in the same order. A group instantiates
/// through one signature, so it must be one this call can bind at all. Empty
/// when the kept signature fits (the group is instantiated through it, as
/// before) or when none does.
fn overloads_fitting_call<'a>(
    kept: &'a FunctionSignatureInfo,
    callee: &Type,
    argument_count: usize,
    type_argument_count: usize,
) -> Vec<&'a FunctionSignatureInfo> {
    let Type::Function(fold) = callee else {
        return Vec::new();
    };
    let Some(members) = fold.overloads() else {
        return Vec::new();
    };
    if !kept.overloaded || kept.overload_alternatives.is_empty() || members.len() != kept.overload_alternatives.len() + 1 {
        return Vec::new();
    }
    let type_arity_fits = |info: &FunctionSignatureInfo| {
        type_argument_count == 0
            || (type_argument_count <= info.type_parameters.len()
                && type_argument_count
                    >= info.type_parameters.iter().filter(|parameter| parameter.default_type.is_none()).count())
    };
    let fits = |info: &FunctionSignatureInfo, member: &FunctionType| {
        overload_arity_fits(member, argument_count) && type_arity_fits(info)
    };
    if fits(kept, &members[0]) {
        return Vec::new();
    }
    kept.overload_alternatives
        .iter()
        .zip(members.iter().skip(1))
        .filter(|(info, member)| fits(info, member))
        .map(|(info, _)| &**info)
        .collect()
}

/// `chooseOverload` for an inferred call to an overload group whose members
/// carry no written signature of their own (a namespace's folded overloads):
/// each declaration in order, instantiated through its own signature, the
/// first whose instantiation accepts the arguments answering the call.
pub(crate) fn select_overload_return_type_by_signatures(
    fold: &FunctionType,
    kept: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let members = fold.overloads()?;
    if !kept.overloaded || kept.overload_alternatives.is_empty() || members.len() != kept.overload_alternatives.len() + 1 {
        return None;
    }
    let shapes = inferred_argument_shapes(fold, arguments, symbols, ctx)?;
    let checkpoint = ctx.diagnostics().len();
    let candidates = std::iter::once(kept).chain(kept.overload_alternatives.iter().map(|alternative| &**alternative));
    let mut selected = None;
    for (candidate, member) in candidates.zip(members.iter()) {
        let (minimum, maximum) = declared_type_argument_arity(&candidate.type_parameters);
        if !overload_arity_fits(member, arguments.len())
            || !type_arguments.is_empty() && (type_arguments.len() < minimum || type_arguments.len() > maximum)
        {
            continue;
        }
        let instantiated =
            instantiate_function_type(fold, Some(candidate), &[], type_arguments, None, arguments, None, symbols, ctx);
        // Each candidate's literal arguments are read under its own parameters
        // (tsc checks them with that contextual type): `{ enum: ['A', 'C'] }`
        // is a tuple to `T extends Readonly<[U, ...U[]]>`, not a `string[]`.
        let contextual_shapes: Vec<ArgumentShape> = shapes
            .iter()
            .zip(arguments)
            .enumerate()
            .map(|(index, (shape, argument))| ArgumentShape {
                ty: literal_argument_type_for_candidate(candidate, index, argument, symbols, ctx)
                    .filter(|ty| !type_contains_unknown(ty))
                    .or_else(|| shape.ty.clone()),
                written_keys: shape.written_keys.clone(),
                element_keys: shape.element_keys.clone(),
                callback: shape.callback,
            })
            .collect();
        if signature_accepts_argument_types(&instantiated, &contextual_shapes, OverloadRelation::Assignable)
            && !names_open_parameter(instantiated.return_type())
        {
            selected = Some(instantiated.return_type().clone());
            break;
        }
    }
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    selected
}

/// The first of several candidates whose instantiation for this call accepts
/// its arguments — `chooseOverload`'s walk — or the first when none does. The
/// probes' diagnostics are discarded.
#[allow(clippy::too_many_arguments)]
fn first_accepting_overload<'a>(
    candidates: &[&'a FunctionSignatureInfo],
    fold: &FunctionType,
    type_arguments: &[ParsedType],
    callee_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> &'a FunctionSignatureInfo {
    let checkpoint = ctx.diagnostics().len();
    let shapes = inferred_argument_shapes(fold, arguments, symbols, ctx);
    let picked = shapes.and_then(|shapes| {
        candidates.iter().copied().find(|candidate| {
            let instantiated = instantiate_function_type(
                fold,
                Some(candidate),
                &[],
                type_arguments,
                callee_span,
                arguments,
                expected_return_type,
                symbols,
                ctx,
            );
            signature_accepts_argument_types(&instantiated, &shapes, OverloadRelation::Assignable)
        })
    });
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    picked.unwrap_or(candidates[0])
}

/// Where tsc anchors a type-argument error: the first type argument, just
/// past the `<` that follows the callee.
pub(crate) fn type_arguments_start(callee_span: Option<SyntaxTextSpan>) -> Option<SyntaxTextSpan> {
    callee_span.map(|span| SyntaxTextSpan { start: span.end + 1, end: span.end + 1 })
}

/// tsc's `reportCallResolutionErrors` when no candidate signature takes the
/// written type arguments (`hasCorrectTypeArgumentArity`): TS2558, or TS2743
/// when overloads exist on both sides of the count. `false` when some
/// signature takes them or when surge cannot tell how many type parameters a
/// signature declares; the call then proceeds.
pub(crate) fn report_type_argument_arity(
    callee: &Type,
    type_arguments: &[ParsedType],
    span: Option<SyntaxTextSpan>,
    shape_decides: bool,
    ctx: &mut CheckerContext,
) -> bool {
    if type_arguments.is_empty() {
        return false;
    }
    let signatures: Vec<FunctionType> = match callee {
        Type::Function(function) => function.overloads().map_or_else(|| vec![function.clone()], |members| members.to_vec()),
        Type::Object(object) => match object.call_signature() {
            Some(signature) => signature.overloads().map_or_else(|| vec![signature.clone()], |members| members.to_vec()),
            None => return false,
        },
        _ => return false,
    };
    let mut arities = Vec::with_capacity(signatures.len());
    for signature in &signatures {
        let Some(arity) = signature_type_argument_arity(signature, shape_decides) else {
            return false;
        };
        arities.push(arity);
    }
    report_type_argument_arities(&arities, type_arguments, span, ctx)
}

/// How many type arguments a declaration's type parameters take: (without
/// defaults, all).
fn declared_type_argument_arity(parameters: &[surge_ts_syntax::ParsedTypeParameter]) -> (usize, usize) {
    let minimum = parameters.iter().filter(|parameter| parameter.default_type.is_none()).count();
    (minimum, parameters.len())
}

/// TS2558/TS2743 when no signature of `arities` takes the written count.
fn report_type_argument_arities(
    arities: &[(usize, usize)],
    type_arguments: &[ParsedType],
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if type_arguments.is_empty() || arities.is_empty() {
        return false;
    }
    let count = type_arguments.len();
    if arities.iter().any(|&(minimum, maximum)| count >= minimum && count <= maximum) {
        return false;
    }
    let diagnostic = if let [(minimum, maximum)] = *arities {
        let expected = if minimum < maximum { format!("{minimum}-{maximum}") } else { minimum.to_string() };
        Diagnostic::ts2558(expected, count, ctx.file_name.clone())
    } else {
        let below = arities.iter().filter(|&&(_, maximum)| maximum < count).map(|&(_, maximum)| maximum).max();
        let above = arities.iter().filter(|&&(minimum, _)| minimum > count).map(|&(minimum, _)| minimum).min();
        match (below, above) {
            (Some(below), Some(above)) => Diagnostic::ts2743(count, below, above, ctx.file_name.clone()),
            (below, above) => Diagnostic::ts2558(below.or(above).unwrap_or(0).to_string(), count, ctx.file_name.clone()),
        }
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
    true
}

/// The declaration record of a signature written with no type parameters of
/// its own (`f(x: number)` in a class, interface or type literal). tsgo's
/// signature knows it takes no type arguments wherever it is reached; the
/// resolved handle otherwise carries neither a declaration nor a rendered
/// list to say so, and `c.f<string>(1)` went unchecked (TS2558).
pub(crate) struct DeclaredWithoutTypeParameters;

/// `function`, resolved from a written signature that declares no type
/// parameters, marked as taking no type arguments. A handle that already
/// records its declaration keeps it.
pub(crate) fn declared_without_type_parameters(function: FunctionType) -> FunctionType {
    static DECLARED: std::sync::LazyLock<std::sync::Arc<dyn std::any::Any + Send + Sync>> =
        std::sync::LazyLock::new(|| {
            let declared: std::sync::Arc<dyn std::any::Any + Send + Sync> =
                std::sync::Arc::new(DeclaredWithoutTypeParameters);
            declared
        });
    if function.declaration().is_some() || function.type_parameter_head().is_some() {
        return function;
    }
    function.with_declaration(std::sync::Arc::clone(&*DECLARED))
}

/// How many type arguments a signature takes, (without defaults, all): from
/// its declaration when the handle carries one, else from its rendered
/// type-parameter list. A signature with neither is generic only if its
/// shape still has holes, which leaves the count unknown — and only a value's
/// own type (`shape_decides`) is read that way: a member of an instantiated
/// or synthesized object may have lost the list it was declared with.
fn signature_type_argument_arity(function: &FunctionType, shape_decides: bool) -> Option<(usize, usize)> {
    if let Some(declaration) = function.declaration() {
        let parameters = if let Some(member) = declaration.downcast_ref::<DeclaredMemberSignature>() {
            &member.signature.type_parameters
        } else if let Some(signature) = declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>() {
            &signature.type_parameters
        } else if declaration.is::<DeclaredWithoutTypeParameters>() {
            return Some((0, 0));
        } else {
            return None;
        };
        return Some(declared_type_argument_arity(parameters));
    }
    if let Some(head) = function.type_parameter_head() {
        let segments = split_top_level_commas(head);
        let minimum = segments.iter().filter(|segment| !has_top_level_default(segment)).count();
        return Some((minimum, segments.len()));
    }
    if !shape_decides || type_contains_degradation(&Type::Function(function.clone())) {
        return None;
    }
    // `(…args: any[]) => any` is the permissive stand-in surge gives a value
    // it did not model, whatever the declaration's type parameters were.
    let any_like = |ty: &Type| match ty {
        Type::Any => true,
        Type::Array(element) => matches!(**element, Type::Any),
        _ => false,
    };
    if matches!(function.return_type(), Type::Any) && function.parameters().iter().all(any_like) {
        return None;
    }
    Some((0, 0))
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (index, character) in text.char_indices() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if !text[..index].ends_with('=') => depth -= 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                segments.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    segments.push(text[start..].trim());
    segments.retain(|segment| !segment.is_empty());
    segments
}

fn has_top_level_default(segment: &str) -> bool {
    let mut depth = 0i32;
    let bytes = segment.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        match byte {
            b'<' | b'(' | b'[' | b'{' => depth += 1,
            b'>' if index == 0 || bytes[index - 1] != b'=' => depth -= 1,
            b')' | b']' | b'}' => depth -= 1,
            b'=' if depth == 0 && bytes.get(index + 1) != Some(&b'>') && bytes.get(index + 1) != Some(&b'=') => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// `checkTypeArguments` for `new C<…>(…)`: the first written type argument
/// that does not satisfy its class type parameter's constraint instantiated
/// with every argument (defaults filling the rest), paired with that
/// constraint. tsc only gets there with the right number of type arguments
/// (TS2558 otherwise) and a construct signature fitting the argument count.
/// The class must be declared in the file being checked, so its constraints
/// and defaults resolve where they are written.
fn unsatisfied_class_type_argument(
    class: &crate::symbols::InterfaceInfo,
    callee: &ParsedExpression,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Type, Type)> {
    let ParsedExpression::Identifier { name, .. } = callee else {
        return None;
    };
    let type_parameters = &class.body.type_parameters;
    let minimum = type_parameters
        .iter()
        .filter(|parameter| parameter.default_type.is_none())
        .count();
    if *class.file_name != *ctx.file_name
        || type_arguments.len() < minimum
        || type_arguments.len() > type_parameters.len()
        || type_parameters[..type_arguments.len()]
            .iter()
            .all(|parameter| parameter.constraint.is_none())
    {
        return None;
    }
    let Type::Object(constructor) = symbols.get(name)?.ty.peeled() else {
        return None;
    };
    let construct = constructor.construct_signature()?;
    let fits = match construct.overloads() {
        Some(members) => members.iter().any(|member| arguments_fit(member, arguments)),
        None => arguments_fit(construct, arguments),
    };
    if !fits {
        return None;
    }
    let checkpoint = ctx.diagnostics().len();
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for (index, parameter) in type_parameters.iter().enumerate() {
        let resolved = match (type_arguments.get(index), &parameter.default_type) {
            (Some(argument), _) => {
                let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
                let resolved = crate::infer::map_parsed_type_with_substitution(
                    argument.clone(),
                    ctx,
                    &crate::infer::TypeParameterSubstitution::new(),
                );
                ctx.symbols = saved_symbols;
                resolved
            }
            (None, Some(default_type)) => crate::infer::map_parsed_type_with_substitution(
                default_type.clone(),
                ctx,
                &substitution,
            ),
            (None, None) => Type::Unknown,
        };
        substitution.insert(parameter.name.clone(), resolved);
    }
    let mut unsatisfied = None;
    for parameter in &type_parameters[..type_arguments.len()] {
        let (Some(constraint), Some(argument)) =
            (parameter.constraint.clone(), substitution.get(&parameter.name).cloned())
        else {
            continue;
        };
        let constraint =
            crate::infer::map_parsed_type_with_substitution(constraint, ctx, &substitution);
        if !crate::infer::types::constraint_relation_decidable(&argument, &constraint) {
            continue;
        }
        if !is_assignable_to(&argument, &constraint) {
            unsatisfied = Some((argument, constraint));
            break;
        }
    }
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    unsatisfied
}

/// tsc's `hasCorrectArity` for written arguments. A spread's count is its
/// operand's, so with one the spread's position must fall where the signature
/// still takes arguments: past the required ones, within the parameters or
/// into a rest.
fn arguments_fit(candidate: &FunctionType, arguments: &[ParsedCallArgument]) -> bool {
    match arguments.iter().position(|argument| argument.spread) {
        Some(index) => {
            index >= candidate.required_parameter_count()
                && (candidate.is_variadic() || index < candidate.parameters().len())
        }
        None => overload_arity_fits(candidate, arguments.len()),
    }
}

fn spread_operand_is_array(spread: &ParsedCallArgument, symbols: &SymbolTable, ctx: &mut CheckerContext) -> bool {
    if matches!(spread.expression, ParsedExpression::ArrayLiteral { .. }) {
        return false;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let operand = crate::infer::infer_expression(&spread.expression, symbols, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    matches!(operand, InferredExpression::Known(ty)
        if matches!(ty.peeled(), Type::Array(element) if !type_contains_unknown(&element)))
}

fn overload_arity_fits(candidate: &FunctionType, argument_count: usize) -> bool {
    let parameters = candidate.parameters();
    let mut required = candidate.required_parameter_count();
    while required > 0 && parameter_is_void_optional(&parameters[required - 1]) {
        required -= 1;
    }
    argument_count >= required && (candidate.is_variadic() || argument_count <= parameters.len())
}

/// The parameter an argument at `index` is checked against: the rest
/// parameter's element past the fixed ones, and an optional parameter with the
/// `undefined` a caller may pass it. `None` once the signature has run out.
fn effective_parameter_type(function_type: &FunctionType, index: usize) -> Option<Type> {
    let parameters = function_type.parameters();
    let expected = parameters.len();
    if function_type.is_variadic() && expected > 0 && index >= expected - 1 {
        // tsc relates the arguments a non-array rest type receives as one
        // gathered tuple (`getSpreadArgumentType`), not position by position.
        return match parameters[expected - 1].peeled() {
            Type::Array(element) => Some(*element),
            _ => None,
        };
    }
    let declared = parameters.get(index)?.clone();
    Some(
        if index >= function_type.required_parameter_count()
            && !is_assignable_to(&Type::Undefined, &declared)
        {
            union_type(vec![declared, Type::Undefined])
        } else {
            declared
        },
    )
}

/// Whether an overload group's implementation accepts the argument at `index`;
/// one surge could not type only reaches an `any` or `unknown` parameter.
fn implementation_accepts(implementation: &FunctionType, index: usize, argument: Option<&Type>) -> bool {
    let Some(parameter) = effective_parameter_type(implementation, index) else {
        return false;
    };
    matches!(parameter, Type::Any | Type::GenuineUnknown)
        || argument.is_some_and(|argument| {
            !type_contains_unknown(argument)
                && !type_contains_unknown(&parameter)
                && is_assignable_to(argument, &parameter)
        })
}

/// A rest parameter tsc relates to its arguments as one gathered tuple:
/// `start` is `getParameterCount - 1`, past the fixed elements a variadic
/// tuple leads with (those are positions of their own, `getTypeAtPosition`),
/// and `target` is `getNonArrayRestType`.
struct GatheredRest {
    start: usize,
    target: Type,
}

impl GatheredRest {
    /// The contextual type `getSpreadArgumentType` gives the argument at
    /// `position` of `count` gathered ones: a tuple's trailing element counted
    /// from the end, else its rest element (`getContextualTypeForElementExpression`),
    /// and the indexed element of any other rest type.
    fn contextual_element_type(&self, position: usize, count: usize) -> Type {
        match &self.target {
            Type::OpenTuple(tuple) => {
                let from_end = count - position;
                if from_end <= tuple.trailing.len() {
                    tuple.trailing[tuple.trailing.len() - from_end].clone()
                } else {
                    tuple.rest.as_ref().clone()
                }
            }
            other => rest_parameter_element_type(other, position),
        }
    }
}

/// relater.go `getNonArrayRestType` over `getEffectiveRestType`: a variadic
/// tuple is sliced at its fixed length, which leaves an array unless fixed
/// elements follow the variable one; a union of tuples, or a named object type
/// that is not the lib's array (an interface extending it), is gathered as it
/// is written.
fn gathered_rest_parameter(function_type: &FunctionType) -> Option<GatheredRest> {
    if !function_type.is_variadic() {
        return None;
    }
    let parameters = function_type.parameters();
    let rest = parameters.last()?;
    let rest_index = parameters.len() - 1;
    match rest.peeled() {
        Type::OpenTuple(tuple) if !tuple.trailing.is_empty() => Some(GatheredRest {
            start: rest_index + tuple.leading.len(),
            target: Type::OpenTuple(surge_ts_types::OpenTupleType {
                leading: Vec::new(),
                rest: tuple.rest,
                trailing: tuple.trailing,
            }),
        }),
        Type::Union(union)
            if union
                .types()
                .iter()
                .all(|member| matches!(member.peeled(), Type::Tuple(_) | Type::OpenTuple(_))) =>
        {
            Some(GatheredRest {
                start: rest_index,
                target: rest.clone(),
            })
        }
        Type::Object(_) if matches!(rest, Type::Reference(_)) && !names_lib_array(rest) => {
            Some(GatheredRest {
                start: rest_index,
                target: rest.clone(),
            })
        }
        // A rest written as no array at all (`...args: string`, TS2370) still
        // gathers its arguments. Only a signature resolved from a written type
        // is read this way: surge's own member tables store some rests as their
        // element (`concat(...strings)` as `string`).
        primitive @ (Type::String | Type::Number | Type::Boolean | Type::BigInt | Type::Symbol)
            if function_type
                .declaration()
                .is_some_and(|declaration| declaration.is::<DeclaredWithoutTypeParameters>()) =>
        {
            Some(GatheredRest {
                start: rest_index,
                target: primitive,
            })
        }
        _ => None,
    }
}

/// `getMutableArrayOrTupleType` of a spread argument, over each member of a
/// union: a tuple is its mutable self, and any other array-like its element
/// type's mutable array. `None` for what surge cannot read.
fn spread_argument_array(spread: &Type) -> Option<Type> {
    if spread.is_unknown() || type_contains_degradation(spread) || spread.is_type_variable() {
        return None;
    }
    if let Type::Reference(reference) = spread
        && reference.is_readonly_array()
    {
        return spread_argument_array(&reference.resolve());
    }
    match spread.peeled() {
        Type::Union(union) => {
            let members: Option<Vec<Type>> = union.types().iter().map(spread_argument_array).collect();
            Some(union_type(members?))
        }
        tuple @ Type::Tuple(_) => Some(tuple),
        Type::Array(element) => Some(Type::Array(element)),
        Type::Object(object) if object.number_index_type.is_some() => {
            object.number_index_type.as_deref().map(|element| Type::Array(Box::new(element.clone())))
        }
        other => {
            let element = crate::checks::function::for_of_element_type(&other);
            (!element.is_unknown()).then(|| Type::Array(Box::new(element)))
        }
    }
}

/// `isArrayType`: the lib's `Array<T>` or `ReadonlyArray<T>` written by name.
fn names_lib_array(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference)
        if reference.arguments.len() == 1
            && matches!(reference.id.split('\u{0}').next_back(), Some("Array" | "ReadonlyArray")))
}

/// Whether a gathered rest's target still carries a hole surge could not
/// fill, in any element of a tuple or member of a union. A recursive alias
/// deeper than the bound counts as one.
fn gathered_rest_is_unmodelled(ty: &Type, depth: usize) -> bool {
    if depth > 4 {
        return true;
    }
    let unmodelled = |ty: &Type| gathered_rest_is_unmodelled(ty, depth + 1);
    match ty.peeled() {
        Type::Tuple(elements) => elements.iter().any(unmodelled),
        Type::OpenTuple(tuple) => tuple
            .leading
            .iter()
            .chain(std::iter::once(tuple.rest.as_ref()))
            .chain(tuple.trailing.iter())
            .any(unmodelled),
        Type::Union(union) => union.types().iter().any(unmodelled),
        // An uninferred type argument is a real `unknown` (`CoolArray<unknown>`).
        other => type_contains_degradation(&other) || surge_ts_types::parameter_type_is_degraded(&other),
    }
}

fn rest_parameter_element_type(parameter_type: &Type, rest_offset: usize) -> Type {
    match parameter_type {
        Type::Array(element) => element.as_ref().clone(),
        // A tuple-typed rest parameter (`...args: [name: string]`) accepts each
        // argument at its tuple position; an overload folded into a union of
        // tuples (`...args: [string] | [RequestCookie]`, next's cookie store)
        // accepts the union of the per-position elements. Without these arms the
        // whole tuple/union was compared against each single argument — a false
        // TS2345 on every call.
        Type::Tuple(elements) => elements
            .get(rest_offset)
            .or_else(|| elements.last())
            .cloned()
            .unwrap_or(Type::Any),
        // `...x: [number, string, ...boolean[]]` expands to the positional
        // parameters the tuple spells out, then to what its own rest holds —
        // a trailing element cannot be placed without knowing the argument
        // count, so it joins the rest.
        Type::OpenTuple(tuple) => tuple.leading.get(rest_offset).cloned().unwrap_or_else(|| {
            let mut members = vec![tuple.rest.as_ref().clone()];
            members.extend(tuple.trailing.iter().cloned());
            union_type(members)
        }),
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| rest_parameter_element_type(member, rest_offset))
                .collect(),
        ),
        Type::Reference(_) => rest_parameter_element_type(&parameter_type.peeled(), rest_offset),
        // A generic tuple places an argument only in its leading fixed
        // elements; past a variadic one the position depends on what the
        // variable is instantiated with.
        Type::TypeParameter(_) => match surge_ts_types::type_variable::generic_tuple(parameter_type) {
            Some((elements, _)) => {
                use surge_ts_types::type_variable::TupleElementKind;
                elements
                    .iter()
                    .take_while(|(kind, _)| matches!(kind, TupleElementKind::Required | TupleElementKind::Optional))
                    .nth(rest_offset)
                    .map_or(Type::Unknown, |(_, ty)| ty.clone())
            }
            None => parameter_type.clone(),
        },
        other => other.clone(),
    }
}

thread_local! {
    /// The `this` argument of the call about to be checked
    /// (`getThisArgumentType`): `void` for a plain call, the receiver for a
    /// member call, with where to report it. The signature check takes it, so
    /// a call nested in the arguments sets its own.
    static CALL_THIS_ARGUMENT: std::cell::RefCell<Option<(Type, Option<SyntaxTextSpan>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Makes `this_argument` the next checked call's `this` argument until dropped.
pub(crate) struct CallThisArgument(Option<(Type, Option<SyntaxTextSpan>)>);

impl CallThisArgument {
    pub(crate) fn set(this_argument: Type, span: Option<SyntaxTextSpan>) -> Self {
        Self(CALL_THIS_ARGUMENT.with(|current| current.replace(Some((this_argument, span)))))
    }
}

impl Drop for CallThisArgument {
    fn drop(&mut self) {
        let outer = self.0.take();
        CALL_THIS_ARGUMENT.with(|current| *current.borrow_mut() = outer);
    }
}

/// `checkApplicableSignature`'s `this` check (TS2684): a signature whose
/// written `this` is not `void` takes the call's `this` argument.
/// `getSignatureApplicabilityError` relates the `this` argument before any
/// other: whether it reported is whether the call already failed. Of an
/// overload group, `chooseOverload` tries the candidates whose arity fits — a
/// lone one fails as itself, and several that all reject `this` are TS2769.
fn check_call_this_argument(
    function_type: &FunctionType,
    arguments: &[ParsedCallArgument],
    ctx: &mut CheckerContext,
) -> bool {
    let Some((this_argument, span)) = CALL_THIS_ARGUMENT.with(|current| current.borrow_mut().take()) else {
        return false;
    };
    if this_argument.is_unmodelled() {
        return false;
    }
    let rejects = |signature: &FunctionType| {
        signature.this_type().is_some_and(|this_type| {
            !matches!(this_type, Type::Void) && !this_type.is_unmodelled() && !is_assignable_to(&this_argument, this_type)
        })
    };
    let signature = match function_type.overloads() {
        None => function_type,
        Some(_) if arguments.iter().any(|argument| argument.spread) => return false,
        Some(members) => {
            let fitting: Vec<&FunctionType> =
                members.iter().filter(|member| overload_arity_fits(member, arguments.len())).collect();
            match fitting.as_slice() {
                [only] => *only,
                [] => return false,
                several => {
                    if !several.iter().all(|member| rejects(member)) {
                        return false;
                    }
                    ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2769(ctx.file_name.clone()), span));
                    return true;
                }
            }
        }
    };
    let Some(this_type) = signature.this_type().filter(|_| rejects(signature)) else {
        return false;
    };
    let (source_name, target_name) = (this_argument.name(), this_type.name());
    let diagnostic =
        crate::checks::expr::missing_properties_report(&this_argument, this_type, &source_name, &target_name, &ctx.file_name)
            .unwrap_or_else(|| Diagnostic::ts2684(source_name, target_name, ctx.file_name.clone()));
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
    true
}

pub(crate) fn check_function_type_call(
    function_type: &FunctionType,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let implementation = CALLEE_IMPLEMENTATION.with(|cell| cell.take());
    let this_rejected = check_call_this_argument(function_type, arguments, ctx);
    let combined_union_signature = COMBINED_UNION_SIGNATURE.with(|flag| flag.replace(false));
    let never_from_broken_constraint = instantiate::take_never_from_broken_constraint(arguments);
    // tsc resolves an overloaded call against the candidates whose arity fits
    // (`hasCorrectArity`). With exactly one, a mismatch is that candidate's own
    // argument error; with several, it is TS2769. The permissive fold stays the
    // signature arguments are checked against whenever a candidate is not fully
    // modelled, since a sentinel parameter would reject what tsc accepts.
    let mut arity_candidates = 0;
    let mut fitting_candidates: Vec<FunctionType> = Vec::new();
    if let Some(members) = function_type.overloads()
        && !arguments.iter().any(|argument| argument.spread)
    {
        // Written type arguments leave only the candidates taking that many
        // (`hasCorrectTypeArgumentArity`); a lone one is instantiated with
        // them, and the call is that candidate's. Only a call with arguments
        // to relate: the instantiated return alone is not surge's to trust
        // (`expectTypeOf<T>().toBeUnknown()`).
        if !type_arguments.is_empty() && !arguments.is_empty() {
            let typed: Vec<&FunctionType> = members
                .iter()
                .filter(|member| {
                    overload_arity_fits(member, arguments.len())
                        && member.overloads().is_none()
                        && own_type_parameter_names(member).len() == type_arguments.len()
                })
                .collect();
            // `chooseOverload` passes over a candidate whose constraints the
            // written type arguments break (`checkTypeArguments`).
            let mut rejection = None;
            let viable: Vec<&FunctionType> = if typed.len() > 1 {
                let checkpoint = ctx.diagnostics().len();
                let viable = typed
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        let Some(signature) = declared_signature_info(candidate) else {
                            return true;
                        };
                        match unsatisfied_type_argument(&signature, type_arguments, symbols, ctx) {
                            Some(pair) => {
                                rejection = Some(pair);
                                false
                            }
                            None => true,
                        }
                    })
                    .collect();
                ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                viable
            } else {
                typed
            };
            // Every candidate rejecting the written type arguments: the last
            // is reported (`candidateForTypeArgumentError`), at the first one.
            if viable.is_empty()
                && let Some((argument_type, constraint_type)) = rejection
            {
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts2344(
                        source_display_name(&argument_type, &constraint_type),
                        constraint_type.name(),
                        ctx.file_name.clone(),
                    ),
                    callee_span.map(|span| SyntaxTextSpan {
                        start: span.end + 1,
                        end: span.end + 1,
                    }),
                ));
                for argument in arguments {
                    let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                }
                return None;
            }
            if let [chosen] = viable.as_slice() {
                let instantiated = instantiate_overload_member(
                    chosen,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                );
                if own_type_parameter_names(&instantiated).is_empty()
                    && !instantiated.parameters().iter().any(type_contains_unknown)
                {
                    COMBINED_UNION_SIGNATURE.with(|flag| flag.set(combined_union_signature));
                    CALLEE_IMPLEMENTATION.with(|cell| *cell.borrow_mut() = implementation.clone());
                    return check_function_type_call(
                        &instantiated,
                        callee_span,
                        call_span,
                        &[],
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    );
                }
            }
        }
        let fitting: Vec<&FunctionType> = members
            .iter()
            .filter(|member| overload_arity_fits(member, arguments.len()))
            .collect();
        arity_candidates = fitting.len();
        fitting_candidates = fitting
            .iter()
            .map(|candidate| (*candidate).clone())
            .collect();
        // A lone generic candidate is inferred for this call first, as
        // `chooseOverload` instantiates each candidate it tries.
        let lone_generic = match fitting.as_slice() {
            [chosen]
                if type_arguments.is_empty()
                    && chosen.overloads().is_none()
                    && !own_type_parameter_names(chosen).is_empty() =>
            {
                Some(instantiate_overload_member(
                    chosen,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                ))
            }
            _ => None,
        };
        // A written `unknown` (`Promise<unknown>`'s `(value: unknown) => U`) is
        // a type there; the arguments' own checks decline what they cannot
        // judge.
        if let Some(instantiated) = lone_generic
            && !instantiated.parameters().iter().any(|parameter| {
                type_contains_degradation(parameter) || matches!(parameter, Type::Never)
            })
        {
            COMBINED_UNION_SIGNATURE.with(|flag| flag.set(combined_union_signature));
            CALLEE_IMPLEMENTATION.with(|cell| *cell.borrow_mut() = implementation.clone());
            return check_function_type_call(
                &instantiated,
                callee_span,
                call_span,
                type_arguments,
                arguments,
                expected_return_type,
                symbols,
                ctx,
            );
        }
        if let [chosen] = fitting.as_slice()
            && chosen.overloads().is_none()
            && !chosen.parameters().iter().any(|parameter| {
                type_contains_degradation(parameter)
                    || matches!(parameter, Type::Never) && !combined_union_signature
            })
        {
            COMBINED_UNION_SIGNATURE.with(|flag| flag.set(combined_union_signature));
            CALLEE_IMPLEMENTATION.with(|cell| *cell.borrow_mut() = implementation.clone());
            return check_function_type_call(
                chosen,
                callee_span,
                call_span,
                type_arguments,
                arguments,
                expected_return_type,
                symbols,
                ctx,
            );
        }
    }
    // tsc's `getParameterCount`: a rest parameter typed by a fixed tuple
    // stands for one parameter per element, the optional ones trailing.
    let literal_rest_arity = function_literal_rest_arity(function_type, arguments);
    let fixed_rest_arity = literal_rest_arity.is_none().then(|| fixed_tuple_rest_arity(function_type)).flatten();
    let rest_tuple_arity = literal_rest_arity
        .map(|(_, total)| total)
        .or(fixed_rest_arity.map(|(_, total)| total));
    let variadic = function_type.is_variadic() && rest_tuple_arity.is_none();
    let expected = function_type.parameters().len();
    let arity = rest_tuple_arity.unwrap_or(expected);
    let actual = arguments.len();
    let mut has_unresolved_argument = false;
    let mut mismatch_reported = this_rejected;
    let mut overload_failure_argument: Option<usize> = None;

    // A trailing parameter typed `void` (or a union containing `void`) is optional
    // at the call site — `cb()` is valid for `cb: (x: void) => void`, and a
    // `Promise<void>` executor's `resolve: (value: void | PromiseLike<void>) => void`
    // accepts `resolve()`.
    // A trailing parameter surge could not type (`unknown | PromiseLike<T>`
    // with `T` still open — a Promise executor read through a polluted
    // instantiation) may well be that `void`, so it cannot count as required.
    // An overload group's minimum is its candidates' smallest
    // (`getArgumentArityError`), not its fold's, whose slot the overloads
    // disagree on may be the sentinel.
    let required = match function_type.overloads() {
        Some(members) => members.iter().map(call_site_required_count).min().unwrap_or(0),
        None => call_site_required_count(function_type),
    };
    let required = literal_rest_arity.map_or(required, |(literal_required, _)| required.max(literal_required));
    let required = fixed_rest_arity.map_or(required, |(fixed_required, _)| required.max(fixed_required));

    // `f(...xs)` supplies as many arguments as the spread's type has elements,
    // which is one for a tuple of one and any number for an array. Counting the
    // spread as a single argument made `three(...tupleOfThree)` a false TS2554,
    // so a call carrying one has no statically known count and skips the check —
    // except an array literal spread, a tuple (`isSpreadIntoCallOrNew`) that
    // tsc's `getEffectiveCallArguments` expands into one argument per element.
    // Only the too-few bound is checked for it, since an excess element has no
    // argument node of its own to anchor the error on.
    let has_spread_argument = arguments.iter().any(|argument| argument.spread);
    // `resolveCall`: when no overload `hasCorrectArity` for a call whose first
    // spread is an array, `getArgumentArityError` reports TS2556 on it. Only
    // an array spread is taken, so positions before it are the written ones.
    if let Some(members) = function_type.overloads()
        && let Some(spread) = arguments.iter().find(|argument| argument.spread)
        && !members.iter().any(|member| arguments_fit(member, arguments))
        && spread_operand_is_array(spread, symbols, ctx)
    {
        ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2556(ctx.file_name.clone()), spread.span));
        for argument in arguments {
            ctx.degraded_expected_type_depth += 1;
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            ctx.degraded_expected_type_depth -= 1;
        }
        return None;
    }
    let effective = if has_spread_argument {
        effective_spread_arguments(arguments, symbols, ctx)
    } else {
        None
    };
    let (actual, too_many) = match &effective {
        Some(effective) => (
            effective.count,
            !variadic && effective.count > arity,
        ),
        None => (actual, !variadic && actual > arity),
    };
    let arity_known = !has_spread_argument || effective.is_some();
    if arity_known && (actual < required || too_many) {
        let expected_count = if actual < required {
            required
        } else {
            arity
        };
        // tsc anchors a too-many-arguments error on the excess arguments (from the
        // first excess argument through the last), not on the call expression.
        let span = if too_many {
            let first_excess = effective
                .as_ref()
                .and_then(|effective| effective.source_of(arity))
                .unwrap_or(arity);
            excess_argument_span(arguments, first_excess)
                .or(call_span)
                .or(callee_span)
        } else {
            call_span.or(callee_span)
        };
        // tsc's `getArgumentArityError`: a rest parameter makes the minimum the
        // only bound, and optional parameters widen the count to a range.
        let diagnostic = if actual < required && variadic {
            Diagnostic::ts2555(required, actual, ctx.file_name.clone())
        } else if required < arity && !variadic && (rest_tuple_arity.is_none() || fixed_rest_arity.is_some()) {
            Diagnostic::ts2554(
                format!("{required}-{arity}"),
                actual,
                ctx.file_name.clone(),
            )
        } else {
            Diagnostic::ts2554(expected_count, actual, ctx.file_name.clone())
        };
        ctx.push(diagnostic_with_syntax_span(diagnostic, span));
        // tsc still checks every argument of a call it rejected for arity. One
        // the signature covers is contextually typed by it, which surge cannot
        // apply without also relating it, so its callbacks stay unreported; an
        // excess argument has no context at all.
        for (index, argument) in arguments.iter().enumerate() {
            let covered = index < expected || function_type.is_variadic();
            if covered {
                ctx.degraded_expected_type_depth += 1;
            }
            let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
            if covered {
                ctx.degraded_expected_type_depth -= 1;
            }
        }
        // `getCandidateForOverloadFailure`: a lone non-generic candidate is
        // the signature the rejected call resolves to, so its return type is
        // still the call's.
        let lone_candidate = function_type.overloads().is_none()
            && own_type_parameter_names(function_type).is_empty()
            && type_arguments.is_empty()
            && !type_contains_unknown(function_type.return_type());
        return lone_candidate.then(|| function_type.return_type().clone());
    }

    // tsc's `isSignatureApplicable`: a rest parameter whose type is not an
    // array (`getNonArrayRestType`) is related to the arguments it receives
    // gathered into one tuple (`getSpreadArgumentType`), not one at a time. A
    // spread argument or an overload fold keeps the positional reading.
    let gathered_rest = if has_spread_argument || function_type.overloads().is_some() {
        None
    } else {
        gathered_rest_parameter(function_type).filter(|rest| rest.start <= arguments.len())
    };
    let mut gathered_types: Option<Vec<Type>> = Some(Vec::new());

    // What each argument evaluated to, for overload selection afterwards. A
    // `None` type is a wildcard: a callback (typed by whichever overload is
    // picked, so it cannot pick), a degraded or unresolved argument, or a
    // spread.
    let mut argument_types: Vec<ArgumentShape> = Vec::with_capacity(arguments.len());

    // `addImplementationSuccessElaboration`: tsc retries a rejected call
    // against the overloads' implementation, which contextually types each
    // argument no candidate reached by the implementation's parameter, until
    // the implementation rejects one too.
    let mut implementation_pending = implementation.filter(|implementation| {
        !has_spread_argument
            && overload_arity_fits(implementation, arguments.len())
            && !implementation
                .parameters()
                .iter()
                .any(surge_ts_types::type_variable::mentions_type_variable)
    });
    let mut implementation_context: Option<FunctionType> = None;
    let mut implementation_types: Vec<Option<Type>> = Vec::new();

    // tsc's `getEffectiveCallArguments`: a tuple spread stands for one argument
    // per element, an array spread for one argument of its element type, and
    // what follows lines up with the parameters after them.
    let mut i = 0usize;
    for argument in arguments.iter() {
        if argument.spread {
            let spread_result = match &argument.expression {
                // tsc's `checkArrayLiteral`: an array literal spread straight
                // into a call is a tuple (`isSpreadIntoCallOrNew`), one argument
                // per element.
                ParsedExpression::ArrayLiteral { elements, .. }
                    if elements.iter().all(|element| !element.spread) =>
                {
                    InferredExpression::Known(Type::Tuple(
                        elements
                            .iter()
                            .map(|element| {
                                match evaluate_expression(&element.expression, element.span, symbols, ctx) {
                                    InferredExpression::Known(ty) => ty,
                                    _ => Type::Unknown,
                                }
                            })
                            .collect(),
                    ))
                }
                _ => evaluate_expression(&argument.expression, argument.span, symbols, ctx),
            };
            crate::checks::expr::check_iterable_operand(
                &spread_result,
                argument.expression_span,
                true,
                ctx,
            );
            argument_types.push(ArgumentShape::wildcard());
            // Each element the spread supplies, and whether it is a variable
            // run (an array, or a tuple's `...T[]` part) rather than one fixed
            // argument.
            let spread_elements: Vec<(Type, bool)> = match &spread_result {
                // `getEffectiveCallArguments` expands only a tuple type, never
                // a type variable, whatever its constraint.
                InferredExpression::Known(spread) if spread.is_type_variable() => vec![(Type::Any, true)],
                InferredExpression::Known(spread) => match spread.peeled() {
                    Type::Tuple(elements) => elements.into_iter().map(|element| (element, false)).collect(),
                    // A tuple's variable part is only judged by the parameter
                    // it lands on: surge's own rest modelling of the callee
                    // (an IIFE's `...rest` included) is not reliable enough to
                    // call its position out of range.
                    Type::OpenTuple(tuple) => tuple
                        .leading
                        .iter()
                        .map(|element| (element.clone(), false))
                        .chain(std::iter::once((tuple.rest.as_ref().clone(), false)))
                        .chain(tuple.trailing.iter().map(|element| (element.clone(), false)))
                        .collect(),
                    // `getSpreadArgumentType`: any spread that is not a
                    // tuple is a variable run of its iterated type.
                    other => {
                        let element = crate::checks::function::for_of_element_type(&other);
                        vec![(element, true)]
                    }
                },
                // A name or member that did not resolve is tsc's error type,
                // an `any` that is no tuple.
                InferredExpression::UnresolvedIdentifier { .. } | InferredExpression::MissingProperty { .. } => {
                    vec![(Type::Any, true)]
                }
                _ => vec![(Type::Unknown, false)],
            };
            // surge reads a spread as `any[]` where tsc can have a tuple (a
            // contextual rest parameter, a typed field), so an `any` run is
            // only taken from a type variable or a binding written with a type.
            let trusted_any = matches!(&spread_result, InferredExpression::Known(spread) if spread.is_type_variable())
                || matches!(
                    &spread_result,
                    InferredExpression::UnresolvedIdentifier { .. } | InferredExpression::MissingProperty { .. }
                )
                || matches!(&argument.expression, ParsedExpression::Identifier { name, .. }
                    if crate::checks::function::binding_has_explicit_type(name, ctx) == Some(true));
            for (element, variable) in spread_elements {
                // `hasCorrectArity`: a variable run may only begin where every
                // required parameter is already supplied and a parameter is
                // still there to receive it.
                if variable
                    && match &element {
                        Type::Any => trusted_any,
                        Type::GenuineUnknown => true,
                        element => !type_contains_unknown(element),
                    }
                    && !mismatch_reported
                    && function_type.overloads().is_none()
                    && (i < function_type.required_parameter_count()
                        || (!function_type.is_variadic() && i >= expected))
                {
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2556(ctx.file_name.clone()),
                        argument.span,
                    ));
                    mismatch_reported = true;
                }
                // A union's combined parameter is synthetic and carries no
                // `undefined` of its own (`combineUnionParameters`).
                let parameter_type = if combined_union_signature
                    && !(function_type.is_variadic() && i + 1 >= expected)
                {
                    function_type.parameters().get(i).cloned()
                } else {
                    effective_parameter_type(function_type, i)
                };
                if !mismatch_reported
                    && !element.is_unknown()
                    && function_type.overloads().is_none()
                    && let Some(parameter_type) = parameter_type
                    // `any` relates to everything but `never`, which only a
                    // spread written as `any` is trusted to reach.
                    && (!matches!(element, Type::Any) || (trusted_any && matches!(parameter_type, Type::Never)))
                    && !type_contains_unknown(&parameter_type)
                    && !surge_ts_types::parameter_type_is_degraded(&parameter_type)
                    && !type_contains_unknown(&element)
                    && !is_assignable_to(&element, &parameter_type)
                {
                    let reported_parameter =
                        crate::checks::expr::reported_relation_target(&element, &parameter_type);
                    let element_name = source_display_name(&element, &reported_parameter);
                    ctx.push(diagnostic_with_syntax_span(
                        crate::checks::expr::assignability_mismatch_diagnostic(
                            &element,
                            &parameter_type,
                            &element_name,
                            &reported_parameter.name(),
                            true,
                            ctx.file_name.clone(),
                        ),
                        argument.span,
                    ));
                    mismatch_reported = true;
                }
                i += 1;
            }
            continue;
        }
        let i = {
            let position = i;
            i += 1;
            position
        };

        if mismatch_reported
            && let Some(implementation) = implementation_pending.take()
            && implementation_types
                .iter()
                .enumerate()
                .all(|(index, argument_type)| implementation_accepts(&implementation, index, argument_type.as_ref()))
        {
            implementation_context = Some(implementation);
        }
        let implementation_parameter = implementation_context
            .as_ref()
            .and_then(|implementation| effective_parameter_type(implementation, i));
        if implementation_parameter.is_none() {
            implementation_context = None;
        }

        // For a variadic signature the trailing rest parameter (declared as an
        // array) matches each remaining argument against its *element* type, not
        // the array itself — `cn(...inputs: string[])` accepts `cn("a", "b")`.
        let is_rest_position = function_type.is_variadic() && expected > 0 && i >= expected - 1;
        let gathered = gathered_rest.as_ref().filter(|rest| i >= rest.start);
        let parameter_type: Type = if let Some(parameter) = implementation_parameter {
            parameter
        } else if let Some(rest) = gathered {
            rest.contextual_element_type(i - rest.start, arguments.len() - rest.start)
        } else if is_rest_position {
            rest_parameter_element_type(
                &function_type.parameters()[expected - 1],
                i - (expected - 1),
            )
        } else if i < expected {
            let declared = function_type.parameters()[i].clone();
            // An optional parameter (`x?: T`, declared past the required count)
            // accepts `undefined` at the call site — passing `T | undefined` to it
            // is valid, matching tsc. Widen so a provided argument is checked
            // against `T | undefined` rather than the bare `T`. A union's
            // combined parameters are synthetic and carry no `undefined` of their
            // own (`combineUnionParameters`).
            if i >= function_type.required_parameter_count()
                && !combined_union_signature
                && !is_assignable_to(&Type::Undefined, &declared)
            {
                union_type(vec![declared, Type::Undefined])
            } else {
                declared
            }
        } else {
            Type::Any
        };

        // tsc reports a call's first inapplicable argument and stops; later
        // arguments are still evaluated (their own diagnostics and contextual
        // typing survive), only their mismatch is withheld.
        let argument_diagnostic_span = argument.span.map(crate::context::convert_span);
        let outer_suppressed = ctx.suppressed_argument_mismatch_span;
        if mismatch_reported {
            ctx.suppressed_argument_mismatch_span = argument_diagnostic_span;
        }
        let diagnostics_before = ctx.diagnostics.len();
        let inferred_argument = evaluate_expression_with_expected_type(
            &argument.expression,
            argument.span,
            Some(&parameter_type),
            if mismatch_reported || gathered.is_some() {
                ExpectedTypeDiagnostic::ContextOnly
            } else {
                ExpectedTypeDiagnostic::ArgumentNotAssignable
            },
            symbols,
            ctx,
        );
        let inferred_argument = match inferred_argument {
            InferredExpression::Known(Type::Any)
                if matches!(parameter_type.peeled(), Type::Function(_)) =>
            {
                instantiate::construct_only_argument_type(&argument.expression, symbols, ctx)
                    .map(InferredExpression::Known)
                    .unwrap_or(InferredExpression::Known(Type::Any))
            }
            other => other,
        };
        if implementation_pending.is_some() || implementation_context.is_some() {
            let argument_type = match &inferred_argument {
                InferredExpression::Known(argument_type) => Some(argument_type.clone()),
                _ => None,
            };
            if let Some(implementation) = &implementation_context
                && !implementation_accepts(implementation, i, argument_type.as_ref())
            {
                implementation_context = None;
            }
            implementation_types.push(argument_type);
        }
        ctx.suppressed_argument_mismatch_span = outer_suppressed;
        if arity_candidates > 1 && !mismatch_reported && retag_overload_mismatches(diagnostics_before, ctx) {
            mismatch_reported = true;
        }
        // A literal argument's mismatch is elaborated into it (TS2741 on the
        // literal, TS2322 on a member), which is still the call's first
        // inapplicable argument.
        if !mismatch_reported
            && matches!(argument.expression, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. })
            && let Some(argument_span) = argument_diagnostic_span
            && ctx.diagnostics[diagnostics_before..].iter().any(|diagnostic| {
                matches!(
                    diagnostic.code,
                    surge_ts_diagnostics::DiagnosticCode::TypeScript(2322 | 2353 | 2561 | 2739 | 2740 | 2741)
                ) && diagnostic
                    .span
                    .is_some_and(|span| argument_span.start <= span.start && span.end <= argument_span.end)
            })
        {
            mismatch_reported = true;
        }
        if !mismatch_reported
            && ctx.diagnostics[diagnostics_before..]
                .iter()
                .any(|diagnostic| {
                    matches!(
                        diagnostic.code,
                        surge_ts_diagnostics::DiagnosticCode::TypeScript(2345)
                    ) && diagnostic.span.is_some()
                        && diagnostic.span == argument_diagnostic_span
                })
        {
            mismatch_reported = true;
        }

        argument_types.push(ArgumentShape {
            ty: match &inferred_argument {
                InferredExpression::Known(argument_type)
                    if !argument_is_callback(&argument.expression)
                        && !type_contains_unknown(argument_type) =>
                {
                    Some(argument_type.clone())
                }
                _ => None,
            },
            written_keys: written_object_keys(&argument.expression),
            element_keys: written_element_keys(&argument.expression),
            callback: argument_is_callback(&argument.expression),
        });

        if gathered.is_some() {
            gathered_types = match (&inferred_argument, gathered_types.take()) {
                (InferredExpression::Known(argument_type), Some(mut types))
                    if !argument_is_callback(&argument.expression)
                        && !type_contains_unknown(argument_type)
                        && !as_source(|| type_contains_degradation(argument_type))
                        && !is_open_instantiation(argument_type) =>
                {
                    types.push(argument_type.clone());
                    Some(types)
                }
                _ => None,
            };
        }

        match inferred_argument {
            InferredExpression::Known(argument_type) => {
                // The sentinel and an open type parameter say nothing about
                // the source; the `unknown` keyword does — tsc rejects it for
                // every parameter that is not `unknown`/`any`. A type variable
                // of the body being checked is a type like any other.
                if matches!(argument_type, Type::Unknown)
                    || matches!(&argument_type, Type::TypeParameter(parameter) if !parameter.is_active_variable())
                    // surge narrows the variable, not its constraint, so such
                    // a reference's flow type under this contextual type is
                    // not modelled.
                    || (surge_ts_types::type_variable::is_generic_with_union_constraint(&argument_type)
                        && !surge_ts_types::type_variable::mentions_type_variable(&parameter_type)
                        && matches!(
                            argument.expression,
                            ParsedExpression::Identifier { .. }
                                | ParsedExpression::PropertyAccess { .. }
                                | ParsedExpression::ElementAccess { .. }
                        ))
                    || mismatch_reported
                    || gathered.is_some()
                {
                    continue;
                }
                let genuine_unknown_argument = matches!(argument_type, Type::GenuineUnknown);

                // A `never` parameter is an exhaustiveness assertion
                // (`util.assertNever(check)`): reporting it requires having
                // narrowed the argument to `never`, which surge's narrowing only
                // under-approximates, so every incompletely-narrowed residual
                // would read as a false positive.
                // A declared parameter that still carries holes surge could
                // not fill — an unsubstituted type parameter, or a reference to
                // a generic declaration written without arguments whose members
                // were built from its own parameters — cannot reject anything.
                // `type_contains_unknown` does not see through a reference;
                // this predicate peels one level, as the signature comparison
                // already does for the same shape.
                // A rest slot of `never` (`push` on a `never[]`) is no such
                // assertion, so it is checked like any other, and neither is an
                // argument written as a literal, which no narrowing reaches. Nor
                // is the `never` a union's combined signature takes where its
                // members' parameters are disjoint, or the `never` constraint
                // inference put in place of a candidate breaking it.
                let read_parameter = parameter_read_by_argument(&parameter_type, &argument_type);
                if (!matches!(parameter_type, Type::Never)
                    || never_from_broken_constraint
                    || is_rest_position
                    || combined_union_signature
                    || is_unnarrowable_literal(&argument.expression))
                    && ((!parameter_holds_hole(&read_parameter)
                        && !surge_ts_types::parameter_type_is_degraded(&read_parameter)
                        && (genuine_unknown_argument
                            || !as_source(|| type_contains_degradation(&argument_type))))
                        || crate::checks::assign::definite_unit_member_mismatch(
                            &argument_type,
                            &parameter_type,
                        )
                        || crate::checks::assign::definite_type_variable_member_mismatch(
                            &argument_type,
                            &parameter_type,
                        )
                        // Whatever holes an object or function type carries,
                        // it never takes `null` or `undefined`, written or read.
                        || (surge_ts_types::strict_null_checks()
                            && (matches!(
                                argument.expression,
                                ParsedExpression::NullLiteral | ParsedExpression::UndefinedLiteral
                            ) || matches!(argument_type, Type::Null | Type::Undefined))
                            && match parameter_type.peeled() {
                                Type::Function(_) | Type::Array(_) | Type::Tuple(_) => true,
                                Type::Object(object) => !object.is_intersection && !object.synthetic_open_index,
                                _ => false,
                            })
                        // Not at a rest position: a rest type that is not an
                        // array is related to the gathered arguments as a
                        // whole, which this per-argument pairing is not.
                        || (!is_rest_position
                            && crate::checks::assign::definite_primitive_member_mismatch(
                                &argument_type,
                                &parameter_type,
                            ))
                        || type_variable_decides(&argument_type, &parameter_type)
                        || generic_rest_callback_decides(&argument_type, &parameter_type)
                        || instantiate::lacks_constraint_call_signature(
                            &argument_type,
                            &parameter_type,
                        ))
                    && (!is_open_instantiation(&argument_type)
                        || definitely_mismatches(&argument_type, &parameter_type))
                    && !is_assignable_to(&argument_type, &parameter_type)
                {
                    let reported_parameter = crate::checks::expr::reported_relation_target(
                        &argument_type,
                        &parameter_type,
                    );
                    let argument_type_name =
                        source_display_name(&argument_type, &reported_parameter);
                    let parameter_type_name = reported_parameter.name();
                    // Every fitting candidate's parameter at this position is a
                    // member of the fold's, so none of them accepts it either.
                    // Where tsc says so is decided once every argument is typed:
                    // at the last candidate's first rejected argument.
                    if arity_candidates > 1 {
                        // This argument's shape is already recorded.
                        overload_failure_argument = Some(argument_types.len().saturating_sub(1));
                    } else {
                        let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
                            &argument_type,
                            &parameter_type,
                            &argument_type_name,
                            &parameter_type_name,
                            true,
                            ctx.file_name.clone(),
                        );
                        ctx.push(diagnostic_with_syntax_span(diagnostic, argument.span));
                    }
                    mismatch_reported = true;
                }
            }
            InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. } => {
                has_unresolved_argument = true;
            }
            InferredExpression::Unknown => {}
        }
    }

    if let Some(rest) = gathered_rest.as_ref()
        && let Some(types) = gathered_types
        && !mismatch_reported
        && !has_unresolved_argument
        && !gathered_rest_is_unmodelled(&rest.target, 0)
    {
        let gathered = Type::Tuple(types);
        if !is_assignable_to(&gathered, &rest.target) {
            let reported_target = crate::checks::expr::reported_relation_target(&gathered, &rest.target);
            let gathered_name = source_display_name(&gathered, &reported_target);
            // No gathered argument reports on the call, one on itself, and
            // several on the run from the first to the last.
            let span = match arguments.len() - rest.start {
                0 => call_span.or(callee_span),
                1 => arguments[rest.start].span,
                _ => excess_argument_span(arguments, rest.start),
            };
            ctx.push(diagnostic_with_syntax_span(
                crate::checks::expr::assignability_mismatch_diagnostic(
                    &gathered,
                    &rest.target,
                    &gathered_name,
                    &reported_target.name(),
                    true,
                    ctx.file_name.clone(),
                ),
                span,
            ));
            mismatch_reported = true;
        }
    }

    // `getSpreadArgumentType`: one spread argument filling a non-array rest
    // is its mutable array or tuple, related to the rest type whole.
    if has_spread_argument
        && !mismatch_reported
        && function_type.overloads().is_none()
        && let Some(rest) = gathered_rest_parameter(function_type)
        && arguments.len() == rest.start + 1
        && arguments[rest.start].spread
        && !gathered_rest_is_unmodelled(&rest.target, 0)
    {
        let diagnostics_before = ctx.diagnostics().len();
        let spread = crate::infer::infer_expression(&arguments[rest.start].expression, symbols, ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
        if let InferredExpression::Known(spread) = spread
            && let Some(gathered) = spread_argument_array(&spread)
            && !is_assignable_to(&gathered, &rest.target)
        {
            let reported_target = crate::checks::expr::reported_relation_target(&gathered, &rest.target);
            let gathered_name = source_display_name(&gathered, &reported_target);
            ctx.push(diagnostic_with_syntax_span(
                crate::checks::expr::assignability_mismatch_diagnostic(
                    &gathered,
                    &rest.target,
                    &gathered_name,
                    &reported_target.name(),
                    true,
                    ctx.file_name.clone(),
                ),
                arguments[rest.start].span,
            ));
            mismatch_reported = true;
        }
    }

    // The fold accepts what no member does when its positions disagree
    // (`set(key: string, …)` / `set(key: number, …)` fold `key` to the
    // sentinel). tsc's `chooseOverload` finds no applicable candidate then, so
    // the call is TS2769 all the same — decided only where every candidate
    // rejects a typed argument outright.
    // Explicit type arguments filter the candidates first (a constraint they
    // violate drops the overload), which this arity-only list does not model.
    if overload_failure_argument.is_none()
        && !mismatch_reported
        && type_arguments.is_empty()
        && fitting_candidates.len() > 1
        && fitting_candidates
            .iter()
            .all(|candidate| candidate_definitely_rejects(candidate, &argument_types).is_some())
    {
        overload_failure_argument = fitting_candidates
            .last()
            .and_then(|last| candidate_definitely_rejects(last, &argument_types));
    }
    let overload_failed = overload_failure_argument.is_some();
    if let Some(fold_failure) = overload_failure_argument {
        // With explicit type arguments the candidate list tsc re-checks is not
        // this arity-only one, so the anchor stays where the fold rejected.
        let last = fitting_candidates.last().filter(|_| type_arguments.is_empty());
        let rejected = last.and_then(|last| last_candidate_rejected_argument(last, &argument_types));
        let anchor = rejected.unwrap_or(fold_failure);
        if let Some(argument) = arguments.get(anchor).or_else(|| arguments.get(fold_failure)) {
            let parameter_type = rejected.zip(last).and_then(|(index, last)| effective_parameter_type(last, index));
            push_overload_failure(argument, parameter_type.as_ref(), symbols, ctx);
        }
        mismatch_reported = true;
    }

    if has_unresolved_argument {
        return None;
    }

    // Each argument fits the fold — the union of what the candidates take at its
    // position — but a call is resolved against one candidate at a time, and
    // `pair(1, 2)` fits neither `(string, number)` nor `(number, string)`. When
    // every candidate of fitting arity provably rejects some argument, tsc
    // reports TS2769 on the argument the *last* candidate rejects. A generic
    // member of an interface or class group still carries its own type
    // parameters, which `chooseOverload` infers for the call before it relates
    // the arguments.
    let mut instantiated_overloads: Vec<Option<FunctionType>> = Vec::new();
    if !mismatch_reported
        && !has_spread_argument
        && arity_candidates > 1
        && let Some(members) = function_type.overloads()
    {
        instantiated_overloads = vec![None; members.len()];
        let mut last_rejected = None;
        let mut every_candidate_rejects = true;
        for (index, member) in members.iter().enumerate() {
            if !overload_arity_fits(member, arguments.len()) {
                continue;
            }
            // A function declaration's generic member is inferred for its
            // return only: surge's inference (a `const` parameter, a readonly
            // tuple) is not exact enough to reject an argument by.
            let declared_function_member = member
                .declaration()
                .is_some_and(|declaration| declaration.is::<crate::symbols::FunctionSignatureInfo>());
            if type_arguments.is_empty() && !own_type_parameter_names(member).is_empty() && !declared_function_member {
                instantiated_overloads[index] = Some(instantiate_overload_member(
                    member,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                ));
            }
            let candidate = instantiated_overloads[index].as_ref().unwrap_or(member);
            let literal_shapes = literal_shapes_for_candidate(candidate, &argument_types, arguments, symbols, ctx);
            match first_rejected_argument(candidate, literal_shapes.as_deref().unwrap_or(&argument_types)) {
                Some(rejected) => last_rejected = Some((rejected, effective_parameter_type(candidate, rejected))),
                None => {
                    every_candidate_rejects = false;
                    break;
                }
            }
        }
        if every_candidate_rejects && let Some((index, parameter_type)) = last_rejected {
            push_overload_failure(&arguments[index], parameter_type.as_ref(), symbols, ctx);
            return overload_failure_return_type(
                function_type,
                type_arguments,
                callee_span,
                arguments,
                expected_return_type,
                symbols,
                ctx,
            );
        }
    }

    // The arguments were checked once, against the permissive fold. With their
    // types in hand, the return type is the first overload's that accepts them
    // — tsc's resolution order — and the fold's when none does, which keeps a
    // no-match call exactly where it was.
    let return_type = if overload_failed {
        overload_failure_return_type(
            function_type,
            type_arguments,
            callee_span,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        )
    } else if mismatch_reported || has_spread_argument {
        None
    } else {
        check_callbacks_against_chosen_overload(function_type, &argument_types, arguments, symbols, ctx);
        choose_overload_return_type(
            function_type,
            &argument_types,
            type_arguments,
            callee_span,
            arguments,
            expected_return_type,
            instantiated_overloads,
            symbols,
            ctx,
        )
    };
    Some(with_type_copy_reason(
        TypeCopyReason::CallResolution,
        || return_type.unwrap_or_else(|| function_type.return_type().clone()),
    ))
}

/// Overload selection for a call that is only *inferred* — a callback body
/// sketched to bind a caller's type parameter. tsc resolves every call through
/// `chooseOverload`, whatever asks for its type, so `plain(() => z.string())`
/// binds `ZodString` from `string()`'s first overload; the sketch read the
/// group's permissive fold instead and bound nothing. Shapes are built exactly
/// as `check_function_type_call` builds them, from inferred rather than checked
/// arguments, and the probe's diagnostics are discarded.
///
/// `function_type` is the group as declared: instantiating a generic group for
/// the call rebuilds one kept signature and drops the rest, while `chooseOverload`
/// walks every candidate in declaration order. A pick that is itself generic is
/// declined by `select_overload_return_type`, leaving the caller's instantiation.
pub(crate) fn select_overload_return_type_for_inferred_call(
    function_type: &FunctionType,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let argument_types = inferred_argument_shapes(function_type, arguments, symbols, ctx)?;
    match select_overload_member_for_argument_shapes(function_type, &argument_types, arguments, symbols, ctx) {
        Some(member) => Some(member.return_type().clone()),
        None => select_overload_return_type(function_type, &argument_types),
    }
}

/// The overload member [`select_overload_return_type_for_inferred_call`] picks,
/// instantiated for the call: what `resolveCall` hands a `new` expression
/// over an overloaded generic construct signature (`SetConstructor`).
pub(crate) fn select_overload_member_for_inferred_call(
    function_type: &FunctionType,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    let argument_types = inferred_argument_shapes(function_type, arguments, symbols, ctx)?;
    select_overload_member_for_argument_shapes(function_type, &argument_types, arguments, symbols, ctx)
}

fn select_overload_member_for_argument_shapes(
    function_type: &FunctionType,
    argument_types: &[ArgumentShape],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    // The candidates in declaration order, a generic one instantiated for the
    // call first, as `chooseOverload` does (`literal<const T>("a")` past a
    // candidate taking a readonly array) and as the checked call picks.
    let diagnostics_before = ctx.diagnostics.len();
    let mut selected = None;
    for candidate in function_type.overloads()? {
        if !overload_arity_fits(candidate, arguments.len()) {
            continue;
        }
        let instantiated = if own_type_parameter_names(candidate).is_empty() {
            candidate.clone()
        } else {
            instantiate_overload_member(candidate, &[], None, arguments, None, symbols, ctx)
        };
        let literal_shapes = literal_shapes_for_candidate(&instantiated, argument_types, arguments, symbols, ctx);
        if signature_accepts_argument_types(
            &instantiated,
            literal_shapes.as_deref().unwrap_or(argument_types),
            OverloadRelation::Assignable,
        ) && !names_open_parameter(instantiated.return_type())
            && !names_open_parameter(&instantiated.return_type().peeled())
        {
            selected = Some(instantiated);
            break;
        }
    }
    ctx.diagnostics.truncate(diagnostics_before);
    selected
}

fn inferred_argument_shapes(
    function_type: &FunctionType,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Vec<ArgumentShape>> {
    function_type.overloads()?;
    if arguments.iter().any(|argument| argument.spread) {
        return None;
    }
    let diagnostics_before = ctx.diagnostics.len();
    let argument_types: Vec<ArgumentShape> = arguments
        .iter()
        .map(|argument| ArgumentShape {
            ty: match crate::infer::infer_expression(&argument.expression, symbols, ctx) {
                InferredExpression::Known(argument_type)
                    if !argument_is_callback(&argument.expression)
                        && !type_contains_unknown(&argument_type) =>
                {
                    Some(argument_type)
                }
                _ => None,
            },
            written_keys: written_object_keys(&argument.expression),
            element_keys: written_element_keys(&argument.expression),
            callback: argument_is_callback(&argument.expression),
        })
        .collect();
    ctx.diagnostics.truncate(diagnostics_before);
    Some(argument_types)
}

/// What overload selection knows about one argument.
#[derive(Clone)]
struct ArgumentShape {
    /// The evaluated type, or `None` for a wildcard position.
    ty: Option<Type>,
    /// The property names an object-literal argument writes, spread-free. Known
    /// from the syntax alone, so it survives a degraded member type: an
    /// overload requiring a property the literal never writes is rejected even
    /// when the literal's type had to be a wildcard.
    written_keys: Option<Vec<String>>,
    /// The keys each element of an array-literal argument writes, when every
    /// element is a spread-free object literal.
    element_keys: Option<Vec<Vec<String>>>,
    /// An arrow function, whose type is left to the parameter but which no
    /// parameter a function cannot satisfy takes.
    callback: bool,
}

impl ArgumentShape {
    fn wildcard() -> Self {
        Self {
            ty: None,
            written_keys: None,
            element_keys: None,
            callback: false,
        }
    }
}

/// Whether a function value could be assignable to a parameter of this type.
/// Only shapes a function provably misses answer no: a primitive, an array or
/// tuple, and an object without signatures that requires a string index (a
/// function has none, nor an inferable one) or a member the `Function` surface
/// does not carry.
fn callback_may_satisfy(parameter_type: &Type) -> bool {
    const FUNCTION_MEMBERS: &[&str] = &[
        "name", "length", "prototype", "call", "apply", "bind", "toString", "arguments", "caller",
        "constructor", "valueOf", "toLocaleString", "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable",
    ];
    match parameter_type.peeled() {
        Type::Union(union) => union.types().iter().any(callback_may_satisfy),
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::Never
        | Type::Array(_)
        | Type::Tuple(_)
        | Type::OpenTuple(_) => false,
        Type::Object(object) => {
            object.call_signature().is_some()
                || object.construct_signature().is_some()
                || object.synthetic_open_index
                || !object.string_index_type.as_deref().is_some_and(|index| !matches!(index, Type::Any))
                    && object
                        .required_properties()
                        .all(|(name, _)| FUNCTION_MEMBERS.contains(&name.as_ref()) || name.starts_with('['))
        }
        _ => true,
    }
}

fn written_object_keys(expression: &ParsedExpression) -> Option<Vec<String>> {
    let ParsedExpression::ObjectLiteral { properties, .. } = expression else {
        return None;
    };
    if properties.iter().any(|property| property.is_spread) {
        return None;
    }
    Some(
        properties
            .iter()
            .map(|property| property.name.clone())
            .collect(),
    )
}

fn written_element_keys(expression: &ParsedExpression) -> Option<Vec<Vec<String>>> {
    let ParsedExpression::ArrayLiteral { elements, .. } = expression else {
        return None;
    };
    elements
        .iter()
        .map(|element| if element.spread { None } else { written_object_keys(&element.expression) })
        .collect()
}

/// The element type of an array parameter, an optional one's included.
fn array_parameter_element(parameter_type: &Type) -> Option<Type> {
    match parameter_type.peeled() {
        Type::Array(element) => Some(*element),
        Type::Union(union) => {
            let mut arrays = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void));
            let element = array_parameter_element(arrays.next()?)?;
            arrays.next().is_none().then_some(element)
        }
        _ => match parameter_type {
            Type::Reference(reference) if reference.is_readonly_array() => reference.arguments.first().cloned(),
            _ => None,
        },
    }
}

/// Whether an object-literal argument writing exactly `keys` can satisfy a
/// parameter of this type: every required property of the object it must land
/// in is written. A union is satisfied by any member; a type this cannot see
/// into is not judged.
fn written_keys_satisfy(parameter_type: &Type, keys: &[String]) -> bool {
    match parameter_type.peeled() {
        Type::Object(object) => object
            .required_properties()
            .all(|(name, _)| keys.iter().any(|key| key.as_str() == name.as_ref())),
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| written_keys_satisfy(member, keys)),
        _ => true,
    }
}

/// An argument whose type is *given* by the parameter it lands in: it was
/// contextually typed by the fold, so it cannot tell the overloads apart.
fn argument_is_callback(expression: &ParsedExpression) -> bool {
    matches!(expression, ParsedExpression::ArrowFunction(_))
}

thread_local! {
    static CHECKING_OVERLOAD_CALLBACKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// What re-checking a call's callbacks reported, by file and first
    /// callback span (`None`: the overload rejected them). A call is
    /// re-evaluated, often in a throwaway context, whenever its enclosing body
    /// is; its callbacks are checked once per file and the result replayed.
    static OVERLOAD_CALLBACK_MEMO: std::cell::RefCell<
        surge_ts_types::fx::FxHashMap<(String, usize, usize), Option<Vec<surge_ts_diagnostics::Diagnostic>>>,
    > = std::cell::RefCell::new(surge_ts_types::fx::FxHashMap::default());
}

pub(crate) fn clear_overload_callback_memo() {
    OVERLOAD_CALLBACK_MEMO.with(|memo| memo.borrow_mut().clear());
}

/// `chooseOverload` contextually types each callback argument by the
/// candidate it is testing, and the chosen candidate's final check reports
/// what the callback body does wrong. The arguments were checked against the
/// fold, which types no callback parameter; this re-checks the callbacks
/// against the first non-generic candidate the other arguments fit, when it
/// accepts the callbacks too, keeping only diagnostics the fold's pass did not
/// already report.
fn check_callbacks_against_chosen_overload(
    function_type: &FunctionType,
    argument_types: &[ArgumentShape],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let Some(overloads) = function_type.overloads() else {
        return;
    };
    // Only a callback with an unannotated parameter is typed by its context.
    let contextually_typed = |expression: &ParsedExpression| {
        matches!(expression, ParsedExpression::ArrowFunction(function)
            if function.parameters.iter().any(|parameter| parameter.declared_type.is_none()))
    };
    if !arguments.iter().any(|argument| contextually_typed(&argument.expression)) {
        return;
    }
    // A generic candidate is tried as `chooseOverload` tries it: instantiated
    // for the call (`on<K extends keyof Events>(type: K, listener: (ev:
    // Events[K]) => any)` types the listener from `type`).
    let instantiated: Vec<FunctionType>;
    let overloads: &[FunctionType] = if overloads
        .iter()
        .any(|member| member.type_parameter_head().is_some() || !own_type_parameter_names(member).is_empty())
    {
        if CHECKING_OVERLOAD_CALLBACKS.with(std::cell::Cell::get) {
            return;
        }
        // The instantiation sketches the callbacks; what that reports is the
        // re-check's below, or nobody's.
        let checkpoint = ctx.diagnostics().len();
        CHECKING_OVERLOAD_CALLBACKS.with(|flag| flag.set(true));
        instantiated = overloads
            .iter()
            .map(|member| {
                if own_type_parameter_names(member).is_empty() || !overload_arity_fits(member, arguments.len()) {
                    member.clone()
                } else {
                    instantiate_overload_member(member, &[], None, arguments, None, symbols, ctx)
                }
            })
            .collect();
        CHECKING_OVERLOAD_CALLBACKS.with(|flag| flag.set(false));
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        &instantiated
    } else {
        overloads
    };
    let callback_parameter = |candidate: &FunctionType, index: usize| -> Option<Type> {
        if candidate.is_variadic() && index + 1 >= candidate.parameters().len() {
            return None;
        }
        let parameter = candidate.parameters().get(index)?;
        (matches!(parameter.peeled(), Type::Function(_))
            && !type_contains_unknown(parameter)
            && !surge_ts_types::parameter_type_is_degraded(parameter))
        .then(|| parameter.clone())
    };
    // The first candidate to type a callback fixes its parameters
    // (`assignContextualParameterTypes`), so a later one never re-types it:
    // only a call that candidate accepts is checked here.
    // A callback slot typed by a predicate signature takes only a callback
    // that is one (`refine<R>(check: (arg) => arg is R)` is passed over for
    // `(value) => Promise.resolve(true)`).
    let predicate_slot_rejects = |candidate: &FunctionType| {
        arguments.iter().enumerate().any(|(index, argument)| {
            let ParsedExpression::ArrowFunction(arrow) = &argument.expression else {
                return false;
            };
            let slot_is_predicate = candidate.parameters().get(index).is_some_and(|parameter| {
                matches!(parameter.peeled(), Type::Function(slot) if slot.type_predicate().is_some())
            });
            slot_is_predicate && !matches!(arrow.return_type, Some(surge_ts_syntax::ParsedType::Predicate(_)))
        })
    };
    let Some(candidate) = overload_relations(overloads).find_map(|relation| {
        overloads.iter().find(|candidate| {
            overload_arity_fits(candidate, arguments.len())
                && signature_accepts_argument_types(candidate, argument_types, relation)
                && !predicate_slot_rejects(candidate)
        })
    }) else {
        return;
    };
    let mut callbacks = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if !argument_is_callback(&argument.expression) {
            continue;
        }
        let Some(parameter) = callback_parameter(candidate, index) else {
            return;
        };
        // The fold already typed a callback whose slot holds this one
        // signature, alone or beside non-callable members (`replace`'s
        // `string | (substring: string, ...args: any[]) => string`).
        let fold_agrees = function_type.parameters().get(index).is_some_and(|folded| match folded.peeled() {
            Type::Union(union) => {
                let mut callables = union.types().iter().filter(|member| matches!(member.peeled(), Type::Function(_)));
                callables.next().is_some_and(|only| only.peeled() == parameter.peeled()) && callables.next().is_none()
            }
            peeled => peeled == parameter.peeled(),
        });
        if contextually_typed(&argument.expression) && !fold_agrees {
            callbacks.push((index, parameter));
        }
    }
    if callbacks.is_empty() {
        return;
    }
    // A callback re-checked here holds calls of its own; re-checking those
    // too would re-evaluate every enclosing callback once per nesting level.
    if CHECKING_OVERLOAD_CALLBACKS.with(std::cell::Cell::get) {
        return;
    }
    let key = arguments[callbacks[0].0].span.map(|span| (ctx.file_name.clone(), span.start, span.end));
    if let Some(recorded) = key.as_ref().and_then(|key| OVERLOAD_CALLBACK_MEMO.with(|memo| memo.borrow().get(key).cloned())) {
        if let Some(recorded) = recorded {
            crate::program::extend_diagnostics_dedup(&mut ctx.diagnostics, recorded);
        }
        return;
    }
    CHECKING_OVERLOAD_CALLBACKS.with(|flag| flag.set(true));
    let checkpoint = ctx.diagnostics().len();
    let accepts = callbacks.iter().all(|(index, parameter)| {
        let argument = &arguments[*index];
        match evaluate_expression_with_expected_type(
            &argument.expression,
            argument.span,
            Some(parameter),
            ExpectedTypeDiagnostic::ContextOnly,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(callback) => {
                type_contains_unknown(&callback) || is_assignable_to(&callback, parameter)
            }
            _ => false,
        }
    });
    CHECKING_OVERLOAD_CALLBACKS.with(|flag| flag.set(false));
    if !accepts {
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        if let Some(key) = key {
            OVERLOAD_CALLBACK_MEMO.with(|memo| memo.borrow_mut().insert(key, None));
        }
        return;
    }
    let fresh = ctx.diagnostics.split_off(checkpoint);
    if let Some(key) = key {
        OVERLOAD_CALLBACK_MEMO.with(|memo| memo.borrow_mut().insert(key, Some(fresh.clone())));
    }
    crate::program::extend_diagnostics_dedup(&mut ctx.diagnostics, fresh);
}

/// The return type of the first overload the evaluated arguments satisfy, or
/// `None` when the signature carries no group or no member accepts them.
fn select_overload_return_type(
    function_type: &FunctionType,
    argument_types: &[ArgumentShape],
) -> Option<Type> {
    let overloads = function_type.overloads()?;
    crate::program::record_overload_selection_attempt();
    let picked = overload_relations(overloads).find_map(|relation| {
        overloads
            .iter()
            .find(|candidate| signature_accepts_argument_types(candidate, argument_types, relation))
    })?;
    // A member whose return still names a type parameter, or stands at the
    // sentinel, was folded before instantiation and never re-resolved for this
    // call — a generic group reached without its written signature. Its return
    // is the declaration's, not the call's; the fold's answer (which widened
    // such disagreements to `any`) is the honest one. zod's
    // `registry.get(schema)?.id` came back as an unbound `$replace<Meta, S>`
    // without this.
    if names_open_parameter(picked.return_type()) {
        return None;
    }
    // A generic member resolved outside a call has its type parameters at
    // their defaults (`querySelector<E extends Element = Element>` answers
    // `Element`), where tsc would infer them — from the arguments or from the
    // contextual return type (`const r: SVGRectElement = q.querySelector(…)!`).
    // Only the call's own instantiation can answer for it.
    if picked.type_parameter_head().is_some() {
        return None;
    }
    crate::program::record_overload_selection_pick();
    Some(picked.return_type().clone())
}

/// tsc's `chooseOverload` over the evaluated arguments: the candidates in
/// declaration order, a generic one instantiated for this call from its own
/// written signature before it is tested, and the first that accepts the
/// arguments answers the call. `None` when none does or the answer still names
/// an open parameter, which leaves the fold's return.
///
/// A candidate with no written signature to instantiate keeps its open
/// parameters, which reject every argument exactly as they did before this
/// walk: a free function's group is instantiated per call before it is attached,
/// so only an interface method group reaches the instantiation.
///
/// `instantiated` holds the candidates the caller already instantiated for this
/// call (by index); one is used only where this walk would instantiate it.
#[allow(clippy::too_many_arguments)]
fn choose_overload_return_type(
    function_type: &FunctionType,
    argument_types: &[ArgumentShape],
    type_arguments: &[ParsedType],
    callee_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    mut instantiated: Vec<Option<FunctionType>>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let overloads = function_type.overloads()?;
    crate::program::record_overload_selection_attempt();
    instantiated.resize(overloads.len(), None);
    for relation in overload_relations(overloads) {
        for (index, candidate) in overloads.iter().enumerate() {
            // Written type arguments replace what a member was resolved with,
            // its type parameters' defaults included.
            let open = !type_arguments.is_empty() || names_open_parameter(&Type::Function(candidate.clone()));
            // A member resolved with its type parameters' defaults is still
            // generic to the call: the contextual return infers them
            // (`querySelector<E = Element>(s): E | null` under `Rect | null`).
            let generic_under_context = expected_return_type.is_some() && !own_type_parameter_names(candidate).is_empty();
            if (open || generic_under_context) && instantiated[index].is_none() {
                instantiated[index] = Some(instantiate_overload_member(
                    candidate,
                    type_arguments,
                    callee_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                ));
            }
            let candidate = match &instantiated[index] {
                Some(own) if open => own,
                Some(own) if generic_under_context && !names_open_parameter(&Type::Function(own.clone())) => own,
                _ => candidate,
            };
            let literal_shapes = literal_shapes_for_candidate(candidate, argument_types, arguments, symbols, ctx);
            if !signature_accepts_argument_types(candidate, literal_shapes.as_deref().unwrap_or(argument_types), relation) {
                continue;
            }
            if names_open_parameter(candidate.return_type()) {
                return None;
            }
            crate::program::record_overload_selection_pick();
            return Some(candidate.return_type().clone());
        }
    }
    None
}

/// The argument shapes with each array literal retyped against a tuple
/// parameter of `candidate`, as `chooseOverload` checks it in that
/// candidate's tuple context (`checkArrayLiteral`'s `inTupleContext`):
/// `[1, 2]` is the tuple `[1, 2]` against a `const` tuple parameter and
/// `number[]` on its own. An `xs.map(…)` argument is retyped too, its
/// callback's array literals tuples against tuple elements (`new Map(xs.map(x
/// => [x.id, x]))`). `None` when no argument meets such a context.
fn literal_shapes_for_candidate(
    candidate: &FunctionType,
    argument_types: &[ArgumentShape],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Vec<ArgumentShape>> {
    let parameters = candidate.parameters();
    let mut shapes: Option<Vec<ArgumentShape>> = None;
    for (index, (argument, shape)) in arguments.iter().zip(argument_types).enumerate() {
        let mapped = matches!(&argument.expression,
            ParsedExpression::PropertyCall { property_name, .. } if property_name == "map");
        if argument.spread || !(mapped || matches!(argument.expression, ParsedExpression::ArrayLiteral { .. })) {
            continue;
        }
        let parameter_type = if candidate.is_variadic() && !parameters.is_empty() && index + 1 >= parameters.len() {
            rest_parameter_element_type(&parameters[parameters.len() - 1], index + 1 - parameters.len())
        } else {
            match parameters.get(index) {
                Some(parameter) => parameter.clone(),
                None => continue,
            }
        };
        let diagnostics_before = ctx.diagnostics().len();
        let retyped = if mapped {
            crate::infer::expression::with_mapped_argument_context(parameter_type, || {
                crate::infer::infer_expression(&argument.expression, symbols, ctx)
            })
        } else if let Some(context) = tuple_literal_context(&parameter_type) {
            crate::infer::expression::with_literal_element_context(context, || {
                crate::infer::infer_expression(&argument.expression, symbols, ctx)
            })
        } else {
            // A sequence of tuples (`entries: readonly (readonly [K, V])[] | null`)
            // types each of the literal's array literals as a tuple.
            let Some(context) = sequence_element_type(&parameter_type)
                .as_ref()
                .and_then(tuple_literal_context)
            else {
                continue;
            };
            let ParsedExpression::ArrayLiteral { elements, .. } = &argument.expression else {
                continue;
            };
            if elements.iter().any(|element| element.spread) {
                continue;
            }
            let members: Option<Vec<Type>> = elements
                .iter()
                .map(|element| {
                    crate::infer::expression::with_literal_element_context(context, || {
                        crate::infer::infer_expression(&element.expression, symbols, ctx)
                    })
                    .flowing_type()
                })
                .collect();
            match members {
                Some(members) if !members.is_empty() => {
                    InferredExpression::Known(Type::Array(Box::new(surge_ts_types::union_type(members))))
                }
                _ => continue,
            }
        }
        .flowing_type();
        ctx.truncate_diagnostics(diagnostics_before);
        let Some(retyped) = retyped.filter(|retyped| !type_contains_unknown(retyped)) else {
            continue;
        };
        let shapes = shapes.get_or_insert_with(|| argument_types.to_vec());
        shapes[index] = ArgumentShape { ty: Some(retyped), ..shape.clone() };
    }
    shapes
}

/// The element type of an array, readonly array or iterable parameter, read
/// through a union's `null` and `undefined`.
fn sequence_element_type(parameter_type: &Type) -> Option<Type> {
    match parameter_type.peeled() {
        Type::Array(element) => Some(*element),
        Type::Union(union) => {
            let mut sequences = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Null | Type::Undefined))
                .filter_map(sequence_element_type);
            match (sequences.next(), sequences.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        }
        Type::Reference(reference) if reference.is_readonly_array() => sequence_element_type(&reference.resolve()),
        Type::Reference(reference) => {
            let base = reference.display.split('<').next().unwrap_or_default();
            match (base, &*reference.arguments) {
                ("Array" | "ReadonlyArray" | "Iterable" | "IterableIterator" | "ArrayLike", [element]) => {
                    Some(element.clone())
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The tuple context a parameter gives an array literal: present when a member
/// is a tuple, keeping the literal kinds its elements name
/// (`isLiteralOfContextualType`).
fn tuple_literal_context(parameter_type: &Type) -> Option<crate::infer::expression::LiteralElementContext> {
    let members = match parameter_type.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    let mut context = crate::infer::expression::LiteralElementContext::default();
    let mut mark = |element: &Type, context: &mut crate::infer::expression::LiteralElementContext| {
        let members = match element {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other.clone()],
        };
        for member in members {
            match member {
                Type::StringLiteral(_) => context.string = true,
                Type::NumberLiteral(_) => context.number = true,
                _ => {}
            }
        }
    };
    for member in &members {
        if let Some((elements, _)) = surge_ts_types::fixed_tuple_parts(member) {
            context.tuple = true;
            for element in elements {
                mark(element, &mut context);
            }
        } else if let Type::OpenTuple(open) = member.peeled() {
            context.tuple = true;
            for element in open.leading.iter().chain(std::iter::once(open.rest.as_ref())).chain(&open.trailing) {
                mark(element, &mut context);
            }
        }
    }
    context.tuple.then_some(context)
}

/// tsc's `getCandidateForOverloadFailure` when a candidate is generic
/// (`pickLongestCandidateSignature`): the first candidate taking every
/// argument, or else the one taking the most, its own type parameters inferred
/// from the arguments. `None` for a group without a generic member, which tsc
/// answers with the union of its signatures.
fn overload_failure_return_type(
    function_type: &FunctionType,
    type_arguments: &[ParsedType],
    callee_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let members = function_type.overloads().unwrap_or(std::slice::from_ref(function_type));
    if members.iter().all(|member| own_type_parameter_names(member).is_empty()) {
        return None;
    }
    let mut longest: Option<usize> = None;
    for (index, member) in members.iter().enumerate() {
        if member.is_variadic() || member.parameters().len() >= arguments.len() {
            longest = Some(index);
            break;
        }
        if longest.is_none_or(|best| member.parameters().len() > members[best].parameters().len()) {
            longest = Some(index);
        }
    }
    let candidate = &members[longest?];
    if own_type_parameter_names(candidate).is_empty() {
        return Some(candidate.return_type().clone());
    }
    let instantiated = instantiate_overload_member(
        candidate,
        type_arguments,
        callee_span,
        arguments,
        expected_return_type,
        symbols,
        ctx,
    );
    Some(instantiated.return_type().clone())
}

/// `candidate` with its own type parameters inferred for this call, as
/// `chooseOverload` instantiates each generic candidate before it relates the
/// arguments. The probe's diagnostics are discarded.
fn instantiate_overload_member(
    candidate: &FunctionType,
    type_arguments: &[ParsedType],
    callee_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> FunctionType {
    let diagnostics_before = ctx.diagnostics.len();
    let instantiated = instantiate::while_choosing_overload(|| {
        property::instantiate_declared_member_signature(
            candidate,
            None,
            type_arguments,
            callee_span,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        )
        .into_owned()
    });
    ctx.diagnostics.truncate(diagnostics_before);
    instantiated
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OverloadRelation {
    Subtype,
    Assignable,
}

/// tsc's `resolveCall`: with more than one candidate, the arguments are first
/// held to the subtype relation, so an overload the arguments only reach by
/// assignability (`any` to `number`) loses to a later one they are a subtype
/// of; only when none is do they fall back to assignability. A group with a
/// generic member keeps to assignability alone: tsc rejects a candidate whose
/// inferred type arguments break their constraints, which surge's instantiation
/// does not check, so a generic member would win the subtype pass it loses.
fn overload_relations(candidates: &[FunctionType]) -> impl Iterator<Item = OverloadRelation> {
    let subtype = (candidates.len() > 1 && candidates.iter().all(|candidate| candidate.type_parameter_head().is_none()))
        .then_some(OverloadRelation::Subtype);
    subtype.into_iter().chain(std::iter::once(OverloadRelation::Assignable))
}

/// `type_contains_unknown` without the written `unknown`: an overload taking
/// `(pattern: unknown)` or returning `(value: unknown) => value is T` is fully
/// instantiated, and treating the keyword as the sentinel declined every such
/// member, handing the call the fold's `any` instead of the predicate.
fn names_open_parameter(ty: &Type) -> bool {
    names_open_parameter_in(ty, false)
}

/// `in_member` is set below an object's members, where a resolved generic
/// method (`then<TResult1>` on a `Promise`) keeps its own type parameters only
/// as the sentinel: that says nothing about the call being left open.
fn names_open_parameter_in(ty: &Type, in_member: bool) -> bool {
    let open = |ty: &Type| names_open_parameter_in(ty, in_member);
    match ty {
        // A member surge could not model is a gap in it, not a type
        // parameter the call left open.
        Type::Unknown => !in_member,
        Type::TypeParameter(_) => true,
        Type::GenuineUnknown => false,
        Type::Array(element) => open(element),
        Type::Reference(reference) if reference.is_readonly_array() => {
            reference.arguments.iter().any(open)
        }
        Type::Tuple(elements) => elements.iter().any(open),
        Type::Function(function) if in_member && !function.type_parameter_names().is_empty() => false,
        Type::Function(function) => {
            function.parameters().iter().any(open) || open(function.return_type())
        }
        Type::Object(object) => {
            let member = |ty: &Type| names_open_parameter_in(ty, true);
            object.properties.values().any(|property| member(&property.ty))
                || object.string_index_type.as_deref().is_some_and(member)
        }
        Type::Union(union) => union.types().iter().any(open),
        _ => false,
    }
}

/// The first argument `signature` provably rejects. Only a fully modelled
/// parameter and a fully known argument count: a callback, a degraded type or
/// an open type parameter on either side proves nothing, and the candidate is
/// then not known to reject the call at all.
fn first_rejected_argument(
    signature: &FunctionType,
    argument_types: &[ArgumentShape],
) -> Option<usize> {
    let parameters = signature.parameters();
    let expected = parameters.len();
    argument_types.iter().enumerate().position(|(i, argument)| {
        let is_rest_position = signature.is_variadic() && expected > 0 && i >= expected - 1;
        let parameter_type = if is_rest_position {
            rest_parameter_element_type(&parameters[expected - 1], i - (expected - 1))
        } else if i < expected {
            let declared = parameters[i].clone();
            if i >= signature.required_parameter_count() {
                union_type(vec![declared, Type::Undefined])
            } else {
                declared
            }
        } else {
            return false;
        };
        let Some(argument_type) = argument.ty.as_ref() else {
            return false;
        };
        let read_parameter = parameter_read_by_argument(&parameter_type, argument_type);
        !matches!(parameter_type, Type::Never)
            && !names_open_parameter(&parameter_type)
            && !parameter_holds_hole(&read_parameter)
            && !surge_ts_types::parameter_type_is_degraded(&read_parameter)
            && !is_open_instantiation(argument_type)
            && (argument
                .written_keys
                .as_deref()
                .is_some_and(|keys| excess_written_key(&parameter_type, keys).is_some())
                || argument.element_keys.as_deref().is_some_and(|elements| {
                    array_parameter_element(&parameter_type).is_some_and(|element| {
                        elements.iter().any(|keys| excess_written_key(&element, keys).is_some())
                    })
                })
                || !is_assignable_to(argument_type, &parameter_type))
    })
}

/// tsc's excess property check of a fresh object literal argument
/// (`hasExcessProperties`): the first key no object member of the parameter
/// declares. `None` when a member is open, indexed or not an object type,
/// which may know any key, or is the global `Object` or an empty object type,
/// which tsc never excess-checks; an array or tuple knows none of them.
fn excess_written_key<'a>(parameter_type: &Type, keys: &'a [String]) -> Option<&'a str> {
    let members = match parameter_type.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    let mut objects = Vec::new();
    for member in members {
        match member.peeled() {
            Type::Undefined | Type::Null | Type::Void => {}
            Type::Array(_) | Type::Tuple(_) => objects.push(None),
            Type::Object(object) if object.properties.is_empty() || object.alias_name.as_deref() == Some("Object") => {
                return None;
            }
            Type::Object(object)
                if !object.synthetic_open_index
                    && object.string_index_type.is_none()
                    && object.number_index_type.is_none()
                    && object.keyed_indexes.is_none()
                    && object.intersection_operands.is_none()
                    && object.call_signature().is_none()
                    && object.construct_signature().is_none() =>
            {
                objects.push(Some(object));
            }
            _ => return None,
        }
    }
    if objects.is_empty() {
        return None;
    }
    keys.iter().map(String::as_str).find(|key| {
        objects
            .iter()
            .all(|object| object.as_ref().is_none_or(|object| !object.properties.contains_key(*key)))
    })
}

/// tsc's `reportCallResolutionErrors`: each error the last candidate reports
/// against the argument it rejects, under "No overload matches this call" —
/// an array literal's every rejected element, an object literal's rejected
/// properties, else the argument itself. Re-checking the argument against that
/// candidate's parameter finds those anchors; whatever else the check reports
/// was already reported against the fold.
fn push_overload_failure(
    argument: &ParsedCallArgument,
    parameter_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let mut spans = Vec::new();
    let best_match = parameter_type.map(|parameter_type| best_indexed_member(&argument.expression, parameter_type));
    if let Some(parameter_type) = best_match.as_ref()
        && matches!(
            argument.expression,
            ParsedExpression::ArrayLiteral { .. } | ParsedExpression::ObjectLiteral { .. }
        )
        && !parameter_holds_hole(parameter_type)
        && !surge_ts_types::type_carries_hole_near_surface(parameter_type)
    {
        let before = ctx.diagnostics().len();
        let outer_suppressed = ctx.suppressed_argument_mismatch_span.take();
        let _ = evaluate_expression_with_expected_type(
            &argument.expression,
            argument.span,
            Some(parameter_type),
            ExpectedTypeDiagnostic::ArgumentNotAssignable,
            symbols,
            ctx,
        );
        ctx.suppressed_argument_mismatch_span = outer_suppressed;
        for diagnostic in &ctx.diagnostics()[before..] {
            if is_relation_diagnostic(diagnostic)
                && let Some(span) = diagnostic.span
                && !spans.contains(&span)
            {
                spans.push(span);
            }
        }
        ctx.truncate_diagnostics_releasing_utility_keys(before);
    }
    if spans.is_empty()
        && let Some(span) = overload_failure_span(argument, parameter_type).or(argument.expression_span)
    {
        spans.push(crate::context::convert_span(span));
    }
    for span in spans {
        ctx.push(Diagnostic::ts2769(ctx.file_name.clone()).with_span(span));
    }
}

fn is_relation_diagnostic(diagnostic: &Diagnostic) -> bool {
    const RELATION_CODES: [u32; 10] = [2322, 2345, 2353, 2375, 2379, 2559, 2561, 2739, 2740, 2741];
    matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(code) if RELATION_CODES.contains(&code))
}

/// An argument checked against an overloaded call's fold that no candidate
/// accepts reports what the last candidate's elaboration does, each under
/// "No overload matches this call": the fold's elements, properties and
/// callback returns are the candidates' own. Says whether it retagged any.
fn retag_overload_mismatches(from: usize, ctx: &mut CheckerContext) -> bool {
    if !ctx.diagnostics()[from..].iter().any(is_relation_diagnostic) {
        return false;
    }
    let tail = ctx.diagnostics()[from..].to_vec();
    ctx.truncate_diagnostics(from);
    for diagnostic in tail {
        match diagnostic.span {
            Some(span) if is_relation_diagnostic(&diagnostic) => {
                ctx.push(Diagnostic::ts2769(ctx.file_name.clone()).with_span(span));
            }
            _ => ctx.push(diagnostic),
        }
    }
    true
}

/// `getBestMatchIndexedAccessTypeOrUndefined`: an array literal elaborated
/// against a union is related element by element to the one member it can
/// index (`ArrayLike<bigint> | ArrayBuffer`).
fn best_indexed_member(expression: &ParsedExpression, parameter_type: &Type) -> Type {
    let Type::Union(union) = parameter_type.peeled() else {
        return parameter_type.clone();
    };
    if !matches!(expression, ParsedExpression::ArrayLiteral { .. }) {
        return parameter_type.clone();
    }
    let indexed = |member: &Type| match member.peeled() {
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Object(object) => object.number_index_type.is_some(),
        _ => false,
    };
    let mut members = union
        .types()
        .iter()
        .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void) && indexed(member));
    match (members.next(), members.next()) {
        (Some(only), None) => only.clone(),
        _ => parameter_type.clone(),
    }
}

/// Where tsc reports an overload call no candidate accepts: on the excess
/// property of an object literal the last candidate rejects for one, else on
/// the argument.
fn overload_failure_span(argument: &ParsedCallArgument, parameter_type: Option<&Type>) -> Option<SyntaxTextSpan> {
    if let (ParsedExpression::ObjectLiteral { properties, .. }, Some(parameter_type)) =
        (&argument.expression, parameter_type)
    {
        let keys: Vec<String> = properties
            .iter()
            .filter(|property| !property.is_spread)
            .map(|property| property.name.clone())
            .collect();
        if let Some(excess) = excess_written_key(parameter_type, &keys)
            && let Some(span) = properties
                .iter()
                .find(|property| !property.is_spread && property.name == excess)
                .and_then(|property| property.name_span)
        {
            return Some(span);
        }
    }
    if let (ParsedExpression::ArrayLiteral { elements, .. }, Some(element_type)) =
        (&argument.expression, parameter_type.and_then(array_parameter_element))
    {
        for element in elements.iter().filter(|element| !element.spread) {
            if let ParsedExpression::ObjectLiteral { properties, .. } = &element.expression
                && let Some(keys) = written_object_keys(&element.expression)
                && let Some(excess) = excess_written_key(&element_type, &keys)
                && let Some(span) = properties
                    .iter()
                    .find(|property| property.name == excess)
                    .and_then(|property| property.name_span)
            {
                return Some(span);
            }
        }
    }
    argument.span
}

fn signature_accepts_argument_types(
    signature: &FunctionType,
    argument_types: &[ArgumentShape],
    relation: OverloadRelation,
) -> bool {
    let parameters = signature.parameters();
    let expected = parameters.len();
    let actual = argument_types.len();
    let mut required = signature.required_parameter_count();
    while required > 0 && parameter_is_void_optional(&parameters[required - 1]) {
        required -= 1;
    }
    if actual < required || (!signature.is_variadic() && actual > expected) {
        return false;
    }

    argument_types.iter().enumerate().all(|(i, argument)| {
        let is_rest_position = signature.is_variadic() && expected > 0 && i >= expected - 1;
        let parameter_type = if is_rest_position {
            rest_parameter_element_type(&parameters[expected - 1], i - (expected - 1))
        } else if i < expected {
            let declared = parameters[i].clone();
            if i >= signature.required_parameter_count() {
                union_type(vec![declared, Type::Undefined])
            } else {
                declared
            }
        } else {
            return true;
        };
        if let Some(keys) = argument.written_keys.as_deref()
            && !written_keys_satisfy(&parameter_type, keys)
        {
            return false;
        }
        if argument.callback && !callback_may_satisfy(&parameter_type) {
            return false;
        }
        let Some(argument_type) = argument.ty.as_ref() else {
            return true;
        };
        // A parameter standing at the degradation sentinel proves nothing about
        // this position; committing to such an overload would hand its (equally
        // degraded) return to every consumer.
        let related = match (relation, argument.written_keys.as_deref()) {
            (OverloadRelation::Subtype, Some(keys)) => {
                let shape = surge_ts_types::LiteralShape::Object(
                    keys.iter()
                        .map(|key| (std::sync::Arc::from(key.as_str()), surge_ts_types::LiteralShape::Regular))
                        .collect(),
                );
                surge_ts_types::is_subtype_of_expression(argument_type, &shape, &parameter_type)
            }
            (OverloadRelation::Subtype, None) => surge_ts_types::is_subtype_of(argument_type, &parameter_type),
            (OverloadRelation::Assignable, _) => is_assignable_to(argument_type, &parameter_type),
        };
        !names_open_parameter(&parameter_type) && related && !weak_type_rejects(argument_type, &parameter_type)
    })
}

/// tsc's weak-type rule, applied to selection only: an object type whose
/// properties are all optional accepts nothing that shares no property with it.
/// surge's assignability is lenient there — `'utf8'` passes against
/// `{ encoding?: null; flag?: string } | null` — and a lenient match commits to
/// the wrong overload: `fs.readFileSync(path, 'utf8')` picked the `Buffer`
/// overload on both zod and trpc. Rejects when every parameter member that
/// accepted the argument is a weak object the argument shares nothing with.
fn weak_type_rejects(argument_type: &Type, parameter_type: &Type) -> bool {
    let parameter_type = parameter_type.peeled();
    let members: Vec<Type> = match &parameter_type {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let mut accepting = 0usize;
    let mut weakly_rejected = 0usize;
    for member in &members {
        if !is_assignable_to(argument_type, member) {
            continue;
        }
        accepting += 1;
        if let Type::Object(object) = member.peeled()
            && is_weak_object(&object)
            && !shares_a_property(argument_type, &object)
        {
            weakly_rejected += 1;
        }
    }
    accepting > 0 && accepting == weakly_rejected
}

pub(crate) fn is_weak_object(object: &surge_ts_types::ObjectType) -> bool {
    !(object.non_primitive && !object.is_intersection)
        && object
            .intersection_operands
            .as_deref()
            .filter(|_| object.is_intersection)
            .is_none_or(|operands| operands.iter().all(surge_ts_types::is_weak_type))
        && !object.properties.is_empty()
        && object.required_properties().next().is_none()
        && !object.declares_string_index_access()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
}

/// Whether `argument_type` is an object carrying at least one of `target`'s
/// property names, or an empty object (which tsc lets through). Primitives,
/// arrays and functions share nothing.
pub(crate) fn shares_a_property(argument_type: &Type, target: &surge_ts_types::ObjectType) -> bool {
    match argument_type.peeled() {
        Type::Any => true,
        Type::Object(object) => {
            object.properties.is_empty()
                || object
                    .properties
                    .keys()
                    .any(|name| target.contains_property(name))
        }
        _ => false,
    }
}

/// Whether a construct signature's resolved return type is the instance tsc
/// would produce. surge does not instantiate a generic construct signature's
/// own type parameters yet, so a result still naming one (`Promise<T>`), or
/// holding the sentinel a constrained one is erased to (`WeakRef<T>` for
/// `T extends WeakKey`), is not.
fn is_resolved_construct_instance(ty: &Type) -> bool {
    fn names_signature_type_parameter(ty: &Type) -> bool {
        match ty {
            Type::Unknown => true,
            Type::TypeParameter(parameter) => !parameter.is_active_variable(),
            Type::Reference(reference) => {
                reference.arguments.iter().any(names_signature_type_parameter)
            }
            Type::Union(union) => union.types().iter().any(names_signature_type_parameter),
            Type::Array(element) => names_signature_type_parameter(element),
            Type::Tuple(elements) => elements.iter().any(names_signature_type_parameter),
            _ => false,
        }
    }
    !ty.is_degraded() && !names_signature_type_parameter(ty)
}

/// When `expected` is a nominal reference to the interface `name` with `arity`
/// type arguments (e.g. the contextual `Promise<void>` for an un-annotated
/// `new Promise(...)`), returns that reference (the instance type) together with
/// its resolved type arguments.
fn contextual_instance_reference(
    name: &str,
    arity: usize,
    expected: &Type,
) -> Option<(Type, Vec<Type>)> {
    let Type::Reference(reference) = expected else {
        return None;
    };
    let display = reference.display.as_ref();
    let base = display.split('<').next().unwrap_or(display);
    if base != name || reference.arguments.len() != arity {
        return None;
    }
    Some((expected.clone(), reference.arguments.to_vec()))
}

/// Re-resolves a generic construct signature with `type_arguments` substituted
/// for its per-signature type parameters, by re-reading the declaring
/// constructor interface's parsed construct signature (`PromiseConstructor`'s
/// `new <T>(executor): Promise<T>`). Returns `None` when the interface has no
/// matching generic construct signature, so callers fall back to the
/// (un-substituted) resolved signature.
fn substituted_construct_signature(
    constructor_type: &Type,
    base_signature: &FunctionType,
    type_arguments: &[Type],
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    let constructor_name = constructor_type.name();
    let parsed = match ctx.lookup_type_declaration(&constructor_name) {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => info
            .body
            .construct_signatures
            .iter()
            .find(|signature| {
                !signature.type_parameters.is_empty()
                    && signature.type_parameters.len() == type_arguments.len()
                    && signature.parameters.len() == base_signature.parameters().len()
            })
            .cloned(),
        _ => None,
    }?;

    let signature_info = crate::symbols::FunctionSignatureInfo {
        overloaded: false,
        type_parameters: parsed.type_parameters.clone(),
        parameter_types: parsed
            .parameters
            .iter()
            .map(|parameter| Some(parameter.ty.clone()))
            .collect(),
        parameter_names: parsed
            .parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        rest: parsed
            .parameters
            .last()
            .is_some_and(|parameter| parameter.rest),
        return_type: Some((*parsed.return_type).clone()),
        declaring_file: None,
        namespace_prefix: None,
        predicate_overload: None,
        overload_alternatives: Vec::new(),
        inferred_predicate: None,
        body_return: None,
        construct_signatures: None,
    };
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for (type_parameter, argument) in parsed.type_parameters.iter().zip(type_arguments.iter()) {
        substitution.insert(type_parameter.name.clone(), argument.clone());
    }

    Some(
        instantiate_function_type_with_substitution(
            base_signature,
            &signature_info,
            &substitution,
            ctx,
        )
        .into_owned(),
    )
}

/// Whether a parameter of this type may be omitted at a call site. tsc treats a
/// parameter typed `void` — or a union with a `void` member (e.g. a `Promise<void>`
/// executor's `resolve: (value: void | PromiseLike<void>) => void`) — as
/// optional, but not `undefined`.
fn parameter_is_void_optional(ty: &Type) -> bool {
    match ty {
        Type::Void => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| matches!(member, Type::Void)),
        _ => false,
    }
}

/// A type argument surge could not resolve, as opposed to one that merely
/// mentions a type parameter. A *generic* function value
/// (`vi.fn<typeof fetchData>(fetchData)`) carries its own parameters as
/// placeholders; refusing it as a candidate left the call uninstantiated and
/// every use of the result degraded.
pub(crate) fn type_argument_is_unresolved(ty: &Type) -> bool {
    match ty {
        Type::Unknown => true,
        Type::Array(element) => type_argument_is_unresolved(element),
        Type::Tuple(elements) => elements.iter().any(type_argument_is_unresolved),
        // A *member's* signature carrying the sentinel does not disqualify the
        // object that holds it. `class SQL implements SQLWrapper` and
        // `SQLWrapper.getSQL(): SQL` are mutually recursive, so surge's cycle
        // break leaves `getSQL`'s return as the sentinel — and every drizzle
        // class that implements the interface was then refused as a type
        // argument, which is what abandoned `is(dialect, PgDialect)`. Binding
        // `T` to such a shape still types every use of `T` correctly except that
        // one member, which was already unmodelled.
        Type::Function(_) => false,
        // Nor does a *property* carrying it, for the same reason and with the
        // same reach: `router({ a: p1, b: p2 })` hands `TIn` an object whose
        // members are procedures, and one member's unmodelled `input` vetoed
        // the whole record — so the router lost every key and the client built
        // from it lost every route. Go has no such veto at all
        // (`inferFromTypes` records the source it is given); surge keeps it
        // only where the sentinel *is* the candidate, or is the element every
        // use of the parameter would read.
        Type::Object(_) => false,
        Type::Union(union) => union.types().iter().any(type_argument_is_unresolved),
        _ => false,
    }
}

/// `Mock<T>` with `T` bare, `FetchQueryOptions<TQueryFnData>` inside a generic
/// body: an instantiation over an open argument is not settled enough to reject
/// on either side of a check. Only the type itself is asked — a generic
/// function *value* whose parameter names its own `T` is still a concrete
/// value (`const n: number = Controller` is a real mismatch).
/// A mismatch no argument of an open instantiation can repair: an object
/// against a primitive, or a target's required member the source's shape
/// lacks entirely (TS2741 does not depend on the member types).
pub(crate) fn definitely_mismatches(source: &Type, target: &Type) -> bool {
    let Type::Object(source_object) = source.peeled() else {
        return false;
    };
    if source_object.is_intersection
        || source_object.string_index_type.is_some()
        || source_object.number_index_type.is_some()
        || source_object.synthetic_open_index
        || source.is_unmodelled()
    {
        return false;
    }
    match target.peeled() {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Object(target_object) => {
            !target_object.is_intersection
                && target_object.call_signature().is_none()
                && target_object.construct_signature().is_none()
                && !target.is_unmodelled()
                && target_object.properties.iter().any(|(name, property)| {
                    !property.optional && !name.starts_with('[') && !source_object.contains_property(name)
                })
        }
        _ => false,
    }
}

pub(crate) fn is_open_instantiation(ty: &Type) -> bool {
    open_instantiation(ty, true)
}

/// [`is_open_instantiation`] for a value read from a binding written with its
/// type (`declare const p: Parent<unknown>`): an `unknown` argument there is
/// the keyword, not an inference left open.
pub(crate) fn is_open_instantiation_past_unknown_keyword(ty: &Type) -> bool {
    open_instantiation(ty, false)
}

fn open_instantiation(ty: &Type, unknown_keyword_is_open: bool) -> bool {
    let argument_is_open = |ty: &Type| open_argument(ty, unknown_keyword_is_open);
    fn open_argument(ty: &Type, unknown_keyword_is_open: bool) -> bool {
        match ty {
            // A type variable of the body being checked is a type there
            // (`F<CP>` inside `wu<CP>`), not an inference left open.
            Type::TypeParameter(_) => !ty.is_type_variable(),
            Type::Unknown => true,
            Type::GenuineUnknown => unknown_keyword_is_open,
            Type::Reference(reference) => {
                reference.arguments.iter().any(|argument| open_argument(argument, unknown_keyword_is_open))
            }
            _ => false,
        }
    }
    let Type::Reference(reference) = ty else {
        return false;
    };
    // The lib promise is related through its one argument, covariantly, so a
    // type variable of the body being checked settles it: `Promise<T>` inside
    // `f<T>` is as checkable as the `T` it resolves to.
    if reference.arguments.len() == 1
        && matches!(reference.display.split('<').next(), Some("Promise" | "PromiseLike"))
        && promise_nominal_enabled()
    {
        return reference
            .arguments
            .iter()
            .any(|argument| !argument.is_type_variable() && argument_is_open(argument));
    }
    // A generator's `TNext` is `unknown` by tsc's own default
    // (`getReturnTypeFromBody`), not an argument an inference left open.
    if reference.arguments.len() == 3
        && matches!(reference.display.split('<').next(), Some("Generator" | "AsyncGenerator"))
    {
        return reference.arguments[..2].iter().any(argument_is_open);
    }
    reference.arguments.iter().any(argument_is_open)
}

/// Whether `function` declares type parameters of its own. Such a signature is
/// fully modelled whatever its parameters say: they are written over its own
/// placeholders (`static deserialize: <T>(s: string) => T`, or superjson's
/// `registerCustom: <I, O>(t: Omit<Transformer<I, O>, 'name'>) => void`, whose
/// `Omit` over placeholders resolves to the sentinel), so a "contains unknown"
/// walk does not read them as a value surge failed to model. Treating them so
/// silenced every assignment, argument and return of `SuperJSON`.
///
/// Only a *source* is read this way. A target carrying such signatures (ts-pattern's
/// `Matcher` inside `P.Pattern<Input>`) is still where surge models least, and
/// checking against it surfaced its gaps as false positives.
pub(crate) fn is_generic_signature(function: &FunctionType) -> bool {
    GENERIC_SIGNATURES_ARE_MODELLED.with(std::cell::Cell::get)
        && !own_type_parameter_names(function).is_empty()
}

thread_local! {
    static GENERIC_SIGNATURES_ARE_MODELLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs a "contains unknown" query over a value being checked, the side
/// [`is_generic_signature`] applies to.
pub(crate) fn as_source<R>(query: impl FnOnce() -> R) -> R {
    let previous = GENERIC_SIGNATURES_ARE_MODELLED.with(|flag| flag.replace(true));
    let result = query();
    GENERIC_SIGNATURES_ARE_MODELLED.with(|flag| flag.set(previous));
    result
}

pub(crate) fn own_type_parameter_names(function: &FunctionType) -> Vec<String> {
    // A signature read from a type annotation (`static parse: <T>(s: string) =>
    // T`) carries no declaration, only its rendered parameter list.
    let Some(declaration) = function.declaration() else {
        return function.type_parameter_names();
    };
    let type_parameters =
        if let Some(member) = declaration.downcast_ref::<DeclaredMemberSignature>() {
            &member.signature.type_parameters
        } else if let Some(signature) =
            declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>()
        {
            &signature.type_parameters
        } else {
            return function.type_parameter_names();
        };
    type_parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect()
}

/// [`type_contains_unknown`] for a value's own shape: a written `unknown` in it
/// (a method's `(o: unknown)` parameter) is a real type, and only surge's
/// sentinel or a free type parameter says the shape is incomplete.
/// [`type_contains_unknown`] for a parameter an argument is related to: an
/// `unknown` the parameter's object, index or element types hold directly is
/// the keyword (`{ [key: string]: unknown }`, an uninferred `T`), while one
/// inside a signature still reads as a shape surge may not have finished.
fn parameter_holds_hole(ty: &Type) -> bool {
    match ty {
        Type::GenuineUnknown => false,
        Type::Object(object) if object.call_signature().is_none() && object.construct_signature().is_none() => {
            object.properties.values().any(|property| parameter_holds_hole(&property.ty))
                || object.string_index_type.as_deref().is_some_and(parameter_holds_hole)
                || object.number_index_type.as_deref().is_some_and(parameter_holds_hole)
        }
        // A constructor is read by its members and what it constructs
        // (`new () => { prop: T }` with `T` uninferred); its parameters, as
        // any object's signatures, are not looked into.
        Type::Object(object) if object.call_signature().is_none() => {
            let Some(construct) = object.construct_signature() else {
                return type_contains_unknown(ty);
            };
            let members = match construct.overloads().filter(|members| members.len() > 1) {
                Some(members) => members.to_vec(),
                None => vec![construct.clone()],
            };
            members.iter().any(|signature| parameter_holds_hole(signature.return_type()))
                || object.properties.values().any(|property| parameter_holds_hole(&property.ty))
                || object.string_index_type.as_deref().is_some_and(parameter_holds_hole)
        }
        Type::Union(union) => union.types().iter().any(parameter_holds_hole),
        Type::Array(element) => parameter_holds_hole(element),
        Type::Tuple(elements) => elements.iter().any(parameter_holds_hole),
        other => type_contains_unknown(other),
    }
}

fn type_contains_degradation(ty: &Type) -> bool {
    UNKNOWN_KEYWORD_IS_REAL.with(|flag| {
        let previous = flag.replace(true);
        let result = type_contains_unknown(ty);
        flag.set(previous);
        result
    })
}

thread_local! {
    static UNKNOWN_KEYWORD_IS_REAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn type_contains_unknown(ty: &Type) -> bool {
    match ty {
        Type::GenuineUnknown => !UNKNOWN_KEYWORD_IS_REAL.with(std::cell::Cell::get),
        Type::Unknown => true,
        Type::TypeParameter(parameter) => {
            !crate::checks::assign::is_bound_type_parameter(parameter)
        }
        Type::Array(element) => type_contains_unknown(element),
        Type::Reference(reference) if reference.is_readonly_array() => {
            reference.arguments.iter().any(type_contains_unknown)
        }
        Type::Tuple(elements) => elements.iter().any(type_contains_unknown),
        // An overload group is as modelled as its members, a generic one with
        // its own type parameters; their fold mixes those as holes.
        Type::Function(function) if function.overloads().is_some_and(|members| members.len() > 1) => function
            .overloads()
            .into_iter()
            .flatten()
            .any(|member| type_contains_unknown(&Type::Function(member.clone()))),
        Type::Function(function) => {
            !is_generic_signature(function)
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

/// relater.go relates a source to a type-parameter target only when the
/// source is that parameter, one constrained to it, `any` or `never`, so an
/// argument that mentions no type variable is decided against a variable of
/// the body being checked (`cb(null)` for `cb: (t: T) => U` inside the
/// generic), though `parameter_type_is_degraded` reads every type parameter as
/// a hole. So is an argument that is itself such a variable: `T` passed for
/// an `S extends T` is not `S`.
fn type_variable_decides(argument_type: &Type, parameter_type: &Type) -> bool {
    parameter_type.is_type_variable()
        && (argument_type.is_type_variable() || !surge_ts_types::type_variable::mentions_type_variable(argument_type))
        && !as_source(|| type_contains_degradation(argument_type))
}

/// The part of a function parameter an argument function is compared on: a
/// parameter the argument does not declare is never compared (a source may
/// take fewer), so a hole there decides nothing — the uninferred `T` of a
/// receiver in `success: (value: T) => U`, which `() => IPromise<number>`
/// never reads.
fn parameter_read_by_argument(parameter_type: &Type, argument_type: &Type) -> Type {
    let Type::Function(argument) = argument_type.peeled() else {
        return parameter_type.clone();
    };
    if argument.is_variadic() || argument.overloads().is_some() {
        return parameter_type.clone();
    }
    let read = |member: &Type| match member.peeled() {
        Type::Function(parameter)
            if parameter.overloads().is_none() && argument.parameters().len() < parameter.parameters().len() =>
        {
            let read: Vec<Type> = parameter.parameters()[..argument.parameters().len()].to_vec();
            let required = parameter.required_parameter_count().min(read.len());
            Type::Function(FunctionType::new(read, parameter.return_type().clone(), false, required))
        }
        _ => member.clone(),
    };
    // An optional callback parameter is the function or `undefined`.
    match parameter_type {
        Type::Union(union) => union_type(union.types().iter().map(read).collect()),
        other => read(other),
    }
}

/// relater.go `compareSignaturesRelated` against a callback whose rest is a
/// type variable of the body being checked (`cb: (x: number, ...args: T) =>
/// void` inside `f<T extends any[]>`): the parameters an argument function
/// takes past the callback's fixed ones form a tuple the variable must fit,
/// which its constraint decides (`(a, b, ...x) => {}` needs a `T` of at least
/// one element), though `parameter_type_is_degraded` reads it as a hole.
fn generic_rest_callback_decides(argument_type: &Type, parameter_type: &Type) -> bool {
    let (Type::Function(argument), Type::Function(parameter)) = (argument_type.peeled(), parameter_type.peeled())
    else {
        return false;
    };
    let Some((rest, fixed)) = parameter.parameters().split_last() else {
        return false;
    };
    parameter.is_variadic()
        && rest.is_type_variable()
        && argument.required_parameter_count() > fixed.len()
        && !fixed.iter().any(surge_ts_types::parameter_type_is_degraded)
        && !as_source(|| type_contains_degradation(argument_type))
}

fn is_unnarrowable_literal(expression: &ParsedExpression) -> bool {
    matches!(
        expression,
        ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
            | ParsedExpression::TemplateLiteral { .. }
    )
}

/// tsc's `resolveCallExpression` for a callee with no call signature: one that
/// can only be constructed is TS2348, which names it and suggests `new`.
pub(crate) fn not_callable_diagnostic(callee: &Type, ctx: &CheckerContext) -> Diagnostic {
    not_callable_member_diagnostic(callee, false, ctx)
}

/// [`not_callable_diagnostic`] for a call through a member read:
/// `invocationErrorDetails` heads the report with TS6234 when the call passes
/// no arguments and the member is a `get` accessor.
pub(crate) fn not_callable_member_diagnostic(
    callee: &Type,
    get_accessor_call: bool,
    ctx: &CheckerContext,
) -> Diagnostic {
    match callee {
        Type::Object(object) if object.construct_signature().is_some() && object.call_signature().is_none() => {
            Diagnostic::ts2348(callee.name(), ctx.file_name.clone())
        }
        _ if get_accessor_call => Diagnostic::ts6234(ctx.file_name.clone()),
        _ => Diagnostic::ts2349(ctx.file_name.clone()),
    }
}

fn declared_signature_info(function: &FunctionType) -> Option<std::sync::Arc<crate::symbols::FunctionSignatureInfo>> {
    let declaration = function.declaration()?;
    if let Some(member) = declaration.downcast_ref::<DeclaredMemberSignature>() {
        return Some(member.signature.clone());
    }
    declaration
        .downcast_ref::<crate::symbols::FunctionSignatureInfo>()
        .map(|signature| std::sync::Arc::new(signature.clone()))
}
