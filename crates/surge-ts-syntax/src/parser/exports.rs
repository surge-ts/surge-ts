use oxc_ast::ast::{
    Declaration, ExportAllDeclaration, ExportDefaultDeclaration, ExportDefaultDeclarationKind,
    ExportNamedDeclaration, ExportSpecifier, Expression, ImportOrExportKind, ModuleExportName,
    TSExportAssignment,
};
use oxc_span::GetSpan;

use crate::{
    ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedExportSpecifier,
    ParsedExpression, ParsedImportKind, ParsedStatement,
};

use super::expressions::{
    parse_array_expression, parse_conditional_expression, parse_expression,
    parse_object_properties, parse_static_member_expression, parse_unary_expression,
};
use super::spans::text_span_from_oxc_span;
use super::{
    parse_function_declaration, parse_interface_declaration, parse_type_alias_declaration,
    parse_variable_declaration,
};

pub(crate) fn parse_export_named_declaration(
    declaration: &ExportNamedDeclaration<'_>,
) -> Option<Vec<ParsedStatement>> {
    let span = Some(text_span_from_oxc_span(declaration.span));

    if let Some(wrapped_declaration) = declaration.declaration.as_ref() {
        if declaration.source.is_some() {
            return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                ParsedExportDeclaration::Unsupported { span },
            ))]);
        }

        return parse_exported_declaration(wrapped_declaration, declaration.export_kind, span).or_else(
            || {
                Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Unsupported { span },
                ))])
            },
        );
    }

    if declaration.specifiers.is_empty() {
        return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
            ParsedExportDeclaration::Empty { span },
        ))]);
    }

    let mut specifiers = Vec::new();

    for specifier in &declaration.specifiers {
        let Some(parsed_specifier) = parse_export_specifier(specifier) else {
            return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                ParsedExportDeclaration::Unsupported { span },
            ))]);
        };

        specifiers.push(parsed_specifier);
    }

    Some(vec![ParsedStatement::ExportDeclaration(Box::new(
        ParsedExportDeclaration::Named {
            is_type_only: matches!(declaration.export_kind, ImportOrExportKind::Type),
            specifiers,
            module_specifier: declaration
                .source
                .as_ref()
                .map(|source| source.value.to_string()),
            module_specifier_span: declaration
                .source
                .as_ref()
                .map(|source| text_span_from_oxc_span(source.span)),
            span,
            resolution_mode: super::imports::resolution_mode_attribute(
                declaration.with_clause.as_deref(),
                matches!(declaration.export_kind, ImportOrExportKind::Type),
            ),
        },
    ))])
}

