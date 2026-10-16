//! tsc's `--isolatedDeclarations` errors (TS9007–TS9038), which its
//! declaration emitter (`transformers/declarations`) reports wherever a
//! declaration file would need a type the checker inferred. Whether a type is
//! inferrable without the checker is the `pseudochecker`'s syntactic verdict:
//! literals, object and array literals in their allowed forms, functions whose
//! single `return` is inferrable, and written annotations.
//!
//! tsc reports these through `Program.Emit`, even with `noEmit`, whatever
//! other errors the program has.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrayExpressionElement, BindingPattern, Class, ClassElement, Declaration, ExportDefaultDeclarationKind,
    Expression, FormalParameter, FormalParameters, Function, FunctionBody, MethodDefinitionKind,
    ObjectPropertyKind, Program, PropertyKey, PropertyKind, Statement, TSModuleDeclaration,
    TSModuleDeclarationBody, TSType, TSTypeName, UnaryOperator, VariableDeclaration, VariableDeclarationKind,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

/// One isolated-declarations error: its code and where tsc anchors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsolatedDeclarationDiagnostic {
    pub code: u32,
    pub start: u32,
    pub end: u32,
}

const FUNCTION_RETURN: u32 = 9007;
const METHOD_RETURN: u32 = 9008;
const ACCESSOR: u32 = 9009;
const VARIABLE: u32 = 9010;
const PARAMETER: u32 = 9011;
const PROPERTY: u32 = 9012;
const EXPRESSION: u32 = 9013;
const SPREAD_ASSIGNMENT: u32 = 9015;
const SHORTHAND: u32 = 9016;
const ARRAY_LITERAL: u32 = 9017;
const SPREAD_ELEMENT: u32 = 9018;
const BINDING_ELEMENT: u32 = 9019;
const ENUM_MEMBER: u32 = 9020;
const EXTENDS_EXPRESSION: u32 = 9021;
const CLASS_EXPRESSION: u32 = 9022;
const EXPANDO: u32 = 9023;
const IMPLICIT_UNDEFINED: u32 = 9025;
const DEFAULT_EXPORT: u32 = 9037;
const COMPUTED_NAME: u32 = 9038;

/// The errors a TypeScript source file's declaration emit reports under
/// `isolatedDeclarations`.
pub fn isolated_declaration_diagnostics(
    source_text: &str,
    file_name: &str,
    strict_null_checks: bool,
) -> Vec<IsolatedDeclarationDiagnostic> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source_text, source_type).parse();
    let mut walker = Walker {
        source_text,
        strict_null_checks,
        out: Vec::new(),
        exported_names: exported_local_names(&parsed.program),
        overloaded: Vec::new(),
        functions: Vec::new(),
    };
    let module = super::grammar_context::is_external_module(&parsed.program);
    if module {
        walker.module_statements(&parsed.program.body);
    } else {
        walker.statements(&parsed.program.body, true);
    }
    walker.expando_assignments(&parsed.program.body);
    let mut out = walker.out;
    out.sort_by_key(|diagnostic| (diagnostic.start, diagnostic.code));
    out.dedup();
    out
}

/// What decides whether a file's declaration emit keeps an import only for
/// the module augmentations it brings in (`isImportRequiredByAugmentation`).
#[derive(Debug, Default)]
pub struct AugmentationFacts {
    /// Imports with bindings, none of which the file's declarations name, so
    /// emit elides them.
    pub elided_imports: Vec<ElidedImport>,
    pub exported_names: Vec<String>,
    /// `declare module "<specifier>" { … }` at the top level, with the names
    /// it declares.
    pub augmentations: Vec<(String, Vec<String>)>,
}

#[derive(Debug)]
pub struct ElidedImport {
    pub specifier: String,
    pub start: u32,
    pub end: u32,
}

pub fn augmentation_facts(source_text: &str, file_name: &str) -> AugmentationFacts {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source_text, source_type).parse();
    let mut facts = AugmentationFacts::default();
    if !super::grammar_context::is_external_module(&parsed.program) {
        return facts;
    }
    // Over-collecting only keeps imports emit would elide, which misses a
    // report rather than inventing one.
    let mut emitted = Vec::new();
    for statement in &parsed.program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                if let Some(declaration) = &export.declaration {
                    facts.exported_names.extend(declared_names(declaration));
                    if let Declaration::TSInterfaceDeclaration(interface) = declaration {
                        facts.exported_names.push(interface.id.name.to_string());
                    }
                    if let Declaration::TSTypeAliasDeclaration(alias) = declaration {
                        facts.exported_names.push(alias.id.name.to_string());
                    }
                }
                for specifier in &export.specifiers {
                    facts.exported_names.push(specifier.exported.name().to_string());
                }
                collect_referenced_names(statement, &mut emitted);
            }
            Statement::ExportDefaultDeclaration(_) | Statement::TSExportAssignment(_) => {
                collect_referenced_names(statement, &mut emitted);
            }
            Statement::TSModuleDeclaration(module) if module.declare => {
                if let oxc_ast::ast::TSModuleDeclarationName::StringLiteral(name) = &module.id
                    && let Some(TSModuleDeclarationBody::TSModuleBlock(block)) = &module.body
                {
                    let names = block
                        .body
                        .iter()
                        .filter_map(|statement| match statement {
                            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                            other => other.as_declaration(),
                        })
                        .flat_map(|declaration| match declaration {
                            Declaration::TSInterfaceDeclaration(interface) => vec![interface.id.name.to_string()],
                            Declaration::TSTypeAliasDeclaration(alias) => vec![alias.id.name.to_string()],
                            other => declared_names(other),
                        })
                        .collect();
                    facts.augmentations.push((name.value.to_string(), names));
                }
                collect_referenced_names(statement, &mut emitted);
            }
            other => collect_referenced_names(other, &mut emitted),
        }
    }
    for statement in &parsed.program.body {
        if let Statement::ImportDeclaration(import) = statement
            && let Some(specifiers) = &import.specifiers
            && !specifiers.is_empty()
            && !specifiers.iter().any(|specifier| emitted.iter().any(|name| name == specifier.local().name.as_str()))
        {
            facts.elided_imports.push(ElidedImport {
                specifier: import.source.value.to_string(),
                start: import.span.start,
                end: import.span.end,
            });
        }
    }
    facts
}

