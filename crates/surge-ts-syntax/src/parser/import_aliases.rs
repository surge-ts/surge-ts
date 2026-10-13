//! `import x = N.M` — an alias of an entity name.
//!
//! tsc binds the alias as a symbol carrying every meaning of the entity it
//! names, resolved from the declaring block (`getTargetOfImportEqualsDeclaration`
//! → `getSymbolOfPartOfRightHandSideOfImportEquals`). surge resolves a written
//! entity path the same way everywhere, so each reference to an alias is
//! rewritten here to the path it stands for, before lowering. The alias is
//! scoped like tsc's: visible throughout its program or namespace block and the
//! scopes nested in it, and hidden by a nearer declaration of the same name for
//! the meaning that declaration has (a `let x` hides a value reference `x`, not
//! a type reference `x.T`).

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::AstBuilder;
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, BlockStatement, CatchClause, Class, Declaration,
    Expression, ForInStatement, ForOfStatement, ForStatement, ForStatementInit, ForStatementLeft,
    FormalParameters, Function, JSXMemberExpressionObject, ObjectProperty, Program, Statement,
    StaticBlock,
    TSInterfaceDeclaration, TSModuleBlock, TSModuleDeclarationBody, TSModuleDeclarationName,
    TSModuleReference, TSQualifiedName, TSTypeAliasDeclaration, TSTypeName,
    TSTypeParameterDeclaration, TSTypeQueryExprName, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::{VisitMut, walk_mut};
use oxc_span::Span;
use oxc_syntax::scope::ScopeFlags;

pub(crate) fn expand_import_aliases<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    if !statements_declare_alias(&program.body) {
        return;
    }
    let mut expander = AliasExpander {
        ast: AstBuilder::new(allocator),
        frames: Vec::new(),
    };
    expander.visit_program(program);
}

fn statements_declare_alias(statements: &[Statement<'_>]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::TSImportEqualsDeclaration(declaration) => {
            !matches!(declaration.module_reference, TSModuleReference::ExternalModuleReference(_))
        }
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(Declaration::TSImportEqualsDeclaration(declaration)) => !matches!(
                declaration.module_reference,
                TSModuleReference::ExternalModuleReference(_)
            ),
            Some(Declaration::TSModuleDeclaration(module)) => module_declares_alias(module),
            _ => false,
        },
        Statement::TSModuleDeclaration(module) => module_declares_alias(module),
        _ => false,
    })
}

fn module_declares_alias(module: &oxc_ast::ast::TSModuleDeclaration<'_>) -> bool {
    match &module.body {
        Some(TSModuleDeclarationBody::TSModuleBlock(block)) => statements_declare_alias(&block.body),
        Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => module_declares_alias(inner),
        None => false,
    }
}

#[derive(Clone, Copy)]
enum Meaning {
    Value,
    /// A type name, or the namespace a qualified name starts from.
    Type,
}

#[derive(Default)]
struct Frame {
    values: HashSet<String>,
    types: HashSet<String>,
    aliases: HashMap<String, Vec<String>>,
}

struct AliasExpander<'a> {
    ast: AstBuilder<'a>,
    frames: Vec<Frame>,
}

