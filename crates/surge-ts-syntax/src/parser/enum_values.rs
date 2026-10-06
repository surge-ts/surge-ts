//! tsc's enum member evaluation — `computeEnumMemberValues` running the
//! `evaluator` package with the checker's `evaluateEntity` — over what one
//! file can resolve: the members of its enums, its unannotated `const`s, and
//! the global `NaN` and `Infinity`. A name the file does not declare (an
//! import, another file's global, a namespace's member) leaves the answer
//! unresolved, which is never reported and lowers to `number`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use oxc_ast::ast::{
    BindingPattern, ComputedMemberExpression, Declaration, ExportDefaultDeclarationKind,
    Expression, ForStatementInit, ForStatementLeft, Function, ImportDeclarationSpecifier, Program,
    Statement, StaticMemberExpression, TSEnumDeclaration, TSEnumMemberName, TSModuleDeclaration,
    TSModuleDeclarationBody, TSModuleDeclarationName, TSModuleReference, TemplateLiteral,
    VariableDeclaration, VariableDeclarationKind, VariableDeclarator,
};
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EnumValue {
    Number(f64),
    String(String),
}

impl EnumValue {
    /// The evaluator's `AnyToString`.
    fn to_js_string(&self) -> String {
        match self {
            EnumValue::Number(number) => super::number_text::js_number_to_string(*number),
            EnumValue::String(text) => text.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Evaluated {
    Value(EnumValue),
    /// tsc's evaluator has no value for it: the member is computed.
    Computed,
    /// The answer depends on a declaration the file does not hold.
    Unresolved,
}

#[derive(Debug)]
pub(crate) struct MemberEvaluation {
    pub(crate) value: Evaluated,
    /// No initializer after a member whose value is not a number (TS1061).
    pub(crate) needs_initializer: bool,
}

#[derive(Debug)]
pub(crate) struct EnumEvaluation {
    /// Written `declare`, or in an ambient context (tsc's `NodeFlagsAmbient`).
    pub(crate) ambient: bool,
    /// One per member, in declaration order.
    pub(crate) members: Vec<MemberEvaluation>,
}

/// Every enum declaration the file walk reached, by where it starts.
#[derive(Default)]
pub(crate) struct EnumValues {
    by_start: HashMap<u32, Rc<EnumEvaluation>>,
}

thread_local! {
    static ENUM_VALUES: RefCell<Option<Rc<EnumValues>>> = const { RefCell::new(None) };
}

/// Installs `values` for the duration of `f` (the statement lowering or the
/// grammar walk, which ask for each enum's evaluation). The previous set is
/// restored on exit, including unwinds.
pub(crate) fn with_enum_values<R>(values: Rc<EnumValues>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<EnumValues>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ENUM_VALUES.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = ENUM_VALUES.with(|slot| slot.borrow_mut().replace(values));
    let _restore = Restore(previous);
    f()
}

/// `declaration`'s evaluation: the file walk's, or for an enum the walk does
/// not reach (one in a function written inside an expression) one that knows
/// only the enum's own members.
pub(crate) fn enum_evaluation(declaration: &TSEnumDeclaration<'_>) -> Rc<EnumEvaluation> {
    ENUM_VALUES
        .with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|values| values.by_start.get(&declaration.span.start).cloned())
        })
        .unwrap_or_else(|| Walker::new(false, true).evaluate_enum(declaration, 0))
}

/// Evaluates the enums the file's statements declare, in the order tsc's
/// checker reaches them.
pub(crate) fn evaluate_program(program: &Program<'_>) -> EnumValues {
    if !program.source_text.contains("enum") {
        return EnumValues::default();
    }
    let mut walker = Walker::new(is_external_module(program), false);
    let statements: Vec<&Statement<'_>> = program.body.iter().collect();
    walker.walk_scope(
        &statements,
        ScopeKind::Container,
        program.source_type.is_typescript_definition(),
        &[],
    );
    EnumValues {
        by_start: walker
            .enums
            .into_iter()
            .filter_map(|(start, state)| match state {
                EnumState::Done(evaluation) => Some((start, evaluation)),
                EnumState::InProgress(_) => None,
            })
            .collect(),
    }
}

