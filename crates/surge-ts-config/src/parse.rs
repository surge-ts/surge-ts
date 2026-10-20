use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value};

use crate::diagnostics::{ConfigDiagnostic, ConfigDiagnosticCode};
use crate::extends::load_merged_config;
use crate::files::resolve_source_files;
use crate::model::{LoadedTsConfig, TsConfigLoadOptions};
use crate::normalize::normalize_compiler_options;
use crate::paths::resolve_project_path;

#[derive(Debug, Clone, Default)]
pub(crate) struct RawTsConfig {
    pub(crate) compiler_options: Option<Map<String, Value>>,
    pub(crate) files: Option<Vec<Value>>,
    pub(crate) include: Option<Vec<Value>>,
    pub(crate) exclude: Option<Vec<Value>>,
}

pub fn load_tsconfig(options: TsConfigLoadOptions) -> LoadedTsConfig {
    let (config_path, root_dir) = resolve_project_path(&options.project);
    let mut diagnostics = Vec::new();
    let mut visited = HashSet::new();

    let merged = load_merged_config(&config_path, &mut diagnostics, &mut visited);
    let project_references = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| parse_config_text(&text).ok())
        .map(|value| referenced_config_files(&value, &root_dir))
        .unwrap_or_default();
    finish_loaded_config(config_path, root_dir, merged, diagnostics, project_references)
}

/// The config file of each project a config's `references` name (`{ "path":
/// "projects/shared" }`, a directory or its config file), resolved against
/// the config's directory, and of those the referenced projects reference in
/// turn, a parent before its own. `references` is never inherited through
/// `extends`.
fn referenced_config_files(config: &Value, config_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    collect_project_references(config, config_dir, &mut found);
    found
}

fn collect_project_references(config: &Value, config_dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    let Some(references) = config.get("references").and_then(Value::as_array) else {
        return;
    };
    for path in references.iter().filter_map(|reference| reference.get("path").and_then(Value::as_str)) {
        collect_project_reference(&crate::resolve_path(config_dir, path), found);
    }
}

/// One reference's config file (`path` is a directory or a config file) and,
/// read from it, those it references in turn.
fn collect_project_reference(path: &Path, found: &mut Vec<std::path::PathBuf>) {
    let resolved = crate::normalize_path_buf(path);
    let (directory, config_file) = if resolved.extension().is_some_and(|extension| extension == "json") {
        (resolved.parent().map(Path::to_path_buf).unwrap_or_else(|| resolved.clone()), resolved)
    } else {
        (resolved.clone(), resolved.join("tsconfig.json"))
    };
    if found.contains(&config_file) {
        return;
    }
    found.push(config_file.clone());
    if let Some(referenced) = std::fs::read_to_string(&config_file).ok().and_then(|text| parse_config_text(&text).ok()) {
        collect_project_references(&referenced, &directory, found);
    }
}

/// The config files of the projects `references` name (a program's
/// `projectReferences`, each a directory or config file path), and of those
/// they reference in turn, a parent before its own.
pub fn project_reference_config_files(references: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for reference in references {
        collect_project_reference(reference, &mut found);
    }
    found
}

/// A config the caller already parsed: what `load_tsconfig` makes of a file,
/// made of `value` as if it were the file at `config_path` (its `extends`
/// and `include` resolve against `config_path`'s directory). Also returns the
/// merged `compilerOptions` as written, before normalization.
pub fn load_tsconfig_from_value(value: &Value, config_path: &Path) -> (LoadedTsConfig, Map<String, Value>) {
    let root_dir = config_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut diagnostics = Vec::new();
    let mut visited = HashSet::new();
    let merged = match value.as_object() {
        Some(object) => crate::extends::merge_config_object(config_path, object, &mut diagnostics, &mut visited),
        None => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::ConfigParseError,
                message: "tsconfig.json must contain a JSON object".to_string(),
                file_name: config_path.to_path_buf(),
            });
            RawTsConfig::default()
        }
    };
    let raw_options = merged.compiler_options.clone().unwrap_or_default();
    let project_references = referenced_config_files(value, &root_dir);
    (finish_loaded_config(config_path.to_path_buf(), root_dir, merged, diagnostics, project_references), raw_options)
}

/// A `compilerOptions` object normalized as a config at `config_dir` would
/// be: for callers holding options without a config file.
pub fn normalize_compiler_options_json(
    options: &Map<String, Value>,
    config_dir: &Path,
) -> (crate::model::NormalizedCompilerOptions, Vec<ConfigDiagnostic>) {
    let mut diagnostics = Vec::new();
    let normalized = normalize_compiler_options(Some(options), config_dir, &mut diagnostics);
    (normalized, diagnostics)
}

/// Parses a config file's text the way `load_tsconfig` does: JSON with
/// comments and trailing commas.
pub fn parse_config_text(text: &str) -> Result<Value, String> {
    jsonc_parser::parse_to_serde_value::<Value>(text, &Default::default()).map_err(|error| error.to_string())
}

