//! Module type/import binding collection across the multi-pass fixpoint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_types::Type;

use super::*;

use crate::context::{CheckerContext, FileKind};
use crate::driver::{collect_type_declarations, lower_global_augmentation_values_from_statements};
use crate::modules::{
    ModuleExportTable, ModuleImportBindings, build_module_export_table,
    resolve_module_export_tables, resolve_module_imports,
};
use crate::symbols::{
    SymbolTable, TypeDeclarationInfo, TypeDeclarationScope, TypeDeclarationTable,
};

pub(crate) fn collect_preliminary_module_type_bindings(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> (
    Vec<Option<Arc<TypeDeclarationTable>>>,
    Vec<Option<ModuleImportBindings>>,
    Vec<Diagnostic>,
) {
    let mut local_type_declarations_by_module = Vec::with_capacity(parsed_files.len());
    let mut preliminary_local_export_tables = Vec::with_capacity(parsed_files.len());
    let mut preliminary_type_diagnostics = Vec::new();
    let initial_diagnostics_len = ctx.diagnostics().len();
    let has_file_keyed_augmentations = has_file_keyed_module_augmentations(ctx);

    for parsed_file in parsed_files {
        if !parsed_file.is_module && parsed_file.file_kind != FileKind::DependencyDeclaration {
            local_type_declarations_by_module.push(None);
            preliminary_local_export_tables.push(None);
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
        ctx.type_declaration_scope = None;

        let diagnostics_before_collect = ctx.diagnostics().len();
        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
        let saved_symbols = std::mem::replace(&mut ctx.symbols, SymbolTable::new());
        let collect_start = Instant::now();
        collect_type_declarations(&parsed_file.statements, ctx);
        let mut raw_local_type_declarations = std::mem::take(&mut ctx.type_declarations);
        if has_file_keyed_augmentations {
            let file_identity = crate::modules::canonical_file_identity(&parsed_file.file_name);
            merge_file_keyed_module_augmentation_into_declarations(
                &mut raw_local_type_declarations,
                &file_identity,
                &parsed_file.statements,
                ctx,
            );
        }
        let raw_local_type_declarations = Arc::new(raw_local_type_declarations);
        let preliminary_raw_scope = Arc::new(
            TypeDeclarationScope::new(vec![raw_local_type_declarations.clone()]).preliminary(),
        );
        let local_type_declarations = Arc::new(attach_resolution_scope_to_declarations(
            raw_local_type_declarations.as_ref(),
            preliminary_raw_scope,
        ));
        let lowered_type_declarations = local_type_declarations.len() as u64;
        let collect_duration = collect_start.elapsed();
        preliminary_type_diagnostics.extend(
            ctx.diagnostics()[diagnostics_before_collect..]
                .iter()
                .cloned(),
        );
        record_program_file_timing(timings, &parsed_file.file_name, |metrics| {
            metrics.collect_type_declarations_passes += 1;
            metrics.lowered_type_declarations += lowered_type_declarations;
            metrics.collect_type_declarations_duration += collect_duration;
        });

        let preliminary_resolution_scope = Arc::new(
            TypeDeclarationScope::new(vec![local_type_declarations.clone()]).preliminary(),
        );
        ctx.thin_superseded_value_collection = true;
        let preliminary_export_table = build_module_export_table(
            parsed_file,
            local_type_declarations.as_ref(),
            &SymbolTable::new(), // empty symbols
            &SymbolTable::new(), // imports not resolved yet in the preliminary pass
            Some(preliminary_resolution_scope),
            ctx,
        );
        ctx.thin_superseded_value_collection = false;

        ctx.type_declarations = saved_type_declarations;
        ctx.symbols = saved_symbols;
        ctx.type_declaration_scope = saved_type_declaration_scope;

        local_type_declarations_by_module.push(Some(local_type_declarations));
        preliminary_local_export_tables.push(Some(preliminary_export_table));
    }

    let preliminary_export_resolution_start = Instant::now();
    let preliminary_module_export_tables =
        resolve_module_export_tables(parsed_files, &preliminary_local_export_tables, ctx);
    record_program_timing(timings, |timings| {
        timings.preliminary_export_table_resolution += preliminary_export_resolution_start.elapsed()
    });

    let preliminary_scope_start = Instant::now();
    let preliminary_module_resolution_scopes = local_type_declarations_by_module
        .iter()
        .map(|local_type_declarations| {
            let Some(local_type_declarations) = local_type_declarations else {
                return None;
            };

            Some(Arc::new(
                TypeDeclarationScope::new(vec![local_type_declarations.clone()]).preliminary(),
            ))
        })
        .collect::<Vec<_>>();
    record_program_timing(timings, |timings| {
        timings.module_resolution_scope_construction += preliminary_scope_start.elapsed()
    });

    let mut preliminary_module_import_bindings = Vec::with_capacity(parsed_files.len());
    let preliminary_import_resolution_start = Instant::now();
    for (_file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !parsed_file.is_module && parsed_file.file_kind != FileKind::DependencyDeclaration {
            if !parsed_file.import_call_specifiers.is_empty() {
                ctx.set_file_name(parsed_file.file_name.clone());
                crate::modules::register_import_type_namespaces(
                    parsed_file,
                    parsed_files,
                    &preliminary_module_export_tables,
                    &preliminary_module_resolution_scopes,
                    ctx,
                );
            }
            preliminary_module_import_bindings.push(None);
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());
        let imported_bindings = resolve_module_imports(
            parsed_file,
            parsed_files,
            &preliminary_module_export_tables,
            &preliminary_module_resolution_scopes,
            &|_| false,
            ctx,
        );
        preliminary_module_import_bindings.push(Some(imported_bindings));
    }
    record_program_timing(timings, |timings| {
        timings.import_binding_resolution += preliminary_import_resolution_start.elapsed()
    });

    let replayable_preliminary_diagnostics = ctx.diagnostics()[initial_diagnostics_len..]
        .iter()
        .filter(|diagnostic| should_replay_preliminary_diagnostic(diagnostic, ctx))
        .cloned()
        .collect::<Vec<_>>();
    ctx.truncate_diagnostics(initial_diagnostics_len);
    preliminary_type_diagnostics.extend(replayable_preliminary_diagnostics);

    (
        local_type_declarations_by_module,
        preliminary_module_import_bindings,
        preliminary_type_diagnostics,
    )
}

fn upgrade_analysis_scopes_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_UPGRADE_ANALYSIS_SCOPES").as_deref() == Ok("1"))
}

fn declaration_resolution_scope(
    declaration: &TypeDeclarationInfo,
) -> Option<&Arc<TypeDeclarationScope>> {
    match declaration {
        TypeDeclarationInfo::Alias(alias) => alias.resolution_scope.as_ref(),
        TypeDeclarationInfo::Interface(interface) => interface.resolution_scope.as_ref(),
    }
}

/// Opt-in (`SURGE_UPGRADE_ANALYSIS_SCOPES=1`): replaces the import-less scope
/// `collect_preliminary_module_type_bindings` attaches at collection time with
/// the module's real one (own declarations + import layers).
///
/// A declaration keeps its attached scope wherever it travels, so an alias
/// exported with the preliminary scope resolves every *imported* name in its
/// body to `unknown` — and for a consumer that is the published type of
/// anything built from it. The check phase recovers through
/// `module_scope_by_file`; module analysis runs map-less by design, so the
/// degraded shape is what the export table carries. tRPC's
/// `createTRPCNext<AppRouter>` is this shape: the export is `unknown` for every
/// page that imports it, which silences the 12 `TS2339`s tsc reports there.
///
/// Off by default because the honest export is not yet an improvement: with it
/// on, a router surge cannot model decides `ProtectedIntersection`'s
/// `keyof A & keyof B extends never` from a key set it never had, and the
/// string-literal error type that conditional produces swallows every property
/// read off the router — 12 `TS2339` closed, 13 opened, in the examples where
/// tsc models the router and surge does not. Turning it on for good needs
/// degraded-argument provenance the substitution does not carry today.
fn upgrade_preliminary_resolution_scopes(
    declarations: &TypeDeclarationTable,
    scope: &Arc<TypeDeclarationScope>,
) -> Option<TypeDeclarationTable> {
    let needs_upgrade = declarations.iter().any(|(_, declaration)| {
        declaration_resolution_scope(declaration)
            .is_none_or(|attached| attached.is_preliminary())
    });
    if !needs_upgrade {
        return None;
    }

    let mut upgraded = TypeDeclarationTable::new();
    for (name, declaration) in declarations.iter() {
        let declaration = match declaration_resolution_scope(declaration)
            .is_none_or(|attached| attached.is_preliminary())
        {
            false => declaration.clone(),
            true => match declaration.clone() {
                TypeDeclarationInfo::Alias(mut alias) => {
                    alias.resolution_scope = Some(scope.clone());
                    TypeDeclarationInfo::Alias(alias)
                }
                TypeDeclarationInfo::Interface(mut interface) => {
                    interface.resolution_scope = Some(scope.clone());
                    TypeDeclarationInfo::Interface(interface)
                }
            },
        };
        let _ = upgraded.insert(name.as_ref(), declaration);
    }
    Some(upgraded)
}

pub(crate) fn attach_resolution_scope_to_declarations(
    declarations: &TypeDeclarationTable,
    scope: Arc<TypeDeclarationScope>,
) -> TypeDeclarationTable {
    let mut attached = TypeDeclarationTable::new();
    for (name, declaration) in declarations.iter() {
        let declaration = match declaration.clone() {
            TypeDeclarationInfo::Alias(mut alias) => {
                alias.resolution_scope = Some(scope.clone());
                TypeDeclarationInfo::Alias(alias)
            }
            TypeDeclarationInfo::Interface(mut interface) => {
                interface.resolution_scope = Some(scope.clone());
                TypeDeclarationInfo::Interface(interface)
            }
        };
        let _ = attached.insert(name.as_ref(), declaration);
    }
    attached
}

/// Opt-in per-module memory trace (`SURGE_TRACE_MODULE_MEMORY=1`, or a byte
/// threshold instead of `1`): emits one JSON line per module whose analysis
/// raised the macOS physical-footprint high-water mark by at least the threshold
/// (default 8MB; RSS is used on platforms without `phys_footprint`),
/// identifying the module active at the high-water mark. Round numbers
/// distinguish the preliminary (1) and final (2) analysis passes.
fn module_memory_trace_threshold() -> Option<u64> {
    static THRESHOLD: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *THRESHOLD.get_or_init(|| {
        let value = std::env::var("SURGE_TRACE_MODULE_MEMORY").ok()?;
        Some(
            value
                .parse::<u64>()
                .ok()
                .filter(|&threshold| threshold > 1)
                .unwrap_or(8 * 1024 * 1024),
        )
    })
}

fn next_analysis_round() -> u64 {
    static ROUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ROUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Analyzes one module: seeds its value/signature environment, collects
/// function signatures, and builds its export table. Extracted from the serial
/// loop so the parallel driver runs the identical body per worker; the caller
/// owns declaration-file type dedup (its shared cache picks pointer-identity
/// representatives, which must be chosen in deterministic file order) and the
/// ordered `analyses` placement.
#[allow(clippy::too_many_arguments)]
/// Opt-in `SURGE_ANALYZE_SPLIT=1` probe: cumulative micros per
/// analyze_module sub-phase (plus `build_module_export_table`'s internal
/// split), printed after each analysis round. Zero-cost when unset. This is
/// how the thin-preliminary win was found — value collection via the export
/// table, not signature collection, dominated the analysis rounds.
static ANALYZE_SPLIT: [std::sync::atomic::AtomicU64; 5] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];

pub(crate) fn analyze_split_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_ANALYZE_SPLIT").is_some())
}

