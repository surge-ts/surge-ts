use std::sync::Arc;
use surge_ts_syntax::ParsedUnaryOperator;
use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator};
use surge_ts_types::{Type, TypeCopyReason, union_type, with_type_copy_reason};

use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};
use super::guards::*;
use super::narrow_to_instanceof_subclass;

/// A guard applied to one reference (`o.p`, `this.a.b`, or a bare identifier).
#[derive(Debug, Clone, Copy)]
pub(super) enum ReferenceGuard<'a> {
    Truthy,
    Typeof {
        tag: &'a str,
        keep_matching: bool,
    },
    PropertyPresence {
        property: &'a str,
        keep_present: bool,
    },
    Instanceof {
        ctor_name: &'a str,
        /// The constructor's instance type, when it resolved. A subject that is
        /// not a union narrows *down* to it (`s: Base` -> `Sub`), which member
        /// filtering cannot express.
        instance: Option<&'a Type>,
        keep_matching: bool,
    },
    Arrayness {
        keep_arrays: bool,
    },
    ArrayBufferView {
        keep_views: bool,
    },
    Nullish {
        keep_matching: bool,
        test: super::guards::NullishTest,
    },
    /// `declared` is the slot's declared type when the root has one: tsc's
    /// `getAssignmentReducedType` reduces the *declared* type by what was
    /// assigned, so a write after a guard (`if (!o.c) o.c = v`) is not stuck
    /// with the guard's `undefined`.
    Assigned {
        assigned: &'a Type,
        declared: Option<&'a Type>,
    },
    /// A user-defined type predicate applied to a property path
    /// (`ts.isStringLiteral(node.moduleSpecifier)`).
    Predicate {
        target: &'a Type,
        keep_matching: bool,
    },
    /// `o.p === "lit"` / `o.p !== 3` on the property itself. The discriminant
    /// narrowers filter the *base* union by the same test; this one narrows the
    /// property's own type, which is what a later read of `o.p` sees.
    LiteralEquality {
        literal: &'a ParsedExpression,
        keep_matching: bool,
    },
    /// The reference tested falsy (`if (!x)`, the `else` of `if (x)`): tsc's
    /// `getTypeWithFacts(type, TypeFacts.Falsy)`, where members that are always
    /// truthy go and anything that may be falsy stays (`string` keeps `string`,
    /// not `""`).
    Falsy,
    /// `x.constructor === C` held: only what `C` itself constructs is left.
    ConstructedBy { ctor_name: &'a str },
    /// `a.b.c.kind === "x"`: the union at `a.b.c` is filtered by its `kind`
    /// member, however deep the reference is.
    Discriminant {
        property: &'a str,
        literal: &'a Type,
        keep_matching: bool,
    },
    /// `"p" in o` or `o.hasOwnProperty("p")`, applied to `o.p` under
    /// `exactOptionalPropertyTypes`: a read typed with tsc's missing type loses
    /// it, or is left with only it (`narrowTypeByInKeyword`'s
    /// `containsMissingType` arm).
    ExactOptionalPresence { keep_present: bool },
}

impl ReferenceGuard<'_> {
    /// Whether the guard holding proves the tested reference is not
    /// `undefined` — and so, for `base?.p`, that `base` is not nullish either:
    /// a nullish base makes the whole chain `undefined`.
    fn proves_defined(&self) -> bool {
        match self {
            Self::Truthy => true,
            Self::Typeof { tag, keep_matching } => (*tag == "undefined") != *keep_matching,
            Self::Nullish {
                keep_matching: false,
                test,
            } => test.undefined,
            _ => false,
        }
    }

    /// Narrows one leaf — a property's type plus its `optional` flag, or a whole
    /// binding's type with `optional = false`. `None` leaves the leaf alone.
    pub(super) fn narrow_leaf(&self, ty: &Type, optional: bool) -> Option<(Type, bool)> {
        match self {
            Self::ConstructedBy { ctor_name } => {
                let narrowed = narrow_union_by_constructor(ty, ctor_name)?;
                (narrowed != *ty).then_some((narrowed, optional))
            }
            Self::Falsy => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = super::truthy::keep_possibly_falsy(&effective);
                (narrowed != effective).then_some((narrowed, false))
            }
            // `getAdjustedTypeWithFacts(unknown, Truthy)` is `{}`.
            Self::Truthy if matches!(ty, Type::GenuineUnknown) => Some((super::unknown_empty_object(), false)),
            Self::Truthy => {
                let narrowed = super::truthy::remove_definitely_falsy(ty);
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            // An optional property carries its `undefined` in the `optional` flag
            // rather than the type, so the tag test has to see it put back.
            Self::Typeof { tag, keep_matching } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_union_by_typeof(&effective, tag, *keep_matching)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::PropertyPresence {
                property,
                keep_present,
            } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed =
                    narrow_union_by_property_presence(&effective, property, *keep_present)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::Instanceof {
                ctor_name,
                instance,
                keep_matching,
            } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed =
                    narrow_union_by_instanceof(&effective, ctor_name, *instance, *keep_matching)
                        .or_else(|| {
                            narrow_to_instanceof_subclass(&effective, *instance, *keep_matching)
                        })?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::Arrayness { keep_arrays } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_union_by_arrayness(&effective, *keep_arrays)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::ArrayBufferView { keep_views } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_union_by_arraybufferview(&effective, *keep_views)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::Nullish {
                keep_matching,
                test,
            } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_union_by_nullish(&effective, *keep_matching, *test)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::Predicate {
                target,
                keep_matching,
            } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_by_predicate(&effective, target, *keep_matching)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            // The complement of a literal test keeps `undefined` (an absent
            // property is not the literal either), and an optional slot stores
            // that `undefined` in its flag, so it is moved back there.
            Self::Discriminant {
                property,
                literal,
                keep_matching,
            } => {
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed =
                    narrow_union_by_discriminant(&effective, property, literal, *keep_matching)?;
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
            Self::LiteralEquality {
                literal,
                keep_matching,
            } => {
                let literal = literal_expression_value(literal)?;
                let effective = Self::effective_leaf_type(ty, optional);
                let narrowed = narrow_by_literal_equality(&effective, &literal, *keep_matching)?;
                let still_optional = optional && type_includes_undefined_member(&narrowed);
                let narrowed = match narrowed {
                    // Nothing but the absent case is left: `tag?: never`.
                    // `remove_undefined` answers the degradation sentinel for a
                    // bare `undefined`, which would make the whole object read
                    // as unresolved and silence every later check on it.
                    Type::Undefined if still_optional => Type::Never,
                    narrowed if still_optional => surge_ts_types::remove_undefined(&narrowed),
                    narrowed => narrowed,
                };
                (narrowed != *ty || still_optional != optional).then_some((narrowed, still_optional))
            }
            Self::ExactOptionalPresence { keep_present } => {
                if !crate::checks::expr::is_exact_optional_slot(ty, optional) {
                    return None;
                }
                Some(if *keep_present {
                    (ty.clone(), false)
                } else {
                    (Type::Undefined, false)
                })
            }
            // An assignment narrows a union-declared slot to the members the
            // assigned value can inhabit, as tsc does. A non-union slot is
            // already as precise as the declaration allows, so it is left alone.
            Self::Assigned { assigned, declared } => {
                let effective = match declared {
                    Some(declared) => (*declared).clone(),
                    None => Self::effective_leaf_type(ty, optional),
                };
                let Type::Union(union) = &effective else {
                    return (declared.is_some() && (optional || effective != *ty) && !effective.is_unmodelled())
                        .then_some((effective, false));
                };
                // Same scan-before-clone rule as the truthiness split: an
                // assignment that rules nothing out must not rebuild the union.
                let kept_count = union
                    .types()
                    .iter()
                    .filter(|member| surge_ts_types::is_assignable_to(assigned, member))
                    .count();
                if kept_count == 0 || kept_count == union.types().len() {
                    return None;
                }
                let narrowed = union_type(
                    union
                        .types()
                        .iter()
                        .filter(|member| surge_ts_types::is_assignable_to(assigned, member))
                        .cloned()
                        .collect(),
                );
                // An optional slot narrows even when the kept members equal
                // its written type: the assignment proved it present.
                (optional || narrowed != *ty).then_some((narrowed, false))
            }
        }
    }

    /// An optional property's `undefined` lives in its `optional` flag, not its
    /// type; a guard has to see it put back to split on it correctly.
    pub(super) fn effective_leaf_type(ty: &Type, optional: bool) -> Type {
        if optional {
            union_type(vec![ty.clone(), Type::Undefined])
        } else {
            ty.clone()
        }
    }
}

