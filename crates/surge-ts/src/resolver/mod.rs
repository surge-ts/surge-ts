//! A port of tsgo's module resolver (`internal/module/resolver.go`): how a
//! module name or type reference directive written in a file resolves under
//! `node16`, `nodenext` and `bundler`, the only kinds tsgo has.

pub mod config;
pub(crate) mod host;
pub(crate) mod packagejson;
pub(crate) mod tspath;
pub(crate) mod validate;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value};
use surge_ts_types::fx::FxHashMap;

use packagejson::{ObjectKind, PackageJson, PackageJsonInfo, is_falsy, object_kind};
use tspath::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionMode {
    None,
    CommonJs,
    Esm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    Node16,
    NodeNext,
    Bundler,
}

/// The compiler options the resolver reads, with tsgo's defaults applied by
/// whoever builds it (`GetModuleResolutionKind`, `GetResolveJsonModule`,
/// `GetAllowJS`, `GetPathsBasePath`).
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    pub kind: ResolutionKind,
    pub resolve_package_json_exports: Option<bool>,
    pub resolve_package_json_imports: Option<bool>,
    pub custom_conditions: Vec<String>,
    pub no_dts_resolution: bool,
    pub resolve_json_module: bool,
    pub allow_js: bool,
    /// `paths`, in the order written.
    pub paths: Vec<(String, Vec<String>)>,
    pub paths_base_path: String,
    pub root_dirs: Vec<String>,
    /// `typeRoots` as written in the config, made absolute.
    pub type_roots: Option<Vec<String>>,
    pub types: Option<Vec<String>>,
    pub config_file_path: Option<String>,
    pub current_directory: String,
    pub preserve_symlinks: bool,
    pub module_suffixes: Vec<String>,
    pub out_dir: Option<String>,
    pub declaration_dir: Option<String>,
    pub root_dir: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedModule {
    pub resolved_file_name: String,
    /// The path before its symlinks were resolved, when they changed it.
    pub original_path: String,
    pub extension: String,
    pub resolved_using_ts_extension: bool,
    pub is_external_library_import: bool,
    /// What `exports`-less resolution finds when the modern resolution reached
    /// only JavaScript (`AlternateResult`).
    pub alternate_result: String,
}

impl ResolvedModule {
    pub fn is_resolved(&self) -> bool {
        !self.resolved_file_name.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTypeReferenceDirective {
    pub resolved_file_name: String,
    pub original_path: String,
    pub primary: bool,
    pub is_external_library_import: bool,
}

impl ResolvedTypeReferenceDirective {
    pub fn is_resolved(&self) -> bool {
        !self.resolved_file_name.is_empty()
    }
}

/// The file system the resolver asks, as tsgo's `vfs.FS`.
pub trait ResolverHost {
    fn file_exists(&self, path: &str) -> bool;
    fn directory_exists(&self, path: &str) -> bool;
    fn read_file(&self, path: &str) -> Option<String>;
    fn realpath(&self, path: &str) -> String;
    fn directories(&self, path: &str) -> Vec<String>;
}

const TYPESCRIPT: u8 = 1;
const JAVASCRIPT: u8 = 1 << 1;
const DECLARATION: u8 = 1 << 2;
const JSON: u8 = 1 << 3;
const IMPLEMENTATION: u8 = TYPESCRIPT | JAVASCRIPT;

const FEATURE_IMPORTS: u8 = 1;
const FEATURE_SELF_NAME: u8 = 1 << 1;
const FEATURE_EXPORTS: u8 = 1 << 2;
const FEATURE_EXPORTS_PATTERN_TRAILERS: u8 = 1 << 3;
const FEATURE_IMPORTS_PATTERN_ROOT: u8 = 1 << 4;
const FEATURES_NODE16: u8 = FEATURE_IMPORTS | FEATURE_SELF_NAME | FEATURE_EXPORTS | FEATURE_EXPORTS_PATTERN_TRAILERS;
const FEATURES_ALL: u8 = FEATURES_NODE16 | FEATURE_IMPORTS_PATTERN_ROOT;

const INFERRED_TYPES_CONTAINING_FILE: &str = "__inferred type names__.ts";

/// tsgo's `*resolved`: `None` keeps searching; a `Resolved` with no path
/// stops the search unresolved.
#[derive(Debug, Clone, Default)]
struct Resolved {
    path: String,
    extension: String,
    original_path: String,
    resolved_using_ts_extension: bool,
}

type Search = Option<Resolved>;

fn unresolved() -> Search {
    Some(Resolved::default())
}

fn extension_is_ok(extensions: u8, extension: &str) -> bool {
    (extensions & JAVASCRIPT != 0 && matches!(extension, EXTENSION_JS | EXTENSION_JSX | EXTENSION_MJS | EXTENSION_CJS))
        || (extensions & TYPESCRIPT != 0
            && matches!(extension, EXTENSION_TS | EXTENSION_TSX | EXTENSION_MTS | EXTENSION_CTS))
        || (extensions & DECLARATION != 0 && matches!(extension, EXTENSION_DTS | EXTENSION_DMTS | EXTENSION_DCTS))
        || (extensions & JSON != 0 && extension == EXTENSION_JSON)
}

/// `ParsePackageName`: `@scope/name` or `name`, and the rest.
pub(crate) fn parse_package_name(module_name: &str) -> (&str, &str) {
    let mut index = module_name.find('/');
    if module_name.starts_with('@')
        && let Some(first) = index
    {
        index = module_name[first + 1..].find('/').map(|second| second + first + 1);
    }
    match index {
        None => (module_name, ""),
        Some(index) => (&module_name[..index], &module_name[index + 1..]),
    }
}

pub(crate) fn mangle_scoped_package_name(package_name: &str) -> String {
    match package_name.strip_prefix('@').and_then(|rest| rest.split_once('/')) {
        Some((scope, name)) => format!("{scope}__{name}"),
        None => package_name.to_string(),
    }
}

/// `ComparePatternKeys`: longer prefixes first, then the key with a `*`.
fn compare_pattern_keys(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let a_star = a.find('*');
    let b_star = b.find('*');
    let base_a = a_star.map_or(a.len(), |index| index + 1);
    let base_b = b_star.map_or(b.len(), |index| index + 1);
    if base_a > base_b {
        return Less;
    }
    if base_b > base_a {
        return Greater;
    }
    if a_star.is_none() {
        return Greater;
    }
    if b_star.is_none() {
        return Less;
    }
    b.len().cmp(&a.len())
}

fn matches_pattern_with_trailer(target: &str, name: &str) -> bool {
    if target.ends_with('*') {
        return false;
    }
    match target.split_once('*') {
        Some((before, after)) => name.starts_with(before) && name.ends_with(after),
        None => false,
    }
}

/// `core.Pattern`: a `paths`/`typesVersions` key with at most one `*`.
#[derive(Debug, Clone)]
struct Pattern {
    text: String,
    star: Option<usize>,
}

impl Pattern {
    fn parse(text: &str) -> Option<Self> {
        match text.find('*') {
            None => Some(Self { text: text.to_string(), star: None }),
            Some(star) if !text[star + 1..].contains('*') => Some(Self { text: text.to_string(), star: Some(star) }),
            Some(_) => None,
        }
    }

    fn matches(&self, candidate: &str) -> bool {
        match self.star {
            None => self.text == candidate,
            Some(star) => {
                candidate.len() + 1 >= self.text.len()
                    && candidate.starts_with(&self.text[..star])
                    && candidate.ends_with(&self.text[star + 1..])
            }
        }
    }

    fn matched_text<'c>(&self, candidate: &'c str) -> &'c str {
        match self.star {
            None => "",
            Some(star) => &candidate[star..candidate.len() + star + 1 - self.text.len()],
        }
    }
}