pub(crate) fn analyze_split_record(slot: usize, start: Option<Instant>) {
    if let Some(start) = start {
        ANALYZE_SPLIT[slot].fetch_add(
            start.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

fn analyze_split_report(round: u64) {
    if analyze_split_enabled() {
        use std::sync::atomic::Ordering::Relaxed;
        eprintln!(
            "[analyze-split] round={round} values={:.3}s signatures={:.3}s export_table={:.3}s \
             (et_values={:.3}s et_statements={:.3}s)",
            ANALYZE_SPLIT[0].swap(0, Relaxed) as f64 / 1e6,
            ANALYZE_SPLIT[1].swap(0, Relaxed) as f64 / 1e6,
            ANALYZE_SPLIT[2].swap(0, Relaxed) as f64 / 1e6,
            ANALYZE_SPLIT[3].swap(0, Relaxed) as f64 / 1e6,
            ANALYZE_SPLIT[4].swap(0, Relaxed) as f64 / 1e6,
        );
    }
}

fn analyze_module(
    file_index: usize,
    parsed_file: &ParsedProgramFile,
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    preliminary_module_import_bindings: &[Option<ModuleImportBindings>],
    lower_global_augmentation_values: bool,
    thin_value_collection: bool,
    values_resolve_through_module_scopes: bool,
    analysis_round: u64,
    memory_trace_threshold: Option<u64>,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> Option<ModuleAnalysis> {
    let eq_probe_start = super::eq_probe_enabled().then(Instant::now);
    let module_time_start = super::module_time_dump_enabled().then(Instant::now);
    let degraded_before = super::degraded_resolution_count();
    let memory_before = memory_trace_threshold.map(|_| {
        (
            Instant::now(),
            crate::metrics::rss::current_rss_bytes().unwrap_or(0),
            crate::metrics::rss::peak_rss_bytes().unwrap_or(0),
            crate::metrics::rss::current_footprint_bytes(),
            crate::metrics::rss::peak_footprint_bytes(),
        )
    });

    record_program_counter(|c| {
        c.module_analysis_total_calls += 1;
        c.module_analysis_unique_files += 1;
    });

    ctx.set_file_name(parsed_file.file_name.clone());
    ctx.jsx_factory_uses = parsed_file.jsx_factory_uses.clone();
    ctx.jsx_namespace = None;
    // The named-resolution memo is module-scoped, like `begin_file_check` makes
    // it file-scoped for the check phase. Without this reset the module's early
    // phase (value collection / signature seeding) reads entries left by
    // whichever module happened to be analyzed just before it — a
    // schedule-dependent carry-over that leaks another module's resolution
    // context (observed as a type-parameter name flipping in one zod display)
    // and would make parallel analysis diverge from serial.
    ctx.replace_resolved_named_types(0);
    let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
    ctx.type_declaration_scope = None;
    let Some(local_type_declarations) = local_type_declarations_by_module[file_index].as_ref()
    else {
        ctx.type_declaration_scope = saved_type_declaration_scope;
        return None;
    };

    // Set up the full type environment for function signature collection
    let merge_start = Instant::now();
    let imported_layers = preliminary_module_import_bindings[file_index]
        .as_ref()
        .map(|bindings| bindings.scope_layers());
    let mut scope_layers = vec![local_type_declarations.clone()];
    if let Some(imported_layers) = imported_layers {
        scope_layers.extend(imported_layers);
    }
    let full_type_declarations_scope = Arc::new(TypeDeclarationScope::new(scope_layers));
    let local_type_declarations = crate::driver::with_namespace_reexports(
        local_type_declarations.clone(),
        &parsed_file.statements,
        &full_type_declarations_scope,
    );
    let local_type_declarations = match upgrade_analysis_scopes_enabled()
        .then(|| {
            upgrade_preliminary_resolution_scopes(
                &local_type_declarations,
                &full_type_declarations_scope,
            )
        })
        .flatten()
    {
        Some(upgraded) => Arc::new(upgraded),
        None => local_type_declarations,
    };
    let local_type_declarations = &local_type_declarations;
    ctx.type_declarations = local_type_declarations.as_ref().clone();
    ctx.type_declaration_scope = Some(full_type_declarations_scope.clone());
    record_program_timing(timings, |timings| {
        timings.declaration_table_merging_cloning += merge_start.elapsed()
    });

    // `typeof <value>` inside a parameter annotation resolves against the
    // module's value bindings — imports and `const`s alike — which signature
    // collection alone never sees (function declarations hoist above them).
    // Collect the signatures inside a seeded environment (imported bindings +
    // the module's inferred value symbols, mirroring the check phase's merged
    // environment) so the exported function types carry real parameter types.
    // The seed is environment-only: `local_symbols` keeps just what collection
    // itself declared (the file's functions and classes) — a leaked seed would
    // re-report the declaration as TS2451 when the check phase declares it
    // again. Declaration files skip the seeding: their exports resolve through
    // the declaration tables, and running initializer inference over large
    // dependency `.d.ts` files here would be pure cost.
    // The per-file scope fallback (`module_scope_by_file`) is live only for
    // SIGNATURE collection: a parameter typed through a local alias of an
    // imported qualified type (`type BtnProps = React.ComponentProps<…>`)
    // must not bake a degraded signature into the module's symbols and
    // export table (the alias's attached scope carries no import layers).
    // Value collection and export-table construction stay map-less: with the
    // fallback live they eagerly materialize every exported initializer of a
    // large cyclic program (zod: +11s/+380MB), and their degraded shapes are
    // re-resolved lazily by the check phase anyway.
    let saved_module_scope_by_file = if values_resolve_through_module_scopes {
        ctx.module_scope_by_file.clone()
    } else {
        std::mem::take(&mut ctx.module_scope_by_file)
    };
    let mut signature_env = SymbolTable::new();
    let mut seeded_names: std::collections::HashSet<Arc<str>> = std::collections::HashSet::new();
    if !parsed_file.file_kind.is_declaration() {
        let mut import_seed = SymbolTable::new();
        if let Some(bindings) = preliminary_module_import_bindings[file_index].as_ref() {
            for (name, symbol) in bindings.symbols.iter_shared() {
                let _ = import_seed.insert_shared(name.clone(), symbol.clone());
            }
        }
        // Signature collection reads a value symbol in exactly three places: a
        // `typeof x` annotation, a class whose heritage names a value
        // (`class D extends mixin()`), and the key of an element write on a
        // declared function (`fn[key] = v`), whose type names the member it
        // declares. A file with none of them gets the imports alone; inferring
        // every initializer for it would only be redone by
        // `build_module_export_table` a moment later.
        let seed_needed = parsed_file.contains_typeof
            || statements_declare_class_with_heritage(&parsed_file.statements)
            || statements_write_computed_function_members(&parsed_file.statements);
        let value_env = if seed_needed {
            let split_start = analyze_split_enabled().then(Instant::now);
            ctx.thin_superseded_value_collection = thin_value_collection;
            let value_env = crate::modules::collect_exportable_value_symbols(
                &parsed_file.statements,
                local_type_declarations.as_ref(),
                &import_seed,
                None,
                parsed_file.is_module,
                ctx,
            );
            ctx.thin_superseded_value_collection = false;
            analyze_split_record(0, split_start);
            value_env
        } else {
            import_seed
        };
        for (name, symbol) in value_env.iter_shared() {
            let _ = signature_env.insert_shared(name.clone(), symbol.clone());
            seeded_names.insert(name.clone());
        }
    }
    ctx.module_scope_by_file = saved_module_scope_by_file;
    // The per-location signature map is only needed by the global path
    // (`collect_global_function_signatures`); module analysis consumes the
    // signatures through `signature_env` → `local_symbols`, so this one is
    // discarded.
    let mut discarded_function_signatures = HashMap::new();
    let diagnostics_before_signatures = ctx.diagnostics().len();
    let consults_before_signatures = super::scope_fallback_consult_count();
    let split_start = analyze_split_enabled().then(Instant::now);
    with_dts_expansion_reason(DtsExpansionReason::ModuleExportCollection, || {
        collect_function_signatures_from_statements(
            &parsed_file.statements,
            file_index,
            &mut signature_env,
            &mut discarded_function_signatures,
            ctx,
        )
    });
    analyze_split_record(1, split_start);
    let signature_scope_consults =
        super::scope_fallback_consult_count() - consults_before_signatures;
    let saved_module_scope_by_file = if values_resolve_through_module_scopes {
        ctx.module_scope_by_file.clone()
    } else {
        std::mem::take(&mut ctx.module_scope_by_file)
    };
    // A function the file declares is collection's own even when the seed
    // carried its name (a namespace merged into it gave the value walk an
    // entry), and its collected signature is the one to keep.
    let declared_functions: surge_ts_types::fx::FxHashSet<&str> = parsed_file
        .statements
        .iter()
        .filter_map(|statement| match crate::modules::peel_exported_statement(statement) {
            ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
            _ => None,
        })
        .collect();
    let mut local_symbols = SymbolTable::new();
    for (name, symbol) in signature_env.iter_shared() {
        if !seeded_names.contains(name) || declared_functions.contains(name.as_ref()) {
            let _ = local_symbols.insert_shared(name.clone(), symbol.clone());
        }
    }
    ctx.truncate_diagnostics(diagnostics_before_signatures);
    ctx.replace_resolved_named_types(1);

    // Lower this module's `declare global` augmentation values now that its
    // type environment (local declarations + import scope) is active. The
    // augmentation types were merged globally before binding, so a value such
    // as `var Buffer: BufferConstructor` sees the fully-merged interface while
    // `var x: ImportedType` still resolves through the module's imports.
    if lower_global_augmentation_values {
        lower_global_augmentation_values_from_statements(&parsed_file.statements, ctx);
    }

    let imported_symbols = preliminary_module_import_bindings[file_index]
        .as_ref()
        .map(|bindings| &bindings.symbols);
    let empty_imported_symbols = SymbolTable::new();
    let split_start = analyze_split_enabled().then(Instant::now);
    ctx.thin_superseded_value_collection = thin_value_collection;
    let (export_table, hidden_declaration_values) =
        with_dts_expansion_reason(DtsExpansionReason::ModuleExportCollection, || {
            crate::modules::build_module_export_table_and_hidden_values(
                parsed_file,
                local_type_declarations.as_ref(),
                &local_symbols,
                imported_symbols.unwrap_or(&empty_imported_symbols),
                Some(full_type_declarations_scope),
                ctx,
            )
        });
    ctx.thin_superseded_value_collection = false;
    analyze_split_record(2, split_start);
    ctx.module_scope_by_file = saved_module_scope_by_file;

    let analysis = ModuleAnalysis {
        local_type_declarations: local_type_declarations.clone(),
        local_symbols,
        local_symbol_names: None,
        local_export_table: export_table,
        hidden_declaration_values: hidden_declaration_values.filter(|hidden| hidden.iter_shared().next().is_some()).map(Arc::new),
    };
    ctx.type_declaration_scope = saved_type_declaration_scope;
    if let Some(start) = module_time_start {
        super::record_module_time(
            analysis_round,
            &parsed_file.file_name,
            start.elapsed().as_micros(),
        );
    }
    if let Some(start) = eq_probe_start {
        super::record_eq_probe_visit(
            file_index,
            super::EqProbeVisit {
                elapsed: start.elapsed(),
                signature_scope_consults,
                degraded_resolutions: super::degraded_resolution_count() - degraded_before,
                augmentation_insertions_after: super::augmentation_value_insertion_count(),
            },
        );
    }
    if let Some((start, rss_before, peak_before, footprint_before, footprint_peak_before)) =
        memory_before
    {
        let peak_after = crate::metrics::rss::peak_rss_bytes().unwrap_or(0);
        let footprint_after = crate::metrics::rss::current_footprint_bytes();
        let footprint_peak_after = crate::metrics::rss::peak_footprint_bytes();
        let high_water_before = footprint_peak_before.unwrap_or(peak_before);
        let high_water_after = footprint_peak_after.unwrap_or(peak_after);
        if high_water_after.saturating_sub(high_water_before)
            >= memory_trace_threshold.unwrap_or(u64::MAX)
        {
            let rss_after = crate::metrics::rss::current_rss_bytes().unwrap_or(0);
            eprintln!(
                "{{\"moduleMemory\":\"{}\",\"round\":{analysis_round},\"peakDeltaBytes\":{},\
                 \"rssBeforeBytes\":{rss_before},\"rssAfterBytes\":{rss_after},\
                 \"peakAfterBytes\":{peak_after},\"footprintBeforeBytes\":{},\
                 \"footprintAfterBytes\":{},\"peakFootprintAfterBytes\":{},\
                 \"elapsedMs\":{:.3}}}",
                parsed_file.file_name,
                high_water_after - high_water_before,
                footprint_before.map_or_else(|| "null".to_string(), |v| v.to_string()),
                footprint_after.map_or_else(|| "null".to_string(), |v| v.to_string()),
                footprint_peak_after.map_or_else(|| "null".to_string(), |v| v.to_string()),
                start.elapsed().as_secs_f64() * 1e3,
            );
        }
    }

    Some(analysis)
}

pub(crate) fn collect_module_analyses_with_bindings(
    parsed_files: &mut [ParsedProgramFile],
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    preliminary_module_import_bindings: &[Option<ModuleImportBindings>],
    lower_global_augmentation_values: bool,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> Vec<Option<ModuleAnalysis>> {
    let mut analyses = Vec::with_capacity(parsed_files.len());
    let memory_trace_threshold = module_memory_trace_threshold();
    let mut type_dedup_cache = TypeDedupCache::new();
    let analysis_round = next_analysis_round();
    let release_declaration_bodies = lower_global_augmentation_values
        && ctx.options.skip_lib_check
        && !super::eq_probe_enabled();

    for file_index in 0..parsed_files.len() {
        let parsed_file = &parsed_files[file_index];
        if !parsed_file.is_module && parsed_file.file_kind != FileKind::DependencyDeclaration {
            analyses.push(None);
            continue;
        }

        let analysis_result = analyze_module(
            file_index,
            parsed_file,
            local_type_declarations_by_module,
            preliminary_module_import_bindings,
            lower_global_augmentation_values,
            !lower_global_augmentation_values,
            false,
            analysis_round,
            memory_trace_threshold,
            ctx,
            timings,
        );
        let Some(mut analysis) = analysis_result else {
            analyses.push(None);
            continue;
        };
        if parsed_file.file_kind.is_declaration() && module_type_dedup_enabled() {
            with_dts_expansion_reason(DtsExpansionReason::ModuleDedup, || {
                dedup_module_analysis_types(&mut analysis, &mut type_dedup_cache)
            });
        }
        record_retained_export_nodes(
            &parsed_file.file_name,
            retained_module_analysis_type_nodes(&analysis),
        );
        if ctx.options.skip_lib_check
            && !super::eq_probe_enabled()
            && parsed_file.file_kind.is_declaration()
            && !parsed_file.contains_typeof
        {
            analysis.release_local_symbols_to_names();
        }
        analyses.push(Some(analysis));
        if release_declaration_bodies {
            retain_declaration_binding_surface(&mut parsed_files[file_index]);
        }
        if (file_index + 1) % 256 == 0 {
            crate::metrics::release_free_memory();
        }
    }
    analyze_split_report(analysis_round);

    analyses
}

pub(crate) fn retain_declaration_binding_surface(parsed_file: &mut ParsedProgramFile) {
    if !parsed_file.file_kind.is_declaration() {
        return;
    }
    parsed_file.statements.retain(|statement| match statement {
        ParsedStatement::ImportDeclaration(_) => true,
        ParsedStatement::ExportDeclaration(export) => !matches!(
            export.as_ref(),
            ParsedExportDeclaration::Statement { .. } | ParsedExportDeclaration::Default { .. }
        ),
        _ => false,
    });
    parsed_file.statements.shrink_to_fit();
}

pub(crate) fn release_lowered_declaration_type_bodies(parsed_file: &mut ParsedProgramFile) {
    if !parsed_file.file_kind.is_declaration() {
        return;
    }
    release_lowered_type_bodies_from_statements(&mut parsed_file.statements);
}

fn release_lowered_type_bodies_from_statements(statements: &mut [ParsedStatement]) {
    for statement in statements {
        match statement {
            ParsedStatement::TypeAliasDeclaration(alias) => {
                alias.type_parameters = Vec::new();
                alias.ty = surge_ts_syntax::ParsedType::Unknown;
            }
            ParsedStatement::InterfaceDeclaration(interface) => {
                interface.type_parameters = Vec::new();
                interface.extends = Vec::new();
                interface.members = Vec::new();
                interface.string_index_type = None;
                interface.call_signature = None;
                interface.construct_signatures = Vec::new();
            }
            ParsedStatement::ExportDeclaration(export) => {
                if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_mut() {
                    release_lowered_type_bodies_from_statements(std::slice::from_mut(declaration));
                }
            }
            ParsedStatement::DeclareModuleDeclaration(module) => {
                release_lowered_type_bodies_from_statements(&mut module.statements);
            }
            ParsedStatement::NamespaceDeclaration(namespace) => {
                release_lowered_type_bodies_from_statements(&mut namespace.statements);
            }
            _ => {}
        }
    }
}

/// Parallel counterpart of [`collect_module_analyses_with_bindings`] for the
/// preliminary pass (which never lowers `declare global` values — the final
/// pass's first-wins global publication is order-sensitive and stays serial
/// until it is scheduled around explicitly). Workers run the identical
/// [`analyze_module`] body against a speculative cache session; the commit
/// walk below publishes cache insertions, analyses, and diagnostics in serial
/// file order and re-analyzes any module whose observed cache hit/miss
/// pattern serial analysis would not have produced, so the result is
/// byte-identical to the serial pass (see `crate::speculative`).
struct WorkerModuleOutcome {
    file_index: usize,
    analysis: Option<ModuleAnalysis>,
    diagnostics: Vec<Diagnostic>,
    /// Utility-diagnostic keys this module's analysis recorded
    /// (`push_utility_diagnostic_once`). Serial analysis accumulates these
    /// on the rolling context — including keys whose diagnostics were
    /// truncated — and later phases consult them for suppression, so a
    /// clean commit must merge them and a key another module already
    /// published must invalidate the speculation (the worker emitted a
    /// diagnostic serial suppression would have swallowed).
    utility_key_additions: std::collections::HashSet<
        crate::context::UtilityDiagnosticKey,
        surge_ts_types::fx::FxBuildHasher,
    >,
    /// The module's post-analysis named-resolution memo, captured only for
    /// the last analyzed module: serial analysis leaves the last module's
    /// memo on the rolling context and later stages observably resolve
    /// through it. Earlier modules' memos are never observable (every
    /// analysis replaces the memo at entry) and retaining them would keep
    /// speculative intermediate expansions alive in the weak canonical
    /// store.
    resolved_named_types: Option<
        Arc<
            Mutex<
                surge_ts_types::fx::FxHashMap<
                    crate::context::DeclarationResolutionKey,
                    crate::context::DeclarationResolutionState,
                >,
            >,
        >,
    >,
    resolved_named_types_identity: crate::context::EnvironmentMapIdentity,
}

/// Divergence-hunt probes read once per pass from the environment; see the
/// comments on each field's reader for what it isolates.
#[derive(Clone, Copy)]
struct AnalysisProbes {
    par_range: Option<(usize, usize)>,
    per_module_sessions: bool,
    declarations_serial: bool,
    fresh_range: Option<(usize, usize)>,
    probe_all: bool,
    product_probe: Option<usize>,
}

impl AnalysisProbes {
    fn from_env() -> Self {
        // Divergence-bisection probe: `SURGE_ANALYSIS_PAR_RANGE=lo:hi` restricts
        // worker dispatch to modules with `lo <= file_index < hi`; everything else
        // takes the serial-only commit path (the exact serial regime).
        let par_range: Option<(usize, usize)> = std::env::var("SURGE_ANALYSIS_PAR_RANGE")
            .ok()
            .and_then(|spec| {
                let (lo, hi) = spec.split_once(':')?;
                Some((lo.parse().ok()?, hi.parse().ok()?))
            });
        let per_module_sessions = std::env::var_os("SURGE_ANALYSIS_MODULE_SESSIONS").is_some();
        let declarations_serial = std::env::var_os("SURGE_ANALYSIS_DECL_SERIAL").is_some();
        // Divergence-bisection probe: `SURGE_ANALYSIS_FRESH_RANGE=lo:hi` analyzes
        // serial-path modules in the range on a fresh pass-start context clone
        // (live cache view), separating context-instance effects from
        // snapshot-view effects during hunts.
        let fresh_range: Option<(usize, usize)> = std::env::var("SURGE_ANALYSIS_FRESH_RANGE")
            .ok()
            .and_then(|spec| {
                let (lo, hi) = spec.split_once(':')?;
                Some((lo.parse().ok()?, hi.parse().ok()?))
            });
        // Divergence-hunt probe: `SURGE_ANALYSIS_PRODUCT_PROBE=<file_index>` dumps
        // the module's committed analysis-product fingerprints and per-insert
        // value fingerprints, for diffing regimes.
        let product_probe_env = std::env::var("SURGE_ANALYSIS_PRODUCT_PROBE").ok();
        let probe_all = product_probe_env.as_deref() == Some("all");
        let product_probe: Option<usize> = product_probe_env.and_then(|value| value.parse().ok());
        Self {
            par_range,
            per_module_sessions,
            declarations_serial,
            fresh_range,
            probe_all,
            product_probe,
        }
    }

    fn probed(&self, file_index: usize) -> bool {
        self.probe_all || self.product_probe == Some(file_index)
    }
}

pub(crate) fn collect_module_analyses_with_bindings_parallel(
    parsed_files: &[ParsedProgramFile],
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    preliminary_module_import_bindings: &[Option<ModuleImportBindings>],
    lower_global_augmentation_values: bool,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
    worker_count: usize,
) -> Vec<Option<ModuleAnalysis>> {
    debug_assert!(worker_count > 1, "serial analysis uses the dedicated path");

    let memory_trace_threshold = module_memory_trace_threshold();
    let analysis_round = next_analysis_round();
    let live = crate::speculative::LiveCacheHandles::capture(ctx);
    let base = Arc::new(crate::speculative::CacheSnapshots::capture(&live));
    let probes = AnalysisProbes::from_env();

    // The worker seed carries the pass-start utility keys as a shared
    // baseline: per-module clones then start with an empty overlay whose
    // post-analysis content is exactly the module's own key additions, and the
    // clone stops deep-copying the accumulated key set per module.
    let worker_seed = {
        let mut seed = ctx.clone();
        seed.diagnostics.clear();
        seed.clear_diagnostic_keys();
        // `analyze_module` replaces `type_declarations` with the module's own
        // table before any read, so the seed doesn't carry the rolling table —
        // every per-module clone would deep-copy it for nothing.
        seed.type_declarations = TypeDeclarationTable::new();
        seed.snapshot_utility_keys_into_baseline();
        seed
    };
    let worker_phase_start = Instant::now();
    let worker_outputs = run_analysis_workers(
        parsed_files,
        local_type_declarations_by_module,
        preliminary_module_import_bindings,
        lower_global_augmentation_values,
        timings,
        worker_count,
        memory_trace_threshold,
        analysis_round,
        &live,
        &base,
        &probes,
        &worker_seed,
    );

    let worker_phase = worker_phase_start.elapsed();
    let commit_phase_start = Instant::now();
    // The fan-out snapshot is only read through worker sessions; every session
    // is gone once the scope joins, so release the six cloned maps before the
    // commit walk instead of holding them across it.
    drop(base);
    let analyses = commit_module_analyses(
        parsed_files,
        local_type_declarations_by_module,
        preliminary_module_import_bindings,
        lower_global_augmentation_values,
        ctx,
        timings,
        memory_trace_threshold,
        analysis_round,
        &live,
        &probes,
        &worker_seed,
        worker_outputs,
        worker_phase,
        commit_phase_start,
    );
    crate::metrics::release_free_memory();
    analyses
}

fn run_analysis_workers(
    parsed_files: &[ParsedProgramFile],
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    preliminary_module_import_bindings: &[Option<ModuleImportBindings>],
    lower_global_augmentation_values: bool,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
    worker_count: usize,
    memory_trace_threshold: Option<u64>,
    analysis_round: u64,
    live: &crate::speculative::LiveCacheHandles,
    base: &Arc<crate::speculative::CacheSnapshots>,
    probes: &AnalysisProbes,
    worker_seed: &CheckerContext,
) -> Vec<(Vec<WorkerModuleOutcome>, Vec<crate::speculative::FileCacheLog>)> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let AnalysisProbes {
        par_range,
        per_module_sessions,
        declarations_serial,
        ..
    } = *probes;
    // Only the LAST analyzed module's named-resolution memo is observable
    // after the pass (later stages resolve through the rolling context's memo;
    // every earlier module's memo is replaced before anything reads it), so
    // only that outcome captures its memo. Retaining every module's memo until
    // its commit both bloats the walk's footprint and — worse — keeps each
    // speculative attempt's intermediate expansions strongly alive, so a
    // serial recheck can intern-hit a discarded display variant in the weak
    // canonical store instead of computing the serial display form.
    let last_module_index = parsed_files
        .iter()
        .enumerate()
        .rev()
        .find(|(_, file)| file.is_module || file.file_kind == FileKind::DependencyDeclaration)
        .map(|(index, _)| index);
    let next_index = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let next_index = &next_index;
        let worker_seed = worker_seed;
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let seed_ctx = worker_seed.clone();
            let session = Arc::new(crate::speculative::CheckSession::new(
                live.clone(),
                base.clone(),
            ));
            let live_for_workers = live.clone();
            let base = base.clone();
            handles.push(scope.spawn(move || {
                let type_store = seed_ctx.program_type_store.clone();
                let mut module_logs = Vec::new();
                let module_logs_ref = &mut module_logs;
                let outcomes = with_program_type_store(type_store, || {
                    crate::speculative::with_check_session(session.clone(), || {
                        let module_logs = module_logs_ref;
                        let mut outcomes = Vec::new();
                        loop {
                            // Ascending dispatch keeps each worker's overlay a
                            // subsequence of serial module order.
                            let file_index = next_index.fetch_add(1, Ordering::Relaxed);
                            if file_index >= parsed_files.len() {
                                break;
                            }
                            let parsed_file = &parsed_files[file_index];
                            if !parsed_file.is_module
                                && parsed_file.file_kind != FileKind::DependencyDeclaration
                            {
                                continue;
                            }
                            if declarations_serial && parsed_file.file_kind.is_declaration() {
                                continue;
                            }
                            if let Some((lo, hi)) = par_range
                                && !(lo..hi).contains(&file_index)
                            {
                                continue;
                            }
                            // Final pass: a module with a `declare global`
                            // block publishes global values first-wins, which
                            // is order-sensitive — it runs on the serial
                            // coordinator path at its file-order position.
                            if lower_global_augmentation_values
                                && crate::driver::has_global_augmentation_block(
                                    &parsed_file.statements,
                                )
                            {
                                continue;
                            }
                            // A fresh per-module context: unlike the check
                            // phase (whose `begin_file_check` re-scopes a
                            // reused context per file), the analysis body has
                            // no per-module reset, so a reused worker context
                            // would carry another module's resolution state in
                            // a schedule-dependent way.
                            let mut local_ctx = seed_ctx.clone();
                            local_ctx.diagnostics.clear();
                            // Probe (`SURGE_ANALYSIS_MODULE_SESSIONS=1`): give
                            // the module a private session with no worker
                            // overlay, isolating overlay-visibility effects
                            // from fresh-context effects during divergence
                            // hunts.
                            let module_session = per_module_sessions.then(|| {
                                Arc::new(crate::speculative::CheckSession::new(
                                    live_for_workers.clone(),
                                    base.clone(),
                                ))
                            });
                            let active_session = module_session.as_ref().unwrap_or(&session);
                            active_session.begin_file(file_index);
                            let analysis = crate::speculative::with_check_session(
                                active_session.clone(),
                                || {
                                    analyze_module(
                                        file_index,
                                        parsed_file,
                                        local_type_declarations_by_module,
                                        preliminary_module_import_bindings,
                                        false,
                                        true,
                                        false,
                                        analysis_round,
                                        memory_trace_threshold,
                                        &mut local_ctx,
                                        timings,
                                    )
                                },
                            );
                            if let Some(module_session) = module_session {
                                module_logs.extend(module_session.take_file_logs());
                            }
                            let diagnostics = std::mem::take(&mut local_ctx.diagnostics);
                            let utility_key_additions =
                                std::mem::take(&mut local_ctx.utility_diagnostic_keys);
                            let capture_memo = Some(file_index) == last_module_index;
                            outcomes.push(WorkerModuleOutcome {
                                file_index,
                                analysis,
                                diagnostics,
                                utility_key_additions,
                                resolved_named_types: capture_memo
                                    .then(|| local_ctx.resolved_named_types.clone()),
                                resolved_named_types_identity: local_ctx
                                    .resolved_named_types_identity
                                    .clone(),
                            });
                        }
                        outcomes
                    })
                });
                let mut logs = session.take_file_logs();
                logs.extend(module_logs);
                (outcomes, logs)
            }));
        }
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("parallel module analysis worker panicked")
            })
            .collect::<Vec<_>>()
    })
}

