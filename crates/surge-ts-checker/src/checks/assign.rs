use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::ParsedAssignment;
use surge_ts_types::is_assignable_to;

use super::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type_anchored};
use crate::context::{CheckerContext, convert_span};
use crate::program::{
    DtsExpansionReason, record_assignability_check, record_program_timing,
    with_dts_expansion_reason,
};
use crate::symbols::{SymbolKind, SymbolTable};

/// tsc's `checkIdentifier` write rejection, by what the name declares: an
/// enum (TS2628), a class (TS2629), a function (TS2630), or a `const`
/// (TS2588). Enum and class values are bound as `const`s whose static side
/// carries the `typeof Name` display; a class's alone can be constructed.
pub(crate) fn unwritable_binding_diagnostic(
    name: &str,
    symbol: &crate::symbols::SymbolInfo,
    ctx: &CheckerContext,
) -> Option<Diagnostic> {
    let file_name = ctx.file_name.clone();
    match symbol.kind {
        SymbolKind::Function => Some(Diagnostic::ts2630(name, file_name)),
        SymbolKind::Const => {
            if let surge_ts_types::Type::Object(object) = &symbol.ty
                && !ctx.file_const_class_names.contains(name)
                && object.alias_name.as_deref() == Some(format!("typeof {name}").as_str())
            {
                return Some(if object.construct_signature.is_some() {
                    Diagnostic::ts2629(name, file_name)
                } else {
                    Diagnostic::ts2628(name, file_name)
                });
            }
            Some(Diagnostic::ts2588(name, file_name))
        }
        _ => None,
    }
}

/// The scope a rejected write's value is checked in: tsc's `checkIdentifier`
/// answers a write it rejects (to a function, a class, an enum, a constant…)
/// with the error type, so the operator of a compound assignment (`f += 1`)
/// meets `errorType` rather than the binding's value.
pub(crate) fn rejected_write_symbols(symbols: &SymbolTable, target_name: &str, compound: bool) -> SymbolTable {
    let mut table = symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    if compound && let Some(symbol) = symbols.get(target_name) {
        let kind = symbol.kind;
        let _ = table.insert(
            target_name.to_string(),
            crate::symbols::SymbolInfo {
                ty: surge_ts_types::Type::ErrorType,
                kind,
                function_signature: None,
            },
        );
    }
    table
}

pub(crate) fn check_assignment(assignment: ParsedAssignment, ctx: &mut CheckerContext) -> bool {
    let symbols = ctx.symbols.clone();
    check_assignment_with_symbols(assignment, &symbols, false, ctx)
}

pub(crate) fn check_assignment_with_symbols(
    assignment: ParsedAssignment,
    symbols: &SymbolTable,
    shadowed_locally: bool,
    ctx: &mut CheckerContext,
) -> bool {
    check_assignment_with_symbols_inferred(assignment, symbols, shadowed_locally, ctx).0
}

