mod global_alloc;
mod memory_watchdog;
mod probe;
mod report;
mod run_report;

use std::io::IsTerminal;
use std::time::Instant;
use std::{fs, path::PathBuf, process::ExitCode};

use clap::{Error, Parser, error::ErrorKind};
use report::{
    ReportFormat, build_project_compatibility_report, render_project_compatibility_report_json,
    render_project_compatibility_report_text, render_project_diagnostics_json,
    render_project_diagnostics_preview,
};
use serde_json::{Map, Value};
use surge_ts::{Checker, CheckerOptions, LibSourceChoice, Project, ProjectOptions, ProjectTimings};
use surge_ts_config::canonicalize_if_exists_string;
use surge_ts_diagnostics::{
    Diagnostic, DiagnosticCode, TscRenderItem, TscRenderOptions, render_diagnostics,
    render_diagnostics_tsc,
};

/// Selects how diagnostics are rendered for human-readable (non-`--format json`)
/// output. `tsc` is the default and mirrors the TypeScript compiler's text
/// output; `custom` preserves the project's original Rust-style report; `json`
/// is the machine-readable form (equivalent to `--format json`).
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CliDiagnosticStyle {
    Tsc,
    Custom,
    Json,
}

#[derive(Debug, Clone, Copy)]
enum DiagnosticStyle {
    Tsc,
    Custom,
    Json,
}

/// Controls the multi-line `tsc`-style code-frame output. `auto` follows the
/// terminal (pretty when stdout is a TTY, like `tsc`).
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum PrettyMode {
    #[value(name = "true")]
    Always,
    #[value(name = "false")]
    Never,
    #[value(name = "auto")]
    Auto,
}

fn resolve_diagnostic_style(
    style: Option<CliDiagnosticStyle>,
    format: Option<ReportFormat>,
) -> DiagnosticStyle {
    match style {
        Some(CliDiagnosticStyle::Tsc) => DiagnosticStyle::Tsc,
        Some(CliDiagnosticStyle::Custom) => DiagnosticStyle::Custom,
        Some(CliDiagnosticStyle::Json) => DiagnosticStyle::Json,
        // Back-compat: `--format json` keeps emitting JSON (the oracle harness
        // relies on it). Plain `--format text` and the unset default both map to
        // the new tsc-compatible renderer.
        None => match format {
            Some(ReportFormat::Json) => DiagnosticStyle::Json,
            _ => DiagnosticStyle::Tsc,
        },
    }
}

fn resolve_pretty(mode: PrettyMode) -> bool {
    match mode {
        PrettyMode::Always => true,
        PrettyMode::Never => false,
        PrettyMode::Auto => std::io::stdout().is_terminal(),
    }
}

fn resolve_color(pretty: bool) -> bool {
    if !pretty {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("FORCE_COLOR").is_some() {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// The display label `tsc` would use for a file: relative to the current working
/// directory (with forward slashes), climbing with `..` out of it as
/// `convertToRelativePath` does. Empty for diagnostics with no real file
/// (globals, command-line diagnostics).
fn tsc_path_label(file_name: &str) -> String {
    if file_name.is_empty() || file_name == "<command line>" {
        return String::new();
    }
    let path = std::path::Path::new(file_name);
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.to_string_lossy().replace('\\', "/");
        }
        if let Some(relative) = relative_path_outside(&cwd, path) {
            return relative;
        }
    }
    file_name.replace('\\', "/")
}

/// tsc's `getRelativePathToDirectoryOrUrl` for a file outside `directory`:
/// climb to their common ancestor with `..`, then down to the file. A file on
/// another root keeps its absolute path.
fn relative_path_outside(directory: &std::path::Path, path: &std::path::Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let directory: Vec<_> = directory.components().collect();
    let target: Vec<_> = path.components().collect();
    let common = directory
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 || !matches!(directory.first(), Some(std::path::Component::RootDir | std::path::Component::Prefix(_))) {
        return None;
    }
    let mut segments = vec!["..".to_string(); directory.len() - common];
    segments.extend(
        target[common..]
            .iter()
            .map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );
    Some(segments.join("/"))
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum CliDiagnosticProfile {
    Tsc,
    Native,
}

impl Into<surge_ts_checker::DiagnosticProfile> for CliDiagnosticProfile {
    fn into(self) -> surge_ts_checker::DiagnosticProfile {
        match self {
            CliDiagnosticProfile::Tsc => surge_ts_checker::DiagnosticProfile::Tsc,
            CliDiagnosticProfile::Native => surge_ts_checker::DiagnosticProfile::Native,
        }
    }
}

#[derive(Debug, Default)]
struct CliTimings {
    config_project_loading: std::time::Duration,
    file_discovery: std::time::Duration,
    default_lib_loading: std::time::Duration,
    package_declaration_discovery: std::time::Duration,
    import_graph_expansion: std::time::Duration,
    path_mapping_resolution: std::time::Duration,
    checking: std::time::Duration,
    diagnostic_rendering: std::time::Duration,
    total: std::time::Duration,
    source_read_io: std::time::Duration,
    source_files_read: u64,
    source_bytes_read: u64,
    default_lib_files_read: u64,
    default_lib_bytes_read: u64,
    default_lib_read_io: std::time::Duration,
    default_lib_existence_probes: u64,
    default_lib_canonicalize_syscalls: u64,
    expansion_read_io: std::time::Duration,
    expansion_files_read: u64,
    expansion_bytes_read: u64,
    package_json_reads: u64,
    package_declaration_read_io: std::time::Duration,
    package_declaration_probes: u64,
    package_declaration_probe_io: std::time::Duration,
    fs_existence_probes: u64,
    fs_existence_probe_io: std::time::Duration,
    fs_read_dir_count: u64,
    fs_read_dir_io: std::time::Duration,
    canonicalize_memo_misses: u64,
    canonicalize_full_realpaths: u64,
    canonicalize_leaf_probes: u64,
    canonicalize_miss_io: std::time::Duration,
}

#[derive(Debug, Parser)]
#[command(author, version, about, disable_help_subcommand = true)]
struct Cli {
    #[arg(value_name = "FILE")]
    file_path: Option<PathBuf>,

    #[arg(short, long, value_name = "TSCONFIG")]
    project: Option<PathBuf>,

    #[arg(long = "showConfig")]
    show_config: bool,

    #[arg(long = "showSpans")]
    show_spans: bool,

    #[arg(long = "ignoreConfig")]
    ignore_config: bool,

    #[arg(long = "compatReport")]
    compat_report: bool,

    /// Print an extended run-statistics block (files, phase times, memory) to
    /// stderr after the normal diagnostics output.
    #[arg(long = "extendedDiagnostics")]
    extended_diagnostics: bool,

    /// Print a memory-focused report (physical footprint, peak RSS) to stderr
    /// after the normal diagnostics output.
    #[arg(long = "memoryReport")]
    memory_report: bool,

    /// Write a versioned machine-readable run report (JSON) to the given file.
    #[arg(long = "reportJson", value_name = "PATH")]
    report_json: Option<PathBuf>,

    #[arg(long, value_enum)]
    format: Option<ReportFormat>,

    #[arg(
        long = "diagnosticStyle",
        visible_alias = "diagnostic-style",
        value_enum
    )]
    diagnostic_style: Option<CliDiagnosticStyle>,

    #[arg(long, value_enum)]
    pretty: Option<PrettyMode>,

    #[arg(long = "diagnosticProfile", value_enum)]
    diagnostic_profile: Option<CliDiagnosticProfile>,

    #[arg(long = "maxDiagnostics")]
    max_diagnostics: Option<usize>,

    /// Worker threads for project checking: `auto` (default) sizes by available
    /// cores and workload, `1` forces serial, or pass an explicit count.
    #[arg(long, value_parser = parse_jobs, value_name = "auto|N")]
    jobs: Option<usize>,

    #[arg(long = "stubExternalModules")]
    stub_external_modules: bool,

    #[arg(long)]
    no_implicit_any: bool,

    #[arg(long = "noLib")]
    no_lib: bool,

    /// Load the standard library from the project's installed TypeScript
    /// instead of the bundled snapshot, discovered by walking up from the
    /// project root to a `node_modules/typescript/lib`. Also settable via a
    /// `.physicalLibs` marker file beside the resolved `tsconfig.json` or the
    /// `SURGE_PHYSICAL_LIBS` env var. A warning is emitted if no such package
    /// is found; the bundled snapshot is used in that case.
    #[arg(long = "physicalLibs")]
    physical_libs: bool,

    /// Load the standard library from this directory of `lib.*.d.ts` files
    /// instead of the bundled snapshot. Takes precedence over `--physicalLibs`.
    #[arg(long = "typescript-lib-path", value_name = "DIR")]
    typescript_lib_path: Option<PathBuf>,

    #[arg(long, hide = true)]
    timings: bool,

    /// Per-stage RSS profiling without the full timings/counters report.
    #[arg(long, hide = true)]
    rss: bool,
}

