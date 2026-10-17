use super::*;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn temp_dir(prefix: &str) -> PathBuf {
    let unique = format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = env::temp_dir().join(unique);
    fs::create_dir_all(&path).unwrap();
    canonicalize_if_exists(&path)
}

fn write_file(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

fn load(project: impl AsRef<Path>) -> LoadedTsConfig {
    load_tsconfig(TsConfigLoadOptions {
        project: project.as_ref().to_path_buf(),
    })
}

fn diagnostic_codes(diagnostics: &[ConfigDiagnostic]) -> Vec<ConfigDiagnosticCode> {
    diagnostics.iter().map(|diag| diag.code).collect()
}

fn has_diagnostic(diagnostics: &[ConfigDiagnostic], code: ConfigDiagnosticCode) -> bool {
    diagnostics.iter().any(|diag| diag.code == code)
}

#[test]
fn parses_jsonc_with_comments_and_trailing_commas() {
    let root = temp_dir("jsonc");
    write_file(
        &root,
        "tsconfig.json",
        r#"
            {
              // comment
              "compilerOptions": {
                "strict": true,
                "noImplicitAny": true,
              },
              "include": ["src/**/*.ts",]
            }
            "#,
    );
    write_file(&root, "src/index.ts", "const ok: string = 'ok';");

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(loaded.compiler_options.strict, true);
    assert_eq!(loaded.compiler_options.no_implicit_any, true);
    assert_eq!(loaded.files, vec![root.join("src/index.ts")]);
}

#[test]
fn strict_true_implies_no_implicit_any() {
    let root = temp_dir("strict");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
    assert!(loaded.compiler_options.no_implicit_any);
}

#[test]
fn strict_null_checks_follows_strict_unless_written() {
    let root = temp_dir("strict-null-checks");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": false } }"#,
    );
    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(!loaded.compiler_options.strict_null_checks);

    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true, "strictNullChecks": false } }"#,
    );
    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.no_implicit_any);
    assert!(!loaded.compiler_options.strict_null_checks);
}

#[test]
fn no_implicit_any_false_overrides_strict_true() {
    let root = temp_dir("strict-override");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true, "noImplicitAny": false } }"#,
    );
    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
    assert!(!loaded.compiler_options.no_implicit_any);
}

#[test]
fn empty_config_uses_ts7_defaults() {
    let root = temp_dir("defaults");
    write_file(&root, "tsconfig.json", r#"{ "compilerOptions": {} }"#);

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
    assert!(loaded.compiler_options.no_implicit_any);
    assert_eq!(loaded.compiler_options.target, ScriptTarget::ES2024);
    assert_eq!(loaded.compiler_options.module, ModuleKind::Preserve);
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
}

#[test]
fn lib_option_is_preserved() {
    let root = temp_dir("lib-option");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "lib": ["ES2022", "DOM"] } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.compiler_options.lib,
        vec!["ES2022".to_string(), "DOM".to_string()]
    );
}

#[test]
fn no_lib_is_preserved() {
    let root = temp_dir("no-lib");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noLib": true } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.no_lib);
}

#[test]
fn relative_extends_merges_compiler_options() {
    let root = temp_dir("extends");
    write_file(
        &root,
        "tsconfig.base.json",
        r#"{ "compilerOptions": { "noImplicitAny": true, "module": "commonjs" } }"#,
    );
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "extends": "./tsconfig.base.json", "compilerOptions": { "strict": true } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
    assert!(loaded.compiler_options.no_implicit_any);
    assert_eq!(loaded.compiler_options.module, ModuleKind::CommonJS);
}

