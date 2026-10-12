use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator};
use surge_ts_types::{Type, TypeCopyReason, with_type_copy_reason};

use std::sync::Arc;

use crate::context::CheckerContext;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};
use super::guards::*;
use super::narrow_discriminant_in_scope;
use super::{
    ReferenceGuard, narrow_instanceof_heritage_symbol_table, narrow_reference_in_scope,
    narrowed_reference_type, reference_path,
};

/// The signature a predicate guard reads for a callee: the declaration's own
/// collected signature, else the written call signature of the callable type
/// a value is declared with (`declare const isE: { (v: unknown): v is E }`).
fn predicate_signature_of(
    symbol: &SymbolInfo,
    ctx: &CheckerContext,
) -> Option<Arc<crate::symbols::FunctionSignatureInfo>> {
    symbol.function_signature.clone().or_else(|| {
        crate::checks::call::written_call_signature_info(&symbol.ty, ctx, false)
            .map(|written| written.signature)
    })
}

/// A substitution carrying the enclosing bindings a member predicate was read
/// under, so `v is T` on `Type<Identifier>.check` resolves `T` to `Identifier`.
fn seeded_predicate_substitution(guard: &PredicateGuardInfo) -> crate::infer::TypeParameterSubstitution {
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for (name, ty) in &guard.outer_type_arguments {
        substitution.insert(name.clone(), ty.clone());
    }
    substitution
}

/// Resolves what a guard site asked for: a named callee through `resolve`,
/// a member callee through the receiver expression's type. A member's
/// predicate rides on the written signature attached at resolution (see
/// `DeclaredMemberSignature`); a union receiver (`Type<T>` is a union of
/// classes sharing `check`) reads the first callable member.
pub(super) fn predicate_callee_signature<'a>(
    callee: PredicateCallee<'_>,
    resolve: impl Fn(&str) -> Option<&'a SymbolInfo>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<PredicateSignature> {
    match callee {
        PredicateCallee::Name(name) => {
            let symbol = resolve(name)?;
            if let Some(signature) = predicate_signature_of(symbol, ctx) {
                return Some(PredicateSignature::plain(signature));
            }
            // A binding holding a function *value* whose signature was read
            // off a declaration (`const isPost = isMatching(pattern)` returns
            // `(value: unknown) => value is …`) carries that written signature
            // on the handle, exactly like a member predicate does.
            let Type::Function(function) = symbol.ty.peeled() else {
                return None;
            };
            let declared = function
                .declaration()?
                .downcast_ref::<crate::checks::call::DeclaredMemberSignature>()?;
            Some(PredicateSignature {
                signature: declared.signature.clone(),
                outer_type_arguments: declared.outer_type_arguments.clone(),
            })
        }
        PredicateCallee::Member { object, property } => {
            let diagnostics_before = ctx.diagnostics().len();
            let inferred = crate::infer::infer_expression(object, symbols, ctx);
            ctx.truncate_diagnostics(diagnostics_before);
            let crate::infer::InferredExpression::Known(object_ty) = inferred else {
                return None;
            };
            let member = match object_ty.peeled() {
                Type::Union(union) => union
                    .types()
                    .iter()
                    .find_map(|member| member.get_property_access_type(property))?,
                other => other.get_property_access_type(property)?,
            };
            let function = match member.peeled() {
                Type::Function(function) => function,
                Type::Union(union) => union.types().iter().find_map(|member| match member.peeled() {
                    Type::Function(function) => Some(function),
                    _ => None,
                })?,
                _ => return None,
            };
            let declared = function.declaration_handle().and_then(|declaration| {
                // A generic function exported from a module carries its own written
                // signature on the namespace object's member
                // (`modules::exports::namespace_member_type`).
                if let Ok(signature) = Arc::clone(declaration).downcast::<crate::symbols::FunctionSignatureInfo>() {
                    return Some(PredicateSignature::plain(signature));
                }
                let declared = (**declaration).downcast_ref::<crate::checks::call::DeclaredMemberSignature>()?;
                Some(PredicateSignature {
                    signature: declared.signature.clone(),
                    outer_type_arguments: declared.outer_type_arguments.clone(),
                })
            });
            match declared {
                Some(found)
                    if matches!(found.signature.return_type, Some(surge_ts_syntax::ParsedType::Predicate(_)))
                        || found.signature.inferred_predicate.is_some()
                        || found.signature.predicate_overload.is_some() =>
                {
                    Some(found)
                }
                declared => inferred_method_predicate_signature(&object_ty, property, &function, symbols, ctx).or(declared),
            }
        }
    }
}

