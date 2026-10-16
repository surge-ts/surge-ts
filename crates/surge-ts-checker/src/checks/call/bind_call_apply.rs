//! `call`, `apply` and `bind` on a value with call or construct signatures,
//! which under `strictBindCallApply` are the lib's `CallableFunction` /
//! `NewableFunction` members (tsgo's `getPropertyOfTypeEx`). Their
//! `this: (this: T, ...args: A) => R` parameter is inferred from the receiver
//! (`inferTypeArguments` infers from the `this` argument first), so `A` is the
//! receiver's own parameter list and the remaining arguments are checked
//! against it.

use std::sync::Arc;

use surge_ts_syntax::{ParsedCallArgument, ParsedType, TextSpan as SyntaxTextSpan};
use surge_ts_types::{
    FunctionType, ObjectType, OpenTupleType, PropertyMap, Type, union_type, written_tuple_type,
};

use crate::context::CheckerContext;
use crate::symbols::SymbolTable;

#[derive(Clone, Copy)]
enum FunctionInterface {
    Callable,
    Newable,
}

struct BoundCall {
    signature: FunctionType,
    result: Type,
}

/// Checks `f.call(…)`, `f.apply(…)` or `f.bind(…)` against the signature the
/// lib member instantiates to. `None` when the member is not the lib's (or
/// surge cannot relate the receiver's parameters position by position), so
/// the caller resolves the call as before.
pub(crate) fn check_bind_call_apply(
    receiver: &Type,
    property_name: &str,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Option<Type>> {
    let bound = bound_call(receiver, property_name, type_arguments, arguments, ctx)?;
    // `bind(thisArg)` fits both overloads, so a `thisArg` the receiver's
    // written `this` rejects fails them both (TS2769).
    if property_name == "bind"
        && let [this_argument] = arguments
        && !this_argument.spread
        && let Some((_, target)) = receiver_signature(receiver, property_name, true)
        && let Some(this_type) = target.this_type().filter(|this_type| {
            !this_type.is_unmodelled() && !matches!(this_type, Type::Void | Type::Any)
        })
        && let crate::infer::InferredExpression::Known(argument_type) = {
            let checkpoint = ctx.diagnostics().len();
            let evaluated = crate::checks::expr::evaluate_expression(
                &this_argument.expression,
                this_argument.span,
                symbols,
                ctx,
            );
            ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
            evaluated
        }
        && !argument_type.is_unmodelled()
        && !surge_ts_types::is_assignable_to(&argument_type, this_type)
    {
        // tsc reports the last overload's failure: against a receiver whose
        // only parameter is a type-variable rest that is the method's `this`
        // context, at the call; otherwise the argument.
        let lone_variable_rest = matches!(target.parameters(), [rest] if rest.is_type_variable());
        let mut diagnostic = surge_ts_diagnostics::Diagnostic::ts2769(ctx.file_name.clone());
        if !lone_variable_rest {
            let target = crate::checks::expr::reported_relation_target(&argument_type, this_type);
            let last = crate::checks::expr::argument_not_assignable_diagnostic(
                &argument_type,
                &target,
                &crate::checks::expr::source_display_name(&argument_type, &target),
                &target.name(),
                ctx.file_name.clone(),
            );
            let mut chain = vec!["  The last overload gave the following error.".to_string()];
            chain.extend(last.message.lines().map(|line| format!("    {line}")));
            diagnostic = crate::checks::relation_chain::with_chain(diagnostic, &chain);
        }
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            diagnostic,
            if lone_variable_rest { call_span } else { this_argument.span },
        ));
        return Some(Some(bound.result));
    }
    Some(
        super::check_property_function_type_call(
            &bound.signature,
            property_span,
            call_span,
            &[],
            arguments,
            None,
            symbols,
            ctx,
        )
        .map(|_| bound.result),
    )
}

/// The type [`check_bind_call_apply`] gives the call, for the inference path.
pub(crate) fn bind_call_apply_result(
    receiver: &Type,
    property_name: &str,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    ctx: &CheckerContext,
) -> Option<Type> {
    bound_call(receiver, property_name, type_arguments, arguments, ctx).map(|bound| bound.result)
}

