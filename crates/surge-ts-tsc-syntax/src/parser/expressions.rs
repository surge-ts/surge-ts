//! Expressions: typescript-go `parser.go` from `parseExpression` through
//! `parseLiteralExpression`, in the Go order.

#![allow(unused_imports)]

use crate::ast::{self, Node, NodeId, NodeList, OperatorPrecedence};
use crate::flags::{NodeFlags, TokenFlags};
use crate::kind::Kind;
use crate::messages as diagnostics;
use crate::scanner::{self, token_is_identifier_or_keyword, token_to_string};
use crate::Message;

use super::{PC, ParseFlags, Parser, Tristate};

/// `modifierListHasAsync`.
fn modifier_list_has_async(parser: &Parser<'_>, modifiers: &Option<NodeList>) -> bool {
    modifiers
        .as_ref()
        .is_some_and(|list| list.nodes.iter().any(|&m| parser.kind(m) == Kind::AsyncKeyword))
}

/// `typeHasArrowFunctionBlockingParseError`: if true, we should abort parsing an arrow function.
fn type_has_arrow_function_blocking_parse_error(parser: &Parser<'_>, node: NodeId) -> bool {
    match parser.kind(node) {
        Kind::TypeReference => parser.node_is_missing(parser.node(node).name),
        Kind::FunctionType | Kind::ConstructorType => {
            parser.node(node).parameters.as_ref().is_some_and(|list| list.missing)
                || parser.node(node).ty.is_some_and(|ty| type_has_arrow_function_blocking_parse_error(parser, ty))
        }
        Kind::ParenthesizedType => {
            parser.node(node).ty.is_some_and(|ty| type_has_arrow_function_blocking_parse_error(parser, ty))
        }
        _ => false,
    }
}

/// `scanner.GetTextOfNodeFromSourceText(sourceText, node, false)`.
fn text_of_node(parser: &Parser<'_>, id: NodeId) -> String {
    if parser.node_is_missing(Some(id)) {
        return String::new();
    }
    let node = parser.node(id);
    let pos = scanner::skip_trivia(parser.source_text, node.pos).min(node.end);
    parser.source_text[pos..node.end].to_string()
}

fn jsx_tag_name(parser: &Parser<'_>, id: NodeId) -> NodeId {
    parser.node(id).name.expect("a JSX opening/closing element has a tag name")
}

// slot: JsxElement is `.child(openingElement)`, `.list(children)`, `.child(closingElement)`.
fn jsx_element_opening(parser: &Parser<'_>, id: NodeId) -> NodeId {
    parser.node(id).children[0].expect("a JSX element has an opening element")
}

fn jsx_element_closing(parser: &Parser<'_>, id: NodeId) -> NodeId {
    parser.node(id).children[1].expect("a JSX element has a closing element")
}

fn jsx_element_children(parser: &Parser<'_>, id: NodeId) -> NodeList {
    parser.node(id).lists[0].clone().unwrap_or_default()
}

impl<'a> Parser<'a> {
    pub(crate) fn parse_expression(&mut self) -> NodeId {
        // clear the decorator context when parsing Expression, as it should be unambiguous when parsing a decorator
        let save_context_flags = self.context_flags;
        self.context_flags &= !NodeFlags::DecoratorContext;
        let pos = self.node_pos();
        let mut expr = self.parse_assignment_expression_or_higher();
        loop {
            let Some(operator_token) = self.parse_optional_token(Kind::CommaToken) else {
                break;
            };
            let right = self.parse_assignment_expression_or_higher();
            expr = self.make_binary_expression(expr, operator_token, right, pos);
        }
        self.context_flags = save_context_flags;
        expr
    }

    pub(crate) fn parse_expression_allow_in(&mut self) -> NodeId {
        self.do_in_context(NodeFlags::DisallowInContext, false, Self::parse_expression)
    }

    pub(crate) fn parse_assignment_expression_or_higher(&mut self) -> NodeId {
        self.parse_assignment_expression_or_higher_worker(true)
    }

