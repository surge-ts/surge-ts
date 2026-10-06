use std::collections::{HashMap, HashSet};

use surge_ts_diagnostics::Diagnostic;

// Instrumentation lives in `metrics`; re-export it so existing callers that
// reference `crate::program::record_*` / `ProgramTimings` keep resolving and so
// the bare `record_*` calls throughout this file stay in scope.

use crate::context::CheckerContext;
use super::ParsedProgramFile;

/// oxc's own wording for the token-level failures tsc's parser reports too.
/// oxc also rejects syntax tsc parses and then reports from its checker (a
/// `const` without an initializer, modifiers out of order); those carry other
/// messages, or codes outside tsc's parser set.
const GENERIC_SYNTAX_ERROR_PREFIXES: &[&str] = &[
    "Unexpected token",
    "Unexpected end of file",
    "Unexpected exponentiation expression",
    "Unexpected private identifier",
    "Expected `",
    "Expected a semicolon or an implicit semicolon after a statement",
    "Expected corresponding JSX closing tag",
    "Expected corresponding closing tag for JSX fragment",
    "Expected function name",
    "Expected function body",
    "Expected switch clause",
    "Invalid Unicode escape sequence",
    "Invalid Character",
    "Invalid characters after number",
    "Invalid escape sequence",
    "Bad escape sequence in untagged template literal",
    "Keywords cannot contain escape characters",
    "Unterminated string",
    "Unterminated template",
    "Unterminated regular expression",
    "Unterminated multiline comment",
    "Empty parenthesized expression",
    "Parenthesized expressions may not have a trailing comma",
    "Encountered diff marker",
    "File appears to be binary",
];

/// Whether tsc's parser reports this failure as well, which makes tsc report
/// the program's syntactic diagnostics alone.
pub(super) fn is_syntactic_parser_error(error: &surge_ts_syntax::ParserError) -> bool {
    match error.code {
        Some(code) => surge_ts_diagnostics::is_tsc_parser_code(code),
        None => {
            error.message == "Identifier expected."
                || GENERIC_SYNTAX_ERROR_PREFIXES
                    .iter()
                    .any(|prefix| error.message.starts_with(prefix))
        }
    }
}

/// What tsc reports for a file before type checking, from the port of its
/// parser and binder; `None` for a file tsc does not parse as script.
pub(super) struct TscFileErrors {
    /// The parser's diagnostics. oxc stops at the first failure, words most
    /// failures its own way, and rejects some syntax tsc accepts and accepts
    /// some it rejects, while tsc recovers and reports every error — and a
    /// program with any syntax error reports those alone.
    pub(super) syntactic: Vec<surge_ts_syntax::ParserError>,
    pub(super) bind: Vec<surge_ts_syntax::ParserError>,
    /// What a TypeScript file adds to the global scope. A JavaScript file's
    /// CommonJS use can make it a module, which the port does not see, so
    /// its declarations are left out of the merge.
    pub(super) globals: Option<surge_ts_tsc_syntax::FileGlobals>,
}

