//! tsc's `isPastLastAssignment` for the bindings a closure captures. A
//! function expression, arrow or object-literal method continues the flow of
//! the code that creates it only for a `const`, or for a parameter or mutable
//! local no assignment follows: one assigned after the closure starts, or
//! assigned inside any closure at all, reads as its declared type there. The
//! walk records, per closure, the captured names that lose their narrowing.

use std::collections::HashMap;
use std::rc::Rc;

use oxc_ast::ast::{
    ArrowFunctionExpression, AssignmentExpression, AssignmentTarget, BindingPattern, Class,
    ForInStatement, ForOfStatement, FormalParameters, Function, IdentifierReference, Program,
    SimpleAssignmentTarget, Statement, UpdateExpression, VariableDeclaration,
    VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_span::GetSpan;
use oxc_syntax::scope::{ScopeFlags, ScopeId};

/// Closure start → the captured names it reads at their declared types.
pub(crate) type ClosureResets = HashMap<u32, Vec<String>>;

thread_local! {
    static RESETS: std::cell::RefCell<Option<Rc<ClosureResets>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_closure_resets<R>(resets: Rc<ClosureResets>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<ClosureResets>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RESETS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = RESETS.with(|slot| slot.borrow_mut().replace(resets));
    let _restore = Restore(previous);
    f()
}

/// The names the closure starting at `start` reads at their declared types.
pub(crate) fn resets_at(start: u32) -> Vec<String> {
    RESETS.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|resets| resets.get(&start).cloned())
            .unwrap_or_default()
    })
}

pub(crate) fn collect_closure_resets(program: &Program<'_>) -> ClosureResets {
    let mut collector = Collector::default();
    collector.visit_program(program);
    collector.resolve()
}

struct Scope {
    parent: Option<usize>,
    function: usize,
    names: HashMap<String, Option<usize>>,
}

struct Binding {
    name: String,
    function: usize,
    declaration_start: u32,
    last_assignment: Option<u32>,
}

struct Write {
    name: String,
    scope: usize,
    function: usize,
    position: u32,
    statements: Vec<(u32, u32)>,
}

struct Closure {
    start: u32,
    /// The closure's own function scope, set when it is entered.
    function: Option<usize>,
    captured: Vec<usize>,
}

#[derive(Default)]
struct Collector {
    scopes: Vec<Scope>,
    stack: Vec<usize>,
    bindings: Vec<Binding>,
    writes: Vec<Write>,
    closures: Vec<Closure>,
    open_closures: Vec<usize>,
    pending_closure: Option<usize>,
    statements: Vec<(u32, u32)>,
}

impl Collector {
    fn current(&self) -> usize {
        *self.stack.last().expect("inside the program scope")
    }

    fn function_scope(&self) -> usize {
        self.scopes[self.current()].function
    }

    fn declare(&mut self, scope: usize, name: &str, mutable: bool, declaration_start: u32) {
        let binding = mutable.then(|| {
            self.bindings.push(Binding {
                name: name.to_string(),
                function: self.scopes[scope].function,
                declaration_start,
                last_assignment: None,
            });
            self.bindings.len() - 1
        });
        self.scopes[scope].names.insert(name.to_string(), binding);
    }

    fn declare_pattern(&mut self, scope: usize, pattern: &BindingPattern<'_>, mutable: bool) {
        let mut names = Vec::new();
        collect_binding_names(pattern, &mut names);
        for (name, start) in names {
            self.declare(scope, &name, mutable, start);
        }
    }

    fn resolve_name(&self, name: &str, scope: usize) -> Option<usize> {
        let mut scope = Some(scope);
        while let Some(current) = scope {
            if let Some(found) = self.scopes[current].names.get(name) {
                return *found;
            }
            scope = self.scopes[current].parent;
        }
        None
    }

    fn record_write(&mut self, name: &str, position: u32) {
        let scope = self.current();
        self.writes.push(Write {
            name: name.to_string(),
            scope,
            function: self.scopes[scope].function,
            position,
            statements: self.statements.clone(),
        });
    }

