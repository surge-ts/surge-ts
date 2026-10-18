use oxc_ast::ast::{
    ArrayPattern, AssignmentOperator, AssignmentTarget, BindingPattern, BindingProperty,
    Declaration, Expression, ExpressionStatement, ModuleDeclaration, ObjectPattern, PropertyKey,
    Statement, TSGlobalDeclaration, TSModuleDeclaration, TSModuleDeclarationBody,
    TSModuleDeclarationName, VariableDeclaration, VariableDeclarationKind,
};

use crate::{
    ParsedAssignment, ParsedDeclareModuleDeclaration, ParsedExportDeclaration, ParsedExpression,
    ParsedNamespaceDeclaration, ParsedStatement, ParsedVariableDeclaration, ParsedVariableKind,
};

mod classes;
mod commonjs;
mod emit_helpers;
mod entry;
mod enum_values;
mod enums;
mod exports;
mod expressions;
mod function_types;
mod functions;
mod grammar;
mod grammar_context;
mod grammar_modifiers;
mod grammar_merges;
mod grammar_recovered;
mod import_aliases;
mod import_attributes;
mod import_calls;
mod imports;
mod interfaces;
mod declaration_emit;
mod isolated_declarations;
mod jsdoc;
mod json;
mod reachability;
mod reads;
mod cycle_reads;
pub use cycle_reads::{cycle_read, enable_cycle_reads};
mod jsx_uses;
mod number_text;
mod private_names;
mod reference_directives;
mod scanner_checks;
mod spans;
mod suppressions;
mod types;
mod callback_functions;
mod circular_returns;
mod written_names;
mod closure_resets;
mod let_assignments;
mod writes;

use self::classes::parse_class_declaration;
use self::exports::parse_export_named_declaration;
use self::exports::{
    parse_export_all_declaration, parse_export_assignment, parse_export_default_declaration,
};
use self::expressions::{
    parse_call_expression, parse_conditional_expression, parse_expression,
    parse_static_member_expression, parse_unary_expression,
};
use self::functions::parse_function_declaration;
use self::imports::{parse_import_declarations, parse_import_equals_declaration};
use self::interfaces::parse_interface_declaration;
pub use self::reference_directives::{
    extract_check_directive, extract_reference_path_directives, extract_reference_type_directives,
    invalid_reference_resolution_modes,
};
use self::spans::text_span_from_oxc_span;
use self::types::{parse_type_alias_declaration, parse_type_annotation};
pub use emit_helpers::{EmitHelperOptions, EmitHelperRequest, emit_helper_requests, external_emit_helpers};
pub use entry::{ParserWorker, is_declaration_file_name, is_javascript_file_name, parse_source};
pub use isolated_declarations::{
    AugmentationFacts, ElidedImport, IsolatedDeclarationDiagnostic, augmentation_facts, isolated_declaration_diagnostics,
};
pub use declaration_emit::{DeclarationEmitDiagnostic, declaration_emit_diagnostics};
pub use json::{is_json_file_name, parse_json_module_type};
pub use jsx_uses::{JsxRuntimeOptions, entity_root as jsx_entity_root, jsx_runtime_import};
pub use number_text::js_number_to_string;
pub use reachability::{AMBIENT_EXPORT_ASSIGNMENT, UNREACHABLE_PRESERVED_CONST_ENUMS, UNREACHABLE_UNPRESERVED_CONST_ENUMS};
pub use types::merge_parsed_call_signatures;

fn parse_statement(statement: &Statement<'_>) -> Option<Vec<ParsedStatement>> {
    let lowered = lower_statement(statement);
    let Some(export) = commonjs::chained_module_exports_declaration(statement) else {
        return lowered;
    };
    let mut statements = lowered.unwrap_or_default();
    statements.push(export);
    Some(statements)
}

fn lower_statement(statement: &Statement<'_>) -> Option<Vec<ParsedStatement>> {
    if let Statement::VariableDeclaration(declaration) = statement
        && let Some(imports) = commonjs::require_imports(declaration)
    {
        return Some(imports);
    }
    if let Statement::ExpressionStatement(expression_statement) = statement
        && let Some(exports) = commonjs::export_statement(expression_statement)
    {
        return Some(exports);
    }
    if let Some(module_declaration) = statement.as_module_declaration() {
        return parse_module_declaration(module_declaration);
    }

    if let Some(declaration) = statement.as_declaration() {
        return parse_declaration(declaration);
    }

    match statement {
        Statement::ExpressionStatement(expression_statement) => {
            let excess = destructuring_assignment_excess(&expression_statement.expression)
                .map(|excess| ParsedStatement::Expression(Box::new(excess)));
            let destructured = parse_destructuring_assignment(&expression_statement.expression);
            if !destructured.is_empty() {
                return Some(
                    destructured
                        .into_iter()
                        .map(|assignment| match assignment {
                            LoweredAssignment::Name(assignment) => ParsedStatement::Assignment(Box::new(assignment)),
                            LoweredAssignment::Member(assignment) => {
                                ParsedStatement::MemberAssignment(Box::new(assignment))
                            }
                        })
                        .chain(excess)
                        .collect(),
                );
            }
            parse_expression_statement(expression_statement)
                .map(|statement| std::iter::once(statement).chain(excess).collect())
        }
        Statement::IfStatement(if_statement) => {
            let mut statements: Vec<ParsedStatement> = functions::unbraced_branch_declarations(if_statement)
                .filter_map(parse_statement)
                .flatten()
                .collect();
            statements.extend(
                functions::parse_if_statement(if_statement).map(|if_statement| ParsedStatement::If(Box::new(if_statement))),
            );
            Some(statements)
        }
        Statement::BlockStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::WhileStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::TryStatement(_)
        | Statement::WithStatement(_)
        | Statement::ThrowStatement(_) => functions::parse_function_body_statement(statement)
            .map(|statements| vec![ParsedStatement::Block(statements)]),
        // A label is not a scope: what its statement declares is declared
        // where the label is.
        Statement::LabeledStatement(labeled) => parse_statement(&labeled.body),
        _ => None,
    }
}

fn parse_module_declaration(
    module_declaration: &ModuleDeclaration<'_>,
) -> Option<Vec<ParsedStatement>> {
    match module_declaration {
        ModuleDeclaration::ImportDeclaration(import) => parse_import_declarations(import).map(|imports| {
            imports
                .into_iter()
                .map(|import| ParsedStatement::ImportDeclaration(Box::new(import)))
                .collect()
        }),
        ModuleDeclaration::ExportNamedDeclaration(export) => parse_export_named_declaration(export),
        ModuleDeclaration::ExportDefaultDeclaration(export) => {
            parse_export_default_declaration(export)
        }
        ModuleDeclaration::ExportAllDeclaration(export) => parse_export_all_declaration(export),
        ModuleDeclaration::TSExportAssignment(export) => parse_export_assignment(export),
        ModuleDeclaration::TSNamespaceExportDeclaration(export) => {
            Some(vec![ParsedStatement::ExportDeclaration(Box::new(
                ParsedExportDeclaration::NamespaceExport {
                    exported_name: export.id.name.to_string(),
                    exported_name_span: Some(text_span_from_oxc_span(export.id.span)),
                    span: Some(text_span_from_oxc_span(export.span)),
                },
            ))])
        }
    }
}

fn parse_declaration(declaration: &Declaration<'_>) -> Option<Vec<ParsedStatement>> {
    // Exhaustive against today's oxc, but the fallback arm stays: a new oxc
    // `Declaration` variant must degrade to `UnsupportedDeclaration`, not fail
    // the build.
    #[allow(unreachable_patterns)]
    match declaration {
        Declaration::VariableDeclaration(declaration) => {
            Some(parse_variable_declaration(declaration))
        }
        Declaration::FunctionDeclaration(function) => Some(
            functions::parse_function_declarations(function)
                .into_iter()
                .map(|function| ParsedStatement::FunctionDeclaration(Box::new(function)))
                .collect(),
        ),
        Declaration::TSTypeAliasDeclaration(type_alias) => parse_type_alias_declaration(type_alias)
            .map(|type_alias| vec![ParsedStatement::TypeAliasDeclaration(Box::new(type_alias))]),
        Declaration::TSInterfaceDeclaration(interface) => parse_interface_declaration(interface)
            .map(|interface| vec![ParsedStatement::InterfaceDeclaration(Box::new(interface))]),
        Declaration::ClassDeclaration(class) => parse_class_declaration(class)
            .map(|class| vec![ParsedStatement::ClassDeclaration(Box::new(class))]),
        Declaration::TSEnumDeclaration(enum_declaration) => {
            Some(enums::parse_enum_declaration(enum_declaration, false))
        }
        Declaration::TSModuleDeclaration(module) => Some(parse_ts_module_declaration(module)),
        Declaration::TSGlobalDeclaration(global) => Some(parse_ts_global_declaration(global)),
        Declaration::TSImportEqualsDeclaration(import_equals) => {
            parse_import_equals_declaration(import_equals)
                .map(|import| vec![ParsedStatement::ImportDeclaration(Box::new(import))])
        }
        _ => Some(vec![ParsedStatement::UnsupportedDeclaration {
            span: Some(text_span_from_oxc_span(oxc_span::GetSpan::span(
                declaration,
            ))),
        }]),
    }
}

