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
    Ok(function
        .with_signature_types(parameters, return_type)
        .with_this_type(this_type))
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
        ty.name().chars().take(160).collect()
    }
    fn walk(a: &Type, b: &Type, path: &str, depth: usize) -> Option<String> {
        if a == b || depth > 12 {
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
                here()
            }
            (Type::Reference(left), Type::Reference(right)) if left.id == right.id && left.arguments.len() == right.arguments.len() => {
                for (index, (l, r)) in left.arguments.iter().zip(right.arguments.iter()).enumerate() {
                    if let Some(found) = walk(l, r, &format!("{path}<#{index}>"), depth + 1) {
                        return Some(found);
                    }
                }
                None
            }
            _ => here(),
        }
    }
    walk(a, b, "$", 0)
}
