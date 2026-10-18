//! Reads that cycle back into the type being declared, for the compiler
//! API's cycle markers (see `surge_ts_types::cycle_marker`).
//!
//! tsc gives `const f = () => f` a type whose return type is that type itself;
//! surge's tree types cut such a cycle where the type is read back. This walk
//! finds those reads by scope, over the full oxc AST:
//! - a binding read inside a function nested in its own initializer
//!   (`const f = () => f`, `(read = () => read)`, `for (const c of [() => c])`);
//! - `typeof x` inside `x`'s own annotation (`let m: { x: typeof m }`), or a
//!   named function expression's own signature;
//! - a body-local function's signature reading itself or a sibling function;
//! - `this` inside an object literal's own methods, accessors and function
//!   expression members.
//!
//! Each read maps to the span of what it reads: the declaration's name, or the
//! object literal.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, CatchClause, Class, Expression, ForInStatement, ForOfStatement,
    FormalParameter, Function, FunctionBody, IdentifierReference, ObjectExpression, ObjectPropertyKind, Program,
    Statement, TSTypeName, TSTypeQuery, TSTypeQueryExprName, ThisExpression, VariableDeclarator, BlockStatement,
    ForStatement, ForStatementInit, ForStatementLeft,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Collects cycle reads in every file parsed from now on (the compiler API's
/// programs); the command line never reads them.
pub fn enable_cycle_reads() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Where each cycling read starts, and the span of what it reads.
pub type CycleReads = HashMap<u32, (u32, u32)>;

static REGISTRY: std::sync::LazyLock<std::sync::RwLock<HashMap<String, std::sync::Arc<CycleReads>>>> =
    std::sync::LazyLock::new(Default::default);

pub(crate) fn register(file_name: &str, reads: CycleReads) {
    if let Ok(mut registry) = REGISTRY.write() {
        registry.insert(file_name.to_string(), std::sync::Arc::new(reads));
    }
}

/// What the read starting at `start` in `file_name` cycles back to: the span
/// of a declaration's name, or of an object literal.
pub fn cycle_read(file_name: &str, start: u32) -> Option<(u32, u32)> {
    if !enabled() {
        return None;
    }
    REGISTRY.read().ok()?.get(file_name)?.get(&start).copied()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A value read inside a function nested deeper than the target.
    Deferred,
    /// A `typeof` name.
    TypeQuery,
    /// A value read or a `typeof` name.
    Any,
}

struct Target<'a> {
    name: &'a str,
    referent: (u32, u32),
    kind: Kind,
    function_depth: usize,
    scope_depth: usize,
}

#[derive(Default)]
struct Collector<'a> {
    reads: CycleReads,
    targets: Vec<Target<'a>>,
    scopes: Vec<HashSet<&'a str>>,
    function_depth: usize,
    /// For each enclosing function or class, the object literal its `this` is,
    /// where it is an object literal's member.
    this_owners: Vec<Option<(u32, u32)>>,
    /// The object literal whose member function is entered next.
    next_this_owner: Option<(u32, u32)>,
}

pub(crate) fn collect(program: &Program<'_>) -> CycleReads {
    let mut collector = Collector::default();
    collector.visit_program(program);
    collector.reads
}

fn span_of(span: oxc_span::Span) -> (u32, u32) {
    (span.start, span.end)
}

fn pattern_names<'a>(pattern: &BindingPattern<'a>, names: &mut HashSet<&'a str>) {
    for identifier in pattern.get_binding_identifiers() {
        names.insert(identifier.name.as_str());
    }
}

/// The names a statement list declares in its own scope; with `hoisted`, the
/// `var`s its nested blocks declare too.
fn declared_names<'a>(statements: &[Statement<'a>], hoisted: bool, names: &mut HashSet<&'a str>) {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                if hoisted || !declaration.kind.is_var() {
                    for declarator in &declaration.declarations {
                        pattern_names(&declarator.id, names);
                    }
                }
            }
            Statement::FunctionDeclaration(function) => {
                if let Some(id) = &function.id {
                    names.insert(id.name.as_str());
                }
            }
            Statement::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    names.insert(id.name.as_str());
                }
            }
            _ if hoisted => nested_vars(statement, names),
            _ => {}
        }
    }
}

