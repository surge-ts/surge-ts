//! Generic call type-argument inference and function-type instantiation.

use super::*;

use std::borrow::Cow;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedCallArgument, ParsedFunctionType, ParsedNamedType, ParsedObjectType, ParsedType, TextSpan,
};
use surge_ts_types::{FunctionType, Type, TypeCopyReason, with_type_copy_reason};

use crate::context::{CheckerContext, convert_span};
use crate::infer::string_literal_union_keys;
use crate::infer::{InferredExpression, infer_expression};
use crate::infer::types::InferenceCandidate;
use crate::infer::{
    TypeParameterSubstitution, map_parsed_type_with_substitution,
    try_map_parsed_type_with_substitution,
};
use crate::metrics::{alloc_function_type, alloc_object_type};
use crate::program::{
    record_generic_call_inference_attempt, record_generic_call_inference_candidate,
    record_generic_call_inference_explicit_type_args_skip, record_generic_call_inference_failed,
    record_generic_call_inference_success,
    record_generic_call_inference_unresolved_argument_skip,
};
use crate::symbols::{FunctionSignatureInfo, SymbolTable, TypeDeclarationInfo};

pub(crate) fn instantiate_function_type<'a>(
    function_type: &'a FunctionType,
    function_signature: Option<&FunctionSignatureInfo>,
    outer_type_arguments: &[(String, Type)],
    type_arguments: &[ParsedType],
    type_argument_span: Option<TextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Cow<'a, FunctionType> {
    // A signature the call site knows no generic declaration for (a
    // parameter's or a variable's annotation) may carry its own, which
    // explicit type arguments instantiate. Inference through it needs the
    // call's contextual return: without it, it binds too little.
    let own = (!type_arguments.is_empty() || expected_return_type.is_some())
        .then(|| own_generic_declaration(function_type))
        .flatten();
    let (function_signature, outer_type_arguments) = match (function_signature, own) {
        (Some(signature), _) if !signature.type_parameters.is_empty() => (Some(signature), outer_type_arguments),
        (_, Some((signature, outer))) => (Some(signature), outer),
        (signature, None) => (signature, outer_type_arguments),
    };
    let Some(function_signature) = function_signature else {
        return instantiate_written_type_parameters(function_type, type_arguments, symbols, ctx)
            .map_or(Cow::Borrowed(function_type), Cow::Owned);
    };

    if function_signature.type_parameters.is_empty() {
        return instantiate_written_type_parameters(function_type, type_arguments, symbols, ctx)
            .map_or(Cow::Borrowed(function_type), Cow::Owned);
    }

    record_generic_call_inference_attempt();

    if !type_arguments.is_empty() {
        record_generic_call_inference_explicit_type_args_skip();
        // Explicit type arguments name types visible at the CALL, which may be
        // function locals; `ctx.symbols` is the file-level table and does not
        // hold them. The caller already passed the right one.
        let substitution = {
            let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
            let substitution =
                explicit_type_argument_substitution(function_signature, type_arguments, ctx);
            ctx.symbols = saved_symbols;
            substitution
        };

        // The enclosing bindings (`T` of the interface a `<K extends keyof T>`
        // member was read off) are seeded first: the constraint checks below
        // resolve `keyof T` against them.
        let mut substitution = substitution;
        seed_outer_type_arguments(&mut substitution, outer_type_arguments);
        // A type argument that failed to resolve, or one that violates a
        // `K extends keyof T` constraint, must not cascade into the `T[K]`
        // return type; fall back to the declared (generic) return type instead.
        let has_unresolved_argument = substitution
            .iter()
            .filter(|(name, _)| {
                !outer_type_arguments
                    .iter()
                    .any(|(outer, _)| outer == name.as_ref())
            })
            .any(|(_, candidate)| type_argument_is_unresolved(candidate));
        let constraint_violation = enforce_explicit_keyof_constraints(
            function_signature,
            type_arguments,
            &substitution,
            type_argument_span,
            ctx,
        );
        if has_unresolved_argument || constraint_violation {
            return Cow::Borrowed(function_type);
        }

        let instantiated = instantiate_function_type_with_substitution(
            function_type,
            function_signature,
            &substitution,
            ctx,
        );
        return fold_overload_alternative_parameters(
            instantiated,
            function_type,
            function_signature,
            &substitution,
            outer_type_arguments,
            type_arguments,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        );
    }

    let (mut substitution, argument_types) = infer_type_arguments_reading_arguments(
        function_signature,
        arguments,
        outer_type_arguments,
        expected_return_type,
        symbols,
        ctx,
    );
    let unknown_inferred = UNKNOWN_INFERRED_FROM_ARGUMENTS.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
    if std::env::var("SURGE_DBG_INFER").is_ok() {
        eprintln!(
            "DBG instantiate tps={:?} args={} subst={:?}",
            function_signature
                .type_parameters
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>(),
            arguments.len(),
            substitution
                .iter()
                .map(|(n, t)| (n.to_string(), t.name()))
                .collect::<Vec<_>>()
        );
    }
    if function_signature.return_type.is_none()
        && function_signature.body_return.is_some()
        && let Some(expected) = expected_return_type
    {
        infer_from_expected_to_body_return(function_type, function_signature, expected, &mut substitution, ctx);
    }
    seed_outer_type_arguments(&mut substitution, outer_type_arguments);
    if enforce_inferred_constraints(function_signature, &mut substitution, ctx) {
        NEVER_FROM_BROKEN_CONSTRAINT.set(arguments.as_ptr() as usize);
    }
    let defaults_completed_binding = apply_uninferred_type_parameter_defaults(
        function_signature,
        arguments,
        &argument_types,
        expected_return_type,
        &mut substitution,
        ctx,
    );

    if std::env::var("SURGE_DBG_INFER").is_ok() {
        eprintln!(
            "DBG after-enforce subst={:?}",
            substitution
                .iter()
                .map(|(n, t)| (n.to_string(), t.name()))
                .collect::<Vec<_>>()
        );
    }
    let inferred_nothing = !defaults_completed_binding
        && substitution
            .iter()
            .filter(|(name, _)| {
                !outer_type_arguments
                    .iter()
                    .any(|(outer, _)| outer == name.as_ref())
            })
            // A parameter fixed at `unknown` for want of a candidate
            // (`getInferredType`) is inferred, not a hole — unless the return
            // type names it, where tsc's return-type inference (which surge
            // follows only partly) may still have bound it.
            .all(|(name, candidate)| {
                candidate.is_degraded()
                    && !candidate.is_type_variable()
                    && !(matches!(candidate, Type::GenuineUnknown)
                        && !substitution.is_placeholder(name)
                        && (unknown_inferred.contains(name)
                            || function_signature
                                .return_type
                                .as_ref()
                                .is_some_and(|return_type| !parsed_type_mentions_name(return_type, name))))
            });
    if inferred_nothing {
        record_generic_call_inference_failed();
        if is_declaration_backed_lazy_signature(function_type) || !outer_type_arguments.is_empty() {
            return instantiate_function_type_with_substitution(
                function_type,
                function_signature,
                &substitution,
                ctx,
            );
        }
        return Cow::Borrowed(function_type);
    }

    record_generic_call_inference_success();
    let instantiated = instantiate_function_type_with_substitution(
        function_type,
        function_signature,
        &substitution,
        ctx,
    );
    if std::env::var("SURGE_DBG_INFER").is_ok() {
        {
            let r = instantiated.return_type();
            let args = if let Type::Reference(rf) = r {
                rf.arguments.iter().map(|a| a.name()).collect::<Vec<_>>()
            } else {
                vec![]
            };
            eprintln!(
                "DBG instantiated ret={} variant={} args={:?} peeled={}",
                r.name(),
                format!("{r:?}").chars().take(60).collect::<String>(),
                args,
                r.peeled().name().chars().take(200).collect::<String>()
            );
        }
    }
    fold_overload_alternative_parameters(
        instantiated,
        function_type,
        function_signature,
        &substitution,
        outer_type_arguments,
        type_arguments,
        arguments,
        expected_return_type,
        symbols,
        ctx,
    )
}

/// Restores an overload group's parameter fold after instantiation.
///
/// The group's value type already holds the union of every overload's parameter
/// at a position, but a generic call rebuilds each parameter from the *kept*
/// signature's written annotation, which leaves only the first overload's shape
/// behind: an argument written for a later overload
/// (`useQuery({ queryKey, queryFn })`, whose first overload demands
/// `initialData`) was then reported against the first. Each later overload is
/// instantiated against the same call and its parameter unioned back in.
///
/// Return type, arity and variadic flag are untouched — which overload's return
/// applies needs real overload resolution, and widening it here would degrade
/// every generic group's result.
fn fold_overload_alternative_parameters<'a>(
    instantiated: Cow<'a, FunctionType>,
    function_type: &FunctionType,
    function_signature: &FunctionSignatureInfo,
    kept_substitution: &TypeParameterSubstitution,
    outer_type_arguments: &[(String, Type)],
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Cow<'a, FunctionType> {
    if function_signature.overload_alternatives.is_empty() {
        return instantiated;
    }
    // Each member is its own declaration's signature (tsc resolves the call
    // against those), so it keeps that declaration's arity rather than the
    // group fold's. The group lists its members in the same order.
    let group_members = function_type.overloads();
    let member_base = |index: usize, signature: &FunctionSignatureInfo| {
        group_members
            .and_then(|members| members.get(index))
            .filter(|member| member.parameters().len() == signature.parameter_types.len())
            .cloned()
    };

    let mut parameters = instantiated.parameters().to_vec();
    let mut folded = false;
    // The kept signature first, then each alternative in declaration order:
    // the same list a non-generic group carries, instantiated for this call.
    let mut members = Vec::with_capacity(1 + function_signature.overload_alternatives.len());
    let kept_member = match member_base(0, function_signature) {
        Some(base) => instantiate_function_type_with_substitution(&base, function_signature, kept_substitution, ctx)
            .into_owned(),
        None => instantiated.clone().into_owned(),
    };
    // An alternative's own annotations are resolved here only to read their
    // shape; a constraint violation or an unresolved name in an overload this
    // call did not pick is not the call's error.
    let diagnostics_before = ctx.diagnostics.len();
    // `chooseOverload` passes over the kept signature too when the written
    // type arguments break its constraints; a lone survivor then reports its
    // own argument error rather than TS2769.
    let kept_breaks_constraints = !type_arguments.is_empty()
        && explicit_type_arguments_break_constraints(function_signature, kept_substitution, ctx);
    if !kept_breaks_constraints {
        members.push(kept_member.clone());
    }

    for (alternative_index, alternative) in function_signature.overload_alternatives.iter().enumerate() {
        let mut substitution = if type_arguments.is_empty() {
            // Each candidate is inferred under the call's contextual return,
            // as tsc's `chooseOverload` infers every one.
            let (mut substitution, argument_types) = infer_type_arguments_reading_arguments(
                alternative,
                arguments,
                &[],
                expected_return_type,
                symbols,
                ctx,
            );
            // `getInferredType` puts the constraint in place of a candidate
            // that breaks it for every overload tried, not only the kept one.
            seed_outer_type_arguments(&mut substitution, outer_type_arguments);
            enforce_inferred_constraints(alternative, &mut substitution, ctx);
            apply_uninferred_type_parameter_defaults(
                alternative,
                arguments,
                &argument_types,
                None,
                &mut substitution,
                ctx,
            );
            substitution
        } else {
            let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
            let substitution =
                explicit_type_argument_substitution(alternative, type_arguments, ctx);
            ctx.symbols = saved_symbols;
            // `chooseOverload` passes over a candidate whose type parameters
            // the written type arguments do not satisfy (`checkTypeArguments`).
            if explicit_type_arguments_break_constraints(alternative, &substitution, ctx) {
                continue;
            }
            substitution
        };
        seed_outer_type_arguments(&mut substitution, outer_type_arguments);

        let alternative_type = instantiate_function_type_with_substitution(
            function_type,
            alternative,
            &substitution,
            ctx,
        );

        members.push(match member_base(alternative_index + 1, alternative) {
            Some(base) => instantiate_function_type_with_substitution(&base, alternative, &substitution, ctx)
                .into_owned(),
            None => alternative_type.clone().into_owned(),
        });

        for (index, parameter) in parameters.iter_mut().enumerate() {
            let Some(candidate) = alternative_type.parameters().get(index) else {
                continue;
            };
            // A candidate standing at the degradation sentinel says nothing about
            // what this position accepts, and folding it in would make the
            // position permissive enough to swallow a real mismatch.
            if candidate == parameter || candidate.is_unknown() {
                continue;
            }
            *parameter = surge_ts_types::union_type(vec![parameter.clone(), candidate.clone()]);
            folded = true;
        }
    }

    ctx.diagnostics.truncate(diagnostics_before);
    if members.is_empty() {
        members.push(kept_member);
    }

    if !folded {
        return Cow::Owned(instantiated.into_owned().with_overloads(members));
    }

    Cow::Owned(
        alloc_function_type(
            parameters,
            instantiated.return_type().clone(),
            instantiated.is_variadic(),
            instantiated.required_parameter_count(),
        )
        .with_overloads(members),
    )
}

/// The declaring interface's own type parameters, bound by the reference that
/// named it. They are not inferred from the call and must not take part in the
/// "inference found nothing" test, but the written annotation mentions them, so
/// re-resolution needs them present. A name the signature already bound wins:
/// an inner type parameter shadows an outer one of the same name.
/// The written element a call argument lands in when the signature's last
/// parameter is a rest tuple (`...params: [TClient] | [TClient, Config<T>]`):
/// the tuple arm with the call's arity is selected and its element at the
/// argument's offset is the inference target. `None` when the last parameter
/// is not such a tuple, or no arm has that arity. Only a rest parameter
/// spreads over positions (`tryGetTypeAtPosition`): `f<T>(x: [T])` takes its
/// one argument whole.
fn rest_tuple_parameter_element(
    function_signature: &FunctionSignatureInfo,
    arity: usize,
    index: usize,
) -> Option<&ParsedType> {
    if !function_signature.rest {
        return None;
    }
    let last = function_signature.parameter_types.len().checked_sub(1)?;
    if index < last {
        return None;
    }
    let rest_type = function_signature.parameter_types.get(last)?.as_ref()?;
    let offset = index - last;
    let tuple_arity = arity - last;
    let arms: Vec<&std::sync::Arc<Vec<ParsedType>>> = match rest_type {
        ParsedType::Tuple(elements) => vec![elements],
        ParsedType::Union(members) => members
            .iter()
            .filter_map(|member| match member {
                ParsedType::Tuple(elements) => Some(elements),
                _ => None,
            })
            .collect(),
        _ => return None,
    };
    arms.iter()
        .find(|elements| elements.len() == tuple_arity)
        .and_then(|elements| elements.get(offset))
}

enum RestInferenceTarget<'a> {
    Whole(&'a ParsedType),
    Element(&'a ParsedType),
}

/// What a rest argument infers against. A rest written as one of the
/// signature's own type parameters (`<E extends any[]>(...args: E)`) is bound
/// to the tuple of all rest arguments; `f(1, [1, 2])` binds `E` to
/// `[number[]]`, not `number[]` as matching the first argument alone did (which
/// then rejected that very argument against the element `number`). A rest
/// written as an array (`...args: T[]`) infers its element from each argument.
/// A tuple-shaped rest keeps the positional path.
fn rest_parameter_inference_target<'a>(
    written: &'a ParsedType,
    function_signature: &FunctionSignatureInfo,
) -> Option<RestInferenceTarget<'a>> {
    match written {
        ParsedType::Array(element) => Some(RestInferenceTarget::Element(element)),
        ParsedType::Named(named)
            if named.type_arguments.is_empty()
                && function_signature
                    .type_parameters
                    .iter()
                    .any(|type_parameter| type_parameter.name == named.name) =>
        {
            Some(RestInferenceTarget::Whole(written))
        }
        _ => None,
    }
}

/// The tuple of the rest arguments' (widened) types, or `None` when any of
/// them is unresolved — a partial tuple would bind the wrong arity.
fn rest_arguments_tuple(
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let mut elements = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let diagnostics_before = ctx.diagnostics().len();
        let inferred = infer_expression(&argument.expression, symbols, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        let Some(argument_type) = inferred.flowing_type() else {
            return None;
        };
        if argument_type.is_degraded()
            || crate::checks::expr::carries_leaked_type_parameter(&argument_type, ctx)
        {
            return None;
        }
        // `f(...xs)` contributes the elements of `xs`, not `xs` itself. Only a
        // fixed tuple has a known length: spreading an array makes the whole
        // rest list variadic, which no `Type::Tuple` can stand for, so the
        // parameter is left uninferred rather than bound to a tuple of one.
        if argument.spread {
            let Type::Tuple(spread_elements) = argument_type.peeled() else {
                return None;
            };
            elements.extend(spread_elements);
            continue;
        }
        // `getSpreadArgumentType` widens fresh literals only: a binding of a
        // declared literal union (`actions: { onUpdate?: Act }`) keeps it.
        elements.push(if argument_is_fresh_literal(&argument.expression) {
            crate::checks::expr::widen_type(&argument_type)
        } else {
            argument_type
        });
    }
    Some(Type::Tuple(elements))
}

fn seed_outer_type_arguments(
    substitution: &mut TypeParameterSubstitution,
    outer_type_arguments: &[(String, Type)],
) {
    for (name, ty) in outer_type_arguments {
        if substitution.get(name).is_some() {
            continue;
        }
        substitution.insert(name.clone(), ty.clone());
    }
}

fn is_declaration_backed_lazy_signature(function_type: &FunctionType) -> bool {
    function_type
        .parameters()
        .iter()
        .chain(std::iter::once(function_type.return_type()))
        .any(|ty| {
            matches!(
                ty,
                Type::Reference(reference)
                    if reference.id.contains("\0signature-annotation\0")
            )
        })
}

/// Runs `resolve` with the checker positioned in the signature's declaring
/// file and namespace, so annotations written there (a module-local or imported
/// name the caller cannot see) resolve against that scope rather than the call
/// site's.
fn with_declaring_scope<R>(
    function_signature: &FunctionSignatureInfo,
    ctx: &mut CheckerContext,
    resolve: impl FnOnce(&mut CheckerContext) -> R,
) -> R {
    let declaring_file = function_signature
        .declaring_file
        .as_deref()
        .filter(|file| *file != ctx.file_name);
    let saved_file_name = declaring_file.map(|file| {
        let saved = ctx.file_name.clone();
        ctx.set_file_name(file.to_string());
        saved
    });
    let namespace_prefix = function_signature.namespace_prefix.clone();
    if let Some(prefix) = namespace_prefix.as_deref() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix.to_string());
    }

    let resolved = resolve(ctx);

    if namespace_prefix.is_some() {
        ctx.namespace_member_prefix_stack.pop();
        ctx.namespace_member_resolution_depth -= 1;
    }
    if let Some(saved) = saved_file_name {
        ctx.set_file_name(saved);
    }
    resolved
}

/// The generic declaration a signature's handle carries. An overload fold's
/// is only its first member's.
fn own_generic_declaration(function_type: &FunctionType) -> Option<(&FunctionSignatureInfo, &[(String, Type)])> {
    if function_type.overloads().is_some() {
        return None;
    }
    let declaration = function_type.declaration()?;
    if let Some(member) = declaration.downcast_ref::<DeclaredMemberSignature>() {
        return (!member.signature.type_parameters.is_empty())
            .then(|| (&*member.signature, member.outer_type_arguments.as_slice()));
    }
    declaration
        .downcast_ref::<FunctionSignatureInfo>()
        .filter(|signature| !signature.type_parameters.is_empty())
        .map(|signature| (signature, &[][..]))
}

/// Explicit type arguments on a signature known only by its type (a
/// parameter's or a variable's annotation): its own type-parameter names,
/// read off the rendered head when that is a plain list, take the arguments.
fn instantiate_written_type_parameters(
    function_type: &FunctionType,
    type_arguments: &[ParsedType],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    if type_arguments.is_empty() || function_type.declaration().is_some() {
        return None;
    }
    let names: Vec<String> =
        function_type.type_parameter_head()?.split(',').map(|name| name.trim().to_string()).collect();
    let plain = |name: &String| {
        !name.is_empty() && name.chars().all(|character| character.is_alphanumeric() || matches!(character, '_' | '$'))
    };
    if names.len() != type_arguments.len() || !names.iter().all(plain) {
        return None;
    }
    let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
    let resolved: Vec<Type> =
        type_arguments.iter().map(|argument| crate::infer::map_parsed_type(argument.clone(), ctx)).collect();
    ctx.symbols = saved_symbols;
    if resolved.iter().any(type_argument_is_unresolved) {
        return None;
    }
    Some(surge_ts_types::instantiate_named_type_parameters(function_type, &names, &resolved))
}

pub(crate) fn instantiate_function_type_with_substitution<'a>(
    function_type: &'a FunctionType,
    function_signature: &FunctionSignatureInfo,
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> Cow<'a, FunctionType> {
    // The declared parameter/return annotations may reference the declaring
    // module's local types (an imported generic like react-hook-form's
    // `useForm(props?: UseFormProps<…>)`), which the caller's file scope cannot
    // see; resolving under the declaring file keys the `module_scope_by_file`
    // fallback to the right per-file scope. Substituted type arguments are
    // already-resolved `Type`s, so they are unaffected by the swap.
    let declaring_file = function_signature
        .declaring_file
        .as_deref()
        .filter(|file| *file != ctx.file_name);
    let saved_file_name = declaring_file.map(|file| {
        let saved = ctx.file_name.clone();
        ctx.set_file_name(file.to_string());
        saved
    });
    // A namespace member's annotations name its siblings unqualified; they live
    // under qualified keys, so re-resolution needs the declaring namespace's
    // scope (the same one the type-member bodies expand in).
    let namespace_prefix = function_signature.namespace_prefix.clone();
    if let Some(prefix) = namespace_prefix.as_deref() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix.to_string());
    }

    // Re-resolving a written annotation under a substitution is not a fresh
    // declaration check: the declaration was already checked where it was
    // written, against the type parameter's *constraint*. Anything raised here
    // is raised at the declaration's span from a call site that cannot see it —
    // `(o: K[1])` under `K = [""]` reported a false TS2493 on the parameter
    // list of a generic the call never looked at.
    let diagnostics_before = ctx.diagnostics.len();

    let instantiated = with_type_copy_reason(TypeCopyReason::CallResolution, || {
        let mut instantiated_parameters = Vec::with_capacity(function_type.parameters().len());
        for (index, parameter) in function_type.parameters().iter().enumerate() {
            let Some(parsed_parameter) = function_signature
                .parameter_types
                .get(index)
                .and_then(|ty| ty.clone())
            else {
                instantiated_parameters.push(parameter.clone());
                continue;
            };

            instantiated_parameters.push(map_parsed_type_with_substitution(
                parsed_parameter,
                ctx,
                substitution,
            ));
        }

        let instantiated_return_type = match function_signature.return_type.as_ref() {
            Some(return_type) => {
                map_parsed_type_with_substitution(return_type.clone(), ctx, substitution)
            }
            None => function_signature
                .body_return
                .as_ref()
                .and_then(|source| {
                    crate::checks::function::instantiated_body_return(
                        source,
                        substitution,
                        &instantiated_parameters,
                        ctx,
                    )
                })
                .unwrap_or_else(|| function_type.return_type().clone()),
        };

        // The written `this` is checked against the call's receiver
        // (TS2684); one naming the signature's own type parameters would need
        // them substituted, which only the parsed types above are.
        let this_type = function_type
            .this_type()
            .filter(|this_type| !crate::checks::expected::mentions_type_parameter(this_type))
            .cloned();
        Cow::Owned(
            alloc_function_type(
                instantiated_parameters,
                instantiated_return_type,
                function_type.is_variadic(),
                function_type.required_parameter_count(),
            )
            .with_this_type(this_type),
        )
    });

    ctx.diagnostics.truncate(diagnostics_before);

    if namespace_prefix.is_some() {
        ctx.namespace_member_prefix_stack.pop();
        ctx.namespace_member_resolution_depth -= 1;
    }
    if let Some(saved) = saved_file_name {
        ctx.set_file_name(saved);
    }
    instantiated
}

pub(crate) fn instantiate_function_return_type_for_call(
    function_type: &FunctionType,
    function_signature: Option<&FunctionSignatureInfo>,
    type_arguments: &[ParsedType],
    type_argument_span: Option<TextSpan>,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    with_type_copy_reason(TypeCopyReason::CallResolution, || {
        let instantiated = instantiate_function_type(
            function_type,
            function_signature,
            &[],
            type_arguments,
            type_argument_span,
            arguments,
            None,
            symbols,
            ctx,
        );
        function_signature
            .and_then(|kept| {
                super::select_overload_return_type_by_signatures(function_type, kept, type_arguments, arguments, symbols, ctx)
            })
            .or_else(|| super::select_overload_return_type_for_inferred_call(function_type, arguments, symbols, ctx))
            .unwrap_or_else(|| instantiated.return_type().clone())
    })
}

pub(crate) fn explicit_type_argument_substitution(
    function_signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    ctx: &mut CheckerContext,
) -> TypeParameterSubstitution {
    let mut substitution = TypeParameterSubstitution::new();
    for (index, type_parameter) in function_signature.type_parameters.iter().enumerate() {
        // A call may supply fewer type arguments than the signature declares;
        // the rest take their declared default. Leaving them out of the
        // substitution let the *name* survive into the instantiated
        // parameter/return types, where it read as an unknown type
        // (`registry<GlobalMeta>()` against `registry<T, S>(): Registry<T, S>`
        // reported a false TS2304 for `S`). Defaults resolve under the
        // substitution built so far, so one that names an earlier parameter
        // (`S extends T = T`) sees it.
        let resolved = match type_arguments.get(index) {
            Some(type_argument) => map_parsed_type_with_substitution(
                type_argument.clone(),
                ctx,
                &TypeParameterSubstitution::new(),
            ),
            None => match type_parameter.default_type.clone() {
                // A default annotation is written in the DECLARING file and may
                // name that module's imports (`TSSRContext = NextPageContext`,
                // which `@trpc/next` imports and the app calling it cannot see),
                // so it resolves under the declaring scope, not the call site's.
                Some(default_type) => with_declaring_scope(function_signature, ctx, |ctx| {
                    map_parsed_type_with_substitution(default_type, ctx, &substitution)
                }),
                // No default: the parameter is genuinely unconstrained here, so
                // it degrades rather than leaking its name.
                None => Type::Unknown,
            },
        };
        substitution.insert(type_parameter.name.clone(), resolved);
    }
    substitution
}

/// tsc's `checkTypeArguments` for one signature: the first written type
/// argument that does not satisfy its parameter's constraint instantiated with
/// every argument (defaults filling the rest), paired with that constraint.
/// The arguments name types visible at the call; the constraint resolves where
/// it was written. A constraint that computes keys or branches is left alone —
/// `keyof` has [`enforce_explicit_keyof_constraints`] — and so is a pair either
/// side of which surge could not model.
pub(crate) fn unsatisfied_type_argument(
    function_signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Type, Type)> {
    unsatisfied_type_argument_at(function_signature, type_arguments, symbols, ctx)
        .map(|(_, argument, constraint)| (argument, constraint))
}

