use std::path::PathBuf;

use crate::diagnostics::ConfigDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsConfigLoadOptions {
    pub project: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTsConfig {
    pub config_path: PathBuf,
    pub root_dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub compiler_options: NormalizedCompilerOptions,
    pub diagnostics: Vec<ConfigDiagnostic>,
    /// Options TypeScript 7 removed, resolved against the root config's source
    /// ranges so the caller can report `TS5102`/`TS5108` where tsc does.
    pub removed_options: Vec<crate::RemovedCompilerOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedCompilerOptions {
    pub strict: bool,
    pub no_implicit_any: bool,
    /// `compilerOptions.noImplicitThis`. Defaults to `strict`.
    pub no_implicit_this: bool,
    /// `compilerOptions.strictNullChecks`. Defaults to `strict`.
    pub strict_null_checks: bool,
    /// `compilerOptions.strictBindCallApply`. Defaults to `strict`.
    pub strict_bind_call_apply: bool,
    /// `compilerOptions.strictBuiltinIteratorReturn`. Defaults to `strict`.
    pub strict_builtin_iterator_return: bool,
    /// `compilerOptions.strictPropertyInitialization`. Defaults to `strict`.
    pub strict_property_initialization: bool,
    pub use_unknown_in_catch_variables: bool,
    /// `compilerOptions.noImplicitReturns`. Independent of `strict`; defaults off.
    pub no_implicit_returns: bool,
    /// `compilerOptions.noFallthroughCasesInSwitch`. Independent of `strict`; defaults off.
    pub no_fallthrough_cases_in_switch: bool,
    /// `compilerOptions.noImplicitOverride`. Independent of `strict`; defaults off.
    pub no_implicit_override: bool,
    /// `compilerOptions.declaration`.
    pub declaration: bool,
    /// `compilerOptions.composite`, which implies `declaration`.
    pub composite: bool,
    /// `compilerOptions.isolatedDeclarations`.
    pub isolated_declarations: bool,
    /// `compilerOptions.noEmitOnError`, which skips emit — and what declaration
    /// emit reports — when the program reports anything else.
    pub no_emit_on_error: bool,
    /// `compilerOptions.isolatedModules`.
    pub isolated_modules: bool,
    /// `compilerOptions.preserveConstEnums`.
    pub preserve_const_enums: bool,
    /// `compilerOptions.verbatimModuleSyntax`.
    pub verbatim_module_syntax: bool,
    /// `compilerOptions.noPropertyAccessFromIndexSignature`. Independent of `strict`; defaults off.
    pub no_property_access_from_index_signature: bool,
    /// `compilerOptions.noUncheckedIndexedAccess`. Independent of `strict`; defaults off.
    pub no_unchecked_indexed_access: bool,
    /// `compilerOptions.noUncheckedSideEffectImports`. tsgo reads it as a
    /// tristate whose unset value is on (`IsTrueOrUnknown`).
    pub no_unchecked_side_effect_imports: bool,
    /// `compilerOptions.exactOptionalPropertyTypes`. Independent of `strict`; defaults off.
    pub exact_optional_property_types: bool,
    /// `compilerOptions.allowImportingTsExtensions`. Defaults off; without it an
    /// import path ending in a TypeScript extension is TS5097.
    pub allow_importing_ts_extensions: bool,
    /// `compilerOptions.rewriteRelativeImportExtensions`. Defaults off; it
    /// permits TypeScript-extension imports as `allowImportingTsExtensions`
    /// does (tsc's `GetAllowImportingTsExtensions`).
    pub rewrite_relative_import_extensions: bool,
    /// `compilerOptions.allowArbitraryExtensions`. Defaults off; without it an
    /// import of `./x.html` resolving to `./x.d.html.ts` is TS6263.
    pub allow_arbitrary_extensions: bool,
    /// `compilerOptions.deduplicatePackages`. Defaults on: a second copy of a
    /// package version is a redirect to the first.
    pub deduplicate_packages: bool,
    /// `compilerOptions.experimentalDecorators`. Defaults off, which checks
    /// decorators as ES decorators.
    pub experimental_decorators: bool,
    /// `compilerOptions.importHelpers`: emit helpers come from `tslib`,
    /// which the checker resolves and reads.
    pub import_helpers: bool,
    /// `compilerOptions.emitDecoratorMetadata`.
    pub emit_decorator_metadata: bool,
    /// `compilerOptions.noUnusedLocals`. Independent of `strict`; defaults off.
    pub no_unused_locals: bool,
    /// `compilerOptions.noUnusedParameters`. Independent of `strict`; defaults off.
    pub no_unused_parameters: bool,
    /// `compilerOptions.allowUnreachableCode`, set only when written `true`.
    pub allow_unreachable_code: bool,
    /// `compilerOptions.allowUnreachableCode` written `false`, which is the
    /// only setting under which tsc reports unreachable code as an error.
    pub report_unreachable_code: bool,
    /// `compilerOptions.allowUnusedLabels` as written; `None` when unset.
    pub allow_unused_labels: Option<bool>,
    pub target: ScriptTarget,
    pub module: ModuleKind,
    /// tsgo's `GetEmitModuleKind`: the written `module`, or the kind its
    /// `target` implies when none is written (`module` itself defaults to
    /// `preserve` here for resolution purposes, which tsc does not).
    pub emit_module: ModuleKind,
    /// tsgo's `GetUseDefineForClassFields`: the written flag, or whether the
    /// target is ES2022 or later.
    pub use_define_for_class_fields: bool,
    pub module_resolution: ModuleResolutionKind,
    pub jsx: Option<JsxMode>,
    /// `jsxFactory`, `jsxFragmentFactory`, `reactNamespace` and
    /// `jsxImportSource`: the names a JSX tag refers to implicitly.
    pub jsx_factory: Option<String>,
    pub jsx_fragment_factory: Option<String>,
    pub react_namespace: Option<String>,
    pub jsx_import_source: Option<String>,
    pub allow_js: bool,
    /// `compilerOptions.checkJs`; unset is not `false`: tsc reports a plain
    /// JavaScript file's binder and grammar errors only when it is unset.
    pub check_js: Option<bool>,
    pub erasable_syntax_only: bool,
    pub module_detection: ModuleDetectionKind,
    pub no_emit: bool,
    /// `compilerOptions.noCheck`: no file is type-checked, so only syntactic
    /// diagnostics are reported.
    pub no_check: bool,
    /// `compilerOptions.noResolve`: a file's `/// <reference>` directives
    /// neither add files nor report what they fail to name.
    pub no_resolve: bool,
    pub skip_lib_check: bool,
    pub es_module_interop: bool,
    pub allow_synthetic_default_imports: bool,
    /// `compilerOptions.allowUmdGlobalAccess`. When true, referencing a UMD
    /// global from a module is permitted and TS2686 is suppressed.
    pub allow_umd_global_access: bool,
    pub no_lib: bool,
    pub lib: Vec<String>,
    pub paths: Vec<PathMapping>,
    /// `compilerOptions.baseUrl`, resolved to an absolute path against the
    /// config directory. `paths` substitutions and non-relative bare-import
    /// fallback resolution are anchored here, matching `tsc`. `None` when unset.
    pub base_url: Option<PathBuf>,
    pub type_roots: Vec<PathBuf>,
    /// `compilerOptions.types`. Under the pinned TypeScript 6 behavior, absent
    /// and empty both include no type packages; `["*"]` opts into visible
    /// `@types` discovery.
    pub types: Option<Vec<String>>,
    /// `compilerOptions.resolvePackageJsonExports`. When false, the package
    /// `exports` field is bypassed during declaration resolution. Defaults to
    /// `true` for modern resolvers (node16/nodenext/bundler).
    pub resolve_package_json_exports: bool,
    /// `compilerOptions.resolvePackageJsonImports`. When false, the package
    /// `imports` (`#alias`) field is bypassed. Defaults to `true` for modern
    /// resolvers.
    pub resolve_package_json_imports: bool,
    /// `compilerOptions.customConditions`. Extra export/import conditions that
    /// participate in condition matching, in configured priority order.
    pub custom_conditions: Vec<String>,
    /// `compilerOptions.resolveJsonModule`. When true a `.json` specifier
    /// resolves to the JSON value's type; when false the import reports
    /// `TS2732`. Defaults on for every resolver except `node16`, which is what
    /// tsc 7.0.2 does.
    pub resolve_json_module: bool,
    /// `compilerOptions.libReplacement`: a default lib an installed
    /// `@typescript/lib-*` package provides is read from that package.
    pub lib_replacement: bool,
}

impl Default for NormalizedCompilerOptions {
    fn default() -> Self {
        Self {
            strict: true,
            no_implicit_any: true,
            no_implicit_this: true,
            strict_null_checks: true,
            strict_bind_call_apply: true,
            strict_builtin_iterator_return: true,
            strict_property_initialization: true,
            use_unknown_in_catch_variables: true,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            declaration: false,
            composite: false,
            no_emit_on_error: false,
            isolated_declarations: false,
            isolated_modules: false,
            preserve_const_enums: false,
            verbatim_module_syntax: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            rewrite_relative_import_extensions: false,
            allow_arbitrary_extensions: false,
            deduplicate_packages: true,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
            target: ScriptTarget::ES2024,
            module: ModuleKind::Preserve,
            emit_module: ModuleKind::ES2022,
            use_define_for_class_fields: true,
            module_resolution: ModuleResolutionKind::Bundler,
            jsx: None,
            jsx_factory: None,
            jsx_fragment_factory: None,
            react_namespace: None,
            jsx_import_source: None,
            allow_js: false,
            check_js: None,
            erasable_syntax_only: false,
            module_detection: ModuleDetectionKind::Auto,
            no_emit: false,
            no_check: false,
            no_resolve: false,
            skip_lib_check: false,
            es_module_interop: false,
            allow_synthetic_default_imports: false,
            allow_umd_global_access: false,
            no_lib: false,
            lib: Vec::new(),
            paths: Vec::new(),
            base_url: None,
            type_roots: Vec::new(),
            types: None,
            resolve_json_module: true,
            lib_replacement: false,
            resolve_package_json_exports: true,
            resolve_package_json_imports: true,
            custom_conditions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScriptTarget {
    ES2015,
    ES2016,
    ES2017,
    ES2018,
    ES2019,
    ES2020,
    ES2021,
    ES2022,
    ES2023,
    ES2024,
    ES2025,
    ESNext,
}

impl Default for ScriptTarget {
    fn default() -> Self {
        Self::ES2015
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    CommonJS,
    ES2015,
    ES2020,
    ES2022,
    ESNext,
    Node16,
    Node18,
    Node20,
    NodeNext,
    Preserve,
}

impl Default for ModuleKind {
    fn default() -> Self {
        Self::Preserve
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleResolutionKind {
    Node16,
    Node20,
    NodeNext,
    Bundler,
}

impl Default for ModuleResolutionKind {
    fn default() -> Self {
        Self::Bundler
    }
}

/// `compilerOptions.moduleDetection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModuleDetectionKind {
    #[default]
    Auto,
    Legacy,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsxMode {
    Preserve,
    React,
    ReactJsx,
    ReactJsxDev,
    ReactNative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMapping {
    pub pattern: String,
    pub substitutions: Vec<String>,
}
