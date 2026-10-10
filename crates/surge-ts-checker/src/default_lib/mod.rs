pub(crate) mod embedded;
mod loader;
mod physical;
mod provider;
mod source;

pub use embedded::{EMBEDDED_LIB_DIR, EmbeddedLib, bundled_typescript_version, embedded_lib_source};
pub use loader::{
    DefaultLibLoad, DefaultLibRequest, LibSourceChoice, load_default_lib_inputs,
    load_generated_default_lib_inputs,
};
pub(crate) use physical::is_physical_default_lib_file_name;
pub use physical::{
    DefaultLibIoStats, PhysicalLibResolution, default_full_lib_seed_for_target,
    find_typescript_lib_dir, reference_lib_directives,
};
pub use provider::{DirectoryLibSource, EmbeddedLibSource, LibSource};
pub(crate) use source::is_generated_default_lib_file_name;
