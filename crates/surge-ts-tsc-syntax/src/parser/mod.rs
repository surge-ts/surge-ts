//! typescript-go's `parser.Parser`. This file holds the machinery every
//! production shares (state, diagnostics, lookahead, lists, identifiers, the
//! token predicates, context flags, `checkJSSyntax`, the source file); the
//! productions themselves are in `statements`, `types` and `expressions`, each
//! following the Go file's order and names (`parseFooBar` is `parse_foo_bar`).

use std::collections::HashSet;

use crate::ast::{self, Node, NodeId, NodeList, OperatorPrecedence};
use crate::flags::NodeFlags;
use crate::kind::Kind;
use crate::messages as diagnostics;
use crate::scanner::{self, Scanner, ScannerState, token_is_identifier_or_keyword, token_to_string};
use crate::{Diagnostic, Message, ParseOptions, ScriptKind};

mod expressions;
mod statements;
mod types;

pub(crate) use statements::get_spelling_suggestion_for_strings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PC {
    SourceElements,
    BlockStatements,
    SwitchClauses,
    SwitchClauseStatements,
    TypeMembers,
    ClassMembers,
    EnumMembers,
    HeritageClauseElement,
    VariableDeclarations,
    ObjectBindingElements,
    ArrayBindingElements,
    ArgumentExpressions,
    ObjectLiteralMembers,
    JsxAttributes,
    JsxChildren,
    ArrayLiteralMembers,
    Parameters,
    JSDocParameters,
    RestProperties,
    TypeParameters,
    TypeArguments,
    TupleElementTypes,
    HeritageClauses,
    ImportOrExportSpecifiers,
    ImportAttributes,
    JSDocComment,
}

const PC_ALL: [PC; 26] = [
    PC::SourceElements,
    PC::BlockStatements,
    PC::SwitchClauses,
    PC::SwitchClauseStatements,
    PC::TypeMembers,
    PC::ClassMembers,
    PC::EnumMembers,
    PC::HeritageClauseElement,
    PC::VariableDeclarations,
    PC::ObjectBindingElements,
    PC::ArrayBindingElements,
    PC::ArgumentExpressions,
    PC::ObjectLiteralMembers,
    PC::JsxAttributes,
    PC::JsxChildren,
    PC::ArrayLiteralMembers,
    PC::Parameters,
    PC::JSDocParameters,
    PC::RestProperties,
    PC::TypeParameters,
    PC::TypeArguments,
    PC::TupleElementTypes,
    PC::HeritageClauses,
    PC::ImportOrExportSpecifiers,
    PC::ImportAttributes,
    PC::JSDocComment,
];

/// `ParseFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ParseFlags(pub u32);

#[allow(non_upper_case_globals, dead_code)]
impl ParseFlags {
    pub const None: ParseFlags = ParseFlags(0);
    pub const Yield: ParseFlags = ParseFlags(1 << 0);
    pub const Await: ParseFlags = ParseFlags(1 << 1);
    pub const Type: ParseFlags = ParseFlags(1 << 2);
    pub const IgnoreMissingOpenBrace: ParseFlags = ParseFlags(1 << 4);
    pub const JSDoc: ParseFlags = ParseFlags(1 << 5);

    pub fn has(self, other: ParseFlags) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for ParseFlags {
    type Output = ParseFlags;
    fn bitor(self, other: ParseFlags) -> ParseFlags {
        ParseFlags(self.0 | other.0)
    }
}

/// `core.Tristate`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tristate {
    False,
    True,
    Unknown,
}

#[derive(Clone)]
pub(crate) struct ParserState {
    scanner_state: ScannerState,
    context_flags: NodeFlags,
    diagnostics_len: usize,
    js_diagnostics_len: usize,
    statement_has_await_identifier: bool,
    has_parse_error: bool,
}

pub(crate) struct Parser<'a> {
    pub scanner: Scanner<'a>,
    pub nodes: Vec<Node>,
    pub source_text: &'a str,
    /// `languageVariant == LanguageVariantJSX`.
    pub jsx: bool,
    pub js_diagnostics: Vec<Diagnostic>,
    pub token: Kind,
    pub source_flags: NodeFlags,
    pub context_flags: NodeFlags,
    pub parsing_contexts: u32,
    pub statement_has_await_identifier: bool,
    pub not_parenthesized_arrow: HashSet<usize>,
    pub possible_await_spans: Vec<usize>,
    force_module: bool,
    jsx_forces_module: bool,
    is_declaration_file: bool,
}

/// A parsed file: its tree, rooted at a `SourceFile` node, and what the parser
/// reported.
pub(crate) struct ParsedFile {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub parents: Vec<Option<NodeId>>,
    pub diagnostics: Vec<Diagnostic>,
    pub js_diagnostics: Vec<Diagnostic>,
    /// `ast.IsExternalModule(file)`: the file has an `ExternalModuleIndicator`.
    pub external_module: bool,
    pub is_declaration_file: bool,
    pub jsx: bool,
}

impl ParsedFile {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents[id as usize]
    }

    /// `node.ForEachChild`: every child, in source order.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        child_ids(&self.nodes, id)
    }

    /// The full start of every regular expression literal in the tree, in
    /// source order.
    pub fn regular_expression_literals(&self) -> Vec<usize> {
        let mut starts: Vec<usize> = (0..self.nodes.len())
            .filter(|&id| {
                self.nodes[id].kind == Kind::RegularExpressionLiteral
                    && (self.parents[id].is_some() || id as NodeId == self.root)
            })
            .map(|id| self.nodes[id].pos)
            .collect();
        starts.sort_unstable();
        starts
    }
}

fn child_ids(nodes: &[Node], id: NodeId) -> Vec<NodeId> {
    let node = &nodes[id as usize];
    let mut children: Vec<NodeId> = Vec::new();
    let lists = [&node.modifiers, &node.type_parameters, &node.type_arguments, &node.parameters];
    for list in lists.into_iter().flatten() {
        children.extend(&list.nodes);
    }
    for list in node.lists.iter().flatten() {
        children.extend(&list.nodes);
    }
    children.extend(
        [node.name, node.ty, node.body, node.expression, node.initializer, node.question_token, node.import_clause]
            .into_iter()
            .flatten(),
    );
    children.extend(node.children.iter().flatten());
    children.sort_by_key(|&child| (nodes[child as usize].pos, nodes[child as usize].end, child));
    children.dedup();
    children
}

/// Parses `text`.
pub(crate) fn parse(text: &str, options: &ParseOptions) -> ParsedFile {
    let jsx = matches!(options.script_kind, ScriptKind::Tsx | ScriptKind::Jsx | ScriptKind::Js);
    let mut parser = Parser {
        scanner: Scanner::new(text, jsx),
        nodes: Vec::new(),
        source_text: text,
        jsx,
        js_diagnostics: Vec::new(),
        token: Kind::Unknown,
        source_flags: NodeFlags::None,
        context_flags: if matches!(options.script_kind, ScriptKind::Js | ScriptKind::Jsx) {
            NodeFlags::JavaScriptFile
        } else {
            NodeFlags::None
        },
        parsing_contexts: 0,
        statement_has_await_identifier: false,
        not_parenthesized_arrow: HashSet::new(),
        possible_await_spans: Vec::new(),
        force_module: options.force_module,
        jsx_forces_module: options.jsx_forces_module,
        is_declaration_file: options.is_declaration_file,
    };
    parser.next_token();
    let (statements, external_module) = parser.parse_source_file_worker();
    let mut root = Node::new(Kind::SourceFile).list(NodeList::new(0, text.len(), statements));
    root.pos = 0;
    root.end = text.len();
    root.flags = parser.context_flags;
    let root_id = parser.nodes.len() as NodeId;
    parser.nodes.push(root);
    let mut parents = vec![None; parser.nodes.len()];
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        for child in child_ids(&parser.nodes, id) {
            if parents[child as usize].is_none() && child != root_id {
                parents[child as usize] = Some(id);
                stack.push(child);
            }
        }
    }
    let diagnostics = std::mem::take(&mut parser.scanner.diagnostics);
    ParsedFile {
        nodes: parser.nodes,
        root: root_id,
        parents,
        diagnostics,
        js_diagnostics: parser.js_diagnostics,
        external_module,
        is_declaration_file: options.is_declaration_file,
        jsx,
    }
}

