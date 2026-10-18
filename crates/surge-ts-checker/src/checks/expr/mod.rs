// Withheld, not deleted: the rule and its fixture are correct, what is not
// good enough is surge's structural expansion of a library's generic types.
// See `tests/compat-projects/assertion-overlap-basic/README.md`.
#[allow(dead_code)]
mod accessibility;
mod assertion;
mod diagnostics;
mod evaluate;
mod guarded_unknown;
mod index_access;
mod inferred;
mod iteration;
mod lib_features;
mod operand_types;
mod operand_writes;
mod private_names;
mod unresolved;

pub(crate) use accessibility::{
    ClassIdentity, ThisParameterClassScope, base_interface, check_member_accessibility,
    class_has_static_member, constructor_accessibility_error, enclosing_class_lineage,
    restricted_member_owner, this_parameter_class,
};
pub(crate) use accessibility::{class_unspreadable_members, member_is_get_accessor};
pub(crate) use diagnostics::*;
pub(crate) use evaluate::*;
pub(crate) use guarded_unknown::downgrade_guarded_genuine_unknown;
use guarded_unknown::downgrade_predicate_guarded_genuine_unknown;
use index_access::*;
pub(crate) use index_access::{
    UnionKeyRead, constraint_element_read, object_element_read, report_global_this_element_access,
    report_invalid_deferred_index, report_unusable_index_type, union_numeric_key_read,
};
pub(crate) use index_access::well_known_symbol_key_type;
pub(crate) use inferred::*;
pub(crate) use iteration::{
    IterationTypes, IterationUse, array_pattern_element_type, generator_return_iteration_types,
    iteration_types_of_iterable,
};
pub(crate) use lib_features::{lib_feature_of_missing_member, suggested_lib_for_nonexistent_name};
pub(crate) use operand_types::{
    IterationSend, check_async_iterable_operand, check_instanceof_has_instance, check_instanceof_left_operand,
    check_instanceof_right_operand, check_iterable_operand, check_iteration_next_type,
    check_object_spread_type,
    is_definitely_not_iterable,
};
pub(crate) use operand_writes::{
    check_delete_operand, check_update_operand, report_readonly_member_write, update_result_type,
};
pub(crate) use private_names::{PrivateNameReceiver, check_private_name_access};
pub(crate) use unresolved::{
    EnclosingClassMembers, UnresolvedNameSite, cannot_find_name_message,
    export_assignment_target_is_exempt, failed_value_name_diagnostic, is_es2015_or_later_constructor_name,
    is_primitive_type_name, report_unresolved_value_name, resolves_value_name, unresolved_type_query_diagnostic,
};

use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedJsxChild, TextSpan as SyntaxTextSpan};
use surge_ts_types::{NumberLiteralType, Type, is_assignable_to, union_type};

use super::call::{
    check_call_like, check_new_like, check_optional_call_like, check_optional_property_call,
    check_property_call_like,
};
use super::function::check_arrow_function_expression;
use super::ops;
use crate::context::CheckerContext;
use crate::infer::{InferredExpression, infer_expression};
use crate::metrics::alloc_object_type;
use crate::program::{record_expression_check, record_program_timing};
use crate::spans::{choose_span, diagnostic_with_syntax_span};
use crate::symbols::SymbolTable;
use surge_ts_types::{TypeCopyReason, with_type_copy_reason};

pub(crate) fn check_expression_statement(expression: ParsedExpression, ctx: &mut CheckerContext) {
    let start = Instant::now();
    let symbols = std::mem::take(&mut ctx.symbols);
    let _ = evaluate_expression(&expression, None, &symbols, ctx);
    ctx.symbols = symbols;
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.expression_statement_checking += start.elapsed()
    });
}

