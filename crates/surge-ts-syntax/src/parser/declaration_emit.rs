//! What tsc's declaration emitter (`transformers/declarations`) reports under
//! `declaration` alone, where the answer is in the source text.
//!
//! tsc's `GetDiagnosticsOfAnyProgram` asks for these under `noEmit` only when
//! the program reports nothing else, which the caller decides.

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, ClassElement, Declaration, Expression, MethodDefinitionKind, PropertyKey, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// One declaration-emit error, with its message arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationEmitDiagnostic {
    pub code: u32,
    pub start: u32,
    pub end: u32,
    pub args: Vec<String>,
}

/// TS4094 for an exported variable typed by the class expression it is
/// initialized with: the emitter writes that anonymous class type as an
/// object type literal, which has no way to keep a member private or
/// protected. Members a class inherits from its base are not read here.
pub fn declaration_emit_diagnostics(source_text: &str, file_name: &str) -> Vec<DeclarationEmitDiagnostic> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source_text, source_type).parse();
    if parsed.panicked {
        return Vec::new();
    }
    let mut out = Vec::new();
    for statement in &parsed.program.body {
        let Statement::ExportNamedDeclaration(export) = statement else {
            continue;
        };
        let Some(Declaration::VariableDeclaration(declaration)) = &export.declaration else {
            continue;
        };
        if declaration.declare {
            continue;
        }
        for declarator in &declaration.declarations {
            let (BindingPattern::BindingIdentifier(identifier), None, Some(init)) =
                (&declarator.id, &declarator.type_annotation, &declarator.init)
            else {
                continue;
            };
            let Expression::ClassExpression(class) = init.without_parentheses() else {
                continue;
            };
            let mut names = hidden_member_names(class);
            names.sort();
            names.dedup();
            for name in names {
                out.push(DeclarationEmitDiagnostic {
                    code: 4094,
                    start: identifier.span.start,
                    end: identifier.span.end,
                    args: vec![name],
                });
            }
        }
    }
    out
}

fn hidden_member_names(class: &oxc_ast::ast::Class<'_>) -> Vec<String> {
    use oxc_ast::ast::TSAccessibility;
    let hidden = |accessibility: Option<TSAccessibility>| {
        matches!(accessibility, Some(TSAccessibility::Private | TSAccessibility::Protected))
    };
    let key_name = |key: &PropertyKey<'_>| match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    };
    let mut names = Vec::new();
    for element in &class.body.body {
        let (key, accessibility) = match element {
            ClassElement::PropertyDefinition(property) => (&property.key, property.accessibility),
            ClassElement::AccessorProperty(property) => (&property.key, property.accessibility),
            ClassElement::MethodDefinition(method) if method.kind == MethodDefinitionKind::Constructor => {
                for parameter in &method.value.params.items {
                    if hidden(parameter.accessibility)
                        && let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern
                    {
                        names.push(identifier.name.to_string());
                    }
                }
                continue;
            }
            ClassElement::MethodDefinition(method) => (&method.key, method.accessibility),
            _ => continue,
        };
        if hidden(accessibility) || matches!(key, PropertyKey::PrivateIdentifier(_)) {
            names.extend(key_name(key));
        }
    }
    names
}
