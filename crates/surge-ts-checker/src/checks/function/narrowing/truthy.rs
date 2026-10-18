use std::sync::Arc;
use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator};
use surge_ts_types::{Type, TypeCopyReason, union_type, with_type_copy_reason};

use crate::checks::expr::evaluate_expression;
use crate::checks::ops;
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};
use super::{
    ReferenceGuard, collect_guard_operand_identifiers, collect_holding_guard_identifiers,
    narrow_condition_symbol_table, narrow_reference_in_scope, reference_path,
};

/// What is left of a type once it is known falsy (tsc's `TypeFacts.Falsy`): a
/// primitive keeps its type, since one of its values is falsy, `boolean` is
/// `false`, the nullish members and the falsy literals stay, and everything
/// that is always truthy goes — objects, functions, arrays and the other
/// literals. Nothing left is `never`. `any`, `unknown` and what surge could not
/// model say nothing and are returned whole, as is an enum, some member of
/// which may be `0`.
pub(super) fn keep_possibly_falsy(ty: &Type) -> Type {
    fn members_of(ty: &Type, out: &mut Vec<Type>) -> bool {
        match ty {
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_) => {
                return false;
            }
            Type::String | Type::Number | Type::BigInt | Type::Undefined | Type::Null | Type::Void => {
                out.push(ty.clone());
            }
            Type::Boolean | Type::BooleanLiteral(false) => out.push(Type::BooleanLiteral(false)),
            Type::StringLiteral(value) if value.is_empty() => out.push(ty.clone()),
            Type::NumberLiteral(literal) if literal.value == "0" => out.push(ty.clone()),
            Type::Union(union) => {
                for member in union.types() {
                    if !members_of(member, out) {
                        return false;
                    }
                }
            }
            Type::Reference(reference) if reference.enum_owner.is_some() => return false,
            Type::Reference(_) => match ty.peeled() {
                Type::Reference(_) => return false,
                peeled => return members_of(&peeled, out),
            },
            // A memberless shape (`{}`) admits `""` and `0`.
            Type::Object(_) if type_truthiness(ty).is_none() => out.push(ty.clone()),
            _ => {}
        }
        true
    }
    let mut kept = Vec::new();
    if !members_of(ty, &mut kept) {
        return ty.clone();
    }
    if kept.is_empty() {
        Type::Never
    } else {
        surge_ts_types::union_type(kept)
    }
}

/// What is left of a type once it is known truthy: tsc drops every member that
/// is definitely falsy, the literals `false`, `0` and `""` as well as the
/// nullish ones, so `false | Performance` tested with `perf &&` is a
/// `Performance`. [`surge_ts_types::remove_definitely_falsy`] leaves `null` in
/// place, so it is removed here with the rest of the nullish members.
pub(super) fn remove_definitely_falsy(ty: &Type) -> Type {
    surge_ts_types::remove_nullish(&surge_ts_types::remove_definitely_falsy(ty))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TruthyGuardTarget {
    Identifier(String),
    Property { base: String, property: String },
}

pub(crate) fn narrow_truthy_guarded_identifiers(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
) {
    let mut targets = Vec::new();
    collect_truthy_guarded_identifiers(condition, &mut targets);

    for target in targets {
        let base_name = match &target {
            TruthyGuardTarget::Identifier(name) => name,
            TruthyGuardTarget::Property { base, .. } => base,
        };

        let Some(symbol) = scopes.resolve(base_name) else {
            continue;
        };

        let narrowed = match &target {
            TruthyGuardTarget::Identifier(_) => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    remove_definitely_falsy(&symbol.ty)
                })
            }
            TruthyGuardTarget::Property { property, .. } => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    narrow_truthy_guarded_property(&symbol.ty, property)
                })
            }
        };

        if narrowed == symbol.ty {
            continue;
        }

        let _ = scopes.update_visible(
            base_name,
            SymbolInfo {
                ty: narrowed,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            },
        );
    }
}

