//! tsgo's `packagejson`: the fields the module resolver reads, kept as tsgo
//! validates them.

use std::cell::OnceCell;

use serde_json::{Map, Value};

use crate::semver::VersionRange;

/// A field tsgo reads as a string: present when written at all, valid only
/// when a JSON string.
#[derive(Debug, Clone, Default)]
pub(crate) struct StringField {
    pub(crate) present: bool,
    pub(crate) null: bool,
    pub(crate) value: Option<String>,
}

impl StringField {
    fn read(object: &Map<String, Value>, key: &str) -> Self {
        match object.get(key) {
            None => Self::default(),
            Some(Value::Null) => Self { present: true, null: true, value: None },
            Some(Value::String(text)) => Self { present: true, null: false, value: Some(text.clone()) },
            Some(_) => Self { present: true, null: false, value: None },
        }
    }
}

/// `typesVersions` paths for the first range the compiler's version
/// satisfies, in the order they were written.
#[derive(Debug, Clone)]
pub(crate) struct VersionPaths {
    pub(crate) paths: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Default)]
pub(crate) struct PackageJson {
    pub(crate) name: StringField,
    pub(crate) version: StringField,
    pub(crate) type_field: StringField,
    pub(crate) main: StringField,
    pub(crate) types: StringField,
    pub(crate) typings: StringField,
    types_versions: Option<Value>,
    /// `None` when the field is absent; `Some(Value::Null)` when written `null`.
    pub(crate) imports: Option<Value>,
    pub(crate) exports: Option<Value>,
    version_paths: OnceCell<Option<VersionPaths>>,
}

impl PackageJson {
    /// tsgo keeps a package.json it cannot parse as one with no fields.
    pub(crate) fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text) else {
            return Self::default();
        };
        Self {
            name: StringField::read(&object, "name"),
            version: StringField::read(&object, "version"),
            type_field: StringField::read(&object, "type"),
            main: StringField::read(&object, "main"),
            types: StringField::read(&object, "types"),
            typings: StringField::read(&object, "typings"),
            types_versions: object.get("typesVersions").cloned(),
            imports: object.get("imports").cloned(),
            exports: object.get("exports").cloned(),
            version_paths: OnceCell::new(),
        }
    }

    /// `GetVersionPaths`.
    pub(crate) fn version_paths(&self) -> Option<&VersionPaths> {
        self.version_paths
            .get_or_init(|| {
                let Some(Value::Object(versions)) = &self.types_versions else {
                    return None;
                };
                for (key, value) in versions {
                    let Some(range) = VersionRange::parse(key) else {
                        continue;
                    };
                    if !range.test(crate::package_resolution::typescript_version()) {
                        continue;
                    }
                    let Value::Object(paths) = value else {
                        return None;
                    };
                    let paths = paths
                        .iter()
                        .filter_map(|(pattern, targets)| {
                            let Value::Array(targets) = targets else {
                                return None;
                            };
                            let targets = targets
                                .iter()
                                .map(|target| match target {
                                    Value::String(text) => text.clone(),
                                    _ => String::new(),
                                })
                                .collect();
                            Some((pattern.clone(), targets))
                        })
                        .collect();
                    return Some(VersionPaths { paths });
                }
                None
            })
            .as_ref()
    }
}

/// `JSONValue.IsFalsy`, for an absent field too. tsgo decodes a number as a
/// `float64` and compares it with the integer `0`, so no number is falsy.
pub(crate) fn is_falsy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Bool(flag)) => !flag,
        Some(_) => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    Subpaths,
    Conditions,
    Imports,
    Invalid,
}

/// `ExportsOrImports.initObjectKind`: keys starting with `.` are subpaths,
/// with `#` imports, anything else conditions; a mix of conditions with
/// either is invalid.
pub(crate) fn object_kind(object: &Map<String, Value>) -> ObjectKind {
    if object.is_empty() {
        return ObjectKind::Conditions;
    }
    let (mut seen_dot, mut seen_hash, mut seen_other) = (false, false, false);
    for key in object.keys() {
        let Some(first) = key.as_bytes().first() else {
            continue;
        };
        seen_dot |= *first == b'.';
        seen_hash |= *first == b'#';
        seen_other |= *first != b'.' && *first != b'#';
        if seen_other && (seen_dot || seen_hash) {
            return ObjectKind::Invalid;
        }
    }
    if seen_dot {
        ObjectKind::Subpaths
    } else if seen_hash {
        ObjectKind::Imports
    } else {
        ObjectKind::Conditions
    }
}

/// A `package.json` lookup: the directory it was looked up for, and its
/// contents when the file exists.
#[derive(Debug, Clone)]
pub(crate) struct PackageJsonInfo {
    pub(crate) package_directory: String,
    pub(crate) contents: std::rc::Rc<PackageJson>,
}