fn commit_module_analyses(
    parsed_files: &[ParsedProgramFile],
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    preliminary_module_import_bindings: &[Option<ModuleImportBindings>],
    lower_global_augmentation_values: bool,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
    memory_trace_threshold: Option<u64>,
    analysis_round: u64,
    live: &crate::speculative::LiveCacheHandles,
    probes: &AnalysisProbes,
    worker_seed: &CheckerContext,
    worker_outputs: Vec<(Vec<WorkerModuleOutcome>, Vec<crate::speculative::FileCacheLog>)>,
    worker_phase: std::time::Duration,
    commit_phase_start: Instant,
) -> Vec<Option<ModuleAnalysis>> {
    let AnalysisProbes {
        fresh_range,
        probe_all,
        product_probe,
        ..
    } = *probes;
    let probed = |file_index: usize| probes.probed(file_index);
    let mut analyses: Vec<Option<ModuleAnalysis>> = (0..parsed_files.len()).map(|_| None).collect();
    let mut slots: Vec<Option<WorkerModuleOutcome>> =
        (0..parsed_files.len()).map(|_| None).collect();
    let mut logs_by_index: Vec<Option<crate::speculative::FileCacheLog>> =
        (0..parsed_files.len()).map(|_| None).collect();
    let mut logs = Vec::new();
    for (outcomes, worker_logs) in worker_outputs {
        for outcome in outcomes {
            let file_index = outcome.file_index;
            slots[file_index] = Some(outcome);
        }
        logs.extend(worker_logs);
    }
    for log in logs {
        let file_index = log.file_index;
        logs_by_index[file_index] = Some(log);
    }
    crate::speculative::report_conflict_dag(&logs_by_index);

    // Deterministic commit in file order. Declaration-file type dedup runs here
    // (not in workers): its shared cache picks pointer-identity representatives,
    // and representative identity feeds the pinned dedup-fingerprint machinery,
    // so it must be chosen in the serial file order.
    let cap = crate::infer::types::cache::generic_instantiation_bucket_cap();
    let mut published = surge_ts_types::fx::FxHashSet::default();
    let mut dirty = surge_ts_types::fx::FxHashSet::default();
    let mut stats = crate::speculative::StcCommitStats::default();
    let mut type_dedup_cache = TypeDedupCache::new();
    for file_index in 0..parsed_files.len() {
        let parsed_file = &parsed_files[file_index];
        if !parsed_file.is_module && parsed_file.file_kind != FileKind::DependencyDeclaration {
            continue;
        }
        let mut outcome_analysis;
        let had_speculative_attempt = logs_by_index[file_index].is_some();
        // A key this module recorded that an earlier-committed module also
        // recorded means serial suppression would have swallowed the worker's
        // emission: the speculation is invalid even though every cache
        // observation validated. Checked against the rolling overlay, which at
        // this point holds exactly the earlier modules' merged additions plus
        // the pass-start keys (which worker baselines also contained, so they
        // can never appear among a worker's additions).
        let utility_conflict = slots[file_index].as_ref().is_some_and(|outcome| {
            outcome
                .utility_key_additions
                .iter()
                .any(|key| ctx.utility_diagnostic_keys.contains(key))
        });
        if product_probe == Some(file_index) {
            let mut lines: Vec<String> = published
                .iter()
                .map(|digest| format!("x{digest:x}"))
                .collect();
            lines.sort_unstable();
            for line in lines {
                eprintln!("[probe-published] {line}");
            }
        }
        let verdict = match logs_by_index[file_index].as_ref() {
            Some(_) if utility_conflict => {
                stats.files += 1;
                stats.miss_conflicts += 1;
                crate::speculative::CommitVerdict::MissConflict
            }
            Some(log) => crate::speculative::commit_file_log(
                &live,
                log,
                &mut published,
                &dirty,
                cap,
                &mut stats,
            ),
            // Serial-only module (declaration kind): always analyzed here, on
            // the rolling coordinator context — the exact serial regime.
            None => {
                stats.files += 1;
                crate::speculative::CommitVerdict::MissConflict
            }
        };
        match verdict {
            crate::speculative::CommitVerdict::Clean => {
                let outcome = slots[file_index]
                    .take()
                    .expect("worker outcome for committed module");
                outcome_analysis = outcome.analysis;
                for diagnostic in outcome.diagnostics {
                    ctx.push_collected(diagnostic);
                }
                ctx.utility_diagnostic_keys
                    .extend(outcome.utility_key_additions);
                // Reproduce the serial carry: after the pass, the rolling
                // context holds the last analyzed module's memo, which later
                // stages observably resolve through.
                if let Some(memo) = outcome.resolved_named_types {
                    ctx.resolved_named_types = memo;
                    ctx.resolved_named_types_identity = outcome.resolved_named_types_identity;
                }
            }
            crate::speculative::CommitVerdict::MissConflict
            | crate::speculative::CommitVerdict::DependencyConflict => {
                dirty.insert(file_index);
                // Drop the discarded attempt's outcome AND its log before
                // re-analyzing: the log's insert entries hold the attempt's
                // computed types strongly, which keeps their weak canonical-
                // store entries alive — the recheck would then intern-hit the
                // discarded attempt's display variants instead of computing
                // the serial display forms.
                slots[file_index] = None;
                logs_by_index[file_index] = None;
                let session = Arc::new(crate::speculative::CheckSession::new_live_reading(
                    live.clone(),
                ));
                let fresh_serial =
                    fresh_range.is_some_and(|(lo, hi)| (lo..hi).contains(&file_index));
                if fresh_serial {
                    let mut fresh_ctx = worker_seed.clone();
                    if std::env::var_os("SURGE_ANALYSIS_FRESH_STORE").is_some() {
                        fresh_ctx.program_type_store = surge_ts_types::ProgramTypeStore::new();
                        fresh_ctx
                            .program_type_store
                            .set_strict_null_checks(fresh_ctx.options.strict_null_checks);
                        fresh_ctx
                            .program_type_store
                            .set_exact_optional_property_types(fresh_ctx.options.exact_optional_property_types);
                    }
                    let fresh_store = fresh_ctx.program_type_store.clone();
                    outcome_analysis = with_program_type_store(fresh_store, || {
                        crate::speculative::with_check_session(session.clone(), || {
                            session.begin_file(file_index);
                            analyze_module(
                                file_index,
                                parsed_file,
                                local_type_declarations_by_module,
                                preliminary_module_import_bindings,
                                lower_global_augmentation_values,
                                !lower_global_augmentation_values,
                                false,
                                analysis_round,
                                memory_trace_threshold,
                                &mut fresh_ctx,
                                timings,
                            )
                        })
                    });
                    for diagnostic in std::mem::take(&mut fresh_ctx.diagnostics) {
                        ctx.push_collected(diagnostic);
                    }
                    ctx.utility_diagnostic_keys
                        .extend(std::mem::take(&mut fresh_ctx.utility_diagnostic_keys));
                    ctx.resolved_named_types = fresh_ctx.resolved_named_types.clone();
                    ctx.resolved_named_types_identity =
                        fresh_ctx.resolved_named_types_identity.clone();
                    for fresh_log in session.take_file_logs() {
                        if probed(file_index) {
                            for line in fresh_log.debug_value_lines() {
                                eprintln!("[probe-insert-fresh] f{file_index} {line}");
                            }
                            for line in fresh_log.debug_miss_lines() {
                                eprintln!("[probe-miss-fresh] f{file_index} {line}");
                            }
                        }
                        crate::speculative::apply_file_log(
                            &live,
                            &fresh_log,
                            &mut published,
                            cap,
                            &mut stats,
                        );
                    }
                } else {
                    // The recheck keeps attempt 0: its environment identities
                    // (and every downstream cache key formed from them, e.g.
                    // by the final pass reading prelim-created environments)
                    // must match the serial regime exactly, or later passes
                    // key the same logical entries differently and cleanly
                    // commit divergent display variants. Colliding with the
                    // discarded speculative attempt's identities is harmless:
                    // the discarded attempt's cache insertions are never
                    // published, and interned environments only attach
                    // content-stamped table snapshots.
                    let _ = had_speculative_attempt;
                    ctx.environment_attempt = 0;
                    outcome_analysis =
                        crate::speculative::with_check_session(session.clone(), || {
                            session.begin_file(file_index);
                            analyze_module(
                                file_index,
                                parsed_file,
                                local_type_declarations_by_module,
                                preliminary_module_import_bindings,
                                lower_global_augmentation_values,
                                !lower_global_augmentation_values,
                                false,
                                analysis_round,
                                memory_trace_threshold,
                                ctx,
                                timings,
                            )
                        });
                    ctx.environment_attempt = 0;
                    for recheck_log in session.take_file_logs() {
                        if probed(file_index) {
                            for line in recheck_log.debug_value_lines() {
                                eprintln!("[probe-insert-serial] f{file_index} {line}");
                            }
                            for line in recheck_log.debug_miss_lines() {
                                eprintln!("[probe-miss-serial] f{file_index} {line}");
                            }
                        }
                        let complete = crate::speculative::apply_file_log(
                            &live,
                            &recheck_log,
                            &mut published,
                            cap,
                            &mut stats,
                        );
                        debug_assert!(complete, "re-analysis publication must be complete");
                    }
                }
            }
        }
        if let Some(analysis) = outcome_analysis.as_mut()
            && parsed_file.file_kind.is_declaration()
            && module_type_dedup_enabled()
        {
            with_dts_expansion_reason(DtsExpansionReason::ModuleDedup, || {
                dedup_module_analysis_types(analysis, &mut type_dedup_cache)
            });
        }
        if let Some(analysis) = outcome_analysis.as_ref() {
            record_retained_export_nodes(
                &parsed_file.file_name,
                retained_module_analysis_type_nodes(analysis),
            );
        }
        analyses[file_index] = outcome_analysis;
        // The committed file's log (insert-value clones) is dead now; release
        // it progressively rather than holding every file's inserts across the
        // whole walk.
        if product_probe.is_none() && !probe_all {
            logs_by_index[file_index] = None;
        }
        if probed(file_index) {
            if let Some(log) = logs_by_index[file_index].as_ref() {
                for line in log.debug_value_lines() {
                    eprintln!("[probe-insert-worker] f{file_index} {line}");
                }
                for line in log.debug_miss_lines() {
                    eprintln!("[probe-miss-worker] f{file_index} {line}");
                }
            }
            if let Some(analysis) = analyses[file_index].as_ref() {
                let mut lines = Vec::new();
                for (name, symbol) in analysis.local_symbols.iter_shared() {
                    lines.push(format!(
                        "sym {name}=v{:x}",
                        crate::speculative::display_type_fingerprint(&symbol.ty)
                    ));
                }
                for (name, symbol) in analysis.local_export_table.symbols.iter_shared() {
                    lines.push(format!(
                        "exp {name}=v{:x}",
                        crate::speculative::display_type_fingerprint(&symbol.ty)
                    ));
                }
                if let Some(symbol) = &analysis.local_export_table.default_symbol {
                    lines.push(format!(
                        "exp default=v{:x}",
                        crate::speculative::display_type_fingerprint(&symbol.ty)
                    ));
                }
                if let Some(symbol) = &analysis.local_export_table.export_assignment_symbol {
                    lines.push(format!(
                        "exp assign=v{:x}",
                        crate::speculative::display_type_fingerprint(&symbol.ty)
                    ));
                }
                lines.sort_unstable();
                for line in lines {
                    eprintln!("[probe-product] f{file_index} {line}");
                }
            }
        }
    }
    if std::env::var_os("SURGE_STC_STATS").is_some() {
        eprintln!(
            "[stc-analysis] modules={} clean={} miss_conflicts={} dep_conflicts={} published={} \
             worker_phase={:.2}s commit_phase={:.2}s",
            stats.files,
            stats.clean_commits,
            stats.miss_conflicts,
            stats.dependency_conflicts,
            stats.published_entries,
            worker_phase.as_secs_f64(),
            commit_phase_start.elapsed().as_secs_f64(),
        );
    }
    analyses
}