/// `parseJSONText`: a JSON file's value, recovered as tsc's parser recovers
/// it, then checked by `validateJsonValue`.
pub(crate) fn parse_json(text: &str) -> Vec<Diagnostic> {
    let mut parser = Parser {
        scanner: Scanner::new(text, false),
        nodes: Vec::new(),
        source_text: text,
        jsx: false,
        js_diagnostics: Vec::new(),
        token: Kind::Unknown,
        source_flags: NodeFlags::None,
        context_flags: NodeFlags::JavaScriptFile | NodeFlags::JsonFile,
        parsing_contexts: 0,
        statement_has_await_identifier: false,
        not_parenthesized_arrow: HashSet::new(),
        possible_await_spans: Vec::new(),
        force_module: false,
        jsx_forces_module: false,
        is_declaration_file: false,
    };
    parser.next_token();
    let mut expressions = Vec::new();
    while parser.token != Kind::EndOfFile {
        let start = parser.scanner.token_start();
        let expression = match parser.token {
            Kind::OpenBracketToken => parser.parse_array_literal_expression(),
            Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword => parser.parse_token_node(),
            Kind::MinusToken => {
                if parser.look_ahead(|p| p.next_token() == Kind::NumericLiteral && p.next_token() != Kind::ColonToken) {
                    parser.parse_prefix_unary_expression()
                } else {
                    parser.parse_object_literal_expression()
                }
            }
            Kind::NumericLiteral | Kind::StringLiteral => {
                if parser.look_ahead(|p| p.next_token() != Kind::ColonToken) {
                    parser.parse_literal_expression(false)
                } else {
                    parser.parse_object_literal_expression()
                }
            }
            _ => parser.parse_object_literal_expression(),
        };
        if expressions.is_empty() && parser.token != Kind::EndOfFile {
            parser.parse_error_at_current_token(diagnostics::Unexpected_token, &[]);
        }
        expressions.push(expression);
        if parser.token != Kind::EndOfFile && parser.scanner.token_start() == start {
            break;
        }
    }
    let mut reported = std::mem::take(&mut parser.scanner.diagnostics);
    for &expression in &expressions {
        parser.validate_json_value(expression, &mut reported);
    }
    reported
}