/// tsc's `getTypePredicateFromBody` for an unannotated method of a
/// non-generic class: the predicate its body implies, read off the method the
/// receiver's class declares.
fn inferred_method_predicate_signature(
    receiver: &Type,
    property: &str,
    function: &surge_ts_types::FunctionType,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<PredicateSignature> {
    if !matches!(function.return_type().peeled(), Type::Boolean) {
        return None;
    }
    let class_name = match receiver {
        Type::Reference(reference) => reference.display.clone(),
        other => match other.peeled() {
            Type::Object(object) => object.alias_name.clone()?,
            _ => return None,
        },
    };
    let handle = ctx.lookup_type_declaration_handle(&class_name)?;
    let crate::symbols::TypeDeclarationInfo::Interface(class) = handle.get() else {
        return None;
    };
    if !class.is_class_instance || !class.body.type_parameters.is_empty() {
        return None;
    }
    let member = class.body.members.iter().find(|member| member.name == property)?;
    let surge_ts_syntax::ParsedType::Function(signature) = &member.ty else {
        return None;
    };
    let surge_ts_syntax::ParsedType::InferredMember(inferred) = signature.return_type.as_ref() else {
        return None;
    };
    let surge_ts_syntax::ParsedInferredMemberSource::MethodBody(method) = &inferred.source else {
        return None;
    };
    if method.is_static || method.is_async || method.is_generator || !method.type_parameters.is_empty() {
        return None;
    }
    let predicate = crate::checks::function::infer_predicate_from_function_body(
        &method.parameters,
        function.parameters(),
        &method.body,
        symbols,
        ctx,
    )?;
    let file_name = ctx.file_name.clone();
    Some(PredicateSignature::plain(crate::checks::function::with_inferred_predicate(
        crate::checks::function::function_signature_info(&[], &method.parameters, None, &file_name),
        Some(predicate),
    )))
}

/// Binds a generic predicate's type parameters for the guard site. A
/// non-generic predicate needs none. `subject_ty` is the type of the tested
/// argument itself — for `isDefined(this.x)` the type of `this.x` — which is
/// what tsc infers the predicate's type parameters from.
pub(super) fn predicate_type_argument_substitution(
    guard: &PredicateGuardInfo,
    subject_ty: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<crate::infer::TypeParameterSubstitution> {
    let mut substitution = seeded_predicate_substitution(guard);
    if guard.signature.type_parameters.is_empty() {
        return Some(substitution);
    }
    if !guard.explicit_type_arguments.is_empty() {
        return Some(explicit_predicate_type_argument_substitution(guard, ctx));
    }
    for type_parameter in &guard.signature.type_parameters {
        substitution.insert_placeholder(
            type_parameter.name.clone(),
            Type::type_parameter(&type_parameter.name),
        );
    }
    if let Some(subject_ty) = subject_ty
        && let Some(Some(parameter_type)) =
            guard.signature.parameter_types.get(guard.parameter_index)
    {
        crate::checks::call::collect_inferred_type_argument(
            parameter_type,
            subject_ty,
            &mut substitution,
            false,
            ctx,
            0,
        );
    }

    // Only the tested argument used to contribute, so a predicate that states
    // its target in terms of *another* argument (`isMatching(pattern, value)` is
    // `value is T & …narrow<T, P>…`, and `P` is the pattern) left that parameter
    // unbound and fell back to `Any` below — which then proves nothing and the
    // guard gives up. Each argument is inferred against its own declared
    // parameter, and only while something is still unbound.
    for (position, argument) in &guard.other_arguments {
        if !guard
            .signature
            .type_parameters
            .iter()
            .any(|type_parameter| substitution.is_placeholder(&type_parameter.name))
        {
            break;
        }
        let Some(Some(parameter_type)) = guard.signature.parameter_types.get(*position) else {
            continue;
        };
        let diagnostics_before = ctx.diagnostics().len();
        // An array literal handed to a `const` or tuple-constrained type
        // parameter is the tuple a call infers it as.
        let inferred = match crate::checks::call::array_literal_tuple_inference(
            parameter_type,
            &guard.signature.type_parameters,
            argument,
            symbols,
            ctx,
        ) {
            Some(tuple) => crate::infer::InferredExpression::Known(tuple),
            None => crate::infer::infer_expression(argument, symbols, ctx),
        };
        ctx.truncate_diagnostics(diagnostics_before);
        let crate::infer::InferredExpression::Known(argument_ty) = inferred else {
            continue;
        };
        if argument_ty.is_unknown() {
            continue;
        }
        let argument_ty = generic_class_value_surface(argument, &argument_ty, ctx)
            .unwrap_or(argument_ty);
        crate::checks::call::collect_inferred_type_argument(
            parameter_type,
            &argument_ty,
            &mut substitution,
            false,
            ctx,
            0,
        );
    }

    // A parameter written as an *alias* whose expansion is a union
    // (`x: SyncParseReturnType<T>` = `OK<T> | DIRTY<T> | INVALID`) leaves nothing
    // to align the alias's argument against, so `T` stays unbound. The predicate
    // is still usable: narrowing only ever *filters* the subject's own union
    // members, so binding the leftovers to `Any` selects the right member and
    // the members that survive keep their own precision.
    let unresolved: Vec<&surge_ts_syntax::ParsedTypeParameter> = guard
        .signature
        .type_parameters
        .iter()
        .filter(|type_parameter| substitution.is_placeholder(&type_parameter.name))
        .collect();
    if !unresolved.is_empty() {
        // Filling with `Any` is only sound while the predicate still *selects*
        // among the subject's union members, which needs the subject's type.
        // Without it (a property-path subject) an unbound parameter drops the
        // guard, as it always has.
        let subject_ty = subject_ty?;
        // Off a union there is nothing to select among: tsc infers a parameter
        // with no candidate as `unknown`, so `isAsyncIterable(value: unknown)`
        // narrows to `AsyncIterable<unknown>`.
        if !matches!(subject_ty.peeled(), Type::Union(_)) {
            let mut filled = substitution.clone_with_reason(TypeCopyReason::ScopeOrContext);
            for type_parameter in unresolved {
                filled.insert(type_parameter.name.clone(), Type::GenuineUnknown);
            }
            return Some(filled);
        }
        let mut filled = substitution.clone_with_reason(TypeCopyReason::ScopeOrContext);
        for type_parameter in unresolved {
            filled.insert(type_parameter.name.clone(), Type::Any);
        }
        return predicate_filters_subject_union(guard, &filled, subject_ty, ctx)
            .then_some(filled);
    }
    Some(substitution)
}

/// A constructor surface over the instance type of the class `argument` names,
/// for an argument whose own type cannot say which class it is.
///
/// A generic class's value side is deliberately `any` (see
/// [`crate::program::classes::build_class_value_symbol_with_scope`]): a real
/// static object over it was measured to open TS2351/TS2554 across zod, trpc and
/// ofetch. That `any` also erases the class identity, which is the whole content
/// of a guard written as `value is InstanceType<T>` with `T` inferred from the
/// class passed alongside — every such call bound `T` to something with no
/// construct signature, and the guard then either replaced the subject with
/// `Any` or proved nothing at all. A generic class merged with a namespace
/// (`class SQL` + `namespace SQL { class Aliased }`) lands in the same place
/// from the other side: its value is the namespace object, which carries the
/// members but no way to construct. Standing a constructor over the instance
/// type here recovers the identity for inference only; the value's own type is
/// untouched.
fn generic_class_value_surface(
    argument: &ParsedExpression,
    argument_ty: &Type,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let already_constructable = match argument_ty.peeled() {
        // A generic class's own static side constructs `any`.
        Type::Object(ref object) => object
            .construct_signature()
            .is_some_and(|signature| !matches!(signature.return_type(), Type::Any)),
        _ => !matches!(argument_ty, Type::Any),
    };
    if already_constructable {
        return None;
    }
    let name = class_reference_name(argument)?;
    // Only a *generic* class reaches here without a constructor; a non-generic
    // one already carries its static object, and requiring type parameters keeps
    // an unrelated value that happens to share a name with a type declaration
    // out.
    let handle = ctx.lookup_type_declaration_handle(&name)?;
    let crate::symbols::TypeDeclarationInfo::Interface(interface) = handle.get() else {
        return None;
    };
    if interface.body.type_parameters.is_empty() {
        return None;
    }
    // Each type parameter is filled with `any` rather than left off: the class
    // is being named for its identity, not its arguments, so `SQL<any>` selects
    // `SQL<number>` out of the subject's union while claiming nothing about the
    // argument. Leaving them off resolves the same shape but reports TS2314 at
    // the reference, which is why the resolution runs with diagnostics dropped.
    let type_arguments =
        vec![surge_ts_syntax::ParsedType::Any; interface.body.type_parameters.len()];
    let diagnostics_before = ctx.diagnostics().len();
    let instance = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        crate::infer::types::resolve_parsed_type(
            surge_ts_syntax::ParsedType::Named(Arc::new(surge_ts_syntax::ParsedNamedType {
                type_argument_spans: Vec::new(),
                name,
                span: None,
                type_arguments,
            })),
            ctx,
            &mut Vec::new(),
            &crate::infer::TypeParameterSubstitution::new(),
        )
    });
    ctx.truncate_diagnostics(diagnostics_before);
    if instance.had_error() {
        return None;
    }
    let instance = instance.into_ty();
    if instance.is_unknown() {
        return None;
    }
    let mut properties = surge_ts_types::PropertyMap::default();
    properties.insert(
        "prototype".into(),
        surge_ts_types::ObjectProperty::required(instance.clone()),
    );
    let constructor = surge_ts_types::FunctionType::new(vec![Type::Any], instance, true, 0);
    Some(Type::Object(
        surge_ts_types::ObjectType::new(properties, None)
            .with_open_index_marker()
            .with_construct_signature(constructor),
    ))
}

/// The dotted type name an expression references, for a bare identifier or a
/// path of them (`SQL.Aliased` names the class inside the namespace `SQL` is
/// merged with). Anything else is not a class reference.
fn class_reference_name(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        } => Some(format!(
            "{}.{property_name}",
            class_reference_name(object)?
        )),
        _ => None,
    }
}

/// Whether an `Any`-filled predicate would narrow `subject_ty` by *selecting*
/// among its union members rather than by replacing it wholesale. Only the
/// selecting outcome is sound when the predicate's type arguments are unknown:
/// [`super::guards::narrow_by_predicate`] substitutes the predicate itself when
/// no member matches or the subject is not a union, which would install the
/// `Any` fillers as real types.
pub(super) fn predicate_filters_subject_union(
    guard: &PredicateGuardInfo,
    substitution: &crate::infer::TypeParameterSubstitution,
    subject_ty: &Type,
    ctx: &mut CheckerContext,
) -> bool {
    let Type::Union(union) = subject_ty.peeled() else {
        return false;
    };
    let Some(predicate_ty) = resolve_predicate_type_in_declaring_scope(guard, substitution, ctx)
    else {
        return false;
    };
    let members = union.types();
    let matching = members
        .iter()
        .filter(|member| surge_ts_types::is_assignable_to(member, &predicate_ty))
        .count();
    matching > 0 && matching < members.len()
}

/// The target type of a value that *is* a type predicate (`isFoo` passed to
/// `filter`), resolved in the signature's declaring scope. `None` when the value
/// carries no collected signature, its return is not a predicate, or the
/// predicate is generic — a `T` there needs a guard site to infer against.
pub(crate) fn predicate_target_of_value(
    name: &str,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let signature = symbols.get(name)?.function_signature.clone()?;
    if !signature.type_parameters.is_empty() {
        return None;
    }
    if signature.return_type.is_none()
        && let Some(inferred) = &signature.inferred_predicate
    {
        return (inferred.parameter_index == 0).then(|| inferred.target.clone());
    }
    let surge_ts_syntax::ParsedType::Predicate(predicate) = signature.return_type.as_ref()? else {
        return None;
    };
    if predicate.asserts {
        return None;
    }
    let guard = PredicateGuardInfo {
        subject: String::new(),
        path: Vec::new(),
        other_arguments: Vec::new(),
        predicate_type: predicate.ty.clone()?,
        declaring_file: signature.declaring_file.clone(),
        namespace_prefix: signature.namespace_prefix.clone(),
        parameter_index: 0,
        signature,
        explicit_type_arguments: Vec::new(),
        outer_type_arguments: Vec::new(),
    };
    resolve_predicate_type_in_declaring_scope(&guard, &crate::infer::TypeParameterSubstitution::new(), ctx)
}