fn finish_loaded_config(
    config_path: std::path::PathBuf,
    root_dir: std::path::PathBuf,
    merged: RawTsConfig,
    mut diagnostics: Vec<ConfigDiagnostic>,
    project_references: Vec<std::path::PathBuf>,
) -> LoadedTsConfig {
    let compiler_options = normalize_compiler_options(
        merged.compiler_options.as_ref(),
        &root_dir,
        &mut diagnostics,
    );
    let files = resolve_source_files(
        &root_dir,
        merged.files.as_ref(),
        merged.include.as_ref(),
        merged.exclude.as_ref(),
        &compiler_options,
        &mut diagnostics,
    );

    let removed_options = crate::removed_options::collect_removed_options(
        &config_path,
        merged.compiler_options.as_ref(),
    );

    LoadedTsConfig {
        config_path,
        root_dir,
        files,
        compiler_options,
        diagnostics,
        removed_options,
        project_references,
    }
}

pub(crate) fn parse_current_config(
    config_path: &Path,
    object: &Map<String, Value>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> RawTsConfig {
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let compiler_options = match object.get("compilerOptions") {
        Some(Value::Object(options)) => Some(absolutize_path_options(options.clone(), config_dir)),
        Some(_) => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: "`compilerOptions` must be a JSON object".to_string(),
                file_name: config_path.to_path_buf(),
            });
            None
        }
        None => None,
    };

    let files = parse_string_array_entry(
        object.get("files"),
        config_path,
        ConfigDiagnosticCode::InvalidFilesEntry,
        diagnostics,
    );
    let include = parse_string_array_entry(
        object.get("include"),
        config_path,
        ConfigDiagnosticCode::InvalidIncludeEntry,
        diagnostics,
    );
    let exclude = parse_string_array_entry(
        object.get("exclude"),
        config_path,
        ConfigDiagnosticCode::InvalidExcludeEntry,
        diagnostics,
    );

    RawTsConfig {
        compiler_options,
        files,
        include,
        exclude,
    }
}

/// tsc's `convertToOptionsWithAbsolutePaths`: a path option is relative to
/// the config that writes it, which an extending config's directory must not
/// replace. `paths` targets resolve against that config's directory too
/// (`pathsBasePath`), recorded beside them so it merges as they do. A
/// `${configDir}` path names the root config's directory, so it is left as
/// written.
fn absolutize_path_options(mut options: Map<String, Value>, config_dir: &Path) -> Map<String, Value> {
    let absolute = |raw: &str| {
        if raw.starts_with("${configDir}") {
            raw.to_string()
        } else {
            crate::paths::resolve_path(config_dir, raw).to_string_lossy().into_owned()
        }
    };
    for key in ["baseUrl", "rootDir", "outDir", "declarationDir", "tsBuildInfoFile", "outFile"] {
        if let Some(Value::String(raw)) = options.get(key) {
            let rebased = absolute(raw);
            options.insert(key.to_string(), Value::String(rebased));
        }
    }
    for key in ["rootDirs", "typeRoots"] {
        if let Some(Value::Array(items)) = options.get_mut(key) {
            for item in items.iter_mut() {
                if let Value::String(raw) = item {
                    *raw = absolute(raw);
                }
            }
        }
    }
    if options.contains_key("paths") {
        options.insert("pathsBasePath".to_string(), Value::String(config_dir.to_string_lossy().into_owned()));
    }
    options
}

pub(crate) fn parse_string_array_entry(
    value: Option<&Value>,
    config_path: &Path,
    code: ConfigDiagnosticCode,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<Vec<Value>> {
    let Some(value) = value else {
        return None;
    };

    let Some(items) = value.as_array() else {
        diagnostics.push(ConfigDiagnostic {
            code,
            message: "expected an array of strings".to_string(),
            file_name: config_path.to_path_buf(),
        });
        return None;
    };

    Some(items.clone())
}

pub(crate) fn merge_configs(base: Option<&RawTsConfig>, child: &RawTsConfig) -> RawTsConfig {
    let mut compiler_options = base
        .and_then(|base| base.compiler_options.clone())
        .unwrap_or_default();
    if let Some(child_options) = &child.compiler_options {
        compiler_options.extend(child_options.clone());
    }

    RawTsConfig {
        compiler_options: if compiler_options.is_empty() {
            None
        } else {
            Some(compiler_options)
        },
        files: child
            .files
            .clone()
            .or_else(|| base.and_then(|base| base.files.clone())),
        include: child
            .include
            .clone()
            .or_else(|| base.and_then(|base| base.include.clone())),
        exclude: child
            .exclude
            .clone()
            .or_else(|| base.and_then(|base| base.exclude.clone())),
    }
}
