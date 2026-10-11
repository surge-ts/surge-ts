//! What tsc's declaration emitter (`transformers/declarations`) reports under
//! `declaration` alone, where the answer is in the source text.
//!
//! `Program.Emit` runs under `noEmit` too, so these are reported beside the
//! program's other errors unless `noEmitOnError` skips emit, which the caller
//! decides.

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, ClassElement, Declaration, Expression, MethodDefinitionKind, PropertyKey, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// One declaration-emit error, with its message arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationEmitDiagnostic {
    pub code: u32,
    pub start: u32,
    pub end: u32,
    pub args: Vec<String>,
}

/// TS4094 for an exported variable typed by the class expression it is
/// initialized with: the emitter writes that anonymous class type as an
/// object type literal, which has no way to keep a member private or
/// protected. Members a class inherits from its base are not read here.
pub fn declaration_emit_diagnostics(source_text: &str, file_name: &str) -> Vec<DeclarationEmitDiagnostic> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source_text, source_type).parse();
    if parsed.panicked {
        return Vec::new();
    }
    let mut out = Vec::new();
    for class in emitted_classes(&parsed.program.body) {
        inaccessible_this_members(class, &mut out);
    }
    elided_block_var_queries(&parsed.program.body, &mut out);
    for statement in &parsed.program.body {
        let Statement::ExportNamedDeclaration(export) = statement else {
            continue;
        };
        let Some(Declaration::VariableDeclaration(declaration)) = &export.declaration else {
            continue;
        };
        if declaration.declare {
            continue;
        }
        for declarator in &declaration.declarations {
            let (BindingPattern::BindingIdentifier(identifier), None, Some(init)) =
                (&declarator.id, &declarator.type_annotation, &declarator.init)
            else {
                continue;
            };
            let Expression::ClassExpression(class) = init.without_parentheses() else {
                continue;
            };
            let mut names = hidden_member_names(class);
            names.sort();
            names.dedup();
            for name in names {
                out.push(DeclarationEmitDiagnostic {
                    code: 4094,
                    start: identifier.span.start,
                    end: identifier.span.end,
                    args: vec![name],
                });
            }
        }
    }
    out
}

fn hidden_member_names(class: &oxc_ast::ast::Class<'_>) -> Vec<String> {
    use oxc_ast::ast::TSAccessibility;
    let hidden = |accessibility: Option<TSAccessibility>| {
        matches!(accessibility, Some(TSAccessibility::Private | TSAccessibility::Protected))
    };
    let key_name = |key: &PropertyKey<'_>| match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    };
    let mut names = Vec::new();
    for element in &class.body.body {
        let (key, accessibility) = match element {
            ClassElement::PropertyDefinition(property) => (&property.key, property.accessibility),
            ClassElement::AccessorProperty(property) => (&property.key, property.accessibility),
            ClassElement::MethodDefinition(method) if method.kind == MethodDefinitionKind::Constructor => {
                for parameter in &method.value.params.items {
                    if hidden(parameter.accessibility)
                        && let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern
                    {
                        names.push(identifier.name.to_string());
                    }
                }
                continue;
            }
            ClassElement::MethodDefinition(method) => (&method.key, method.accessibility),
            _ => continue,
        };
        if hidden(accessibility) || matches!(key, PropertyKey::PrivateIdentifier(_)) {
            names.extend(key_name(key));
        }
    }
    names
}

