use oxc_span::Span;

use crate::TextSpan;

pub(crate) fn text_span_from_oxc_span(span: Span) -> TextSpan {
    TextSpan {
        start: span.start as usize,
        end: span.end as usize,
    }
}

thread_local! {
    static LOWERING_SOURCE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static LOWERING_JAVASCRIPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes `source_text` readable to the lowering for the duration of `f`. oxc
/// records no span for an operator token, so the one diagnostic tsc anchors
/// there has to find it in the text between the operands. `javascript` says
/// the file is JavaScript, whose untyped signatures lower differently.
pub(crate) fn with_lowering_source<R>(
    source_text: &str,
    javascript: bool,
    f: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<String>, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            LOWERING_SOURCE.with(|source| *source.borrow_mut() = self.0.take());
            LOWERING_JAVASCRIPT.with(|flag| flag.set(self.1));
        }
    }
    let previous =
        LOWERING_SOURCE.with(|source| source.borrow_mut().replace(source_text.to_string()));
    let previous_javascript = LOWERING_JAVASCRIPT.with(|flag| flag.replace(javascript));
    let _restore = Restore(previous, previous_javascript);
    f()
}

/// The source text `span` covers, empty outside [`with_lowering_source`].
pub(crate) fn source_text_of(span: Span) -> String {
    LOWERING_SOURCE.with(|source| {
        source
            .borrow()
            .as_deref()
            .and_then(|text| text.get(span.start as usize..span.end as usize))
            .unwrap_or_default()
            .to_string()
    })
}

/// A computed key's span widened to its brackets. `None` outside
/// [`with_lowering_source`].
pub(crate) fn bracketed_key_span(key: Span) -> Option<TextSpan> {
    LOWERING_SOURCE.with(|source| {
        let source = source.borrow();
        let text = source.as_deref()?;
        let open = text.get(..key.start as usize)?.rfind('[')?;
        let close = key.end as usize + text.get(key.end as usize..)?.find(']')? + 1;
        Some(TextSpan { start: open, end: close })
    })
}

/// tsc's `getErrorSpanForNode` for a function expression: its name, or else
/// its first token (`async` or `function`).
pub(crate) fn function_expression_error_span(function: &oxc_ast::ast::Function<'_>) -> Span {
    if let Some(id) = &function.id {
        return id.span;
    }
    let keyword = if function.r#async { "async" } else { "function" };
    Span::new(function.span.start, (function.span.start + keyword.len() as u32).min(function.span.end))
}

/// tsc's `getErrorSpanForNode` for a class expression: its name, or else its
/// `class` keyword.
pub(crate) fn class_expression_error_span(class: &oxc_ast::ast::Class<'_>) -> Span {
    match &class.id {
        Some(id) => id.span,
        None => Span::new(class.span.start, (class.span.start + "class".len() as u32).min(class.span.end)),
    }
}

/// Whether the file being lowered is JavaScript.
pub(crate) fn lowering_javascript() -> bool {
    LOWERING_JAVASCRIPT.with(std::cell::Cell::get)
}

/// The span of `keyword`, the first token written between `left_end` and
/// `right_start` after whitespace and comments. `None` outside
/// [`with_lowering_source`] or when something else comes first.
pub(crate) fn keyword_span_between(left_end: u32, right_start: u32, keyword: &str) -> Option<crate::TextSpan> {
    LOWERING_SOURCE.with(|source| {
        let source = source.borrow();
        let text = source.as_deref()?.get(left_end as usize..right_start as usize)?;
        let mut rest = text;
        loop {
            let trimmed = rest.trim_start();
            if let Some(comment) = trimmed.strip_prefix("/*") {
                rest = &comment[comment.find("*/")? + 2..];
            } else if let Some(comment) = trimmed.strip_prefix("//") {
                rest = &comment[comment.find('\n')?..];
            } else {
                rest = trimmed;
                break;
            }
        }
        if !rest.starts_with(keyword) {
            return None;
        }
        let start = left_end as usize + (text.len() - rest.len());
        Some(crate::TextSpan { start, end: start + keyword.len() })
    })
}

/// The span of the single-character `operator` written between `left_end` and
/// `right_start`, skipping comments. `None` outside [`with_lowering_source`].
pub(crate) fn operator_token_span(
    left_end: u32,
    right_start: u32,
    operator: u8,
) -> Option<crate::TextSpan> {
    LOWERING_SOURCE.with(|source| {
        let source = source.borrow();
        let bytes = source.as_deref()?.as_bytes();
        let (start, end) = (left_end as usize, (right_start as usize).min(bytes.len()));
        let mut index = start;
        while index < end {
            match (bytes[index], bytes.get(index + 1)) {
                (b'/', Some(b'*')) => {
                    index += 2;
                    while index + 1 < end && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                        index += 1;
                    }
                    index += 2;
                }
                (b'/', Some(b'/')) => {
                    while index < end && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                (byte, _) if byte == operator => {
                    return Some(crate::TextSpan {
                        start: index,
                        end: index + 1,
                    });
                }
                _ => index += 1,
            }
        }
        None
    })
}
