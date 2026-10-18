//! tsc-compatible diagnostic rendering.
//!
//! Reproduces the `tsc` terminal output for both `--pretty false` (one line per
//! diagnostic) and `--pretty true` (file/line/column header, source excerpt,
//! squiggle underline, and a summary footer). ANSI escape sequences match the
//! ones emitted by `tsc`'s `formatDiagnosticsWithColorAndContext`.
//!
//! This is a renderer only: it never changes which diagnostics, spans, or
//! messages the checker produces. Oracle comparison consumes the JSON output and
//! is unaffected by anything here.

use std::collections::{HashMap, HashSet};

use crate::{Diagnostic, LineIndex};

const CYAN: &str = "\x1b[96m";
const YELLOW: &str = "\x1b[93m";
const RED: &str = "\x1b[91m";
const GRAY: &str = "\x1b[90m";
const INVERSE: &str = "\x1b[7m";
const RESET: &str = "\x1b[0m";

#[derive(Debug, Clone, Copy)]
pub struct TscRenderOptions {
    pub pretty: bool,
    pub color: bool,
}

/// One diagnostic paired with the display label and source text needed to render
/// it. `label` is the file path exactly as it should appear (already relativized
/// by the caller); it is empty for diagnostics with no associated file (such as
/// global "Cannot find global type" diagnostics or command-line diagnostics).
/// `source_text` is that file's contents, or empty when the source is
/// unavailable — the renderer then degrades to a header-only frame rather than
/// panicking.
pub struct TscRenderItem<'a> {
    pub label: &'a str,
    pub source_text: &'a str,
    pub diagnostic: &'a Diagnostic,
}

struct Location<'a> {
    line: usize,
    column: usize,
    source_line: &'a str,
    squiggle_chars: usize,
}

/// Render diagnostics in `tsc`-compatible form. Returns an empty string when
/// there are no diagnostics (matching `tsc`, which prints nothing on success).
/// The returned string already contains the trailing newline(s) `tsc` emits, so
/// callers should `print!` it rather than `println!`.
pub fn render_diagnostics_tsc(items: &[TscRenderItem<'_>], options: TscRenderOptions) -> String {
    if items.is_empty() {
        return String::new();
    }

    let locations = locate_all(items);
    if options.pretty {
        render_pretty(items, &locations, options.color)
    } else {
        render_plain(items, &locations)
    }
}

/// Locate every item up front, sharing one [`LineIndex`] per distinct source
/// text (keyed by slice identity), so rendering never rescans a file from byte
/// 0 per diagnostic.
fn locate_all<'a>(items: &[TscRenderItem<'a>]) -> Vec<Option<Location<'a>>> {
    let mut indices: HashMap<(usize, usize), LineIndex> = HashMap::new();
    items
        .iter()
        .map(|item| {
            let source = item.source_text;
            if item.diagnostic.span.is_none() || source.is_empty() {
                return None;
            }
            let index = indices
                .entry((source.as_ptr() as usize, source.len()))
                .or_insert_with(|| LineIndex::new(source));
            locate(item, index)
        })
        .collect()
}

fn render_plain(items: &[TscRenderItem<'_>], locations: &[Option<Location<'_>>]) -> String {
    let mut out = String::new();
    for (item, location) in items.iter().zip(locations) {
        out.push_str(&plain_header(item, location.as_ref()));
        out.push('\n');
    }
    out
}

fn plain_header(item: &TscRenderItem<'_>, location: Option<&Location<'_>>) -> String {
    let code = item.diagnostic.code.to_string();
    let prefix = match (item.label.is_empty(), location) {
        (false, Some(loc)) => format!("{}({},{}): ", item.label, loc.line, loc.column),
        (false, None) => format!("{}: ", item.label),
        (true, _) => String::new(),
    };
    format!("{prefix}error {code}: {}", item.diagnostic.message)
}