/// The binder's diagnostics are what tsc reports for a file without syntax
/// errors (conflicting declarations, strict-mode restrictions), as tsc's
/// `getBindAndCheckDiagnostics` includes them: none for a file tsc skips
/// (`// @ts-nocheck`, or JavaScript with `checkJs: false`), and for plain
/// JavaScript (neither `checkJs` nor `// @ts-check` written) only the ones
/// in its `plainJSErrors`.
pub(super) fn tsc_file_errors(
    source_text: &str,
    file_name: &str,
    checker_options: &crate::CheckerOptions,
    parsed: &surge_ts_syntax::ParsedSource,
) -> Option<TscFileErrors> {
    let mut options = surge_ts_tsc_syntax::ParseOptions::for_file_name(file_name)?;
    options.language_version = checker_options.language_version;
    if checker_options.no_unused_locals || checker_options.no_unused_parameters {
        let (jsx_element_reads, jsx_fragment_reads) =
            super::unused_locals::jsx_factory_reads(&parsed.jsx_factory_uses, file_name, checker_options);
        let javascript = surge_ts_syntax::is_javascript_file_name(file_name);
        let mut jsdoc_link_names = parsed.jsdoc_link_names.clone();
        if javascript {
            jsdoc_link_names.extend(jsdoc_type_names(source_text));
        }
        options.unused = Some(surge_ts_tsc_syntax::UnusedCheck {
            locals: checker_options.no_unused_locals,
            parameters: checker_options.no_unused_parameters,
            jsx_element_reads,
            jsx_fragment_reads,
            jsdoc_link_names,
            emit_standard_class_fields: checker_options.emit_standard_class_fields(),
        });
    }
    let detection = &checker_options.module_detection;
    if !options.is_declaration_file {
        if detection.legacy {
            options.force_module = false;
        } else if detection.force {
            options.force_module = true;
        } else {
            options.force_module |= checker_options.esm_module_files.contains(file_name);
            options.jsx_forces_module = checker_options.jsx_automatic_runtime;
        }
    }
    let check_js = checker_options.check_js;
    let diagnostics = surge_ts_tsc_syntax::file_diagnostics(source_text, &options);
    // The regular expression check reads a pattern without `u` or `v` byte by
    // byte, so an error can start inside a character; it is anchored at the
    // character's start.
    let boundary = |mut offset: usize| {
        while !source_text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let to_parser_error = |diagnostic: surge_ts_tsc_syntax::SyntaxDiagnostic| {
        let (start, end) = (boundary(diagnostic.start), boundary(diagnostic.end.max(diagnostic.start)));
        surge_ts_syntax::ParserError {
            code: Some(diagnostic.code),
            span_text: source_text.get(start..end).map(str::to_string),
            span: Some(surge_ts_syntax::TextSpan { start, end }),
            message: diagnostic.message,
        }
    };
    if !diagnostics.syntactic.is_empty() {
        return Some(TscFileErrors {
            syntactic: diagnostics.syntactic.into_iter().map(to_parser_error).collect(),
            bind: Vec::new(),
            globals: None,
        });
    }
    let is_javascript = surge_ts_syntax::is_javascript_file_name(file_name);
    let reported = diagnostics
        .bind
        .into_iter()
        .chain(diagnostics.regular_expressions)
        .chain(diagnostics.unused);
    let bind = match (surge_ts_syntax::extract_check_directive(source_text), check_js) {
        (Some(false), _) => Vec::new(),
        (Some(true), _) => reported.collect(),
        (None, _) if !is_javascript => reported.collect(),
        (None, Some(true)) => reported.collect(),
        (None, Some(false)) => Vec::new(),
        (None, None) => reported
            .filter(|diagnostic| PLAIN_JS_BIND_CODES.contains(&diagnostic.code))
            .collect(),
    };
    Some(TscFileErrors {
        syntactic: Vec::new(),
        bind: bind.into_iter().map(to_parser_error).collect(),
        globals: Some(diagnostics.globals),
    })
}

/// Every name a JavaScript file's JSDoc types may read: the identifiers of
/// each `{…}` type expression, and the type a tag writes without braces. The
/// ported binder parses no JSDoc, so each such name counts as used wherever it
/// is declared.
fn jsdoc_type_names(source_text: &str) -> Vec<String> {
    const BRACELESS_TYPE_TAGS: &[&str] =
        &["augments", "extends", "implements", "type", "return", "returns", "this", "satisfies", "throws", "exception"];
    let is_part = |ch: char| ch == '_' || ch == '$' || ch.is_alphanumeric();
    let mut names: Vec<String> = Vec::new();
    let mut rest = source_text;
    while let Some(start) = rest.find("/**") {
        let body = &rest[start + 3..];
        let end = body.find("*/").unwrap_or(body.len());
        let comment = &body[..end];
        rest = &body[end..];
        let mut depth = 0usize;
        let mut after_type_tag = false;
        let mut chars = comment.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            match ch {
                '{' => {
                    depth += 1;
                    after_type_tag = false;
                }
                '}' => depth = depth.saturating_sub(1),
                '@' if depth == 0 => {
                    let tag: String = comment[index + 1..].chars().take_while(|&ch| is_part(ch)).collect();
                    after_type_tag = BRACELESS_TYPE_TAGS.contains(&tag.as_str());
                    for _ in 0..tag.chars().count() {
                        chars.next();
                    }
                }
                ch if is_part(ch) => {
                    let mut word = String::from(ch);
                    while let Some(&(_, next)) = chars.peek() {
                        if !is_part(next) {
                            break;
                        }
                        word.push(next);
                        chars.next();
                    }
                    if depth > 0 || after_type_tag {
                        names.push(word);
                    }
                    if depth == 0 {
                        after_type_tag = false;
                    }
                }
                _ => {}
            }
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// tsc's `initializeChecker` merge of every file's global declarations: two
/// files' declarations that cannot share a name are reported where each is
/// declared, as the binder reports them within one file. The merge resolves
/// aliases as tsc's does (`resolveAlias`), and so does checking a file's
/// aliases: one whose resolution comes back to itself is TS2303.
pub(super) fn report_global_merge_conflicts(parsed_files: &mut [ParsedProgramFile], ctx: &CheckerContext) {
    // A CommonJS file is a module (tsc's `commonJsModuleIndicator`), which
    // the ported binder does not detect, so its declarations are not globals.
    let contributing: Vec<usize> = (0..parsed_files.len())
        .filter(|&index| {
            let file = &parsed_files[index];
            file.tsc_globals.is_some()
                && !(surge_ts_syntax::is_javascript_file_name(&file.file_name)
                    && (file.is_module || file.commonjs_module))
        })
        .collect();
    if contributing.is_empty() {
        return;
    }
    let globals: Vec<std::sync::Arc<surge_ts_tsc_syntax::FileGlobals>> = contributing
        .iter()
        .filter_map(|&index| parsed_files[index].tsc_globals.clone())
        .collect();
    // A JavaScript file's globals merge too; without `checkJs` nothing is
    // reported on its side (`mergeSymbol` skips a plain JavaScript file).
    let inputs: Vec<surge_ts_tsc_syntax::GlobalsInput<'_>> = globals
        .iter()
        .zip(&contributing)
        .map(|(globals, &index)| surge_ts_tsc_syntax::GlobalsInput {
            globals,
            plain_js: surge_ts_syntax::is_javascript_file_name(&parsed_files[index].file_name)
                && ctx.options.check_js != Some(true),
        })
        .collect();
    let report = {
        let files: &[ParsedProgramFile] = parsed_files;
        let input_of: HashMap<usize, u32> =
            contributing.iter().enumerate().map(|(input, &index)| (index, input as u32)).collect();
        let resolve_module = |input: u32, specifier: &str| -> Option<u32> {
            let importer = files[contributing[input as usize]].file_name.as_str();
            let resolved = ctx
                .options
                .resolved_module_for(importer, specifier)
                .and_then(|resolved| {
                    ctx.module_file_index_by_identity
                        .get(crate::modules::canonical_file_identity(resolved).as_str())
                        .copied()
                })
                .or_else(|| {
                    crate::modules::resolve_relative_module(
                        importer,
                        specifier,
                        files,
                        &ctx.module_file_index_by_identity,
                    )
                    .map(|resolution| resolution.resolved_file_index)
                })?;
            input_of.get(&resolved).copied()
        };
        // tsc checks neither a `// @ts-nocheck` file nor, under
        // `skipLibCheck`, a declaration file, so it resolves their aliases
        // only as others reach them.
        let is_checked = |input: u32| {
            let file = &files[contributing[input as usize]];
            !file.no_check && !(ctx.options.skip_lib_check && file.file_kind.is_declaration())
        };
        surge_ts_tsc_syntax::merge_globals_report_with(&inputs, &resolve_module, &is_checked)
    };
    for ((&index, reports), shared) in contributing
        .iter()
        .zip(report.diagnostics)
        .zip(report.shared_type_parameters)
    {
        let file = &mut parsed_files[index];
        if file.no_check {
            continue;
        }
        // The file checked a type parameter list alone for unused
        // parameters; tsc does not check one whose symbol other files
        // declare too.
        file.bind_errors.retain(|error| {
            !(matches!(error.code, Some(6196 | 6205))
                && error
                    .span
                    .is_some_and(|span| shared.iter().any(|&(start, end)| start <= span.start && span.start < end)))
        });
        file.bind_errors.extend(reports.into_iter().map(|diagnostic| surge_ts_syntax::ParserError {
            code: Some(diagnostic.code),
            span_text: None,
            span: Some(surge_ts_syntax::TextSpan {
                start: diagnostic.start,
                end: diagnostic.end,
            }),
            message: diagnostic.message,
        }));
    }
}

/// The binder codes in tsc's `plainJSErrors`: what it reports for a
/// JavaScript file that neither `checkJs` nor `// @ts-check` opts in.
const PLAIN_JS_BIND_CODES: &[u32] = &[
    1100, 1101, 1102, 1184, 1210, 1214, 1215, 1262, 1344, 1359, 2451, 2528, 18012,
];

/// tsc's `GetDiagnosticsOfAnyProgram`: a program with a syntax error anywhere
/// reports its syntactic diagnostics and nothing else.
pub(super) fn program_has_syntax_errors(parsed_files: &[ParsedProgramFile]) -> bool {
    parsed_files.iter().any(|file| {
        !matches!(
            file.file_kind,
            crate::FileKind::GeneratedDeclaration | crate::FileKind::PhysicalDefaultLib
        ) && file.parser_errors.iter().any(is_syntactic_parser_error)
    })
}

/// Every file with a syntax error is one oxc gave up on while tsc's parser
/// accepted it.
pub(super) fn syntax_errors_are_oxc_aborts(parsed_files: &[ParsedProgramFile]) -> bool {
    parsed_files.iter().all(|file| {
        file.oxc_aborted
            || matches!(
                file.file_kind,
                crate::FileKind::GeneratedDeclaration | crate::FileKind::PhysicalDefaultLib
            )
            || !file.parser_errors.iter().any(is_syntactic_parser_error)
    })
}

pub(super) fn is_syntactic_diagnostic(diagnostic: &Diagnostic) -> bool {
    match diagnostic.code {
        surge_ts_diagnostics::DiagnosticCode::TypeScript(code) => {
            surge_ts_diagnostics::is_tsc_parser_code(code)
        }
        surge_ts_diagnostics::DiagnosticCode::Custom(_) => false,
    }
}

/// A parse failure reported the way tsc reports it when oxc classified the
/// failure: the catalogued message for its code, anchored at oxc's own span.
/// oxc's rendering is used only for a failure it left unnumbered, and for a
/// number surge does not catalog, so neither can invent a TypeScript code.
pub(crate) fn parser_error_diagnostic(
    error: &surge_ts_syntax::ParserError,
    file_name: &str,
) -> Diagnostic {
    let Some((descriptor, args)) = error.code.and_then(|code| parser_error_descriptor(code, error))
    else {
        return Diagnostic::surge_parser_error(error.message.clone(), file_name.to_string());
    };

    crate::spans::diagnostic_with_syntax_span(
        Diagnostic::from_descriptor(descriptor, args, file_name.to_string()),
        error.span,
    )
}

/// Codes whose one argument is the text oxc's label covers: the modifier, or
/// the `this` of a misplaced `this` parameter (TS2680). oxc words some of them
/// differently from tsc (TS1031 on a constructor, TS1273), so the argument
/// cannot always be read back out of its message.
const MODIFIER_LABEL_CODES: &[u32] = &[1030, 1031, 1070, 1071, 1090, 1273, 2680];

fn parser_error_descriptor(
    code: u32,
    error: &surge_ts_syntax::ParserError,
) -> Option<(
    &'static surge_ts_diagnostics::DiagnosticDescriptor,
    Vec<surge_ts_diagnostics::DiagnosticArg>,
)> {
    if let Some(descriptor) = surge_ts_diagnostics::emitted_descriptor_for_number(code) {
        return Some((descriptor, Vec::new()));
    }
    let descriptor = surge_ts_diagnostics::emitted_descriptor_for_number_with_arity(code, 1)?;
    let argument = match_message_template(descriptor.message_template, &error.message)
        .and_then(|mut args| (args.len() == 1).then(|| args.remove(0)))
        .or_else(|| {
            MODIFIER_LABEL_CODES
                .contains(&code)
                .then(|| error.span_text.clone())
                .flatten()
        })?;
    Some((descriptor, vec![argument.into()]))
}

/// The arguments that fill `template`'s `{n}` placeholders to produce
/// `message`, tolerating a missing final period. A placeholder written twice
/// (`'{0}' … 'export default {0}'`) must be filled the same both times and is
/// one argument.
fn match_message_template(template: &str, message: &str) -> Option<Vec<String>> {
    let template = template.strip_suffix('.').unwrap_or(template);
    let message = message.strip_suffix('.').unwrap_or(message);
    let mut pieces = Vec::new();
    let mut indices = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}')? + open;
        pieces.push(&rest[..open]);
        indices.push(rest[open + 1..close].parse::<usize>().ok()?);
        rest = &rest[close + 1..];
    }
    let (first, literals) = pieces.split_first().map_or((rest, &[][..]), |(f, l)| (*f, l));
    let mut remaining = message.strip_prefix(first)?;
    let mut filled = Vec::new();
    for literal in literals.iter().copied().chain(std::iter::once(rest)) {
        if literal.is_empty() {
            filled.push(remaining.to_string());
            remaining = "";
            continue;
        }
        let at = remaining.find(literal)?;
        filled.push(remaining[..at].to_string());
        remaining = &remaining[at + literal.len()..];
    }
    if !remaining.is_empty() || pieces.is_empty() {
        return None;
    }
    let mut args: Vec<Option<String>> = vec![None; indices.iter().max().map_or(0, |max| max + 1)];
    for (index, value) in indices.into_iter().zip(filled) {
        match &args[index] {
            Some(existing) if *existing != value => return None,
            _ => args[index] = Some(value),
        }
    }
    args.into_iter().collect()
}

pub(super) fn emit_parser_diagnostics(parsed_files: &[ParsedProgramFile], ctx: &mut CheckerContext) {
    for parsed_file in parsed_files {
        ctx.set_file_name(parsed_file.file_name.clone());

        let diagnostics: Vec<Diagnostic> = super::check_files::unclaimed_parser_errors(
            &parsed_file.parser_errors,
            &parsed_file.grammar_diagnostics,
            ctx,
        )
        .map(|error| parser_error_diagnostic(error, &parsed_file.file_name))
        .collect();
        for diagnostic in diagnostics {
            ctx.push(diagnostic);
        }
    }
}

pub(super) type DiagnosticDedupKey = (
    String,
    String,
    String,
    Option<surge_ts_diagnostics::TextSpan>,
);

pub(super) fn diagnostic_dedup_key(diagnostic: &surge_ts_diagnostics::Diagnostic) -> DiagnosticDedupKey {
    (
        diagnostic.code.to_string(),
        diagnostic.file_name.clone(),
        diagnostic.message.clone(),
        diagnostic.span,
    )
}

/// Order-preserving diagnostic dedup backed by a key set. The previous
/// implementation rescanned the whole accumulated `Vec` (and re-rendered every
/// code to a `String`) for each incoming diagnostic, so merging N files' results
/// was O(D^2) in the total diagnostic count. Hoisting the set across the merge
/// keeps it O(D).
#[derive(Default)]
pub(super) struct DiagnosticDeduper {
    pub(super) seen: HashSet<DiagnosticDedupKey>,
}

impl DiagnosticDeduper {
    pub(super) fn with_existing(diagnostics: &[surge_ts_diagnostics::Diagnostic]) -> Self {
        Self {
            seen: diagnostics.iter().map(diagnostic_dedup_key).collect(),
        }
    }

    pub(super) fn extend(
        &mut self,
        diagnostics: &mut Vec<surge_ts_diagnostics::Diagnostic>,
        new_diagnostics: Vec<surge_ts_diagnostics::Diagnostic>,
    ) {
        for diagnostic in new_diagnostics {
            if self.seen.insert(diagnostic_dedup_key(&diagnostic)) {
                diagnostics.push(diagnostic);
            }
        }
    }
}

pub(crate) fn extend_diagnostics_dedup(
    diagnostics: &mut Vec<surge_ts_diagnostics::Diagnostic>,
    new_diagnostics: Vec<surge_ts_diagnostics::Diagnostic>,
) {
    DiagnosticDeduper::with_existing(diagnostics).extend(diagnostics, new_diagnostics);
}

/// `SURGE_REPORT_UNUSED_EXPECT_ERROR=1` turns on TS2578. The check matches
/// tsc, but every real error surge misses under an `@ts-expect-error` becomes
/// a TS2578 false positive, which on the real-project corpora outnumbered the
/// directives surge does satisfy. Off until surge's recall catches up.
fn report_unused_expect_error() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SURGE_REPORT_UNUSED_EXPECT_ERROR").as_deref() == Ok("1")
    })
}