/// Checking recurses with the program (nested lazy annotation forces, deep
/// member chains); the main thread's default 8 MB stack overflowed on tRPC once
/// `typeof import("…")` queries resolved. The reservation is virtual, so a
/// large stack costs nothing until it is used.
const CHECK_THREAD_STACK_BYTES: usize = 512 << 20;

fn main() -> ExitCode {
    if let Err(error) = memory_watchdog::start_from_env() {
        eprintln!("error: {error}");
        return ExitCode::from(2);
    }
    let handle = std::thread::Builder::new()
        .name("surge-check".into())
        .stack_size(CHECK_THREAD_STACK_BYTES)
        .spawn(main_on_check_thread)
        .expect("spawn the checking thread");
    match handle.join() {
        Ok(code) => code,
        Err(_) => ExitCode::FAILURE,
    }
}

fn main_on_check_thread() -> ExitCode {
    // Benchmark-harness self-check: report the compiled-in global allocator and
    // exit without checking anything. Env-var gated so the CLI surface is
    // unchanged.
    if std::env::var_os("SURGE_PRINT_ALLOCATOR").is_some() {
        println!("{}", global_alloc::ACTIVE_ALLOCATOR);
        return ExitCode::SUCCESS;
    }

    let cli = Cli::parse();

    if cli.timings {
        unsafe {
            std::env::set_var("SURGE_TIMINGS", "1");
        }
    }

    if cli.rss {
        unsafe {
            std::env::set_var("SURGE_RSS", "1");
        }
    }

    if cli.max_diagnostics == Some(0) {
        Error::raw(
            ErrorKind::InvalidValue,
            "--maxDiagnostics must be greater than 0",
        )
        .exit();
    }

    if cli.project.is_some() && cli.file_path.is_some() {
        Error::raw(
            ErrorKind::ArgumentConflict,
            "cannot use a positional file path together with --project",
        )
        .exit();
    }

    let report_request = run_report::ReportRequest {
        extended: cli.extended_diagnostics,
        memory: cli.memory_report,
        json_path: cli.report_json.clone(),
    };

    if cli.show_config && report_request.any() {
        let flag = if cli.extended_diagnostics {
            "--extendedDiagnostics"
        } else if cli.memory_report {
            "--memoryReport"
        } else {
            "--reportJson"
        };
        Error::raw(
            ErrorKind::ArgumentConflict,
            format!("{flag} cannot be used with --showConfig"),
        )
        .exit();
    }

    if cli.project.is_some() {
        if cli.ignore_config {
            Error::raw(
                ErrorKind::ArgumentConflict,
                "--ignoreConfig cannot be used with --project",
            )
            .exit();
        }
        let style = resolve_diagnostic_style(cli.diagnostic_style, cli.format);
        let pretty = resolve_pretty(cli.pretty.unwrap_or(PrettyMode::Auto));
        let color = resolve_color(pretty);
        return run_project_mode(
            cli.project.unwrap(),
            cli.show_config,
            cli.show_spans,
            cli.compat_report,
            style,
            pretty,
            color,
            cli.max_diagnostics,
            cli.jobs.unwrap_or(0),
            cli.stub_external_modules,
            cli.diagnostic_profile
                .unwrap_or(CliDiagnosticProfile::Tsc)
                .into(),
            cli.physical_libs,
            cli.typescript_lib_path,
            cli.timings,
            report_request,
        );
    }

    if cli.show_config {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "--showConfig requires --project",
        )
        .exit();
    }

    if cli.jobs.is_some() {
        Error::raw(
            ErrorKind::ArgumentConflict,
            "--jobs is only supported with --project",
        )
        .exit();
    }

    if cli.compat_report {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "--compatReport requires --project",
        )
        .exit();
    }

    if cli.extended_diagnostics {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "--extendedDiagnostics requires --project",
        )
        .exit();
    }

    if cli.memory_report {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "--memoryReport requires --project",
        )
        .exit();
    }

    if cli.report_json.is_some() {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "--reportJson requires --project",
        )
        .exit();
    }

    let Some(file_path) = cli.file_path else {
        Error::raw(
            ErrorKind::MissingRequiredArgument,
            "expected a file path or --project",
        )
        .exit();
    };

    let style = resolve_diagnostic_style(cli.diagnostic_style, cli.format);
    let pretty = resolve_pretty(cli.pretty.unwrap_or(PrettyMode::Auto));
    let color = resolve_color(pretty);
    run_single_file_mode(
        file_path,
        cli.no_implicit_any,
        cli.no_lib,
        cli.stub_external_modules,
        cli.show_spans,
        style,
        pretty,
        color,
        cli.max_diagnostics,
        cli.ignore_config,
        cli.diagnostic_profile
            .unwrap_or(CliDiagnosticProfile::Tsc)
            .into(),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_single_file_mode(
    file_path: PathBuf,
    no_implicit_any: bool,
    no_lib: bool,
    stub_external_modules: bool,
    show_spans: bool,
    style: DiagnosticStyle,
    pretty: bool,
    color: bool,
    max_diagnostics: Option<usize>,
    ignore_config: bool,
    diagnostic_profile: surge_ts_checker::DiagnosticProfile,
) -> ExitCode {
    if !ignore_config
        && std::env::current_dir()
            .map(|dir| dir.join("tsconfig.json").exists())
            .unwrap_or(false)
    {
        let diagnostic = surge_ts_diagnostics::Diagnostic::ts5112("<command line>");
        let diagnostics = [diagnostic];
        match style {
            DiagnosticStyle::Json => {
                let json = serde_json::to_string_pretty(&render_single_file_diagnostics_json(
                    &file_path,
                    &diagnostics,
                    "",
                    max_diagnostics,
                ))
                .unwrap();
                println!("{}", json);
            }
            DiagnosticStyle::Custom => println!("{}", diagnostics[0].render("")),
            DiagnosticStyle::Tsc => print!(
                "{}",
                render_single_file_diagnostics_tsc(
                    &diagnostics,
                    "",
                    pretty,
                    color,
                    max_diagnostics
                )
            ),
        }
        return ExitCode::from(1);
    }
    let source_text = match fs::read_to_string(&file_path) {
        Ok(source_text) => source_text,
        Err(error) => {
            eprintln!("failed to read {}: {error}", file_path.display());
            return ExitCode::from(1);
        }
    };

    let file_name = canonicalize_if_exists_string(&file_path);

    let diagnostics = Checker::new()
        .options(CheckerOptions {
            no_implicit_any,
            no_implicit_this: no_implicit_any,
            module_emit: surge_ts_checker::ModuleEmitKind::ES2022,
            verbatim_module_syntax: false,
            isolated_modules: false,
            preserve_const_enums: false,
            use_define_for_class_fields: true,
            target_es2022: true,
            target_es2016: true,
            no_emit: false,
            node_module_resolution: false,
            esm_module_files: Default::default(),
            esm_package_files: Default::default(),
            strict_null_checks: true,
            // The single-file path has no tsconfig: `--noImplicitAny` stands in
            // for `strict`, which is what these derive from.
            strict_property_initialization: no_implicit_any,
            use_unknown_in_catch_variables: no_implicit_any,
            strict_bind_call_apply: no_implicit_any,
            strict_builtin_iterator_return: no_implicit_any,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            rewrite_relative_import_extensions: false,
            declaration: false,
            allow_arbitrary_extensions: false,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
            no_lib,
            skip_lib_check: false,
            stub_external_modules,
            resolved_modules: Default::default(),
            resolved_modules_by_importer: Default::default(),
            types: Vec::new(),
            jsx_automatic_runtime: false,
            jsx_classic_react: false,
            jsx_emit_none: false,
            allow_umd_global_access: false,
            resolve_json_module: true,
            allow_js: false,
            check_js: None,
            erasable_syntax_only: false,
            module_detection: Default::default(),
            language_version: Default::default(),
            jsx_configured: false,
            jsx_factory_names: Default::default(),
            diagnostic_profile,
        })
        .check_source(&source_text, &file_name);
    // `--showSpans` is a debug aid that forces the custom span renderer even
    // under the default tsc style; JSON output ignores it.
    let force_custom = matches!(style, DiagnosticStyle::Custom) || show_spans;
    match style {
        DiagnosticStyle::Json => println!(
            "{}",
            serde_json::to_string_pretty(&render_single_file_diagnostics_json(
                &file_path,
                &diagnostics,
                &source_text,
                max_diagnostics
            ))
            .unwrap()
        ),
        _ if force_custom => println!(
            "{}",
            render_single_file_diagnostics(
                &file_path,
                &diagnostics,
                &source_text,
                show_spans,
                max_diagnostics
            )
        ),
        DiagnosticStyle::Tsc => print!(
            "{}",
            render_single_file_diagnostics_tsc(
                &diagnostics,
                &source_text,
                pretty,
                color,
                max_diagnostics
            )
        ),
        DiagnosticStyle::Custom => unreachable!("custom handled by force_custom"),
    }

    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

/// Render single-file diagnostics in tsc-compatible form. The display label is
/// derived from each diagnostic's own file name, so command-line diagnostics
/// (with no real file) collapse to a header-only line.
fn render_single_file_diagnostics_tsc(
    diagnostics: &[Diagnostic],
    source_text: &str,
    pretty: bool,
    color: bool,
    max_diagnostics: Option<usize>,
) -> String {
    let total = diagnostics.len();
    if total == 0 {
        return String::new();
    }
    let limit = max_diagnostics.unwrap_or(total).min(total);
    let shown = &diagnostics[..limit];

    let labels: Vec<String> = shown
        .iter()
        .map(|diagnostic| tsc_path_label(&diagnostic.file_name))
        .collect();
    let items: Vec<TscRenderItem> = shown
        .iter()
        .zip(&labels)
        .map(|(diagnostic, label)| TscRenderItem {
            label,
            source_text,
            diagnostic,
        })
        .collect();

    let mut out = render_diagnostics_tsc(&items, TscRenderOptions { pretty, color });
    if limit < total {
        out.push_str(&format!(
            "\nShowing first {limit} of {total} diagnostics.\n"
        ));
    }
    out
}

/// Pick the standard-library source. An explicit directory wins over
/// `--physicalLibs` discovery, which in turn wins over the bundled snapshot, so
/// the selected source never depends on what happens to be installed unless the
/// user asked for it.
fn resolve_lib_source(
    typescript_lib_path: Option<PathBuf>,
    installed_typescript_requested: bool,
) -> LibSourceChoice {
    if let Some(dir) = typescript_lib_path {
        return LibSourceChoice::Directory(dir);
    }
    if installed_typescript_requested {
        return LibSourceChoice::InstalledTypeScript;
    }
    LibSourceChoice::Bundled
}

/// Whether the project's installed TypeScript was explicitly requested via the
/// `--physicalLibs` flag, a `.physicalLibs` marker file beside the resolved
/// `tsconfig.json`, or the `SURGE_PHYSICAL_LIBS` env var.
fn physical_libs_explicitly_requested(cli_flag: bool, config_path: &std::path::Path) -> bool {
    if cli_flag {
        return true;
    }
    if std::env::var_os("SURGE_PHYSICAL_LIBS").is_some() {
        return true;
    }
    config_path
        .parent()
        .map(|dir| dir.join(".physicalLibs").exists())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn run_project_mode(
    project: PathBuf,
    show_config: bool,
    show_spans: bool,
    compat_report: bool,
    style: DiagnosticStyle,
    pretty: bool,
    color: bool,
    max_diagnostics: Option<usize>,
    jobs: usize,
    stub_external_modules: bool,
    diagnostic_profile: surge_ts_checker::DiagnosticProfile,
    physical_libs_flag: bool,
    typescript_lib_path: Option<PathBuf>,
    timings_enabled: bool,
    report_request: run_report::ReportRequest,
) -> ExitCode {
    let mut timings = CliTimings::default();
    // The report flags reuse the existing `collect_timings` plumbing; only
    // `--timings` controls whether the CLI timings block itself is rendered.
    let collect_timings = timings_enabled || report_request.wants_timings();

    let run_start = Instant::now();
    let config_start = run_start;
    let project = Project::load(project);
    if collect_timings {
        timings.config_project_loading += config_start.elapsed();
    }
    let loaded = project.config();

    for diagnostic in project.config_diagnostics() {
        eprintln!("{diagnostic}");
    }

    if show_config {
        let config = build_show_config_json(loaded);
        println!("{}", serde_json::to_string_pretty(&config).unwrap());
        if timings_enabled {
            timings.total = run_start.elapsed();
            render_cli_timings(&timings);
        }
        return ExitCode::SUCCESS;
    }

    if loaded.files.is_empty() {
        // A removed option is a config error in its own right; tsc reports it
        // even when the project resolves to no files.
        let mut diagnostics = surge_ts::removed_option_diagnostics(loaded);
        diagnostics.push(project_has_no_source_files_diagnostic(loaded));
        let stats = surge_ts_checker::CompatibilityStats::default();
        let exit_code = render_project_mode_output(
            loaded,
            &diagnostics,
            &[],
            &stats,
            show_spans,
            compat_report,
            style,
            pretty,
            color,
            max_diagnostics,
            collect_timings,
            &mut timings,
        );
        if collect_timings {
            timings.total = run_start.elapsed();
        }
        if let Err(error) =
            run_report::emit_run_reports(&report_request, &[], diagnostics.len(), jobs, &timings)
        {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
        if timings_enabled {
            render_cli_timings(&timings);
        }
        return exit_code;
    }

    let options = ProjectOptions {
        jobs,
        stub_external_modules,
        diagnostic_profile,
        lib_source: resolve_lib_source(
            typescript_lib_path,
            physical_libs_explicitly_requested(physical_libs_flag, &loaded.config_path),
        ),
        collect_timings,
        // The compatibility report re-parses every source text.
        retain_all_sources: compat_report,
        fast_process_exit: !collect_timings,
    };

    let result = match project.check(&options) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    if collect_timings {
        merge_project_timings(&mut timings, &result.timings);
    }

    let exit_code = render_project_mode_output(
        loaded,
        &result.diagnostics,
        &result.sources,
        &result.stats,
        show_spans,
        compat_report,
        style,
        pretty,
        color,
        max_diagnostics,
        collect_timings,
        &mut timings,
    );
    if collect_timings {
        timings.total = run_start.elapsed();
    }
    if let Err(error) = run_report::emit_run_reports(
        &report_request,
        &result.sources,
        result.diagnostics.len(),
        jobs,
        &timings,
    ) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    if timings_enabled {
        render_cli_timings(&timings);
    }
    if options.fast_process_exit {
        // Everything the run produced has been written; dropping the parsed
        // program, the sources and the diagnostics only delays the exit.
        std::process::exit(if exit_code == ExitCode::SUCCESS { 0 } else { 2 });
    }
    exit_code
}

/// Fold the facade's per-phase project timings into the CLI's timing record,
/// which additionally tracks config loading, diagnostic rendering, and total.
fn merge_project_timings(timings: &mut CliTimings, project: &ProjectTimings) {
    timings.file_discovery += project.file_discovery;
    timings.default_lib_loading += project.default_lib_loading;
    timings.package_declaration_discovery += project.package_declaration_discovery;
    timings.import_graph_expansion += project.import_graph_expansion;
    timings.path_mapping_resolution += project.path_mapping_resolution;
    timings.checking += project.checking;
    timings.source_read_io += project.source_read_io;
    timings.source_files_read += project.source_files_read;
    timings.source_bytes_read += project.source_bytes_read;
    timings.default_lib_files_read += project.default_lib_files_read;
    timings.default_lib_bytes_read += project.default_lib_bytes_read;
    timings.default_lib_read_io += project.default_lib_read_io;
    timings.default_lib_existence_probes += project.default_lib_existence_probes;
    timings.default_lib_canonicalize_syscalls += project.default_lib_canonicalize_syscalls;
    timings.expansion_read_io += project.expansion_read_io;
    timings.expansion_files_read += project.expansion_files_read;
    timings.expansion_bytes_read += project.expansion_bytes_read;
    timings.package_json_reads += project.package_json_reads;
    timings.package_declaration_read_io += project.package_declaration_read_io;
    timings.package_declaration_probes += project.package_declaration_probes;
    timings.package_declaration_probe_io += project.package_declaration_probe_io;
    timings.fs_existence_probes += project.fs_existence_probes;
    timings.fs_existence_probe_io += project.fs_existence_probe_io;
    timings.fs_read_dir_count += project.fs_read_dir_count;
    timings.fs_read_dir_io += project.fs_read_dir_io;
    timings.canonicalize_memo_misses += project.canonicalize_memo_misses;
    timings.canonicalize_full_realpaths += project.canonicalize_full_realpaths;
    timings.canonicalize_leaf_probes += project.canonicalize_leaf_probes;
    timings.canonicalize_miss_io += project.canonicalize_miss_io;
}

/// `0` is the checker's sentinel for automatic worker-count selection, so `auto`
/// maps to it while a literal `0` from the user is still rejected.
fn parse_jobs(value: &str) -> Result<usize, String> {
    if value.eq_ignore_ascii_case("auto") {
        return Ok(0);
    }

    let jobs = value
        .parse::<usize>()
        .map_err(|_| format!("invalid value for --jobs: {value}"))?;
    if jobs == 0 {
        return Err("--jobs must be greater than 0".to_string());
    }

    Ok(jobs)
}

#[allow(clippy::too_many_arguments)]
fn render_project_mode_output(
    loaded: &surge_ts_config::LoadedTsConfig,
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
    stats: &surge_ts_checker::CompatibilityStats,
    show_spans: bool,
    compat_report: bool,
    style: DiagnosticStyle,
    pretty: bool,
    color: bool,
    max_diagnostics: Option<usize>,
    collect_timings: bool,
    timings: &mut CliTimings,
) -> ExitCode {
    let render_start = Instant::now();
    let json_output = matches!(style, DiagnosticStyle::Json);

    // The renderers resolve a diagnostic's line/column by looking its file up in
    // `sources`. A removed-option diagnostic lives in the config file, which is
    // not a source file, so it is appended for rendering only — the
    // compatibility report below still sees the real source list.
    let config_backed_sources = config_backed_sources(loaded, diagnostics, sources);
    let sources_for_diagnostics = config_backed_sources.as_deref().unwrap_or(sources);

    if compat_report {
        let report = build_project_compatibility_report(loaded, diagnostics, sources, stats);
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&render_project_compatibility_report_json(&report))
                    .unwrap()
            );
        } else {
            println!("{}", render_project_compatibility_report_text(&report));
            let preview = render_project_diagnostics_preview(
                diagnostics,
                sources_for_diagnostics,
                show_spans,
                max_diagnostics,
            );
            if !preview.is_empty() {
                println!();
                println!("{}", preview);
            }
        }
        if collect_timings {
            timings.diagnostic_rendering += render_start.elapsed();
        }
        return ExitCode::SUCCESS;
    }

    // `--showSpans` is a debug aid that forces the custom span renderer even
    // under the default tsc style.
    let force_custom = matches!(style, DiagnosticStyle::Custom) || show_spans;
    match style {
        DiagnosticStyle::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&render_project_diagnostics_json(
                    loaded,
                    diagnostics,
                    sources_for_diagnostics,
                    max_diagnostics
                ))
                .unwrap()
            );
        }
        _ if force_custom => {
            let preview = render_project_diagnostics_preview(
                diagnostics,
                sources_for_diagnostics,
                show_spans,
                max_diagnostics,
            );
            if !preview.is_empty() {
                println!("{}", preview);
            }
        }
        DiagnosticStyle::Tsc => {
            print!(
                "{}",
                render_project_diagnostics_tsc(
                    diagnostics,
                    sources_for_diagnostics,
                    pretty,
                    color,
                    max_diagnostics
                )
            );
        }
        DiagnosticStyle::Custom => unreachable!("custom handled by force_custom"),
    }

    if collect_timings {
        timings.diagnostic_rendering += render_start.elapsed();
    }

    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

