use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedExportDeclaration, ParsedExpression, ParsedJsxAttribute, ParsedJsxAttributeValueKind,
    ParsedJsxChild, ParsedJsxTag, ParsedNamedType, ParsedStatement, ParsedType,
    TextSpan as SyntaxTextSpan,
};
use surge_ts_types::fx::FxHashMap;
use surge_ts_types::{
    FunctionType, ObjectProperty, ObjectType, PropertyMap, Type, is_assignable_to, union_type,
};

use super::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type};
use super::expr::{evaluate_expression, source_display_name};
use crate::context::CheckerContext;
use crate::infer::{InferredExpression, map_parsed_type};
use crate::metrics::alloc_object_type;
use crate::spans::diagnostic_with_syntax_span;
use crate::program::ParsedProgramFile;
use crate::symbols::{SymbolTable, TypeDeclarationScope, TypeDeclarationTable};

/// Where a file's JSX looks up `IntrinsicElements`, `Element` and the other
/// `JSX` members (tsc's `getJsxNamespaceAt`).
#[derive(Clone, Debug)]
pub(crate) enum JsxNamespace {
    /// The members resolve as `<prefix>.<member>` from the file's own scope.
    Qualified(Arc<str>),
    /// The members resolve as `<prefix>.<member>` among a module's exports:
    /// the runtime a file imports implicitly, or the module behind a UMD
    /// global, neither of which the file's own scope binds.
    Declared {
        scope: Arc<TypeDeclarationScope>,
        prefix: Arc<str>,
    },
    /// tsc resolves a namespace whose members surge cannot see from this file:
    /// a UMD global factory namespace read from a module.
    Unmodelled,
    /// No JSX namespace at all: every member is tsc's error type.
    Missing,
}

/// A member of the JSX namespace (tsc's `getJsxType`).
enum JsxMember {
    Found(Type),
    /// tsc's error type: the namespace does not declare the member.
    Missing,
    Unmodelled,
}

/// tsc's `getJsxNamespace`: the root of the factory a tag compiles to — the
/// file's `@jsx` pragma, else `jsxFactory`, else `reactNamespace`, else
/// `React`. A fragment compiles to the fragment factory, so it reads `@jsxFrag`,
/// else `jsxFragmentFactory`, and the default namespace (never the file's
/// `@jsx` pragma) when neither is set.
fn jsx_factory_namespace_name(fragment: bool, ctx: &CheckerContext) -> String {
    let names = &ctx.options.jsx_factory_names;
    let default_namespace = || match names.factory.as_deref().filter(|factory| !factory.is_empty()) {
        Some(factory) => {
            surge_ts_syntax::jsx_entity_root(factory).unwrap_or_else(|| "React".to_string())
        }
        None => names
            .react_namespace
            .clone()
            .filter(|namespace| !namespace.is_empty())
            .unwrap_or_else(|| "React".to_string()),
    };
    if fragment {
        return ctx
            .jsx_factory_uses
            .fragment_pragma
            .clone()
            .or_else(|| {
                names
                    .fragment_factory
                    .as_deref()
                    .and_then(surge_ts_syntax::jsx_entity_root)
            })
            .unwrap_or_else(default_namespace);
    }
    ctx.jsx_factory_uses
        .factory_pragma
        .clone()
        .unwrap_or_else(default_namespace)
}

/// Reports the implicit factory reference a JSX tag makes (tsc's
/// `markJsxAliasReferenced`). Under `jsx: react` tsc resolves the factory
/// namespace as a value at every opening element, self-closing element, and
/// opening fragment — reporting at the tag name, or at the `<` of a fragment —
/// so a module that never imports it reports once per tag. A fragment also
/// resolves the file's element factory. `preserve` and `react-native` resolve
/// it without error reporting and the automatic runtime never names it, which
/// is why this is gated on the classic React mode alone — and skipped for a
/// file whose runtime import (an `@jsxImportSource` pragma) resolves.
pub(crate) fn check_jsx_factory_reference(
    fragment: bool,
    location_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.jsx_classic_react
        || matches!(
            ctx.jsx_namespace_modules
                .implicit_imports
                .get(ctx.file_name.as_str()),
            Some(JsxImplicitImport::Resolved(_))
        )
    {
        return;
    }

    let span = location_span.or(fallback_span);
    let namespace = jsx_factory_namespace_name(fragment, ctx);
    // `jsxFragmentFactory: "null"` names no binding.
    if !(fragment && namespace == "null") {
        report_jsx_factory_reference(&namespace, span, Diagnostic::ts2874(&namespace, ctx.file_name.clone()), symbols, ctx);
        // tsc's `getJSXFragmentType` resolves the fragment factory again, with
        // its own message, once per file.
        if fragment && !ctx.jsx_fragment_factory_resolved {
            ctx.jsx_fragment_factory_resolved = true;
            let not_found = Diagnostic::ts2879(&namespace, ctx.file_name.clone());
            report_jsx_factory_reference(&namespace, span, not_found, symbols, ctx);
        }
    }
    if fragment {
        let element_namespace = jsx_factory_namespace_name(false, ctx);
        if element_namespace != namespace {
            let not_found = Diagnostic::ts2874(&element_namespace, ctx.file_name.clone());
            report_jsx_factory_reference(&element_namespace, span, not_found, symbols, ctx);
        }
    }
}

/// `resolveName` of the factory namespace as a value: when nothing answers
/// it, `onFailedToResolveSymbol` with the JSX message standing in for "Cannot
/// find name".
fn report_jsx_factory_reference(
    name: &str,
    span: Option<SyntaxTextSpan>,
    not_found: Diagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if crate::checks::emit_value_position_reference_diagnostic(name, span, ctx)
        || crate::checks::expr::resolves_value_name(name, symbols, ctx)
    {
        return;
    }
    let diagnostic = crate::checks::expr::failed_value_name_diagnostic(
        name,
        crate::checks::expr::UnresolvedNameSite::Implicit,
        Some(not_found),
        symbols,
        ctx,
    );
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}

/// tsc's `checkJsxPreconditions`: with no `jsx` option at all, every opening
/// element, self-closing element, and opening fragment is TS17004 at its `<`.
pub(crate) fn check_jsx_preconditions(
    span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.jsx_emit_none {
        return;
    }
    let mut diagnostic = Diagnostic::ts17004(ctx.file_name.clone());
    if let Some(span) = span.or(fallback_span) {
        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
    }
    ctx.push(diagnostic);
}

/// tsc's `checkJsxFragment`: under a JSX transform, a fragment in a file whose
/// element factory is configured — the `jsxFactory` option or an `@jsx`
/// pragma — needs its fragment factory configured as well: TS17016 for the
/// option, TS17017 for the pragma.
pub(crate) fn check_jsx_fragment_factory(
    span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let names = &ctx.options.jsx_factory_names;
    let configured = |name: &Option<String>| name.as_deref().is_some_and(|name| !name.is_empty());
    let factory_option = configured(&names.factory);
    if !(ctx.options.jsx_classic_react || ctx.options.jsx_automatic_runtime)
        || !(factory_option || ctx.jsx_factory_uses.factory_pragma.is_some())
        || configured(&names.fragment_factory)
        || ctx.jsx_factory_uses.fragment_pragma.is_some()
    {
        return;
    }
    let diagnostic = if factory_option {
        Diagnostic::ts17016(ctx.file_name.clone())
    } else {
        Diagnostic::ts17017(ctx.file_name.clone())
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span.or(fallback_span)));
}

/// Checks a JSX element: resolves the tag to an intrinsic element or function
/// component, lowers attributes into a props object, and reports missing,
/// excess, and mistyped props plus basic `children` mismatches. Attribute and
/// tsc's `getJsxNamespaceAt`, resolved once per file.
pub(crate) fn jsx_namespace(ctx: &mut CheckerContext) -> JsxNamespace {
    if let Some((file_name, namespace)) = &ctx.jsx_namespace
        && file_name.as_ref() == ctx.file_name.as_str()
    {
        return namespace.clone();
    }
    let namespace = resolve_jsx_namespace(ctx);
    ctx.jsx_namespace = Some((Arc::from(ctx.file_name.as_str()), namespace.clone()));
    namespace
}

/// The implicitly imported runtime module's `JSX` when the file has one, else
/// the factory namespace's `JSX` when the factory resolves to a namespace that
/// has one, else the global `JSX`.
fn resolve_jsx_namespace(ctx: &CheckerContext) -> JsxNamespace {
    let modules = ctx.jsx_namespace_modules.clone();
    if let Some(JsxImplicitImport::Resolved(jsx)) =
        modules.implicit_imports.get(ctx.file_name.as_str())
    {
        return match jsx {
            Some(exports) => exported_jsx_namespace(exports),
            None => global_jsx_namespace(ctx),
        };
    }
    let factory = jsx_factory_namespace_name(false, ctx);
    let qualified = format!("{factory}.JSX");
    if namespace_is_visible(&qualified, ctx) {
        return JsxNamespace::Qualified(qualified.into());
    }
    // A UMD global resolves as a namespace from a module too; only its value
    // is off limits there (TS2686).
    if ctx.is_umd_global_value_reference(&factory) {
        return match modules.umd_globals.get(factory.as_str()) {
            Some(Some(exports)) => exported_jsx_namespace(exports),
            Some(None) => global_jsx_namespace(ctx),
            None => JsxNamespace::Unmodelled,
        };
    }
    global_jsx_namespace(ctx)
}

/// The `JSX` namespace among a module's exports.
fn exported_jsx_namespace(exports: &Arc<TypeDeclarationTable>) -> JsxNamespace {
    JsxNamespace::Declared {
        scope: Arc::new(TypeDeclarationScope::new(vec![exports.clone()])),
        prefix: "JSX".into(),
    }
}

/// tsc's JSX global fallback.
fn global_jsx_namespace(ctx: &CheckerContext) -> JsxNamespace {
    if global_jsx_namespace_exists(ctx) {
        JsxNamespace::Qualified("JSX".into())
    } else {
        JsxNamespace::Missing
    }
}

/// The modules JSX namespaces are read from that no lexical scope exposes:
/// the runtime each file imports implicitly under the automatic runtime
/// (tsc's `getJsxNamespaceContainerForImplicitImport`), and the module behind
/// each UMD global a factory can name.
#[derive(Debug, Default)]
pub(crate) struct JsxNamespaceModules {
    implicit_imports: FxHashMap<Arc<str>, JsxImplicitImport>,
    /// Each UMD global's module exports, when they hold a `JSX` namespace.
    umd_globals: FxHashMap<Arc<str>, Option<Arc<TypeDeclarationTable>>>,
}

/// How a file's implicit JSX runtime import resolved.
#[derive(Debug)]
enum JsxImplicitImport {
    /// The runtime module's exports, when they hold a `JSX` namespace.
    Resolved(Option<Arc<TypeDeclarationTable>>),
    /// Nothing answers the specifier (TS2875).
    NotFound(String),
    /// The specifier names a file surge has no declarations for.
    Untyped,
}

/// Resolves every file's implicit JSX runtime import the way its written
/// imports resolve, and records each `export as namespace` module's exports.
pub(crate) fn collect_jsx_namespace_modules(
    parsed_files: &[ParsedProgramFile],
    module_export_tables: &[Option<crate::modules::ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) -> JsxNamespaceModules {
    let mut modules = JsxNamespaceModules::default();
    // Many files share one runtime module; its exports are scanned once.
    let mut jsx_exports: FxHashMap<usize, bool> = FxHashMap::default();
    let mut with_jsx = |exports: &Arc<TypeDeclarationTable>| {
        let declares_jsx = *jsx_exports
            .entry(Arc::as_ptr(exports) as usize)
            .or_insert_with(|| exports.iter().any(|(key, _)| key.starts_with("JSX.")));
        declares_jsx.then(|| exports.clone())
    };
    for (parsed_file, export_table) in parsed_files.iter().zip(module_export_tables) {
        let Some(export_table) = export_table.as_ref().filter(|_| parsed_file.is_module) else {
            continue;
        };
        for statement in &parsed_file.statements {
            if let ParsedStatement::ExportDeclaration(export) = statement
                && let ParsedExportDeclaration::NamespaceExport { exported_name, .. } =
                    export.as_ref()
            {
                if !modules.umd_globals.contains_key(exported_name.as_str()) {
                    let jsx = with_jsx(&export_table.type_declarations);
                    modules.umd_globals.insert(exported_name.as_str().into(), jsx);
                }
            }
        }
    }

    let runtime_options = surge_ts_syntax::JsxRuntimeOptions {
        automatic: ctx.options.jsx_automatic_runtime,
        development: ctx.options.jsx_factory_names.development,
        import_source: ctx.options.jsx_factory_names.import_source.clone(),
    };
    let saved_file_name = ctx.file_name.clone();
    for parsed_file in parsed_files {
        let Some(specifier) = surge_ts_syntax::jsx_runtime_import(
            &parsed_file.file_name,
            &parsed_file.jsx_factory_uses,
            &runtime_options,
        ) else {
            continue;
        };
        ctx.set_file_name(parsed_file.file_name.clone());
        let resolution = match crate::modules::try_resolve_module(
            &specifier,
            ctx,
            parsed_files,
            module_export_tables,
            module_resolution_scopes,
        ) {
            Some((export_table, _, _)) => {
                JsxImplicitImport::Resolved(with_jsx(&export_table.type_declarations))
            }
            None if ctx
                .options
                .resolved_module_for(&parsed_file.file_name, &specifier)
                .is_some() =>
            {
                JsxImplicitImport::Untyped
            }
            None => JsxImplicitImport::NotFound(specifier),
        };
        modules
            .implicit_imports
            .insert(parsed_file.file_name.as_str().into(), resolution);
    }
    ctx.set_file_name(saved_file_name);
    modules
}

/// tsc's `getJsxNamespaceContainerForImplicitImport` error: a runtime import
/// nothing answers is reported once per file, at its first tag.
pub(crate) fn check_jsx_runtime_import(ctx: &mut CheckerContext) {
    let Some(first_tag) = ctx.jsx_factory_uses.first_tag else {
        return;
    };
    let modules = ctx.jsx_namespace_modules.clone();
    let Some(JsxImplicitImport::NotFound(specifier)) =
        modules.implicit_imports.get(ctx.file_name.as_str())
    else {
        return;
    };
    if ctx.options.stub_external_modules && crate::modules::is_external_specifier(specifier) {
        return;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2875(specifier, ctx.file_name.clone()),
        Some(first_tag),
    ));
}