/// tsc's `getDiagnosticsWithPrecedingDirectives` plus the unused-directive
/// report that follows it: a diagnostic starting on a directive's suppressed
/// line is dropped and marks the nearest directive above it used; with the
/// gate on, every `@ts-expect-error` left unused is TS2578 at the directive.
/// An `@ts-ignore` never reports. Runs only for files surge checks in full: a
/// declaration file returns before it, since its statements are never checked
/// and every directive there would read as unused. A file with parse errors
/// reports no unused directive either: tsc skips semantic diagnostics once any
/// syntax error exists, and surge's checker may not have seen the file's
/// statements.
pub(crate) fn apply_comment_directives(
    diagnostics: &mut Vec<surge_ts_diagnostics::Diagnostic>,
    directives: &[surge_ts_syntax::CommentDirective],
    has_parse_errors: bool,
    file_name: &str,
) {
    if directives.is_empty() {
        return;
    }
    let mut used = vec![false; directives.len()];
    diagnostics.retain(|diagnostic| {
        let Some(span) = diagnostic.span.as_ref() else {
            return true;
        };
        let nearest = directives
            .iter()
            .enumerate()
            .filter(|(_, directive)| {
                directive
                    .suppressed_line
                    .is_some_and(|line| span.start >= line.start && span.start <= line.end)
            })
            .max_by_key(|(_, directive)| directive.span.start);
        match nearest {
            Some((index, _)) => {
                used[index] = true;
                false
            }
            None => true,
        }
    });
    if !report_unused_expect_error() {
        return;
    }
    for (directive, used) in directives.iter().zip(used) {
        if used
            || has_parse_errors
            || directive.kind != surge_ts_syntax::CommentDirectiveKind::ExpectError
        {
            continue;
        }
        let diagnostic = Diagnostic::ts2578(file_name.to_string())
            .with_span(crate::context::convert_span(directive.span));
        let position = diagnostics
            .iter()
            .position(|existing| {
                existing
                    .span
                    .as_ref()
                    .is_some_and(|span| span.start > directive.span.start)
            })
            .unwrap_or(diagnostics.len());
        diagnostics.insert(position, diagnostic);
    }
}