/// Render project diagnostics in tsc-compatible form. Diagnostics are grouped by
/// file in source-load order (matching the custom preview and tsc's file
/// ordering); diagnostics whose file is not among the loaded sources (such as
/// global "Cannot find global type" diagnostics) are appended afterwards with no
/// source excerpt.
fn render_project_diagnostics_tsc(
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
    pretty: bool,
    color: bool,
    max_diagnostics: Option<usize>,
) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let mut by_file: std::collections::HashMap<&str, Vec<&Diagnostic>> =
        std::collections::HashMap::new();
    for diagnostic in diagnostics {
        by_file
            .entry(diagnostic.file_name.as_str())
            .or_default()
            .push(diagnostic);
    }

    let mut ordered: Vec<(String, &str, &Diagnostic)> = Vec::with_capacity(diagnostics.len());
    for (_, file_name, source_text) in sources {
        if let Some(file_diagnostics) = by_file.remove(file_name.as_str()) {
            let label = tsc_path_label(file_name);
            for diagnostic in file_diagnostics {
                ordered.push((label.clone(), source_text.as_str(), diagnostic));
            }
        }
    }
    // Any diagnostics not attached to a loaded source file, in original order.
    if !by_file.is_empty() {
        for diagnostic in diagnostics {
            if by_file.contains_key(diagnostic.file_name.as_str()) {
                ordered.push((tsc_path_label(&diagnostic.file_name), "", diagnostic));
            }
        }
    }

    let total = ordered.len();
    let limit = max_diagnostics.unwrap_or(total).min(total);
    let truncated = limit < total;
    ordered.truncate(limit);

    let items: Vec<TscRenderItem> = ordered
        .iter()
        .map(|(label, source_text, diagnostic)| TscRenderItem {
            label,
            source_text,
            diagnostic,
        })
        .collect();

    let mut out = render_diagnostics_tsc(&items, TscRenderOptions { pretty, color });
    if truncated {
        out.push_str(&format!(
            "\nShowing first {limit} of {total} diagnostics.\n"
        ));
    }
    out
}