/// tsc's `isExternalModule`: an import or export makes the top level the
/// file's own scope rather than the global one.
fn is_external_module(program: &Program<'_>) -> bool {
    program.body.iter().any(|statement| match statement {
        Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::TSExportAssignment(_) => true,
        Statement::TSImportEqualsDeclaration(import) => {
            matches!(import.module_reference, TSModuleReference::ExternalModuleReference(_))
        }
        _ => false,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    /// The file or a namespace body: where a `var` stops, and what other
    /// declarations of the same scope (another file's, another block's) merge
    /// into.
    Container,
    /// A function body. A use from inside one is deferred (tsc's
    /// `isUsedInFunctionOrInstanceProperty`).
    Function,
    Block,
}

struct Scope<'b, 'a> {
    kind: ScopeKind,
    ambient: bool,
    names: Names<'b, 'a>,
}

type Names<'b, 'a> = HashMap<&'b str, Binding<'b, 'a>>;

#[derive(Clone)]
enum Binding<'b, 'a> {
    /// `const x = …` with no annotation, the one declaration tsc's
    /// `evaluateEntity` reads a value from.
    Constant { declarator: &'b VariableDeclarator<'a>, ambient: bool },
    /// Any other value, which has none for the evaluator: a `let`, a `var`, a
    /// parameter, a function, a class, a destructured or annotated `const`.
    Value,
    /// Every declaration of one enum in the scope; `merged` when a namespace
    /// or another declaration shares its name.
    Enum { declarations: Vec<&'b TSEnumDeclaration<'a>>, merged: bool },
    /// A namespace or an import, whose contents are not followed here.
    Opaque,
}

enum EnumState {
    /// The values of the members computed so far.
    InProgress(Vec<Evaluated>),
    Done(Rc<EnumEvaluation>),
}

type MemberIndex<'b, 'a> = HashMap<&'b str, (&'b TSEnumDeclaration<'a>, usize)>;

/// Where an expression is evaluated: tsc's `location`.
struct Usage<'b, 'a> {
    depth: usize,
    position: u32,
    ambient: bool,
    /// For a member initializer: the enum's members (every declaration of it),
    /// which its bare names resolve to first, and where the member starts.
    own: Option<(Rc<MemberIndex<'b, 'a>>, u32)>,
    /// For a `const`'s initializer: where the declarator starts.
    constant: Option<u32>,
}

struct Walker<'b, 'a> {
    scopes: Vec<Scope<'b, 'a>>,
    enums: HashMap<u32, EnumState>,
    /// Each `const`'s initializer value; `None` while it is being evaluated.
    constants: HashMap<u32, Option<Evaluated>>,
    member_indexes: HashMap<u32, Rc<MemberIndex<'b, 'a>>>,
    /// The file has an import or export: its top level is its own scope.
    module: bool,
    /// One enum evaluated with nothing around it: every outer name is
    /// unresolved, the globals included, since a shadowing binding may exist.
    detached: bool,
    nesting: u32,
}

impl<'b, 'a> Walker<'b, 'a> {
    fn new(module: bool, detached: bool) -> Self {
        Self {
            scopes: Vec::new(),
            enums: HashMap::new(),
            constants: HashMap::new(),
            member_indexes: HashMap::new(),
            module,
            detached,
            nesting: 0,
        }
    }

    fn ambient(&self) -> bool {
        self.scopes.last().is_some_and(|scope| scope.ambient)
    }

    fn walk_scope(
        &mut self,
        statements: &[&'b Statement<'a>],
        kind: ScopeKind,
        ambient: bool,
        parameters: &[&'b BindingPattern<'a>],
    ) {
        let mut names = Names::new();
        for parameter in parameters.iter().copied() {
            for identifier in parameter.get_binding_identifiers() {
                declare(&mut names, identifier.name.as_str(), Binding::Value);
            }
        }
        if kind != ScopeKind::Block {
            for statement in statements.iter().copied() {
                hoist_vars(statement, &mut names);
            }
        }
        for statement in statements.iter().copied() {
            declare_statement(statement, ambient, &mut names);
        }
        self.scopes.push(Scope { kind, ambient, names });
        for statement in statements.iter().copied() {
            self.walk_statement(statement);
        }
        self.scopes.pop();
    }

    fn walk_statement(&mut self, statement: &'b Statement<'a>) {
        if let Some(declaration) = statement.as_declaration() {
            self.walk_declaration(declaration);
            return;
        }
        let ambient = self.ambient();
        match statement {
            Statement::BlockStatement(block) => {
                let statements: Vec<&Statement<'a>> = block.body.iter().collect();
                self.walk_scope(&statements, ScopeKind::Block, ambient, &[]);
            }
            Statement::IfStatement(statement) => {
                self.walk_statement(&statement.consequent);
                if let Some(alternate) = &statement.alternate {
                    self.walk_statement(alternate);
                }
            }
            Statement::ForStatement(statement) => {
                let head = match &statement.init {
                    Some(ForStatementInit::VariableDeclaration(declaration)) => Some(&**declaration),
                    _ => None,
                };
                self.walk_loop(head, &statement.body);
            }
            Statement::ForInStatement(statement) => {
                self.walk_loop(loop_head(&statement.left), &statement.body);
            }
            Statement::ForOfStatement(statement) => {
                self.walk_loop(loop_head(&statement.left), &statement.body);
            }
            Statement::WhileStatement(statement) => self.walk_statement(&statement.body),
            Statement::DoWhileStatement(statement) => self.walk_statement(&statement.body),
            Statement::LabeledStatement(statement) => self.walk_statement(&statement.body),
            Statement::WithStatement(statement) => self.walk_statement(&statement.body),
            Statement::TryStatement(statement) => {
                let block: Vec<&Statement<'a>> = statement.block.body.iter().collect();
                self.walk_scope(&block, ScopeKind::Block, ambient, &[]);
                if let Some(handler) = &statement.handler {
                    let parameters: Vec<&BindingPattern<'a>> =
                        handler.param.iter().map(|parameter| &parameter.pattern).collect();
                    let body: Vec<&Statement<'a>> = handler.body.body.iter().collect();
                    self.walk_scope(&body, ScopeKind::Block, ambient, &parameters);
                }
                if let Some(finalizer) = &statement.finalizer {
                    let body: Vec<&Statement<'a>> = finalizer.body.iter().collect();
                    self.walk_scope(&body, ScopeKind::Block, ambient, &[]);
                }
            }
            Statement::SwitchStatement(statement) => {
                let body: Vec<&Statement<'a>> = statement
                    .cases
                    .iter()
                    .flat_map(|case| case.consequent.iter())
                    .collect();
                self.walk_scope(&body, ScopeKind::Block, ambient, &[]);
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(declaration) = &export.declaration {
                    self.walk_declaration(declaration);
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(function) = &export.declaration {
                    self.walk_function(function);
                }
            }
            _ => {}
        }
    }

    fn walk_declaration(&mut self, declaration: &'b Declaration<'a>) {
        match declaration {
            Declaration::TSEnumDeclaration(declaration) => {
                let depth = self.scopes.len().saturating_sub(1);
                self.evaluate_enum(declaration, depth);
            }
            Declaration::FunctionDeclaration(function) => self.walk_function(function),
            Declaration::TSModuleDeclaration(module) => {
                let ambient = self.ambient() || module.declare;
                self.walk_module(module, ambient);
            }
            Declaration::TSGlobalDeclaration(global) => {
                let statements: Vec<&Statement<'a>> = global.body.body.iter().collect();
                self.walk_scope(&statements, ScopeKind::Container, true, &[]);
            }
            _ => {}
        }
    }

    fn walk_module(&mut self, module: &'b TSModuleDeclaration<'a>, ambient: bool) {
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                let statements: Vec<&Statement<'a>> = block.body.iter().collect();
                self.walk_scope(&statements, ScopeKind::Container, ambient, &[]);
            }
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
                self.walk_module(inner, ambient || inner.declare);
            }
            None => {}
        }
    }

    fn walk_function(&mut self, function: &'b Function<'a>) {
        let Some(body) = &function.body else {
            return;
        };
        let mut parameters: Vec<&BindingPattern<'a>> =
            function.params.items.iter().map(|parameter| &parameter.pattern).collect();
        if let Some(rest) = &function.params.rest {
            parameters.push(&rest.rest.argument);
        }
        let statements: Vec<&Statement<'a>> = body.statements.iter().collect();
        let ambient = self.ambient() || function.declare;
        self.walk_scope(&statements, ScopeKind::Function, ambient, &parameters);
    }

    fn walk_loop(&mut self, head: Option<&'b VariableDeclaration<'a>>, body: &'b Statement<'a>) {
        let ambient = self.ambient();
        let mut names = Names::new();
        if let Some(head) = head.filter(|head| head.kind != VariableDeclarationKind::Var) {
            declare_variables(head, ambient, &mut names);
        }
        self.scopes.push(Scope { kind: ScopeKind::Block, ambient, names });
        self.walk_statement(body);
        self.scopes.pop();
    }

    /// tsc's `computeEnumMemberValues` for one declaration, made in the scope
    /// at `depth`.
    fn evaluate_enum(&mut self, declaration: &'b TSEnumDeclaration<'a>, depth: usize) -> Rc<EnumEvaluation> {
        let start = declaration.span.start;
        let ambient =
            declaration.declare || self.scopes.get(depth).is_some_and(|scope| scope.ambient);
        match self.enums.get(&start) {
            Some(EnumState::Done(evaluation)) => return evaluation.clone(),
            Some(EnumState::InProgress(_)) => {
                return Rc::new(EnumEvaluation { ambient, members: Vec::new() });
            }
            None => {}
        }
        let own = self.own_members(declaration, depth);
        self.enums.insert(start, EnumState::InProgress(Vec::new()));
        let mut members = Vec::with_capacity(declaration.body.members.len());
        let mut auto_value = number(0.0);
        for member in declaration.body.members.iter() {
            let (value, needs_initializer) = match &member.initializer {
                Some(initializer) => {
                    let usage = Usage {
                        depth,
                        position: member.span.start,
                        ambient,
                        own: Some((own.clone(), member.span.start)),
                        constant: None,
                    };
                    (self.evaluate(initializer, &usage), false)
                }
                // An ambient enum's members without an initializer are
                // computed rather than numbered (`computeEnumMemberValue`).
                None if ambient && !declaration.r#const => (Evaluated::Computed, false),
                None => match &auto_value {
                    Evaluated::Value(EnumValue::Number(value)) => (number(*value), false),
                    Evaluated::Unresolved => (Evaluated::Unresolved, false),
                    _ => (Evaluated::Computed, true),
                },
            };
            auto_value = match &value {
                Evaluated::Value(EnumValue::Number(value)) => number(value + 1.0),
                Evaluated::Unresolved => Evaluated::Unresolved,
                _ => Evaluated::Computed,
            };
            if let Some(EnumState::InProgress(values)) = self.enums.get_mut(&start) {
                values.push(value.clone());
            }
            members.push(MemberEvaluation { value, needs_initializer });
        }
        let evaluation = Rc::new(EnumEvaluation { ambient, members });
        self.enums.insert(start, EnumState::Done(evaluation.clone()));
        evaluation
    }

    /// The members tsc's merged enum symbol holds: those of every declaration
    /// of the enum in the scope at `depth`.
    fn own_members(&mut self, declaration: &'b TSEnumDeclaration<'a>, depth: usize) -> Rc<MemberIndex<'b, 'a>> {
        let declarations = match self
            .scopes
            .get(depth)
            .and_then(|scope| scope.names.get(declaration.id.name.as_str()))
        {
            Some(Binding::Enum { declarations, .. })
                if declarations.iter().any(|other| other.span == declaration.span) =>
            {
                declarations.clone()
            }
            _ => vec![declaration],
        };
        self.member_index(&declarations)
    }

    fn member_index(&mut self, declarations: &[&'b TSEnumDeclaration<'a>]) -> Rc<MemberIndex<'b, 'a>> {
        let key = declarations.first().map_or(0, |declaration| declaration.span.start);
        if let Some(index) = self.member_indexes.get(&key) {
            return index.clone();
        }
        let mut index = MemberIndex::new();
        for declaration in declarations.iter().copied() {
            for (position, member) in declaration.body.members.iter().enumerate() {
                if let Some(name) = member_name(&member.id) {
                    index.entry(name).or_insert((declaration, position));
                }
            }
        }
        let index = Rc::new(index);
        self.member_indexes.insert(key, index.clone());
        index
    }

    fn evaluate(&mut self, expression: &'b Expression<'a>, usage: &Usage<'b, 'a>) -> Evaluated {
        if self.nesting > 256 {
            return Evaluated::Unresolved;
        }
        self.nesting += 1;
        let value = self.evaluate_expression(expression, usage);
        self.nesting -= 1;
        value
    }

    /// The evaluator package's `evaluate`. Only parentheses are looked
    /// through: an `as`, `!` or `satisfies` has no value.
    fn evaluate_expression(&mut self, expression: &'b Expression<'a>, usage: &Usage<'b, 'a>) -> Evaluated {
        match expression {
            Expression::ParenthesizedExpression(parenthesized) => {
                self.evaluate(&parenthesized.expression, usage)
            }
            Expression::NumericLiteral(literal) => number(literal.value),
            Expression::StringLiteral(literal) => {
                Evaluated::Value(EnumValue::String(literal.value.as_str().to_string()))
            }
            Expression::TemplateLiteral(template) => self.evaluate_template(template, usage),
            Expression::UnaryExpression(unary) => {
                let operation: fn(f64) -> f64 = match unary.operator {
                    UnaryOperator::UnaryPlus => |value| value,
                    UnaryOperator::UnaryNegation => |value| -value,
                    UnaryOperator::BitwiseNot => |value| f64::from(!to_int32(value)),
                    _ => return Evaluated::Computed,
                };
                match self.evaluate(&unary.argument, usage) {
                    Evaluated::Value(EnumValue::Number(value)) => number(operation(value)),
                    Evaluated::Unresolved => Evaluated::Unresolved,
                    _ => Evaluated::Computed,
                }
            }
            Expression::BinaryExpression(binary) => {
                self.evaluate_binary(binary.operator, &binary.left, &binary.right, usage)
            }
            Expression::Identifier(identifier) => {
                self.evaluate_identifier(identifier.name.as_str(), usage)
            }
            Expression::StaticMemberExpression(member) => self.evaluate_property_access(member, usage),
            Expression::ComputedMemberExpression(member) => self.evaluate_element_access(member, usage),
            // tsc reads `a?.b` as the entity `a.b`; the chain is not followed here.
            Expression::ChainExpression(_) => Evaluated::Unresolved,
            _ => Evaluated::Computed,
        }
    }

    fn evaluate_template(&mut self, template: &'b TemplateLiteral<'a>, usage: &Usage<'b, 'a>) -> Evaluated {
        let Some(mut text) = cooked(template, 0).map(str::to_string) else {
            return Evaluated::Unresolved;
        };
        let mut unresolved = false;
        for (index, expression) in template.expressions.iter().enumerate() {
            match self.evaluate(expression, usage) {
                Evaluated::Value(value) => text.push_str(&value.to_js_string()),
                Evaluated::Computed => return Evaluated::Computed,
                Evaluated::Unresolved => unresolved = true,
            }
            let Some(quasi) = cooked(template, index + 1) else {
                return Evaluated::Unresolved;
            };
            text.push_str(quasi);
        }
        if unresolved {
            Evaluated::Unresolved
        } else {
            Evaluated::Value(EnumValue::String(text))
        }
    }

    fn evaluate_binary(
        &mut self,
        operator: BinaryOperator,
        left: &'b Expression<'a>,
        right: &'b Expression<'a>,
        usage: &Usage<'b, 'a>,
    ) -> Evaluated {
        if !is_arithmetic(operator) {
            return Evaluated::Computed;
        }
        let left = self.evaluate(left, usage);
        let right = self.evaluate(right, usage);
        let addition = operator == BinaryOperator::Addition;
        match (left, right) {
            (Evaluated::Value(EnumValue::Number(left)), Evaluated::Value(EnumValue::Number(right))) => {
                number(apply_binary(operator, left, right))
            }
            (Evaluated::Value(left), Evaluated::Value(right)) if addition => {
                Evaluated::Value(EnumValue::String(left.to_js_string() + &right.to_js_string()))
            }
            (Evaluated::Value(EnumValue::String(_)), _) | (_, Evaluated::Value(EnumValue::String(_)))
                if !addition =>
            {
                Evaluated::Computed
            }
            (Evaluated::Unresolved, Evaluated::Value(_) | Evaluated::Unresolved)
            | (Evaluated::Value(_), Evaluated::Unresolved) => Evaluated::Unresolved,
            _ => Evaluated::Computed,
        }
    }

    /// `evaluateEntity` for a bare name: the enum's own members first, then
    /// the enclosing scopes, then the globals.
    fn evaluate_identifier(&mut self, name: &'b str, usage: &Usage<'b, 'a>) -> Evaluated {
        if let Some((own, _)) = &usage.own
            && let Some(&(declaration, index)) = own.get(name)
        {
            return self.evaluate_enum_member(declaration, index, usage.depth, usage);
        }
        match self.lookup(name, usage.depth) {
            Some((depth, Binding::Constant { declarator, ambient })) => {
                self.evaluate_constant(declarator, depth, ambient, usage)
            }
            Some((_, Binding::Value | Binding::Enum { .. })) => Evaluated::Computed,
            Some((_, Binding::Opaque)) => Evaluated::Unresolved,
            None if self.detached => Evaluated::Unresolved,
            None => match name {
                "NaN" => number(f64::NAN),
                "Infinity" => number(f64::INFINITY),
                "undefined" => Evaluated::Computed,
                _ => Evaluated::Unresolved,
            },
        }
    }

    /// `X.Y`: tsc resolves `X` as a namespace, which a plain value is not, and
    /// only an enum member has a value.
    fn evaluate_property_access(
        &mut self,
        member: &'b StaticMemberExpression<'a>,
        usage: &Usage<'b, 'a>,
    ) -> Evaluated {
        if member.optional {
            return Evaluated::Unresolved;
        }
        let Some(root) = entity_root(&member.object) else {
            return Evaluated::Computed;
        };
        let direct = matches!(member.object, Expression::Identifier(_));
        match self.lookup_namespace(root, usage.depth) {
            (Some((depth, Binding::Enum { declarations, merged })), _) if direct => {
                let index = self.member_index(&declarations);
                match index.get(member.property.name.as_str()) {
                    Some(&(declaration, position)) => {
                        self.evaluate_enum_member(declaration, position, depth, usage)
                    }
                    None if merged || !self.is_closed(depth) => Evaluated::Unresolved,
                    None => Evaluated::Computed,
                }
            }
            // `E.A.B`: an enum member holds nothing.
            (Some((_, Binding::Enum { merged: false, .. })), _) => Evaluated::Computed,
            (None, true) => Evaluated::Computed,
            _ => Evaluated::Unresolved,
        }
    }

    /// `E["A"]`: tsc resolves `E` as a value and reads a member only from an
    /// enum.
    fn evaluate_element_access(
        &mut self,
        member: &'b ComputedMemberExpression<'a>,
        usage: &Usage<'b, 'a>,
    ) -> Evaluated {
        if member.optional {
            return Evaluated::Unresolved;
        }
        if entity_root(&member.object).is_none() {
            return Evaluated::Computed;
        }
        let name = match &member.expression {
            Expression::StringLiteral(literal) => literal.value.as_str(),
            Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
                match cooked(template, 0) {
                    Some(name) => name,
                    None => return Evaluated::Unresolved,
                }
            }
            _ => return Evaluated::Computed,
        };
        let Expression::Identifier(root) = &member.object else {
            return Evaluated::Unresolved;
        };
        let root = root.name.as_str();
        if usage.own.as_ref().is_some_and(|(own, _)| own.contains_key(root)) {
            return Evaluated::Computed;
        }
        match self.lookup(root, usage.depth) {
            Some((depth, Binding::Enum { declarations, merged })) => {
                let index = self.member_index(&declarations);
                match index.get(name) {
                    Some(&(declaration, position)) => {
                        self.evaluate_enum_member(declaration, position, depth, usage)
                    }
                    None if merged || !self.is_closed(depth) => Evaluated::Unresolved,
                    None => Evaluated::Computed,
                }
            }
            Some((_, Binding::Constant { .. } | Binding::Value)) => Evaluated::Computed,
            _ => Evaluated::Unresolved,
        }
    }

    /// `evaluateEnumMember`: a member's own name has no value yet (TS2565), and
    /// one declared after the use evaluates to 0 (TS2651).
    fn evaluate_enum_member(
        &mut self,
        declaration: &'b TSEnumDeclaration<'a>,
        index: usize,
        depth: usize,
        usage: &Usage<'b, 'a>,
    ) -> Evaluated {
        let Some(member) = declaration.body.members.get(index) else {
            return Evaluated::Unresolved;
        };
        if usage.own.as_ref().is_some_and(|(_, start)| *start == member.span.start) {
            return Evaluated::Computed;
        }
        if !self.declared_before_use(member.span.start, depth, usage) {
            return number(0.0);
        }
        let start = declaration.span.start;
        if !self.enums.contains_key(&start) {
            self.evaluate_enum(declaration, depth);
        }
        match self.enums.get(&start) {
            Some(EnumState::Done(evaluation)) => evaluation
                .members
                .get(index)
                .map_or(Evaluated::Unresolved, |member| member.value.clone()),
            // tsc reads the value the member has so far: none for one not
            // computed yet.
            Some(EnumState::InProgress(values)) => {
                values.get(index).cloned().unwrap_or(Evaluated::Computed)
            }
            None => Evaluated::Unresolved,
        }
    }

    /// `evaluateEntity` for a constant variable: its initializer's value,
    /// where the declaration is visible before the use.
    fn evaluate_constant(
        &mut self,
        declarator: &'b VariableDeclarator<'a>,
        depth: usize,
        ambient: bool,
        usage: &Usage<'b, 'a>,
    ) -> Evaluated {
        let start = declarator.span.start;
        if usage.constant == Some(start) || !self.declared_before_use(start, depth, usage) {
            return Evaluated::Computed;
        }
        match self.constants.get(&start) {
            Some(Some(value)) => return value.clone(),
            Some(None) => return Evaluated::Unresolved,
            None => {}
        }
        let Some(initializer) = &declarator.init else {
            return Evaluated::Computed;
        };
        self.constants.insert(start, None);
        let inner = Usage { depth, position: start, ambient, own: None, constant: Some(start) };
        let value = self.evaluate(initializer, &inner);
        self.constants.insert(start, Some(value.clone()));
        value
    }

    /// tsc's `isBlockScopedNameDeclaredBeforeUse` for a declaration made in
    /// the scope at `depth`: written before the use, used from an ambient
    /// context, or used inside a function that runs later.
    fn declared_before_use(&self, declaration: u32, depth: usize, usage: &Usage<'b, 'a>) -> bool {
        usage.ambient
            || declaration <= usage.position
            || self
                .scopes
                .iter()
                .take(usage.depth + 1)
                .skip(depth + 1)
                .any(|scope| scope.kind == ScopeKind::Function)
    }

    fn lookup(&self, name: &str, depth: usize) -> Option<(usize, Binding<'b, 'a>)> {
        let visible = self.scopes.len().min(depth + 1);
        self.scopes[..visible]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, scope)| scope.names.get(name).map(|binding| (index, binding.clone())))
    }

    /// [`Self::lookup`] for a name resolved as a namespace, which a plain value
    /// does not answer; the flag says whether one was passed over.
    fn lookup_namespace(&self, name: &str, depth: usize) -> (Option<(usize, Binding<'b, 'a>)>, bool) {
        let visible = self.scopes.len().min(depth + 1);
        let mut passed_value = false;
        for (index, scope) in self.scopes[..visible].iter().enumerate().rev() {
            match scope.names.get(name) {
                Some(Binding::Constant { .. } | Binding::Value) => passed_value = true,
                Some(binding) => return (Some((index, binding.clone())), passed_value),
                None => {}
            }
        }
        (None, passed_value)
    }

    /// Whether every declaration of an enum made in the scope at `depth` is in
    /// this file: a block's or function's is, and so is one at a module's top
    /// level; a script's top level and a namespace body merge across files.
    fn is_closed(&self, depth: usize) -> bool {
        self.scopes
            .get(depth)
            .is_some_and(|scope| scope.kind != ScopeKind::Container || (depth == 0 && self.module))
    }
}

