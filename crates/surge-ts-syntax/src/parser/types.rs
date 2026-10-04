use oxc_ast::ast::{
    Expression, PropertyKey, TSConditionalType, TSIndexedAccessType, TSIntersectionType, TSLiteral,
    TSLiteralType, TSMappedType, TSMappedTypeModifierOperator, TSMethodSignature,
    TSMethodSignatureKind, TSPropertySignature, TSSignature, TSTupleElement, TSTupleType, TSType,
    TSTypeAliasDeclaration, TSTypeLiteral, TSTypeName, TSTypeOperator, TSTypeOperatorOperator,
    TSTypeParameter, TSTypeParameterDeclaration, TSTypeParameterInstantiation, TSTypeQuery,
    TSTypeQueryExprName, TSTypeReference, TSUnionType,
};
use oxc_span::GetSpan;

use crate::{
    MappedOptionality, ParsedConditionalType, ParsedFunctionType, ParsedIndexedAccessType,
    ParsedKeyedIndexSignature, ParsedKeyedIndexes, ParsedMappedType,
    ParsedNamedType, ParsedObjectType, ParsedObjectTypeProperty, ParsedPredicateType,
    ParsedTemplateLiteralType, ParsedTupleElement, ParsedType, ParsedTypeAliasDeclaration,
    ParsedTypeOfType, ParsedTypeParameter,
};

use super::function_types::{
    parse_constructor_type, parse_function_type, parse_function_type_parameter,
    parse_function_type_rest_parameter,
};
use super::spans::text_span_from_oxc_span;

pub(crate) fn parse_type_annotation(
    type_annotation: &oxc_ast::ast::TSTypeAnnotation<'_>,
) -> Option<ParsedType> {
    parse_type(&type_annotation.type_annotation)
}

pub(crate) fn parse_type(type_annotation: &TSType<'_>) -> Option<ParsedType> {
    match type_annotation {
        TSType::TSStringKeyword(_) => Some(ParsedType::String),
        TSType::TSNumberKeyword(_) => Some(ParsedType::Number),
        TSType::TSBooleanKeyword(_) => Some(ParsedType::Boolean),
        TSType::TSUndefinedKeyword(_) => Some(ParsedType::Undefined),
        TSType::TSNullKeyword(_) => Some(ParsedType::Null),
        // `object` is every non-primitive: the empty object type models its
        // member surface exactly (a property read off it is an error, an
        // assignment out of it is checked), and the marker is what keeps a
        // primitive from satisfying it — `1 extends object` is false, which is
        // how `IsPlainObject`-style guards tell a value from a record.
        TSType::TSObjectKeyword(_) => Some(ParsedType::Object(std::sync::Arc::new(
            ParsedObjectType {
                properties: Vec::new(),
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                call_signature: None,
                call_signature_overloads: Vec::new(),
                construct_signature: None,
                construct_signature_overloads: Vec::new(),
                non_primitive: true,
                display_name: None,
                abstract_construct_signature: false,
                keyed_indexes: None,
            },
        ))),
        // `symbol` and `bigint` have no modelled representation; degrade to
        // `Unknown` rather than dropping the annotation, which would poison any
        // call/construct signature that mentions them (e.g. `SymbolConstructor`).
        TSType::TSSymbolKeyword(_) => Some(ParsedType::Symbol),
        TSType::TSBigIntKeyword(_) => Some(ParsedType::BigInt),
        TSType::TSVoidKeyword(_) => Some(ParsedType::Void),
        TSType::TSAnyKeyword(_) => Some(ParsedType::Any),
        TSType::TSUnknownKeyword(_) => Some(ParsedType::UnknownKeyword),
        // `intrinsic`-bodied lib aliases (`Uppercase`, `BuiltinIteratorReturn`,
        // `NoInfer`, …) are compiler built-ins with no user-modellable body.
        // Lower to `Unknown` so the alias resolves instead of dropping to TS2304;
        // the alias's nominal/generic surface is preserved by its declaration.
        TSType::TSIntrinsicKeyword(_) => Some(ParsedType::Unknown),
        TSType::TSNeverKeyword(_) => Some(ParsedType::Never),
        TSType::TSLiteralType(literal_type) => Some(parse_literal_type(literal_type)),
        TSType::TSTypeLiteral(type_literal) => Some(parse_type_literal(type_literal)),
        TSType::TSArrayType(array_type) => parse_type(&array_type.element_type)
            .map(|ty| ParsedType::Array(std::sync::Arc::new(ty))),
        TSType::TSTupleType(tuple_type) => parse_tuple_type(tuple_type),
        TSType::TSFunctionType(function_type) => parse_function_type(function_type)
            .map(|function| ParsedType::Function(std::sync::Arc::new(function))),
        // A constructor type (`new (args) => T`) becomes an object carrying only
        // a construct signature. Lowering it to a plain callable made
        // `T extends new (...args: any[]) => any` true for every function type,
        // which inverted the vitest `Mock<T>` conditional and made every mock
        // call a false TS2349.
        TSType::TSConstructorType(constructor_type) => {
            parse_constructor_type(constructor_type).map(|function| {
                ParsedType::Object(std::sync::Arc::new(ParsedObjectType {
                    properties: Vec::new(),
                    string_index_type: None,
                    number_index_type: None,
                    string_index_readonly: false,
                    number_index_readonly: false,
                    call_signature: None,
                    call_signature_overloads: Vec::new(),
                    construct_signature: Some(Box::new(function)),
                    construct_signature_overloads: Vec::new(),
                    non_primitive: false,
                    display_name: None,
                    keyed_indexes: None,
                    abstract_construct_signature: constructor_type.r#abstract,
                }))
            })
        }
        TSType::TSParenthesizedType(parenthesized_type) => {
            parse_type(&parenthesized_type.type_annotation)
        }
        // A JSDoc `T!` or `?T` written in TypeScript is TS17019/TS17020, and
        // tsc still reads it: `T`, or `T | null` (`getTypeFromTypeNode`).
        TSType::JSDocNonNullableType(non_nullable) => parse_type(&non_nullable.type_annotation),
        TSType::JSDocNullableType(nullable) => Some(ParsedType::Union(std::sync::Arc::new(vec![
            parse_type(&nullable.type_annotation)?,
            ParsedType::Null,
        ]))),
        TSType::TSUnionType(union_type) => Some(parse_union_type(union_type)),
        TSType::TSIntersectionType(intersection_type) => {
            Some(parse_intersection_type(intersection_type))
        }
        TSType::TSTypeReference(type_reference) => parse_type_reference(type_reference),
        TSType::TSTypeQuery(type_query) => parse_type_query(type_query),
        TSType::TSTypeOperatorType(type_operator) => parse_type_operator(type_operator),
        TSType::TSIndexedAccessType(indexed_access) => parse_indexed_access_type(indexed_access),
        TSType::TSMappedType(mapped_type) => parse_mapped_type(mapped_type),
        TSType::TSConditionalType(conditional_type) => parse_conditional_type(conditional_type),
        TSType::TSTemplateLiteralType(template_literal) => {
            parse_template_literal_type(template_literal)
        }
        // Polymorphic `this` (common in lib builder methods like `Map.set`,
        // `Array.prototype` chaining) is not modelled; fall back to `any` so the
        // member still parses rather than being dropped, keeping the surrounding
        // declaration intact and cascade-free.
        TSType::TSThisType(_) => Some(ParsedType::Any),
        // A type predicate (`x is T`) evaluates to `boolean` at the value level;
        // an assertion predicate (`asserts x`, `asserts x is T`) evaluates to
        // `void`. The payload (tested parameter + target type) is preserved so
        // guard narrowing can consume calls of the predicate.
        TSType::TSTypePredicate(predicate) => {
            let parameter_name = match &predicate.parameter_name {
                oxc_ast::ast::TSTypePredicateName::Identifier(identifier) => {
                    identifier.name.to_string()
                }
                oxc_ast::ast::TSTypePredicateName::This(_) => "this".to_string(),
            };
            Some(ParsedType::Predicate(std::sync::Arc::new(
                ParsedPredicateType {
                    parameter_name,
                    ty: predicate
                        .type_annotation
                        .as_ref()
                        .and_then(|annotation| parse_type(&annotation.type_annotation)),
                    type_span: predicate
                        .type_annotation
                        .as_ref()
                        .map(|annotation| text_span_from_oxc_span(annotation.type_annotation.span())),
                    asserts: predicate.asserts,
                },
            )))
        }
        // `infer X` in a conditional `extends` clause. Carrying the name (rather
        // than dropping to `None`) keeps the enclosing conditional alive; the
        // resolver treats the capture as a permissive hole.
        // `import("m").T`: the module's type export, bound in the importer
        // under the name written here (`import_type_name`).
        // An import attribute can pick another resolution mode for the
        // specifier, which the binding by specifier cannot follow.
        TSType::TSImportType(import_type) if import_type.options.is_some() => Some(ParsedType::Unknown),
        TSType::TSImportType(import_type) => {
            let mut members = Vec::new();
            let mut spans = Vec::new();
            if let Some(qualifier) = &import_type.qualifier {
                flatten_import_type_qualifier(qualifier, &mut members, &mut spans);
            }
            let type_arguments = match import_type.type_arguments.as_deref() {
                Some(arguments) => parse_type_arguments(arguments)?,
                None => Vec::new(),
            };
            Some(ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: crate::import_type_name(&import_type.source.value, &members),
                span: Some(text_span_from_oxc_span(import_type.span)),
                type_arguments,
            })))
        }
        TSType::TSInferType(infer_type) => Some(ParsedType::Infer(std::sync::Arc::new(
            crate::ParsedInferType {
                name: infer_type.type_parameter.name.name.to_string(),
                constraint: infer_type
                    .type_parameter
                    .constraint
                    .as_ref()
                    .and_then(parse_type),
            },
        ))),
        _ => None,
    }
}

