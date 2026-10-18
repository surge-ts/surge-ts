//! `tsconfig.json` loading: `readConfigFile`, `parseJsonConfigFileContent`,
//! `findConfigFile`. The parsing, `extends` merging, option normalization
//! and file discovery are `surge-ts-config`'s, the same code the CLI runs.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::diagnostics::{Diagnostic, DiagnosticCategory};
use super::options::CompilerOptions;

/// TypeScript's `ParsedCommandLine`, kept to what a config yields.
#[derive(Clone, Debug)]
pub struct ParsedCommandLine {
    pub options: CompilerOptions,
    pub file_names: Vec<String>,
    pub errors: Vec<Diagnostic>,
    pub raw: Value,
}

fn cannot_read_file(file_name: &str) -> Diagnostic {
    Diagnostic {
        file: None,
        start: None,
        length: None,
        code: 5083,
        category: DiagnosticCategory::Error,
        message_text: format!("Cannot read file '{file_name}'."),
        surge_code: None,
        file_name: None,
    }
}

/// `parseConfigFileTextToJson`: a config's text as JSON (comments and
/// trailing commas allowed).
pub fn parse_config_file_text(file_name: &str, text: &str) -> Result<Value, Diagnostic> {
    surge_ts_config::parse_config_text(text).map_err(|message| Diagnostic {
        file: None,
        start: None,
        length: None,
        code: 0,
        category: DiagnosticCategory::Error,
        message_text: message,
        surge_code: Some("ConfigParseError".to_string()),
        file_name: Some(file_name.to_string()),
    })
}

/// `readConfigFile`: reads and parses a config file.
pub fn read_config_file(file_name: &str, read_file: impl FnOnce(&str) -> Option<String>) -> Result<Value, Diagnostic> {
    let text = read_file(file_name).ok_or_else(|| cannot_read_file(file_name))?;
    parse_config_file_text(file_name, &text)
}

/// `parseJsonConfigFileContent`: a parsed config's options (as TypeScript's
/// API spells them), its root files and its errors. `extends` and
/// `include` are resolved against `base_path`, or against the config file's
/// directory when `config_file_name` is given.
pub fn parse_json_config_file_content(
    json: &Value,
    base_path: &str,
    existing_options: Option<&CompilerOptions>,
    config_file_name: Option<&str>,
) -> ParsedCommandLine {
    let config_path = match config_file_name {
        Some(name) => surge_ts_config::resolve_path(Path::new(base_path), name),
        None => PathBuf::from(base_path).join("tsconfig.json"),
    };
    let config_path = surge_ts_config::normalize_path_buf(&config_path);
    let (loaded, raw_options) = surge_ts_config::load_tsconfig_from_value(json, &config_path);
    let config_dir = config_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut options = existing_options.cloned().unwrap_or_default();
    for (name, value) in CompilerOptions::from_tsconfig_json(&raw_options, &config_dir).as_json() {
        options.set(name.clone(), value.clone());
    }
    if config_file_name.is_some() {
        options.set("configFilePath", Value::String(config_path.to_string_lossy().replace('\\', "/")));
    }
    ParsedCommandLine {
        options,
        file_names: loaded.files.iter().map(|file| file.to_string_lossy().replace('\\', "/")).collect(),
        // tsc reports a removed option among the program's option
        // diagnostics, not the config's errors.
        errors: loaded
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.is_legacy_option())
            .map(Diagnostic::from_config)
            .collect(),
        raw: json.clone(),
    }
}

/// `findConfigFile`: the nearest `config_name` (default `tsconfig.json`) at
/// or above `search_path`.
pub fn find_config_file(search_path: &str, file_exists: impl Fn(&str) -> bool, config_name: Option<&str>) -> Option<String> {
    let config_name = config_name.unwrap_or("tsconfig.json");
    let mut directory = Some(Path::new(search_path));
    while let Some(current) = directory {
        let candidate = current.join(config_name).to_string_lossy().replace('\\', "/");
        if file_exists(&candidate) {
            return Some(candidate);
        }
        directory = current.parent();
    }
    None
}
