//! Truthiness-guard narrowing of identifiers and properties within function bodies.

use std::sync::Arc;
use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator};
use surge_ts_types::{Type, TypeCopyReason, union_type};

use crate::context::CheckerContext;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};

mod aliases;
mod element_reference;
mod guards;
mod predicate;
mod reference;
mod truthy;
mod type_guards;

pub(crate) use aliases::{ALIAS_INLINE_LIMIT, retain_constant_reference_guards};
use aliases::enter_alias_inlining;
pub(crate) use element_reference::*;
use guards::*;
pub(crate) use predicate::*;
pub(crate) use guards::typeof_tags_of;
pub(crate) use reference::*;
pub(crate) use truthy::*;
pub(crate) use type_guards::*;

/// Narrows `ty` for variable `var_name` under `condition`, returning the narrowed
/// type or `None` when the condition does not constrain `var_name` (or leaves it
/// unchanged). Composes `||` (true branch: union of disjuncts — every disjunct
/// must constrain `var_name`), `&&` (true branch: sequential), and `!`.
///
/// `operand_types` answers the root types of the operands the condition
/// compares, as they were before any binding it tests narrowed.
fn narrow_type_for_identifier(
    condition: &ParsedExpression,
    var_name: &str,
    ty: &Type,
    branch_is_true: bool,
    scopes: &ScopeStack,
    operand_types: &dyn Fn(&str) -> Option<Type>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => narrow_type_for_identifier(
            operand,
            var_name,
            ty,
            !branch_is_true,
            scopes,
            operand_types,
            ctx,
        ),
        _ if strip_boolean_literal_comparison(condition).is_some() => {
            let (inner, flip) = strip_boolean_literal_comparison(condition)
                .expect("boolean-literal comparison checked above");
            narrow_type_for_identifier(
                inner,
                var_name,
                ty,
                branch_is_true != flip,
                scopes,
                operand_types,
                ctx,
            )
        }
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } if branch_is_true => {
            // `A || B` true branch: A held, or A failed and then B held — the
            // union of what the two edges into the branch prove. A disjunct that
            // does not constrain `var_name` leaves it unconstrained, so the whole
            // guard cannot narrow — bail.
            let left_narrowed =
                narrow_type_for_identifier(left, var_name, ty, true, scopes, operand_types, ctx)?;
            let after_left =
                narrow_type_for_identifier(left, var_name, ty, false, scopes, operand_types, ctx)
                    .unwrap_or_else(|| ty.clone());
            let right_narrowed = narrow_type_for_identifier(
                right,
                var_name,
                &after_left,
                true,
                scopes,
                operand_types,
                ctx,
            )?;
            Some(union_type(vec![left_narrowed, right_narrowed]))
        }
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And,
            right,
            ..
        } if branch_is_true => {
            // `A && B` true branch: apply each guard in sequence.
            let after_left =
                narrow_type_for_identifier(left, var_name, ty, true, scopes, operand_types, ctx)
                    .unwrap_or_else(|| ty.clone());
            Some(
                narrow_type_for_identifier(
                    right,
                    var_name,
                    &after_left,
                    true,
                    scopes,
                    operand_types,
                    ctx,
                )
                .unwrap_or(after_left),
            )
        }
        // Fall-through of `A || B` is `!A && !B`: apply each disjunct's negation
        // in sequence. Treating the whole guard as one atom here narrowed to the
        // disjunct's *true* shape, so `if (r.kind === "x" || other) continue;`
        // left `r` as the `"x"` member instead of removing it.
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } => {
            let after_left =
                narrow_type_for_identifier(left, var_name, ty, false, scopes, operand_types, ctx)
                    .unwrap_or_else(|| ty.clone());
            Some(
                narrow_type_for_identifier(
                    right,
                    var_name,
                    &after_left,
                    false,
                    scopes,
                    operand_types,
                    ctx,
                )
                .unwrap_or(after_left),
            )
        }
        // Fall-through of `A && B`: A failed, or A held and then B failed — a
        // union, and only sound when both operands constrain the value.
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And,
            right,
            ..
        } => {
            let left_narrowed =
                narrow_type_for_identifier(left, var_name, ty, false, scopes, operand_types, ctx)?;
            let after_left =
                narrow_type_for_identifier(left, var_name, ty, true, scopes, operand_types, ctx)
                    .unwrap_or_else(|| ty.clone());
            let right_narrowed = narrow_type_for_identifier(
                right,
                var_name,
                &after_left,
                false,
                scopes,
                operand_types,
                ctx,
            )?;
            Some(union_type(vec![left_narrowed, right_narrowed]))
        }
        _ => narrow_single_guard_for_identifier(
            condition,
            var_name,
            ty,
            branch_is_true,
            scopes,
            operand_types,
            ctx,
        )
        .or_else(|| narrow_by_property_truthiness(condition, var_name, ty, branch_is_true)),
    }
}

/// `narrowTypeByTruthiness` through a discriminant property of `var_name`
/// (`value.type` in `value.type && …`): the members whose property the test
/// rules out are dropped, down to `never`.
fn narrow_by_property_truthiness(
    condition: &ParsedExpression,
    var_name: &str,
    ty: &Type,
    branch_is_true: bool,
) -> Option<Type> {
    let (base, path) = reference_path(condition)?;
    if base != var_name || path.is_empty() {
        return None;
    }
    let peeled = ty.peeled();
    let members: &[Type] = match &peeled {
        Type::Union(union) => union.types(),
        other => std::slice::from_ref(other),
    };
    let kept: Vec<Type> = members
        .iter()
        .filter(|member| truthy::path_truthiness(member, &path) != Some(!branch_is_true))
        .cloned()
        .collect();
    (kept.len() != members.len()).then(|| union_type(kept))
}