pub(crate) fn check_assignment_with_symbols_inferred(
    assignment: ParsedAssignment,
    symbols: &SymbolTable,
    shadowed_locally: bool,
    ctx: &mut CheckerContext,
) -> (bool, crate::infer::InferredExpression) {
    let Some(target_span) = assignment.target_span else {
        return (false, crate::infer::InferredExpression::Unknown);
    };

    // The target resolves exactly as a read of the name does, module-scope
    // fallback included: `var as1 = (as1 = 2)` writes the `var` it declares.
    let target = match symbols.get_handle(&assignment.target_name) {
        Some(handle)
            if ctx
                .ambient_global_symbols
                .get_handle(&assignment.target_name)
                .is_some_and(|global| std::sync::Arc::ptr_eq(&handle, &global)) =>
        {
            ctx.module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get_own_shared(&assignment.target_name))
                .or(Some(handle))
        }
        Some(handle) => Some(handle),
        None => None,
    };
    let target = match target {
        Some(target) => target,
        None => {
            // `undefined` is not a variable to tsc (`checkIdentifier` finds its
            // symbol and rejects the write) — TS2539.
            if assignment.target_name == "undefined" {
                let diagnostic = Diagnostic::ts2539("undefined", ctx.file_name.clone())
                    .with_span(convert_span(target_span));
                ctx.push(diagnostic);
                return (false, crate::infer::InferredExpression::Unknown);
            }
            // tsc rejects a write by the symbol's module meaning before it
            // asks whether a variable is a constant, and the fallback binds a
            // namespace's value as a plain `const`, so the meaning goes first.
            if ctx.namespace_meaning(&assignment.target_name) == Some(true) {
                let diagnostic = Diagnostic::ts2631(&assignment.target_name, ctx.file_name.clone())
                    .with_span(convert_span(target_span));
                ctx.push(diagnostic);
                return (false, crate::infer::InferredExpression::Unknown);
            }
            match ctx
                .module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get_handle(&assignment.target_name))
            {
                Some(target) => target,
                None => {
                    crate::checks::expr::report_unresolved_value_name(
                        &assignment.target_name,
                        Some(target_span),
                        crate::checks::expr::UnresolvedNameSite::Reference,
                        symbols,
                        ctx,
                    );
                    return (false, crate::infer::InferredExpression::Unknown);
                }
            }
        }
    };

    if !shadowed_locally && ctx.is_import_binding(&assignment.target_name) {
        let diagnostic = Diagnostic::ts2632(&assignment.target_name, ctx.file_name.clone())
            .with_span(convert_span(target_span));
        ctx.push(diagnostic);
        return (false, crate::infer::InferredExpression::Unknown);
    }

    if let Some(diagnostic) =
        unwritable_binding_diagnostic(&assignment.target_name, &target, ctx)
    {
        ctx.push(diagnostic.with_span(convert_span(target_span)));
        return (false, crate::infer::InferredExpression::Unknown);
    }

    // An assignment is checked against the *declared* type, not the type flow
    // narrowing installed for the enclosing branch: inside
    // `if (v === undefined) { v = "x" }` the target is still `string | undefined`.
    let target_type = symbols
        .declared_type(&assignment.target_name)
        .unwrap_or(&target.ty)
        .clone();

    // tsc reports a mismatched write at the assignment target, elaborating into
    // the value only where it can (an object or array literal member).
    let written_target_span = assignment.written_target_span.unwrap_or(target_span);
    let inferred_value = evaluate_expression_with_expected_type_anchored(
        &assignment.value,
        assignment.value_span,
        Some(written_target_span),
        Some(&target_type),
        ExpectedTypeDiagnostic::TypeNotAssignable,
        symbols,
        ctx,
    );
    if crate::semantic::recording()
        && let crate::infer::InferredExpression::Known(value_type) = &inferred_value
    {
        let written = assignment.written_target_span.or(assignment.target_span);
        let span = written.zip(assignment.value_span).map(|(target, value)| surge_ts_syntax::TextSpan {
            start: target.start,
            end: value.end,
        });
        crate::semantic::record_span_type(span, Some(value_type), ctx);
    }

    match &inferred_value {
        crate::infer::InferredExpression::Known(inferred_value_type) => {
            let assignability_start = Instant::now();
            record_assignability_check();
            if *inferred_value_type != surge_ts_types::Type::Unknown
                && ((!type_contains_unknown(&target_type)
                    && !crate::checks::call::as_source(|| {
                        type_contains_unknown(&inferred_value_type)
                    }))
                    || definite_unit_member_mismatch(&inferred_value_type, &target_type)
                    || definite_primitive_member_mismatch(&inferred_value_type, &target_type)
                    || definite_type_variable_mismatch(&inferred_value_type, &target_type))
                && !with_dts_expansion_reason(DtsExpansionReason::Assignability, || {
                    is_assignable_to(inferred_value_type, &target_type)
                })
            {
                let reported_target = crate::checks::expr::reported_relation_target(
                    &inferred_value_type,
                    &target_type,
                );
                // tsc generalizes a literal source unless the target could hold
                // one (`reportRelationError`): `t = 1` into `string` reads `number`.
                let inferred_type_name = crate::checks::expr::source_display_name(
                    &inferred_value_type,
                    &reported_target,
                );
                let target_type_name = reported_target.name();
                let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
                    &inferred_value_type,
                    &reported_target,
                    &inferred_type_name,
                    &target_type_name,
                    false,
                    ctx.file_name.clone(),
                );

                // tsc's `elaborateDidYouMeanToCallOrConstruct`: a value that
                // would fit once called or constructed (`a = B` where `new B()`
                // is an `A`) is reported on the value, not on the target.
                let anchor = match assignment.value_span {
                    Some(value_span)
                        if called_or_constructed_fits(&inferred_value_type, &target_type) =>
                    {
                        value_span
                    }
                    _ => written_target_span,
                };
                let diagnostic = diagnostic.with_span(convert_span(anchor));
                ctx.push(diagnostic);
            }
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.assignability_checking += assignability_start.elapsed()
            });
        }
        crate::infer::InferredExpression::UnresolvedIdentifier { .. } => {}
        crate::infer::InferredExpression::MissingProperty { .. } => {}
        crate::infer::InferredExpression::Unknown => {}
    }
    (true, inferred_value)
}