pub(crate) fn collect_truthy_guarded_identifiers(
    condition: &ParsedExpression,
    targets: &mut Vec<TruthyGuardTarget>,
) {
    match condition {
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or,
            right,
            ..
        } => {
            collect_truthy_guarded_identifiers(left, targets);
            collect_truthy_guarded_identifiers(right, targets);
        }
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => {
            if let Some(target) = truthy_guard_target(operand) {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn truthy_guard_target(expression: &ParsedExpression) -> Option<TruthyGuardTarget> {
    match expression {
        ParsedExpression::Identifier { name, .. } => {
            Some(TruthyGuardTarget::Identifier(name.clone()))
        }
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => truthy_guard_base_identifier(object).map(|base| TruthyGuardTarget::Property {
            base,
            property: property_name.clone(),
        }),
        ParsedExpression::NonNullAssertion { expression, .. } => truthy_guard_target(expression),
        // `!!x` is `x`'s truthiness spelled out; as an `&&` operand it proves
        // the same thing (`!!q && q.isFetched()`).
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => match operand.as_ref() {
            ParsedExpression::Unary {
                operator: ParsedUnaryOperator::Not,
                operand: inner,
                ..
            } => truthy_guard_target(inner),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn truthy_guard_base_identifier(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::NonNullAssertion { expression, .. } => {
            truthy_guard_base_identifier(expression)
        }
        _ => None,
    }
}

pub(crate) fn narrow_truthy_guarded_property(ty: &Type, property: &str) -> Type {
    let narrowed_base = remove_definitely_falsy(&ty.peeled());

    match narrowed_base {
        Type::Object(mut object_type) => {
            if let Some(existing) = object_type.properties.get(property).cloned() {
                let properties = Arc::make_mut(&mut object_type.properties);
                properties.insert(
                    property.into(),
                    surge_ts_types::ObjectProperty {
                        declaration: existing.declaration.clone(),
                        ty: remove_definitely_falsy(&existing.ty),
                        optional: false,
                        method: existing.method,
                        readonly: existing.readonly,
                        restriction: existing.restriction.clone(),
                        index_slot: existing.index_slot,
                        written_name: existing.written_name.clone(),
                    },
                );
            }

            Type::Object(object_type)
        }
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| narrow_truthy_guarded_property(member, property))
                .collect(),
        ),
        _ => narrowed_base,
    }
}

/// The `||` counterpart of [`narrow_truthy_operand_symbol_table`]: `a || b` only
/// evaluates `b` when `a` is falsy, so `b` sees every guard `a`'s falsity proves.
pub(crate) fn narrow_falsy_operand_symbol_table(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<SymbolTable> {
    let structured = narrow_condition_symbol_table(operand, symbols, false);

    let mut guarded_identifiers = Vec::new();
    collect_holding_guard_identifiers(operand, false, &mut guarded_identifiers);
    if guarded_identifiers.is_empty() {
        return structured;
    }

    let base = structured.as_ref().unwrap_or(symbols);
    let mut narrowed = base.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let mut changed = structured.is_some();
    for name in &guarded_identifiers {
        let downgraded = narrowed.get(name).and_then(|symbol| {
            matches!(symbol.ty, Type::GenuineUnknown).then(|| SymbolInfo {
                ty: Type::Unknown,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            })
        });
        if let Some(downgraded) = downgraded {
            narrowed.insert(name.clone(), downgraded);
            changed = true;
        }
    }
    changed.then_some(narrowed)
}

/// Collects the identifiers/properties an `&&` chain proves truthy, so the right
/// side of `a.b && a.b > c` (and the then-branch) sees them non-nullish.
pub(super) fn collect_and_chain_truthy_targets(
    condition: &ParsedExpression,
    targets: &mut Vec<TruthyGuardTarget>,
) {
    if let ParsedExpression::Logical {
        left,
        operator: ParsedLogicalOperator::And,
        right,
        ..
    } = condition
    {
        collect_and_chain_truthy_targets(left, targets);
        collect_and_chain_truthy_targets(right, targets);
    } else if let Some(target) = truthy_guard_target(condition) {
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
}

pub(crate) fn narrow_truthy_operand_symbol_table(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<SymbolTable> {
    let structured = narrow_condition_symbol_table(operand, symbols, true);

    let mut targets = Vec::new();
    collect_and_chain_truthy_targets(operand, &mut targets);

    // Identifiers the left side guards as a whole value — `typeof x === "object"`,
    // `"p" in x`, `x instanceof C`, `Array.isArray(x)`, and a bare truthy `x`.
    // A guard on a genuinely-`unknown` value (`x: unknown`) makes tsc narrow it,
    // so a later access (`typeof x === "object" && "p" in x && x.p`) is not a
    // `TS18046`. surge does not compute the narrowed shape for `unknown`, but it
    // must at least stop treating the value as a genuine-unknown receiver — drop
    // it to the degradation sentinel so the property-access check stays silent,
    // matching tsc's no-cascade behavior.
    let mut guarded_identifiers = Vec::new();
    collect_guard_operand_identifiers(operand, &mut guarded_identifiers);
    for target in &targets {
        if let TruthyGuardTarget::Identifier(name) = target {
            if !guarded_identifiers.iter().any(|existing| existing == name) {
                guarded_identifiers.push(name.clone());
            }
        }
    }

    if targets.is_empty() && guarded_identifiers.is_empty() {
        return structured;
    }

    let base = structured.as_ref().unwrap_or(symbols);
    let mut narrowed = base.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let mut changed = structured.is_some();

    for name in &guarded_identifiers {
        let downgraded = narrowed.get(name).and_then(|symbol| {
            matches!(symbol.ty, Type::GenuineUnknown).then(|| SymbolInfo {
                ty: Type::Unknown,
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            })
        });
        if let Some(downgraded) = downgraded {
            narrowed.insert(name.clone(), downgraded);
            changed = true;
        }
    }
    for target in targets {
        let base_name = match &target {
            TruthyGuardTarget::Identifier(name) => name,
            TruthyGuardTarget::Property { base, .. } => base,
        };
        let Some(symbol) = narrowed.get(base_name) else {
            continue;
        };
        let new_ty = match &target {
            TruthyGuardTarget::Identifier(_) => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    remove_definitely_falsy(&symbol.ty)
                })
            }
            TruthyGuardTarget::Property { property, .. } => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    narrow_truthy_guarded_property(&symbol.ty, property)
                })
            }
        };
        if new_ty == symbol.ty {
            continue;
        }
        changed = true;
        let declared = symbol.ty.clone();
        let narrowed_symbol = SymbolInfo {
            ty: new_ty,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        narrowed.insert_narrowed(base_name.clone(), narrowed_symbol, declared);
    }

    changed.then_some(narrowed)
}

/// Positive truthy narrowing of a reference guard (`if (x) { … }`,
/// `if (o.p) { … }`, `if (this.p) { … }`): the true branch drops the nullish
/// members (`undefined`/`void`) the truthy test excludes, so a `T | undefined`
/// callee/value/property resolves to `T` inside the block. Only the true branch
/// narrows; the falsy complement (`""`, `0`, `false`, …) is not modelled, so the
/// false branch keeps the original type. The `!guard` unwrap in
/// [`narrow_discriminant_in_scope`] routes `if (!x)` else/fall-through here with
/// `branch_is_true` already flipped. Returns whether the condition was such a
/// reference (always handled, so equality-discriminant parsing is skipped).
///
/// A true branch first applies tsc's `optionalChainContainsReference` (in
/// `narrowTypeByTruthiness`): see [`narrow_optional_chain_receivers_non_null`].
pub(super) fn narrow_truthy_reference_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    if branch_is_true && surge_ts_types::strict_null_checks() {
        narrow_optional_chain_receivers_non_null(condition, scopes);
    }
    // `if (a?.b())` proves `a` non-nullish in the true branch: had it been
    // nullish the whole chain would short-circuit to `undefined` and the test
    // would be false. `reference_path` stops at the call, so unwrap to its
    // receiver first.
    let unwrapped_optional_call = matches!(
        condition,
        ParsedExpression::OptionalPropertyCall { .. } | ParsedExpression::OptionalCall { .. }
    );
    let condition = match condition {
        ParsedExpression::OptionalPropertyCall { object, .. }
        | ParsedExpression::OptionalCall { callee: object, .. } => object.as_ref(),
        other => other,
    };
    let Some((base, path)) = reference_path(condition) else {
        return false;
    };
    // `if (!merged.valid) throw` discriminates a union by a boolean *literal*
    // member, which the leaf-level truthy guard cannot express: it narrows the
    // property in place instead of dropping the members the test rules out.
    narrow_union_by_property_truthiness_in_scope(&base, &path, branch_is_true, scopes);
    if branch_is_true {
        narrow_reference_in_scope(&base, &path, ReferenceGuard::Truthy, scopes);
    } else if !unwrapped_optional_call {
        // `a?.b()` being falsy says nothing about `a`; a plain reference being
        // falsy leaves only what can be.
        narrow_reference_in_scope(&base, &path, ReferenceGuard::Falsy, scopes);
    }
    true
}