fn narrow_single_guard_for_identifier(
    condition: &ParsedExpression,
    var_name: &str,
    ty: &Type,
    branch_is_true: bool,
    scopes: &ScopeStack,
    operand_types: &dyn Fn(&str) -> Option<Type>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if let Some((ParsedExpression::Identifier { name, .. }, ctor_name)) =
        parse_instanceof_condition(condition).map(|(operand, ctor)| (operand, ctor))
        && name == var_name
    {
        let constructor_value = instanceof_constructor_value(condition, scopes.visible_symbols(), ctx);
        if constructs_only_any(constructor_value.as_ref()) {
            return None;
        }
        let instance =
            resolve_constructor_instance_type(ctor_name, constructor_value.as_ref(), ctx);
        if let Some(narrowed) =
            narrow_union_by_instanceof(ty, ctor_name, instance.as_ref(), branch_is_true)
        {
            return Some(narrowed);
        }
        return narrow_to_instanceof_subclass(ty, instance.as_ref(), branch_is_true);
    }
    if let Some((ParsedExpression::Identifier { name, .. }, tag, eq)) =
        parse_typeof_condition(condition)
        && name == var_name
    {
        return narrow_union_by_typeof(ty, tag, branch_is_true == eq);
    }
    if let Some(ParsedExpression::Identifier { name, .. }) =
        parse_array_isarray_condition(condition)
        && name == var_name
    {
        return narrow_union_by_arrayness(ty, branch_is_true);
    }
    if let Some(ParsedExpression::Identifier { name, .. }) =
        parse_arraybuffer_isview_condition(condition)
        && name == var_name
    {
        return narrow_union_by_arraybufferview(ty, branch_is_true);
    }
    if let Some(guard) = parse_type_predicate_condition(condition, &mut |callee| {
        predicate_callee_signature(
            callee,
            |name| scopes.resolve(name),
            scopes.visible_symbols(),
            ctx,
        )
    }) && guard.subject == var_name
        && guard.path.is_empty()
    {
        return match resolve_predicate_guard_target(
            &guard,
            Some(ty),
            scopes.visible_symbols(),
            ctx,
        )? {
            PredicateTarget::Resolved(predicate_ty) => {
                narrow_by_predicate(ty, &predicate_ty, branch_is_true)
            }
            PredicateTarget::Degraded => degraded_predicate_subject(branch_is_true),
        };
    }
    if let Some((subject, path, method)) = parse_this_predicate_call(condition)
        && path.is_empty()
        && subject == var_name
        && let Some(target) = this_predicate_target(ty, method, ctx)
    {
        return narrow_by_predicate(ty, &target, branch_is_true);
    }
    if let Some((ParsedExpression::Identifier { name, .. }, property, literal, eq)) =
        parse_discriminant_condition_with(condition, &|expression| {
            const_member_literal_value(expression, scopes.visible_symbols())
        })
        && name == var_name
    {
        // The reference's declared type is its symbol's type (`getTypeOfSymbol`),
        // which a binding with no recorded declaration still holds in scope;
        // `ty` may already be narrowed by an earlier operand of the condition.
        let declared = scopes
            .visible_symbols()
            .declared_type(name)
            .cloned()
            .or_else(|| scopes.resolve(name).map(|symbol| symbol.ty.clone()));
        if !guards::narrows_by_discriminant(ty, declared.as_ref()) {
            return None;
        }
        let keep_matching = branch_is_true == eq;
        return narrow_discriminant_through_optional_chain(
            condition,
            ty,
            property,
            &literal,
            keep_matching,
        );
    }
    if let Some((ParsedExpression::Identifier { name, .. }, eq, test)) =
        parse_nullish_equality_condition(condition)
        && name == var_name
    {
        return narrow_binding_by_nullish(ty, branch_is_true == eq, test);
    }
    if let Some((name, literal, eq)) =
        parse_identifier_literal_equality(condition, scopes.visible_symbols())
        && name == var_name
    {
        let keep_matching = branch_is_true == eq;
        // Excluding the one literal the value is already narrowed to leaves
        // `never`, as `narrow_literal_equality_in_scope` does for a lone `if`:
        // it is what the `default` of an exhausted `switch (k)` sees.
        if let Some(narrowed) = narrow_by_literal_equality(ty, &literal, keep_matching)
            .or_else(|| (!keep_matching && *ty == literal).then_some(Type::Never))
        {
            return Some(narrowed);
        }
    }
    if let Some((ParsedExpression::Identifier { name, .. }, property)) =
        parse_in_condition(condition)
        && name == var_name
    {
        return narrow_union_by_property_presence(ty, property, branch_is_true);
    }
    if let Some(narrowed) =
        narrow_binding_by_reference_equality(condition, var_name, ty, branch_is_true, operand_types)
    {
        return Some(narrowed);
    }
    narrow_by_property_guard(condition, var_name, ty, branch_is_true, &|expression| {
        const_member_literal_value(expression, scopes.visible_symbols())
    })
}

/// tsc's `narrowTypeByDiscriminant` for a guard on a property below `var_name`:
/// a truthiness test (`if (r.kind)`), a nullish comparison
/// (`r.kind === undefined`) or a literal comparison (`env.result.kind === "a"`).
/// The union the property is read from keeps the members whose own property
/// can pass the test.
fn narrow_by_property_guard(
    condition: &ParsedExpression,
    var_name: &str,
    ty: &Type,
    branch_is_true: bool,
    resolve_literal: &dyn Fn(&ParsedExpression) -> Option<Type>,
) -> Option<Type> {
    if let Some((reference, eq, _)) = parse_nullish_equality_condition(condition)
        && let Some(path) = property_path_below(reference, var_name)
    {
        let keep_nullish = branch_is_true == eq;
        return narrow_union_at_path(ty, &path, &|member, rest| {
            truthy::path_nullishness(member, rest) != Some(!keep_nullish)
        });
    }
    if let Some((object, property, literal, eq)) =
        parse_discriminant_condition_with(condition, resolve_literal)
        && let Some(mut path) = property_path_below(object, var_name)
    {
        path.push(property.to_string());
        let keep_matching = branch_is_true == eq;
        return narrow_union_at_path(ty, &path, &|member, rest| {
            match truthy::path_leaf_type(member, rest) {
                Some(leaf) if keep_matching => may_equal_literal(&leaf, &literal),
                Some(leaf) => leaf != literal,
                None => true,
            }
        });
    }
    let path = property_path_below(condition, var_name)?;
    narrow_union_at_path(ty, &path, &|member, rest| {
        truthy::path_truthiness(member, rest) != Some(!branch_is_true)
    })
}

/// [`narrow_by_property_guard`] over a symbol table, for the binding the
/// guarded property hangs off.
fn narrow_property_guard_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let reference = parse_nullish_equality_condition(condition)
        .map(|(reference, ..)| reference)
        .or_else(|| {
            parse_discriminant_condition_with(condition, &|_| None).map(|(object, ..)| object)
        })
        .unwrap_or(condition);
    let (base, path) = reference::reference_path(reference)?;
    if path.is_empty() && !matches!(condition, ParsedExpression::Binary { .. }) {
        return None;
    }
    let symbol = symbols.get(&base)?;
    let narrowed = narrow_by_property_guard(condition, &base, &symbol.ty, branch_is_true, &|expression| {
        const_member_literal_value(expression, symbols)
    })?;
    if narrowed == symbol.ty {
        return None;
    }
    let narrowed_symbol = SymbolInfo {
        ty: narrowed,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let declared = symbol.ty.clone();
    let mut table = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    table.insert_narrowed(base, narrowed_symbol, declared);
    Some(table)
}

/// Whether a value of type `leaf` can be `===` the unit type `literal`.
fn may_equal_literal(leaf: &Type, literal: &Type) -> bool {
    match (leaf.peeled(), literal) {
        (Type::Union(union), _) => union.types().iter().any(|member| may_equal_literal(member, literal)),
        (
            leaf @ (Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Undefined),
            _,
        ) => leaf == *literal,
        (Type::String, Type::StringLiteral(_))
        | (Type::Number, Type::NumberLiteral(_))
        | (Type::Boolean, Type::BooleanLiteral(_)) => true,
        (Type::String | Type::Number | Type::Boolean, _) => false,
        _ => true,
    }
}

/// The property path from `var_name` down to `reference`, when `reference` is
/// a non-empty member access rooted at that name (`r.a.b` below `r` is
/// `["a", "b"]`, below `r.a` it is `["b"]`).
fn property_path_below(reference: &ParsedExpression, var_name: &str) -> Option<Vec<String>> {
    let (base, path) = reference::reference_path(reference)?;
    let mut prefix = base;
    for (index, segment) in path.iter().enumerate() {
        if prefix == var_name {
            return Some(path[index..].to_vec());
        }
        prefix.push('.');
        prefix.push_str(segment);
    }
    None
}

/// Filters the union the discriminant `path` is read from — `ty` itself, or
/// the union some property above the discriminant holds (`env.result` for
/// `env.result.type`) — and rebuilds the objects around it. `keep` sees each
/// member with the path that remains below it.
fn narrow_union_at_path(
    ty: &Type,
    path: &[String],
    keep: &dyn Fn(&Type, &[String]) -> bool,
) -> Option<Type> {
    match ty.peeled() {
        Type::Union(union) => {
            let kept: Vec<Type> =
                union.types().iter().filter(|member| keep(member, path)).cloned().collect();
            (!kept.is_empty()).then(|| union_type(kept))
        }
        Type::Object(mut object) if path.len() > 1 => {
            let (head, rest) = path.split_first()?;
            let existing = object.properties.get(head.as_str())?.clone();
            let narrowed = narrow_union_at_path(&existing.ty, rest, keep)?;
            Arc::make_mut(&mut object.properties).insert(
                head.as_str().into(),
                surge_ts_types::ObjectProperty {
                    ty: narrowed,
                    ..existing
                },
            );
            Some(Type::Object(object))
        }
        _ => None,
    }
}

