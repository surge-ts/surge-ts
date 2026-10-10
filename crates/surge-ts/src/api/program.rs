use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use surge_ts_checker::FileKind;
use surge_ts_checker::lowlevel::semantic::RetainedProgram;
use surge_ts_tsc_syntax::{BoundFile, ParseOptions, ScriptKind as TscScriptKind, SyntaxTree};
use surge_ts_types::fx::FxHashMap;

use super::checker::{SemanticTables, TypeChecker};
use super::diagnostics::{Diagnostic, convert_diagnostic};
use super::enums_generated::SyntaxKind;
use super::handles::{NodeId, SourceFileId, next_program_identity};
use super::host::{CompilerHost, SystemHost};
use super::options::CompilerOptions;
use super::positions::{LineAndCharacter, PositionMap};
use crate::{DiagnosticParts, Prepared, Project, ProjectOptions};

/// What [`Program::create`] builds a program from: TypeScript's
/// `CreateProgramOptions`.
#[derive(Clone, Default)]
pub struct CreateProgramOptions {
    pub root_names: Vec<String>,
    pub options: CompilerOptions,
    /// Where the program reads files from. `None` is the file system.
    pub host: Option<Arc<dyn CompilerHost>>,
    /// Diagnostics from parsing the config the options came from, reported by
    /// [`Program::config_file_parsing_diagnostics`].
    pub config_file_parsing_diagnostics: Vec<Diagnostic>,
}

/// A failure that keeps a program from being built. Diagnostics about the
/// program's source are never errors: they are what the program reports.
#[derive(Debug)]
pub enum ProgramError {
    /// A file the program includes could not be read.
    SourceRead { path: PathBuf, error: std::io::Error },
}

