//! CommonJS in JavaScript files, as typescript-go binds it: a `require`
//! call reads a module (`isCommonJSRequire`), a variable initialized to one
//! is an alias of it (`IsVariableDeclarationInitializedToRequire`),
//! `module.exports = e` is the module's `export =` and `exports.x = e` one of
//! its exports (`GetAssignmentDeclarationKind`), and a file doing either is a
//! module with `module` and `exports` in scope (`declareCommonJSVariable`).

use oxc_ast::ast::{
    Argument, AssignmentTarget, BindingPattern, CallExpression, Declaration, Expression,
    ExpressionStatement, Program, Statement, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;

use super::spans::text_span_from_oxc_span;
use crate::{
    ParsedExportDeclaration, ParsedExportSpecifier, ParsedExpression, ParsedImportDeclaration,
    ParsedImportKind, ParsedImportSpecifier, ParsedObjectType, ParsedObjectTypeProperty,
    ParsedStatement, ParsedType, ParsedTypeOfType, ParsedVariableDeclaration, ParsedVariableKind,
};

#[derive(Clone, Default)]
pub(crate) struct CommonJs {
    /// `CommonJSModuleIndicator`: the file is a CommonJS module.
    pub(crate) module: bool,
    /// The file declares a `require` of its own, so a call of it is a call.
    require_declared: bool,
    /// The file assigns `module.exports`, whose value is then the export.
    exports_assigned: bool,
    /// The `exports.x` names every top-level assignment of which is
    /// `undefined` or `null`: their type is an implicit `any`
    /// (`getWidenedTypeForAssignmentDeclaration`).
    nullable_exports: std::rc::Rc<std::collections::HashSet<String>>,
    /// Every `exports.x`/`module.exports.x` name assigned or defined
    /// anywhere in the file, which the binder declares wherever it is written.
    export_names: std::rc::Rc<Vec<String>>,
    /// An export is assigned more than one non-nullable value. tsc types
    /// it by all of them and narrows each read by the last write, where surge
    /// types it by its first; `exports` then stays `any` in the file.
    reassigned_export: bool,
    /// Per `exports.x` written more than once, the union of what every write
    /// assigns, where each one names its type (see [`assigned_value_type`]).
    written_unions: std::rc::Rc<std::collections::HashMap<String, ParsedType>>,
}

thread_local! {
    static COMMONJS: std::cell::RefCell<Option<CommonJs>> = const { std::cell::RefCell::new(None) };
    /// What lowering the module finds wrong: an implicitly `any` export.
    static FINDINGS: std::cell::RefCell<Vec<crate::ParsedGrammarDiagnostic>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The `exports.x` names already declared: a later assignment to one is
    /// a write to the export, not a second declaration of it.
    static EXPORTED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

pub(crate) fn with_commonjs<R>(commonjs: Option<CommonJs>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<CommonJs>, std::collections::HashSet<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            COMMONJS.with(|slot| *slot.borrow_mut() = self.0.take());
            EXPORTED.with(|exported| *exported.borrow_mut() = std::mem::take(&mut self.1));
        }
    }
    let _restore = Restore(
        COMMONJS.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), commonjs)),
        EXPORTED.with(|exported| std::mem::take(&mut *exported.borrow_mut())),
    );
    f()
}

/// The findings lowering the module reported, taken.
pub(crate) fn take_findings() -> Vec<crate::ParsedGrammarDiagnostic> {
    FINDINGS.with(|findings| std::mem::take(&mut *findings.borrow_mut()))
}

impl CommonJs {
    pub(crate) fn exports_assigned(&self) -> bool {
        self.exports_assigned
    }
}

/// Whether the file being lowered is a CommonJS module.
pub(crate) fn is_commonjs_module() -> bool {
    current().is_some_and(|commonjs| commonjs.module)
}

fn current() -> Option<CommonJs> {
    COMMONJS.with(|slot| slot.borrow().clone())
}

/// Scan a JavaScript program for what makes it a CommonJS module.
pub(crate) fn scan(program: &Program<'_>) -> CommonJs {
    let has_module_syntax = program.body.iter().any(|statement| statement.is_module_declaration());
    let require_declared = program.body.iter().any(|statement| match statement {
        Statement::FunctionDeclaration(function) => {
            function.id.as_ref().is_some_and(|id| id.name == "require") && !function.declare
        }
        Statement::VariableDeclaration(declaration) => {
            !declaration.declare
                && declaration.declarations.iter().any(|declarator| {
                    matches!(&declarator.id, BindingPattern::BindingIdentifier(id) if id.name == "require")
                })
        }
        _ => false,
    });
    let mut scanner = IndicatorScanner {
        indicator: false,
        exports_assigned: false,
        export_assignment: None,
        exports_property: false,
        export_names: Vec::new(),
        nullable: std::collections::HashMap::new(),
        valued: std::collections::HashSet::new(),
        reassigned: false,
        writes: std::collections::HashMap::new(),
    };
    scanner.visit_program(program);
    let written_unions = written_unions(program, &scanner.writes);
    // tsc's `checkExternalModuleExports`: the binder declares every
    // `module.exports = e` as the file's `export=` and every exports property
    // beside it, wherever they are written.
    if !has_module_syntax
        && scanner.exports_property
        && let Some(span) = scanner.export_assignment
    {
        FINDINGS.with(|findings| {
            findings.borrow_mut().push(crate::ParsedGrammarDiagnostic {
                kind: crate::ParsedGrammarDiagnosticKind::Ts(2309),
                span: text_span_from_oxc_span(span),
                name: None,
            })
        });
    }
    CommonJs {
        module: scanner.indicator && !has_module_syntax,
        require_declared,
        exports_assigned: scanner.exports_assigned && !has_module_syntax,
        nullable_exports: std::rc::Rc::new(
            scanner.nullable.into_iter().filter(|(_, nullable)| *nullable).map(|(name, _)| name).collect(),
        ),
        export_names: std::rc::Rc::new(scanner.export_names),
        reassigned_export: scanner.reassigned,
        written_unions: std::rc::Rc::new(written_unions),
    }
}