/// Binds a generic predicate's type parameters from the type arguments written
/// at the call, resolved in the call site's scope. A parameter the call leaves
/// out takes its default, and a written argument surge cannot resolve binds
/// the degradation sentinel and marks the parameter degraded, so the filled
/// predicate is treated as degraded rather than narrowed to.
fn explicit_predicate_type_argument_substitution(
    guard: &PredicateGuardInfo,
    ctx: &mut CheckerContext,
) -> crate::infer::TypeParameterSubstitution {
    let mut substitution = seeded_predicate_substitution(guard);
    let scope = crate::infer::types::merged_type_parameter_substitution(ctx, &substitution);
    for (type_parameter, argument) in guard
        .signature
        .type_parameters
        .iter()
        .zip(guard.explicit_type_arguments.iter())
    {
        let resolved = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            crate::infer::types::resolve_parsed_type(
                argument.clone(),
                ctx,
                &mut Vec::new(),
                &scope,
            )
        });
        let degraded = resolved.had_error();
        let ty = resolved.into_ty();
        let degraded = degraded || matches!(ty, Type::Unknown | Type::TypeParameter(_));
        substitution.insert(type_parameter.name.clone(), if degraded { Type::Unknown } else { ty });
        if degraded {
            substitution.mark_degraded(&type_parameter.name);
        }
    }
    for type_parameter in guard
        .signature
        .type_parameters
        .iter()
        .skip(guard.explicit_type_arguments.len())
    {
        match &type_parameter.default_type {
            Some(default_type) => {
                let resolved = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    crate::infer::types::resolve_parsed_type(
                        default_type.clone(),
                        ctx,
                        &mut Vec::new(),
                        &substitution,
                    )
                });
                let degraded = resolved.had_error();
                let ty = resolved.into_ty();
                substitution.insert(type_parameter.name.clone(), if degraded { Type::Unknown } else { ty });
                if degraded {
                    substitution.mark_degraded(&type_parameter.name);
                }
            }
            None => {
                substitution.insert(type_parameter.name.clone(), Type::Unknown);
                substitution.mark_degraded(&type_parameter.name);
            }
        }
    }
    substitution
}

/// What a predicate guard proves at its site.
pub(super) enum PredicateTarget {
    Resolved(Type),
    /// The target could not be modelled — a degraded resolution or a type
    /// argument surge could not reconstruct. tsc narrows the subject to a type
    /// surge does not have, so the holding branch must not keep reasoning from
    /// the declared type: it reads the subject as the degradation sentinel.
    Degraded,
}

/// [`resolve_predicate_guard_type`], but keeping apart a target that degraded
/// from a guard that cannot be evaluated at all (no subject type, unbound
/// generic), so a caller can open the subject in the holding branch.
pub(super) fn resolve_predicate_guard_target(
    guard: &PredicateGuardInfo,
    subject_ty: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<PredicateTarget> {
    let substitution = predicate_type_argument_substitution(guard, subject_ty, symbols, ctx)?;
    let explicit_degraded = guard
        .signature
        .type_parameters
        .iter()
        .any(|type_parameter| substitution.is_degraded(&type_parameter.name));
    if explicit_degraded {
        return Some(PredicateTarget::Degraded);
    }
    let has_explicit = !guard.explicit_type_arguments.is_empty();
    match resolve_predicate_type_in_declaring_scope(guard, &substitution, ctx) {
        Some(ty) => Some(PredicateTarget::Resolved(ty)),
        None if has_explicit => Some(PredicateTarget::Degraded),
        None => None,
    }
}

/// The narrowed subject for a degraded target: the holding branch opens it,
/// the other branch keeps it.
pub(super) fn degraded_predicate_subject(branch_is_true: bool) -> Option<Type> {
    branch_is_true.then_some(Type::Unknown)
}

/// Resolves a predicate guard's target type under the predicate's declaring
/// file (see [`crate::symbols::FunctionSignatureInfo::declaring_file`]). A
/// resolution that degrades (`had_error` or the `Unknown` sentinel) proves
/// nothing — narrowing on it would manufacture facts from a modeling gap — so
/// it yields `None`. A generic predicate needs its `T` bound first, inferred
/// from `subject_ty`; without a subject type, or when a type parameter stays
/// unbound and the filled predicate would not merely select among the subject's
/// union members, the guard is dropped.
pub(super) fn resolve_predicate_guard_type(
    guard: &PredicateGuardInfo,
    subject_ty: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let substitution = predicate_type_argument_substitution(guard, subject_ty, symbols, ctx)?;
    resolve_predicate_type_in_declaring_scope(guard, &substitution, ctx)
}

/// Resolves `guard.predicate_type` under the signature's declaring file and
/// namespace prefix, with `substitution` bound for its type parameters.
pub(super) fn resolve_predicate_type_in_declaring_scope(
    guard: &PredicateGuardInfo,
    substitution: &crate::infer::TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !matches!(
        guard.signature.return_type,
        Some(surge_ts_syntax::ParsedType::Predicate(_))
    ) && let Some(inferred) = &guard.signature.inferred_predicate
    {
        return Some(inferred.target.clone());
    }
    resolve_predicate_type_under(
        guard.predicate_type.clone(),
        guard.declaring_file.as_deref(),
        guard.namespace_prefix.as_deref(),
        substitution,
        ctx,
    )
}

/// Resolves a predicate's written target type under the file (and namespace)
/// that declared it. A resolution that degrades proves nothing, so it is
/// dropped rather than narrowed to.
pub(super) fn resolve_predicate_type_under(
    predicate_type: surge_ts_syntax::ParsedType,
    declaring_file: Option<&str>,
    namespace_prefix: Option<&str>,
    substitution: &crate::infer::TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let declaring_file = declaring_file
        .filter(|file| *file != ctx.file_name)
        .map(str::to_string);
    let saved_file_name = declaring_file.map(|file| {
        let saved = ctx.file_name.clone();
        ctx.set_file_name(file);
        saved
    });
    if let Some(prefix) = namespace_prefix {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix.to_string());
    }
    // The predicate's target names whatever is in scope where the predicate was
    // *declared*, which includes an enclosing generic's type parameters
    // (`function isTarget(n: any): n is TFunc` inside `outer<TFunc>`). Those live
    // on the scope stack, not in the guard's own substitution, so resolving
    // against the substitution alone reported the enclosing parameter as an
    // unresolved name — at the declaration's span, long after the declaration
    // itself checked clean.
    let substitution = crate::infer::types::merged_type_parameter_substitution(ctx, substitution);
    let resolved = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        crate::infer::types::resolve_parsed_type(predicate_type, ctx, &mut Vec::new(), &substitution)
    });
    if namespace_prefix.is_some() {
        ctx.namespace_member_prefix_stack.pop();
        ctx.namespace_member_resolution_depth -= 1;
    }
    if let Some(saved) = saved_file_name {
        ctx.set_file_name(saved);
    }
    if resolved.had_error() {
        return None;
    }
    let ty = resolved.into_ty();
    // A type variable of the body being checked (`Extract<T, Function>` over
    // the caller's `T`) is a real target; a placeholder is not.
    (!matches!(ty, Type::Unknown) && (!matches!(ty, Type::TypeParameter(_)) || ty.is_type_variable()))
        .then_some(ty)
}

/// A zero-argument method call (`type.isUnion()`) plus the reference it is
/// called on, the shape a `this is T` predicate guards.
pub(super) fn parse_this_predicate_call(
    condition: &ParsedExpression,
) -> Option<(String, Vec<String>, &str)> {
    // A `this is T` predicate ignores the call's arguments.
    let ParsedExpression::PropertyCall {
        object,
        property_name,
        type_arguments,
        ..
    } = condition
    else {
        return None;
    };
    if !type_arguments.is_empty() {
        return None;
    }
    let (subject, path) = reference_path(object)?;
    Some((subject, path, property_name.as_str()))
}

/// The resolved target of a `this is T` method predicate the receiver's
/// member declares: the one its own interface declares
/// (`interface Type { isUnion(): this is UnionType }`), or the one the member
/// signature it resolves to carries — inherited from a base class or
/// interface, or merged into an intersection — as tsc reads the predicate off
/// the call's resolved signature (`getEffectsSignature`).
///
/// Predicates are dropped when a signature resolves to a type — `x is T` and
/// `this is T` both resolve to `boolean` — so the declaration has to be read
/// back.
pub(super) fn this_predicate_target(
    receiver_ty: &Type,
    method: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if let Type::Union(union) = receiver_ty {
        return union_this_predicate_target(union.types(), method, ctx);
    }
    own_this_predicate_target(receiver_ty, method, ctx)
        .or_else(|| member_this_predicate_target(receiver_ty, method, ctx))
}

/// tsc's `getUnionOrIntersectionTypePredicate` over a union receiver's method:
/// every member's signature has a `this is T` predicate, or returns `false`
/// (permitted and ignored), and the composite predicate is the union of the
/// targets.
fn union_this_predicate_target(members: &[Type], method: &str, ctx: &mut CheckerContext) -> Option<Type> {
    let mut targets = Vec::new();
    // An optional chain's call reads its signature off the non-nullable part.
    for member in members.iter().filter(|member| !matches!(member, Type::Undefined | Type::Null)) {
        if let Some(target) = own_this_predicate_target(member, method, ctx)
            .or_else(|| member_this_predicate_target(member, method, ctx))
        {
            targets.push(target);
            continue;
        }
        let returns_false = matches!(
            member.get_property_access_type(method).map(|ty| ty.peeled()),
            Some(Type::Function(function)) if matches!(function.return_type(), Type::BooleanLiteral(false))
        );
        if !returns_false {
            return None;
        }
    }
    (!targets.is_empty()).then(|| surge_ts_types::union_type(targets))
}

