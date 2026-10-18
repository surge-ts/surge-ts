//! The native half of the `surge-ts` npm package: `surge_ts::api` exposed to
//! JavaScript as a program object whose methods take and return plain
//! numbers for files, nodes, types, symbols and signatures. The
//! TypeScript-shaped objects (`Node`, `Type`, `Symbol`, ...) are built in
//! JavaScript over these; see `packages/surge-ts`.
//!
//! Every method validates the handles it is given and throws a `TypeError`
//! for one this program never handed out. A panic inside the engine becomes
//! a JavaScript exception rather than an abort.

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{Map, Value};
use surge_ts::api::{
    self, CompilerHost, CompilerOptions, CreateProgramOptions, Diagnostic, DirectoryEntry, NodeId, NodePropertyValue,
    Program, SignatureId, SignatureKind, SourceFileId, SymbolId, TypeChecker, TypeId,
};

fn invalid(what: &str) -> Error {
    Error::new(Status::InvalidArg, format!("{what} does not belong to this program"))
}

fn unsupported(error: surge_ts::api::Unsupported) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}

fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|message| message.to_string()))
                .unwrap_or_else(|| "internal error".to_string());
            Err(Error::new(Status::GenericFailure, format!("surge-ts internal error: {message}")))
        }
    }
}

/// A JavaScript `CompilerHost`, called back on the thread that created the
/// program (the loader reads serially under a host, so no other thread
/// calls it).
struct JsHost {
    env: RawEnv,
    owner: std::thread::ThreadId,
    read_file: FunctionRef<String, Option<String>>,
    file_exists: FunctionRef<String, bool>,
    directory_exists: Option<FunctionRef<String, bool>>,
    get_directories: Option<FunctionRef<String, Vec<String>>>,
    realpath: Option<FunctionRef<String, String>>,
    current_directory: String,
    case_sensitive: bool,
    new_line: String,
}

struct RawEnv(napi::sys::napi_env);

// SAFETY: the environment is only dereferenced by `JsHost::env`, which
// asserts it runs on the thread that owns it.
unsafe impl Send for RawEnv {}
unsafe impl Sync for RawEnv {}

impl JsHost {
    fn env(&self) -> Env {
        assert_eq!(std::thread::current().id(), self.owner, "a JavaScript host was called from another thread");
        Env::from_raw(self.env.0)
    }
}

impl CompilerHost for JsHost {
    fn read_file(&self, path: &str) -> Option<String> {
        let env = self.env();
        self.read_file.borrow_back(&env).ok()?.call(path.to_string()).ok().flatten()
    }

    fn file_exists(&self, path: &str) -> bool {
        let env = self.env();
        self.file_exists
            .borrow_back(&env)
            .and_then(|function| function.call(path.to_string()))
            .unwrap_or(false)
    }

    fn directory_exists(&self, path: &str) -> bool {
        let env = self.env();
        match &self.directory_exists {
            Some(function) => function.borrow_back(&env).and_then(|function| function.call(path.to_string())).unwrap_or(false),
            // tsc's default for a host without `directoryExists`.
            None => true,
        }
    }

    fn read_directory_entries(&self, path: &str) -> Option<Vec<DirectoryEntry>> {
        let env = self.env();
        let directories = self.get_directories.as_ref()?.borrow_back(&env).ok()?.call(path.to_string()).ok()?;
        Some(
            directories
                .into_iter()
                .map(|name| DirectoryEntry { name, is_file: false, is_directory: true })
                .collect(),
        )
    }

    fn current_directory(&self) -> String {
        self.current_directory.clone()
    }

    fn realpath(&self, path: &str) -> String {
        let env = self.env();
        self.realpath
            .as_ref()
            .and_then(|function| function.borrow_back(&env).ok()?.call(path.to_string()).ok())
            .unwrap_or_else(|| path.to_string())
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        self.case_sensitive
    }

    fn new_line(&self) -> String {
        self.new_line.clone()
    }
}

