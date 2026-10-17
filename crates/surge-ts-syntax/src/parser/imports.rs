use oxc_ast::ast::{
    ImportAttributeKey, ImportDeclaration, ImportDeclarationSpecifier, ImportOrExportKind,
    ImportSpecifier, ModuleExportName, TSImportEqualsDeclaration, TSModuleReference, WithClause,
};
use oxc_span::GetSpan;

use crate::{
    ParsedImportDeclaration, ParsedImportKind, ParsedImportSpecifier,
    ParsedResolutionModeAttribute, ResolutionModeOverride,
};

use super::spans::text_span_from_oxc_span;

/// An import declaration, with its inline `type` specifiers
/// (`import { type A, b }`) split off into an `import type` of their own: they
/// bind no value, exactly as if written that way.
pub(crate) fn parse_import_declarations(
    declaration: &ImportDeclaration<'_>,
) -> Option<Vec<ParsedImportDeclaration>> {
    let resolution_mode = resolution_mode_attribute(
        declaration.with_clause.as_deref(),
        matches!(declaration.import_kind, ImportOrExportKind::Type),
    );
    let mut parsed = parse_import_declaration_parts(declaration)?;
    for import in &mut parsed {
        import.resolution_mode = resolution_mode;
    }
    Some(parsed)
}

/// tsc's `GetResolutionModeOverride`: the first attribute keyed
/// `resolution-mode`, when its value is `import` or `require`.
pub(crate) fn resolution_mode_attribute(
    with_clause: Option<&WithClause<'_>>,
    type_only: bool,
) -> Option<ParsedResolutionModeAttribute> {
    let attribute = with_clause?.with_entries.iter().find(|attribute| {
        let key = match &attribute.key {
            ImportAttributeKey::Identifier(identifier) => identifier.name.as_str(),
            ImportAttributeKey::StringLiteral(literal) => literal.value.as_str(),
        };
        key == "resolution-mode"
    })?;
    let mode = match attribute.value.value.as_str() {
        "import" => ResolutionModeOverride::Import,
        "require" => ResolutionModeOverride::Require,
        _ => return None,
    };
    Some(ParsedResolutionModeAttribute {
        mode,
        selects_resolution: type_only,
    })
}

fn parse_import_declaration_parts(
    declaration: &ImportDeclaration<'_>,
) -> Option<Vec<ParsedImportDeclaration>> {
    // `import d, * as ns from "m"` binds the module's default and its
    // namespace, as the two imports written apart would.
    if let Some(specifiers) = declaration.specifiers.as_ref()
        && let [
            ImportDeclarationSpecifier::ImportDefaultSpecifier(default),
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace),
        ] = specifiers.as_slice()
    {
        let is_type_only = matches!(declaration.import_kind, ImportOrExportKind::Type);
        let import = |kind| ParsedImportDeclaration {
            kind,
            module_specifier: declaration.source.value.to_string(),
            module_specifier_span: Some(text_span_from_oxc_span(declaration.source.span)),
            span: Some(text_span_from_oxc_span(declaration.span)),
            resolution_mode: None,
            inline_type_specifiers: false,
        };
        let default_name = default.local.name.to_string();
        let default_span = Some(text_span_from_oxc_span(default.local.span));
        let default_kind = if is_type_only {
            ParsedImportKind::TypeOnlyDefault {
                local_name: default_name,
                name_span: default_span,
            }
        } else {
            ParsedImportKind::Default {
                local_name: default_name,
                name_span: default_span,
            }
        };
        return Some(vec![
            import(default_kind),
            import(ParsedImportKind::Namespace {
                local_name: namespace.local.name.to_string(),
                name_span: Some(text_span_from_oxc_span(namespace.local.span)),
                is_type_only,
            }),
        ]);
    }
    let mut parsed = vec![parse_import_declaration(declaration)?];
    if matches!(declaration.import_kind, ImportOrExportKind::Type) {
        return Some(parsed);
    }
    let type_specifiers: Vec<ParsedImportSpecifier> = declaration
        .specifiers
        .iter()
        .flatten()
        .filter_map(|specifier| match specifier {
            ImportDeclarationSpecifier::ImportSpecifier(specifier)
                if matches!(specifier.import_kind, ImportOrExportKind::Type) =>
            {
                parse_import_specifier(specifier.as_ref())
            }
            _ => None,
        })
        .collect();
    if !type_specifiers.is_empty() {
        parsed.push(ParsedImportDeclaration {
            kind: ParsedImportKind::Named {
                is_type_only: true,
                specifiers: type_specifiers,
            },
            module_specifier: declaration.source.value.to_string(),
            module_specifier_span: Some(text_span_from_oxc_span(declaration.source.span)),
            span: Some(text_span_from_oxc_span(declaration.span)),
            resolution_mode: None,
            inline_type_specifiers: true,
        });
    }
    Some(parsed)
}

