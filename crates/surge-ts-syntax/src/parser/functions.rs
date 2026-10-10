use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, BindingProperty, BlockStatement,
    CatchClause, Declaration, DoWhileStatement, Expression, ExpressionStatement, ForInStatement, ForOfStatement,
    ForStatement, ForStatementInit, ForStatementLeft, FormalParameter, Function, IfStatement,
    ObjectPattern, Statement, SwitchCase, SwitchStatement, ThrowStatement,
    TryStatement, VariableDeclaration, WhileStatement,
};
use oxc_span::GetSpan;

use crate::{
    ParsedArrayBindingPattern, ParsedBindingName, ParsedExpression, ParsedForOfStatement,
    ParsedFunctionBodyStatement, ParsedFunctionDeclaration, ParsedFunctionParameter,
    ParsedIfStatement, ParsedObjectBindingElement, ParsedObjectBindingPattern,
    ParsedReturnStatement, ParsedSwitchCase, ParsedSwitchStatement, ParsedThisPropertyAssignment,
    ParsedThrowStatement, ParsedTryStatement, ParsedWhileStatement,
};

use super::expressions::parse_expression;
use super::spans::text_span_from_oxc_span;
use super::types::parse_type_annotation;

pub(crate) fn parse_function_declaration(
    function: &Function<'_>,
) -> Option<ParsedFunctionDeclaration> {
    let id = function.id.as_ref()?;
    parse_function_declaration_named(
        function,
        id.name.to_string(),
        Some(text_span_from_oxc_span(id.span)),
    )
}

/// A function declaration with the overload declarations its JSDoc
/// `@overload` tags write before it, as a TypeScript file writes them.
pub(crate) fn parse_function_declarations(function: &Function<'_>) -> Vec<ParsedFunctionDeclaration> {
    let Some(implementation) = parse_function_declaration(function) else {
        return Vec::new();
    };
    let mut declarations: Vec<ParsedFunctionDeclaration> = super::jsdoc::overloads_at(function.span.start)
        .into_iter()
        .filter_map(|(signature, span)| jsdoc_overload_declaration(&implementation.name, signature, span))
        .collect();
    declarations.push(implementation);
    declarations
}

fn jsdoc_overload_declaration(
    name: &str,
    signature: crate::ParsedType,
    span: crate::TextSpan,
) -> Option<ParsedFunctionDeclaration> {
    let crate::ParsedType::Function(signature) = signature else {
        return None;
    };
    let this_parameter_type = signature
        .parameters
        .iter()
        .find(|parameter| parameter.is_this)
        .map(|parameter| parameter.ty.clone());
    let parameters = signature
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .map(|parameter| crate::ParsedFunctionParameter {
            binding_name: crate::ParsedBindingName::Identifier {
                name: parameter.name.clone().unwrap_or_default(),
                span: parameter.name_span,
            },
            declared_type: Some(parameter.ty.clone()),
            initializer: None,
            initializer_span: None,
            optional: parameter.optional,
            untyped_javascript: false,
            rest: parameter.rest,
            is_parameter_property: false,
            is_readonly_parameter_property: false,
            is_override_parameter_property: false,
        })
        .collect();
    Some(ParsedFunctionDeclaration {
        has_this_parameter: this_parameter_type.is_some(),
        this_parameter_type,
        is_declare: false,
        name: name.to_string(),
        name_span: Some(span),
        type_parameters: Vec::new(),
        parameters,
        return_type: Some((*signature.return_type).clone()),
        return_type_span: None,
        body: Vec::new(),
        has_body: false,
        is_generator: false,
        is_async: false,
        full_signature: None,
        body_reads: Vec::new(),
    })
}

/// Parse a function whose binding name is supplied by the caller — used for an
/// anonymous `export default function () {}`, which tsc binds as `default`.
pub(crate) fn parse_function_declaration_named(
    function: &Function<'_>,
    name: String,
    name_span: Option<crate::TextSpan>,
) -> Option<ParsedFunctionDeclaration> {
    let mut parameters: Vec<_> = function
        .params
        .items
        .iter()
        .filter_map(parse_function_parameter)
        .collect();
    if let Some(rest) = function.params.rest.as_deref() {
        if let Some(rest_parameter) = parse_rest_function_parameter(rest) {
            parameters.push(rest_parameter);
        }
    }
    let (return_type, return_type_span) = super::jsdoc::annotated_or_jsdoc_return_type(
        function.return_type.as_deref(),
        function.span.start,
    );
    let body = function
        .body
        .as_ref()
        .map(|body| parse_statement_list_as_function_body(&body.statements))
        .unwrap_or_default();
    let jsdoc_this_type = function
        .this_param
        .is_none()
        .then(|| super::jsdoc::this_type_at(function.span.start))
        .flatten();

    Some(ParsedFunctionDeclaration {
        has_this_parameter: function.this_param.is_some() || jsdoc_this_type.is_some(),
        this_parameter_type: function
            .this_param
            .as_ref()
            .and_then(|this_param| this_param.type_annotation.as_ref())
            .and_then(|annotation| parse_type_annotation(annotation))
            .or(jsdoc_this_type),
        is_declare: function.declare,
        name,
        name_span,
        type_parameters: super::jsdoc::written_or_jsdoc_type_parameters(
            function.type_parameters.as_deref(),
            function.span.start,
        ),
        parameters,
        return_type,
        return_type_span,
        body,
        has_body: function.body.is_some(),
        is_generator: function.generator,
        is_async: function.r#async,
        full_signature: super::jsdoc::full_signature_at(function.span.start),
        body_reads: function
            .body
            .as_ref()
            .map(|body| super::reads::collect_function_body_reads(body))
            .unwrap_or_default(),
    })
}

fn parse_return_statement(statement: &Statement<'_>) -> Option<ParsedReturnStatement> {
    let Statement::ReturnStatement(return_statement) = statement else {
        return None;
    };

    let (expression, expression_span) = return_statement
        .argument
        .as_ref()
        .map(parse_expression)
        .map(|(expression, span)| (Some(expression), Some(text_span_from_oxc_span(span))))
        .unwrap_or((None, None));
    // `/** @type {T} */ return e` returns `e as T` (tsgo's `makeNewCast`).
    let expression = match (expression, super::jsdoc::return_cast_at(return_statement.span.start)) {
        (Some(expression), Some((ty, type_span, true))) => Some(ParsedExpression::TypeAssertion {
            expression: Box::new(expression),
            expression_span,
            ty,
            type_span: Some(type_span),
            annotation: false,
        }),
        (Some(expression), Some((ty, type_span, false))) => {
            Some(ParsedExpression::SatisfiesExpression {
                expression: Box::new(expression),
                target_type: ty,
                span: expression_span,
                target_span: Some(type_span),
                keyword_span: None,
            })
        }
        (expression, _) => expression,
    };

    Some(ParsedReturnStatement {
        expression,
        expression_span,
        span: Some(text_span_from_oxc_span(return_statement.span)),
    })
}

