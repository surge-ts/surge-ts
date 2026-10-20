//! `resolveModuleName`: one specifier resolved from one file, by the loader's
//! own resolvers, outside any program.

use std::path::{Path, PathBuf};

use super::options::CompilerOptions;

/// TypeScript's `ResolvedModuleFull`, kept to what surge's resolver knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModule {
    pub resolved_file_name: String,
    /// The resolved file's extension, as TypeScript's `Extension` spells it.
    pub extension: String,
    pub is_external_library_import: bool,
}

/// TypeScript's `Extension` of a file name.
pub(crate) fn extension_of(file_name: &str) -> String {
    const EXTENSIONS: &[&str] =
        &[".d.ts", ".d.mts", ".d.cts", ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"];
    let lower = file_name.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|extension| lower.ends_with(*extension))
        .map_or_else(String::new, |extension| extension.to_string())
}

/// A `paths` specifier tsc lands on a JavaScript file under `allowJs`, which
/// the import graph (and so the resolution it shares) does not load: the
/// target as written, or with a JavaScript extension appended.
fn javascript_through_paths(specifier: &str, loaded: &surge_ts_config::LoadedTsConfig) -> Option<PathBuf> {
    let options = &loaded.compiler_options;
    if !options.allow_js {
        return None;
    }
    let base = options.base_url.as_deref().unwrap_or(&loaded.root_dir);
    let is_javascript = |path: &Path| {
        let name = path.to_string_lossy();
        [".js", ".jsx", ".mjs", ".cjs"].iter().any(|extension| name.ends_with(extension))
    };
    surge_ts_config::select_path_mapping_targets(specifier, &options.paths)?.into_iter().find_map(|target| {
        let joined = base.join(&target);
        if is_javascript(&joined) && joined.is_file() {
            return Some(joined);
        }
        [".js", ".jsx"]
            .into_iter()
            .map(|extension| PathBuf::from(format!("{}{extension}", joined.to_string_lossy())))
            .find(|candidate| candidate.is_file())
    })
}

/// `resolveModuleName`: where `specifier`, imported from `containing_file`,
/// resolves under `options`; relative paths and `paths` among the files on
/// disk, bare specifiers through `node_modules` packages. `None` when nothing
/// resolves.
pub fn resolve_module_name(specifier: &str, containing_file: &str, options: &CompilerOptions) -> Option<ResolvedModule> {
    let containing = PathBuf::from(containing_file);
    let base_dir = options
        .config_file_path()
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
        .or_else(|| containing.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let (json, mut diagnostics) = options.to_tsconfig_json(&base_dir);
    let (compiler_options, normalize_diagnostics) = surge_ts_config::normalize_compiler_options_json(&json, &base_dir);
    diagnostics.extend(normalize_diagnostics);
    let loaded = surge_ts_config::LoadedTsConfig {
        config_path: base_dir.join("tsconfig.json"),
        root_dir: base_dir.clone(),
        files: Vec::new(),
        compiler_options,
        diagnostics,
        removed_options: Vec::new(),
        project_references: Vec::new(),
    };
    let resolver = crate::resolver_options(&loaded);
    let on_disk = crate::import_graph::resolve_specifier_on_disk(
        &containing,
        specifier,
        loaded.compiler_options.resolve_json_module,
        &loaded.compiler_options.paths,
        loaded.compiler_options.base_url.as_deref(),
        &loaded.root_dir,
    );
    let resolved = match on_disk.or_else(|| javascript_through_paths(specifier, &loaded)) {
        Some(path) => path,
        None => crate::package_declarations::resolve_package_specifier(specifier, &containing, &resolver, &base_dir)?.0,
    };
    // tsgo's `createResolvedModuleHandlingSymlink`: only an external library
    // import is its real path; a project file keeps the path it was reached by.
    let lexical = surge_ts_config::normalize_path_buf(&resolved);
    let resolved_file_name = if lexical.to_string_lossy().replace('\\', "/").contains("/node_modules/")
        && !loaded.compiler_options.preserve_symlinks
    {
        surge_ts_config::canonicalize_if_exists_string(&resolved)
    } else {
        lexical.to_string_lossy().into_owned()
    }
    .replace('\\', "/");
    Some(ResolvedModule {
        extension: extension_of(&resolved_file_name),
        is_external_library_import: resolved_file_name.contains("/node_modules/"),
        resolved_file_name,
    })
}