#[test]
fn package_extends_scoped_with_explicit_tsconfig_path_merges_compiler_options() {
    let root = temp_dir("package-extends-scoped-explicit");
    write_file(
        &root,
        "node_modules/@tsconfig/node24/tsconfig.json",
        r#"
            {
              "compilerOptions": {
                "strict": true,
                "target": "es2024",
                "module": "preserve",
                "moduleResolution": "bundler"
              }
            }
            "#,
    );
    write_file(
        &root,
        "tsconfig.json",
        r#"
            {
              "extends": "@tsconfig/node24/tsconfig.json",
              "compilerOptions": {
                "noImplicitAny": false
              }
            }
            "#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
    assert!(!loaded.compiler_options.no_implicit_any);
    assert_eq!(loaded.compiler_options.target, ScriptTarget::ES2024);
    assert_eq!(loaded.compiler_options.module, ModuleKind::Preserve);
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
}

#[test]
fn package_extends_scoped_default_tsconfig_resolves() {
    let root = temp_dir("package-extends-scoped-default");
    write_file(
        &root,
        "node_modules/@tsconfig/node24/tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "extends": "@tsconfig/node24" }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
}

#[test]
fn package_extends_unscoped_explicit_tsconfig_resolves() {
    let root = temp_dir("package-extends-unscoped-explicit");
    write_file(
        &root,
        "node_modules/my-config/tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "extends": "my-config/tsconfig.json" }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
}

#[test]
fn package_extends_searches_parent_node_modules() {
    let root = temp_dir("package-extends-parent-node-modules");
    write_file(
        &root,
        "node_modules/my-config/tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    write_file(
        &root,
        "packages/app/tsconfig.json",
        r#"{ "extends": "my-config" }"#,
    );

    let loaded = load(root.join("packages/app/tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.strict);
}

#[test]
fn package_extends_missing_emits_diagnostic() {
    let root = temp_dir("package-extends-missing");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "extends": "@tsconfig/missing" }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(
        diagnostic_codes(&loaded.diagnostics).contains(&ConfigDiagnosticCode::ExtendsFileNotFound)
    );
}

#[test]
fn package_extends_cycle_emits_diagnostic() {
    let root = temp_dir("package-extends-cycle");
    write_file(&root, "tsconfig.json", r#"{ "extends": "pkg" }"#);
    write_file(
        &root,
        "node_modules/pkg/tsconfig.json",
        r#"{ "extends": "../../tsconfig.json" }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(diagnostic_codes(&loaded.diagnostics).contains(&ConfigDiagnosticCode::ExtendsCycle));
}

#[test]
fn extends_cycle_emits_diagnostic() {
    let root = temp_dir("cycle");
    write_file(
        &root,
        "a.json",
        r#"{ "extends": "./b.json", "compilerOptions": { "strict": true } }"#,
    );
    write_file(
        &root,
        "b.json",
        r#"{ "extends": "./a.json", "compilerOptions": { "noImplicitAny": true } }"#,
    );

    let loaded = load(root.join("a.json"));
    assert!(diagnostic_codes(&loaded.diagnostics).contains(&ConfigDiagnosticCode::ExtendsCycle));
}

#[test]
fn files_selects_exact_files() {
    let root = temp_dir("files");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "files": ["src/a.ts", "src/b.tsx"] }"#,
    );
    write_file(&root, "src/a.ts", "const a = 1;");
    write_file(&root, "src/b.tsx", "const b = 2;");
    write_file(&root, "src/c.js", "const c = 3;");

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(
        loaded.files,
        vec![root.join("src/a.ts"), root.join("src/b.tsx")]
    );
}

#[test]
fn include_selects_matching_ts_files() {
    let root = temp_dir("include");
    write_file(&root, "tsconfig.json", r#"{ "include": ["src/**/*.ts"] }"#);
    write_file(&root, "src/a.ts", "const a = 1;");
    write_file(&root, "src/b.tsx", "const b = 2;");
    write_file(&root, "src/c.js", "const c = 3;");

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.files, vec![root.join("src/a.ts")]);
}

#[test]
fn include_wildcards_skip_dot_segments() {
    let root = temp_dir("include-dot-segments");
    write_file(&root, "tsconfig.json", r#"{ "include": ["src", "gen/*/*.ts"] }"#);
    write_file(&root, "src/index.ts", "const a = 1;");
    write_file(&root, "src/.hidden/skipped.ts", "const b = 2;");
    write_file(&root, "src/.dotfile.ts", "const c = 3;");
    write_file(&root, "gen/plain/kept.ts", "const d = 4;");
    write_file(&root, "gen/.hidden/skipped.ts", "const e = 5;");

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.files,
        vec![root.join("gen/plain/kept.ts"), root.join("src/index.ts")]
    );
}

#[test]
fn include_pattern_spelling_a_dot_segment_still_matches() {
    let root = temp_dir("include-literal-dot-segment");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "include": ["src/.generated/**/*", "src/.dotfile.ts"] }"#,
    );
    write_file(&root, "src/.generated/api.ts", "const a = 1;");
    write_file(&root, "src/.generated/deep/more.ts", "const b = 2;");
    write_file(&root, "src/.dotfile.ts", "const c = 3;");
    write_file(&root, "src/other.ts", "const d = 4;");

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.files,
        vec![
            root.join("src/.dotfile.ts"),
            root.join("src/.generated/api.ts"),
            root.join("src/.generated/deep/more.ts"),
        ]
    );
}

#[test]
fn include_bare_dot_directory_entry_is_not_a_recursive_root() {
    // `isImplicitGlob` only expands an entry whose last component has no `.`, so
    // `src/.generated` is a file spec that matches nothing.
    let root = temp_dir("include-bare-dot-directory");
    write_file(&root, "tsconfig.json", r#"{ "include": ["src/.generated"] }"#);
    write_file(&root, "src/.generated/api.ts", "const a = 1;");

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.files.is_empty());
}

#[test]
fn include_treats_directories_as_recursive_roots_and_keeps_source_extensions() {
    let root = temp_dir("include-directory-roots");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "include": ["src", "examples"] }"#,
    );
    write_file(&root, "src/index.ts", "const fromTs: string = 1;");
    write_file(&root, "src/component.tsx", "const fromTsx: string = 1;");
    write_file(&root, "src/types.d.ts", "declare class Thing {}");
    write_file(
        &root,
        "examples/nested/module.mts",
        "const fromMts: string = 1;",
    );
    write_file(
        &root,
        "examples/nested/common.cts",
        "const fromCts: string = 1;",
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.files,
        vec![
            root.join("examples/nested/common.cts"),
            root.join("examples/nested/module.mts"),
            root.join("src/component.tsx"),
            root.join("src/index.ts"),
            root.join("src/types.d.ts"),
        ]
    );
}