fn render_pretty(
    items: &[TscRenderItem<'_>],
    locations: &[Option<Location<'_>>],
    color: bool,
) -> String {
    let mut out = String::new();
    for (item, location) in items.iter().zip(locations) {
        out.push_str(&pretty_header(item, location.as_ref(), color));
        out.push('\n');

        if let Some(loc) = location {
            out.push('\n');
            out.push_str(&pretty_source_line(loc, color));
            out.push('\n');
            out.push_str(&pretty_squiggle_line(loc, color));
            out.push('\n');
            out.push('\n');
        }
    }

    out.push('\n');
    out.push_str(&footer(items, locations, color));
    out
}

fn pretty_header(item: &TscRenderItem<'_>, location: Option<&Location<'_>>, color: bool) -> String {
    let code = item.diagnostic.code.to_string();
    let message = &item.diagnostic.message;

    let location_part = match (item.label.is_empty(), location) {
        (false, Some(loc)) if color => format!(
            "{CYAN}{}{RESET}:{YELLOW}{}{RESET}:{YELLOW}{}{RESET} - ",
            item.label, loc.line, loc.column
        ),
        (false, Some(loc)) => format!("{}:{}:{} - ", item.label, loc.line, loc.column),
        _ => String::new(),
    };

    if color {
        format!("{location_part}{RED}error{RESET}{GRAY} {code}: {RESET}{message}")
    } else {
        format!("{location_part}error {code}: {message}")
    }
}

fn pretty_source_line(loc: &Location<'_>, color: bool) -> String {
    let gutter = loc.line.to_string();
    let content = display_line(loc.source_line);
    if color {
        format!("{INVERSE}{gutter}{RESET} {content}")
    } else {
        format!("{gutter} {content}")
    }
}

fn pretty_squiggle_line(loc: &Location<'_>, color: bool) -> String {
    let gutter_width = loc.line.to_string().len();
    let gutter_pad = " ".repeat(gutter_width);
    let leading = " ".repeat(loc.column.saturating_sub(1));
    let tildes = "~".repeat(loc.squiggle_chars.max(1));
    if color {
        format!("{INVERSE}{gutter_pad}{RESET} {RED}{leading}{tildes}{RESET}")
    } else {
        format!("{gutter_pad} {leading}{tildes}")
    }
}

fn footer(items: &[TscRenderItem<'_>], locations: &[Option<Location<'_>>], color: bool) -> String {
    let count = items.len();

    let located: Vec<(&str, usize)> = items
        .iter()
        .zip(locations)
        .filter_map(|(item, location)| location.as_ref().map(|loc| (item.label, loc.line)))
        .collect();

    let first_ref = located
        .first()
        .map(|(label, line)| reference(label, *line, color));

    let mut seen_labels = HashSet::new();
    let mut distinct_files: Vec<(&str, usize)> = Vec::new();
    for (label, line) in &located {
        if seen_labels.insert(*label) {
            distinct_files.push((*label, *line));
        }
    }

    if count == 1 {
        return match first_ref {
            Some(reference) => format!("Found 1 error in {reference}\n\n"),
            None => "Found 1 error.\n\n".to_string(),
        };
    }

    if distinct_files.len() <= 1 {
        return match first_ref {
            Some(reference) => {
                format!("Found {count} errors in the same file, starting at: {reference}\n\n")
            }
            None => format!("Found {count} errors.\n\n"),
        };
    }

    let mut out = format!(
        "Found {count} errors in {} files.\n\n",
        distinct_files.len()
    );
    // tsgo's `writeTabularErrorsDisplay` lists the files by name and ends the
    // table with a blank line.
    distinct_files.sort_by(|a, b| a.0.cmp(b.0));
    out.push_str(&error_table(&located, &distinct_files, color));
    out.push('\n');
    out
}

fn error_table(located: &[(&str, usize)], distinct_files: &[(&str, usize)], color: bool) -> String {
    let mut counts_by_label: HashMap<&str, usize> = HashMap::new();
    for (label, _) in located {
        *counts_by_label.entry(label).or_default() += 1;
    }
    let counts: Vec<usize> = distinct_files
        .iter()
        .map(|(label, _)| counts_by_label.get(label).copied().unwrap_or(0))
        .collect();

    let max_count = counts.iter().copied().max().unwrap_or(0);
    let heading = "Errors";
    let left_goal = heading.len().max(digit_count(max_count));

    let header_pad = " ".repeat(digit_count(max_count).saturating_sub(heading.len()));
    let mut out = format!("{header_pad}{heading}  Files\n");

    for ((label, line), file_count) in distinct_files.iter().zip(counts) {
        let pad = " ".repeat(left_goal.saturating_sub(digit_count(file_count)));
        out.push_str(&format!(
            "{pad}{file_count}  {}\n",
            reference(label, *line, color)
        ));
    }

    out
}

fn reference(label: &str, line: usize, color: bool) -> String {
    if color {
        format!("{label}{GRAY}:{line}{RESET}")
    } else {
        format!("{label}:{line}")
    }
}

fn digit_count(value: usize) -> usize {
    value
        .checked_ilog10()
        .map(|log| log as usize + 1)
        .unwrap_or(1)
}

fn locate<'a>(item: &TscRenderItem<'a>, index: &LineIndex) -> Option<Location<'a>> {
    let span = item.diagnostic.span?;
    let source = item.source_text;
    if source.is_empty() {
        return None;
    }

    let (line, column) = index.line_col(source, span.start);
    let (line_start, line_end) = index.line_bounds(source, span.start);
    let source_line = &source[line_start..line_end];

    let squiggle_start = span.start.min(source.len());
    let squiggle_end = span.end.min(line_end).max(squiggle_start);
    let squiggle_chars = source[squiggle_start..squiggle_end].chars().count().max(1);

    Some(Location {
        line,
        column,
        source_line,
        squiggle_chars,
    })
}