/// `TryParsePatterns` + `MatchPatternOrExact`.
fn match_pattern_or_exact(keys: &[(String, Vec<String>)], candidate: &str) -> Option<Pattern> {
    let patterns: Vec<Pattern> = keys.iter().filter_map(|(key, _)| Pattern::parse(key)).collect();
    if patterns.iter().any(|pattern| pattern.star.is_none() && pattern.text == candidate) {
        return Some(Pattern { text: candidate.to_string(), star: None });
    }
    let mut best: Option<Pattern> = None;
    let mut longest: isize = -1;
    for pattern in patterns.into_iter().filter(|pattern| pattern.star.is_some()) {
        let star = pattern.star.map_or(-1, |star| star as isize);
        if star > longest && pattern.matches(candidate) {
            longest = star;
            best = Some(pattern);
        }
    }
    best
}

/// `normalizePathForCJSResolution`: a name ending in `.` or `..` names a
/// directory.
fn normalize_path_for_cjs_resolution(containing_directory: &str, module_name: &str) -> String {
    let combined = combine(containing_directory, module_name);
    let components = get_path_components(&combined);
    match components.last().map(String::as_str) {
        Some("." | "..") => ensure_trailing_directory_separator(&normalize_path(&combined)),
        _ => normalize_path(&combined),
    }
}

pub struct ModuleResolver<H: ResolverHost> {
    config: ResolverConfig,
    host: H,
    package_json_cache: RefCell<FxHashMap<String, Option<Rc<PackageJson>>>>,
    module_cache: RefCell<FxHashMap<(String, String, ResolutionMode), ResolvedModule>>,
    type_reference_cache: RefCell<FxHashMap<(String, String, ResolutionMode, bool), ResolvedTypeReferenceDirective>>,
}

impl<H: ResolverHost> ModuleResolver<H> {
    pub fn new(config: ResolverConfig, host: H) -> Self {
        Self {
            config,
            host,
            package_json_cache: RefCell::default(),
            module_cache: RefCell::default(),
            type_reference_cache: RefCell::default(),
        }
    }

    pub fn config(&self) -> &ResolverConfig {
        &self.config
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    /// `ResolveModuleName`.
    pub fn resolve_module_name(&self, name: &str, containing_file: &str, mode: ResolutionMode) -> ResolvedModule {
        let containing_directory = get_directory_path(containing_file);
        let key = (containing_directory.clone(), name.to_string(), mode);
        if let Some(cached) = self.module_cache.borrow().get(&key) {
            return cached.clone();
        }
        let mut state = State::new(self, name, &containing_directory, false, mode);
        let result = state.resolve_node_like();
        self.module_cache.borrow_mut().insert(key, result.clone());
        result
    }

    /// `ResolveTypeReferenceDirective`.
    pub fn resolve_type_reference_directive(
        &self,
        name: &str,
        containing_file: &str,
        mode: ResolutionMode,
    ) -> ResolvedTypeReferenceDirective {
        let containing_directory = get_directory_path(containing_file);
        let from_inferred = containing_file.ends_with(INFERRED_TYPES_CONTAINING_FILE);
        let key = (containing_directory.clone(), name.to_string(), mode, from_inferred);
        if let Some(cached) = self.type_reference_cache.borrow().get(&key) {
            return cached.clone();
        }
        let (type_roots, from_config) = self.effective_type_roots();
        let mut state = State::new(self, name, &containing_directory, true, mode);
        let result = state.resolve_type_reference_directive(&type_roots, from_config, from_inferred);
        self.type_reference_cache.borrow_mut().insert(key, result.clone());
        result
    }

    /// `GetEffectiveTypeRoots`.
    pub fn effective_type_roots(&self) -> (Vec<String>, bool) {
        if let Some(type_roots) = &self.config.type_roots {
            return (type_roots.clone(), true);
        }
        let base = match &self.config.config_file_path {
            Some(config) => get_directory_path(config),
            None => self.config.current_directory.clone(),
        };
        let roots = ancestor_directories(&base)
            .into_iter()
            .map(|directory| combine_paths(&directory, &["node_modules", "@types"]))
            .collect();
        (roots, false)
    }

    /// `GetAutomaticTypeDirectiveNames`: `types` as written, with a `*` standing
    /// for every package under the type roots.
    pub fn automatic_type_directive_names(&self) -> Vec<String> {
        let types = self.config.types.clone().unwrap_or_default();
        if !types.iter().any(|name| name == "*") {
            return types;
        }
        let mut wildcard = Vec::new();
        for root in self.effective_type_roots().0 {
            if !self.host.directory_exists(&root) {
                continue;
            }
            for directory in self.host.directories(&root) {
                let normalized = normalize_path(&directory);
                let package_json = combine_paths(&root, &[&normalized, "package.json"]);
                let not_needed = self.host.file_exists(&package_json)
                    && self
                        .host
                        .read_file(&package_json)
                        .is_some_and(|text| PackageJson::parse(&text).typings.null);
                if !not_needed {
                    let base = get_base_file_name(&normalized);
                    if !base.starts_with('.') {
                        wildcard.push(base);
                    }
                }
            }
        }
        let mut result: Vec<String> = Vec::new();
        for name in types {
            let names = if name == "*" { wildcard.clone() } else { vec![name] };
            for name in names {
                if !result.contains(&name) {
                    result.push(name);
                }
            }
        }
        result
    }

    /// `getPackageJsonInfo`, cached by the package.json path.
    fn package_json_info(&self, package_directory: &str) -> Option<PackageJsonInfo> {
        let path = combine(package_directory, "package.json");
        if let Some(cached) = self.package_json_cache.borrow().get(&path) {
            return cached.clone().map(|contents| PackageJsonInfo {
                package_directory: package_directory.to_string(),
                contents,
            });
        }
        let contents = (self.host.directory_exists(package_directory) && self.host.file_exists(&path))
            .then(|| Rc::new(PackageJson::parse(&self.host.read_file(&path).unwrap_or_default())));
        self.package_json_cache.borrow_mut().insert(path, contents.clone());
        contents.map(|contents| PackageJsonInfo { package_directory: package_directory.to_string(), contents })
    }
}

struct State<'r, H: ResolverHost> {
    resolver: &'r ModuleResolver<H>,
    name: String,
    containing_directory: String,
    features: u8,
    esm_mode: bool,
    conditions: Vec<String>,
    extensions: u8,
    candidate_ending_is_from_config: bool,
    resolved_package_directory: bool,
}

