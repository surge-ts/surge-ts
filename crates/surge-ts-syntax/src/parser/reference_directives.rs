use crate::{ReferenceTypeDirective, ResolutionModeOverride, TextSpan};

/// Collect leading `/// <reference types="..." />` directives from a source file
/// without a full parse. Mirroring tsc, only comments in the leading trivia of
/// the first token are scanned: an optional hashbang, then any run of whitespace,
/// line comments, and block comments; the scan stops at the first real token.
/// Cheap enough to run over every file (including large library declarations).
pub fn extract_reference_type_directives(source_text: &str) -> Vec<ReferenceTypeDirective> {
    extract_reference_directives_named(source_text, "types")
}

/// Collect leading `/// <reference path="..." />` directives. The path is a
/// file specifier resolved relative to the referencing file; callers do the
/// resolution. Like the `types` scanner, only leading trivia is examined.
pub fn extract_reference_path_directives(source_text: &str) -> Vec<ReferenceTypeDirective> {
    extract_reference_directives_named(source_text, "path")
}

/// The values of leading `types` reference directives' `resolution-mode`
/// attributes that are neither `require` nor `import` (tsc's
/// `parseResolutionMode`, TS1453), as spans of the value text.
pub fn invalid_reference_resolution_modes(source_text: &str) -> Vec<TextSpan> {
    for_each_leading_line_comment(source_text, |comment, base| {
        let after_slashes = comment.strip_prefix("///")?;
        let after_reference = after_slashes.trim_start().strip_prefix("<reference")?;
        if !after_reference.chars().next().is_some_and(char::is_whitespace) {
            return None;
        }
        extract_named_attribute(comment, "types")?;
        let (mode, start, end) = extract_named_attribute(comment, "resolution-mode")?;
        (mode != "import" && mode != "require").then_some(TextSpan {
            start: base + start,
            end: base + end,
        })
    })
}

/// tsc's `CheckJsDirective`: the last `// @ts-check` (`Some(true)`) or
/// `// @ts-nocheck` (`Some(false)`) line comment in the file's leading trivia.
pub fn extract_check_directive(source_text: &str) -> Option<bool> {
    let mut directive = None;
    for_each_leading_line_comment(source_text, |comment, _| {
        let body = comment.strip_prefix("///").or_else(|| comment.strip_prefix("//"))?;
        let name = body.trim_start_matches([' ', '\t']).strip_prefix('@')?;
        let end = name
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(name.len());
        match name[..end].to_ascii_lowercase().as_str() {
            "ts-check" => directive = Some(true),
            "ts-nocheck" => directive = Some(false),
            _ => {}
        }
        None::<()>
    });
    directive
}

/// Calls `visit` with each line comment (and its byte offset) in the leading
/// trivia of the file's first token: an optional hashbang, then any run of
/// whitespace, line comments, and block comments.
fn for_each_leading_line_comment<T>(source_text: &str, mut visit: impl FnMut(&str, usize) -> Option<T>) -> Vec<T> {
    let bytes = source_text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut collected = Vec::new();

    if len >= 2 && bytes[0] == b'#' && bytes[1] == b'!' {
        while i < len && bytes[i] != b'\n' {
            i += 1;
        }
    }

    loop {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 >= len || bytes[i] != b'/' {
            break;
        }

        match bytes[i + 1] {
            b'/' => {
                let start = i;
                let mut end = i;
                while end < len && bytes[end] != b'\n' {
                    end += 1;
                }
                let mut text_end = end;
                if text_end > start && bytes[text_end - 1] == b'\r' {
                    text_end -= 1;
                }
                if let Some(item) = visit(&source_text[start..text_end], start) {
                    collected.push(item);
                }
                i = end;
            }
            b'*' => {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
            }
            _ => break,
        }
    }

    collected
}

fn extract_reference_directives_named(
    source_text: &str,
    attribute: &str,
) -> Vec<ReferenceTypeDirective> {
    for_each_leading_line_comment(source_text, |comment, start| {
        parse_reference_directive(comment, start, attribute)
    })
}

/// Parse a single leading line comment as a `/// <reference types="..." />`
/// directive. `comment_text` is the comment slice starting at the first `/`, and
/// `base` is that slice's byte offset in the source (used to anchor the returned
/// span). Returns `None` for any comment that is not a `types` reference
/// directive (regular comments, `path`/`lib` references, malformed input).
fn parse_reference_directive(
    comment_text: &str,
    base: usize,
    attribute: &str,
) -> Option<ReferenceTypeDirective> {
    let after_slashes = comment_text.strip_prefix("///")?;
    let after_reference = after_slashes.trim_start().strip_prefix("<reference")?;
    // `<reference` must be followed by whitespace before any attribute, matching
    // tsc's `^///\s*<reference\s`.
    if !after_reference
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        return None;
    }

    let (value, value_start, value_end) = extract_named_attribute(comment_text, attribute)?;
    // tsc's `parseResolutionMode`: any other value is an error and no override.
    let resolution_mode = match extract_named_attribute(comment_text, "resolution-mode") {
        Some((mode, _, _)) if mode == "import" => Some(ResolutionModeOverride::Import),
        Some((mode, _, _)) if mode == "require" => Some(ResolutionModeOverride::Require),
        _ => None,
    };
    Some(ReferenceTypeDirective {
        value,
        value_span: TextSpan {
            start: base + value_start,
            end: base + value_end,
        },
        resolution_mode,
    })
}

