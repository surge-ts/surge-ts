use std::sync::Arc;

use surge_ts_syntax::{
    JsxRuntimeOptions, ParsedExportDeclaration, ParsedImportKind, ParsedResolutionModeAttribute,
    ParsedSource, ParsedStatement, ParserWorker, ResolutionModeOverride,
};

/// One scanned source: the specifiers both fixpoint scanners ask for, and the
/// parse they came from. The parse is kept because the checker would otherwise
/// re-parse the identical `(text, file_name)` pair from scratch —
/// [`ModuleSpecifierScanner::take_parsed_sources`] hands it over instead.
struct ScannedSource {
    specifiers: Arc<[String]>,
    augmentation_specifiers: Box<[String]>,
    usages: Arc<[ModuleUsage]>,
    parsed: Option<ParsedSource>,
}

impl ScannedSource {
    fn new(
        parsed: ParsedSource,
        retain_parse: bool,
        jsx_runtime: &JsxRuntimeOptions,
        import_helpers: ImportHelpersScan,
    ) -> ScannedSource {
        // tsc's file loader resolves the `tslib` a file's emit helpers come
        // from and the automatic JSX runtime a file imports implicitly, in
        // that order, before the imports it writes.
        let runtime_import =
            surge_ts_syntax::jsx_runtime_import(&parsed.file_name, &parsed.jsx_factory_uses, jsx_runtime);
        let implicit_imports: Vec<&str> = import_helpers
            .helpers_import(&parsed)
            .into_iter()
            .chain(runtime_import.as_deref())
            .collect();
        ScannedSource {
            specifiers: source_specifiers(&parsed, &implicit_imports),
            usages: source_usages(&parsed, &implicit_imports),
            augmentation_specifiers: module_augmentation_specifiers(&parsed),
            parsed: retain_parse.then_some(parsed),
        }
    }
}

/// tsgo's `needsImportHelpersImportSpecifier`: under `importHelpers`, a
/// JavaScript file, and a non-declaration file that is a module (any, under
/// `isolatedModules`), imports `tslib` for the helpers its emit calls.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImportHelpersScan {
    pub(crate) enabled: bool,
    /// tsgo's `GetIsolatedModules`: `isolatedModules` or `verbatimModuleSyntax`.
    pub(crate) isolated_modules: bool,
    /// `moduleDetection: force`, which makes every non-declaration file a module.
    pub(crate) force_module_detection: bool,
}

impl ImportHelpersScan {
    fn helpers_import(self, parsed: &ParsedSource) -> Option<&'static str> {
        let file_name = parsed.file_name.as_str();
        if !self.enabled || surge_ts_syntax::is_json_file_name(file_name) {
            return None;
        }
        let lower = file_name.to_ascii_lowercase();
        let module = parsed.is_module
            || self.force_module_detection
            || lower.ends_with(".mts")
            || lower.ends_with(".cts");
        let needed = surge_ts_syntax::is_javascript_file_name(file_name)
            || (!surge_ts_syntax::is_declaration_file_name(file_name) && (self.isolated_modules || module));
        needed.then_some("tslib")
    }
}

/// One written module specifier and the syntax it is written in, which
/// decides the mode tsc resolves it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleUsage {
    pub specifier: String,
    /// `import x = require("...")`.
    pub import_equals: bool,
    /// A `resolution-mode` attribute that decides the mode.
    pub resolution_mode: Option<ResolutionModeOverride>,
    /// A dynamic `import("...")` or a JavaScript `require("...")`, which pick
    /// their own mode.
    pub call: Option<surge_ts_syntax::ParsedImportCallKind>,
}

/// Shared module-specifier extraction for the loader's fixpoint scanners.
///
/// The package-declaration and import-graph scanners both need the module
/// specifiers of every source, and the loader loop used to make each of them
/// re-parse every file on every fixpoint iteration. This cache parses each
/// source exactly once (keyed by its append-only index in `sources`) and hands
/// both scanners the same extracted specifier list.
pub(crate) struct ModuleSpecifierScanner {
    parser: ParserWorker,
    scanned: Vec<Option<ScannedSource>>,
    retain_parses: bool,
    jsx_runtime: JsxRuntimeOptions,
    import_helpers: ImportHelpersScan,
}

/// `SURGE_PRESCANNED_PARSE_REUSE=0` drops each parse as soon as its specifiers
/// are extracted, which puts the checker back on its own parse of every file.
/// Retention and reuse move together so the A/B arms differ in nothing else.
fn parse_reuse_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_PRESCANNED_PARSE_REUSE").as_deref() != Ok("0"))
}