/// The host callbacks the JavaScript layer passes: plain functions, already
/// bound to the user's host object.
#[napi(object)]
pub struct HostCallbacks<'env> {
    pub read_file: Function<'env, String, Option<String>>,
    pub file_exists: Function<'env, String, bool>,
    pub directory_exists: Option<Function<'env, String, bool>>,
    pub get_directories: Option<Function<'env, String, Vec<String>>>,
    pub realpath: Option<Function<'env, String, String>>,
    pub current_directory: String,
    pub use_case_sensitive_file_names: bool,
    pub new_line: String,
}

fn js_host(env: &Env, callbacks: HostCallbacks<'_>) -> Result<Arc<dyn CompilerHost>> {
    Ok(Arc::new(JsHost {
        env: RawEnv(env.raw()),
        owner: std::thread::current().id(),
        read_file: callbacks.read_file.create_ref()?,
        file_exists: callbacks.file_exists.create_ref()?,
        directory_exists: callbacks.directory_exists.map(|function| function.create_ref()).transpose()?,
        get_directories: callbacks.get_directories.map(|function| function.create_ref()).transpose()?,
        realpath: callbacks.realpath.map(|function| function.create_ref()).transpose()?,
        current_directory: callbacks.current_directory,
        case_sensitive: callbacks.use_case_sensitive_file_names,
        new_line: callbacks.new_line,
    }))
}

fn options_from_json(options: Option<Value>) -> CompilerOptions {
    match options {
        Some(Value::Object(map)) => CompilerOptions::from_json(map),
        _ => CompilerOptions::new(),
    }
}

/// A diagnostic as plain data; the JavaScript layer links `file` to its
/// `SourceFile`.
#[napi(object)]
pub struct DiagnosticRecord {
    pub file: Option<u32>,
    pub start: Option<u32>,
    pub length: Option<u32>,
    pub code: u32,
    pub category: i32,
    pub message_text: String,
    pub surge_code: Option<String>,
    pub file_name: Option<String>,
}

fn record(diagnostic: Diagnostic) -> DiagnosticRecord {
    DiagnosticRecord {
        file: diagnostic.file.map(SourceFileId::index),
        start: diagnostic.start,
        length: diagnostic.length,
        code: diagnostic.code,
        category: diagnostic.category as i32,
        message_text: diagnostic.message_text,
        surge_code: diagnostic.surge_code,
        file_name: diagnostic.file_name,
    }
}

/// A source file's syntax tree as flat arrays indexed by node index; the
/// children of node `i` are `children[childOffsets[i]..childOffsets[i + 1]]`
/// in `forEachChild` order.
#[napi(object)]
pub struct NodeTable {
    pub root: u32,
    pub kinds: Uint16Array,
    pub pos: Uint32Array,
    pub end: Uint32Array,
    pub start: Uint32Array,
    pub parent: Int32Array,
    pub flags: Int32Array,
    pub child_offsets: Uint32Array,
    pub children: Uint32Array,
}

#[napi(object)]
pub struct NodeProperty {
    pub name: String,
    /// The child node's index, for a node-valued property.
    pub node: Option<u32>,
    /// The nodes' indices, for a `NodeArray`.
    pub nodes: Option<Vec<u32>>,
    pub pos: Option<u32>,
    pub end: Option<u32>,
}

/// tsc's `TemplateLiteralType`: the texts around its placeholders and their types.
#[napi(object)]
pub struct TemplateLiteralRecord {
    pub texts: Vec<String>,
    pub types: Vec<u32>,
}

/// tsc's `TupleType` shape: each element's `ElementFlags`, and `readonly`.
#[napi(object)]
pub struct TupleShapeRecord {
    pub element_flags: Vec<i32>,
    pub readonly: bool,
    /// Each element's label, or `null` for an unlabeled one; absent when the
    /// tuple has none.
    pub labels: Option<Vec<Option<String>>>,
}

#[napi(object)]
pub struct NodeScalars {
    /// An identifier's or literal's text.
    pub text: Option<String>,
    /// A recorded operator or keyword, as a `SyntaxKind`.
    pub operator: Option<u32>,
    pub is_type_only: bool,
    pub is_export_equals: bool,
}

#[napi(object)]
pub struct SourceFileInfo {
    pub file_name: String,
    pub is_declaration_file: bool,
    pub is_default_library: bool,
    pub is_external_library: bool,
    pub language_version: i32,
    pub script_kind: i32,
    pub language_variant: i32,
}