/// Whether calling or constructing `source` yields something assignable to
/// `target`. `any` and `never` results prove nothing, as in tsc.
fn called_or_constructed_fits(source: &surge_ts_types::Type, target: &surge_ts_types::Type) -> bool {
    use surge_ts_types::Type;
    let fits = |signature: &surge_ts_types::FunctionType| {
        let returned = signature.return_type();
        !matches!(returned, Type::Any | Type::Never)
            && !returned.is_unknown()
            && is_assignable_to(returned, target)
    };
    match source.peeled() {
        Type::Function(function) => fits(&function),
        Type::Object(object) => {
            object.construct_signature().is_some_and(fits) || object.call_signature().is_some_and(fits)
        }
        _ => false,
    }
}

const MAX_REFERENCE_DEPTH: usize = 50;

thread_local! {
    static BOUND_TYPE_PARAMETERS: std::cell::RefCell<Vec<std::sync::Arc<str>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether `parameter` is declared by a generic signature enclosing the type
/// being walked (see [`with_signature_type_parameters`]). Such a placeholder
/// is a bound name, not a type surge failed to model.
/// A variable of the generic body being checked is bound the same way.
pub(crate) fn is_bound_type_parameter(parameter: &surge_ts_types::TypeParameterType) -> bool {
    parameter.is_active_variable()
        || BOUND_TYPE_PARAMETERS.with(|bound| bound.borrow().iter().any(|name| **name == *parameter.name))
}

/// Runs `walk` with no type parameter bound. A reference's expansion is a
/// different scope: a placeholder met inside it is not the enclosing
/// signature's parameter, and treating it as unmodelled keeps the walk
/// short-circuiting as it always has.
pub(crate) fn without_bound_type_parameters<R>(walk: impl FnOnce() -> R) -> R {
    let saved = BOUND_TYPE_PARAMETERS.with(|stack| std::mem::take(&mut *stack.borrow_mut()));
    let result = walk();
    BOUND_TYPE_PARAMETERS.with(|stack| *stack.borrow_mut() = saved);
    result
}

/// Runs `walk` with `function`'s own type parameters bound.
pub(crate) fn with_signature_type_parameters<R>(
    function: &surge_ts_types::FunctionType,
    walk: impl FnOnce() -> R,
) -> R {
    let bound = function
        .type_parameter_head()
        .map(declared_type_parameter_names)
        .unwrap_or_default();
    let pushed = bound.len();
    BOUND_TYPE_PARAMETERS.with(|stack| stack.borrow_mut().extend(bound));
    let result = walk();
    BOUND_TYPE_PARAMETERS.with(|stack| {
        let mut stack = stack.borrow_mut();
        let keep = stack.len() - pushed;
        stack.truncate(keep);
    });
    result
}

/// The names a rendered type-parameter head (`const T extends X, U = Y`)
/// declares.
fn declared_type_parameter_names(head: &str) -> Vec<std::sync::Arc<str>> {
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut segment = String::new();
    let flush = |segment: &mut String, names: &mut Vec<std::sync::Arc<str>>| {
        if let Some(name) = segment
            .split_whitespace()
            .find(|word| !matches!(*word, "const" | "in" | "out"))
        {
            names.push(name.into());
        }
        segment.clear();
    };
    for ch in head.chars() {
        match ch {
            '<' | '(' | '{' | '[' => depth += 1,
            '>' | ')' | '}' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                flush(&mut segment, &mut names);
                continue;
            }
            _ => {}
        }
        segment.push(ch);
    }
    flush(&mut segment, &mut names);
    names
}

