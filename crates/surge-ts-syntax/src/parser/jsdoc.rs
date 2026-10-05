//! JSDoc in JavaScript files. A JavaScript file writes its types in JSDoc
//! comments; typescript-go parses those (`parser/jsdoc.go`) and reparses the
//! tags into the annotations, type parameters and type aliases a TypeScript
//! file writes directly (`parser/reparser.go`). This module is that pair: the
//! tag parser, and a pass over the program that records what each
//! declaration's JSDoc contributes, which the lowering reads wherever the
//! source has no annotation of its own.

use std::cell::RefCell;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Class, Comment, Expression,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExpressionStatement, FormalParameter,
    FormalParameters, Function, MethodDefinition, MethodDefinitionKind, ObjectProperty,
    ParenthesizedExpression, Program, PropertyDefinition, ReturnStatement, TSType,
    TSTypeName, VariableDeclaration, VariableDeclarator,
};
use oxc_ast::AstBuilder;
use oxc_ast_visit::{walk, walk_mut, Visit, VisitMut};
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::identifier::{is_identifier_part, is_identifier_start};
use oxc_syntax::scope::ScopeFlags;

use crate::{
    ParsedExportDeclaration, ParsedFunctionType, ParsedFunctionTypeParameter, ParsedObjectType,
    ParsedObjectTypeProperty, ParsedStatement, ParsedType, ParsedTypeAliasDeclaration,
    ParsedTypeParameter, TextSpan,
};

// ---------------------------------------------------------------------------
// Scanner: typescript-go's `ScanJSDocToken` / `ScanJSDocCommentTextToken`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    EndOfFile,
    Whitespace,
    NewLine,
    At,
    Asterisk,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    OpenParen,
    CloseParen,
    LessThan,
    GreaterThan,
    Equals,
    Comma,
    Dot,
    Backtick,
    Hash,
    Identifier,
    CommentText,
    Unknown,
}

fn is_line_break(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// tsgo's `peekUnicodeEscape`: the character `\\uXXXX` or `\\u{X…}` at
/// `pos` names, and the escape's length.
fn unicode_escape_at(text: &str, pos: usize) -> Option<(char, usize)> {
    let rest = text.get(pos..)?.strip_prefix("\\u")?;
    let (digits, length) = match rest.strip_prefix('{') {
        Some(braced) => {
            let close = braced.find('}')?;
            (&braced[..close], 2 + 1 + close + 1)
        }
        None => (rest.get(..4)?, 2 + 4),
    };
    if digits.is_empty() || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let ch = char::from_u32(u32::from_str_radix(digits, 16).ok()?)?;
    Some((ch, length))
}

fn is_white_space_single_line(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\u{b}'
            | '\u{c}'
            | '\u{a0}'
            | '\u{85}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200b}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

#[derive(Clone, Copy)]
struct ScannerState {
    pos: usize,
    token: Token,
    token_start: usize,
    full_start: usize,
}

struct Scanner<'s> {
    /// The file's text up to the comment's closing `*/`.
    text: &'s str,
    pos: usize,
    token: Token,
    token_start: usize,
    full_start: usize,
}

impl<'s> Scanner<'s> {
    fn char_at(&self, pos: usize) -> Option<char> {
        self.text.get(pos..)?.chars().next()
    }

    fn token_text(&self) -> &'s str {
        &self.text[self.token_start..self.pos]
    }

    /// The identifier the current token spells, its unicode escapes decoded
    /// (tsgo's `tokenValue`).
    fn token_value(&self) -> std::borrow::Cow<'s, str> {
        let text = self.token_text();
        if !text.contains('\\') {
            return std::borrow::Cow::Borrowed(text);
        }
        let mut value = String::with_capacity(text.len());
        let mut pos = 0;
        while pos < text.len() {
            if let Some((ch, length)) = unicode_escape_at(text, pos) {
                value.push(ch);
                pos += length;
            } else {
                let ch = text[pos..].chars().next().unwrap_or_default();
                value.push(ch);
                pos += ch.len_utf8().max(1);
            }
        }
        std::borrow::Cow::Owned(value)
    }

    /// tsgo's `scanIdentifierParts` from `self.pos`: identifier characters
    /// and the unicode escapes of identifier characters.
    fn scan_identifier_parts(&mut self, allow_dash: bool) {
        while let Some(next) = self.char_at(self.pos) {
            if is_identifier_part(next) || (allow_dash && next == '-') {
                self.pos += next.len_utf8();
                continue;
            }
            if next == '\\'
                && let Some((escaped, length)) = unicode_escape_at(self.text, self.pos)
                && is_identifier_part(escaped)
            {
                self.pos += length;
                continue;
            }
            break;
        }
    }

    fn mark(&self) -> ScannerState {
        ScannerState {
            pos: self.pos,
            token: self.token,
            token_start: self.token_start,
            full_start: self.full_start,
        }
    }

    fn rewind(&mut self, state: ScannerState) {
        self.pos = state.pos;
        self.token = state.token;
        self.token_start = state.token_start;
        self.full_start = state.full_start;
    }

    fn scan_jsdoc_token(&mut self) -> Token {
        self.full_start = self.pos;
        self.token_start = self.pos;
        let Some(ch) = self.char_at(self.pos) else {
            self.token = Token::EndOfFile;
            return self.token;
        };
        self.pos += ch.len_utf8();
        self.token = match ch {
            '\t' | '\u{b}' | '\u{c}' | ' ' => {
                while let Some(next) = self.char_at(self.pos) {
                    if !is_white_space_single_line(next) {
                        break;
                    }
                    self.pos += next.len_utf8();
                }
                Token::Whitespace
            }
            '@' => Token::At,
            '\r' => {
                if self.char_at(self.pos) == Some('\n') {
                    self.pos += 1;
                }
                Token::NewLine
            }
            '\n' => Token::NewLine,
            '*' => Token::Asterisk,
            '{' => Token::OpenBrace,
            '}' => Token::CloseBrace,
            '[' => Token::OpenBracket,
            ']' => Token::CloseBracket,
            '(' => Token::OpenParen,
            ')' => Token::CloseParen,
            '<' => Token::LessThan,
            '>' => Token::GreaterThan,
            '=' => Token::Equals,
            ',' => Token::Comma,
            '.' => Token::Dot,
            '`' => Token::Backtick,
            '#' => Token::Hash,
            ch if is_identifier_start(ch) => {
                while let Some(next) = self.char_at(self.pos) {
                    if !is_identifier_part(next) && next != '-' {
                        break;
                    }
                    self.pos += next.len_utf8();
                }
                if self.char_at(self.pos) == Some('\\') {
                    self.scan_identifier_parts(false);
                }
                Token::Identifier
            }
            '\\' => match unicode_escape_at(self.text, self.token_start) {
                Some((escaped, length)) if is_identifier_start(escaped) => {
                    self.pos = self.token_start + length;
                    self.scan_identifier_parts(false);
                    Token::Identifier
                }
                _ => Token::Unknown,
            },
            _ => Token::Unknown,
        };
        self.token
    }

    fn scan_comment_text_token(&mut self, in_backticks: bool) -> Token {
        self.full_start = self.pos;
        if self.pos >= self.text.len() {
            self.token_start = self.pos;
            self.token = Token::EndOfFile;
            return self.token;
        }
        self.token_start = self.pos;
        while let Some(ch) = self.char_at(self.pos) {
            if is_line_break(ch) || ch == '`' {
                break;
            }
            if !in_backticks {
                if ch == '{' {
                    break;
                }
                if ch == '@' {
                    let previous = self.text[..self.pos].chars().next_back();
                    let next = self.char_at(self.pos + 1);
                    if previous.is_some_and(is_white_space_single_line)
                        && next.is_some_and(is_identifier_start)
                    {
                        break;
                    }
                }
            }
            self.pos += ch.len_utf8();
        }
        if self.pos == self.token_start {
            return self.scan_jsdoc_token();
        }
        self.token = Token::CommentText;
        self.token
    }

    /// Whether a tag can follow the `@` just scanned: an identifier start, or
    /// whitespace, a line break or the end for an incomplete tag.
    fn can_follow_at(&self) -> bool {
        match self.char_at(self.pos) {
            None => true,
            Some(ch) => is_identifier_start(ch) || is_white_space_single_line(ch) || is_line_break(ch),
        }
    }
}

// ---------------------------------------------------------------------------
// Tags.

/// A type written in a tag, lowered. `ty` is `None` when the text did not
/// parse as a type.
#[derive(Clone, Debug)]
pub(crate) struct JsDocType {
    pub(crate) ty: Option<ParsedType>,
    pub(crate) span: TextSpan,
    /// `...T`: a rest parameter's element type.
    pub(crate) variadic: bool,
    /// `T=`: an optional parameter.
    pub(crate) optional: bool,
    /// `Object`, `object` or an array of either, which nested `@param` or
    /// `@property` tags can fill in (`isObjectOrObjectArrayTypeReference`).
    pub(crate) object_like: bool,
    pub(crate) is_array: bool,
}

/// tsgo's `JSDocTypeLiteral`: the properties nested `@param opts.x` or
/// `@property` tags give an `Object` type.
#[derive(Clone, Debug)]
pub(crate) struct JsDocTypeLiteral {
    properties: Vec<JsDocParameterTag>,
    is_array: bool,
    span: TextSpan,
}

#[derive(Clone, Debug)]
pub(crate) enum JsDocTypeExpression {
    Type(JsDocType),
    Literal(JsDocTypeLiteral),
}

#[derive(Clone, Debug)]
pub(crate) struct JsDocParameterTag {
    /// Where the tag's `@` is.
    start: usize,
    /// The dotted name as written (`opts.name` is `["opts", "name"]`).
    name: Vec<String>,
    name_span: TextSpan,
    bracketed: bool,
    type_expression: Option<JsDocTypeExpression>,
    /// Written `@param name {T}` or without a type (`IsNameFirst`).
    name_first: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct JsDocTemplateParameter {
    name: String,
    name_span: TextSpan,
    default_type: Option<JsDocType>,
    is_const: bool,
    span: TextSpan,
}

#[derive(Clone, Debug)]
pub(crate) struct JsDocSignature {
    parameters: Vec<JsDocParameterTag>,
    this_type: Option<JsDocType>,
    return_type: Option<JsDocType>,
}

#[derive(Clone, Debug)]
pub(crate) enum JsDocTag {
    Parameter(JsDocParameterTag),
    Property(JsDocParameterTag),
    Return(Option<JsDocType>),
    Type(Option<JsDocType>),
    This(Option<JsDocType>),
    Satisfies(Option<JsDocType>),
    Template {
        constraint: Option<JsDocType>,
        parameters: Vec<JsDocTemplateParameter>,
    },
    Typedef {
        name: Vec<String>,
        name_span: TextSpan,
        type_expression: Option<JsDocTypeExpression>,
    },
    Callback {
        name: Vec<String>,
        name_span: TextSpan,
        signature: JsDocSignature,
        span: TextSpan,
    },
    /// `@overload` with the signature its following tags write, at the tag
    /// name.
    Overload {
        signature: JsDocSignature,
        span: TextSpan,
    },
    /// `@augments`/`@extends`: the base class with its type arguments.
    Augments(Option<JsDocType>),
    /// `@implements`: an interface the class implements.
    Implements(Option<JsDocType>),
    /// `@import`: a type-only import declaration.
    Import(crate::ParsedImportDeclaration),
    /// `@public`, `@private`, `@protected`, `@readonly`, `@override`: the
    /// modifier, at the tag.
    Modifier(JsDocModifier, TextSpan),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsDocModifier {
    Public,
    Private,
    Protected,
    Readonly,
    Override,
}

#[derive(Clone, Debug)]
pub(crate) struct JsDocComment {
    pub(crate) tags: Vec<JsDocTag>,
    /// The parse errors tsgo reports as the file's `JSDocDiagnostics`.
    diagnostics: Vec<crate::ParsedGrammarDiagnostic>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    BeginningOfLine,
    SawAsterisk,
    SavingComments,
    SavingBackticks,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PropertyLikeParse {
    Property = 1,
    Parameter = 2,
    CallbackParameter = 4,
}

fn is_jsdoc_like(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 4 && bytes[1] == b'*' && bytes[2] == b'*' && bytes[3] != b'/'
}

/// Parse the JSDoc comment spanning `start..end` (delimiters included).
pub(crate) fn parse_jsdoc_comment(source_text: &str, start: usize, end: usize) -> Option<JsDocComment> {
    if end < start + 4 || !is_jsdoc_like(source_text.get(start..end)?) {
        return None;
    }
    let text = source_text.get(..end - 2)?;
    let line_start = source_text[..start].rfind('\n').map_or(0, |index| index + 1);
    let initial_indent = (start + 4 - line_start) as isize;
    let mut parser = TagParser {
        s: Scanner {
            text,
            pos: start + 3,
            token: Token::EndOfFile,
            token_start: start + 3,
            full_start: start + 3,
        },
        diagnostics: Vec::new(),
        child_tag_name_span: None,
    };
    let tags = parser.parse_comment_worker(initial_indent);
    Some(JsDocComment { tags, diagnostics: parser.diagnostics })
}

struct TagParser<'s> {
    s: Scanner<'s>,
    diagnostics: Vec<crate::ParsedGrammarDiagnostic>,
    /// The name of the child tag [`TagParser::try_parse_child_tag`] parsed
    /// last, where tsc reports a child that may not follow its parent.
    child_tag_name_span: Option<TextSpan>,
}

impl<'s> TagParser<'s> {
    fn token(&self) -> Token {
        self.s.token
    }

    /// `parseErrorAtRange` over the last child tag's name: a `@template`
    /// under a `@typedef`, `@callback` or `@overload` (TS8039).
    fn report_misplaced_template(&mut self) {
        if let Some(span) = self.child_tag_name_span {
            self.diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(8039),
                span,
                name: None,
            });
        }
    }

