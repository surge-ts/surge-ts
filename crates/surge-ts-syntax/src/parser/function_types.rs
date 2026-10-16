use crate::{ParsedFunctionType, ParsedFunctionTypeParameter, ParsedType};
use oxc_ast::ast::{
    BindingPattern, FormalParameter, FormalParameterRest, TSConstructorType, TSFunctionType,
    TSThisParameter,
};

use super::spans::text_span_from_oxc_span;

fn pattern_bound_names(pattern: &BindingPattern<'_>) -> Vec<crate::ParsedBoundName> {
    match pattern {
        BindingPattern::BindingIdentifier(_) => Vec::new(),
        pattern => super::functions::parse_binding_name(pattern).bound_names(),
    }
}

fn widened_literal_type(initializer: &oxc_ast::ast::Expression<'_>) -> Option<ParsedType> {
    use oxc_ast::ast::Expression;
    Some(match initializer {
        Expression::NumericLiteral(_) => ParsedType::Number,
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => ParsedType::String,
        Expression::BooleanLiteral(_) => ParsedType::Boolean,
        Expression::BigIntLiteral(_) => ParsedType::BigInt,
        _ => return None,
    })
}
use super::types::{parse_type_annotation, parse_type_parameters};

/// Lowers a constructor type (`new (args) => T`, `abstract new (args) => T`) to a
/// plain callable signature. surge does not model newability separately, and the
/// only thing downstream inference needs from a constructor type is its parameter
/// and return shapes — so reusing [`ParsedFunctionType`] keeps a union member like
/// React's `JSXElementConstructor<P>` (`((props: P) => …) | (new (props: P) => …)`)
/// fully parsed instead of collapsing the whole union to `Unknown`.
pub(crate) fn parse_constructor_type(
    constructor_type: &TSConstructorType<'_>,
) -> Option<ParsedFunctionType> {
    let mut parameters = Vec::new();

    for parameter in &constructor_type.params.items {
        parameters.push(parse_function_type_parameter(parameter)?);
    }

    if let Some(rest) = constructor_type.params.rest.as_deref() {
        parameters.push(parse_function_type_rest_parameter(rest)?);
    }

    let return_type = parse_type_annotation(&constructor_type.return_type)?;

    Some(ParsedFunctionType {
        parameters,
        return_type: Box::new(return_type),
        type_parameters: parse_type_parameters(constructor_type.type_parameters.as_deref()),
    })
}

pub(crate) fn parse_function_type(
    function_type: &TSFunctionType<'_>,
) -> Option<ParsedFunctionType> {
    let mut parameters = Vec::new();

    // A leading `this: T` is a fake parameter: keep it parser-safe and stored as
    // metadata so later phases can exclude it from arity and argument matching.
    // A `this` without a type annotation carries no metadata, so it is dropped
    // rather than failing the whole signature.
    if let Some(this_parameter) = function_type
        .this_param
        .as_deref()
        .and_then(parse_this_parameter)
    {
        parameters.push(this_parameter);
    }

    for parameter in &function_type.params.items {
        parameters.push(parse_function_type_parameter(parameter)?);
    }

    if let Some(rest) = function_type.params.rest.as_deref() {
        parameters.push(parse_function_type_rest_parameter(rest)?);
    }

    let return_type = parse_type_annotation(&function_type.return_type)?;

    Some(ParsedFunctionType {
        parameters,
        return_type: Box::new(return_type),
        type_parameters: parse_type_parameters(function_type.type_parameters.as_deref()),
    })
}

pub(crate) fn parse_this_parameter(
    this_parameter: &TSThisParameter<'_>,
) -> Option<ParsedFunctionTypeParameter> {
    let annotation = this_parameter.type_annotation.as_ref()?;
    let polymorphic_this = super::types::is_polymorphic_this_annotation(annotation);
    let ty = super::types::parse_this_parameter_annotation(annotation)?;

    Some(ParsedFunctionTypeParameter {
        name: Some("this".to_string()),
        name_span: Some(text_span_from_oxc_span(this_parameter.this_span)),
        ty,
        optional: false,
        question: false,
        is_this: true,
        polymorphic_this,
        rest: false,
        bound_names: Vec::new(),
    })
}

pub(crate) fn parse_function_type_rest_parameter(
    rest: &FormalParameterRest<'_>,
) -> Option<ParsedFunctionTypeParameter> {
    // Only the annotation carries meaning for a signature; the binding name is
    // cosmetic. Bailing on a non-identifier pattern dropped the whole parameter
    // and with it the method that declares it — `Iterator.next(...args: [] | [T])`
    // is written with a destructuring rest in lib.es2015.iterable.d.ts, so
    // `next` vanished from Iterator/Generator/AsyncIterator/AsyncGenerator.
    let (name, name_span) = match &rest.rest.argument {
        BindingPattern::BindingIdentifier(binding) => (
            Some(binding.name.to_string()),
            Some(text_span_from_oxc_span(binding.span)),
        ),
        _ => (None, None),
    };

    // An unannotated rest in a signature is `any[]` (TS7019 under
    // `noImplicitAny`); dropping it dropped the whole signature.
    let ty = match rest.type_annotation.as_ref() {
        Some(annotation) => parse_type_annotation(annotation)?,
        None => ParsedType::Array(std::sync::Arc::new(ParsedType::Any)),
    };

    Some(ParsedFunctionTypeParameter {
        name,
        name_span,
        ty,
        optional: false,
        question: false,
        is_this: false,
        polymorphic_this: false,
        rest: true,
        bound_names: pattern_bound_names(&rest.rest.argument),
    })
}

pub(crate) fn parse_function_type_parameter(
    parameter: &FormalParameter<'_>,
) -> Option<ParsedFunctionTypeParameter> {
    // A destructuring-pattern parameter (`render: ({ field }: Props) => …`,
    // react-hook-form's `ControllerProps.render`) has no single name — only its
    // annotation matters for the signature. Failing here would degrade the whole
    // enclosing function type (and any intersection it sits in) to `Unknown`.
    let (name, name_span) = match &parameter.pattern {
        BindingPattern::BindingIdentifier(binding) => (
            Some(binding.name.to_string()),
            Some(text_span_from_oxc_span(binding.span)),
        ),
        _ => (None, None),
    };

    // A type-level signature has no body, so an unannotated parameter is
    // `any` (TS7006 under `noImplicitAny`) unless an initializer — itself
    // TS2371 there — gives it a type. Dropping it dropped the signature, and
    // with it every construct/call/method signature written that way.
    let ty = match parameter.type_annotation.as_ref() {
        Some(annotation) => parse_type_annotation(annotation)?,
        None => parameter
            .initializer
            .as_ref()
            .and_then(|initializer| widened_literal_type(initializer))
            .unwrap_or(ParsedType::Any),
    };

    Some(ParsedFunctionTypeParameter {
        name,
        name_span,
        ty,
        optional: parameter.optional || parameter.initializer.is_some(),
        question: parameter.optional && parameter.initializer.is_none(),
        is_this: false,
        polymorphic_this: false,
        rest: false,
        bound_names: pattern_bound_names(&parameter.pattern),
    })
}