#[test]
fn exclude_removes_matching_files() {
    let root = temp_dir("exclude");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "include": ["src/**/*.ts"], "exclude": ["src/skip/**"] }"#,
    );
    write_file(&root, "src/a.ts", "const a = 1;");
    write_file(&root, "src/skip/b.ts", "const b = 2;");

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.files, vec![root.join("src/a.ts")]);
}

#[test]
fn default_excludes_node_modules() {
    let root = temp_dir("default-exclude");
    write_file(&root, "tsconfig.json", r#"{ "include": ["**/*"] }"#);
    write_file(&root, "src/a.ts", "const a = 1;");
    write_file(&root, "node_modules/pkg/index.ts", "const b = 2;");

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.files, vec![root.join("src/a.ts")]);
}

#[test]
fn invalid_target_es5_emits_legacy_diagnostic_and_falls_back_to_es2015() {
    let root = temp_dir("target");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "target": "es5" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.compiler_options.target, ScriptTarget::ES2015);
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn module_resolution_node_is_legacy_and_falls_back_to_bundler() {
    let root = temp_dir("module-resolution-node");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "node" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn module_resolution_node10_is_legacy_and_falls_back_to_bundler() {
    let root = temp_dir("module-resolution-node10");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "node10" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn module_resolution_classic_is_legacy_and_falls_back_to_bundler() {
    let root = temp_dir("module-resolution-classic");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "classic" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn module_resolution_bundler_is_valid() {
    let root = temp_dir("module-resolution-bundler");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "bundler" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Bundler
    );
}

#[test]
fn module_resolution_nodenext_is_valid() {
    let root = temp_dir("module-resolution-nodenext");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "nodenext" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::NodeNext
    );
}

#[test]
fn module_resolution_follows_the_module_kind_when_unwritten() {
    for (module, expected) in [
        ("node16", ModuleResolutionKind::Node16),
        ("node18", ModuleResolutionKind::Node16),
        ("node20", ModuleResolutionKind::Node16),
        ("nodenext", ModuleResolutionKind::NodeNext),
        ("commonjs", ModuleResolutionKind::Bundler),
    ] {
        let root = temp_dir(&format!("module-resolution-from-{module}"));
        write_file(
            &root,
            "tsconfig.json",
            &format!(r#"{{ "compilerOptions": {{ "module": "{module}" }} }}"#),
        );
        let loaded = load(root.join("tsconfig.json"));
        assert_eq!(loaded.compiler_options.module_resolution, expected, "{module}");
    }
}

#[test]
fn ts6_node20_and_newer_options_are_recognized() {
    let root = temp_dir("ts6-node20-options");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
              "compilerOptions": {
                "module": "node20",
                "moduleResolution": "node20",
                "newLine": "lf",
                "stripInternal": true,
                "erasableSyntaxOnly": true,
                "noImplicitOverride": true,
                "noPropertyAccessFromIndexSignature": true,
                "noUncheckedSideEffectImports": true,
                "noEmitOnError": true,
                "useDefineForClassFields": true
              }
            }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(
        loaded.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        loaded.diagnostics
    );
    assert_eq!(loaded.compiler_options.module, ModuleKind::Node20);
    assert_eq!(
        loaded.compiler_options.module_resolution,
        ModuleResolutionKind::Node20
    );
}

#[test]
fn display_only_options_are_accepted() {
    let root = temp_dir("display-only-options");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
              "compilerOptions": {
                "pretty": false,
                "noErrorTruncation": true
              }
            }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(
        loaded.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        loaded.diagnostics
    );
}

