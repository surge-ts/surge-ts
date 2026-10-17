//! The elaboration tsgo chains beneath a failed relation's head message: the
//! `reportError` calls the relater makes on its way out of a failure
//! (relater.go `isRelatedToEx`, `reportErrorResults`, `reportRelationError`,
//! `propertiesRelatedTo`, `compareSignaturesRelated`, `indexInfoRelatedTo`).
//!
//! surge's relation answers yes or no, so the failure path is replayed here
//! over the same types: at each level the first constituent, argument,
//! property, parameter or index signature the relation rejects is the one tsgo
//! reports, in tsgo's order. A shape this replay does not model ends the chain
//! where it is, which leaves the head exactly as it was before.

use surge_ts_types::{FunctionType, ObjectType, Type, is_assignable_to};

/// How deep a replay goes; tsgo's own depth limit is far beyond any message
/// it would print.
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    /// `Type '{0}' is not assignable to type '{1}'.` for this source/target.
    Relation,
    /// A missing-property report for this source/target display pair, which
    /// suppresses the relation line above it (`reportRelationError`).
    MissingProperty { source: String, target: String },
    /// `Types of property '{0}' are incompatible.`
    TypesOfProperty(String),
    /// `The types of '{0}' are incompatible between these types.`
    TypesOf(String),
    /// `The types returned by '{0}' are incompatible between these types.`
    TypesReturnedBy(String),
    /// tsgo's return-type markers (`Call_signature_return_types_0_and_1_are_incompatible`
    /// and its siblings): elided from the printed chain, they only drive the
    /// `TypesOfProperty` rewrite.
    ReturnMarker { no_arguments: bool, construct: bool },
    Other,
}

#[derive(Clone, Debug)]
struct Entry {
    kind: Kind,
    text: String,
}

/// The chain tsgo prints beneath the head of a failed relation of `source` to
/// `target`, outermost line first, each line carrying its own indentation
/// relative to the head (one level is two spaces).
pub(crate) fn chain_beneath_head(source: &Type, target: &Type) -> Vec<String> {
    let mut elaborator = Elaborator::new(false);
    let target = crate::checks::expr::reported_relation_target(source, target);
    elaborator.in_progress.push((source.name(), target.name()));
    elaborator.beneath(source, &target);
    elaborator.render()
}

/// [`chain_beneath_head`] under the comparable relation (`checkTypeComparableTo`).
pub(crate) fn comparable_chain_beneath_head(source: &Type, target: &Type) -> Vec<String> {
    let mut elaborator = Elaborator::new(true);
    elaborator.beneath(source, target);
    elaborator.render()
}

/// The chain of a relation tsgo reports without a head message of its own
/// beneath some other root: the relation line itself, then what it chains.
pub(crate) fn relation_chain(source: &Type, target: &Type) -> Vec<String> {
    let mut elaborator = Elaborator::new(false);
    elaborator.explain(source, target);
    elaborator.render()
}

/// `diagnostic` with `chain` printed beneath its message.
pub(crate) fn with_chain(
    mut diagnostic: surge_ts_diagnostics::Diagnostic,
    chain: &[String],
) -> surge_ts_diagnostics::Diagnostic {
    for line in chain {
        diagnostic.message.push('\n');
        diagnostic.message.push_str(line);
    }
    diagnostic
}

struct Elaborator {
    /// tsgo's `errorChain`, outermost entry first.
    chain: Vec<Entry>,
    depth: usize,
    strict_function_types: bool,
    comparable: bool,
    /// The pairs being explained, by display: one met again is assumed
    /// related, as tsgo's relation assumes a pair it is already comparing.
    in_progress: Vec<(String, String)>,
}

impl Elaborator {
    fn new(comparable: bool) -> Self {
        Elaborator {
            chain: Vec::new(),
            depth: 0,
            strict_function_types: surge_ts_types::strict_function_types(),
            comparable,
            in_progress: Vec::new(),
        }
    }

    fn render(&self) -> Vec<String> {
        self.chain
            .iter()
            .filter(|entry| !matches!(entry.kind, Kind::ReturnMarker { .. }))
            .enumerate()
            .map(|(level, entry)| format!("{}{}", "  ".repeat(level + 1), unmark(&entry.text)))
            .collect()
    }

    fn top(&self, index: usize) -> Option<&Kind> {
        self.chain.get(index).map(|entry| &entry.kind)
    }

    /// relater.go `reportError`, with its rewrites of a property
    /// incompatibility over a return-type marker or another property.
    fn report(&mut self, kind: Kind, text: String) {
        let Kind::TypesOfProperty(name) = &kind else {
            let text = match &kind {
                Kind::TypesOf(_) | Kind::TypesReturnedBy(_) => render_property_kind(&kind),
                _ => text,
            };
            self.chain.insert(0, Entry { kind, text });
            return;
        };
        let mut kind = kind.clone();
        let mut arg = name.clone();
        if let Some(Kind::ReturnMarker { no_arguments, construct }) = self.top(1).cloned() {
            let base = property_name_arg(&arg);
            arg = match (construct, no_arguments) {
                (false, true) => format!("{base}()"),
                (true, true) => format!("new {base}()"),
                (false, false) => format!("{base}(...)"),
                (true, false) => format!("new {base}(...)"),
            };
            kind = Kind::TypesReturnedBy(arg.clone());
            self.chain.drain(0..2);
        }
        if let Some(Kind::TypesOfProperty(tail) | Kind::TypesOf(tail) | Kind::TypesReturnedBy(tail)) =
            self.top(1).cloned()
        {
            let dotted = add_to_dotted_name(&property_name_arg(&arg), &property_name_arg(&tail));
            self.chain.drain(0..2);
            let next = match kind {
                Kind::TypesReturnedBy(_) => Kind::TypesReturnedBy(dotted),
                _ => Kind::TypesOf(dotted),
            };
            self.report(next, String::new());
            return;
        }
        let text = render_property_kind(&kind);
        self.chain.insert(0, Entry { kind, text });
    }

    /// A pair already being explained, or a member signature pair whose
    /// returns are one: tsgo's relation assumes it related (`Maybe`).
    fn assumed_related(&self, source: &Type, target: &Type) -> bool {
        let pending = |source: &Type, target: &Type| {
            let pair = (source.name(), target.name());
            self.in_progress.iter().any(|entry| *entry == pair)
        };
        if pending(source, target) {
            return true;
        }
        match (single_signature(source), single_signature(target)) {
            (Some(source), Some(target)) => pending(source.return_type(), target.return_type()),
            _ => false,
        }
    }

