use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, union_type};

/// A user-defined type-predicate guard extracted from a call condition
/// (`isFoo(x)` where `isFoo`'s collected signature returns `param is T`).
pub(crate) struct PredicateGuardInfo {
    /// The base identifier of the argument in the tested parameter's position.
    pub(crate) subject: String,
    /// Property path from `subject` to the tested reference, empty for a bare
    /// identifier (`ts.isStringLiteral(node.moduleSpecifier)` -> `["moduleSpecifier"]`).
    pub(crate) path: Vec<String>,
    /// The predicate's parsed target type (`T` in `param is T`), unresolved.
    pub(crate) predicate_type: surge_ts_syntax::ParsedType,
    /// File the predicate signature was declared in; its module-local type
    /// names resolve under this file's scope (see
    /// [`crate::symbols::FunctionSignatureInfo::declaring_file`]).
    pub(crate) declaring_file: Option<std::sync::Arc<str>>,
    /// Namespace the predicate signature was declared in, when it was published
    /// as a qualified `ns.member`. `node is ImportDeclaration` inside
    /// `declare namespace ts` names `ts.ImportDeclaration`, which only resolves
    /// under that prefix (see
    /// [`crate::symbols::FunctionSignatureInfo::namespace_prefix`]).
    pub(crate) namespace_prefix: Option<std::sync::Arc<str>>,
    /// The callee's signature, kept so a generic predicate (`x is OK<T>`) can
    /// infer `T` from the tested argument at the guard site.
    pub(crate) signature: std::sync::Arc<crate::symbols::FunctionSignatureInfo>,
    /// The call's other arguments, by declared position, so a generic
    /// predicate's remaining type parameters can be inferred from them —
    /// `isMatching(pattern, value)` states its target in terms of the *pattern*.
    /// Empty unless the signature is generic, since nothing else reads it.
    pub(crate) other_arguments: Vec<(usize, ParsedExpression)>,
    /// Position of the tested parameter in `signature.parameter_types`.
    pub(crate) parameter_index: usize,
    /// Type arguments written at the call (`isTRPCClientError<typeof
    /// appRouter>(err)`); they bind the signature's type parameters by
    /// position instead of inference from the arguments.
    pub(crate) explicit_type_arguments: Vec<surge_ts_syntax::ParsedType>,
    /// Bindings of the enclosing declaration's type parameters for a predicate
    /// read off a member (`j.Identifier.check(x)` is `Type<T>.check(v): v is
    /// T` with `T` already `Identifier`); seeded into the resolution.
    pub(crate) outer_type_arguments: Vec<(String, Type)>,
}

/// What a guard site is asked to look up for a predicate call.
pub(crate) enum PredicateCallee<'a> {
    /// A bare or namespace-qualified callee name (`isFoo`, `ts.isFoo`).
    Name(&'a str),
    /// A method call on an arbitrary receiver expression.
    Member {
        object: &'a ParsedExpression,
        property: &'a str,
    },
}

/// A predicate callee's signature plus the enclosing bindings it was read under.
pub(crate) struct PredicateSignature {
    pub(crate) signature: std::sync::Arc<crate::symbols::FunctionSignatureInfo>,
    pub(crate) outer_type_arguments: Vec<(String, Type)>,
}

impl PredicateSignature {
    pub(crate) fn plain(signature: std::sync::Arc<crate::symbols::FunctionSignatureInfo>) -> Self {
        Self {
            signature,
            outer_type_arguments: Vec::new(),
        }
    }
}