pub(crate) fn parse_function_body_statement(
    statement: &Statement<'_>,
) -> Option<Vec<ParsedFunctionBodyStatement>> {
    match statement {
        Statement::BlockStatement(block) => Some(vec![ParsedFunctionBodyStatement::Block(
            parse_block_statement_as_function_body(block),
        )]),
        Statement::IfStatement(if_statement) => parse_if_statement(if_statement)
            .map(|if_statement| vec![ParsedFunctionBodyStatement::If(Box::new(if_statement))]),
        Statement::WhileStatement(while_statement) => {
            parse_while_statement(while_statement).map(|while_statement| {
                vec![ParsedFunctionBodyStatement::While(Box::new(
                    while_statement,
                ))]
            })
        }
        // A label only names a `break`/`continue` target. Those are modelled
        // without a target already, so the label is dropped and its statement
        // parsed in place — before this, the whole labelled statement was
        // dropped and its body went unchecked.
        Statement::LabeledStatement(labeled_statement) => {
            parse_function_body_statement(&labeled_statement.body)
        }
        Statement::ForStatement(for_statement) => Some(parse_for_statement(for_statement)),
        Statement::ForInStatement(for_in_statement) => match parse_for_in_statement(for_in_statement) {
            Some(parsed) => Some(vec![ParsedFunctionBodyStatement::ForOf(Box::new(parsed))]),
            // A head with nothing to bind (`for (var in X)`, TS1123) still
            // evaluates its object and runs its body.
            None => {
                let mut statements = vec![ParsedFunctionBodyStatement::Expression(Box::new(
                    parse_expression(&for_in_statement.right).0,
                ))];
                statements.extend(parse_function_body_statement(&for_in_statement.body).unwrap_or_default());
                Some(statements)
            }
        },
        Statement::DoWhileStatement(do_while_statement) => {
            Some(parse_do_while_statement(do_while_statement))
        }
        Statement::ForOfStatement(for_of_statement) => parse_for_of_statement(for_of_statement)
            .map(|for_of_statement| {
                vec![ParsedFunctionBodyStatement::ForOf(Box::new(
                    for_of_statement,
                ))]
            }),
        Statement::SwitchStatement(switch_statement) => parse_switch_statement(switch_statement)
            .map(|switch_statement| {
                vec![ParsedFunctionBodyStatement::Switch(Box::new(
                    switch_statement,
                ))]
            }),
        Statement::ThrowStatement(throw_statement) => {
            parse_throw_statement(throw_statement).map(|throw_statement| {
                vec![ParsedFunctionBodyStatement::Throw(Box::new(
                    throw_statement,
                ))]
            })
        }
        Statement::TryStatement(try_statement) => parse_try_statement(try_statement)
            .map(|try_statement| vec![ParsedFunctionBodyStatement::Try(Box::new(try_statement))]),
        Statement::ReturnStatement(_) => {
            parse_return_statement(statement).map(|return_statement| {
                vec![ParsedFunctionBodyStatement::Return(Box::new(
                    return_statement,
                ))]
            })
        }
        Statement::ContinueStatement(_) => Some(vec![ParsedFunctionBodyStatement::Continue]),
        Statement::BreakStatement(_) => Some(vec![ParsedFunctionBodyStatement::Break]),
        Statement::ExpressionStatement(expression_statement) => {
            parse_expression_statement_as_function_body(expression_statement)
        }
        // `checkWithStatement` checks the object and never the body, whose
        // names may all be members of it. The body's `var`s still bind in the
        // enclosing scope, typed `any` like every symbol in the block.
        Statement::WithStatement(with) => {
            let mut statements = parse_expression_as_function_body_statements(&with.object);
            let mut hoisted = Vec::new();
            collect_with_body_vars(&with.body, &mut hoisted);
            if !hoisted.is_empty() {
                statements.get_or_insert_with(Vec::new).extend(hoisted.into_iter().map(|(name, span)| {
                    ParsedFunctionBodyStatement::VariableDeclaration(Box::new(crate::ParsedVariableDeclaration {
                        is_declare: false,
                        kind: crate::ParsedVariableKind::Var,
                        from_binding_pattern: false,
                        has_definite_assertion: false,
                        array_pattern_span: None,
                        is_enum_object: false,
                        enum_members: None,
                        array_rest_start: None,
                        name,
                        name_span: Some(text_span_from_oxc_span(span)),
                        declared_type: Some(crate::ParsedType::Any),
                        initializer: None,
                        initializer_span: None,
                        declaration_list: None,
                        annotated_pattern: None,
                        pattern_excess_properties: Vec::new(),
                    }))
                }));
            }
            statements
        }
        _ => {
            let declaration = statement.as_declaration()?;

            match declaration {
                Declaration::VariableDeclaration(declaration) => {
                    Some(parse_variable_declaration_as_function_body(declaration))
                }
                Declaration::FunctionDeclaration(function) => Some(
                    parse_function_declarations(function)
                        .into_iter()
                        .map(|function| ParsedFunctionBodyStatement::Function(Box::new(function)))
                        .collect(),
                ),
                Declaration::TSTypeAliasDeclaration(alias) => {
                    super::types::parse_type_alias_declaration(alias)
                        .map(|alias| vec![ParsedFunctionBodyStatement::TypeAlias(Box::new(alias))])
                }
                Declaration::TSInterfaceDeclaration(interface) => {
                    super::interfaces::parse_interface_declaration(interface).map(|interface| {
                        vec![ParsedFunctionBodyStatement::Interface(Box::new(interface))]
                    })
                }
                Declaration::ClassDeclaration(class) => {
                    super::classes::parse_class_declaration(class)
                        .map(|class| vec![ParsedFunctionBodyStatement::Class(Box::new(class))])
                }
                Declaration::TSEnumDeclaration(enum_declaration) => Some(
                    super::enums::parse_enum_declaration_as_function_body(enum_declaration),
                ),
                _ => None,
            }
        }
    }
}