/// The unique symbol a `const` declares: tsc's `TypeFlagsUniqueESSymbol` type
/// is its declaration's, so the name carries the declaration's offset after a
/// NUL — `s` and a namespace's `N.s` in one file are different symbols.
fn unique_symbol_declaration(identifier: &oxc_ast::ast::BindingIdentifier<'_>) -> crate::ParsedType {
    crate::ParsedType::UniqueSymbol(std::sync::Arc::from(format!(
        "{}\u{0}{}",
        identifier.name, identifier.span.start
    )))
}

thread_local! {
    static CIRCULAR_ANNOTATIONS: std::cell::RefCell<std::rc::Rc<Vec<u32>>> = Default::default();
}

/// Installs, for the duration of `f`, where the variables whose annotations
/// are circular through `typeof` start (TS2502): tsc types each `errorType`.
pub(crate) fn with_circular_annotations<R>(starts: std::rc::Rc<Vec<u32>>, f: impl FnOnce() -> R) -> R {
    struct Restore(std::rc::Rc<Vec<u32>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = std::mem::take(&mut self.0);
            CIRCULAR_ANNOTATIONS.with(|slot| *slot.borrow_mut() = previous);
        }
    }
    let previous = CIRCULAR_ANNOTATIONS.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), starts));
    let _restore = Restore(previous);
    f()
}

fn is_circular_annotation(name_start: u32) -> bool {
    CIRCULAR_ANNOTATIONS.with(|slot| slot.borrow().binary_search(&name_start).is_ok())
}

fn parse_variable_declaration(declaration: &VariableDeclaration<'_>) -> Vec<ParsedStatement> {
    let list = std::sync::Arc::new(declaration_list_shape(declaration));
    let kind = match declaration.kind {
        VariableDeclarationKind::Var => ParsedVariableKind::Var,
        VariableDeclarationKind::Let => ParsedVariableKind::Let,
        // `using` is block-scoped and const-like (`isVarConstLike`).
        VariableDeclarationKind::Const | VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing => {
            ParsedVariableKind::Const
        }
    };

    declaration
        .declarations
        .iter()
        .flat_map(|declarator| {
            let declared_type = match (&declarator.id, declarator.type_annotation.as_ref()) {
                (BindingPattern::BindingIdentifier(identifier), Some(annotation))
                    if matches!(
                        &annotation.type_annotation,
                        oxc_ast::ast::TSType::TSTypeOperatorType(operator)
                            if operator.operator == oxc_ast::ast::TSTypeOperatorOperator::Unique
                    ) =>
                {
                    Some(unique_symbol_declaration(identifier))
                }
                (BindingPattern::BindingIdentifier(identifier), Some(_))
                    if is_circular_annotation(identifier.span.start) =>
                {
                    Some(crate::ParsedType::ErrorType)
                }
                (_, annotation) => {
                    annotation.and_then(|annotation| parse_type_annotation(annotation))
                }
            };
            let jsdoc_type = declarator
                .type_annotation
                .is_none()
                .then(|| jsdoc::declared_type_at(declarator.span.start))
                .flatten()
                .map(|(ty, span)| match (&declarator.id, kind) {
                    (BindingPattern::BindingIdentifier(identifier), ParsedVariableKind::Const)
                        if jsdoc::declared_unique_symbol_at(declarator.span.start) =>
                    {
                        (unique_symbol_declaration(identifier), span)
                    }
                    _ => (ty, span),
                });
            let annotation_span = declarator
                .type_annotation
                .as_ref()
                .map(|annotation| text_span_from_oxc_span(annotation.span))
                .or(jsdoc_type.as_ref().map(|(_, span)| *span));
            let declared_type = declared_type.or(jsdoc_type.map(|(ty, _)| ty)).or_else(|| {
                // tsc's `getESSymbolLikeTypeForNode`: `Symbol()` initializing a
                // `const` creates that declaration's own unique symbol.
                let (BindingPattern::BindingIdentifier(identifier), ParsedVariableKind::Const, Some(init)) =
                    (&declarator.id, kind, declarator.init.as_ref())
                else {
                    return None;
                };
                is_symbol_constructor_call(init.without_parentheses()).then(|| unique_symbol_declaration(identifier))
            });
            let Some(init) = declarator.init.as_ref() else {
                // A destructuring declaration with no initializer — an ambient
                // one — declares its elements with the annotation's types.
                if let (
                    BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_),
                    Some(ty),
                ) = (&declarator.id, declared_type.clone())
                {
                    let mut declarations = Vec::new();
                    annotated_pattern_declarations(
                        &declarator.id,
                        ty,
                        declaration.declare,
                        kind,
                        &mut declarations,
                    );
                    return declarations;
                }
                return parse_binding_pattern_declarations_with_definite(
                    &declarator.id,
                    None,
                    None,
                    declaration.declare,
                    kind,
                    declared_type,
                    declarator.definite,
                );
            };

            if is_empty_binding_pattern(&declarator.id) {
                let (source, source_span) = parse_expression(init);
                return vec![ParsedStatement::Expression(Box::new(ParsedExpression::EmptyBindingPattern {
                    pattern: Box::new(functions::parse_binding_name(&declarator.id)),
                    source: Box::new(source),
                    source_span: Some(text_span_from_oxc_span(source_span)),
                    declared_type,
                }))];
            }

            // `const X = class {}` declares a class under the variable's name,
            // which is how tsc writes it; a binding that can be reassigned is
            // left a variable.
            if let (BindingPattern::BindingIdentifier(identifier), None, ParsedVariableKind::Const) =
                (&declarator.id, declarator.type_annotation.as_ref(), kind)
                && !declaration.declare
                && let Expression::ClassExpression(class) = init.without_parentheses()
                && class.id.as_ref().is_none_or(|id| id.name == identifier.name)
            {
                let name_span = match &class.id {
                    Some(id) => id.span,
                    None => oxc_span::Span::new(class.span.start, class.span.start + "class".len() as u32),
                };
                if let Some(mut parsed) = classes::parse_class_declaration_named(
                    class,
                    identifier.name.to_string(),
                    text_span_from_oxc_span(name_span),
                ) {
                    parsed.const_binding = true;
                    parsed.expression_span = Some(text_span_from_oxc_span(class.span));
                    return vec![ParsedStatement::ClassDeclaration(Box::new(parsed))];
                }
            }
            let (mut initializer, initializer_span) = match expressions::parse_javascript_member_assignment_chain(init) {
                Some(chain) => (chain, oxc_span::GetSpan::span(init)),
                None => parse_expression(init),
            };
            expressions::mark_expando_container(&mut initializer);
            let initializer_span = Some(text_span_from_oxc_span(initializer_span));
            let initializer = match jsdoc::initializer_satisfies_at(oxc_span::GetSpan::span(init).start) {
                Some((ty, span)) => ParsedExpression::SatisfiesExpression {
                    expression: Box::new(initializer),
                    span: initializer_span,
                    target_type: ty,
                    target_span: Some(span),
                    keyword_span: None,
                },
                None => initializer,
            };
            // An annotated pattern's elements read from the annotation, which
            // the initializer must be assignable to; an unannotated pattern is
            // the initializer's contextual type.
            let initializer = match (&declarator.id, declared_type.clone(), annotation_span) {
                (
                    BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_),
                    Some(ty),
                    Some(_),
                ) => ParsedExpression::TypeAssertion {
                    expression: Box::new(initializer),
                    expression_span: initializer_span,
                    ty,
                    type_span: Some(text_span_from_oxc_span(oxc_span::GetSpan::span(&declarator.id))),
                    annotation: true,
                },
                _ if annotation_span.is_none() => {
                    with_pattern_context(&declarator.id, initializer)
                }
                _ => initializer,
            };
            let excess_properties = if annotation_span.is_none() {
                pattern_excess_properties(&declarator.id, &initializer)
            } else {
                Vec::new()
            };
            let annotated_pattern = match (&declarator.id, &declared_type) {
                (BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_), Some(ty))
                    if binding_pattern_has_default(&declarator.id) =>
                {
                    Some(std::sync::Arc::new(crate::ParsedAnnotatedBindingPattern {
                        pattern: functions::parse_binding_name(&declarator.id),
                        declared_type: ty.clone(),
                    }))
                }
                _ => None,
            };

            let mut declarations = parse_binding_pattern_declarations(
                &declarator.id,
                Some(initializer),
                initializer_span,
                declaration.declare,
                kind,
                declared_type,
            );
            if let Some(annotated_pattern) = annotated_pattern
                && let Some(ParsedStatement::VariableDeclaration(first)) = declarations
                    .iter_mut()
                    .find(|statement| matches!(statement, ParsedStatement::VariableDeclaration(_)))
            {
                first.annotated_pattern = Some(annotated_pattern);
            }
            if !excess_properties.is_empty()
                && let Some(ParsedStatement::VariableDeclaration(first)) = declarations
                    .iter_mut()
                    .find(|statement| matches!(statement, ParsedStatement::VariableDeclaration(_)))
            {
                first.pattern_excess_properties = excess_properties;
            }
            declarations
        })
        .map(|statement| match statement {
            ParsedStatement::VariableDeclaration(mut variable) => {
                variable.declaration_list = Some(list.clone());
                ParsedStatement::VariableDeclaration(variable)
            }
            other => other,
        })
        .collect()
}

fn binding_pattern_has_default(pattern: &BindingPattern<'_>) -> bool {
    match pattern {
        BindingPattern::BindingIdentifier(_) => false,
        BindingPattern::AssignmentPattern(_) => true,
        BindingPattern::ObjectPattern(object) => object
            .properties
            .iter()
            .any(|property| binding_pattern_has_default(&property.value)),
        BindingPattern::ArrayPattern(array) => array
            .elements
            .iter()
            .flatten()
            .any(binding_pattern_has_default),
    }
}

