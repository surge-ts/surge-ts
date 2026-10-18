//! A read-only view of a parsed file for callers outside the crate: the
//! compiler API indexes its `Node` handles into a [`SyntaxTree`] and reads
//! declarations off the [`BoundFile`] the binder leaves behind.
//!
//! Offsets are byte offsets into the UTF-8 source text, exactly as the parser
//! records them; converting them to the UTF-16 offsets TypeScript's API
//! reports is the caller's job.

use crate::ast::{Node, NodeId};
use crate::flags::NodeFlags;
use crate::kind::Kind;
use crate::parser::{self, ParsedFile};
use crate::{ParseOptions, SyntaxDiagnostic};

pub use crate::binder::{BoundFile, BoundSymbol, symbol_flags};

/// A parsed file: an arena of nodes rooted at a `SourceFile` node.
///
/// Nodes the parser built while looking ahead and then discarded stay in the
/// arena without a parent; [`SyntaxTree::is_attached`] tells them apart.
pub struct SyntaxTree {
    parsed: ParsedFile,
    end_of_file: NodeId,
}

impl SyntaxTree {
    pub fn parse(text: &str, options: &ParseOptions) -> SyntaxTree {
        Self::with_end_of_file(parser::parse(text, options), text)
    }

    /// A JSON file's tree (`parseJsonText`).
    pub fn parse_json(text: &str) -> SyntaxTree {
        Self::with_end_of_file(parser::parse_json_file(text), text)
    }

    fn with_end_of_file(mut parsed: ParsedFile, text: &str) -> SyntaxTree {
        // tsc's `SourceFile.endOfFileToken`: this port's parser keeps no node for
        // it, so the tree gains one spanning the trailing trivia.
        let root = parsed.root;
        let statements_end = parsed.end_of_file_pos;
        let mut end_of_file = Node::new(Kind::EndOfFile);
        end_of_file.pos = statements_end;
        end_of_file.end = text.len();
        end_of_file.flags = parsed.node(root).flags & NodeFlags::ContextFlags;
        let end_of_file_id = parsed.nodes.len() as NodeId;
        parsed.nodes.push(end_of_file);
        parsed.parents.push(Some(root));
        parsed.nodes[root as usize].children.push(Some(end_of_file_id));
        // tsc ends the statement list where the end-of-file token's leading
        // trivia starts; this port's parser ends it at the end of the text.
        if let Some(Some(statements)) = parsed.nodes[root as usize].lists.first_mut() {
            statements.end = statements_end;
        }
        SyntaxTree { parsed, end_of_file: end_of_file_id }
    }

    pub fn root(&self) -> NodeId {
        self.parsed.root
    }

    pub fn end_of_file_token(&self) -> NodeId {
        self.end_of_file
    }

    /// The arena size, including detached nodes.
    pub fn node_count(&self) -> usize {
        self.parsed.nodes.len()
    }

    pub fn node(&self, id: NodeId) -> &Node {
        self.parsed.node(id)
    }

    pub fn kind(&self, id: NodeId) -> Kind {
        self.parsed.node(id).kind
    }

    pub fn pos(&self, id: NodeId) -> usize {
        self.parsed.node(id).pos
    }

    pub fn end(&self, id: NodeId) -> usize {
        self.parsed.node(id).end
    }

    pub fn flags(&self, id: NodeId) -> NodeFlags {
        self.parsed.node(id).flags
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parsed.parent(id)
    }

    /// Whether the node is part of the tree rather than a leftover of a
    /// discarded look-ahead.
    pub fn is_attached(&self, id: NodeId) -> bool {
        id == self.parsed.root || self.parsed.parent(id).is_some()
    }

    /// `ForEachChild`: every child, in source order.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.parsed.children(id)
    }

    /// `GetTokenPosOfNode`: the node's start after its leading trivia. A
    /// missing (zero-width) node and JSX text start where they are.
    pub fn start(&self, text: &str, id: NodeId) -> usize {
        let node = self.parsed.node(id);
        if node.pos == node.end || node.kind == Kind::EndOfFile && node.pos == text.len() {
            return node.pos;
        }
        if node.kind == Kind::JsxText {
            // JSX text holds no comments, so tsc skips only its leading
            // whitespace (`SkipTriviaEx` with `StopAtComments`).
            let rest = &text[node.pos..node.end];
            return node.pos + (rest.len() - rest.trim_start_matches(|ch: char| crate::chars::is_white_space_like(ch as i32)).len());
        }
        crate::scanner::skip_trivia(text, node.pos)
    }

    pub fn is_external_module(&self) -> bool {
        self.parsed.external_module
    }

    pub fn is_declaration_file(&self) -> bool {
        self.parsed.is_declaration_file
    }

    /// `LanguageVariantJSX`.
    pub fn is_jsx(&self) -> bool {
        self.parsed.jsx
    }

    /// tsc's `GetSyntacticDiagnostics` for the file (see
    /// [`crate::syntactic_diagnostics`]).
    pub fn syntactic_diagnostics(&self) -> Vec<SyntaxDiagnostic> {
        crate::render(self.parsed.diagnostics.iter().chain(self.parsed.js_diagnostics.iter()))
    }

    /// Runs the binder over the tree. tsc binds a file whatever its syntax
    /// errors, and so does this.
    pub fn bind(&self, text: &str) -> BoundFile {
        crate::binder::bind(&self.parsed, text).into_bound_file()
    }

    /// The node's identifier or literal text, as the parser recorded it.
    pub fn text(&self, id: NodeId) -> &str {
        &self.parsed.node(id).text
    }
}
