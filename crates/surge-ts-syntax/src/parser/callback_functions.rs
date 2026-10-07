//! The `function` expressions written directly as a call or `new` argument.
//! tsc types such a function's `this` by the contextual signature alone; one
//! assigned to `obj.m` takes `obj`, and one with a JSDoc `@this` takes that, so
//! only these are the ones whose missing contextual `this` is TS2683.

use std::collections::HashSet;
use std::rc::Rc;

use oxc_ast::ast::{Argument, CallExpression, Expression, NewExpression, Program};
use oxc_ast_visit::{Visit, walk};

thread_local! {
    static CALLBACKS: std::cell::RefCell<Option<Rc<HashSet<u32>>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_callback_functions<R>(callbacks: Rc<HashSet<u32>>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<HashSet<u32>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CALLBACKS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = CALLBACKS.with(|slot| slot.borrow_mut().replace(callbacks));
    let _restore = Restore(previous);
    f()
}

pub(crate) fn is_callback_function(start: u32) -> bool {
    CALLBACKS.with(|slot| slot.borrow().as_ref().is_some_and(|callbacks| callbacks.contains(&start)))
}

pub(crate) fn collect_callback_functions(program: &Program<'_>) -> HashSet<u32> {
    let this_tags: Vec<(u32, u32)> = program
        .comments
        .iter()
        .filter(|comment| {
            program
                .source_text
                .get(comment.span.start as usize..comment.span.end as usize)
                .is_some_and(|text| text.contains("@this"))
        })
        .map(|comment| (comment.span.start, comment.span.end))
        .collect();
    let mut collector = Collector { source_text: program.source_text, this_tags, starts: HashSet::new() };
    collector.visit_program(program);
    collector.starts
}

struct Collector<'s> {
    source_text: &'s str,
    this_tags: Vec<(u32, u32)>,
    starts: HashSet<u32>,
}

impl Collector<'_> {
    fn record(&mut self, arguments: &[Argument<'_>]) {
        for argument in arguments {
            let Some(expression) = argument.as_expression() else {
                continue;
            };
            let Expression::FunctionExpression(function) = expression.without_parentheses() else {
                continue;
            };
            let start = function.span.start;
            // A `@this` tag in a comment directly before the function types it.
            let tagged = self.this_tags.iter().any(|&(tag_start, tag_end)| {
                tag_end <= start
                    && self
                        .source_text
                        .get(tag_end as usize..start as usize)
                        .is_some_and(|between| between.trim().is_empty() || between.trim() == "(")
                    && tag_start < start
            });
            if !tagged {
                self.starts.insert(start);
            }
        }
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        self.record(&call.arguments);
        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
        self.record(&new.arguments);
        walk::walk_new_expression(self, new);
    }
}
