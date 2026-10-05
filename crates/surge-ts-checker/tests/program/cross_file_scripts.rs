use surge_ts_checker::{
    CheckerOptions, DiagnosticProfile, SourceFileInput, check_program,
    check_source, check_source_with_options,
};

use super::*;

#[test]
fn program_empty_files_valid() {
    assert!(check_program(Vec::new()).is_empty());
}

#[test]
fn program_single_file_matches_check_source_for_basic_valid() {
    let source = "let value: string = \"Ada\";";
    let program_diagnostics = program(&[("example.ts", source)]);
    let single_file_diagnostics = check_source(source, "example.ts");

    assert_eq!(codes(&program_diagnostics), codes(&single_file_diagnostics));
}

#[test]
fn program_api_empty_files_valid() {
    assert!(program(&[]).is_empty());
}

#[test]
fn program_api_single_file_matches_check_source_valid() {
    let source = "let value: string = \"Ada\";";
    let program_diagnostics = program(&[("example.ts", source)]);
    let single_file_diagnostics = check_source(source, "example.ts");

    assert_eq!(codes(&program_diagnostics), codes(&single_file_diagnostics));
    assert_eq!(
        file_names(&program_diagnostics),
        file_names(&single_file_diagnostics)
    );
}

#[test]
fn program_api_single_file_matches_check_source_mismatch() {
    let source = "let value: string = 123;";
    let program_diagnostics = program(&[("example.ts", source)]);
    let single_file_diagnostics = check_source(source, "example.ts");

    assert_eq!(codes(&program_diagnostics), codes(&single_file_diagnostics));
    assert_eq!(
        file_names(&program_diagnostics),
        file_names(&single_file_diagnostics)
    );
}

#[test]
fn program_api_single_file_no_implicit_any_matches_check_source_with_options() {
    let source = "function f(value): string { return \"ok\"; }";
    let program_diagnostics = program_with_options(
        &[("example.ts", source)],
        CheckerOptions {
            use_unknown_in_catch_variables: false,
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
            stub_external_modules: false,
            no_implicit_any: true,
            no_implicit_this: true,
            module_emit: Default::default(),
            verbatim_module_syntax: false,
            isolated_modules: false,
            use_define_for_class_fields: true,
            target_es2022: true,
            target_es2016: true,
            no_emit: false,
            node_module_resolution: false,
            esm_module_files: Default::default(),
            strict_null_checks: true,
            strict_bind_call_apply: false,
            strict_builtin_iterator_return: false,
            strict_property_initialization: false,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            allow_arbitrary_extensions: false,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
            no_lib: false,
            skip_lib_check: false,
            jsx_automatic_runtime: false,
            jsx_classic_react: false,
            jsx_emit_none: false,
            allow_umd_global_access: false,
            types: Vec::new(),
        },
    );
    let single_file_diagnostics = check_source_with_options(
        source,
        "example.ts",
        CheckerOptions {
            use_unknown_in_catch_variables: false,
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
            stub_external_modules: false,
            no_implicit_any: true,
            no_implicit_this: true,
            module_emit: Default::default(),
            verbatim_module_syntax: false,
            isolated_modules: false,
            use_define_for_class_fields: true,
            target_es2022: true,
            target_es2016: true,
            no_emit: false,
            node_module_resolution: false,
            esm_module_files: Default::default(),
            strict_null_checks: true,
            strict_bind_call_apply: false,
            strict_builtin_iterator_return: false,
            strict_property_initialization: false,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            allow_arbitrary_extensions: false,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
            no_lib: false,
            skip_lib_check: false,
            jsx_automatic_runtime: false,
            jsx_classic_react: false,
            jsx_emit_none: false,
            allow_umd_global_access: false,
            types: Vec::new(),
        },
    );

    assert_eq!(codes(&program_diagnostics), codes(&single_file_diagnostics));
    assert_eq!(
        file_names(&program_diagnostics),
        file_names(&single_file_diagnostics)
    );
}

#[test]
fn program_api_accepts_owned_source_file_inputs() {
    let diagnostics = check_program(vec![SourceFileInput {
        file_name: "a.ts".into(),
        source_text: "let value: string = 123;".into(),
    }]);

    assert_eq!(codes(&diagnostics), vec!["TS2322"]);
    assert_eq!(file_names(&diagnostics), vec!["a.ts"]);
}

#[test]
fn program_order_parser_before_type_prepass() {
    let diagnostics = program_with_options(
        &[
            ("a.ts", "let value: string | = \"bad\";"),
            ("b.ts", "type Name = string; type Name = number;"),
            ("c.ts", "function f(value): string { return 123; }"),
        ],
        CheckerOptions {
            use_unknown_in_catch_variables: false,
            diagnostic_profile: DiagnosticProfile::Native,
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
            stub_external_modules: false,
            no_implicit_any: true,
            no_implicit_this: true,
            module_emit: Default::default(),
            verbatim_module_syntax: false,
            isolated_modules: false,
            use_define_for_class_fields: true,
            target_es2022: true,
            target_es2016: true,
            no_emit: false,
            node_module_resolution: false,
            esm_module_files: Default::default(),
            strict_null_checks: true,
            strict_bind_call_apply: false,
            strict_builtin_iterator_return: false,
            strict_property_initialization: false,
            no_implicit_returns: false,
            no_fallthrough_cases_in_switch: false,
            no_implicit_override: false,
            no_property_access_from_index_signature: false,
            no_unchecked_indexed_access: false,
            no_unchecked_side_effect_imports: true,
            exact_optional_property_types: false,
            allow_importing_ts_extensions: false,
            allow_arbitrary_extensions: false,
            experimental_decorators: false,
            import_helpers: false,
            emit_decorator_metadata: false,
            no_unused_locals: false,
            no_unused_parameters: false,
            allow_unreachable_code: false,
            report_unreachable_code: false,
            allow_unused_labels: None,
            no_lib: false,
            skip_lib_check: false,
            jsx_automatic_runtime: false,
            jsx_classic_react: false,
            jsx_emit_none: false,
            allow_umd_global_access: false,
            types: Vec::new(),
        },
    );

    assert_eq!(
        codes(&diagnostics),
        vec!["surge::parser-error", "TS2300", "TS7006", "TS2322"]
    );
    assert_eq!(
        file_names(&diagnostics),
        vec!["a.ts", "b.ts", "c.ts", "c.ts"]
    );
}
