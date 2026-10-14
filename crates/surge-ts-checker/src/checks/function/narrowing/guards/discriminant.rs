use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, TypeCopyReason};

use super::{const_member_literal_value, literal_expression_value, narrow_union_by_discriminant};
use crate::symbols::{SymbolInfo, SymbolTable};

/// `x?.p === lit` holds only when `x` is not nullish (a nullish `x` reads
/// `undefined`, which equals no literal), so the true branch drops `x`'s
/// `undefined` even when `p` is no discriminant — tsc's optional-chain
/// containment narrowing. `None` when nothing changes.
pub(crate) fn narrow_optional_chain_base(
    condition: &ParsedExpression,
    subject_ty: &Type,
    literal: &Type,
    keep_matching: bool,
) -> Option<Type> {
    if !optional_chain_excludes_nullish(condition, literal, keep_matching) {
        return None;
    }
    let narrowed = surge_ts_types::remove_nullish(subject_ty);
    (narrowed != *subject_ty && !narrowed.is_unknown()).then_some(narrowed)
}

/// Whether `condition` holds only for a non-nullish optional-chain base.
fn optional_chain_excludes_nullish(condition: &ParsedExpression, literal: &Type, keep_matching: bool) -> bool {
    let ParsedExpression::Binary { left, right, .. } = condition else {
        return false;
    };
    let optional_access = matches!(
        left.as_ref(),
        ParsedExpression::OptionalPropertyAccess { .. }
    ) || matches!(
        right.as_ref(),
        ParsedExpression::OptionalPropertyAccess { .. }
    );
    optional_access && keep_matching && *literal != Type::Undefined
}

fn is_nullish_only(ty: &Type) -> bool {
    match ty {
        Type::Null | Type::Undefined | Type::Void => true,
        Type::Union(union) => union.types().iter().all(is_nullish_only),
        _ => false,
    }
}

/// A discriminant test through an optional chain narrows twice: the union by
/// the discriminant, and the base by the chain's non-nullishness. Both apply to
/// `x?.kind === "a"` on `A | B | null`, which leaves `A`.
/// flow.go `getDiscriminantPropertyAccess`: only a reference whose declared
/// or current type is a union is narrowed by one of its properties.
pub(crate) fn narrows_by_discriminant(current: &Type, declared: Option<&Type>) -> bool {
    let is_union = |ty: &Type| matches!(ty.peeled(), Type::Union(_));
    is_union(current) || declared.is_some_and(is_union)
}

pub(crate) fn narrow_discriminant_through_optional_chain(
    condition: &ParsedExpression,
    subject_ty: &Type,
    property: &str,
    literal: &Type,
    keep_matching: bool,
) -> Option<Type> {
    match narrow_union_by_discriminant(subject_ty, property, literal, keep_matching) {
        Some(narrowed) => Some(
            narrow_optional_chain_base(condition, &narrowed, literal, keep_matching)
                .unwrap_or(narrowed),
        ),
        None => narrow_optional_chain_base(condition, subject_ty, literal, keep_matching),
    }
}

/// `base.prop?.kind === "a"` narrows the reference `base.prop` by the
/// discriminant and by the chain's non-nullishness
/// (`narrow_discriminant_through_optional_chain`). tsc narrows the reference
/// itself; surge rewrites the property on the base's type instead, and on a
/// union base (`VariableDeclarator` is `LetOrConstOrVarDeclarator |
/// UsingDeclarator`) in every member, so a read of the reference sees the
/// same type.
///
/// tsc narrows a reference by a discriminant only when its declared type is a
/// union (`isMatchingReferenceDiscriminant`): `p.generated.flag !== true` on a
/// `generated: { flag: true }` leaves `p.generated` as it is, while a member
/// left by earlier tests of a declared union still narrows to `never`.
pub(crate) fn narrow_base_property_by_discriminant(
    base_ty: &Type,
    declared_base: &Type,
    base_property: &str,
    condition: &ParsedExpression,
    property: &str,
    literal: &Type,
    keep_matching: bool,
) -> Option<Type> {
    let declared_property_is_union = match declared_base.peeled() {
        Type::Object(object) => object.properties.get(base_property).is_none_or(|declared| {
            matches!(surge_ts_types::remove_nullish(&declared.ty).peeled(), Type::Union(_))
        }),
        _ => true,
    };
    let narrow_object = |object_type: &surge_ts_types::ObjectType| {
        let base_property_type = object_type.properties.get(base_property)?;
        // A property that is only nullish cannot pass `x?.k === lit`: in that
        // member the reference is `never`.
        let narrowed_property = match narrow_discriminant_through_optional_chain(
            condition,
            &base_property_type.ty,
            property,
            literal,
            keep_matching,
        ) {
            Some(narrowed) => narrowed,
            None if is_nullish_only(&base_property_type.ty)
                && optional_chain_excludes_nullish(condition, literal, keep_matching) =>
            {
                Type::Never
            }
            None => return None,
        };
        if matches!(narrowed_property, Type::Never)
            && !declared_property_is_union
            && !is_nullish_only(&base_property_type.ty)
        {
            return None;
        }
        let mut new_object = object_type.clone();
        let properties = std::sync::Arc::make_mut(&mut new_object.properties);
        properties.insert(
            base_property.into(),
            surge_ts_types::ObjectProperty {
                ty: narrowed_property,
                optional: base_property_type.optional,
                method: base_property_type.method,
                readonly: base_property_type.readonly,
                restriction: base_property_type.restriction.clone(),
                index_slot: base_property_type.index_slot,
            },
        );
        Some(new_object)
    };
    match base_ty.peeled() {
        Type::Object(object_type) => narrow_object(&object_type).map(Type::Object),
        Type::Union(union) => {
            let mut changed = false;
            let members: Vec<Type> = union
                .types()
                .iter()
                .map(|member| match member.peeled() {
                    Type::Object(object_type) => match narrow_object(&object_type) {
                        Some(narrowed) => {
                            changed = true;
                            Type::Object(narrowed)
                        }
                        None => member.clone(),
                    },
                    _ => member.clone(),
                })
                .collect();
            changed.then(|| surge_ts_types::union_type(members))
        }
        _ => None,
    }
}

