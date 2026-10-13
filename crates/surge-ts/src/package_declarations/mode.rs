//! Resolution modes: whether a module specifier resolves against a package's
//! `import` or `require` face. Ported from tsgo's program-side selection
//! (`loadSourceFileMetaData`, `GetImpliedNodeFormatForFile`,
//! `GetImpliedNodeFormatForEmitWorker`, `getDefaultResolutionModeForFile`,
//! `getModeForUsageLocation`) and the resolver's `GetConditions`.

use super::*;
use surge_ts_config::{ModuleKind, ModuleResolutionKind};

/// tsc's `ResolutionMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ResolutionMode {
    None,
    CommonJs,
    Esm,
}

/// How a module specifier is written, which is what tsc derives a usage's
/// mode from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ImportUsage {
    /// An `import`/`export ... from` declaration or an `import("...")`.
    Declaration,
    /// `import x = require("...")`: always a CommonJS-mode resolution.
    ImportEquals,
    /// An `import type`/`export type` whose `resolution-mode` attribute
    /// names the mode.
    ModeOverride(surge_ts_syntax::ResolutionModeOverride),
    /// A dynamic `import("...")` expression.
    DynamicImport,
    /// A JavaScript `require("...")` call.
    Require,
}

struct SourceFileMetaData {
    package_json_type: Option<String>,
    implied_node_format: ResolutionMode,
}

fn is_node_resolution(opts: &ResolverOptions) -> bool {
    matches!(
        opts.module_resolution,
        ModuleResolutionKind::Node16 | ModuleResolutionKind::Node20 | ModuleResolutionKind::NodeNext
    )
}

fn ends_with_any(file: &str, extensions: &[&str]) -> bool {
    extensions.iter().any(|extension| file.ends_with(extension))
}

/// tsgo's `loadSourceFileMetaData`: the package.json scope's `type` only
/// counts for a file under node16/nodenext whose extension does not fix its
/// format, or for any file inside `node_modules`.
fn source_file_metadata(
    file: &Path,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
) -> SourceFileMetaData {
    let name = file.to_string_lossy();
    let type_counts = (!ends_with_any(&name, &[".mts", ".cts", ".mjs", ".cjs"])
        && is_node_resolution(opts))
        || name.contains("/node_modules/");
    let package_json_type = if type_counts {
        file.parent()
            .and_then(|directory| package_scope_type(directory, cache))
    } else {
        None
    };
    SourceFileMetaData {
        implied_node_format: implied_node_format_for_file(&name, package_json_type.as_deref()),
        package_json_type,
    }
}

/// tsgo's `GetPackageScopeForPath` read for its `type` field: the nearest
/// ancestor directory holding a `package.json` is the scope, parseable or not.
fn package_scope_type(
    directory: &Path,
    cache: &mut PackageDeclarationResolverCache,
) -> Option<String> {
    let mut current = Some(directory);
    while let Some(directory) = current {
        let manifest = directory.join("package.json");
        if crate::probe::is_existing_file(&manifest) {
            return read_package_json(&manifest, cache)
                .and_then(|json| json.get("type")?.as_str().map(str::to_string));
        }
        current = directory.parent();
    }
    None
}

/// tsgo's `GetImpliedNodeFormatForFile`.
fn implied_node_format_for_file(file: &str, package_json_type: Option<&str>) -> ResolutionMode {
    if ends_with_any(file, &[".d.mts", ".mts", ".mjs"]) {
        ResolutionMode::Esm
    } else if ends_with_any(file, &[".d.cts", ".cts", ".cjs"]) {
        ResolutionMode::CommonJs
    } else if ends_with_any(file, &[".d.ts", ".ts", ".tsx", ".js", ".jsx"]) {
        if package_json_type == Some("module") {
            ResolutionMode::Esm
        } else {
            ResolutionMode::CommonJs
        }
    } else {
        ResolutionMode::None
    }
}

