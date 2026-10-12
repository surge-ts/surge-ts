use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use surge_ts_syntax::{ParsedSource, ParserWorker};

// Instrumentation lives in `metrics`; re-export it so existing callers that
// reference `crate::program::record_*` / `ProgramTimings` keep resolving and so
// the bare `record_*` calls throughout this file stay in scope.
use crate::metrics::*;

use crate::context::{CheckerOptions, FileKind};
use super::{AUTO_JOBS, ParsedProgramFile, SourceFileInput, classify_file_kind};

/// Minimum source bytes assigned to a worker before `AUTO_JOBS` parallelizes the
/// parse phase. Parse cost tracks byte length, and files are pulled through a
/// shared cursor so large declaration/default-lib files don't stall one worker.
/// This is a calibration threshold — tune with `bench:test`.
pub(super) const MIN_BYTES_PER_PARSE_WORKER: usize = 256 * 1024;

pub(super) fn resolve_parse_worker_count(jobs: usize, files: &[SourceFileInput]) -> usize {
    let file_count = files.len();
    if file_count <= 1 {
        return 1;
    }

    let requested = if jobs == AUTO_JOBS {
        let cores = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let bytes: usize = files.iter().map(|file| file.source_text.len()).sum();
        let by_work = (bytes / MIN_BYTES_PER_PARSE_WORKER).max(1);
        cores.min(by_work)
    } else {
        jobs
    };

    requested.max(1).min(file_count)
}

pub(super) fn parse_program_files(
    files: Vec<SourceFileInput>,
    prescanned: Vec<ParsedSource>,
    jobs: usize,
    options: &CheckerOptions,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> Vec<ParsedProgramFile> {
    let prescanned = prescanned_by_file(prescanned, &files);
    let worker_count = resolve_parse_worker_count(jobs, &files);
    if worker_count <= 1 {
        let mut parser = ParserWorker::new();
        return files
            .iter()
            .zip(prescanned)
            .map(|(input, reused)| {
                parse_program_file(
                    &mut parser,
                    input,
                    reused.into_inner().ok().flatten(),
                    options,
                    timings,
                )
            })
            .collect();
    }

    let next_index = AtomicUsize::new(0);
    let timings_owned = timings.cloned();

    let mut indexed = thread::scope(|scope| {
        let next_index = &next_index;
        let files = &files;
        let prescanned = &prescanned;
        let mut handles = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let timings = timings_owned.clone();
            handles.push(scope.spawn(move || {
                // One arena per parse thread; never shared across threads.
                let mut parser = ParserWorker::new();
                let mut worker_results = Vec::new();
                loop {
                    let file_index = next_index.fetch_add(1, Ordering::Relaxed);
                    if file_index >= files.len() {
                        break;
                    }
                    // Each index is claimed by exactly one worker, so the lock
                    // never contends; it is what lets a worker take ownership
                    // of its own slot out of the shared slice.
                    let reused = prescanned[file_index]
                        .lock()
                        .ok()
                        .and_then(|mut slot| slot.take());
                    worker_results.push((
                        file_index,
                        parse_program_file(
                            &mut parser,
                            &files[file_index],
                            reused,
                            options,
                            timings.as_ref(),
                        ),
                    ));
                }
                worker_results
            }));
        }

        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("parse worker panicked"))
            .collect::<Vec<_>>()
    });

    indexed.sort_by_key(|(file_index, _)| *file_index);
    indexed.into_iter().map(|(_, parsed)| parsed).collect()
}

/// Both flags are deliberately *textual*, not AST facts: `contains_typeof`
/// gates releasing a declaration module's local symbols, so a `typeof` in a
/// comment or a string must keep the symbols alive. Every file's full text is
/// swept for them, which on a project whose dependency declarations dwarf its
/// sources is megabytes per run, and `str::contains` builds a fresh two-way
/// searcher on each call. The needle-only finders are built once instead.
pub(super) fn source_text_has_export_default(source_text: &str) -> bool {
    static FINDER: std::sync::OnceLock<memchr::memmem::Finder<'static>> =
        std::sync::OnceLock::new();
    FINDER
        .get_or_init(|| memchr::memmem::Finder::new("export default"))
        .find(source_text.as_bytes())
        .is_some()
}

pub(super) fn source_text_has_typeof(source_text: &str) -> bool {
    static FINDER: std::sync::OnceLock<memchr::memmem::Finder<'static>> =
        std::sync::OnceLock::new();
    FINDER
        .get_or_init(|| memchr::memmem::Finder::new("typeof"))
        .find(source_text.as_bytes())
        .is_some()
}

