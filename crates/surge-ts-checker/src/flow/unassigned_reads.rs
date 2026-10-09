//! tsc's `checkIdentifier` reports a read whose flow type admits `undefined`
//! that its declared type does not as used before being assigned (TS2454),
//! and types that read as the declared type "to reduce follow-on errors" —
//! whatever a guard narrowed the variable to there. Top-level statements are
//! type-checked before the module's definite-assignment pass runs, so that
//! pass is first run silently to find the reads it reports.

use std::cell::RefCell;
use std::collections::HashSet;

use surge_ts_syntax::{
    ParsedFunctionBodyStatement, ParsedStatement, ParsedVariableDeclaration, ParsedVariableKind,
    TextSpan,
};

use crate::context::CheckerContext;

#[derive(Default)]
struct UnassignedReads {
    file: String,
    recording: bool,
    spans: HashSet<(usize, usize)>,
}

thread_local! {
    static UNASSIGNED_READS: RefCell<UnassignedReads> = RefCell::new(UnassignedReads::default());
}

/// Finds the reads of `statements`' module-level flow that are used before
/// being assigned, for [`is_unassigned_read`] to answer while the file's
/// statements are type-checked.
pub(crate) fn record_module_unassigned_reads(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    UNASSIGNED_READS.with(|reads| {
        let mut reads = reads.borrow_mut();
        reads.file.clone_from(&ctx.file_name);
        reads.spans.clear();
    });
    if !surge_ts_types::strict_null_checks() || !declares_unassigned_binding(statements) {
        return;
    }
    // The module's bindings join the table as their statements are checked;
    // the pass reads their declared types from the file's full table, as it
    // does once every statement has run.
    let Some(module_symbols) = ctx.module_value_fallback.clone() else {
        return;
    };
    let statement_symbols = std::mem::replace(&mut ctx.symbols, (*module_symbols).clone());
    UNASSIGNED_READS.with(|reads| reads.borrow_mut().recording = true);
    let diagnostics_before = ctx.diagnostics().len();
    super::check_module_definite_assignment(statements, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    UNASSIGNED_READS.with(|reads| reads.borrow_mut().recording = false);
    ctx.symbols = statement_symbols;
}

/// Notes a read the definite-assignment pass reports as TS2454: in the
/// silent module pre-pass, and in a function body's own pass, which runs before
/// the statement holding the read is type-checked.
pub(crate) fn record_unassigned_read(span: Option<TextSpan>, file: &str) {
    let Some(span) = span else {
        return;
    };
    UNASSIGNED_READS.with(|reads| {
        let mut reads = reads.borrow_mut();
        if reads.recording || reads.file == file {
            reads.spans.insert((span.start, span.end));
        }
    });
}

/// Whether the identifier read at `span` in `file` is used before being
/// assigned, so tsc types it as its declared type.
pub(crate) fn is_unassigned_read(span: Option<TextSpan>, file: &str) -> bool {
    let Some(span) = span else {
        return false;
    };
    UNASSIGNED_READS.with(|reads| {
        let reads = reads.borrow();
        !reads.spans.is_empty()
            && reads.file == file
            && reads.spans.contains(&(span.start, span.end))
    })
}

/// Whether the module's own flow declares a binding a read can find
/// unassigned: a `var` or `let` written with a type and no initializer, at the
/// top level or hoisted out of a top-level block. (A `let` or `const` read
/// before its declaration is unassigned too, but that read is already TS2448.)
fn declares_unassigned_binding(statements: &[ParsedStatement]) -> bool {
    statements.iter().any(|statement| {
        let statement = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => {
                    declaration.as_ref()
                }
                _ => statement,
            },
            other => other,
        };
        match statement {
            ParsedStatement::VariableDeclaration(variable) => is_unassigned_declaration(variable),
            ParsedStatement::Block(block) => body_declares_unassigned_var(block),
            ParsedStatement::If(if_statement) => {
                body_declares_unassigned_var(&if_statement.then_body)
                    || body_declares_unassigned_var(&if_statement.else_body)
            }
            ParsedStatement::NamespaceDeclaration(namespace) => {
                declares_unassigned_binding(&namespace.statements)
            }
            _ => false,
        }
    })
}

fn is_unassigned_declaration(variable: &ParsedVariableDeclaration) -> bool {
    variable.initializer.is_none()
        && variable.declared_type.is_some()
        && !variable.is_declare
        && !variable.has_definite_assertion
}

fn body_declares_unassigned_var(body: &[ParsedFunctionBodyStatement]) -> bool {
    body.iter().any(|statement| match statement {
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            variable.kind == ParsedVariableKind::Var && is_unassigned_declaration(variable)
        }
        ParsedFunctionBodyStatement::Block(block) => body_declares_unassigned_var(block),
        ParsedFunctionBodyStatement::If(if_statement) => {
            body_declares_unassigned_var(&if_statement.then_body)
                || body_declares_unassigned_var(&if_statement.else_body)
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            body_declares_unassigned_var(&while_statement.body)
        }
        ParsedFunctionBodyStatement::ForOf(for_of) => body_declares_unassigned_var(&for_of.body),
        ParsedFunctionBodyStatement::Switch(switch) => switch
            .cases
            .iter()
            .any(|case| body_declares_unassigned_var(&case.consequent)),
        ParsedFunctionBodyStatement::Try(try_statement) => {
            body_declares_unassigned_var(&try_statement.block)
                || try_statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| body_declares_unassigned_var(&handler.body))
                || body_declares_unassigned_var(&try_statement.finalizer)
        }
        _ => false,
    })
}