    /// `isRelatedTo` with `reportErrors`, for a pair the relation rejects.
    fn explain(&mut self, source: &Type, target: &Type) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        let target = crate::checks::expr::reported_relation_target(source, target);
        self.in_progress.push((source.name(), target.name()));
        self.beneath(source, &target);
        self.in_progress.pop();
        self.relation_error(source, &target);
        self.depth -= 1;
    }

    /// Everything tsgo reports beneath the relation line of this pair: the
    /// structural elaboration, then `reportErrorResults`' own hints. The
    /// type-variable line `reportRelationError` adds belongs here too, since
    /// it sits between the relation line and the structure.
    fn beneath(&mut self, source: &Type, target: &Type) {
        self.structural(source, target);
        if let Some(primitive) = wrapper_primitive(source)
            && primitive == *target
        {
            self.report(
                Kind::Other,
                format!(
                    "'{}' is a primitive, but '{}' is a wrapper object. Prefer using '{}' when possible.",
                    target.name(),
                    source.name(),
                    target.name()
                ),
            );
        } else if is_global_interface(source, "Object") && !is_primitive_like(target) {
            self.report(
                Kind::Other,
                "The 'Object' type is assignable to very few other types. Did you mean to use the 'any' type instead?"
                    .to_string(),
            );
        }
        self.type_variable_line(source, target);
    }

    /// The first half of `reportRelationError`: a target that is a type
    /// variable explains why the source cannot be relied on to fit it.
    fn type_variable_line(&mut self, source: &Type, target: &Type) {
        let Some((target_name, constraint)) = type_variable_target(target).or_else(|| {
            // An indexed access over a type parameter reports like the
            // parameter, unless the source is an indexed access too.
            if indexed_access(source).is_some() {
                return None;
            }
            let (object, _) = indexed_access(target)?;
            let Type::TypeParameter(object) = object else {
                return None;
            };
            surge_ts_types::type_variable::deferred_type(&object).is_none().then_some(())?;
            let Type::TypeParameter(parameter) = target else {
                return None;
            };
            let constraint = surge_ts_types::type_variable::active_constraint(parameter)?;
            Some((target.name(), constraint.filter(|constraint| !constraint.is_unknown())))
        }) else {
            return;
        };
        let source_name = crate::checks::expr::source_display_name(source, target);
        let generalized = generalized_source(source, target);
        match constraint {
            Some(constraint) if is_assignable_to(&generalized, &constraint) => self.report(
                Kind::Other,
                format!(
                    "'{source_name}' is assignable to the constraint of type '{target_name}', but '{target_name}' could be instantiated with a different subtype of constraint '{}'.",
                    constraint.name()
                ),
            ),
            Some(constraint) if is_assignable_to(source, &constraint) => self.report(
                Kind::Other,
                format!(
                    "'{}' is assignable to the constraint of type '{target_name}', but '{target_name}' could be instantiated with a different subtype of constraint '{}'.",
                    source.name(),
                    constraint.name()
                ),
            ),
            _ => {
                // "Only report this error once": the elaboration beneath goes.
                self.chain.clear();
                self.report(
                    Kind::Other,
                    format!(
                        "'{target_name}' could be instantiated with an arbitrary type which could be unrelated to '{source_name}'."
                    ),
                );
            }
        }
    }

    /// The second half of `reportRelationError`: the relation line itself,
    /// unless the report beneath already names this pair as missing members.
    fn relation_error(&mut self, source: &Type, target: &Type) {
        let source_name = crate::checks::expr::source_display_name(source, target);
        let target_name = target.name();
        if let Some(Kind::MissingProperty { source: missing_source, target: missing_target }) = self.top(0)
            && *missing_source == source_name
            && *missing_target == target_name
        {
            return;
        }
        let text = if self.comparable {
            format!("Type '{source_name}' is not comparable to type '{target_name}'.")
        } else if unmark(&source_name) == unmark(&target_name) {
            format!(
                "Type '{source_name}' is not assignable to type '{target_name}'. Two different types with this name exist, but they are unrelated."
            )
        } else {
            format!("Type '{source_name}' is not assignable to type '{target_name}'.")
        };
        self.report(Kind::Relation, text);
    }

    /// `unionOrIntersectionRelatedTo` and `structuredTypeRelatedTo`, as far as
    /// they report.
    fn structural(&mut self, source: &Type, target: &Type) {
        // A string mapping relates its operand to another of the same kind,
        // and its base constraint to anything but a mapping; template
        // literal relations report nothing beneath the relation line.
        match (string_mapping(source), string_mapping(target)) {
            (Some((source_kind, source_inner)), Some((target_kind, target_inner))) => {
                if source_kind == target_kind && !is_assignable_to(&source_inner, &target_inner) {
                    self.explain(&source_inner, &target_inner);
                }
                return;
            }
            (Some(_), None) => {
                if !is_assignable_to(&Type::String, target) {
                    self.explain(&Type::String, target);
                }
                return;
            }
            _ => {}
        }
        if surge_ts_types::is_template_literal_type(source) || surge_ts_types::is_template_literal_type(target) {
            return;
        }
        if let (Some(source_enum), Some(target_enum)) = (enum_declaration(source), enum_declaration(target)) {
            if let Some(line) = enum_mismatch(&source_enum, &target_enum) {
                self.report(Kind::Other, line);
            }
            return;
        }
        // `S[K]` to `T[J]` relates the object types, then the index types.
        if let (Some((source_object, source_index)), Some((target_object, target_index))) =
            (indexed_access(source), indexed_access(target))
        {
            if !is_assignable_to(&source_object, &target_object) {
                self.explain(&source_object, &target_object);
            } else if !is_assignable_to(&source_index, &target_index) {
                self.explain(&source_index, &target_index);
            }
            return;
        }
        // Two instantiations of one alias relate through its arguments.
        if let (Some((source_id, source_arguments, variances)), Some((target_id, target_arguments, _))) =
            (alias_instantiation(source), alias_instantiation(target))
            && source_id == target_id
            && source_arguments.len() == target_arguments.len()
        {
            self.type_arguments(&source_arguments, &target_arguments, variances);
            return;
        }
        if let Type::Union(union) = source {
            // A union tsgo flags primitive (`boolean`, an enum) is not
            // elaborated member by member.
            if is_enum_union(source) {
                return;
            }
            // The comparable relation needs one member related, and reports
            // against the last (`someTypeRelatedToType`).
            let member = if self.comparable {
                tsgo_order(union.types()).pop()
            } else {
                tsgo_order(union.types()).into_iter().find(|member| !is_assignable_to(member, target))
            };
            if let Some(member) = member {
                self.explain(&member, target);
            }
            return;
        }
        // A constrained type parameter matches no member of a union target;
        // it relates through its constraint below (`structuredTypeRelatedTo`).
        let constrained_type_parameter = matches!(source, Type::TypeParameter(parameter)
            if surge_ts_types::type_variable::deferred_type(parameter).is_none()
                && matches!(surge_ts_types::type_variable::active_constraint(parameter), Some(Some(_))))
            || indexed_access_constraint_step(source).is_some();
        if let Type::Union(union) = target
            && !constrained_type_parameter
        {
            if is_primitive_like(source) || is_enum_union(target) {
                return;
            }
            if let Some(best) = best_matching_member(source, union.types()) {
                self.explain(source, &best);
            }
            return;
        }
        if let Some(operands) = intersection_operands(target) {
            if let Some(operand) = operands.iter().find(|operand| !is_assignable_to(source, operand)) {
                let operand = operand.clone();
                self.explain(source, &operand);
            }
            return;
        }
        // `keyof T` relates as `string | number | symbol`, `S[K]` through its
        // constraint.
        if let Type::TypeParameter(parameter) = source
            && matches!(
                surge_ts_types::type_variable::deferred_type(parameter),
                Some(
                    surge_ts_types::type_variable::DeferredType::Keyof(_)
                        | surge_ts_types::type_variable::DeferredType::IndexedAccess { .. }
                )
            )
        {
            // The stepped constraint fails wherever its source did; surge's
            // relation of a freshly stepped variable is no guide.
            if let Some(constraint) = indexed_access_constraint_step(source) {
                self.explain(&constraint, target);
            } else if let Some(Some(constraint)) = surge_ts_types::type_variable::active_constraint(parameter)
                && !constraint.is_unknown()
                && !is_assignable_to(&constraint, target)
            {
                self.explain(&constraint, target);
            }
            return;
        }
        if let Type::TypeParameter(parameter) = source
            && surge_ts_types::type_variable::deferred_type(parameter).is_none()
            && let Some(Some(constraint)) = surge_ts_types::type_variable::active_constraint(parameter)
        {
            if !target.is_type_variable() && !constraint.is_unknown() && !is_assignable_to(&constraint, target) {
                self.explain(&constraint, target);
            }
            return;
        }
        match (source, target) {
            (Type::Array(source_element), Type::Array(target_element)) => {
                if !is_assignable_to(source_element, target_element) {
                    self.explain(source_element, target_element);
                }
                return;
            }
            (Type::Tuple(elements), Type::Array(target_element)) => {
                let element = surge_ts_types::union_type(elements.clone());
                if !is_assignable_to(&element, target_element) {
                    self.explain(&element, target_element);
                }
                return;
            }
            (Type::Tuple(source_elements), Type::Tuple(target_elements)) => {
                self.tuple(source_elements, target_elements);
                return;
            }
            (Type::Tuple(_) | Type::OpenTuple(_) | Type::Array(_), Type::OpenTuple(_)) | (Type::OpenTuple(_), Type::Tuple(_)) => {
                if let (Some(sources), Some(targets)) = (tuple_elements(source), tuple_elements(target)) {
                    self.variable_tuple(&sources, &targets);
                }
                return;
            }
            (Type::Array(_), Type::Tuple(target_elements)) => {
                // An array has a rest element and a minimum length of zero.
                if !target_elements.is_empty() {
                    self.report(
                        Kind::Other,
                        format!("Target requires {} element(s) but source may have fewer.", target_elements.len()),
                    );
                } else {
                    self.report(Kind::Other, "Target allows only 0 element(s) but source may have more.".to_string());
                }
                return;
            }
            (Type::Reference(source_reference), Type::Reference(target_reference))
                if source_reference.id == target_reference.id
                    && source_reference.arguments.len() == target_reference.arguments.len()
                    && !source_reference.arguments.is_empty() =>
            {
                self.type_arguments(
                    &source_reference.arguments,
                    &target_reference.arguments,
                    source_reference.declared_variances,
                );
                return;
            }
            _ => {}
        }
        if is_primitive_like(source) {
            return;
        }
        let peeled_source = source.peeled();
        let peeled_target = target.peeled();
        // A reference to a tuple or array type relates as what it names.
        if (matches!(source, Type::Reference(_)) || matches!(target, Type::Reference(_)))
            && matches!(peeled_source, Type::Array(_) | Type::Tuple(_))
            && matches!(peeled_target, Type::Array(_) | Type::Tuple(_))
        {
            self.structural(&peeled_source, &peeled_target);
            return;
        }
        self.members(source, target, &peeled_source, &peeled_target);
    }

    /// `propertiesRelatedTo`'s tuple branch, for two fixed-length tuples.
    fn tuple(&mut self, sources: &[Type], targets: &[Type]) {
        let (source_arity, target_arity) = (sources.len(), targets.len());
        if source_arity < target_arity {
            self.report(
                Kind::Other,
                format!("Source has {source_arity} element(s) but target requires {target_arity}."),
            );
            return;
        }
        if target_arity < source_arity {
            self.report(
                Kind::Other,
                format!("Source has {source_arity} element(s) but target allows only {target_arity}."),
            );
            return;
        }
        for (position, (source, target)) in sources.iter().zip(targets).enumerate() {
            if !is_assignable_to(source, target) {
                self.explain(source, target);
                if target_arity > 1 || source_arity > 1 {
                    self.report(
                        Kind::Other,
                        format!(
                            "Type at position {position} in source is not compatible with type at position {position} in target."
                        ),
                    );
                }
                return;
            }
        }
    }

    /// `propertiesRelatedTo`'s tuple branch where either side has a rest
    /// element (an array source is one rest element).
    fn variable_tuple(&mut self, sources: &[(Type, bool)], targets: &[(Type, bool)]) {
        let (source_arity, target_arity) = (sources.len(), targets.len());
        let source_rest = sources.iter().any(|(_, rest)| *rest);
        let target_rest = targets.iter().any(|(_, rest)| *rest);
        let source_min = sources.iter().filter(|(_, rest)| !rest).count();
        let target_min = targets.iter().filter(|(_, rest)| !rest).count();
        if !source_rest && source_arity < target_min {
            self.report(Kind::Other, format!("Source has {source_arity} element(s) but target requires {target_min}."));
            return;
        }
        if !target_rest && target_arity < source_min {
            self.report(Kind::Other, format!("Source has {source_min} element(s) but target allows only {target_arity}."));
            return;
        }
        if !target_rest && (source_rest || target_arity < source_arity) {
            let line = if source_min < target_min {
                format!("Target requires {target_min} element(s) but source may have fewer.")
            } else {
                format!("Target allows only {target_arity} element(s) but source may have more.")
            };
            self.report(Kind::Other, line);
            return;
        }
        let target_start = targets.iter().take_while(|(_, rest)| !rest).count();
        let target_end = targets.iter().rev().take_while(|(_, rest)| !rest).count();
        for (source_position, (source_type, source_is_rest)) in sources.iter().enumerate() {
            let from_end = source_arity - 1 - source_position;
            let target_position = if target_rest && source_position >= target_start {
                target_arity - 1 - from_end.min(target_end)
            } else {
                source_position
            };
            let Some((target_type, target_is_rest)) = targets.get(target_position) else {
                return;
            };
            if !target_is_rest && *source_is_rest {
                self.report(
                    Kind::Other,
                    format!("Source provides no match for required element at position {target_position} in target."),
                );
                return;
            }
            if is_assignable_to(source_type, target_type) {
                continue;
            }
            self.explain(source_type, target_type);
            if target_arity > 1 || source_arity > 1 {
                let last = (source_arity - target_end).checked_sub(1);
                let line = match last {
                    Some(last)
                        if target_rest
                            && source_position >= target_start
                            && from_end >= target_end
                            && target_start != last =>
                    {
                        format!(
                            "Type at positions {target_start} through {last} in source is not compatible with type at position {target_position} in target."
                        )
                    }
                    _ => format!(
                        "Type at position {source_position} in source is not compatible with type at position {target_position} in target."
                    ),
                };
                self.report(Kind::Other, line);
            }
            return;
        }
    }

    /// `typeArgumentsRelatedTo`: the first argument the declared variance
    /// rejects, related in that variance's direction.
    fn type_arguments(&mut self, sources: &[Type], targets: &[Type], variances: u32) {
        use surge_ts_types::DeclaredVariance as Variance;
        for (index, (source, target)) in sources.iter().zip(targets).enumerate() {
            match surge_ts_types::declared_variance(variances, index) {
                None | Some(Variance::Covariant) => {
                    if !is_assignable_to(source, target) {
                        self.explain(source, target);
                        return;
                    }
                }
                Some(Variance::Contravariant | Variance::UnreliableContravariant | Variance::MeasuredContravariant) => {
                    if !is_assignable_to(target, source) {
                        self.explain(target, source);
                        return;
                    }
                }
                // Compared covariantly first, then contravariantly.
                Some(Variance::Invariant | Variance::MeasuredInvariant) => {
                    if !is_assignable_to(source, target) {
                        self.explain(source, target);
                        return;
                    }
                    if !is_assignable_to(target, source) {
                        self.explain(target, source);
                        return;
                    }
                }
                // Unmeasurable arguments fall back to the structure, which
                // this replay does not follow.
                Some(_) => {
                    if source != target {
                        return;
                    }
                }
            }
        }
    }

    /// `propertiesRelatedTo`, `signaturesRelatedTo` and
    /// `indexSignaturesRelatedTo`, in that order.
    fn members(&mut self, source: &Type, target: &Type, peeled_source: &Type, peeled_target: &Type) {
        let source_name = crate::checks::expr::source_display_name(source, target);
        let target_name = target.name();
        if let Some(missing) =
            crate::checks::expr::missing_properties_report(source, target, &source_name, &target_name, "")
        {
            self.report(
                Kind::MissingProperty { source: source_name, target: target_name },
                missing.message,
            );
            return;
        }
        // The global `Object` keeps the plain head, with its missing members
        // reported beneath it.
        if is_global_interface(source, "Object")
            && let Type::Object(target_object) = peeled_target
        {
            let missing: Vec<String> = target_object
                .required_properties()
                .filter(|(name, _)| source.get_property_access_type(name).is_none())
                .map(|(name, _)| display_property_name(name))
                .collect();
            match missing.as_slice() {
                [] => {}
                [only] => {
                    self.report(
                        Kind::Other,
                        format!("Property '{only}' is missing in type '{source_name}' but required in type '{target_name}'."),
                    );
                    return;
                }
                several if several.len() > 5 => {
                    self.report(
                        Kind::Other,
                        format!(
                            "Type '{source_name}' is missing the following properties from type '{target_name}': {}, and {} more.",
                            several[..4].join(", "),
                            several.len() - 4
                        ),
                    );
                    return;
                }
                several => {
                    self.report(
                        Kind::Other,
                        format!(
                            "Type '{source_name}' is missing the following properties from type '{target_name}': {}",
                            several.join(", ")
                        ),
                    );
                    return;
                }
            }
        }
        if let Type::Object(target_object) = peeled_target {
            for (name, target_property) in target_object.properties.iter() {
                if target_property.index_slot {
                    continue;
                }
                let shown = target_property.written_name.as_deref().map_or_else(|| display_property_name(name), String::clone);
                let Some((source_property_type, source_optional, source_restriction)) = property_of(peeled_source, name)
                else {
                    continue;
                };
                if let Some(text) = restriction_mismatch(
                    &shown.clone(),
                    source_restriction.as_ref(),
                    target_property.restriction.as_ref(),
                    &source.name(),
                    &target.name(),
                ) {
                    self.report(Kind::Other, text);
                    return;
                }
                if target_property.ty.is_unknown() || matches!(target_property.ty, Type::Any) {
                    continue;
                }
                // An optional property reads `undefined` as well, on either side.
                let with_optionality = |ty: &Type, optional: bool| {
                    if optional
                        && surge_ts_types::strict_null_checks()
                        && !surge_ts_types::exact_optional_property_types()
                        && !nullable(ty)
                    {
                        surge_ts_types::union_type(vec![ty.clone(), Type::Undefined])
                    } else {
                        ty.clone()
                    }
                };
                let source_property_type = with_optionality(&source_property_type, source_optional);
                let target_property_type = &with_optionality(&target_property.ty, target_property.optional);
                if self.assumed_related(&source_property_type, target_property_type) {
                    continue;
                }
                if is_assignable_to(&source_property_type, target_property_type) {
                    if source_optional && !target_property.optional {
                        self.report(
                            Kind::Other,
                            format!(
                                "Property '{}' is optional in type '{}' but required in type '{}'.",
                                shown.clone(),
                                source.name(),
                                target.name()
                            ),
                        );
                        return;
                    }
                    continue;
                }
                {
                    if single_signature(&source_property_type).is_some() && single_signature(target_property_type).is_some() {
                        self.explain_member_function(&source_property_type, target_property_type, target_property.method);
                    } else {
                        self.explain(&source_property_type, target_property_type);
                    }
                    self.report(Kind::TypesOfProperty(shown.clone()), String::new());
                    return;
                }
            }
        }
        let target_call = call_signature(peeled_target);
        if let Some(target_signature) = target_call {
            match call_signature(peeled_source) {
                Some(source_signature) => {
                    self.signatures(&source_signature, &target_signature, false, false);
                    return;
                }
                None => {
                    if let Some(signature) = signature_display(&target_signature, false) {
                        self.report(
                            Kind::Other,
                            format!("Type '{}' provides no match for the signature '{signature}'.", source.name()),
                        );
                    }
                    return;
                }
            }
        }
        if let Some(target_signature) = construct_signature(peeled_target) {
            if let Some(source_signature) = construct_signature(peeled_source) {
                if let Some(line) = construct_modifier_mismatch(&source_signature, &target_signature) {
                    self.report(Kind::Other, line);
                    return;
                }
                self.signatures(&source_signature, &target_signature, true, false);
                return;
            }
            if let Some(signature) = signature_display(&target_signature, true) {
                self.report(
                    Kind::Other,
                    format!("Type '{}' provides no match for the signature '{signature}'.", source.name()),
                );
            }
            return;
        }
        if let Type::Object(target_object) = peeled_target {
            for (key, target_index) in [
                ("string", target_object.string_index_type.as_deref()),
                ("number", target_object.number_index_type.as_deref()),
            ] {
                let Some(target_index) = target_index else {
                    continue;
                };
                if matches!(target_index, Type::Any) && target_object.string_index_type.is_some() {
                    continue;
                }
                if self.index_signature(source, peeled_source, key, target_index) {
                    return;
                }
            }
        }
    }

    /// A property of function type: the relation line over the signature's
    /// elaboration, compared bivariantly when the target is a method.
    fn explain_member_function(&mut self, source: &Type, target: &Type, method: bool) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        if let (Some(source_signature), Some(target_signature)) = (single_signature(source), single_signature(target)) {
            self.signatures(&source_signature, &target_signature, false, method);
        }
        self.relation_error(source, target);
        self.depth -= 1;
    }

    /// `typeRelatedToIndexInfo` for one of the target's index signatures.
    /// `true` when it reported.
    fn index_signature(&mut self, source: &Type, peeled_source: &Type, key: &str, target_index: &Type) -> bool {
        let missing = |source: &Type| format!("Index signature for type '{key}' is missing in type '{}'.", source.name());
        let source_object = match peeled_source {
            Type::Object(source_object) => source_object,
            Type::Function(_) => {
                self.report(Kind::Other, missing(source));
                return true;
            }
            Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) if key == "string" => {
                self.report(Kind::Other, missing(source));
                return true;
            }
            Type::Array(element) => {
                if is_assignable_to(element, target_index) {
                    return false;
                }
                self.explain(element, target_index);
                self.report(Kind::Other, "'number' index signatures are incompatible.".to_string());
                return true;
            }
            _ => return false,
        };
        let source_index = match key {
            "string" => source_object.string_index_type.as_deref(),
            _ => source_object.number_index_type.as_deref().or(source_object.string_index_type.as_deref()),
        };
        if let Some(source_index) = source_index {
            if is_assignable_to(source_index, target_index) {
                return false;
            }
            self.explain(source_index, target_index);
            let source_key = if key == "number" && source_object.number_index_type.is_none() { "string" } else { key };
            let text = if source_key == key {
                format!("'{key}' index signatures are incompatible.")
            } else {
                format!("'{source_key}' and '{key}' index signatures are incompatible.")
            };
            self.report(Kind::Other, text);
            return true;
        }
        if inferable_index(source_object) {
            for (name, property) in source_object.properties.iter() {
                if key == "number" && name.parse::<f64>().is_err() {
                    continue;
                }
                let property_type = if property.optional
                    && surge_ts_types::strict_null_checks()
                    && !surge_ts_types::exact_optional_property_types()
                    && !nullable(&property.ty)
                {
                    surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
                } else {
                    property.ty.clone()
                };
                if !is_assignable_to(&property_type, target_index) {
                    self.explain(&property_type, target_index);
                    let shown = property.written_name.as_deref().map_or_else(|| display_property_name(name), String::clone);
                    self.report(Kind::Other, format!("Property '{shown}' is incompatible with index signature."));
                    return true;
                }
            }
            return false;
        }
        self.report(Kind::Other, missing(source));
        true
    }

    /// `signaturesRelatedTo`: one signature each, or two overload lists of the
    /// same length (instantiations of one declaration), compared pairwise.
    fn signatures(&mut self, source: &FunctionType, target: &FunctionType, construct: bool, method: bool) {
        match (source.overloads(), target.overloads()) {
            (None, None) => self.signature(source, target, construct, method),
            (Some(sources), Some(targets)) if sources.len() == targets.len() => {
                for (source_member, target_member) in sources.iter().zip(targets) {
                    let source_type = Type::Function(source_member.clone());
                    let target_type = Type::Function(target_member.clone());
                    if !is_assignable_to(&source_type, &target_type) {
                        self.signature(source_member, target_member, construct, method);
                        return;
                    }
                }
            }
            (sources, targets) => {
                // The first target signature no source signature matches is
                // elaborated against the first source signature alone.
                let sources: Vec<FunctionType> = sources.map_or_else(|| vec![source.clone()], <[FunctionType]>::to_vec);
                let targets: Vec<FunctionType> = targets.map_or_else(|| vec![target.clone()], <[FunctionType]>::to_vec);
                let unmatched = targets.iter().find(|target_member| {
                    let target_type = Type::Function((*target_member).clone());
                    !sources.iter().any(|source_member| is_assignable_to(&Type::Function(source_member.clone()), &target_type))
                });
                if let (Some(target_member), Some(source_member)) = (unmatched, sources.first()) {
                    self.signature(source_member, target_member, construct, method);
                }
            }
        }
    }

    /// `compareSignaturesRelated` for a source signature the relation
    /// rejects against a target one.
    fn signature(&mut self, source: &FunctionType, target: &FunctionType, construct: bool, method: bool) {
        self.named_signature(source, target, source, target, construct, method);
    }

    /// [`Self::signature`] with the signatures whose parameter names are
    /// reported: instantiation keeps the types, not the names.
    fn named_signature(
        &mut self,
        source: &FunctionType,
        target: &FunctionType,
        named_source: &FunctionType,
        named_target: &FunctionType,
        construct: bool,
        method: bool,
    ) {
        // A generic side is compared as the relation compared it: the
        // target's canonical signature, the source instantiated in its context.
        let canonical = surge_ts_types::canonical_signatures(source, target);
        let (source, target) = match &canonical {
            Some(canonical) => (&canonical.source, &canonical.target),
            None => (source, target),
        };
        let target_count = target.parameters().len();
        if !target.is_variadic() && source.required_parameter_count() > target_count {
            self.report(
                Kind::Other,
                format!(
                    "Target signature provides too few arguments. Expected {} or more, but got {target_count}.",
                    source.required_parameter_count()
                ),
            );
            return;
        }
        let source_count = source.parameters().len();
        let strict = self.strict_function_types && !method && !target.is_method_declaration();
        let count = if has_non_array_rest(source) || has_non_array_rest(target) {
            source_count.min(target_count)
        } else {
            source_count.max(target_count)
        };
        for index in 0..count {
            let (Some(source_parameter), Some(target_parameter)) =
                (parameter_type_at(source, index), parameter_type_at(target, index))
            else {
                continue;
            };
            let (source_parameter, target_parameter) = (&source_parameter, &target_parameter);
            if source_parameter == target_parameter {
                continue;
            }
            let names = || {
                parameter_name(named_source, index)
                    .or_else(|| parameter_name(source, index))
                    .zip(parameter_name(named_target, index).or_else(|| parameter_name(target, index)))
            };
            let callbacks = single_signature(&non_nullable(source_parameter))
                .zip(single_signature(&non_nullable(target_parameter)))
                .filter(|(source_callback, target_callback)| {
                    source_callback.type_predicate().is_none()
                        && target_callback.type_predicate().is_none()
                        && nullable(source_parameter) == nullable(target_parameter)
                });
            if let Some((source_callback, target_callback)) = callbacks {
                if is_assignable_to(target_parameter, source_parameter)
                    || (!strict && is_assignable_to(source_parameter, target_parameter))
                {
                    continue;
                }
                let Some((source_name, target_name)) = names() else {
                    return;
                };
                let named_callback = |function: &FunctionType| {
                    parameter_type_at(function, index).and_then(|parameter| single_signature(&non_nullable(&parameter)))
                };
                let named_target_callback = named_callback(named_target).unwrap_or_else(|| target_callback.clone());
                let named_source_callback = named_callback(named_source).unwrap_or_else(|| source_callback.clone());
                self.named_signature(
                    &target_callback,
                    &source_callback,
                    &named_target_callback,
                    &named_source_callback,
                    false,
                    !strict,
                );
                self.report(
                    Kind::Other,
                    format!("Types of parameters '{source_name}' and '{target_name}' are incompatible."),
                );
                return;
            }
            let related = (!strict && is_assignable_to(source_parameter, target_parameter))
                || is_assignable_to(target_parameter, source_parameter);
            if !related {
                let Some((source_name, target_name)) = names() else {
                    return;
                };
                self.explain(target_parameter, source_parameter);
                self.report(
                    Kind::Other,
                    format!("Types of parameters '{source_name}' and '{target_name}' are incompatible."),
                );
                return;
            }
        }
        // `compareTypePredicateRelatedTo` stands in for the return types.
        if let Some(target_predicate) = target.type_predicate() {
            use surge_ts_types::TypePredicateKind;
            match source.type_predicate() {
                Some(source_predicate) if source_predicate.kind == target_predicate.kind => {
                    if let (Some(source_type), Some(target_type)) = (&source_predicate.ty, &target_predicate.ty)
                        && !is_assignable_to(source_type, target_type)
                    {
                        self.explain(source_type, target_type);
                        self.report(
                            Kind::Other,
                            format!(
                                "Type predicate '{}' is not assignable to '{}'.",
                                predicate_display(source_predicate, named_source),
                                predicate_display(target_predicate, named_target)
                            ),
                        );
                    }
                }
                None if matches!(target_predicate.kind, TypePredicateKind::This | TypePredicateKind::Identifier) => {
                    if let Some(signature) = signature_display(named_source, construct) {
                        self.report(Kind::Other, format!("Signature '{signature}' must be a type predicate."));
                    }
                }
                _ => {}
            }
            return;
        }
        let target_return = target.return_type();
        if matches!(target_return, Type::Void | Type::Any) {
            return;
        }
        let source_return = source.return_type();
        if !is_assignable_to(source_return, target_return) {
            self.explain(source_return, target_return);
            self.report(
                Kind::ReturnMarker {
                    no_arguments: source.parameters().is_empty() && target.parameters().is_empty(),
                    construct,
                },
                String::new(),
            );
        }
    }
}