/// Extracts a user-defined type-predicate guard from a call condition. The
/// callee's collected signature must declare a non-asserts `param is T`
/// predicate over a named value parameter, the call must not carry explicit
/// type arguments, and the argument in the tested position must be a bare
/// identifier. A *generic* predicate is kept: its type parameters are inferred
/// from the tested argument's own type when the guard is resolved.
pub(crate) fn parse_type_predicate_condition(
    condition: &ParsedExpression,
    signature_of: &mut dyn FnMut(PredicateCallee<'_>) -> Option<PredicateSignature>,
) -> Option<PredicateGuardInfo> {
    // A guard reached through a namespace (`ts.isImportDeclaration(node)`) parses
    // as a property call; its signature is registered under the qualified
    // `<alias>.<member>` key. Any other receiver is a method predicate read off
    // the receiver's type (`j.Identifier.check(node)`).
    let (found, type_arguments, arguments) = match condition {
        ParsedExpression::Call {
            callee_name,
            type_arguments,
            arguments,
            ..
        } => (
            signature_of(PredicateCallee::Name(callee_name.as_str()))?,
            type_arguments,
            arguments,
        ),
        ParsedExpression::PropertyCall {
            object,
            property_name,
            type_arguments,
            arguments,
            ..
        } => {
            let qualified = if let ParsedExpression::Identifier { name, .. } = object.as_ref() {
                signature_of(PredicateCallee::Name(&format!("{name}.{property_name}")))
            } else {
                None
            };
            let found = match qualified {
                Some(found) => found,
                None => signature_of(PredicateCallee::Member {
                    object,
                    property: property_name,
                })?,
            };
            (found, type_arguments, arguments)
        }
        _ => return None,
    };
    let PredicateSignature {
        signature,
        outer_type_arguments,
    } = found;
    // An overload group keeps one declaration's parsed signature, which need not
    // be the one that declared the predicate; the fold records that overload
    // alongside. `arguments.get(index)` below still decides whether this call
    // reaches the predicate's parameter, so a call of a *different* overload
    // narrows nothing.
    let signature = match &signature.return_type {
        Some(surge_ts_syntax::ParsedType::Predicate(_)) => signature,
        _ if signature.inferred_predicate.is_some() => signature,
        _ => signature.predicate_overload.clone()?,
    };
    let (predicate_type, index) = match (&signature.return_type, &signature.inferred_predicate) {
        (Some(surge_ts_syntax::ParsedType::Predicate(predicate)), _) => {
            if predicate.asserts || predicate.parameter_name == "this" {
                return None;
            }
            let index = signature
                .parameter_names
                .iter()
                .position(|name| name.as_deref() == Some(predicate.parameter_name.as_str()))?;
            (predicate.ty.clone()?, index)
        }
        // The target is already a type; the written slot is never resolved.
        (_, Some(inferred)) => (surge_ts_syntax::ParsedType::Any, inferred.parameter_index),
        _ => return None,
    };
    let tested = &arguments.get(index)?.expression;
    // An element access (`node.arguments[0]`) is a reference tsc narrows too;
    // it is keyed the way the element-reference narrowing records reads.
    let (subject, path) = super::super::reference_path(tested)
        .or_else(|| super::super::element_access_parts(tested).map(|key| (key, Vec::new())))?;
    let other_arguments = if signature.type_parameters.is_empty() || !type_arguments.is_empty() {
        Vec::new()
    } else {
        arguments
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .map(|(position, argument)| (position, argument.expression.clone()))
            .collect()
    };
    Some(PredicateGuardInfo {
        subject,
        path,
        other_arguments,
        predicate_type,
        declaring_file: signature.declaring_file.clone(),
        namespace_prefix: signature.namespace_prefix.clone(),
        parameter_index: index,
        signature,
        explicit_type_arguments: type_arguments.clone(),
        outer_type_arguments,
    })
}

/// Narrows a value tested by a `param is T` predicate. In the true branch a
/// union keeps the members assignable to `T`; when none are but `T` itself fits
/// a member (`string | undefined` guarded by `x is "a" | "b"`), the value *is*
/// `T`. The false branch removes the members assignable to `T`. A non-union
/// subject narrows to `T` in the true branch when `T` is a subtype of it.
/// Returns `None` when the guard proves nothing new (or would empty the type —
/// stay conservative rather than model `never`).
fn has_string_index(ty: &Type) -> bool {
    matches!(ty.peeled(), Type::Object(object) if object.string_index_type.is_some() && !object.synthetic_open_index)
}

pub(crate) fn narrow_by_predicate(
    ty: &Type,
    predicate: &Type,
    keep_matching: bool,
) -> Option<Type> {
    // The false branch takes what the predicate leaves of a union constraint
    // (`getNarrowableTypeForReference`); the true branch keeps `T & P` below.
    if !keep_matching
        && let Some((variable, constraint)) = super::super::reference::union_constrained_variable(ty)
    {
        let narrowed = narrow_by_predicate(&constraint, predicate, false)?;
        return Some(surge_ts_types::type_variable::intersect_type_variable(&variable, narrowed));
    }
    let peeled = ty.peeled();
    if let Type::Union(union) = &peeled {
        let members = union.types();
        if keep_matching {
            // `getNarrowedTypeWorker` keeps a member only when it is a strict
            // subtype of the predicate; one the predicate is a strict subtype
            // of instead gives way to it. An object without a string index
            // signature is no strict subtype of one with it, however mutually
            // assignable: `{} | undefined` (an `unknown` past `=== null`)
            // narrows to `Readonly<Record<string, unknown>>`.
            let predicate_indexed = has_string_index(predicate);
            let matching: Vec<Type> = members
                .iter()
                .filter(|member| surge_ts_types::is_assignable_to(member, predicate))
                .map(|member| {
                    if predicate_indexed
                        && !has_string_index(member)
                        && matches!(member.peeled(), Type::Object(_))
                        && surge_ts_types::is_assignable_to(predicate, member)
                    {
                        predicate.clone()
                    } else {
                        member.clone()
                    }
                })
                .collect();
            if !matching.is_empty() {
                if matching.as_slice() == members {
                    return None;
                }
                return Some(union_type(matching));
            }
            if members
                .iter()
                .any(|member| surge_ts_types::is_assignable_to(predicate, member))
            {
                return Some(predicate.clone());
            }
            let variables: Vec<Type> = members
                .iter()
                .filter(|member| member.is_type_variable())
                .map(|member| surge_ts_types::type_variable::intersect_type_variable(member, predicate.clone()))
                .collect();
            if !variables.is_empty() {
                return Some(union_type(variables));
            }
            return None;
        }
        let remaining: Vec<Type> = members
            .iter()
            .filter(|member| !surge_ts_types::is_assignable_to(member, predicate))
            .cloned()
            .collect();
        if remaining.is_empty() || remaining.len() == members.len() {
            return None;
        }
        return Some(union_type(remaining));
    }
    // A non-union subject narrows to the predicate whenever the predicate is a
    // subtype of it, which is tsc's rule. Requiring the subject *not* to be
    // assignable to the predicate as well is stricter than tsc and silently
    // depended on that direction failing: an `unknown` subject narrowed because
    // nothing is assignable to it, while `{}` did not, because an
    // index-signature-only target such as `Record<string, unknown>` accepts any
    // object. `any` and the degradation sentinel are excluded — both are
    // assignable in every direction, so narrowing them would invent a type for
    // a subject whose real one was never reconstructed. The genuine `unknown`
    // keyword is not excluded: narrowing it is the case that already works.
    // A type variable the predicate does not already cover is `T & P`
    // (`getNarrowedTypeWorker`'s fallback): it keeps its identity, and the
    // predicate's members come with it.
    // A candidate that is itself a subtype of the variable (`Extract<T, F>`
    // narrowing `T`) is the narrowed type.
    // `narrowTypeByTypePredicate` on `any`: narrowed to the predicate unless
    // that is the global `Object` or `Function`, which `any` already is.
    if keep_matching
        && matches!(peeled, Type::Any)
        && !predicate.is_unmodelled()
        && !matches!(predicate, Type::Any | Type::GenuineUnknown)
        && !matches!(predicate, Type::Reference(reference)
            if matches!(reference.display.as_ref(), "Object" | "Function"))
    {
        return Some(predicate.clone());
    }
    if keep_matching && peeled.is_type_variable() {
        if predicate.is_unmodelled() || surge_ts_types::is_assignable_to(&peeled, predicate) {
            return None;
        }
        if predicate.is_type_variable() && surge_ts_types::is_assignable_to(predicate, &peeled) {
            return Some(predicate.clone());
        }
        return Some(surge_ts_types::type_variable::intersect_type_variable(&peeled, predicate.clone()));
    }
    // An already narrowed `T & X` gains the predicate as one more constituent.
    if keep_matching
        && surge_ts_types::type_variable::is_narrowed_type_variable(&peeled)
        && let Type::Object(object) = &peeled
        && let Some(operands) = object.intersection_operands.as_deref()
    {
        if predicate.is_unmodelled() || surge_ts_types::is_assignable_to(&peeled, predicate) {
            return None;
        }
        let mut operands = operands.to_vec();
        operands.push(predicate.clone());
        return Some(surge_ts_types::type_variable::type_variable_intersection(operands));
    }
    if keep_matching
        && !matches!(peeled, Type::Any | Type::Unknown | Type::TypeParameter(_))
        && peeled != *predicate
        && surge_ts_types::is_assignable_to(predicate, &peeled)
    {
        // `getNarrowedTypeWorker` tries the subject as a strict subtype of the
        // predicate first: `number[]` stays itself under `arg is any[]`.
        if surge_ts_types::is_strict_subtype_of(&peeled, predicate) {
            return None;
        }
        return Some(predicate.clone());
    }
    // `getNarrowedTypeWorker`'s false branch keeps what the true branch would
    // not: a subject the predicate already covers is left with nothing.
    if !keep_matching
        && !peeled.is_unknown()
        && !matches!(peeled, Type::Any | Type::Never)
        && !predicate.is_unmodelled()
        && surge_ts_types::is_assignable_to(&peeled, predicate)
    {
        return Some(Type::Never);
    }
    // tsc narrows a non-union subject the predicate does not refine to the
    // intersection of the two (`getNarrowedTypeWorker`'s last resort, flow.go):
    // `conn: Connection` under `this is { ws: WebSocket }` reads `conn.ws` as
    // `WebSocket`.
    if keep_matching
        && matches!(peeled, Type::Object(_))
        && matches!(predicate.peeled(), Type::Object(_))
        && !surge_ts_types::is_assignable_to(&peeled, predicate)
    {
        return Some(crate::infer::types::merge_intersection_members(vec![
            ty.clone(),
            predicate.clone(),
        ]));
    }
    None
}
