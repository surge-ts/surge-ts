use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::{ParsedSource, ParsedType};

/// A reusable parsing context that owns one oxc arena allocator and amortizes
/// its chunk allocations across many `parse` calls.
///
/// Safety model: [`ParsedSource`] is fully owned (`String`s and `Vec`s, no
/// lifetimes), so nothing returned by `parse` can reference the arena. The
/// arena is reset at the *start* of each parse, which also guarantees a valid
/// worker state after an earlier parse panicked or errored. A worker must not
/// be shared between threads (the arena is not thread-safe); create one worker
/// per parsing thread.
pub struct ParserWorker {
    allocator: Allocator,
}

impl ParserWorker {
    pub fn new() -> Self {
        Self {
            allocator: Allocator::default(),
        }
    }

    pub fn parse(&mut self, source_text: &str, file_name: &str) -> ParsedSource {
        // Mass-deallocates the previous file's AST (no Drop impls run; oxc AST
        // nodes are Drop-free by design) and keeps the largest chunk for reuse.
        self.allocator.reset();
        parse_source_in(&self.allocator, source_text, file_name)
    }
}

impl Default for ParserWorker {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot parse with a fresh arena. Prefer [`ParserWorker`] when parsing many
/// files in a loop.
pub fn parse_source(source_text: &str, file_name: &str) -> ParsedSource {
    let allocator = Allocator::default();
    parse_source_in(&allocator, source_text, file_name)
}

const FOR_AWAIT_IN_MESSAGE: &str = "await can only be used in conjunction with `for...of` statements";

/// Parse `source_text` into fully owned surge structures. Every borrow of
/// `allocator` ends inside this function: the returned [`ParsedSource`] holds
/// no references, pointers, or arena-backed strings, which is what makes
/// resetting the allocator between calls sound.
/// The TypeScript number of a failure oxc reports without one, where tsc
/// reports the same construct as a grammar error: an invalid write target —
/// TS2364 for an assignment, TS2357 for `++`/`--`, TS2779/TS2777 when it is an
/// optional chain — and a misplaced rest parameter (TS1014) or rest element
/// (TS2462).
/// The tsc code of a recovered parse error tsc reports from its checker
/// instead (`checkGrammarModifiers`, `checkGrammarArrowFunction`,
/// `checkGrammarIndexSignatureParameters`).
fn checker_grammar_code(error: &crate::ParserError) -> Option<u32> {
    match (error.code?, error.message.as_str()) {
        (1206, "Decorators are not valid here.")
        | (1275, "'accessor' modifier cannot be used here.")
        | (1200, "Line terminator not permitted before arrow")
        | (2370, "A rest parameter must be of an array type.") => error.code,
        (code @ (1013 | 1021 | 1096 | 1325 | 1326 | 7061), _) => Some(code),
        _ => None,
    }
}

fn classify_uncoded_parser_error(
    message: &str,
    span: crate::TextSpan,
    source_text: &str,
) -> Option<(u32, crate::TextSpan)> {
    let text = source_text.get(span.start..span.end)?;
    match message {
        // On the first token (`checkGrammarModifiers`).
        "Decorators are not valid here." => Some((1206, crate::TextSpan { start: span.start, end: span.start + 1 })),
        // On the modifier (`checkGrammarModifiers`).
        "'accessor' modifier cannot be used here." => Some((1275, span)),
        // On the `=>` (`checkGrammarArrowFunction`).
        "Line terminator not permitted before arrow" => Some((1200, span)),
        "Cannot assign to this expression" => {
            if let Some(stop) = update_target_stop(text, span, source_text) {
                return Some(stop);
            }
            // tsc's target node keeps the parentheses oxc's label drops.
            let mut span = span;
            loop {
                let before = source_text[..span.start].trim_end();
                let after = source_text[span.end..].trim_start();
                if !(before.ends_with('(') && after.starts_with(')')) {
                    break;
                }
                span = crate::TextSpan {
                    start: before.len() - 1,
                    end: source_text.len() - after.len() + 1,
                };
            }
            let before = source_text[..span.start].trim_end();
            let after = source_text[span.end..].trim_start();
            let is_update = before.ends_with("++")
                || before.ends_with("--")
                || after.starts_with("++")
                || after.starts_with("--");
            let optional = text.contains("?.");
            let keyword_follows = |keyword: &str| {
                after.strip_prefix(keyword).is_some_and(|rest| {
                    !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$')
                })
            };
            let object_rest = before
                .strip_suffix("...")
                .is_some_and(|head| innermost_open_bracket(head) == Some('{'));
            // `[...x = a] = b`: `checkArrayLiteralDestructuringAssignment`
            // reports the initializer's `=` instead (TS1186).
            let array_rest = before
                .strip_suffix("...")
                .is_some_and(|head| innermost_open_bracket(head) == Some('['));
            if array_rest
                && !is_update
                && let Some(offset) = text.char_indices().find_map(|(at, c)| {
                    (c == '='
                        && !matches!(text[at + 1..].chars().next(), Some('=' | '>'))
                        && !matches!(text[..at].chars().last(), Some('=' | '!' | '<' | '>')))
                    .then_some(at)
                })
            {
                let start = span.start + offset;
                return Some((1186, crate::TextSpan { start, end: start + 1 }));
            }
            // tsc words the message by the statement holding the target; the
            // for-in form assumes the type check before it (TS2405) passed.
            let code = match (is_update, optional) {
                (true, false) => 2357,
                (true, true) => 2777,
                (false, true) if keyword_follows("in") => 2780,
                (false, true) if keyword_follows("of") => 2781,
                (false, true) if object_rest => 2778,
                (false, true) => 2779,
                (false, false) if object_rest => 2701,
                (false, false) if keyword_follows("in") => 2406,
                (false, false) if keyword_follows("of") => 2487,
                (false, false) => 2364,
            };
            Some((code, span))
        }
        // tsc's parser expects `of` where `for await (x in y)` has `in`.
        FOR_AWAIT_IN_MESSAGE => {
            let open = span.start + source_text.get(span.start..)?.find('(')?;
            let bytes = source_text.as_bytes();
            let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
            let start = (open + 1..source_text.len().saturating_sub(1)).find(|&at| {
                &bytes[at..at + 2] == b"in"
                    && !is_word(bytes[at - 1])
                    && bytes.get(at + 2).is_none_or(|&next| !is_word(next))
            })?;
            Some((1005, crate::TextSpan { start, end: start + 2 }))
        }
        "A rest parameter must be last in a parameter list" => Some((1014, span)),
        "A rest parameter must be of an array type." => Some((2370, span)),
        // At the comma (`checkGrammarForDisallowedTrailingComma`).
        "A rest parameter or binding pattern may not have a trailing comma." => Some((1013, span)),
        "Identifier expected. 'this' is a reserved word that cannot be used here." => {
            let start = parameter_start(source_text, span.start)?;
            let head = source_text.get(start..span.start)?;
            let first_word = head.split(|c: char| !is_identifier_char(c)).next().unwrap_or("");
            if head.starts_with('@') || MODIFIER_KEYWORDS.contains(&first_word) {
                return Some((1433, crate::TextSpan { start, end: span.start }));
            }
            // A bare `this` after another parameter: tsc's `checkParameter`.
            let preceded_by_parameter = head.is_empty()
                && source_text[..start].trim_end().ends_with(',')
                && innermost_open_bracket(&source_text[..start]) == Some('(');
            preceded_by_parameter.then_some((2680, span))
        }
        // tsc parses a parameter-property modifier on a rest parameter and
        // rejects it as TS1317 over the whole parameter; oxc fails at the `...`.
        "Unexpected token" if text.starts_with("...") => {
            let start = parameter_property_modifiers_start(source_text, span.start)?;
            Some((1317, crate::TextSpan { start, end: span.end }))
        }
        // tsc reports the rest element at its name, past the `...`.
        "A rest element must be last in a destructuring pattern" => {
            let name = text.strip_prefix("...").map_or(text, str::trim_start);
            let start = span.end - name.len();
            Some((2462, crate::TextSpan { start, end: span.end }))
        }
        // The same failure in an assignment pattern, reported over the whole `...x`.
        "Spread must be last element" => Some((2462, span)),
        _ => None,
    }
}

/// The bracket that is open at the end of `text`: `{` for a target inside an
/// object literal. Brackets inside strings and comments are not skipped.
fn innermost_open_bracket(text: &str) -> Option<char> {
    let mut depth = 0usize;
    for ch in text.chars().rev() {
        match ch {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' => {
                if depth == 0 {
                    return Some(ch);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

const MODIFIER_KEYWORDS: &[&str] = &[
    "public", "private", "protected", "readonly", "override", "static", "abstract", "accessor",
    "declare", "async", "export", "default", "const", "in", "out",
];

const PARAMETER_PROPERTY_MODIFIERS: &[&str] =
    &["public", "private", "protected", "readonly", "override"];

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The first token of the parameter holding `position`: just past the `(` or
/// `,` that opens it.
fn parameter_start(source_text: &str, position: usize) -> Option<usize> {
    let head = source_text.get(..position)?;
    let mut depth = 0usize;
    for (index, ch) in head.char_indices().rev() {
        match ch {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' | '{' | ',' if depth == 0 => {
                if ch != '(' && ch != ',' {
                    return None;
                }
                let rest = &source_text[index + 1..];
                return Some(source_text.len() - rest.trim_start().len());
            }
            '(' | '[' | '{' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The start of the parameter-property modifiers written right before
/// `position`, when the parameter begins with them (no decorator).
fn parameter_property_modifiers_start(source_text: &str, position: usize) -> Option<usize> {
    let start = parameter_start(source_text, position)?;
    let mut words = source_text.get(start..position)?.split_whitespace().peekable();
    words.peek()?;
    words
        .all(|word| PARAMETER_PROPERTY_MODIFIERS.contains(&word))
        .then_some(start)
}

/// Where tsc's parser stops at a write target it cannot take, and with what.
/// tsc parses an assignment's target and a prefix `++`/`--` operand as a
/// left-hand-side expression only. A prefix operand that cannot begin one —
/// another update, a unary operator, `await` (`++ ++x`, `++await x`) — is
/// TS1109 `Expression expected` on its first token (`parsePrimaryExpression`).
/// An update expression that ends where the statement must end is TS1005
/// `';' expected` on the next token (`parseExpressionStatement`'s
/// `parseSemicolon`): the assignment operator after it, or the trailing
/// `++`/`--` of `--x--`. Parenthesized, the operand is a left-hand-side
/// expression and the checker's to judge.
fn update_target_stop(text: &str, span: crate::TextSpan, source_text: &str) -> Option<(u32, crate::TextSpan)> {
    let before = source_text.get(..span.start)?.trim_end();
    let under_prefix = before.ends_with("++") || before.ends_with("--");
    let missing_operand = |start: usize| {
        let end = super::grammar_context::first_token_end(source_text, start);
        Some((1109, crate::TextSpan { start, end }))
    };
    // oxc starts a prefix update nested in another at the outer operator.
    if let Some(rest) = text.strip_prefix("++").or_else(|| text.strip_prefix("--")) {
        let inner = rest.trim_start();
        if begins_no_left_hand_side(inner) {
            return missing_operand(span.start + (text.len() - inner.len()));
        }
    }
    if under_prefix && begins_no_left_hand_side(text) {
        return missing_operand(span.start);
    }
    let postfix = text.ends_with("++") || text.ends_with("--");
    let prefix = text.starts_with("++") || text.starts_with("--");
    if !postfix && !prefix {
        return None;
    }
    let after = source_text.get(span.end..)?.trim_start();
    let after_start = source_text.len() - after.len();
    let assignment = ASSIGNMENT_OPERATORS
        .iter()
        .find(|operator| after.starts_with(**operator))
        .filter(|operator| **operator != "=" || !matches!(after.as_bytes().get(1), Some(b'=' | b'>')));
    if let Some(operator) = assignment {
        return Some((1005, crate::TextSpan { start: after_start, end: after_start + operator.len() }));
    }
    (postfix && under_prefix).then(|| (1005, crate::TextSpan { start: span.end - 2, end: span.end }))
}

/// Whether `text` begins with a token no left-hand-side expression starts
/// with: an update or unary operator, or a keyword operator.
fn begins_no_left_hand_side(text: &str) -> bool {
    if text.starts_with(['+', '-', '!', '~']) {
        return true;
    }
    ["typeof", "void", "delete", "await", "yield"].iter().any(|keyword| {
        text.strip_prefix(keyword)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$'))
    })
}

const ASSIGNMENT_OPERATORS: [&str; 16] = [
    ">>>=", "**=", "<<=", ">>=", "&&=", "||=", "??=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "=",
];

/// The code tsc gives a failure oxc numbers differently. tsc's parser
/// reports a decorator or modifier on a `this` parameter as TS1433 at the
/// first of them, where oxc stops at the reserved word (after a TS1090 for
/// each modifier outside a constructor, which tsc does not report).
fn recode_parser_error(
    code: Option<u32>,
    span: Option<crate::TextSpan>,
    source_text: &str,
) -> Option<u32> {
    let Some(span) = span else {
        return code;
    };
    if code == Some(1090) {
        let mut after = source_text.get(span.end..).unwrap_or("");
        loop {
            after = after.trim_start();
            let word_len = after.find(|c: char| !is_identifier_char(c)).unwrap_or(after.len());
            let word = &after[..word_len];
            if word == "this" {
                return None;
            }
            if word.is_empty() {
                break;
            }
            after = &after[word_len..];
        }
    }
    code
}

/// Where tsc anchors a failure oxc labels elsewhere: the name after a
/// `const` class member modifier (TS1248), the `<` of an instantiation
/// expression (TS1477), the second of the `u`/`v` flags (TS1502), and the
/// first keyword of an ambient `using` or `await using` (TS1545/TS1546).
fn tsc_anchor_for_parser_error(
    code: Option<u32>,
    span: crate::TextSpan,
    source_text: &str,
) -> crate::TextSpan {
    let text = |start: usize, end: usize| source_text.get(start..end).unwrap_or("");
    let at = |start: usize, len: usize| crate::TextSpan { start, end: start + len };
    match code {
        Some(1248) => {
            let rest = text(span.end, source_text.len());
            let skipped = rest.len() - rest.trim_start().len();
            let start = span.end + skipped;
            let len = text(start, source_text.len())
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
                .unwrap_or(0);
            at(start, len)
        }
        Some(1477) => match text(span.start, span.end).find('<') {
            Some(offset) => at(span.start + offset, 1),
            None => span,
        },
        Some(1502) if span.end > span.start => at(span.end - 1, 1),
        Some(1545) => match text(0, span.start).rfind("using") {
            Some(start) => at(start, 5),
            None => span,
        },
        Some(1546) => match text(0, span.start).rfind("await") {
            Some(start) => at(start, 5),
            None => span,
        },
        _ => span,
    }
}

fn parse_source_in(allocator: &Allocator, source_text: &str, file_name: &str) -> ParsedSource {
    // A `.json` file holds a value, not a program. Handing it to the TypeScript
    // parser produces nothing usable (and a pile of syntax errors), so it takes
    // its own path and carries only the value's type.
    if super::is_json_file_name(file_name) {
        return ParsedSource {
            jsdoc_parse_errors: Vec::new(),
            file_name: file_name.to_string(),
            statements: Vec::new(),
            parser_errors: Vec::new(),
            parse_aborted: false,
            is_module: true,
            commonjs_module: false,
            reference_type_directives: Vec::new(),
            module_reads: Vec::new(),
            jsdoc_link_names: Vec::new(),
            definite_writes: Vec::new(),
            let_assignments: Vec::new(),
            comment_directives: Vec::new(),
            import_call_specifiers: Vec::new(),
            import_calls: Vec::new(),
            import_attributes: Vec::new(),
            grammar_diagnostics: Vec::new(),
            parenthesized_expressions: Vec::new(),
            global_this_starts: Vec::new(),
            literal_this_members: Vec::new(),
            // A `.json` file that does not parse still *is* a JSON module —
            // reporting its importer as unresolved would be a worse answer than
            // an unmodelled value, and surge does not report JSON syntax errors.
            json_module_type: Some(
                super::parse_json_module_type(source_text).unwrap_or(ParsedType::Unknown),
            ),
            jsx_factory_uses: Default::default(),
        };
    }

    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parser = Parser::new(allocator, source_text, source_type);
    let mut parsed = parser.parse();
    super::import_aliases::expand_import_aliases(allocator, &mut parsed.program);

    let reference_type_directives = super::extract_reference_type_directives(source_text);
    let comment_directives =
        super::suppressions::collect_comment_directives(source_text, &parsed.program.comments);

    let enum_values = std::rc::Rc::new(super::enum_values::evaluate_program(&parsed.program));
    let circular_annotations = std::rc::Rc::new(if is_declaration_file_name(file_name) {
        Vec::new()
    } else {
        super::grammar_context::circular_annotation_starts(&parsed.program)
    });
    let closure_resets = std::rc::Rc::new(if is_declaration_file_name(file_name) {
        Default::default()
    } else {
        super::closure_resets::collect_closure_resets(&parsed.program)
    });
    let collect_statements = || -> Vec<crate::ParsedStatement> {
        super::closure_resets::with_closure_resets(closure_resets.clone(), || {
        super::with_circular_annotations(circular_annotations.clone(), || {
        super::enum_values::with_enum_values(enum_values.clone(), || {
            super::private_names::with_file(file_name, || {
                let mut statements: Vec<crate::ParsedStatement> = parsed
                    .program
                    .body
                    .iter()
                    .filter_map(super::parse_statement)
                    .flatten()
                    .collect();
                super::enums::merge_lowered_enum_declarations(&mut statements);
                statements
            })
        })
        })
        })
    };

    // Declaration files never participate in noUnusedLocals, and `declare`
    // functions carry no body to index, so skip the read walk for them entirely
    // (it would otherwise run over every dependency `.d.ts`). The conversion
    // still asks each body for its reads; without an index those calls fall back
    // to walking the body, which for a `.d.ts` is nothing.
    let javascript = is_javascript_file_name(file_name);
    let commonjs = javascript.then(|| super::commonjs::scan(&parsed.program));
    let jsdoc_index = javascript
        .then(|| std::rc::Rc::new(super::jsdoc::build_jsdoc_index(&parsed.program, source_text)));
    let mut commonjs_findings = Vec::new();
    let mut commonjs_own_specifier = None;
    let (mut module_reads, statements) = if is_declaration_file_name(file_name) {
        (Vec::new(), collect_statements())
    } else {
        super::spans::with_lowering_source(source_text, javascript, || {
            super::commonjs::with_commonjs(commonjs.clone(), || {
                super::jsdoc::with_jsdoc_index(jsdoc_index.clone(), || {
                    let (reads, mut statements) =
                        super::reads::with_body_read_index(&parsed.program, collect_statements);
                    if let Some(commonjs) = &commonjs {
                        // An `@import` binds its names in the file, which surge
                        // does only for a module.
                        statements.splice(0..0, super::jsdoc::import_statements());
                        let is_module = commonjs.module
                            || parsed.program.source_type.is_module()
                            || statements.iter().any(|statement| {
                                matches!(
                                    statement,
                                    crate::ParsedStatement::ImportDeclaration(_)
                                        | crate::ParsedStatement::ExportDeclaration(_)
                                )
                            });
                        // With `module.exports` replaced, a typedef is a member
                        // of that export (`bindCommonJSTypeExports`), not an
                        // export beside it.
                        let exported = is_module && !commonjs.exports_assigned();
                        statements.extend(super::jsdoc::alias_statements(exported));
                        let (variables, own_specifier) =
                            super::commonjs::module_variables(&parsed.program, commonjs, file_name);
                        statements.extend(variables);
                        commonjs_own_specifier = own_specifier;
                        commonjs_findings = super::commonjs::take_findings();
                    }
                    (reads, statements)
                })
            })
        })
    };
    let jsdoc_link_names = jsdoc_link_reads(&parsed.program.comments, source_text);
    module_reads.extend(jsdoc_link_names.iter().cloned());

    let mut parser_errors: Vec<crate::ParserError> = parsed
        .errors
        .into_iter()
        .filter_map(|error| {
            let code = error
                .code
                .scope
                .as_deref()
                .filter(|scope| *scope == "TS")
                .and(error.code.number.as_deref())
                .and_then(|number| number.parse::<u32>().ok());
            let span = error
                .labels
                .as_ref()
                .and_then(|labels| labels.first())
                .map(|label| crate::TextSpan {
                    start: label.offset(),
                    end: label.offset() + label.len(),
                });
            let (code, span) = match (code, span) {
                (None, Some(span)) => match classify_uncoded_parser_error(&error.to_string(), span, source_text) {
                    Some((code, span)) => (Some(code), Some(span)),
                    None => (None, Some(span)),
                },
                other => other,
            };
            let span = span.map(|span| tsc_anchor_for_parser_error(code, span, source_text));
            let code = recode_parser_error(code, span, source_text);
            let span_text = span
                .and_then(|span| source_text.get(span.start..span.end))
                .map(str::to_string);
            let mut message = error.to_string();
            // The checker reports an update expression's rejected operand once
            // it passes the arithmetic check (`ParsedExpression::Update`).
            if matches!(code, Some(2357 | 2777)) && message == "Cannot assign to this expression" {
                return None;
            }
            // tsc reports this (TS1497) only for a decorator its checker checks, which the
            // lowering records (`ParsedDecorator::needs_parentheses`).
            if message == "Expression must be enclosed in parentheses to be used as a decorator." {
                return None;
            }
            if message == FOR_AWAIT_IN_MESSAGE {
                message = "'of' expected.".to_string();
            } else if code == Some(1005) && message == "Cannot assign to this expression" {
                message = "';' expected.".to_string();
            } else if code == Some(1109) && message == "Cannot assign to this expression" {
                message = "Expression expected.".to_string();
            }
            Some(crate::ParserError { code, message, span, span_text })
        })
        .collect();
    parser_errors.extend(super::scanner_checks::collect_missing_parser_errors(
        &parsed.program,
        source_text,
    ));
    let jsdoc_parse_errors: Vec<crate::ParserError> = jsdoc_index
        .as_ref()
        .map(|index| {
            super::jsdoc::reparse_errors(index)
                .into_iter()
                .map(|(code, span)| crate::ParserError {
                    code: Some(code),
                    message: "Identifier expected.".to_string(),
                    span: Some(span),
                    span_text: source_text.get(span.start..span.end).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();

    let is_module = parsed.program.source_type.is_module()
        || commonjs.as_ref().is_some_and(|commonjs| commonjs.module)
        || statements.iter().any(|statement| {
            matches!(
                statement,
                crate::ParsedStatement::ImportDeclaration(_)
                    | crate::ParsedStatement::ExportDeclaration(_)
            )
        })
        || super::grammar_context::contains_import_meta(&parsed.program);

    let (mut import_call_specifiers, import_calls) =
        super::import_calls::collect_import_call_specifiers(&parsed.program, source_text, javascript);
    // `module` and `exports` read the module's own export through its own
    // specifier, which resolves as any other import type's.
    if let Some(specifier) = commonjs_own_specifier
        && !import_call_specifiers.contains(&specifier)
    {
        import_call_specifiers.push(specifier);
    }
    if let Some(index) = &jsdoc_index {
        for specifier in super::jsdoc::import_type_specifiers(index) {
            if !import_call_specifiers.contains(specifier) {
                import_call_specifiers.push(specifier.clone());
            }
        }
    }

    // A declaration file gets the grammar findings whose answer is that every
    // node in it is ambient (which `skipLibCheck` then suppresses).
    let (grammar_diagnostics, parenthesized_expressions) = if collects_grammar_diagnostics(file_name) {
        super::enum_values::with_enum_values(enum_values.clone(), || {
            super::grammar::collect_grammar_diagnostics(&parsed.program)
        })
    } else if javascript {
        // A JavaScript file's are reported only when it is checked (`checkJs`),
        // its JSDoc's parse errors with them (`JSDocDiagnostics`).
        super::commonjs::with_commonjs(commonjs.clone(), || {
            super::jsdoc::with_jsdoc_index(jsdoc_index.clone(), || {
                let (mut diagnostics, parenthesized) =
                    super::grammar::collect_grammar_diagnostics(&parsed.program);
                diagnostics.extend(super::jsdoc::diagnostics());
                diagnostics.append(&mut commonjs_findings);
                (diagnostics, parenthesized)
            })
        })
    } else if is_declaration_file_name(file_name) {
        let mut diagnostics = Vec::new();
        super::grammar_modifiers::collect_declaration_file_diagnostics(&parsed.program, &mut diagnostics);
        // surge checks only the declaration files a project writes itself; a
        // package's and a bundled default library's stay unchecked, as under
        // `skipLibCheck`.
        if !file_name
            .split(['/', '\\'])
            .any(|segment| segment == "node_modules" || segment == "generated-libs")
        {
            super::grammar::collect_declaration_file_implicit_any(&parsed.program, &mut diagnostics);
        }
        super::grammar_recovered::collect_recovered_grammar_diagnostics(&parsed.program, true, &mut diagnostics);
        (diagnostics, Vec::new())
    } else {
        (Vec::new(), Vec::new())
    };
    let mut grammar_diagnostics = grammar_diagnostics;
    // What the recovering parse rejects that tsc's parser accepts and its
    // checker's grammar checks reject: reported whether or not tsc's parser
    // finds the file clean. The parse drops some of it from the tree
    // (decorators before a declaration that cannot take them), so the grammar
    // walk cannot see it.
    let collects = collects_grammar_diagnostics(file_name) || javascript;
    let examined_modifiers = super::grammar_context::take_examined_modifier_starts();
    // A CommonJS module has no `ExternalModuleIndicator` either, so its
    // top-level `this` is `globalThis` as a script's is (`tryGetThisTypeAtEx`).
    let global_this_starts = super::grammar_context::take_global_this_starts();
    let literal_this_members = super::grammar_context::take_literal_this_members();
    parser_errors.retain(|error| {
        if error.code == Some(1029)
            && error.span.is_some_and(|span| examined_modifiers.contains(&(span.start as u32)))
        {
            return false;
        }
        let Some(code) = checker_grammar_code(error) else {
            return true;
        };
        if collects && let Some(span) = error.span {
            let bare_import = error.message.contains("type arguments and no call");
            grammar_diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(code),
                span,
                name: bare_import.then(|| "bare".to_string()),
            });
        }
        false
    });

    let let_assignments =
        super::let_assignments::collect_let_assignments(&parsed.program, source_text);
    let jsx_factory_uses = if source_type.is_jsx() {
        super::jsx_uses::collect_jsx_factory_uses(&parsed.program, source_text)
    } else {
        Default::default()
    };
    ParsedSource {
        jsdoc_parse_errors,
        file_name: file_name.to_string(),
        statements,
        parser_errors,
        parse_aborted: parsed.panicked,
        is_module,
        commonjs_module: commonjs.as_ref().is_some_and(|commonjs| commonjs.module),
        reference_type_directives,
        module_reads,
        jsdoc_link_names,
        definite_writes: super::writes::collect_definite_writes(&parsed.program),
        let_assignments,
        comment_directives,
        import_call_specifiers,
        import_calls,
        import_attributes: super::import_attributes::collect_import_attributes(&parsed.program, source_text),
        grammar_diagnostics,
        parenthesized_expressions,
        global_this_starts,
        literal_this_members,
        json_module_type: None,
        jsx_factory_uses,
    }
}

/// tsc's `IsDeclarationFileName`: a `.d.ts`/`.d.mts`/`.d.cts` file, or a `.ts`
/// file whose base name carries `.d.` — the `{name}.d.{extension}.ts` form
/// `allowArbitraryExtensions` resolves `{name}.{extension}` imports to.
pub fn is_declaration_file_name(file_name: &str) -> bool {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let ends_with = |suffix: &str| {
        let bytes = base.as_bytes();
        bytes.len() >= suffix.len() && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
    };
    ends_with(".d.ts") || ends_with(".d.mts") || ends_with(".d.cts") || (base.ends_with(".ts") && base.contains(".d."))
}

/// The names JSDoc `{@link X}`, `{@linkcode X}` and `{@linkplain X}` tags
/// refer to (the first identifier of an entity name): tsc resolves each
/// (`checkJSDocLinkLikeTag`), which counts as a use of the import it names.
fn jsdoc_link_reads(comments: &[oxc_ast::Comment], source_text: &str) -> Vec<String> {
    let mut reads = Vec::new();
    for comment in comments.iter().filter(|comment| comment.is_jsdoc()) {
        let Some(mut rest) = source_text.get(comment.span.start as usize..comment.span.end as usize)
        else {
            continue;
        };
        while let Some(at) = rest.find("{@link") {
            rest = &rest[at + "{@link".len()..];
            let after_tag = rest
                .strip_prefix("code")
                .or_else(|| rest.strip_prefix("plain"))
                .unwrap_or(rest);
            if !after_tag.starts_with(char::is_whitespace) {
                continue;
            }
            let name: String = after_tag
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            if !name.is_empty() {
                reads.push(name);
            }
        }
    }
    reads
}

/// A JavaScript source file: `.js`, `.jsx`, `.mjs` or `.cjs`.
pub fn is_javascript_file_name(file_name: &str) -> bool {
    let bytes = file_name.as_bytes();
    [".js", ".jsx", ".mjs", ".cjs"].iter().any(|suffix| {
        bytes.len() >= suffix.len()
            && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
    })
}

fn collects_grammar_diagnostics(file_name: &str) -> bool {
    if is_declaration_file_name(file_name) {
        return false;
    }
    [".ts", ".tsx", ".mts", ".cts"]
        .iter()
        .any(|extension| file_name.ends_with(extension))
}