/// Locate a `<name> = "value"` attribute and return the value plus its byte range
/// within `text`. Other attributes (`lib`, `resolution-mode`, ...) are ignored.
fn extract_named_attribute(text: &str, name: &str) -> Option<(String, usize, usize)> {
    let bytes = text.as_bytes();
    let mut search_from = 0;

    while let Some(rel) = text[search_from..].find(name) {
        let key_start = search_from + rel;
        let key_end = key_start + name.len();
        search_from = key_end;

        // The attribute name must stand alone, not be a suffix of another key
        // (`no-types`) or part of a value (`path="my-types.d.ts"`).
        let preceded_by_name = key_start > 0 && is_attribute_name_byte(bytes[key_start - 1]);
        if preceded_by_name {
            continue;
        }

        let mut i = key_end;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            continue;
        }

        let quote = bytes[i];
        let value_start = i + 1;
        let rel_end = text[value_start..].find(quote as char)?;
        let value_end = value_start + rel_end;
        return Some((
            text[value_start..value_end].to_string(),
            value_start,
            value_end,
        ));
    }

    None
}

fn is_attribute_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<ReferenceTypeDirective> {
        parse_reference_directive(text, 0, "types")
    }

    #[test]
    fn parses_basic_types_reference() {
        let directive = parse(r#"/// <reference types="node" />"#).unwrap();
        assert_eq!(directive.value, "node");
        let span = directive.value_span;
        assert_eq!(
            &r#"/// <reference types="node" />"#[span.start..span.end],
            "node"
        );
    }

    #[test]
    fn parses_scoped_types_reference() {
        let directive = parse(r#"/// <reference types="@scope/pkg" />"#).unwrap();
        assert_eq!(directive.value, "@scope/pkg");
    }

    #[test]
    fn parses_single_quoted_and_extra_whitespace() {
        let directive = parse(r#"///   <reference   types =  'bar'  />"#).unwrap();
        assert_eq!(directive.value, "bar");
    }

    #[test]
    fn ignores_path_and_lib_references() {
        assert!(parse(r#"/// <reference path="./other.d.ts" />"#).is_none());
        assert!(parse(r#"/// <reference lib="dom" />"#).is_none());
    }

    #[test]
    fn ignores_non_directive_comments() {
        assert!(parse("// just a comment").is_none());
        assert!(parse("//// <reference types=\"node\" />").is_none());
        assert!(parse(r#"/// not a reference"#).is_none());
    }

    #[test]
    fn does_not_match_types_inside_other_attribute_values() {
        assert!(parse(r#"/// <reference path="my-types.d.ts" />"#).is_none());
    }

    #[test]
    fn span_is_offset_by_base() {
        let directive =
            parse_reference_directive(r#"/// <reference types="x" />"#, 100, "types").unwrap();
        assert_eq!(directive.value_span.start, 100 + 22);
        assert_eq!(directive.value_span.end, 100 + 23);
    }

    #[test]
    fn extracts_leading_directives_in_order() {
        let source = concat!(
            "/// <reference types=\"a\" />\n",
            "/// <reference types=\"@scope/b\" />\n",
            "export const x = 1;\n",
        );
        let directives = extract_reference_type_directives(source);
        let values: Vec<&str> = directives.iter().map(|d| d.value.as_str()).collect();
        assert_eq!(values, vec!["a", "@scope/b"]);
        let first = directives[0].value_span;
        assert_eq!(&source[first.start..first.end], "a");
    }

    #[test]
    fn stops_at_first_code_token() {
        let source = concat!(
            "/// <reference types=\"a\" />\n",
            "const y = 1;\n",
            "/// <reference types=\"b\" />\n",
        );
        let values: Vec<String> = extract_reference_type_directives(source)
            .into_iter()
            .map(|d| d.value)
            .collect();
        assert_eq!(values, vec!["a".to_string()]);
    }

    #[test]
    fn skips_hashbang_and_block_comments_before_directive() {
        let source = concat!(
            "#!/usr/bin/env node\n",
            "/* license */\n",
            "/// <reference types=\"a\" />\n",
            "declare const z: number;\n",
        );
        let values: Vec<String> = extract_reference_type_directives(source)
            .into_iter()
            .map(|d| d.value)
            .collect();
        assert_eq!(values, vec!["a".to_string()]);
    }

    #[test]
    fn no_directives_for_plain_declaration_file() {
        assert!(extract_reference_type_directives("interface Foo { a: number }\n").is_empty());
    }
}