/// `getWidenedTypeForAssignmentDeclaration`: an export written more than once
/// is typed by the union of every write, an initial `undefined` aside. Only
/// an export written solely at the top level, each time with a value
/// [`assigned_value_type`] names, is typed this way; any other keeps the type
/// of the write that declares it.
fn written_unions(
    program: &Program<'_>,
    writes: &std::collections::HashMap<String, usize>,
) -> std::collections::HashMap<String, ParsedType> {
    let functions: std::collections::HashSet<&str> = program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::FunctionDeclaration(function) if function.body.is_some() => {
                function.id.as_ref().map(|id| id.name.as_str())
            }
            _ => None,
        })
        .collect();
    let mut assigned: std::collections::HashMap<String, Vec<Option<ParsedType>>> =
        std::collections::HashMap::new();
    for statement in &program.body {
        let Statement::ExpressionStatement(statement) = statement else {
            continue;
        };
        let (names, Some(value)) = export_property_chain(&statement.expression) else {
            continue;
        };
        let ty = assigned_value_type(value, &functions);
        for (name, _) in names {
            assigned.entry(name).or_default().push(ty.clone());
        }
    }
    assigned
        .into_iter()
        .filter(|(name, types)| types.len() > 1 && writes.get(name) == Some(&types.len()))
        .filter_map(|(name, types)| {
            let mut members: Vec<ParsedType> = Vec::new();
            for (index, ty) in types.into_iter().enumerate() {
                let ty = ty?;
                if index == 0 && matches!(ty, ParsedType::Undefined) {
                    continue;
                }
                if !members.contains(&ty) {
                    members.push(ty);
                }
            }
            (members.len() > 1).then(|| (name, ParsedType::Union(std::sync::Arc::new(members))))
        })
        .collect()
}

/// The type a write assigns (`getRegularTypeOfLiteralType` of the value),
/// for a value that names it unchecked: a literal, `undefined`, `null`, a
/// `void` expression, or a top-level function declaration (`typeof f`).
fn assigned_value_type(value: &Expression<'_>, functions: &std::collections::HashSet<&str>) -> Option<ParsedType> {
    match value {
        Expression::Identifier(identifier) if identifier.name == "undefined" => Some(ParsedType::Undefined),
        Expression::Identifier(identifier) if functions.contains(identifier.name.as_str()) => {
            Some(ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
                name: identifier.name.to_string(),
                name_span: None,
                members: Vec::new(),
                import_specifier: None,
                member_spans: Vec::new(),
                type_arguments: Vec::new(),
                type_arguments_span: None,
            })))
        }
        Expression::NullLiteral(_) => Some(ParsedType::Null),
        Expression::UnaryExpression(unary) if unary.operator == oxc_syntax::operator::UnaryOperator::Void => {
            Some(ParsedType::Undefined)
        }
        Expression::StringLiteral(literal) => Some(ParsedType::StringLiteral(literal.value.to_string())),
        Expression::NumericLiteral(literal) => {
            Some(ParsedType::NumberLiteral(super::number_text::js_number_to_string(literal.value)))
        }
        Expression::BooleanLiteral(literal) => Some(ParsedType::BooleanLiteral(literal.value)),
        _ => None,
    }
}

/// `exports.a = exports.b = v`: the names it assigns, and `v`.
fn export_property_chain<'e, 'a>(
    expression: &'e Expression<'a>,
) -> (Vec<(String, oxc_span::Span)>, Option<&'e Expression<'a>>) {
    let mut names = Vec::new();
    let mut value = expression;
    while let Expression::AssignmentExpression(assignment) = value {
        if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
            break;
        }
        match export_target(&assignment.left) {
            Some(ExportTarget::Property(name)) => {
                names.push((name, assignment.left.span()));
                value = &assignment.right;
            }
            _ => break,
        }
    }
    if names.is_empty() || matches!(value, Expression::AssignmentExpression(_)) {
        return (Vec::new(), None);
    }
    (names, Some(value))
}

/// A value whose type is nothing but `undefined` or `null`.
/// `null`, `undefined` or a `void` expression: a value whose widened type is
/// nullable alone.
pub(crate) fn is_nullish_value(value: &Expression<'_>) -> bool {
    is_nullable_value(value)
}