/// [`unsatisfied_type_argument`] with the position of the argument it rejects.
pub(crate) fn unsatisfied_type_argument_at(
    function_signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(usize, Type, Type)> {
    let related: Vec<(usize, &surge_ts_syntax::ParsedTypeParameter)> = function_signature
        .type_parameters
        .iter()
        .take(type_arguments.len())
        .enumerate()
        .filter(|(_, parameter)| {
            parameter
                .constraint
                .as_ref()
                .is_some_and(|constraint| !constraint_computes_keys(constraint))
        })
        .collect();
    if related.is_empty() {
        return None;
    }
    let checkpoint = ctx.diagnostics().len();
    let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
    let substitution = explicit_type_argument_substitution(function_signature, type_arguments, ctx);
    ctx.symbols = saved_symbols;
    let mut unsatisfied = None;
    for (index, parameter) in related {
        let (Some(constraint), Some(argument)) =
            (parameter.constraint.clone(), substitution.get(&parameter.name).cloned())
        else {
            continue;
        };
        let constraint = with_declaring_scope(function_signature, ctx, |ctx| {
            map_parsed_type_with_substitution(constraint, ctx, &substitution)
        });
        // A primitive lacking a member the constraint requires fails however
        // deep the constraint's own gaps lie (a lib interface's walk can
        // exhaust before it ends).
        if crate::checks::assign::definite_primitive_member_mismatch(&argument, &constraint) {
            unsatisfied = Some((index, argument, constraint));
            break;
        }
        if crate::checks::function::type_contains_degradation(&argument)
            || crate::checks::function::type_contains_degradation(&constraint)
        {
            continue;
        }
        // Without the lib's `BigInt` interface a `bigint` has no apparent
        // members, so the weak-type check does not apply to it.
        let bigint_without_members = matches!(argument, Type::BigInt)
            && ctx.lookup_type_declaration("BigInt").is_none()
            && surge_ts_types::has_no_common_properties(&argument, &constraint);
        if !bigint_without_members && !surge_ts_types::is_assignable_to(&argument, &constraint) {
            unsatisfied = Some((index, argument, constraint));
            break;
        }
    }
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    unsatisfied
}

/// Whether a written constraint computes keys or branches somewhere
/// (`keyof`, an indexed access, a conditional or mapped type).
fn constraint_computes_keys(constraint: &ParsedType) -> bool {
    match constraint {
        ParsedType::KeyOf(_)
        | ParsedType::IndexedAccess(_)
        | ParsedType::Conditional(_)
        | ParsedType::Mapped(_) => true,
        ParsedType::Named(named) => named.type_arguments.iter().any(constraint_computes_keys),
        ParsedType::Array(inner) | ParsedType::Readonly(inner) => constraint_computes_keys(inner),
        ParsedType::Union(members)
        | ParsedType::Intersection(members)
        | ParsedType::Tuple(members) => members.iter().any(constraint_computes_keys),
        _ => false,
    }
}

/// Binds the type parameters inference left untouched to their declared
/// defaults — but only when that completes the binding, i.e. every parameter
/// of the signature is then either inferred or defaulted. `vi.fn()` is the
/// shape that needs it: `fn<T extends Procedure = Procedure>(impl?: T):
/// Mock<T>` called with no argument infers nothing, and without the default
/// the result kept the bare parameter, so `Mock<T>`'s conditional operand
/// (which carries the call signature) could never resolve.
///
/// The completeness rule is what makes this safe. Defaulting *some* parameters
/// while another stays a placeholder moved the call off the all-unknown bail
/// and into an instantiation with a placeholder argument, which picked a wrong
/// conditional branch (ofetch's `$fetch<T = any, R = "json">(url, options)`,
/// where surge could not infer `R` from the options object). Inference seeds
/// every parameter as a placeholder and inserts any candidate as a real
/// binding, so the placeholder flag separates "nothing inferred" from
/// "`unknown` inferred". Defaults resolve in the declaring scope, under the
/// bindings made so far, so `S extends T = T` sees `T`.
fn apply_uninferred_type_parameter_defaults(
    function_signature: &FunctionSignatureInfo,
    arguments: &[ParsedCallArgument],
    argument_types: &[Option<Type>],
    expected_return_type: Option<&Type>,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> bool {
    // A parameter a supplied argument's annotation mentions is one surge failed
    // to infer, not one tsc would default: zod's `hash(alg, { enc: "base64" })`
    // against `Enc = "hex"` reported every call once the default stood in for
    // the failed inference. An unannotated parameter counts as mentioning
    // everything. The contextual return type is handled below.
    let rest_index = function_signature
        .rest
        .then(|| function_signature.parameter_types.len().checked_sub(1))
        .flatten();
    if arguments.len() > function_signature.parameter_types.len() && rest_index.is_none() {
        return false;
    }
    // A default written over a parameter surge did infer stays out: that
    // inference can lose what tsc keeps (a spread tuple `readonly [...T]` is
    // read as `T`), and the default would carry the loss into the result.
    let names_inferred = |type_parameter: &surge_ts_syntax::ParsedTypeParameter| {
        type_parameter
            .default_type
            .as_ref()
            .or(type_parameter.constraint.as_ref())
            .is_some_and(|written| {
                function_signature.type_parameters.iter().any(|other| {
                    !substitution.is_placeholder(&other.name) && parsed_type_mentions_name(written, &other.name)
                })
            })
    };
    // A `null` or `undefined` argument gives no parameter anything to infer,
    // and neither does one lacking every member that names it: an object
    // literal judged by its keys, any other argument by its inferred type.
    let mentioned_by_argument = |type_parameter: &surge_ts_syntax::ParsedTypeParameter, ctx: &mut CheckerContext| {
        let name = type_parameter.name.as_str();
        arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| {
                argument.spread
                    || !matches!(argument.expression, ParsedExpression::NullLiteral | ParsedExpression::UndefinedLiteral)
            })
            .any(|(index, argument)| {
                let (parameter_index, rest_position) = match rest_index {
                    Some(rest) if index >= rest => (rest, Some(index - rest)),
                    _ => (index, None),
                };
                let Some(Some(parameter_type)) = function_signature.parameter_types.get(parameter_index) else {
                    return true;
                };
                // A rest parameter spelled as tuples is read at the argument's
                // own position: `[string] | [string, Config<S>]` names `S` only
                // past the first argument.
                let slots = rest_position
                    .filter(|_| !argument.spread)
                    .and_then(|position| rest_tuple_elements_at(parameter_type, position))
                    .unwrap_or_else(|| vec![parameter_type]);
                slots.into_iter().any(|slot| {
                    parsed_type_mentions_name(slot, name)
                        && (argument.spread
                            || names_inferred(type_parameter)
                            || !(object_literal_records_nothing(slot, &argument.expression, name, function_signature, ctx)
                                || argument_types.get(index).and_then(Option::as_ref).is_some_and(|source| {
                                    with_declaring_scope(function_signature, ctx, |ctx| {
                                        inference_provably_records_nothing(slot, Some(source), name, ctx, 0)
                                    })
                                })))
                })
            })
    };
    let uninferred: Vec<&surge_ts_syntax::ParsedTypeParameter> = function_signature
        .type_parameters
        .iter()
        .filter(|type_parameter| substitution.is_placeholder(&type_parameter.name))
        .collect();
    if uninferred.is_empty()
        || uninferred
            .iter()
            .any(|type_parameter| mentioned_by_argument(type_parameter, ctx))
    {
        return false;
    }
    // The contextual return type has already been inferred from by the time
    // defaults apply, so a parameter it left a placeholder is one of two
    // things: tsc also found no candidate there — `getInferredType` then takes
    // the default (inference.go:1362) — or surge's walk could not follow the
    // shape. Only the first may default, so it has to be *proven*:
    // `console.warn = vi.fn()` is `Mock<Procedure>` in tsc, and surge left
    // `Mock<T>` bare because the expected type merely mentioned `T`.
    if let Some(expected) = expected_return_type {
        for type_parameter in &uninferred {
            let blocked = match function_signature.return_type.as_ref() {
                None => true,
                Some(return_type) => {
                    parsed_type_mentions_name(return_type, &type_parameter.name)
                        && !with_declaring_scope(function_signature, ctx, |ctx| {
                            inference_provably_records_nothing(
                                return_type,
                                Some(expected),
                                &type_parameter.name,
                                ctx,
                                0,
                            )
                        })
                }
            };
            if blocked {
                return false;
            }
        }
    }

    // `getInferredType` instantiates a default or constraint through the
    // inference mapper, which infers a parameter it names first
    // (`<U extends T, T extends A>` fixes `T` before `U`).
    let mut ordered = Vec::with_capacity(uninferred.len());
    let mut pending = uninferred;
    while !pending.is_empty() {
        let ready: Vec<usize> = (0..pending.len())
            .filter(|&index| {
                let written = pending[index].default_type.as_ref().or(pending[index].constraint.as_ref());
                written.is_none_or(|written| {
                    pending
                        .iter()
                        .enumerate()
                        .all(|(other, parameter)| other == index || !parsed_type_mentions_name(written, &parameter.name))
                })
            })
            .collect();
        if ready.is_empty() {
            ordered.append(&mut pending);
            break;
        }
        let mut batch: Vec<_> = ready.into_iter().rev().map(|index| pending.remove(index)).collect();
        batch.reverse();
        ordered.extend(batch);
    }
    let mut bound = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    for type_parameter in ordered {
        // No default: the *constraint* stands in, as tsc's inference does when a
        // parameter has no candidate at all. Leaving it unbound kept the
        // declaration's own name in the result, and a conditional over it then
        // decided from a type surge never resolved — `createNext({})` (no
        // explicit type argument) picked tRPC's key-collision arm where tsc
        // reduces `keyof Decorated<object> & keyof Builtins` to `never` and
        // takes the clean one. With neither, `getInferredType` settles on
        // `unknown` (inference.go `getInferredType`).
        let Some(default_type) = type_parameter
            .default_type
            .clone()
            .or_else(|| type_parameter.constraint.clone())
        else {
            bound.insert(type_parameter.name.clone(), Type::GenuineUnknown);
            continue;
        };
        let snapshot = bound.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
            map_parsed_type_with_substitution(default_type, ctx, &snapshot)
        });
        // A written `unknown` keyword is a real type, not a hole, wherever it
        // sits: `S = unknown` or `O extends { e?: unknown }` must not abandon
        // the whole binding the way an unresolved default does.
        if contains_unresolved_hole(&resolved) {
            return false;
        }
        bound.insert(type_parameter.name.clone(), resolved);
    }
    *substitution = bound;
    true
}

/// The element types a rest parameter written as tuples (or a union of them,
/// or an array) gives the argument at `position`; `None` for any other shape.
fn rest_tuple_elements_at(parameter_type: &ParsedType, position: usize) -> Option<Vec<&ParsedType>> {
    match parameter_type {
        ParsedType::Tuple(elements) => Some(elements.get(position).into_iter().collect()),
        ParsedType::Array(element) => Some(vec![&**element]),
        ParsedType::Union(members) => {
            let mut slots = Vec::new();
            for member in members.iter() {
                slots.extend(rest_tuple_elements_at(member, position)?);
            }
            Some(slots)
        }
        _ => None,
    }
}

/// Whether inferring from an object literal argument provably records nothing
/// for `name`. Only the written member names matter to that proof, so the
/// literal is not evaluated.
fn object_literal_records_nothing(
    parameter_type: &ParsedType,
    argument: &ParsedExpression,
    name: &str,
    function_signature: &FunctionSignatureInfo,
    ctx: &mut CheckerContext,
) -> bool {
    let ParsedExpression::ObjectLiteral { properties, .. } = argument else {
        return false;
    };
    let mut surface = surge_ts_types::PropertyMap::default();
    for property in properties {
        if property.is_spread
            || property.computed_key.is_some()
            || property.unnamed_key_value.is_some()
            || property.member_key.is_some()
        {
            return false;
        }
        surface.insert(property.name.as_str().into(), surge_ts_types::ObjectProperty::required(Type::Any));
    }
    let source = Type::Object(alloc_object_type(surface, None));
    with_declaring_scope(function_signature, ctx, |ctx| {
        inference_provably_records_nothing(parameter_type, Some(&source), name, ctx, 0)
    })
}

thread_local! {
    /// Whether the argument being inferred from is written as a function
    /// literal, whose type therefore has no alias identity of its own.
    static SOURCE_IS_FUNCTION_LITERAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether the argument being inferred from is written as an object or
    /// array literal. surge's types carry no freshness, so this is what marks a
    /// structural candidate taken from it as an object or array literal type
    /// (`isObjectOrArrayLiteralType`).
    static SOURCE_IS_OBJECT_OR_ARRAY_LITERAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Whether the argument's literal types are fresh, which is what lets a
    /// fixed inference widen them: one the call's contextual type names was
    /// made regular (`getWidenedLiteralLikeTypeForContextualType`).
    static SOURCE_IS_FRESH_LITERAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// tsc's `InferenceState.contravariant`: the walk is inside an odd number
    /// of signature parameter positions.
    static INFERENCE_CONTRAVARIANT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// tsc's `InferenceState.bivariant`: the walk has entered a method's
    /// signature, whose parameters are related bivariantly and so infer as
    /// ordinary candidates.
    static INFERENCE_BIVARIANT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The declared parameter type the current argument is inferred against,
    /// which decides whether a candidate is inferred to a top-level occurrence
    /// of its type parameter. `None` for the contextual return type, whose
    /// inferences never clear `topLevel`.
    static INFERENCE_ROOT_PARAMETER: std::cell::RefCell<Option<ParsedType>> =
        const { std::cell::RefCell::new(None) };
    /// Whether the walk infers from the call's contextual return type, whose
    /// inferences only fill what the arguments left open — tsc's
    /// `InferencePriority.ReturnType`, below every argument's.
    static INFERRING_FROM_RETURN_TYPE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// tsc's `InferenceState.depth`: how many `inferFromTypeArguments` levels
    /// the walk is inside.
    static INFERENCE_TYPE_ARGUMENT_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Inferences made to a type parameter so far (`inferFromTypes` reaching
    /// one), which is what `inferToMultipleTypes` asks when it records whether a
    /// source matched some target.
    static INFERENCES_MADE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The argument being inferred from is a written `unknown`; see
    /// [`is_inference_hole`].
    static WRITTEN_UNKNOWN_ARGUMENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The type parameters the last inference bound to an `unknown` an
    /// argument supplied, read by its caller before any other inference runs.
    static UNKNOWN_INFERRED_FROM_ARGUMENTS: std::cell::RefCell<Vec<std::sync::Arc<str>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `walk` one `inferFromTypeArguments` level deeper.
fn inferring_from_type_arguments<R>(walk: impl FnOnce() -> R) -> R {
    INFERENCE_TYPE_ARGUMENT_DEPTH.set(INFERENCE_TYPE_ARGUMENT_DEPTH.get() + 1);
    let result = walk();
    INFERENCE_TYPE_ARGUMENT_DEPTH.set(INFERENCE_TYPE_ARGUMENT_DEPTH.get() - 1);
    result
}

fn with_inference_root<R>(root: &ParsedType, walk: impl FnOnce() -> R) -> R {
    let outer = INFERENCE_ROOT_PARAMETER.with(|cell| cell.replace(Some(root.clone())));
    let result = walk();
    INFERENCE_ROOT_PARAMETER.with(|cell| cell.replace(outer));
    result
}

fn with_bivariant_inference<R>(bivariant: bool, walk: impl FnOnce() -> R) -> R {
    let outer = INFERENCE_BIVARIANT.replace(INFERENCE_BIVARIANT.get() || bivariant);
    let result = walk();
    INFERENCE_BIVARIANT.set(outer);
    result
}

fn contains_unresolved_hole(ty: &Type) -> bool {
    match ty {
        Type::GenuineUnknown => false,
        Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => true,
        Type::Array(element) => contains_unresolved_hole(element),
        Type::Reference(reference) if reference.is_readonly_array() => {
            reference.arguments.iter().any(contains_unresolved_hole)
        }
        Type::Tuple(elements) => elements.iter().any(contains_unresolved_hole),
        Type::Function(function) => {
            function.parameters().iter().any(contains_unresolved_hole)
                || contains_unresolved_hole(function.return_type())
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| contains_unresolved_hole(&property.ty))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(contains_unresolved_hole)
        }
        Type::Union(union) => union.types().iter().any(contains_unresolved_hole),
        _ => false,
    }
}

/// Whether tsc's `inferFromTypes(source, target)` provably records no
/// candidate for the type parameter `name`. `false` means a candidate may
/// exist *or* surge cannot tell, so a caller only ever acts on `true`.
///
/// `source` is `None` when the source type is not known (a parameter position
/// paired by `applyToParameterTypes`); only rules that hold for every source
/// apply then. Each rule cites the Go it mirrors.
fn inference_provably_records_nothing(
    target: &ParsedType,
    source: Option<&Type>,
    name: &str,
    ctx: &mut CheckerContext,
    depth: usize,
) -> bool {
    const MAX_DEPTH: usize = 12;
    // `inferFromTypes` returns at once for a target that cannot contain the
    // type variable (inference.go:66).
    if !parsed_type_mentions_name(target, name) {
        return true;
    }
    if depth >= MAX_DEPTH {
        return false;
    }
    match target {
        // A naked type variable records the source (inference.go:188-205).
        ParsedType::Named(named) if named.type_arguments.is_empty() => false,
        ParsedType::Named(named) => {
            reference_provably_records_nothing(named, source, name, ctx, depth)
        }
        // `inferToTemplateLiteralType`: a literal source the texts do not fit
        // captures nothing for any placeholder.
        ParsedType::TemplateLiteral(template) => {
            let Some(source @ Type::StringLiteral(_)) = source else {
                return false;
            };
            let texts: Vec<&str> = template.quasis.iter().map(String::as_str).collect();
            let targets = vec![Type::GenuineUnknown; template.interpolations.len()];
            let target_refs: Vec<&Type> = targets.iter().collect();
            !texts.iter().all(|text| text.is_empty())
                && surge_ts_types::infer_template_literal_placeholders(source, &texts, &target_refs).is_none()
        }
        // `inferToMultipleTypes`: every non-naked member is inferred to, and a
        // naked one records the source (inference.go:448-537).
        ParsedType::Intersection(members) | ParsedType::Union(members) => {
            members.iter().all(|member| {
                !matches!(member, ParsedType::Named(named)
                    if named.type_arguments.is_empty() && named.name == name)
                    && inference_provably_records_nothing(member, source, name, ctx, depth + 1)
            })
        }
        // `inferToConditionalType` with a non-conditional source infers to the
        // true and false branches only; the check type is not a target
        // (inference.go:554-564). An `infer` of the same name would shadow it.
        ParsedType::Conditional(conditional) => {
            !declares_infer_named(&conditional.extends_type, name)
                && inference_provably_records_nothing(
                    &conditional.true_type,
                    source,
                    name,
                    ctx,
                    depth + 1,
                )
                && inference_provably_records_nothing(
                    &conditional.false_type,
                    source,
                    name,
                    ctx,
                    depth + 1,
                )
        }
        // `{ [P in keyof T]: X }` with `T` inferred: `inferToMappedType` answers
        // for the mapped type (no template inference) and records only a reverse
        // mapped type, which `createReverseMappedType` refuses for a source with
        // no string index and no properties (inference.go:960-977, 1014-1019).
        // Over any other key set without an `as` clause, nothing is inferred to
        // the keys (`inferToMappedType` answers only for a type parameter or a
        // `keyof` one) and the members pair by name: `{ [P in C]: X[P] }`
        // (`Pick`, `Omit`) has a subset of `X`'s members, so it records at most
        // what `X` would.
        ParsedType::Mapped(mapped) => {
            let naked_or_keyof_naked = match mapped.constraint.as_ref() {
                ParsedType::Named(named) => named.type_arguments.is_empty(),
                ParsedType::KeyOf(inner) => {
                    matches!(inner.as_ref(), ParsedType::Named(named) if named.type_arguments.is_empty())
                }
                _ => false,
            };
            mapped.name_type.is_none()
                && if naked_or_keyof_naked {
                    matches!(mapped.constraint.as_ref(), ParsedType::KeyOf(inner)
                        if matches!(inner.as_ref(), ParsedType::Named(named)
                            if named.type_arguments.is_empty() && named.name == name))
                        && source.and_then(function_source).is_some()
                } else {
                    mapped.key_name != name
                        && match mapped.value_type.as_ref() {
                            ParsedType::IndexedAccess(access)
                                if matches!(access.index_type.as_ref(), ParsedType::Named(index)
                                    if index.type_arguments.is_empty() && index.name == mapped.key_name) =>
                            {
                                !parsed_type_mentions_name(&access.object_type, &mapped.key_name)
                                    && inference_provably_records_nothing(
                                        &access.object_type,
                                        source,
                                        name,
                                        ctx,
                                        depth + 1,
                                    )
                            }
                            template => !parsed_type_mentions_name(template, name),
                        }
                }
        }
        // An object type inferred from a function source: properties pair by
        // name, call signatures pair, and a function has no construct signature
        // or index info to pair (inference.go:822-826, 838-850).
        ParsedType::Function(function) => match source.and_then(source_surface) {
            Some(SourceSurface::Function) => {
                signature_provably_records_nothing(function, name, ctx, depth)
            }
            // No call signature on the source to pair with.
            Some(
                SourceSurface::Members(_)
                | SourceSurface::Properties(_)
                | SourceSurface::Indexes { .. }
                | SourceSurface::Nothing,
            ) => true,
            None => false,
        },
        ParsedType::Object(object) => {
            let Some(surface) = source.and_then(source_surface) else {
                return false;
            };
            let properties_record_nothing = object.properties.iter().all(|property| {
                !parsed_type_mentions_name(&property.ty, name)
                    || !source_has_property(&surface, &property.name, ctx)
            });
            let object_indexes_record_nothing = match &surface {
                SourceSurface::Properties(_) => indexes_record_nothing(
                    [object.string_index_type.as_deref(), object.number_index_type.as_deref()],
                    object.keyed_indexes.is_some(),
                    name,
                ),
                SourceSurface::Indexes { string, number } => {
                    object.keyed_indexes.is_none()
                        && applicable_indexes_record_nothing(
                            object.string_index_type.as_deref(),
                            object.number_index_type.as_deref(),
                            *string,
                            *number,
                            name,
                        )
                }
                _ => true,
            };
            properties_record_nothing
                && object_indexes_record_nothing
                && (!matches!(surface, SourceSurface::Function)
                    || object
                        .call_signature
                        .as_deref()
                        .into_iter()
                        .chain(object.call_signature_overloads.iter())
                        .all(|signature| {
                            signature_provably_records_nothing(signature, name, ctx, depth)
                        }))
        }
        _ => false,
    }
}

/// A generic reference. An alias is inferred to as its instantiated body; an
/// interface as an object type (inference.go:699-826).
fn reference_provably_records_nothing(
    named: &ParsedNamedType,
    source: Option<&Type>,
    name: &str,
    ctx: &mut CheckerContext,
    depth: usize,
) -> bool {
    let Some(handle) = lookup_declaration_for_inference(&named.name, ctx) else {
        return false;
    };
    // Two instantiations of one declaration infer argument to argument
    // (inference.go:79, 222); surge does not model which source that is. A
    // reference to any other declaration is inferred from structurally.
    if source.and_then(function_source).is_none() {
        let declaration_id = crate::infer::types::type_declaration_alias_id(
            handle.get(),
            &crate::infer::types::type_declaration_resolution_key(handle.get()),
        );
        let mut current = source.cloned();
        let mut hops = 0;
        while let Some(Type::Reference(reference)) = current {
            if reference.id == declaration_id || hops == 8 {
                return false;
            }
            hops += 1;
            current = Some(reference.resolve());
        }
    }
    match handle.get() {
        TypeDeclarationInfo::Alias(alias) => {
            if alias.body.type_parameters.len() < named.type_arguments.len() {
                return false;
            }
            let map = parameter_argument_map(&alias.body.type_parameters, &named.type_arguments);
            let body = crate::infer::substitute_parsed_type_parameters_deep(&alias.body.ty, &map);
            let file = alias.file_name.clone();
            with_inference_scope_file(&file, ctx, |ctx| {
                inference_provably_records_nothing(&body, source, name, ctx, depth + 1)
            })
        }
        TypeDeclarationInfo::Interface(interface) => {
            let Some(surface) = source.and_then(source_surface) else {
                return false;
            };
            if interface.body.type_parameters.len() < named.type_arguments.len() {
                return false;
            }
            let body = interface.body.clone();
            let file = interface.file_name.clone();
            let map = parameter_argument_map(&body.type_parameters, &named.type_arguments);
            with_inference_scope_file(&file, ctx, |ctx| {
                let members_record_nothing = body.members.iter().all(|member| {
                    let member_type =
                        crate::infer::substitute_parsed_type_parameters_deep(&member.ty, &map);
                    !parsed_type_mentions_name(&member_type, name)
                        || !source_has_property(&surface, &member.name, ctx)
                });
                let substitute = |index: &ParsedType| {
                    crate::infer::substitute_parsed_type_parameters_deep(index, &map)
                };
                let indexes_record_nothing = match &surface {
                    SourceSurface::Properties(_) => indexes_record_nothing(
                        [body.string_index_type.as_ref().map(substitute).as_ref(), body.number_index_type.as_ref().map(substitute).as_ref()],
                        body.keyed_indexes.is_some(),
                        name,
                    ),
                    SourceSurface::Indexes { string, number } => {
                        body.keyed_indexes.is_none()
                            && applicable_indexes_record_nothing(
                                body.string_index_type.as_ref().map(substitute).as_ref(),
                                body.number_index_type.as_ref().map(substitute).as_ref(),
                                *string,
                                *number,
                                name,
                            )
                    }
                    _ => true,
                };
                let signatures_record_nothing = !matches!(surface, SourceSurface::Function)
                    || body.call_signature.iter().all(|signature| {
                    match crate::infer::substitute_parsed_type_parameters_deep(
                        &ParsedType::Function(std::sync::Arc::new(signature.clone())),
                        &map,
                    ) {
                        ParsedType::Function(signature) => {
                            signature_provably_records_nothing(&signature, name, ctx, depth)
                        }
                        _ => false,
                    }
                });
                members_record_nothing
                    && indexes_record_nothing
                    && signatures_record_nothing
                    && body.extends.iter().all(|base| {
                        let base = ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                            type_argument_spans: Vec::new(),
                            name: base.name.clone(),
                            span: base.span,
                            type_arguments: base
                                .type_arguments
                                .iter()
                                .map(|argument| {
                                    crate::infer::substitute_parsed_type_parameters_deep(argument, &map)
                                })
                                .collect(),
                        }));
                        inference_provably_records_nothing(&base, source, name, ctx, depth + 1)
                    })
            })
        }
    }
}

/// `inferFromIndexTypes`: an object source's properties are inferred to every
/// index signature of the target. Keyed (template or symbol) indexes are not
/// read here, so one counts as recording.
/// `inferFromIndexTypes` over a source whose only members are index
/// signatures: a target's string index reads the source's string index, its
/// number index the number index or else the string one.
fn applicable_indexes_record_nothing(
    target_string: Option<&ParsedType>,
    target_number: Option<&ParsedType>,
    source_string: bool,
    source_number: bool,
    name: &str,
) -> bool {
    target_string.is_none_or(|index| !source_string || !parsed_type_mentions_name(index, name))
        && target_number
            .is_none_or(|index| !(source_number || source_string) || !parsed_type_mentions_name(index, name))
}

fn indexes_record_nothing(indexes: [Option<&ParsedType>; 2], keyed: bool, name: &str) -> bool {
    !keyed && indexes.into_iter().flatten().all(|index| !parsed_type_mentions_name(index, name))
}

/// `inferFromSignature`: parameters and return types pair positionally
/// (inference.go:853-866). Which source each target position meets depends on
/// arity and rest handling, so each is checked for every source.
fn signature_provably_records_nothing(
    signature: &surge_ts_syntax::ParsedFunctionType,
    name: &str,
    ctx: &mut CheckerContext,
    depth: usize,
) -> bool {
    // The target is erased (`getErasedSignature`), so its own type parameters
    // become `any`; one sharing the name would shadow it.
    if signature.type_parameters.iter().any(|parameter| parameter.name == name) {
        return false;
    }
    signature
        .parameters
        .iter()
        .all(|parameter| inference_provably_records_nothing(&parameter.ty, None, name, ctx, depth + 1))
        && inference_provably_records_nothing(&signature.return_type, None, name, ctx, depth + 1)
}

/// What `inferFromObjectTypes` can pair a target's members with, for the
/// sources whose answer is known: a function's apparent members, a primitive's
/// wrapper interface (`getApparentType`, inference.go), or nothing at all for a
/// source that is no object and has no apparent one.
enum SourceSurface {
    Function,
    Members(&'static [&'static str]),
    /// A closed object type with no signatures: its own properties, then
    /// `Object`'s (`getPropertyOfType`), and an inferable index for a target's
    /// index signatures (`inferFromIndexTypes`).
    Properties(Vec<std::sync::Arc<str>>),
    /// A type with index signatures and nothing else: its indexes are what a
    /// target's index signatures read (`getApplicableIndexInfo`).
    Indexes { string: bool, number: bool },
    Nothing,
}

fn source_surface(source: &Type) -> Option<SourceSurface> {
    match source.peeled() {
        Type::Function(_) => Some(SourceSurface::Function),
        Type::Object(object)
            if object.call_signature().is_none()
                && object.construct_signature().is_none()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && !object.synthetic_open_index =>
        {
            Some(SourceSurface::Properties(object.properties.keys().cloned().collect()))
        }
        Type::Object(object)
            if object.call_signature().is_none()
                && object.construct_signature().is_none()
                && object.properties.is_empty()
                && !object.synthetic_open_index
                && object.keyed_indexes.is_none()
                && !object.is_intersection =>
        {
            Some(SourceSurface::Indexes {
                string: object.string_index_type.is_some(),
                number: object.number_index_type.is_some(),
            })
        }
        Type::Boolean | Type::BooleanLiteral(_) => Some(SourceSurface::Members(&["Boolean", "Object"])),
        Type::String | Type::StringLiteral(_) => Some(SourceSurface::Members(&["String", "Object"])),
        Type::Number | Type::NumberLiteral(_) => Some(SourceSurface::Members(&["Number", "Object"])),
        Type::BigInt => Some(SourceSurface::Members(&["BigInt", "Object"])),
        Type::Symbol => Some(SourceSurface::Members(&["Symbol", "Object"])),
        Type::Undefined | Type::Null | Type::Void | Type::Never => Some(SourceSurface::Nothing),
        _ => None,
    }
}

/// Whether the source answers `member`. A lib surge cannot find counts as
/// declaring it, the direction that keeps a default out, and so does a
/// computed key against a source with computed keys of its own: surge names
/// one by its written text, and `[observable]` may be the very symbol
/// `[Symbol.obs]` names.
fn source_has_property(surface: &SourceSurface, member: &str, ctx: &CheckerContext) -> bool {
    if member.starts_with('[')
        && let SourceSurface::Properties(names) = surface
        && names.iter().any(|name| name.starts_with('['))
    {
        return true;
    }
    let interfaces: &[&str] = match surface {
        SourceSurface::Function => return function_source_has_property(member, ctx),
        SourceSurface::Members(interfaces) => interfaces,
        SourceSurface::Properties(names) if names.iter().any(|name| &**name == member) => return true,
        SourceSurface::Properties(_) | SourceSurface::Indexes { .. } => &["Object"],
        SourceSurface::Nothing => return false,
    };
    interfaces
        .iter()
        .any(|interface| lookup_declaration_for_inference(interface, ctx).is_none())
        || interfaces
            .iter()
            .any(|interface| interface_declares_member(interface, member, ctx, 0))
}

/// The function a source is, when it is one.
fn function_source(source: &Type) -> Option<surge_ts_types::FunctionType> {
    match source.peeled() {
        Type::Function(function) => Some(function),
        _ => None,
    }
}

/// `getPropertyOfType` on a callable source answers from `CallableFunction`
/// (or `Function`), then `Object` (checker.go:19240-19254). A lib surge cannot
/// find counts as declaring the member, the direction that keeps a default out.
fn function_source_has_property(member: &str, ctx: &CheckerContext) -> bool {
    const FALLBACKS: [&str; 3] = ["CallableFunction", "Function", "Object"];
    FALLBACKS
        .iter()
        .any(|interface| lookup_declaration_for_inference(interface, ctx).is_none())
        || FALLBACKS
            .iter()
            .any(|interface| interface_declares_member(interface, member, ctx, 0))
}

fn interface_declares_member(interface: &str, member: &str, ctx: &CheckerContext, depth: usize) -> bool {
    if depth > 8 {
        return true;
    }
    let Some(handle) = lookup_declaration_for_inference(interface, ctx) else {
        return false;
    };
    let TypeDeclarationInfo::Interface(info) = handle.get() else {
        return false;
    };
    info.body.members.iter().any(|declared| declared.name == member)
        || info
            .body
            .extends
            .iter()
            .any(|base| interface_declares_member(&base.name, member, ctx, depth + 1))
}