/// Applies `||`/`&&`-composed guard narrowing in place to a `ScopeStack`. Returns
/// whether `condition` was such a logical composition (so the caller can stop).
fn narrow_logical_guard_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> bool {
    if !matches!(
        condition,
        ParsedExpression::Logical {
            operator: ParsedLogicalOperator::Or | ParsedLogicalOperator::And,
            ..
        }
    ) {
        return false;
    }

    let mut operand_names = Vec::new();
    collect_guard_operand_identifiers(condition, &mut operand_names);
    collect_predicate_guard_subjects(condition, scopes, &mut operand_names, ctx);
    collect_equality_guard_subjects(condition, scopes, &mut operand_names);
    collect_property_guard_bases(condition, &mut operand_names);

    // A compared operand is typed where it is read, before the bindings this
    // loop narrows: `y !== z || z !== y` narrows `z` by what `y` was.
    let mut operand_roots: Vec<(String, Type)> = Vec::new();
    collect_compared_operand_roots(condition, scopes, &mut operand_roots);
    let operand_types = |name: &str| {
        operand_roots
            .iter()
            .find(|(root, _)| root == name)
            .map(|(_, ty)| ty.clone())
    };

    for name in operand_names {
        // An element reference (`xs[0]`) was narrowed by every operand, in
        // order, by `narrow_element_reference_guards_in_scope`; narrowing it
        // again from one operand alone would undo the later ones.
        if name.ends_with(']') {
            continue;
        }
        let Some(symbol) = scopes.resolve(&name) else {
            continue;
        };
        let symbol_ty = symbol.ty.clone();
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let Some(narrowed) = narrow_type_for_identifier(
            condition,
            &name,
            &symbol_ty,
            branch_is_true,
            scopes,
            &operand_types,
            ctx,
        ) else {
            continue;
        };
        if narrowed == symbol_ty {
            continue;
        }
        let narrowed_symbol = SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        };
        let _ = scopes.insert_current_narrowed(name, narrowed_symbol, symbol_ty);
    }

    // Every operand of an `&&` holds in its true branch, so the chain also proves
    // each truthy-tested reference (`o.p && o.p.q`) non-nullish — which
    // `narrow_type_for_identifier` above, keyed on a whole binding, cannot express.
    let mut reference_guards = Vec::new();
    collect_reference_guards(
        condition,
        branch_is_true,
        &compared_operand_type(scopes.visible_symbols()),
        &mut reference_guards,
    );
    for (base, path, guard) in reference_guards {
        narrow_reference_in_scope(&base, &path, guard, scopes);
    }
    let mut holding_predicates = Vec::new();
    collect_holding_operands(condition, branch_is_true, &mut holding_predicates);
    for operand in holding_predicates {
        predicate::narrow_predicate_chain_containment_in_scope(operand, scopes, ctx);
    }
    // A member-reference operand (`p.key.type === T.Id`) discriminates the
    // union it is read from; the syntactic literal tests above see only
    // literals spelled inline.
    let mut unit_tests = Vec::new();
    collect_unit_member_discriminants(condition, branch_is_true, scopes.visible_symbols(), &mut unit_tests);
    for (base, path, property, literal, keep_matching) in unit_tests {
        narrow_reference_in_scope(
            &base,
            &path,
            ReferenceGuard::Discriminant { property: &property, literal: &literal, keep_matching },
            scopes,
        );
    }
    true
}

/// The operands of a `&&` chain (or of a failing `||` chain) that hold
/// wherever the whole condition takes `branch_is_true`, with any `!` peeled.
fn collect_holding_operands<'a>(
    condition: &'a ParsedExpression,
    branch_is_true: bool,
    found: &mut Vec<&'a ParsedExpression>,
) {
    match condition {
        ParsedExpression::Logical { left, operator, right, .. }
            if matches!(
                (operator, branch_is_true),
                (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
            ) =>
        {
            collect_holding_operands(left, branch_is_true, found);
            collect_holding_operands(right, branch_is_true, found);
        }
        ParsedExpression::Unary { operator: ParsedUnaryOperator::Not, operand, .. } => {
            collect_holding_operands(operand, !branch_is_true, found);
        }
        ParsedExpression::Logical { .. } => {}
        other if branch_is_true => found.push(other),
        _ => {}
    }
}

/// The `o.p.kind === E.A` operands of a `&&` chain (or the failing operands of
/// an `||` chain) whose compared value is a unit member rather than a literal.
pub(super) fn collect_unit_member_discriminants(
    condition: &ParsedExpression,
    branch_is_true: bool,
    symbols: &SymbolTable,
    found: &mut Vec<(String, Vec<String>, String, Type, bool)>,
) {
    match condition {
        ParsedExpression::Logical { left, operator, right, .. } => {
            if matches!(
                (operator, branch_is_true),
                (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
            ) {
                collect_unit_member_discriminants(left, branch_is_true, symbols, found);
                collect_unit_member_discriminants(right, branch_is_true, symbols, found);
                return;
            }
            // `k.type === A || k.type === B` holds where `k` is either member.
            if *operator == ParsedLogicalOperator::Or && branch_is_true {
                let (mut lefts, mut rights) = (Vec::new(), Vec::new());
                collect_unit_member_discriminants(left, true, symbols, &mut lefts);
                collect_unit_member_discriminants(right, true, symbols, &mut rights);
                if let ([l], [r]) = (lefts.as_slice(), rights.as_slice())
                    && l.4
                    && r.4
                    && l.0 == r.0
                    && l.1 == r.1
                    && l.2 == r.2
                {
                    let literal = union_type(vec![l.3.clone(), r.3.clone()]);
                    found.push((l.0.clone(), l.1.clone(), l.2.clone(), literal, true));
                }
            }
        }
        ParsedExpression::Unary { operator: ParsedUnaryOperator::Not, operand, .. } => {
            collect_unit_member_discriminants(operand, !branch_is_true, symbols, found);
        }
        _ => {
            if guards::parse_reference_literal_equality(condition).is_some() {
                return;
            }
            let Some((object, property, literal, eq)) =
                parse_discriminant_condition_with(condition, &|expression| {
                    const_member_literal_value(expression, symbols)
                })
            else {
                return;
            };
            if let Some((base, path)) = reference_path(object)
                && !path.is_empty()
            {
                found.push((base, path, property.to_string(), literal, branch_is_true == eq));
            }
        }
    }
}

/// The roots of the operands every equality test in a `||`/`&&`/`!`-composed
/// condition compares, with their current types.
fn collect_compared_operand_roots(
    condition: &ParsedExpression,
    scopes: &ScopeStack,
    roots: &mut Vec<(String, Type)>,
) {
    match condition {
        ParsedExpression::Logical { left, right, .. } => {
            collect_compared_operand_roots(left, scopes, roots);
            collect_compared_operand_roots(right, scopes, roots);
        }
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_compared_operand_roots(operand, scopes, roots),
        _ if strip_boolean_literal_comparison(condition).is_some() => {
            let (inner, _) = strip_boolean_literal_comparison(condition)
                .expect("boolean-literal comparison checked above");
            collect_compared_operand_roots(inner, scopes, roots);
        }
        _ => {
            let Some(test) = parse_equality_test(condition) else {
                return;
            };
            for (operand, _) in test.operand_pairs() {
                if let Some((root, _)) = reference_path(operand)
                    && !roots.iter().any(|(existing, _)| *existing == root)
                    && let Some(symbol) = scopes.resolve(&root)
                {
                    let ty = symbol.ty.clone();
                    roots.push((root, ty));
                }
            }
        }
    }
}

/// The bindings whose properties a `||`/`&&`/`!`-composed condition tests for
/// truthiness or nullishness (`env` in `!env.result.type || …`).
fn collect_property_guard_bases(condition: &ParsedExpression, names: &mut Vec<String>) {
    match condition {
        ParsedExpression::Logical { left, right, .. } => {
            collect_property_guard_bases(left, names);
            collect_property_guard_bases(right, names);
        }
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_property_guard_bases(operand, names),
        _ => {
            let reference = parse_nullish_equality_condition(condition)
                .map(|(reference, ..)| reference)
                .unwrap_or(condition);
            if let Some((base, path)) = reference::reference_path(reference)
                && !path.is_empty()
                && base != "this"
                && !names.contains(&base)
            {
                names.push(base);
            }
        }
    }
}

/// Collects the distinct identifiers tested by single guards within a
/// (possibly `||`/`&&`/`!`-composed) condition.
/// The identifiers a condition guards as whole values: the `typeof`/`in`/
/// `instanceof`/`Array.isArray` operands plus any bare-truthy identifier across
/// an `&&` chain. A guard on such an identifier removes its genuine-unknownness
/// in the branch where the condition holds (property bases like `x.p` are
/// excluded — guarding `x.p` does not narrow `x`).
pub(crate) fn guarded_value_identifiers(condition: &ParsedExpression) -> Vec<String> {
    let mut names = Vec::new();
    collect_guard_operand_identifiers(condition, &mut names);

    let mut truthy = Vec::new();
    collect_and_chain_truthy_targets(condition, &mut truthy);
    for target in truthy {
        if let TruthyGuardTarget::Identifier(name) = target {
            if !names.iter().any(|existing| existing == &name) {
                names.push(name);
            }
        }
    }
    names
}

/// Drops a guarded [`Type::GenuineUnknown`] identifier to [`Type::Unknown`] in a
/// scope stack, so an access inside the guarded branch is not reported as
/// `TS18046`.
fn downgrade_guarded_genuine_unknown_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
) {
    downgrade_genuine_unknown_in_scope(&guarded_value_identifiers(condition), scopes);
}

/// Drops each named [`Type::GenuineUnknown`] binding to [`Type::Unknown`] in a
/// scope stack, so an access in the guarded branch is not reported as `TS18046`.
pub(crate) fn downgrade_genuine_unknown_in_scope(names: &[String], scopes: &mut ScopeStack) {
    for name in names {
        let downgraded = scopes.resolve(name).and_then(|symbol| {
            matches!(symbol.ty, Type::GenuineUnknown).then(|| SymbolInfo {
                ty: Type::Unknown,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            })
        });
        if let Some(downgraded) = downgraded {
            let _ = scopes.insert_current_flow(name.clone(), downgraded);
        }
    }
}

fn collect_guard_operand_identifiers(condition: &ParsedExpression, names: &mut Vec<String>) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_guard_operand_identifiers(operand, names),
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or | ParsedLogicalOperator::And,
            right,
            ..
        } => {
            collect_guard_operand_identifiers(left, names);
            collect_guard_operand_identifiers(right, names);
        }
        _ => {
            if let Some(name) = guard_operand_identifier(condition)
                && !names.iter().any(|existing| existing == name)
            {
                names.push(name.to_string());
            }
        }
    }
}

