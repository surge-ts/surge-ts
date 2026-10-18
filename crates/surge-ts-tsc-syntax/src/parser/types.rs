//! Types, signatures, parameters, property names, modifiers and decorators:
//! parser.go from `parseType` through `nextTokenIsOpenBrace`, in the Go order.

#![allow(unused_imports)]

use crate::ast::{self, Node, NodeId, NodeList, OperatorPrecedence};
use crate::flags::{NodeFlags, TokenFlags};
use crate::kind::Kind;
use crate::messages as diagnostics;
use crate::scanner::{self, token_is_identifier_or_keyword, token_to_string};
use crate::Message;

use super::{PC, ParseFlags, Parser, Tristate};

fn token_is_identifier_or_keyword_or_greater_than(token: Kind) -> bool {
    token == Kind::GreaterThanToken || token_is_identifier_or_keyword(token)
}

impl<'a> Parser<'a> {
    // TYPES

    pub(crate) fn parse_type(&mut self) -> NodeId {
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::TypeExcludesFlags, false);
        let type_node;
        if self.is_start_of_function_type_or_constructor_type() {
            type_node = self.parse_function_or_constructor_type();
        } else {
            let pos = self.node_pos();
            let mut check_type = self.parse_union_type_or_higher();
            if !self.in_disallow_conditional_types_context()
                && !self.has_preceding_line_break()
                && self.parse_optional(Kind::ExtendsKeyword)
            {
                // The type following 'extends' is not permitted to be another conditional type
                let extends_type = self.do_in_context(NodeFlags::DisallowConditionalTypesContext, true, Self::parse_type);
                self.parse_expected(Kind::QuestionToken);
                let true_type = self.do_in_context(NodeFlags::DisallowConditionalTypesContext, false, Self::parse_type);
                self.parse_expected(Kind::ColonToken);
                let false_type = self.do_in_context(NodeFlags::DisallowConditionalTypesContext, false, Self::parse_type);
                let conditional_type = Node::new(Kind::ConditionalType)
                    .child(check_type)
                    .child(extends_type)
                    .child(true_type)
                    .child(false_type);
                check_type = self.finish_node(conditional_type, pos);
            }
            type_node = check_type;
        }
        self.context_flags = save_context_flags;
        type_node
    }

    pub(crate) fn parse_union_type_or_higher(&mut self) -> NodeId {
        self.parse_union_or_intersection_type(Kind::BarToken, Self::parse_intersection_type_or_higher)
    }

    pub(crate) fn parse_intersection_type_or_higher(&mut self) -> NodeId {
        self.parse_union_or_intersection_type(Kind::AmpersandToken, Self::parse_type_operator_or_higher)
    }

    pub(crate) fn parse_union_or_intersection_type(
        &mut self,
        operator: Kind,
        parse_constituent_type: fn(&mut Self) -> NodeId,
    ) -> NodeId {
        let pos = self.node_pos();
        let is_union_type = operator == Kind::BarToken;
        let has_leading_operator = self.parse_optional(operator);
        let mut type_node = if has_leading_operator {
            self.parse_function_or_constructor_type_to_error(is_union_type, parse_constituent_type)
        } else {
            parse_constituent_type(self)
        };
        if self.token == operator || has_leading_operator {
            let mut types = vec![type_node];
            while self.parse_optional(operator) {
                types.push(self.parse_function_or_constructor_type_to_error(is_union_type, parse_constituent_type));
            }
            let list = NodeList::new(pos, self.node_pos(), types);
            let node = self.create_union_or_intersection_type_node(operator, list);
            type_node = self.finish_node(node, pos);
        }
        type_node
    }

    pub(crate) fn create_union_or_intersection_type_node(&self, operator: Kind, types: NodeList) -> Node {
        match operator {
            Kind::BarToken => Node::new(Kind::UnionType).list(types),
            Kind::AmpersandToken => Node::new(Kind::IntersectionType).list(types),
            _ => panic!("Unhandled case in createUnionOrIntersectionType"),
        }
    }

    pub(crate) fn parse_type_operator_or_higher(&mut self) -> NodeId {
        let operator = self.token;
        match operator {
            Kind::KeyOfKeyword | Kind::UniqueKeyword | Kind::ReadonlyKeyword => {
                return self.parse_type_operator(operator);
            }
            Kind::InferKeyword => return self.parse_infer_type(),
            _ => {}
        }
        self.do_in_context(NodeFlags::DisallowConditionalTypesContext, false, Self::parse_postfix_type_or_higher)
    }

    pub(crate) fn parse_type_operator(&mut self, operator: Kind) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(operator);
        let type_node = self.parse_type_operator_or_higher();
        self.finish_node(Node::new(Kind::TypeOperator).op(operator).ty(type_node), pos)
    }

    pub(crate) fn parse_infer_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::InferKeyword);
        let type_parameter = self.parse_type_parameter_of_infer_type();
        self.finish_node(Node::new(Kind::InferType).child(type_parameter), pos)
    }

    pub(crate) fn parse_type_parameter_of_infer_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        let name = self.parse_identifier();
        let constraint = self.try_parse_constraint_of_infer_type();
        let node = Node::new(Kind::TypeParameter)
            .modifiers(None)
            .name(name)
            .child(constraint)
            .expression(None::<NodeId>)
            .child(None::<NodeId>);
        self.finish_node(node, pos)
    }

    pub(crate) fn try_parse_constraint_of_infer_type(&mut self) -> Option<NodeId> {
        let state = self.mark();
        if self.parse_optional(Kind::ExtendsKeyword) {
            let constraint = self.do_in_context(NodeFlags::DisallowConditionalTypesContext, true, Self::parse_type);
            if self.in_disallow_conditional_types_context() || self.token != Kind::QuestionToken {
                return Some(constraint);
            }
        }
        self.rewind(state);
        None
    }

    pub(crate) fn parse_postfix_type_or_higher(&mut self) -> NodeId {
        let pos = self.node_pos();
        let mut type_node = self.parse_non_array_type();
        while !self.has_preceding_line_break() {
            match self.token {
                Kind::ExclamationToken => {
                    self.next_token();
                    type_node = self.finish_node(Node::new(Kind::JSDocNonNullableType).ty(type_node), pos);
                }
                Kind::QuestionToken => {
                    // If next token is start of a type we have a conditional type
                    if self.look_ahead(Self::next_is_start_of_type) {
                        return type_node;
                    }
                    self.next_token();
                    type_node = self.finish_node(Node::new(Kind::JSDocNullableType).ty(type_node), pos);
                }
                Kind::OpenBracketToken => {
                    self.parse_expected(Kind::OpenBracketToken);
                    if self.is_start_of_type(false) {
                        let index_type = self.parse_type();
                        self.parse_expected(Kind::CloseBracketToken);
                        let node = Node::new(Kind::IndexedAccessType).child(type_node).child(index_type);
                        type_node = self.finish_node(node, pos);
                    } else {
                        self.parse_expected(Kind::CloseBracketToken);
                        type_node = self.finish_node(Node::new(Kind::ArrayType).ty(type_node), pos);
                    }
                }
                _ => return type_node,
            }
        }
        type_node
    }

    pub(crate) fn next_is_start_of_type(&mut self) -> bool {
        self.next_token();
        self.is_start_of_type(false)
    }

    pub(crate) fn parse_non_array_type(&mut self) -> NodeId {
        match self.token {
            Kind::AnyKeyword
            | Kind::UnknownKeyword
            | Kind::StringKeyword
            | Kind::NumberKeyword
            | Kind::BigIntKeyword
            | Kind::SymbolKeyword
            | Kind::BooleanKeyword
            | Kind::UndefinedKeyword
            | Kind::NeverKeyword
            | Kind::ObjectKeyword => {
                let state = self.mark();
                let keyword_type_node = self.parse_keyword_type_node();
                // If these are followed by a dot then parse these out as a dotted type reference instead
                if self.token != Kind::DotToken {
                    return keyword_type_node;
                }
                self.rewind(state);
                self.parse_type_reference()
            }
            Kind::AsteriskEqualsToken | Kind::AsteriskToken => {
                if self.token == Kind::AsteriskEqualsToken {
                    // If there is '*=', treat it as * followed by postfix =
                    // port: Go does not store the rescanned token in p.token either.
                    self.scanner.re_scan_asterisk_equals_token();
                }
                self.parse_jsdoc_all_type()
            }
            Kind::QuestionQuestionToken | Kind::QuestionToken => {
                if self.token == Kind::QuestionQuestionToken {
                    // If there is '??', treat it as prefix-'?' in JSDoc type.
                    // port: Go does not store the rescanned token in p.token either.
                    self.scanner.re_scan_question_token();
                }
                self.parse_jsdoc_nullable_type()
            }
            Kind::ExclamationToken => self.parse_jsdoc_non_nullable_type(),
            Kind::NoSubstitutionTemplateLiteral
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::NullKeyword => self.parse_literal_type_node(false),
            Kind::MinusToken => {
                if self.look_ahead(Self::next_token_is_numeric_or_big_int_literal) {
                    return self.parse_literal_type_node(true);
                }
                self.parse_type_reference()
            }
            Kind::VoidKeyword => self.parse_keyword_type_node(),
            Kind::ThisKeyword => {
                let this_keyword = self.parse_this_type_node();
                if self.token == Kind::IsKeyword && !self.has_preceding_line_break() {
                    return self.parse_this_type_predicate(this_keyword);
                }
                this_keyword
            }
            Kind::TypeOfKeyword => {
                if self.look_ahead(Self::next_is_start_of_type_of_import_type) {
                    return self.parse_import_type();
                }
                self.parse_type_query()
            }
            Kind::OpenBraceToken => {
                if self.look_ahead(Self::next_is_start_of_mapped_type) {
                    return self.parse_mapped_type();
                }
                self.parse_type_literal()
            }
            Kind::OpenBracketToken => self.parse_tuple_type(),
            Kind::OpenParenToken => self.parse_parenthesized_type(),
            Kind::ImportKeyword => self.parse_import_type(),
            Kind::AssertsKeyword => {
                if self.look_ahead(Self::next_token_is_identifier_or_keyword_on_same_line) {
                    return self.parse_asserts_type_predicate();
                }
                self.parse_type_reference()
            }
            Kind::TemplateHead => self.parse_template_type(),
            _ => self.parse_type_reference(),
        }
    }

    pub(crate) fn parse_keyword_type_node(&mut self) -> NodeId {
        let pos = self.node_pos();
        let result = Node::new(self.token);
        self.next_token();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_this_type_node(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        self.finish_node(Node::new(Kind::ThisType), pos)
    }

    pub(crate) fn parse_this_type_predicate(&mut self, lhs: NodeId) -> NodeId {
        self.next_token();
        let type_node = self.parse_type();
        let pos = self.node(lhs).pos;
        let node = Node::new(Kind::TypePredicate).child(None::<NodeId>).child(lhs).ty(type_node);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsdoc_all_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        self.finish_node(Node::new(Kind::JSDocAllType), pos)
    }

    pub(crate) fn parse_jsdoc_non_nullable_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let type_node = self.parse_type_operator_or_higher();
        self.finish_node(Node::new(Kind::JSDocNonNullableType).ty(type_node), pos)
    }

    pub(crate) fn parse_jsdoc_nullable_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        // skip the ?
        self.next_token();
        // typescript@6's `parseJSDocUnknownOrNullableType` reads a lone `?`
        // before these tokens as a `JSDocUnknownType`; tsgo parses (and
        // reports) a missing type, whose error is kept.
        let unknown = matches!(
            self.token,
            Kind::CommaToken
                | Kind::CloseBraceToken
                | Kind::CloseParenToken
                | Kind::GreaterThanToken
                | Kind::EqualsToken
                | Kind::BarToken
        );
        let type_start = self.node_pos();
        let type_node = self.parse_type_operator_or_higher();
        if unknown && self.node_pos() == type_start {
            return self.finish_node(Node::new(Kind::JSDocUnknownType), pos);
        }
        self.finish_node(Node::new(Kind::JSDocNullableType).ty(type_node), pos)
    }

    // port: only jsdoc.go calls this; the scanner has no JSDoc mode, so the
    // SetSkipJSDocLeadingAsterisks toggles are dropped.
    #[allow(dead_code)]
    pub(crate) fn parse_jsdoc_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        let has_dot_dot_dot = self.parse_optional(Kind::DotDotDotToken);
        let mut t = self.parse_type_or_type_predicate();
        if has_dot_dot_dot {
            t = self.finish_node(Node::new(Kind::JSDocVariadicType).ty(t), pos);
        }
        if self.token == Kind::EqualsToken {
            self.next_token();
            return self.finish_node(Node::new(Kind::JSDocOptionalType).ty(t), pos);
        }
        t
    }

    pub(crate) fn parse_literal_type_node(&mut self, negative: bool) -> NodeId {
        let pos = self.node_pos();
        if negative {
            self.next_token();
        }
        let mut expression =
            if matches!(self.token, Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword) {
                self.parse_keyword_expression()
            } else {
                self.parse_literal_expression(false)
            };
        if negative {
            let node = Node::new(Kind::PrefixUnaryExpression).op(Kind::MinusToken).expression(expression);
            expression = self.finish_node(node, pos);
        }
        self.finish_node(Node::new(Kind::LiteralType).child(expression), pos)
    }

    pub(crate) fn parse_type_reference(&mut self) -> NodeId {
        let pos = self.node_pos();
        let type_name = self.parse_entity_name_of_type_reference();
        let type_arguments = self.parse_type_arguments_of_type_reference();
        self.finish_node(Node::new(Kind::TypeReference).name(type_name).type_arguments(type_arguments), pos)
    }

    pub(crate) fn parse_entity_name_of_type_reference(&mut self) -> NodeId {
        self.parse_entity_name(true, Some(diagnostics::Type_expected))
    }

    pub(crate) fn parse_entity_name(
        &mut self,
        allow_reserved_words: bool,
        diagnostic_message: Option<&'static Message>,
    ) -> NodeId {
        let pos = self.node_pos();
        let mut entity = if allow_reserved_words {
            self.parse_identifier_name_with_diagnostic(diagnostic_message)
        } else {
            self.parse_identifier_with_diagnostic(diagnostic_message, None)
        };
        while self.parse_optional(Kind::DotToken) {
            if self.token == Kind::LessThanToken {
                // The entity is part of a JSDoc-style generic. We will use the gap between `typeName` and
                // `typeArguments` to report it as a grammar error in the checker.
                break;
            }
            let right = self.parse_right_side_of_dot(allow_reserved_words, false, true);
            entity = self.finish_node(Node::new(Kind::QualifiedName).child(entity).child(right), pos);
        }
        entity
    }

    pub(crate) fn parse_type_arguments_of_type_reference(&mut self) -> Option<NodeList> {
        if !self.has_preceding_line_break() && self.re_scan_less_than_token() == Kind::LessThanToken {
            return self.parse_type_arguments();
        }
        None
    }

    pub(crate) fn parse_type_arguments(&mut self) -> Option<NodeList> {
        if self.token == Kind::LessThanToken {
            return Some(self.parse_bracketed_list(
                PC::TypeArguments,
                Self::parse_type,
                Kind::LessThanToken,
                Kind::GreaterThanToken,
            ));
        }
        None
    }

    pub(crate) fn next_is_start_of_type_of_import_type(&mut self) -> bool {
        self.next_token();
        self.token == Kind::ImportKeyword
    }

    pub(crate) fn parse_import_type(&mut self) -> NodeId {
        self.source_flags |= NodeFlags::PossiblyContainsDynamicImport;
        let pos = self.node_pos();
        let is_type_of = self.parse_optional(Kind::TypeOfKeyword);
        self.parse_expected(Kind::ImportKeyword);
        self.parse_expected(Kind::OpenParenToken);
        let type_node = self.parse_type();
        let mut attributes = None;
        if self.parse_optional(Kind::CommaToken) {
            // port: openBracePosition only feeds related information, which is dropped.
            self.parse_expected(Kind::OpenBraceToken);
            let current_token = self.token;
            if current_token == Kind::WithKeyword || current_token == Kind::AssertKeyword {
                if current_token == Kind::AssertKeyword {
                    self.parse_error_at_current_token(
                        diagnostics::Import_assertions_have_been_replaced_by_import_attributes_Use_with_instead_of_assert,
                        &[],
                    );
                }
                self.next_token();
            } else {
                self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(Kind::WithKeyword)]);
            }
            self.parse_expected(Kind::ColonToken);
            attributes = Some(self.parse_import_attributes(current_token, true));
            self.parse_optional(Kind::CommaToken);
            self.parse_expected(Kind::CloseBraceToken);
        }
        self.parse_expected(Kind::CloseParenToken);
        let mut qualifier = None;
        if self.parse_optional(Kind::DotToken) {
            qualifier = Some(self.parse_entity_name_of_type_reference());
        }
        let type_arguments = self.parse_type_arguments_of_type_reference();
        // slot: isTypeOf is kept in the type-only flag.
        let node = Node::new(Kind::ImportType)
            .type_only(is_type_of)
            .ty(type_node)
            .child(attributes)
            .child(qualifier)
            .type_arguments(type_arguments);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_import_attribute(&mut self) -> NodeId {
        let pos = self.node_pos();
        let name = if token_is_identifier_or_keyword(self.token) {
            Some(self.parse_identifier_name())
        } else if self.token == Kind::StringLiteral {
            Some(self.parse_literal_expression(false))
        } else {
            None
        };
        if name.is_some() {
            self.parse_expected(Kind::ColonToken);
        } else {
            self.parse_error_at_current_token(diagnostics::Identifier_or_string_literal_expected, &[]);
        }
        let value = self.parse_assignment_expression_or_higher();
        self.finish_node(Node::new(Kind::ImportAttribute).name(name).child(value), pos)
    }

    pub(crate) fn parse_import_attributes(&mut self, token: Kind, skip_keyword: bool) -> NodeId {
        let pos = self.node_pos();
        if !skip_keyword {
            self.parse_expected(token);
        }
        let elements;
        // port: openBracePosition only feeds related information, and multiLine
        // is not observable in diagnostics; both are dropped.
        if self.parse_expected(Kind::OpenBraceToken) {
            elements = self.parse_delimited_list(PC::ImportAttributes, Self::parse_import_attribute);
            self.parse_expected(Kind::CloseBraceToken);
        } else {
            elements = self.parse_empty_node_list();
        }
        self.finish_node(Node::new(Kind::ImportAttributes).op(token).list(elements), pos)
    }

    pub(crate) fn parse_type_query(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::TypeOfKeyword);
        let entity_name = self.parse_entity_name(true, None);
        // Make sure we perform ASI to prevent parsing the next line's type arguments as part of an instantiation expression
        let mut type_arguments = None;
        if !self.has_preceding_line_break() {
            type_arguments = self.parse_type_arguments();
        }
        self.finish_node(Node::new(Kind::TypeQuery).child(entity_name).type_arguments(type_arguments), pos)
    }

    pub(crate) fn next_is_start_of_mapped_type(&mut self) -> bool {
        self.next_token();
        if self.token == Kind::PlusToken || self.token == Kind::MinusToken {
            return self.next_token() == Kind::ReadonlyKeyword;
        }
        if self.token == Kind::ReadonlyKeyword {
            self.next_token();
        }
        self.token == Kind::OpenBracketToken && self.next_token_is_identifier() && self.next_token() == Kind::InKeyword
    }

    pub(crate) fn parse_mapped_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBraceToken);
        let mut readonly_token = None; // ReadonlyKeyword | PlusToken | MinusToken
        if matches!(self.token, Kind::ReadonlyKeyword | Kind::PlusToken | Kind::MinusToken) {
            let token = self.parse_token_node();
            readonly_token = Some(token);
            if self.kind(token) != Kind::ReadonlyKeyword {
                self.parse_expected(Kind::ReadonlyKeyword);
            }
        }
        self.parse_expected(Kind::OpenBracketToken);
        let type_parameter = self.parse_mapped_type_parameter();
        let mut name_type = None;
        if self.parse_optional(Kind::AsKeyword) {
            name_type = Some(self.parse_type());
        }
        self.parse_expected(Kind::CloseBracketToken);
        let mut question_token = None; // QuestionToken | PlusToken | MinusToken
        if matches!(self.token, Kind::QuestionToken | Kind::PlusToken | Kind::MinusToken) {
            let token = self.parse_token_node();
            question_token = Some(token);
            if self.kind(token) != Kind::QuestionToken {
                self.parse_expected(Kind::QuestionToken);
            }
        }
        let type_node = self.parse_type_annotation();
        self.parse_semicolon();
        let members = self.parse_list(PC::TypeMembers, Self::parse_type_member);
        self.parse_expected(Kind::CloseBraceToken);
        let node = Node::new(Kind::MappedType)
            .child(readonly_token)
            .child(type_parameter)
            .child(name_type)
            .question(question_token)
            .ty(type_node)
            .list(members);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_mapped_type_parameter(&mut self) -> NodeId {
        let pos = self.node_pos();
        let name = self.parse_identifier_name();
        self.parse_expected(Kind::InKeyword);
        let type_node = self.parse_type();
        let node = Node::new(Kind::TypeParameter)
            .modifiers(None)
            .name(name)
            .child(type_node)
            .expression(None::<NodeId>)
            .child(None::<NodeId>);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_type_member(&mut self) -> NodeId {
        if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
            return self.parse_signature_member(Kind::CallSignature);
        }
        if self.token == Kind::NewKeyword && self.look_ahead(Self::next_token_is_open_paren_or_less_than) {
            return self.parse_signature_member(Kind::ConstructSignature);
        }
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers();
        if self.parse_contextual_modifier(Kind::GetKeyword) {
            return self.parse_accessor_declaration(pos, modifiers, Kind::GetAccessor, ParseFlags::Type);
        }
        if self.parse_contextual_modifier(Kind::SetKeyword) {
            return self.parse_accessor_declaration(pos, modifiers, Kind::SetAccessor, ParseFlags::Type);
        }
        if self.is_index_signature() {
            return self.parse_index_signature_declaration(pos, modifiers);
        }
        self.parse_property_or_method_signature(pos, modifiers)
    }

    pub(crate) fn next_token_is_open_paren_or_less_than(&mut self) -> bool {
        self.next_token();
        self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken
    }

    pub(crate) fn parse_signature_member(&mut self, kind: Kind) -> NodeId {
        let pos = self.node_pos();
        if kind == Kind::ConstructSignature {
            self.parse_expected(Kind::NewKeyword);
        }
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters(ParseFlags::Type);
        let type_node = self.parse_return_type(Kind::ColonToken, true);
        self.parse_type_member_semicolon();
        let result = Node::new(kind).type_parameters(type_parameters).parameters(parameters).ty(type_node);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_type_parameters(&mut self) -> Option<NodeList> {
        if self.token == Kind::LessThanToken {
            return Some(self.parse_bracketed_list(
                PC::TypeParameters,
                Self::parse_type_parameter,
                Kind::LessThanToken,
                Kind::GreaterThanToken,
            ));
        }
        None
    }

    pub(crate) fn parse_type_parameter(&mut self) -> NodeId {
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers_ex(false, true, false);
        let name = self.parse_identifier();
        let mut constraint = None;
        let mut expression = None;
        if self.parse_optional(Kind::ExtendsKeyword) {
            // It's not uncommon for people to write improper constraints to a generic.  If the
            // user writes a constraint that is an expression and not an actual type, then parse
            // it out as an expression (so we can recover well), but report that a type is needed
            // instead.
            if self.is_start_of_type(false) || !self.is_start_of_expression() {
                constraint = Some(self.parse_type());
            } else {
                // It was not a type, and it looked like an expression.  Parse out an expression
                // here so we recover well.  Note: it is important that we call parseUnaryExpression
                // and not parseExpression here.  If the user has:
                //
                //      <T extends "">
                //
                // We do *not* want to consume the `>` as we're consuming the expression for "".
                expression = Some(self.parse_unary_expression_or_higher());
            }
        }
        let mut default_type = None;
        if self.parse_optional(Kind::EqualsToken) {
            default_type = Some(self.parse_type());
        }
        let result = Node::new(Kind::TypeParameter)
            .modifiers(modifiers)
            .name(name)
            .child(constraint)
            .expression(expression)
            .child(default_type);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_parameters(&mut self, flags: ParseFlags) -> NodeList {
        if self.parse_expected(Kind::OpenParenToken) {
            let parameters = self
                .parse_parameters_worker(flags, true)
                .expect("parameters parsed with ambiguity allowed never fail");
            self.parse_expected(Kind::CloseParenToken);
            return parameters;
        }
        self.create_missing_list()
    }

    pub(crate) fn parse_parameters_worker(&mut self, flags: ParseFlags, allow_ambiguity: bool) -> Option<NodeList> {
        let in_await_context = self.context_flags.has(NodeFlags::AwaitContext);
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::YieldContext, flags.has(ParseFlags::Yield));
        self.set_context_flags(NodeFlags::AwaitContext, flags.has(ParseFlags::Await));
        let parameters = self.parse_delimited_list_opt(PC::Parameters, |p| {
            let parameter = p.parse_parameter_ex(in_await_context, allow_ambiguity);
            if let Some(parameter) = parameter
                && !flags.has(ParseFlags::Type)
            {
                p.check_js_syntax(parameter);
            }
            parameter
        });
        self.context_flags = save_context_flags;
        parameters
    }

    pub(crate) fn parse_parameter(&mut self) -> NodeId {
        self.parse_parameter_ex(false, true)
            .expect("a parameter parsed with ambiguity allowed is never nil")
    }

    pub(crate) fn parse_parameter_ex(&mut self, in_outer_await_context: bool, allow_ambiguity: bool) -> Option<NodeId> {
        let pos = self.node_pos();
        // FormalParameter [Yield,Await]:
        //      BindingElement[?Yield,?Await]
        // Decorators are parsed in the outer [Await] context, the rest of the parameter is parsed in the function's [Await] context.
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::AwaitContext, in_outer_await_context);
        let modifiers = self.parse_modifiers_ex(true, false, false);
        self.context_flags = save_context_flags;
        if self.token == Kind::ThisKeyword {
            let name = self.create_identifier(true);
            let type_node = self.parse_type_annotation();
            if let Some(list) = &modifiers {
                let first = self.node(list.nodes[0]);
                let (start, end) = (first.pos, first.end);
                self.parse_error_at_range(
                    start,
                    end,
                    diagnostics::Neither_decorators_nor_modifiers_may_be_applied_to_this_parameters,
                    &[],
                );
            }
            let result = Node::new(Kind::Parameter)
                .modifiers(modifiers)
                .child(None::<NodeId>)
                .name(name)
                .question(None::<NodeId>)
                .ty(type_node)
                .initializer(None::<NodeId>);
            return Some(self.finish_node(result, pos));
        }
        let dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
        if !allow_ambiguity && !self.is_parameter_name_start() {
            return None;
        }
        let name = self.parse_name_of_parameter(modifiers.as_ref());
        let question_token = self.parse_optional_token(Kind::QuestionToken);
        let type_node = self.parse_type_annotation();
        let initializer = self.parse_initializer();
        let result = Node::new(Kind::Parameter)
            .modifiers(modifiers)
            .child(dot_dot_dot_token)
            .name(name)
            .question(question_token)
            .ty(type_node)
            .initializer(initializer);
        Some(self.finish_node(result, pos))
    }

    pub(crate) fn is_parameter_name_start(&self) -> bool {
        // Be permissive about await and yield by calling isBindingIdentifier instead of isIdentifier; disallowing
        // them during a speculative parse leads to many more follow-on errors than allowing the function to parse then later
        // complaining about the use of the keywords.
        self.is_binding_identifier() || self.token == Kind::OpenBracketToken || self.token == Kind::OpenBraceToken
    }

    pub(crate) fn parse_name_of_parameter(&mut self, modifiers: Option<&NodeList>) -> NodeId {
        // FormalParameter [Yield,Await]:
        //      BindingElement[?Yield,?Await]
        let name = self.parse_identifier_or_pattern_with_diagnostic(Some(
            diagnostics::Private_identifiers_cannot_be_used_as_parameters,
        ));
        let name_is_empty = self.node(name).end == self.node(name).pos;
        if name_is_empty && modifiers.is_none() && ast::is_modifier_kind(self.token) {
            // in cases like
            // 'use strict'
            // function foo(static)
            // isParameter('static') == true, because of isModifier('static')
            // however 'static' is not a legal identifier in a strict mode.
            // so result of this function will be Parameter (flags = 0, name = missing, type = undefined, initializer = undefined)
            // and current token will not change => parsing of the enclosing parameter list will last till the end of time (or OOM)
            // to avoid this we'll advance cursor to the next token.
            self.next_token();
        }
        name
    }

    pub(crate) fn parse_return_type(&mut self, return_token: Kind, is_type: bool) -> Option<NodeId> {
        if self.should_parse_return_type(return_token, is_type) {
            return Some(self.do_in_context(
                NodeFlags::DisallowConditionalTypesContext,
                false,
                Self::parse_type_or_type_predicate,
            ));
        }
        None
    }

    pub(crate) fn should_parse_return_type(&mut self, return_token: Kind, is_type: bool) -> bool {
        if return_token == Kind::EqualsGreaterThanToken {
            self.parse_expected(return_token);
            return true;
        } else if self.parse_optional(Kind::ColonToken) {
            return true;
        } else if is_type && self.token == Kind::EqualsGreaterThanToken {
            // This is easy to get backward, especially in type contexts, so parse the type anyway
            self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(Kind::ColonToken)]);
            self.next_token();
            return true;
        }
        false
    }

    pub(crate) fn parse_type_or_type_predicate(&mut self) -> NodeId {
        if self.is_identifier() {
            let state = self.mark();
            let pos = self.node_pos();
            let id = self.parse_identifier();
            if self.token == Kind::IsKeyword && !self.has_preceding_line_break() {
                self.next_token();
                let type_node = self.parse_type();
                let node = Node::new(Kind::TypePredicate).child(None::<NodeId>).child(id).ty(type_node);
                return self.finish_node(node, pos);
            }
            self.rewind(state);
        }
        self.parse_type()
    }

    pub(crate) fn parse_type_member_semicolon(&mut self) {
        // We allow type members to be separated by commas or (possibly ASI) semicolons.
        // First check if it was a comma.  If so, we're done with the member.
        if self.parse_optional(Kind::CommaToken) {
            return;
        }
        // Didn't have a comma.  We must have a (possible ASI) semicolon.
        self.parse_semicolon();
    }

    pub(crate) fn parse_accessor_declaration(
        &mut self,
        pos: usize,
        modifiers: Option<NodeList>,
        kind: Kind,
        flags: ParseFlags,
    ) -> NodeId {
        let name = self.parse_property_name();
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters(ParseFlags::None);
        let return_type = self.parse_return_type(Kind::ColonToken, false);
        let body = self.parse_function_block_or_semicolon(flags, None);
        // Keep track of `typeParameters` (for both) and `type` (for setters) if they were parsed those indicate grammar errors
        let node_kind = if kind == Kind::GetAccessor { Kind::GetAccessor } else { Kind::SetAccessor };
        let node = Node::new(node_kind)
            .modifiers(modifiers)
            .name(name)
            .type_parameters(type_parameters)
            .parameters(parameters)
            .ty(return_type)
            .body(body);
        let result = self.finish_node(node, pos);
        if !flags.has(ParseFlags::Type) {
            self.check_js_syntax(result);
        }
        result
    }

    pub(crate) fn parse_property_name(&mut self) -> NodeId {
        let save_has_await_identifier = self.statement_has_await_identifier;
        let prop = self.parse_property_name_worker(true);
        self.statement_has_await_identifier = save_has_await_identifier;
        prop
    }

    pub(crate) fn parse_property_name_worker(&mut self, allow_computed_property_names: bool) -> NodeId {
        if matches!(self.token, Kind::StringLiteral | Kind::NumericLiteral | Kind::BigIntLiteral) {
            return self.parse_literal_expression(true);
        }
        if allow_computed_property_names && self.token == Kind::OpenBracketToken {
            return self.parse_computed_property_name();
        }
        if self.token == Kind::PrivateIdentifier {
            return self.parse_private_identifier();
        }
        self.parse_identifier_name()
    }

    pub(crate) fn parse_computed_property_name(&mut self) -> NodeId {
        // PropertyName [Yield]:
        //      LiteralPropertyName
        //      ComputedPropertyName[?Yield]
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBracketToken);
        // We parse any expression (including a comma expression). But the grammar
        // says that only an assignment expression is allowed, so the grammar checker
        // will error if it sees a comma expression.
        let expression = self.parse_expression_allow_in();
        self.parse_expected(Kind::CloseBracketToken);
        self.finish_node(Node::new(Kind::ComputedPropertyName).expression(expression), pos)
    }

    pub(crate) fn parse_function_block_or_semicolon(
        &mut self,
        flags: ParseFlags,
        diagnostic_message: Option<&'static Message>,
    ) -> Option<NodeId> {
        if self.token != Kind::OpenBraceToken {
            if flags.has(ParseFlags::Type) {
                self.parse_type_member_semicolon();
                return None;
            }
            if self.can_parse_semicolon() {
                self.parse_semicolon();
                return None;
            }
        }
        Some(self.parse_function_block(flags, diagnostic_message))
    }

    pub(crate) fn parse_function_block(
        &mut self,
        flags: ParseFlags,
        diagnostic_message: Option<&'static Message>,
    ) -> NodeId {
        let save_context_flags = self.context_flags;
        let save_has_await_identifier = self.statement_has_await_identifier;
        self.set_context_flags(NodeFlags::YieldContext, flags.has(ParseFlags::Yield));
        self.set_context_flags(NodeFlags::AwaitContext, flags.has(ParseFlags::Await));
        // We may be in a [Decorator] context when parsing a function expression or
        // arrow function. The body of the function is not in [Decorator] context.
        self.set_context_flags(NodeFlags::DecoratorContext, false);
        let block = self.parse_block(flags.has(ParseFlags::IgnoreMissingOpenBrace), diagnostic_message);
        self.context_flags = save_context_flags;
        self.statement_has_await_identifier = save_has_await_identifier;
        block
    }

    pub(crate) fn is_index_signature(&mut self) -> bool {
        self.token == Kind::OpenBracketToken && self.look_ahead(Self::next_is_unambiguously_index_signature)
    }

    pub(crate) fn next_is_unambiguously_index_signature(&mut self) -> bool {
        // The only allowed sequence is:
        //
        //   [id:
        //
        // However, for error recovery, we also check the following cases:
        //
        //   [...
        //   [id,
        //   [id?,
        //   [id?:
        //   [id?]
        //   [public id
        //   [private id
        //   [protected id
        //   []
        //
        self.next_token();
        if self.token == Kind::DotDotDotToken || self.token == Kind::CloseBracketToken {
            return true;
        }
        if ast::is_modifier_kind(self.token) {
            self.next_token();
            if self.is_identifier() {
                return true;
            }
        } else if !self.is_identifier() {
            return false;
        } else {
            // Skip the identifier
            self.next_token();
        }
        // A colon signifies a well formed indexer
        // A comma should be a badly formed indexer because comma expressions are not allowed
        // in computed properties.
        if self.token == Kind::ColonToken || self.token == Kind::CommaToken {
            return true;
        }
        // Question mark could be an indexer with an optional property,
        // or it could be a conditional expression in a computed property.
        if self.token != Kind::QuestionToken {
            return false;
        }
        // If any of the following tokens are after the question mark, it cannot
        // be a conditional expression, so treat it as an indexer.
        self.next_token();
        self.token == Kind::ColonToken || self.token == Kind::CommaToken || self.token == Kind::CloseBracketToken
    }

    pub(crate) fn parse_index_signature_declaration(&mut self, pos: usize, modifiers: Option<NodeList>) -> NodeId {
        let parameters = self.parse_bracketed_list(
            PC::Parameters,
            Self::parse_parameter,
            Kind::OpenBracketToken,
            Kind::CloseBracketToken,
        );
        let type_node = self.parse_type_annotation();
        self.parse_type_member_semicolon();
        let node = Node::new(Kind::IndexSignature).modifiers(modifiers).parameters(parameters).ty(type_node);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_property_or_method_signature(&mut self, pos: usize, modifiers: Option<NodeList>) -> NodeId {
        let name = self.parse_property_name();
        let question_token = self.parse_optional_token(Kind::QuestionToken);
        let result;
        if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
            // Method signatures don't exist in expression contexts.  So they have neither
            // [Yield] nor [Await]
            let type_parameters = self.parse_type_parameters();
            let parameters = self.parse_parameters(ParseFlags::Type);
            let return_type = self.parse_return_type(Kind::ColonToken, true);
            result = Node::new(Kind::MethodSignature)
                .modifiers(modifiers)
                .name(name)
                .question(question_token)
                .type_parameters(type_parameters)
                .parameters(parameters)
                .ty(return_type);
        } else {
            let type_node = self.parse_type_annotation();
            // Although type literal properties cannot not have initializers, we attempt
            // to parse an initializer so we can report in the checker that an interface
            // property or type literal property cannot have an initializer.
            let mut initializer = None;
            if self.token == Kind::EqualsToken {
                initializer = self.parse_initializer();
            }
            result = Node::new(Kind::PropertySignature)
                .modifiers(modifiers)
                .name(name)
                .question(question_token)
                .ty(type_node)
                .initializer(initializer);
        }
        self.parse_type_member_semicolon();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_type_literal(&mut self) -> NodeId {
        let pos = self.node_pos();
        let members = self.parse_object_type_members();
        self.finish_node(Node::new(Kind::TypeLiteral).list(members), pos)
    }

    pub(crate) fn parse_object_type_members(&mut self) -> NodeList {
        if self.parse_expected(Kind::OpenBraceToken) {
            let members = self.parse_list(PC::TypeMembers, Self::parse_type_member);
            self.parse_expected(Kind::CloseBraceToken);
            return members;
        }
        self.create_missing_list()
    }

    pub(crate) fn parse_tuple_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        let elements = self.parse_bracketed_list(
            PC::TupleElementTypes,
            Self::parse_tuple_element_name_or_tuple_element_type,
            Kind::OpenBracketToken,
            Kind::CloseBracketToken,
        );
        self.finish_node(Node::new(Kind::TupleType).list(elements), pos)
    }

    pub(crate) fn parse_tuple_element_name_or_tuple_element_type(&mut self) -> NodeId {
        if self.look_ahead(Self::scan_start_of_named_tuple_element) {
            let pos = self.node_pos();
            let dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
            let name = self.parse_identifier_name();
            let question_token = self.parse_optional_token(Kind::QuestionToken);
            self.parse_expected(Kind::ColonToken);
            let type_node = self.parse_tuple_element_type();
            let node = Node::new(Kind::NamedTupleMember)
                .child(dot_dot_dot_token)
                .name(name)
                .question(question_token)
                .ty(type_node);
            return self.finish_node(node, pos);
        }
        self.parse_tuple_element_type()
    }

    pub(crate) fn scan_start_of_named_tuple_element(&mut self) -> bool {
        if self.token == Kind::DotDotDotToken {
            return token_is_identifier_or_keyword(self.next_token()) && self.next_token_is_colon_or_question_colon();
        }
        token_is_identifier_or_keyword(self.token) && self.next_token_is_colon_or_question_colon()
    }

    pub(crate) fn next_token_is_colon_or_question_colon(&mut self) -> bool {
        self.next_token() == Kind::ColonToken
            || self.token == Kind::QuestionToken && self.next_token() == Kind::ColonToken
    }

    pub(crate) fn parse_tuple_element_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        if self.parse_optional(Kind::DotDotDotToken) {
            let type_node = self.parse_type();
            return self.finish_node(Node::new(Kind::RestType).ty(type_node), pos);
        }
        let type_node = self.parse_type();
        if self.kind(type_node) == Kind::JSDocNullableType
            && let Some(inner) = self.node(type_node).ty
            && self.node(type_node).pos == self.node(inner).pos
        {
            // port: Go builds a fresh OptionalType with the nullable type's flags
            // and range and the same inner type; retagging the node is equivalent.
            self.node_mut(type_node).kind = Kind::OptionalType;
            return type_node;
        }
        type_node
    }

    pub(crate) fn parse_parenthesized_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenParenToken);
        let type_node = self.parse_type();
        self.parse_expected(Kind::CloseParenToken);
        self.finish_node(Node::new(Kind::ParenthesizedType).ty(type_node), pos)
    }

    pub(crate) fn parse_asserts_type_predicate(&mut self) -> NodeId {
        let pos = self.node_pos();
        let asserts_modifier = self.parse_expected_token(Kind::AssertsKeyword);
        let parameter_name = if self.token == Kind::ThisKeyword {
            self.parse_this_type_node()
        } else {
            self.parse_identifier()
        };
        let mut type_node = None;
        if self.parse_optional(Kind::IsKeyword) {
            type_node = Some(self.parse_type());
        }
        let node = Node::new(Kind::TypePredicate).child(asserts_modifier).child(parameter_name).ty(type_node);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_template_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        let head = self.parse_template_head(false);
        let spans = self.parse_template_type_spans();
        self.finish_node(Node::new(Kind::TemplateLiteralType).child(head).list(spans), pos)
    }

    pub(crate) fn parse_template_head(&mut self, is_tagged_template: bool) -> NodeId {
        if !is_tagged_template && self.scanner.token_flags().has(TokenFlags::IsInvalid) {
            self.re_scan_template_token(false);
        }
        let pos = self.node_pos();
        let text = self.scanner.token_value().to_string();
        let _raw_text = self.get_template_literal_raw_text(2);
        let result = Node::new(Kind::TemplateHead).text(text).token_flags(self.scanner.token_flags());
        self.next_token();
        self.finish_node(result, pos)
    }

    pub(crate) fn get_template_literal_raw_text(&self, end_length: usize) -> &'a str {
        let token_text = self.scanner.token_text();
        let end_length = if self.scanner.token_flags().has(TokenFlags::Unterminated) { 0 } else { end_length };
        // port: Go slices unchecked; the bounds always hold for template tokens.
        token_text.get(1..token_text.len().saturating_sub(end_length)).unwrap_or("")
    }

    pub(crate) fn parse_template_type_spans(&mut self) -> NodeList {
        let pos = self.node_pos();
        let mut list = Vec::new();
        loop {
            let span = self.parse_template_type_span();
            list.push(span);
            // slot: a TemplateLiteralTypeSpan's literal is children[0].
            let literal = self.node(span).children[0];
            if literal.map(|literal| self.kind(literal)) != Some(Kind::TemplateMiddle) {
                break;
            }
        }
        NodeList::new(pos, self.node_pos(), list)
    }

    pub(crate) fn parse_template_type_span(&mut self) -> NodeId {
        let pos = self.node_pos();
        let type_node = self.parse_type();
        let literal = self.parse_literal_of_template_span(false);
        self.finish_node(Node::new(Kind::TemplateLiteralTypeSpan).ty(type_node).child(literal), pos)
    }

    pub(crate) fn parse_literal_of_template_span(&mut self, is_tagged_template: bool) -> NodeId {
        if self.token == Kind::CloseBraceToken {
            self.re_scan_template_token(is_tagged_template);
            return self.parse_template_middle_or_tail();
        }
        self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(Kind::CloseBraceToken)]);
        let pos = self.node_pos();
        self.finish_node(Node::new(Kind::TemplateTail).text("").token_flags(TokenFlags::None), pos)
    }

    pub(crate) fn parse_template_middle_or_tail(&mut self) -> NodeId {
        let pos = self.node_pos();
        let text = self.scanner.token_value().to_string();
        let flags = self.scanner.token_flags();
        let result = if self.token == Kind::TemplateMiddle {
            let _raw_text = self.get_template_literal_raw_text(2);
            Node::new(Kind::TemplateMiddle).text(text).token_flags(flags)
        } else {
            let _raw_text = self.get_template_literal_raw_text(1);
            Node::new(Kind::TemplateTail).text(text).token_flags(flags)
        };
        self.next_token();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_function_or_constructor_type_to_error(
        &mut self,
        is_in_union_type: bool,
        parse_constituent_type: fn(&mut Self) -> NodeId,
    ) -> NodeId {
        // the function type and constructor type shorthand notation
        // are not allowed directly in unions and intersections, but we'll
        // try to parse them gracefully and issue a helpful message.
        if self.is_start_of_function_type_or_constructor_type() {
            let type_node = self.parse_function_or_constructor_type();
            let diagnostic = if self.kind(type_node) == Kind::FunctionType {
                if is_in_union_type {
                    diagnostics::Function_type_notation_must_be_parenthesized_when_used_in_a_union_type
                } else {
                    diagnostics::Function_type_notation_must_be_parenthesized_when_used_in_an_intersection_type
                }
            } else if is_in_union_type {
                diagnostics::Constructor_type_notation_must_be_parenthesized_when_used_in_a_union_type
            } else {
                diagnostics::Constructor_type_notation_must_be_parenthesized_when_used_in_an_intersection_type
            };
            let (start, end) = (self.node(type_node).pos, self.node(type_node).end);
            self.parse_error_at_range(start, end, diagnostic, &[]);
            return type_node;
        }
        parse_constituent_type(self)
    }

    pub(crate) fn is_start_of_function_type_or_constructor_type(&mut self) -> bool {
        self.token == Kind::LessThanToken
            || self.token == Kind::OpenParenToken && self.look_ahead(Self::next_is_unambiguously_start_of_function_type)
            || self.token == Kind::NewKeyword
            || self.token == Kind::AbstractKeyword && self.look_ahead(Self::next_token_is_new_keyword)
    }

    pub(crate) fn parse_function_or_constructor_type(&mut self) -> NodeId {
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers_for_constructor_type();
        let is_constructor_type = self.parse_optional(Kind::NewKeyword);
        debug_assert!(
            modifiers.is_none() || is_constructor_type,
            "Per isStartOfFunctionOrConstructorType, a function type cannot have modifiers."
        );
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters(ParseFlags::Type);
        let return_type = self.parse_return_type(Kind::EqualsGreaterThanToken, false);
        let result = if is_constructor_type {
            Node::new(Kind::ConstructorType)
                .modifiers(modifiers)
                .type_parameters(type_parameters)
                .parameters(parameters)
                .ty(return_type)
        } else {
            Node::new(Kind::FunctionType).type_parameters(type_parameters).parameters(parameters).ty(return_type)
        };
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_modifiers_for_constructor_type(&mut self) -> Option<NodeList> {
        if self.token == Kind::AbstractKeyword {
            let pos = self.node_pos();
            let modifier = Node::new(self.token);
            self.next_token();
            let modifier = self.finish_node(modifier, pos);
            let (start, end) = (self.node(modifier).pos, self.node(modifier).end);
            return Some(NodeList::new(start, end, vec![modifier]));
        }
        None
    }

    pub(crate) fn next_token_is_new_keyword(&mut self) -> bool {
        self.next_token() == Kind::NewKeyword
    }

    pub(crate) fn next_is_unambiguously_start_of_function_type(&mut self) -> bool {
        self.next_token();
        if self.token == Kind::CloseParenToken || self.token == Kind::DotDotDotToken {
            // ( )
            // ( ...
            return true;
        }
        if self.skip_parameter_start() {
            // We successfully skipped modifiers (if any) and an identifier or binding pattern,
            // now see if we have something that indicates a parameter declaration
            if matches!(
                self.token,
                Kind::ColonToken | Kind::CommaToken | Kind::QuestionToken | Kind::EqualsToken
            ) {
                // ( xxx :
                // ( xxx ,
                // ( xxx ?
                // ( xxx =
                return true;
            }
            if self.token == Kind::CloseParenToken && self.next_token() == Kind::EqualsGreaterThanToken {
                // ( xxx ) =>
                return true;
            }
        }
        false
    }

    pub(crate) fn skip_parameter_start(&mut self) -> bool {
        if ast::is_modifier_kind(self.token) {
            // Skip modifiers
            self.parse_modifiers();
        }
        self.parse_optional(Kind::DotDotDotToken);
        if self.is_identifier() || self.token == Kind::ThisKeyword {
            self.next_token();
            return true;
        }
        if self.token == Kind::OpenBracketToken || self.token == Kind::OpenBraceToken {
            // Return true if we can parse an array or object binding pattern with no errors
            let previous_error_count = self.scanner.diagnostics.len();
            self.parse_identifier_or_pattern();
            return previous_error_count == self.scanner.diagnostics.len();
        }
        false
    }

    pub(crate) fn parse_modifiers(&mut self) -> Option<NodeList> {
        self.parse_modifiers_ex(false, false, false)
    }

    pub(crate) fn parse_modifiers_ex(
        &mut self,
        allow_decorators: bool,
        permit_const_as_modifier: bool,
        stop_on_start_of_class_static_block: bool,
    ) -> Option<NodeList> {
        let mut has_leading_modifier = false;
        let mut has_trailing_decorator = false;
        let mut has_trailing_modifier = false;
        let mut has_static_modifier = false;
        // Decorators should be contiguous in a list of modifiers but can potentially appear in two places (i.e., `[...leadingDecorators, ...leadingModifiers, ...trailingDecorators, ...trailingModifiers]`).
        // The leading modifiers *should* only contain `export` and `default` when trailingDecorators are present, but we'll handle errors for any other leading modifiers in the checker.
        // It is illegal to have both leadingDecorators and trailingDecorators, but we will report that as a grammar check in the checker.
        // parse leading decorators
        let pos = self.node_pos();
        let mut list = Vec::new();
        loop {
            if allow_decorators && self.token == Kind::AtToken && !has_trailing_modifier {
                let decorator = self.parse_decorator();
                list.push(decorator);
                if has_leading_modifier {
                    has_trailing_decorator = true;
                }
            } else {
                let Some(modifier) = self.try_parse_modifier(
                    has_static_modifier,
                    permit_const_as_modifier,
                    stop_on_start_of_class_static_block,
                ) else {
                    break;
                };
                if self.kind(modifier) == Kind::StaticKeyword {
                    has_static_modifier = true;
                }
                list.push(modifier);
                if has_trailing_decorator {
                    has_trailing_modifier = true;
                } else {
                    has_leading_modifier = true;
                }
            }
        }
        if !list.is_empty() {
            return Some(NodeList::new(pos, self.node_pos(), list));
        }
        None
    }

    pub(crate) fn parse_decorator(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::AtToken);
        let expression = self.do_in_context(NodeFlags::DecoratorContext, true, Self::parse_decorator_expression);
        self.finish_node(Node::new(Kind::Decorator).expression(expression), pos)
    }

    pub(crate) fn parse_decorator_expression(&mut self) -> NodeId {
        if self.in_await_context() && self.token == Kind::AwaitKeyword {
            // `@await` is disallowed in an [Await] context, but can cause parsing to go off the rails
            // This simply parses the missing identifier and moves on.
            let pos = self.node_pos();
            let await_expression = self.parse_identifier_with_diagnostic(Some(diagnostics::Expression_expected), None);
            self.next_token();
            let member_expression = self.parse_member_expression_rest(pos, await_expression, true);
            return self.parse_call_expression_rest(pos, member_expression);
        }
        self.parse_left_hand_side_expression_or_higher()
    }

    pub(crate) fn try_parse_modifier(
        &mut self,
        has_seen_static_modifier: bool,
        permit_const_as_modifier: bool,
        stop_on_start_of_class_static_block: bool,
    ) -> Option<NodeId> {
        let pos = self.node_pos();
        let kind = self.token;
        if self.token == Kind::ConstKeyword && permit_const_as_modifier {
            // We need to ensure that any subsequent modifiers appear on the same line
            // so that when 'const' is a standalone declaration, we don't issue an error.
            if !self.look_ahead(Self::next_token_is_on_same_line_and_can_follow_modifier) {
                return None;
            } else {
                self.next_token();
            }
        } else if stop_on_start_of_class_static_block
            && self.token == Kind::StaticKeyword
            && self.look_ahead(Self::next_token_is_open_brace)
        {
            return None;
        } else if has_seen_static_modifier && self.token == Kind::StaticKeyword {
            return None;
        } else if !self.parse_any_contextual_modifier() {
            return None;
        }
        Some(self.finish_node(Node::new(kind), pos))
    }

    pub(crate) fn parse_contextual_modifier(&mut self, t: Kind) -> bool {
        let state = self.mark();
        if self.token == t && self.next_token_can_follow_modifier() {
            return true;
        }
        self.rewind(state);
        false
    }

    pub(crate) fn parse_any_contextual_modifier(&mut self) -> bool {
        let state = self.mark();
        if ast::is_modifier_kind(self.token) && self.next_token_can_follow_modifier() {
            return true;
        }
        self.rewind(state);
        false
    }

    pub(crate) fn next_token_can_follow_modifier(&mut self) -> bool {
        match self.token {
            Kind::ConstKeyword => {
                // 'const' is only a modifier if followed by 'enum'.
                self.next_token() == Kind::EnumKeyword
            }
            Kind::ExportKeyword => {
                self.next_token();
                if self.token == Kind::DefaultKeyword {
                    return self.look_ahead(Self::next_token_can_follow_default_keyword);
                }
                if self.token == Kind::TypeKeyword {
                    return self.look_ahead(Self::next_token_can_follow_export_modifier);
                }
                self.can_follow_export_modifier()
            }
            Kind::DefaultKeyword => self.next_token_can_follow_default_keyword(),
            Kind::StaticKeyword => {
                self.next_token();
                self.can_follow_modifier()
            }
            Kind::GetKeyword | Kind::SetKeyword => {
                self.next_token();
                self.can_follow_get_or_set_keyword()
            }
            _ => self.next_token_is_on_same_line_and_can_follow_modifier(),
        }
    }

    pub(crate) fn next_token_can_follow_default_keyword(&mut self) -> bool {
        match self.next_token() {
            Kind::ClassKeyword | Kind::FunctionKeyword | Kind::InterfaceKeyword | Kind::AtToken => true,
            Kind::AbstractKeyword => self.look_ahead(Self::next_token_is_class_keyword_on_same_line),
            Kind::AsyncKeyword => self.look_ahead(Self::next_token_is_function_keyword_on_same_line),
            _ => false,
        }
    }

    pub(crate) fn next_token_is_identifier_or_keyword(&mut self) -> bool {
        token_is_identifier_or_keyword(self.next_token())
    }

    pub(crate) fn next_token_is_identifier_or_keyword_or_greater_than(&mut self) -> bool {
        token_is_identifier_or_keyword_or_greater_than(self.next_token())
    }

    pub(crate) fn next_token_is_identifier_or_keyword_on_same_line(&mut self) -> bool {
        self.next_token_is_identifier_or_keyword() && !self.has_preceding_line_break()
    }

    pub(crate) fn next_token_is_identifier_or_keyword_or_literal_on_same_line(&mut self) -> bool {
        (self.next_token_is_identifier_or_keyword()
            || matches!(self.token, Kind::NumericLiteral | Kind::BigIntLiteral | Kind::StringLiteral))
            && !self.has_preceding_line_break()
    }

    pub(crate) fn next_token_is_class_keyword_on_same_line(&mut self) -> bool {
        self.next_token() == Kind::ClassKeyword && !self.has_preceding_line_break()
    }

    pub(crate) fn next_token_is_function_keyword_on_same_line(&mut self) -> bool {
        self.next_token() == Kind::FunctionKeyword && !self.has_preceding_line_break()
    }

    pub(crate) fn next_token_can_follow_export_modifier(&mut self) -> bool {
        self.next_token();
        self.can_follow_export_modifier()
    }

    pub(crate) fn can_follow_export_modifier(&self) -> bool {
        self.token == Kind::AtToken
            || self.token != Kind::AsteriskToken
                && self.token != Kind::AsKeyword
                && self.token != Kind::OpenBraceToken
                && self.can_follow_modifier()
    }

    pub(crate) fn can_follow_modifier(&self) -> bool {
        matches!(
            self.token,
            Kind::OpenBracketToken | Kind::OpenBraceToken | Kind::AsteriskToken | Kind::DotDotDotToken
        ) || self.is_literal_property_name()
    }

    pub(crate) fn can_follow_get_or_set_keyword(&self) -> bool {
        self.token == Kind::OpenBracketToken || self.is_literal_property_name()
    }

    pub(crate) fn next_token_is_on_same_line_and_can_follow_modifier(&mut self) -> bool {
        self.next_token();
        if self.has_preceding_line_break() {
            return false;
        }
        self.can_follow_modifier()
    }

    pub(crate) fn next_token_is_open_brace(&mut self) -> bool {
        self.next_token() == Kind::OpenBraceToken
    }
}