    pub(crate) fn parse_assignment_expression_or_higher_worker(&mut self, allow_return_type_in_arrow_function: bool) -> NodeId {
        if self.is_yield_expression() {
            return self.parse_yield_expression();
        }
        if let Some(arrow_expression) =
            self.try_parse_parenthesized_arrow_function_expression(allow_return_type_in_arrow_function)
        {
            return arrow_expression;
        }
        if let Some(arrow_expression) =
            self.try_parse_async_simple_arrow_function_expression(allow_return_type_in_arrow_function)
        {
            return arrow_expression;
        }
        let pos = self.node_pos();
        let expr = self.parse_binary_expression_or_higher(OperatorPrecedence::Lowest);
        // A single un-parenthesized parameter ('x => ...') is handled here rather than with a look-ahead above.
        if self.kind(expr) == Kind::Identifier && self.token == Kind::EqualsGreaterThanToken {
            return self.parse_simple_arrow_function_expression(pos, expr, allow_return_type_in_arrow_function, None);
        }
        // reScanGreaterThanToken merges `> > =` into `>>=`.
        if ast::is_left_hand_side_expression_kind(self.kind(expr))
            && ast::is_assignment_operator(self.re_scan_greater_than_token())
        {
            let operator_token = self.parse_token_node();
            let right = self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function);
            return self.make_binary_expression(expr, operator_token, right, pos);
        }
        self.parse_conditional_expression_rest(expr, pos, allow_return_type_in_arrow_function)
    }

    pub(crate) fn is_yield_expression(&mut self) -> bool {
        if self.token == Kind::YieldKeyword {
            if self.in_yield_context() {
                return true;
            }
            // Outside a yield context, only `yield` followed by something that cannot continue an
            // identifier expression is treated as a yield expression (reported later).
            return self.look_ahead(Self::next_token_is_identifier_or_keyword_or_literal_on_same_line);
        }
        false
    }

    pub(crate) fn parse_yield_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let result = if !self.has_preceding_line_break()
            && (self.token == Kind::AsteriskToken || self.is_start_of_expression())
        {
            let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
            let expression = self.parse_assignment_expression_or_higher();
            Node::new(Kind::YieldExpression).child(asterisk_token).expression(expression)
        } else {
            Node::new(Kind::YieldExpression).child(None)
        };
        self.finish_node(result, pos)
    }

    pub(crate) fn is_parenthesized_arrow_function_expression(&mut self) -> Tristate {
        if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken || self.token == Kind::AsyncKeyword {
            let state = self.mark();
            let result = self.next_is_parenthesized_arrow_function_expression();
            self.rewind(state);
            return result;
        }
        if self.token == Kind::EqualsGreaterThanToken {
            // ERROR RECOVERY TWEAK: a standalone => is parsed as an arrow function expression.
            return Tristate::True;
        }
        Tristate::False
    }

    pub(crate) fn next_is_parenthesized_arrow_function_expression(&mut self) -> Tristate {
        if self.token == Kind::AsyncKeyword {
            self.next_token();
            if self.has_preceding_line_break() {
                return Tristate::False;
            }
            if self.token != Kind::OpenParenToken && self.token != Kind::LessThanToken {
                return Tristate::False;
            }
        }
        let first = self.token;
        let second = self.next_token();
        if first == Kind::OpenParenToken {
            if second == Kind::CloseParenToken {
                // "() =>", "(): " and "() {".
                let third = self.next_token();
                return match third {
                    Kind::EqualsGreaterThanToken | Kind::ColonToken | Kind::OpenBraceToken => Tristate::True,
                    _ => Tristate::False,
                };
            }
            // "([" or "({" could be the start of a binding pattern.
            if second == Kind::OpenBracketToken || second == Kind::OpenBraceToken {
                return Tristate::Unknown;
            }
            // "(..." is an arrow function with a rest parameter.
            if second == Kind::DotDotDotToken {
                return Tristate::True;
            }
            // "(modifier identifier" is not allowed, but is treated as a lambda for a better error.
            if ast::is_modifier_kind(second)
                && second != Kind::AsyncKeyword
                && self.look_ahead(Self::next_token_is_identifier)
            {
                if self.next_token() == Kind::AsKeyword {
                    // https://github.com/microsoft/TypeScript/issues/44466
                    return Tristate::False;
                }
                return Tristate::True;
            }
            // "(" followed by something other than an identifier or "this" is not a lambda.
            if !self.is_identifier() && second != Kind::ThisKeyword {
                return Tristate::False;
            }
            match self.next_token() {
                Kind::ColonToken => return Tristate::True,
                Kind::QuestionToken => {
                    self.next_token();
                    if matches!(
                        self.token,
                        Kind::ColonToken | Kind::CommaToken | Kind::EqualsToken | Kind::CloseParenToken
                    ) {
                        return Tristate::True;
                    }
                    return Tristate::False;
                }
                Kind::CommaToken | Kind::EqualsToken | Kind::CloseParenToken => return Tristate::Unknown,
                _ => {}
            }
            Tristate::False
        } else {
            debug_assert!(first == Kind::LessThanToken);
            if !self.is_identifier() && self.token != Kind::ConstKeyword {
                return Tristate::False;
            }
            if self.jsx {
                let is_arrow_function_in_jsx = self.look_ahead(|parser| {
                    parser.parse_optional(Kind::ConstKeyword);
                    let third = parser.next_token();
                    if third == Kind::ExtendsKeyword {
                        let fourth = parser.next_token();
                        return !matches!(fourth, Kind::EqualsToken | Kind::GreaterThanToken | Kind::SlashToken);
                    } else if third == Kind::CommaToken || third == Kind::EqualsToken {
                        return true;
                    }
                    false
                });
                if is_arrow_function_in_jsx {
                    return Tristate::True;
                }
                return Tristate::False;
            }
            Tristate::Unknown
        }
    }

    pub(crate) fn try_parse_parenthesized_arrow_function_expression(
        &mut self,
        allow_return_type_in_arrow_function: bool,
    ) -> Option<NodeId> {
        let tristate = self.is_parenthesized_arrow_function_expression();
        if tristate == Tristate::False {
            return None;
        }
        if tristate == Tristate::True {
            return self.parse_parenthesized_arrow_function_expression(true, true);
        }
        let state = self.mark();
        let result = self.parse_possible_parenthesized_arrow_function_expression(allow_return_type_in_arrow_function);
        if result.is_none() {
            self.rewind(state);
        }
        result
    }

    pub(crate) fn parse_parenthesized_arrow_function_expression(
        &mut self,
        allow_ambiguity: bool,
        allow_return_type_in_arrow_function: bool,
    ) -> Option<NodeId> {
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers_for_arrow_function();
        let is_async = modifier_list_has_async(self, &modifiers);
        let signature_flags = if is_async { ParseFlags::Await } else { ParseFlags::None };
        // When speculating, the parameter list must be complete: in `a => (b => c)`, "(b =>" is not a
        // parenthesized arrow function with a missing close paren.
        let type_parameters = self.parse_type_parameters();
        let parameters: Option<NodeList>;
        if !self.parse_expected(Kind::OpenParenToken) {
            if !allow_ambiguity {
                return None;
            }
            parameters = Some(self.create_missing_list());
        } else {
            if !allow_ambiguity {
                let maybe_parameters = self.parse_parameters_worker(signature_flags, allow_ambiguity);
                if maybe_parameters.is_none() {
                    return None;
                }
                parameters = maybe_parameters;
            } else {
                parameters = self.parse_parameters_worker(signature_flags, allow_ambiguity);
            }
            if !self.parse_expected(Kind::CloseParenToken) && !allow_ambiguity {
                return None;
            }
        }
        let has_return_colon = self.token == Kind::ColonToken;
        let return_type = self.parse_return_type(Kind::ColonToken, false);
        if let Some(return_type) = return_type
            && !allow_ambiguity
            && type_has_arrow_function_blocking_parse_error(self, return_type)
        {
            return None;
        }
        // port: Go computes an unused `unwrappedType` (return type without parentheses) here.
        if !allow_ambiguity && self.token != Kind::EqualsGreaterThanToken && self.token != Kind::OpenBraceToken {
            // The caller rewinds.
            return None;
        }
        // Parse the body after an arrow, and also after an opening brace in case of an error state.
        let last_token = self.token;
        let equals_greater_than_token = self.parse_expected_token(Kind::EqualsGreaterThanToken);
        let body = if last_token == Kind::EqualsGreaterThanToken || last_token == Kind::OpenBraceToken {
            self.parse_arrow_function_expression_body(is_async, allow_return_type_in_arrow_function)
        } else {
            self.parse_identifier()
        };
        // In `x ? y => ({ y }) : z => ({ z })` the colon terminates the true branch, so a return type is
        // only allowed when not in that position, unless another colon follows (`a ? (x): string => x : null`).
        if !allow_return_type_in_arrow_function && has_return_colon && self.token != Kind::ColonToken {
            return None;
        }
        let node = Node::new(Kind::ArrowFunction)
            .modifiers(modifiers)
            .type_parameters(type_parameters)
            .parameters(parameters)
            .ty(return_type)
            .child(equals_greater_than_token)
            .body(body);
        let result = self.finish_node(node, pos);
        self.check_js_syntax(result);
        Some(result)
    }

    pub(crate) fn parse_modifiers_for_arrow_function(&mut self) -> Option<NodeList> {
        if self.token == Kind::AsyncKeyword {
            let pos = self.node_pos();
            self.next_token();
            let modifier = self.finish_node(Node::new(Kind::AsyncKeyword), pos);
            let (start, end) = (self.node(modifier).pos, self.node(modifier).end);
            return Some(NodeList::new(start, end, vec![modifier]));
        }
        None
    }

    pub(crate) fn parse_arrow_function_expression_body(
        &mut self,
        is_async: bool,
        allow_return_type_in_arrow_function: bool,
    ) -> NodeId {
        let await_flag = if is_async { ParseFlags::Await } else { ParseFlags::None };
        if self.token == Kind::OpenBraceToken {
            return self.parse_function_block(await_flag, None);
        }
        if self.token != Kind::SemicolonToken
            && self.token != Kind::FunctionKeyword
            && self.token != Kind::ClassKeyword
            && self.is_start_of_statement()
            && !self.is_start_of_expression_statement()
        {
            // A plain statement after `=>` most likely means a missing open brace
            // (`a => let v = 0; }`); recover as a block so the next `}` does not close the container.
            return self.parse_function_block(ParseFlags::IgnoreMissingOpenBrace | await_flag, None);
        }
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::AwaitContext, is_async);
        self.set_context_flags(NodeFlags::YieldContext, false);
        let node = self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function);
        self.context_flags = save_context_flags;
        node
    }

    pub(crate) fn is_start_of_expression_statement(&mut self) -> bool {
        self.token != Kind::OpenBraceToken
            && self.token != Kind::FunctionKeyword
            && self.token != Kind::ClassKeyword
            && self.token != Kind::AtToken
            && self.is_start_of_expression()
    }

    pub(crate) fn parse_possible_parenthesized_arrow_function_expression(
        &mut self,
        allow_return_type_in_arrow_function: bool,
    ) -> Option<NodeId> {
        let token_pos = self.scanner.token_start();
        if self.not_parenthesized_arrow.contains(&token_pos) {
            return None;
        }
        let result = self.parse_parenthesized_arrow_function_expression(false, allow_return_type_in_arrow_function);
        if result.is_none() {
            self.not_parenthesized_arrow.insert(token_pos);
        }
        result
    }

    pub(crate) fn try_parse_async_simple_arrow_function_expression(
        &mut self,
        allow_return_type_in_arrow_function: bool,
    ) -> Option<NodeId> {
        if self.token == Kind::AsyncKeyword && self.look_ahead(Self::next_is_un_parenthesized_async_arrow_function) {
            let pos = self.node_pos();
            let async_modifier = self.parse_modifiers_for_arrow_function();
            let expr = self.parse_binary_expression_or_higher(OperatorPrecedence::Lowest);
            return Some(self.parse_simple_arrow_function_expression(
                pos,
                expr,
                allow_return_type_in_arrow_function,
                async_modifier,
            ));
        }
        None
    }

    pub(crate) fn next_is_un_parenthesized_async_arrow_function(&mut self) -> bool {
        if self.token == Kind::AsyncKeyword {
            self.next_token();
            // `async =>` is a simple arrow function with a parameter named async.
            if self.has_preceding_line_break() || self.token == Kind::EqualsGreaterThanToken {
                return false;
            }
            let expr = self.parse_binary_expression_or_higher(OperatorPrecedence::Lowest);
            if !self.has_preceding_line_break()
                && self.kind(expr) == Kind::Identifier
                && self.token == Kind::EqualsGreaterThanToken
            {
                return true;
            }
        }
        false
    }

    pub(crate) fn parse_simple_arrow_function_expression(
        &mut self,
        pos: usize,
        identifier: NodeId,
        allow_return_type_in_arrow_function: bool,
        async_modifier: Option<NodeList>,
    ) -> NodeId {
        debug_assert!(
            self.token == Kind::EqualsGreaterThanToken,
            "parseSimpleArrowFunctionExpression should only have been called if we had a =>"
        );
        let identifier_pos = self.node(identifier).pos;
        let parameter = self.finish_node(
            Node::new(Kind::Parameter)
                .modifiers(None)
                .child(None)
                .name(identifier)
                .question(None)
                .ty(None)
                .initializer(None),
            identifier_pos,
        );
        let (parameter_pos, parameter_end) = (self.node(parameter).pos, self.node(parameter).end);
        let parameters = NodeList::new(parameter_pos, parameter_end, vec![parameter]);
        let equals_greater_than_token = self.parse_expected_token(Kind::EqualsGreaterThanToken);
        let body =
            self.parse_arrow_function_expression_body(async_modifier.is_some(), allow_return_type_in_arrow_function);
        let node = Node::new(Kind::ArrowFunction)
            .modifiers(async_modifier)
            .type_parameters(None)
            .parameters(parameters)
            .ty(None)
            .child(equals_greater_than_token)
            .body(body);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_conditional_expression_rest(
        &mut self,
        left_operand: NodeId,
        pos: usize,
        allow_return_type_in_arrow_function: bool,
    ) -> NodeId {
        let Some(question_token) = self.parse_optional_token(Kind::QuestionToken) else {
            return left_operand;
        };
        // 'in' is explicitly allowed in the whenTrue part, but not in the whenFalse part.
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::DisallowInContext, false);
        let true_expression = self.parse_assignment_expression_or_higher_worker(false);
        self.context_flags = save_context_flags;
        let colon_token = self.parse_expected_token(Kind::ColonToken);
        let false_expression = if self.node_is_present(Some(colon_token)) {
            self.parse_assignment_expression_or_higher_worker(allow_return_type_in_arrow_function)
        } else {
            self.create_missing_identifier()
        };
        let node = Node::new(Kind::ConditionalExpression)
            .child(left_operand)
            .child(question_token)
            .child(true_expression)
            .child(colon_token)
            .child(false_expression);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_binary_expression_or_higher(&mut self, precedence: OperatorPrecedence) -> NodeId {
        let pos = self.node_pos();
        let left_operand = self.parse_unary_expression_or_higher();
        self.parse_binary_expression_rest(precedence, left_operand, pos)
    }

    pub(crate) fn parse_binary_expression_rest(
        &mut self,
        precedence: OperatorPrecedence,
        mut left_operand: NodeId,
        pos: usize,
    ) -> NodeId {
        let mut last_operand = left_operand;
        loop {
            // reScanGreaterThanToken merges token sequences like > and = into >=.
            let operator = self.re_scan_greater_than_token();
            let new_precedence = ast::get_binary_operator_precedence(operator);
            // `**` is right associative; every other operator is left associative.
            let consume_current_operator = if operator == Kind::AsteriskAsteriskToken {
                new_precedence >= precedence
            } else {
                new_precedence > precedence
            };
            if !consume_current_operator {
                break;
            }
            if operator == Kind::InKeyword && self.in_disallow_in_context() {
                break;
            }
            if operator == Kind::AsKeyword || operator == Kind::SatisfiesKeyword {
                // ASI applies before `as`/`satisfies` on a new line.
                if self.has_preceding_line_break() {
                    break;
                } else {
                    self.next_token();
                    // After `a ## b as T`, stop at an operator binding tighter than ## so the `as` can be
                    // erased without changing meaning (microsoft/TypeScript#63527).
                    let mut last_precedence = OperatorPrecedence::Highest;
                    if self.kind(last_operand) == Kind::BinaryExpression {
                        last_precedence = ast::get_binary_operator_precedence(self.node(last_operand).op);
                    }
                    let ty = self.parse_type();
                    if operator == Kind::SatisfiesKeyword {
                        left_operand = self.make_satisfies_expression(left_operand, ty);
                    } else {
                        left_operand = self.make_as_expression(left_operand, ty);
                    }
                    if ast::get_binary_operator_precedence(self.re_scan_greater_than_token()) > last_precedence {
                        break;
                    }
                }
            } else {
                let operator_token = self.parse_token_node();
                let right = self.parse_binary_expression_or_higher(new_precedence);
                left_operand = self.make_binary_expression(left_operand, operator_token, right, pos);
                last_operand = left_operand;
            }
        }
        left_operand
    }

    pub(crate) fn make_satisfies_expression(&mut self, expression: NodeId, type_node: NodeId) -> NodeId {
        let pos = self.node(expression).pos;
        let node = Node::new(Kind::SatisfiesExpression).expression(expression).ty(type_node);
        let result = self.finish_node(node, pos);
        self.check_js_syntax(result)
    }

    pub(crate) fn make_as_expression(&mut self, left: NodeId, right: NodeId) -> NodeId {
        let pos = self.node(left).pos;
        let node = Node::new(Kind::AsExpression).expression(left).ty(right);
        let result = self.finish_node(node, pos);
        self.check_js_syntax(result)
    }

    pub(crate) fn make_binary_expression(&mut self, left: NodeId, operator_token: NodeId, right: NodeId, pos: usize) -> NodeId {
        let operator = self.kind(operator_token);
        let node = Node::new(Kind::BinaryExpression)
            .modifiers(None)
            .child(left)
            .child(operator_token)
            .child(right)
            .op(operator);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_unary_expression_or_higher(&mut self) -> NodeId {
        if self.is_update_expression() {
            let pos = self.node_pos();
            let update_expression = self.parse_update_expression();
            if self.token == Kind::AsteriskAsteriskToken {
                let precedence = ast::get_binary_operator_precedence(self.token);
                return self.parse_binary_expression_rest(precedence, update_expression, pos);
            }
            return update_expression;
        }
        let unary_operator = self.token;
        let simple_unary_expression = self.parse_simple_unary_expression();
        if self.token == Kind::AsteriskAsteriskToken {
            let pos = scanner::skip_trivia(self.source_text, self.node(simple_unary_expression).pos);
            let end = self.node(simple_unary_expression).end;
            if self.kind(simple_unary_expression) == Kind::TypeAssertionExpression {
                self.parse_error_at(
                    pos,
                    end,
                    diagnostics::A_type_assertion_expression_is_not_allowed_in_the_left_hand_side_of_an_exponentiation_expression_Consider_enclosing_the_expression_in_parentheses,
                    &[],
                );
            } else {
                self.parse_error_at(
                    pos,
                    end,
                    diagnostics::An_unary_expression_with_the_0_operator_is_not_allowed_in_the_left_hand_side_of_an_exponentiation_expression_Consider_enclosing_the_expression_in_parentheses,
                    &[token_to_string(unary_operator)],
                );
            }
        }
        simple_unary_expression
    }

    pub(crate) fn is_update_expression(&self) -> bool {
        match self.token {
            Kind::PlusToken
            | Kind::MinusToken
            | Kind::TildeToken
            | Kind::ExclamationToken
            | Kind::DeleteKeyword
            | Kind::TypeOfKeyword
            | Kind::VoidKeyword
            | Kind::AwaitKeyword => false,
            Kind::LessThanToken => self.jsx,
            _ => true,
        }
    }

    pub(crate) fn parse_update_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        if self.token == Kind::PlusPlusToken || self.token == Kind::MinusMinusToken {
            let operator = self.token;
            self.next_token();
            let operand = self.parse_left_hand_side_expression_or_higher();
            return self.finish_node(Node::new(Kind::PrefixUnaryExpression).op(operator).expression(operand), pos);
        } else if self.jsx
            && self.token == Kind::LessThanToken
            && self.look_ahead(Self::next_token_is_identifier_or_keyword_or_greater_than)
        {
            // JSXElement is part of primaryExpression
            return self.parse_jsx_element_or_self_closing_element_or_fragment(true, None, None, false);
        }
        let expression = self.parse_left_hand_side_expression_or_higher();
        if (self.token == Kind::PlusPlusToken || self.token == Kind::MinusMinusToken) && !self.has_preceding_line_break() {
            let operator = self.token;
            self.next_token();
            return self.finish_node(Node::new(Kind::PostfixUnaryExpression).op(operator).expression(expression), pos);
        }
        expression
    }

    /// `topInvalidNodePosition` is `-1` in Go when absent.
    pub(crate) fn parse_jsx_element_or_self_closing_element_or_fragment(
        &mut self,
        in_expression_context: bool,
        top_invalid_node_position: Option<usize>,
        opening_tag: Option<NodeId>,
        must_be_unary: bool,
    ) -> NodeId {
        let pos = self.node_pos();
        let opening = self.parse_jsx_opening_or_self_closing_element_or_opening_fragment(in_expression_context);
        let mut result = match self.kind(opening) {
            Kind::JsxOpeningElement => {
                let mut children = self.parse_jsx_children(opening);
                let closing_element;
                let last_child = children.nodes.last().copied();
                if let Some(last_child) = last_child
                    && self.kind(last_child) == Kind::JsxElement
                    && !self.tag_names_are_equivalent(
                        jsx_tag_name(self, jsx_element_opening(self, last_child)),
                        jsx_tag_name(self, jsx_element_closing(self, last_child)),
                    )
                    && self.tag_names_are_equivalent(
                        jsx_tag_name(self, opening),
                        jsx_tag_name(self, jsx_element_closing(self, last_child)),
                    )
                {
                    // An unclosed JsxOpeningElement took its parent's JsxClosingElement: restructure
                    // (<div>(...<span>...</div>)) --> (<div>(...<span>...</>)</div>); the parent reports.
                    let last_opening = jsx_element_opening(self, last_child);
                    let last_children = jsx_element_children(self, last_child);
                    let end = last_children.end;
                    let missing = self.new_identifier(String::new());
                    let missing_identifier = self.finish_node_with_end(missing, end, end);
                    let new_closing_element =
                        self.finish_node_with_end(Node::new(Kind::JsxClosingElement).name(missing_identifier), end, end);
                    let last_opening_pos = self.node(last_opening).pos;
                    let new_last = self.finish_node_with_end(
                        Node::new(Kind::JsxElement).child(last_opening).list(last_children).child(new_closing_element),
                        last_opening_pos,
                        end,
                    );
                    let mut nodes = children.nodes.clone();
                    nodes.pop();
                    nodes.push(new_last);
                    let new_last_end = self.node(new_last).end;
                    children = NodeList::new(children.pos, new_last_end, nodes);
                    closing_element = jsx_element_closing(self, last_child);
                } else {
                    closing_element = self.parse_jsx_closing_element(opening, in_expression_context);
                    let opening_tag_name = jsx_tag_name(self, opening);
                    let closing_tag_name = jsx_tag_name(self, closing_element);
                    if !self.tag_names_are_equivalent(opening_tag_name, closing_tag_name) {
                        if let Some(opening_tag) = opening_tag
                            && self.kind(opening_tag) == Kind::JsxOpeningElement
                            && self.tag_names_are_equivalent(closing_tag_name, jsx_tag_name(self, opening_tag))
                        {
                            // opening incorrectly matched with its parent's closing -- put error on opening
                            let text = text_of_node(self, opening_tag_name);
                            let (start, end) = (self.node(opening_tag_name).pos, self.node(opening_tag_name).end);
                            self.parse_error_at_range(
                                start,
                                end,
                                diagnostics::JSX_element_0_has_no_corresponding_closing_tag,
                                &[text.as_str()],
                            );
                        } else {
                            // other opening/closing mismatches -- put error on closing
                            let text = text_of_node(self, opening_tag_name);
                            let (start, end) = (self.node(closing_tag_name).pos, self.node(closing_tag_name).end);
                            self.parse_error_at_range(
                                start,
                                end,
                                diagnostics::Expected_corresponding_JSX_closing_tag_for_0,
                                &[text.as_str()],
                            );
                        }
                    }
                }
                let node = Node::new(Kind::JsxElement).child(opening).list(children).child(closing_element);
                self.finish_node(node, pos)
            }
            Kind::JsxOpeningFragment => {
                let children = self.parse_jsx_children(opening);
                let closing = self.parse_jsx_closing_fragment(in_expression_context);
                self.finish_node(Node::new(Kind::JsxFragment).child(opening).list(children).child(closing), pos)
            }
            Kind::JsxSelfClosingElement => opening,
            _ => unreachable!("Unhandled case in parseJsxElementOrSelfClosingElementOrFragment"),
        };
        // `<div></div><div></div>` in an expression context: speculatively parse the second element and
        // wrap both in a synthetic comma expression for a better error. Not possible in a unary context.
        if !must_be_unary && in_expression_context && self.token == Kind::LessThanToken {
            let top_bad_pos = top_invalid_node_position.unwrap_or(self.node(result).pos);
            let invalid_element =
                self.parse_jsx_element_or_self_closing_element_or_fragment(true, Some(top_bad_pos), None, false);
            // port: Go builds this token with NewToken and sets its Loc without finishNode.
            let invalid_pos = self.node(invalid_element).pos;
            let mut comma = Node::new(Kind::CommaToken);
            comma.pos = invalid_pos;
            comma.end = invalid_pos;
            let operator_token = self.nodes.len() as NodeId;
            self.nodes.push(comma);
            let start = scanner::skip_trivia(self.source_text, top_bad_pos);
            let end = self.node(invalid_element).end;
            self.parse_error_at(start, end, diagnostics::JSX_expressions_must_have_one_parent_element, &[]);
            let node = Node::new(Kind::BinaryExpression)
                .modifiers(None)
                .child(result)
                .child(operator_token)
                .child(invalid_element)
                .op(Kind::CommaToken);
            result = self.finish_node(node, pos);
        }
        result
    }

    pub(crate) fn parse_jsx_children(&mut self, opening_tag: NodeId) -> NodeList {
        let pos = self.node_pos();
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << PC::JsxChildren as u32;
        let mut list = Vec::new();
        loop {
            // port: like Go, the rescanned token is passed along without updating `self.token`.
            let current_token = self.scanner.re_scan_jsx_token(true);
            let Some(child) = self.parse_jsx_child(opening_tag, current_token) else {
                break;
            };
            list.push(child);
            if self.kind(opening_tag) == Kind::JsxOpeningElement
                && self.kind(child) == Kind::JsxElement
                && !self.tag_names_are_equivalent(
                    jsx_tag_name(self, jsx_element_opening(self, child)),
                    jsx_tag_name(self, jsx_element_closing(self, child)),
                )
                && self.tag_names_are_equivalent(
                    jsx_tag_name(self, opening_tag),
                    jsx_tag_name(self, jsx_element_closing(self, child)),
                )
            {
                // stop after a mismatched child like <div>...(<span></div>) to reattach the </div> higher
                break;
            }
        }
        self.parsing_contexts = save_parsing_contexts;
        NodeList::new(pos, self.node_pos(), list)
    }

    pub(crate) fn parse_jsx_child(&mut self, opening_tag: NodeId, token: Kind) -> Option<NodeId> {
        match token {
            Kind::EndOfFile => {
                // Report at the tag that lacks its closing element rather than at the end of the file.
                if self.kind(opening_tag) == Kind::JsxOpeningFragment {
                    let (start, end) = (self.node(opening_tag).pos, self.node(opening_tag).end);
                    self.parse_error_at_range(start, end, diagnostics::JSX_fragment_has_no_corresponding_closing_tag, &[]);
                } else {
                    // Cover only 'Foo.Bar' in < Foo.Bar >, or only 'Foo' in < Foo >.
                    let tag = jsx_tag_name(self, opening_tag);
                    let (tag_pos, tag_end) = (self.node(tag).pos, self.node(tag).end);
                    let start = scanner::skip_trivia(self.source_text, tag_pos).min(tag_end);
                    let text = text_of_node(self, tag);
                    self.parse_error_at(start, tag_end, diagnostics::JSX_element_0_has_no_corresponding_closing_tag, &[text.as_str()]);
                }
                None
            }
            Kind::LessThanSlashToken | Kind::ConflictMarkerTrivia => None,
            Kind::JsxText | Kind::JsxTextAllWhiteSpaces => Some(self.parse_jsx_text()),
            Kind::OpenBraceToken => self.parse_jsx_expression(false),
            Kind::LessThanToken => {
                Some(self.parse_jsx_element_or_self_closing_element_or_fragment(false, None, Some(opening_tag), false))
            }
            _ => unreachable!("Unhandled case in parseJsxChild"),
        }
    }

    pub(crate) fn parse_jsx_text(&mut self) -> NodeId {
        let pos = self.node_pos();
        // port: `containsOnlyTriviaWhiteSpaces` (`self.token == JsxTextAllWhiteSpaces`) is not kept.
        let text = self.scanner.token_value().to_string();
        let result = Node::new(Kind::JsxText).text(text).token_flags(self.scanner.token_flags());
        self.scan_jsx_text();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_jsx_expression(&mut self, in_expression_context: bool) -> Option<NodeId> {
        let pos = self.node_pos();
        if !self.parse_expected(Kind::OpenBraceToken) {
            return None;
        }
        let mut dot_dot_dot_token = None;
        let mut expression = None;
        if self.token != Kind::CloseBraceToken {
            if !in_expression_context {
                dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
            }
            // Only an AssignmentExpression is valid here, but a comma sequence is parsed for a
            // better error in grammar checking.
            expression = Some(self.parse_expression());
        }
        if in_expression_context {
            self.parse_expected(Kind::CloseBraceToken);
        } else if self.parse_expected_without_advancing(Kind::CloseBraceToken) {
            self.scan_jsx_text();
        }
        Some(self.finish_node(Node::new(Kind::JsxExpression).child(dot_dot_dot_token).expression(expression), pos))
    }

    pub(crate) fn scan_jsx_text(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_token();
        self.token
    }

    pub(crate) fn scan_jsx_identifier(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_identifier();
        self.token
    }

    pub(crate) fn scan_jsx_attribute_value(&mut self) -> Kind {
        self.token = self.scanner.scan_jsx_attribute_value();
        self.token
    }

    pub(crate) fn parse_jsx_closing_element(&mut self, open: NodeId, in_expression_context: bool) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        let tag_name = self.parse_jsx_element_name();
        if self.parse_expected_with_diagnostic(Kind::GreaterThanToken, None, false) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context || !self.tag_names_are_equivalent(jsx_tag_name(self, open), tag_name) {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        self.finish_node(Node::new(Kind::JsxClosingElement).name(tag_name), pos)
    }

    pub(crate) fn parse_jsx_opening_or_self_closing_element_or_opening_fragment(
        &mut self,
        in_expression_context: bool,
    ) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanToken);
        if self.token == Kind::GreaterThanToken {
            // See below for explanation of scanJsxText
            self.scan_jsx_text();
            return self.finish_node(Node::new(Kind::JsxOpeningFragment), pos);
        }
        let tag_name = self.parse_jsx_element_name();
        let mut type_arguments = None;
        if !self.context_flags.has(NodeFlags::JavaScriptFile) {
            type_arguments = self.parse_type_arguments();
        }
        let attributes = self.parse_jsx_attributes();
        let result;
        if self.token == Kind::GreaterThanToken {
            // Scan the text after `>` with JSX scanning so characters like '#' are not scanning errors.
            self.scan_jsx_text();
            result = Node::new(Kind::JsxOpeningElement)
                .name(tag_name)
                .type_arguments(type_arguments)
                .child(attributes);
        } else {
            self.parse_expected(Kind::SlashToken);
            if self.parse_expected_without_advancing(Kind::GreaterThanToken) {
                if in_expression_context {
                    self.next_token();
                } else {
                    self.scan_jsx_text();
                }
            }
            result = Node::new(Kind::JsxSelfClosingElement)
                .name(tag_name)
                .type_arguments(type_arguments)
                .child(attributes);
        }
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_jsx_element_name(&mut self) -> NodeId {
        let pos = self.node_pos();
        // Only `this` is considered a primary expression here, not class/function/etc. keywords.
        let initial_expression = self.parse_jsx_tag_name();
        if self.kind(initial_expression) == Kind::JsxNamespacedName {
            // `a:b.c` is invalid; let `parseAttribute` report "unexpected :".
            return initial_expression;
        }
        let mut expression = initial_expression;
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_right_side_of_dot(true, false, false);
            expression = self.finish_node(
                Node::new(Kind::PropertyAccessExpression).expression(expression).child(None).name(name),
                pos,
            );
        }
        expression
    }

    pub(crate) fn parse_jsx_tag_name(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let is_this = self.token == Kind::ThisKeyword;
        let tag_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            return self.finish_node(Node::new(Kind::JsxNamespacedName).expression(tag_name).name(name), pos);
        }
        if is_this {
            return self.finish_node(Node::new(Kind::ThisKeyword), pos);
        }
        tag_name
    }

    pub(crate) fn parse_jsx_attributes(&mut self) -> NodeId {
        let pos = self.node_pos();
        let properties = self.parse_list(PC::JsxAttributes, Self::parse_jsx_attribute);
        self.finish_node(Node::new(Kind::JsxAttributes).list(properties), pos)
    }

    pub(crate) fn parse_jsx_attribute(&mut self) -> NodeId {
        if self.token == Kind::OpenBraceToken {
            return self.parse_jsx_spread_attribute();
        }
        let pos = self.node_pos();
        let name = self.parse_jsx_attribute_name();
        let initializer = self.parse_jsx_attribute_value();
        self.finish_node(Node::new(Kind::JsxAttribute).name(name).initializer(initializer), pos)
    }

    pub(crate) fn parse_jsx_spread_attribute(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBraceToken);
        self.parse_expected(Kind::DotDotDotToken);
        let expression = self.parse_expression();
        self.parse_expected(Kind::CloseBraceToken);
        self.finish_node(Node::new(Kind::JsxSpreadAttribute).expression(expression), pos)
    }

    pub(crate) fn parse_jsx_attribute_name(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.scan_jsx_identifier();
        let attr_name = self.parse_identifier_name_error_on_unicode_escape_sequence();
        if self.parse_optional(Kind::ColonToken) {
            self.scan_jsx_identifier();
            let name = self.parse_identifier_name_error_on_unicode_escape_sequence();
            return self.finish_node(Node::new(Kind::JsxNamespacedName).expression(attr_name).name(name), pos);
        }
        attr_name
    }

    pub(crate) fn parse_jsx_attribute_value(&mut self) -> Option<NodeId> {
        if self.token == Kind::EqualsToken {
            if self.scan_jsx_attribute_value() == Kind::StringLiteral {
                return Some(self.parse_literal_expression(false));
            }
            if self.token == Kind::OpenBraceToken {
                return self.parse_jsx_expression(true);
            }
            if self.token == Kind::LessThanToken {
                return Some(self.parse_jsx_element_or_self_closing_element_or_fragment(true, None, None, false));
            }
            self.parse_error_at_current_token(diagnostics::X_or_JSX_element_expected, &[]);
        }
        None
    }

    pub(crate) fn parse_jsx_closing_fragment(&mut self, in_expression_context: bool) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanSlashToken);
        if self.parse_expected_with_diagnostic(
            Kind::GreaterThanToken,
            Some(diagnostics::Expected_corresponding_closing_tag_for_JSX_fragment),
            false,
        ) {
            // manually advance the scanner in order to look for jsx text inside jsx
            if in_expression_context {
                self.next_token();
            } else {
                self.scan_jsx_text();
            }
        }
        self.finish_node(Node::new(Kind::JsxClosingFragment), pos)
    }

    pub(crate) fn parse_simple_unary_expression(&mut self) -> NodeId {
        match self.token {
            Kind::PlusToken | Kind::MinusToken | Kind::TildeToken | Kind::ExclamationToken => {
                self.parse_prefix_unary_expression()
            }
            Kind::DeleteKeyword => self.parse_delete_expression(),
            Kind::TypeOfKeyword => self.parse_type_of_expression(),
            Kind::VoidKeyword => self.parse_void_expression(),
            Kind::LessThanToken => {
                // In JSX, `+ <foo> bar` is not a type assertion.
                if self.jsx {
                    return self.parse_jsx_element_or_self_closing_element_or_fragment(true, None, None, true);
                }
                self.parse_type_assertion()
            }
            Kind::AwaitKeyword => {
                if self.is_await_expression() {
                    return self.parse_await_expression();
                }
                self.parse_update_expression()
            }
            _ => self.parse_update_expression(),
        }
    }

    pub(crate) fn parse_prefix_unary_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let operator = self.token;
        self.next_token();
        let operand = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::PrefixUnaryExpression).op(operator).expression(operand), pos)
    }

    pub(crate) fn parse_delete_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::DeleteExpression).expression(expression), pos)
    }

    pub(crate) fn parse_type_of_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::TypeOfExpression).expression(expression), pos)
    }

    pub(crate) fn parse_void_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::VoidExpression).expression(expression), pos)
    }

    pub(crate) fn is_await_expression(&mut self) -> bool {
        if self.token == Kind::AwaitKeyword {
            if self.in_await_context() {
                return true;
            }
            // here we are using similar heuristics as 'isYieldExpression'
            return self.look_ahead(Self::next_token_is_identifier_or_keyword_or_literal_on_same_line);
        }
        false
    }

    pub(crate) fn parse_await_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.next_token();
        let expression = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::AwaitExpression).expression(expression), pos)
    }

    pub(crate) fn parse_type_assertion(&mut self) -> NodeId {
        debug_assert!(!self.jsx, "Type assertions should never be parsed in JSX");
        let pos = self.node_pos();
        self.parse_expected(Kind::LessThanToken);
        let type_node = self.parse_type();
        self.parse_expected(Kind::GreaterThanToken);
        let expression = self.parse_simple_unary_expression();
        self.finish_node(Node::new(Kind::TypeAssertionExpression).ty(type_node).expression(expression), pos)
    }

    pub(crate) fn parse_left_hand_side_expression_or_higher(&mut self) -> NodeId {
        // Bottom out on `super`, `import` (call or meta property), or a MemberExpression, then
        // consume the rest of any CallExpression / OptionalExpression.
        let pos = self.node_pos();
        let expression;
        if self.token == Kind::ImportKeyword {
            if self.look_ahead(Self::next_token_is_open_paren_or_less_than) {
                // Only `import(` / `import<` is an import call; otherwise `import` starts a statement.
                self.source_flags |= NodeFlags::PossiblyContainsDynamicImport;
                expression = self.parse_keyword_expression();
            } else if self.look_ahead(Self::next_token_is_dot) {
                // This is an 'import.*' metaproperty (i.e. 'import.meta')
                self.next_token(); // advance past the 'import'
                self.next_token(); // advance past the dot
                let name = self.parse_identifier_name();
                expression = self.finish_node(Node::new(Kind::MetaProperty).op(Kind::ImportKeyword).name(name), pos);
                if self.node(name).text == "defer" {
                    if self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
                        self.source_flags |= NodeFlags::PossiblyContainsDynamicImport;
                    }
                } else {
                    self.source_flags |= NodeFlags::PossiblyContainsImportMeta;
                }
            } else {
                expression = self.parse_member_expression_or_higher();
            }
        } else if self.token == Kind::SuperKeyword {
            expression = self.parse_super_expression();
        } else {
            expression = self.parse_member_expression_or_higher();
        }
        self.parse_call_expression_rest(pos, expression)
    }

    pub(crate) fn next_token_is_dot(&mut self) -> bool {
        self.next_token() == Kind::DotToken
    }

    pub(crate) fn parse_super_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let mut expression = self.parse_keyword_expression();
        if self.token == Kind::LessThanToken {
            let start_pos = self.node_pos();
            let type_arguments = self.try_parse_type_arguments_in_expression();
            if type_arguments.is_some() {
                let end = self.node_pos();
                self.parse_error_at(start_pos, end, diagnostics::X_super_may_not_use_type_arguments, &[]);
                if !self.is_template_start_of_tagged_template() {
                    // typescript@6 never finishes this node, so the compiler API
                    // sees it at 0-0; tsgo spans it from `super`. A call absorbs
                    // it, so only `super<T>.m` and `super<T>[k]` keep it.
                    expression = self.finish_node_with_end(
                        Node::new(Kind::ExpressionWithTypeArguments).expression(expression).type_arguments(type_arguments),
                        0,
                        0,
                    );
                }
            }
        }
        if self.token == Kind::OpenParenToken || self.token == Kind::DotToken || self.token == Kind::OpenBracketToken {
            return expression;
        }
        // `super` must be followed by '(' or '.'; parse a '.' anyway and report.
        self.parse_error_at_current_token(diagnostics::X_super_must_be_followed_by_an_argument_list_or_member_access, &[]);
        // private names will never work with `super` (`super.#foo`), but that's a semantic error, not syntactic
        let name = self.parse_right_side_of_dot(true, true, true);
        self.finish_node(
            Node::new(Kind::PropertyAccessExpression).expression(expression).child(None).name(name),
            pos,
        )
    }

    pub(crate) fn is_template_start_of_tagged_template(&self) -> bool {
        self.token == Kind::NoSubstitutionTemplateLiteral || self.token == Kind::TemplateHead
    }

    pub(crate) fn try_parse_type_arguments_in_expression(&mut self) -> Option<NodeList> {
        // Never in JavaScript files (ambiguous with binary operators); and only `<` or `<<` (which
        // reScanLessThanToken splits) can start a type argument list.
        if self.context_flags.has(NodeFlags::JavaScriptFile)
            || (self.token != Kind::LessThanToken && self.token != Kind::LessThanLessThanToken)
        {
            return None;
        }
        let state = self.mark();
        if self.re_scan_less_than_token() == Kind::LessThanToken {
            self.next_token();
            let type_arguments = self.parse_delimited_list(PC::TypeArguments, Self::parse_type);
            // Without the closing `>` it is definitely not a type argument list.
            if self.re_scan_greater_than_token() == Kind::GreaterThanToken {
                self.next_token();
                if self.can_follow_type_arguments_in_expression() {
                    return Some(type_arguments);
                }
            }
        }
        self.rewind(state);
        None
    }

    pub(crate) fn can_follow_type_arguments_in_expression(&mut self) -> bool {
        match self.token {
            // foo<x>(   foo<T> `...`   foo<T> `...${100}...`
            Kind::OpenParenToken | Kind::NoSubstitutionTemplateLiteral | Kind::TemplateHead => return true,
            // `<` never makes sense, `>` is ambiguous with a rescanned `>>`, and `+`/`-` are unary here.
            Kind::LessThanToken | Kind::GreaterThanToken | Kind::PlusToken | Kind::MinusToken => return false,
            _ => {}
        }
        // Favor type arguments before a line break, a binary operator, or something that can't start an expression.
        self.has_preceding_line_break() || self.is_binary_operator() || !self.is_start_of_expression()
    }

    pub(crate) fn parse_member_expression_or_higher(&mut self) -> NodeId {
        let pos = self.node_pos();
        let expression = self.parse_primary_expression();
        self.parse_member_expression_rest(pos, expression, true)
    }

    pub(crate) fn parse_member_expression_rest(
        &mut self,
        pos: usize,
        mut expression: NodeId,
        allow_optional_chain: bool,
    ) -> NodeId {
        loop {
            let mut question_dot_token = None;
            let is_property_access;
            if allow_optional_chain && self.is_start_of_optional_property_or_element_access_chain() {
                question_dot_token = Some(self.parse_expected_token(Kind::QuestionDotToken));
                is_property_access = token_is_identifier_or_keyword(self.token);
            } else {
                is_property_access = self.parse_optional(Kind::DotToken);
            }
            if is_property_access {
                expression = self.parse_property_access_expression_rest(pos, expression, question_dot_token);
                continue;
            }
            // In a [Decorator] context ElementAccess is not parsed: it could be a ComputedPropertyName.
            if (question_dot_token.is_some() || !self.in_decorator_context()) && self.parse_optional(Kind::OpenBracketToken)
            {
                expression = self.parse_element_access_expression_rest(pos, expression, question_dot_token);
                continue;
            }
            if self.is_template_start_of_tagged_template() {
                // Absorb type arguments into TemplateExpression when preceding expression is ExpressionWithTypeArguments
                if question_dot_token.is_none() && self.kind(expression) == Kind::ExpressionWithTypeArguments {
                    let original_expression = self.node(expression).expression.expect("an expression");
                    let original_type_arguments = self.node(expression).type_arguments.clone();
                    expression = self.parse_tagged_template_rest(
                        pos,
                        original_expression,
                        question_dot_token,
                        original_type_arguments.clone(),
                    );
                    self.unparse_expression_with_type_arguments(
                        Some(original_expression),
                        original_type_arguments.as_ref(),
                        expression,
                    );
                } else {
                    expression = self.parse_tagged_template_rest(pos, expression, question_dot_token, None);
                }
                continue;
            }
            if question_dot_token.is_none() {
                if self.token == Kind::ExclamationToken && !self.has_preceding_line_break() {
                    self.next_token();
                    let node = self.finish_node(Node::new(Kind::NonNullExpression).expression(expression), pos);
                    expression = self.check_js_syntax(node);
                    continue;
                }
                let type_arguments = self.try_parse_type_arguments_in_expression();
                if type_arguments.is_some() {
                    expression = self.finish_node(
                        Node::new(Kind::ExpressionWithTypeArguments).expression(expression).type_arguments(type_arguments),
                        pos,
                    );
                    continue;
                }
            }
            return expression;
        }
    }

    pub(crate) fn is_start_of_optional_property_or_element_access_chain(&mut self) -> bool {
        self.token == Kind::QuestionDotToken
            && self.look_ahead(Self::next_token_is_identifier_or_keyword_or_open_bracket_or_template)
    }

    pub(crate) fn next_token_is_identifier_or_keyword_or_open_bracket_or_template(&mut self) -> bool {
        self.next_token();
        token_is_identifier_or_keyword(self.token)
            || self.token == Kind::OpenBracketToken
            || self.is_template_start_of_tagged_template()
    }

    pub(crate) fn parse_property_access_expression_rest(
        &mut self,
        pos: usize,
        expression: NodeId,
        question_dot_token: Option<NodeId>,
    ) -> NodeId {
        let name = self.parse_right_side_of_dot(true, true, true);
        let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
        let property_access = Node::new(Kind::PropertyAccessExpression)
            .expression(expression)
            .child(question_dot_token)
            .name(name)
            .flags(if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        if is_optional_chain && self.kind(name) == Kind::PrivateIdentifier {
            let (start, end) = self.skip_range_trivia(self.node(name).pos, self.node(name).end);
            self.parse_error_at_range(start, end, diagnostics::An_optional_chain_cannot_contain_private_identifiers, &[]);
        }
        if self.kind(expression) == Kind::ExpressionWithTypeArguments
            && let Some(type_arguments) = self.node(expression).type_arguments.clone()
        {
            let start = type_arguments.pos - 1;
            let end = scanner::skip_trivia(self.source_text, type_arguments.end) + 1;
            self.parse_error_at_range(
                start,
                end,
                diagnostics::An_instantiation_expression_cannot_be_followed_by_a_property_access,
                &[],
            );
        }
        self.finish_node(property_access, pos)
    }

    pub(crate) fn try_reparse_optional_chain(&mut self, node: NodeId) -> bool {
        if self.node(node).flags.has(NodeFlags::OptionalChain) {
            return true;
        }
        // check for an optional chain in a non-null expression
        if self.kind(node) == Kind::NonNullExpression {
            let mut expr = self.node(node).expression.expect("a non-null expression has an operand");
            while self.kind(expr) == Kind::NonNullExpression && !self.node(expr).flags.has(NodeFlags::OptionalChain) {
                expr = self.node(expr).expression.expect("a non-null expression has an operand");
            }
            if self.node(expr).flags.has(NodeFlags::OptionalChain) {
                // this is part of an optional chain. Walk down from `node` to `expression` and set the flag.
                let mut node = node;
                while self.kind(node) == Kind::NonNullExpression {
                    self.node_mut(node).flags |= NodeFlags::OptionalChain;
                    node = self.node(node).expression.expect("a non-null expression has an operand");
                }
                return true;
            }
        }
        false
    }

    pub(crate) fn parse_element_access_expression_rest(
        &mut self,
        pos: usize,
        expression: NodeId,
        question_dot_token: Option<NodeId>,
    ) -> NodeId {
        let argument_expression;
        if self.token == Kind::CloseBracketToken {
            let node_pos = self.node_pos();
            self.parse_error_at(node_pos, node_pos, diagnostics::An_element_access_expression_should_take_an_argument, &[]);
            argument_expression = self.create_missing_identifier();
        } else {
            // port: Go interns the text of a string/template/numeric argument here; no effect on diagnostics.
            argument_expression = self.parse_expression_allow_in();
        }
        self.parse_expected(Kind::CloseBracketToken);
        let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
        let node = Node::new(Kind::ElementAccessExpression)
            .expression(expression)
            .child(question_dot_token)
            .child(argument_expression)
            .flags(if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_call_expression_rest(&mut self, pos: usize, mut expression: NodeId) -> NodeId {
        loop {
            expression = self.parse_member_expression_rest(pos, expression, true);
            let mut type_arguments = None;
            let question_dot_token = self.parse_optional_token(Kind::QuestionDotToken);
            if question_dot_token.is_some() {
                type_arguments = self.try_parse_type_arguments_in_expression();
                if self.is_template_start_of_tagged_template() {
                    expression = self.parse_tagged_template_rest(pos, expression, question_dot_token, type_arguments);
                    continue;
                }
            }
            if type_arguments.is_some() || self.token == Kind::OpenParenToken {
                // Absorb type arguments into CallExpression when preceding expression is ExpressionWithTypeArguments
                if question_dot_token.is_none() && self.kind(expression) == Kind::ExpressionWithTypeArguments {
                    type_arguments = self.node(expression).type_arguments.clone();
                    expression = self.node(expression).expression.expect("an expression");
                }
                let inner = expression;
                let argument_list = self.parse_argument_list();
                let is_optional_chain = question_dot_token.is_some() || self.try_reparse_optional_chain(expression);
                let node = Node::new(Kind::CallExpression)
                    .expression(expression)
                    .child(question_dot_token)
                    .type_arguments(type_arguments.clone())
                    .list(argument_list)
                    .flags(if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
                let finished = self.finish_node(node, pos);
                expression = self.check_js_syntax(finished);
                self.unparse_expression_with_type_arguments(Some(inner), type_arguments.as_ref(), expression);
                continue;
            }
            if let Some(question_dot_token) = question_dot_token {
                // We parsed `?.` but then failed to parse anything, so report a missing identifier here.
                self.parse_error_at_current_token(diagnostics::Identifier_expected, &[]);
                let name = self.create_missing_identifier();
                expression = self.finish_node(
                    Node::new(Kind::PropertyAccessExpression)
                        .expression(expression)
                        .child(question_dot_token)
                        .name(name)
                        .flags(NodeFlags::OptionalChain),
                    pos,
                );
            }
            break;
        }
        expression
    }

    pub(crate) fn parse_argument_list(&mut self) -> NodeList {
        self.parse_expected(Kind::OpenParenToken);
        let result = self.parse_delimited_list(PC::ArgumentExpressions, Self::parse_argument_expression);
        self.parse_expected(Kind::CloseParenToken);
        result
    }

    pub(crate) fn parse_argument_expression(&mut self) -> NodeId {
        self.do_in_context(
            NodeFlags::DisallowInContext | NodeFlags::DecoratorContext,
            false,
            Self::parse_argument_or_array_literal_element,
        )
    }

    pub(crate) fn parse_argument_or_array_literal_element(&mut self) -> NodeId {
        match self.token {
            Kind::DotDotDotToken => return self.parse_spread_element(),
            Kind::CommaToken => {
                let pos = self.node_pos();
                return self.finish_node(Node::new(Kind::OmittedExpression), pos);
            }
            _ => {}
        }
        self.parse_assignment_expression_or_higher()
    }

    pub(crate) fn parse_spread_element(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::DotDotDotToken);
        let expression = self.parse_assignment_expression_or_higher();
        self.finish_node(Node::new(Kind::SpreadElement).expression(expression), pos)
    }

    pub(crate) fn parse_tagged_template_rest(
        &mut self,
        pos: usize,
        tag: NodeId,
        question_dot_token: Option<NodeId>,
        type_arguments: Option<NodeList>,
    ) -> NodeId {
        let template = if self.token == Kind::NoSubstitutionTemplateLiteral {
            self.re_scan_template_token(true);
            self.parse_literal_expression(false)
        } else {
            self.parse_template_expression(true)
        };
        let is_optional_chain = question_dot_token.is_some() || self.node(tag).flags.has(NodeFlags::OptionalChain);
        let node = Node::new(Kind::TaggedTemplateExpression)
            .expression(tag)
            .child(question_dot_token)
            .type_arguments(type_arguments)
            .body(template)
            .flags(if is_optional_chain { NodeFlags::OptionalChain } else { NodeFlags::None });
        let result = self.finish_node(node, pos);
        self.check_js_syntax(result)
    }

    pub(crate) fn parse_template_expression(&mut self, is_tagged_template: bool) -> NodeId {
        let pos = self.node_pos();
        let head = self.parse_template_head(is_tagged_template);
        let spans = self.parse_template_spans(is_tagged_template);
        self.finish_node(Node::new(Kind::TemplateExpression).child(head).list(spans), pos)
    }

    pub(crate) fn parse_template_spans(&mut self, is_tagged_template: bool) -> NodeList {
        let pos = self.node_pos();
        let mut list = Vec::new();
        loop {
            let span = self.parse_template_span(is_tagged_template);
            list.push(span);
            // slot: TemplateSpan is `.expression(expression)`, `.child(literal)`.
            let literal = self.node(span).children[0].expect("a template span has a literal");
            if self.kind(literal) != Kind::TemplateMiddle {
                break;
            }
        }
        NodeList::new(pos, self.node_pos(), list)
    }

    pub(crate) fn parse_template_span(&mut self, is_tagged_template: bool) -> NodeId {
        let pos = self.node_pos();
        let expression = self.parse_expression_allow_in();
        let literal = self.parse_literal_of_template_span(is_tagged_template);
        self.finish_node(Node::new(Kind::TemplateSpan).expression(expression).child(literal), pos)
    }

    pub(crate) fn parse_primary_expression(&mut self) -> NodeId {
        match self.token {
            Kind::NoSubstitutionTemplateLiteral | Kind::NumericLiteral | Kind::BigIntLiteral | Kind::StringLiteral => {
                if self.token == Kind::NoSubstitutionTemplateLiteral
                    && self.scanner.token_flags().has(TokenFlags::IsInvalid)
                {
                    self.re_scan_template_token(false);
                }
                return self.parse_literal_expression(false);
            }
            Kind::ThisKeyword | Kind::SuperKeyword | Kind::NullKeyword | Kind::TrueKeyword | Kind::FalseKeyword => {
                return self.parse_keyword_expression();
            }
            Kind::OpenParenToken => return self.parse_parenthesized_expression(),
            Kind::OpenBracketToken => return self.parse_array_literal_expression(),
            Kind::OpenBraceToken => return self.parse_object_literal_expression(),
            Kind::AsyncKeyword => {
                // Async arrow functions are parsed earlier in parseAssignmentExpressionOrHigher;
                // `async [no LineTerminator here] function` is an async function, otherwise an identifier.
                if self.look_ahead(Self::next_token_is_function_keyword_on_same_line) {
                    return self.parse_function_expression();
                }
            }
            Kind::AtToken => return self.parse_decorated_expression(),
            Kind::ClassKeyword => return self.parse_class_expression(),
            Kind::FunctionKeyword => return self.parse_function_expression(),
            Kind::NewKeyword => return self.parse_new_expression_or_new_dot_target(),
            Kind::SlashToken | Kind::SlashEqualsToken => {
                if self.re_scan_slash_token() == Kind::RegularExpressionLiteral {
                    return self.parse_literal_expression(false);
                }
            }
            Kind::TemplateHead => return self.parse_template_expression(false),
            Kind::PrivateIdentifier => return self.parse_private_identifier(),
            _ => {}
        }
        self.parse_identifier_with_diagnostic(Some(diagnostics::Expression_expected), None)
    }

    pub(crate) fn parse_parenthesized_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected(Kind::CloseParenToken);
        self.finish_node(Node::new(Kind::ParenthesizedExpression).expression(expression), pos)
    }

    pub(crate) fn parse_array_literal_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let open_bracket_position = self.scanner.token_start();
        let open_bracket_parsed = self.parse_expected(Kind::OpenBracketToken);
        // port: `multiLine` (hasPrecedingLineBreak here) is not kept.
        let elements = self.parse_delimited_list(PC::ArrayLiteralMembers, Self::parse_argument_or_array_literal_element);
        self.parse_expected_matching_brackets(
            Kind::OpenBracketToken,
            Kind::CloseBracketToken,
            open_bracket_parsed,
            open_bracket_position,
        );
        self.finish_node(Node::new(Kind::ArrayLiteralExpression).list(elements), pos)
    }

    pub(crate) fn parse_object_literal_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let open_brace_position = self.scanner.token_start();
        let open_brace_parsed = self.parse_expected(Kind::OpenBraceToken);
        // port: `multiLine` (hasPrecedingLineBreak here) is not kept.
        let properties = self.parse_delimited_list(PC::ObjectLiteralMembers, Self::parse_object_literal_element);
        self.parse_expected_matching_brackets(
            Kind::OpenBraceToken,
            Kind::CloseBraceToken,
            open_brace_parsed,
            open_brace_position,
        );
        self.finish_node(Node::new(Kind::ObjectLiteralExpression).list(properties), pos)
    }

    pub(crate) fn parse_object_literal_element(&mut self) -> NodeId {
        let pos = self.node_pos();
        if self.parse_optional(Kind::DotDotDotToken) {
            let expression = self.parse_assignment_expression_or_higher();
            return self.finish_node(Node::new(Kind::SpreadAssignment).expression(expression), pos);
        }
        let modifiers = self.parse_modifiers_ex(true, false, false);
        if self.parse_contextual_modifier(Kind::GetKeyword) {
            return self.parse_accessor_declaration(pos, modifiers, Kind::GetAccessor, ParseFlags::None);
        }
        if self.parse_contextual_modifier(Kind::SetKeyword) {
            return self.parse_accessor_declaration(pos, modifiers, Kind::SetAccessor, ParseFlags::None);
        }
        let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
        let token_is_identifier = self.is_identifier();
        let name = self.parse_property_name();
        // Optional property assignments and definite assignment assertions are reported by the grammar checker.
        let mut postfix_token = self.parse_optional_token(Kind::QuestionToken);
        if postfix_token.is_none() {
            postfix_token = self.parse_optional_token(Kind::ExclamationToken);
        }
        if asterisk_token.is_some() || self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
            return self.parse_method_declaration(pos, modifiers, asterisk_token, name, postfix_token, None);
        }
        // `=` after an identifier is the CoverInitializedName production (object assignment patterns).
        let node;
        let is_shorthand_property_assignment = token_is_identifier && self.token != Kind::ColonToken;
        if is_shorthand_property_assignment {
            let equals_token = self.parse_optional_token(Kind::EqualsToken);
            let mut initializer = None;
            if equals_token.is_some() {
                initializer = Some(self.do_in_context(
                    NodeFlags::DisallowInContext,
                    false,
                    Self::parse_assignment_expression_or_higher,
                ));
            }
            node = Node::new(Kind::ShorthandPropertyAssignment)
                .modifiers(modifiers)
                .name(name)
                .question(postfix_token)
                .ty(None)
                .child(equals_token)
                .initializer(initializer);
        } else {
            self.parse_expected(Kind::ColonToken);
            let initializer =
                self.do_in_context(NodeFlags::DisallowInContext, false, Self::parse_assignment_expression_or_higher);
            node = Node::new(Kind::PropertyAssignment)
                .modifiers(modifiers)
                .name(name)
                .question(postfix_token)
                .ty(None)
                .initializer(initializer);
        }
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_function_expression(&mut self) -> NodeId {
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::DecoratorContext, false);
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers();
        self.parse_expected(Kind::FunctionKeyword);
        let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
        let is_generator = asterisk_token.is_some();
        let is_async = modifier_list_has_async(self, &modifiers);
        let signature_flags = (if is_generator { ParseFlags::Yield } else { ParseFlags::None })
            | (if is_async { ParseFlags::Await } else { ParseFlags::None });
        let name = match (is_generator, is_async) {
            (true, true) => self.do_in_context(
                NodeFlags::YieldContext | NodeFlags::AwaitContext,
                true,
                Self::parse_optional_binding_identifier,
            ),
            (true, false) => self.do_in_context(NodeFlags::YieldContext, true, Self::parse_optional_binding_identifier),
            (false, true) => self.do_in_context(NodeFlags::AwaitContext, true, Self::parse_optional_binding_identifier),
            (false, false) => self.parse_optional_binding_identifier(),
        };
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters(signature_flags);
        let return_type = self.parse_return_type(Kind::ColonToken, false);
        let body = self.parse_function_block(signature_flags, None);
        self.context_flags = save_context_flags;
        let node = Node::new(Kind::FunctionExpression)
            .modifiers(modifiers)
            .child(asterisk_token)
            .name(name)
            .type_parameters(type_parameters)
            .parameters(parameters)
            .ty(return_type)
            .body(body);
        let result = self.finish_node(node, pos);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_optional_binding_identifier(&mut self) -> Option<NodeId> {
        if self.is_binding_identifier() {
            return Some(self.parse_binding_identifier());
        }
        None
    }

    pub(crate) fn parse_decorated_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let modifiers = self.parse_modifiers_ex(true, false, false);
        if self.token == Kind::ClassKeyword {
            return self.parse_class_declaration_or_expression(pos, modifiers, Kind::ClassExpression);
        }
        let node_pos = self.node_pos();
        self.parse_error_at(node_pos, node_pos, diagnostics::Expression_expected, &[]);
        self.finish_node(Node::new(Kind::MissingDeclaration).modifiers(modifiers), pos)
    }

    /// port: Go only resets `.Parent` pointers here; the arena keeps no parents.
    pub(crate) fn unparse_expression_with_type_arguments(
        &mut self,
        _expression: Option<NodeId>,
        _type_arguments: Option<&NodeList>,
        _result: NodeId,
    ) {
    }

    pub(crate) fn parse_new_expression_or_new_dot_target(&mut self) -> NodeId {
        let pos = self.node_pos();
        self.parse_expected(Kind::NewKeyword);
        if self.parse_optional(Kind::DotToken) {
            let name = self.parse_identifier_name();
            return self.finish_node(Node::new(Kind::MetaProperty).op(Kind::NewKeyword).name(name), pos);
        }
        let expression_pos = self.node_pos();
        let primary = self.parse_primary_expression();
        let mut expression = self.parse_member_expression_rest(expression_pos, primary, false);
        let mut type_arguments = None;
        // Absorb type arguments into NewExpression when preceding expression is ExpressionWithTypeArguments
        if self.kind(expression) == Kind::ExpressionWithTypeArguments {
            type_arguments = self.node(expression).type_arguments.clone();
            expression = self.node(expression).expression.expect("an expression");
        }
        if self.token == Kind::QuestionDotToken {
            let text = text_of_node(self, expression);
            self.parse_error_at_current_token(
                diagnostics::Invalid_optional_chain_from_new_expression_Did_you_mean_to_call_0,
                &[text.as_str()],
            );
        }
        let mut argument_list = None;
        if self.token == Kind::OpenParenToken {
            argument_list = Some(self.parse_argument_list());
        }
        let node = Node::new(Kind::NewExpression)
            .expression(expression)
            .child(None)
            .type_arguments(type_arguments.clone())
            .list(argument_list);
        let finished = self.finish_node(node, pos);
        let result = self.check_js_syntax(finished);
        self.unparse_expression_with_type_arguments(Some(expression), type_arguments.as_ref(), result);
        result
    }

    pub(crate) fn parse_keyword_expression(&mut self) -> NodeId {
        let pos = self.node_pos();
        let result = Node::new(self.token);
        self.next_token();
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_literal_expression(&mut self, _intern: bool) -> NodeId {
        let pos = self.node_pos();
        // port: interning is dropped; it does not affect diagnostics.
        let text = self.scanner.token_value().to_string();
        let token_flags = self.scanner.token_flags();
        let result = match self.token {
            Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::RegularExpressionLiteral
            | Kind::NoSubstitutionTemplateLiteral => Node::new(self.token).text(text).token_flags(token_flags),
            _ => unreachable!("Unhandled case in parseLiteralExpression"),
        };
        self.next_token();
        self.finish_node(result, pos)
    }
}