fn declare<'b, 'a>(names: &mut Names<'b, 'a>, name: &'b str, binding: Binding<'b, 'a>) {
    match names.entry(name) {
        Entry::Vacant(slot) => {
            slot.insert(binding);
        }
        Entry::Occupied(mut slot) => match (slot.get_mut(), binding) {
            (Binding::Enum { declarations, .. }, Binding::Enum { declarations: more, .. }) => {
                declarations.extend(more);
            }
            (Binding::Enum { merged, .. }, _) => *merged = true,
            (existing, Binding::Enum { declarations, .. }) => {
                *existing = Binding::Enum { declarations, merged: true };
            }
            _ => {}
        },
    }
}

fn declare_statement<'b, 'a>(statement: &'b Statement<'a>, ambient: bool, names: &mut Names<'b, 'a>) {
    if let Some(declaration) = statement.as_declaration() {
        declare_declaration(declaration, ambient, names);
        return;
    }
    match statement {
        Statement::ImportDeclaration(import) => {
            if let Some(specifiers) = &import.specifiers {
                for specifier in specifiers.iter() {
                    let local = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => &specifier.local,
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => &specifier.local,
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => &specifier.local,
                    };
                    declare(names, local.name.as_str(), Binding::Opaque);
                }
            }
        }
        Statement::ExportNamedDeclaration(export) => {
            if let Some(declaration) = &export.declaration {
                declare_declaration(declaration, ambient, names);
            }
        }
        Statement::ExportDefaultDeclaration(export) => {
            let id = match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => function.id.as_ref(),
                ExportDefaultDeclarationKind::ClassDeclaration(class) => class.id.as_ref(),
                _ => None,
            };
            if let Some(id) = id {
                declare(names, id.name.as_str(), Binding::Value);
            }
        }
        _ => {}
    }
}

