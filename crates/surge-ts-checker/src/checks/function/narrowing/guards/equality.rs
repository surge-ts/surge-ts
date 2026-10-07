use surge_ts_syntax::{ParsedBinaryOperator, ParsedExpression};
use surge_ts_types::{Type, is_comparable_to, union_type};

use super::{NullishTest, narrow_binding_by_nullish};

/// An `==`, `!=`, `===` or `!==` test, read the way `narrowTypeByBinaryExpression`
/// reads it: whichever operand is the narrowed reference, the other one is the
/// compared value.
#[derive(Clone, Copy)]
pub(crate) struct EqualityTest<'a> {
    pub(crate) left: &'a ParsedExpression,
    pub(crate) right: &'a ParsedExpression,
    pub(crate) negated: bool,
    pub(crate) double_equals: bool,
}

impl<'a> EqualityTest<'a> {
    /// The operands as `(subject, value)` pairs, left first.
    pub(crate) fn operand_pairs(self) -> [(&'a ParsedExpression, &'a ParsedExpression); 2] {
        [(self.left, self.right), (self.right, self.left)]
    }

    /// Whether the comparison, as written, holds in `branch_is_true`.
    pub(crate) fn assume_true(self, branch_is_true: bool) -> bool {
        branch_is_true != self.negated
    }
}

pub(crate) fn parse_equality_test(condition: &ParsedExpression) -> Option<EqualityTest<'_>> {
    let ParsedExpression::Binary {
        left,
        operator,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let (negated, double_equals) = match operator {
        ParsedBinaryOperator::StrictEquals => (false, false),
        ParsedBinaryOperator::StrictNotEquals => (true, false),
        ParsedBinaryOperator::Equals => (false, true),
        ParsedBinaryOperator::NotEquals => (true, true),
        _ => return None,
    };
    Some(EqualityTest {
        left,
        right,
        negated,
        double_equals,
    })
}

/// tsc's `narrowTypeByEquality` (flow.go). `assume_true` is whether the
/// comparison holds, the `!=`/`!==` flip already applied. The literal and
/// nullish narrowers keep their own spellings of the literal operands; this is
/// the rule for any other compared value, whose type alone decides.
pub(crate) fn narrow_type_by_equality(
    ty: &Type,
    value: &Type,
    double_equals: bool,
    assume_true: bool,
) -> Type {
    if !narrowable_subject(ty) || !decidable_value(value) {
        return ty.clone();
    }
    if matches!(value, Type::Undefined | Type::Null) {
        if !surge_ts_types::strict_null_checks() {
            return ty.clone();
        }
        let null = double_equals || matches!(value, Type::Null);
        let test = NullishTest {
            null,
            undefined: double_equals || matches!(value, Type::Undefined),
        };
        return narrow_binding_by_nullish(ty, assume_true, test).unwrap_or_else(|| ty.clone());
    }
    if assume_true {
        if !double_equals
            && (matches!(ty, Type::GenuineUnknown) || some_member(ty, is_empty_anonymous_object))
        {
            if is_primitive(value) || is_non_primitive(value) || is_empty_anonymous_object(value) {
                return value.clone();
            }
            if is_object_flagged(value) {
                return non_primitive_object();
            }
        }
        if !double_equals
            && is_primitive(value)
            && is_uniform_union(ty)
            && as_union(ty).is_some_and(|union| union_members(&union).contains(value))
        {
            return value.clone();
        }
        let filtered = filter_type(ty, &|member| {
            are_types_comparable(member, value)
                || double_equals && is_coercible_under_double_equals(member, value)
        });
        return replace_primitives_with_literals(&filtered, value);
    }
    if is_unit_type(value) {
        if is_uniform_union(ty) {
            let filtered = remove_type(ty, value);
            if filtered != *ty {
                return filtered;
            }
        }
        return filter_type(ty, &|member| {
            !(is_unit_like_type(member) && are_types_comparable(member, value))
        });
    }
    ty.clone()
}

/// A subject surge can filter: not `any`, not a degraded or placeholder type,
/// and not a type variable, whose narrowing goes through its constraint.
fn narrowable_subject(ty: &Type) -> bool {
    !matches!(ty, Type::Any | Type::Never)
        && (matches!(ty, Type::GenuineUnknown) || !ty.is_unknown())
}

/// A compared value whose type can rule anything out. `unknown` and `any` are
/// comparable to every member, and a degraded type says nothing.
fn decidable_value(value: &Type) -> bool {
    !matches!(value, Type::Any) && !value.is_unknown()
}

fn are_types_comparable(left: &Type, right: &Type) -> bool {
    is_comparable_to(left, right) || is_comparable_to(right, left)
}

fn is_coercible_under_double_equals(source: &Type, target: &Type) -> bool {
    matches!(source, Type::Number | Type::String | Type::BooleanLiteral(_))
        && matches!(target, Type::Number | Type::String | Type::Boolean)
}

/// The constituents `filterType` visits: a union's members, with `boolean`
/// read as the `true | false` union it is to tsc.
fn union_members(ty: &Type) -> Vec<Type> {
    let mut members = Vec::new();
    match ty {
        Type::Union(union) => union.types().iter().for_each(|member| push_member(member, &mut members)),
        other => push_member(other, &mut members),
    }
    members
}