impl<'r, H: ResolverHost> State<'r, H> {
    /// `newResolutionState`.
    fn new(
        resolver: &'r ModuleResolver<H>,
        name: &str,
        containing_directory: &str,
        is_type_reference_directive: bool,
        mode: ResolutionMode,
    ) -> Self {
        let config = &resolver.config;
        let mut extensions = if is_type_reference_directive {
            DECLARATION
        } else if config.no_dts_resolution {
            IMPLEMENTATION
        } else {
            TYPESCRIPT | JAVASCRIPT | DECLARATION
        };
        if !is_type_reference_directive && config.resolve_json_module {
            extensions |= JSON;
        }
        let (features, esm_mode) = match config.kind {
            ResolutionKind::Node16 => (FEATURES_NODE16, mode == ResolutionMode::Esm),
            ResolutionKind::NodeNext => (FEATURES_ALL, mode == ResolutionMode::Esm),
            ResolutionKind::Bundler => {
                let mut features = FEATURES_ALL;
                match config.resolve_package_json_exports {
                    Some(true) => features |= FEATURE_EXPORTS,
                    Some(false) => features &= !FEATURE_EXPORTS,
                    None => {}
                }
                match config.resolve_package_json_imports {
                    Some(true) => features |= FEATURE_IMPORTS,
                    Some(false) => features &= !FEATURE_IMPORTS,
                    None => {}
                }
                (features, false)
            }
        };
        Self {
            resolver,
            name: name.to_string(),
            containing_directory: containing_directory.to_string(),
            features,
            esm_mode,
            conditions: conditions(config, mode),
            extensions,
            candidate_ending_is_from_config: false,
            resolved_package_directory: false,
        }
    }

    fn host(&self) -> &H {
        &self.resolver.host
    }

    fn config(&self) -> &ResolverConfig {
        &self.resolver.config
    }

    fn resolve_node_like(&mut self) -> ResolvedModule {
        let mut result = self.resolve_node_like_worker();
        if self.resolved_package_directory
            && self.features & FEATURE_EXPORTS != 0
            && self.extensions & (TYPESCRIPT | DECLARATION) != 0
            && !is_external_module_name_relative(&self.name)
            && result.is_resolved()
            && result.is_external_library_import
            && !extension_is_ok(TYPESCRIPT | DECLARATION, &result.extension)
            && self.conditions.iter().any(|condition| condition == "import")
        {
            self.features &= !FEATURE_EXPORTS;
            self.extensions &= TYPESCRIPT | DECLARATION;
            let diagnostic_result = self.resolve_node_like_worker();
            if diagnostic_result.is_resolved() && diagnostic_result.is_external_library_import {
                result.alternate_result = diagnostic_result.resolved_file_name;
            }
        }
        result
    }

    fn resolve_node_like_worker(&mut self) -> ResolvedModule {
        if let Some(resolved) = self.try_load_module_using_optional_resolution_settings() {
            return self.create_resolved_module_handling_symlink(Some(resolved));
        }
        if !is_external_module_name_relative(&self.name) {
            if self.features & FEATURE_IMPORTS != 0
                && self.name.starts_with('#')
                && let Some(resolved) = self.load_module_from_imports()
            {
                return self.create_resolved_module_handling_symlink(Some(resolved));
            }
            if self.features & FEATURE_SELF_NAME != 0
                && let Some(resolved) = self.load_module_from_self_name_reference()
            {
                return self.create_resolved_module_handling_symlink(Some(resolved));
            }
            if self.name.contains(':') {
                return self.create_resolved_module(None, false);
            }
            if let Some(resolved) = self.load_module_from_nearest_node_modules_directory(false) {
                return self.create_resolved_module_handling_symlink(Some(resolved));
            }
            if self.extensions & DECLARATION != 0
                && let Some(resolved) = self.resolve_from_type_root()
            {
                return self.create_resolved_module_handling_symlink(Some(resolved));
            }
        } else {
            let candidate = normalize_path_for_cjs_resolution(&self.containing_directory, &self.name);
            let extensions = self.extensions;
            let resolved = self.node_load_module_by_relative_name(extensions, &candidate, true);
            let external = resolved.as_ref().is_some_and(|resolved| resolved.path.contains("/node_modules/"));
            return self.create_resolved_module(resolved, external);
        }
        self.create_resolved_module(None, false)
    }

    fn load_module_from_self_name_reference(&mut self) -> Search {
        let directory = get_normalized_absolute_path(&self.containing_directory, &self.config().current_directory);
        let scope = self.package_scope_for_path(&directory)?;
        if is_falsy(scope.contents.exports.as_ref()) {
            return None;
        }
        let package_name = scope.contents.name.value.clone()?;
        let parts = get_path_components(&self.name);
        let name_parts = get_path_components(&package_name);
        if parts.len() < name_parts.len() || name_parts[..] != parts[..name_parts.len()] {
            return None;
        }
        let trailing: Vec<&str> = parts[name_parts.len()..].iter().map(String::as_str).collect();
        let subpath = if trailing.is_empty() { ".".to_string() } else { combine_paths(".", &trailing) };
        if self.config().allow_js && !self.containing_directory.contains("/node_modules/") {
            let extensions = self.extensions;
            return self.load_module_from_exports(&scope, extensions, &subpath);
        }
        let priority = self.extensions & (TYPESCRIPT | DECLARATION);
        let secondary = self.extensions & !(TYPESCRIPT | DECLARATION);
        if let Some(resolved) = self.load_module_from_exports(&scope, priority, &subpath) {
            return Some(resolved);
        }
        self.load_module_from_exports(&scope, secondary, &subpath)
    }