impl<'a> AliasExpander<'a> {
    /// The path the alias `name` stands for at this point, unless a nearer
    /// declaration hides it or hides the first name of that path.
    fn resolve(&self, name: &str, meaning: Meaning) -> Option<Vec<String>> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            if let Some(target) = frame.aliases.get(name) {
                let head = target.first()?;
                let hidden = self.frames[index + 1..]
                    .iter()
                    .any(|nearer| nearer.values.contains(head) || nearer.types.contains(head));
                return (!hidden).then(|| target.clone());
            }
            let declared = match meaning {
                Meaning::Value => frame.values.contains(name),
                Meaning::Type => frame.types.contains(name),
            };
            if declared {
                return None;
            }
        }
        None
    }

    fn with_frame(&mut self, frame: Frame, visit: impl FnOnce(&mut Self)) {
        self.frames.push(frame);
        visit(self);
        self.frames.pop();
    }

    /// A program or namespace block: its aliases (chains resolved), and every
    /// declaration of the block, `var`s of nested statements included.
    fn block_frame(&self, statements: &[Statement<'a>]) -> Frame {
        let mut frame = Frame::default();
        let mut raw: Vec<(String, Vec<String>)> = Vec::new();
        for statement in statements {
            let declaration = match statement {
                Statement::TSImportEqualsDeclaration(declaration) => Some(declaration),
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::TSImportEqualsDeclaration(declaration)) => Some(declaration),
                    _ => None,
                },
                _ => None,
            };
            if let Some(declaration) = declaration {
                let path = match &declaration.module_reference {
                    TSModuleReference::IdentifierReference(identifier) => {
                        vec![identifier.name.to_string()]
                    }
                    TSModuleReference::QualifiedName(name) => qualified_segments(name),
                    TSModuleReference::ExternalModuleReference(_) => {
                        let name = declaration.id.name.to_string();
                        frame.values.insert(name.clone());
                        frame.types.insert(name);
                        continue;
                    }
                };
                raw.push((declaration.id.name.to_string(), path));
                continue;
            }
            declare_statement(statement, &mut frame, true);
        }
        let local: HashMap<String, Vec<String>> = raw.iter().cloned().collect();
        for (name, mut path) in raw {
            // The first name of the path resolves from the declaring block:
            // another alias of the block, then the enclosing blocks'.
            for _ in 0..8 {
                let Some(head) = path.first() else { break };
                let expansion = match local.get(head) {
                    Some(target) if *head != name => Some(target.clone()),
                    Some(_) => None,
                    None if frame.values.contains(head) || frame.types.contains(head) => None,
                    None => self.resolve(head, Meaning::Type),
                };
                let Some(expansion) = expansion else { break };
                path.splice(0..1, expansion);
            }
            frame.aliases.insert(name, path);
        }
        frame
    }

    fn expanded_expression(&self, span: Span, path: &[String]) -> Expression<'a> {
        let mut expression = self.ast.expression_identifier(span, self.ast.ident(&path[0]));
        for segment in &path[1..] {
            let property = self.ast.identifier_name(span, self.ast.ident(segment));
            expression = Expression::StaticMemberExpression(
                self.ast.alloc_static_member_expression(span, expression, property, false),
            );
        }
        expression
    }

    fn expanded_jsx_object(&self, span: Span, path: &[String]) -> JSXMemberExpressionObject<'a> {
        let mut object = self
            .ast
            .jsx_member_expression_object_identifier_reference(span, self.ast.ident(&path[0]));
        for segment in &path[1..] {
            let property = self.ast.jsx_identifier(span, self.ast.str(segment));
            object = self
                .ast
                .jsx_member_expression_object_member_expression(span, object, property);
        }
        object
    }

    fn expanded_type_name(&self, span: Span, path: &[String]) -> TSTypeName<'a> {
        let mut name = self.ast.ts_type_name_identifier_reference(span, self.ast.ident(&path[0]));
        for segment in &path[1..] {
            let right = self.ast.identifier_name(span, self.ast.ident(segment));
            name = self.ast.ts_type_name_qualified_name(span, name, right);
        }
        name
    }

    /// Rewrites the name a qualified name starts from.
    fn expand_leftmost_of_qualified(&self, qualified: &mut TSQualifiedName<'a>, meaning: Meaning) {
        match &mut qualified.left {
            TSTypeName::QualifiedName(inner) => self.expand_leftmost_of_qualified(inner, meaning),
            TSTypeName::IdentifierReference(identifier) => {
                if let Some(path) = self.resolve(identifier.name.as_str(), meaning) {
                    let span = identifier.span;
                    qualified.left = self.expanded_type_name(span, &path);
                }
            }
            TSTypeName::ThisExpression(_) => {}
        }
    }
}