impl std::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProgramError::SourceRead { path, error } => write!(f, "failed to read {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for ProgramError {}

/// A parsed and type-checked program: TypeScript's `Program`.
///
/// Cloning is cheap and shares the program. Every handle a program hands out
/// is valid for as long as any clone of it is alive; passing a handle of one
/// program to another panics.
#[derive(Clone)]
pub struct Program {
    pub(crate) inner: Arc<ProgramInner>,
}

pub(crate) struct ProgramInner {
    pub(crate) identity: u32,
    pub(crate) options: CompilerOptions,
    pub(crate) current_directory: String,
    pub(crate) root_names: Vec<String>,
    pub(crate) files: Vec<SourceFileData>,
    pub(crate) file_by_name: FxHashMap<String, u32>,
    pub(crate) checked: Option<RetainedProgram>,
    diagnostics: ProgramDiagnostics,
    pub(crate) parse: ParseSettings,
    pub(crate) strict_null_checks: bool,
    pub(crate) semantics: Mutex<SemanticTables>,
}

pub(crate) struct SourceFileData {
    pub(crate) file_name: String,
    pub(crate) text: Arc<str>,
    pub(crate) kind: FileKind,
    /// The file's index in the checker's program.
    pub(crate) checker_index: Option<usize>,
    positions: OnceLock<PositionMap>,
    syntax: OnceLock<Arc<FileSyntax>>,
}

/// A file's syntax tree and what its binder declared.
pub(crate) struct FileSyntax {
    pub(crate) tree: SyntaxTree,
    pub(crate) bound: BoundFile,
}

/// What decides how a file parses, beyond its name: the options the
/// checker's own syntactic pass uses.
#[derive(Clone, Default)]
pub(crate) struct ParseSettings {
    language_version: surge_ts_checker::LanguageVersion,
    module_detection_force: bool,
    module_detection_legacy: bool,
    jsx_automatic_runtime: bool,
    esm_module_files: std::collections::HashSet<String>,
}

#[derive(Default)]
struct ProgramDiagnostics {
    config: Vec<Diagnostic>,
    options: Vec<surge_ts_diagnostics::Diagnostic>,
    global: Vec<surge_ts_diagnostics::Diagnostic>,
    syntax_errors: bool,
    by_file: FxHashMap<String, FileDiagnostics>,
}

#[derive(Default)]
struct FileDiagnostics {
    syntactic: Vec<surge_ts_diagnostics::Diagnostic>,
    semantic: Vec<surge_ts_diagnostics::Diagnostic>,
    declaration: Vec<surge_ts_diagnostics::Diagnostic>,
}

impl Program {
    /// TypeScript's `createProgram`: reads the root files and everything they
    /// reach, then checks the program. The configuration and resolution
    /// pipeline is the one the `surge` CLI runs.
    pub fn create(options: CreateProgramOptions) -> Result<Program, ProgramError> {
        let host: Arc<dyn CompilerHost> = options.host.unwrap_or_else(|| Arc::new(SystemHost::new()));
        let current_directory = host.current_directory();
        let base_dir = options
            .options
            .config_file_path()
            .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from(&current_directory));
        let (tsconfig_options, mut config_diagnostics) = options.options.to_tsconfig_json(&base_dir);
        let (compiler_options, normalize_diagnostics) =
            surge_ts_config::normalize_compiler_options_json(&tsconfig_options, &base_dir);
        config_diagnostics.extend(normalize_diagnostics);
        let files = options
            .root_names
            .iter()
            .map(|name| absolute(&current_directory, name))
            .collect();
        let loaded = surge_ts_config::LoadedTsConfig {
            config_path: options
                .options
                .config_file_path()
                .map(PathBuf::from)
                .unwrap_or_else(|| base_dir.join("tsconfig.json")),
            root_dir: base_dir,
            files,
            compiler_options,
            diagnostics: config_diagnostics,
            removed_options: Vec::new(),
        };
        let project = Project { loaded };
        let project_options = ProjectOptions {
            retain_all_sources: true,
            ..ProjectOptions::default()
        };
        let mut config = options.config_file_parsing_diagnostics;
        config.extend(project.loaded.diagnostics.iter().map(Diagnostic::from_config));
        crate::host_fs::with_host(host.clone(), || {
            build(project, &project_options, options.root_names, options.options, current_directory, config)
        })
    }

    /// TypeScript's `createSourceFile`: `text` parsed on its own, with no
    /// program around it. Its nodes are a program's nodes (the handles and
    /// accessors are the same), but nothing is resolved or checked: a
    /// checker of it answers every query with the error type.
    pub fn parse_source_file(file_name: &str, text: &str) -> Program {
        let file = SourceFileData::new(file_name.to_string(), Arc::from(text), FileKind::RootSource, None);
        let mut diagnostics = ProgramDiagnostics::default();
        let program = assemble(
            next_program_identity(),
            CompilerOptions::new(),
            String::new(),
            vec![file_name.to_string()],
            vec![file],
            None,
            ProgramDiagnostics::default(),
            ParseSettings::default(),
            false,
        );
        let id = program.file_id(0);
        let syntactic = program.syntax(id).tree.syntactic_diagnostics();
        diagnostics.syntax_errors = !syntactic.is_empty();
        let entry = diagnostics.by_file.entry(file_name.to_string()).or_default();
        for diagnostic in syntactic {
            entry.syntactic.push(
                surge_ts_diagnostics::Diagnostic::new(
                    surge_ts_diagnostics::DiagnosticCode::TypeScript(diagnostic.code),
                    diagnostic.message,
                    file_name.to_string(),
                )
                .with_span(surge_ts_diagnostics::TextSpan { start: diagnostic.start, end: diagnostic.end }),
            );
        }
        let inner = Arc::try_unwrap(program.inner).unwrap_or_else(|_| unreachable!("a fresh program has one owner"));
        Program { inner: Arc::new(ProgramInner { diagnostics, ..inner }) }
    }

    pub(crate) fn inner(&self) -> &ProgramInner {
        &self.inner
    }

    /// `getRootFileNames`.
    pub fn root_file_names(&self) -> &[String] {
        &self.inner.root_names
    }

    /// `getSourceFiles`: every file of the program, default libraries first.
    pub fn source_files(&self) -> Vec<SourceFileId> {
        (0..self.inner.files.len() as u32)
            .map(|index| SourceFileId { program: self.inner.identity, index })
            .collect()
    }

    /// `getSourceFile`: the file with this name, as the host spells it or as
    /// the program resolved it.
    pub fn source_file(&self, file_name: &str) -> Option<SourceFileId> {
        let index = self.inner.file_by_name.get(file_name).copied().or_else(|| {
            let absolute = absolute(&self.inner.current_directory, file_name);
            let canonical = surge_ts_config::canonicalize_if_exists_string(&absolute);
            self.inner
                .file_by_name
                .get(&canonical)
                .or_else(|| self.inner.file_by_name.get(absolute.to_string_lossy().as_ref()))
                .copied()
        })?;
        Some(SourceFileId { program: self.inner.identity, index })
    }

    /// `getCompilerOptions`.
    pub fn compiler_options(&self) -> &CompilerOptions {
        &self.inner.options
    }

    /// `getCurrentDirectory`.
    pub fn current_directory(&self) -> &str {
        &self.inner.current_directory
    }

    /// `getTypeChecker`.
    pub fn type_checker(&self) -> TypeChecker {
        TypeChecker::new(self.clone())
    }

    pub(crate) fn file(&self, file: SourceFileId) -> &SourceFileData {
        assert_eq!(file.program, self.inner.identity, "source file handle belongs to another program");
        &self.inner.files[file.index as usize]
    }

    pub(crate) fn file_id(&self, index: u32) -> SourceFileId {
        SourceFileId { program: self.inner.identity, index }
    }

    /// A file handle for the file at `index` in [`Self::source_files`].
    pub fn source_file_at(&self, index: u32) -> Option<SourceFileId> {
        ((index as usize) < self.inner.files.len()).then(|| self.file_id(index))
    }

    // --- source files --------------------------------------------------------

    /// `SourceFile.fileName`.
    pub fn file_name(&self, file: SourceFileId) -> &str {
        &self.file(file).file_name
    }

    /// `SourceFile.text`.
    pub fn text(&self, file: SourceFileId) -> &Arc<str> {
        &self.file(file).text
    }

    /// `SourceFile.isDeclarationFile`.
    pub fn is_declaration_file(&self, file: SourceFileId) -> bool {
        self.syntax(file).tree.is_declaration_file()
    }

    /// Whether the file is a module (tsc's `externalModuleIndicator`).
    pub fn is_external_module(&self, file: SourceFileId) -> bool {
        self.syntax(file).tree.is_external_module()
    }

    /// `SourceFile.languageVersion`, as a `ScriptTarget` value.
    pub fn language_version(&self, file: SourceFileId) -> i32 {
        let _ = file;
        self.inner.parse.language_version as i32
    }

    /// `SourceFile.scriptKind`, as a `ScriptKind` value.
    pub fn script_kind(&self, file: SourceFileId) -> i32 {
        use super::enums_generated::script_kind as sk;
        let name = &self.file(file).file_name;
        match TscScriptKind::from_file_name(name) {
            Some(TscScriptKind::Ts) => sk::TS,
            Some(TscScriptKind::Tsx) => sk::TSX,
            Some(TscScriptKind::Js) => sk::JS,
            Some(TscScriptKind::Jsx) => sk::JSX,
            None if name.to_ascii_lowercase().ends_with(".json") => sk::JSON,
            None => sk::Unknown,
        }
    }

    /// `SourceFile.languageVariant`: 1 for a JSX file.
    pub fn language_variant(&self, file: SourceFileId) -> i32 {
        i32::from(self.syntax(file).tree.is_jsx())
    }

    /// `isSourceFileDefaultLibrary`.
    pub fn is_source_file_default_library(&self, file: SourceFileId) -> bool {
        matches!(self.file(file).kind, FileKind::PhysicalDefaultLib | FileKind::GeneratedDeclaration)
    }

    /// `isSourceFileFromExternalLibrary`: a declaration file a package
    /// provided.
    pub fn is_source_file_from_external_library(&self, file: SourceFileId) -> bool {
        self.file(file).kind == FileKind::DependencyDeclaration
    }

    pub(crate) fn positions(&self, file: SourceFileId) -> &PositionMap {
        let data = self.file(file);
        data.positions.get_or_init(|| PositionMap::new(&data.text))
    }

    /// The UTF-16 length of the file's text.
    pub fn text_length(&self, file: SourceFileId) -> u32 {
        self.positions(file).utf16_len()
    }

    /// `getLineAndCharacterOfPosition`.
    pub fn line_and_character_of_position(&self, file: SourceFileId, position: u32) -> LineAndCharacter {
        self.positions(file).line_and_character(position)
    }

    /// `getPositionOfLineAndCharacter`; `None` when the line or character is
    /// out of range.
    pub fn position_of_line_and_character(&self, file: SourceFileId, line: u32, character: u32) -> Option<u32> {
        self.positions(file).position_of(line, character)
    }

    /// `getLineStarts`.
    pub fn line_starts(&self, file: SourceFileId) -> &[u32] {
        self.positions(file).line_starts()
    }

    pub(crate) fn syntax(&self, file: SourceFileId) -> &Arc<FileSyntax> {
        let data = self.file(file);
        data.syntax.get_or_init(|| {
            let options = self.inner.parse.parse_options(&data.file_name);
            let tree = SyntaxTree::parse(&data.text, &options);
            let bound = tree.bind(&data.text);
            Arc::new(FileSyntax { tree, bound })
        })
    }

    // --- nodes ---------------------------------------------------------------

    /// The file's `SourceFile` node.
    pub fn root(&self, file: SourceFileId) -> NodeId {
        let root = self.syntax(file).tree.root();
        NodeId { program: file.program, file: file.index, node: root }
    }

    /// The size of the file's node index space: every node index is below it.
    pub fn node_count(&self, file: SourceFileId) -> usize {
        self.syntax(file).tree.node_count()
    }

    /// A node handle for a node index of a file, if the file has such an
    /// attached node.
    pub fn node_at(&self, file: SourceFileId, index: u32) -> Option<NodeId> {
        let tree = &self.syntax(file).tree;
        ((index as usize) < tree.node_count() && tree.is_attached(index))
            .then_some(NodeId { program: file.program, file: file.index, node: index })
    }

    pub(crate) fn tree(&self, node: NodeId) -> &SyntaxTree {
        &self.syntax(node.source_file()).tree
    }

    pub fn node_kind(&self, node: NodeId) -> SyntaxKind {
        super::node::typescript_kind(self.tree(node), node.node)
    }

    /// `Node.pos`: where the node's leading trivia starts, in UTF-16 units.
    pub fn node_pos(&self, node: NodeId) -> u32 {
        self.positions(node.source_file()).to_utf16(self.tree(node).pos(node.node))
    }

    /// `Node.end`, in UTF-16 units.
    pub fn node_end(&self, node: NodeId) -> u32 {
        self.positions(node.source_file()).to_utf16(self.tree(node).end(node.node))
    }

    /// `Node.getStart()`: where the node's first token starts.
    pub fn node_start(&self, node: NodeId) -> u32 {
        let file = node.source_file();
        let start = self.tree(node).start(&self.file(file).text, node.node);
        self.positions(file).to_utf16(start)
    }

    pub(crate) fn node_byte_span(&self, node: NodeId) -> (usize, usize) {
        let tree = self.tree(node);
        (tree.start(&self.file(node.source_file()).text, node.node), tree.end(node.node))
    }

    /// `Node.parent`; `None` for a source file.
    pub fn node_parent(&self, node: NodeId) -> Option<NodeId> {
        let parent = self.tree(node).parent(node.node)?;
        Some(NodeId { node: parent, ..node })
    }

    /// The children `forEachChild` visits, in its order.
    pub fn node_children(&self, node: NodeId) -> Vec<NodeId> {
        self.node_properties(node)
            .into_iter()
            .flat_map(|(_, value)| match value {
                super::node::NodePropertyValue::Node(child) => vec![child],
                super::node::NodePropertyValue::List { nodes, .. } => nodes,
            })
            .collect()
    }

    /// `Node.flags`, as TypeScript's `NodeFlags` values.
    pub fn node_flags(&self, node: NodeId) -> i32 {
        let tree = self.tree(node);
        super::node::typescript_node_flags(tree.kind(node.node), tree.flags(node.node), tree.node(node.node).op)
    }

    /// `Node.getText()`: the node's source text without its leading trivia.
    pub fn node_text(&self, node: NodeId) -> &str {
        let (start, end) = self.node_byte_span(node);
        self.file(node.source_file()).text.get(start..end.max(start)).unwrap_or_default()
    }

    /// `Node.getFullText()`: the node's source text with its leading trivia.
    pub fn node_full_text(&self, node: NodeId) -> &str {
        let tree = self.tree(node);
        let (pos, end) = (tree.pos(node.node), tree.end(node.node));
        self.file(node.source_file()).text.get(pos..end.max(pos)).unwrap_or_default()
    }

    // --- diagnostics ---------------------------------------------------------

    fn convert(&self, diagnostics: &[surge_ts_diagnostics::Diagnostic]) -> Vec<Diagnostic> {
        diagnostics
            .iter()
            .map(|diagnostic| {
                let file = self.inner.file_by_name.get(&diagnostic.file_name).map(|&index| self.file_id(index));
                convert_diagnostic(diagnostic, file, file.map(|file| self.positions(file)))
            })
            .collect()
    }

    fn file_diagnostics(
        &self,
        file: Option<SourceFileId>,
        select: fn(&FileDiagnostics) -> &Vec<surge_ts_diagnostics::Diagnostic>,
    ) -> Vec<Diagnostic> {
        match file {
            Some(file) => {
                let name = &self.file(file).file_name;
                self.inner
                    .diagnostics
                    .by_file
                    .get(name)
                    .map(|diagnostics| self.convert(select(diagnostics)))
                    .unwrap_or_default()
            }
            None => (0..self.inner.files.len() as u32)
                .flat_map(|index| self.file_diagnostics(Some(self.file_id(index)), select))
                .collect(),
        }
    }

    /// `getSyntacticDiagnostics`.
    pub fn syntactic_diagnostics(&self, file: Option<SourceFileId>) -> Vec<Diagnostic> {
        self.file_diagnostics(file, |diagnostics| &diagnostics.syntactic)
    }

    /// `getSemanticDiagnostics`. Like `tsc`, surge does not type-check a
    /// program any of whose files has syntax errors, so this is empty then.
    pub fn semantic_diagnostics(&self, file: Option<SourceFileId>) -> Vec<Diagnostic> {
        self.file_diagnostics(file, |diagnostics| &diagnostics.semantic)
    }

    /// `getDeclarationDiagnostics`: what declaration emit reports under
    /// `isolatedDeclarations`.
    pub fn declaration_diagnostics(&self, file: Option<SourceFileId>) -> Vec<Diagnostic> {
        self.file_diagnostics(file, |diagnostics| &diagnostics.declaration)
    }

    /// `getGlobalDiagnostics`.
    pub fn global_diagnostics(&self) -> Vec<Diagnostic> {
        self.convert(&self.inner.diagnostics.global)
    }

    /// `getOptionsDiagnostics`.
    pub fn options_diagnostics(&self) -> Vec<Diagnostic> {
        self.convert(&self.inner.diagnostics.options)
    }

    /// `getConfigFileParsingDiagnostics`.
    pub fn config_file_parsing_diagnostics(&self) -> Vec<Diagnostic> {
        self.inner.diagnostics.config.clone()
    }

    /// Whether some file failed to parse, in which case no file was
    /// type-checked.
    pub fn has_syntax_errors(&self) -> bool {
        self.inner.diagnostics.syntax_errors
    }

    /// `ts.getPreEmitDiagnostics`: config, options, syntactic, global and
    /// semantic diagnostics, then declaration diagnostics when the options
    /// emit declarations; sorted and deduplicated as tsc does.
    pub fn pre_emit_diagnostics(&self, file: Option<SourceFileId>) -> Vec<Diagnostic> {
        let mut diagnostics = self.config_file_parsing_diagnostics();
        diagnostics.extend(self.options_diagnostics());
        diagnostics.extend(self.syntactic_diagnostics(file));
        diagnostics.extend(self.global_diagnostics());
        diagnostics.extend(self.semantic_diagnostics(file));
        if self.inner.options.emits_declarations() {
            diagnostics.extend(self.declaration_diagnostics(file));
        }
        super::diagnostics::sort_and_deduplicate(self, diagnostics)
    }
}