/// tsc's `tryElaborateArrayLikeErrors`, which `reportErrorResults` runs on
/// every failed relation before the head is reported: a `readonly` array or
/// tuple source against a mutable array or tuple target is TS4104
/// (`The_type_0_is_readonly_and_cannot_be_assigned_to_the_mutable_type_1`), and
/// `reportRelationError` then drops its own head — TS2322 or TS2345 alike —
/// because that chain message names the same pair.
pub(crate) fn readonly_to_mutable_mismatch(source: &Type, target: &Type) -> bool {
    array_like_mutability(source, 0) == Some(false) && array_like_mutability(target, 0) == Some(true)
}

/// Whether `ty` is a mutable (`Some(true)`) or `readonly` (`Some(false)`)
/// array or tuple type, looking through the aliases that name one; `None` for
/// anything else. The lib's `Array<T>` and `ReadonlyArray<T>` written by name
/// are the same types tsc's `isArrayType` and `isReadonlyArrayType` test for.
fn array_like_mutability(ty: &Type, depth: usize) -> Option<bool> {
    if let Some((_, readonly)) = surge_ts_types::type_variable::generic_tuple(ty) {
        return Some(!readonly);
    }
    let Type::Reference(reference) = ty else {
        return matches!(ty, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)).then_some(true);
    };
    if reference.is_readonly_array() {
        return Some(false);
    }
    match reference.id.split('\u{0}').next_back() {
        Some("Array") if reference.arguments.len() == 1 => Some(true),
        Some("ReadonlyArray") if reference.arguments.len() == 1 => Some(false),
        _ if depth < 8 => array_like_mutability(&reference.resolve_arc(), depth + 1),
        _ => None,
    }
}

/// The diagnostic an assignability failure reports: TS2345 for an argument,
/// TS2322 everywhere else.
pub(crate) fn assignability_mismatch_diagnostic(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    argument_position: bool,
    file_name: impl Into<String>,
) -> surge_ts_diagnostics::Diagnostic {
    if argument_position {
        argument_not_assignable_diagnostic(source, target, source_name, target_name, file_name)
    } else {
        type_not_assignable_diagnostic(source, target, source_name, target_name, file_name)
    }
}

/// TS2345, or the report that replaces its head: TS4104 (see
/// [`readonly_to_mutable_mismatch`]), a missing-property report (see
/// [`missing_properties_report`]) or TS2379 (see
/// [`has_exact_optional_unassignable_property`]).
pub(crate) fn argument_not_assignable_diagnostic(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    file_name: impl Into<String>,
) -> surge_ts_diagnostics::Diagnostic {
    let file_name = file_name.into();
    let (source_name, target_name) =
        disambiguated_pair(source, source_name.to_string(), target, target_name.to_string(), &file_name);
    let (source_name, target_name) = (source_name.as_str(), target_name.as_str());
    if readonly_to_mutable_mismatch(source, target) {
        return surge_ts_diagnostics::Diagnostic::ts4104(source_name, target_name, file_name);
    }
    if surge_ts_types::has_no_common_properties(source, target) {
        return weak_type_diagnostic(source, target, source_name, target_name, file_name);
    }
    missing_properties_report(source, target, source_name, target_name, &file_name)
        .unwrap_or_else(|| {
            if has_exact_optional_unassignable_property(source, target) {
                crate::checks::relation_chain::with_chain(
                    surge_ts_diagnostics::Diagnostic::ts2379(source_name, target_name, file_name),
                    &crate::checks::relation_chain::chain_beneath_head(source, target),
                )
            } else {
                crate::checks::relation_chain::with_chain(
                    surge_ts_diagnostics::Diagnostic::ts2345(source_name, target_name, file_name),
                    &crate::checks::relation_chain::chain_beneath_head(source, target),
                )
            }
        })
}