#[test]
fn no_implicit_returns_parses_into_normalized_options() {
    let root = temp_dir("no-implicit-returns");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noImplicitReturns": true } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(
        loaded.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        loaded.diagnostics
    );
    assert!(loaded.compiler_options.no_implicit_returns);
}

#[test]
fn no_implicit_returns_defaults_off() {
    let root = temp_dir("no-implicit-returns-default");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(!loaded.compiler_options.no_implicit_returns);
}

#[test]
fn no_implicit_override_parses_and_defaults_off() {
    let on = temp_dir("no-implicit-override-on");
    write_file(
        &on,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noImplicitOverride": true } }"#,
    );
    let loaded_on = load(on.join("tsconfig.json"));
    assert!(loaded_on.diagnostics.is_empty());
    assert!(loaded_on.compiler_options.no_implicit_override);

    let off = temp_dir("no-implicit-override-off");
    write_file(
        &off,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    let loaded_off = load(off.join("tsconfig.json"));
    assert!(!loaded_off.compiler_options.no_implicit_override);
}

#[test]
fn no_property_access_from_index_signature_parses_and_defaults_off() {
    let on = temp_dir("no-prop-access-index-on");
    write_file(
        &on,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noPropertyAccessFromIndexSignature": true } }"#,
    );
    let loaded_on = load(on.join("tsconfig.json"));
    assert!(loaded_on.diagnostics.is_empty());
    assert!(
        loaded_on
            .compiler_options
            .no_property_access_from_index_signature
    );

    let off = temp_dir("no-prop-access-index-off");
    write_file(
        &off,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    let loaded_off = load(off.join("tsconfig.json"));
    assert!(
        !loaded_off
            .compiler_options
            .no_property_access_from_index_signature
    );
}

#[test]
fn resolve_json_module_defaults_by_resolver_and_takes_an_explicit_override() {
    // tsc 7.0.2 turns `.json` resolution on for every resolver it still accepts
    // except `node16`, whatever order the options appear in.
    let default_resolver = temp_dir("resolve-json-default");
    write_file(
        &default_resolver,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    assert!(
        load(default_resolver.join("tsconfig.json"))
            .compiler_options
            .resolve_json_module
    );

    let node16 = temp_dir("resolve-json-node16");
    write_file(
        &node16,
        "tsconfig.json",
        r#"{ "compilerOptions": { "moduleResolution": "node16", "module": "node16" } }"#,
    );
    assert!(
        !load(node16.join("tsconfig.json"))
            .compiler_options
            .resolve_json_module
    );

    let node16_opted_in = temp_dir("resolve-json-node16-on");
    write_file(
        &node16_opted_in,
        "tsconfig.json",
        r#"{ "compilerOptions": { "resolveJsonModule": true, "moduleResolution": "node16", "module": "node16" } }"#,
    );
    let loaded = load(node16_opted_in.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.compiler_options.resolve_json_module);

    let bundler_opted_out = temp_dir("resolve-json-bundler-off");
    write_file(
        &bundler_opted_out,
        "tsconfig.json",
        r#"{ "compilerOptions": { "resolveJsonModule": false, "moduleResolution": "bundler" } }"#,
    );
    assert!(
        !load(bundler_opted_out.join("tsconfig.json"))
            .compiler_options
            .resolve_json_module
    );
}

#[test]
fn no_unused_locals_and_parameters_parse_and_default_off() {
    let on = temp_dir("no-unused-on");
    write_file(
        &on,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noUnusedLocals": true, "noUnusedParameters": true } }"#,
    );
    let loaded_on = load(on.join("tsconfig.json"));
    assert!(loaded_on.diagnostics.is_empty());
    assert!(loaded_on.compiler_options.no_unused_locals);
    assert!(loaded_on.compiler_options.no_unused_parameters);

    let off = temp_dir("no-unused-off");
    write_file(
        &off,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    let loaded_off = load(off.join("tsconfig.json"));
    assert!(!loaded_off.compiler_options.no_unused_locals);
    assert!(!loaded_off.compiler_options.no_unused_parameters);
}

#[test]
fn no_fallthrough_cases_in_switch_parses_and_defaults_off() {
    let on = temp_dir("no-fallthrough-on");
    write_file(
        &on,
        "tsconfig.json",
        r#"{ "compilerOptions": { "noFallthroughCasesInSwitch": true } }"#,
    );
    let loaded_on = load(on.join("tsconfig.json"));
    assert!(loaded_on.diagnostics.is_empty());
    assert!(loaded_on.compiler_options.no_fallthrough_cases_in_switch);

    let off = temp_dir("no-fallthrough-off");
    write_file(
        &off,
        "tsconfig.json",
        r#"{ "compilerOptions": { "strict": true } }"#,
    );
    let loaded_off = load(off.join("tsconfig.json"));
    assert!(!loaded_off.compiler_options.no_fallthrough_cases_in_switch);
}

#[test]
fn module_none_is_legacy_and_falls_back_to_preserve() {
    let root = temp_dir("module-none");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "module": "none" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.compiler_options.module, ModuleKind::Preserve);
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn module_legacy_values_are_rejected() {
    for module in ["amd", "umd", "system", "systemjs"] {
        let root = temp_dir(&format!("module-legacy-{module}"));
        write_file(
            &root,
            "tsconfig.json",
            &format!(
                r#"{{
                      "compilerOptions": {{
                        "module": "{module}"
                      }}
                    }}"#
            ),
        );

        let loaded = load(root.join("tsconfig.json"));
        assert_eq!(loaded.compiler_options.module, ModuleKind::Preserve);
        assert!(has_diagnostic(
            &loaded.diagnostics,
            ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
        ));
    }
}

#[test]
fn target_es5_is_legacy_and_falls_back_to_es2015() {
    let root = temp_dir("target-es5");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "target": "es5" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.compiler_options.target, ScriptTarget::ES2015);
    assert!(has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue
    ));
}