/// Whether two object types disagree on a member both declare with a unit
/// type — `[Symbol.toStringTag]: "Int8Array"` against `"Uint8Array"`. Such a
/// mismatch is definite whatever else about the two types surge failed to
/// model, so it reports even where a degraded member elsewhere would suppress
/// the comparison: the lib's typed arrays differ in nothing else, and every
/// cross-assignment between them went unreported.
pub(crate) fn definite_unit_member_mismatch(
    source: &surge_ts_types::Type,
    target: &surge_ts_types::Type,
) -> bool {
    use surge_ts_types::Type;
    let is_unit = |ty: &Type| {
        matches!(
            ty,
            Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_)
        )
    };
    let (Type::Object(source), Type::Object(target)) = (source.peeled(), target.peeled()) else {
        return false;
    };
    target.properties.iter().any(|(name, expected)| {
        !expected.optional
            && is_unit(&expected.ty)
            && source
                .properties
                .get(name)
                .is_some_and(|actual| is_unit(&actual.ty) && actual.ty != expected.ty)
    })
}

/// Whether a source with structure of its own is related to a type variable
/// of the body being checked. Only `any`, `never`, the variable itself or a
/// type built on it is, so the verdict does not depend on how well surge
/// modelled the source's members.
pub(crate) fn definite_type_variable_mismatch(
    source: &surge_ts_types::Type,
    target: &surge_ts_types::Type,
) -> bool {
    use surge_ts_types::Type;
    target.is_type_variable()
        && match source.peeled() {
            Type::Object(object) => object.intersection_operands.is_none(),
            Type::Function(_)
            | Type::Array(_)
            | Type::Tuple(_)
            | Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => true,
            _ => false,
        }
}