impl<'a> VisitMut<'a> for AliasExpander<'a> {
    fn visit_program(&mut self, program: &mut Program<'a>) {
        let frame = self.block_frame(&program.body);
        self.with_frame(frame, |this| walk_mut::walk_program(this, program));
    }

    fn visit_ts_module_block(&mut self, block: &mut TSModuleBlock<'a>) {
        let frame = self.block_frame(&block.body);
        self.with_frame(frame, |this| walk_mut::walk_ts_module_block(this, block));
    }

    fn visit_function(&mut self, function: &mut Function<'a>, flags: ScopeFlags) {
        let mut frame = Frame::default();
        if function.is_expression()
            && let Some(id) = &function.id
        {
            frame.values.insert(id.name.to_string());
        }
        declare_type_parameters(function.type_parameters.as_deref(), &mut frame);
        declare_parameters(&function.params, &mut frame);
        if let Some(body) = &function.body {
            for statement in &body.statements {
                declare_statement(statement, &mut frame, true);
            }
        }
        self.with_frame(frame, |this| walk_mut::walk_function(this, function, flags));
    }

    fn visit_arrow_function_expression(&mut self, arrow: &mut ArrowFunctionExpression<'a>) {
        let mut frame = Frame::default();
        declare_type_parameters(arrow.type_parameters.as_deref(), &mut frame);
        declare_parameters(&arrow.params, &mut frame);
        for statement in &arrow.body.statements {
            declare_statement(statement, &mut frame, true);
        }
        self.with_frame(frame, |this| walk_mut::walk_arrow_function_expression(this, arrow));
    }

    fn visit_block_statement(&mut self, block: &mut BlockStatement<'a>) {
        let mut frame = Frame::default();
        for statement in &block.body {
            declare_statement(statement, &mut frame, false);
        }
        self.with_frame(frame, |this| walk_mut::walk_block_statement(this, block));
    }

    fn visit_static_block(&mut self, block: &mut StaticBlock<'a>) {
        let mut frame = Frame::default();
        for statement in &block.body {
            declare_statement(statement, &mut frame, true);
        }
        self.with_frame(frame, |this| walk_mut::walk_static_block(this, block));
    }

    fn visit_catch_clause(&mut self, clause: &mut CatchClause<'a>) {
        let mut frame = Frame::default();
        if let Some(parameter) = &clause.param {
            declare_binding(&parameter.pattern, &mut frame);
        }
        self.with_frame(frame, |this| walk_mut::walk_catch_clause(this, clause));
    }

    fn visit_for_statement(&mut self, statement: &mut ForStatement<'a>) {
        let mut frame = Frame::default();
        if let Some(ForStatementInit::VariableDeclaration(declaration)) = &statement.init {
            declare_lexical_variables(declaration, &mut frame);
        }
        self.with_frame(frame, |this| walk_mut::walk_for_statement(this, statement));
    }

    fn visit_for_in_statement(&mut self, statement: &mut ForInStatement<'a>) {
        let mut frame = Frame::default();
        if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left {
            declare_lexical_variables(declaration, &mut frame);
        }
        self.with_frame(frame, |this| walk_mut::walk_for_in_statement(this, statement));
    }

    fn visit_for_of_statement(&mut self, statement: &mut ForOfStatement<'a>) {
        let mut frame = Frame::default();
        if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left {
            declare_lexical_variables(declaration, &mut frame);
        }
        self.with_frame(frame, |this| walk_mut::walk_for_of_statement(this, statement));
    }

    fn visit_class(&mut self, class: &mut Class<'a>) {
        let mut frame = Frame::default();
        if class.is_expression()
            && let Some(id) = &class.id
        {
            frame.values.insert(id.name.to_string());
            frame.types.insert(id.name.to_string());
        }
        declare_type_parameters(class.type_parameters.as_deref(), &mut frame);
        self.with_frame(frame, |this| walk_mut::walk_class(this, class));
    }

    fn visit_ts_interface_declaration(&mut self, declaration: &mut TSInterfaceDeclaration<'a>) {
        let mut frame = Frame::default();
        declare_type_parameters(declaration.type_parameters.as_deref(), &mut frame);
        self.with_frame(frame, |this| walk_mut::walk_ts_interface_declaration(this, declaration));
    }

    fn visit_ts_type_alias_declaration(&mut self, declaration: &mut TSTypeAliasDeclaration<'a>) {
        let mut frame = Frame::default();
        declare_type_parameters(declaration.type_parameters.as_deref(), &mut frame);
        self.with_frame(frame, |this| walk_mut::walk_ts_type_alias_declaration(this, declaration));
    }

    fn visit_expression(&mut self, expression: &mut Expression<'a>) {
        if let Expression::Identifier(identifier) = expression {
            if let Some(path) = self.resolve(identifier.name.as_str(), Meaning::Value) {
                let span = identifier.span;
                *expression = self.expanded_expression(span, &path);
            }
            return;
        }
        walk_mut::walk_expression(self, expression);
    }

    /// A tag's leftmost name resolves as a value (`<alias.Member />`), so it
    /// is rewritten like any other value reference.
    fn visit_jsx_member_expression_object(&mut self, object: &mut JSXMemberExpressionObject<'a>) {
        if let JSXMemberExpressionObject::IdentifierReference(identifier) = object {
            if let Some(path) = self.resolve(identifier.name.as_str(), Meaning::Value) {
                let span = identifier.span;
                *object = self.expanded_jsx_object(span, &path);
            }
            return;
        }
        walk_mut::walk_jsx_member_expression_object(self, object);
    }

    fn visit_object_property(&mut self, property: &mut ObjectProperty<'a>) {
        let shorthand = property.shorthand;
        walk_mut::walk_object_property(self, property);
        if shorthand && !matches!(property.value, Expression::Identifier(_)) {
            property.shorthand = false;
        }
    }

    fn visit_ts_type_name(&mut self, name: &mut TSTypeName<'a>) {
        let expanded = match name {
            // A whole name that is the alias reads the alias's target, whose
            // failure tsc reports at the alias declaration alone: the empty
            // span marks the name as one no use site wrote.
            TSTypeName::IdentifierReference(identifier) => self
                .resolve(identifier.name.as_str(), Meaning::Type)
                .map(|path| self.expanded_type_name(Span::new(identifier.span.start, identifier.span.start), &path)),
            TSTypeName::QualifiedName(qualified) => {
                self.expand_leftmost_of_qualified(qualified, Meaning::Type);
                None
            }
            TSTypeName::ThisExpression(_) => None,
        };
        if let Some(expanded) = expanded {
            *name = expanded;
        }
    }

    fn visit_ts_type_query_expr_name(&mut self, name: &mut TSTypeQueryExprName<'a>) {
        let expanded = match name {
            TSTypeQueryExprName::IdentifierReference(identifier) => self
                .resolve(identifier.name.as_str(), Meaning::Value)
                .map(|path| self.expanded_type_name(identifier.span, &path)),
            TSTypeQueryExprName::QualifiedName(qualified) => {
                self.expand_leftmost_of_qualified(qualified, Meaning::Value);
                None
            }
            _ => {
                walk_mut::walk_ts_type_query_expr_name(self, name);
                None
            }
        };
        if let Some(expanded) = expanded {
            *name = match expanded {
                TSTypeName::IdentifierReference(identifier) => {
                    TSTypeQueryExprName::IdentifierReference(identifier)
                }
                TSTypeName::QualifiedName(qualified) => TSTypeQueryExprName::QualifiedName(qualified),
                TSTypeName::ThisExpression(this) => TSTypeQueryExprName::ThisExpression(this),
            };
        }
    }
}