/// A weak-type failure: TS2559, or TS2560 when calling the source would give
/// what the target accepts.
fn weak_type_diagnostic(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    file_name: String,
) -> surge_ts_diagnostics::Diagnostic {
    // `isRelatedTo` names a weak-type failure's source as it is, without the
    // literal generalization `reportRelationError` applies.
    let source_name = if is_unit_type(source) { source.name() } else { source_name.to_string() };
    if surge_ts_types::weak_type_source_returns_target(source, target) {
        surge_ts_diagnostics::Diagnostic::ts2560(&source_name, target_name, file_name)
    } else {
        surge_ts_diagnostics::Diagnostic::ts2559(&source_name, target_name, file_name)
    }
}

/// tsc's `reportRelationError` suppresses its own head (TS2322, TS2345) when
/// the chain beneath it is a missing-property report for the same pair, and
/// `propertiesRelatedTo` looks for a missing property before it compares any
/// property's type. So an object source that lacks a required member of an
/// object target reports TS2741 for one missing member, TS2739 for up to five
/// and TS2740 beyond, whatever else is wrong with it.
pub(crate) fn missing_properties_report(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    file_name: &str,
) -> Option<surge_ts_diagnostics::Diagnostic> {
    // relater.go `reportErrorResults` lets a missing-property message head the
    // report only when it names the same types (`chainArgsMatch`). A reference
    // displayed by its alias is elaborated as the bare reference
    // (`getNormalizedType`), and so is an interface that only extends one base
    // (`getSingleBaseForNonAugmentingSubtype`): the head stays TS2322/TS2345.
    let aliased = |ty: &Type| {
        matches!(ty, Type::Reference(reference) if reference.alias_display.is_some() || reference.non_augmenting_subtype)
    };
    if aliased(source) || aliased(target) {
        return None;
    }
    // A class's polymorphic `this` is a type parameter, related through its
    // constraint beneath the head.
    if matches!(source, Type::Reference(reference) if reference.is_polymorphic_this()) {
        return None;
    }
    let missing = missing_required_properties(source, target)?;
    let first = missing.first()?.clone();
    Some(missing_properties_diagnostic(
        &first,
        &missing,
        source_name,
        target_name,
        file_name,
    ))
}

/// The members of the global `Array` an object source lacks, in the order the
/// lib declares them (tsc lists the first four). The count beyond those follows
/// surge's own table of array members, not the configured lib's.
fn missing_array_members(source: &Type) -> Option<Vec<String>> {
    const DECLARATION_ORDER: [&str; 20] = [
        "length", "pop", "push", "concat", "join", "reverse", "shift", "slice", "sort", "splice",
        "unshift", "indexOf", "lastIndexOf", "every", "some", "forEach", "map", "filter", "reduce",
        "reduceRight",
    ];
    let Type::Object(object) = source else {
        return None;
    };
    if object.is_intersection
        || object.synthetic_open_index
        || object.non_primitive
        || object.call_signature().is_some()
        || object.construct_signature().is_some()
        || object.alias_name.as_deref() == Some("Object")
    {
        return None;
    }
    let missing: Vec<String> = DECLARATION_ORDER
        .iter()
        .chain(
            surge_ts_types::array_property_names()
                .iter()
                .filter(|name| !DECLARATION_ORDER.contains(name)),
        )
        .filter(|name| {
            !object.properties.contains_key(**name)
                && surge_ts_types::object_prototype_member_type(name).is_none()
        })
        .map(|name| name.to_string())
        .collect();
    (!missing.is_empty()).then_some(missing)
}

