//! The diagnostics tsc reports for a file before type checking: a port of
//! typescript-go's scanner, recovering parser and binder (`internal/scanner`,
//! `internal/parser`, `internal/binder`), kept to the parts that decide which
//! errors are reported and where.
//!
//! The syntax tree is also the one the compiler API hands out: [`SyntaxTree`]
//! is its read-only view, and [`BoundFile`] what the binder declared in it.

pub mod ast;
mod binder;
mod chars;
mod checker_grammar;
pub mod flags;
pub mod kind;
mod merge;
mod messages;
mod parser;
pub mod properties;
mod regexp;
mod scanner;
mod tree;

pub use kind::Kind;
pub use scanner::skip_trivia;
pub use tree::{BoundFile, BoundSymbol, SyntaxTree, symbol_flags};

pub struct Message {
    pub code: u32,
    pub text: &'static str,
}

#[derive(Clone)]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: &'static Message,
    pub args: Vec<String>,
}

/// A reported syntax error: byte offsets into the source text, tsc's code,
/// and its message with the arguments filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxDiagnostic {
    pub start: usize,
    pub end: usize,
    pub code: u32,
    pub message: String,
}

impl Diagnostic {
    fn render(&self) -> SyntaxDiagnostic {
        let mut message = self.message.text.to_string();
        for (index, arg) in self.args.iter().enumerate() {
            message = message.replace(&format!("{{{index}}}"), arg);
        }
        SyntaxDiagnostic { start: self.start, end: self.end, code: self.message.code, message }
    }
}

/// `core.ScriptTarget`: the language version a file is checked against.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ScriptTarget {
    ES5 = 1,
    ES2015 = 2,
    ES2016 = 3,
    ES2017 = 4,
    ES2018 = 5,
    ES2019 = 6,
    ES2020 = 7,
    ES2021 = 8,
    ES2022 = 9,
    ES2023 = 10,
    ES2024 = 11,
    ES2025 = 12,
    ESNext = 99,
}

impl Default for ScriptTarget {
    /// `ScriptTargetLatestStandard`, what `GetEmitScriptTarget` gives an unset
    /// `target`.
    fn default() -> Self {
        Self::ES2025
    }
}

impl ScriptTarget {
    /// The name tsc's messages use (`strings.ToLower(target.String())`).
    pub fn name(self) -> &'static str {
        match self {
            Self::ES5 => "es5",
            Self::ES2015 => "es2015",
            Self::ES2016 => "es2016",
            Self::ES2017 => "es2017",
            Self::ES2018 => "es2018",
            Self::ES2019 => "es2019",
            Self::ES2020 => "es2020",
            Self::ES2021 => "es2021",
            Self::ES2022 => "es2022",
            Self::ES2023 => "es2023",
            Self::ES2024 => "es2024",
            Self::ES2025 => "es2025",
            Self::ESNext => "esnext",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScriptKind {
    Ts,
    Tsx,
    Js,
    Jsx,
}

impl ScriptKind {
    /// The kind tsc gives a file by its extension; `None` for JSON and
    /// anything tsc does not parse as script.
    pub fn from_file_name(file_name: &str) -> Option<Self> {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".tsx") {
            Some(Self::Tsx)
        } else if lower.ends_with(".ts") || lower.ends_with(".mts") || lower.ends_with(".cts") {
            Some(Self::Ts)
        } else if lower.ends_with(".jsx") {
            Some(Self::Jsx)
        } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs") {
            Some(Self::Js)
        } else {
            None
        }
    }
}

/// `ast.SourceFileParseOptions`, with its `ExternalModuleIndicatorOptions`:
/// what makes a file without an import or export a module.
pub struct ParseOptions {
    pub script_kind: ScriptKind,
    pub is_declaration_file: bool,
    /// The file is a module whatever its statements say: `moduleDetection:
    /// force`, or a format that forces it (an `.mts`/`.cts`/`.mjs`/`.cjs`
    /// file, or one a `"type": "module"` package holds). Never set for a
    /// declaration file.
    pub force_module: bool,
    /// A JSX tag makes the file a module (`jsx: react-jsx` or `react-jsxdev`
    /// under `moduleDetection: auto`).
    pub jsx_forces_module: bool,
    /// The checker's language version, for what its regular expression
    /// check allows.
    pub language_version: ScriptTarget,
    /// The checker's unused-identifier check, when either option asks for it.
    pub unused: Option<UnusedCheck>,
}