fn render_property_kind(kind: &Kind) -> String {
    match kind {
        Kind::TypesOfProperty(name) => format!("Types of property '{name}' are incompatible."),
        Kind::TypesOf(name) => format!("The types of '{name}' are incompatible between these types."),
        Kind::TypesReturnedBy(name) => format!("The types returned by '{name}' are incompatible between these types."),
        _ => String::new(),
    }
}

/// relater.go `getPropertyNameArg`.
fn property_name_arg(name: &str) -> String {
    if name.starts_with(['"', '\'', '`']) { format!("[{name}]") } else { name.to_string() }
}

/// relater.go `addToDottedName`.
fn add_to_dotted_name(head: &str, tail: &str) -> String {
    let head = if head.starts_with("new ") { format!("({head})") } else { head.to_string() };
    let mut position = 0;
    loop {
        if tail[position..].starts_with('(') {
            position += 1;
        } else if tail[position..].starts_with("new ") {
            position += 4;
        } else {
            break;
        }
    }
    let (prefix, suffix) = tail.split_at(position);
    if suffix.starts_with('[') { format!("{prefix}{head}{suffix}") } else { format!("{prefix}{head}.{suffix}") }
}

/// `symbolToString` of a property: an identifier or numeric name as is,
/// anything else quoted.
fn display_property_name(name: &str) -> String {
    let identifier = name
        .chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$' || first == '#')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '#');
    if identifier || name.parse::<f64>().is_ok() || name.starts_with('[') {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

/// The order tsgo keeps a union's members in (`CompareTypes`), with
/// `boolean` as its two literals.
pub(crate) fn tsgo_order(members: &[Type]) -> Vec<Type> {
    let mut flattened = Vec::with_capacity(members.len());
    for member in members {
        match member {
            Type::Boolean => {
                flattened.push(Type::BooleanLiteral(false));
                flattened.push(Type::BooleanLiteral(true));
            }
            other => flattened.push(other.clone()),
        }
    }
    surge_ts_types::sort_union_members(&mut flattened);
    flattened
}

/// An enum's own type: its owner, unqualified name, qualified name and members.
struct EnumDeclaration {
    owner: std::sync::Arc<str>,
    name: String,
    qualified: String,
    members: std::sync::Arc<surge_ts_types::EnumMembers>,
}

fn enum_declaration(ty: &Type) -> Option<EnumDeclaration> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    let owner = reference.enum_owner.clone()?;
    if *reference.id != *owner {
        return None;
    }
    let members = reference.enum_members.clone()?;
    let qualified = owner.rsplit('\u{0}').next().unwrap_or(&owner).to_string();
    let name = qualified.rsplit('.').next().unwrap_or(&qualified).to_string();
    Some(EnumDeclaration { owner, name, qualified, members })
}

/// relater.go `isEnumTypeRelatedTo` between two same-named regular enums: the
/// first source member the target lacks or values differently.
fn enum_mismatch(source: &EnumDeclaration, target: &EnumDeclaration) -> Option<String> {
    if source.owner == target.owner || source.name != target.name || !source.members.regular || !target.members.regular {
        return None;
    }
    let value_text = |value: &Type| value.name();
    for (member, source_value) in &source.members.members {
        let Some((_, target_value)) = target.members.members.iter().find(|(name, _)| name == member) else {
            return Some(format!("Property '{member}' is missing in type '{}'.", target.qualified));
        };
        if source_value == target_value {
            continue;
        }
        match (source_value, target_value) {
            (Some(source_value), Some(target_value)) => {
                return Some(format!(
                    "Each declaration of '{}.{member}' differs in its value, where '{}' was expected but '{}' was given.",
                    target.name,
                    value_text(target_value),
                    value_text(source_value)
                ));
            }
            (Some(known), None) | (None, Some(known)) if matches!(known, Type::StringLiteral(_)) => {
                return Some(format!(
                    "One value of '{}.{member}' is the string '{}', and the other is assumed to be an unknown numeric value.",
                    target.name,
                    value_text(known)
                ));
            }
            _ => {}
        }
    }
    None
}

/// An enum's own type spelled as the union of its members. tsgo interns that
/// union as the enum (flagged primitive); an alias of it, or a union of only
/// some members, is an ordinary union.
fn is_enum_union(ty: &Type) -> bool {
    let Type::Union(union) = ty else {
        return false;
    };
    if union.alias_name().is_some() {
        return false;
    }
    let mut owner = None;
    let mut members = None;
    for member in union.types() {
        let Type::Reference(reference) = member else {
            return false;
        };
        let Some(member_owner) = &reference.enum_owner else {
            return false;
        };
        if *owner.get_or_insert(member_owner) != member_owner {
            return false;
        }
        members = members.or(reference.enum_members.clone());
    }
    members.is_none_or(|members| union.types().len() >= members.members.len())
}

/// A source tsgo does not elaborate structurally: a primitive, or a type
/// whose apparent type is a primitive's wrapper.
fn is_primitive_like(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Any
            | Type::Unknown
            | Type::GenuineUnknown
            | Type::ErrorType
    ) || matches!(ty, Type::Reference(reference) if reference.enum_owner.is_some())
        || surge_ts_types::is_template_literal_type(ty)
}

