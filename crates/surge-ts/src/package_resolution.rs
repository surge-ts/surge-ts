//! Modern package declaration resolution: `exports`/`imports` maps,
//! conditional targets, wildcard/pattern keys, and `typesVersions`.
//!
//! These are pure functions over already-parsed `package.json` JSON values. They
//! return declaration *candidate target strings* (relative to the package root)
//! and never touch the filesystem; the caller in [`crate::package_declarations`]
//! is responsible for joining against the package directory, probing declaration
//! extensions, and loading the result.
//!
//! Runtime JavaScript resolution and full Node loader parity are out of scope —
//! this only finds `.d.ts` / `.d.mts` / `.d.cts` entrypoints (and the runtime
//! target a declaration would sit beside, for stub/no-cascade bookkeeping).

use serde_json::Value;
use surge_ts_config::ModuleResolutionKind;

use crate::semver::{Version, VersionRange};

/// Inputs that steer condition selection, derived from `compilerOptions`.
#[derive(Debug, Clone)]
pub struct ResolverOptions {
    pub module_resolution: ModuleResolutionKind,
    /// `resolvePackageJsonExports` — when false, `exports` is bypassed.
    pub resolve_exports: bool,
    /// `resolvePackageJsonImports` — when false, `imports` is bypassed.
    pub resolve_imports: bool,
    /// `customConditions`, in configured priority order.
    pub custom_conditions: Vec<String>,
    /// tsconfig `paths` patterns. A bare specifier that matches a pattern and
    /// resolves to an existing mapped file is handled by path mapping, so the
    /// package-declaration walk must not run for it (tsc only falls back to
    /// `node_modules` when no mapped target exists).
    pub path_mappings: Vec<surge_ts_config::PathMapping>,
    /// Base directory `paths` targets resolve against: `baseUrl` when set,
    /// else the config directory.
    pub path_mapping_base: Option<std::path::PathBuf>,
    /// tsgo's `GetEmitModuleKind`, which decides a file's resolution mode.
    pub emit_module: surge_ts_config::ModuleKind,
    /// `resolveJsonModule`: a package entry naming a `.json` file resolves to
    /// it.
    pub resolve_json_module: bool,
}

impl Default for ResolverOptions {
    fn default() -> Self {
        Self {
            module_resolution: ModuleResolutionKind::Bundler,
            resolve_exports: true,
            resolve_imports: true,
            custom_conditions: Vec::new(),
            path_mappings: Vec::new(),
            path_mapping_base: None,
            emit_module: surge_ts_config::ModuleKind::ES2022,
            resolve_json_module: true,
        }
    }
}

impl ResolverOptions {
    /// The condition names that are "on" for declaration resolution, besides the
    /// always-matching `default`. Mirrors TypeScript's `getConditions`:
    ///
    /// * the mode condition (`import` for an ESM-mode resolution, `require`
    ///   otherwise; bundler reads an unset mode as ESM before this point),
    /// * `types` (declaration resolution always opts in),
    /// * `node` for node16/nodenext (never under bundler),
    /// * then `customConditions` in configured order.
    ///
    /// Priority between branches of a conditional object is decided by the
    /// *package author's key order* (Node semantics), not by this list — this is
    /// only the membership set.
    pub fn active_conditions(&self, importer_is_esm: bool) -> Vec<String> {
        let mut conditions = Vec::new();
        let is_bundler = self.module_resolution == ModuleResolutionKind::Bundler;
        if importer_is_esm {
            conditions.push("import".to_string());
        } else {
            conditions.push("require".to_string());
        }
        conditions.push("types".to_string());
        if !is_bundler {
            conditions.push("node".to_string());
        }
        conditions.extend(self.custom_conditions.iter().cloned());
        conditions
    }
}

/// Whether an `exports`/`imports` object is a subpath map (keys start with `.`
/// for exports, `#` for imports) rather than a bare conditional target.
fn is_subpath_map(map: &serde_json::Map<String, Value>, prefix: char) -> bool {
    map.keys().any(|key| key.starts_with(prefix))
}