/// Every name a statement's emitted parts reference: identifiers and type
/// names anywhere outside function bodies.
fn collect_referenced_names(statement: &Statement<'_>, names: &mut Vec<String>) {
    use oxc_ast_visit::Visit;
    struct Collector<'n>(&'n mut Vec<String>);
    impl<'a> Visit<'a> for Collector<'_> {
        fn visit_function_body(&mut self, _: &FunctionBody<'a>) {}
        fn visit_identifier_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'a>) {
            self.0.push(identifier.name.to_string());
        }
    }
    if matches!(statement, Statement::ExpressionStatement(_)) {
        return;
    }
    Collector(names).visit_statement(statement);
}

/// The local names an `export { a, b as c }` without a module specifier, or
/// an `export default a`, makes visible.
fn exported_local_names(program: &Program<'_>) -> Vec<String> {
    let mut names = Vec::new();
    for statement in &program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) if export.source.is_none() => {
                for specifier in &export.specifiers {
                    names.push(specifier.local.name().to_string());
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let Some(Expression::Identifier(identifier)) = export.declaration.as_expression() {
                    names.push(identifier.name.to_string());
                }
            }
            Statement::TSExportAssignment(assignment) => {
                if let Expression::Identifier(identifier) = &assignment.expression {
                    names.push(identifier.name.to_string());
                }
            }
            _ => {}
        }
    }
    names
}

#[derive(Clone, Copy)]
struct Report {
    code: u32,
    span: Span,
}

/// The pseudochecker's verdict on a type (`PseudoType`), keeping only what
/// decides the errors.
enum Pseudo {
    /// A written annotation, reused as it is.
    Direct,
    /// A primitive, a literal, `undefined` or `null`.
    Plain,
    /// A type only the checker knows: reported at its error nodes, or where
    /// its expression is when it has none.
    Inferred { errors: Vec<Report>, fallback: Report },
    /// Nothing written to read a type from.
    NoResult(Report),
    MaybeConst { const_type: Box<Pseudo>, regular: Box<Pseudo>, const_context: bool },
    /// A function, a tuple or an object literal: its parts are reported.
    Composite(Vec<Pseudo>),
}

impl Pseudo {
    fn inferred(fallback: Report) -> Self {
        Pseudo::Inferred { errors: Vec::new(), fallback }
    }

    /// `PseudoTypeInferred` without error nodes, which a declaration turns
    /// into its own `NoResult`.
    fn is_bare_inferred(&self) -> bool {
        matches!(self, Pseudo::Inferred { errors, .. } if errors.is_empty())
    }
}

/// Where an expression sits, which picks the code an error on it gets
/// (`createExpressionErrorEx`): the nearest declaration tsc names
/// (`findNearestDeclaration`), and whether the expression is that
/// declaration's own value rather than a part of it.
#[derive(Clone, Copy)]
struct Site {
    declaration: Option<u32>,
    direct: bool,
    /// `IsInConstContext`: under an `as const`, through array and object
    /// literals only.
    const_context: bool,
}

impl Site {
    fn of(declaration: u32) -> Self {
        Site { declaration: Some(declaration), direct: true, const_context: false }
    }

    fn detached() -> Self {
        Site { declaration: None, direct: false, const_context: false }
    }

    fn nested(self) -> Self {
        Site { declaration: self.declaration, direct: false, const_context: self.const_context }
    }

    fn expression_code(self) -> u32 {
        match self.declaration {
            Some(code) if self.direct => code,
            _ => EXPRESSION,
        }
    }
}

struct Walker<'s> {
    source_text: &'s str,
    strict_null_checks: bool,
    out: Vec<IsolatedDeclarationDiagnostic>,
    exported_names: Vec<String>,
    /// Function names of the statement list being walked that have bodiless
    /// overload signatures, whose implementation is not emitted.
    overloaded: Vec<String>,
    /// Visible function declarations, which an assignment to one of their
    /// properties turns into an expando (TS9023).
    functions: Vec<String>,
}