fn is_nullable_value(value: &Expression<'_>) -> bool {
    match value.without_parentheses() {
        Expression::Identifier(identifier) => identifier.name == "undefined",
        Expression::NullLiteral(_) => true,
        Expression::UnaryExpression(unary) => {
            unary.operator == oxc_syntax::operator::UnaryOperator::Void
        }
        _ => false,
    }
}

struct IndicatorScanner {
    indicator: bool,
    exports_assigned: bool,
    /// The first `module.exports = e`, the file's `export=` declaration.
    export_assignment: Option<oxc_span::Span>,
    /// An `exports.x`/`module.exports.x` assignment or an
    /// `Object.defineProperty(exports, …)` declared a value export.
    exports_property: bool,
    export_names: Vec<String>,
    /// Per exported name, whether every value assigned to it is nullable.
    nullable: std::collections::HashMap<String, bool>,
    valued: std::collections::HashSet<String>,
    reassigned: bool,
    /// Per exported name, how many writes declare it anywhere in the file.
    writes: std::collections::HashMap<String, usize>,
}

impl IndicatorScanner {
    fn note_export(&mut self, name: String, value_nullable: bool) {
        if !self.export_names.contains(&name) {
            self.export_names.push(name.clone());
        }
        *self.writes.entry(name.clone()).or_default() += 1;
        if !value_nullable && !self.valued.insert(name.clone()) {
            self.reassigned = true;
        }
        *self.nullable.entry(name).or_insert(true) &= value_nullable;
    }
}

impl<'a> Visit<'a> for IndicatorScanner {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if is_require_call(it, false) {
            self.indicator = true;
        }
        if is_define_property_of_exports(it) {
            self.indicator = true;
            self.exports_property = true;
            if let Some(Argument::StringLiteral(name)) = it.arguments.get(1) {
                self.note_export(name.value.to_string(), false);
            }
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &oxc_ast::ast::AssignmentExpression<'a>) {
        if it.operator == oxc_syntax::operator::AssignmentOperator::Assign {
            match export_target(&it.left) {
                Some(ExportTarget::ModuleExports) if !is_exports_identifier(&it.right) => {
                    self.indicator = true;
                    self.exports_assigned = true;
                    self.export_assignment.get_or_insert(it.span);
                }
                Some(ExportTarget::Property(name)) => {
                    self.indicator = true;
                    self.exports_property = true;
                    let mut value = &it.right;
                    while let Expression::AssignmentExpression(inner) = value
                        && inner.operator == oxc_syntax::operator::AssignmentOperator::Assign
                    {
                        value = &inner.right;
                    }
                    self.note_export(name, is_nullable_value(value));
                }
                _ => {}
            }
        }
        walk::walk_assignment_expression(self, it);
    }
}

/// `IsRequireCall`: `require` called with one argument, a string literal when
/// `literal`.
fn is_require_call(call: &CallExpression<'_>, literal: bool) -> bool {
    matches!(&call.callee, Expression::Identifier(callee) if callee.name == "require")
        && call.arguments.len() == 1
        && (!literal || require_specifier(call).is_some())
}

fn require_specifier<'c>(call: &'c CallExpression<'_>) -> Option<(&'c str, oxc_span::Span)> {
    match call.arguments.first()? {
        Argument::StringLiteral(literal) => Some((literal.value.as_str(), literal.span)),
        Argument::TemplateLiteral(template) if template.expressions.is_empty() => {
            let quasi = template.quasis.first()?;
            Some((quasi.value.cooked.as_deref().unwrap_or(quasi.value.raw.as_str()), template.span))
        }
        _ => None,
    }
}

fn is_exports_identifier(expression: &Expression<'_>) -> bool {
    matches!(expression, Expression::Identifier(identifier) if identifier.name == "exports")
}

/// `module.exports`.
fn is_module_exports(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::StaticMemberExpression(member) => {
            member.property.name == "exports"
                && matches!(&member.object, Expression::Identifier(object) if object.name == "module")
        }
        Expression::ComputedMemberExpression(member) => {
            matches!(&member.expression, Expression::StringLiteral(key) if key.value == "exports")
                && matches!(&member.object, Expression::Identifier(object) if object.name == "module")
        }
        _ => false,
    }
}

fn is_define_property_of_exports(call: &CallExpression<'_>) -> bool {
    let Expression::StaticMemberExpression(callee) = &call.callee else {
        return false;
    };
    call.arguments.len() == 3
        && callee.property.name == "defineProperty"
        && matches!(&callee.object, Expression::Identifier(object) if object.name == "Object")
        && call.arguments.first().and_then(Argument::as_expression).is_some_and(|target| {
            is_exports_identifier(target) || is_module_exports(target)
        })
}

enum ExportTarget {
    /// `module.exports = e`
    ModuleExports,
    /// `exports.x = e`, `module.exports.x = e`
    Property(String),
}

/// Whether `target` is `module.exports` or a property of `exports` /
/// `module.exports`, which the CommonJS lowering declares.
pub(crate) fn is_export_target(target: &AssignmentTarget<'_>) -> bool {
    export_target(target).is_some()
}