/// Lowers a type-position template literal `` `a${X}b${Y}c` `` into its literal
/// segments plus the interpolated types. The `quasis` are the cooked string
/// pieces (always one more than the interpolation count). If any interpolation
/// uses a construct we cannot model, the whole template degrades to `Unknown`
/// so a reference to it resolves conservatively instead of vanishing and
/// cascading into `TS2304`.
///
/// This is distinct from expression-position template literals (parsed in
/// `parser/expressions.rs`); only `TSTemplateLiteralType` reaches here.
fn parse_template_literal_type(
    template_literal: &oxc_ast::ast::TSTemplateLiteralType<'_>,
) -> Option<ParsedType> {
    let mut quasis = Vec::with_capacity(template_literal.quasis.len());
    for quasi in &template_literal.quasis {
        let text = quasi
            .value
            .cooked
            .as_ref()
            .map(|cooked| cooked.to_string())
            .unwrap_or_else(|| quasi.value.raw.to_string());
        quasis.push(text);
    }

    let mut interpolations = Vec::with_capacity(template_literal.types.len());
    for interpolation in &template_literal.types {
        let Some(parsed) = parse_type(interpolation) else {
            return Some(ParsedType::Unknown);
        };
        interpolations.push(parsed);
    }

    Some(ParsedType::TemplateLiteral(std::sync::Arc::new(ParsedTemplateLiteralType {
        quasis,
        interpolations,
        span: Some(text_span_from_oxc_span(template_literal.span)),
    })))
}

/// Lowers `Check extends Extends ? True : False`. If any branch contains a
/// construct we do not model yet (e.g. nested `infer`), the whole conditional
/// degrades to `Unknown` so a reference to it resolves conservatively instead of
/// disappearing and cascading into `TS2304`.
fn parse_conditional_type(conditional_type: &TSConditionalType<'_>) -> Option<ParsedType> {
    let (Some(check_type), Some(extends_type), Some(true_type), Some(false_type)) = (
        parse_type(&conditional_type.check_type),
        parse_type(&conditional_type.extends_type),
        parse_type(&conditional_type.true_type),
        parse_type(&conditional_type.false_type),
    ) else {
        return Some(ParsedType::Unknown);
    };

    Some(ParsedType::Conditional(std::sync::Arc::new(ParsedConditionalType {
        check_type: Box::new(check_type),
        extends_type: Box::new(extends_type),
        true_type: Box::new(true_type),
        false_type: Box::new(false_type),
        span: Some(text_span_from_oxc_span(conditional_type.span)),
    })))
}

fn parse_type_query_arguments(type_query: &TSTypeQuery<'_>) -> Vec<ParsedType> {
    type_query
        .type_arguments
        .as_ref()
        .map(|arguments| {
            arguments
                .params
                .iter()
                .map(|argument| parse_type(argument).unwrap_or(ParsedType::Unknown))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_type_query(type_query: &TSTypeQuery<'_>) -> Option<ParsedType> {
    match &type_query.expr_name {
        TSTypeQueryExprName::IdentifierReference(identifier) => {
            Some(ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
                name: identifier.name.to_string(),
                name_span: Some(text_span_from_oxc_span(identifier.span)),
                members: Vec::new(),
                import_specifier: None,
                member_spans: Vec::new(),
                type_arguments: parse_type_query_arguments(type_query),
                type_arguments_span: type_query
                    .type_arguments
                    .as_ref()
                    .map(|arguments| text_span_from_oxc_span(arguments.span)),
            })))
        }
        TSTypeQueryExprName::QualifiedName(qualified_name) => {
            let mut members = Vec::new();
            // `typeof this.x` is not modelled either, but it is an annotation:
            // dropping it would read the binding as unannotated (TS7006).
            let Some((base, base_span)) = flatten_qualified_type_name(qualified_name, &mut members) else {
                return Some(ParsedType::Unknown);
            };
            Some(ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
                name: base,
                name_span: Some(text_span_from_oxc_span(base_span)),
                members,
                import_specifier: None,
                member_spans: Vec::new(),
                type_arguments: parse_type_query_arguments(type_query),
                type_arguments_span: type_query
                    .type_arguments
                    .as_ref()
                    .map(|arguments| text_span_from_oxc_span(arguments.span)),
            })))
        }
        // `typeof import("vitest")['assert']` reads the module's namespace value;
        // a qualifier after the call (`typeof import("m").ns.x`) walks it.
        TSTypeQueryExprName::TSImportType(import_type) => {
            let mut members = Vec::new();
            let mut member_spans = Vec::new();
            if let Some(qualifier) = &import_type.qualifier {
                flatten_import_type_qualifier(qualifier, &mut members, &mut member_spans);
            }
            let specifier = import_type.source.value.to_string();
            Some(ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
                name: format!("import(\"{specifier}\")"),
                name_span: Some(text_span_from_oxc_span(import_type.source.span)),
                members,
                import_specifier: Some(specifier),
                member_spans,
                type_arguments: parse_type_query_arguments(type_query),
                type_arguments_span: type_query
                    .type_arguments
                    .as_ref()
                    .map(|arguments| text_span_from_oxc_span(arguments.span)),
            })))
        }
        // `typeof this` is not modelled.
        _ => Some(ParsedType::Unknown),
    }
}

fn flatten_import_type_qualifier(
    qualifier: &oxc_ast::ast::TSImportTypeQualifier<'_>,
    members: &mut Vec<String>,
    spans: &mut Vec<crate::TextSpan>,
) {
    match qualifier {
        oxc_ast::ast::TSImportTypeQualifier::Identifier(identifier) => {
            members.push(identifier.name.to_string());
            spans.push(text_span_from_oxc_span(identifier.span));
        }
        oxc_ast::ast::TSImportTypeQualifier::QualifiedName(qualified) => {
            flatten_import_type_qualifier(&qualified.left, members, spans);
            members.push(qualified.right.name.to_string());
            spans.push(text_span_from_oxc_span(qualified.right.span));
        }
    }
}