fn vars_in<'a>(statements: &[Statement<'a>], names: &mut HashSet<&'a str>) {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(declaration) if declaration.kind.is_var() => {
                for declarator in &declaration.declarations {
                    pattern_names(&declarator.id, names);
                }
            }
            other => nested_vars(other, names),
        }
    }
}

fn left_vars<'a>(left: &ForStatementLeft<'a>, names: &mut HashSet<&'a str>) {
    if let ForStatementLeft::VariableDeclaration(declaration) = left
        && declaration.kind.is_var()
    {
        for declarator in &declaration.declarations {
            pattern_names(&declarator.id, names);
        }
    }
}

fn nested_vars<'a>(statement: &Statement<'a>, names: &mut HashSet<&'a str>) {
    match statement {
        Statement::BlockStatement(inner) => vars_in(&inner.body, names),
        Statement::IfStatement(inner) => {
            vars_in(std::slice::from_ref(&inner.consequent), names);
            if let Some(alternate) = &inner.alternate {
                vars_in(std::slice::from_ref(alternate), names);
            }
        }
        Statement::ForStatement(inner) => {
            if let Some(ForStatementInit::VariableDeclaration(declaration)) = &inner.init
                && declaration.kind.is_var()
            {
                for declarator in &declaration.declarations {
                    pattern_names(&declarator.id, names);
                }
            }
            vars_in(std::slice::from_ref(&inner.body), names);
        }
        Statement::ForInStatement(inner) => {
            left_vars(&inner.left, names);
            vars_in(std::slice::from_ref(&inner.body), names);
        }
        Statement::ForOfStatement(inner) => {
            left_vars(&inner.left, names);
            vars_in(std::slice::from_ref(&inner.body), names);
        }
        Statement::WhileStatement(inner) => vars_in(std::slice::from_ref(&inner.body), names),
        Statement::DoWhileStatement(inner) => vars_in(std::slice::from_ref(&inner.body), names),
        Statement::LabeledStatement(inner) => vars_in(std::slice::from_ref(&inner.body), names),
        Statement::TryStatement(inner) => {
            vars_in(&inner.block.body, names);
            if let Some(handler) = &inner.handler {
                vars_in(&handler.body.body, names);
            }
            if let Some(finalizer) = &inner.finalizer {
                vars_in(&finalizer.body, names);
            }
        }
        Statement::SwitchStatement(inner) => {
            for case in &inner.cases {
                vars_in(&case.consequent, names);
            }
        }
        _ => {}
    }
}

impl<'a> Collector<'a> {
    fn with_target(&mut self, name: &'a str, referent: (u32, u32), kind: Kind, visit: impl FnOnce(&mut Self)) {
        self.targets.push(Target {
            name,
            referent,
            kind,
            function_depth: self.function_depth,
            scope_depth: self.scopes.len(),
        });
        visit(self);
        self.targets.pop();
    }

    fn with_scope(&mut self, names: HashSet<&'a str>, visit: impl FnOnce(&mut Self)) {
        self.scopes.push(names);
        visit(self);
        self.scopes.pop();
    }

    /// The target `name` read here refers back to, innermost first.
    fn record(&mut self, name: &str, span: oxc_span::Span, type_query: bool) {
        for target in self.targets.iter().rev() {
            if target.name != name {
                continue;
            }
            if self.scopes[target.scope_depth..].iter().any(|scope| scope.contains(name)) {
                return;
            }
            let applies = match target.kind {
                Kind::Deferred => !type_query && self.function_depth > target.function_depth,
                Kind::TypeQuery => type_query,
                Kind::Any => true,
            };
            if applies {
                self.reads.insert(span.start, target.referent);
            }
            return;
        }
    }

