use super::*;

pub(crate) fn report_inferred_expression(
    inferred_expression: InferredExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    match inferred_expression {
        InferredExpression::Known(known_type) => {
            if known_type.is_unknown() {
                return;
            }
        }
        InferredExpression::UnresolvedIdentifier { name, span } => {
            // `super` outside a class body with a resolvable base is left to
            // the grammar; there is nothing to look it up in.
            if name == "super" {
                return;
            }
            report_unresolved_value_name(
                &name,
                choose_span(span, fallback_span),
                UnresolvedNameSite::Reference,
                symbols,
                ctx,
            );
        }
        InferredExpression::MissingProperty {
            property_name,
            object_type,
            span,
        } => {
            let diagnostic = match global_this_missing_member(&property_name, &object_type, ctx) {
                Some(None) => return,
                Some(Some(diagnostic)) => diagnostic,
                None => missing_property_diagnostic(
                    &property_name,
                    &object_type,
                    symbols,
                    ctx,
                ),
            };
            ctx.push(diagnostic_with_syntax_span(
                diagnostic,
                choose_span(span, fallback_span),
            ));
        }
        InferredExpression::Unknown => {}
    }
}

/// The receiver of a non-optional member access, checked the way tsc's
/// `checkNonNullExpression` does: `unknown` is TS18046 named and TS2571
/// otherwise, and a receiver that can be `undefined` is TS18048 named and
/// TS2532 otherwise. The access itself proceeds on the non-`undefined` part, so
/// nothing cascades.
pub(crate) fn check_property_receiver(
    object: &ParsedExpression,
    receiver: &InferredExpression,
    object_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let InferredExpression::Known(object_type) = receiver else {
        return false;
    };
    if *object_type == Type::GenuineUnknown {
        report_unknown_operand(object, choose_span(object_span, fallback_span), ctx);
        return false;
    }
    maybe_emit_possibly_undefined_receiver(
        object,
        object_type,
        object_span,
        fallback_span,
        symbols,
        ctx,
    )
}

/// Reports a possibly-`undefined` receiver and returns the type the access
/// continues on.
pub(crate) fn strip_reported_undefined_receiver(
    object: &ParsedExpression,
    object_type: Type,
    object_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    if !maybe_emit_possibly_undefined_receiver(
        object,
        &object_type,
        object_span,
        fallback_span,
        symbols,
        ctx,
    ) {
        return object_type;
    }
    // `checkNonNullType`: nothing left once the nullish part is removed is
    // the error type.
    match object_type.peeled() {
        Type::Undefined | Type::Null => Type::ErrorType,
        _ => surge_ts_types::remove_nullish(&object_type),
    }
}

/// Reports a possibly-`undefined` operand, the way tsc's `checkNonNullType`
/// does, and says whether it reported. Shared with the `++`/`--` operand rule.
pub(crate) fn maybe_emit_possibly_undefined_receiver(
    object: &ParsedExpression,
    object_type: &Type,
    object_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    // A later link of an optional chain carries the chain's short-circuit
    // `undefined`, which is not the receiver's own: `b?.value.x` is fine, while
    // `b?.inner.x` with an optional `inner` is still an error. tsc keeps the
    // two apart with a marker; surge re-derives the link without it.
    // A `null` keyword names a value, like `undefined` below: TS18050.
    if matches!(object, ParsedExpression::NullLiteral) {
        push_nullish_operand_diagnostic(
            object,
            (true, false),
            choose_span(object_span, fallback_span),
            ctx,
        );
        return true;
    }
    let receiver_type = if object.continues_optional_chain() {
        let Some(without_marker) = chain_link_type_without_marker(object, symbols, ctx) else {
            return false;
        };
        without_marker
    } else {
        object_type.clone()
    };
    let Some((null, undefined)) =
        receiver_nullability(&receiver_type).or_else(|| written_constraint_nullability(&receiver_type, ctx))
    else {
        return false;
    };
    push_nullish_operand_diagnostic(
        object,
        (null, undefined),
        choose_span(object_span, fallback_span),
        ctx,
    );
    true
}

