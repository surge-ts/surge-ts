//! `@ts-expect-error` / `@ts-ignore` comment directives.
//!
//! Each directive suppresses every diagnostic reported on the next line that
//! carries code: tsc walks *backwards* from a diagnostic's line over blank and
//! `//`-comment lines looking for a directive, so an
//! `// @ts-expect-error` followed by an `// eslint-disable-next-line` still
//! suppresses the statement below both. The checker reports an
//! `@ts-expect-error` that suppressed nothing as TS2578 at the directive.

use oxc_ast::{Comment, CommentKind};

use crate::{CommentDirective, CommentDirectiveKind, TextSpan};

/// A file's directives, in source order.
pub(crate) fn collect_comment_directives(
    source_text: &str,
    comments: &[Comment],
) -> Vec<CommentDirective> {
    let bytes = source_text.as_bytes();
    let mut directives: Vec<CommentDirective> = Vec::new();

    for comment in comments {
        let start = comment.span.start as usize;
        let end = (comment.span.end as usize).min(bytes.len());
        if start >= end {
            continue;
        }
        let Some(directive) = comment_directive(bytes, start, end, comment.kind) else {
            continue;
        };
        if directives.last().is_some_and(|last| last.span == directive.span) {
            continue;
        }
        directives.push(directive);
    }

    directives.sort_by_key(|directive| directive.span.start);
    directives.dedup();
    directives
}

/// tsc's `processCommentDirective`: the directive must be the first thing in
/// the comment after its delimiters — for a block comment, the first thing on
/// its last line — and the location tsc reports is the comment for a `//`
/// comment and that last line for a block comment.
fn comment_directive(
    bytes: &[u8],
    start: usize,
    end: usize,
    kind: CommentKind,
) -> Option<CommentDirective> {
    let mut position = start;
    let location_start = match kind {
        CommentKind::Line => {
            position += 2;
            while position < end && bytes[position] == b'/' {
                position += 1;
            }
            start
        }
        CommentKind::SingleLineBlock | CommentKind::MultiLineBlock => {
            let last_line_start = bytes[start..end]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(start, |offset| start + offset + 1);
            position = last_line_start;
            while position < end && (bytes[position] == b' ' || bytes[position] == b'\t') {
                position += 1;
            }
            while position < end && (bytes[position] == b'/' || bytes[position] == b'*') {
                position += 1;
            }
            last_line_start
        }
    };
    while position < end && (bytes[position] == b' ' || bytes[position] == b'\t') {
        position += 1;
    }
    if position >= end || bytes[position] != b'@' {
        return None;
    }
    position += 1;
    let rest = &bytes[position..end];
    let kind = if rest.starts_with(b"ts-expect-error") {
        CommentDirectiveKind::ExpectError
    } else if rest.starts_with(b"ts-ignore") {
        CommentDirectiveKind::Ignore
    } else {
        return None;
    };
    let suppressed_line = next_line_range(bytes, end);
    Some(CommentDirective {
        kind,
        span: TextSpan {
            start: location_start,
            end,
        },
        covers_type_declaration: suppressed_line.is_some_and(|line| starts_type_declaration(&bytes[line.start..line.end])),
        suppressed_line,
    })
}

fn starts_type_declaration(line: &[u8]) -> bool {
    let mut words = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .skip_while(|word| matches!(*word, b"export" | b"declare"));
    matches!(words.next(), Some(b"type" | b"interface"))
}

/// The byte range of the first line after `offset` that is neither blank nor a
/// `//` comment — the line tsc's backwards walk stops at, and therefore the one
/// the directive suppresses. `None` when the directive is followed by no such
/// line.
fn next_line_range(bytes: &[u8], offset: usize) -> Option<TextSpan> {
    let mut index = offset;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }

    loop {
        if index >= bytes.len() {
            return None;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'\n' {
            end += 1;
        }
        if !is_comment_or_blank_line(bytes, start, end) {
            return Some(TextSpan { start, end });
        }
        index = end;
    }
}

/// tsc's `isCommentOrBlankLine`: whitespace only, or the first non-whitespace
/// characters are `//`. A block comment does not count, exactly as there.
fn is_comment_or_blank_line(bytes: &[u8], start: usize, end: usize) -> bool {
    let mut index = start;
    while index < end && (bytes[index] == b' ' || bytes[index] == b'\t') {
        index += 1;
    }
    if index >= end || bytes[index] == b'\r' {
        return true;
    }
    index + 1 < end && bytes[index] == b'/' && bytes[index + 1] == b'/'
}