/// tsc's `optionalChainContainsReference`: a truthy optional chain proves every
/// reference it reads through non-nullish. The path narrowing cannot see this
/// through a union receiver: `{ error: null } | { error: E }` has no `shape` to
/// test on its first member, so `m.error?.shape` used to keep it.
fn narrow_optional_chain_receivers_non_null(condition: &ParsedExpression, scopes: &mut ScopeStack) {
    for receiver in super::reference::optional_chain_contained_references(condition) {
        super::narrow_reference_non_null_in_scope(receiver, scopes);
    }
}

/// The symbol-table form of [`narrow_union_by_property_truthiness_in_scope`],
/// for the branches of `c ? a : b`. tsc binds both branches under the flow
/// condition an `if` gets (`bindConditionalExpressionFlow`, binder.go:2319-2326),
/// so the members the test rules out are gone in each branch just as they are in
/// the statement form. Only the `if` path had this, which left
/// `r.success ? … : r.error?.issues` reading `error` off the success member too.
pub(super) fn narrow_union_by_property_truthiness_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let (base, path) = reference_path(condition)?;
    if path.is_empty() {
        return None;
    }
    let symbol = symbols.get(&base)?;
    let Type::Union(union) = symbol.ty.peeled() else {
        return None;
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| path_truthiness(member, &path) != Some(!branch_is_true))
        .cloned()
        .collect();
    if kept.is_empty() || kept.len() == union.types().len() {
        return None;
    }
    let narrowed_symbol = SymbolInfo {
        ty: union_type(kept),
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let declared = symbol.ty.clone();
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(base, narrowed_symbol, declared);
    Some(narrowed_symbols)
}