/// `getTypeFacts` of a type variable whose base constraint surge could not
/// resolve (one naming the variable itself, `T extends Record<keyof T, string>
/// | undefined`): the `null` and `undefined` its written constraint unions in.
fn written_constraint_nullability(ty: &Type, ctx: &CheckerContext) -> Option<(bool, bool)> {
    let peeled = ty.peeled();
    let Type::TypeParameter(parameter) = &peeled else {
        return None;
    };
    let base = surge_ts_types::type_variable::base_constraint_or_type(&peeled);
    if !base.is_unmodelled() && !matches!(base, Type::TypeParameter(_)) {
        return None;
    }
    let surge_ts_syntax::ParsedType::Union(members) = ctx.type_parameter_constraint(&parameter.name)? else {
        return None;
    };
    let null = members.iter().any(|member| matches!(member, surge_ts_syntax::ParsedType::Null));
    let undefined = members.iter().any(|member| matches!(member, surge_ts_syntax::ParsedType::Undefined));
    (null || undefined).then_some((null, undefined))
}

/// tsc's `checkNonNullType` on an operator operand. Unlike a member receiver,
/// an optional chain's own `undefined` counts here. Returns the type the
/// operator goes on with when it reported: the non-nullish part, or `any`
/// (tsc's error type) when nothing is left.
pub(crate) fn check_non_null_operand(
    operand: &ParsedExpression,
    operand_type: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if *operand_type == Type::GenuineUnknown {
        return report_unknown_operand(operand, span, ctx).then_some(Type::Any);
    }
    if matches!(operand, ParsedExpression::NullLiteral) {
        push_nullish_operand_diagnostic(operand, (true, false), span, ctx);
        return Some(Type::Any);
    }
    let nullability = receiver_nullability(operand_type)?;
    push_nullish_operand_diagnostic(operand, nullability, span, ctx);
    Some(match operand_type.peeled() {
        Type::Undefined | Type::Null => Type::Any,
        _ => surge_ts_types::remove_nullish(operand_type),
    })
}