/// `path` against `current_directory`, normalized.
fn absolute(current_directory: &str, path: &str) -> PathBuf {
    surge_ts_config::normalize_path_buf(&surge_ts_config::resolve_path(Path::new(current_directory), path))
}

impl ParseSettings {
    fn parse_options(&self, file_name: &str) -> ParseOptions {
        let mut options = ParseOptions::for_file_name(file_name).unwrap_or(ParseOptions {
            script_kind: TscScriptKind::Ts,
            is_declaration_file: false,
            force_module: false,
            jsx_forces_module: false,
            language_version: Default::default(),
            unused: None,
        });
        options.language_version = self.language_version;
        if !options.is_declaration_file {
            if self.module_detection_legacy {
                options.force_module = false;
            } else if self.module_detection_force {
                options.force_module = true;
            } else {
                options.force_module |= self.esm_module_files.contains(file_name);
                options.jsx_forces_module = self.jsx_automatic_runtime;
            }
        }
        options
    }
}

fn build(
    project: Project,
    project_options: &ProjectOptions,
    root_names: Vec<String>,
    options: CompilerOptions,
    current_directory: String,
    config: Vec<Diagnostic>,
) -> Result<Program, ProgramError> {
    let identity = next_program_identity();
    let prepared = project.prepare(project_options).map_err(|error| match error {
        crate::ProjectError::SourceRead { path, error } => ProgramError::SourceRead { path, error },
    })?;
    let (run, finish) = match prepared {
        Prepared::Empty(result) => {
            let mut diagnostics = ProgramDiagnostics { config, ..ProgramDiagnostics::default() };
            diagnostics.options.extend(result.diagnostics);
            return Ok(assemble(identity, options, current_directory, root_names, Vec::new(), None, diagnostics, ParseSettings::default(), false));
        }
        Prepared::Ready(run, finish) => (run, finish),
    };
    let checker_options = &run.checker_options;
    let parse = ParseSettings {
        language_version: checker_options.language_version,
        module_detection_force: checker_options.module_detection.force,
        module_detection_legacy: checker_options.module_detection.legacy,
        jsx_automatic_runtime: checker_options.jsx_automatic_runtime,
        esm_module_files: checker_options.esm_module_files.clone(),
    };
    let strict_null_checks = checker_options.strict_null_checks;
    let checking_start = std::time::Instant::now();
    let (result, checked) = RetainedProgram::check(run.inputs, run.prescanned, run.checker_options, project_options.jobs);
    let (parts, sources, _warnings, _timings) = finish.into_parts(result, checking_start.elapsed(), project_options);

    let texts: FxHashMap<String, Arc<str>> = sources
        .into_iter()
        .map(|(_, file_name, text)| (file_name, Arc::from(text)))
        .collect();
    let mut files = Vec::new();
    match &checked {
        Some(checked) => {
            for (checker_index, file_name) in checked.file_names().iter().enumerate() {
                let Some(text) = texts.get(file_name) else { continue };
                files.push(SourceFileData::new(
                    file_name.clone(),
                    text.clone(),
                    checked.file_kind(checker_index).unwrap_or(FileKind::RootSource),
                    Some(checker_index),
                ));
            }
        }
        None => {
            for (file_name, text) in &texts {
                files.push(SourceFileData::new(file_name.clone(), text.clone(), FileKind::RootSource, None));
            }
        }
    }
    let diagnostics = sort_diagnostics(parts, config);
    Ok(assemble(identity, options, current_directory, root_names, files, checked, diagnostics, parse, strict_null_checks))
}

