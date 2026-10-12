use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedAssignment, ParsedExpression, ParsedMemberAssignment, ParsedThisPropertyAssignment,
    ParsedType,
};
use surge_ts_types::{Type, TypeCopyReason, is_assignable_to, union_type, with_type_copy_reason};

use super::super::visible_symbols;
use crate::checks::assign::check_assignment_with_symbols_inferred;
use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::context::convert_span;
use crate::flow::{
    FlowCheck, FunctionFlowState, check_assignment_target_flow, mark_assignment_state,
};
use crate::infer::InferredExpression;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolKind, SymbolTable};

pub(crate) fn check_function_assignment(
    assignment: ParsedAssignment,
    statement_index: usize,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let _pattern = crate::infer::expression::enter_assignment_pattern_value(assignment.from_pattern);
    let target_name = assignment.target_name.clone();
    let compound = crate::flow::is_compound_assignment(
        &assignment.target_name,
        assignment.target_span,
        &assignment.value,
    );

    let (target_blocked, value_blocked) = if flow_state.tracked_local_count() > 0 {
        (
            check_assignment_target_flow(
                &target_name,
                flow_state,
                statement_index,
                ctx,
                assignment.target_span,
            ),
            {
                let (read, read_span) = crate::flow::assignment_value_read(&assignment);
                crate::flow::check_substituting_read_flow(
                    read,
                    read_span,
                    flow_state,
                    statement_index,
                    ctx,
                )
            },
        )
    } else {
        (FlowCheck::Clear, FlowCheck::Clear)
    };

    if !target_blocked.is_blocked() && !value_blocked.is_blocked() {
        let visible_symbols = visible_symbols(scopes);
        if assignment.from_pattern {
            crate::checks::var::check_computed_pattern_key(&assignment.value, &visible_symbols, ctx);
        }
        let shadowed_locally = scopes.declares_locally(&target_name);
        let assigns_empty_array = matches!(
            &assignment.value,
            ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty()
        );
        let value = assignment.value.clone();
        let value_span = assignment.value_span;
        // The value is checked in its target's context (tsc's
        // `getContextualTypeForAssignmentExpression`), which types a callback's
        // parameters. Object literals keep that contextual shape for flow;
        // re-inference would discard constructor arguments inferred from it.
        let diagnostics_before = ctx.diagnostics().len();
        let (checked, contextual_value) = check_assignment_with_symbols_inferred(
            assignment,
            &visible_symbols,
            shadowed_locally,
            ctx,
        );
        let assignment_valid = checked && ctx.diagnostics().len() == diagnostics_before;
        let checkpoint = ctx.diagnostics().len();
        let inferred_value = if checked {
            evaluate_expression(&value, value_span, &visible_symbols, ctx)
        } else {
            let rejected = crate::checks::assign::rejected_write_symbols(&visible_symbols, &target_name, compound);
            evaluate_expression(&value, value_span, &rejected, ctx)
        };
        if checked {
            ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        }
        let inferred_value = match (&value, contextual_value) {
            (ParsedExpression::ObjectLiteral { .. }, InferredExpression::Known(contextual))
                if assignment_valid && !contextual.is_degraded() =>
            {
                InferredExpression::Known(contextual)
            }
            _ => inferred_value,
        };
        if compound {
            widen_compound_assigned_symbol_type(&target_name, scopes);
        } else if !super::evolving_arrays::assign_evolving_array(
            &target_name,
            assigns_empty_array,
            &inferred_value,
            scopes,
            ctx,
        ) {
            update_assigned_symbol_type(&target_name, inferred_value, scopes);
        }
    }

    if !target_blocked.is_blocked()
        && flow_state.tracked_local_count() > 0
        && !compound
    {
        mark_assignment_state(&target_name, flow_state);
    }
}

/// Checks an `o.p = v` assignment against the target property's declared type
/// and narrows the target for the code that follows. Nothing is reported when
/// either side carries the degradation sentinel, and the narrowing is
/// block-scoped exactly like the identifier-assignment one above.
/// The declared (un-narrowed) type of an `a.b.c` reference: the base symbol's
/// declared type walked through the written members, so a write to
/// `spec.imported.name` after `spec.imported.name === "x"` narrowed the read
/// is still checked against the property's declaration.
fn declared_reference_type(object: &ParsedExpression, symbols: &SymbolTable) -> Option<Type> {
    match object {
        ParsedExpression::Identifier { name, .. } => symbols.declared_type(name).cloned(),
        ParsedExpression::This { .. } => symbols
            .declared_type("this")
            .cloned()
            .or_else(|| symbols.get("this").map(|symbol| symbol.ty.clone())),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } => declared_reference_type(object, symbols)?.get_property_access_type(property_name),
        _ => None,
    }
}

/// The declaration behind a narrowed object (`Identifier` with `name` read as
/// `"x"` after `=== "x"`), re-resolved so a write sees the declared member.
/// Only a non-generic named declaration can be recovered from the display name.
fn declared_object_type_by_name(object_type: &Type, ctx: &mut CheckerContext) -> Option<Type> {
    let Type::Object(object) = object_type.peeled() else {
        return None;
    };
    let name = object.alias_name.as_deref()?;
    if name.contains(['<', ' ', '|', '&', '{']) {
        return None;
    }
    // A generic declaration's instance can be named without its arguments (a
    // recursive alias's nested reference); the bare name recovers nothing.
    let generic = match ctx.lookup_type_declaration(name)? {
        crate::symbols::TypeDeclarationInfo::Alias(alias) => !alias.body.type_parameters.is_empty(),
        crate::symbols::TypeDeclarationInfo::Interface(interface) => !interface.body.type_parameters.is_empty(),
    };
    if generic {
        return None;
    }
    let declared = crate::infer::map_parsed_type(
        ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    (!declared.is_unknown()).then_some(declared)
}

/// A read narrowed to a literal (`o.kind === "a"`) still writes against the
/// declared primitive; when the declaration itself is out of reach, the
/// literal is widened to its base so the write is not held to the narrowing.
fn widen_narrowed_literals(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Union(union) => {
            union_type(union.types().iter().map(widen_narrowed_literals).collect())
        }
        other => other.clone(),
    }
}

/// tsc's `getWriteTypeOfSymbol`: a write is checked against the *setter's*
/// parameter type when an accessor pair declares two different types, while a
/// read produces the getter's. Only a non-generic declaration is answered here
/// — under type arguments the write type would have to be resolved through the
/// interface member cache, which owns that substitution.
fn accessor_write_type(
    receiver: &Type,
    property_name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let (write_ty, substitution) = {
        let (member, substitution) = declared_member(receiver, property_name, ctx)?;
        (member.write_ty.clone()?, substitution)
    };
    Some(crate::infer::types::map_parsed_type_with_substitution(
        write_ty,
        ctx,
        &substitution,
    ))
}

/// The member a write names, as its *declaration* wrote it, together with the
/// substitution its annotation resolves under — the reference's type arguments
/// bound to the declaration's own parameter names, so a generic accessor's
/// write type (`set value(next: T | string)`) resolves like any other member.
fn declared_member<'a>(
    receiver: &Type,
    property_name: &str,
    ctx: &'a CheckerContext,
) -> Option<(
    &'a surge_ts_syntax::ParsedInterfaceMember,
    crate::infer::types::TypeParameterSubstitution,
)> {
    let Type::Reference(reference) = receiver else {
        return None;
    };
    // A deferred intersection is no declaration: its id ends with an operand's.
    if reference.id.starts_with("\u{0}intersection") {
        return None;
    }
    let name = reference.id.split('\u{0}').next_back()?;
    let crate::symbols::TypeDeclarationInfo::Interface(info) = ctx.lookup_type_declaration(name)?
    else {
        return None;
    };
    let member = info
        .body
        .members
        .iter()
        .find(|member| member.name == property_name)?;

    let mut substitution = crate::infer::types::TypeParameterSubstitution::new();
    for (type_parameter, argument) in info
        .body
        .type_parameters
        .iter()
        .zip(reference.arguments.iter())
    {
        substitution.set(type_parameter.name.clone(), argument.clone(), false);
    }

    Some((member, substitution))
}

/// Reports a literal tuple index outside the tuple's fixed length (TS2493) or a
/// negative one (TS2514), and answers `undefined` as the element they name —
/// tsc reports the assigned value against that, so a write to a missing element
/// carries both diagnostics.
fn tuple_index_out_of_bounds(
    receiver: &Type,
    index_type: &Type,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Tuple(elements) = receiver.peeled() else {
        return None;
    };
    let Type::NumberLiteral(literal) = index_type else {
        return None;
    };
    let index = literal.value.parse::<i64>().ok()?;

    let diagnostic = if index < 0 {
        Diagnostic::ts2514(ctx.file_name.clone())
    } else if index as usize >= elements.len() {
        Diagnostic::ts2493(
            Type::Tuple(elements.clone()).name(),
            elements.len(),
            index,
            ctx.file_name.clone(),
        )
    } else {
        return None;
    };

    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    Some(Type::Undefined)
}

/// tsc checks a write's value whatever its target turns out to be
/// (`checkBinaryLikeExpression` checks both operands before the assignment).
/// With no target type there is no contextual type either; where the receiver
/// is one surge could not model rather than tsc's error type, what that
/// missing context would report is surge's gap and is suppressed.
fn check_value_without_target(
    assignment: &ParsedMemberAssignment,
    receiver: Option<&Type>,
    symbols: &crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    let degraded = !matches!(receiver, Some(Type::ErrorType));
    if degraded {
        ctx.degraded_expected_type_depth += 1;
    }
    let (value, value_span) = compound_operand(assignment).unwrap_or((&assignment.value, assignment.value_span));
    let _ = evaluate_expression(value, value_span, symbols, ctx);
    if degraded {
        ctx.degraded_expected_type_depth -= 1;
    }
}

/// The written operand of `o.p op= v`, which the parser carries as the value
/// `o.p op v`: the target's own read is the write the caller already
/// resolved, so only `v` is left to check.
fn compound_operand(assignment: &ParsedMemberAssignment) -> Option<(&ParsedExpression, Option<surge_ts_syntax::TextSpan>)> {
    let (left, left_span, right, right_span) = match &assignment.value {
        ParsedExpression::Binary { left, left_span, right, right_span, .. }
        | ParsedExpression::Logical { left, left_span, right, right_span, .. }
        | ParsedExpression::NullishCoalescing { left, left_span, right, right_span } => {
            (left, left_span, right, right_span)
        }
        _ => return None,
    };
    (*left_span == assignment.target_span && **left == assignment.target).then_some((right.as_ref(), *right_span))
}

/// tsc's `isReadonlySymbol` for the members surge records it for: a `readonly`
/// property and a getter with no setter. The write is reported and the
/// assignability check skipped, exactly as tsc does after `checkReferenceExpression`
/// returns the error type.
fn report_readonly_property_write(
    receiver: &Type,
    property_name: &str,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if !property_write_is_readonly(receiver, property_name, ctx) {
        return false;
    }

    let diagnostic = Diagnostic::ts2540(
        surge_ts_types::private_name::display(property_name),
        ctx.file_name.clone(),
    );
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    true
}

/// Whether writing `receiver.property_name` is rejected because the member is
/// `readonly`. Shared with the `delete` and increment/decrement operand checks,
/// which reject the same members under their own diagnostic codes.
pub(crate) fn property_write_is_readonly(
    receiver: &Type,
    property_name: &str,
    ctx: &CheckerContext,
) -> bool {
    // An intersection's synthetic member is read-only only when every
    // constituent declaring it is.
    if let Some(operands) = intersection_operands(receiver) {
        let mut declaring = operands.iter().filter(|operand| {
            matches!(operand.peeled(), Type::Object(object) if object.get_property(property_name).is_some())
        });
        let first = declaring.next();
        return first.is_some()
            && first
                .into_iter()
                .chain(declaring)
                .all(|operand| property_write_is_readonly(operand, property_name, ctx));
    }
    object_property_is_readonly(receiver, property_name)
        || readonly_array_length(receiver, property_name)
        || declared_member(receiver, property_name, ctx).is_some_and(|(member, _)| member.readonly)
}

/// `ReadonlyArray<T>` declares `readonly length`, and a readonly tuple is one.
fn readonly_array_length(receiver: &Type, property_name: &str) -> bool {
    property_name == "length"
        && matches!(receiver, Type::Reference(reference)
            if reference.is_readonly_array()
                || reference.arguments.len() == 1
                    && reference.id.split('\u{0}').next_back() == Some("ReadonlyArray"))
}

/// Whether the receiver's own object surface declares the member `readonly`.
/// A union is read-only in the member only when every constituent is, which is
/// what tsc's synthetic union property records.
fn object_property_is_readonly(receiver: &Type, property_name: &str) -> bool {
    match receiver.peeled() {
        Type::Object(object) => object
            .properties
            .get(property_name)
            .is_some_and(|property| property.readonly),
        // tsc's `createUnionOrIntersectionProperty`: a union has the member
        // when some constituent declares it and every other answers the name,
        // and it is read-only when any constituent's is — an index signature
        // standing in for it included.
        Type::Union(union) => {
            let members: Vec<&Type> = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null))
                .collect();
            members.iter().any(|member| {
                matches!(member.peeled(), Type::Object(object)
                    if object.get_property(property_name).is_some_and(|property| !property.index_slot))
            }) && members
                .iter()
                .all(|member| member.get_property_access_type(property_name).is_some())
                && members.iter().any(|member| {
                    object_property_is_readonly(member, property_name)
                        || member.writes_readonly_index(
                            Some(property_name),
                            surge_ts_types::is_numeric_key(property_name),
                        )
                })
        }
        _ => false,
    }
}