/// Each element of a pattern with no initializer is typed as the annotation
/// indexed by its key (`T["a"]`, `T[0]`), the type tsc's
/// `getTypeForBindingElement` reads off the annotated parent. A computed key
/// and a rest element name no single member and are left out.
fn annotated_pattern_declarations(
    pattern: &BindingPattern<'_>,
    parent: crate::ParsedType,
    is_declare: bool,
    kind: ParsedVariableKind,
    declarations: &mut Vec<ParsedStatement>,
) {
    let indexed = |index: crate::ParsedType| {
        crate::ParsedType::IndexedAccess(std::sync::Arc::new(crate::ParsedIndexedAccessType {
            object_type: Box::new(parent.clone()),
            index_type: Box::new(index),
            span: None,
            index_span: None,
        }))
    };
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            declarations.push(ParsedStatement::VariableDeclaration(Box::new(
                ParsedVariableDeclaration {
                    is_enum_object: false,
                    enum_members: None,
                    array_rest_start: None,
                    is_declare,
                    kind,
                    from_binding_pattern: true,
                    has_definite_assertion: false,
                    array_pattern_span: None,
                    name: identifier.name.to_string(),
                    name_span: Some(text_span_from_oxc_span(identifier.span)),
                    declared_type: Some(parent),
                    initializer: None,
                    initializer_span: None,
                    declaration_list: None,
                    annotated_pattern: None,
                    pattern_excess_properties: Vec::new(),
                },
            )));
        }
        BindingPattern::AssignmentPattern(assignment) => {
            annotated_pattern_declarations(&assignment.left, parent, is_declare, kind, declarations);
        }
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                if property.computed {
                    continue;
                }
                let Some(key) = property.key.static_name() else {
                    continue;
                };
                annotated_pattern_declarations(
                    &property.value,
                    indexed(crate::ParsedType::StringLiteral(key.to_string())),
                    is_declare,
                    kind,
                    declarations,
                );
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for (index, element) in array.elements.iter().enumerate() {
                let Some(element) = element else {
                    continue;
                };
                annotated_pattern_declarations(
                    element,
                    indexed(crate::ParsedType::NumberLiteral(index.to_string())),
                    is_declare,
                    kind,
                    declarations,
                );
            }
        }
    }
}

fn declaration_list_shape(declaration: &VariableDeclaration<'_>) -> crate::ParsedDeclarationList {
    use crate::ParsedDeclarationShape;
    let using = matches!(
        declaration.kind,
        VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
    );
    fn name(identifier: &oxc_ast::ast::BindingIdentifier<'_>, always_used: bool) -> ParsedDeclarationShape {
        ParsedDeclarationShape::Name {
            name: identifier.name.to_string(),
            span: Some(text_span_from_oxc_span(identifier.span)),
            always_used,
        }
    }
    // An element of a pattern: `underscore_exempts` when an `_`-prefixed name
    // counts as used there, `rest_sibling` when the pattern's rest needs it.
    fn element(pattern: &BindingPattern<'_>, underscore_exempts: bool, rest_sibling: bool) -> ParsedDeclarationShape {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => name(
                identifier,
                rest_sibling || (underscore_exempts && identifier.name.starts_with('_')),
            ),
            BindingPattern::AssignmentPattern(assignment) => {
                element(&assignment.left, underscore_exempts, rest_sibling)
            }
            BindingPattern::ObjectPattern(object) => {
                let has_rest = object.rest.is_some();
                let mut elements: Vec<_> = object
                    .properties
                    .iter()
                    .map(|property| element(&property.value, !property.shorthand, has_rest))
                    .collect();
                if let Some(rest) = &object.rest {
                    elements.push(element(&rest.argument, false, false));
                }
                ParsedDeclarationShape::Pattern {
                    span: Some(text_span_from_oxc_span(object.span)),
                    elements,
                }
            }
            BindingPattern::ArrayPattern(array) => {
                let mut elements: Vec<_> = array
                    .elements
                    .iter()
                    .map(|element_pattern| match element_pattern {
                        Some(element_pattern) => element(element_pattern, true, false),
                        None => ParsedDeclarationShape::Omitted,
                    })
                    .collect();
                if let Some(rest) = &array.rest {
                    elements.push(element(&rest.argument, true, false));
                }
                ParsedDeclarationShape::Pattern {
                    span: Some(text_span_from_oxc_span(array.span)),
                    elements,
                }
            }
        }
    }
    crate::ParsedDeclarationList {
        disposal: match declaration.kind {
            VariableDeclarationKind::Using => Some(false),
            VariableDeclarationKind::AwaitUsing => Some(true),
            _ => None,
        },
        span: Some(text_span_from_oxc_span(declaration.span)),
        declarations: declaration
            .declarations
            .iter()
            .map(|declarator| match &declarator.id {
                BindingPattern::BindingIdentifier(identifier) => {
                    name(identifier, using && identifier.name.starts_with('_'))
                }
                pattern => element(pattern, false, false),
            })
            .collect(),
    }
}

fn parse_expression_statement(
    expression_statement: &ExpressionStatement<'_>,
) -> Option<ParsedStatement> {
    match &expression_statement.expression {
        Expression::CallExpression(call) if let Some(require) = commonjs::require_call_expression(call) => {
            Some(ParsedStatement::Expression(Box::new(require)))
        }
        Expression::CallExpression(_) => {
            if let Some(call) = parse_call_expression(match &expression_statement.expression {
                Expression::CallExpression(call_expression) => call_expression,
                _ => unreachable!(),
            }) {
                return Some(ParsedStatement::Call(Box::new(call)));
            }

            let (expression, _) = parse_expression(&expression_statement.expression);
            Some(ParsedStatement::Expression(Box::new(expression)))
        }
        Expression::AssignmentExpression(assignment) => {
            if let Some(assignment) = parse_assignment_expression(assignment) {
                return Some(ParsedStatement::Assignment(Box::new(assignment)));
            }
            if let Some(rejected) = expressions::parse_rejected_assignment(assignment) {
                return Some(ParsedStatement::Expression(Box::new(rejected)));
            }
            // A write to a member or element, which the identifier-target
            // parser above does not accept. Dropping it left every module-scope
            // `o.a = …` and `o[k] = …` unchecked.
            functions::parse_member_assignment(assignment)
                .map(|assignment| ParsedStatement::MemberAssignment(Box::new(assignment)))
        }
        Expression::UnaryExpression(unary_expression) => parse_unary_expression(unary_expression)
            .map(|expression| ParsedStatement::Expression(Box::new(expression))),
        Expression::ConditionalExpression(conditional_expression) => {
            parse_conditional_expression(conditional_expression)
                .map(|expression| ParsedStatement::Expression(Box::new(expression)))
        }
        Expression::StaticMemberExpression(member_expression) => {
            parse_static_member_expression(member_expression)
                .map(|expression| ParsedStatement::Expression(Box::new(expression)))
        }
        _ => {
            let (expression, _) = parse_expression(&expression_statement.expression);
            Some(ParsedStatement::Expression(Box::new(expression)))
        }
    }
}


/// The value a logical assignment stores: `x ??= v` is `x = x ?? v`, and
/// `||=`/`&&=` likewise, so the ordinary assignment check and the narrowing it
/// establishes apply. Other compound operators are not modelled and yield
/// `None`.
pub(super) fn logical_assignment_value(
    operator: AssignmentOperator,
    target: ParsedExpression,
    target_span: Option<crate::TextSpan>,
    value: ParsedExpression,
    value_span: Option<crate::TextSpan>,
) -> Option<ParsedExpression> {
    let logical = match operator {
        AssignmentOperator::Assign => return Some(value),
        AssignmentOperator::LogicalNullish => {
            return Some(ParsedExpression::NullishCoalescing {
                left: Box::new(target),
                left_span: target_span,
                right: Box::new(value),
                right_span: value_span,
            });
        }
        AssignmentOperator::LogicalOr => crate::ParsedLogicalOperator::Or,
        AssignmentOperator::LogicalAnd => crate::ParsedLogicalOperator::And,
        // `x op= v` is `x = x op v`, which is how tsc checks it too: the
        // operator's own result type is what the assignment is then checked
        // against (`checkBinaryLikeExpression` feeds `checkAssignmentOperator`).
        other => {
            let binary = compound_assignment_operator(other)?;
            return Some(ParsedExpression::Binary {
                left: Box::new(target),
                left_span: target_span,
                operator: binary,
                operator_span: None,
                right: Box::new(value),
                right_span: value_span,
                compound: true,
            });
        }
    };
    Some(ParsedExpression::Logical {
        left: Box::new(target),
        left_span: target_span,
        operator: logical,
        operator_span: None,
        right: Box::new(value),
        right_span: value_span,
        truthiness_tests: Vec::new(),
    })
}

