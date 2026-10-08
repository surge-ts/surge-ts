use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, union_type};

use crate::symbols::SymbolTable;

/// Whether a union member's discriminant `property` is the given literal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscriminantMatch {
    Yes,
    No,
    Unknown,
}

pub(crate) fn literal_expression_value(expression: &ParsedExpression) -> Option<Type> {
    match expression {
        ParsedExpression::StringLiteral(value) => Some(Type::StringLiteral(value.clone())),
        ParsedExpression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
        ParsedExpression::NumberLiteral(value) => {
            Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType {
                value: value.clone(),
            }))
        }
        _ => None,
    }
}

/// A case/comparison operand written as a member of a const-enum-like object
/// (`ZodIssueCode.invalid_string`, `Codes.a`) still denotes a unit literal, so
/// it must discriminate exactly like the literal spelled inline. Reads the
/// member type out of scope instead of inferring the expression, which keeps
/// this free of diagnostics and of re-entrant inference.
pub(crate) fn const_member_literal_value(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<Type> {
    // A bare `const CACHE_VERSION = 1` keeps its literal type, so it
    // discriminates like the member form.
    if let ParsedExpression::Identifier { name, .. } = expression {
        let symbol = symbols.get(name)?;
        // A declared `unique symbol` is its own unit type; the equality test
        // removes exactly that member.
        if matches!(&symbol.ty, Type::Reference(reference) if reference.is_unique_symbol()) {
            return Some(symbol.ty.clone());
        }
        let ty = symbol.ty.peeled();
        // A `const` of type `symbol` is a `unique symbol` to tsc — a unit type
        // an equality test can remove (`fn !== skipToken`). surge types it as
        // `symbol`, so the whole `symbol` member stands in for it.
        let unique_symbol =
            ty == Type::Symbol && matches!(symbol.kind, crate::symbols::SymbolKind::Const);
        return (unique_symbol
            || matches!(
                ty,
                Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_)
            ))
        .then_some(ty);
    }
    let ParsedExpression::PropertyAccess {
        object,
        property_name,
        ..
    } = expression
    else {
        return None;
    };
    // A namespace-qualified member (`TSESTree.AST_NODE_TYPES.Identifier`) reads
    // through the namespace object the same way.
    fn qualifier_type(expression: &ParsedExpression, symbols: &SymbolTable) -> Option<Type> {
        match expression {
            ParsedExpression::Identifier { name, .. } => Some(symbols.get(name)?.ty.peeled()),
            ParsedExpression::PropertyAccess { object, property_name, is_bracketed: false, .. } => {
                let Type::Object(object_type) = qualifier_type(object, symbols)? else {
                    return None;
                };
                Some(object_type.properties.get(property_name.as_str())?.ty.peeled())
            }
            _ => None,
        }
    }
    let symbol_ty = qualifier_type(object, symbols)?;
    let Type::Object(object_type) = &symbol_ty else {
        return None;
    };
    let property = object_type.properties.get(property_name.as_str())?;
    match &property.ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => {
            Some(property.ty.clone())
        }
        _ => None,
    }
}

pub(super) fn discriminant_match(member: &Type, property: &str, literal: &Type) -> DiscriminantMatch {
    // A discriminated-union member is often a named type (nominal reference);
    // peel it to read its discriminant property.
    let member = member.peeled();
    let Type::Object(object) = &member else {
        return DiscriminantMatch::Unknown;
    };
    let Some(property_type) = object.properties.get(property) else {
        // A member without the discriminant property cannot equal the literal.
        return DiscriminantMatch::No;
    };
    // The discriminant is often written as `typeof Codes.a`, which resolves to a
    // lazy reference around the literal rather than the literal itself.
    match discriminant_type_match(&property_type.ty, literal) {
        // `tag?: "a"` may be absent, so the member also survives `tag !== "a"`.
        DiscriminantMatch::Yes if property_type.optional => DiscriminantMatch::Unknown,
        matched => matched,
    }
}