/// The base symbol name and property chain of a plain reference expression:
/// `x` → `("x", [])`, `this.a.b` → `("this", ["a", "b"])`. `None` for anything
/// that is not a static reference (a call, a computed index, a literal).
pub(super) fn reference_path(expression: &ParsedExpression) -> Option<(String, Vec<String>)> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some((name.clone(), Vec::new())),
        ParsedExpression::This { .. } => Some(("this".to_string(), Vec::new())),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => {
            let (base, mut path) = reference_path(object)?;
            path.push(property_name.clone());
            Some((base, path))
        }
        ParsedExpression::NonNullAssertion { expression, .. } => reference_path(expression),
        // tsc's `isMatchingReference`: a comma expression is the reference its
        // right operand is.
        ParsedExpression::Sequence { expressions } => reference_path(&expressions.last()?.0),
        // ...and an assignment is the reference it writes.
        ParsedExpression::Assignment { target_name, .. } => Some((target_name.clone(), Vec::new())),
        _ => None,
    }
}

/// Rebuilds `ty` with the property reached by `path` replaced by the guard's
/// narrowing, distributing over unions and peeling named references. `None` when
/// the path does not exist or nothing narrows, so the caller leaves the binding
/// untouched.
pub(super) fn narrow_property_path(ty: &Type, path: &[String], guard: ReferenceGuard<'_>) -> Option<Type> {
    let (head, rest) = path.split_first()?;

    match ty.peeled() {
        Type::Object(mut object_type) => {
            // A name reached only through the string index signature is a
            // reference like any other (`typeof config.works !== "boolean"`);
            // the narrowed slot is recorded as a property marked `index_slot`.
            let mut existing = match object_type.properties.get(head.as_str()) {
                Some(existing) => existing.clone(),
                None => surge_ts_types::ObjectProperty {
                    index_slot: true,
                    ..surge_ts_types::ObjectProperty::required(
                        object_type.string_index_type.as_deref()?.clone(),
                    )
                },
            };
            existing.ty = crate::checks::function::settle_lazy_read(existing.ty);
            let (narrowed_ty, narrowed_optional) = if rest.is_empty() {
                match guard.narrow_leaf(&existing.ty, existing.optional) {
                    Some(narrowed) => narrowed,
                    // An index slot a guard proves present, or a write fills,
                    // is recorded even when its type stands: that is what
                    // keeps `noUncheckedIndexedAccess` from widening the read
                    // with `undefined` again.
                    None if existing.index_slot
                        && (guard.proves_defined()
                            || matches!(guard, ReferenceGuard::Assigned { .. })) =>
                    {
                        (existing.ty.clone(), false)
                    }
                    None => return None,
                }
            } else {
                // A truthy test further down the chain proves this link is
                // present too — the same fact that drops a nullish union
                // member below, applied to an `optional` property. That holds
                // even when the leaf itself does not change (`a.b?.c` with a
                // `string` `c`), so an unchanged leaf is not "nothing narrows".
                let deeper = narrow_property_path(&existing.ty, rest, guard);
                let proves_present = existing.optional && matches!(guard, ReferenceGuard::Truthy);
                if deeper.is_none() && !proves_present {
                    return None;
                }
                (
                    deeper.unwrap_or_else(|| existing.ty.clone()),
                    existing.optional && !proves_present,
                )
            };
            let properties = Arc::make_mut(&mut object_type.properties);
            properties.insert(
                head.as_str().into(),
                surge_ts_types::ObjectProperty {
                    ty: narrowed_ty,
                    optional: narrowed_optional,
                    method: existing.method,
                    readonly: existing.readonly,
                    restriction: existing.restriction.clone(),
                    index_slot: existing.index_slot,
                },
            );
            Some(Type::Object(object_type))
        }
        Type::Union(union) => {
            // A member that is an alias of a union (`TRPCConnectionState<E>`
            // beside the result shapes, as a reference or a union kept whole
            // for its alias display) is that union's members: tested whole,
            // every one of them failing reads as "nothing narrowed" and the
            // alias survives.
            let flattened: Vec<Type>;
            let union_members: &[Type] = if union
                .types()
                .iter()
                .any(|member| matches!(member, Type::Reference(_) | Type::Union(_)) && matches!(member.peeled(), Type::Union(_)))
            {
                flattened = union
                    .types()
                    .iter()
                    .flat_map(|member| match (member, member.peeled()) {
                        (Type::Reference(_) | Type::Union(_), Type::Union(inner)) => inner.types().to_vec(),
                        _ => vec![member.clone()],
                    })
                    .collect();
                &flattened
            } else {
                union.types()
            };
            let mut narrowed_any = false;
            let mut members: Vec<Type> = Vec::with_capacity(union_members.len());
            for member in union_members {
                match narrow_property_path(member, path, guard) {
                    Some(narrowed) => {
                        narrowed_any = true;
                        // tsc's `narrowTypeByDiscriminant`: a member whose
                        // property the guard leaves nothing of cannot be the
                        // narrowed value (`!o.r.success` drops `success: true`).
                        if !matches!(property_path_leaf_type(&narrowed, path), Some(Type::Never)) {
                            members.push(narrowed);
                        }
                    }
                    // A truthy test of `base?.p` also proves `base` itself is
                    // not nullish — a nullish base makes the whole chain
                    // `undefined`, which is falsy. Keeping the nullish member
                    // left `opts?.t ? f(opts.t) : …` reading `opts.t` as
                    // `string | undefined`.
                    None if guard.proves_defined()
                        && matches!(member, Type::Undefined | Type::Null | Type::Void) =>
                    {
                        narrowed_any = true;
                    }
                    // The leaf narrowers answer `None` both for "nothing to
                    // narrow" and for "this member cannot satisfy the guard at
                    // all", and keeping the member was wrong in the second case.
                    None if property_path_is_impossible(member, path, guard) => {
                        narrowed_any = true;
                    }
                    None if nullish_discriminant_rules_out(member, union_members, path, guard) => {
                        narrowed_any = true;
                    }
                    None => members.push(member.clone()),
                }
            }
            (narrowed_any && !members.is_empty()).then(|| union_type(members))
        }
        _ => None,
    }
}