fn indexed_access(ty: &Type) -> Option<(Type, Type)> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match surge_ts_types::type_variable::deferred_type(parameter)? {
        surge_ts_types::type_variable::DeferredType::IndexedAccess { object, index } => Some((object, index)),
        _ => None,
    }
}

/// The alias a function type was instantiated from: its id, arguments and
/// variances.
fn alias_instantiation(ty: &Type) -> Option<(std::sync::Arc<str>, Vec<Type>, u32)> {
    match ty {
        Type::Function(function) => function
            .alias_instantiation()
            .map(|alias| (alias.id.clone(), alias.arguments.to_vec(), alias.variances)),
        _ => None,
    }
}

/// A tuple-like type's elements, each flagged when it is a rest element.
fn tuple_elements(ty: &Type) -> Option<Vec<(Type, bool)>> {
    match ty {
        Type::Tuple(elements) => Some(elements.iter().map(|element| (element.clone(), false)).collect()),
        Type::OpenTuple(tuple) => {
            if tuple.rest.is_type_variable() {
                return None;
            }
            let mut elements: Vec<(Type, bool)> = tuple.leading.iter().map(|element| (element.clone(), false)).collect();
            elements.push((tuple.rest.as_ref().clone(), true));
            elements.extend(tuple.trailing.iter().map(|element| (element.clone(), false)));
            Some(elements)
        }
        Type::Array(element) => Some(vec![(element.as_ref().clone(), true)]),
        _ => None,
    }
}