pub(crate) fn parse_export_default_declaration(
    declaration: &ExportDefaultDeclaration<'_>,
) -> Option<Vec<ParsedStatement>> {
    let span = Some(text_span_from_oxc_span(declaration.span));

    let parsed_declaration = match &declaration.declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
            // An anonymous `export default function () {}` binds as `default`.
            let parsed = match function.id.as_ref() {
                Some(_) => parse_function_declaration(function),
                None => super::functions::parse_function_declaration_named(
                    function,
                    "default".to_string(),
                    Some(text_span_from_oxc_span(function.span)),
                ),
            };
            let Some(function) = parsed else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported {
                            span: Some(text_span_from_oxc_span(declaration.span)),
                        },
                        span,
                    },
                ))]);
            };

            ParsedDefaultExportDeclaration::Function(function)
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            // An anonymous `export default class {}` binds as `default`.
            let parsed = match &class.id {
                Some(_) => super::classes::parse_class_declaration(class),
                None => super::classes::parse_class_declaration_named(
                    class,
                    "default".to_string(),
                    text_span_from_oxc_span(oxc_span::Span::new(
                        class.span.start,
                        class.span.start + "class".len() as u32,
                    )),
                ),
            };
            match parsed {
                Some(class) => ParsedDefaultExportDeclaration::Class(class),
                None => ParsedDefaultExportDeclaration::Unsupported {
                    span: Some(text_span_from_oxc_span(declaration.span)),
                },
            }
        }
        ExportDefaultDeclarationKind::BooleanLiteral(boolean_literal) => {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::BooleanLiteral(
                boolean_literal.value,
            ))
        }
        ExportDefaultDeclarationKind::NumericLiteral(numeric_literal) => {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::NumberLiteral(
                numeric_literal.value.to_string(),
            ))
        }
        ExportDefaultDeclarationKind::StringLiteral(string_literal) => {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::StringLiteral(
                string_literal.value.to_string(),
            ))
        }
        ExportDefaultDeclarationKind::Identifier(identifier) => {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::Identifier {
                name: identifier.name.to_string(),
                span: Some(text_span_from_oxc_span(identifier.span)),
            })
        }
        ExportDefaultDeclarationKind::ObjectExpression(object_expression) => {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::ObjectLiteral {
                properties: parse_object_properties(object_expression),
                span: Some(text_span_from_oxc_span(object_expression.span())),
            })
        }
        ExportDefaultDeclarationKind::ArrayExpression(array_expression) => {
            let Some(parsed_expression) = parse_array_expression(array_expression) else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                        span,
                    },
                ))]);
            };

            ParsedDefaultExportDeclaration::Expression(parsed_expression)
        }
        ExportDefaultDeclarationKind::UnaryExpression(unary_expression) => {
            let Some(parsed_expression) = parse_unary_expression(unary_expression) else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                        span,
                    },
                ))]);
            };

            ParsedDefaultExportDeclaration::Expression(parsed_expression)
        }
        ExportDefaultDeclarationKind::ConditionalExpression(conditional_expression) => {
            let Some(parsed_expression) = parse_conditional_expression(conditional_expression)
            else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                        span,
                    },
                ))]);
            };

            ParsedDefaultExportDeclaration::Expression(parsed_expression)
        }
        ExportDefaultDeclarationKind::StaticMemberExpression(member_expression) => {
            let Some(parsed_expression) = parse_static_member_expression(member_expression) else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                        span,
                    },
                ))]);
            };

            ParsedDefaultExportDeclaration::Expression(parsed_expression)
        }
        // Every remaining expression form — `satisfies`, `as`, a call, a
        // template — is still an expression and must be checked. Only the
        // genuinely non-expression kinds (an interface declaration, an
        // unparseable node) fall through to `Unsupported` below.
        // `ExportDefaultDeclarationKind` inherits `Expression` through oxc's
        // `inherit_variants!`, which is what makes `to_expression` available.
        other if other.is_expression() => {
            let (parsed_expression, _) = parse_expression(other.to_expression());
            ParsedDefaultExportDeclaration::Expression(parsed_expression)
        }
        // `export default interface A {}` declares `A` in the module and
        // exports it as `default`, as `interface A {}; export { A as default }`.
        ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) => {
            let Some(parsed) = parse_interface_declaration(interface) else {
                return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                        span,
                    },
                ))]);
            };
            let name_span = Some(text_span_from_oxc_span(interface.id.span));
            let local_name = parsed.name.clone();
            return Some(vec![
                ParsedStatement::InterfaceDeclaration(Box::new(parsed)),
                ParsedStatement::ExportDeclaration(Box::new(ParsedExportDeclaration::Named {
                    is_type_only: false,
                    specifiers: vec![ParsedExportSpecifier {
                        local_name,
                        exported_name: "default".to_string(),
                        name_span,
                        exported_name_span: name_span,
                        is_type_only: false,
                    }],
                    module_specifier: None,
                    module_specifier_span: None,
                    span,
                    resolution_mode: None,
                })),
            ]);
        }
        _ => {
            return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                    span,
                },
            ))]);
        }
    };
    // The literal forms above are lowered without `parse_expression`, which
    // is what applies a JSDoc `@type` to the exported expression.
    let parsed_declaration = match parsed_declaration {
        ParsedDefaultExportDeclaration::Expression(expression)
            if super::spans::lowering_javascript()
                && !matches!(expression, ParsedExpression::TypeAssertion { annotation: true, .. })
                && let Some(value) = declaration.declaration.as_expression()
                && let Some((ty, _, anchor)) =
                    super::jsdoc::assignment_type_at(value.span().start, value.span().end) =>
        {
            ParsedDefaultExportDeclaration::Expression(ParsedExpression::TypeAssertion {
                expression: Box::new(expression),
                expression_span: Some(text_span_from_oxc_span(value.span())),
                ty,
                type_span: Some(anchor),
                annotation: true,
            })
        }
        other => other,
    };

    Some(vec![ParsedStatement::ExportDeclaration(Box::new(
        ParsedExportDeclaration::Default {
            declaration: parsed_declaration,
            span,
        },
    ))])
}