/// Whether `guard` can never hold for `member` at `path`, which makes the member
/// an impossible shape of the narrowed value rather than an unchanged one.
///
/// The AWS SDK's union-member pattern is what needs this: every member of
/// `Field` declares the other members' keys as `?: never`, so
/// `field.arrayValue !== undefined` leaves exactly one member possible. At such
/// a leaf `never | undefined` collapses to plain `undefined`, which has no union
/// to split, so [`ReferenceGuard::narrow_leaf`] answers `None` — and `None` is
/// also what an already-narrow leaf answers. Read as "keep it", the union never
/// narrowed and every later `field.arrayValue` read stayed optional.
///
/// Only the not-nullish direction is decided here. The matching direction keeps
/// such a member (it *is* the undefined one), and the truthy guard reaches its
/// own arm above.
fn property_path_is_impossible(member: &Type, path: &[String], guard: ReferenceGuard<'_>) -> bool {
    // A literal test reached through a path discriminates the unions it
    // crosses: `w.thing && w.thing.kind === "a"` drops the members of
    // `w.thing` whose `kind` can never be `"a"`.
    if let ReferenceGuard::LiteralEquality {
        literal,
        keep_matching,
    } = guard
    {
        let (Some(literal), Some(leaf)) = (
            literal_expression_value(literal),
            property_path_leaf_type(member, path),
        ) else {
            return false;
        };
        let result = discriminant_type_match(&leaf, &literal);
        return if keep_matching {
            result == DiscriminantMatch::No
        } else {
            result == DiscriminantMatch::Yes
        };
    }
    let ReferenceGuard::Nullish {
        keep_matching: false,
        test,
    } = guard
    else {
        return false;
    };
    let Some(leaf) = property_path_leaf_type(member, path) else {
        return false;
    };
    let selected = |ty: &Type| match ty {
        Type::Null => test.null,
        Type::Undefined | Type::Void => test.undefined,
        Type::Never => true,
        _ => false,
    };
    match leaf {
        Type::Union(union) => union.types().iter().all(selected),
        other => selected(&other),
    }
}

/// tsc's `narrowTypeByDiscriminant` for a nullish equality: `null` and
/// `undefined` are unit types, so a property some member declares as one of
/// them discriminates the union, and `x.p === null` drops every member whose
/// `p` can never be `null`.
fn nullish_discriminant_rules_out(
    member: &Type,
    members: &[Type],
    path: &[String],
    guard: ReferenceGuard<'_>,
) -> bool {
    let ReferenceGuard::Nullish {
        keep_matching: true,
        test,
    } = guard
    else {
        return false;
    };
    let selects = |ty: &Type| match ty {
        Type::Null => test.null,
        Type::Undefined | Type::Void => test.undefined,
        _ => false,
    };
    let leaf_members = |ty: &Type| -> Option<Vec<Type>> {
        match property_path_leaf_type(ty, path)?.peeled() {
            Type::Union(union) => Some(union.types().to_vec()),
            other => Some(vec![other]),
        }
    };
    let Some(leaf) = leaf_members(member) else {
        return false;
    };
    let definitely_unselected = leaf.iter().all(|ty| {
        !selects(ty)
            && !ty.is_unknown()
            && !matches!(ty, Type::Any | Type::Never | Type::Reference(_))
    });
    definitely_unselected
        && members.iter().any(|other| {
            !std::ptr::eq(other, member)
                && leaf_members(other).is_some_and(|leaf| leaf.iter().any(selects))
        })
}