impl ModuleSpecifierScanner {
    pub(crate) fn new(jsx_runtime: JsxRuntimeOptions) -> Self {
        Self {
            parser: ParserWorker::new(),
            scanned: Vec::new(),
            retain_parses: parse_reuse_enabled(),
            jsx_runtime,
            import_helpers: ImportHelpersScan::default(),
        }
    }

    pub(crate) fn with_import_helpers(mut self, import_helpers: ImportHelpersScan) -> Self {
        self.import_helpers = import_helpers;
        self
    }

    /// Parse `sources[start..]` on a small worker pool and fill the cache, so
    /// the scanners' serial BFS resolution only pays lookup cost. Extraction is
    /// pure per file (one arena per worker thread, results index-ordered), so
    /// this changes no observable ordering. The loader's source reads are
    /// already unconditionally parallel, so this follows the same contract.
    pub(crate) fn prefetch(
        &mut self,
        sources: &[(std::path::PathBuf, String, String)],
        start: usize,
    ) {
        let pending: Vec<usize> = (start..sources.len())
            .filter(|&index| self.scanned.get(index).is_none_or(|slot| slot.is_none()))
            .collect();
        if pending.len() < 32 {
            return;
        }
        if self.scanned.len() < sources.len() {
            self.scanned.resize_with(sources.len(), || None);
        }
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(pending.len());
        if workers <= 1 {
            return;
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let retain_parses = self.retain_parses;
        let jsx_runtime = &self.jsx_runtime;
        let import_helpers = self.import_helpers;
        let results: Vec<(usize, ScannedSource)> = std::thread::scope(|scope| {
            let pending = &pending;
            let next = &next;
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                handles.push(scope.spawn(move || {
                    let mut parser = ParserWorker::new();
                    let mut out = Vec::new();
                    loop {
                        let slot = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if slot >= pending.len() {
                            break;
                        }
                        let index = pending[slot];
                        let (_, file_name, source_text) = &sources[index];
                        let parsed = parser.parse(source_text, file_name);
                        out.push((index, ScannedSource::new(parsed, retain_parses, jsx_runtime, import_helpers)));
                    }
                    out
                }));
            }
            handles
                .into_iter()
                .flat_map(|handle| handle.join().expect("specifier scan worker panicked"))
                .collect()
        });
        for (index, scanned) in results {
            self.scanned[index] = Some(scanned);
        }
    }

    /// Module specifiers of `sources[index]`, parsing on first request only.
    /// `index` must be the file's position in the loader's append-only
    /// `sources` vector so repeated requests hit the cache.
    pub(crate) fn specifiers(
        &mut self,
        index: usize,
        file_name: &str,
        source_text: &str,
    ) -> Arc<[String]> {
        self.scanned_source(index, file_name, source_text)
            .specifiers
            .clone()
    }

    /// Like [`Self::specifiers`], with the syntax each specifier is written in.
    pub(crate) fn usages(
        &mut self,
        index: usize,
        file_name: &str,
        source_text: &str,
    ) -> Arc<[ModuleUsage]> {
        self.scanned_source(index, file_name, source_text)
            .usages
            .clone()
    }

    fn scanned_source(&mut self, index: usize, file_name: &str, source_text: &str) -> &ScannedSource {
        if self.scanned.len() <= index {
            self.scanned.resize_with(index + 1, || None);
        }
        if self.scanned[index].is_none() {
            let parsed = self.parser.parse(source_text, file_name);
            self.scanned[index] = Some(ScannedSource::new(parsed, self.retain_parses, &self.jsx_runtime, self.import_helpers));
        }
        self.scanned[index]
            .as_ref()
            .expect("scanned source was just filled")
    }

    /// `(sources index, names)` of every scanned module file's top-level
    /// `declare module "name"` augmentations.
    pub(crate) fn augmentation_specifiers(&self) -> impl Iterator<Item = (usize, &[String])> {
        self.scanned.iter().enumerate().filter_map(|(index, slot)| {
            let names = &slot.as_ref()?.augmentation_specifiers;
            (!names.is_empty()).then_some((index, &names[..]))
        })
    }

    /// Hand the scanned parses to the checker, which would otherwise parse the
    /// identical `(source text, file name)` pairs a second time. Every file the
    /// loader scanned is in the program, so nothing is dropped on the floor;
    /// default libs never enter the scan and stay the checker's to parse.
    pub(crate) fn take_parsed_sources(&mut self) -> Vec<ParsedSource> {
        self.scanned
            .iter_mut()
            .filter_map(|slot| slot.as_mut().and_then(|scanned| scanned.parsed.take()))
            .collect()
    }
}

