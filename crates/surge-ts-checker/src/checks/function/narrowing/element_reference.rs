use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator};
use surge_ts_types::{Type, TypeCopyReason};

use crate::context::CheckerContext;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};
use super::guards::*;
use super::{ReferenceGuard, reference_path};

/// The key a narrowable element access is remembered under: `xs[0]`,
/// `db.posts[nextIndex]`. tsc narrows an element access whose key is a
/// literal or a `const` identifier; surge cannot see constness, so any
/// identifier key qualifies — an assignment to the key inside the guarded
/// block would then read a stale narrowing, which is rare and only under-reports.
pub(crate) fn element_reference_key(
    object: &ParsedExpression,
    index: &ParsedExpression,
) -> Option<String> {
    let (base, path) = reference_path(object)?;
    element_reference_key_named(&base, &path, index)
}

pub(crate) fn element_reference_key_named(
    base: &str,
    path: &[String],
    index: &ParsedExpression,
) -> Option<String> {
    let key = match index {
        ParsedExpression::NumberLiteral(value) => value.as_str(),
        ParsedExpression::Identifier { name, .. } => name.as_str(),
        _ => return None,
    };
    let mut rendered = String::with_capacity(base.len() + 8);
    rendered.push_str(base);
    for segment in path {
        rendered.push('.');
        rendered.push_str(segment);
    }
    rendered.push('[');
    rendered.push_str(key);
    rendered.push(']');
    Some(rendered)
}

pub(crate) fn element_access_parts(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::ElementAccess { object, index, .. }
        | ParsedExpression::OptionalIndexAccess { object, index, .. } => {
            element_reference_key(object, index)
        }
        ParsedExpression::IndexAccess {
            object_name, index, ..
        } => element_reference_key_named(object_name, &[], index),
        _ => None,
    }
}

/// Guards on element accesses (`if (xs[0])`, `xs[i] !== undefined`) narrow the
/// access itself, which no binding's type can express: an array has one element
/// type for every index. The narrowed read is remembered under the access's
/// rendered key and consulted by the element-access inference paths.
pub(super) fn narrow_element_reference_guards_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    let narrowed = {
        let visible = scopes.visible_symbols();
        narrowed_element_references(condition, branch_is_true, visible, ctx)
    };
    for (key, narrowed, declared) in narrowed {
        let _ = scopes.insert_current_narrowed(
            key,
            SymbolInfo {
                ty: narrowed,
                kind: crate::symbols::SymbolKind::Var,
                function_signature: None,
            },
            declared,
        );
    }
}

/// Symbol-table counterpart of [`narrow_element_reference_guards_in_scope`],
/// for the operand and branch positions (`xs[i] && xs[i].x`, `xs[i] ? … : …`)
/// that narrow a table rather than the scope stack.
pub(crate) fn narrow_element_reference_guards_symbol_table(
    condition: &ParsedExpression,
    branch_is_true: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    let narrowed = narrowed_element_references(condition, branch_is_true, symbols, ctx);
    if narrowed.is_empty() {
        return None;
    }
    let mut table = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    for (key, narrowed, declared) in narrowed {
        table.insert_narrowed(
            key,
            SymbolInfo {
                ty: narrowed,
                kind: crate::symbols::SymbolKind::Var,
                function_signature: None,
            },
            declared,
        );
    }
    Some(table)
}