fn render_cli_timings(timings: &CliTimings) {
    eprintln!("CLI timings:");
    eprintln!(
        "  config_project_loading: {}",
        format_duration(timings.config_project_loading)
    );
    eprintln!(
        "  file_discovery: {}",
        format_duration(timings.file_discovery)
    );
    eprintln!(
        "  default_lib_loading: {}",
        format_duration(timings.default_lib_loading)
    );
    eprintln!(
        "  package_declaration_discovery: {}",
        format_duration(timings.package_declaration_discovery)
    );
    eprintln!(
        "  import_graph_expansion: {}",
        format_duration(timings.import_graph_expansion)
    );
    eprintln!(
        "  path_mapping_resolution: {}",
        format_duration(timings.path_mapping_resolution)
    );
    eprintln!("  checking: {}", format_duration(timings.checking));
    eprintln!(
        "  diagnostic_rendering: {}",
        format_duration(timings.diagnostic_rendering)
    );
    let accounted = timings.config_project_loading
        + timings.file_discovery
        + timings.default_lib_loading
        + timings.package_declaration_discovery
        + timings.import_graph_expansion
        + timings.path_mapping_resolution
        + timings.checking
        + timings.diagnostic_rendering;
    eprintln!("  total: {}", format_duration(timings.total));
    eprintln!(
        "  unaccounted: {}",
        format_duration(timings.total.saturating_sub(accounted))
    );
    eprintln!("  io:");
    eprintln!(
        "    source_read_io: {}",
        format_duration(timings.source_read_io)
    );
    eprintln!("    source_files_read: {}", timings.source_files_read);
    eprintln!(
        "    source_bytes_read: {} ({})",
        timings.source_bytes_read,
        format_throughput(timings.source_bytes_read, timings.source_read_io)
    );
    eprintln!(
        "    default_lib_files_read: {}",
        timings.default_lib_files_read
    );
    eprintln!(
        "    default_lib_bytes_read: {}",
        timings.default_lib_bytes_read
    );
    eprintln!(
        "    default_lib_read_io: {}",
        format_duration(timings.default_lib_read_io)
    );
    eprintln!(
        "    default_lib_existence_probes: {}",
        timings.default_lib_existence_probes
    );
    eprintln!(
        "    default_lib_canonicalize_syscalls: {}",
        timings.default_lib_canonicalize_syscalls
    );
    eprintln!(
        "    expansion_read_io: {}",
        format_duration(timings.expansion_read_io)
    );
    eprintln!("    expansion_files_read: {}", timings.expansion_files_read);
    eprintln!(
        "    expansion_bytes_read: {} ({})",
        timings.expansion_bytes_read,
        format_throughput(timings.expansion_bytes_read, timings.expansion_read_io)
    );
    eprintln!("    package_json_reads: {}", timings.package_json_reads);
    eprintln!(
        "    package_declaration_read_io: {}",
        format_duration(timings.package_declaration_read_io)
    );
    eprintln!(
        "    package_declaration_probes: {}",
        timings.package_declaration_probes
    );
    eprintln!(
        "    package_declaration_probe_io: {}",
        format_duration(timings.package_declaration_probe_io)
    );
    eprintln!("    fs_existence_probes: {}", timings.fs_existence_probes);
    eprintln!(
        "    fs_existence_probe_io: {}",
        format_duration(timings.fs_existence_probe_io)
    );
    eprintln!("    fs_read_dir_count: {}", timings.fs_read_dir_count);
    eprintln!(
        "    fs_read_dir_io: {}",
        format_duration(timings.fs_read_dir_io)
    );
    eprintln!(
        "    canonicalize_memo_misses: {}",
        timings.canonicalize_memo_misses
    );
    eprintln!(
        "    canonicalize_full_realpaths: {}",
        timings.canonicalize_full_realpaths
    );
    eprintln!(
        "    canonicalize_leaf_probes: {}",
        timings.canonicalize_leaf_probes
    );
    eprintln!(
        "    canonicalize_miss_io: {}",
        format_duration(timings.canonicalize_miss_io)
    );
}