impl Walker<'_> {
    fn push(&mut self, report: Report) {
        self.out.push(IsolatedDeclarationDiagnostic {
            code: report.code,
            start: report.span.start,
            end: report.span.end,
        });
    }

    /// `pseudoTypeToNodeWithCheckerFallback`: a declaration's own type.
    fn report_declaration_type(&mut self, pseudo: Pseudo) {
        match pseudo {
            Pseudo::Inferred { errors, fallback } => {
                if errors.is_empty() {
                    self.push(fallback);
                } else {
                    errors.into_iter().for_each(|report| self.push(report));
                }
            }
            Pseudo::Direct => {}
            other => self.report_nested(other),
        }
    }

    /// `pseudoTypeToNode`: the parts of a type it has to write out.
    fn report_nested(&mut self, pseudo: Pseudo) {
        match pseudo {
            Pseudo::Direct | Pseudo::Plain => {}
            Pseudo::Inferred { errors, fallback } => {
                if errors.is_empty() {
                    self.push(fallback);
                } else {
                    errors.into_iter().for_each(|report| self.push(report));
                }
            }
            Pseudo::NoResult(report) => self.push(report),
            Pseudo::MaybeConst { const_type, regular, const_context } => {
                self.report_nested(if const_context { *const_type } else { *regular })
            }
            Pseudo::Composite(members) => {
                members.into_iter().for_each(|member| self.report_nested(member))
            }
        }
    }

    fn statements(&mut self, statements: &[Statement<'_>], all_visible: bool) {
        let outer = std::mem::replace(&mut self.overloaded, overloaded_function_names(statements));
        for statement in statements {
            self.statement(statement, all_visible);
        }
        self.overloaded = outer;
    }

    /// A module's top level, where a declaration that is not exported is still
    /// emitted once a visible one names it in a type query or a computed key
    /// (`isLateVisibilityPaintedStatement`), and is then checked like an
    /// exported one.
    fn module_statements(&mut self, statements: &[Statement<'_>]) {
        let outer = std::mem::replace(&mut self.overloaded, overloaded_function_names(statements));
        let mut painted = vec![false; statements.len()];
        let mut referenced = Vec::new();
        for (index, statement) in statements.iter().enumerate() {
            let visible = matches!(
                statement,
                Statement::ExportNamedDeclaration(_)
                    | Statement::ExportDefaultDeclaration(_)
                    | Statement::TSExportAssignment(_)
            ) || statement.as_declaration().is_some_and(|declaration| self.declares_exported_name(declaration));
            if visible {
                painted[index] = true;
                self.statement(statement, false);
                collect_emitted_entity_names(statement, &mut referenced);
            }
        }
        loop {
            let mut changed = false;
            for (index, statement) in statements.iter().enumerate() {
                if painted[index] {
                    continue;
                }
                let Some(declaration) = statement.as_declaration() else {
                    continue;
                };
                if !declared_names(declaration).iter().any(|name| referenced.contains(name)) {
                    continue;
                }
                painted[index] = true;
                changed = true;
                self.declaration(declaration, true);
                collect_emitted_entity_names(statement, &mut referenced);
            }
            if !changed {
                break;
            }
        }
        self.overloaded = outer;
    }

    fn statement(&mut self, statement: &Statement<'_>, all_visible: bool) {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                if let Some(declaration) = &export.declaration {
                    self.declaration(declaration, true);
                }
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => self.function_declaration(function),
                ExportDefaultDeclarationKind::ClassDeclaration(class) => self.class(class),
                ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
                other => {
                    if let Some(expression) = other.as_expression() {
                        self.default_export(expression);
                    }
                }
            },
            Statement::TSExportAssignment(assignment) => self.default_export(&assignment.expression),
            _ => {
                if let Some(declaration) = statement.as_declaration() {
                    let visible = all_visible || self.declares_exported_name(declaration);
                    self.declaration(declaration, visible);
                }
            }
        }
    }

    fn declares_exported_name(&self, declaration: &Declaration<'_>) -> bool {
        let name = match declaration {
            Declaration::FunctionDeclaration(function) => function.id.as_ref().map(|id| id.name.as_str()),
            Declaration::ClassDeclaration(class) => class.id.as_ref().map(|id| id.name.as_str()),
            Declaration::TSEnumDeclaration(declaration) => Some(declaration.id.name.as_str()),
            Declaration::TSInterfaceDeclaration(interface) => Some(interface.id.name.as_str()),
            Declaration::VariableDeclaration(variable) => {
                return variable.declarations.iter().any(|declarator| {
                    matches!(&declarator.id, BindingPattern::BindingIdentifier(id)
                        if self.exported_names.iter().any(|name| name == id.name.as_str()))
                });
            }
            _ => None,
        };
        name.is_some_and(|name| self.exported_names.iter().any(|exported| exported == name))
    }

    fn declaration(&mut self, declaration: &Declaration<'_>, visible: bool) {
        if !visible {
            return;
        }
        match declaration {
            Declaration::VariableDeclaration(variable) => self.variable_statement(variable),
            Declaration::FunctionDeclaration(function) => self.function_declaration(function),
            Declaration::ClassDeclaration(class) => self.class(class),
            Declaration::TSEnumDeclaration(declaration) => self.enum_declaration(declaration),
            Declaration::TSModuleDeclaration(module) => self.namespace(module),
            Declaration::TSInterfaceDeclaration(interface) => self.interface(interface),
            _ => {}
        }
    }

    /// An interface member with nothing written for its type: there is no
    /// declaration tsc names for it (`findNearestDeclaration` stops at the
    /// statement), so the error is the general one (TS9013).
    fn interface(&mut self, interface: &oxc_ast::ast::TSInterfaceDeclaration<'_>) {
        for member in &interface.body.body {
            match member {
                oxc_ast::ast::TSSignature::TSPropertySignature(property) if property.type_annotation.is_none() => {
                    self.push(Report { code: EXPRESSION, span: property.span });
                }
                oxc_ast::ast::TSSignature::TSMethodSignature(method)
                    if method.return_type.is_none()
                        && matches!(method.kind, oxc_ast::ast::TSMethodSignatureKind::Method) =>
                {
                    self.push(Report { code: EXPRESSION, span: method.span });
                }
                _ => {}
            }
        }
    }

    fn namespace(&mut self, module: &TSModuleDeclaration<'_>) {
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                // Members are visible when exported, or in an ambient body.
                self.statements(&block.body, module.declare);
            }
            Some(TSModuleDeclarationBody::TSModuleDeclaration(nested)) => self.namespace(nested),
            None => {}
        }
    }

    fn variable_statement(&mut self, variable: &VariableDeclaration<'_>) {
        let is_const = matches!(
            variable.kind,
            VariableDeclarationKind::Const | VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        );
        for declarator in &variable.declarations {
            if is_const
                && let BindingPattern::BindingIdentifier(identifier) = &declarator.id
                && matches!(
                    declarator.init.as_ref().map(|init| init.without_parentheses()),
                    Some(Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_))
                )
            {
                self.functions.push(identifier.name.to_string());
            }
            let name_span = match &declarator.id {
                BindingPattern::BindingIdentifier(identifier) => identifier.span,
                pattern => {
                    self.binding_elements(pattern);
                    continue;
                }
            };
            if declarator.type_annotation.is_some() {
                continue;
            }
            let Some(init) = &declarator.init else {
                self.push(Report { code: VARIABLE, span: name_span });
                continue;
            };
            if is_const && is_fresh_literal(init) {
                continue;
            }
            let own = Report { code: VARIABLE, span: name_span };
            let pseudo = if is_const && matches!(init.without_parentheses(), Expression::TemplateLiteral(t) if !t.expressions.is_empty())
            {
                Pseudo::NoResult(own)
            } else {
                let expression = self.expression(init, Site::of(VARIABLE));
                if expression.is_bare_inferred() { Pseudo::NoResult(own) } else { expression }
            };
            self.report_declaration_type(pseudo);
        }
    }

    /// `createBindingElementError`: an exported destructuring declares no
    /// type it could reuse, element by element.
    fn binding_elements(&mut self, pattern: &BindingPattern<'_>) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                self.push(Report { code: BINDING_ELEMENT, span: identifier.span })
            }
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    self.binding_elements(&property.value);
                }
                if let Some(rest) = &object.rest {
                    self.binding_elements(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(array) => {
                for element in array.elements.iter().flatten() {
                    self.binding_elements(element);
                }
                if let Some(rest) = &array.rest {
                    self.binding_elements(&rest.argument);
                }
            }
            BindingPattern::AssignmentPattern(assignment) => self.binding_elements(&assignment.left),
        }
    }

    fn function_declaration(&mut self, function: &Function<'_>) {
        if let Some(id) = &function.id {
            self.functions.push(id.name.to_string());
        }
        // The implementation of an overload set is not emitted.
        if function.body.is_some()
            && function.id.as_ref().is_some_and(|id| self.overloaded.iter().any(|name| name == id.name.as_str()))
        {
            return;
        }
        let anchor = function.id.as_ref().map_or(function.span, |id| id.span);
        self.parameters(&function.params, false);
        if function.return_type.is_none() {
            let pseudo = self.function_return(function.body.as_deref(), function.r#async, function.generator, false, Report {
                code: FUNCTION_RETURN,
                span: anchor,
            });
            self.report_declaration_type(pseudo);
        }
    }

    /// Every parameter of a visible signature has its type written out
    /// (`updateParamList` → `ensureType`).
    fn parameters(&mut self, parameters: &FormalParameters<'_>, setter: bool) {
        let items: Vec<&FormalParameter<'_>> = parameters.items.iter().collect();
        let last_required = last_required_parameter(&items);
        for (index, parameter) in items.iter().enumerate() {
            if setter {
                continue;
            }
            let pseudo = self.parameter(parameter, index, last_required, false);
            self.report_declaration_type(pseudo);
        }
        if let Some(rest) = &parameters.rest
            && rest.type_annotation.is_none()
        {
            self.push(Report { code: PARAMETER, span: binding_span(&rest.rest.argument) });
        }
    }

    /// `typeFromParameter`, with the error a parameter itself gets
    /// (`createParameterError`). In a function expression's signature, an
    /// initialized parameter a required one follows takes `undefined` from its
    /// callers, which a reused type reference cannot be shown to include
    /// (TS9025); a declaration's own signature adds it itself.
    fn parameter(&self, parameter: &FormalParameter<'_>, index: usize, last_required: usize, expression_signature: bool) -> Pseudo {
        let name_span = binding_span(&parameter.pattern);
        let has_required_after = index + 1 < last_required;
        let reused_type = parameter.type_annotation.as_ref().map(|annotation| &annotation.type_annotation).or_else(|| {
            match parameter.initializer.as_ref().map(|initializer| initializer.without_parentheses()) {
                Some(Expression::TSAsExpression(assertion)) if !is_const_type(&assertion.type_annotation) => {
                    Some(&assertion.type_annotation)
                }
                Some(Expression::TSTypeAssertion(assertion)) if !is_const_type(&assertion.type_annotation) => {
                    Some(&assertion.type_annotation)
                }
                _ => None,
            }
        });
        if expression_signature
            && self.strict_null_checks
            && parameter.initializer.is_some()
            && has_required_after
            && reused_type.is_some_and(type_could_refer_to_undefined)
        {
            return Pseudo::NoResult(Report { code: IMPLICIT_UNDEFINED, span: name_span });
        }
        if parameter.type_annotation.is_some() {
            return Pseudo::Direct;
        }
        let Some(initializer) = &parameter.initializer else {
            return Pseudo::NoResult(Report { code: PARAMETER, span: name_span });
        };
        let site = Site::of(PARAMETER);
        let initializer_error = Report { code: PARAMETER, span: expression_span(initializer) };
        if matches!(parameter.pattern, BindingPattern::BindingIdentifier(_)) {
            let expression = self.expression(initializer, site);
            if expression.is_bare_inferred() {
                return Pseudo::Inferred { errors: vec![initializer_error], fallback: initializer_error };
            }
            return expression;
        }
        Pseudo::NoResult(initializer_error)
    }

    /// `createReturnFromSignature` for a signature with no written return
    /// type: its single `return`'s type, when it has exactly one at the top
    /// of its body (`typeFromSingleReturnExpression`).
    fn function_return(
        &self,
        body: Option<&FunctionBody<'_>>,
        is_async: bool,
        is_generator: bool,
        expression_body: bool,
        own: Report,
    ) -> Pseudo {
        let Some(body) = body else {
            return Pseudo::NoResult(own);
        };
        if is_async && is_generator {
            return Pseudo::inferred(own);
        }
        let candidate = if expression_body {
            match body.statements.first() {
                Some(Statement::ExpressionStatement(statement)) => Some(&statement.expression),
                _ => None,
            }
        } else {
            single_top_level_return(body)
        };
        // Neither `async` nor a generator changes what the single return says
        // for the pseudochecker; the checker's wrapping is its own.
        let _ = (is_async, is_generator);
        match candidate {
            Some(expression) => {
                let site = Site::detached();
                match expression.without_parentheses() {
                    Expression::TSAsExpression(assertion) if !is_const_type(&assertion.type_annotation) => Pseudo::Direct,
                    Expression::TSTypeAssertion(assertion) if !is_const_type(&assertion.type_annotation) => Pseudo::Direct,
                    _ => self.expression(expression, site),
                }
            }
            None => Pseudo::inferred(own),
        }
    }

    fn class(&mut self, class: &Class<'_>) {
        if let Some(super_class) = &class.super_class
            && !is_entity_name_expression(super_class)
        {
            self.push(Report { code: EXTENDS_EXPRESSION, span: expression_span(super_class) });
        }
        let accessor_pairs = accessor_pairs(class);
        for element in &class.body.body {
            match element {
                ClassElement::PropertyDefinition(property) => {
                    if is_private(property.accessibility, &property.key) {
                        continue;
                    }
                    if property.computed && !is_literal_key(&property.key) && !is_well_known_symbol(&property.key) {
                        self.push(Report { code: COMPUTED_NAME, span: computed_name_span(property.span, &property.key) });
                        continue;
                    }
                    if property.type_annotation.is_some() {
                        continue;
                    }
                    let own = Report { code: PROPERTY, span: property.key.span() };
                    let Some(value) = &property.value else {
                        self.push(own);
                        continue;
                    };
                    if property.readonly && is_fresh_literal(value) {
                        continue;
                    }
                    if property.readonly
                        && matches!(value.without_parentheses(), Expression::TemplateLiteral(t) if !t.expressions.is_empty())
                    {
                        self.push(own);
                        continue;
                    }
                    let pseudo = self.expression(value, Site::of(PROPERTY));
                    let pseudo = if pseudo.is_bare_inferred() { Pseudo::NoResult(own) } else { pseudo };
                    self.report_declaration_type(pseudo);
                }
                ClassElement::MethodDefinition(method) => {
                    let private = is_private(method.accessibility, &method.key);
                    match method.kind {
                        MethodDefinitionKind::Constructor => {
                            if method.value.body.is_some() || !self.has_overloads(class, &method.key, method.kind) {
                                self.constructor_parameters(&method.value.params);
                            }
                        }
                        _ if private => {}
                        MethodDefinitionKind::Get | MethodDefinitionKind::Set
                            if method.computed && !is_literal_key(&method.key) && !is_well_known_symbol(&method.key) =>
                        {
                            self.push(Report { code: COMPUTED_NAME, span: computed_name_span(method.span, &method.key) });
                        }
                        MethodDefinitionKind::Method => {
                            // A dynamic name is its own error; a name that
                            // may still be late-bound keeps its signature.
                            if method.computed && !is_literal_key(&method.key) && !is_well_known_symbol(&method.key) {
                                self.push(Report { code: COMPUTED_NAME, span: computed_name_span(method.span, &method.key) });
                                if !matches!(&method.key, PropertyKey::Identifier(_) | PropertyKey::StaticMemberExpression(_)) {
                                    continue;
                                }
                            }
                            if method.value.body.is_some() && self.has_overloads(class, &method.key, method.kind) {
                                continue;
                            }
                            self.parameters(&method.value.params, false);
                            if method.value.return_type.is_none() {
                                let own = Report { code: METHOD_RETURN, span: member_name_span(method.span, method.computed, &method.key) };
                                let pseudo = self.function_return(
                                    method.value.body.as_deref(),
                                    method.value.r#async,
                                    method.value.generator,
                                    false,
                                    own,
                                );
                                self.report_declaration_type(pseudo);
                            }
                        }
                        MethodDefinitionKind::Get | MethodDefinitionKind::Set => {
                            if let Some(report) = self.accessor(&accessor_pairs, method) {
                                self.push(report);
                            }
                        }
                    }
                }
                ClassElement::AccessorProperty(property) => {
                    if is_private(property.accessibility, &property.key) || property.type_annotation.is_some() {
                        continue;
                    }
                    let own = Report { code: PROPERTY, span: property.key.span() };
                    match &property.value {
                        Some(value) => {
                            let pseudo = self.expression(value, Site::of(PROPERTY));
                            let pseudo = if pseudo.is_bare_inferred() { Pseudo::NoResult(own) } else { pseudo };
                            self.report_declaration_type(pseudo);
                        }
                        None => self.push(own),
                    }
                }
                _ => {}
            }
        }
    }

    fn has_overloads(&self, class: &Class<'_>, key: &PropertyKey<'_>, kind: MethodDefinitionKind) -> bool {
        let name = static_key_name(key);
        class.body.body.iter().any(|element| {
            matches!(element, ClassElement::MethodDefinition(other)
                if other.kind == kind && other.value.body.is_none() && static_key_name(&other.key) == name && name.is_some())
        })
    }

    /// A constructor's parameters, a parameter property's included: its type
    /// is visible even when the property is private.
    fn constructor_parameters(&mut self, parameters: &FormalParameters<'_>) {
        self.parameters(parameters, false);
    }

    /// `typeFromAccessor` for one accessor of a class: the pair's written
    /// type, else the getter's single return; an error lands on the getter
    /// and on the setter's parameter (`createAccessorTypeError`).
    fn accessor(&self, pairs: &[AccessorPair<'_, '_>], method: &oxc_ast::ast::MethodDefinition<'_>) -> Option<Report> {
        let name = static_key_name(&method.key)?;
        let pair = pairs.iter().find(|pair| pair.name == name && pair.is_static == method.r#static)?;
        if pair.annotated {
            return None;
        }
        let own = Report { code: ACCESSOR, span: method.key.span() };
        let getter_ok = pair.getter_body.is_some_and(|getter| {
            !matches!(self.function_return(Some(getter), false, false, false, own), Pseudo::Inferred { .. } | Pseudo::NoResult(_))
        });
        if getter_ok {
            return None;
        }
        let span = match method.kind {
            MethodDefinitionKind::Set => method.value.params.items.first().map_or(method.key.span(), |parameter| binding_span(&parameter.pattern)),
            _ => method.key.span(),
        };
        Some(Report { code: ACCESSOR, span })
    }

    /// `EvaluatorResult.HasExternalReferences`: a member whose value reads
    /// another declaration — a variable, another enum — or one of its own
    /// enum's members that did.
    fn enum_declaration(&mut self, declaration: &oxc_ast::ast::TSEnumDeclaration<'_>) {
        let enum_name = declaration.id.name.as_str();
        let mut clean: Vec<String> = Vec::new();
        let mut tainted: Vec<String> = Vec::new();
        for member in &declaration.body.members {
            let name = enum_member_name(&member.id);
            let external = member.initializer.as_ref().is_some_and(|initializer| {
                !matches!(member.id, oxc_ast::ast::TSEnumMemberName::ComputedTemplateString(_) | oxc_ast::ast::TSEnumMemberName::ComputedString(_))
                    && references_external_symbol(initializer, &clean, &tainted, enum_name)
            });
            if external {
                self.push(Report { code: ENUM_MEMBER, span: member.span });
            }
            if let Some(name) = name {
                if external { tainted.push(name) } else { clean.push(name) }
            }
        }
    }

    fn default_export(&mut self, expression: &Expression<'_>) {
        if is_entity_name_expression(expression) {
            return;
        }
        let pseudo = self.expression(expression, Site::of(DEFAULT_EXPORT));
        self.report_declaration_type(pseudo);
    }

    /// `foo.x = …` on a visible function declaration: tsc declares the
    /// property on the function, which isolated declarations cannot.
    fn expando_assignments(&mut self, statements: &[Statement<'_>]) {
        if self.functions.is_empty() {
            return;
        }
        let mut assigned: Vec<(String, String)> = Vec::new();
        for statement in statements {
            let Statement::ExpressionStatement(statement) = statement else {
                continue;
            };
            let Expression::AssignmentExpression(assignment) = &statement.expression else {
                continue;
            };
            // Only a name tsc binds as a declaration: a property name, a
            // literal key, or an entity name that may be a literal.
            let (target, key) = match &assignment.left {
                oxc_ast::ast::AssignmentTarget::StaticMemberExpression(member) => {
                    (&member.object, member.property.name.to_string())
                }
                oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => {
                    let key = match &member.expression {
                        Expression::StringLiteral(literal) => literal.value.to_string(),
                        Expression::NumericLiteral(literal) => literal.value.to_string(),
                        key if is_entity_name_expression(key) => format!("[{}]", key.span().source_text(self.source_text)),
                        _ => continue,
                    };
                    (&member.object, key)
                }
                _ => continue,
            };
            if let Expression::Identifier(object) = target
                && self.functions.iter().any(|name| name == object.name.as_str())
            {
                let entry = (object.name.to_string(), key);
                if !assigned.contains(&entry) {
                    assigned.push(entry);
                    self.push(Report { code: EXPANDO, span: statement.span });
                }
            }
        }
    }

    /// `typeFromExpression`.
    fn expression(&self, expression: &Expression<'_>, site: Site) -> Pseudo {
        let fallback = Report { code: site.expression_code(), span: expression_span(expression) };
        match expression {
            Expression::ParenthesizedExpression(parenthesized) => self.expression(&parenthesized.expression, site),
            Expression::Identifier(identifier) if identifier.name == "undefined" => Pseudo::Plain,
            Expression::NullLiteral(_) => Pseudo::Plain,
            Expression::ArrowFunctionExpression(arrow) => {
                let own = Report { code: FUNCTION_RETURN, span: arrow.span };
                let mut parts = self.signature_parameters(&arrow.params);
                let return_type = if arrow.return_type.is_some() {
                    Pseudo::Direct
                } else {
                    self.function_return(Some(&arrow.body), arrow.r#async, false, arrow.expression, own)
                };
                parts.push(return_type);
                Pseudo::Composite(parts)
            }
            Expression::FunctionExpression(function) => {
                let own = Report { code: FUNCTION_RETURN, span: super::spans::function_expression_error_span(function) };
                let mut parts = self.signature_parameters(&function.params);
                let return_type = if function.return_type.is_some() {
                    Pseudo::Direct
                } else {
                    self.function_return(function.body.as_deref(), function.r#async, function.generator, false, own)
                };
                parts.push(return_type);
                Pseudo::Composite(parts)
            }
            Expression::TSAsExpression(assertion) => {
                if is_const_type(&assertion.type_annotation) {
                    self.expression(&assertion.expression, Site { const_context: true, ..site })
                } else {
                    Pseudo::Direct
                }
            }
            Expression::TSTypeAssertion(assertion) => {
                if is_const_type(&assertion.type_annotation) {
                    self.expression(&assertion.expression, Site { const_context: true, ..site })
                } else {
                    Pseudo::Direct
                }
            }
            Expression::UnaryExpression(unary)
                if matches!(unary.operator, UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus)
                    && matches!(unary.argument, Expression::NumericLiteral(_) | Expression::BigIntLiteral(_)) =>
            {
                Pseudo::MaybeConst { const_type: Box::new(Pseudo::Plain), regular: Box::new(Pseudo::Plain), const_context: site.const_context }
            }
            Expression::ArrayExpression(array) => {
                if !site.const_context {
                    return Pseudo::Inferred {
                        errors: vec![Report { code: ARRAY_LITERAL, span: array.span }],
                        fallback,
                    };
                }
                if let Some(ArrayExpressionElement::SpreadElement(spread)) =
                    array.elements.iter().find(|element| matches!(element, ArrayExpressionElement::SpreadElement(_)))
                {
                    return Pseudo::Inferred {
                        errors: vec![Report { code: SPREAD_ELEMENT, span: spread.span }],
                        fallback,
                    };
                }
                let element_site = site.nested();
                let elements = array
                    .elements
                    .iter()
                    .map(|element| match element.as_expression() {
                        Some(element) => self.expression(element, element_site),
                        None => Pseudo::Plain,
                    })
                    .collect();
                Pseudo::Composite(elements)
            }
            Expression::ObjectExpression(object) => {
                let mut errors = Vec::new();
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            errors.push(Report { code: SPREAD_ASSIGNMENT, span: spread.span })
                        }
                        ObjectPropertyKind::ObjectProperty(property) => {
                            if property.shorthand {
                                errors.push(Report { code: SHORTHAND, span: property.span });
                            } else if matches!(property.key, PropertyKey::PrivateIdentifier(_)) {
                                errors.push(Report { code: EXPRESSION, span: property.span });
                            } else if property.computed && !is_literal_key(&property.key) {
                                errors.push(Report { code: COMPUTED_NAME, span: computed_name_span(property.span, &property.key) });
                            }
                        }
                    }
                }
                if !errors.is_empty() {
                    return Pseudo::Inferred { errors, fallback };
                }
                let member_site = site.nested();
                let mut parts = Vec::new();
                let pairs = object_accessor_pairs(object);
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    match property.kind {
                        PropertyKind::Init if property.method => {
                            let Expression::FunctionExpression(function) = &property.value else {
                                continue;
                            };
                            parts.extend(self.signature_parameters(&function.params));
                            if function.return_type.is_none() {
                                let own = Report { code: METHOD_RETURN, span: property.key.span() };
                                parts.push(self.function_return(
                                    function.body.as_deref(),
                                    function.r#async,
                                    function.generator,
                                    false,
                                    own,
                                ));
                            }
                        }
                        PropertyKind::Init => parts.push(self.expression(&property.value, member_site)),
                        PropertyKind::Get | PropertyKind::Set => {
                            if let Some(report) = self.object_accessor(&pairs, property) {
                                parts.push(Pseudo::NoResult(report));
                            }
                        }
                    }
                }
                Pseudo::Composite(parts)
            }
            Expression::ClassExpression(class) => Pseudo::Inferred {
                errors: vec![Report { code: CLASS_EXPRESSION, span: super::spans::class_expression_error_span(class) }],
                fallback,
            },
            Expression::TemplateLiteral(template) => {
                if template.expressions.is_empty() {
                    Pseudo::MaybeConst { const_type: Box::new(Pseudo::Plain), regular: Box::new(Pseudo::Plain), const_context: site.const_context }
                } else if site.const_context {
                    Pseudo::inferred(fallback)
                } else {
                    Pseudo::MaybeConst {
                        const_type: Box::new(Pseudo::inferred(fallback)),
                        regular: Box::new(Pseudo::Plain),
                        const_context: site.const_context,
                    }
                }
            }
            Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::BooleanLiteral(_) => {
                Pseudo::MaybeConst { const_type: Box::new(Pseudo::Plain), regular: Box::new(Pseudo::Plain), const_context: site.const_context }
            }
            _ => Pseudo::inferred(fallback),
        }
    }

    /// A function expression's parameters, as `cloneParameters` types them.
    fn signature_parameters(&self, parameters: &FormalParameters<'_>) -> Vec<Pseudo> {
        let items: Vec<&FormalParameter<'_>> = parameters.items.iter().collect();
        let last_required = last_required_parameter(&items);
        let mut parts: Vec<Pseudo> = items
            .iter()
            .enumerate()
            .map(|(index, parameter)| self.parameter(parameter, index, last_required, true))
            .collect();
        if let Some(rest) = &parameters.rest
            && rest.type_annotation.is_none()
        {
            parts.push(Pseudo::NoResult(Report { code: PARAMETER, span: binding_span(&rest.rest.argument) }));
        }
        parts
    }

    fn object_accessor(&self, pairs: &[ObjectAccessorPair], property: &oxc_ast::ast::ObjectProperty<'_>) -> Option<Report> {
        let name = static_key_name(&property.key)?;
        let pair = pairs.iter().find(|pair| pair.name == name)?;
        if pair.annotated {
            return None;
        }
        if property.kind == PropertyKind::Get || pair.has_getter {
            if property.kind == PropertyKind::Set {
                return None;
            }
            let Expression::FunctionExpression(getter) = &property.value else {
                return None;
            };
            let own = Report { code: ACCESSOR, span: property.key.span() };
            let pseudo = self.function_return(getter.body.as_deref(), false, false, false, own);
            return matches!(pseudo, Pseudo::Inferred { .. } | Pseudo::NoResult(_)).then_some(own);
        }
        let Expression::FunctionExpression(setter) = &property.value else {
            return None;
        };
        let span = setter.params.items.first().map_or(property.key.span(), |parameter| binding_span(&parameter.pattern));
        Some(Report { code: ACCESSOR, span })
    }
}