/// `(key, narrowed, declared)` for every element-access guard `condition`
/// proves in the branch.
pub(super) fn narrowed_element_references(
    condition: &ParsedExpression,
    branch_is_true: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Vec<(String, Type, Type)> {
    // tsc's `narrowTypeByBinaryExpression`: `A || B` holds where A does, or
    // where A fails and B then holds (`A && B` fails alike), so an access is
    // the union of the two edges' narrowings; one an edge leaves alone stays.
    if let ParsedExpression::Logical {
        left,
        operator,
        right,
        ..
    } = condition
        && matches!(
            (operator, branch_is_true),
            (ParsedLogicalOperator::Or, true) | (ParsedLogicalOperator::And, false)
        )
    {
        let left_edge = narrowed_element_references(left, branch_is_true, symbols, ctx);
        if left_edge.is_empty() {
            return Vec::new();
        }
        let left_other = narrowed_element_references(left, !branch_is_true, symbols, ctx);
        let mut right_base = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
        for (key, narrowed, declared) in left_other {
            right_base.insert_narrowed(
                key,
                SymbolInfo {
                    ty: narrowed,
                    kind: crate::symbols::SymbolKind::Var,
                    function_signature: None,
                },
                declared,
            );
        }
        let right_edge = narrowed_element_references(right, branch_is_true, &right_base, ctx);
        return left_edge
            .into_iter()
            .filter_map(|(key, left_type, declared)| {
                let (_, right_type, _) = right_edge.iter().find(|(right_key, ..)| *right_key == key)?;
                let joined = surge_ts_types::union_type(vec![left_type, right_type.clone()]);
                (joined != declared).then_some((key, joined, declared))
            })
            .collect();
    }
    // `A && B` holds where A held and then B: B reads the accesses A narrowed
    // (`xs[0] && isOne(xs[0]) && xs[0].config` sees `xs[0]` as `One`).
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
        let mut combined = narrowed_element_references(left, branch_is_true, symbols, ctx);
        if combined.is_empty() {
            return narrowed_element_references(right, branch_is_true, symbols, ctx);
        }
        let mut right_base = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
        for (key, narrowed, declared) in &combined {
            right_base.insert_narrowed(
                key.clone(),
                SymbolInfo {
                    ty: narrowed.clone(),
                    kind: crate::symbols::SymbolKind::Var,
                    function_signature: None,
                },
                declared.clone(),
            );
        }
        for (key, narrowed, declared) in narrowed_element_references(right, branch_is_true, &right_base, ctx) {
            match combined.iter_mut().find(|(existing, ..)| *existing == key) {
                Some(entry) => entry.1 = narrowed,
                None => combined.push((key, narrowed, declared)),
            }
        }
        return combined;
    }
    let mut guards = Vec::new();
    collect_element_reference_guards(condition, branch_is_true, &mut guards);
    let mut narrowed = Vec::new();
    narrowed_element_predicate_references(condition, branch_is_true, symbols, ctx, &mut narrowed);
    for (access, guard) in guards {
        let Some(key) = element_access_parts(access) else {
            continue;
        };
        let declared = match crate::infer::infer_expression(access, symbols, ctx) {
            crate::infer::InferredExpression::Known(ty) => ty,
            _ => continue,
        };
        if declared.is_unmodelled() && declared != Type::GenuineUnknown {
            continue;
        }
        let Some((narrowed_ty, _)) = guard.narrow_leaf(&declared, false) else {
            continue;
        };
        if narrowed_ty == declared {
            continue;
        }
        narrowed.push((key, narrowed_ty, declared));
    }
    narrowed_element_property_references(condition, branch_is_true, symbols, ctx, &mut narrowed);
    narrowed
}