pub(crate) fn parse_statement_list_as_function_body(
    statements: &[Statement<'_>],
) -> Vec<ParsedFunctionBodyStatement> {
    let aliases = statements
        .first()
        .map(|first| super::jsdoc::statement_list_aliases(first.span().start))
        .unwrap_or_default();
    aliases
        .into_iter()
        .chain(statements.iter().filter_map(parse_function_body_statement).flatten())
        .collect()
}

fn parse_single_statement_as_function_body(
    statement: &Statement<'_>,
) -> Vec<ParsedFunctionBodyStatement> {
    parse_function_body_statement(statement).unwrap_or_default()
}

fn parse_block_statement_as_function_body(
    block: &BlockStatement<'_>,
) -> Vec<ParsedFunctionBodyStatement> {
    parse_statement_list_as_function_body(&block.body)
}

fn parse_expression_statement_as_function_body(
    expression_statement: &ExpressionStatement<'_>,
) -> Option<Vec<ParsedFunctionBodyStatement>> {
    parse_expression_as_function_body_statements(&expression_statement.expression)
}

/// The clauses of a `for` header are expressions evaluated for effect, checked
/// like the statements they would be on their own; a comma list is one per
/// operand.
fn parse_for_clause_statements(expression: &Expression<'_>) -> Vec<ParsedFunctionBodyStatement> {
    match expression {
        Expression::SequenceExpression(sequence) => sequence
            .expressions
            .iter()
            .flat_map(parse_for_clause_statements)
            .collect(),
        Expression::ParenthesizedExpression(parenthesized) => {
            parse_for_clause_statements(&parenthesized.expression)
        }
        _ => parse_expression_as_function_body_statements(expression).unwrap_or_default(),
    }
}

fn collect_with_body_vars(statement: &Statement<'_>, out: &mut Vec<(String, oxc_span::Span)>) {
    let declare = |variable: &VariableDeclaration<'_>, out: &mut Vec<(String, oxc_span::Span)>| {
        if variable.kind == oxc_ast::ast::VariableDeclarationKind::Var {
            for declarator in variable.declarations.iter() {
                for identifier in declarator.id.get_binding_identifiers() {
                    out.push((identifier.name.to_string(), identifier.span));
                }
            }
        }
    };
    let loop_head = |left: &ForStatementLeft<'_>, out: &mut Vec<(String, oxc_span::Span)>| {
        if let ForStatementLeft::VariableDeclaration(variable) = left {
            declare(variable, out);
        }
    };
    match statement {
        Statement::VariableDeclaration(variable) => declare(variable, out),
        Statement::BlockStatement(block) => block.body.iter().for_each(|statement| collect_with_body_vars(statement, out)),
        Statement::IfStatement(statement) => {
            collect_with_body_vars(&statement.consequent, out);
            if let Some(alternate) = &statement.alternate {
                collect_with_body_vars(alternate, out);
            }
        }
        Statement::ForStatement(statement) => {
            if let Some(ForStatementInit::VariableDeclaration(variable)) = &statement.init {
                declare(variable, out);
            }
            collect_with_body_vars(&statement.body, out);
        }
        Statement::ForInStatement(statement) => {
            loop_head(&statement.left, out);
            collect_with_body_vars(&statement.body, out);
        }
        Statement::ForOfStatement(statement) => {
            loop_head(&statement.left, out);
            collect_with_body_vars(&statement.body, out);
        }
        Statement::WhileStatement(statement) => collect_with_body_vars(&statement.body, out),
        Statement::DoWhileStatement(statement) => collect_with_body_vars(&statement.body, out),
        Statement::LabeledStatement(statement) => collect_with_body_vars(&statement.body, out),
        Statement::WithStatement(statement) => collect_with_body_vars(&statement.body, out),
        Statement::TryStatement(statement) => {
            statement.block.body.iter().for_each(|statement| collect_with_body_vars(statement, out));
            if let Some(handler) = &statement.handler {
                handler.body.body.iter().for_each(|statement| collect_with_body_vars(statement, out));
            }
            if let Some(finalizer) = &statement.finalizer {
                finalizer.body.iter().for_each(|statement| collect_with_body_vars(statement, out));
            }
        }
        Statement::SwitchStatement(statement) => {
            for case in statement.cases.iter() {
                case.consequent.iter().for_each(|statement| collect_with_body_vars(statement, out));
            }
        }
        _ => {}
    }
}

fn parse_expression_as_function_body_statements(
    statement_expression: &Expression<'_>,
) -> Option<Vec<ParsedFunctionBodyStatement>> {
    let mut statements = parse_expression_as_function_body_statements_unchecked(statement_expression);
    if let Some(excess) = super::destructuring_assignment_excess(statement_expression) {
        statements
            .get_or_insert_with(Vec::new)
            .push(ParsedFunctionBodyStatement::Expression(Box::new(excess)));
    }
    statements
}

fn parse_expression_as_function_body_statements_unchecked(
    statement_expression: &Expression<'_>,
) -> Option<Vec<ParsedFunctionBodyStatement>> {
    // A comma expression's operands run in order, so a destructuring
    // assignment among them assigns its targets as one standing alone does
    // (`[a] = xs, f();`) — each operand is lowered as its own statement.
    if let Expression::SequenceExpression(sequence) = statement_expression.without_parentheses()
        && sequence
            .expressions
            .iter()
            .any(|operand| !super::parse_destructuring_assignment(operand).is_empty())
    {
        return Some(
            sequence
                .expressions
                .iter()
                .flat_map(|operand| {
                    parse_expression_as_function_body_statements(operand).unwrap_or_default()
                })
                .collect(),
        );
    }
    let destructured = super::parse_destructuring_assignment(statement_expression);
    if !destructured.is_empty() {
        return Some(
            destructured
                .into_iter()
                .map(|assignment| ParsedFunctionBodyStatement::Assignment(Box::new(assignment)))
                .collect(),
        );
    }
    match statement_expression {
        Expression::AssignmentExpression(assignment) => {
            if let Some(this_assignment) = parse_this_property_assignment(assignment) {
                return Some(vec![ParsedFunctionBodyStatement::ThisPropertyAssignment(
                    Box::new(this_assignment),
                )]);
            }

            if let Some(member_assignment) = parse_member_assignment(assignment) {
                return Some(vec![ParsedFunctionBodyStatement::MemberAssignment(
                    Box::new(member_assignment),
                )]);
            }

            if let Some(rejected) = super::expressions::parse_rejected_assignment(assignment) {
                return Some(vec![ParsedFunctionBodyStatement::Expression(Box::new(rejected))]);
            }

            super::parse_assignment_expression(assignment).map(|assignment| {
                vec![ParsedFunctionBodyStatement::Assignment(Box::new(
                    assignment,
                ))]
            })
        }
        _ => {
            let (expression, _) = parse_expression(statement_expression);

            if expression == ParsedExpression::Unknown {
                return None;
            }

            Some(vec![ParsedFunctionBodyStatement::Expression(Box::new(
                expression,
            ))])
        }
    }
}

