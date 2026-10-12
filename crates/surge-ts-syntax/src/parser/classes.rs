use oxc_ast::ast::{
    Class, ClassElement, Expression, MethodDefinitionKind, MethodDefinitionType,
    PropertyDefinitionType, PropertyKey,
};
use oxc_syntax::operator::BinaryOperator;

use oxc_span::GetSpan;

use crate::{
    ParsedClassAccessor, ParsedClassConstructor, ParsedClassDeclaration, ParsedClassMember,
    ParsedAccessorSide, ParsedClassMethod, ParsedClassProperty, ParsedDecorator,
    ParsedDecoratorTarget, ParsedMemberAccessibility, ParsedNamedType, ParsedRestrictedMember,
};

use super::expressions::parse_expression;
use super::functions::{
    parse_function_parameter, parse_rest_function_parameter, parse_statement_list_as_function_body,
};
use super::spans::text_span_from_oxc_span;
use super::types::{parse_type_annotation, parse_type_arguments};

pub(crate) fn parse_class_declaration(class: &Class<'_>) -> Option<ParsedClassDeclaration> {
    let id = class.id.as_ref()?;
    parse_class_declaration_named(class, id.name.to_string(), text_span_from_oxc_span(id.span))
}

/// A class under a name it may not write itself: an anonymous class
/// expression takes its variable's (`getNameOfSymbolAsWritten`) and an
/// anonymous default export is `default`. `name_span` is where tsc reports
/// the class, its name or else its first token.
pub(crate) fn parse_class_declaration_named(
    class: &Class<'_>,
    name: String,
    name_span: crate::TextSpan,
) -> Option<ParsedClassDeclaration> {
    // The class's own decorators name private names from outside its body
    // (`getContainingClassExcludingClassDecorators`); everything else lowers
    // inside it.
    let decorators = class_decorators(class);
    let scope_name = name.clone();
    super::private_names::with_class(class, &scope_name, || {
        super::types::with_declaring_type(&scope_name, class.type_parameters.as_deref(), || {
            parse_class_body(class, name, name_span, decorators)
        })
    })
}