/// The declared type at the end of `path`, with an optional slot's `undefined`
/// put back. `None` when any link is missing or is not an object.
pub(super) fn property_path_leaf_type(ty: &Type, path: &[String]) -> Option<Type> {
    let (head, rest) = path.split_first()?;
    let Type::Object(object_type) = ty.peeled() else {
        return None;
    };
    let property = object_type.properties.get(head.as_str())?;
    let property_ty = crate::checks::function::settle_lazy_read(property.ty.clone());
    let effective = ReferenceGuard::effective_leaf_type(&property_ty, property.optional);
    if rest.is_empty() {
        return Some(effective);
    }
    property_path_leaf_type(&property_ty, rest)
}

/// The narrowed type of `base` under `guard` applied at `path`, or `None` when
/// nothing changes. An empty `path` guards the binding itself.
pub(super) fn narrowed_reference_type(ty: &Type, path: &[String], guard: ReferenceGuard<'_>) -> Option<Type> {
    let narrowed = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || match guard {
        _ if let Some((variable, constraint)) = union_constrained_variable(ty) => {
            let narrowed = match guard {
                ReferenceGuard::Nullish {
                    keep_matching,
                    test,
                } if path.is_empty() => narrow_binding_by_nullish(&constraint, keep_matching, test),
                _ if path.is_empty() => guard.narrow_leaf(&constraint, false).map(|(narrowed, _)| narrowed),
                _ => narrow_property_path(&constraint, path, guard),
            }?;
            Some(surge_ts_types::type_variable::intersect_type_variable(&variable, narrowed))
        }
        ReferenceGuard::Nullish {
            keep_matching,
            test,
        } if path.is_empty() => narrow_binding_by_nullish(ty, keep_matching, test),
        _ if path.is_empty() => guard.narrow_leaf(ty, false).map(|(narrowed, _)| narrowed),
        _ if ty.is_type_variable() => narrow_type_variable_root(ty, path, guard),
        // A member of the polymorphic `this` narrows the instance it stands
        // for, which stays that `this` (a branch join leaves a union of them).
        _ if let Some(instance) = polymorphic_this_instance(ty) => {
            narrow_property_path(&instance, path, guard).map(surge_ts_types::polymorphic_this_type)
        }
        _ => narrow_property_path(ty, path, guard),
    })?;
    (narrowed != *ty).then_some(narrowed)
}