/// Identifiers a condition *proves* guarded in `branch_is_true`. Unlike
/// [`collect_guard_operand_identifiers`], which ignores polarity, this walks the
/// operand shapes whose individual outcomes the branch pins down: `!`, the `&&`
/// operands of a true branch, and the `||` operands of a false one. That last
/// case is what the fall-through of `if (typeof x !== "object" || !("p" in x))
/// return;` needs — both disjuncts are false there, so both guards hold.
///
/// The true branch keeps the historical permissive reading of `A || B` (either
/// disjunct counts), since tightening it would turn suppressed `TS18046`s into
/// new false positives rather than removing any.
pub(crate) fn collect_holding_guard_identifiers(
    condition: &ParsedExpression,
    branch_is_true: bool,
    names: &mut Vec<String>,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_holding_guard_identifiers(operand, !branch_is_true, names),
        _ if strip_boolean_literal_comparison(condition).is_some() => {
            let (inner, flip) = strip_boolean_literal_comparison(condition)
                .expect("boolean-literal comparison checked above");
            collect_holding_guard_identifiers(inner, branch_is_true != flip, names);
        }
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } if !branch_is_true => {
            collect_holding_guard_identifiers(left, false, names);
            collect_holding_guard_identifiers(right, false, names);
        }
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And,
            right,
            ..
        } if branch_is_true => {
            collect_holding_guard_identifiers(left, true, names);
            collect_holding_guard_identifiers(right, true, names);
        }
        ParsedExpression::Logical {
            operator: ParsedLogicalOperator::Or,
            ..
        } if branch_is_true => collect_guard_operand_identifiers(condition, names),
        ParsedExpression::Logical { .. } => {}
        _ => {
            let name = if branch_is_true {
                guard_operand_identifier(condition).map(str::to_string)
            } else {
                // `typeof x !== "tag"` being false is itself a guard that holds.
                match parse_typeof_condition(condition) {
                    Some((ParsedExpression::Identifier { name, .. }, _, false)) => {
                        Some(name.clone())
                    }
                    _ => None,
                }
            };
            if let Some(name) = name
                && !names.iter().any(|existing| *existing == name)
            {
                names.push(name);
            }
        }
    }
}

/// Narrows for a branch by any recognized type guard: a discriminated-union
/// equality test (`x.kind === "a"`), a `typeof x === "tag"` test, an
/// `x instanceof Ctor` test, an `Array.isArray(x)` test, an `in`
/// property-presence test (`"prop" in x`), or a nullish equality test
/// (`x !== undefined`). Returns `None` if none apply.
pub(crate) fn narrow_condition_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let narrowed = narrow_condition_symbol_table_by_guard(condition, symbols, branch_is_true);
    let siblings =
        tuple_destructure_sibling_narrowings(condition, narrowed.as_ref().unwrap_or(symbols), branch_is_true);
    if siblings.is_empty() {
        return narrowed;
    }
    let mut table = narrowed.unwrap_or_else(|| symbols.clone_with_reason(TypeCopyReason::ScopeOrContext));
    for (name, symbol, declared) in siblings {
        table.insert_narrowed(name, symbol, declared);
    }
    Some(table)
}

/// The binding a condition proves truthy (`true`) or falsy (`false`), when the
/// condition is exactly that binding or its negation.
fn truthiness_tested_binding(condition: &ParsedExpression, branch_is_true: bool) -> Option<(&str, bool)> {
    match condition {
        ParsedExpression::Identifier { name, .. } => Some((name.as_str(), branch_is_true)),
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => truthiness_tested_binding(operand, !branch_is_true),
        _ => None,
    }
}