/// The JSX member names tsc reads; one of them resolving is the cheap proof
/// that a namespace exists before the declaration tables are scanned.
const JSX_MEMBER_NAMES: [&str; 9] = [
    "IntrinsicElements",
    "Element",
    "ElementClass",
    "ElementAttributesProperty",
    "ElementChildrenAttribute",
    "IntrinsicAttributes",
    "IntrinsicClassAttributes",
    "LibraryManagedAttributes",
    "ElementType",
];

/// Whether `name` resolves as a namespace from the current file: a namespace
/// block surge registered, or a type declared beneath it in any table the
/// file's type lookups consult (an import copies its module's members in under
/// the local name).
fn namespace_is_visible(name: &str, ctx: &CheckerContext) -> bool {
    if ctx.namespace_info(name).is_some() {
        return true;
    }
    if JSX_MEMBER_NAMES
        .iter()
        .any(|member| ctx.lookup_type_declaration(&format!("{name}.{member}")).is_some())
    {
        return true;
    }
    let heads = |key: &Arc<str>| {
        key.strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('.'))
    };
    ctx.type_declarations.iter().any(|(key, _)| heads(key))
        || ctx.type_declaration_scope.as_ref().is_some_and(|scope| {
            scope
                .layers()
                .iter()
                .any(|layer| layer.iter().any(|(key, _)| heads(key)))
        })
        || ctx
            .ambient_global_type_declarations
            .iter()
            .any(|(key, _)| heads(key))
}

/// tsc's JSX global fallback (`getGlobalSymbol(JSX, Namespace)`): only a
/// global `JSX` counts, never one a module declares for itself.
fn global_jsx_namespace_exists(ctx: &CheckerContext) -> bool {
    ctx.namespace_registry.is_global("JSX")
        || JSX_MEMBER_NAMES.iter().any(|member| {
            ctx.ambient_global_type_declarations
                .get(&format!("JSX.{member}"))
                .is_some()
        })
}

/// tsc's `getJsxType`: the declared type of the JSX namespace's `member`.
fn jsx_namespace_member(member: &str, ctx: &mut CheckerContext) -> JsxMember {
    let (prefix, scope) = match jsx_namespace(ctx) {
        JsxNamespace::Qualified(prefix) => (prefix, None),
        JsxNamespace::Declared { scope, prefix } => (prefix, Some(scope)),
        JsxNamespace::Unmodelled => return JsxMember::Unmodelled,
        JsxNamespace::Missing => return JsxMember::Missing,
    };
    let name = format!("{prefix}.{member}");
    let resolve = |ctx: &mut CheckerContext| {
        if ctx.lookup_type_declaration(&name).is_none() {
            return JsxMember::Missing;
        }
        JsxMember::Found(map_parsed_type(
            ParsedType::Named(Arc::new(ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: name.clone(),
                span: None,
                type_arguments: Vec::new(),
            })),
            ctx,
        ))
    };
    match scope {
        Some(scope) => crate::infer::with_type_declaration_scope(&Some(scope), ctx, resolve),
        None => resolve(ctx),
    }
}

/// tsc's `getJsxElementTypeAt`: a JSX element evaluates to the namespace's
/// `Element`, and to tsc's error type when the namespace declares none — a
/// fragment to `any` then (`checkJsxFragment`). `None` when surge cannot see
/// the namespace's `Element`.
pub(crate) fn jsx_expression_type(fragment: bool, ctx: &mut CheckerContext) -> Option<Type> {
    match jsx_namespace_member("Element", ctx) {
        JsxMember::Found(element) if !is_unmodelled_member(&element) => Some(element),
        JsxMember::Missing => Some(if fragment { Type::Any } else { Type::ErrorType }),
        JsxMember::Found(_) | JsxMember::Unmodelled => None,
    }
}

/// One candidate's props as tsc's JSX signature resolution relates the
/// attributes to them.
struct JsxProps {
    target: Type,
    /// The intersection's constituents, the namespace's intrinsic attributes
    /// first, when the props are joined with them.
    constituents: Vec<Type>,
    /// tsc's `reportErrorResults` names only the failing constituent, not the
    /// whole intersection, when that holds the namespace's intrinsic
    /// attributes and the namespace declares both kinds of them.
    reports_constituent: bool,
    /// What the candidate's signature returns — a class component's instance —
    /// and whether it constructs, which `checkJsxReturnAssignableToAppropriateBound`
    /// relates to the namespace's element types.
    returns: Option<(Type, bool)>,
}

impl JsxProps {
    fn plain(target: Type) -> Self {
        JsxProps {
            target,
            constituents: Vec::new(),
            reports_constituent: false,
            returns: None,
        }
    }
}

/// What a tag's attributes are checked against.
enum PropsResolution {
    /// tsc's `resolveCall` over the tag's JSX signatures: the props each
    /// candidate relates the attributes to, in declaration order, and the type
    /// that contextually types the attributes.
    Props {
        contextual: Type,
        candidates: Vec<JsxProps>,
        /// What the element resolves to when no candidate matches (tsc's
        /// `createUnionOfSignaturesForOverloadFailure`): one signature returning
        /// the intersection of the overloads' return types, and whether it
        /// constructs. Generic overloads are never combined
        /// (`getCandidateForOverloadFailure`).
        failure_returns: Option<(Type, bool)>,
    },
    /// tsc's error type or a non-component value: nothing to check against,
    /// and no contextual type for a callback attribute — tsc reports its
    /// parameters implicitly `any`.
    Unchecked,
    /// The props exist for tsc but surge could not model them, so an
    /// implicit-any report inside the attributes would describe that gap.
    Unmodelled,
}

/// Checks a JSX element the way tsc's `checkJsxOpeningLikeElementOrOpeningFragment`
/// resolves it: the tag to an intrinsic element or a component's signatures,
/// the attributes and children into one attributes object contextually typed by
/// the props, and that object related to each candidate's props. Attribute and
/// child expressions are always evaluated for ordinary diagnostics (e.g. an
/// unresolved name in `{expr}`), even when the tag itself does not resolve, so a
/// missing component never cascades into a prop-checking storm.
pub(crate) fn check_jsx_element(
    tag_name: &str,
    tag: &ParsedJsxTag,
    attributes: &[ParsedJsxAttribute],
    children: &[ParsedJsxChild],
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let tag_name_span = tag.name_span.or(fallback_span);
    check_grammar_jsx_element(tag_name, tag_name_span, attributes, ctx);
    let has_children = children.iter().any(is_semantic_child);
    // Which attribute the body forms decides what the attributes object is;
    // when surge cannot tell, the element's props cannot be judged.
    // `elaborateJsxComponents` checks the body under `children` even when
    // the namespace names no property for it to form.
    let (children_name, body_name, children_unmodelled) = match jsx_children_property_name(ctx) {
        ChildrenPropertyName::Name(name) => (Some(name.clone()), Some(name), false),
        ChildrenPropertyName::Missing => (None, Some("children".to_string()), false),
        ChildrenPropertyName::Empty => (None, None, false),
        ChildrenPropertyName::Unmodelled => (None, None, has_children),
    };
    let mut value_tag_type = None;
    let resolution = match &tag.expression {
        Some(expression) => resolve_component_props(
            &JsxCallSite {
                tag_expression: expression,
                type_arguments: &tag.type_arguments,
                type_arguments_span: tag.type_arguments_span,
                attributes,
                children: has_children.then_some(children).unwrap_or_default(),
                children_name: children_name.as_deref(),
                tag_name,
                // tsc reports on the opening element.
                element_span: tag.opening_span.or(tag.span).or(fallback_span),
            },
            tag_name_span,
            &mut value_tag_type,
            symbols,
            ctx,
        ),
        None => {
            check_intrinsic_type_arguments(tag, ctx);
            intrinsic_element_props(tag_name, tag.opening_span.or(fallback_span), ctx)
        }
    };
    let (contextual, candidates, failure_returns, unmodelled_props) = match resolution {
        PropsResolution::Props { .. } | PropsResolution::Unmodelled if children_unmodelled => {
            (None, Vec::new(), None, true)
        }
        PropsResolution::Props {
            contextual,
            candidates,
            failure_returns,
        } => (Some(contextual), candidates, failure_returns, false),
        PropsResolution::Unchecked => (None, Vec::new(), None, false),
        PropsResolution::Unmodelled => (None, Vec::new(), None, true),
    };
    let overloaded = candidates.len() > 1;
    let contextual = contextual.map(|contextual| {
        discriminate_by_attributes(
            &contextual,
            attributes,
            has_children.then_some(children_name.as_deref()).flatten(),
            symbols,
            ctx,
        )
    });

    // A component whose props type collapsed to the degradation sentinel offers
    // no contextual type for an inline callback attribute, so any implicit-any
    // report inside those attributes would describe surge's modelling gap rather
    // than the source. Suppress it for the element's attributes and children, the
    // same no-cascade rule a sentinel receiver gets elsewhere.
    if unmodelled_props {
        ctx.unmodelled_jsx_props_depth += 1;
    }
    // Overload candidates are tried against the attributes one at a time, so a
    // value is only contextually typed while they are evaluated; the report is
    // the chosen candidate's, or TS2769.
    let value_diagnostic = if overloaded {
        ExpectedTypeDiagnostic::ContextOnly
    } else {
        ExpectedTypeDiagnostic::TypeNotAssignable
    };
    let evaluated = evaluate_attributes(
        attributes,
        contextual.as_ref(),
        value_diagnostic,
        fallback_span,
        symbols,
        ctx,
    );
    let children_attribute = evaluate_children(
        children,
        &tag.child_spans,
        contextual.as_ref(),
        children_name.as_deref(),
        fallback_span,
        symbols,
        ctx,
    );
    if unmodelled_props {
        ctx.unmodelled_jsx_props_depth -= 1;
    }

    let body = body_name.filter(|_| has_children).map(|name| JsxBody {
        name,
        in_attributes: children_name.is_some(),
        children: children_attribute,
    });
    let site = RelationSite {
        tag_name,
        tag_name_span,
    };
    match candidates.as_slice() {
        [] => {}
        [props] => {
            for (diagnostic, span) in relate_attributes(&evaluated, body.as_ref(), props, &site, ctx) {
                ctx.push(diagnostic_with_syntax_span(diagnostic, span));
            }
            check_jsx_return_bound(props, &site, ctx);
        }
        [.., last] => {
            let chosen = candidates.iter().find(|props| {
                relate_attributes(&evaluated, body.as_ref(), props, &site, ctx).is_empty()
            });
            match chosen {
                Some(props) => check_jsx_return_bound(props, &site, ctx),
                None => {
                    // tsc reports the last candidate's errors, each as "No overload
                    // matches this call".
                    for (diagnostic, span) in relate_attributes(&evaluated, body.as_ref(), last, &site, ctx) {
                        let mut chain = vec!["  The last overload gave the following error.".to_string()];
                        chain.extend(diagnostic.message.lines().map(|line| format!("    {line}")));
                        ctx.push(diagnostic_with_syntax_span(
                            crate::checks::relation_chain::with_chain(Diagnostic::ts2769(ctx.file_name.clone()), &chain),
                            span,
                        ));
                    }
                    if let Some(returns) = failure_returns {
                        let failure = JsxProps {
                            returns: Some(returns),
                            ..JsxProps::plain(Type::Any)
                        };
                        check_jsx_return_bound(&failure, &site, ctx);
                    }
                }
            }
        }
    }

    // tsc's `checkJsxOpeningLikeElementOrOpeningFragment`: an intrinsic tag is
    // judged as its name's string literal type, a value tag as its type (an
    // unresolved one is tsc's error type, which fits).
    match &value_tag_type {
        Some(tag_type) => check_jsx_element_type(tag_type, &site, ctx),
        None if tag.expression.is_none() => {
            check_jsx_element_type(&Type::StringLiteral(tag_name.to_string()), &site, ctx);
        }
        None => {}
    }

    if let Some(closing) = &tag.closing {
        let span = closing.span.or(fallback_span);
        match &closing.expression {
            Some(expression) => {
                let _ = evaluate_expression(expression, span, symbols, ctx);
            }
            None => {
                let _ = intrinsic_element_props(tag_name, span, ctx);
            }
        }
    }
}

/// tsc's `checkGrammarJsxElement`, which stops at its first error: a
/// namespaced tag whose namespace is no intrinsic name under a JSX transform
/// (TS2639), else, attribute by attribute, a repeated name (TS17001) or a
/// value written as an empty `{}` (TS17000).
fn check_grammar_jsx_element(
    tag_name: &str,
    tag_name_span: Option<SyntaxTextSpan>,
    attributes: &[ParsedJsxAttribute],
    ctx: &mut CheckerContext,
) {
    if let Some((namespace, _)) = tag_name.split_once(':')
        && (ctx.options.jsx_classic_react || ctx.options.jsx_automatic_runtime)
        && !is_intrinsic_jsx_name(namespace)
    {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2639(ctx.file_name.clone()),
            tag_name_span,
        ));
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for attribute in attributes {
        if attribute.name.is_empty() {
            continue;
        }
        if !seen.insert(attribute.name.as_str()) {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts17001(ctx.file_name.clone()),
                attribute.name_span,
            ));
            return;
        }
        if matches!(attribute.value_kind, ParsedJsxAttributeValueKind::Expression)
            && attribute.value.is_none()
        {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts17000(ctx.file_name.clone()),
                attribute.value_span,
            ));
            return;
        }
    }
}

/// tsc's `scanner.IsIntrinsicJsxName`.
fn is_intrinsic_jsx_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase) || name.contains('-')
}