    fn function_scope(&self, params: &oxc_ast::ast::FormalParameters<'a>, body: Option<&FunctionBody<'a>>) -> HashSet<&'a str> {
        let mut names = HashSet::new();
        for parameter in &params.items {
            pattern_names(&parameter.pattern, &mut names);
        }
        if let Some(rest) = &params.rest {
            pattern_names(&rest.rest.argument, &mut names);
        }
        if let Some(body) = body {
            declared_names(&body.statements, true, &mut names);
        }
        names
    }

    /// A body-local function's signature reads its siblings back.
    fn sibling_functions(&self, statements: &[Statement<'a>]) -> Vec<(&'a str, (u32, u32))> {
        if self.function_depth == 0 {
            return Vec::new();
        }
        statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::FunctionDeclaration(function) => {
                    function.id.as_ref().map(|id| (id.name.as_str(), span_of(id.span)))
                }
                _ => None,
            })
            .collect()
    }

    fn visit_statements_with_siblings(&mut self, statements: &[Statement<'a>]) {
        let siblings = self.sibling_functions(statements);
        for statement in statements {
            match statement {
                Statement::FunctionDeclaration(function) if !siblings.is_empty() => {
                    self.visit_local_function(function, &siblings);
                }
                other => self.visit_statement(other),
            }
        }
    }

    fn visit_local_function(&mut self, function: &Function<'a>, siblings: &[(&'a str, (u32, u32))]) {
        let this_owner = self.next_this_owner.take();
        self.this_owners.push(this_owner);
        self.function_depth += 1;
        let scope = self.function_scope(&function.params, function.body.as_deref());
        // The siblings are visible in the signature as declared one scope
        // out; only the parameters' own names shadow them there.
        let mut parameters = HashSet::new();
        for parameter in &function.params.items {
            pattern_names(&parameter.pattern, &mut parameters);
        }
        let depth = self.targets.len();
        for &(name, referent) in siblings.iter().filter(|(name, _)| !parameters.contains(name)) {
            self.targets.push(Target {
                name,
                referent,
                kind: Kind::Any,
                function_depth: self.function_depth,
                scope_depth: self.scopes.len() + 1,
            });
        }
        self.with_scope(scope, |this| {
            if let Some(parameters) = &function.type_parameters {
                this.visit_ts_type_parameter_declaration(parameters);
            }
            this.visit_formal_parameters(&function.params);
            if let Some(return_type) = &function.return_type {
                this.visit_ts_type_annotation(return_type);
            }
            this.targets.truncate(depth);
            if let Some(body) = &function.body {
                this.visit_function_body(body);
            }
        });
        self.targets.truncate(depth);
        self.function_depth -= 1;
        self.this_owners.pop();
    }
}