fn declare_declaration<'b, 'a>(declaration: &'b Declaration<'a>, ambient: bool, names: &mut Names<'b, 'a>) {
    match declaration {
        Declaration::VariableDeclaration(variable) if variable.kind != VariableDeclarationKind::Var => {
            declare_variables(variable, ambient, names);
        }
        Declaration::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                declare(names, id.name.as_str(), Binding::Value);
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                declare(names, id.name.as_str(), Binding::Value);
            }
        }
        Declaration::TSEnumDeclaration(enumeration) => {
            let binding = Binding::Enum { declarations: vec![&**enumeration], merged: false };
            declare(names, enumeration.id.name.as_str(), binding);
        }
        Declaration::TSModuleDeclaration(module) => {
            if let TSModuleDeclarationName::Identifier(id) = &module.id {
                declare(names, id.name.as_str(), Binding::Opaque);
            }
        }
        Declaration::TSImportEqualsDeclaration(import) => {
            declare(names, import.id.name.as_str(), Binding::Opaque);
        }
        _ => {}
    }
}

fn declare_variables<'b, 'a>(variable: &'b VariableDeclaration<'a>, ambient: bool, names: &mut Names<'b, 'a>) {
    let constant_kind =
        !matches!(variable.kind, VariableDeclarationKind::Var | VariableDeclarationKind::Let);
    for declarator in variable.declarations.iter() {
        match &declarator.id {
            BindingPattern::BindingIdentifier(identifier) => {
                let binding = if constant_kind
                    && declarator.type_annotation.is_none()
                    && declarator.init.is_some()
                {
                    Binding::Constant { declarator, ambient: ambient || variable.declare }
                } else {
                    Binding::Value
                };
                declare(names, identifier.name.as_str(), binding);
            }
            pattern => {
                for identifier in pattern.get_binding_identifiers() {
                    declare(names, identifier.name.as_str(), Binding::Value);
                }
            }
        }
    }
}

