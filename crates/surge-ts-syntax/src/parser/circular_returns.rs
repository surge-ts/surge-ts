//! The functions written as call arguments in an unannotated variable's
//! initializer whose return expressions read the variable. Relating such a
//! function to its parameter needs its return type, which needs the
//! variable's own type while it is still being resolved: tsc reports the
//! circularity on both (`reportCircularityError`, TS7022, and TS7023/TS7024
//! from `getReturnTypeOfSignature`).

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, Expression, FormalParameters, FunctionBody, ObjectPropertyKind,
    Program, PropertyKey, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::scope::ScopeFlags;

use super::spans::text_span_from_oxc_span;
use crate::{CircularReturnFunction, TextSpan};

pub(crate) fn collect_circular_return_functions(program: &Program<'_>) -> Vec<CircularReturnFunction> {
    let mut collector = Collector { found: Vec::new() };
    collector.visit_program(program);
    collector.found
}

struct Collector {
    found: Vec<CircularReturnFunction>,
}

struct Binding<'n> {
    name: &'n str,
    span: TextSpan,
}

impl<'a> Visit<'a> for Collector {
    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(identifier), None, Some(init)) =
            (&declarator.id, declarator.type_annotation.as_ref(), declarator.init.as_ref())
        {
            let binding = Binding { name: identifier.name.as_str(), span: text_span_from_oxc_span(identifier.span) };
            self.scan(init, false, None, &binding);
        }
        walk::walk_variable_declarator(self, declarator);
    }
}

impl Collector {
    fn scan(&mut self, expression: &Expression<'_>, argument: bool, name: Option<(String, TextSpan)>, binding: &Binding<'_>) {
        match expression.without_parentheses() {
            Expression::ArrowFunctionExpression(arrow) if argument && arrow.return_type.is_none() => {
                let returns = if arrow.expression {
                    arrow.body.statements.first().and_then(|statement| match statement {
                        oxc_ast::ast::Statement::ExpressionStatement(statement) => Some(vec![&statement.expression]),
                        _ => None,
                    })
                } else {
                    Some(return_expressions(&arrow.body))
                };
                if let Some(returns) = returns {
                    self.record(arrow.span.start, &arrow.params, &arrow.body, &returns, name, binding);
                }
            }
            Expression::FunctionExpression(function) if argument && function.return_type.is_none() => {
                let name = function
                    .id
                    .as_ref()
                    .map(|id| (id.name.to_string(), text_span_from_oxc_span(id.span)))
                    .or(name);
                if let Some(body) = function.body.as_ref() {
                    self.record(function.span.start, &function.params, body, &return_expressions(body), name, binding);
                }
            }
            Expression::CallExpression(call) => {
                self.scan(&call.callee, false, None, binding);
                self.scan_arguments(&call.arguments, binding);
            }
            Expression::NewExpression(new) => {
                self.scan(&new.callee, false, None, binding);
                self.scan_arguments(&new.arguments, binding);
            }
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            let key = static_key(&property.key);
                            if key.is_some() || !argument {
                                self.scan(&property.value, argument, key, binding);
                            }
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.scan(&spread.argument, argument, None, binding);
                        }
                    }
                }
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    match element {
                        ArrayExpressionElement::SpreadElement(spread) => self.scan(&spread.argument, argument, None, binding),
                        ArrayExpressionElement::Elision(_) => {}
                        element => {
                            if let Some(element) = element.as_expression() {
                                self.scan(element, argument, None, binding);
                            }
                        }
                    }
                }
            }
            Expression::TSAsExpression(inner) => self.scan(&inner.expression, argument, name, binding),
            Expression::TSSatisfiesExpression(inner) => self.scan(&inner.expression, argument, name, binding),
            Expression::TSNonNullExpression(inner) => self.scan(&inner.expression, argument, name, binding),
            Expression::ConditionalExpression(conditional) => {
                self.scan(&conditional.consequent, argument, None, binding);
                self.scan(&conditional.alternate, argument, None, binding);
            }
            Expression::LogicalExpression(logical) => {
                self.scan(&logical.left, argument, None, binding);
                self.scan(&logical.right, argument, None, binding);
            }
            Expression::AwaitExpression(inner) => self.scan(&inner.argument, false, None, binding),
            Expression::StaticMemberExpression(member) => self.scan(&member.object, false, None, binding),
            Expression::ComputedMemberExpression(member) => self.scan(&member.object, false, None, binding),
            Expression::ChainExpression(chain) => {
                if let oxc_ast::ast::ChainElement::CallExpression(call) = &chain.expression {
                    self.scan(&call.callee, false, None, binding);
                    self.scan_arguments(&call.arguments, binding);
                }
            }
            _ => {}
        }
    }

    fn scan_arguments(&mut self, arguments: &[Argument<'_>], binding: &Binding<'_>) {
        for argument in arguments {
            match argument {
                Argument::SpreadElement(spread) => self.scan(&spread.argument, true, None, binding),
                argument => {
                    if let Some(expression) = argument.as_expression() {
                        self.scan(expression, true, None, binding);
                    }
                }
            }
        }
    }

    fn record(
        &mut self,
        start: u32,
        parameters: &FormalParameters<'_>,
        body: &FunctionBody<'_>,
        returns: &[&Expression<'_>],
        name: Option<(String, TextSpan)>,
        binding: &Binding<'_>,
    ) {
        if binds_name(parameters, body, binding.name)
            || !returns.iter().any(|expression| reads_name(expression, binding.name))
        {
            return;
        }
        self.found.push(CircularReturnFunction {
            function_start: start,
            name,
            binding: binding.name.to_string(),
            binding_span: binding.span,
        });
    }
}

fn static_key(key: &PropertyKey<'_>) -> Option<(String, TextSpan)> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => {
            Some((identifier.name.to_string(), text_span_from_oxc_span(identifier.span)))
        }
        PropertyKey::StringLiteral(literal) => Some((literal.value.to_string(), text_span_from_oxc_span(literal.span))),
        _ => None,
    }
}