/// The `unknown` branch of `checkNonNullType`: under `strictNullChecks` an
/// `unknown` operand is TS18046 named and TS2571 otherwise, and goes on as the
/// error type. Without it `unknown` is an ordinary type the operation then
/// rejects in its own terms. Returns whether it reported.
pub(crate) fn report_unknown_operand(
    operand: &ParsedExpression,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if !surge_ts_types::strict_null_checks() {
        return false;
    }
    let file_name = ctx.file_name.clone();
    let (diagnostic, span) = match span.and_then(|span| ctx.parenthesized_outer_span(span)) {
        Some(outer) => (Diagnostic::ts2571(file_name), Some(outer)),
        None => match nameable_receiver(operand) {
            Some(name) => (Diagnostic::ts18046(name, file_name), span),
            None => (Diagnostic::ts2571(file_name), span),
        },
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
    true
}

/// `reportObjectPossiblyNullOrUndefinedError` for an operand that admits the
/// given `(null, undefined)` values. The `null` keyword and the identifier
/// `undefined` name values rather than possibly-nullish places, so they are
/// TS18050; an entity name picks TS18047/TS18048/TS18049 by which nullish
/// values it admits, and anything else TS2531/TS2532/TS2533. A parenthesized
/// operand is not an entity name to tsc, so it is unnamed, anchored at the
/// parentheses.
fn push_nullish_operand_diagnostic(
    operand: &ParsedExpression,
    (null, undefined): (bool, bool),
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let file_name = ctx.file_name.clone();
    let unnamed = |file_name: String| match (null, undefined) {
        (true, true) => Diagnostic::ts2533(file_name),
        (true, false) => Diagnostic::ts2531(file_name),
        _ => Diagnostic::ts2532(file_name),
    };
    if let Some(outer) = span.and_then(|span| ctx.parenthesized_outer_span(span)) {
        ctx.push(diagnostic_with_syntax_span(unnamed(file_name), Some(outer)));
        return;
    }
    let diagnostic = match operand {
        ParsedExpression::NullLiteral => Diagnostic::ts18050("null", file_name),
        ParsedExpression::UndefinedLiteral => Diagnostic::ts18050("undefined", file_name),
        _ => match (nameable_receiver(operand), null, undefined) {
            (Some(name), true, true) => Diagnostic::ts18049(name, file_name),
            (Some(name), true, false) => Diagnostic::ts18047(name, file_name),
            (Some(name), _, _) => Diagnostic::ts18048(name, file_name),
            (None, _, _) => unnamed(file_name),
        },
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}

/// The type of an optional-chain link with the chain's own `undefined` left
/// out: the last property read applied to the link's receiver with `undefined`
/// removed. `None` for a link surge cannot re-derive (a call, an element
/// access), which then reports nothing.
fn chain_link_type_without_marker(
    link: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    match link {
        ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => {
            let InferredExpression::Known(base) = infer_expression(object, symbols, ctx) else {
                return None;
            };
            property_type_on(&surge_ts_types::remove_nullish(&base), property_name)
        }
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } => {
            let base = chain_link_type_without_marker(object, symbols, ctx)?;
            property_type_on(&surge_ts_types::remove_nullish(&base), property_name)
        }
        ParsedExpression::NonNullAssertion { expression, .. } => {
            chain_link_type_without_marker(expression, symbols, ctx)
                .map(|ty| surge_ts_types::remove_nullish(&ty))
        }
        _ => None,
    }
}

/// `base.name` read on a type, distributing over a union; `None` when any
/// member lacks the property or the base is unmodelled.
fn property_type_on(base: &Type, name: &str) -> Option<Type> {
    match base.peeled() {
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => None,
        Type::Union(union) => {
            let mut members = Vec::with_capacity(union.types().len());
            for member in union.types() {
                if member.is_unknown() {
                    return None;
                }
                if *member == Type::Undefined {
                    continue;
                }
                members.push(member.get_property_access_type(name)?);
            }
            Some(surge_ts_types::union_type(members))
        }
        other => other.get_property_access_type(name),
    }
}

/// A receiver tsc rejects as possibly `undefined`. A union with a degradation
/// sentinel is left alone: part of it is unmodelled, so the `undefined` member
/// may be surge's, not the source's. `void` is not nullable to tsc — a member
/// read on it is a missing property, reported elsewhere.
/// Which nullish values a receiver admits, as `(null, undefined)`; `None` when
/// it admits neither, or when part of it is unmodelled.
pub(crate) fn receiver_nullability(ty: &Type) -> Option<(bool, bool)> {
    let peeled = ty.peeled();
    let (null, undefined) = match &peeled {
        Type::Undefined => (false, true),
        Type::Null => (true, false),
        // `getTypeFacts` of a type variable is its base constraint's.
        Type::TypeParameter(_) => {
            let base = surge_ts_types::type_variable::base_constraint_or_type(&peeled);
            if matches!(base, Type::TypeParameter(_)) || base.is_unmodelled() {
                return None;
            }
            return receiver_nullability(&base);
        }
        Type::Union(union) => {
            let members = union.types();
            if members.iter().any(Type::is_unknown) {
                return None;
            }
            (
                members.iter().any(|member| *member == Type::Null),
                members.iter().any(|member| *member == Type::Undefined),
            )
        }
        _ => return None,
    };
    (null || undefined).then_some((null, undefined))
}

/// The receiver name tsc is willing to render. `reportObjectPossiblyNullOrUndefinedError`
/// and its `unknown` sibling both drop to the unnamed diagnostic past 100 bytes
/// (`len(nodeText) < 100`), so a long dotted chain reports as TS2532/TS2571.
fn nameable_receiver(expression: &ParsedExpression) -> Option<String> {
    property_receiver_name(expression).filter(|name| name.len() < 100)
}

/// The entity name tsc renders for a receiver: an identifier or a dotted chain
/// of them (`b?.inner` renders as `b.inner`). A bracketed access is an element
/// access to tsc, so it has none.
fn property_receiver_name(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        } => {
            let mut name = property_receiver_name(object)?;
            name.push('.');
            name.push_str(surge_ts_types::private_name::display(property_name));
            Some(name)
        }
        _ => None,
    }
}