/// The binary operator a compound assignment applies.
fn compound_assignment_operator(
    operator: AssignmentOperator,
) -> Option<crate::ParsedBinaryOperator> {
    use crate::ParsedBinaryOperator as B;
    Some(match operator {
        AssignmentOperator::Addition => B::Add,
        AssignmentOperator::Subtraction => B::Subtract,
        AssignmentOperator::Multiplication => B::Multiply,
        AssignmentOperator::Division => B::Divide,
        AssignmentOperator::Remainder => B::Remainder,
        AssignmentOperator::Exponential => B::Exponential,
        AssignmentOperator::ShiftLeft => B::ShiftLeft,
        AssignmentOperator::ShiftRight => B::ShiftRight,
        AssignmentOperator::ShiftRightZeroFill => B::ShiftRightZeroFill,
        AssignmentOperator::BitwiseOR => B::BitwiseOR,
        AssignmentOperator::BitwiseXOR => B::BitwiseXOR,
        AssignmentOperator::BitwiseAnd => B::BitwiseAnd,
        _ => return None,
    })
}

/// Where an assignment's target ends as written: past the parentheses that
/// open before it (`(x) = ''`).
pub(crate) fn parenthesized_target_end(assignment: &oxc_ast::ast::AssignmentExpression<'_>, target: oxc_span::Span) -> usize {
    let opened = spans::source_text_of(oxc_span::Span::new(assignment.span.start, target.start)).matches('(').count();
    if opened == 0 {
        return target.end as usize;
    }
    let after = spans::source_text_of(oxc_span::Span::new(target.end, oxc_span::GetSpan::span(&assignment.right).start));
    target.end as usize + after.match_indices(')').nth(opened - 1).map_or(0, |(at, _)| at + 1)
}

fn parse_assignment_expression(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedAssignment> {
    let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left else {
        return None;
    };

    let target_span = Some(text_span_from_oxc_span(identifier.span));
    let (value, value_span) = parse_expression(&assignment.right);
    let value_span = Some(text_span_from_oxc_span(value_span));
    let target = ParsedExpression::Identifier {
        name: identifier.name.to_string(),
        span: target_span,
    };
    let value = logical_assignment_value(assignment.operator, target, target_span, value, value_span)?;

    // tsc reports on the target as written, parentheses included: `(x) = ''`.
    let written_target_span = Some(crate::TextSpan {
        start: assignment.span.start as usize,
        end: parenthesized_target_end(assignment, identifier.span),
    });
    Some(ParsedAssignment {
        target_name: identifier.name.to_string(),
        target_span,
        written_target_span,
        value,
        value_span,
        from_pattern: false,
        shorthand: false,
    })
}

/// One target a destructuring assignment lowers to: a name (`[a] = xs`) or a
/// member (`[o.a] = xs`), each assigned what the pattern reads for it.
pub(crate) enum LoweredAssignment {
    Name(ParsedAssignment),
    Member(crate::ParsedMemberAssignment),
}

/// `[a, b] = source` / `({ a, b: { c } } = source)`: each identifier target is
/// an ordinary assignment of the element or property it reads (with its default
/// applied as `??`, as a binding pattern does), so the checker reports each write
/// on its target the way tsc's `checkDestructuringAssignment` does. A nested
/// pattern reads its elements or properties from its own read, and a rest target
/// is assigned a value the lowering does not type (tsc's
/// `bindDestructuringTargetFlow` assigns every target either way). A member
/// target is a member assignment of what it reads.
pub(crate) fn parse_destructuring_assignment(expression: &Expression<'_>) -> Vec<LoweredAssignment> {
    let Expression::AssignmentExpression(assignment) = expression.without_parentheses() else {
        return Vec::new();
    };
    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
        return Vec::new();
    }
    let (source, source_span) = parse_expression(&assignment.right);
    let source_span = Some(text_span_from_oxc_span(source_span));
    let mut assignments = Vec::new();
    lower_assignment_pattern(&assignment.left, &source, source_span, false, &mut assignments);
    assignments
}

/// A destructuring assignment used as a value (`f(({ a } = o))`): its targets
/// are assigned as the statement form lowers them, and the expression is the
/// source's value, which a contextual type still reaches.
pub(crate) fn destructuring_assignment_value(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedExpression> {
    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign
        || !matches!(
            assignment.left,
            AssignmentTarget::ObjectAssignmentTarget(_) | AssignmentTarget::ArrayAssignmentTarget(_)
        )
    {
        return None;
    }
    let (source, source_span) = parse_expression(&assignment.right);
    let source_span = Some(text_span_from_oxc_span(source_span));
    let mut assignments = Vec::new();
    lower_assignment_pattern(&assignment.left, &source, source_span, false, &mut assignments);
    let mut expressions: Vec<_> = assignments
        .into_iter()
        .map(|assignment| match assignment {
            LoweredAssignment::Name(assignment) => (
                ParsedExpression::Assignment {
                    target_name: assignment.target_name,
                    target_span: assignment.target_span,
                    value: Box::new(assignment.value),
                    value_span: assignment.value_span,
                    from_pattern: assignment.from_pattern,
                    shorthand: assignment.shorthand,
                },
                assignment.target_span,
            ),
            LoweredAssignment::Member(assignment) => {
                let span = assignment.target_span;
                (ParsedExpression::MemberAssignment(Box::new(assignment)), span)
            }
        })
        .collect();
    let mut excess = Vec::new();
    collect_pattern_excess_properties(&assignment_target_binding_name(&assignment.left), &source, &mut excess);
    for property in &mut excess {
        property.typed_targets = true;
    }
    if !excess.is_empty() {
        expressions.push((ParsedExpression::PatternExcessProperties(excess), source_span));
    }
    expressions.push((source, source_span));
    Some(ParsedExpression::Sequence { expressions })
}

/// `({ x } = { x: 0, y: 0 })`: the literal is contextually typed by the
/// pattern, so a member it does not name is excess (`checkObjectLiteral`).
pub(crate) fn destructuring_assignment_excess(expression: &Expression<'_>) -> Option<ParsedExpression> {
    let Expression::AssignmentExpression(assignment) = expression.without_parentheses() else {
        return None;
    };
    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign
        || !matches!(
            assignment.left,
            AssignmentTarget::ObjectAssignmentTarget(_) | AssignmentTarget::ArrayAssignmentTarget(_)
        )
    {
        return None;
    }
    let (source, _) = parse_expression(&assignment.right);
    let mut excess = Vec::new();
    collect_pattern_excess_properties(&assignment_target_binding_name(&assignment.left), &source, &mut excess);
    for property in &mut excess {
        property.typed_targets = true;
    }
    (!excess.is_empty()).then_some(ParsedExpression::PatternExcessProperties(excess))
}

fn assignment_target_binding_name(target: &AssignmentTarget<'_>) -> crate::ParsedBindingName {
    use oxc_ast::ast::{AssignmentTargetMaybeDefault, AssignmentTargetProperty};
    let split = |target: &AssignmentTargetMaybeDefault<'_>| match target {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
            Some((assignment_target_binding_name(&with_default.binding), true))
        }
        other => other.as_assignment_target().map(|target| (assignment_target_binding_name(target), false)),
    };
    let unsupported = || crate::ParsedBindingName::Unsupported {
        span: Some(text_span_from_oxc_span(oxc_span::GetSpan::span(target))),
    };
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => crate::ParsedBindingName::Identifier {
            name: identifier.name.to_string(),
            span: Some(text_span_from_oxc_span(identifier.span)),
        },
        AssignmentTarget::ObjectAssignmentTarget(pattern) => {
            let element = |property_name: String,
                           computed_key: bool,
                           shorthand: bool,
                           binding_name: crate::ParsedBindingName,
                           has_default: bool,
                           span: oxc_span::Span| crate::ParsedObjectBindingElement {
                property_name,
                computed_key,
                key_expression: None,
                shorthand,
                binding_name,
                name_span: None,
                has_default,
                default_value: None,
                default_span: None,
                span: Some(text_span_from_oxc_span(span)),
            };
            let elements = pattern
                .properties
                .iter()
                .map(|property| match property {
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => element(
                        shorthand.binding.name.to_string(),
                        false,
                        true,
                        crate::ParsedBindingName::Identifier {
                            name: shorthand.binding.name.to_string(),
                            span: Some(text_span_from_oxc_span(shorthand.binding.span)),
                        },
                        shorthand.init.is_some(),
                        shorthand.span,
                    ),
                    AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                        let name = property.name.static_name();
                        let (binding_name, has_default) =
                            split(&property.binding).unwrap_or_else(|| (unsupported(), false));
                        element(
                            name.as_deref().unwrap_or_default().to_string(),
                            name.is_none(),
                            false,
                            binding_name,
                            has_default,
                            property.span,
                        )
                    }
                })
                .collect();
            crate::ParsedBindingName::ObjectPattern(crate::ParsedObjectBindingPattern {
                elements,
                rest: pattern
                    .rest
                    .as_ref()
                    .map(|rest| Box::new(assignment_target_binding_name(&rest.target))),
                span: Some(text_span_from_oxc_span(pattern.span)),
            })
        }
        AssignmentTarget::ArrayAssignmentTarget(pattern) => {
            let (elements, defaults): (Vec<_>, Vec<_>) = pattern
                .elements
                .iter()
                .map(|element| match element.as_ref().and_then(split) {
                    Some((binding_name, has_default)) => (Some(binding_name), has_default),
                    None => (None, false),
                })
                .unzip();
            crate::ParsedBindingName::ArrayPattern(crate::ParsedArrayBindingPattern {
                default_values: vec![None; elements.len()],
                elements,
                defaults,
                rest: pattern
                    .rest
                    .as_ref()
                    .map(|rest| Box::new(assignment_target_binding_name(&rest.target))),
                span: Some(text_span_from_oxc_span(pattern.span)),
            })
        }
        _ => unsupported(),
    }
}

