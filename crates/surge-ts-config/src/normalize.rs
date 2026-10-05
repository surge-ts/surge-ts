use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::diagnostics::{ConfigDiagnostic, ConfigDiagnosticCode};
use crate::model::{
    JsxMode, ModuleDetectionKind, ModuleKind, ModuleResolutionKind, NormalizedCompilerOptions, PathMapping, ScriptTarget,
};
use crate::options::{
    TsConfigOptionDefinition, TsConfigOptionSupport, TsConfigOptionValueKind, find_tsconfig_option,
};
use crate::paths::resolve_path;

pub(crate) fn normalize_compiler_options(
    compiler_options: Option<&Map<String, Value>>,
    config_dir: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> NormalizedCompilerOptions {
    let mut normalized = NormalizedCompilerOptions::default();
    let Some(compiler_options) = compiler_options else {
        return normalized;
    };

    let mut explicit_no_implicit_any = None;
    let mut explicit_no_implicit_this = None;
    let mut explicit_module = None;
    let mut explicit_use_define_for_class_fields = None;
    let mut explicit_strict_null_checks = None;
    let mut explicit_strict_bind_call_apply = None;
    let mut explicit_strict_builtin_iterator_return = None;
    let mut explicit_strict_property_initialization = None;
    let mut explicit_use_unknown_in_catch_variables = None;
    let mut explicit_resolve_json_module = None;
    let mut explicit_module_resolution = None;

    for (key, value) in compiler_options {
        match key.as_str() {
            "strict" => {
                if let Some(strict) = parse_bool_option(key, value, config_dir, diagnostics) {
                    normalized.strict = strict;
                }
            }
            "noImplicitAny" => {
                explicit_no_implicit_any = parse_bool_option(key, value, config_dir, diagnostics);
                if let Some(no_implicit_any) = explicit_no_implicit_any {
                    normalized.no_implicit_any = no_implicit_any;
                }
            }
            "noImplicitThis" => {
                explicit_no_implicit_this = parse_bool_option(key, value, config_dir, diagnostics);
            }
            "strictNullChecks" => {
                explicit_strict_null_checks = parse_bool_option(key, value, config_dir, diagnostics);
            }
            "strictBindCallApply" => {
                explicit_strict_bind_call_apply =
                    parse_bool_option(key, value, config_dir, diagnostics);
            }
            "strictBuiltinIteratorReturn" => {
                explicit_strict_builtin_iterator_return =
                    parse_bool_option(key, value, config_dir, diagnostics);
            }
            "strictPropertyInitialization" => {
                explicit_strict_property_initialization =
                    parse_bool_option(key, value, config_dir, diagnostics);
                if let Some(value) = explicit_strict_property_initialization {
                    normalized.strict_property_initialization = value;
                }
            }
            "useUnknownInCatchVariables" => {
                explicit_use_unknown_in_catch_variables =
                    parse_bool_option(key, value, config_dir, diagnostics);
                if let Some(value) = explicit_use_unknown_in_catch_variables {
                    normalized.use_unknown_in_catch_variables = value;
                }
            }
            "noImplicitReturns" => {
                normalized.no_implicit_returns =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_implicit_returns);
            }
            "noFallthroughCasesInSwitch" => {
                normalized.no_fallthrough_cases_in_switch =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_fallthrough_cases_in_switch);
            }
            "noImplicitOverride" => {
                normalized.no_implicit_override =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_implicit_override);
            }
            "declaration" => {
                normalized.declaration = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.declaration);
            }
            "composite" => {
                normalized.composite = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.composite);
            }
            "isolatedDeclarations" => {
                normalized.isolated_declarations = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.isolated_declarations);
            }
            "preserveConstEnums" => {
                normalized.preserve_const_enums = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.preserve_const_enums);
            }
            "isolatedModules" => {
                normalized.isolated_modules = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.isolated_modules);
            }
            "verbatimModuleSyntax" => {
                normalized.verbatim_module_syntax = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.verbatim_module_syntax);
            }
            "noPropertyAccessFromIndexSignature" => {
                normalized.no_property_access_from_index_signature =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_property_access_from_index_signature);
            }
            "noUncheckedSideEffectImports" => {
                normalized.no_unchecked_side_effect_imports =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_unchecked_side_effect_imports);
            }
            "noUncheckedIndexedAccess" => {
                normalized.no_unchecked_indexed_access =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_unchecked_indexed_access);
            }
            "exactOptionalPropertyTypes" => {
                normalized.exact_optional_property_types =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.exact_optional_property_types);
            }
            "allowImportingTsExtensions" => {
                normalized.allow_importing_ts_extensions =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.allow_importing_ts_extensions);
            }
            "rewriteRelativeImportExtensions" => {
                normalized.rewrite_relative_import_extensions =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.rewrite_relative_import_extensions);
            }
            "allowArbitraryExtensions" => {
                normalized.allow_arbitrary_extensions =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.allow_arbitrary_extensions);
            }
            "experimentalDecorators" => {
                normalized.experimental_decorators =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.experimental_decorators);
            }
            "importHelpers" => {
                normalized.import_helpers = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.import_helpers);
            }
            "emitDecoratorMetadata" => {
                normalized.emit_decorator_metadata =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.emit_decorator_metadata);
            }
            "noUnusedLocals" => {
                normalized.no_unused_locals =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_unused_locals);
            }
            "noUnusedParameters" => {
                normalized.no_unused_parameters =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.no_unused_parameters);
            }
            "allowUnreachableCode" => {
                if let Some(allow) = parse_bool_option(key, value, config_dir, diagnostics) {
                    normalized.allow_unreachable_code = allow;
                    normalized.report_unreachable_code = !allow;
                }
            }
            "allowUnusedLabels" => {
                normalized.allow_unused_labels = parse_bool_option(key, value, config_dir, diagnostics)
                    .or(normalized.allow_unused_labels);
            }
            "target" => {
                normalized.target = parse_target_option(value, config_dir, diagnostics);
            }
            "module" => {
                normalized.module = parse_module_option(value, config_dir, diagnostics);
                explicit_module = Some(normalized.module);
            }
            "useDefineForClassFields" => {
                explicit_use_define_for_class_fields =
                    parse_bool_option(key, value, config_dir, diagnostics);
            }
            "moduleResolution" => {
                explicit_module_resolution =
                    parse_module_resolution_option(value, config_dir, diagnostics);
            }
            "jsx" => {
                normalized.jsx = parse_jsx_option(value, config_dir, diagnostics);
            }
            "moduleDetection" => {
                if let Some(kind) = parse_module_detection_option(value, config_dir, diagnostics) {
                    normalized.module_detection = kind;
                }
            }
            "jsxFactory" | "jsxFragmentFactory" | "reactNamespace" | "jsxImportSource" => {
                let Some(text) = value.as_str() else {
                    diagnostics.push(ConfigDiagnostic {
                        code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                        message: format!("`{key}` must be a string"),
                        file_name: config_dir.to_path_buf(),
                    });
                    continue;
                };
                let slot = match key.as_str() {
                    "jsxFactory" => &mut normalized.jsx_factory,
                    "jsxFragmentFactory" => &mut normalized.jsx_fragment_factory,
                    "reactNamespace" => &mut normalized.react_namespace,
                    _ => &mut normalized.jsx_import_source,
                };
                *slot = Some(text.to_string());
            }
            "allowJs" => {
                normalized.allow_js = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.allow_js);
            }
            "erasableSyntaxOnly" => {
                normalized.erasable_syntax_only = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.erasable_syntax_only);
            }
            "checkJs" => {
                if let Some(check_js) = parse_bool_option(key, value, config_dir, diagnostics) {
                    normalized.check_js = Some(check_js);
                }
            }
            "noEmit" => {
                normalized.no_emit = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.no_emit);
            }
            "noCheck" => {
                normalized.no_check = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.no_check);
            }
            "noResolve" => {
                normalized.no_resolve = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.no_resolve);
            }
            "skipLibCheck" => {
                normalized.skip_lib_check = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.skip_lib_check);
            }
            "esModuleInterop" => {
                normalized.es_module_interop =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.es_module_interop);
            }
            "allowSyntheticDefaultImports" => {
                normalized.allow_synthetic_default_imports =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.allow_synthetic_default_imports);
            }
            "allowUmdGlobalAccess" => {
                normalized.allow_umd_global_access =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.allow_umd_global_access);
            }
            "noLib" => {
                normalized.no_lib = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.no_lib);
            }
            "paths" => {
                normalized.paths = parse_paths_option(value, diagnostics, config_dir);
            }
            "baseUrl" => {
                normalized.base_url = parse_base_url_option(value, config_dir, diagnostics);
            }
            "lib" => {
                normalized.lib = parse_string_list_option(value, diagnostics, config_dir);
            }
            "typeRoots" => {
                normalized.type_roots = parse_path_list_option(value, config_dir, diagnostics);
            }
            "types" => {
                normalized.types = Some(parse_string_list_option(value, diagnostics, config_dir));
            }
            "resolveJsonModule" => {
                explicit_resolve_json_module =
                    parse_bool_option(key, value, config_dir, diagnostics);
            }
            "resolvePackageJsonExports" => {
                normalized.resolve_package_json_exports =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.resolve_package_json_exports);
            }
            "resolvePackageJsonImports" => {
                normalized.resolve_package_json_imports =
                    parse_bool_option(key, value, config_dir, diagnostics)
                        .unwrap_or(normalized.resolve_package_json_imports);
            }
            "customConditions" => {
                normalized.custom_conditions =
                    parse_string_list_option(value, diagnostics, config_dir);
            }
            "libReplacement" => {
                normalized.lib_replacement = parse_bool_option(key, value, config_dir, diagnostics)
                    .unwrap_or(normalized.lib_replacement);
            }
            other => match find_tsconfig_option(other) {
                Some(definition) => match definition.support {
                    TsConfigOptionSupport::KnownNoop => {
                        validate_tsconfig_option_value(definition, value, config_dir, diagnostics);
                    }
                    TsConfigOptionSupport::UnsupportedLegacy => {
                        handle_legacy_tsconfig_option(definition, value, config_dir, diagnostics);
                    }
                    TsConfigOptionSupport::Supported => {
                        diagnostics.push(ConfigDiagnostic {
                            code: ConfigDiagnosticCode::UnknownCompilerOption,
                            message: format!("unknown compiler option `{other}`"),
                            file_name: config_dir.to_path_buf(),
                        });
                    }
                },
                None => {
                    diagnostics.push(ConfigDiagnostic {
                        code: ConfigDiagnosticCode::UnknownCompilerOption,
                        message: format!("unknown compiler option `{other}`"),
                        file_name: config_dir.to_path_buf(),
                    });
                }
            },
        }
    }

    normalized.no_implicit_any = explicit_no_implicit_any.unwrap_or(normalized.strict);
    normalized.no_implicit_this = explicit_no_implicit_this.unwrap_or(normalized.strict);
    normalized.emit_module = explicit_module.unwrap_or(match normalized.target {
        ScriptTarget::ESNext => ModuleKind::ESNext,
        target if target >= ScriptTarget::ES2022 => ModuleKind::ES2022,
        target if target >= ScriptTarget::ES2020 => ModuleKind::ES2020,
        _ => ModuleKind::ES2015,
    });
    normalized.use_define_for_class_fields = explicit_use_define_for_class_fields
        .unwrap_or(normalized.target >= ScriptTarget::ES2022);
    normalized.strict_null_checks = explicit_strict_null_checks.unwrap_or(normalized.strict);
    normalized.strict_bind_call_apply =
        explicit_strict_bind_call_apply.unwrap_or(normalized.strict);
    normalized.strict_builtin_iterator_return =
        explicit_strict_builtin_iterator_return.unwrap_or(normalized.strict);
    normalized.strict_property_initialization =
        explicit_strict_property_initialization.unwrap_or(normalized.strict);
    normalized.use_unknown_in_catch_variables =
        explicit_use_unknown_in_catch_variables.unwrap_or(normalized.strict);
    // tsgo's `GetModuleResolutionKind`: an unwritten (or legacy) resolver
    // follows the emit module kind.
    normalized.module_resolution =
        explicit_module_resolution.unwrap_or(match normalized.emit_module {
            ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 => {
                ModuleResolutionKind::Node16
            }
            ModuleKind::NodeNext => ModuleResolutionKind::NodeNext,
            _ => ModuleResolutionKind::Bundler,
        });
    // tsgo's `GetResolveJsonModule`, read after the whole option map so the
    // resolver and module kind have landed whatever order they appear in.
    normalized.resolve_json_module = explicit_resolve_json_module.unwrap_or(
        matches!(normalized.emit_module, ModuleKind::Node20 | ModuleKind::NodeNext)
            || normalized.module_resolution == ModuleResolutionKind::Bundler,
    );
    if normalized.es_module_interop {
        normalized.allow_synthetic_default_imports = true;
    }

    normalized
}