fn parse_class_body(
    class: &Class<'_>,
    name: String,
    name_span: crate::TextSpan,
    mut decorators: Vec<ParsedDecorator>,
) -> Option<ParsedClassDeclaration> {
    decorators.extend(member_decorators(class));
    let mut members = merge_class_accessors(
        class
            .body
            .body
            .iter()
            .flat_map(|member| parse_class_members(member))
            .collect(),
    );
    let dynamic_members = class
        .body
        .body
        .iter()
        .filter(|member| has_dynamic_name(member))
        .filter_map(|member| parse_class_member(member, true))
        .collect();
    let mut restricted_members = restricted_class_members(class);
    if super::spans::lowering_javascript() {
        let assigned = javascript_this_members(class, &members, &mut restricted_members);
        members.extend(assigned);
    }

    // The last signature of each key kind and side wins, as for an interface.
    // A `symbol` or pattern key answers no named member (Prisma's client class
    // declares `[K: symbol]`), so only `string` and `number` keys are kept.
    let index_signature_of = |numeric: bool, is_static: bool| {
        class
            .body
            .body
            .iter()
            .filter_map(|element| match element {
                ClassElement::TSIndexSignature(index_signature)
                    if index_signature.r#static == is_static
                        && index_signature.parameters.first().is_some_and(|parameter| {
                            matches!(
                                parameter.type_annotation.type_annotation,
                                oxc_ast::ast::TSType::TSStringKeyword(_)
                                    | oxc_ast::ast::TSType::TSNumberKeyword(_)
                            )
                        })
                        && super::types::index_signature_is_numeric(index_signature) == numeric =>
                {
                    super::types::parse_index_signature_value_type(index_signature).map(|value_type| {
                        (
                            (value_type, text_span_from_oxc_span(index_signature.span)),
                            index_signature.readonly,
                        )
                    })
                }
                _ => None,
            })
            .next_back()
    };
    let (string_index, string_index_readonly) = index_signature_of(false, false).unzip();
    let (string_index_type, string_index_span) = string_index.unzip();
    let (number_index, number_index_readonly) = index_signature_of(true, false).unzip();
    let (number_index_type, number_index_span) = number_index.unzip();
    let (static_string_index, static_string_index_readonly) = index_signature_of(false, true).unzip();
    let static_string_index_type = static_string_index.map(|(ty, _)| ty);
    let (static_number_index, static_number_index_readonly) = index_signature_of(true, true).unzip();
    let (static_number_index_type, static_number_index_span) = static_number_index.unzip();

    let (symbol_index, symbol_index_span) = class
        .body
        .body
        .iter()
        .find_map(|element| match element {
            ClassElement::TSIndexSignature(index_signature)
                if !index_signature.r#static && super::types::index_signature_slots(index_signature).symbol =>
            {
                super::types::parse_index_signature_value_type(index_signature)
                    .map(|value| (value, text_span_from_oxc_span(index_signature.span)))
            }
            _ => None,
        })
        .unzip();
    let keyed_indexes = symbol_index.map(|symbol| {
        std::sync::Arc::new(crate::ParsedKeyedIndexes {
            signatures: Vec::new(),
            symbol: Some(symbol),
        })
    });

    Some(ParsedClassDeclaration {
        keyed_indexes,
        string_index_type,
        number_index_type,
        string_index_span,
        number_index_span,
        symbol_index_span,
        static_string_index_type,
        static_number_index_type,
        static_number_index_span,
        string_index_readonly: string_index_readonly.unwrap_or(false),
        number_index_readonly: number_index_readonly.unwrap_or(false),
        static_string_index_readonly: static_string_index_readonly.unwrap_or(false),
        static_number_index_readonly: static_number_index_readonly.unwrap_or(false),
        is_declare: class.declare,
        const_binding: false,
        is_abstract: class.r#abstract,
        name,
        name_span: Some(name_span),
        type_parameters: super::jsdoc::written_or_jsdoc_type_parameters(
            class.type_parameters.as_deref(),
            class.span.start,
        ),
        extends: parse_class_heritage(class),
        heritage_expression: class
            .super_class
            .as_ref()
            .filter(|super_class| super::types::flatten_heritage_expression(super_class).is_none())
            .map(|super_class| Box::new(parse_expression(super_class).0)),
        // oxc folds every `implements` clause into one list; tsc's
        // `getImplementsTypeNodes` reads the first clause only (a second one
        // is TS1175).
        implements: class
            .implements
            .iter()
            .enumerate()
            .take_while(|(index, implemented)| {
                *index == 0 || {
                    let previous = &class.implements[index - 1];
                    let gap = super::spans::source_text_of(oxc_span::Span::new(previous.span.end, implemented.span.start));
                    !gap.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$').any(|word| word == "implements")
                }
            })
            .map(|(_, implemented)| implemented)
            .filter_map(|implemented| {
                let (name, span) = super::types::flatten_type_name(&implemented.expression)?;
                Some(ParsedNamedType {
                    type_argument_spans: Vec::new(),
                    name,
                    span: Some(span),
                    type_arguments: implemented
                        .type_arguments
                        .as_deref()
                        .and_then(super::types::parse_type_arguments)
                        .unwrap_or_default(),
                })
            })
            .chain(super::jsdoc::implements_at(class.span.start))
            .collect(),
        members,
        dynamic_members,
        restricted_members,
        span: Some(text_span_from_oxc_span(class.span)),
        non_emitting_computed_keys: class
            .body
            .body
            .iter()
            .filter_map(|element| {
                let (key, computed, is_get_or_set, non_emitting) = match element {
                    ClassElement::MethodDefinition(member) => (
                        &member.key,
                        member.computed,
                        member.kind.is_accessor(),
                        member.r#type == MethodDefinitionType::TSAbstractMethodDefinition,
                    ),
                    ClassElement::PropertyDefinition(member) => (
                        &member.key,
                        member.computed,
                        false,
                        member.declare
                            || member.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition,
                    ),
                    ClassElement::AccessorProperty(member) => (
                        &member.key,
                        member.computed,
                        false,
                        member.r#type == oxc_ast::ast::AccessorPropertyType::TSAbstractAccessorProperty,
                    ),
                    _ => return None,
                };
                let expression = key.as_expression().filter(|_| computed)?;
                if !is_get_or_set && is_in_expression(expression) {
                    return None;
                }
                Some(non_emitting)
            })
            .collect(),
        this_computed_keys: class
            .body
            .body
            .iter()
            .filter_map(|element| {
                let (key, computed, is_get_or_set, is_static) = match element {
                    ClassElement::MethodDefinition(member) => {
                        (&member.key, member.computed, member.kind.is_accessor(), member.r#static)
                    }
                    ClassElement::PropertyDefinition(member) => (&member.key, member.computed, false, member.r#static),
                    ClassElement::AccessorProperty(member) => (&member.key, member.computed, false, member.r#static),
                    _ => return None,
                };
                let expression = key.as_expression().filter(|_| computed)?;
                if !is_get_or_set && is_in_expression(expression) {
                    return None;
                }
                Some(super::spans::source_text_of(expression.span()).contains("this").then_some(is_static))
            })
            .collect(),
        computed_keys: class
            .body
            .body
            .iter()
            .filter_map(|element| {
                let (key, computed, is_get_or_set) = match element {
                    ClassElement::MethodDefinition(member) => (
                        &member.key,
                        member.computed,
                        member.kind.is_accessor(),
                    ),
                    ClassElement::PropertyDefinition(member) => (&member.key, member.computed, false),
                    ClassElement::AccessorProperty(member) => (&member.key, member.computed, false),
                    _ => return None,
                };
                let expression = key.as_expression().filter(|_| computed)?;
                // tsc's `isInvalidComputedPropertyName`: a `[k in T]` name is a
                // misplaced mapped type (TS7061) and its expression is never checked.
                if !is_get_or_set && is_in_expression(expression) {
                    return None;
                }
                let (parsed, _) = parse_expression(expression);
                // The name's `[`, which the key's own span leaves out.
                let span = key.span();
                Some((
                    parsed,
                    Some(crate::TextSpan {
                        start: (span.start as usize).saturating_sub(1),
                        end: span.end as usize + 1,
                    }),
                ))
            })
            .collect(),
        decorators,
    })
}

fn lower_decorators(
    decorators: &[oxc_ast::ast::Decorator<'_>],
    target: ParsedDecoratorTarget,
) -> Vec<ParsedDecorator> {
    decorators
        .iter()
        .map(|decorator| ParsedDecorator {
            expression: parse_expression(&decorator.expression).0,
            span: Some(text_span_from_oxc_span(decorator.expression.span())),
            target,
            needs_parentheses: decorator_needs_parentheses(&decorator.expression),
            decorator_span: Some(text_span_from_oxc_span(decorator.span)),
            expression_text: super::spans::source_text_of(decorator.expression.span()),
            is_parenthesized: matches!(decorator.expression, Expression::ParenthesizedExpression(_)),
        })
        .collect()
}

fn class_decorators(class: &Class<'_>) -> Vec<ParsedDecorator> {
    lower_decorators(&class.decorators, ParsedDecoratorTarget::Class)
}

/// The decorators on the class's members and their parameters, in source
/// order.
fn member_decorators(class: &Class<'_>) -> Vec<ParsedDecorator> {
    let mut out = Vec::new();
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                let target = ParsedDecoratorTarget::Method {
                    has_body: method.value.body.is_some(),
                    private_name: matches!(method.key, PropertyKey::PrivateIdentifier(_)),
                };
                out.extend(lower_decorators(&method.decorators, target));
                if method.value.body.is_some() && method.kind != MethodDefinitionKind::Get {
                    let target =
                        ParsedDecoratorTarget::Parameter { of_constructor: method.kind == MethodDefinitionKind::Constructor };
                    for parameter in &method.value.params.items {
                        out.extend(lower_decorators(&parameter.decorators, target));
                    }
                }
            }
            ClassElement::PropertyDefinition(property) => {
                let target = ParsedDecoratorTarget::Property {
                    is_abstract: property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition,
                    is_declare: property.declare,
                    private_name: matches!(property.key, PropertyKey::PrivateIdentifier(_)),
                    is_auto_accessor: false,
                };
                out.extend(lower_decorators(&property.decorators, target));
            }
            ClassElement::AccessorProperty(accessor) => {
                let target = ParsedDecoratorTarget::Property {
                    is_abstract: accessor.r#type == oxc_ast::ast::AccessorPropertyType::TSAbstractAccessorProperty,
                    is_declare: false,
                    private_name: matches!(accessor.key, PropertyKey::PrivateIdentifier(_)),
                    is_auto_accessor: true,
                };
                out.extend(lower_decorators(&accessor.decorators, target));
            }
            _ => {}
        }
    }
    out
}

/// tsc's `checkGrammarDecorator`: a decorator is `(expression)`, or a
/// dotted name, each part a property access, with at most one call at the
/// end, no optional chaining, and non-null assertions or type arguments
/// anywhere.
fn decorator_needs_parentheses(expression: &Expression<'_>) -> bool {
    use oxc_ast::ast::ChainElement;
    if matches!(expression, Expression::ParenthesizedExpression(_)) {
        return false;
    }
    enum Part<'e, 'a> {
        Expression(&'e Expression<'a>),
        Chain(&'e ChainElement<'a>),
    }
    let mut node = Part::Expression(expression);
    let mut can_have_call = true;
    let mut invalid = false;
    loop {
        let (inner, optional, is_call): (Part<'_, '_>, bool, bool) = match node {
            Part::Expression(Expression::TSNonNullExpression(e)) => (Part::Expression(&e.expression), false, false),
            Part::Expression(Expression::TSInstantiationExpression(e)) => (Part::Expression(&e.expression), false, false),
            Part::Expression(Expression::ChainExpression(chain)) => (Part::Chain(&chain.expression), false, false),
            Part::Expression(Expression::CallExpression(call)) => (Part::Expression(&call.callee), call.optional, true),
            Part::Chain(ChainElement::CallExpression(call)) => (Part::Expression(&call.callee), call.optional, true),
            Part::Expression(Expression::StaticMemberExpression(member)) => {
                (Part::Expression(&member.object), member.optional, false)
            }
            Part::Chain(ChainElement::StaticMemberExpression(member)) => {
                (Part::Expression(&member.object), member.optional, false)
            }
            Part::Expression(Expression::PrivateFieldExpression(member)) => {
                (Part::Expression(&member.object), member.optional, false)
            }
            Part::Chain(ChainElement::PrivateFieldExpression(member)) => {
                (Part::Expression(&member.object), member.optional, false)
            }
            Part::Chain(ChainElement::TSNonNullExpression(e)) => (Part::Expression(&e.expression), false, false),
            Part::Expression(Expression::Identifier(_)) => return invalid,
            _ => return true,
        };
        if optional || is_call && !can_have_call {
            invalid = true;
        }
        let is_member_or_call = is_call
            || matches!(
                node,
                Part::Expression(Expression::StaticMemberExpression(_) | Expression::PrivateFieldExpression(_))
                    | Part::Chain(ChainElement::StaticMemberExpression(_) | ChainElement::PrivateFieldExpression(_))
            );
        if is_member_or_call {
            can_have_call = false;
        }
        node = inner;
    }
}

fn is_in_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::BinaryExpression(binary) => binary.operator == BinaryOperator::In,
        Expression::PrivateInExpression(_) => true,
        _ => false,
    }
}