/// `o.p = v` where the target is a member of something other than `this`.
/// Without this the whole statement was dropped, so neither the assignment's own
/// type check nor the narrowing it establishes for the code after it happened.
pub(super) fn parse_member_assignment(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<crate::ParsedMemberAssignment> {
    // The target is parsed into the same shape a *read* of it produces, so the
    // checker resolves the written member exactly as it resolves the read.
    let (target, member_span) = match &assignment.left {
        AssignmentTarget::StaticMemberExpression(member) => {
            let (object, object_span) = parse_expression(&member.object);
            if object == ParsedExpression::Unknown {
                return None;
            }
            (
                ParsedExpression::PropertyAccess {
                    object: Box::new(object),
                    object_span: Some(text_span_from_oxc_span(object_span)),
                    property_name: member.property.name.to_string(),
                    property_span: Some(text_span_from_oxc_span(member.property.span)),
                    is_bracketed: false,
                    binding_element: false,
                },
                member.span,
            )
        }
        AssignmentTarget::ComputedMemberExpression(member) => (
            super::expressions::parse_computed_member_expression(member)?,
            member.span,
        ),
        AssignmentTarget::PrivateFieldExpression(member) => {
            let target = super::expressions::parse_private_field_expression(member);
            if matches!(&target, ParsedExpression::PropertyAccess { object, .. } if **object == ParsedExpression::Unknown) {
                return None;
            }
            (target, member.span)
        }
        _ => return None,
    };

    let (value, value_span) = match super::expressions::parse_javascript_member_assignment_chain(&assignment.right) {
        Some(chain) => (chain, oxc_span::GetSpan::span(&assignment.right)),
        None => parse_expression(&assignment.right),
    };
    if value == ParsedExpression::Unknown {
        return None;
    }
    // The target as written, parentheses included: `(M.y) = ''`.
    let target_span = Some(crate::TextSpan {
        start: assignment.span.start as usize,
        end: member_span.end as usize,
    });
    let value_span = Some(text_span_from_oxc_span(value_span));
    let value = super::logical_assignment_value(
        assignment.operator,
        target.clone(),
        target_span,
        value,
        value_span,
    )?;

    Some(crate::ParsedMemberAssignment {
        target,
        target_span,
        value,
        value_span,
        declares_commonjs_export: assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign
            && super::commonjs::declares_export(&assignment.left),
    })
}

fn parse_this_property_assignment(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedThisPropertyAssignment> {
    // `this["x"] = v` with a literal key writes the same member as `this.x`.
    let (object, property_name, property_span, member_span, is_bracketed) = match &assignment.left {
        AssignmentTarget::StaticMemberExpression(member) => (
            &member.object,
            member.property.name.to_string(),
            member.property.span,
            member.span,
            false,
        ),
        AssignmentTarget::ComputedMemberExpression(member) => {
            let Expression::StringLiteral(key) = &member.expression else {
                return None;
            };
            (&member.object, key.value.to_string(), key.span, member.span, true)
        }
        AssignmentTarget::PrivateFieldExpression(member) => (
            &member.object,
            super::private_names::access_key(member.field.name.as_str()),
            member.field.span,
            member.span,
            false,
        ),
        _ => return None,
    };

    let Expression::ThisExpression(this_expression) = object else {
        return None;
    };

    let (value, value_span) = parse_expression(&assignment.right);
    let value_span = Some(text_span_from_oxc_span(value_span));
    let property_span = Some(text_span_from_oxc_span(property_span));
    let target = ParsedExpression::PropertyAccess {
        object: Box::new(ParsedExpression::This {
            span: Some(text_span_from_oxc_span(this_expression.span)),
        }),
        object_span: Some(text_span_from_oxc_span(this_expression.span)),
        property_name: property_name.clone(),
        property_span,
        is_bracketed,
        binding_element: false,
    };
    let value = super::logical_assignment_value(
        assignment.operator,
        target,
        Some(text_span_from_oxc_span(member_span)),
        value,
        value_span,
    )?;

    Some(ParsedThisPropertyAssignment {
        property_name,
        property_span,
        is_bracketed,
        target_span: Some(text_span_from_oxc_span(member_span)),
        value,
        value_span,
    })
}

fn parse_variable_declaration_as_function_body(
    declaration: &VariableDeclaration<'_>,
) -> Vec<ParsedFunctionBodyStatement> {
    super::parse_variable_declaration(declaration)
        .into_iter()
        .filter_map(|statement| match statement {
            crate::ParsedStatement::VariableDeclaration(variable) => {
                Some(ParsedFunctionBodyStatement::VariableDeclaration(variable))
            }
            // `const C = class {}`, which lowers to a class declaration.
            crate::ParsedStatement::ClassDeclaration(class) => {
                Some(ParsedFunctionBodyStatement::Class(class))
            }
            crate::ParsedStatement::Expression(expression) => {
                Some(ParsedFunctionBodyStatement::Expression(expression))
            }
            _ => None,
        })
        .collect()
}

pub(super) fn parse_if_statement(if_statement: &IfStatement<'_>) -> Option<ParsedIfStatement> {
    let (mut condition, condition_span) = parse_expression(&if_statement.test);
    clear_condition_chain_tests(&mut condition);
    let then_body = parse_branch_body(&if_statement.consequent);
    let else_body = if_statement
        .alternate
        .as_ref()
        .map(parse_branch_body)
        .unwrap_or_default();

    Some(ParsedIfStatement {
        condition,
        condition_span: Some(text_span_from_oxc_span(condition_span)),
        then_body,
        else_body,
        unreferenced_truthiness_tests: unreferenced_truthiness_tests(if_statement),
        alias_source: None,
    })
}

/// tsc's `checkTestingKnownTruthyTypes` candidates in an `if` condition. The
/// condition itself is checked (`checkIfStatement`), and so is the left operand
/// of every logical operator along it, since each one's parent walk reaches the
/// `if` (`checkBinaryLikeExpression`) — together that is every operand of the
/// condition's logical chain, each tested against the then branch.
fn unreferenced_truthiness_tests(
    if_statement: &IfStatement<'_>,
) -> Vec<crate::ParsedTruthinessTest> {
    let mut found = Vec::new();
    if_condition_operands(&if_statement.test, Vec::new(), &mut found);
    unreferenced_candidates(found, Some(&TruthinessBody::Statement(&if_statement.consequent)))
}

/// The logical operators along an `if` condition are tested against its then
/// branch by [`unreferenced_truthiness_tests`], not on their own.
fn clear_condition_chain_tests(condition: &mut ParsedExpression) {
    match condition {
        ParsedExpression::Logical {
            left,
            right,
            truthiness_tests,
            ..
        } => {
            truthiness_tests.clear();
            clear_condition_chain_tests(left);
            clear_condition_chain_tests(right);
        }
        ParsedExpression::NullishCoalescing { left, right, .. } => {
            clear_condition_chain_tests(left);
            clear_condition_chain_tests(right);
        }
        _ => {}
    }
}

/// Where a truthiness test holds: an `if`'s then branch, or a conditional
/// expression's `whenTrue` operand.
pub(crate) enum TruthinessBody<'e, 'a> {
    Statement(&'e Statement<'a>),
    Expression(&'e Expression<'a>),
}

/// A tested operand and the right operands `isSymbolUsedInBinaryExpressionChain`
/// visits from the node tsc handed the check: the `&&` expressions it is the
/// unparenthesized left side of, outward.
type TruthinessCandidate<'e, 'a> = (&'e Expression<'a>, Vec<&'e Expression<'a>>);

/// A conditional expression's test, checked against `whenTrue`
/// (`checkConditionalExpression`). The `&&` operands inside it carry their own
/// tests, see [`and_operand_truthiness_tests`].
pub(crate) fn unreferenced_truthiness_tests_in(
    test: &Expression<'_>,
    body: TruthinessBody<'_, '_>,
) -> Vec<crate::ParsedTruthinessTest> {
    let mut found = Vec::new();
    testing_known_truthy_types(test, Vec::new(), &mut found);
    unreferenced_candidates(found, Some(&body))
}

/// tsc checks the left operand of every `&&` (`checkBinaryLikeExpression`);
/// outside an `if` condition nothing but the rest of the chain can use it. The
/// operands of a left-leaning chain are all computed at its root, where every
/// operand's chain is known; an `if` condition drops these for its own.
pub(crate) fn and_operand_truthiness_tests(
    logical: &oxc_ast::ast::LogicalExpression<'_>,
) -> Vec<crate::ParsedTruthinessTest> {
    use oxc_ast::ast::LogicalOperator;
    let mut found = Vec::new();
    let mut chain = vec![&logical.right];
    let mut current = logical;
    loop {
        testing_known_truthy_types(&current.left, chain.clone(), &mut found);
        match &current.left {
            Expression::LogicalExpression(inner) if inner.operator == LogicalOperator::And => {
                chain.insert(0, &inner.right);
                current = inner;
            }
            _ => break,
        }
    }
    unreferenced_candidates(found, None)
}

fn unparenthesized<'e, 'a>(
    expression: &'e Expression<'a>,
    seen: Vec<&'e Expression<'a>>,
) -> (&'e Expression<'a>, Vec<&'e Expression<'a>>) {
    match expression {
        Expression::ParenthesizedExpression(_) => (expression.without_parentheses(), Vec::new()),
        _ => (expression, seen),
    }
}

/// The chain a direct operand of `logical` walks: an `&&` adds its right side
/// to the chain `logical` itself walks.
fn operand_chain<'e, 'a>(
    logical: &'e oxc_ast::ast::LogicalExpression<'a>,
    seen: &[&'e Expression<'a>],
) -> Vec<&'e Expression<'a>> {
    if logical.operator != oxc_ast::ast::LogicalOperator::And {
        return Vec::new();
    }
    let mut chain = vec![&logical.right];
    chain.extend_from_slice(seen);
    chain
}

/// `checkTestingKnownTruthyTypes`: the expression, then each left operand down
/// its `||`/`??` chain.
fn testing_known_truthy_types<'e, 'a>(
    expression: &'e Expression<'a>,
    seen: Vec<&'e Expression<'a>>,
    found: &mut Vec<TruthinessCandidate<'e, 'a>>,
) {
    use oxc_ast::ast::LogicalOperator;
    let (condition, seen) = unparenthesized(expression, seen);
    testing_known_truthy_type(condition, seen, found);
    let mut current = condition;
    while let Expression::LogicalExpression(logical) = current
        && matches!(logical.operator, LogicalOperator::Or | LogicalOperator::Coalesce)
    {
        current = logical.left.without_parentheses();
        testing_known_truthy_type(current, Vec::new(), found);
    }
}

/// `checkTestingKnownTruthyType`: a logical expression is tested at its right
/// operand, whose use is still judged from the logical expression's own chain.
fn testing_known_truthy_type<'e, 'a>(
    condition: &'e Expression<'a>,
    seen: Vec<&'e Expression<'a>>,
    found: &mut Vec<TruthinessCandidate<'e, 'a>>,
) {
    let Expression::LogicalExpression(logical) = condition else {
        found.push((condition, seen));
        return;
    };
    let location = logical.right.without_parentheses();
    if matches!(location, Expression::LogicalExpression(_)) {
        let location_seen = match &logical.right {
            Expression::ParenthesizedExpression(_) => Vec::new(),
            _ => operand_chain(logical, &seen),
        };
        testing_known_truthy_types(location, location_seen, found);
        return;
    }
    found.push((location, seen));
}

fn if_condition_operands<'e, 'a>(
    expression: &'e Expression<'a>,
    seen: Vec<&'e Expression<'a>>,
    found: &mut Vec<TruthinessCandidate<'e, 'a>>,
) {
    let (condition, seen) = unparenthesized(expression, seen);
    let Expression::LogicalExpression(logical) = condition else {
        found.push((condition, seen));
        return;
    };
    let chain = operand_chain(logical, &seen);
    if_condition_operands(&logical.left, chain.clone(), found);
    match logical.right.without_parentheses() {
        Expression::LogicalExpression(_) => if_condition_operands(&logical.right, chain, found),
        right => found.push((right, seen)),
    }
}

/// Drops a candidate its chain or the guarded code mentions again
/// (`isSymbolUsedInBinaryExpressionChain`, `isSymbolUsedInConditionBody`),
/// matched by name and member path.
fn unreferenced_candidates(
    found: Vec<TruthinessCandidate<'_, '_>>,
    body: Option<&TruthinessBody<'_, '_>>,
) -> Vec<crate::ParsedTruthinessTest> {
    let mut references = PathReferences::default();
    match body {
        Some(TruthinessBody::Statement(statement)) => {
            oxc_ast_visit::Visit::visit_statement(&mut references, statement)
        }
        Some(TruthinessBody::Expression(expression)) => {
            oxc_ast_visit::Visit::visit_expression(&mut references, expression)
        }
        None => {}
    }
    let mut tests: Vec<crate::ParsedTruthinessTest> = Vec::new();
    for (leaf, chain_rights) in found {
        let Some(path) = member_path(leaf) else {
            continue;
        };
        // The walk visits each right operand's children, so a bare name there
        // is not a use of itself.
        let mut chain = PathReferences::default();
        for right in chain_rights {
            if !matches!(right, Expression::Identifier(_)) {
                oxc_ast_visit::Visit::visit_expression(&mut chain, right);
            }
        }
        if chain.mentions_name(&path) || references.mentions(&path) {
            continue;
        }
        let span = Some(text_span_from_oxc_span(leaf.span()));
        if tests.iter().any(|test| test.span == span) {
            continue;
        }
        let (expression, span) = parse_expression(leaf);
        tests.push(crate::ParsedTruthinessTest {
            expression,
            span: Some(text_span_from_oxc_span(span)),
        });
    }
    tests
}

/// `a`, `this.m` or `a.b.c` as written; `None` for anything else, including a
/// member of an asserted expression (`(x as T).m`), which tsc exempts.
fn member_path(expression: &Expression<'_>) -> Option<Vec<String>> {
    match expression {
        Expression::Identifier(identifier) => Some(vec![identifier.name.to_string()]),
        Expression::ThisExpression(_) => Some(vec!["this".to_string()]),
        Expression::StaticMemberExpression(member) => {
            let mut path = member_path(member.object.without_parentheses())?;
            path.push(member.property.name.to_string());
            Some(path)
        }
        _ => None,
    }
}

#[derive(Default)]
struct PathReferences {
    names: std::collections::HashSet<String>,
    paths: std::collections::HashSet<Vec<String>>,
    member_names: std::collections::HashSet<String>,
}

impl PathReferences {
    /// The `&&` chain check compares symbols only, so any reference to the
    /// tested name — as a binding or as a member — counts.
    fn mentions_name(&self, path: &[String]) -> bool {
        let Some(last) = path.last() else {
            return false;
        };
        self.names.contains(last) || self.member_names.contains(last)
    }

    /// A bare name is used by any reference to it; a member path only by a
    /// member access along the same path.
    fn mentions(&self, path: &[String]) -> bool {
        match path {
            [name] => self.names.contains(name),
            _ => self.paths.contains(path),
        }
    }
}

impl<'a> oxc_ast_visit::Visit<'a> for PathReferences {
    fn visit_identifier_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'a>) {
        self.names.insert(identifier.name.to_string());
    }

    fn visit_static_member_expression(&mut self, member: &oxc_ast::ast::StaticMemberExpression<'a>) {
        self.member_names.insert(member.property.name.to_string());
        if let Some(mut path) = member_path(member.object.without_parentheses()) {
            path.push(member.property.name.to_string());
            self.paths.insert(path);
        }
        oxc_ast_visit::walk::walk_static_member_expression(self, member);
    }
}

fn parse_while_statement(while_statement: &WhileStatement<'_>) -> Option<ParsedWhileStatement> {
    let (condition, condition_span) = parse_expression(&while_statement.test);
    let body = parse_branch_body(&while_statement.body);

    // `while (true)` enters its body unconditionally, so what the body assigns
    // is assigned afterwards — tsc's flow graph gets this from the same
    // `true`-keyword rule that makes the loop's exit unreachable.
    let runs_at_least_once = matches!(condition, ParsedExpression::BooleanLiteral(true));

    Some(ParsedWhileStatement {
        condition,
        condition_span: Some(text_span_from_oxc_span(condition_span)),
        body,
        runs_at_least_once,
        has_update: false,
    })
}

/// `for (init; test; update) body` is lowered to a block holding the
/// initializer followed by a `while (test)` whose body ends with the update
/// expression. Nothing downstream matches a classic `for` statement, so without
/// the lowering the whole statement was dropped and every diagnostic inside its
/// body went missing. The enclosing block keeps the initializer's bindings
/// loop-scoped, so two sibling `for (let i = ...)` loops do not collide.
fn parse_for_statement(for_statement: &ForStatement<'_>) -> Vec<ParsedFunctionBodyStatement> {
    let mut block = Vec::new();

    match &for_statement.init {
        Some(ForStatementInit::VariableDeclaration(declaration)) => {
            block.extend(parse_variable_declaration_as_function_body(declaration));
        }
        Some(init) => {
            if let Some(expression) = init.as_expression() {
                block.extend(parse_for_clause_statements(expression));
            }
        }
        None => {}
    }

    let mut body = parse_branch_body(&for_statement.body);
    let mut has_update = false;
    if let Some(update) = &for_statement.update {
        let update = parse_for_clause_statements(update);
        if !update.is_empty() {
            has_update = true;
            // The update clause runs in the header's scope: a `const c` the body
            // declares does not shadow the header's `c` there.
            body = vec![ParsedFunctionBodyStatement::Block(body)];
            body.extend(update);
        }
    }

    // A `for` with no test never falls through on its own, and enters its body
    // unconditionally, exactly like `while (true)` — tsc models both the same
    // way.
    let runs_at_least_once = for_statement.test.is_none();
    let (condition, condition_span) = match &for_statement.test {
        Some(test) => {
            let (condition, condition_span) = parse_expression(test);
            (condition, Some(text_span_from_oxc_span(condition_span)))
        }
        None => (ParsedExpression::BooleanLiteral(true), None),
    };

    block.push(ParsedFunctionBodyStatement::While(Box::new(
        ParsedWhileStatement {
            condition,
            condition_span,
            body,
            runs_at_least_once,
            has_update,
        },
    )));

    vec![ParsedFunctionBodyStatement::Block(block)]
}

/// `do body while (test)` is lowered to a `while (test) body` carrying
/// `runs_at_least_once`, which is what tells the checker to run the body before
/// the condition and to keep the body's assignments.
fn parse_do_while_statement(
    do_while_statement: &DoWhileStatement<'_>,
) -> Vec<ParsedFunctionBodyStatement> {
    let (condition, condition_span) = parse_expression(&do_while_statement.test);

    vec![ParsedFunctionBodyStatement::While(Box::new(
        ParsedWhileStatement {
            condition,
            condition_span: Some(text_span_from_oxc_span(condition_span)),
            body: parse_branch_body(&do_while_statement.body),
            runs_at_least_once: true,
            has_update: false,
        },
    ))]
}

fn for_binding_kind(left: &ForStatementLeft<'_>) -> crate::ParsedForBindingKind {
    match left {
        ForStatementLeft::VariableDeclaration(declaration)
            if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var =>
        {
            crate::ParsedForBindingKind::Var
        }
        ForStatementLeft::VariableDeclaration(_) => crate::ParsedForBindingKind::BlockScoped,
        _ => crate::ParsedForBindingKind::ExistingBinding,
    }
}

/// A `for…in`/`for…of` head that writes rather than declares.
#[derive(Default)]
struct ForHeadWrites {
    /// Member targets read as expressions, and the defaults a pattern
    /// evaluates.
    reads: Vec<(ParsedExpression, Option<crate::TextSpan>)>,
    names: Vec<(String, Option<crate::TextSpan>)>,
}

/// A `for…in`/`for…of` head: a declaration or a name binds; any other target
/// (a member, a pattern, one tsc rejects) keeps the loop, with what the
/// target reads checked (`checkDestructuringAssignment` and
/// `checkReferenceExpression` in tsc).
fn parse_for_head(
    left: &ForStatementLeft<'_>,
) -> Option<(ParsedBindingName, Option<(ParsedExpression, Option<crate::TextSpan>)>, Vec<(String, Option<crate::TextSpan>)>)> {
    let binding = match left {
        ForStatementLeft::VariableDeclaration(declaration) => {
            let declarator = declaration.declarations.first()?;
            parse_binding_name(&declarator.id)
        }
        ForStatementLeft::AssignmentTargetIdentifier(identifier) => ParsedBindingName::Identifier {
            name: identifier.name.to_string(),
            span: Some(text_span_from_oxc_span(identifier.span)),
        },
        _ => {
            let span = Some(text_span_from_oxc_span(left.span()));
            let mut writes = ForHeadWrites::default();
            if let Some(target) = left.as_assignment_target() {
                assignment_target_writes(target, &mut writes);
            }
            let target = match writes.reads.len() {
                0 => None,
                1 => writes.reads.pop(),
                _ => Some((ParsedExpression::Sequence { expressions: writes.reads }, span)),
            };
            return Some((ParsedBindingName::Unsupported { span }, target, writes.names));
        }
    };
    Some((binding, None, Vec::new()))
}

fn assignment_target_writes(target: &AssignmentTarget<'_>, writes: &mut ForHeadWrites) {
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => writes
            .names
            .push((identifier.name.to_string(), Some(text_span_from_oxc_span(identifier.span)))),
        AssignmentTarget::StaticMemberExpression(member) => {
            if let Some(read) = super::expressions::parse_static_member_expression(member) {
                writes.reads.push((read, Some(text_span_from_oxc_span(member.span))));
            }
        }
        AssignmentTarget::ComputedMemberExpression(member) => {
            if let Some(read) = super::expressions::parse_computed_member_expression(member) {
                writes.reads.push((read, Some(text_span_from_oxc_span(member.span))));
            }
        }
        AssignmentTarget::TSNonNullExpression(recovered) if recovered.span == recovered.expression.span() => {
            let (read, span) = parse_expression(&recovered.expression);
            writes.reads.push((read, Some(text_span_from_oxc_span(span))));
        }
        AssignmentTarget::ArrayAssignmentTarget(array) => {
            for element in array.elements.iter().flatten() {
                assignment_target_maybe_default_writes(element, writes);
            }
            if let Some(rest) = &array.rest {
                assignment_target_writes(&rest.target, writes);
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(object) => {
            for property in &object.properties {
                match property {
                    oxc_ast::ast::AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(property) => {
                        writes.names.push((
                            property.binding.name.to_string(),
                            Some(text_span_from_oxc_span(property.binding.span)),
                        ));
                        if let Some(init) = &property.init {
                            let (read, span) = parse_expression(init);
                            writes.reads.push((read, Some(text_span_from_oxc_span(span))));
                        }
                    }
                    oxc_ast::ast::AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                        if property.computed
                            && let Some(key) = property.name.as_expression()
                        {
                            let (read, span) = parse_expression(key);
                            writes.reads.push((read, Some(text_span_from_oxc_span(span))));
                        }
                        assignment_target_maybe_default_writes(&property.binding, writes);
                    }
                }
            }
            if let Some(rest) = &object.rest {
                assignment_target_writes(&rest.target, writes);
            }
        }
        _ => {}
    }
}

fn assignment_target_maybe_default_writes(
    target: &oxc_ast::ast::AssignmentTargetMaybeDefault<'_>,
    writes: &mut ForHeadWrites,
) {
    match target {
        oxc_ast::ast::AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
            assignment_target_writes(&with_default.binding, writes);
            let (read, span) = parse_expression(&with_default.init);
            writes.reads.push((read, Some(text_span_from_oxc_span(span))));
        }
        _ => {
            if let Some(target) = target.as_assignment_target() {
                assignment_target_writes(target, writes);
            }
        }
    }
}