#[test]
fn es6_is_an_alias_of_es2015_for_target_and_module() {
    let root = temp_dir("target-module-es6");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "target": "es6", "module": "es6" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.compiler_options.target, ScriptTarget::ES2015);
    assert_eq!(loaded.compiler_options.module, ModuleKind::ES2015);
    assert!(loaded.diagnostics.is_empty());
}

#[test]
fn base_url_is_supported_and_resolved_against_config_dir() {
    let root = temp_dir("base-url");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": "src" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(!has_diagnostic(
        &loaded.diagnostics,
        ConfigDiagnosticCode::UnsupportedLegacyCompilerOption
    ));
    let base_url = loaded
        .compiler_options
        .base_url
        .as_ref()
        .expect("baseUrl should be resolved");
    assert!(base_url.ends_with("src"));
    assert!(base_url.is_absolute());
}

#[test]
fn paths_are_accepted_without_base_url() {
    let root = temp_dir("paths");
    write_file(
        &root,
        "tsconfig.json",
        r#"
            {
              "compilerOptions": {
                "paths": {
                  "@app/*": ["src/*"]
                }
              }
            }
            "#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.compiler_options.paths,
        vec![PathMapping {
            pattern: "@app/*".to_string(),
            substitutions: vec!["src/*".to_string()],
        }]
    );
}

#[test]
fn config_includes_d_ts_from_include_glob() {
    let root = temp_dir("config_includes_d_ts_from_include_glob");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "include": ["src/**/*", "types/**/*.d.ts"]
    }"#,
    );
    write_file(&root, "src/index.ts", "");
    write_file(&root, "types/globals.d.ts", "");

    let loaded = load(root.join("tsconfig.json"));
    let mut names: Vec<_> = loaded
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["globals.d.ts", "index.ts"]);
}

