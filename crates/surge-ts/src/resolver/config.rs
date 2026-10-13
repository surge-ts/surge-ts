//! The resolver's options from a loaded config, with tsgo's defaults.

use std::path::Path;

use surge_ts_config::{LoadedTsConfig, ModuleResolutionKind};

use super::{ResolutionKind, ResolverConfig};

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn resolver_config(loaded: &LoadedTsConfig) -> ResolverConfig {
    let options = &loaded.compiler_options;
    let current_directory = std::env::current_dir().map(|path| path_string(&path)).unwrap_or_default();
    ResolverConfig {
        kind: match options.module_resolution {
            ModuleResolutionKind::Node16 | ModuleResolutionKind::Node20 => ResolutionKind::Node16,
            ModuleResolutionKind::NodeNext => ResolutionKind::NodeNext,
            ModuleResolutionKind::Bundler => ResolutionKind::Bundler,
        },
        resolve_package_json_exports: Some(options.resolve_package_json_exports),
        resolve_package_json_imports: Some(options.resolve_package_json_imports),
        custom_conditions: options.custom_conditions.clone(),
        no_dts_resolution: options.no_dts_resolution,
        resolve_json_module: options.resolve_json_module,
        allow_js: options.allow_js || options.check_js == Some(true),
        paths: options
            .paths
            .iter()
            .map(|mapping| (mapping.pattern.clone(), mapping.substitutions.clone()))
            .collect(),
        paths_base_path: options
            .paths_base_path
            .as_deref()
            .map(path_string)
            .unwrap_or_else(|| current_directory.clone()),
        root_dirs: options.root_dirs.iter().map(|path| path_string(path)).collect(),
        type_roots: (!options.type_roots.is_empty())
            .then(|| options.type_roots.iter().map(|path| path_string(path)).collect()),
        types: options.types.clone(),
        config_file_path: Some(path_string(&loaded.config_path)),
        current_directory,
        preserve_symlinks: options.preserve_symlinks,
        module_suffixes: options.module_suffixes.clone(),
        out_dir: options.out_dir.as_deref().map(path_string),
        declaration_dir: options.declaration_dir.as_deref().map(path_string),
        root_dir: options.root_dir.as_deref().map(path_string),
    }
}