fn qualified_segments(name: &TSQualifiedName<'_>) -> Vec<String> {
    let mut segments = match &name.left {
        TSTypeName::IdentifierReference(identifier) => vec![identifier.name.to_string()],
        TSTypeName::QualifiedName(left) => qualified_segments(left),
        TSTypeName::ThisExpression(_) => vec!["this".to_string()],
    };
    segments.push(name.right.name.to_string());
    segments
}

/// The names `statement` declares in its scope. `hoist_vars` also takes the
/// `var`s of nested blocks, which belong to the enclosing function or block.
fn declare_statement(statement: &Statement<'_>, frame: &mut Frame, hoist_vars: bool) {
    match statement {
        Statement::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                declare_binding(&declarator.id, frame);
            }
        }
        Statement::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                frame.values.insert(id.name.to_string());
            }
        }
        Statement::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                frame.values.insert(id.name.to_string());
                frame.types.insert(id.name.to_string());
            }
        }
        Statement::TSInterfaceDeclaration(interface) => {
            frame.types.insert(interface.id.name.to_string());
        }
        Statement::TSTypeAliasDeclaration(alias) => {
            frame.types.insert(alias.id.name.to_string());
        }
        Statement::TSEnumDeclaration(declaration) => {
            frame.values.insert(declaration.id.name.to_string());
            frame.types.insert(declaration.id.name.to_string());
        }
        Statement::TSModuleDeclaration(module) => {
            if let TSModuleDeclarationName::Identifier(id) = &module.id {
                frame.values.insert(id.name.to_string());
                frame.types.insert(id.name.to_string());
            }
        }
        Statement::TSImportEqualsDeclaration(declaration) => {
            frame.values.insert(declaration.id.name.to_string());
            frame.types.insert(declaration.id.name.to_string());
        }
        Statement::ImportDeclaration(import) => {
            for specifier in import.specifiers.iter().flatten() {
                let name = specifier.local().name.to_string();
                frame.values.insert(name.clone());
                frame.types.insert(name);
            }
        }
        Statement::ExportNamedDeclaration(export) => {
            if let Some(declaration) = &export.declaration {
                declare_declaration(declaration, frame);
            }
        }
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if let Some(id) = &function.id {
                    frame.values.insert(id.name.to_string());
                }
            }
            oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    frame.values.insert(id.name.to_string());
                    frame.types.insert(id.name.to_string());
                }
            }
            _ => {}
        },
        _ if hoist_vars => declare_nested_vars(statement, frame),
        _ => {}
    }
}