struct AccessorPair<'b, 'a> {
    name: String,
    is_static: bool,
    annotated: bool,
    getter_body: Option<&'b FunctionBody<'a>>,
}

fn accessor_pairs<'b, 'a>(class: &'b Class<'a>) -> Vec<AccessorPair<'b, 'a>> {
    let mut pairs: Vec<AccessorPair<'b, 'a>> = Vec::new();
    for element in &class.body.body {
        let ClassElement::MethodDefinition(method) = element else {
            continue;
        };
        if !matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set) {
            continue;
        }
        let Some(name) = static_key_name(&method.key) else {
            continue;
        };
        let annotated = match method.kind {
            MethodDefinitionKind::Get => method.value.return_type.is_some(),
            _ => method.value.params.items.first().is_some_and(|parameter| parameter.type_annotation.is_some()),
        };
        let index = match pairs.iter().position(|pair| pair.name == name && pair.is_static == method.r#static) {
            Some(index) => index,
            None => {
                pairs.push(AccessorPair { name, is_static: method.r#static, annotated: false, getter_body: None });
                pairs.len() - 1
            }
        };
        let pair = &mut pairs[index];
        pair.annotated |= annotated;
        if method.kind == MethodDefinitionKind::Get {
            pair.getter_body = method.value.body.as_deref();
        }
    }
    pairs
}

struct ObjectAccessorPair {
    name: String,
    annotated: bool,
    has_getter: bool,
}

fn object_accessor_pairs(object: &oxc_ast::ast::ObjectExpression<'_>) -> Vec<ObjectAccessorPair> {
    let mut pairs: Vec<ObjectAccessorPair> = Vec::new();
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };
        if !matches!(property.kind, PropertyKind::Get | PropertyKind::Set) {
            continue;
        }
        let Some(name) = static_key_name(&property.key) else {
            continue;
        };
        let Expression::FunctionExpression(function) = &property.value else {
            continue;
        };
        let annotated = match property.kind {
            PropertyKind::Get => function.return_type.is_some(),
            _ => function.params.items.first().is_some_and(|parameter| parameter.type_annotation.is_some()),
        };
        let index = match pairs.iter().position(|pair| pair.name == name) {
            Some(index) => index,
            None => {
                pairs.push(ObjectAccessorPair { name, annotated: false, has_getter: false });
                pairs.len() - 1
            }
        };
        pairs[index].annotated |= annotated;
        pairs[index].has_getter |= property.kind == PropertyKind::Get;
    }
    pairs
}