#[test]
fn config_includes_d_ts_from_files_list() {
    let root = temp_dir("config_includes_d_ts_from_files_list");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "files": ["index.ts", "globals.d.ts"]
    }"#,
    );
    write_file(&root, "index.ts", "");
    write_file(&root, "globals.d.ts", "");

    let loaded = load(root.join("tsconfig.json"));
    let mut names: Vec<_> = loaded
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["globals.d.ts", "index.ts"]);
}

#[test]
fn config_excludes_node_modules_d_ts_by_default() {
    let root = temp_dir("config_excludes_node_modules_d_ts_by_default");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "include": ["**/*"]
    }"#,
    );
    write_file(&root, "index.ts", "");
    write_file(&root, "node_modules/pkg/index.d.ts", "");

    let loaded = load(root.join("tsconfig.json"));
    let names: Vec<_> = loaded
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, vec!["index.ts"]);
}

#[test]
fn config_includes_mixed_ts_and_d_ts() {
    let root = temp_dir("config_includes_mixed_ts_and_d_ts");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "include": ["**/*"]
    }"#,
    );
    write_file(&root, "index.ts", "");
    write_file(&root, "globals.d.ts", "");

    let config = load_tsconfig(TsConfigLoadOptions {
        project: root.join("tsconfig.json"),
    });

    assert_eq!(config.diagnostics.len(), 0);
    let mut names: Vec<String> = config
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["globals.d.ts", "index.ts"]);
}

#[test]
fn config_does_not_auto_load_typescript_lib() {
    let root = temp_dir("config_does_not_auto_load_typescript_lib");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "compilerOptions": { "lib": ["es2015"] },
        "include": ["**/*"]
    }"#,
    );
    write_file(&root, "index.ts", "");

    let config = load_tsconfig(TsConfigLoadOptions {
        project: root.join("tsconfig.json"),
    });

    assert_eq!(config.diagnostics.len(), 0);
    let mut names: Vec<String> = config
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["index.ts"]);
}

#[test]
fn config_d_ts_does_not_require_allow_js() {
    let root = temp_dir("config_d_ts_does_not_require_allow_js");
    write_file(
        &root,
        "tsconfig.json",
        r#"{
        "compilerOptions": { "allowJs": false },
        "include": ["**/*"]
    }"#,
    );
    write_file(&root, "index.ts", "");
    write_file(&root, "globals.d.ts", "");
    write_file(&root, "util.js", "");

    let config = load_tsconfig(TsConfigLoadOptions {
        project: root.join("tsconfig.json"),
    });

    assert_eq!(config.diagnostics.len(), 0);
    let mut names: Vec<String> = config
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["globals.d.ts", "index.ts"]);
}

/// The old canonicalization algorithm, kept as the reference the leaf-probe
/// shortcut must match byte-for-byte.
fn reference_canonicalize(path: &Path) -> PathBuf {
    match fs::canonicalize(path) {
        Ok(canonical) => normalize_path_buf(&canonical),
        Err(_) => normalize_path_buf(path),
    }
}

#[test]
fn canonicalize_matches_full_realpath_on_every_spelling() {
    let root = temp_dir("canon-equivalence");
    write_file(&root, "Dir/File.ts", "");
    write_file(&root, "Dir/nested/deep.ts", "");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("Dir"), root.join("dirlink")).unwrap();
        std::os::unix::fs::symlink(root.join("Dir/File.ts"), root.join("filelink.ts")).unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("dangling")).unwrap();
    }

    let spellings = [
        "Dir/File.ts",
        "dir/file.ts",
        "DIR/FILE.TS",
        "Dir/./File.ts",
        "Dir/nested/../File.ts",
        "Dir/nested/deep.ts",
        "Dir/missing.ts",
        "missing-dir/missing.ts",
        "Dir/File.ts/",
        "Dir/File.ts/extra.ts",
        "Dir",
        "Dir/",
        "dirlink/File.ts",
        "dirlink/file.ts",
        "filelink.ts",
        "dangling",
        "dangling/x.ts",
    ];
    for spelling in spellings {
        let path = root.join(spelling);
        clear_canonicalize_cache();
        let expected = reference_canonicalize(&path);
        let actual = canonicalize_if_exists(&path);
        assert_eq!(actual, expected, "spelling {spelling:?} diverged");
        assert_eq!(
            canonicalize_if_exists_string(&path),
            expected.to_string_lossy().replace('\\', "/"),
            "string form for {spelling:?} diverged"
        );
        // Same answers when ancestors were already cached by a prior call.
        let warm = canonicalize_if_exists(&path);
        assert_eq!(warm, expected, "warm spelling {spelling:?} diverged");
    }

    fs::remove_dir_all(&root).ok();
}