fn export_target(target: &AssignmentTarget<'_>) -> Option<ExportTarget> {
    let (object, name) = match target {
        AssignmentTarget::StaticMemberExpression(member) => {
            (&member.object, member.property.name.to_string())
        }
        AssignmentTarget::ComputedMemberExpression(member) => match &member.expression {
            Expression::StringLiteral(key) => (&member.object, key.value.to_string()),
            _ => return None,
        },
        _ => return None,
    };
    if name == "exports" && matches!(object, Expression::Identifier(object) if object.name == "module") {
        return Some(ExportTarget::ModuleExports);
    }
    if is_exports_identifier(object) || is_module_exports(object) {
        return Some(ExportTarget::Property(name));
    }
    None
}

/// The module a `require("m")` call reads, as the value `typeof import("m")`
/// describes. A `require` of anything but a literal is untyped.
pub(crate) fn require_call_expression(call: &CallExpression<'_>) -> Option<ParsedExpression> {
    if !super::spans::lowering_javascript() || !is_require_call(call, false) {
        return None;
    }
    if current().is_some_and(|commonjs| commonjs.require_declared) {
        return None;
    }
    let ty = match require_specifier(call) {
        Some((specifier, span)) => ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
            name: format!("import(\"{specifier}\")"),
            name_span: Some(text_span_from_oxc_span(span)),
            members: Vec::new(),
            import_specifier: Some(specifier.to_string()),
            member_spans: Vec::new(),
            type_arguments: Vec::new(),
            type_arguments_span: None,
        })),
        None => ParsedType::Any,
    };
    let argument = call.arguments.first().and_then(Argument::as_expression);
    let (expression, expression_span) = match argument.filter(|_| matches!(ty, ParsedType::Any)) {
        Some(argument) => {
            let (expression, span) = super::expressions::parse_expression(argument);
            (expression, Some(text_span_from_oxc_span(span)))
        }
        None => (ParsedExpression::Unknown, None),
    };
    Some(ParsedExpression::TypeAssertion {
        expression: Box::new(expression),
        expression_span,
        ty,
        type_span: None,
        annotation: true,
    })
}

/// The starts of the `require` calls [`require_imports`] turns into import
/// declarations (every one when the file declares a `require` of its own, so
/// that none is read as a module): the top-level variable statements whose
/// every declaration is initialized to one.
pub(crate) fn top_level_require_declaration_calls(program: &Program<'_>) -> std::collections::HashSet<u32> {
    let declares_require = program.body.iter().any(|statement| match statement {
        Statement::FunctionDeclaration(function) => {
            function.id.as_ref().is_some_and(|id| id.name == "require") && !function.declare
        }
        Statement::VariableDeclaration(declaration) => declaration.declarations.iter().any(|declarator| {
            matches!(&declarator.id, BindingPattern::BindingIdentifier(id) if id.name == "require")
        }),
        _ => false,
    });
    let mut calls = std::collections::HashSet::new();
    for statement in &program.body {
        let Statement::VariableDeclaration(declaration) = statement else {
            continue;
        };
        let requires: Option<Vec<u32>> = declaration
            .declarations
            .iter()
            .map(|declarator| match &declarator.init {
                Some(Expression::CallExpression(call))
                    if declarator.type_annotation.is_none() && is_require_call(call, true) =>
                {
                    Some(call.span.start)
                }
                _ => None,
            })
            .collect();
        calls.extend(requires.unwrap_or_default());
    }
    if declares_require {
        struct AllRequires(std::collections::HashSet<u32>);
        impl<'a> Visit<'a> for AllRequires {
            fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
                if is_require_call(it, true) {
                    self.0.insert(it.span.start);
                }
                walk::walk_call_expression(self, it);
            }
        }
        let mut all = AllRequires(calls);
        all.visit_program(program);
        return all.0;
    }
    calls
}

/// A top-level variable statement whose every declaration is initialized to
/// a `require`: aliases of the modules, as `import x = require("m")` and
/// `import { a } from "m"` declare them.
pub(crate) fn require_imports(declaration: &VariableDeclaration<'_>) -> Option<Vec<ParsedStatement>> {
    if !super::spans::lowering_javascript() || current().is_none_or(|commonjs| commonjs.require_declared) {
        return None;
    }
    let mut imports = Vec::new();
    for declarator in &declaration.declarations {
        // `isVariableDeclarationInitializedToRequire` asks for no type node,
        // and the reparser makes a `@type` tag one: the declaration is then a
        // variable checked against its initializer, not an alias.
        if declarator.type_annotation.is_some()
            || super::jsdoc::declared_type_at(declarator.span.start).is_some()
        {
            return None;
        }
        let Some(Expression::CallExpression(call)) = &declarator.init else {
            return None;
        };
        if !is_require_call(call, true) {
            return None;
        }
        let (specifier, specifier_span) = require_specifier(call)?;
        let kind = match &declarator.id {
            BindingPattern::BindingIdentifier(identifier) => ParsedImportKind::Equals {
                local_name: identifier.name.to_string(),
                name_span: Some(text_span_from_oxc_span(identifier.span)),
                is_type_only: false,
                exported: false,
            },
            BindingPattern::ObjectPattern(pattern) => {
                if pattern.rest.is_some() {
                    return None;
                }
                let mut specifiers = Vec::new();
                for property in &pattern.properties {
                    let BindingPattern::BindingIdentifier(local) = &property.value else {
                        return None;
                    };
                    let imported = property.key.static_name()?;
                    if property.computed {
                        return None;
                    }
                    specifiers.push(ParsedImportSpecifier {
                        imported_name: imported.to_string(),
                        local_name: local.name.to_string(),
                        name_span: Some(text_span_from_oxc_span(local.span)),
                    });
                }
                ParsedImportKind::Named { is_type_only: false, specifiers }
            }
            _ => return None,
        };
        imports.push(ParsedStatement::ImportDeclaration(Box::new(ParsedImportDeclaration {
            kind,
            module_specifier: specifier.to_string(),
            module_specifier_span: Some(text_span_from_oxc_span(specifier_span)),
            span: Some(text_span_from_oxc_span(declarator.span)),
            resolution_mode: None,
            inline_type_specifiers: false,
        })));
    }
    Some(imports)
}