#[napi(object)]
pub struct SymbolInfo {
    pub name: String,
    pub escaped_name: String,
    pub flags: i32,
}

#[napi(object)]
pub struct NodeRef {
    pub file: u32,
    pub node: u32,
}

fn node_ref(node: NodeId) -> NodeRef {
    NodeRef { file: node.source_file().index(), node: node.index() }
}

#[napi(object)]
pub struct CreateOptions<'env> {
    pub root_names: Vec<String>,
    pub options: Option<Value>,
    pub host: Option<HostCallbacks<'env>>,
    pub config_file_parsing_diagnostics: Option<Vec<DiagnosticRecord>>,
}

fn diagnostic_from_record(record: DiagnosticRecord) -> Diagnostic {
    Diagnostic {
        file: None,
        start: record.start,
        length: record.length,
        code: record.code,
        category: match record.category {
            0 => api::DiagnosticCategory::Warning,
            2 => api::DiagnosticCategory::Suggestion,
            3 => api::DiagnosticCategory::Message,
            _ => api::DiagnosticCategory::Error,
        },
        message_text: record.message_text,
        surge_code: record.surge_code,
        file_name: record.file_name,
    }
}

#[napi(custom_finalize)]
pub struct NativeProgram {
    program: Program,
    checker: TypeChecker,
    /// The native memory reported to V8 for this program, so a program no
    /// longer referenced is collected with the urgency of its size: the
    /// source text it holds, a lower bound of what the checked program keeps.
    external_bytes: i64,
}

impl NativeProgram {
    fn from_program(env: &Env, program: Program) -> Result<NativeProgram> {
        let external_bytes = program.source_files().iter().map(|file| program.text(*file).len() as i64).sum();
        env.adjust_external_memory(external_bytes)?;
        let checker = program.type_checker();
        Ok(NativeProgram { program, checker, external_bytes })
    }
}

impl ObjectFinalize for NativeProgram {
    fn finalize(self, env: Env) -> Result<()> {
        env.adjust_external_memory(-self.external_bytes)?;
        Ok(())
    }
}

#[napi]
impl NativeProgram {
    #[napi(factory)]
    pub fn create(env: Env, options: CreateOptions<'_>) -> Result<NativeProgram> {
        let host = options.host.map(|callbacks| js_host(&env, callbacks)).transpose()?;
        let config = options
            .config_file_parsing_diagnostics
            .unwrap_or_default()
            .into_iter()
            .map(diagnostic_from_record)
            .collect();
        let create = CreateProgramOptions {
            root_names: options.root_names,
            options: options_from_json(options.options),
            host,
            config_file_parsing_diagnostics: config,
        };
        guarded(|| {
            let program = Program::create(create).map_err(|error| Error::new(Status::GenericFailure, error.to_string()))?;
            NativeProgram::from_program(&env, program)
        })
    }

    /// `createSourceFile`: one text parsed with no program around it.
    #[napi(factory)]
    pub fn parse_source_file(env: Env, file_name: String, text: String) -> Result<NativeProgram> {
        guarded(|| NativeProgram::from_program(&env, Program::parse_source_file(&file_name, &text)))
    }

    fn file(&self, file: u32) -> Result<SourceFileId> {
        self.program.source_file_at(file).ok_or_else(|| invalid("source file"))
    }

    fn node(&self, file: u32, node: u32) -> Result<NodeId> {
        let file = self.file(file)?;
        self.program.node_at(file, node).ok_or_else(|| invalid("node"))
    }

    fn ty(&self, ty: u32) -> Result<TypeId> {
        self.checker.type_handle(ty).ok_or_else(|| invalid("type"))
    }

    fn symbol(&self, symbol: u32) -> Result<SymbolId> {
        self.checker.symbol_handle(symbol).ok_or_else(|| invalid("symbol"))
    }

    fn signature(&self, signature: u32) -> Result<SignatureId> {
        self.checker.signature_handle(signature).ok_or_else(|| invalid("signature"))
    }

    // --- program -------------------------------------------------------------

    #[napi]
    pub fn root_file_names(&self) -> Vec<String> {
        self.program.root_file_names().to_vec()
    }