/// Flattens a left-nested `TSQualifiedName` (`A.B.C`) into its leftmost
/// identifier (base) plus the trailing member segments in source order. Returns
/// `None` if the leftmost element is not a plain identifier (e.g. `this.x`),
/// which we do not model.
fn flatten_qualified_type_name(
    qualified_name: &oxc_ast::ast::TSQualifiedName<'_>,
    members: &mut Vec<String>,
) -> Option<(String, oxc_span::Span)> {
    let base = match &qualified_name.left {
        TSTypeName::IdentifierReference(identifier) => {
            (identifier.name.to_string(), identifier.span)
        }
        TSTypeName::QualifiedName(inner) => flatten_qualified_type_name(inner, members)?,
        TSTypeName::ThisExpression(_) => return None,
    };
    members.push(qualified_name.right.name.to_string());
    Some(base)
}

fn parse_type_operator(type_operator: &TSTypeOperator<'_>) -> Option<ParsedType> {
    match type_operator.operator {
        TSTypeOperatorOperator::Keyof => parse_type(&type_operator.type_annotation)
            .map(|ty| ParsedType::KeyOf(std::sync::Arc::new(ty))),
        // The operand is only ever an array or tuple shape (tsc rejects anything
        // else with TS1354); an operand that lowered to something else stays as
        // it is so the annotation is kept rather than dropped (which would
        // cascade into `TS7006`/`TS7031` on the annotated binding). `unique
        // symbol` is modelled as `symbol`; an equality test against the `const`
        // holding it still removes it (see `const_member_literal_value`).
        TSTypeOperatorOperator::Readonly => {
            let inner = parse_type(&type_operator.type_annotation)?;
            Some(match inner {
                ParsedType::Array(_) | ParsedType::Tuple(_) | ParsedType::VariadicTuple(_)
                    if readonly_array_types_enabled() =>
                {
                    ParsedType::Readonly(std::sync::Arc::new(inner))
                }
                other => other,
            })
        }
        TSTypeOperatorOperator::Unique => Some(ParsedType::Symbol),
    }
}

fn parse_indexed_access_type(indexed_access: &TSIndexedAccessType<'_>) -> Option<ParsedType> {
    let object_type = parse_type(&indexed_access.object_type)?;
    let index_type = parse_type(&indexed_access.index_type)?;

    Some(ParsedType::IndexedAccess(std::sync::Arc::new(ParsedIndexedAccessType {
        object_type: Box::new(object_type),
        index_type: Box::new(index_type),
        span: Some(text_span_from_oxc_span(indexed_access.span)),
        index_span: Some(text_span_from_oxc_span(indexed_access.index_type.span())),
    })))
}

/// A mapped type's modifiers have three distinct states: `-?` makes a property
/// required even when the homomorphic source's is optional, so it cannot fold
/// into `?`, and `readonly` works the same way.
fn parse_mapped_type(mapped_type: &TSMappedType<'_>) -> Option<ParsedType> {
    let name_type = match mapped_type.name_type.as_ref() {
        Some(name_type) => Some(Box::new(parse_type(name_type)?)),
        None => None,
    };

    let optional = match mapped_type.optional {
        Some(TSMappedTypeModifierOperator::True | TSMappedTypeModifierOperator::Plus) => {
            MappedOptionality::Add
        }
        Some(TSMappedTypeModifierOperator::Minus) => MappedOptionality::Remove,
        None => MappedOptionality::Keep,
    };
    let readonly = match mapped_type.readonly {
        Some(TSMappedTypeModifierOperator::True | TSMappedTypeModifierOperator::Plus) => {
            MappedOptionality::Add
        }
        Some(TSMappedTypeModifierOperator::Minus) => MappedOptionality::Remove,
        None => MappedOptionality::Keep,
    };

    let constraint = parse_type(&mapped_type.constraint)?;
    let value_type = match &mapped_type.type_annotation {
        Some(t) => parse_type(t)?,
        None => ParsedType::Any, // Though typically TS requires a type, fall back to Any or Unknown
    };

    Some(ParsedType::Mapped(std::sync::Arc::new(ParsedMappedType {
        key_name: mapped_type.key.name.to_string(),
        key_span: Some(text_span_from_oxc_span(mapped_type.key.span)),
        constraint: Box::new(constraint),
        value_type: Box::new(value_type),
        optional,
        readonly,
        name_type,
        span: Some(text_span_from_oxc_span(mapped_type.span)),
    })))
}

fn parse_type_reference(type_reference: &TSTypeReference<'_>) -> Option<ParsedType> {
    let (name, span) = flatten_type_name(&type_reference.type_name)?;

    let type_arguments = match type_reference.type_arguments.as_deref() {
        Some(type_arguments) => parse_type_arguments(type_arguments)?,
        None => Vec::new(),
    };

    Some(ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: type_reference
            .type_arguments
            .as_ref()
            .map(|arguments| {
                arguments.params.iter().map(|argument| text_span_from_oxc_span(argument.span())).collect()
            })
            .unwrap_or_default(),
        name,
        span: Some(span),
        type_arguments,
    })))
}

/// Flatten a (possibly qualified) type name into a dotted string and the span of
/// the head identifier, e.g. `React.ComponentProps` -> `"React.ComponentProps"`.
/// Mirrors how namespace members are registered under qualified keys.
pub(crate) fn flatten_type_name(type_name: &TSTypeName<'_>) -> Option<(String, crate::TextSpan)> {
    match type_name {
        TSTypeName::IdentifierReference(identifier) => Some((
            identifier.name.to_string(),
            text_span_from_oxc_span(identifier.span),
        )),
        TSTypeName::QualifiedName(qualified) => {
            let (left_name, left_span) = flatten_type_name(&qualified.left)?;
            Some((format!("{}.{}", left_name, qualified.right.name), left_span))
        }
        _ => None,
    }
}

/// Flatten a heritage clause expression (`extends NS.Member`) into the same
/// dotted name [`flatten_type_name`] produces for type references, so a
/// qualified base resolves through the namespace-member key instead of being
/// dropped. Computed members and non-identifier heads have no such key.
pub(super) fn flatten_heritage_expression(
    expression: &Expression<'_>,
) -> Option<(String, crate::TextSpan)> {
    match expression {
        Expression::Identifier(identifier) => Some((
            identifier.name.to_string(),
            text_span_from_oxc_span(identifier.span),
        )),
        Expression::StaticMemberExpression(member) => {
            let (object_name, object_span) = flatten_heritage_expression(&member.object)?;
            Some((
                format!("{}.{}", object_name, member.property.name),
                object_span,
            ))
        }
        _ => None,
    }
}

fn parse_literal_type(literal_type: &TSLiteralType<'_>) -> ParsedType {
    match &literal_type.literal {
        TSLiteral::StringLiteral(string_literal) => {
            ParsedType::StringLiteral(string_literal.value.to_string())
        }
        TSLiteral::NumericLiteral(numeric_literal) => {
            ParsedType::NumberLiteral(super::number_text::js_number_to_string(numeric_literal.value))
        }
        TSLiteral::BooleanLiteral(boolean_literal) => {
            ParsedType::BooleanLiteral(boolean_literal.value)
        }
        TSLiteral::UnaryExpression(unary_expression) => {
            match super::expressions::signed_number_literal_text(unary_expression) {
                Some(text) => ParsedType::NumberLiteral(text),
                None => ParsedType::Unknown,
            }
        }
        TSLiteral::BigIntLiteral(_) | TSLiteral::TemplateLiteral(_) => ParsedType::Unknown,
    }
}