/// `tsc` strips trailing whitespace from the excerpt and renders tabs as a
/// single space; both keep the squiggle (which uses 1 column per tab) aligned.
fn display_line(source_line: &str) -> String {
    source_line.trim_end().replace('\t', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diagnostic, DiagnosticCode, TextSpan};

    fn diag(code: u32, message: &str, file: &str, start: usize, end: usize) -> Diagnostic {
        Diagnostic::new(DiagnosticCode::TypeScript(code), message, file)
            .with_span(TextSpan { start, end })
    }

    #[test]
    fn plain_single_diagnostic_matches_tsc() {
        let source = "const a = 1;\nexport {};\na = 3;\n";
        let d = diag(
            2588,
            "Cannot assign to 'a' because it is a constant.",
            "src/index.ts",
            24,
            25,
        );
        let items = [TscRenderItem {
            label: "src/index.ts",
            source_text: source,
            diagnostic: &d,
        }];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: false,
                color: false,
            },
        );
        assert_eq!(
            out,
            "src/index.ts(3,1): error TS2588: Cannot assign to 'a' because it is a constant.\n"
        );
    }

    #[test]
    fn pretty_color_single_diagnostic_matches_tsc_bytes() {
        let source = "const a = 1;\nexport {};\na = 3;\n";
        let d = diag(
            2588,
            "Cannot assign to 'a' because it is a constant.",
            "src/index.ts",
            24,
            25,
        );
        let items = [TscRenderItem {
            label: "src/index.ts",
            source_text: source,
            diagnostic: &d,
        }];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: true,
                color: true,
            },
        );
        let expected = "\x1b[96msrc/index.ts\x1b[0m:\x1b[93m3\x1b[0m:\x1b[93m1\x1b[0m - \x1b[91merror\x1b[0m\x1b[90m TS2588: \x1b[0mCannot assign to 'a' because it is a constant.\n\n\x1b[7m3\x1b[0m a = 3;\n\x1b[7m \x1b[0m \x1b[91m~\x1b[0m\n\n\nFound 1 error in src/index.ts\x1b[90m:3\x1b[0m\n\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn pretty_plain_single_diagnostic_strips_ansi() {
        let source = "const a = 1;\nexport {};\na = 3;\n";
        let d = diag(
            2588,
            "Cannot assign to 'a' because it is a constant.",
            "src/index.ts",
            24,
            25,
        );
        let items = [TscRenderItem {
            label: "src/index.ts",
            source_text: source,
            diagnostic: &d,
        }];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: true,
                color: false,
            },
        );
        let expected = "src/index.ts:3:1 - error TS2588: Cannot assign to 'a' because it is a constant.\n\n3 a = 3;\n  ~\n\n\nFound 1 error in src/index.ts:3\n\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn empty_diagnostics_render_nothing() {
        let out = render_diagnostics_tsc(
            &[],
            TscRenderOptions {
                pretty: true,
                color: true,
            },
        );
        assert!(out.is_empty());
    }

    #[test]
    fn plain_multiple_files() {
        let a = "export {};\nlet a: number = \"x\";\n";
        let b = "export {};\nlet c: number = \"z\";\n";
        let d1 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/a.ts",
            26,
            29,
        );
        let d2 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/b.ts",
            26,
            29,
        );
        let items = [
            TscRenderItem {
                label: "src/a.ts",
                source_text: a,
                diagnostic: &d1,
            },
            TscRenderItem {
                label: "src/b.ts",
                source_text: b,
                diagnostic: &d2,
            },
        ];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: false,
                color: false,
            },
        );
        assert_eq!(
            out,
            "src/a.ts(2,16): error TS2322: Type 'string' is not assignable to type 'number'.\nsrc/b.ts(2,16): error TS2322: Type 'string' is not assignable to type 'number'.\n"
        );
    }

    #[test]
    fn pretty_same_file_two_errors_footer() {
        let source = "export {};\nlet a: number = \"x\";\nlet b: number = \"y\";\n";
        let d1 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/a.ts",
            26,
            29,
        );
        let d2 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/a.ts",
            46,
            49,
        );
        let items = [
            TscRenderItem {
                label: "src/a.ts",
                source_text: source,
                diagnostic: &d1,
            },
            TscRenderItem {
                label: "src/a.ts",
                source_text: source,
                diagnostic: &d2,
            },
        ];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: true,
                color: false,
            },
        );
        assert!(out.ends_with("Found 2 errors in the same file, starting at: src/a.ts:2\n\n"));
    }

    #[test]
    fn pretty_multi_file_footer_table() {
        let a = "export {};\nlet a: number = \"x\";\nlet b: number = \"y\";\n";
        let b = "export {};\nlet c: number = \"z\";\n";
        let d1 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/a.ts",
            26,
            29,
        );
        let d2 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/a.ts",
            46,
            49,
        );
        let d3 = diag(
            2322,
            "Type 'string' is not assignable to type 'number'.",
            "src/b.ts",
            26,
            29,
        );
        let items = [
            TscRenderItem {
                label: "src/a.ts",
                source_text: a,
                diagnostic: &d1,
            },
            TscRenderItem {
                label: "src/a.ts",
                source_text: a,
                diagnostic: &d2,
            },
            TscRenderItem {
                label: "src/b.ts",
                source_text: b,
                diagnostic: &d3,
            },
        ];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: true,
                color: false,
            },
        );
        assert!(out.contains(
            "Found 3 errors in 2 files.\n\nErrors  Files\n     2  src/a.ts:2\n     1  src/b.ts:2\n"
        ));
    }

    #[test]
    fn no_file_diagnostic_omits_location() {
        let d = Diagnostic::new(
            DiagnosticCode::TypeScript(2318),
            "Cannot find global type 'Array'.",
            "",
        );
        let items = [TscRenderItem {
            label: "",
            source_text: "",
            diagnostic: &d,
        }];
        let out = render_diagnostics_tsc(
            &items,
            TscRenderOptions {
                pretty: false,
                color: false,
            },
        );
        assert_eq!(out, "error TS2318: Cannot find global type 'Array'.\n");
    }
}
