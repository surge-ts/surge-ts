//! The constructs tsc's checker looks `tslib` up for under `importHelpers`
//! (`checkExternalEmitHelpers` and its callers): where each one asks, for
//! which helpers, in the order the checker asks. A file reports a `tslib` it
//! cannot find once, at its first request, and a helper `tslib` lacks once, at
//! the first request for it, so the order decides where tsc reports.
//!
//! The checker checks function-expression bodies and class-expression members
//! after the rest of the file (`checkNodeDeferred`), breadth first; the walk
//! reports one deferral depth per pass to keep that order.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AccessorPropertyType, ArrowFunctionExpression, AssignmentExpression, AssignmentTarget,
    AssignmentTargetMaybeDefault, AssignmentTargetProperty, BindingPattern, CatchParameter, Class,
    ClassElement, Decorator, ExportAllDeclaration, ExportDefaultDeclaration,
    ExportDefaultDeclarationKind, ExportNamedDeclaration, Expression, ForInStatement, ForOfStatement,
    ForStatementLeft, FormalParameters, Function, ImportDeclaration, ImportDeclarationSpecifier,
    MethodDefinition, MethodDefinitionKind, ModuleExportName, ObjectProperty, PrivateFieldExpression,
    PrivateInExpression, PropertyDefinitionType, PropertyKey, PropertyKind, SimpleAssignmentTarget,
    TSEnumDeclaration, TSExportAssignment, TSGlobalDeclaration, TSInterfaceDeclaration,
    TSModuleDeclaration, TSModuleDeclarationName, TSType, TSTypeAliasDeclaration, TSTypeAnnotation,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, UpdateExpression, VariableDeclaration,
    VariableDeclarationKind, YieldExpression,
};
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::TextSpan;

/// tsgo's `ExternalEmitHelpers` (`checker/types.go`), bit for bit: the helpers
/// one request adds are looked up lowest bit first.
pub mod external_emit_helpers {
    pub const REST: u32 = 1 << 0;
    /// `__decorate`, or `ESDecorateAndRunInitializers` under ECMAScript
    /// decorators, which shares the bit.
    pub const DECORATE: u32 = 1 << 1;
    pub const METADATA: u32 = 1 << 2;
    pub const PARAM: u32 = 1 << 3;
    pub const AWAITER: u32 = 1 << 4;
    pub const AWAIT: u32 = 1 << 5;
    pub const ASYNC_GENERATOR: u32 = 1 << 6;
    pub const ASYNC_DELEGATOR: u32 = 1 << 7;
    pub const ASYNC_VALUES: u32 = 1 << 8;
    pub const EXPORT_STAR: u32 = 1 << 9;
    pub const IMPORT_STAR: u32 = 1 << 10;
    pub const IMPORT_DEFAULT: u32 = 1 << 11;
    pub const MAKE_TEMPLATE_OBJECT: u32 = 1 << 12;
    pub const CLASS_PRIVATE_FIELD_GET: u32 = 1 << 13;
    pub const CLASS_PRIVATE_FIELD_SET: u32 = 1 << 14;
    pub const CLASS_PRIVATE_FIELD_IN: u32 = 1 << 15;
    pub const SET_FUNCTION_NAME: u32 = 1 << 16;
    pub const PROP_KEY: u32 = 1 << 17;
    pub const ADD_DISPOSABLE_RESOURCE_AND_DISPOSE_RESOURCES: u32 = 1 << 18;
    pub const REWRITE_RELATIVE_IMPORT_EXTENSION: u32 = 1 << 19;
    pub const FIRST: u32 = REST;
    pub const LAST: u32 = REWRITE_RELATIVE_IMPORT_EXTENSION;

    /// tsgo's `getHelperNames`.
    pub fn names(helper: u32, legacy_decorators: bool) -> &'static [&'static str] {
        match helper {
            REST => &["__rest"],
            DECORATE if legacy_decorators => &["__decorate"],
            DECORATE => &["__esDecorate", "__runInitializers"],
            METADATA => &["__metadata"],
            PARAM => &["__param"],
            AWAITER => &["__awaiter"],
            AWAIT => &["__await"],
            ASYNC_GENERATOR => &["__asyncGenerator"],
            ASYNC_DELEGATOR => &["__asyncDelegator"],
            ASYNC_VALUES => &["__asyncValues"],
            EXPORT_STAR => &["__exportStar"],
            IMPORT_STAR => &["__importStar"],
            IMPORT_DEFAULT => &["__importDefault"],
            MAKE_TEMPLATE_OBJECT => &["__makeTemplateObject"],
            CLASS_PRIVATE_FIELD_GET => &["__classPrivateFieldGet"],
            CLASS_PRIVATE_FIELD_SET => &["__classPrivateFieldSet"],
            CLASS_PRIVATE_FIELD_IN => &["__classPrivateFieldIn"],
            SET_FUNCTION_NAME => &["__setFunctionName"],
            PROP_KEY => &["__propKey"],
            ADD_DISPOSABLE_RESOURCE_AND_DISPOSE_RESOURCES => {
                &["__addDisposableResource", "__disposeResources"]
            }
            REWRITE_RELATIVE_IMPORT_EXTENSION => &["__rewriteRelativeImportExtension"],
            _ => &[],
        }
    }
}

use external_emit_helpers::*;

/// tsgo's `core.ScriptTarget` numbering, for `LanguageFeatureMinimumTarget`.
const ES2017: u32 = 4;
const ES2018: u32 = 5;
const ESNEXT: u32 = 99;