    /// `parseErrorAtCurrentToken`.
    fn error_at_current_token(&mut self, code: u32, argument: Option<&str>) {
        let span = TextSpan { start: self.s.token_start, end: self.s.pos.max(self.s.token_start) };
        if self.diagnostics.last().is_some_and(|last| last.span.start == span.start) {
            return;
        }
        self.diagnostics.push(crate::ParsedGrammarDiagnostic {
            kind: crate::ParsedGrammarDiagnosticKind::Ts(code),
            span,
            name: argument.map(str::to_string),
        });
    }

    fn next_token_jsdoc(&mut self) -> Token {
        self.s.scan_jsdoc_token()
    }

    fn next_comment_text_token(&mut self, in_backticks: bool) -> Token {
        self.s.scan_comment_text_token(in_backticks)
    }

    /// The regular scanner, which tsgo uses between some JSDoc tokens: trivia
    /// is skipped.
    fn next_token(&mut self) -> Token {
        loop {
            let token = self.s.scan_jsdoc_token();
            if !matches!(token, Token::Whitespace | Token::NewLine) {
                return token;
            }
        }
    }

    fn node_pos(&self) -> usize {
        self.s.full_start
    }

    fn parse_optional_jsdoc(&mut self, token: Token) -> bool {
        if self.token() == token {
            self.next_token_jsdoc();
            return true;
        }
        false
    }

    fn parse_comment_worker(&mut self, mut indent: isize) -> Vec<JsDocTag> {
        let mut tags = Vec::new();
        let mut state = State::SawAsterisk;
        let mut backtick_count = 0;
        let mut in_fenced_code_block = false;
        let mut margin: isize = -1;

        self.next_token_jsdoc();
        while self.parse_optional_jsdoc(Token::Whitespace) {}
        if self.parse_optional_jsdoc(Token::NewLine) {
            state = State::BeginningOfLine;
            indent = 0;
        }
        loop {
            if self.token() != Token::Backtick && backtick_count > 0 {
                if backtick_count >= 3 {
                    in_fenced_code_block = !in_fenced_code_block;
                }
                backtick_count = 0;
            }
            let saving = if in_fenced_code_block { State::SavingBackticks } else { State::SavingComments };
            let token_len = self.s.token_text().len() as isize;
            let push_comment = |margin: &mut isize, indent: &mut isize| {
                if *margin == -1 {
                    *margin = *indent;
                }
                *indent += token_len;
            };
            match self.token() {
                Token::At => {
                    if in_fenced_code_block || !self.s.can_follow_at() {
                        state = saving;
                        push_comment(&mut margin, &mut indent);
                    } else {
                        let tag = self.parse_tag(indent);
                        tags.push(tag);
                        state = State::BeginningOfLine;
                        margin = -1;
                    }
                }
                Token::NewLine => {
                    state = State::BeginningOfLine;
                    indent = 0;
                }
                Token::Asterisk => {
                    if state == State::SawAsterisk {
                        state = State::SavingComments;
                        push_comment(&mut margin, &mut indent);
                    } else {
                        state = State::SawAsterisk;
                        indent += token_len;
                    }
                }
                Token::Whitespace => indent += token_len,
                Token::EndOfFile => break,
                Token::CommentText => {
                    if state != State::SavingBackticks {
                        state = saving;
                    }
                    push_comment(&mut margin, &mut indent);
                }
                Token::Backtick => {
                    backtick_count += 1;
                    state = if state == State::SavingBackticks {
                        State::SavingComments
                    } else {
                        State::SavingBackticks
                    };
                    push_comment(&mut margin, &mut indent);
                }
                Token::OpenBrace if in_fenced_code_block => {
                    state = State::SavingBackticks;
                    push_comment(&mut margin, &mut indent);
                }
                Token::OpenBrace => {
                    state = State::SavingComments;
                    if !self.parse_jsdoc_link() {
                        push_comment(&mut margin, &mut indent);
                    }
                }
                _ => {
                    if state != State::SavingBackticks {
                        state = saving;
                    }
                    push_comment(&mut margin, &mut indent);
                }
            }
            if matches!(state, State::SavingComments | State::SavingBackticks) {
                self.next_comment_text_token(state == State::SavingBackticks);
            } else {
                self.next_token_jsdoc();
            }
        }
        tags
    }

    fn is_next_nonwhitespace_token_end_of_file(&mut self) -> bool {
        let state = self.s.mark();
        let result = loop {
            let token = self.next_token_jsdoc();
            if token == Token::EndOfFile {
                break true;
            }
            if !matches!(token, Token::Whitespace | Token::NewLine) {
                break false;
            }
        };
        self.s.rewind(state);
        result
    }

    fn skip_whitespace(&mut self) {
        if matches!(self.token(), Token::Whitespace | Token::NewLine)
            && self.is_next_nonwhitespace_token_end_of_file()
        {
            return;
        }
        while matches!(self.token(), Token::Whitespace | Token::NewLine) {
            self.next_token_jsdoc();
        }
    }

    fn skip_whitespace_or_asterisk(&mut self) -> String {
        if matches!(self.token(), Token::Whitespace | Token::NewLine)
            && self.is_next_nonwhitespace_token_end_of_file()
        {
            return String::new();
        }
        let mut preceding_line_break = self.token() == Token::NewLine;
        let mut seen_line_break = false;
        let mut indents = String::new();
        while (preceding_line_break && self.token() == Token::Asterisk)
            || matches!(self.token(), Token::Whitespace | Token::NewLine)
        {
            indents.push_str(self.s.token_text());
            if self.token() == Token::NewLine {
                preceding_line_break = true;
                seen_line_break = true;
                indents.clear();
            } else if self.token() == Token::Asterisk {
                preceding_line_break = false;
            }
            self.next_token_jsdoc();
        }
        if seen_line_break { indents } else { String::new() }
    }

    /// `{@link X}`, `{@linkcode X}` or `{@linkplain X}`, consumed up to its
    /// closing brace; `false` (nothing consumed) for any other brace.
    fn parse_jsdoc_link(&mut self) -> bool {
        let state = self.s.mark();
        let is_link = self.token() == Token::OpenBrace
            && self.next_token_jsdoc() == Token::At
            && self.next_token_jsdoc() == Token::Identifier
            && matches!(self.s.token_text(), "link" | "linkcode" | "linkplain");
        if !is_link {
            self.s.rewind(state);
            return false;
        }
        self.next_token_jsdoc();
        while !matches!(self.token(), Token::CloseBrace | Token::NewLine | Token::EndOfFile) {
            self.next_token_jsdoc();
        }
        true
    }

    fn parse_jsdoc_identifier_name(&mut self) -> Option<(String, TextSpan)> {
        if self.token() != Token::Identifier {
            return None;
        }
        let span = TextSpan { start: self.s.token_start, end: self.s.pos };
        let text = self.s.token_value().into_owned();
        self.next_token_jsdoc();
        Some((text, span))
    }

    fn parse_tag(&mut self, margin: isize) -> JsDocTag {
        let start = self.s.token_start;
        self.next_token_jsdoc();
        let tag_name = self.parse_jsdoc_identifier_name().map(|(name, _)| name).unwrap_or_default();
        let indent_text = self.skip_whitespace_or_asterisk();
        match tag_name.as_str() {
            "this" => {
                let ty = self.parse_jsdoc_type_expression(true);
                self.skip_whitespace();
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::This(ty)
            }
            "arg" | "argument" | "param" => {
                match self.parse_parameter_or_property_tag(start, PropertyLikeParse::Parameter, margin) {
                    Some(tag) => JsDocTag::Parameter(tag),
                    None => JsDocTag::Other,
                }
            }
            "return" | "returns" => {
                let ty = self.try_parse_type_expression();
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Return(ty)
            }
            "template" => self.parse_template_tag(start, margin, &indent_text),
            "type" => {
                let ty = self.parse_jsdoc_type_expression(true);
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Type(ty)
            }
            "typedef" => self.parse_typedef_tag(start, margin, &indent_text),
            "callback" => self.parse_callback_tag(start, margin, &indent_text),
            "overload" => {
                self.skip_whitespace();
                let commented = self.parse_tag_comments(margin, None);
                let signature = self.parse_jsdoc_signature(margin);
                if !commented {
                    self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                }
                let name_start = start + 1;
                JsDocTag::Overload {
                    signature,
                    span: TextSpan { start: name_start, end: name_start + "overload".len() },
                }
            }
            "satisfies" => {
                let ty = self.parse_jsdoc_type_expression(false);
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Satisfies(ty)
            }
            "augments" | "extends" => {
                // `parseExpressionWithTypeArgumentsForAugments`: an entity
                // name with type arguments, which reads as a type reference.
                let ty = self.parse_jsdoc_type_expression(true);
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Augments(ty)
            }
            // `parseImplementsTag` reads its class the same way.
            "implements" => {
                let ty = self.parse_jsdoc_type_expression(true);
                if ty.as_ref().is_none_or(|ty| ty.ty.is_none()) {
                    self.error_at_current_token(1003, None);
                }
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Implements(ty)
            }
            "public" | "private" | "protected" | "readonly" | "override" => {
                let modifier = match tag_name.as_str() {
                    "public" => JsDocModifier::Public,
                    "private" => JsDocModifier::Private,
                    "protected" => JsDocModifier::Protected,
                    "readonly" => JsDocModifier::Readonly,
                    _ => JsDocModifier::Override,
                };
                let span = TextSpan { start, end: self.s.token_start.max(start + 1) };
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Modifier(modifier, span)
            }
            "import" => {
                let import = self.parse_import_tag(start);
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                import.map_or(JsDocTag::Other, JsDocTag::Import)
            }
            "exception" | "throws" => {
                let _ = self.try_parse_type_expression();
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Other
            }
            _ => {
                self.parse_trailing_tag_comments(start, self.node_pos(), margin, &indent_text);
                JsDocTag::Other
            }
        }
    }

    fn parse_trailing_tag_comments(&mut self, pos: usize, end: usize, mut margin: isize, indent_text: &str) -> bool {
        if indent_text.is_empty() {
            margin += end as isize - pos as isize;
        }
        let initial_margin = if margin >= 0 && (margin as usize) < indent_text.len() {
            indent_text.get(margin as usize..).unwrap_or_default().to_string()
        } else {
            String::new()
        };
        self.parse_tag_comments(margin.max(0), Some(&initial_margin))
    }

    /// tsgo's `parseTagComments`, reading past a tag's comment up to the next
    /// tag. Returns whether any comment text was read.
    fn parse_tag_comments(&mut self, mut indent: isize, initial_margin: Option<&str>) -> bool {
        let mut state = State::BeginningOfLine;
        let mut backtick_count = 0;
        let mut in_fenced_code_block = false;
        let mut margin: isize = -1;
        let mut saw_text = false;
        if let Some(initial_margin) = initial_margin {
            if !initial_margin.is_empty() {
                margin = indent;
                indent += initial_margin.len() as isize;
                saw_text = true;
            }
            state = State::SawAsterisk;
        }
        let mut token = self.token();
        loop {
            if token != Token::Backtick && backtick_count > 0 {
                if backtick_count >= 3 {
                    in_fenced_code_block = !in_fenced_code_block;
                }
                backtick_count = 0;
            }
            let saving = if in_fenced_code_block { State::SavingBackticks } else { State::SavingComments };
            let token_len = self.s.token_text().len() as isize;
            let push_comment = |margin: &mut isize, indent: &mut isize, saw_text: &mut bool| {
                if *margin == -1 {
                    *margin = *indent;
                }
                *indent += token_len;
                *saw_text = true;
            };
            match token {
                Token::NewLine => {
                    state = State::BeginningOfLine;
                    indent = 0;
                }
                Token::At => {
                    if !in_fenced_code_block && self.s.can_follow_at() {
                        self.s.pos = self.s.token_start;
                        break;
                    }
                    state = saving;
                    push_comment(&mut margin, &mut indent, &mut saw_text);
                }
                Token::EndOfFile => break,
                Token::Whitespace => {
                    if margin > -1 && indent + token_len > margin {
                        state = saving;
                    }
                    indent += token_len;
                }
                Token::OpenBrace if in_fenced_code_block => {
                    state = State::SavingBackticks;
                    push_comment(&mut margin, &mut indent, &mut saw_text);
                }
                Token::OpenBrace => {
                    state = State::SavingComments;
                    if !self.parse_jsdoc_link() {
                        push_comment(&mut margin, &mut indent, &mut saw_text);
                    } else {
                        saw_text = true;
                    }
                }
                Token::Backtick => {
                    backtick_count += 1;
                    state = if state == State::SavingBackticks {
                        State::SavingComments
                    } else {
                        State::SavingBackticks
                    };
                    push_comment(&mut margin, &mut indent, &mut saw_text);
                }
                Token::CommentText => {
                    if state != State::SavingBackticks {
                        state = saving;
                    }
                    push_comment(&mut margin, &mut indent, &mut saw_text);
                }
                Token::Asterisk if state == State::BeginningOfLine => {
                    state = State::SawAsterisk;
                    indent += 1;
                }
                _ => {
                    if state != State::SavingBackticks {
                        state = saving;
                    }
                    push_comment(&mut margin, &mut indent, &mut saw_text);
                }
            }
            token = if matches!(state, State::SavingComments | State::SavingBackticks) {
                self.next_comment_text_token(state == State::SavingBackticks)
            } else {
                self.next_token_jsdoc()
            };
        }
        saw_text
    }

    fn try_parse_type_expression(&mut self) -> Option<JsDocType> {
        self.skip_whitespace_or_asterisk();
        if self.token() == Token::OpenBrace {
            self.parse_jsdoc_type_expression(false)
        } else {
            None
        }
    }