fn parse_for_of_statement(for_of_statement: &ForOfStatement<'_>) -> Option<ParsedForOfStatement> {
    let binding_kind = for_binding_kind(&for_of_statement.left);
    let (binding_name, head_target, head_names) = parse_for_head(&for_of_statement.left)?;

    let (iterable, iterable_span) = parse_expression(&for_of_statement.right);
    let iterable_span = Some(text_span_from_oxc_span(iterable_span));
    let body = parse_branch_body(&for_of_statement.body);
    let mut head_assignments = Vec::new();
    if !for_of_statement.r#await
        && let Some(
            target @ (AssignmentTarget::ArrayAssignmentTarget(_) | AssignmentTarget::ObjectAssignmentTarget(_)),
        ) = for_of_statement.left.as_assignment_target()
    {
        // A lone object literal element is the value iterated, read as the
        // literal: a defaulted member it lacks reads `undefined`, as tsc's
        // `AccessFlagsAllowMissing` does on an object literal type.
        let element = match (target, &iterable) {
            (AssignmentTarget::ObjectAssignmentTarget(_), ParsedExpression::ArrayLiteral { elements, .. })
                if elements.len() == 1
                    && !elements[0].spread
                    && matches!(elements[0].expression, ParsedExpression::ObjectLiteral { .. }) =>
            {
                elements[0].expression.clone()
            }
            _ => ParsedExpression::Identifier {
                name: crate::FOR_OF_HEAD_ELEMENT.to_string(),
                span: None,
            },
        };
        super::lower_assignment_pattern(target, &element, iterable_span, true, &mut head_assignments);
    }

    Some(ParsedForOfStatement {
        binding_name,
        binding_kind,
        iterable,
        iterable_span,
        body,
        keys_only: false,
        is_await: for_of_statement.r#await,
        head_target,
        head_names,
        head_assignments,
    })
}