fn lower_assignment_pattern(
    pattern: &AssignmentTarget<'_>,
    source: &ParsedExpression,
    source_span: Option<crate::TextSpan>,
    split_defaults: bool,
    assignments: &mut Vec<LoweredAssignment>,
) {
    use oxc_ast::ast::{AssignmentTargetMaybeDefault, AssignmentTargetProperty, IdentifierReference};

    let with_default = |read: ParsedExpression, default: Option<&Expression<'_>>| match default {
        Some(default) => {
            let (default_value, default_span) = parse_expression(default);
            ParsedExpression::NullishCoalescing {
                left: Box::new(read),
                left_span: source_span,
                right: Box::new(default_value),
                right_span: Some(text_span_from_oxc_span(default_span)),
            }
        }
        None => read,
    };
    // `checkDestructuringAssignment` checks a default as an assignment of its
    // own (`checkBinaryExpression`) and then the source less `undefined`;
    // `split_defaults` lowers them apart, as values the target is only checked
    // against.
    let values = |read: ParsedExpression, default: Option<&Expression<'_>>| match default {
        Some(default) if split_defaults => vec![
            parse_expression(default).0,
            ParsedExpression::NonNullAssertion {
                expression: Box::new(read),
                span: source_span,
                in_optional_chain: false,
            },
        ],
        default => vec![with_default(read, default)],
    };
    let assign_identifier =
        |identifier: &IdentifierReference<'_>, value, shorthand: bool, assignments: &mut Vec<LoweredAssignment>| {
            assignments.push(LoweredAssignment::Name(ParsedAssignment {
                target_name: identifier.name.to_string(),
                target_span: Some(text_span_from_oxc_span(identifier.span)),
                written_target_span: Some(text_span_from_oxc_span(identifier.span)),
                value,
                value_span: source_span,
                from_pattern: true,
                shorthand,
            }));
        };
    let assign = |target: &AssignmentTarget<'_>, value, assignments: &mut Vec<LoweredAssignment>| {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                assign_identifier(identifier, value, false, assignments);
            }
            AssignmentTarget::ArrayAssignmentTarget(_)
            | AssignmentTarget::ObjectAssignmentTarget(_) => {
                lower_assignment_pattern(target, &value, source_span, split_defaults, assignments);
            }
            // `checkDestructuringAssignment` checks a member target as it
            // checks the assignment of what it reads; JavaScript's member
            // writes declare expandos, which a pattern does not.
            AssignmentTarget::StaticMemberExpression(_)
            | AssignmentTarget::ComputedMemberExpression(_)
            | AssignmentTarget::PrivateFieldExpression(_)
                if !spans::lowering_javascript() =>
            {
                if let Some((member, member_span)) = functions::member_assignment_target(target) {
                    assignments.push(LoweredAssignment::Member(crate::ParsedMemberAssignment {
                        target: member,
                        target_span: Some(text_span_from_oxc_span(member_span)),
                        value,
                        value_span: source_span,
                        declares_commonjs_export: false,
                    }));
                }
            }
            _ => {}
        }
    };
    fn split_default<'t, 'a>(
        target: &'t AssignmentTargetMaybeDefault<'a>,
    ) -> Option<(&'t AssignmentTarget<'a>, Option<&'t Expression<'a>>)> {
        match target {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
                Some((&with_default.binding, Some(&with_default.init)))
            }
            other => other.as_assignment_target().map(|target| (target, None)),
        }
    }

    match pattern {
        AssignmentTarget::ArrayAssignmentTarget(pattern) => {
            let mut pattern_span = Some(text_span_from_oxc_span(pattern.span));
            for (index, element) in pattern.elements.iter().enumerate() {
                let Some((target, default)) = element.as_ref().and_then(split_default) else {
                    continue;
                };
                let span = Some(text_span_from_oxc_span(oxc_span::GetSpan::span(target)));
                let read = match source {
                    // An array literal source is contextually a tuple: each
                    // target takes its own element, not the union of all of them.
                    ParsedExpression::ArrayLiteral { elements, .. }
                        if elements.iter().take(index + 1).all(|element| !element.spread)
                            && index < elements.len() =>
                    {
                        elements[index].expression.clone()
                    }
                    ParsedExpression::Identifier { name, .. } => ParsedExpression::IndexAccess {
                        object_name: name.clone(),
                        object_span: source_span,
                        index: Box::new(ParsedExpression::NumberLiteral(index.to_string())),
                        index_span: span,
                    },
                    _ => ParsedExpression::ElementAccess {
                        object: Box::new(source.clone()),
                        object_span: source_span,
                        index: Box::new(ParsedExpression::NumberLiteral(index.to_string())),
                        index_span: span,
                    },
                };
                let read = match source {
                    ParsedExpression::ArrayLiteral { .. } => read,
                    _ => ParsedExpression::ArrayPatternElement {
                        source: Box::new(source.clone()),
                        source_span,
                        index,
                        rest: false,
                        tuple_literal: false,
                        read: Box::new(read),
                        pattern_span: pattern_span.take(),
                    },
                };
                for value in values(read, default) {
                    assign(target, value, assignments);
                }
            }
            if let Some(rest) = &pattern.rest {
                // `checkArrayLiteralAssignment` reports a rest element that is
                // not last and checks nothing through it.
                let rest_start = rest.span.start;
                let rest_is_last = pattern
                    .elements
                    .iter()
                    .flatten()
                    .all(|element| oxc_span::GetSpan::span(element).start < rest_start);
                // The pattern is the array literal's contextual type
                // (`getContextualTypeForAssignmentDeclaration`); it is tuple-like
                // with an element before the rest, or a nested array pattern
                // as the rest.
                let tuple_literal = !pattern.elements.is_empty()
                    || matches!(rest.target, AssignmentTarget::ArrayAssignmentTarget(_));
                let value = if rest_is_last {
                    ParsedExpression::ArrayPatternElement {
                        source: Box::new(source.clone()),
                        source_span,
                        index: pattern.elements.len(),
                        rest: true,
                        tuple_literal,
                        read: Box::new(source.clone()),
                        pattern_span: pattern_span.take(),
                    }
                } else {
                    ParsedExpression::Unknown
                };
                assign(&rest.target, value, assignments);
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(pattern) => {
            // A defaulted element of an object-literal source that lacks it
            // reads `undefined` (tsc's `AccessFlags.AllowMissing`); anything
            // else missing is TS2339 on the element.
            let literal_properties = static_object_literal(source);
            let property_read = |name: &str, span, has_default: bool| {
                if has_default
                    && literal_properties.is_some_and(|properties| literal_lacks_property(properties, name))
                {
                    return ParsedExpression::UndefinedLiteral;
                }
                if is_numeric_literal_name(name) {
                    return numeric_name_read(source.clone(), source_span, name, span);
                }
                ParsedExpression::PropertyAccess {
                    object: Box::new(source.clone()),
                    object_span: source_span,
                    property_name: name.to_string(),
                    property_span: span,
                    is_bracketed: true,
                    binding_element: true,
                }
            };
            for property in &pattern.properties {
                match property {
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                        let identifier = &shorthand.binding;
                        let span = Some(text_span_from_oxc_span(identifier.span));
                        let read = property_read(&identifier.name, span, shorthand.init.is_some());
                        for value in values(read, shorthand.init.as_ref()) {
                            assign_identifier(identifier, value, true, assignments);
                        }
                    }
                    AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                        let Some((target, default)) = split_default(&property.binding) else {
                            continue;
                        };
                        let span = Some(text_span_from_oxc_span(oxc_span::GetSpan::span(&property.name)));
                        let read = match property.name.static_name() {
                            Some(name) => property_read(&name, span, default.is_some()),
                            // A computed key's target reads the source indexed by
                            // the key, evaluated where the pattern reaches it.
                            None => match property.name.as_expression() {
                                Some(key) => ParsedExpression::ElementAccess {
                                    object: Box::new(source.clone()),
                                    object_span: source_span,
                                    index: Box::new(parse_expression(key).0),
                                    index_span: Some(text_span_from_oxc_span(oxc_span::GetSpan::span(key))),
                                },
                                None => continue,
                            },
                        };
                        for value in values(read, default) {
                            assign(target, value, assignments);
                        }
                    }
                }
            }
            if let Some(rest) = &pattern.rest {
                // tsc's `getRestType`: the source without the properties the
                // pattern names, a computed key omitting what its type admits.
                let mut omitted = Vec::new();
                let mut computed = Vec::new();
                let mut known = true;
                for property in &pattern.properties {
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                            omitted.push(shorthand.binding.name.to_string());
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                            match property.name.static_name() {
                                Some(name) => omitted.push(name.to_string()),
                                None => match property.name.as_expression() {
                                    Some(key) => computed.push(parse_expression(key).0),
                                    None => known = false,
                                },
                            }
                        }
                    }
                }
                let value = if known {
                    ParsedExpression::ObjectRest {
                        source: Box::new(source.clone()),
                        omitted,
                        computed,
                        name_span: Some(text_span_from_oxc_span(oxc_span::GetSpan::span(&rest.target))),
                    }
                } else {
                    ParsedExpression::Unknown
                };
                assign(&rest.target, value, assignments);
            }
        }
        _ => {}
    }
}

