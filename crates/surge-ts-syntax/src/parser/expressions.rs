use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionExpression, BinaryExpression,
    BinaryOperator,
    ChainElement, ChainExpression, ComputedMemberExpression, ConditionalExpression, Expression,
    JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement, JSXElementName,
    JSXExpressionContainer, JSXFragment, JSXMemberExpression, JSXMemberExpressionObject,
    LogicalExpression, LogicalOperator, NewExpression, ObjectExpression, ObjectPropertyKind,
    PrivateFieldExpression, PropertyKey, PropertyKind, SimpleAssignmentTarget,
    StaticMemberExpression, UnaryExpression, UnaryOperator, UpdateExpression,
};
use oxc_span::{GetSpan, Span};

use crate::{
    ParsedArrowFunction, ParsedArrowFunctionBody, ParsedBinaryOperator, ParsedCall,
    ParsedCallArgument, ParsedExpression, ParsedJsxAttribute, ParsedJsxAttributeValueKind,
    ParsedJsxChild, ParsedJsxClosingElement, ParsedJsxTag, ParsedLogicalOperator,
    ParsedObjectProperty, ParsedThisBinding,
    ParsedUnaryOperator, TextSpan,
};

use super::spans::text_span_from_oxc_span;
use super::types::parse_type_arguments;
use super::{
    functions::parse_function_parameter, functions::parse_rest_function_parameter,
    functions::parse_statement_list_as_function_body,
};

fn lower_type_assertion(
    expression: &Expression<'_>,
    type_annotation: &oxc_ast::ast::TSType<'_>,
    span: Span,
) -> ParsedExpression {
    let (expression, expression_span) = parse_expression(expression);
    let ty = crate::parser::types::parse_type(type_annotation).unwrap_or(crate::ParsedType::Unknown);
    if let crate::ParsedType::Named(named_type) = &ty
        && named_type.name == "const"
        && named_type.type_arguments.is_empty()
    {
        return ParsedExpression::ConstAssertion {
            expression: Box::new(expression),
            span: Some(text_span_from_oxc_span(span)),
        };
    }
    ParsedExpression::TypeAssertion {
        expression: Box::new(expression),
        expression_span: Some(text_span_from_oxc_span(expression_span)),
        ty,
        type_span: Some(text_span_from_oxc_span(type_annotation.span())),
        annotation: false,
    }
}

fn satisfies_keyword_span(expression: &oxc_ast::ast::TSSatisfiesExpression<'_>) -> Option<crate::TextSpan> {
    super::spans::keyword_span_between(
        expression.expression.span().end,
        expression.type_annotation.span().start,
        "satisfies",
    )
}

/// `/** @type {T} */ (e)` is `e as T`, and `/** @satisfies {T} */ (e)` is
/// `e satisfies T` (tsgo's `makeNewCast`).
fn lower_jsdoc_cast(
    expression: &Expression<'_>,
    (ty, type_span, is_assertion): (crate::ParsedType, crate::TextSpan, bool),
) -> ParsedExpression {
    let (expression, expression_span) = parse_expression(expression);
    if !is_assertion {
        return ParsedExpression::SatisfiesExpression {
            expression: Box::new(expression),
            span: Some(text_span_from_oxc_span(expression_span)),
            target_type: ty,
            target_span: Some(type_span),
            keyword_span: None,
        };
    }
    if let crate::ParsedType::Named(named_type) = &ty
        && named_type.name == "const"
        && named_type.type_arguments.is_empty()
    {
        return ParsedExpression::ConstAssertion {
            expression: Box::new(expression),
            span: Some(text_span_from_oxc_span(expression_span)),
        };
    }
    ParsedExpression::TypeAssertion {
        expression: Box::new(expression),
        expression_span: Some(text_span_from_oxc_span(expression_span)),
        ty,
        type_span: Some(type_span),
        annotation: false,
    }
}

pub(crate) fn parse_expression(expression: &Expression<'_>) -> (ParsedExpression, Span) {
    let (lowered, span) = parse_expression_unannotated(expression);
    // A JSDoc-typed assignment declaration's value is checked against its
    // type, which is what the declared member then has, and a mismatch is
    // reported where tsc's `checkTypeAssignableToAndOptionallyElaborate` puts
    // it: the assignment's target, the property, or an export's expression.
    if super::spans::lowering_javascript()
        && let Some((ty, _, anchor)) = super::jsdoc::assignment_type_at(expression.span().start, expression.span().end)
    {
        return (
            ParsedExpression::TypeAssertion {
                expression: Box::new(lowered),
                expression_span: Some(text_span_from_oxc_span(span)),
                ty,
                type_span: Some(anchor),
                annotation: true,
            },
            span,
        );
    }
    (lowered, span)
}