fn missing_required_properties(source: &Type, target: &Type) -> Option<Vec<String>> {
    // Two instantiations of one generic relate through their type arguments,
    // so any missing member is reported for an argument pair, beneath the head.
    // An `Unmeasurable` parameter's differing arguments relate structurally
    // (`mappedTypeRelatedTo`), so there the missing member is the report.
    if let (Type::Reference(source), Type::Reference(target)) = (source, target)
        && source.id == target.id
        && !(0..source.arguments.len()).any(|index| {
            matches!(
                surge_ts_types::declared_variance(source.declared_variances, index),
                Some(surge_ts_types::DeclaredVariance::Unmeasurable)
            ) && source.arguments.get(index) != target.arguments.get(index)
        })
    {
        return None;
    }
    let source = source.peeled();
    let target = target.peeled();
    if let Type::Array(_) = &target {
        return missing_array_members(&source);
    }
    if let Type::Tuple(elements) = &target { return surge_ts_types::tuple_target_missing_property(&source, elements).map(|name| vec![name]); }
    let Type::Object(target_object) = &target else {
        return None;
    };
    // A property of the `unknown` keyword takes every source, so it decides
    // nothing about the head.
    if target_object.is_intersection
        || target_object.synthetic_open_index
        || target_object
            .properties
            .values()
            .any(|property| property.ty.is_unmodelled() && !matches!(property.ty, Type::GenuineUnknown))
    {
        return None;
    }
    let has_property = |name: &str| -> bool {
        match &source {
            Type::Object(object) => {
                object.properties.contains_key(name)
                    || surge_ts_types::object_prototype_member_type(name).is_some()
                    || ((object.call_signature().is_some() || object.construct_signature().is_some())
                        && source.get_property_access_type(name).is_some())
            }
            _ => source.get_property_access_type(name).is_some(),
        }
    };
    // An intersection with a primitive operand (`number & { __brand: T }`)
    // relates through its apparent type, so tsc names the missing members
    // beneath the plain head rather than as it. One of object types alone is
    // reported like any object.
    let relates_through_apparent_type = |object: &surge_ts_types::ObjectType| {
        object.is_intersection
            && object.intersection_operands.as_deref().is_some_and(|operands| {
                operands
                    .iter()
                    .any(|operand| !matches!(operand, Type::Reference(_) | Type::Object(_)))
            })
    };
    // A function source keeps the plain head, as does the global `Object`
    // (tsc chains its "assignable to very few other types" hint beneath it).
    match &source {
        Type::Object(object)
            if !relates_through_apparent_type(object)
                && !object.synthetic_open_index
                && object.alias_name.as_deref() != Some("Object")
                && !object.non_primitive
                && (object.call_signature().is_none() && object.construct_signature().is_none()
                    || !object.properties.is_empty()) => {}
        Type::Array(_) | Type::Tuple(_) => {}
        _ => return None,
    }
    // `getUnmatchedProperties` passes over a static private name.
    let missing: Vec<&str> = target_object
        .required_properties()
        .map(|(name, _)| name.as_ref())
        .filter(|name| !surge_ts_types::private_name::is_static(name) && !has_property(name))
        .collect();
    let first = missing.first()?;
    // `reportUnmatchedProperty`: a private name the source declares under the
    // same spelling is another class's member, reported beneath the head.
    if surge_ts_types::private_name::declaring_class(first).is_some()
        && let Type::Object(source_object) = &source
        && source_object.properties.keys().any(|name| {
            surge_ts_types::private_name::is_private_name_key(name)
                && surge_ts_types::private_name::display(name) == surge_ts_types::private_name::display(first)
        })
    {
        return None;
    }
    Some(
        missing
            .into_iter()
            .map(|name| {
                target_object
                    .properties
                    .get(name)
                    .and_then(|property| property.written_name.as_deref())
                    .map_or_else(|| surge_ts_types::private_name::display(name).to_string(), String::clone)
            })
            .collect(),
    )
}