    fn record_target(&mut self, target: &AssignmentTarget<'_>) {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                self.record_write(&identifier.name, identifier.span.start);
            }
            AssignmentTarget::ArrayAssignmentTarget(_) | AssignmentTarget::ObjectAssignmentTarget(_) => {
                let mut names = Vec::new();
                collect_target_names(target, &mut names);
                for (name, position) in names {
                    self.record_write(&name, position);
                }
            }
            _ => {}
        }
    }

    fn enter_closure(&mut self, start: u32) {
        self.closures.push(Closure {
            start,
            function: None,
            captured: Vec::new(),
        });
        let index = self.closures.len() - 1;
        self.open_closures.push(index);
        self.pending_closure = Some(index);
    }

    fn resolve(mut self) -> ClosureResets {
        for write in std::mem::take(&mut self.writes) {
            let Some(index) = self.resolve_name(&write.name, write.scope) else {
                continue;
            };
            let binding = &mut self.bindings[index];
            if binding.last_assignment == Some(u32::MAX) {
                continue;
            }
            // `extendAssignmentPosition`: a write inside a statement that
            // follows the declaration counts at that statement's end.
            let position = if write.function == binding.function {
                let mut position = write.position;
                for &(start, end) in write.statements.iter().rev() {
                    if start <= binding.declaration_start {
                        break;
                    }
                    position = end;
                }
                position
            } else {
                u32::MAX
            };
            binding.last_assignment = Some(binding.last_assignment.map_or(position, |last| last.max(position)));
        }
        let mut resets = ClosureResets::new();
        for closure in &self.closures {
            let mut names: Vec<String> = closure
                .captured
                .iter()
                .map(|&index| &self.bindings[index])
                .filter(|binding| binding.last_assignment.is_some_and(|last| last > closure.start))
                .map(|binding| binding.name.clone())
                .collect();
            if names.is_empty() {
                continue;
            }
            names.sort_unstable();
            names.dedup();
            resets.insert(closure.start, names);
        }
        resets
    }
}

fn collect_binding_names(pattern: &BindingPattern<'_>, names: &mut Vec<(String, u32)>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => names.push((identifier.name.to_string(), identifier.span.start)),
        BindingPattern::AssignmentPattern(assignment) => collect_binding_names(&assignment.left, names),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                collect_binding_names(&property.value, names);
            }
            if let Some(rest) = &object.rest {
                collect_binding_names(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                collect_binding_names(element, names);
            }
            if let Some(rest) = &array.rest {
                collect_binding_names(&rest.argument, names);
            }
        }
    }
}

