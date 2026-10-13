use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use surge_ts_checker::SourceFileInput;
use surge_ts_config::canonicalize_if_exists_string;
use surge_ts_syntax::{
    ReferenceTypeDirective, TextSpan, extract_reference_path_directives,
    extract_reference_type_directives,
};

use crate::package_resolution::{
    ResolverOptions, select_export_target, select_export_targets, select_import_targets,
    types_versions_candidates,
};
use crate::specifier::{is_external_specifier, is_relative_specifier};

mod helpers;
mod mode;
use helpers::*;
pub(crate) use helpers::supported_extensions;
pub(crate) use mode::ImportUsage;
pub(crate) use mode::{ResolutionMode as UsageResolutionMode, usage_resolution_mode};
use mode::{ResolutionMode, default_resolution_mode, resolves_with_import_condition};

pub struct PackageDeclarationRequest {
    pub specifier: String,
    pub package_name: String,
    pub subpath: Option<String>,
    pub importer_dir: PathBuf,
    pub importer_file: PathBuf,
    /// `#alias` specifier resolved through the importer's own `imports` field.
    pub is_imports: bool,
    pub usage: ImportUsage,
    /// The usage's resolution mode as `GetConditions` reads it: `import`
    /// when set, `require` otherwise.
    pub import_condition: bool,
}

/// One loader resolution of a bare or `#imports` specifier.
pub(crate) struct PackageResolution {
    pub importer: String,
    pub specifier: String,
    /// Empty when nothing resolved, so the importer never borrows another
    /// importer's answer for the same specifier.
    pub resolved_file: String,
    pub usage: ImportUsage,
}