/// tsc's `getSpellingSuggestion`: the closest in-scope value name within an
/// edit distance of `floor(len * 0.4)`, skipping candidates whose length
/// differs by more than `max(2, floor(len * 0.34))`. A case-only difference
/// costs a tenth of an edit, so it beats any real edit.
pub(super) fn suggested_value_name(
    name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<String> {
    let max_length_difference = 2.max(name.len() * 34 / 100);
    let mut best_distance = (name.len() * 4 / 10) as f64 + 1.0;
    let mut best: Option<&str> = None;
    let admits = |candidate: &str| {
        candidate != name
            && candidate.len().abs_diff(name.len()) <= max_length_difference
            && (candidate.len() >= 3 || candidate.eq_ignore_ascii_case(name))
    };
    // The file's own values bound past the statement scope (a namespace, a
    // later declaration) are in scope for tsc's `resolveName` too.
    let file_values = ctx.module_value_fallback.as_deref();
    let mut candidates: Vec<&str> = symbols
        .visible_names()
        .chain(ctx.symbols.visible_names())
        .chain(
            file_values
                .into_iter()
                .flat_map(|values| values.visible_names())
                .filter(|candidate| !candidate.contains('.')),
        )
        .chain(ctx.ambient_global_symbols.visible_names())
        .map(|candidate| candidate.as_ref())
        // Globals the lib does not declare as bindings.
        .chain(["undefined", "globalThis"])
        .filter(|candidate| admits(candidate))
        .collect();
    // Symbol tables have no declaration order to offer; a name order at least
    // keeps the tie-break deterministic.
    candidates.sort_unstable();
    candidates.dedup();
    for candidate in candidates {
        let Some(distance) = levenshtein_with_max(name, candidate, best_distance - 0.1) else {
            continue;
        };
        if distance < best_distance {
            best_distance = distance;
            best = Some(candidate);
        }
    }
    best.map(str::to_string)
}

/// tsc's `getSpellingSuggestion` over an ordered candidate list. On a distance
/// tie the earlier candidate wins, which is tsc's order for union members.
/// More than `max_candidates` candidates (when non-zero) means no suggestion.
pub(crate) fn spelling_suggestion<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
    max_candidates: usize,
) -> Option<&'a str> {
    let screen = SpellingScreen::new(name);
    let mut best_distance = (screen.name_length * 4 / 10) as f64 + 0.9;
    let mut best: Option<&str> = None;
    for (index, candidate) in candidates.into_iter().enumerate() {
        if max_candidates > 0 && index >= max_candidates {
            return None;
        }
        if !screen.admits(candidate) {
            continue;
        }
        let Some(distance) = levenshtein_with_max(name, candidate, best_distance) else {
            continue;
        };
        if distance < best_distance {
            best_distance = distance;
            best = Some(candidate);
        } else if best.is_none() {
            best = Some(candidate);
        }
    }
    best
}

/// The screens [`spelling_suggestion`] applies before measuring a candidate:
/// not the name itself, a length within `max(2, floor(len * 0.34))` of it, and
/// a candidate shorter than three only as a case-insensitive match.
pub(crate) struct SpellingScreen<'a> {
    name: &'a str,
    name_length: usize,
    max_length_difference: usize,
}

impl<'a> SpellingScreen<'a> {
    pub(crate) fn new(name: &'a str) -> Self {
        let name_length = name.chars().count();
        Self {
            name,
            name_length,
            max_length_difference: 2.max(name_length * 34 / 100),
        }
    }

    pub(crate) fn admits(&self, candidate: &str) -> bool {
        !(candidate.is_empty()
            || candidate == self.name
            || candidate.len().abs_diff(self.name_length) > self.max_length_difference
            || (candidate.len() < 3 && !candidate.eq_ignore_ascii_case(self.name)))
    }
}

/// Levenshtein distance where a case-only substitution costs 0.1, abandoning a
/// row once every cell exceeds `max`.
fn levenshtein_with_max(source: &str, target: &str, max: f64) -> Option<f64> {
    // Case folding can map a non-ASCII character onto an ASCII one (the Kelvin
    // sign folds to `k`), so bytes stand in for characters only when both
    // names are ASCII.
    if source.is_ascii() && target.is_ascii() {
        return levenshtein_rows(source.as_bytes(), target.as_bytes(), max, u8::eq_ignore_ascii_case);
    }
    let source: Vec<char> = source.chars().collect();
    let target: Vec<char> = target.chars().collect();
    levenshtein_rows(&source, &target, max, |left: &char, right: &char| {
        left.to_lowercase().eq(right.to_lowercase())
    })
}