/// What decides which constructs ask for helpers.
#[derive(Debug, Clone, Copy)]
pub struct EmitHelperOptions {
    /// `GetEmitScriptTarget`, numbered as tsgo's `core.ScriptTarget`.
    pub language_version: u32,
    /// `experimentalDecorators`.
    pub legacy_decorators: bool,
    /// tsgo's `GetUseDefineForClassFields`.
    pub use_define_for_class_fields: bool,
    /// tsgo's `GetEmitStandardClassFields`.
    pub emit_standard_class_fields: bool,
    /// `emitDecoratorMetadata` without `verbatimModuleSyntax`: the checker
    /// marks decorator references (`markLinkedReferences`) only when it can
    /// collect alias accessibility data.
    pub decorator_metadata: bool,
    /// The file's `GetEmitModuleFormatOfFile` is CommonJS.
    pub commonjs: bool,
}

/// One `checkExternalEmitHelpers` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitHelperRequest {
    /// Where the checker reports it (`GetErrorRangeForNode` of the location).
    pub span: TextSpan,
    pub helpers: u32,
    /// The module a named `default` import comes from: `checkImportDeclaration`
    /// binds named imports, and asks for `__importDefault` there, only when
    /// the module resolves.
    pub unless_unresolved: Option<String>,
}

/// The requests a file's check makes, in the checker's order.
pub fn emit_helper_requests(
    source_text: &str,
    file_name: &str,
    options: &EmitHelperOptions,
) -> Vec<EmitHelperRequest> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file_name).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source_text, source_type).parse();
    let mut walker = Walker {
        source_text,
        options,
        requests: Vec::new(),
        pass: 0,
        depth: 0,
        deepest: 0,
        frames: Vec::new(),
        named: None,
        export_default_start: None,
    };
    loop {
        walker.visit_program(&parsed.program);
        if walker.pass >= walker.deepest {
            break;
        }
        walker.pass += 1;
    }
    walker.requests
}

#[derive(Clone, Copy)]
struct FunctionFlags {
    is_async: bool,
    generator: bool,
    has_body: bool,
}

#[derive(Clone, Copy)]
enum Frame {
    Function(FunctionFlags),
    StaticBlock,
}

/// tsgo's `getAssignmentTargetKind`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AssignmentKind {
    None,
    Definite,
    Compound,
}

/// What a class or function expression learns from its parent: the name
/// `GetAssignedName` finds (a function expression's error location), whether
/// the parent is an `IsNamedEvaluationSource`, and whether it names the value
/// with a computed key.
#[derive(Clone, Copy)]
struct NamedEvaluation {
    name: Option<Span>,
    source: bool,
    computed_key: bool,
}

/// How a declaration's initializer relates to its name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InitializerRole {
    /// A variable's own initializer: it names a function or class expression.
    Variable,
    /// A parameter's default: a named evaluation source, but not an assigned
    /// name for a function expression's error location.
    Parameter,
    /// The expression a `for…of` declaration iterates.
    Iterated,
}

struct Walker<'s> {
    source_text: &'s str,
    options: &'s EmitHelperOptions,
    requests: Vec<EmitHelperRequest>,
    /// The deferral depth this pass reports: shallower code is walked through
    /// for what it defers, deeper code is left to a later pass.
    pass: u32,
    depth: u32,
    deepest: u32,
    /// The enclosing functions and class static blocks, innermost last.
    frames: Vec<Frame>,
    /// Set just before a value whose parent names it is visited; taken by the
    /// class or function expression it holds.
    named: Option<NamedEvaluation>,
    /// Where the `export default` holding the function about to be visited
    /// starts: an anonymous declaration reports at the statement's first token.
    export_default_start: Option<u32>,
}