    /// tsgo's `parseJSDocTypeExpression`: `{T}`, or a bare `T` when the braces
    /// may be omitted (`@type`, `@this`).
    fn parse_jsdoc_type_expression(&mut self, may_omit_braces: bool) -> Option<JsDocType> {
        let has_brace = self.token() == Token::OpenBrace;
        if !has_brace && !may_omit_braces {
            return None;
        }
        let type_start = if has_brace { self.s.pos } else { self.s.token_start };
        let parsed = parse_type_text(self.s.text, type_start);
        for (name, span) in &parsed.not_generic {
            self.diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(2315),
                span: *span,
                name: Some(name.clone()),
            });
        }
        for end in &parsed.missing_types {
            let bytes = self.s.text.as_bytes();
            let mut start = *end;
            while start < bytes.len() && bytes[start].is_ascii_whitespace() {
                start += 1;
            }
            self.diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(1110),
                span: TextSpan { start, end: (start + 1).min(bytes.len()) },
                name: None,
            });
        }
        self.s.pos = parsed.end.max(type_start);
        // tsgo's parser recovers through a malformed type up to the braces
        // that close it; oxc's gives up where it failed.
        if has_brace && parsed.ty.ty.is_none() {
            if let Some(close) = matching_close_brace(self.s.text, type_start) {
                self.s.pos = close;
            }
        }
        self.next_token_jsdoc();
        if has_brace {
            if self.token() == Token::CloseBrace {
                self.next_token_jsdoc();
            } else {
                self.error_at_current_token(1005, Some("}"));
            }
        }
        Some(parsed.ty)
    }

    fn parse_bracket_name_in_property_and_param_tag(
        &mut self,
        target: PropertyLikeParse,
    ) -> Option<(Vec<String>, TextSpan, bool)> {
        let is_bracketed = self.parse_optional_jsdoc(Token::OpenBracket);
        if is_bracketed {
            self.skip_whitespace();
        }
        let is_backquoted = self.parse_optional_jsdoc(Token::Backtick);
        // A missing name is an empty one, which a parameter matches by
        // position (`findMatchingParameter`).
        let name = self.parse_jsdoc_entity_name().or_else(|| {
            if target != PropertyLikeParse::Parameter {
                self.error_at_current_token(1003, None);
            }
            let at = self.s.token_start;
            Some((vec![String::new()], TextSpan { start: at, end: at }))
        });
        if is_backquoted && self.token() == Token::Backtick {
            self.next_token_jsdoc();
        }
        if is_bracketed {
            self.skip_whitespace();
            if self.token() == Token::Equals {
                self.skip_default_value();
            }
            if self.token() == Token::CloseBracket {
                self.next_token();
            }
        }
        name.map(|(name, span)| (name, span, is_bracketed))
    }

    /// Skip a bracketed name's `= default` up to the closing `]`.
    fn skip_default_value(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.next_token_jsdoc() {
                Token::EndOfFile => return,
                Token::OpenBracket | Token::OpenBrace | Token::OpenParen => depth += 1,
                Token::CloseBrace | Token::CloseParen => depth = depth.saturating_sub(1),
                Token::CloseBracket if depth == 0 => return,
                Token::CloseBracket => depth -= 1,
                _ => {}
            }
        }
    }

    fn parse_jsdoc_entity_name(&mut self) -> Option<(Vec<String>, TextSpan)> {
        let (first, span) = self.parse_jsdoc_identifier_name()?;
        let mut names = vec![first];
        let mut span = span;
        if self.token() == Token::OpenBracket {
            self.next_token();
            if self.token() == Token::CloseBracket {
                self.next_token_jsdoc();
            }
        }
        while self.token() == Token::Dot {
            self.next_token_jsdoc();
            let Some((name, name_span)) = self.parse_jsdoc_identifier_name() else {
                break;
            };
            names.push(name);
            span.end = name_span.end;
            if self.token() == Token::OpenBracket {
                self.next_token();
                if self.token() == Token::CloseBracket {
                    self.next_token_jsdoc();
                }
            }
        }
        Some((names, span))
    }

    fn parse_parameter_or_property_tag(
        &mut self,
        start: usize,
        target: PropertyLikeParse,
        indent: isize,
    ) -> Option<JsDocParameterTag> {
        let mut type_expression = self.try_parse_type_expression();
        let is_name_first = type_expression.is_none();
        self.skip_whitespace_or_asterisk();
        let (name, name_span, bracketed) = self.parse_bracket_name_in_property_and_param_tag(target)?;
        let indent_text = self.skip_whitespace_or_asterisk();
        if is_name_first && self.token() == Token::OpenBrace && !self.at_jsdoc_link() {
            type_expression = self.try_parse_type_expression();
        }
        self.parse_trailing_tag_comments(start, self.node_pos(), indent, &indent_text);
        let mut type_expression = type_expression.map(JsDocTypeExpression::Type);
        if let Some(JsDocTypeExpression::Type(ty)) = &type_expression {
            if ty.object_like {
                if let Some(literal) = self.parse_nested_type_literal(ty, &name, target, indent) {
                    type_expression = Some(JsDocTypeExpression::Literal(literal));
                }
            }
        }
        Some(JsDocParameterTag { start, name, name_span, bracketed, type_expression, name_first: is_name_first })
    }

    fn at_jsdoc_link(&mut self) -> bool {
        let state = self.s.mark();
        let is_link = self.parse_jsdoc_link();
        self.s.rewind(state);
        is_link
    }

    fn parse_nested_type_literal(
        &mut self,
        ty: &JsDocType,
        name: &[String],
        target: PropertyLikeParse,
        indent: isize,
    ) -> Option<JsDocTypeLiteral> {
        let mut children = Vec::new();
        loop {
            let state = self.s.mark();
            match self.parse_child_parameter_or_property_tag(target, indent, Some(name)) {
                Some(JsDocTag::Parameter(child) | JsDocTag::Property(child)) => children.push(child),
                Some(JsDocTag::Template { .. }) => self.report_misplaced_template(),
                Some(_) => {}
                None => {
                    self.s.rewind(state);
                    break;
                }
            }
        }
        if children.is_empty() {
            return None;
        }
        Some(JsDocTypeLiteral { properties: children, is_array: ty.is_array, span: ty.span })
    }

    fn parse_child_parameter_or_property_tag(
        &mut self,
        target: PropertyLikeParse,
        indent: isize,
        name: Option<&[String]>,
    ) -> Option<JsDocTag> {
        let mut can_parse_tag = true;
        let mut seen_asterisk = false;
        loop {
            match self.next_token_jsdoc() {
                Token::At => {
                    if can_parse_tag && self.s.can_follow_at() {
                        let child = self.try_parse_child_tag(target, indent)?;
                        if let (Some(name), JsDocTag::Parameter(tag) | JsDocTag::Property(tag)) = (name, &child) {
                            if tag.name.len() < 2 || tag.name[..tag.name.len() - 1] != *name {
                                return None;
                            }
                        }
                        return Some(child);
                    }
                    seen_asterisk = false;
                }
                Token::NewLine => {
                    can_parse_tag = true;
                    seen_asterisk = false;
                }
                Token::Asterisk => {
                    if seen_asterisk {
                        can_parse_tag = false;
                    }
                    seen_asterisk = true;
                }
                Token::Identifier => can_parse_tag = false,
                Token::EndOfFile => return None,
                _ => {}
            }
        }
    }

    fn try_parse_child_tag(&mut self, target: PropertyLikeParse, indent: isize) -> Option<JsDocTag> {
        let start = self.s.token_start;
        self.next_token_jsdoc();
        let (tag_name, tag_name_span) = self.parse_jsdoc_identifier_name().unzip();
        self.child_tag_name_span = tag_name_span;
        let tag_name = tag_name.unwrap_or_default();
        let indent_text = self.skip_whitespace_or_asterisk();
        let accepts: u8 = match tag_name.as_str() {
            "type" => {
                if target == PropertyLikeParse::Property {
                    let ty = self.parse_jsdoc_type_expression(true);
                    return Some(JsDocTag::Type(ty));
                }
                return None;
            }
            "prop" | "property" => PropertyLikeParse::Property as u8,
            "arg" | "argument" | "param" => {
                PropertyLikeParse::Parameter as u8 | PropertyLikeParse::CallbackParameter as u8
            }
            "template" => return Some(self.parse_template_tag(start, indent, &indent_text)),
            "this" => {
                let ty = self.parse_jsdoc_type_expression(true);
                self.skip_whitespace();
                self.parse_trailing_tag_comments(start, self.node_pos(), indent, &indent_text);
                return Some(JsDocTag::This(ty));
            }
            _ => return None,
        };
        if target as u8 & accepts == 0 {
            return None;
        }
        let tag = self.parse_parameter_or_property_tag(start, target, indent)?;
        Some(if target == PropertyLikeParse::Property {
            JsDocTag::Property(tag)
        } else {
            JsDocTag::Parameter(tag)
        })
    }

    fn parse_template_tag(&mut self, start: usize, indent: isize, indent_text: &str) -> JsDocTag {
        let constraint = if self.token() == Token::OpenBrace {
            self.parse_jsdoc_type_expression(false)
        } else {
            None
        };
        let mut parameters = Vec::new();
        loop {
            self.skip_whitespace();
            if let Some(parameter) = self.parse_template_tag_type_parameter() {
                parameters.push(parameter);
            }
            self.skip_whitespace_or_asterisk();
            if !self.parse_optional_jsdoc(Token::Comma) {
                break;
            }
        }
        self.parse_trailing_tag_comments(start, self.node_pos(), indent, indent_text);
        JsDocTag::Template { constraint, parameters }
    }

    fn parse_template_tag_type_parameter(&mut self) -> Option<JsDocTemplateParameter> {
        let start = self.s.token_start;
        let is_bracketed = self.parse_optional_jsdoc(Token::OpenBracket);
        if is_bracketed {
            self.skip_whitespace();
        }
        let mut is_const = false;
        // `parseModifiersEx` takes every modifier keyword followed by a name;
        // the checker, not the parser, rejects the ones a type parameter
        // cannot carry. `default` needs a declaration keyword after it.
        while self.token() == Token::Identifier
            && matches!(
                self.s.token_text(),
                "abstract"
                    | "accessor"
                    | "async"
                    | "const"
                    | "declare"
                    | "export"
                    | "in"
                    | "out"
                    | "override"
                    | "private"
                    | "protected"
                    | "public"
                    | "readonly"
                    | "static"
            )
        {
            let state = self.s.mark();
            let modifier = self.s.token_text() == "const";
            self.next_token();
            if self.token() == Token::Identifier {
                is_const |= modifier;
                continue;
            }
            self.s.rewind(state);
            break;
        }
        // `parseJSDocIdentifierName` with the template tag's own message.
        let Some((name, name_span)) = self.parse_jsdoc_identifier_name() else {
            self.error_at_current_token(1069, None);
            return None;
        };
        let mut default_type = None;
        if is_bracketed {
            self.skip_whitespace();
            if self.token() == Token::Equals {
                let parsed = parse_type_text(self.s.text, self.s.pos);
                self.s.pos = parsed.end.max(self.s.pos);
                let missing = parsed.ty.ty.is_none();
                default_type = Some(parsed.ty);
                self.next_token();
                if missing {
                    self.error_at_current_token(1110, None);
                }
            } else {
                self.error_at_current_token(1005, Some("="));
            }
            if self.token() == Token::CloseBracket {
                self.next_token();
            } else {
                self.error_at_current_token(1005, Some("]"));
            }
        }
        Some(JsDocTemplateParameter {
            name,
            name_span,
            default_type,
            is_const,
            span: TextSpan { start, end: name_span.end },
        })
    }

    /// tsgo's `parseImportTag`: `Default`, `{ A, B as C }`, `* as ns` or
    /// `Default, { … }`, then `from "specifier"`.
    fn parse_import_tag(&mut self, start: usize) -> Option<crate::ParsedImportDeclaration> {
        use crate::{ParsedImportKind, ParsedImportSpecifier};
        // Only a line's leading `*` is the comment's margin
        // (`skipJsDocLeadingAsterisks`); on the tag's own line it is the
        // namespace import's `*`.
        let skip = |parser: &mut Self| {
            let mut line_start = false;
            loop {
                match parser.token() {
                    Token::Whitespace => {}
                    Token::NewLine => line_start = true,
                    Token::Asterisk if line_start => {}
                    _ => break,
                }
                parser.next_token_jsdoc();
            }
        };
        skip(self);
        let mut default = None;
        if self.token() == Token::Identifier && self.s.token_text() != "from" {
            default = self.parse_jsdoc_identifier_name();
            skip(self);
            if self.token() == Token::Comma {
                self.next_token_jsdoc();
                skip(self);
            }
        }
        let mut named = None;
        let mut namespace = None;
        match self.token() {
            Token::OpenBrace => {
                self.next_token_jsdoc();
                let mut specifiers = Vec::new();
                loop {
                    skip(self);
                    if self.token() == Token::CloseBrace {
                        self.next_token_jsdoc();
                        break;
                    }
                    let (imported, imported_span) = self.parse_jsdoc_identifier_name()?;
                    skip(self);
                    let (local, local_span) = if self.token() == Token::Identifier && self.s.token_text() == "as" {
                        self.next_token_jsdoc();
                        skip(self);
                        self.parse_jsdoc_identifier_name()?
                    } else {
                        (imported.clone(), imported_span)
                    };
                    specifiers.push(ParsedImportSpecifier {
                        imported_name: imported,
                        local_name: local,
                        name_span: Some(local_span),
                    });
                    skip(self);
                    if self.token() == Token::Comma {
                        self.next_token_jsdoc();
                    }
                }
                named = Some(specifiers);
            }
            Token::Asterisk => {
                self.next_token_jsdoc();
                skip(self);
                if !(self.token() == Token::Identifier && self.s.token_text() == "as") {
                    return None;
                }
                self.next_token_jsdoc();
                skip(self);
                namespace = self.parse_jsdoc_identifier_name();
            }
            _ => {}
        }
        skip(self);
        if !(self.token() == Token::Identifier && self.s.token_text() == "from") {
            return None;
        }
        self.next_token_jsdoc();
        skip(self);
        let quote = self.s.char_at(self.s.token_start).filter(|quote| matches!(quote, '"' | '\''))?;
        let literal_start = self.s.token_start;
        let value_start = literal_start + 1;
        let value_end = value_start + self.s.text[value_start..].find(quote)?;
        let specifier = self.s.text[value_start..value_end].to_string();
        self.s.pos = value_end + 1;
        self.next_token_jsdoc();
        let resolution_mode = self.parse_import_tag_resolution_mode();
        let kind = match (default, named, namespace) {
            (Some((local_name, span)), None, None) => ParsedImportKind::TypeOnlyDefault {
                local_name,
                name_span: Some(span),
            },
            (Some((local_name, span)), Some(specifiers), None) => ParsedImportKind::DefaultAndNamed {
                local_name,
                name_span: Some(span),
                is_type_only: true,
                specifiers,
            },
            (None, Some(specifiers), None) => ParsedImportKind::Named { is_type_only: true, specifiers },
            (None, None, Some((local_name, span))) => ParsedImportKind::Namespace {
                local_name,
                name_span: Some(span),
                is_type_only: true,
            },
            _ => return None,
        };
        Some(crate::ParsedImportDeclaration {
            kind,
            module_specifier: specifier,
            module_specifier_span: Some(TextSpan { start: literal_start, end: value_end + 1 }),
            span: Some(TextSpan { start, end: value_end + 1 }),
            resolution_mode,
            inline_type_specifiers: false,
        })
    }

    /// The `resolution-mode` of an `@import`'s `with { … }` attributes
    /// (`tryParseImportAttributes`), which selects the resolution of a
    /// declaration that is type-only as a whole, as every `@import` is.
    fn parse_import_tag_resolution_mode(&mut self) -> Option<crate::ParsedResolutionModeAttribute> {
        while self.token() == Token::Whitespace {
            self.next_token_jsdoc();
        }
        if !(self.token() == Token::Identifier && self.s.token_text() == "with") {
            return None;
        }
        let rest = &self.s.text[self.s.pos..];
        let open = rest.find('{')?;
        if !rest[..open].trim().is_empty() {
            return None;
        }
        let close = open + rest[open..].find('}')?;
        let attributes = &rest[open + 1..close];
        self.s.pos += close + 1;
        self.next_token_jsdoc();
        let unquote = |text: &str| {
            let text = text.trim();
            text.strip_prefix(['"', '\'']).and_then(|text| text.strip_suffix(['"', '\''])).map(str::to_string)
        };
        attributes.split(',').find_map(|attribute| {
            let (key, value) = attribute.split_once(':')?;
            if unquote(key).or_else(|| Some(key.trim().to_string()))?.as_str() != "resolution-mode" {
                return None;
            }
            let mode = match unquote(value)?.as_str() {
                "import" => crate::ResolutionModeOverride::Import,
                "require" => crate::ResolutionModeOverride::Require,
                _ => return None,
            };
            Some(crate::ParsedResolutionModeAttribute { mode, selects_resolution: true })
        })
    }

    fn parse_jsdoc_type_name_with_namespace(&mut self) -> Option<(Vec<String>, TextSpan)> {
        let (first, span) = self.parse_jsdoc_identifier_name()?;
        let mut names = vec![first];
        let mut span = span;
        while self.token() == Token::Dot {
            self.next_token_jsdoc();
            let Some((name, name_span)) = self.parse_jsdoc_identifier_name() else {
                break;
            };
            names.push(name);
            span = name_span;
        }
        Some((names, span))
    }

    fn parse_typedef_tag(&mut self, start: usize, indent: isize, indent_text: &str) -> JsDocTag {
        let mut type_expression = self.try_parse_type_expression().map(JsDocTypeExpression::Type);
        self.skip_whitespace_or_asterisk();
        let (name, name_span) = self.parse_jsdoc_type_name_with_namespace().unwrap_or_else(|| {
            self.error_at_current_token(1003, None);
            let at = self.s.full_start;
            (vec![String::new()], TextSpan { start: at, end: at })
        });
        self.skip_whitespace();
        let commented = self.parse_tag_comments(indent, None);

        let object_like = match &type_expression {
            None => true,
            Some(JsDocTypeExpression::Type(ty)) => ty.object_like,
            Some(JsDocTypeExpression::Literal(_)) => false,
        };
        if object_like {
            let mut properties = Vec::new();
            let mut child_type: Option<JsDocType> = None;
            let mut seen_type_tag = false;
            let mut has_children = false;
            loop {
                let state = self.s.mark();
                match self.parse_child_parameter_or_property_tag(PropertyLikeParse::Property, indent, None) {
                    Some(JsDocTag::Property(property) | JsDocTag::Parameter(property)) => {
                        has_children = true;
                        properties.push(property);
                    }
                    Some(JsDocTag::Type(ty)) => {
                        has_children = true;
                        if seen_type_tag {
                            self.error_at_current_token(8033, None);
                        } else {
                            seen_type_tag = true;
                            child_type = ty;
                        }
                    }
                    Some(JsDocTag::Template { .. }) => {
                        has_children = true;
                        self.report_misplaced_template();
                    }
                    Some(_) => has_children = true,
                    None => {
                        self.s.rewind(state);
                        break;
                    }
                }
            }
            if has_children {
                let is_array = matches!(&type_expression, Some(JsDocTypeExpression::Type(ty)) if ty.is_array);
                type_expression = match child_type {
                    Some(ty) if !ty.object_like => Some(JsDocTypeExpression::Type(ty)),
                    _ => Some(JsDocTypeExpression::Literal(JsDocTypeLiteral {
                        span: properties.first().map_or(TextSpan { start, end: start }, |p| p.name_span),
                        properties,
                        is_array,
                    })),
                };
            }
        }
        if !commented {
            self.parse_trailing_tag_comments(start, self.node_pos(), indent, indent_text);
        }
        JsDocTag::Typedef { name, name_span, type_expression }
    }

    fn parse_callback_tag(&mut self, start: usize, indent: isize, indent_text: &str) -> JsDocTag {
        let (name, name_span) = self.parse_jsdoc_type_name_with_namespace().unwrap_or_else(|| {
            self.error_at_current_token(1003, None);
            let at = self.s.full_start;
            (vec![String::new()], TextSpan { start: at, end: at })
        });
        self.skip_whitespace();
        let commented = self.parse_tag_comments(indent, None);
        let signature = self.parse_jsdoc_signature(indent);
        if !commented {
            self.parse_trailing_tag_comments(start, self.node_pos(), indent, indent_text);
        }
        JsDocTag::Callback { name, name_span, signature, span: TextSpan { start, end: self.node_pos() } }
    }

    fn parse_jsdoc_signature(&mut self, indent: isize) -> JsDocSignature {
        let mut parameters = Vec::new();
        let mut this_type = None;
        loop {
            let state = self.s.mark();
            match self.parse_child_parameter_or_property_tag(PropertyLikeParse::CallbackParameter, indent, None) {
                Some(JsDocTag::Parameter(parameter)) => parameters.push(parameter),
                Some(JsDocTag::This(ty)) => this_type = ty,
                Some(JsDocTag::Template { .. }) => self.report_misplaced_template(),
                Some(_) => {}
                None => {
                    self.s.rewind(state);
                    break;
                }
            }
        }
        let state = self.s.mark();
        let mut return_type = None;
        let mut found_return = false;
        if self.next_token_jsdoc() == Token::At || self.token() == Token::At {
            if let JsDocTag::Return(ty) = self.parse_tag(indent) {
                return_type = ty;
                found_return = true;
            }
        }
        if !found_return {
            self.s.rewind(state);
        }
        JsDocSignature { parameters, this_type, return_type }
    }
}

