//! Program file discovery through the module resolver, as tsc's
//! `processImportedModules`: each loaded file's module names resolve through
//! [`ModuleResolver`], every answer is recorded for the checker, and a
//! resolved file the program takes in is loaded and scanned in turn.

use std::path::{Path, PathBuf};

use surge_ts_checker::SourceFileInput;
use surge_ts_types::fx::{FxHashMap, FxHashSet};

use crate::package_declarations::{
    ImportUsage, PackageDeclarationResolverCache, UsageResolutionMode, usage_resolution_mode,
};
use crate::package_resolution::ResolverOptions;
use crate::resolver::host::DiskHost;
use crate::resolver::tspath::{self, extension_is_ts};
use crate::resolver::{ModuleResolver, ResolutionMode, ResolvedModule};
use crate::specifier_scan::ModuleSpecifierScanner;
use surge_ts_config::canonicalize_if_exists_string;

/// The options that decide whether a resolved file joins the program
/// (`getResolutionDiagnostic` and the `allowJs` / `maxNodeModuleJsDepth`
/// gates).
pub(crate) struct InclusionOptions {
    pub(crate) allow_js: bool,
    pub(crate) jsx: bool,
    pub(crate) resolve_json_module: bool,
    pub(crate) allow_arbitrary_extensions: bool,
    pub(crate) max_node_module_js_depth: u32,
}

pub(crate) struct ModuleDiscovery {
    resolver: ModuleResolver<DiskHost>,
    inclusion: InclusionOptions,
    known_files: FxHashSet<String>,
    next_source_index: usize,
    augmentations_done: FxHashSet<usize>,
    /// How many `node_modules` searches each discovered source sits below a
    /// root (`increaseDepth`); a root is at 0.
    source_depths: FxHashMap<usize, u32>,
}

/// One recorded resolution: the importer, the key the checker reads it by
/// (the specifier, or its mode-override key), and the resolved file (empty
/// when the module did not resolve).
pub(crate) struct DiscoveredResolution {
    pub(crate) importer: String,
    pub(crate) specifier: String,
    pub(crate) usage: ImportUsage,
    pub(crate) resolved_file: String,
    pub(crate) using_ts_extension: bool,
}

fn resolver_mode(mode: UsageResolutionMode) -> ResolutionMode {
    match mode {
        UsageResolutionMode::None => ResolutionMode::None,
        UsageResolutionMode::CommonJs => ResolutionMode::CommonJs,
        UsageResolutionMode::Esm => ResolutionMode::Esm,
    }
}

impl ModuleDiscovery {
    pub(crate) fn new(resolver: ModuleResolver<DiskHost>, inclusion: InclusionOptions) -> Self {
        Self {
            resolver,
            inclusion,
            known_files: FxHashSet::default(),
            next_source_index: 0,
            augmentations_done: FxHashSet::default(),
            source_depths: FxHashMap::default(),
        }
    }

    /// Whether tsc adds a resolved file to the program: TypeScript and
    /// declaration files always (`.tsx` with `jsx` set), JSON under
    /// `resolveJsonModule`, JavaScript under `allowJs` unless a `node_modules`
    /// search found it deeper than `maxNodeModuleJsDepth`, and an arbitrary
    /// extension's declaration from a declaration file or under
    /// `allowArbitraryExtensions`.
    fn joins_program(&self, resolved: &ResolvedModule, importer_is_declaration: bool, depth: u32) -> bool {
        let javascript_within_depth = || {
            !(resolved.is_external_library_import
                && resolved.resolved_file_name.contains("/node_modules/")
                && depth > self.inclusion.max_node_module_js_depth)
        };
        let extension = resolved.extension.as_str();
        match extension {
            tspath::EXTENSION_TS
            | tspath::EXTENSION_DTS
            | tspath::EXTENSION_MTS
            | tspath::EXTENSION_DMTS
            | tspath::EXTENSION_CTS
            | tspath::EXTENSION_DCTS => true,
            tspath::EXTENSION_TSX => self.inclusion.jsx,
            tspath::EXTENSION_JSON => self.inclusion.resolve_json_module,
            tspath::EXTENSION_JS | tspath::EXTENSION_MJS | tspath::EXTENSION_CJS => {
                self.inclusion.allow_js && javascript_within_depth()
            }
            tspath::EXTENSION_JSX => self.inclusion.jsx && self.inclusion.allow_js && javascript_within_depth(),
            other => {
                extension_is_ts(other) && (importer_is_declaration || self.inclusion.allow_arbitrary_extensions)
            }
        }
    }