/// tsc's `isMatchingReference` reads `xs[0]` as a reference like `xs.p`, so a
/// guard on a property reached through it (`!results[0].success`,
/// `xs[i].kind === "a"`) narrows the access. The condition is read with each
/// keyed access standing as a name, which the reference-guard collector then
/// sees as a base with a property path.
fn narrowed_element_property_references(
    condition: &ParsedExpression,
    branch_is_true: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
    narrowed: &mut Vec<(String, Type, Type)>,
) {
    let mut accesses: Vec<(String, ParsedExpression)> = Vec::new();
    let mut rooted = condition.clone();
    root_element_accesses(&mut rooted, &mut accesses);
    if accesses.is_empty() {
        return;
    }
    let mut guards = Vec::new();
    super::collect_reference_guards(
        &rooted,
        branch_is_true,
        &super::compared_operand_type(symbols),
        &mut guards,
    );
    // Every guard on an access a member path guards applies in turn
    // (`xs[0]?.kind === "a"`: the chain proves `xs[0]` present, then the
    // comparison discriminates it); an access only the condition itself
    // guards is the bare access's, read above.
    let mut path_guarded: Vec<&str> = Vec::new();
    for (base, path, _) in &guards {
        if !path.is_empty() && !path_guarded.contains(&base.as_str()) {
            path_guarded.push(base);
        }
    }
    for base in path_guarded {
        let Some((_, access)) = accesses.iter().find(|(key, _)| key == base) else {
            continue;
        };
        if !element_key_is_constant(access, symbols, ctx) {
            continue;
        }
        let declared = match crate::infer::infer_expression(access, symbols, ctx) {
            crate::infer::InferredExpression::Known(ty) if !ty.is_unmodelled() => ty,
            _ => continue,
        };
        let mut narrowed_ty = declared.clone();
        for (_, path, guard) in guards.iter().filter(|(guarded, ..)| guarded == base) {
            if let Some(next) = super::narrowed_reference_type(&narrowed_ty, path, *guard) {
                narrowed_ty = next;
            }
        }
        if narrowed_ty != declared {
            narrowed.push((base.to_string(), narrowed_ty, declared));
        }
    }
}

/// tsc's `isMatchingReference` for an element access: its key is a literal or
/// a constant reference (`isConstantReference` — a `const`, or a parameter or
/// `let` its function never assigns), so `arr[i]` stops narrowing once `i += 1`.
fn element_key_is_constant(access: &ParsedExpression, symbols: &SymbolTable, ctx: &CheckerContext) -> bool {
    let index = match access {
        ParsedExpression::ElementAccess { index, .. }
        | ParsedExpression::IndexAccess { index, .. }
        | ParsedExpression::OptionalIndexAccess { index, .. } => index,
        _ => return false,
    };
    match index.as_ref() {
        ParsedExpression::NumberLiteral(_) | ParsedExpression::StringLiteral(_) => true,
        ParsedExpression::Identifier { name, .. } => symbols.get(name).is_some_and(|symbol| match symbol.kind {
            crate::symbols::SymbolKind::Const | crate::symbols::SymbolKind::ForInNumericKey => true,
            crate::symbols::SymbolKind::Parameter | crate::symbols::SymbolKind::Let => {
                !ctx.container_assigned_bindings.contains(name.as_str())
            }
            _ => false,
        }),
        _ => false,
    }
}

fn root_element_accesses(expression: &mut ParsedExpression, accesses: &mut Vec<(String, ParsedExpression)>) {
    match expression {
        ParsedExpression::ElementAccess { .. }
        | ParsedExpression::IndexAccess { .. }
        | ParsedExpression::OptionalIndexAccess { .. } => {
            let Some(key) = element_access_parts(expression) else {
                return;
            };
            if !accesses.iter().any(|(existing, _)| *existing == key) {
                accesses.push((key.clone(), expression.clone()));
            }
            *expression = ParsedExpression::Identifier { name: key, span: None };
        }
        ParsedExpression::Unary { operand, .. } => root_element_accesses(operand, accesses),
        ParsedExpression::Binary { left, right, .. } | ParsedExpression::Logical { left, right, .. } => {
            root_element_accesses(left, accesses);
            root_element_accesses(right, accesses);
        }
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. }
        | ParsedExpression::NonNullAssertion { expression: object, .. } => root_element_accesses(object, accesses),
        _ => {}
    }
}

