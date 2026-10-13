//! Embeddable TypeScript noEmit compatibility checker.
//!
//! This is the umbrella crate for `surge-ts`. It re-exports the in-memory
//! checking API from [`surge_ts_checker`] and adds [`Project`] — full
//! `tsconfig.json` project checking (config loading, package/`paths`/reference
//! resolution, default-lib loading, and the import-graph fixpoint) behind a
//! single call:
//!
//! ```no_run
//! use surge_ts::{Project, ProjectOptions};
//!
//! let project = Project::load("tsconfig.json");
//! let result = project.check(&ProjectOptions::default());
//! for diagnostic in &result.diagnostics {
//!     println!("{}: {}", diagnostic.code, diagnostic.message);
//! }
//! ```
//!
//! For in-memory, single- or multi-file checking without a tsconfig, use the
//! re-exported [`Checker`] builder directly.

pub mod api;
mod host_fs;
mod import_graph;
mod io_stats;
mod module_discovery;
mod package_declarations;
mod package_dedup;
mod package_resolution;
mod path_mapping;
mod probe;
pub mod resolver;
mod semver;
mod specifier;
mod specifier_scan;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use surge_ts_checker::{
    CheckResult, Checker, CheckerOptions, CompatibilityStats, Diagnostic, DiagnosticCategory,
    DiagnosticCode, DiagnosticProfile, FileKind, ProgramCheckResult, SourceFileInput, TextSpan,
};
pub use surge_ts_config::{ConfigDiagnostic, LoadedTsConfig, ScriptTarget, TsConfigLoadOptions};

use surge_ts_checker::lowlevel::{DefaultLibRequest, load_default_lib_inputs};

pub use surge_ts_checker::lowlevel::LibSourceChoice;
use surge_ts_config::{canonicalize_if_exists_string, load_tsconfig};

/// A resolved project source file: `(path, canonical file name, source text)`.
/// Returned alongside diagnostics so callers can render code frames.
pub type ProjectSource = (PathBuf, String, String);

/// Options for [`Project::check`]. Strictness flags and module/lib options come
/// from the loaded `tsconfig.json`; these are the run-level knobs the config
/// does not carry.
#[derive(Debug, Clone)]
pub struct ProjectOptions {
    /// Worker threads for checking. `0` selects automatically; otherwise the
    /// literal count is used.
    pub jobs: usize,
    /// Suppress non-relative (package) missing-module diagnostics.
    pub stub_external_modules: bool,
    /// `Tsc` (default) or `Native` diagnostic output.
    pub diagnostic_profile: DiagnosticProfile,
    /// Where the standard library is loaded from. Defaults to the bundled,
    /// version-pinned snapshot compiled into the binary; an installed
    /// `typescript` package is used only when explicitly selected.
    pub lib_source: LibSourceChoice,
    /// Populate [`ProjectCheckResult::timings`]. Off by default to avoid the
    /// per-step `Instant` and global counter overhead.
    pub collect_timings: bool,
    /// Keep every source text in [`ProjectCheckResult::sources`]. By default
    /// texts are dropped before checking (they are only needed again to render
    /// code frames, and holding a second copy of every dependency `.d.ts`
    /// through the checker's peak costs tens of MB); files that end up with
    /// diagnostics are re-read from disk afterwards. Consumers that re-parse
    /// every source (`--compatReport`) must opt in.
    pub retain_all_sources: bool,
    /// The caller will `std::process::exit` right after consuming the result,
    /// so the checker may skip its end-of-run teardown. See
    /// [`surge_ts_checker::set_fast_process_exit`].
    pub fast_process_exit: bool,
}

impl Default for ProjectOptions {
    fn default() -> Self {
        Self {
            jobs: 0,
            stub_external_modules: false,
            diagnostic_profile: DiagnosticProfile::default(),
            lib_source: LibSourceChoice::default(),
            collect_timings: false,
            fast_process_exit: false,
            retain_all_sources: false,
        }
    }
}

/// Per-phase timing and I/O counters for a project check. Populated only when
/// [`ProjectOptions::collect_timings`] is set; all zero otherwise.
#[derive(Debug, Clone, Default)]
pub struct ProjectTimings {
    pub file_discovery: Duration,
    pub default_lib_loading: Duration,
    pub package_declaration_discovery: Duration,
    pub import_graph_expansion: Duration,
    pub path_mapping_resolution: Duration,
    pub checking: Duration,
    pub source_read_io: Duration,
    pub source_files_read: u64,
    pub source_bytes_read: u64,
    pub default_lib_files_read: u64,
    pub default_lib_bytes_read: u64,
    pub default_lib_read_io: Duration,
    pub default_lib_existence_probes: u64,
    pub default_lib_canonicalize_syscalls: u64,
    pub expansion_read_io: Duration,
    pub expansion_files_read: u64,
    pub expansion_bytes_read: u64,
    pub package_declaration_read_io: Duration,
    pub package_declaration_probes: u64,
    pub package_declaration_probe_io: Duration,
    pub package_json_reads: u64,
    pub fs_existence_probes: u64,
    pub fs_existence_probe_io: Duration,
    pub fs_read_dir_count: u64,
    pub fs_read_dir_io: Duration,
    pub canonicalize_memo_misses: u64,
    pub canonicalize_full_realpaths: u64,
    pub canonicalize_leaf_probes: u64,
    pub canonicalize_miss_io: Duration,
}

/// Outcome of [`Project::check`]: diagnostics, tsc-compatibility stats, the
/// resolved source set (for rendering), any non-fatal warnings, and optional
/// timings.
#[derive(Debug, Clone)]
pub struct ProjectCheckResult {
    pub diagnostics: Vec<Diagnostic>,
    pub stats: CompatibilityStats,
    pub sources: Vec<ProjectSource>,
    pub warnings: Vec<String>,
    pub timings: ProjectTimings,
}