/// Whether the type at `path` inside `member` is always truthy (`Some(true)`),
/// always falsy (`Some(false)`), or undecidable (`None`). An optional segment
/// may be absent, so below it only a leaf that is itself falsy decides.
pub(crate) fn path_truthiness(member: &Type, path: &[String]) -> Option<bool> {
    let mut current = member.peeled();
    let mut optional = false;
    for segment in path {
        let Type::Object(object) = &current else {
            return None;
        };
        let property = object.properties.get(segment.as_str())?;
        optional |= property.is_optional();
        current = property.ty.peeled();
    }
    let leaf = type_truthiness(&current)?;
    (!optional || !leaf).then_some(leaf)
}

/// The type at `path` inside `member`, with an optional segment's `undefined`
/// put back.
pub(crate) fn path_leaf_type(member: &Type, path: &[String]) -> Option<Type> {
    let mut current = member.peeled();
    let mut optional = false;
    for segment in path {
        let Type::Object(object) = &current else {
            return None;
        };
        let property = object.properties.get(segment.as_str())?;
        optional |= property.is_optional();
        current = property.ty.peeled();
    }
    Some(if optional {
        union_type(vec![current, Type::Undefined])
    } else {
        current
    })
}

/// Whether the type at `path` inside `member` is always nullish (`Some(true)`),
/// never nullish (`Some(false)`), or either (`None`). An optional segment may
/// be absent, which reads as `undefined`.
pub(crate) fn path_nullishness(member: &Type, path: &[String]) -> Option<bool> {
    let mut current = member.peeled();
    let mut optional = false;
    for segment in path {
        let Type::Object(object) = &current else {
            return None;
        };
        let property = object.properties.get(segment.as_str())?;
        optional |= property.is_optional();
        current = property.ty.peeled();
    }
    let leaf = type_nullishness(&current)?;
    (!optional || leaf).then_some(leaf)
}

fn type_nullishness(ty: &Type) -> Option<bool> {
    match ty {
        Type::Undefined | Type::Null | Type::Void => Some(true),
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Function(_)
        | Type::Object(_)
        | Type::Array(_)
        | Type::Tuple(_) => Some(false),
        Type::Union(union) => {
            let mut members = union
                .types()
                .iter()
                .map(|member| type_nullishness(&member.peeled()));
            let first = members.next()??;
            members.all(|member| member == Some(first)).then_some(first)
        }
        Type::Reference(_) => type_nullishness(&ty.peeled()),
        _ => None,
    }
}

/// tsc's truthiness facts for a type: every unit type decides, a callable or a
/// shape with at least one member is always truthy, and a union decides only
/// when all of its members agree. `{}` admits `""` and `0`, so a memberless
/// object stays undecided.
pub(crate) fn type_truthiness(ty: &Type) -> Option<bool> {
    match ty {
        Type::BooleanLiteral(value) => Some(*value),
        Type::StringLiteral(value) => Some(!value.is_empty()),
        Type::NumberLiteral(literal) => Some(literal.value.parse::<f64>().ok()? != 0.0),
        Type::Undefined | Type::Null | Type::Void | Type::Never => Some(false),
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) => Some(true),
        Type::Object(object) => (!object.properties.is_empty()
            || object.call_signature().is_some()
            || object.construct_signature().is_some())
        .then_some(true),
        Type::Union(union) => {
            let mut members = union.types().iter().map(|member| type_truthiness(&member.peeled()));
            let first = members.next()??;
            members.all(|member| member == Some(first)).then_some(first)
        }
        Type::Reference(_) => type_truthiness(&ty.peeled()),
        _ => None,
    }
}