/// A top-level `module.exports = e` or `exports.x = e` in a CommonJS module:
/// the export it declares. `exports.x` names an export only while
/// `module.exports` is not replaced.
pub(crate) fn export_statement(statement: &ExpressionStatement<'_>) -> Option<Vec<ParsedStatement>> {
    let commonjs = current().filter(|commonjs| commonjs.module)?;
    if let Expression::CallExpression(call) = &statement.expression {
        return (!commonjs.exports_assigned)
            .then(|| define_property_export(call, statement.span))
            .flatten();
    }
    let Expression::AssignmentExpression(assignment) = &statement.expression else {
        return None;
    };
    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
        return None;
    }
    match export_target(&assignment.left)? {
        ExportTarget::ModuleExports => {
            module_exports_declaration(assignment, statement.span).map(|export| vec![export])
        }
        ExportTarget::Property(_) if !commonjs.exports_assigned => {
            let (names, Some(value)) = export_property_chain(&statement.expression) else {
                return None;
            };
            let nullable = is_nullable_value(value);
            // A name assigned something else later is declared there; an
            // initial `undefined` does not type it.
            let declared: Vec<(String, oxc_span::Span)> = names
                .into_iter()
                .filter(|(name, _)| !nullable || commonjs.nullable_exports.contains(name))
                .filter(|(name, _)| EXPORTED.with(|exported| exported.borrow_mut().insert(name.clone())))
                .collect();
            if declared.is_empty() {
                return None;
            }
            let (value, value_span) = super::expressions::parse_expression(value);
            let value_span = Some(text_span_from_oxc_span(value_span));
            let mut statements = Vec::new();
            for (name, name_span) in declared {
                if nullable {
                    FINDINGS.with(|findings| {
                        findings.borrow_mut().push(crate::ParsedGrammarDiagnostic {
                            kind: crate::ParsedGrammarDiagnosticKind::ImplicitAnyMember,
                            span: text_span_from_oxc_span(statement.span),
                            name: Some(name.clone()),
                        })
                    });
                }
                let declared_type = if nullable {
                    Some(ParsedType::Any)
                } else {
                    commonjs.written_unions.get(&name).cloned()
                };
                statements.extend(declared_export(
                    name,
                    name_span,
                    value.clone(),
                    value_span,
                    statement.span,
                    declared_type,
                ));
            }
            Some(statements)
        }
        ExportTarget::Property(_) => None,
    }
}

/// `module.exports = e`: the module's `export =`, the first one the top level
/// writes. `module.exports = exports` declares nothing.
fn module_exports_declaration(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
    statement_span: oxc_span::Span,
) -> Option<ParsedStatement> {
    if is_exports_identifier(&assignment.right)
        || !EXPORTED.with(|exported| exported.borrow_mut().insert("\u{0}module.exports".to_string()))
    {
        return None;
    }
    let span = Some(text_span_from_oxc_span(statement_span));
    let export = match &assignment.right {
        Expression::Identifier(identifier) => ParsedExportDeclaration::Equals {
            exported_name: identifier.name.to_string(),
            exported_name_span: Some(text_span_from_oxc_span(identifier.span)),
            span,
        },
        right => {
            let (expression, expression_span) = super::expressions::parse_expression(right);
            ParsedExportDeclaration::EqualsExpression {
                expression: Box::new(expression),
                expression_span: Some(text_span_from_oxc_span(expression_span)),
                entity_name: None,
                span,
            }
        }
    };
    Some(ParsedStatement::ExportDeclaration(Box::new(export)))
}

/// A top-level statement that replaces `module.exports` inside an assignment
/// chain (`var log = module.exports = new EE()`, `exports = module.exports =
/// C`). The binder declares every `module.exports = e` as the module's
/// `export =` wherever it is written (`bindModuleExportsAssignment`), beside
/// what the statement itself declares.
pub(crate) fn chained_module_exports_declaration(statement: &Statement<'_>) -> Option<ParsedStatement> {
    current().filter(|commonjs| commonjs.module)?;
    let (link, span) = match statement {
        Statement::VariableDeclaration(declaration) => (
            declaration
                .declarations
                .iter()
                .filter_map(|declarator| declarator.init.as_ref())
                .find_map(|init| module_exports_link(init))?,
            declaration.span,
        ),
        // `module.exports = e` itself is `export_statement`'s.
        Statement::ExpressionStatement(expression_statement) => match &expression_statement.expression {
            Expression::AssignmentExpression(assignment)
                if assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign
                    && !matches!(export_target(&assignment.left), Some(ExportTarget::ModuleExports)) =>
            {
                (module_exports_link(&assignment.right)?, expression_statement.span)
            }
            _ => return None,
        },
        _ => return None,
    };
    module_exports_declaration(link, span)
}