/// tsc's destructured-discriminated-union narrowing: `const [error, value] =
/// tuple` over a *union of tuples* binds dependent names, so proving `error`
/// falsy rules out the union members whose first element is not nullish, and
/// `value` is retyped from the survivors.
///
/// Only the source's own union is filtered — each sibling is re-derived from
/// it, never narrowed on its own — so a binding whose element is identical in
/// every surviving member keeps exactly the type it already had.
pub(crate) fn tuple_destructure_sibling_narrowings(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Vec<(Arc<str>, SymbolInfo, Type)> {
    // What the condition proves about one binding, as a test on the type a
    // source member gives that binding: truthiness keeps the members where it
    // can (or cannot) be nullish-only, a literal comparison the members that
    // can (or cannot only) be that literal — the discriminant of
    // `const { kind, payload } = action`.
    let (tested, keeps): (&str, Box<dyn Fn(&Type) -> bool>) =
        if let Some((tested, holds)) = truthiness_tested_binding(condition, branch_is_true) {
            // A member stays where its read can be what the test found: truthy
            // (`done: true`, `true | false`) or falsy (`done: false`, a nullish
            // or empty literal) — so a boolean-literal discriminant partitions
            // `{ done: false; value: T } | { done: true; value?: undefined }`.
            (
                tested,
                Box::new(move |read: &Type| {
                    let definitely_falsy = |ty: &Type| match ty {
                        Type::Never
                        | Type::Undefined
                        | Type::Null
                        | Type::Void
                        | Type::BooleanLiteral(false) => true,
                        Type::StringLiteral(value) => value.is_empty(),
                        Type::NumberLiteral(literal) => literal.value == "0",
                        _ => false,
                    };
                    if holds {
                        !definitely_falsy(&truthy::remove_definitely_falsy(read))
                    } else {
                        !matches!(truthy::keep_possibly_falsy(read), Type::Never)
                    }
                }),
            )
        } else if let Some((tested, nullish, eq)) = identifier_nullish_equality(condition) {
            // `err === null` keeps the members whose read can be `null`; its
            // other edge the ones whose read is something besides. `==`
            // matches `null` and `undefined` alike.
            let holds = eq == branch_is_true;
            (
                tested,
                Box::new(move |read: &Type| {
                    let is_nullish = |ty: &Type| nullish.iter().any(|nullish| ty == nullish);
                    let members = match read {
                        Type::Union(union) => union.types().to_vec(),
                        other => vec![other.clone()],
                    };
                    if holds {
                        read.is_unknown()
                            || matches!(read, Type::Any)
                            || members.iter().any(|member| is_nullish(member))
                    } else {
                        members.iter().any(|member| !is_nullish(member))
                    }
                }),
            )
        } else if let Some((tested, literal, eq)) = parse_identifier_literal_equality(condition, symbols) {
            let holds = eq == branch_is_true;
            (
                tested,
                Box::new(move |read: &Type| {
                    if holds {
                        surge_ts_types::is_assignable_to(&literal, read)
                    } else {
                        *read != literal
                    }
                }),
            )
        } else {
            return Vec::new();
        };
    let Some(binding) = symbols.tuple_destructure(tested) else {
        return Vec::new();
    };
    let (source, tested_key) = (binding.source, binding.key);
    let Some(source_type) = binding
        .source_type
        .or_else(|| symbols.get(&source).map(|symbol| symbol.ty.clone()))
    else {
        return Vec::new();
    };
    // A named source the same test already narrowed (it is a discriminant
    // alias too: `kind` stands for `action.kind`) arrives as the one member
    // left, and the siblings are simply read off it again.
    let members: Vec<Type> = match source_type.peeled() {
        Type::Union(union) => union.types().iter().map(Type::peeled).collect(),
        narrowed @ (Type::Object(_) | Type::Tuple(_)) => vec![narrowed],
        _ => return Vec::new(),
    };
    let read_union = |key: &crate::symbols::DestructureKey| -> Option<Type> {
        members
            .iter()
            .map(|member| key.read(member))
            .collect::<Option<Vec<Type>>>()
            .map(union_type)
    };
    // A binding holds what it reads or a narrowing of it; anything else is a
    // different declaration under the same name.
    let still_bound = |name: &str, key: &crate::symbols::DestructureKey| {
        symbols.get(name).is_some_and(|symbol| {
            read_union(key).is_some_and(|read| surge_ts_types::is_assignable_to(&symbol.ty, &read))
        })
    };
    let already_narrowed = members.len() == 1;
    if !already_narrowed && !still_bound(tested, &tested_key) {
        return Vec::new();
    }

    let kept: Vec<&Type> = members
        .iter()
        .filter(|member| {
            already_narrowed || tested_key.read(member).is_some_and(|read| keeps(&read))
        })
        .collect();
    if kept.is_empty() || (!already_narrowed && kept.len() == members.len()) {
        return Vec::new();
    }

    let mut narrowings = Vec::new();
    for (name, key) in symbols.tuple_destructure_siblings(&source) {
        if !already_narrowed && !still_bound(&name, &key) {
            continue;
        }
        let Some(symbol) = symbols.get(&name) else {
            continue;
        };
        let selected: Vec<Type> = kept.iter().filter_map(|member| key.read(member)).collect();
        if selected.len() != kept.len() {
            continue;
        }
        // tsc makes the read off the surviving members the sibling's declared
        // type, which the sibling's own guards then narrow: after
        // `if (payload)`, `kind === 'A'` leaves `payload` the `number` of
        // `number | undefined`. The tested binding keeps what its own guard
        // made of it.
        let narrowed = surge_ts_types::with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            renarrowed(&union_type(selected), &symbol.ty)
        });
        if narrowed == symbol.ty {
            continue;
        }
        let declared = symbols.declared_type(&name).unwrap_or(&symbol.ty).clone();
        narrowings.push((
            name,
            SymbolInfo {
                ty: narrowed,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            },
            declared,
        ));
    }
    narrowings
}

/// `x === null` / `undefined !== x` / `x == null`: the local compared, the
/// nullish types the comparison matches, and whether it is an equality.
fn identifier_nullish_equality(condition: &ParsedExpression) -> Option<(&str, Vec<Type>, bool)> {
    use surge_ts_syntax::{ParsedBinaryOperator, ParsedUnaryOperator};
    let ParsedExpression::Binary {
        left,
        operator,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let (eq, strict) = match operator {
        ParsedBinaryOperator::StrictEquals => (true, true),
        ParsedBinaryOperator::StrictNotEquals => (false, true),
        ParsedBinaryOperator::Equals => (true, false),
        ParsedBinaryOperator::NotEquals => (false, false),
        _ => return None,
    };
    let nullish = |expression: &ParsedExpression| match expression {
        ParsedExpression::NullLiteral => Some(Type::Null),
        ParsedExpression::UndefinedLiteral
        | ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Void,
            ..
        } => Some(Type::Undefined),
        _ => None,
    };
    let (name, matched) = match (left.as_ref(), right.as_ref()) {
        (ParsedExpression::Identifier { name, .. }, other)
        | (other, ParsedExpression::Identifier { name, .. }) => (name.as_str(), nullish(other)?),
        _ => return None,
    };
    let matched = if strict {
        vec![matched]
    } else {
        vec![Type::Null, Type::Undefined]
    };
    Some((name, matched, eq))
}