pub(crate) fn parse_export_assignment(
    declaration: &TSExportAssignment<'_>,
) -> Option<Vec<ParsedStatement>> {
    let span = Some(text_span_from_oxc_span(declaration.span));

    let Expression::Identifier(identifier) = &declaration.expression else {
        let (expression, expression_span) = parse_expression(&declaration.expression);
        return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
            ParsedExportDeclaration::EqualsExpression {
                expression: Box::new(expression),
                expression_span: Some(text_span_from_oxc_span(expression_span)),
                entity_name: entity_name_expression_text(&declaration.expression),
                later_writes: Vec::new(),
                span,
            },
        ))]);
    };

    Some(vec![ParsedStatement::ExportDeclaration(Box::new(
        ParsedExportDeclaration::Equals {
            exported_name: identifier.name.to_string(),
            exported_name_span: Some(text_span_from_oxc_span(identifier.span)),
            span,
        },
    ))])
}

/// tsc's `isEntityNameExpression`: an identifier, or a property access naming
/// an identifier on one. Parentheses end it — `export = (A.B)` exports a value,
/// not an alias — which is why this reads the oxc node rather than the parsed
/// expression, where they are already gone.
fn entity_name_expression_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::StaticMemberExpression(member) if !member.optional => Some(format!(
            "{}.{}",
            entity_name_expression_text(&member.object)?,
            member.property.name
        )),
        _ => None,
    }
}

pub(crate) fn parse_export_all_declaration(
    declaration: &ExportAllDeclaration<'_>,
) -> Option<Vec<ParsedStatement>> {
    let span = Some(text_span_from_oxc_span(declaration.span));
    let module_specifier = declaration.source.value.to_string();
    let module_specifier_span = Some(text_span_from_oxc_span(declaration.source.span));

    let is_type_only = matches!(declaration.export_kind, ImportOrExportKind::Type);

    if let Some(exported) = declaration.exported.as_ref() {
        return Some(vec![ParsedStatement::ExportDeclaration(Box::new(
            ParsedExportDeclaration::Namespace {
                exported_name: module_export_name_to_string(exported),
                exported_name_span: Some(text_span_from_oxc_span(exported.span())),
                module_specifier,
                module_specifier_span,
                span,
                is_type_only,
            },
        ))]);
    }

    Some(vec![ParsedStatement::ExportDeclaration(Box::new(
        ParsedExportDeclaration::All {
            module_specifier,
            module_specifier_span,
            span,
            is_type_only,
            resolution_mode: super::imports::resolution_mode_attribute(
                declaration.with_clause.as_deref(),
                is_type_only,
            ),
        },
    ))])
}