fn restricted_class_members(class: &Class<'_>) -> Vec<ParsedRestrictedMember> {
    let mut restricted = Vec::new();
    // A duplicate member's modifiers are its symbol's `valueDeclaration`'s,
    // which is the first declaration of the name.
    let mut declared: std::collections::HashSet<(String, bool)> = std::collections::HashSet::new();
    for element in &class.body.body {
        let (key, computed, is_static, accessibility, accessor_side) = match element {
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor =>
            {
                for parameter in &method.value.params.items {
                    if let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) =
                        &parameter.pattern
                        && (parameter.accessibility.is_some() || parameter.readonly || parameter.r#override)
                        && declared.insert((identifier.name.to_string(), false))
                        && let Some(accessibility) = restricted_accessibility(parameter.accessibility)
                    {
                        restricted.push(ParsedRestrictedMember {
                            name: identifier.name.to_string(),
                            is_static: false,
                            accessibility,
                            accessor_side: None,
                        });
                    }
                }
                continue;
            }
            ClassElement::MethodDefinition(method) => (
                &method.key,
                method.computed,
                method.r#static,
                method.accessibility,
                match method.kind {
                    MethodDefinitionKind::Get => Some(ParsedAccessorSide::Get),
                    MethodDefinitionKind::Set => Some(ParsedAccessorSide::Set),
                    _ => None,
                },
            ),
            ClassElement::PropertyDefinition(property) => (
                &property.key,
                property.computed,
                property.r#static,
                property.accessibility,
                None,
            ),
            ClassElement::AccessorProperty(property) => (
                &property.key,
                property.computed,
                property.r#static,
                property.accessibility,
                None,
            ),
            _ => continue,
        };
        let name = if computed {
            super::types::computed_key_name(key)
        } else {
            match key {
                PropertyKey::StaticIdentifier(key) => Some(key.name.to_string()),
                key => super::types::computed_key_name(key),
            }
        };
        if accessor_side.is_none()
            && let Some(name) = &name
            && !declared.insert((name.clone(), is_static))
            && !matches!(element, ClassElement::MethodDefinition(_))
        {
            continue;
        }
        let Some(accessibility) = restricted_accessibility(accessibility)
            .or_else(|| super::jsdoc::member_accessibility_at(element.span().start))
        else {
            continue;
        };
        if let Some(name) = name {
            restricted.push(ParsedRestrictedMember {
                name,
                is_static,
                accessibility,
                accessor_side,
            });
        }
    }
    restricted
}

