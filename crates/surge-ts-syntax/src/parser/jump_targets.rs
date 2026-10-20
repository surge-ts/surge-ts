//! tsc's `bindBreakOrContinueFlow` ends the flow at a `break` or `continue`
//! only when it has a target: an enclosing loop or `switch` (a loop for
//! `continue`), or an enclosing label (a loop's for `continue`). Functions,
//! class static blocks and namespace bodies start without one. A jump with
//! no target is a grammar error (TS1104/TS1105/TS1107/TS1115/TS1116) and
//! leaves the flow running, so `() => { break; }` still returns `void`.

use std::rc::Rc;

use oxc_ast::ast::{
    ArrowFunctionExpression, BreakStatement, ContinueStatement, DoWhileStatement, ForInStatement,
    ForOfStatement, ForStatement, Function, LabeledStatement, Program, StaticBlock, Statement,
    SwitchStatement, TSModuleDeclaration, WhileStatement,
};
use oxc_ast_visit::Visit;
use oxc_syntax::scope::ScopeFlags;

thread_local! {
    static ORPHANS: std::cell::RefCell<Option<Rc<Vec<u32>>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_orphan_jumps<R>(orphans: Rc<Vec<u32>>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<Vec<u32>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ORPHANS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = ORPHANS.with(|slot| slot.borrow_mut().replace(orphans));
    let _restore = Restore(previous);
    f()
}

/// Whether the `break` or `continue` starting at `start` has no target.
pub(crate) fn is_orphan_jump(start: u32) -> bool {
    ORPHANS.with(|slot| slot.borrow().as_ref().is_some_and(|orphans| orphans.binary_search(&start).is_ok()))
}

/// The starts of the program's `break` and `continue` statements that have no
/// target, sorted.
pub(crate) fn collect_orphan_jumps(program: &Program<'_>) -> Vec<u32> {
    let mut collector = Collector { frames: vec![Frame::default()], orphans: Vec::new() };
    collector.visit_program(program);
    collector.orphans.sort_unstable();
    collector.orphans
}

#[derive(Default)]
struct Frame {
    loops: usize,
    switches: usize,
    /// Enclosing labels, and whether each labels a loop.
    labels: Vec<(String, bool)>,
}

struct Collector {
    frames: Vec<Frame>,
    orphans: Vec<u32>,
}

impl Collector {
    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the program frame")
    }

    fn in_frame(&mut self, walk: impl FnOnce(&mut Self)) {
        self.frames.push(Frame::default());
        walk(self);
        self.frames.pop();
    }

    fn in_loop(&mut self, walk: impl FnOnce(&mut Self)) {
        self.frame().loops += 1;
        walk(self);
        self.frame().loops -= 1;
    }
}

fn labels_loop(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::LabeledStatement(labeled) => labels_loop(&labeled.body),
        Statement::WhileStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_) => true,
        _ => false,
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        self.in_frame(|collector| oxc_ast_visit::walk::walk_function(collector, function, flags));
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.in_frame(|collector| oxc_ast_visit::walk::walk_arrow_function_expression(collector, arrow));
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.in_frame(|collector| oxc_ast_visit::walk::walk_static_block(collector, block));
    }

    fn visit_ts_module_declaration(&mut self, declaration: &TSModuleDeclaration<'a>) {
        self.in_frame(|collector| oxc_ast_visit::walk::walk_ts_module_declaration(collector, declaration));
    }

    fn visit_while_statement(&mut self, statement: &WhileStatement<'a>) {
        self.in_loop(|collector| oxc_ast_visit::walk::walk_while_statement(collector, statement));
    }

    fn visit_do_while_statement(&mut self, statement: &DoWhileStatement<'a>) {
        self.in_loop(|collector| oxc_ast_visit::walk::walk_do_while_statement(collector, statement));
    }

    fn visit_for_statement(&mut self, statement: &ForStatement<'a>) {
        self.in_loop(|collector| oxc_ast_visit::walk::walk_for_statement(collector, statement));
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        self.in_loop(|collector| oxc_ast_visit::walk::walk_for_in_statement(collector, statement));
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        self.in_loop(|collector| oxc_ast_visit::walk::walk_for_of_statement(collector, statement));
    }

    fn visit_switch_statement(&mut self, statement: &SwitchStatement<'a>) {
        self.frame().switches += 1;
        oxc_ast_visit::walk::walk_switch_statement(self, statement);
        self.frame().switches -= 1;
    }

    fn visit_labeled_statement(&mut self, statement: &LabeledStatement<'a>) {
        let entry = (statement.label.name.to_string(), labels_loop(&statement.body));
        self.frame().labels.push(entry);
        oxc_ast_visit::walk::walk_labeled_statement(self, statement);
        self.frame().labels.pop();
    }

    fn visit_break_statement(&mut self, statement: &BreakStatement<'a>) {
        let frame = self.frame();
        let targeted = match &statement.label {
            Some(label) => frame.labels.iter().any(|(name, _)| name.as_str() == label.name.as_str()),
            None => frame.loops + frame.switches > 0,
        };
        if !targeted {
            self.orphans.push(statement.span.start);
        }
    }

    fn visit_continue_statement(&mut self, statement: &ContinueStatement<'a>) {
        let frame = self.frame();
        let targeted = match &statement.label {
            // A label reached last-declared first, as `findActiveLabel` does.
            Some(label) => frame
                .labels
                .iter()
                .rev()
                .find(|(name, _)| name.as_str() == label.name.as_str())
                .is_some_and(|(_, is_loop)| *is_loop),
            None => frame.loops > 0,
        };
        if !targeted {
            self.orphans.push(statement.span.start);
        }
    }
}