fn parse_import_declaration(
    declaration: &ImportDeclaration<'_>,
) -> Option<ParsedImportDeclaration> {
    let module_specifier = declaration.source.value.to_string();
    let module_specifier_span = Some(text_span_from_oxc_span(declaration.source.span));
    let span = Some(text_span_from_oxc_span(declaration.span));

    let Some(specifiers) = declaration.specifiers.as_ref() else {
        return Some(ParsedImportDeclaration {
            kind: ParsedImportKind::SideEffect,
            module_specifier,
            module_specifier_span,
            span,
            resolution_mode: None,
            inline_type_specifiers: false,
        });
    };

    let mut parsed_specifiers = Vec::new();
    let mut parsed_default_specifier = None;
    let mut parsed_namespace_specifier = None;

    for specifier in specifiers {
        match specifier {
            // Split off by `parse_import_declarations`.
            ImportDeclarationSpecifier::ImportSpecifier(specifier)
                if matches!(specifier.import_kind, ImportOrExportKind::Type)
                    && !matches!(declaration.import_kind, ImportOrExportKind::Type) => {}
            ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                let Some(parsed_specifier) = parse_import_specifier(specifier.as_ref()) else {
                    return Some(ParsedImportDeclaration {
                        kind: ParsedImportKind::Unsupported,
                        module_specifier,
                        module_specifier_span,
                        span,
                        resolution_mode: None,
                        inline_type_specifiers: false,
                    });
                };

                parsed_specifiers.push(parsed_specifier);
            }
            ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                if parsed_default_specifier.is_some() || parsed_namespace_specifier.is_some() {
                    return Some(ParsedImportDeclaration {
                        kind: ParsedImportKind::Unsupported,
                        module_specifier,
                        module_specifier_span,
                        span,
                        resolution_mode: None,
                        inline_type_specifiers: false,
                    });
                }

                parsed_default_specifier = Some(ParsedImportKind::Default {
                    local_name: specifier.local.name.to_string(),
                    name_span: Some(text_span_from_oxc_span(specifier.local.span)),
                });
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                if parsed_default_specifier.is_some()
                    || parsed_namespace_specifier.is_some()
                    || !parsed_specifiers.is_empty()
                {
                    return Some(ParsedImportDeclaration {
                        kind: ParsedImportKind::Unsupported,
                        module_specifier,
                        module_specifier_span,
                        span,
                        resolution_mode: None,
                        inline_type_specifiers: false,
                    });
                }

                parsed_namespace_specifier = Some(ParsedImportKind::Namespace {
                    local_name: specifier.local.name.to_string(),
                    name_span: Some(text_span_from_oxc_span(specifier.local.span)),
                    is_type_only: false,
                });
            }
        }
    }

    let is_type_only = matches!(declaration.import_kind, ImportOrExportKind::Type);

    if is_type_only
        && let Some(ParsedImportKind::Default {
            local_name,
            name_span,
        }) = parsed_default_specifier
    {
        return Some(ParsedImportDeclaration {
            kind: ParsedImportKind::TypeOnlyDefault {
                local_name,
                name_span,
            },
            module_specifier,
            module_specifier_span,
            span,
            resolution_mode: None,
            inline_type_specifiers: false,
        });
    }

    if let Some(default_specifier) = parsed_default_specifier {
        if !parsed_specifiers.is_empty() {
            let ParsedImportKind::Default {
                local_name,
                name_span,
            } = default_specifier
            else {
                unreachable!();
            };

            return Some(ParsedImportDeclaration {
                kind: ParsedImportKind::DefaultAndNamed {
                    local_name,
                    name_span,
                    is_type_only,
                    specifiers: parsed_specifiers,
                },
                module_specifier,
                module_specifier_span,
                span,
                resolution_mode: None,
                inline_type_specifiers: false,
            });
        }

        return Some(ParsedImportDeclaration {
            kind: default_specifier,
            module_specifier,
            module_specifier_span,
            span,
            resolution_mode: None,
            inline_type_specifiers: false,
        });
    }

    if let Some(namespace_specifier) = parsed_namespace_specifier {
        let kind = match namespace_specifier {
            ParsedImportKind::Namespace {
                local_name,
                name_span,
                ..
            } => ParsedImportKind::Namespace {
                local_name,
                name_span,
                is_type_only,
            },
            _ => namespace_specifier,
        };

        return Some(ParsedImportDeclaration {
            kind,
            module_specifier,
            module_specifier_span,
            span,
            resolution_mode: None,
            inline_type_specifiers: false,
        });
    }

    Some(ParsedImportDeclaration {
        kind: ParsedImportKind::Named {
            is_type_only,
            specifiers: parsed_specifiers,
        },
        module_specifier,
        module_specifier_span,
        span,
        resolution_mode: None,
        inline_type_specifiers: false,
    })
}