fn parse_union_type(union_type: &TSUnionType<'_>) -> ParsedType {
    let mut types = Vec::new();

    for ty in &union_type.types {
        let Some(parsed_type) = parse_type(ty) else {
            return ParsedType::Unknown;
        };

        types.push(parsed_type);
    }

    ParsedType::Union(std::sync::Arc::new(types))
}

fn parse_intersection_type(intersection_type: &TSIntersectionType<'_>) -> ParsedType {
    let mut types = Vec::new();

    for ty in &intersection_type.types {
        let Some(parsed_type) = parse_type(ty) else {
            return ParsedType::Unknown;
        };

        types.push(parsed_type);
    }

    ParsedType::Intersection(std::sync::Arc::new(types))
}

/// Tuple labels are display-only, so named members lower to their element type.
/// A tuple carrying a rest element cannot state its length here, so it takes the
/// [`parse_variadic_tuple`] path instead of the fixed-length shape.
fn parse_tuple_type(tuple_type: &TSTupleType<'_>) -> Option<ParsedType> {
    let mut elements = Vec::new();

    for element in &tuple_type.element_types {
        match element {
            TSTupleElement::TSNamedTupleMember(member) => {
                if matches!(member.element_type, TSTupleElement::TSRestType(_)) {
                    return Some(parse_variadic_tuple(tuple_type));
                }
                let Some(inner) = member.element_type.as_ts_type() else {
                    return Some(ParsedType::Unknown);
                };
                let Some(parsed_element) = parse_type(inner) else {
                    return None;
                };

                elements.push(optional_tuple_element(parsed_element, member.optional));
            }
            TSTupleElement::TSOptionalType(optional) => {
                let Some(parsed_element) = parse_type(&optional.type_annotation) else {
                    return None;
                };
                elements.push(optional_tuple_element(parsed_element, true));
            }
            TSTupleElement::TSRestType(_) => {
                return Some(parse_variadic_tuple(tuple_type));
            }
            _ => {
                let Some(parsed_element) = parse_type(element.as_ts_type()?) else {
                    return None;
                };

                elements.push(optional_tuple_element(parsed_element, false));
            }
        }
    }

    Some(ParsedType::Tuple(std::sync::Arc::new(elements)))
}

/// An optional element reads as `T | undefined`, but tsc keeps its
/// optionality as a flag beside the type (`ElementFlags`), and the type alone
/// cannot carry it: `[a?: any]` reads `any`, while `[a: T | undefined]` is
/// required. So an optional element lowers to `undefined | T` with the
/// `undefined` first, and a written union keeps its `undefined` after the
/// other members, where the order means nothing else; see
/// [`ParsedType::is_optional_tuple_element`].
fn optional_tuple_element(element: ParsedType, optional: bool) -> ParsedType {
    if optional {
        return ParsedType::Union(std::sync::Arc::new(vec![ParsedType::Undefined, element]));
    }
    match element {
        ParsedType::Union(members) if matches!(members.first(), Some(ParsedType::Undefined)) => {
            let (undefined, mut written): (Vec<ParsedType>, Vec<ParsedType>) = members
                .iter()
                .cloned()
                .partition(|member| matches!(member, ParsedType::Undefined));
            if written.is_empty() {
                return ParsedType::Undefined;
            }
            written.extend(undefined);
            ParsedType::Union(std::sync::Arc::new(written))
        }
        other => other,
    }
}

impl ParsedType {
    /// Whether this tuple element was written optional (`[a?: T]`, `[T?]`),
    /// as [`optional_tuple_element`] lowers it.
    pub fn is_optional_tuple_element(&self) -> bool {
        matches!(self, ParsedType::Union(members) if matches!(members.first(), Some(ParsedType::Undefined)))
    }

    /// tsc's `minLength` of a written tuple without a rest element: the
    /// number of its required elements.
    pub fn tuple_min_length(elements: &[ParsedType]) -> usize {
        elements
            .iter()
            .filter(|element| !element.is_optional_tuple_element())
            .count()
    }
}

/// A tuple with a rest element. `[...T]` is `T`; the homogeneous idiom is the
/// array it always was (see below); everything else — `[...a, ...b]`,
/// `[infer head, ...infer tail]`, `['x', ...number[]]` — keeps its elements and
/// resolves to a fixed or open tuple.
fn parse_variadic_tuple(tuple_type: &TSTupleType<'_>) -> ParsedType {
    // `[...T]` spreads a tuple *type parameter*: it is `T` itself, not `T[]`.
    if let [TSTupleElement::TSRestType(rest)] = tuple_type.element_types.as_slice()
        && !matches!(rest.type_annotation, TSType::TSArrayType(_))
    {
        return parse_type(&rest.type_annotation).unwrap_or(ParsedType::Unknown);
    }
    // `[...T[]]` normalizes to `T[]` (`createNormalizedTupleType`); any fixed
    // element keeps the tuple, whose minimum length and element positions
    // the array cannot state.
    let lone_rest = matches!(tuple_type.element_types.as_slice(), [TSTupleElement::TSRestType(_)]);
    match if lone_rest { homogeneous_variadic_tuple(tuple_type) } else { ParsedType::Unknown } {
        ParsedType::Unknown if variadic_tuple_elements_enabled() => {
            match variadic_tuple_elements(tuple_type) {
                Some(elements) => ParsedType::VariadicTuple(std::sync::Arc::new(elements)),
                None => ParsedType::Unknown,
            }
        }
        lowered => lowered,
    }
}

/// `SURGE_VARIADIC_TUPLES=0` restores the pre-element behaviour: a tuple the
/// homogeneous lowerings cannot express degrades to `Unknown` wholesale. Kept as
/// the off switch for the A/B this change has to carry, since turning the shape
/// on changes which conditional branches are reachable across every corpus.
/// `SURGE_READONLY_ARRAYS=0` lowers `readonly T[]` / `readonly [A, B]` to the
/// mutable shape again, the behaviour before the modifier was modelled.
fn readonly_array_types_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_READONLY_ARRAYS").as_deref() != Ok("0"))
}

fn variadic_tuple_elements_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_VARIADIC_TUPLES").as_deref() != Ok("0"))
}

/// The spread type of a rest element; a labeled one (`...rest: T[]`) holds
/// it inside the label.
fn rest_type_annotation<'b, 'a>(rest: &'b oxc_ast::ast::TSRestType<'a>) -> Option<&'b TSType<'a>> {
    match &rest.type_annotation {
        TSType::TSNamedTupleMember(member) => member.element_type.as_ts_type(),
        other => Some(other),
    }
}

fn variadic_tuple_elements(tuple_type: &TSTupleType<'_>) -> Option<Vec<ParsedTupleElement>> {
    let mut elements = Vec::with_capacity(tuple_type.element_types.len());

    for element in &tuple_type.element_types {
        match element {
            TSTupleElement::TSRestType(rest) => {
                elements.push(ParsedTupleElement::Rest(
                    parse_type(rest_type_annotation(rest)?)?,
                    Some(text_span_from_oxc_span(rest.span)),
                ));
            }
            TSTupleElement::TSOptionalType(optional) => {
                let parsed = parse_type(&optional.type_annotation)?;
                elements.push(ParsedTupleElement::Fixed(optional_tuple_element(
                    parsed, true,
                )));
            }
            TSTupleElement::TSNamedTupleMember(member) => match &member.element_type {
                TSTupleElement::TSRestType(rest) => {
                    elements.push(ParsedTupleElement::Rest(
                    parse_type(&rest.type_annotation)?,
                    Some(text_span_from_oxc_span(rest.span)),
                ));
                }
                other => {
                    let parsed = parse_type(other.as_ts_type()?)?;
                    elements.push(ParsedTupleElement::Fixed(optional_tuple_element(
                        parsed,
                        member.optional,
                    )));
                }
            },
            other => {
                elements.push(ParsedTupleElement::Fixed(optional_tuple_element(
                    parse_type(other.as_ts_type()?)?,
                    false,
                )));
            }
        }
    }

    Some(elements)
}

