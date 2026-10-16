//! Assertion safety (TS2352), ported from `checkAssertionDeferred` in
//! `tsc/internal/checker/checker.go`.
//!
//! `x as T` is rejected only when the two types do not overlap in *either*
//! direction — an upcast and a downcast are both legitimate, and only a
//! conversion between unrelated types is the mistake the rule is about. tsc
//! widens the source's literal types before asking, which is why `lit as "z"`
//! on a `"a" | "b"` source is accepted: the source is compared as `string`.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, TextSpan as SyntaxTextSpan};
use surge_ts_types::{FunctionType, Type, is_comparable_to};

use super::widen_type;
use crate::context::{CheckerContext, convert_span};
use crate::infer::InferredExpression;
use crate::symbols::SymbolTable;

/// `checkAssertionDeferred` asks the comparable relation in both directions: an
/// upcast relates target-to-source and a downcast source-to-target, and only a
/// conversion that relates neither way is the mistake. A type surge failed to
/// model, and `any`, relate to everything and so can never be it.
fn overlaps(source: &Type, target: &Type) -> bool {
    if source.is_unmodelled()
        || target.is_unmodelled()
        || matches!(source, Type::Any | Type::GenuineUnknown | Type::Never)
        || matches!(target, Type::Any | Type::GenuineUnknown | Type::Never)
    {
        return true;
    }
    is_comparable_to(target, source) || is_comparable_to(source, target)
}

/// `x as T` where neither type is comparable to the other. tsc anchors on the
/// whole assertion expression.
pub(crate) fn check_assertion_overlap(
    source_result: &InferredExpression,
    target: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(source), Some(span)) = (source_result, span) else {
        return;
    };

    // `getBaseTypeOfLiteralType` on the source, which is what makes a
    // literal-to-sibling-literal assertion legal.
    let widened = widen_type(source);
    if overlaps(&widened, target) {
        return;
    }

    let diagnostic = crate::checks::relation_chain::with_chain(
        Diagnostic::ts2352(widened.name(), target.name(), ctx.file_name.clone()),
        &crate::checks::relation_chain::comparable_chain_beneath_head(source, target),
    );
    ctx.push(diagnostic.with_span(convert_span(span)));
}

/// Whether surge models both sides of an assertion closely enough for the
/// comparable relation to decide it as tsc does: neither holds the degradation
/// sentinel, and neither mentions a type variable of the body being checked,
/// which relates through a constraint surge only approximates
/// (`SomeClass as (new () => T)` inside `getClass<T>`).
pub(crate) fn assertion_sides_modelled(source: &Type, target: &Type) -> bool {
    [source, target].into_iter().all(|side| {
        !crate::checks::function::type_contains_degradation(side) && !mentions_type_variable(side, 0)
    })
}

/// `type_variable::mentions_type_variable`, also through call and construct
/// signatures and open tuples. Past the depth bound it answers yes, which only
/// withholds a report.
fn mentions_type_variable(ty: &Type, depth: usize) -> bool {
    if depth > 8 {
        return true;
    }
    let nested = |ty: &Type| mentions_type_variable(ty, depth + 1);
    let signature =
        |function: &FunctionType| function.parameters().iter().any(nested) || nested(function.return_type());
    match ty {
        Type::TypeParameter(_) => true,
        Type::Array(element) => nested(element),
        Type::Tuple(elements) => elements.iter().any(nested),
        Type::OpenTuple(tuple) => {
            tuple.leading.iter().chain(&tuple.trailing).any(nested) || nested(tuple.rest.as_ref())
        }
        Type::Union(union) => union.types().iter().any(nested),
        Type::Function(function) => signature(function),
        Type::Object(object) => {
            object.properties.values().any(|property| nested(&property.ty))
                || object.string_index_type.as_deref().is_some_and(nested)
                || object.number_index_type.as_deref().is_some_and(nested)
                || object.call_signature().is_some_and(signature)
                || object.construct_signature().is_some_and(signature)
                || object
                    .intersection_operands
                    .as_deref()
                    .is_some_and(|operands| operands.iter().any(nested))
        }
        Type::Reference(reference) => reference.arguments.iter().any(nested),
        _ => false,
    }
}

/// The type tsc's flow analysis gives a binding whose members alone were
/// narrowed (`o.kind = "a"`, `o.kind === "a"` on a non-union `o`): surge
/// records such a narrowing on the binding's object, where tsc narrows only
/// the `o.kind` reference and `o` keeps its declared type. Recognised as the
/// declared object with the same name and members, only their types changed.
pub(crate) fn member_narrowing_undone(narrowed: &Type, declared: &Type) -> Option<Type> {
    let (Type::Object(narrowed_object), Type::Object(declared_object)) = (narrowed.peeled(), declared.peeled()) else {
        return None;
    };
    let same_members = narrowed_object.alias_name == declared_object.alias_name
        && narrowed_object.properties.len() == declared_object.properties.len()
        && narrowed_object
            .properties
            .keys()
            .all(|name| declared_object.properties.contains_key(name));
    (same_members && narrowed != declared).then(|| declared.clone())
}

/// An object-literal operand's own type. Evaluated against the asserted type,
/// a literal that fits answers that type, where tsc's `checkExpression` yields
/// the literal's; this reads it again without reporting, the contextual
/// evaluation having reported already.
pub(crate) fn object_literal_own_type(
    literal: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let checkpoint = ctx.diagnostics().len();
    let own = crate::infer::infer_expression(literal, symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    own
}
