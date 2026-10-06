//! Unreachable code (TS7027), as the binder's control-flow marking decides it.
//!
//! tsc's binder threads a flow node through each container; once it hits
//! `unreachableFlow` every later statement of the container is marked, and
//! `checkSourceElementUnreachable` reports the first marked *potentially
//! executable* statement of a statement list together with the consecutive
//! marked statements after it, then skips them. Only what the binder can see
//! is modelled: `return`/`throw`/`break`/`continue`, loops whose exit
//! condition is the literal `true`, branches guarded by a literal `false`,
//! `try`/`finally` completion, and a non-async, non-generator IIFE that never
//! completes. Of what the checker adds on top, a `FlowCall` whose effects
//! signature the file itself declares is modelled (see [`call_effects`]);
//! calls that need types, and exhaustive `switch`, are not.

mod call_effects;

use oxc_ast::ast::{
    AccessorProperty, ArrowFunctionExpression, Class, Declaration, ExportDefaultDeclarationKind,
    Expression, ForStatementInit, Function, FunctionBody, LogicalOperator, MethodDefinition,
    ModuleExportName, Program, PropertyDefinition, Statement, StaticBlock, TSModuleBlock,
    TSModuleDeclaration, TSModuleDeclarationBody, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};
use oxc_syntax::operator::UnaryOperator;
use oxc_syntax::scope::ScopeFlags;

use crate::{ParsedGrammarDiagnostic, ParsedGrammarDiagnosticKind as Kind, TextSpan};

/// Marks a TS7027 run that holds only without tsgo's
/// `ShouldPreserveConstEnums`, and one that holds only with it: a const enum,
/// or a namespace holding nothing else, is then executable.
pub const UNREACHABLE_UNPRESERVED_CONST_ENUMS: &str = "\u{1}unpreserved";
pub const UNREACHABLE_PRESERVED_CONST_ENUMS: &str = "\u{1}preserved";
/// Tags the TS1203 finding of a declaration file's `export =`.
pub const AMBIENT_EXPORT_ASSIGNMENT: &str = "\u{1}ambient";

thread_local! {
    static PRESERVE_CONST_ENUMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn preserves_const_enums() -> bool {
    PRESERVE_CONST_ENUMS.with(std::cell::Cell::get)
}

pub(crate) fn collect_unreachable_code(program: &Program<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    let start = out.len();
    let mut collector = UnreachableCollector {
        out,
        reported: Vec::new(),
        calls: call_effects::CallEffects::new(program.source_text),
    };
    collector.visit_program(program);
    // Only a file with a const enum can tell the two option values apart.
    if !program.source_text.contains("const enum") {
        return;
    }
    for finding in &mut out[start..] {
        if finding.kind == Kind::Ts(7027) {
            finding.name = Some(UNREACHABLE_UNPRESERVED_CONST_ENUMS.to_string());
        }
    }
    let mut preserved = Vec::new();
    PRESERVE_CONST_ENUMS.with(|flag| flag.set(true));
    let mut collector = UnreachableCollector {
        out: &mut preserved,
        reported: Vec::new(),
        calls: call_effects::CallEffects::new(program.source_text),
    };
    collector.visit_program(program);
    PRESERVE_CONST_ENUMS.with(|flag| flag.set(false));
    out.extend(preserved.into_iter().filter(|finding| finding.kind == Kind::Ts(7027)).map(|mut finding| {
        finding.name = Some(UNREACHABLE_PRESERVED_CONST_ENUMS.to_string());
        finding
    }));
}

/// Finds every control-flow container and runs the statement walk on it.
/// A statement the walk reported is not descended into, exactly as the
/// checker skips it, so nothing nested in it reports again.
struct UnreachableCollector<'o> {
    out: &'o mut Vec<ParsedGrammarDiagnostic>,
    reported: Vec<Span>,
    calls: call_effects::CallEffects,
}