/// Whether a discriminant of type `property_ty` can hold `literal`.
pub(crate) fn discriminant_type_match(property_ty: &Type, literal: &Type) -> DiscriminantMatch {
    // Any of several literals (`k === A || k === B`): the discriminant can hold
    // one of them unless it can hold none.
    if let Type::Union(literals) = literal {
        let matches: Vec<DiscriminantMatch> = literals
            .types()
            .iter()
            .map(|literal| discriminant_type_match(property_ty, literal))
            .collect();
        return if matches.iter().all(|result| *result == DiscriminantMatch::No) {
            DiscriminantMatch::No
        } else if matches.iter().all(|result| *result == DiscriminantMatch::Yes) {
            DiscriminantMatch::Yes
        } else {
            DiscriminantMatch::Unknown
        };
    }
    if let Some(matched) = template_discriminant_match(property_ty, literal) {
        return matched;
    }
    let property_ty = property_ty.peeled();
    match &property_ty {
        Type::Union(union) => {
            let matches: Vec<DiscriminantMatch> = union
                .types()
                .iter()
                .map(|ty| template_discriminant_match(ty, literal).unwrap_or_else(|| constituent_match(&ty.peeled(), literal)))
                .collect();
            if matches.iter().all(|result| *result == DiscriminantMatch::No) {
                DiscriminantMatch::No
            } else {
                // The literal is one of several possibilities; keep the member in
                // both branches conservatively.
                DiscriminantMatch::Unknown
            }
        }
        _ => constituent_match(&property_ty, literal),
    }
}

/// A template literal discriminant (`` `${string}_SUCCESS` ``), which a full
/// peel erases to `string`: a string literal the pattern does not match is
/// not comparable to it, and one it matches leaves the member in both
/// branches, the pattern being no unit type.
fn template_discriminant_match(property_ty: &Type, literal: &Type) -> Option<DiscriminantMatch> {
    let Type::StringLiteral(_) = literal else {
        return None;
    };
    let pattern = surge_ts_types::peel_to_pattern_literal(property_ty);
    let (texts, types) = surge_ts_types::template_literal_parts(&pattern)?;
    Some(if surge_ts_types::is_type_matched_by_template_literal(literal, &texts, &types) {
        DiscriminantMatch::Unknown
    } else {
        DiscriminantMatch::No
    })
}

/// Whether one non-union discriminant type can hold `literal`. tsc keeps a
/// member only when its property type is comparable to the compared value, so a
/// primitive of another domain (`kind: string` against `=== false`) rules the
/// member out just as a different literal does.
fn constituent_match(property_ty: &Type, literal: &Type) -> DiscriminantMatch {
    if matches!(
        property_ty,
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_)
    ) {
        return if property_ty == literal {
            DiscriminantMatch::Yes
        } else {
            DiscriminantMatch::No
        };
    }
    let same_domain = match literal {
        Type::StringLiteral(_) => matches!(property_ty, Type::String),
        Type::NumberLiteral(_) => matches!(property_ty, Type::Number),
        Type::BooleanLiteral(_) => matches!(property_ty, Type::Boolean),
        _ => return DiscriminantMatch::Unknown,
    };
    let is_primitive = matches!(
        property_ty,
        Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::Void
    );
    if is_primitive && !same_domain {
        DiscriminantMatch::No
    } else {
        DiscriminantMatch::Unknown
    }
}