/// tsc's `getNarrowableTypeForReference`: a type variable whose constraint is
/// a union narrows as that union, kept beside the variable (`T & C'`) — the
/// bare variable, or one an earlier guard already narrowed, with what is left
/// of its constraint.
pub(crate) fn union_constrained_variable(ty: &Type) -> Option<(Type, Type)> {
    let union_constraint = |variable: &Type| match variable {
        Type::TypeParameter(parameter) if variable.is_type_variable() => {
            surge_ts_types::type_variable::active_constraint(parameter)
                .flatten()
                .filter(|constraint| matches!(constraint.peeled(), Type::Union(_)))
        }
        _ => None,
    };
    match ty {
        Type::TypeParameter(_) => union_constraint(ty).map(|constraint| (ty.clone(), constraint)),
        Type::Object(object) if surge_ts_types::type_variable::is_narrowed_type_variable(ty) => {
            match object.intersection_operands.as_deref()? {
                // `T & {}`, what proving the variable non-nullish leaves.
                [variable, Type::Object(empty)]
                    if empty.properties.is_empty()
                        && empty.string_index_type.is_none()
                        && empty.number_index_type.is_none()
                        && empty.call_signature().is_none()
                        && empty.construct_signature().is_none()
                        && !empty.is_intersection =>
                {
                    let constraint = union_constraint(variable)?;
                    Some((variable.clone(), surge_ts_types::remove_nullish(&constraint)))
                }
                [variable, narrowed] if union_constraint(variable).is_some() => {
                    Some((variable.clone(), narrowed.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn polymorphic_this_instance(ty: &Type) -> Option<Type> {
    let is_this = |ty: &Type| matches!(ty, Type::Reference(reference) if reference.is_polymorphic_this());
    match ty {
        Type::Reference(_) if is_this(ty) => Some(surge_ts_types::without_polymorphic_this(ty.clone())),
        Type::Union(union) if union.types().iter().all(is_this) => Some(surge_ts_types::union_type(
            union.types().iter().cloned().map(surge_ts_types::without_polymorphic_this).collect(),
        )),
        _ => None,
    }
}

/// tsgo narrows a member reference below a type variable (`schema._zod.p`)
/// as a reference of its own, read through the variable's constraint; surge
/// narrows the root binding, so a bare variable becomes `T & C'` with `C'` its
/// constraint narrowed at the path — still a `T`, and read first where members
/// are read. A union constraint is narrowed as the union itself where tsgo
/// reads it (`getNarrowableTypeForReference`), every earlier guard included,
/// which one path's narrowing cannot stand in for.
fn narrow_type_variable_root(ty: &Type, path: &[String], guard: ReferenceGuard<'_>) -> Option<Type> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    let constraint = surge_ts_types::type_variable::active_constraint(parameter)??;
    if matches!(constraint.peeled(), Type::Union(_)) {
        return None;
    }
    let narrowed = narrow_property_path(&constraint, path, guard)?;
    Some(surge_ts_types::type_variable::intersect_type_variable(ty, narrowed))
}

/// Narrows the reference `target` (an `o.p` member expression) to what an
/// assignment of `assigned` leaves it able to be. Block-scoped like every other
/// in-scope narrowing, so a branch's assignment does not leak past `pop_child`.
pub(crate) fn narrow_assignment_target_in_scope(
    target: &ParsedExpression,
    assigned: &Type,
    scopes: &mut ScopeStack,
) {
    let Some((base, path)) = reference_path(target) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    // A binding no narrowing has touched yet is its own declaration.
    let declared = match scopes.visible_symbols().declared_type(&base) {
        Some(declared) => declared_path_type(declared, &path),
        None => scopes.resolve(&base).and_then(|symbol| declared_path_type(&symbol.ty, &path)),
    };
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::Assigned { assigned, declared: declared.as_ref() },
        scopes,
    );
}

/// The declared type of the property `path` reaches from a root's declared
/// type, when every link is a plain property of an object.
fn declared_path_type(declared: &Type, path: &[String]) -> Option<Type> {
    let mut current = declared.clone();
    for name in path {
        let Type::Object(object) = current.peeled() else {
            return None;
        };
        let property = object.properties.get(name.as_str())?;
        current = if property.optional {
            union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        };
    }
    Some(current)
}

/// Applies `guard` to `base`'s `path` in the *current* scope frame — see the
/// shadowing note in [`narrow_discriminant_in_scope`].
/// A function declaration is not a reference tsc narrows — only variables,
/// parameters and properties are — so `isFoo || isFoo()` still calls a
/// function, not the `never` a falsy `isFoo` would be.
fn is_unnarrowable_binding(symbol: &SymbolInfo, path: &[String]) -> bool {
    path.is_empty() && matches!(symbol.kind, crate::symbols::SymbolKind::Function)
}

pub(super) fn narrow_reference_in_scope(
    base: &str,
    path: &[String],
    guard: ReferenceGuard<'_>,
    scopes: &mut ScopeStack,
) {
    let Some(symbol) = scopes.resolve(base) else {
        return;
    };
    if is_unnarrowable_binding(symbol, path) {
        return;
    }
    let Some(narrowed) = narrowed_reference_type(&symbol.ty, path, guard) else {
        return;
    };
    let declared = symbol.ty.clone();
    let narrowed_symbol = SymbolInfo {
        ty: narrowed,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let _ = scopes.insert_current_narrowed(base.to_string(), narrowed_symbol, declared);
}

/// The reference guards a condition establishes for `branch_is_true`: a `typeof`
/// test on a reference, or every reference an `&&` chain proves truthy. Only
/// truthiness of the true branch is modelled — the falsy complement (`""`, `0`,
/// …) is not.
/// tsc's `narrowTypeByOptionalChainContainment`: the reference an optional
/// chain starts from is not nullish wherever the chain is known not to have
/// short-circuited — it equals a value that is never `undefined` (`null`
/// included, for `==`), or it differs from one that always is. `value_type`
/// answers the compared operand's type.
fn optional_chain_contained_receiver<'a>(
    condition: &'a ParsedExpression,
    branch_is_true: bool,
    value_type: &dyn Fn(&ParsedExpression) -> Option<Type>,
) -> Option<&'a ParsedExpression> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        operator,
        left,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let (equals, strict) = match operator {
        ParsedBinaryOperator::StrictEquals => (true, true),
        ParsedBinaryOperator::Equals => (true, false),
        ParsedBinaryOperator::StrictNotEquals => (false, true),
        ParsedBinaryOperator::NotEquals => (false, false),
        _ => return None,
    };
    let (chain, value) = if left.continues_optional_chain() {
        (left.as_ref(), right.as_ref())
    } else if right.continues_optional_chain() {
        (right.as_ref(), left.as_ref())
    } else {
        return None;
    };
    let value_type = match value {
        ParsedExpression::UndefinedLiteral => Type::Undefined,
        ParsedExpression::NullLiteral => Type::Null,
        other => value_type(other)?,
    };
    let nullable = |ty: &Type| match ty {
        Type::Undefined | Type::Void => true,
        Type::Null => !strict,
        _ => false,
    };
    let members: Vec<Type> = match value_type.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    let removes_nullable = if equals == branch_is_true {
        members.iter().all(|member| {
            !nullable(member) && !matches!(member, Type::Any) && !member.is_unknown()
        })
    } else {
        members.iter().all(nullable)
    };
    if !removes_nullable {
        return None;
    }
    optional_chain_receiver(chain)
}

/// The expression written before the chain's innermost `?.`.
/// The references an optional chain reads through before its last `?.`, in
/// tsc's `optionalChainContainsReference` order: `a?.b.c` yields `a?.b`, then
/// `a`. Empty for an expression that is not an optional chain.
pub(super) fn optional_chain_contained_references(chain: &ParsedExpression) -> Vec<&ParsedExpression> {
    let mut references = Vec::new();
    let mut link = chain;
    while link.continues_optional_chain() {
        let receiver = match link {
            ParsedExpression::OptionalPropertyAccess { object, .. }
            | ParsedExpression::OptionalIndexAccess { object, .. }
            | ParsedExpression::OptionalPropertyCall { object, .. }
            | ParsedExpression::PropertyAccess { object, .. }
            | ParsedExpression::ElementAccess { object, .. }
            | ParsedExpression::PropertyCall { object, .. } => object.as_ref(),
            ParsedExpression::OptionalCall { callee, .. }
            | ParsedExpression::ExpressionCall { callee, .. } => callee.as_ref(),
            ParsedExpression::NonNullAssertion { expression, .. } => expression.as_ref(),
            _ => break,
        };
        references.push(receiver);
        link = receiver;
    }
    references
}

fn optional_chain_receiver(chain: &ParsedExpression) -> Option<&ParsedExpression> {
    match chain {
        ParsedExpression::OptionalPropertyAccess { object, .. }
        | ParsedExpression::OptionalIndexAccess { object, .. }
        | ParsedExpression::OptionalPropertyCall { object, .. }
        | ParsedExpression::OptionalCall { callee: object, .. } => Some(object.as_ref()),
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::ElementAccess { object, .. }
        | ParsedExpression::PropertyCall { object, .. } => optional_chain_receiver(object),
        _ => None,
    }
}

/// The type of a compared operand that is itself a reference (`y`, `o.p`),
/// read off the type `root_type` gives its root — what tsc's
/// `getTypeOfExpression` answers for it at the comparison.
pub(super) fn reference_operand_type(
    expression: &ParsedExpression,
    root_type: &dyn Fn(&str) -> Option<Type>,
) -> Option<Type> {
    let (base, path) = reference_path(expression)?;
    let ty = root_type(&base)?;
    if path.is_empty() {
        Some(ty)
    } else {
        property_path_leaf_type(&ty, &path)
    }
}

/// tsc's `narrowTypeByEquality` for the binding `name` of type `ty`, when it is
/// an operand of an equality test whose compared value is a reference rather
/// than a literal the literal narrowers spell. `isMatchingReference` tries the
/// left operand first, so a binding compared with itself reads the right one.
pub(super) fn narrow_binding_by_reference_equality(
    condition: &ParsedExpression,
    name: &str,
    ty: &Type,
    branch_is_true: bool,
    root_type: &dyn Fn(&str) -> Option<Type>,
) -> Option<Type> {
    let test = parse_equality_test(condition)?;
    let value = test.operand_pairs().into_iter().find_map(|(subject, value)| {
        matches!(reference_path(subject), Some((base, path)) if path.is_empty() && base == name)
            .then_some(value)
    })?;
    let value_type = reference_operand_type(value, root_type)?;
    let narrowed = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        narrow_type_by_equality(
            ty,
            &value_type,
            test.double_equals,
            test.assume_true(branch_is_true),
        )
    });
    (narrowed != *ty).then_some(narrowed)
}

/// [`narrow_binding_by_reference_equality`] for every binding the test
/// compares (`y === z` narrows both), each read before either narrows.
pub(super) fn reference_equality_narrowings(
    condition: &ParsedExpression,
    branch_is_true: bool,
    current: &dyn Fn(&str) -> Option<SymbolInfo>,
    root_type: &dyn Fn(&str) -> Option<Type>,
) -> Vec<(String, SymbolInfo, Type)> {
    let Some(test) = parse_equality_test(condition) else {
        return Vec::new();
    };
    let mut narrowings: Vec<(String, SymbolInfo, Type)> = Vec::new();
    for (subject, _) in test.operand_pairs() {
        let Some((name, path)) = reference_path(subject) else {
            continue;
        };
        if !path.is_empty() || narrowings.iter().any(|(narrowed, ..)| *narrowed == name) {
            continue;
        }
        let Some(symbol) = current(&name) else {
            continue;
        };
        if is_unnarrowable_binding(&symbol, &path) {
            continue;
        }
        let Some(narrowed) = narrow_binding_by_reference_equality(
            condition,
            &name,
            &symbol.ty,
            branch_is_true,
            root_type,
        ) else {
            continue;
        };
        let declared = symbol.ty.clone();
        narrowings.push((name, SymbolInfo { ty: narrowed, ..symbol }, declared));
    }
    narrowings
}

/// The type of an operand a guard compares with, as far as the symbol table
/// alone answers it: a literal, a binding, or an enum member (`T.Id`).
pub(super) fn compared_operand_type(
    symbols: &SymbolTable,
) -> impl Fn(&ParsedExpression) -> Option<Type> + '_ {
    move |expression| match expression {
        ParsedExpression::Identifier { name, .. } => {
            symbols.get(name).map(|symbol| symbol.ty.clone())
        }
        other => super::guards::literal_expression_value(other)
            .or_else(|| super::guards::const_member_literal_value(other, symbols)),
    }
}

/// tsc's `narrowTypeByConstructor`, for `x.constructor == C` and
/// `x["constructor"] == C` written either way round: the reference whose
/// constructor is compared, the constructor's name, and whether the operator is
/// an equality. tsc narrows only in the branch where the comparison holds.
fn parse_constructor_equality(
    condition: &ParsedExpression,
) -> Option<((String, Vec<String>), &str, bool)> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        operator,
        left,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let eq = match operator {
        ParsedBinaryOperator::StrictEquals | ParsedBinaryOperator::Equals => true,
        ParsedBinaryOperator::StrictNotEquals | ParsedBinaryOperator::NotEquals => false,
        _ => return None,
    };
    let constructor_of = |expression: &ParsedExpression| -> Option<(String, Vec<String>)> {
        let is_constructor_key =
            |index: &ParsedExpression| matches!(index, ParsedExpression::StringLiteral(key) if key == "constructor");
        match expression {
            ParsedExpression::PropertyAccess {
                object,
                property_name,
                ..
            } if property_name == "constructor" => reference_path(object),
            ParsedExpression::ElementAccess { object, index, .. } if is_constructor_key(index) => {
                reference_path(object)
            }
            ParsedExpression::IndexAccess {
                object_name, index, ..
            } if is_constructor_key(index) => Some((object_name.clone(), Vec::new())),
            _ => None,
        }
    };
    fn constructor_name(expression: &ParsedExpression) -> Option<&str> {
        match expression {
            ParsedExpression::Identifier { name, .. } => Some(name.as_str()),
            ParsedExpression::PropertyAccess {
                object,
                property_name,
                ..
            } if matches!(object.as_ref(), ParsedExpression::Identifier { .. })
                && property_name != "constructor" =>
            {
                Some(property_name.as_str())
            }
            _ => None,
        }
    }
    if let (Some(reference), Some(name)) = (constructor_of(left), constructor_name(right)) {
        return Some((reference, name, eq));
    }
    let (reference, name) = (constructor_of(right)?, constructor_name(left)?);
    Some((reference, name, eq))
}

/// The members of a union that `ctor_name` itself constructs (tsc's
/// `isConstructedBy`): a class instance of exactly that class — a subclass has
/// its own constructor — a primitive for its wrapper, an array for `Array`.
/// What surge cannot judge stays. `None` when nothing would change, or when
/// nothing would be left: the match is by name, and an empty result is more
/// likely a name surge did not recognize than a contradiction in the source.
fn narrow_union_by_constructor(ty: &Type, ctor_name: &str) -> Option<Type> {
    let Type::Union(union) = ty.peeled() else {
        return None;
    };
    let constructed_by = |member: &Type| -> bool {
        match member {
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_) => true,
            Type::Number | Type::NumberLiteral(_) => ctor_name == "Number",
            Type::String | Type::StringLiteral(_) => ctor_name == "String",
            Type::Boolean | Type::BooleanLiteral(_) => ctor_name == "Boolean",
            Type::BigInt => ctor_name == "BigInt",
            Type::Symbol => ctor_name == "Symbol",
            Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => ctor_name == "Array",
            Type::Undefined | Type::Null | Type::Void | Type::Never => false,
            Type::Function(_) => ctor_name == "Function",
            other => {
                let name = other.name();
                let base = name.split('<').next().unwrap_or(name.as_str());
                base == ctor_name || base.rsplit('.').next() == Some(ctor_name)
            }
        }
    };
    let members = union.types();
    let kept: Vec<Type> = members.iter().filter(|member| constructed_by(member)).cloned().collect();
    if kept.is_empty() || kept.len() == members.len() {
        return None;
    }
    Some(union_type(kept))
}

