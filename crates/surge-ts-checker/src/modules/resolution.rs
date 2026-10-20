//! Relative module specifier resolution and filesystem-path candidate logic.

use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::paths::{canonicalize_if_exists_string, normalize_path_string};
use crate::program::ParsedProgramFile;
use crate::symbols::TypeDeclarationScope;

pub(crate) fn is_relative_specifier(specifier: &str) -> bool {
    specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with(".\\")
        || specifier.starts_with("..\\")
}

pub(crate) fn is_external_specifier(specifier: &str) -> bool {
    !is_relative_specifier(specifier)
}

thread_local! {
    // Per-run memoization of relative-module resolution. Within a single check
    // the file set (`program_files` / `file_index_by_identity`) is fixed, so a
    // given (importer, specifier) always resolves the same way. The multi-pass
    // import/export binding fixpoint resolves the same specifiers 3-5 times and
    // each miss rebuilds candidate path strings and probes them with
    // `canonicalize` (realpath syscalls), so caching removes both the recompute
    // and the syscalls on passes after the first. Resolved indices are
    // run-specific, so the cache is cleared at the start of each check.
    static RELATIVE_MODULE_CACHE: RefCell<HashMap<(String, String, bool), Option<ModuleResolution>>> =
        RefCell::new(HashMap::new());
}

/// Under node16/nodenext resolution, the files whose imports resolve in ESM
/// mode. Set for the length of one program check; every checking thread
/// resolves through it, which is why it is not thread-local.
static NODE_ESM_FILES: std::sync::RwLock<Option<Arc<std::collections::HashSet<String>>>> =
    std::sync::RwLock::new(None);

pub(crate) fn set_node_esm_files(files: Option<Arc<std::collections::HashSet<String>>>) {
    if let Ok(mut policy) = NODE_ESM_FILES.write() {
        *policy = files;
    }
}

/// `allowArbitraryExtensions` of the program being checked, read by the
/// relative resolver on every checking thread.
static ALLOW_ARBITRARY_EXTENSIONS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_allow_arbitrary_extensions(allowed: bool) {
    ALLOW_ARBITRARY_EXTENSIONS.store(allowed, std::sync::atomic::Ordering::Relaxed);
}

/// tsc's `GetResolutionDiagnostic` for a `.d.{extension}.ts` resolution: it
/// needs `allowArbitraryExtensions` unless the importer is a declaration file.
pub(crate) fn arbitrary_extension_resolution_allowed(importer_file_name: &str) -> bool {
    ALLOW_ARBITRARY_EXTENSIONS.load(std::sync::atomic::Ordering::Relaxed)
        || is_declaration_file_name(importer_file_name)
}

/// tsc's `tryAddingExtensions` for an extension it does not know: `./x.html`
/// names the declaration file `./x.d.html.ts`. `./x.json` tries `./x.d.json.ts`
/// ahead of the JSON file itself.
pub(crate) fn arbitrary_extension_declaration(joined: &str, specifier: &str) -> Option<String> {
    if !matches!(
        super::candidates::classify_relative_specifier(specifier),
        super::candidates::RelativeSpecifierShape::Extensionless
            | super::candidates::RelativeSpecifierShape::ExplicitJson
    ) {
        return None;
    }
    let (directory, base) = match joined.rsplit_once('/') {
        Some((directory, base)) => (Some(directory), base),
        None => (None, joined),
    };
    let (stem, extension) = base.rsplit_once('.')?;
    if extension.is_empty() || is_declaration_file_name(base) {
        return None;
    }
    let file = format!("{stem}.d.{extension}.ts");
    Some(match directory {
        Some(directory) => format!("{directory}/{file}"),
        None => file,
    })
}

/// Whether node16/nodenext resolves this file's imports in ESM mode, where a
/// relative specifier gets no extension appended and no directory lookup.
pub(crate) fn resolves_in_node_esm_mode(importer_file_name: &str) -> bool {
    NODE_ESM_FILES
        .read()
        .ok()
        .and_then(|policy| policy.as_ref().map(|files| files.contains(importer_file_name)))
        .unwrap_or(false)
}