fn restricted_accessibility(
    accessibility: Option<oxc_ast::ast::TSAccessibility>,
) -> Option<ParsedMemberAccessibility> {
    match accessibility? {
        oxc_ast::ast::TSAccessibility::Private => Some(ParsedMemberAccessibility::Private),
        oxc_ast::ast::TSAccessibility::Protected => Some(ParsedMemberAccessibility::Protected),
        oxc_ast::ast::TSAccessibility::Public => None,
    }
}

fn parse_class_heritage(class: &Class<'_>) -> Vec<ParsedNamedType> {
    let Some(super_class) = class.super_class.as_ref() else {
        return Vec::new();
    };
    let Some((name, span)) = super::types::flatten_heritage_expression(super_class) else {
        return vec![ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: crate::EXPRESSION_HERITAGE_BASE.to_string(),
            span: Some(text_span_from_oxc_span(super_class.span())),
            // `extends f()<A, B>` instantiates the base constructors.
            type_arguments: class
                .super_type_arguments
                .as_deref()
                .and_then(parse_type_arguments)
                .unwrap_or_default(),
        }];
    };

    let (type_arguments, type_argument_spans) =
        match class.super_type_arguments.as_deref().and_then(parse_type_arguments) {
            Some(arguments) => (arguments, Vec::new()),
            // The arguments are written on the `@augments` tag, where tsc
            // reports what is wrong with them.
            None => match super::jsdoc::extends_type_arguments_at(class.span.start) {
                Some((arguments, Some(tag_span))) => {
                    let spans = vec![tag_span; arguments.len()];
                    (arguments, spans)
                }
                Some((arguments, None)) => (arguments, Vec::new()),
                None => (Vec::new(), Vec::new()),
            },
        };

    vec![ParsedNamedType {
        type_argument_spans,
        name,
        span: Some(span),
        type_arguments,
    }]
}

/// Collapses separate `get`/`set` accessor members that share a name into a
/// single accessor member (a getter and its matching setter become one).
fn merge_class_accessors(members: Vec<ParsedClassMember>) -> Vec<ParsedClassMember> {
    let mut merged: Vec<ParsedClassMember> = Vec::with_capacity(members.len());

    for member in members {
        let ParsedClassMember::Accessor(accessor) = member else {
            merged.push(member);
            continue;
        };

        let existing = merged.iter_mut().find_map(|candidate| match candidate {
            ParsedClassMember::Accessor(existing)
                if existing.name == accessor.name && existing.is_static == accessor.is_static =>
            {
                Some(existing)
            }
            _ => None,
        });

        match existing {
            Some(existing) => {
                existing.declarations.extend(accessor.declarations);
                if accessor.has_getter {
                    existing.has_getter = true;
                    existing.getter_return_type = accessor.getter_return_type;
                }
                if accessor.has_setter {
                    existing.has_setter = true;
                    existing.setter_param_type = accessor.setter_param_type;
                }
            }
            None => merged.push(ParsedClassMember::Accessor(accessor)),
        }
    }

    merged
}

/// A computed key's member name: the property it names or, for a key no
/// property name can be read off (`[1 << 6]`) when `dynamic`, its written
/// text in brackets.
fn computed_member_name(key: &PropertyKey<'_>, dynamic: bool) -> Option<String> {
    super::types::computed_key_name(key)
        .or_else(|| dynamic.then(|| format!("[{}]", super::spans::source_text_of(key.span()))))
}