#[derive(Debug, Default)]
pub(crate) struct PackageDeclarationResolverCache {
    // Fx-hashed: these sit on the per-(importer, specifier) resolution path,
    // and SipHash over `PathBuf`/multi-`String` keys showed up in profiles.
    // Neither map is iterated, so the hasher cannot affect output order.
    package_json_cache:
        surge_ts_types::fx::FxHashMap<PathBuf, Option<std::sync::Arc<serde_json::Value>>>,
    entrypoint_cache: surge_ts_types::fx::FxHashMap<
        PackageEntrypointCacheKey,
        Option<PackageEntrypointResolution>,
    >,
    /// Count of `sources` entries whose specifiers this resolver has already
    /// queued, so loader fixpoint iterations scan each source exactly once.
    scanned_sources: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PackageEntrypointCacheKey {
    importer_dir: String,
    package_name: String,
    subpath: Option<String>,
    is_imports: bool,
    import_condition: bool,
}

#[derive(Debug, Clone)]
struct PackageEntrypointResolution {
    path: PathBuf,
    kind: PackageEntrypointKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageEntrypointKind {
    Declaration,
    RuntimeOnly,
}

fn parse_package_specifier(specifier: &str) -> Option<(String, Option<String>)> {
    if specifier.starts_with('@') {
        let parts: Vec<&str> = specifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            let pkg_name = format!("{}/{}", parts[0], parts[1]);
            let subpath = if parts.len() == 3 {
                Some(parts[2].to_string())
            } else {
                None
            };
            Some((pkg_name, subpath))
        } else {
            None
        }
    } else {
        let mut parts = specifier.splitn(2, '/');
        if let Some(pkg_name) = parts.next() {
            let subpath = parts.next().map(|s| s.to_string());
            Some((pkg_name.to_string(), subpath))
        } else {
            None
        }
    }
}

/// Package-declaration resolutions in BFS resolution order.
///
/// The same bare specifier may resolve differently from different importers
/// (nested `node_modules`, `#imports` scopes, self-name imports, resolution
/// modes), so results are keyed by importer rather than flattened to
/// `specifier → file`. The ordered `Vec` keeps the project-wide
/// first-resolution fallback map deterministic when the caller merges.
pub(crate) type PackageResolutions = Vec<PackageResolution>;

#[allow(dead_code)]
pub fn resolve_package_declaration_entrypoints(
    inputs: &mut Vec<SourceFileInput>,
    sources: &mut Vec<(PathBuf, String, String)>,
    root_dir: &Path,
) -> PackageResolutions {
    let mut cache = PackageDeclarationResolverCache::default();
    let mut scanner = crate::specifier_scan::ModuleSpecifierScanner::new(Default::default());
    resolve_package_declaration_entrypoints_with_cache(
        inputs,
        sources,
        root_dir,
        &ResolverOptions::default(),
        &mut cache,
        &mut scanner,
    )
}

pub(crate) fn resolve_package_declaration_entrypoints_with_cache(
    inputs: &mut Vec<SourceFileInput>,
    sources: &mut Vec<(PathBuf, String, String)>,
    root_dir: &Path,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
    scanner: &mut crate::specifier_scan::ModuleSpecifierScanner,
) -> PackageResolutions {
    let mut packages_to_resolve: VecDeque<PackageDeclarationRequest> = VecDeque::new();
    let mut resolutions: PackageResolutions = Vec::new();
    let mut resolved_packages: HashSet<(String, String, ImportUsage)> = HashSet::new();
    let mut known_file_names: HashSet<String> = inputs
        .iter()
        .map(|input| canonicalize_if_exists_string(Path::new(&input.file_name)))
        .collect();
    let mut queued_specifiers: HashSet<(String, String, ImportUsage)> = HashSet::new();

    scanner.prefetch(sources, cache.scanned_sources);
    for index in cache.scanned_sources..sources.len() {
        let (file_path, file_name, source_text) = {
            let (file_path, file_name, source_text) = &sources[index];
            (file_path.clone(), file_name.clone(), source_text.clone())
        };
        let importer_dir = file_path.parent().unwrap_or(root_dir).to_path_buf();
        let usages = scanner.usages(index, &file_name, &source_text);
        extract_packages_from_source(
            &usages,
            &file_path.to_string_lossy(),
            &importer_dir,
            opts,
            cache,
            &mut packages_to_resolve,
            &mut queued_specifiers,
        );
    }

    // The queue is finite: every (importer file, specifier, usage) triple is
    // enqueued at most once, and newly loaded files enqueue only their own.
    while let Some(req) = packages_to_resolve.pop_front() {
        let importer_key = canonicalize_if_exists_string(&req.importer_file);
        let resolved_key = (importer_key.clone(), req.specifier.clone(), req.usage);
        if resolved_packages.contains(&resolved_key) {
            continue;
        }

        let cache_key = PackageEntrypointCacheKey {
            importer_dir: canonicalize_if_exists_string(&req.importer_dir),
            package_name: req.package_name.clone(),
            subpath: req.subpath.clone(),
            is_imports: req.is_imports,
            import_condition: req.import_condition,
        };

        let resolution = if let Some(cached) = cache.entrypoint_cache.get(&cache_key) {
            cached.clone()
        } else {
            let resolved =
                resolve_package_entrypoint(&req, opts, req.import_condition, cache, root_dir);
            cache.entrypoint_cache.insert(cache_key, resolved.clone());
            resolved
        };

        let Some(resolution) = resolution else {
            resolved_packages.insert(resolved_key);
            resolutions.push(PackageResolution {
                importer: importer_key,
                specifier: req.specifier.clone(),
                resolved_file: String::new(),
                usage: req.usage,
            });
            continue;
        };

        match resolution.kind {
            PackageEntrypointKind::Declaration => {
                let Ok(path) = crate::host_fs::canonicalize(&resolution.path) else {
                    continue;
                };

                let normalized_file_name = canonicalize_if_exists_string(&path);
                resolved_packages.insert(resolved_key);
                resolutions.push(PackageResolution {
                    importer: importer_key.clone(),
                    specifier: req.specifier.clone(),
                    resolved_file: normalized_file_name.clone(),
                    usage: req.usage,
                });

                if !known_file_names.contains(&normalized_file_name) {
                    let read_start = std::time::Instant::now();
                    let Ok(source_text) = crate::host_fs::read_to_string(&path) else {
                        continue;
                    };
                    crate::io_stats::record_package_declaration_read(
                        source_text.len(),
                        read_start.elapsed(),
                    );

                    known_file_names.insert(normalized_file_name.clone());
                    inputs.push(SourceFileInput {
                        file_name: normalized_file_name.clone(),
                        source_text: source_text.clone(),
                    });
                    sources.push((
                        path.clone(),
                        normalized_file_name.clone(),
                        source_text.clone(),
                    ));

                    let new_index = sources.len() - 1;
                    let usages = scanner.usages(new_index, &normalized_file_name, &source_text);
                    let new_importer_dir = path.parent().unwrap_or(root_dir).to_path_buf();
                    extract_packages_from_source(
                        &usages,
                        &normalized_file_name,
                        &new_importer_dir,
                        opts,
                        cache,
                        &mut packages_to_resolve,
                        &mut queued_specifiers,
                    );
                }
            }
            PackageEntrypointKind::RuntimeOnly => {
                if let Ok(path) = crate::host_fs::canonicalize(&resolution.path) {
                    let file_name = canonicalize_if_exists_string(&path);
                    let is_json = file_name.to_ascii_lowercase().ends_with(".json");
                    resolved_packages.insert(resolved_key);
                    resolutions.push(PackageResolution {
                        importer: importer_key,
                        specifier: req.specifier.clone(),
                        resolved_file: if !is_json || opts.resolve_json_module {
                            file_name.clone()
                        } else {
                            String::new()
                        },
                        usage: req.usage,
                    });
                    // A `.json` module joins the program like the relative
                    // ones the import graph loads.
                    if is_json
                        && opts.resolve_json_module
                        && known_file_names.insert(file_name.clone())
                        && let Ok(source_text) = crate::host_fs::read_to_string(&path)
                    {
                        inputs.push(SourceFileInput {
                            file_name: file_name.clone(),
                            source_text: source_text.clone(),
                        });
                        sources.push((path, file_name, source_text));
                    }
                }
            }
        }
    }

    // Every source present at exit has been queued (pre-existing ones by the
    // entry loop, self-loaded declarations inline above); later calls resume
    // from here.
    cache.scanned_sources = sources.len();

    resolutions
}

fn cached_package_entrypoint(
    req: &PackageDeclarationRequest,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
    root_dir: &Path,
) -> Option<PackageEntrypointResolution> {
    let cache_key = PackageEntrypointCacheKey {
        importer_dir: canonicalize_if_exists_string(&req.importer_dir),
        package_name: req.package_name.clone(),
        subpath: req.subpath.clone(),
        is_imports: req.is_imports,
        import_condition: req.import_condition,
    };
    if let Some(cached) = cache.entrypoint_cache.get(&cache_key) {
        return cached.clone();
    }
    let resolved = resolve_package_entrypoint(req, opts, req.import_condition, cache, root_dir);
    cache.entrypoint_cache.insert(cache_key, resolved.clone());
    resolved
}

/// Resolves one bare or `#imports` specifier from `importer_file` as the
/// loader does, outside a program build: the declaration file it names and
/// whether it is one (a runtime-only JavaScript entrypoint is not).
pub(crate) fn resolve_package_specifier(
    specifier: &str,
    importer_file: &Path,
    opts: &ResolverOptions,
    root_dir: &Path,
) -> Option<(PathBuf, bool)> {
    let mut cache = PackageDeclarationResolverCache::default();
    let mut queue = VecDeque::new();
    let mut queued = HashSet::new();
    let importer_dir = importer_file.parent().unwrap_or(root_dir);
    helpers::queue_specifier(
        specifier,
        ImportUsage::Declaration,
        importer_dir,
        importer_file,
        opts,
        &mut cache,
        &mut queue,
        &mut queued,
    );
    let request = queue.pop_front()?;
    let resolution = resolve_package_entrypoint(&request, opts, request.import_condition, &mut cache, root_dir)?;
    let path = crate::host_fs::canonicalize(&resolution.path).unwrap_or(resolution.path);
    Some((path, resolution.kind == PackageEntrypointKind::Declaration))
}

/// Resolves each source module file's bare `declare module "m"` augmentation
/// names the way an import of `m` from that file resolves, without loading
/// anything: tsc resolves augmentation names alongside imports but adds only
/// the imports' targets to the program, so an augmentation whose target
/// nothing imports is TS2664.
pub(crate) fn resolve_module_augmentation_specifiers(
    sources: &[(PathBuf, String, String)],
    root_dir: &Path,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
    scanner: &crate::specifier_scan::ModuleSpecifierScanner,
) -> PackageResolutions {
    let mut requests: VecDeque<PackageDeclarationRequest> = VecDeque::new();
    let mut queued_specifiers: HashSet<(String, String, ImportUsage)> = HashSet::new();
    for (index, specifiers) in scanner.augmentation_specifiers() {
        let Some((file_path, file_name, _)) = sources.get(index) else {
            continue;
        };
        if is_declaration_file_path_str(file_name) {
            continue;
        }
        let importer_dir = file_path.parent().unwrap_or(root_dir).to_path_buf();
        let usages: Vec<crate::specifier_scan::ModuleUsage> = specifiers
            .iter()
            .map(|specifier| crate::specifier_scan::ModuleUsage {
                specifier: specifier.clone(),
                import_equals: false,
                resolution_mode: None,
                call: None,
            })
            .collect();
        extract_packages_from_source(
            &usages,
            &file_path.to_string_lossy(),
            &importer_dir,
            opts,
            cache,
            &mut requests,
            &mut queued_specifiers,
        );
    }

    let mut resolutions: PackageResolutions = Vec::new();
    for req in requests {
        let Some(resolution) = cached_package_entrypoint(&req, opts, cache, root_dir) else {
            continue;
        };
        let Ok(path) = crate::host_fs::canonicalize(&resolution.path) else {
            continue;
        };
        resolutions.push(PackageResolution {
            importer: canonicalize_if_exists_string(&req.importer_file),
            specifier: req.specifier,
            resolved_file: canonicalize_if_exists_string(&path),
            usage: req.usage,
        });
    }
    resolutions
}

/// tsc's `pathForLibFile` under `libReplacement`: `lib.dom.iterable.d.ts` is
/// whatever `@typescript/lib-dom/iterable` resolves to from the config
/// directory in CommonJS mode (`getLibraryNameFromLibFileName`,
/// `resolveLibrary`), and stays the bundled lib when nothing does.
pub(crate) fn resolve_lib_replacement(
    normalized_name: &str,
    config_dir: &Path,
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
) -> Option<PathBuf> {
    let library_name = lib_replacement_library_name(normalized_name);
    let (package_name, subpath) = parse_package_specifier(&library_name)?;
    let import_condition = resolves_with_import_condition(ResolutionMode::CommonJs, opts);
    let req = PackageDeclarationRequest {
        specifier: library_name,
        package_name,
        subpath,
        importer_dir: config_dir.to_path_buf(),
        importer_file: config_dir
            .join(format!("__lib_node_modules_lookup_lib.{normalized_name}.d.ts__.ts")),
        is_imports: false,
        usage: ImportUsage::Declaration,
        import_condition,
    };
    let resolution = resolve_package_entrypoint(&req, opts, import_condition, cache, config_dir)?;
    (resolution.kind == PackageEntrypointKind::Declaration)
        .then(|| crate::host_fs::canonicalize(&resolution.path).unwrap_or(resolution.path))
}

/// The package a lib's replacement comes from: `@typescript/lib-dom` for
/// `dom.iterable`.
pub(crate) fn lib_replacement_package_name(normalized_name: &str) -> Option<String> {
    parse_package_specifier(&lib_replacement_library_name(normalized_name))
        .map(|(package_name, _)| package_name)
}

fn lib_replacement_library_name(normalized_name: &str) -> String {
    let mut components = normalized_name.split('.');
    let mut library_name = format!("@typescript/lib-{}", components.next().unwrap_or_default());
    for (index, component) in components.enumerate() {
        library_name.push(if index == 0 { '/' } else { '-' });
        library_name.push_str(component);
    }
    library_name
}

/// Outcome of resolving the project's configured type packages.
pub(crate) struct TypePackageResolution {
    /// Type-package names actually included, in load order. Passed to the checker
    /// as `CheckerOptions.types` so node-specific builtins and the `@types`
    /// ambient-global gate fire for them.
    pub effective_type_names: Vec<String>,
    /// Explicitly-listed `types` entries that could not be resolved. The caller
    /// emits a TS2688 for each; wildcard-discovered packages never appear here.
    pub missing: Vec<String>,
}

/// Resolve the project's type packages and load them as dependency declarations.
///
/// Mirrors TypeScript 6.0's `getAutomaticTypeDirectiveNames`:
///
/// * `types` absent (`None`) or `types: []` → include nothing. TypeScript 6.0
///   removed the implicit inclusion of every visible `@types` package; automatic
///   discovery is now opt-in via the `"*"` wildcard below.
/// * `types` containing `"*"` → expand the wildcard to every package found under
///   the effective type roots (`typeRoots` if set, otherwise the ancestor
///   `node_modules/@types` chain), skipping dot-prefixed and "not needed"
///   packages. Other literal entries are preserved; the list is deduped in order.
/// * each resulting name resolves nearest-root-first; scoped names are mangled
///   (`@scope/pkg` -> `scope__pkg`) only under `@types` roots, matching
///   `getCandidateFromTypeRoot`.
///
/// Entrypoint resolution stays narrow: `types` / `typings` / exact
/// `exports["."].types` / `index.d.ts`. A name no type root provides then falls
/// through to the secondary `node_modules` lookup, which is what resolves
/// subpath directives such as `vitest/globals`.
pub(crate) fn resolve_type_packages(
    inputs: &mut Vec<SourceFileInput>,
    sources: &mut Vec<(PathBuf, String, String)>,
    root_dir: &Path,
    types: Option<&[String]>,
    type_roots: &[PathBuf],
    opts: &ResolverOptions,
    cache: &mut PackageDeclarationResolverCache,
) -> TypePackageResolution {
    let mut resolution = TypePackageResolution {
        effective_type_names: Vec::new(),
        missing: Vec::new(),
    };

    // `types` absent or empty: nothing is auto-included (TypeScript 6.0).
    let Some(configured) = types else {
        return resolution;
    };
    if configured.is_empty() {
        return resolution;
    }

    let roots = effective_type_roots(root_dir, type_roots);

    // Expand `"*"` entries to the packages discovered under the type roots.
    let wildcard_matches = if configured.iter().any(|entry| entry == "*") {
        discover_wildcard_type_names(&roots, cache)
    } else {
        Vec::new()
    };

    // Flatten the directive list in order, deduping by name. `explicit` entries
    // emit TS2688 when unresolved; wildcard matches never do (they came from
    // directories that exist).
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut directives: Vec<(String, bool)> = Vec::new();
    for entry in configured {
        if entry == "*" {
            for name in &wildcard_matches {
                if seen_names.insert(name.clone()) {
                    directives.push((name.clone(), false));
                }
            }
        } else if seen_names.insert(entry.clone()) {
            directives.push((entry.clone(), true));
        }
    }

    let mut known_file_names: HashSet<String> = inputs
        .iter()
        .map(|input| canonicalize_if_exists_string(Path::new(&input.file_name)))
        .collect();

    // tsc resolves `types` entries with no resolution mode.
    let import_condition = resolves_with_import_condition(ResolutionMode::None, opts);
    for (name, explicit) in directives {
        let resolved = resolve_type_directive_in_roots(&name, &roots, cache).or_else(|| {
            resolve_type_directive_in_node_modules(
                &name,
                root_dir,
                root_dir,
                import_condition,
                opts,
                cache,
            )
        });
        match resolved {
            Some(path) => {
                load_type_package_file(&path, inputs, sources, &mut known_file_names);
                resolution.effective_type_names.push(name);
            }
            None => {
                if explicit {
                    resolution.missing.push(name);
                }
            }
        }
    }

    resolution
}

/// A `/// <reference types="..." />` site that could not be resolved to a type
/// package. The caller emits a TS2688 located at `value_span` in `file_name`.
pub(crate) struct MissingReferenceTypeDirective {
    pub file_name: String,
    pub type_name: String,
    pub value_span: TextSpan,
    /// Whether the referencing file is a declaration (`.d.ts`) file. tsc gates the
    /// TS2688 from such a site behind `skipLibCheck`, like any other `.d.ts`
    /// diagnostic.
    pub from_declaration_file: bool,
}

/// A `/// <reference path="..." />` site that names no file the program can
/// load, reported at `value_span` in `file_name` with that file's semantic
/// diagnostics.
pub(crate) struct UnresolvedReferencePath {
    pub file_name: String,
    /// The directive's value as written.
    pub path: String,
    pub value_span: TextSpan,
    /// Whether the referencing file is a declaration file, whose diagnostics
    /// `skipLibCheck` skips (tsgo's `SkipTypeChecking`).
    pub from_declaration_file: bool,
    pub failure: ReferencePathFailure,
}

/// Why a `/// <reference path>` names no file (tsgo's
/// `getSourceFileFromReference`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferencePathFailure {
    /// TS6053: the named file does not exist.
    NotFound,
    /// TS6231: no extension of the first group completes an extensionless
    /// path.
    NoExtensionMatched,
    /// TS6504: a JavaScript file without `allowJs`.
    JavaScriptFile,
    /// TS6054.
    UnsupportedExtension,
    /// TS1006.
    SelfReference,
}