/// A variadic tuple whose elements are all the *same* type lowered to that
/// type's array; `Unknown` otherwise.
fn homogeneous_variadic_tuple(tuple_type: &TSTupleType<'_>) -> ParsedType {
    // `[...T]` spreads a tuple *type parameter*: it is `T` itself, not `T[]`.
    if let [TSTupleElement::TSRestType(rest)] = tuple_type.element_types.as_slice()
        && !matches!(rest.type_annotation, TSType::TSArrayType(_))
    {
        return parse_type(&rest.type_annotation).unwrap_or(ParsedType::Unknown);
    }

    let mut element_type: Option<ParsedType> = None;

    for element in &tuple_type.element_types {
        let inner = match element {
            // A rest element only contributes an element type when it is written
            // `...T[]`; `...T` spreads a whole tuple whose element types this
            // shape cannot express.
            TSTupleElement::TSNamedTupleMember(member) => match &member.element_type {
                TSTupleElement::TSRestType(rest) => match &rest.type_annotation {
                    TSType::TSArrayType(array) => Some(&array.element_type),
                    _ => return ParsedType::Unknown,
                },
                other => other.as_ts_type(),
            },
            TSTupleElement::TSRestType(rest) => match rest_type_annotation(rest) {
                Some(TSType::TSArrayType(array)) => Some(&array.element_type),
                _ => return ParsedType::Unknown,
            },
            TSTupleElement::TSOptionalType(optional) => Some(&optional.type_annotation),
            other => other.as_ts_type(),
        };
        let Some(inner) = inner else {
            return ParsedType::Unknown;
        };
        let Some(parsed) = parse_type(inner) else {
            return ParsedType::Unknown;
        };
        match &element_type {
            Some(existing) if !same_tuple_element_type(existing, &parsed) => {
                return ParsedType::Unknown;
            }
            Some(_) => {}
            None => element_type = Some(parsed),
        }
    }

    match element_type {
        Some(element_type) => ParsedType::Array(std::sync::Arc::new(element_type)),
        None => ParsedType::Unknown,
    }
}

/// Structural equality for tuple elements that ignores name spans: the `T` of
/// `[T, ...T[]]` is the same type written twice, and `ParsedNamedType`'s derived
/// equality would separate them on position alone.
fn same_tuple_element_type(left: &ParsedType, right: &ParsedType) -> bool {
    match (left, right) {
        (ParsedType::Named(left), ParsedType::Named(right)) => {
            left.name == right.name
                && left.type_arguments.len() == right.type_arguments.len()
                && left
                    .type_arguments
                    .iter()
                    .zip(right.type_arguments.iter())
                    .all(|(left, right)| same_tuple_element_type(left, right))
        }
        (ParsedType::Array(left), ParsedType::Array(right)) => same_tuple_element_type(left, right),
        _ => left == right,
    }
}

fn parse_type_literal(type_literal: &TSTypeLiteral<'_>) -> ParsedType {
    let mut properties = Vec::new();
    let mut string_index_type: Option<Box<ParsedType>> = None;
    let mut keyed_indexes = KeyedIndexCollector::default();
    let mut number_index_type: Option<Box<ParsedType>> = None;
    let mut string_index_readonly = false;
    let mut number_index_readonly = false;
    let mut call_signature: Option<Box<ParsedFunctionType>> = None;
    let mut call_signature_overloads: Vec<ParsedFunctionType> = Vec::new();
    let mut construct_signature: Option<Box<ParsedFunctionType>> = None;
    let mut construct_signature_overloads: Vec<ParsedFunctionType> = Vec::new();
    let getters = getter_accessor_names(&type_literal.members);

    for member in &type_literal.members {
        if is_shadowed_setter(member, &getters) {
            continue;
        }

        let property = match member {
            TSSignature::TSPropertySignature(property_signature) => {
                parse_type_property_signature(property_signature)
            }
            TSSignature::TSMethodSignature(method_signature) => {
                parse_type_method_signature(method_signature)
            }
            TSSignature::TSCallSignatureDeclaration(signature) => {
                // Same-shaped overloads fold into one permissive signature
                // (max parameters, everything past a shorter overload's arity
                // optional), mirroring the interface overload merge — a callable
                // type literal like expect-type's `_ExpectTypeOf` declares both
                // `<T>(actual: T): …` and `<T>(): …`, and keeping only the
                // first made every zero-argument call a false TS2554.
                if let Some(parsed) = parse_call_signature(signature) {
                    call_signature_overloads.push(parsed.clone());
                    call_signature = Some(match call_signature.take() {
                        Some(existing) => {
                            Box::new(merge_parsed_call_signatures(&existing, &parsed))
                        }
                        None => Box::new(parsed),
                    });
                }
                continue;
            }
            TSSignature::TSConstructSignatureDeclaration(signature) => {
                // `{ new (...): T }` — without this arm the whole type literal
                // degraded to `Unknown`, which is what made vitest's `Mock<T>`
                // (a conditional branch object carrying both a construct and a
                // call signature) uncallable.
                if let Some(parsed) = parse_construct_signature(signature) {
                    construct_signature_overloads.push(parsed.clone());
                    construct_signature = Some(match construct_signature.take() {
                        Some(existing) => {
                            Box::new(merge_parsed_call_signatures(&existing, &parsed))
                        }
                        None => Box::new(parsed),
                    });
                }
                continue;
            }
            TSSignature::TSIndexSignature(index_signature) => {
                let slots = index_signature_slots(index_signature);
                keyed_indexes.add(&slots, index_signature);
                let fills_string = slots.string && string_index_type.is_none();
                let fills_number = slots.number && number_index_type.is_none();
                if (fills_string || fills_number)
                    && let Some(value_type) = parse_index_signature_value_type(index_signature)
                {
                    if fills_number {
                        number_index_type = Some(Box::new(value_type.clone()));
                        number_index_readonly = index_signature.readonly;
                    }
                    if fills_string {
                        string_index_type = Some(Box::new(value_type));
                        string_index_readonly = index_signature.readonly;
                    }
                }
                continue;
            }
            // Exhaustive against today's oxc; kept so a new `TSSignature`
            // variant degrades to `Unknown` instead of failing the build.
            #[allow(unreachable_patterns)]
            _ => return ParsedType::Unknown,
        };

        let Some(property) = property else {
            return ParsedType::Unknown;
        };

        properties.push(property);
    }

    attach_accessor_write_types(&mut properties, &setter_accessor_types(&type_literal.members));

    ParsedType::Object(std::sync::Arc::new(ParsedObjectType {
        properties,
        string_index_type,
        number_index_type,
        string_index_readonly,
        number_index_readonly,
        call_signature,
        // One signature is already the whole story; the fold is lossless there.
        call_signature_overloads: if call_signature_overloads.len() > 1 {
            call_signature_overloads
        } else {
            Vec::new()
        },
        construct_signature,
        construct_signature_overloads: if construct_signature_overloads.len() > 1 {
            construct_signature_overloads
        } else {
            Vec::new()
        },
        non_primitive: false,
        display_name: None,
        abstract_construct_signature: false,
        keyed_indexes: keyed_indexes.finish(),
    }))
}