impl UnreachableCollector<'_> {
    fn analyze(&mut self, statements: &[Statement<'_>]) {
        let mut flow = Flow {
            frames: Vec::new(),
            returned: false,
            reported: Vec::new(),
            after_exhaustive_switch: Vec::new(),
            calls: Some(&self.calls),
        };
        flow.bind_statements(statements, true);
        for (switch, run) in flow.after_exhaustive_switch {
            self.out.push(ParsedGrammarDiagnostic {
                kind: Kind::UnreachableAfterExhaustiveSwitch,
                span: TextSpan {
                    start: run.first.start as usize,
                    end: run.last_end as usize,
                },
                name: Some(format!("{}\0{}", switch.start, switch.end)),
            });
        }
        for run in flow.reported {
            // Every statement of the run is reported (`reportedUnreachableNodes`),
            // so the checker descends into none of them.
            self.reported.extend(run.statements);
            self.out.push(ParsedGrammarDiagnostic {
                kind: Kind::Ts(7027),
                span: TextSpan {
                    start: run.first.start as usize,
                    end: run.last_end as usize,
                },
                name: None,
            });
        }
    }
}

impl<'a> Visit<'a> for UnreachableCollector<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.calls.enter_statements(&program.body);
        self.analyze(&program.body);
        oxc_ast_visit::walk::walk_program(self, program);
        self.calls.leave();
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        self.calls.enter_function(function);
        oxc_ast_visit::walk::walk_function(self, function, flags);
        self.calls.leave();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.calls.enter_arrow(arrow);
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.calls.leave_arrow();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.calls.enter_class(class);
        oxc_ast_visit::walk::walk_class(self, class);
        self.calls.leave_class();
    }

    fn visit_method_definition(&mut self, method: &MethodDefinition<'a>) {
        self.calls.enter_method(method);
        oxc_ast_visit::walk::walk_method_definition(self, method);
        self.calls.leave_method();
    }

    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        self.calls.enter_property(property.r#static);
        oxc_ast_visit::walk::walk_property_definition(self, property);
        self.calls.leave_property();
    }

    fn visit_accessor_property(&mut self, property: &AccessorProperty<'a>) {
        self.calls.enter_property(property.r#static);
        oxc_ast_visit::walk::walk_accessor_property(self, property);
        self.calls.leave_property();
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.analyze(&body.statements);
        oxc_ast_visit::walk::walk_function_body(self, body);
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.calls.enter_static_block(block);
        self.analyze(&block.body);
        oxc_ast_visit::walk::walk_static_block(self, block);
        self.calls.leave();
    }

    fn visit_ts_module_block(&mut self, block: &TSModuleBlock<'a>) {
        self.calls.enter_statements(&block.body);
        self.analyze(&block.body);
        oxc_ast_visit::walk::walk_ts_module_block(self, block);
        self.calls.leave();
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        if self.reported.contains(&statement.span()) {
            return;
        }
        oxc_ast_visit::walk::walk_statement(self, statement);
    }
}

struct ReportedRun {
    first: Span,
    last_end: u32,
    statements: Vec<Span>,
}

/// The target a `break`/`continue` can name: a loop, a `switch`, or a label
/// on some other statement. Labels written directly on a loop or `switch`
/// belong to that frame, since tsc's `setContinueTarget` and the label's
/// break target both resolve to the statement's own exit.
struct Frame {
    kind: FrameKind,
    labels: Vec<String>,
    broke: bool,
    continued: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum FrameKind {
    Loop,
    Switch,
    Label,
}

struct Flow<'c> {
    frames: Vec<Frame>,
    /// A `return` was bound on a reachable path of the current container;
    /// what makes an IIFE's call complete.
    returned: bool,
    reported: Vec<ReportedRun>,
    /// The runs a `switch` would cut off if its cases were exhaustive, which
    /// only the checker can tell.
    after_exhaustive_switch: Vec<(Span, ReportedRun)>,
    /// `None` inside an IIFE body, whose calls are not resolved.
    calls: Option<&'c call_effects::CallEffects>,
}