/// A value tag seen as the call tsc resolves it as: the attributes (with the
/// body's children) are its single argument.
struct JsxCallSite<'a> {
    tag_expression: &'a ParsedExpression,
    type_arguments: &'a [ParsedType],
    type_arguments_span: Option<SyntaxTextSpan>,
    attributes: &'a [ParsedJsxAttribute],
    /// The children when the body has semantic ones.
    children: &'a [ParsedJsxChild],
    children_name: Option<&'a str>,
    /// The tag as written, which TS2604 names.
    tag_name: &'a str,
    /// The opening element, where tsc reports what the tag resolves against
    /// rather than the tag itself.
    element_span: Option<SyntaxTextSpan>,
}

/// The props a value tag's attributes are checked against (tsc's
/// `resolveJsxOpeningLikeElement` for a non-intrinsic tag). Evaluating the tag
/// reports what an unresolved name or member reports anywhere else.
fn resolve_component_props(
    site: &JsxCallSite<'_>,
    span: Option<SyntaxTextSpan>,
    tag_type: &mut Option<Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> PropsResolution {
    let component_type = match evaluate_expression(site.tag_expression, span, symbols, ctx) {
        InferredExpression::Known(component_type) => component_type,
        // A value surge could not type at all is its own gap: tsc has the
        // component's props and types the callbacks from them.
        InferredExpression::Unknown => return PropsResolution::Unmodelled,
        // An unresolved name or member is tsc's error type. Its
        // attributes get no contextual signature (`getContextualSignature`
        // of `any` is undefined), so tsc *does* report a callback in them
        // as implicit `any` — leave them unsuppressed.
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. } => return PropsResolution::Unchecked,
    };
    *tag_type = Some(component_type.clone());
    check_class_component_type_arguments(site, span, symbols, ctx);
    let Some((signatures, construct)) = jsx_signatures(&component_type) else {
        if component_is_unmodelled(&component_type) {
            return PropsResolution::Unmodelled;
        }
        if let Type::StringLiteral(tag) = component_type.peeled() {
            return string_literal_tag_props(&tag, site, span, ctx);
        }
        if check_mixed_component_union(&component_type, site, span, symbols, ctx) {
            return PropsResolution::Unchecked;
        }
        if has_no_jsx_signatures(&component_type, ctx) {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2604(site.tag_name, ctx.file_name.clone()),
                span,
            ));
        }
        return PropsResolution::Unchecked;
    };
    if signatures.len() > 1 || signatures.iter().any(|signature| signature.type_parameter_head().is_some()) {
        ctx.mark_check_uncertain();
    }
    let signatures = match signatures.as_slice() {
        [signature] if !construct => match instantiate_jsx_signature(signature, site, symbols, ctx) {
            Some(signature) => vec![signature],
            // tsc instantiates the props with what inference found; surge
            // found nothing, so props that name the type parameters are
            // unknown to it. Props that do not are the same either way.
            None if signature
                .parameters()
                .first()
                .is_some_and(|props| mentions_type_parameter(&props.peeled())) =>
            {
                return PropsResolution::Unmodelled;
            }
            None => signatures,
        },
        _ => signatures,
    };
    let mut candidates = Vec::with_capacity(signatures.len());
    for signature in &signatures {
        let declared = if construct {
            class_component_props(signature, &component_type, site, symbols, ctx)
        } else {
            let props = signature
                .parameters()
                .first()
                .cloned()
                .unwrap_or(Type::GenuineUnknown);
            managed_attributes(&component_type, props, ctx)
        };
        // Props typed by a type variable of the body being checked take
        // nothing but that variable (`type_variable_attributes_failure`);
        // intersected with the namespace's own attributes they are not modelled.
        let type_variable_props = !construct && declared.peeled().is_type_variable();
        if is_unmodelled_props(&declared) && !type_variable_props {
            return PropsResolution::Unmodelled;
        }
        let mut props = with_intrinsic_attributes(
            declared,
            construct.then(|| signature.return_type()),
            ctx,
        );
        if type_variable_props && !props.target.peeled().is_type_variable() {
            return PropsResolution::Unmodelled;
        }
        props.returns = Some((signature.return_type().clone(), construct));
        candidates.push(props);
    }
    let contextual = match candidates.as_slice() {
        [single] => single.target.clone(),
        _ => union_type(candidates.iter().map(|props| props.target.clone()).collect()),
    };
    let failure_returns = (signatures.len() > 1
        && signatures
            .iter()
            .all(|signature| signature.type_parameter_head().is_none()))
    .then(|| {
        let returns = signatures
            .iter()
            .map(|signature| signature.return_type().clone())
            .collect();
        (crate::infer::types::merge_intersection_members(returns), construct)
    });
    PropsResolution::Props {
        contextual,
        candidates,
        failure_returns,
    }
}

/// tsc's `getIntrinsicAttributesTypeFromStringLiteralType`: a tag whose value
/// is a string literal (`const Tag: "h1"`) resolves to the intrinsic element it
/// names through a signature whose props carry the intrinsic attributes. When
/// `IntrinsicElements` declares no such element the element is TS2339 and the
/// tag, left with no signature, TS2604; with no `IntrinsicElements` at all the
/// props are `any`, which contextually types nothing.
fn string_literal_tag_props(
    tag: &str,
    site: &JsxCallSite<'_>,
    tag_name_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> PropsResolution {
    let intrinsic_elements = match jsx_namespace_member("IntrinsicElements", ctx) {
        JsxMember::Found(intrinsic_elements) => intrinsic_elements,
        JsxMember::Missing => return PropsResolution::Unchecked,
        JsxMember::Unmodelled => return PropsResolution::Unmodelled,
    };
    let Type::Object(object) = intrinsic_elements.peeled() else {
        return PropsResolution::Unmodelled;
    };
    let props = if let Some(property_type) = object.get_property_type(tag) {
        property_type.clone()
    } else if object.synthetic_open_index {
        return PropsResolution::Unmodelled;
    } else if let Some(index_type) = object.string_index_type.as_deref() {
        index_type.clone()
    } else {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2339(tag, "JSX.IntrinsicElements", ctx.file_name.clone()),
            site.element_span,
        ));
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2604(site.tag_name, ctx.file_name.clone()),
            tag_name_span,
        ));
        return PropsResolution::Unchecked;
    };
    if is_unmodelled_props(&props) {
        return PropsResolution::Unmodelled;
    }
    let props = with_intrinsic_attributes(props, None, ctx);
    PropsResolution::Props {
        contextual: props.target.clone(),
        candidates: vec![props],
        failure_returns: None,
    }
}

/// tsc's `resolveJsxOpeningLikeElement` finding no signature to resolve: a
/// primitive other than `string`, a union of primitives one of which has none,
/// or an object — an array included — with neither construct nor call
/// signatures that is no subtype of the global `Function` (a call through
/// which is untyped). Anything surge left open is not judged.
fn has_no_jsx_signatures(component_type: &Type, ctx: &mut CheckerContext) -> bool {
    let primitive = |ty: &Type| {
        matches!(
            ty,
            Type::String
                | Type::Number
                | Type::NumberLiteral(_)
                | Type::Boolean
                | Type::BooleanLiteral(_)
                | Type::BigInt
                | Type::Symbol
        )
    };
    match component_type.peeled() {
        Type::Union(union) => {
            union.types().iter().all(primitive)
                && union.types().iter().any(|member| !matches!(member, Type::String))
        }
        Type::String => false,
        ty if primitive(&ty) => true,
        Type::Array(_) | Type::Tuple(_) => true,
        Type::Object(object) => {
            !object.synthetic_open_index
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !is_function_subtype(&Type::Object(object), ctx)
        }
        _ => false,
    }
}

/// `isTypeSubtypeOf(tagType, globalFunctionType)`, as assignability answers it.
fn is_function_subtype(ty: &Type, ctx: &mut CheckerContext) -> bool {
    if ctx.lookup_type_declaration("Function").is_none() {
        return false;
    }
    let function = map_parsed_type(named_type("Function", Vec::new()), ctx);
    !function.is_unknown() && is_assignable_to(ty, &function)
}

/// `resolveJsxOpeningLikeElement` on an intrinsic tag: its type arguments are
/// checked for what they name, and any at all are one too many.
fn check_intrinsic_type_arguments(tag: &ParsedJsxTag, ctx: &mut CheckerContext) {
    if tag.type_arguments.is_empty() {
        return;
    }
    for type_argument in &tag.type_arguments {
        let _ = map_parsed_type(type_argument.clone(), ctx);
    }
    let span = tag.type_arguments_span.map(|span| SyntaxTextSpan { start: span.start + 1, end: span.end - 1 });
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2558("0".to_string(), tag.type_arguments.len(), ctx.file_name.clone()),
        span,
    ));
}

/// `resolveCall` over a class component's construct signatures, which all
/// take the class's own type parameters: written type arguments outside their
/// count are TS2558, and within it the first one outside its constraint is
/// TS2344. Judged only for a class declared in a source file, as
/// `new C<…>()` is.
fn check_class_component_type_arguments(
    site: &JsxCallSite<'_>,
    tag_name_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if site.type_arguments.is_empty() {
        return;
    }
    let ParsedExpression::Identifier { name, .. } = site.tag_expression else {
        return;
    };
    let constructs = symbols.get(name).is_some_and(|symbol| match &symbol.ty {
        Type::Any => true,
        Type::Object(object) => object.construct_signature().is_some(),
        _ => false,
    });
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(name)
    else {
        return;
    };
    if !constructs || !info.is_class_instance || crate::modules::is_declaration_file_name(&info.file_name) {
        return;
    }
    let declared_here = *info.file_name == *ctx.file_name;
    let type_parameters = info.body.type_parameters.clone();
    let maximum = type_parameters.len();
    let minimum = type_parameters
        .iter()
        .filter(|parameter| parameter.default_type.is_none())
        .count();
    let count = site.type_arguments.len();
    if count < minimum || count > maximum {
        let expected = if minimum == maximum {
            maximum.to_string()
        } else {
            format!("{minimum}-{maximum}")
        };
        let span = site
            .type_arguments_span
            .map(|span| SyntaxTextSpan { start: span.start + 1, end: span.end - 1 });
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2558(expected, count, ctx.file_name.clone()),
            span,
        ));
        return;
    }
    if declared_here {
        let _ = crate::checks::call::construct::violates_type_parameter_constraint(
            &type_parameters,
            site.type_arguments,
            tag_name_span,
            ctx,
        );
    }
}

/// tsc's `inferJsxTypeArguments`: a generic component's type arguments are
/// inferred from the attributes object, read as the call's one argument, and
/// explicit type arguments on the tag are taken as written. `None` when the
/// signature is generic but could not be instantiated.
fn instantiate_jsx_signature(
    signature: &FunctionType,
    site: &JsxCallSite<'_>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    let declared = signature.declaration().and_then(|declaration| {
        if let Some(member) = declaration.downcast_ref::<crate::checks::call::DeclaredMemberSignature>() {
            return Some((member.signature.clone(), member.outer_type_arguments.clone()));
        }
        declaration
            .downcast_ref::<crate::symbols::FunctionSignatureInfo>()
            .map(|info| (Arc::new(info.clone()), Vec::new()))
    });
    let declared = declared.or_else(|| match site.tag_expression {
        ParsedExpression::Identifier { name, .. } => symbols
            .get(name)
            .and_then(|symbol| symbol.function_signature.clone())
            .map(|info| (info, Vec::new())),
        _ => None,
    });
    let Some((info, outer_type_arguments)) = declared else {
        return Some(signature.clone());
    };
    if info.overloaded {
        ctx.mark_check_uncertain();
    }
    if info.type_parameters.is_empty() || info.overloaded {
        return Some(signature.clone());
    }
    let argument = lone_type_variable_spread(site, symbols, ctx).unwrap_or_else(|| jsx_attributes_argument(site));
    match crate::checks::call::instantiate_function_type(
        signature,
        Some(&info),
        &outer_type_arguments,
        site.type_arguments,
        site.type_arguments_span,
        std::slice::from_ref(&argument),
        None,
        symbols,
        ctx,
    ) {
        std::borrow::Cow::Owned(instantiated) => Some(instantiated),
        std::borrow::Cow::Borrowed(_) => None,
    }
}

/// Whether a type still names an uninstantiated type parameter anywhere.
fn mentions_type_parameter(ty: &Type) -> bool {
    match ty {
        Type::TypeParameter(_) => true,
        Type::Reference(reference) => reference.arguments.iter().any(mentions_type_parameter),
        Type::Union(union) => union.types().iter().any(mentions_type_parameter),
        Type::Array(element) => mentions_type_parameter(element),
        Type::Tuple(elements) => elements.iter().any(mentions_type_parameter),
        Type::Function(function) => {
            function.parameters().iter().any(mentions_type_parameter)
                || mentions_type_parameter(function.return_type())
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| mentions_type_parameter(&property.ty))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(mentions_type_parameter)
                || object
                    .intersection_operands
                    .as_deref()
                    .is_some_and(|operands| operands.iter().any(mentions_type_parameter))
        }
        _ => false,
    }
}

/// The attributes object as an object literal argument: each attribute a
/// property, each spread a spread, and a lone child as the children.
/// `createJsxAttributesTypeFromAttributesProperty`: attributes that are one
/// spread of a type variable, with no children, are that type itself
/// (`getSpreadType` keeps a generic object type), which inference reads as is.
fn lone_type_variable_spread(
    site: &JsxCallSite<'_>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<surge_ts_syntax::ParsedCallArgument> {
    let [attribute] = site.attributes else {
        return None;
    };
    if !attribute.name.is_empty() || site.children.iter().any(is_semantic_child) {
        return None;
    }
    let value = attribute.value.as_ref()?;
    let reported = ctx.diagnostics().len();
    let ty = crate::infer::infer_expression(value, symbols, ctx);
    ctx.truncate_diagnostics(reported);
    matches!(&ty, InferredExpression::Known(ty) if ty.is_type_variable()).then(|| surge_ts_syntax::ParsedCallArgument {
        expression: value.clone(),
        span: attribute.value_span,
        spread: false,
        expression_span: attribute.value_span,
    })
}

