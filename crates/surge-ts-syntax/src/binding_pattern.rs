//! How a destructuring parameter reads in a signature: the name tsc prints
//! for it and the type an unannotated one implies.

use std::sync::Arc;

use crate::{ParsedBindingName, ParsedType};

impl ParsedBindingName {
    /// A parameter's name as tsc prints it in a signature: an identifier, or
    /// a binding pattern written without its defaults (`{ a, b: [c] }`).
    pub fn signature_display(&self) -> Option<String> {
        binding_display(self)
    }

    /// The type an unannotated destructured parameter implies
    /// (`getTypeFromBindingPattern`); see [`binding_pattern_implied_type`].
    pub fn implied_type(&self, typed_targets: bool) -> Option<ParsedType> {
        binding_pattern_implied_type(self, typed_targets)
    }
}

/// tsc's `getTypeFromBindingPattern`, the type an unannotated destructured
/// parameter with no initializer or contextual type is given
/// (`getTypeForVariableLikeDeclaration`): an object pattern implies an object of
/// its named elements, an array pattern a tuple of its positions, each `any`
/// unless it is itself a pattern. An element with a default is optional and
/// read as `any` where tsc reads the default's widened type. `None` where the
/// implied type is not a plain object or tuple: a computed key (tsc leaves it
/// out and stops checking excess properties against the pattern), an array
/// pattern of nothing but a rest (`Iterable<any>`), and a rest after an
/// optional position, which an open tuple cannot state.
/// `typed_targets`: the pattern of a destructuring assignment, whose names are
/// existing targets typed as declared (`checkObjectLiteral` of the target).
fn binding_pattern_implied_type(binding: &ParsedBindingName, typed_targets: bool) -> Option<ParsedType> {
    match binding {
        ParsedBindingName::ObjectPattern(pattern) => {
            let mut properties = Vec::with_capacity(pattern.elements.len());
            // `getTypeFromObjectBindingPattern` leaves a computed name out.
            for element in pattern.elements.iter().filter(|element| !element.computed_key) {
                let ty = if element.has_default && !typed_targets {
                    ParsedType::Any
                } else {
                    binding_element_implied_type(&element.binding_name, typed_targets)?
                };
                properties.push(crate::ParsedObjectTypeProperty {
                    written_name: None,
                    name: element.property_name.clone(),
                    name_span: None,
                    signature_span: None,
                    ty,
                    optional: element.has_default,
                    is_method: false,
                    readonly: false,
                    write_ty: None,
                });
            }
            Some(ParsedType::Object(Arc::new(crate::ParsedObjectType {
                properties,
                string_index_type: pattern.rest.is_some().then(|| Box::new(ParsedType::Any)),
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                call_signature: None,
                call_signature_overloads: Vec::new(),
                construct_signature: None,
                construct_signature_overloads: Vec::new(),
                non_primitive: false,
                display_name: None,
                abstract_construct_signature: false,
                keyed_indexes: None,
                number_index_span: None,
                index_parameter_names: None,
            })))
        }
        ParsedBindingName::ArrayPattern(pattern) => {
            if pattern.elements.is_empty() {
                return None;
            }
            let has_default = |index: usize| pattern.defaults.get(index).copied().unwrap_or(false);
            let min_length = (0..pattern.elements.len())
                .rev()
                .find(|&index| pattern.elements[index].is_some() && !has_default(index))
                .map_or(0, |index| index + 1);
            let mut elements = Vec::with_capacity(pattern.elements.len());
            for (index, element) in pattern.elements.iter().enumerate() {
                let ty = match element {
                    Some(element) if typed_targets || !has_default(index) => {
                        binding_element_implied_type(element, typed_targets)?
                    }
                    _ => ParsedType::Any,
                };
                elements.push(if index < min_length {
                    ty
                } else {
                    ParsedType::Union(Arc::new(vec![ParsedType::Undefined, ty]))
                });
            }
            let Some(rest) = pattern.rest.as_deref() else {
                return Some(ParsedType::tuple(elements));
            };
            if min_length < elements.len() || !matches!(rest, ParsedBindingName::Identifier { .. }) {
                return None;
            }
            let mut elements: Vec<crate::ParsedTupleElement> = elements
                .into_iter()
                .map(crate::ParsedTupleElement::Fixed)
                .collect();
            elements.push(crate::ParsedTupleElement::Rest(
                ParsedType::Array(Arc::new(ParsedType::Any)),
                None,
            ));
            Some(ParsedType::variadic_tuple(elements))
        }
        ParsedBindingName::Identifier { .. } | ParsedBindingName::Unsupported { .. } => None,
    }
}

/// tsc's `getTypeFromBindingElement` for an element with no default: a nested
/// pattern's implied type, else `any`.
fn binding_element_implied_type(binding: &ParsedBindingName, typed_targets: bool) -> Option<ParsedType> {
    match binding {
        ParsedBindingName::Identifier { name, .. } if typed_targets => {
            Some(ParsedType::TypeOf(Arc::new(crate::ParsedTypeOfType {
                name: name.clone(),
                name_span: None,
                members: Vec::new(),
                import_specifier: None,
                member_spans: Vec::new(),
                type_arguments: Vec::new(),
                type_arguments_span: None,
            })))
        }
        ParsedBindingName::Identifier { .. } => Some(ParsedType::Any),
        ParsedBindingName::Unsupported { .. } if typed_targets => Some(ParsedType::Any),
        ParsedBindingName::Unsupported { .. } => None,
        pattern => binding_pattern_implied_type(pattern, typed_targets),
    }
}

/// A parameter's name as tsc prints it in a signature: an identifier, or a
/// binding pattern written without its defaults (`{ a, b: [c] }`).
fn binding_display(binding: &ParsedBindingName) -> Option<String> {
    match binding {
        ParsedBindingName::Identifier { name, .. } => Some(name.clone()),
        ParsedBindingName::ObjectPattern(pattern) => {
            let mut parts = Vec::with_capacity(pattern.elements.len() + 1);
            for element in &pattern.elements {
                let key = if element.computed_key {
                    match element.key_expression.as_deref().map(|(key, _)| key) {
                        Some(crate::ParsedExpression::StringLiteral(text)) => format!("[{text:?}]"),
                        Some(crate::ParsedExpression::Identifier { name, .. }) => format!("[{name}]"),
                        _ => return None,
                    }
                } else {
                    element.property_name.clone()
                };
                parts.push(match (&element.binding_name, element.shorthand) {
                    (ParsedBindingName::Identifier { .. }, true) => key,
                    (binding, _) => format!("{key}: {}", binding_display(binding)?),
                });
            }
            if let Some(rest) = &pattern.rest {
                parts.push(format!("...{}", binding_display(rest)?));
            }
            Some(if parts.is_empty() { "{}".to_string() } else { format!("{{ {} }}", parts.join(", ")) })
        }
        ParsedBindingName::ArrayPattern(pattern) => {
            let mut parts = Vec::with_capacity(pattern.elements.len() + 1);
            for element in &pattern.elements {
                parts.push(match element {
                    Some(binding) => binding_display(binding)?,
                    None => String::new(),
                });
            }
            if let Some(rest) = &pattern.rest {
                parts.push(format!("...{}", binding_display(rest)?));
            }
            Some(format!("[{}]", parts.join(", ")))
        }
        ParsedBindingName::Unsupported { .. } => None,
    }
}