/// The `var`s a function body, namespace body or file declares through its
/// nested statements, which all bind in it.
fn hoist_vars<'b, 'a>(statement: &'b Statement<'a>, names: &mut Names<'b, 'a>) {
    match statement {
        Statement::VariableDeclaration(variable) => hoist_var_declaration(variable, names),
        Statement::ExportNamedDeclaration(export) => {
            if let Some(Declaration::VariableDeclaration(variable)) = &export.declaration {
                hoist_var_declaration(variable, names);
            }
        }
        Statement::BlockStatement(block) => {
            block.body.iter().for_each(|statement| hoist_vars(statement, names));
        }
        Statement::IfStatement(statement) => {
            hoist_vars(&statement.consequent, names);
            if let Some(alternate) = &statement.alternate {
                hoist_vars(alternate, names);
            }
        }
        Statement::ForStatement(statement) => {
            if let Some(ForStatementInit::VariableDeclaration(variable)) = &statement.init {
                hoist_var_declaration(variable, names);
            }
            hoist_vars(&statement.body, names);
        }
        Statement::ForInStatement(statement) => {
            if let Some(variable) = loop_head(&statement.left) {
                hoist_var_declaration(variable, names);
            }
            hoist_vars(&statement.body, names);
        }
        Statement::ForOfStatement(statement) => {
            if let Some(variable) = loop_head(&statement.left) {
                hoist_var_declaration(variable, names);
            }
            hoist_vars(&statement.body, names);
        }
        Statement::WhileStatement(statement) => hoist_vars(&statement.body, names),
        Statement::DoWhileStatement(statement) => hoist_vars(&statement.body, names),
        Statement::LabeledStatement(statement) => hoist_vars(&statement.body, names),
        Statement::WithStatement(statement) => hoist_vars(&statement.body, names),
        Statement::TryStatement(statement) => {
            statement.block.body.iter().for_each(|statement| hoist_vars(statement, names));
            if let Some(handler) = &statement.handler {
                handler.body.body.iter().for_each(|statement| hoist_vars(statement, names));
            }
            if let Some(finalizer) = &statement.finalizer {
                finalizer.body.iter().for_each(|statement| hoist_vars(statement, names));
            }
        }
        Statement::SwitchStatement(statement) => {
            for case in statement.cases.iter() {
                case.consequent.iter().for_each(|statement| hoist_vars(statement, names));
            }
        }
        _ => {}
    }
}