    /// Resolves the module names of every source not yet scanned, loading the
    /// files they bring in, until no new file appears. Returns the number of
    /// files added and every resolution made.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expand(
        &mut self,
        scanner: &mut ModuleSpecifierScanner,
        inputs: &mut Vec<SourceFileInput>,
        sources: &mut Vec<(PathBuf, String, String)>,
        mode_options: &ResolverOptions,
        mode_cache: &mut PackageDeclarationResolverCache,
    ) -> (usize, Vec<DiscoveredResolution>) {
        for input in inputs.iter() {
            self.known_files.insert(canonicalize_if_exists_string(Path::new(&input.file_name)));
        }
        let mut added = 0usize;
        let mut resolutions = Vec::new();
        while self.next_source_index < sources.len() {
            let wave_end = sources.len();
            scanner.prefetch(sources, self.next_source_index);
            let mut discovered: Vec<(PathBuf, String, u32)> = Vec::new();
            while self.next_source_index < wave_end {
                let index = self.next_source_index;
                self.next_source_index += 1;
                let (file_path, file_name, source_text) = {
                    let (file_path, file_name, source_text) = &sources[index];
                    (file_path.clone(), file_name.clone(), source_text.clone())
                };
                let importer = canonicalize_if_exists_string(Path::new(&file_name));
                let importer_depth = self.source_depths.get(&index).copied().unwrap_or(0);
                let importer_is_declaration = surge_ts_syntax::is_declaration_file_name(&file_name);
                let mut usages: Vec<(String, ImportUsage)> = scanner
                    .usages(index, &file_name, &source_text)
                    .iter()
                    .map(|usage| {
                        let kind = if let Some(mode) = usage.resolution_mode {
                            ImportUsage::ModeOverride(mode)
                        } else if usage.import_equals {
                            ImportUsage::ImportEquals
                        } else {
                            match usage.call {
                                Some(surge_ts_syntax::ParsedImportCallKind::Expression) => ImportUsage::DynamicImport,
                                Some(surge_ts_syntax::ParsedImportCallKind::Require) => ImportUsage::Require,
                                _ => ImportUsage::Declaration,
                            }
                        };
                        (usage.specifier.clone(), kind)
                    })
                    .collect();
                if self.augmentations_done.insert(index)
                    && let Some((_, names)) = scanner.augmentation_specifiers().find(|(scanned, _)| *scanned == index)
                {
                    let names: Vec<String> = names.to_vec();
                    usages.extend(names.into_iter().map(|name| (name, ImportUsage::Declaration)));
                }
                let mut seen = FxHashSet::default();
                for (specifier, usage) in usages {
                    if !seen.insert((specifier.clone(), usage)) {
                        continue;
                    }
                    let mode = resolver_mode(usage_resolution_mode(&file_path, usage, mode_options, mode_cache));
                    let resolved = self.resolver.resolve_module_name(&specifier, &importer, mode);
                    let resolved_file = if resolved.is_resolved() {
                        canonicalize_if_exists_string(Path::new(&resolved.resolved_file_name))
                    } else {
                        String::new()
                    };
                    let depth = importer_depth + u32::from(resolved.is_external_library_import);
                    if !resolved_file.is_empty()
                        && self.joins_program(&resolved, importer_is_declaration, depth)
                        && self.known_files.insert(resolved_file.clone())
                    {
                        discovered.push((PathBuf::from(&resolved_file), resolved_file.clone(), depth));
                    }
                    resolutions.push(DiscoveredResolution {
                        importer: importer.clone(),
                        specifier,
                        usage,
                        resolved_file,
                        using_ts_extension: resolved.resolved_using_ts_extension,
                    });
                }
            }
            for (path, file_name, depth) in discovered {
                let read_start = std::time::Instant::now();
                let Ok(source_text) = crate::host_fs::read_to_string(&path) else {
                    continue;
                };
                self.source_depths.insert(sources.len(), depth);
                crate::io_stats::record_package_declaration_read(source_text.len(), read_start.elapsed());
                inputs.push(SourceFileInput { file_name: file_name.clone(), source_text: source_text.clone() });
                sources.push((path, file_name, source_text));
                added += 1;
            }
        }
        (added, resolutions)
    }
}

/// Records discovered resolutions the way the checker reads them: each
/// importer's own answers (a usage that picks its mode under a key of its
/// own), and the first answer per specifier as the project-wide fallback.
pub(crate) fn record_resolutions(
    resolutions: Vec<DiscoveredResolution>,
    resolved_modules: &mut FxHashMap<String, String>,
    by_importer: &mut FxHashMap<String, FxHashMap<String, String>>,
    using_ts_extension: &mut FxHashSet<(String, String)>,
) {
    use surge_ts_checker::lowlevel::resolution_candidates::resolution_mode_override_key;
    for resolution in resolutions {
        if resolution.using_ts_extension && !resolution.resolved_file.is_empty() {
            using_ts_extension.insert((resolution.importer.clone(), resolution.specifier.clone()));
        }
        if !resolution.resolved_file.is_empty() && !matches!(resolution.usage, ImportUsage::ModeOverride(_)) {
            resolved_modules
                .entry(resolution.specifier.clone())
                .or_insert_with(|| resolution.resolved_file.clone());
        }
        let per_importer = by_importer.entry(resolution.importer).or_default();
        match resolution.usage {
            ImportUsage::Declaration => {
                per_importer.insert(resolution.specifier, resolution.resolved_file);
            }
            ImportUsage::ImportEquals | ImportUsage::Require => {
                per_importer.insert(
                    resolution_mode_override_key(
                        &resolution.specifier,
                        surge_ts_syntax::ResolutionModeOverride::Require,
                    ),
                    resolution.resolved_file.clone(),
                );
                per_importer.entry(resolution.specifier).or_insert(resolution.resolved_file);
            }
            // The checker reads a dynamic import by the mode it names.
            ImportUsage::DynamicImport => {
                per_importer.insert(
                    resolution_mode_override_key(&resolution.specifier, surge_ts_syntax::ResolutionModeOverride::Import),
                    resolution.resolved_file,
                );
            }
            ImportUsage::ModeOverride(mode) => {
                per_importer.insert(resolution_mode_override_key(&resolution.specifier, mode), resolution.resolved_file);
            }
        }
    }
}