/// What tsc's unused-identifier check (`checkUnusedIdentifiers`) reads
/// beyond the file itself.
pub struct UnusedCheck {
    /// `noUnusedLocals`.
    pub locals: bool,
    /// `noUnusedParameters`.
    pub parameters: bool,
    /// The names a JSX element resolves for its factory
    /// (`markJsxAliasReferenced`); none under the automatic runtime.
    pub jsx_element_reads: Vec<String>,
    /// The names a JSX fragment resolves for its factories.
    pub jsx_fragment_reads: Vec<String>,
    /// The names JSDoc `{@link}` tags refer to: tsc resolves each
    /// (`checkJSDocLinkLikeTag`), and this port does not parse JSDoc. In a
    /// JavaScript file, also every name its JSDoc types may refer to.
    pub jsdoc_link_names: Vec<String>,
    /// `GetEmitStandardClassFields`.
    pub emit_standard_class_fields: bool,
}

impl ParseOptions {
    pub fn for_file_name(file_name: &str) -> Option<Self> {
        let script_kind = ScriptKind::from_file_name(file_name)?;
        let lower = file_name.to_ascii_lowercase();
        let is_declaration_file = lower.ends_with(".d.ts")
            || lower.ends_with(".d.mts")
            || lower.ends_with(".d.cts")
            || lower.contains(".d.") && lower.ends_with(".ts");
        let force_module =
            !is_declaration_file && [".mts", ".cts", ".mjs", ".cjs"].iter().any(|ext| lower.ends_with(ext));
        Some(Self {
            script_kind,
            is_declaration_file,
            force_module,
            jsx_forces_module: false,
            language_version: ScriptTarget::default(),
            unused: None,
        })
    }
}

/// tsc's `GetSyntacticDiagnostics` for one file: the parser's diagnostics,
/// then, for a JavaScript file, the ones it reports for TypeScript-only syntax.
///
/// Identical diagnostics are reported once, as tsc's
/// `SortAndDeduplicateDiagnostics` does: an unclosed JSX element nested in
/// another reports the missing `</` at the end of the file for both.
pub fn syntactic_diagnostics(text: &str, options: &ParseOptions) -> Vec<SyntaxDiagnostic> {
    let parsed = parser::parse(text, options);
    render(parsed.diagnostics.iter().chain(parsed.js_diagnostics.iter()))
}

fn render<'d>(diagnostics: impl Iterator<Item = &'d Diagnostic>) -> Vec<SyntaxDiagnostic> {
    let mut rendered: Vec<SyntaxDiagnostic> = Vec::new();
    for diagnostic in diagnostics.map(Diagnostic::render) {
        if !rendered.contains(&diagnostic) {
            rendered.push(diagnostic);
        }
    }
    rendered
}

pub use merge::{
    FileGlobals, GlobalsInput, MergeReport, excluded_symbol_flags, merge_globals, merge_globals_report, merge_globals_report_with,
};

/// What tsc reports for a file before type checking: its syntactic
/// diagnostics, and, for a file with none, what its binder reports
/// (declarations that conflict, strict-mode restrictions) and what it adds to
/// the global scope, for [`merge_globals`].
pub struct FileDiagnostics {
    pub syntactic: Vec<SyntaxDiagnostic>,
    pub bind: Vec<SyntaxDiagnostic>,
    /// The checker's grammar check of each regular expression literal.
    pub regular_expressions: Vec<SyntaxDiagnostic>,
    /// What the checker's unused-identifier check reports, for a TypeScript
    /// file the options ask it of.
    pub unused: Vec<SyntaxDiagnostic>,
    pub globals: FileGlobals,
}

/// `GetSyntacticDiagnostics` of a JSON file (`parseJSONText`).
pub fn json_file_diagnostics(text: &str) -> Vec<SyntaxDiagnostic> {
    render(parser::parse_json(text).iter())
}

pub fn file_diagnostics(text: &str, options: &ParseOptions) -> FileDiagnostics {
    let parsed = parser::parse(text, options);
    let syntactic = render(parsed.diagnostics.iter().chain(parsed.js_diagnostics.iter()));
    if !parsed.diagnostics.is_empty() {
        return FileDiagnostics {
            syntactic,
            bind: Vec::new(),
            regular_expressions: Vec::new(),
            unused: Vec::new(),
            globals: FileGlobals::default(),
        };
    }
    let binder = binder::bind(&parsed, text);
    let unused = match &options.unused {
        Some(check) if !parsed.is_declaration_file => {
            render(binder.unused_diagnostics(check, options.language_version).iter())
        }
        _ => Vec::new(),
    };
    let (mut bind, globals) = binder.finish();
    bind.extend(checker_grammar::checker_grammar_diagnostics(&parsed, text));
    let regular_expressions: Vec<Diagnostic> = parsed
        .regular_expression_literals()
        .into_iter()
        .flat_map(|start| {
            regexp::regular_expression_literal_diagnostics(text, parsed.jsx, options.language_version, start)
        })
        .collect();
    FileDiagnostics {
        syntactic,
        bind: render(bind.iter()),
        regular_expressions: render(regular_expressions.iter()),
        unused,
        globals,
    }
}