fn jsx_attributes_argument(site: &JsxCallSite<'_>) -> surge_ts_syntax::ParsedCallArgument {
    let property = |name: &str, name_span, value: ParsedExpression, value_span, is_spread| {
        surge_ts_syntax::ParsedObjectProperty {
            written_name: None,
            name: name.to_string(),
            name_span,
            value,
            value_span,
            span: name_span.or(value_span),
            is_method: false,
            is_spread,
            is_shorthand: false,
            is_accessor: false,
            is_getter: false,
            computed_key: None,
            paired_setter: None,
            unnamed_key_value: None,
            member_key: None,
        }
    };
    let mut properties = Vec::with_capacity(site.attributes.len() + 1);
    for attribute in site.attributes {
        if attribute.name.is_empty() {
            if let Some(value) = &attribute.value {
                properties.push(property("", None, value.clone(), attribute.value_span, true));
            }
            continue;
        }
        let value = match &attribute.value_kind {
            ParsedJsxAttributeValueKind::StringLiteral(value) => {
                ParsedExpression::StringLiteral(value.clone())
            }
            ParsedJsxAttributeValueKind::BooleanShorthand => ParsedExpression::BooleanLiteral(true),
            ParsedJsxAttributeValueKind::Expression => match &attribute.value {
                Some(value) => value.clone(),
                None => continue,
            },
        };
        properties.push(property(
            &attribute.name,
            attribute.name_span,
            value,
            attribute.value_span,
            false,
        ));
    }
    let mut semantic = site.children.iter().filter(|child| is_semantic_child(child));
    if let (Some(name), Some(child), None) = (site.children_name, semantic.next(), semantic.next()) {
        let child = match child {
            ParsedJsxChild::Expression {
                expression: Some(expression),
                span,
                ..
            } => Some((expression.clone(), *span)),
            ParsedJsxChild::Element(element) => Some((element.clone(), jsx_child_span(element))),
            _ => None,
        };
        if let Some((value, span)) = child {
            properties.push(property(name, None, value, span, false));
        }
    }
    surge_ts_syntax::ParsedCallArgument {
        expression: ParsedExpression::ObjectLiteral {
            properties,
            span: None,
        },
        span: None,
        spread: false,
        expression_span: None,
    }
}

/// A JSX namespace member surge resolved without its members: the degradation
/// sentinel, or an object an unmodelled operand left open.
fn is_unmodelled_member(member: &Type) -> bool {
    match member.peeled() {
        Type::Object(object) => object.synthetic_open_index,
        other => is_unmodelled_props(&other),
    }
}

/// Props surge could not model: its degradation sentinel, or a bare type
/// parameter nothing instantiated. The `unknown` keyword and tsc's error type
/// are props of their own.
fn is_unmodelled_props(props: &Type) -> bool {
    matches!(props.peeled(), Type::Unknown | Type::TypeParameter(_))
}

/// Whether a component that yields no props type is surge's modelling gap
/// rather than the source's. Only surge's own sentinel and a bare type parameter
/// qualify — tRPC's `stream.Provider`, read off a `createHydrationStreamProvider<…>()`
/// surge could not type, where tsc has the props and types every callback in
/// them. `any` and tsc's error type do not: a callback attribute there has no
/// contextual signature in tsc either, and it reports the parameter (the
/// unresolved `Textarea` in tRPC's `next-sse-chat`).
fn component_is_unmodelled(component_type: &Type) -> bool {
    matches!(
        component_type.peeled(),
        Type::Unknown | Type::TypeParameter(_)
    )
}

/// tsc's `getUninstantiatedJsxSignaturesOfType`: a component's construct
/// signatures when it has any, else its call signatures, each overload a
/// candidate. The flag says which kind they are.
fn jsx_signatures(component_type: &Type) -> Option<(Vec<FunctionType>, bool)> {
    let peeled = component_type.peeled();
    let (signature, construct) = match &peeled {
        Type::Function(function_type) => (function_type.clone(), false),
        Type::Object(object) => match object.construct_signature() {
            Some(construct_signature) => (construct_signature.clone(), true),
            None => (object.call_signature()?.clone(), false),
        },
        _ => return None,
    };
    let mut signatures = Vec::new();
    signature.push_overload_members(&mut signatures);
    Some((signatures, construct))
}

/// Whether the class this instance belongs to extends a member of an `any`
/// value (`React.Component` with `React: any`): its open index is then the
/// `any` base's (`anyBaseTypeIndexInfo`), which declares no `props`.
fn class_extends_any_value(instance: &surge_ts_types::ObjectType, symbols: &SymbolTable, ctx: &CheckerContext) -> bool {
    let Some(name) = instance.alias_id.as_deref().and_then(|id| id.rsplit('\u{0}').next()) else {
        return false;
    };
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(name) else {
        return false;
    };
    let [base] = info.body.extends.as_slice() else {
        return false;
    };
    let head = base.name.split('.').next().unwrap_or(&base.name);
    info.is_class_instance
        && symbols.get(head).is_some_and(|symbol| matches!(symbol.ty, Type::Any))
}

/// tsc's `getJsxPropsTypeFromClassType` before the intrinsic attributes are
/// added: the instance's `ElementAttributesProperty` member, the instance
/// itself when that interface declares no member, and the first constructor
/// parameter when the JSX namespace has no such interface.
fn class_component_props(
    signature: &FunctionType,
    component_type: &Type,
    site: &JsxCallSite<'_>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let first_parameter = || {
        signature
            .parameters()
            .first()
            .cloned()
            .unwrap_or(Type::GenuineUnknown)
    };
    match jsx_attributes_container_member("ElementAttributesProperty", ctx) {
        ContainerMember::Missing => managed_attributes(component_type, first_parameter(), ctx),
        ContainerMember::Unmodelled => Type::Unknown,
        ContainerMember::Empty => {
            managed_attributes(component_type, signature.return_type().clone(), ctx)
        }
        ContainerMember::Name(name) => match signature.return_type().peeled() {
            Type::Object(instance) => match instance.get_property(&name) {
                Some(property) => managed_attributes(component_type, property.ty.clone(), ctx),
                // tsc's `getJsxPropsTypeFromClassType`: TS2607 at the element
                // when it writes any attribute; the props are then `unknown`,
                // which any attributes object fits.
                None => {
                    if (!instance.synthetic_open_index || class_extends_any_value(&instance, symbols, ctx))
                        && !site.attributes.is_empty()
                    {
                        ctx.push(diagnostic_with_syntax_span(
                            Diagnostic::ts2607(&name, ctx.file_name.clone()),
                            site.element_span,
                        ));
                    }
                    Type::GenuineUnknown
                }
            },
            // A generic class's construct signature returns `any` here; with
            // no attribute to infer from that is not context-sensitive, its
            // type parameters are their defaults or constraints
            // (`inferJsxTypeArguments`).
            Type::Any => match generic_class_component_instance(site, ctx)
                .and_then(|instance| match instance.peeled() {
                    Type::Object(instance) => instance.get_property(&name).map(|property| property.ty.clone()),
                    _ => None,
                }) {
                Some(props) => managed_attributes(component_type, props, ctx),
                None => managed_attributes(component_type, Type::Any, ctx),
            },
            other if other.is_unknown() => other,
            _ => Type::GenuineUnknown,
        },
    }
}

/// The instance of a generic class component the tag names, its type
/// arguments as written or else each parameter's default or constraint.
fn generic_class_component_instance(site: &JsxCallSite<'_>, ctx: &mut CheckerContext) -> Option<Type> {
    let ParsedExpression::Identifier { name, .. } = site.tag_expression else {
        return None;
    };
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(name) else {
        return None;
    };
    if !info.is_class_instance || info.body.type_parameters.is_empty() {
        return None;
    }
    let arguments: Vec<ParsedType> = if site.type_arguments.is_empty() {
        // Inference skips context-sensitive attributes; any other attribute
        // (or child) is an inference source this does not model.
        let only_context_sensitive = !site.attributes.is_empty()
            && site.children.is_empty()
            && site.attributes.iter().all(|attribute| {
                !attribute.name.is_empty()
                    && attribute
                        .value
                        .as_ref()
                        .is_some_and(super::call::is_context_sensitive_argument)
            });
        if !only_context_sensitive {
            return None;
        }
        info.body
            .type_parameters
            .iter()
            .map(|parameter| {
                parameter
                    .default_type
                    .clone()
                    .or_else(|| parameter.constraint.clone())
                    .unwrap_or(ParsedType::UnknownKeyword)
            })
            .collect()
    } else {
        site.type_arguments.to_vec()
    };
    let checkpoint = ctx.diagnostics().len();
    let instance = map_parsed_type(named_type(name, arguments), ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    (!instance.is_unknown()).then_some(instance)
}

/// tsc's `getJsxManagedAttributesFromLocatedAttributes`: a component's props
/// are what the namespace's `LibraryManagedAttributes<C, P>` makes of the tag's
/// own type and the props the component declares, when the namespace declares
/// it with room for both (`instantiateAliasOrInterfaceWithDefaults`). A result
/// surge cannot resolve leaves the declared props.
fn managed_attributes(component_type: &Type, props: Type, ctx: &mut CheckerContext) -> Type {
    if is_unmodelled_props(&props) {
        return props;
    }
    let (prefix, scope) = match jsx_namespace(ctx) {
        JsxNamespace::Qualified(prefix) => (prefix, None),
        JsxNamespace::Declared { scope, prefix } => (prefix, Some(scope)),
        JsxNamespace::Unmodelled | JsxNamespace::Missing => return props,
    };
    let name = format!("{prefix}.LibraryManagedAttributes");
    let resolve = |ctx: &mut CheckerContext| {
        let parameter_count = match ctx.lookup_type_declaration(&name)? {
            crate::symbols::TypeDeclarationInfo::Interface(info) => info.body.type_parameters.len(),
            crate::symbols::TypeDeclarationInfo::Alias(info) => info.body.type_parameters.len(),
        };
        if parameter_count < 2 {
            return None;
        }
        // Each slot is named after the type it holds, as for
        // `IntrinsicClassAttributes`: a reference renders its written arguments.
        let component_slot = component_type.name();
        let mut props_slot = props.name();
        if props_slot == component_slot {
            props_slot.push('\'');
        }
        let mut substitution = crate::infer::TypeParameterSubstitution::new();
        substitution.insert(component_slot.clone(), component_type.clone());
        substitution.insert(props_slot.clone(), props.clone());
        // Errors inside the declaration are its own, reported where it is
        // declared.
        let checkpoint = ctx.diagnostics().len();
        let managed = crate::infer::map_parsed_type_with_substitution(
            named_type(
                &name,
                vec![named_type(&component_slot, Vec::new()), named_type(&props_slot, Vec::new())],
            ),
            ctx,
            &substitution,
        );
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        (!is_unmodelled_props(&managed)).then_some(managed)
    };
    let managed = match scope {
        Some(scope) => crate::infer::with_type_declaration_scope(&Some(scope), ctx, resolve),
        None => resolve(ctx),
    };
    managed.unwrap_or(props)
}

/// tsc's `getJsxPropsTypeFromCallSignature` / `getJsxPropsTypeFromClassType`
/// tail: `IntrinsicAttributes & IntrinsicClassAttributes<Instance> & Props`,
/// each part only when the JSX namespace declares it. `any` props stay `any`
/// for a class component.
fn with_intrinsic_attributes(
    props: Type,
    class_instance: Option<&Type>,
    ctx: &mut CheckerContext,
) -> JsxProps {
    if class_instance.is_some() && matches!(props.peeled(), Type::Any) {
        return JsxProps::plain(props);
    }
    let intrinsic_attributes = match jsx_namespace_member("IntrinsicAttributes", ctx) {
        JsxMember::Found(intrinsic_attributes) if !intrinsic_attributes.is_unknown() => {
            Some(intrinsic_attributes)
        }
        _ => None,
    };
    let class_attributes =
        class_instance.and_then(|instance| intrinsic_class_attributes(instance, ctx));
    if intrinsic_attributes.is_none() && class_attributes.is_none() {
        return JsxProps::plain(props);
    }
    let both_declared = intrinsic_attributes.is_some()
        && matches!(
            jsx_namespace_member("IntrinsicClassAttributes", ctx),
            JsxMember::Found(_)
        );
    let intrinsic_parts: Vec<Type> = intrinsic_attributes.into_iter().chain(class_attributes).collect();
    let target = crate::infer::types::merge_intersection_members(
        intrinsic_parts
            .iter()
            .cloned()
            .chain(std::iter::once(props.clone()))
            .collect(),
    );
    let mut constituents = intrinsic_parts;
    match props.peeled() {
        Type::Object(object) if object.is_intersection => match object.intersection_operands.as_deref() {
            Some(operands) if !operands.is_empty() => constituents.extend(operands.iter().cloned()),
            _ => constituents.push(props),
        },
        _ => constituents.push(props),
    }
    JsxProps {
        target,
        constituents,
        reports_constituent: both_declared,
        returns: None,
    }
}

/// `IntrinsicClassAttributes<Instance>`, instantiated with the class
/// component's instance type when the interface is generic.
fn intrinsic_class_attributes(instance: &Type, ctx: &mut CheckerContext) -> Option<Type> {
    let (prefix, scope) = match jsx_namespace(ctx) {
        JsxNamespace::Qualified(prefix) => (prefix, None),
        JsxNamespace::Declared { scope, prefix } => (prefix, Some(scope)),
        JsxNamespace::Unmodelled | JsxNamespace::Missing => return None,
    };
    let name = format!("{prefix}.IntrinsicClassAttributes");
    let instance = instance.clone();
    let resolve = |ctx: &mut CheckerContext| {
        let parameter_count = match ctx.lookup_type_declaration(&name)? {
            crate::symbols::TypeDeclarationInfo::Interface(info) => info.body.type_parameters.len(),
            crate::symbols::TypeDeclarationInfo::Alias(info) => info.body.type_parameters.len(),
        };
        // The instance fills the first parameter; tsc defaults the others,
        // which only a single-parameter declaration spares surge from doing.
        let resolved = match parameter_count {
            0 => map_parsed_type(named_type(&name, Vec::new()), ctx),
            1 => {
                let slot = instance.name();
                let mut substitution = crate::infer::TypeParameterSubstitution::new();
                substitution.insert(slot.clone(), instance.clone());
                crate::infer::map_parsed_type_with_substitution(
                    named_type(&name, vec![named_type(&slot, Vec::new())]),
                    ctx,
                    &substitution,
                )
            }
            _ => return None,
        };
        (!resolved.is_unknown()).then_some(resolved)
    };
    match scope {
        Some(scope) => crate::infer::with_type_declaration_scope(&Some(scope), ctx, resolve),
        None => resolve(ctx),
    }
}

fn named_type(name: &str, type_arguments: Vec<ParsedType>) -> ParsedType {
    ParsedType::Named(Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span: None,
        type_arguments,
    }))
}