/// Experiment toggle while the dedup is being validated: `SURGE_MODULE_TYPE_DEDUP=0`
/// disables the retained-type dedup without rebuilding.
fn module_type_dedup_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_MODULE_TYPE_DEDUP").as_deref() != Ok("0"))
}

/// Node budget for the "is this type large enough to dedup" walk: a type whose
/// bounded traversal exhausts the budget is a dedup candidate. Small types are
/// cheaper to keep private than to hash.
const TYPE_DEDUP_NODE_BUDGET: usize = 64;

/// Bound on structurally-distinct types sharing one display key, so a
/// pathological same-display family cannot make every lookup a deep-equality
/// scan of an unbounded bucket.
const TYPE_DEDUP_BUCKET_CAP: usize = 8;

/// Per-round dedup store for retained module-analysis symbol types, keyed by
/// a bounded structural fingerprint. See [`dedup_module_analysis_types`].
type TypeDedupCache = HashMap<u64, Vec<Type>>;

/// Node budget for the fingerprint walk. Rendering (`Type::name`) is not usable
/// as a key here: displays render a shared DAG as a tree, so a heavily-shared
/// expansion explodes exponentially. The fingerprint instead hashes a bounded
/// DFS prefix — structure plus the display fields equality excludes
/// (`alias_name`, reference display) so payload sharing cannot change
/// diagnostic text within the fingerprinted prefix.
const TYPE_DEDUP_FINGERPRINT_NODE_BUDGET: usize = 4096;