fn parameter_argument_map(
    parameters: &[surge_ts_syntax::ParsedTypeParameter],
    arguments: &[ParsedType],
) -> surge_ts_types::fx::FxHashMap<String, ParsedType> {
    parameters
        .iter()
        .zip(arguments.iter())
        .map(|(parameter, argument)| (parameter.name.clone(), argument.clone()))
        .collect()
}

/// Whether a conditional's `extends` clause introduces `infer <name>`. A shape
/// this walk does not descend into answers `true`, keeping a default out.
fn declares_infer_named(ty: &ParsedType, name: &str) -> bool {
    let in_signature = |signature: &surge_ts_syntax::ParsedFunctionType| {
        signature
            .parameters
            .iter()
            .any(|parameter| declares_infer_named(&parameter.ty, name))
            || declares_infer_named(&signature.return_type, name)
    };
    match ty {
        ParsedType::Infer(infer) => infer.name == name,
        ParsedType::Named(named) => named
            .type_arguments
            .iter()
            .any(|argument| declares_infer_named(argument, name)),
        ParsedType::Array(inner) | ParsedType::Readonly(inner) | ParsedType::KeyOf(inner) => {
            declares_infer_named(inner, name)
        }
        ParsedType::Tuple(members) | ParsedType::Union(members) | ParsedType::Intersection(members) => {
            members.iter().any(|member| declares_infer_named(member, name))
        }
        ParsedType::Function(function) => in_signature(function),
        ParsedType::Object(object) => {
            object
                .properties
                .iter()
                .any(|property| declares_infer_named(&property.ty, name))
                || object.call_signature.as_deref().is_some_and(in_signature)
                || object.construct_signature.as_deref().is_some_and(in_signature)
        }
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Undefined
        | ParsedType::Void
        | ParsedType::Any
        | ParsedType::ErrorType
        | ParsedType::Unknown
        | ParsedType::UnknownKeyword
        | ParsedType::Never
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_) => false,
        _ => true,
    }
}

/// Whether a parsed annotation names `name` anywhere within it. Positions this
/// walker does not descend into (`typeof`, indexed access, mapped, conditional,
/// template, predicate) report `true`: the caller uses a mention as a reason
/// *not* to act, so over-reporting is the safe direction.
fn parsed_type_names_literal(ty: &ParsedType) -> bool {
    match ty {
        ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_)
        | ParsedType::TemplateLiteral(_) => true,
        ParsedType::Union(members) | ParsedType::Intersection(members) => members.iter().any(parsed_type_names_literal),
        // A named type may be a literal union; it is not judged here.
        ParsedType::Named(_) => true,
        _ => false,
    }
}

pub(crate) fn parsed_type_mentions_name(ty: &ParsedType, name: &str) -> bool {
    match ty {
        ParsedType::Named(named) => {
            named.name == name
                || named
                    .type_arguments
                    .iter()
                    .any(|argument| parsed_type_mentions_name(argument, name))
        }
        ParsedType::Object(object) => {
            object
                .properties
                .iter()
                .any(|property| parsed_type_mentions_name(&property.ty, name))
                || object
                    .string_index_type
                    .iter()
                    .chain(object.number_index_type.iter())
                    .any(|index| parsed_type_mentions_name(index, name))
                || object
                    .construct_signature
                    .as_deref()
                    .into_iter()
                    .chain(object.call_signature.as_deref())
                    .chain(object.call_signature_overloads.iter())
                    .any(|signature| {
                        signature
                            .parameters
                            .iter()
                            .any(|parameter| parsed_type_mentions_name(&parameter.ty, name))
                            || parsed_type_mentions_name(&signature.return_type, name)
                    })
        }
        ParsedType::Array(inner) | ParsedType::KeyOf(inner) => {
            parsed_type_mentions_name(inner, name)
        }
        ParsedType::Union(members)
        | ParsedType::Intersection(members)
        | ParsedType::Tuple(members) => members
            .iter()
            .any(|member| parsed_type_mentions_name(member, name)),
        // A variadic tuple is where a middleware chain carries its accumulator
        // (`init: Creator<T, [...Mps, ['zustand/devtools', never]]>`). Falling
        // through to the `false` arm below said `Mps` had no inference source,
        // which let `apply_uninferred_type_parameter_defaults` bind it to its
        // declared `[]` before the contextual return type could infer it — so
        // the outer middleware was dropped from the mutator list.
        ParsedType::VariadicTuple(elements) => elements.iter().any(|element| match element {
            surge_ts_syntax::ParsedTupleElement::Fixed(ty)
            | surge_ts_syntax::ParsedTupleElement::Rest(ty, _) => {
                parsed_type_mentions_name(ty, name)
            }
        }),
        ParsedType::Function(function) => {
            function
                .parameters
                .iter()
                .any(|parameter| parsed_type_mentions_name(&parameter.ty, name))
                || parsed_type_mentions_name(&function.return_type, name)
        }
        // `Dict<T>[keyof Dict<T>]` mentions what its object and index do: a
        // contextual parameter read that way must not fix a type parameter
        // only the callback's return infers.
        ParsedType::IndexedAccess(access) => {
            parsed_type_mentions_name(&access.object_type, name) || parsed_type_mentions_name(&access.index_type, name)
        }
        ParsedType::Conditional(conditional) => [
            &conditional.check_type,
            &conditional.extends_type,
            &conditional.true_type,
            &conditional.false_type,
        ]
        .into_iter()
        .any(|branch| parsed_type_mentions_name(branch, name)),
        // Inside the mapped type its own key shadows a parameter it shares a
        // name with; only the key's constraint is written outside it.
        ParsedType::Mapped(mapped) => {
            parsed_type_mentions_name(&mapped.constraint, name)
                || mapped.key_name != name
                    && (parsed_type_mentions_name(&mapped.value_type, name)
                        || mapped
                            .name_type
                            .as_deref()
                            .is_some_and(|name_type| parsed_type_mentions_name(name_type, name)))
        }
        ParsedType::TemplateLiteral(template) => {
            template.interpolations.iter().any(|interpolation| parsed_type_mentions_name(interpolation, name))
        }
        ParsedType::Predicate(predicate) => {
            predicate.ty.as_ref().is_some_and(|ty| parsed_type_mentions_name(ty, name))
        }
        ParsedType::Infer(infer) => {
            infer.constraint.as_ref().is_some_and(|constraint| parsed_type_mentions_name(constraint, name))
        }
        // A `typeof` query reads a value of the declaring scope, which an
        // expression-bodied callback's contextual type re-read at the call
        // site would find there instead (zod's `makeSchema(factory: (z: typeof
        // zod4) => T)` beside a local `zod4`); it is kept answering every name.
        ParsedType::TypeOf(_) => true,
        _ => false,
    }
}

/// An inferred candidate that does not satisfy its parameter's constraint is
/// replaced by the constraint, as tsc does (`transform({ value: 1 })` against
/// `T extends string` instantiates `{ value: string }` and reports the
/// argument).
///
/// tsc substitutes unconditionally because its inference is complete. Here a
/// failed assignability check is at least as often surge's own inference miss,
/// and the substituted constraint re-types every parameter of the signature —
/// so one miss becomes a TS2345 at every argument of the call. Both operands
/// must therefore be settled before the check counts; an unsettled one leaves
/// the candidate alone.
/// Whether a candidate was replaced by a constraint that resolved to `never`.
fn enforce_inferred_constraints(
    function_signature: &FunctionSignatureInfo,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> bool {
    let mut replaced_by_never = false;
    for type_parameter in &function_signature.type_parameters {
        let Some(constraint) = type_parameter.constraint.clone() else {
            continue;
        };
        let Some(candidate) = substitution.get(&type_parameter.name).cloned() else {
            continue;
        };
        // A type variable candidate relates through its base constraint,
        // `unknown` when it has none: one that definitely lacks what this
        // parameter's constraint requires gives way to that constraint.
        if let Type::TypeParameter(variable) = &candidate
            && candidate.is_type_variable()
            && let Some(base) = surge_ts_types::type_variable::active_constraint(variable)
        {
            let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
                try_map_parsed_type_with_substitution(constraint.clone(), ctx, substitution)
            });
            if !resolved.had_error() {
                let resolved = resolved.into_ty();
                // A constraint that is itself a type variable of the body
                // (`then<U extends S>` inferring a `T`) is decided by relating
                // the two variables.
                let violated = if resolved.is_type_variable() {
                    !surge_ts_types::is_assignable_to(&candidate, &resolved)
                } else {
                    !resolved.is_degraded()
                    && match &base {
                        None => !surge_ts_types::is_assignable_to(&Type::GenuineUnknown, &resolved),
                        Some(base) => {
                            lacks_required_member(base, &resolved.peeled())
                                || array_lacks_required_member(base, &resolved)
                                || primitive_lacks_required_member(base, &resolved, ctx)
                        }
                    }
                };
                if violated {
                    substitution.set(type_parameter.name.clone(), resolved, false);
                    continue;
                }
            }
        }
        // An object candidate can still be judged by the members it lacks.
        let candidate_settled = constraint_operand_is_settled(&candidate);
        if !candidate_settled
            && !matches!(candidate.peeled(), Type::Object(_) | Type::Function(_))
        {
            continue;
        }
        let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
            try_map_parsed_type_with_substitution(constraint, ctx, substitution)
        });
        // A constraint that failed to resolve — an alias surge rejected as a
        // cycle, a name it could not find, a recursion it cut — carries no
        // verdict about the candidate. Enforcing the degraded shape anyway
        // installs it as the type argument and rejects every argument against
        // it.
        if resolved.had_error() {
            continue;
        }
        let resolved = resolved.into_ty();
        // A primitive candidate against a named constraint (`len(5)` for
        // `T extends HasLen`) is judged by the constraint's own shape: the
        // argument check reads that shape anyway, and a primitive seldom meets
        // a named constraint, so peeling it expands nothing a call would not.
        let settled_constraint = if lacks_required_member(&candidate, &resolved.peeled())
            || array_lacks_required_member(&candidate, &resolved)
            || primitive_lacks_required_member(&candidate, &resolved, ctx)
            || lacks_constraint_call_signature(&candidate, &resolved)
            || CHOOSING_OVERLOAD.get() && function_lacks_constraint_index(&candidate, &resolved)
            || (candidate_settled && surge_ts_types::is_global_function_interface(&resolved))
        {
            Some(resolved.clone())
        } else if matches!(candidate.peeled(), Type::Function(_))
            && matches!(resolved.peeled(), Type::Function(_))
        {
            // A signature candidate against a signature constraint read through
            // an alias (`R extends ReducerWithoutAction<any>`): the relation of
            // the two signatures decides it, arity included.
            Some(resolved.peeled())
        } else if candidate_is_primitive(&resolved) && is_plain_object_reference(&candidate) {
            // A named object type is no primitive whatever its members
            // (`NumberVariant extends Number` for `U extends T`, `T` a number).
            Some(resolved.clone())
        } else if !candidate_settled {
            None
        } else if constraint_operand_is_settled(&resolved) {
            Some(resolved.clone())
        } else if matches!(resolved, Type::Reference(_))
            && candidate_is_primitive(&candidate)
            && constraint_operand_is_settled(&resolved.peeled())
        {
            Some(resolved.clone())
        } else if matches!(candidate, Type::Undefined | Type::Null)
            && matches!(resolved, Type::Reference(_) | Type::Object(_) | Type::Function(_))
        {
            // No object or signature shape admits `undefined`/`null` where
            // they are not everywhere assignable, whatever its members.
            Some(resolved.clone())
        } else {
            None
        };
        let Some(settled_constraint) = settled_constraint else {
            continue;
        };
        if matches!(settled_constraint, Type::Any)
            || surge_ts_types::is_assignable_to(&candidate, &settled_constraint)
        {
            continue;
        }
        replaced_by_never |= matches!(resolved, Type::Never);
        substitution.set(type_parameter.name.clone(), resolved, false);
    }
    replaced_by_never
}

/// Whether an explicit type argument fails its parameter's constraint, both
/// resolved.
fn explicit_type_arguments_break_constraints(
    function_signature: &FunctionSignatureInfo,
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> bool {
    function_signature.type_parameters.iter().any(|type_parameter| {
        let (Some(constraint), Some(argument)) =
            (type_parameter.constraint.clone(), substitution.get(&type_parameter.name).cloned())
        else {
            return false;
        };
        if argument.is_degraded() {
            return false;
        }
        let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
            try_map_parsed_type_with_substitution(constraint, ctx, substitution)
        });
        if resolved.had_error() {
            return false;
        }
        let resolved = resolved.into_ty();
        !resolved.is_degraded() && !surge_ts_types::is_assignable_to(&argument, &resolved)
    })
}

/// What an inference neither records nor counts as inferred: surge's own
/// degradation sentinel, a type parameter no scope binds, and `unknown`
/// except as a whole argument read off a declaration or an assertion. An
/// active type variable is a candidate as in tsc, and so is that `unknown`:
/// `replaceData(prev?.data, x as unknown)` binds `TData` to `unknown`, not to
/// the outer `TData` alone. Any other `unknown` may be the
/// `getDefaultTypeArgumentType` of a nested generic call whose contextual
/// type was the outer call's own type parameter, which tsc's return mapper
/// resolves and surge does not (`m({ x: absorb() })`).
fn is_inference_hole(ty: &Type) -> bool {
    // A callback parameter's `unknown` (`(vars: unknown) => …`) was written:
    // surge types an unannotated one from context, never as the keyword.
    ty.is_degraded()
        && !ty.is_type_variable()
        && !(matches!(ty, Type::GenuineUnknown)
            && (WRITTEN_UNKNOWN_ARGUMENT.get() || INFERENCE_CONTRAVARIANT.get()))
}

/// An argument built only from declared values and literals: every `unknown`
/// its type holds was written somewhere, none left by a nested call.
fn assembles_written_types(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::StringLiteral(_)
        | ParsedExpression::NumberLiteral(_)
        | ParsedExpression::BigIntLiteral(_)
        | ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::UndefinedLiteral
        | ParsedExpression::NullLiteral => true,
        ParsedExpression::ArrayLiteral { elements, .. } => elements
            .iter()
            .all(|element| element.omitted || assembles_written_types(&element.expression)),
        ParsedExpression::ObjectLiteral { properties, .. } => properties.iter().all(|property| {
            !property.is_method
                && !property.is_accessor
                && property.computed_key.is_none()
                && assembles_written_types(&property.value)
        }),
        other => reads_a_written_type(other),
    }
}

fn reads_a_written_type(expression: &ParsedExpression) -> bool {
    matches!(
        expression,
        ParsedExpression::Identifier { .. }
            | ParsedExpression::PropertyAccess { .. }
            | ParsedExpression::OptionalPropertyAccess { .. }
            | ParsedExpression::IndexAccess { .. }
            | ParsedExpression::OptionalIndexAccess { .. }
            | ParsedExpression::ElementAccess { .. }
            | ParsedExpression::TypeAssertion { .. }
            | ParsedExpression::SatisfiesExpression { .. }
    )
}

thread_local! {
    /// The arguments of the call whose inference put a constraint resolving
    /// to `never` in place of a candidate that broke it (`getInferredType`).
    static NEVER_FROM_BROKEN_CONSTRAINT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether `arguments`' call took a `never` from a broken constraint, which
/// makes its `never` parameters no exhaustiveness assertion.
pub(super) fn take_never_from_broken_constraint(arguments: &[ParsedCallArgument]) -> bool {
    let taken = NEVER_FROM_BROKEN_CONSTRAINT.replace(0);
    !arguments.is_empty() && taken == arguments.as_ptr() as usize
}

thread_local! {
    /// Set while an overload member is instantiated for `chooseOverload`.
    static CHOOSING_OVERLOAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn while_choosing_overload<R>(instantiate: impl FnOnce() -> R) -> R {
    let outer = CHOOSING_OVERLOAD.replace(true);
    let result = instantiate();
    CHOOSING_OVERLOAD.set(outer);
    result
}

thread_local! {
    /// The awaited type an async function's returned value, or an `await`
    /// operand, is contextually typed by.
    static AWAITED_CONTEXT: std::cell::RefCell<Option<Type>> = const { std::cell::RefCell::new(None) };
}

/// Evaluates under `awaited` as the awaited contextual type: tsc hands such an
/// expression `X | PromiseLike<X>`, so a call returning `Promise<R>` infers `R`
/// from `X`. Only a call whose expected type is `awaited` itself reads it.
pub(crate) fn with_awaited_context<R>(awaited: &Type, evaluate: impl FnOnce() -> R) -> R {
    let outer = AWAITED_CONTEXT.with(|cell| cell.replace(Some(awaited.clone())));
    let result = evaluate();
    AWAITED_CONTEXT.with(|cell| *cell.borrow_mut() = outer);
    result
}

fn is_awaited_context(expected: &Type) -> bool {
    AWAITED_CONTEXT.with(|cell| cell.borrow().as_ref() == Some(expected))
}

/// A function has no index signature and none inferable
/// (`isObjectTypeWithInferableIndex` excludes signatures), so it cannot meet a
/// constraint's string index unless that index is `any`. Read only while an
/// overload member is chosen: a candidate the member's constraint rejects
/// sends the call on to the next member (`chooseOverload`), while a fold's
/// union parameter records the callback itself beside what it returns.
fn function_lacks_constraint_index(candidate: &Type, constraint: &Type) -> bool {
    matches!(candidate.peeled(), Type::Function(_))
        && matches!(constraint.peeled(), Type::Object(object)
            if object.call_signature().is_none()
                && !object.synthetic_open_index
                && object.string_index_type.as_deref().is_some_and(|index| !matches!(index, Type::Any)))
}

pub(super) fn lacks_constraint_call_signature(candidate: &Type, constraint: &Type) -> bool {
    let target = match constraint.peeled() {
        Type::Function(signature) => signature,
        Type::Object(object) => match object.call_signature() {
            Some(signature) => signature.clone(),
            None => return false,
        },
        _ => return false,
    };
    match candidate.peeled() {
        Type::Object(object) => {
            object.call_signature().is_none() && !object.synthetic_open_index
        }
        Type::Function(source) => {
            !target.is_variadic()
                && source.required_parameter_count() > target.parameters().len()
        }
        _ => false,
    }
}

// Generic class values may still be `any`, but their symbols retain constructor
// metadata. A callable constraint can reject them without inventing static members.
pub(super) fn construct_only_argument_type(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Type> {
    let ParsedExpression::Identifier { name, .. } = expression else {
        return None;
    };
    let symbol = symbols.get(name)?;
    if !matches!(symbol.ty, Type::Any)
        || !matches!(symbol.kind, crate::symbols::SymbolKind::Const)
        || !matches!(ctx.lookup_type_declaration(name),
            Some(TypeDeclarationInfo::Interface(info)) if info.is_class_instance)
    {
        return None;
    }
    let constructor = symbol.function_signature.as_ref()?.construct_signatures.as_ref()?.first()?;
    Some(Type::Object(
        alloc_object_type(Default::default(), None)
            .with_construct_signature(constructor.template.clone())
            .with_alias_name(format!("typeof {name}")),
    ))
}

/// An object candidate without a member the constraint requires fails it
/// whatever that member's type, so the constraint's members need not be
/// settled for the verdict. A candidate with `any` members is the stand-in
/// surge builds for a value it did not model (a class merged with a
/// namespace), which says nothing about what the value lacks.
fn lacks_required_member(candidate: &Type, constraint: &Type) -> bool {
    let (Type::Object(candidate), Type::Object(constraint)) = (candidate.peeled(), constraint) else {
        return false;
    };
    candidate.string_index_type.is_none()
        && !candidate.synthetic_open_index
        && candidate.call_signature.is_none()
        && candidate.construct_signature.is_none()
        && !candidate.properties.values().any(|property| matches!(property.ty, Type::Any))
        && constraint
            .properties
            .iter()
            .any(|(name, property)| !property.optional && !candidate.properties.contains_key(name))
}

/// An array or tuple candidate against a constraint each member of which
/// requires a property the array surface lacks (`number[]` for
/// `TArrayBuffer extends ArrayBufferLike`).
fn array_lacks_required_member(candidate: &Type, constraint: &Type) -> bool {
    if !matches!(candidate, Type::Array(_) | Type::Tuple(_)) {
        return false;
    }
    let members = match constraint.peeled() {
        Type::Union(union) => union.types().iter().map(Type::peeled).collect(),
        other => vec![other],
    };
    members.iter().all(|member| {
        matches!(member, Type::Object(object) if object.properties.iter().any(|(name, property)| {
            !property.optional && !name.starts_with('[') && candidate.get_property_access_type(name).is_none()
        }))
    })
}

/// A primitive candidate against a constraint each member of which requires
/// a property the primitive's apparent type lacks (`'x'` for `TClient extends
/// Pool | Client`). The apparent members are read off the lib interfaces; one
/// surge cannot find counts as declaring the member.
fn primitive_lacks_required_member(candidate: &Type, constraint: &Type, ctx: &CheckerContext) -> bool {
    let apparent = match candidate {
        Type::String | Type::StringLiteral(_) => "String",
        Type::Number | Type::NumberLiteral(_) => "Number",
        Type::Boolean | Type::BooleanLiteral(_) => "Boolean",
        Type::BigInt => "BigInt",
        Type::Symbol => "Symbol",
        _ => return false,
    };
    let declares = |member: &str| {
        [apparent, "Object"].iter().any(|interface| {
            lookup_declaration_for_inference(interface, ctx).is_none()
                || interface_declares_member(interface, member, ctx, 0)
        })
    };
    let members = match constraint.peeled() {
        Type::Union(union) => union.types().iter().map(Type::peeled).collect(),
        other => vec![other],
    };
    members.iter().all(|member| {
        matches!(member, Type::Object(object) if object.intersected_primitive().is_none()
            && object.properties.iter().any(|(name, property)| {
                !property.optional && !name.starts_with('[') && !declares(name)
            }))
    })
}

fn is_plain_object_reference(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    !reference.numeric_enum
        && reference.enum_owner.is_none()
        && reference.enum_base.is_none()
        && !reference.is_unique_symbol()
        && matches!(ty.peeled(), Type::Object(object) if object.intersection_operands.is_none())
}

fn candidate_is_primitive(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(candidate_is_primitive),
        _ => false,
    }
}

/// Whether a type is settled enough to stand on either side of the constraint
/// check above.
///
/// `Unknown` is surge's degradation sentinel and `GenuineUnknown` a written
/// `unknown`; neither settles assignability here, so a constraint carrying one
/// (`QueryKey = ReadonlyArray<unknown>`) must not displace an inferred
/// candidate. A lazy reference is declined rather than peeled: forcing an
/// expansion to settle a constraint would expand declarations the call never
/// reads. The walk reaches the places a degraded member hides one level down —
/// index and call/construct signatures included — which a property-only walk
/// misses.
fn constraint_operand_is_settled(ty: &Type) -> bool {
    constraint_operand_is_settled_at_depth(ty, 0)
}

fn constraint_operand_is_settled_at_depth(ty: &Type, depth: usize) -> bool {
    if depth >= 8 {
        return false;
    }
    match ty {
        Type::Unknown | Type::GenuineUnknown | Type::Reference(_) => false,
        Type::TypeParameter(parameter) => surge_ts_types::type_variable::active_constraint(parameter)
            .flatten()
            .is_some_and(|constraint| constraint_operand_is_settled_at_depth(&constraint, depth + 1)),
        Type::Array(element) => constraint_operand_is_settled_at_depth(element, depth),
        Type::Tuple(elements) => elements.iter().all(|element| constraint_operand_is_settled_at_depth(element, depth)),
        Type::Union(union) => union.types().iter().all(|member| constraint_operand_is_settled_at_depth(member, depth)),
        Type::Function(function) => constraint_signature_is_settled_at_depth(function, depth),
        Type::Object(object) => {
            object
                .properties
                .values()
                .all(|property| constraint_operand_is_settled_at_depth(&property.ty, depth))
                && object
                    .string_index_type
                    .as_deref()
                    .is_none_or(|index| constraint_operand_is_settled_at_depth(index, depth))
                && object
                    .call_signature
                    .as_deref()
                    .is_none_or(|signature| constraint_signature_is_settled_at_depth(signature, depth))
                && object
                    .construct_signature
                    .as_deref()
                    .is_none_or(|signature| constraint_signature_is_settled_at_depth(signature, depth))
        }
        _ => true,
    }
}

fn constraint_signature_is_settled_at_depth(function: &FunctionType, depth: usize) -> bool {
    function
        .parameters()
        .iter()
        .all(|parameter| constraint_operand_is_settled_at_depth(parameter, depth))
        && constraint_operand_is_settled_at_depth(function.return_type(), depth)
}

/// Validate explicit type arguments against `K extends keyof T` constraints when
/// both `T` (a concrete object) and `K` (concrete string-literal keys) are
/// resolved. Emits TS2344 for keys that are not members of `T` and reports
/// whether any constraint was violated so the caller can avoid a cascading
/// `T[K]` diagnostic. Other constraint forms are intentionally left untouched.
pub(crate) fn enforce_explicit_keyof_constraints(
    function_signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    substitution: &TypeParameterSubstitution,
    type_argument_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let mut violated = false;

    for type_parameter in &function_signature.type_parameters {
        let Some(ParsedType::KeyOf(inner)) = &type_parameter.constraint else {
            continue;
        };
        let ParsedType::Named(constraint_target) = inner.as_ref() else {
            continue;
        };

        let Some(constraint_value) = substitution.get(&constraint_target.name).cloned() else {
            continue;
        };
        // `T` may be bound to a nominal reference (`get<User, …>`); peel it to read
        // the constrained object's keys.
        let Type::Object(object_type) = constraint_value.peeled() else {
            continue;
        };
        let Some(key_type) = substitution.get(&type_parameter.name).cloned() else {
            continue;
        };
        let Some(keys) = string_literal_union_keys(&key_type) else {
            continue;
        };

        let all_keys_present = keys
            .iter()
            .all(|key| object_type.get_property_access_type(key).is_some());
        if all_keys_present {
            continue;
        }

        violated = true;
        let constraint_name = format!(
            "keyof {}",
            constraint_target_display(
                function_signature,
                type_arguments,
                &constraint_target.name,
                &object_type
            )
        );
        let mut diagnostic =
            Diagnostic::ts2344(&key_type.name(), &constraint_name, ctx.file_name.clone());
        if let Some(span) = type_argument_span {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push_utility_diagnostic_once(diagnostic);
    }

    violated
}

fn constraint_target_display(
    function_signature: &FunctionSignatureInfo,
    type_arguments: &[ParsedType],
    target_name: &str,
    resolved_object: &surge_ts_types::ObjectType,
) -> String {
    let target_argument = function_signature
        .type_parameters
        .iter()
        .position(|type_parameter| type_parameter.name == target_name)
        .and_then(|index| type_arguments.get(index));

    if let Some(ParsedType::Named(named)) = target_argument {
        return named.name.clone();
    }

    Type::Object(resolved_object.clone()).name()
}

/// What an object or array literal argument infers to against one overload
/// candidate's written parameter, read as the inference probe reads it: under
/// that parameter's literal and tuple context. `None` for any other argument.
pub(super) fn literal_argument_type_for_candidate(
    candidate: &FunctionSignatureInfo,
    index: usize,
    argument: &ParsedCallArgument,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if argument.spread
        || !matches!(argument.expression, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. })
        || candidate.rest && index + 1 >= candidate.parameter_types.len()
    {
        return None;
    }
    let parameter_type = candidate.parameter_types.get(index)?.as_ref()?;
    let top_level_return_type_parameters: Vec<&str> = candidate
        .return_type
        .as_ref()
        .map(|return_type| {
            candidate
                .type_parameters
                .iter()
                .map(|type_parameter| type_parameter.name.as_str())
                .filter(|name| type_parameter_at_top_level_in_return_type(return_type, name))
                .collect()
        })
        .unwrap_or_default();
    let diagnostics_before = ctx.diagnostics().len();
    let literal_context =
        literal_element_context(parameter_type, candidate, &top_level_return_type_parameters, None, ctx);
    let inferred = match array_literal_tuple_inference(
        parameter_type,
        &candidate.type_parameters,
        &argument.expression,
        symbols,
        ctx,
    )
    .or_else(|| {
        written_tuple_argument_inference(
            parameter_type,
            &candidate.type_parameters,
            false,
            &argument.expression,
            symbols,
            ctx,
        )
    })
    {
        Some(tuple) => Some(tuple),
        None => crate::infer::expression::with_literal_element_context(literal_context, || {
            infer_expression(&argument.expression, symbols, ctx)
        })
        .flowing_type(),
    };
    ctx.truncate_diagnostics(diagnostics_before);
    inferred
}

/// The literal kinds an argument's array elements keep while it is inferred
/// from: those a type parameter the written parameter mentions is constrained
/// to (tsc's `isLiteralOfContextualType`). `T extends string` infers
/// `"a" | "b"` from `["a", "b"]`; an unconstrained `T` infers `string`.
fn literal_element_context(
    parameter_type: &ParsedType,
    function_signature: &FunctionSignatureInfo,
    top_level_return_type_parameters: &[&str],
    expected_return_type: Option<&Type>,
    ctx: &mut CheckerContext,
) -> crate::infer::expression::LiteralElementContext {
    let mut context = crate::infer::expression::LiteralElementContext::default();
    for type_parameter in &function_signature.type_parameters {
        if !parsed_type_mentions_name(parameter_type, &type_parameter.name) {
            continue;
        }
        // The contextual return type reaches the elements through the return
        // mapper: `const c: "a" | "b" = pick(["a", "b"])` instantiates `T` to
        // the literal union, which is a literal context in its own right.
        if let Some(expected) = expected_return_type
            && top_level_return_type_parameters.contains(&type_parameter.name.as_str())
        {
            let members = match surge_ts_types::peel_to_pattern_literal(expected) {
                Type::Union(union) => union.types().to_vec(),
                other => vec![other],
            };
            for member in &members {
                match member {
                    Type::StringLiteral(_) => context.string = true,
                    Type::NumberLiteral(_) => context.number = true,
                    pattern
                        if surge_ts_types::is_template_literal_type(pattern)
                            || surge_ts_types::string_mapping_parts(pattern).is_some() =>
                    {
                        context.string = true;
                    }
                    _ => {}
                }
            }
        }
        let Some(constraint) = type_parameter.constraint.as_ref() else {
            continue;
        };
        if let Some(element) = tuple_like_constraint_element(constraint) {
            context.tuple = true;
            let element_constraint = match element {
                ParsedType::Named(named) if named.type_arguments.is_empty() => function_signature
                    .type_parameters
                    .iter()
                    .find(|parameter| parameter.name == named.name)
                    .and_then(|parameter| parameter.constraint.as_ref())
                    .unwrap_or(element),
                other => other,
            };
            match element_constraint {
                ParsedType::String => context.string = true,
                ParsedType::Number => context.number = true,
                _ => {}
            }
        }
        let diagnostics_before = ctx.diagnostics().len();
        let constraint = with_declaring_scope(function_signature, ctx, |ctx| {
            crate::infer::map_parsed_type(constraint.clone(), ctx)
        });
        ctx.truncate_diagnostics(diagnostics_before);
        let members = match surge_ts_types::peel_to_pattern_literal(&constraint) {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other],
        };
        for member in members {
            match surge_ts_types::peel_to_pattern_literal(&member) {
                Type::String | Type::StringLiteral(_) => context.string = true,
                Type::Number | Type::NumberLiteral(_) => context.number = true,
                pattern
                    if surge_ts_types::is_template_literal_type(&pattern)
                        || surge_ts_types::string_mapping_parts(&pattern).is_some() =>
                {
                    context.string = true;
                }
                _ => {}
            }
        }
    }
    context
}

fn contextual_type_names_literal(contextual: &Type, literal: &Type, depth: u8) -> bool {
    if depth > 4
        || !matches!(
            literal,
            Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_)
        )
    {
        return false;
    }
    match contextual {
        ty if ty == literal => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| contextual_type_names_literal(member, literal, depth + 1)),
        Type::Reference(reference) => reference
            .arguments
            .iter()
            .any(|argument| contextual_type_names_literal(argument, literal, depth + 1)),
        _ => false,
    }
}

