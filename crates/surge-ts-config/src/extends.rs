use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use jsonc_parser::parse_to_serde_value;
use serde_json::Value;

use crate::diagnostics::{ConfigDiagnostic, ConfigDiagnosticCode};
use crate::parse::{RawTsConfig, merge_configs, parse_current_config};
use crate::paths::{canonicalize_if_exists, cycle_key};

pub(crate) fn load_merged_config(
    config_path: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
    visited: &mut HashSet<PathBuf>,
) -> RawTsConfig {
    let config_key = cycle_key(config_path);
    if !visited.insert(config_key.clone()) {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::ExtendsCycle,
            message: "detected tsconfig extends cycle".to_string(),
            file_name: config_path.to_path_buf(),
        });
        return RawTsConfig::default();
    }

    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(_) => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::ConfigFileNotFound,
                message: format!("could not read {}", config_path.display()),
                file_name: config_path.to_path_buf(),
            });
            visited.remove(&config_key);
            return RawTsConfig::default();
        }
    };

    let value = match parse_to_serde_value::<Value>(&text, &Default::default()) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::ConfigParseError,
                message: error.to_string(),
                file_name: config_path.to_path_buf(),
            });
            visited.remove(&config_key);
            return RawTsConfig::default();
        }
    };

    let Some(object) = value.as_object() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::ConfigParseError,
            message: "tsconfig.json must contain a JSON object".to_string(),
            file_name: config_path.to_path_buf(),
        });
        visited.remove(&config_key);
        return RawTsConfig::default();
    };

    let merged = merge_config_object(config_path, object, diagnostics, visited);
    visited.remove(&config_key);
    merged
}

/// A config object's own settings merged over what its `extends` names.
pub(crate) fn merge_config_object(
    config_path: &Path,
    object: &serde_json::Map<String, Value>,
    diagnostics: &mut Vec<ConfigDiagnostic>,
    visited: &mut HashSet<PathBuf>,
) -> RawTsConfig {
    let parsed = parse_current_config(config_path, object, diagnostics);
    let base = match object.get("extends").and_then(Value::as_str) {
        Some(extends_spec) => match resolve_extends(config_path, extends_spec) {
            Ok(Some(base_path)) => {
                let mut base = load_merged_config(&base_path, diagnostics, visited);
                rebase_file_entries(&mut base, config_path, &base_path);
                Some(base)
            }
            Ok(None) => None,
            Err(message) => {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::ExtendsFileNotFound,
                    message,
                    file_name: config_path.to_path_buf(),
                });
                None
            }
        },
        None => None,
    };

    merge_configs(base.as_ref(), &parsed)
}

/// tsc's `applyExtendedConfig`: the `files`, `include` and `exclude` an
/// extended config writes are relative to its own directory, so each is
/// prefixed with the way there from the extending config's directory.
fn rebase_file_entries(base: &mut RawTsConfig, config_path: &Path, base_path: &Path) {
    let from = config_path.parent().unwrap_or_else(|| Path::new("."));
    let to = base_path.parent().unwrap_or_else(|| Path::new("."));
    let difference = relative_path(from, to);
    if difference.as_os_str().is_empty() {
        return;
    }
    for entries in [&mut base.files, &mut base.include, &mut base.exclude].into_iter().flatten() {
        for entry in entries.iter_mut() {
            if let Value::String(raw) = entry
                && !raw.starts_with("${configDir}")
                && !Path::new(raw.as_str()).is_absolute()
            {
                *raw = difference.join(raw.as_str()).to_string_lossy().into_owned();
            }
        }
    }
}

/// The relative path from directory `from` to directory `to`.
fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..from.len() {
        path.push("..");
    }
    for component in &to[common..] {
        path.push(component.as_os_str());
    }
    path
}

fn resolve_extends(config_path: &Path, extends_spec: &str) -> Result<Option<PathBuf>, String> {
    if Path::new(extends_spec).is_absolute() {
        let absolute = PathBuf::from(extends_spec);
        return if absolute.exists() {
            Ok(Some(absolute))
        } else {
            Err(format!(
                "Cannot resolve extends '{}' from {}",
                extends_spec,
                config_path.display()
            ))
        };
    }

    if extends_spec.starts_with("./")
        || extends_spec.starts_with("../")
        || extends_spec.starts_with('/')
    {
        return resolve_relative_extends(config_path, extends_spec);
    }

    let from_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    if let Some(resolved) = resolve_package_extends(extends_spec, from_dir) {
        return Ok(Some(resolved));
    }

    Err(format!(
        "Cannot resolve extends '{}' from {}",
        extends_spec,
        config_path.display()
    ))
}

fn resolve_relative_extends(
    config_path: &Path,
    extends_spec: &str,
) -> Result<Option<PathBuf>, String> {
    let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let primary = base_dir.join(extends_spec);
    if primary.exists() {
        return Ok(Some(primary));
    }

    let json_candidate = if primary.extension().is_some() {
        primary.clone()
    } else {
        primary.with_extension("json")
    };
    if json_candidate.exists() {
        return Ok(Some(json_candidate));
    }

    Err(format!(
        "Cannot resolve extends '{}' from {}",
        extends_spec,
        config_path.display()
    ))
}

fn resolve_package_extends(specifier: &str, from_dir: &Path) -> Option<PathBuf> {
    let (package_name, subpath) = split_package_specifier(specifier)?;

    for ancestor in from_dir.ancestors() {
        let package_root = ancestor.join("node_modules").join(&package_name);
        let candidate = package_root.join(&subpath);
        if let Some(resolved) = resolve_existing_config_candidate(&candidate) {
            return Some(resolved);
        }
    }

    None
}

fn split_package_specifier(specifier: &str) -> Option<(String, String)> {
    let mut segments = specifier.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next()?;

    let mut package_name = first.to_string();
    let mut subpath_segments = Vec::new();

    if first.starts_with('@') {
        let second = segments.next()?;
        package_name.push('/');
        package_name.push_str(second);
    }

    subpath_segments.extend(segments);

    let subpath = if subpath_segments.is_empty() {
        "tsconfig.json".to_string()
    } else {
        subpath_segments.join("/")
    };

    Some((package_name, subpath))
}

fn resolve_existing_config_candidate(candidate: &Path) -> Option<PathBuf> {
    if candidate.exists() && candidate.is_file() {
        return Some(canonicalize_if_exists(candidate));
    }

    if candidate.extension().is_none() {
        let json_candidate = candidate.with_extension("json");
        if json_candidate.exists() && json_candidate.is_file() {
            return Some(canonicalize_if_exists(&json_candidate));
        }
    }

    if candidate.exists() && candidate.is_dir() {
        let directory_default = candidate.join("tsconfig.json");
        if directory_default.exists() && directory_default.is_file() {
            return Some(canonicalize_if_exists(&directory_default));
        }
    }

    None
}