/// What `ElementAttributesProperty` / `ElementChildrenAttribute` name (tsc's
/// `getNameFromJsxElementAttributesContainer`).
enum ContainerMember {
    /// The namespace does not declare the interface, or it declares several
    /// members.
    Missing,
    /// It declares no member.
    Empty,
    Name(String),
    /// surge could not enumerate the interface's members.
    Unmodelled,
}

fn jsx_attributes_container_member(container: &str, ctx: &mut CheckerContext) -> ContainerMember {
    let declared = match jsx_namespace_member(container, ctx) {
        JsxMember::Found(declared) => declared,
        JsxMember::Missing => return ContainerMember::Missing,
        JsxMember::Unmodelled => return ContainerMember::Unmodelled,
    };
    let Type::Object(object) = declared.peeled() else {
        return ContainerMember::Unmodelled;
    };
    if object.synthetic_open_index {
        return ContainerMember::Unmodelled;
    }
    let mut names = object.properties.keys();
    match (names.next(), names.next()) {
        (None, _) => ContainerMember::Empty,
        (Some(name), None) => ContainerMember::Name(name.to_string()),
        (Some(_), Some(_)) => {
            report_container_with_several_members(container, ctx);
            ContainerMember::Missing
        }
    }
}

/// TS2608 at the interface's declaration, which tsc reports once however many
/// elements read it; only a declaration in the file being checked is
/// reported, so the files reading another's do not each repeat it.
fn report_container_with_several_members(container: &str, ctx: &mut CheckerContext) {
    let JsxNamespace::Qualified(prefix) = jsx_namespace(ctx) else {
        return;
    };
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) =
        ctx.lookup_type_declaration(&format!("{prefix}.{container}"))
    else {
        return;
    };
    if info.file_name.as_ref() != ctx.file_name.as_str() {
        return;
    }
    let span = info.name_span;
    ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2608(container, ctx.file_name.clone()), span));
}

/// tsc's `getJsxElementChildrenPropertyName`: fixed to `children` under the
/// automatic runtime, else the `ElementChildrenAttribute` member. Without a
/// name the element's body is not an attribute at all.
enum ChildrenPropertyName {
    Name(String),
    /// The namespace declares no `ElementChildrenAttribute`, or one with
    /// several members (tsc's `InternalSymbolNameMissing`).
    Missing,
    /// `ElementChildrenAttribute` declares no member.
    Empty,
    Unmodelled,
}

fn jsx_children_property_name(ctx: &mut CheckerContext) -> ChildrenPropertyName {
    if ctx.options.jsx_automatic_runtime {
        return ChildrenPropertyName::Name("children".to_string());
    }
    match jsx_attributes_container_member("ElementChildrenAttribute", ctx) {
        ContainerMember::Name(name) => ChildrenPropertyName::Name(name),
        ContainerMember::Missing => ChildrenPropertyName::Missing,
        ContainerMember::Empty => ChildrenPropertyName::Empty,
        ContainerMember::Unmodelled => ChildrenPropertyName::Unmodelled,
    }
}

/// tsc's `getIntrinsicTagSymbol` and `getIntrinsicAttributesTypeFromJsxOpeningLikeElement`:
/// the props `IntrinsicElements` declares for `tag_name`, reported at `node_span`
/// as TS2339 when it declares none and as TS7026 when the JSX namespace has no
/// `IntrinsicElements` at all. The attributes relate to those props as they
/// are, but are contextually typed by them joined with `IntrinsicAttributes`
/// (the intrinsic element's fake signature).
fn intrinsic_element_props(
    tag_name: &str,
    node_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> PropsResolution {
    let intrinsic_elements = match jsx_namespace_member("IntrinsicElements", ctx) {
        JsxMember::Found(intrinsic_elements) => intrinsic_elements,
        JsxMember::Missing => {
            if ctx.options.no_implicit_any {
                ctx.push(diagnostic_with_syntax_span(
                    Diagnostic::ts7026("IntrinsicElements", ctx.file_name.clone()),
                    node_span,
                ));
            }
            return PropsResolution::Unchecked;
        }
        JsxMember::Unmodelled => return PropsResolution::Unmodelled,
    };
    let Type::Object(object) = intrinsic_elements.peeled() else {
        return PropsResolution::Unmodelled;
    };

    let props = if let Some(intersected) = overlapping_keyed_props(&object, tag_name) {
        intersected
    } else if let Some(property_type) = object.get_property_type(tag_name) {
        property_type.clone()
    } else if object.synthetic_open_index {
        // The index stands for members surge could not enumerate.
        return PropsResolution::Unmodelled;
    } else if let Some(index_type) = object.string_index_type.as_deref() {
        index_type.clone()
    } else {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2339(tag_name, "JSX.IntrinsicElements", ctx.file_name.clone()),
            node_span,
        ));
        return PropsResolution::Unchecked;
    };
    if is_unmodelled_props(&props) {
        return PropsResolution::Unmodelled;
    }
    PropsResolution::Props {
        contextual: with_intrinsic_attributes(props.clone(), None, ctx).target,
        candidates: vec![JsxProps::plain(props)],
        failure_returns: None,
    }
}

/// `getApplicableIndexInfoForName` when several of the namespace's keyed
/// index signatures take the tag (`foo${string}` and `foobar${string}`):
/// the intersection of their values.
fn overlapping_keyed_props(object: &surge_ts_types::ObjectType, tag_name: &str) -> Option<Type> {
    if object.properties.contains_key(tag_name) {
        return None;
    }
    let keyed = object.active_keyed_indexes()?;
    let tag = Type::StringLiteral(tag_name.to_string());
    let applicable: Vec<Type> = keyed
        .infos
        .iter()
        .filter(|(key, _)| surge_ts_types::is_assignable_to(&tag, key))
        .map(|(_, value)| value.clone())
        .collect();
    (applicable.len() > 1).then(|| crate::infer::types::merge_intersection_members(applicable))
}

/// tsc's `isHyphenatedJsxName`: `data-*`/`aria-*` style attributes are never
/// excess and never checked against an index signature.
fn is_hyphenated_jsx_name(name: &str) -> bool {
    name.contains('-')
}

/// An explicit attribute's value as the attributes object carries it (tsc's
/// `checkJsxAttribute`): the value's type for a mutable location, `true` for
/// the shorthand, or `None` when surge could not type it.
struct AttributeValue<'a> {
    attribute: &'a ParsedJsxAttribute,
    ty: Option<Type>,
    /// What a later spread writes over the value when its type always has the
    /// property (tsc's `getSpreadType`): the attributes object carries that
    /// instead, and elaboration reads it back.
    overridden: Option<Type>,
}

/// tsc's `createJsxAttributesTypeFromAttributesProperty`, as far as the checks
/// read it.
#[derive(Default)]
struct EvaluatedAttributes<'a> {
    explicit: Vec<AttributeValue<'a>>,
    /// What the `{...spread}` attributes contribute, a later one overriding an
    /// earlier one.
    spread: PropertyMap,
    /// A spread of `any`: tsc types the whole attributes object `any`.
    any_spread: bool,
    /// A spread whose members surge cannot enumerate: the attributes object
    /// may have any property, so none can be reported missing.
    opaque_spread: bool,
    /// Every spread's type, in order.
    spread_types: Vec<Type>,
}

/// The element's body as tsc's `elaborateJsxComponents` relates it.
struct JsxBody {
    /// The property the body is checked under.
    name: String,
    /// Whether the body forms that property of the attributes object; when it
    /// does not, the attributes object has no such property to relate.
    in_attributes: bool,
    children: ChildrenAttribute,
}

/// Where a relation failure is reported, and the tag it names.
struct RelationSite<'a> {
    tag_name: &'a str,
    tag_name_span: Option<SyntaxTextSpan>,
}

/// The children attribute tsc synthesizes from the element's body.
struct ChildrenAttribute {
    ty: Option<Type>,
    /// The lone semantic child: where a mismatch against the children's type
    /// is reported and what it was.
    single: Option<(ChildKind, Option<SyntaxTextSpan>)>,
    /// Each semantic child of a body with several, in order: what it was,
    /// its type, and where it is written.
    each: Vec<(ChildKind, Option<Type>, Option<SyntaxTextSpan>)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildKind {
    Text,
    Expression,
    Element,
}

/// An object literal the spread's value can be (`cond ? { k } : {}`), which
/// the contextual walk answers with the type it was checked against.
fn yields_object_literal(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::ObjectLiteral { .. } => true,
        ParsedExpression::Conditional { when_true, when_false, .. } => {
            yields_object_literal(when_true) || yields_object_literal(when_false)
        }
        ParsedExpression::Logical { left, right, .. } => yields_object_literal(left) || yields_object_literal(right),
        _ => false,
    }
}

fn evaluate_attributes<'a>(
    attributes: &'a [ParsedJsxAttribute],
    contextual: Option<&Type>,
    value_diagnostic: ExpectedTypeDiagnostic,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> EvaluatedAttributes<'a> {
    let mut evaluated = EvaluatedAttributes::default();
    for attribute in attributes {
        // `{...spread}` attributes carry no name. tsc contextually types the
        // spread expression by the attributes' contextual type.
        if attribute.name.is_empty() {
            let Some(value) = &attribute.value else {
                continue;
            };
            let spread = match evaluate_expression_with_expected_type(
                value,
                attribute.value_span.or(fallback_span),
                contextual,
                ExpectedTypeDiagnostic::ContextOnly,
                symbols,
                ctx,
            ) {
                // tsc spreads the expression's own type; the context only
                // types the functions inside it. The contextual walk answers
                // an object literal with what it was checked against, or with
                // nothing when it does not fit, so its type is read again.
                _ if yields_object_literal(value) => crate::infer::infer_expression(value, symbols, ctx),
                spread => spread,
            };
            // tsc's `isValidSpreadType` (TS2698) and, for a valid spread,
            // `checkSpreadPropOverrides` against every attribute written so far.
            super::expr::check_object_spread_type(
                &spread,
                attribute.value_span.or(fallback_span),
                ctx,
            );
            let written: Vec<(&str, Option<SyntaxTextSpan>)> = evaluated
                .explicit
                .iter()
                .map(|value| (value.attribute.name.as_str(), value.attribute.span.or(value.attribute.name_span)))
                .collect();
            super::expr::report_overwritten_properties(&spread, &written, ctx);
            if let InferredExpression::Known(ty) = &spread {
                evaluated.spread_types.push(ty.clone());
            }
            match spread {
                InferredExpression::Known(ty) => match ty.peeled() {
                    Type::Any | Type::ErrorType => evaluated.any_spread = true,
                    Type::Object(object) => {
                        if object.allows_string_index_access() {
                            evaluated.opaque_spread = true;
                        }
                        for value in &mut evaluated.explicit {
                            if let Some(property) = object.get_property(&value.attribute.name)
                                && !property.is_optional()
                            {
                                value.overridden = Some(property.ty.clone());
                            }
                        }
                        for (name, property) in object.properties.iter() {
                            evaluated.spread.shift_remove(name);
                            evaluated.spread.insert(name.clone(), property.clone());
                        }
                    }
                    _ => evaluated.opaque_spread = true,
                },
                _ => evaluated.opaque_spread = true,
            }
            continue;
        }

        let contextual_type =
            contextual.and_then(|contextual| attribute_contextual_type(contextual, &attribute.name));
        let ty = attribute_value_type(
            attribute,
            contextual_type.as_ref(),
            value_diagnostic,
            fallback_span,
            symbols,
            ctx,
        );
        evaluated.explicit.push(AttributeValue {
            attribute,
            ty,
            overridden: None,
        });
    }
    evaluated
}

/// tsc's `checkJsxAttribute`: a string-literal value and an `{expression}`
/// are checked for a mutable location, so a literal widens unless the
/// contextual type holds literals of its kind; the shorthand is `true`.
fn attribute_value_type(
    attribute: &ParsedJsxAttribute,
    contextual_type: Option<&Type>,
    value_diagnostic: ExpectedTypeDiagnostic,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let ty = checked_attribute_value_type(attribute, contextual_type, value_diagnostic, fallback_span, symbols, ctx);
    if let Some(ty) = &ty {
        crate::semantic::record_declaration_type(attribute.name_span, ty, ctx);
    }
    ty
}

fn checked_attribute_value_type(
    attribute: &ParsedJsxAttribute,
    contextual_type: Option<&Type>,
    value_diagnostic: ExpectedTypeDiagnostic,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    match &attribute.value_kind {
        ParsedJsxAttributeValueKind::StringLiteral(value) => Some(
            crate::checks::function::widen_unit_return_type(
                Type::StringLiteral(value.clone()),
                contextual_type,
            ),
        ),
        ParsedJsxAttributeValueKind::BooleanShorthand => Some(Type::BooleanLiteral(true)),
        ParsedJsxAttributeValueKind::Expression => {
            let value = attribute.value.as_ref()?;
            let inferred = evaluate_expression_with_expected_type(
                value,
                attribute.value_span.or(fallback_span),
                contextual_type,
                value_diagnostic,
                symbols,
                ctx,
            );
            match inferred {
                InferredExpression::Known(ty) if !ty.is_unknown() => Some(
                    crate::checks::function::widen_unit_return_type(ty, contextual_type),
                ),
                _ => None,
            }
        }
    }
}

