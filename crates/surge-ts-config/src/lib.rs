//! tsconfig loading and normalization.

mod diagnostics;
mod extends;
mod files;
mod mapping;
mod model;
mod normalize;
mod options;
mod parse;
mod paths;
mod references;
mod removed_options;

pub use diagnostics::*;
pub use mapping::select_path_mapping_targets;
pub use model::*;
pub use options::*;
pub use references::ProjectReferenceOutputs;
pub use parse::{load_tsconfig, load_tsconfig_from_value, normalize_compiler_options_json, parse_config_text, project_reference_config_files};
pub use removed_options::{
    InvalidEnumOption, RemovedCompilerOption, invalid_enum_options, non_relative_path_substitutions, option_value_range,
};
pub use paths::{
    CanonicalizeIoSnapshot, absolutize, canonicalize_if_exists, canonicalize_if_exists_string,
    canonicalize_io_snapshot, clear_canonicalize_cache, cycle_key, normalize_path_buf,
    normalize_path_string, resolve_path, resolve_project_path,
};

#[cfg(test)]
mod tests;