fn levenshtein_rows<T: PartialEq>(
    source: &[T],
    target: &[T],
    max: f64,
    same_ignoring_case: impl Fn(&T, &T) -> bool,
) -> Option<f64> {
    let mut previous: Vec<f64> = (0..=target.len()).map(|column| column as f64).collect();
    let mut current = vec![0.0; target.len() + 1];
    let big = max + 0.01;
    for (row, source_char) in source.iter().enumerate() {
        let row_number = row + 1;
        let min_column = ((row_number as f64 - max).ceil() as i64).max(1) as usize;
        let max_column = ((max + row_number as f64).floor() as usize).min(target.len());
        let mut column_min = row_number as f64;
        current[0] = column_min;
        for column in 1..min_column.min(target.len() + 1) {
            current[column] = big;
        }
        for column in min_column..=max_column {
            let target_char = &target[column - 1];
            let value = if source_char == target_char {
                previous[column - 1]
            } else {
                let substitution = if same_ignoring_case(source_char, target_char) {
                    previous[column - 1] + 0.1
                } else {
                    previous[column - 1] + 2.0
                };
                (previous[column] + 1.0).min((current[column - 1] + 1.0).min(substitution))
            };
            current[column] = value;
            column_min = column_min.min(value);
        }
        for column in max_column + 1..=target.len() {
            current[column] = big;
        }
        if column_min > max {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let result = previous[target.len()];
    (result <= max).then_some(result)
}

/// tsc's `checkIdentifier` for a binding it types by control flow (`autoType`,
/// `autoArrayType`). The receiver of `push`/`unshift`/`length`/`x[n] = v` is
/// `any[]` and unreported. In its own function a read sees the flow; only an
/// array with no element type yet is an implicit `any[]`. A closure sees the
/// flow only for a `let` past its last assignment, and otherwise the declared
/// type — an implicit `any` (or `any[]`) unless the `let` is never initialized,
/// which reads as `undefined`. An implicit read is TS7005, with TS7034 on the
/// declaration. `None` leaves the read to the binding's ordinary type.
pub(crate) fn check_auto_array_read(
    name: &str,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !ctx.auto_arrays_declared {
        return None;
    }
    let (binding, from_enclosing_function) = symbols.auto_array(name)?;
    // A module-level binding evolves in the module table; the scope stack a
    // statement is checked through only holds a snapshot of it.
    let binding = match binding.module_level {
        true => ctx
            .symbols
            .auto_array(name)
            .map_or(binding, |(current, _)| current),
        false => binding,
    };
    let any_array = Type::Array(Box::new(Type::Any));
    if binding.evolving && span.is_some() && ctx.evolving_array_operation_target == span {
        ctx.evolving_array_operation_target = None;
        return Some(any_array);
    }
    // Unreachable code has no flow type: tsc reads the declared type made
    // `any` there (`getTypeAtFlowNode`'s default case), which is no implicit
    // `any` to report. A nested function has a flow of its own.
    if !from_enclosing_function
        && span.is_some_and(|span| {
            ctx.unreachable_statement_runs
                .iter()
                .any(|run| run.start <= span.start && span.end <= run.end)
        })
    {
        return Some(if binding.declared_array { any_array } else { Type::Any });
    }
    let position = span.map_or(0, |span| span.start);
    // A top-level function or class declaration never continues the module's flow.
    let declared_only = binding.module_level && ctx.module_declared_only_depth > 0;
    let sees_flow =
        !declared_only && (!from_enclosing_function || binding.closure_sees_flow(position));
    let never_initialized = binding.never_initialized();
    let declared_array = binding.declared_array;
    let element_less = binding.is_element_less();
    let name_span = binding.name_span;
    let implicit = if sees_flow {
        element_less.then(|| any_array.clone())
    } else if never_initialized {
        return Some(Type::Undefined);
    } else if declared_array {
        Some(any_array.clone())
    } else {
        Some(Type::Any)
    };
    let implicit = implicit?;
    let type_name = if implicit == Type::Any {
        "any"
    } else {
        "any[]"
    };
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts7034(name, type_name, ctx.file_name.clone()),
        name_span,
    ));
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts7005(name, type_name, ctx.file_name.clone()),
        span,
    ));
    Some(implicit)
}

/// Marks `object` as the receiver of an evolving-array operation, for
/// [`check_auto_array_read`] to recognize when it evaluates it.
pub(crate) fn mark_evolving_array_operation(object: &ParsedExpression, ctx: &mut CheckerContext) {
    if ctx.auto_arrays_declared
        && let ParsedExpression::Identifier {
            span: Some(span), ..
        } = object
    {
        ctx.evolving_array_operation_target = Some(*span);
    }
}