/// Whether a `number`, `boolean` or `bigint` source is related to an object
/// type requiring a member no such value has. Their apparent types are small
/// closed interfaces, so the verdict does not depend on how well surge
/// modelled the rest of the target — an overload fold or a degraded member
/// elsewhere in it would otherwise suppress the comparison.
pub(crate) fn definite_primitive_member_mismatch(
    source: &surge_ts_types::Type,
    target: &surge_ts_types::Type,
) -> bool {
    use surge_ts_types::Type;
    const OBJECT_MEMBERS: &[&str] = &[
        "constructor",
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ];
    // Every member the lib's `String` declares, so a member surge's own
    // string surface does not model is never taken for one strings lack.
    const STRING_MEMBERS: &[&str] = &[
        "length", "charAt", "charCodeAt", "concat", "indexOf", "lastIndexOf", "localeCompare", "match",
        "replace", "search", "slice", "split", "substring", "toLowerCase", "toLocaleLowerCase",
        "toUpperCase", "toLocaleUpperCase", "trim", "substr", "codePointAt", "includes", "endsWith",
        "normalize", "repeat", "startsWith", "anchor", "big", "blink", "bold", "fixed", "fontcolor",
        "fontsize", "italics", "link", "small", "strike", "sub", "sup", "padStart", "padEnd", "trimEnd",
        "trimStart", "trimLeft", "trimRight", "matchAll", "replaceAll", "at", "isWellFormed",
        "toWellFormed", "[Symbol.iterator]",
    ];
    // Nor does `null`/`undefined` under strictNullChecks fit any object type,
    // whatever its members.
    if matches!(source, Type::Null | Type::Undefined) {
        return surge_ts_types::strict_null_checks()
            && matches!(target.peeled(), Type::Object(object)
                if !object.is_intersection && !object.synthetic_open_index);
    }
    let own_members: &[&str] = match source {
        Type::Number | Type::NumberLiteral(_) => &["toFixed", "toExponential", "toPrecision"],
        Type::Boolean | Type::BooleanLiteral(_) | Type::BigInt => &[],
        Type::String | Type::StringLiteral(_) => STRING_MEMBERS,
        _ => return false,
    };
    let Type::Object(target) = target.peeled() else {
        return false;
    };
    if target.synthetic_open_index {
        return false;
    }
    target.properties.iter().any(|(name, expected)| {
        !expected.optional
            && !OBJECT_MEMBERS.contains(&&**name)
            && !own_members.contains(&&**name)
            && source.get_property_access_type(name).is_none()
    })
}

pub(crate) fn type_contains_unknown(ty: &surge_ts_types::Type) -> bool {
    super::structural_walk::query(|| contains_unknown(ty))
}