/// Whether `member`'s computed key names no property (see
/// [`ParsedClassDeclaration::dynamic_members`]).
fn has_dynamic_name(member: &ClassElement<'_>) -> bool {
    let (key, computed) = match member {
        ClassElement::MethodDefinition(member) => (&member.key, member.computed),
        ClassElement::PropertyDefinition(member) => (&member.key, member.computed),
        ClassElement::AccessorProperty(member) => (&member.key, member.computed),
        _ => return false,
    };
    computed && key.as_expression().is_some() && super::types::computed_key_name(key).is_none()
}

/// A class member, preceded by the constructor overloads its JSDoc
/// `@overload` tags write, as a TypeScript file writes them.
fn parse_class_members(member: &ClassElement<'_>) -> Vec<ParsedClassMember> {
    let Some(parsed) = parse_class_member(member, false) else {
        return Vec::new();
    };
    let (ClassElement::MethodDefinition(method), ParsedClassMember::Constructor(constructor)) = (member, &parsed) else {
        return vec![parsed];
    };
    let mut members: Vec<ParsedClassMember> = super::jsdoc::overloads_at(method.value.span.start)
        .into_iter()
        .filter_map(|(signature, span)| match signature {
            crate::ParsedType::Function(signature) => Some(ParsedClassMember::Constructor(ParsedClassConstructor {
                body_reads: Vec::new(),
                accessibility: constructor.accessibility,
                parameters: super::functions::jsdoc_overload_parameters(&signature),
                body: Vec::new(),
                span: Some(span),
            })),
            _ => None,
        })
        .collect();
    members.push(parsed);
    members
}