fn intersection_operands(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Object(object) if object.is_intersection => object.intersection_operands.as_deref().map(<[Type]>::to_vec),
        // An intersection of named types is deferred as a reference over its
        // operands.
        Type::Reference(reference) if reference.id.starts_with("\u{0}intersection") => {
            Some(reference.arguments.to_vec())
        }
        _ => None,
    }
}

/// `getBestMatchingType` for an object source and a union target, as far as
/// surge can tell its steps apart: the same generic reference, an invokable
/// member for an invokable source, then the member sharing the most keys.
fn best_matching_member(source: &Type, members: &[Type]) -> Option<Type> {
    // A type variable shares no literal keys with anything
    // (`findMostOverlappyType`), so nothing is elaborated.
    if marker_name(source).is_some() || matches!(source, Type::TypeParameter(_)) {
        return None;
    }
    let members = tsgo_order(members);
    if let Some(discriminated) = discriminated_member(source, &members) {
        return Some(discriminated);
    }
    if let Type::Reference(source_reference) = source
        && let Some(found) = members
            .iter()
            .find(|member| matches!(member, Type::Reference(reference) if reference.id == source_reference.id))
    {
        return Some(found.clone());
    }
    let peeled = source.peeled();
    if call_signature(&peeled).is_some()
        && let Some(found) = members.iter().find(|member| call_signature(&member.peeled()).is_some())
    {
        return Some(found.clone());
    }
    if construct_signature(&peeled).is_some()
        && let Some(found) = members.iter().find(|member| construct_signature(&member.peeled()).is_some())
    {
        return Some(found.clone());
    }
    let source_keys = property_keys(&peeled)?;
    let mut best = None;
    let mut matching = 0;
    for member in &members {
        if is_primitive_like(member) {
            continue;
        }
        let Some(member_keys) = property_keys(&member.peeled()) else {
            continue;
        };
        let overlap = source_keys.iter().filter(|key| member_keys.contains(key)).count();
        if overlap > 0 && overlap >= matching {
            best = Some(member.clone());
            matching = overlap;
        }
    }
    best
}