/// `for (k in o) body` reuses the `for…of` statement shape with `keys_only`
/// set: the two differ only in what the binding is typed as and in whether the
/// right-hand side has to be iterable, and every caller that recurses into the
/// body works unchanged.
fn parse_for_in_statement(
    for_in_statement: &ForInStatement<'_>,
) -> Option<ParsedForOfStatement> {
    let binding_kind = for_binding_kind(&for_in_statement.left);
    let (binding_name, head_target, head_names) = parse_for_head(&for_in_statement.left)?;

    let (iterable, iterable_span) = parse_expression(&for_in_statement.right);

    Some(ParsedForOfStatement {
        binding_name,
        binding_kind,
        iterable,
        iterable_span: Some(text_span_from_oxc_span(iterable_span)),
        body: parse_branch_body(&for_in_statement.body),
        keys_only: true,
        is_await: false,
        head_target,
        head_names,
        head_assignments: Vec::new(),
    })
}

fn parse_switch_statement(switch_statement: &SwitchStatement<'_>) -> Option<ParsedSwitchStatement> {
    let (discriminant, discriminant_span) = parse_expression(&switch_statement.discriminant);
    let cases = switch_statement
        .cases
        .iter()
        .map(parse_switch_case)
        .collect::<Option<Vec<_>>>()?;

    Some(ParsedSwitchStatement {
        discriminant,
        discriminant_span: Some(text_span_from_oxc_span(discriminant_span)),
        cases,
        span: Some(text_span_from_oxc_span(switch_statement.span)),
    })
}