impl Flow<'_> {
    /// Binds a statement list starting with `reachable` flow; returns whether
    /// flow is reachable after the last statement. Once flow is unreachable
    /// it stays so for the rest of the list, and the consecutive potentially
    /// executable statements from each unreachable one are one report.
    fn bind_statements(&mut self, statements: &[Statement<'_>], mut reachable: bool) -> bool {
        let mut index = 0;
        while index < statements.len() {
            let statement = &statements[index];
            if reachable {
                if let Statement::SwitchStatement(switch) = statement {
                    let (end_reachable, default_only) = self.bind_switch_exits(switch, Vec::new());
                    if default_only {
                        let run = first_executable_run(&statements[index + 1..]);
                        self.after_exhaustive_switch.extend(run.map(|run| (switch.span, run)));
                    }
                    reachable = end_reachable;
                } else {
                    reachable = self.bind_statement(statement);
                }
                index += 1;
                continue;
            }
            if !is_potentially_executable(statement) {
                if let Statement::BlockStatement(block) = statement {
                    self.bind_statements(&block.body, false);
                }
                index += 1;
                continue;
            }
            let mut last = index;
            while last + 1 < statements.len() && is_potentially_executable(&statements[last + 1]) {
                last += 1;
            }
            self.reported.push(ReportedRun {
                first: statement.span(),
                last_end: statements[last].span().end,
                statements: statements[index..=last].iter().map(GetSpan::span).collect(),
            });
            index = last + 1;
        }
        reachable
    }

    fn bind_statement(&mut self, statement: &Statement<'_>) -> bool {
        match statement {
            Statement::BlockStatement(block) => self.bind_statements(&block.body, true),
            Statement::ReturnStatement(_) => {
                self.returned = true;
                false
            }
            Statement::ThrowStatement(_) => false,
            Statement::BreakStatement(statement) => {
                self.jump(statement.label.as_ref().map(|label| label.name.as_str()), true);
                false
            }
            Statement::ContinueStatement(statement) => {
                self.jump(statement.label.as_ref().map(|label| label.name.as_str()), false);
                false
            }
            Statement::ExpressionStatement(statement) => {
                !diverges(&statement.expression) && !self.call_ends_flow(&statement.expression, true)
            }
            Statement::VariableDeclaration(declaration) => !declaration.declarations.iter().any(|declarator| {
                declarator.init.as_ref().is_some_and(|init| diverges(init) || self.call_ends_flow(init, false))
            }),
            Statement::IfStatement(statement) => {
                let then_end =
                    self.bind_statement_from(&statement.consequent, !definitely_false(&statement.test));
                let else_start = !definitely_true(&statement.test);
                let else_end = match &statement.alternate {
                    Some(alternate) => self.bind_statement_from(alternate, else_start),
                    None => else_start,
                };
                then_end || else_end
            }
            Statement::WhileStatement(statement) => {
                self.push_frame(FrameKind::Loop, Vec::new());
                self.bind_statement_from(&statement.body, !definitely_false(&statement.test));
                let frame = self.pop_frame();
                !definitely_true(&statement.test) || frame.broke
            }
            Statement::DoWhileStatement(statement) => {
                self.push_frame(FrameKind::Loop, Vec::new());
                let body_end = self.bind_statement_from(&statement.body, true);
                let frame = self.pop_frame();
                let condition_reachable = body_end || frame.continued;
                (condition_reachable && !definitely_true(&statement.test)) || frame.broke
            }
            Statement::ForStatement(statement) => {
                let init_diverges = match &statement.init {
                    Some(ForStatementInit::VariableDeclaration(declaration)) => {
                        declaration.declarations.iter().any(|declarator| {
                            declarator
                                .init
                                .as_ref()
                                .is_some_and(|init| diverges(init) || self.call_ends_flow(init, false))
                        })
                    }
                    Some(init) => init
                        .as_expression()
                        .is_some_and(|init| diverges(init) || self.call_ends_flow(init, false)),
                    None => false,
                };
                if init_diverges {
                    return false;
                }
                self.push_frame(FrameKind::Loop, Vec::new());
                let body_start = statement.test.as_ref().is_none_or(|test| !definitely_false(test));
                self.bind_statement_from(&statement.body, body_start);
                let frame = self.pop_frame();
                statement.test.as_ref().is_some_and(|test| !definitely_true(test)) || frame.broke
            }
            Statement::ForInStatement(statement) => {
                self.push_frame(FrameKind::Loop, Vec::new());
                self.bind_statement_from(&statement.body, true);
                self.pop_frame();
                true
            }
            Statement::ForOfStatement(statement) => {
                self.push_frame(FrameKind::Loop, Vec::new());
                self.bind_statement_from(&statement.body, true);
                self.pop_frame();
                true
            }
            Statement::SwitchStatement(statement) => self.bind_switch(statement, Vec::new()),
            Statement::LabeledStatement(statement) => self.bind_labeled(statement, Vec::new()),
            Statement::TryStatement(statement) => {
                let try_end = self.bind_statements(&statement.block.body, true);
                let catch_end = statement
                    .handler
                    .as_ref()
                    .map(|handler| self.bind_statements(&handler.body.body, true));
                let normal_exit = try_end || catch_end.unwrap_or(false);
                match &statement.finalizer {
                    Some(finalizer) => self.bind_statements(&finalizer.body, true) && normal_exit,
                    None => normal_exit,
                }
            }
            Statement::WithStatement(statement) => self.bind_statement_from(&statement.body, true),
            _ => true,
        }
    }

    fn bind_statement_from(&mut self, statement: &Statement<'_>, reachable: bool) -> bool {
        self.bind_statements(std::slice::from_ref(statement), reachable)
    }

    fn call_ends_flow(&self, expression: &Expression<'_>, whole_statement: bool) -> bool {
        self.calls.is_some_and(|calls| calls.ends_flow(expression, whole_statement))
    }

    fn bind_switch(&mut self, statement: &oxc_ast::ast::SwitchStatement<'_>, labels: Vec<String>) -> bool {
        self.bind_switch_exits(statement, labels).0
    }

    /// Whether flow continues past the `switch`, and whether only the
    /// implicit `default` clause carries it there.
    fn bind_switch_exits(
        &mut self,
        statement: &oxc_ast::ast::SwitchStatement<'_>,
        labels: Vec<String>,
    ) -> (bool, bool) {
        self.push_frame(FrameKind::Switch, labels);
        let mut last_end = true;
        for case in &statement.cases {
            last_end = self.bind_statements(&case.consequent, true);
        }
        let frame = self.pop_frame();
        let has_default = statement.cases.iter().any(|case| case.test.is_none());
        let default_only = !last_end && !frame.broke && !has_default;
        (last_end || frame.broke || !has_default, default_only)
    }

    /// Labels directly on a loop or `switch` join that statement's frame;
    /// any other labeled statement gets a frame of its own.
    fn bind_labeled(&mut self, statement: &oxc_ast::ast::LabeledStatement<'_>, mut labels: Vec<String>) -> bool {
        labels.push(statement.label.name.to_string());
        match &statement.body {
            Statement::LabeledStatement(inner) => self.bind_labeled(inner, labels),
            Statement::SwitchStatement(inner) => self.bind_switch(inner, labels),
            Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_)
            | Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_) => {
                self.push_frame(FrameKind::Label, labels.clone());
                let end = self.bind_statement(&statement.body);
                let label_frame = self.pop_frame();
                end || label_frame.broke
            }
            body => {
                self.push_frame(FrameKind::Label, labels);
                let end = self.bind_statement(body);
                let frame = self.pop_frame();
                end || frame.broke
            }
        }
    }

    fn push_frame(&mut self, kind: FrameKind, labels: Vec<String>) {
        self.frames.push(Frame {
            kind,
            labels,
            broke: false,
            continued: false,
        });
    }

    fn pop_frame(&mut self) -> Frame {
        self.frames.pop().expect("frame stack underflow")
    }

    /// tsc's `bindBreakOrContinueStatement`: a labeled jump targets the
    /// active label, an unlabeled `break` the innermost loop or `switch`, an
    /// unlabeled `continue` the innermost loop. A `continue label` on a loop
    /// wrapped only by a label frame reaches the loop through that frame.
    fn jump(&mut self, label: Option<&str>, is_break: bool) {
        let index = match label {
            Some(label) => self
                .frames
                .iter()
                .rposition(|frame| frame.labels.iter().any(|candidate| candidate == label)),
            None => self.frames.iter().rposition(|frame| match frame.kind {
                FrameKind::Loop => true,
                FrameKind::Switch => is_break,
                FrameKind::Label => false,
            }),
        };
        let Some(index) = index else {
            return;
        };
        if is_break {
            self.frames[index].broke = true;
        } else if self.frames[index].kind == FrameKind::Loop {
            self.frames[index].continued = true;
        } else if self.frames[index].kind == FrameKind::Label
            && let Some(loop_frame) = self.frames.get_mut(index + 1)
            && loop_frame.kind == FrameKind::Loop
        {
            loop_frame.continued = true;
        }
    }
}