fn overloaded_function_names(statements: &[Statement<'_>]) -> Vec<String> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::FunctionDeclaration(function) => Some(function),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::FunctionDeclaration(function)) => Some(function),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => Some(function),
                _ => None,
            },
            _ => None,
        })
        .filter(|function| function.body.is_none())
        .filter_map(|function| function.id.as_ref().map(|id| id.name.to_string()))
        .collect()
}

fn last_required_parameter(parameters: &[&FormalParameter<'_>]) -> usize {
    parameters
        .iter()
        .rposition(|parameter| !parameter.optional && parameter.initializer.is_none())
        .map_or(0, |index| index + 1)
}

fn single_top_level_return<'b, 'a>(body: &'b FunctionBody<'a>) -> Option<&'b Expression<'a>> {
    let mut found: Option<&Expression<'a>> = None;
    let mut count = 0usize;
    let mut nested = false;
    for statement in &body.statements {
        match statement {
            Statement::ReturnStatement(statement) => {
                count += 1;
                found = statement.argument.as_ref();
            }
            other => nested |= contains_return(other),
        }
    }
    (count == 1 && !nested).then_some(found).flatten()
}

fn declared_names(declaration: &Declaration<'_>) -> Vec<String> {
    match declaration {
        Declaration::VariableDeclaration(variable) => variable
            .declarations
            .iter()
            .filter_map(|declarator| match &declarator.id {
                BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
                _ => None,
            })
            .collect(),
        Declaration::FunctionDeclaration(function) => function.id.iter().map(|id| id.name.to_string()).collect(),
        Declaration::ClassDeclaration(class) => class.id.iter().map(|id| id.name.to_string()).collect(),
        Declaration::TSEnumDeclaration(declaration) => vec![declaration.id.name.to_string()],
        _ => Vec::new(),
    }
}

/// The leading names of the entity names declaration emit writes for a
/// statement: type queries and computed member keys. Function bodies are not
/// emitted.
fn collect_emitted_entity_names(statement: &Statement<'_>, names: &mut Vec<String>) {
    use oxc_ast_visit::Visit;
    struct Collector<'n>(&'n mut Vec<String>);
    impl<'a> Visit<'a> for Collector<'_> {
        fn visit_function_body(&mut self, _: &FunctionBody<'a>) {}
        fn visit_ts_type_query(&mut self, query: &oxc_ast::ast::TSTypeQuery<'a>) {
            if let Some(mut head) = query.expr_name.as_ts_type_name() {
                while let TSTypeName::QualifiedName(qualified) = head {
                    head = &qualified.left;
                }
                if let TSTypeName::IdentifierReference(identifier) = head {
                    self.0.push(identifier.name.to_string());
                }
            }
            oxc_ast_visit::walk::walk_ts_type_query(self, query);
        }
        fn visit_property_key(&mut self, key: &PropertyKey<'a>) {
            if let Some(mut expression) = key.as_expression().filter(|expression| is_entity_name_expression(expression)) {
                while let Expression::StaticMemberExpression(member) = expression {
                    expression = &member.object;
                }
                if let Expression::Identifier(identifier) = expression {
                    self.0.push(identifier.name.to_string());
                }
            }
            oxc_ast_visit::walk::walk_property_key(self, key);
        }
    }
    Collector(names).visit_statement(statement);
}