/// Clears the per-thread relative-module resolution cache. Called at the start
/// of a program check so resolved indices from a prior run are never reused.
pub(crate) fn clear_relative_module_cache() {
    RELATIVE_MODULE_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// The loader's resolution of `specifier` from `importer_file`: in the mode
/// the usage picked (`import x = require()`, a `resolution-mode` attribute)
/// when the loader recorded one, else in the importer's own mode.
pub(crate) fn resolved_module_in_mode<'a>(
    ctx: &'a crate::context::CheckerContext,
    importer_file: &str,
    specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
) -> Option<&'a String> {
    if let Some(mode) = resolution_mode
        && let Some(resolved) = ctx
            .options
            .resolved_modules_by_importer
            .get(importer_file)
            .and_then(|per_importer| {
                per_importer.get(&super::candidates::resolution_mode_override_key(specifier, mode))
            })
    {
        return (!resolved.is_empty()).then_some(resolved);
    }
    ctx.options.resolved_module_for(importer_file, specifier)
}

/// The mode an import declaration's specifier resolves in when the usage,
/// not the file, decides it: `import x = require()` is CommonJS, and an
/// `import type`'s `resolution-mode` attribute names its own.
pub(crate) fn import_resolution_mode(
    import: &surge_ts_syntax::ParsedImportDeclaration,
) -> Option<surge_ts_syntax::ResolutionModeOverride> {
    if matches!(import.kind, surge_ts_syntax::ParsedImportKind::Equals { .. }) {
        return Some(surge_ts_syntax::ResolutionModeOverride::Require);
    }
    surge_ts_syntax::ParsedResolutionModeAttribute::resolution_override(import.resolution_mode)
}

/// Whether a relative specifier resolves in node16/nodenext ESM mode, which
/// appends no extension and looks in no directory: the mode a usage names
/// (`import x = require()` is CommonJS, a `resolution-mode` attribute names
/// its own), else the importer's.
pub(crate) fn relative_resolution_is_esm(
    importer_file_name: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
) -> bool {
    match resolution_mode {
        None => resolves_in_node_esm_mode(importer_file_name),
        Some(surge_ts_syntax::ResolutionModeOverride::Require) => false,
        Some(surge_ts_syntax::ResolutionModeOverride::Import) => NODE_ESM_FILES
            .read()
            .ok()
            .is_some_and(|policy| policy.is_some()),
    }
}

/// tsc's `isExtensionlessRelativePathImport`. Its `HasExtension` reads the
/// base name with trailing separators dropped, so `./` and `.` have one.
pub(crate) fn is_extensionless_relative_specifier(specifier: &str) -> bool {
    let trimmed = specifier.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    !base.contains('.')
}

pub(crate) fn resolve_relative_module(
    importer_file_name: &str,
    specifier: &str,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    resolve_relative_module_in_mode(
        importer_file_name,
        specifier,
        None,
        program_files,
        file_index_by_identity,
    )
}

/// [`resolve_relative_module`] in the mode a usage names (see
/// [`relative_resolution_is_esm`]).
pub(crate) fn resolve_relative_module_in_mode(
    importer_file_name: &str,
    specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    if !is_relative_specifier(specifier) {
        return None;
    }
    if program_files.is_empty() {
        return None;
    }
    let esm = relative_resolution_is_esm(importer_file_name, resolution_mode);
    if esm && is_extensionless_relative_specifier(specifier) {
        return None;
    }

    let cache_key = (importer_file_name.to_string(), specifier.to_string(), esm);
    if let Some(cached) =
        RELATIVE_MODULE_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned())
    {
        return cached;
    }

    let base = file_index_by_identity
        .get(importer_file_name)
        .and_then(|&index| program_files.get(index)?.relative_resolution_base.as_deref())
        .unwrap_or(importer_file_name);
    let resolved = if super::candidates::relative_specifier_names_directory(specifier) {
        if esm {
            None
        } else {
            resolve_relative_directory(base, specifier, program_files, file_index_by_identity)
        }
    } else {
        resolve_relative_module_uncached(base, specifier, program_files, file_index_by_identity).or_else(|| {
            if esm {
                return None;
            }
            resolve_relative_path_with_implicit_extensions(base, specifier, program_files, file_index_by_identity)
        })
    };
    RELATIVE_MODULE_CACHE.with(|cache| {
        cache.borrow_mut().insert(cache_key, resolved.clone());
    });
    resolved
}