/// What a write through an index into a `readonly` array or tuple reaches: a
/// tuple's fixed element is a property, and any other index the read-only
/// index signature, named as tsc prints the receiver.
pub(crate) enum ReadonlyElementWrite {
    Property(String),
    IndexSignature(String),
}

pub(crate) fn readonly_element_write(receiver: &Type, index_type: &Type) -> Option<ReadonlyElementWrite> {
    let Type::Reference(reference) = receiver else {
        return None;
    };
    // The lib's `ReadonlyArray<T>` written by name is the same readonly array
    // as `readonly T[]`, which tsc also prints it as.
    let lib_readonly_array = reference.arguments.len() == 1
        && reference.id.split('\u{0}').next_back() == Some("ReadonlyArray");
    if !reference.is_readonly_array() && !lib_readonly_array {
        return None;
    }

    // Only the leading fixed elements of an open tuple are properties; the
    // rest of it is read through its index signature.
    let key = literal_index_key(index_type);
    match (receiver.peeled(), key) {
        (Type::Tuple(_), Some(key)) => return Some(ReadonlyElementWrite::Property(key)),
        (Type::OpenTuple(tuple), Some(key)) if tuple.leading_element(&key).is_some() => {
            return Some(ReadonlyElementWrite::Property(key));
        }
        _ => {}
    }
    let name = if lib_readonly_array {
        let element = &reference.arguments[0];
        match element {
            Type::Union(_) | Type::Function(_) => format!("readonly ({})[]", element.name()),
            _ => format!("readonly {}[]", element.name()),
        }
    } else {
        receiver.name()
    };
    Some(ReadonlyElementWrite::IndexSignature(name))
}

/// A write through an index into a `readonly` array or tuple. tsc reports the
/// index signature (TS2542) for an array-like receiver and the element itself
/// (TS2540) for a tuple, whose elements are properties.
fn report_readonly_element_write(
    receiver: &Type,
    index_type: &Type,
    element_span: Option<surge_ts_syntax::TextSpan>,
    access_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let Some(write) = readonly_element_write(receiver, index_type) else {
        return false;
    };

    // A tuple element is a property, reported on the index; an array's index
    // signature is reported on the whole access (`errorIfWritingToReadonlyIndex`).
    let (diagnostic, span) = match write {
        ReadonlyElementWrite::Property(key) => {
            (Diagnostic::ts2540(key, ctx.file_name.clone()), element_span)
        }
        ReadonlyElementWrite::IndexSignature(name) => {
            (Diagnostic::ts2542(name, ctx.file_name.clone()), access_span)
        }
    };
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    true
}

/// The type a write through `receiver[index]` is checked against: the element
/// of an array, the element a literal index selects out of a tuple, the member
/// a literal key names, or an index signature's value type. `None` leaves the
/// write unchecked — the receiver is a shape whose element type surge cannot
/// name, and reporting against a guess would be a false positive.
fn element_write_target_type(receiver: &Type, index_type: &Type) -> Option<Type> {
    match receiver {
        Type::Array(element) => {
            is_assignable_to(index_type, &Type::Number).then(|| element.as_ref().clone())
        }
        Type::Tuple(elements) => {
            if let Some(index) = crate::infer::tuple_index_value(index_type) {
                // An out-of-range index is an error in its own right (TS2493),
                // which surge does not report yet; it is not an assignability
                // failure, so the write stays unchecked rather than being held
                // to a made-up element type.
                return elements.get(index).cloned();
            }
            is_assignable_to(index_type, &Type::Number).then(|| union_type(elements.to_vec()))
        }
        Type::Object(object) => {
            if let Some(key) = literal_index_key(index_type)
                && let Some(member) = object.get_property_access_type(&key)
            {
                return Some(member);
            }
            // A declared unique symbol names the member its declaration wrote.
            if let Type::Reference(reference) = index_type
                && reference.is_unique_symbol()
                && let Some(name) = reference.unique_symbol_name()
                && let Some(member) = object
                    .get_property_access_type(&format!("[{}]", name.split('\u{0}').next().unwrap_or(name)))
            {
                return Some(member);
            }
            // A numeric key prefers the number index signature and falls back to
            // the string one; every other key can only use the string index.
            let key_is_numeric = literal_index_key(index_type)
                .map(|key| surge_ts_types::is_numeric_key(&key))
                .unwrap_or_else(|| is_assignable_to(index_type, &Type::Number));
            object.applicable_index_type(key_is_numeric).cloned()
        }
        // A nominal reference (`Array<number>`, an alias) writes like whatever
        // it names; an open tuple like the array of everything it can hold.
        Type::Reference(_) => match receiver.peeled() {
            peeled @ (Type::Array(_) | Type::Tuple(_) | Type::Object(_)) => {
                element_write_target_type(&peeled, index_type)
            }
            _ => None,
        },
        Type::OpenTuple(tuple) => {
            is_assignable_to(index_type, &Type::Number).then(|| tuple.element_union())
        }
        // A union receiver writes against the union of its members' write
        // types — tsc's synthetic union property defers to the union of its
        // write constituents (`getWriteTypeOfSymbolWithDeferredType`). Every
        // member has to answer: one that does not is a missing property, which
        // this path does not report.
        Type::Union(union) => {
            let mut members = Vec::with_capacity(union.types().len());
            for member in union.types() {
                members.push(element_write_target_type(member, index_type)?);
            }
            Some(union_type(members))
        }
        Type::Never => Some(Type::Never),
        _ => None,
    }
}

/// A property written through a union receiver whose members declare it as an
/// accessor: tsc's synthetic union property writes against the union of its
/// constituents' write types (`createUnionOrIntersectionProperty`), a setter's
/// parameter type where a member has one.
fn union_accessor_write_type(
    receiver: &Type,
    property_name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Union(union) = receiver else {
        return None;
    };
    let mut has_accessor = false;
    let mut targets = Vec::with_capacity(union.types().len());
    for member in union.types() {
        match accessor_write_type(member, property_name, ctx) {
            Some(target) => {
                has_accessor = true;
                targets.push(target);
            }
            None => targets.push(member.get_property_access_type(property_name)?),
        }
    }
    has_accessor.then(|| union_type(targets))
}

/// The operands of an intersection, deferred (a reference over them) or merged.
fn intersection_operands(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Reference(reference) if reference.id.starts_with("\u{0}intersection") => Some(reference.arguments.to_vec()),
        _ => match ty.peeled() {
            Type::Object(object) if object.is_intersection => Some(object.intersection_operands.as_deref()?.to_vec()),
            _ => None,
        },
    }
}

/// [`union_accessor_write_type`] for an intersection receiver: its synthetic
/// property writes against the intersection of the write types of the
/// constituents that declare it.
fn intersection_accessor_write_type(
    receiver: &Type,
    property_name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let operands = intersection_operands(receiver)?;
    let mut has_accessor = false;
    let mut targets = Vec::with_capacity(operands.len());
    for operand in &operands {
        if let Some(target) = accessor_write_type(operand, property_name, ctx) {
            has_accessor = true;
            targets.push(target);
        } else if let Type::Object(constituent) = operand.peeled()
            && constituent.get_property(property_name).is_some()
        {
            targets.push(constituent.get_property_access_type(property_name)?);
        }
    }
    (has_accessor && !targets.iter().any(crate::checks::call::type_contains_unknown))
        .then(|| crate::infer::types::merge_intersection_members(targets))
}

/// A literal key written through a union receiver: tsc's synthetic union
/// property writes against the union of its constituents' write types, an
/// accessor's setter type included (`createUnionOrIntersectionProperty`).
fn union_receiver_write_target_type(
    receiver: &Type,
    index_type: &Type,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Union(union) = receiver else {
        return None;
    };
    let key = literal_index_key(index_type)?;
    let mut targets = Vec::with_capacity(union.types().len());
    for member in union.types() {
        let target = accessor_write_type(member, &key, ctx)
            .or_else(|| element_write_target_type(member, index_type))?;
        targets.push(target);
    }
    Some(union_type(targets))
}

/// A key that is itself a union (`o[k]` with `k: "a" | "b"`) writes against the
/// *intersection* of what each key names — tsc's
/// `getIndexedAccessTypeOrUndefined` intersects the constituents' types under
/// `AccessFlags.Writing`, so two members of different types leave `never` and
/// no value can be written.
fn union_index_write_target_type(
    receiver: &Type,
    index_type: &Type,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Union(union) = index_type else {
        return None;
    };

    let mut targets = Vec::with_capacity(union.types().len());
    for key_type in union.types() {
        let key = literal_index_key(key_type)?;
        let target = accessor_write_type(receiver, &key, ctx)
            .or_else(|| element_write_target_type(receiver, key_type))?;
        targets.push(target);
    }

    (!targets.is_empty()).then(|| crate::infer::types::merge_intersection_members(targets))
}

/// `checkPropertyAccessExpressionOrQualifiedName` widens the receiver of a
/// write (`getWidenedType`), so the `{}` of `(o || {}).p = v` is a plain empty
/// object there, not the fallback a read sees carrying `o`'s names as
/// `undefined`.
fn widened_write_receiver(
    object: &ParsedExpression,
    span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    let ParsedExpression::Logical {
        left,
        left_span,
        operator: surge_ts_syntax::ParsedLogicalOperator::Or,
        right,
        ..
    } = object
    else {
        return None;
    };
    if !matches!(right.as_ref(), ParsedExpression::ObjectLiteral { properties, .. } if properties.is_empty()) {
        return None;
    }
    let left_result = evaluate_expression(left, left_span.or(span), symbols, ctx);
    let InferredExpression::Known(left_type) = &left_result else {
        return Some(left_result);
    };
    if left_type.is_unmodelled() {
        return Some(InferredExpression::Unknown);
    }
    let empty = Type::Object(crate::metrics::alloc_object_type(Default::default(), None));
    Some(InferredExpression::Known(union_type(vec![crate::infer::truthy_part(left_type), empty])))
}

/// `o || {}`, whose `{}` a write widens (see [`widened_write_receiver`]).
fn is_widened_empty_fallback(object: &ParsedExpression) -> bool {
    matches!(
        object,
        ParsedExpression::Logical { operator: surge_ts_syntax::ParsedLogicalOperator::Or, right, .. }
            if matches!(right.as_ref(), ParsedExpression::ObjectLiteral { properties, .. } if properties.is_empty())
    )
}

/// The property name a literal index names. A numeric key indexes an object by
/// its string form, which is why `record[1]` reaches a string index signature.
fn literal_index_key(index_type: &Type) -> Option<String> {
    match index_type {
        Type::StringLiteral(value) => Some(value.clone()),
        Type::NumberLiteral(literal) => Some(literal.value.clone()),
        // Only a well-known symbol's key names its member as written
        // (`[Symbol.iterator]`); a declared unique symbol's members are named
        // by the expression each declaration wrote.
        Type::Reference(reference) if reference.is_unique_symbol() => reference
            .unique_symbol_name()
            .filter(|name| name.starts_with("Symbol."))
            .map(|name| format!("[{name}]")),
        _ => None,
    }
}

/// Whether every key `index_type` stands for is a literal naming a member of
/// `object`: `getIndexedAccessType` distributes a union of keys, each reading
/// the member it names (`window[action]` with `action: "resizeTo" | "resizeBy"`).
fn literal_keys_name_members(index_type: &Type, object: &surge_ts_types::ObjectType) -> bool {
    let names_member =
        |key: &Type| literal_index_key(key).and_then(|key| object.get_property_access_type(&key)).is_some();
    match index_type {
        Type::Union(union) => union.types().iter().all(names_member),
        other => names_member(other),
    }
}