/// The `module.exports = e` link of an assignment chain `a = b = … = e`.
fn module_exports_link<'e, 'a>(
    mut value: &'e Expression<'a>,
) -> Option<&'e oxc_ast::ast::AssignmentExpression<'a>> {
    while let Expression::AssignmentExpression(assignment) = value.without_parentheses() {
        if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
            return None;
        }
        if matches!(export_target(&assignment.left), Some(ExportTarget::ModuleExports)) {
            return Some(&**assignment);
        }
        value = &assignment.right;
    }
    None
}

/// A `module.exports = e` or `exports.x = e` whose value is read
/// (`var log = module.exports = new EE()`): the assignment's value is `e`'s,
/// which takes no contextual type from a CommonJS target
/// (`getContextualTypeForBinaryOperand`). Kept apart from a plain `e` so the
/// variable it initializes is no expando (`IsExpandoInitializer`).
pub(crate) fn export_assignment_value(
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Option<ParsedExpression> {
    if !super::spans::lowering_javascript()
        || assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign
        || current().is_none_or(|commonjs| !commonjs.module)
        || export_target(&assignment.left).is_none()
    {
        return None;
    }
    let (value, value_span) = super::expressions::parse_expression(&assignment.right);
    Some(ParsedExpression::Sequence {
        expressions: vec![(value, Some(text_span_from_oxc_span(value_span)))],
    })
}

/// `Object.defineProperty(exports, "name", descriptor)`: an export typed by
/// the descriptor's `value` (`getTypeFromPropertyDescriptor`), which an
/// accessor descriptor leaves untyped here.
fn define_property_export(call: &CallExpression<'_>, statement_span: oxc_span::Span) -> Option<Vec<ParsedStatement>> {
    use oxc_ast::ast::ObjectPropertyKind;
    if !is_define_property_of_exports(call) {
        return None;
    }
    let Some(Argument::StringLiteral(name)) = call.arguments.get(1) else {
        return None;
    };
    let descriptor_argument = call.arguments.get(2)?.as_expression()?;
    if !EXPORTED.with(|exported| exported.borrow_mut().insert(name.value.to_string())) {
        return None;
    }
    let Expression::ObjectExpression(descriptor) = descriptor_argument.without_parentheses() else {
        // tsc binds the export whatever the descriptor is and reads its type
        // from the descriptor's (`getTypeFromPropertyDescriptor`); only a
        // literal is read syntactically here.
        let (descriptor_expression, descriptor_span) = super::expressions::parse_expression(descriptor_argument);
        let mut statements = declared_export(
            name.value.to_string(),
            name.span,
            ParsedExpression::Unknown,
            Some(text_span_from_oxc_span(descriptor_span)),
            statement_span,
            Some(ParsedType::Any),
        );
        if let Some(ParsedStatement::VariableDeclaration(variable)) = statements.first_mut() {
            variable.initializer = None;
        }
        statements.push(ParsedStatement::Expression(Box::new(descriptor_expression)));
        return Some(statements);
    };
    let member = |key: &str| {
        descriptor.properties.iter().find_map(|property| match property {
            ObjectPropertyKind::ObjectProperty(property) if property.key.static_name().as_deref() == Some(key) => {
                Some(property.as_ref())
            }
            _ => None,
        })
    };
    let (value, writable, get, set) = (member("value"), member("writable"), member("get"), member("set"));
    // `isReadonlyAssignmentDeclaration`: a `value` is read-only unless
    // `writable` is there and not the literal `false`; an accessor pair
    // without a `set` is read-only.
    let readonly = match value {
        Some(_) => writable.is_none_or(|writable| matches!(&writable.value, Expression::BooleanLiteral(literal) if !literal.value)),
        None => set.is_none(),
    };
    // The call's argument is contextually typed by `defineProperty`'s
    // `PropertyDescriptor & ThisType<any>` parameter, which is what types its
    // accessors' parameters, and their `this` as `any`.
    let (descriptor_expression, descriptor_span) = super::expressions::parse_expression(
        call.arguments[2].as_expression().expect("an object literal is an expression"),
    );
    let descriptor_span = Some(text_span_from_oxc_span(descriptor_span));
    let named = |name: &str, type_arguments: Vec<ParsedType>| {
        ParsedType::Named(std::sync::Arc::new(crate::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments,
        }))
    };
    let descriptor_expression = ParsedExpression::SatisfiesExpression {
        expression: Box::new(descriptor_expression),
        target_type: ParsedType::Intersection(std::sync::Arc::new(vec![
            named("PropertyDescriptor", Vec::new()),
            named("ThisType", vec![ParsedType::Any]),
        ])),
        span: descriptor_span,
        target_span: None,
        keyword_span: None,
    };
    // `getTypeFromPropertyDescriptor`: the descriptor's `value`, else its
    // getter's return type, else its setter's parameter type, else `any`.
    let member_read = |property_name: &str, call: bool| {
        if call {
            ParsedExpression::PropertyCall {
                object: Box::new(descriptor_expression.clone()),
                object_span: descriptor_span,
                property_name: property_name.to_string(),
                property_span: None,
                call_span: descriptor_span,
                type_arguments: Vec::new(),
                arguments: Vec::new(),
            }
        } else {
            ParsedExpression::PropertyAccess {
                object: Box::new(descriptor_expression.clone()),
                object_span: descriptor_span,
                property_name: property_name.to_string(),
                property_span: None,
                is_bracketed: false,
                binding_element: false,
            }
        }
    };
    let setter_parameter_type = || {
        let Expression::FunctionExpression(function) = &set?.value else {
            return None;
        };
        let parameter = function.params.items.first()?;
        parameter
            .type_annotation
            .as_ref()
            .and_then(|annotation| super::types::parse_type_annotation(annotation))
            .or_else(|| super::jsdoc::parameter_at(parameter.span.start).and_then(|jsdoc| jsdoc.ty.map(|(ty, _)| ty)))
    };
    let (initializer, declared_type) = if value.is_some() {
        (Some(member_read("value", false)), None)
    } else if get.is_some() {
        (Some(member_read("get", true)), None)
    } else if set.is_some() {
        (None, Some(setter_parameter_type().unwrap_or(ParsedType::Any)))
    } else {
        (Some(descriptor_expression.clone()), Some(ParsedType::Any))
    };
    let mut statements = declared_export(
        name.value.to_string(),
        name.span,
        initializer.clone().unwrap_or(ParsedExpression::Unknown),
        descriptor_span,
        statement_span,
        declared_type,
    );
    if let Some(ParsedStatement::VariableDeclaration(variable)) = statements.first_mut() {
        variable.initializer = initializer;
        if readonly {
            variable.kind = ParsedVariableKind::Const;
        }
    }
    if matches!(statements.first(), Some(ParsedStatement::VariableDeclaration(variable)) if variable.initializer.is_none()) {
        statements.push(ParsedStatement::Expression(Box::new(descriptor_expression)));
    }
    Some(statements)
}