fn format_throughput(bytes: u64, elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 || bytes == 0 {
        return "-".to_string();
    }
    let mib_per_sec = (bytes as f64 / (1024.0 * 1024.0)) / seconds;
    format!("{mib_per_sec:.1} MiB/s")
}

fn format_duration(duration: std::time::Duration) -> String {
    format!("{:.3}ms", duration.as_secs_f64() * 1000.0)
}

fn render_single_file_diagnostics(
    file_path: &std::path::Path,
    diagnostics: &[surge_ts_diagnostics::Diagnostic],
    source_text: &str,
    show_spans: bool,
    max_diagnostics: Option<usize>,
) -> String {
    if diagnostics.is_empty() {
        return render_diagnostics(&[], source_text);
    }

    let total = diagnostics.len();
    let limit = max_diagnostics.unwrap_or(total).min(total);
    let truncated = limit < total;
    let diagnostics = &diagnostics[..limit];

    let rendered = if show_spans {
        render_diagnostics_with_spans(file_path, diagnostics, source_text)
    } else {
        render_diagnostics(diagnostics, source_text)
    };

    if truncated {
        format!(
            "{rendered}\n\nShowing first {} of {} diagnostics.",
            diagnostics.len(),
            total
        )
    } else {
        rendered
    }
}

fn render_single_file_diagnostics_json(
    file_path: &std::path::Path,
    diagnostics: &[surge_ts_diagnostics::Diagnostic],
    source_text: &str,
    max_diagnostics: Option<usize>,
) -> Value {
    let limit = max_diagnostics.unwrap_or(usize::MAX);
    let line_index = surge_ts_diagnostics::LineIndex::new(source_text);
    let diagnostics = diagnostics
        .iter()
        .take(limit)
        .map(|diagnostic| {
            build_single_file_diagnostic_json(file_path, diagnostic, source_text, &line_index)
        })
        .collect::<Vec<_>>();

    let mut root = Map::new();
    root.insert("diagnostics".to_string(), Value::Array(diagnostics));

    Value::Object(root)
}