    #[napi]
    pub fn current_directory(&self) -> String {
        self.program.current_directory().to_string()
    }

    #[napi]
    pub fn compiler_options(&self) -> Value {
        Value::Object(self.program.compiler_options().as_json().clone())
    }

    #[napi]
    pub fn source_file_count(&self) -> u32 {
        self.program.source_files().len() as u32
    }

    #[napi]
    pub fn source_file_index(&self, file_name: String) -> Option<u32> {
        self.program.source_file(&file_name).map(SourceFileId::index)
    }

    #[napi]
    pub fn source_file_info(&self, file: u32) -> Result<SourceFileInfo> {
        guarded(|| {
            let file = self.file(file)?;
            Ok(SourceFileInfo {
                file_name: self.program.file_name(file).to_string(),
                is_declaration_file: self.program.is_declaration_file(file),
                is_default_library: self.program.is_source_file_default_library(file),
                is_external_library: self.program.is_source_file_from_external_library(file),
                language_version: self.program.language_version(file),
                script_kind: self.program.script_kind(file),
                language_variant: self.program.language_variant(file),
            })
        })
    }

    #[napi]
    pub fn source_file_text(&self, file: u32) -> Result<String> {
        Ok(self.program.text(self.file(file)?).to_string())
    }

    #[napi]
    pub fn line_starts(&self, file: u32) -> Result<Uint32Array> {
        Ok(Uint32Array::new(self.program.line_starts(self.file(file)?).to_vec()))
    }

    #[napi]
    pub fn node_table(&self, file: u32) -> Result<NodeTable> {
        guarded(|| {
            let file = self.file(file)?;
            let root = self.program.root(file);
            let count = self.program.node_count(file);
            let mut kinds = vec![0u16; count];
            let mut pos = vec![0u32; count];
            let mut end = vec![0u32; count];
            let mut start = vec![0u32; count];
            let mut parent = vec![-1i32; count];
            let mut flags = vec![0i32; count];
            let mut child_offsets = vec![0u32; count + 1];
            let mut children = Vec::new();
            for index in 0..count as u32 {
                child_offsets[index as usize] = children.len() as u32;
                let Some(node) = self.program.node_at(file, index) else { continue };
                kinds[index as usize] = self.program.node_kind(node).0;
                pos[index as usize] = self.program.node_pos(node);
                end[index as usize] = self.program.node_end(node);
                start[index as usize] = self.program.node_start(node);
                flags[index as usize] = self.program.node_flags(node);
                if let Some(owner) = self.program.node_parent(node) {
                    parent[index as usize] = owner.index() as i32;
                }
                children.extend(self.program.node_children(node).into_iter().map(NodeId::index));
            }
            child_offsets[count] = children.len() as u32;
            Ok(NodeTable {
                root: root.index(),
                kinds: Uint16Array::new(kinds),
                pos: Uint32Array::new(pos),
                end: Uint32Array::new(end),
                start: Uint32Array::new(start),
                parent: Int32Array::new(parent),
                flags: Int32Array::new(flags),
                child_offsets: Uint32Array::new(child_offsets),
                children: Uint32Array::new(children),
            })
        })
    }

    #[napi]
    pub fn node_properties(&self, file: u32, node: u32) -> Result<Vec<NodeProperty>> {
        guarded(|| {
            let node = self.node(file, node)?;
            Ok(self
                .program
                .node_properties(node)
                .into_iter()
                .map(|(name, value)| match value {
                    NodePropertyValue::Node(child) => NodeProperty {
                        name: name.to_string(),
                        node: Some(child.index()),
                        nodes: None,
                        pos: None,
                        end: None,
                    },
                    NodePropertyValue::List { nodes, pos, end } => NodeProperty {
                        name: name.to_string(),
                        node: None,
                        nodes: Some(nodes.into_iter().map(NodeId::index).collect()),
                        pos: Some(pos),
                        end: Some(end),
                    },
                })
                .collect())
        })
    }