fn parse_binding_pattern_declarations(
    binding: &BindingPattern<'_>,
    initializer: Option<ParsedExpression>,
    initializer_span: Option<crate::TextSpan>,
    is_declare: bool,
    kind: ParsedVariableKind,
    declared_type: Option<crate::ParsedType>,
) -> Vec<ParsedStatement> {
    parse_binding_pattern_declarations_with_definite(
        binding,
        initializer,
        initializer_span,
        is_declare,
        kind,
        declared_type,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_binding_pattern_declarations_with_definite(
    binding: &BindingPattern<'_>,
    initializer: Option<ParsedExpression>,
    initializer_span: Option<crate::TextSpan>,
    is_declare: bool,
    kind: ParsedVariableKind,
    declared_type: Option<crate::ParsedType>,
    has_definite_assertion: bool,
) -> Vec<ParsedStatement> {
    match binding {
        BindingPattern::BindingIdentifier(binding_identifier) => {
            vec![ParsedStatement::VariableDeclaration(Box::new(
                ParsedVariableDeclaration {
                    is_enum_object: false,
                    enum_members: None,
                    array_rest_start: None,
                    is_declare,
                    kind,
                    from_binding_pattern: false,
                    has_definite_assertion,
                    array_pattern_span: None,
                    name: binding_identifier.name.to_string(),
                    name_span: Some(text_span_from_oxc_span(binding_identifier.span)),
                    declared_type,
                    initializer,
                    initializer_span,
                    declaration_list: None,
                    annotated_pattern: None,
                    pattern_excess_properties: Vec::new(),
                },
            ))]
        }
        BindingPattern::AssignmentPattern(assignment_pattern) => {
            // `const { a = 0 } = o` binds `o.a ?? 0`: the default applies exactly
            // when the property is absent, so the binding is never `undefined`.
            let initializer = match initializer {
                Some(initializer) => {
                    let (default_value, default_span) = parse_expression(&assignment_pattern.right);
                    let default_value = with_pattern_context(&assignment_pattern.left, default_value);
                    Some(ParsedExpression::NullishCoalescing {
                        left: Box::new(initializer),
                        left_span: initializer_span,
                        right: Box::new(default_value),
                        right_span: Some(text_span_from_oxc_span(default_span)),
                    })
                }
                None => None,
            };
            parse_binding_pattern_declarations_with_definite(
                &assignment_pattern.left,
                initializer,
                initializer_span,
                is_declare,
                kind,
                declared_type,
                has_definite_assertion,
            )
        }
        BindingPattern::ObjectPattern(object_pattern) => {
            mark_binding_pattern_declarations(parse_object_pattern_declarations(
                object_pattern,
                initializer,
                initializer_span,
                is_declare,
                kind,
            ))
        }
        BindingPattern::ArrayPattern(array_pattern) => {
            let pattern_span = text_span_from_oxc_span(array_pattern.span);
            let declarations = parse_array_pattern_declarations(
                array_pattern,
                initializer,
                initializer_span,
                is_declare,
                kind,
            )
            .into_iter()
            .map(|statement| match statement {
                ParsedStatement::VariableDeclaration(mut variable)
                    if variable.array_pattern_span.is_none() =>
                {
                    variable.array_pattern_span = Some(pattern_span);
                    ParsedStatement::VariableDeclaration(variable)
                }
                other => other,
            })
            .collect();
            mark_binding_pattern_declarations(declarations)
        }
    }
}

fn is_empty_binding_pattern(binding: &BindingPattern<'_>) -> bool {
    match binding {
        BindingPattern::ObjectPattern(pattern) => pattern.properties.is_empty() && pattern.rest.is_none(),
        BindingPattern::ArrayPattern(pattern) => {
            pattern.elements.iter().all(Option::is_none) && pattern.rest.is_none()
        }
        _ => false,
    }
}

/// Flags every binding produced by a destructuring pattern, including the ones
/// nested inside it.
fn mark_binding_pattern_declarations(statements: Vec<ParsedStatement>) -> Vec<ParsedStatement> {
    statements
        .into_iter()
        .map(|statement| match statement {
            ParsedStatement::VariableDeclaration(mut variable) => {
                variable.from_binding_pattern = true;
                ParsedStatement::VariableDeclaration(variable)
            }
            other => other,
        })
        .collect()
}

fn parse_object_pattern_declarations(
    object_pattern: &ObjectPattern<'_>,
    initializer: Option<ParsedExpression>,
    initializer_span: Option<crate::TextSpan>,
    is_declare: bool,
    kind: ParsedVariableKind,
) -> Vec<ParsedStatement> {
    let Some(initializer) = initializer else {
        return Vec::new();
    };

    let mut declarations = Vec::new();

    for property in &object_pattern.properties {
        declarations.extend(parse_object_binding_property_declarations(
            property,
            initializer.clone(),
            initializer_span,
            is_declare,
            kind,
        ));
    }

    // `const { a, ...rest } = obj` binds `rest` to the remaining properties;
    // a computed key omits what its type admits.
    if let Some(rest) = object_pattern.rest.as_deref() {
        let mut computed = Vec::new();
        let omitted: Option<Vec<String>> = object_pattern
            .properties
            .iter()
            .filter_map(|property| match binding_property_key(&property.key) {
                Some((name, _)) => Some(Some(name)),
                None => match property.key.as_expression() {
                    Some(key) => {
                        computed.push(parse_expression(key).0);
                        None
                    }
                    None => Some(None),
                },
            })
            .collect();
        let initializer = match omitted {
            Some(omitted) => ParsedExpression::ObjectRest {
                source: Box::new(initializer),
                omitted,
                computed,
                name_span: match &rest.argument {
                    BindingPattern::BindingIdentifier(identifier) => {
                        Some(text_span_from_oxc_span(identifier.span))
                    }
                    _ => None,
                },
            },
            None => initializer,
        };
        declarations.extend(parse_binding_pattern_declarations(
            &rest.argument,
            Some(initializer),
            initializer_span,
            is_declare,
            kind,
            None,
        ));
    }

    declarations
}

fn parse_object_binding_property_declarations(
    property: &BindingProperty<'_>,
    source_initializer: ParsedExpression,
    source_initializer_span: Option<crate::TextSpan>,
    is_declare: bool,
    kind: ParsedVariableKind,
) -> Vec<ParsedStatement> {
    let Some((property_name, key_span)) = binding_property_key(&property.key) else {
        // A computed key's element reads the source indexed by the key.
        let Some(key) = property.key.as_expression() else {
            return Vec::new();
        };
        let key_span = Some(text_span_from_oxc_span(oxc_span::GetSpan::span(&property.key)));
        return parse_binding_pattern_declarations(
            &property.value,
            Some(ParsedExpression::ElementAccess {
                object: Box::new(source_initializer),
                object_span: source_initializer_span,
                index: Box::new(parse_expression(key).0),
                index_span: key_span,
            }),
            key_span,
            is_declare,
            kind,
            None,
        );
    };

    // An object literal initializer is contextually typed by the pattern, whose
    // defaulted elements are optional properties (`getTypeFromBindingPattern`),
    // so a defaulted name the literal does not write reads `undefined` rather
    // than a missing property.
    if matches!(property.value, BindingPattern::AssignmentPattern(_))
        && static_object_literal(&source_initializer).is_some_and(|properties| {
            literal_lacks_property(properties, &property_name)
        })
    {
        return parse_binding_pattern_declarations(
            &property.value,
            Some(ParsedExpression::UndefinedLiteral),
            Some(text_span_from_oxc_span(key_span)),
            is_declare,
            kind,
            None,
        );
    }

    // tsc indexes the source by the key's literal type
    // (`getLiteralTypeFromPropertyName`); a numeric name stays an element access,
    // as `obj["0"]` does, so a tuple or array source answers it.
    let is_numeric_index = !property_name.is_empty() && property_name.bytes().all(|byte| byte.is_ascii_digit());
    let property_initializer = if is_numeric_index {
        let index = Box::new(match &property.key {
            PropertyKey::NumericLiteral(_) => ParsedExpression::NumberLiteral(property_name),
            _ => ParsedExpression::StringLiteral(property_name),
        });
        let index_span = Some(text_span_from_oxc_span(key_span));
        match source_initializer {
            ParsedExpression::Identifier { name, .. } => ParsedExpression::IndexAccess {
                object_name: name,
                object_span: source_initializer_span,
                index,
                index_span,
            },
            source => ParsedExpression::ElementAccess {
                object: Box::new(source),
                object_span: source_initializer_span,
                index,
                index_span,
            },
        }
    } else {
        // Marked bracketed: this access is synthesized from a binding pattern,
        // and tsc does not apply `noPropertyAccessFromIndexSignature` (TS4111)
        // to destructuring — only to written dotted accesses.
        ParsedExpression::PropertyAccess {
            object: Box::new(source_initializer),
            object_span: source_initializer_span,
            property_name,
            property_span: Some(text_span_from_oxc_span(key_span)),
            is_bracketed: true,
            binding_element: true,
        }
    };

    parse_binding_pattern_declarations(
        &property.value,
        Some(property_initializer),
        Some(text_span_from_oxc_span(key_span)),
        is_declare,
        kind,
        None,
    )
}

/// A numeric name read from `source`. An array literal source is
/// contextually a tuple under a pattern with numeric names (tsc's
/// `isTupleLikeType`), so the name takes its own element.
fn numeric_name_read(
    source: ParsedExpression,
    source_span: Option<crate::TextSpan>,
    name: &str,
    span: Option<crate::TextSpan>,
) -> ParsedExpression {
    if let ParsedExpression::ArrayLiteral { elements, .. } = &source
        && let Ok(index) = name.parse::<usize>()
        && index < elements.len()
        && elements.iter().take(index + 1).all(|element| !element.spread)
    {
        return elements[index].expression.clone();
    }
    ParsedExpression::ElementAccess {
        object: Box::new(source),
        object_span: source_span,
        index: Box::new(ParsedExpression::NumberLiteral(name.to_string())),
        index_span: span,
    }
}

/// tsc's `isNumericLiteralName`: the name is the canonical text of a number.
fn is_numeric_literal_name(name: &str) -> bool {
    name.parse::<f64>()
        .is_ok_and(|value| number_text::js_number_to_string(value) == name)
}

/// The property a non-computed binding property reads. A quoted or numeric key
/// names the property its value spells, as it does in an object literal
/// (`{ 0: first }` reads `"0"`).
pub(crate) fn binding_property_key(key: &PropertyKey<'_>) -> Option<(String, oxc_span::Span)> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some((identifier.name.to_string(), identifier.span)),
        PropertyKey::StringLiteral(literal) => Some((literal.value.to_string(), literal.span)),
        PropertyKey::NumericLiteral(literal) => {
            Some((number_text::js_number_to_string(literal.value), literal.span))
        }
        _ => None,
    }
}

/// The properties of the object literal `expression` statically evaluates to:
/// the literal itself, a property of one whose value is a literal, or the
/// default an absent value falls back to.
fn static_object_literal(expression: &ParsedExpression) -> Option<&[crate::ParsedObjectProperty]> {
    match expression {
        ParsedExpression::ObjectLiteral { properties, .. } => Some(properties),
        ParsedExpression::ConstAssertion { expression, .. } => static_object_literal(expression),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } => {
            let properties = static_object_literal(object)?;
            let property = properties
                .iter()
                .find(|property| property.name == *property_name && !property.is_spread)?;
            static_object_literal(&property.value)
        }
        ParsedExpression::NullishCoalescing { left, right, .. } => match left.as_ref() {
            ParsedExpression::UndefinedLiteral => static_object_literal(right),
            left => static_object_literal(left),
        },
        _ => None,
    }
}