/// [`drop_suppressed_diagnostics`] for the semantic diagnostics reported
/// against a file outside its own check — import binding runs first — which
/// tsc's directive filter covers all the same. Syntax errors are never
/// suppressed. The directive a dropped diagnostic uses was reported unused
/// by the file's own check (TS2578), which is withdrawn.
pub(crate) fn drop_suppressed_program_diagnostics(
    diagnostics: &mut Vec<surge_ts_diagnostics::Diagnostic>,
    parsed_files: &[ParsedProgramFile],
) {
    let directives_by_file: std::collections::HashMap<&str, &[surge_ts_syntax::CommentDirective]> = parsed_files
        .iter()
        .filter(|file| file.comment_directives.iter().any(|directive| directive.suppressed_line.is_some()))
        .map(|file| (file.file_name.as_str(), file.comment_directives.as_slice()))
        .collect();
    if directives_by_file.is_empty() {
        return;
    }
    let mut used: HashSet<(String, usize)> = HashSet::new();
    diagnostics.retain(|diagnostic| {
        let (Some(span), Some(directives)) = (
            diagnostic.span.as_ref(),
            directives_by_file.get(diagnostic.file_name.as_str()),
        ) else {
            return true;
        };
        if is_syntactic_diagnostic(diagnostic)
            || matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(2578))
        {
            return true;
        }
        let nearest = directives
            .iter()
            .filter(|directive| {
                directive
                    .suppressed_line
                    .is_some_and(|line| span.start >= line.start && span.start <= line.end)
            })
            .max_by_key(|directive| directive.span.start);
        match nearest {
            Some(directive) => {
                used.insert((diagnostic.file_name.clone(), directive.span.start));
                false
            }
            None => true,
        }
    });
    if used.is_empty() {
        return;
    }
    diagnostics.retain(|diagnostic| {
        !matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(2578))
            || !diagnostic
                .span
                .as_ref()
                .is_some_and(|span| used.contains(&(diagnostic.file_name.clone(), span.start)))
    });
}