fn contains_unknown(ty: &surge_ts_types::Type) -> bool {
    type ReferenceKey = (std::sync::Arc<str>, std::sync::Arc<[surge_ts_types::Type]>);
    match super::structural_walk::visit(ty) {
        super::structural_walk::Visit::Walk => {}
        super::structural_walk::Visit::Seen => return false,
        super::structural_walk::Visit::Exhausted => return true,
    }
    thread_local! {
        // References already on the walk, to break the cyclic structural graphs
        // lazy nominal references form (interface A whose member resolves to B
        // whose member resolves back to A). Re-entering one introduces no *new*
        // `unknown`, so it reports false — same guard as the return-type walker in
        // `checks::function::body`.
        static VISITING_REFERENCES: std::cell::RefCell<Vec<ReferenceKey>> =
            const { std::cell::RefCell::new(Vec::new()) };
        // Verdicts for references already walked under the current outermost
        // call. A library's reference graph is a DAG reached along many paths,
        // and re-walking each shared node made the walk exponential. A `false`
        // is kept only when no on-path assumption fed it.
        static WALKED_REFERENCES: std::cell::RefCell<surge_ts_types::fx::FxHashMap<(std::sync::Arc<str>, u64), Vec<(std::sync::Arc<[surge_ts_types::Type]>, bool)>>> =
            std::cell::RefCell::new(Default::default());
        static CYCLE_ASSUMPTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    match ty {
        surge_ts_types::Type::Unknown => true,
        // A signature's own type parameter is a bound name, not a type surge
        // failed to model.
        surge_ts_types::Type::TypeParameter(parameter) => !is_bound_type_parameter(parameter),
        // A degraded member hidden behind a lazy nominal reference must suppress
        // the comparison exactly as an inline one does: `is_assignable_to` peels
        // the reference and compares the unmodelled members structurally, so
        // without the peel here two degraded expansions of the same declaration
        // (`Server` reached through `ReturnType<typeof serve>` vs. through the
        // interface itself) mismatch and report a false TS2322.
        surge_ts_types::Type::Reference(reference) => {
            let on_path = VISITING_REFERENCES.with(|visiting| {
                visiting
                    .borrow()
                    .iter()
                    .any(|(id, arguments)| *id == reference.id && *arguments == reference.arguments)
                    // tsc's `isDeeplyNestedType`: a declaration on the path three
                    // times is an expanding recursion (`Deep<T>` reaching
                    // `Deep<T[]>`) whose arguments never repeat.
                    || visiting.borrow().iter().filter(|(id, _)| *id == reference.id).count() >= 3
            });
            if on_path {
                CYCLE_ASSUMPTIONS.with(|count| count.set(count.get() + 1));
                return false;
            }
            // A generic reference whose expansion instantiates itself with ever
            // larger arguments never revisits an on-path key. Past the bound the
            // walk answers "unmodelled", which only suppresses a comparison.
            if VISITING_REFERENCES.with(|visiting| visiting.borrow().len()) >= MAX_REFERENCE_DEPTH {
                return true;
            }
            let digest = reference
                .arguments
                .iter()
                .fold(0u64, |acc, argument| {
                    acc.rotate_left(5) ^ surge_ts_types::type_conflict_digest(argument)
                });
            let memo_key = (reference.id.clone(), digest);
            let walked = WALKED_REFERENCES.with(|walked| {
                walked.borrow().get(&memo_key).and_then(|entries| {
                    entries
                        .iter()
                        .find(|(arguments, _)| *arguments == reference.arguments)
                        .map(|(_, verdict)| *verdict)
                })
            });
            if let Some(verdict) = walked {
                return verdict;
            }
            let outermost = VISITING_REFERENCES.with(|visiting| visiting.borrow().is_empty());
            let assumptions_before = CYCLE_ASSUMPTIONS.with(std::cell::Cell::get);
            VISITING_REFERENCES.with(|visiting| {
                visiting
                    .borrow_mut()
                    .push((reference.id.clone(), reference.arguments.clone()));
            });
            let result = without_bound_type_parameters(|| contains_unknown(&reference.resolve()));
            VISITING_REFERENCES.with(|visiting| {
                visiting.borrow_mut().pop();
            });
            if outermost {
                WALKED_REFERENCES.with(|walked| walked.borrow_mut().clear());
                CYCLE_ASSUMPTIONS.with(|count| count.set(0));
            } else if result || CYCLE_ASSUMPTIONS.with(std::cell::Cell::get) == assumptions_before {
                WALKED_REFERENCES.with(|walked| {
                    walked
                        .borrow_mut()
                        .entry(memo_key)
                        .or_default()
                        .push((reference.arguments.clone(), result));
                });
            }
            result
        }
        surge_ts_types::Type::Array(element) => contains_unknown(element),
        surge_ts_types::Type::Tuple(elements) => elements.iter().any(contains_unknown),
        // A resolved generic member signature carries the sentinel where its
        // own type parameters were erased: a bound name, not a gap — the same
        // arm as the `checks::var` and `checks::function::body` walkers. Without
        // it every lib `Promise<T>` target (`then<TResult1, TResult2>`) reads as
        // unmodelled and a `this.p = promise.then(…)` write drops everything its
        // callback reports. A written shape relates such a signature whether or
        // not its type-parameter list renders.
        surge_ts_types::Type::Function(function)
            if function.type_parameter_head().is_some() || function.generic_shape().is_some() =>
        {
            false
        }
        // An overload group's payload is the permissive fold of its members,
        // which puts the sentinel wherever their parameters differ
        // (`String.match`, `Date.toLocaleString`); the members are the shape.
        surge_ts_types::Type::Function(function) if function.overloads().is_some_and(|members| !members.is_empty()) => {
            function
                .overloads()
                .unwrap_or_default()
                .iter()
                .any(|member| contains_unknown(&surge_ts_types::Type::Function(member.clone())))
        }
        surge_ts_types::Type::Function(function) => {
            !crate::checks::call::is_generic_signature(function)
                && with_signature_type_parameters(function, || {
                    function.parameters().iter().any(contains_unknown)
                        || contains_unknown(function.return_type())
                })
        }
        surge_ts_types::Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| contains_unknown(&property.ty))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(contains_unknown)
        }
        surge_ts_types::Type::Union(union) => union.types().iter().any(contains_unknown),
        _ => false,
    }
}