/// tsc names *every* missing required property, and picks the code by how many
/// there are: one is TS2741, two to five are listed in full as TS2739, and six
/// or more list the first four as TS2740 with the rest counted.
pub(crate) fn missing_properties_diagnostic(
    first_missing: &str,
    missing: &[String],
    source_type_name: &str,
    target_type_name: &str,
    file_name: &str,
) -> surge_ts_diagnostics::Diagnostic {
    use surge_ts_diagnostics::Diagnostic;
    const LISTED_WHEN_TRUNCATED: usize = 4;
    const MAX_LISTED: usize = 5;

    let listed = |names: &[String]| {
        names
            .iter()
            .map(|name| surge_ts_types::private_name::display(name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match missing.len() {
        0 | 1 => Diagnostic::ts2741(
            surge_ts_types::private_name::display(first_missing),
            source_type_name,
            target_type_name,
            file_name,
        ),
        count if count <= MAX_LISTED => {
            Diagnostic::ts2739(source_type_name, target_type_name, listed(missing), file_name)
        }
        count => Diagnostic::ts2740(
            source_type_name,
            target_type_name,
            listed(&missing[..LISTED_WHEN_TRUNCATED]),
            count - LISTED_WHEN_TRUNCATED,
            file_name,
        ),
    }
}

/// tsc's plain assignability message (`reportRelationError` with no head
/// message): TS2322, or TS2820 when a string literal missed a union by a typo
/// of one of its string-literal members, unless TS4104 replaces it (see
/// [`readonly_to_mutable_mismatch`]).
/// A `satisfies` mismatch (TS1360), except where the relation fails on a
/// missing property or on readonly-ness: tsgo's `reportRelationError` drops
/// any head message but a conversion's or an implementation's when the
/// elaboration it would head is a missing-property one for the same pair, and
/// any head message at all over TS4104, so that elaboration is the error.
pub(crate) fn satisfies_mismatch_diagnostic(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    file_name: impl Into<String>,
) -> surge_ts_diagnostics::Diagnostic {
    let file_name = file_name.into();
    let relation = type_not_assignable_diagnostic(source, target, source_name, target_name, file_name.clone());
    if matches!(
        relation.code,
        surge_ts_diagnostics::DiagnosticCode::TypeScript(2739 | 2740 | 2741 | 4104)
    ) {
        relation
    } else {
        surge_ts_diagnostics::Diagnostic::ts1360(source_name, target_name, file_name)
    }
}

pub(crate) fn type_not_assignable_diagnostic(
    source: &Type,
    target: &Type,
    source_name: &str,
    target_name: &str,
    file_name: impl Into<String>,
) -> surge_ts_diagnostics::Diagnostic {
    let file_name = file_name.into();
    // `getTypeNamesForErrorDisplay`: two types that print alike are named in
    // full.
    let (source_name, target_name) =
        disambiguated_pair(source, source_name.to_string(), target, target_name.to_string(), &file_name);
    let (source_name, target_name) = (source_name.as_str(), target_name.as_str());
    if readonly_to_mutable_mismatch(source, target) {
        return surge_ts_diagnostics::Diagnostic::ts4104(source_name, target_name, file_name);
    }
    if surge_ts_types::has_no_common_properties(source, target) {
        return weak_type_diagnostic(source, target, source_name, target_name, file_name);
    }
    if let Some(diagnostic) =
        missing_properties_report(source, target, source_name, target_name, &file_name)
    {
        return diagnostic;
    }
    if has_exact_optional_unassignable_property(source, target) {
        return crate::checks::relation_chain::with_chain(
            surge_ts_diagnostics::Diagnostic::ts2375(source_name, target_name, file_name),
            &crate::checks::relation_chain::chain_beneath_head(source, target),
        );
    }
    match suggested_string_literal_member(source, target) {
        Some(suggestion) => surge_ts_diagnostics::Diagnostic::ts2820(
            source_name,
            target_name,
            Type::StringLiteral(suggestion).name(),
            file_name,
        ),
        None => crate::checks::relation_chain::with_chain(
            surge_ts_diagnostics::Diagnostic::ts2322(source_name, target_name, file_name),
            &crate::checks::relation_chain::chain_beneath_head(source, target),
        ),
    }
}

/// tsc's `getExactOptionalUnassignableProperties` is not empty: under
/// `exactOptionalPropertyTypes` some optional property of `target` declared
/// without `undefined` would read a `source` value that may be `undefined`
/// (`isExactOptionalPropertyMismatch`). Two tuples never qualify.
pub(crate) fn has_exact_optional_unassignable_property(source: &Type, target: &Type) -> bool {
    if !surge_ts_types::exact_optional_property_types() {
        return false;
    }
    let target = target.peeled();
    let is_tuple = |ty: &Type| matches!(ty, Type::Tuple(_) | Type::OpenTuple(_));
    if is_tuple(&target) && is_tuple(&source.peeled()) {
        return false;
    }
    let Type::Object(target_object) = target else {
        return false;
    };
    target_object.properties.iter().any(|(name, property)| {
        is_exact_optional_slot(&property.ty, property.is_optional())
            && source
                .get_property_access_type(name)
                .is_some_and(|read| may_be_undefined(&read))
    })
}

/// Whether an optional property declared `ty` reads `ty` plus tsc's missing
/// type under `exactOptionalPropertyTypes` (`containsMissingType`): its
/// declared type holds no `undefined` of its own and does not absorb one.
pub(crate) fn is_exact_optional_slot(ty: &Type, optional: bool) -> bool {
    optional
        && surge_ts_types::exact_optional_property_types()
        && !matches!(
            ty,
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_)
        )
        && !may_be_undefined(ty)
}

/// tsc's `maybeTypeOfKind(t, TypeFlagsUndefined)`, through a few alias links.
pub(crate) fn may_be_undefined(ty: &Type) -> bool {
    fn walk(ty: &Type, depth: usize) -> bool {
        match ty {
            Type::Undefined => true,
            Type::Union(union) => union.types().iter().any(|member| walk(member, depth)),
            Type::Reference(reference) if depth < 4 && reference.enum_owner.is_none() => {
                walk(&ty.peeled(), depth + 1)
            }
            _ => false,
        }
    }
    walk(ty, 0)
}

/// `getWriteTypeOfSymbol` for an optional property under
/// `exactOptionalPropertyTypes`: its declared type, without the `undefined` a
/// read of it adds. `None` for any other member, whose write type is its read
/// type.
pub(crate) fn exact_optional_write_type(receiver: &Type, property_name: &str) -> Option<Type> {
    if !surge_ts_types::exact_optional_property_types() {
        return None;
    }
    let Type::Object(object) = receiver.peeled() else {
        return None;
    };
    let property = object.properties.get(property_name)?;
    is_exact_optional_slot(&property.ty, property.is_optional()).then(|| property.ty.clone())
}

/// tsc's `getSuggestedTypeForNonexistentStringLiteralType`.
fn suggested_string_literal_member(source: &Type, target: &Type) -> Option<String> {
    let Type::StringLiteral(value) = source else {
        return None;
    };
    let peeled;
    let union = match target {
        Type::Union(union) => union,
        Type::Reference(_) => {
            peeled = target.peeled();
            let Type::Union(union) = &peeled else {
                return None;
            };
            union
        }
        _ => return None,
    };
    let members: Vec<Type> = union
        .types()
        .iter()
        .map(|member| match member {
            Type::Reference(_) => member.peeled(),
            other => other.clone(),
        })
        .collect();
    let candidates = members.iter().filter_map(|member| match member {
        Type::StringLiteral(candidate) => Some(candidate.as_str()),
        _ => None,
    });
    crate::checks::expr::inferred::spelling_suggestion(value, candidates, 1000)
        .map(str::to_string)
}

pub(crate) fn evaluate_const_expression(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let result = evaluate_const_expression_unrecorded(expression, fallback_span, symbols, ctx);
    if let InferredExpression::Known(ty) = &result {
        crate::semantic::record_const_operand_type(expression, ty, ctx);
    }
    result
}

fn evaluate_const_expression_unrecorded(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match expression {
        ParsedExpression::ArrayLiteral { elements, .. } => {
            let mut element_types = Vec::new();
            for element in elements {
                let inferred = evaluate_const_expression(
                    &element.expression,
                    element.span.or(fallback_span),
                    symbols,
                    ctx,
                );
                element_types.push(match inferred {
                    InferredExpression::Known(ty) => ty,
                    _ => Type::Unknown,
                });
            }
            // `as const` on an array makes it `readonly [...]`, which is what
            // makes a write through an index TS2540 rather than a type error.
            let result = InferredExpression::Known(
                crate::infer::types::readonly_reference(Type::tuple(element_types)),
            );
            report_inferred_expression(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || result.clone()),
                fallback_span,
                symbols,
                ctx,
            );
            result
        }
        ParsedExpression::ObjectLiteral { properties, span } => {
            let properties = &*crate::infer::expression::resolve_computed_property_names(
                properties, symbols, ctx,
            );
            let build = |member_this: Option<&crate::infer::expression::LiteralMemberThis>, ctx: &mut CheckerContext| {
                const_object_literal_type(properties, member_this, fallback_span, symbols, ctx)
            };
            let literal = crate::infer::expression::infer_const_literal_with_this(properties, *span, ctx, |member_this, ctx| {
                build(Some(member_this), ctx)
            })
            .unwrap_or_else(|| build(None, ctx));
            let result = InferredExpression::Known(literal);
            report_inferred_expression(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || result.clone()),
                fallback_span,
                symbols,
                ctx,
            );
            result
        }
        // A template in a const context is typed by its own pattern (tsc's
        // `checkTemplateExpression`); its interpolations are still checked as
        // the expressions they are.
        ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        } if !expressions.is_empty() => {
            let evaluated = evaluate_expression(expression, fallback_span, symbols, ctx);
            match crate::infer::expression::template_expression_pattern_type(
                expressions,
                quasis,
                symbols,
                ctx,
            ) {
                Some(pattern) => InferredExpression::Known(pattern),
                None => evaluated,
            }
        }
        // Primitives just evaluate normally without widening
        _ => evaluate_expression(expression, fallback_span, symbols, ctx),
    }
}