/// A `this is T` predicate on the written signature a member's resolved type
/// carries (`DeclaredMemberSignature`), with the enclosing bindings it was
/// resolved under.
fn member_this_predicate_target(
    receiver_ty: &Type,
    method: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Function(function) = receiver_ty.get_property_access_type(method)?.peeled() else {
        return None;
    };
    let declared = function
        .declaration()?
        .downcast_ref::<crate::checks::call::DeclaredMemberSignature>()?;
    let Some(surge_ts_syntax::ParsedType::Predicate(predicate)) = &declared.signature.return_type
    else {
        return None;
    };
    // A generic method's predicate names type parameters the call infers,
    // which this reading does not bind.
    if predicate.asserts || predicate.parameter_name != "this" || !declared.signature.type_parameters.is_empty() {
        return None;
    }
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for (name, ty) in &declared.outer_type_arguments {
        substitution.insert(name.clone(), ty.clone());
    }
    resolve_predicate_type_under(
        predicate.ty.clone()?,
        declared.signature.declaring_file.as_deref(),
        declared.signature.namespace_prefix.as_deref(),
        &substitution,
        ctx,
    )
}

fn own_this_predicate_target(
    receiver_ty: &Type,
    method: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // A constructed instance (`connection = new Connection()`) is the resolved
    // object, which keeps its declaration's identity in the same `file\0name`
    // form a nominal reference id uses.
    let id = match receiver_ty {
        Type::Reference(reference) => reference.id.clone(),
        Type::Object(object) => object.alias_id.clone()?,
        _ => return None,
    };
    let id = id.as_ref();
    let separator = id.rfind('\u{0}')?;
    let (declaring_file, declaration_name) = (&id[..separator], &id[separator + 1..]);
    let declaration_name = declaration_name.to_string();

    let found = {
        let crate::symbols::TypeDeclarationInfo::Interface(info) =
            ctx.lookup_type_declaration(&declaration_name)?
        else {
            return None;
        };
        if info.file_name.as_ref() != declaring_file {
            return None;
        }
        let member = info.body.members.iter().find(|member| member.name == method)?;
        let surge_ts_syntax::ParsedType::Function(function) = &member.ty else {
            return None;
        };
        let surge_ts_syntax::ParsedType::Predicate(predicate) = function.return_type.as_ref()
        else {
            return None;
        };
        if predicate.asserts || predicate.parameter_name != "this" || !function.type_parameters.is_empty() {
            return None;
        }
        (predicate.ty.clone()?, info.file_name.to_string())
    };
    let (predicate_type, declaring_file) = found;
    // A predicate written inside `declare namespace ts` names `ts.UnionType`,
    // which only resolves under that prefix.
    let namespace_prefix = declaration_name
        .rfind('.')
        .map(|dot| declaration_name[..dot].to_string());
    resolve_predicate_type_under(
        predicate_type,
        Some(declaring_file.as_str()),
        namespace_prefix.as_deref(),
        &crate::infer::TypeParameterSubstitution::new(),
        ctx,
    )
}

/// The reference a predicate tests — the argument of `isDefined(this.x)`, or
/// the object a `this is T` method is called on
/// (`this.activeConnection.isOpen()`): the subject itself, or the member its
/// path reaches (tsc's `getTypePredicateArgument`).
fn predicate_argument_type(subject_ty: &Type, path: &[String]) -> Option<Type> {
    if path.is_empty() {
        return Some(subject_ty.clone());
    }
    super::reference::property_path_leaf_type(subject_ty, path)
}

/// Narrows `symbols` by a `this is T` method predicate, or `None` when the
/// condition is not one (or it proves nothing new).
pub(super) fn narrow_this_predicate_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    let (subject, path, method) = parse_this_predicate_call(condition)?;
    let symbol = symbols.get(&subject)?;
    let subject_ty = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let target = this_predicate_target(&predicate_argument_type(&subject_ty, &path)?, method, ctx)?;
    let narrowed = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        narrowed_predicate_subject(&subject_ty, &path, &target, branch_is_true)
    })?;
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        subject,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        subject_ty,
    );
    Some(narrowed_symbols)
}

/// The `ScopeStack` counterpart of [`narrow_this_predicate_symbol_table`].
pub(super) fn narrow_this_predicate_call_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> bool {
    let Some((subject, path, method)) = parse_this_predicate_call(condition) else {
        return false;
    };
    let Some(symbol) = scopes.resolve(&subject) else {
        return false;
    };
    let subject_ty = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let Some(target) = predicate_argument_type(&subject_ty, &path)
        .and_then(|receiver| this_predicate_target(&receiver, method, ctx))
    else {
        return false;
    };
    let Some(narrowed) = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        narrowed_predicate_subject(&subject_ty, &path, &target, branch_is_true)
    }) else {
        return true;
    };
    let _ = scopes.insert_current_narrowed(
        subject,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        subject_ty,
    );
    true
}

/// Whether `expression` calls a `param is T` predicate with a reference in the
/// tested position — the one call shape that narrows as a condition.
pub(crate) fn is_type_predicate_call(
    expression: &ParsedExpression,
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) -> bool {
    let arguments = match expression {
        ParsedExpression::Call { arguments, .. }
        | ParsedExpression::PropertyCall { arguments, .. } => arguments,
        _ => return false,
    };
    if !arguments
        .iter()
        .any(|argument| reference_path(&argument.expression).is_some())
    {
        return false;
    }
    parse_type_predicate_condition(expression, &mut |callee| {
        predicate_callee_signature(callee, |name| scopes.resolve(name), scopes.visible_symbols(), ctx)
    })
    .is_some()
}

/// What `guard`'s predicate narrows to, its argument read off the subject's
/// path or, for an optional chain (`isDefined(o?.x)`) that has no path type off
/// a nullable `o`, inferred from the argument itself as tsc does.
fn predicate_guard_target_in_scope(
    condition: &ParsedExpression,
    guard: &PredicateGuardInfo,
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) -> Option<PredicateTarget> {
    let argument_ty = scopes
        .resolve(&guard.subject)
        .and_then(|symbol| predicate_argument_type(&symbol.ty, &guard.path))
        .or_else(|| {
            call_argument(condition, guard.parameter_index)
                .and_then(|argument| inferred_argument_type(argument, scopes.visible_symbols(), ctx))
        });
    resolve_predicate_guard_target(guard, argument_ty.as_ref(), scopes.visible_symbols(), ctx)
}

/// The optional chains a predicate call that holds reads through
/// (`isFoo(o?.x)` proves `o` present), for a predicate that is one operand of
/// a logical condition the caller knows holds.
pub(super) fn narrow_predicate_chain_containment_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
        predicate_callee_signature(callee, |name| scopes.resolve(name), scopes.visible_symbols(), ctx)
    }) else {
        return;
    };
    if let Some(PredicateTarget::Resolved(predicate_ty)) = predicate_guard_target_in_scope(condition, &guard, scopes, ctx)
        && let Some(argument) = call_argument(condition, guard.parameter_index)
    {
        narrow_optional_chain_containment(argument, &predicate_ty, scopes);
    }
}

/// Applies user-defined type-predicate narrowing (`isFoo(x)`) in place to a
/// `ScopeStack`. Returns whether the condition was such a predicate call over a
/// bare-identifier argument.
pub(super) fn narrow_predicate_call_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> bool {
    let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
        predicate_callee_signature(callee, |name| scopes.resolve(name), scopes.visible_symbols(), ctx)
    }) else {
        return false;
    };
    let target = predicate_guard_target_in_scope(condition, &guard, scopes, ctx);
    if branch_is_true
        && let Some(PredicateTarget::Resolved(predicate_ty)) = &target
        && let Some(argument) = call_argument(condition, guard.parameter_index)
    {
        narrow_optional_chain_containment(argument, predicate_ty, scopes);
    }
    let Some(symbol) = scopes.resolve(&guard.subject) else {
        return true;
    };
    let subject_ty = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let narrowed = match target {
        Some(PredicateTarget::Resolved(predicate_ty)) => {
            with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                narrowed_predicate_subject(&subject_ty, &guard.path, &predicate_ty, branch_is_true)
            })
        }
        Some(PredicateTarget::Degraded) if guard.path.is_empty() => {
            degraded_predicate_subject(branch_is_true)
        }
        _ => None,
    };
    let Some(narrowed) = narrowed else {
        return true;
    };
    let _ = scopes.insert_current_narrowed(
        guard.subject,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        subject_ty,
    );
    true
}

/// The value names a function body (with its parameters) or a file declares,
/// and which of them are declared without a type annotation: tsc's
/// `getExplicitTypeOfSymbol` reads a variable, parameter or property only
/// through its annotation, and a function, class or namespace as it is.
#[derive(Debug, Default, Clone)]
pub(crate) struct ExplicitTypeScope {
    declared: std::collections::HashSet<String>,
    inferred: std::collections::HashSet<String>,
}