/// tsc's `getTypeOfPropertyOfContextualType` for an attribute: a member's
/// declared type (with `undefined` when optional) or its index signature's,
/// joined over a union's members. `any` offers none
/// (`getContextualTypeForJsxAttribute`).
fn attribute_contextual_type(contextual: &Type, name: &str) -> Option<Type> {
    match contextual.peeled() {
        Type::Union(union) => {
            let members: Vec<Type> = union
                .types()
                .iter()
                .filter_map(|member| attribute_contextual_type(member, name))
                .collect();
            (!members.is_empty()).then(|| union_type(members))
        }
        Type::Object(object) => match object.get_property(name) {
            Some(property) => Some(optional_aware_property_type(property)),
            // The index stands for members surge could not enumerate, so it
            // is no context at all: the sentinel says so.
            None if object.synthetic_open_index => Some(Type::Unknown),
            None => object
                .applicable_index_type(surge_ts_types::is_numeric_key(name))
                .cloned(),
        },
        _ => None,
    }
}

/// An optional prop's type includes `undefined` without
/// `exactOptionalPropertyTypes`, both as the contextual type for its value and
/// as the target it is compared against.
fn optional_aware_property_type(property: &ObjectProperty) -> Type {
    if property.is_optional() {
        union_type(vec![property.ty.clone(), Type::Undefined])
    } else {
        property.ty.clone()
    }
}

/// tsc's `discriminateContextualTypeByJSXAttributes`: a union of props is
/// narrowed to the members whose discriminant properties admit the values the
/// attributes write, a discriminant the attributes leave out counting as
/// `undefined` — except `children` when the body supplies it.
fn discriminate_by_attributes(
    contextual: &Type,
    attributes: &[ParsedJsxAttribute],
    supplied_children: Option<&str>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let Type::Union(union) = contextual.peeled() else {
        return contextual.clone();
    };
    let members = union.types().to_vec();

    let mut discriminators: Vec<(String, Type)> = Vec::new();
    for attribute in attributes {
        if attribute.name.is_empty()
            || !is_possibly_discriminant_value(attribute)
            || !is_discriminant_property(&members, &attribute.name)
        {
            continue;
        }
        let value = match &attribute.value_kind {
            ParsedJsxAttributeValueKind::BooleanShorthand => Type::BooleanLiteral(true),
            ParsedJsxAttributeValueKind::StringLiteral(value) => Type::StringLiteral(value.clone()),
            ParsedJsxAttributeValueKind::Expression => {
                let Some(value) = &attribute.value else {
                    continue;
                };
                match crate::infer::infer_expression(value, symbols, ctx) {
                    InferredExpression::Known(ty) => ty,
                    _ => continue,
                }
            }
        };
        discriminators.push((attribute.name.clone(), value));
    }
    for name in common_optional_properties(&members) {
        if attributes.iter().any(|attribute| attribute.name == name)
            || supplied_children == Some(name.as_str())
            || !is_discriminant_property(&members, &name)
        {
            continue;
        }
        discriminators.push((name, Type::Undefined));
    }

    match discriminate_members(&members, &discriminators) {
        Some(remaining) => union_type(remaining),
        None => contextual.clone(),
    }
}

/// tsc's `discriminateTypeByDiscriminableItems`: each discriminator drops the
/// members whose property does not admit its value, unless none admits it.
/// `None` when nothing is dropped.
fn discriminate_members(members: &[Type], discriminators: &[(String, Type)]) -> Option<Vec<Type>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Include {
        Yes,
        Maybe,
        No,
    }
    let mut include: Vec<Include> = members
        .iter()
        .map(|member| match member.peeled() {
            Type::Object(_) | Type::Function(_) | Type::Array(_) | Type::Tuple(_) => Include::Yes,
            _ => Include::No,
        })
        .collect();
    for (name, value) in discriminators {
        let mut matched = false;
        for (index, member) in members.iter().enumerate() {
            if include[index] == Include::No {
                continue;
            }
            let Some(target) = member_property_type(member, name) else {
                continue;
            };
            let admits = match value {
                Type::Union(value) => value
                    .types()
                    .iter()
                    .any(|constituent| is_assignable_to(constituent, &target)),
                value => is_assignable_to(value, &target),
            };
            if admits {
                matched = true;
            } else {
                include[index] = Include::Maybe;
            }
        }
        for state in include.iter_mut() {
            if *state == Include::Maybe {
                *state = if matched { Include::No } else { Include::Yes };
            }
        }
    }
    if !include.contains(&Include::No) {
        return None;
    }
    let remaining: Vec<Type> = members
        .iter()
        .zip(&include)
        .filter(|(_, state)| **state == Include::Yes)
        .map(|(member, _)| member.clone())
        .collect();
    (!remaining.is_empty()).then_some(remaining)
}

/// tsc's `isPossiblyDiscriminantValue` for an attribute's initializer.
fn is_possibly_discriminant_value(attribute: &ParsedJsxAttribute) -> bool {
    fn expression(value: &ParsedExpression) -> bool {
        match value {
            ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::TemplateLiteral { .. }
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::NullLiteral
            | ParsedExpression::UndefinedLiteral
            | ParsedExpression::Identifier { .. } => true,
            ParsedExpression::PropertyAccess { object, .. } => expression(object),
            _ => false,
        }
    }
    match &attribute.value_kind {
        ParsedJsxAttributeValueKind::BooleanShorthand
        | ParsedJsxAttributeValueKind::StringLiteral(_) => true,
        ParsedJsxAttributeValueKind::Expression => attribute.value.as_ref().is_none_or(expression),
    }
}

/// A union member's property type as `getTypeOfPropertyOrIndexSignatureOfType`
/// reads it.
fn member_property_type(member: &Type, name: &str) -> Option<Type> {
    let Type::Object(object) = member.peeled() else {
        return None;
    };
    match object.get_property(name) {
        Some(property) => Some(optional_aware_property_type(property)),
        None => object
            .applicable_index_type(surge_ts_types::is_numeric_key(name))
            .cloned(),
    }
}

/// tsc's `isDiscriminantProperty`: the union's members declare the property
/// with differing types, one of them a literal.
fn is_discriminant_property(members: &[Type], name: &str) -> bool {
    let types: Vec<Type> = members
        .iter()
        .filter_map(|member| {
            let Type::Object(object) = member.peeled() else {
                return None;
            };
            object.get_property(name).map(|property| property.ty.clone())
        })
        .collect();
    let Some(first) = types.first() else {
        return false;
    };
    types.iter().any(|ty| ty != first) && types.iter().any(is_literal_type)
}

/// tsc's `isLiteralType`: `boolean`, a unit type, or a union of unit types.
fn is_literal_type(ty: &Type) -> bool {
    let is_unit = |ty: &Type| {
        matches!(
            ty,
            Type::StringLiteral(_)
                | Type::NumberLiteral(_)
                | Type::BooleanLiteral(_)
                | Type::Undefined
                | Type::Null
        )
    };
    match ty {
        Type::Boolean => true,
        Type::Union(union) => union.types().iter().all(is_unit),
        other => is_unit(other),
    }
}

/// The properties every member of a union declares, optional in at least one
/// of them (tsc's union property flags).
fn common_optional_properties(members: &[Type]) -> Vec<String> {
    let objects: Vec<ObjectType> = members
        .iter()
        .filter_map(|member| match member.peeled() {
            Type::Object(object) => Some(object),
            _ => None,
        })
        .collect();
    let Some(first) = objects.first() else {
        return Vec::new();
    };
    first
        .properties
        .keys()
        .filter(|name| {
            objects
                .iter()
                .all(|object| object.properties.contains_key(name.as_ref()))
                && objects.iter().any(|object| {
                    object
                        .properties
                        .get(name.as_ref())
                        .is_some_and(ObjectProperty::is_optional)
                })
        })
        .map(|name| name.to_string())
        .collect()
}

/// tsc's `GetSemanticJsxChildren` entry test: an empty `{}` container is no
/// child. Whitespace-only text never reaches the checker.
fn is_semantic_child(child: &ParsedJsxChild) -> bool {
    !matches!(
        child,
        ParsedJsxChild::Expression {
            expression: None,
            ..
        }
    )
}

/// tsc's `checkJsxChildren` with each `{expression}` child contextually typed
/// by the children attribute (`getContextualTypeForChildJsxExpression`), and
/// the children attribute they form: the lone child's type, else an array of
/// their union.
fn evaluate_children(
    children: &[ParsedJsxChild],
    child_spans: &[Option<SyntaxTextSpan>],
    contextual: Option<&Type>,
    children_name: Option<&str>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> ChildrenAttribute {
    let semantic: Vec<(&ParsedJsxChild, Option<SyntaxTextSpan>)> = children
        .iter()
        .enumerate()
        .filter(|(_, child)| is_semantic_child(child))
        .map(|(index, child)| (child, child_spans.get(index).copied().flatten()))
        .collect();
    let children_contextual = contextual
        .zip(children_name)
        .and_then(|(contextual, name)| attribute_contextual_type(contextual, name));
    let element_type = match semantic
        .iter()
        .any(|(child, _)| matches!(child, ParsedJsxChild::Element(_)))
    {
        true => match jsx_namespace_member("Element", ctx) {
            JsxMember::Found(element) if !element.is_unknown() => Some(element),
            _ => None,
        },
        false => None,
    };

    let mut types: Vec<Option<Type>> = Vec::with_capacity(semantic.len());
    let mut single = None;
    let mut each = Vec::new();
    for (index, &(child, child_span)) in semantic.iter().enumerate() {
        let (kind, ty) = match child {
            ParsedJsxChild::Text => (ChildKind::Text, Some(Type::String)),
            ParsedJsxChild::Expression {
                expression,
                span,
                spread,
            } => {
                let Some(expression) = expression else {
                    continue;
                };
                let child_contextual = children_contextual.as_ref().map(|contextual| {
                    if semantic.len() == 1 {
                        contextual.clone()
                    } else {
                        child_contextual_type(contextual, index)
                    }
                });
                // The element's own props say nothing about a callback
                // nested inside a child expression: `{items.map((item) =>
                // …)}` takes its parameter types from `map`, not from this
                // tag, and tsc reports it whatever the tag resolved to.
                // Only a *render prop* — a function written directly as the
                // child — is contextually typed by `props.children`, so the
                // suppression is kept for that one shape.
                // The suppression is cleared outright, not decremented:
                // a child expression sits outside the props context of
                // *every* enclosing element, and a nested tag raises the
                // depth again, so stepping down by one left it elevated.
                let release = !matches!(expression, ParsedExpression::ArrowFunction(_));
                let saved_depth = ctx.unmodelled_jsx_props_depth;
                if release {
                    ctx.unmodelled_jsx_props_depth = 0;
                }
                let inferred = evaluate_expression_with_expected_type(
                    expression,
                    span.or(fallback_span),
                    child_contextual.as_ref(),
                    ExpectedTypeDiagnostic::ContextOnly,
                    symbols,
                    ctx,
                );
                ctx.unmodelled_jsx_props_depth = saved_depth;
                if *spread {
                    check_jsx_spread_child(&inferred, child_span.or(*span), ctx);
                }
                let ty = match inferred {
                    InferredExpression::Known(ty) if !ty.is_unknown() => Some(
                        crate::checks::function::widen_unit_return_type(
                            ty,
                            child_contextual.as_ref(),
                        ),
                    ),
                    _ => None,
                };
                (ChildKind::Expression, ty)
            }
            ParsedJsxChild::Element(element) => {
                let _ = evaluate_expression(element, fallback_span, symbols, ctx);
                (ChildKind::Element, element_type.clone())
            }
        };
        if semantic.len() == 1 {
            single = Some((kind, child_span));
        } else {
            each.push((kind, ty.clone(), child_span));
        }
        types.push(ty);
    }

    let ty = match types.as_slice() {
        [] => None,
        [single] => single.clone(),
        many => many
            .iter()
            .cloned()
            .collect::<Option<Vec<Type>>>()
            .map(|types| Type::Array(Box::new(union_type(types)))),
    };
    ChildrenAttribute { ty, single, each }
}

/// tsc's `checkJsxExpression`: a `{...spread}` child must be `any` or an array
/// (`isArrayType`, which a tuple or a union is not). TS2609 at the container.
pub(crate) fn check_jsx_spread_child(
    child: &InferredExpression,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let InferredExpression::Known(ty) = child else {
        return;
    };
    let is_array = match ty {
        Type::Reference(reference) => {
            reference.is_readonly_array() || matches!(ty.peeled(), Type::Array(_))
        }
        other => matches!(other, Type::Array(_)),
    };
    if is_array || matches!(ty, Type::Any | Type::Unknown | Type::TypeParameter(_)) {
        return;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2609(ctx.file_name.clone()),
        span,
    ));
}

fn jsx_child_span(element: &ParsedExpression) -> Option<SyntaxTextSpan> {
    match element {
        ParsedExpression::JsxElement { tag, .. } => tag.span,
        ParsedExpression::JsxFragment { span, .. } => *span,
        _ => None,
    }
}

/// The contextual type of the `index`th of several children: an array-like
/// children type's element, anything else as is.
fn child_contextual_type(children: &Type, index: usize) -> Type {
    match children.peeled() {
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| child_contextual_type(member, index))
                .collect(),
        ),
        Type::Array(element) => *element,
        Type::Tuple(elements) => elements.get(index).cloned().unwrap_or(Type::Undefined),
        other => other,
    }
}