/// A union's members as `filterType` sees them, lazy alias instantiations
/// read as what they evaluate to (see [`push_member`]); `boolean` stays whole.
pub(crate) fn evaluated_union_members(members: &[Type]) -> Vec<Type> {
    let mut out = Vec::new();
    for member in members {
        match member {
            Type::Boolean => out.push(Type::Boolean),
            other => push_member(other, &mut out),
        }
    }
    out
}

/// What a lazy alias instantiation may stand for in a union: a primitive or a
/// `unique symbol`, never another reference that names a declaration.
fn evaluated_primitive(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => reference.is_unique_symbol(),
        other => is_primitive(other),
    }
}

/// A member that is still a lazy alias instantiation (`Awaited<"abort">`, left
/// by distributing a conditional over a union) is the primitive it evaluates
/// to, which tsc has already resolved when building the union.
fn push_member(member: &Type, members: &mut Vec<Type>) {
    match member {
        Type::Boolean => {
            members.push(Type::BooleanLiteral(true));
            members.push(Type::BooleanLiteral(false));
        }
        Type::Reference(reference)
            if !reference.is_unique_symbol()
                && reference.enum_owner.is_none()
                && !is_pattern_literal(member) =>
        {
            // One level at a time: peeling all the way would read a
            // `unique symbol` the alias evaluates to as plain `symbol`.
            let evaluated = match reference.resolve() {
                unique @ Type::Reference(_) if matches!(&unique, Type::Reference(inner) if inner.is_unique_symbol()) => unique,
                other => other.peeled(),
            };
            match evaluated {
                Type::Union(union) if union.types().iter().all(evaluated_primitive) => {
                    union.types().iter().for_each(|inner| push_member(inner, members));
                }
                peeled if evaluated_primitive(&peeled) => push_member(&peeled, members),
                _ => members.push(member.clone()),
            }
        }
        other => members.push(other.clone()),
    }
}

/// The union a type is to tsc: a union, `boolean`, or a reference naming one
/// (an alias of a union, a union enum). Any other reference is one constituent.
fn as_union(ty: &Type) -> Option<Type> {
    match ty {
        Type::Union(_) | Type::Boolean => Some(ty.clone()),
        Type::Reference(reference) if !reference.is_unique_symbol() => match ty.peeled() {
            peeled @ Type::Union(_) => Some(peeled),
            _ => None,
        },
        _ => None,
    }
}

fn some_member(ty: &Type, predicate: fn(&Type) -> bool) -> bool {
    match as_union(ty) {
        Some(union) => union_members(&union).iter().any(predicate),
        None => predicate(ty),
    }
}

/// tsc's `filterType`: the union constituents `keep` accepts, or the type
/// itself when it is not a union and `keep` accepts it, `never` otherwise.
fn filter_type(ty: &Type, keep: &dyn Fn(&Type) -> bool) -> Type {
    if matches!(ty, Type::Never) {
        return Type::Never;
    }
    let Some(union) = as_union(ty) else {
        return if keep(ty) { ty.clone() } else { Type::Never };
    };
    let members = union_members(&union);
    let kept: Vec<Type> = members.iter().filter(|member| keep(member)).cloned().collect();
    if kept.len() == members.len() {
        return ty.clone();
    }
    if kept.is_empty() {
        return Type::Never;
    }
    union_type(kept)
}

fn remove_type(ty: &Type, target: &Type) -> Type {
    filter_type(ty, &|member| member != target)
}

/// tsc's `isUnitType`: a literal, `undefined`, `null`, an enum member or a
/// `unique symbol`.
fn is_unit_type(ty: &Type) -> bool {
    match ty {
        Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Undefined
        | Type::Null => true,
        Type::Reference(reference) => {
            reference.is_unique_symbol() || enum_member_literal(ty).is_some()
        }
        _ => false,
    }
}

/// tsc's `isUnitLikeType`: a branded literal (`"a" & { tag: 1 }`) counts, as
/// its unit operand makes it one.
fn is_unit_like_type(ty: &Type) -> bool {
    match ty {
        Type::Object(object) if object.is_intersection => object
            .intersection_operands
            .as_deref()
            .is_some_and(|operands| operands.iter().any(is_unit_type)),
        other => is_unit_type(other),
    }
}

fn enum_member_literal(ty: &Type) -> Option<Type> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    reference.enum_owner.as_ref()?;
    match ty.peeled() {
        literal @ (Type::StringLiteral(_) | Type::NumberLiteral(_)) => Some(literal),
        _ => None,
    }
}

/// tsc's `TypeFlagsPrimitive` on a single type; a union enum carries it too.
fn is_primitive(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Void
        | Type::Undefined
        | Type::Null
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Reference(reference) => {
            reference.is_unique_symbol() || reference.enum_owner.is_some() || is_pattern_literal(ty)
        }
        _ => false,
    }
}