pub(crate) fn parse_import_equals_declaration(
    declaration: &TSImportEqualsDeclaration<'_>,
) -> Option<ParsedImportDeclaration> {
    let span = Some(text_span_from_oxc_span(declaration.span));

    let TSModuleReference::ExternalModuleReference(reference) = &declaration.module_reference
    else {
        let target = match &declaration.module_reference {
            TSModuleReference::IdentifierReference(identifier) => identifier.name.to_string(),
            TSModuleReference::QualifiedName(name) => qualified_name_text(name),
            TSModuleReference::ExternalModuleReference(_) => unreachable!(),
        };
        return Some(ParsedImportDeclaration {
            kind: ParsedImportKind::EntityAlias {
                local_name: declaration.id.name.to_string(),
                name_span: Some(text_span_from_oxc_span(declaration.id.span)),
                target,
            },
            module_specifier: String::new(),
            module_specifier_span: Some(text_span_from_oxc_span(declaration.module_reference.span())),
            span,
            resolution_mode: None,
            inline_type_specifiers: false,
        });
    };

    // The parser's stand-in for a specifier that is no string literal has no
    // raw text.
    let module_specifier = if reference.expression.raw.is_none() {
        crate::NON_LITERAL_MODULE_SPECIFIER.to_string()
    } else {
        reference.expression.value.to_string()
    };
    let module_specifier_span = Some(text_span_from_oxc_span(reference.expression.span));

    Some(ParsedImportDeclaration {
        kind: ParsedImportKind::Equals {
            local_name: declaration.id.name.to_string(),
            name_span: Some(text_span_from_oxc_span(declaration.id.span)),
            is_type_only: declaration.import_kind.is_type(),
            exported: false,
        },
        module_specifier,
        module_specifier_span,
        span,
        resolution_mode: None,
        inline_type_specifiers: false,
    })
}

fn parse_import_specifier(specifier: &ImportSpecifier<'_>) -> Option<ParsedImportSpecifier> {
    let imported_name = module_export_name_to_string(&specifier.imported);
    let local_name = specifier.local.name.to_string();

    Some(ParsedImportSpecifier {
        imported_name,
        local_name,
        name_span: Some(text_span_from_oxc_span(specifier.imported.span())),
        span: Some(text_span_from_oxc_span(specifier.span)),
    })
}

fn module_export_name_to_string(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(string_literal) => string_literal.value.to_string(),
    }
}

fn qualified_name_text(name: &oxc_ast::ast::TSQualifiedName<'_>) -> String {
    let left = match &name.left {
        oxc_ast::ast::TSTypeName::IdentifierReference(identifier) => identifier.name.to_string(),
        oxc_ast::ast::TSTypeName::QualifiedName(left) => qualified_name_text(left),
        oxc_ast::ast::TSTypeName::ThisExpression(_) => "this".to_string(),
    };
    format!("{left}.{}", name.right.name)
}