/// The elements of the array literal `expression` statically evaluates to, as
/// [`static_object_literal`] finds an object literal.
fn static_array_literal(expression: &ParsedExpression) -> Option<&[crate::ParsedArrayElement]> {
    match expression {
        ParsedExpression::ArrayLiteral { elements, .. } => Some(elements),
        ParsedExpression::ConstAssertion { expression, .. } => static_array_literal(expression),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } => {
            let properties = static_object_literal(object)?;
            let property = properties
                .iter()
                .find(|property| property.name == *property_name && !property.is_spread)?;
            static_array_literal(&property.value)
        }
        ParsedExpression::NullishCoalescing { left, right, .. } => match left.as_ref() {
            ParsedExpression::UndefinedLiteral => static_array_literal(right),
            left => static_array_literal(left),
        },
        _ => None,
    }
}

/// Whether a literal certainly does not write `name`: a spread or a computed key
/// could supply it.
fn literal_lacks_property(properties: &[crate::ParsedObjectProperty], name: &str) -> bool {
    properties
        .iter()
        .all(|property| !property.is_spread && property.computed_key.is_none() && property.name != name)
}

fn parse_array_pattern_declarations(
    array_pattern: &ArrayPattern<'_>,
    initializer: Option<ParsedExpression>,
    initializer_span: Option<crate::TextSpan>,
    is_declare: bool,
    kind: ParsedVariableKind,
) -> Vec<ParsedStatement> {
    let Some(initializer) = initializer else {
        return Vec::new();
    };

    let mut declarations = Vec::new();

    for (index, element) in array_pattern.elements.iter().enumerate() {
        let Some(element) = element else {
            continue;
        };

        // tsc reports an element the source lacks on the binding element itself.
        let element_span = Some(text_span_from_oxc_span(oxc_span::GetSpan::span(element)));
        // An array literal initializer is contextually typed by the pattern as
        // a tuple (`getTypeFromArrayBindingPattern`), so each element reads its
        // own entry rather than the union of all of them.
        let literal_element = static_array_literal(&initializer).and_then(|elements| {
            (index < elements.len() && elements.iter().take(index + 1).all(|element| !element.spread))
                .then(|| elements[index].expression.clone())
        });
        let element_initializer = literal_element.unwrap_or_else(|| match &initializer {
            ParsedExpression::Identifier { name, .. } => ParsedExpression::IndexAccess {
                object_name: name.clone(),
                object_span: initializer_span,
                index: Box::new(ParsedExpression::NumberLiteral(index.to_string())),
                index_span: element_span,
            },
            // A non-identifier initializer (`const [a, b] = useState()`) indexes
            // the source expression directly so each binding gets its own element
            // type (e.g. the `Dispatch` setter), instead of the whole source.
            _ => ParsedExpression::ElementAccess {
                object: Box::new(initializer.clone()),
                object_span: initializer_span,
                index: Box::new(ParsedExpression::NumberLiteral(index.to_string())),
                index_span: element_span,
            },
        });

        declarations.extend(parse_binding_pattern_declarations(
            element,
            Some(element_initializer),
            initializer_span,
            is_declare,
            kind,
            None,
        ));
    }

    // `const [a, ...rest] = xs` binds `rest` to the remaining elements. Bound to
    // the whole initializer, the same shape the object-pattern sibling uses: for
    // an array source that is already the right element type, and a tuple
    // source is sliced when the binding is checked (`array_rest_start`). A
    // literal typed as a tuple is sliced here, as tsc slices a tuple source
    // (`sliceTupleType`).
    if let Some(rest) = array_pattern.rest.as_deref() {
        let (rest_initializer, sliced) = match &initializer {
            ParsedExpression::ArrayLiteral {
                elements,
                span,
                tuple_context: true,
            } if elements.iter().all(|element| !element.spread) => (
                ParsedExpression::ArrayLiteral {
                    elements: elements.iter().skip(array_pattern.elements.len()).cloned().collect(),
                    span: *span,
                    tuple_context: true,
                },
                true,
            ),
            _ => (initializer.clone(), false),
        };
        let mut rest_declarations = parse_binding_pattern_declarations(
            &rest.argument,
            Some(rest_initializer),
            initializer_span,
            is_declare,
            kind,
            None,
        );
        if !sliced && matches!(rest.argument, BindingPattern::BindingIdentifier(_)) {
            for statement in &mut rest_declarations {
                if let ParsedStatement::VariableDeclaration(variable) = statement {
                    variable.array_rest_start = Some(array_pattern.elements.len());
                }
            }
        }
        declarations.extend(rest_declarations);
    }

    declarations
}

/// tsc's `getContextualTypeForInitializerExpression`: an unannotated
/// declaration's initializer is contextually typed by the type its binding
/// pattern implies (`getTypeFromBindingPattern`) — a tuple for an array
/// pattern, an object with the pattern's keys for an object pattern — and an
/// array literal whose contextual type is tuple-like (a tuple, or a type with a
/// `"0"` property: `isTupleLikeType`) is a tuple (`checkArrayLiteral`). The
/// context reaches a nested literal through the element or property a nested
/// pattern destructures.
fn with_pattern_context(pattern: &BindingPattern<'_>, initializer: ParsedExpression) -> ParsedExpression {
    let take = |expression: &mut ParsedExpression| std::mem::replace(expression, ParsedExpression::Unknown);
    match (pattern, initializer) {
        (
            BindingPattern::ArrayPattern(array),
            ParsedExpression::ArrayLiteral { mut elements, span, .. },
        ) if !array.elements.is_empty() || array.rest.is_some() => {
            for (element, nested) in elements.iter_mut().zip(&array.elements) {
                if element.spread {
                    break;
                }
                if let Some(nested) = nested {
                    element.expression = with_element_context(nested, take(&mut element.expression));
                }
            }
            ParsedExpression::ArrayLiteral {
                elements,
                span,
                tuple_context: true,
            }
        }
        (
            BindingPattern::ObjectPattern(object),
            ParsedExpression::ArrayLiteral {
                mut elements,
                span,
                tuple_context,
            },
        ) => {
            let property_at = |index: usize| {
                let key = index.to_string();
                object
                    .properties
                    .iter()
                    .find(|property| binding_property_key(&property.key).is_some_and(|(name, _)| name == key))
            };
            if property_at(0).is_none() {
                return ParsedExpression::ArrayLiteral {
                    elements,
                    span,
                    tuple_context,
                };
            }
            for (index, element) in elements.iter_mut().enumerate() {
                if element.spread {
                    break;
                }
                if let Some(property) = property_at(index) {
                    element.expression = with_element_context(&property.value, take(&mut element.expression));
                }
            }
            ParsedExpression::ArrayLiteral {
                elements,
                span,
                tuple_context: true,
            }
        }
        (BindingPattern::ObjectPattern(object), ParsedExpression::ObjectLiteral { mut properties, span }) => {
            for property in &mut properties {
                if property.is_spread || property.computed_key.is_some() {
                    continue;
                }
                if let Some(pattern_property) = object.properties.iter().find(|pattern_property| {
                    binding_property_key(&pattern_property.key).is_some_and(|(name, _)| name == property.name)
                }) {
                    property.value = with_element_context(&pattern_property.value, take(&mut property.value));
                }
            }
            ParsedExpression::ObjectLiteral { properties, span }
        }
        // `getContextualTypeForConditionalOperand`: each branch is typed by the
        // whole expression's context.
        (
            pattern @ (BindingPattern::ArrayPattern(_) | BindingPattern::ObjectPattern(_)),
            ParsedExpression::Conditional {
                condition,
                condition_span,
                mut when_true,
                when_true_span,
                mut when_false,
                when_false_span,
                truthiness_tests,
            },
        ) => {
            *when_true = with_pattern_context(pattern, take(&mut *when_true));
            *when_false = with_pattern_context(pattern, take(&mut *when_false));
            ParsedExpression::Conditional {
                condition,
                condition_span,
                when_true,
                when_true_span,
                when_false,
                when_false_span,
                truthiness_tests,
            }
        }
        (_, initializer) => initializer,
    }
}