/// `declared` narrowed the way `current` already narrowed the binding: a
/// member `current` admits stays, and one it narrowed further (`string` to
/// `"a"`) becomes the members of `current` it narrowed to.
fn renarrowed(declared: &Type, current: &Type) -> Type {
    let members = |ty: &Type| match ty {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let current_members = members(current);
    let mut kept = Vec::new();
    for member in members(declared) {
        if surge_ts_types::is_assignable_to(&member, current) {
            kept.push(member);
        } else {
            kept.extend(
                current_members
                    .iter()
                    .filter(|narrowed| surge_ts_types::is_assignable_to(narrowed, &member))
                    .cloned(),
            );
        }
    }
    union_type(kept)
}

fn narrow_condition_symbol_table_by_guard(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    // `ok ? a : b` where `ok` is a boolean `const` alias narrows by the
    // condition the alias was written as, not by the opaque identifier.
    // …and by the alias binding itself, which is what was tested.
    if let ParsedExpression::Identifier { name, .. } = condition
        && let Some(alias) = symbols.alias_condition(name)
        && let Some(inlining) = enter_alias_inlining()
    {
        let inlined = retain_constant_reference_guards(&alias, symbols, &|_| false);
        let by_alias = narrow_condition_symbol_table(&inlined, symbols, branch_is_true);
        drop(inlining);
        return narrow_reference_guard_symbol_table(
            condition,
            by_alias.as_ref().unwrap_or(symbols),
            branch_is_true,
        )
        .or(by_alias);
    }
    // `!guard` narrows the opposite branch.
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    {
        return narrow_condition_symbol_table(operand, symbols, !branch_is_true);
    }
    if let Some((inner, flip)) = strip_boolean_literal_comparison(condition) {
        return narrow_condition_symbol_table(inner, symbols, branch_is_true != flip);
    }
    // tsc's `narrowType` reads through `satisfies`.
    if let ParsedExpression::SatisfiesExpression { expression, .. } = condition {
        return narrow_condition_symbol_table(expression, symbols, branch_is_true);
    }
    // tsc's `narrowTypeByBinaryExpression`: an assignment tested as a
    // condition narrows by its value first, then by the target's truthiness
    // (`if (thing &&= other)` proves `other` truthy as well).
    if let ParsedExpression::Assignment { value, .. } = condition {
        let by_value = narrow_condition_symbol_table(value, symbols, branch_is_true);
        return narrow_reference_guard_symbol_table(
            condition,
            by_value.as_ref().unwrap_or(symbols),
            branch_is_true,
        )
        .or(by_value);
    }

    // Every operand of an `&&` holds in its true branch, so a chain narrows by
    // all of them (`a !== undefined && b !== undefined && a <= b`). The false
    // branch proves nothing about any individual operand.
    if branch_is_true
        && let ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And,
            right,
            ..
        } = condition
    {
        let left_narrowed = narrow_condition_symbol_table(left, symbols, true);
        let base = left_narrowed.as_ref().unwrap_or(symbols);
        return narrow_condition_symbol_table(right, base, true).or(left_narrowed);
    }

    // Every operand of an `||` fails in its false branch, so the chain narrows
    // by all of them (`!a || !b || a.x` reads `a.x` with both present).
    if !branch_is_true
        && let ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } = condition
    {
        let left_narrowed = narrow_condition_symbol_table(left, symbols, false);
        let base = left_narrowed.as_ref().unwrap_or(symbols);
        return narrow_condition_symbol_table(right, base, false).or(left_narrowed);
    }

    // `A || B` holds when A does, or when A fails and B then holds, so the
    // subject is the union of what those two edges prove. A disjunct that
    // narrows nothing leaves the subject unconstrained, and the union collapses
    // back to the declared type — which is why both sides have to narrow for
    // this to say anything.
    if branch_is_true
        && let ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } = condition
    {
        let left_holds = narrow_condition_symbol_table(left, symbols, true)?;
        let left_fails = narrow_condition_symbol_table(left, symbols, false);
        let right_holds = narrow_condition_symbol_table(
            right,
            left_fails.as_ref().unwrap_or(symbols),
            true,
        )?;
        return union_disjunct_narrowings(symbols, &left_holds, &right_holds);
    }

    // `A && B` fails when A does, or when A holds and B then fails.
    if !branch_is_true
        && let ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And,
            right,
            ..
        } = condition
    {
        let left_fails = narrow_condition_symbol_table(left, symbols, false);
        let left_holds = narrow_condition_symbol_table(left, symbols, true);
        let right_fails = narrow_condition_symbol_table(
            right,
            left_holds.as_ref().unwrap_or(symbols),
            false,
        );
        if left_fails.is_none() && right_fails.is_none() {
            return None;
        }
        return union_disjunct_narrowings(
            symbols,
            left_fails.as_ref().unwrap_or(symbols),
            right_fails.as_ref().or(left_holds.as_ref()).unwrap_or(symbols),
        );
    }

    let narrowed = narrow_discriminant_symbol_table(condition, symbols, branch_is_true)
        .or_else(|| narrow_literal_equality_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_typeof_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_instanceof_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_array_isarray_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_property_presence_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_nullish_equality_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| narrow_property_guard_symbol_table(condition, symbols, branch_is_true))
        .or_else(|| {
            // As in `narrow_truthy_reference_in_scope`: the members the test rules
            // out go first, and the reference guard then narrows what is left.
            let filtered = truthy::narrow_union_by_property_truthiness_symbol_table(
                condition,
                symbols,
                branch_is_true,
            );
            narrow_reference_guard_symbol_table(
                condition,
                filtered.as_ref().unwrap_or(symbols),
                branch_is_true,
            )
            .or(filtered)
        })
        .or_else(|| narrow_reference_equality_symbol_table(condition, symbols, branch_is_true));
    match destructured_discriminant_guard(condition, symbols) {
        Some(rewritten) => narrow_condition_symbol_table_by_guard(
            &rewritten,
            narrowed.as_ref().unwrap_or(symbols),
            branch_is_true,
        )
        .or(narrowed),
        None => narrowed,
    }
}

/// A guard over a binding destructured from another (`const { kind } = obj`),
/// written over the property that binding reads (`obj.kind === "a"`): tsc's
/// `getCandidateDiscriminantPropertyAccess` narrows `obj` by a test of `kind`
/// as it would by a test of `obj.kind`.
fn destructured_discriminant_guard(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<ParsedExpression> {
    let property_read = |expression: &ParsedExpression| -> Option<ParsedExpression> {
        let ParsedExpression::Identifier { name, span } = expression else {
            return None;
        };
        let binding = symbols.tuple_destructure(name)?;
        let crate::symbols::DestructureKey::Property(property) = &binding.key else {
            return None;
        };
        if binding.source.starts_with('\0') {
            return None;
        }
        Some(ParsedExpression::PropertyAccess {
            object: Box::new(ParsedExpression::Identifier {
                name: binding.source.to_string(),
                span: *span,
            }),
            object_span: *span,
            property_name: property.to_string(),
            property_span: None,
            is_bracketed: false,
            binding_element: false,
        })
    };
    let operand = |expression: &ParsedExpression| match expression {
        ParsedExpression::Unary {
            operator,
            operator_span,
            operand,
            operand_span,
        } => property_read(operand).map(|read| ParsedExpression::Unary {
            operator: *operator,
            operator_span: *operator_span,
            operand: Box::new(read),
            operand_span: *operand_span,
        }),
        other => property_read(other),
    };
    match condition {
        ParsedExpression::Binary {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
        } => {
            let new_left = operand(left);
            let new_right = operand(right);
            if new_left.is_none() && new_right.is_none() {
                return None;
            }
            Some(ParsedExpression::Binary {
                left: Box::new(new_left.unwrap_or_else(|| left.as_ref().clone())),
                left_span: *left_span,
                operator: *operator,
                operator_span: *operator_span,
                right: Box::new(new_right.unwrap_or_else(|| right.as_ref().clone())),
                right_span: *right_span,
            })
        }
        other => property_read(other),
    }
}

/// Merges the two disjunct narrowings of an `A || B` true branch: each name
/// either side narrowed becomes the union of what both leave it as.
fn union_disjunct_narrowings(
    symbols: &SymbolTable,
    left: &SymbolTable,
    right: &SymbolTable,
) -> Option<SymbolTable> {
    let mut names: Vec<Arc<str>> = left.narrowed_names().cloned().collect();
    for name in right.narrowed_names() {
        if !names.contains(name) {
            names.push(Arc::clone(name));
        }
    }

    let mut merged = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let mut changed = false;
    for name in names {
        let (Some(declared), Some(left_symbol), Some(right_symbol)) =
            (symbols.get(&name), left.get(&name), right.get(&name))
        else {
            continue;
        };
        let narrowed = union_type(vec![left_symbol.ty.clone(), right_symbol.ty.clone()]);
        if narrowed == declared.ty {
            continue;
        }
        let declared_ty = declared.ty.clone();
        let narrowed_symbol = SymbolInfo {
            ty: narrowed,
            kind: declared.kind,
            function_signature: declared.function_signature.clone(),
        };
        merged.insert_narrowed(Arc::clone(&name), narrowed_symbol, declared_ty);
        changed = true;
    }

    changed.then_some(merged)
}

/// Removes `null`/`undefined` from a reference's type in the current scope.
/// `for (const _ in ref)` is the one statement form that narrows this way
/// without a condition (tsc, flow.go: "for (const _ in ref) acts as a nonnull
/// on ref").
pub(crate) fn narrow_reference_non_null_in_scope(
    expression: &ParsedExpression,
    scopes: &mut ScopeStack,
) {
    let Some((base, path)) = reference_path(expression) else {
        return;
    };
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::Nullish {
            keep_matching: false,
            test: guards::NullishTest {
                null: true,
                undefined: true,
            },
        },
        scopes,
    );
}