/// `isIdentifier(node.arguments[0])`: a type predicate over an element access
/// narrows that access, as it would a binding.
fn narrowed_element_predicate_references(
    condition: &ParsedExpression,
    branch_is_true: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
    narrowed: &mut Vec<(String, Type, Type)>,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => narrowed_element_predicate_references(operand, !branch_is_true, symbols, ctx, narrowed),
        ParsedExpression::Logical {
            left,
            operator,
            right,
            ..
        } => {
            if matches!(
                (operator, branch_is_true),
                (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
            ) {
                narrowed_element_predicate_references(left, branch_is_true, symbols, ctx, narrowed);
                narrowed_element_predicate_references(right, branch_is_true, symbols, ctx, narrowed);
            }
        }
        ParsedExpression::Call { arguments, .. }
        | ParsedExpression::PropertyCall { arguments, .. } => {
            let Some(guard) = super::guards::parse_type_predicate_condition(condition, &mut |callee| {
                super::predicate::predicate_callee_signature(callee, |name| symbols.get(name), symbols, ctx)
            }) else {
                return;
            };
            if !guard.path.is_empty() || !guard.subject.ends_with(']') {
                return;
            }
            let Some(access) = arguments
                .get(guard.parameter_index)
                .map(|argument| &argument.expression)
                .filter(|expression| {
                    super::element_access_parts(expression).as_deref() == Some(guard.subject.as_str())
                })
            else {
                return;
            };
            let declared = match crate::infer::infer_expression(access, symbols, ctx) {
                crate::infer::InferredExpression::Known(ty) if !ty.is_unmodelled() => ty,
                _ => return,
            };
            let Some(target) =
                super::predicate::resolve_predicate_guard_type(&guard, Some(&declared), symbols, ctx)
            else {
                return;
            };
            let Some(narrowed_ty) = super::guards::narrow_by_predicate(&declared, &target, branch_is_true)
            else {
                return;
            };
            narrowed.push((guard.subject, narrowed_ty, declared));
        }
        _ => {}
    }
}

pub(super) fn collect_element_reference_guards<'a>(
    condition: &'a ParsedExpression,
    branch_is_true: bool,
    guards: &mut Vec<(&'a ParsedExpression, ReferenceGuard<'a>)>,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_element_reference_guards(operand, !branch_is_true, guards),
        ParsedExpression::Logical {
            left,
            operator,
            right,
            ..
        } if matches!(
            (operator, branch_is_true),
            (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
        ) =>
        {
            collect_element_reference_guards(left, branch_is_true, guards);
            collect_element_reference_guards(right, branch_is_true, guards);
        }
        ParsedExpression::ElementAccess { .. }
        | ParsedExpression::IndexAccess { .. }
        | ParsedExpression::OptionalIndexAccess { .. } => {
            if branch_is_true {
                guards.push((condition, ReferenceGuard::Truthy));
            }
        }
        _ => {
            // `typeof args[0] === 'string'`. The identifier path narrows a
            // binding's own symbol, which an element access has none of, so the
            // tag test has to reach the same per-access record the truthiness and
            // nullish guards use.
            if let Some((subject, tag, eq)) = super::guards::parse_typeof_condition(condition)
                && matches!(
                    subject,
                    ParsedExpression::ElementAccess { .. }
                        | ParsedExpression::IndexAccess { .. }
                        | ParsedExpression::OptionalIndexAccess { .. }
                )
            {
                guards.push((
                    subject,
                    ReferenceGuard::Typeof {
                        tag,
                        keep_matching: branch_is_true == eq,
                    },
                ));
                return;
            }
            if let Some((subject, eq, test)) = parse_nullish_equality_condition(condition)
                && matches!(
                    subject,
                    ParsedExpression::ElementAccess { .. }
                        | ParsedExpression::IndexAccess { .. }
                        | ParsedExpression::OptionalIndexAccess { .. }
                )
            {
                guards.push((
                    subject,
                    ReferenceGuard::Nullish {
                        keep_matching: branch_is_true == eq,
                        test,
                    },
                ));
            }
        }
    }
}
