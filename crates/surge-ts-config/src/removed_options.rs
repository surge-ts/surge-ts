//! Compiler options TypeScript 7 removed, reported as `TS5102` (the option is
//! gone entirely) or `TS5108` (the option survives but not at this value).
//!
//! The tables mirror tsc 7.0.2's behavior as probed against the real compiler;
//! the rendered value is TypeScript's canonical spelling (`target: "es5"` is
//! reported as `target=ES5`), not what the config literally wrote.
//!
//! Spans come from the *root* config file: an option inherited through
//! `extends` has no node here, so tsc anchors it on the extending file's
//! `compilerOptions` key, and so does this.

use std::path::Path;

use jsonc_parser::ast::{ObjectPropName, Value as AstValue};
use jsonc_parser::common::Ranged;
use jsonc_parser::{CollectOptions, ParseOptions, parse_to_ast};
use serde_json::{Map, Value};

/// A compiler option TypeScript 7 no longer accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedCompilerOption {
    pub name: String,
    /// Canonical value spelling for the `name=value` (`TS5108`) form. `None`
    /// when the option is removed regardless of its value (`TS5102`).
    pub value: Option<String>,
    /// Byte range inside the root config file.
    pub start: usize,
    pub end: usize,
}

/// Options removed outright: any value reports `TS5102`.
const REMOVED_OPTIONS: &[&str] = &["baseUrl", "downlevelIteration", "outFile"];

/// `false` is no longer a legal value for these; `true` is now the only behavior.
const REMOVED_FALSE_OPTIONS: &[&str] = &[
    "allowSyntheticDefaultImports",
    "alwaysStrict",
    "esModuleInterop",
];

/// Removed enum values, as `(option, lowercased written value, canonical
/// spelling)`. Values absent from the table either still work or are rejected
/// earlier as unknown (`target: "es3"` is no longer in the enum at all).
const REMOVED_ENUM_VALUES: &[(&str, &str, &str)] = &[
    ("target", "es5", "ES5"),
    ("module", "amd", "AMD"),
    ("module", "system", "System"),
    ("module", "umd", "UMD"),
    ("moduleResolution", "classic", "Classic"),
    ("moduleResolution", "node", "node10"),
    ("moduleResolution", "node10", "node10"),
];

fn removal_for(name: &str, value: &Value) -> Option<Option<String>> {
    if REMOVED_OPTIONS.contains(&name) {
        return Some(None);
    }
    if REMOVED_FALSE_OPTIONS.contains(&name) && value.as_bool() == Some(false) {
        return Some(Some("false".to_string()));
    }
    let written = value.as_str()?.to_ascii_lowercase();
    REMOVED_ENUM_VALUES
        .iter()
        .find(|(option, removed, _)| *option == name && *removed == written)
        .map(|(_, _, canonical)| Some((*canonical).to_string()))
}

/// Byte ranges of the root config's `compilerOptions` entries: the value node
/// per option (where `TS5108` points), the key node per option (where `TS5102`
/// points), and the `compilerOptions` key itself (the inherited-option anchor).
struct ConfigOptionRanges {
    compiler_options_key: (usize, usize),
    entries: Vec<(String, (usize, usize), (usize, usize))>,
}

fn collect_option_ranges(text: &str) -> Option<ConfigOptionRanges> {
    let parsed = parse_to_ast(
        text,
        &CollectOptions::default(),
        &ParseOptions::default(),
    )
    .ok()?;
    let AstValue::Object(root) = parsed.value? else {
        return None;
    };
    let property = root
        .properties
        .iter()
        .find(|property| property.name.as_str() == "compilerOptions")?;
    let compiler_options_key = match &property.name {
        ObjectPropName::String(literal) => (literal.range.start, literal.range.end),
        ObjectPropName::Word(word) => (word.range.start, word.range.end),
    };
    let AstValue::Object(options) = &property.value else {
        return None;
    };

    let entries = options
        .properties
        .iter()
        .map(|option| {
            let key = match &option.name {
                ObjectPropName::String(literal) => (literal.range.start, literal.range.end),
                ObjectPropName::Word(word) => (word.range.start, word.range.end),
            };
            let value = option.value.range();
            (
                option.name.as_str().to_string(),
                key,
                (value.start, value.end),
            )
        })
        .collect();

    Some(ConfigOptionRanges {
        compiler_options_key,
        entries,
    })
}

/// A compiler option whose value is none of its enum's keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidEnumOption {
    pub name: String,
    /// The keys tsc lists (`formatEnumTypeKeys`): the option's map without
    /// its deprecated keys, quoted and comma-separated.
    pub keys: String,
    /// Byte range of the value inside the root config file.
    pub start: usize,
    pub end: usize,
}

/// The enum options `convertJsonOptionOfEnumType` validates: each map's keys
/// in tsc's order (`enummaps.go`), and the keys `commandLineOptionDeprecated`
/// leaves out of the message.
const ENUM_OPTIONS: &[(&str, &[&str], &[&str])] = &[
    (
        "module",
        &[
            "commonjs", "amd", "system", "umd", "es6", "es2015", "es2020", "es2022", "esnext", "node16", "node18",
            "node20", "nodenext", "preserve",
        ],
        &["none", "amd", "system", "umd"],
    ),
    ("moduleResolution", &["node16", "nodenext", "bundler", "classic", "node", "node10"], &["node", "classic", "node10"]),
    (
        "target",
        &[
            "es5", "es6", "es2015", "es2016", "es2017", "es2018", "es2019", "es2020", "es2021", "es2022", "es2023",
            "es2024", "es2025", "esnext",
        ],
        &["es5"],
    ),
    ("moduleDetection", &["auto", "legacy", "force"], &[]),
    ("jsx", &["preserve", "react-native", "react-jsx", "react-jsxdev", "react"], &[]),
    ("newLine", &["crlf", "lf"], &[]),
];