fn narrow_value_guards_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    // `!guard` narrows the opposite branch.
    if let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    {
        narrow_value_guards_in_scope(operand, scopes, !branch_is_true, ctx);
        return;
    }
    if let Some((inner, flip)) = strip_boolean_literal_comparison(condition) {
        narrow_value_guards_in_scope(inner, scopes, branch_is_true != flip, ctx);
        return;
    }
    if let ParsedExpression::SatisfiesExpression { expression, .. } = condition {
        narrow_value_guards_in_scope(expression, scopes, branch_is_true, ctx);
        return;
    }
    if let ParsedExpression::Assignment { value, .. } = condition {
        narrow_value_guards_in_scope(value, scopes, branch_is_true, ctx);
    }

    narrow_value_guards_by_guard(condition, scopes, branch_is_true, ctx);

    // A guard surge could not turn into a type still narrows a
    // genuinely-`unknown` value for tsc, so a later access inside the branch is
    // not a `TS18046`: what is left `unknown` after the narrowers above drops
    // to the degradation sentinel. See the matching `&&`-operand path in
    // `narrow_truthy_operand_symbol_table`.
    if branch_is_true {
        downgrade_guarded_genuine_unknown_in_scope(condition, scopes);
    } else {
        let mut names = Vec::new();
        collect_holding_guard_identifiers(condition, false, &mut names);
        downgrade_genuine_unknown_in_scope(&names, scopes);
    }
}