pub(crate) fn resolve_relative_module_uncached(
    importer_file_name: &str,
    specifier: &str,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    let importer_dir = module_directory(importer_file_name);
    let normalized_specifier = normalize_path_string(specifier);
    let joined_specifier = if importer_dir.is_empty() {
        normalized_specifier.clone()
    } else {
        normalize_path_string(&format!("{importer_dir}/{normalized_specifier}"))
    };

    if let Some(declaration) = arbitrary_extension_declaration(&joined_specifier, &normalized_specifier)
        && let Some(resolved_file_index) =
            file_index_by_identity.get(canonical_file_identity(&declaration).as_str())
    {
        if !arbitrary_extension_resolution_allowed(importer_file_name) {
            return None;
        }
        return Some(ModuleResolution {
            resolved_file_index: *resolved_file_index,
            resolved_file_name: program_files.get(*resolved_file_index)?.file_name.clone(),
        });
    }

    let candidate_paths =
        super::candidates::relative_import_candidates(&joined_specifier, &normalized_specifier)?;

    for candidate in candidate_paths {
        let candidate = canonical_file_identity(&candidate);
        if let Some(resolved_file_index) = file_index_by_identity.get(candidate.as_str()) {
            return Some(ModuleResolution {
                resolved_file_index: *resolved_file_index,
                resolved_file_name: program_files.get(*resolved_file_index)?.file_name.clone(),
            });
        }
    }

    None
}

/// A specifier naming a directory (`.`, `..`, `./dir/`) resolves to the
/// directory's index only.
fn resolve_relative_directory(
    importer_file_name: &str,
    specifier: &str,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    let directory = relative_specifier_path(importer_file_name, specifier);
    first_program_file(
        super::candidates::directory_index_candidates(&directory),
        program_files,
        file_index_by_identity,
    )
}

/// tsc's CommonJS-mode lookups once a specifier with an extension misses as
/// that file (`loadModuleFromFile`, `nodeLoadModuleByRelativeName`): the whole
/// path takes the implicit extensions, then loads as a directory, so
/// `./foo.ts` can be `./foo.ts/index.ts`.
fn resolve_relative_path_with_implicit_extensions(
    importer_file_name: &str,
    specifier: &str,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    use super::candidates::RelativeSpecifierShape;
    if !matches!(
        super::candidates::classify_relative_specifier(&normalize_path_string(specifier)),
        RelativeSpecifierShape::ExplicitTs
            | RelativeSpecifierShape::ExplicitJs
            | RelativeSpecifierShape::ExplicitMjs
            | RelativeSpecifierShape::ExplicitCjs
            | RelativeSpecifierShape::ExplicitJson
    ) {
        return None;
    }
    let path = relative_specifier_path(importer_file_name, specifier);
    let mut candidates = vec![
        format!("{path}.ts"),
        format!("{path}.tsx"),
        format!("{path}.d.ts"),
    ];
    candidates.extend(super::candidates::directory_index_candidates(&path));
    first_program_file(candidates, program_files, file_index_by_identity)
}

pub(crate) fn relative_specifier_path(importer_file_name: &str, specifier: &str) -> String {
    let importer_dir = module_directory(importer_file_name);
    if importer_dir.is_empty() {
        normalize_path_string(specifier)
    } else {
        normalize_path_string(&format!("{importer_dir}/{specifier}"))
    }
}

fn first_program_file(
    candidates: Vec<String>,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> Option<ModuleResolution> {
    candidates.into_iter().find_map(|candidate| {
        let resolved_file_index =
            *file_index_by_identity.get(canonical_file_identity(&candidate).as_str())?;
        Some(ModuleResolution {
            resolved_file_index,
            resolved_file_name: program_files.get(resolved_file_index)?.file_name.clone(),
        })
    })
}

#[allow(dead_code)]
pub(crate) fn resolve_relative_local_type_scope(
    importer_file_name: &str,
    module_specifier: &str,
    program_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
) -> Option<(usize, Arc<TypeDeclarationScope>)> {
    let resolved = resolve_relative_module(
        importer_file_name,
        module_specifier,
        program_files,
        file_index_by_identity,
    )?;
    let local_scope = module_resolution_scopes
        .get(resolved.resolved_file_index)
        .and_then(|scope| scope.clone())?;
    Some((resolved.resolved_file_index, local_scope))
}

pub(crate) fn module_directory(file_name: &str) -> String {
    let normalized = normalize_path_string(file_name);
    normalized
        .rsplit_once('/')
        .map(|(directory, _)| directory.to_string())
        .unwrap_or_default()
}

pub(crate) fn canonical_file_identity(file_name: &str) -> String {
    crate::program::record_canonical_file_id_lookup();
    canonicalize_if_exists_string(Path::new(file_name))
}

pub(crate) fn is_declaration_file_name(file_name: &str) -> bool {
    surge_ts_syntax::is_declaration_file_name(file_name)
}