/// The context a pattern element gives the value it destructures
/// (`getTypeFromBindingElement`): a nested pattern's own, or with a default, the
/// default's type — which mirrors the nested pattern only when the default is
/// itself a literal.
fn with_element_context(element: &BindingPattern<'_>, value: ParsedExpression) -> ParsedExpression {
    match element {
        BindingPattern::AssignmentPattern(assignment) => {
            if matches!(
                &assignment.right,
                Expression::ArrayExpression(_) | Expression::ObjectExpression(_)
            ) {
                with_pattern_context(&assignment.left, value)
            } else {
                value
            }
        }
        pattern => with_pattern_context(pattern, value),
    }
}

/// tsc's excess-property check against the type a binding pattern implies
/// (`checkObjectLiteral`, contextually typed by
/// `getContextualTypeForInitializerExpression`): the members of the
/// initializer's literals the pattern does not name. A pattern without
/// elements gives its initializer no contextual type.
fn pattern_excess_properties(
    pattern: &BindingPattern<'_>,
    initializer: &ParsedExpression,
) -> Vec<crate::ParsedPatternExcessProperty> {
    let has_elements = match pattern {
        BindingPattern::ObjectPattern(object) => !object.properties.is_empty() || object.rest.is_some(),
        BindingPattern::ArrayPattern(array) => !array.elements.is_empty() || array.rest.is_some(),
        _ => false,
    };
    let mut excess = Vec::new();
    if has_elements
        && matches!(
            initializer,
            ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
        )
    {
        collect_pattern_excess_properties(&functions::parse_binding_name(pattern), initializer, &mut excess);
    }
    excess
}

/// A literal answers to the pattern it initializes. An object pattern with a
/// rest element implies a string index, and one with a computed key leaves its
/// members open (`ObjectLiteralPatternWithComputedProperties`): neither has an
/// excess member. A nested literal answers to the nested pattern unless the
/// element has a default, whose own type is then the context; a spread literal
/// shares the context of the literal it spreads into.
fn collect_pattern_excess_properties(
    pattern: &crate::ParsedBindingName,
    literal: &ParsedExpression,
    excess: &mut Vec<crate::ParsedPatternExcessProperty>,
) {
    match (pattern, literal) {
        (crate::ParsedBindingName::ObjectPattern(object), ParsedExpression::ObjectLiteral { properties, .. }) => {
            let closed = object.rest.is_none()
                && !object
                    .elements
                    .iter()
                    .any(|element| matches!(element.binding_name, crate::ParsedBindingName::Unsupported { .. }));
            for property in properties {
                if property.is_spread {
                    collect_pattern_excess_properties(pattern, &property.value, excess);
                    continue;
                }
                if property.is_accessor || property.computed_key.is_some() {
                    continue;
                }
                match object.elements.iter().find(|element| element.property_name == property.name) {
                    Some(element) if !element.has_default => {
                        collect_pattern_excess_properties(&element.binding_name, &property.value, excess);
                    }
                    Some(_) => {}
                    None if closed => excess.push(crate::ParsedPatternExcessProperty {
                        pattern: pattern.clone(),
                        name: property.name.clone(),
                        name_span: property.name_span.or(property.span),
                        typed_targets: false,
                    }),
                    None => {}
                }
            }
        }
        (crate::ParsedBindingName::ArrayPattern(array), ParsedExpression::ArrayLiteral { elements, .. }) => {
            for (index, element) in elements.iter().enumerate() {
                if element.spread {
                    break;
                }
                if let Some(Some(nested)) = array.elements.get(index)
                    && !array.defaults.get(index).copied().unwrap_or(false)
                {
                    collect_pattern_excess_properties(nested, &element.expression, excess);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn parse_ts_module_declaration(
    module: &TSModuleDeclaration<'_>,
) -> Vec<ParsedStatement> {
    use oxc_span::GetSpan;
    let module_specifier = match &module.id {
        TSModuleDeclarationName::StringLiteral(literal) => literal.value.to_string(),
        // `namespace JSX { ... }` / `module Foo { ... }` (identifier-named). The body
        // is preserved so the checker can resolve qualified members such as
        // `JSX.IntrinsicElements`.
        TSModuleDeclarationName::Identifier(identifier) => {
            return parse_ts_namespace_declaration(
                identifier.name.to_string(),
                Some(text_span_from_oxc_span(identifier.span)),
                module,
            );
        }
    };

    let statements = match &module.body {
        Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
            let mut statements: Vec<ParsedStatement> = block
                .body
                .iter()
                .filter_map(parse_statement)
                .flatten()
                .collect();
            enums::merge_lowered_enum_declarations(&mut statements);
            statements
        }
        None => Vec::new(),
        _ => {
            return vec![ParsedStatement::UnsupportedDeclaration {
                span: Some(text_span_from_oxc_span(module.span)),
            }];
        }
    };

    let mut statements = statements;
    mark_ambient_namespaces(&mut statements);
    enums::merge_namespace_block_enums(&mut statements);
    vec![ParsedStatement::DeclareModuleDeclaration(Box::new(
        ParsedDeclareModuleDeclaration {
            module_specifier,
            module_specifier_span: Some(text_span_from_oxc_span(module.id.span())),
            statements,
            span: Some(text_span_from_oxc_span(module.span)),
            is_shorthand: module.body.is_none(),
        },
    ))]
}

/// A namespace inside an ambient module or `declare global` block is ambient
/// like one written `declare namespace`: its declarations are exported
/// without `export`.
fn mark_ambient_namespaces(statements: &mut [ParsedStatement]) {
    for statement in statements {
        match statement {
            ParsedStatement::NamespaceDeclaration(namespace) => {
                namespace.is_declare = true;
                mark_ambient_namespaces(&mut namespace.statements);
            }
            ParsedStatement::ExportDeclaration(export) => {
                if let crate::ParsedExportDeclaration::Statement { declaration, .. } = export.as_mut() {
                    mark_ambient_namespaces(std::slice::from_mut(declaration.as_mut()));
                }
            }
            _ => {}
        }
    }
}

fn parse_ts_namespace_declaration(
    name: String,
    name_span: Option<crate::TextSpan>,
    module: &TSModuleDeclaration<'_>,
) -> Vec<ParsedStatement> {
    let statements = match &module.body {
        Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
            let mut statements: Vec<ParsedStatement> = block
                .body
                .iter()
                .filter_map(parse_statement)
                .flatten()
                .collect();
            enums::merge_lowered_enum_declarations(&mut statements);
            statements
        }
        // `namespace A.B { ... }` nests as a module body; flatten it into a
        // dotted-name namespace so members resolve as `A.B.Member`.
        Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
            let inner_name = match &inner.id {
                TSModuleDeclarationName::Identifier(identifier) => identifier.name.to_string(),
                TSModuleDeclarationName::StringLiteral(literal) => literal.value.to_string(),
            };
            let mut nested =
                parse_ts_namespace_declaration(format!("{name}.{inner_name}"), name_span, inner);
            if module.declare {
                for statement in &mut nested {
                    if let ParsedStatement::NamespaceDeclaration(namespace) = statement {
                        namespace.is_declare = true;
                    }
                }
            }
            nested
        }
        None => Vec::new(),
    };

    vec![ParsedStatement::NamespaceDeclaration(Box::new(
        ParsedNamespaceDeclaration {
            name,
            is_declare: module.declare,
            name_span,
            statements,
            span: Some(text_span_from_oxc_span(module.span)),
        },
    ))]
}

fn parse_ts_global_declaration(global: &TSGlobalDeclaration<'_>) -> Vec<ParsedStatement> {
    let mut statements: Vec<ParsedStatement> = global
        .body
        .body
        .iter()
        .filter_map(parse_statement)
        .flatten()
        .collect();
    enums::merge_lowered_enum_declarations(&mut statements);
    mark_ambient_namespaces(&mut statements);
    enums::merge_namespace_block_enums(&mut statements);

    vec![ParsedStatement::DeclareModuleDeclaration(Box::new(
        ParsedDeclareModuleDeclaration {
            module_specifier: "global".to_string(),
            module_specifier_span: Some(text_span_from_oxc_span(global.global_span)),
            statements,
            span: Some(text_span_from_oxc_span(global.span)),
            is_shorthand: false,
        },
    ))]
}

/// `Symbol(…)` or `Symbol.for(…)` (tsc's `isSymbolOrSymbolForCall`).
fn is_symbol_constructor_call(expression: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expression else {
        return false;
    };
    match call.callee.without_parentheses() {
        Expression::Identifier(callee) => callee.name == "Symbol",
        Expression::StaticMemberExpression(member) => {
            member.property.name == "for"
                && matches!(&member.object, Expression::Identifier(object) if object.name == "Symbol")
        }
        _ => false,
    }
}