/// The statement form of the constructor guard in [`collect_reference_guards`].
pub(super) fn narrow_constructor_equality_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some(((base, path), ctor_name, eq)) = parse_constructor_equality(condition) else {
        return false;
    };
    if eq == branch_is_true {
        narrow_reference_in_scope(&base, &path, ReferenceGuard::ConstructedBy { ctor_name }, scopes);
    }
    true
}

/// The statement form of the containment guard in [`collect_reference_guards`].
pub(super) fn narrow_optional_call_containment_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let receiver = {
        let value_type = compared_operand_type(scopes.visible_symbols());
        optional_chain_contained_receiver(condition, branch_is_true, &value_type)
            .or_else(|| match parse_typeof_condition(condition) {
                Some((operand, tag, eq)) if (branch_is_true == eq) != (tag == "undefined") => {
                    optional_chain_receiver(operand)
                }
                _ => None,
            })
            .or_else(|| match parse_instanceof_condition(condition) {
                Some((operand, _)) if branch_is_true => optional_chain_receiver(operand),
                _ => None,
            })
            .and_then(reference_path)
    };
    let Some((base, path)) = receiver else {
        return false;
    };
    narrow_reference_in_scope(&base, &path, ReferenceGuard::Truthy, scopes);
    // Not the end of it: `o?.kind === "a"` still discriminates `o`.
    false
}