fn contains_return(statement: &Statement<'_>) -> bool {
    use oxc_ast_visit::Visit;
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_return_statement(&mut self, _: &oxc_ast::ast::ReturnStatement<'a>) {
            self.0 = true;
        }
        fn visit_function(&mut self, _: &Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
        fn visit_class(&mut self, _: &Class<'a>) {}
    }
    let mut finder = Finder(false);
    finder.visit_statement(statement);
    finder.0
}

/// `isLiteralConstDeclaration`'s fresh literal: the initializer is printed in
/// place of a type.
fn is_fresh_literal(expression: &Expression<'_>) -> bool {
    match expression.without_parentheses() {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::BooleanLiteral(_) => true,
        Expression::TemplateLiteral(template) => template.expressions.is_empty(),
        Expression::UnaryExpression(unary) => {
            unary.operator == UnaryOperator::UnaryNegation
                && matches!(unary.argument, Expression::NumericLiteral(_) | Expression::BigIntLiteral(_))
        }
        _ => false,
    }
}

fn is_const_type(ty: &TSType<'_>) -> bool {
    matches!(ty, TSType::TSTypeReference(reference)
        if reference.type_arguments.is_none()
            && matches!(&reference.type_name, TSTypeName::IdentifierReference(name) if name.name == "const"))
}

/// `IsDefinitelyReferenceToGlobalSymbolObject`: `Symbol.iterator` and the
/// like are unique symbols whatever the file declares.
fn is_well_known_symbol(key: &PropertyKey<'_>) -> bool {
    matches!(key, PropertyKey::StaticMemberExpression(member)
        if matches!(&member.object, Expression::Identifier(object) if object.name == "Symbol"))
}

/// Where tsc anchors an error on a member: its name, a computed one from `[`.
fn member_name_span(member_span: Span, computed: bool, key: &PropertyKey<'_>) -> Span {
    if computed { computed_name_span(member_span, key) } else { key.span() }
}

fn is_entity_name_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::Identifier(_) => true,
        Expression::StaticMemberExpression(member) => is_entity_name_expression(&member.object),
        _ => false,
    }
}