    #[napi]
    pub fn node_scalars(&self, file: u32, node: u32) -> Result<NodeScalars> {
        guarded(|| {
            let node = self.node(file, node)?;
            Ok(NodeScalars {
                text: self.program.node_value_text(node).map(str::to_string),
                operator: self.program.node_operator(node).map(|kind| u32::from(kind.0)),
                is_type_only: self.program.node_is_type_only(node),
                is_export_equals: self.program.node_is_export_equals(node),
            })
        })
    }

    #[napi]
    pub fn diagnostics(&self, kind: String, file: Option<u32>) -> Result<Vec<DiagnosticRecord>> {
        guarded(|| {
            let file = file.map(|file| self.file(file)).transpose()?;
            let diagnostics = match kind.as_str() {
                "syntactic" => self.program.syntactic_diagnostics(file),
                "semantic" => self.program.semantic_diagnostics(file),
                "declaration" => self.program.declaration_diagnostics(file),
                "global" => self.program.global_diagnostics(),
                "options" => self.program.options_diagnostics(),
                "config" => self.program.config_file_parsing_diagnostics(),
                "preEmit" => self.program.pre_emit_diagnostics(file),
                _ => return Err(Error::new(Status::InvalidArg, format!("unknown diagnostic set `{kind}`"))),
            };
            Ok(diagnostics.into_iter().map(record).collect())
        })
    }

    #[napi]
    pub fn resolved_module(&self, file: u32, specifier: String) -> Result<Option<u32>> {
        guarded(|| {
            let file = self.file(file)?;
            Ok(self.checker.resolved_module(file, &specifier).map(SourceFileId::index))
        })
    }

    // --- checker -------------------------------------------------------------

    #[napi]
    pub fn symbol_at_location(&self, file: u32, node: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.symbol_at_location(self.node(file, node)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn type_at_location(&self, file: u32, node: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.type_at_location(self.node(file, node)?).index()))
    }

    #[napi]
    pub fn type_of_symbol_at_location(&self, symbol: u32, file: u32, node: u32) -> Result<u32> {
        guarded(|| {
            let symbol = self.symbol(symbol)?;
            Ok(self.checker.type_of_symbol_at_location(symbol, self.node(file, node)?).index())
        })
    }