/// The spans of the bare references to any of `names` in `ty`.
fn named_type_references(ty: &ParsedType, names: &[&str], out: &mut Vec<TextSpan>) {
    match ty {
        ParsedType::Named(named) => {
            if named.type_arguments.is_empty()
                && names.contains(&named.name.as_str())
                && let Some(span) = named.span
            {
                out.push(span);
            }
            for argument in &named.type_arguments {
                named_type_references(argument, names, out);
            }
        }
        ParsedType::Array(element) => named_type_references(element, names, out),
        ParsedType::Tuple(elements) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
            for element in elements.iter() {
                named_type_references(element, names, out);
            }
        }
        _ => {}
    }
}

/// The `}` that closes the brace opened just before `start`.
fn matching_close_brace(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in text.as_bytes().get(start..)?.iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' if depth == 0 => return Some(start + offset),
            b'}' => depth -= 1,
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Types.

struct ParsedTypeText {
    ty: JsDocType,
    end: usize,
    /// Where each lone `?` ends: tsgo's `parseJSDocNullableType` requires a
    /// type after it and reports TS1110 at the next token.
    missing_types: Vec<usize>,
    /// Each intended type written with type arguments (`Void<T>`), which
    /// `checkNoTypeArguments` reports as not generic: its name and span.
    not_generic: Vec<(String, TextSpan)>,
}

/// tsgo's `parseJSDocType` over the text at `start`: the vendored oxc parser
/// reads the type in place, so every span in it is the file's own.
fn parse_type_text(text: &str, start: usize) -> ParsedTypeText {
    let allocator = Allocator::default();
    let parsed = oxc_parser::Parser::new(&allocator, text, SourceType::ts()).parse_jsdoc_type(start as u32);
    let end = (parsed.end as usize).min(text.len());
    let Some(mut ty) = parsed.ty else {
        return ParsedTypeText {
            ty: JsDocType {
                ty: None,
                span: TextSpan { start, end },
                variadic: parsed.variadic,
                optional: parsed.optional,
                object_like: false,
                is_array: false,
            },
            end,
            missing_types: Vec::new(),
            not_generic: Vec::new(),
        };
    };
    let span = ty.span();
    let mut missing = MissingNullableTypes(Vec::new(), AstBuilder::new(&allocator));
    missing.visit_ts_type(&mut ty);
    let missing_types = missing.0;
    let (object_like, is_array) = object_or_object_array(&ty);
    let mut import_types = ImportTypeSpecifiers(Vec::new());
    import_types.visit_ts_type(&ty);
    IMPORT_TYPE_SPECIFIERS.with(|specifiers| specifiers.borrow_mut().extend(import_types.0));
    let mut intended = IntendedTypes { ast: AstBuilder::new(&allocator), not_generic: Vec::new() };
    intended.visit_ts_type(&mut ty);
    let not_generic = intended.not_generic;
    let lowered = super::types::parse_type(&ty);
    ParsedTypeText {
        ty: JsDocType {
            ty: lowered,
            span: TextSpan { start: span.start as usize, end: span.end as usize },
            variadic: parsed.variadic,
            optional: parsed.optional,
            object_like,
            is_array,
        },
        end,
        missing_types,
        not_generic,
    }
}

/// oxc reads a lone `?` as JSDoc's unknown type; tsgo reads a nullable type
/// whose type is missing, which resolves to the error type.
struct MissingNullableTypes<'a>(Vec<usize>, AstBuilder<'a>);

impl<'a> VisitMut<'a> for MissingNullableTypes<'a> {
    fn visit_ts_type(&mut self, ty: &mut TSType<'a>) {
        if let TSType::JSDocUnknownType(unknown) = ty {
            self.0.push(unknown.span.end as usize);
            *ty = self.1.ts_type_any_keyword(unknown.span);
            return;
        }
        walk_mut::walk_ts_type(self, ty);
    }
}

/// The modules a JSDoc type names with `import("…")`, which belong to the
/// program's module graph as a written import type's do
/// (`ForEachDynamicImportOrRequireCall` with `includeTypeSpaceImports`).
struct ImportTypeSpecifiers(Vec<String>);

impl<'a> Visit<'a> for ImportTypeSpecifiers {
    fn visit_ts_import_type(&mut self, it: &oxc_ast::ast::TSImportType<'a>) {
        self.0.push(it.source.value.to_string());
        walk::walk_ts_import_type(self, it);
    }
}

/// tsgo's `isObjectOrObjectArrayTypeReference`, and whether it is the array.
fn object_or_object_array(ty: &TSType<'_>) -> (bool, bool) {
    match ty {
        TSType::TSObjectKeyword(_) => (true, false),
        TSType::TSArrayType(array) => (object_or_object_array(&array.element_type).0, true),
        TSType::TSTypeReference(reference) => (
            reference.type_arguments.is_none()
                && matches!(&reference.type_name, TSTypeName::IdentifierReference(name) if name.name == "Object"),
            false,
        ),
        _ => (false, false),
    }
}

/// tsgo's `getIntendedTypeFromJSDocTypeReference`: the names JSDoc writes for
/// primitives and `Object.<K, V>` for a record.
struct IntendedTypes<'a> {
    ast: AstBuilder<'a>,
    not_generic: Vec<(String, TextSpan)>,
}

impl<'a> VisitMut<'a> for IntendedTypes<'a> {
    fn visit_ts_type(&mut self, ty: &mut TSType<'a>) {
        if let TSType::TSTypeReference(reference) = ty {
            let span = reference.span;
            let arguments = reference.type_arguments.as_ref().map_or(0, |arguments| arguments.params.len());
            // `IsJSDocIndexSignature`: `Object.<K, V>` is an index signature
            // only for a `string` or `number` key; any other key is `any`.
            let index_signature_key = reference.type_arguments.as_ref().is_some_and(|arguments| {
                matches!(
                    arguments.params.first(),
                    Some(TSType::TSStringKeyword(_) | TSType::TSNumberKeyword(_))
                )
            });
            if let TSTypeName::IdentifierReference(name) = &mut reference.type_name {
                let replacement = match name.name.as_str() {
                    "String" => Some(self.ast.ts_type_string_keyword(span)),
                    "Number" => Some(self.ast.ts_type_number_keyword(span)),
                    "BigInt" => Some(self.ast.ts_type_big_int_keyword(span)),
                    "Boolean" => Some(self.ast.ts_type_boolean_keyword(span)),
                    "Void" => Some(self.ast.ts_type_void_keyword(span)),
                    "Undefined" => Some(self.ast.ts_type_undefined_keyword(span)),
                    "Null" => Some(self.ast.ts_type_null_keyword(span)),
                    _ => None,
                };
                let intended_function = matches!(name.name.as_str(), "Function" | "function");
                if arguments > 0 && (replacement.is_some() || intended_function) {
                    self.not_generic.push((
                        name.name.to_string(),
                        TextSpan { start: span.start as usize, end: span.end as usize },
                    ));
                }
                if let Some(replacement) = replacement {
                    *ty = replacement;
                    return;
                }
                if intended_function && arguments > 0 {
                    name.name = self.ast.ident("Function");
                    reference.type_arguments = None;
                    return;
                }
                match (name.name.as_str(), arguments) {
                    ("function", 0) => name.name = self.ast.ident("Function"),
                    ("Object", 2) if index_signature_key => name.name = self.ast.ident("Record"),
                    ("Object", 2) => {
                        *ty = self.ast.ts_type_any_keyword(span);
                        return;
                    }
                    _ => {}
                }
            }
        }
        walk_mut::walk_ts_type(self, ty);
    }
}