impl ExplicitTypeScope {
    pub(crate) fn of_function(
        parameters: &[surge_ts_syntax::ParsedFunctionParameter],
        body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    ) -> Self {
        let mut scope = Self::default();
        let mut annotated = std::collections::HashSet::new();
        for parameter in parameters {
            let plain = matches!(parameter.binding_name, surge_ts_syntax::ParsedBindingName::Identifier { .. });
            for bound in parameter.binding_name.bound_names() {
                if plain && parameter.declared_type.is_some() {
                    annotated.insert(bound.name.clone());
                } else {
                    scope.inferred.insert(bound.name.clone());
                }
                scope.declared.insert(bound.name);
            }
        }
        scope.collect_body(body, &mut annotated);
        scope.inferred.retain(|name| !annotated.contains(name));
        scope
    }

    pub(crate) fn of_module(statements: &[surge_ts_syntax::ParsedStatement]) -> Self {
        use surge_ts_syntax::ParsedStatement;
        let mut scope = Self::default();
        let mut annotated = std::collections::HashSet::new();
        for statement in statements {
            let statement = match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                    _ => continue,
                },
                other => other,
            };
            match statement {
                ParsedStatement::VariableDeclaration(variable) => scope.declare_variable(variable, &mut annotated),
                ParsedStatement::FunctionDeclaration(function) => {
                    scope.declared.insert(function.name.clone());
                }
                ParsedStatement::ClassDeclaration(class) => {
                    scope.declared.insert(class.name.clone());
                }
                _ => {}
            }
        }
        scope.inferred.retain(|name| !annotated.contains(name));
        scope
    }

    fn declare_variable(
        &mut self,
        variable: &surge_ts_syntax::ParsedVariableDeclaration,
        annotated: &mut std::collections::HashSet<String>,
    ) {
        self.declared.insert(variable.name.clone());
        if variable.declared_type.is_some() && !variable.from_binding_pattern {
            annotated.insert(variable.name.clone());
        } else {
            self.inferred.insert(variable.name.clone());
        }
    }

    fn collect_body(
        &mut self,
        body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
        annotated: &mut std::collections::HashSet<String>,
    ) {
        use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
        for statement in body {
            match statement {
                Statement::VariableDeclaration(variable) => self.declare_variable(variable, annotated),
                Statement::Function(function) => {
                    self.declared.insert(function.name.clone());
                }
                Statement::Class(class) => {
                    self.declared.insert(class.name.clone());
                }
                Statement::Block(block) => self.collect_body(block, annotated),
                Statement::If(statement) => {
                    self.collect_body(&statement.then_body, annotated);
                    self.collect_body(&statement.else_body, annotated);
                }
                Statement::While(statement) => self.collect_body(&statement.body, annotated),
                // A `for…of` binding is read through its iterated expression,
                // which this walk does not judge: taken as explicit.
                Statement::ForOf(statement) => {
                    for bound in statement.binding_name.bound_names() {
                        self.declared.insert(bound.name);
                    }
                    self.collect_body(&statement.body, annotated);
                }
                Statement::Switch(statement) => {
                    for case in &statement.cases {
                        self.collect_body(&case.consequent, annotated);
                    }
                }
                Statement::Try(statement) => {
                    self.collect_body(&statement.block, annotated);
                    if let Some(handler) = &statement.handler {
                        if let Some(binding) = &handler.binding_name {
                            for bound in binding.bound_names() {
                                if handler.declared_type.is_none() {
                                    self.inferred.insert(bound.name.clone());
                                }
                                self.declared.insert(bound.name);
                            }
                        }
                        self.collect_body(&handler.body, annotated);
                    }
                    self.collect_body(&statement.finalizer, annotated);
                }
                _ => {}
            }
        }
    }
}

/// Whether the value binding `name` names is declared with an explicit type,
/// read from the innermost body (or the file) that declares it. `None` for a
/// name none of them declares.
pub(crate) fn binding_has_explicit_type(name: &str, ctx: &CheckerContext) -> Option<bool> {
    ctx.assertion_binding_scopes
        .iter()
        .rev()
        .chain(ctx.module_assertion_bindings.iter())
        .find(|scope| scope.declared.contains(name))
        .map(|scope| !scope.inferred.contains(name))
}