fn parse_switch_case(switch_case: &SwitchCase<'_>) -> Option<ParsedSwitchCase> {
    let test = switch_case
        .test
        .as_ref()
        .map(|expression| parse_expression(expression).0);
    let test_span = switch_case
        .test
        .as_ref()
        .map(|expression| text_span_from_oxc_span(expression.span()));

    Some(ParsedSwitchCase {
        test,
        test_span,
        consequent: parse_statement_list_as_function_body(&switch_case.consequent),
        span: Some(text_span_from_oxc_span(switch_case.span)),
    })
}

fn parse_throw_statement(throw_statement: &ThrowStatement<'_>) -> Option<ParsedThrowStatement> {
    let (expression, expression_span) = parse_expression(&throw_statement.argument);

    Some(ParsedThrowStatement {
        expression,
        expression_span: Some(text_span_from_oxc_span(expression_span)),
        span: Some(text_span_from_oxc_span(throw_statement.span)),
    })
}

fn parse_try_statement(try_statement: &TryStatement<'_>) -> Option<ParsedTryStatement> {
    let block = parse_block_statement_as_function_body(&try_statement.block);
    let handler = try_statement
        .handler
        .as_ref()
        .map(|catch_clause| parse_catch_clause(catch_clause.as_ref()));
    let finalizer = try_statement
        .finalizer
        .as_ref()
        .map(|finalizer| parse_block_statement_as_function_body(finalizer))
        .unwrap_or_default();

    Some(ParsedTryStatement {
        block,
        handler,
        finalizer,
        span: Some(text_span_from_oxc_span(try_statement.span)),
    })
}

fn parse_catch_clause(catch_clause: &CatchClause<'_>) -> crate::ParsedCatchClause {
    let jsdoc = catch_clause
        .param
        .as_ref()
        .filter(|param| param.type_annotation.is_none())
        .and_then(|param| super::jsdoc::declared_type_at(param.span.start));
    let declared_type = catch_clause
        .param
        .as_ref()
        .and_then(|param| param.type_annotation.as_ref())
        .and_then(|annotation| parse_type_annotation(annotation))
        .or_else(|| jsdoc.as_ref().map(|(ty, _)| ty.clone()));
    let binding_name = catch_clause
        .param
        .as_ref()
        .map(|param| parse_binding_name(&param.pattern));

    let declared_type_span = catch_clause
        .param
        .as_ref()
        .and_then(|param| param.type_annotation.as_ref())
        .map(|annotation| text_span_from_oxc_span(annotation.type_annotation.span()))
        .or(jsdoc.map(|(_, span)| span));

    crate::ParsedCatchClause {
        binding_name,
        declared_type,
        declared_type_span,
        body: parse_block_statement_as_function_body(&catch_clause.body),
        span: Some(text_span_from_oxc_span(catch_clause.span)),
    }
}

fn parse_branch_body(statement: &Statement<'_>) -> Vec<ParsedFunctionBodyStatement> {
    match statement {
        Statement::BlockStatement(block) => parse_block_statement_as_function_body(block),
        _ => parse_single_statement_as_function_body(statement),
    }
}