/// Narrows a discriminated union by `property` against `literal`. `keep_matching`
/// selects the members that can equal the literal (the `===` true branch); its
/// negation keeps the rest. Returns `None` when the type is not a union or the
/// condition does not partition it.
pub(crate) fn narrow_union_by_discriminant(
    ty: &Type,
    property: &str,
    literal: &Type,
    keep_matching: bool,
) -> Option<Type> {
    // An alias reached through a lazy reference is the same union: whether it
    // was expanded already depends on the scope the annotation resolved in (a
    // generic class body leaves it deferred), and narrowing must not.
    let peeled = matches!(ty, Type::Reference(_)).then(|| ty.peeled());
    let ty = peeled.as_ref().unwrap_or(ty);
    let Type::Union(union) = ty else {
        // The last member left by earlier tests: excluding the one literal its
        // discriminant can be leaves nothing, which is what makes
        // `const unreachable: never = shape` after the final `case` type-check.
        return (!keep_matching && discriminant_is_exactly(ty, property, literal))
            .then_some(Type::Never);
    };
    // A member that is itself an alias of a union (`TRPCConnectionState<E>`
    // beside the result shapes) is that union's members, as `getUnionType`
    // flattens it; left whole, its discriminant reads as undecided and the
    // whole alias survives every test.
    let flattened;
    let union = if union.types().iter().any(|member| matches!(member, Type::Reference(_)) && matches!(member.peeled(), Type::Union(_))) {
        let members: Vec<Type> = union
            .types()
            .iter()
            .flat_map(|member| match (member, member.peeled()) {
                (Type::Reference(_), Type::Union(inner)) => inner.types().to_vec(),
                _ => vec![member.clone()],
            })
            .collect();
        match union_type(members) {
            Type::Union(flat) => {
                flattened = flat;
                &flattened
            }
            _ => union,
        }
    } else {
        union
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| {
            let result = discriminant_match(member, property, literal);
            if keep_matching {
                result != DiscriminantMatch::No
            } else {
                result != DiscriminantMatch::Yes
            }
        })
        .cloned()
        .collect();

    if kept.is_empty() {
        let exhausted = !keep_matching
            && union
                .types()
                .iter()
                .all(|member| discriminant_is_exactly(member, property, literal));
        return exhausted.then_some(Type::Never);
    }
    if kept.len() == union.types().len() {
        return None;
    }
    Some(union_type(kept))
}

/// Whether `member` is an object whose *required* `property` is exactly
/// `literal`, so that `member.property !== literal` cannot hold. An optional
/// discriminant may be absent, and anything surge could not read as an object
/// proves nothing — both keep the member.
fn discriminant_is_exactly(member: &Type, property: &str, literal: &Type) -> bool {
    let Type::Object(object) = &member.peeled() else {
        return false;
    };
    object
        .properties
        .get(property)
        .is_some_and(|declared| !declared.optional && declared.ty.peeled() == *literal)
}

/// Parses `value === "lit"` / `value !== 3` on a bare identifier — the
/// whole-value form of an equality test, as opposed to the discriminant form
/// (`value.kind === "lit"`) [`parse_discriminant_condition_with`] handles.
/// Returns the identifier, the literal type, and whether the operator is an
/// equality test.
pub(crate) fn parse_identifier_literal_equality<'a>(
    condition: &'a ParsedExpression,
    symbols: &SymbolTable,
) -> Option<(&'a str, Type, bool)> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        left,
        operator,
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

    // The compared value may be a `const` holding a unit type
    // (`fn !== skipToken`, `status === DONE`).
    let literal_of = |expression: &ParsedExpression| {
        literal_expression_value(expression)
            .or_else(|| const_member_literal_value(expression, symbols))
    };
    if let ParsedExpression::Identifier { name, .. } = left.as_ref()
        && let Some(literal) = literal_of(right)
    {
        return Some((name.as_str(), literal, eq));
    }
    if let ParsedExpression::Identifier { name, .. } = right.as_ref()
        && let Some(literal) = literal_of(left)
    {
        return Some((name.as_str(), literal, eq));
    }
    None
}

/// Parses `o.p === "lit"` / `o?.p !== 3` — the literal-equality test on a
/// property reference, which [`parse_identifier_literal_equality`] leaves alone.
/// Returns the reference, the literal's expression (the guard re-reads it, so the
/// guard stays a borrow of the condition), and whether the operator is an
/// equality test. The discriminant narrowers read the same shape to filter the *base*
/// union; this feeds the reference guard that narrows the property itself, so
/// `if (filters?.refetchType === 'none') return` leaves `filters?.refetchType`
/// without `'none'` afterwards.
pub(crate) fn parse_reference_literal_equality(
    condition: &'_ ParsedExpression,
) -> Option<(&'_ ParsedExpression, &'_ ParsedExpression, bool)> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        left,
        operator,
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
    let is_reference = |expression: &ParsedExpression| {
        matches!(
            expression,
            ParsedExpression::PropertyAccess { .. } | ParsedExpression::OptionalPropertyAccess { .. }
        )
    };
    if is_reference(left) && literal_expression_value(right).is_some() {
        return Some((left.as_ref(), right.as_ref(), eq));
    }
    if is_reference(right) && literal_expression_value(left).is_some() {
        return Some((right.as_ref(), left.as_ref(), eq));
    }
    None
}