fn parse_bool_option(
    key: &str,
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<bool> {
    match value.as_bool() {
        Some(bool) => Some(bool),
        None => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("`{key}` must be a boolean"),
                file_name: file_name.to_path_buf(),
            });
            None
        }
    }
}

fn parse_target_option(
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ScriptTarget {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`target` must be a string".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return ScriptTarget::ES2015;
    };

    match raw.to_ascii_lowercase().as_str() {
        "es6" | "es2015" => ScriptTarget::ES2015,
        "es2016" => ScriptTarget::ES2016,
        "es2017" => ScriptTarget::ES2017,
        "es2018" => ScriptTarget::ES2018,
        "es2019" => ScriptTarget::ES2019,
        "es2020" => ScriptTarget::ES2020,
        "es2021" => ScriptTarget::ES2021,
        "es2022" => ScriptTarget::ES2022,
        "es2023" => ScriptTarget::ES2023,
        "es2024" => ScriptTarget::ES2024,
        "es2025" => ScriptTarget::ES2025,
        "esnext" => ScriptTarget::ESNext,
        "es3" | "es5" => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue,
                message: format!("legacy target `{raw}` is not supported; using es2015"),
                file_name: file_name.to_path_buf(),
            });
            ScriptTarget::ES2015
        }
        other => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("unsupported target `{other}`"),
                file_name: file_name.to_path_buf(),
            });
            ScriptTarget::ES2015
        }
    }
}