/// Line up the loader's parses with the program's file order. Matching is by
/// file name because the program splices in generated default libs of its own,
/// and a loader is free to hand over fewer files than the program checks.
pub(super) fn prescanned_by_file(
    prescanned: Vec<ParsedSource>,
    files: &[SourceFileInput],
) -> Vec<Mutex<Option<ParsedSource>>> {
    if prescanned.is_empty() {
        return files.iter().map(|_| Mutex::new(None)).collect();
    }

    let mut by_name = prescanned
        .into_iter()
        .map(|parsed| (parsed.file_name.clone(), parsed))
        .collect::<HashMap<_, _>>();
    files
        .iter()
        .map(|file| Mutex::new(by_name.remove(&file.file_name)))
        .collect()
}

pub(super) fn parse_program_file(
    parser: &mut ParserWorker,
    input: &SourceFileInput,
    prescanned: Option<ParsedSource>,
    options: &CheckerOptions,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> ParsedProgramFile {
    record_program_counter(|c| c.files_total += 1);
    let file_kind = classify_file_kind(&input.file_name);
    if file_kind == FileKind::GeneratedDeclaration {
        record_program_counter(|c| c.generated_default_lib_files += 1);
    }

    let parse_start = Instant::now();
    let mut parsed = match prescanned {
        Some(parsed) => parsed,
        None => parser.parse(&input.source_text, &input.file_name),
    };
    // tsc parses a JSON file as one value (`parseJSONText`) and reports where
    // the text is no JSON as syntax errors.
    if parsed.json_module_type.is_some() && surge_ts_syntax::is_json_file_name(&input.file_name) {
        parsed.parser_errors = surge_ts_tsc_syntax::json_file_diagnostics(&input.source_text)
            .into_iter()
            .map(|diagnostic| surge_ts_syntax::ParserError {
                code: Some(diagnostic.code),
                span_text: input.source_text.get(diagnostic.start..diagnostic.end).map(str::to_string),
                span: Some(surge_ts_syntax::TextSpan { start: diagnostic.start, end: diagnostic.end }),
                message: diagnostic.message,
            })
            .collect();
    }
    let tsc_errors = match file_kind {
        FileKind::GeneratedDeclaration | FileKind::PhysicalDefaultLib => None,
        _ => super::diagnostics::tsc_file_errors(&input.source_text, &input.file_name, options, &parsed),
    };
    let mut bind_errors = Vec::new();
    let mut tsc_bound = false;
    let mut oxc_aborted = false;
    let mut tsc_globals = None;
    if let Some(tsc_errors) = tsc_errors {
        if !tsc_errors.syntactic.is_empty() {
            // The grammar walk reports tsc's checker grammar errors, which a
            // program with syntax errors never reaches.
            parsed.parser_errors = tsc_errors.syntactic;
            parsed.grammar_diagnostics.clear();
        } else {
            // tsc's parser accepts the file, so the syntax errors oxc reports
            // are its own, and so is its verdict on a regular expression,
            // which tsc's checker gives (see `tsc_file_errors`). The rest are
            // grammar errors tsc's checker reports. A file oxc gave up on
            // keeps them: its statements are gone, and checking it as empty
            // would misreport every use of what it declares.
            if !parsed.parse_aborted {
                parsed.parser_errors.retain(|error| {
                    !super::diagnostics::is_syntactic_parser_error(error)
                        && !error.code.is_some_and(|code| (1499..=1538).contains(&code))
                });
            }
            oxc_aborted = parsed.parse_aborted;
            // Opt-in (`SURGE_TRACE_PARSE_ABORT=1`): a file oxc gave up on while
            // tsc's parser accepted it, with oxc's first complaint.
            if oxc_aborted && std::env::var_os("SURGE_TRACE_PARSE_ABORT").is_some() {
                eprintln!(
                    "[parse-abort] {} {}",
                    input.file_name,
                    parsed.parser_errors.first().map_or("", |error| error.message.as_str())
                );
            }
            bind_errors = tsc_errors.bind;
            // A grammar error the port's checker pass reports where oxc does
            // too is the pass's alone.
            parsed.parser_errors.retain(|error| {
                !bind_errors.iter().any(|bound| {
                    bound.code.is_some()
                        && bound.code == error.code
                        && bound.span.map(|span| span.start) == error.span.map(|span| span.start)
                })
            });
            tsc_bound = true;
            tsc_globals = tsc_errors.globals.map(std::sync::Arc::new);
        }
    }
    // Checked like any parse error: they stop the program's semantic checks.
    parsed.parser_errors.append(&mut parsed.jsdoc_parse_errors);
    // tsc's parser reports a bad `resolution-mode` while reading the file's
    // pragmas.
    if matches!(file_kind, FileKind::RootSource | FileKind::RootDeclaration) {
        parsed.parser_errors.extend(
            surge_ts_syntax::invalid_reference_resolution_modes(&input.source_text)
                .into_iter()
                .map(|span| surge_ts_syntax::ParserError {
                    code: Some(1453),
                    span_text: input.source_text.get(span.start..span.end).map(str::to_string),
                    span: Some(span),
                    message: "`resolution-mode` should be either `require` or `import`.".to_string(),
                }),
        );
    }
    let parse_duration = parse_start.elapsed();
    let file_name = parsed.file_name;
    record_program_timing(timings, |timings| match file_kind {
        FileKind::DependencyDeclaration => {
            timings.dependency_declaration_parse_time += parse_duration
        }
        FileKind::GeneratedDeclaration | FileKind::PhysicalDefaultLib => {
            timings.generated_default_lib_parse_time += parse_duration
        }
        FileKind::RootSource | FileKind::RootDeclaration => {}
    });
    record_program_counter(|c| match file_kind {
        FileKind::RootSource => c.root_source_files += 1,
        FileKind::RootDeclaration | FileKind::DependencyDeclaration => {
            if matches!(file_kind, FileKind::DependencyDeclaration) {
                c.dependency_declaration_files += 1;
            }
            if matches!(file_kind, FileKind::RootDeclaration) {
                c.root_source_files += 1;
            }
            if matches!(file_kind, FileKind::DependencyDeclaration) {
                c.parsed_dependency_declaration_files += 1;
            } else {
                c.parsed_root_source_files += 1;
            }
        }
        FileKind::GeneratedDeclaration => {}
        FileKind::PhysicalDefaultLib => {
            c.generated_default_lib_files += 1;
        }
    });
    let emit_helper_requests =
        if file_kind == FileKind::RootSource && parsed.json_module_type.is_none() {
            // tsgo's `IsEffectiveExternalModule`.
            let external_module = parsed.is_module
                || super::file_is_forced_module(
                    &file_name,
                    parsed.jsx_factory_uses.first_tag.is_some(),
                    options,
                )
                || (parsed.commonjs_module
                    && (options.module_emit == crate::context::ModuleEmitKind::CommonJS
                        || options.module_emit.is_node()));
            super::emit_helpers::file_emit_helper_requests(
                &file_name,
                &input.source_text,
                external_module,
                options,
            )
        } else {
            Vec::new()
        };
    ParsedProgramFile {
        file_name: file_name.clone(),
        has_export_default: source_text_has_export_default(&input.source_text),
        contains_typeof: source_text_has_typeof(&input.source_text),
        statements: parsed.statements,
        parser_errors: parsed.parser_errors,
        bind_errors,
        tsc_bound,
        oxc_aborted,
        tsc_globals,
        shadowed_globals: Vec::new(),
        no_check: surge_ts_syntax::extract_check_directive(&input.source_text) == Some(false),
        plain_js: surge_ts_syntax::is_javascript_file_name(&input.file_name)
            && options.check_js.is_none()
            && surge_ts_syntax::extract_check_directive(&input.source_text).is_none(),
        is_module: parsed.is_module,
        commonjs_module: parsed.commonjs_module,
        import_call_specifiers: parsed.import_call_specifiers,
        import_calls: parsed.import_calls,
        import_attributes: parsed.import_attributes,
        file_kind,
        module_reads: parsed.module_reads,
                definite_writes: parsed.definite_writes,
        comment_directives: parsed.comment_directives,
        grammar_diagnostics: parsed.grammar_diagnostics,
        parenthesized_expressions: parsed.parenthesized_expressions.into(),
        global_this_starts: parsed.global_this_starts.into(),
        literal_this_members: parsed.literal_this_members.into(),
        let_assignments: parsed.let_assignments.into(),
        json_module_type: parsed.json_module_type,
        jsx_factory_uses: parsed.jsx_factory_uses,
        emit_helper_requests,
    }
}