pub(super) fn collect_reference_guards<'a>(
    condition: &'a ParsedExpression,
    branch_is_true: bool,
    value_type: &dyn Fn(&ParsedExpression) -> Option<Type>,
    guards: &mut Vec<(String, Vec<String>, ReferenceGuard<'a>)>,
) {
    if let ParsedExpression::Logical {
        left,
        operator: ParsedLogicalOperator::And,
        right,
        ..
    } = condition
    {
        if branch_is_true {
            collect_reference_guards(left, true, value_type, guards);
            collect_reference_guards(right, true, value_type, guards);
        }
        return;
    }
    // By De Morgan every operand of an `||` fails where the whole test does, so
    // `if (!s?.program || !s.map) return;` leaves both references truthy.
    if let ParsedExpression::Logical {
        left,
        operator: ParsedLogicalOperator::Or,
        right,
        ..
    } = condition
    {
        if !branch_is_true {
            collect_reference_guards(left, false, value_type, guards);
            collect_reference_guards(right, false, value_type, guards);
        }
        return;
    }

    // `!!x` is `x`'s truthiness spelled out; as an `&&` operand (`!!query &&
    // query.isFetched()`, or an alias of it) it proves the same thing the bare
    // reference does. Unwrapped here so every guard below sees the reference.
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
        && let ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand: inner,
            ..
        } = operand.as_ref()
    {
        collect_reference_guards(inner, branch_is_true, value_type, guards);
        return;
    }
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    {
        collect_reference_guards(operand, !branch_is_true, value_type, guards);
        return;
    }

    // `o?.f() === value` holds only where `o` is not nullish. The comparison
    // may narrow further below (`o?.kind === "a"` still discriminates `o`).
    if let Some(receiver) =
        optional_chain_contained_receiver(condition, branch_is_true, value_type)
        && let Some((base, path)) = reference_path(receiver)
    {
        guards.push((base, path, ReferenceGuard::Truthy));
    }
    // A truthy chain proves each reference it reads through non-nullish
    // (tsc's `optionalChainContainsReference` in `narrowTypeByTruthiness`).
    if branch_is_true && surge_ts_types::strict_null_checks() {
        for receiver in optional_chain_contained_references(condition) {
            if let Some((base, path)) = reference_path(receiver) {
                guards.push((
                    base,
                    path,
                    ReferenceGuard::Nullish {
                        keep_matching: false,
                        test: super::guards::NullishTest {
                            null: true,
                            undefined: true,
                        },
                    },
                ));
            }
        }
    }

    if let Some(((base, path), ctor_name, eq)) = parse_constructor_equality(condition) {
        if eq == branch_is_true {
            guards.push((base, path, ReferenceGuard::ConstructedBy { ctor_name }));
        }
        return;
    }

    if let Some((operand, tag, eq)) = parse_typeof_condition(condition) {
        // A chain whose `typeof` is anything but `"undefined"` did not
        // short-circuit, so what it starts from is present.
        if (branch_is_true == eq) != (tag == "undefined")
            && let Some((base, path)) = optional_chain_receiver(operand).and_then(reference_path)
        {
            guards.push((base, path, ReferenceGuard::Truthy));
        }
        if let Some((base, path)) = reference_path(operand) {
            guards.push((
                base,
                path,
                ReferenceGuard::Typeof {
                    tag,
                    keep_matching: branch_is_true == eq,
                },
            ));
        }
        return;
    }

    if let Some((operand, ctor_name)) = parse_instanceof_condition(condition) {
        if branch_is_true
            && let Some((base, path)) = optional_chain_receiver(operand).and_then(reference_path)
        {
            guards.push((base, path, ReferenceGuard::Truthy));
        }
        let constructor = value_type(&ParsedExpression::Identifier {
            name: ctor_name.to_string(),
            span: None,
        });
        if let Some((base, path)) = reference_path(operand)
            && !constructs_only_any(constructor.as_ref())
        {
            guards.push((
                base,
                path,
                ReferenceGuard::Instanceof {
                    ctor_name,
                    instance: None,
                    keep_matching: branch_is_true,
                },
            ));
        }
        return;
    }

    if let Some(operand) = parse_array_isarray_condition(condition) {
        if let Some((base, path)) = reference_path(operand) {
            guards.push((
                base,
                path,
                ReferenceGuard::Arrayness {
                    keep_arrays: branch_is_true,
                },
            ));
        }
        return;
    }

    if let Some(operand) = parse_arraybuffer_isview_condition(condition) {
        if let Some((base, path)) = reference_path(operand) {
            guards.push((
                base,
                path,
                ReferenceGuard::ArrayBufferView {
                    keep_views: branch_is_true,
                },
            ));
        }
        return;
    }

    if let Some((subject, eq, test)) = parse_nullish_equality_condition(condition) {
        if let Some((base, path)) = reference_path(subject) {
            guards.push((
                base,
                path,
                ReferenceGuard::Nullish {
                    keep_matching: branch_is_true == eq,
                    test,
                },
            ));
        }
        return;
    }

    if let Some((object, property)) = parse_in_condition(condition) {
        if let Some((base, path)) = reference_path(object) {
            let member = surge_ts_types::exact_optional_property_types().then(|| {
                let mut member = path.clone();
                member.push(property.to_string());
                (base.clone(), member)
            });
            guards.push((
                base,
                path,
                ReferenceGuard::PropertyPresence {
                    property,
                    keep_present: branch_is_true,
                },
            ));
            if let Some((base, member)) = member {
                guards.push((
                    base,
                    member,
                    ReferenceGuard::ExactOptionalPresence {
                        keep_present: branch_is_true,
                    },
                ));
            }
        }
        return;
    }

    if let Some((reference, literal, eq)) = parse_reference_literal_equality(condition) {
        if let Some((base, path)) = reference_path(reference)
            && !path.is_empty()
        {
            guards.push((
                base,
                path,
                ReferenceGuard::LiteralEquality {
                    literal,
                    keep_matching: branch_is_true == eq,
                },
            ));
        }
        return;
    }

    // `a?.b()` being truthy proves `a` present: a short-circuited chain is
    // `undefined`. `reference_path` stops at the call, so test its receiver.
    let tested = match condition {
        ParsedExpression::OptionalPropertyCall { object, .. }
        | ParsedExpression::OptionalCall { callee: object, .. } => object.as_ref(),
        other => other,
    };
    if branch_is_true && let Some((base, path)) = reference_path(tested) {
        guards.push((base, path, ReferenceGuard::Truthy));
    } else if !branch_is_true && let Some((base, path)) = reference_path(condition) {
        guards.push((base, path, ReferenceGuard::Falsy));
    }
}

