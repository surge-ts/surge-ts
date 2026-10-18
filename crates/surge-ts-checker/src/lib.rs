//! Type checker for TypeScript, embeddable as a library.
//!
//! The stable entry point is [`Checker`], a builder that takes source files and
//! returns a [`CheckResult`] (diagnostics plus tsc-compatibility stats):
//!
//! ```
//! use surge_ts_checker::{Checker, SourceFileInput};
//!
//! let result = Checker::new().check(vec![SourceFileInput {
//!     file_name: "index.ts".to_string(),
//!     source_text: "let x: string = 1;".to_string(),
//! }]);
//! assert!(!result.diagnostics.is_empty());
//! ```
//!
//! Lower-level building blocks (default-lib loading and resolution) live in
//! [`lowlevel`] and are not covered by the stable-API guarantees.

mod api;
mod checks;
mod context;
mod default_lib;
mod driver;
mod flow;
mod infer;
mod metrics;
mod modules;
mod paths;
mod program;
mod replay;
mod semantic;
mod spans;
mod speculative;
mod symbols;
#[cfg(test)]
mod test_support;

pub use api::{CheckResult, Checker};
pub use context::{
    CheckerOptions, CompatibilityStats, DiagnosticProfile, FileKind, JsxFactoryNames,
    ModuleDetection, ModuleEmitKind,
};
pub use program::set_fast_process_exit;
pub use surge_ts_tsc_syntax::ScriptTarget as LanguageVersion;
pub use program::{ProgramCheckResult, SourceFileInput};

/// Diagnostic types are re-exported so embedders can read [`Checker`] output
/// (code, message, span, severity) without depending on `surge-ts-diagnostics`
/// directly.
pub use surge_ts_diagnostics::{Diagnostic, DiagnosticCategory, DiagnosticCode, TextSpan};

#[doc(hidden)]
pub use driver::{check_source, check_source_with_options};
#[doc(hidden)]
pub use program::{
    check_program, check_program_with_options, check_program_with_stats,
    check_program_with_stats_and_jobs,
};

/// Lower-level building blocks for custom checking pipelines: default-lib
/// loading, resolution, and the seed catalog. Not part of the stable API
/// surface — these shapes can change without a major-version bump.
pub mod lowlevel {
    pub use crate::default_lib::{
        DefaultLibIoStats, DefaultLibLoad, DefaultLibRequest, DirectoryLibSource, EMBEDDED_LIB_DIR,
        EmbeddedLib, EmbeddedLibSource, LibSource, LibSourceChoice, PhysicalLibResolution,
        bundled_typescript_version, default_full_lib_seed_for_target, embedded_lib_source, find_typescript_lib_dir,
        load_default_lib_inputs, load_generated_default_lib_inputs, reference_lib_directives,
    };
    pub use crate::metrics::{last_rss_stage_label, record_loader_rss_stage};
    pub use crate::program::check_program_with_prescanned_sources;

    /// A checked program kept alive for queries: the compiler API's backing
    /// store. See [`crate::semantic`].
    pub mod semantic {
        pub use crate::semantic::{FileSemanticIndex, PropertyInfo, Query, RetainedProgram, settled_read};
    }

    /// Centralized relative-path candidate generation shared by the loader's
    /// import-graph/`paths` resolution and the checker's module binding, so the
    /// extension-substitution matrix cannot drift between layers.
    pub mod resolution_candidates {
        pub use crate::modules::candidates::{
            RelativeSpecifierShape, classify_relative_specifier, directory_index_candidates,
            extensionless_candidates, mapped_target_candidates, relative_import_candidates,
            relative_specifier_names_directory, resolution_mode_override_key, strip_extension,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_profile_is_public_api() {
        let options = CheckerOptions {
            diagnostic_profile: DiagnosticProfile::Native,
            types: Vec::new(),
            ..CheckerOptions::default()
        };
        assert_eq!(options.diagnostic_profile, DiagnosticProfile::Native);
    }
}