fn lowered(ty: &JsDocType) -> ParsedType {
    let base = ty.ty.clone().unwrap_or(ParsedType::Unknown);
    if ty.variadic { ParsedType::Array(std::sync::Arc::new(base)) } else { base }
}

/// A type as `getTypeFromTypeNode` reads it outside a parameter: `T=` adds
/// `undefined` (`addOptionality`).
fn lowered_with_optionality(ty: &JsDocType) -> ParsedType {
    let base = lowered(ty);
    if ty.optional {
        ParsedType::Union(std::sync::Arc::new(vec![base, ParsedType::Undefined]))
    } else {
        base
    }
}

/// tsgo's `reparseJSDocTypeLiteral`.
fn type_of_expression(expression: &JsDocTypeExpression) -> Option<ParsedType> {
    match expression {
        // `T=` is the `JSDocOptionalType` whose type includes `undefined`.
        JsDocTypeExpression::Type(ty) => ty.ty.as_ref().map(|_| lowered_with_optionality(ty)),
        JsDocTypeExpression::Literal(literal) => {
            let properties = literal
                .properties
                .iter()
                .map(|property| ParsedObjectTypeProperty {
                    name: property.name.last().cloned().unwrap_or_default(),
                    name_span: Some(property.name_span),
                    ty: property
                        .type_expression
                        .as_ref()
                        .and_then(type_of_expression)
                        .unwrap_or(ParsedType::Any),
                    optional: is_optional(property),
                    is_method: false,
                    readonly: false,
                    write_ty: None,
                })
                .collect();
            let object = ParsedType::Object(std::sync::Arc::new(ParsedObjectType {
                properties,
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                call_signature: None,
                call_signature_overloads: Vec::new(),
                construct_signature: None,
                construct_signature_overloads: Vec::new(),
                non_primitive: false,
                display_name: None,
                abstract_construct_signature: false,
                keyed_indexes: None,
            }));
            Some(if literal.is_array { ParsedType::Array(std::sync::Arc::new(object)) } else { object })
        }
    }
}

fn type_span_of_expression(expression: &JsDocTypeExpression) -> TextSpan {
    match expression {
        JsDocTypeExpression::Type(ty) => ty.span,
        JsDocTypeExpression::Literal(literal) => literal.span,
    }
}

/// tsgo's `makeQuestionIfOptional`.
fn is_optional(tag: &JsDocParameterTag) -> bool {
    tag.bracketed || matches!(&tag.type_expression, Some(JsDocTypeExpression::Type(ty)) if ty.optional)
}

fn template_parameters(tags: &[JsDocTag], typedef_or_callback: bool) -> Vec<ParsedTypeParameter> {
    let mut parameters = Vec::new();
    for tag in tags {
        if !typedef_or_callback && matches!(tag, JsDocTag::Typedef { .. } | JsDocTag::Callback { .. }) {
            return Vec::new();
        }
        let JsDocTag::Template { constraint, parameters: template } = tag else {
            continue;
        };
        for (index, parameter) in template.iter().enumerate() {
            parameters.push(ParsedTypeParameter {
                name: parameter.name.clone(),
                name_span: Some(parameter.name_span),
                constraint: if index == 0 { constraint.as_ref().and_then(|ty| ty.ty.clone()) } else { None },
                default_type: parameter.default_type.as_ref().and_then(|ty| ty.ty.clone()),
                span: Some(parameter.span),
                is_const: parameter.is_const,
                is_in: false,
                is_out: false,
            });
        }
    }
    parameters
}

fn signature_type(signature: &JsDocSignature, type_parameters: Vec<ParsedTypeParameter>) -> ParsedType {
    let mut parameters = Vec::new();
    if let Some(this_type) = &signature.this_type {
        parameters.push(ParsedFunctionTypeParameter {
            name: Some("this".to_string()),
            name_span: None,
            ty: lowered(this_type),
            optional: false,
            is_this: true,
            polymorphic_this: false,
            rest: false,
            bound_names: Vec::new(),
        });
    }
    for parameter in &signature.parameters {
        if parameter.name.len() != 1 {
            continue;
        }
        let variadic = matches!(&parameter.type_expression, Some(JsDocTypeExpression::Type(ty)) if ty.variadic);
        parameters.push(ParsedFunctionTypeParameter {
            name: parameter.name.first().cloned(),
            name_span: Some(parameter.name_span),
            ty: parameter
                .type_expression
                .as_ref()
                .and_then(type_of_expression)
                .unwrap_or(ParsedType::Any),
            optional: is_optional(parameter),
            is_this: false,
            polymorphic_this: false,
            rest: variadic,
            bound_names: Vec::new(),
        });
    }
    let return_type = signature
        .return_type
        .as_ref()
        .and_then(|ty| ty.ty.as_ref().map(|_| lowered(ty)))
        .unwrap_or(ParsedType::Any);
    ParsedType::Function(std::sync::Arc::new(ParsedFunctionType {
        parameters,
        return_type: Box::new(return_type),
        type_parameters,
    }))
}

// ---------------------------------------------------------------------------
// The reparse: what each declaration's JSDoc contributes.

#[derive(Clone, Debug, Default)]
pub(crate) struct JsDocParameter {
    pub(crate) ty: Option<(ParsedType, TextSpan)>,
    pub(crate) optional: bool,
    /// `makeQuestionIfOptional`: the question token an optional `@param`
    /// gives the parameter, located at the tag.
    pub(crate) question: Option<TextSpan>,
}

#[derive(Default)]
pub(crate) struct JsDocIndex {
    /// A parameter's type and optionality, by the parameter's start.
    parameters: std::collections::HashMap<u32, JsDocParameter>,
    /// The parameters of a signature a JSDoc typed, which are therefore not
    /// the optional parameters of an untyped JavaScript signature.
    typed_signature_parameters: std::collections::HashSet<u32>,
    /// A function's return type, by the function's start.
    returns: std::collections::HashMap<u32, (ParsedType, TextSpan)>,
    /// A function's or class's `@template` parameters, by its start.
    type_parameters: std::collections::HashMap<u32, Vec<ParsedTypeParameter>>,
    /// A function's `@this` type, by the function's start.
    this_types: std::collections::HashMap<u32, ParsedType>,
    /// The functions whose `@type` full signature is a type whose `this`
    /// parameter cannot be read off it (a named type), by the function's start.
    opaque_full_signatures: std::collections::HashSet<u32>,
    /// The functions `reparseHosted` gives their `@type` as the full
    /// signature, by the function's start.
    full_signature_hosts: std::collections::HashSet<u32>,
    /// The full signature of each such function that could not be spread over
    /// its parameters here (a named, generic or differently sized signature),
    /// left for the checker to resolve; by the function's start.
    full_signatures: std::collections::HashMap<u32, ParsedType>,
    /// A class's `@implements` interfaces, by the class's start.
    implements: std::collections::HashMap<u32, Vec<crate::ParsedNamedType>>,
    /// A function's `@overload` signatures with their tags, by the function's
    /// start.
    overloads: std::collections::HashMap<u32, Vec<(ParsedType, TextSpan)>>,
    /// The `@type` of a variable, class field or accessor, by its start.
    declared: std::collections::HashMap<u32, (ParsedType, TextSpan)>,
    /// `/** @type {T} */ (e)` and `@satisfies`, by the parenthesized
    /// expression's start. `true` for an assertion.
    casts: std::collections::HashMap<u32, (ParsedType, TextSpan, bool)>,
    /// A variable statement's `@satisfies`, by the initializer's start.
    initializer_satisfies: std::collections::HashMap<u32, (ParsedType, TextSpan)>,
    /// `/** @type {T} */ o.x = e`: an assignment declaration typed `T`, by
    /// the assigned value's span; likewise a property assignment's value
    /// (`{ /** @type {T} */ x: e }`). The last span is the node tsc reports
    /// a mismatch on: the assignment's target, the property, or the value.
    assignment_types: std::collections::HashMap<(u32, u32), (ParsedType, TextSpan, TextSpan)>,
    /// `/** @type {T} */ return e`: `e as T`, by the return statement's start.
    return_casts: std::collections::HashMap<u32, (ParsedType, TextSpan, bool)>,
    /// A class's `@augments` type arguments for its `extends` clause, by the
    /// class's start.
    extends_type_arguments: std::collections::HashMap<u32, (Vec<ParsedType>, Option<crate::TextSpan>)>,
    /// The parse errors of the attached JSDoc comments.
    diagnostics: Vec<crate::ParsedGrammarDiagnostic>,
    /// What reparsing a tag finds wrong: a parse error of the file itself
    /// (`checkNonIdentifierName`), which stops the program's checking.
    parse_errors: Vec<(u32, TextSpan)>,
    /// `@typedef` and `@callback` aliases.
    aliases: Vec<ParsedTypeAliasDeclaration>,
    /// Beside each alias: the statement list its comment sits in, by that
    /// list's first statement, or `None` for the file's own statements. tsgo's
    /// reparser appends a typedef to the statement list being parsed.
    alias_scopes: Vec<Option<u32>>,
    /// `@import` declarations.
    imports: Vec<crate::ParsedImportDeclaration>,
    /// Every `import("…")` specifier the file's JSDoc types name.
    import_type_specifiers: Vec<String>,
    /// A class member's JSDoc modifiers, by the member's start (a `this.x = e`
    /// member's by the assignment's).
    member_modifiers: std::collections::HashMap<u32, Vec<(JsDocModifier, TextSpan)>>,
}

thread_local! {
    static INDEX: RefCell<Option<std::rc::Rc<JsDocIndex>>> = const { RefCell::new(None) };
    /// See [`JsDocIndex::import_type_specifiers`], gathered while the index
    /// is built.
    static IMPORT_TYPE_SPECIFIERS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The `import("…")` specifiers `index`'s JSDoc types name.
pub(crate) fn import_type_specifiers(index: &JsDocIndex) -> &[String] {
    &index.import_type_specifiers
}

/// Makes `index` readable to the lowering for the duration of `f`.
pub(crate) fn with_jsdoc_index<R>(index: Option<std::rc::Rc<JsDocIndex>>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<std::rc::Rc<JsDocIndex>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INDEX.with(|index| *index.borrow_mut() = self.0.take());
        }
    }
    let previous = INDEX.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), index));
    let _restore = Restore(previous);
    f()
}

fn with_index<R>(f: impl FnOnce(&JsDocIndex) -> Option<R>) -> Option<R> {
    INDEX.with(|slot| slot.borrow().as_deref().and_then(f))
}

pub(crate) fn parameter_at(start: u32) -> Option<JsDocParameter> {
    with_index(|index| index.parameters.get(&start).cloned())
}

pub(crate) fn in_typed_signature(parameter_start: u32) -> bool {
    with_index(|index| index.typed_signature_parameters.contains(&parameter_start).then_some(())).is_some()
}

pub(crate) fn return_type_at(function_start: u32) -> Option<(ParsedType, TextSpan)> {
    with_index(|index| index.returns.get(&function_start).cloned())
}

pub(crate) fn type_parameters_at(start: u32) -> Option<Vec<ParsedTypeParameter>> {
    with_index(|index| index.type_parameters.get(&start).cloned())
}

pub(crate) fn this_type_at(function_start: u32) -> Option<ParsedType> {
    with_index(|index| index.this_types.get(&function_start).cloned())
}

/// Whether the function at `function_start` takes its signature from a JSDoc
/// `@type` naming a type, whose `this` parameter is not known here.
pub(crate) fn has_opaque_full_signature(function_start: u32) -> bool {
    with_index(|index| index.opaque_full_signatures.contains(&function_start).then_some(())).is_some()
}

/// The `@overload` signatures of the function starting at `function_start`.
pub(crate) fn overloads_at(function_start: u32) -> Vec<(ParsedType, TextSpan)> {
    with_index(|index| index.overloads.get(&function_start).cloned()).unwrap_or_default()
}

pub(crate) fn full_signature_at(function_start: u32) -> Option<ParsedType> {
    with_index(|index| index.full_signatures.get(&function_start).cloned())
}

pub(crate) fn declared_type_at(start: u32) -> Option<(ParsedType, TextSpan)> {
    with_index(|index| index.declared.get(&start).cloned())
}

/// Whether the JSDoc `@type` of the declaration at `start` gives a function
/// initializer a contextual `this`: `None` without one, `Some(false)` for a
/// function type with no `this` parameter or for JSDoc's `Function`, which has
/// no signature at all.
pub(crate) fn declared_type_supplies_this(start: u32) -> Option<bool> {
    declared_type_at(start).map(|(ty, _)| match ty {
        ParsedType::Function(function) => function.parameters.iter().any(|parameter| parameter.is_this),
        ParsedType::Named(named) => !is_jsdoc_function_type(&named),
        _ => true,
    })
}

pub(crate) fn cast_at(start: u32) -> Option<(ParsedType, TextSpan, bool)> {
    with_index(|index| index.casts.get(&start).cloned())
}

pub(crate) fn initializer_satisfies_at(start: u32) -> Option<(ParsedType, TextSpan)> {
    with_index(|index| index.initializer_satisfies.get(&start).cloned())
}

pub(crate) fn assignment_type_at(start: u32, end: u32) -> Option<(ParsedType, TextSpan, TextSpan)> {
    with_index(|index| index.assignment_types.get(&(start, end)).cloned())
}

pub(crate) fn return_cast_at(start: u32) -> Option<(ParsedType, TextSpan, bool)> {
    with_index(|index| index.return_casts.get(&start).cloned())
}