pub(super) fn type_includes_undefined_member(ty: &Type) -> bool {
    match ty {
        Type::Undefined => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| matches!(member, Type::Undefined)),
        _ => false,
    }
}

/// Narrows `symbols` by a truthy or `typeof` guard on a reference the
/// identifier-keyed guards above do not reach — `o.p ? o.p : …`,
/// `typeof o.p === "string" ? o.p : …`. Returns `None` when nothing narrows.
pub(super) fn narrow_reference_guard_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let mut guards = Vec::new();
    collect_reference_guards(
        condition,
        branch_is_true,
        &compared_operand_type(symbols),
        &mut guards,
    );
    let mut unit_tests = Vec::new();
    super::collect_unit_member_discriminants(condition, branch_is_true, symbols, &mut unit_tests);
    if guards.is_empty() && unit_tests.is_empty() {
        return None;
    }
    guards.extend(unit_tests.iter().map(|(base, path, property, literal, keep_matching)| {
        (
            base.clone(),
            path.clone(),
            ReferenceGuard::Discriminant { property, literal, keep_matching: *keep_matching },
        )
    }));

    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let mut changed = false;
    for (base, path, guard) in guards {
        let Some(symbol) = narrowed_symbols.get(&base) else {
            continue;
        };
        if is_unnarrowable_binding(symbol, &path) {
            continue;
        }
        let Some(narrowed) = narrowed_reference_type(&symbol.ty, &path, guard) else {
            continue;
        };
        let declared = symbol.ty.clone();
        let narrowed_symbol = SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        narrowed_symbols.insert_narrowed(base, narrowed_symbol, declared);
        changed = true;
    }

    changed.then_some(narrowed_symbols)
}

/// Applies `o.p === "lit"` / `o?.p !== 3` narrowing to the property `o.p` in
/// place. Returns whether a reference test was recognised (not whether it
/// narrowed): the caller keeps going either way.
pub(super) fn narrow_literal_equality_reference_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some((reference, literal, eq)) = parse_reference_literal_equality(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(reference) else {
        return false;
    };
    if path.is_empty() {
        return false;
    }
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::LiteralEquality {
            literal,
            keep_matching: branch_is_true == eq,
        },
        scopes,
    );
    true
}