fn bound_call(
    receiver: &Type,
    property_name: &str,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    ctx: &CheckerContext,
) -> Option<BoundCall> {
    if !ctx.options.strict_bind_call_apply
        || !type_arguments.is_empty()
        || !matches!(property_name, "call" | "apply" | "bind")
    {
        return None;
    }
    let bind_this_only = property_name == "bind" && arguments.len() == 1 && !arguments[0].spread;
    let (interface, target) = receiver_signature(receiver, property_name, bind_this_only)?;
    let global_interface = match interface {
        FunctionInterface::Callable => "globalThis.CallableFunction",
        FunctionInterface::Newable => "globalThis.NewableFunction",
    };
    ctx.lookup_type_declaration(global_interface)?;
    let invoked = match interface {
        FunctionInterface::Callable => target.return_type().clone(),
        FunctionInterface::Newable => Type::Void,
    };
    let has_spread = arguments.iter().any(|argument| argument.spread);
    match property_name {
        "call" if arguments.is_empty() => Some(BoundCall {
            signature: arity_only_signature(invoked.clone(), true),
            result: invoked,
        }),
        "call" => Some(BoundCall {
            signature: with_this_argument(
                &target,
                target.parameters().len(),
                target.is_variadic(),
                invoked.clone(),
            ),
            result: invoked,
        }),
        "apply" if has_spread => None,
        "apply" => {
            // With `thisArg` alone only `apply<T, R>(this: (this: T) => R,
            // thisArg: T)` fits.
            let signature = match arguments.len() {
                1 => FunctionType::new(vec![this_argument_type(&target)], invoked.clone(), false, 1),
                2 => FunctionType::new(
                    vec![this_argument_type(&target), apply_arguments_type(&target)?],
                    invoked.clone(),
                    false,
                    2,
                ),
                _ => arity_only_signature(invoked.clone(), false),
            };
            Some(BoundCall {
                signature,
                result: invoked,
            })
        }
        _ if has_spread => None,
        _ => Some(bind_call(interface, &target, receiver, arguments.len())),
    }
}

/// The signature tsgo's `inferFromSignatures` infers `A` and `R` from: the
/// receiver's last, paired with the member's one `this` signature.
///
/// `bind` given `thisArg` alone binds no argument, so a rest typed by a type
/// variable of the body (`...args: T`) still keeps its `this` checked.
fn receiver_signature(
    receiver: &Type,
    property_name: &str,
    bind_this_only: bool,
) -> Option<(FunctionInterface, FunctionType)> {
    let (interface, signature) = match receiver.peeled() {
        Type::Function(function) => (FunctionInterface::Callable, function),
        Type::Object(object) => {
            // A member of its own (or one an index signature answers) is not
            // the lib's.
            if object.is_intersection
                || object.string_index_type.is_some()
                || object.properties.contains_key(property_name)
            {
                return None;
            }
            match (object.call_signature(), object.construct_signature()) {
                (Some(call), _) => (FunctionInterface::Callable, call.clone()),
                (None, Some(construct)) => (FunctionInterface::Newable, construct.clone()),
                (None, None) => return None,
            }
        }
        _ => return None,
    };
    let last_overload = signature.overloads().map(|overloads| overloads.last().cloned());
    let signature = match last_overload {
        Some(last) => last?,
        None => signature,
    };
    // A generic receiver infers from its base signature (`getBaseSignature`),
    // which surge does not build.
    let modelled = signature.overloads().is_none()
        && super::own_type_parameter_names(&signature).is_empty()
        && !signature.parameters().iter().any(super::type_contains_degradation)
        && (!signature.is_variadic()
            || matches!(signature.parameters().last().map(Type::peeled), Some(Type::Array(_)))
            || bind_this_only && signature.parameters().last().is_some_and(Type::is_type_variable));
    modelled.then_some((interface, signature))
}

/// The members' shapes before anything is inferred, which is what an arity
/// error is reported against: `thisArg` and a rest (`call`, `bind`), or
/// `thisArg` and an optional argument list (`apply`'s two overloads).
fn arity_only_signature(result: Type, rest: bool) -> FunctionType {
    if rest {
        FunctionType::new(vec![Type::Any, Type::Array(Box::new(Type::Any))], result, true, 1)
    } else {
        FunctionType::new(vec![Type::Any, Type::Any], result, false, 1)
    }
}