/// `findMatchingDiscriminantType` and `discriminateTypeByDiscriminableItems`:
/// the members left once each discriminant property of the source rules out
/// the members it cannot match, unless it would rule out all of them.
fn discriminated_member(source: &Type, members: &[Type]) -> Option<Type> {
    let Type::Object(source_object) = source.peeled() else {
        return None;
    };
    let member_properties: Vec<Option<ObjectType>> = members
        .iter()
        .map(|member| match member.peeled() {
            Type::Object(object) => Some(object),
            _ => None,
        })
        .collect();
    #[derive(Clone, Copy, PartialEq)]
    enum Include {
        True,
        Maybe,
        False,
    }
    let mut include: Vec<Include> = member_properties
        .iter()
        .map(|object| if object.is_some() { Include::True } else { Include::False })
        .collect();
    let mut discriminated = false;
    for (name, property) in source_object.properties.iter() {
        let types: Vec<&Type> = member_properties
            .iter()
            .filter_map(|object| object.as_ref()?.properties.get(name).map(|property| &property.ty))
            .collect();
        // `isDiscriminantProperty`: types that differ across the members, at
        // least one of them literal.
        let literal = types.iter().any(|ty| is_unit(ty) || matches!(ty, Type::Union(union) if union.types().iter().any(is_unit)));
        let uniform = types.windows(2).all(|pair| pair[0] == pair[1]);
        if types.len() < 2 || !literal || uniform {
            continue;
        }
        discriminated = true;
        let distributed: Vec<Type> = match &property.ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other.clone()],
        };
        let mut matched = false;
        for (index, object) in member_properties.iter().enumerate() {
            if include[index] == Include::False {
                continue;
            }
            let Some(member_type) = object.as_ref().and_then(|object| object.properties.get(name)).map(|property| &property.ty)
            else {
                continue;
            };
            if distributed.iter().any(|candidate| is_assignable_to(candidate, member_type)) {
                matched = true;
            } else {
                include[index] = Include::Maybe;
            }
        }
        for state in &mut include {
            if *state == Include::Maybe {
                *state = if matched { Include::False } else { Include::True };
            }
        }
    }
    if !discriminated || !include.contains(&Include::False) {
        return None;
    }
    let filtered: Vec<Type> = members
        .iter()
        .zip(&include)
        .filter(|(_, state)| **state == Include::True)
        .map(|(member, _)| member.clone())
        .collect();
    match filtered.len() {
        0 => None,
        1 => filtered.into_iter().next(),
        _ => Some(surge_ts_types::union_type(filtered)),
    }
}