pub(crate) fn infer_type_argument_substitution(
    function_signature: &FunctionSignatureInfo,
    arguments: &[ParsedCallArgument],
    outer_type_arguments: &[(String, Type)],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> TypeParameterSubstitution {
    infer_type_arguments_reading_arguments(
        function_signature,
        arguments,
        outer_type_arguments,
        expected_return_type,
        symbols,
        ctx,
    )
    .0
}

/// The substitution, and the clean type each argument was inferred from:
/// `None` where inference read none (a spread, a deferred callback, a rest
/// bound whole) or read a degraded one.
fn infer_type_arguments_reading_arguments(
    function_signature: &FunctionSignatureInfo,
    arguments: &[ParsedCallArgument],
    outer_type_arguments: &[(String, Type)],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> (TypeParameterSubstitution, Vec<Option<Type>>) {
    // A call inferred while another's argument is being walked starts from a
    // covariant, non-literal position of its own.
    let outer_walk = (
        INFERENCE_CONTRAVARIANT.replace(false),
        INFERENCE_BIVARIANT.replace(false),
        SOURCE_IS_OBJECT_OR_ARRAY_LITERAL.replace(false),
        INFERENCE_ROOT_PARAMETER.with(|cell| cell.replace(None)),
        SOURCE_IS_FRESH_LITERAL.replace(false),
        INFERRING_FROM_RETURN_TYPE.replace(false),
    );
    let mut substitution = TypeParameterSubstitution::new();
    for type_parameter in &function_signature.type_parameters {
        substitution.insert_placeholder(
            type_parameter.name.clone(),
            Type::type_parameter(&type_parameter.name),
        );
        // `getConstraintFromTypeParameter` reads `extends any` as `unknown`, and
        // an `unknown` constraint is no literal context: it has no primitive
        // kind (`hasPrimitiveConstraint`) and, as `{}`, no member whose
        // contextual type could keep a literal.
        // Nor is a written object type whose members name no literal and no
        // type parameter: `hasPrimitiveConstraint` asks for a primitive kind,
        // and a member's own contextual type (`length: number`) is no literal
        // context (`isLiteralOfContextualType`).
        let plain_object_constraint = |constraint: &ParsedType| match constraint {
            ParsedType::Object(object) => object
                .properties
                .iter()
                .map(|property| &property.ty)
                .chain(object.string_index_type.as_deref())
                .chain(object.number_index_type.as_deref())
                .all(|member| {
                    !function_signature
                        .type_parameters
                        .iter()
                        .any(|parameter| parsed_type_mentions_name(member, &parameter.name))
                        && !parsed_type_names_literal(member)
                }),
            _ => false,
        };
        if type_parameter.constraint.as_ref().is_some_and(|constraint| {
            !matches!(constraint, ParsedType::Any | ParsedType::UnknownKeyword) && !plain_object_constraint(constraint)
        }) {
            substitution.mark_keeps_literal(&type_parameter.name);
        }
    }

    let top_level_return_type_parameters: Vec<&str> = function_signature
        .return_type
        .as_ref()
        .map(|return_type| {
            function_signature
                .type_parameters
                .iter()
                .map(|type_parameter| type_parameter.name.as_str())
                .filter(|name| type_parameter_at_top_level_in_return_type(return_type, name))
                .collect()
        })
        .unwrap_or_default();

    let rest_index = function_signature
        .rest
        .then(|| function_signature.parameter_types.len().checked_sub(1))
        .flatten();
    let mut rest_tuple_bound = false;
    let mut return_mapper: Option<Option<TypeParameterSubstitution>> = None;
    let mut deferred_callbacks: Vec<DeferredCallback<'_>> = Vec::new();
    // The written parameter each argument is inferred against, `None` where
    // the declaration writes none.
    let mut argument_parameter_types: Vec<Option<&ParsedType>> = vec![None; arguments.len()];
    let mut argument_types: Vec<Option<Type>> = vec![None; arguments.len()];
    for (index, argument) in arguments.iter().enumerate() {
        let rest_target = rest_index
            .filter(|rest_index| index >= *rest_index)
            .and_then(|rest_index| function_signature.parameter_types[rest_index].as_ref())
            .and_then(|written| rest_parameter_inference_target(written, function_signature));
        let parameter_type = match rest_target {
            // `...args: E` binds `E` to the tuple of every rest argument, once.
            Some(RestInferenceTarget::Whole(parameter_type)) => {
                argument_parameter_types[index] = Some(parameter_type);
                if !rest_tuple_bound {
                    rest_tuple_bound = true;
                    if let Some(tuple) = rest_arguments_tuple(&arguments[index..], symbols, ctx) {
                        with_declaring_scope(function_signature, ctx, |ctx| {
                            with_inference_root(parameter_type, || {
                                collect_inferred_type_argument(
                                    parameter_type,
                                    &tuple,
                                    &mut substitution,
                                    false,
                                    ctx,
                                    0,
                                );
                            });
                        });
                    }
                }
                continue;
            }
            Some(RestInferenceTarget::Element(element)) => element,
            None => {
                let Some(parameter_type) = function_signature
                    .parameter_types
                    .get(index)
                    .and_then(|ty| ty.as_ref())
                    .or_else(|| {
                        rest_tuple_parameter_element(function_signature, arguments.len(), index)
                    })
                    .map(|ty| {
                        rest_tuple_parameter_element(function_signature, arguments.len(), index)
                            .filter(|_| index + 1 == function_signature.parameter_types.len())
                            .unwrap_or(ty)
                    })
                else {
                    continue;
                };
                parameter_type
            }
        };
        argument_parameter_types[index] = Some(parameter_type);

        // A callback with an un-annotated parameter is *context-sensitive*: what
        // its body — and so its return type — infers depends on the parameter
        // types this very signature gives it. Sketching it now types those
        // parameters as `any`, and a type parameter appearing only in the
        // callback's return position would be bound to `any` from the `any`
        // body. It waits for the second pass below, which types it against the
        // parameters the arguments here have already pinned.
        if let Some(callback) = context_sensitive_callback_argument(parameter_type, argument, ctx) {
            deferred_callbacks.push(DeferredCallback {
                argument_index: index,
                parameter_type,
                arrow: callback,
            });
            continue;
        }

        // This is an inference *probe*: the argument is evaluated only to infer the
        // call's type parameters, without the contextual parameter type the
        // authoritative `check_function_type_call` pass supplies afterwards. An
        // object-literal argument with a method (`{ run(ctx, input) {} }`) would
        // here type those parameters as implicit `any` and emit a spurious TS7006,
        // even though the instantiated parameter type gives them real contextual
        // types in the authoritative pass. Discard any diagnostics this probe
        // emits; the authoritative pass re-evaluates every argument and reports the
        // genuine ones.
        let diagnostics_before = ctx.diagnostics().len();
        let literal_context = literal_element_context(
            parameter_type,
            function_signature,
            &top_level_return_type_parameters,
            expected_return_type,
            ctx,
        );
        let contextual_return = callback_contextual_return_type(
            parameter_type,
            &argument.expression,
            function_signature,
            expected_return_type,
            &substitution,
            &mut return_mapper,
            ctx,
        );
        let inferred_argument = match array_literal_tuple_inference(
            parameter_type,
            &function_signature.type_parameters,
            &argument.expression,
            symbols,
            ctx,
        )
        .or_else(|| {
            written_tuple_argument_inference(
                parameter_type,
                &function_signature.type_parameters,
                expected_return_type.is_some() || CONSTRUCTED_UNDER_CONTEXT.with(std::cell::Cell::get),
                &argument.expression,
                symbols,
                ctx,
            )
        })
        .or_else(|| {
            return_mapped_tuple_argument(
                parameter_type,
                &argument.expression,
                function_signature,
                expected_return_type,
                &mut return_mapper,
                symbols,
                ctx,
            )
        }) {
            Some(tuple) => InferredExpression::Known(tuple),
            None => crate::infer::expression::with_literal_element_context(literal_context, || match contextual_return {
                Some(contextual) => crate::infer::expression::with_callback_contextual_return(contextual, || {
                    infer_expression(&argument.expression, symbols, ctx)
                }),
                None => match mapped_argument_context(parameter_type, &argument.expression, function_signature, &substitution, ctx) {
                    Some(contextual) => crate::infer::expression::with_mapped_argument_context(contextual, || {
                        infer_expression(&argument.expression, symbols, ctx)
                    }),
                    None => infer_expression(&argument.expression, symbols, ctx),
                },
            }),
        };
        ctx.truncate_diagnostics(diagnostics_before);
        let Some(mut argument_type) = inferred_argument.flowing_type() else {
            record_generic_call_inference_unresolved_argument_skip();
            continue;
        };

        if matches!(argument_type, Type::Any)
            && matches!(parameter_type, ParsedType::Named(named) if named.type_arguments.is_empty()
                && function_signature.type_parameters.iter().any(|parameter| {
                    parameter.name == named.name
                        && matches!(parameter.constraint, Some(ParsedType::Function(_)))
                }))
            && let Some(constructor) = construct_only_argument_type(&argument.expression, symbols, ctx)
        {
            argument_type = constructor;
        }
        // A leaked placeholder (`Mock<T>` off a `vi.fn()` whose `T` no scope
        // binds) infers garbage — `TData` as the mock's own call signature —
        // so the argument contributes nothing, as an unresolved one does.
        let written_unknown = matches!(argument_type, Type::GenuineUnknown)
            && !argument.spread
            && reads_a_written_type(&argument.expression);
        if (is_inference_hole(&argument_type) && !written_unknown)
            || crate::checks::expr::carries_leaked_type_parameter(&argument_type, ctx)
        {
            record_generic_call_inference_unresolved_argument_skip();
            continue;
        }

        // The argument's type is in hand; matching it against the written
        // parameter (`FetchOptions<R>`) resolves the declaring module's names,
        // so that side runs under the declaring file like instantiation does.
        // A constrained parameter keeps its literals whatever the argument is;
        // `mark_keeps_literal` recorded that above, and
        // `record_type_argument_candidate` enforces it.
        // tsc widens an inferred literal only when the type parameter does not
        // occur at the top level of the return type (`getCovariantInference`):
        // `id(1)` is `1` while `box(1)` is `{ v: number }`. A literal inside an
        // object or array literal argument has already widened at its mutable
        // location, so only a bare primitive literal is kept.
        // Elements kept as literals by their contextual type were never
        // widened at their mutable location, and tsc does not widen them here
        // either.
        // A literal the call's contextual type itself names stays a literal too
        // (tsc's `isLiteralOfContextualType`): `Promise.resolve('data')` where a
        // `'data'` result is expected infers `'data'`.
        let fresh_literal = argument_is_fresh_literal(&argument.expression)
            && literal_context == crate::infer::expression::LiteralElementContext::default()
            && !expected_return_type
                .is_some_and(|expected| contextual_type_names_literal(expected, &argument_type, 0));
        let widen_literals = fresh_literal
            && !(argument_is_primitive_literal(&argument.expression)
                && top_level_return_type_parameters
                    .iter()
                    .any(|name| type_parameter_at_top_level(parameter_type, name, 0)));
        if !argument.spread && !argument_type.is_degraded() {
            argument_types[index] = Some(argument_type.clone());
        }
        // `f(...xs)` supplies the *elements* of `xs`, each lined up with the
        // position it covers — tsc's `getSpreadArgumentType`. Matching the
        // spread's own type against the parameter bound a rest `T[]`'s `T` to
        // `number[]` rather than `number`, which then rejected every ordinary
        // argument standing beside it (`rest(...a, 5)`). A tuple contributes
        // its elements in order, so first-wins candidate order still picks the
        // first, as tsc does; anything else contributes what iterating it
        // yields, and an unknown shape contributes nothing.
        let candidates: Vec<Type> = if argument.spread {
            match argument_type.peeled() {
                Type::Tuple(elements) => elements,
                // `for_of_element_type` reads a nominal collection reference
                // (`Set<T>`, `Map<K, V>`) off its type arguments, so it is
                // handed the *unpeeled* type: the resolved object surface it
                // would otherwise see carries no element to find.
                _ => vec![crate::checks::function::for_of_element_type(&argument_type)],
            }
        } else {
            vec![argument_type]
        };
        let source_is_function_literal =
            matches!(argument.expression, ParsedExpression::ArrowFunction(_));
        let outer_literal_source = SOURCE_IS_FUNCTION_LITERAL.replace(source_is_function_literal);
        let outer_object_literal_source = SOURCE_IS_OBJECT_OR_ARRAY_LITERAL.replace(matches!(
            argument.expression,
            ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
        ));
        let outer_fresh_source = SOURCE_IS_FRESH_LITERAL.replace(fresh_literal);
        // An `unknown` anywhere in a type read off a declaration was written
        // there (`o: Obs<unknown>`), not left by a nested call's inference.
        let written_source = !argument.spread && assembles_written_types(&argument.expression);
        let outer_written_unknown = WRITTEN_UNKNOWN_ARGUMENT.replace(written_unknown || written_source);
        with_declaring_scope(function_signature, ctx, |ctx| {
            with_inference_root(parameter_type, || {
                for candidate in &candidates {
                    if is_inference_hole(candidate) {
                        continue;
                    }
                    collect_inferred_type_argument(
                        parameter_type,
                        candidate,
                        &mut substitution,
                        widen_literals,
                        ctx,
                        0,
                    );
                }
            });
        });
        WRITTEN_UNKNOWN_ARGUMENT.set(outer_written_unknown);
        SOURCE_IS_FUNCTION_LITERAL.set(outer_literal_source);
        SOURCE_IS_OBJECT_OR_ARRAY_LITERAL.set(outer_object_literal_source);
        SOURCE_IS_FRESH_LITERAL.set(outer_fresh_source);
    }

    infer_from_context_sensitive_callbacks(
        function_signature,
        &deferred_callbacks,
        &argument_parameter_types,
        outer_type_arguments,
        expected_return_type,
        &mut substitution,
        symbols,
        ctx,
    );

    with_declaring_scope(function_signature, ctx, |ctx| {
        infer_type_arguments_from_expected_return_type(
            function_signature,
            expected_return_type,
            &mut substitution,
            ctx,
        );
    });

    let unknown_inferred: Vec<std::sync::Arc<str>> = substitution
        .iter()
        .filter(|(name, _)| {
            substitution
                .inference_candidates(name)
                .iter()
                .any(|candidate| matches!(candidate.ty, Type::GenuineUnknown))
        })
        .map(|(name, _)| name.clone())
        .collect();
    UNKNOWN_INFERRED_FROM_ARGUMENTS.with(|cell| *cell.borrow_mut() = unknown_inferred);
    substitution.clear_inference_candidates();
    INFERENCE_CONTRAVARIANT.set(outer_walk.0);
    INFERENCE_BIVARIANT.set(outer_walk.1);
    SOURCE_IS_OBJECT_OR_ARRAY_LITERAL.set(outer_walk.2);
    INFERENCE_ROOT_PARAMETER.with(|cell| cell.replace(outer_walk.3));
    SOURCE_IS_FRESH_LITERAL.set(outer_walk.4);
    INFERRING_FROM_RETURN_TYPE.set(outer_walk.5);
    (substitution, argument_types)
}

/// The written function annotation a callback argument is matched against, with
/// the optionality a lib signature wraps it in removed: `then`'s parameter is
/// `((value: T) => TResult1 | PromiseLike<TResult1>) | undefined | null`, and
/// the callback is the union's only function member.
/// The callback signature a parameter spells out: written, the one function
/// member of a union, or a generic function alias's body written in the
/// reference's arguments (`ObjectIterator<Dictionary<T>, R>`).
fn callback_parameter_annotation(
    parameter_type: &ParsedType,
    ctx: &CheckerContext,
) -> Option<std::sync::Arc<surge_ts_syntax::ParsedFunctionType>> {
    match parameter_type {
        ParsedType::Function(function) => Some(function.clone()),
        ParsedType::Union(members) => {
            let mut functions = members.iter().filter_map(|member| match member {
                ParsedType::Function(function) => Some(function),
                _ => None,
            });
            let function = functions.next()?;
            functions.next().is_none().then(|| function.clone())
        }
        ParsedType::Named(_) => match signature_alias_body(parameter_type, ctx)? {
            (ParsedType::Function(function), _) => Some(function),
            _ => None,
        },
        _ => None,
    }
}

/// tsc's `inferTypeArguments` return mapper: what the call's contextual type,
/// inferred to the declared return type on its own (`InferencePriorityNone`),
/// binds. `None` when it binds nothing, as tsc leaves `returnMapper` unset.
fn return_mapper_substitution(
    function_signature: &FunctionSignatureInfo,
    expected_return_type: Option<&Type>,
    ctx: &mut CheckerContext,
) -> Option<TypeParameterSubstitution> {
    let expected_return_type = expected_return_type?;
    let declared_return_type = function_signature.return_type.as_ref()?;
    let mut mapper = TypeParameterSubstitution::new();
    for type_parameter in &function_signature.type_parameters {
        mapper.insert_placeholder(type_parameter.name.clone(), Type::type_parameter(&type_parameter.name));
    }
    let diagnostics_before = ctx.diagnostics().len();
    let outer_from_return = INFERRING_FROM_RETURN_TYPE.replace(true);
    with_declaring_scope(function_signature, ctx, |ctx| {
        collect_inferred_type_argument(declared_return_type, expected_return_type, &mut mapper, false, ctx, 0);
    });
    INFERRING_FROM_RETURN_TYPE.set(outer_from_return);
    ctx.truncate_diagnostics(diagnostics_before);
    mapper.clear_inference_candidates();
    let binds_any = function_signature
        .type_parameters
        .iter()
        .any(|type_parameter| mapper.get(&type_parameter.name).is_some_and(|bound| !bound.is_degraded()));
    binds_any.then_some(mapper)
}

/// tsc's `instantiateContextualType` under the call's `returnMapper`: an array
/// literal passed straight to a type parameter is contextually typed by what
/// the expected return type infers for that parameter, which is what makes it
/// a tuple in `const pair: [string, number] = id(['a', 1])` and
/// `const entries: readonly [string, number][] = Object.freeze([['a', 1]])`.
fn return_mapped_tuple_argument(
    parameter_type: &ParsedType,
    argument: &ParsedExpression,
    function_signature: &FunctionSignatureInfo,
    expected_return_type: Option<&Type>,
    return_mapper: &mut Option<Option<TypeParameterSubstitution>>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !matches!(argument, ParsedExpression::ArrayLiteral { .. }) {
        return None;
    }
    let ParsedType::Named(named) = parameter_type else {
        return None;
    };
    if !named.type_arguments.is_empty()
        || !function_signature
            .type_parameters
            .iter()
            .any(|type_parameter| type_parameter.name == named.name)
    {
        return None;
    }
    let mapper = return_mapper
        .get_or_insert_with(|| return_mapper_substitution(function_signature, expected_return_type, ctx))
        .as_ref()?;
    let contextual = mapper.get(&named.name).filter(|bound| !bound.is_degraded())?.clone();
    tuple_shaped_array_literal(argument, &contextual, symbols, ctx)
}

/// An array literal read against a contextual type that makes it a tuple
/// (`inTupleContext`), or a sequence of such; `None` where it does neither.
fn tuple_shaped_array_literal(
    argument: &ParsedExpression,
    contextual: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let ParsedExpression::ArrayLiteral { elements, .. } = argument else {
        return None;
    };
    if elements.is_empty() || elements.iter().any(|element| element.spread) {
        return None;
    }
    let contextual = contextual.peeled();
    if let Type::Reference(reference) = &contextual
        && reference.is_readonly_array()
    {
        return tuple_shaped_array_literal(argument, reference.arguments.first()?, symbols, ctx);
    }
    if let Some((expected_elements, _)) = surge_ts_types::fixed_tuple_parts(&contextual) {
        if elements.len() > expected_elements.len() {
            return None;
        }
        let mut element_types = Vec::with_capacity(elements.len());
        for (element, expected) in elements.iter().zip(expected_elements) {
            if let Some(nested) = tuple_shaped_array_literal(&element.expression, expected, symbols, ctx) {
                element_types.push(nested);
                continue;
            }
            let InferredExpression::Known(element_type) = infer_expression(&element.expression, symbols, ctx) else {
                return None;
            };
            if element_type.is_unknown() {
                return None;
            }
            element_types.push(if contextual_type_has_literal_kind(expected, &element_type) {
                element_type
            } else {
                crate::checks::expr::widen_type(&element_type)
            });
        }
        return Some(Type::Tuple(element_types));
    }
    let Type::Array(element_context) = &contextual else {
        return None;
    };
    let mut members = Vec::with_capacity(elements.len());
    for element in elements {
        members.push(tuple_shaped_array_literal(&element.expression, element_context, symbols, ctx)?);
    }
    Some(Type::Array(Box::new(surge_ts_types::union_type(members))))
}

/// `isLiteralOfContextualType` for a literal element: the contextual type
/// names a literal of the same kind.
fn contextual_type_has_literal_kind(contextual: &Type, literal: &Type) -> bool {
    let members = match contextual.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    members.iter().any(|member| {
        matches!(
            (member, literal),
            (Type::StringLiteral(_), Type::StringLiteral(_))
                | (Type::NumberLiteral(_), Type::NumberLiteral(_))
                | (Type::BooleanLiteral(_), Type::BooleanLiteral(_))
        )
    })
}

/// The contextual return type a non-context-sensitive arrow argument's
/// expression body is checked under (tsc's `instantiateContextualType`): the
/// callback's written return type with the inferences made so far, and the
/// return mapper's for type parameters nothing has inferred yet. A literal it
/// names keeps its type, so `const x: Promise<I> = g(() => ({ name: 'test' }))`
/// infers `{ name: 'test' }` rather than widening the name to `string`. `None`
/// where a type parameter stays open, which instantiates nothing.
/// The parameter type an `xs.map(…)` argument is passed to, its type
/// parameters still open: what decides whether the mapped elements are tuples
/// (`new Map(xs.map(x => [x.id, x]))`), whatever they infer.
fn mapped_argument_context(
    parameter_type: &ParsedType,
    argument: &ParsedExpression,
    function_signature: &FunctionSignatureInfo,
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !matches!(argument, ParsedExpression::PropertyCall { property_name, .. } if property_name == "map") {
        return None;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
        map_parsed_type_with_substitution(parameter_type.clone(), ctx, substitution)
    });
    ctx.truncate_diagnostics(diagnostics_before);
    (!resolved.is_unmodelled()).then_some(resolved)
}

fn callback_contextual_return_type(
    parameter_type: &ParsedType,
    argument: &ParsedExpression,
    function_signature: &FunctionSignatureInfo,
    expected_return_type: Option<&Type>,
    substitution: &TypeParameterSubstitution,
    return_mapper: &mut Option<Option<TypeParameterSubstitution>>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let ParsedExpression::ArrowFunction(arrow) = argument else {
        return None;
    };
    if arrow.is_async
        || arrow.is_generator
        || !arrow.type_parameters.is_empty()
        || arrow.return_type.is_some()
        || !matches!(arrow.body, surge_ts_syntax::ParsedArrowFunctionBody::Expression(_))
    {
        return None;
    }
    let callback = callback_parameter_annotation(parameter_type, ctx)?;
    let callback = &callback;
    let return_type = callback.return_type.as_ref();
    if !function_signature
        .type_parameters
        .iter()
        .any(|type_parameter| parsed_type_mentions_name(return_type, &type_parameter.name))
    {
        return None;
    }
    let mapper = return_mapper
        .get_or_insert_with(|| return_mapper_substitution(function_signature, expected_return_type, ctx))
        .as_ref()?;
    let mut contextual_substitution = substitution.clone_with_reason(TypeCopyReason::CallResolution);
    for type_parameter in &function_signature.type_parameters {
        let name = type_parameter.name.as_str();
        if contextual_substitution.get(name).is_some_and(|bound| !bound.is_degraded()) {
            continue;
        }
        if let Some(returned) = mapper.get(name).filter(|returned| !returned.is_degraded()) {
            contextual_substitution.set(name.to_string(), returned.clone(), false);
        }
    }
    let contextual = with_declaring_scope(function_signature, ctx, |ctx| {
        map_parsed_type_with_substitution(return_type.clone(), ctx, &contextual_substitution)
    });
    is_settled_contextual_type(&contextual).then_some(contextual)
}

/// A contextual type with no hole left in it, a reference's arguments
/// included, so a literal is kept only by what the call has really inferred.
fn is_settled_contextual_type(ty: &Type) -> bool {
    !contains_unresolved_hole(ty)
        && match ty {
            Type::Reference(reference) => reference.arguments.iter().all(is_settled_contextual_type),
            Type::Union(union) => union.types().iter().all(is_settled_contextual_type),
            _ => true,
        }
}

/// An arrow argument whose parameters this signature is the one to type: at
/// least one is written without an annotation, and the parameter it is passed
/// to is a callback the signature spells out. A generic arrow keeps its own
/// type parameters and is left to the ordinary path.
fn context_sensitive_callback_argument<'a>(
    parameter_type: &ParsedType,
    argument: &'a ParsedCallArgument,
    ctx: &CheckerContext,
) -> Option<&'a surge_ts_syntax::ParsedArrowFunction> {
    let callback = callback_parameter_annotation(parameter_type, ctx)?;
    let surge_ts_syntax::ParsedExpression::ArrowFunction(arrow) = &argument.expression else {
        return None;
    };
    if !arrow.type_parameters.is_empty() || arrow.parameters.is_empty() {
        return None;
    }
    if !arrow
        .parameters
        .iter()
        .any(|parameter| parameter.declared_type.is_none())
    {
        return None;
    }
    // Nothing to gain when the signature does not type the position either.
    callback
        .parameters
        .iter()
        .any(|parameter| !parameter.is_this)
        .then_some(arrow.as_ref())
}