    fn load_module_from_imports(&mut self) -> Search {
        if self.name == "#" || (self.name.starts_with("#/") && self.features & FEATURE_IMPORTS_PATTERN_ROOT == 0) {
            return None;
        }
        let directory = get_normalized_absolute_path(&self.containing_directory, &self.config().current_directory);
        let scope = self.package_scope_for_path(&directory)?;
        let Some(Value::Object(imports)) = &scope.contents.imports else {
            return None;
        };
        let imports = imports.clone();
        let (extensions, name) = (self.extensions, self.name.clone());
        self.load_module_from_exports_or_imports(extensions, &name, &imports, &scope, true)
    }

    fn load_module_from_exports(&mut self, info: &PackageJsonInfo, extensions: u8, subpath: &str) -> Search {
        let exports = info.contents.exports.as_ref();
        if is_falsy(exports) {
            return None;
        }
        let exports = exports?.clone();
        if subpath == "." {
            let main_export = match &exports {
                Value::String(_) | Value::Array(_) => Some(exports.clone()),
                Value::Object(object) if object_kind(object) == ObjectKind::Conditions => Some(exports.clone()),
                Value::Object(object) => object.get(".").cloned(),
                _ => None,
            };
            if let Some(main_export) = main_export {
                return self.load_module_from_target_export_or_import(
                    extensions,
                    subpath,
                    info,
                    false,
                    &main_export,
                    "",
                    false,
                    ".",
                );
            }
        } else if let Value::Object(object) = &exports
            && object_kind(object) == ObjectKind::Subpaths
            && let Some(resolved) = self.load_module_from_exports_or_imports(extensions, subpath, object, info, false)
        {
            return Some(resolved);
        }
        None
    }