/// The type `as const` gives an object literal: every property read-only,
/// each value in a const context. `member_this` is handed to the members
/// that take the literal's `this`.
fn const_object_literal_type(
    properties: &[surge_ts_syntax::ParsedObjectProperty],
    member_this: Option<&crate::infer::expression::LiteralMemberThis>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let mut props = surge_ts_types::PropertyMap::default();
    for property in properties {
        if let (Some(member_this), ParsedExpression::ArrowFunction(member)) = (member_this, &property.value) {
            member_this.hand_to(member, property.is_method || property.is_accessor, ctx);
        }
        let inferred = evaluate_const_expression(
            &property.value,
            property.value_span.or(fallback_span),
            symbols,
            ctx,
        );
        ctx.next_arrow_this = None;
        // `getSpreadType` merges a spread's properties, read-only in a
        // const context (`getSpreadSymbol`), after what came before.
        if property.is_spread && property.unnamed_key_value.is_none() {
            if let InferredExpression::Known(spread) = &inferred
                && let Type::Object(source) = spread.peeled()
            {
                for (name, source_property) in source.properties.iter() {
                    props.insert(name.clone(), source_property.clone().with_readonly(true));
                }
            }
            continue;
        }
        let ty = match inferred {
            InferredExpression::Known(ty) => ty,
            _ => Type::Unknown,
        };
        props.insert(
            property.name.as_str().into(),
            surge_ts_types::ObjectProperty {
                declaration: crate::infer::expression::object_literal_member_site(property, ctx),
                ty,
                optional: false,
                method: false,
                // `as const` makes every property read-only, which is
                // what turns a write to one into TS2540.
                readonly: true,
                restriction: None,
                index_slot: false,
                written_name: None,
            },
        );
    }
    Type::Object(alloc_object_type(props, None))
}
