//! Which calls end the flow: the binder's `FlowCall` nodes as
//! `isReachableFlowNode` reads their effects signature.
//!
//! A call that is a whole expression statement or an operand of a comma
//! expression, with a dotted-name callee, is a `FlowCall`
//! (`maybeBindExpressionFlowIfCall`). `getEffectsSignature` resolves the
//! callee through `getTypeOfDottedName`, which takes declared types only: a
//! function, method, class or namespace, or a variable, parameter or property
//! written with a type annotation. The flow ends after a call whose signature
//! returns `never`, or asserts a parameter the call passes a false argument
//! for. Names resolve through the scopes of the walk; what the file alone
//! cannot decide (imports, named types, members inherited from a class
//! declared elsewhere) does not end the flow.

use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, CallExpression, Class, ClassElement, Declaration,
    ExportDefaultDeclarationKind, Expression, ForStatementInit, ForStatementLeft,
    FormalParameters, Function, FunctionType, LogicalOperator, MethodDefinition,
    MethodDefinitionKind, PropertyKey, Statement, StaticBlock, TSModuleDeclaration,
    TSModuleDeclarationBody, TSModuleDeclarationName, TSType, TSTypeAnnotation,
    TSTypePredicateName, VariableDeclaration,
};
use oxc_span::Span;

/// How far a member lookup follows `extends`; a cycle is an error tsc
/// reports elsewhere.
const MAX_BASE_DEPTH: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect {
    Never,
    /// `asserts <parameter>` with no type: a false argument at that index.
    Asserts(usize),
}

/// What a name or member denotes, as far as calling it goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    Plain,
    Call(Effect),
    Namespace(usize),
    /// A class, read as its constructor.
    Class(usize),
}

#[derive(Clone, Copy)]
enum Target {
    Binding(Binding),
    /// An instance of the class.
    Instance(usize),
}

/// Names declared once each; a name declared twice with different meanings
/// is taken as meaning nothing callable.
#[derive(Default)]
struct Scope {
    names: Vec<(String, Binding)>,
}

impl Scope {
    fn declare(&mut self, name: &str, binding: Binding) {
        match self.names.iter_mut().find(|(declared, _)| declared == name) {
            Some((_, existing)) if *existing != binding => *existing = Binding::Plain,
            Some(_) => {}
            None => self.names.push((name.to_string(), binding)),
        }
    }

    fn get(&self, name: &str) -> Option<Binding> {
        self.names.iter().find(|(declared, _)| declared == name).map(|(_, binding)| *binding)
    }
}

struct ClassMembers {
    span: Span,
    base: Option<String>,
    instance: Scope,
    statics: Scope,
}

struct Namespace {
    spans: Vec<Span>,
    members: Scope,
}