/// tsc's `checkApplicableSignatureForJsxCallLikeElement` relation of the
/// attributes object to one candidate's props, as the diagnostics it reports:
/// when the relation fails, every explicit attribute whose value does not fit
/// its prop and the body against the children prop (`elaborateJsxComponents`),
/// else the relation's own failure — the first excess attribute, else the
/// missing required props.
fn relate_attributes(
    evaluated: &EvaluatedAttributes<'_>,
    body: Option<&JsxBody>,
    props: &JsxProps,
    site: &RelationSite<'_>,
    ctx: &CheckerContext,
) -> Vec<(Diagnostic, Option<SyntaxTextSpan>)> {
    let target = props.target.peeled();
    if let Some(failure) = type_variable_attributes_failure(evaluated, body, &target, site.tag_name_span, ctx) {
        return vec![failure];
    }
    if evaluated.any_spread || target.is_unknown() || matches!(target, Type::Any) {
        return Vec::new();
    }

    let mut elaborated = Vec::new();
    for value in &evaluated.explicit {
        let name = value.attribute.name.as_str();
        if is_hyphenated_jsx_name(name) {
            continue;
        }
        let source = value.overridden.as_ref().or(value.ty.as_ref());
        let (Some(ty), Some(expected)) = (source, attribute_target_type(&target, name)) else {
            continue;
        };
        if type_contains_unknown_or_any(&expected) || is_assignable_to(ty, &expected) {
            continue;
        }
        elaborated.push((
            relation_diagnostic(ty, &expected, ctx),
            value.attribute.name_span.or(site.tag_name_span),
        ));
    }
    let body_diagnostics = body.map(|body| relate_children(body, &target, site, ctx)).unwrap_or_default();
    // A body the attributes object carries fails the relation itself; one it
    // does not carry is only elaborated once something else has.
    let body_fails_relation = !body_diagnostics.is_empty() && body.is_some_and(|body| body.in_attributes);
    if !elaborated.is_empty() || body_fails_relation {
        elaborated.extend(body_diagnostics);
        return elaborated;
    }
    let attributes_body = body.filter(|body| body.in_attributes);
    match relation_failure(evaluated, attributes_body, props, &target, site.tag_name_span, ctx) {
        Some(failure) if body_diagnostics.is_empty() => vec![failure],
        Some(_) => body_diagnostics,
        None => Vec::new(),
    }
}

/// The attributes relation where a type variable of the body being checked
/// is involved: a lone `{...obj}` of type `T` makes the attributes object `T`
/// itself (`getSpreadType`), which relates through its constraint, and props
/// of type `P` take nothing but `P`.
fn type_variable_attributes_failure(
    evaluated: &EvaluatedAttributes<'_>,
    body: Option<&JsxBody>,
    target: &Type,
    tag_name_span: Option<SyntaxTextSpan>,
    ctx: &CheckerContext,
) -> Option<(Diagnostic, Option<SyntaxTextSpan>)> {
    if evaluated.any_spread {
        return None;
    }
    let lone_spread = match (evaluated.explicit.is_empty(), body.is_some_and(|body| body.in_attributes), evaluated.spread_types.as_slice()) {
        (true, false, [spread]) => Some(spread),
        _ => None,
    };
    let source = match lone_spread {
        Some(spread) if spread.is_type_variable() => spread.clone(),
        _ if target.is_type_variable() && evaluated.spread_types.is_empty() => {
            Type::Object(crate::metrics::alloc_object_type(Default::default(), None))
        }
        _ => return None,
    };
    if target.is_unmodelled() || matches!(target, Type::Any) || is_assignable_to(&source, target) {
        return None;
    }
    let source_name = if source.is_type_variable() { source.name() } else { attributes_object_name(evaluated, body) };
    Some((Diagnostic::ts2322(source_name, target.name(), ctx.file_name.clone()), tag_name_span))
}

/// tsc's `checkJsxReturnAssignableToAppropriateBound` for a namespace that
/// declares no `ElementType` (which replaces it with a relation of the tag
/// itself): what a function component returns must be an `Element` or `null`,
/// what a class component constructs an `ElementClass`, each only when the
/// namespace declares it. TS2786 at the tag name.
fn check_jsx_return_bound(props: &JsxProps, site: &RelationSite<'_>, ctx: &mut CheckerContext) {
    let Some((returned, construct)) = &props.returns else {
        return;
    };
    if !matches!(jsx_namespace_member("ElementType", ctx), JsxMember::Missing) {
        return;
    }
    let member = if *construct { "ElementClass" } else { "Element" };
    let bound = match jsx_namespace_member(member, ctx) {
        JsxMember::Found(bound) if !is_unmodelled_member(&bound) => bound,
        _ => return,
    };
    let bound = if *construct {
        bound
    } else {
        union_type(vec![bound, Type::Null])
    };
    if returned.is_unknown()
        || matches!(returned.peeled(), Type::Any)
        || mentions_type_parameter(returned)
        || crate::checks::call::type_contains_unknown(returned)
        || crate::checks::call::type_contains_unknown(&bound)
        || is_assignable_to(returned, &bound)
    {
        return;
    }
    let kind = if *construct { "instance" } else { "return" };
    let chain = component_chain(format!("Its {kind} type '{}' is not a valid JSX element.", returned.name()), returned, &bound);
    ctx.push(diagnostic_with_syntax_span(
        crate::checks::relation_chain::with_chain(Diagnostic::ts2786(site.tag_name, ctx.file_name.clone()), &chain),
        site.tag_name_span,
    ));
}

/// TS2786's chain: the line that replaces the relation's head, then what the
/// relation elaborates beneath it.
fn component_chain(line: String, source: &Type, target: &Type) -> Vec<String> {
    let mut chain = vec![format!("  {line}")];
    chain.extend(
        crate::checks::relation_chain::chain_beneath_head(source, target)
            .into_iter()
            .map(|beneath| format!("  {beneath}")),
    );
    chain
}

/// A union of function and class components has neither kind's signatures
/// on the union itself (`getJsxReferenceKind` is `Mixed`): its members'
/// returns, unioned through `getUnionSignatures`, must suit `Element | null |
/// ElementClass`, else TS2786. Whether every member has signatures is what
/// decides it handled the tag; its attributes are left unchecked.
fn check_mixed_component_union(
    component_type: &Type,
    site: &JsxCallSite<'_>,
    tag_name_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let Type::Union(union) = component_type.peeled() else {
        return false;
    };
    let mut members = Vec::new();
    for member in union.types() {
        match jsx_signatures(member) {
            Some((signatures, construct)) if !signatures.is_empty() => members.push((signatures, construct)),
            _ => return false,
        }
    }
    if !(members.iter().any(|(_, construct)| *construct) && members.iter().any(|(_, construct)| !*construct)) {
        return false;
    }
    let mut returns = Vec::new();
    for (signatures, construct) in &members {
        for signature in signatures {
            let signature = if *construct {
                signature.clone()
            } else {
                match instantiate_jsx_signature(signature, site, symbols, ctx)
                    .or_else(|| site.attributes.is_empty().then(|| constrained_signature(signature, ctx)).flatten())
                {
                    Some(signature) => signature,
                    None => return true,
                }
            };
            returns.push(signature.return_type().clone());
        }
    }
    if !matches!(jsx_namespace_member("ElementType", ctx), JsxMember::Missing) {
        return true;
    }
    let (JsxMember::Found(element), JsxMember::Found(element_class)) =
        (jsx_namespace_member("Element", ctx), jsx_namespace_member("ElementClass", ctx))
    else {
        return true;
    };
    if is_unmodelled_member(&element) || is_unmodelled_member(&element_class) {
        return true;
    }
    let bound = union_type(vec![element, Type::Null, element_class]);
    let returned = union_type(returns);
    if returned.is_unknown()
        || matches!(returned.peeled(), Type::Any)
        || mentions_type_parameter(&returned)
        || crate::checks::call::type_contains_unknown(&returned)
        || crate::checks::call::type_contains_unknown(&bound)
        || is_assignable_to(&returned, &bound)
    {
        return true;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2786(site.tag_name, ctx.file_name.clone()),
        tag_name_span,
    ));
    true
}

/// `getInferredType` with nothing to infer from: each of the signature's own
/// type parameters is its constraint, or `unknown`.
fn constrained_signature(signature: &FunctionType, ctx: &mut CheckerContext) -> Option<FunctionType> {
    let Some(info) = signature
        .declaration()
        .and_then(|declaration| declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>())
    else {
        // A signature whose declaration a merge dropped (a function with
        // expando members) still knows its own type parameters' names.
        let names = signature.type_parameter_names();
        return (!names.is_empty())
            .then(|| surge_ts_types::signature_with_type_parameters_as(signature, &names, &Type::GenuineUnknown));
    };
    if info.type_parameters.is_empty() {
        return None;
    }
    let names: Vec<String> = info.type_parameters.iter().map(|parameter| parameter.name.clone()).collect();
    let checkpoint = ctx.diagnostics().len();
    let types: Vec<Type> = info
        .type_parameters
        .iter()
        .map(|parameter| match &parameter.constraint {
            Some(constraint) => map_parsed_type(constraint.clone(), ctx),
            None => Type::GenuineUnknown,
        })
        .collect();
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if types.iter().any(crate::checks::function::type_contains_degradation) {
        return None;
    }
    Some(surge_ts_types::instantiate_named_type_parameters(signature, &names, &types))
}

/// tsc's `checkJsxOpeningLikeElementOrOpeningFragment` for a namespace that
/// declares `ElementType` (`getJsxElementTypeTypeAt`): the tag's own type must
/// be assignable to it, else TS2786 at the tag name. It replaces the return
/// bound.
fn check_jsx_element_type(tag_type: &Type, site: &RelationSite<'_>, ctx: &mut CheckerContext) {
    let constraint = match jsx_namespace_member("ElementType", ctx) {
        JsxMember::Found(constraint) if !is_unmodelled_member(&constraint) => constraint,
        _ => return,
    };
    if constraint.is_unknown()
        || tag_type.is_unknown()
        || matches!(tag_type.peeled(), Type::Any)
        || is_assignable_to(tag_type, &constraint)
        || mentions_type_parameter(tag_type)
        || crate::checks::call::type_contains_unknown(tag_type)
        || crate::checks::call::type_contains_unknown(&constraint)
    {
        return;
    }
    let chain = component_chain(
        format!("Its type '{}' is not a valid JSX element type.", tag_type.name()),
        tag_type,
        &constraint,
    );
    ctx.push(diagnostic_with_syntax_span(
        crate::checks::relation_chain::with_chain(Diagnostic::ts2786(site.tag_name, ctx.file_name.clone()), &chain),
        site.tag_name_span,
    ));
}

/// The relation's own report when no attribute or child elaborates it.
fn relation_failure(
    evaluated: &EvaluatedAttributes<'_>,
    body: Option<&JsxBody>,
    props: &JsxProps,
    target: &Type,
    tag_name_span: Option<SyntaxTextSpan>,
    ctx: &CheckerContext,
) -> Option<(Diagnostic, Option<SyntaxTextSpan>)> {
    let source_name = || attributes_object_name(evaluated, body);
    // Only a written attribute makes the attributes object a fresh literal
    // (`createJsxAttributesType`); spreads and children alone are never
    // checked for excess properties.
    if !evaluated.explicit.is_empty() && is_excess_property_check_target(target) {
        let known_in = excess_check_members(target, evaluated);
        let excess = evaluated
            .explicit
            .iter()
            .map(|value| (value.attribute.name.as_str(), value.attribute.name_span))
            .chain(body.map(|body| (body.name.as_str(), None)))
            .find(|(name, _)| {
                !is_hyphenated_jsx_name(name)
                    && !known_in.iter().any(|member| is_known_property(member, name))
            });
        if let Some((name, span)) = excess {
            let target_name = props.target.name();
            let line = match excess_attribute_suggestion(name, target) {
                Some(suggestion) => {
                    format!("  Property '{name}' does not exist on type '{target_name}'. Did you mean '{suggestion}'?")
                }
                None => format!("  Property '{name}' does not exist on type '{target_name}'."),
            };
            return Some((
                crate::checks::relation_chain::with_chain(
                    Diagnostic::ts2322(source_name(), target_name.clone(), ctx.file_name.clone()),
                    &[line],
                ),
                span.or(tag_name_span),
            ));
        }
    }

    // An attributes object relates to no primitive, which the first
    // constructor parameter a class-like component is typed by can be when the
    // namespace names no `ElementAttributesProperty` member.
    if is_primitive_type(target) && !evaluated.opaque_spread {
        return Some((
            Diagnostic::ts2322(source_name(), props.target.name(), ctx.file_name.clone()),
            tag_name_span,
        ));
    }
    // A union of props is related member by member; the attributes object
    // fitting none of them fails as a whole, at the tag. A written value that
    // does not fit the members its discriminants select is elaborated where it
    // is written instead (`getBestMatchIndexedAccessTypeOrUndefined`), which
    // surge does not model, so that relation is left unreported.
    if let Type::Union(_) = target {
        let members = excess_check_members(target, evaluated);
        let values_fit = evaluated.explicit.iter().all(|value| {
            let name = value.attribute.name.as_str();
            let Some(ty) = value.overridden.as_ref().or(value.ty.as_ref()) else {
                return true;
            };
            is_hyphenated_jsx_name(name)
                || members.iter().all(|member| {
                    attribute_target_type(&member.peeled(), name).is_none_or(|expected| {
                        type_contains_unknown_or_any(&expected) || is_assignable_to(ty, &expected)
                    })
                })
        });
        let fits = !values_fit
            || evaluated.opaque_spread
            || type_contains_unknown_or_any(target)
            || attributes_object_type(evaluated, body, &members)
                .is_none_or(|source| is_assignable_to(&source, target));
        return (!fits).then(|| {
            (
                Diagnostic::ts2322(source_name(), props.target.name(), ctx.file_name.clone()),
                tag_name_span,
            )
        });
    }
    let Type::Object(object) = target else {
        return None;
    };
    if evaluated.opaque_spread {
        return None;
    }
    let present = |name: &str| {
        evaluated
            .explicit
            .iter()
            .any(|value| value.attribute.name == name)
            || evaluated.spread.contains_key(name)
            || body.is_some_and(|body| body.name == name)
            || surge_ts_types::object_prototype_member_type(name).is_some()
    };
    let missing_from = |object: &ObjectType| -> Vec<String> {
        object
            .required_properties()
            .filter(|(name, _)| !present(name))
            .map(|(name, _)| name.to_string())
            .collect()
    };
    let missing = missing_from(object);
    // A primitive operand (the props of a class component constructed from a
    // `string`) fails every attributes object, whatever it carries.
    let primitive_operand = object.is_intersection
        && object
            .intersection_operands
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|operand| is_primitive_type(&operand.peeled()));
    if missing.is_empty() && !primitive_operand {
        // A hyphenated attribute is never elaborated, but a prop the target
        // declares under its name still has to fit; the whole object is
        // reported then.
        let hyphenated_mismatch = evaluated.explicit.iter().any(|value| {
            is_hyphenated_jsx_name(&value.attribute.name)
                && value.ty.as_ref().is_some_and(|ty| {
                    object.get_property(&value.attribute.name).is_some_and(|property| {
                        let expected = optional_aware_property_type(property);
                        !type_contains_unknown_or_any(&expected) && !is_assignable_to(ty, &expected)
                    })
                })
        });
        // Nor is a member a spread supplies (`elaborateJsxComponents` skips
        // spread attributes). A written attribute of the same name is what the
        // object holds, and is elaborated on its own.
        let spread_mismatch = evaluated.spread.iter().any(|(name, property)| {
            !property.is_optional()
                && !evaluated
                    .explicit
                    .iter()
                    .any(|value| value.attribute.name.as_str() == name.as_ref())
                && object.get_property(name).is_some_and(|target_property| {
                    let expected = optional_aware_property_type(target_property);
                    !type_contains_unknown_or_any(&expected)
                        && !type_contains_unknown_or_any(&property.ty)
                        && !is_assignable_to(&property.ty, &expected)
                })
        });
        return (hyphenated_mismatch || spread_mismatch).then(|| {
            (
                Diagnostic::ts2322(source_name(), props.target.name(), ctx.file_name.clone()),
                tag_name_span,
            )
        });
    }
    if !object.is_intersection {
        return Some((
            crate::checks::expr::missing_properties_diagnostic(
                &missing[0],
                &missing,
                &source_name(),
                &props.target.name(),
                &ctx.file_name,
            ),
            tag_name_span,
        ));
    }
    if props.reports_constituent {
        // The intersection is related one constituent at a time, and the
        // first that fails is what tsc names: an object missing props, or a
        // primitive no attributes object is assignable to.
        for constituent in &props.constituents {
            let constituent_object = match constituent.peeled() {
                Type::Object(constituent_object) => constituent_object,
                other if is_primitive_type(&other) => {
                    return Some((
                        Diagnostic::ts2322(source_name(), constituent.name(), ctx.file_name.clone()),
                        tag_name_span,
                    ));
                }
                _ => continue,
            };
            let missing = missing_from(&constituent_object);
            if let Some(first) = missing.first() {
                return Some((
                    crate::checks::expr::missing_properties_diagnostic(
                        first,
                        &missing,
                        &source_name(),
                        &constituent.name(),
                        &ctx.file_name,
                    ),
                    tag_name_span,
                ));
            }
        }
    }
    // Beneath the head, the first constituent missing props names them.
    let chain: Vec<String> = props
        .constituents
        .iter()
        .find_map(|constituent| {
            let Type::Object(constituent_object) = constituent.peeled() else {
                return None;
            };
            let missing = missing_from(&constituent_object);
            let first = missing.first()?;
            let report = crate::checks::expr::missing_properties_diagnostic(
                first,
                &missing,
                &source_name(),
                &constituent.name(),
                &ctx.file_name,
            );
            Some(vec![format!("  {}", report.message)])
        })
        .unwrap_or_default();
    Some((
        crate::checks::relation_chain::with_chain(
            Diagnostic::ts2322(source_name(), props.target.name(), ctx.file_name.clone()),
            &chain,
        ),
        tag_name_span,
    ))
}