fn is_unit(ty: &Type) -> bool {
    matches!(
        ty,
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) | Type::Undefined | Type::Null
    ) || matches!(ty, Type::Reference(reference) if reference.enum_owner.is_some())
}

fn property_keys(ty: &Type) -> Option<Vec<std::sync::Arc<str>>> {
    match ty {
        Type::Object(object) => Some(object.properties.keys().cloned().collect()),
        _ => None,
    }
}

fn property_of(source: &Type, name: &str) -> Option<(Type, bool, Option<surge_ts_types::MemberRestriction>)> {
    match source {
        Type::Object(object) => object
            .properties
            .get(name)
            .map(|property| (property.ty.clone(), property.optional, property.restriction.clone())),
        _ => None,
    }
}

/// `propertyRelatedTo`'s accessibility checks: a private member relates only
/// to its own declaration, a protected source member not to a public target.
fn restriction_mismatch(
    name: &str,
    source: Option<&surge_ts_types::MemberRestriction>,
    target: Option<&surge_ts_types::MemberRestriction>,
    source_name: &str,
    target_name: &str,
) -> Option<String> {
    let source_private = source.is_some_and(|restriction| restriction.private);
    let target_private = target.is_some_and(|restriction| restriction.private);
    if source_private || target_private {
        let same_declaration = matches!((source, target), (Some(source), Some(target)) if source.owner == target.owner);
        if same_declaration {
            return None;
        }
        return Some(if source_private && target_private {
            format!("Types have separate declarations of a private property '{name}'.")
        } else if source_private {
            format!("Property '{name}' is private in type '{source_name}' but not in type '{target_name}'.")
        } else {
            format!("Property '{name}' is private in type '{target_name}' but not in type '{source_name}'.")
        });
    }
    if source.is_some() && target.is_none() {
        return Some(format!("Property '{name}' is protected in type '{source_name}' but public in type '{target_name}'."));
    }
    None
}

/// `signaturesRelatedTo`'s construct-signature checks: an abstract constructor
/// type to a non-abstract one, then `constructorVisibilitiesAreCompatible`.
fn construct_modifier_mismatch(source: &FunctionType, target: &FunctionType) -> Option<String> {
    use surge_ts_types::ConstructorAccessibility::{Private, Protected, Public, Undeclared};
    let (source, target) = (source.construct_modifiers(), target.construct_modifiers());
    if source.is_abstract() && !target.is_abstract() {
        return Some("Cannot assign an abstract constructor type to a non-abstract constructor type.".to_string());
    }
    let name = |accessibility| match accessibility {
        Private => "private",
        Protected => "protected",
        _ => "public",
    };
    let compatible = match (source.accessibility(), target.accessibility()) {
        (Undeclared, _) | (_, Undeclared) | (_, Private) => true,
        (source, Protected) => source != Private,
        (source, Public) => source == Public,
    };
    (!compatible).then(|| {
        format!(
            "Cannot assign a '{}' constructor type to a '{}' constructor type.",
            name(source.accessibility()),
            name(target.accessibility())
        )
    })
}

fn call_signature(ty: &Type) -> Option<FunctionType> {
    match ty {
        Type::Function(function) => Some(function.clone()),
        Type::Object(object) => object.call_signature().cloned(),
        _ => None,
    }
}

fn construct_signature(ty: &Type) -> Option<FunctionType> {
    match ty {
        Type::Object(object) => object.construct_signature().cloned(),
        _ => None,
    }
}

fn single_signature(ty: &Type) -> Option<FunctionType> {
    let peeled = ty.peeled();
    match &peeled {
        Type::Function(function) if function.overloads().is_none() => Some(function.clone()),
        Type::Object(object)
            if object.properties.is_empty()
                && object.construct_signature().is_none()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none() =>
        {
            object.call_signature().filter(|signature| signature.overloads().is_none()).cloned()
        }
        _ => None,
    }
}

fn non_nullable(ty: &Type) -> Type {
    surge_ts_types::remove_nullish(ty)
}

fn nullable(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Null => true,
        Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Undefined | Type::Null)),
        _ => false,
    }
}

/// `getTypeAtPosition` for a fixed parameter: an optional one reads
/// `undefined` as well under `strictNullChecks`.
fn parameter_type_at(function: &FunctionType, index: usize) -> Option<Type> {
    let parameters = function.parameters();
    let rest_index = parameters.len().checked_sub(1).filter(|_| function.is_variadic());
    // A rest parameter stands for each position from its own on, typed by its
    // element (`getTypeAtPosition`).
    if let Some(rest_index) = rest_index
        && index >= rest_index
    {
        let rest = &parameters[rest_index];
        return match rest.peeled() {
            Type::Array(element) => Some(*element),
            Type::Tuple(elements) => elements.get(index - rest_index).cloned(),
            _ if index == rest_index => Some(rest.clone()),
            _ => None,
        };
    }
    let ty = parameters.get(index)?;
    if index >= function.required_parameter_count() && surge_ts_types::strict_null_checks() && !nullable(ty) {
        return Some(surge_ts_types::union_type(vec![ty.clone(), Type::Undefined]));
    }
    Some(ty.clone())
}

/// A rest parameter that is not an array (a tuple or a type variable), which
/// `compareSignaturesRelated` compares only up to the shorter signature.
fn has_non_array_rest(function: &FunctionType) -> bool {
    function.is_variadic() && function.parameters().last().is_some_and(|rest| !matches!(rest.peeled(), Type::Array(_)))
}

/// `getParameterNameAtPosition`: the declared name, or the rest parameter's
/// beyond the fixed ones. `None` when surge kept no name.
fn parameter_name(function: &FunctionType, index: usize) -> Option<String> {
    let pick = |names: &[Option<String>]| {
        let last = names.len().checked_sub(1)?;
        let at = if function.is_variadic() && index > last { last } else { index };
        names.get(at)?.clone()
    };
    let recorded: Option<Vec<Option<String>>> = function
        .parameter_names()
        .map(|names| {
            names
                .iter()
                .map(|name| name.as_deref().and_then(surge_ts_types::plain_parameter_name).map(str::to_string))
                .collect()
        });
    // The rendered signature names each parameter as declared; a recorded
    // list can lag behind an instantiation.
    rendered_parameter_names(&function.name())
        .as_deref()
        .and_then(pick)
        .or_else(|| recorded.as_deref().and_then(pick))
}

/// The parameter names a rendered signature `(a: T, b?: U, ...c: V[]) => R`
/// spells, for a function type that kept no names of its own.
fn rendered_parameter_names(rendered: &str) -> Option<Vec<Option<String>>> {
    let (parameters, _) = split_arrow(rendered)?;
    let parameters = parameters.trim();
    let start = parameters.find('(')?;
    let inner = parameters.get(start + 1..parameters.len().checked_sub(1)?)?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut segment_start = 0;
    let bytes = inner.as_bytes();
    for index in 0..=bytes.len() {
        let at_end = index == bytes.len();
        if !at_end {
            match bytes[index] {
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'>' if index > 0 && bytes[index - 1] != b'=' => depth -= 1,
                _ => {}
            }
        }
        if at_end || (bytes[index] == b',' && depth == 0) {
            let segment = inner[segment_start..index].trim().trim_start_matches("...");
            let name: String = segment
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            names.push((!name.is_empty()).then_some(name));
            segment_start = index + 1;
        }
    }
    Some(names)
}