/// Outcome of resolving every `/// <reference types>` directive reachable from the
/// program.
pub(crate) struct ReferenceTypeDirectiveResolution {
    pub effective_type_names: Vec<String>,
    pub missing: Vec<MissingReferenceTypeDirective>,
    pub unresolved_paths: Vec<UnresolvedReferencePath>,
    /// `allowJs`, which widens the extensions an unresolved path's message
    /// lists.
    pub allow_js: bool,
}

/// Resolves explicit `/// <reference types="..." />` directives against the same
/// type roots and entrypoint logic as `compilerOptions.types`. The resolver is
/// stateful so it can be re-run as the file set grows during import-graph and
/// package-declaration expansion: each call scans only files not seen before and
/// follows references recursively (a loaded type package's own directives).
pub(crate) struct ReferenceTypeDirectiveResolver {
    roots: Vec<PathBuf>,
    root_dir: PathBuf,
    scanned_files: HashSet<String>,
    // The secondary `node_modules` lookup walks up from the referencing file
    // in the directive's mode, so the same directive name can resolve
    // differently per directory and per mode.
    resolution_cache: HashMap<(PathBuf, String, bool), Option<PathBuf>>,
    effective_type_names: Vec<String>,
    seen_effective: HashSet<String>,
    missing: Vec<MissingReferenceTypeDirective>,
    allow_js: bool,
    /// `noResolve`: tsgo's file loader skips every file's reference
    /// directives (`filesparser.go`), `path` and `types` alike.
    no_resolve: bool,
    unresolved_paths: Vec<UnresolvedReferencePath>,
}