fn parse_module_option(
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ModuleKind {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`module` must be a string".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return ModuleKind::Preserve;
    };

    match raw.to_ascii_lowercase().as_str() {
        "commonjs" => ModuleKind::CommonJS,
        "es6" | "es2015" => ModuleKind::ES2015,
        "es2020" => ModuleKind::ES2020,
        "es2022" => ModuleKind::ES2022,
        "esnext" => ModuleKind::ESNext,
        "node16" => ModuleKind::Node16,
        "node18" => ModuleKind::Node18,
        "node20" => ModuleKind::Node20,
        "nodenext" => ModuleKind::NodeNext,
        "preserve" => ModuleKind::Preserve,
        "amd" | "umd" | "system" | "systemjs" | "none" => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue,
                message: format!("legacy module kind `{raw}` is not supported; using preserve"),
                file_name: file_name.to_path_buf(),
            });
            ModuleKind::Preserve
        }
        other => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("unsupported module kind `{other}`"),
                file_name: file_name.to_path_buf(),
            });
            ModuleKind::Preserve
        }
    }
}

fn parse_module_resolution_option(
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<ModuleResolutionKind> {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`moduleResolution` must be a string".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return None;
    };

    match raw.to_ascii_lowercase().as_str() {
        "node16" => Some(ModuleResolutionKind::Node16),
        "node20" => Some(ModuleResolutionKind::Node20),
        "nodenext" => Some(ModuleResolutionKind::NodeNext),
        "bundler" => Some(ModuleResolutionKind::Bundler),
        "classic" | "node" | "node10" => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::UnsupportedLegacyCompilerOptionValue,
                message: format!(
                    "legacy moduleResolution `{raw}` is not supported; the module kind decides"
                ),
                file_name: file_name.to_path_buf(),
            });
            None
        }
        other => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("unsupported moduleResolution `{other}`"),
                file_name: file_name.to_path_buf(),
            });
            None
        }
    }
}