impl Walker<'_> {
    fn push(&mut self, span: Span, helpers: u32) {
        self.push_request(span, helpers, None);
    }

    fn push_request(&mut self, span: Span, helpers: u32, unless_unresolved: Option<String>) {
        if helpers != 0 && self.depth == self.pass {
            self.requests.push(EmitHelperRequest {
                span: TextSpan {
                    start: span.start as usize,
                    end: span.end as usize,
                },
                helpers,
                unless_unresolved,
            });
        }
    }

    fn below(&self, language_version: u32) -> bool {
        self.options.language_version < language_version
    }

    /// Private names are read and written through `tslib` below ESNext, or
    /// without [[Define]] class fields.
    fn transforms_private_names(&self) -> bool {
        self.below(ESNEXT) || !self.options.use_define_for_class_fields
    }

    /// `checkNodeDeferred`.
    fn deferred(&mut self, check: impl FnOnce(&mut Self)) {
        self.depth += 1;
        if self.depth > self.pass {
            self.deepest = self.deepest.max(self.depth);
        } else {
            check(self);
        }
        self.depth -= 1;
    }

    /// tsgo's `GetContainingFunction`.
    fn containing_function(&self) -> Option<FunctionFlags> {
        self.frames.iter().rev().find_map(|frame| match frame {
            Frame::Function(flags) => Some(*flags),
            Frame::StaticBlock => None,
        })
    }

    /// `GetRangeOfTokenAtPosition`: the keyword or name at `start`.
    fn first_token(&self, start: u32) -> Span {
        let rest = self.source_text.get(start as usize..).unwrap_or_default();
        let length = rest
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
            .map_or(rest.len(), |(index, _)| index);
        Span::new(start, start + length.max(1) as u32)
    }

    /// A member or property name as tsc spans it: a computed name with its
    /// brackets.
    fn property_name_span(&self, key: &PropertyKey<'_>, computed: bool) -> Span {
        let span = key.span();
        if !computed {
            return span;
        }
        let before = self.source_text.get(..span.start as usize).unwrap_or_default().trim_end();
        let start = if before.ends_with('[') { before.len() as u32 - 1 } else { span.start };
        let after = self.source_text.get(span.end as usize..).unwrap_or_default();
        let trimmed = after.trim_start();
        let end = if trimmed.starts_with(']') {
            span.end + (after.len() - trimmed.len()) as u32 + 1
        } else {
            span.end
        };
        Span::new(start, end)
    }

    /// `getErrorRangeForArrowFunction`: a block body spanning lines reports
    /// the arrow's first line.
    fn arrow_error_span(&self, arrow: &ArrowFunctionExpression<'_>) -> Span {
        let body = arrow.body.span;
        let body_text = self.source_text.get(body.start as usize..body.end as usize).unwrap_or_default();
        if arrow.expression || !body_text.contains('\n') {
            return arrow.span;
        }
        let line_end = self
            .source_text
            .get(body.start as usize..)
            .and_then(|rest| rest.find('\n'))
            .map_or(arrow.span.end, |offset| body.start + offset as u32 + 1);
        Span::new(arrow.span.start, line_end)
    }

    /// The helpers `checkDecorators` asks for at a decorated node's first
    /// decorator, `markDecoratorAliasReferenced`'s `__metadata` included.
    fn decorator_helpers(&self, parameter: bool) -> u32 {
        let mut helpers = if self.options.legacy_decorators {
            if parameter { DECORATE | PARAM } else { DECORATE }
        } else if self.below(ESNEXT) {
            DECORATE
        } else {
            0
        };
        if self.options.decorator_metadata {
            helpers |= METADATA;
        }
        helpers
    }

    /// `NodeCanBeDecorated` for a class member: experimental decorators take a
    /// class declaration's members without private names, ECMAScript
    /// decorators any class's.
    fn member_decoratable(&self, class: &Class<'_>, key: &PropertyKey<'_>) -> bool {
        !self.options.legacy_decorators || (class.is_declaration() && !key.is_private_identifier())
    }

    /// `checkDecorators` past its guards: the request, then each decorator's
    /// expression.
    fn check_decorators<'a>(&mut self, decorators: &[Decorator<'a>], helpers: u32) {
        let Some(first) = decorators.first() else {
            return;
        };
        self.push(first.span, helpers);
        for decorator in decorators {
            self.visit_expression(&decorator.expression);
        }
    }

    /// `getFirstTransformableStaticClassElement`.
    fn first_transformable_static_element(&self, class: &Class<'_>) -> Option<Span> {
        let decorated =
            !self.options.legacy_decorators && self.below(ESNEXT) && !class.decorators.is_empty();
        let transforms_private = self.below(ESNEXT);
        if !decorated && !transforms_private {
            return None;
        }
        let transforms_initializers = !self.options.emit_standard_class_fields;
        for member in &class.body.body {
            if decorated && member_is_decorated(member) {
                return class.decorators.first().map(|decorator| decorator.span);
            }
            if !transforms_private {
                continue;
            }
            match member {
                ClassElement::StaticBlock(block) => return Some(block.span),
                ClassElement::MethodDefinition(method)
                    if method.r#static && method.key.is_private_identifier() =>
                {
                    return Some(self.property_name_span(&method.key, method.computed));
                }
                ClassElement::PropertyDefinition(property)
                    if property.r#static
                        && (property.key.is_private_identifier()
                            || (transforms_initializers && property.value.is_some())) =>
                {
                    return Some(self.property_name_span(&property.key, property.computed));
                }
                ClassElement::AccessorProperty(property)
                    if property.r#static
                        && (property.key.is_private_identifier()
                            || (transforms_initializers && property.value.is_some())) =>
                {
                    return Some(self.property_name_span(&property.key, property.computed));
                }
                _ => {}
            }
        }
        None
    }

    /// `checkClassExpressionExternalHelpers`: an anonymous class expression
    /// that takes its name from where it is assigned is named through
    /// `__setFunctionName` when decorated or when a static element of it is
    /// transformed, and a computed key through `__propKey`.
    fn check_class_expression_helpers(&mut self, class: &Class<'_>, named: Option<NamedEvaluation>) {
        if class.id.is_some() {
            return;
        }
        let Some(named) = named.filter(|named| named.source) else {
            return;
        };
        let decorator = if !self.options.legacy_decorators && self.below(ESNEXT) {
            class.decorators.first().map(|decorator| decorator.span)
        } else {
            None
        };
        let location = decorator.or_else(|| self.first_transformable_static_element(class));
        if let Some(location) = location {
            let helpers =
                if named.computed_key { SET_FUNCTION_NAME | PROP_KEY } else { SET_FUNCTION_NAME };
            self.push(location, helpers);
        }
    }

    /// `checkSourceElements(members)` of a class.
    fn check_class_members<'a>(&mut self, class: &Class<'a>) {
        for member in &class.body.body {
            match member {
                ClassElement::StaticBlock(block) => {
                    self.frames.push(Frame::StaticBlock);
                    self.visit_statements(&block.body);
                    self.frames.pop();
                }
                ClassElement::MethodDefinition(method) => self.check_method(class, method),
                ClassElement::PropertyDefinition(property) => {
                    if property.declare {
                        continue;
                    }
                    let decoratable = self.member_decoratable(class, &property.key)
                        && (self.options.legacy_decorators
                            || !matches!(
                                property.r#type,
                                PropertyDefinitionType::TSAbstractPropertyDefinition
                            ));
                    self.check_property(
                        decoratable,
                        &property.decorators,
                        &property.key,
                        property.computed,
                        false,
                        property.value.as_ref(),
                    );
                }
                ClassElement::AccessorProperty(property) => {
                    let decoratable = self.member_decoratable(class, &property.key)
                        && (self.options.legacy_decorators
                            || !matches!(
                                property.r#type,
                                AccessorPropertyType::TSAbstractAccessorProperty
                            ));
                    self.check_property(
                        decoratable,
                        &property.decorators,
                        &property.key,
                        property.computed,
                        true,
                        property.value.as_ref(),
                    );
                }
                ClassElement::TSIndexSignature(_) => {}
            }
        }
    }

    /// `checkPropertyDeclaration` (an auto-accessor is a property too):
    /// decorators, then a computed name, then the initializer.
    fn check_property<'a>(
        &mut self,
        decoratable: bool,
        decorators: &[Decorator<'a>],
        key: &PropertyKey<'a>,
        computed: bool,
        auto_accessor: bool,
        value: Option<&Expression<'a>>,
    ) {
        if decoratable && !decorators.is_empty() {
            let mut helpers = self.decorator_helpers(false);
            if !self.options.legacy_decorators && self.below(ESNEXT) {
                if auto_accessor && key.is_private_identifier() {
                    helpers |= SET_FUNCTION_NAME;
                }
                if computed {
                    helpers |= PROP_KEY;
                }
            }
            self.check_decorators(decorators, helpers);
        }
        if computed {
            self.visit_property_key(key);
        }
        if let Some(value) = value {
            self.visit_named_value(value, NamedEvaluation { name: None, source: true, computed_key: computed });
        }
    }

    /// `checkMethodDeclaration`, `checkConstructorDeclaration` and
    /// `checkAccessorDeclaration`: decorators, the signature, a computed
    /// name, then the body.
    fn check_method<'a>(&mut self, class: &Class<'a>, method: &MethodDefinition<'a>) {
        let function = &method.value;
        let has_body = function.body.is_some();
        if method.kind != MethodDefinitionKind::Constructor
            && has_body
            && self.member_decoratable(class, &method.key)
            && !method.decorators.is_empty()
        {
            let mut helpers = self.decorator_helpers(false);
            if !self.options.legacy_decorators && self.below(ESNEXT) {
                if method.key.is_private_identifier() {
                    helpers |= SET_FUNCTION_NAME;
                }
                if method.computed {
                    helpers |= PROP_KEY;
                }
            }
            self.check_decorators(&method.decorators, helpers);
        }
        let flags = FunctionFlags { is_async: function.r#async, generator: function.generator, has_body };
        // Parameters take decorators under experimentalDecorators alone, on a
        // class declaration's constructor, method or setter with a body.
        let parameter_decorators = self.options.legacy_decorators
            && has_body
            && class.is_declaration()
            && method.kind != MethodDefinitionKind::Get;
        let name = self.property_name_span(&method.key, method.computed);
        self.frames.push(Frame::Function(flags));
        self.check_signature(flags, name, &function.params, parameter_decorators);
        if method.computed {
            self.visit_property_key(&method.key);
        }
        if let Some(body) = &function.body {
            self.visit_function_body(body);
        }
        self.frames.pop();
    }

    /// `checkSignatureDeclaration`: an async function with a body is
    /// transformed below ES2017 and an async generator below ES2018, then the
    /// parameters are checked.
    fn check_signature<'a>(
        &mut self,
        flags: FunctionFlags,
        span: Span,
        params: &FormalParameters<'a>,
        parameter_decorators: bool,
    ) {
        if flags.has_body && flags.is_async {
            if flags.generator {
                if self.below(ES2018) {
                    self.push(span, AWAIT | ASYNC_GENERATOR);
                }
            } else if self.below(ES2017) {
                self.push(span, AWAITER);
            }
        }
        for parameter in &params.items {
            if parameter_decorators && !parameter.decorators.is_empty() {
                let helpers = self.decorator_helpers(true);
                self.check_decorators(&parameter.decorators, helpers);
            }
            self.check_variable_like(
                &parameter.pattern,
                parameter.type_annotation.is_some(),
                parameter.initializer.as_deref(),
                InitializerRole::Parameter,
            );
        }
        if let Some(rest) = &params.rest {
            if parameter_decorators && !rest.decorators.is_empty() {
                let helpers = self.decorator_helpers(true);
                self.check_decorators(&rest.decorators, helpers);
            }
            self.check_variable_like(
                &rest.rest.argument,
                rest.type_annotation.is_some(),
                None,
                InitializerRole::Parameter,
            );
        }
    }

    /// `checkVariableDeclarationList`: `using` declarations go through
    /// `tslib` below ESNext. A `for…of` declaration is typed from the
    /// expression it iterates (`checkRightHandSideOfForOf`).
    fn check_declaration_list<'b, 'a: 'b>(
        &mut self,
        declaration: &'b VariableDeclaration<'a>,
        iterated: Option<&'b Expression<'a>>,
    ) {
        if declaration.declare {
            return;
        }
        if matches!(declaration.kind, VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing)
            && self.below(ESNEXT)
        {
            // The declaration list, which ends before the statement's `;`.
            let end = declaration.declarations.last().map_or(declaration.span.end, |last| last.span.end);
            self.push(oxc_span::Span::new(declaration.span.start, end), ADD_DISPOSABLE_RESOURCE_AND_DISPOSE_RESOURCES);
        }
        let mut iterated = iterated;
        for declarator in &declaration.declarations {
            match iterated.take() {
                Some(expression) => {
                    self.check_variable_like(&declarator.id, false, Some(expression), InitializerRole::Iterated);
                }
                None => self.check_variable_like(
                    &declarator.id,
                    declarator.type_annotation.is_some(),
                    declarator.init.as_ref(),
                    InitializerRole::Variable,
                ),
            }
        }
        if let Some(expression) = iterated {
            self.visit_expression(expression);
        }
    }

    /// `checkVariableLikeDeclaration`: a binding pattern's elements come
    /// first, and the first of them checks the initializer while typing its
    /// parent (`getTypeForBindingElementParent`) unless an annotation types
    /// it; the initializer is checked last otherwise.
    fn check_variable_like<'b, 'a: 'b>(
        &mut self,
        pattern: &'b BindingPattern<'a>,
        annotated: bool,
        initializer: Option<&'b Expression<'a>>,
        role: InitializerRole,
    ) {
        if let BindingPattern::BindingIdentifier(identifier) = pattern {
            let Some(initializer) = initializer else {
                return;
            };
            match role {
                InitializerRole::Variable => self.visit_named_value(
                    initializer,
                    NamedEvaluation { name: Some(identifier.span), source: true, computed_key: false },
                ),
                InitializerRole::Parameter => self.visit_named_value(
                    initializer,
                    NamedEvaluation { name: None, source: true, computed_key: false },
                ),
                InitializerRole::Iterated => self.visit_expression(initializer),
            }
            return;
        }
        let mut parent_initializer = if annotated { None } else { initializer };
        self.check_binding_elements(pattern, &mut parent_initializer);
        let unchecked = if annotated { initializer } else { parent_initializer };
        if let Some(initializer) = unchecked {
            self.visit_expression(initializer);
        }
    }

    fn check_binding_elements<'b, 'a: 'b>(
        &mut self,
        pattern: &'b BindingPattern<'a>,
        parent_initializer: &mut Option<&'b Expression<'a>>,
    ) {
        match pattern {
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    if property.computed {
                        self.visit_property_key(&property.key);
                    }
                    self.check_parent_initializer(parent_initializer);
                    self.check_binding_element(&property.value);
                }
                if let Some(rest) = &object.rest {
                    if self.below(ES2018) {
                        self.push(rest.argument.span(), REST);
                    }
                    self.check_parent_initializer(parent_initializer);
                    self.check_binding_element(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(array) => {
                for element in array.elements.iter().flatten() {
                    self.check_parent_initializer(parent_initializer);
                    self.check_binding_element(element);
                }
                if let Some(rest) = &array.rest {
                    self.check_parent_initializer(parent_initializer);
                    self.check_binding_element(&rest.argument);
                }
            }
            BindingPattern::BindingIdentifier(_) | BindingPattern::AssignmentPattern(_) => {}
        }
    }

    fn check_parent_initializer<'b, 'a: 'b>(&mut self, parent_initializer: &mut Option<&'b Expression<'a>>) {
        if let Some(initializer) = parent_initializer.take() {
            self.visit_expression(initializer);
        }
    }

    /// A binding element's own pattern, then its default.
    fn check_binding_element<'b, 'a: 'b>(&mut self, element: &'b BindingPattern<'a>) {
        let mut typed = None;
        match element {
            BindingPattern::AssignmentPattern(assignment) => {
                self.check_binding_elements(&assignment.left, &mut typed);
                match &assignment.left {
                    BindingPattern::BindingIdentifier(identifier) => self.visit_named_value(
                        &assignment.right,
                        NamedEvaluation { name: Some(identifier.span), source: true, computed_key: false },
                    ),
                    _ => self.visit_expression(&assignment.right),
                }
            }
            pattern => self.check_binding_elements(pattern, &mut typed),
        }
    }

    /// Visits a value its parent may name: a function expression directly
    /// under the parent reads the name for its error location, and a class
    /// expression under parentheses and type assertions whether it is named at
    /// run time (`walkUpOuterExpressions`).
    fn visit_named_value<'a>(&mut self, value: &Expression<'a>, named: NamedEvaluation) {
        let reads_name = match value {
            Expression::FunctionExpression(_) => true,
            value => matches!(skip_outer_expressions(value), Expression::ClassExpression(_)),
        };
        if reads_name {
            self.named = Some(named);
        }
        self.visit_expression(value);
        self.named = None;
    }

    /// `checkDestructuringAssignment` and `checkReferenceExpression` of an
    /// assignment target: an object pattern's rest goes through `__rest`
    /// below ES2018.
    fn check_assignment_target<'a>(&mut self, target: &AssignmentTarget<'a>, kind: AssignmentKind) {
        match target {
            AssignmentTarget::ObjectAssignmentTarget(object) => {
                for property in &object.properties {
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                            if let Some(init) = &shorthand.init {
                                self.visit_named_value(
                                    init,
                                    NamedEvaluation { name: None, source: true, computed_key: false },
                                );
                            }
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                            if property.computed {
                                self.visit_property_key(&property.name);
                            }
                            self.check_assignment_target_maybe_default(&property.binding);
                        }
                    }
                }
                if let Some(rest) = &object.rest {
                    if self.below(ES2018) {
                        self.push(rest.span, REST);
                    }
                    self.check_assignment_target(&rest.target, AssignmentKind::Definite);
                }
            }
            AssignmentTarget::ArrayAssignmentTarget(array) => {
                for element in array.elements.iter().flatten() {
                    self.check_assignment_target_maybe_default(element);
                }
                if let Some(rest) = &array.rest {
                    self.check_assignment_target(&rest.target, AssignmentKind::Definite);
                }
            }
            target => {
                if let Some(target) = target.as_simple_assignment_target() {
                    self.check_simple_target(target, kind);
                }
            }
        }
    }

    fn check_assignment_target_maybe_default<'a>(&mut self, element: &AssignmentTargetMaybeDefault<'a>) {
        if let AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) = element {
            self.check_assignment_target(&with_default.binding, AssignmentKind::Definite);
            match with_default.binding.as_simple_assignment_target() {
                Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)) => self.visit_named_value(
                    &with_default.init,
                    NamedEvaluation { name: Some(identifier.span), source: true, computed_key: false },
                ),
                _ => self.visit_expression(&with_default.init),
            }
        } else if let Some(target) = element.as_assignment_target() {
            self.check_assignment_target(target, AssignmentKind::Definite);
        }
    }

    fn check_simple_target<'a>(&mut self, target: &SimpleAssignmentTarget<'a>, kind: AssignmentKind) {
        match target {
            SimpleAssignmentTarget::PrivateFieldExpression(field) => self.check_private_field(field, kind),
            // `GetAssignmentTarget` looks through `!`, not through type assertions.
            SimpleAssignmentTarget::TSNonNullExpression(non_null) => {
                match skip_parentheses(&non_null.expression) {
                    Expression::PrivateFieldExpression(field) => self.check_private_field(field, kind),
                    _ => self.visit_expression(&non_null.expression),
                }
            }
            target => oxc_ast_visit::walk::walk_simple_assignment_target(self, target),
        }
    }

    /// `checkPropertyAccessExpressionOrQualifiedName` for `x.#name`.
    fn check_private_field<'a>(&mut self, field: &PrivateFieldExpression<'a>, kind: AssignmentKind) {
        self.visit_expression(&field.object);
        if self.transforms_private_names() {
            let mut helpers = 0;
            if kind != AssignmentKind::None {
                helpers |= CLASS_PRIVATE_FIELD_SET;
            }
            if kind != AssignmentKind::Definite {
                helpers |= CLASS_PRIVATE_FIELD_GET;
            }
            self.push(field.span, helpers);
        }
    }
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_ts_type(&mut self, _: &TSType<'a>) {}

    fn visit_ts_type_annotation(&mut self, _: &TSTypeAnnotation<'a>) {}

    fn visit_ts_type_parameter_declaration(&mut self, _: &TSTypeParameterDeclaration<'a>) {}

    fn visit_ts_type_parameter_instantiation(&mut self, _: &TSTypeParameterInstantiation<'a>) {}

    fn visit_ts_type_alias_declaration(&mut self, _: &TSTypeAliasDeclaration<'a>) {}

    fn visit_ts_interface_declaration(&mut self, _: &TSInterfaceDeclaration<'a>) {}

    // Nothing ambient asks for a helper (`location.Flags&NodeFlagsAmbient`).
    fn visit_ts_global_declaration(&mut self, _: &TSGlobalDeclaration<'a>) {}

    fn visit_ts_module_declaration(&mut self, it: &TSModuleDeclaration<'a>) {
        if !it.declare && !matches!(it.id, TSModuleDeclarationName::StringLiteral(_)) {
            oxc_ast_visit::walk::walk_ts_module_declaration(self, it);
        }
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        if !it.declare {
            oxc_ast_visit::walk::walk_ts_enum_declaration(self, it);
        }
    }

    /// `checkImportDeclaration` and `checkImportBinding` under CommonJS emit:
    /// a namespace import asks for `__importStar`, a default import for
    /// `__importDefault`.
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let Some(specifiers) = it.specifiers.as_ref() else {
            return;
        };
        if !self.options.commonjs || it.phase.is_some() || type_only_import_clause_error(it) {
            return;
        }
        let namespace = specifiers
            .iter()
            .any(|specifier| matches!(specifier, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_)));
        if namespace {
            self.push(it.span, IMPORT_STAR);
        } else {
            for specifier in specifiers {
                if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier
                    && is_default(&specifier.imported)
                {
                    self.push_request(specifier.span, IMPORT_DEFAULT, Some(it.source.value.to_string()));
                }
            }
        }
        let default = specifiers
            .iter()
            .any(|specifier| matches!(specifier, ImportDeclarationSpecifier::ImportDefaultSpecifier(_)));
        if default && !namespace {
            self.push(it.span, IMPORT_DEFAULT);
        }
    }

    /// `checkExportSpecifier`: re-exporting a module's default goes through
    /// `__importDefault` under CommonJS emit.
    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if it.source.is_some() && self.options.commonjs {
            for specifier in &it.specifiers {
                if is_default(&specifier.local) {
                    self.push(specifier.span, IMPORT_DEFAULT);
                }
            }
        }
        if let Some(declaration) = &it.declaration {
            self.visit_declaration(declaration);
        }
    }

    /// `checkExportDeclaration`: `export * as ns` goes through `__importStar`
    /// and `export *` through `__exportStar` under CommonJS emit.
    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        if self.options.commonjs {
            self.push(it.span, if it.exported.is_some() { IMPORT_STAR } else { EXPORT_STAR });
        }
    }

    fn visit_export_default_declaration(&mut self, it: &ExportDefaultDeclaration<'a>) {
        match &it.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                self.export_default_start = Some(it.span.start);
                self.visit_function(function, ScopeFlags::Function);
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => self.visit_class(class),
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
            declaration => {
                if let Some(expression) = declaration.as_expression() {
                    self.visit_named_value(
                        expression,
                        NamedEvaluation { name: None, source: true, computed_key: false },
                    );
                }
            }
        }
    }

    fn visit_ts_export_assignment(&mut self, it: &TSExportAssignment<'a>) {
        self.visit_named_value(
            &it.expression,
            NamedEvaluation { name: None, source: true, computed_key: false },
        );
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        self.check_declaration_list(it, None);
    }

    fn visit_catch_parameter(&mut self, it: &CatchParameter<'a>) {
        self.check_variable_like(&it.pattern, it.type_annotation.is_some(), None, InitializerRole::Variable);
    }

    /// `checkForOfStatement`: `for await` in an async function goes through
    /// `__asyncValues` below ES2018; an expression target is checked after the
    /// iterated expression.
    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        if it.r#await
            && self.below(ES2018)
            && matches!(self.frames.last(), Some(Frame::Function(flags)) if flags.is_async && flags.has_body)
        {
            self.push(it.span, ASYNC_VALUES);
        }
        match &it.left {
            ForStatementLeft::VariableDeclaration(declaration) => {
                self.check_declaration_list(declaration, Some(&it.right));
            }
            left => {
                self.visit_expression(&it.right);
                if let Some(target) = left.as_assignment_target() {
                    self.check_assignment_target(target, AssignmentKind::Definite);
                }
            }
        }
        self.visit_statement(&it.body);
    }

    /// `checkForInStatement`: the object first, then the target.
    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.visit_expression(&it.right);
        match &it.left {
            ForStatementLeft::VariableDeclaration(declaration) => self.check_declaration_list(declaration, None),
            left => {
                if let Some(target) = left.as_assignment_target() {
                    self.check_assignment_target(target, AssignmentKind::Definite);
                }
            }
        }
        self.visit_statement(&it.body);
    }

    fn visit_function(&mut self, it: &Function<'a>, _: ScopeFlags) {
        let named = self.named.take();
        let export_default_start = self.export_default_start.take();
        if it.declare {
            return;
        }
        let flags = FunctionFlags { is_async: it.r#async, generator: it.generator, has_body: it.body.is_some() };
        let expression = it.is_expression();
        // A function expression reports at its name, else at the name it is
        // assigned to (`GetAssignedName`), else at its first token; an
        // anonymous `export default` function at the statement's.
        let span = match &it.id {
            Some(id) => id.span,
            None if expression => named
                .and_then(|named| named.name)
                .unwrap_or_else(|| self.first_token(it.span.start)),
            None => self.first_token(export_default_start.unwrap_or(it.span.start)),
        };
        self.frames.push(Frame::Function(flags));
        self.check_signature(flags, span, &it.params, false);
        if let Some(body) = &it.body {
            if expression {
                self.deferred(|walker| walker.visit_function_body(body));
            } else {
                self.visit_function_body(body);
            }
        }
        self.frames.pop();
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.named = None;
        let flags = FunctionFlags { is_async: it.r#async, generator: false, has_body: true };
        let span = self.arrow_error_span(it);
        self.frames.push(Frame::Function(flags));
        self.check_signature(flags, span, &it.params, false);
        self.deferred(|walker| walker.visit_function_body(&it.body));
        self.frames.pop();
    }

    /// `checkClassLikeDeclaration` (decorators, then the heritage expression)
    /// and the members: at once for a declaration, deferred for an
    /// expression, which then asks for what naming it takes.
    fn visit_class(&mut self, it: &Class<'a>) {
        let named = self.named.take();
        self.export_default_start = None;
        if it.declare {
            return;
        }
        let declaration = it.is_declaration();
        if (declaration || !self.options.legacy_decorators) && !it.decorators.is_empty() {
            let mut helpers = self.decorator_helpers(false);
            if declaration
                && !self.options.legacy_decorators
                && self.below(ESNEXT)
                && (it.id.is_none() || self.first_transformable_static_element(it).is_some())
            {
                helpers |= SET_FUNCTION_NAME;
            }
            self.check_decorators(&it.decorators, helpers);
        }
        if let Some(super_class) = &it.super_class {
            self.visit_expression(super_class);
        }
        if declaration {
            self.check_class_members(it);
        } else {
            self.deferred(|walker| walker.check_class_members(it));
            self.check_class_expression_helpers(it, named);
        }
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if it.computed {
            self.visit_property_key(&it.key);
        }
        let proto_setter = !it.computed
            && match &it.key {
                PropertyKey::StaticIdentifier(key) => key.name == "__proto__",
                PropertyKey::StringLiteral(key) => key.value == "__proto__",
                _ => false,
            };
        let source = matches!(it.kind, PropertyKind::Init) && !it.method && !it.shorthand && !proto_setter;
        let name = self.property_name_span(&it.key, it.computed);
        self.visit_named_value(&it.value, NamedEvaluation { name: Some(name), source, computed_key: it.computed });
    }

    /// `checkBinaryLikeExpression` for an assignment: a destructuring
    /// assignment checks its value before its pattern.
    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if it.operator.is_assign() && it.left.as_assignment_target_pattern().is_some() {
            self.visit_expression(&it.right);
            self.check_assignment_target(&it.left, AssignmentKind::Definite);
            return;
        }
        let definite = it.operator.is_assign() || it.operator.is_logical();
        let kind = if definite { AssignmentKind::Definite } else { AssignmentKind::Compound };
        self.check_assignment_target(&it.left, kind);
        match it.left.as_simple_assignment_target() {
            Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)) => self.visit_named_value(
                &it.right,
                NamedEvaluation { name: Some(identifier.span), source: definite, computed_key: false },
            ),
            Some(SimpleAssignmentTarget::StaticMemberExpression(member)) => self.visit_named_value(
                &it.right,
                NamedEvaluation { name: Some(member.property.span), source: false, computed_key: false },
            ),
            _ => self.visit_expression(&it.right),
        }
    }

    fn visit_update_expression(&mut self, it: &UpdateExpression<'a>) {
        self.check_simple_target(&it.argument, AssignmentKind::Compound);
    }

    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        self.check_private_field(it, AssignmentKind::None);
    }

    /// `checkInExpression`: `#name in x`, after both operands.
    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        self.visit_expression(&it.right);
        if self.transforms_private_names() {
            self.push(it.left.span, CLASS_PRIVATE_FIELD_IN);
        }
    }

    /// `checkYieldExpression`: `yield*` in an async generator goes through
    /// `tslib` below ES2018, before its operand is checked.
    fn visit_yield_expression(&mut self, it: &YieldExpression<'a>) {
        if it.delegate
            && self.below(ES2018)
            && self.containing_function().is_some_and(|flags| flags.is_async && flags.generator)
        {
            self.push(self.first_token(it.span.start), AWAIT | ASYNC_DELEGATOR | ASYNC_VALUES);
        }
        if let Some(argument) = &it.argument {
            self.visit_expression(argument);
        }
    }
}