fn type_dedup_fingerprint(ty: &Type) -> u64 {
    use std::hash::Hasher;
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    let mut budget = TYPE_DEDUP_FINGERPRINT_NODE_BUDGET;
    fingerprint_type(ty, &mut hasher, &mut budget);
    hasher.finish()
}

fn fingerprint_type(ty: &Type, hasher: &mut impl std::hash::Hasher, budget: &mut usize) {
    use std::hash::Hash;
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    std::mem::discriminant(ty).hash(hasher);
    match ty {
        Type::StringLiteral(value) => value.hash(hasher),
        Type::NumberLiteral(value) => value.value.hash(hasher),
        Type::BooleanLiteral(value) => value.hash(hasher),
        Type::Object(object) => {
            object.alias_name.hash(hasher);
            object.alias_id.hash(hasher);
            object.properties.len().hash(hasher);
            for (name, property) in object.properties.iter() {
                name.hash(hasher);
                property.is_optional().hash(hasher);
                fingerprint_type(&property.ty, hasher, budget);
                if *budget == 0 {
                    return;
                }
            }
            if let Some(index) = object.string_index_type.as_deref() {
                fingerprint_type(index, hasher, budget);
            }
            if let Some(call) = object.call_signature() {
                fingerprint_function(call, hasher, budget);
            }
            if let Some(construct) = object.construct_signature() {
                fingerprint_function(construct, hasher, budget);
            }
        }
        Type::Function(function) => fingerprint_function(function, hasher, budget),
        Type::Array(element) => fingerprint_type(element, hasher, budget),
        Type::Tuple(elements) => {
            elements.len().hash(hasher);
            for element in elements {
                fingerprint_type(element, hasher, budget);
                if *budget == 0 {
                    return;
                }
            }
        }
        Type::Union(union) => {
            union.types().len().hash(hasher);
            for member in union.types() {
                fingerprint_type(member, hasher, budget);
                if *budget == 0 {
                    return;
                }
            }
        }
        Type::Reference(reference) => {
            reference.id.hash(hasher);
            reference.display.hash(hasher);
            reference.arguments.len().hash(hasher);
            for argument in reference.arguments.iter() {
                fingerprint_type(argument, hasher, budget);
                if *budget == 0 {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn fingerprint_function(
    function: &surge_ts_types::FunctionType,
    hasher: &mut impl std::hash::Hasher,
    budget: &mut usize,
) {
    use std::hash::Hash;
    function.parameters().len().hash(hasher);
    for parameter in function.parameters() {
        fingerprint_type(parameter, hasher, budget);
        if *budget == 0 {
            return;
        }
    }
    fingerprint_type(function.return_type(), hasher, budget);
}

/// Full equality for dedup candidates. `Type::PartialEq` is unusable here: the
/// retained expansions are heavily-shared DAGs, and a naive recursive compare
/// of two separately-built equal DAGs expands them into trees (exponential
/// blow-up). This comparator memoizes visited payload-pointer pairs so every
/// distinct subtree pair is compared once, and — unlike `PartialEq` — includes
/// the display fields (`alias_name`, reference `display`), so a shared payload
/// can never change rendered diagnostic text.
fn types_identical_for_dedup(
    a: &Type,
    b: &Type,
    seen: &mut HashSet<(usize, usize)>,
    nodes_visited: &mut u64,
) -> bool {
    *nodes_visited += 1;
    fn pair_visited(seen: &mut HashSet<(usize, usize)>, a: usize, b: usize) -> bool {
        a == b || !seen.insert((a, b))
    }

    match (a, b) {
        (Type::Object(a), Type::Object(b)) => {
            if a.alias_name != b.alias_name
                || a.alias_id != b.alias_id
                || a.properties.len() != b.properties.len()
            {
                return false;
            }
            // The pointer-pair memo covers only the property-list recursion:
            // objects can share a properties `Arc` while differing in the
            // sibling index/call/construct fields (`open_if_unmodelled`), so
            // those are always compared.
            let (pa, pb) = (
                Arc::as_ptr(&a.properties) as usize,
                Arc::as_ptr(&b.properties) as usize,
            );
            if !pair_visited(seen, pa, pb) {
                for ((name_a, prop_a), (name_b, prop_b)) in
                    a.properties.iter().zip(b.properties.iter())
                {
                    if name_a != name_b
                        || prop_a.is_optional() != prop_b.is_optional()
                        || !types_identical_for_dedup(&prop_a.ty, &prop_b.ty, seen, nodes_visited)
                    {
                        return false;
                    }
                }
            }
            match (
                a.string_index_type.as_deref(),
                b.string_index_type.as_deref(),
            ) {
                (None, None) => {}
                (Some(ia), Some(ib)) => {
                    if !types_identical_for_dedup(ia, ib, seen, nodes_visited) {
                        return false;
                    }
                }
                _ => return false,
            }
            match (a.call_signature(), b.call_signature()) {
                (None, None) => {}
                (Some(ca), Some(cb)) => {
                    if !functions_identical_for_dedup(ca, cb, seen, nodes_visited) {
                        return false;
                    }
                }
                _ => return false,
            }
            match (a.construct_signature(), b.construct_signature()) {
                (None, None) => true,
                (Some(ca), Some(cb)) => functions_identical_for_dedup(ca, cb, seen, nodes_visited),
                _ => false,
            }
        }
        (Type::Function(a), Type::Function(b)) => {
            functions_identical_for_dedup(a, b, seen, nodes_visited)
        }
        (Type::Array(a), Type::Array(b)) => types_identical_for_dedup(a, b, seen, nodes_visited),
        (Type::Tuple(a), Type::Tuple(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(ea, eb)| types_identical_for_dedup(ea, eb, seen, nodes_visited))
        }
        (Type::Union(a), Type::Union(b)) => {
            let (ta, tb) = (a.types(), b.types());
            if ta.len() != tb.len() {
                return false;
            }
            // Empty slices share a dangling pointer, so only non-empty payloads
            // are valid identity keys (same below for reference arguments).
            if !ta.is_empty() && pair_visited(seen, ta.as_ptr() as usize, tb.as_ptr() as usize) {
                return true;
            }
            ta.iter()
                .zip(tb.iter())
                .all(|(ma, mb)| types_identical_for_dedup(ma, mb, seen, nodes_visited))
        }
        (Type::Reference(a), Type::Reference(b)) => {
            if a.id != b.id || a.display != b.display || a.arguments.len() != b.arguments.len() {
                return false;
            }
            if !a.arguments.is_empty()
                && pair_visited(
                    seen,
                    a.arguments.as_ptr() as usize,
                    b.arguments.as_ptr() as usize,
                )
            {
                return true;
            }
            a.arguments
                .iter()
                .zip(b.arguments.iter())
                .all(|(aa, ab)| types_identical_for_dedup(aa, ab, seen, nodes_visited))
        }
        (Type::StringLiteral(a), Type::StringLiteral(b)) => a == b,
        (Type::NumberLiteral(a), Type::NumberLiteral(b)) => a == b,
        (Type::BooleanLiteral(a), Type::BooleanLiteral(b)) => a == b,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

fn functions_identical_for_dedup(
    a: &surge_ts_types::FunctionType,
    b: &surge_ts_types::FunctionType,
    seen: &mut HashSet<(usize, usize)>,
    nodes_visited: &mut u64,
) -> bool {
    if a.is_variadic() != b.is_variadic()
        || a.required_parameter_count() != b.required_parameter_count()
        || a.parameters().len() != b.parameters().len()
    {
        return false;
    }
    if pair_visited_fn(seen, a, b) {
        return true;
    }
    a.parameters()
        .iter()
        .zip(b.parameters().iter())
        .all(|(pa, pb)| types_identical_for_dedup(pa, pb, seen, nodes_visited))
        && types_identical_for_dedup(a.return_type(), b.return_type(), seen, nodes_visited)
}

fn pair_visited_fn(
    seen: &mut HashSet<(usize, usize)>,
    a: &surge_ts_types::FunctionType,
    b: &surge_ts_types::FunctionType,
) -> bool {
    if a.parameters().is_empty() {
        return false;
    }
    let (pa, pb) = (
        a.parameters().as_ptr() as usize,
        b.parameters().as_ptr() as usize,
    );
    pa == pb || !seen.insert((pa, pb))
}

/// Walks `ty` decrementing `budget` per node; returns `true` (large) once the
/// budget is exhausted. References count their arguments but are never
/// resolved, so the walk cannot force an expansion.
fn type_dedup_budget_exhausted(ty: &Type, budget: &mut usize) -> bool {
    if *budget == 0 {
        return true;
    }
    *budget -= 1;
    match ty {
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| type_dedup_budget_exhausted(&property.ty, budget))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(|index| type_dedup_budget_exhausted(index, budget))
                || object
                    .call_signature()
                    .is_some_and(|call| function_dedup_budget_exhausted(call, budget))
                || object
                    .construct_signature()
                    .is_some_and(|construct| function_dedup_budget_exhausted(construct, budget))
        }
        Type::Function(function) => function_dedup_budget_exhausted(function, budget),
        Type::Array(element) => type_dedup_budget_exhausted(element, budget),
        Type::Tuple(elements) => elements
            .iter()
            .any(|element| type_dedup_budget_exhausted(element, budget)),
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| type_dedup_budget_exhausted(member, budget)),
        Type::Reference(reference) => reference
            .arguments
            .iter()
            .any(|argument| type_dedup_budget_exhausted(argument, budget)),
        _ => false,
    }
}

fn function_dedup_budget_exhausted(
    function: &surge_ts_types::FunctionType,
    budget: &mut usize,
) -> bool {
    function
        .parameters()
        .iter()
        .any(|parameter| type_dedup_budget_exhausted(parameter, budget))
        || type_dedup_budget_exhausted(function.return_type(), budget)
}

/// Returns the canonical equal type when `ty` duplicates one already retained
/// this round, sharing its payload instead of keeping a private copy.
///
/// Equality is `Type == Type` (structural/nominal) *plus* an equal display
/// fingerprint (the cache key covers `alias_name`/reference display), so a
/// shared payload cannot change a diagnostic's text within the fingerprinted
/// prefix. Sharing an equal `Type::Reference` is consistent with the program
/// instantiation interner, which already assumes `(declaration, resolved
/// arguments)` determines an expansion. The cache lives for one analysis round:
/// duplicates within a round (e.g. hundreds of icon `.d.ts` modules whose
/// identical annotation degrades and therefore never interns) collapse to one
/// payload, while cross-round sharing — where deferred-resolution snapshots
/// differ — is never attempted.
fn dedup_retained_type(ty: &Type, cache: &mut TypeDedupCache) -> Option<Type> {
    let mut budget = TYPE_DEDUP_NODE_BUDGET;
    if !type_dedup_budget_exhausted(ty, &mut budget) {
        return None;
    }
    let key = type_dedup_fingerprint(ty);
    let bucket = cache.entry(key).or_default();
    for existing in bucket.iter() {
        let nominal = matches!((existing, ty), (Type::Reference(_), Type::Reference(_)));
        let mut nodes_visited = 0;
        let identical =
            types_identical_for_dedup(existing, ty, &mut HashSet::new(), &mut nodes_visited);
        record_program_counter(|c| {
            c.module_type_dedup_structural_nodes_visited_count += nodes_visited;
            if !nominal {
                c.module_type_dedup_structural_comparison_count += 1;
            }
        });
        if identical {
            record_program_counter(|c| {
                c.module_type_dedup_hit_count += 1;
                if nominal {
                    c.module_type_dedup_nominal_match_count += 1;
                }
            });
            return Some(existing.clone());
        }
    }
    if bucket.len() < TYPE_DEDUP_BUCKET_CAP {
        record_program_counter(|c| c.module_type_dedup_insert_count += 1);
        bucket.push(ty.clone());
    }
    None
}

fn dedup_symbol_table_types(table: &mut SymbolTable, cache: &mut TypeDedupCache) {
    let replacements: Vec<_> = table
        .iter_shared()
        .filter_map(|(name, symbol)| {
            dedup_retained_type(&symbol.ty, cache).map(|canonical| {
                (
                    name.clone(),
                    Arc::new(crate::symbols::SymbolInfo {
                        ty: canonical,
                        kind: symbol.kind,
                        function_signature: symbol.function_signature.clone(),
                    }),
                )
            })
        })
        .collect();
    for (name, symbol) in replacements {
        let _ = table.insert_shared(name, symbol);
    }
}

fn dedup_symbol_handle_type(
    symbol: &mut Option<Arc<crate::symbols::SymbolInfo>>,
    cache: &mut TypeDedupCache,
) {
    if let Some(existing) = symbol.as_ref()
        && let Some(canonical) = dedup_retained_type(&existing.ty, cache)
    {
        *symbol = Some(Arc::new(crate::symbols::SymbolInfo {
            ty: canonical,
            kind: existing.kind,
            function_signature: existing.function_signature.clone(),
        }));
    }
}

/// Collapses value-identical large types retained by a declaration module's
/// analysis. Dependency graphs routinely contain hundreds of modules whose
/// exported annotation resolves to the same shape (icon packs: one
/// `ForwardRefExoticComponent<SVGProps & …>` per icon file); when that shape
/// degrades (`had_error`) it is barred from the instantiation interner, so
/// every module otherwise retains a private multi-megabyte copy — the dominant
/// transient of the module-analysis passes on trpc/unnamed.
fn dedup_module_analysis_types(analysis: &mut ModuleAnalysis, cache: &mut TypeDedupCache) {
    dedup_symbol_table_types(&mut analysis.local_symbols, cache);
    dedup_symbol_table_types(&mut analysis.local_export_table.symbols, cache);
    dedup_symbol_handle_type(&mut analysis.local_export_table.default_symbol, cache);
    dedup_symbol_handle_type(
        &mut analysis.local_export_table.export_assignment_symbol,
        cache,
    );
}

fn retained_module_analysis_type_nodes(analysis: &ModuleAnalysis) -> u64 {
    let mut seen = HashSet::new();
    let mut count = 0;
    for (_, symbol) in analysis.local_symbols.iter_shared() {
        count += retained_type_nodes(&symbol.ty, &mut seen);
    }
    for (_, symbol) in analysis.local_export_table.symbols.iter_shared() {
        count += retained_type_nodes(&symbol.ty, &mut seen);
    }
    if let Some(symbol) = &analysis.local_export_table.default_symbol {
        count += retained_type_nodes(&symbol.ty, &mut seen);
    }
    if let Some(symbol) = &analysis.local_export_table.export_assignment_symbol {
        count += retained_type_nodes(&symbol.ty, &mut seen);
    }
    count
}

fn retained_type_nodes(ty: &Type, seen: &mut HashSet<usize>) -> u64 {
    match ty {
        Type::Object(object) => {
            let identity = Arc::as_ptr(&object.properties) as usize;
            if !seen.insert(identity) {
                return 0;
            }
            1 + object
                .properties
                .values()
                .map(|property| retained_type_nodes(&property.ty, seen))
                .sum::<u64>()
                + object
                    .string_index_type
                    .as_deref()
                    .map_or(0, |index| retained_type_nodes(index, seen))
                + object
                    .call_signature()
                    .map_or(0, |call| retained_function_nodes(call, seen))
                + object
                    .construct_signature()
                    .map_or(0, |construct| retained_function_nodes(construct, seen))
        }
        Type::Function(function) => retained_function_nodes(function, seen),
        Type::Array(element) => 1 + retained_type_nodes(element, seen),
        Type::Tuple(elements) => {
            let identity = elements.as_ptr() as usize;
            if !elements.is_empty() && !seen.insert(identity) {
                return 0;
            }
            1 + elements
                .iter()
                .map(|element| retained_type_nodes(element, seen))
                .sum::<u64>()
        }
        Type::Union(union) => {
            let types = union.types();
            let identity = types.as_ptr() as usize;
            if !types.is_empty() && !seen.insert(identity) {
                return 0;
            }
            1 + types
                .iter()
                .map(|member| retained_type_nodes(member, seen))
                .sum::<u64>()
        }
        Type::Reference(reference) => {
            let identity = reference.arguments.as_ptr() as usize;
            if !reference.arguments.is_empty() && !seen.insert(identity) {
                return 0;
            }
            1 + reference
                .arguments
                .iter()
                .map(|argument| retained_type_nodes(argument, seen))
                .sum::<u64>()
        }
        _ => 1,
    }
}

fn retained_function_nodes(
    function: &surge_ts_types::FunctionType,
    seen: &mut HashSet<usize>,
) -> u64 {
    let identity = function.parameters().as_ptr() as usize;
    if !function.parameters().is_empty() && !seen.insert(identity) {
        return 0;
    }
    1 + function
        .parameters()
        .iter()
        .map(|parameter| retained_type_nodes(parameter, seen))
        .sum::<u64>()
        + retained_type_nodes(function.return_type(), seen)
}

pub(crate) fn build_module_resolution_scopes(
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    module_import_bindings: &[Option<ModuleImportBindings>],
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> Vec<Option<Arc<TypeDeclarationScope>>> {
    local_type_declarations_by_module
        .iter()
        .enumerate()
        .map(|(file_index, local_type_declarations)| {
            let Some(local_type_declarations) = local_type_declarations else {
                return None;
            };

            let clone_start = Instant::now();
            let mut layers = vec![local_type_declarations.clone()];
            if let Some(imported) = module_import_bindings
                .get(file_index)
                .and_then(|bindings| bindings.as_ref())
            {
                layers.extend(imported.scope_layers());
            }
            record_program_timing(timings, |timings| {
                timings.clone_copy_heavy_operations += clone_start.elapsed()
            });
            Some(Arc::new(TypeDeclarationScope::new(layers)))
        })
        .collect()
}

pub(crate) fn collect_module_import_bindings(
    parsed_files: &[ParsedProgramFile],
    module_analyses: &[Option<ModuleAnalysis>],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) -> Vec<Option<ModuleImportBindings>> {
    let mut module_import_bindings = Vec::with_capacity(parsed_files.len());

    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !parsed_file.is_module && parsed_file.file_kind != FileKind::DependencyDeclaration {
            if !parsed_file.import_call_specifiers.is_empty() {
                ctx.set_file_name(parsed_file.file_name.clone());
                crate::modules::register_import_type_namespaces(
                    parsed_file,
                    parsed_files,
                    module_export_tables,
                    module_resolution_scopes,
                    ctx,
                );
            }
            module_import_bindings.push(None);
            continue;
        }

        let Some(module_analysis) = module_analyses[file_index].as_ref() else {
            module_import_bindings.push(None);
            continue;
        };

        ctx.set_file_name(parsed_file.file_name.clone());
        let imported_bindings = resolve_module_imports(
            parsed_file,
            parsed_files,
            module_export_tables,
            module_resolution_scopes,
            &|name| module_analysis.has_local_symbol(name),
            ctx,
        );
        module_import_bindings.push(Some(imported_bindings));
    }

    module_import_bindings
}

/// `SURGE_VALUE_EXPORT_REFINEMENT=1`: settle exported values whose initializer
/// reads an imported value, by re-analyzing a module once its imports have
/// types (see [`refine_imported_value_exports`]).
///
/// OFF by default: correct, but no gain yet, and costly. A module is re-offered
/// when its own value imports improved *or* any file it imports from — value or
/// type-only — changed; the type-only half is what reaches tRPC's
/// `utils/trpc.ts`, whose `import type { AppRouter }` carries no value symbol.
/// With it, the exported `createTRPCNext<AppRouter>` client is the right
/// `CreateTRPCNextBase & DecorateRouterRecord` intersection (verified by probe),
/// and trpc's diagnostics are byte-identical to the pass being off.
///
/// An earlier measurement credited this pass with 6 closed false negatives. That
/// was an artifact: its dependency map missed relative imports, so the client
/// was still published as tRPC's "collides with a built-in method" error string,
/// every `trpc.todo` read off it was a false TS2339, and error-type propagation
/// turned those into TS7006 reports that happened to land on tsc's lines.
///
/// What actually blocks those TS7006 is the router's root types, which surge has
/// as `{ ctx: unknown; errorShape: unknown; transformer: unknown }` where tsc has
/// a real context, `DefaultErrorShape` and `true` — `Unwrap<T>`'s `Awaited<R>`
/// leaking `R`, and `TOptions` not inferred through `create(opts?:
/// ValidateShape<TOptions, …>)` — plus `useQuery` itself degrading. Measured
/// cost on zod: about 7s to 53s wall, from re-analyzing its module graph.
fn value_export_refinement_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_VALUE_EXPORT_REFINEMENT").as_deref() == Ok("1"))
}

/// How many value hops past the first an exported initializer is followed
/// through. Each round settles the exports one import further from a module
/// whose values were already known; a chain longer than this keeps the sentinel.
const MAX_VALUE_EXPORT_REFINEMENT_ROUNDS: usize = 8;

/// The program files each module reads from, through any import — value or
/// type-only — or `export … from`. A type-only import still carries a type
/// whose meaning can change when its source settles: `import type { AppRouter }`
/// of a `typeof appRouter` alias has no value symbol at all, yet the client
/// built from it is decided by that router's key set.
fn module_dependency_indices(
    parsed_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Vec<Vec<usize>> {
    use surge_ts_syntax::ParsedExportDeclaration;
    parsed_files
        .iter()
        .map(|parsed_file| {
            let mut dependencies: Vec<usize> = Vec::new();
            // Resolved the way `try_resolve_module` does: a relative specifier
            // is resolved on the fly against the program's files, a bare one
            // through the resolver's per-importer map.
            let mut add = |specifier: &str| {
                let index = crate::modules::resolve_relative_module(
                    &parsed_file.file_name,
                    specifier,
                    parsed_files,
                    &ctx.module_file_index_by_identity,
                )
                .map(|resolution| resolution.resolved_file_index)
                .or_else(|| {
                    let resolved = ctx
                        .options
                        .resolved_module_for(&parsed_file.file_name, specifier)?;
                    let identity = crate::modules::canonical_file_identity(resolved);
                    ctx.module_file_index_by_identity
                        .get(identity.as_str())
                        .copied()
                });
                if let Some(index) = index
                    && !dependencies.contains(&index)
                {
                    dependencies.push(index);
                }
            };
            for statement in &parsed_file.statements {
                match statement {
                    ParsedStatement::ImportDeclaration(import) => add(&import.module_specifier),
                    ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                        ParsedExportDeclaration::Named {
                            module_specifier: Some(specifier),
                            ..
                        }
                        | ParsedExportDeclaration::All {
                            module_specifier: specifier,
                            ..
                        }
                        | ParsedExportDeclaration::Namespace {
                            module_specifier: specifier,
                            ..
                        } => add(specifier),
                        _ => {}
                    },
                    _ => {}
                }
            }
            dependencies
        })
        .collect()
}

/// Whether two export tables publish the same value surface. Display-inclusive
/// so a type that only *renders* differently still counts as a change; it is the
/// loop's termination test, so equality must mean "nothing left to settle".
fn module_export_tables_agree(published: &ModuleExportTable, refined: &ModuleExportTable) -> bool {
    fn value_fingerprint(symbols: &SymbolTable) -> Vec<(Arc<str>, u64)> {
        let mut entries: Vec<(Arc<str>, u64)> = symbols
            .iter()
            .map(|(name, symbol)| {
                (
                    name.clone(),
                    crate::speculative::display_type_fingerprint(&symbol.ty),
                )
            })
            .collect();
        entries.sort_unstable();
        entries
    }
    value_fingerprint(&published.symbols) == value_fingerprint(&refined.symbols)
        && published.default_symbol.as_ref().map(|s| {
            crate::speculative::display_type_fingerprint(&s.ty)
        }) == refined.default_symbol.as_ref().map(|s| {
            crate::speculative::display_type_fingerprint(&s.ty)
        })
}

fn degraded_value_count(symbols: &SymbolTable) -> usize {
    symbols
        .iter()
        .filter(|(_, symbol)| matches!(symbol.ty, Type::Unknown))
        .count()
}

/// Settles exported values whose initializer reads an *imported* value.
///
/// The final analysis round binds a module's imports from the preliminary
/// export tables, and those carry no initializer types, so
/// `export const procedure = t.procedure` — with `t` built from an import — is
/// published at the sentinel, and so is everything downstream of it
/// (`router({ … })`, `typeof appRouter`, the client built from that). tsc types
/// a symbol on demand and has no such horizon.
///
/// Only a source module that exports a sentinel-typed value *and* whose imports
/// lost sentinels since it was last analyzed is re-analyzed, and only its
/// sentinel-typed exports are replaced: the type surface, the signatures and
/// every export that already had a type stay exactly what the final round
/// produced.
pub(crate) fn refine_imported_value_exports(
    parsed_files: &[ParsedProgramFile],
    local_type_declarations_by_module: &[Option<Arc<TypeDeclarationTable>>],
    analyzed_import_bindings: &[Option<ModuleImportBindings>],
    module_analyses: &mut [Option<ModuleAnalysis>],
    module_export_tables: &mut Vec<Option<ModuleExportTable>>,
    module_import_bindings: &mut Vec<Option<ModuleImportBindings>>,
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) {
    let mut degraded_imports_when_analyzed: Vec<usize> = analyzed_import_bindings
        .iter()
        .map(|bindings| {
            bindings
                .as_ref()
                .map_or(0, |bindings| degraded_value_count(&bindings.symbols))
        })
        .collect();
    if !value_export_refinement_enabled() {
        return;
    }
    let dependencies = module_dependency_indices(parsed_files, ctx);
    let mut changed_last_round: Vec<bool> = vec![false; parsed_files.len()];
    let saved_file_name = ctx.file_name.clone();

    for _ in 0..MAX_VALUE_EXPORT_REFINEMENT_ROUNDS {
        let analysis_round = next_analysis_round();
        let mut refined_any = false;
        let mut changed_this_round: Vec<bool> = vec![false; parsed_files.len()];
        for file_index in 0..parsed_files.len() {
            let parsed_file = &parsed_files[file_index];
            if !parsed_file.is_module || parsed_file.file_kind.is_declaration() {
                continue;
            }
            if module_analyses[file_index].is_none() {
                continue;
            }
            let degraded_imports = module_import_bindings[file_index]
                .as_ref()
                .map_or(0, |bindings| degraded_value_count(&bindings.symbols));
            let imports_improved = degraded_imports < degraded_imports_when_analyzed[file_index];
            let dependency_changed = dependencies[file_index]
                .iter()
                .any(|dependency| changed_last_round[*dependency]);
            if !imports_improved && !dependency_changed {
                continue;
            }
            degraded_imports_when_analyzed[file_index] = degraded_imports;

            let diagnostics_before = ctx.diagnostics().len();
            let refined = crate::modules::exports::with_source_exports_sharing_environment_store(|| {
                analyze_module(
                    file_index,
                    parsed_file,
                    local_type_declarations_by_module,
                    module_import_bindings,
                    false,
                    false,
                    true,
                    analysis_round,
                    None,
                    ctx,
                    timings,
                )
            });
            ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
            let Some(refined) = refined else {
                continue;
            };
            let Some(analysis) = module_analyses[file_index].as_mut() else {
                continue;
            };
            // The whole table is installed, not just the exports that were at
            // the sentinel: a value can be published with a *wrong* type rather
            // than no type, because a conditional decided from an import that
            // had none (tRPC's `createTRPCNext<AppRouter>` publishes the
            // "your router collides with a built-in method" error string when
            // the router's key set is unknown). This round's inputs are
            // strictly better than the ones that produced the table it
            // replaces, so its answer is the better answer.
            if module_export_tables_agree(&analysis.local_export_table, &refined.local_export_table)
            {
                continue;
            }
            analysis.local_export_table = refined.local_export_table;
            changed_this_round[file_index] = true;
            refined_any = true;
        }
        if !refined_any {
            break;
        }
        changed_last_round = changed_this_round;

        let local_module_export_tables = module_analyses
            .iter()
            .map(|analysis| {
                analysis
                    .as_ref()
                    .map(|analysis| analysis.local_export_table.clone())
            })
            .collect::<Vec<_>>();
        ctx.begin_resolution_stage();
        *module_export_tables =
            resolve_module_export_tables(parsed_files, &local_module_export_tables, ctx);
        refresh_reexported_namespace_objects(parsed_files, module_export_tables);
        *module_import_bindings = collect_module_import_bindings(
            parsed_files,
            module_analyses,
            module_export_tables,
            module_resolution_scopes,
            ctx,
        );
        // `typeof <value>` in a consumer resolves through this map, so a module
        // whose exports just settled must be visible in it before the next
        // round reads them — a type-only import of a `typeof`-backed alias
        // (`import type { AppRouter }`) reaches the value this way and by no
        // other.
        super::build_module_local_values(
            parsed_files,
            module_analyses,
            module_import_bindings,
            ctx,
        );
    }
    ctx.set_file_name(saved_file_name.to_string());
}

pub(crate) fn should_replay_preliminary_diagnostic(
    diagnostic: &Diagnostic,
    ctx: &CheckerContext,
) -> bool {
    if diagnostic.code.to_string() != "TS2305" {
        return false;
    }

    let Some(module_specifier) = diagnostic
        .message
        .strip_prefix("Module '")
        .and_then(|message| {
            message
                .split_once("' has no exported member ")
                .map(|(module_specifier, _)| module_specifier)
        })
    else {
        return true;
    };

    crate::modules::imports::ambient_module_export_table(ctx, module_specifier).is_some()
}

pub(crate) fn merge_module_import_bindings(
    final_bindings: &[Option<ModuleImportBindings>],
    fallback_bindings: &[Option<ModuleImportBindings>],
) -> Vec<Option<ModuleImportBindings>> {
    final_bindings
        .iter()
        .zip(fallback_bindings.iter())
        .map(|(final_bindings, fallback_bindings)| {
            let Some(final_bindings) = final_bindings else {
                return surge_ts_types::with_type_copy_reason(
                    surge_ts_types::TypeCopyReason::ModuleExport,
                    || fallback_bindings.clone(),
                );
            };

            let Some(fallback_bindings) = fallback_bindings else {
                return Some(
                    final_bindings.clone_with_reason(surge_ts_types::TypeCopyReason::ModuleExport),
                );
            };

            let mut merged_type_declarations = final_bindings.type_declarations.as_ref().clone();
            record_type_declaration_table_merge(
                None,
                fallback_bindings.type_declarations.len(),
                TableMergeKind::General,
            );
            for (name, declaration) in fallback_bindings.type_declarations.iter() {
                if merged_type_declarations.get(name.as_ref()).is_none() {
                    let _ = merged_type_declarations.insert(name.clone(), declaration.clone());
                }
            }
            let mut merged_bindings =
                final_bindings.clone_with_reason(surge_ts_types::TypeCopyReason::ModuleExport);
            merged_bindings.type_declarations = Arc::new(merged_type_declarations);
            for (name, symbol) in fallback_bindings.symbols.iter_shared() {
                if merged_bindings.symbols.get(name).is_none() {
                    let _ = merged_bindings
                        .symbols
                        .insert_shared(name.clone(), symbol.clone());
                }
            }

            Some(merged_bindings)
        })
        .collect()
}

/// An element write with a non-literal key on a function the statements
/// declare (`fn[key] = v`): the expando member it declares is named by the
/// key's type (`lateBindMember`).
fn statements_write_computed_function_members(statements: &[ParsedStatement]) -> bool {
    let functions: Vec<&str> = statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                    ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
                    _ => None,
                },
                ParsedExportDeclaration::Default {
                    declaration: surge_ts_syntax::ParsedDefaultExportDeclaration::Function(function),
                    ..
                } => Some(function.name.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    if functions.is_empty() {
        return false;
    }
    let computed = |assignment: &surge_ts_syntax::ParsedMemberAssignment| {
        matches!(
            &assignment.target,
            surge_ts_syntax::ParsedExpression::IndexAccess { object_name, index, .. }
                if functions.contains(&object_name.as_str())
                    && !matches!(
                        index.as_ref(),
                        surge_ts_syntax::ParsedExpression::StringLiteral(_)
                            | surge_ts_syntax::ParsedExpression::NumberLiteral(_)
                    )
        )
    };
    statements.iter().any(|statement| {
        let mut nested = Vec::new();
        match statement {
            ParsedStatement::MemberAssignment(assignment) => return computed(&**assignment),
            ParsedStatement::If(if_statement) => {
                crate::modules::exports::collect_nested_member_assignments(&if_statement.then_body, &[], &mut nested);
                crate::modules::exports::collect_nested_member_assignments(&if_statement.else_body, &[], &mut nested);
            }
            ParsedStatement::Block(body) => {
                crate::modules::exports::collect_nested_member_assignments(body, &[], &mut nested)
            }
            _ => {}
        }
        nested.into_iter().any(|assignment| computed(assignment))
    })
}

fn statements_declare_class_with_heritage(statements: &[ParsedStatement]) -> bool {
    statements.iter().any(|statement| match statement {
        ParsedStatement::ClassDeclaration(class) => !class.extends.is_empty(),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                statements_declare_class_with_heritage(std::slice::from_ref(declaration))
            }
            ParsedExportDeclaration::Default {
                declaration: surge_ts_syntax::ParsedDefaultExportDeclaration::Class(class),
                ..
            } => !class.extends.is_empty(),
            _ => false,
        },
        _ => false,
    })
}