/// Lowers a bare call signature (`(value?: any): number`) into a
/// [`ParsedFunctionType`], shared by interface and object-type-literal parsing.
/// Folds two call-signature overloads into one permissive signature: the longer
/// parameter list wins, a position typed differently across overloads widens to
/// the degradation sentinel, and a position absent (or optional) in either
/// overload is optional.
/// The return type is taken from the first overload. This mirrors the checker's
/// `merge_overload_signatures`, applied at parse time because a type literal
/// stores a single call signature.
pub fn merge_parsed_call_signatures(
    a: &ParsedFunctionType,
    b: &ParsedFunctionType,
) -> ParsedFunctionType {
    let (longer, shorter) = if a.parameters.len() >= b.parameters.len() {
        (a, b)
    } else {
        (b, a)
    };

    // The merged arity floor is the smaller overload's required count: a call
    // that satisfies either overload must pass (`<E>(v: E): true` merged with
    // `<E>(): true` requires nothing).
    let required = |signature: &ParsedFunctionType| {
        signature
            .parameters
            .iter()
            .filter(|parameter| !parameter.is_this)
            .take_while(|parameter| !parameter.optional && !parameter.rest)
            .count()
    };
    let min_required = required(a).min(required(b));

    let parameters = longer
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let mut merged = parameter.clone();
            if let Some(other) = shorter.parameters.get(index)
                && other.ty != merged.ty
            {
                // The degradation sentinel, not `any`: the overloads really do
                // constrain this slot, surge just cannot say how. `any` claims
                // the source wrote no contextual type, so a callback nested in
                // the argument (`new ReadableStream({ start(controller) {} })`,
                // whose three `new` overloads disagree on the source object)
                // became a false implicit-any report.
                merged.ty = ParsedType::Unknown;
            }
            if index >= min_required && !merged.rest {
                merged.optional = true;
            }
            merged
        })
        .collect();

    // Return type and type parameters must come from the SAME overload as the
    // parameter list: mixing them leaves the return type referencing the other
    // overload's type-parameter names (ky's `json` mixes `JsonType` and
    // `Schema`), which surfaces as a false TS2304.
    ParsedFunctionType {
        parameters,
        return_type: longer.return_type.clone(),
        type_parameters: longer.type_parameters.clone(),
    }
}

pub(crate) fn parse_call_signature(
    signature: &oxc_ast::ast::TSCallSignatureDeclaration<'_>,
) -> Option<ParsedFunctionType> {
    let mut parameters = signature
        .params
        .items
        .iter()
        .map(parse_function_type_parameter)
        .collect::<Option<Vec<_>>>()?;

    if let Some(rest) = signature.params.rest.as_deref() {
        parameters.push(parse_function_type_rest_parameter(rest)?);
    }

    // No return type is an implicit `any` (TS7020/TS7013 under
    // `noImplicitAny`), not a signature to drop.
    let return_type = match signature.return_type.as_ref() {
        Some(annotation) => parse_type_annotation(annotation.as_ref())?,
        None => ParsedType::Any,
    };

    Some(ParsedFunctionType {
        parameters,
        return_type: Box::new(return_type),
        type_parameters: parse_type_parameters(signature.type_parameters.as_deref()),
    })
}

pub(crate) fn parse_construct_signature(
    signature: &oxc_ast::ast::TSConstructSignatureDeclaration<'_>,
) -> Option<ParsedFunctionType> {
    let mut parameters = signature
        .params
        .items
        .iter()
        .map(parse_function_type_parameter)
        .collect::<Option<Vec<_>>>()?;

    if let Some(rest) = signature.params.rest.as_deref() {
        parameters.push(parse_function_type_rest_parameter(rest)?);
    }

    // No return type is an implicit `any` (TS7020/TS7013 under
    // `noImplicitAny`), not a signature to drop.
    let return_type = match signature.return_type.as_ref() {
        Some(annotation) => parse_type_annotation(annotation.as_ref())?,
        None => ParsedType::Any,
    };

    Some(ParsedFunctionType {
        parameters,
        return_type: Box::new(return_type),
        type_parameters: parse_type_parameters(signature.type_parameters.as_deref()),
    })
}