fn collect_target_names(target: &AssignmentTarget<'_>, names: &mut Vec<(String, u32)>) {
    #[derive(Default)]
    struct Names(Vec<(String, u32)>);
    impl<'a> Visit<'a> for Names {
        fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
            self.0.push((reference.name.to_string(), reference.span.start));
        }
        fn visit_member_expression(&mut self, _: &oxc_ast::ast::MemberExpression<'a>) {}
        fn visit_expression(&mut self, _: &oxc_ast::ast::Expression<'a>) {}
    }
    let mut collected = Names::default();
    collected.visit_assignment_target(target);
    names.extend(collected.0);
}

fn extends_assignment_position(statement: &Statement<'_>) -> bool {
    matches!(
        statement,
        Statement::VariableDeclaration(_)
            | Statement::ExpressionStatement(_)
            | Statement::IfStatement(_)
            | Statement::DoWhileStatement(_)
            | Statement::WhileStatement(_)
            | Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::WithStatement(_)
            | Statement::SwitchStatement(_)
            | Statement::TryStatement(_)
            | Statement::ClassDeclaration(_)
    )
}

impl<'a> Visit<'a> for Collector {
    fn enter_scope(&mut self, flags: ScopeFlags, _: &std::cell::Cell<Option<ScopeId>>) {
        let index = self.scopes.len();
        let parent = self.stack.last().copied();
        let function = if flags.is_function() || flags.is_top() || parent.is_none() {
            index
        } else {
            self.scopes[parent.expect("checked above")].function
        };
        self.scopes.push(Scope {
            parent,
            function,
            names: HashMap::new(),
        });
        self.stack.push(index);
        if flags.is_function()
            && let Some(closure) = self.pending_closure.take()
        {
            self.closures[closure].function = Some(index);
        }
    }

    fn leave_scope(&mut self) {
        self.stack.pop();
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        let extends = extends_assignment_position(statement);
        if extends {
            let span = statement.span();
            self.statements.push((span.start, span.end));
        }
        oxc_ast_visit::walk::walk_statement(self, statement);
        if extends {
            self.statements.pop();
        }
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        let scope = match declaration.kind {
            VariableDeclarationKind::Var => self.function_scope(),
            _ => self.current(),
        };
        let mutable = matches!(declaration.kind, VariableDeclarationKind::Var | VariableDeclarationKind::Let);
        for declarator in &declaration.declarations {
            let mut names = Vec::new();
            collect_binding_names(&declarator.id, &mut names);
            for (name, _) in names {
                self.declare(scope, &name, mutable, declarator.span.start);
            }
        }
        oxc_ast_visit::walk::walk_variable_declaration(self, declaration);
    }

    fn visit_formal_parameters(&mut self, parameters: &FormalParameters<'a>) {
        let scope = self.current();
        for parameter in &parameters.items {
            self.declare_pattern(scope, &parameter.pattern, true);
        }
        if let Some(rest) = &parameters.rest {
            self.declare_pattern(scope, &rest.rest.argument, true);
        }
        oxc_ast_visit::walk::walk_formal_parameters(self, parameters);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        if let Some(id) = &function.id
            && function.is_declaration()
        {
            let scope = self.current();
            self.declare(scope, &id.name, false, id.span.start);
        }
        let closure = !function.is_declaration();
        if closure {
            self.enter_closure(function.span.start);
        }
        oxc_ast_visit::walk::walk_function(self, function, flags);
        if closure {
            self.open_closures.pop();
        }
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.enter_closure(arrow.span.start);
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.open_closures.pop();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if let Some(id) = &class.id
            && class.is_declaration()
        {
            let scope = self.current();
            self.declare(scope, &id.name, false, id.span.start);
        }
        oxc_ast_visit::walk::walk_class(self, class);
    }

    fn visit_catch_parameter(&mut self, parameter: &oxc_ast::ast::CatchParameter<'a>) {
        let scope = self.current();
        self.declare_pattern(scope, &parameter.pattern, true);
        oxc_ast_visit::walk::walk_catch_parameter(self, parameter);
    }

    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        if self.open_closures.is_empty() {
            return;
        }
        let Some(binding) = self.resolve_name(&reference.name, self.current()) else {
            return;
        };
        let declared_in = self.bindings[binding].function;
        for &closure in &self.open_closures {
            if let Some(function) = self.closures[closure].function
                && declared_in < function
                && !self.closures[closure].captured.contains(&binding)
            {
                self.closures[closure].captured.push(binding);
            }
        }
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        self.record_target(&assignment.left);
        oxc_ast_visit::walk::walk_assignment_expression(self, assignment);
    }

    fn visit_update_expression(&mut self, update: &UpdateExpression<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) = &update.argument {
            self.record_write(&identifier.name, identifier.span.start);
        }
        oxc_ast_visit::walk::walk_update_expression(self, update);
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        if let Some(target) = statement.left.as_assignment_target() {
            self.record_target(target);
        }
        oxc_ast_visit::walk::walk_for_in_statement(self, statement);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        if let Some(target) = statement.left.as_assignment_target() {
            self.record_target(target);
        }
        oxc_ast_visit::walk::walk_for_of_statement(self, statement);
    }
}
