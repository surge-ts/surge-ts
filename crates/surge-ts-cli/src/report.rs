use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use surge_ts_checker::CompatibilityStats;
use surge_ts_config::LoadedTsConfig;
use surge_ts_diagnostics::{
    Diagnostic, DiagnosticCoverageStats, LineIndex, catalog_coverage_stats, render_diagnostics,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub struct CompatReportCountEntry {
    pub key: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct CompatReportParserErrorEntry {
    pub file_name: String,
    pub message: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct CompatReportBuildInfo {
    pub package_version: String,
    pub build_profile: String,
    pub binary_path: Option<String>,
    pub current_dir: Option<String>,
    pub workspace_root: String,
}

#[derive(Debug, Clone)]
pub struct ProjectCompatibilityReport {
    pub root_dir: String,
    pub files_loaded: usize,
    pub visibility_warning: Option<String>,
    pub build_info: CompatReportBuildInfo,
    pub diagnostics_total: usize,
    pub loaded_source_files: usize,
    pub loaded_root_declaration_files: usize,
    pub loaded_dependency_declaration_files: usize,
    pub loaded_generated_declaration_files: usize,
    pub suppressed_declaration_diagnostics_total: usize,
    pub suppressed_rust_only_diagnostics_total: usize,
    pub diagnostics_root_source_total: usize,
    pub diagnostics_root_declaration_total: usize,
    pub diagnostics_dependency_declaration_total: usize,
    pub diagnostics_generated_declaration_total: usize,
    pub diagnostics_by_file_kind: Vec<CompatReportCountEntry>,
    pub by_code: Vec<CompatReportCountEntry>,
    pub by_file: Vec<CompatReportCountEntry>,
    pub parser_errors: Vec<CompatReportParserErrorEntry>,
    pub external_module_stubs_total: usize,
    pub external_modules_unresolved_total: usize,
    pub declaration_files_loaded: usize,
    pub ambient_external_modules: Vec<String>,
    pub diagnostic_coverage: DiagnosticCoverageStats,
}

fn is_relative_specifier(specifier: &str) -> bool {
    specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with(".\\")
        || specifier.starts_with("..\\")
}

pub fn build_project_compatibility_report(
    loaded: &LoadedTsConfig,
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
    stats: &CompatibilityStats,
) -> ProjectCompatibilityReport {
    let mut by_code = HashMap::<String, usize>::new();
    let mut by_file = HashMap::<String, usize>::new();
    let mut parser_errors = HashMap::<(String, String), usize>::new();
    let mut external_module_stubs_total = 0;
    let mut declaration_files_loaded = 0;
    let mut loaded_source_files = 0;
    let mut loaded_root_declaration_files = 0;
    let mut loaded_dependency_declaration_files = 0;
    let mut loaded_generated_declaration_files = 0;
    let mut diagnostics_root_source_total = 0;
    let mut diagnostics_root_declaration_total = 0;
    let mut diagnostics_dependency_declaration_total = 0;
    let mut diagnostics_generated_declaration_total = 0;
    let mut diagnostics_by_file_kind = HashMap::<String, usize>::new();
    let mut ambient_external_modules_set = std::collections::HashSet::new();

    let mut parser = surge_ts_syntax::ParserWorker::new();
    for (_, file_name, source_text) in sources {
        match classify_file_kind(file_name) {
            FileKindLabel::RootSource => loaded_source_files += 1,
            FileKindLabel::RootDeclaration => loaded_root_declaration_files += 1,
            FileKindLabel::DependencyDeclaration => loaded_dependency_declaration_files += 1,
            FileKindLabel::GeneratedDeclaration => loaded_generated_declaration_files += 1,
        }

        if is_declaration_file_name(file_name)
            && classify_file_kind(file_name) != FileKindLabel::GeneratedDeclaration
        {
            declaration_files_loaded += 1;
        }
        let parsed = parser.parse(source_text, file_name);
        for statement in parsed.statements {
            match statement {
                surge_ts_syntax::ParsedStatement::ImportDeclaration(import) => {
                    if !is_relative_specifier(&import.module_specifier) {
                        external_module_stubs_total += 1;
                    }
                }
                surge_ts_syntax::ParsedStatement::DeclareModuleDeclaration(declare_module) => {
                    ambient_external_modules_set.insert(declare_module.module_specifier.clone());
                }
                surge_ts_syntax::ParsedStatement::ExportDeclaration(export) => {
                    let module_specifier = match export.as_ref() {
                        surge_ts_syntax::ParsedExportDeclaration::Named {
                            module_specifier,
                            ..
                        } => module_specifier.as_deref(),
                        surge_ts_syntax::ParsedExportDeclaration::All {
                            module_specifier, ..
                        } => Some(module_specifier.as_str()),
                        _ => None,
                    };
                    if let Some(spec) = module_specifier {
                        if !is_relative_specifier(spec) {
                            external_module_stubs_total += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    for diagnostic in diagnostics {
        let code = diagnostic.code.to_string();
        *by_code.entry(code.clone()).or_default() += 1;
        let file_label = report_path_label(&loaded.root_dir, &diagnostic.file_name);
        *by_file.entry(file_label.clone()).or_default() += 1;

        match classify_file_kind(&diagnostic.file_name) {
            FileKindLabel::RootSource => diagnostics_root_source_total += 1,
            FileKindLabel::RootDeclaration => diagnostics_root_declaration_total += 1,
            FileKindLabel::DependencyDeclaration => diagnostics_dependency_declaration_total += 1,
            FileKindLabel::GeneratedDeclaration => diagnostics_generated_declaration_total += 1,
        }
        *diagnostics_by_file_kind
            .entry(
                classify_file_kind(&diagnostic.file_name)
                    .label()
                    .to_string(),
            )
            .or_default() += 1;

        // Parse failures carry tsc's own code; `surge::parser-error` is left
        // only for one the parser could not number.
        let is_parser_error = match diagnostic.code {
            surge_ts_diagnostics::DiagnosticCode::TypeScript(number) => {
                surge_ts_diagnostics::is_tsc_parser_code(number)
            }
            _ => code == "surge::parser-error",
        };
        if is_parser_error {
            *parser_errors
                .entry((file_label, diagnostic.message.clone()))
                .or_default() += 1;
        }
    }

    ProjectCompatibilityReport {
        root_dir: loaded.root_dir.display().to_string(),
        files_loaded: loaded.files.len(),
        visibility_warning: if loaded.files.is_empty() {
            Some("no source files were discovered for the project".to_string())
        } else {
            None
        },
        build_info: build_report_build_info(),
        diagnostics_total: diagnostics.len(),
        loaded_source_files,
        loaded_root_declaration_files,
        loaded_dependency_declaration_files,
        loaded_generated_declaration_files,
        suppressed_declaration_diagnostics_total: stats.suppressed_declaration_diagnostics_total,
        suppressed_rust_only_diagnostics_total: stats.suppressed_rust_only_diagnostics_total,
        diagnostics_root_source_total,
        diagnostics_root_declaration_total,
        diagnostics_dependency_declaration_total,
        diagnostics_generated_declaration_total,
        diagnostics_by_file_kind: sort_counts(diagnostics_by_file_kind),
        by_code: sort_counts(by_code),
        by_file: sort_counts(by_file),
        parser_errors: sort_parser_errors(parser_errors),
        external_module_stubs_total,
        external_modules_unresolved_total: stats.external_modules_unresolved_total,
        declaration_files_loaded,
        ambient_external_modules: {
            let mut list: Vec<_> = ambient_external_modules_set.into_iter().collect();
            list.sort();
            list
        },
        diagnostic_coverage: catalog_coverage_stats(),
    }
}

pub fn render_project_diagnostics_json(
    loaded: &LoadedTsConfig,
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
    max_diagnostics: Option<usize>,
) -> Value {
    let limit = max_diagnostics.unwrap_or(usize::MAX);
    let mut source_by_file: HashMap<&str, &str> = HashMap::with_capacity(sources.len());
    for (_, source_file_name, source_text) in sources {
        source_by_file
            .entry(source_file_name.as_str())
            .or_insert(source_text.as_str());
    }
    let mut line_indices: HashMap<&str, LineIndex> = HashMap::new();
    let diagnostics = diagnostics
        .iter()
        .take(limit)
        .map(|diagnostic| {
            render_diagnostic_json(loaded, diagnostic, &source_by_file, &mut line_indices)
        })
        .collect::<Vec<_>>();

    let mut root = Map::new();
    root.insert("diagnostics".to_string(), Value::Array(diagnostics));

    Value::Object(root)
}

pub fn render_project_compatibility_report_text(report: &ProjectCompatibilityReport) -> String {
    let mut lines = Vec::new();
    lines.push("Compatibility report".to_string());
    lines.push(format!("Root dir: {}", report.root_dir));
    lines.push(format!("Files loaded: {}", report.files_loaded));
    lines.push("Build info:".to_string());
    lines.push(format!(
        "  package version: {}",
        report.build_info.package_version
    ));
    lines.push(format!(
        "  build profile: {}",
        report.build_info.build_profile
    ));
    lines.push(format!(
        "  binary path: {}",
        report
            .build_info
            .binary_path
            .as_deref()
            .unwrap_or("(unavailable)")
    ));
    lines.push(format!(
        "  current dir: {}",
        report
            .build_info
            .current_dir
            .as_deref()
            .unwrap_or("(unavailable)")
    ));
    lines.push(format!(
        "  workspace root: {}",
        report.build_info.workspace_root
    ));
    lines.push(format!(
        "Loaded source files: {}",
        report.loaded_source_files
    ));
    lines.push(format!(
        "Loaded root declarations: {}",
        report.loaded_root_declaration_files
    ));
    lines.push(format!(
        "Loaded dependency declarations: {}",
        report.loaded_dependency_declaration_files
    ));
    lines.push(format!(
        "Loaded generated declarations: {}",
        report.loaded_generated_declaration_files
    ));
    if let Some(warning) = &report.visibility_warning {
        lines.push(format!("Visibility warning: {warning}"));
    }
    lines.push(format!("Diagnostics: {}", report.diagnostics_total));
    lines.push(format!(
        "Suppressed declaration diagnostics: {}",
        report.suppressed_declaration_diagnostics_total
    ));
    lines.push(format!(
        "Suppressed Rust-only diagnostics: {}",
        report.suppressed_rust_only_diagnostics_total
    ));
    lines.push(String::new());
    lines.push("Diagnostics by file kind:".to_string());
    if report.diagnostics_by_file_kind.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for entry in &report.diagnostics_by_file_kind {
            lines.push(format!("{}  {}", entry.key, entry.count));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Diagnostics from root source files: {}",
        report.diagnostics_root_source_total
    ));
    lines.push(format!(
        "Diagnostics from root declaration files: {}",
        report.diagnostics_root_declaration_total
    ));
    lines.push(format!(
        "Diagnostics from dependency declarations: {}",
        report.diagnostics_dependency_declaration_total
    ));
    lines.push(format!(
        "Diagnostics from generated declarations: {}",
        report.diagnostics_generated_declaration_total
    ));
    lines.push(String::new());
    lines.push("Diagnostic coverage:".to_string());
    lines.push(format!(
        "  catalog total: {}",
        report.diagnostic_coverage.catalog_total
    ));
    lines.push(format!(
        "  emitted total: {}",
        report.diagnostic_coverage.emitted_total
    ));
    lines.push(format!(
        "  catalog-only total: {}",
        report.diagnostic_coverage.catalog_only_total
    ));
    lines.push(format!(
        "  emitted TypeScript diagnostics: {}",
        report.diagnostic_coverage.emitted_typescript_total
    ));
    lines.push(format!(
        "  catalog-only TypeScript diagnostics: {}",
        report.diagnostic_coverage.catalog_only_typescript_total
    ));
    lines.push(String::new());
    lines.push("By code:".to_string());
    if report.by_code.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for entry in &report.by_code {
            lines.push(format!("{}  {}", entry.key, entry.count));
        }
    }
    lines.push(String::new());
    lines.push("By file:".to_string());
    if report.by_file.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for entry in &report.by_file {
            lines.push(format!("{}  {}", entry.key, entry.count));
        }
    }

    lines.push(String::new());
    lines.push(format!("Parser errors: {}", parser_error_total(report)));
    lines.push("Top parser errors:".to_string());
    if report.parser_errors.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for entry in &report.parser_errors {
            lines.push(format!(
                "{}: {}  {}",
                entry.file_name, entry.message, entry.count
            ));
        }
    }

    lines.push(String::new());
    let unresolved_externals = report
        .external_modules_unresolved_total
        .min(report.external_module_stubs_total);
    lines.push(format!(
        "External module stubs: {} (resolved: {}, unresolved: {})",
        report.external_module_stubs_total,
        report.external_module_stubs_total - unresolved_externals,
        unresolved_externals
    ));

    if report.declaration_files_loaded > 0 {
        lines.push(String::new());
        lines.push(format!(
            "Declaration files loaded: {}",
            report.declaration_files_loaded
        ));
        if !report.ambient_external_modules.is_empty() {
            lines.push(format!(
                "Ambient external modules: {}",
                report.ambient_external_modules.len()
            ));
        }
    }

    lines.join("\n")
}

pub fn render_project_compatibility_report_json(report: &ProjectCompatibilityReport) -> Value {
    let mut root = Map::new();
    root.insert(
        "rootDir".to_string(),
        Value::String(report.root_dir.clone()),
    );
    root.insert(
        "filesLoaded".to_string(),
        Value::from(report.files_loaded as u64),
    );
    root.insert("buildInfo".to_string(), {
        let mut item = Map::new();
        item.insert(
            "packageVersion".to_string(),
            Value::String(report.build_info.package_version.clone()),
        );
        item.insert(
            "buildProfile".to_string(),
            Value::String(report.build_info.build_profile.clone()),
        );
        if let Some(binary_path) = &report.build_info.binary_path {
            item.insert("binaryPath".to_string(), Value::String(binary_path.clone()));
        }
        if let Some(current_dir) = &report.build_info.current_dir {
            item.insert("currentDir".to_string(), Value::String(current_dir.clone()));
        }
        item.insert(
            "workspaceRoot".to_string(),
            Value::String(report.build_info.workspace_root.clone()),
        );
        Value::Object(item)
    });
    if let Some(warning) = &report.visibility_warning {
        root.insert(
            "visibilityWarning".to_string(),
            Value::String(warning.clone()),
        );
    }
    root.insert(
        "diagnosticsTotal".to_string(),
        Value::from(report.diagnostics_total as u64),
    );
    root.insert(
        "loadedSourceFiles".to_string(),
        Value::from(report.loaded_source_files as u64),
    );
    root.insert(
        "loadedRootDeclarationFiles".to_string(),
        Value::from(report.loaded_root_declaration_files as u64),
    );
    root.insert(
        "loadedDependencyDeclarationFiles".to_string(),
        Value::from(report.loaded_dependency_declaration_files as u64),
    );
    root.insert(
        "loadedGeneratedDeclarationFiles".to_string(),
        Value::from(report.loaded_generated_declaration_files as u64),
    );
    root.insert(
        "suppressedDeclarationDiagnosticsTotal".to_string(),
        Value::from(report.suppressed_declaration_diagnostics_total as u64),
    );
    root.insert(
        "suppressedRustOnlyDiagnosticsTotal".to_string(),
        Value::from(report.suppressed_rust_only_diagnostics_total as u64),
    );
    root.insert(
        "diagnosticsRootSourceTotal".to_string(),
        Value::from(report.diagnostics_root_source_total as u64),
    );
    root.insert(
        "diagnosticsRootDeclarationTotal".to_string(),
        Value::from(report.diagnostics_root_declaration_total as u64),
    );
    root.insert(
        "diagnosticsDependencyDeclarationTotal".to_string(),
        Value::from(report.diagnostics_dependency_declaration_total as u64),
    );
    root.insert(
        "diagnosticsGeneratedDeclarationTotal".to_string(),
        Value::from(report.diagnostics_generated_declaration_total as u64),
    );
    root.insert(
        "diagnosticsByFileKind".to_string(),
        Value::Array(
            report
                .diagnostics_by_file_kind
                .iter()
                .map(|entry| {
                    let mut item = Map::new();
                    item.insert("kind".to_string(), Value::String(entry.key.clone()));
                    item.insert("count".to_string(), Value::from(entry.count as u64));
                    Value::Object(item)
                })
                .collect(),
        ),
    );

    let mut coverage_map = Map::new();
    coverage_map.insert(
        "catalogTotal".to_string(),
        Value::from(report.diagnostic_coverage.catalog_total as u64),
    );
    coverage_map.insert(
        "emittedTotal".to_string(),
        Value::from(report.diagnostic_coverage.emitted_total as u64),
    );
    coverage_map.insert(
        "catalogOnlyTotal".to_string(),
        Value::from(report.diagnostic_coverage.catalog_only_total as u64),
    );
    coverage_map.insert(
        "emittedTypeScriptTotal".to_string(),
        Value::from(report.diagnostic_coverage.emitted_typescript_total as u64),
    );
    coverage_map.insert(
        "catalogOnlyTypeScriptTotal".to_string(),
        Value::from(report.diagnostic_coverage.catalog_only_typescript_total as u64),
    );
    root.insert(
        "diagnosticCoverage".to_string(),
        Value::Object(coverage_map),
    );

    root.insert(
        "byCode".to_string(),
        Value::Array(
            report
                .by_code
                .iter()
                .map(|entry| {
                    let mut item = Map::new();
                    item.insert("code".to_string(), Value::String(entry.key.clone()));
                    item.insert("count".to_string(), Value::from(entry.count as u64));
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    root.insert(
        "byFile".to_string(),
        Value::Array(
            report
                .by_file
                .iter()
                .map(|entry| {
                    let mut item = Map::new();
                    item.insert("fileName".to_string(), Value::String(entry.key.clone()));
                    item.insert("count".to_string(), Value::from(entry.count as u64));
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    root.insert(
        "parserErrors".to_string(),
        Value::Array(
            report
                .parser_errors
                .iter()
                .map(|entry| {
                    let mut item = Map::new();
                    item.insert(
                        "fileName".to_string(),
                        Value::String(entry.file_name.clone()),
                    );
                    item.insert("message".to_string(), Value::String(entry.message.clone()));
                    item.insert("count".to_string(), Value::from(entry.count as u64));
                    Value::Object(item)
                })
                .collect(),
        ),
    );

    let mut stubs_json = Map::new();
    let unresolved = report
        .external_modules_unresolved_total
        .min(report.external_module_stubs_total);
    stubs_json.insert(
        "total".to_string(),
        Value::from(report.external_module_stubs_total as u64),
    );
    stubs_json.insert("unresolved".to_string(), Value::from(unresolved as u64));
    stubs_json.insert(
        "resolved".to_string(),
        Value::from((report.external_module_stubs_total - unresolved) as u64),
    );
    root.insert("externalModuleStubs".to_string(), Value::Object(stubs_json));

    if report.declaration_files_loaded > 0 {
        root.insert(
            "declarationFilesLoaded".to_string(),
            Value::from(report.declaration_files_loaded as u64),
        );
        root.insert(
            "ambientExternalModules".to_string(),
            Value::Array(
                report
                    .ambient_external_modules
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }

    Value::Object(root)
}

fn build_report_build_info() -> CompatReportBuildInfo {
    let package_version = env!("CARGO_PKG_VERSION").to_string();
    let build_profile = option_env!("PROFILE").unwrap_or("unknown").to_string();
    let binary_path = env::current_exe()
        .ok()
        .map(|path| path.display().to_string());
    let current_dir = env::current_dir()
        .ok()
        .map(|path| path.display().to_string());
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .display()
        .to_string();

    CompatReportBuildInfo {
        package_version,
        build_profile,
        binary_path,
        current_dir,
        workspace_root,
    }
}

pub fn render_project_diagnostics_preview(
    diagnostics: &[Diagnostic],
    sources: &[(PathBuf, String, String)],
    show_spans: bool,
    max_diagnostics: Option<usize>,
) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let mut diagnostics_by_file: HashMap<String, Vec<Diagnostic>> = HashMap::new();
    for diagnostic in diagnostics {
        diagnostics_by_file
            .entry(diagnostic.file_name.clone())
            .or_default()
            .push(diagnostic.clone());
    }

    let limit = max_diagnostics.unwrap_or(usize::MAX);
    let mut rendered = Vec::new();
    let mut rendered_count = 0usize;
    let mut truncated = false;

    for (file_path, file_name, source_text) in sources {
        if rendered_count >= limit {
            truncated = diagnostics.len() > rendered_count;
            break;
        }

        let Some(file_diagnostics) = diagnostics_by_file.remove(file_name) else {
            continue;
        };

        let block = render_diagnostic_block(
            file_path.as_path(),
            source_text,
            &file_diagnostics,
            show_spans,
            limit.saturating_sub(rendered_count),
            &mut rendered_count,
        );

        if !block.is_empty() {
            rendered.push(block);
        }

        if rendered_count >= limit {
            truncated = diagnostics.len() > rendered_count || !diagnostics_by_file.is_empty();
            break;
        }
    }

    if !diagnostics_by_file.is_empty() && rendered_count < limit {
        let mut remaining = diagnostics_by_file.into_iter().collect::<Vec<_>>();
        remaining.sort_by(|left, right| left.0.cmp(&right.0));

        for (file_name, file_diagnostics) in remaining {
            if rendered_count >= limit {
                truncated = diagnostics.len() > rendered_count;
                break;
            }

            let block = render_diagnostic_block(
                Path::new(&file_name),
                "",
                &file_diagnostics,
                show_spans,
                limit.saturating_sub(rendered_count),
                &mut rendered_count,
            );

            if !block.is_empty() {
                rendered.push(block);
            }
        }

        truncated = truncated || diagnostics.len() > rendered_count;
    }

    if truncated {
        rendered.push(format!(
            "Showing first {} of {} diagnostics.",
            rendered_count,
            diagnostics.len()
        ));
    }

    rendered.join("\n\n")
}

fn render_diagnostic_block(
    file_path: &Path,
    source_text: &str,
    diagnostics: &[Diagnostic],
    show_spans: bool,
    remaining_limit: usize,
    rendered_count: &mut usize,
) -> String {
    if diagnostics.is_empty() || remaining_limit == 0 {
        return String::new();
    }

    let take = remaining_limit.min(diagnostics.len());
    let diagnostics = &diagnostics[..take];
    *rendered_count += take;

    let rendered = if show_spans {
        render_diagnostics_with_spans(diagnostics, source_text)
    } else {
        render_diagnostics(diagnostics, source_text)
    };

    format!("{}\n{}", file_path.display(), rendered)
}

fn render_diagnostics_with_spans(diagnostics: &[Diagnostic], source_text: &str) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    let line_index = LineIndex::new(source_text);
    diagnostics
        .iter()
        .map(|diagnostic| render_diagnostic_with_span(diagnostic, source_text, &line_index))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_diagnostic_with_span(
    diagnostic: &Diagnostic,
    source_text: &str,
    line_index: &LineIndex,
) -> String {
    let mut header = format!("{} {}", diagnostic.code, diagnostic.file_name);

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

fn render_diagnostic_json<'a>(
    loaded: &LoadedTsConfig,
    diagnostic: &Diagnostic,
    source_by_file: &HashMap<&'a str, &'a str>,
    line_indices: &mut HashMap<&'a str, LineIndex>,
) -> Value {
    let mut item = Map::new();
    item.insert(
        "code".to_string(),
        Value::String(diagnostic.code.to_string()),
    );
    item.insert(
        "fileName".to_string(),
        Value::String(report_path_label(&loaded.root_dir, &diagnostic.file_name)),
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

        if let Some((source_file_name, source_text)) =
            source_by_file.get_key_value(diagnostic.file_name.as_str())
        {
            let line_index = line_indices
                .entry(source_file_name)
                .or_insert_with(|| LineIndex::new(source_text));
            let (line, column) = line_index.line_col(source_text, span.start);
            item.insert("line".to_string(), Value::from(line as u64));
            item.insert("column".to_string(), Value::from(column as u64));
        }
    }

    Value::Object(item)
}

fn sort_counts(counts: HashMap<String, usize>) -> Vec<CompatReportCountEntry> {
    let mut entries = counts
        .into_iter()
        .map(|(key, count)| CompatReportCountEntry { key, count })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    entries
}

fn sort_parser_errors(
    counts: HashMap<(String, String), usize>,
) -> Vec<CompatReportParserErrorEntry> {
    let mut entries = counts
        .into_iter()
        .map(
            |((file_name, message), count)| CompatReportParserErrorEntry {
                file_name,
                message,
                count,
            },
        )
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.file_name.cmp(&right.file_name))
            .then_with(|| left.message.cmp(&right.message))
    });
    entries
}

fn parser_error_total(report: &ProjectCompatibilityReport) -> usize {
    report.parser_errors.iter().map(|entry| entry.count).sum()
}

fn report_path_label(root_dir: &Path, file_name: &str) -> String {
    let path = Path::new(file_name);
    path.strip_prefix(root_dir)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| file_name.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKindLabel {
    RootSource,
    RootDeclaration,
    DependencyDeclaration,
    GeneratedDeclaration,
}

impl FileKindLabel {
    fn label(self) -> &'static str {
        match self {
            FileKindLabel::RootSource => "root-source",
            FileKindLabel::RootDeclaration => "root-declaration",
            FileKindLabel::DependencyDeclaration => "dependency-declaration",
            FileKindLabel::GeneratedDeclaration => "generated-declaration",
        }
    }
}

fn classify_file_kind(file_name: &str) -> FileKindLabel {
    let lower = file_name.to_ascii_lowercase();
    let is_decl = is_declaration_file_name(file_name);

    if is_decl {
        if lower.contains("/.nuxt/")
            || lower.contains("/.generated/")
            || lower.contains("/generated-libs/")
            || lower.contains("/generated/")
            || lower.contains("/dist/")
            || lower.ends_with(".generated.d.ts")
            || lower.ends_with(".generated.d.mts")
            || lower.ends_with(".generated.d.cts")
            // Default libs are infrastructure, not project or dependency
            // declarations; bucket every backend together so project metrics
            // stay stable regardless of where the libs came from.
            || lower.starts_with("<surge-lib>/")
            || lower.contains("/typescript/lib/lib.")
        {
            return FileKindLabel::GeneratedDeclaration;
        }

        if lower.contains("/node_modules/") || lower.contains("/node_modules/.pnpm/") {
            return FileKindLabel::DependencyDeclaration;
        }

        return FileKindLabel::RootDeclaration;
    }

    FileKindLabel::RootSource
}

fn is_declaration_file_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".d.ts") || lower.ends_with(".d.mts") || lower.ends_with(".d.cts")
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_source_has_no_classifier_terms() {
        let report_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/report.rs");
        let source = std::fs::read_to_string(report_path).expect("report source");
        let implementation_source = source.split("#[cfg(test)]").next().unwrap_or(&source);
        for needle in [
            "CategorizedCountEntry",
            "nodeModulesSourceDiagnostics",
            "nodeModulesJavaScriptSourceDiagnostics",
            "candidate",
            "category",
        ] {
            assert!(
                !implementation_source.contains(&needle),
                "report.rs still contains banned classifier text: {needle}"
            );
        }
    }
}