type FunctionGroups<'s, 'a> = Vec<(&'s str, Vec<&'s Function<'a>>)>;

pub(super) struct CallEffects {
    /// Off when the file writes neither `never` nor `asserts`: no call can
    /// end the flow.
    enabled: bool,
    scopes: Vec<Scope>,
    classes: Vec<ClassMembers>,
    namespaces: Vec<Namespace>,
    /// What `this` is in each enclosing non-arrow function: an instance of
    /// the class, or with `true` its constructor.
    this_stack: Vec<Option<(usize, bool)>>,
    class_stack: Vec<usize>,
    method_values: Vec<(Span, Option<(usize, bool)>)>,
}

impl CallEffects {
    pub(super) fn new(source_text: &str) -> Self {
        Self {
            enabled: source_text.contains("never") || source_text.contains("asserts"),
            scopes: Vec::new(),
            classes: Vec::new(),
            namespaces: Vec::new(),
            this_stack: Vec::new(),
            class_stack: Vec::new(),
            method_values: Vec::new(),
        }
    }

    /// A source file or namespace body; paired with [`Self::leave`].
    pub(super) fn enter_statements(&mut self, statements: &[Statement<'_>]) {
        if !self.enabled {
            return;
        }
        self.this_stack.push(None);
        let scope = self.scope_of(None, statements, None);
        self.scopes.push(scope);
    }

    /// Paired with [`Self::leave`].
    pub(super) fn enter_function(&mut self, function: &Function<'_>) {
        if !self.enabled {
            return;
        }
        let this = match self.method_values.last() {
            Some(&(span, this)) if span == function.span => this,
            _ => None,
        };
        self.this_stack.push(this);
        let own = match (&function.r#type, &function.id) {
            (FunctionType::FunctionExpression, Some(id)) => Some((id.name.as_str(), function_binding(&[function]))),
            _ => None,
        };
        let statements = function.body.as_ref().map_or(&[][..], |body| body.statements.as_slice());
        let scope = self.scope_of(Some(&function.params), statements, own);
        self.scopes.push(scope);
    }

    /// Paired with [`Self::leave`].
    pub(super) fn enter_static_block(&mut self, block: &StaticBlock<'_>) {
        if !self.enabled {
            return;
        }
        let class = self.class_stack.last().copied();
        self.this_stack.push(class.map(|class| (class, true)));
        let scope = self.scope_of(None, &block.body, None);
        self.scopes.push(scope);
    }

    pub(super) fn leave(&mut self) {
        if !self.enabled {
            return;
        }
        self.scopes.pop();
        self.this_stack.pop();
    }

    /// An arrow keeps the enclosing `this`; paired with
    /// [`Self::leave_arrow`].
    pub(super) fn enter_arrow(&mut self, arrow: &ArrowFunctionExpression<'_>) {
        if !self.enabled {
            return;
        }
        let scope = self.scope_of(Some(&arrow.params), &arrow.body.statements, None);
        self.scopes.push(scope);
    }

    pub(super) fn leave_arrow(&mut self) {
        if self.enabled {
            self.scopes.pop();
        }
    }

    pub(super) fn enter_class(&mut self, class: &Class<'_>) {
        if self.enabled {
            let index = self.class_index(class);
            self.class_stack.push(index);
        }
    }

    pub(super) fn leave_class(&mut self) {
        if self.enabled {
            self.class_stack.pop();
        }
    }

    pub(super) fn enter_method(&mut self, method: &MethodDefinition<'_>) {
        if self.enabled {
            let class = self.class_stack.last().copied();
            self.method_values.push((method.value.span, class.map(|class| (class, method.r#static))));
        }
    }

    pub(super) fn leave_method(&mut self) {
        if self.enabled {
            self.method_values.pop();
        }
    }

    /// A property initializer, where `this` is an instance of the class or,
    /// for a static property, its constructor.
    pub(super) fn enter_property(&mut self, is_static: bool) {
        if self.enabled {
            let class = self.class_stack.last().copied();
            self.this_stack.push(class.map(|class| (class, is_static)));
        }
    }

    pub(super) fn leave_property(&mut self) {
        if self.enabled {
            self.this_stack.pop();
        }
    }

    /// Whether evaluating `expression` binds a `FlowCall` that ends the flow;
    /// `whole_statement` when it is an expression statement's expression.
    pub(super) fn ends_flow(&self, expression: &Expression<'_>, whole_statement: bool) -> bool {
        if !self.enabled {
            return false;
        }
        match expression {
            Expression::CallExpression(call) => whole_statement && self.call_ends_flow(call),
            Expression::ParenthesizedExpression(parenthesized) => self.ends_flow(&parenthesized.expression, false),
            Expression::AssignmentExpression(assignment) => self.ends_flow(&assignment.right, false),
            Expression::SequenceExpression(sequence) => sequence.expressions.iter().any(|operand| match operand {
                Expression::CallExpression(call) => self.call_ends_flow(call),
                other => self.ends_flow(other, false),
            }),
            _ => false,
        }
    }

    fn call_ends_flow(&self, call: &CallExpression<'_>) -> bool {
        if matches!(call.callee, Expression::Super(_)) || !is_dotted_name(&call.callee) {
            return false;
        }
        match self.target(&call.callee) {
            Some(Target::Binding(Binding::Call(Effect::Never))) => true,
            Some(Target::Binding(Binding::Call(Effect::Asserts(index)))) => call
                .arguments
                .get(index)
                .and_then(|argument| argument.as_expression())
                .is_some_and(is_false_expression),
            _ => false,
        }
    }

    /// `getTypeOfDottedName`.
    fn target(&self, expression: &Expression<'_>) -> Option<Target> {
        match expression {
            Expression::ParenthesizedExpression(parenthesized) => self.target(&parenthesized.expression),
            Expression::Identifier(identifier) => self.resolve(identifier.name.as_str()).map(Target::Binding),
            Expression::ThisExpression(_) => {
                let (class, is_static) = self.this_stack.last().copied().flatten()?;
                Some(receiver(class, is_static))
            }
            Expression::Super(_) => {
                let (class, is_static) = self.this_stack.last().copied().flatten()?;
                Some(receiver(self.base_class(class)?, is_static))
            }
            Expression::StaticMemberExpression(member) => {
                let object = self.target(&member.object)?;
                self.member(object, member.property.name.as_str())
            }
            Expression::PrivateFieldExpression(member) => {
                let object = self.target(&member.object)?;
                self.member(object, &format!("#{}", member.field.name))
            }
            _ => None,
        }
    }

    fn member(&self, object: Target, name: &str) -> Option<Target> {
        let (mut class, is_static) = match object {
            Target::Binding(Binding::Namespace(namespace)) => {
                return self.namespaces[namespace].members.get(name).map(Target::Binding);
            }
            Target::Binding(Binding::Class(class)) => (class, true),
            Target::Instance(class) => (class, false),
            Target::Binding(Binding::Plain | Binding::Call(_)) => return None,
        };
        for _ in 0..MAX_BASE_DEPTH {
            let members = &self.classes[class];
            let members = if is_static { &members.statics } else { &members.instance };
            if let Some(binding) = members.get(name) {
                return Some(Target::Binding(binding));
            }
            class = self.base_class(class)?;
        }
        None
    }

    fn base_class(&self, class: usize) -> Option<usize> {
        match self.resolve(self.classes[class].base.as_deref()?)? {
            Binding::Class(base) => Some(base),
            _ => None,
        }
    }

    fn resolve(&self, name: &str) -> Option<Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Every name a function, arrow, block-bodied container or source file
    /// declares. A name declared in a nested block is taken as declared
    /// throughout, so it only ever hides an outer declaration.
    fn scope_of(
        &mut self,
        params: Option<&FormalParameters<'_>>,
        statements: &[Statement<'_>],
        own: Option<(&str, Binding)>,
    ) -> Scope {
        let mut scope = Scope::default();
        if let Some((name, binding)) = own {
            scope.declare(name, binding);
        }
        if let Some(params) = params {
            for parameter in &params.items {
                let binding = match (&parameter.pattern, &parameter.type_annotation) {
                    (BindingPattern::BindingIdentifier(_), Some(annotation)) => annotation_binding(annotation),
                    _ => Binding::Plain,
                };
                for identifier in parameter.pattern.get_binding_identifiers() {
                    scope.declare(identifier.name.as_str(), binding);
                }
            }
            if let Some(rest) = &params.rest {
                for identifier in rest.rest.argument.get_binding_identifiers() {
                    scope.declare(identifier.name.as_str(), Binding::Plain);
                }
            }
        }
        let mut functions: FunctionGroups<'_, '_> = Vec::new();
        for statement in statements {
            match statement {
                Statement::ExportNamedDeclaration(export) => {
                    if let Some(declaration) = &export.declaration {
                        self.declare(declaration, false, &mut scope, &mut functions);
                    }
                }
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => group(&mut functions, function),
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        if let Some(id) = &class.id {
                            let index = self.class_index(class);
                            scope.declare(id.name.as_str(), Binding::Class(index));
                        }
                    }
                    _ => {}
                },
                Statement::ImportDeclaration(import) => {
                    for specifier in import.specifiers.iter().flatten() {
                        scope.declare(specifier.local().name.as_str(), Binding::Plain);
                    }
                }
                other => match other.as_declaration() {
                    Some(declaration) => self.declare(declaration, false, &mut scope, &mut functions),
                    None => declare_nested(other, &mut scope),
                },
            }
        }
        for (name, group) in functions {
            scope.declare(name, function_binding(&group));
        }
        scope
    }

    fn declare<'s, 'a>(
        &mut self,
        declaration: &'s Declaration<'a>,
        ambient: bool,
        scope: &mut Scope,
        functions: &mut FunctionGroups<'s, 'a>,
    ) {
        match declaration {
            Declaration::FunctionDeclaration(function) => group(functions, function),
            Declaration::VariableDeclaration(variables) => declare_variables(variables, scope),
            Declaration::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    let index = self.class_index(class);
                    scope.declare(id.name.as_str(), Binding::Class(index));
                }
            }
            Declaration::TSModuleDeclaration(module) => self.declare_namespace(module, ambient, scope),
            Declaration::TSEnumDeclaration(declaration) => scope.declare(declaration.id.name.as_str(), Binding::Plain),
            Declaration::TSImportEqualsDeclaration(declaration) => {
                scope.declare(declaration.id.name.as_str(), Binding::Plain);
            }
            Declaration::TSTypeAliasDeclaration(_)
            | Declaration::TSInterfaceDeclaration(_)
            | Declaration::TSGlobalDeclaration(_) => {}
        }
    }

    /// Declarations of one namespace in one scope merge their exports.
    fn declare_namespace(&mut self, module: &TSModuleDeclaration<'_>, ambient: bool, scope: &mut Scope) {
        let TSModuleDeclarationName::Identifier(id) = &module.id else {
            return;
        };
        let name = id.name.as_str();
        if let Some(index) = self.namespaces.iter().position(|namespace| namespace.spans.contains(&module.span)) {
            scope.declare(name, Binding::Namespace(index));
            return;
        }
        let members = self.namespace_members(module, ambient);
        let index = match scope.get(name) {
            Some(Binding::Namespace(index)) => {
                let namespace = &mut self.namespaces[index];
                namespace.spans.push(module.span);
                for (member, binding) in members.names {
                    namespace.members.declare(&member, binding);
                }
                index
            }
            _ => {
                self.namespaces.push(Namespace {
                    spans: vec![module.span],
                    members,
                });
                self.namespaces.len() - 1
            }
        };
        scope.declare(name, Binding::Namespace(index));
    }

    /// What the namespace exports. In an ambient namespace with no export
    /// declarations every declaration is exported (the binder's
    /// `ExportContext`).
    fn namespace_members(&mut self, module: &TSModuleDeclaration<'_>, ambient: bool) -> Scope {
        let ambient = ambient || module.declare;
        let mut members = Scope::default();
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleDeclaration(nested)) => {
                self.declare_namespace(nested, ambient, &mut members);
            }
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                let export_context = ambient
                    && !block.body.iter().any(|statement| match statement {
                        Statement::ExportNamedDeclaration(export) => export.declaration.is_none(),
                        Statement::ExportAllDeclaration(_)
                        | Statement::ExportDefaultDeclaration(_)
                        | Statement::TSExportAssignment(_) => true,
                        _ => false,
                    });
                let mut functions: FunctionGroups<'_, '_> = Vec::new();
                for statement in &block.body {
                    let declaration = match statement {
                        Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                        other if export_context => other.as_declaration(),
                        _ => None,
                    };
                    if let Some(declaration) = declaration {
                        self.declare(declaration, ambient, &mut members, &mut functions);
                    }
                }
                for (name, group) in functions {
                    members.declare(name, function_binding(&group));
                }
            }
            None => {}
        }
        members
    }

    fn class_index(&mut self, class: &Class<'_>) -> usize {
        if let Some(index) = self.classes.iter().position(|members| members.span == class.span) {
            return index;
        }
        let base = match &class.super_class {
            Some(Expression::Identifier(identifier)) => Some(identifier.name.to_string()),
            _ => None,
        };
        let mut instance = Scope::default();
        let mut statics = Scope::default();
        let mut methods: Vec<(String, bool, Vec<&Function<'_>>)> = Vec::new();
        for element in &class.body.body {
            let (name, is_static, binding) = match element {
                ClassElement::MethodDefinition(method) => {
                    let Some(name) = member_name(&method.key) else {
                        continue;
                    };
                    match method.kind {
                        MethodDefinitionKind::Method => {
                            match methods.iter_mut().find(|(declared, is_static, _)| *declared == name && *is_static == method.r#static) {
                                Some((_, _, group)) => group.push(&method.value),
                                None => methods.push((name, method.r#static, vec![&method.value])),
                            }
                            continue;
                        }
                        MethodDefinitionKind::Get | MethodDefinitionKind::Set => (name, method.r#static, Binding::Plain),
                        MethodDefinitionKind::Constructor => {
                            for parameter in &method.value.params.items {
                                if parameter.accessibility.is_some() || parameter.readonly || parameter.r#override {
                                    for identifier in parameter.pattern.get_binding_identifiers() {
                                        instance.declare(identifier.name.as_str(), Binding::Plain);
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }
                ClassElement::PropertyDefinition(property) => {
                    let Some(name) = member_name(&property.key) else {
                        continue;
                    };
                    let binding = property.type_annotation.as_ref().map_or(Binding::Plain, |annotation| annotation_binding(annotation));
                    (name, property.r#static, binding)
                }
                ClassElement::AccessorProperty(property) => {
                    let Some(name) = member_name(&property.key) else {
                        continue;
                    };
                    (name, property.r#static, Binding::Plain)
                }
                ClassElement::StaticBlock(_) | ClassElement::TSIndexSignature(_) => continue,
            };
            let members = if is_static { &mut statics } else { &mut instance };
            members.declare(&name, binding);
        }
        for (name, is_static, group) in methods {
            let members = if is_static { &mut statics } else { &mut instance };
            members.declare(&name, function_binding(&group));
        }
        self.classes.push(ClassMembers {
            span: class.span,
            base,
            instance,
            statics,
        });
        self.classes.len() - 1
    }
}

fn receiver(class: usize, is_static: bool) -> Target {
    if is_static {
        Target::Binding(Binding::Class(class))
    } else {
        Target::Instance(class)
    }
}

fn member_name(key: &PropertyKey<'_>) -> Option<String> {
    match key.private_name() {
        Some(name) => Some(format!("#{name}")),
        None => key.static_name().map(|name| name.into_owned()),
    }
}

fn group<'s, 'a>(functions: &mut FunctionGroups<'s, 'a>, function: &'s Function<'a>) {
    let Some(id) = &function.id else {
        return;
    };
    let name = id.name.as_str();
    match functions.iter_mut().find(|(declared, _)| *declared == name) {
        Some((_, group)) => group.push(function),
        None => functions.push((name, vec![function])),
    }
}

/// The declarations of one function or method: the overloads when there
/// are any, else the one implementation. Resolution of several signatures
/// picks one of them, so the effect holds when they all agree.
fn function_binding(functions: &[&Function<'_>]) -> Binding {
    let overloaded = functions.iter().any(|function| function.body.is_some())
        && functions.iter().any(|function| function.body.is_none());
    let effects: Vec<Option<Effect>> = functions
        .iter()
        .filter(|function| !overloaded || function.body.is_none())
        .map(|function| match &function.return_type {
            Some(annotation) => return_effect(annotation, &function.params),
            None => jsdoc_return_effect(function),
        })
        .collect();
    match effects.split_first() {
        Some((Some(first), rest)) if rest.iter().all(|effect| *effect == Some(*first)) => Binding::Call(*first),
        _ => Binding::Plain,
    }
}

/// A JavaScript function's `@returns {never}`, which is its return type
/// annotation as `getEffectiveReturnTypeNode` reads it.
fn jsdoc_return_effect(function: &Function<'_>) -> Option<Effect> {
    let (ty, _) = super::super::jsdoc::return_type_at(function.span.start)?;
    matches!(ty, crate::ParsedType::Never).then_some(Effect::Never)
}

fn annotation_binding(annotation: &TSTypeAnnotation<'_>) -> Binding {
    match without_parentheses(&annotation.type_annotation) {
        TSType::TSFunctionType(function) => {
            return_effect(&function.return_type, &function.params).map_or(Binding::Plain, Binding::Call)
        }
        _ => Binding::Plain,
    }
}

fn return_effect(annotation: &TSTypeAnnotation<'_>, params: &FormalParameters<'_>) -> Option<Effect> {
    match without_parentheses(&annotation.type_annotation) {
        TSType::TSNeverKeyword(_) => Some(Effect::Never),
        TSType::TSTypePredicate(predicate) if predicate.asserts && predicate.type_annotation.is_none() => {
            let TSTypePredicateName::Identifier(name) = &predicate.parameter_name else {
                return None;
            };
            params
                .items
                .iter()
                .position(|parameter| {
                    parameter.pattern.get_identifier_name().is_some_and(|parameter| parameter.as_str() == name.name.as_str())
                })
                .map(Effect::Asserts)
        }
        _ => None,
    }
}

fn without_parentheses<'b, 'a>(ty: &'b TSType<'a>) -> &'b TSType<'a> {
    match ty {
        TSType::TSParenthesizedType(inner) => without_parentheses(&inner.type_annotation),
        other => other,
    }
}

fn declare_variables(variables: &VariableDeclaration<'_>, scope: &mut Scope) {
    for declarator in &variables.declarations {
        let binding = match (&declarator.id, &declarator.type_annotation) {
            (BindingPattern::BindingIdentifier(_), Some(annotation)) => annotation_binding(annotation),
            _ => Binding::Plain,
        };
        for identifier in declarator.id.get_binding_identifiers() {
            scope.declare(identifier.name.as_str(), binding);
        }
    }
}

fn declare_variable_names(variables: &VariableDeclaration<'_>, scope: &mut Scope) {
    for declarator in &variables.declarations {
        for identifier in declarator.id.get_binding_identifiers() {
            scope.declare(identifier.name.as_str(), Binding::Plain);
        }
    }
}

/// The names a statement of a container declares below its own level.
fn declare_nested(statement: &Statement<'_>, scope: &mut Scope) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                declare_block_statement(statement, scope);
            }
        }
        Statement::IfStatement(statement) => {
            declare_block_statement(&statement.consequent, scope);
            if let Some(alternate) = &statement.alternate {
                declare_block_statement(alternate, scope);
            }
        }
        Statement::ForStatement(statement) => {
            if let Some(ForStatementInit::VariableDeclaration(variables)) = &statement.init {
                declare_variable_names(variables, scope);
            }
            declare_block_statement(&statement.body, scope);
        }
        Statement::ForInStatement(statement) => {
            if let ForStatementLeft::VariableDeclaration(variables) = &statement.left {
                declare_variable_names(variables, scope);
            }
            declare_block_statement(&statement.body, scope);
        }
        Statement::ForOfStatement(statement) => {
            if let ForStatementLeft::VariableDeclaration(variables) = &statement.left {
                declare_variable_names(variables, scope);
            }
            declare_block_statement(&statement.body, scope);
        }
        Statement::WhileStatement(statement) => declare_block_statement(&statement.body, scope),
        Statement::DoWhileStatement(statement) => declare_block_statement(&statement.body, scope),
        Statement::LabeledStatement(statement) => declare_block_statement(&statement.body, scope),
        Statement::WithStatement(statement) => declare_block_statement(&statement.body, scope),
        Statement::TryStatement(statement) => {
            for statement in &statement.block.body {
                declare_block_statement(statement, scope);
            }
            if let Some(handler) = &statement.handler {
                if let Some(parameter) = &handler.param {
                    for identifier in parameter.pattern.get_binding_identifiers() {
                        scope.declare(identifier.name.as_str(), Binding::Plain);
                    }
                }
                for statement in &handler.body.body {
                    declare_block_statement(statement, scope);
                }
            }
            if let Some(finalizer) = &statement.finalizer {
                for statement in &finalizer.body {
                    declare_block_statement(statement, scope);
                }
            }
        }
        Statement::SwitchStatement(statement) => {
            for case in &statement.cases {
                for statement in &case.consequent {
                    declare_block_statement(statement, scope);
                }
            }
        }
        _ => {}
    }
}

fn declare_block_statement(statement: &Statement<'_>, scope: &mut Scope) {
    match statement {
        Statement::VariableDeclaration(variables) => declare_variable_names(variables, scope),
        Statement::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                scope.declare(id.name.as_str(), Binding::Plain);
            }
        }
        Statement::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                scope.declare(id.name.as_str(), Binding::Plain);
            }
        }
        other => declare_nested(other, scope),
    }
}

/// `IsDottedName`.
fn is_dotted_name(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::Identifier(_) | Expression::ThisExpression(_) | Expression::Super(_) | Expression::MetaProperty(_) => true,
        Expression::StaticMemberExpression(member) => is_dotted_name(&member.object),
        Expression::PrivateFieldExpression(member) => is_dotted_name(&member.object),
        Expression::ParenthesizedExpression(parenthesized) => is_dotted_name(&parenthesized.expression),
        _ => false,
    }
}

/// `isFalseExpression`.
fn is_false_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ParenthesizedExpression(parenthesized) => is_false_expression(&parenthesized.expression),
        Expression::BooleanLiteral(literal) => !literal.value,
        Expression::LogicalExpression(logical) => match logical.operator {
            LogicalOperator::And => is_false_expression(&logical.left) || is_false_expression(&logical.right),
            LogicalOperator::Or => is_false_expression(&logical.left) && is_false_expression(&logical.right),
            LogicalOperator::Coalesce => false,
        },
        _ => false,
    }
}
