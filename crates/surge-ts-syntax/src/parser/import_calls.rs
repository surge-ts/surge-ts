//! Module specifiers written as `import("...")` rather than as an
//! `import`/`export` declaration: type-position import types
//! (`import("pkg").T`, `typeof import("pkg")`) and dynamic import expressions.
//!
//! tsc puts both in the program's module graph, so the loader has to resolve
//! them too — a package reached only through an import type still contributes
//! its `/// <reference types="..." />` directives and its ambient
//! `declare module` blocks.

use oxc_ast::ast::{Expression, ImportExpression, Program, TSImportType};
use oxc_ast_visit::Visit;

use super::spans::text_span_from_oxc_span;
use crate::{ParsedImportCall, ParsedImportCallKind};

#[derive(Default)]
struct ImportCallCollector {
    specifiers: Vec<String>,
    import_calls: Vec<ParsedImportCall>,
    /// A JavaScript file's `require("m")` reads a module too.
    javascript: bool,
    /// The `require` calls a top-level variable statement lowers to import
    /// declarations, which report an unresolved module themselves.
    declared_requires: std::collections::HashSet<u32>,
}

impl<'a> Visit<'a> for ImportCallCollector {
    fn visit_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        if self.javascript
            && matches!(&call.callee, Expression::Identifier(callee) if callee.name == "require")
            && call.arguments.len() == 1
            && let Some(oxc_ast::ast::Argument::StringLiteral(literal)) = call.arguments.first()
        {
            self.specifiers.push(literal.value.to_string());
            if !self.declared_requires.contains(&call.span.start) {
                self.import_calls.push(ParsedImportCall {
                    specifier: literal.value.to_string(),
                    specifier_span: text_span_from_oxc_span(literal.span),
                    kind: ParsedImportCallKind::Require,
                });
            }
        }
        oxc_ast_visit::walk::walk_call_expression(self, call);
    }

    fn visit_ts_import_type(&mut self, import_type: &TSImportType<'a>) {
        self.specifiers.push(import_type.source.value.to_string());
        // Import attributes can pick another resolution mode, which the
        // lowering does not follow either (`parse_type`). A written argument
        // that is no string literal (`import(T)`) is recovered as an empty one,
        // which names no module (TS1141 is the parser's).
        if import_type.options.is_none() && !import_type.source.value.is_empty() {
            self.import_calls.push(ParsedImportCall {
                specifier: import_type.source.value.to_string(),
                specifier_span: text_span_from_oxc_span(import_type.source.span),
                kind: ParsedImportCallKind::Type,
            });
        }
        oxc_ast_visit::walk::walk_ts_import_type(self, import_type);
    }

    fn visit_import_expression(&mut self, import_expression: &ImportExpression<'a>) {
        // tsc's `IsStringLiteralLike`: a template without substitutions names
        // a module as well as a string literal does.
        let literal = match &import_expression.source {
            Expression::StringLiteral(literal) => Some((literal.value.to_string(), literal.span)),
            Expression::TemplateLiteral(template) => template
                .single_quasi()
                .map(|value| (value.to_string(), template.span)),
            _ => None,
        };
        if let Some((specifier, span)) = literal {
            self.specifiers.push(specifier.clone());
            self.import_calls.push(ParsedImportCall {
                specifier,
                specifier_span: text_span_from_oxc_span(span),
                kind: ParsedImportCallKind::Expression,
            });
        }
        oxc_ast_visit::walk::walk_import_expression(self, import_expression);
    }
}

/// Cheap pre-filter: an `import` keyword immediately followed by `(` (modulo
/// whitespace). Skips the whole-AST walk for the vast majority of files, which
/// never use the form.
fn has_import_call(source_text: &str) -> bool {
    let bytes = source_text.as_bytes();
    let mut rest = source_text;
    let mut base = 0usize;
    while let Some(offset) = rest.find("import") {
        let mut index = base + offset + "import".len();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) == Some(&b'(') {
            return true;
        }
        base += offset + "import".len();
        rest = &source_text[base..];
    }
    false
}

/// The deduplicated specifiers the loader resolves, and every
/// [`ParsedImportCall`] in source order.
pub(crate) fn collect_import_call_specifiers(
    program: &Program<'_>,
    source_text: &str,
    javascript: bool,
) -> (Vec<String>, Vec<ParsedImportCall>) {
    if !has_import_call(source_text) && !(javascript && source_text.contains("require")) {
        return (Vec::new(), Vec::new());
    }
    let mut collector = ImportCallCollector { javascript, ..ImportCallCollector::default() };
    if javascript {
        collector.declared_requires = super::commonjs::top_level_require_declaration_calls(program);
    }
    collector.visit_program(program);

    // Bundled `.d.ts` files repeat the same `import("pkg")` on dozens of
    // members; the loader resolves each specifier per importer, so collapse
    // them here rather than in every consumer.
    let mut seen = std::collections::HashSet::new();
    collector
        .specifiers
        .retain(|specifier| seen.insert(specifier.clone()));
    (collector.specifiers, collector.import_calls)
}