fn parse_exported_declaration(
    declaration: &Declaration<'_>,
    export_kind: ImportOrExportKind,
    export_span: Option<crate::TextSpan>,
) -> Option<Vec<ParsedStatement>> {
    let is_type_only = matches!(export_kind, ImportOrExportKind::Type);

    let statements = match declaration {
        Declaration::VariableDeclaration(variable) => parse_variable_declaration(variable),
        Declaration::FunctionDeclaration(function) => {
            let declarations: Vec<ParsedStatement> = super::functions::parse_function_declarations(function)
                .into_iter()
                .map(|function| ParsedStatement::FunctionDeclaration(Box::new(function)))
                .collect();
            if declarations.is_empty() {
                return None;
            }
            declarations
        }
        Declaration::TSTypeAliasDeclaration(type_alias) => parse_type_alias_declaration(type_alias)
            .map(|type_alias| vec![ParsedStatement::TypeAliasDeclaration(Box::new(type_alias))])?,
        Declaration::TSInterfaceDeclaration(interface) => parse_interface_declaration(interface)
            .map(|interface| vec![ParsedStatement::InterfaceDeclaration(Box::new(interface))])?,
        Declaration::ClassDeclaration(class) => super::classes::parse_class_declaration(class)
            .map(|class| vec![ParsedStatement::ClassDeclaration(Box::new(class))])?,
        Declaration::TSEnumDeclaration(enum_declaration) => {
            super::enums::parse_enum_declaration(enum_declaration, true)
        }
        Declaration::TSModuleDeclaration(module) => super::parse_ts_module_declaration(module),
        Declaration::TSImportEqualsDeclaration(import_equals) => {
            let mut import = super::imports::parse_import_equals_declaration(import_equals)?;
            // tsc's node for `export import x = …` begins at its `export`.
            if matches!(import.kind, ParsedImportKind::EntityAlias { .. } | ParsedImportKind::Equals { .. }) {
                import.span = export_span;
            }
            if let ParsedImportKind::Equals { exported, .. } = &mut import.kind {
                *exported = true;
            }
            // `export import local = require("m")` declares the alias and
            // exports it — the exported alias `export { local }` would give.
            if let ParsedImportKind::Equals {
                local_name,
                name_span,
                ..
            } = &import.kind
            {
                let export = ParsedExportDeclaration::Named {
                    is_type_only: false,
                    specifiers: vec![ParsedExportSpecifier {
                        local_name: local_name.clone(),
                        exported_name: local_name.clone(),
                        name_span: *name_span,
                        exported_name_span: *name_span,
                        is_type_only: false,
                    }],
                    module_specifier: None,
                    module_specifier_span: None,
                    span: import.span,
                    resolution_mode: None,
                };
                return Some(vec![
                    ParsedStatement::ImportDeclaration(Box::new(import)),
                    ParsedStatement::ExportDeclaration(Box::new(export)),
                ]);
            }
            vec![ParsedStatement::ImportDeclaration(Box::new(import))]
        }
        _ => return None,
    };

    Some(wrap_exported_statements(statements, is_type_only))
}

fn parse_export_specifier(specifier: &ExportSpecifier<'_>) -> Option<ParsedExportSpecifier> {
    let local_name = module_export_name_to_string(&specifier.local);
    let exported_name = module_export_name_to_string(&specifier.exported);

    Some(ParsedExportSpecifier {
        local_name,
        exported_name,
        name_span: Some(text_span_from_oxc_span(specifier.local.span())),
        exported_name_span: Some(text_span_from_oxc_span(specifier.exported.span())),
        is_type_only: matches!(specifier.export_kind, ImportOrExportKind::Type),
    })
}

fn module_export_name_to_string(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(string_literal) => string_literal.value.to_string(),
    }
}

fn wrap_exported_statements(
    statements: Vec<ParsedStatement>,
    is_type_only: bool,
) -> Vec<ParsedStatement> {
    statements
        .into_iter()
        .map(|statement| {
            ParsedStatement::ExportDeclaration(Box::new(ParsedExportDeclaration::Statement {
                declaration: Box::new(statement),
                is_type_only,
            }))
        })
        .collect()
}