/// `typePredicateToString`.
fn predicate_display(predicate: &surge_ts_types::TypePredicate, function: &FunctionType) -> String {
    use surge_ts_types::TypePredicateKind;
    let subject = match predicate.kind {
        TypePredicateKind::This | TypePredicateKind::AssertsThis => "this".to_string(),
        TypePredicateKind::Identifier | TypePredicateKind::AssertsIdentifier => predicate
            .parameter_index
            .and_then(|index| parameter_name(function, index))
            .unwrap_or_default(),
    };
    let asserts = matches!(predicate.kind, TypePredicateKind::AssertsThis | TypePredicateKind::AssertsIdentifier);
    match (&predicate.ty, asserts) {
        (Some(ty), true) => format!("asserts {subject} is {}", ty.name()),
        (Some(ty), false) => format!("{subject} is {}", ty.name()),
        (None, _) => format!("asserts {subject}"),
    }
}

/// `signatureToString`: a signature as its declaration reads, with the return
/// type after a colon.
fn signature_display(function: &FunctionType, construct: bool) -> Option<String> {
    let rendered = function.name();
    let (parameters, return_type) = split_arrow(&rendered)?;
    Some(if construct { format!("new {parameters}: {return_type}") } else { format!("{parameters}: {return_type}") })
}

/// `(a: T) => R` split at its top-level arrow.
fn split_arrow(rendered: &str) -> Option<(&str, &str)> {
    let rendered = rendered.strip_prefix("new ").unwrap_or(rendered);
    let mut depth = 0i32;
    let bytes = rendered.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'>' if index > 0 && bytes[index - 1] != b'=' => depth -= 1,
            b'=' if depth == 0 && bytes.get(index + 1) == Some(&b'>') && index > 0 && bytes[index - 1] == b' ' => {
                return Some((rendered[..index - 1].trim_end(), rendered[index + 2..].trim_start()));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// `isObjectTypeWithInferableIndex`: an object literal or type literal type,
/// with no signatures and not a class instance.
fn inferable_index(object: &ObjectType) -> bool {
    object.call_signature().is_none()
        && object.construct_signature().is_none()
        && object.alias_name.is_none()
        && !object.is_intersection
}

fn is_global_interface(ty: &Type, name: &str) -> bool {
    matches!(ty.peeled(), Type::Object(object) if object.alias_name.as_deref() == Some(name))
}

/// The primitive whose global wrapper interface `ty` is.
fn wrapper_primitive(ty: &Type) -> Option<Type> {
    let Type::Object(object) = ty.peeled() else {
        return None;
    };
    match object.alias_name.as_deref()? {
        "String" => Some(Type::String),
        "Number" => Some(Type::Number),
        "Boolean" => Some(Type::Boolean),
        "Symbol" => Some(Type::Symbol),
        "BigInt" => Some(Type::BigInt),
        _ => None,
    }
}

/// The marker an opaque generic target stands in for its type parameter
/// `T` with (`{ "\0type parameter T": never }`).
const TYPE_PARAMETER_MARKER: &str = "\u{0}type parameter ";

pub(crate) fn marker_name(ty: &Type) -> Option<String> {
    let Type::Object(object) = ty else {
        return None;
    };
    let mut keys = object.properties.keys();
    let key = keys.next()?;
    keys.next().is_none().then(|| key.strip_prefix(TYPE_PARAMETER_MARKER).map(str::to_string)).flatten()
}

/// A rendered type with each type-parameter marker printed as its name.
fn unmark(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("{ ") {
        let after = &rest[at + 2..];
        let body = after.strip_prefix('\u{0}').unwrap_or(after);
        let replaced = body.strip_prefix("type parameter ").and_then(|tail| {
            let end = tail.find(": never; }")?;
            let name = &tail[..end];
            (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
                .then(|| (name.to_string(), &tail[end + ": never; }".len()..]))
        });
        match replaced {
            Some((name, tail)) => {
                out.push_str(&rest[..at]);
                out.push_str(&name);
                rest = tail;
            }
            None => {
                out.push_str(&rest[..at + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A string mapping type, also when reached through the lib alias that
/// names it (`Capitalize<string>`).
fn string_mapping(ty: &Type) -> Option<(surge_ts_types::StringMappingKind, Type)> {
    if let Some((kind, inner)) = surge_ts_types::string_mapping_parts(ty) {
        return Some((kind, inner.clone()));
    }
    let Type::Reference(reference) = ty else {
        return None;
    };
    surge_ts_types::string_mapping_parts(&reference.resolve()).map(|(kind, inner)| (kind, inner.clone()))
}

/// checker.go `getConstraintFromIndexedAccess`, one step at a time: `T[K]`
/// reads its index's constraint (`T[keyof T]`), and `keyof T` as an index
/// distributes over `string | number | symbol`.
fn indexed_access_constraint_step(source: &Type) -> Option<Type> {
    use surge_ts_types::type_variable::{DeferredType, active_constraint, deferred_type, indexed_access_variable};
    let Type::TypeParameter(parameter) = source else {
        return None;
    };
    let Some(DeferredType::IndexedAccess { object, index }) = deferred_type(parameter) else {
        return None;
    };
    let Type::TypeParameter(index_parameter) = &index else {
        return None;
    };
    let index_constraint = match deferred_type(index_parameter) {
        Some(DeferredType::Keyof(_)) => active_constraint(index_parameter).flatten()?,
        Some(_) => return None,
        None => active_constraint(index_parameter).flatten()?,
    };
    let members = match &index_constraint {
        Type::Union(union) => union.types().to_vec(),
        constraint => vec![constraint.clone()],
    };
    let reads = members
        .iter()
        .map(|member| indexed_access_variable(&object, member))
        .collect::<Option<Vec<_>>>()?;
    Some(if reads.len() == 1 { reads.into_iter().next()? } else { surge_ts_types::union_type(reads) })
}

/// The target as a type variable: its name and base constraint
/// (`getBaseConstraintOfType`), `None` inside when it has none.
fn type_variable_target(target: &Type) -> Option<(String, Option<Type>)> {
    if let Some(name) = marker_name(target) {
        return Some((name, None));
    }
    match target {
        // surge models a deferred type (a conditional, `keyof T`, a string
        // mapping) as a type variable too, but tsgo's line is for a type
        // parameter alone.
        Type::TypeParameter(parameter) if surge_ts_types::type_variable::deferred_type(parameter).is_none() => {
            let mut constraint = surge_ts_types::type_variable::active_constraint(parameter)?;
            // A constraint that is itself a type parameter resolves through it.
            for _ in 0..MAX_DEPTH {
                let Some(Type::TypeParameter(next)) = &constraint else {
                    break;
                };
                if surge_ts_types::type_variable::deferred_type(next).is_some() {
                    break;
                }
                constraint = surge_ts_types::type_variable::active_constraint(next).flatten();
            }
            Some((parameter.name.to_string(), constraint.filter(|constraint| !constraint.is_unknown())))
        }
        Type::Reference(reference) if reference.is_polymorphic_this() => {
            Some(("this".to_string(), Some(target.peeled()).filter(|peeled| !peeled.is_unknown())))
        }
        _ => None,
    }
}

/// `reportRelationError`'s generalized source: a literal widened to its base
/// type when the target could not hold a literal.
fn generalized_source(source: &Type, target: &Type) -> Type {
    if !matches!(target, Type::Never)
        && crate::checks::expr::is_literal_like(source)
        && !crate::checks::expr::could_have_singleton_types(target)
    {
        crate::checks::expr::widen_type(source)
    } else {
        source.clone()
    }
}