    #[napi]
    pub fn type_of_symbol(&self, symbol: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.type_of_symbol(self.symbol(symbol)?).index()))
    }

    #[napi]
    pub fn declared_type_of_symbol(&self, symbol: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.declared_type_of_symbol(self.symbol(symbol)?).index()))
    }

    #[napi]
    pub fn symbol_info(&self, symbol: u32) -> Result<SymbolInfo> {
        guarded(|| {
            let symbol = self.symbol(symbol)?;
            Ok(SymbolInfo {
                name: self.checker.symbol_name(symbol),
                escaped_name: self.checker.symbol_escaped_name(symbol),
                flags: self.checker.symbol_flags(symbol),
            })
        })
    }

    #[napi]
    pub fn symbol_declarations(&self, symbol: u32) -> Result<Vec<NodeRef>> {
        guarded(|| Ok(self.checker.symbol_declarations(self.symbol(symbol)?).into_iter().map(node_ref).collect()))
    }

    #[napi]
    pub fn symbol_value_declaration(&self, symbol: u32) -> Result<Option<NodeRef>> {
        guarded(|| Ok(self.checker.symbol_value_declaration(self.symbol(symbol)?).map(node_ref)))
    }

    #[napi]
    pub fn symbol_parent(&self, symbol: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.symbol_parent(self.symbol(symbol)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn symbol_members(&self, symbol: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.symbol_members(self.symbol(symbol)?).into_iter().map(SymbolId::index).collect()))
    }

    #[napi]
    pub fn symbol_exports(&self, symbol: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.symbol_exports(self.symbol(symbol)?).into_iter().map(SymbolId::index).collect()))
    }

    #[napi]
    pub fn exports_of_module(&self, symbol: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.exports_of_module(self.symbol(symbol)?).into_iter().map(SymbolId::index).collect()))
    }

    #[napi]
    pub fn aliased_symbol(&self, symbol: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.aliased_symbol(self.symbol(symbol)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn symbol_to_string(&self, symbol: u32) -> Result<String> {
        guarded(|| Ok(self.checker.symbol_to_string(self.symbol(symbol)?)))
    }

    #[napi]
    pub fn type_to_string(&self, ty: u32) -> Result<String> {
        guarded(|| Ok(self.checker.type_to_string(self.ty(ty)?)))
    }

    #[napi]
    pub fn type_flags(&self, ty: u32) -> Result<i32> {
        guarded(|| Ok(self.checker.type_flags(self.ty(ty)?)))
    }

    #[napi]
    pub fn object_flags(&self, ty: u32) -> Result<i32> {
        guarded(|| Ok(self.checker.object_flags(self.ty(ty)?)))
    }

    #[napi]
    pub fn type_symbol(&self, ty: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.type_symbol(self.ty(ty)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn alias_symbol(&self, ty: u32) -> Result<Option<u32>> {
        guarded(|| {
            let symbol = self.checker.alias_symbol(self.ty(ty)?).map_err(unsupported)?;
            Ok(symbol.map(SymbolId::index))
        })
    }

    #[napi]
    pub fn type_constituents(&self, ty: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.type_constituents(self.ty(ty)?).into_iter().map(TypeId::index).collect()))
    }

    /// A literal type's value: a string, or a number.
    #[napi]
    pub fn literal_value(&self, ty: u32) -> Result<Option<Value>> {
        guarded(|| {
            Ok(self.checker.literal_value(self.ty(ty)?).map(|value| match value {
                api::LiteralValue::String(text) => Value::String(text),
                api::LiteralValue::Number(number) => serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number),
            }))
        })
    }

    #[napi]
    pub fn properties_of_type(&self, ty: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.properties_of_type(self.ty(ty)?).into_iter().map(SymbolId::index).collect()))
    }

    #[napi]
    pub fn property_of_type(&self, ty: u32, name: String) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.property_of_type(self.ty(ty)?, &name).map(SymbolId::index)))
    }

    #[napi]
    pub fn index_type_of_type(&self, ty: u32, number: bool) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.index_type_of_type(self.ty(ty)?, number).map(TypeId::index)))
    }

    #[napi]
    pub fn signatures_of_type(&self, ty: u32, construct: bool) -> Result<Vec<u32>> {
        guarded(|| {
            let kind = if construct { SignatureKind::Construct } else { SignatureKind::Call };
            Ok(self.checker.signatures_of_type(self.ty(ty)?, kind).into_iter().map(SignatureId::index).collect())
        })
    }

    #[napi]
    pub fn apparent_type(&self, ty: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.apparent_type(self.ty(ty)?).index()))
    }

    #[napi]
    pub fn base_constraint_of_type(&self, ty: u32) -> Result<Option<u32>> {
        guarded(|| {
            let constraint = self.checker.base_constraint_of_type(self.ty(ty)?).map_err(unsupported)?;
            Ok(constraint.map(TypeId::index))
        })
    }

    #[napi]
    pub fn non_nullable_type(&self, ty: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.non_nullable_type(self.ty(ty)?).index()))
    }

    #[napi]
    pub fn base_type_of_literal_type(&self, ty: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.base_type_of_literal_type(self.ty(ty)?).index()))
    }

    #[napi]
    pub fn widened_type(&self, ty: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.widened_type(self.ty(ty)?).map_err(unsupported)?.index()))
    }

    #[napi]
    pub fn is_array_type(&self, ty: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_array_type(self.ty(ty)?)))
    }

    #[napi]
    pub fn is_tuple_type(&self, ty: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_tuple_type(self.ty(ty)?)))
    }

    #[napi]
    pub fn type_arguments(&self, ty: u32) -> Result<Vec<u32>> {
        guarded(|| Ok(self.checker.type_arguments(self.ty(ty)?).into_iter().map(TypeId::index).collect()))
    }

    #[napi]
    pub fn is_type_assignable_to(&self, source: u32, target: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_type_assignable_to(self.ty(source)?, self.ty(target)?)))
    }

    // --- TypeScript-internal queries -----------------------------------------

    #[napi]
    pub fn declaration_symbol(&self, file: u32, node: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.declaration_symbol(self.node(file, node)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn resolve_name(&self, name: String, file: Option<u32>, node: Option<u32>, meaning: i32) -> Result<Option<u32>> {
        guarded(|| {
            let location = match (file, node) {
                (Some(file), Some(node)) => Some(self.node(file, node)?),
                _ => None,
            };
            Ok(self.checker.resolve_name_at(location, &name, meaning).map(SymbolId::index))
        })
    }

    #[napi]
    pub fn shorthand_assignment_value_symbol(&self, file: u32, node: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.shorthand_assignment_value_symbol(self.node(file, node)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn immediate_aliased_symbol(&self, symbol: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.immediate_aliased_symbol(self.symbol(symbol)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn signature_this_parameter(&self, signature: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.signature_this_parameter(self.signature(signature)?).map(SymbolId::index)))
    }

    #[napi]
    pub fn template_literal_parts(&self, ty: u32) -> Result<Option<TemplateLiteralRecord>> {
        guarded(|| {
            Ok(self.checker.template_literal_parts(self.ty(ty)?).map(|(texts, types)| TemplateLiteralRecord {
                texts,
                types: types.into_iter().map(TypeId::index).collect(),
            }))
        })
    }

    #[napi]
    pub fn tuple_shape(&self, ty: u32) -> Result<Option<TupleShapeRecord>> {
        guarded(|| {
            Ok(self.checker.tuple_shape(self.ty(ty)?).map(|shape| TupleShapeRecord {
                element_flags: shape.element_flags,
                readonly: shape.readonly,
                labels: shape.labels,
            }))
        })
    }

    #[napi]
    pub fn contextual_type(&self, file: u32, node: u32) -> Result<Option<u32>> {
        guarded(|| Ok(self.checker.contextual_type(self.node(file, node)?).map(TypeId::index)))
    }

    #[napi]
    pub fn is_undefined_symbol(&self, symbol: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_undefined_symbol(self.symbol(symbol)?)))
    }

    #[napi]
    pub fn is_arguments_symbol(&self, symbol: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_arguments_symbol(self.symbol(symbol)?)))
    }

    #[napi]
    pub fn union_type(&self, types: Vec<u32>) -> Result<u32> {
        guarded(|| {
            let types = types.into_iter().map(|ty| self.ty(ty)).collect::<Result<Vec<_>>>()?;
            Ok(self.checker.union_type(&types).index())
        })
    }

    #[napi]
    pub fn array_type(&self, element: u32, readonly: bool) -> Result<u32> {
        guarded(|| Ok(self.checker.array_type(self.ty(element)?, readonly).index()))
    }

    #[napi]
    pub fn string_literal_type(&self, value: String) -> Result<u32> {
        guarded(|| Ok(self.checker.string_literal_type(&value).index()))
    }

    #[napi]
    pub fn number_literal_type(&self, value: String) -> Result<u32> {
        guarded(|| Ok(self.checker.number_literal_type(&value).index()))
    }

    #[napi]
    pub fn boolean_literal_type(&self, value: bool) -> Result<u32> {
        guarded(|| Ok(self.checker.boolean_literal_type(value).index()))
    }

    #[napi]
    pub fn is_array_like_type(&self, ty: u32) -> Result<bool> {
        guarded(|| Ok(self.checker.is_array_like_type(self.ty(ty)?)))
    }

    #[napi]
    pub fn intrinsic_type(&self, name: String) -> Result<u32> {
        guarded(|| {
            let checker = &self.checker;
            let ty = match name.as_str() {
                "any" => checker.any_type(),
                "unknown" => checker.unknown_type(),
                "string" => checker.string_type(),
                "number" => checker.number_type(),
                "boolean" => checker.boolean_type(),
                "bigint" => checker.bigint_type(),
                "symbol" => checker.es_symbol_type(),
                "undefined" => checker.undefined_type(),
                "null" => checker.null_type(),
                "void" => checker.void_type(),
                "never" => checker.never_type(),
                "error" => checker.error_type(),
                _ => return Err(Error::new(Status::InvalidArg, format!("unknown intrinsic type `{name}`"))),
            };
            Ok(ty.index())
        })
    }

    #[napi]
    pub fn return_type_of_signature(&self, signature: u32) -> Result<u32> {
        guarded(|| Ok(self.checker.return_type_of_signature(self.signature(signature)?).index()))
    }

    #[napi]
    pub fn signature_declaration(&self, signature: u32) -> Result<Option<NodeRef>> {
        guarded(|| Ok(self.checker.signature_declaration(self.signature(signature)?).map(node_ref)))
    }

    #[napi]
    pub fn signature_parameters(&self, signature: u32) -> Result<Vec<u32>> {
        guarded(|| {
            Ok(self.checker.signature_parameters(self.signature(signature)?).into_iter().map(SymbolId::index).collect())
        })
    }

    #[napi]
    pub fn signature_type_parameters(&self, signature: u32) -> Result<Vec<u32>> {
        guarded(|| {
            Ok(self
                .checker
                .signature_type_parameters(self.signature(signature)?)
                .into_iter()
                .map(TypeId::index)
                .collect())
        })
    }

    #[napi]
    pub fn signature_to_string(&self, signature: u32) -> Result<String> {
        guarded(|| Ok(self.checker.signature_to_string(self.signature(signature)?)))
    }
}