/// tsc's `IsPotentiallyExecutableNode`: the statement kinds proper (not a
/// block, an empty statement, or a declaration), a `let`/`const`/`using`
/// statement, a `var` statement with some initializer, and a class, enum, or
/// namespace declaration — minus `isSourceElementUnreachable`'s exemptions
/// for a `const enum` and a non-instantiated namespace.
fn is_potentially_executable(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::VariableDeclaration(declaration) => variable_statement_is_executable(declaration),
        Statement::ExpressionStatement(_)
        | Statement::IfStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::WhileStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ReturnStatement(_)
        | Statement::WithStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::LabeledStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::TryStatement(_)
        | Statement::DebuggerStatement(_)
        | Statement::ClassDeclaration(_) => true,
        Statement::TSEnumDeclaration(declaration) => !declaration.r#const || preserves_const_enums(),
        Statement::TSModuleDeclaration(declaration) => is_executable_module(declaration),
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(Declaration::VariableDeclaration(declaration)) => {
                variable_statement_is_executable(declaration)
            }
            Some(Declaration::ClassDeclaration(_)) => true,
            Some(Declaration::TSEnumDeclaration(declaration)) => !declaration.r#const || preserves_const_enums(),
            Some(Declaration::TSModuleDeclaration(declaration)) => is_executable_module(declaration),
            _ => false,
        },
        Statement::ExportDefaultDeclaration(export) => {
            matches!(export.declaration, ExportDefaultDeclarationKind::ClassDeclaration(_))
        }
        _ => false,
    }
}