/// `exports.name = value`: a module-local variable no source name can reach,
/// exported as `name`.
fn declared_export(
    name: String,
    name_span: oxc_span::Span,
    value: ParsedExpression,
    value_span: Option<crate::TextSpan>,
    statement_span: oxc_span::Span,
    declared_type: Option<ParsedType>,
) -> Vec<ParsedStatement> {
    let local_name = format!("\u{0}exports.{name}");
    let name_span = Some(text_span_from_oxc_span(name_span));
    // `getRegularTypeOfLiteralType`: an assigned literal keeps its literal
    // type, which a writable binding does only for an assertion's.
    let value = match value {
        ParsedExpression::StringLiteral(_)
        | ParsedExpression::NumberLiteral(_)
        | ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::BigIntLiteral(_) => ParsedExpression::ConstAssertion {
            expression: Box::new(value),
            span: value_span,
        },
        value => value,
    };
    let statement_span = Some(text_span_from_oxc_span(statement_span));
    vec![
                ParsedStatement::VariableDeclaration(Box::new(ParsedVariableDeclaration {
                    is_declare: false,
                    kind: ParsedVariableKind::Var,
                    from_binding_pattern: false,
                    has_definite_assertion: false,
                    array_pattern_span: None,
                    is_enum_object: false,
                    enum_members: None,
                    array_rest_start: None,
                    name: local_name.clone(),
                    name_span,
                    declared_type,
                    initializer: Some(value),
                    initializer_span: value_span,
                    declaration_list: None,
                    annotated_pattern: None,
                    pattern_excess_properties: Vec::new(),
                })),
                ParsedStatement::ExportDeclaration(Box::new(ParsedExportDeclaration::Named {
                    is_type_only: false,
                    specifiers: vec![ParsedExportSpecifier {
                        local_name,
                        exported_name: name,
                        name_span,
                        exported_name_span: name_span,
                        is_type_only: false,
                    }],
                    module_specifier: None,
                    module_specifier_span: None,
                    span: statement_span,
                    resolution_mode: None,
                })),
    ]
}