impl ReferenceTypeDirectiveResolver {
    pub fn new(root_dir: &Path, type_roots: &[PathBuf], allow_js: bool, no_resolve: bool) -> Self {
        Self {
            roots: effective_type_roots(root_dir, type_roots),
            root_dir: root_dir.to_path_buf(),
            scanned_files: HashSet::new(),
            resolution_cache: HashMap::new(),
            effective_type_names: Vec::new(),
            seen_effective: HashSet::new(),
            missing: Vec::new(),
            allow_js,
            no_resolve,
            unresolved_paths: Vec::new(),
        }
    }

    /// Scan every not-yet-scanned source for reference-type directives, loading
    /// each resolved type package into `inputs`/`sources`. Loading appends files,
    /// which are scanned in the same call, so recursive references converge here.
    pub fn scan_and_resolve(
        &mut self,
        inputs: &mut Vec<SourceFileInput>,
        sources: &mut Vec<(PathBuf, String, String)>,
        opts: &ResolverOptions,
        cache: &mut PackageDeclarationResolverCache,
    ) {
        // Built once, then maintained incrementally: `load_type_package_file` is
        // the only place `inputs` grows here, and it inserts the very string it
        // pushes as the input's `file_name`, so a per-round rebuild only
        // re-derived what the set already held — at one canonicalize-cache probe
        // and one `String` per input, per round.
        let mut known_file_names: HashSet<String> = inputs
            .iter()
            .map(|input| canonicalize_if_exists_string(Path::new(&input.file_name)))
            .collect();

        loop {
            let pending: Vec<(String, String)> = sources
                .iter()
                .filter(|(_, file_name, _)| !self.scanned_files.contains(file_name))
                .map(|(_, file_name, source_text)| (file_name.clone(), source_text.clone()))
                .collect();

            if pending.is_empty() {
                break;
            }

            for (file_name, source_text) in pending {
                self.scanned_files.insert(file_name.clone());
                if self.no_resolve {
                    continue;
                }

                // `/// <reference path="..." />` pulls in a sibling declaration
                // file relative to the referencing file. This is how a type
                // package such as `@types/node` assembles its full surface
                // (`index.d.ts` references `globals.d.ts`, `buffer.d.ts`, ...),
                // where its ambient globals (`process`, `Buffer`) are declared.
                for directive in extract_reference_path_directives(&source_text) {
                    match resolve_reference_path(
                        &file_name,
                        &directive.value,
                        self.allow_js,
                        opts.resolve_json_module,
                    ) {
                        Ok(path) => load_type_package_file(&path, inputs, sources, &mut known_file_names),
                        Err(failure) => self.unresolved_paths.push(UnresolvedReferencePath {
                            file_name: file_name.clone(),
                            path: directive.value,
                            value_span: directive.value_span,
                            from_declaration_file: is_declaration_file_path_str(&file_name),
                            failure,
                        }),
                    }
                }

                let directives = extract_reference_type_directives(&source_text);
                if directives.is_empty() {
                    continue;
                }

                let from_declaration_file = is_declaration_file_path_str(&file_name);
                for directive in directives {
                    self.resolve_directive(
                        directive,
                        &file_name,
                        from_declaration_file,
                        inputs,
                        sources,
                        &mut known_file_names,
                        opts,
                        cache,
                    );
                }
            }
        }
    }

