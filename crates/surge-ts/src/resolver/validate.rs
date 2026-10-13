//! `SURGE_RESOLVER_VALIDATE=<trace>`: replays the resolutions a
//! `tsc --traceResolution` log records through [`super::ModuleResolver`] and
//! reports each answer that differs (`RESOLVE-DIFF`), then a summary.

use std::path::Path;

use super::host::DiskHost;
use super::{ModuleResolver, ResolutionMode};

fn canonical(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    std::fs::canonicalize(path).map_or_else(|_| path.to_string(), |real| real.to_string_lossy().into_owned())
}

fn quoted(line: &str, prefix: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(prefix)?;
    let (first, rest) = rest.split_once('\'')?;
    let rest = rest.strip_prefix(" from '").or_else(|| rest.strip_prefix(", containing file '"))?;
    let (second, _) = rest.split_once('\'')?;
    Some((first.to_string(), second.to_string()))
}

fn result(line: &str, prefix: &str, name: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?.strip_prefix(name)?.strip_prefix("' was ")?;
    if rest.starts_with("not resolved") {
        return Some(String::new());
    }
    let rest = rest.strip_prefix("successfully resolved to '")?;
    rest.split_once('\'').map(|(path, _)| path.to_string())
}

pub(crate) fn validate(loaded: &surge_ts_config::LoadedTsConfig, trace: &Path) {
    let Ok(text) = std::fs::read_to_string(trace) else {
        eprintln!("RESOLVE-SUMMARY unreadable trace {}", trace.display());
        return;
    };
    let resolver = ModuleResolver::new(super::config::resolver_config(loaded), DiskHost::default());
    let (mut compared, mut differing) = (0usize, 0usize);
    let mut pending: Vec<(bool, String, String, ResolutionMode)> = Vec::new();
    for line in text.lines() {
        if let Some((name, file)) = quoted(line, "======== Resolving module '") {
            pending.push((false, name, file, ResolutionMode::None));
            continue;
        }
        if let Some((name, file)) = quoted(line, "======== Resolving type reference directive '") {
            pending.push((true, name, file, ResolutionMode::None));
            continue;
        }
        if let Some(conditions) = line.strip_prefix("Resolving in ")
            && let Some(last) = pending.last_mut()
        {
            last.3 = if conditions.contains("'import'") { ResolutionMode::Esm } else { ResolutionMode::CommonJs };
            continue;
        }
        let Some(index) = pending.iter().rposition(|(is_type, name, ..)| {
            let prefix = if *is_type { "======== Type reference directive '" } else { "======== Module name '" };
            result(line, prefix, name).is_some()
        }) else {
            continue;
        };
        let (is_type, name, file, mode) = pending.remove(index);
        let prefix = if is_type { "======== Type reference directive '" } else { "======== Module name '" };
        let expected = result(line, prefix, &name).unwrap_or_default();
        let actual = if is_type {
            resolver.resolve_type_reference_directive(&name, &file, mode).resolved_file_name
        } else {
            resolver.resolve_module_name(&name, &file, mode).resolved_file_name
        };
        compared += 1;
        if canonical(&expected) != canonical(&actual) {
            differing += 1;
            eprintln!(
                "RESOLVE-DIFF {} '{name}' from '{file}' ({mode:?})\n    tsc:   {expected}\n    surge: {actual}",
                if is_type { "type" } else { "module" }
            );
        }
    }
    eprintln!("RESOLVE-SUMMARY compared {compared} differing {differing}");
}
