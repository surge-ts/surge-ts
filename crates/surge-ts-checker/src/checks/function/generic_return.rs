//! A generic declaration's unannotated return read once over its own type
//! variables and instantiated per call (`instantiateType(getReturnTypeOfSignature(
//! sig.target), sig.mapper)`), checked in shadow against the per-call body read.

use std::sync::Arc;

use surge_ts_types::{ObjectProperty, PropertyMap, Type, TypeParameterType};

/// Why a type could not be instantiated faithfully.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Unsupported {
    /// A reference whose arguments name a mapped variable: re-instantiating it
    /// needs its declaration, which the reference does not carry.
    Reference(Arc<str>),
    /// A deferred type the variable substitution cannot rebuild.
    Deferred,
    /// Nesting beyond the walk's bound.
    Depth,
}

const MAX_DEPTH: usize = 48;

/// `SURGE_TYPE_LEVEL_DEFERRED=0` instantiates a deferred conditional only by
/// resolving its written node again, and keeps an intersection over a variable
/// as its operands once they are concrete.
pub(crate) fn type_level_deferred_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_TYPE_LEVEL_DEFERRED").as_deref() != Ok("0"))
}

thread_local! {
    /// The check types of the conditionals whose true branches are being
    /// instantiated: in such a branch the check type is read through the
    /// extends type (`getImpliedConstraint`), an intersection tsc's
    /// substitution type resolves to the check type once that satisfies the
    /// constraint. A conditional nested in a true branch keeps the outer one.
    static IMPLIED_CHECKS: std::cell::RefCell<Vec<TypeParameterType>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `ty` with every variable in `mapping` replaced by its type.
pub(crate) fn instantiate_type(ty: &Type, mapping: &[(TypeParameterType, Type)]) -> Result<Type, Unsupported> {
    instantiate(ty, mapping, 0)
}

fn instantiate(ty: &Type, mapping: &[(TypeParameterType, Type)], depth: usize) -> Result<Type, Unsupported> {
    if depth > MAX_DEPTH {
        return Err(Unsupported::Depth);
    }
    if !mentions(ty, mapping, 0) {
        return Ok(ty.clone());
    }
    let sub = |ty: &Type| instantiate(ty, mapping, depth + 1);
    Ok(match ty {
        Type::TypeParameter(parameter) => {
            if let Some((_, to)) = mapping.iter().find(|(from, _)| from == parameter) {
                return Ok(to.clone());
            }
            if type_level_deferred_enabled()
                && let Some(surge_ts_types::type_variable::DeferredType::Conditional(conditional)) =
                    surge_ts_types::type_variable::deferred_type(parameter)
                && let Some(evaluated) = instantiate_conditional(&conditional, mapping, depth)?
            {
                return Ok(evaluated);
            }
            if let Some(evaluated) = instantiate_over_reference(parameter, mapping, depth)? {
                return Ok(evaluated);
            }
            if let Some(origin) = surge_ts_types::type_variable::deferred_origin(parameter) {
                return origin.reinstantiate(mapping).ok_or(Unsupported::Deferred);
            }
            let mut current = ty.clone();
            for (from, to) in mapping {
                let next = surge_ts_types::type_variable::substitute_variable(&current, from, to);
                if matches!(next, Type::Unknown) {
                    return Err(Unsupported::Deferred);
                }
                current = next;
            }
            current
        }
        Type::Union(union) => {
            let members = union.types().iter().map(sub).collect::<Result<Vec<_>, _>>()?;
            surge_ts_types::union_type(members)
        }
        Type::Array(element) => Type::Array(Box::new(sub(element)?)),
        Type::Tuple(elements) => Type::Tuple(elements.iter().map(sub).collect::<Result<_, _>>()?),
        Type::OpenTuple(tuple) => {
            let mut tuple = tuple.clone();
            tuple.leading = tuple.leading.iter().map(sub).collect::<Result<_, _>>()?;
            tuple.rest = Box::new(sub(&tuple.rest)?);
            tuple.trailing = tuple.trailing.iter().map(sub).collect::<Result<_, _>>()?;
            Type::OpenTuple(tuple)
        }
        Type::Function(function) => Type::Function(instantiate_function(function, mapping, depth)?),
        Type::Object(object) if type_level_deferred_enabled() && is_variable_intersection(object) => {
            let operands = object.intersection_operands.as_deref().unwrap_or_default();
            if let [Type::TypeParameter(base), constraints @ ..] = operands
                && IMPLIED_CHECKS.with(|checks| checks.borrow().contains(base))
            {
                let base = sub(&operands[0])?;
                let constraints = constraints.iter().map(sub).collect::<Result<Vec<_>, _>>()?;
                if !surge_ts_types::type_variable::mentions_type_variable(&base)
                    && constraints.iter().all(|constraint| surge_ts_types::is_assignable_to(&base, constraint))
                {
                    return Ok(base);
                }
                let mut operands = vec![base];
                operands.extend(constraints);
                return Ok(intersection_of(operands));
            }
            intersection_of(operands.iter().map(sub).collect::<Result<Vec<_>, _>>()?)
        }
        Type::Object(object) => {
            let mut object = object.clone();
            let mut properties = PropertyMap::default();
            for (name, property) in object.properties.iter() {
                properties.insert(
                    name.clone(),
                    ObjectProperty {
                        ty: sub(&property.ty)?,
                        ..property.clone()
                    },
                );
            }
            object.properties = Arc::new(properties);
            object.property_map_id = None;
            object.string_index_type = object.string_index_type.as_deref().map(sub).transpose()?.map(Arc::new);
            object.number_index_type = object.number_index_type.as_deref().map(sub).transpose()?.map(Arc::new);
            object.call_signature = object
                .call_signature
                .as_deref()
                .map(|function| instantiate_function(function, mapping, depth))
                .transpose()?
                .map(Arc::new);
            object.construct_signature = object
                .construct_signature
                .as_deref()
                .map(|function| instantiate_function(function, mapping, depth))
                .transpose()?
                .map(Arc::new);
            object.intersection_operands = object
                .intersection_operands
                .as_deref()
                .map(|operands| operands.iter().map(sub).collect::<Result<Vec<_>, _>>())
                .transpose()?
                .map(Arc::from);
            Type::Object(object)
        }
        Type::Reference(reference) => {
            if let Some(reinstantiated) = reference.reinstantiate(mapping) {
                return Ok(reinstantiated);
            }
            // A reference built over the variables holds its expansion, not its
            // declaration: instantiate the expansion, whose deferred parts
            // resolve again from their origins.
            let arguments = reference.arguments.iter().map(sub).collect::<Result<Vec<_>, _>>()?;
            if let Some(rebuilt) = crate::infer::types::origin::reference_from_declaration(
                &reference.id,
                &instantiated_display(&reference.display, &arguments),
                arguments.clone(),
            ) {
                return Ok(rebuilt);
            }
            let resolved = sub(&reference.resolve()).map_err(|_| Unsupported::Reference(reference.id.clone()))?;
            crate::infer::types::make_type_reference(
                reference.id.clone(),
                instantiated_display(&reference.display, &arguments),
                arguments,
                Arc::new(resolved),
            )
        }
        other => other.clone(),
    })
}

/// A deferred `reference[index]` or `keyof reference` over a reference whose
/// arguments name the variables
/// ([`surge_ts_types::type_variable::indexed_access_reference_variable`]):
/// `substitute_variable` cannot rebuild the reference, so it instantiates here
/// and the access reads off the instance, deferred again while the instance
/// still names a variable.
fn instantiate_over_reference(
    parameter: &TypeParameterType,
    mapping: &[(TypeParameterType, Type)],
    depth: usize,
) -> Result<Option<Type>, Unsupported> {
    use surge_ts_types::type_variable::{self, DeferredType};
    match type_variable::deferred_type(parameter) {
        Some(DeferredType::IndexedAccess { object: object @ Type::Reference(_), index }) => {
            let object = instantiate(&object, mapping, depth + 1)?;
            let index = instantiate(&index, mapping, depth + 1)?;
            let read = if matches!(object, Type::Reference(_)) && type_variable::mentions_type_variable(&object) {
                type_variable::indexed_access_reference_variable(&object, &index)
            } else {
                type_variable::instantiated_indexed_access(&object, &index)
            };
            read.map(Some).ok_or(Unsupported::Deferred)
        }
        Some(DeferredType::Keyof(operand @ Type::Reference(_))) => {
            let operand = instantiate(&operand, mapping, depth + 1)?;
            let read = if type_variable::mentions_type_variable(&operand) {
                type_variable::keyof_reference_variable(&operand)
            } else {
                type_variable::instantiated_keyof(&operand)
            };
            read.map(Some).ok_or(Unsupported::Deferred)
        }
        _ => Ok(None),
    }
}

/// A memberless intersection kept as its operands because one is a type
/// variable ([`surge_ts_types::type_variable::type_variable_intersection`]).
fn is_variable_intersection(object: &surge_ts_types::ObjectType) -> bool {
    object.is_intersection
        && object.properties.is_empty()
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature.is_none()
        && object.construct_signature.is_none()
        && object.alias_name.is_none()
        && object.intersection_operands.is_some()
}

/// `getIntersectionType` over instantiated operands: kept as operands while
/// one is still a variable, merged once none is.
fn intersection_of(operands: Vec<Type>) -> Type {
    if operands.iter().any(surge_ts_types::type_variable::mentions_type_variable) {
        surge_ts_types::type_variable::type_variable_intersection(operands)
    } else {
        crate::infer::types::merge_intersection_members(operands)
    }
}

/// `getConditionalTypeInstantiation` of a deferred conditional at the type
/// level: the check and extends types instantiated, the branch chosen by
/// assignability (distributed over a union check type) and instantiated with
/// the check type parameter bound to each member. `None` where the written
/// node decides differently — `infer` captures, `any`, an operand surge could
/// not model, an operand still generic, a part this walk cannot instantiate —
/// so the origin resolves it.
fn instantiate_conditional(
    conditional: &surge_ts_types::type_variable::DeferredConditional,
    mapping: &[(TypeParameterType, Type)],
    depth: usize,
) -> Result<Option<Type>, Unsupported> {
    use surge_ts_types::lazy_probe::count;
    use surge_ts_types::type_variable::mentions_type_variable;
    if conditional.has_infer {
        count("typelevel.cond.infer");
        return Ok(None);
    }
    let (Ok(check), Ok(extends)) = (
        instantiate(&conditional.check, mapping, depth + 1),
        instantiate(&conditional.extends, mapping, depth + 1),
    ) else {
        count("typelevel.cond.unsupported");
        return Ok(None);
    };
    if mentions_type_variable(&check) || mentions_type_variable(&extends) {
        count("typelevel.cond.still_generic");
        return Ok(None);
    }
    if matches!(check, Type::ErrorType) || matches!(extends, Type::ErrorType) {
        return Ok(Some(Type::ErrorType));
    }
    if matches!(extends, Type::Any) || (extends.is_unknown() && !matches!(extends, Type::GenuineUnknown)) {
        count("typelevel.cond.extends_unmodelled");
        return Ok(None);
    }
    let check_parameter = match &conditional.check {
        Type::TypeParameter(parameter) => Some(parameter.clone()),
        _ => None,
    };
    let members = if conditional.distributive {
        match crate::program::with_dts_expansion_reason(crate::program::DtsExpansionReason::ConditionalType, || {
            check.peeled()
        }) {
            Type::Union(union) => union.types().to_vec(),
            Type::Never => Vec::new(),
            _ => vec![check.clone()],
        }
    } else {
        vec![check.clone()]
    };
    let mut results = Vec::with_capacity(members.len());
    for member in members {
        let peeled = match &member {
            Type::Reference(_) => member.peeled(),
            other => other.clone(),
        };
        if matches!(member, Type::ErrorType) {
            results.push(Type::ErrorType);
            continue;
        }
        if matches!(peeled, Type::Any) {
            count("typelevel.cond.member_any");
            return Ok(None);
        }
        if matches!(peeled, Type::Unknown | Type::TypeParameter(_))
            || (matches!(peeled, Type::GenuineUnknown) && extends.is_unknown())
            || holds_placeholder(&member, 4)
        {
            count("typelevel.cond.member_unmodelled");
            return Ok(None);
        }
        let take_true = matches!(extends, Type::GenuineUnknown) || surge_ts_types::is_assignable_to(&member, &extends);
        let branch_mapping: Vec<(TypeParameterType, Type)> = match (&check_parameter, conditional.distributive) {
            (Some(parameter), true) => mapping
                .iter()
                .filter(|(from, _)| from != parameter)
                .cloned()
                .chain(std::iter::once((parameter.clone(), member.clone())))
                .collect(),
            _ => mapping.to_vec(),
        };
        let branch = if take_true {
            let pushed = check_parameter.clone().map(|check| IMPLIED_CHECKS.with(|checks| checks.borrow_mut().push(check)));
            let instantiated = instantiate(&conditional.true_type, &branch_mapping, depth + 1);
            if pushed.is_some() {
                IMPLIED_CHECKS.with(|checks| checks.borrow_mut().pop());
            }
            instantiated
        } else {
            instantiate(&conditional.false_type, &branch_mapping, depth + 1)
        };
        let Ok(branch) = branch else {
            count("typelevel.cond.unsupported");
            return Ok(None);
        };
        // A lazy reference built over the variables would close over them
        // once peeled; the origin builds it over the instantiated arguments.
        if mentions_type_variable(&branch) || branch.is_unmodelled() || captures_variable(&branch, &branch_mapping, 3) {
            count("typelevel.cond.branch_unmodelled");
            return Ok(None);
        }
        results.push(branch);
    }
    count("typelevel.cond.decided");
    Ok(Some(surge_ts_types::union_type(results)))
}

/// A placeholder type parameter within `depth` levels: a check type the
/// written node would not decide.
fn holds_placeholder(ty: &Type, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    let nested = |ty: &Type| holds_placeholder(ty, depth - 1);
    match ty {
        Type::TypeParameter(_) => true,
        Type::Union(union) => union.types().iter().any(nested),
        Type::Array(element) => nested(element),
        Type::Tuple(elements) => elements.iter().any(nested),
        Type::Reference(reference) => reference.arguments.iter().any(nested),
        Type::Object(object) => {
            object.properties.values().any(|property| nested(&property.ty))
                || object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(nested))
        }
        _ => false,
    }
}