fn variable_statement_is_executable(declaration: &VariableDeclaration<'_>) -> bool {
    declaration.kind != VariableDeclarationKind::Var
        || declaration
            .declarations
            .iter()
            .any(|declarator| declarator.init.is_some())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ModuleInstanceState {
    NonInstantiated,
    ConstEnumOnly,
    Instantiated,
}

/// tsc's `isInstantiatedModule` without `preserveConstEnums`.
pub(crate) fn is_instantiated_module(declaration: &TSModuleDeclaration<'_>) -> bool {
    module_instance_state(declaration, &[]) == ModuleInstanceState::Instantiated
}

fn is_executable_module(declaration: &TSModuleDeclaration<'_>) -> bool {
    let state = module_instance_state(declaration, &[]);
    state == ModuleInstanceState::Instantiated
        || (preserves_const_enums() && state == ModuleInstanceState::ConstEnumOnly)
}

/// tsc's `getModuleInstanceState`: a namespace holding only types, non-exported
/// imports, and re-exports of such is never instantiated; `enclosing` are the
/// statement lists an `export { x }` may find `x` in.
fn module_instance_state<'a>(
    declaration: &'a TSModuleDeclaration<'a>,
    enclosing: &[&'a [Statement<'a>]],
) -> ModuleInstanceState {
    match &declaration.body {
        None => ModuleInstanceState::Instantiated,
        Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => module_instance_state(inner, enclosing),
        Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
            let mut enclosing = enclosing.to_vec();
            enclosing.push(&block.body);
            let mut state = ModuleInstanceState::NonInstantiated;
            for statement in &block.body {
                state = state.max(statement_instance_state(statement, &enclosing));
                if state == ModuleInstanceState::Instantiated {
                    break;
                }
            }
            state
        }
    }
}

