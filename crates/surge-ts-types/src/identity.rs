//! tsc's identity relation (`isTypeIdenticalTo`): what TS2403 requires of
//! every redeclaration of a `var`. Stricter than mutual assignability —
//! `any` is identical only to `any`, and optionality, `readonly` and member
//! visibility must agree — but blind to how a type was written: an alias, a
//! mapped type over a closed key set, a reordered union, and a callable
//! object literal all denote the same type as their expansion.

use crate::{FunctionType, ObjectType, Type};

/// Past this depth a recursive type is taken as identical, as tsc's relation
/// cache assumes a cycle it is already comparing to hold.
const MAX_DEPTH: usize = 16;

pub fn is_type_identical_to(left: &Type, right: &Type) -> bool {
    identical(left, right, MAX_DEPTH)
}

fn identical(left: &Type, right: &Type, depth: usize) -> bool {
    if left == right || depth == 0 {
        return true;
    }
    // The polymorphic `this` is a type parameter to tsc, identical only to
    // itself; peeling would read it as the instance it stands for.
    let polymorphic_this = |ty: &Type| matches!(ty, Type::Reference(reference) if reference.is_polymorphic_this());
    if polymorphic_this(left) != polymorphic_this(right) {
        return false;
    }
    // Peeling drops the `readonly` wrapper, but `readonly [A]` is another
    // tuple target than `[A]`, and `ReadonlyArray<T>` another generic than
    // `Array<T>`.
    if is_readonly_array_like(left) != is_readonly_array_like(right) {
        return false;
    }
    // Likewise a written tuple's optional elements are flags on its target.
    if fixed_tuple_min_length(left) != fixed_tuple_min_length(right) {
        return false;
    }
    let (left, right) = (left.peeled(), right.peeled());
    if left == right {
        return true;
    }
    match (&left, &right) {
        (Type::Union(left), Type::Union(right)) => {
            members_identical(left.types(), right.types(), depth)
        }
        (Type::Boolean, Type::Union(union)) | (Type::Union(union), Type::Boolean) => {
            members_identical(
                &[Type::BooleanLiteral(false), Type::BooleanLiteral(true)],
                union.types(),
                depth,
            )
        }
        (Type::Object(left), Type::Object(right)) => objects_identical(left, right, depth),
        (Type::Function(function), Type::Object(object))
        | (Type::Object(object), Type::Function(function)) => {
            object_is_bare_call_signature(object).is_some_and(|signature| {
                signatures_identical(function, signature, depth)
            })
        }
        (Type::Function(left), Type::Function(right)) => signatures_identical(left, right, depth),
        // A mapping reached through an alias is its own variable, the same
        // deferred type as the mapping written out.
        (Type::TypeParameter(left), Type::TypeParameter(right)) => {
            crate::type_variable::deferred_type(left)
                .is_some_and(|kind| crate::type_variable::deferred_type(right).is_some_and(|other| other == kind))
        }
        (Type::Array(left), Type::Array(right)) => identical(left, right, depth - 1),
        (Type::Tuple(left), Type::Tuple(right)) => elements_identical(left, right, depth),
        (Type::OpenTuple(left), Type::OpenTuple(right)) => {
            elements_identical(&left.leading, &right.leading, depth)
                && identical(&left.rest, &right.rest, depth - 1)
                && elements_identical(&left.trailing, &right.trailing, depth)
        }
        _ => false,
    }
}

fn is_readonly_array_like(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => {
            reference.is_readonly_array() || is_readonly_array_like(&reference.resolve_arc())
        }
        _ => false,
    }
}

fn fixed_tuple_min_length(ty: &Type) -> Option<usize> {
    match ty {
        Type::Reference(reference) => match reference.written_tuple() {
            Some((_, min_length)) => Some(min_length),
            None => fixed_tuple_min_length(&reference.resolve_arc()),
        },
        other => crate::fixed_tuple_parts(other).map(|(_, min_length)| min_length),
    }
}

/// Two tuples are one target only with the same element positions, each
/// element identical to its counterpart.
fn elements_identical(left: &[Type], right: &[Type], depth: usize) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| identical(left, right, depth - 1))
}

fn members_identical(left: &[Type], right: &[Type], depth: usize) -> bool {
    let covers = |from: &[Type], to: &[Type]| {
        from.iter()
            .all(|member| to.iter().any(|other| identical(member, other, depth - 1)))
    };
    covers(left, right) && covers(right, left)
}

fn objects_identical(left: &ObjectType, right: &ObjectType, depth: usize) -> bool {
    if left.properties.len() != right.properties.len()
        || !optional_identical(left.string_index_type.as_deref(), right.string_index_type.as_deref(), depth)
        || !optional_identical(left.number_index_type.as_deref(), right.number_index_type.as_deref(), depth)
        || !optional_signatures_identical(left.call_signature.as_deref(), right.call_signature.as_deref(), depth)
        || !optional_signatures_identical(
            left.construct_signature.as_deref(),
            right.construct_signature.as_deref(),
            depth,
        )
    {
        return false;
    }
    left.properties.iter().all(|(name, property)| {
        right.properties.get(name).is_some_and(|other| {
            property.optional == other.optional
                && property.readonly == other.readonly
                && property.restriction == other.restriction
                && identical(&member_type(property), &member_type(other), depth - 1)
        })
    })
}

/// The type `compareProperties` reads off a member, `getNonMissingTypeOfSymbol`:
/// under `strictNullChecks` an optional one carries the `undefined` its
/// optionality adds (`addOptionality`), so `a?: number` and `a?: number |
/// undefined` are one member. Under `exactOptionalPropertyTypes` that is the
/// missing type, which the read removes again.
fn member_type(property: &crate::ObjectProperty) -> Type {
    if property.optional && crate::strict_null_checks() && !crate::exact_optional_property_types() {
        crate::union_type(vec![property.ty.clone(), Type::Undefined])
    } else {
        property.ty.clone()
    }
}

/// An object type literal whose only member is one call signature is the
/// function type `{ (n: number): string }` spells.
fn object_is_bare_call_signature(object: &ObjectType) -> Option<&FunctionType> {
    (object.properties.is_empty()
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.construct_signature.is_none())
    .then_some(object.call_signature.as_deref())
    .flatten()
}

fn optional_identical(left: Option<&Type>, right: Option<&Type>, depth: usize) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => identical(left, right, depth - 1),
        _ => false,
    }
}

fn optional_signatures_identical(
    left: Option<&FunctionType>,
    right: Option<&FunctionType>,
    depth: usize,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => signatures_identical(left, right, depth),
        _ => false,
    }
}

/// `compareSignaturesIdentical` reads the source through the target's type
/// parameters. A written generic signature resolves its own to the sentinel
/// while a generic arrow keeps them named, so both are read with their own
/// erased to it.
fn signatures_identical(left: &FunctionType, right: &FunctionType, depth: usize) -> bool {
    let (left_names, right_names) = (left.type_parameter_names(), right.type_parameter_names());
    let erased = |ty: &Type, names: &[String]| crate::assignability::erase_type_parameters(ty, names);
    left.parameters().len() == right.parameters().len()
        && left.is_variadic() == right.is_variadic()
        && left.required_parameter_count() == right.required_parameter_count()
        && left_names.len() == right_names.len()
        && left
            .parameters()
            .iter()
            .zip(right.parameters())
            .all(|(left, right)| identical(&erased(left, &left_names), &erased(right, &right_names), depth - 1))
        && identical(
            &erased(left.return_type(), &left_names),
            &erased(right.return_type(), &right_names),
            depth - 1,
        )
}