/// tsc's `getTypeOfDottedName` for an assertion's call target: the span of
/// the target when a name in it lacks an explicit type, so the call has no
/// effects signature (TS2775).
fn assertion_target_without_explicit_type(
    expression: &ParsedExpression,
    ctx: &CheckerContext,
) -> Option<surge_ts_syntax::TextSpan> {
    match expression {
        ParsedExpression::Call { callee_name, callee_span, .. } => {
            (binding_has_explicit_type(callee_name, ctx) == Some(false)).then_some(*callee_span).flatten()
        }
        ParsedExpression::PropertyCall { object, property_name, .. } => {
            let mut head = object.as_ref();
            while let ParsedExpression::PropertyAccess { object, is_bracketed: false, .. } = head {
                head = object.as_ref();
            }
            match head {
                ParsedExpression::Identifier { name, span } => {
                    (binding_has_explicit_type(name, ctx) == Some(false)).then_some(*span).flatten()
                }
                ParsedExpression::This { span } if std::ptr::eq(head, object.as_ref()) => ctx
                    .unannotated_class_properties
                    .last()
                    .is_some_and(|properties| properties.contains(property_name))
                    .then_some(*span)
                    .flatten(),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether a function value carries an `asserts` signature of its own.
fn asserting_function(ty: &Type) -> bool {
    let Type::Function(function) = ty.peeled() else {
        return false;
    };
    let return_type = function.declaration().and_then(|declaration| {
        if let Some(signature) = declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>() {
            signature.return_type.clone()
        } else {
            declaration
                .downcast_ref::<crate::checks::call::DeclaredMemberSignature>()
                .and_then(|declared| declared.signature.return_type.clone())
        }
    });
    matches!(return_type, Some(surge_ts_syntax::ParsedType::Predicate(predicate)) if predicate.asserts)
}

/// tsc's `checkCallExpression` for an assertion called through anything but
/// an identifier or qualified name (`fns[0](x)`): TS2776 on the target.
pub(crate) fn report_assertion_call_target(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let ParsedExpression::ExpressionCall { callee, callee_span, .. } = expression else {
        return;
    };
    if matches!(callee.as_ref(), ParsedExpression::ArrowFunction(_)) {
        return;
    }
    let checkpoint = ctx.diagnostics().len();
    let callee_type = crate::infer::infer_expression(callee, symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    if let crate::infer::InferredExpression::Known(callee_type) = callee_type
        && asserting_function(&callee_type)
    {
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            surge_ts_diagnostics::Diagnostic::ts2776(ctx.file_name.clone()),
            *callee_span,
        ));
    }
}

/// Applies an assertion call's narrowing (`assertIsObject(obj);`) to the
/// enclosing scope. Unlike a `x is T` predicate, an `asserts x is T` signature
/// narrows from the *statement* onward rather than inside a branch, so it is
/// applied where the expression statement is checked.
pub(crate) fn narrow_assertion_call_in_scope(
    expression: &ParsedExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    // A value declared with a callable interface type (`declare const assert:
    // Chai.Assert`) carries no collected signature of its own; the written call
    // signature is read off the declaration instead. A static assertion
    // (`ZodError.assert(err)`) is a member call and reads its signature off
    // the receiver's type the way a member predicate guard does.
    let (found, type_arguments, arguments) = match expression {
        ParsedExpression::Call {
            callee_name,
            type_arguments,
            arguments,
            ..
        } => {
            let Some(found) = predicate_callee_signature(
                PredicateCallee::Name(callee_name),
                |name| scopes.resolve(name),
                scopes.visible_symbols(),
                ctx,
            ) else {
                return;
            };
            (found, type_arguments, arguments)
        }
        ParsedExpression::PropertyCall {
            object,
            property_name,
            type_arguments,
            arguments,
            ..
        } => {
            let qualified = if let ParsedExpression::Identifier { name, .. } = object.as_ref() {
                predicate_callee_signature(
                    PredicateCallee::Name(&format!("{name}.{property_name}")),
                    |name| scopes.resolve(name),
                    scopes.visible_symbols(),
                    ctx,
                )
            } else {
                None
            };
            let found = match qualified {
                Some(found) => found,
                None => {
                    let Some(found) = predicate_callee_signature(
                        PredicateCallee::Member {
                            object,
                            property: property_name,
                        },
                        |name| scopes.resolve(name),
                        scopes.visible_symbols(),
                        ctx,
                    ) else {
                        return;
                    };
                    found
                }
            };
            (found, type_arguments, arguments)
        }
        _ => return,
    };
    let PredicateSignature {
        signature,
        outer_type_arguments,
    } = found;
    let Some(surge_ts_syntax::ParsedType::Predicate(predicate)) = signature.return_type.as_ref()
    else {
        return;
    };
    if !predicate.asserts {
        return;
    }
    // The call has no effects signature then, so it asserts nothing.
    if let Some(span) = assertion_target_without_explicit_type(expression, ctx) {
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            surge_ts_diagnostics::Diagnostic::ts2775(ctx.file_name.clone()),
            Some(span),
        ));
        return;
    }
    // tsc's `getTypePredicateArgument`: an `asserts this` signature asserts
    // the receiver of the method call.
    if predicate.parameter_name == "this" {
        let ParsedExpression::PropertyCall { object, .. } = expression else {
            return;
        };
        let Some(predicate_type) = predicate.ty.clone() else {
            narrow_discriminant_in_scope(object, scopes, true, ctx);
            return;
        };
        if !signature.type_parameters.is_empty() || !type_arguments.is_empty() {
            return;
        }
        let Some((subject, path)) = reference_path(object) else {
            return;
        };
        let mut substitution = crate::infer::TypeParameterSubstitution::new();
        for (name, ty) in &outer_type_arguments {
            substitution.insert(name.clone(), ty.clone());
        }
        let Some(target) = resolve_predicate_type_under(
            predicate_type,
            signature.declaring_file.as_deref(),
            signature.namespace_prefix.as_deref(),
            &substitution,
            ctx,
        ) else {
            return;
        };
        narrow_asserted_reference(subject, &path, &target, scopes);
        return;
    }
    let Some(index) = signature
        .parameter_names
        .iter()
        .position(|name| name.as_deref() == Some(predicate.parameter_name.as_str()))
    else {
        return;
    };
    // A generic assertion binds its type parameters at the call site the way
    // a `x is T` guard does (from the written type arguments, else inferred
    // from the arguments), so it resolves through the guard machinery.
    if (!signature.type_parameters.is_empty() || !type_arguments.is_empty())
        && let Some(predicate_type) = predicate.ty.clone()
    {
        let Some(argument) = arguments.get(index) else {
            return;
        };
        let reference = reference_path(&argument.expression);
        let (subject, path) = reference.clone().unwrap_or_default();
        let other_arguments = if type_arguments.is_empty() {
            arguments
                .iter()
                .enumerate()
                .filter(|(position, _)| *position != index)
                .map(|(position, argument)| (position, argument.expression.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let guard = PredicateGuardInfo {
            subject,
            path,
            other_arguments,
            predicate_type,
            declaring_file: signature.declaring_file.clone(),
            namespace_prefix: signature.namespace_prefix.clone(),
            parameter_index: index,
            signature: signature.clone(),
            explicit_type_arguments: type_arguments.clone(),
            outer_type_arguments,
        };
        let argument_ty = reference
            .as_ref()
            .and_then(|_| scopes.resolve(&guard.subject))
            .and_then(|symbol| predicate_argument_type(&symbol.ty, &guard.path))
            .or_else(|| inferred_argument_type(&argument.expression, scopes.visible_symbols(), ctx));
        let target = resolve_predicate_guard_target(&guard, argument_ty.as_ref(), scopes.visible_symbols(), ctx);
        if let Some(PredicateTarget::Resolved(target)) = target {
            narrow_optional_chain_containment(&argument.expression, &target, scopes);
            if reference.is_some() {
                narrow_asserted_reference(guard.subject, &guard.path, &target, scopes);
            }
        }
        return;
    }
    if predicate.ty.is_some() && (!type_arguments.is_empty() || !signature.type_parameters.is_empty()) {
        return;
    }
    let Some(argument) = arguments.get(index) else {
        return;
    };
    let Some((subject, path)) = reference_path(&argument.expression) else {
        // `asserts condition` over an arbitrary expression holds from the
        // statement on, exactly as the true branch of `if (condition)` does.
        match predicate.ty.clone() {
            None => narrow_discriminant_in_scope(&argument.expression, scopes, true, ctx),
            Some(predicate_type) => {
                let mut substitution = crate::infer::TypeParameterSubstitution::new();
                for (name, ty) in &outer_type_arguments {
                    substitution.insert(name.clone(), ty.clone());
                }
                if let Some(target) = resolve_predicate_type_under(
                    predicate_type,
                    signature.declaring_file.as_deref(),
                    signature.namespace_prefix.as_deref(),
                    &substitution,
                    ctx,
                ) {
                    narrow_optional_chain_containment(&argument.expression, &target, scopes);
                }
            }
        }
        return;
    };
    // `asserts x` with no target proves only that `x` is truthy.
    let Some(predicate_type) = predicate.ty.clone() else {
        let Some(symbol) = scopes.resolve(&subject) else {
            return;
        };
        let subject_ty = symbol.ty.clone();
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        if !path.is_empty() {
            narrow_discriminant_in_scope(&argument.expression, scopes, true, ctx);
        } else {
            let narrowed = surge_ts_types::remove_nullish(&subject_ty);
            if narrowed != subject_ty {
                let _ = scopes.insert_current_narrowed(
                    subject,
                    SymbolInfo {
                        ty: narrowed,
                        kind,
                        function_signature,
                    },
                    subject_ty,
                );
            }
        }
        return;
    };
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for (name, ty) in &outer_type_arguments {
        substitution.insert(name.clone(), ty.clone());
    }
    let Some(target) = resolve_predicate_type_under(
        predicate_type,
        signature.declaring_file.as_deref(),
        signature.namespace_prefix.as_deref(),
        &substitution,
        ctx,
    ) else {
        return;
    };
    narrow_optional_chain_containment(&argument.expression, &target, scopes);
    narrow_asserted_reference(subject, &path, &target, scopes);
}

fn inferred_argument_type(argument: &ParsedExpression, symbols: &SymbolTable, ctx: &mut CheckerContext) -> Option<Type> {
    let diagnostics_before = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(argument, symbols, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    match inferred {
        crate::infer::InferredExpression::Known(ty) if !ty.is_unmodelled() => Some(ty),
        _ => None,
    }
}

fn call_argument(call: &ParsedExpression, index: usize) -> Option<&ParsedExpression> {
    match call {
        ParsedExpression::Call { arguments, .. } | ParsedExpression::PropertyCall { arguments, .. } => {
            arguments.get(index).map(|argument| &argument.expression)
        }
        _ => None,
    }
}

/// tsc's `narrowTypeByTypePredicate` for a reference an optional-chain
/// argument reads through (`optionalChainContainsReference`): a target that
/// cannot be `undefined` proves the chain did not short-circuit.
/// [`narrow_optional_chain_containment`] over a plain symbol table.
fn optional_chain_contained_symbols(
    argument: &ParsedExpression,
    target: &Type,
    symbols: &SymbolTable,
) -> Option<SymbolTable> {
    let admits_undefined = |ty: &Type| matches!(ty, Type::Undefined | Type::Void) || ty.is_unmodelled();
    let target_admits_undefined = match target.peeled() {
        Type::Any | Type::GenuineUnknown => true,
        Type::Union(union) => union.types().iter().any(admits_undefined),
        other => admits_undefined(&other),
    };
    if target_admits_undefined {
        return None;
    }
    let mut narrowed_symbols: Option<SymbolTable> = None;
    for contained in super::optional_chain_contained_references(argument) {
        let Some((base, path)) = reference_path(contained) else {
            continue;
        };
        let current = narrowed_symbols.as_ref().unwrap_or(symbols);
        let Some(symbol) = current.get(&base) else {
            continue;
        };
        let guard = ReferenceGuard::Nullish {
            keep_matching: false,
            test: super::guards::NullishTest { null: true, undefined: true },
        };
        let Some(narrowed) = narrowed_reference_type(&symbol.ty, &path, guard) else {
            continue;
        };
        let info = SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        let declared = symbol.ty.clone();
        narrowed_symbols
            .get_or_insert_with(|| symbols.clone_with_reason(TypeCopyReason::ScopeOrContext))
            .insert_narrowed(base, info, declared);
    }
    narrowed_symbols
}

fn narrow_optional_chain_containment(argument: &ParsedExpression, target: &Type, scopes: &mut ScopeStack) {
    let admits_undefined = |ty: &Type| matches!(ty, Type::Undefined | Type::Void) || ty.is_unmodelled();
    let target_admits_undefined = match target.peeled() {
        Type::Any | Type::GenuineUnknown => true,
        Type::Union(union) => union.types().iter().any(admits_undefined),
        other => admits_undefined(&other),
    };
    if target_admits_undefined {
        return;
    }
    for contained in super::optional_chain_contained_references(argument) {
        super::narrow_reference_non_null_in_scope(contained, scopes);
    }
}

/// Narrows `subject` (or the member `path` reaches) to what an assertion
/// proved about it, from the statement on.
fn narrow_asserted_reference(subject: String, path: &[String], target: &Type, scopes: &mut ScopeStack) {
    let Some(symbol) = scopes.resolve(&subject) else {
        return;
    };
    let subject_ty = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let Some(narrowed) = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        narrowed_predicate_subject(&subject_ty, path, target, true)
    }) else {
        return;
    };
    let _ = scopes.insert_current_narrowed(
        subject,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        subject_ty,
    );
}

/// Collects the subjects of user-defined predicate guard calls (`isFoo(x)` → `x`)
/// within a (possibly `||`/`&&`/`!`-composed) condition. Kept separate from
/// [`collect_guard_operand_identifiers`]: recognizing a predicate call needs the
/// callee's collected signature, and a non-predicate call must not count as a
/// guard (tsc does not narrow `if (foo(x))`).
pub(super) fn collect_predicate_guard_subjects(
    condition: &ParsedExpression,
    scopes: &ScopeStack,
    names: &mut Vec<String>,
    ctx: &mut CheckerContext,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_predicate_guard_subjects(operand, scopes, names, ctx),
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or | ParsedLogicalOperator::And,
            right,
            ..
        } => {
            collect_predicate_guard_subjects(left, scopes, names, ctx);
            collect_predicate_guard_subjects(right, scopes, names, ctx);
        }
        _ => {
            if let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
                predicate_callee_signature(callee, |name| scopes.resolve(name), scopes.visible_symbols(), ctx)
            })
                // A predicate over a member (`isX(decl.id)`) proves something
                // about that member, not that the base is non-nullish; only a
                // guard over the identifier itself joins the operand set.
                && guard.path.is_empty()
                && !names.iter().any(|existing| existing == &guard.subject)
            {
                names.push(guard.subject);
            }
        }
    }
}