fn parse_jsx_option(
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<JsxMode> {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`jsx` must be a string".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return None;
    };

    match raw.to_ascii_lowercase().as_str() {
        "preserve" => Some(JsxMode::Preserve),
        "react" => Some(JsxMode::React),
        "react-jsx" => Some(JsxMode::ReactJsx),
        "react-jsxdev" => Some(JsxMode::ReactJsxDev),
        "react-native" => Some(JsxMode::ReactNative),
        other => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("unsupported jsx mode `{other}`"),
                file_name: file_name.to_path_buf(),
            });
            None
        }
    }
}

fn parse_module_detection_option(
    value: &Value,
    file_name: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<ModuleDetectionKind> {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`moduleDetection` must be a string".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return None;
    };
    match raw.to_ascii_lowercase().as_str() {
        "auto" => Some(ModuleDetectionKind::Auto),
        "legacy" => Some(ModuleDetectionKind::Legacy),
        "force" => Some(ModuleDetectionKind::Force),
        other => {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("unsupported moduleDetection `{other}`"),
                file_name: file_name.to_path_buf(),
            });
            None
        }
    }
}

fn validate_tsconfig_option_value(
    definition: &TsConfigOptionDefinition,
    value: &Value,
    config_dir: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    match definition.value_kind {
        TsConfigOptionValueKind::Boolean => {
            let _ = parse_bool_option(definition.name, value, config_dir, diagnostics);
        }
        TsConfigOptionValueKind::String => {
            if value.as_str().is_none() {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                    message: format!("`{}` must be a string", definition.name),
                    file_name: config_dir.to_path_buf(),
                });
            }
        }
        TsConfigOptionValueKind::StringArray => {
            let _ = parse_string_list_option(value, diagnostics, config_dir);
        }
        TsConfigOptionValueKind::StringMapToStringArray => {
            let _ = parse_paths_option(value, diagnostics, config_dir);
        }
        TsConfigOptionValueKind::ObjectArray => {
            let Some(entries) = value.as_array() else {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                    message: format!("`{}` must be an array of objects", definition.name),
                    file_name: config_dir.to_path_buf(),
                });
                return;
            };

            for entry in entries {
                if !entry.is_object() {
                    diagnostics.push(ConfigDiagnostic {
                        code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                        message: format!("`{}` entries must be objects", definition.name),
                        file_name: config_dir.to_path_buf(),
                    });
                    return;
                }
            }
        }
    }
}