/// The `@implements` interfaces of the class starting at `class_start`.
pub(crate) fn implements_at(class_start: u32) -> Vec<crate::ParsedNamedType> {
    with_index(|index| index.implements.get(&class_start).cloned()).unwrap_or_default()
}

pub(crate) fn extends_type_arguments_at(class_start: u32) -> Option<(Vec<ParsedType>, Option<crate::TextSpan>)> {
    with_index(|index| index.extends_type_arguments.get(&class_start).cloned())
}

/// The parse errors of the file's JSDoc comments.
pub(crate) fn diagnostics() -> Vec<crate::ParsedGrammarDiagnostic> {
    with_index(|index| Some(index.diagnostics.clone())).unwrap_or_default()
}

/// The file's parse errors its JSDoc reparse reports.
pub(crate) fn reparse_errors(index: &JsDocIndex) -> Vec<(u32, TextSpan)> {
    index.parse_errors.clone()
}

/// tsgo's `checkNonIdentifierName`: a typedef or callback name that is not an
/// identifier, reported on it (or on the character before a missing one).
fn check_non_identifier_name(name: &[String], span: TextSpan, errors: &mut Vec<(u32, TextSpan)>) {
    let Some(last) = name.last() else {
        return;
    };
    let mut chars = last.chars();
    let valid = chars.next().is_some_and(is_identifier_start) && chars.all(is_identifier_part);
    if valid {
        return;
    }
    let span = if span.end == span.start {
        TextSpan { start: span.start.saturating_sub(1), end: span.start }
    } else {
        span
    };
    errors.push((1003, span));
}

/// A function's written return type, or its JSDoc `@returns`.
pub(crate) fn annotated_or_jsdoc_return_type(
    annotation: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>,
    function_start: u32,
) -> (Option<ParsedType>, Option<TextSpan>) {
    match annotation {
        Some(annotation) => (
            super::types::parse_type_annotation(annotation),
            Some(super::spans::text_span_from_oxc_span(annotation.type_annotation.span())),
        ),
        None => match return_type_at(function_start) {
            Some((ty, span)) => (Some(ty), Some(span)),
            None => (None, None),
        },
    }
}

/// A declaration's written type parameters, or its JSDoc `@template`s.
pub(crate) fn written_or_jsdoc_type_parameters(
    written: Option<&oxc_ast::ast::TSTypeParameterDeclaration<'_>>,
    start: u32,
) -> Vec<ParsedTypeParameter> {
    match written {
        Some(_) => super::types::parse_type_parameters(written),
        None => type_parameters_at(start).unwrap_or_default(),
    }
}

/// The JSDoc modifiers of the class member (or `this.x = e` assignment)
/// starting at `start`, in tag order.
pub(crate) fn member_modifiers_at(start: u32) -> Vec<(JsDocModifier, TextSpan)> {
    with_index(|index| index.member_modifiers.get(&start).cloned()).unwrap_or_default()
}

/// The accessibility a member's JSDoc gives it.
pub(crate) fn member_accessibility_at(start: u32) -> Option<crate::ParsedMemberAccessibility> {
    member_modifiers_at(start).iter().find_map(|(modifier, _)| match modifier {
        JsDocModifier::Private => Some(crate::ParsedMemberAccessibility::Private),
        JsDocModifier::Protected => Some(crate::ParsedMemberAccessibility::Protected),
        _ => None,
    })
}

pub(crate) fn member_has_modifier(start: u32, modifier: JsDocModifier) -> bool {
    member_modifiers_at(start).iter().any(|(written, _)| *written == modifier)
}

/// The type-only imports the file's `@import` tags declare.
pub(crate) fn import_statements() -> Vec<ParsedStatement> {
    with_index(|index| {
        Some(
            index
                .imports
                .iter()
                .map(|import| ParsedStatement::ImportDeclaration(Box::new(import.clone())))
                .collect(),
        )
    })
    .unwrap_or_default()
}

/// The type aliases the `@typedef` and `@callback` tags inside the statement
/// list starting at `first_statement` declare.
pub(crate) fn statement_list_aliases(first_statement: u32) -> Vec<crate::ParsedFunctionBodyStatement> {
    with_index(|index| {
        Some(
            index
                .aliases
                .iter()
                .zip(&index.alias_scopes)
                .filter(|(_, scope)| **scope == Some(first_statement))
                .map(|(alias, _)| crate::ParsedFunctionBodyStatement::TypeAlias(Box::new(alias.clone())))
                .collect(),
        )
    })
    .unwrap_or_default()
}

/// The statements the file's own `@typedef` and `@callback` tags declare; in
/// a module each is exported (`IsImplicitlyExportedJSDocDeclaration`).
pub(crate) fn alias_statements(is_module: bool) -> Vec<ParsedStatement> {
    with_index(|index| {
        Some(
            index
                .aliases
                .iter()
                .zip(&index.alias_scopes)
                .filter(|(_, scope)| scope.is_none())
                .map(|(alias, _)| {
                    let statement = ParsedStatement::TypeAliasDeclaration(Box::new(alias.clone()));
                    if is_module {
                        ParsedStatement::ExportDeclaration(Box::new(ParsedExportDeclaration::Statement {
                            declaration: Box::new(statement),
                            is_type_only: false,
                        }))
                    } else {
                        statement
                    }
                })
                .collect(),
        )
    })
    .unwrap_or_default()
}

/// Build the index for a JavaScript program.
pub(crate) fn build_jsdoc_index(program: &Program<'_>, source_text: &str) -> JsDocIndex {
    IMPORT_TYPE_SPECIFIERS.with(|specifiers| specifiers.borrow_mut().clear());
    let mut builder = IndexBuilder {
        source_text,
        comments: &program.comments,
        parsed: std::collections::HashMap::default(),
        unhosted_done: std::collections::HashSet::default(),
        index: JsDocIndex::default(),
        alias_comment_starts: Vec::new(),
        statement_lists: Vec::new(),
        hosting_constructor: false,
    };
    builder.visit_program(program);
    builder.index.alias_scopes = builder
        .alias_comment_starts
        .iter()
        .map(|&comment| {
            builder
                .statement_lists
                .iter()
                .filter(|(start, end, _)| *start <= comment && comment < *end)
                .min_by_key(|(start, end, _)| end - start)
                .map(|(_, _, first)| *first)
        })
        .collect();
    builder.index.import_type_specifiers =
        IMPORT_TYPE_SPECIFIERS.with(|specifiers| std::mem::take(&mut *specifiers.borrow_mut()));
    builder.index
}

struct IndexBuilder<'s, 'c> {
    source_text: &'s str,
    comments: &'c [Comment],
    parsed: std::collections::HashMap<u32, std::rc::Rc<JsDocComment>>,
    unhosted_done: std::collections::HashSet<u32>,
    index: JsDocIndex,
    /// The comment each alias was declared by, beside `index.aliases`.
    alias_comment_starts: Vec<u32>,
    /// Every nested statement list: its container's span and its first
    /// statement's start.
    statement_lists: Vec<(u32, u32, u32)>,
    /// A constructor's tags are being hosted: its overloads need no return.
    hosting_constructor: bool,
}

/// The function-like node a JSDoc on some declaration applies to
/// (`getFunctionLikeHost`).
#[derive(Clone, Copy)]
enum FunctionHost<'n, 'a> {
    Function(&'n Function<'a>),
    Arrow(&'n ArrowFunctionExpression<'a>),
}

impl<'n, 'a> FunctionHost<'n, 'a> {
    fn start(self) -> u32 {
        match self {
            Self::Function(function) => function.span.start,
            Self::Arrow(arrow) => arrow.span.start,
        }
    }

    fn params(self) -> &'n FormalParameters<'a> {
        match self {
            Self::Function(function) => &function.params,
            Self::Arrow(arrow) => &arrow.params,
        }
    }

    fn has_return_type(self) -> bool {
        match self {
            Self::Function(function) => function.return_type.is_some(),
            Self::Arrow(arrow) => arrow.return_type.is_some(),
        }
    }

    fn has_type_parameters(self) -> bool {
        match self {
            Self::Function(function) => function.type_parameters.is_some(),
            Self::Arrow(arrow) => arrow.type_parameters.is_some(),
        }
    }

    fn has_this_parameter(self) -> bool {
        match self {
            Self::Function(function) => function.this_param.is_some(),
            Self::Arrow(_) => false,
        }
    }

    /// tsc's `containsArgumentsReference`: the body names `arguments` outside
    /// any nested function.
    fn reads_arguments(self) -> bool {
        struct Arguments(bool);
        impl<'a> Visit<'a> for Arguments {
            fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
                self.0 |= it.name == "arguments";
            }
            fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
            fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
        }
        let mut visitor = Arguments(false);
        match self {
            Self::Function(function) => {
                if let Some(body) = &function.body {
                    visitor.visit_function_body(body);
                }
            }
            Self::Arrow(arrow) => visitor.visit_function_body(&arrow.body),
        }
        visitor.0
    }
}

/// Whether some call signature of a written type takes `required`
/// arguments (`isAritySmaller`); `None` when the type's signatures are not
/// written out.
fn signature_takes(ty: &ParsedType, required: usize) -> Option<bool> {
    let fits = |function: &crate::ParsedFunctionType| {
        let parameters = function.parameters.iter().filter(|parameter| !parameter.is_this);
        let (count, rest) = parameters.fold((0, false), |(count, rest), parameter| (count + 1, rest || parameter.rest));
        rest || count >= required
    };
    match ty {
        ParsedType::Function(function) => Some(fits(function)),
        ParsedType::Object(object) => match &object.call_signature {
            Some(signature) => Some(fits(signature)),
            None => Some(false),
        },
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Undefined
        | ParsedType::Null
        | ParsedType::Void
        | ParsedType::Never
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_)
        | ParsedType::Array(_)
        | ParsedType::Tuple(_) => Some(false),
        ParsedType::Named(named) if is_jsdoc_function_type(named) => Some(false),
        _ => None,
    }
}

/// `getIntendedTypeFromJSDocTypeReference`: JSDoc's `Function` (or
/// `function`) is the global `Function` whatever else is in scope, and that
/// declares no call signature.
fn is_jsdoc_function_type(named: &crate::ParsedNamedType) -> bool {
    matches!(named.name.as_str(), "Function" | "function")
}

/// `getSingleCallSignature` of a written type: a function type, or a type
/// literal whose one member is a call signature.
fn single_call_signature(ty: &ParsedType) -> Option<&ParsedFunctionType> {
    match ty {
        ParsedType::Function(function) => Some(function.as_ref()),
        ParsedType::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature_overloads.is_empty()
                && object.construct_signature.is_none()
                && object.construct_signature_overloads.is_empty() =>
        {
            object.call_signature.as_deref()
        }
        _ => None,
    }
}

fn function_of_expression<'n, 'a>(expression: &'n Expression<'a>) -> Option<FunctionHost<'n, 'a>> {
    match expression {
        Expression::FunctionExpression(function) => Some(FunctionHost::Function(function)),
        Expression::ArrowFunctionExpression(arrow) => Some(FunctionHost::Arrow(arrow)),
        Expression::TSSatisfiesExpression(satisfies) => function_of_expression(&satisfies.expression),
        _ => None,
    }
}

/// `GetRightMostAssignedExpression`.
fn right_most_assigned_expression<'n, 'a>(mut expression: &'n Expression<'a>) -> &'n Expression<'a> {
    while let Expression::AssignmentExpression(assignment) = expression {
        if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
            break;
        }
        expression = &assignment.right;
    }
    expression
}