/// The classes declaration emit writes: every top-level class of a script,
/// and the exported ones of a module.
fn emitted_classes<'a>(body: &'a [Statement<'a>]) -> Vec<&'a oxc_ast::ast::Class<'a>> {
    use oxc_ast::ast::{ExportDefaultDeclarationKind, ModuleExportName};
    let is_module = body.iter().any(|statement| {
        matches!(
            statement,
            Statement::ImportDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportAllDeclaration(_)
                | Statement::TSExportAssignment(_)
        )
    });
    let mut exported_names: Vec<&str> = Vec::new();
    let mut classes = Vec::new();
    for statement in body {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                if let Some(Declaration::ClassDeclaration(class)) = &export.declaration {
                    classes.push(class.as_ref());
                }
                if export.source.is_none() {
                    for specifier in &export.specifiers {
                        if let ModuleExportName::IdentifierReference(local) = &specifier.local {
                            exported_names.push(local.name.as_str());
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => classes.push(class.as_ref()),
                ExportDefaultDeclarationKind::Identifier(identifier) => exported_names.push(identifier.name.as_str()),
                _ => {}
            },
            Statement::TSExportAssignment(assignment) => {
                if let Expression::Identifier(identifier) = &assignment.expression {
                    exported_names.push(identifier.name.as_str());
                }
            }
            _ => {}
        }
    }
    for statement in body {
        if let Statement::ClassDeclaration(class) = statement
            && !class.declare
            && (!is_module || class.id.as_ref().is_some_and(|id| exported_names.contains(&id.name.as_str())))
        {
            classes.push(class.as_ref());
        }
    }
    classes
}

/// TS2527 for a member whose inferred type writes an object type holding the
/// class's `this`: inside a type literal `this` would name the literal, so
/// the emitter cannot write it. Private and static members are written
/// without that type; an annotated member has its own.
fn inaccessible_this_members(class: &oxc_ast::ast::Class<'_>, out: &mut Vec<DeclarationEmitDiagnostic>) {
    use oxc_ast::ast::TSAccessibility;
    for element in &class.body.body {
        let (key, holds) = match element {
            ClassElement::PropertyDefinition(property)
                if !property.r#static
                    && !property.declare
                    && property.type_annotation.is_none()
                    && property.accessibility != Some(TSAccessibility::Private) =>
            {
                (&property.key, property.value.as_ref().is_some_and(|value| holds_this(value, false)))
            }
            ClassElement::MethodDefinition(method)
                if !method.r#static
                    && matches!(method.kind, MethodDefinitionKind::Method | MethodDefinitionKind::Get)
                    && method.value.return_type.is_none()
                    && method.accessibility != Some(TSAccessibility::Private) =>
            {
                (
                    &method.key,
                    method.value.body.as_ref().is_some_and(|body| returns_hold_this(&body.statements, false)),
                )
            }
            _ => continue,
        };
        if !holds {
            continue;
        }
        let (name, span) = match key {
            PropertyKey::StaticIdentifier(identifier) => (identifier.name.to_string(), identifier.span),
            PropertyKey::StringLiteral(literal) => (literal.value.to_string(), literal.span),
            _ => continue,
        };
        out.push(DeclarationEmitDiagnostic {
            code: 2527,
            start: span.start,
            end: span.end,
            args: vec![name, "this".to_string()],
        });
    }
}

/// Whether `expression` is typed with an object literal type holding the
/// class's `this` (`in_object`: already inside one). Only what keeps the
/// literal's type is followed: a call or an assertion types its result
/// otherwise, and an object method or a `function` binds a `this` of its own.
fn holds_this(expression: &Expression<'_>, in_object: bool) -> bool {
    use oxc_ast::ast::{ArrayExpressionElement, ObjectPropertyKind, PropertyKind};
    match expression.without_parentheses() {
        Expression::ThisExpression(_) => in_object,
        Expression::ObjectExpression(object) => object.properties.iter().any(|property| match property {
            ObjectPropertyKind::ObjectProperty(property) => {
                property.kind == PropertyKind::Init && !property.method && holds_this(&property.value, true)
            }
            ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        Expression::ArrayExpression(array) => array.elements.iter().any(|element| match element {
            ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_) => false,
            element => element.as_expression().is_some_and(|element| holds_this(element, in_object)),
        }),
        Expression::ConditionalExpression(conditional) => {
            holds_this(&conditional.consequent, in_object) || holds_this(&conditional.alternate, in_object)
        }
        Expression::LogicalExpression(logical) => {
            holds_this(&logical.left, in_object) || holds_this(&logical.right, in_object)
        }
        Expression::ArrowFunctionExpression(arrow) => {
            if arrow.expression {
                arrow.body.statements.first().is_some_and(|statement| match statement {
                    Statement::ExpressionStatement(body) => holds_this(&body.expression, in_object),
                    _ => false,
                })
            } else {
                returns_hold_this(&arrow.body.statements, in_object)
            }
        }
        _ => false,
    }
}

/// [`holds_this`] over what a body returns, nested functions aside.
fn returns_hold_this(statements: &[Statement<'_>], in_object: bool) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::ReturnStatement(ret) => ret.argument.as_ref().is_some_and(|argument| holds_this(argument, in_object)),
        Statement::BlockStatement(block) => returns_hold_this(&block.body, in_object),
        Statement::IfStatement(if_statement) => {
            returns_hold_this(std::slice::from_ref(&if_statement.consequent), in_object)
                || if_statement
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| returns_hold_this(std::slice::from_ref(alternate), in_object))
        }
        Statement::TryStatement(try_statement) => {
            returns_hold_this(&try_statement.block.body, in_object)
                || try_statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| returns_hold_this(&handler.body.body, in_object))
                || try_statement
                    .finalizer
                    .as_ref()
                    .is_some_and(|finalizer| returns_hold_this(&finalizer.body, in_object))
        }
        Statement::SwitchStatement(switch) => {
            switch.cases.iter().any(|case| returns_hold_this(&case.consequent, in_object))
        }
        Statement::ForStatement(loop_statement) => returns_hold_this(std::slice::from_ref(&loop_statement.body), in_object),
        Statement::ForInStatement(loop_statement) => returns_hold_this(std::slice::from_ref(&loop_statement.body), in_object),
        Statement::ForOfStatement(loop_statement) => returns_hold_this(std::slice::from_ref(&loop_statement.body), in_object),
        Statement::WhileStatement(loop_statement) => returns_hold_this(std::slice::from_ref(&loop_statement.body), in_object),
        Statement::DoWhileStatement(loop_statement) => returns_hold_this(std::slice::from_ref(&loop_statement.body), in_object),
        _ => false,
    })
}