    fn resolve_directive(
        &mut self,
        directive: ReferenceTypeDirective,
        file_name: &str,
        from_declaration_file: bool,
        inputs: &mut Vec<SourceFileInput>,
        sources: &mut Vec<(PathBuf, String, String)>,
        known_file_names: &mut HashSet<String>,
        opts: &ResolverOptions,
        cache: &mut PackageDeclarationResolverCache,
    ) {
        let name = directive.value;
        let lookup_dir = Path::new(file_name)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root_dir.clone());
        // tsc's `getModeForTypeReferenceDirectiveInFile`.
        let mode = match directive.resolution_mode {
            Some(surge_ts_syntax::ResolutionModeOverride::Import) => ResolutionMode::Esm,
            Some(surge_ts_syntax::ResolutionModeOverride::Require) => ResolutionMode::CommonJs,
            None => default_resolution_mode(Path::new(file_name), opts, cache),
        };
        let import_condition = resolves_with_import_condition(mode, opts);
        let cache_key = (lookup_dir.clone(), name.clone(), import_condition);
        let resolved = match self.resolution_cache.get(&cache_key) {
            Some(cached) => cached.clone(),
            None => {
                let resolved = if is_relative_specifier(&name) || Path::new(&name).is_absolute() {
                    resolve_relative_type_directive(&name, &lookup_dir, cache)
                } else {
                    resolve_type_directive_in_roots(&name, &self.roots, cache).or_else(|| {
                        resolve_type_directive_in_node_modules(
                            &name,
                            &lookup_dir,
                            &self.root_dir,
                            import_condition,
                            opts,
                            cache,
                        )
                    })
                };
                self.resolution_cache.insert(cache_key, resolved.clone());
                resolved
            }
        };

        match resolved {
            Some(path) => {
                load_type_package_file(&path, inputs, sources, known_file_names);
                if self.seen_effective.insert(name.clone()) {
                    self.effective_type_names.push(name);
                }
            }
            None => {
                self.missing.push(MissingReferenceTypeDirective {
                    file_name: file_name.to_string(),
                    type_name: name,
                    value_span: directive.value_span,
                    from_declaration_file,
                });
            }
        }
    }

    pub fn into_resolution(self) -> ReferenceTypeDirectiveResolution {
        ReferenceTypeDirectiveResolution {
            effective_type_names: self.effective_type_names,
            missing: self.missing,
            unresolved_paths: self.unresolved_paths,
            allow_js: self.allow_js,
        }
    }
}