/// tsc's `convertJsonOptionOfEnumType` over the root config's own
/// `compilerOptions`: a string value that, lowercased, is none of its
/// option's keys is TS6046 at the value (`createDiagnosticForInvalidEnumType`).
/// An empty string sets nothing and is not reported.
pub fn invalid_enum_options(config_path: &Path) -> Vec<InvalidEnumOption> {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };
    let Ok(parsed) = parse_to_ast(&text, &CollectOptions::default(), &ParseOptions::default()) else {
        return Vec::new();
    };
    let Some(AstValue::Object(root)) = parsed.value else {
        return Vec::new();
    };
    let Some(property) = root.properties.iter().find(|property| property.name.as_str() == "compilerOptions") else {
        return Vec::new();
    };
    let AstValue::Object(options) = &property.value else {
        return Vec::new();
    };
    options
        .properties
        .iter()
        .filter_map(|option| {
            let (_, keys, deprecated) = ENUM_OPTIONS.iter().find(|(name, _, _)| *name == option.name.as_str())?;
            let AstValue::StringLit(value) = &option.value else {
                return None;
            };
            let written = value.value.to_ascii_lowercase();
            if written.is_empty() || keys.contains(&written.as_str()) {
                return None;
            }
            let listed: Vec<String> =
                keys.iter().filter(|key| !deprecated.contains(*key)).map(|key| format!("'{key}'")).collect();
            let range = option.value.range();
            Some(InvalidEnumOption {
                name: option.name.as_str().to_string(),
                keys: listed.join(", "),
                start: range.start,
                end: range.end,
            })
        })
        .collect()
}

/// tsc's `verifyCompilerOptions` over the root config's own `paths`: a
/// substitution neither relative nor absolute is TS5090 at that element, as
/// TypeScript 7 has no `baseUrl` to resolve it against. Byte ranges.
pub fn non_relative_path_substitutions(config_path: &Path) -> Vec<(usize, usize)> {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };
    let Ok(parsed) = parse_to_ast(&text, &CollectOptions::default(), &ParseOptions::default()) else {
        return Vec::new();
    };
    let Some(AstValue::Object(root)) = parsed.value else {
        return Vec::new();
    };
    let Some(AstValue::Object(options)) =
        root.properties.iter().find(|property| property.name.as_str() == "compilerOptions").map(|property| &property.value)
    else {
        return Vec::new();
    };
    let Some(AstValue::Object(paths)) =
        options.properties.iter().find(|option| option.name.as_str() == "paths").map(|option| &option.value)
    else {
        return Vec::new();
    };
    paths
        .properties
        .iter()
        .filter_map(|mapping| match &mapping.value {
            AstValue::Array(array) => Some(array),
            _ => None,
        })
        .flat_map(|array| array.elements.iter())
        .filter_map(|element| match element {
            AstValue::StringLit(substitution)
                if !is_relative_path(&substitution.value) && !is_absolute_path(&substitution.value) =>
            {
                let range = element.range();
                Some((range.start, range.end))
            }
            _ => None,
        })
        .collect()
}

/// `tspath.PathIsRelative`.
fn is_relative_path(path: &str) -> bool {
    let rest = path.strip_prefix("..").or_else(|| path.strip_prefix('.'));
    rest.is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
}

/// `tspath.PathIsAbsolute`: a root (`/`, `\\`, a drive letter) or a URL.
fn is_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.contains("://")
}

/// Where tsc's `createOptionValueDiagnostic` reports on `name`'s value: the
/// value in the root config, else its `compilerOptions` key (an option
/// inherited through `extends`), else nowhere.
pub fn option_value_range(config_path: &Path, name: &str) -> Option<(usize, usize)> {
    let text = std::fs::read_to_string(config_path).ok()?;
    let ranges = collect_option_ranges(&text)?;
    Some(
        ranges
            .entries
            .iter()
            .find(|(option, _, _)| option == name)
            .map_or(ranges.compiler_options_key, |(_, _, value)| *value),
    )
}

/// Every removed option in the merged compiler options, in root-config source
/// order, with inherited ones (no node in this file) last.
pub(crate) fn collect_removed_options(
    config_path: &Path,
    compiler_options: Option<&Map<String, Value>>,
) -> Vec<RemovedCompilerOption> {
    let Some(compiler_options) = compiler_options else {
        return Vec::new();
    };
    let removed: Vec<(&String, Option<String>)> = compiler_options
        .iter()
        .filter_map(|(name, value)| removal_for(name, value).map(|value| (name, value)))
        .collect();
    if removed.is_empty() {
        return Vec::new();
    }

    let text = std::fs::read_to_string(config_path).unwrap_or_default();
    let Some(ranges) = collect_option_ranges(&text) else {
        return Vec::new();
    };

    let mut local = Vec::new();
    let mut inherited = Vec::new();
    for (name, value) in removed {
        match ranges
            .entries
            .iter()
            .find(|(option, _, _)| option == name.as_str())
        {
            Some((_, key, option_value)) => {
                let (start, end) = if value.is_some() { *option_value } else { *key };
                local.push(RemovedCompilerOption {
                    name: name.clone(),
                    value,
                    start,
                    end,
                });
            }
            None => inherited.push(RemovedCompilerOption {
                name: name.clone(),
                value,
                start: ranges.compiler_options_key.0,
                end: ranges.compiler_options_key.1,
            }),
        }
    }

    local.sort_by_key(|option| option.start);
    local.extend(inherited);
    local
}