/// `ClassElementOrClassElementParameterIsDecorated` under ECMAScript
/// decorators, which parameters cannot take.
fn member_is_decorated(member: &ClassElement<'_>) -> bool {
    match member {
        ClassElement::MethodDefinition(method) => {
            method.kind != MethodDefinitionKind::Constructor
                && method.value.body.is_some()
                && !method.decorators.is_empty()
        }
        ClassElement::PropertyDefinition(property) => {
            !property.decorators.is_empty()
                && !property.declare
                && !matches!(property.r#type, PropertyDefinitionType::TSAbstractPropertyDefinition)
        }
        ClassElement::AccessorProperty(property) => {
            !property.decorators.is_empty()
                && !matches!(property.r#type, AccessorPropertyType::TSAbstractAccessorProperty)
        }
        ClassElement::StaticBlock(_) | ClassElement::TSIndexSignature(_) => false,
    }
}

/// `checkGrammarImportClause` rejects a type-only import naming both a default
/// and named bindings, or marking a named binding type-only again, and the
/// checker then skips the import's bindings.
fn type_only_import_clause_error(import: &ImportDeclaration<'_>) -> bool {
    let Some(specifiers) = import.specifiers.as_ref().filter(|_| import.import_kind.is_type()) else {
        return false;
    };
    let default = specifiers
        .iter()
        .any(|specifier| matches!(specifier, ImportDeclarationSpecifier::ImportDefaultSpecifier(_)));
    let bindings = specifiers
        .iter()
        .any(|specifier| !matches!(specifier, ImportDeclarationSpecifier::ImportDefaultSpecifier(_)));
    let retyped = specifiers.iter().any(|specifier| {
        matches!(specifier, ImportDeclarationSpecifier::ImportSpecifier(specifier) if specifier.import_kind.is_type())
    });
    (default && bindings) || retyped
}

/// tsgo's `ModuleExportNameIsDefault`.
fn is_default(name: &ModuleExportName<'_>) -> bool {
    match name {
        ModuleExportName::IdentifierName(name) => name.name == "default",
        ModuleExportName::IdentifierReference(name) => name.name == "default",
        ModuleExportName::StringLiteral(name) => name.value == "default",
    }
}

fn skip_parentheses<'b, 'a>(mut expression: &'b Expression<'a>) -> &'b Expression<'a> {
    while let Expression::ParenthesizedExpression(inner) = expression {
        expression = &inner.expression;
    }
    expression
}

/// tsgo's `IsOuterExpression(node, OEKAll)`: parentheses, type assertions,
/// `!` and instantiation expressions.
fn skip_outer_expressions<'b, 'a>(mut expression: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        expression = match expression {
            Expression::ParenthesizedExpression(inner) => &inner.expression,
            Expression::TSAsExpression(inner) => &inner.expression,
            Expression::TSSatisfiesExpression(inner) => &inner.expression,
            Expression::TSNonNullExpression(inner) => &inner.expression,
            Expression::TSTypeAssertion(inner) => &inner.expression,
            Expression::TSInstantiationExpression(inner) => &inner.expression,
            _ => return expression,
        };
    }
}