/// The wide primitive a unit literal inhabits, so `raw === "query"` can narrow a
/// plainly `string`-typed binding down to the literal the way tsc does.
pub(super) fn literal_base_primitive(literal: &Type) -> Option<Type> {
    match literal {
        Type::StringLiteral(_) => Some(Type::String),
        Type::NumberLiteral(_) => Some(Type::Number),
        Type::BooleanLiteral(_) => Some(Type::Boolean),
        // The `unique symbol` stand-in above, and a declared one.
        Type::Symbol => Some(Type::Symbol),
        Type::Reference(reference) if reference.is_unique_symbol() => Some(Type::Symbol),
        _ => None,
    }
}

/// Whether narrowing a subject by a literal equality test says anything at all.
/// A degraded or `any` subject carries no members to filter, and narrowing it to
/// the literal would invent a type the value never had.
pub(super) fn literal_equality_applies(ty: &Type) -> bool {
    !ty.is_unknown() && !matches!(ty, Type::Any | Type::GenuineUnknown | Type::Never)
}

/// Narrows `ty` by `=== <literal>` (`keep_matching`) or `!== <literal>`.
///
/// The matching branch keeps the union members the literal can inhabit, and
/// replaces a member the literal is strictly narrower than — `string` for
/// `"query"` — with the literal itself. The complement drops only the members
/// that *are* the literal; a wider member survives, since a `string` that is not
/// `"query"` is still a `string`. `boolean` is the exception: tsc's is the union
/// `true | false`, so what is not one of its literals is the other.
pub(crate) fn narrow_by_literal_equality(
    ty: &Type,
    literal: &Type,
    keep_matching: bool,
) -> Option<Type> {
    // `narrowTypeByEquality`: an `unknown` that equals a primitive value is
    // that value's type.
    if matches!(ty, Type::GenuineUnknown) && keep_matching {
        return Some(literal.clone());
    }
    if !literal_equality_applies(ty) {
        return None;
    }
    let base = literal_base_primitive(literal)?;

    let Type::Union(union) = ty else {
        if *ty != base {
            return None;
        }
        return Some(if keep_matching {
            literal.clone()
        } else {
            other_boolean_literal(literal)?
        });
    };

    let members = &super::equality::evaluated_union_members(union.types())[..];
    if keep_matching {
        let mut kept: Vec<Type> = Vec::new();
        for member in members {
            let narrowed = if *member == *literal {
                literal.clone()
            } else if *member == base {
                literal.clone()
            } else {
                continue;
            };
            if !kept.contains(&narrowed) {
                kept.push(narrowed);
            }
        }
        if kept.is_empty() {
            return None;
        }
        let narrowed = union_type(kept);
        (narrowed != *ty).then_some(narrowed)
    } else {
        let kept: Vec<Type> = members
            .iter()
            .filter(|member| **member != *literal)
            .map(|member| match member {
                Type::Boolean => other_boolean_literal(literal).unwrap_or_else(|| member.clone()),
                _ => member.clone(),
            })
            .collect();
        if kept.is_empty() || kept.as_slice() == union.types() {
            return None;
        }
        Some(union_type(kept))
    }
}

/// The `boolean` member left when `literal` is removed from it.
fn other_boolean_literal(literal: &Type) -> Option<Type> {
    match literal {
        Type::BooleanLiteral(value) => Some(Type::BooleanLiteral(!*value)),
        _ => None,
    }
}
