//! Module system: resolution, export tables, import binding, and diagnostics.
//!
//! Split into focused submodules. Shared types live here; each submodule does
//! `use super::*` to reach siblings via the re-exports below.

use std::sync::Arc;

use surge_ts_types::{Type, TypeCopyReason, with_type_copy_reason};

use crate::symbols::{SymbolInfo, SymbolTable, TypeDeclarationTable};

pub(crate) mod candidates;
mod diagnostics;
pub(crate) mod exports;
pub(crate) use exports::{
    peel_exported_statement, statics_merged_into, statics_merged_into_symbol,
};
pub(crate) mod imports;
mod node_builtins;
mod resolution;
mod synthetic_default;

pub(crate) use diagnostics::*;
pub(crate) use exports::*;
pub(crate) use imports::*;
pub(crate) use node_builtins::*;
pub(crate) use resolution::*;
pub(crate) use synthetic_default::*;

/// tsc's `InternalSymbolNameExportEquals`. An `export =` module's type table
/// keys the assigned entity's type meaning under this name and each of its
/// namespace members under `export=.<member>`, which is what an
/// `import x = require(...)` binds. No written name can reach these keys.
pub(crate) const EXPORT_ASSIGNMENT_NAME: &str = "export=";

/// Whether `key` is one of the [`EXPORT_ASSIGNMENT_NAME`] keys, which belong to
/// the module that declares them: `export *` and namespace re-exports do not
/// carry a module's `export =`.
pub(crate) fn is_export_assignment_key(key: &str) -> bool {
    key.strip_prefix(EXPORT_ASSIGNMENT_NAME)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The kind of type-only declaration an alias chain passes through (tsc's
/// `typeOnlyDeclaration`), which picks the diagnostic for a use of its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeOnlyAliasKind {
    /// `import type` — TS1361.
    Import,
    /// `export type { … }` or `export type *` — TS1362.
    Export,
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleResolution {
    pub(crate) resolved_file_index: usize,
    #[allow(dead_code)]
    pub(crate) resolved_file_name: String,
}

#[derive(Debug, Default)]
pub(crate) struct ModuleExportTable {
    pub(crate) type_declarations: Arc<TypeDeclarationTable>,
    pub(crate) symbols: SymbolTable,
    pub(crate) default_symbol: Option<Arc<SymbolInfo>>,
    /// The value bound by a declaration-lite `export = identifier`. Consumed by
    /// `import local = require("specifier")` to bind `local`. Kept distinct from
    /// `default_symbol` so plain ESM default imports do not resolve through it
    /// (no synthetic default / `esModuleInterop`).
    pub(crate) export_assignment_symbol: Option<Arc<SymbolInfo>>,
    /// Whether the module writes `export =`, whether or not surge resolved its
    /// entity: `import x = require()` of it binds that entity, never the
    /// module namespace.
    pub(crate) writes_export_assignment: bool,
    /// The `export =` names an `import x = require()` alias of a module with
    /// no `export =` of its own: the entity is that module's namespace, whose
    /// surface this table adopted.
    pub(crate) export_assignment_names_module: bool,
    pub(crate) namespace_export_object_type: Option<Type>,
    pub(crate) has_unresolved_star_export: bool,
    pub(crate) has_incomplete_declaration_surface: bool,
    /// A shorthand ambient module (`declare module "x";`) as its first
    /// declaration made it: every binding imported from it is the module
    /// itself (tsc's `isShorthandAmbientModuleSymbol`).
    pub(crate) shorthand: bool,
    /// Exports whose alias chain passes through a type-only declaration. Their
    /// values stay in `symbols` — tsc's alias still resolves to them, so a use
    /// of one is reported rather than unresolved — but the module namespace
    /// object leaves them out.
    pub(crate) type_only_exports: Arc<surge_ts_types::fx::FxHashMap<Arc<str>, TypeOnlyAliasKind>>,
}

impl Clone for ModuleExportTable {
    fn clone(&self) -> Self {
        crate::program::record_module_export_table_clone_count();
        if crate::program::counters_enabled() {
            let entry_count = self.symbols.iter_shared().count() as u64
                + u64::from(self.default_symbol.is_some());
            crate::program::record_module_export_entry_clone_count(entry_count);
            crate::program::record_module_export_symbol_handle_copy_count(entry_count);
        }

        Self {
            type_declarations: self.type_declarations.clone(),
            symbols: self.symbols.clone(),
            default_symbol: self.default_symbol.clone(),
            export_assignment_symbol: self.export_assignment_symbol.clone(),
            writes_export_assignment: self.writes_export_assignment,
            export_assignment_names_module: self.export_assignment_names_module,
            namespace_export_object_type: self.namespace_export_object_type.clone(),
            has_unresolved_star_export: self.has_unresolved_star_export,
            has_incomplete_declaration_surface: self.has_incomplete_declaration_surface,
            shorthand: self.shorthand,
            type_only_exports: self.type_only_exports.clone(),
        }
    }
}

impl ModuleExportTable {
    pub(crate) fn clone_with_reason(&self, reason: TypeCopyReason) -> Self {
        with_type_copy_reason(reason, || self.clone())
    }

    /// Records that `name`'s alias chain passes through a type-only
    /// declaration; the nearest one to the export decides the kind.
    pub(crate) fn mark_type_only_export(&mut self, name: &str, kind: TypeOnlyAliasKind) {
        Arc::make_mut(&mut self.type_only_exports)
            .entry(Arc::from(name))
            .or_insert(kind);
    }

    pub(crate) fn get_shared_value(&self, name: &str) -> Option<Arc<SymbolInfo>> {
        if name == "default" {
            return self.default_symbol.as_ref().map(|symbol| {
                crate::program::record_module_export_borrowed_lookup_count();
                crate::program::record_module_export_symbol_handle_copy_count(1);
                symbol.clone()
            });
        }

        self.symbols.get_shared(name).map(|symbol| {
            crate::program::record_module_export_borrowed_lookup_count();
            crate::program::record_module_export_symbol_handle_copy_count(1);
            symbol
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleImportBindings {
    pub(crate) type_declarations: Arc<TypeDeclarationTable>,
    pub(crate) symbols: SymbolTable,
    /// Per-namespace-import type layers (`import * as ns from "m"`). Each holds the
    /// module's exported types re-keyed under the `ns.` alias. They are shared,
    /// cached `Arc`s appended *after* `type_declarations` in the resolution scope
    /// so an explicit local/named binding of the same qualified name still wins.
    /// Materializing them as a scope layer (rather than copying every member into
    /// `type_declarations`) keeps `import * as` of a large barrel O(1) per importer
    /// instead of O(exports).
    pub(crate) namespace_alias_layers: Vec<Arc<TypeDeclarationTable>>,
    /// Local names whose import reaches a type-only export (`export type`
    /// upstream of a plain `import { A }`), with that declaration's kind.
    pub(crate) type_only_aliases: Vec<(Arc<str>, TypeOnlyAliasKind)>,
    /// `import * as` bindings whose module's whole export list surge saw.
    pub(crate) complete_namespace_imports: Vec<Arc<str>>,
    /// Those of them whose module is JavaScript.
    pub(crate) javascript_namespace_imports: Vec<Arc<str>>,
    /// `import x = require` bindings with no namespace meaning: an `export =`
    /// of a variable, function, class, interface or type alias.
    pub(crate) value_only_import_aliases: Vec<Arc<str>>,
}

impl ModuleImportBindings {
    pub(crate) fn clone_with_reason(&self, reason: TypeCopyReason) -> Self {
        with_type_copy_reason(reason, || self.clone())
    }

    /// The type-declaration layers this binding contributes to a resolution scope,
    /// ordered so the primary import table is consulted before namespace aliases.
    pub(crate) fn scope_layers(&self) -> Vec<Arc<TypeDeclarationTable>> {
        let mut layers = Vec::with_capacity(1 + self.namespace_alias_layers.len());
        layers.push(self.type_declarations.clone());
        layers.extend(self.namespace_alias_layers.iter().cloned());
        layers
    }

    /// Whether an import in this file binds `name` at all, as a value or as a
    /// type. A type-only import binds the name too: it shadows a same-named
    /// global, and tsc reports using it as a value as TS1361 rather than as a
    /// UMD-global reference.
    pub(crate) fn binds_name(&self, name: &str) -> bool {
        self.symbols.get_handle(name).is_some() || self.type_declarations.get(name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{CheckerContext, FileKind};
    use crate::program::ParsedProgramFile;

    fn program(files: &[(&str, &str)]) -> Vec<ParsedProgramFile> {
        files
            .iter()
            .map(|(file_name, source_text)| {
                let parsed = surge_ts_syntax::parse_source(source_text, file_name);
                ParsedProgramFile {
                    file_name: parsed.file_name,
                    has_export_default: source_text.contains("export default"),
                    contains_typeof: source_text.contains("typeof"),
                    statements: parsed.statements,
                    parser_errors: parsed.parser_errors,
                    bind_errors: Vec::new(),
                    tsc_bound: false,
                    oxc_aborted: false,
                    tsc_globals: None,
                    shadowed_globals: Vec::new(),
                    no_check: false,
                    plain_js: false,
                    is_module: parsed.is_module,
                    jsdoc_import_module: parsed.jsdoc_import_module,
                    commonjs_module: parsed.commonjs_module,
                    import_call_specifiers: parsed.import_call_specifiers,
                    import_calls: parsed.import_calls,
                    import_attributes: parsed.import_attributes,
                    file_kind: FileKind::RootSource,
                    module_reads: parsed.module_reads,
                definite_writes: parsed.definite_writes,
                    comment_directives: parsed.comment_directives,
                    grammar_diagnostics: parsed.grammar_diagnostics,
                    parenthesized_expressions: parsed.parenthesized_expressions.into(),
                    global_this_starts: parsed.global_this_starts.into(),
                    literal_this_members: parsed.literal_this_members.into(),
                    member_this_starts: parsed.member_this_starts.into(),
                    circular_return_functions: parsed.circular_return_functions.into(),
                    written_member_names: parsed.written_member_names.into(),
                    let_assignments: parsed.let_assignments.into(),
                    json_module_type: parsed.json_module_type,
                    jsx_factory_uses: parsed.jsx_factory_uses,
                    emit_helper_requests: Vec::new(),
                }
            })
            .collect()
    }

    fn resolve_relative_module(
        importer_file_name: &str,
        specifier: &str,
        program_files: &[ParsedProgramFile],
    ) -> Option<ModuleResolution> {
        let file_index_by_identity = program_files
            .iter()
            .enumerate()
            .map(|(index, file)| (canonical_file_identity(&file.file_name).into(), index))
            .collect::<surge_ts_types::fx::FxHashMap<Arc<str>, usize>>();

        super::resolve_relative_module(
            importer_file_name,
            specifier,
            program_files,
            &file_index_by_identity,
        )
    }

    #[test]
    fn module_resolver_relative_same_dir_extensionless() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", "./user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_relative_same_dir_with_ts_extension() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", "./user.ts", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_relative_parent_dir() {
        let files = program(&[("src/index.ts", "export {}"), ("user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", "../user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "user.ts");
    }

    #[test]
    fn module_resolver_relative_dot_segments() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/pages/index.ts", ".././user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_relative_windows_separators() {
        let files = program(&[("src\\index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src\\index.ts", ".\\user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_non_relative_unsupported_loaded_files_only() {
        let files = program(&[("src/index.ts", "export {}")]);
        assert!(resolve_relative_module("src/index.ts", "pkg", &files).is_none());
    }

    #[test]
    fn module_resolver_missing_file() {
        let files = program(&[("src/index.ts", "export {}")]);
        assert!(resolve_relative_module("src/index.ts", "./missing", &files).is_none());
    }

    #[test]
    fn module_resolver_index_file_optional_policy() {
        let files = program(&[
            ("src/pages/index.ts", "export {}"),
            ("src/user.ts", "export {}"),
        ]);
        let resolved = resolve_relative_module("src/pages/index.ts", "../user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_directory_index_current_directory() {
        let files = program(&[
            ("src/index.ts", "export {}"),
            ("src/models/index.ts", "export {}"),
            ("src/pages/index.ts", "export {}"),
        ]);
        let resolved = resolve_relative_module("src/pages/index.ts", "..", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/index.ts");
    }

    #[test]
    fn module_resolver_directory_index_grandparent_directory() {
        let files = program(&[
            ("src/index.ts", "export {}"),
            ("src/models/index.ts", "export {}"),
            ("src/pages/nested/index.ts", "export {}"),
        ]);
        let resolved =
            resolve_relative_module("src/pages/nested/index.ts", "../..", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/index.ts");
    }

    #[test]
    fn module_resolver_resolves_script_file_target_for_side_effect() {
        let files = program(&[
            ("src/index.ts", "import \"./setup\";"),
            ("src/setup.ts", "let initialized = true;"),
        ]);
        let resolved = resolve_relative_module("src/index.ts", "./setup", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/setup.ts");
    }

    #[test]
    fn module_resolver_resolves_module_file_target_for_side_effect() {
        let files = program(&[
            ("src/index.ts", "import \"./setup\";"),
            ("src/setup.ts", "export {};"),
        ]);
        let resolved = resolve_relative_module("src/index.ts", "./setup", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/setup.ts");
    }

    #[test]
    fn module_resolver_named_import_from_script_file_is_resolved_but_not_exported() {
        let files = program(&[
            ("src/index.ts", "import { value } from \"./setup\";"),
            ("src/setup.ts", "let value = 1;"),
        ]);
        let resolved = resolve_relative_module("src/index.ts", "./setup", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/setup.ts");
    }

    #[test]
    fn module_resolver_marks_unresolved_star_exports() {
        let files = program(&[("src/index.ts", "export * from \"./missing\";")]);
        let mut file_kinds = surge_ts_types::fx::FxHashMap::default();
        file_kinds.insert("src/index.ts".to_string(), FileKind::RootSource);
        let mut ctx =
            CheckerContext::new("src/index.ts".to_string(), Default::default(), file_kinds);
        let local_tables = files
            .iter()
            .map(|file| {
                let local_types = TypeDeclarationTable::new();
                let local_symbols = SymbolTable::new();
                Some(build_module_export_table(
                    file,
                    &local_types,
                    &local_symbols,
                    &SymbolTable::new(),
                    None,
                    &mut ctx,
                ))
            })
            .collect::<Vec<_>>();
        let resolved = resolve_module_export_tables(&files, &local_tables, &mut ctx);

        assert!(
            resolved[0]
                .as_ref()
                .map(|table| table.has_unresolved_star_export)
                .unwrap_or(false)
        );
    }

    #[test]
    fn resolved_module_index_domain_no_panic() {
        // A resolved module index from the global project domain
        // (`module_file_index_by_identity`) can exceed the length of a narrow
        // local file vector — exactly what the ambient-module binding path passes.
        // The export resolver must degrade to a conservative unresolved result
        // rather than index out of bounds and panic.
        let files = program(&[("src/index.ts", "export { foo } from \"target-pkg\";")]);

        let mut file_kinds = surge_ts_types::fx::FxHashMap::default();
        file_kinds.insert("src/index.ts".to_string(), FileKind::RootSource);
        let mut options = crate::context::CheckerOptions::default();
        options
            .resolved_modules
            .insert("target-pkg".to_string(), "src/target.ts".to_string());
        let mut ctx = CheckerContext::new("src/index.ts".to_string(), options, file_kinds);

        let mut identity = surge_ts_types::fx::FxHashMap::default();
        identity.insert(canonical_file_identity("src/target.ts").into(), 194usize);
        ctx.set_module_file_index_by_identity(identity);

        let local_tables = files
            .iter()
            .map(|file| {
                Some(build_module_export_table(
                    file,
                    &TypeDeclarationTable::new(),
                    &SymbolTable::new(),
                    &SymbolTable::new(),
                    None,
                    &mut ctx,
                ))
            })
            .collect::<Vec<_>>();

        let resolved = resolve_module_export_tables(&files, &local_tables, &mut ctx);
        assert!(resolved[0].is_some());
    }

    #[test]
    fn module_resolver_extensionless_ts() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", "./user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_explicit_ts_exact() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", "./user.ts", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_parent_directory() {
        let files = program(&[
            ("src/pages/index.ts", "export {}"),
            ("src/user.ts", "export {}"),
        ]);
        let resolved = resolve_relative_module("src/pages/index.ts", "../user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_dot_segments() {
        let files = program(&[
            ("src/pages/index.ts", "export {}"),
            ("src/user.ts", "export {}"),
        ]);
        let resolved = resolve_relative_module("src/pages/index.ts", ".././user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_windows_importer_path() {
        let files = program(&[("src\\index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src\\index.ts", "./user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_windows_specifier() {
        let files = program(&[("src/index.ts", "export {}"), ("src/user.ts", "export {}")]);
        let resolved = resolve_relative_module("src/index.ts", ".\\user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user.ts");
    }

    #[test]
    fn module_resolver_index_file() {
        let files = program(&[
            ("src/pages/index.ts", "export {}"),
            ("src/user/index.ts", "export {}"),
        ]);
        let resolved = resolve_relative_module("src/pages/index.ts", "../user", &files).unwrap();
        assert_eq!(resolved.resolved_file_name, "src/user/index.ts");
    }

    #[test]
    fn module_resolver_non_relative_unsupported() {
        let files = program(&[("src/index.ts", "export {}")]);
        assert!(resolve_relative_module("src/index.ts", "pkg", &files).is_none());
    }

    #[test]
    fn module_resolver_missing_relative_file() {
        let files = program(&[("src/index.ts", "export {}")]);
        assert!(resolve_relative_module("src/index.ts", "./missing", &files).is_none());
    }

    #[test]
    fn module_resolver_does_not_read_disk() {
        let files = program(&[("src/index.ts", "export {}")]);
        assert!(resolve_relative_module("src/index.ts", "./missing", &files).is_none());
    }

    #[test]
    fn module_resolver_relative_js_specifier_matches_ts_source() {
        let files = program(&[
            ("src/index.ts", "export {}"),
            ("src/user.tsx", "export {}"),
            ("src/user.js", "export {}"),
            ("src/user.jsx", "export {}"),
            ("src/user.json", "export {}"),
            ("src/user.d.ts", "export {}"),
        ]);

        assert!(resolve_relative_module("src/index.ts", "./user.tsx", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./user.js", &files).is_some());
        // tsc substitutes `.jsx` like `.js` (`tryAddingExtensions` default case),
        // so `./user.jsx` resolves to the loaded `user.tsx` source.
        assert!(resolve_relative_module("src/index.ts", "./user.jsx", &files).is_some());
        // `.json` resolves to exactly the named file — no substitution, no
        // directory index — once `resolveJsonModule` has let the loader take it.
        assert!(resolve_relative_module("src/index.ts", "./user.json", &files).is_some());
        assert!(resolve_relative_module("src/index.ts", "./user.d.ts", &files).is_none());
    }

    #[test]
    fn module_resolver_explicit_js_never_resolves_directory_index() {
        let files = program(&[
            ("src/index.ts", "export {}"),
            ("src/user/index.ts", "export {}"),
        ]);
        // `./user.js` is a file-shaped runtime path: extension substitution may
        // replace the extension, but must never fall back to `user/index.ts`.
        assert!(resolve_relative_module("src/index.ts", "./user.js", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./user.mjs", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./user.cjs", &files).is_none());
        // The extensionless form still resolves through the directory index.
        assert!(resolve_relative_module("src/index.ts", "./user", &files).is_some());
    }

    #[test]
    fn module_resolver_js_flavor_substitution_matrix() {
        let files = program(&[
            ("src/index.ts", "export {}"),
            ("src/m.mts", "export {}"),
            ("src/c.cts", "export {}"),
            ("src/plain.ts", "export {}"),
        ]);
        // `.mjs` → `.mts`, `.cjs` → `.cts`.
        assert!(resolve_relative_module("src/index.ts", "./m.mjs", &files).is_some());
        assert!(resolve_relative_module("src/index.ts", "./c.cjs", &files).is_some());
        // `.js` never crosses into the m/c flavors, and vice versa.
        assert!(resolve_relative_module("src/index.ts", "./m.js", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./c.js", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./plain.mjs", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./plain.cjs", &files).is_none());
        // Extensionless specifiers never probe the m/c flavors.
        assert!(resolve_relative_module("src/index.ts", "./m", &files).is_none());
        assert!(resolve_relative_module("src/index.ts", "./c", &files).is_none());
    }
}