fn is_private(accessibility: Option<oxc_ast::ast::TSAccessibility>, key: &PropertyKey<'_>) -> bool {
    accessibility == Some(oxc_ast::ast::TSAccessibility::Private) || matches!(key, PropertyKey::PrivateIdentifier(_))
}

/// `isPrimitiveLiteralValue` on a computed key.
fn is_literal_key(key: &PropertyKey<'_>) -> bool {
    match key {
        PropertyKey::StringLiteral(_) | PropertyKey::NumericLiteral(_) | PropertyKey::BigIntLiteral(_) => true,
        PropertyKey::TemplateLiteral(template) => template.expressions.is_empty(),
        PropertyKey::UnaryExpression(unary) => {
            unary.operator == UnaryOperator::UnaryNegation && matches!(unary.argument, Expression::NumericLiteral(_))
        }
        PropertyKey::StaticIdentifier(_) | PropertyKey::PrivateIdentifier(_) => true,
        _ => false,
    }
}

fn static_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::NumericLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

/// A computed name's `[`: the key's span leaves it out.
fn computed_name_span(property_span: Span, key: &PropertyKey<'_>) -> Span {
    let key_span = key.span();
    Span::new(property_span.start.min(key_span.start.saturating_sub(1)), key_span.end + 1)
}