/// `declareCommonJSVariable`: `module` and `exports` in a CommonJS module
/// that does not declare them itself, with the specifier of the file's own
/// module, which types them (`resolveExternalModuleSymbol`). The exports
/// written below the top level are declared here as well, `any`-typed.
///
/// A `module.exports = e` the lowering does not declare (one written inside
/// a function) or a reassigned export leaves both `any`: the module surge
/// would see is not the one tsc does.
pub(crate) fn module_variables(
    program: &Program<'_>,
    commonjs: &CommonJs,
    file_name: &str,
) -> (Vec<ParsedStatement>, Option<String>) {
    if !commonjs.module {
        return (Vec::new(), None);
    }
    let replaced = EXPORTED.with(|exported| exported.borrow().contains("\u{0}module.exports"));
    let own_specifier = (replaced || !(commonjs.exports_assigned || commonjs.reassigned_export))
        .then(|| std::path::Path::new(file_name).file_name()?.to_str())
        .flatten()
        .map(|name| format!("./{name}"));
    let declares = |name: &str| {
        program.body.iter().any(|statement| match statement {
            Statement::VariableDeclaration(declaration) => declaration.declarations.iter().any(|declarator| {
                matches!(&declarator.id, BindingPattern::BindingIdentifier(id) if id.name == name)
            }),
            Statement::FunctionDeclaration(function) => function.id.as_ref().is_some_and(|id| id.name == name),
            Statement::ClassDeclaration(class) => class.id.as_ref().is_some_and(|id| id.name == name),
            _ => statement
                .as_declaration()
                .is_some_and(|declaration| matches!(declaration, Declaration::TSEnumDeclaration(e) if e.id.name == name)),
        })
    };
    let mut variables: Vec<ParsedStatement> = ["module", "exports"]
        .into_iter()
        .filter(|name| !declares(name))
        .map(|name| {
            let declared_type = match &own_specifier {
                Some(specifier) => own_module_variable_type(name, specifier),
                None => ParsedType::Any,
            };
            ParsedStatement::VariableDeclaration(Box::new(ParsedVariableDeclaration {
                is_declare: true,
                kind: ParsedVariableKind::Var,
                from_binding_pattern: false,
                has_definite_assertion: false,
                array_pattern_span: None,
                is_enum_object: false,
                enum_members: None,
                array_rest_start: None,
                name: name.to_string(),
                name_span: None,
                declared_type: Some(declared_type),
                initializer: None,
                initializer_span: None,
                declaration_list: None,
                annotated_pattern: None,
                pattern_excess_properties: Vec::new(),
            }))
        })
        .collect();
    if !commonjs.exports_assigned {
        for name in commonjs.export_names.iter() {
            if EXPORTED.with(|exported| exported.borrow_mut().insert(name.clone())) {
                variables.extend(nested_export(name));
            }
        }
    }
    (variables, own_specifier)
}

/// An export only a nested write declares: `any`, where tsc unions the types
/// of all its writes (`getWidenedTypeForAssignmentDeclaration`).
fn nested_export(name: &str) -> Vec<ParsedStatement> {
    let local_name = format!("\u{0}exports.{name}");
    vec![
        ParsedStatement::VariableDeclaration(Box::new(ParsedVariableDeclaration {
            is_declare: true,
            kind: ParsedVariableKind::Var,
            from_binding_pattern: false,
            has_definite_assertion: false,
            array_pattern_span: None,
            is_enum_object: false,
            enum_members: None,
            array_rest_start: None,
            name: local_name.clone(),
            name_span: None,
            declared_type: Some(ParsedType::Any),
            initializer: None,
            initializer_span: None,
            declaration_list: None,
            annotated_pattern: None,
            pattern_excess_properties: Vec::new(),
        })),
        ParsedStatement::ExportDeclaration(Box::new(ParsedExportDeclaration::Named {
            is_type_only: false,
            specifiers: vec![ParsedExportSpecifier {
                local_name,
                exported_name: name.to_string(),
                name_span: None,
                exported_name_span: None,
                is_type_only: false,
            }],
            module_specifier: None,
            module_specifier_span: None,
            span: None,
            resolution_mode: None,
        })),
    ]
}

/// Whether the write `target = …` declares an export (`GetAssignmentDeclarationKind`):
/// `module.exports` itself, or a member of `exports`/`module.exports` while
/// the module is not replaced. The checker leaves such a write unchecked.
pub(crate) fn declares_export(target: &AssignmentTarget<'_>) -> bool {
    let Some(commonjs) = current().filter(|commonjs| commonjs.module) else {
        return false;
    };
    match export_target(target) {
        Some(ExportTarget::ModuleExports) => true,
        Some(ExportTarget::Property(_)) => !commonjs.exports_assigned,
        None => false,
    }
}

/// `exports` is the module itself, `typeof import("./self")`, and `module`
/// the object holding it (`{ exports: typeof import("./self") }`).
fn own_module_variable_type(name: &str, specifier: &str) -> ParsedType {
    let module = ParsedType::TypeOf(std::sync::Arc::new(ParsedTypeOfType {
        name: format!("import(\"{specifier}\")"),
        name_span: None,
        members: Vec::new(),
        import_specifier: Some(specifier.to_string()),
        member_spans: Vec::new(),
        type_arguments: Vec::new(),
        type_arguments_span: None,
    }));
    if name == "exports" {
        return module;
    }
    ParsedType::Object(std::sync::Arc::new(ParsedObjectType {
        properties: vec![ParsedObjectTypeProperty {
            name: "exports".to_string(),
            name_span: None,
            ty: module,
            optional: false,
            is_method: false,
            readonly: false,
            write_ty: None,
        }],
        string_index_type: None,
        number_index_type: None,
        string_index_readonly: false,
        number_index_readonly: false,
        call_signature: None,
        call_signature_overloads: Vec::new(),
        construct_signature: None,
        construct_signature_overloads: Vec::new(),
        non_primitive: false,
        display_name: None,
        abstract_construct_signature: false,
        keyed_indexes: None,
        number_index_span: None,
    }))
}