/// The `return` expressions of a body, outside the functions and classes it
/// holds.
fn return_expressions<'b, 'a>(body: &'b FunctionBody<'a>) -> Vec<&'b Expression<'a>> {
    struct Returns<'b, 'a> {
        found: Vec<&'b Expression<'a>>,
    }
    impl<'b, 'a> Returns<'b, 'a> {
        fn walk_statements(&mut self, statements: &'b [oxc_ast::ast::Statement<'a>]) {
            for statement in statements {
                self.walk_statement(statement);
            }
        }
        fn walk_statement(&mut self, statement: &'b oxc_ast::ast::Statement<'a>) {
            use oxc_ast::ast::Statement;
            match statement {
                Statement::ReturnStatement(statement) => {
                    if let Some(argument) = &statement.argument {
                        self.found.push(argument);
                    }
                }
                Statement::BlockStatement(block) => self.walk_statements(&block.body),
                Statement::IfStatement(statement) => {
                    self.walk_statement(&statement.consequent);
                    if let Some(alternate) = &statement.alternate {
                        self.walk_statement(alternate);
                    }
                }
                Statement::TryStatement(statement) => {
                    self.walk_statements(&statement.block.body);
                    if let Some(handler) = &statement.handler {
                        self.walk_statements(&handler.body.body);
                    }
                    if let Some(finalizer) = &statement.finalizer {
                        self.walk_statements(&finalizer.body);
                    }
                }
                Statement::SwitchStatement(statement) => {
                    for case in &statement.cases {
                        self.walk_statements(&case.consequent);
                    }
                }
                Statement::ForStatement(statement) => self.walk_statement(&statement.body),
                Statement::ForInStatement(statement) => self.walk_statement(&statement.body),
                Statement::ForOfStatement(statement) => self.walk_statement(&statement.body),
                Statement::WhileStatement(statement) => self.walk_statement(&statement.body),
                Statement::DoWhileStatement(statement) => self.walk_statement(&statement.body),
                Statement::LabeledStatement(statement) => self.walk_statement(&statement.body),
                _ => {}
            }
        }
    }
    let mut returns = Returns { found: Vec::new() };
    returns.walk_statements(&body.statements);
    returns.found
}

/// Whether a function's parameters or body (outside the functions it holds)
/// declare `name`, shadowing the variable.
fn binds_name(parameters: &FormalParameters<'_>, body: &FunctionBody<'_>, name: &str) -> bool {
    struct Bindings<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visit<'a> for Bindings<'_> {
        fn visit_binding_identifier(&mut self, identifier: &oxc_ast::ast::BindingIdentifier<'a>) {
            self.found |= identifier.name == self.name;
        }
        fn visit_function(&mut self, function: &oxc_ast::ast::Function<'a>, _: ScopeFlags) {
            if let Some(id) = &function.id {
                self.visit_binding_identifier(id);
            }
        }
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
        fn visit_class(&mut self, class: &oxc_ast::ast::Class<'a>) {
            if let Some(id) = &class.id {
                self.visit_binding_identifier(id);
            }
        }
    }
    let mut bindings = Bindings { name, found: false };
    bindings.visit_formal_parameters(parameters);
    bindings.visit_function_body(body);
    bindings.found
}

/// Whether `expression` reads `name` outside the functions and classes it holds.
fn reads_name(expression: &Expression<'_>, name: &str) -> bool {
    struct Reads<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visit<'a> for Reads<'_> {
        fn visit_identifier_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'a>) {
            self.found |= identifier.name == self.name;
        }
        fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: ScopeFlags) {}
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
        fn visit_class(&mut self, _: &oxc_ast::ast::Class<'a>) {}
        fn visit_ts_type_annotation(&mut self, _: &oxc_ast::ast::TSTypeAnnotation<'a>) {}
    }
    let mut reads = Reads { name, found: false };
    reads.visit_expression(expression);
    reads.found
}