    fn load_module_from_exports_or_imports(
        &mut self,
        extensions: u8,
        module_name: &str,
        table: &Map<String, Value>,
        scope: &PackageJsonInfo,
        is_imports: bool,
    ) -> Search {
        if !module_name.ends_with('/')
            && !module_name.contains('*')
            && let Some(target) = table.get(module_name)
        {
            return self.load_module_from_target_export_or_import(
                extensions,
                module_name,
                scope,
                is_imports,
                target,
                "",
                false,
                module_name,
            );
        }
        let mut expanding: Vec<&String> =
            table.keys().filter(|key| key.matches('*').count() == 1 || key.ends_with('/')).collect();
        expanding.sort_by(|a, b| compare_pattern_keys(a, b));
        for key in expanding {
            let target = &table[key.as_str()];
            if self.features & FEATURE_EXPORTS_PATTERN_TRAILERS != 0 && matches_pattern_with_trailer(key, module_name) {
                let star = key.find('*').unwrap_or_default();
                let subpath = &module_name[star..module_name.len() - (key.len() - 1 - star)];
                return self.load_module_from_target_export_or_import(
                    extensions,
                    module_name,
                    scope,
                    is_imports,
                    target,
                    subpath,
                    true,
                    key,
                );
            } else if key.ends_with('*') && module_name.starts_with(&key[..key.len() - 1]) {
                let subpath = &module_name[key.len() - 1..];
                return self.load_module_from_target_export_or_import(
                    extensions,
                    module_name,
                    scope,
                    is_imports,
                    target,
                    subpath,
                    true,
                    key,
                );
            } else if module_name.starts_with(key.as_str()) {
                let subpath = &module_name[key.len()..];
                return self.load_module_from_target_export_or_import(
                    extensions,
                    module_name,
                    scope,
                    is_imports,
                    target,
                    subpath,
                    false,
                    key,
                );
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn load_module_from_target_export_or_import(
        &mut self,
        extensions: u8,
        module_name: &str,
        scope: &PackageJsonInfo,
        is_imports: bool,
        target: &Value,
        subpath: &str,
        is_pattern: bool,
        key: &str,
    ) -> Search {
        match target {
            Value::String(target) => {
                if !is_pattern && !subpath.is_empty() && !target.ends_with('/') {
                    return None;
                }
                if !target.starts_with("./") {
                    if is_imports && !target.starts_with("../") && !target.starts_with('/') && !is_rooted_disk_path(target)
                    {
                        let combined =
                            if is_pattern { target.replace('*', subpath) } else { format!("{target}{subpath}") };
                        let scope_directory = ensure_trailing_directory_separator(&scope.package_directory);
                        let saved_name = std::mem::replace(&mut self.name, combined);
                        let saved_directory = std::mem::replace(&mut self.containing_directory, scope_directory);
                        let result = self.resolve_node_like();
                        self.name = saved_name;
                        self.containing_directory = saved_directory;
                        if result.is_resolved() {
                            return Some(Resolved {
                                path: result.resolved_file_name,
                                extension: result.extension,
                                original_path: result.original_path,
                                resolved_using_ts_extension: result.resolved_using_ts_extension,
                            });
                        }
                    }
                    return None;
                }
                let components = get_path_components(target);
                let parts = if path_is_relative(target) { &components[1..] } else { &components[..] };
                let invalid = |part: &String| part == ".." || part == "." || part == "node_modules";
                if parts.iter().skip(1).any(invalid) {
                    return None;
                }
                let resolved_target = combine(&scope.package_directory, target);
                if get_path_components(subpath).iter().any(invalid) {
                    return None;
                }
                let current_directory = self.config().current_directory.clone();
                let final_path = if is_pattern {
                    get_normalized_absolute_path(&resolved_target.replace('*', subpath), &current_directory)
                } else {
                    get_normalized_absolute_path(&format!("{resolved_target}{subpath}"), &current_directory)
                };
                let package_path = combine(&scope.package_directory, "package.json");
                if let Some(resolved) = self.try_load_input_file_for_path(&final_path, subpath, &package_path, is_imports) {
                    return Some(resolved);
                }
                self.load_file_name_from_package_json_field(extensions, &final_path, target)
            }
            Value::Object(conditions) => {
                for (condition, sub_target) in conditions {
                    if self.condition_matches(condition)
                        && let Some(resolved) = self.load_module_from_target_export_or_import(
                            extensions,
                            module_name,
                            scope,
                            is_imports,
                            sub_target,
                            subpath,
                            is_pattern,
                            key,
                        )
                    {
                        return Some(resolved);
                    }
                }
                None
            }
            Value::Array(elements) => {
                for element in elements {
                    if let Some(resolved) = self.load_module_from_target_export_or_import(
                        extensions,
                        module_name,
                        scope,
                        is_imports,
                        element,
                        subpath,
                        is_pattern,
                        key,
                    ) {
                        return Some(resolved);
                    }
                }
                None
            }
            Value::Null => unresolved(),
            _ => None,
        }
    }

    /// `tryLoadInputFileForPath`: an export into the project's own `outDir` or
    /// `declarationDir` read as the input file it is built from.
    fn try_load_input_file_for_path(&mut self, final_path: &str, entry: &str, package_path: &str, is_imports: bool) -> Search {
        let _ = (entry, is_imports);
        if self.config().declaration_dir.is_none() && self.config().out_dir.is_none() {
            return None;
        }
        let config = self.config().clone();
        if (config.declaration_dir.is_none() && config.out_dir.is_none())
            || final_path.contains("/node_modules/")
            || config
                .config_file_path
                .as_ref()
                .is_some_and(|config_path| {
                    !contains_path(&get_directory_path(package_path), config_path, &config.current_directory)
                })
        {
            return None;
        }
        let root_dir = match (&config.root_dir, &config.config_file_path) {
            (Some(root_dir), _) => root_dir.clone(),
            (None, Some(config_path)) => get_directory_path(config_path),
            (None, None) => return unresolved(),
        };
        let current = if config.config_file_path.is_some() { config.current_directory.clone() } else { root_dir.clone() };
        let mut candidate_directories = Vec::new();
        if let Some(declaration_dir) = &config.declaration_dir {
            candidate_directories
                .push(get_normalized_absolute_path(&combine(&current, declaration_dir), &config.current_directory));
        }
        if let Some(out_dir) = &config.out_dir
            && Some(out_dir) != config.declaration_dir.as_ref()
        {
            candidate_directories.push(get_normalized_absolute_path(&combine(&current, out_dir), &config.current_directory));
        }
        for candidate_directory in candidate_directories {
            if !contains_path(&candidate_directory, final_path, &config.current_directory) {
                continue;
            }
            let fragment =
                if final_path.len() > candidate_directory.len() { &final_path[candidate_directory.len() + 1..] } else { "" };
            let base = combine(&root_dir, fragment);
            for extension in [
                EXTENSION_MJS,
                EXTENSION_CJS,
                EXTENSION_JS,
                EXTENSION_JSON,
                EXTENSION_DMTS,
                EXTENSION_DCTS,
                EXTENSION_DTS,
            ] {
                if !file_extension_is(&base, extension) {
                    continue;
                }
                for input_extension in possible_original_input_extensions(&base) {
                    if !extension_is_ok(self.extensions, &input_extension) {
                        continue;
                    }
                    let input = change_extension(&base, &input_extension);
                    if self.host().file_exists(&input) {
                        let extensions = self.extensions;
                        if let Some(resolved) = self.load_file_name_from_package_json_field(extensions, &input, "") {
                            return Some(resolved);
                        }
                    }
                }
            }
        }
        None
    }

    fn load_module_from_nearest_node_modules_directory(&mut self, types_scope_only: bool) -> Search {
        let priority = self.extensions & (TYPESCRIPT | DECLARATION);
        let secondary = self.extensions & !(TYPESCRIPT | DECLARATION);
        if priority != 0
            && let Some(resolved) = self.load_module_from_nearest_node_modules_directory_worker(priority, types_scope_only)
        {
            return Some(resolved);
        }
        if secondary != 0 && !types_scope_only {
            return self.load_module_from_nearest_node_modules_directory_worker(secondary, types_scope_only);
        }
        None
    }

    fn load_module_from_nearest_node_modules_directory_worker(&mut self, extensions: u8, types_scope_only: bool) -> Search {
        for directory in ancestor_directories(&self.containing_directory) {
            if get_base_file_name(&directory) != "node_modules"
                && let Some(resolved) =
                    self.load_module_from_immediate_node_modules_directory(extensions, &directory, types_scope_only)
            {
                return Some(resolved);
            }
        }
        None
    }

    fn load_module_from_immediate_node_modules_directory(
        &mut self,
        extensions: u8,
        directory: &str,
        types_scope_only: bool,
    ) -> Search {
        let node_modules = combine(directory, "node_modules");
        if !self.host().directory_exists(&node_modules) {
            return None;
        }
        if !types_scope_only {
            let name = self.name.clone();
            if let Some(resolved) = self.load_module_from_specific_node_modules_directory(extensions, &name, &node_modules) {
                return Some(resolved);
            }
        }
        if extensions & DECLARATION != 0 {
            let at_types = combine(&node_modules, "@types");
            if !self.host().directory_exists(&at_types) {
                return None;
            }
            let mangled = mangle_scoped_package_name(&self.name);
            return self.load_module_from_specific_node_modules_directory(DECLARATION, &mangled, &at_types);
        }
        None
    }

    fn load_module_from_specific_node_modules_directory(
        &mut self,
        extensions: u8,
        module_name: &str,
        node_modules_directory: &str,
    ) -> Search {
        let candidate =
            remove_trailing_directory_separator(&normalize_path(&combine(node_modules_directory, module_name))).to_string();
        let (package_name, rest) = parse_package_name(module_name);
        let package_directory =
            if package_name.is_empty() { candidate.clone() } else { combine(node_modules_directory, package_name) };
        let rest = rest.to_string();

        let mut package_info = self.resolver.package_json_info(&candidate);
        let mut root_package_info = None;
        if !rest.is_empty() && package_info.is_some() {
            if self.features & FEATURE_EXPORTS != 0 {
                root_package_info = self.resolver.package_json_info(&package_directory);
            }
            if root_package_info.as_ref().is_none_or(|root| root.contents.exports.is_none()) {
                if let Some(resolved) = self.load_module_from_file(extensions, &candidate) {
                    return Some(resolved);
                }
                if let Some(resolved) =
                    self.load_node_module_from_directory_worker(extensions, &candidate, package_info.as_ref())
                {
                    return Some(resolved);
                }
            }
        }

        if !rest.is_empty() {
            package_info = root_package_info.or_else(|| self.resolver.package_json_info(&package_directory));
        }
        if let Some(info) = &package_info {
            self.resolved_package_directory = true;
            if self.features & FEATURE_EXPORTS != 0 && !is_falsy(info.contents.exports.as_ref()) {
                return self.load_module_from_exports(info, extensions, &combine(".", &rest));
            }
            if !rest.is_empty()
                && let Some(version_paths) = info.contents.version_paths()
            {
                let paths = version_paths.paths.clone();
                if let Some(resolved) = self.try_load_module_using_paths(
                    extensions,
                    &rest,
                    &package_directory,
                    &paths,
                    &mut |state, extensions, candidate| {
                        state.node_modules_package_loader(extensions, candidate, &rest, package_info.as_ref())
                    },
                ) {
                    return Some(resolved);
                }
            }
        }
        self.node_modules_package_loader(extensions, &candidate, &rest, package_info.as_ref())
    }

    /// The `loader` of `loadModuleFromSpecificNodeModulesDirectory`.
    fn node_modules_package_loader(
        &mut self,
        extensions: u8,
        candidate: &str,
        rest: &str,
        package_info: Option<&PackageJsonInfo>,
    ) -> Search {
        if (!rest.is_empty() || !self.esm_mode)
            && let Some(resolved) = self.load_module_from_file(extensions, candidate)
        {
            return Some(resolved);
        }
        if let Some(resolved) = self.load_node_module_from_directory_worker(extensions, candidate, package_info) {
            return Some(resolved);
        }
        if rest.is_empty()
            && package_info.is_some_and(|info| matches!(info.contents.exports, None | Some(Value::Null)))
            && self.esm_mode
            && let Some(resolved) = self.load_module_from_file(extensions, &combine(candidate, "index.js"))
        {
            return Some(resolved);
        }
        None
    }

    fn create_resolved_module_handling_symlink(&self, resolved: Search) -> ResolvedModule {
        let external = resolved.as_ref().is_some_and(|resolved| resolved.path.contains("/node_modules/"));
        let mut resolved = resolved;
        if !self.config().preserve_symlinks
            && external
            && let Some(inner) = resolved.as_mut()
            && inner.original_path.is_empty()
            && !is_external_module_name_relative(&self.name)
        {
            let (original, real) = self.original_and_resolved_file_name(&inner.path);
            if !original.is_empty() {
                inner.path = real;
                inner.original_path = original;
            }
        }
        self.create_resolved_module(resolved, external)
    }

    fn create_resolved_module(&self, resolved: Search, external: bool) -> ResolvedModule {
        match resolved {
            Some(resolved) => ResolvedModule {
                resolved_file_name: resolved.path,
                original_path: resolved.original_path,
                extension: resolved.extension,
                resolved_using_ts_extension: resolved.resolved_using_ts_extension,
                is_external_library_import: external,
                alternate_result: String::new(),
            },
            None => ResolvedModule::default(),
        }
    }

    fn original_and_resolved_file_name(&self, file_name: &str) -> (String, String) {
        let real = normalize_path(&self.host().realpath(file_name));
        if paths_equal(file_name, &real) {
            (String::new(), file_name.to_string())
        } else {
            (file_name.to_string(), real)
        }
    }

    fn try_load_module_using_optional_resolution_settings(&mut self) -> Search {
        if let Some(resolved) = self.try_load_module_using_paths_if_eligible() {
            return Some(resolved);
        }
        if !is_external_module_name_relative(&self.name) { None } else { self.try_load_module_using_root_dirs() }
    }

    fn try_load_module_using_paths_if_eligible(&mut self) -> Search {
        if self.config().paths.is_empty() || path_is_relative(&self.name) {
            return None;
        }
        let base = self.config().paths_base_path.clone();
        let paths = self.config().paths.clone();
        let (extensions, name) = (self.extensions, self.name.clone());
        self.try_load_module_using_paths(extensions, &name, &base, &paths, &mut |state, extensions, candidate| {
            state.node_load_module_by_relative_name(extensions, candidate, true)
        })
    }

    fn try_load_module_using_paths(
        &mut self,
        extensions: u8,
        module_name: &str,
        containing_directory: &str,
        paths: &[(String, Vec<String>)],
        loader: &mut dyn FnMut(&mut Self, u8, &str) -> Search,
    ) -> Search {
        let pattern = match_pattern_or_exact(paths, module_name)?;
        let matched_star = pattern.matched_text(module_name).to_string();
        let substitutions =
            paths.iter().find(|(key, _)| *key == pattern.text).map(|(_, targets)| targets.clone()).unwrap_or_default();
        for substitution in substitutions {
            let path = substitution.replacen('*', &matched_star, 1);
            let candidate = normalize_path(&combine(containing_directory, &path));
            let extension_from_substitution = try_get_extension_from_path(&substitution);
            if !extension_from_substitution.is_empty()
                && let Some(path) = self.try_file(&candidate)
            {
                return Some(Resolved {
                    path,
                    extension: extension_from_substitution.to_string(),
                    ..Resolved::default()
                });
            }
            let saved = self.candidate_ending_is_from_config;
            if !extension_from_substitution.is_empty() {
                self.candidate_ending_is_from_config = true;
            }
            let resolved = loader(self, extensions, &candidate);
            self.candidate_ending_is_from_config = saved;
            if resolved.is_some() {
                return resolved;
            }
        }
        None
    }

    fn try_load_module_using_root_dirs(&mut self) -> Search {
        let root_dirs = self.config().root_dirs.clone();
        if root_dirs.is_empty() {
            return None;
        }
        let candidate = normalize_path(&combine(&self.containing_directory, &self.name));
        let mut matched_root_dir: Option<String> = None;
        let mut matched_prefix = String::new();
        for root_dir in &root_dirs {
            let normalized = ensure_trailing_directory_separator(&normalize_path(root_dir));
            if candidate.starts_with(&normalized) && (matched_prefix.is_empty() || matched_prefix.len() < normalized.len()) {
                matched_prefix = normalized;
                matched_root_dir = Some(root_dir.clone());
            }
        }
        if matched_prefix.is_empty() {
            return None;
        }
        let suffix = candidate[matched_prefix.len()..].to_string();
        let extensions = self.extensions;
        if let Some(resolved) = self.node_load_module_by_relative_name(extensions, &candidate, true) {
            return Some(resolved);
        }
        for root_dir in &root_dirs {
            if Some(root_dir) == matched_root_dir.as_ref() {
                continue;
            }
            let candidate = combine(&normalize_path(root_dir), &suffix);
            if let Some(resolved) = self.node_load_module_by_relative_name(extensions, &candidate, true) {
                return Some(resolved);
            }
        }
        None
    }

    fn node_load_module_by_relative_name(&mut self, extensions: u8, candidate: &str, consider_package_json: bool) -> Search {
        if !has_trailing_directory_separator(candidate) {
            let parent = get_directory_path(candidate);
            if !self.host().directory_exists(&parent) {
                return None;
            }
            if let Some(resolved) = self.load_module_from_file(extensions, candidate) {
                return Some(resolved);
            }
        }
        if !self.host().directory_exists(candidate) {
            return None;
        }
        if !self.esm_mode {
            return self.load_node_module_from_directory(extensions, candidate, consider_package_json);
        }
        None
    }

    fn load_module_from_file(&mut self, extensions: u8, candidate: &str) -> Search {
        if let Some(resolved) = self.load_module_from_file_no_implicit_extensions(extensions, candidate) {
            return Some(resolved);
        }
        if !self.esm_mode {
            return self.try_adding_extensions(candidate, extensions, "");
        }
        None
    }

    fn load_module_from_file_no_implicit_extensions(&mut self, extensions: u8, candidate: &str) -> Search {
        if !get_base_file_name(candidate).contains('.') {
            return None;
        }
        let mut extensionless = remove_file_extension(candidate);
        if extensionless.len() == candidate.len() {
            extensionless = &candidate[..candidate.rfind('.').unwrap_or(candidate.len())];
        }
        let extension = candidate[extensionless.len()..].to_string();
        let extensionless = extensionless.to_string();
        self.try_adding_extensions(&extensionless, extensions, &extension)
    }

    fn try_adding_extensions(&mut self, extensionless: &str, extensions: u8, original_extension: &str) -> Search {
        let directory = get_directory_path(extensionless);
        if !directory.is_empty() && !self.host().directory_exists(&directory) {
            return None;
        }
        let try_all = |state: &mut Self, candidates: &[(u8, &str, bool)]| -> Search {
            for (needed, extension, using_ts) in candidates {
                if extensions & needed != 0
                    && let Some(resolved) = state.try_extension(extension, extensionless, *using_ts)
                {
                    return Some(resolved);
                }
            }
            None
        };
        match original_extension {
            EXTENSION_MJS | EXTENSION_MTS | EXTENSION_DMTS => {
                let ts = original_extension == EXTENSION_MTS || original_extension == EXTENSION_DMTS;
                try_all(self, &[(TYPESCRIPT, EXTENSION_MTS, ts), (DECLARATION, EXTENSION_DMTS, ts), (JAVASCRIPT, EXTENSION_MJS, false)])
            }
            EXTENSION_CJS | EXTENSION_CTS | EXTENSION_DCTS => {
                let ts = original_extension == EXTENSION_CTS || original_extension == EXTENSION_DCTS;
                try_all(self, &[(TYPESCRIPT, EXTENSION_CTS, ts), (DECLARATION, EXTENSION_DCTS, ts), (JAVASCRIPT, EXTENSION_CJS, false)])
            }
            EXTENSION_JSON => try_all(self, &[(DECLARATION, ".d.json.ts", false), (JSON, EXTENSION_JSON, false)]),
            EXTENSION_TSX | EXTENSION_JSX => {
                let ts = original_extension == EXTENSION_TSX;
                try_all(
                    self,
                    &[
                        (TYPESCRIPT, EXTENSION_TSX, ts),
                        (TYPESCRIPT, EXTENSION_TS, ts),
                        (DECLARATION, EXTENSION_DTS, ts),
                        (JAVASCRIPT, EXTENSION_JSX, false),
                        (JAVASCRIPT, EXTENSION_JS, false),
                    ],
                )
            }
            EXTENSION_TS | EXTENSION_DTS | EXTENSION_JS | "" => {
                let ts = original_extension == EXTENSION_TS || original_extension == EXTENSION_DTS;
                try_all(
                    self,
                    &[
                        (TYPESCRIPT, EXTENSION_TS, ts),
                        (TYPESCRIPT, EXTENSION_TSX, ts),
                        (DECLARATION, EXTENSION_DTS, ts),
                        (JAVASCRIPT, EXTENSION_JS, false),
                        (JAVASCRIPT, EXTENSION_JSX, false),
                    ],
                )
            }
            _ => {
                if extensions & DECLARATION != 0 && !is_declaration_file_name(&format!("{extensionless}{original_extension}")) {
                    return self.try_extension(&format!(".d{original_extension}.ts"), extensionless, false);
                }
                None
            }
        }
    }

    fn try_extension(&mut self, extension: &str, extensionless: &str, resolved_using_ts_extension: bool) -> Search {
        let path = self.try_file(&format!("{extensionless}{extension}"))?;
        Some(Resolved {
            path,
            extension: extension.to_string(),
            original_path: String::new(),
            resolved_using_ts_extension: !self.candidate_ending_is_from_config && resolved_using_ts_extension,
        })
    }

    fn try_file(&self, file_name: &str) -> Option<String> {
        let suffixes = &self.config().module_suffixes;
        if suffixes.is_empty() {
            return self.host().file_exists(file_name).then(|| file_name.to_string());
        }
        let extension = try_get_extension_from_path(file_name);
        let without = &file_name[..file_name.len() - extension.len()];
        suffixes
            .iter()
            .map(|suffix| format!("{without}{suffix}{extension}"))
            .find(|path| self.host().file_exists(path))
    }

    fn load_node_module_from_directory(&mut self, extensions: u8, candidate: &str, consider_package_json: bool) -> Search {
        let info = if consider_package_json { self.resolver.package_json_info(candidate) } else { None };
        self.load_node_module_from_directory_worker(extensions, candidate, info.as_ref())
    }

    fn load_node_module_from_directory_worker(
        &mut self,
        extensions: u8,
        candidate: &str,
        package_info: Option<&PackageJsonInfo>,
    ) -> Search {
        let mut package_file: Option<String> = None;
        let mut version_paths = None;
        if let Some(info) = package_info {
            version_paths = info.contents.version_paths().cloned();
            if paths_equal(candidate, &info.package_directory) {
                package_file = self.package_file(extensions, info);
            }
        }
        let package_type_is_module =
            package_info.is_some_and(|info| info.contents.type_field.value.as_deref() == Some("module"));
        let has_package_info = package_info.is_some();
        let package_file_value = package_file.clone().unwrap_or_default();
        let mut loader = |state: &mut Self, extensions: u8, candidate: &str| -> Search {
            if let Some(resolved) = state.load_file_name_from_package_json_field(extensions, candidate, &package_file_value) {
                return Some(resolved);
            }
            let expanded = if extensions == DECLARATION { TYPESCRIPT | DECLARATION } else { extensions };
            let saved_esm = state.esm_mode;
            let saved_config = state.candidate_ending_is_from_config;
            state.candidate_ending_is_from_config = true;
            if has_package_info && !package_type_is_module {
                state.esm_mode = false;
            }
            let result = state.node_load_module_by_relative_name(expanded, candidate, false);
            state.esm_mode = saved_esm;
            state.candidate_ending_is_from_config = saved_config;
            result
        };
        let index_path = combine(candidate, "index");
        if let Some(version_paths) = &version_paths
            && package_file.as_ref().is_none_or(|file| contains_path(candidate, file, ""))
        {
            let module_name = match &package_file {
                Some(file) => get_relative_path_from_directory(candidate, file),
                None => get_relative_path_from_directory(candidate, &index_path),
            };
            if let Some(resolved) =
                self.try_load_module_using_paths(extensions, &module_name, candidate, &version_paths.paths, &mut loader)
            {
                return Some(resolved);
            }
        }
        if let Some(file) = &package_file
            && let Some(resolved) = loader(self, extensions, file)
        {
            return Some(resolved);
        }
        if !self.esm_mode {
            if !self.host().directory_exists(candidate) {
                return None;
            }
            return self.load_module_from_file(extensions, &index_path);
        }
        None
    }

    fn load_file_name_from_package_json_field(&mut self, extensions: u8, candidate: &str, package_json_value: &str) -> Search {
        if (extensions & TYPESCRIPT != 0 && has_implementation_ts_file_extension(candidate))
            || (extensions & DECLARATION != 0 && is_declaration_file_name(candidate))
        {
            let path = self.try_file(candidate)?;
            let extension = try_extract_ts_extension(&path);
            return Some(Resolved {
                resolved_using_ts_extension: package_json_value.ends_with('*') && !extension.is_empty(),
                extension: extension.to_string(),
                path,
                original_path: String::new(),
            });
        }
        self.load_module_from_file_no_implicit_extensions(extensions, candidate)
    }

    fn package_file(&self, extensions: u8, info: &PackageJsonInfo) -> Option<String> {
        let field = |field: &packagejson::StringField| -> Option<String> {
            let value = field.value.as_deref().filter(|value| !value.is_empty())?;
            Some(normalize_path(&combine(&info.package_directory, value)))
        };
        if extensions & DECLARATION != 0 {
            if let Some(file) = field(&info.contents.typings) {
                return Some(file);
            }
            if let Some(file) = field(&info.contents.types) {
                return Some(file);
            }
        }
        if extensions & (IMPLEMENTATION | DECLARATION) != 0 {
            return field(&info.contents.main);
        }
        None
    }

    fn package_scope_for_path(&self, directory: &str) -> Option<PackageJsonInfo> {
        ancestor_directories(directory).into_iter().find_map(|directory| self.resolver.package_json_info(&directory))
    }

    fn condition_matches(&self, condition: &str) -> bool {
        if condition == "default" || self.conditions.iter().any(|active| active == condition) {
            return true;
        }
        if !self.conditions.iter().any(|active| active == "types") {
            return false;
        }
        condition
            .strip_prefix("types@")
            .and_then(crate::semver::VersionRange::parse)
            .is_some_and(|range| range.test(crate::package_resolution::typescript_version()))
    }

    fn resolve_from_type_root(&mut self) -> Search {
        let type_roots = self.config().type_roots.clone()?;
        for type_root in type_roots {
            let candidate = self.candidate_from_type_root(&type_root);
            if !self.host().directory_exists(&type_root) {
                continue;
            }
            if let Some(resolved) = self.load_module_from_file(DECLARATION, &candidate) {
                return Some(resolved);
            }
            if let Some(resolved) = self.load_node_module_from_directory(DECLARATION, &candidate, true) {
                return Some(resolved);
            }
        }
        None
    }

    fn candidate_from_type_root(&self, type_root: &str) -> String {
        let name = if type_root.ends_with("/node_modules/@types") || type_root.ends_with("/node_modules/@types/") {
            mangle_scoped_package_name(&self.name)
        } else {
            self.name.clone()
        };
        combine(type_root, &name)
    }

    fn resolve_type_reference_directive(
        &mut self,
        type_roots: &[String],
        from_config: bool,
        from_inferred: bool,
    ) -> ResolvedTypeReferenceDirective {
        for type_root in type_roots {
            let candidate = self.candidate_from_type_root(type_root);
            if !self.host().directory_exists(type_root) {
                continue;
            }
            if from_config && let Some(resolved) = self.load_module_from_file(DECLARATION, &candidate) {
                return self.create_resolved_type_reference_directive(Some(resolved), true);
            }
            if let Some(resolved) = self.load_node_module_from_directory(DECLARATION, &candidate, true) {
                return self.create_resolved_type_reference_directive(Some(resolved), true);
            }
        }
        let resolved = if !from_config || !from_inferred {
            if !is_external_module_name_relative(&self.name) {
                self.load_module_from_nearest_node_modules_directory(false)
            } else {
                let candidate = normalize_path_for_cjs_resolution(&self.containing_directory, &self.name);
                self.node_load_module_by_relative_name(DECLARATION, &candidate, true)
            }
        } else {
            None
        };
        self.create_resolved_type_reference_directive(resolved, false)
    }

    fn create_resolved_type_reference_directive(&self, resolved: Search, primary: bool) -> ResolvedTypeReferenceDirective {
        let Some(resolved) = resolved.filter(|resolved| !resolved.path.is_empty()) else {
            return ResolvedTypeReferenceDirective::default();
        };
        let mut result = ResolvedTypeReferenceDirective {
            is_external_library_import: resolved.path.contains("/node_modules/"),
            resolved_file_name: resolved.path.clone(),
            original_path: String::new(),
            primary,
        };
        if !self.config().preserve_symlinks {
            let (original, real) = self.original_and_resolved_file_name(&resolved.path);
            if !original.is_empty() {
                result.resolved_file_name = real;
                result.original_path = original;
            }
        }
        result
    }
}

/// `GetConditions`.
fn conditions(config: &ResolverConfig, mode: ResolutionMode) -> Vec<String> {
    let mode = if mode == ResolutionMode::None && config.kind == ResolutionKind::Bundler { ResolutionMode::Esm } else { mode };
    let mut conditions = vec![if mode == ResolutionMode::Esm { "import" } else { "require" }.to_string()];
    if !config.no_dts_resolution {
        conditions.push("types".to_string());
    }
    if config.kind != ResolutionKind::Bundler {
        conditions.push("node".to_string());
    }
    conditions.extend(config.custom_conditions.iter().cloned());
    conditions
}