/// The second inference pass: every deferred callback is sketched with the
/// parameter types the signature gives it — the first pass's candidates
/// substituted in, over the enclosing bindings the member was read under — and
/// the sketch then infers the type parameters the callback's return names.
fn infer_from_context_sensitive_callbacks(
    function_signature: &FunctionSignatureInfo,
    deferred_callbacks: &[DeferredCallback<'_>],
    argument_parameter_types: &[Option<&ParsedType>],
    outer_type_arguments: &[(String, Type)],
    expected_return_type: Option<&Type>,
    substitution: &mut TypeParameterSubstitution,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if deferred_callbacks.is_empty() {
        return;
    }

    let mut return_inferences: Option<TypeParameterSubstitution> = None;
    let callback_arguments: Vec<usize> = deferred_callbacks
        .iter()
        .map(|deferred| deferred.argument_index)
        .collect();
    for (position, deferred) in deferred_callbacks.iter().enumerate() {
        let (parameter_type, arrow) = (deferred.parameter_type, deferred.arrow);
        let Some(callback) = callback_parameter_annotation(parameter_type, ctx) else {
            continue;
        };
        let callback = &callback;
        infer_from_annotated_parameters(function_signature, callback, arrow, substitution, ctx);
        let mut fixing = TypeArgumentFixing {
            function_signature,
            expected_return_type,
            return_inferences: &mut return_inferences,
            callback_argument: deferred.argument_index,
            argument_parameter_types,
            later_callbacks: &callback_arguments[position + 1..],
        };
        fix_contextual_parameter_type_arguments(&mut fixing, callback, arrow, substitution, ctx);
        // The enclosing interface's own arguments (`T` of the `Box<string>` the
        // method was read off) are seeded here and here only: they type the
        // callback's parameters, but they are not this call's to infer, and the
        // caller seeds them into the real substitution after inference.
        let mut contextual_substitution =
            substitution.clone_with_reason(TypeCopyReason::CallResolution);
        seed_outer_type_arguments(&mut contextual_substitution, outer_type_arguments);
        let diagnostics_before = ctx.diagnostics().len();
        let mut contextual_parameters = with_declaring_scope(function_signature, ctx, |ctx| {
            callback
                .parameters
                .iter()
                .filter(|parameter| !parameter.is_this)
                .map(|parameter| {
                    map_parsed_type_with_substitution(
                        parameter.ty.clone(),
                        ctx,
                        &contextual_substitution,
                    )
                })
                .collect::<Vec<_>>()
        });
        // `getTypeAtPosition`: from a contextual rest parameter on, each
        // parameter takes the rest type's element at its position; a rest
        // parameter there takes the rest type itself.
        let contextual_rest = callback
            .parameters
            .iter()
            .filter(|parameter| !parameter.is_this)
            .last()
            .is_some_and(|parameter| parameter.rest);
        if contextual_rest && let Some(rest) = contextual_parameters.pop() {
            let rest_index = contextual_parameters.len();
            for (index, parameter) in arrow.parameters.iter().enumerate().skip(rest_index) {
                contextual_parameters.push(if parameter.rest && index == rest_index {
                    rest.clone()
                } else {
                    rest_parameter_element_type(&rest, index - rest_index)
                });
            }
        }
        let sketch = crate::infer::expression::infer_arrow_function_with_contextual_parameters(
            arrow,
            &contextual_parameters,
            symbols,
            ctx,
        );
        // The sketch lists a rest parameter as a plain one; its slot holds the
        // rest type, so the signature it infers from is variadic.
        let sketch = Type::Function(if arrow.parameters.last().is_some_and(|parameter| parameter.rest) {
            alloc_function_type(
                sketch.parameters().to_vec(),
                sketch.return_type().clone(),
                true,
                sketch.required_parameter_count(),
            )
        } else {
            sketch
        });
        ctx.truncate_diagnostics(diagnostics_before);
        if crate::checks::expr::carries_leaked_type_parameter(&sketch, ctx) {
            continue;
        }
        let callback_type = ParsedType::Function(callback.clone());
        with_declaring_scope(function_signature, ctx, |ctx| {
            with_inference_root(&callback_type, || {
                collect_inferred_type_argument(&callback_type, &sketch, substitution, false, ctx, 0);
            });
        });
    }
}

/// tsc's `inferFromAnnotatedParametersAndReturn`, which runs before a
/// context-sensitive callback's own parameters are typed: each annotated
/// parameter but a rest one infers to the contextual parameter type at its
/// position, and an annotated return type to the contextual return type.
fn infer_from_annotated_parameters(
    function_signature: &FunctionSignatureInfo,
    callback: &ParsedFunctionType,
    arrow: &surge_ts_syntax::ParsedArrowFunction,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) {
    let contextual: Vec<&surge_ts_syntax::ParsedFunctionTypeParameter> = callback
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .collect();
    let fixed_count = arrow.parameters.len()
        - usize::from(arrow.parameters.last().is_some_and(|parameter| parameter.rest));
    let diagnostics_before = ctx.diagnostics().len();
    let mut inferences: Vec<(ParsedType, Type)> = Vec::new();
    for (index, parameter) in arrow.parameters.iter().enumerate().take(fixed_count) {
        let Some(declared) = parameter.declared_type.as_ref() else {
            continue;
        };
        let Some(target) = contextual_parameter_type_at(&contextual, index) else {
            continue;
        };
        let source = crate::infer::map_parsed_type(declared.clone(), ctx);
        let source = if parameter.optional {
            surge_ts_types::union_type(vec![source, Type::Undefined])
        } else {
            source
        };
        inferences.push((target, source));
    }
    if let Some(declared_return) = arrow.return_type.as_ref() {
        let source = crate::infer::map_parsed_type(declared_return.clone(), ctx);
        inferences.push((callback.return_type.as_ref().clone(), source));
    }
    ctx.truncate_diagnostics(diagnostics_before);
    with_declaring_scope(function_signature, ctx, |ctx| {
        for (target, source) in &inferences {
            if source.is_degraded() {
                continue;
            }
            with_inference_root(target, || {
                collect_inferred_type_argument(target, source, substitution, false, ctx, 0);
            });
        }
    });
}

/// `getTypeAtPosition` over a written signature: the parameter at `index`,
/// or an array rest parameter's element from its own position on. A rest
/// typed by a type parameter reads an indexed access, which no inference is
/// made to.
fn contextual_parameter_type_at(
    contextual: &[&surge_ts_syntax::ParsedFunctionTypeParameter],
    index: usize,
) -> Option<ParsedType> {
    let (last, fixed) = contextual.split_last()?;
    if !last.rest {
        return contextual.get(index).map(|parameter| parameter.ty.clone());
    }
    if let Some(parameter) = fixed.get(index) {
        return Some(parameter.ty.clone());
    }
    match &last.ty {
        ParsedType::Array(element) => Some(element.as_ref().clone()),
        ParsedType::Readonly(inner) => match inner.as_ref() {
            ParsedType::Array(element) => Some(element.as_ref().clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A context-sensitive callback argument, left for the second inference pass.
struct DeferredCallback<'a> {
    argument_index: usize,
    parameter_type: &'a ParsedType,
    arrow: &'a surge_ts_syntax::ParsedArrowFunction,
}

/// What fixing a type parameter needs beyond its candidates: where its default
/// and constraint are written, the contextual return type tsc infers from
/// before any argument (`InferencePriority.ReturnType`), and what the call's
/// other arguments were inferred against — every argument the first pass
/// inferred from, and the context-sensitive ones before this callback.
struct TypeArgumentFixing<'a, 'b> {
    function_signature: &'a FunctionSignatureInfo,
    expected_return_type: Option<&'a Type>,
    return_inferences: &'b mut Option<TypeParameterSubstitution>,
    callback_argument: usize,
    argument_parameter_types: &'a [Option<&'a ParsedType>],
    later_callbacks: &'a [usize],
}

/// tsc's fixing mapper: a callback parameter written without an annotation is
/// typed from the contextual signature (`assignContextualParameterTypes`), and
/// every type parameter its contextual type names is fixed at the inference
/// made so far. A contextual rest parameter typed by a bare type parameter is
/// instantiated without fixing it (`contextuallyCheckFunctionExpressionOrObjectLiteralMethod`).
fn fix_contextual_parameter_type_arguments(
    fixing: &mut TypeArgumentFixing<'_, '_>,
    callback: &ParsedFunctionType,
    arrow: &surge_ts_syntax::ParsedArrowFunction,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) {
    let function_signature = fixing.function_signature;
    let contextual: Vec<&surge_ts_syntax::ParsedFunctionTypeParameter> = callback
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .collect();
    let Some(last) = contextual.len().checked_sub(1) else {
        return;
    };
    for (index, parameter) in arrow.parameters.iter().enumerate() {
        if parameter.declared_type.is_some() {
            continue;
        }
        let Some(contextual_parameter) = contextual
            .get(index)
            .or_else(|| contextual[last].rest.then_some(&contextual[last]))
        else {
            continue;
        };
        let bare_rest_parameter = contextual_parameter.rest
            && matches!(&contextual_parameter.ty, ParsedType::Named(named)
                if named.type_arguments.is_empty()
                    && function_signature
                        .type_parameters
                        .iter()
                        .any(|type_parameter| type_parameter.name == named.name));
        if bare_rest_parameter {
            continue;
        }
        for type_parameter in &function_signature.type_parameters {
            if parsed_type_mentions_name(&contextual_parameter.ty, &type_parameter.name) {
                fix_type_argument(fixing, substitution, &type_parameter.name, ctx);
            }
        }
    }
}

/// Fixes `name` at `getInferredType` of its candidates with `isFixed` set,
/// which widens literal candidates (`widenLiteralTypes`) unless the parameter
/// is a literal context or a candidate was inferred below the top level.
/// Without a candidate it is fixed at what the contextual return type infers
/// for it, else its default, else its constraint, else `unknown`.
fn fix_type_argument(
    fixing: &mut TypeArgumentFixing<'_, '_>,
    substitution: &mut TypeParameterSubstitution,
    name: &str,
    ctx: &mut CheckerContext,
) {
    if substitution.is_inference_fixed(name) {
        return;
    }
    let candidates = substitution.inference_candidates(name);
    let fixed = if candidates.is_empty() {
        match uninferred_type_argument(fixing, substitution, name, ctx) {
            Some(fixed) => fixed,
            None => return,
        }
    } else {
        let widen_literals = !substitution.keeps_literal(name)
            && candidates
                .iter()
                .filter(|candidate| !candidate.contravariant)
                .all(|candidate| candidate.top_level);
        inferred_type(candidates, widen_literals)
    };
    substitution.set(name.to_string(), fixed, false);
    substitution.fix_inference(name);
}

/// `getInferredType` for a type parameter no candidate names yet. Another
/// argument already inferred from whose written parameter names it would have
/// given tsc a candidate surge's walk failed to record, so such a parameter is
/// left unfixed rather than defaulted, as `apply_uninferred_type_parameter_defaults`
/// rules; an unwritten parameter, or one surge could not read, names every
/// one. A callback after this one has not been inferred from yet. surge
/// reads the contextual return type only through a generic-instantiation
/// return annotation (`infer_type_arguments_from_expected_return_type`), so
/// under a contextual type a parameter another return shape names — or an
/// inferred return type may — is left unfixed too.
fn uninferred_type_argument(
    fixing: &mut TypeArgumentFixing<'_, '_>,
    substitution: &TypeParameterSubstitution,
    name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let function_signature = fixing.function_signature;
    let named_by_another_argument = fixing
        .argument_parameter_types
        .iter()
        .enumerate()
        .any(|(index, parameter_type)| {
            index != fixing.callback_argument
                && !fixing.later_callbacks.contains(&index)
                && parameter_type.is_none_or(|parameter_type| {
                    matches!(parameter_type, ParsedType::Unknown)
                        || parsed_type_mentions_name(parameter_type, name)
                })
        });
    if named_by_another_argument {
        return None;
    }
    if let Some(expected_return_type) = fixing.expected_return_type {
        let return_inferences = fixing.return_inferences.get_or_insert_with(|| {
            let mut from_return = substitution.clone_with_reason(TypeCopyReason::CallResolution);
            if let Some(ParsedType::Named(declared_return_type)) =
                function_signature.return_type.as_ref()
                && !declared_return_type.type_arguments.is_empty()
            {
                with_declaring_scope(function_signature, ctx, |ctx| {
                    infer_through_generic_reference(
                        declared_return_type,
                        expected_return_type,
                        &mut from_return,
                        false,
                        ctx,
                        0,
                    );
                });
            }
            from_return
        });
        if let Some(inferred) = return_inferences.get(name)
            && !inferred.is_degraded()
        {
            return Some(inferred.clone());
        }
        if function_signature
            .return_type
            .as_ref()
            .is_none_or(|return_type| parsed_type_mentions_name(return_type, name))
        {
            return None;
        }
    }
    let type_parameter = function_signature
        .type_parameters
        .iter()
        .find(|type_parameter| type_parameter.name == name)?;
    // `getConstraintFromTypeParameter` reads `extends any` as `unknown`.
    let written = type_parameter.default_type.clone().or_else(|| {
        type_parameter
            .constraint
            .clone()
            .filter(|constraint| !matches!(constraint, ParsedType::Any))
    });
    let Some(written) = written else {
        return Some(Type::GenuineUnknown);
    };
    let snapshot = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    let diagnostics_before = ctx.diagnostics().len();
    let resolved = with_declaring_scope(function_signature, ctx, |ctx| {
        map_parsed_type_with_substitution(written, ctx, &snapshot)
    });
    ctx.truncate_diagnostics(diagnostics_before);
    (!contains_unresolved_hole(&resolved)).then_some(resolved)
}

/// tsc infers the *widened* type from a literal expression — `behaviorSubject(1)`
/// is `BehaviorSubject<number>`, so a later `value.next(2)` is fine. The literal
/// survives only when it is not fresh (`{ n: 1 } as const`, an annotated
/// binding) or when the parameter's constraint asks for one, so this is limited
/// to a literal written at the call site inferring a bare type parameter.
/// tsc's `isTypeParameterAtTopLevelInReturnType`: the return type (or a type
/// predicate's type) is the parameter itself, a union or intersection with it
/// as a member, or a conditional whose branch is, up to three levels deep.
fn type_parameter_at_top_level_in_return_type(return_type: &ParsedType, name: &str) -> bool {
    match return_type {
        ParsedType::Predicate(predicate) => predicate
            .ty
            .as_ref()
            .is_some_and(|ty| type_parameter_at_top_level(ty, name, 0)),
        other => type_parameter_at_top_level(other, name, 0),
    }
}

fn type_parameter_at_top_level(ty: &ParsedType, name: &str, depth: usize) -> bool {
    match ty {
        ParsedType::Named(named) => named.type_arguments.is_empty() && named.name == name,
        ParsedType::Union(members) | ParsedType::Intersection(members) => members
            .iter()
            .any(|member| type_parameter_at_top_level(member, name, depth)),
        // In the true branch of `T extends X ? T : …` the parameter is a
        // substitution type carrying the implied constraint, not the parameter
        // itself (`getImpliedConstraint`), so only the false branch counts there.
        ParsedType::Conditional(conditional) if depth < 3 => {
            (!conditional_implies_constraint(
                &conditional.check_type,
                &conditional.extends_type,
                name,
            ) && type_parameter_at_top_level(&conditional.true_type, name, depth + 1))
                || type_parameter_at_top_level(&conditional.false_type, name, depth + 1)
        }
        _ => false,
    }
}

fn conditional_implies_constraint(check: &ParsedType, extends: &ParsedType, name: &str) -> bool {
    match (check, extends) {
        (ParsedType::Tuple(check), ParsedType::Tuple(extends))
            if check.len() == 1 && extends.len() == 1 =>
        {
            conditional_implies_constraint(&check[0], &extends[0], name)
        }
        (ParsedType::Named(named), _) => named.type_arguments.is_empty() && named.name == name,
        _ => false,
    }
}

fn argument_is_primitive_literal(argument: &surge_ts_syntax::ParsedExpression) -> bool {
    matches!(
        argument,
        surge_ts_syntax::ParsedExpression::StringLiteral(_)
            | surge_ts_syntax::ParsedExpression::NumberLiteral(_)
            | surge_ts_syntax::ParsedExpression::BooleanLiteral(_)
    )
}

fn argument_is_fresh_literal(argument: &surge_ts_syntax::ParsedExpression) -> bool {
    // A fresh literal: a primitive literal, or an object/array literal whose
    // property and element literals widen the same way (`subject({ n: 1 })`
    // binds `T` to `{ n: number }`). A `const` assertion is not fresh.
    //
    // Freshness is a property of the *argument*, not of where the parameter it
    // binds sits, so this decision is taken once and carried through every
    // nested position. Deciding per position — widening whatever an array
    // element, tuple slot or member walk reached — widened the literal members
    // of a *declared* type, and a `[ChunkIndex, 0, …] | [ChunkIndex, 1, …]`
    // argument bound its type parameter to `[ChunkIndex, number, …]`, which no
    // longer matched the declaration it came from.
    matches!(
        argument,
        surge_ts_syntax::ParsedExpression::StringLiteral(_)
            | surge_ts_syntax::ParsedExpression::NumberLiteral(_)
            | surge_ts_syntax::ParsedExpression::BooleanLiteral(_)
            | surge_ts_syntax::ParsedExpression::ObjectLiteral { .. }
            | surge_ts_syntax::ParsedExpression::ArrayLiteral { .. }
    )
}

/// What a tuple-shaped constraint says its elements are. `[A, ...A[]] | []` —
/// zod's `tuple` and every other "one or more, or none" signature — describes
/// the elements in its non-empty member; the `[]` member only says the argument
/// may also be empty. Without looking through the union the parameter stayed
/// unsolved and landed on that empty tuple, so every well-formed argument
/// reported `Type '[…]' is not assignable to type '[]'`.
/// The element type of a tuple-shaped constraint (`[U, ...U[]]`,
/// `Readonly<[U, ...U[]]>`), the shape that gives an array literal a tuple
/// context; a plain array constraint does not.
fn tuple_like_constraint_element(constraint: &ParsedType) -> Option<&ParsedType> {
    match constraint {
        ParsedType::Named(named) if named.name == "Readonly" && named.type_arguments.len() == 1 => {
            tuple_like_constraint_element(&named.type_arguments[0])
        }
        ParsedType::Readonly(inner) => tuple_like_constraint_element(inner),
        ParsedType::Tuple(_) | ParsedType::VariadicTuple(_) => tuple_element_constraint(constraint),
        _ => None,
    }
}

fn tuple_element_constraint(constraint: &ParsedType) -> Option<&ParsedType> {
    match constraint {
        ParsedType::Array(element) => Some(element.as_ref()),
        ParsedType::Tuple(elements) => elements.first(),
        ParsedType::VariadicTuple(elements) => elements.iter().find_map(|element| match element {
            surge_ts_syntax::ParsedTupleElement::Fixed(element) => Some(element),
            surge_ts_syntax::ParsedTupleElement::Rest(ParsedType::Array(element), _) => Some(element.as_ref()),
            surge_ts_syntax::ParsedTupleElement::Rest(..) => None,
        }),
        ParsedType::Union(members) => members.iter().find_map(tuple_element_constraint),
        _ => None,
    }
}

/// The argument type an array literal takes when the *written* parameter gives
/// it a tuple context, which is tsc's `checkArrayLiteral` rule: `inTupleContext`
/// is set whenever the contextual type is tuple-like, and the literal is then
/// typed positionally instead of widening to an array of its element union.
///
/// [`array_literal_tuple_inference`] covers only the parameter written as a
/// bare type parameter constrained to a tuple (`<T extends readonly unknown[]>(x: T)`).
/// A parameter written *as* the tuple — `x: readonly [string, T]`, or a sequence
/// of them (`(readonly [string, T])[]`, `Iterable<readonly [string, T]>`, which
/// is `Object.fromEntries`) — got no tuple context at all, so `[['a', 1]]`
/// widened to `(string | number)[][]` and `T` was never inferred.
thread_local! {
    /// A `new` whose contextual type its overload selection does not take
    /// (`const s: ReadonlyMap<"a", number> = new Map([["a", 1]])`): tsc's
    /// return mapper still gives its argument literals a literal context.
    static CONSTRUCTED_UNDER_CONTEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `run` for a `new` whose contextual type is `context`. Only a context
/// naming literal types lends its arguments a literal context through the
/// return mapper; any other leaves them widened as without one.
pub(crate) fn with_constructed_under_context<R>(context: Option<&Type>, run: impl FnOnce() -> R) -> R {
    let under_context = context.is_some_and(|context| mentions_literal_types(context, 3));
    let previous = CONSTRUCTED_UNDER_CONTEXT.with(|cell| cell.replace(under_context));
    let result = run();
    CONSTRUCTED_UNDER_CONTEXT.with(|cell| cell.set(previous));
    result
}

fn mentions_literal_types(ty: &Type, depth: usize) -> bool {
    match ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => true,
        _ if depth == 0 => false,
        Type::Union(union) => union.types().iter().any(|member| mentions_literal_types(member, depth)),
        Type::Reference(reference) if reference.enum_owner.is_some() => true,
        Type::Reference(reference) => {
            reference.arguments.iter().any(|argument| mentions_literal_types(argument, depth - 1))
        }
        Type::Object(object) => object
            .properties
            .values()
            .any(|property| mentions_literal_types(&property.ty, depth - 1)),
        _ => false,
    }
}

/// `contextual_return`: the call has a contextual return type, through which
/// tsc's return mapper hands the elements a literal context of its own
/// (`const s: ReadonlyMap<"a" | "b", number> = new Map([["a", 1]])`), so they
/// keep their literals.
fn written_tuple_argument_inference(
    parameter_type: &ParsedType,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    contextual_return: bool,
    argument: &surge_ts_syntax::ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    fn is_sequence_name(name: &str) -> bool {
        matches!(
            name,
            "Array" | "ReadonlyArray" | "Iterable" | "IterableIterator" | "ArrayLike"
        )
    }

    let surge_ts_syntax::ParsedExpression::ArrayLiteral { elements, .. } = argument else {
        return None;
    };
    // A spread contributes an unknown number of positions, so the literal has no
    // tuple shape to read off it.
    if elements.is_empty() || elements.iter().any(|element| element.spread) {
        return None;
    }

    match parameter_type {
        ParsedType::Readonly(inner) => {
            written_tuple_argument_inference(inner, type_parameters, contextual_return, argument, symbols, ctx)
        }
        // The contextual type of an optional parameter (`entries?: readonly
        // (readonly [K, V])[] | null`) is tuple-like through its members.
        ParsedType::Union(members) => members
            .iter()
            .filter(|member| !matches!(member, ParsedType::Null | ParsedType::Undefined))
            .find_map(|member| {
                written_tuple_argument_inference(member, type_parameters, contextual_return, argument, symbols, ctx)
            }),
        ParsedType::Tuple(expected_elements) => {
            let mut element_types = Vec::with_capacity(elements.len());
            for (index, element) in elements.iter().enumerate() {
                let InferredExpression::Known(element_type) =
                    infer_expression(&element.expression, symbols, ctx)
                else {
                    return None;
                };
                if element_type.is_unknown() {
                    return None;
                }
                // `checkExpressionForMutableLocation`: an element keeps its
                // literal only where its contextual type is a literal or a
                // type parameter constrained to a primitive
                // (`isLiteralOfContextualType`).
                let keeps_literal = contextual_return
                    || expected_elements
                        .get(index)
                        .is_some_and(|expected| literal_of_written_context(expected, type_parameters));
                element_types.push(if keeps_literal {
                    element_type
                } else {
                    crate::checks::expr::widen_type(&element_type)
                });
            }
            Some(Type::Tuple(element_types))
        }
        ParsedType::Array(inner) => {
            sequence_of_tuples(inner, type_parameters, contextual_return, elements, symbols, ctx)
        }
        ParsedType::Named(named)
            if is_sequence_name(&named.name) && named.type_arguments.len() == 1 =>
        {
            sequence_of_tuples(&named.type_arguments[0], type_parameters, contextual_return, elements, symbols, ctx)
        }
        _ => None,
    }
}

/// Whether a written contextual type keeps a literal assigned to it: a
/// literal, or a type parameter constrained to a primitive or literal.
fn literal_of_written_context(written: &ParsedType, type_parameters: &[surge_ts_syntax::ParsedTypeParameter]) -> bool {
    match written {
        ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_)
        | ParsedType::KeyOf(_)
        | ParsedType::TemplateLiteral(_) => true,
        ParsedType::Union(members) => members.iter().any(|member| literal_of_written_context(member, type_parameters)),
        ParsedType::Named(named) if named.type_arguments.is_empty() => {
            match type_parameters.iter().find(|parameter| parameter.name == named.name) {
                Some(parameter) => parameter.constraint.as_ref().is_some_and(|constraint| {
                    matches!(constraint, ParsedType::String | ParsedType::Number | ParsedType::Boolean)
                        || literal_of_written_context(constraint, type_parameters)
                }),
                // A named type that is no parameter of the call: its literal
                // kinds are unknown here, so the literal is kept as before.
                None => true,
            }
        }
        _ => false,
    }
}

/// Each element of the literal typed against the sequence's element type, which
/// must itself be tuple-shaped for this to contribute anything.
fn sequence_of_tuples(
    element_parameter: &ParsedType,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    contextual_return: bool,
    elements: &[surge_ts_syntax::ParsedArrayElement],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let mut members = Vec::with_capacity(elements.len());
    for element in elements.iter() {
        members.push(written_tuple_argument_inference(
            element_parameter,
            type_parameters,
            contextual_return,
            &element.expression,
            symbols,
            ctx,
        )?);
    }
    Some(Type::Array(Box::new(surge_ts_types::union_type(members))))
}

/// tsc infers an array-literal argument as a *tuple* when the inference target
/// is a tuple-shaped type parameter, and keeps its element literal types when
/// that tuple's element type is itself a parameter constrained to a primitive.
/// Both halves are load-bearing for the `[T, ...T[]]` enum-builder idiom
/// (`arrayToEnum(["a", "b"])` must yield `{ a: "a"; b: "b" }`, not
/// `{ [k: string]: string }`); array inference alone collapses the member
/// literals and every `switch (issue.code)` narrowing built on them.
pub(crate) fn array_literal_tuple_inference(
    parameter_type: &ParsedType,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    argument: &surge_ts_syntax::ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let surge_ts_syntax::ParsedExpression::ArrayLiteral { elements, .. } = argument else {
        return None;
    };
    if elements.is_empty() {
        return None;
    }
    let ParsedType::Named(named) = parameter_type else {
        return None;
    };
    if !named.type_arguments.is_empty() {
        return None;
    }
    let type_parameter = type_parameters
        .iter()
        .find(|type_parameter| type_parameter.name == named.name)?;
    // A `const` type parameter infers its argument in a const context: an array
    // literal is a tuple of its elements' own types, whatever the constraint
    // spells (zod's `input<const Items extends util.TupleItems>([schema])`).
    let keep_literals = if type_parameter.is_const {
        if elements.iter().any(|element| element.spread) {
            return None;
        }
        true
    } else {
        let constraint = type_parameter.constraint.as_ref()?;
        let element_constraint = tuple_element_constraint(constraint)?;
        matches!(
            element_constraint,
            ParsedType::String | ParsedType::Number | ParsedType::Boolean
        ) || matches!(
        element_constraint,
        ParsedType::Named(element_named)
            if type_parameters.iter().any(|type_parameter| {
                type_parameter.name == element_named.name
                    && matches!(
                        type_parameter.constraint,
                        Some(ParsedType::String | ParsedType::Number | ParsedType::Boolean)
                    )
            })
        )
    };

    let mut element_types = Vec::with_capacity(elements.len());
    for element in elements {
        let InferredExpression::Known(element_type) =
            infer_expression(&element.expression, symbols, ctx)
        else {
            return None;
        };
        if element_type.is_unknown() {
            return None;
        }
        element_types.push(if keep_literals {
            element_type
        } else {
            crate::checks::expr::widen_type(&element_type)
        });
    }
    Some(Type::Tuple(element_types))
}

/// A type parameter that occurs only in the return type (`$constructor<T>`,
/// `Ctor<T>`) is inferable solely from the call's contextual type — no argument
/// mentions it, so it would otherwise stay `Type::Unknown` and degrade every
/// callback parameter typed by it. Runs after the argument loop so an
/// argument-derived candidate always wins, and only for a generic-instantiation
/// return annotation, where the match is positional against the expected
/// reference's type arguments rather than a whole-type guess.
fn infer_type_arguments_from_expected_return_type(
    function_signature: &FunctionSignatureInfo,
    expected_return_type: Option<&Type>,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) {
    let Some(expected_return_type) = expected_return_type else {
        return;
    };
    // The expected return type is a lower-priority source than the arguments:
    // it fills in what they left open and never overrides what they fixed.
    // A conditional return (`Promise<MappedResponseType<R, T>>`) matched
    // against a contextual type is otherwise free to re-bind `R`.
    let unresolved: Vec<String> = substitution
        .iter()
        .filter(|(_, candidate)| candidate.is_degraded())
        .map(|(name, _)| name.to_string())
        .collect();
    if unresolved.is_empty() {
        return;
    }
    let Some(declared_return_type) = function_signature.return_type.as_ref() else {
        return;
    };
    let mut from_return = substitution.clone_with_reason(TypeCopyReason::CallResolution);
    let outer_from_return = INFERRING_FROM_RETURN_TYPE.replace(true);
    match declared_return_type {
        ParsedType::Named(named) => {
            let awaited = match named.type_arguments.as_slice() {
                [awaited_return_type]
                    if matches!(named.name.as_str(), "Promise" | "PromiseLike")
                        && is_awaited_context(expected_return_type)
                        && crate::checks::call::awaited_type(expected_return_type) == *expected_return_type =>
                {
                    Some(awaited_return_type)
                }
                _ => None,
            };
            if let Some(awaited_return_type) = awaited {
                // `X | PromiseLike<X>` against `Promise<R>`: the `PromiseLike`
                // member matches, so `R` is inferred from `X`.
                collect_inferred_type_argument(awaited_return_type, expected_return_type, &mut from_return, false, ctx, 0);
            } else if named.type_arguments.is_empty() {
                // A bare type parameter return (`absorb<T>(): T`) is itself the
                // inference target `inferTypes` is handed.
                if unresolved.iter().any(|name| *name == named.name) {
                    collect_inferred_type_argument(declared_return_type, expected_return_type, &mut from_return, false, ctx, 0);
                }
            } else {
                infer_through_generic_reference(named, expected_return_type, &mut from_return, false, ctx, 0);
            }
        }
        // `querySelector<E>(s): E | null` under `Rect | null`: the matching
        // `null` members drop out and `E` is inferred from what remains.
        ParsedType::Array(_) | ParsedType::Tuple(_) | ParsedType::Union(_) => {
            collect_inferred_type_argument(declared_return_type, expected_return_type, &mut from_return, false, ctx, 0);
        }
        _ => {}
    }
    INFERRING_FROM_RETURN_TYPE.set(outer_from_return);
    for name in unresolved {
        // The enclosing body's own type variable is an inference like any
        // other (`raw<T>()` returned under `Promise<T[]>`).
        if let Some(candidate) = from_return.get(&name)
            && (!candidate.is_degraded() || candidate.is_type_variable())
        {
            substitution.set(name, candidate.clone(), false);
        }
    }
}

pub(crate) fn collect_inferred_type_argument(
    parameter_type: &ParsedType,
    argument_type: &Type,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    if std::env::var("SURGE_DBG_INFER").is_ok() {
        eprintln!(
            "DBG collect d={depth} param={parameter_type:?}\n    arg={}",
            format!("{argument_type:?}")
                .chars()
                .take(300)
                .collect::<String>()
        );
    }
    // A degraded *part* of an argument does not discard the whole: a callback
    // whose body the sketch cannot type (a block with no `return`, or a call
    // surge does not model) still proves what its parameters are, and an object
    // literal still proves its other members. Only the recording of a candidate
    // checks the shape it is about to bind, so the sentinel never reaches a
    // substitution.
    // tsc's error type is an `any` source: `inferFromTypes` still hands it to a
    // naked type parameter, so `query(() => missing.member)` binds `$Output`.
    if is_inference_hole(argument_type) {
        return;
    }
    // tsc's `inferFromTypes`: a union source infers into a structural target
    // from each of its constituents (`[["", true], ["", 0]]` into
    // `readonly [K, V]` gives `V` both candidates).
    if matches!(
        parameter_type,
        ParsedType::Object(_)
            | ParsedType::Tuple(_)
            | ParsedType::Array(_)
            | ParsedType::Function(_)
            | ParsedType::Readonly(_)
    ) && let Type::Union(union) = argument_type.peeled()
    {
        for member in union.types() {
            collect_inferred_type_argument(parameter_type, member, substitution, widen_literals, ctx, depth);
        }
        return;
    }

    match parameter_type {
        ParsedType::Named(named_type) => {
            record_type_argument_candidate(
                substitution,
                &named_type.name,
                argument_type,
                widen_literals,
            );
            // `Array<T>` / `ReadonlyArray<T>` spelled as a reference infers the
            // same way as the `T[]` shorthand: element-wise, against an array or
            // tuple argument. Falling through to the member walk below instead
            // inferred nothing, because an array argument is `Type::Array` and
            // has no object surface to match the interface's members against —
            // so `addToStart(items, item)` against
            // `addToStart<T>(items: Array<T>, item: T)` saw only the second
            // argument and instantiated `T` as the literal `4`, then rejected
            // `number[]` against `4[]`.
            // The same element-wise inference serves every lib interface whose
            // FIRST type argument is the element an array satisfies it with.
            // `ArrayLike<T>` and `ConcatArray<T>` were held out of the set while
            // an array was not assignable to either — inferring through them
            // turned a silent call into a false `TS2345`. The array surface now
            // answers `length` and `[Symbol.iterator]`, so the rejection is
            // gone and they belong here with the rest.
            // `Object.fromEntries(entries: Iterable<readonly [PropertyKey, T]>)`
            // is handed a `[string, V][]`, and without this `T` is never
            // inferred, the call falls to the `any`-returning overload, and the
            // index signature tsc gives the result — with every `TS4111` that
            // depends on it — disappears. `Type::Array` has no object surface,
            // so the member walk below cannot do it.
            if !named_type.type_arguments.is_empty() && infers_element_wise(&named_type.name) {
                let element_type = &named_type.type_arguments[0];
                // Elements are never widened here. A *fresh* array literal has
                // already widened its own elements in `infer_array_literal`
                // (`take([1, 2, 3])` arrives as `number[]`), so there is nothing
                // left to widen; an argument whose type was **declared** —
                // `ReadonlyArray<Checked>` off a `as const` table, or a
                // `const x: ['a', 'b']` — keeps its literals, as tsc does.
                // Widening unconditionally bound `T` to `string` for a declared
                // literal union, and every `indexOf` on the result was then a
                // false `TS2345`.
                let argument_type = match argument_type {
                    // A `readonly` array or tuple carries its shape inside the
                    // readonly reference; `as const` produces exactly that, and
                    // `ReadonlyArray<T>` is what such an argument is usually
                    // handed to.
                    Type::Reference(reference) if reference.is_readonly_array() => {
                        reference.resolve()
                    }
                    // Any other reference that *is* an array — a written
                    // `ReadonlyArray<Checked>` parameter, an alias for `T[]` —
                    // carries the shape behind the reference. Without peeling,
                    // the element-wise walk below matched nothing and the
                    // parameter was left to the member walk, which reads the
                    // array's own surface and bound `T` to `string`.
                    Type::Reference(_) => argument_type.peeled(),
                    other => other.clone(),
                };
                let widen_elements = false;
                inferring_from_type_arguments(|| match &argument_type {
                    Type::Array(actual_element_type) => {
                        collect_inferred_type_argument(
                            element_type,
                            actual_element_type.as_ref(),
                            substitution,
                            widen_elements,
                            ctx,
                            depth,
                        );
                    }
                    // A tuple's element type is the union of its elements, and
                    // that union is one candidate: recording the elements one
                    // by one makes two literal candidates collapse to their
                    // common primitive, so `as const` would infer `string`
                    // where tsc infers `"a" | "b"`. Widening is still per the
                    // const context.
                    Type::Tuple(elements) if !widen_elements => {
                        collect_inferred_type_argument(
                            element_type,
                            &surge_ts_types::union_type(elements.clone()),
                            substitution,
                            false,
                            ctx,
                            depth,
                        );
                    }
                    Type::Tuple(elements) => {
                        for element in elements {
                            collect_inferred_type_argument(
                                element_type,
                                element,
                                substitution,
                                widen_elements,
                                ctx,
                                depth,
                            );
                        }
                    }
                    // A string's apparent `String` interface is an
                    // `ArrayLike<string>` and an `Iterable<string>`.
                    Type::String | Type::StringLiteral(_)
                        if matches!(named_type.name.as_str(), "ArrayLike" | "Iterable") =>
                    {
                        collect_inferred_type_argument(
                            element_type,
                            &Type::String,
                            substitution,
                            widen_elements,
                            ctx,
                            depth,
                        );
                    }
                    _ => {}
                });
            }

            // surge models a resolved `Promise<T>` as its awaited `T`, so a
            // written `Promise<TData>` is handed the awaited value itself —
            // `fn: (v: string) => Promise<TData>` against a callback returning
            // `string` — and the generic-reference walk below, which matches
            // member for member, infers nothing from a primitive. Infer the
            // argument against the awaited actual, which is the actual itself
            // when it is already awaited.
            // A namespace's own `Promise` is an ordinary generic interface: it
            // is matched by its type arguments like any other reference.
            let namespaced_promise = matches!(
                argument_type,
                Type::Reference(reference)
                    if reference.display.split('<').next().is_some_and(|base| {
                        base.rsplit_once('.').is_some_and(|(_, name)| {
                            matches!(name, "Promise" | "PromiseLike")
                        })
                    })
            );
            if named_type.type_arguments.len() == 1
                && matches!(named_type.name.as_str(), "Promise" | "PromiseLike")
                && !namespaced_promise
            {
                let awaited = crate::checks::call::promise_like_awaited_type(argument_type);
                // A promise kept as the interface it is can reach here as what
                // names it (`ReturnType<() => Promise<string>>`) or as its
                // expansion (a resolved member's `Promise<…>` return); tsc infers
                // from the reference's own argument, which is what `then` hands
                // its callback. That is the whole inference: the lib
                // declaration's members name only that argument, and walking them
                // below would bind their own `T` to whatever the call calls `T`.
                let promised = (awaited == *argument_type
                    && crate::checks::call::promise_nominal_enabled())
                .then(|| crate::checks::call::thenable_awaited_type(argument_type))
                .flatten();
                let nominal_promise = promised.is_some()
                    || (awaited != *argument_type
                        && crate::checks::call::promise_nominal_enabled());
                collect_inferred_type_argument(
                    &named_type.type_arguments[0],
                    promised.as_ref().unwrap_or(&awaited),
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
                if nominal_promise {
                    return;
                }
            }

            // A generic-instantiation parameter (`Wrapper<T>`,
            // `SignalDefinition<TPayload>`) carries its type parameters inside the
            // declaration's members, not at the surface — match the argument
            // against that shape so `T` is inferred from the nested field.
            if !named_type.type_arguments.is_empty() {
                infer_through_generic_reference(
                    named_type,
                    argument_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
            }
        }
        ParsedType::Mapped(mapped)
            if mapped.name_type.is_none()
                && matches!(mapped.value_type.as_ref(), ParsedType::IndexedAccess(access)
                    if matches!(access.index_type.as_ref(), ParsedType::Named(index)
                        if index.name == mapped.key_name)) =>
        {
            // `inferTypeForHomomorphicMappedType`: an array or tuple read through
            // `{ [P in keyof T]: T[P] }` (`Readonly<T>`) reverse-maps to itself,
            // its readonly-ness included, and that is what `T` infers.
            if is_identity_homomorphic_mapped(mapped)
                && let ParsedType::KeyOf(variable) = mapped.constraint.as_ref()
                && (matches!(argument_type.peeled(), Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
                    || matches!(argument_type, Type::Reference(reference) if reference.is_readonly_array()))
            {
                collect_inferred_type_argument(variable, argument_type, substitution, widen_literals, ctx, depth);
                return;
            }
            if let Some(keys) = surge_ts_types::type_variable::literal_keys_of(argument_type) {
                collect_inferred_type_argument(
                    mapped.constraint.as_ref(),
                    &keys,
                    substitution,
                    false,
                    ctx,
                    depth,
                );
            }
        }
        ParsedType::Array(element_type) => match &argument_type.peeled() {
            Type::Array(actual_element_type) => inferring_from_type_arguments(|| {
                collect_inferred_type_argument(
                    element_type.as_ref(),
                    actual_element_type.as_ref(),
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
            }),
            Type::Tuple(elements) => {
                for element in elements {
                    collect_inferred_type_argument(
                        element_type.as_ref(),
                        element,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                }
            }
            _ => {}
        },
        // `readonly [string, T]` / `readonly T[]`: the modifier is not part of
        // what inference reads — tsc's `inferFromTypes` sees a tuple type
        // reference whose readonly-ness is a flag on the target, and infers
        // element-wise regardless. Without this arm the whole annotation was
        // skipped, so `firstValue<T>(entries: Iterable<readonly [string, T]>)`
        // handed a `[string, number][]` inferred nothing and `T` stayed
        // `unknown`.
        ParsedType::Readonly(inner) => {
            // A readonly array or tuple argument carries its shape inside the
            // readonly reference, and its elements keep their literal types:
            // the const context is what the assertion was written for.
            let (argument_type, widen) = match argument_type {
                Type::Reference(reference) if reference.is_readonly_array() => {
                    (reference.resolve(), false)
                }
                other => (other.clone(), widen_literals),
            };
            collect_inferred_type_argument(
                inner,
                &argument_type,
                substitution,
                widen,
                ctx,
                depth,
            );
        }
        ParsedType::Tuple(expected_elements) => {
            // Under `exactOptionalPropertyTypes` an optional element's type is
            // the written one, as `resolve_tuple_type` resolves it.
            let exact_optional =
                surge_ts_types::strict_null_checks() && surge_ts_types::exact_optional_property_types();
            if let Some((actual_elements, _)) = surge_ts_types::fixed_tuple_parts(argument_type)
                && expected_elements.len() == actual_elements.len()
            {
                for (expected_element, actual_element) in
                    expected_elements.iter().zip(actual_elements.iter())
                {
                    let expected_element = match expected_element {
                        ParsedType::Union(members)
                            if exact_optional && members.len() == 2 && matches!(members[0], ParsedType::Undefined) =>
                        {
                            &members[1]
                        }
                        other => other,
                    };
                    collect_inferred_type_argument(
                        expected_element,
                        actual_element,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                }
            }
        }
        ParsedType::Object(expected_object_type) => {
            // A reference argument (`Box<number>`, a class instance) is
            // inferred from as the structure it expands to.
            if let Type::Object(actual_object_type) = argument_type.peeled() {
                collect_object_type_candidates(
                    expected_object_type,
                    &actual_object_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
                infer_from_index_types(
                    [
                        expected_object_type.string_index_type.as_deref(),
                        expected_object_type.number_index_type.as_deref(),
                    ],
                    &actual_object_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
            }
            // `inferFromSignatures` over construct signatures: `cb: new (a: T)
            // => U` handed a constructor infers through its signature.
            if let Some(expected_construct) = expected_object_type.construct_signature.as_deref()
                && let Type::Object(actual_object) = argument_type.peeled()
                && let Some(actual_construct) = actual_object.construct_signature()
            {
                collect_signature_inferences(
                    expected_construct,
                    actual_construct.clone(),
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
            }
        }
        // A callback parameter (`factory: () => T`, `initialState: () => S`):
        // match the argument's own signature so the type parameter is inferred
        // from what the callback takes and returns.
        ParsedType::Function(expected_function) => {
            // A callable object (vitest's `Mock<…>`, an interface with a call
            // signature) infers through its signature the way a function does.
            let actual_function = match argument_type.peeled() {
                Type::Function(function) => function,
                Type::Object(object) => match object.call_signature() {
                    Some(signature) => signature.clone(),
                    None => return,
                },
                _ => return,
            };
            collect_signature_inferences(
                expected_function,
                actual_function,
                substitution,
                widen_literals,
                ctx,
                depth,
            );
        }
        // tsc's `inferToConditionalType`: a source that is not itself a
        // conditional infers into both branches (`inferToMultipleTypes` over the
        // true and false types). tRPC's `create(opts?: ValidateShape<TOptions,
        // …>)` reaches `TOptions` only through the inner conditional's true
        // branch.
        ParsedType::Conditional(conditional) if depth < 8 => {
            for branch in [&conditional.true_type, &conditional.false_type] {
                collect_inferred_type_argument(
                    branch,
                    argument_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth + 1,
                );
            }
        }
        ParsedType::Intersection(expected_types) => {
            // An intersection's members all constrain the *same* argument at
            // once, so there is no member to choose between — but inferring
            // from every one of them lets a member that merely happens to line
            // up bind a parameter it should not (tanstack's `QueryKey`, a
            // `readonly unknown[]`, came back mutable that way).
            //
            // Only the *callable* members are walked, which is the shape this
            // exists for: zustand's
            // `StateCreator<T, Mis, Mos> = ((set: …) => U) & { $$storeMutators?: Mos }`
            // carries `Mis` in the call signature's parameters, and the marker
            // object half carries nothing an argument can be zipped against.
            for expected_element in expected_types.iter() {
                let callable = match expected_element {
                    ParsedType::Function(_) => true,
                    ParsedType::Object(object) => object.call_signature.is_some(),
                    _ => false,
                };
                if !callable {
                    continue;
                }
                collect_inferred_type_argument(
                    expected_element,
                    argument_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth,
                );
            }
        }
        ParsedType::Union(expected_types) => {
            // A union reached through an alias (`CallbackOrValue<string>` read
            // off an imported interface) is matched member by member, as the
            // union it names.
            let peeled_argument;
            let argument_type = match argument_type {
                Type::Reference(_) if matches!(argument_type.peeled(), Type::Union(_)) => {
                    peeled_argument = argument_type.peeled();
                    &peeled_argument
                }
                other => other,
            };
            // `getUnionType` flattens a union written inside a union — the shape
            // substituting `T := void | C2` into `MaybePromise<T> = T |
            // Promise<T>` writes — so the naked member stays visible below.
            let flattened;
            let expected_types: &[ParsedType] =
                if expected_types.iter().any(|member| matches!(member, ParsedType::Union(_))) {
                    flattened = flatten_parsed_union_members(expected_types);
                    &flattened
                } else {
                    expected_types
                };
            // tsc's union holds the alias's resolved signature, so `T |
            // InitialDataFunction<T>` pairs a function argument member with
            // `() => T | undefined` exactly as the written signature would.
            let alias_bodies: Vec<Option<(ParsedType, std::sync::Arc<str>)>> =
                expected_types.iter().map(|member| signature_alias_body(member, ctx)).collect();
            let expanded;
            let expected_types: &[ParsedType] = if alias_bodies.iter().any(Option::is_some) {
                expanded = expected_types
                    .iter()
                    .zip(&alias_bodies)
                    .map(|(member, body)| body.as_ref().map_or_else(|| member.clone(), |(body, _)| body.clone()))
                    .collect::<Vec<_>>();
                &expanded
            } else {
                expected_types
            };
            let alias_file_of = |target: &ParsedType| {
                expected_types
                    .iter()
                    .position(|member| std::ptr::eq(member, target))
                    .and_then(|index| alias_bodies[index].as_ref())
                    .map(|(_, file)| file.clone())
            };
            // `T | PromiseLike<T>` (the lib's `then` callbacks, `Awaited`-style
            // parameters): a promise argument infers `T` from what it resolves
            // to, never as the whole promise — tsc pairs it with the
            // `PromiseLike<T>` member first (`inferToMultipleTypes` infers to
            // every structured member before a naked one records the source),
            // and what remains for the naked `T` is the awaited value.
            if let Some(promised) = promise_like_member_target(expected_types)
                && crate::checks::call::promise_nominal_enabled()
            {
                let awaited = crate::checks::call::awaited_type(argument_type);
                if awaited != *argument_type {
                    collect_inferred_type_argument(
                        promised,
                        &awaited,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                    return;
                }
            }
            // `inferFromMatchingTypes` with `isTypeOrBaseIdenticalTo`: a source
            // member identical to a target member that names no type parameter
            // is matched with it, and both leave the inference (`get<U>(x: U |
            // void)` handed `string | void` binds `U` to `string`).
            if let Some((remaining_targets, remaining_sources)) = infer_from_matching_members(
                expected_types,
                argument_type,
                substitution,
                widen_literals,
                ctx,
                depth,
            ) {
                let Some(target) = (match remaining_targets.as_slice() {
                    [] => None,
                    [only] => Some(only.clone()),
                    _ => Some(ParsedType::Union(std::sync::Arc::new(remaining_targets.clone()))),
                }) else {
                    return;
                };
                // Every source matched: tsc still infers the whole source to what
                // is left, at a lower priority.
                let source = if remaining_sources.is_empty() {
                    argument_type.clone()
                } else {
                    surge_ts_types::union_type(remaining_sources)
                };
                collect_inferred_type_argument(&target, &source, substitution, widen_literals, ctx, depth);
                return;
            }
            // A union carries no member order, so zipping positionally is only
            // meaningful when every member has a shape to pin it to. Once the
            // union mixes a naked type parameter with structured members
            // (`undefined | TValue | ((q: Q) => TValue)`), a same-arity argument
            // union lines up by accident: tanstack's `resolveQueryValue` bound
            // `TValue` to the callback arm and made every `!== false` on the
            // result a false TS2367. Fall through to the shape-matching path.
            let stands_for_the_rest = |member: &ParsedType, ctx: &CheckerContext| {
                is_naked_type_parameter(member, substitution)
                    || is_conditional_alias_reference(member, ctx)
            };
            let mixes_naked_and_structured = expected_types
                .iter()
                .any(|member| stands_for_the_rest(member, ctx))
                && expected_types
                    .iter()
                    .any(|member| !stands_for_the_rest(member, ctx));
            if let Type::Union(actual_union) = argument_type
                && !mixes_naked_and_structured
                && expected_types.len() == actual_union.types().len()
            {
                for (expected_element, actual_element) in
                    expected_types.iter().zip(actual_union.types().iter())
                {
                    collect_inferred_type_argument(
                        expected_element,
                        actual_element,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                }
                return;
            }

            // React's `SetStateAction<S>` shape: `S | (() => S)`. Each argument
            // member goes to the union member whose *shape* it matches, and what
            // matches none is what a naked type parameter stands for. The naked
            // one is recorded first: candidates are first-wins, and a direct
            // member (`resultOf(value)` binding `T` to the value itself) is a
            // better answer than one read back out of a callback's return type.
            // A conditional-bodied alias member (`MaybePromise<DefaultValue<In,
            // $Output>>`) has no shape of its own either; what fits no shaped
            // member is what it, like a naked parameter, stands for.
            let (naked, structured): (Vec<&ParsedType>, Vec<&ParsedType>) = expected_types
                .iter()
                .partition(|member| stands_for_the_rest(member, ctx));
            if structured.is_empty() {
                // `inferToMultipleTypes` with no structured member infers the
                // source to every naked type variable, at a priority below any
                // argument's: `then`'s `Promise<TResult1 | TResult2>` under a
                // `Promise<string>` context binds the `TResult2` its callback
                // left open. Only the contextual return type's pass models that
                // priority (it fills only what the arguments left open).
                if INFERRING_FROM_RETURN_TYPE.get() {
                    for target in naked {
                        if is_naked_type_parameter(target, substitution) {
                            collect_inferred_type_argument(
                                target,
                                argument_type,
                                substitution,
                                widen_literals,
                                ctx,
                                depth,
                            );
                        }
                    }
                }
                return;
            }
            let argument_members: Vec<&Type> = match argument_type {
                Type::Union(union) => union.types().iter().collect(),
                other => vec![other],
            };
            let mut matches: Vec<(&ParsedType, &Type)> = Vec::new();
            let mut unmatched: Vec<Type> = Vec::new();
            // `inferToMultipleTypes` infers each source to every structured
            // target, not only the first that could take it; a source is matched
            // when some target made an inference from it. A generic reference
            // target has no shape to test, so it is inferred into directly.
            for member in argument_members {
                let before = matches.len();
                for target in structured.iter().filter(|target| parsed_shape_matches(target, member)) {
                    matches.push((target, member));
                }
                if matches.len() > before {
                    continue;
                }
                // Inferred as `inferFromTypes` would: element-wise for an array
                // target, otherwise through the reference's declaration, so a
                // promise target reached by a non-thenable source infers nothing
                // (`inferFromObjectTypes` finds no `then`).
                let inferences_before = INFERENCES_MADE.get();
                for target in structured.iter() {
                    if let ParsedType::Named(named) = target
                        && !named.type_arguments.is_empty()
                    {
                        if infers_element_wise(&named.name) {
                            collect_inferred_type_argument(target, member, substitution, widen_literals, ctx, depth);
                        } else {
                            infer_through_generic_reference(named, member, substitution, widen_literals, ctx, depth);
                        }
                    }
                }
                if INFERENCES_MADE.get() == inferences_before {
                    unmatched.push(member.clone());
                }
            }
            // Nothing shaped matched and there is no naked member to take the
            // rest: no evidence. With a naked member, an argument none of the
            // shapes fit (`A | B` against `TOut | Promise<TOut>`, or `{ a: 2 }`
            // against `T | undefined`) is what it stands for, in full.
            if matches.is_empty() && naked.is_empty() {
                return;
            }
            if !unmatched.is_empty() {
                let leftover = if unmatched.len() == 1 {
                    unmatched.remove(0)
                } else {
                    surge_ts_types::union_type(unmatched)
                };
                for target in naked {
                    collect_inferred_type_argument(
                        target,
                        &leftover,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                }
            }
            // Several argument members fitting the same shape are one candidate
            // (tsc unions the candidates): matched one at a time, the first-wins
            // rule kept a single member of a union return.
            let mut grouped: Vec<(&ParsedType, Vec<Type>)> = Vec::new();
            for (target, member) in matches {
                match grouped
                    .iter_mut()
                    .find(|(seen, _)| std::ptr::eq(*seen, target))
                {
                    Some((_, members)) => members.push(member.clone()),
                    None => grouped.push((target, vec![member.clone()])),
                }
            }
            for (target, mut members) in grouped {
                let member = if members.len() == 1 {
                    members.remove(0)
                } else {
                    surge_ts_types::union_type(members)
                };
                match alias_file_of(target) {
                    Some(file) => with_inference_scope_file(&file, ctx, |ctx| {
                        collect_inferred_type_argument(target, &member, substitution, widen_literals, ctx, depth)
                    }),
                    None => collect_inferred_type_argument(target, &member, substitution, widen_literals, ctx, depth),
                }
            }
        }
        // tsc's `inferToTemplateLiteralType`: each placeholder infers from what
        // it captures of the source. A source that does not fit the texts
        // infers nothing, so the placeholder keeps its constraint and the
        // argument then fails it, unless every text is empty: such a template
        // only extracts characters, and each placeholder infers `never`.
        ParsedType::TemplateLiteral(template) => {
            let texts: Vec<&str> = template.quasis.iter().map(String::as_str).collect();
            let targets = vec![Type::GenuineUnknown; template.interpolations.len()];
            let target_refs: Vec<&Type> = targets.iter().collect();
            let captured = match surge_ts_types::infer_template_literal_placeholders(argument_type, &texts, &target_refs)
            {
                Some(captured) => captured,
                None if texts.iter().all(|text| text.is_empty())
                    && matches!(argument_type, Type::StringLiteral(_)) =>
                {
                    vec![Type::Never; template.interpolations.len()]
                }
                None => return,
            };
            for (interpolation, captured) in template.interpolations.iter().zip(&captured) {
                collect_inferred_type_argument(interpolation, captured, substitution, widen_literals, ctx, depth + 1);
            }
        }
        _ => {}
    }
}

/// `{ [P in keyof T]: T[P] }` with only modifiers on top: what a source reverse
/// maps to through it is the source itself.
fn is_identity_homomorphic_mapped(mapped: &surge_ts_syntax::ParsedMappedType) -> bool {
    let (ParsedType::KeyOf(variable), ParsedType::IndexedAccess(access)) =
        (mapped.constraint.as_ref(), mapped.value_type.as_ref())
    else {
        return false;
    };
    let (ParsedType::Named(variable), ParsedType::Named(object)) = (variable.as_ref(), access.object_type.as_ref())
    else {
        return false;
    };
    mapped.name_type.is_none()
        && variable.type_arguments.is_empty()
        && object.type_arguments.is_empty()
        && object.name == variable.name
        && matches!(access.index_type.as_ref(), ParsedType::Named(index) if index.name == mapped.key_name)
}

/// `inferFromTypes`' two `inferFromMatchingTypes` passes over a union target:
/// first a source identical to a target free of the call's type parameters
/// (`isTypeOrBaseIdenticalTo`), then a source reference to the declaration a
/// target reference names (`isTypeCloselyMatchedBy`), inferred from its type
/// arguments. Returns the targets and sources left, or `None` when nothing
/// pairs. The target union is flattened through aliases of unions, as
/// `getUnionType` flattens it.
fn infer_from_matching_members(
    targets: &[ParsedType],
    source: &Type,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) -> Option<(Vec<ParsedType>, Vec<Type>)> {
    let sources: Vec<Type> = match source {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let flattened;
    let targets: &[ParsedType] = if sources.iter().any(|source| matches!(source, Type::Reference(_))) {
        flattened = flatten_alias_union_targets(targets, ctx, 0);
        &flattened
    } else {
        targets
    };
    let type_parameters: Vec<std::sync::Arc<str>> = substitution.iter().map(|(name, _)| name.clone()).collect();
    let mut matched_targets = vec![false; targets.len()];
    let mut matched_sources = vec![false; sources.len()];
    for (target_index, target) in targets.iter().enumerate() {
        if type_parameters.iter().any(|name| parsed_type_mentions_name(target, name)) {
            continue;
        }
        let diagnostics_before = ctx.diagnostics().len();
        let mapped = map_parsed_type_with_substitution(target.clone(), ctx, substitution);
        ctx.truncate_diagnostics(diagnostics_before);
        if mapped.is_unknown() {
            continue;
        }
        for (source_index, source) in sources.iter().enumerate() {
            let identical = *source == mapped
                || matches!((&mapped, source), (Type::String, Type::StringLiteral(_)))
                || matches!((&mapped, source), (Type::Number, Type::NumberLiteral(_)));
            if identical {
                matched_targets[target_index] = true;
                matched_sources[source_index] = true;
            }
        }
    }
    for (target_index, target) in targets.iter().enumerate() {
        let ParsedType::Named(named) = target else {
            continue;
        };
        if matched_targets[target_index] || named.type_arguments.is_empty() {
            continue;
        }
        for (source_index, source) in sources.iter().enumerate() {
            if matched_sources[source_index] {
                continue;
            }
            let Type::Reference(reference) = source else {
                continue;
            };
            if !reference.arguments.is_empty() && reference_names_declaration(reference, named, ctx) {
                infer_through_generic_reference(named, source, substitution, widen_literals, ctx, depth);
                matched_targets[target_index] = true;
                matched_sources[source_index] = true;
            }
        }
    }
    if !matched_sources.iter().any(|matched| *matched) {
        return None;
    }
    let remaining_targets = targets
        .iter()
        .zip(&matched_targets)
        .filter(|(_, matched)| !**matched)
        .map(|(target, _)| target.clone())
        .collect();
    let remaining_sources = sources
        .into_iter()
        .zip(matched_sources)
        .filter(|(_, matched)| !*matched)
        .map(|(source, _)| source)
        .collect();
    Some((remaining_targets, remaining_sources))
}

/// A reference's declaration is the one `named` resolves to. Unlike
/// [`reference_targets_declaration`], a name that resolves to nothing matches
/// nothing.
fn reference_names_declaration(
    reference: &surge_ts_types::TypeReference,
    named: &ParsedNamedType,
    ctx: &CheckerContext,
) -> bool {
    lookup_declaration_for_inference(&named.name, ctx).is_some()
        && !matches!(named.name.as_str(), "Array" | "ReadonlyArray")
        && reference_targets_declaration(reference, named, ctx)
}

/// The signature a generic alias reference stands for, written in the
/// reference's arguments, and the file declaring the alias, whose names the
/// signature is written in.
fn signature_alias_body(
    member: &ParsedType,
    ctx: &CheckerContext,
) -> Option<(ParsedType, std::sync::Arc<str>)> {
    signature_alias_body_at(member, ctx, 0)
}

fn signature_alias_body_at(
    member: &ParsedType,
    ctx: &CheckerContext,
    depth: usize,
) -> Option<(ParsedType, std::sync::Arc<str>)> {
    let ParsedType::Named(named) = member else {
        return None;
    };
    if named.type_arguments.is_empty() {
        return None;
    }
    let handle = lookup_declaration_for_inference(&named.name, ctx)?;
    let TypeDeclarationInfo::Alias(alias) = handle.get() else {
        return None;
    };
    let map = filled_type_parameter_map(&alias.body.type_parameters, &named.type_arguments);
    match &alias.body.ty {
        ParsedType::Function(_) => Some((
            crate::infer::substitute_parsed_type_parameters_deep(&alias.body.ty, &map),
            alias.file_name.clone(),
        )),
        // An alias of another signature alias (`DictionaryIterator<T, R> =
        // ObjectIterator<Dictionary<T>, R>`) is that one's signature, written
        // in the arguments this one passes it.
        ParsedType::Named(_) if depth < 4 => signature_alias_body_at(
            &crate::infer::substitute_parsed_type_parameters_deep(&alias.body.ty, &map),
            ctx,
            depth + 1,
        ),
        _ => None,
    }
}

/// Union targets with each alias whose body is a union replaced by that
/// union's members, written in the reference's arguments.
fn flatten_alias_union_targets(targets: &[ParsedType], ctx: &mut CheckerContext, depth: usize) -> Vec<ParsedType> {
    let mut flattened = Vec::with_capacity(targets.len());
    for target in targets {
        let expanded = match target {
            ParsedType::Union(members) if depth < 4 => Some(flatten_alias_union_targets(members, ctx, depth + 1)),
            ParsedType::Named(named) if depth < 4 && !named.type_arguments.is_empty() => {
                lookup_declaration_for_inference(&named.name, ctx).and_then(|handle| match handle.get() {
                    TypeDeclarationInfo::Alias(alias) => match &alias.body.ty {
                        ParsedType::Union(members) => {
                            let map = filled_type_parameter_map(&alias.body.type_parameters, &named.type_arguments);
                            let members: Vec<ParsedType> = members
                                .iter()
                                .map(|member| crate::infer::substitute_parsed_type_parameters_deep(member, &map))
                                .collect();
                            Some(members)
                        }
                        _ => None,
                    },
                    TypeDeclarationInfo::Interface(_) => None,
                })
                .map(|members| flatten_alias_union_targets(&members, ctx, depth + 1))
            }
            _ => None,
        };
        match expanded {
            Some(members) => flattened.extend(members),
            None => flattened.push(target.clone()),
        }
    }
    flattened
}

/// Lib references whose first type argument is the element an array or tuple
/// argument supplies, and which surge infers element by element.
fn infers_element_wise(name: &str) -> bool {
    matches!(
        name,
        "Array" | "ReadonlyArray" | "Iterable" | "IterableIterator" | "ArrayLike" | "ConcatArray"
    )
}

/// tsc's `getRestTypeAtPosition`: the source's parameters from `position` on,
/// as a tuple that ends in the source's own rest. A parameter the sketch left
/// un-annotated (a bare `any` before the rest) is no evidence, so none is
/// built from it.
fn rest_type_at_position(source: &FunctionType, position: usize) -> Option<Type> {
    let parameters = source.parameters();
    let rest_index = source
        .is_variadic()
        .then(|| parameters.len().checked_sub(1))
        .flatten();
    let fixed_end = rest_index.unwrap_or(parameters.len());
    let written = parameters.get(position..fixed_end).unwrap_or_default();
    if written.iter().any(|parameter| matches!(parameter, Type::Any)) {
        return None;
    }
    // An optional parameter is an optional element (`[foo: string, bar?:
    // number]`): its slot reads the element or `undefined`, and the tuple
    // records the `minLength` the optional elements leave (`ElementFlags`).
    let required = source.required_parameter_count();
    let leading: Vec<Type> = written
        .iter()
        .enumerate()
        .map(|(offset, parameter)| {
            if position + offset >= required {
                surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
            } else {
                parameter.clone()
            }
        })
        .collect();
    let leading_min_length = required.saturating_sub(position).min(leading.len());
    let Some(rest_index) = rest_index else {
        return Some(surge_ts_types::written_tuple_type(leading, leading_min_length));
    };
    // A variadic slot holds either the whole rest type (a signature mapped
    // from source, a tuple rest) or, as the resolver writes an array rest, its
    // element.
    let slot = &parameters[rest_index];
    if let Some((elements, rest_min_length)) = surge_ts_types::fixed_tuple_parts(slot)
        .map(|(elements, min_length)| (elements.to_vec(), min_length))
        .or_else(|| match slot.peeled() {
            Type::Tuple(elements) => {
                let min_length = surge_ts_types::tuple_min_length(&elements);
                Some((elements, min_length))
            }
            _ => None,
        })
    {
        if position >= rest_index {
            let skip = (position - rest_index).min(elements.len());
            return Some(surge_ts_types::written_tuple_type(
                elements[skip..].to_vec(),
                rest_min_length.saturating_sub(skip),
            ));
        }
        let min_length = if leading_min_length == leading.len() {
            leading.len() + rest_min_length
        } else {
            leading_min_length
        };
        return Some(surge_ts_types::written_tuple_type(
            leading.into_iter().chain(elements).collect(),
            min_length,
        ));
    }
    let rest = match slot.peeled() {
        shape @ (Type::Array(_) | Type::OpenTuple(_)) => shape,
        element => Type::Array(Box::new(element)),
    };
    if position >= rest_index {
        let skip = position - rest_index;
        return Some(match rest {
            Type::OpenTuple(open) if skip <= open.leading.len() => {
                Type::OpenTuple(surge_ts_types::OpenTupleType {
                    leading: open.leading.into_iter().skip(skip).collect(),
                    rest: open.rest,
                    trailing: open.trailing,
                })
            }
            other => other,
        });
    }
    Some(match rest {
        Type::Array(element) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading,
            rest: element,
            trailing: Vec::new(),
        }),
        Type::OpenTuple(open) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading: leading.into_iter().chain(open.leading).collect(),
            rest: open.rest,
            trailing: open.trailing,
        }),
        _ => return None,
    })
}

/// A union member that names a type parameter directly (`S` in `S | (() => S)`),
/// as opposed to one that gives the argument a shape to match against.
fn is_conditional_alias_reference(member: &ParsedType, ctx: &CheckerContext) -> bool {
    let ParsedType::Named(named) = member else {
        return false;
    };
    if named.type_arguments.is_empty() {
        return false;
    }
    lookup_declaration_for_inference(&named.name, ctx).is_some_and(|handle| {
        matches!(handle.get(), TypeDeclarationInfo::Alias(info)
            if matches!(info.body.ty, ParsedType::Conditional(_)))
    })
}

/// The `T` of a `T | PromiseLike<T>` / `T | Promise<T>` union, when the union
/// has exactly that shape.
fn flatten_parsed_union_members(members: &[ParsedType]) -> Vec<ParsedType> {
    let mut flattened = Vec::with_capacity(members.len());
    for member in members {
        match member {
            ParsedType::Union(inner) => flattened.extend(flatten_parsed_union_members(inner)),
            other => flattened.push(other.clone()),
        }
    }
    flattened
}

fn promise_like_member_target(members: &[ParsedType]) -> Option<&ParsedType> {
    let [first, second] = members else {
        return None;
    };
    fn promised(member: &ParsedType) -> Option<&ParsedType> {
        match member {
            ParsedType::Named(named)
                if matches!(named.name.as_str(), "PromiseLike" | "Promise")
                    && named.type_arguments.len() == 1 =>
            {
                Some(&named.type_arguments[0])
            }
            _ => None,
        }
    }
    // Written twice, so compared by name: the spans differ.
    fn bare_name(member: &ParsedType) -> Option<&str> {
        match member {
            ParsedType::Named(named) if named.type_arguments.is_empty() => Some(&named.name),
            _ => None,
        }
    }
    match (promised(first), promised(second)) {
        (Some(inner), None) if bare_name(inner).is_some() && bare_name(inner) == bare_name(second) => {
            Some(second)
        }
        (None, Some(inner)) if bare_name(inner).is_some() && bare_name(inner) == bare_name(first) => {
            Some(first)
        }
        _ => None,
    }
}

fn is_naked_type_parameter(member: &ParsedType, substitution: &TypeParameterSubstitution) -> bool {
    let ParsedType::Named(named) = member else {
        return false;
    };
    named.type_arguments.is_empty() && substitution.get(&named.name).is_some()
}

/// Whether an argument type could inhabit this union member at all, by shape.
/// Deliberately conservative: only the constructs that carry inference targets.
fn parsed_shape_matches(member: &ParsedType, ty: &Type) -> bool {
    match member {
        ParsedType::Function(_) => matches!(ty, Type::Function(_)),
        ParsedType::Array(_) => matches!(ty, Type::Array(_) | Type::Tuple(_)),
        ParsedType::Tuple(_) => matches!(ty, Type::Tuple(_)),
        ParsedType::Object(_) => matches!(ty, Type::Object(_)),
        // The nullish arms of an optional-value parameter
        // (`undefined | TValue | …`) claim their own argument member, so the
        // naked parameter is left standing for the value alone rather than for
        // `value | undefined`.
        ParsedType::Undefined => matches!(ty, Type::Undefined),
        ParsedType::Null => matches!(ty, Type::Null),
        // `readonly T[]` (`SetConstructor`'s `values?: readonly T[] | null`)
        // takes a mutable array or tuple as well as a readonly one.
        ParsedType::Readonly(inner) => {
            parsed_shape_matches(inner, ty) || matches!(ty, Type::Reference(reference) if reference.is_readonly_array())
        }
        _ => false,
    }
}

/// Infers type parameters that appear *inside* a generic parameter
/// (`def: SignalDefinition<TPayload>`, `w: Wrapper<T>`). When the argument is an
/// instantiation of the same declaration we match type arguments positionally;
/// when it is an object literal we resolve the declaration's members, substitute
/// the declaration's own type parameters with this position's type arguments, and
/// match member-by-member. Bounded by `depth` so mutually-generic declarations
/// cannot loop.
/// Whether an alias's body is a function type that names the alias's own type
/// parameters in a *parameter* position, the shape a callback argument infers
/// from.
fn alias_parameters_carry_type_parameters(alias: &crate::symbols::TypeAliasInfo, ctx: &CheckerContext) -> bool {
    !carried_type_parameters(alias, ctx, 0).is_empty()
}

/// The type parameters of `alias` its callback parameters carry (see
/// [`alias_parameters_carry_type_parameters`]).
fn carried_type_parameters<'a>(
    alias: &'a crate::symbols::TypeAliasInfo,
    ctx: &CheckerContext,
    depth: usize,
) -> Vec<&'a str> {
    // An intersection of a call signature with an object is still a callable
    // alias: zustand's `StateCreator<T, Mis, Mos> = ((set: …) => U) & { … }` is
    // the shape a middleware chain is written in, and refusing to look through
    // the intersection left the whole alias without an inference source.
    let signatures: Vec<&ParsedFunctionType> = match &alias.body.ty {
        ParsedType::Function(body) => vec![body.as_ref()],
        ParsedType::Intersection(members) => members
            .iter()
            .filter_map(|member| match member {
                ParsedType::Function(body) => Some(body.as_ref()),
                ParsedType::Object(object) => object.call_signature.as_deref(),
                _ => None,
            })
            .collect(),
        // An alias of another callback alias (`DictionaryIterator<T, R> =
        // ObjectIterator<Dictionary<T>, R>`) carries a parameter the arguments
        // it passes put where the aliased one's parameters read them.
        ParsedType::Named(named) if !named.type_arguments.is_empty() && depth < 4 => {
            let Some(handle) = lookup_declaration_for_inference(&named.name, ctx) else {
                return Vec::new();
            };
            let TypeDeclarationInfo::Alias(inner) = handle.get() else {
                return Vec::new();
            };
            let inner_carried: Vec<String> =
                carried_type_parameters(inner, ctx, depth + 1).into_iter().map(str::to_string).collect();
            return alias
                .body
                .type_parameters
                .iter()
                .filter(|type_parameter| {
                    inner.body.type_parameters.iter().zip(&named.type_arguments).any(|(inner_parameter, argument)| {
                        inner_carried.contains(&inner_parameter.name)
                            && parsed_type_mentions_name(argument, &type_parameter.name)
                    })
                })
                .map(|type_parameter| type_parameter.name.as_str())
                .collect();
        }
        _ => return Vec::new(),
    };
    alias
        .body
        .type_parameters
        .iter()
        .filter(|type_parameter| {
            signatures.iter().any(|signature| {
                signature
                    .parameters
                    .iter()
                    .any(|parameter| parsed_type_mentions_name(&parameter.ty, &type_parameter.name))
            })
        })
        .map(|type_parameter| type_parameter.name.as_str())
        .collect()
}

fn infer_through_generic_reference(
    named_type: &ParsedNamedType,
    argument_type: &Type,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    const MAX_DEPTH: usize = 6;
    if depth >= MAX_DEPTH {
        return;
    }

    // A reference that is not an instantiation (a deferred value annotation,
    // `let VariableDeclaration: Type<VariableDeclaration>` read off a namespace
    // object) carries no arguments to zip; what it resolves to does.
    if let Type::Reference(reference) = argument_type
        && reference.arguments.is_empty()
        && !named_type.type_arguments.is_empty()
    {
        let resolved = argument_type.peeled();
        if !matches!(&resolved, Type::Reference(_)) && !resolved.is_unknown() {
            return infer_through_generic_reference(
                named_type,
                &resolved,
                substitution,
                widen_literals,
                ctx,
                depth + 1,
            );
        }
    }

    // tsc infers from type arguments only between references to the same
    // generic target, or two array types (`inferFromObjectTypes`); a reference
    // to another declaration is inferred from as the structure it expands to.
    // Zipping any reference by position bound `TQueryFnData` to the `() =>
    // number` of a `Mock<() => number>` matched against `Query<TQueryFnData, …>`.
    if let Type::Reference(reference) = argument_type
        && !reference_targets_declaration(reference, named_type, ctx)
    {
        // A readonly array or tuple keeps its readonly wrapper when peeled; the
        // shape it stands for is inside it.
        let resolved = if reference.is_readonly_array() {
            reference.resolve()
        } else {
            argument_type.peeled()
        };
        if !matches!(&resolved, Type::Reference(_)) && !resolved.is_unknown() {
            return infer_through_generic_reference(
                named_type,
                &resolved,
                substitution,
                widen_literals,
                ctx,
                depth + 1,
            );
        }
        return;
    }
    if let Type::Reference(reference) = argument_type {
        // A reference written with fewer arguments than its declaration takes
        // stands for the defaults too (`AsyncIterable<T>` is
        // `AsyncIterable<T, any, any>`), as its instantiation fills them in.
        let defaulted = if named_type.type_arguments.len() > reference.arguments.len() {
            defaulted_type_arguments(&named_type.name, reference.arguments.len(), ctx)
        } else {
            Vec::new()
        };
        inferring_from_type_arguments(|| {
            for (pattern_argument, actual_argument) in named_type
                .type_arguments
                .iter()
                .zip(reference.arguments.iter().chain(defaulted.iter()))
            {
                collect_inferred_type_argument(
                    pattern_argument,
                    actual_argument,
                    substitution,
                    widen_literals,
                    ctx,
                    depth + 1,
                );
            }
        });
        return;
    }

    // The alias is tried before the object requirement below: its body can be any
    // shape, so `MutationFunction<TData, TVariables>` matches a function argument
    // that has no object surface at all.
    let (body, declaring_file) = {
        let Some(handle) = lookup_declaration_for_inference(&named_type.name, ctx) else {
            return;
        };
        // `inferFromTypes` takes a union source member by member against a
        // target that is not itself a union: two links of one alias that
        // resolved apart (`[logLink, okLink]`) still bind its arguments.
        if let Type::Union(union) = argument_type
            && !matches!(
                handle.get(),
                TypeDeclarationInfo::Alias(info)
                    if matches!(info.body.ty, ParsedType::Conditional(_) | ParsedType::Union(_))
            )
        {
            let members = union.types().to_vec();
            for member in &members {
                infer_through_generic_reference(named_type, member, substitution, widen_literals, ctx, depth + 1);
            }
            return;
        }
        match handle.get() {
            TypeDeclarationInfo::Interface(info) => (info.body.clone(), info.file_name.clone()),
            TypeDeclarationInfo::Alias(info) => {
                // An object argument matches an alias body of any shape, as it
                // always has. A *function* argument is admitted only where the
                // inference is the one this path exists for: a callback alias
                // whose PARAMETERS carry the type parameters
                // (`MutationFunction<TData, TVariables> = (variables:
                // TVariables, …) => …`). An alias that mentions them only in its
                // return (`TRPCLink<TRouter> = (opts) => OperationLink<TRouter>`)
                // is left alone — matching one bound tRPC's `TRouter` from a
                // link argument and collapsed `inferClientTypes<TRouter>`. Go
                // never walks that body: `inferFromTypes` infers straight from
                // the type arguments when source and target share the alias
                // symbol, an identity surge's resolved types do not carry.
                // A union body is entered for any argument: `inferFromTypes`
                // sees only the resolved union, so `string` against
                // `MaybePromise<O> = Promise<O> | O` reaches
                // `inferToMultipleTypes` and binds the naked `O`.
                // A function *literal* is the exception to the exception: its
                // type carries no alias, so `inferFromTypes` has nothing to
                // match by identity and walks the signatures, return included
                // (`query(() => 1)` against `Resolver<…, $Output> = (opts) =>
                // MaybePromise<$Output>`). The link above was a *declared*
                // `TRPCLink<AnyRouter>` value.
                let shape_matches = matches!(argument_type, Type::Object(_))
                    || matches!(
                        info.body.ty,
                        ParsedType::Conditional(_) | ParsedType::Union(_)
                    )
                    // An alias for an array (`RecArray<T> = Array<T | RecArray<T>>`)
                    // is the array reference an array argument infers against.
                    || (matches!(argument_type.peeled(), Type::Array(_) | Type::Tuple(_))
                        && match &info.body.ty {
                            ParsedType::Array(_) => true,
                            ParsedType::Named(body) => {
                                matches!(body.name.as_str(), "Array" | "ReadonlyArray")
                                    && !body.type_arguments.is_empty()
                            }
                            // `Readonly<T>` reverse-maps an array or tuple to itself.
                            ParsedType::Mapped(mapped) => is_identity_homomorphic_mapped(mapped),
                            _ => false,
                        })
                    || (matches!(argument_type, Type::Function(_))
                        && (alias_parameters_carry_type_parameters(info, ctx)
                            || SOURCE_IS_FUNCTION_LITERAL.get()));
                if !shape_matches {
                    return;
                }
                let file_name = info.file_name.clone();
                with_inference_scope_file(&file_name, ctx, |ctx| {
                    infer_through_generic_alias(
                        info,
                        named_type,
                        argument_type,
                        substitution,
                        widen_literals,
                        ctx,
                        depth,
                    );
                });
                return;
            }
        }
    };
    // A reference may write fewer arguments than the declaration has
    // parameters (`FetchOptions<R>` for `FetchOptions<R, T = any>`); the
    // trailing defaults carry no inference and are simply left unmapped.
    if body.type_parameters.len() < named_type.type_arguments.len() {
        return;
    }
    let parameter_map = filled_type_parameter_map(&body.type_parameters, &named_type.type_arguments);
    infer_from_interface_signatures(
        &body,
        &parameter_map,
        &declaring_file,
        argument_type,
        substitution,
        widen_literals,
        ctx,
        depth,
    );
    let Type::Object(actual_object) = argument_type else {
        return;
    };

    // A member's annotation names what the *declaring* file has in scope
    // (`MutationFunction`, which the consumer never imported), so the walk
    // records that file for the declaration lookups inside it.
    with_inference_scope_file(&declaring_file, ctx, |ctx| {
        for member in &body.members {
            let Some(actual_member_type) = actual_object.get_property_access_type(&member.name)
            else {
                continue;
            };
            let Some(actual_member_type) = optional_member_source(actual_member_type, member.optional, &member.ty)
            else {
                continue;
            };
            // The declaration's parameters are replaced wherever the member
            // names them, a method's parameters and return included, except
            // where a generic method's own parameter shadows one.
            let expected_member_type = match &member.ty {
                ParsedType::Function(function)
                    if function
                        .type_parameters
                        .iter()
                        .any(|parameter| parameter_map.contains_key(&parameter.name)) =>
                {
                    let mut unshadowed = parameter_map.clone();
                    for parameter in &function.type_parameters {
                        unshadowed.remove(&parameter.name);
                    }
                    crate::infer::substitute_parsed_type_parameters_deep(&member.ty, &unshadowed)
                }
                _ => crate::infer::substitute_parsed_type_parameters_deep(&member.ty, &parameter_map),
            };
            with_bivariant_inference(member.is_method, || {
                collect_inferred_type_argument(
                    &expected_member_type,
                    &actual_member_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth + 1,
                );
            });
        }

        let string_index = body
            .string_index_type
            .as_ref()
            .map(|index| crate::infer::substitute_parsed_type_parameters_deep(index, &parameter_map));
        let number_index = body
            .number_index_type
            .as_ref()
            .map(|index| crate::infer::substitute_parsed_type_parameters_deep(index, &parameter_map));
        infer_from_index_types(
            [string_index.as_ref(), number_index.as_ref()],
            actual_object,
            substitution,
            widen_literals,
            ctx,
            depth + 1,
        );

        // An options interface usually declares almost nothing itself:
        // `MutationObserverOptions<TData, TError, TVariables, TOnMutateResult>`
        // extends `MutationOptions<…>`, which is where `mutationFn` lives — the
        // member the call infers `TVariables` from. The base's arguments are
        // written in terms of this declaration's parameters, so they carry the
        // same map.
        for base in &body.extends {
            let base_arguments: Vec<ParsedType> = base
                .type_arguments
                .iter()
                .map(|argument| substitute_parsed_type_parameters(argument, &parameter_map))
                .collect();
            if base_arguments.is_empty() {
                continue;
            }
            let base = ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: base.name.clone(),
                span: base.span,
                type_arguments: base_arguments,
            };
            infer_through_generic_reference(
                &base,
                argument_type,
                substitution,
                widen_literals,
                ctx,
                depth + 1,
            );
        }
    });
}

/// `inferFromSignatures` for an interface target: a function argument infers
/// through its call signature (`FunctionComponent<P>`'s `(props: P)`), a
/// constructor through its first construct signature. A callable object
/// argument is left to the member walk.
#[allow(clippy::too_many_arguments)]
fn infer_from_interface_signatures(
    body: &crate::symbols::InterfaceBody,
    parameter_map: &surge_ts_types::fx::FxHashMap<String, ParsedType>,
    declaring_file: &std::sync::Arc<str>,
    argument_type: &Type,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    let substituted = |signature: &ParsedFunctionType| {
        let mut unshadowed = parameter_map.clone();
        for parameter in &signature.type_parameters {
            unshadowed.remove(&parameter.name);
        }
        match crate::infer::substitute_parsed_type_parameters_deep(
            &ParsedType::Function(std::sync::Arc::new(signature.clone())),
            &unshadowed,
        ) {
            ParsedType::Function(function) => Some(function),
            _ => None,
        }
    };
    let call = match (argument_type, body.call_signature.as_ref()) {
        (Type::Function(_), Some(call)) => substituted(call),
        _ => None,
    };
    let construct = match (argument_type, body.construct_signatures.first()) {
        (Type::Object(object), Some(construct)) if object.properties.is_empty() => object
            .construct_signature()
            .cloned()
            .and_then(|actual| Some((substituted(construct)?, actual))),
        _ => None,
    };
    if call.is_none() && construct.is_none() {
        return;
    }
    with_inference_scope_file(declaring_file, ctx, |ctx| {
        if let Some(call) = call {
            collect_inferred_type_argument(
                &ParsedType::Function(call),
                argument_type,
                substitution,
                widen_literals,
                ctx,
                depth + 1,
            );
        }
        if let Some((expected, actual)) = construct {
            collect_signature_inferences(&expected, actual, substitution, widen_literals, ctx, depth + 1);
        }
    });
}

thread_local! {
    /// Files whose declarations the current inference walk is reading members
    /// from, innermost last. Consulted only to *find* a declaration a member's
    /// annotation names; the context keeps pointing at the file being checked,
    /// so nothing resolved during the walk is keyed on a borrowed scope — moving
    /// the context itself drifted tRPC's `inferClientTypes<T>['errorShape']` to
    /// `never`.
    static INFERENCE_SCOPE_FILES: std::cell::RefCell<Vec<std::sync::Arc<str>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn with_inference_scope_file<R>(
    file: &std::sync::Arc<str>,
    ctx: &mut CheckerContext,
    resolve: impl FnOnce(&mut CheckerContext) -> R,
) -> R {
    INFERENCE_SCOPE_FILES.with(|files| files.borrow_mut().push(file.clone()));
    let resolved = resolve(ctx);
    INFERENCE_SCOPE_FILES.with(|files| {
        files.borrow_mut().pop();
    });
    resolved
}

/// Whether `reference` instantiates the declaration `named_type` names: its
/// nominal id is that declaration's `file\0Name`, or both are array types. A
/// declaration surge cannot find proves nothing either way and keeps the
/// positional reading.
fn reference_targets_declaration(
    reference: &surge_ts_types::TypeReference,
    named_type: &ParsedNamedType,
    ctx: &CheckerContext,
) -> bool {
    if matches!(named_type.name.as_str(), "Array" | "ReadonlyArray") && reference.is_readonly_array()
    {
        return true;
    }
    let Some(handle) = lookup_declaration_for_inference(&named_type.name, ctx) else {
        return true;
    };
    let (file_name, declaration_name) = match handle.get() {
        TypeDeclarationInfo::Alias(alias) => (alias.file_name.clone(), alias.name.clone()),
        TypeDeclarationInfo::Interface(interface) => {
            (interface.file_name.clone(), interface.name.clone())
        }
    };
    reference
        .id
        .strip_prefix(file_name.as_ref() as &str)
        .and_then(|rest| rest.strip_prefix('\u{0}'))
        .is_some_and(|written| names_one_declaration(written, &declaration_name))
}

/// A reference's id carries the name as written where it was referenced
/// (`core.$constructor` through a namespace import) and a declaration the name
/// it declares; within one file they name one declaration when one is the
/// other's dotted tail.
fn names_one_declaration(written: &str, declared: &str) -> bool {
    written == declared
        || written
            .strip_suffix(declared)
            .is_some_and(|qualifier| qualifier.ends_with('.'))
        || declared
            .strip_suffix(written)
            .is_some_and(|qualifier| qualifier.ends_with('.'))
}

/// The declared defaults of `name`'s type parameters from position `written`
/// on, as far as each resolves on its own; one naming another type parameter
/// stops the list.
fn defaulted_type_arguments(name: &str, written: usize, ctx: &mut CheckerContext) -> Vec<Type> {
    let Some(handle) = lookup_declaration_for_inference(name, ctx) else {
        return Vec::new();
    };
    let parameters = match handle.get() {
        TypeDeclarationInfo::Interface(info) => info.body.type_parameters.clone(),
        TypeDeclarationInfo::Alias(info) => info.body.type_parameters.clone(),
    };
    let mut defaults = Vec::new();
    for parameter in parameters.iter().skip(written) {
        let Some(default) = parameter.default_type.clone() else {
            break;
        };
        let diagnostics_before = ctx.diagnostics().len();
        let resolved = crate::infer::map_parsed_type(default, ctx);
        let failed = ctx.diagnostics().len() != diagnostics_before;
        ctx.truncate_diagnostics(diagnostics_before);
        if failed || resolved.is_unknown() {
            break;
        }
        defaults.push(resolved);
    }
    defaults
}

fn lookup_declaration_for_inference(
    name: &str,
    ctx: &CheckerContext,
) -> Option<crate::symbols::TypeDeclarationHandle> {
    if let Some(handle) = ctx.lookup_type_declaration_handle(name) {
        return Some(handle);
    }
    INFERENCE_SCOPE_FILES.with(|files| {
        files
            .borrow()
            .iter()
            .rev()
            .find_map(|file| ctx.lookup_type_declaration_handle_in_file(name, file))
    })
}

/// Infers through a generic *alias* parameter (`config?: Config<T>`). A plain
/// object body matches like an interface's members. A conditional body whose
/// check type is one of the alias's own parameters (`T extends ConfigSchema ?
/// { variants?: T; … } : never` — the class-variance-authority shape) matches
/// the argument against the substituted TRUE branch: selecting that branch is
/// exactly what a successful inference implies, and its members are where the
/// parameter occurs (tsc infers `T` from `variants` the same way).
/// A declaration's type parameters mapped to a reference's arguments, the
/// missing ones filled with their defaults as `fillMissingTypeArguments` does
/// (`unknown` without one), so no name of the declaration's own is left in
/// the walk to be read as one of the call's type parameters.
fn filled_type_parameter_map(
    parameters: &[surge_ts_syntax::ParsedTypeParameter],
    arguments: &[ParsedType],
) -> surge_ts_types::fx::FxHashMap<String, ParsedType> {
    let mut map: surge_ts_types::fx::FxHashMap<String, ParsedType> = Default::default();
    for (index, parameter) in parameters.iter().enumerate() {
        let argument = match arguments.get(index) {
            Some(argument) => argument.clone(),
            None => match &parameter.default_type {
                Some(default) => crate::infer::substitute_parsed_type_parameters_deep(default, &map),
                None => ParsedType::UnknownKeyword,
            },
        };
        map.insert(parameter.name.clone(), argument);
    }
    map
}

fn infer_through_generic_alias(
    alias: &crate::symbols::TypeAliasInfo,
    named_type: &ParsedNamedType,
    argument_type: &Type,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    if alias.body.type_parameters.len() < named_type.type_arguments.len() {
        return;
    }
    let parameter_map = filled_type_parameter_map(&alias.body.type_parameters, &named_type.type_arguments);

    let mut body = &alias.body.ty;
    if let ParsedType::Conditional(conditional) = body {
        let check_is_own_parameter = matches!(
            conditional.check_type.as_ref(),
            ParsedType::Named(check) if parameter_map.contains_key(&check.name)
        );
        if check_is_own_parameter {
            body = conditional.true_type.as_ref();
        } else {
            // The branch is not decided by the argument, so the argument infers
            // into both, as tsc does (`DefaultValue<In, $Output> = Unset extends
            // In ? $Output : In` binds `$Output` from a resolver's return).
            for branch in [&conditional.true_type, &conditional.false_type] {
                let substituted =
                    crate::infer::substitute_parsed_type_parameters_deep(branch, &parameter_map);
                collect_inferred_type_argument(
                    &substituted,
                    argument_type,
                    substitution,
                    widen_literals,
                    ctx,
                    depth + 1,
                );
            }
            return;
        }
    }

    let substituted = crate::infer::substitute_parsed_type_parameters_deep(body, &parameter_map);
    collect_inferred_type_argument(
        &substituted,
        argument_type,
        substitution,
        widen_literals,
        ctx,
        depth + 1,
    );
}

/// Substitutes bare named references in a parsed type using `map`, recursing into
/// generic arguments and array elements. Used to rewrite a declaration's member
/// types from the declaration's own type parameters into the enclosing call's
/// type parameters before inference.
fn substitute_parsed_type_parameters(
    parsed_type: &ParsedType,
    map: &surge_ts_types::fx::FxHashMap<String, ParsedType>,
) -> ParsedType {
    match parsed_type {
        ParsedType::Named(named) => {
            if named.type_arguments.is_empty() {
                if let Some(replacement) = map.get(&named.name) {
                    return replacement.clone();
                }
                ParsedType::Named(named.clone())
            } else {
                ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                    type_argument_spans: Vec::new(),
                    name: named.name.clone(),
                    span: named.span,
                    type_arguments: named
                        .type_arguments
                        .iter()
                        .map(|argument| substitute_parsed_type_parameters(argument, map))
                        .collect(),
                }))
            }
        }
        ParsedType::Array(element) => ParsedType::Array(std::sync::Arc::new(
            substitute_parsed_type_parameters(element, map),
        )),
        other => other.clone(),
    }
}

/// `inferFromSignatures` from an argument's signature to an expected one.
fn collect_signature_inferences(
    expected_function: &ParsedFunctionType,
    actual_function: FunctionType,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    // `inferFromSignatures` pairs a single target signature with the
    // source's *last* one, not with an overload group's fold.
    let (actual_function, single_signature) = match actual_function
        .overloads()
        .and_then(|members| members.last())
    {
        Some(last) => (last.clone(), false),
        None => (actual_function, true),
    };
    // tsc's `instantiateTypeWithSingleGenericCallSignature`: a generic
    // function passed where a non-generic signature is expected is
    // instantiated in that signature's context first, so `id` against
    // `(values: number[]) => r` contributes `number[]` for `r` rather
    // than a placeholder return that infers nothing.
    // It applies only to a type with exactly one call signature.
    let actual_function = if single_signature
        && actual_function.type_parameter_head().is_some()
        && expected_function.type_parameters.is_empty()
    {
        let diagnostics_before = ctx.diagnostics().len();
        let contextual = map_parsed_type_with_substitution(
            ParsedType::Function(std::sync::Arc::new(expected_function.clone())),
            ctx,
            substitution,
        );
        ctx.truncate_diagnostics(diagnostics_before);
        match contextual {
            Type::Function(contextual) => {
                surge_ts_types::generic_source_in_context_of(&actual_function, &contextual)
                    .unwrap_or(actual_function)
            }
            _ => actual_function,
        }
    } else {
        actual_function
    };
    let actual_parameters = actual_function.parameters();
    let rest_index = actual_function
        .is_variadic()
        .then(|| actual_parameters.len().checked_sub(1))
        .flatten();
    for (index, expected_parameter) in expected_function.parameters.iter().enumerate() {
        // `applyToParameterTypes`: a rest parameter of the target infers
        // from the source's parameters from its position on, as one
        // tuple (`getRestTypeAtPosition`), so `(...args: T) => U`
        // against `(x: string, y: number) => …` binds `T` to
        // `[string, number]`.
        if expected_parameter.rest && index + 1 == expected_function.parameters.len() {
            if let Some(rest) = rest_type_at_position(&actual_function, index) {
                let outer_variance = INFERENCE_CONTRAVARIANT
                    .replace(INFERENCE_CONTRAVARIANT.get() != surge_ts_types::strict_function_types());
                collect_inferred_type_argument(
                    &expected_parameter.ty,
                    &rest,
                    substitution,
                    false,
                    ctx,
                    depth,
                );
                INFERENCE_CONTRAVARIANT.set(outer_variance);
            }
            break;
        }
        // A rest parameter stands for every position from its own on,
        // and it is its *element* the expected parameter there lines up
        // with: `(...args: any[]) => any` — vitest's `vi.fn()` — binds
        // each of `(data: TData, variables: TVariables)` to `any`, as
        // tsc does, rather than the first to `any[]` and the rest to
        // nothing (which left `TVariables` at its `void` default and
        // rejected every `mutate(1)`).
        let rest_element = match rest_index {
            Some(rest_index) if index >= rest_index => {
                // A rest spelled as a fixed tuple (`...args: []`, which
                // `Parameters<() => R>` resolves to) has exactly its
                // elements' positions: tsc's `getParameterCount` counts
                // those, so nothing past them is inferred from.
                let rest_type = actual_parameters[rest_index].peeled();
                if let Type::Tuple(elements) = &rest_type
                    && index - rest_index >= elements.len()
                {
                    break;
                }
                Some(rest_parameter_element_type(&rest_type, index - rest_index))
            }
            _ => None,
        };
        let Some(actual_parameter) = rest_element
            .as_ref()
            .or_else(|| actual_parameters.get(index))
        else {
            break;
        };
        // A bare `any` on a non-rest position is the sketch's placeholder
        // for an *un-annotated* callback parameter, which is what the
        // signature is about to type — inferring from it would bind the
        // type parameter to `any` and silence the callback body's own
        // errors. A rest element is written, never sketched.
        if rest_element.is_none() && matches!(actual_parameter, Type::Any) {
            continue;
        }
        // `inferFromSignature` infers parameter types contravariantly under
        // `strictFunctionTypes` (`inferFromContravariantTypesIfStrictFunctionTypes`),
        // and covariantly without it.
        let outer_variance = INFERENCE_CONTRAVARIANT
            .replace(INFERENCE_CONTRAVARIANT.get() != surge_ts_types::strict_function_types());
        collect_inferred_type_argument(
            &expected_parameter.ty,
            actual_parameter,
            substitution,
            false,
            ctx,
            depth,
        );
        INFERENCE_CONTRAVARIANT.set(outer_variance);
    }
    // `inferFromSignature`: two predicates of the same kind infer between
    // their types instead of between the `boolean` returns.
    if let ParsedType::Predicate(expected_predicate) = expected_function.return_type.as_ref()
        && let Some(expected_ty) = &expected_predicate.ty
        && let Some(actual_predicate) = actual_function.type_predicate()
        && let Some(actual_ty) = &actual_predicate.ty
        && predicate_kinds_match(expected_predicate, actual_predicate.kind)
    {
        collect_inferred_type_argument(expected_ty, actual_ty, substitution, widen_literals, ctx, depth);
        return;
    }
    collect_inferred_type_argument(
        &expected_function.return_type,
        actual_function.return_type(),
        substitution,
        widen_literals,
        ctx,
        depth,
    );
}

/// What a source member infers to an optional target member: tsc's target is
/// the member's type with `undefined` added, which matches the source's own
/// `undefined` (`inferFromMatchingTypes`) and leaves the rest. `None` when
/// nothing but `undefined` is left.
fn optional_member_source(source: Type, optional: bool, target: &ParsedType) -> Option<Type> {
    // Under `exactOptionalPropertyTypes` the member adds the missing type,
    // which a source `undefined` does not match.
    if !optional || matches!(target, ParsedType::Undefined) || surge_ts_types::exact_optional_property_types() {
        return Some(source);
    }
    match source {
        Type::Undefined => None,
        Type::Union(union) if union.types().iter().any(|member| *member == Type::Undefined) => {
            if let ParsedType::Union(members) = target
                && members.iter().any(|member| matches!(member, ParsedType::Undefined))
            {
                return Some(Type::Union(union));
            }
            Some(surge_ts_types::union_type(
                union.types().iter().filter(|member| **member != Type::Undefined).cloned().collect(),
            ))
        }
        other => Some(other),
    }
}

/// `inferFromIndexTypes`: a declared string or number index signature infers
/// from the argument's applicable one, and from an object literal type's
/// properties, its implicit index signature (`Dictionary<T>` handed a
/// `Record<string, Foo>` or `{ a: Foo }` binds `T` to `Foo`).
fn infer_from_index_types(
    [string_index, number_index]: [Option<&ParsedType>; 2],
    actual_object: &surge_ts_types::ObjectType,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    for (target_index, numeric) in [(string_index, false), (number_index, true)] {
        let Some(target_index) = target_index else {
            continue;
        };
        let mut sources: Vec<Type> = Vec::new();
        if !actual_object.without_inferable_index {
            for (name, property) in actual_object.properties.iter() {
                if name.starts_with('[') || numeric && name.parse::<f64>().is_err() {
                    continue;
                }
                sources.push(if property.optional {
                    surge_ts_types::remove_undefined(&property.ty)
                } else {
                    property.ty.clone()
                });
            }
        }
        let source_index = if numeric {
            actual_object.number_index_type.as_deref().or(actual_object.string_index_type.as_deref())
        } else {
            actual_object.string_index_type.as_deref()
        };
        if let Some(source_index) = source_index
            && !actual_object.synthetic_open_index
        {
            sources.push(source_index.clone());
        }
        // An object literal's members are typed widened where the argument
        // is checked, and so is the index signature its computed keys build;
        // a candidate kept literal would reject that very argument. Only the
        // literal itself widens: a declared member type (`dataType:
        // ColumnDataType` of a column class) is the candidate as written.
        if !sources.is_empty() {
            collect_inferred_type_argument(
                target_index,
                &surge_ts_types::union_type(sources.iter().map(widen_literal_type).collect()),
                substitution,
                widen_literals,
                ctx,
                depth,
            );
        }
    }
}

pub(crate) fn collect_object_type_candidates(
    expected_object_type: &ParsedObjectType,
    actual_object_type: &surge_ts_types::ObjectType,
    substitution: &mut TypeParameterSubstitution,
    widen_literals: bool,
    ctx: &mut CheckerContext,
    depth: usize,
) {
    for property in &expected_object_type.properties {
        let Some(actual_property_type) =
            actual_object_type.get_property_access_type(&property.name)
        else {
            continue;
        };
        let Some(actual_property_type) = optional_member_source(actual_property_type, property.optional, &property.ty)
        else {
            continue;
        };

        with_bivariant_inference(property.is_method, || {
            collect_inferred_type_argument(
                &property.ty,
                &actual_property_type,
                substitution,
                widen_literals,
                ctx,
                depth,
            );
        });
    }
}

pub(crate) fn record_type_argument_candidate(
    substitution: &mut TypeParameterSubstitution,
    type_parameter_name: &str,
    argument_type: &Type,
    widen_literals: bool,
) {
    // Binding a type parameter to a shape carrying the degradation sentinel
    // re-types every parameter of the signature with it, so a candidate surge
    // could not fully infer is no candidate at all. A placeholder is not that: a
    // generic function passed as an argument carries its own type parameters.
    if type_argument_is_unresolved(argument_type) {
        return;
    }
    let Some(existing) = substitution.get(type_parameter_name).cloned() else {
        return;
    };
    INFERENCES_MADE.set(INFERENCES_MADE.get() + 1);

    record_generic_call_inference_candidate();
    let candidate = with_type_copy_reason(TypeCopyReason::CallResolution, || argument_type.clone());
    let widened = (widen_literals && !substitution.keeps_literal(type_parameter_name))
        .then(|| widen_candidate_type(argument_type));

    if substitution.is_inference_fixed(type_parameter_name) {
        return;
    }
    let contravariant = INFERENCE_CONTRAVARIANT.get() && !INFERENCE_BIVARIANT.get();
    let top_level = INFERENCE_ROOT_PARAMETER.with(|root| {
        root.borrow()
            .as_ref()
            .is_none_or(|root| type_parameter_at_top_level(root, type_parameter_name, 0))
    });
    let fresh = !contravariant && SOURCE_IS_FRESH_LITERAL.get();
    let literal = !contravariant
        && SOURCE_IS_OBJECT_OR_ARRAY_LITERAL.get()
        && matches!(
            candidate,
            Type::Object(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)
        );
    // A binding made before inference recorded anything stands as its first
    // candidate.
    if substitution.inference_candidates(type_parameter_name).is_empty()
        && !existing.is_degraded()
    {
        substitution.push_inference_candidate(
            type_parameter_name,
            InferenceCandidate {
                ty: existing,
                literal: false,
                contravariant: false,
                top_level: true,
                fresh: false,
                widened: None,
                depth: 0,
            },
        );
    }
    let recorded = InferenceCandidate {
        ty: candidate,
        literal,
        contravariant,
        top_level,
        fresh,
        widened: widened.filter(|_| !contravariant),
        depth: INFERENCE_TYPE_ARGUMENT_DEPTH.get(),
    };
    // `inferFromTypes` records a candidate once in each list.
    if contravariant {
        if substitution
            .inference_candidates(type_parameter_name)
            .iter()
            .any(|existing| existing.contravariant && existing.ty == recorded.ty)
        {
            return;
        }
        substitution.push_inference_candidate(type_parameter_name, recorded);
    } else if !substitution.insert_covariant_candidate(type_parameter_name, recorded) {
        return;
    }
    let inferred = inferred_type(substitution.inference_candidates(type_parameter_name), false);
    substitution.set(type_parameter_name.to_string(), inferred, false);
}

/// tsc's `getInferredType` from the candidates recorded so far. With both
/// kinds, the covariant inference is preferred when it is neither `never` nor
/// `any`, every covariant candidate is assignable to it, and it is assignable
/// to some contravariant candidate; otherwise the contravariant one stands.
/// (tsc also asks that no other parameter constrained to this one has a
/// candidate the covariant inference would not accept; one parameter's
/// candidates are all this sees.) `widen_literals` is `widenLiteralTypes`
/// beyond what recording a candidate already widened: set once the parameter
/// is fixed.
fn inferred_type(candidates: &[InferenceCandidate], widen_literals: bool) -> Type {
    let every_top_level = candidates
        .iter()
        .filter(|candidate| !candidate.contravariant)
        .all(|candidate| candidate.top_level);
    let effective: Vec<InferenceCandidate> = candidates
        .iter()
        .map(|candidate| match (&candidate.widened, every_top_level) {
            (Some(widened), true) => InferenceCandidate {
                ty: widened.clone(),
                ..candidate.clone()
            },
            _ => candidate.clone(),
        })
        .collect();
    let (contra, co): (Vec<&InferenceCandidate>, Vec<&InferenceCandidate>) =
        effective.iter().partition(|candidate| candidate.contravariant);
    // `getBaseTypeOfLiteralType` gives an enum member its enum as base, so
    // members of different enums share none (`g(E1.X, E2.X)` binds `E1`).
    // Applied only where every candidate is an enum member and none is
    // contravariant: elsewhere the covariant supertype surge settles on still
    // matches the one tsc's fuller inference picks.
    let enum_identity = contra.is_empty() && co.len() > 1 && co.iter().all(|candidate| enum_member_owner(&candidate.ty).is_some());
    let covariant = (!co.is_empty()).then(|| covariant_inference(&co, widen_literals, enum_identity));
    let Some(contravariant) = contravariant_inference(&contra) else {
        return covariant.unwrap_or(Type::Never);
    };
    match covariant {
        Some(covariant)
            if !matches!(covariant, Type::Never | Type::Any | Type::ErrorType)
                && co
                    .iter()
                    .all(|candidate| is_assignable_to(&candidate.ty, &covariant))
                && contra
                    .iter()
                    .any(|candidate| is_assignable_to(&covariant, &candidate.ty)) =>
        {
            covariant
        }
        _ => contravariant,
    }
}

/// tsc's `getWidenedLiteralType`: a literal becomes its primitive, member by
/// member through a union; an object's members are left as they are.
fn widen_literal_type(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Union(union) => {
            surge_ts_types::union_type(union.types().iter().map(widen_literal_type).collect())
        }
        other => other.clone(),
    }
}

/// tsc's `getCovariantInference`. The object and array literal candidates are
/// first replaced by their union (`unionObjectAndArrayLiteralCandidates`),
/// placed after the others; the common supertype is then taken and widened
/// (`getWidenedType`): literal members widen together, and without
/// `strictNullChecks` `null` and `undefined` widen to `any`. Literal widening
/// itself happened as each candidate was recorded.
fn covariant_inference(candidates: &[&InferenceCandidate], widen_literals: bool, enum_identity: bool) -> Type {
    let widened = |candidate: &InferenceCandidate| {
        if widen_literals && candidate.fresh {
            widen_literal_type(&candidate.ty)
        } else {
            candidate.ty.clone()
        }
    };
    if let [only] = candidates {
        return crate::checks::var::widen_nullable_type(&widened(only));
    }
    let literals: Vec<Type> = candidates
        .iter()
        .filter(|candidate| candidate.literal)
        .map(|candidate| candidate.ty.clone())
        .collect();
    let mut types: Vec<(Type, bool)> = candidates
        .iter()
        .filter(|candidate| !candidate.literal)
        .map(|candidate| (widened(candidate), false))
        .collect();
    if !literals.is_empty() {
        types.push((surge_ts_types::union_type(literals.clone()), true));
    }
    crate::checks::var::widen_nullable_type(&normalize_object_literal_members(
        &common_supertype(
            types,
            candidates
                .iter()
                .any(|candidate| !candidate.fresh && literal_base_type(&candidate.ty).is_some()),
            enum_identity,
        ),
        &literals,
    ))
}

/// tsc's `getContravariantInference` through `getCommonSubtype`: the leftmost
/// candidate no later one is a subtype of.
fn contravariant_inference(candidates: &[&InferenceCandidate]) -> Option<Type> {
    candidates
        .iter()
        .map(|candidate| candidate.ty.clone())
        .reduce(|subtype, candidate| {
            if is_candidate_subtype(&candidate, &subtype, false) {
                candidate
            } else {
                subtype
            }
        })
}

/// tsc's `getCommonSupertype`. Under `strictNullChecks` nullable candidates do
/// not compete: the supertype is chosen among the rest and every candidate's
/// `null` and `undefined` are added back, so `eq(b as B, d as D | undefined)`
/// binds `T` to `B | undefined`. The supertype is the leftmost candidate no
/// later one is a supertype of (`findLeftmostType` with `isTypeSubtypeOf`).
/// Each type is paired with whether it is the object literal candidates' union.
fn common_supertype(types: Vec<(Type, bool)>, union_same_base_literals: bool, enum_identity: bool) -> Type {
    if types.len() == 1 {
        return types.into_iter().next().map(|(ty, _)| ty).unwrap_or(Type::Never);
    }
    let mut nullable: Vec<Type> = Vec::new();
    let mut primary: Vec<(Type, bool)> = Vec::with_capacity(types.len());
    for (ty, literal) in types {
        if !surge_ts_types::strict_null_checks() {
            primary.push((ty, literal));
            continue;
        }
        let members: Vec<Type> = match ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other],
        };
        let (nulls, rest): (Vec<Type>, Vec<Type>) = members
            .into_iter()
            .partition(|member| matches!(member, Type::Null | Type::Undefined));
        nullable.extend(nulls);
        if !rest.is_empty() {
            primary.push((surge_ts_types::union_type(rest), literal));
        }
    }
    // `literalTypesWithSameBaseType`: literal candidates of one primitive are
    // unioned rather than reduced to a supertype. Candidates that are all fresh
    // literals still settle on the primitive below: tsc's union of them stays
    // fresh and a mutable binding widens it, a freshness surge's types do not
    // carry.
    if union_same_base_literals && primary.len() > 1 {
        let bases: Vec<Option<Type>> = primary.iter().map(|(ty, _)| literal_base_type(ty)).collect();
        if bases.iter().all(|base| base.is_some() && *base == bases[0]) {
            let mut members: Vec<Type> = primary.into_iter().map(|(ty, _)| ty).collect();
            members.extend(nullable);
            return surge_ts_types::union_type(members);
        }
    }
    let supertype = primary.into_iter().reduce(|left, right| {
        // A later `never` never decides the supertype: `withFew(xs, id, fail)`
        // binds `r` from `id`'s return, not from `fail`'s.
        if left.0 == right.0 || matches!(right.0, Type::Never) {
            return left;
        }
        // Candidates of one primitive kind settle on the primitive; tsc unions
        // same-kind literals (`literalTypesWithSameBaseType`), which surge's
        // declarations would keep unwidened.
        if enum_identity
            && let (Some(left_enum), Some(right_enum)) = (enum_member_owner(&left.0), enum_member_owner(&right.0))
        {
            if left_enum.0 == right_enum.0
                && let Some(base) = left_enum.1
            {
                return ((*base).clone(), false);
            }
        } else if let Some(primitive) = common_primitive_candidate(&left.0, &right.0) {
            return (primitive, false);
        }
        if is_candidate_subtype(&left.0, &right.0, right.1) {
            right
        } else {
            left
        }
    });
    let mut members: Vec<Type> = supertype.into_iter().map(|(ty, _)| ty).collect();
    members.extend(nullable);
    surge_ts_types::union_type(members)
}

/// The primitive a literal type, or a union of literals of one primitive,
/// widens to (`getBaseTypeOfLiteralType`); `None` for anything that is not a
/// literal.
fn literal_base_type(ty: &Type) -> Option<Type> {
    match ty {
        Type::StringLiteral(_) => Some(Type::String),
        Type::NumberLiteral(_) => Some(Type::Number),
        Type::BooleanLiteral(_) => Some(Type::Boolean),
        Type::Union(union) => {
            let mut bases = union.types().iter().map(literal_base_type);
            let first = bases.next()??;
            bases.all(|base| base.as_ref() == Some(&first)).then_some(first)
        }
        Type::Reference(reference) if reference.enum_base.is_none() => match reference.resolve() {
            resolved @ Type::Union(_) => literal_base_type(&resolved),
            _ => None,
        },
        _ => None,
    }
}

/// `isTypeSubtypeOf` for candidate selection, whose source is never an object
/// literal: the literal candidates' union always comes last. Every type is a
/// subtype of `any`, which is itself a subtype only of `unknown`
/// (`isSimpleTypeRelatedTo` admits an `any` source only for assignability).
/// Between object types the subtype relation asks more than assignability:
/// the source lacks no target property, optional or not
/// (`requireOptionalProperties`), and an object literal target has every
/// property the source has (`propertiesRelatedTo`).
fn is_candidate_subtype(source: &Type, target: &Type, literal_target: bool) -> bool {
    let (written_source, written_target) = (source, target);
    let (source, target) = (source.peeled(), target.peeled());
    match (&source, &target) {
        (_, Type::Any | Type::ErrorType | Type::GenuineUnknown) => return true,
        (Type::Any | Type::ErrorType, _) => return false,
        // Surge's sentinel and an unsubstituted placeholder relate to anything,
        // which says nothing about either being a supertype.
        (_, Type::Unknown | Type::TypeParameter(_)) => return false,
        _ => {}
    }
    if let Type::Union(union) = &source {
        return union
            .types()
            .iter()
            .all(|member| is_candidate_subtype(member, &target, literal_target));
    }
    if let Type::Union(union) = &target {
        return union
            .types()
            .iter()
            .any(|member| is_candidate_subtype(&source, member, literal_target));
    }
    // An object surge could not enumerate (`synthetic_open_index`) has members
    // it does not list, so only assignability can judge it.
    if let (Type::Object(source_object), Type::Object(target_object)) = (&source, &target)
        && !source_object.synthetic_open_index
        && !target_object.synthetic_open_index
    {
        let lacks_target_property = target_object
            .properties
            .keys()
            .any(|name| !source_object.properties.contains_key(name));
        let carries_unknown_property = literal_target
            && source_object
                .properties
                .keys()
                .any(|name| !target_object.properties.contains_key(name));
        if lacks_target_property || carries_unknown_property {
            return false;
        }
    }
    is_assignable_to(written_source, written_target)
}

/// `getWidenedType` of the inferred type: object literals widened together
/// (`getWidenedTypeOfObjectLiteral` with the union as the widening context)
/// each gain the properties their sibling literals write, as optional
/// `undefined` members, so `f({ x: 1 }, { y: '' })` infers
/// `{ x: number; y?: undefined } | { x?: undefined; y: string }`.
fn normalize_object_literal_members(ty: &Type, literals: &[Type]) -> Type {
    let Type::Union(union) = ty else {
        return ty.clone();
    };
    let is_literal_object =
        |member: &Type| matches!(member, Type::Object(_)) && literals.contains(member);
    let mut names: Vec<std::sync::Arc<str>> = Vec::new();
    for member in union.types().iter().filter(|member| is_literal_object(member)) {
        if let Type::Object(object) = member {
            for name in object.properties.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
    }
    let members: Vec<Type> = union
        .types()
        .iter()
        .map(|member| match member {
            Type::Object(object)
                if is_literal_object(member)
                    && names.iter().any(|name| !object.properties.contains_key(name)) =>
            {
                let mut properties = (*object.properties).clone();
                for name in &names {
                    if !properties.contains_key(name) {
                        properties.insert(name.clone(), crate::checks::var::undefined_or_missing_property());
                    }
                }
                let mut widened = alloc_object_type(
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
            other => other.clone(),
        })
        .collect();
    surge_ts_types::union_type(members)
}

/// The enum an enum member's type belongs to, with the enum type its literal
/// widens to.
fn enum_member_owner(ty: &Type) -> Option<(std::sync::Arc<str>, Option<std::sync::Arc<Type>>)> {
    match ty {
        Type::Reference(reference) => reference
            .enum_owner
            .clone()
            .map(|owner| (owner, reference.enum_base.clone())),
        _ => None,
    }
}

pub(crate) fn common_primitive_candidate(existing: &Type, candidate: &Type) -> Option<Type> {
    match (existing.base_primitive(), candidate.base_primitive()) {
        (Some(Type::String), Some(Type::String)) => Some(Type::String),
        (Some(Type::Number), Some(Type::Number)) => Some(Type::Number),
        (Some(Type::Boolean), Some(Type::Boolean)) => Some(Type::Boolean),
        _ => None,
    }
}

pub(crate) fn widen_candidate_type(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Array(element) => Type::Array(Box::new(widen_candidate_type(element.as_ref()))),
        Type::Tuple(elements) => Type::Tuple(elements.iter().map(widen_candidate_type).collect()),
        Type::Object(object) => {
            let properties = object
                .properties
                .iter()
                .map(|(name, property)| {
                    (
                        name.clone(),
                        surge_ts_types::ObjectProperty {
                            ty: widen_candidate_type(&property.ty),
                            optional: property.optional,
                            method: property.method,
                            readonly: property.readonly,
                            restriction: property.restriction.clone(),
                            index_slot: property.index_slot,
                        },
                    )
                })
                .collect::<surge_ts_types::PropertyMap>();

            // Only the members widen. The index signature, surge's openness
            // marker and the signatures still describe the value: dropping the
            // marker closed `makeAsyncResource({ ...degraded, k })`'s `T` and
            // every member the spread stood for read as missing.
            let mut widened = alloc_object_type(
                properties,
                object
                    .string_index_type
                    .as_deref()
                    .map(widen_candidate_type),
            );
            if object.synthetic_open_index {
                widened = widened.with_open_index_marker();
            }
            if object.non_primitive {
                widened = widened.with_non_primitive_marker();
            }
            if let Some(call_signature) = object.call_signature() {
                widened = widened.with_call_signature(call_signature.clone());
            }
            if let Some(construct_signature) = object.construct_signature() {
                widened = widened.with_construct_signature(construct_signature.clone());
            }
            Type::Object(widened)
        }
        Type::Union(union_payload) => surge_ts_types::union_type(
            union_payload
                .types()
                .iter()
                .map(widen_candidate_type)
                .collect(),
        ),
        _ => with_type_copy_reason(TypeCopyReason::CallResolution, || ty.clone()),
    }
}

/// `inferTypes` from the contextual type to a return type read from the body,
/// which has no written annotation for `infer_type_arguments_from_expected_return_type`
/// to walk: `let d: Deferred<T> | null = createDeferred()` binds `createDeferred`'s
/// own parameter from `Deferred<T>`. The body is read once with each open
/// parameter bound to a marker, and the contextual type is matched against
/// where the markers land. Only parameters nothing else bound take it.
fn infer_from_expected_to_body_return(
    function_type: &FunctionType,
    function_signature: &FunctionSignatureInfo,
    expected: &Type,
    substitution: &mut TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) {
    const MARKER: &str = "\u{0}body-return-inference\u{0}";
    fn walk(source: &Type, target: &Type, found: &mut Vec<(String, Type)>, depth: usize) {
        if depth > 8 || target.is_unmodelled() {
            return;
        }
        if let Type::StringLiteral(value) = source {
            if let Some(name) = value.strip_prefix(MARKER)
                && !found.iter().any(|(bound, _)| bound == name)
            {
                found.push((name.to_string(), target.clone()));
            }
            return;
        }
        if let Type::Union(union) = target {
            let mut members = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Null | Type::Undefined));
            if let (Some(only), None) = (members.next(), members.next()) {
                walk(source, only, found, depth + 1);
            }
            return;
        }
        match (source, target) {
            (Type::Reference(source_reference), Type::Reference(target_reference))
                if source_reference.id == target_reference.id
                    && source_reference.arguments.len() == target_reference.arguments.len() =>
            {
                for (source_argument, target_argument) in
                    source_reference.arguments.iter().zip(target_reference.arguments.iter())
                {
                    walk(source_argument, target_argument, found, depth + 1);
                }
            }
            (Type::Array(source_element), Type::Array(target_element)) => {
                walk(source_element, target_element, found, depth + 1);
            }
            (Type::Function(source_function), Type::Function(target_function)) => {
                for (source_parameter, target_parameter) in
                    source_function.parameters().iter().zip(target_function.parameters())
                {
                    walk(source_parameter, target_parameter, found, depth + 1);
                }
                walk(source_function.return_type(), target_function.return_type(), found, depth + 1);
            }
            (Type::Object(source_object), _) => {
                let Type::Object(target_object) = target.peeled() else {
                    return;
                };
                for (name, property) in source_object.properties.iter() {
                    if let Some(target_property) = target_object.get_property(name) {
                        walk(&property.ty, &target_property.ty, found, depth + 1);
                    }
                }
            }
            (Type::Reference(_), _) | (_, Type::Reference(_)) => {
                let (source, target) = (source.peeled(), target.peeled());
                if matches!(source, Type::Object(_)) {
                    walk(&source, &target, found, depth + 1);
                }
            }
            _ => {}
        }
    }
    let open: Vec<String> = substitution
        .iter()
        .filter(|(name, candidate)| candidate.is_degraded() && substitution.is_placeholder(name))
        .map(|(name, _)| name.to_string())
        .collect();
    if open.is_empty() {
        return;
    }
    let mut probe = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    for name in &open {
        probe.insert(name.clone(), Type::StringLiteral(format!("{MARKER}{name}").into()));
    }
    let probed = instantiate_function_type_with_substitution(function_type, function_signature, &probe, ctx);
    let mut found = Vec::new();
    walk(probed.return_type(), expected, &mut found, 0);
    for (name, bound) in found {
        if open.contains(&name) {
            substitution.insert(name, bound);
        }
    }
}

fn predicate_kinds_match(
    expected: &surge_ts_syntax::ParsedPredicateType,
    actual: surge_ts_types::TypePredicateKind,
) -> bool {
    use surge_ts_types::TypePredicateKind as Kind;
    let this = expected.parameter_name == "this";
    matches!(
        (this, expected.asserts, actual),
        (true, false, Kind::This)
            | (false, false, Kind::Identifier)
            | (true, true, Kind::AssertsThis)
            | (false, true, Kind::AssertsIdentifier)
    )
}