/// Resolve a package `exports` field for `subpath_key` (`"."` or `"./sub/path"`)
/// to a target template string (relative path, `*` already substituted).
///
/// Returns `None` when the subpath is blocked — i.e. `exports` is present but no
/// key matches, or the matched condition target is `null`. The caller treats
/// `None` as "do not fall back to legacy file probing".
pub fn select_export_target(
    exports: &Value,
    subpath_key: &str,
    conditions: &[String],
) -> Option<String> {
    select_export_targets(exports, subpath_key, conditions)
        .into_iter()
        .next()
}

/// Like [`select_export_target`], but returns *every* reachable target in
/// priority order. tsc falls through to the next matching condition when the
/// selected target does not resolve to a file (e.g. a source condition naming a
/// file the published package does not ship), so the caller must be able to
/// probe each candidate in turn rather than committing to the first.
pub fn select_export_targets(
    exports: &Value,
    subpath_key: &str,
    conditions: &[String],
) -> Vec<String> {
    match exports {
        Value::String(_) | Value::Array(_) => {
            // Bare string / array sugar: the entire value is the `"."` target.
            if subpath_key == "." {
                collect_targets(exports, conditions)
            } else {
                Vec::new()
            }
        }
        Value::Object(map) => {
            if is_subpath_map(map, '.') {
                select_all_from_subpath_map(map, subpath_key, conditions)
            } else if subpath_key == "." {
                // Conditions-only object is `"."` sugar.
                collect_targets(exports, conditions)
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// Resolve a package `imports` field (`#alias`) for `specifier` to every
/// reachable target template string, in priority order (see
/// [`select_export_targets`]). Only `#`-prefixed specifiers are valid here.
pub fn select_import_targets(
    imports: &Value,
    specifier: &str,
    conditions: &[String],
) -> Vec<String> {
    let Value::Object(map) = imports else {
        return Vec::new();
    };
    if !is_subpath_map(map, '#') {
        return Vec::new();
    }
    select_all_from_subpath_map(map, specifier, conditions)
}

/// Match `key` against an `exports`/`imports`/`typesVersions` subpath map and
/// resolve the winning entry's conditions, substituting a captured `*`.
///
/// Exact keys win over patterns; among patterns the longest static prefix wins
/// (Node's specificity rule). A single `*` is supported; keys with more than one
/// `*` are ignored.
fn select_all_from_subpath_map(
    map: &serde_json::Map<String, Value>,
    key: &str,
    conditions: &[String],
) -> Vec<String> {
    // A specifier that writes a `*` itself names no exact key
    // (`loadModuleFromExportsOrImports`).
    if !key.contains('*')
        && let Some(value) = map.get(key)
    {
        return collect_targets(value, conditions);
    }

    let mut best: Option<(&str, &Value, String)> = None;
    for (pattern, value) in map {
        let Some((prefix, suffix)) = single_star_parts(pattern) else {
            continue;
        };
        if let Some(captured) = match_star(key, prefix, suffix) {
            let more_specific = match &best {
                Some((best_prefix, _, _)) => prefix.len() > best_prefix.len(),
                None => true,
            };
            if more_specific {
                best = Some((prefix, value, captured));
            }
        }
    }

    let Some((_, value, captured)) = best else {
        return Vec::new();
    };
    collect_targets(value, conditions)
        .into_iter()
        .map(|target| substitute_star(&target, &captured))
        .collect()
}

/// Walk a conditional target (string / array / nested condition object) and
/// collect every reachable target string in priority order. `default` always
/// matches; other keys match only when present in `conditions`. Object key order
/// (the package author's order) decides priority. A matched `null` target is an
/// explicit block: collection stops there and later conditions are not
/// consulted, matching Node (tsc only falls through when a matched target fails
/// to *resolve*, never past an explicit `null`).
fn collect_targets(value: &Value, conditions: &[String]) -> Vec<String> {
    let mut targets = Vec::new();
    collect_targets_into(value, conditions, &mut targets);
    targets
}

/// Returns `false` when collection hit an explicit `null` block and the caller
/// must not consult lower-priority alternatives.
fn collect_targets_into(value: &Value, conditions: &[String], targets: &mut Vec<String>) -> bool {
    match value {
        Value::String(s) => {
            targets.push(s.clone());
            true
        }
        // Array entries are fallbacks: unsupported/blocked entries are skipped
        // rather than blocking (Node ignores invalid array members).
        Value::Array(items) => {
            for item in items {
                collect_targets_into(item, conditions, targets);
            }
            true
        }
        Value::Object(map) => {
            for (condition, target) in map {
                if condition_matches(condition, conditions) {
                    if !collect_targets_into(target, conditions, targets) {
                        return false;
                    }
                }
            }
            true
        }
        Value::Null => false,
        _ => true,
    }
}

/// tsgo's `conditionMatches`: `default` and the active conditions, plus a
/// `types@<range>` key the TypeScript version satisfies once `types` is active.
fn condition_matches(condition: &str, conditions: &[String]) -> bool {
    if condition == "default" || conditions.iter().any(|active| active == condition) {
        return true;
    }
    conditions.iter().any(|active| active == "types")
        && is_applicable_versioned_types_key(condition)
}

/// tsgo's `IsApplicableVersionedTypesKey`.
fn is_applicable_versioned_types_key(key: &str) -> bool {
    key.strip_prefix("types@")
        .and_then(VersionRange::parse)
        .is_some_and(|range| range.test(typescript_version()))
}

static EMULATED_VERSION: std::sync::OnceLock<Version> = std::sync::OnceLock::new();

/// Test `typesVersions` and `types@` ranges against `version` instead of the
/// bundled release, for the rest of the process: a host that presents itself
/// as another TypeScript (the Compiler API's `typescript@6`) resolves packages
/// as that one does. The first call wins.
pub(crate) fn emulate_typescript_version(version: &str) {
    if let Some(version) = Version::parse(version) {
        let _ = EMULATED_VERSION.set(version);
    }
}

/// The version `typesVersions` and `types@` ranges are tested against: the
/// TypeScript release the bundled libs come from, as tsc tests its own.
pub(crate) fn typescript_version() -> &'static Version {
    if let Some(emulated) = EMULATED_VERSION.get() {
        return emulated;
    }
    static VERSION: std::sync::OnceLock<Version> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        let bundled = surge_ts_checker::lowlevel::bundled_typescript_version();
        Version::parse(bundled)
            .unwrap_or_else(|| panic!("bundled TypeScript version `{bundled}` is not semver"))
    })
}

/// Split a key with exactly one `*` into `(prefix, suffix)`. Returns `None` for
/// keys with zero or more than one `*`.
fn single_star_parts(key: &str) -> Option<(&str, &str)> {
    let first = key.find('*')?;
    if key[first + 1..].contains('*') {
        return None;
    }
    Some((&key[..first], &key[first + 1..]))
}

/// If `value` matches `prefix` + `*` + `suffix`, return the captured `*` text.
fn match_star(value: &str, prefix: &str, suffix: &str) -> Option<String> {
    if value.len() < prefix.len() + suffix.len() {
        return None;
    }
    if !value.starts_with(prefix) || !value.ends_with(suffix) {
        return None;
    }
    Some(value[prefix.len()..value.len() - suffix.len()].to_string())
}

/// Replace every `*` in `template` with `captured`.
fn substitute_star(template: &str, captured: &str) -> String {
    template.replace('*', captured)
}

/// Resolve a `typesVersions` field for `subpath` (a package-relative path such as
/// `"index.d.ts"` for the root, or `"server"` / `"features/auth"` for a subpath)
/// to its ordered list of candidate target templates (with `*` substituted).
///
/// tsgo's `GetVersionPaths`: the first key whose npm range the TypeScript
/// version satisfies wins, an unparsable key is skipped, and a matching key
/// whose value is not an object leaves the package with no version paths.
pub fn types_versions_candidates(types_versions: &Value, subpath: &str) -> Vec<String> {
    let Value::Object(version_map) = types_versions else {
        return Vec::new();
    };

    for (version_range, mapping) in version_map {
        let Some(range) = VersionRange::parse(version_range) else {
            continue;
        };
        if !range.test(typescript_version()) {
            continue;
        }
        return match mapping {
            Value::Object(paths) => match_types_versions_paths(paths, subpath),
            _ => Vec::new(),
        };
    }

    Vec::new()
}

/// Match a `typesVersions` paths object (`{ pattern: [targets] }`) against
/// `subpath`, exact keys first then longest-prefix pattern, returning the
/// substituted target list in order.
fn match_types_versions_paths(
    paths: &serde_json::Map<String, Value>,
    subpath: &str,
) -> Vec<String> {
    if let Some(targets) = paths.get(subpath) {
        return string_list(targets);
    }

    let mut best: Option<(&str, &Value, String)> = None;
    for (pattern, targets) in paths {
        let Some((prefix, suffix)) = single_star_parts(pattern) else {
            continue;
        };
        if let Some(captured) = match_star(subpath, prefix, suffix) {
            let more_specific = match &best {
                Some((best_prefix, _, _)) => prefix.len() > best_prefix.len(),
                None => true,
            };
            if more_specific {
                best = Some((prefix, targets, captured));
            }
        }
    }

    match best {
        Some((_, targets, captured)) => string_list(targets)
            .into_iter()
            .map(|target| substitute_star(&target, &captured))
            .collect(),
        None => Vec::new(),
    }
}

fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundler_conditions() -> Vec<String> {
        ResolverOptions::default().active_conditions(true)
    }

    fn first_import_target(
        imports: &Value,
        specifier: &str,
        conditions: &[String],
    ) -> Option<String> {
        select_import_targets(imports, specifier, conditions)
            .into_iter()
            .next()
    }

    #[test]
    fn active_conditions_bundler_follows_the_mode_without_node() {
        let opts = ResolverOptions::default();
        assert_eq!(opts.active_conditions(true), vec!["import", "types"]);
        assert_eq!(opts.active_conditions(false), vec!["require", "types"]);
    }

    #[test]
    fn active_conditions_node16_uses_require_and_node() {
        let opts = ResolverOptions {
            module_resolution: ModuleResolutionKind::Node16,
            ..ResolverOptions::default()
        };
        assert_eq!(
            opts.active_conditions(false),
            vec!["require", "types", "node"]
        );
        assert_eq!(
            opts.active_conditions(true),
            vec!["import", "types", "node"]
        );
    }

    #[test]
    fn active_conditions_appends_custom_conditions() {
        let opts = ResolverOptions {
            custom_conditions: vec!["development".to_string()],
            ..ResolverOptions::default()
        };
        assert_eq!(
            opts.active_conditions(true),
            vec!["import", "types", "development"]
        );
    }

    #[test]
    fn export_string_sugar_is_root_only() {
        let exports = json!("./dist/index.d.ts");
        assert_eq!(
            select_export_target(&exports, ".", &bundler_conditions()),
            Some("./dist/index.d.ts".to_string())
        );
        assert_eq!(
            select_export_target(&exports, "./server", &bundler_conditions()),
            None
        );
    }

    #[test]
    fn export_conditions_prefer_types_by_author_order() {
        let exports = json!({
            ".": {
                "types": "./dist/index.d.ts",
                "import": "./dist/index.mjs",
                "require": "./dist/index.cjs"
            }
        });
        assert_eq!(
            select_export_target(&exports, ".", &bundler_conditions()),
            Some("./dist/index.d.ts".to_string())
        );
    }

    #[test]
    fn export_subpath_basic() {
        let exports = json!({
            "./server": { "types": "./dist/server.d.ts", "default": "./dist/server.js" }
        });
        assert_eq!(
            select_export_target(&exports, "./server", &bundler_conditions()),
            Some("./dist/server.d.ts".to_string())
        );
        // Unlisted subpath is blocked.
        assert_eq!(
            select_export_target(&exports, "./other", &bundler_conditions()),
            None
        );
        // Root has no entry → blocked.
        assert_eq!(
            select_export_target(&exports, ".", &bundler_conditions()),
            None
        );
    }

    #[test]
    fn export_pattern_substitutes_capture() {
        let exports = json!({
            "./features/*": { "types": "./dist/features/*.d.ts", "default": "./dist/features/*.js" }
        });
        assert_eq!(
            select_export_target(&exports, "./features/auth", &bundler_conditions()),
            Some("./dist/features/auth.d.ts".to_string())
        );
        assert_eq!(
            select_export_target(&exports, "./features/nested/deep", &bundler_conditions()),
            Some("./dist/features/nested/deep.d.ts".to_string())
        );
    }

    #[test]
    fn export_exact_wins_over_pattern() {
        let exports = json!({
            "./features/*": { "types": "./dist/features/*.d.ts" },
            "./features/special": { "types": "./dist/special.d.ts" }
        });
        assert_eq!(
            select_export_target(&exports, "./features/special", &bundler_conditions()),
            Some("./dist/special.d.ts".to_string())
        );
    }

    #[test]
    fn export_longest_prefix_pattern_wins() {
        let exports = json!({
            "./*": { "types": "./dist/*.d.ts" },
            "./features/*": { "types": "./dist/features/*.d.ts" }
        });
        assert_eq!(
            select_export_target(&exports, "./features/auth", &bundler_conditions()),
            Some("./dist/features/auth.d.ts".to_string())
        );
    }

    #[test]
    fn export_custom_condition_selected_over_fallback() {
        let exports = json!({
            ".": {
                "development": { "types": "./dist/dev.d.ts", "default": "./dist/dev.js" },
                "types": "./dist/prod.d.ts",
                "default": "./dist/prod.js"
            }
        });
        let opts = ResolverOptions {
            custom_conditions: vec!["development".to_string()],
            ..ResolverOptions::default()
        };
        assert_eq!(
            select_export_target(&exports, ".", &opts.active_conditions(true)),
            Some("./dist/dev.d.ts".to_string())
        );
        // Without the custom condition, the later `types` branch wins.
        assert_eq!(
            select_export_target(&exports, ".", &bundler_conditions()),
            Some("./dist/prod.d.ts".to_string())
        );
    }

    #[test]
    fn export_null_target_blocks() {
        let exports = json!({ "./blocked": null });
        assert_eq!(
            select_export_target(&exports, "./blocked", &bundler_conditions()),
            None
        );
    }

    #[test]
    fn import_field_exact_and_pattern() {
        let imports = json!({
            "#internal": { "types": "./dist/internal.d.ts", "default": "./dist/internal.js" },
            "#features/*": { "types": "./dist/features/*.d.ts" }
        });
        assert_eq!(
            first_import_target(&imports, "#internal", &bundler_conditions()),
            Some("./dist/internal.d.ts".to_string())
        );
        assert_eq!(
            first_import_target(&imports, "#features/auth", &bundler_conditions()),
            Some("./dist/features/auth.d.ts".to_string())
        );
        assert_eq!(
            first_import_target(&imports, "#missing", &bundler_conditions()),
            None
        );
    }

    #[test]
    fn types_versions_root_star_pattern() {
        let tv = json!({ "*": { "*": ["dist/*"] } });
        assert_eq!(
            types_versions_candidates(&tv, "index.d.ts"),
            vec!["dist/index.d.ts".to_string()]
        );
    }

    #[test]
    fn types_versions_subpath_exact_and_pattern() {
        let tv = json!({
            "*": {
                "server": ["dist/server.d.ts"],
                "features/*": ["dist/features/*.d.ts"]
            }
        });
        assert_eq!(
            types_versions_candidates(&tv, "server"),
            vec!["dist/server.d.ts".to_string()]
        );
        assert_eq!(
            types_versions_candidates(&tv, "features/auth"),
            vec!["dist/features/auth.d.ts".to_string()]
        );
        assert!(types_versions_candidates(&tv, "missing").is_empty());
    }

    #[test]
    fn types_versions_picks_first_matching_version() {
        let tv = json!({
            "<7.0": { "*": ["legacy/*"] },
            ">=7.0": { "*": ["dist/*"] }
        });
        assert_eq!(
            types_versions_candidates(&tv, "index.d.ts"),
            vec!["dist/index.d.ts".to_string()]
        );
    }

    #[test]
    fn types_versions_skips_an_unparsable_range() {
        let tv = json!({
            "latest": { "*": ["wrong/*"] },
            "*": { "*": ["dist/*"] }
        });
        assert_eq!(
            types_versions_candidates(&tv, "index.d.ts"),
            vec!["dist/index.d.ts".to_string()]
        );
    }

    #[test]
    fn versioned_types_conditions_apply_with_types_active() {
        let exports = json!({
            "./yep": { "types@>=4": "./types/foo.d.ts" },
            "./nah": { "types@<4": "./types/foo.d.ts" }
        });
        let conditions = bundler_conditions();
        assert_eq!(
            select_export_target(&exports, "./yep", &conditions),
            Some("./types/foo.d.ts".to_string())
        );
        assert_eq!(select_export_target(&exports, "./nah", &conditions), None);
        assert_eq!(
            select_export_target(&exports, "./yep", &["import".to_string()]),
            None
        );
    }
}