/// `getSuggestedSymbolForNonexistentJSXAttribute`: `htmlFor` for `for` and
/// `className` for `class` when the props declare them, else the spelling
/// suggestion, named as `symbolToString` writes it.
fn excess_attribute_suggestion(name: &str, target: &Type) -> Option<String> {
    let peeled = target.peeled();
    let declares = |member: &str| matches!(&peeled, Type::Object(object) if object.properties.contains_key(member));
    let suggestion = match name {
        "for" if declares("htmlFor") => "htmlFor".to_string(),
        "class" if declares("className") => "className".to_string(),
        _ => crate::checks::expr::property_spelling_suggestion(name, &peeled)?,
    };
    let identifier = suggestion.chars().next().is_some_and(|first| !first.is_ascii_digit())
        && suggestion.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    Some(if identifier { suggestion } else { format!("\"{suggestion}\"") })
}

/// A type no object is assignable to.
fn is_primitive_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Never
    )
}

/// A value's relation failure against `expected`, named the way tsc's
/// `isRelatedTo` reports it: a non-nullable source against `T | undefined`
/// names `T`.
fn relation_diagnostic(source: &Type, expected: &Type, ctx: &CheckerContext) -> Diagnostic {
    let reported = crate::checks::expr::reported_relation_target(source, expected);
    let (source_name, target_name) = crate::checks::expr::disambiguated_pair(
        source,
        source_display_name(source, &reported),
        &reported,
        reported.name(),
        &ctx.file_name,
    );
    crate::checks::expr::type_not_assignable_diagnostic(
        source,
        &reported,
        &source_name,
        &target_name,
        ctx.file_name.clone(),
    )
}

/// The members an attribute has to be known in (tsc's `hasExcessProperties`):
/// a union target is first narrowed by the discriminants the attributes write
/// (`findMatchingDiscriminantType`).
fn excess_check_members(target: &Type, evaluated: &EvaluatedAttributes<'_>) -> Vec<Type> {
    let Type::Union(union) = target else {
        return vec![target.clone()];
    };
    let members = union.types().to_vec();
    let discriminators: Vec<(String, Type)> = evaluated
        .explicit
        .iter()
        .filter_map(|value| Some((value.attribute.name.clone(), value.ty.clone()?)))
        .filter(|(name, _)| is_discriminant_property(&members, name))
        .collect();
    discriminate_members(&members, &discriminators).unwrap_or(members)
}

/// The type an attribute's value relates to (tsc's
/// `getIndexedAccessTypeOrUndefined` on the props): the prop's type, with
/// `undefined` when it is optional, or the applicable index signature's.
fn attribute_target_type(target: &Type, name: &str) -> Option<Type> {
    let Type::Object(object) = target else {
        return None;
    };
    match object.get_property(name) {
        Some(property) => Some(optional_aware_property_type(property)),
        None => object
            .applicable_index_type(surge_ts_types::is_numeric_key(name))
            .cloned(),
    }
}

/// The children branch of tsc's `elaborateJsxComponents`. A lone child is
/// related to the children type when a member of it is not iterable, and
/// reported at the child (TS2747 for text); otherwise a lone child against an
/// iterable-only type, or several against a type with no iterable member, is
/// TS2745/TS2746 at the tag. A body the attributes object does not carry is
/// `unknown` there, so only those two can report it.
fn relate_children(
    body: &JsxBody,
    target: &Type,
    site: &RelationSite<'_>,
    ctx: &CheckerContext,
) -> Vec<(Diagnostic, Option<SyntaxTextSpan>)> {
    let Some(expected) = attribute_target_type(target, &body.name) else {
        return Vec::new();
    };
    if type_contains_unknown_or_any(&expected) {
        return Vec::new();
    }
    let source = match (body.in_attributes, body.children.ty.as_ref()) {
        (true, Some(source)) if !type_contains_unknown_or_any(source) => Some(source),
        (true, _) => return Vec::new(),
        (false, _) => None,
    };
    let relates = source.is_some_and(|source| is_assignable_to(source, &expected));
    let iterable_declared = ctx.lookup_type_declaration("Iterable").is_some();
    match body.children.single {
        Some((kind, span)) if has_non_array_like_member(&expected, iterable_declared) => {
            let Some(source) = source.filter(|_| !relates) else {
                return Vec::new();
            };
            let diagnostic = match kind {
                ChildKind::Text => Diagnostic::ts2747(
                    site.tag_name,
                    &body.name,
                    expected.name(),
                    ctx.file_name.clone(),
                ),
                ChildKind::Expression | ChildKind::Element => {
                    relation_diagnostic(source, &expected, ctx)
                }
            };
            vec![(diagnostic, span.or(site.tag_name_span))]
        }
        Some(_) => (!relates)
            .then(|| {
                (
                    Diagnostic::ts2745(&body.name, expected.name(), ctx.file_name.clone()),
                    site.tag_name_span,
                )
            })
            .into_iter()
            .collect(),
        // Several children against an iterable member are related one by one
        // (`elaborateIterableOrArrayLikeTargetElementwise`): each child's own
        // type against the element at its position, reported at the child.
        None if has_array_like_member(&expected, iterable_declared) => {
            if relates || source.is_none() {
                return Vec::new();
            }
            body.children
                .each
                .iter()
                .enumerate()
                .filter_map(|(index, (kind, ty, span))| {
                    let element = array_like_element_at(&expected, index)?;
                    if type_contains_unknown_or_any(&element) {
                        return None;
                    }
                    let ty = ty.as_ref()?;
                    if is_assignable_to(ty, &element) {
                        return None;
                    }
                    let diagnostic = match kind {
                        ChildKind::Text => Diagnostic::ts2747(
                            site.tag_name,
                            &body.name,
                            element.name(),
                            ctx.file_name.clone(),
                        ),
                        ChildKind::Expression | ChildKind::Element => relation_diagnostic(ty, &element, ctx),
                    };
                    Some((diagnostic, span.or(site.tag_name_span)))
                })
                .collect()
        }
        None => (!relates)
            .then(|| {
                (
                    Diagnostic::ts2746(&body.name, expected.name(), ctx.file_name.clone()),
                    site.tag_name_span,
                )
            })
            .into_iter()
            .collect(),
    }
}

/// The element an array-like children type takes at `index`
/// (`getBestMatchIndexedAccessTypeOrUndefined`): an array's element, a
/// tuple's at the position, the union of those over a union's array-like
/// members.
fn array_like_element_at(ty: &Type, index: usize) -> Option<Type> {
    match ty.peeled() {
        Type::Array(element) => Some(*element),
        Type::Tuple(elements) => elements.get(index).cloned(),
        Type::Union(union) => {
            let elements: Vec<Type> = union
                .types()
                .iter()
                .filter_map(|member| array_like_element_at(member, index))
                .collect();
            (!elements.is_empty()).then(|| union_type(elements))
        }
        _ => None,
    }
}

/// Whether some member of a children type is array-like, the other half of
/// [`has_non_array_like_member`]'s split.
fn has_array_like_member(ty: &Type, iterable_declared: bool) -> bool {
    match ty.peeled() {
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| has_array_like_member(member, iterable_declared)),
        other => !has_non_array_like_member(&other, iterable_declared),
    }
}

/// Whether some member of a children type is not array-like: tsc splits the
/// type by assignability to `Iterable<any>` when the lib declares `Iterable`,
/// and by being an array or tuple otherwise.
fn has_non_array_like_member(ty: &Type, iterable_declared: bool) -> bool {
    match ty.peeled() {
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| has_non_array_like_member(member, iterable_declared)),
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => false,
        Type::String | Type::StringLiteral(_) => !iterable_declared,
        Type::Object(object) => {
            !(iterable_declared
                && object
                    .get_property(surge_ts_types::ITERATION_PROTOCOL_MEMBER)
                    .is_some())
        }
        _ => true,
    }
}

/// tsc's `isExcessPropertyCheckTarget`: an object type, a union with one
/// among its members, or an intersection of nothing else — a primitive or
/// type-variable operand exempts the whole intersection.
fn is_excess_property_check_target(target: &Type) -> bool {
    match target {
        Type::Object(object) if object.is_intersection => object
            .intersection_operands
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(|operand| is_excess_property_check_target(&operand.peeled())),
        Type::Object(object) => object.alias_name.as_deref() != Some("Object"),
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| is_excess_property_check_target(&member.peeled())),
        _ => false,
    }
}

/// tsc's `isKnownProperty` for JSX attributes: declared by the props, answered
/// by an index signature, or declared by any member of a union.
fn is_known_property(target: &Type, name: &str) -> bool {
    match target {
        Type::Object(object) => object.contains_property(name),
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| is_known_property(&member.peeled(), name)),
        _ => true,
    }
}

/// The attributes object as tsc displays it in a relation error: the explicit
/// attributes in order with their values' types, the spreads' members, and
/// the children.
fn attributes_object_name(
    evaluated: &EvaluatedAttributes<'_>,
    body: Option<&JsxBody>,
) -> String {
    let mut properties = evaluated.spread.clone();
    for value in &evaluated.explicit {
        let name: Arc<str> = value.attribute.name.as_str().into();
        properties.shift_remove(&name);
        properties.insert(
            name,
            ObjectProperty::required(value.ty.clone().unwrap_or(Type::Any)),
        );
    }
    if let Some(body) = body {
        properties.insert(
            body.name.as_str().into(),
            ObjectProperty::required(body.children.ty.clone().unwrap_or(Type::Any)),
        );
    }
    if properties.is_empty() {
        return "{}".to_string();
    }
    Type::Object(alloc_object_type(properties, None)).name()
}

/// The attributes object as tsc relates it (`createJsxAttributesTypeFromAttributesProperty`):
/// the spreads' members, then each written attribute's value — or what a later
/// spread writes over it — and the children the body forms. `None` when surge
/// could not type one of them.
/// A hyphenated attribute no member declares is left out: tsc's
/// `isKnownProperty` takes it as known when comparing JSX attributes, so it
/// neither is excess nor fails the weak-type check.
fn attributes_object_type(
    evaluated: &EvaluatedAttributes<'_>,
    body: Option<&JsxBody>,
    members: &[Type],
) -> Option<Type> {
    let mut properties = evaluated.spread.clone();
    for value in &evaluated.explicit {
        let written = value.attribute.name.as_str();
        if is_hyphenated_jsx_name(written)
            && members.iter().all(|member| attribute_target_type(&member.peeled(), written).is_none())
        {
            continue;
        }
        let ty = value.overridden.clone().or_else(|| value.ty.clone())?;
        let name: Arc<str> = written.into();
        properties.shift_remove(&name);
        properties.insert(name, ObjectProperty::required(ty));
    }
    if let Some(body) = body {
        let children = body.children.ty.clone()?;
        properties.insert(body.name.as_str().into(), ObjectProperty::required(children));
    }
    Some(Type::Object(alloc_object_type(properties, None)))
}

fn type_contains_unknown_or_any(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) | Type::Any
    ) || matches!(ty, Type::Union(union) if union.types().iter().any(type_contains_unknown_or_any))
}