fn parse_class_member(member: &ClassElement<'_>, dynamic: bool) -> Option<ParsedClassMember> {
    match member {
        ClassElement::MethodDefinition(method) => {
            // A computed constructor is not a constructor; a computed method or
            // accessor keeps the `[…]` name the type side uses
            // (`[Symbol.iterator]()`, `get [matcher]()`).
            if method.computed && method.kind == MethodDefinitionKind::Constructor {
                return None;
            }

            let mut parameters: Vec<_> = method
                .value
                .params
                .items
                .iter()
                .filter_map(parse_function_parameter)
                .collect();
            if let Some(rest) = method.value.params.rest.as_deref() {
                if let Some(rest_parameter) = parse_rest_function_parameter(rest) {
                    parameters.push(rest_parameter);
                }
            }
            let body = method
                .value
                .body
                .as_ref()
                .map(|body| {
                    if method.r#static || method.kind == MethodDefinitionKind::Constructor {
                        parse_statement_list_as_function_body(&body.statements)
                    } else {
                        super::types::with_member_body(|| parse_statement_list_as_function_body(&body.statements))
                    }
                })
                .unwrap_or_default();
            let body_reads = method
                .value
                .body
                .as_ref()
                .map(|body| super::reads::collect_function_body_reads(body))
                .unwrap_or_default();

            match method.kind {
                MethodDefinitionKind::Constructor => {
                    Some(ParsedClassMember::Constructor(ParsedClassConstructor {
                        accessibility: restricted_accessibility(method.accessibility)
                            .or_else(|| super::jsdoc::member_accessibility_at(method.span.start)),
                        parameters,
                        body,
                        body_reads,
                        span: Some(text_span_from_oxc_span(method.span)),
                    }))
                }
                MethodDefinitionKind::Method => {
                    let (name, name_span) = if method.computed {
                        (computed_member_name(&method.key, dynamic)?, method.key.span())
                    } else {
                        match &method.key {
                            PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
                            PropertyKey::PrivateIdentifier(key) => (
                                super::private_names::member_key(key.name.as_str(), method.r#static),
                                key.span,
                            ),
                            // `1: T` and `"a": T` name members as their computed forms do.
                            key => (super::types::computed_key_name(key)?, key.span()),
                        }
                    };
                    let (return_type, return_type_span) =
                        super::jsdoc::annotated_or_jsdoc_return_type(
                            method.value.return_type.as_deref(),
                            method.value.span.start,
                        );
                    let return_type = match method.value.return_type.as_deref() {
                        Some(annotation) => {
                            super::types::parse_member_return_annotation(annotation)
                        }
                        _ => return_type,
                    };

                    Some(ParsedClassMember::Method(ParsedClassMethod {
                        span: Some(text_span_from_oxc_span(method.span)),
                        name,
                        name_span: Some(text_span_from_oxc_span(name_span)),
                        computed_name_span: method
                            .computed
                            .then(|| super::spans::bracketed_key_span(name_span))
                            .flatten(),
                        is_static: method.r#static,
                        is_override: method.r#override
                            || super::jsdoc::member_has_modifier(
                                method.span.start,
                                super::jsdoc::JsDocModifier::Override,
                            ),
                        is_abstract: matches!(
                            method.r#type,
                            MethodDefinitionType::TSAbstractMethodDefinition
                        ),
                        type_parameters: super::jsdoc::written_or_jsdoc_type_parameters(
                            method.value.type_parameters.as_deref(),
                            method.value.span.start,
                        ),
                        parameters,
                        this_parameter_type: method
                            .value
                            .this_param
                            .as_ref()
                            .and_then(|this_param| this_param.type_annotation.as_ref())
                            .and_then(|annotation| super::types::parse_this_parameter_annotation(annotation))
                            .or_else(|| super::jsdoc::this_type_at(method.value.span.start)),
                        this_parameter_polymorphic: method
                            .value
                            .this_param
                            .as_ref()
                            .and_then(|this_param| this_param.type_annotation.as_ref())
                            .is_some_and(|annotation| super::types::is_polymorphic_this_annotation(annotation)),
                        return_type,
                        return_type_span,
                        body,
                        has_body: method.value.body.is_some(),
                        is_generator: method.value.generator,
                        is_async: method.value.r#async,
                        body_reads,
                    }))
                }
                MethodDefinitionKind::Get | MethodDefinitionKind::Set => {
                    let (name, name_span) = if method.computed {
                        (computed_member_name(&method.key, dynamic)?, method.key.span())
                    } else {
                        match &method.key {
                            PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
                            PropertyKey::PrivateIdentifier(key) => (
                                super::private_names::member_key(key.name.as_str(), method.r#static),
                                key.span,
                            ),
                            // `1: T` and `"a": T` name members as their computed forms do.
                            key => (super::types::computed_key_name(key)?, key.span()),
                        }
                    };

                    let is_getter = matches!(method.kind, MethodDefinitionKind::Get);
                    let getter_return_type = if is_getter {
                        super::jsdoc::annotated_or_jsdoc_return_type(
                            method.value.return_type.as_deref(),
                            method.value.span.start,
                        )
                        .0
                    } else {
                        None
                    };
                    let setter_param_type = if is_getter {
                        None
                    } else {
                        method.value.params.items.first().and_then(|param| {
                            param
                                .type_annotation
                                .as_ref()
                                .and_then(|annotation| parse_type_annotation(annotation))
                                .or_else(|| {
                                    super::jsdoc::parameter_at(param.span.start)
                                        .and_then(|jsdoc| jsdoc.ty.map(|(ty, _)| ty))
                                })
                        })
                    };

                    Some(ParsedClassMember::Accessor(ParsedClassAccessor {
                        span: Some(text_span_from_oxc_span(method.span)),
                        name,
                        name_span: Some(text_span_from_oxc_span(name_span)),
                        computed_name_span: method
                            .computed
                            .then(|| super::spans::bracketed_key_span(name_span))
                            .flatten(),
                        is_static: method.r#static,
                        is_override: method.r#override,
                        is_abstract: matches!(
                            method.r#type,
                            MethodDefinitionType::TSAbstractMethodDefinition
                        ),
                        getter_return_type,
                        setter_param_type,
                        has_getter: is_getter,
                        has_setter: !is_getter,
                        declarations: vec![crate::ParsedAccessorDeclaration {
                            is_getter,
                            parameters,
                            return_type_span: method.value.return_type.as_ref().map(
                                |annotation| text_span_from_oxc_span(annotation.type_annotation.span()),
                            ),
                            body,
                            body_reads,
                            has_body: method.value.body.is_some(),
                        }],
                    }))
                }
            }
        }
        ClassElement::PropertyDefinition(property) => {
            let has_literal_name = !property.computed && is_literal_property_key(&property.key);
            let (name, name_span) = if property.computed {
                (computed_member_name(&property.key, dynamic)?, property.key.span())
            } else {
                match &property.key {
                    PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
                    PropertyKey::PrivateIdentifier(key) => (
                        super::private_names::member_key(key.name.as_str(), property.r#static),
                        key.span,
                    ),
                    // `1: T` and `"a": T` name members as their computed forms do.
                    key => (super::types::computed_key_name(key)?, key.span()),
                }
            };

            let declared_type = property
                .type_annotation
                .as_ref()
                .and_then(|annotation| super::types::parse_property_annotation(annotation))
                .or_else(|| super::jsdoc::declared_type_at(property.span.start).map(|(ty, _)| ty));
            let initialization_type = property
                .type_annotation
                .as_ref()
                .and_then(|annotation| this_as_object(&annotation.type_annotation));
            let (initializer, initializer_span) = match property.value.as_ref() {
                Some(value) => {
                    let (expression, span) = if property.r#static {
                        parse_expression(value)
                    } else {
                        super::types::with_member_body(|| parse_expression(value))
                    };
                    (Some(expression), Some(text_span_from_oxc_span(span)))
                }
                None => (None, None),
            };

            Some(ParsedClassMember::Property(ParsedClassProperty {
                is_auto_accessor: false,
                span: Some(text_span_from_oxc_span(property.span)),
                name,
                name_span: Some(text_span_from_oxc_span(name_span)),
                computed_name_span: property
                    .computed
                    .then(|| super::spans::bracketed_key_span(name_span))
                    .flatten(),
                has_literal_name,
                is_static: property.r#static,
                is_override: property.r#override,
                is_abstract: matches!(
                    property.r#type,
                    PropertyDefinitionType::TSAbstractPropertyDefinition
                ),
                is_declare: property.declare,
                has_definite_assertion: property.definite,
                optional: property.optional,
                readonly: property.readonly
                    || super::jsdoc::member_has_modifier(
                        property.span.start,
                        super::jsdoc::JsDocModifier::Readonly,
                    ),
                declared_type,
                initialization_type,
                initializer,
                initializer_span,
                this_assignments: None,
            }))
        }
        ClassElement::StaticBlock(block) => {
            Some(ParsedClassMember::StaticBlock(crate::ParsedClassStaticBlock {
                span: Some(text_span_from_oxc_span(block.span)),
                body: parse_statement_list_as_function_body(&block.body),
                body_reads: super::reads::collect_statement_reads(&block.body),
            }))
        }
        // An auto-accessor (`accessor x: T = v`) is a get/set pair over a private
        // slot: for typing it is the property it looks like. Dropping it made
        // every use of the member a missing property.
        ClassElement::AccessorProperty(property) => {
            let has_literal_name = !property.computed && is_literal_property_key(&property.key);
            let (name, name_span) = if property.computed {
                (computed_member_name(&property.key, dynamic)?, property.key.span())
            } else {
                match &property.key {
                    PropertyKey::StaticIdentifier(key) => (key.name.to_string(), key.span),
                    PropertyKey::PrivateIdentifier(key) => (
                        super::private_names::member_key(key.name.as_str(), property.r#static),
                        key.span,
                    ),
                    key => (super::types::computed_key_name(key)?, key.span()),
                }
            };
            let declared_type = property
                .type_annotation
                .as_ref()
                .and_then(|annotation| parse_type_annotation(annotation));
            let initialization_type = property
                .type_annotation
                .as_ref()
                .and_then(|annotation| this_as_object(&annotation.type_annotation));
            let (initializer, initializer_span) = match property.value.as_ref() {
                Some(value) => {
                    let (expression, span) = if property.r#static {
                        parse_expression(value)
                    } else {
                        super::types::with_member_body(|| parse_expression(value))
                    };
                    (Some(expression), Some(text_span_from_oxc_span(span)))
                }
                None => (None, None),
            };
            Some(ParsedClassMember::Property(ParsedClassProperty {
                is_auto_accessor: true,
                span: Some(text_span_from_oxc_span(property.span)),
                name,
                name_span: Some(text_span_from_oxc_span(name_span)),
                computed_name_span: property
                    .computed
                    .then(|| super::spans::bracketed_key_span(name_span))
                    .flatten(),
                has_literal_name,
                is_static: property.r#static,
                is_override: property.r#override,
                is_abstract: matches!(
                    property.r#type,
                    oxc_ast::ast::AccessorPropertyType::TSAbstractAccessorProperty
                ),
                is_declare: false,
                has_definite_assertion: property.definite,
                optional: false,
                readonly: false,
                declared_type,
                initialization_type,
                initializer,
                initializer_span,
                this_assignments: None,
            }))
        }
        // Index signatures are not part of this slice.
        ClassElement::TSIndexSignature(_) => None,
    }
}

/// A property annotation with polymorphic `this` at its top — alone, as a
/// union or intersection member, or indexed — with that `this` read as
/// `object`, for [`ParsedClassProperty::initialization_type`]. `None` when no
/// such `this` is written.
fn this_as_object(annotation: &oxc_ast::ast::TSType<'_>) -> Option<crate::ParsedType> {
    use oxc_ast::ast::TSType;
    fn is_this(ty: &TSType<'_>) -> bool {
        match ty {
            TSType::TSThisType(_) => true,
            TSType::TSParenthesizedType(parenthesized) => is_this(&parenthesized.type_annotation),
            TSType::TSIndexedAccessType(indexed) => is_this(&indexed.object_type),
            _ => false,
        }
    }
    fn members(types: &[TSType<'_>]) -> Option<Vec<crate::ParsedType>> {
        let mut found = false;
        let mut parsed = Vec::with_capacity(types.len());
        for ty in types {
            match this_as_object(ty) {
                Some(lowered) => {
                    found = true;
                    parsed.push(lowered);
                }
                None => parsed.push(super::types::parse_type(ty)?),
            }
        }
        found.then_some(parsed)
    }
    match annotation {
        _ if is_this(annotation) => Some(crate::ParsedType::Object(std::sync::Arc::new(crate::ParsedObjectType {
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
            number_index_span: None,
        }))),
        TSType::TSParenthesizedType(parenthesized) => this_as_object(&parenthesized.type_annotation),
        TSType::TSUnionType(union) => {
            members(&union.types).map(|types| crate::ParsedType::Union(std::sync::Arc::new(types)))
        }
        TSType::TSIntersectionType(intersection) => members(&intersection.types)
            .map(|types| crate::ParsedType::Intersection(std::sync::Arc::new(types))),
        _ => None,
    }
}

/// tsc's `bindThisPropertyAssignment` for a JavaScript class: `this.x = v` in
/// a constructor, method, accessor, static block or property initializer — or
/// an arrow function within one — declares `x` on the instance (on the class,
/// in a static one) unless the class declares it. An assignment whose value
/// reads the same member declares it without typing it
/// (`containsSameNamedThisProperty`).
fn javascript_this_members(
    class: &Class<'_>,
    declared: &[ParsedClassMember],
    restricted: &mut Vec<ParsedRestrictedMember>,
) -> Vec<ParsedClassMember> {
    use oxc_ast::ast::{AssignmentTarget, Expression};
    use oxc_ast_visit::{Visit, walk};

    struct Found {
        name: String,
        name_span: oxc_span::Span,
        value: Option<crate::ParsedExpression>,
        is_static: bool,
        in_constructor: bool,
        /// Where the assignment starts, which its JSDoc modifiers are keyed by.
        start: u32,
    }
    struct Collector {
        found: Vec<Found>,
        is_static: bool,
        in_constructor: bool,
    }
    impl<'a> Visit<'a> for Collector {
        fn visit_assignment_expression(&mut self, assignment: &oxc_ast::ast::AssignmentExpression<'a>) {
            if assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign
                && let AssignmentTarget::StaticMemberExpression(member) = &assignment.left
                && matches!(member.object, Expression::ThisExpression(_))
            {
                self.found.push(Found {
                    name: member.property.name.to_string(),
                    name_span: member.property.span,
                    value: (!reads_same_this_member(&assignment.right, &member.property.name))
                        .then(|| parse_expression(&assignment.right).0),
                    is_static: self.is_static,
                    in_constructor: self.in_constructor,
                    start: assignment.span.start,
                });
            }
            walk::walk_assignment_expression(self, assignment);
        }
        // A nested function or class has a `this` of its own.
        fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
        fn visit_class(&mut self, _: &Class<'a>) {}
    }

    let mut collector = Collector { found: Vec::new(), is_static: false, in_constructor: false };
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                let Some(body) = &method.value.body else { continue };
                collector.is_static = method.r#static;
                collector.in_constructor = method.kind == MethodDefinitionKind::Constructor;
                collector.visit_function_body(body);
            }
            ClassElement::StaticBlock(block) => {
                collector.is_static = true;
                collector.in_constructor = true;
                for statement in &block.body {
                    collector.visit_statement(statement);
                }
            }
            ClassElement::PropertyDefinition(property) => {
                let Some(value) = &property.value else { continue };
                collector.is_static = property.r#static;
                collector.in_constructor = false;
                collector.visit_expression(value);
            }
            // An `accessor` field is a property declaration to tsc's
            // `thisContainer`.
            ClassElement::AccessorProperty(property) => {
                let Some(value) = &property.value else { continue };
                collector.is_static = property.r#static;
                collector.in_constructor = false;
                collector.visit_expression(value);
            }
            _ => {}
        }
    }

    let is_declared = |name: &str, is_static: bool| {
        declared.iter().any(|member| match member {
            ParsedClassMember::Property(property) => property.name == name && property.is_static == is_static,
            ParsedClassMember::Method(method) => method.name == name && method.is_static == is_static,
            ParsedClassMember::Accessor(accessor) => accessor.name == name && accessor.is_static == is_static,
            _ => false,
        })
    };
    let mut members: Vec<ParsedClassMember> = Vec::new();
    let mut seen: Vec<(String, bool)> = Vec::new();
    for found in &collector.found {
        let key = (found.name.clone(), found.is_static);
        if seen.contains(&key) || is_declared(&found.name, found.is_static) {
            continue;
        }
        seen.push(key);
        let assignments: Vec<&Found> = collector
            .found
            .iter()
            .filter(|other| other.name == found.name && other.is_static == found.is_static)
            .collect();
        let in_constructor = assignments.iter().any(|assignment| assignment.in_constructor);
        let values = assignments
            .iter()
            .filter(|assignment| !in_constructor || assignment.in_constructor)
            .filter_map(|assignment| assignment.value.clone())
            .collect();
        let span = Some(text_span_from_oxc_span(found.name_span));
        members.push(ParsedClassMember::Property(ParsedClassProperty {
            is_auto_accessor: false,
            span,
            name: found.name.clone(),
            name_span: span,
            computed_name_span: None,
            has_literal_name: false,
            is_static: found.is_static,
            is_override: false,
            is_abstract: false,
            is_declare: false,
            has_definite_assertion: false,
            optional: false,
            readonly: assignments.iter().any(|assignment| {
                super::jsdoc::member_has_modifier(assignment.start, super::jsdoc::JsDocModifier::Readonly)
            }),
            declared_type: None,
            initialization_type: None,
            initializer: None,
            initializer_span: None,
            this_assignments: Some(crate::ParsedThisAssignments {
                values,
                in_constructor,
                constructor_parameters: Vec::new(),
            }),
        }));
        if let Some(accessibility) =
            assignments.iter().find_map(|assignment| super::jsdoc::member_accessibility_at(assignment.start))
        {
            restricted.push(ParsedRestrictedMember {
                name: found.name.clone(),
                is_static: found.is_static,
                accessibility,
                accessor_side: None,
            });
        }
    }
    members
}

/// tsc's `containsSameNamedThisProperty`: whether `this.<name>` is one of the
/// value's direct operands (`this.x = this.x || {}`).
fn reads_same_this_member(value: &oxc_ast::ast::Expression<'_>, name: &str) -> bool {
    use oxc_ast::ast::Expression;
    let is_same = |operand: &Expression<'_>| {
        matches!(operand, Expression::StaticMemberExpression(member)
            if matches!(member.object, Expression::ThisExpression(_)) && member.property.name == name)
    };
    match value {
        Expression::LogicalExpression(logical) => is_same(&logical.left) || is_same(&logical.right),
        Expression::BinaryExpression(binary) => is_same(&binary.left) || is_same(&binary.right),
        Expression::ConditionalExpression(conditional) => {
            is_same(&conditional.test) || is_same(&conditional.consequent) || is_same(&conditional.alternate)
        }
        Expression::CallExpression(call) => {
            is_same(&call.callee)
                || call.arguments.iter().any(|argument| argument.as_expression().is_some_and(is_same))
        }
        other => is_same(other),
    }
}

fn is_literal_property_key(key: &PropertyKey<'_>) -> bool {
    matches!(key, PropertyKey::StringLiteral(_) | PropertyKey::NumericLiteral(_))
}
