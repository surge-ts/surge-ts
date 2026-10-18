use std::{fs, path::PathBuf};

use surge_ts_checker::{CheckerOptions, check_source_with_options};
use surge_ts_config::{TsConfigLoadOptions, load_tsconfig};

fn workspace_root() -> PathBuf {
    fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".."),
    )
    .unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
    })
}

#[test]
fn project_mode_maps_strict_to_no_implicit_any() {
    let workspace_root = workspace_root();
    let project = workspace_root.join("tests/tsconfig/basic/tsconfig.json");

    let loaded = load_tsconfig(TsConfigLoadOptions {
        project: project.clone(),
    });
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(
        loaded.files,
        vec![workspace_root.join("tests/tsconfig/basic/src/index.ts")]
    );

    let source = fs::read_to_string(&loaded.files[0]).unwrap();
    let diagnostics = check_source_with_options(
        &source,
        &loaded.files[0].to_string_lossy(),
        CheckerOptions {
            diagnostic_profile: Default::default(),
            resolve_json_module: true,
            allow_js: false,
            check_js: None,
            erasable_syntax_only: false,
            module_detection: Default::default(),
            language_version: Default::default(),
            jsx_configured: false,
            jsx_factory_names: Default::default(),
            resolved_modules: Default::default(),
            resolved_modules_by_importer: Default::default(),
            file_redirects: Default::default(),
            resolved_using_ts_extension: Default::default(),
            no_lib: false,
            skip_lib_check: false,
            jsx_automatic_runtime: false,
            jsx_classic_react: false,
            jsx_emit_none: false,
            allow_umd_global_access: false,
            types: Vec::new(),
            stub_external_modules: false,
            no_implicit_any: loaded.compiler_options.no_implicit_any,
            no_implicit_this: loaded.compiler_options.no_implicit_any,
            module_emit: Default::default(),
            verbatim_module_syntax: false,
            isolated_modules: false,
            use_define_for_class_fields: true,
            target_es2022: true,
            target_es2016: true,
            preserve_const_enums: false,
            no_emit: false,
            node_module_resolution: false,
            resolve_package_json_exports: true,
            esm_module_files: Default::default(),
            esm_package_files: Default::default(),
            strict_property_initialization: loaded
                .compiler_options
                .strict_property_initialization,
            strict_null_checks: loaded.compiler_options.strict_null_checks,
            strict_function_types: loaded.compiler_options.strict_function_types,
            strict_bind_call_apply: false,
            strict_builtin_iterator_return: false,
            use_unknown_in_catch_variables: false,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            rewrite_relative_import_extensions: false,
            declaration: false,
            allow_arbitrary_extensions: false,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
        },
    );

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.to_string() == "TS7006")
    );
}