impl<'a> Visit<'a> for Collector<'a> {
    fn visit_program(&mut self, program: &Program<'a>) {
        let mut names = HashSet::new();
        declared_names(&program.body, true, &mut names);
        self.with_scope(names, |this| this.visit_statements_with_siblings(&program.body));
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.visit_statements_with_siblings(&body.statements);
    }

    fn visit_block_statement(&mut self, block: &BlockStatement<'a>) {
        let mut names = HashSet::new();
        declared_names(&block.body, false, &mut names);
        self.with_scope(names, |this| this.visit_statements_with_siblings(&block.body));
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return walk::walk_variable_declarator(self, declarator);
        };
        let (name, referent) = (id.name.as_str(), span_of(id.span));
        if let Some(annotation) = &declarator.type_annotation {
            self.with_target(name, referent, Kind::TypeQuery, |this| this.visit_ts_type_annotation(annotation));
        }
        if let Some(init) = &declarator.init {
            self.with_target(name, referent, Kind::Deferred, |this| this.visit_expression(init));
        }
    }

    fn visit_formal_parameter(&mut self, parameter: &FormalParameter<'a>) {
        let BindingPattern::BindingIdentifier(id) = &parameter.pattern else {
            return walk::walk_formal_parameter(self, parameter);
        };
        let (name, referent) = (id.name.as_str(), span_of(id.span));
        for decorator in &parameter.decorators {
            self.visit_decorator(decorator);
        }
        if let Some(annotation) = &parameter.type_annotation {
            self.visit_ts_type_annotation(annotation);
        }
        if let Some(initializer) = &parameter.initializer {
            self.with_target(name, referent, Kind::Deferred, |this| this.visit_expression(initializer));
        }
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        let mut names = HashSet::new();
        let target = match &statement.left {
            ForStatementLeft::VariableDeclaration(declaration) if !declaration.kind.is_var() => {
                for declarator in &declaration.declarations {
                    pattern_names(&declarator.id, &mut names);
                }
                match declaration.declarations.as_slice() {
                    [declarator] => match &declarator.id {
                        BindingPattern::BindingIdentifier(id) => Some((id.name.as_str(), span_of(id.span))),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        };
        self.visit_for_statement_left(&statement.left);
        match target {
            Some((name, referent)) => {
                self.with_target(name, referent, Kind::Deferred, |this| this.visit_expression(&statement.right));
            }
            None => self.visit_expression(&statement.right),
        }
        self.with_scope(names, |this| this.visit_statement(&statement.body));
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        let mut names = HashSet::new();
        if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left
            && !declaration.kind.is_var()
        {
            for declarator in &declaration.declarations {
                pattern_names(&declarator.id, &mut names);
            }
        }
        self.with_scope(names, |this| walk::walk_for_in_statement(this, statement));
    }

    fn visit_for_statement(&mut self, statement: &ForStatement<'a>) {
        let mut names = HashSet::new();
        if let Some(ForStatementInit::VariableDeclaration(declaration)) = &statement.init
            && !declaration.kind.is_var()
        {
            for declarator in &declaration.declarations {
                pattern_names(&declarator.id, &mut names);
            }
        }
        self.with_scope(names, |this| walk::walk_for_statement(this, statement));
    }

    fn visit_catch_clause(&mut self, clause: &CatchClause<'a>) {
        let mut names = HashSet::new();
        if let Some(parameter) = &clause.param {
            pattern_names(&parameter.pattern, &mut names);
        }
        self.with_scope(names, |this| walk::walk_catch_clause(this, clause));
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let this_owner = self.next_this_owner.take();
        self.this_owners.push(this_owner);
        // A named function expression binds its own name around itself, and
        // its signature reads that name back.
        let own_name = function
            .id
            .as_ref()
            .filter(|_| function.is_expression())
            .map(|id| (id.name.as_str(), span_of(id.span)));
        let mut outer = HashSet::new();
        if let Some((name, _)) = own_name {
            outer.insert(name);
        }
        self.with_scope(outer, |this| {
            this.function_depth += 1;
            let scope = this.function_scope(&function.params, function.body.as_deref());
            this.with_scope(scope, |this| {
                let signature = |this: &mut Self| {
                    if let Some(parameters) = &function.type_parameters {
                        this.visit_ts_type_parameter_declaration(parameters);
                    }
                    this.visit_formal_parameters(&function.params);
                    if let Some(return_type) = &function.return_type {
                        this.visit_ts_type_annotation(return_type);
                    }
                };
                match own_name {
                    Some((name, referent)) => {
                        this.targets.push(Target {
                            name,
                            referent,
                            kind: Kind::TypeQuery,
                            function_depth: this.function_depth,
                            scope_depth: this.scopes.len(),
                        });
                        signature(this);
                        this.targets.pop();
                    }
                    None => signature(this),
                }
                if let Some(body) = &function.body {
                    this.visit_function_body(body);
                }
            });
            this.function_depth -= 1;
        });
        let _ = flags;
        self.this_owners.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.next_this_owner = None;
        self.function_depth += 1;
        let scope = self.function_scope(&arrow.params, Some(&arrow.body));
        self.with_scope(scope, |this| walk::walk_arrow_function_expression(this, arrow));
        self.function_depth -= 1;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.next_this_owner = None;
        self.this_owners.push(None);
        walk::walk_class(self, class);
        self.this_owners.pop();
    }

    fn visit_object_expression(&mut self, object: &ObjectExpression<'a>) {
        let owner = span_of(object.span);
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    if property.computed {
                        self.visit_property_key(&property.key);
                    }
                    if matches!(property.value, Expression::FunctionExpression(_)) {
                        self.next_this_owner = Some(owner);
                    }
                    self.visit_expression(&property.value);
                    self.next_this_owner = None;
                }
                ObjectPropertyKind::SpreadProperty(spread) => self.visit_spread_element(spread),
            }
        }
    }

    fn visit_this_expression(&mut self, this: &ThisExpression) {
        if let Some(Some(owner)) = self.this_owners.last() {
            self.reads.insert(this.span.start, *owner);
        }
    }

    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        self.record(reference.name.as_str(), reference.span, false);
    }

    fn visit_ts_type_query(&mut self, query: &TSTypeQuery<'a>) {
        let TSTypeQueryExprName::IdentifierReference(reference) = &query.expr_name else {
            return walk::walk_ts_type_query(self, query);
        };
        self.record(reference.name.as_str(), reference.span(), true);
        if let Some(arguments) = &query.type_arguments {
            self.visit_ts_type_parameter_instantiation(arguments);
        }
    }

    fn visit_ts_type_name(&mut self, name: &TSTypeName<'a>) {
        // A type name is no value read.
        if let TSTypeName::QualifiedName(qualified) = name {
            self.visit_ts_type_name(&qualified.left);
        }
    }
}