fn sort_diagnostics(parts: DiagnosticParts, config: Vec<Diagnostic>) -> ProgramDiagnostics {
    let mut sorted = ProgramDiagnostics {
        config,
        options: parts.config.into_iter().chain(parts.program).collect(),
        syntax_errors: parts.syntax_errors,
        ..ProgramDiagnostics::default()
    };
    for diagnostic in parts.checked {
        if diagnostic.file_name.is_empty() {
            if !parts.syntax_errors {
                sorted.global.push(diagnostic);
            }
            continue;
        }
        let entry = sorted.by_file.entry(diagnostic.file_name.clone()).or_default();
        if parts.syntax_errors {
            entry.syntactic.push(diagnostic);
        } else {
            entry.semantic.push(diagnostic);
        }
    }
    for diagnostic in parts.declaration {
        sorted.by_file.entry(diagnostic.file_name.clone()).or_default().declaration.push(diagnostic);
    }
    sorted
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    identity: u32,
    options: CompilerOptions,
    current_directory: String,
    root_names: Vec<String>,
    files: Vec<SourceFileData>,
    checked: Option<RetainedProgram>,
    diagnostics: ProgramDiagnostics,
    parse: ParseSettings,
    strict_null_checks: bool,
) -> Program {
    let mut file_by_name = FxHashMap::default();
    for (index, file) in files.iter().enumerate() {
        file_by_name.insert(file.file_name.clone(), index as u32);
    }
    Program {
        inner: Arc::new(ProgramInner {
            identity,
            options,
            current_directory,
            root_names,
            files,
            file_by_name,
            checked,
            diagnostics,
            parse,
            strict_null_checks,
            semantics: Mutex::new(SemanticTables::default()),
        }),
    }
}

impl SourceFileData {
    fn new(file_name: String, text: Arc<str>, kind: FileKind, checker_index: Option<usize>) -> Self {
        SourceFileData {
            file_name,
            text,
            kind,
            checker_index,
            positions: OnceLock::new(),
            syntax: OnceLock::new(),
        }
    }
}