fn declare_declaration(declaration: &Declaration<'_>, frame: &mut Frame) {
    match declaration {
        Declaration::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                declare_binding(&declarator.id, frame);
            }
        }
        Declaration::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                frame.values.insert(id.name.to_string());
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                frame.values.insert(id.name.to_string());
                frame.types.insert(id.name.to_string());
            }
        }
        Declaration::TSInterfaceDeclaration(interface) => {
            frame.types.insert(interface.id.name.to_string());
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            frame.types.insert(alias.id.name.to_string());
        }
        Declaration::TSEnumDeclaration(declaration) => {
            frame.values.insert(declaration.id.name.to_string());
            frame.types.insert(declaration.id.name.to_string());
        }
        Declaration::TSModuleDeclaration(module) => {
            if let TSModuleDeclarationName::Identifier(id) = &module.id {
                frame.values.insert(id.name.to_string());
                frame.types.insert(id.name.to_string());
            }
        }
        Declaration::TSImportEqualsDeclaration(declaration) => {
            frame.values.insert(declaration.id.name.to_string());
            frame.types.insert(declaration.id.name.to_string());
        }
        Declaration::TSGlobalDeclaration(_) => {}
    }
}

/// The `var`s a statement nested in a function body declares for the body.
fn declare_nested_vars(statement: &Statement<'_>, frame: &mut Frame) {
    let mut nested = |statement: &Statement<'_>| declare_nested_vars(statement, frame);
    match statement {
        Statement::VariableDeclaration(declaration)
            if declaration.kind == VariableDeclarationKind::Var =>
        {
            for declarator in &declaration.declarations {
                declare_binding(&declarator.id, frame);
            }
        }
        Statement::BlockStatement(block) => block.body.iter().for_each(nested),
        Statement::IfStatement(statement) => {
            nested(&statement.consequent);
            if let Some(alternate) = &statement.alternate {
                nested(alternate);
            }
        }
        Statement::ForStatement(statement) => {
            if let Some(ForStatementInit::VariableDeclaration(declaration)) = &statement.init
                && declaration.kind == VariableDeclarationKind::Var
            {
                for declarator in &declaration.declarations {
                    declare_binding(&declarator.id, frame);
                }
            }
            declare_nested_vars(&statement.body, frame);
        }
        Statement::ForInStatement(statement) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left
                && declaration.kind == VariableDeclarationKind::Var
            {
                for declarator in &declaration.declarations {
                    declare_binding(&declarator.id, frame);
                }
            }
            declare_nested_vars(&statement.body, frame);
        }
        Statement::ForOfStatement(statement) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left
                && declaration.kind == VariableDeclarationKind::Var
            {
                for declarator in &declaration.declarations {
                    declare_binding(&declarator.id, frame);
                }
            }
            declare_nested_vars(&statement.body, frame);
        }
        Statement::WhileStatement(statement) => nested(&statement.body),
        Statement::DoWhileStatement(statement) => nested(&statement.body),
        Statement::LabeledStatement(statement) => nested(&statement.body),
        Statement::TryStatement(statement) => {
            statement.block.body.iter().for_each(&mut nested);
            if let Some(handler) = &statement.handler {
                handler.body.body.iter().for_each(&mut nested);
            }
            if let Some(finalizer) = &statement.finalizer {
                finalizer.body.iter().for_each(&mut nested);
            }
        }
        Statement::SwitchStatement(statement) => {
            for case in &statement.cases {
                case.consequent.iter().for_each(&mut nested);
            }
        }
        _ => {}
    }
}

fn declare_lexical_variables(declaration: &VariableDeclaration<'_>, frame: &mut Frame) {
    if declaration.kind != VariableDeclarationKind::Var {
        for declarator in &declaration.declarations {
            declare_binding(&declarator.id, frame);
        }
    }
}

fn declare_parameters(parameters: &FormalParameters<'_>, frame: &mut Frame) {
    for parameter in &parameters.items {
        declare_binding(&parameter.pattern, frame);
    }
    if let Some(rest) = &parameters.rest {
        declare_binding(&rest.rest.argument, frame);
    }
}

fn declare_type_parameters(parameters: Option<&TSTypeParameterDeclaration<'_>>, frame: &mut Frame) {
    for parameter in parameters.into_iter().flat_map(|declaration| declaration.params.iter()) {
        frame.types.insert(parameter.name.name.to_string());
    }
}

fn declare_binding(pattern: &BindingPattern<'_>, frame: &mut Frame) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            frame.values.insert(identifier.name.to_string());
        }
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                declare_binding(&property.value, frame);
            }
            if let Some(rest) = &object.rest {
                declare_binding(&rest.argument, frame);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                declare_binding(element, frame);
            }
            if let Some(rest) = &array.rest {
                declare_binding(&rest.argument, frame);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => declare_binding(&assignment.left, frame),
    }
}