/// The `object` keyword.
fn is_non_primitive(ty: &Type) -> bool {
    matches!(ty, Type::Object(object) if object.non_primitive && object.properties.is_empty())
}

/// tsc's `TypeFlagsObject`: an object, function, array or tuple type — not the
/// `object` keyword or an intersection.
fn is_object_flagged(ty: &Type) -> bool {
    match ty {
        Type::Object(object) => !object.non_primitive && !object.is_intersection,
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Reference(reference)
            if !reference.is_unique_symbol() && reference.enum_owner.is_none() =>
        {
            !is_pattern_literal(ty) && is_object_flagged(&ty.peeled())
        }
        _ => false,
    }
}

/// tsc's `isEmptyAnonymousObjectType`: a memberless type literal (`{}`), not an
/// interface or class that happens to declare nothing.
fn is_empty_anonymous_object(ty: &Type) -> bool {
    let Type::Object(object) = ty else {
        return false;
    };
    object.properties.is_empty()
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature.is_none()
        && object.construct_signature.is_none()
        && !object.non_primitive
        && !object.is_intersection
        && !object.without_inferable_index
}

fn is_pattern_literal(ty: &Type) -> bool {
    surge_ts_types::is_template_literal_type(ty)
        || surge_ts_types::string_mapping_parts(ty).is_some()
}

/// tsc's `isUniformUnionType`: a union of primitives no two literals of which
/// are comparable — no members of two different enums, and no enum member
/// beside a string or number literal.
fn is_uniform_union(ty: &Type) -> bool {
    let Some(Type::Union(union)) = as_union(ty) else {
        return false;
    };
    let mut enum_owner: Option<&str> = None;
    let mut has_string_or_number_literal = false;
    for member in union.types() {
        match member {
            Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::BooleanLiteral(_) => {}
            Type::StringLiteral(_) | Type::NumberLiteral(_) => {
                if enum_owner.is_some() {
                    return false;
                }
                has_string_or_number_literal = true;
            }
            Type::Reference(reference) if reference.is_unique_symbol() => {}
            Type::Reference(reference) if reference.enum_owner.is_some() => {
                if has_string_or_number_literal {
                    return false;
                }
                let owner = reference.enum_owner.as_deref();
                if enum_owner.is_some() && enum_owner != owner {
                    return false;
                }
                enum_owner = owner;
            }
            _ => return false,
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LiteralKind {
    String,
    StringLiteral,
    Pattern,
    Number,
    NumberLiteral,
    BigInt,
    Other,
}

fn literal_kind(ty: &Type) -> LiteralKind {
    match ty {
        Type::String => LiteralKind::String,
        Type::StringLiteral(_) => LiteralKind::StringLiteral,
        Type::Number => LiteralKind::Number,
        Type::NumberLiteral(_) => LiteralKind::NumberLiteral,
        Type::BigInt => LiteralKind::BigInt,
        Type::Reference(_) if is_pattern_literal(ty) => LiteralKind::Pattern,
        Type::Reference(_) => match enum_member_literal(ty) {
            Some(Type::StringLiteral(_)) => LiteralKind::StringLiteral,
            Some(Type::NumberLiteral(_)) => LiteralKind::NumberLiteral,
            _ => LiteralKind::Other,
        },
        _ => LiteralKind::Other,
    }
}

fn maybe_of_kind(ty: &Type, kinds: &[LiteralKind]) -> bool {
    let members = match as_union(ty) {
        Some(union) => union_members(&union),
        None => vec![ty.clone()],
    };
    members.iter().any(|member| kinds.contains(&literal_kind(member)))
}

fn extract_of_kind(ty: &Type, kinds: &[LiteralKind]) -> Type {
    filter_type(ty, &|member| kinds.contains(&literal_kind(member)))
}

/// tsc's `replacePrimitivesWithLiterals`: `string`, `number` and `bigint`
/// constituents of the narrowed type give way to the literals of the same kind
/// the compared value holds.
fn replace_primitives_with_literals(with_primitives: &Type, with_literals: &Type) -> Type {
    use LiteralKind::*;
    if !maybe_of_kind(with_primitives, &[String, Pattern, Number, BigInt])
        || !maybe_of_kind(with_literals, &[StringLiteral, Pattern, NumberLiteral])
    {
        return with_primitives.clone();
    }
    let members = match as_union(with_primitives) {
        Some(union) => union_members(&union),
        None => vec![with_primitives.clone()],
    };
    let mapped: Vec<Type> = members
        .iter()
        .map(|member| match literal_kind(member) {
            String => extract_of_kind(with_literals, &[String, StringLiteral, Pattern]),
            Pattern if !maybe_of_kind(with_literals, &[String, Pattern]) => {
                extract_of_kind(with_literals, &[StringLiteral])
            }
            Number => extract_of_kind(with_literals, &[Number, NumberLiteral]),
            BigInt => extract_of_kind(with_literals, &[BigInt]),
            _ => member.clone(),
        })
        .collect();
    union_type(mapped)
}

fn non_primitive_object() -> Type {
    Type::Object(
        surge_ts_types::ObjectType::new(Default::default(), None).with_non_primitive_marker(),
    )
}