/// `receiver[index] = value`. The written element resolves exactly as a read of
/// the same expression does; before this the whole statement was dropped, so
/// `arr[1] = "s"` and `tuple[0] = "s"` went unreported.
fn check_element_assignment(
    object: &ParsedExpression,
    object_span: Option<surge_ts_syntax::TextSpan>,
    index: &ParsedExpression,
    index_span: Option<surge_ts_syntax::TextSpan>,
    assignment: &ParsedMemberAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let visible_symbols = visible_symbols(scopes);

    // tsc checks the index and then the value whatever the receiver or the
    // index turned out to be (`checkElementAccessExpression`,
    // `checkBinaryLikeExpression`).
    let object_type = match widened_write_receiver(object, object_span.or(assignment.target_span), &visible_symbols, ctx)
        .unwrap_or_else(|| evaluate_expression(object, object_span.or(assignment.target_span), &visible_symbols, ctx))
    {
        InferredExpression::Known(ty) => surge_ts_types::without_polymorphic_this(ty),
        receiver => {
            let _ = evaluate_expression(
                index,
                index_span.or(assignment.target_span),
                &visible_symbols,
                ctx,
            );
            check_value_without_target(assignment, receiver.flowing_type().as_ref(), &visible_symbols, ctx);
            return;
        }
    };

    let index_type = match evaluate_expression(
        index,
        index_span.or(assignment.target_span),
        &visible_symbols,
        ctx,
    ) {
        InferredExpression::Known(ty) => ty,
        // A key that names nothing is tsc's error type, an `any`, which still
        // indexes the receiver's number or string index signature.
        InferredExpression::UnresolvedIdentifier { .. } | InferredExpression::MissingProperty { .. } => Type::Any,
        index_result => {
            check_value_without_target(assignment, index_result.flowing_type().as_ref(), &visible_symbols, ctx);
            return;
        }
    };
    if crate::checks::expr::report_unusable_index_type(
        &index_type,
        index_span.or(assignment.target_span),
        ctx,
    ) {
        check_value_without_target(assignment, None, &visible_symbols, ctx);
        return;
    }
    let index_type =
        crate::checks::expr::well_known_symbol_key_type(index, &visible_symbols).unwrap_or(index_type);

    // A write keyed by a unique symbol, or `this[k] = v` with a key of one
    // known name, resolves the member as a read of it does: one the receiver
    // neither declares nor indexes is TS7053.
    if matches!(&index_type, Type::Reference(key) if key.is_unique_symbol())
        || (matches!(object, ParsedExpression::This { .. }) && literal_index_key(&index_type).is_some())
    {
        let diagnostics_before = ctx.diagnostics().len();
        evaluate_element_write_as_read(object, object_span, index, index_span, assignment, &visible_symbols, ctx);
        if ctx.diagnostics().len() > diagnostics_before {
            check_value_without_target(assignment, None, &visible_symbols, ctx);
            return;
        }
    }

    // `errorIfWritingToReadonlyIndex` reads the index signature off the
    // receiver as flow left it. A key of any other kind (`symbol`) falls back
    // to the string index only to be reported as unusable (TS2538).
    let literal_key = literal_index_key(&index_type);
    let key_is_numeric = literal_key
        .as_deref()
        .map_or_else(|| is_assignable_to(&index_type, &Type::Number), surge_ts_types::is_numeric_key);
    let string_or_number_key =
        key_is_numeric || literal_key.is_some() || is_assignable_to(&index_type, &Type::String);
    let readonly_index_owner = (string_or_number_key
        && object_type.writes_readonly_index(literal_key.as_deref(), key_is_numeric))
        .then(|| object_type.name());

    // A write is checked against the *declared* element type, not whatever the
    // enclosing branch narrowed the receiver to.
    let declared_receiver = declared_reference_type(object, &visible_symbols);
    // A union's member is read-only when any constituent's is, so the
    // constituents flow left decide it.
    let readonly_receiver = match &declared_receiver {
        Some(declared) if !matches!(declared.peeled(), Type::Union(_)) => declared.clone(),
        _ => object_type.clone(),
    };
    let receiver_type = declared_receiver.unwrap_or(object_type);

    let property_span = index_span.or(assignment.target_span);

    // tsc sets `AccessFlags.NoIndexSignatures` for a generic receiver, so a
    // write that lands on an index signature is TS2862 instead. Two keys do
    // *not* land on one and are excluded: a literal names a member of the
    // constraint, and a generic key (`K extends keyof T`) defers to an
    // indexed-access type — `shouldDeferIndexedAccessType` returns before the
    // index-signature lookup, which is why zod's
    // `defineLazy<T, K extends keyof T>` writes without complaint.
    let open_keys = match &index_type {
        Type::Union(union) => {
            union.types().iter().all(|key| matches!(key, Type::String | Type::Number | Type::Symbol))
        }
        key => matches!(key, Type::String | Type::Number | Type::Symbol),
    };
    // `getPropertyTypeForIndexType` over the receiver's apparent type: with
    // no index signature there to land on, the key is an implicit `any`
    // (TS7053) rather than a write refused through the constraint.
    if receiver_type.is_type_variable()
        && open_keys
        && let Type::Object(apparent) =
            surge_ts_types::type_variable::base_constraint_or_type(&receiver_type).peeled()
        && !apparent.synthetic_open_index
        && apparent.string_index_type.is_none()
        && apparent.number_index_type.is_none()
        && !crate::checks::function::type_contains_degradation(&Type::Object(apparent.clone()))
    {
        if ctx.options.no_implicit_any {
            let diagnostic = Diagnostic::ts7053(index_type.name(), Type::Object(apparent).name(), ctx.file_name.clone());
            ctx.push(match crate::checks::expr::element_access_span(object_span, index_span).or(assignment.target_span) {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
        check_value_without_target(assignment, None, &visible_symbols, ctx);
        return;
    }
    if let Type::TypeParameter(type_parameter) = &receiver_type
        && open_keys
    {
        let diagnostic = Diagnostic::ts2862(type_parameter.name.clone(), ctx.file_name.clone());
        ctx.push(match object_span.or(assignment.target_span) {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
        return;
    }

    if crate::infer::expression::deferred_element_access(&receiver_type, &index_type).is_some()
        && crate::checks::expr::report_invalid_deferred_index(
            &receiver_type,
            &index_type,
            crate::checks::expr::element_access_span(object_span, index_span).or(assignment.target_span),
            ctx,
        )
    {
        check_value_without_target(assignment, None, &visible_symbols, ctx);
        return;
    }

    // `checkIndexedAccessIndexType`: a valid key written through into a
    // mapped type that adds `readonly` only reads; the write is still checked.
    if crate::infer::expression::deferred_element_access(&receiver_type, &index_type).is_some()
        && matches!(
            surge_ts_types::type_variable::mapped_type(&receiver_type),
            Some(surge_ts_types::type_variable::DeferredType::Mapped { modifiers, .. }) if modifiers.readonly > 0
        )
    {
        let diagnostic = Diagnostic::ts2542(receiver_type.name(), ctx.file_name.clone());
        ctx.push(
            match crate::checks::expr::element_access_span(object_span, index_span).or(assignment.target_span) {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            },
        );
    }

    if report_readonly_element_write(
        &receiver_type,
        &index_type,
        property_span,
        assignment.target_span.or(property_span),
        ctx,
    ) {
        return;
    }
    // `isAssignmentToReadonlyEntity`: a constructor writes its own class's
    // `readonly` members through `this`, by an element access as well.
    if let Some(key) = literal_index_key(&index_type)
        && !(matches!(object, ParsedExpression::This { .. })
            && ctx
                .constructor_writable_members
                .as_ref()
                .is_some_and(|members| members.contains(&key)))
        && report_readonly_property_write(&readonly_receiver, &key, property_span, ctx)
    {
        return;
    }

    // The write is still checked against the index signature's value type.
    if let Some(owner) = readonly_index_owner
        && crate::infer::expression::deferred_element_access(&receiver_type, &index_type).is_none()
    {
        let diagnostic = Diagnostic::ts2542(owner, ctx.file_name.clone());
        ctx.push(match assignment.target_span.or(property_span) {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }

    // A literal index outside a tuple's fixed length is an error in its own
    // right, and the element it names is `undefined` — which is what tsc then
    // reports the assigned value against, so both diagnostics appear.
    let out_of_bounds_target =
        tuple_index_out_of_bounds(&receiver_type, &index_type, property_span, ctx);

    let index_indexes_as_number = matches!(
        index,
        ParsedExpression::Identifier { name, .. }
            if visible_symbols
                .get(name)
                .is_some_and(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::ForInNumericKey))
    );

    // A key a number-only receiver cannot answer is an implicit `any`, and a
    // write through one is the same TS7015 a read of it reports.
    if !index_indexes_as_number
        && ctx.options.no_implicit_any
        && let Type::Object(object) = receiver_type.peeled()
        && object.number_index_type.is_some()
        && object
            .applicable_index_type(is_assignable_to(&index_type, &Type::Number))
            .is_none()
        && !literal_keys_name_members(&index_type, &object)
    {
        let diagnostic = Diagnostic::ts7015(ctx.file_name.clone());
        ctx.push(match property_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
        return;
    }

    // `getIndexedAccessType` on an element access: a type variable receiver
    // indexed by a key that is not generic resolves eagerly through its
    // apparent type, for writing. A key landing on an index signature is left
    // to the checks above (`AccessFlags.NoIndexSignatures`).
    let apparent_receiver = (receiver_type.is_type_variable()
        && crate::infer::expression::deferred_element_access(&receiver_type, &index_type).is_none())
    .then(|| surge_ts_types::type_variable::base_constraint_or_type(&receiver_type))
    .filter(|apparent| {
        let keys: Vec<&Type> = match &index_type {
            Type::Union(union) => union.types().iter().collect(),
            other => vec![other],
        };
        !apparent.is_type_variable()
            && !crate::checks::function::type_contains_degradation(apparent)
            && matches!(apparent.peeled(), Type::Object(ref object) if keys.iter().all(|key| {
                literal_index_key(key).is_some_and(|key| object.get_property(&key).is_some())
            }))
    });
    let write_receiver = apparent_receiver.as_ref().unwrap_or(&receiver_type);
    let target_type = out_of_bounds_target
        .or_else(|| crate::infer::expression::deferred_element_access(&receiver_type, &index_type))
        .or_else(|| {
            literal_index_key(&index_type).and_then(|key| {
                accessor_write_type(write_receiver, &key, ctx)
                    .or_else(|| intersection_accessor_write_type(write_receiver, &key, ctx))
            })
        })
        .or_else(|| union_receiver_write_target_type(write_receiver, &index_type, ctx))
        .or_else(|| union_index_write_target_type(write_receiver, &index_type, ctx))
        .or_else(|| {
            element_write_target_type(
                write_receiver,
                // A `for…in` key over a numerically-keyed object writes through
                // the numeric index signature, as it reads through it.
                if index_indexes_as_number {
                    &Type::Number
                } else {
                    &index_type
                },
            )
        });
    let Some(target_type) = target_type else {
        if crate::checks::expr::report_global_this_element_access(
            &receiver_type,
            &index_type,
            crate::checks::expr::element_access_span(object_span, index_span).or(assignment.target_span),
            ctx,
        ) {
            return;
        }
        // A key of one literal type a plain object neither declares nor
        // indexes is the implicit `any` a read of it reports (TS7053), and so
        // is a `string` or `number` key, which names no member at all. A
        // written literal key reaches the member path instead, and a function
        // or class receiver declares the member (a late-bound expando).
        let unanswered_key = match &index_type {
            Type::StringLiteral(_) | Type::NumberLiteral(_) => {
                !matches!(index, ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_))
                    && literal_index_key(&index_type)
                        .is_some_and(|key| matches!(receiver_type.peeled(), Type::Object(receiver) if receiver.get_property(&key).is_none()))
            }
            Type::String | Type::Number => true,
            _ => false,
        };
        if unanswered_key
            && let Type::Object(receiver) = receiver_type.peeled()
            && !receiver.synthetic_open_index
            && receiver.string_index_type.is_none()
            && receiver.number_index_type.is_none()
            && receiver.call_signature().is_none()
            && receiver.construct_signature().is_none()
            && !crate::checks::function::type_contains_degradation(&receiver_type)
        {
            evaluate_element_write_as_read(object, object_span, index, index_span, assignment, &visible_symbols, ctx);
        }
        // A numeric key some member of a union neither declares nor indexes
        // names no property of the union to write (TS7053).
        if ctx.options.no_implicit_any
            && let Type::Union(union) = receiver_type.peeled()
            && let Some(key) = literal_index_key(&index_type)
            && matches!(
                crate::checks::expr::union_numeric_key_read(union.types(), &key, ctx),
                crate::checks::expr::UnionKeyRead::Missing
            )
        {
            let diagnostic = Diagnostic::ts7053(index_type.name(), receiver_type.name(), ctx.file_name.clone());
            ctx.push(match crate::checks::expr::element_access_span(object_span, index_span).or(assignment.target_span) {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
        return;
    };

    let assigned = check_assigned_value(&target_type, assignment, &visible_symbols, ctx);

    // A write is what a later read of the same element sees: tsc narrows an
    // element access with a literal or const-like key to the assigned type, and
    // without it `counts[key] = (counts[key] ?? 0) + 1` left the next read of
    // `counts[key]` possibly-undefined under `noUncheckedIndexedAccess`. A
    // generic key names no property (`tryGetNameFromType`), so `x[k] = y[k]`
    // leaves `x[k]` at its declared `T[K]`.
    if let Some(assigned) = assigned
        && !surge_ts_types::type_variable::mentions_type_variable(&index_type)
        && let Some(key) = crate::checks::function::element_reference_key(object, index)
    {
        let _ = scopes.insert_current_narrowed(
            key,
            crate::symbols::SymbolInfo {
                ty: assigned,
                kind: crate::symbols::SymbolKind::Var,
                function_signature: None,
            },
            target_type,
        );
    }
}

/// Resolves the written element as a read of it does, which reports a key
/// the receiver cannot answer. A named receiver is read as `name[key]` is,
/// which resolves the key against it (`evaluate_index_access`).
fn evaluate_element_write_as_read(
    object: &ParsedExpression,
    object_span: Option<surge_ts_syntax::TextSpan>,
    index: &ParsedExpression,
    index_span: Option<surge_ts_syntax::TextSpan>,
    assignment: &ParsedMemberAssignment,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let read = match object {
        ParsedExpression::Identifier { name, .. } => ParsedExpression::IndexAccess {
            object_name: name.clone(),
            object_span,
            index: Box::new(index.clone()),
            index_span,
        },
        _ => ParsedExpression::ElementAccess {
            object: Box::new(object.clone()),
            object_span,
            index: Box::new(index.clone()),
            index_span,
        },
    };
    let _ = crate::checks::expr::with_element_write_target(|| {
        evaluate_expression(&read, assignment.target_span, symbols, ctx)
    });
}

/// The value half of a member write: evaluate it against the target type and
/// report a mismatch. Shared by the property and element paths.
fn check_assigned_value(
    target_type: &Type,
    assignment: &ParsedMemberAssignment,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // tsc reports a mismatch of the whole value on the assignment's target.
    let inferred_value = crate::checks::expected::evaluate_expression_with_expected_type_anchored(
        &assignment.value,
        assignment.value_span,
        assignment.target_span,
        Some(target_type),
        crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
        symbols,
        ctx,
    );

    let InferredExpression::Known(value_type) = inferred_value else {
        return None;
    };

    if value_type.is_unmodelled() || target_type.is_unmodelled() {
        return None;
    }

    if is_assignable_to(&value_type, target_type) {
        return Some(value_type);
    }

    let reported_target = crate::checks::expr::reported_relation_target(&value_type, &target_type);
    let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
        &value_type,
        &reported_target,
        &crate::checks::expr::source_display_name(&value_type, &reported_target),
        &reported_target.name(),
        false,
        ctx.file_name.clone(),
    );
    // tsc anchors the assignment's type error on the whole assignment, which
    // starts at its target — not on the value.
    let diagnostic = match assignment.target_span.or(assignment.value_span) {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };
    ctx.push(diagnostic);
    None
}

/// `exports.foo = …` / `module.exports.foo = …` in a CommonJS module is an
/// export *declaration* (`getAssignmentDeclarationKind`): the export's type
/// is what its writes assign, so the write never mismatches it. Once
/// `module.exports` is replaced the module is typed by the replacement and
/// such a write is an ordinary one, while `module.exports = …` always
/// declares the replacement (`bindModuleExportsAssignment`). A deeper write
/// (`exports.n.K = …`) is ordinary as well. A file with ECMAScript module
/// syntax is no CommonJS module (`setCommonJsModuleIndicator`): it has no
/// `module` of its own, and a write through one it does not declare is
/// checked as any other.
fn is_commonjs_export_declaration(
    assignment: &ParsedMemberAssignment,
    scopes: &ScopeStack,
    ctx: &CheckerContext,
) -> bool {
    surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && (assignment.declares_commonjs_export
            || (is_module_exports_reference(&assignment.target)
                && scopes.visible_symbols().get("module").is_some()))
}

/// `module.exports` / `module["exports"]`.
fn is_module_exports_reference(target: &ParsedExpression) -> bool {
    match target {
        ParsedExpression::PropertyAccess { object, property_name, .. } => {
            property_name == "exports"
                && matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "module")
        }
        ParsedExpression::IndexAccess { object_name, index, .. } => {
            object_name == "module"
                && matches!(index.as_ref(), ParsedExpression::StringLiteral(key) if key == "exports")
        }
        _ => false,
    }
}

/// tsc binds `fn.x = …` as a declaration of `x` on `fn` (an expando) when `fn`
/// is a function declaration or a `const` initialized with a function or arrow
/// expression (`getInitializerSymbol`). surge does not keep the initializer, so
/// any `const` holding a function counts.
/// In JavaScript any variable holding a function is one, and so is one
/// initialized with an empty object literal.
fn is_expando_receiver(object: &ParsedExpression, symbols: &SymbolTable, ctx: &CheckerContext) -> bool {
    let ParsedExpression::Identifier { name, .. } = object else {
        return false;
    };
    let Some(symbol) = symbols.get(name) else {
        return false;
    };
    let callable = |ty: &Type| match ty.peeled() {
        Type::Function(_) => true,
        Type::Object(object) => object.call_signature().is_some(),
        _ => false,
    };
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    // The binder declares the member only where the receiver is a local of
    // the container the write sits in (`lookupSymbolForPropertyAccess`): a
    // write from a nested function or a parameter default declares nothing.
    if !javascript && symbols.binding_crosses_function_boundary(name) == Some(true) {
        return false;
    }
    match symbol.kind {
        SymbolKind::Function => true,
        // A member declared in one branch only leaves the join of the function
        // with and without it; it is the same expando receiver.
        SymbolKind::Const => match &symbol.ty {
            Type::Union(union) => union.types().iter().all(callable),
            other => callable(other) || (javascript && ctx.javascript_expando_objects.contains(name)),
        },
        SymbolKind::Var | SymbolKind::Let if javascript => ctx.javascript_expando_objects.contains(name),
        _ => false,
    }
}

/// Whether `value` reads `name.property_name` outside the functions it holds.
fn reads_own_member(value: &ParsedExpression, name: &str, property_name: &str) -> bool {
    let mut found = false;
    let mut visit = |expression: &ParsedExpression| {
        if !found && reads_own_member(expression, name, property_name) {
            found = true;
        }
    };
    match value {
        ParsedExpression::PropertyAccess { object, property_name: read, .. }
            if read == property_name
                && matches!(object.as_ref(), ParsedExpression::Identifier { name: object, .. } if object == name) =>
        {
            return true;
        }
        _ => value.for_each_child(&mut visit),
    }
    found
}

/// Declares `name.property_name` as `any` and checks the value with the member
/// readable as such, as the circular resolution leaves it.
fn declare_circular_expando_member(
    name: &str,
    property_name: &str,
    assignment: &ParsedMemberAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let Some(symbol) = scopes.resolve(name) else {
        return;
    };
    let declared = match symbol.ty.peeled() {
        Type::Object(object) => {
            let mut properties = (*object.properties).clone();
            properties.insert(property_name.into(), surge_ts_types::ObjectProperty::required(Type::Any));
            let mut object = object.clone();
            object.properties = std::sync::Arc::new(properties);
            object.property_map_id = None;
            Type::Object(object)
        }
        Type::Function(function) => {
            let mut properties = surge_ts_types::PropertyMap::default();
            properties.insert(property_name.into(), surge_ts_types::ObjectProperty::required(Type::Any));
            Type::Object(crate::metrics::alloc_object_type(properties, None).with_call_signature(function))
        }
        _ => return,
    };
    let updated = SymbolInfo {
        ty: declared,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let mut provisional = visible_symbols(scopes).clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let _ = provisional.insert(name.to_string(), updated.clone());
    let _ = evaluate_expression(&assignment.value, assignment.value_span, &provisional, ctx);
    let _ = scopes.update_visible(name, updated);
}

/// Records `fn.x = value` on the binding, so the reads after it see the member
/// tsc declares there: the function becomes `{ (…): R; x: typeof value }`.
fn declare_expando_member(
    object: &ParsedExpression,
    property_name: &str,
    assignment: &ParsedMemberAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let ParsedExpression::Identifier { name, .. } = object else {
        return;
    };
    let symbols = visible_symbols(scopes);
    // An assignment declaration's value has no contextual type, so an object
    // literal written there is its own members' `this`.
    let value_literal = match &assignment.value {
        ParsedExpression::ObjectLiteral { span: Some(span), .. } => u32::try_from(span.start).ok(),
        _ => None,
    };
    // The member's type is its value's (`getWidenedTypeForAssignmentDeclaration`),
    // so a value reading the member itself (`o.K = o.K || f`) is circular:
    // `any`, reported under noImplicitAny (`reportCircularityError`).
    if reads_own_member(&assignment.value, name, property_name) {
        if ctx.options.no_implicit_any {
            let diagnostic = Diagnostic::ts7022(property_name, ctx.file_name.clone());
            ctx.push(match assignment.target_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
        declare_circular_expando_member(name, property_name, assignment, scopes, ctx);
        return;
    }
    // The binder declares the member before a class body assigned to it is
    // checked, so the body may already read it (`NS.K = class { m() { new
    // NS.K() } }`); surge types it only once the class is evaluated.
    let provisional = matches!(assignment.value, ParsedExpression::ClassExpression(_))
        .then(|| {
            let symbol = symbols.get(name)?;
            let Type::Object(object) = symbol.ty.peeled() else {
                return None;
            };
            if object.get_property(property_name).is_some() {
                return None;
            }
            let mut properties = (*object.properties).clone();
            properties.insert(property_name.into(), surge_ts_types::ObjectProperty::required(Type::Unknown));
            let mut declared = symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            let _ = declared.insert(
                name.clone(),
                crate::symbols::SymbolInfo {
                    ty: Type::Object(crate::metrics::alloc_object_type(
                        properties,
                        object.string_index_type.as_deref().cloned(),
                    )),
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
            );
            Some(declared)
        })
        .flatten();
    let outer_literal = std::mem::replace(&mut ctx.expando_object_literal, value_literal);
    let evaluated = evaluate_expression(
        &assignment.value,
        assignment.value_span,
        provisional.as_ref().unwrap_or(symbols),
        ctx,
    );
    ctx.expando_object_literal = outer_literal;
    let InferredExpression::Known(value_type) = evaluated else {
        return;
    };
    if value_type.is_unknown() {
        return;
    }
    let Some(symbol) = scopes.resolve(name) else {
        return;
    };
    let members = match &symbol.ty {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let mut call_signature = None;
    let mut properties = surge_ts_types::PropertyMap::default();
    let plain_object = surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && ctx.javascript_expando_objects.contains(name);
    for member in &members {
        match member.peeled() {
            Type::Function(function) => {
                call_signature.get_or_insert(function);
            }
            Type::Object(object) => {
                match object.call_signature() {
                    Some(signature) => {
                        call_signature.get_or_insert(signature.clone());
                    }
                    None if plain_object => {}
                    None => return,
                }
                // A member only some branches declared may be absent.
                for (name, property) in object.properties.iter() {
                    properties.entry(name.clone()).or_insert_with(|| {
                        let mut property = property.clone();
                        property.optional |= members.len() > 1;
                        property
                    });
                }
            }
            _ => return,
        }
    }
    if call_signature.is_none() && !plain_object {
        return;
    }
    let value_type = crate::checks::var::widen_implicit_variable_initializer_type(
        SymbolKind::Let,
        &assignment.value,
        &value_type,
        false,
    );
    properties.insert(
        property_name.into(),
        surge_ts_types::ObjectProperty::required(value_type),
    );
    let object = crate::metrics::alloc_object_type(properties, None);
    let updated = SymbolInfo {
        ty: Type::Object(match call_signature {
            Some(call_signature) => object.with_call_signature(call_signature),
            None => object,
        }),
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let _ = scopes.update_visible(name, updated);
}

/// tsc's `getContextualThisParameterType` for `obj.xxx = function () {…}`:
/// under `noImplicitThis`, and always in JavaScript, the function's `this` is
/// `obj`'s widened type — except `exports.xxx = …`, which a CommonJS module
/// leaves untyped. Handed to the function's check, which runs next.
fn hand_off_assigned_function_this(
    assignment: &ParsedMemberAssignment,
    object: &ParsedExpression,
    receiver_type: &Type,
    ctx: &mut CheckerContext,
) {
    let ParsedExpression::ArrowFunction(function) = &assignment.value else {
        return;
    };
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    let exports = matches!(object, ParsedExpression::Identifier { name, .. } if name == "exports");
    if function.this_binding != surge_ts_syntax::ParsedThisBinding::ImplicitAny
        || !(javascript || ctx.options.no_implicit_this)
        || (javascript && exports)
    {
        return;
    }
    ctx.next_arrow_this = Some(crate::checks::expr::widen_type(receiver_type));
}

/// Whether the member's type writes the `this` of the signature it gives an
/// assigned function (`onclick: ((this: GlobalEventHandlers, …) => any) |
/// null`), which `getContextualThisParameterType` takes before the receiver.
fn writes_contextual_this(target: &Type) -> bool {
    match target.peeled() {
        Type::Function(function) => function.this_type().is_some(),
        Type::Union(union) => union.types().iter().any(writes_contextual_this),
        Type::Object(object) => object.call_signature().is_some_and(|signature| signature.this_type().is_some()),
        _ => false,
    }
}

pub(crate) fn check_member_assignment(
    assignment: ParsedMemberAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    // `x[n] = v` on an evolving array writes through an `any[]` receiver.
    let evolving_receiver = ctx.auto_arrays_declared
        && super::evolving_arrays::element_write_receiver(
            &assignment.target,
            scopes.visible_symbols(),
        )
        .is_some();
    if evolving_receiver {
        ctx.evolving_array_operation_target = match &assignment.target {
            ParsedExpression::IndexAccess { object_span, .. } => *object_span,
            ParsedExpression::ElementAccess { object, .. } => match object.as_ref() {
                ParsedExpression::Identifier { span, .. } => *span,
                _ => None,
            },
            _ => None,
        };
    }
    // The target is bound before the value (`bindBinaryExpressionFlow`), so
    // what it assigns holds there: `(x = "", o).p = x.length`.
    if assignment.target.contains_assignment() {
        for (write, conditional) in crate::flow::expression_assignments(&assignment.target) {
            apply_assignment_expression(write, conditional, None, scopes, ctx);
        }
    }
    check_member_assignment_itself(assignment, scopes, ctx);
    if evolving_receiver {
        ctx.evolving_array_operation_target = None;
    }
}

/// `checkPropertyAccessExpressionOrQualifiedName`: a write through a generic
/// object type never consults index signatures, so a name only its
/// constraint's index signature answers is missing on the type variable.
fn writes_through_constraint_index_only(receiver: &Type, property_name: &str) -> bool {
    const OBJECT_MEMBERS: &[&str] = &[
        "constructor",
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ];
    if !receiver.is_type_variable()
        || surge_ts_types::private_name::is_private_name_key(property_name)
        || OBJECT_MEMBERS.contains(&property_name)
    {
        return false;
    }
    let apparent = surge_ts_types::type_variable::base_constraint_or_type(receiver);
    if apparent.is_type_variable() || crate::checks::function::type_contains_degradation(&apparent) {
        return false;
    }
    let Type::Object(object) = apparent.peeled() else {
        return false;
    };
    if object.get_property(property_name).is_some()
        || object.call_signature.is_some()
        || object.construct_signature.is_some()
        || object.synthetic_open_index
    {
        return false;
    }
    object.string_index_type.is_some()
        || (object.number_index_type.is_some() && surge_ts_types::is_numeric_key(property_name))
}

fn check_member_assignment_itself(
    assignment: ParsedMemberAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let module_level = std::mem::take(&mut ctx.module_level_member_write);
    // A private name is never a CommonJS export.
    let writes_private_name = matches!(
        &assignment.target,
        ParsedExpression::PropertyAccess { property_name, is_bracketed: false, .. }
            if surge_ts_types::private_name::is_private_name_key(property_name)
    );
    if !writes_private_name && is_commonjs_export_declaration(&assignment, scopes, ctx) {
        return;
    }

    match &assignment.target {
        ParsedExpression::IndexAccess {
            object_name,
            object_span,
            index,
            index_span,
        } => {
            let object = ParsedExpression::Identifier {
                name: object_name.clone(),
                span: *object_span,
            };
            check_element_assignment(
                &object,
                *object_span,
                index,
                *index_span,
                &assignment,
                scopes,
                ctx,
            );
            return;
        }
        ParsedExpression::ElementAccess {
            object,
            object_span,
            index,
            index_span,
        } => {
            check_element_assignment(
                object,
                *object_span,
                index,
                *index_span,
                &assignment,
                scopes,
                ctx,
            );
            return;
        }
        _ => {}
    }

    let ParsedExpression::PropertyAccess {
        object,
        object_span,
        property_name,
        property_span,
        is_bracketed,
        ..
    } = &assignment.target
    else {
        return;
    };

    let visible_symbols = visible_symbols(scopes);

    // A module namespace object's members are read-only properties. The
    // member is resolved first: one the module does not export is reported as
    // a read of it would be, not as a read-only write.
    if let ParsedExpression::Identifier { name, .. } = object.as_ref()
        && ctx.is_namespace_import_binding(name)
        && !scopes.declares_locally(name)
    {
        let exported = match evaluate_expression(object, object_span.or(assignment.target_span), &visible_symbols, ctx) {
            InferredExpression::Known(namespace) => namespace.get_property_access_type(property_name).is_some(),
            _ => true,
        };
        if !exported {
            let _ = evaluate_expression(&assignment.target, assignment.target_span, &visible_symbols, ctx);
        } else if let Some(span) = property_span.or(assignment.target_span) {
            ctx.push(
                Diagnostic::ts2540(property_name, ctx.file_name.clone())
                    .with_span(convert_span(span)),
            );
        }
        check_value_without_target(&assignment, None, &visible_symbols, ctx);
        return;
    }

    // The receiver of the write is still read: `decl.init.callee = x` reports
    // a possibly-undefined `decl` and a missing `init` on it exactly as a read
    // would. The written member itself is only reported below on a union
    // receiver, so a value surge models incompletely
    // (`Component.getInitialProps = …`) is not a false TS2339 here.
    let object_type = match widened_write_receiver(object, object_span.or(assignment.target_span), &visible_symbols, ctx)
        .unwrap_or_else(|| evaluate_expression(object, object_span.or(assignment.target_span), &visible_symbols, ctx))
    {
        InferredExpression::Known(ty) => surge_ts_types::without_polymorphic_this(ty),
        receiver => {
            if !*is_bracketed
                && surge_ts_types::private_name::is_private_name_key(property_name)
                && let Some(private_receiver) = crate::checks::expr::PrivateNameReceiver::of(&receiver)
            {
                let _ = crate::checks::expr::check_private_name_access(
                    private_receiver,
                    property_name,
                    *property_span,
                    &visible_symbols,
                    ctx,
                );
            }
            check_value_without_target(&assignment, receiver.flowing_type().as_ref(), &visible_symbols, ctx);
            return;
        }
    };
    // A write through a possibly-undefined receiver (`decl.id = x` with
    // `decl` from an indexed read) is the same TS18048 a read reports.
    let object_type = crate::checks::expr::strip_reported_undefined_receiver(
        object,
        object_type,
        *object_span,
        assignment.target_span,
        &visible_symbols,
        ctx,
    );
    let _ = object_span;
    if !*is_bracketed
        && surge_ts_types::private_name::is_private_name_key(property_name)
        && let Some(ty) = crate::checks::expr::check_private_name_access(
            crate::checks::expr::PrivateNameReceiver::Type(&object_type),
            property_name,
            *property_span,
            &visible_symbols,
            ctx,
        )
    {
        check_value_without_target(&assignment, Some(&ty), &visible_symbols, ctx);
        return;
    }
    if !*is_bracketed && writes_through_constraint_index_only(&object_type, property_name) {
        if let Some(span) = property_span.or(assignment.target_span) {
            ctx.push(
                Diagnostic::ts2339(property_name, object_type.name(), ctx.file_name.clone())
                    .with_span(convert_span(span)),
            );
        }
        check_value_without_target(&assignment, None, &visible_symbols, ctx);
        return;
    }
    if !*is_bracketed {
        crate::checks::expr::check_member_accessibility(
            object,
            &object_type,
            property_name,
            *property_span,
            true,
            &visible_symbols,
            ctx,
        );
        // A dotted write through the index signature is the same TS4111 a
        // dotted read is.
        crate::checks::expr::emit_index_signature_access_on(
            &object_type,
            property_name,
            property_span.or(assignment.target_span),
            ctx,
        );
    }

    // tsc's binder declares `F.prototype = …` on an expando container it finds
    // in the write's own container (`bindDeferredExpandoAssignment`,
    // `lookupEntity`): a function's own symbol never declares `prototype`,
    // which only its apparent type has. The write is an assignment
    // declaration, whose value has no contextual type
    // (`getContextualTypeForAssignmentExpression`).
    if !*is_bracketed
        && property_name == "prototype"
        && let ParsedExpression::Identifier { name, .. } = object.as_ref()
        && (module_level || scopes.declares_locally(name))
        && !ctx.is_import_binding(name)
        && is_expando_receiver(object, &visible_symbols, ctx)
    {
        declare_expando_member(object, property_name, &assignment, scopes, ctx);
        return;
    }

    // A write checks against the property's *declared* type: after
    // `if (o.flag === undefined)` the read type is narrowed to `undefined`, but
    // `o.flag = true` is still an assignment to `boolean | undefined`.
    // A receiver declared `any` and narrowed (`o instanceof A`) is written
    // through what it narrowed to; `any` would admit every member.
    let declared_object_type = declared_reference_type(object, &visible_symbols)
        .filter(|declared| !matches!(declared, Type::Any) || matches!(object_type, Type::Any))
        .or_else(|| declared_object_type_by_name(&object_type, ctx));
    let receiver_for_declaration = declared_object_type
        .as_ref()
        .unwrap_or(&object_type)
        .clone();
    // A union's member is read-only when any constituent's is, so the
    // constituents flow left decide it.
    let readonly_receiver = if matches!(receiver_for_declaration.peeled(), Type::Union(_)) {
        &object_type
    } else {
        &receiver_for_declaration
    };
    let constructor_may_write = matches!(object.as_ref(), ParsedExpression::This { .. })
        && ctx
            .constructor_writable_members
            .as_ref()
            .is_some_and(|members| members.contains(property_name));
    if !constructor_may_write
        && report_readonly_property_write(
            readonly_receiver,
            property_name,
            property_span.or(assignment.target_span),
            ctx,
        )
    {
        check_value_without_target(&assignment, None, &visible_symbols, ctx);
        return;
    }
    // `checkPropertyAccessExpressionOrQualifiedName`: a name no member answers
    // is read through the receiver's index signature, and a `readonly` one
    // refuses the write. The write is still checked against its value type.
    if object_type.writes_readonly_index(
        Some(property_name.as_str()),
        surge_ts_types::is_numeric_key(property_name),
    ) {
        let diagnostic = Diagnostic::ts2542(object_type.name(), ctx.file_name.clone());
        ctx.push(match assignment.target_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
    let accessor_write_type = accessor_write_type(&receiver_for_declaration, property_name, ctx)
        .or_else(|| union_accessor_write_type(&receiver_for_declaration, property_name, ctx))
        .or_else(|| intersection_accessor_write_type(&receiver_for_declaration, property_name, ctx));
    let exact_optional_write_type = if accessor_write_type.is_none() {
        crate::checks::expr::exact_optional_write_type(&receiver_for_declaration, property_name)
    } else {
        None
    };
    let Some(target_type) = accessor_write_type.or_else(|| exact_optional_write_type.clone()).or_else(|| {
        declared_object_type
            .as_ref()
            .and_then(|declared| declared.get_property_access_type(property_name))
            .or_else(|| {
                let member = object_type.get_property_access_type(property_name)?;
                // Narrowing a binding records its declared type, so a binding
                // without one holds its declared member, literal or not
                // (`t.length = 0` on `[number]` writes to `1`). Any other
                // receiver expression keeps the widening.
                Some(
                    if declared_object_type.is_none()
                        && matches!(object.as_ref(), ParsedExpression::Identifier { .. })
                    {
                        member
                    } else {
                        widen_narrowed_literals(&member)
                    },
                )
            })
            // A type variable receiver writes through its apparent type
            // (`getApparentType` of its base constraint).
            .or_else(|| {
                object_type
                    .is_type_variable()
                    .then(|| surge_ts_types::type_variable::base_constraint_or_type(&object_type))
                    .filter(|apparent| {
                        !apparent.is_type_variable() && !crate::checks::function::type_contains_degradation(apparent)
                    })
                    .and_then(|apparent| match apparent.peeled() {
                        Type::Object(object) => object.get_property(property_name).map(|property| {
                            if property.optional {
                                union_type(vec![property.ty.clone(), Type::Undefined])
                            } else {
                                property.ty.clone()
                            }
                        }),
                        _ => None,
                    })
            })
    }) else {
        // `decl.id = x` on `A | B` where `B` has no `id`: tsc reports the
        // member on the union. Only a union of fully modelled objects is
        // reported, so an incompletely modelled receiver stays silent.
        //
        // The union has to be the receiver's *own* form, not something it peels
        // to. A reference that peels to one may have lost members on the way —
        // `NextComponentType<…> = ComponentType<P> & { getInitialProps?… }`
        // peels to `ComponentType`'s union alone, and reporting off that made
        // `MyApp.getInitialProps = …` a false TS2339.
        let receiver = declared_object_type.as_ref().unwrap_or(&object_type);
        if is_expando_receiver(object, &visible_symbols, ctx) {
            hand_off_assigned_function_this(&assignment, object, &object_type, ctx);
            declare_expando_member(object, property_name, &assignment, scopes, ctx);
            ctx.next_arrow_this = None;
            return;
        }
        let receiver = receiver.clone();
        if !*is_bracketed
            && let Type::Union(union) = &receiver
            && union.types().iter().all(|member| {
                matches!(member.peeled(), Type::Object(_))
                    && !crate::checks::expr::carries_leaked_type_parameter(member, ctx)
            })
        {
            let diagnostic = crate::checks::expr::missing_property_diagnostic(
                property_name,
                &receiver,
                &visible_symbols,
                ctx,
            );
            let span = property_span.or(assignment.target_span);
            ctx.push(match span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        } else if *is_bracketed && is_widened_empty_fallback(object) {
            // The read of `(o || {})["k"]` keeps `o`'s names on the fallback,
            // which the write's widened `{}` no longer has (TS7053).
            let members = match receiver.peeled() {
                Type::Union(union) => union.types().to_vec(),
                other => vec![other],
            };
            let empty = members.iter().find(|member| {
                matches!(member.peeled(), Type::Object(fallback)
                    if fallback.properties.is_empty()
                        && fallback.string_index_type.is_none()
                        && fallback.number_index_type.is_none()
                        && !fallback.synthetic_open_index)
            });
            if ctx.options.no_implicit_any
                && let Some(empty) = empty
            {
                let diagnostic = Diagnostic::ts7053(
                    Type::StringLiteral(property_name.clone()).name(),
                    empty.name(),
                    ctx.file_name.clone(),
                );
                ctx.push(match assignment.target_span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                });
            }
        } else if *is_bracketed {
            // tsc's `checkElementAccessExpression` resolves a written element
            // through `getPropertyTypeForIndexType` exactly as it resolves a read
            // one, union receivers included: a key neither a member nor an index
            // signature answers is the implicit `any` the read reports (TS7053,
            // TS7052 naming `set`), not TS2339.
            let _ = crate::checks::expr::with_element_write_target(|| {
                evaluate_expression(&assignment.target, assignment.target_span, &visible_symbols, ctx)
            });
        } else if let InferredExpression::MissingProperty {
            property_name,
            object_type,
            span,
        } = crate::infer::infer_expression(&assignment.target, &visible_symbols, ctx)
        {
            // Any other receiver reports a missing member exactly as a read of
            // it does, gated by the same modelling checks the read applies.
            let diagnostic = match crate::checks::expr::global_this_missing_member(
                &property_name,
                &object_type,
                ctx,
            ) {
                Some(diagnostic) => diagnostic,
                None => Some(crate::checks::expr::missing_property_diagnostic(
                    &property_name,
                    &object_type,
                    &visible_symbols,
                    ctx,
                )),
            };
            if let Some(diagnostic) = diagnostic
                && let Some(span) = span.or(*property_span).or(assignment.target_span)
            {
                ctx.push(diagnostic.with_span(convert_span(span)));
            }
        }
        hand_off_assigned_function_this(&assignment, object, &object_type, ctx);
        check_value_without_target(&assignment, Some(&receiver), &visible_symbols, ctx);
        ctx.next_arrow_this = None;
        return;
    };

    // A target surge could not fully resolve carries no contextual type for the
    // value either: the value is evaluated as against tsc's error type, which
    // still reports what it reads (an unresolved name) but nothing it could
    // not be given (an object literal's method parameter as implicit-any). It
    // is evaluated at all because the narrowing the write installs depends on
    // it (`bag.patterns = []` makes the reads after it non-optional). The
    // comparison below is skipped for such a target.
    let target_unresolved = crate::checks::assign::type_contains_unknown(&target_type);

    // `checkObjectLiteral`: an empty JavaScript object literal that expando
    // writes declare members on is typed by those members, and the write
    // initializing an expando member with it is that member's declaration —
    // its value is the member's own type.
    if surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && matches!(&assignment.value, ParsedExpression::ObjectLiteral { properties, .. } if properties.is_empty())
        && is_expando_receiver(object, &visible_symbols, ctx)
    {
        crate::checks::function::narrowing::narrow_assignment_target_in_scope(
            &assignment.target,
            &target_type,
            scopes,
        );
        return;
    }

    // An expando container's member is an assignment declaration, whose
    // value has no contextual signature to take `this` from; a declared
    // member's signature gives `this` only where it writes one.
    if is_expando_receiver(object, &visible_symbols, ctx)
        || (!target_unresolved && !writes_contextual_this(&target_type))
    {
        hand_off_assigned_function_this(&assignment, object, &object_type, ctx);
    }
    // A JSDoc `@type` on the write types the member an assignment declaration
    // declares; a member the receiver already has keeps its own type, and the
    // value relates to that (`checkAssignmentOperator` against the left side).
    let (value, value_span) = match &assignment.value {
        ParsedExpression::TypeAssertion {
            expression,
            expression_span,
            annotation: true,
            type_span: Some(_),
            ..
        } if surge_ts_syntax::is_javascript_file_name(&ctx.file_name) => {
            (expression.as_ref(), expression_span.or(assignment.value_span))
        }
        value => (value, assignment.value_span),
    };
    let inferred_value = if target_unresolved {
        ctx.degraded_expected_type_depth += 1;
        let inferred = evaluate_expression(value, value_span, &visible_symbols, ctx);
        ctx.degraded_expected_type_depth -= 1;
        inferred
    } else {
        crate::checks::expr::with_property_write_target(*property_span, || {
            crate::checks::expected::evaluate_expression_with_expected_type_anchored(
                value,
                value_span,
                assignment.target_span,
                Some(&target_type),
                crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
                &visible_symbols,
                ctx,
            )
        })
    };
    ctx.next_arrow_this = None;

    let InferredExpression::Known(value_type) = inferred_value else {
        return;
    };

    if target_unresolved || value_type.is_unmodelled() || target_type.is_unmodelled() {
        crate::checks::function::narrowing::narrow_assignment_target_in_scope(
            &assignment.target,
            &value_type,
            scopes,
        );
        return;
    }

    if !is_assignable_to(&value_type, &target_type) {
        let reported_target =
            crate::checks::expr::reported_relation_target(&value_type, &target_type);
        let source_name = crate::checks::expr::source_display_name(&value_type, &reported_target);
        // tsc heads a dotted write's report TS2412 when the value may be
        // `undefined` and the property is optional without it
        // (`checkAssignmentOperatorWorker`).
        let diagnostic = if exact_optional_write_type.is_some()
            && !*is_bracketed
            && crate::checks::expr::may_be_undefined(&value_type)
        {
            Diagnostic::ts2412(&source_name, reported_target.name(), ctx.file_name.clone())
        } else {
            crate::checks::expr::assignability_mismatch_diagnostic(
                &value_type,
                &reported_target,
                &source_name,
                &reported_target.name(),
                false,
                ctx.file_name.clone(),
            )
        };
        let diagnostic = match assignment.target_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };
        ctx.push(diagnostic);
        return;
    }

    crate::checks::function::narrowing::narrow_assignment_target_in_scope(
        &assignment.target,
        &value_type,
        scopes,
    );
}

/// A write to a member `this` does not declare is TS2339 in a `.ts` file (only
/// JS binds `this.x = …` as a declaration), reported exactly where a read of
/// it would be.
fn report_missing_this_member(
    assignment: &ParsedThisPropertyAssignment,
    this_span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let read = ParsedExpression::PropertyAccess {
        object: Box::new(ParsedExpression::This { span: this_span }),
        object_span: this_span,
        property_name: assignment.property_name.clone(),
        property_span: assignment.property_span,
        is_bracketed: assignment.is_bracketed,
        binding_element: false,
    };
    // A bracketed write resolves as an element access does (TS7053 under
    // `noImplicitAny`); JavaScript binds it as a declaration instead.
    if assignment.is_bracketed {
        if !surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
            let _ = crate::checks::expr::with_element_write_target(|| {
                evaluate_expression(&read, assignment.target_span, symbols, ctx)
            });
        }
        return;
    }
    if let InferredExpression::MissingProperty {
        property_name,
        object_type,
        span,
    } = crate::infer::infer_expression(&read, symbols, ctx)
        && let Some(span) = span.or(assignment.property_span)
    {
        let diagnostic = match crate::checks::expr::global_this_missing_member(
            &property_name,
            &object_type,
            ctx,
        ) {
            Some(diagnostic) => diagnostic,
            None => Some(crate::checks::expr::missing_property_diagnostic(
                &property_name,
                &object_type,
                symbols,
                ctx,
            )),
        };
        if let Some(diagnostic) = diagnostic {
            ctx.push(diagnostic.with_span(convert_span(span)));
        }
    }
}

/// Checks a `this.<property> = <value>` assignment against the instance
/// property's declared type. The `this` symbol is bound to the class instance
/// type for the duration of the method/constructor body. When `this` or the
/// property cannot be resolved, no diagnostic is emitted so unsupported class
/// shapes do not cascade.
pub(crate) fn check_this_property_assignment(
    assignment: ParsedThisPropertyAssignment,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let visible_symbols = visible_symbols(scopes);

    let Some(this_symbol) = visible_symbols.get("this") else {
        // tsc's `tryGetThisTypeAt`: where a script's top level (arrows looked
        // through) owns `this` it is `globalThis`, and a member that lacks is
        // reported as a read of it would be.
        let this_span = assignment.target_span.map(|span| surge_ts_syntax::TextSpan {
            start: span.start,
            end: span.start + "this".len(),
        });
        // A module's top level owns `this` too, and it is `undefined`: the
        // write reads it as any member access does, and the value is checked.
        if this_span
            .and_then(|span| u32::try_from(span.start).ok())
            .is_some_and(|start| ctx.module_this_starts.contains(&start))
        {
            let read = ParsedExpression::PropertyAccess {
                object: Box::new(ParsedExpression::This { span: this_span }),
                object_span: this_span,
                property_name: assignment.property_name.clone(),
                property_span: assignment.property_span,
                is_bracketed: assignment.is_bracketed,
                binding_element: false,
            };
            let _ = evaluate_expression(&read, assignment.target_span, visible_symbols, ctx);
            let _ = evaluate_expression(&assignment.value, assignment.value_span, visible_symbols, ctx);
            return;
        }
        report_missing_this_member(&assignment, this_span, visible_symbols, ctx);
        return;
    };

    if surge_ts_types::private_name::is_private_name_key(&assignment.property_name) {
        let receiver = visible_symbols
            .declared_type("this")
            .cloned()
            .unwrap_or_else(|| this_symbol.ty.clone());
        if crate::checks::expr::check_private_name_access(
            crate::checks::expr::PrivateNameReceiver::Type(&receiver),
            &assignment.property_name,
            assignment.property_span,
            &visible_symbols,
            ctx,
        )
        .is_some()
        {
            // The value is still checked, as against tsc's error type: nothing
            // it holds is contextually typed by the missing member.
            ctx.degraded_expected_type_depth += 1;
            let _ = evaluate_expression(&assignment.value, assignment.value_span, &visible_symbols, ctx);
            ctx.degraded_expected_type_depth -= 1;
            return;
        }
    }

    // A write checks against the property's *declared* type, as
    // `check_member_assignment` does for `o.p = …`: inside
    // `if (this.value === "valid") this.value = "dirty"` the read type of
    // `this.value` is narrowed to `"valid"`, but the write still targets
    // `"aborted" | "dirty" | "valid"`.
    let Some(property_type) = visible_symbols
        .declared_type("this")
        .and_then(|declared| declared.get_property_access_type(&assignment.property_name))
        .or_else(|| {
            this_symbol
                .ty
                .get_property_access_type(&assignment.property_name)
        })
        .or_else(|| {
            // `this: T` reads its members off `T`'s constraint.
            this_symbol
                .ty
                .is_type_variable()
                .then(|| surge_ts_types::type_variable::base_constraint_or_type(&this_symbol.ty))
                .and_then(|constraint| constraint.get_property_access_type(&assignment.property_name))
        })
    else {
        report_missing_this_member(&assignment, None, visible_symbols, ctx);
        // The value is still checked, as against tsc's error type: nothing it
        // holds is contextually typed by the missing member.
        ctx.degraded_expected_type_depth += 1;
        let _ = evaluate_expression(&assignment.value, assignment.value_span, visible_symbols, ctx);
        ctx.degraded_expected_type_depth -= 1;
        return;
    };
    // A member typed by the polymorphic `this` (`self = this`) is written as
    // the `this` of the body, as it is read.
    let property_type = crate::checks::function::bind_this_member(&this_symbol.ty, property_type);

    // The write goes through the setter's accessibility
    // (`getDeclarationModifierFlagsFromSymbol` with `isWrite`).
    let this_receiver = visible_symbols
        .declared_type("this")
        .cloned()
        .unwrap_or_else(|| this_symbol.ty.clone());
    // `this: T` is judged by `T`'s constraint, as its members are read.
    let this_receiver = if this_receiver.is_type_variable() {
        surge_ts_types::type_variable::base_constraint_or_type(&this_receiver)
    } else {
        this_receiver
    };
    crate::checks::expr::check_member_accessibility(
        &ParsedExpression::This { span: None },
        &this_receiver,
        &assignment.property_name,
        assignment.property_span,
        true,
        &visible_symbols,
        ctx,
    );

    let constructor_may_write = ctx
        .constructor_writable_members
        .as_ref()
        .is_some_and(|members| members.contains(&assignment.property_name));
    if !constructor_may_write {
        let receiver = visible_symbols
            .declared_type("this")
            .cloned()
            .unwrap_or_else(|| this_symbol.ty.clone());
        if report_readonly_property_write(
            &receiver,
            &assignment.property_name,
            assignment.property_span,
            ctx,
        ) {
            return;
        }
    }

    // Evaluated against the member like an `o.p = …` write: the target types
    // the value contextually, an object literal elaborates into its members, and
    // a whole-value mismatch is reported on `this.<property>`.
    // A target surge could not resolve types nothing: the value is checked as
    // against tsc's error type, which still reports what it reads.
    let target_unresolved = crate::checks::assign::type_contains_unknown(&property_type);
    let inferred_value = if target_unresolved {
        ctx.degraded_expected_type_depth += 1;
        let inferred = evaluate_expression(&assignment.value, assignment.value_span, &visible_symbols, ctx);
        ctx.degraded_expected_type_depth -= 1;
        inferred
    } else {
        crate::checks::expr::with_property_write_target(assignment.property_span, || {
            crate::checks::expected::evaluate_expression_with_expected_type_anchored(
                &assignment.value,
                assignment.value_span,
                assignment.target_span,
                Some(&property_type),
                crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
                &visible_symbols,
                ctx,
            )
        })
    };

    let InferredExpression::Known(value_type) = inferred_value else {
        return;
    };

    // The write narrows `this.<property>` for the reads after it, as any
    // property reference's assignment does (`getAssignmentReducedType`).
    let target = ParsedExpression::PropertyAccess {
        object: Box::new(ParsedExpression::This { span: None }),
        object_span: None,
        property_name: assignment.property_name.clone(),
        property_span: assignment.property_span,
        is_bracketed: false,
        binding_element: false,
    };
    if target_unresolved || value_type.is_unmodelled() || property_type.is_unmodelled() {
        crate::checks::function::narrowing::narrow_assignment_target_in_scope(&target, &value_type, scopes);
        return;
    }

    if is_assignable_to(&value_type, &property_type) {
        crate::checks::function::narrowing::narrow_assignment_target_in_scope(&target, &value_type, scopes);
    } else {
        let reported_target =
            crate::checks::expr::reported_relation_target(&value_type, &property_type);
        let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
            &value_type,
            &reported_target,
            &crate::checks::expr::source_display_name(&value_type, &reported_target),
            &reported_target.name(),
            false,
            ctx.file_name.clone(),
        );
        let diagnostic = match assignment.target_span.or(assignment.value_span) {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };
        ctx.push(diagnostic);
    }
}

/// tsc's `getAssignmentReducedType`: an assignment narrows a union-declared
/// binding to the declared members the value may be assigned to — `result =
/// { type: "data", data }` leaves the declared `data` member, optional `id`
/// and all, not the literal's own shape. A fresh boolean literal keeps its
/// literal; surge's `boolean` is not a `true | false` union, so that member is
/// narrowed here directly. When the members kept do not admit the value, the
/// value itself stands in (tsc falls back to the declaration; surge's
/// relation is incomplete enough that the value is the safer answer).
pub(crate) fn assignment_reduced_type(declared: Option<&Type>, assigned: Type) -> Type {
    // An `any` fits every member, so tsc keeps the whole declaration. The
    // degradation sentinel stands for a value surge could not type; keeping
    // the declaration for it would report members the real value rules out.
    if matches!(assigned, Type::Unknown) {
        return assigned;
    }
    if matches!(assigned, Type::Any) {
        return match declared {
            Some(declared) if matches!(declared.peeled(), Type::Union(_)) => declared.clone(),
            _ => assigned,
        };
    }
    // A mapped type applied to a union (`Partial<A | B>`) is the union of its
    // applications, as tsc's instantiation distributes it.
    let Some(Type::Union(declared)) = declared.map(Type::peeled) else {
        return assigned;
    };
    let kept: Vec<Type> = declared
        .types()
        .iter()
        .filter(|member| type_maybe_assignable_to(&assigned, member))
        .map(|member| match (member, &assigned) {
            (Type::Boolean, Type::BooleanLiteral(_)) => assigned.clone(),
            _ => member.clone(),
        })
        .collect();
    if kept.is_empty() {
        return assigned;
    }
    let reduced = union_type(kept);
    if is_assignable_to(&assigned, &reduced) {
        reduced
    } else {
        assigned
    }
}

fn type_maybe_assignable_to(source: &Type, target: &Type) -> bool {
    match source {
        // A member surge could not type fits every declared member, which would
        // keep all of them (`x ??= f()` keeping `null`); the modelled members
        // decide when there are any.
        Type::Union(union) if union.types().iter().any(|member| !member.is_unmodelled()) => union
            .types()
            .iter()
            .filter(|member| !member.is_unmodelled())
            .any(|member| member == target || assignable_with_common_properties(member, target)),
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| member == target || assignable_with_common_properties(member, target)),
        _ => assignable_with_common_properties(source, target),
    }
}

/// Assignability with tsc's weak-type rule, which surge's relation leaves
/// out: an object whose properties are all optional accepts nothing that
/// shares none of them (`isRelatedTo`'s common-property check).
pub(crate) fn assignable_with_common_properties(source: &Type, target: &Type) -> bool {
    if !is_assignable_to(source, target) {
        return false;
    }
    match target.peeled() {
        Type::Object(object) if crate::checks::call::is_weak_object(&object) => {
            crate::checks::call::shares_a_property(source, &object)
        }
        _ => true,
    }
}

/// Applies the assignments evaluating `expression` performs (`if ((m = f()))`,
/// `a = b = c`) to the bindings, as an assignment statement does once checked:
/// the target takes the assigned type and is definitely assigned after it. One
/// that runs on only some paths (`c && (x = 1)`) joins the assigned type with
/// the binding's and leaves it as assigned as it was.
pub(crate) fn apply_expression_assignments(
    expression: &ParsedExpression,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    apply_expression_member_assignments(expression, scopes, ctx);
    let assignments = crate::flow::expression_assignments(expression);
    if assignments.is_empty() {
        return;
    }
    let certain = crate::flow::certainly_assigned_names(expression);
    let skip_edge = short_circuit_skip_edge(expression, scopes.visible_symbols());
    // Every path assigns a target a `?:` writes on both sides, so it holds one
    // of those values: their union replaces what it held before.
    let mut joined: Vec<(&str, Vec<Type>)> = Vec::new();
    for (assignment, conditional) in &assignments {
        let ParsedExpression::Assignment {
            target_name, value, ..
        } = assignment
        else {
            continue;
        };
        if *conditional && certain.contains(&target_name.as_str()) {
            if let InferredExpression::Known(ty) =
                crate::infer::infer_expression(value, visible_symbols(scopes), ctx)
            {
                match joined.iter_mut().find(|(name, _)| name == target_name) {
                    Some((_, types)) => types.push(ty),
                    None => joined.push((target_name.as_str(), vec![ty])),
                }
            }
            continue;
        }
        let skipped_type = skip_edge.as_ref().and_then(|(skipped, right_assignments)| {
            right_assignments
                .iter()
                .any(|seen| std::ptr::eq(*seen, *assignment))
                .then(|| skipped.get(target_name).map(|symbol| symbol.ty.clone()))
                .flatten()
        });
        apply_assignment_expression(assignment, *conditional, skipped_type, scopes, ctx);
    }
    for (name, types) in joined {
        let value = InferredExpression::Known(union_type(types));
        if !super::evolving_arrays::assign_evolving_array(name, false, &value, scopes, ctx) {
            update_assigned_symbol_type(name, value, scopes);
        }
    }
    if flow_state.tracked_local_count() > 0 {
        crate::flow::mark_expression_assignments(expression, flow_state);
    }
}

/// A member write nested in an expression (`const r = (this.#r = f())`)
/// narrows its reference for the reads after it, as the statement form does.
/// One that runs on only some paths leaves the reference as declared.
fn apply_expression_member_assignments(
    expression: &ParsedExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    for (write, conditional) in crate::flow::expression_member_assignments(expression) {
        let ParsedExpression::MemberAssignment(assignment) = write else {
            continue;
        };
        if conditional {
            continue;
        }
        let checkpoint = ctx.diagnostics().len();
        let inferred = crate::infer::infer_expression(&assignment.value, visible_symbols(scopes), ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        if let InferredExpression::Known(value_type) = inferred {
            crate::checks::function::narrowing::narrow_assignment_target_in_scope(
                &assignment.target,
                &value_type,
                scopes,
            );
        }
    }
}

/// The types of the assignments a condition has run once it is known true
/// (the right of a `&&` chain), for the branch that sees it true. Applied
/// before that branch narrows by the condition, which then refines them.
pub(crate) fn apply_condition_true_assignment_types(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let certain: Vec<&ParsedExpression> = crate::flow::expression_assignments(condition)
        .into_iter()
        .filter(|(_, conditional)| !conditional)
        .map(|(assignment, _)| assignment)
        .collect();
    for assignment in crate::flow::condition_true_assignments(condition) {
        if !certain.iter().any(|seen| std::ptr::eq(*seen, assignment)) {
            apply_assignment_expression(assignment, false, None, scopes, ctx);
        }
    }
}

/// Definite assignment for the branch that sees `condition` true.
pub(crate) fn mark_condition_true_assignments(
    condition: &ParsedExpression,
    flow_state: &mut FunctionFlowState,
) {
    if flow_state.tracked_local_count() == 0 {
        return;
    }
    for assignment in crate::flow::condition_true_assignments(condition) {
        if let ParsedExpression::Assignment { target_name, .. } = assignment {
            mark_assignment_state(target_name, flow_state);
        }
    }
}

/// The bindings on the edge where a `left && right`, `left || right` or
/// `left ?? right` skips its right operand, with the assignments the right
/// operand makes on every path through it. tsc's `bindLogicalLikeExpression`
/// routes that edge through the left's condition, so what such an assignment
/// joins with is the binding as the left narrowed it (`d ?? (d = "x")` leaves
/// `d` a `string`), not what it held before the statement.
fn short_circuit_skip_edge<'a>(
    expression: &'a ParsedExpression,
    symbols: &SymbolTable,
) -> Option<(SymbolTable, Vec<&'a ParsedExpression>)> {
    let (left, right, skipped) = match expression {
        ParsedExpression::Logical { left, operator, right, .. } => {
            let skips_when = *operator == surge_ts_syntax::ParsedLogicalOperator::Or;
            (left, right, crate::checks::function::narrow_condition_symbol_table(left, symbols, skips_when))
        }
        // `narrowTypeByOptionality`: the left skips the right when it is not nullish.
        ParsedExpression::NullishCoalescing { left, right, .. } => {
            let present = ParsedExpression::Binary {
                left: left.clone(),
                left_span: None,
                operator: surge_ts_syntax::ParsedBinaryOperator::NotEquals,
                operator_span: None,
                right: Box::new(ParsedExpression::NullLiteral),
                right_span: None,
                compound: false,
            };
            (left, right, crate::checks::function::narrow_condition_symbol_table(&present, symbols, true))
        }
        _ => return None,
    };
    if left.contains_assignment() {
        return None;
    }
    let right_assignments: Vec<&ParsedExpression> = crate::flow::expression_assignments(right)
        .into_iter()
        .filter(|(_, conditional)| !conditional)
        .map(|(assignment, _)| assignment)
        .collect();
    if right_assignments.is_empty() {
        return None;
    }
    Some((skipped?, right_assignments))
}

fn apply_assignment_expression(
    assignment: &ParsedExpression,
    conditional: bool,
    skipped_type: Option<Type>,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let ParsedExpression::Assignment {
        target_name,
        target_span,
        value,
        ..
    } = assignment
    else {
        return;
    };
    let before = skipped_type.or_else(|| scopes.resolve(target_name).map(|symbol| symbol.ty.clone()));
    if crate::flow::is_compound_assignment(target_name, *target_span, value) {
        widen_compound_assigned_symbol_type(target_name, scopes);
        return;
    }
    let inferred_value = crate::infer::infer_expression(value, visible_symbols(scopes), ctx);
    let assigns_empty_array = matches!(
        value.as_ref(),
        ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty()
    );
    if !super::evolving_arrays::assign_evolving_array(
        target_name,
        assigns_empty_array,
        &inferred_value,
        scopes,
        ctx,
    ) {
        update_assigned_symbol_type(target_name, inferred_value, scopes);
    }
    if conditional
        && let (Some(before), Some(symbol)) = (before, scopes.resolve(target_name))
        && symbol.ty != before
    {
        let joined = SymbolInfo {
            ty: union_type(vec![before, symbol.ty.clone()]),
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        scopes.insert_current(target_name.as_str(), joined);
    }
}

/// tsc's `getAssignmentReducedType` for a write surge could not relate
/// exactly: the declared union keeps the members the value may inhabit, or is
/// taken whole when none can be told apart. A binding narrowed to `null` that
/// is assigned an object (`d ??= { … }`) no longer reads as `null` afterwards.
/// A non-nullable value rules out the nullable members even where surge's
/// model of the value is too coarse to relate it to the others.
fn assignment_reduced_declared_type(
    declared: Option<&Type>,
    current: &Type,
    value: &Type,
) -> Option<Type> {
    let declared = declared?;
    if declared == current || value.is_unknown() {
        return None;
    }
    let Type::Union(union) = declared else {
        return Some(declared.clone());
    };
    let nullish = |ty: &Type| matches!(ty, Type::Null | Type::Undefined | Type::Void);
    let value_nullable = match value {
        Type::Union(members) => members.types().iter().any(nullish),
        other => nullish(other),
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| is_assignable_to(value, member) || (!value_nullable && !nullish(member)))
        .cloned()
        .collect();
    Some(if kept.is_empty() {
        declared.clone()
    } else {
        union_type(kept)
    })
}

/// tsc's `getTypeAtFlowAssignment` for a compound assignment (`x += v`): the
/// binding holds the base type of what it held before, whatever the operation
/// computes — it is never narrowed to the value.
pub(crate) fn widen_compound_assigned_symbol_type(target_name: &str, scopes: &mut ScopeStack) {
    let Some(symbol) = scopes.resolve(target_name) else {
        return;
    };
    let widened = base_type_of_literal(&symbol.ty);
    if widened == symbol.ty {
        return;
    }
    let updated = SymbolInfo {
        ty: widened,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let declared = scopes
        .visible_symbols()
        .declared_type(target_name)
        .cloned()
        .unwrap_or_else(|| symbol.ty.clone());
    scopes.insert_current_narrowed(target_name, updated, declared);
}

/// tsc's `getBaseTypeOfLiteralType`.
fn base_type_of_literal(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Reference(reference) if reference.enum_base.is_some() => {
            reference.enum_base.as_deref().cloned().unwrap_or_else(|| ty.clone())
        }
        Type::Union(union) => union_type(union.types().iter().map(base_type_of_literal).collect()),
        other => other.clone(),
    }
}

fn widen_to_declared(target_name: &str, scopes: &mut ScopeStack) {
    let Some(symbol) = scopes.resolve(target_name) else {
        return;
    };
    let Some(declared) = scopes.visible_symbols().declared_type(target_name).cloned() else {
        return;
    };
    if symbol.ty == declared {
        return;
    }
    let updated = SymbolInfo {
        ty: declared.clone(),
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    scopes.insert_current_narrowed(target_name, updated, declared);
}

fn forget_narrowing(target_name: &str, scopes: &mut ScopeStack) {
    let Some(symbol) = scopes.resolve(target_name) else {
        return;
    };
    let Some(declared) = scopes.visible_symbols().declared_type(target_name).cloned() else {
        return;
    };
    // Holding the declaration (a loop head joins it in) is a narrowing too:
    // its `null` is not what the degraded write left.
    if symbol.ty.is_unmodelled() {
        return;
    }
    let updated = SymbolInfo {
        ty: Type::Unknown,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    scopes.insert_current_narrowed(target_name, updated, declared);
}

pub(crate) fn update_assigned_symbol_type(
    target_name: &str,
    inferred_value: InferredExpression,
    scopes: &mut ScopeStack,
) {
    let assigned = match &inferred_value {
        InferredExpression::Known(ty) if !ty.is_unmodelled() => Some(ty.clone()),
        _ => None,
    };
    note_assigned_type_for_substituting_reads(target_name, assigned, scopes);
    let InferredExpression::Known(value_ty) = inferred_value else {
        // The back edge carries whatever the write left, which surge could not
        // type either; the binding must not keep its entry type at the head.
        if super::branch_assignments::in_loop_prepass() {
            widen_to_declared(target_name, scopes);
        }
        return;
    };
    if value_ty.is_unmodelled() {
        // Inside a loop pre-pass the back edge must not claim the binding
        // still holds what it held on entry. Elsewhere tsc narrows to what
        // was assigned (`getAssignmentReducedType`), which surge cannot tell:
        // a binding narrowed before the write (`let p: P | null = null; p ??=
        // make()`) no longer holds that narrowing, and is left unmodelled.
        if super::branch_assignments::in_loop_prepass() {
            widen_to_declared(target_name, scopes);
        } else {
            forget_narrowing(target_name, scopes);
        }
        return;
    }

    let Some(symbol) = scopes.resolve(target_name) else {
        return;
    };

    // A binding whose *declaration* is a union the value inhabits re-narrows to
    // that value, whatever it is narrowed to right now. Checked up front so the
    // `undefined` arm below cannot claim an annotated binding: `let ctx:
    // undefined | Ctx = undefined; ctx = make();` is `Ctx` afterwards, not the
    // declared union again, and every later use of it was reading the union.
    // Only an *un-annotated* `let x = undefined` widens.
    let declared_union_admits_value = scopes
        .visible_symbols()
        .declared_type(target_name)
        .is_some_and(|declared| {
            matches!(declared.peeled(), Type::Union(_)) && is_assignable_to(&value_ty, declared)
        });
    let mut narrowed_by_assignment = false;
    let has_declared_type = scopes
        .visible_symbols()
        .declared_type(target_name)
        .is_some();
    let updated_ty =
        if symbol.ty == Type::Undefined && !declared_union_admits_value && !has_declared_type {
            union_type(vec![
                Type::Undefined,
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || value_ty.clone()),
            ])
        } else if symbol.ty == Type::Undefined {
            narrowed_by_assignment = true;
            assignment_reduced_type(
                scopes.visible_symbols().declared_type(target_name),
                value_ty,
            )
        } else if symbol.ty == value_ty || is_assignable_to(&value_ty, &symbol.ty) {
            // Assigning to a union-declared variable narrows it to what was
            // assigned, as tsc does: the lazy-singleton idiom
            // (`let client: Redis | null = null; … client = new Redis(); return client;`)
            // otherwise keeps reading as the full union at every later use.
            if matches!(symbol.ty.peeled(), Type::Union(_)) && !value_ty.is_unmodelled() {
                narrowed_by_assignment = true;
                let declared = scopes
                    .visible_symbols()
                    .declared_type(target_name)
                    .unwrap_or(&symbol.ty);
                assignment_reduced_type(Some(declared), value_ty)
            } else if matches!(symbol.ty.peeled(), Type::Union(_)) && value_ty.is_unmodelled() {
                // Keeping the whole declaration for a partly degraded value
                // would report the members (`null`) its modelled part rules
                // out, which an `any` write does not; the reduction decides by
                // the modelled members and keeps a wholly degraded value.
                let declared = scopes
                    .visible_symbols()
                    .declared_type(target_name)
                    .unwrap_or(&symbol.ty);
                assignment_reduced_type(Some(declared), value_ty)
            } else if matches!(value_ty, Type::Any) && declared_union_admits_value {
                // An `any` write undoes an earlier narrowing to one member.
                narrowed_by_assignment = true;
                assignment_reduced_type(scopes.visible_symbols().declared_type(target_name), value_ty)
            } else {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || symbol.ty.clone())
            }
        } else if matches!(
            symbol.ty,
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_)
        )
            // A body's type variable is a declared type: tsc narrows only a
            // union-declared binding on assignment (`getTypeAtFlowAssignment`),
            // so a write it rejects leaves the binding its declaration.
            && !symbol.ty.is_type_variable()
        {
            union_type(vec![
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || symbol.ty.clone()),
                value_ty,
            ])
        } else if scopes
            .visible_symbols()
            .declared_type(target_name)
            .is_some_and(|declared| {
                matches!(declared.peeled(), Type::Union(_)) && is_assignable_to(&value_ty, declared)
            })
        {
            // The binding is already narrowed (by its initializer, or by an earlier
            // assignment) to something this value does not inhabit. The assignment is
            // still legal against the *declaration*, and it re-narrows to the new
            // value — `let s: Wide = "a"; s = "c";` is `"c"`, not a rejected write.
            narrowed_by_assignment = true;
            assignment_reduced_type(
                scopes.visible_symbols().declared_type(target_name),
                value_ty,
            )
        } else if let Some(reduced) = assignment_reduced_declared_type(
            scopes.visible_symbols().declared_type(target_name),
            &symbol.ty,
            &value_ty,
        ) {
            narrowed_by_assignment = true;
            reduced
        } else {
            // Preserve the declared/inferred symbol type when an incompatible assignment
            // is already reported to avoid cascading return/usage diagnostics.
            with_type_copy_reason(TypeCopyReason::ScopeOrContext, || symbol.ty.clone())
        };

    if updated_ty == symbol.ty {
        return;
    }

    let updated = SymbolInfo {
        ty: updated_ty,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };

    if narrowed_by_assignment {
        // Assignment narrowing is block-scoped: written into the current frame it
        // is discarded when a branch scope pops, so `if (t === "draft-4") t = "draft-04";`
        // leaves the declared union in place for the code that follows. The
        // declaration rides along, or a binding captured from an enclosing
        // function (an IIFE body) would read as undeclared afterwards and the
        // next write would be checked against this narrowing.
        let declared = scopes
            .visible_symbols()
            .declared_type(target_name)
            .cloned()
            .unwrap_or_else(|| symbol.ty.clone());
        scopes.insert_current_narrowed(target_name, updated, declared);
        return;
    }

    let _ = scopes.update_visible(target_name, updated);
}

/// The scope entry holding what the last assignment stored in `name`.
fn substituting_read_name(name: &str) -> String {
    format!("\u{0}assigned\u{0}{name}")
}

/// tsc reads a union-constrained variable under a non-generic contextual type
/// as its constraint (`getNarrowableTypeForReference`), which the flow then
/// reduces by the last assignment. The binding itself stays the variable, as
/// every other read sees it, so the assigned type rides beside it in the same
/// frame; a write surge could not type leaves the declaration there.
fn note_assigned_type_for_substituting_reads(target_name: &str, assigned: Option<Type>, scopes: &mut ScopeStack) {
    let declared = scopes
        .visible_symbols()
        .declared_type(target_name)
        .cloned()
        .or_else(|| scopes.resolve(target_name).map(|symbol| symbol.ty.clone()));
    let Some(declared) = declared else {
        return;
    };
    if crate::checks::function::union_constrained_variable(&declared).is_none() {
        return;
    }
    let assigned = assigned.unwrap_or_else(|| declared.clone());
    let _ = scopes.insert_current_narrowed(
        substituting_read_name(target_name),
        SymbolInfo {
            ty: assigned.clone(),
            kind: SymbolKind::Const,
            function_signature: None,
        },
        assigned,
    );
}

/// The scope a bare read of a union-constrained variable is checked in under a
/// non-generic contextual type: the variable's constraint reduced by the last
/// assignment (`getAssignmentReducedType`). `None` where no assignment was
/// recorded, or a guard has narrowed the binding since.
pub(crate) fn constraint_substituted_read_symbols(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<SymbolTable> {
    let ParsedExpression::Identifier { name, .. } = expression else {
        return None;
    };
    let assigned = symbols.get(&substituting_read_name(name))?;
    let symbol = symbols.get(name)?;
    let declared = symbols.declared_type(name).unwrap_or(&symbol.ty);
    if symbol.ty != *declared {
        return None;
    }
    let (_, constraint) = crate::checks::function::union_constrained_variable(declared)?;
    let reduced = assignment_reduced_type(Some(&constraint), assigned.ty.clone());
    let mut substituted = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    let _ = substituted.insert_narrowed(
        name.as_str(),
        SymbolInfo {
            ty: reduced,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        },
        declared.clone(),
    );
    Some(substituted)
}

/// The body of a non-async, non-generator function called where it is
/// written (`(function () { … })()`, `(() => { … })()`).
fn immediately_invoked_body(
    expression: &ParsedExpression,
) -> Option<&surge_ts_syntax::ParsedArrowFunction> {
    let ParsedExpression::ExpressionCall { callee, .. } = expression else {
        return None;
    };
    let ParsedExpression::ArrowFunction(function) = callee.as_ref() else {
        return None;
    };
    (!function.is_async && !function.is_generator).then_some(function.as_ref())
}

/// What an immediately invoked body assigns to the bindings it does not
/// declare itself: each straight-line write before the body can leave, which
/// holds once the call returns, and every other write, which may or may not
/// have run.
#[derive(Default)]
pub(crate) struct ImmediatelyInvokedAssignments {
    certain: Vec<ParsedExpression>,
    possible: Vec<ParsedExpression>,
}

fn immediately_invoked_assignments(
    function: &surge_ts_syntax::ParsedArrowFunction,
) -> Option<ImmediatelyInvokedAssignments> {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;

    let surge_ts_syntax::ParsedArrowFunctionBody::Block(body) = &function.body else {
        return None;
    };
    fn declared(statements: &[Statement], names: &mut Vec<String>) {
        for statement in statements {
            match statement {
                Statement::VariableDeclaration(variable) => names.push(variable.name.clone()),
                Statement::Function(function) => names.push(function.name.clone()),
                Statement::Class(class) => names.push(class.name.clone()),
                Statement::Block(block) => declared(block, names),
                Statement::If(statement) => {
                    declared(&statement.then_body, names);
                    declared(&statement.else_body, names);
                }
                Statement::While(statement) => declared(&statement.body, names),
                Statement::ForOf(statement) => declared(&statement.body, names),
                Statement::Switch(statement) => {
                    for case in &statement.cases {
                        declared(&case.consequent, names);
                    }
                }
                Statement::Try(statement) => {
                    declared(&statement.block, names);
                    if let Some(handler) = &statement.handler {
                        declared(&handler.body, names);
                    }
                    declared(&statement.finalizer, names);
                }
                _ => {}
            }
        }
    }
    fn writes(statements: &[Statement], out: &mut Vec<ParsedExpression>) {
        for statement in statements {
            match statement {
                Statement::Assignment(assignment) => out.push(assignment_expression(assignment)),
                Statement::Block(block) => writes(block, out),
                Statement::If(statement) => {
                    writes(&statement.then_body, out);
                    writes(&statement.else_body, out);
                }
                Statement::While(statement) => writes(&statement.body, out),
                Statement::ForOf(statement) => writes(&statement.body, out),
                Statement::Switch(statement) => {
                    for case in &statement.cases {
                        writes(&case.consequent, out);
                    }
                }
                Statement::Try(statement) => {
                    writes(&statement.block, out);
                    if let Some(handler) = &statement.handler {
                        writes(&handler.body, out);
                    }
                    writes(&statement.finalizer, out);
                }
                _ => {}
            }
        }
    }
    fn assignment_expression(assignment: &ParsedAssignment) -> ParsedExpression {
        ParsedExpression::Assignment {
            target_name: assignment.target_name.clone(),
            target_span: assignment.target_span,
            value: Box::new(assignment.value.clone()),
            value_span: assignment.value_span,
        }
    }

    let mut own = Vec::new();
    for parameter in &function.parameters {
        own.extend(parameter.binding_name.bound_names().into_iter().map(|bound| bound.name));
    }
    declared(body, &mut own);
    let mut assignments = ImmediatelyInvokedAssignments::default();
    let mut leaves = false;
    for statement in body {
        match statement {
            Statement::Assignment(assignment) if !leaves => {
                assignments.certain.push(assignment_expression(assignment));
            }
            Statement::Return(_) | Statement::Throw(_) => leaves = true,
            other => writes(std::slice::from_ref(other), &mut assignments.possible),
        }
    }
    let foreign = |assignment: &ParsedExpression| {
        matches!(assignment, ParsedExpression::Assignment { target_name, .. } if !own.contains(target_name))
    };
    assignments.certain.retain(foreign);
    assignments.possible.retain(foreign);
    Some(assignments)
}

/// tsc binds a non-async, non-generator IIFE inline (`bindContainer`), so
/// once the call returns its writes are the flow the caller continues from.
/// A write that certainly ran narrows its target as an assignment statement
/// does; one that may not have joins what the target held before.
impl ImmediatelyInvokedAssignments {
    pub(crate) fn target_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for assignment in self.certain.iter().chain(&self.possible) {
            if let ParsedExpression::Assignment { target_name, .. } = assignment
                && !names.contains(target_name)
            {
                names.push(target_name.clone());
            }
        }
        names
    }
}

pub(crate) fn immediately_invoked_writes(expression: &ParsedExpression) -> Option<ImmediatelyInvokedAssignments> {
    immediately_invoked_body(expression)
        .and_then(immediately_invoked_assignments)
        .filter(|assignments| !assignments.certain.is_empty() || !assignments.possible.is_empty())
}

pub(crate) fn apply_immediately_invoked_assignments(
    assignments: ImmediatelyInvokedAssignments,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let possible_targets: Vec<&str> = assignments
        .possible
        .iter()
        .filter_map(|assignment| match assignment {
            ParsedExpression::Assignment { target_name, .. } => Some(target_name.as_str()),
            _ => None,
        })
        .collect();
    for assignment in &assignments.certain {
        let ParsedExpression::Assignment { target_name, .. } = assignment else {
            continue;
        };
        apply_immediately_invoked_assignment(assignment, false, scopes, ctx);
        if !possible_targets.contains(&target_name.as_str()) {
            mark_assignment_state(target_name, flow_state);
        }
    }
    for assignment in &assignments.possible {
        apply_immediately_invoked_assignment(assignment, true, scopes, ctx);
    }
}

/// A write the outer scope cannot type (it reads the body's own bindings)
/// leaves the target holding its declared type.
fn apply_immediately_invoked_assignment(
    assignment: &ParsedExpression,
    conditional: bool,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let ParsedExpression::Assignment { target_name, value, .. } = assignment else {
        return;
    };
    let typed = matches!(
        crate::infer::infer_expression(value, visible_symbols(scopes), ctx),
        InferredExpression::Known(ref ty) if !ty.is_unmodelled()
    );
    if typed {
        apply_assignment_expression(assignment, conditional, None, scopes, ctx);
    } else {
        widen_to_declared(target_name, scopes);
    }
}