impl<'a> Parser<'a> {
    fn validate_json_value(&self, id: NodeId, reported: &mut Vec<Diagnostic>) {
        let node = self.node(id);
        let report = |node: &Node, message: &'static Message, reported: &mut Vec<Diagnostic>| {
            let start = scanner::skip_trivia(self.source_text, node.pos);
            reported.push(Diagnostic { start, end: node.end, message, args: Vec::new() });
        };
        let double_quoted = |node: &Node| {
            node.kind == Kind::StringLiteral && !node.token_flags.has(crate::flags::TokenFlags::SingleQuote)
        };
        match node.kind {
            Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword | Kind::NumericLiteral => {}
            Kind::StringLiteral => {
                if !double_quoted(node) {
                    report(node, diagnostics::String_literal_with_double_quotes_expected, reported);
                }
            }
            Kind::PrefixUnaryExpression
                if node.op == Kind::MinusToken
                    && node.expression.is_some_and(|operand| self.kind(operand) == Kind::NumericLiteral) => {}
            Kind::ObjectLiteralExpression => {
                let properties = node.lists.first().cloned().flatten().map(|list| list.nodes).unwrap_or_default();
                for property in properties {
                    let element = self.node(property);
                    if element.kind != Kind::PropertyAssignment {
                        report(element, diagnostics::Property_assignment_expected, reported);
                        continue;
                    }
                    if let Some(name) = element.name
                        && !double_quoted(self.node(name))
                    {
                        report(self.node(name), diagnostics::String_literal_with_double_quotes_expected, reported);
                    }
                    if let Some(initializer) = element.initializer {
                        self.validate_json_value(initializer, reported);
                    }
                }
            }
            Kind::ArrayLiteralExpression => {
                let elements = node.lists.first().cloned().flatten().map(|list| list.nodes).unwrap_or_default();
                for element in elements {
                    self.validate_json_value(element, reported);
                }
            }
            _ => report(
                node,
                diagnostics::Property_value_can_only_be_string_literal_numeric_literal_true_false_null_object_literal_or_array_literal,
                reported,
            ),
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    pub fn kind(&self, id: NodeId) -> Kind {
        self.nodes[id as usize].kind
    }


    // --- Diagnostics ------------------------------------------------------

    /// `parseErrorAtRange`. Returns whether the diagnostic was recorded (the
    /// Go function's non-nil result): none is at the position of the last one.
    pub fn parse_error_at_range(&mut self, pos: usize, end: usize, message: &'static Message, args: &[&str]) -> bool {
        let before = self.scanner.diagnostics.len();
        self.scanner.error_at_range(pos, end, message, args.iter().map(|arg| arg.to_string()).collect());
        self.scanner.diagnostics.len() > before
    }

    pub fn parse_error_at(&mut self, pos: usize, end: usize, message: &'static Message, args: &[&str]) -> bool {
        self.parse_error_at_range(pos, end, message, args)
    }

    pub fn parse_error_at_current_token(&mut self, message: &'static Message, args: &[&str]) -> bool {
        let (start, end) = (self.scanner.token_start(), self.scanner.token_end());
        self.parse_error_at_range(start, end, message, args)
    }


    // --- State ------------------------------------------------------------

    pub fn mark(&self) -> ParserState {
        ParserState {
            scanner_state: self.scanner.mark(),
            context_flags: self.context_flags,
            diagnostics_len: self.scanner.diagnostics.len(),
            js_diagnostics_len: self.js_diagnostics.len(),
            statement_has_await_identifier: self.statement_has_await_identifier,
            has_parse_error: self.scanner.has_parse_error,
        }
    }

    pub fn rewind(&mut self, state: ParserState) {
        self.scanner.rewind(state.scanner_state);
        self.token = self.scanner.token();
        self.context_flags = state.context_flags;
        self.scanner.diagnostics.truncate(state.diagnostics_len);
        self.js_diagnostics.truncate(state.js_diagnostics_len);
        self.statement_has_await_identifier = state.statement_has_await_identifier;
        self.scanner.has_parse_error = state.has_parse_error;
    }

    pub fn look_ahead<T>(&mut self, callback: impl FnOnce(&mut Self) -> T) -> T {
        let state = self.mark();
        let result = callback(self);
        self.rewind(state);
        result
    }

    pub fn next_token(&mut self) -> Kind {
        if ast::is_keyword(self.token) && (self.scanner.has_unicode_escape() || self.scanner.has_extended_unicode_escape()) {
            self.parse_error_at_current_token(diagnostics::Keywords_cannot_contain_escape_characters, &[]);
        }
        self.token = self.scanner.scan();
        self.token
    }

    pub fn next_token_without_check(&mut self) -> Kind {
        self.token = self.scanner.scan();
        self.token
    }

    pub fn node_pos(&self) -> usize {
        self.scanner.token_full_start()
    }

    pub fn has_preceding_line_break(&self) -> bool {
        self.scanner.has_preceding_line_break()
    }

    // --- Source file --------------------------------------------------------

    /// The file's statements, and whether it is an external module
    /// (`getExternalModuleIndicator`: a declaration file is only one by its
    /// statements).
    fn parse_source_file_worker(&mut self) -> (Vec<NodeId>, bool) {
        if self.is_declaration_file {
            self.context_flags |= NodeFlags::Ambient;
        }
        let statements = self.parse_list_index(PC::SourceElements, Self::parse_toplevel_statement);
        let probably_module = statements.iter().any(|&statement| self.is_an_external_module_indicator_node(statement))
            || self.source_flags.has(NodeFlags::PossiblyContainsImportMeta);
        let is_module = probably_module
            || !self.is_declaration_file
                && (self.jsx_forces_module && self.contains_jsx_tag(&statements) || self.force_module);
        if !self.is_declaration_file && is_module && !self.possible_await_spans.is_empty() {
            let statements = self.reparse_top_level_await(&statements);
            return (statements, is_module);
        }
        (statements, is_module)
    }

    fn is_an_external_module_indicator_node(&self, id: NodeId) -> bool {
        let node = self.node(id);
        node.modifier_nodes().iter().any(|&modifier| self.kind(modifier) == Kind::ExportKeyword)
            || node.kind == Kind::ImportEqualsDeclaration
                && node.children.iter().flatten().any(|&child| self.kind(child) == Kind::ExternalModuleReference)
            || matches!(node.kind, Kind::ImportDeclaration | Kind::ExportAssignment | Kind::ExportDeclaration)
    }

    fn parse_toplevel_statement(&mut self, i: usize) -> NodeId {
        self.statement_has_await_identifier = false;
        let statement = self.parse_statement();
        if self.statement_has_await_identifier && !self.node(statement).flags.has(NodeFlags::AwaitContext) {
            match self.possible_await_spans.last_mut() {
                Some(last) if *last == i => *last = i + 1,
                _ => {
                    self.possible_await_spans.push(i);
                    self.possible_await_spans.push(i + 1);
                }
            }
        }
        statement
    }

    /// `isFileModuleFromUsingJSXTag`.
    fn contains_jsx_tag(&self, statements: &[NodeId]) -> bool {
        let mut stack = statements.to_vec();
        while let Some(id) = stack.pop() {
            if matches!(
                self.node(id).kind,
                Kind::JsxOpeningElement | Kind::JsxSelfClosingElement | Kind::JsxFragment
            ) {
                return true;
            }
            stack.extend(child_ids(&self.nodes, id));
        }
        false
    }

    /// `reparseTopLevelAwait`: the statements that used `await` as an
    /// identifier are parsed again in an await context, and their diagnostics
    /// replace the first parse's for that range.
    fn reparse_top_level_await(&mut self, statements: &[NodeId]) -> Vec<NodeId> {
        let mut reparsed = Vec::new();
        let saved = std::mem::take(&mut self.scanner.diagnostics);
        let copy_range = |parser: &mut Self, from: usize, to: Option<usize>| {
            if let Some(start) = saved.iter().position(|d| d.start >= from) {
                let end = to.and_then(|to| saved[start..].iter().position(|d| d.start >= to)).map(|n| start + n);
                parser.scanner.diagnostics.extend_from_slice(&saved[start..end.unwrap_or(saved.len())]);
            }
        };
        let mut after_await_statement = 0;
        let mut i = 0;
        while i < self.possible_await_spans.len() {
            let next_await_statement = self.possible_await_spans[i];
            let prev_pos = self.node(statements[after_await_statement]).pos;
            let next_pos = self.node(statements[next_await_statement]).pos;
            reparsed.extend_from_slice(&statements[after_await_statement..next_await_statement]);
            copy_range(self, prev_pos, Some(next_pos));

            let mut state = self.mark();
            self.context_flags |= NodeFlags::AwaitContext;
            self.scanner.reset_pos(next_pos);
            self.next_token();

            after_await_statement = self.possible_await_spans[i + 1];
            while self.token != Kind::EndOfFile {
                let start_pos = self.scanner.token_full_start();
                let statement = self.parse_statement();
                reparsed.push(statement);
                if start_pos == self.scanner.token_full_start() {
                    self.next_token();
                }
                if after_await_statement < statements.len() {
                    let last_await_end = self.node(statements[after_await_statement - 1]).end;
                    let statement_end = self.node(statement).end;
                    if statement_end == last_await_end {
                        break;
                    }
                    if statement_end > last_await_end {
                        i += 2;
                        after_await_statement = if i < self.possible_await_spans.len() {
                            self.possible_await_spans[i + 1]
                        } else {
                            statements.len()
                        };
                    }
                }
            }
            state.diagnostics_len = self.scanner.diagnostics.len();
            self.rewind(state);
            i += 2;
        }
        if after_await_statement < statements.len() {
            let prev_pos = self.node(statements[after_await_statement]).pos;
            reparsed.extend_from_slice(&statements[after_await_statement..]);
            copy_range(self, prev_pos, None);
        }
        reparsed
    }

    // --- Lists --------------------------------------------------------------

    pub fn parse_list_index(&mut self, kind: PC, mut parse_element: impl FnMut(&mut Self, usize) -> NodeId) -> Vec<NodeId> {
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << kind as u32;
        let mut list = Vec::new();
        while !self.is_list_terminator(kind) {
            if self.is_list_element(kind, false) {
                let element = parse_element(self, list.len());
                list.push(element);
                continue;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        self.parsing_contexts = save_parsing_contexts;
        list
    }

    pub fn parse_list(&mut self, kind: PC, mut parse_element: impl FnMut(&mut Self) -> NodeId) -> NodeList {
        let pos = self.node_pos();
        let nodes = self.parse_list_index(kind, |parser, _| parse_element(parser));
        NodeList::new(pos, self.node_pos(), nodes)
    }

    /// `parseDelimitedList` with an element parser that always produces a node.
    pub fn parse_delimited_list(&mut self, kind: PC, mut parse_element: impl FnMut(&mut Self) -> NodeId) -> NodeList {
        self.parse_delimited_list_opt(kind, |parser| Some(parse_element(parser)))
            .expect("the element parser never fails")
    }

    /// `parseDelimitedList` with an element parser that can fail (return nil in
    /// Go), which fails the whole list.
    pub fn parse_delimited_list_opt(
        &mut self,
        kind: PC,
        mut parse_element: impl FnMut(&mut Self) -> Option<NodeId>,
    ) -> Option<NodeList> {
        let pos = self.node_pos();
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << kind as u32;
        let mut list = Vec::new();
        loop {
            if self.is_list_element(kind, false) {
                let start_pos = self.node_pos();
                let Some(element) = parse_element(self) else {
                    self.parsing_contexts = save_parsing_contexts;
                    return None;
                };
                list.push(element);
                if self.parse_optional(Kind::CommaToken) {
                    continue;
                }
                if self.is_list_terminator(kind) {
                    break;
                }
                if self.token != Kind::CommaToken && kind == PC::EnumMembers {
                    self.parse_error_at_current_token(diagnostics::An_enum_member_name_must_be_followed_by_a_or, &[]);
                } else {
                    self.parse_expected(Kind::CommaToken);
                }
                if (kind == PC::ObjectLiteralMembers || kind == PC::ImportAttributes)
                    && self.token == Kind::SemicolonToken
                    && !self.has_preceding_line_break()
                {
                    self.next_token();
                }
                if start_pos == self.node_pos() {
                    self.next_token();
                }
                continue;
            }
            if self.is_list_terminator(kind) {
                break;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        self.parsing_contexts = save_parsing_contexts;
        Some(NodeList::new(pos, self.node_pos(), list))
    }

    pub fn parse_bracketed_list(
        &mut self,
        kind: PC,
        parse_element: impl FnMut(&mut Self) -> NodeId,
        opening: Kind,
        closing: Kind,
    ) -> NodeList {
        if self.parse_expected(opening) {
            let result = self.parse_delimited_list(kind, parse_element);
            self.parse_expected(closing);
            return result;
        }
        self.create_missing_list()
    }

    pub fn parse_empty_node_list(&self) -> NodeList {
        NodeList::new(self.node_pos(), self.node_pos(), Vec::new())
    }

    pub fn create_missing_list(&self) -> NodeList {
        let mut list = self.parse_empty_node_list();
        list.missing = true;
        list
    }

    pub fn abort_parsing_list_or_move_to_next_token(&mut self, kind: PC) -> bool {
        self.parsing_context_errors(kind);
        if self.is_in_some_parsing_context() {
            return true;
        }
        self.next_token();
        false
    }

    pub fn is_in_some_parsing_context(&mut self) -> bool {
        for kind in PC_ALL {
            if self.parsing_contexts & (1 << kind as u32) != 0
                && (self.is_list_element(kind, true) || self.is_list_terminator(kind))
            {
                return true;
            }
        }
        false
    }

    pub fn parsing_context_errors(&mut self, context: PC) {
        use diagnostics as d;
        match context {
            PC::SourceElements => {
                if self.token == Kind::DefaultKeyword {
                    self.parse_error_at_current_token(d::X_0_expected, &["export"]);
                } else {
                    self.parse_error_at_current_token(d::Declaration_or_statement_expected, &[]);
                }
            }
            PC::BlockStatements => {
                self.parse_error_at_current_token(d::Declaration_or_statement_expected, &[]);
            }
            PC::SwitchClauses => {
                self.parse_error_at_current_token(d::X_case_or_default_expected, &[]);
            }
            PC::SwitchClauseStatements => {
                self.parse_error_at_current_token(d::Statement_expected, &[]);
            }
            PC::RestProperties | PC::TypeMembers => {
                self.parse_error_at_current_token(d::Property_or_signature_expected, &[]);
            }
            PC::ClassMembers => {
                self.parse_error_at_current_token(
                    d::Unexpected_token_A_constructor_method_accessor_or_property_was_expected,
                    &[],
                );
            }
            PC::EnumMembers => {
                self.parse_error_at_current_token(d::Enum_member_expected, &[]);
            }
            PC::HeritageClauseElement => {
                self.parse_error_at_current_token(d::Expression_expected, &[]);
            }
            PC::VariableDeclarations => {
                if ast::is_keyword(self.token) {
                    let text = token_to_string(self.token);
                    self.parse_error_at_current_token(d::X_0_is_not_allowed_as_a_variable_declaration_name, &[text]);
                } else {
                    self.parse_error_at_current_token(d::Variable_declaration_expected, &[]);
                }
            }
            PC::ObjectBindingElements => {
                self.parse_error_at_current_token(d::Property_destructuring_pattern_expected, &[]);
            }
            PC::ArrayBindingElements => {
                self.parse_error_at_current_token(d::Array_element_destructuring_pattern_expected, &[]);
            }
            PC::ArgumentExpressions => {
                self.parse_error_at_current_token(d::Argument_expression_expected, &[]);
            }
            PC::ObjectLiteralMembers => {
                self.parse_error_at_current_token(d::Property_assignment_expected, &[]);
            }
            PC::ArrayLiteralMembers => {
                self.parse_error_at_current_token(d::Expression_or_comma_expected, &[]);
            }
            PC::JSDocParameters => {
                self.parse_error_at_current_token(d::Parameter_declaration_expected, &[]);
            }
            PC::Parameters => {
                if ast::is_keyword(self.token) {
                    let text = token_to_string(self.token);
                    self.parse_error_at_current_token(d::X_0_is_not_allowed_as_a_parameter_name, &[text]);
                } else {
                    self.parse_error_at_current_token(d::Parameter_declaration_expected, &[]);
                }
            }
            PC::TypeParameters => {
                self.parse_error_at_current_token(d::Type_parameter_declaration_expected, &[]);
            }
            PC::TypeArguments => {
                self.parse_error_at_current_token(d::Type_argument_expected, &[]);
            }
            PC::TupleElementTypes => {
                self.parse_error_at_current_token(d::Type_expected, &[]);
            }
            PC::HeritageClauses => {
                self.parse_error_at_current_token(d::Unexpected_token_expected, &[]);
            }
            PC::ImportOrExportSpecifiers => {
                if self.token == Kind::FromKeyword {
                    self.parse_error_at_current_token(d::X_0_expected, &["}"]);
                } else {
                    self.parse_error_at_current_token(d::Identifier_expected, &[]);
                }
            }
            PC::JsxAttributes | PC::JsxChildren | PC::JSDocComment => {
                self.parse_error_at_current_token(d::Identifier_expected, &[]);
            }
            PC::ImportAttributes => {
                self.parse_error_at_current_token(d::Identifier_or_string_literal_expected, &[]);
            }
        }
    }

    pub fn is_list_element(&mut self, parsing_context: PC, in_error_recovery: bool) -> bool {
        match parsing_context {
            PC::SourceElements | PC::BlockStatements | PC::SwitchClauseStatements => {
                !(self.token == Kind::SemicolonToken && in_error_recovery) && self.is_start_of_statement()
            }
            PC::SwitchClauses => self.token == Kind::CaseKeyword || self.token == Kind::DefaultKeyword,
            PC::TypeMembers => self.look_ahead(Self::scan_type_member_start),
            PC::ClassMembers => {
                self.look_ahead(Self::scan_class_member_start)
                    || self.token == Kind::SemicolonToken && !in_error_recovery
            }
            PC::EnumMembers => self.token == Kind::OpenBracketToken || self.is_literal_property_name(),
            PC::ObjectLiteralMembers => match self.token {
                Kind::OpenBracketToken | Kind::AsteriskToken | Kind::DotDotDotToken | Kind::DotToken => true,
                _ => self.is_literal_property_name(),
            },
            PC::RestProperties => self.is_literal_property_name(),
            PC::ObjectBindingElements => {
                self.token == Kind::OpenBracketToken
                    || self.token == Kind::DotDotDotToken
                    || self.is_literal_property_name()
            }
            PC::ImportAttributes => self.is_import_attribute_name(),
            PC::HeritageClauseElement => {
                if self.token == Kind::OpenBraceToken {
                    return self.is_valid_heritage_clause_object_literal();
                }
                if !in_error_recovery {
                    return self.is_start_of_left_hand_side_expression()
                        && !self.is_heritage_clause_extends_or_implements_keyword();
                }
                self.is_identifier() && !self.is_heritage_clause_extends_or_implements_keyword()
            }
            PC::VariableDeclarations => self.is_binding_identifier_or_private_identifier_or_pattern(),
            PC::ArrayBindingElements => {
                self.token == Kind::CommaToken
                    || self.token == Kind::DotDotDotToken
                    || self.is_binding_identifier_or_private_identifier_or_pattern()
            }
            PC::TypeParameters => {
                self.token == Kind::InKeyword || self.token == Kind::ConstKeyword || self.is_identifier()
            }
            PC::ArrayLiteralMembers => {
                if self.token == Kind::CommaToken || self.token == Kind::DotToken {
                    return true;
                }
                self.token == Kind::DotDotDotToken || self.is_start_of_expression()
            }
            PC::ArgumentExpressions => self.token == Kind::DotDotDotToken || self.is_start_of_expression(),
            PC::Parameters => self.is_start_of_parameter(false),
            PC::JSDocParameters => self.is_start_of_parameter(true),
            PC::TypeArguments | PC::TupleElementTypes => {
                self.token == Kind::CommaToken || self.is_start_of_type(false)
            }
            PC::HeritageClauses => self.is_heritage_clause(),
            PC::ImportOrExportSpecifiers => {
                if self.token == Kind::FromKeyword && self.look_ahead(Self::next_token_is_token_string_literal) {
                    return false;
                }
                if self.token == Kind::StringLiteral {
                    return true;
                }
                token_is_identifier_or_keyword(self.token)
            }
            PC::JsxAttributes => token_is_identifier_or_keyword(self.token) || self.token == Kind::OpenBraceToken,
            PC::JsxChildren | PC::JSDocComment => true,
        }
    }

    pub fn is_list_terminator(&mut self, kind: PC) -> bool {
        if self.token == Kind::EndOfFile {
            return true;
        }
        match kind {
            PC::BlockStatements
            | PC::SwitchClauses
            | PC::TypeMembers
            | PC::ClassMembers
            | PC::EnumMembers
            | PC::ObjectLiteralMembers
            | PC::ObjectBindingElements
            | PC::ImportOrExportSpecifiers
            | PC::ImportAttributes => self.token == Kind::CloseBraceToken,
            PC::SwitchClauseStatements => {
                self.token == Kind::CloseBraceToken
                    || self.token == Kind::CaseKeyword
                    || self.token == Kind::DefaultKeyword
            }
            PC::HeritageClauseElement => {
                self.token == Kind::OpenBraceToken
                    || self.token == Kind::ExtendsKeyword
                    || self.token == Kind::ImplementsKeyword
            }
            PC::VariableDeclarations => {
                self.can_parse_semicolon()
                    || self.token == Kind::InKeyword
                    || self.token == Kind::OfKeyword
                    || self.token == Kind::EqualsGreaterThanToken
            }
            PC::TypeParameters => matches!(
                self.token,
                Kind::GreaterThanToken
                    | Kind::OpenParenToken
                    | Kind::OpenBraceToken
                    | Kind::ExtendsKeyword
                    | Kind::ImplementsKeyword
            ),
            PC::ArgumentExpressions => self.token == Kind::CloseParenToken || self.token == Kind::SemicolonToken,
            PC::ArrayLiteralMembers | PC::TupleElementTypes | PC::ArrayBindingElements => {
                self.token == Kind::CloseBracketToken
            }
            PC::JSDocParameters | PC::Parameters | PC::RestProperties => {
                self.token == Kind::CloseParenToken || self.token == Kind::CloseBracketToken
            }
            PC::TypeArguments => self.token != Kind::CommaToken,
            PC::HeritageClauses => self.token == Kind::OpenBraceToken || self.token == Kind::CloseBraceToken,
            PC::JsxAttributes => self.token == Kind::GreaterThanToken || self.token == Kind::SlashToken,
            PC::JsxChildren => self.token == Kind::LessThanToken && self.look_ahead(Self::next_token_is_slash),
            _ => false,
        }
    }

    // --- Tokens -------------------------------------------------------------

    pub fn parse_expected_matching_brackets(
        &mut self,
        _open_kind: Kind,
        close_kind: Kind,
        _open_parsed: bool,
        _open_position: usize,
    ) {
        if self.token == close_kind {
            self.next_token();
            return;
        }
        self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(close_kind)]);
    }

    pub fn parse_optional(&mut self, token: Kind) -> bool {
        if self.token == token {
            self.next_token();
            return true;
        }
        false
    }

    pub fn parse_expected(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, true)
    }

    pub fn parse_expected_without_advancing(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, false)
    }

    pub fn parse_expected_with_diagnostic(
        &mut self,
        kind: Kind,
        message: Option<&'static Message>,
        should_advance: bool,
    ) -> bool {
        if self.token == kind {
            if should_advance {
                self.next_token();
            }
            return true;
        }
        match message {
            Some(message) => self.parse_error_at_current_token(message, &[]),
            None => self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(kind)]),
        };
        false
    }

    pub fn parse_token_node(&mut self) -> NodeId {
        let pos = self.node_pos();
        let kind = self.token;
        self.next_token();
        self.finish_node(Node::new(kind), pos)
    }

    pub fn parse_expected_token(&mut self, kind: Kind) -> NodeId {
        match self.parse_optional_token(kind) {
            Some(token) => token,
            None => {
                self.parse_error_at_current_token(diagnostics::X_0_expected, &[token_to_string(kind)]);
                let pos = self.node_pos();
                self.finish_node(Node::new(kind), pos)
            }
        }
    }

    pub fn parse_optional_token(&mut self, kind: Kind) -> Option<NodeId> {
        if self.token == kind {
            return Some(self.parse_token_node());
        }
        None
    }

    // --- Nodes --------------------------------------------------------------

    pub fn finish_node(&mut self, node: Node, pos: usize) -> NodeId {
        let end = self.node_pos();
        self.finish_node_with_end(node, pos, end)
    }

    pub fn finish_node_with_end(&mut self, mut node: Node, pos: usize, end: usize) -> NodeId {
        node.pos = pos;
        node.end = end;
        node.flags |= self.context_flags;
        if self.scanner.has_parse_error {
            node.flags |= NodeFlags::ThisNodeHasError;
            self.scanner.has_parse_error = false;
        }
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        id
    }


    pub fn node_is_missing(&self, id: Option<NodeId>) -> bool {
        match id {
            None => true,
            Some(id) => {
                let node = self.node(id);
                node.pos == node.end && node.kind != Kind::EndOfFile
            }
        }
    }

    pub fn node_is_present(&self, id: Option<NodeId>) -> bool {
        !self.node_is_missing(id)
    }

    // --- Identifiers --------------------------------------------------------

    pub fn new_identifier(&mut self, text: String) -> Node {
        if text == "await" {
            self.statement_has_await_identifier = true;
        }
        Node::new(Kind::Identifier).text(text)
    }

    pub fn create_missing_identifier(&mut self) -> NodeId {
        let node = self.new_identifier(String::new());
        let pos = self.node_pos();
        self.finish_node(node, pos)
    }

    pub fn parse_private_identifier(&mut self) -> NodeId {
        let pos = self.node_pos();
        let text = self.scanner.token_value().to_string();
        self.next_token();
        self.finish_node(Node::new(Kind::PrivateIdentifier).text(text), pos)
    }

    pub fn re_scan_less_than_token(&mut self) -> Kind {
        self.token = self.scanner.re_scan_less_than_token();
        self.token
    }

    pub fn re_scan_greater_than_token(&mut self) -> Kind {
        self.token = self.scanner.re_scan_greater_than_token();
        self.token
    }

    pub fn re_scan_slash_token(&mut self) -> Kind {
        self.token = self.scanner.re_scan_slash_token(false);
        self.token
    }

    pub fn re_scan_template_token(&mut self, is_tagged_template: bool) -> Kind {
        self.token = self.scanner.re_scan_template_token(is_tagged_template);
        self.token
    }

    pub fn parse_right_side_of_dot(
        &mut self,
        allow_identifier_names: bool,
        allow_private_identifiers: bool,
        allow_unicode_escape_sequence_in_identifier_name: bool,
    ) -> NodeId {
        if self.has_preceding_line_break()
            && token_is_identifier_or_keyword(self.token)
            && self.look_ahead(Self::next_token_is_identifier_or_keyword_on_same_line)
        {
            let pos = self.node_pos();
            self.parse_error_at(pos, pos, diagnostics::Identifier_expected, &[]);
            return self.create_missing_identifier();
        }
        if self.token == Kind::PrivateIdentifier {
            let node = self.parse_private_identifier();
            if allow_private_identifiers {
                return node;
            }
            let pos = self.node_pos();
            self.parse_error_at(pos, pos, diagnostics::Identifier_expected, &[]);
            return self.create_missing_identifier();
        }
        if allow_identifier_names {
            if allow_unicode_escape_sequence_in_identifier_name {
                return self.parse_identifier_name();
            }
            return self.parse_identifier_name_error_on_unicode_escape_sequence();
        }
        let save_has_await_identifier = self.statement_has_await_identifier;
        let id = self.parse_identifier();
        self.statement_has_await_identifier = save_has_await_identifier;
        id
    }

    pub fn parse_identifier_name_error_on_unicode_escape_sequence(&mut self) -> NodeId {
        if self.scanner.has_unicode_escape() || self.scanner.has_extended_unicode_escape() {
            self.parse_error_at_current_token(diagnostics::Unicode_escape_sequence_cannot_appear_here, &[]);
        }
        self.create_identifier(token_is_identifier_or_keyword(self.token))
    }

    pub fn parse_binding_identifier(&mut self) -> NodeId {
        self.parse_binding_identifier_with_diagnostic(None)
    }

    pub fn parse_binding_identifier_with_diagnostic(
        &mut self,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> NodeId {
        let save_has_await_identifier = self.statement_has_await_identifier;
        let is_binding_identifier = self.is_binding_identifier();
        let id = self.create_identifier_with_diagnostic(is_binding_identifier, None, private_identifier_diagnostic_message);
        self.statement_has_await_identifier = save_has_await_identifier;
        id
    }

    pub fn parse_identifier_name(&mut self) -> NodeId {
        self.parse_identifier_name_with_diagnostic(None)
    }

    pub fn parse_identifier_name_with_diagnostic(&mut self, diagnostic_message: Option<&'static Message>) -> NodeId {
        let is_identifier = token_is_identifier_or_keyword(self.token);
        self.create_identifier_with_diagnostic(is_identifier, diagnostic_message, None)
    }

    pub fn parse_identifier(&mut self) -> NodeId {
        self.parse_identifier_with_diagnostic(None, None)
    }

    pub fn parse_identifier_with_diagnostic(
        &mut self,
        diagnostic_message: Option<&'static Message>,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> NodeId {
        let is_identifier = self.is_identifier();
        self.create_identifier_with_diagnostic(is_identifier, diagnostic_message, private_identifier_diagnostic_message)
    }

    pub fn create_identifier(&mut self, is_identifier: bool) -> NodeId {
        self.create_identifier_with_diagnostic(is_identifier, None, None)
    }

    pub fn create_identifier_with_diagnostic(
        &mut self,
        is_identifier: bool,
        diagnostic_message: Option<&'static Message>,
        private_identifier_diagnostic_message: Option<&'static Message>,
    ) -> NodeId {
        if is_identifier {
            let pos = self.node_pos();
            let text = self.scanner.token_value().to_string();
            self.next_token_without_check();
            let node = self.new_identifier(text);
            return self.finish_node(node, pos);
        }
        if self.token == Kind::PrivateIdentifier {
            self.parse_error_at_current_token(
                private_identifier_diagnostic_message
                    .unwrap_or(diagnostics::Private_identifiers_are_not_allowed_outside_class_bodies),
                &[],
            );
            return self.create_identifier(true);
        }
        let report_at_current_position = self.token == Kind::EndOfFile;
        let token_text = self.scanner.token_text();
        let (message, args): (&'static Message, Vec<&str>) = match diagnostic_message {
            Some(message) => (message, Vec::new()),
            None if ast::is_reserved_word(self.token) => {
                (diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here, vec![token_text])
            }
            None => (diagnostics::Identifier_expected, Vec::new()),
        };
        if report_at_current_position {
            let pos = self.scanner.token_full_start();
            self.parse_error_at(pos, pos, message, &args);
        } else {
            self.parse_error_at_current_token(message, &args);
        }
        self.create_missing_identifier()
    }

    // --- Predicates ---------------------------------------------------------

    pub fn next_token_is_slash(&mut self) -> bool {
        self.next_token() == Kind::SlashToken
    }

    pub fn scan_type_member_start(&mut self) -> bool {
        if matches!(self.token, Kind::OpenParenToken | Kind::LessThanToken | Kind::GetKeyword | Kind::SetKeyword) {
            return true;
        }
        let mut id_token = false;
        while ast::is_modifier_kind(self.token) {
            id_token = true;
            self.next_token();
        }
        if self.token == Kind::OpenBracketToken {
            return true;
        }
        if self.is_literal_property_name() {
            id_token = true;
            self.next_token();
        }
        if id_token {
            return matches!(
                self.token,
                Kind::OpenParenToken | Kind::LessThanToken | Kind::QuestionToken | Kind::ColonToken | Kind::CommaToken
            ) || self.can_parse_semicolon();
        }
        false
    }

    pub fn scan_class_member_start(&mut self) -> bool {
        let mut id_token = Kind::Unknown;
        if self.token == Kind::AtToken {
            return true;
        }
        while ast::is_modifier_kind(self.token) {
            id_token = self.token;
            if ast::is_class_member_modifier(id_token) {
                return true;
            }
            self.next_token();
        }
        if self.token == Kind::AsteriskToken {
            return true;
        }
        if self.is_literal_property_name() {
            id_token = self.token;
            self.next_token();
        }
        if self.token == Kind::OpenBracketToken {
            return true;
        }
        if id_token != Kind::Unknown {
            if !ast::is_keyword(id_token) || id_token == Kind::SetKeyword || id_token == Kind::GetKeyword {
                return true;
            }
            if matches!(
                self.token,
                Kind::OpenParenToken
                    | Kind::LessThanToken
                    | Kind::ExclamationToken
                    | Kind::ColonToken
                    | Kind::EqualsToken
                    | Kind::QuestionToken
            ) {
                return true;
            }
            return self.can_parse_semicolon();
        }
        false
    }

    pub fn can_parse_semicolon(&self) -> bool {
        self.token == Kind::SemicolonToken
            || self.token == Kind::CloseBraceToken
            || self.token == Kind::EndOfFile
            || self.has_preceding_line_break()
    }

    pub fn try_parse_semicolon(&mut self) -> bool {
        if !self.can_parse_semicolon() {
            return false;
        }
        if self.token == Kind::SemicolonToken {
            self.next_token();
        }
        true
    }

    pub fn parse_semicolon(&mut self) -> bool {
        self.try_parse_semicolon() || self.parse_expected(Kind::SemicolonToken)
    }

    pub fn is_literal_property_name(&self) -> bool {
        token_is_identifier_or_keyword(self.token)
            || matches!(self.token, Kind::StringLiteral | Kind::NumericLiteral | Kind::BigIntLiteral)
    }

    pub fn is_start_of_statement(&mut self) -> bool {
        match self.token {
            Kind::AtToken
            | Kind::SemicolonToken
            | Kind::OpenBraceToken
            | Kind::VarKeyword
            | Kind::LetKeyword
            | Kind::UsingKeyword
            | Kind::FunctionKeyword
            | Kind::ClassKeyword
            | Kind::EnumKeyword
            | Kind::IfKeyword
            | Kind::DoKeyword
            | Kind::WhileKeyword
            | Kind::ForKeyword
            | Kind::ContinueKeyword
            | Kind::BreakKeyword
            | Kind::ReturnKeyword
            | Kind::WithKeyword
            | Kind::SwitchKeyword
            | Kind::ThrowKeyword
            | Kind::TryKeyword
            | Kind::DebuggerKeyword
            | Kind::CatchKeyword
            | Kind::FinallyKeyword => true,
            Kind::ImportKeyword => self.is_start_of_declaration() || self.is_next_token_open_paren_or_less_than_or_dot(),
            Kind::ConstKeyword | Kind::ExportKeyword => self.is_start_of_declaration(),
            Kind::AsyncKeyword
            | Kind::DeclareKeyword
            | Kind::InterfaceKeyword
            | Kind::ModuleKeyword
            | Kind::NamespaceKeyword
            | Kind::TypeKeyword
            | Kind::GlobalKeyword
            | Kind::DeferKeyword => true,
            Kind::AccessorKeyword
            | Kind::PublicKeyword
            | Kind::PrivateKeyword
            | Kind::ProtectedKeyword
            | Kind::StaticKeyword
            | Kind::ReadonlyKeyword => {
                self.is_start_of_declaration()
                    || !self.look_ahead(Self::next_token_is_identifier_or_keyword_on_same_line)
            }
            _ => self.is_start_of_expression(),
        }
    }

    pub fn is_start_of_declaration(&mut self) -> bool {
        self.look_ahead(Self::scan_start_of_declaration)
    }

    pub fn scan_start_of_declaration(&mut self) -> bool {
        loop {
            match self.token {
                Kind::VarKeyword
                | Kind::LetKeyword
                | Kind::ConstKeyword
                | Kind::FunctionKeyword
                | Kind::ClassKeyword
                | Kind::EnumKeyword => return true,
                Kind::UsingKeyword => return self.is_using_declaration(),
                Kind::AwaitKeyword => return self.is_await_using_declaration(),
                Kind::InterfaceKeyword | Kind::TypeKeyword | Kind::DeferKeyword => {
                    return self.next_token_is_identifier_on_same_line();
                }
                Kind::ModuleKeyword | Kind::NamespaceKeyword => {
                    return self.next_token_is_identifier_or_string_literal_on_same_line();
                }
                Kind::AbstractKeyword
                | Kind::AccessorKeyword
                | Kind::AsyncKeyword
                | Kind::DeclareKeyword
                | Kind::PrivateKeyword
                | Kind::ProtectedKeyword
                | Kind::PublicKeyword
                | Kind::ReadonlyKeyword => {
                    let previous_token = self.token;
                    self.next_token();
                    if self.has_preceding_line_break() {
                        return false;
                    }
                    if previous_token == Kind::DeclareKeyword && self.token == Kind::TypeKeyword {
                        return true;
                    }
                    continue;
                }
                Kind::GlobalKeyword => {
                    self.next_token();
                    return matches!(self.token, Kind::OpenBraceToken | Kind::Identifier | Kind::ExportKeyword);
                }
                Kind::ImportKeyword => {
                    self.next_token();
                    return matches!(
                        self.token,
                        Kind::DeferKeyword | Kind::StringLiteral | Kind::AsteriskToken | Kind::OpenBraceToken
                    ) || token_is_identifier_or_keyword(self.token);
                }
                Kind::ExportKeyword => {
                    self.next_token();
                    if matches!(
                        self.token,
                        Kind::EqualsToken
                            | Kind::AsteriskToken
                            | Kind::OpenBraceToken
                            | Kind::DefaultKeyword
                            | Kind::AsKeyword
                            | Kind::AtToken
                    ) {
                        return true;
                    }
                    if self.token == Kind::TypeKeyword {
                        self.next_token();
                        return self.token == Kind::AsteriskToken
                            || self.token == Kind::OpenBraceToken
                            || self.is_identifier() && !self.has_preceding_line_break();
                    }
                    continue;
                }
                Kind::StaticKeyword => {
                    self.next_token();
                    continue;
                }
                _ => return false,
            }
        }
    }

    pub fn is_start_of_expression(&mut self) -> bool {
        if self.is_start_of_left_hand_side_expression() {
            return true;
        }
        if matches!(
            self.token,
            Kind::PlusToken
                | Kind::MinusToken
                | Kind::TildeToken
                | Kind::ExclamationToken
                | Kind::DeleteKeyword
                | Kind::TypeOfKeyword
                | Kind::VoidKeyword
                | Kind::PlusPlusToken
                | Kind::MinusMinusToken
                | Kind::LessThanToken
                | Kind::AwaitKeyword
                | Kind::YieldKeyword
                | Kind::PrivateIdentifier
                | Kind::AtToken
        ) {
            return true;
        }
        if self.is_binary_operator() {
            return true;
        }
        self.is_identifier()
    }

    pub fn is_start_of_left_hand_side_expression(&mut self) -> bool {
        match self.token {
            Kind::ThisKeyword
            | Kind::SuperKeyword
            | Kind::NullKeyword
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::StringLiteral
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateHead
            | Kind::OpenParenToken
            | Kind::OpenBracketToken
            | Kind::OpenBraceToken
            | Kind::FunctionKeyword
            | Kind::ClassKeyword
            | Kind::NewKeyword
            | Kind::SlashToken
            | Kind::SlashEqualsToken
            | Kind::Identifier => true,
            Kind::ImportKeyword => self.is_next_token_open_paren_or_less_than_or_dot(),
            _ => self.is_identifier(),
        }
    }

    pub fn is_start_of_type(&mut self, in_start_of_parameter: bool) -> bool {
        match self.token {
            Kind::AnyKeyword
            | Kind::UnknownKeyword
            | Kind::StringKeyword
            | Kind::NumberKeyword
            | Kind::BigIntKeyword
            | Kind::BooleanKeyword
            | Kind::ReadonlyKeyword
            | Kind::SymbolKeyword
            | Kind::UniqueKeyword
            | Kind::VoidKeyword
            | Kind::UndefinedKeyword
            | Kind::NullKeyword
            | Kind::ThisKeyword
            | Kind::TypeOfKeyword
            | Kind::NeverKeyword
            | Kind::OpenBraceToken
            | Kind::OpenBracketToken
            | Kind::LessThanToken
            | Kind::BarToken
            | Kind::AmpersandToken
            | Kind::NewKeyword
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::ObjectKeyword
            | Kind::AsteriskToken
            | Kind::QuestionToken
            | Kind::ExclamationToken
            | Kind::DotDotDotToken
            | Kind::InferKeyword
            | Kind::ImportKeyword
            | Kind::AssertsKeyword
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateHead => true,
            Kind::FunctionKeyword => !in_start_of_parameter,
            Kind::MinusToken => !in_start_of_parameter && self.look_ahead(Self::next_token_is_numeric_or_big_int_literal),
            Kind::OpenParenToken => {
                !in_start_of_parameter && self.look_ahead(Self::next_is_parenthesized_or_function_type)
            }
            _ => self.is_identifier(),
        }
    }

    pub fn next_token_is_numeric_or_big_int_literal(&mut self) -> bool {
        self.next_token();
        self.token == Kind::NumericLiteral || self.token == Kind::BigIntLiteral
    }

    pub fn next_is_parenthesized_or_function_type(&mut self) -> bool {
        self.next_token();
        self.token == Kind::CloseParenToken || self.is_start_of_parameter(false) || self.is_start_of_type(false)
    }

    pub fn is_start_of_parameter(&mut self, is_jsdoc_parameter: bool) -> bool {
        self.token == Kind::DotDotDotToken
            || self.is_binding_identifier_or_private_identifier_or_pattern()
            || ast::is_modifier_kind(self.token)
            || self.token == Kind::AtToken
            || self.is_start_of_type(!is_jsdoc_parameter)
    }

    pub fn is_binding_identifier_or_private_identifier_or_pattern(&self) -> bool {
        matches!(self.token, Kind::OpenBraceToken | Kind::OpenBracketToken | Kind::PrivateIdentifier)
            || self.is_binding_identifier()
    }

    pub fn is_next_token_open_paren_or_less_than_or_dot(&mut self) -> bool {
        self.look_ahead(Self::next_token_is_open_paren_or_less_than_or_dot)
    }

    pub fn next_token_is_open_paren_or_less_than_or_dot(&mut self) -> bool {
        matches!(self.next_token(), Kind::OpenParenToken | Kind::LessThanToken | Kind::DotToken)
    }

    pub fn next_token_is_identifier_on_same_line(&mut self) -> bool {
        self.next_token();
        self.is_identifier() && !self.has_preceding_line_break()
    }

    pub fn next_token_is_identifier_or_string_literal_on_same_line(&mut self) -> bool {
        self.next_token();
        (self.is_identifier() || self.token == Kind::StringLiteral) && !self.has_preceding_line_break()
    }

    pub fn is_identifier(&self) -> bool {
        if self.token == Kind::Identifier {
            return true;
        }
        if self.token == Kind::YieldKeyword && self.in_yield_context()
            || self.token == Kind::AwaitKeyword && self.in_await_context()
        {
            return false;
        }
        self.token > Kind::LastReservedWord
    }

    pub fn is_binding_identifier(&self) -> bool {
        self.token == Kind::Identifier || self.token > Kind::LastReservedWord
    }

    pub fn is_import_attribute_name(&self) -> bool {
        token_is_identifier_or_keyword(self.token) || self.token == Kind::StringLiteral
    }

    pub fn is_binary_operator(&self) -> bool {
        if self.in_disallow_in_context() && self.token == Kind::InKeyword {
            return false;
        }
        ast::get_binary_operator_precedence(self.token) != OperatorPrecedence::Invalid
    }

    pub fn is_valid_heritage_clause_object_literal(&mut self) -> bool {
        self.look_ahead(Self::next_is_valid_heritage_clause_object_literal)
    }

    pub fn next_is_valid_heritage_clause_object_literal(&mut self) -> bool {
        if self.next_token() == Kind::CloseBraceToken {
            let next = self.next_token();
            return matches!(
                next,
                Kind::CommaToken | Kind::OpenBraceToken | Kind::ExtendsKeyword | Kind::ImplementsKeyword
            );
        }
        true
    }

    pub fn is_heritage_clause(&self) -> bool {
        self.token == Kind::ExtendsKeyword || self.token == Kind::ImplementsKeyword
    }

    pub fn is_heritage_clause_extends_or_implements_keyword(&mut self) -> bool {
        self.is_heritage_clause() && self.look_ahead(Self::next_is_start_of_expression)
    }

    pub fn next_is_start_of_expression(&mut self) -> bool {
        self.next_token();
        self.is_start_of_expression()
    }

    pub fn is_using_declaration(&mut self) -> bool {
        self.look_ahead(|parser| parser.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(false))
    }

    pub fn next_token_is_equals_or_semicolon_or_colon_token(&mut self) -> bool {
        self.next_token();
        matches!(self.token, Kind::EqualsToken | Kind::SemicolonToken | Kind::ColonToken)
    }

    pub fn next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(&mut self, disallow_of: bool) -> bool {
        self.next_token();
        if disallow_of && self.token == Kind::OfKeyword {
            return self.look_ahead(Self::next_token_is_equals_or_semicolon_or_colon_token);
        }
        (self.is_binding_identifier() || self.token == Kind::OpenBraceToken) && !self.has_preceding_line_break()
    }

    pub fn next_token_is_binding_identifier_or_start_of_destructuring_on_same_line_disallow_of(&mut self) -> bool {
        self.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(true)
    }

    pub fn is_await_using_declaration(&mut self) -> bool {
        self.look_ahead(Self::next_is_using_keyword_then_binding_identifier_or_start_of_object_destructuring_on_same_line)
    }

    pub fn next_is_using_keyword_then_binding_identifier_or_start_of_object_destructuring_on_same_line(&mut self) -> bool {
        self.next_token() == Kind::UsingKeyword
            && self.next_token_is_binding_identifier_or_start_of_destructuring_on_same_line(false)
    }

    pub fn next_token_is_token_string_literal(&mut self) -> bool {
        self.next_token() == Kind::StringLiteral
    }

    // --- Context flags ------------------------------------------------------

    pub fn set_context_flags(&mut self, flags: NodeFlags, value: bool) {
        if value {
            self.context_flags |= flags;
        } else {
            self.context_flags &= !flags;
        }
    }

    pub fn do_in_context<T>(&mut self, flags: NodeFlags, value: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        let save_context_flags = self.context_flags;
        self.set_context_flags(flags, value);
        let result = f(self);
        self.context_flags = save_context_flags;
        result
    }

    pub fn in_yield_context(&self) -> bool {
        self.context_flags.has(NodeFlags::YieldContext)
    }

    pub fn in_disallow_in_context(&self) -> bool {
        self.context_flags.has(NodeFlags::DisallowInContext)
    }

    pub fn in_disallow_conditional_types_context(&self) -> bool {
        self.context_flags.has(NodeFlags::DisallowConditionalTypesContext)
    }

    pub fn in_decorator_context(&self) -> bool {
        self.context_flags.has(NodeFlags::DecoratorContext)
    }

    pub fn in_await_context(&self) -> bool {
        self.context_flags.has(NodeFlags::AwaitContext)
    }

    /// `skipRangeTrivia`.
    pub fn skip_range_trivia(&self, pos: usize, end: usize) -> (usize, usize) {
        (scanner::skip_trivia(self.source_text, pos), end)
    }

    // --- JSX ----------------------------------------------------------------

    /// `ast.TagNamesAreEquivalent`.
    pub fn tag_names_are_equivalent(&self, lhs: NodeId, rhs: NodeId) -> bool {
        let (left, right) = (self.node(lhs), self.node(rhs));
        if left.kind != right.kind {
            return false;
        }
        match left.kind {
            Kind::Identifier => left.text == right.text,
            Kind::ThisKeyword => true,
            Kind::JsxNamespacedName => {
                let text = |id: Option<NodeId>| id.map(|id| self.node(id).text.as_str());
                text(left.expression) == text(right.expression) && text(left.name) == text(right.name)
            }
            Kind::PropertyAccessExpression => {
                let text = |id: Option<NodeId>| id.map(|id| self.node(id).text.as_str());
                text(left.name) == text(right.name)
                    && match (left.expression, right.expression) {
                        (Some(l), Some(r)) => self.tag_names_are_equivalent(l, r),
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    // --- JavaScript-only syntax ---------------------------------------------

    pub fn js_error_at_range(&mut self, pos: usize, end: usize, message: &'static Message, args: &[&str]) {
        let start = scanner::skip_trivia(self.source_text, pos);
        self.js_diagnostics.push(Diagnostic {
            start,
            end,
            message,
            args: args.iter().map(|arg| arg.to_string()).collect(),
        });
    }

    fn js_error_at_node(&mut self, id: NodeId, message: &'static Message, args: &[&str]) {
        let (pos, end) = (self.node(id).pos, self.node(id).end);
        self.js_error_at_range(pos, end, message, args);
    }

    pub fn check_js_decorator_syntax(&mut self, id: NodeId) {
        let modifiers = self.node(id).modifier_nodes().to_vec();
        if modifiers.is_empty() {
            return;
        }
        let kind = self.kind(id);
        let is_decorator = |parser: &Self, m: NodeId| parser.kind(m) == Kind::Decorator;
        if ast::can_have_illegal_decorators(kind) {
            if let Some(&decorator) = modifiers.iter().find(|&&m| is_decorator(self, m)) {
                self.js_error_at_node(decorator, diagnostics::Decorators_are_not_valid_here, &[]);
            }
        } else if ast::can_have_decorators(kind)
            && let Some(decorator_index) = modifiers.iter().position(|&m| is_decorator(self, m))
            && kind == Kind::ClassDeclaration
            && let Some(export_index) = modifiers.iter().position(|&m| self.kind(m) == Kind::ExportKeyword)
        {
            let default_index = modifiers.iter().position(|&m| self.kind(m) == Kind::DefaultKeyword);
            if decorator_index > export_index && default_index.is_some_and(|d| decorator_index < d) {
                self.js_error_at_node(modifiers[decorator_index], diagnostics::Decorators_are_not_valid_here, &[]);
            } else if decorator_index < export_index
                && let Some(trailing) = modifiers[export_index..].iter().find(|&&m| is_decorator(self, m))
            {
                self.js_error_at_node(
                    *trailing,
                    diagnostics::Decorators_may_not_appear_after_export_or_export_default_if_they_also_appear_before_export,
                    &[],
                );
            }
        }
    }

    pub fn check_js_syntax(&mut self, id: NodeId) -> NodeId {
        let flags = self.node(id).flags;
        if !flags.has(NodeFlags::JavaScriptFile) || flags.has(NodeFlags::JSDoc | NodeFlags::Reparsed) {
            return id;
        }
        let kind = self.kind(id);
        match kind {
            Kind::Parameter
            | Kind::PropertyDeclaration
            | Kind::MethodDeclaration
            | Kind::MethodSignature
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::FunctionExpression
            | Kind::FunctionDeclaration
            | Kind::ArrowFunction
            | Kind::VariableDeclaration
            | Kind::IndexSignature => {
                if matches!(kind, Kind::Parameter | Kind::PropertyDeclaration | Kind::MethodDeclaration)
                    && let Some(token) = self.node(id).question_token
                    && self.kind(token) == Kind::QuestionToken
                {
                    self.js_error_at_node(token, diagnostics::The_0_modifier_can_only_be_used_in_TypeScript_files, &["?"]);
                }
                if ast::is_function_like_kind(kind) && self.node(id).body.is_none() {
                    self.js_error_at_node(id, diagnostics::Signature_declarations_can_only_be_used_in_TypeScript_files, &[]);
                } else if let Some(ty) = self.node(id).ty {
                    self.js_error_at_node(ty, diagnostics::Type_annotations_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            Kind::ImportDeclaration => {
                if let Some(clause) = self.node(id).import_clause
                    && self.node(clause).is_type_only
                {
                    self.js_error_at_node(id, diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files, &["import type"]);
                }
            }
            Kind::ExportDeclaration => {
                if self.node(id).is_type_only {
                    self.js_error_at_node(id, diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files, &["export type"]);
                }
            }
            Kind::ImportSpecifier => {
                if self.node(id).is_type_only {
                    self.js_error_at_node(id, diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files, &["import...type"]);
                }
            }
            Kind::ExportSpecifier => {
                if self.node(id).is_type_only {
                    self.js_error_at_node(id, diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files, &["export...type"]);
                }
            }
            Kind::ImportEqualsDeclaration => {
                self.js_error_at_node(id, diagnostics::X_import_can_only_be_used_in_TypeScript_files, &[]);
            }
            Kind::ExportAssignment => {
                if self.node(id).is_export_equals {
                    self.js_error_at_node(id, diagnostics::X_export_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            Kind::HeritageClause => {
                if self.node(id).op == Kind::ImplementsKeyword {
                    self.js_error_at_node(id, diagnostics::X_implements_clauses_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            Kind::InterfaceDeclaration | Kind::ModuleDeclaration | Kind::TypeAliasDeclaration | Kind::EnumDeclaration => {
                if let Some(name) = self.node(id).name {
                    match kind {
                        Kind::InterfaceDeclaration => self.js_error_at_node(
                            name,
                            diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files,
                            &["interface"],
                        ),
                        Kind::ModuleDeclaration => {
                            let keyword = token_to_string(self.node(id).op);
                            self.js_error_at_node(
                                name,
                                diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files,
                                &[keyword],
                            )
                        }
                        Kind::TypeAliasDeclaration => self.js_error_at_node(
                            name,
                            diagnostics::Type_aliases_can_only_be_used_in_TypeScript_files,
                            &[],
                        ),
                        _ => self.js_error_at_node(
                            name,
                            diagnostics::X_0_declarations_can_only_be_used_in_TypeScript_files,
                            &["enum"],
                        ),
                    }
                }
            }
            Kind::NonNullExpression => {
                self.js_error_at_node(id, diagnostics::Non_null_assertions_can_only_be_used_in_TypeScript_files, &[]);
            }
            Kind::AsExpression => {
                if let Some(ty) = self.node(id).ty {
                    self.js_error_at_node(ty, diagnostics::Type_assertion_expressions_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            Kind::SatisfiesExpression => {
                if let Some(ty) = self.node(id).ty {
                    self.js_error_at_node(ty, diagnostics::Type_satisfaction_expressions_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            _ => {}
        }
        self.check_js_decorator_syntax(id);
        match kind {
            Kind::ClassDeclaration
            | Kind::ClassExpression
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::FunctionExpression
            | Kind::FunctionDeclaration
            | Kind::ArrowFunction
            | Kind::VariableStatement
            | Kind::PropertyDeclaration => {
                if !matches!(kind, Kind::VariableStatement | Kind::PropertyDeclaration)
                    && let Some(list) = self.node(id).type_parameters.clone()
                    && list.nodes.iter().any(|&n| !self.node(n).flags.has(NodeFlags::Reparsed))
                {
                    self.js_error_at_range(list.pos, list.end, diagnostics::Type_parameter_declarations_can_only_be_used_in_TypeScript_files, &[]);
                }
                for modifier in self.node(id).modifier_nodes().to_vec() {
                    let modifier_kind = self.kind(modifier);
                    if !self.node(modifier).flags.has(NodeFlags::Reparsed)
                        && modifier_kind != Kind::Decorator
                        && !ast::is_javascript_modifier(modifier_kind)
                    {
                        let text = token_to_string(modifier_kind);
                        self.js_error_at_node(modifier, diagnostics::The_0_modifier_can_only_be_used_in_TypeScript_files, &[text]);
                    }
                }
            }
            Kind::Parameter => {
                if self.node(id).modifier_nodes().iter().any(|&m| ast::is_modifier_kind(self.kind(m)))
                    && let Some(list) = self.node(id).modifiers.clone()
                {
                    self.js_error_at_range(list.pos, list.end, diagnostics::Parameter_modifiers_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            Kind::CallExpression
            | Kind::NewExpression
            | Kind::ExpressionWithTypeArguments
            | Kind::JsxSelfClosingElement
            | Kind::JsxOpeningElement
            | Kind::TaggedTemplateExpression => {
                if let Some(list) = self.node(id).type_arguments.clone()
                    && list.nodes.iter().any(|&n| !self.node(n).flags.has(NodeFlags::Reparsed))
                {
                    self.js_error_at_range(list.pos, list.end, diagnostics::Type_arguments_can_only_be_used_in_TypeScript_files, &[]);
                }
            }
            _ => {}
        }
        id
    }
}