/// Drops the union members a truthiness test on `path` rules out. Leaves a
/// non-union base, and any member whose leaf is not a unit type, untouched.
pub(super) fn narrow_union_by_property_truthiness_in_scope(
    base: &str,
    path: &[String],
    branch_is_true: bool,
    scopes: &mut ScopeStack,
) {
    if path.is_empty() {
        return;
    }
    let Some(symbol) = scopes.resolve(base) else {
        return;
    };
    let peeled = symbol.ty.peeled();
    let Type::Union(union) = &peeled else {
        return;
    };
    // Decide before cloning: the overwhelmingly common case is a union the test
    // does not partition at all, and building the kept vector first paid a full
    // member clone for every one of them.
    let dropped = union
        .types()
        .iter()
        .filter(|member| path_truthiness(member, path) == Some(!branch_is_true))
        .count();
    if dropped == 0 || dropped == union.types().len() {
        return;
    }
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| path_truthiness(member, path) != Some(!branch_is_true))
        .cloned()
        .collect();
    let declared = symbol.ty.clone();
    let narrowed_symbol = SymbolInfo {
        ty: union_type(kept),
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let _ = scopes.insert_current_narrowed(base.to_string(), narrowed_symbol, declared);
}

pub(crate) fn evaluate_condition_expression_with_truthy_guards(
    expression: &ParsedExpression,
    fallback_span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let result = evaluate_guarded_condition(expression, fallback_span, symbols, ctx);
    crate::checks::expr::report_void_truthiness(&result, fallback_span, ctx);
    result
}

fn evaluate_guarded_condition(
    expression: &ParsedExpression,
    fallback_span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match expression {
        ParsedExpression::Logical {
            left,
            left_span,
            operator: ParsedLogicalOperator::Or,
            right,
            right_span,
            ..
        } => {
            let left_result =
                evaluate_guarded_condition(left, left_span.or(fallback_span), symbols, ctx);
            crate::checks::expr::report_void_truthiness(&left_result, left_span.or(fallback_span), ctx);
            // `b` in `a || b` runs only when `a` is falsy, so it also sees the
            // guards that falsity proves.
            let falsy = narrow_falsy_operand_symbol_table(left, symbols);
            let narrowed_symbols =
                narrow_truthy_guarded_symbol_table(left, falsy.as_ref().unwrap_or(symbols));
            // ...and the predicates it proves false (`!isNode(n) || isBar(n)`
            // reads `n` as a `Node`), as any other `||` right operand does.
            let narrowed_symbols =
                crate::checks::function::narrow_predicate_guards_symbol_table(left, &narrowed_symbols, false, ctx)
                    .unwrap_or(narrowed_symbols);
            let right_result = evaluate_guarded_condition(
                right,
                right_span.or(fallback_span),
                &narrowed_symbols,
                ctx,
            );
            ops::evaluate_logical_expression(ParsedLogicalOperator::Or, left_result, right_result)
        }
        _ => evaluate_expression(expression, fallback_span, symbols, ctx),
    }
}

pub(crate) fn narrow_truthy_guarded_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
) -> SymbolTable {
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let mut targets = Vec::new();
    collect_truthy_guarded_identifiers(condition, &mut targets);

    for target in targets {
        let base_name = match &target {
            TruthyGuardTarget::Identifier(name) => name,
            TruthyGuardTarget::Property { base, .. } => base,
        };

        let Some(symbol) = narrowed_symbols.get(base_name) else {
            continue;
        };

        let narrowed = match &target {
            TruthyGuardTarget::Identifier(_) => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    remove_definitely_falsy(&symbol.ty)
                })
            }
            TruthyGuardTarget::Property { property, .. } => {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                    narrow_truthy_guarded_property(&symbol.ty, property)
                })
            }
        };

        if narrowed == symbol.ty {
            continue;
        }

        let declared = symbol.ty.clone();
        let narrowed_symbol = SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        narrowed_symbols.insert_narrowed(base_name.clone(), narrowed_symbol, declared);
    }

    narrowed_symbols
}