/// Narrows `symbols` by everything the truthy operand of an `&&` proves: a
/// structured guard (`x.kind === "k" && …`) plus each identifier/property in the
/// `&&` chain narrowed to non-nullish (`a.b && a.b > c`). Returns `None` when
/// nothing narrows. Used to type the right operand of `&&`.
/// Narrows the guarded reference: the value itself for a bare identifier, or the
/// property the guard tested (`ts.isStringLiteral(node.moduleSpecifier)`).
pub(super) fn narrowed_predicate_subject(
    subject_ty: &Type,
    path: &[String],
    predicate_ty: &Type,
    branch_is_true: bool,
) -> Option<Type> {
    if path.is_empty() {
        return narrow_by_predicate(subject_ty, predicate_ty, branch_is_true);
    }
    narrowed_reference_type(
        subject_ty,
        path,
        ReferenceGuard::Predicate {
            target: predicate_ty,
            keep_matching: branch_is_true,
        },
    )
}

/// Applies user-defined type-predicate narrowing (`isFoo(x)`,
/// `ts.isImportDeclaration(node)`) to a plain symbol table, for the expression
/// paths that have no `ScopeStack` — the right operand of `&&` and a ternary's
/// branches. Kept apart from [`narrow_condition_symbol_table`] because
/// resolving the predicate's target type needs the checker context, which those
/// syntactic guards do not take.
pub(crate) fn narrow_predicate_guards_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    {
        return narrow_predicate_guards_symbol_table(operand, symbols, !branch_is_true, ctx);
    }
    // `ok ? a : b` where `ok` is a `const` alias of a predicate guard narrows
    // by the guard it was written as (tsc's aliased-condition narrowing).
    if let ParsedExpression::Identifier { name, .. } = condition
        && let Some(alias) = symbols.alias_condition(name)
        && let Some(inlining) = super::aliases::enter_alias_inlining()
    {
        let inlined = super::aliases::retain_constant_reference_guards(&alias, symbols, &|_| false);
        let narrowed = narrow_predicate_guards_symbol_table(&inlined, symbols, branch_is_true, ctx);
        drop(inlining);
        return narrowed;
    }

    // Every operand of an `&&` holds in its true branch, and — by De Morgan —
    // every operand of an `||` fails in its false branch, so either chain
    // narrows by all of its operands. The other branch proves nothing about any
    // single operand.
    if let ParsedExpression::Logical {
        left,
        operator,
        right,
        ..
    } = condition
        && matches!(
            (operator, branch_is_true),
            (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
        )
    {
        let left_narrowed =
            narrow_predicate_guards_symbol_table(left, symbols, branch_is_true, ctx);
        let right_narrowed = narrow_predicate_guards_symbol_table(
            right,
            left_narrowed.as_ref().unwrap_or(symbols),
            branch_is_true,
            ctx,
        );
        return right_narrowed.or(left_narrowed);
    }
    // The other branch is reached over two edges — the left operand deciding
    // it, or the left failing to and the right deciding it — and holds the
    // union of what each proves. A subject one edge leaves alone is not
    // narrowed.
    if let ParsedExpression::Logical {
        left,
        operator: ParsedLogicalOperator::And | ParsedLogicalOperator::Or,
        right,
        ..
    } = condition
    {
        let left_edge = narrow_predicate_guards_symbol_table(left, symbols, branch_is_true, ctx)?;
        let past_left = narrow_predicate_guards_symbol_table(left, symbols, !branch_is_true, ctx);
        let right_edge = narrow_predicate_guards_symbol_table(
            right,
            past_left.as_ref().unwrap_or(symbols),
            branch_is_true,
            ctx,
        )?;
        let mut subjects = Vec::new();
        predicate_guard_subjects(condition, symbols, &mut subjects, ctx);
        let mut joined = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
        let mut changed = false;
        for subject in subjects {
            let (Some(original), Some(from_left), Some(from_right)) =
                (symbols.get(&subject), left_edge.get(&subject), right_edge.get(&subject))
            else {
                continue;
            };
            if from_left.ty == original.ty || from_right.ty == original.ty {
                continue;
            }
            let ty = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                surge_ts_types::union_type(vec![from_left.ty.clone(), from_right.ty.clone()])
            });
            let info = SymbolInfo {
                ty,
                kind: original.kind,
                function_signature: original.function_signature.clone(),
            };
            let declared = original.ty.clone();
            joined.insert_narrowed(subject, info, declared);
            changed = true;
        }
        return changed.then_some(joined);
    }

    if let Some(narrowed) =
        narrow_this_predicate_symbol_table(condition, symbols, branch_is_true, ctx)
    {
        return Some(narrowed);
    }

    if let Some(narrowed) =
        narrow_instanceof_heritage_symbol_table(condition, symbols, branch_is_true, ctx)
    {
        return Some(narrowed);
    }

    let guard = parse_type_predicate_condition(condition, &mut |callee| {
        predicate_callee_signature(callee, |name| symbols.get(name), symbols, ctx)
    })?;
    let Some(symbol) = symbols.get(&guard.subject) else {
        // An element reference (`isId(t?.elements?.[0])`) is no binding here,
        // but holding still proves the chain it reads through present.
        if !branch_is_true {
            return None;
        }
        let argument = call_argument(condition, guard.parameter_index)?;
        let argument_ty = inferred_argument_type(argument, symbols, ctx);
        let PredicateTarget::Resolved(predicate_ty) =
            resolve_predicate_guard_target(&guard, argument_ty.as_ref(), symbols, ctx)?
        else {
            return None;
        };
        return optional_chain_contained_symbols(argument, &predicate_ty, symbols);
    };
    let subject_ty = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let argument_ty = predicate_argument_type(&subject_ty, &guard.path).or_else(|| {
        call_argument(condition, guard.parameter_index)
            .and_then(|argument| inferred_argument_type(argument, symbols, ctx))
    });
    let target = resolve_predicate_guard_target(&guard, argument_ty.as_ref(), symbols, ctx)?;
    // `isFoo(o?.x)` holding proves `o` present, as the scoped form does
    // (`narrow_optional_chain_containment`).
    let contained = match (&target, branch_is_true, call_argument(condition, guard.parameter_index)) {
        (PredicateTarget::Resolved(predicate_ty), true, Some(argument)) => {
            optional_chain_contained_symbols(argument, predicate_ty, symbols)
        }
        _ => None,
    };
    let base_symbols = contained.as_ref().unwrap_or(symbols);
    let subject_ty = base_symbols.get(&guard.subject).map_or(subject_ty, |symbol| symbol.ty.clone());
    let narrowed = match target {
        PredicateTarget::Resolved(predicate_ty) => {
            with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                narrowed_predicate_subject(&subject_ty, &guard.path, &predicate_ty, branch_is_true)
            })
        }
        PredicateTarget::Degraded if guard.path.is_empty() => degraded_predicate_subject(branch_is_true),
        PredicateTarget::Degraded => None,
    };
    let Some(narrowed) = narrowed.filter(|narrowed| *narrowed != subject_ty) else {
        return contained;
    };
    let mut narrowed_symbols = base_symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        guard.subject,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        subject_ty,
    );
    Some(narrowed_symbols)
}

/// The bare identifiers the predicate calls in `condition` guard, through `!`,
/// `&&` and `||`.
fn predicate_guard_subjects(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    subjects: &mut Vec<String>,
    ctx: &mut CheckerContext,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => predicate_guard_subjects(operand, symbols, subjects, ctx),
        ParsedExpression::Logical { left, right, .. } => {
            predicate_guard_subjects(left, symbols, subjects, ctx);
            predicate_guard_subjects(right, symbols, subjects, ctx);
        }
        _ => {
            if let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
                predicate_callee_signature(callee, |name| symbols.get(name), symbols, ctx)
            }) && guard.path.is_empty()
                && !subjects.contains(&guard.subject)
            {
                subjects.push(guard.subject);
            }
        }
    }
}