fn build_single_file_diagnostic_json(
    file_path: &std::path::Path,
    diagnostic: &surge_ts_diagnostics::Diagnostic,
    source_text: &str,
    line_index: &surge_ts_diagnostics::LineIndex,
) -> Value {
    let mut item = Map::new();
    item.insert(
        "code".to_string(),
        Value::String(diagnostic.code.to_string()),
    );
    item.insert(
        "fileName".to_string(),
        if diagnostic.file_name == "<command line>" {
            Value::String(diagnostic.file_name.clone())
        } else {
            Value::String(file_path.display().to_string())
        },
    );
    item.insert(
        "message".to_string(),
        Value::String(diagnostic.message.clone()),
    );

    if let Some(span) = diagnostic.span {
        let mut span_json = Map::new();
        span_json.insert("start".to_string(), Value::from(span.start as u64));
        span_json.insert("end".to_string(), Value::from(span.end as u64));
        item.insert("span".to_string(), Value::Object(span_json));

        let (line, column) = line_index.line_col(source_text, span.start);
        item.insert("line".to_string(), Value::from(line as u64));
        item.insert("column".to_string(), Value::from(column as u64));
    }

    Value::Object(item)
}

fn build_show_config_json(loaded: &surge_ts_config::LoadedTsConfig) -> Value {
    let mut root = Map::new();
    root.insert(
        "configPath".to_string(),
        Value::String(loaded.config_path.display().to_string()),
    );
    root.insert(
        "rootDir".to_string(),
        Value::String(loaded.root_dir.display().to_string()),
    );
    root.insert(
        "files".to_string(),
        Value::Array(
            loaded
                .files
                .iter()
                .map(|path| Value::String(path.display().to_string()))
                .collect(),
        ),
    );
    root.insert(
        "compilerOptions".to_string(),
        build_compiler_options_json(&loaded.compiler_options),
    );

    Value::Object(root)
}