fn statement_instance_state<'a>(
    statement: &'a Statement<'a>,
    enclosing: &[&'a [Statement<'a>]],
) -> ModuleInstanceState {
    match statement {
        Statement::TSInterfaceDeclaration(_)
        | Statement::TSTypeAliasDeclaration(_)
        | Statement::ImportDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => ModuleInstanceState::NonInstantiated,
        Statement::TSEnumDeclaration(declaration) if declaration.r#const => {
            ModuleInstanceState::ConstEnumOnly
        }
        Statement::TSModuleDeclaration(declaration) => module_instance_state(declaration, enclosing),
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_)) => {
                ModuleInstanceState::NonInstantiated
            }
            Some(Declaration::TSEnumDeclaration(declaration)) if declaration.r#const => {
                ModuleInstanceState::ConstEnumOnly
            }
            Some(Declaration::TSModuleDeclaration(declaration)) => {
                module_instance_state(declaration, enclosing)
            }
            Some(_) => ModuleInstanceState::Instantiated,
            None if export.source.is_none() => {
                let mut state = ModuleInstanceState::NonInstantiated;
                for specifier in &export.specifiers {
                    let ModuleExportName::IdentifierReference(local) = &specifier.local else {
                        return ModuleInstanceState::Instantiated;
                    };
                    state = state.max(alias_target_instance_state(local.name.as_str(), enclosing));
                    if state == ModuleInstanceState::Instantiated {
                        break;
                    }
                }
                state
            }
            None => ModuleInstanceState::Instantiated,
        },
        _ => ModuleInstanceState::Instantiated,
    }
}

/// tsc's `getModuleInstanceStateForAliasTarget`: the state of the nearest
/// enclosing statement list's declaration of `name`, instantiated when none
/// is found or the declaration is an `import =` alias.
fn alias_target_instance_state<'a>(name: &str, enclosing: &[&'a [Statement<'a>]]) -> ModuleInstanceState {
    for statements in enclosing.iter().rev() {
        let mut found: Option<ModuleInstanceState> = None;
        for statement in *statements {
            if !statement_has_name(statement, name) {
                continue;
            }
            let state = statement_instance_state(statement, enclosing);
            found = Some(found.map_or(state, |found| found.max(state)));
            if found == Some(ModuleInstanceState::Instantiated) {
                return ModuleInstanceState::Instantiated;
            }
            if matches!(statement, Statement::TSImportEqualsDeclaration(_)) {
                found = Some(ModuleInstanceState::Instantiated);
            }
        }
        if let Some(found) = found {
            return found;
        }
    }
    ModuleInstanceState::Instantiated
}

fn statement_has_name(statement: &Statement<'_>, name: &str) -> bool {
    let declaration = match statement {
        Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
        other => other.as_declaration(),
    };
    match declaration {
        Some(Declaration::VariableDeclaration(declaration)) => declaration.declarations.iter().any(
            |declarator| declarator.id.get_binding_identifier().is_some_and(|id| id.name == name),
        ),
        Some(Declaration::FunctionDeclaration(function)) => {
            function.id.as_ref().is_some_and(|id| id.name == name)
        }
        Some(Declaration::ClassDeclaration(class)) => class.id.as_ref().is_some_and(|id| id.name == name),
        Some(Declaration::TSTypeAliasDeclaration(alias)) => alias.id.name == name,
        Some(Declaration::TSInterfaceDeclaration(interface)) => interface.id.name == name,
        Some(Declaration::TSEnumDeclaration(declaration)) => declaration.id.name == name,
        Some(Declaration::TSModuleDeclaration(declaration)) => {
            matches!(&declaration.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(id) if id.name == name)
        }
        Some(Declaration::TSImportEqualsDeclaration(declaration)) => declaration.id.name == name,
        _ => false,
    }
}

