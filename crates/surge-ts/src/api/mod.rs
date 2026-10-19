//! A TypeScript-compatible compiler API over surge's checker: programs,
//! source files and their syntax trees, and a type checker.
//!
//! The model is TypeScript's own (`createProgram`, `Program`, `SourceFile`,
//! `TypeChecker`), expressed with opaque handles: a [`NodeId`], [`TypeId`],
//! [`SymbolId`] or [`SignatureId`] names something inside one [`Program`] and
//! is answered by that program. Positions are UTF-16 offsets, as TypeScript
//! reports them. What is and is not supported, method by method, is in
//! `docs/COMPILER_API.md`.

mod checker;
mod classify;
mod config;
mod diagnostics;
mod enums_generated;
mod handles;
mod host;
mod internal;
mod node;
mod options;
mod positions;
mod program;
mod resolution;
mod symbols;

pub use checker::{ConditionalParts, LiteralValue, SignatureKind, TypeChecker, Unsupported};

/// The TypeScript release this API presents itself as (its enums, messages
/// and syntax are typescript@6.0.3's).
pub const EMULATED_TYPESCRIPT_VERSION: &str = "6.0.3";

/// Resolve packages for the rest of the process as
/// [`EMULATED_TYPESCRIPT_VERSION`] does: `typesVersions` and `types@` ranges
/// test against it. For a process that hosts the API alone (the Node binding);
/// the first call wins.
pub fn emulate_typescript_resolution() {
    crate::package_resolution::emulate_typescript_version(EMULATED_TYPESCRIPT_VERSION);
}
pub use config::{ParsedCommandLine, find_config_file, parse_config_file_text, parse_json_config_file_content, read_config_file};
pub use diagnostics::{Diagnostic, DiagnosticCategory, format_diagnostic};
pub use enums_generated::{
    ENUMS, EnumValue, SyntaxKind, jsx_emit, modifier_flags, module_detection_kind, module_kind, module_resolution_kind,
    node_flags, object_flags, script_kind, script_target, symbol_flags, type_flags,
};
pub use handles::{NodeId, SignatureId, SourceFileId, SymbolId, TypeId};
pub use host::{CompilerHost, DirectoryEntry, SystemHost};
pub use node::{NodePropertyValue, node_schema};
pub use options::CompilerOptions;
pub use positions::LineAndCharacter;
pub use program::{CreateProgramOptions, Program, ProgramError};
pub use resolution::{ResolvedModule, resolve_module_name};