/// Declaration specifiers first, then the `import("...")` forms the lossy
/// `Parsed*` tree cannot carry. Both belong to the module graph: a package
/// reached only through an import type still supplies its
/// `/// <reference types>` directives and ambient `declare module` blocks.
fn source_specifiers(parsed: &ParsedSource, implicit_imports: &[&str]) -> Arc<[String]> {
    implicit_imports
        .iter()
        .copied()
        .chain(parsed.statements.iter().filter_map(statement_module_specifier))
        .chain(parsed.import_call_specifiers.iter().map(String::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into()
}

/// Kept apart from [`source_specifiers`]: tsc resolves an augmentation's name
/// but never loads a file for it, so these must not grow the program.
fn module_augmentation_specifiers(parsed: &ParsedSource) -> Box<[String]> {
    if !parsed.is_module {
        return Box::default();
    }
    parsed
        .statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::DeclareModuleDeclaration(module)
                if module.module_specifier != "global" =>
            {
                Some(module.module_specifier.clone())
            }
            _ => None,
        })
        .collect()
}

/// The usages behind [`source_specifiers`], in the same order.
fn source_usages(parsed: &ParsedSource, implicit_imports: &[&str]) -> Arc<[ModuleUsage]> {
    implicit_imports
        .iter()
        .map(|specifier| ModuleUsage {
            specifier: (*specifier).to_owned(),
            import_equals: false,
            resolution_mode: None,
            call: None,
        })
        .chain(parsed.statements.iter().filter_map(|statement| {
            let specifier = statement_module_specifier(statement)?;
            let (import_equals, attribute) = match statement {
                ParsedStatement::ImportDeclaration(import) => (
                    matches!(import.kind, ParsedImportKind::Equals { .. }),
                    import.resolution_mode,
                ),
                ParsedStatement::ExportDeclaration(export) => match &**export {
                    ParsedExportDeclaration::Named {
                        resolution_mode, ..
                    }
                    | ParsedExportDeclaration::All {
                        resolution_mode, ..
                    } => (false, *resolution_mode),
                    _ => (false, None),
                },
                _ => (false, None),
            };
            Some(ModuleUsage {
                specifier: specifier.to_owned(),
                import_equals,
                resolution_mode: ParsedResolutionModeAttribute::resolution_override(attribute),
                call: None,
            })
        }))
        .chain(ambient_module_usages(parsed))
        .chain(parsed.import_calls.iter().map(|call| ModuleUsage {
            specifier: call.specifier.clone(),
            import_equals: false,
            resolution_mode: None,
            call: Some(call.kind),
        }))
        .chain(
            parsed
                .import_call_specifiers
                .iter()
                .filter(|specifier| !parsed.import_calls.iter().any(|call| call.specifier == **specifier))
                .map(|specifier| ModuleUsage {
                    specifier: specifier.clone(),
                    import_equals: false,
                    resolution_mode: None,
                    call: None,
                }),
        )
        .filter(|usage| !usage.specifier.is_empty())
        .collect::<Vec<_>>()
        .into()
}

/// tsc's `collectModuleReferences` inside the ambient modules a global file
/// declares: their non-relative imports and re-exports are the file's too.
fn ambient_module_usages(parsed: &ParsedSource) -> Vec<ModuleUsage> {
    if parsed.is_module {
        return Vec::new();
    }
    parsed
        .statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::DeclareModuleDeclaration(module) if module.module_specifier != "global" => {
                Some(module.statements.iter())
            }
            _ => None,
        })
        .flatten()
        .filter_map(|statement| {
            let specifier = statement_module_specifier(statement)?;
            if specifier.starts_with("./") || specifier.starts_with("../") || specifier == "." || specifier == ".." || specifier.starts_with('/') {
                return None;
            }
            Some(ModuleUsage {
                specifier: specifier.to_owned(),
                import_equals: matches!(statement, ParsedStatement::ImportDeclaration(import) if matches!(import.kind, ParsedImportKind::Equals { .. })),
                resolution_mode: None,
                call: None,
            })
        })
        .collect()
}

fn statement_module_specifier(statement: &ParsedStatement) -> Option<&str> {
    match statement {
        ParsedStatement::ImportDeclaration(import) => Some(&import.module_specifier),
        ParsedStatement::ExportDeclaration(export) => match &**export {
            ParsedExportDeclaration::Named {
                module_specifier: Some(module_specifier),
                ..
            }
            | ParsedExportDeclaration::All {
                module_specifier, ..
            }
            | ParsedExportDeclaration::Namespace {
                module_specifier, ..
            } => Some(module_specifier),
            _ => None,
        },
        _ => None,
    }
}