/// tsc's `createFlowCondition` folds only the literal keywords, and
/// `bindLogicalLikeExpression` only propagates them through `&&`, `||` and
/// a `!` that sits on such a logical expression — `if (!true)` is bound as
/// an opaque condition. `??` operands are exempt by name.
fn definitely_true(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::BooleanLiteral(literal) => literal.value,
        Expression::LogicalExpression(logical) => match logical.operator {
            LogicalOperator::Or => definitely_true(&logical.left) || definitely_true(&logical.right),
            LogicalOperator::And => definitely_true(&logical.left) && definitely_true(&logical.right),
            LogicalOperator::Coalesce => false,
        },
        Expression::ParenthesizedExpression(parenthesized)
            if is_logical_expression(&parenthesized.expression) =>
        {
            definitely_true(&parenthesized.expression)
        }
        Expression::UnaryExpression(unary)
            if unary.operator == UnaryOperator::LogicalNot && is_logical_expression(&unary.argument) =>
        {
            definitely_false(&unary.argument)
        }
        _ => false,
    }
}

fn definitely_false(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::BooleanLiteral(literal) => !literal.value,
        Expression::LogicalExpression(logical) => match logical.operator {
            LogicalOperator::And => definitely_false(&logical.left) || definitely_false(&logical.right),
            LogicalOperator::Or => definitely_false(&logical.left) && definitely_false(&logical.right),
            LogicalOperator::Coalesce => false,
        },
        Expression::ParenthesizedExpression(parenthesized)
            if is_logical_expression(&parenthesized.expression) =>
        {
            definitely_false(&parenthesized.expression)
        }
        Expression::UnaryExpression(unary)
            if unary.operator == UnaryOperator::LogicalNot && is_logical_expression(&unary.argument) =>
        {
            definitely_true(&unary.argument)
        }
        _ => false,
    }
}

/// tsc's `IsLogicalExpression`: a `&&`/`||`/`??` under any parentheses and `!`.
fn is_logical_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ParenthesizedExpression(parenthesized) => is_logical_expression(&parenthesized.expression),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            is_logical_expression(&unary.argument)
        }
        Expression::LogicalExpression(_) => true,
        _ => false,
    }
}

/// A non-async, non-generator IIFE is bound as part of the enclosing flow:
/// when its body neither completes nor returns, nothing after the call is
/// reachable (`bindContainer`'s `isImmediatelyInvoked`). Only an IIFE in an
/// unconditionally evaluated position of the expression counts.
fn diverges(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ParenthesizedExpression(parenthesized) => diverges(&parenthesized.expression),
        Expression::AwaitExpression(await_expression) => diverges(&await_expression.argument),
        Expression::AssignmentExpression(assignment) => diverges(&assignment.right),
        Expression::SequenceExpression(sequence) => sequence.expressions.iter().any(diverges),
        Expression::UnaryExpression(unary) => diverges(&unary.argument),
        Expression::CallExpression(call) if !call.optional => {
            if call.arguments.iter().any(|argument| argument.as_expression().is_some_and(diverges)) {
                return true;
            }
            let body = match strip_parentheses(&call.callee) {
                Expression::FunctionExpression(function) if !function.r#async && !function.generator => {
                    function.body.as_deref()
                }
                Expression::ArrowFunctionExpression(arrow) if !arrow.r#async && !arrow.expression => {
                    Some(&*arrow.body)
                }
                _ => None,
            };
            body.is_some_and(|body| !function_body_completes(body))
        }
        _ => false,
    }
}

fn strip_parentheses<'a, 'b>(expression: &'b Expression<'a>) -> &'b Expression<'a> {
    match expression {
        Expression::ParenthesizedExpression(parenthesized) => strip_parentheses(&parenthesized.expression),
        other => other,
    }
}

/// The first run [`Flow::bind_statements`] would report if `statements`
/// started unreachable.
fn first_executable_run(statements: &[Statement<'_>]) -> Option<ReportedRun> {
    let first = statements.iter().position(is_potentially_executable)?;
    let mut last = first;
    while last + 1 < statements.len() && is_potentially_executable(&statements[last + 1]) {
        last += 1;
    }
    Some(ReportedRun {
        first: statements[first].span(),
        last_end: statements[last].span().end,
        statements: Vec::new(),
    })
}

fn function_body_completes(body: &FunctionBody<'_>) -> bool {
    let mut flow = Flow {
        frames: Vec::new(),
        returned: false,
        reported: Vec::new(),
        after_exhaustive_switch: Vec::new(),
        calls: None,
    };
    let end_reachable = flow.bind_statements(&body.statements, true);
    end_reachable || flow.returned
}