fn narrow_value_guards_by_guard(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    narrow_element_reference_guards_in_scope(condition, scopes, branch_is_true, ctx);

    if narrow_logical_guard_in_scope(condition, scopes, branch_is_true, ctx) {
        return;
    }
    if narrow_optional_call_containment_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_constructor_equality_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_typeof_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_instanceof_in_scope(condition, scopes, branch_is_true, ctx) {
        return;
    }
    if narrow_array_isarray_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_arraybuffer_isview_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_property_presence_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    narrow_has_own_property_in_scope(condition, scopes, branch_is_true);
    if narrow_predicate_call_in_scope(condition, scopes, branch_is_true, ctx) {
        return;
    }
    if narrow_this_predicate_call_in_scope(condition, scopes, branch_is_true, ctx) {
        return;
    }
    if narrow_truthy_reference_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_nullish_equality_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    if narrow_literal_equality_in_scope(condition, scopes, branch_is_true) {
        return;
    }
    // Narrows the property itself; the discriminant test below then still
    // filters the base union by the same condition, so the two compose.
    narrow_literal_equality_reference_in_scope(condition, scopes, branch_is_true);

    let parsed = {
        let symbols = scopes.visible_symbols();
        parse_discriminant_condition_with(condition, &|expression| {
            const_member_literal_value(expression, symbols)
        })
    };
    let Some((discriminant_object, property, literal, eq)) = parsed else {
        narrow_reference_equality_in_scope(condition, scopes, branch_is_true);
        return;
    };
    let keep_matching = branch_is_true == eq;

    // A discriminant more than one member down (`this.state.inner.kind`) is
    // narrowed along its reference path; the arms below keep the two shallow
    // shapes they were written for.
    if let Some((base, path)) = reference_path(discriminant_object)
        && path.len() > 1
    {
        narrow_reference_in_scope(
            &base,
            &path,
            ReferenceGuard::Discriminant {
                property,
                literal: &literal,
                keep_matching,
            },
            scopes,
        );
        return;
    }

    let (base_name, narrowed_symbol, declared) = match discriminant_object {
        ParsedExpression::Identifier { name, .. } => {
            let Some(symbol) = scopes.resolve(name) else {
                return;
            };
            if !guards::narrows_by_discriminant(&symbol.ty, scopes.visible_symbols().declared_type(name)) {
                return;
            }
            let Some(narrowed) = narrow_discriminant_through_optional_chain(
                condition,
                &symbol.ty,
                property,
                &literal,
                keep_matching,
            ) else {
                return;
            };
            (
                name.clone(),
                SymbolInfo {
                    ty: narrowed,
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
                symbol.ty.clone(),
            )
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
                _ => return,
            };
            let Some(symbol) = scopes.resolve(name) else {
                return;
            };
            // The helper peels a named-typed base (`draft: Draft`) to narrow its
            // discriminant property in scope.
            let Some(narrowed) = narrow_base_property_by_discriminant(
                &symbol.ty,
                base_property,
                condition,
                property,
                &literal,
                keep_matching,
            ) else {
                return;
            };
            (
                name.clone(),
                SymbolInfo {
                    ty: narrowed,
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
                symbol.ty.clone(),
            )
        }
        _ => return,
    };

    // Insert into the *current* frame (shadowing the binding's owner) rather than
    // mutating the owning frame: the then-branch narrowing must stay confined to
    // its pushed child scope, or it leaks into the parent and corrupts the
    // else/fall-through branch (which would then narrow an already-narrowed
    // non-union to nothing). `pop_child` restores the shadow.
    let _ = scopes.insert_current_narrowed(base_name, narrowed_symbol, declared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use surge_ts_types::union_type;

    fn union3() -> Type {
        union_type(vec![
            Type::String,
            Type::Number,
            Type::Array(Box::new(Type::Number)),
        ])
    }

    #[test]
    fn typeof_keeps_matching_member_in_true_branch() {
        let narrowed = narrow_union_by_typeof(&union3(), "number", true).unwrap();
        assert_eq!(narrowed, Type::Number);
    }

    #[test]
    fn typeof_removes_matching_member_in_false_branch() {
        let narrowed = narrow_union_by_typeof(&union3(), "string", false).unwrap();
        assert_eq!(
            narrowed,
            union_type(vec![Type::Number, Type::Array(Box::new(Type::Number))])
        );
    }

    #[test]
    fn arrayness_keeps_array_member_in_true_branch() {
        let narrowed = narrow_union_by_arrayness(&union3(), true).unwrap();
        assert_eq!(narrowed, Type::Array(Box::new(Type::Number)));
    }

    #[test]
    fn arrayness_removes_array_member_in_false_branch() {
        let narrowed = narrow_union_by_arrayness(&union3(), false).unwrap();
        assert_eq!(narrowed, union_type(vec![Type::String, Type::Number]));
    }

    #[test]
    fn narrowing_returns_none_when_nothing_changes() {
        // A non-union type is never narrowed.
        assert!(narrow_union_by_arrayness(&Type::String, true).is_none());
    }

    #[test]
    fn typeof_narrows_an_unreachable_branch_to_never() {
        // No member has tag "boolean", so nothing can reach the true branch and
        // tsc types the subject `never` there — not "unchanged".
        assert_eq!(
            narrow_union_by_typeof(&union3(), "boolean", true),
            Some(Type::Never)
        );
    }

    struct FixedResolver(Type);
    impl surge_ts_types::ResolveReference for FixedResolver {
        fn resolve(&self) -> Type {
            self.0.clone()
        }
    }

    fn view_reference(display: &str) -> Type {
        Type::Reference(surge_ts_types::TypeReference::new(
            format!("lib.dom.d.ts\u{0}{}", display.split('<').next().unwrap()),
            display,
            Vec::new(),
            std::sync::Arc::new(FixedResolver(Type::Unknown)),
        ))
    }

    #[test]
    fn arraybufferview_keeps_view_members_in_true_branch() {
        // `ArrayBuffer.isView(x)` keeps the `ArrayBufferView<ArrayBuffer>` member
        // and drops the `ArrayBuffer` / primitive members.
        let union = union_type(vec![
            view_reference("ArrayBufferView<ArrayBuffer>"),
            view_reference("ArrayBuffer"),
            Type::String,
        ]);
        let narrowed = narrow_union_by_arraybufferview(&union, true).unwrap();
        assert_eq!(narrowed, view_reference("ArrayBufferView<ArrayBuffer>"));
    }

    #[test]
    fn arraybufferview_removes_view_members_in_false_branch() {
        let union = union_type(vec![
            view_reference("Int8Array"),
            view_reference("ArrayBuffer"),
            Type::String,
        ]);
        let narrowed = narrow_union_by_arraybufferview(&union, false).unwrap();
        assert_eq!(
            narrowed,
            union_type(vec![view_reference("ArrayBuffer"), Type::String])
        );
    }

    fn object(properties: &[(&str, bool)]) -> Type {
        let mut map = surge_ts_types::PropertyMap::default();
        for (name, optional) in properties {
            map.insert(
                (*name).into(),
                surge_ts_types::ObjectProperty {
                    ty: Type::Number,
                    optional: *optional,
                    method: false,
                    readonly: false,
                    restriction: None,
                    index_slot: false,
                },
            );
        }
        Type::Object(surge_ts_types::ObjectType::new(map, None))
    }

    fn named_reference(display: &str, target: Type) -> Type {
        Type::Reference(surge_ts_types::TypeReference::new(
            format!("test.ts\u{0}{display}"),
            display,
            Vec::new(),
            std::sync::Arc::new(FixedResolver(target)),
        ))
    }

    #[test]
    fn property_presence_narrows_through_nominal_references() {
        // `Named` is an interface/alias wrapper, so the union has no bare
        // `Type::Object` member to decide on without peeling.
        let named = named_reference("Named", object(&[("a", false)]));
        let other = named_reference("Other", object(&[("b", false)]));
        let union = union_type(vec![named.clone(), other.clone()]);

        assert_eq!(
            narrow_union_by_property_presence(&union, "a", true).unwrap(),
            named
        );
        assert_eq!(
            narrow_union_by_property_presence(&union, "a", false).unwrap(),
            other
        );
    }

    #[test]
    fn property_presence_flattens_an_alias_to_a_union() {
        let o1 = object(&[("in", false), ("key", false)]);
        let o2 = object(&[("key", false), ("map", false)]);
        let field = named_reference("Field", union_type(vec![o1.clone(), o2.clone()]));
        let fields = named_reference("Fields", object(&[("fields", false)]));
        let union = union_type(vec![field.clone(), fields.clone()]);

        assert_eq!(
            narrow_union_by_property_presence(&union, "in", true).unwrap(),
            o1
        );
        assert_eq!(
            narrow_union_by_property_presence(&union, "in", false).unwrap(),
            union_type(vec![o2, fields])
        );
        // Every constituent of `Field` has `key`, so the nominal wrapper survives
        // intact rather than being spliced into its constituents.
        assert_eq!(
            narrow_union_by_property_presence(&union, "key", true).unwrap(),
            field
        );
    }

    #[test]
    fn property_presence_narrows_an_alias_to_a_union_at_the_top_level() {
        let o1 = object(&[("in", false)]);
        let o2 = object(&[("map", false)]);
        let field = named_reference("Field", union_type(vec![o1.clone(), o2.clone()]));

        assert_eq!(
            narrow_union_by_property_presence(&field, "map", true).unwrap(),
            o2
        );
        assert_eq!(
            narrow_union_by_property_presence(&field, "map", false).unwrap(),
            o1
        );
    }

    #[test]
    fn property_presence_retains_an_optional_member_in_the_false_branch() {
        // tsc keeps `{ a?: number }` in the else branch of `if ("a" in v)` — the
        // key may legitimately be absent at runtime.
        let optional = object(&[("a", true), ("z", false)]);
        let without = object(&[("b", false)]);
        let union = union_type(vec![optional.clone(), without.clone()]);

        assert_eq!(
            narrow_union_by_property_presence(&union, "a", true).unwrap(),
            optional
        );
        assert!(narrow_union_by_property_presence(&union, "a", false).is_none());
    }

    #[test]
    fn property_presence_keeps_undecidable_members() {
        // A string index signature may supply the key, and a primitive member
        // cannot be classified — neither branch may drop them.
        let indexed = Type::Object(surge_ts_types::ObjectType::new(
            surge_ts_types::PropertyMap::default(),
            Some(Type::Number),
        ));
        let union = union_type(vec![indexed.clone(), Type::String]);
        assert!(narrow_union_by_property_presence(&union, "a", true).is_none());
        assert!(narrow_union_by_property_presence(&union, "a", false).is_none());

        // A decidably-absent member alongside them still goes in the true branch.
        let with_absent = union_type(vec![indexed, Type::String, object(&[("b", false)])]);
        assert_eq!(
            narrow_union_by_property_presence(&with_absent, "a", true).unwrap(),
            union
        );
    }
}

/// The references an optional chain reads through, outermost first: tsc's
/// `optionalChainContainsReference` walks `expression` up the chain while the
/// node is part of it.
pub(crate) fn optional_chain_contained_references(expression: &ParsedExpression) -> Vec<&ParsedExpression> {
    fn in_chain(expression: &ParsedExpression) -> bool {
        match expression {
            ParsedExpression::OptionalPropertyAccess { .. } | ParsedExpression::OptionalIndexAccess { .. } => true,
            ParsedExpression::PropertyAccess { object, .. } | ParsedExpression::ElementAccess { object, .. } => {
                in_chain(object)
            }
            _ => false,
        }
    }
    let mut contained = Vec::new();
    let mut current = expression;
    while in_chain(current) {
        let (ParsedExpression::OptionalPropertyAccess { object, .. }
        | ParsedExpression::OptionalIndexAccess { object, .. }
        | ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::ElementAccess { object, .. }) = current
        else {
            break;
        };
        contained.push(object.as_ref());
        current = object;
    }
    contained
}

/// tsc's `unknownEmptyObjectType`: the `{}` a narrowed `unknown` leaves, told
/// apart from a written `{}` (only it may be a primitive on the right of `in`).
pub(crate) fn unknown_empty_object() -> Type {
    Type::Object(surge_ts_types::ObjectType::new(Default::default(), None).with_alias_name(UNKNOWN_EMPTY_OBJECT))
}

pub(crate) const UNKNOWN_EMPTY_OBJECT: &str = "{}";

pub(crate) fn is_unknown_empty_object(ty: &Type) -> bool {
    matches!(ty, Type::Object(object) if object.alias_name.as_deref() == Some(UNKNOWN_EMPTY_OBJECT) && object.properties.is_empty())
}