#[test]
fn removed_options_are_reported_with_source_ranges() {
    let root = temp_dir("removed-options");
    let source = "{\n  \"compilerOptions\": {\n    \"esModuleInterop\": false,\n    \"baseUrl\": \".\"\n  }\n}\n";
    write_file(&root, "tsconfig.json", source);

    let loaded = load(root.join("tsconfig.json"));
    let removed: Vec<(&str, Option<&str>)> = loaded
        .removed_options
        .iter()
        .map(|option| (option.name.as_str(), option.value.as_deref()))
        .collect();
    assert_eq!(
        removed,
        vec![("esModuleInterop", Some("false")), ("baseUrl", None)]
    );

    // `name=value` spans the value, a whole-option removal spans the key.
    let interop = &loaded.removed_options[0];
    assert_eq!(&source[interop.start..interop.end], "false");
    let base_url = &loaded.removed_options[1];
    assert_eq!(&source[base_url.start..base_url.end], "\"baseUrl\"");
}

#[test]
fn removed_options_from_extends_anchor_on_the_compiler_options_key() {
    let root = temp_dir("removed-options-extends");
    write_file(
        &root,
        "base.json",
        r#"{ "compilerOptions": { "esModuleInterop": false } }"#,
    );
    let source = "{\n  \"extends\": \"./base.json\",\n  \"compilerOptions\": { \"strict\": true }\n}\n";
    write_file(&root, "tsconfig.json", source);

    let loaded = load(root.join("tsconfig.json"));
    assert_eq!(loaded.removed_options.len(), 1);
    let inherited = &loaded.removed_options[0];
    assert_eq!(inherited.name, "esModuleInterop");
    assert_eq!(
        &source[inherited.start..inherited.end],
        "\"compilerOptions\""
    );
}

#[test]
fn removed_option_values_use_typescript_spelling() {
    let root = temp_dir("removed-option-values");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "target": "es5", "module": "amd", "moduleResolution": "node" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    let removed: Vec<(&str, Option<&str>)> = loaded
        .removed_options
        .iter()
        .map(|option| (option.name.as_str(), option.value.as_deref()))
        .collect();
    assert_eq!(
        removed,
        vec![
            ("target", Some("ES5")),
            ("module", Some("AMD")),
            ("moduleResolution", Some("node10")),
        ]
    );
}

#[test]
fn enum_option_values_no_key_spells_are_invalid() {
    let root = temp_dir("invalid-enum-options");
    let source = r#"{ "compilerOptions": { "module": "none", "target": "ES2020", "jsx": "", "newLine": "cr" } }"#;
    write_file(&root, "tsconfig.json", source);

    let invalid = invalid_enum_options(&root.join("tsconfig.json"));
    let reported: Vec<(&str, &str, &str)> = invalid
        .iter()
        .map(|option| (option.name.as_str(), option.keys.as_str(), &source[option.start..option.end]))
        .collect();
    assert_eq!(
        reported,
        vec![
            (
                "module",
                "'commonjs', 'es6', 'es2015', 'es2020', 'es2022', 'esnext', 'node16', 'node18', 'node20', 'nodenext', 'preserve'",
                "\"none\"",
            ),
            ("newLine", "'crlf', 'lf'", "\"cr\""),
        ]
    );
}

#[test]
fn supported_option_values_are_not_reported_as_removed() {
    let root = temp_dir("removed-options-clean");
    write_file(
        &root,
        "tsconfig.json",
        r#"{ "compilerOptions": { "esModuleInterop": true, "target": "es2022", "module": "esnext", "moduleResolution": "bundler" } }"#,
    );

    let loaded = load(root.join("tsconfig.json"));
    assert!(loaded.removed_options.is_empty());
}
