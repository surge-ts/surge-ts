use oxc_ast::ast::{TSInterfaceDeclaration, TSInterfaceHeritage, TSSignature};

use crate::{ParsedInterfaceDeclaration, ParsedInterfaceMember};

use super::spans::text_span_from_oxc_span;
use super::types::parse_type_parameters;
use super::types::{
    parse_call_signature, parse_construct_signature, parse_index_signature_value_type,
    parse_type_method_signature, parse_type_property_signature,
};

pub(crate) fn parse_interface_declaration(
    declaration: &TSInterfaceDeclaration<'_>,
) -> Option<ParsedInterfaceDeclaration> {
    super::types::with_declaring_type(
        declaration.id.name.as_str(),
        declaration.type_parameters.as_deref(),
        || parse_interface_declaration_body(declaration),
    )
}

fn parse_interface_declaration_body(
    declaration: &TSInterfaceDeclaration<'_>,
) -> Option<ParsedInterfaceDeclaration> {
    let getters = super::types::getter_accessor_names(&declaration.body.body);
    let mut members: Vec<ParsedInterfaceMember> = declaration
        .body
        .body
        .iter()
        .filter(|member| !super::types::is_shadowed_setter(member, &getters))
        .filter_map(parse_interface_member)
        .collect();
    attach_member_write_types(
        &mut members,
        &super::types::setter_accessor_types(&declaration.body.body),
    );

    // An index signature (`[key: string]: T`, `[key: number]: T`) contributes
    // the object's index type rather than a named property, and the two kinds
    // are kept apart: a numeric key prefers the number one.
    let index_signature_of = |numeric: bool| {
        declaration.body.body.iter().find_map(|member| match member {
            TSSignature::TSIndexSignature(index_signature) => {
                let slots = super::types::index_signature_slots(index_signature);
                if !(if numeric { slots.number } else { slots.string }) {
                    return None;
                }
                parse_index_signature_value_type(index_signature).map(|value_type| {
                    (
                        (value_type, text_span_from_oxc_span(index_signature.span)),
                        index_signature.readonly,
                    )
                })
            }
            _ => None,
        })
    };
    let mut keyed_indexes = super::types::KeyedIndexCollector::default();
    let mut index_parameter_names = super::types::IndexParameterNameCollector::default();
    for member in &declaration.body.body {
        if let TSSignature::TSIndexSignature(index_signature) = member {
            let slots = super::types::index_signature_slots(index_signature);
            index_parameter_names.record(&slots, index_signature);
            keyed_indexes.add(&slots, index_signature);
        }
    }
    let (string_index, string_index_readonly) = index_signature_of(false).unzip();
    let (string_index_type, string_index_span) = string_index.unzip();
    let (number_index, number_index_readonly) = index_signature_of(true).unzip();
    let (number_index_type, number_index_span) = number_index.unzip();

    // A bare call signature (`(value?: any): number`) makes the interface
    // callable. Multiple overloads fold into one permissive signature the same
    // way a type literal's do — keeping only the first made every call matching
    // a *later* overload a false TS2554 (execa's `(file, args?, options?)`
    // behind its template-tag signature).
    let call_signature_overloads: Vec<crate::ParsedFunctionType> = declaration
        .body
        .body
        .iter()
        .filter_map(|member| match member {
            TSSignature::TSCallSignatureDeclaration(signature) => parse_call_signature(signature),
            _ => None,
        })
        .collect();
    let call_signature = call_signature_overloads
        .iter()
        .cloned()
        .reduce(|merged, signature| {
            super::types::merge_parsed_call_signatures(&merged, &signature)
        });

    // Construct signatures (`new <T>(...): Promise<T>`) make the interface usable
    // with `new` (e.g. `PromiseConstructor`). Collect every overload; the resolver
    // merges them into one permissive signature.
    let construct_signatures = declaration
        .body
        .body
        .iter()
        .filter_map(|member| match member {
            TSSignature::TSConstructSignatureDeclaration(signature) => {
                parse_construct_signature(signature)
            }
            _ => None,
        })
        .collect();

    Some(ParsedInterfaceDeclaration {
        is_declare: declaration.declare,
        name: declaration.id.name.to_string(),
        name_span: Some(text_span_from_oxc_span(declaration.id.span)),
        type_parameters: parse_type_parameters(declaration.type_parameters.as_deref()),
        extends: declaration
            .extends
            .iter()
            .filter_map(parse_interface_heritage)
            .collect(),
        members,
        string_index_type,
        number_index_type,
        string_index_span,
        number_index_span,
        string_index_readonly: string_index_readonly.unwrap_or(false),
        number_index_readonly: number_index_readonly.unwrap_or(false),
        call_signature,
        call_signature_overloads: if call_signature_overloads.len() > 1 {
            call_signature_overloads
        } else {
            Vec::new()
        },
        construct_signatures,
        keyed_indexes: keyed_indexes.finish(),
        index_parameter_names: index_parameter_names.finish(),
    })
}

fn parse_interface_heritage(heritage: &TSInterfaceHeritage<'_>) -> Option<crate::ParsedNamedType> {
    let (name, span) = super::types::flatten_heritage_expression(&heritage.expression)?;

    let type_arguments = heritage
        .type_arguments
        .as_deref()
        .and_then(super::types::parse_type_arguments)
        .unwrap_or_default();

    Some(crate::ParsedNamedType {
        type_argument_spans: Vec::new(),
        name,
        span: Some(span),
        type_arguments,
    })
}

/// The interface-member form of
/// [`super::types::attach_accessor_write_types`]: a getter that shadowed a
/// setter is not read-only, and carries the setter's parameter type as its
/// write type when the two differ.
fn attach_member_write_types(
    members: &mut [ParsedInterfaceMember],
    setters: &std::collections::HashMap<String, crate::ParsedType>,
) {
    for member in members {
        let Some(setter_type) = setters.get(&member.name) else {
            continue;
        };
        member.readonly = false;
        member.write_ty = (member.ty != *setter_type).then(|| setter_type.clone());
    }
}

fn parse_interface_member(member: &TSSignature<'_>) -> Option<ParsedInterfaceMember> {
    let written_name = match member {
        TSSignature::TSPropertySignature(signature) => {
            super::spans::written_key_name(oxc_span::GetSpan::span(&signature.key), signature.computed)
        }
        TSSignature::TSMethodSignature(signature) => {
            super::spans::written_key_name(oxc_span::GetSpan::span(&signature.key), signature.computed)
        }
        _ => None,
    };
    let property = match member {
        TSSignature::TSPropertySignature(property_signature) => {
            parse_type_property_signature(property_signature)?
        }
        TSSignature::TSMethodSignature(method_signature) => {
            parse_type_method_signature(method_signature)?
        }
        _ => return None,
    };

    Some(ParsedInterfaceMember {
        name: property.name,
        name_span: property.name_span,
        written_name,
        optional: property.optional,
        is_abstract: false,
        is_method: property.is_method,
        readonly: property.readonly,
        write_ty: property.write_ty,
        ty: property.ty,
    })
}