impl<'s, 'c> IndexBuilder<'s, 'c> {
    /// The JSDoc comments attached to the node whose first token starts at
    /// `start` (`GetJSDocCommentRanges`): the leading comments after the line
    /// break that follows the previous token, and for `trailing` kinds also
    /// the comments on that token's line.
    fn jsdoc_comments(&mut self, start: u32, trailing: bool) -> Vec<std::rc::Rc<JsDocComment>> {
        let bytes = self.source_text.as_bytes();
        let upper = self.comments.partition_point(|comment| comment.span.end <= start);
        let mut pos = start as usize;
        let mut first = upper;
        loop {
            while pos > 0 && matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                pos -= 1;
            }
            if first > 0 && self.comments[first - 1].span.end as usize == pos {
                first -= 1;
                pos = self.comments[first].span.start as usize;
                continue;
            }
            break;
        }
        let previous_token_end = pos;
        let mut result = Vec::new();
        for comment in &self.comments[first..upper] {
            let comment_start = comment.span.start as usize;
            let on_previous_line = previous_token_end > 0
                && !self.source_text[previous_token_end..comment_start].contains(['\n', '\r']);
            if on_previous_line && !trailing {
                continue;
            }
            if !comment.is_block() {
                continue;
            }
            let key = comment.span.start;
            if let Some(parsed) = self.parsed.get(&key) {
                result.push(parsed.clone());
                continue;
            }
            let Some(parsed) =
                parse_jsdoc_comment(self.source_text, comment_start, comment.span.end as usize)
            else {
                continue;
            };
            let parsed = std::rc::Rc::new(parsed);
            self.parsed.insert(key, parsed.clone());
            self.index.diagnostics.extend(parsed.diagnostics.iter().cloned());
            self.reparse_unhosted(key, &parsed);
            result.push(parsed);
        }
        result
    }

    fn reparse_unhosted(&mut self, key: u32, comment: &JsDocComment) {
        if !self.unhosted_done.insert(key) {
            return;
        }
        self.check_template_parameters(comment);
        for tag in &comment.tags {
            match tag {
                JsDocTag::Typedef { name, name_span, type_expression: Some(type_expression), .. } => {
                    check_non_identifier_name(name, *name_span, &mut self.index.parse_errors);
                    if name.last().is_none_or(|name| name.is_empty()) {
                        continue;
                    }
                    // `@typedef {T} A.B` declares `B` in a namespace `A`, which
                    // the type table keys by the qualified name.
                    let ty = type_of_expression(type_expression).unwrap_or(ParsedType::Unknown);
                    self.alias_comment_starts.push(key);
                    self.index.aliases.push(ParsedTypeAliasDeclaration {
                        is_declare: false,
                        name: name.join("."),
                        name_span: Some(*name_span),
                        type_parameters: template_parameters(&comment.tags, true),
                        ty,
                        type_span: Some(type_span_of_expression(type_expression)),
                        enum_name: None,
                        enum_exported: false,
                        enum_is_const: false,
                        enum_members: Vec::new(),
                    });
                }
                JsDocTag::Import(import) => self.index.imports.push(import.clone()),
                JsDocTag::Callback { name, name_span, signature, span } => {
                    check_non_identifier_name(name, *name_span, &mut self.index.parse_errors);
                    if name.last().is_none_or(|name| name.is_empty()) {
                        continue;
                    }
                    // Namespaced like a `@typedef` (`wrapInJSDocNamespace`).
                    self.alias_comment_starts.push(key);
                    self.index.aliases.push(ParsedTypeAliasDeclaration {
                        is_declare: false,
                        name: name.join("."),
                        name_span: Some(*name_span),
                        type_parameters: template_parameters(&comment.tags, true),
                        ty: signature_type(signature, Vec::new()),
                        type_span: Some(*span),
                        enum_name: None,
                        enum_exported: false,
                        enum_is_const: false,
                        enum_members: Vec::new(),
                    });
                }
                _ => {}
            }
        }
    }

    /// tsc's `checkTypeParameters` over a comment's `@template` list: a
    /// required parameter after a defaulted one (TS2706), and a default naming
    /// itself or a later parameter (TS2744).
    fn check_template_parameters(&mut self, comment: &JsDocComment) {
        let parameters: Vec<&JsDocTemplateParameter> = comment
            .tags
            .iter()
            .filter_map(|tag| match tag {
                JsDocTag::Template { parameters, .. } => Some(parameters.iter()),
                _ => None,
            })
            .flatten()
            .collect();
        let mut defaulted = false;
        for (index, parameter) in parameters.iter().enumerate() {
            let Some(default) = &parameter.default_type else {
                if defaulted {
                    self.index.diagnostics.push(crate::ParsedGrammarDiagnostic {
                        kind: crate::ParsedGrammarDiagnosticKind::RequiredTypeParameterAfterOptional,
                        span: parameter.name_span,
                        name: None,
                    });
                }
                continue;
            };
            defaulted = true;
            let later: Vec<&str> = parameters[index..].iter().map(|later| later.name.as_str()).collect();
            let mut references = Vec::new();
            if let Some(ty) = &default.ty {
                named_type_references(ty, &later, &mut references);
            }
            for span in references {
                self.index.diagnostics.push(crate::ParsedGrammarDiagnostic {
                    kind: crate::ParsedGrammarDiagnosticKind::Ts(2744),
                    span,
                    name: None,
                });
            }
        }
    }

    /// `reparseHosted` for the function-like tags of `comment` on `host`.
    fn host_function(&mut self, host: FunctionHost<'_, '_>, comment: &JsDocComment) {
        self.host_function_tags(host, comment, false);
    }

    /// `reparseHosted` over a function's tags in order. For a host that can
    /// take its `@type` as a full signature, that tag claims the signature
    /// only when nothing of it is typed yet, and once claimed the later
    /// `@param`, `@return` and `@template` tags are ignored.
    fn host_function_tags(&mut self, host: FunctionHost<'_, '_>, comment: &JsDocComment, full_signature_host: bool) {
        let start = host.start();
        let mut parameter_tag_index = 0usize;
        let mut full_signature = false;
        for tag in &comment.tags {
            match tag {
                JsDocTag::Type(Some(ty))
                    if full_signature_host
                        && !full_signature
                        && ty.ty.is_some()
                        && !host.has_type_parameters()
                        && !self.index.type_parameters.contains_key(&start)
                        && !host.has_return_type()
                        && !self.index.returns.contains_key(&start)
                        && !self.has_typed_parameter(host) =>
                {
                    full_signature = true;
                    self.index.full_signature_hosts.insert(start);
                }
                JsDocTag::Parameter(parameter_tag) => {
                    let tag_index = parameter_tag_index;
                    parameter_tag_index += 1;
                    let matched = matching_parameter(host.params(), parameter_tag, tag_index);
                    if matched.is_none() {
                        self.report_unmatched_parameter_tag(host, parameter_tag);
                    }
                    if full_signature {
                        continue;
                    }
                    if let Some(parameter_start) = matched {
                        let entry = self.index.parameters.entry(parameter_start).or_default();
                        if entry.ty.is_none() {
                            if let Some(type_expression) = &parameter_tag.type_expression {
                                if let Some(ty) = type_of_expression(type_expression) {
                                    entry.ty = Some((ty, type_span_of_expression(type_expression)));
                                }
                            }
                        }
                        if is_optional(parameter_tag) {
                            entry.optional = true;
                            entry.question.get_or_insert(TextSpan {
                                start: parameter_tag.start,
                                end: parameter_tag.start + 1,
                            });
                        }
                        if entry.ty.is_some() {
                            self.mark_typed(host);
                        }
                    }
                }
                JsDocTag::Return(Some(ty)) if !host.has_return_type() && !full_signature => {
                    if let Some(parsed) = &ty.ty {
                        self.index.returns.entry(start).or_insert((parsed.clone(), ty.span));
                    }
                }
                JsDocTag::This(Some(ty)) if !host.has_this_parameter() => {
                    if let Some(parsed) = &ty.ty {
                        self.index.this_types.entry(start).or_insert(parsed.clone());
                    }
                }
                // An overload's own `@template`s do not scope its types.
                JsDocTag::Overload { signature, span } => self.record_overload(start, signature, *span),
                JsDocTag::Template { .. } if !host.has_type_parameters() && !full_signature => {
                    let parameters = template_parameters(&comment.tags, false);
                    if !parameters.is_empty() {
                        self.index.type_parameters.entry(start).or_insert(parameters);
                    }
                }
                _ => {}
            }
        }
    }

    /// `reparseHosted` gives a function's own `@type` to it as its full
    /// signature when nothing of the signature is written, and
    /// `checkFunctionOrMethodDeclaration` reports TS8030 on that type when no
    /// call signature of it takes the function's required parameters
    /// (`getContextualCallSignature`).
    /// Whether a parameter is typed by its annotation, its own `@type`
    /// comment, or a `@param` tag already hosted.
    fn record_overload(&mut self, function_start: u32, signature: &JsDocSignature, span: TextSpan) {
        if signature.return_type.is_none() && !self.hosting_constructor {
            self.index.diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::ImplicitAnyOverloadReturn,
                span,
                name: None,
            });
        }
        self.index
            .overloads
            .entry(function_start)
            .or_default()
            .push((signature_type(signature, Vec::new()), span));
    }

    fn has_typed_parameter(&mut self, host: FunctionHost<'_, '_>) -> bool {
        let params = host.params();
        let starts: Vec<(u32, bool)> = params
            .items
            .iter()
            .map(|parameter| (parameter.span.start, parameter.type_annotation.is_some()))
            .chain(params.rest.iter().map(|rest| (rest.span.start, rest.type_annotation.is_some())))
            .collect();
        starts.into_iter().any(|(start, annotated)| {
            annotated
                || self.index.parameters.get(&start).is_some_and(|entry| entry.ty.is_some())
                || self.last_comment(start, true).is_some_and(|comment| Self::type_tag(&comment).is_some())
        })
    }

    fn check_full_signature(&mut self, host: FunctionHost<'_, '_>, comment: &JsDocComment) {
        if !self.index.full_signature_hosts.contains(&host.start()) {
            return;
        }
        let Some(tag) = Self::type_tag(comment) else {
            return;
        };
        let params = host.params();
        if host.has_type_parameters()
            || host.has_return_type()
            || params.items.iter().any(|parameter| parameter.type_annotation.is_some())
            || params.rest.as_ref().is_some_and(|rest| rest.type_annotation.is_some())
        {
            return;
        }
        let required = params
            .items
            .iter()
            .take_while(|parameter| {
                !parameter.optional
                    && parameter.initializer.is_none()
                    && !self.index.parameters.get(&parameter.span.start).is_some_and(|entry| entry.optional)
            })
            .count();
        if tag.ty.as_ref().and_then(|ty| signature_takes(ty, required)) == Some(false) {
            self.index.diagnostics.push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(8030),
                span: tag.span,
                name: None,
            });
        }
        // `getThisTypeOfSignature` of the full signature: its `this` parameter
        // is the function's, unless a `@this` tag already gave one.
        let signature = match tag.ty.as_ref() {
            Some(ParsedType::Function(signature)) => Some(signature.as_ref()),
            Some(ParsedType::Object(object)) => object.call_signature.as_deref(),
            _ => None,
        };
        let this = signature.and_then(|signature| signature.parameters.iter().find(|parameter| parameter.is_this));
        if let Some(this) = this {
            self.index.this_types.entry(host.start()).or_insert(this.ty.clone());
        } else if tag.ty.as_ref().is_some_and(|ty| signature_takes(ty, 0).is_none()) {
            self.index.opaque_full_signatures.insert(host.start());
        }
        let applied = tag
            .ty
            .as_ref()
            .and_then(single_call_signature)
            .is_some_and(|signature| self.apply_full_signature(host, signature, tag.span));
        if !applied && let Some(ty) = &tag.ty {
            self.index.full_signatures.entry(host.start()).or_insert_with(|| ty.clone());
        }
    }

    /// `getParameterTypeOfFullSignature` and `getReturnTypeOfFullSignature`:
    /// each parameter takes the type of the full signature's parameter at its
    /// position, and the return type is the signature's. Callers see the full
    /// signature itself (`getSignaturesOfSymbol`), which the declaration's own
    /// signature reproduces only when the parameters pair one to one; a
    /// generic or rest signature, or one of another length, is left to the
    /// declaration. So is a signature the function's own tags already type,
    /// which the reparser would not have made its full signature.
    fn apply_full_signature(
        &mut self,
        host: FunctionHost<'_, '_>,
        signature: &ParsedFunctionType,
        span: TextSpan,
    ) -> bool {
        let params = host.params();
        let start = host.start();
        let parameters: Vec<&ParsedFunctionTypeParameter> =
            signature.parameters.iter().filter(|parameter| !parameter.is_this).collect();
        if !signature.type_parameters.is_empty()
            || params.rest.is_some()
            || params.items.len() != parameters.len()
            || parameters.iter().any(|parameter| parameter.rest)
            || self.index.returns.contains_key(&start)
            || self.index.type_parameters.contains_key(&start)
            || params.items.iter().any(|parameter| {
                self.index.parameters.get(&parameter.span.start).is_some_and(|entry| entry.ty.is_some())
            })
        {
            return false;
        }
        for (parameter, typed) in params.items.iter().zip(parameters) {
            let entry = self.index.parameters.entry(parameter.span.start).or_default();
            entry.ty = Some((typed.ty.clone(), span));
            entry.optional |= typed.optional;
        }
        self.index.returns.insert(start, ((*signature.return_type).clone(), span));
        self.mark_typed(host);
        true
    }

    /// tsc's `checkUnmatchedJSDocParameters` for a `@param` naming no
    /// parameter: TS8032 for a qualified name, TS8024 for a plain one written
    /// type first. A function that reads `arguments` may take its parameters
    /// that way.
    fn report_unmatched_parameter_tag(&mut self, host: FunctionHost<'_, '_>, tag: &JsDocParameterTag) {
        if tag.name.first().is_none_or(|name| name.is_empty()) || host.reads_arguments() {
            return;
        }
        let (code, arguments) = match tag.name.as_slice() {
            [name] if !tag.name_first => (8024, name.clone()),
            [_] => return,
            [.., _] => {
                let left = tag.name[..tag.name.len() - 1].join(".");
                (8032, format!("{}\0{left}", tag.name.join(".")))
            }
            [] => return,
        };
        self.index.diagnostics.push(crate::ParsedGrammarDiagnostic {
            kind: crate::ParsedGrammarDiagnosticKind::Ts(code),
            span: tag.name_span,
            name: Some(arguments),
        });
    }

    fn mark_typed(&mut self, host: FunctionHost<'_, '_>) {
        let params = host.params();
        for parameter in &params.items {
            self.index.typed_signature_parameters.insert(parameter.span.start);
        }
        if let Some(rest) = &params.rest {
            self.index.typed_signature_parameters.insert(rest.span.start);
        }
    }

    fn last_comment(&mut self, start: u32, trailing: bool) -> Option<std::rc::Rc<JsDocComment>> {
        self.jsdoc_comments(start, trailing).pop()
    }

    fn type_tag(comment: &JsDocComment) -> Option<&JsDocType> {
        comment.tags.iter().find_map(|tag| match tag {
            JsDocTag::Type(Some(ty)) if ty.ty.is_some() => Some(ty),
            _ => None,
        })
    }

    fn satisfies_tag(comment: &JsDocComment) -> Option<&JsDocType> {
        comment.tags.iter().find_map(|tag| match tag {
            JsDocTag::Satisfies(Some(ty)) if ty.ty.is_some() => Some(ty),
            _ => None,
        })
    }

    /// `reparseHosted` for the modifier tags, on a class member or a
    /// `this.x = e` assignment.
    fn host_modifiers(&mut self, start: u32, comment: &JsDocComment) {
        let modifiers: Vec<(JsDocModifier, TextSpan)> = comment
            .tags
            .iter()
            .filter_map(|tag| match tag {
                JsDocTag::Modifier(modifier, span) => Some((*modifier, *span)),
                _ => None,
            })
            .collect();
        if !modifiers.is_empty() {
            self.index.member_modifiers.entry(start).or_insert(modifiers);
        }
    }

    fn declare(&mut self, start: u32, ty: &JsDocType) {
        self.index.declared.entry(start).or_insert((lowered_with_optionality(ty), ty.span));
    }
}