/// Each node kind's node-valued properties in `forEachChild` order, as
/// `{ [kind]: [[name, isNodeArray], ...] }`.
#[napi]
pub fn node_schema() -> Value {
    let mut out = Map::new();
    for (kind, properties) in api::node_schema() {
        let entries = properties
            .into_iter()
            .map(|(name, list)| Value::Array(vec![Value::String(name.to_string()), Value::Bool(list)]))
            .collect();
        out.insert(kind.0.to_string(), Value::Array(entries));
    }
    Value::Object(out)
}

/// The enums the API exports, as `{ name: [[member, value], ...] }` in
/// TypeScript's declaration order.
#[napi]
pub fn enums() -> Value {
    let mut out = Map::new();
    for (name, members) in api::ENUMS {
        let entries = members
            .iter()
            .map(|(member, value)| {
                let value = match value {
                    api::EnumValue::Number(number) => {
                        serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
                    }
                    api::EnumValue::String(text) => Value::String(text.to_string()),
                };
                Value::Array(vec![Value::String(member.to_string()), value])
            })
            .collect();
        out.insert(name.to_string(), Value::Array(entries));
    }
    Value::Object(out)
}

#[napi(object)]
pub struct ConfigText {
    pub config: Option<Value>,
    pub error: Option<DiagnosticRecord>,
}