/// `(thisArg: T, ...args: A)` with `A` the receiver's first `count`
/// parameters, the tuple rest expanded into positions as `getParameterCount`
/// counts them. `T` is inferred from `thisArg` itself unless the receiver
/// declares a `this`, which surge's signatures do not keep, so it is `any`.
/// The `thisArg` `call` and `apply` take: `T` of the receiver's
/// `(this: T, ...) => R`, inferred from it first; `any` without a written one.
fn this_argument_type(target: &FunctionType) -> Type {
    target
        .this_type()
        .filter(|this_type| !this_type.is_unmodelled())
        .cloned()
        .unwrap_or(Type::Any)
}

fn with_this_argument(
    target: &FunctionType,
    count: usize,
    is_variadic: bool,
    result: Type,
) -> FunctionType {
    let mut parameters = Vec::with_capacity(count + 1);
    parameters.push(this_argument_type(target));
    parameters.extend(target.parameters()[..count].iter().cloned());
    let mut names: Vec<Option<Arc<str>>> = Vec::with_capacity(count + 1);
    names.push(Some(Arc::from("thisArg")));
    match target.parameter_names() {
        Some(written) => names.extend(written[..count].iter().cloned()),
        None => names.extend(std::iter::repeat_n(None, count)),
    }
    let required = target.required_parameter_count().min(count) + 1;
    FunctionType::new(parameters, result, is_variadic, required).with_parameter_names(names)
}

/// tsgo's `getRestTypeAtPosition(signature, 0)`: the receiver's parameters as
/// the tuple `apply` takes, an optional parameter as an optional element.
fn apply_arguments_type(target: &FunctionType) -> Option<Type> {
    let parameters = target.parameters();
    let required = target.required_parameter_count();
    if !target.is_variadic() {
        let elements = parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                if index < required {
                    parameter.clone()
                } else {
                    union_type(vec![parameter.clone(), Type::Undefined])
                }
            })
            .collect();
        return Some(written_tuple_type(elements, required));
    }
    let (rest, fixed) = parameters.split_last()?;
    if fixed.is_empty() {
        return Some(rest.clone());
    }
    // An open tuple has no optional leading element.
    if required < fixed.len() {
        return None;
    }
    let Type::Array(element) = rest.peeled() else {
        return None;
    };
    Some(Type::OpenTuple(OpenTupleType {
        leading: fixed.to_vec(),
        rest: element,
        trailing: Vec::new(),
    }))
}

/// `bind`'s two overloads. With `thisArg` alone the first,
/// `bind<T>(this: T, thisArg: ThisParameterType<T>): OmitThisParameter<T>`,
/// hands the receiver back. With bound arguments only the second fits: its
/// `this: (this: T, ...args: [...A, ...B]) => R` splits the receiver's
/// parameters at `A`'s implied arity (the argument count,
/// `inferFromObjectTypes`), `A` checks the arguments and `B` is left for the
/// bound function (`new (...args: B) => R` for `NewableFunction`).
fn bind_call(
    interface: FunctionInterface,
    target: &FunctionType,
    receiver: &Type,
    argument_count: usize,
) -> BoundCall {
    // `OmitThisParameter<T>`: the receiver without the `this` it was bound to.
    let unbound = match receiver.peeled() {
        Type::Function(function) if function.this_type().is_some() => Type::Function(function.with_this_type(None)),
        _ => receiver.clone(),
    };
    let bound = match argument_count {
        0 => {
            return BoundCall {
                signature: arity_only_signature(unbound.clone(), true),
                result: unbound,
            };
        }
        1 => {
            return BoundCall {
                signature: FunctionType::new(vec![Type::Any], unbound.clone(), false, 1),
                result: unbound,
            };
        }
        count => count - 1,
    };
    let parameters = target.parameters();
    let variadic = target.is_variadic();
    let fixed = parameters.len() - usize::from(variadic);
    let signature = if bound <= fixed {
        with_this_argument(target, bound, false, Type::Any)
    } else {
        with_this_argument(target, parameters.len(), variadic, Type::Any)
    };
    let rest_start = bound.min(fixed);
    let mut bound_function = FunctionType::new(
        parameters[rest_start..].to_vec(),
        target.return_type().clone(),
        variadic,
        target.required_parameter_count().saturating_sub(rest_start),
    );
    if let Some(names) = target.parameter_names() {
        bound_function = bound_function.with_parameter_names(names[rest_start..].to_vec());
    }
    let result = match interface {
        FunctionInterface::Callable => Type::Function(bound_function),
        FunctionInterface::Newable => Type::Object(
            ObjectType::new(PropertyMap::default(), None).with_construct_signature(bound_function),
        ),
    };
    BoundCall { signature, result }
}