/// `findMatchingParameter`: the parameter a `@param` names, or the one at its
/// position when the parameter is a pattern or the tag has no name.
fn matching_parameter(params: &FormalParameters<'_>, tag: &JsDocParameterTag, tag_index: usize) -> Option<u32> {
    if tag.name.len() > 1 {
        return None;
    }
    let tag_name = tag.name.first().map(String::as_str).unwrap_or("");
    let mut all: Vec<(u32, Option<&str>)> = params
        .items
        .iter()
        .map(|parameter| (parameter.span.start, binding_identifier(&parameter.pattern)))
        .collect();
    if let Some(rest) = &params.rest {
        all.push((rest.span.start, binding_identifier(&rest.rest.argument)));
    }
    for (index, (start, name)) in all.into_iter().enumerate() {
        match name {
            Some(name) => {
                if name == tag_name || (index == tag_index && tag_name.is_empty()) {
                    return Some(start);
                }
            }
            None if index == tag_index => return Some(start),
            None => {}
        }
    }
    None
}

fn binding_identifier<'a>(pattern: &'a BindingPattern<'_>) -> Option<&'a str> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.as_str()),
        _ => None,
    }
}

impl<'a> Visit<'a> for IndexBuilder<'_, '_> {
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            self.host_variable_statement(it, &comment);
        }
        walk::walk_variable_declaration(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if it.type_annotation.is_none() {
            if let Some(comment) = self.last_comment(it.span.start, true) {
                if let Some(ty) = Self::type_tag(&comment) {
                    let ty = ty.clone();
                    self.declare(it.span.start, &ty);
                }
            }
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        // A declaration's JSDoc precedes its statement (`visit_statement`).
        if it.is_expression() {
            if let Some(comment) = self.last_comment(it.span.start, true) {
                self.host_function(FunctionHost::Function(it), &comment);
            }
        }
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, true) {
            self.host_function(FunctionHost::Arrow(it), &comment);
        }
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.type_annotation.is_none() {
            if let Some(comment) = self.last_comment(it.span.start, true) {
                if let Some(ty) = Self::type_tag(&comment) {
                    let entry = self.index.parameters.entry(it.span.start).or_default();
                    if entry.ty.is_none() {
                        entry.ty = Some((lowered(ty), ty.span));
                    }
                }
            }
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            self.host_modifiers(it.span.start, &comment);
            self.hosting_constructor = it.kind == MethodDefinitionKind::Constructor;
            self.host_function_tags(
                FunctionHost::Function(&it.value),
                &comment,
                it.kind == MethodDefinitionKind::Method,
            );
            self.hosting_constructor = false;
            // An accessor or a constructor keeps the full signature, but its
            // check never reads it (`checkAccessorDeclaration`,
            // `checkConstructorDeclaration`).
            if it.kind == MethodDefinitionKind::Method {
                self.check_full_signature(FunctionHost::Function(&it.value), &comment);
            }
            if it.kind == MethodDefinitionKind::Get && it.value.return_type.is_none() {
                if let Some(ty) = Self::type_tag(&comment) {
                    let parsed = lowered_with_optionality(ty);
                    self.index.returns.entry(it.value.span.start).or_insert((parsed, ty.span));
                }
            }
        }
        walk::walk_method_definition(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            self.host_modifiers(it.span.start, &comment);
            if it.type_annotation.is_none() {
                if let Some(ty) = Self::type_tag(&comment) {
                    let ty = ty.clone();
                    self.declare(it.span.start, &ty);
                }
            }
            if let Some(host) = it.value.as_ref().and_then(function_of_expression) {
                self.host_function(host, &comment);
            }
        }
        walk::walk_property_definition(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            if !it.shorthand {
                if let Some(host) = function_of_expression(&it.value) {
                    self.host_function_tags(host, &comment, it.method);
                    // A method is its own host; a property's `@type` types the
                    // property, and an accessor's is never checked.
                    if it.method {
                        self.check_full_signature(host, &comment);
                    }
                }
            }
            if !it.method
                && it.kind == oxc_ast::ast::PropertyKind::Init
                && let Some(ty) = Self::type_tag(&comment)
            {
                let value = it.value.span();
                self.index
                    .assignment_types
                    .entry((value.start, value.end))
                    .or_insert((lowered_with_optionality(ty), ty.span, super::spans::text_span_from_oxc_span(it.key.span())));
            }
        }
        walk::walk_object_property(self, it);
    }

    fn visit_parenthesized_expression(&mut self, it: &ParenthesizedExpression<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, true) {
            if let Some(ty) = Self::type_tag(&comment) {
                self.index.casts.entry(it.span.start).or_insert((lowered_with_optionality(ty), ty.span, true));
            } else if let Some(ty) = Self::satisfies_tag(&comment) {
                self.index.casts.entry(it.span.start).or_insert((lowered_with_optionality(ty), ty.span, false));
            }
        }
        walk::walk_parenthesized_expression(self, it);
    }

    fn visit_expression_statement(&mut self, it: &ExpressionStatement<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            if let Expression::AssignmentExpression(assignment) = &it.expression {
                self.host_modifiers(assignment.span.start, &comment);
            }
            // `reparseHosted`: an assignment declaration (`o.x = e`,
            // `this.x = e`, `exports.x = e`) takes the `@type` as its type.
            if let Some(ty) = Self::type_tag(&comment)
                && let Expression::AssignmentExpression(assignment) = &it.expression
                && assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign
                && assignment.left.as_member_expression().is_some()
            {
                let value = assignment.right.span();
                let ty = ty.clone();
                self.index
                    .assignment_types
                    .entry((value.start, value.end))
                    .or_insert((lowered_with_optionality(&ty), ty.span, super::spans::text_span_from_oxc_span(assignment.left.span())));
            }
            if let Some(host) = function_of_expression(right_most_assigned_expression(&it.expression)) {
                self.host_function(host, &comment);
            }
        }
        walk::walk_expression_statement(self, it);
    }

    fn visit_return_statement(&mut self, it: &ReturnStatement<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            if it.argument.is_some() {
                if let Some(ty) = Self::type_tag(&comment) {
                    self.index.return_casts.entry(it.span.start).or_insert((lowered_with_optionality(ty), ty.span, true));
                } else if let Some(ty) = Self::satisfies_tag(&comment) {
                    self.index.return_casts.entry(it.span.start).or_insert((lowered_with_optionality(ty), ty.span, false));
                }
            }
            if let Some(host) = it.argument.as_ref().and_then(function_of_expression) {
                self.host_function(host, &comment);
            }
        }
        walk::walk_return_statement(self, it);
    }

    fn visit_catch_parameter(&mut self, it: &oxc_ast::ast::CatchParameter<'a>) {
        if it.type_annotation.is_none() {
            if let Some(comment) = self.last_comment(it.span.start, true) {
                if let Some(ty) = Self::type_tag(&comment) {
                    let ty = ty.clone();
                    self.declare(it.span.start, &ty);
                }
            }
        }
        walk::walk_catch_parameter(self, it);
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        if let Some(comment) = self.last_comment(it.span.start, false) {
            match &it.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    self.host_function_tags(FunctionHost::Function(function), &comment, true);
                    self.check_full_signature(FunctionHost::Function(function), &comment);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => self.host_class(class, &comment),
                other => {
                    if let Some(host) = other.as_expression().and_then(function_of_expression) {
                        self.host_function(host, &comment);
                    }
                    // `reparseHosted` types an export assignment by its `@type`,
                    // and `checkExportAssignment` relates the expression to it.
                    if let Some(expression) = other.as_expression()
                        && let Some(ty) = Self::type_tag(&comment)
                    {
                        let value = expression.span();
                        self.index
                            .assignment_types
                            .entry((value.start, value.end))
                            .or_insert((lowered_with_optionality(ty), ty.span, super::spans::text_span_from_oxc_span(value)));
                    }
                }
            }
        }
        walk::walk_export_default_declaration(self, it);
    }

    fn visit_statement(&mut self, it: &oxc_ast::ast::Statement<'a>) {
        use oxc_ast::ast::{Declaration, Statement};
        let (start, declaration) = match it {
            Statement::ExportNamedDeclaration(export) => (export.span.start, export.declaration.as_ref()),
            _ => (it.span().start, it.as_declaration()),
        };
        match declaration {
            Some(Declaration::FunctionDeclaration(function)) => {
                // Every comment before the declaration may write overloads;
                // the last one's are hosted with the rest of its tags.
                let comments = self.jsdoc_comments(start, false);
                for comment in comments.iter().take(comments.len().saturating_sub(1)) {
                    for tag in &comment.tags {
                        if let JsDocTag::Overload { signature, span } = tag {
                            self.record_overload(function.span.start, signature, *span);
                        }
                    }
                }
                if let Some(comment) = self.last_comment(start, false) {
                    self.host_function_tags(FunctionHost::Function(function), &comment, true);
                    self.check_full_signature(FunctionHost::Function(function), &comment);
                }
            }
            Some(Declaration::ClassDeclaration(class)) => {
                if let Some(comment) = self.last_comment(start, false) {
                    self.host_class(class, &comment);
                }
            }
            Some(Declaration::VariableDeclaration(declaration)) if start != declaration.span.start => {
                if let Some(comment) = self.last_comment(start, false) {
                    self.host_variable_statement(declaration, &comment);
                }
            }
            _ => {}
        }
        walk::walk_statement(self, it);
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if it.is_expression() {
            if let Some(comment) = self.last_comment(it.span.start, false) {
                self.host_class(it, &comment);
            }
        }
        walk::walk_class(self, it);
    }

    fn visit_function_body(&mut self, it: &oxc_ast::ast::FunctionBody<'a>) {
        self.record_statement_list(it.span, &it.statements);
        walk::walk_function_body(self, it);
    }

    fn visit_block_statement(&mut self, it: &oxc_ast::ast::BlockStatement<'a>) {
        self.record_statement_list(it.span, &it.body);
        walk::walk_block_statement(self, it);
    }

    fn visit_static_block(&mut self, it: &oxc_ast::ast::StaticBlock<'a>) {
        self.record_statement_list(it.span, &it.body);
        walk::walk_static_block(self, it);
    }

    fn visit_switch_case(&mut self, it: &oxc_ast::ast::SwitchCase<'a>) {
        self.record_statement_list(it.span, &it.consequent);
        walk::walk_switch_case(self, it);
    }

    fn visit_program(&mut self, it: &Program<'a>) {
        walk::walk_program(self, it);
        // Every JSDoc comment declares its aliases, attached to a node or not.
        let comments: Vec<(u32, u32)> = self
            .comments
            .iter()
            .filter(|comment| comment.is_block())
            .map(|comment| (comment.span.start, comment.span.end))
            .collect();
        // tsc parses a comment after the last statement as the end-of-file
        // token's JSDoc, so its parse errors are the file's.
        let code_end = it.body.last().map_or(0, |statement| statement.span().end);
        for (start, end) in comments {
            if self.parsed.contains_key(&start) {
                continue;
            }
            if let Some(parsed) = parse_jsdoc_comment(self.source_text, start as usize, end as usize) {
                if start >= code_end {
                    self.index.diagnostics.extend(parsed.diagnostics.iter().cloned());
                }
                self.reparse_unhosted(start, &parsed);
            }
        }
    }
}

impl IndexBuilder<'_, '_> {
    fn record_statement_list(&mut self, span: oxc_span::Span, statements: &[oxc_ast::ast::Statement<'_>]) {
        if let Some(first) = statements.first() {
            self.statement_lists.push((span.start, span.end, first.span().start));
        }
    }

    fn host_class(&mut self, class: &Class<'_>, comment: &JsDocComment) {
        // `reparseHosted` for `@augments`: the tag's type arguments fill an
        // `extends` clause that names the same class and writes none.
        if class.super_type_arguments.is_none()
            && let Some(super_class) = &class.super_class
            && let Some((base, _)) = super::types::flatten_heritage_expression(super_class)
        {
            for tag in &comment.tags {
                if let JsDocTag::Augments(Some(JsDocType { ty: Some(ParsedType::Named(named)), .. })) = tag
                    && named.name == base
                    && !named.type_arguments.is_empty()
                {
                    self.index
                        .extends_type_arguments
                        .entry(class.span.start)
                        .or_insert_with(|| (named.type_arguments.clone(), named.span));
                }
            }
        }
        // `getImplementsTypeNodes` reads `@implements` tags when the class
        // writes no `implements` clause.
        if class.implements.is_empty() {
            for tag in &comment.tags {
                if let JsDocTag::Implements(Some(JsDocType { ty: Some(ParsedType::Named(named)), .. })) = tag {
                    self.index
                        .implements
                        .entry(class.span.start)
                        .or_default()
                        .push((**named).clone());
                }
            }
        }
        if class.type_parameters.is_some() {
            return;
        }
        if comment.tags.iter().any(|tag| matches!(tag, JsDocTag::Template { .. })) {
            let parameters = template_parameters(&comment.tags, false);
            if !parameters.is_empty() {
                self.index.type_parameters.entry(class.span.start).or_insert(parameters);
            }
        }
    }

    /// `reparseHosted` on a variable statement: `@type` types the first
    /// declaration without an annotation, `@satisfies` checks the first
    /// initializer, and the function tags reach the first declaration's
    /// function initializer.
    fn host_variable_statement(&mut self, statement: &VariableDeclaration<'_>, comment: &JsDocComment) {
        if let Some(ty) = Self::type_tag(comment) {
            if let Some(declarator) = statement.declarations.iter().find(|d| d.type_annotation.is_none()) {
                let ty = ty.clone();
                self.declare(declarator.span.start, &ty);
            }
        }
        if let Some(ty) = Self::satisfies_tag(comment) {
            if let Some(declarator) = statement.declarations.iter().find(|d| d.init.is_some()) {
                let ty = ty.clone();
                self.satisfies_initializer(declarator, &ty);
            }
        }
        if let Some(host) = statement.declarations.first().and_then(|d| d.init.as_ref()).and_then(function_of_expression) {
            self.host_function(host, comment);
        }
    }

    fn satisfies_initializer(&mut self, declarator: &VariableDeclarator<'_>, ty: &JsDocType) {
        let Some(init) = &declarator.init else {
            return;
        };
        self.index
            .initializer_satisfies
            .entry(init.span().start)
            .or_insert((lowered_with_optionality(ty), ty.span));
    }
}