fn hoist_var_declaration<'b, 'a>(variable: &'b VariableDeclaration<'a>, names: &mut Names<'b, 'a>) {
    if variable.kind != VariableDeclarationKind::Var {
        return;
    }
    for declarator in variable.declarations.iter() {
        for identifier in declarator.id.get_binding_identifiers() {
            declare(names, identifier.name.as_str(), Binding::Value);
        }
    }
}

fn loop_head<'b, 'a>(left: &'b ForStatementLeft<'a>) -> Option<&'b VariableDeclaration<'a>> {
    match left {
        ForStatementLeft::VariableDeclaration(declaration) => Some(&**declaration),
        _ => None,
    }
}

pub(crate) fn member_name<'b>(name: &'b TSEnumMemberName<'_>) -> Option<&'b str> {
    match name {
        TSEnumMemberName::Identifier(identifier) if identifier.name.is_empty() => None,
        TSEnumMemberName::Identifier(identifier) => Some(identifier.name.as_str()),
        TSEnumMemberName::String(literal) | TSEnumMemberName::ComputedString(literal) => {
            Some(literal.value.as_str())
        }
        TSEnumMemberName::ComputedTemplateString(_) => None,
    }
}

/// The first name of an entity name expression (`a`, `a.b.c`), the only
/// access tsc's evaluator follows; `None` for anything else (`(a).b`, `f().b`).
fn entity_root<'b>(expression: &'b Expression<'_>) -> Option<&'b str> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
        Expression::StaticMemberExpression(member) if !member.optional => entity_root(&member.object),
        _ => None,
    }
}