#[napi]
pub fn parse_config_file_text(file_name: String, text: String) -> ConfigText {
    match api::parse_config_file_text(&file_name, &text) {
        Ok(config) => ConfigText { config: Some(config), error: None },
        Err(error) => ConfigText { config: None, error: Some(record(error)) },
    }
}

#[napi(object)]
pub struct ParsedConfig {
    pub options: Value,
    pub file_names: Vec<String>,
    pub errors: Vec<DiagnosticRecord>,
}

#[napi]
pub fn parse_json_config_file_content(
    json: Value,
    base_path: String,
    existing_options: Option<Value>,
    config_file_name: Option<String>,
) -> Result<ParsedConfig> {
    guarded(|| {
        let existing = existing_options.map(|options| options_from_json(Some(options)));
        let parsed = api::parse_json_config_file_content(&json, &base_path, existing.as_ref(), config_file_name.as_deref());
        Ok(ParsedConfig {
            options: Value::Object(parsed.options.as_json().clone()),
            file_names: parsed.file_names,
            errors: parsed.errors.into_iter().map(record).collect(),
        })
    })
}

/// `resolveModuleName`, answered by building nothing: the loader's resolution
/// of `specifier` from `containing_file` under `options`.
#[napi]
pub fn resolve_module_name(specifier: String, containing_file: String, options: Option<Value>) -> Result<Option<Value>> {
    guarded(|| {
        let resolved = api::resolve_module_name(&specifier, &containing_file, &options_from_json(options));
        Ok(resolved.map(|resolved| {
            let mut out = Map::new();
            out.insert("resolvedFileName".to_string(), Value::String(resolved.resolved_file_name));
            out.insert("extension".to_string(), Value::String(resolved.extension));
            out.insert("isExternalLibraryImport".to_string(), Value::Bool(resolved.is_external_library_import));
            Value::Object(out)
        }))
    })
}