/// `parse_discriminant_condition` with an extra resolver for operands that are
/// not literal tokens but still denote a unit literal type.
pub(crate) fn parse_discriminant_condition_with<'a>(
    condition: &'a ParsedExpression,
    resolve_literal: &dyn Fn(&ParsedExpression) -> Option<Type>,
) -> Option<(&'a ParsedExpression, &'a str, Type, bool)> {
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

    fn discriminant_side<'a>(
        access: &'a ParsedExpression,
        value: &ParsedExpression,
        eq: bool,
        resolve_literal: &dyn Fn(&ParsedExpression) -> Option<Type>,
    ) -> Option<(&'a ParsedExpression, &'a str, Type, bool)> {
        let literal = literal_expression_value(value).or_else(|| resolve_literal(value))?;
        // `x?.kind === "a"` discriminates like `x.kind === "a"`; a nullish `x`
        // reads `undefined`, which never equals a literal, so the member
        // filter drops it exactly as it drops any member without the property.
        if let ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } = access
        {
            return Some((
                without_non_null_assertions(object),
                property_name.as_str(),
                literal,
                eq,
            ));
        }
        None
    }

    // tsc's `isMatchingReference` looks through `!`: `w.thing!.kind === "a"`
    // discriminates `w.thing` exactly as `w.thing.kind === "a"` does.
    fn without_non_null_assertions(expression: &ParsedExpression) -> &ParsedExpression {
        match expression {
            ParsedExpression::NonNullAssertion { expression, .. } => {
                without_non_null_assertions(expression)
            }
            other => other,
        }
    }

    discriminant_side(left, right, eq, resolve_literal)
        .or_else(|| discriminant_side(right, left, eq, resolve_literal))
}

/// Builds a symbol table with the discriminated union narrowed for the given
/// branch, or `None` if the condition is not a recognized discriminant test.
/// Handles a base that is a plain identifier (`x.kind === …`) or a single
/// property of one (`obj.id.kind === …` narrows `obj`'s `id` property).
pub(crate) fn narrow_discriminant_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let (discriminant_object, property, literal, eq) =
        parse_discriminant_condition_with(condition, &|expression| {
            const_member_literal_value(expression, symbols)
        })?;
    let keep_matching = branch_is_true == eq;

    if let Some((base, path)) = super::super::reference_path(discriminant_object)
        && path.len() > 1
    {
        let symbol = symbols.get(&base)?;
        let guard = super::super::ReferenceGuard::Discriminant {
            property,
            literal: &literal,
            keep_matching,
        };
        if !super::super::reference::discriminates_declared_reference(
            guard,
            symbols.declared_type(&base).unwrap_or(&symbol.ty),
            &path,
        ) {
            return None;
        }
        let narrowed = super::super::narrowed_reference_type(&symbol.ty, &path, guard)?;
        let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
        narrowed_symbols.insert_narrowed(
            base,
            SymbolInfo {
                ty: narrowed,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            },
            symbol.ty.clone(),
        );
        return Some(narrowed_symbols);
    }

    match discriminant_object {
        ParsedExpression::Identifier { name, .. } => {
            let symbol = symbols.get(name)?;
            if !narrows_by_discriminant(&symbol.ty, symbols.declared_type(name)) {
                return None;
            }
            let narrowed = narrow_discriminant_through_optional_chain(
                condition,
                &symbol.ty,
                property,
                &literal,
                keep_matching,
            )?;
            let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
            narrowed_symbols.insert_narrowed(
                name.clone(),
                SymbolInfo {
                    ty: narrowed,
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
                symbol.ty.clone(),
            );
            Some(narrowed_symbols)
        }
        ParsedExpression::PropertyAccess {
            object,
            property_name: base_property,
            ..
        } => {
            // `this.state.kind === "a"` narrows `this.state` as `p.state.kind`
            // narrows `p.state`: `this` is bound like any other name.
            let this_name = "this".to_string();
            let name = match object.as_ref() {
                ParsedExpression::Identifier { name, .. } => name,
                ParsedExpression::This { .. } => &this_name,
                _ => return None,
            };
            let symbol = symbols.get(name)?;
            // `draft` may be typed by a named declaration (nominal reference);
            // the helper peels it to narrow its property (`draft.identity`).
            let narrowed = narrow_base_property_by_discriminant(
                &symbol.ty,
                symbols.declared_type(name).unwrap_or(&symbol.ty),
                base_property,
                condition,
                property,
                &literal,
                keep_matching,
            )?;
            let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
            narrowed_symbols.insert_narrowed(
                name.clone(),
                SymbolInfo {
                    ty: narrowed,
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
                symbol.ty.clone(),
            );
            Some(narrowed_symbols)
        }
        _ => None,
    }
}