fn cooked<'b>(template: &'b TemplateLiteral<'_>, index: usize) -> Option<&'b str> {
    template.quasis.get(index)?.value.cooked.as_ref().map(|cooked| cooked.as_str())
}

fn number(value: f64) -> Evaluated {
    Evaluated::Value(EnumValue::Number(value))
}

fn is_arithmetic(operator: BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::BitwiseOR
            | BinaryOperator::BitwiseAnd
            | BinaryOperator::BitwiseXOR
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
            | BinaryOperator::ShiftRightZeroFill
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Addition
            | BinaryOperator::Subtraction
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential
    )
}

/// The `jsnum` operations, over values both of which are numbers.
fn apply_binary(operator: BinaryOperator, left: f64, right: f64) -> f64 {
    match operator {
        BinaryOperator::BitwiseOR => f64::from(to_int32(left) | to_int32(right)),
        BinaryOperator::BitwiseAnd => f64::from(to_int32(left) & to_int32(right)),
        BinaryOperator::BitwiseXOR => f64::from(to_int32(left) ^ to_int32(right)),
        BinaryOperator::ShiftLeft => f64::from(to_int32(left).wrapping_shl(shift_count(right))),
        BinaryOperator::ShiftRight => f64::from(to_int32(left) >> shift_count(right)),
        BinaryOperator::ShiftRightZeroFill => f64::from(to_uint32(left) >> shift_count(right)),
        BinaryOperator::Multiplication => left * right,
        BinaryOperator::Division => left / right,
        BinaryOperator::Addition => left + right,
        BinaryOperator::Subtraction => left - right,
        BinaryOperator::Remainder => remainder(left, right),
        BinaryOperator::Exponential => exponentiate(left, right),
        _ => f64::NAN,
    }
}

/// ECMAScript `ToInt32`.
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value.trunc().rem_euclid(4_294_967_296.0) as u32 as i32
}

fn to_uint32(value: f64) -> u32 {
    to_int32(value) as u32
}

fn shift_count(value: f64) -> u32 {
    to_uint32(value) & 31
}

fn remainder(dividend: f64, divisor: f64) -> f64 {
    if dividend.is_nan() || divisor.is_nan() || dividend.is_infinite() || divisor == 0.0 {
        return f64::NAN;
    }
    if divisor.is_infinite() || dividend == 0.0 {
        return dividend;
    }
    dividend % divisor
}

fn exponentiate(base: f64, exponent: f64) -> f64 {
    if (base == 1.0 || base == -1.0) && exponent.is_infinite() {
        return f64::NAN;
    }
    if base == 1.0 && exponent.is_nan() {
        return f64::NAN;
    }
    base.powf(exponent)
}