fn handle_legacy_tsconfig_option(
    definition: &TsConfigOptionDefinition,
    value: &Value,
    config_dir: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    match definition.name {
        "outFile" => {
            if value.as_str().is_none() {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                    message: format!("`{}` must be a string", definition.name),
                    file_name: config_dir.to_path_buf(),
                });
                return;
            }
        }
        "downlevelIteration" => {
            if value.as_bool().is_none() {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                    message: "`downlevelIteration` must be a boolean".to_string(),
                    file_name: config_dir.to_path_buf(),
                });
                return;
            }
        }
        _ => {
            validate_tsconfig_option_value(definition, value, config_dir, diagnostics);
            return;
        }
    }

    diagnostics.push(ConfigDiagnostic {
        code: ConfigDiagnosticCode::UnsupportedLegacyCompilerOption,
        message: format!(
            "legacy compiler option `{}` is not supported",
            definition.name
        ),
        file_name: config_dir.to_path_buf(),
    });
}

fn parse_base_url_option(
    value: &Value,
    config_dir: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Option<PathBuf> {
    let Some(raw) = value.as_str() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`baseUrl` must be a string".to_string(),
            file_name: config_dir.to_path_buf(),
        });
        return None;
    };

    Some(resolve_path(config_dir, raw))
}

fn parse_path_list_option(
    value: &Value,
    config_dir: &Path,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Vec<PathBuf> {
    let Some(entries) = value.as_array() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`typeRoots` must be an array of strings".to_string(),
            file_name: config_dir.to_path_buf(),
        });
        return Vec::new();
    };

    let mut paths = Vec::new();
    for entry in entries {
        let Some(raw) = entry.as_str() else {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: "`typeRoots` entries must be strings".to_string(),
                file_name: config_dir.to_path_buf(),
            });
            continue;
        };
        paths.push(resolve_path(config_dir, raw));
    }
    paths
}

fn parse_string_list_option(
    value: &Value,
    diagnostics: &mut Vec<ConfigDiagnostic>,
    file_name: &Path,
) -> Vec<String> {
    let Some(entries) = value.as_array() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "expected an array of strings".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return Vec::new();
    };

    let mut values = Vec::new();
    for entry in entries {
        let Some(raw) = entry.as_str() else {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: "array entries must be strings".to_string(),
                file_name: file_name.to_path_buf(),
            });
            continue;
        };
        values.push(raw.to_string());
    }
    values
}

fn parse_paths_option(
    value: &Value,
    diagnostics: &mut Vec<ConfigDiagnostic>,
    file_name: &Path,
) -> Vec<PathMapping> {
    let Some(object) = value.as_object() else {
        diagnostics.push(ConfigDiagnostic {
            code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
            message: "`paths` must be an object".to_string(),
            file_name: file_name.to_path_buf(),
        });
        return Vec::new();
    };

    let mut paths = Vec::new();
    for (pattern, substitutions_value) in object {
        let Some(substitutions_array) = substitutions_value.as_array() else {
            diagnostics.push(ConfigDiagnostic {
                code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                message: format!("paths entry `{pattern}` must be an array of strings"),
                file_name: file_name.to_path_buf(),
            });
            continue;
        };

        let mut substitutions = Vec::new();
        for substitution in substitutions_array {
            let Some(raw) = substitution.as_str() else {
                diagnostics.push(ConfigDiagnostic {
                    code: ConfigDiagnosticCode::InvalidCompilerOptionValue,
                    message: format!("paths entry `{pattern}` contains a non-string substitution"),
                    file_name: file_name.to_path_buf(),
                });
                continue;
            };
            substitutions.push(raw.to_string());
        }

        paths.push(PathMapping {
            pattern: pattern.clone(),
            substitutions,
        });
    }

    paths
}