fn binding_span(pattern: &BindingPattern<'_>) -> Span {
    match pattern {
        BindingPattern::AssignmentPattern(assignment) => binding_span(&assignment.left),
        other => other.span(),
    }
}

fn expression_span(expression: &Expression<'_>) -> Span {
    expression.span()
}

/// `typeNodeCouldReferToUndefined`.
fn type_could_refer_to_undefined(ty: &TSType<'_>) -> bool {
    match ty {
        TSType::TSParenthesizedType(inner) => type_could_refer_to_undefined(&inner.type_annotation),
        TSType::TSTypeReference(_)
        | TSType::TSIndexedAccessType(_)
        | TSType::TSTypeQuery(_)
        | TSType::TSImportType(_)
        | TSType::TSConditionalType(_)
        | TSType::TSTypeOperatorType(_)
        | TSType::TSTypePredicate(_)
        | TSType::TSUndefinedKeyword(_) => true,
        TSType::TSUnionType(union) => union.types.iter().any(type_could_refer_to_undefined),
        TSType::TSIntersectionType(intersection) => intersection.types.iter().any(type_could_refer_to_undefined),
        _ => false,
    }
}

fn enum_member_name(name: &oxc_ast::ast::TSEnumMemberName<'_>) -> Option<String> {
    match name {
        oxc_ast::ast::TSEnumMemberName::Identifier(identifier) => Some(identifier.name.to_string()),
        oxc_ast::ast::TSEnumMemberName::String(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

fn references_external_symbol(expression: &Expression<'_>, clean: &[String], tainted: &[String], enum_name: &str) -> bool {
    let own_member = |name: &str| -> Option<bool> {
        if clean.iter().any(|member| member == name) {
            Some(false)
        } else if tainted.iter().any(|member| member == name) {
            Some(true)
        } else {
            None
        }
    };
    match expression.without_parentheses() {
        Expression::TemplateLiteral(template) => template
            .expressions
            .iter()
            .any(|expression| references_external_symbol(expression, clean, tainted, enum_name)),
        Expression::Identifier(identifier) => match own_member(identifier.name.as_str()) {
            Some(tainted) => tainted,
            None => !matches!(identifier.name.as_str(), "Infinity" | "NaN" | "undefined"),
        },
        Expression::StaticMemberExpression(member) => match &member.object {
            Expression::Identifier(object) if object.name == enum_name => {
                own_member(member.property.name.as_str()).unwrap_or(false)
            }
            _ => true,
        },
        Expression::ComputedMemberExpression(member) => match (&member.object, &member.expression) {
            (Expression::Identifier(object), Expression::StringLiteral(key)) if object.name == enum_name => {
                own_member(key.value.as_str()).unwrap_or(false)
            }
            _ => true,
        },
        Expression::UnaryExpression(unary) => references_external_symbol(&unary.argument, clean, tainted, enum_name),
        Expression::BinaryExpression(binary) => {
            references_external_symbol(&binary.left, clean, tainted, enum_name)
                || references_external_symbol(&binary.right, clean, tainted, enum_name)
        }
        _ => false,
    }
}