fn instantiate_function(
    function: &surge_ts_types::FunctionType,
    mapping: &[(TypeParameterType, Type)],
    depth: usize,
) -> Result<surge_ts_types::FunctionType, Unsupported> {
    let parameters = function
        .parameters()
        .iter()
        .map(|parameter| instantiate(parameter, mapping, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let return_type = instantiate(function.return_type(), mapping, depth + 1)?;
    let this_type = function
        .this_type()
        .map(|this_type| instantiate(this_type, mapping, depth + 1))
        .transpose()?;
    let mut instantiated = function
        .with_signature_types(parameters, return_type)
        .with_this_type(this_type);
    // A signature written as a generic alias (`TRPCLink<TRouter>`) is named by
    // its alias instantiation, whose arguments are instantiated with it.
    if let Some(alias) = function.alias_instantiation() {
        let arguments = alias
            .arguments
            .iter()
            .map(|argument| instantiate(argument, mapping, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        instantiated = instantiated.with_alias_instantiation(alias.id.clone(), arguments, alias.variances);
    }
    // The display names the arguments only where it was written with them (a
    // body-local alias capturing the variables renders bare).
    if let Some(name) = function.alias_name()
        && let Some(renamed) = alias_display_over_mapping(name, mapping)
    {
        instantiated = instantiated.with_alias_name(renamed);
    }
    Ok(instantiated)
}

/// An alias display (`TRPCLink<TRouter>`) with each top-level argument that is
/// a mapped variable's name rendered as the type it is mapped to, for a
/// signature that kept the display without its instantiation.
fn alias_display_over_mapping(display: &str, mapping: &[(TypeParameterType, Type)]) -> Option<String> {
    let open = display.find('<')?;
    let inner = display.strip_suffix('>')?.get(open + 1..)?;
    let mut arguments = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (index, ch) in inner.char_indices() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                arguments.push(inner[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    arguments.push(inner[start..].trim());
    let mut changed = false;
    let rendered: Vec<String> = arguments
        .iter()
        .map(|argument| match mapping.iter().find(|(variable, _)| &*variable.name == *argument) {
            Some((_, to)) => {
                changed = true;
                to.name()
            }
            None => argument.to_string(),
        })
        .collect();
    changed.then(|| format!("{}<{}>", &display[..open], rendered.join(", ")))
}

/// Whether `ty` names a mapped variable anywhere a substitution reaches,
/// deferred types included.
fn mentions(ty: &Type, mapping: &[(TypeParameterType, Type)], depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    let nested = |ty: &Type| mentions(ty, mapping, depth + 1);
    match ty {
        Type::TypeParameter(parameter) => {
            parameter.owner != 0
                && (mapping.iter().any(|(from, _)| from.owner == parameter.owner)
                    || surge_ts_types::type_variable::deferred_type(parameter).is_some())
        }
        Type::Union(union) => union.types().iter().any(nested),
        Type::Array(element) => nested(element),
        Type::Tuple(elements) => elements.iter().any(nested),
        Type::OpenTuple(tuple) => tuple.leading.iter().any(nested) || nested(&tuple.rest) || tuple.trailing.iter().any(nested),
        Type::Function(function) => function_mentions(function, mapping, depth),
        Type::Object(object) => {
            object.properties.values().any(|property| nested(&property.ty))
                || object.string_index_type.as_deref().is_some_and(nested)
                || object.number_index_type.as_deref().is_some_and(nested)
                || object.call_signature.as_deref().is_some_and(|function| function_mentions(function, mapping, depth))
                || object.construct_signature.as_deref().is_some_and(|function| function_mentions(function, mapping, depth))
                || object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(nested))
        }
        Type::Reference(reference) => reference.arguments.iter().any(nested),
        _ => false,
    }
}

fn function_mentions(function: &surge_ts_types::FunctionType, mapping: &[(TypeParameterType, Type)], depth: usize) -> bool {
    function.parameters().iter().any(|parameter| mentions(parameter, mapping, depth + 1))
        || mentions(function.return_type(), mapping, depth + 1)
        || function.this_type().is_some_and(|this_type| mentions(this_type, mapping, depth + 1))
}

/// Whether a reference reachable from `ty` (peeling up to `peels` levels)
/// resolves to something that still names a mapped variable: a lazy
/// reference built inside the generic read that closed over a variable.
pub(crate) fn captures_variable(ty: &Type, mapping: &[(TypeParameterType, Type)], peels: usize) -> bool {
    fn walk(ty: &Type, mapping: &[(TypeParameterType, Type)], peels: usize, depth: usize, budget: &mut usize) -> bool {
        if *budget == 0 || depth > MAX_DEPTH {
            return false;
        }
        *budget -= 1;
        let nested = |ty: &Type, budget: &mut usize| walk(ty, mapping, peels, depth + 1, budget);
        match ty {
            Type::TypeParameter(parameter) => mapping.iter().any(|(from, _)| from.owner == parameter.owner),
            Type::Union(union) => union.types().iter().any(|member| nested(member, budget)),
            Type::Array(element) => nested(element, budget),
            Type::Tuple(elements) => elements.iter().any(|element| nested(element, budget)),
            Type::OpenTuple(tuple) => {
                tuple.leading.iter().any(|element| nested(element, budget))
                    || nested(&tuple.rest, budget)
                    || tuple.trailing.iter().any(|element| nested(element, budget))
            }
            Type::Function(function) => {
                function.parameters().iter().any(|parameter| nested(parameter, budget))
                    || nested(function.return_type(), budget)
            }
            Type::Object(object) => {
                object.properties.values().any(|property| nested(&property.ty, budget))
                    || object.call_signature.as_deref().is_some_and(|function| {
                        function.parameters().iter().any(|parameter| nested(parameter, budget))
                            || nested(function.return_type(), budget)
                    })
            }
            Type::Reference(reference) => {
                reference.arguments.iter().any(|argument| nested(argument, budget))
                    || (peels > 0 && walk(&reference.resolve(), mapping, peels - 1, depth + 1, budget))
            }
            _ => false,
        }
    }
    let mut budget = 20_000;
    walk(ty, mapping, peels, 0, &mut budget)
}

/// A generic reference's display over instantiated arguments: the written base
/// name with the arguments rendered as they now are.
pub(crate) fn instantiated_display(display: &str, arguments: &[Type]) -> String {
    let base = display.split('<').next().unwrap_or(display);
    if arguments.is_empty() {
        return base.to_string();
    }
    format!("{base}<{}>", arguments.iter().map(Type::name).collect::<Vec<_>>().join(", "))
}

/// The first place `a` and `b` differ, as a member path with both sides there.
pub(crate) fn first_difference(a: &Type, b: &Type) -> Option<String> {
    fn short(ty: &Type) -> String {
        let variant = match ty {
            Type::Reference(reference) => format!("Reference[{}]", reference.id.rsplit('\u{0}').next().unwrap_or("")),
            Type::Function(function) => format!("Function[alias={}]", function.alias_name().unwrap_or("-")),
            Type::Object(_) => "Object".to_string(),
            Type::Union(_) => "Union".to_string(),
            Type::TypeParameter(parameter) => format!("TypeParameter#{}", parameter.owner),
            _ => "other".to_string(),
        };
        format!("{variant} {}", ty.name().chars().take(500).collect::<String>())
    }
    fn walk(a: &Type, b: &Type, path: &str, depth: usize) -> Option<String> {
        if a == b || depth > 60 {
            return None;
        }
        let here = || Some(format!("{path}: {} <> {}", short(a), short(b)));
        match (a, b) {
            (Type::Object(left), Type::Object(right)) => {
                for (name, property) in left.properties.iter() {
                    match right.properties.get(name) {
                        None => return Some(format!("{path}.{name}: only in per-call")),
                        Some(other) => {
                            if property.optional != other.optional || property.readonly != other.readonly || property.method != other.method {
                                return Some(format!("{path}.{name}: modifiers differ"));
                            }
                            if let Some(found) = walk(&property.ty, &other.ty, &format!("{path}.{name}"), depth + 1) {
                                return Some(found);
                            }
                        }
                    }
                }
                for name in right.properties.keys() {
                    if !left.properties.contains_key(name) {
                        return Some(format!("{path}.{name}: only in instantiated"));
                    }
                }
                match (&left.call_signature, &right.call_signature) {
                    (Some(l), Some(r)) => {
                        if let Some(found) = walk(&Type::Function((**l).clone()), &Type::Function((**r).clone()), &format!("{path}()"), depth + 1) {
                            return Some(found);
                        }
                    }
                    (None, None) => {}
                    _ => return Some(format!("{path}(): call signature on one side")),
                }
                if left.string_index_type != right.string_index_type {
                    return Some(format!("{path}[string]: index differs"));
                }
                if let (Some(l), Some(r)) = (left.intersection_operands.as_deref(), right.intersection_operands.as_deref()) {
                    if l.len() != r.len() {
                        return Some(format!("{path}: intersection operand count differs"));
                    }
                    for (index, (l, r)) in l.iter().zip(r).enumerate() {
                        if let Some(found) = walk(l, r, &format!("{path}&#{index}"), depth + 1) {
                            return Some(found);
                        }
                    }
                } else if left.intersection_operands.is_some() != right.intersection_operands.is_some() {
                    return Some(format!("{path}: intersection operands on one side"));
                }
                match (&left.construct_signature, &right.construct_signature) {
                    (Some(l), Some(r)) => {
                        if let Some(found) = walk(&Type::Function((**l).clone()), &Type::Function((**r).clone()), &format!("{path}new()"), depth + 1) {
                            return Some(found);
                        }
                    }
                    (None, None) => {}
                    _ => return Some(format!("{path}: construct signature on one side")),
                }
                if left.number_index_type != right.number_index_type
                    || left.string_index_readonly != right.string_index_readonly
                    || left.number_index_readonly != right.number_index_readonly
                    || left.keyed_indexes != right.keyed_indexes
                    || left.non_primitive != right.non_primitive
                {
                    return Some(format!("{path}: object index or flags differ"));
                }
                None
            }
            (Type::Function(left), Type::Function(right)) => {
                if left.parameters().len() != right.parameters().len() {
                    return Some(format!("{path}: parameter count {} <> {}", left.parameters().len(), right.parameters().len()));
                }
                for (index, (l, r)) in left.parameters().iter().zip(right.parameters()).enumerate() {
                    if let Some(found) = walk(l, r, &format!("{path}(#{index})"), depth + 1) {
                        return Some(found);
                    }
                }
                if let Some(found) = walk(left.return_type(), right.return_type(), &format!("{path}=>"), depth + 1) {
                    return Some(found);
                }
                let (l, r) = (left.payload(), right.payload());
                if l.is_variadic == r.is_variadic
                    && l.required_parameter_count == r.required_parameter_count
                    && left.this_type() == right.this_type()
                {
                    // Equal but for the canonical store's interning ids.
                    return None;
                }
                Some(format!(
                    "{path}: function flags differ variadic {}/{} required {}/{} this {:?}/{:?} [{} <> {}]",
                    l.is_variadic,
                    r.is_variadic,
                    l.required_parameter_count,
                    r.required_parameter_count,
                    left.this_type().map(short),
                    right.this_type().map(short),
                    short(a),
                    short(b)
                ))
            }
            (Type::Union(left), Type::Union(right)) if left.types().len() == right.types().len() => {
                for (l, r) in left.types().iter().zip(right.types()) {
                    if let Some(found) = walk(l, r, &format!("{path}|"), depth + 1) {
                        return Some(found);
                    }
                }
                None
            }
            (Type::Reference(left), Type::Reference(right)) if left.id == right.id && left.arguments.len() == right.arguments.len() => {
                for (index, (l, r)) in left.arguments.iter().zip(right.arguments.iter()).enumerate() {
                    if let Some(found) = walk(l, r, &format!("{path}<#{index}>"), depth + 1) {
                        return Some(found);
                    }
                }
                None
            }
            (Type::Array(left), Type::Array(right)) => walk(left, right, &format!("{path}[]"), depth + 1),
            // One side kept lazy what the other expanded: equal when the
            // expansion is, and both render alike.
            (Type::Reference(reference), other) | (other, Type::Reference(reference))
                if !matches!(other, Type::Reference(_)) && a.name() == b.name() =>
            {
                let resolved = reference.resolve();
                if matches!(resolved, Type::Reference(_)) {
                    return here();
                }
                let (left, right) = if matches!(a, Type::Reference(_)) { (&resolved, other) } else { (other, &resolved) };
                walk(left, right, &format!("{path}~"), depth + 1)
            }
            (Type::TypeParameter(left), Type::TypeParameter(right)) => Some(format!(
                "{path}: {}#{} <> {}#{}",
                left.name, left.owner, right.name, right.owner
            )),
            (_, Type::TypeParameter(right)) => Some(format!("{path}: {} <> {}#{}", short(a), right.name, right.owner)),
            _ => here(),
        }
    }
    walk(a, b, "$", 0)
}

/// The deepest place `a` and `b` render differently, by display fingerprint.
pub(crate) fn first_display_difference(a: &Type, b: &Type) -> Option<String> {
    fn fingerprint(ty: &Type) -> u64 {
        crate::speculative::display_type_fingerprint(ty)
    }
    fn short(ty: &Type) -> String {
        ty.name().chars().take(300).collect()
    }
    fn walk(a: &Type, b: &Type, path: &str, depth: usize) -> Option<String> {
        if fingerprint(a) == fingerprint(b) {
            return None;
        }
        let deeper = |pairs: Vec<(String, Type, Type)>| {
            pairs.into_iter().find_map(|(step, l, r)| walk(&l, &r, &format!("{path}{step}"), depth + 1))
        };
        let found = (depth < 60)
            .then(|| match (a, b) {
                (Type::Object(left), Type::Object(right)) => deeper(
                    left.properties
                        .iter()
                        .filter_map(|(name, property)| {
                            Some((format!(".{name}"), property.ty.clone(), right.properties.get(name)?.ty.clone()))
                        })
                        .collect(),
                ),
                (Type::Function(left), Type::Function(right)) if left.parameters().len() == right.parameters().len() => {
                    let mut pairs: Vec<(String, Type, Type)> = left
                        .parameters()
                        .iter()
                        .zip(right.parameters())
                        .enumerate()
                        .map(|(index, (l, r))| (format!("(#{index})"), l.clone(), r.clone()))
                        .collect();
                    pairs.push(("=>".to_string(), left.return_type().clone(), right.return_type().clone()));
                    deeper(pairs)
                }
                (Type::Union(left), Type::Union(right)) if left.types().len() == right.types().len() => deeper(
                    left.types()
                        .iter()
                        .zip(right.types())
                        .map(|(l, r)| ("|".to_string(), l.clone(), r.clone()))
                        .collect(),
                ),
                (Type::Array(left), Type::Array(right)) => deeper(vec![("[]".to_string(), (**left).clone(), (**right).clone())]),
                (Type::Reference(left), Type::Reference(right)) if left.arguments.len() == right.arguments.len() => deeper(
                    left.arguments
                        .iter()
                        .zip(right.arguments.iter())
                        .enumerate()
                        .map(|(index, (l, r))| (format!("<#{index}>"), l.clone(), r.clone()))
                        .collect(),
                ),
                _ => None,
            })
            .flatten();
        found.or_else(|| Some(format!("{path}: {} <> {}", short(a), short(b))))
    }
    walk(a, b, "$", 0)
}