/// A failure that prevents a project check from running.
#[derive(Debug)]
pub enum ProjectError {
    /// A project source file could not be read.
    SourceRead {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::SourceRead { path, error } => {
                write!(f, "failed to read {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ProjectError {}

/// A loaded `tsconfig.json` project, ready to check.
///
/// [`Project::load`] performs config discovery and normalization only;
/// [`Project::check`] runs source reading, module/type resolution, default-lib
/// loading, and the type check.
#[derive(Debug, Clone)]
pub struct Project {
    loaded: LoadedTsConfig,
}

impl Project {
    /// Load and normalize a `tsconfig.json`. Configuration-level diagnostics
    /// (parse errors, unknown options) are captured in [`Self::config_diagnostics`]
    /// rather than returned as an error.
    pub fn load(tsconfig_path: impl AsRef<Path>) -> Self {
        surge_ts_config::clear_canonicalize_cache();
        let loaded = load_tsconfig(TsConfigLoadOptions {
            project: tsconfig_path.as_ref().to_path_buf(),
        });
        Self { loaded }
    }

    /// The normalized config, including discovered files and compiler options.
    pub fn config(&self) -> &LoadedTsConfig {
        &self.loaded
    }

    /// Diagnostics produced while loading the config (parse/normalization).
    pub fn config_diagnostics(&self) -> &[ConfigDiagnostic] {
        &self.loaded.diagnostics
    }

    /// Whether the project discovered no input files.
    pub fn is_empty(&self) -> bool {
        self.loaded.files.is_empty()
    }

    /// Read sources, resolve modules/types/default-libs, and type-check the
    /// program. Returns [`ProjectError::SourceRead`] if a discovered file cannot
    /// be read.
    pub fn check(&self, options: &ProjectOptions) -> Result<ProjectCheckResult, ProjectError> {
        let (run, finish) = match self.prepare(options)? {
            Prepared::Empty(result) => return Ok(result),
            Prepared::Ready(run, finish) => (run, finish),
        };
        let checking_start = Instant::now();
        let result = surge_ts_checker::lowlevel::check_program_with_prescanned_sources(
            run.inputs,
            run.prescanned,
            run.checker_options,
            options.jobs,
        );
        Ok(finish.finish(result, checking_start.elapsed(), options))
    }

    /// Everything [`Self::check`] does before and after the type check, which
    /// the compiler API's program runs around a retained check instead.
    pub(crate) fn prepare(&self, options: &ProjectOptions) -> Result<Prepared<'_>, ProjectError> {
        let loaded = &self.loaded;
        if let Some(trace) = std::env::var_os("SURGE_RESOLVER_VALIDATE") {
            resolver::validate::validate(loaded, std::path::Path::new(&trace));
        }
        let mut timings = ProjectTimings::default();
        let collect = options.collect_timings;
        let mut warnings = Vec::new();

        // `io_stats` counters are process-global and accumulate across calls, so
        // take a baseline and report the delta for this check rather than the raw
        // running totals.
        let io_baseline = if collect {
            io_stats::snapshot()
        } else {
            io_stats::IoSnapshot::default()
        };
        let canonicalize_baseline = if collect {
            surge_ts_config::canonicalize_io_snapshot()
        } else {
            surge_ts_config::CanonicalizeIoSnapshot::default()
        };

        if loaded.files.is_empty() {
            return Ok(Prepared::Empty(ProjectCheckResult {
                diagnostics: Vec::new(),
                stats: CompatibilityStats::default(),
                sources: Vec::new(),
                warnings,
                timings,
            }));
        }

        probe::clear_probe_cache();

        let file_discovery_start = Instant::now();
        let read_workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(options.jobs)
            .min(loaded.files.len());
        let source_read_nanos = std::sync::atomic::AtomicU64::new(0);
        let source_entries = read_project_sources(&loaded.files, read_workers, &source_read_nanos)
            .map_err(|(path, error)| ProjectError::SourceRead { path, error })?;

        let mut inputs = Vec::with_capacity(source_entries.len());
        let mut sources = Vec::with_capacity(source_entries.len());
        for (file_path, file_name, source_text) in source_entries {
            inputs.push(SourceFileInput {
                file_name: file_name.clone(),
                source_text: source_text.clone(),
            });
            sources.push((file_path, file_name, source_text));
        }
        surge_ts_checker::lowlevel::record_loader_rss_stage("sources_read");
        if collect {
            timings.file_discovery += file_discovery_start.elapsed();
            timings.source_read_io +=
                Duration::from_nanos(source_read_nanos.load(std::sync::atomic::Ordering::Relaxed));
            timings.source_files_read += inputs.len() as u64;
            timings.source_bytes_read += inputs
                .iter()
                .map(|input| input.source_text.len() as u64)
                .sum::<u64>();
        }

        let mut resolved_modules = surge_ts_types::fx::FxHashMap::default();
        let mut resolved_modules_by_importer: surge_ts_types::fx::FxHashMap<
            String,
            surge_ts_types::fx::FxHashMap<String, String>,
        > = surge_ts_types::fx::FxHashMap::default();
        let mut package_resolution_cache =
            package_declarations::PackageDeclarationResolverCache::default();

        let resolver_options = resolver_options(loaded);

        let type_package_resolution = package_declarations::resolve_type_packages(
            &mut inputs,
            &mut sources,
            &loaded.root_dir,
            loaded.compiler_options.types.as_deref(),
            &loaded.compiler_options.type_roots,
            &resolver_options,
            &mut package_resolution_cache,
        );

        let mut reference_type_resolver = package_declarations::ReferenceTypeDirectiveResolver::new(
            &loaded.root_dir,
            &loaded.compiler_options.type_roots,
            loaded.compiler_options.allow_js,
            loaded.compiler_options.no_resolve,
        );

        let mut specifier_scanner =
            specifier_scan::ModuleSpecifierScanner::new(surge_ts_syntax::JsxRuntimeOptions {
                automatic: matches!(
                    loaded.compiler_options.jsx,
                    Some(surge_ts_config::JsxMode::ReactJsx | surge_ts_config::JsxMode::ReactJsxDev)
                ),
                development: loaded.compiler_options.jsx == Some(surge_ts_config::JsxMode::ReactJsxDev),
                import_source: loaded.compiler_options.jsx_import_source.clone(),
            })
            .with_import_helpers(specifier_scan::ImportHelpersScan {
                enabled: loaded.compiler_options.import_helpers,
                isolated_modules: loaded.compiler_options.isolated_modules
                    || loaded.compiler_options.verbatim_module_syntax,
                force_module_detection: loaded.compiler_options.module_detection
                    == surge_ts_config::ModuleDetectionKind::Force,
            });
        let mut import_graph_state = import_graph::ImportGraphState::default();
        let mut javascript_modules = Vec::new();
        // `SURGE_LEGACY_RESOLUTION=1` keeps the loader's own package and import
        // graph passes in place of the ported resolver, for comparison.
        let legacy_resolution = std::env::var_os("SURGE_LEGACY_RESOLUTION").is_some();
        let mut discovery = module_discovery::ModuleDiscovery::new(
            resolver::ModuleResolver::new(resolver::config::resolver_config(loaded), resolver::host::DiskHost::default()),
            module_discovery::InclusionOptions {
                allow_js: loaded.compiler_options.allow_js || loaded.compiler_options.check_js == Some(true),
                jsx: loaded.compiler_options.jsx.is_some(),
                resolve_json_module: loaded.compiler_options.resolve_json_module,
                allow_arbitrary_extensions: loaded.compiler_options.allow_arbitrary_extensions,
                max_node_module_js_depth: loaded.compiler_options.max_node_module_js_depth,
            },
        );

        loop {
            let files_before = inputs.len();
            if !legacy_resolution {
                let discovery_start = Instant::now();
                let (added, resolutions) = discovery.expand(
                    &mut specifier_scanner,
                    &mut inputs,
                    &mut sources,
                    &resolver_options,
                    &mut package_resolution_cache,
                );
                module_discovery::record_resolutions(resolutions, &mut resolved_modules, &mut resolved_modules_by_importer);
                if collect {
                    timings.import_graph_expansion += discovery_start.elapsed();
                }
                reference_type_resolver.scan_and_resolve(
                    &mut inputs,
                    &mut sources,
                    &resolver_options,
                    &mut package_resolution_cache,
                );
                if added == 0 && inputs.len() == files_before {
                    break;
                }
                continue;
            }

            let package_start = Instant::now();
            let package_io_before = if collect {
                io_stats::snapshot()
            } else {
                io_stats::IoSnapshot::default()
            };
            let package_modules =
                package_declarations::resolve_package_declaration_entrypoints_with_cache(
                    &mut inputs,
                    &mut sources,
                    &loaded.root_dir,
                    &resolver_options,
                    &mut package_resolution_cache,
                    &mut specifier_scanner,
                );
            if collect {
                timings.package_declaration_discovery += package_start.elapsed();
                let package_io = io_stats::snapshot();
                timings.package_declaration_probes +=
                    package_io.fs_existence_probes - package_io_before.fs_existence_probes;
                timings.package_declaration_probe_io += package_io
                    .fs_existence_probe_io
                    .saturating_sub(package_io_before.fs_existence_probe_io);
            }
            // Package resolutions are importer-scoped; the flat map keeps the
            // first (BFS-order) resolution per specifier as the project-wide
            // fallback for importer-agnostic consumers. An importer whose own
            // resolution failed keeps that failure rather than the fallback.
            // A usage whose syntax picks its mode (`import x = require()`, a
            // `resolution-mode` attribute) also has a key of its own; the
            // plain key holds the importer's own mode wherever it was used.
            for resolution in package_modules {
                use package_declarations::ImportUsage;
                if !resolution.resolved_file.is_empty()
                    && !matches!(resolution.usage, ImportUsage::ModeOverride(_))
                {
                    resolved_modules
                        .entry(resolution.specifier.clone())
                        .or_insert_with(|| resolution.resolved_file.clone());
                }
                let per_importer = resolved_modules_by_importer
                    .entry(resolution.importer)
                    .or_default();
                match resolution.usage {
                    ImportUsage::Declaration | ImportUsage::DynamicImport | ImportUsage::Require => {
                        per_importer.insert(resolution.specifier, resolution.resolved_file);
                    }
                    ImportUsage::ImportEquals => {
                        per_importer.insert(
                            surge_ts_checker::lowlevel::resolution_candidates::resolution_mode_override_key(
                                &resolution.specifier,
                                surge_ts_syntax::ResolutionModeOverride::Require,
                            ),
                            resolution.resolved_file.clone(),
                        );
                        per_importer
                            .entry(resolution.specifier)
                            .or_insert(resolution.resolved_file);
                    }
                    ImportUsage::ModeOverride(mode) => {
                        per_importer.insert(
                            surge_ts_checker::lowlevel::resolution_candidates::resolution_mode_override_key(
                                &resolution.specifier,
                                mode,
                            ),
                            resolution.resolved_file,
                        );
                    }
                }
            }

            let import_graph_start = Instant::now();
            let graph_loaded = import_graph::expand_project_inputs(
                &mut import_graph_state,
                &mut specifier_scanner,
                &mut inputs,
                &mut sources,
                &loaded.root_dir,
                loaded.compiler_options.base_url.as_deref(),
                &loaded.compiler_options.paths,
                loaded.compiler_options.resolve_json_module,
                &mut javascript_modules,
            );
            if collect {
                timings.import_graph_expansion += import_graph_start.elapsed();
            }

            reference_type_resolver.scan_and_resolve(
                &mut inputs,
                &mut sources,
                &resolver_options,
                &mut package_resolution_cache,
            );

            if graph_loaded == 0 && inputs.len() == files_before {
                break;
            }
        }
        for (importer, specifier, resolved_file) in javascript_modules {
            resolved_modules_by_importer
                .entry(importer)
                .or_default()
                .entry(specifier)
                .or_insert(resolved_file);
        }
        surge_ts_checker::lowlevel::record_loader_rss_stage("import_graph_expanded");
        io_stats::report_probe_dirs();

        if collect {
            let io = io_stats::snapshot();
            timings.expansion_read_io += io
                .expansion_read_io
                .saturating_sub(io_baseline.expansion_read_io);
            timings.expansion_files_read +=
                io.expansion_files_read - io_baseline.expansion_files_read;
            timings.expansion_bytes_read +=
                io.expansion_bytes_read - io_baseline.expansion_bytes_read;
            timings.package_declaration_read_io += io
                .package_declaration_read_io
                .saturating_sub(io_baseline.package_declaration_read_io);
            timings.package_json_reads += io.package_json_reads - io_baseline.package_json_reads;
            timings.fs_existence_probes += io.fs_existence_probes - io_baseline.fs_existence_probes;
            timings.fs_existence_probe_io += io
                .fs_existence_probe_io
                .saturating_sub(io_baseline.fs_existence_probe_io);
            timings.fs_read_dir_count += io.fs_read_dir_count - io_baseline.fs_read_dir_count;
            timings.fs_read_dir_io += io.fs_read_dir_io.saturating_sub(io_baseline.fs_read_dir_io);
            let canonicalize = surge_ts_config::canonicalize_io_snapshot();
            timings.canonicalize_memo_misses +=
                canonicalize.memo_misses - canonicalize_baseline.memo_misses;
            timings.canonicalize_full_realpaths +=
                canonicalize.full_realpaths - canonicalize_baseline.full_realpaths;
            timings.canonicalize_leaf_probes +=
                canonicalize.leaf_probes - canonicalize_baseline.leaf_probes;
            timings.canonicalize_miss_io += canonicalize
                .miss_io
                .saturating_sub(canonicalize_baseline.miss_io);
        }

        for resolution in (if legacy_resolution { package_declarations::resolve_module_augmentation_specifiers(
            &sources,
            &loaded.root_dir,
            &resolver_options,
            &mut package_resolution_cache,
            &specifier_scanner,
        ) } else { Vec::new() }) {
            resolved_modules_by_importer
                .entry(resolution.importer)
                .or_default()
                .entry(resolution.specifier)
                .or_insert(resolution.resolved_file);
        }

        // Path mapping reuses the scanner's cached per-file specifier lists,
        // so it must run against the pre-splice `sources` order the scanner
        // was indexed by (default libs contribute no external specifiers).
        let path_mapping_start = Instant::now();
        let path_modules = if !legacy_resolution { Default::default() } else { path_mapping::resolve_path_mappings(
            &inputs,
            &sources,
            &mut specifier_scanner,
            &loaded.compiler_options.paths,
            loaded.compiler_options.base_url.as_deref(),
            &loaded.root_dir,
        ) };
        for (k, v) in path_modules {
            resolved_modules.insert(k, v);
        }
        if collect {
            timings.path_mapping_resolution += path_mapping_start.elapsed();
        }

        let file_redirects = if loaded.compiler_options.deduplicate_packages {
            let redirects = package_dedup::package_redirects(&sources);
            package_dedup::apply_package_redirects(
                &redirects,
                &mut inputs,
                &mut sources,
                &mut resolved_modules,
                &mut resolved_modules_by_importer,
            );
            redirects
        } else {
            Default::default()
        };

        // The lib set is only known once every program file is: tsc's file
        // loader adds each file's `/// <reference lib>` to the program.
        let referenced_libs = if loaded.compiler_options.no_lib {
            Vec::new()
        } else {
            referenced_lib_names(&inputs)
        };
        let default_lib_loading_start = Instant::now();
        let lib_replacements = std::cell::RefCell::new((
            std::collections::HashMap::<String, Option<PathBuf>>::new(),
            &mut package_resolution_cache,
        ));
        let config_dir = loaded
            .config_path
            .parent()
            .unwrap_or(&loaded.root_dir)
            .to_path_buf();
        let lib_replacement = |normalized_name: &str| -> Option<PathBuf> {
            let mut state = lib_replacements.borrow_mut();
            let (resolved, cache) = &mut *state;
            resolved
                .entry(normalized_name.to_string())
                .or_insert_with(|| {
                    package_declarations::resolve_lib_replacement(
                        normalized_name,
                        &config_dir,
                        &resolver_options,
                        cache,
                    )
                })
                .clone()
        };
        let default_lib_load = load_default_lib_inputs(DefaultLibRequest {
            no_lib: loaded.compiler_options.no_lib,
            lib_entries: loaded.compiler_options.lib.as_slice(),
            referenced_libs: &referenced_libs,
            root_dir: &loaded.root_dir,
            target_basename: target_lib_basename(loaded.compiler_options.target),
            source: options.lib_source.clone(),
            lib_replacement: loaded
                .compiler_options
                .lib_replacement
                .then_some(&lib_replacement as &dyn Fn(&str) -> Option<PathBuf>),
        });
        // A replacement is a `node_modules` declaration file, whose globals
        // the checker publishes only for a package on its `types` list.
        let lib_replacement_packages = lib_replacements
            .into_inner()
            .0
            .into_iter()
            .filter(|(_, replacement)| replacement.is_some())
            .filter_map(|(normalized_name, _)| {
                package_declarations::lib_replacement_package_name(&normalized_name)
            })
            .collect::<std::collections::BTreeSet<_>>();
        for unknown in &default_lib_load.unknown_libs {
            warnings.push(format!(
                "unknown lib '{unknown}' in compilerOptions.lib; no matching lib*.d.ts file"
            ));
        }
        if let Some(error) = &default_lib_load.override_error
            && !loaded.compiler_options.no_lib
        {
            warnings.push(error.clone());
        }
        let default_lib_io = default_lib_load.io_stats;
        let default_lib_inputs = default_lib_load.inputs;
        if collect {
            timings.default_lib_loading += default_lib_loading_start.elapsed();
            timings.default_lib_files_read += default_lib_inputs.len() as u64;
            timings.default_lib_bytes_read += default_lib_inputs
                .iter()
                .map(|input| input.source_text.len() as u64)
                .sum::<u64>();
            timings.default_lib_read_io += default_lib_io.read_io;
            timings.default_lib_existence_probes += default_lib_io.existence_probes;
            timings.default_lib_canonicalize_syscalls += default_lib_io.canonicalize_syscalls;
        }
        surge_ts_checker::lowlevel::record_loader_rss_stage("default_libs_loaded");

        // Default-lib sources never contribute project imports or package
        // specifiers, so they stay out of the package-declaration / import-graph
        // scan above. Splice them to the front now, preserving the
        // `[default libs..., project files...]` order the checker expects.
        if !default_lib_inputs.is_empty() {
            let default_lib_sources = default_lib_inputs
                .iter()
                .map(|input| {
                    (
                        PathBuf::from(&input.file_name),
                        input.file_name.clone(),
                        if options.retain_all_sources {
                            input.source_text.clone()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect::<Vec<_>>();
            inputs.splice(0..0, default_lib_inputs);
            sources.splice(0..0, default_lib_sources);
        }

        let reference_type_resolution = reference_type_resolver.into_resolution();

        // The scan caches (parser arena, per-file specifier lists, probe and
        // known-file sets) are loader-lifetime only; release them before the
        // checker's peak so they never count against the program footprint.
        // The package-declaration cache (parsed package.json values) and the
        // path-canonicalize memo are likewise done: nothing canonicalizes or
        // resolves packages after this point, and a later `check` on a fresh
        // `Project` simply re-fills them.
        //
        // The parses themselves are not loader-lifetime: the checker would
        // otherwise re-parse every one of these files from the same text, so
        // they move on to it instead of being dropped with the scanner.
        let prescanned_sources = specifier_scanner.take_parsed_sources();
        drop(specifier_scanner);
        drop(import_graph_state);
        drop(package_resolution_cache);
        probe::clear_probe_cache();
        surge_ts_config::clear_canonicalize_cache();

        let mut checker_types = type_package_resolution.effective_type_names.clone();
        for name in &reference_type_resolution.effective_type_names {
            if !checker_types.contains(name) {
                checker_types.push(name.clone());
            }
        }
        for package in lib_replacement_packages {
            if !checker_types.contains(&package) {
                checker_types.push(package);
            }
        }
        if loaded
            .compiler_options
            .types
            .as_deref()
            .is_some_and(|types| types.iter().any(|name| name == "*"))
        {
            checker_types.push("*".to_string());
        }

        if loaded
            .compiler_options
            .lib
            .iter()
            .any(|lib| lib.eq_ignore_ascii_case("dom"))
        {
            resolved_modules.insert(CheckerOptions::LIB_DOM_SENTINEL.to_string(), String::new());
        }

        surge_ts_checker::set_fast_process_exit(options.fast_process_exit);
        let node_module_resolution = matches!(
            loaded.compiler_options.module_resolution,
            surge_ts_config::ModuleResolutionKind::Node16
                | surge_ts_config::ModuleResolutionKind::Node20
                | surge_ts_config::ModuleResolutionKind::NodeNext
        );
        let esm_module_files = if node_module_resolution {
            esm_format_files(sources.iter().map(|(path, name, _)| (path.as_path(), name.as_str())))
        } else {
            Default::default()
        };
        let esm_package_files = if node_module_resolution {
            Default::default()
        } else {
            esm_format_files(
                sources
                    .iter()
                    .filter(|(_, name, _)| name.contains("/node_modules/"))
                    .map(|(path, name, _)| (path.as_path(), name.as_str())),
            )
        };
        let checker_options = CheckerOptions {
            no_implicit_any: loaded.compiler_options.no_implicit_any,
            no_implicit_this: loaded.compiler_options.no_implicit_this,
            module_emit: checker_module_emit(loaded.compiler_options.emit_module),
            verbatim_module_syntax: loaded.compiler_options.verbatim_module_syntax,
            isolated_modules: loaded.compiler_options.isolated_modules,
            preserve_const_enums: loaded.compiler_options.preserve_const_enums
                || loaded.compiler_options.isolated_modules
                || loaded.compiler_options.verbatim_module_syntax,
            use_define_for_class_fields: loaded.compiler_options.use_define_for_class_fields,
            target_es2022: loaded.compiler_options.target >= ScriptTarget::ES2022,
            target_es2016: loaded.compiler_options.target >= ScriptTarget::ES2016,
            no_emit: loaded.compiler_options.no_emit,
            node_module_resolution,
            esm_module_files,
            esm_package_files,
            strict_null_checks: loaded.compiler_options.strict_null_checks,
            strict_function_types: loaded.compiler_options.strict_function_types,
            strict_bind_call_apply: loaded.compiler_options.strict_bind_call_apply,
            strict_builtin_iterator_return: loaded.compiler_options.strict_builtin_iterator_return,
            strict_property_initialization: loaded
                .compiler_options
                .strict_property_initialization,
            use_unknown_in_catch_variables: loaded.compiler_options.use_unknown_in_catch_variables,
            no_implicit_returns: loaded.compiler_options.no_implicit_returns,
            no_fallthrough_cases_in_switch: loaded.compiler_options.no_fallthrough_cases_in_switch,
            no_implicit_override: loaded.compiler_options.no_implicit_override,
            no_property_access_from_index_signature: loaded
                .compiler_options
                .no_property_access_from_index_signature,
            no_unchecked_indexed_access: loaded.compiler_options.no_unchecked_indexed_access,
            no_unchecked_side_effect_imports: loaded
                .compiler_options
                .no_unchecked_side_effect_imports,
            exact_optional_property_types: loaded.compiler_options.exact_optional_property_types,
            // tsc's `GetAllowImportingTsExtensions`.
            allow_importing_ts_extensions: loaded.compiler_options.allow_importing_ts_extensions
                || loaded.compiler_options.rewrite_relative_import_extensions,
            rewrite_relative_import_extensions: loaded.compiler_options.rewrite_relative_import_extensions,
            declaration: loaded.compiler_options.declaration || loaded.compiler_options.composite,
            allow_arbitrary_extensions: loaded.compiler_options.allow_arbitrary_extensions,
            experimental_decorators: loaded.compiler_options.experimental_decorators,
            import_helpers: loaded.compiler_options.import_helpers,
            emit_decorator_metadata: loaded.compiler_options.emit_decorator_metadata,
            no_unused_locals: loaded.compiler_options.no_unused_locals,
            no_unused_parameters: loaded.compiler_options.no_unused_parameters,
            allow_unreachable_code: loaded.compiler_options.allow_unreachable_code,
            report_unreachable_code: loaded.compiler_options.report_unreachable_code,
            allow_unused_labels: loaded.compiler_options.allow_unused_labels,
            no_lib: loaded.compiler_options.no_lib,
            skip_lib_check: loaded.compiler_options.skip_lib_check,
            stub_external_modules: options.stub_external_modules,
            resolved_modules,
            resolved_modules_by_importer,
            file_redirects,
            types: checker_types,
            jsx_automatic_runtime: matches!(
                loaded.compiler_options.jsx,
                Some(surge_ts_config::JsxMode::ReactJsx | surge_ts_config::JsxMode::ReactJsxDev)
            ),
            jsx_classic_react: loaded.compiler_options.jsx == Some(surge_ts_config::JsxMode::React),
            jsx_emit_none: loaded.compiler_options.jsx.is_none(),
            allow_umd_global_access: loaded.compiler_options.allow_umd_global_access,
            resolve_json_module: loaded.compiler_options.resolve_json_module,
            // tsc's `GetAllowJS`: `checkJs` implies `allowJs`.
            allow_js: loaded.compiler_options.allow_js || loaded.compiler_options.check_js == Some(true),
            check_js: loaded.compiler_options.check_js,
            erasable_syntax_only: loaded.compiler_options.erasable_syntax_only,
            module_detection: surge_ts_checker::ModuleDetection {
                force: loaded.compiler_options.module_detection == surge_ts_config::ModuleDetectionKind::Force,
                legacy: loaded.compiler_options.module_detection == surge_ts_config::ModuleDetectionKind::Legacy,
            },
            language_version: language_version(loaded.compiler_options.target),
            jsx_configured: loaded.compiler_options.jsx.is_some(),
            jsx_factory_names: surge_ts_checker::JsxFactoryNames {
                factory: loaded.compiler_options.jsx_factory.clone(),
                fragment_factory: loaded.compiler_options.jsx_fragment_factory.clone(),
                react_namespace: loaded.compiler_options.react_namespace.clone(),
                import_source: loaded.compiler_options.jsx_import_source.clone(),
                development: loaded.compiler_options.jsx == Some(surge_ts_config::JsxMode::ReactJsxDev),
            },
            diagnostic_profile: options.diagnostic_profile,
        };

        // The loader is done with source texts (specifier scan and path mapping
        // ran above), and the checker owns its own copies in `inputs`. Holding
        // this second copy of every dependency `.d.ts` through the checker's
        // peak costs tens of MB; drop the bodies now and re-read the few files
        // that end up with diagnostics after checking. Entries without an
        // on-disk file keep their text — they cannot be re-read.
        if !options.retain_all_sources {
            for (file_path, _, source_text) in &mut sources {
                if !source_text.is_empty() && host_fs::is_file(file_path) {
                    *source_text = String::new();
                }
            }
        }

        Ok(Prepared::Ready(
            PreparedRun {
                inputs,
                prescanned: prescanned_sources,
                checker_options,
            },
            PreparedFinish {
                loaded,
                sources,
                warnings,
                timings,
                type_package_missing: type_package_resolution.missing,
                reference_type_resolution,
            },
        ))
    }
}

/// A diagnostic located in an embedded lib file is reported where tsc reports
/// it, in the installed TypeScript's copy of that file (the embedded snapshot
/// is the same text), with the text kept for its line and column.
fn locate_embedded_lib_diagnostics(diagnostics: &mut [Diagnostic], sources: &mut Vec<ProjectSource>, root_dir: &Path) {
    use surge_ts_checker::lowlevel::{EMBEDDED_LIB_DIR, embedded_lib_source};
    let prefix = format!("{EMBEDDED_LIB_DIR}/");
    if !diagnostics.iter().any(|diagnostic| diagnostic.file_name.starts_with(&prefix)) {
        return;
    }
    let installed = installed_typescript_lib_dir(root_dir);
    let mut located: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for diagnostic in diagnostics.iter_mut() {
        let Some(base_name) = diagnostic.file_name.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(file_name) = located.get(&diagnostic.file_name) {
            diagnostic.file_name = file_name.clone();
            continue;
        }
        let Some(text) = base_name
            .strip_prefix("lib.")
            .and_then(|name| name.strip_suffix(".d.ts"))
            .and_then(embedded_lib_source)
        else {
            continue;
        };
        let installed_file = installed
            .as_ref()
            .map(|dir| dir.join(base_name))
            .filter(|path| host_fs::is_file(path));
        let file_name = match &installed_file {
            Some(path) => path.to_string_lossy().into_owned(),
            None => diagnostic.file_name.clone(),
        };
        let path = installed_file.unwrap_or_else(|| PathBuf::from(&file_name));
        match sources.iter_mut().find(|(_, name, _)| *name == file_name) {
            Some((_, _, source_text)) => {
                if source_text.is_empty() {
                    *source_text = text.to_string();
                }
            }
            None => sources.push((path, file_name.clone(), text.to_string())),
        }
        located.insert(diagnostic.file_name.clone(), file_name.clone());
        diagnostic.file_name = file_name;
    }
}

/// The lib directory of the TypeScript package installed above `root_dir`:
/// a native (7.x) release keeps its libs in the platform package it resolves
/// from beside itself, an earlier one in its own `lib`.
fn installed_typescript_lib_dir(root_dir: &Path) -> Option<PathBuf> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    let platform_package = format!("typescript-{os}-{arch}");
    let mut current = Some(root_dir);
    while let Some(dir) = current {
        let package = dir.join("node_modules").join("typescript");
        if let Ok(real) = std::fs::canonicalize(&package) {
            let own = real.join("lib");
            if own.join("lib.es5.d.ts").is_file() {
                return Some(own);
            }
            let candidates = [
                real.parent().map(|parent| parent.join("@typescript").join(&platform_package).join("lib")),
                Some(dir.join("node_modules").join("@typescript").join(&platform_package).join("lib")),
            ];
            for candidate in candidates.into_iter().flatten() {
                if candidate.join("lib.es5.d.ts").is_file() {
                    return std::fs::canonicalize(&candidate).ok();
                }
            }
        }
        current = dir.parent();
    }
    None
}

/// The package resolver's options for a loaded config.
pub(crate) fn resolver_options(loaded: &LoadedTsConfig) -> package_resolution::ResolverOptions {
    package_resolution::ResolverOptions {
        module_resolution: loaded.compiler_options.module_resolution,
        resolve_exports: loaded.compiler_options.resolve_package_json_exports,
        resolve_imports: loaded.compiler_options.resolve_package_json_imports,
        custom_conditions: loaded.compiler_options.custom_conditions.clone(),
        path_mappings: loaded.compiler_options.paths.clone(),
        path_mapping_base: Some(
            loaded
                .compiler_options
                .base_url
                .clone()
                .unwrap_or_else(|| loaded.root_dir.clone()),
        ),
        emit_module: loaded.compiler_options.emit_module,
        resolve_json_module: loaded.compiler_options.resolve_json_module,
    }
}

/// What [`Project::prepare`] hands the type check.
pub(crate) struct PreparedRun {
    pub(crate) inputs: Vec<SourceFileInput>,
    pub(crate) prescanned: Vec<surge_ts_syntax::ParsedSource>,
    pub(crate) checker_options: CheckerOptions,
}

/// What [`Project::prepare`] keeps for turning the check's result into the
/// project's.
pub(crate) struct PreparedFinish<'a> {
    loaded: &'a LoadedTsConfig,
    pub(crate) sources: Vec<ProjectSource>,
    warnings: Vec<String>,
    timings: ProjectTimings,
    type_package_missing: Vec<String>,
    reference_type_resolution: package_declarations::ReferenceTypeDirectiveResolution,
}

pub(crate) enum Prepared<'a> {
    Empty(ProjectCheckResult),
    Ready(PreparedRun, PreparedFinish<'a>),
}

impl PreparedFinish<'_> {
    pub(crate) fn finish(
        self,
        result: ProgramCheckResult,
        checking: Duration,
        options: &ProjectOptions,
    ) -> ProjectCheckResult {
        let stats = result.stats.clone();
        let root_dir = self.loaded.root_dir.clone();
        let (parts, mut sources, warnings, timings) = self.into_parts(result, checking, options);
        let mut diagnostics = parts.tsc_diagnostics();
        locate_embedded_lib_diagnostics(&mut diagnostics, &mut sources, &root_dir);

        if !options.retain_all_sources {
            let needed: std::collections::HashSet<&str> = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.file_name.as_str())
                .collect();
            for (file_path, file_name, source_text) in &mut sources {
                if source_text.is_empty()
                    && needed.contains(file_name.as_str())
                    && let Ok(read) = host_fs::read_to_string(file_path)
                {
                    *source_text = read;
                }
            }
        }

        ProjectCheckResult {
            diagnostics,
            stats,
            sources,
            warnings,
            timings,
        }
    }

    /// Sorts what the check and the loader found into the sets tsc's program
    /// reports them through.
    pub(crate) fn into_parts(
        self,
        result: ProgramCheckResult,
        checking: Duration,
        options: &ProjectOptions,
    ) -> (DiagnosticParts, Vec<ProjectSource>, Vec<String>, ProjectTimings) {
        let PreparedFinish {
            loaded,
            sources,
            warnings,
            mut timings,
            type_package_missing,
            reference_type_resolution,
        } = self;
        if options.collect_timings {
            timings.checking += checking;
        }

        let config = invalid_enum_option_diagnostics(loaded);
        let mut program_diagnostics = removed_options_only_diagnostics(loaded);
        program_diagnostics.extend(jsx_factory_option_diagnostics(loaded));
        program_diagnostics.extend(root_dir_diagnostics(loaded, &sources));
        program_diagnostics.extend(
            type_package_missing
                .iter()
                .map(|type_name| Diagnostic::ts2688(type_name, String::new())),
        );
        let syntax_errors = result.syntax_errors;
        let no_check = loaded.compiler_options.no_check;
        let checked = if syntax_errors {
            result.diagnostics
        } else if no_check {
            // tsc's `SkipTypeChecking`: every file's bind, check and inclusion
            // diagnostics are skipped, leaving the location-less ones.
            result
                .diagnostics
                .into_iter()
                .filter(|diagnostic| diagnostic.file_name.is_empty())
                .collect()
        } else {
            let mut diagnostics = apply_project_no_lib_compatibility_diagnostics(
                result.diagnostics,
                loaded.compiler_options.no_lib,
                !loaded.compiler_options.type_roots.is_empty(),
                &result.no_lib_missing_global_types,
                options.diagnostic_profile,
            );
            for missing in &reference_type_resolution.missing {
                if loaded.compiler_options.skip_lib_check && missing.from_declaration_file {
                    continue;
                }
                diagnostics.push(
                    Diagnostic::ts2688(&missing.type_name, missing.file_name.clone()).with_span(
                        TextSpan {
                            start: missing.value_span.start,
                            end: missing.value_span.end,
                        },
                    ),
                );
            }
            // tsc's `checkGrammarForUseStrictSimpleParameterList` runs from
            // ES2016 on, a target the grammar pass that finds these cannot see.
            if loaded.compiler_options.target < ScriptTarget::ES2016 {
                diagnostics.retain(|diagnostic| {
                    !matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(1346 | 1347))
                });
            }
            for unresolved in &reference_type_resolution.unresolved_paths {
                if loaded.compiler_options.skip_lib_check && unresolved.from_declaration_file {
                    continue;
                }
                diagnostics.push(
                    reference_path_diagnostic(unresolved, reference_type_resolution.allow_js).with_span(
                        TextSpan {
                            start: unresolved.value_span.start,
                            end: unresolved.value_span.end,
                        },
                    ),
                );
            }
            diagnostics
        };
        let declaration = if loaded.compiler_options.isolated_declarations
            && (loaded.compiler_options.declaration || loaded.compiler_options.composite)
        {
            isolated_declaration_diagnostics(&sources, loaded.compiler_options.strict_null_checks)
        } else {
            Vec::new()
        };
        let mut declaration_emit = if loaded.compiler_options.declaration || loaded.compiler_options.composite {
            declaration_emit_diagnostics(&sources)
        } else {
            Vec::new()
        };
        // What declaration emit finds in the types the checker inferred is
        // reported with the rest of emit's findings, not as a semantic error.
        let (emitted, checked): (Vec<_>, Vec<_>) = checked.into_iter().partition(|diagnostic| {
            matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(4094))
        });
        declaration_emit.extend(emitted);
        let parts = DiagnosticParts {
            config,
            syntax_errors,
            program: program_diagnostics,
            checked,
            declaration,
            declaration_emit,
            no_emit_on_error: loaded.compiler_options.no_emit_on_error,
        };
        (parts, sources, warnings, timings)
    }
}

/// A checked program's diagnostics, sorted by the tsc program API that
/// reports each.
pub(crate) struct DiagnosticParts {
    /// What reading the config file reports (`GetConfigFileParsingDiagnostics`),
    /// which every other set is reported beside.
    pub(crate) config: Vec<Diagnostic>,
    /// A file failed to parse: `checked` holds the syntactic diagnostics only.
    pub(crate) syntax_errors: bool,
    /// Location-less option and file-inclusion diagnostics
    /// (`GetOptionsDiagnostics`).
    pub(crate) program: Vec<Diagnostic>,
    /// The syntactic diagnostics when `syntax_errors`, else the global and
    /// semantic ones.
    pub(crate) checked: Vec<Diagnostic>,
    /// What declaration emit reports under `isolatedDeclarations`.
    pub(crate) declaration: Vec<Diagnostic>,
    /// What declaration emit reports under `declaration` alone.
    pub(crate) declaration_emit: Vec<Diagnostic>,
    /// `noEmitOnError`: emit, and with it what declaration emit reports, is
    /// skipped when the program reports anything else.
    pub(crate) no_emit_on_error: bool,
}

impl DiagnosticParts {
    /// tsc's `GetDiagnosticsOfAnyProgram`: syntactic diagnostics alone when
    /// there are any, else the program's option and location-less
    /// file-inclusion diagnostics alone when there are any, else the
    /// semantic ones; then what declaration emit reports. `Program.Emit` runs
    /// under `noEmit` too, so those are reported whatever else is, unless
    /// `noEmitOnError` skips emit for it (`HandleNoEmitOnError`).
    pub(crate) fn tsc_diagnostics(&self) -> Vec<Diagnostic> {
        let reported = if self.syntax_errors || self.program.is_empty() { &self.checked } else { &self.program };
        let mut diagnostics = self.config.clone();
        diagnostics.extend(reported.iter().cloned());
        if self.no_emit_on_error && !reported.is_empty() {
            return diagnostics;
        }
        if self.declaration.is_empty() {
            diagnostics.extend(self.declaration_emit.iter().cloned());
        }
        diagnostics.extend(self.declaration.iter().cloned());
        diagnostics
    }
}

/// tsc's declaration emit under `isolatedDeclarations`, which reports through
/// `Program.Emit` even with `noEmit` and whatever else the program reports:
/// every emitted TypeScript source — not a declaration file, not a package's.
fn isolated_declaration_diagnostics(
    sources: &[(PathBuf, String, String)],
    strict_null_checks: bool,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (file_path, file_name, source_text) in sources {
        let typescript = [".ts", ".tsx", ".mts", ".cts"].iter().any(|extension| file_name.ends_with(extension));
        if !typescript
            || surge_ts_syntax::is_declaration_file_name(file_name)
            || file_name.contains("/node_modules/")
        {
            continue;
        }
        let read;
        let text = if source_text.is_empty() {
            let Ok(text) = host_fs::read_to_string(file_path) else {
                continue;
            };
            read = text;
            &read
        } else {
            source_text
        };
        for finding in surge_ts_syntax::isolated_declaration_diagnostics(text, file_name, strict_null_checks) {
            let Some(descriptor) = surge_ts_diagnostics::emitted_descriptor_for_number(finding.code) else {
                continue;
            };
            diagnostics.push(
                Diagnostic::from_descriptor(descriptor, Vec::new(), file_name.clone()).with_span(TextSpan {
                    start: finding.start as usize,
                    end: finding.end as usize,
                }),
            );
        }
    }
    diagnostics
}

/// [`isolated_declaration_diagnostics`]'s counterpart for what declaration
/// emit reports under `declaration` alone.
fn declaration_emit_diagnostics(sources: &[(PathBuf, String, String)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (file_path, file_name, source_text) in sources {
        let typescript = [".ts", ".tsx", ".mts", ".cts"].iter().any(|extension| file_name.ends_with(extension));
        if !typescript
            || surge_ts_syntax::is_declaration_file_name(file_name)
            || file_name.contains("/node_modules/")
        {
            continue;
        }
        let read;
        let text = if source_text.is_empty() {
            let Ok(text) = host_fs::read_to_string(file_path) else {
                continue;
            };
            read = text;
            &read
        } else {
            source_text
        };
        for finding in surge_ts_syntax::declaration_emit_diagnostics(text, file_name) {
            let args: Vec<surge_ts_diagnostics::DiagnosticArg> = finding.args.into_iter().map(Into::into).collect();
            let Some(descriptor) =
                surge_ts_diagnostics::emitted_descriptor_for_number_with_arity(finding.code, args.len())
            else {
                continue;
            };
            diagnostics.push(Diagnostic::from_descriptor(descriptor, args, file_name.clone()).with_span(TextSpan {
                start: finding.start as usize,
                end: finding.end as usize,
            }));
        }
    }
    diagnostics
}

type SourceReadResult = Result<ProjectSource, (PathBuf, std::io::Error)>;

fn read_one_source(
    file_path: &PathBuf,
    read_nanos: &std::sync::atomic::AtomicU64,
) -> SourceReadResult {
    let read_start = Instant::now();
    let read = host_fs::read_to_string(file_path);
    read_nanos.fetch_add(
        read_start.elapsed().as_nanos() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    match read {
        Ok(source_text) => {
            let file_name = canonicalize_if_exists_string(file_path);
            Ok((file_path.clone(), file_name, source_text))
        }
        Err(error) => Err((file_path.clone(), error)),
    }
}

// Project source reads are I/O-bound and independent, so reading them across a
// few threads overlaps the waits. Contiguous chunks keep the original file
// ordering, which the checker relies on.
fn read_project_sources(
    files: &[PathBuf],
    workers: usize,
    read_nanos: &std::sync::atomic::AtomicU64,
) -> Result<Vec<ProjectSource>, (PathBuf, std::io::Error)> {
    if workers <= 1 || files.len() <= 1 || host_fs::is_custom() {
        return files
            .iter()
            .map(|f| read_one_source(f, read_nanos))
            .collect();
    }

    let chunk_size = files.len().div_ceil(workers);
    let chunk_results: Vec<Result<Vec<ProjectSource>, (PathBuf, std::io::Error)>> =
        std::thread::scope(|scope| {
            let handles: Vec<_> = files
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|f| read_one_source(f, read_nanos))
                            .collect()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("source reader thread panicked"))
                .collect()
        });

    let mut sources = Vec::with_capacity(files.len());
    for chunk in chunk_results {
        sources.extend(chunk?);
    }
    Ok(sources)
}

/// tsgo's `getSourceFileFromReference` messages for a `/// <reference path>`
/// that names no file: the path as written with its slashes normalized, and
/// every supported extension, flattened.
fn reference_path_diagnostic(
    unresolved: &package_declarations::UnresolvedReferencePath,
    allow_js: bool,
) -> Diagnostic {
    use package_declarations::ReferencePathFailure;
    let path = unresolved.path.replace('\\', "/");
    let file_name = unresolved.file_name.clone();
    let extensions = || {
        package_declarations::supported_extensions(allow_js)
            .iter()
            .map(|extension| format!("'{extension}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match unresolved.failure {
        ReferencePathFailure::NotFound => Diagnostic::ts6053(path, file_name),
        ReferencePathFailure::NoExtensionMatched => Diagnostic::ts6231(path, extensions(), file_name),
        ReferencePathFailure::JavaScriptFile => Diagnostic::ts6504(path, file_name),
        ReferencePathFailure::UnsupportedExtension => Diagnostic::ts6054(path, extensions(), file_name),
        ReferencePathFailure::SelfReference => Diagnostic::ts1006(file_name),
    }
}

/// `TS5102`/`TS5108` for every compiler option TypeScript 7 removed, spanned
/// inside the config file the way tsc spans them (the value node for the
/// `name=value` form, the key node otherwise), and TS6046 for an enum option
/// whose value is none of its keys.
pub fn removed_option_diagnostics(loaded: &LoadedTsConfig) -> Vec<Diagnostic> {
    let mut diagnostics = removed_options_only_diagnostics(loaded);
    diagnostics.extend(invalid_enum_option_diagnostics(loaded));
    diagnostics
}

/// The removed options alone, which `verifyCompilerOptions` reports among
/// the program's option diagnostics.
/// tsc's `checkSourceFilesBelongToPath` under `rootDir`: every source file
/// the program may emit (`sourceFileMayBeEmitted`) lies under it.
fn root_dir_diagnostics(loaded: &LoadedTsConfig, sources: &[ProjectSource]) -> Vec<Diagnostic> {
    let Some(root_dir) = loaded.compiler_options.root_dir.as_deref() else {
        return Vec::new();
    };
    let root = std::fs::canonicalize(root_dir).unwrap_or_else(|_| root_dir.to_path_buf());
    let emitted_extensions: &[&str] = if loaded.compiler_options.allow_js {
        &[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"]
    } else {
        &[".ts", ".tsx", ".mts", ".cts"]
    };
    sources
        .iter()
        .filter(|(path, name, _)| {
            !surge_ts_syntax::is_declaration_file_name(name)
                && !name.split(['/', '\\']).any(|segment| segment == "node_modules")
                && emitted_extensions.iter().any(|extension| name.ends_with(extension))
                && !path.starts_with(&root)
        })
        .map(|(_, name, _)| Diagnostic::ts6059(name, root_dir.display(), String::new()))
        .collect()
}

fn removed_options_only_diagnostics(loaded: &LoadedTsConfig) -> Vec<Diagnostic> {
    let file_name = loaded.config_path.display().to_string();
    loaded
        .removed_options
        .iter()
        .map(|option| {
            let diagnostic = match &option.value {
                Some(value) => Diagnostic::ts5108(&option.name, value, file_name.clone()),
                None => Diagnostic::ts5102(&option.name, file_name.clone()),
            };
            diagnostic.with_span(TextSpan {
                start: option.start,
                end: option.end,
            })
        })
        .collect()
}

/// The invalid enum values alone, which the config file's parser reports.
fn invalid_enum_option_diagnostics(loaded: &LoadedTsConfig) -> Vec<Diagnostic> {
    let file_name = loaded.config_path.display().to_string();
    surge_ts_config::invalid_enum_options(&loaded.config_path)
        .into_iter()
        .map(|option| {
            Diagnostic::ts6046(format!("--{}", option.name), option.keys, file_name.clone()).with_span(TextSpan {
                start: option.start,
                end: option.end,
            })
        })
        .collect()
}

/// tsc's `verifyCompilerOptions` checks of the JSX factory options' values:
/// `jsxFactory` and `jsxFragmentFactory` must parse as an entity name
/// (`ParseIsolatedEntityName`), and a `reactNamespace` no `jsxFactory`
/// overrides as an identifier (`IsIdentifierText`).
fn jsx_factory_option_diagnostics(loaded: &LoadedTsConfig) -> Vec<Diagnostic> {
    let options = &loaded.compiler_options;
    let set = |value: &Option<String>| value.clone().filter(|value| !value.is_empty());
    let file_name = loaded.config_path.display().to_string();
    let mut invalid = Vec::new();
    match set(&options.jsx_factory) {
        Some(factory) => {
            if surge_ts_syntax::jsx_entity_root(&factory).is_none() {
                invalid.push(("jsxFactory", Diagnostic::ts5067(&factory, file_name.clone())));
            }
        }
        None => {
            if let Some(namespace) = set(&options.react_namespace)
                && (namespace.contains('.') || surge_ts_syntax::jsx_entity_root(&namespace).is_none())
            {
                invalid.push(("reactNamespace", Diagnostic::ts5059(&namespace, file_name.clone())));
            }
        }
    }
    if let Some(fragment_factory) = set(&options.jsx_fragment_factory)
        && surge_ts_syntax::jsx_entity_root(&fragment_factory).is_none()
    {
        invalid.push((
            "jsxFragmentFactory",
            Diagnostic::ts18035(&fragment_factory, file_name.clone()),
        ));
    }
    invalid
        .into_iter()
        .map(|(option, diagnostic)| {
            match surge_ts_config::option_value_range(&loaded.config_path, option) {
                Some((start, end)) => diagnostic.with_span(TextSpan { start, end }),
                None => diagnostic,
            }
        })
        .collect()
}

fn referenced_lib_names(inputs: &[SourceFileInput]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for input in inputs {
        for name in surge_ts_checker::lowlevel::reference_lib_directives(&input.source_text) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn language_version(target: ScriptTarget) -> surge_ts_checker::LanguageVersion {
    use surge_ts_checker::LanguageVersion as Version;
    match target {
        ScriptTarget::ES2015 => Version::ES2015,
        ScriptTarget::ES2016 => Version::ES2016,
        ScriptTarget::ES2017 => Version::ES2017,
        ScriptTarget::ES2018 => Version::ES2018,
        ScriptTarget::ES2019 => Version::ES2019,
        ScriptTarget::ES2020 => Version::ES2020,
        ScriptTarget::ES2021 => Version::ES2021,
        ScriptTarget::ES2022 => Version::ES2022,
        ScriptTarget::ES2023 => Version::ES2023,
        ScriptTarget::ES2024 => Version::ES2024,
        ScriptTarget::ES2025 => Version::ES2025,
        ScriptTarget::ESNext => Version::ESNext,
    }
}

/// Map a configured `target` to the lib name base used to derive the default
/// `lib.<base>.full.d.ts` aggregate when `compilerOptions.lib` is unset.
fn target_lib_basename(target: ScriptTarget) -> &'static str {
    match target {
        ScriptTarget::ES2015 => "es2015",
        ScriptTarget::ES2016 => "es2016",
        ScriptTarget::ES2017 => "es2017",
        ScriptTarget::ES2018 => "es2018",
        ScriptTarget::ES2019 => "es2019",
        ScriptTarget::ES2020 => "es2020",
        ScriptTarget::ES2021 => "es2021",
        ScriptTarget::ES2022 => "es2022",
        ScriptTarget::ES2023 => "es2023",
        ScriptTarget::ES2024 => "es2024",
        ScriptTarget::ES2025 => "es2025",
        ScriptTarget::ESNext => "esnext",
    }
}

fn apply_project_no_lib_compatibility_diagnostics(
    diagnostics: Vec<Diagnostic>,
    no_lib: bool,
    provides_global_lib: bool,
    missing_global_types: &[&str],
    diagnostic_profile: DiagnosticProfile,
) -> Vec<Diagnostic> {
    if !no_lib || diagnostic_profile != DiagnosticProfile::Tsc {
        return diagnostics;
    }

    let mut filtered = diagnostics
        .into_iter()
        .filter(|diagnostic| !matches!(diagnostic.code, DiagnosticCode::TypeScript(2304)))
        .collect::<Vec<_>>();

    // tsc emits "Cannot find global type 'Array'/..." for each global type
    // its checker requires that the program does not declare itself (the
    // checker's `missing_global_types`). A project with explicit `typeRoots`
    // (e.g. roblox-ts's `@rbxts/types`) provides those globals itself, so tsc
    // reports none; surge treats them as present there rather than checking
    // each.
    if !provides_global_lib && !missing_global_types.is_empty() {
        // `GetDiagnosticsOfAnyProgram` asks for semantic diagnostics only when
        // the checker's construction reported no global diagnostic, so a
        // missing global type leaves every file's own diagnostics unreported.
        filtered.retain(|diagnostic| diagnostic.file_name.is_empty());
        filtered.extend(project_no_lib_missing_global_type_diagnostics(missing_global_types));
    }
    filtered
}

fn project_no_lib_missing_global_type_diagnostics(missing_global_types: &[&str]) -> Vec<Diagnostic> {
    missing_global_types
        .iter()
        .map(|global_type| {
            Diagnostic::new(
                DiagnosticCode::TypeScript(2318),
                format!("Cannot find global type '{global_type}'."),
                String::new(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(code: u32) -> Diagnostic {
        Diagnostic::new(
            DiagnosticCode::TypeScript(code),
            String::new(),
            String::new(),
        )
    }

    fn codes(diagnostics: &[Diagnostic]) -> Vec<u32> {
        diagnostics
            .iter()
            .filter_map(|d| match d.code {
                DiagnosticCode::TypeScript(code) => Some(code),
                _ => None,
            })
            .collect()
    }

    const ALL_MISSING: [&str; 10] = [
        "Array",
        "Boolean",
        "CallableFunction",
        "Function",
        "IArguments",
        "NewableFunction",
        "Number",
        "Object",
        "RegExp",
        "String",
    ];

    #[test]
    fn no_lib_without_global_lib_drops_2304_and_fabricates_2318() {
        let out = apply_project_no_lib_compatibility_diagnostics(
            vec![ts(2304), ts(2304), ts(2339)],
            true,
            false,
            &ALL_MISSING,
            DiagnosticProfile::Tsc,
        );
        let out = codes(&out);
        assert!(!out.contains(&2304));
        assert!(out.contains(&2339));
        assert_eq!(out.iter().filter(|&&c| c == 2318).count(), 10);
    }

    // roblox-ts and other custom-lib projects set explicit `typeRoots` to supply
    // the globals `noLib` removed; tsc reports no missing-global-type diagnostics,
    // so neither should we.
    #[test]
    fn no_lib_with_global_lib_drops_2304_and_fabricates_no_2318() {
        let out = apply_project_no_lib_compatibility_diagnostics(
            vec![ts(2304), ts(2339)],
            true,
            true,
            &ALL_MISSING,
            DiagnosticProfile::Tsc,
        );
        let out = codes(&out);
        assert!(!out.contains(&2304));
        assert!(out.contains(&2339));
        assert!(!out.contains(&2318));
    }

    #[test]
    fn lib_present_passes_diagnostics_through_untouched() {
        let out = apply_project_no_lib_compatibility_diagnostics(
            vec![ts(2304), ts(2339)],
            false,
            false,
            &[],
            DiagnosticProfile::Tsc,
        );
        assert_eq!(codes(&out), vec![2304, 2339]);
    }
}

fn checker_module_emit(kind: surge_ts_config::ModuleKind) -> surge_ts_checker::ModuleEmitKind {
    use surge_ts_checker::ModuleEmitKind as Emit;
    use surge_ts_config::ModuleKind as Module;
    match kind {
        Module::CommonJS => Emit::CommonJS,
        Module::ES2015 => Emit::ES2015,
        Module::ES2020 => Emit::ES2020,
        Module::ES2022 => Emit::ES2022,
        Module::ESNext => Emit::ESNext,
        Module::Node16 => Emit::Node16,
        Module::Node18 => Emit::Node18,
        Module::Node20 => Emit::Node20,
        Module::NodeNext => Emit::NodeNext,
        Module::Preserve => Emit::Preserve,
    }
}

/// The files whose implied module format under node16/nodenext resolution is
/// ESM: an `.mts`/`.mjs`, or a `.ts`/`.tsx`/`.js`/`.jsx` — declaration files
/// included — whose nearest `package.json` says `"type": "module"` (tsgo's
/// `GetImpliedNodeFormatForFile`).
fn esm_format_files<'a>(
    files: impl Iterator<Item = (&'a std::path::Path, &'a str)>,
) -> std::collections::HashSet<String> {
    let mut package_types: std::collections::HashMap<std::path::PathBuf, bool> =
        std::collections::HashMap::new();
    let mut is_module_package = |start: &std::path::Path| -> bool {
        let mut visited = Vec::new();
        let mut current = Some(start.to_path_buf());
        let mut answer = false;
        while let Some(dir) = current {
            if let Some(known) = package_types.get(&dir) {
                answer = *known;
                break;
            }
            visited.push(dir.clone());
            let manifest = dir.join("package.json");
            if let Ok(text) = host_fs::read_to_string(&manifest) {
                answer = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|json| json.get("type").and_then(|t| t.as_str()).map(|t| t == "module"))
                    .unwrap_or(false);
                break;
            }
            current = dir.parent().map(|parent| parent.to_path_buf());
        }
        for dir in visited {
            package_types.insert(dir, answer);
        }
        answer
    };
    let mut esm = std::collections::HashSet::new();
    for (path, name) in files {
        let lower = name.to_ascii_lowercase();
        let is_esm = if lower.ends_with(".mts") || lower.ends_with(".mjs") {
            true
        } else if lower.ends_with(".cts") || lower.ends_with(".cjs") {
            false
        } else if [".ts", ".tsx", ".js", ".jsx"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            path.parent().is_some_and(&mut is_module_package)
        } else {
            false
        };
        if is_esm {
            esm.insert(name.to_string());
        }
    }
    esm
}