/// Applies every property-path type-predicate guard the condition proves. Walks
/// `!` and the operands of a true-branch `&&`, the shapes that compose guards.
pub(super) fn narrow_predicate_reference_guards_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    {
        narrow_predicate_reference_guards_in_scope(operand, scopes, !branch_is_true, ctx);
        return;
    }
    if let Some((inner, flip)) = strip_boolean_literal_comparison(condition) {
        narrow_predicate_reference_guards_in_scope(inner, scopes, branch_is_true != flip, ctx);
        return;
    }

    if let ParsedExpression::Logical {
        left,
        operator,
        right,
        ..
    } = condition
        && matches!(
            (operator, branch_is_true),
            (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
        )
    {
        narrow_predicate_reference_guards_in_scope(left, scopes, branch_is_true, ctx);
        narrow_predicate_reference_guards_in_scope(right, scopes, branch_is_true, ctx);
        return;
    }

    let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
        predicate_callee_signature(callee, |name| scopes.resolve(name), scopes.visible_symbols(), ctx)
    }) else {
        return;
    };
    if guard.path.is_empty() {
        return;
    }
    let argument_ty = scopes
        .resolve(&guard.subject)
        .and_then(|symbol| predicate_argument_type(&symbol.ty, &guard.path));
    let Some(predicate_ty) =
        resolve_predicate_guard_type(&guard, argument_ty.as_ref(), scopes.visible_symbols(), ctx)
    else {
        return;
    };
    narrow_reference_in_scope(
        &guard.subject,
        &guard.path,
        ReferenceGuard::Predicate {
            target: &predicate_ty,
            keep_matching: branch_is_true,
        },
        scopes,
    );
}

/// tsc's `getTypePredicateFromBody` for an arrow whose body is an expression.
pub(crate) fn infer_predicate_from_body(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    parameter_types: &[Type],
    returned: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<crate::symbols::InferredPredicate> {
    let scope = parameter_scope(parameters, parameter_types, symbols);
    refined_parameter_predicate(
        parameters,
        parameter_types,
        returned,
        &scope,
        &Default::default(),
        ctx,
    )
}

/// tsc's `getTypePredicateFromBody` for a block body: exactly one `return`,
/// with a value, at the end of the body (so it has no implicit return). The
/// parameters are typed as the flow leaves them at that `return`: a preceding
/// `if` whose body cannot complete (`if (x instanceof Date) throw …`) rules its
/// condition out.
pub(crate) fn infer_predicate_from_function_body(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    parameter_types: &[Type],
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<crate::symbols::InferredPredicate> {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    let (Statement::Return(statement), before) = body.split_last()? else {
        return None;
    };
    let returned = statement.expression.as_ref()?;
    if contains_return(before) {
        return None;
    }
    let mut scope = parameter_scope(parameters, parameter_types, symbols);
    for statement in before {
        let Statement::If(if_statement) = statement else {
            continue;
        };
        if !if_statement.else_body.is_empty()
            || !crate::flow::analyze_function_body_flow(&if_statement.then_body).guarantees_exit
        {
            continue;
        }
        if let Some(narrowed) = narrowed_by_condition(&if_statement.condition, &scope, false, ctx) {
            scope = narrowed;
        }
    }
    let assigned = crate::flow::assigned_bindings(body);
    refined_parameter_predicate(parameters, parameter_types, returned, &scope, &assigned, ctx)
}

/// tsc's `checkIfExpressionRefinesAnyParameter`: the first parameter the
/// returned boolean expression refines. A parameter is refined when the
/// expression holding narrows it to some `T` and the expression failing leaves
/// nothing of `T` (`checkIfExpressionRefinesParameter`) — "`x is T`" holds if
/// and only if the call returns true. A `boolean` parameter, a destructured or
/// rest one, and one the body assigns are never refined.
fn refined_parameter_predicate(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    parameter_types: &[Type],
    returned: &ParsedExpression,
    scope: &SymbolTable,
    assigned: &std::collections::HashSet<Arc<str>>,
    ctx: &mut CheckerContext,
) -> Option<crate::symbols::InferredPredicate> {
    if !is_boolean_expression(returned) {
        return None;
    }
    for (parameter_index, (parameter, declared)) in
        parameters.iter().zip(parameter_types).enumerate()
    {
        let Some(name) = identifier_parameter_name(parameter) else {
            continue;
        };
        if matches!(declared, Type::Boolean)
            || declared.is_unmodelled() && !matches!(declared, Type::GenuineUnknown)
            || assigned.contains(name.as_str())
        {
            continue;
        }
        let initial = scope.get(&name).map_or_else(|| declared.clone(), |symbol| symbol.ty.clone());
        let narrowed_parameter = |table: Option<SymbolTable>| {
            table.and_then(|table| table.get(&name).map(|symbol| symbol.ty.clone()))
        };
        let Some(true_type) = narrowed_parameter(narrowed_by_condition(returned, scope, true, ctx))
        else {
            continue;
        };
        if true_type == initial || true_type.is_unmodelled() {
            continue;
        }
        // `checkIfExpressionRefinesParameter` narrows the false branch from
        // the true type with the parameter's declared type still the
        // reference's declared type, which decides discriminant narrowing.
        let mut assumed = scope.clone_with_reason(TypeCopyReason::ScopeOrContext);
        assumed.insert_narrowed(
            name.clone(),
            SymbolInfo {
                ty: true_type.clone(),
                kind: crate::symbols::SymbolKind::Parameter,
                function_signature: None,
            },
            declared.clone(),
        );
        let false_subtype =
            narrowed_parameter(narrowed_by_condition(returned, &assumed, false, ctx))
                .unwrap_or_else(|| true_type.clone());
        if matches!(false_subtype, Type::Never) {
            return Some(crate::symbols::InferredPredicate {
                parameter_index,
                target: true_type,
            });
        }
    }
    None
}

fn identifier_parameter_name(
    parameter: &surge_ts_syntax::ParsedFunctionParameter,
) -> Option<String> {
    match &parameter.binding_name {
        surge_ts_syntax::ParsedBindingName::Identifier { name, .. } if !parameter.rest => {
            Some(name.clone())
        }
        _ => None,
    }
}

/// `symbols` with each named parameter bound to its declared type.
fn parameter_scope(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    parameter_types: &[Type],
    symbols: &SymbolTable,
) -> SymbolTable {
    let mut scope = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    for (parameter, ty) in parameters.iter().zip(parameter_types) {
        if let Some(name) = identifier_parameter_name(parameter) {
            scope.insert(
                name,
                SymbolInfo {
                    ty: ty.clone(),
                    kind: crate::symbols::SymbolKind::Parameter,
                    function_signature: None,
                },
            );
        }
    }
    scope
}

/// `symbols` narrowed by `condition` in one branch, by the syntactic guards
/// and the ones that need the checker (predicate calls, `instanceof` against
/// a constructor's instance type).
fn narrowed_by_condition(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    let narrowed = super::narrow_condition_symbol_table(condition, symbols, branch_is_true);
    narrow_predicate_guards_symbol_table(
        condition,
        narrowed.as_ref().unwrap_or(symbols),
        branch_is_true,
        ctx,
    )
    .or(narrowed)
}

/// Whether an expression is boolean-typed by its shape — tsc requires the
/// returned expression's type to be `boolean` (a comparison, `!`, `in`,
/// `instanceof`, a call, or `&&`/`||` of those). `x => x` narrows `x` when
/// true but returns `x`, so it is no predicate.
fn is_boolean_expression(expression: &ParsedExpression) -> bool {
    use surge_ts_syntax::ParsedBinaryOperator as Op;
    match expression {
        ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::Call { .. }
        | ParsedExpression::PropertyCall { .. }
        | ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            ..
        } => true,
        ParsedExpression::Binary { operator, .. } => matches!(
            operator,
            Op::StrictEquals
                | Op::StrictNotEquals
                | Op::Equals
                | Op::NotEquals
                | Op::LessThan
                | Op::LessThanEquals
                | Op::GreaterThan
                | Op::GreaterThanEquals
                | Op::In
                | Op::Instanceof
        ),
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And | ParsedLogicalOperator::Or,
            right,
            ..
        } => is_boolean_expression(left) && is_boolean_expression(right),
        ParsedExpression::SatisfiesExpression { expression, .. } => {
            is_boolean_expression(expression)
        }
        _ => false,
    }
}

/// Whether any statement contains a `return` (tsc's `forEachReturnStatement`
/// stops at nested functions and classes).
fn contains_return(body: &[surge_ts_syntax::ParsedFunctionBodyStatement]) -> bool {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    body.iter().any(|statement| match statement {
        Statement::Return(_) => true,
        Statement::Block(block) => contains_return(block),
        Statement::If(if_statement) => {
            contains_return(&if_statement.then_body) || contains_return(&if_statement.else_body)
        }
        Statement::While(while_statement) => contains_return(&while_statement.body),
        Statement::ForOf(for_of) => contains_return(&for_of.body),
        Statement::Switch(switch) => {
            switch.cases.iter().any(|case| contains_return(&case.consequent))
        }
        Statement::Try(try_statement) => {
            contains_return(&try_statement.block)
                || try_statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| contains_return(&handler.body))
                || contains_return(&try_statement.finalizer)
        }
        _ => false,
    })
}