/// `sources` plus the config file, when a diagnostic points into the config and
/// the renderers would otherwise find no text to resolve its line/column
/// against. `None` leaves the caller on the original slice.
fn config_backed_sources(
    loaded: &surge_ts_config::LoadedTsConfig,
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
) -> Option<Vec<(PathBuf, String, String)>> {
    let file_name = loaded.config_path.display().to_string();
    if !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.file_name == file_name)
    {
        return None;
    }
    let source_text = fs::read_to_string(&loaded.config_path).ok()?;

    let mut extended = Vec::with_capacity(sources.len() + 1);
    extended.push((loaded.config_path.clone(), file_name, source_text));
    extended.extend(sources.iter().cloned());
    Some(extended)
}

fn project_has_no_source_files_diagnostic(loaded: &surge_ts_config::LoadedTsConfig) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::Custom("surge::project-has-no-source-files"),
        format!(
            "no source files were discovered for {}",
            loaded.config_path.display()
        ),
        loaded.config_path.display().to_string(),
    )
}

fn render_diagnostics_with_spans(
    file_path: &std::path::Path,
    diagnostics: &[surge_ts_diagnostics::Diagnostic],
    source_text: &str,
) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let line_index = surge_ts_diagnostics::LineIndex::new(source_text);
    diagnostics
        .iter()
        .map(|diagnostic| {
            render_diagnostic_with_spans(file_path, diagnostic, source_text, &line_index)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_diagnostic_with_spans(
    file_path: &std::path::Path,
    diagnostic: &surge_ts_diagnostics::Diagnostic,
    source_text: &str,
    line_index: &surge_ts_diagnostics::LineIndex,
) -> String {
    let mut header = format!("{} {}", diagnostic.code, file_path.display());

    if let Some(span) = diagnostic.span {
        let (line, column) = line_index.line_col(source_text, span.start);
        header.push_str(&format!(
            " start={} end={} line={} column={}",
            span.start, span.end, line, column
        ));
    } else {
        header.push_str(" (no span)");
    }

    format!(
        "{header}\n{}",
        diagnostic.render_with_line_index(source_text, line_index)
    )
}

fn build_compiler_options_json(
    compiler_options: &surge_ts_config::NormalizedCompilerOptions,
) -> Value {
    let mut options = Map::new();
    options.insert("strict".to_string(), Value::Bool(compiler_options.strict));
    options.insert(
        "noImplicitAny".to_string(),
        Value::Bool(compiler_options.no_implicit_any),
    );
    options.insert(
        "target".to_string(),
        Value::String(script_target_to_string(compiler_options.target).to_string()),
    );
    options.insert(
        "module".to_string(),
        Value::String(module_kind_to_string(compiler_options.module).to_string()),
    );
    options.insert(
        "moduleResolution".to_string(),
        Value::String(
            module_resolution_kind_to_string(compiler_options.module_resolution).to_string(),
        ),
    );
    options.insert(
        "allowJs".to_string(),
        Value::Bool(compiler_options.allow_js),
    );
    options.insert(
        "checkJs".to_string(),
        Value::Bool(compiler_options.check_js == Some(true)),
    );
    options.insert("noEmit".to_string(), Value::Bool(compiler_options.no_emit));
    options.insert(
        "skipLibCheck".to_string(),
        Value::Bool(compiler_options.skip_lib_check),
    );
    options.insert(
        "esModuleInterop".to_string(),
        Value::Bool(compiler_options.es_module_interop),
    );
    options.insert(
        "allowSyntheticDefaultImports".to_string(),
        Value::Bool(compiler_options.allow_synthetic_default_imports),
    );
    options.insert("noLib".to_string(), Value::Bool(compiler_options.no_lib));

    if let Some(jsx) = compiler_options.jsx.as_ref() {
        options.insert(
            "jsx".to_string(),
            Value::String(jsx_mode_to_string(jsx).to_string()),
        );
    }
    if !compiler_options.paths.is_empty() {
        options.insert(
            "paths".to_string(),
            Value::Object(
                compiler_options
                    .paths
                    .iter()
                    .map(|mapping| {
                        (
                            mapping.pattern.clone(),
                            Value::Array(
                                mapping
                                    .substitutions
                                    .iter()
                                    .map(|substitution| Value::String(substitution.clone()))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            ),
        );
    }
    if !compiler_options.lib.is_empty() {
        options.insert(
            "lib".to_string(),
            Value::Array(
                compiler_options
                    .lib
                    .iter()
                    .map(|lib| Value::String(lib.clone()))
                    .collect(),
            ),
        );
    }
    if !compiler_options.type_roots.is_empty() {
        options.insert(
            "typeRoots".to_string(),
            Value::Array(
                compiler_options
                    .type_roots
                    .iter()
                    .map(|path| Value::String(path.display().to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(types) = &compiler_options.types {
        options.insert(
            "types".to_string(),
            Value::Array(types.iter().map(|ty| Value::String(ty.clone())).collect()),
        );
    }

    Value::Object(options)
}

fn script_target_to_string(target: surge_ts_config::ScriptTarget) -> &'static str {
    match target {
        surge_ts_config::ScriptTarget::ES2015 => "es2015",
        surge_ts_config::ScriptTarget::ES2016 => "es2016",
        surge_ts_config::ScriptTarget::ES2017 => "es2017",
        surge_ts_config::ScriptTarget::ES2018 => "es2018",
        surge_ts_config::ScriptTarget::ES2019 => "es2019",
        surge_ts_config::ScriptTarget::ES2020 => "es2020",
        surge_ts_config::ScriptTarget::ES2021 => "es2021",
        surge_ts_config::ScriptTarget::ES2022 => "es2022",
        surge_ts_config::ScriptTarget::ES2023 => "es2023",
        surge_ts_config::ScriptTarget::ES2024 => "es2024",
        surge_ts_config::ScriptTarget::ES2025 => "es2025",
        surge_ts_config::ScriptTarget::ESNext => "esnext",
    }
}

fn module_kind_to_string(module: surge_ts_config::ModuleKind) -> &'static str {
    match module {
        surge_ts_config::ModuleKind::CommonJS => "commonjs",
        surge_ts_config::ModuleKind::ES2015 => "es2015",
        surge_ts_config::ModuleKind::ES2020 => "es2020",
        surge_ts_config::ModuleKind::ES2022 => "es2022",
        surge_ts_config::ModuleKind::ESNext => "esnext",
        surge_ts_config::ModuleKind::Node16 => "node16",
        surge_ts_config::ModuleKind::Node18 => "node18",
        surge_ts_config::ModuleKind::Node20 => "node20",
        surge_ts_config::ModuleKind::NodeNext => "nodenext",
        surge_ts_config::ModuleKind::Preserve => "preserve",
    }
}

fn module_resolution_kind_to_string(
    module_resolution: surge_ts_config::ModuleResolutionKind,
) -> &'static str {
    match module_resolution {
        surge_ts_config::ModuleResolutionKind::Node16 => "node16",
        surge_ts_config::ModuleResolutionKind::Node20 => "node20",
        surge_ts_config::ModuleResolutionKind::NodeNext => "nodenext",
        surge_ts_config::ModuleResolutionKind::Bundler => "bundler",
    }
}

fn jsx_mode_to_string(jsx: &surge_ts_config::JsxMode) -> &'static str {
    match jsx {
        surge_ts_config::JsxMode::Preserve => "preserve",
        surge_ts_config::JsxMode::React => "react",
        surge_ts_config::JsxMode::ReactJsx => "react-jsx",
        surge_ts_config::JsxMode::ReactJsxDev => "react-jsxdev",
        surge_ts_config::JsxMode::ReactNative => "react-native",
    }
}