/// TS4025/TS4081: an exported variable's annotation or an exported type
/// alias naming in `typeof` a `var` only a block declares. The block is not
/// written to the declaration file, so neither is the variable.
fn elided_block_var_queries(body: &[Statement<'_>], out: &mut Vec<DeclarationEmitDiagnostic>) {
    use oxc_ast::ast::{TSTypeQuery, TSTypeQueryExprName, VariableDeclarationKind};
    use oxc_ast_visit::Visit;

    #[derive(Default)]
    struct BlockVars(Vec<String>);
    impl<'a> Visit<'a> for BlockVars {
        fn visit_variable_declaration(&mut self, declaration: &oxc_ast::ast::VariableDeclaration<'a>) {
            if declaration.kind == VariableDeclarationKind::Var {
                for declarator in &declaration.declarations {
                    if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                        self.0.push(identifier.name.to_string());
                    }
                }
            }
        }
        fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
        fn visit_class(&mut self, _: &oxc_ast::ast::Class<'a>) {}
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
    }

    // A query inside a member or a signature is reported for that member
    // (TS4033 and the like), not for the declaration around it.
    #[derive(Default)]
    struct Queries(Vec<(String, oxc_span::Span)>);
    impl<'a> Visit<'a> for Queries {
        fn visit_ts_type_query(&mut self, query: &TSTypeQuery<'a>) {
            if let TSTypeQueryExprName::IdentifierReference(identifier) = &query.expr_name {
                self.0.push((identifier.name.to_string(), identifier.span));
            }
        }
        fn visit_ts_type_literal(&mut self, _: &oxc_ast::ast::TSTypeLiteral<'a>) {}
        fn visit_ts_function_type(&mut self, _: &oxc_ast::ast::TSFunctionType<'a>) {}
        fn visit_ts_constructor_type(&mut self, _: &oxc_ast::ast::TSConstructorType<'a>) {}
        fn visit_ts_mapped_type(&mut self, _: &oxc_ast::ast::TSMappedType<'a>) {}
    }

    let mut block_vars = BlockVars::default();
    let mut top_level: Vec<&str> = Vec::new();
    for statement in body {
        match statement {
            Statement::BlockStatement(_)
            | Statement::IfStatement(_)
            | Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_)
            | Statement::TryStatement(_)
            | Statement::SwitchStatement(_)
            | Statement::LabeledStatement(_) => block_vars.visit_statement(statement),
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                        top_level.push(identifier.name.as_str());
                    }
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(Declaration::VariableDeclaration(declaration)) = &export.declaration {
                    for declarator in &declaration.declarations {
                        if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                            top_level.push(identifier.name.as_str());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let hidden: Vec<&String> = block_vars.0.iter().filter(|name| !top_level.contains(&name.as_str())).collect();
    if hidden.is_empty() {
        return;
    }
    let mut report = |code: u32, declaration_name: &str, queries: Queries| {
        for (name, span) in queries.0 {
            if hidden.iter().any(|hidden| **hidden == name) {
                out.push(DeclarationEmitDiagnostic {
                    code,
                    start: span.start,
                    end: span.end,
                    args: vec![declaration_name.to_string(), name],
                });
            }
        }
    };
    for statement in body {
        let Statement::ExportNamedDeclaration(export) = statement else {
            continue;
        };
        match &export.declaration {
            Some(Declaration::VariableDeclaration(declaration)) => {
                for declarator in &declaration.declarations {
                    let (BindingPattern::BindingIdentifier(identifier), Some(annotation)) =
                        (&declarator.id, &declarator.type_annotation)
                    else {
                        continue;
                    };
                    let mut queries = Queries::default();
                    queries.visit_ts_type_annotation(annotation);
                    report(4025, &identifier.name, queries);
                }
            }
            Some(Declaration::TSTypeAliasDeclaration(alias)) => {
                let mut queries = Queries::default();
                queries.visit_ts_type(&alias.type_annotation);
                report(4081, &alias.id.name, queries);
            }
            _ => {}
        }
    }
}