pub(crate) fn parse_function_parameter(
    parameter: &FormalParameter<'_>,
) -> Option<ParsedFunctionParameter> {
    let jsdoc = parameter
        .type_annotation
        .is_none()
        .then(|| super::jsdoc::parameter_at(parameter.span.start))
        .flatten();
    let declared_type = parameter
        .type_annotation
        .as_ref()
        .and_then(|annotation| parse_type_annotation(annotation))
        .or_else(|| jsdoc.as_ref().and_then(|jsdoc| jsdoc.ty.as_ref().map(|(ty, _)| ty.clone())));
    let initializer = parameter.initializer.as_ref().map(|expression| {
        let (parsed_expression, _) = parse_expression(expression);
        // Unannotated, the initializer's contextual type is the type the
        // pattern implies (`getContextualTypeForInitializerExpression`). A
        // tuple shorter than its pattern is padded with the pattern's
        // elements (`padTupleToMatchBindingPattern`), which the lowering
        // leaves to the array type.
        let shorter_than_pattern = matches!(
            (&parameter.pattern, &parsed_expression),
            (BindingPattern::ArrayPattern(array), ParsedExpression::ArrayLiteral { elements, .. })
                if array.elements.len() > elements.len() || array.rest.is_some()
        );
        if declared_type.is_none() && !shorter_than_pattern {
            super::with_pattern_context(&parameter.pattern, parsed_expression)
        } else {
            parsed_expression
        }
    });
    let initializer_span = parameter
        .initializer
        .as_ref()
        .map(|initializer| text_span_from_oxc_span(initializer.span()));
    let written_optional = parameter.optional
        || parameter.initializer.is_some()
        || jsdoc.as_ref().is_some_and(|jsdoc| jsdoc.optional);
    // tsc's `isUntypedSignatureInJSFile`: a JavaScript signature no JSDoc
    // types accepts any number of arguments. It marks only the parameters
    // nothing else makes optional, since a contextual type takes it away.
    let untyped_javascript = super::spans::lowering_javascript()
        && !super::jsdoc::in_typed_signature(parameter.span.start)
        && !written_optional;

    Some(ParsedFunctionParameter {
        binding_name: parse_binding_name(&parameter.pattern),
        declared_type,
        initializer,
        initializer_span,
        optional: written_optional || untyped_javascript,
        untyped_javascript,
        rest: false,
        is_parameter_property: parameter.accessibility.is_some() || parameter.readonly || parameter.r#override,
        is_readonly_parameter_property: parameter.readonly,
        is_override_parameter_property: parameter.r#override,
    })
}

pub(crate) fn parse_rest_function_parameter(
    rest: &oxc_ast::ast::FormalParameterRest<'_>,
) -> Option<ParsedFunctionParameter> {
    let declared_type = rest
        .type_annotation
        .as_ref()
        .and_then(|annotation| parse_type_annotation(annotation))
        .or_else(|| {
            super::jsdoc::parameter_at(rest.span.start).and_then(|jsdoc| jsdoc.ty.map(|(ty, _)| ty))
        });

    // `...a = 0` (TS1048) is still typed by its initializer.
    let (initializer, initializer_span) = match &rest.rest.argument {
        BindingPattern::AssignmentPattern(assignment) => {
            let (expression, span) = parse_expression(&assignment.right);
            (Some(expression), Some(text_span_from_oxc_span(span)))
        }
        _ => (None, None),
    };
    Some(ParsedFunctionParameter {
        binding_name: parse_binding_name(&rest.rest.argument),
        declared_type,
        initializer,
        initializer_span,
        optional: false,
        untyped_javascript: false,
        rest: true,
        is_parameter_property: false,
        is_readonly_parameter_property: false,
        is_override_parameter_property: false,
    })
}

pub(crate) fn parse_binding_name(binding: &BindingPattern<'_>) -> ParsedBindingName {
    match binding {
        BindingPattern::BindingIdentifier(binding) => ParsedBindingName::Identifier {
            name: binding.name.to_string(),
            span: Some(text_span_from_oxc_span(binding.span)),
        },
        BindingPattern::ObjectPattern(object_pattern) => {
            ParsedBindingName::ObjectPattern(parse_object_binding_pattern(object_pattern))
        }
        BindingPattern::AssignmentPattern(assignment_pattern) => {
            parse_binding_name(&assignment_pattern.left)
        }
        BindingPattern::ArrayPattern(array_pattern) => {
            ParsedBindingName::ArrayPattern(parse_array_binding_pattern(array_pattern))
        }
    }
}

fn parse_array_binding_pattern(
    array_pattern: &oxc_ast::ast::ArrayPattern<'_>,
) -> ParsedArrayBindingPattern {
    ParsedArrayBindingPattern {
        elements: array_pattern
            .elements
            .iter()
            .map(|element| element.as_ref().map(|element| parse_binding_name(element)))
            .collect(),
        defaults: array_pattern
            .elements
            .iter()
            .map(|element| matches!(element, Some(BindingPattern::AssignmentPattern(_))))
            .collect(),
        default_values: array_pattern
            .elements
            .iter()
            .map(|element| match element {
                Some(BindingPattern::AssignmentPattern(assignment)) => {
                    let (value, span) = parse_expression(&assignment.right);
                    Some((value, Some(text_span_from_oxc_span(span))))
                }
                _ => None,
            })
            .collect(),
        rest: array_pattern
            .rest
            .as_deref()
            .map(|rest| Box::new(parse_binding_name(&rest.argument))),
        span: Some(text_span_from_oxc_span(array_pattern.span)),
    }
}

fn parse_object_binding_pattern(object_pattern: &ObjectPattern<'_>) -> ParsedObjectBindingPattern {
    ParsedObjectBindingPattern {
        elements: object_pattern
            .properties
            .iter()
            .filter_map(parse_object_binding_element)
            .collect(),
        rest: object_pattern
            .rest
            .as_deref()
            .map(|rest| Box::new(parse_binding_name(&rest.argument))),
        span: Some(text_span_from_oxc_span(object_pattern.span)),
    }
}

fn parse_object_binding_element(
    property: &BindingProperty<'_>,
) -> Option<ParsedObjectBindingElement> {
    // `{ "show": x }` and `{ ["show"]: x }` name the same property.
    let (property_name, computed_key) = match super::binding_property_key(&property.key) {
        Some((name, _)) => (name, false),
        None if property.computed => (String::new(), true),
        None => {
            return Some(ParsedObjectBindingElement {
                property_name: "<unsupported>".to_string(),
                computed_key: false,
                key_expression: None,
                shorthand: false,
                binding_name: ParsedBindingName::Unsupported {
                    span: Some(text_span_from_oxc_span(property.span)),
                },
                name_span: Some(text_span_from_oxc_span(property.span)),
                has_default: false,
                default_value: None,
                default_span: None,
                span: Some(text_span_from_oxc_span(property.span)),
            });
        }
    };

    let has_default = matches!(&property.value, BindingPattern::AssignmentPattern(_));
    let (default_value, default_span) = match &property.value {
        BindingPattern::AssignmentPattern(assignment) => {
            let (value, span) = parse_expression(&assignment.right);
            (Some(Box::new(value)), Some(text_span_from_oxc_span(span)))
        }
        _ => (None, None),
    };
    let binding_name = match &property.value {
        BindingPattern::AssignmentPattern(assignment) => parse_binding_name(&assignment.left),
        _ => parse_binding_name(&property.value),
    };
    let name_span = match &binding_name {
        ParsedBindingName::Identifier { span, .. } => *span,
        ParsedBindingName::ObjectPattern(pattern) => pattern.span,
        ParsedBindingName::ArrayPattern(pattern) => pattern.span,
        ParsedBindingName::Unsupported { span } => *span,
    };

    let key_expression = property
        .key
        .as_expression()
        .filter(|_| computed_key)
        .map(|key| {
            let (expression, span) = parse_expression(key);
            Box::new((expression, Some(text_span_from_oxc_span(span))))
        });
    Some(ParsedObjectBindingElement {
        property_name,
        computed_key,
        key_expression,
        shorthand: property.shorthand,
        binding_name,
        name_span,
        has_default,
        default_value,
        default_span,
        span: Some(text_span_from_oxc_span(property.span)),
    })
}