/// tsgo's `GetImpliedNodeFormatForEmitWorker`.
fn implied_node_format_for_emit(
    file: &str,
    emit_module: ModuleKind,
    meta: &SourceFileMetaData,
) -> ResolutionMode {
    if matches!(
        emit_module,
        ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 | ModuleKind::NodeNext
    ) {
        return meta.implied_node_format;
    }
    let package_type = meta.package_json_type.as_deref();
    match meta.implied_node_format {
        ResolutionMode::CommonJs
            if package_type == Some("commonjs") || ends_with_any(file, &[".cjs", ".cts"]) =>
        {
            ResolutionMode::CommonJs
        }
        ResolutionMode::Esm
            if package_type == Some("module") || ends_with_any(file, &[".mjs", ".mts"]) =>
        {
            ResolutionMode::Esm
        }
        _ => ResolutionMode::None,
    }
}

/// tsgo's `importSyntaxAffectsModuleResolution`.
fn import_syntax_affects_module_resolution(opts: &ResolverOptions) -> bool {
    is_node_resolution(opts) || opts.resolve_exports || opts.resolve_imports
}

/// tsgo's `getModeForUsageLocation`: a `resolution-mode` attribute names the
/// mode outright, `import x = require()` is CommonJS, and a declaration
/// follows the file's emit format (`getEmitSyntaxForUsageLocationWorker`).
pub(crate) fn usage_resolution_mode(
    file: &Path,
    usage: ImportUsage,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
) -> ResolutionMode {
    match usage {
        ImportUsage::ModeOverride(surge_ts_syntax::ResolutionModeOverride::Import) => {
            return ResolutionMode::Esm;
        }
        ImportUsage::ModeOverride(surge_ts_syntax::ResolutionModeOverride::Require) => {
            return ResolutionMode::CommonJs;
        }
        _ => {}
    }
    if !import_syntax_affects_module_resolution(opts) {
        return ResolutionMode::None;
    }
    if matches!(usage, ImportUsage::ImportEquals | ImportUsage::Require) {
        return ResolutionMode::CommonJs;
    }
    let meta = source_file_metadata(file, opts, cache);
    let file_emit_format =
        match implied_node_format_for_emit(&file.to_string_lossy(), opts.emit_module, &meta) {
            ResolutionMode::CommonJs => ModuleKind::CommonJS,
            ResolutionMode::Esm => ModuleKind::ESNext,
            ResolutionMode::None => opts.emit_module,
        };
    // `ShouldTransformImportCall`: an `import()` stays ESM under the node
    // module kinds and `preserve`, and is a `require` where its file emits
    // CommonJS otherwise.
    if usage == ImportUsage::DynamicImport {
        let transformed = !matches!(
            opts.emit_module,
            ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 | ModuleKind::NodeNext | ModuleKind::Preserve
        ) && file_emit_format == ModuleKind::CommonJS;
        return if transformed { ResolutionMode::CommonJs } else { ResolutionMode::Esm };
    }
    match file_emit_format {
        ModuleKind::CommonJS => ResolutionMode::CommonJs,
        ModuleKind::ES2015
        | ModuleKind::ES2020
        | ModuleKind::ES2022
        | ModuleKind::ESNext
        | ModuleKind::Preserve => ResolutionMode::Esm,
        ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 | ModuleKind::NodeNext => {
            ResolutionMode::None
        }
    }
}

/// tsgo's `getDefaultResolutionModeForFile`, the mode a `/// <reference
/// types>` without `resolution-mode` resolves in.
pub(crate) fn default_resolution_mode(
    file: &Path,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
) -> ResolutionMode {
    if !import_syntax_affects_module_resolution(opts) {
        return ResolutionMode::None;
    }
    let meta = source_file_metadata(file, opts, cache);
    implied_node_format_for_emit(&file.to_string_lossy(), opts.emit_module, &meta)
}

/// Whether `GetConditions` starts with `import` for this mode: bundler reads
/// an unset mode as ESM.
pub(crate) fn resolves_with_import_condition(mode: ResolutionMode, opts: &ResolverOptions) -> bool {
    match mode {
        ResolutionMode::Esm => true,
        ResolutionMode::CommonJs => false,
        ResolutionMode::None => opts.module_resolution == ModuleResolutionKind::Bundler,
    }
}