fn parse_expression_unannotated(expression: &Expression<'_>) -> (ParsedExpression, Span) {
    let parsed_expression = match expression {
        Expression::StringLiteral(string_literal) => {
            ParsedExpression::StringLiteral(string_literal.value.to_string())
        }
        Expression::NumericLiteral(numeric_literal) => {
            ParsedExpression::NumberLiteral(super::number_text::js_number_to_string(numeric_literal.value))
        }
        Expression::BooleanLiteral(boolean_literal) => {
            ParsedExpression::BooleanLiteral(boolean_literal.value)
        }
        Expression::NullLiteral(_) => ParsedExpression::NullLiteral,
        Expression::RegExpLiteral(_) => ParsedExpression::RegExpLiteral,
        Expression::BigIntLiteral(literal) => ParsedExpression::BigIntLiteral(literal.raw.as_deref().unwrap_or_default().to_string()),
        Expression::Identifier(identifier) => {
            if identifier.name == "undefined" {
                ParsedExpression::UndefinedLiteral
            } else {
                ParsedExpression::Identifier {
                    name: identifier.name.to_string(),
                    span: Some(text_span_from_oxc_span(identifier.span)),
                }
            }
        }
        Expression::ObjectExpression(object_expression) => ParsedExpression::ObjectLiteral {
            properties: parse_object_properties(object_expression),
            span: Some(text_span_from_oxc_span(object_expression.span())),
        },
        Expression::ArrayExpression(array_expression) => {
            parse_array_expression(array_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::BinaryExpression(binary_expression) => {
            parse_binary_expression(binary_expression).unwrap_or(ParsedExpression::Unknown)
        }
        // `#brand in o` checks its object as `in` does and is a boolean. The
        // brand is no property key, so it neither narrows nor is checked as one.
        Expression::PrivateInExpression(private_in) => {
            let (right, right_span) = parse_expression(&private_in.right);
            // oxc recovers a stray `#name` as an `in` against an empty operand.
            if right_span.start >= right_span.end || right_span.start < private_in.left.span.end {
                return (ParsedExpression::Unknown, expression.span());
            }
            ParsedExpression::Binary {
                left: Box::new(ParsedExpression::Unknown),
                left_span: Some(text_span_from_oxc_span(private_in.left.span)),
                operator: ParsedBinaryOperator::In,
                right: Box::new(right),
                right_span: Some(text_span_from_oxc_span(right_span)),
                operator_span: None,
            }
        }
        Expression::LogicalExpression(logical_expression) => {
            parse_logical_expression(logical_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::UnaryExpression(unary_expression) => {
            parse_unary_expression(unary_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::UpdateExpression(update_expression) => {
            parse_update_expression(update_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::FunctionExpression(function) => ParsedExpression::ArrowFunction(Box::new(
            parse_function_expression(function),
        )),
        Expression::ParenthesizedExpression(parenthesized_expression) => {
            if let Some(cast) = super::jsdoc::cast_at(parenthesized_expression.span.start) {
                return (
                    lower_jsdoc_cast(&parenthesized_expression.expression, cast),
                    parenthesized_expression.span,
                );
            }
            return parse_expression(&parenthesized_expression.expression);
        }
        Expression::YieldExpression(yield_expression) => {
            let (operand, operand_span) = match &yield_expression.argument {
                Some(argument) => {
                    let (operand, operand_span) = parse_expression(argument);
                    (Some(Box::new(operand)), Some(text_span_from_oxc_span(operand_span)))
                }
                None => (None, None),
            };
            return (
                ParsedExpression::Yield {
                    operand,
                    operand_span,
                    delegate: yield_expression.delegate,
                    span: Some(text_span_from_oxc_span(yield_expression.span)),
                },
                yield_expression.span,
            );
        }
        Expression::AwaitExpression(await_expression) => {
            let (operand, operand_span) = parse_expression(&await_expression.argument);
            return (
                ParsedExpression::Await {
                    operand: Box::new(operand),
                    operand_span: Some(text_span_from_oxc_span(operand_span)),
                },
                operand_span,
            );
        }
        Expression::ConditionalExpression(conditional_expression) => {
            parse_conditional_expression(conditional_expression)
                .unwrap_or(ParsedExpression::Unknown)
        }
        Expression::SequenceExpression(sequence) => ParsedExpression::Sequence {
            expressions: sequence
                .expressions
                .iter()
                .map(|expression| {
                    let (expression, span) = parse_expression(expression);
                    (expression, Some(text_span_from_oxc_span(span)))
                })
                .collect(),
        },
        Expression::CallExpression(call_expression) => {
            parse_call_expression_expression(call_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::NewExpression(new_expression) => {
            parse_new_expression(new_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::TSInstantiationExpression(instantiation_expression) => {
            parse_instantiation_expression(instantiation_expression)
                .unwrap_or(ParsedExpression::Unknown)
        }
        Expression::StaticMemberExpression(member_expression) => {
            parse_static_member_expression(member_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::ComputedMemberExpression(member_expression) => {
            parse_computed_member_expression(member_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::PrivateFieldExpression(member_expression) => {
            parse_private_field_expression(member_expression)
        }
        Expression::TSAsExpression(as_expression) => lower_type_assertion(
            &as_expression.expression,
            &as_expression.type_annotation,
            as_expression.span,
        ),
        Expression::TSTypeAssertion(type_assertion) => lower_type_assertion(
            &type_assertion.expression,
            &type_assertion.type_annotation,
            type_assertion.span,
        ),
        Expression::TSSatisfiesExpression(satisfies_expression) => {
            let (expression, expression_span) = parse_expression(&satisfies_expression.expression);
            let ty = crate::parser::types::parse_type(&satisfies_expression.type_annotation)
                .unwrap_or(crate::ParsedType::Unknown);
            ParsedExpression::SatisfiesExpression {
                expression: Box::new(expression),
                span: Some(text_span_from_oxc_span(expression_span)),
                target_type: ty,
                target_span: Some(text_span_from_oxc_span(
                    satisfies_expression.type_annotation.span(),
                )),
                keyword_span: satisfies_keyword_span(satisfies_expression),
            }
        }
        Expression::TSNonNullExpression(non_null_expression) => {
            let (expression, _expression_span) = parse_expression(&non_null_expression.expression);
            ParsedExpression::NonNullAssertion {
                expression: Box::new(expression),
                span: Some(text_span_from_oxc_span(non_null_expression.span)),
                in_optional_chain: false,
            }
        }
        Expression::ChainExpression(chain_expression) => {
            parse_chain_expression(chain_expression).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::ArrowFunctionExpression(arrow_expression) => {
            parse_arrow_function_expression(arrow_expression)
                .map(|arrow| ParsedExpression::ArrowFunction(Box::new(arrow)))
                .unwrap_or(ParsedExpression::Unknown)
        }
        Expression::JSXElement(jsx_element) => parse_jsx_element(jsx_element),
        Expression::JSXFragment(jsx_fragment) => parse_jsx_fragment(jsx_fragment),
        Expression::ThisExpression(this_expression) => ParsedExpression::This {
            span: Some(text_span_from_oxc_span(this_expression.span)),
        },
        // `super.member`: the checker binds `super` to the base class in a class
        // member's body. A `super(...)` call has the same callee, which the call
        // check resolves to the base constructor instead.
        Expression::Super(super_keyword) => ParsedExpression::Identifier {
            name: "super".to_string(),
            span: Some(text_span_from_oxc_span(super_keyword.span)),
        },
        Expression::TemplateLiteral(template) => ParsedExpression::TemplateLiteral {
            expressions: template
                .expressions
                .iter()
                .map(|expression| parse_expression(expression).0)
                .collect(),
            span: Some(text_span_from_oxc_span(template.span)),
            quasis: template
                .quasis
                .iter()
                .map(|quasi| quasi.value.cooked.as_ref().map(|cooked| cooked.to_string()))
                .collect(),
            expression_spans: template
                .expressions
                .iter()
                .map(|expression| Some(text_span_from_oxc_span(expression.span())))
                .collect(),
        },
        Expression::TaggedTemplateExpression(tagged) => parse_tagged_template(tagged),
        Expression::AssignmentExpression(assignment) => {
            parse_assignment_value(assignment).unwrap_or(ParsedExpression::Unknown)
        }
        Expression::ImportExpression(import) => parse_import_call(import),
        Expression::ClassExpression(class) => parse_class_expression(class),
        // `import.meta` reads a global the checker binds to `ImportMeta`
        // (`checkImportMetaProperty`), so a property of it narrows.
        Expression::MetaProperty(meta) if meta.meta.name == "import" && meta.property.name == "meta" => {
            ParsedExpression::Identifier {
                name: "import.meta".to_string(),
                span: Some(text_span_from_oxc_span(meta.span)),
            }
        }
        _ => ParsedExpression::Unknown,
    };

    (parsed_expression, expression.span())
}

/// A class expression, under its own name or else `(Anonymous class)`, as tsc
/// writes an unassigned one (`getNameOfSymbolAsWritten`). The name a variable
/// or property lends an anonymous class (`getAssignedName`) only labels it in
/// tsc, and is not taken: the name here is also what the class's instance
/// type is looked up by while it is checked. An unnamed class is reported at
/// its first token.
fn parse_class_expression(class: &oxc_ast::ast::Class<'_>) -> ParsedExpression {
    let (name, name_span) = match &class.id {
        Some(id) => (id.name.to_string(), id.span),
        None => (
            "(Anonymous class)".to_string(),
            Span::new(class.span.start, class.span.start + "class".len() as u32),
        ),
    };
    match super::classes::parse_class_declaration_named(class, name, text_span_from_oxc_span(name_span)) {
        Some(class_declaration) => ParsedExpression::ClassExpression(Box::new(crate::ParsedClassExpression {
            class: class_declaration,
            has_own_name: class.id.is_some(),
        })),
        None => ParsedExpression::Unknown,
    }
}

/// `import(specifier, options)`. The parser recovers `import()` and a spread
/// argument with an empty-span placeholder: tsc checks no argument of the
/// former, and a spread specifier is not checked as one.
fn parse_import_call(import: &oxc_ast::ast::ImportExpression<'_>) -> ParsedExpression {
    fn is_placeholder(expression: &Expression<'_>) -> bool {
        matches!(expression, Expression::NullLiteral(literal) if literal.span.is_empty())
    }
    if is_placeholder(&import.source) {
        return ParsedExpression::Unknown;
    }
    let (specifier, specifier_span) = parse_expression(&import.source);
    let (options, options_span) =
        match import.options.as_ref().filter(|options| !is_placeholder(options)) {
            Some(options) => {
                let (options, span) = parse_expression(options);
                (Some(Box::new(options)), Some(text_span_from_oxc_span(span)))
            }
            None => (None, None),
        };
    ParsedExpression::ImportCall {
        specifier: Box::new(specifier),
        specifier_span: Some(text_span_from_oxc_span(specifier_span)),
        options,
        options_span,
        span: Some(text_span_from_oxc_span(import.span)),
    }
}

/// `` tag`a${x}b` `` is a call of `tag` (tsc's `resolveTaggedTemplateExpression`):
/// the effective arguments are the template's strings array, then each
/// substitution, and the tag's type arguments are the call's.
fn parse_tagged_template(tagged: &oxc_ast::ast::TaggedTemplateExpression<'_>) -> ParsedExpression {
    let Some(type_arguments) = (match tagged.type_arguments.as_deref() {
        Some(type_arguments) => parse_type_arguments(type_arguments),
        None => Some(Vec::new()),
    }) else {
        return ParsedExpression::Unknown;
    };
    let template_span = Some(text_span_from_oxc_span(tagged.quasi.span));
    let mut arguments = vec![ParsedCallArgument {
        expression: ParsedExpression::TemplateStringsArray { span: template_span },
        span: template_span,
        spread: false,
        expression_span: template_span,
    }];
    arguments.extend(tagged.quasi.expressions.iter().map(|expression| {
        let span = Some(text_span_from_oxc_span(expression.span()));
        ParsedCallArgument {
            expression: parse_expression(expression).0,
            span,
            spread: false,
            expression_span: span,
        }
    }));
    let call_span = Some(text_span_from_oxc_span(tagged.span));
    match &tagged.tag {
        Expression::Identifier(callee) if callee.name != "undefined" => ParsedExpression::Call {
            callee_name: callee.name.to_string(),
            callee_span: Some(text_span_from_oxc_span(callee.span)),
            type_arguments,
            arguments,
        },
        Expression::StaticMemberExpression(member) if !member.optional => {
            let (object, object_span) = parse_expression(&member.object);
            ParsedExpression::PropertyCall {
                object: Box::new(object),
                object_span: Some(text_span_from_oxc_span(object_span)),
                property_name: member.property.name.to_string(),
                property_span: Some(text_span_from_oxc_span(member.property.span)),
                call_span,
                type_arguments,
                arguments,
            }
        }
        tag => {
            let (callee, callee_span) = parse_expression(tag);
            ParsedExpression::ExpressionCall {
                callee: Box::new(callee),
                callee_span: Some(text_span_from_oxc_span(callee_span)),
                type_arguments,
                arguments,
            }
        }
    }
}

fn parse_jsx_element(element: &JSXElement<'_>) -> ParsedExpression {
    let opening = &element.opening_element;
    let (tag_name, tag_name_span, component_name, component_span) =
        parse_jsx_element_name(&opening.name);
    let attributes = opening
        .attributes
        .iter()
        .filter_map(parse_jsx_attribute_item)
        .collect();
    let (children, child_spans) = parse_jsx_children(&element.children);
    let (type_arguments, type_arguments_span) = match opening.type_arguments.as_deref() {
        Some(type_arguments) => (
            parse_type_arguments(type_arguments).unwrap_or_default(),
            Some(text_span_from_oxc_span(type_arguments.span)),
        ),
        None => (Vec::new(), None),
    };
    let closing = element
        .closing_element
        .as_deref()
        .map(|closing| ParsedJsxClosingElement {
            span: Some(text_span_from_oxc_span(closing.span)),
            expression: jsx_tag_expression(&closing.name),
        });

    ParsedExpression::JsxElement {
        tag_name,
        component_name,
        component_span,
        attributes,
        children,
        tag: Box::new(ParsedJsxTag {
            name_span: tag_name_span,
            span: Some(text_span_from_oxc_span(element.span)),
            expression: jsx_tag_expression(&opening.name),
            type_arguments,
            type_arguments_span,
            child_spans,
            closing,
        }),
    }
}

/// tsc's `isJsxIntrinsicTagName` decides what a tag is: an identifier with an
/// intrinsic name or a namespaced name names an intrinsic element, anything
/// else (`Button`, `UI.Button`, `this`, `this.tag`) is read as a value. oxc
/// draws the same line between `Identifier` and `IdentifierReference`.
fn jsx_tag_expression(name: &JSXElementName<'_>) -> Option<ParsedExpression> {
    match name {
        JSXElementName::Identifier(_) | JSXElementName::NamespacedName(_) => None,
        JSXElementName::IdentifierReference(identifier) => Some(ParsedExpression::Identifier {
            name: identifier.name.to_string(),
            span: Some(text_span_from_oxc_span(identifier.span)),
        }),
        JSXElementName::ThisExpression(this) => Some(ParsedExpression::This {
            span: Some(text_span_from_oxc_span(this.span)),
        }),
        JSXElementName::MemberExpression(member) => Some(jsx_member_tag_expression(member)),
    }
}

fn jsx_member_tag_expression(member: &JSXMemberExpression<'_>) -> ParsedExpression {
    let (object, object_span) = match &member.object {
        JSXMemberExpressionObject::IdentifierReference(identifier) => (
            ParsedExpression::Identifier {
                name: identifier.name.to_string(),
                span: Some(text_span_from_oxc_span(identifier.span)),
            },
            identifier.span,
        ),
        JSXMemberExpressionObject::MemberExpression(inner) => {
            (jsx_member_tag_expression(inner), inner.span)
        }
        JSXMemberExpressionObject::ThisExpression(this) => (
            ParsedExpression::This {
                span: Some(text_span_from_oxc_span(this.span)),
            },
            this.span,
        ),
    };
    ParsedExpression::PropertyAccess {
        object: Box::new(object),
        object_span: Some(text_span_from_oxc_span(object_span)),
        property_name: member.property.name.to_string(),
        property_span: Some(text_span_from_oxc_span(member.property.span)),
        is_bracketed: false,
        binding_element: false,
    }
}

fn parse_jsx_fragment(fragment: &JSXFragment<'_>) -> ParsedExpression {
    let (children, _) = parse_jsx_children(&fragment.children);

    ParsedExpression::JsxFragment {
        children,
        span: Some(text_span_from_oxc_span(fragment.span)),
    }
}

/// Returns `(tag_name, tag_name_span, component_name, component_span)`. The
/// component name/span are populated only when the tag is a value reference (a
/// capitalized component, a `Foo.Bar` member tag, or `this` and `this.x`, which
/// tsc's `isJsxIntrinsicTagName` never takes for intrinsic) so the checker can
/// resolve it and report TS2304 for missing names; intrinsic lowercase tags carry
/// `None`.
fn parse_jsx_element_name(
    name: &JSXElementName<'_>,
) -> (String, Option<TextSpan>, Option<String>, Option<TextSpan>) {
    match name {
        JSXElementName::Identifier(identifier) => {
            // Intrinsic element such as `<div />`; not a value reference.
            let span = Some(text_span_from_oxc_span(identifier.span));
            (identifier.name.to_string(), span, None, None)
        }
        JSXElementName::IdentifierReference(identifier) => {
            // Component reference such as `<Button />`.
            let span = Some(text_span_from_oxc_span(identifier.span));
            (
                identifier.name.to_string(),
                span,
                Some(identifier.name.to_string()),
                span,
            )
        }
        JSXElementName::MemberExpression(member) => {
            let span = Some(text_span_from_oxc_span(member.span));
            let (head_name, head_span) = jsx_member_expression_head(member);
            (
                jsx_member_expression_name(member),
                span,
                head_name,
                head_span,
            )
        }
        JSXElementName::NamespacedName(namespaced) => {
            let span = Some(text_span_from_oxc_span(namespaced.span));
            (
                format!("{}:{}", namespaced.namespace.name, namespaced.name.name),
                span,
                None,
                None,
            )
        }
        JSXElementName::ThisExpression(this) => {
            let span = Some(text_span_from_oxc_span(this.span));
            ("this".to_string(), span, Some("this".to_string()), span)
        }
    }
}

/// Builds the dotted display name for a member tag (`UI.Button`, `A.B.C`).
fn jsx_member_expression_name(member: &JSXMemberExpression<'_>) -> String {
    let object = match &member.object {
        JSXMemberExpressionObject::IdentifierReference(identifier) => identifier.name.to_string(),
        JSXMemberExpressionObject::MemberExpression(inner) => jsx_member_expression_name(inner),
        JSXMemberExpressionObject::ThisExpression(_) => "this".to_string(),
    };
    format!("{}.{}", object, member.property.name)
}

/// Returns the head of a member tag (the value that must resolve in scope):
/// `<UI.Button />` resolves `UI`, `<this.Foo />` reads `this`.
fn jsx_member_expression_head(
    member: &JSXMemberExpression<'_>,
) -> (Option<String>, Option<TextSpan>) {
    let mut object = &member.object;
    loop {
        match object {
            JSXMemberExpressionObject::IdentifierReference(identifier) => {
                return (
                    Some(identifier.name.to_string()),
                    Some(text_span_from_oxc_span(identifier.span)),
                );
            }
            JSXMemberExpressionObject::MemberExpression(inner) => {
                object = &inner.object;
            }
            JSXMemberExpressionObject::ThisExpression(this) => {
                return (Some("this".to_string()), Some(text_span_from_oxc_span(this.span)));
            }
        }
    }
}

fn parse_jsx_attribute_item(item: &JSXAttributeItem<'_>) -> Option<ParsedJsxAttribute> {
    match item {
        JSXAttributeItem::Attribute(attribute) => {
            let (name, name_span) = parse_jsx_attribute_name(&attribute.name);
            let (value, value_span, value_kind) = match &attribute.value {
                Some(JSXAttributeValue::ExpressionContainer(container)) => {
                    let (value, value_span) = parse_jsx_container_expression(container);
                    (value, value_span, ParsedJsxAttributeValueKind::Expression)
                }
                Some(JSXAttributeValue::Element(element)) => {
                    let span = Some(text_span_from_oxc_span(element.span));
                    (
                        Some(parse_jsx_element(element)),
                        span,
                        ParsedJsxAttributeValueKind::Expression,
                    )
                }
                Some(JSXAttributeValue::Fragment(fragment)) => {
                    let span = Some(text_span_from_oxc_span(fragment.span));
                    (
                        Some(parse_jsx_fragment(fragment)),
                        span,
                        ParsedJsxAttributeValueKind::Expression,
                    )
                }
                // A string-literal value (`id="x"`) keeps its text (typed as the
                // literal) and its span so a prop diagnostic points at the value.
                Some(JSXAttributeValue::StringLiteral(literal)) => (
                    None,
                    Some(text_span_from_oxc_span(literal.span)),
                    ParsedJsxAttributeValueKind::StringLiteral(literal.value.to_string()),
                ),
                // Boolean shorthand (`disabled`) is equivalent to `disabled={true}`.
                None => (None, None, ParsedJsxAttributeValueKind::BooleanShorthand),
            };
            Some(ParsedJsxAttribute {
                name,
                name_span,
                value,
                value_span,
                value_kind,
            })
        }
        JSXAttributeItem::SpreadAttribute(spread) => {
            // Spread checking is out of scope, but the argument is still walked so
            // ordinary diagnostics inside it (e.g. unresolved names) are preserved.
            let (expression, span) = parse_expression(&spread.argument);
            Some(ParsedJsxAttribute {
                name: String::new(),
                name_span: None,
                value: Some(expression),
                value_span: Some(text_span_from_oxc_span(span)),
                value_kind: ParsedJsxAttributeValueKind::Expression,
            })
        }
    }
}

fn parse_jsx_attribute_name(name: &JSXAttributeName<'_>) -> (String, Option<TextSpan>) {
    match name {
        JSXAttributeName::Identifier(identifier) => (
            identifier.name.to_string(),
            Some(text_span_from_oxc_span(identifier.span)),
        ),
        JSXAttributeName::NamespacedName(namespaced) => (
            format!("{}:{}", namespaced.namespace.name, namespaced.name.name),
            Some(text_span_from_oxc_span(namespaced.span)),
        ),
    }
}

fn parse_jsx_container_expression(
    container: &JSXExpressionContainer<'_>,
) -> (Option<ParsedExpression>, Option<TextSpan>) {
    match container.expression.as_expression() {
        Some(expression) => {
            let (parsed, span) = parse_expression(expression);
            (Some(parsed), Some(text_span_from_oxc_span(span)))
        }
        // Empty container `{}`: nothing to check.
        None => (None, Some(text_span_from_oxc_span(container.span))),
    }
}

fn parse_jsx_children(children: &[JSXChild<'_>]) -> (Vec<ParsedJsxChild>, Vec<Option<TextSpan>>) {
    children
        .iter()
        .filter(|child| !is_trivia_jsx_text(child))
        .map(|child| (parse_jsx_child(child), Some(text_span_from_oxc_span(child.span()))))
        .unzip()
}

/// tsc's `JsxText.ContainsOnlyTriviaWhiteSpaces`: whitespace that spans a line
/// break is layout, not a child, so the checker never sees it.
fn is_trivia_jsx_text(child: &JSXChild<'_>) -> bool {
    let JSXChild::Text(text) = child else {
        return false;
    };
    let raw = text.raw.as_ref().map_or(text.value.as_str(), |raw| raw.as_str());
    let is_line_break = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let is_single_line_white_space = |c: char| {
        matches!(
            c,
            ' ' | '\t'
                | '\u{b}'
                | '\u{c}'
                | '\u{a0}'
                | '\u{85}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200b}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
        )
    };
    raw.chars().any(is_line_break)
        && raw
            .chars()
            .all(|c| is_line_break(c) || is_single_line_white_space(c))
}

fn parse_jsx_child(child: &JSXChild<'_>) -> ParsedJsxChild {
    match child {
        JSXChild::Text(_) => ParsedJsxChild::Text,
        JSXChild::Element(element) => ParsedJsxChild::Element(parse_jsx_element(element)),
        JSXChild::Fragment(fragment) => ParsedJsxChild::Element(parse_jsx_fragment(fragment)),
        JSXChild::ExpressionContainer(container) => {
            let (expression, span) = parse_jsx_container_expression(container);
            ParsedJsxChild::Expression {
                expression,
                span,
                spread: false,
            }
        }
        JSXChild::Spread(spread) => {
            let (expression, span) = parse_expression(&spread.expression);
            ParsedJsxChild::Expression {
                expression: Some(expression),
                span: Some(text_span_from_oxc_span(span)),
                spread: true,
            }
        }
    }
}

pub(crate) fn parse_call_expression(
    call_expression: &oxc_ast::ast::CallExpression<'_>,
) -> Option<ParsedCall> {
    let call = parse_call_expression_parts(call_expression)?;

    Some(ParsedCall {
        callee_name: call.callee_name,
        callee_span: call.callee_span,
        span: call.span,
        type_arguments: call.type_arguments,
        arguments: call.arguments,
    })
}

fn parse_call_expression_expression(
    call_expression: &oxc_ast::ast::CallExpression<'_>,
) -> Option<ParsedExpression> {
    if let Some(require) = super::commonjs::require_call_expression(call_expression) {
        return Some(require);
    }
    let arguments = call_expression
        .arguments
        .iter()
        .map(parse_call_argument)
        .collect::<Vec<_>>();
    let type_arguments = parse_call_type_arguments(call_expression)?;

    if call_expression.optional {
        match &call_expression.callee {
            Expression::StaticMemberExpression(member_expression) => {
                let (object, object_span) = parse_expression(&member_expression.object);
                return Some(ParsedExpression::OptionalPropertyCall {
                    object: Box::new(object),
                    object_span: Some(text_span_from_oxc_span(object_span)),
                    property_name: member_expression.property.name.to_string(),
                    property_span: Some(text_span_from_oxc_span(member_expression.property.span)),
                    call_span: Some(text_span_from_oxc_span(call_expression.span)),
                    type_arguments,
                    arguments,
                });
            }
            _ => {
                let (callee, callee_span) = parse_expression(&call_expression.callee);
                return Some(ParsedExpression::OptionalCall {
                    callee: Box::new(callee),
                    callee_span: Some(text_span_from_oxc_span(callee_span)),
                    type_arguments,
                    arguments,
                });
            }
        }
    }

    match &call_expression.callee {
        // `undefined()` invokes the value, not a binding named `undefined`.
        Expression::Identifier(callee) if callee.name != "undefined" => Some(ParsedExpression::Call {
            callee_name: callee.name.to_string(),
            callee_span: Some(text_span_from_oxc_span(callee.span)),
            type_arguments,
            arguments,
        }),
        Expression::StaticMemberExpression(member_expression) => {
            let (object, object_span) = parse_expression(&member_expression.object);

            if member_expression.optional {
                Some(ParsedExpression::OptionalPropertyCall {
                    object: Box::new(object),
                    object_span: Some(text_span_from_oxc_span(object_span)),
                    property_name: member_expression.property.name.to_string(),
                    property_span: Some(text_span_from_oxc_span(member_expression.property.span)),
                    call_span: Some(text_span_from_oxc_span(call_expression.span)),
                    type_arguments,
                    arguments,
                })
            } else {
                Some(ParsedExpression::PropertyCall {
                    object: Box::new(object),
                    object_span: Some(text_span_from_oxc_span(object_span)),
                    property_name: member_expression.property.name.to_string(),
                    property_span: Some(text_span_from_oxc_span(member_expression.property.span)),
                    call_span: Some(text_span_from_oxc_span(call_expression.span)),
                    type_arguments,
                    arguments,
                })
            }
        }
        Expression::PrivateFieldExpression(member_expression) => {
            let (object, object_span) = parse_expression(&member_expression.object);
            let object = Box::new(object);
            let object_span = Some(text_span_from_oxc_span(object_span));
            let property_name =
                super::private_names::access_key(member_expression.field.name.as_str());
            let property_span = Some(text_span_from_oxc_span(member_expression.field.span));
            let call_span = Some(text_span_from_oxc_span(call_expression.span));
            if member_expression.optional {
                Some(ParsedExpression::OptionalPropertyCall {
                    object,
                    object_span,
                    property_name,
                    property_span,
                    call_span,
                    type_arguments,
                    arguments,
                })
            } else {
                Some(ParsedExpression::PropertyCall {
                    object,
                    object_span,
                    property_name,
                    property_span,
                    call_span,
                    type_arguments,
                    arguments,
                })
            }
        }
        Expression::Super(super_keyword) => Some(ParsedExpression::ExpressionCall {
            callee: Box::new(ParsedExpression::Identifier {
                name: "super".to_string(),
                span: Some(text_span_from_oxc_span(super_keyword.span)),
            }),
            callee_span: Some(text_span_from_oxc_span(super_keyword.span)),
            type_arguments,
            arguments,
        }),
        _ => {
            let (callee, callee_span) = parse_expression(&call_expression.callee);
            Some(ParsedExpression::ExpressionCall {
                callee: Box::new(callee),
                callee_span: Some(text_span_from_oxc_span(callee_span)),
                type_arguments,
                arguments,
            })
        }
    }
}

struct ParsedCallExpressionParts {
    callee_name: String,
    callee_span: Option<TextSpan>,
    span: Option<TextSpan>,
    type_arguments: Vec<crate::ParsedType>,
    arguments: Vec<ParsedCallArgument>,
}

fn parse_call_expression_parts(
    call_expression: &oxc_ast::ast::CallExpression<'_>,
) -> Option<ParsedCallExpressionParts> {
    let Expression::Identifier(callee) = &call_expression.callee else {
        return None;
    };
    if callee.name == "undefined" {
        return None;
    }

    let arguments = call_expression
        .arguments
        .iter()
        .map(parse_call_argument)
        .collect();
    let type_arguments = parse_call_type_arguments(call_expression)?;

    Some(ParsedCallExpressionParts {
        callee_name: callee.name.to_string(),
        callee_span: Some(text_span_from_oxc_span(callee.span)),
        span: Some(text_span_from_oxc_span(call_expression.span)),
        type_arguments,
        arguments,
    })
}

fn parse_call_type_arguments(
    call_expression: &oxc_ast::ast::CallExpression<'_>,
) -> Option<Vec<crate::ParsedType>> {
    match call_expression.type_arguments.as_deref() {
        Some(type_arguments) => parse_type_arguments(type_arguments),
        None => Some(Vec::new()),
    }
}

fn parse_new_expression(new_expression: &NewExpression<'_>) -> Option<ParsedExpression> {
    let arguments = new_expression
        .arguments
        .iter()
        .map(parse_call_argument)
        .collect::<Vec<_>>();
    let type_arguments = match new_expression.type_arguments.as_deref() {
        Some(type_arguments) => parse_type_arguments(type_arguments)?,
        None => Vec::new(),
    };
    let (callee, callee_span) = parse_expression(&new_expression.callee);

    Some(ParsedExpression::New {
        callee: Box::new(callee),
        callee_span: Some(text_span_from_oxc_span(callee_span)),
        span: Some(text_span_from_oxc_span(new_expression.span)),
        type_arguments,
        arguments,
    })
}

fn parse_instantiation_expression(
    instantiation_expression: &oxc_ast::ast::TSInstantiationExpression<'_>,
) -> Option<ParsedExpression> {
    let type_arguments = parse_type_arguments(&instantiation_expression.type_arguments)?;
    // `f<T>`, `ns.f<T>` or `h()<T>` with no argument list is an instantiation
    // expression, not a call: lowering it to a zero-argument call reported
    // `TS2554` against a signature nothing here invokes.
    let (expression, expression_span) = parse_expression(&instantiation_expression.expression);
    Some(ParsedExpression::Instantiation {
        expression: Box::new(expression),
        expression_span: Some(text_span_from_oxc_span(expression_span)),
        type_arguments,
        type_arguments_span: Some(text_span_from_oxc_span(instantiation_expression.type_arguments.span)),
    })
}

fn parse_call_argument(argument: &Argument<'_>) -> ParsedCallArgument {
    let (expression, span) = match argument {
        // The spread's own expression is still code — an unresolved name or a
        // bad member inside it is reported the same way — so it is parsed, and
        // the argument carries the flag the arity check needs.
        Argument::SpreadElement(spread) => {
            let (expression, expression_span) = parse_expression(&spread.argument);
            return ParsedCallArgument {
                expression,
                span: Some(text_span_from_oxc_span(argument.span())),
                spread: true,
                expression_span: Some(text_span_from_oxc_span(expression_span)),
            };
        }
        Argument::BooleanLiteral(boolean_literal) => (
            ParsedExpression::BooleanLiteral(boolean_literal.value),
            argument.span(),
        ),
        Argument::NullLiteral(_) => (ParsedExpression::NullLiteral, argument.span()),
        Argument::NumericLiteral(numeric_literal) => (
            ParsedExpression::NumberLiteral(numeric_literal.value.to_string()),
            argument.span(),
        ),
        Argument::StringLiteral(string_literal) => (
            ParsedExpression::StringLiteral(string_literal.value.to_string()),
            argument.span(),
        ),
        Argument::Identifier(identifier) => (
            if identifier.name == "undefined" {
                ParsedExpression::UndefinedLiteral
            } else {
                ParsedExpression::Identifier {
                    name: identifier.name.to_string(),
                    span: Some(text_span_from_oxc_span(identifier.span)),
                }
            },
            argument.span(),
        ),
        Argument::BinaryExpression(binary_expression) => (
            parse_binary_expression(binary_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::LogicalExpression(logical_expression) => (
            parse_logical_expression(logical_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::UnaryExpression(unary_expression) => (
            parse_unary_expression(unary_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        // tsc reports an argument on its effective check node, which skips the
        // parentheses around it.
        Argument::ParenthesizedExpression(parenthesized_expression) => {
            parse_expression(&parenthesized_expression.expression)
        }
        Argument::SequenceExpression(sequence) => (
            ParsedExpression::Sequence {
                expressions: sequence
                    .expressions
                    .iter()
                    .map(|expression| {
                        let (expression, span) = parse_expression(expression);
                        (expression, Some(text_span_from_oxc_span(span)))
                    })
                    .collect(),
            },
            argument.span(),
        ),
        Argument::AwaitExpression(await_expression) => {
            let (operand, operand_span) = parse_expression(&await_expression.argument);
            (
                ParsedExpression::Await {
                    operand: Box::new(operand),
                    operand_span: Some(text_span_from_oxc_span(operand_span)),
                },
                argument.span(),
            )
        }
        Argument::ConditionalExpression(conditional_expression) => (
            parse_conditional_expression(conditional_expression)
                .unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::ObjectExpression(object_expression) => (
            ParsedExpression::ObjectLiteral {
                properties: parse_object_properties(object_expression),
                span: Some(text_span_from_oxc_span(object_expression.span())),
            },
            argument.span(),
        ),
        Argument::ArrayExpression(array_expression) => (
            parse_array_expression(array_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::CallExpression(call_expression) => (
            parse_call_expression_expression(call_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::ArrowFunctionExpression(arrow_expression) => (
            parse_arrow_function_expression(arrow_expression)
                .map(|arrow| ParsedExpression::ArrowFunction(Box::new(arrow)))
                .unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::ChainExpression(chain_expression) => (
            parse_chain_expression(chain_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::StaticMemberExpression(member_expression) => (
            parse_static_member_expression(member_expression).unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::ComputedMemberExpression(member_expression) => (
            parse_computed_member_expression(member_expression)
                .unwrap_or(ParsedExpression::Unknown),
            argument.span(),
        ),
        Argument::TSAsExpression(as_expression) => {
            let (expression, expression_span) = parse_expression(&as_expression.expression);
            let ty = crate::parser::types::parse_type(&as_expression.type_annotation)
                .unwrap_or(crate::ParsedType::Unknown);

            if let crate::ParsedType::Named(named_type) = &ty {
                if named_type.name == "const" && named_type.type_arguments.is_empty() {
                    return ParsedCallArgument {
                        expression: ParsedExpression::ConstAssertion {
                            expression: Box::new(expression),
                            span: Some(text_span_from_oxc_span(as_expression.span)),
                        },
                        span: Some(text_span_from_oxc_span(as_expression.span)),
                        spread: false,
                        expression_span: Some(text_span_from_oxc_span(as_expression.span)),
                    };
                }
            }

            (
                ParsedExpression::TypeAssertion {
                    expression: Box::new(expression),
                    expression_span: Some(text_span_from_oxc_span(expression_span)),
                    ty,
                    type_span: Some(text_span_from_oxc_span(
                        as_expression.type_annotation.span(),
                    )),
                    annotation: false,
                },
                as_expression.span,
            )
        }
        Argument::TSTypeAssertion(type_assertion) => (
            lower_type_assertion(
                &type_assertion.expression,
                &type_assertion.type_annotation,
                type_assertion.span,
            ),
            argument.span(),
        ),
        Argument::TSSatisfiesExpression(satisfies_expression) => {
            let (expression, expression_span) = parse_expression(&satisfies_expression.expression);
            let ty = crate::parser::types::parse_type(&satisfies_expression.type_annotation)
                .unwrap_or(crate::ParsedType::Unknown);
            (
                ParsedExpression::SatisfiesExpression {
                    expression: Box::new(expression),
                    span: Some(text_span_from_oxc_span(expression_span)),
                    target_type: ty,
                    target_span: Some(text_span_from_oxc_span(
                        satisfies_expression.type_annotation.span(),
                    )),
                    keyword_span: satisfies_keyword_span(satisfies_expression),
                },
                satisfies_expression.span,
            )
        }
        Argument::TSNonNullExpression(non_null_expression) => {
            let (expression, _expression_span) = parse_expression(&non_null_expression.expression);
            (
                ParsedExpression::NonNullAssertion {
                    expression: Box::new(expression),
                    span: Some(text_span_from_oxc_span(non_null_expression.span)),
                    in_optional_chain: false,
                },
                non_null_expression.span,
            )
        }
        // Every remaining argument form is an ordinary expression, and tsc
        // checks it as one — `resolveUntypedCall` and the typed-call path both
        // walk every argument through `checkExpression`. Falling through to the
        // sentinel dropped whole `function`/`function*`/`async function`
        // expressions (parameters *and* body went unchecked), along with JSX,
        // `new`, template, tagged-template, `this` and update arguments.
        other => match other.as_expression() {
            Some(expression) => parse_expression(expression),
            None => (ParsedExpression::Unknown, argument.span()),
        },
    };

    ParsedCallArgument {
        expression,
        span: Some(text_span_from_oxc_span(span)),
        spread: false,
        expression_span: Some(text_span_from_oxc_span(span)),
    }
}

fn parse_arrow_function_expression(
    arrow_expression: &ArrowFunctionExpression<'_>,
) -> Option<ParsedArrowFunction> {
    let mut parameters = arrow_expression
        .params
        .items
        .iter()
        .filter_map(parse_function_parameter)
        .collect::<Vec<_>>();
    if let Some(rest) = arrow_expression.params.rest.as_deref() {
        if let Some(rest_parameter) = parse_rest_function_parameter(rest) {
            parameters.push(rest_parameter);
        }
    }
    let (return_type, return_type_span) = super::jsdoc::annotated_or_jsdoc_return_type(
        arrow_expression.return_type.as_deref(),
        arrow_expression.span.start,
    );

    let body_span = arrow_expression
        .get_expression()
        .map(|expression| text_span_from_oxc_span(expression.span()));
    let body = if let Some(expression) = arrow_expression.get_expression() {
        let (expression, _) = parse_expression(expression);
        ParsedArrowFunctionBody::Expression(Box::new(expression))
    } else {
        ParsedArrowFunctionBody::Block(parse_statement_list_as_function_body(
            &arrow_expression.body.statements,
        ))
    };

    Some(ParsedArrowFunction {
        this_binding: ParsedThisBinding::Inherited,
        this_parameter_type: None,
        name: None,
        type_parameters: super::jsdoc::written_or_jsdoc_type_parameters(
            arrow_expression.type_parameters.as_deref(),
            arrow_expression.span.start,
        ),
        parameters,
        return_type,
        return_type_span,
        is_async: arrow_expression.r#async,
        is_generator: false,
        body,
        body_reads: super::reads::collect_function_body_reads(&arrow_expression.body),
        body_span,
        span: Some(text_span_from_oxc_span(arrow_expression.span)),
        narrowing_resets: super::closure_resets::resets_at(arrow_expression.span.start),
        callback_argument: false,
    })
}

fn parse_binary_expression(binary_expression: &BinaryExpression<'_>) -> Option<ParsedExpression> {
    let operator = match binary_expression.operator {
        BinaryOperator::StrictEquality => ParsedBinaryOperator::StrictEquals,
        BinaryOperator::StrictInequality => ParsedBinaryOperator::StrictNotEquals,
        BinaryOperator::Equality => ParsedBinaryOperator::Equals,
        BinaryOperator::Inequality => ParsedBinaryOperator::NotEquals,
        BinaryOperator::LessThan => ParsedBinaryOperator::LessThan,
        BinaryOperator::LessEqualThan => ParsedBinaryOperator::LessThanEquals,
        BinaryOperator::GreaterThan => ParsedBinaryOperator::GreaterThan,
        BinaryOperator::GreaterEqualThan => ParsedBinaryOperator::GreaterThanEquals,
        BinaryOperator::Addition => ParsedBinaryOperator::Add,
        BinaryOperator::Subtraction => ParsedBinaryOperator::Subtract,
        BinaryOperator::Multiplication => ParsedBinaryOperator::Multiply,
        BinaryOperator::Division => ParsedBinaryOperator::Divide,
        BinaryOperator::Remainder => ParsedBinaryOperator::Remainder,
        BinaryOperator::Exponential => ParsedBinaryOperator::Exponential,
        BinaryOperator::ShiftLeft => ParsedBinaryOperator::ShiftLeft,
        BinaryOperator::ShiftRight => ParsedBinaryOperator::ShiftRight,
        BinaryOperator::ShiftRightZeroFill => ParsedBinaryOperator::ShiftRightZeroFill,
        BinaryOperator::BitwiseOR => ParsedBinaryOperator::BitwiseOR,
        BinaryOperator::BitwiseXOR => ParsedBinaryOperator::BitwiseXOR,
        BinaryOperator::BitwiseAnd => ParsedBinaryOperator::BitwiseAnd,
        BinaryOperator::In => ParsedBinaryOperator::In,
        BinaryOperator::Instanceof => ParsedBinaryOperator::Instanceof,
    };

    let (left, left_span) = parse_expression(&binary_expression.left);
    let (right, right_span) = parse_expression(&binary_expression.right);

    // Only the operators TS2447 is about: every other operator diagnostic is
    // anchored on the expression, which is what an absent span falls back to.
    let operator_byte = match operator {
        ParsedBinaryOperator::BitwiseAnd => Some(b'&'),
        ParsedBinaryOperator::BitwiseOR => Some(b'|'),
        ParsedBinaryOperator::BitwiseXOR => Some(b'^'),
        _ => None,
    };
    let operator_span = operator_byte.and_then(|byte| {
        super::spans::operator_token_span(left_span.end, right_span.start, byte)
    });

    Some(ParsedExpression::Binary {
        left: Box::new(left),
        left_span: Some(text_span_from_oxc_span(left_span)),
        operator,
        right: Box::new(right),
        right_span: Some(text_span_from_oxc_span(right_span)),
        operator_span,
    })
}

fn parse_logical_expression(
    logical_expression: &LogicalExpression<'_>,
) -> Option<ParsedExpression> {
    let (mut left, left_span) = parse_expression(&logical_expression.left);
    let (right, right_span) = parse_expression(&logical_expression.right);

    if logical_expression.operator == LogicalOperator::Coalesce {
        return Some(ParsedExpression::NullishCoalescing {
            left: Box::new(left),
            left_span: Some(text_span_from_oxc_span(left_span)),
            right: Box::new(right),
            right_span: Some(text_span_from_oxc_span(right_span)),
        });
    }

    let operator = match logical_expression.operator {
        LogicalOperator::And => ParsedLogicalOperator::And,
        LogicalOperator::Or => ParsedLogicalOperator::Or,
        LogicalOperator::Coalesce => unreachable!(),
    };

    let truthiness_tests = if operator == ParsedLogicalOperator::And {
        if let (
            Expression::LogicalExpression(inner),
            ParsedExpression::Logical { truthiness_tests, .. },
        ) = (&logical_expression.left, &mut left)
            && inner.operator == LogicalOperator::And
        {
            truthiness_tests.clear();
        }
        super::functions::and_operand_truthiness_tests(logical_expression)
    } else {
        Vec::new()
    };

    Some(ParsedExpression::Logical {
        left: Box::new(left),
        left_span: Some(text_span_from_oxc_span(left_span)),
        operator,
        right: Box::new(right),
        right_span: Some(text_span_from_oxc_span(right_span)),
        operator_span: None,
        truthiness_tests,
    })
}

pub(crate) fn parse_conditional_expression(
    conditional_expression: &ConditionalExpression<'_>,
) -> Option<ParsedExpression> {
    let (condition, condition_span) = parse_expression(&conditional_expression.test);
    let (when_true, when_true_span) = parse_expression(&conditional_expression.consequent);
    let (when_false, when_false_span) = parse_expression(&conditional_expression.alternate);

    Some(ParsedExpression::Conditional {
        condition: Box::new(condition),
        condition_span: Some(text_span_from_oxc_span(condition_span)),
        when_true: Box::new(when_true),
        when_true_span: Some(text_span_from_oxc_span(when_true_span)),
        when_false: Box::new(when_false),
        when_false_span: Some(text_span_from_oxc_span(when_false_span)),
        truthiness_tests: super::functions::unreferenced_truthiness_tests_in(
            &conditional_expression.test,
            super::functions::TruthinessBody::Expression(&conditional_expression.consequent),
        ),
    })
}

/// `-1` and `+1` are literals to tsc (`checkPrefixUnaryExpression` returns the
/// fresh literal type of the signed value), so they lower to the literal itself
/// and every syntactic freshness check sees them as one.
pub(crate) fn signed_number_literal_text(unary_expression: &UnaryExpression<'_>) -> Option<String> {
    let Expression::NumericLiteral(literal) = &unary_expression.argument else {
        return None;
    };
    let value = match unary_expression.operator {
        UnaryOperator::UnaryNegation => -literal.value,
        UnaryOperator::UnaryPlus => literal.value,
        _ => return None,
    };
    // `-0` is the literal type `0`.
    Some(super::number_text::js_number_to_string(value))
}

pub(crate) fn parse_unary_expression(
    unary_expression: &UnaryExpression<'_>,
) -> Option<ParsedExpression> {
    if let Some(text) = signed_number_literal_text(unary_expression) {
        return Some(ParsedExpression::NumberLiteral(text));
    }
    let operator = match unary_expression.operator {
        UnaryOperator::LogicalNot => ParsedUnaryOperator::Not,
        UnaryOperator::UnaryPlus => ParsedUnaryOperator::Plus,
        UnaryOperator::UnaryNegation => ParsedUnaryOperator::Minus,
        // `typeof` is preserved (its operand drives type-guard narrowing); it
        // evaluates to `string`. The rest have no modelled result, but dropping
        // the whole expression would stop their operands being checked at all.
        UnaryOperator::Typeof => ParsedUnaryOperator::Typeof,
        UnaryOperator::Delete => ParsedUnaryOperator::Delete,
        UnaryOperator::BitwiseNot => ParsedUnaryOperator::BitwiseNot,
        UnaryOperator::Void => ParsedUnaryOperator::Void,
    };

    let (operand, operand_span) = parse_expression(&unary_expression.argument);

    Some(ParsedExpression::Unary {
        operator,
        operator_span: None,
        operand: Box::new(operand),
        operand_span: Some(text_span_from_oxc_span(operand_span)),
    })
}

/// The member name a non-computed accessor declares. A get/set pair shares one
/// symbol when the names agree as property names — `get 'a'()` with `set a(v)`,
/// `get 0x20()` with `set 3.2e1(v)` — so a quoted or numeric name is the text
/// of its value, as it is for a written property.
fn accessor_key_name(key: &PropertyKey<'_>) -> Option<(String, Span)> {
    match key {
        PropertyKey::StaticIdentifier(key) => Some((key.name.to_string(), key.span)),
        PropertyKey::StringLiteral(literal) => Some((literal.value.to_string(), literal.span)),
        PropertyKey::NumericLiteral(literal) => Some((
            super::number_text::js_number_to_string(literal.value),
            literal.span,
        )),
        _ => None,
    }
}

pub(crate) fn parse_object_properties(
    object_expression: &ObjectExpression<'_>,
) -> Vec<ParsedObjectProperty> {
    let getter_names: Vec<String> = object_expression
        .properties
        .iter()
        .filter_map(|property_kind| match property_kind {
            ObjectPropertyKind::ObjectProperty(property)
                if property.kind == PropertyKind::Get && !property.computed =>
            {
                accessor_key_name(&property.key).map(|(name, _)| name)
            }
            _ => None,
        })
        .collect();
    let computed_getter_names: Vec<String> = object_expression
        .properties
        .iter()
        .filter_map(|property_kind| match property_kind {
            ObjectPropertyKind::ObjectProperty(property)
                if property.kind == PropertyKind::Get && property.computed =>
            {
                super::types::computed_key_name(&property.key)
            }
            _ => None,
        })
        .collect();

    object_expression
        .properties
        .iter()
        .filter_map(|property_kind| {
            let property = match property_kind {
                ObjectPropertyKind::ObjectProperty(property) => property,
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let (value, value_span) = parse_expression(&spread.argument);
                    return Some(ParsedObjectProperty {
                        name: String::new(),
                        name_span: None,
                        value,
                        value_span: Some(text_span_from_oxc_span(value_span)),
                        span: Some(text_span_from_oxc_span(spread.span)),
                        is_method: false,
                        is_spread: true,
                        is_accessor: false,
                        is_getter: false,
                        is_shorthand: false,
                        computed_key: None,
                        paired_setter: None,
                        unnamed_key_value: None,
                        member_key: None,
                    });
                }
            };

            // A computed name is kept as the same `[…]` name the type side uses,
            // so `{ [matcher]: … }` satisfies `Matcher` instead of inferring as
            // `{}` with the member reported missing, and a computed accessor's
            // body is still checked.
            if property.computed && property.kind != PropertyKind::Init {
                let Some(name) = super::types::computed_key_name(&property.key) else {
                    return unnamed_member_key(property);
                };
                // A computed accessor pairs with the one written under the same
                // key, as the late-bound symbol they share does.
                if property.kind == PropertyKind::Set && computed_getter_names.contains(&name) {
                    return None;
                }
                let mut accessor = parse_object_accessor(name, property.key.span(), property)?;
                accessor.member_key = computed_member_key(property);
                if property.kind == PropertyKind::Get {
                    accessor.paired_setter = object_expression.properties.iter().find_map(|other| {
                        let ObjectPropertyKind::ObjectProperty(other) = other else {
                            return None;
                        };
                        if other.kind != PropertyKind::Set || !other.computed {
                            return None;
                        }
                        let Expression::FunctionExpression(function) = &other.value else {
                            return None;
                        };
                        (super::types::computed_key_name(&other.key)? == accessor.name)
                            .then(|| Box::new(function_as_arrow(function, ParsedThisBinding::Own)))
                    });
                }
                return Some(accessor);
            }

            // `get value() { … }` / `set value(v) { … }` declare the property
            // just as a written one does; only their *value* type differs from
            // the accessor function. A setter is dropped when the same literal
            // also declares a getter, which carries it as its pair.
            if matches!(property.kind, PropertyKind::Get | PropertyKind::Set) {
                let (name, key_span) = accessor_key_name(&property.key)?;
                if property.kind == PropertyKind::Set && getter_names.contains(&name) {
                    return None;
                }
                let mut accessor = parse_object_accessor(name, key_span, property)?;
                if property.kind == PropertyKind::Get {
                    let paired_setter = object_expression.properties.iter().find_map(|other| {
                        let ObjectPropertyKind::ObjectProperty(other) = other else {
                            return None;
                        };
                        if other.kind != PropertyKind::Set || other.computed {
                            return None;
                        }
                        let Expression::FunctionExpression(function) = &other.value else {
                            return None;
                        };
                        let (other_name, _) = accessor_key_name(&other.key)?;
                        (other_name == accessor.name).then(|| {
                            Box::new(function_as_arrow(function, ParsedThisBinding::Own))
                        })
                    });
                    accessor.paired_setter = paired_setter;
                }
                return Some(accessor);
            }

            if property.kind != PropertyKind::Init {
                return None;
            }

            if property.method {
                let (name, key_span) = if property.computed {
                    let Some(name) = super::types::computed_key_name(&property.key) else {
                        return unnamed_member_key(property);
                    };
                    (name, property.key.span())
                } else {
                    let PropertyKey::StaticIdentifier(key) = &property.key else {
                        return None;
                    };
                    (key.name.to_string(), key.span)
                };
                let mut method = parse_object_method_shorthand_named(name, key_span, property)?;
                method.member_key = computed_member_key(property);
                return Some(method);
            }

            // A quoted or numeric key names a property like any other. Dropping
            // it silently removed the member from the literal's type, so a
            // fully-quoted literal (Prisma's generated client config) inferred as
            // `{}` and reported every required property as missing.
            let computed_key = || {
                property
                    .computed
                    .then(|| property.key.as_expression())
                    .flatten()
                    .map(|key| Box::new(parse_expression(key).0))
            };
            let bracketed_span = Span::new(property.span.start, property.key.span().end + 1);
            let (name, key_span) = match &property.key {
                // tsc's name node for `[key]` is the whole bracketed name.
                _ if property.computed => match super::types::computed_key_name(&property.key) {
                    Some(name) => (name, bracketed_span),
                    // A key no member name can model (`[f()]`, `` [`k${x}`] ``)
                    // contributes nothing to the literal's type, but its key and
                    // value are still checked; an empty spread carries them
                    // without adding a member.
                    None => {
                        return Some(ParsedObjectProperty {
                            name: String::new(),
                            name_span: Some(text_span_from_oxc_span(bracketed_span)),
                            value: ParsedExpression::ObjectLiteral {
                                properties: Vec::new(),
                                span: None,
                            },
                            value_span: None,
                            span: Some(text_span_from_oxc_span(property.span)),
                            is_method: false,
                            is_spread: true,
                            is_accessor: false,
                            is_getter: false,
                            is_shorthand: false,
                            computed_key: computed_key(),
                            paired_setter: None,
                            unnamed_key_value: Some(Box::new(parse_expression(&property.value).0)),
                            member_key: None,
                        });
                    }
                },
                PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
                PropertyKey::StringLiteral(literal) => (literal.value.to_string(), literal.span),
                PropertyKey::NumericLiteral(literal) => {
                    (super::number_text::js_number_to_string(literal.value), literal.span)
                }
                _ => return None,
            };

            let (value, value_span) = parse_expression(&property.value);

            Some(ParsedObjectProperty {
                name,
                name_span: Some(text_span_from_oxc_span(key_span)),
                value,
                value_span: Some(text_span_from_oxc_span(value_span)),
                span: Some(text_span_from_oxc_span(property.span)),
                is_method: false,
                is_spread: false,
                is_accessor: false,
                is_getter: false,
                is_shorthand: property.shorthand,
                computed_key: computed_key(),
                paired_setter: None,
                unnamed_key_value: None,
                member_key: None,
            })
        })
        .collect()
}

/// The key of a computed method or accessor name, which tsc checks where the
/// literal is (`checkObjectLiteral` runs `checkComputedPropertyName` for every
/// member).
fn computed_member_key(property: &oxc_ast::ast::ObjectProperty<'_>) -> Option<Box<ParsedExpression>> {
    property
        .computed
        .then(|| property.key.as_expression())
        .flatten()
        .map(|key| Box::new(parse_expression(key).0))
}

/// A computed method or accessor whose key no member name can model
/// (`{ *[f()]() {} }`): only its key is kept, on an empty spread that adds
/// nothing to the literal's type.
fn unnamed_member_key(property: &oxc_ast::ast::ObjectProperty<'_>) -> Option<ParsedObjectProperty> {
    Some(ParsedObjectProperty {
        name: String::new(),
        name_span: Some(text_span_from_oxc_span(property.key.span())),
        value: ParsedExpression::ObjectLiteral {
            properties: Vec::new(),
            span: None,
        },
        value_span: None,
        span: Some(text_span_from_oxc_span(property.span)),
        is_method: false,
        is_spread: true,
        is_accessor: false,
        is_getter: false,
        is_shorthand: false,
        computed_key: None,
        paired_setter: None,
        unnamed_key_value: None,
        member_key: Some(computed_member_key(property)?),
    })
}

/// Lowers a `get`/`set` accessor into a property carrying the accessor's arrow.
/// The checker reads the arrow's return type (getter) or parameter type (setter)
/// as the property's type.
fn parse_object_accessor(
    name: String,
    key_span: Span,
    property: &oxc_ast::ast::ObjectProperty<'_>,
) -> Option<ParsedObjectProperty> {
    let mut parsed = parse_object_method_shorthand_named(name, key_span, property)?;
    parsed.is_method = false;
    parsed.is_accessor = true;
    parsed.is_getter = property.kind == PropertyKind::Get;
    Some(parsed)
}

/// Lowers object literal method shorthand (`{ foo(arg): R { ... } }`) into a property whose
/// value is an arrow function, so it reuses the existing arrow-function checking path while
/// honoring the declared parameter and return types.
fn parse_object_method_shorthand_named(
    name: String,
    key_span: oxc_span::Span,
    property: &oxc_ast::ast::ObjectProperty<'_>,
) -> Option<ParsedObjectProperty> {
    let Expression::FunctionExpression(function) = &property.value else {
        return None;
    };

    let arrow = function_as_arrow(function, ParsedThisBinding::Own);

    Some(ParsedObjectProperty {
        name,
        name_span: Some(text_span_from_oxc_span(key_span)),
        value: ParsedExpression::ArrowFunction(Box::new(arrow)),
        value_span: Some(text_span_from_oxc_span(function.span)),
        span: Some(text_span_from_oxc_span(property.span)),
        is_method: true,
        is_spread: false,
        is_accessor: false,
        is_getter: false,
        is_shorthand: false,
        computed_key: None,
        paired_setter: None,
        unnamed_key_value: None,
        member_key: None,
    })
}

pub(crate) fn parse_array_expression(
    array_expression: &ArrayExpression<'_>,
) -> Option<ParsedExpression> {
    let mut elements = Vec::new();

    for element in &array_expression.elements {
        // An omitted element (`[1, , 3]`) is still an element: tsc types it
        // `undefined`, and dropping it shifted every later element's position.
        if element.is_elision() {
            elements.push(crate::ParsedArrayElement {
                expression: ParsedExpression::UndefinedLiteral,
                span: None,
                spread: false,
                omitted: true,
            });
            continue;
        }

        // A spread element used to drop the *whole* literal, so `[...xs]` and
        // `[...xs, tail]` — the commonest way to copy or extend an array — had
        // no type at all and every diagnostic that depends on one went missing.
        // Keep the operand and record that it stands for the elements of what
        // it spreads.
        if let ArrayExpressionElement::SpreadElement(spread) = element {
            let (parsed_expression, span) = parse_expression(&spread.argument);
            elements.push(crate::ParsedArrayElement {
                expression: parsed_expression,
                span: Some(text_span_from_oxc_span(span)),
                spread: true,
                omitted: false,
            });
            continue;
        }

        let Some(expression) = element.as_expression() else {
            return None;
        };

        let (parsed_expression, span) = parse_expression(expression);
        elements.push(crate::ParsedArrayElement {
            expression: parsed_expression,
            span: Some(text_span_from_oxc_span(span)),
            spread: false,
            omitted: false,
        });
    }

    Some(ParsedExpression::ArrayLiteral {
        elements,
        span: Some(text_span_from_oxc_span(array_expression.span())),
        tuple_context: false,
    })
}

/// `x++` / `--o.p`. Without this the whole expression was dropped, so the
/// operand was never even walked: neither its own errors nor the write it
/// performs were checked.
/// Lowers a `function` (expression or object-literal method) to the arrow shape
/// the checker already knows how to walk. The `this` binding is what keeps the
/// two apart afterwards: a method has its own, a plain function expression's is
/// untyped.
fn function_as_arrow(
    function: &oxc_ast::ast::Function<'_>,
    this_binding: ParsedThisBinding,
) -> ParsedArrowFunction {
    let mut parameters = function
        .params
        .items
        .iter()
        .filter_map(parse_function_parameter)
        .collect::<Vec<_>>();
    if let Some(rest) = function.params.rest.as_deref()
        && let Some(rest_parameter) = parse_rest_function_parameter(rest)
    {
        parameters.push(rest_parameter);
    }

    let (return_type, return_type_span) = super::jsdoc::annotated_or_jsdoc_return_type(
        function.return_type.as_deref(),
        function.span.start,
    );
    ParsedArrowFunction {
        this_binding,
        this_parameter_type: function
            .this_param
            .as_ref()
            .and_then(|this_param| this_param.type_annotation.as_ref())
            .and_then(|annotation| super::types::parse_type_annotation(annotation)),
        name: None,
        type_parameters: super::jsdoc::written_or_jsdoc_type_parameters(
            function.type_parameters.as_deref(),
            function.span.start,
        ),
        parameters,
        return_type,
        return_type_span,
        is_async: function.r#async,
        is_generator: function.generator,
        body: ParsedArrowFunctionBody::Block(
            function
                .body
                .as_ref()
                .map(|body| parse_statement_list_as_function_body(&body.statements))
                .unwrap_or_default(),
        ),
        body_reads: function
            .body
            .as_ref()
            .map(|body| super::reads::collect_function_body_reads(body))
            .unwrap_or_default(),
        body_span: None,
        span: Some(text_span_from_oxc_span(function.span)),
        narrowing_resets: super::closure_resets::resets_at(function.span.start),
        callback_argument: matches!(this_binding, ParsedThisBinding::ImplicitAny)
            && super::callback_functions::is_callback_function(function.span.start),
    }
}

/// `const f = function () { … }`. Without this the whole expression was dropped
/// as `Unknown`, so nothing inside the body was checked at all.
pub(crate) fn parse_function_expression(
    function: &oxc_ast::ast::Function<'_>,
) -> ParsedArrowFunction {
    // oxc keeps a `this` parameter out of the parameter list; annotating it is
    // what gives the body a typed `this`.
    let this_binding = if function.this_param.is_some() {
        ParsedThisBinding::Own
    } else {
        ParsedThisBinding::ImplicitAny
    };
    ParsedArrowFunction {
        name: function.id.as_ref().map(|id| id.name.to_string()),
        ..function_as_arrow(function, this_binding)
    }
}

pub(crate) fn parse_update_expression(
    update_expression: &UpdateExpression<'_>,
) -> Option<ParsedExpression> {
    // The operand is parsed into the same shape a *read* of it produces, so the
    // checker resolves the updated binding exactly as it resolves the read.
    let rejected_target = matches!(
        &update_expression.argument,
        SimpleAssignmentTarget::TSNonNullExpression(recovered) if recovered.span == recovered.expression.span()
    );
    let (operand, operand_span) = match &update_expression.argument {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => (
            ParsedExpression::Identifier {
                name: identifier.name.to_string(),
                span: Some(text_span_from_oxc_span(identifier.span)),
            },
            identifier.span,
        ),
        SimpleAssignmentTarget::StaticMemberExpression(member) => (
            parse_static_member_expression(member)?,
            member.span,
        ),
        SimpleAssignmentTarget::ComputedMemberExpression(member) => (
            parse_computed_member_expression(member)?,
            member.span,
        ),
        SimpleAssignmentTarget::PrivateFieldExpression(member) => {
            (parse_private_field_expression(member), member.span)
        }
        // The parser keeps an operand tsc rejects (TS2357/TS2777) in a non-null
        // wrapper spanning exactly that operand; tsc still checks it as an
        // arithmetic operand.
        SimpleAssignmentTarget::TSNonNullExpression(recovered)
            if recovered.span == recovered.expression.span() =>
        {
            (parse_expression(&recovered.expression).0, recovered.span)
        }
        _ => return None,
    };

    if operand == ParsedExpression::Unknown {
        return None;
    }

    Some(ParsedExpression::Update {
        operand: Box::new(operand),
        operand_span: Some(text_span_from_oxc_span(operand_span)),
        rejected_target,
    })
}

pub(crate) fn parse_static_member_expression(
    member_expression: &StaticMemberExpression<'_>,
) -> Option<ParsedExpression> {
    if member_expression.optional {
        let (object, object_span) = parse_expression(&member_expression.object);
        return Some(ParsedExpression::OptionalPropertyAccess {
            object: Box::new(object),
            object_span: Some(text_span_from_oxc_span(object_span)),
            property_name: member_expression.property.name.to_string(),
            property_span: Some(text_span_from_oxc_span(member_expression.property.span)),
            is_bracketed: false,
        });
    }

    let (object, object_span) = parse_expression(&member_expression.object);
    Some(ParsedExpression::PropertyAccess {
        object: Box::new(object),
        object_span: Some(text_span_from_oxc_span(object_span)),
        property_name: member_expression.property.name.to_string(),
        property_span: Some(text_span_from_oxc_span(member_expression.property.span)),
        is_bracketed: false,
        binding_element: false,
    })
}

/// `o.#x`: a member access whose key is the private name as the enclosing
/// class bodies resolve it (see `private_names`).
pub(crate) fn parse_private_field_expression(member_expression: &PrivateFieldExpression<'_>) -> ParsedExpression {
    let (object, object_span) = parse_expression(&member_expression.object);
    let object = Box::new(object);
    let object_span = Some(text_span_from_oxc_span(object_span));
    let property_name = super::private_names::access_key(member_expression.field.name.as_str());
    let property_span = Some(text_span_from_oxc_span(member_expression.field.span));
    if member_expression.optional {
        ParsedExpression::OptionalPropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed: false,
        }
    } else {
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed: false,
            binding_element: false,
        }
    }
}

fn set_in_optional_chain(expr: &mut ParsedExpression) {
    match expr {
        ParsedExpression::NonNullAssertion {
            in_optional_chain,
            expression,
            ..
        } => {
            *in_optional_chain = true;
            set_in_optional_chain(expression);
        }
        ParsedExpression::PropertyAccess { object, .. } => set_in_optional_chain(object),
        ParsedExpression::OptionalPropertyAccess { object, .. } => set_in_optional_chain(object),
        ParsedExpression::OptionalIndexAccess { object, .. } => set_in_optional_chain(object),
        ParsedExpression::PropertyCall { object, .. } => set_in_optional_chain(object),
        ParsedExpression::OptionalPropertyCall { object, .. } => set_in_optional_chain(object),
        ParsedExpression::OptionalCall { callee, .. } => set_in_optional_chain(callee),
        _ => {}
    }
}

fn parse_chain_expression(chain_expression: &ChainExpression<'_>) -> Option<ParsedExpression> {
    let mut parsed = match &chain_expression.expression {
        ChainElement::CallExpression(call_expression) => {
            parse_call_expression_expression(call_expression)
        }
        ChainElement::StaticMemberExpression(member_expression) => {
            parse_static_member_expression(member_expression)
        }
        ChainElement::ComputedMemberExpression(member_expression) => {
            parse_computed_member_expression(member_expression)
        }
        ChainElement::PrivateFieldExpression(member_expression) => {
            Some(parse_private_field_expression(member_expression))
        }
        ChainElement::TSNonNullExpression(non_null_expression) => {
            let (expression, _expression_span) = parse_expression(&non_null_expression.expression);
            Some(ParsedExpression::NonNullAssertion {
                expression: Box::new(expression),
                span: Some(text_span_from_oxc_span(non_null_expression.span)),
                in_optional_chain: true,
            })
        }
        _ => None,
    };

    if let Some(ref mut expr) = parsed {
        set_in_optional_chain(expr);
    }

    parsed
}

pub(super) fn parse_computed_member_expression(
    member_expression: &ComputedMemberExpression<'_>,
) -> Option<ParsedExpression> {
    // `o[]` (TS1011): the parser's placeholder argument is empty, and the access
    // has no key to check.
    if member_expression.expression.span().is_empty() {
        return None;
    }
    // String-literal bracket access (`obj["key"]`, `obj?.["key"]`) lowers to the
    // same (optional) property-access nodes as dot access so it reuses identical
    // property-lookup, optional-widening, and missing-property behavior.
    //
    // Purely numeric keys (e.g. `arr["0"]`) are left on the index-access path so
    // existing array/tuple numeric-index behavior is preserved unchanged.
    //
    // A template without substitutions (`` obj[`key`] ``) is the same key: tsc
    // treats both as string-literal-like.
    let literal_key = match &member_expression.expression {
        Expression::StringLiteral(string_literal) => {
            Some((string_literal.value.to_string(), string_literal.span))
        }
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => template
            .quasis
            .first()
            .and_then(|quasi| quasi.value.cooked.as_ref())
            .map(|cooked| (cooked.to_string(), template.span)),
        _ => None,
    };
    if let Some((key, key_span)) = literal_key {
        let is_numeric_index = !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_digit());

        if !is_numeric_index {
            let (object, object_span) = parse_expression(&member_expression.object);
            let property_name = key;
            let property_span = Some(text_span_from_oxc_span(key_span));

            if member_expression.optional {
                return Some(ParsedExpression::OptionalPropertyAccess {
                    object: Box::new(object),
                    object_span: Some(text_span_from_oxc_span(object_span)),
                    property_name,
                    property_span,
                    is_bracketed: true,
                });
            }

            return Some(ParsedExpression::PropertyAccess {
                object: Box::new(object),
                object_span: Some(text_span_from_oxc_span(object_span)),
                property_name,
                property_span,
                is_bracketed: true,
                binding_element: false,
            });
        }
    }

    if member_expression.optional {
        let (object, object_span) = parse_expression(&member_expression.object);
        let (index, index_span) = parse_expression(&member_expression.expression);
        return Some(ParsedExpression::OptionalIndexAccess {
            object: Box::new(object),
            object_span: Some(text_span_from_oxc_span(object_span)),
            index: Box::new(index),
            index_span: Some(text_span_from_oxc_span(index_span)),
        });
    }

    let (index, index_span) = parse_expression(&member_expression.expression);
    let index_span = Some(text_span_from_oxc_span(index_span));

    if let Expression::Identifier(object_identifier) = &member_expression.object {
        return Some(ParsedExpression::IndexAccess {
            object_name: object_identifier.name.to_string(),
            object_span: Some(text_span_from_oxc_span(object_identifier.span)),
            index: Box::new(index),
            index_span,
        });
    }

    // `box.list[0]`, `f()[i]`: the receiver is any expression, and dropping
    // the access left everything inside it unchecked.
    let (object, object_span) = parse_expression(&member_expression.object);
    Some(ParsedExpression::ElementAccess {
        object: Box::new(object),
        object_span: Some(text_span_from_oxc_span(object_span)),
        index: Box::new(index),
        index_span,
    })
}

/// The expression the parser keeps for a write target tsc rejects
/// (TS2364), in a non-null wrapper of exactly its span.
pub(crate) fn rejected_assignment_target<'a, 'b>(
    target: &'b oxc_ast::ast::AssignmentTarget<'a>,
) -> Option<&'b Expression<'a>> {
    match target {
        oxc_ast::ast::AssignmentTarget::TSNonNullExpression(recovered)
            if recovered.span == recovered.expression.span() =>
        {
            Some(&recovered.expression)
        }
        _ => None,
    }
}

/// An assignment to a target tsc rejects: `checkBinaryLikeExpression` still
/// checks the target as an expression, then the value, which is the result.
pub(crate) fn parse_rejected_assignment(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedExpression> {
    let target = rejected_assignment_target(&assignment.left)?;
    let (target, target_span) = parse_expression(target);
    let (value, value_span) = parse_expression(&assignment.right);
    Some(ParsedExpression::Sequence {
        expressions: vec![
            (target, Some(text_span_from_oxc_span(target_span))),
            (value, Some(text_span_from_oxc_span(value_span))),
        ],
    })
}

fn parse_assignment_value(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedExpression> {
    if let Some(rejected) = parse_rejected_assignment(assignment) {
        return Some(rejected);
    }
    if let Some(value) = super::commonjs::export_assignment_value(assignment) {
        return Some(value);
    }
    let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left
    else {
        return parse_member_assignment_value(assignment);
    };
    let target_span = Some(text_span_from_oxc_span(identifier.span));
    let (value, value_span) = parse_expression(&assignment.right);
    let value_span = Some(text_span_from_oxc_span(value_span));
    let target = ParsedExpression::Identifier {
        name: identifier.name.to_string(),
        span: target_span,
    };
    let value =
        super::logical_assignment_value(assignment.operator, target, target_span, value, value_span)?;
    Some(ParsedExpression::Assignment {
        target_name: identifier.name.to_string(),
        target_span,
        value: Box::new(value),
        value_span,
    })
}

/// In JavaScript, a member write that is the value of another write or of a
/// variable's initializer (`F.a = F.b = v`, `var g = F.g = {}`): the binder
/// declares an expando for it there as anywhere (`bindExpandoPropertyAssignment`),
/// and the declarations collected from those two places see it. Writes
/// through `this`/`super` and the CommonJS export targets keep their own
/// lowering.
pub(super) fn parse_javascript_member_assignment_chain(expression: &Expression<'_>) -> Option<ParsedExpression> {
    if !super::spans::lowering_javascript() {
        return None;
    }
    let Expression::AssignmentExpression(assignment) = expression.without_parentheses() else {
        return None;
    };
    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign
        || super::commonjs::is_export_target(&assignment.left)
    {
        return None;
    }
    let object = match &assignment.left {
        oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) => &member.object,
        oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => &member.object,
        _ => return None,
    };
    if matches!(object.without_parentheses(), Expression::ThisExpression(_) | Expression::Super(_)) {
        return None;
    }
    super::functions::parse_member_assignment(assignment)
        .map(|member| ParsedExpression::MemberAssignment(Box::new(member)))
}

/// A member write used as a value (`(o.p = f())`, `a.b = c.d = v`), which tsc
/// checks as it checks the statement (`checkBinaryLikeExpression`). A write
/// through `this` or `super` keeps to its statement form's own rules, and so
/// does any in JavaScript, where the write may declare the member it names
/// (`bindExpandoPropertyAssignment`, `bindThisPropertyAssignment`) and surge
/// binds those declarations from statements only.
fn parse_member_assignment_value(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedExpression> {
    if super::spans::lowering_javascript() {
        return None;
    }
    let object = match &assignment.left {
        oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) => &member.object,
        oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => &member.object,
        oxc_ast::ast::AssignmentTarget::PrivateFieldExpression(member) => &member.object,
        _ => return None,
    };
    if matches!(
        object.without_parentheses(),
        Expression::ThisExpression(_) | Expression::Super(_)
    ) {
        return None;
    }
    super::functions::parse_member_assignment(assignment)
        .map(|member| ParsedExpression::MemberAssignment(Box::new(member)))
}