/// Names declared with a `get` accessor. A `set` accessor for the same name is
/// dropped so the read type wins, which is what tsc reports for a pair whose
/// getter and setter types differ.
pub(crate) fn getter_accessor_names<'a>(
    members: &'a [TSSignature<'a>],
) -> std::collections::HashSet<&'a str> {
    members
        .iter()
        .filter_map(|member| match member {
            TSSignature::TSMethodSignature(signature)
                if signature.kind == TSMethodSignatureKind::Get =>
            {
                match &signature.key {
                    PropertyKey::StaticIdentifier(key) => Some(key.name.as_str()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// The parameter type of every `set` accessor in a member list, by name. A
/// setter shadowed by a getter of the same name is dropped from the member
/// list, so this is where the pair's *write* type is recovered — tsc keeps the
/// two apart as `getTypeOfSymbol` and `getWriteTypeOfSymbol`.
pub(crate) fn setter_accessor_types(
    members: &[TSSignature<'_>],
) -> std::collections::HashMap<String, ParsedType> {
    members
        .iter()
        .filter_map(|member| match member {
            TSSignature::TSMethodSignature(signature)
                if signature.kind == TSMethodSignatureKind::Set =>
            {
                let PropertyKey::StaticIdentifier(key) = &signature.key else {
                    return None;
                };
                let parameter_type = signature
                    .params
                    .items
                    .first()
                    .and_then(|parameter| parameter.type_annotation.as_ref())
                    .and_then(|annotation| parse_type_annotation(annotation))?;
                Some((key.name.to_string(), parameter_type))
            }
            _ => None,
        })
        .collect()
}

/// Folds each `set` accessor's parameter type into the getter that shadowed it:
/// the member stops being read-only, and carries the setter's type as its write
/// type when the two differ.
pub(crate) fn attach_accessor_write_types(
    properties: &mut [ParsedObjectTypeProperty],
    setters: &std::collections::HashMap<String, ParsedType>,
) {
    for property in properties {
        let Some(setter_type) = setters.get(&property.name) else {
            continue;
        };
        property.readonly = false;
        property.write_ty = (property.ty != *setter_type).then(|| setter_type.clone());
    }
}

pub(crate) fn is_shadowed_setter(
    member: &TSSignature<'_>,
    getters: &std::collections::HashSet<&str>,
) -> bool {
    match member {
        TSSignature::TSMethodSignature(signature)
            if signature.kind == TSMethodSignatureKind::Set =>
        {
            matches!(
                &signature.key,
                PropertyKey::StaticIdentifier(key) if getters.contains(key.name.as_str())
            )
        }
        _ => false,
    }
}

/// Lowers a method signature (`foo(arg: A): R`) into a property whose type is a
/// [`ParsedType::Function`], so method calls reuse the existing function-type property
/// checking. Shared by interface and object-type-literal parsing.
pub(crate) fn parse_type_method_signature(
    method_signature: &TSMethodSignature<'_>,
) -> Option<ParsedObjectTypeProperty> {
    let (name, name_span) = if method_signature.computed {
        // Only a plain method lowers here; a computed accessor pair is rare
        // enough to keep dropping. `[matcher](): …` is the one member of
        // ts-pattern's `Matcher`, and dropping it made every object a Matcher.
        if method_signature.kind != TSMethodSignatureKind::Method {
            return None;
        }
        (computed_key_name(&method_signature.key)?, None)
    } else {
        // A numeric or quoted method name is a member like any other, named as
        // a property signature's is: `6(): string` declares `6`.
        match &method_signature.key {
            PropertyKey::StaticIdentifier(key) => {
                (key.name.to_string(), Some(text_span_from_oxc_span(key.span)))
            }
            PropertyKey::NumericLiteral(literal) => (
                super::number_text::js_number_to_string(literal.value),
                Some(text_span_from_oxc_span(literal.span)),
            ),
            PropertyKey::StringLiteral(literal) => {
                (literal.value.to_string(), Some(text_span_from_oxc_span(literal.span)))
            }
            _ => return None,
        }
    };

    // A `get`/`set` accessor lowers to a plain property: the getter's return
    // type, or the setter's parameter type when only a setter is declared. The
    // pair is written with differing types across the DOM lib
    // (`get location(): Location; set location(href: string)`), so dropping
    // them left `window.location` unresolved.
    if method_signature.kind != TSMethodSignatureKind::Method {
        let accessor_type = match method_signature.kind {
            TSMethodSignatureKind::Get => method_signature
                .return_type
                .as_ref()
                .and_then(|annotation| parse_type_annotation(annotation.as_ref()))?,
            _ => method_signature
                .params
                .items
                .first()
                .and_then(|parameter| parameter.type_annotation.as_ref())
                .and_then(|annotation| parse_type_annotation(annotation))?,
        };

        return Some(ParsedObjectTypeProperty {
            name,
            name_span,
            optional: false,
            is_method: false,
            // A getter is read-only until a setter of the same name merges into
            // it; the merge (in the interface/type-literal member list) is what
            // clears this and records the write type.
            readonly: method_signature.kind == TSMethodSignatureKind::Get,
            write_ty: None,
            ty: accessor_type,
        });
    }

    let mut parameters = method_signature
        .this_param
        .as_deref()
        .and_then(super::function_types::parse_this_parameter)
        .into_iter()
        .map(Some)
        .chain(method_signature.params.items.iter().map(parse_function_type_parameter))
        .collect::<Option<Vec<_>>>()?;

    if let Some(rest) = method_signature.params.rest.as_deref() {
        parameters.push(parse_function_type_rest_parameter(rest)?);
    }

    // A method signature written without a return type returns an implicit
    // `any` (and is TS7010 under `noImplicitAny`); dropping the member instead
    // made every use of it a missing property.
    let return_type = match method_signature.return_type.as_ref() {
        Some(annotation) => parse_type_annotation(annotation.as_ref())?,
        None => ParsedType::Any,
    };

    Some(ParsedObjectTypeProperty {
        name,
        name_span,
        ty: ParsedType::Function(std::sync::Arc::new(ParsedFunctionType {
            parameters,
            return_type: Box::new(return_type),
            type_parameters: parse_type_parameters(method_signature.type_parameters.as_deref()),
        })),
        optional: method_signature.optional,
        readonly: false,
        write_ty: None,
        is_method: true,
    })
}

/// Extract the value type of an index signature (`[key: string]: T` -> `T`).
/// String and number index signatures are not distinguished; both map to the
/// object's string index type.
pub(crate) fn parse_index_signature_value_type(
    index_signature: &oxc_ast::ast::TSIndexSignature<'_>,
) -> Option<ParsedType> {
    parse_type(&index_signature.type_annotation.type_annotation)
}

thread_local! {
    /// What a `this` parameter typed `this` names while a class or interface
    /// body is parsed: the declaring type over its own type parameters.
    static DECLARING_TYPE: std::cell::RefCell<Option<ParsedType>> = const { std::cell::RefCell::new(None) };
}

/// Parses `f` with `name<type_parameters>` as the declaring type. Surge does
/// not model the polymorphic `this`; for a `this` parameter the declaring type
/// is what every receiver a member is read from relates to.
pub(crate) fn with_declaring_type<R>(
    name: &str,
    type_parameters: Option<&TSTypeParameterDeclaration<'_>>,
    f: impl FnOnce() -> R,
) -> R {
    let declaring = ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        name: name.to_string(),
        span: None,
        type_arguments: type_parameters
            .map(|declaration| {
                declaration
                    .params
                    .iter()
                    .map(|parameter| {
                        ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                            name: parameter.name.name.to_string(),
                            span: None,
                            type_arguments: Vec::new(),
                            type_argument_spans: Vec::new(),
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        type_argument_spans: Vec::new(),
    }));
    let outer = DECLARING_TYPE.with(|current| current.replace(Some(declaring)));
    let result = f();
    DECLARING_TYPE.with(|current| *current.borrow_mut() = outer);
    result
}

/// Whether a `this` parameter's annotation is `this` in a class or interface
/// member, which [`parse_this_parameter_annotation`] reads as the declaring type.
pub(crate) fn is_polymorphic_this_annotation(annotation: &oxc_ast::ast::TSTypeAnnotation<'_>) -> bool {
    matches!(annotation.type_annotation, TSType::TSThisType(_))
        && DECLARING_TYPE.with(|current| current.borrow().is_some())
}

/// A `this` parameter's annotation, `this` itself read as the declaring type.
pub(crate) fn parse_this_parameter_annotation(annotation: &oxc_ast::ast::TSTypeAnnotation<'_>) -> Option<ParsedType> {
    if matches!(annotation.type_annotation, TSType::TSThisType(_))
        && let Some(declaring) = DECLARING_TYPE.with(|current| current.borrow().clone())
    {
        return Some(declaring);
    }
    parse_type_annotation(annotation)
}

pub(crate) struct IndexSignatureSlots {
    pub(crate) string: bool,
    pub(crate) number: bool,
    pub(crate) string_keyword: bool,
    pub(crate) symbol: bool,
    /// Key constituents other than the `string`, `number` and `symbol`
    /// keywords; `None` once one of them cannot be parsed.
    pub(crate) keyed: Option<Vec<ParsedType>>,
}

/// The index slots an index signature declares. tsc gives every constituent of
/// the key type its own index info, the first declaration per key type winning
/// (`getIndexInfosOfIndexSymbol`): `[k: string | number]` declares both, and a
/// `symbol` constituent declares a symbol index that answers no string or
/// number key. Surge has no symbol slot, so that constituent fills nothing;
/// pattern and aliased keys fill the string slot and are listed in `keyed`.
pub(crate) fn index_signature_slots(
    index_signature: &oxc_ast::ast::TSIndexSignature<'_>,
) -> IndexSignatureSlots {
    fn visit(key: &TSType<'_>, slots: &mut IndexSignatureSlots) {
        match key {
            TSType::TSUnionType(union) => {
                union.types.iter().for_each(|member| visit(member, slots));
            }
            TSType::TSParenthesizedType(inner) => visit(&inner.type_annotation, slots),
            TSType::TSNumberKeyword(_) => slots.number = true,
            TSType::TSSymbolKeyword(_) => slots.symbol = true,
            TSType::TSStringKeyword(_) => {
                slots.string = true;
                slots.string_keyword = true;
            }
            _ => {
                slots.string = true;
                match parse_type(key) {
                    Some(parsed) => {
                        if let Some(keyed) = slots.keyed.as_mut() {
                            keyed.push(parsed);
                        }
                    }
                    None => slots.keyed = None,
                }
            }
        }
    }
    let mut slots = IndexSignatureSlots {
        string: false,
        number: false,
        string_keyword: false,
        symbol: false,
        keyed: Some(Vec::new()),
    };
    if let Some(parameter) = index_signature.parameters.first() {
        visit(&parameter.type_annotation.type_annotation, &mut slots);
    }
    slots
}

/// Accumulates [`ParsedKeyedIndexes`] across an object's index signatures.
#[derive(Default)]
pub(crate) struct KeyedIndexCollector {
    signatures: Vec<ParsedKeyedIndexSignature>,
    disabled: bool,
    symbol: Option<ParsedType>,
}

impl KeyedIndexCollector {
    pub(crate) fn add(
        &mut self,
        slots: &IndexSignatureSlots,
        index_signature: &oxc_ast::ast::TSIndexSignature<'_>,
    ) {
        let value = parse_index_signature_value_type(index_signature);
        if slots.symbol && self.symbol.is_none() {
            self.symbol = value.clone();
        }
        if slots.string_keyword {
            self.disabled = true;
        }
        if self.disabled {
            return;
        }
        let (Some(keys), Some(value)) = (slots.keyed.as_ref(), value) else {
            self.disabled = slots.keyed.is_none() || slots.string;
            return;
        };
        self.signatures.extend(keys.iter().map(|key| ParsedKeyedIndexSignature {
            key: key.clone(),
            value: value.clone(),
        }));
    }

    pub(crate) fn finish(self) -> Option<std::sync::Arc<ParsedKeyedIndexes>> {
        let signatures = if self.disabled { Vec::new() } else { self.signatures };
        (!signatures.is_empty() || self.symbol.is_some()).then(|| {
            std::sync::Arc::new(ParsedKeyedIndexes {
                signatures,
                symbol: self.symbol,
            })
        })
    }
}

/// Whether an index signature is keyed by `number`. tsc keeps the two kinds
/// apart: a numeric key prefers the number signature, and a string key a
/// number-only type cannot answer is an implicit `any`.
pub(crate) fn index_signature_is_numeric(
    index_signature: &oxc_ast::ast::TSIndexSignature<'_>,
) -> bool {
    index_signature.parameters.first().is_some_and(|parameter| {
        matches!(
            parameter.type_annotation.type_annotation,
            oxc_ast::ast::TSType::TSNumberKeyword(_)
        )
    })
}

pub(crate) fn parse_type_parameters(
    type_parameters: Option<&TSTypeParameterDeclaration<'_>>,
) -> Vec<ParsedTypeParameter> {
    type_parameters
        .map(|type_parameters| {
            type_parameters
                .params
                .iter()
                .map(parse_type_parameter)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_type_parameter(type_parameter: &TSTypeParameter<'_>) -> ParsedTypeParameter {
    ParsedTypeParameter {
        name: type_parameter.name.name.to_string(),
        name_span: Some(text_span_from_oxc_span(type_parameter.name.span)),
        constraint: type_parameter.constraint.as_ref().and_then(parse_type),
        default_type: type_parameter.default.as_ref().and_then(parse_type),
        span: Some(text_span_from_oxc_span(type_parameter.span)),
        is_const: type_parameter.r#const,
        is_in: type_parameter.r#in,
        is_out: type_parameter.out,
    }
}

pub(crate) fn parse_type_arguments(
    type_arguments: &TSTypeParameterInstantiation<'_>,
) -> Option<Vec<ParsedType>> {
    type_arguments.params.iter().map(parse_type).collect()
}

/// A computed member key (`[Symbol.iterator]`, `[matcher]`, `[symbols.select]`)
/// rendered as a name no written identifier can collide with. Symbol keys
/// themselves are not modelled; the name only keeps the member from vanishing,
/// which turned every such type literal or interface into `{}`.
pub(crate) fn computed_key_name(key: &PropertyKey<'_>) -> Option<String> {
    fn render(expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::Identifier(identifier) => Some(identifier.name.to_string()),
            Expression::StaticMemberExpression(member) => {
                Some(format!("{}.{}", render(&member.object)?, member.property.name))
            }
            _ => None,
        }
    }
    match key {
        PropertyKey::StaticIdentifier(_) | PropertyKey::PrivateIdentifier(_) => None,
        // A computed key that is a literal is just that property name —
        // `{ ['a']: T }` and `{ 'a': T }` declare the same member. Dropping it
        // emptied every interface written that way, which is how zustand's
        // `interface StoreMutators<S, A> { ['zustand/immer']: WithImmer<S> }`
        // augmentation contributed nothing at all.
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::NumericLiteral(literal) => Some(super::number_text::js_number_to_string(literal.value)),
        // `[-1]` names the property `-1`, as a written literal key would.
        PropertyKey::UnaryExpression(unary) => {
            super::expressions::signed_number_literal_text(unary)
        }
        // A template without substitutions is a string literal too.
        PropertyKey::TemplateLiteral(template) if template.expressions.is_empty() => template
            .quasis
            .first()
            .and_then(|quasi| quasi.value.cooked.as_ref())
            .map(|cooked| cooked.to_string()),
        other => render(other.to_expression()).map(|path| format!("[{path}]")),
    }
}

pub(crate) fn parse_type_property_signature(
    property_signature: &TSPropertySignature<'_>,
) -> Option<ParsedObjectTypeProperty> {
    // `readonly` is not modelled for assignability, but the property must still
    // be present on the type. Standard lib interfaces (DOM `Response.ok`,
    // `URL`, etc.) rely heavily on `readonly` members, so parse them as ordinary
    // properties rather than dropping them.
    if property_signature.computed {
        // A well-known-symbol member (`{ readonly [Symbol.toStringTag]: string }`)
        // is the whole content of some type literals. Dropping it left an *empty*
        // object, which every object satisfies — so a guard written as
        // `o extends { [Symbol.toStringTag]: string }` matched everything and its
        // false arm became unreachable. Symbol keys are not modelled, but the
        // member is kept under a name no identifier can spell, which is enough to
        // stop the literal being vacuous.
        return computed_key_name(&property_signature.key).and_then(|name| {
            let type_annotation = property_signature
                .type_annotation
                .as_ref()
                .and_then(|annotation| parse_type_annotation(annotation))?;
            // A well-known symbol is a bindable name tsc reports members at
            // (TS2411 against a `symbol` index).
            let name_span = name
                .starts_with("[Symbol.")
                .then(|| text_span_from_oxc_span(property_signature.key.span()));
            Some(ParsedObjectTypeProperty {
                name,
                name_span,
                ty: type_annotation,
                optional: property_signature.optional,
                is_method: false,
                readonly: property_signature.readonly,
                write_ty: None,
            })
        });
    }

    // A numeric key is a property name like any other: numeric-key tables
    // (`{ 0: 1; 1: 0 }[B]`, the shape Prisma's generated `Not<B>` uses) must not
    // drop the member — losing one collapses the whole type literal to
    // `unknown`, which then reports the index as invalid. Quoted string keys stay
    // unsupported: admitting them resolves shapes whose members surge models
    // incompletely. Re-measured 2026-08-19 it is a wash (zod +1, trpc -1) — the
    // `"~standard"` members it unblocks sit on interfaces whose `Parameters<…>`
    // chain degrades to `{}` regardless.
    let (name, key_span) = match &property_signature.key {
        PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
        // A numeric name is the number's canonical string: `1.0` and `1.`
        // name the same member as `1`.
        PropertyKey::NumericLiteral(literal) => {
            (super::number_text::js_number_to_string(literal.value), literal.span)
        }
        PropertyKey::StringLiteral(literal) => (literal.value.to_string(), literal.span),
        _ => return None,
    };

    // A member written without a type (`k;`) is an implicit `any` (TS7008), not
    // an absent member: dropping it reported every read of it as TS2339 and
    // left the interface requiring nothing.
    let type_annotation = match property_signature.type_annotation.as_ref() {
        Some(annotation) => parse_type_annotation(annotation)?,
        None => ParsedType::Any,
    };

    Some(ParsedObjectTypeProperty {
        name,
        name_span: Some(text_span_from_oxc_span(key_span)),
        ty: type_annotation,
        optional: property_signature.optional,
        is_method: false,
        // tsc's `isReadonlySymbol`: a `readonly` member rejects every write,
        // whatever its type.
        readonly: property_signature.readonly,
        write_ty: None,
    })
}

pub(crate) fn parse_type_alias_declaration(
    declaration: &TSTypeAliasDeclaration<'_>,
) -> Option<ParsedTypeAliasDeclaration> {
    let ty = parse_type(&declaration.type_annotation)?;

    Some(ParsedTypeAliasDeclaration {
        is_declare: declaration.declare,
        name: declaration.id.name.to_string(),
        name_span: Some(text_span_from_oxc_span(declaration.id.span)),
        type_parameters: parse_type_parameters(declaration.type_parameters.as_deref()),
        ty,
        type_span: Some(text_span_from_oxc_span(declaration.type_annotation.span())),
        enum_name: None,
        enum_exported: false,
        enum_is_const: false,
        enum_members: Vec::new(),
    })
}
