use super::*;

use surge_ts_syntax::ParsedNamedType;

use crate::symbols::TypeDeclarationInfo;

/// Concrete aliases declared by dependencies stay declaration-backed until a
/// semantic consumer needs their structure. This is the critical half of the
/// dependency-surface invariant: eagerly resolving
/// `ComponentPropsWithoutRef<...>` while indexing another declaration's export
/// surface pulls the React/DOM graph into every importing module.
///
/// The escape hatch exists only for before/after profiling and regression
/// isolation; production behavior is lazy.
fn defer_concrete_library_aliases() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_EAGER_DEPENDENCY_ALIASES").as_deref() != Ok("1"))
}

/// Opt-in (`SURGE_DEFER_PLACEHOLDER_INSTANTIATIONS=1`): defer user generic
/// interface instantiations whose arguments are signature placeholders instead
/// of expanding them eagerly. Measured 2026-09-10 on tanstack-query (see
/// docs/perf/TANSTACK-QUERY-PROFILE-2026-09-10.md): the deferred references are
/// peeled straight back by intersection merges, conditional `extends` checks and
/// heritage resolution, so the expansion volume barely moves (interface
/// resolution attempts +6%), zod peak RSS grows ~45% (captured environments
/// outlive the pre-pass), and the clean shapes unmask latent false positives that
/// the degraded expansions used to hide. Kept for the follow-up that makes those
/// consumers lazy too; production behavior stays eager. Read once.
fn defer_placeholder_instantiations() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SURGE_DEFER_PLACEHOLDER_INSTANTIATIONS").as_deref() == Ok("1")
    })
}

/// Kill switch (`SURGE_DISABLE_SIG_CONTEXT_CACHE=1`) for the signature-context
/// instantiation tier, for regression isolation and A/B experiments. Read once.
fn signature_context_cache_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_DISABLE_SIG_CONTEXT_CACHE").as_deref() != Ok("1"))
}

/// Whether a resolved type argument is a sound cache-key component for the
/// signature-context tier. Rejects the `unknown` degradation/placeholder
/// sentinel anywhere in the type (two placeholder-derived `unknown`s from
/// different generic scopes compare equal but are not interchangeable), while
/// the genuine `unknown` keyword stays eligible. References are keyed by
/// (id, arguments) and never peeled, so their arguments are screened too.
/// Budget exhaustion means "cannot prove safe" and makes the site ineligible.
pub(crate) fn signature_cache_safe_argument(ty: &Type, depth: usize, budget: &mut usize) -> bool {
    if depth >= 16 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    match ty {
        Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => false,
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Undefined
        | Type::Null
        | Type::Void
        | Type::Any
        | Type::GenuineUnknown
        | Type::Never
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Array(element) => signature_cache_safe_argument(element, depth + 1, budget),
        Type::Tuple(elements) => elements
            .iter()
            .all(|element| signature_cache_safe_argument(element, depth + 1, budget)),
        Type::OpenTuple(tuple) => tuple
            .leading
            .iter()
            .chain(std::iter::once(tuple.rest.as_ref()))
            .chain(tuple.trailing.iter())
            .all(|element| signature_cache_safe_argument(element, depth + 1, budget)),
        Type::Union(union) => union
            .types()
            .iter()
            .all(|member| signature_cache_safe_argument(member, depth + 1, budget)),
        Type::Function(function) => {
            function
                .parameters()
                .iter()
                .all(|parameter| signature_cache_safe_argument(parameter, depth + 1, budget))
                && signature_cache_safe_argument(function.return_type(), depth + 1, budget)
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .all(|property| signature_cache_safe_argument(&property.ty, depth + 1, budget))
                && object
                    .string_index_type
                    .as_deref()
                    .is_none_or(|index| signature_cache_safe_argument(index, depth + 1, budget))
                && [object.call_signature(), object.construct_signature()]
                    .into_iter()
                    .flatten()
                    .all(|signature| {
                        signature.parameters().iter().all(|parameter| {
                            signature_cache_safe_argument(parameter, depth + 1, budget)
                        }) && signature_cache_safe_argument(
                            signature.return_type(),
                            depth + 1,
                            budget,
                        )
                    })
        }
        Type::Reference(reference) => reference
            .arguments
            .iter()
            .all(|argument| signature_cache_safe_argument(argument, depth + 1, budget)),
    }
}

/// `class D extends Parent {}` where `Parent` is a *value* — a `const` holding a
/// constructor, the mixin shape zod and every `extends mixin(Base)` helper use —
/// names no type declaration at all: TypeScript takes the base type from the
/// value's construct signature. Only the heritage position may fall back onto a
/// value this way; in an ordinary annotation a value used as a type stays an
/// error. A value whose constructor surge cannot model (a union of constructors,
/// `params?.Parent ?? Object`) degrades to the `unknown` sentinel, which leaves
/// the derived type open rather than reporting an unresolved name.
fn resolve_value_heritage_base(
    named_type: &ParsedNamedType,
    ctx: &CheckerContext,
) -> Option<ResolvedType> {
    if crate::program::current_dts_expansion_reason()
        != crate::program::DtsExpansionReason::InterfaceHeritageResolution
    {
        return None;
    }

    // A dotted base whose head is a value, not a namespace (`UI.TreeElement`
    // for `const UI = { TreeElement: class {} }`), reads the member the same
    // way an expression would.
    let value = match ctx.symbols.get(&named_type.name) {
        Some(symbol) => symbol.ty.clone(),
        None => {
            let mut segments = named_type.name.split('.');
            let head = ctx.symbols.get(segments.next()?)?;
            let mut value = head.ty.clone();
            for segment in segments {
                value = value.get_property_access_type(segment)?;
            }
            value
        }
    };
    // An `any` constructor builds `any` whatever its type arguments
    // (`getBaseConstructorTypeOfClass`).
    if !named_type.type_arguments.is_empty() && !matches!(value.peeled(), Type::Any) {
        return None;
    }
    let constructs = |constructor: Type| match constructor {
        Type::Any => Type::Any,
        Type::Object(object) => object
            .construct_signature()
            .map(|signature| signature.return_type().clone())
            .unwrap_or(Type::Unknown),
        _ => Type::Unknown,
    };
    // A base typed by a type variable (`class extends Base` for `Base: T`,
    // the mixin idiom) constructs what its constraint does: tsc resolves the
    // base types through the apparent type of the base constructor.
    let ty = match value.peeled() {
        Type::TypeParameter(parameter) => surge_ts_types::type_variable::active_constraint(&parameter)
            .flatten()
            .map_or(Type::Unknown, |constraint| constructs(constraint.peeled())),
        constructor => constructs(constructor),
    };

    Some(ResolvedType {
        ty,
        had_error: false,
    })
}

/// tsc's `resolveEntityName` stops at a qualifier bound to `unknownSymbol` — an
/// import whose module does not resolve — and the whole reference is the error
/// type, reported nowhere; only its type arguments are still checked. A class
/// deriving from it has no base type at all (`resolveBaseTypesOfClass`), so its
/// inherited members are missing rather than open. Such an import binds the
/// error type as its type meaning and nothing under its name; a type-only
/// namespace import of a module that resolves binds the error type too, but
/// publishes the module's members as `ns.Member`. Inside a declaration file an
/// import surge could not follow stays its own gap, as a miss there does
/// elsewhere.
fn resolve_through_unresolved_import(
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<ResolvedType> {
    if ctx.options.stub_external_modules
        || crate::modules::is_declaration_file_name(&ctx.file_name)
    {
        return None;
    }
    let (head, _) = named_type.name.split_once('.')?;
    // The head is looked up the way any type name is: a declaration body's own
    // scope may hold only its local layer, and the import binding lives in the
    // file's module scope, which the handle lookup falls back to. Without that,
    // `React.Component` in a heritage clause or an interface member was surge's
    // sentinel rather than the error type.
    let error_alias = ctx.lookup_type_declaration_handle(head).is_some_and(|handle| {
        matches!(
            handle.get(),
            TypeDeclarationInfo::Alias(alias) if matches!(alias.body.ty, ParsedType::ErrorType)
        )
    });
    if !error_alias
        || ctx.is_complete_namespace_import_binding(head)
        || declares_qualified_members(head, ctx)
    {
        return None;
    }
    // The arguments report their own errors; the reference is the error type
    // whatever they resolve to, so they do not degrade it.
    for argument in &named_type.type_arguments {
        let _ = resolve_parsed_type(argument.clone(), ctx, resolving, substitution);
    }
    Some(ResolvedType {
        ty: Type::ErrorType,
        had_error: false,
    })
}

/// Whether a table the reference reads declares anything as `head.Member`.
fn declares_qualified_members(head: &str, ctx: &CheckerContext) -> bool {
    let heads = |key: &std::sync::Arc<str>| {
        key.strip_prefix(head)
            .is_some_and(|rest| rest.starts_with('.'))
    };
    ctx.type_declarations.iter().any(|(key, _)| heads(key))
        || ctx.type_declaration_scope.as_ref().is_some_and(|scope| {
            scope
                .layers()
                .iter()
                .any(|layer| layer.iter().any(|(key, _)| heads(key)))
        })
}

/// Opt-in (`SURGE_TYPE_PROBE=<substring>`) probe: prints what a named type
/// resolved to, with its taint, every time a matching name is resolved.
fn type_probe_filter() -> Option<&'static str> {
    static FILTER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FILTER
        .get_or_init(|| std::env::var("SURGE_TYPE_PROBE").ok())
        .as_deref()
}

pub(crate) fn resolve_named_type(
    named_type: std::sync::Arc<ParsedNamedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    if !named_type.type_arguments.is_empty() {
        ctx.mark_check_uncertain();
    }
    // The function resolving a method's signature marks it
    // (`FunctionType::returns_polymorphic_this`); the type itself stays `any`
    // until a member read binds `this` to the receiver.
    if named_type.name == surge_ts_syntax::POLYMORPHIC_THIS_IN_BODY_TYPE_NAME {
        return ResolvedType {
            ty: ctx.polymorphic_this.clone().unwrap_or(Type::Any),
            had_error: false,
        };
    }
    if named_type.name == surge_ts_syntax::POLYMORPHIC_THIS_TYPE_NAME && named_type.type_arguments.is_empty() {
        return ResolvedType {
            ty: substitution
                .get(surge_ts_syntax::POLYMORPHIC_THIS_TYPE_NAME)
                .cloned()
                .unwrap_or(Type::Any),
            had_error: false,
        };
    }
    let Some(filter) = type_probe_filter() else {
        return resolve_named_type_inner(named_type, ctx, resolving, substitution);
    };
    let probed = named_type.name.contains(filter);
    let name = probed.then(|| named_type.name.clone());
    let resolved = resolve_named_type_inner(named_type, ctx, resolving, substitution);
    if let Some(name) = name {
        eprintln!(
            "[type-probe] {name} had_error={} check_phase={} file={} ty={}",
            resolved.had_error,
            crate::program::in_check_phase(),
            ctx.file_name,
            crate::infer::types::cache::lazy_value_trace_shape(&resolved.ty),
        );
    }
    resolved
}

/// tsc's `instantiationDepth` ceiling (checker.go:22452), verbatim.
const MAX_INSTANTIATION_DEPTH: usize = 100;

fn resolve_named_type_inner(
    named_type: std::sync::Arc<ParsedNamedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    // tsc's name resolver stops at the class: a static member may not name a
    // class type parameter, and the name resolves to nothing afterwards.
    if ctx.names_static_forbidden_type_parameter(
        &named_type.name,
        named_type.span,
        resolving.is_empty(),
    ) {
        let diagnostic = crate::spans::diagnostic_with_syntax_span(
            surge_ts_diagnostics::Diagnostic::ts2302(ctx.file_name.clone()),
            named_type.span,
        );
        ctx.push(diagnostic);
        return ResolvedType {
            ty: Type::ErrorType,
            had_error: true,
        };
    }

    if let Some(ty) = substitution.get(&named_type.name) {
        // A type parameter takes no type arguments (`U<string>`); reported
        // where it is written, not where an instantiation binds it.
        if !named_type.type_arguments.is_empty()
            && matches!(ty, Type::TypeParameter(parameter) if *parameter.name == *named_type.name)
        {
            emit_type_is_not_generic(&named_type.name, crate::infer::types::diagnostics::written_reference_span(&named_type.name, named_type.span, &named_type.type_argument_spans), ctx);
            return ResolvedType { ty: Type::ErrorType, had_error: true };
        }
        return ResolvedType {
            ty: ty.clone(),
            // A binding made from a degraded argument stays degraded: reporting
            // it clean here is what let `keyof`/intersection/conditional decide
            // from a type surge never resolved.
            had_error: substitution.is_degraded(&named_type.name),
        };
    }

    if crate::infer::types::report_unexported_namespace_member(&named_type, ctx) {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }

    // `getIntendedTypeFromJSDocTypeReference`: every annotation in a
    // JavaScript file is JSDoc, where a bare `Object` is `any` unless
    // `noImplicitAny` keeps it the global interface.
    if named_type.name == "Object"
        && named_type.type_arguments.is_empty()
        && !ctx.options.no_implicit_any
        && resolving.is_empty()
        && surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
    {
        return ResolvedType {
            ty: Type::Any,
            had_error: false,
        };
    }
    // Likewise a bare `array` is `any[]` and a bare `promise` is
    // `Promise<any>`.
    if matches!(named_type.name.as_str(), "array" | "promise")
        && named_type.type_arguments.is_empty()
        && !ctx.options.no_implicit_any
        && resolving.is_empty()
        && surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
    {
        if named_type.name == "array" {
            return ResolvedType {
                ty: Type::Array(Box::new(Type::Any)),
                had_error: false,
            };
        }
        let promise = std::sync::Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: "Promise".to_string(),
            span: named_type.span,
            type_arguments: vec![ParsedType::Any],
        });
        return resolve_named_type_inner(promise, ctx, resolving, substitution);
    }

    // Look up the declaration through a context-independent handle so resolution
    // can read the (often large) interface/alias payload while `ctx` is borrowed
    // mutably, without deep-cloning it. The handle owns its payload, so the
    // borrowed declaration below is decoupled from `ctx`.
    let handle = ctx
        .lookup_type_declaration_handle(&named_type.name)
        .or_else(|| ctx.import_type_declaration_handle(&named_type.name));
    let Some(handle) = handle else {
        if named_type.name == surge_ts_syntax::EXPRESSION_HERITAGE_BASE {
            return ResolvedType {
                ty: Type::Any,
                had_error: false,
            };
        }
        if let Some(resolved) = resolve_value_heritage_base(&named_type, ctx) {
            return resolved;
        }
        if let Some(resolved) =
            resolve_through_unresolved_import(&named_type, ctx, resolving, substitution)
        {
            return resolved;
        }
        // `class C extends number`: the class check reports the primitive as a
        // value (TS2863), and tsc says nothing more about the base; nor about
        // `extends undefined`, the value whose type is no constructor (TS2507).
        if ctx.resolving_class_heritage
            && matches!(
                named_type.name.as_str(),
                "any" | "string" | "number" | "boolean" | "never" | "unknown" | "undefined"
            )
        {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: true,
            };
        }
        // A qualified reference (`React.Foo`, `Prisma.Bar`) reports only on a
        // head nothing could resolve: surge does not model a namespace's full
        // member surface (`@types/*`, generated clients), so a miss past the
        // head is surge's, not the source's.
        // A class base is an expression: one naming a value (`extends x<T>`
        // over a constructor-typed variable) is the class check's to judge.
        let may_be_unbound_value_base = ctx.resolving_class_heritage
            && crate::program::current_dts_expansion_reason()
                == crate::program::DtsExpansionReason::InterfaceHeritageResolution
            && (ctx.collecting_signatures
                || !named_type.name.contains('.')
                    && (ctx.symbols.get(&named_type.name).is_some()
                        || ctx
                            .module_value_fallback
                            .as_ref()
                            .is_some_and(|fallback| fallback.get(&named_type.name).is_some())));
        if let Some((specifier, qualifier)) = surge_ts_syntax::split_import_type_name(&named_type.name) {
            let reported = report_missing_import_type_member(specifier, qualifier, &named_type, ctx);
            return ResolvedType {
                ty: if reported { Type::ErrorType } else { Type::Unknown },
                had_error: true,
            };
        }
        let reported = !may_be_unbound_value_base
            && if named_type.name.contains('.') {
                crate::infer::types::emit_unresolved_qualified_type_head(&named_type, ctx)
            } else {
                emit_unknown_type_name(&named_type, ctx)
            };
        // `checkTypeReferenceNode` checks the type arguments before it resolves
        // the name, so an unresolved reference still reports inside them.
        if reported {
            for argument in &named_type.type_arguments {
                let _ = resolve_parsed_type(argument.clone(), ctx, resolving, substitution);
            }
        }
        if crate::infer::types::interface::had_error_trace_enabled() {
            eprintln!(
                "[had-error] lookup-miss '{}' scope_installed={} file_in_map={} map_len={} check_phase={} in file {}",
                named_type.name,
                ctx.type_declaration_scope.is_some(),
                ctx.module_scope_by_file
                    .contains_key(ctx.file_name.as_str()),
                ctx.module_scope_by_file.len(),
                crate::program::in_check_phase(),
                ctx.file_name
            );
            eprintln!(
                "[had-error]   scope-layers {}",
                ctx.type_declaration_scope
                    .as_ref()
                    .map(|scope| scope.debug_layer_summary())
                    .unwrap_or_else(|| "<none>".to_string())
            );
        }
        // A namespace member's body resolves without reporting, for the
        // library namespaces surge models only in part; one declared in a
        // program source is fully modelled, so its miss is the source's.
        let own_namespace_miss = !reported
            && ctx.suppress_unknown_type_name()
            && crate::program::in_check_phase()
            && resolving.last().is_some_and(|key| {
                !surge_ts_syntax::is_declaration_file_name(&key.file_name)
                    && !key.file_name.contains("/node_modules/")
            });
        return ResolvedType {
            // tsc has no answer for this name either — it resolves to the
            // error type, which stays `any`-permissive but is still *reported*
            // through (a callback parameter contextually typed by it is an
            // implicit `any`). Surge's own modelling gaps keep `Type::Unknown`,
            // and a miss surge does not report is one of those: a member past a
            // namespace head it only partly models, a name inside a declaration
            // file whose imports it did not follow.
            ty: if reported || own_namespace_miss { Type::ErrorType } else { Type::Unknown },
            had_error: true,
        };
    };
    let declaration = handle.get();

    let has_type_arguments = !named_type.type_arguments.is_empty();
    let is_generic_declaration = match declaration {
        TypeDeclarationInfo::Alias(alias) => !alias.body.type_parameters.is_empty(),
        TypeDeclarationInfo::Interface(interface) => !interface.body.type_parameters.is_empty(),
    };

    if has_type_arguments && !is_generic_declaration {
        // `getTypeReferenceType`: a name bound to tsc's `unknownSymbol` (an
        // import of a module that does not resolve) is the error type whatever
        // its type arguments; only the arguments themselves are still checked.
        if matches!(declaration, TypeDeclarationInfo::Alias(alias)
            if matches!(alias.body.ty, ParsedType::ErrorType))
        {
            let mut had_error = false;
            for argument in &named_type.type_arguments {
                had_error |=
                    resolve_parsed_type(argument.clone(), ctx, resolving, substitution).had_error;
            }
            return ResolvedType {
                ty: Type::ErrorType,
                had_error,
            };
        }
        for argument in &named_type.type_arguments {
            let _ = resolve_parsed_type(argument.clone(), ctx, resolving, substitution);
        }
        let name = match declaration {
            TypeDeclarationInfo::Alias(alias) => &alias.name,
            TypeDeclarationInfo::Interface(interface) => &interface.name,
        };
        emit_type_is_not_generic(name, crate::infer::types::diagnostics::written_reference_span(&named_type.name, named_type.span, &named_type.type_argument_spans), ctx);
        // `getTypeFromClassOrInterfaceReference` / `getTypeFromTypeAliasReference`
        // answer a reported reference with the error type.
        return ResolvedType {
            ty: Type::ErrorType,
            had_error: true,
        };
    }

    let named_type = javascript_filled_type_arguments(named_type, declaration, ctx);
    if let TypeDeclarationInfo::Interface(interface) = declaration
        && is_generic_declaration
        && !ctx.resolving_class_heritage
        && report_interface_type_argument_count(interface, &named_type, ctx)
    {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }

    if !has_type_arguments && !is_generic_declaration {
        let cache_key = type_declaration_resolution_key(declaration);
        if let Some(cached) = get_cached_named_type_resolution(ctx, &cache_key, resolving) {
            return with_enum_base(cached, declaration, &cache_key, &named_type.name, ctx, resolving, substitution);
        }

        // Defer a library-scoped interface: its body (which transitively pulls the
        // mutually-recursive DOM/iterator graph) is expanded only when the
        // reference is peeled, so using the interface as a type argument no longer
        // collapses the enclosing instantiation. User interfaces and all type
        // aliases stay eager so their diagnostics and primitive/union expansions
        // are unchanged.
        if matches!(declaration, TypeDeclarationInfo::Interface(_))
            && declaration_file_is_library_scoped(declaration, ctx)
        {
            let alias_id = type_declaration_alias_id(declaration, &cache_key);
            let display = named_type.name.clone();
            let resolved = ResolvedType {
                ty: make_lazy_type_reference(
                    ctx,
                    &alias_id,
                    &display,
                    handle,
                    cache_key.clone(),
                    named_type.type_arguments.clone(),
                    Vec::new(),
                    substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
                ),
                had_error: false,
            };
            cache_named_type_resolution(ctx, &cache_key, &resolved);
            return resolved;
        }

        mark_named_type_resolution_in_progress(ctx, &cache_key);
        let floor = resolving.len();
        let saved_unknown_cycle = std::mem::replace(&mut ctx.lowest_unknown_cycle_target_index, usize::MAX);
        let resolved = super::substitution::with_reference_argument_spans(&named_type.type_argument_spans, || {
            match declaration {
                TypeDeclarationInfo::Alias(alias) => resolve_type_alias(
                    alias,
                    handle.clone(),
                    named_type.type_arguments.clone(),
                    named_type.span,
                    ctx,
                    resolving,
                    substitution,
                    None,
                    None,
                ),
                TypeDeclarationInfo::Interface(interface) => resolve_interface(
                    interface,
                    handle.clone(),
                    named_type.type_arguments.clone(),
                    named_type.span,
                    type_argument_report_span(&named_type),
                    ctx,
                    resolving,
                    substitution,
                    None,
                    None,
                ),
            }
        });
        // tsc displays a non-generic interface/type-alias by its name in
        // diagnostics (e.g. `'StrictObj'`, not the structural expansion), and
        // treats it nominally: the qualified `file::name` identity lets
        // assignability recognise two resolutions of the same declaration.
        let alias_id = type_declaration_alias_id(declaration, &cache_key);
        let resolved = attach_object_alias_name(resolved, &named_type.name, &alias_id);
        let resolved = alias_of_generic_reference(resolved, declaration, Some(&named_type.name), ctx);
        // An enum type is nominal in tsc and displayed by the enum's name, which
        // the lowered literal-union body cannot express. Wrap it in a nominal
        // reference so the *display* carries the enum while the payload stays the
        // literal union assignability already understands.
        let resolved =
            wrap_enum_member_reference(resolved, declaration, &alias_id, &cache_key, ctx);
        // Wrap the named object in a lazy nominal reference. A non-generic
        // declaration is concrete and context-independent, so its expansion is
        // interned (the wrapped object keeps its `alias_id`/`alias_name`, so a
        // peeled reference still compares nominally and displays by name).
        let resolved =
            wrap_named_object_reference(resolved, &named_type.name, &alias_id, &cache_key, ctx);
        let resolved = mark_non_augmenting_subtype(resolved, declaration);
        let subtree_unknown_cycle = ctx.lowest_unknown_cycle_target_index;
        ctx.lowest_unknown_cycle_target_index = saved_unknown_cycle.min(subtree_unknown_cycle);
        // A resolution that is the sentinel only because it re-entered an
        // enclosing frame mid-resolution answers for that frame, not for the
        // declaration: `type AnyNode = Node<any>` first read from `interface
        // Node<T> { next: AnyNode }` is `unknown` there, and storing it degraded
        // every later read. A partial answer stays stored, as it always was.
        if subtree_unknown_cycle < floor && matches!(resolved.ty, Type::Unknown) {
            forget_named_type_resolution(ctx, &cache_key);
        } else {
            cache_named_type_resolution(ctx, &cache_key, &resolved);
        }
        return with_enum_base(resolved, declaration, &cache_key, &named_type.name, ctx, resolving, substitution);
    }

    // A generic library/dependency instantiation is context-free once its type
    // arguments are fixed: its body binds against its own captured
    // `resolution_scope` and references only the global ambient surface. The real
    // lib typed-array/iterator cluster (`Uint8Array`, `ArrayIterator`,
    // `IteratorObject`, …) is mutually recursive and generic, so without memoizing
    // it every signature mentioning it re-expands the whole tree. Cache library
    // instantiations program-wide, keyed by the resolved type arguments. The
    // store is gated on the resolution being free of *external* cycles (see
    // `lowest_cycle_target_index`) so a cached value matches a standalone
    // resolution and never depends on what an enclosing frame had on the stack.
    let library_scoped = declaration_file_is_library_scoped(declaration, ctx);
    let decl_key = type_declaration_resolution_key(declaration);
    let library_cache_key = library_scoped.then(|| decl_key.clone());
    crate::program::record_generic_instantiation(&decl_key);
    let reference_id = type_declaration_alias_id(declaration, &decl_key);
    let declared_variances = declaration.declared_variances(&|name| ctx.lookup_type_declaration_handle(name));
    // Resolve the type arguments once. The result is reused for the library cache
    // key, the nominal reference identity, AND — via `pre_resolved` below — the
    // authoritative `bind_type_arguments`, so a generic instantiation resolves its
    // arguments exactly once. Repeating an errored argument during binding is
    // exponential on nested generics too. Keep those results and their error
    // flags local to this binding; only clean arguments participate in shared
    // lookup and nominal identity. Errored arguments of a written reference
    // retain their diagnostics; a synthesized one (no span) has nowhere to
    // report them, and its written source already did.
    let (resolved_arguments, degraded_arguments, argument_errors) = {
        let diagnostics_before = ctx.diagnostics().len();
        let mut arguments = Vec::with_capacity(named_type.type_arguments.len());
        let mut errors = Vec::new();
        let deferred = matches!(declaration, TypeDeclarationInfo::Interface(_));
        for (index, argument) in named_type.type_arguments.iter().enumerate() {
            let resolved = if deferred {
                super::structural::resolve_in_deferred_position(ctx, resolving, |ctx, resolving| {
                    resolve_parsed_type(argument.clone(), ctx, resolving, substitution)
                })
            } else {
                resolve_parsed_type(argument.clone(), ctx, resolving, substitution)
            };
            if resolved.had_error {
                if errors.is_empty() {
                    errors.resize(named_type.type_arguments.len(), false);
                }
                errors[index] = true;
            }
            arguments.push(resolved.ty);
        }
        if errors.is_empty() {
            // An instantiation query no signature takes still resolves
            // cleanly, and its TS2635 is reported where it is written.
            let inapplicable_queries: Vec<surge_ts_diagnostics::Diagnostic> = ctx.diagnostics()
                [diagnostics_before..]
                .iter()
                .filter(|diagnostic| {
                    matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(2635))
                })
                .cloned()
                .collect();
            ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
            for diagnostic in inapplicable_queries {
                ctx.push_utility_diagnostic_once(diagnostic);
            }
            (Some(arguments), None, errors)
        } else {
            if named_type.span.is_none() {
                ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
            }
            (None, Some(arguments), errors)
        }
    };
    if library_scoped && let Some(arguments) = resolved_arguments.as_deref() {
        check_library_reference_constraints(
            declaration,
            &named_type,
            arguments,
            substitution,
            ctx,
            resolving,
        );
    }
    let cached_arguments = if library_scoped {
        resolved_arguments.clone()
    } else {
        None
    };
    // A generic interface written with fewer arguments than parameters carries
    // its *defaults* on the reference too, as tsc's type reference does:
    // `Matcher<unknown, 'x'>` is `Matcher<unknown, 'x', 'default', None, 'x'>`.
    // Positional `infer` binding against `Matcher<infer a, infer b, infer c, any,
    // infer d>` and the same-generic argument comparison both read the reference
    // arguments, and with only the written ones the tail captures stayed
    // unbound. The completed list flows into the authoritative bind below as its
    // pre-resolved arguments, so the defaults are still resolved once.
    let reference_arguments = match (declaration, resolved_arguments) {
        (TypeDeclarationInfo::Interface(interface), Some(arguments))
            if complete_default_arguments_enabled()
                && arguments.len() < interface.body.type_parameters.len()
                && interface.body.type_parameters[arguments.len()..]
                    .iter()
                    .all(|parameter| parameter.default_type.is_some()) =>
        {
            complete_interface_default_arguments(
                interface,
                &named_type,
                arguments,
                ctx,
                resolving,
                substitution,
            )
        }
        (_, arguments) => arguments,
    };

    // tsc displays a generic instantiation by its alias form (`Box<string>`), not
    // the structural expansion. Build that display name from the resolved type
    // arguments and tag the resolved object with it for diagnostics.
    let branch_alias = alias_resolves_to_its_branch(declaration);
    // An interface (a class in a `.d.ts` is bound as one) and a *library*
    // alias fill in their defaults here, which is what lets an argument-less
    // `ServerResponse` or `http.RequestListener` take the lazy path. A user
    // alias keeps its eager path: deferring it the same way changed a zod
    // call-site union (`$ZodSuperRefineIssue` against an `Identity<…>`
    // parameter).
    let alias_display_name = generic_instantiation_display_name(
        &named_type,
        substitution,
        declaration.declared_name(),
        match declaration {
            // `library_scoped` reads `file_kinds`, which an analysis-phase
            // context does not carry; the path decides there.
            TypeDeclarationInfo::Alias(alias)
                if library_scoped || is_dependency_declaration_path(&alias.file_name) =>
            {
                &alias.body.type_parameters
            }
            TypeDeclarationInfo::Alias(_) => &[],
            TypeDeclarationInfo::Interface(interface) => &interface.body.type_parameters,
        },
    )
    .filter(|_| !is_intrinsic_no_infer(declaration));

    // An instantiation is interned/short-circuited only when it is *concrete* —
    // no type parameter is bound in any active scope, so a program-wide entry keyed
    // on (declaration, resolved arguments) matches a standalone resolution. A
    // non-generic function body still pushes an (empty) scope via
    // `with_type_parameter_scope`, so an `is_empty()` check per scope (not stack
    // depth) is the right proxy; otherwise a library generic (`new Uint8Array`)
    // built inside a plain body would be treated as non-concrete and eagerly expand
    // its self-referential cluster into a degraded object instead of a nominal lazy
    // reference. A binding scope active anywhere leaves the resolution
    // context-dependent — an argument can name an in-scope parameter, or the body
    // can capture one and (as measured on zod/ofetch) that capture survives in a
    // form that no cheap post-resolution walk reliably detects — so it is not
    // interned.
    let concrete_instantiation = ctx
        .type_parameter_scopes
        .iter()
        .all(|scope| scope.is_empty());

    // Signature-context tier: a *user interface* instantiation made inside an
    // open type-parameter scope (a generic signature or body) whose explicit
    // argument tuple resolved cleanly to placeholder-free types is a pure
    // function of (declaration, arguments) within that context class — nested
    // references defer identically at every such site, the body binds only its
    // own parameters, and check-phase module scopes are stable. Interning those
    // expansions under a namespace-separated key lets the dominant repeated
    // instantiations (measured on zod: ~7.7k clean re-expansions of
    // `$ZodTypeInternals` alone) reuse one expansion instead of re-resolving
    // the body per use site. Eligibility deliberately excludes: the analysis
    // phase (scopes still move between binding rounds), library-scoped
    // declarations (covered by the persistent library tier), defaults
    // (`type_arguments.len() != type_parameters.len()` — defaults resolve under
    // an effective substitution that can see the consumer), syntactic
    // placeholder arguments (the placeholder mark changes body resolution
    // beyond the argument value), `unknown`-carrying arguments (the sentinel
    // collides across contexts), and declarations currently on the `resolving`
    // stack (mid-cycle sites must keep their cycle behavior).
    let signature_context_key = if !concrete_instantiation
        && !library_scoped
        && signature_context_cache_enabled()
        && crate::program::in_check_phase()
        && matches!(declaration, TypeDeclarationInfo::Interface(_))
        && !resolving.iter().any(|key| key == &decl_key)
        && named_type.type_arguments.len()
            == match declaration {
                TypeDeclarationInfo::Alias(alias) => alias.body.type_parameters.len(),
                TypeDeclarationInfo::Interface(interface) => interface.body.type_parameters.len(),
            }
        && named_type
            .type_arguments
            .iter()
            .all(|argument| !parsed_type_is_placeholder_reference(argument, substitution))
        && reference_arguments.as_ref().is_some_and(|arguments| {
            let mut budget = 96usize;
            arguments
                .iter()
                .all(|argument| signature_cache_safe_argument(argument, 0, &mut budget))
        }) {
        // Display-inclusive identity: `Type` equality compares references by
        // (id, arguments) only, so two argument tuples that are nominally equal
        // but *render* differently (a lazy `$ZodErrorMap<T>` captured under one
        // consumer's parameter name vs another's) would otherwise share a
        // bucket and substitute the first site's rendering into every later
        // consumer's diagnostics (the canonical-store display-substitution
        // class; measured as zod message drift). Folding a deep display
        // fingerprint of the tuple into the key keeps them apart, exactly as
        // the interface cache's `DisplayTagged` argument identity does.
        let mut display_hasher = surge_ts_types::fx::FxHasher::default();
        for argument in reference_arguments.as_deref().unwrap_or_default() {
            std::hash::Hasher::write_u64(
                &mut display_hasher,
                crate::speculative::display_type_fingerprint(argument),
            );
        }
        let display_fingerprint = std::hash::Hasher::finish(&display_hasher);
        Some(DeclarationResolutionKey {
            file_name: decl_key.file_name.clone(),
            name: decl_key.name.clone(),
            namespace: crate::context::DeclarationNamespace::TypeSignatureContext,
            // Carried as a field rather than formatted into the name — this runs
            // once per generic instantiation. The memo keys that share this
            // namespace set the high bit, so the two can never collide.
            fingerprint: display_fingerprint & !(1u64 << 63),
        })
    } else {
        None
    };

    // Perf short-circuit: reuse a previously-interned instantiation with the same
    // resolved arguments without re-expanding the body. The interner holds only
    // diagnostic-free, cycle-independent, concrete expansions (see
    // `tag_generic_object_reference`), so a reused entry cannot drop a body
    // diagnostic — the hazard that makes a naive generic cache unsound.
    if let Some(lookup_key) = if concrete_instantiation {
        Some(&decl_key)
    } else {
        signature_context_key.as_ref()
    } && let Some(arguments) = reference_arguments.as_ref()
        && let Some(entry) = lookup_instantiation(ctx, lookup_key, arguments)
    {
        if !concrete_instantiation {
            crate::program::record_program_counter(|c| c.signature_context_generic_hit_count += 1);
        }
        // An interned object with a display form keeps today's nominal wrapping;
        // any other entry (union, reference, primitive, or a display-less bare
        // reference — see the structural interning arm in
        // `tag_generic_object_reference`) is returned structurally, exactly as a
        // fresh expansion would have been, so reuse changes no downstream shape.
        if let Some(display) = alias_display_name.as_deref()
            && matches!(entry.resolved.as_ref(), Type::Object(_))
        {
            return ResolvedType {
                ty: with_declared_variances(
                    make_type_reference(
                        reference_id.clone(),
                        display.to_string(),
                        arguments.clone(),
                        entry.resolved,
                    ),
                    declared_variances,
                ),
                had_error: false,
            };
        }
        if let (Some(display), Type::Function(function)) = (alias_display_name.as_deref(), entry.resolved.as_ref())
            && function.alias_name().is_none()
        {
            return ResolvedType {
                ty: Type::Function(tag_alias_signature(
                    function.clone(),
                    display,
                    &reference_id,
                    Some(arguments),
                    declared_variances,
                )),
                had_error: false,
            };
        }
        return ResolvedType {
            ty: (*entry.resolved).clone(),
            had_error: false,
        };
    }

    let generic_cache_key = cached_arguments.as_ref().and(library_cache_key);
    if let (Some(key), Some(arguments)) = (generic_cache_key.as_ref(), cached_arguments.as_ref()) {
        if let Some(hit) = get_persistent_generic_resolution(ctx, key, arguments) {
            if is_intrinsic_no_infer(declaration) {
                return hit;
            }
            return tag_generic_object_reference(
                hit,
                alias_display_name.as_deref(),
                branch_alias,
                &reference_id,
                &decl_key,
                reference_arguments.clone(),
                declared_variances,
                Some(&decl_key),
                ctx,
            );
        }
    }

    // Defer a generic *alias* instantiation whose type arguments are all fully
    // resolved (`Omit<ConcreteType, "k">`) even when it is built inside a generic
    // body — `concrete_instantiation` is false whenever *any* type-parameter scope
    // is open, so a utility alias with concrete arguments (`Omit`, `Identity`,
    // `Partial`, `Flatten`; each expanded ~185k× on zod) is otherwise eagerly
    // re-expanded from every reference. Deferring it to a lazy reference that
    // captures this site's substitution expands the body only when a consumer peels
    // it. This is gated on every argument being fully resolved: an argument that
    // collapsed to `unknown` is a placeholder (`Normalize<T>` as a function's
    // return type), and freezing a placeholder-dependent expansion into a shared
    // reference would drop the members a later `T`-substitution should have added.
    //
    // Library-scoped aliases defer even at a concrete site: expanding
    // `ComponentPropsWithoutRef<…>` eagerly opens its type-parameter scope, which
    // turns every nested generic interface reference non-concrete and therefore
    // eager, recursing through the React/DOM interface web (unnamed: 18GB in 45s
    // while collecting dependency export tables). User aliases keep the eager
    // path so their body diagnostics and primitive/union expansions are unchanged.
    if (!concrete_instantiation
        || (defer_concrete_library_aliases()
            && declaration_file_is_library_scoped(declaration, ctx)))
        && matches!(declaration, TypeDeclarationInfo::Alias(_))
        && let (Some(display), Some(arguments)) =
            (alias_display_name.as_ref(), reference_arguments.as_ref())
        && arguments.iter().all(|argument| !argument.is_unknown())
    {
        return ResolvedType {
            ty: make_lazy_type_reference(
                ctx,
                &reference_id,
                display,
                handle,
                decl_key.clone(),
                named_type.type_arguments.clone(),
                arguments.clone(),
                substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
            ),
            had_error: false,
        };
    }

    // Defer a library-scoped generic *interface* instantiation
    // (`HTMLAttributes<HTMLElement>`): expand its body only on peel so a use site
    // does not pull the whole DOM/iterator graph and collapse. Generic type
    // aliases stay eager (their bodies reference interfaces, which are themselves
    // deferred, so they stay bounded). A non-concrete site defers under the same
    // gate as the alias branch above — every argument fully resolved and none
    // `unknown` — because inside a generic body a placeholder argument collapses
    // to `unknown`, and freezing a placeholder-dependent expansion into a shared
    // reference would drop the members a later substitution should have added.
    //
    // A `Promise<T>`/`PromiseLike<T>` whose awaited `T` can be `undefined` is
    // collapsed eagerly instead. `await` is erased at parse time, so the eager
    // path's collapse to `T` is the only place the awaited form is produced,
    // and a deferred one stays an opaque reference member that only a peel
    // opens: inside a generic alias body — `type Promisable<T> = T |
    // Promise<T>` instantiated as `Promisable<Client | undefined>` — the union
    // kept a `Promise<T>` member, and every consumer that reads union members
    // structurally (truthiness narrowing, the `?.` short-circuit, the
    // possibly-`undefined` check) missed the `undefined` inside it. The gate is
    // on the awaited nullability on purpose: collapsing *every* promise here
    // erased the `void | Promise<void>` and `return this.promise` distinctions
    // the deferred form happens to keep under the implicit-await model
    // (measured: ky 0→1, zod 21→22, trpc +1). Collapsing costs no expansion.
    // Scoped to a promise written inside a *source* type-alias body, which is
    // where it sits as a union member next to its own awaited type. A promise
    // written as a return annotation, or inside a dependency's own alias
    // (`MaybePromise`, `Thenable` in rollup/vite), keeps deferring: collapsing
    // those shifted an unrelated zod assignability verdict through the shared
    // instantiation store without ever touching zod's sources.
    // The enclosing-declaration lookup runs last: it is a table probe with
    // its own bookkeeping, and running it for every library instantiation
    // moved a zod verdict on its own, with no collapse ever firing.
    let awaited_may_be_undefined = matches!(declaration.declared_name(), "Promise" | "PromiseLike")
        && reference_arguments
            .as_ref()
            .and_then(|arguments| arguments.first())
            .is_some_and(|awaited| match awaited {
                Type::Undefined => true,
                Type::Union(union) => union
                    .types()
                    .iter()
                    .any(|member| matches!(member, Type::Undefined)),
                _ => false,
            })
        && resolving
            .last()
            .and_then(|key| ctx.lookup_type_declaration(&key.name))
            .is_some_and(|info| match info {
                TypeDeclarationInfo::Alias(alias) => !ctx.is_library_scoped_file(&alias.file_name),
                TypeDeclarationInfo::Interface(_) => false,
            });
    if matches!(declaration, TypeDeclarationInfo::Interface(_))
        && declaration_file_is_library_scoped(declaration, ctx)
        && !awaited_may_be_undefined
        && let (Some(display), Some(arguments)) =
            (alias_display_name.as_ref(), reference_arguments.as_ref())
        && (concrete_instantiation || arguments.iter().all(|argument| !argument.is_unknown()))
    {
        return ResolvedType {
            ty: make_lazy_type_reference(
                ctx,
                &reference_id,
                display,
                handle,
                decl_key.clone(),
                named_type.type_arguments.clone(),
                arguments.clone(),
                substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
            ),
            had_error: false,
        };
    }

    // Opt-in: defer a *user* generic interface instantiation whose type
    // arguments name a signature placeholder (`QueryBehavior<TQueryFnData, …>` in
    // the signature of `function f<TQueryFnData>(…)`). The placeholder resolves
    // to `unknown`, so neither interning tier accepts the expansion, and the
    // eager path re-expands the declaration's whole reachable graph for every
    // generic signature that mentions it — three times per function (final
    // module analysis, the check-phase signature pre-pass, and the body check).
    // Generic calls re-resolve the signature from syntax with real bindings, so
    // the placeholder shape is read only by consumers that peel it. The lazy
    // reference expands on first peel and interns under a placeholder-only key
    // (namespace + argument mask) so it can never alias a literal `Foo<unknown>`
    // whose body resolved without the placeholder marks. See the gate's doc
    // comment for why this is not the default.
    if defer_placeholder_instantiations()
        && matches!(declaration, TypeDeclarationInfo::Interface(_))
        && !library_scoped
        && let (Some(display), Some(arguments)) =
            (alias_display_name.as_ref(), reference_arguments.as_ref())
    {
        let placeholder_mask = named_type
            .type_arguments
            .iter()
            .enumerate()
            .filter(|(_, argument)| parsed_type_is_placeholder_reference(argument, substitution))
            .fold(0u64, |mask, (index, _)| mask | (1u64 << (index % 64)));
        if placeholder_mask != 0 {
            crate::program::record_program_counter(|c| {
                c.lazy_reference_placeholder_deferral_count += 1
            });
            let placeholder_key = DeclarationResolutionKey {
                file_name: decl_key.file_name.clone(),
                name: decl_key.name.clone(),
                namespace: crate::context::DeclarationNamespace::PlaceholderInstantiation,
                fingerprint: placeholder_mask,
            };
            return ResolvedType {
                ty: make_lazy_type_reference(
                    ctx,
                    &reference_id,
                    display,
                    handle,
                    placeholder_key,
                    named_type.type_arguments.clone(),
                    arguments.clone(),
                    substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
                ),
                had_error: false,
            };
        }
    }

    // tsc bounds *every* instantiation at one place: `instantiateTypeWithAlias`
    // yields the error type once `instantiationDepth` reaches 100
    // (checker.go:22452), having already returned early for anything that cannot
    // contain type variables — which is why only the generic path below counts.
    //
    // surge's own ceilings sit inside `resolve_type_alias`, behind a gate that is
    // off by default, and `resolve_interface` has nothing but an exact-key cycle
    // check, so an expansion travelling through interfaces and intersections was
    // counted by nothing at all. drizzle's `Omit`/`Readonly`/intersection chain is
    // exactly that path, and it stopped terminating the moment `Record<any, any>`
    // resolved concretely instead of degrading into the sentinel that had been
    // cutting it by accident.
    //
    // The result on a trip is tsc's: the error type. The *diagnostic* is not —
    // tsc raises TS2589 here, but its per-mapper instantiation cache cuts repeats
    // surge re-expands, so surge trips where the source is not excessively deep
    // and reporting it would be a false positive.
    if ctx.instantiation_depth >= MAX_INSTANTIATION_DEPTH {
        return ResolvedType {
            ty: Type::ErrorType,
            had_error: true,
        };
    }

    // Measure cycles triggered by this resolution alone. The declaration is pushed
    // onto `resolving` (at index `floor`) inside `resolve_interface`/`resolve_type_alias`,
    // so a re-entry at `floor` or deeper is an internal self/mutual cycle that
    // resolves deterministically; a re-entry below `floor` reaches an outer frame.
    let floor = resolving.len();
    let saved_lowest_cycle = ctx.lowest_cycle_target_index;
    ctx.lowest_cycle_target_index = usize::MAX;
    // An instantiation is only safe to intern (and later short-circuit) if its body
    // resolution emitted no diagnostics: reusing one that emits would drop the
    // diagnostic. Track both the plain-diagnostic vector and the once-guard set.
    let diagnostics_before_body = ctx.diagnostics().len();
    let utility_keys_before_body = ctx.utility_diagnostic_keys.len();
    let degradation_epoch_before_body = crate::program::expansion_degradation_epoch();
    let bound_arguments = reference_arguments.as_deref().or(degraded_arguments.as_deref());
    let pre_resolved_errors =
        (!argument_errors.is_empty()).then_some(argument_errors.as_slice());

    use crate::infer::types::interface::LAST_EXPANSION_HERITAGE_UNKNOWN_CYCLE;
    ctx.instantiation_depth += 1;
    LAST_EXPANSION_HERITAGE_UNKNOWN_CYCLE.with(|cell| cell.set(usize::MAX));
    let resolved = super::substitution::with_reference_argument_spans(&named_type.type_argument_spans, || {
        match declaration {
            TypeDeclarationInfo::Alias(alias) => resolve_type_alias(
                alias,
                handle.clone(),
                named_type.type_arguments.clone(),
                named_type.span,
                ctx,
                resolving,
                substitution,
                bound_arguments,
                pre_resolved_errors,
            ),
            TypeDeclarationInfo::Interface(interface) => resolve_interface(
                interface,
                handle.clone(),
                named_type.type_arguments.clone(),
                named_type.span,
                type_argument_report_span(&named_type),
                ctx,
                resolving,
                substitution,
                bound_arguments,
                pre_resolved_errors,
            ),
        }
    });
    ctx.instantiation_depth -= 1;
    // A base that was mid-resolution on an outer frame contributed no members
    // (a generic declaration re-entered on the stack resolves to `unknown`), so
    // the expansion is a function of what the caller had open, not of its key.
    let heritage_cycle_free = !matches!(declaration, TypeDeclarationInfo::Interface(_))
        || LAST_EXPANSION_HERITAGE_UNKNOWN_CYCLE.with(std::cell::Cell::get) >= floor;

    let subtree_lowest_cycle = ctx.lowest_cycle_target_index;
    ctx.lowest_cycle_target_index = saved_lowest_cycle.min(subtree_lowest_cycle);

    if resolved.had_error {
        crate::program::record_degraded_resolution();
        if degraded_resolution_trace_enabled() {
            eprintln!(
                "{{\"degradedResolution\":\"{}\",\"file\":\"{}\"}}",
                declaration.declared_name(),
                ctx.file_name
            );
        }
    }

    let body_emitted_diagnostics = ctx.diagnostics().len() != diagnostics_before_body
        || ctx.utility_diagnostic_keys.len() != utility_keys_before_body;
    // A concrete instantiation that resolved cleanly is interned even when its body
    // re-entered an outer frame (`subtree_lowest_cycle < floor`). A clean re-entry
    // embeds the outer declaration as a lazy nominal `Type::Reference` (same id +
    // resolved arguments regardless of stack state), so the interned structural
    // expansion is context-free — the mutually-recursive user clusters (zod's
    // `$ZodType`/`$ZodTypeInternals`, `RawIssue`) were otherwise re-expanded from
    // every sibling reference, hundreds of thousands of times per file. A degraded
    // re-entry (bounded peel, illegal cycle) sets `had_error`/emits, which the
    // remaining guards still exclude, so a thin shape is never frozen program-wide.
    let cacheable =
        concrete_instantiation && !body_emitted_diagnostics && !resolved.had_error && heritage_cycle_free;
    // The signature-context tier stores under stricter gates than the concrete
    // tier: additionally no degradation anywhere in the subtree (a nested
    // bounded peel embeds a transient `unknown` that depends on the consumer's
    // peel-stack state) and a validated value (no embedded sentinel, no
    // context-retaining resolver).
    let signature_context_store_key = signature_context_key.as_ref().filter(|_| {
        subtree_lowest_cycle >= floor
            && !body_emitted_diagnostics
            && !resolved.had_error
            && crate::program::expansion_degradation_epoch() == degradation_epoch_before_body
            // Full physical-cache value validation, embedded-`unknown`
            // rejection included. The sentinel-embedding shapes are exactly the
            // ones that bake a first-resolution-window difference (a member
            // expanded thin while its declaration was mid-first-resolution):
            // tolerating them was measured to change real zod diagnostics
            // (four TS2339s vanished plus one message render), so they stay
            // per-site.
            && validate_physical_interface_cache_value(&resolved.ty).is_ok()
    });
    let intern_key = if cacheable {
        Some(&decl_key)
    } else {
        signature_context_store_key
    };

    if subtree_lowest_cycle >= floor {
        if let (Some(key), Some(arguments)) = (generic_cache_key, cached_arguments) {
            cache_persistent_generic_resolution(ctx, &key, arguments, &resolved);
        }
    }
    if is_intrinsic_no_infer(declaration) {
        return resolved;
    }
    let tagged = tag_generic_object_reference(
        resolved,
        alias_display_name.as_deref(),
        branch_alias,
        &reference_id,
        &decl_key,
        reference_arguments,
        declared_variances,
        intern_key,
        ctx,
    );
    let tagged = mark_non_augmenting_subtype(tagged, declaration);
    alias_of_generic_reference(tagged, declaration, alias_display_name.as_deref(), ctx)
}

/// The lib's `type NoInfer<T> = intrinsic`, which tsc erases to its argument
/// once that is known, so nothing displays as `NoInfer<Dog>`.
fn is_intrinsic_no_infer(declaration: &TypeDeclarationInfo) -> bool {
    matches!(declaration, TypeDeclarationInfo::Alias(alias)
        if &*alias.name == "NoInfer" && matches!(alias.body.ty, ParsedType::Unknown))
}

/// checker.go `getSingleBaseForNonAugmentingSubtype`: one base type and no
/// members of its own (a class base written as a name).
fn mark_non_augmenting_subtype(resolved: ResolvedType, declaration: &TypeDeclarationInfo) -> ResolvedType {
    let TypeDeclarationInfo::Interface(interface) = declaration else {
        return resolved;
    };
    let body = &interface.body;
    let non_augmenting = body.extends.len() == 1
        && body.members.is_empty()
        && body.string_index_type.is_none()
        && body.number_index_type.is_none()
        && body.call_signature.is_none()
        && body.construct_signatures.is_empty()
        && !(interface.is_class_instance && interface.declares_constructor);
    match resolved.ty {
        Type::Reference(reference) if non_augmenting => {
            ResolvedType { ty: Type::Reference(reference.as_non_augmenting_subtype()), had_error: resolved.had_error }
        }
        ty => ResolvedType { ty, had_error: resolved.had_error },
    }
}

/// checker.go `isDeferredTypeReferenceNode`: an alias whose body is a written
/// reference to a generic declaration names that reference
/// (`getAliasSymbolForTypeNode`), so tsc displays it by the alias. A body
/// naming another alias is that alias's instantiation, which keeps its own
/// name (`getTypeFromTypeAliasReference`).
fn alias_of_generic_reference(
    resolved: ResolvedType,
    declaration: &TypeDeclarationInfo,
    display: Option<&str>,
    ctx: &CheckerContext,
) -> ResolvedType {
    let (TypeDeclarationInfo::Alias(alias), Some(display)) = (declaration, display) else {
        return resolved;
    };
    let written_generic_reference = matches!(&alias.body.ty, ParsedType::Named(body)
        if !body.type_arguments.is_empty()
            && !matches!(ctx.lookup_type_declaration(&body.name), Some(TypeDeclarationInfo::Alias(_))));
    match resolved.ty {
        Type::Reference(reference)
            if written_generic_reference
                && !resolved.had_error
                && !reference.arguments.is_empty()
                && reference.enum_owner.is_none()
                && !reference.render_structurally
                && !reference.is_readonly_array()
                && reference.written_tuple().is_none() =>
        {
            ResolvedType { ty: Type::Reference(reference.with_alias_display(display)), had_error: false }
        }
        ty => ResolvedType { ty, had_error: resolved.had_error },
    }
}

/// Opt-in (`SURGE_TRACE_TYPE_EXPANSION=1`) trace of degraded (`had_error`)
/// top-level named resolutions: which declaration degraded, in which file.
fn degraded_resolution_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_TRACE_TYPE_EXPANSION").is_some())
}

/// Wraps a successfully-resolved generic *object* instantiation in a
/// lazy/nominal [`Type::Reference`] over its interned structural expansion, so it
/// carries nominal identity (declaration + resolved arguments) and a `Box<T>`
/// display form without forcing re-expansion at later use sites. Other cacheable
/// expansions (non-object bodies, display-less objects) are interned but returned
/// structurally; errored or argument-unresolved resolutions fall back to the
/// previous structural object tagging.
/// Renders a generic instantiation from its *resolved* arguments
/// (`Dispatch<SetStateAction<number>>`), which is what tsc displays. The
/// syntactic form would keep an unsubstituted type parameter (`Dispatch<S>`).
/// The declaration name is registered qualified for a namespace member
/// (`Hooks.Dispatch`); tsc names it bare.
/// Whether a type alias resolves *to* an existing type rather than creating one.
/// A conditional alias yields whichever branch matched, and tsc displays that
/// branch's own type because no alias symbol attaches to a type that already
/// exists in its own right.
fn alias_resolves_to_its_branch(declaration: &TypeDeclarationInfo) -> bool {
    matches!(
        declaration,
        TypeDeclarationInfo::Alias(alias) if matches!(alias.body.ty, ParsedType::Conditional(_))
    )
}

fn resolved_argument_display(
    decl_key: &DeclarationResolutionKey,
    arguments: &Option<Vec<Type>>,
) -> Option<String> {
    let arguments = arguments
        .as_ref()
        .filter(|arguments| !arguments.is_empty())?;
    let name = decl_key
        .name
        .rsplit_once('.')
        .map_or(decl_key.name.as_ref(), |(_, bare)| bare);
    Some(format!(
        "{name}<{}>",
        arguments
            .iter()
            .map(surge_ts_types::Type::name)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// A signature alias instantiation (`Dispatch<A>`) displayed by its alias
/// form, and carrying its arguments for a relation to compare by the alias's
/// variances when it has any.
fn tag_alias_signature(
    function: surge_ts_types::FunctionType,
    display: &str,
    reference_id: &str,
    arguments: Option<&Vec<Type>>,
    declared_variances: u32,
) -> surge_ts_types::FunctionType {
    let function = function.with_alias_name(display);
    match arguments.filter(|arguments| !arguments.is_empty()) {
        Some(arguments) if declared_variances != 0 => {
            function.with_alias_instantiation(std::sync::Arc::from(reference_id), arguments.clone(), declared_variances)
        }
        _ => function,
    }
}

fn tag_generic_object_reference(
    resolved: ResolvedType,
    display_name: Option<&str>,
    render_structurally: bool,
    reference_id: &str,
    decl_key: &DeclarationResolutionKey,
    arguments: Option<Vec<Type>>,
    declared_variances: u32,
    intern_key: Option<&DeclarationResolutionKey>,
    ctx: &CheckerContext,
) -> ResolvedType {
    // When the parsed arguments were not renderable (e.g. an object-literal type
    // argument), synthesize a display from the resolved argument types so the
    // instantiation still becomes a nominal `Type::Reference` carrying its
    // arguments. That representation is what conditional `infer` capture matches
    // against; without it an object-argument instantiation degraded to a bare
    // structural object and lost its arguments.
    let effective_display: Option<String> = match (display_name, &arguments) {
        (Some(display), _) => Some(display.to_string()),
        (None, Some(arguments)) if !arguments.is_empty() => Some(format!(
            "{}<{}>",
            decl_key.name,
            arguments
                .iter()
                .map(|argument| argument.name())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => None,
    };
    // A generic alias whose body is a signature or a union (`Dispatch<A>`,
    // `SetStateAction<T>`) has no object to tag, but tsc still displays it by the
    // alias form. The handle-local name carries it without touching identity.
    if !resolved.had_error
        && let Some(display) = resolved_argument_display(decl_key, &arguments)
            .or_else(|| effective_display.clone())
            .as_deref()
    {
        match &resolved.ty {
            Type::Function(function) if function.alias_name().is_none() => {
                return ResolvedType {
                    ty: Type::Function(tag_alias_signature(
                        function.clone(),
                        display,
                        reference_id,
                        arguments.as_ref(),
                        declared_variances,
                    )),
                    had_error: false,
                };
            }
            Type::Union(union) if union.alias_name().is_none() => {
                return ResolvedType {
                    ty: Type::Union(union.clone().with_alias_name(display)),
                    had_error: false,
                };
            }
            _ => {}
        }
    }
    match (effective_display.as_deref(), arguments, &resolved.ty) {
        (Some(display), Some(arguments), Type::Object(object)) if !resolved.had_error => {
            // Tag the structural object with the instantiation's display name so a
            // site that peels the reference (e.g. the TS2353 excess-property
            // message) still renders the nominal `Box<string>` form tsc uses,
            // rather than the structural expansion.
            let structural = Type::Object(object.clone().with_alias_name(display));
            // Only intern (making this instantiation reusable by the short-circuit)
            // when it is diagnostic-free and cycle-independent; otherwise keep a
            // private expansion so no other site reuses a context-dependent or
            // diagnostic-suppressing result.
            let interned = if let Some(key) = intern_key {
                record_signature_context_store(key);
                intern_instantiation(ctx, key, &arguments, structural)
            } else {
                std::sync::Arc::new(structural)
            };
            let reference = with_declared_variances(
                make_type_reference(
                    reference_id.to_string(),
                    display.to_string(),
                    arguments,
                    interned,
                ),
                declared_variances,
            );
            ResolvedType {
                // The reference — display included — is unchanged, so intern
                // identity and every sharing decision keyed on it stay exactly as
                // they were; only the rendering is redirected.
                ty: match (render_structurally, reference) {
                    (true, Type::Reference(reference)) => {
                        Type::Reference(reference.rendered_structurally())
                    }
                    (_, reference) => reference,
                },
                had_error: resolved.had_error,
            }
        }
        // Any other cacheable expansion — a non-object body (union, nested
        // reference, primitive: `Exclude<…>`, `Omit<…>` whose body is itself a
        // reference) or a display-less object (a bare `$ZodType` reference whose
        // parameters all defaulted) — is interned under the same soundness
        // conditions, so the next reference to the same instantiation reuses it
        // instead of re-expanding the body (measured on zod: ~490k of 860k body
        // expansions were such cacheable re-expansions). The expansion is still
        // returned structurally — only the short-circuit reuse changes, not the
        // produced shape. An `unknown` result (or a union carrying one) may be a
        // degradation sentinel from a bounded lazy peel, so it is never frozen
        // program-wide.
        (_, Some(arguments), ty) if intern_key.is_some() && !type_may_carry_degradation(ty) => {
            let key = intern_key.expect("guarded by intern_key.is_some()");
            record_signature_context_store(key);
            intern_instantiation(ctx, key, &arguments, resolved.ty.clone());
            resolved
        }
        (display_name, _, _) => tag_generic_object_alias(resolved, display_name),
    }
}

/// The reference built for a generic declaration carries the variances the
/// declaration states for its type parameters (`getVariancesWorker`).
fn with_declared_variances(reference: Type, variances: u32) -> Type {
    match reference {
        Type::Reference(reference) if variances != 0 => {
            Type::Reference(reference.with_declared_variances(variances))
        }
        other => other,
    }
}

fn record_signature_context_store(key: &DeclarationResolutionKey) {
    if key.namespace == crate::context::DeclarationNamespace::TypeSignatureContext {
        crate::program::record_program_counter(|c| c.signature_context_generic_store_count += 1);
    }
}

/// Whether an expansion may embed the `unknown` degradation sentinel (a bounded
/// lazy peel returns `unknown` without setting `had_error`), making it unsafe to
/// intern program-wide even when the resolution was otherwise clean.
fn type_may_carry_degradation(ty: &Type) -> bool {
    match ty {
        Type::Unknown | Type::TypeParameter(_) => true,
        Type::Union(union) => union
            .payload()
            .types
            .iter()
            .any(|member| matches!(member, Type::Unknown | Type::TypeParameter(_))),
        _ => false,
    }
}

/// Wraps a successfully-resolved *non-generic* named object (interface or type
/// alias) in a lazy nominal [`Type::Reference`] over its interned expansion. The
/// reference carries the declaration name for display and the qualified
/// `file\0name` identity for nominal equality. Non-object or errored resolutions
/// pass through unchanged so a `type Id = string` alias stays a plain `string`.
fn wrap_named_object_reference(
    resolved: ResolvedType,
    display: &str,
    reference_id: &str,
    decl_key: &DeclarationResolutionKey,
    ctx: &CheckerContext,
) -> ResolvedType {
    match &resolved.ty {
        Type::Object(_) if !resolved.had_error => {
            let interned = intern_instantiation(ctx, decl_key, &[], resolved.ty.clone());
            ResolvedType {
                ty: make_type_reference(
                    reference_id.to_string(),
                    display.to_string(),
                    Vec::new(),
                    interned,
                ),
                had_error: resolved.had_error,
            }
        }
        _ => resolved,
    }
}

pub(crate) fn declaration_file_is_library_scoped(
    declaration: &TypeDeclarationInfo,
    ctx: &CheckerContext,
) -> bool {
    // NOT memoizable on the declaration: `is_library_scoped_file` consults
    // `ctx.file_kinds`, which differs between the program context and the
    // synthetic contexts lazy resolvers run under, so the same declaration
    // legitimately gets different answers per consumer (measured 467k
    // divergences on tRPC when a per-declaration memo was attempted).
    let file_name = match declaration {
        TypeDeclarationInfo::Alias(alias) => &alias.file_name,
        TypeDeclarationInfo::Interface(interface) => &interface.file_name,
    };
    // A synthetic context carries no `file_kinds`, which closed the lazy path
    // for every dependency declaration reached from a default-bound
    // substitution; the path answers there.
    ctx.is_library_scoped_file(file_name) || is_dependency_declaration_path(file_name)
}

/// [`crate::driver::parsed_type_display`] for a type-parameter *default*, which
/// may also be a `typeof C` query (`RequestListener<Request extends typeof
/// IncomingMessage = typeof IncomingMessage>`); tsc displays it verbatim. A
/// *written* `typeof` argument must not render: it would let a utility alias
/// over a file-local value (`ReturnType<typeof createDeferred>`) defer to a
/// lazy resolver that has no value scope for that file.
fn default_type_display(ty: &ParsedType) -> Option<String> {
    match ty {
        ParsedType::TypeOf(type_of) => Some(format!(
            "typeof {}",
            std::iter::once(type_of.name.as_str())
                .chain(type_of.members.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(".")
        )),
        // A default is often itself an instantiation (`Record<string, any>`,
        // express's `LocalsObj extends Record<string, any> = Record<string,
        // any>`); tsc renders it nested the same way.
        ParsedType::Named(named) if !named.type_arguments.is_empty() => {
            let arguments = named
                .type_arguments
                .iter()
                .map(default_type_display)
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{}<{}>", named.name, arguments.join(", ")))
        }
        other => crate::driver::parsed_type_display(other),
    }
}

/// A declaration file under `node_modules`: the path-based half of
/// [`declaration_file_is_library_scoped`], for contexts whose `file_kinds` map
/// is absent.
pub(crate) fn is_dependency_declaration_path(file_name: &str) -> bool {
    file_name.ends_with(".d.ts")
        && (file_name.contains("/node_modules/") || file_name.contains("\\node_modules\\"))
}

/// Builds the alias display name for a generic instantiation (`Box<string>`)
/// from the *syntactic* type arguments. This renders arguments without resolving
/// them, so it has no diagnostic or caching side effects and — like tsc — keeps a
/// type-alias argument by its name rather than expanding it. A generic written
/// without arguments instantiates its defaults, and tsc displays it with them
/// filled in (`ServerResponse<IncomingMessage>`); rendering that here is also
/// what lets such a reference take the same lazy path as an explicit one instead
/// of eagerly expanding a library class whose heritage may not resolve. Returns
/// `None` when an argument (or a needed default) is not a simple renderable form.
fn generic_instantiation_display_name(
    named_type: &ParsedNamedType,
    substitution: &TypeParameterSubstitution,
    declaration_name: &str,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
) -> Option<String> {
    let mut names = Vec::with_capacity(type_parameters.len());
    if named_type.type_arguments.is_empty() {
        for parameter in type_parameters {
            names.push(default_type_display(parameter.default_type.as_ref()?)?);
        }
        if names.is_empty() {
            return None;
        }
    } else {
        for argument in &named_type.type_arguments {
            names.push(
                bound_argument_display(argument, substitution)
                    .map_or_else(|| crate::driver::parsed_type_display(argument), Some)?,
            );
        }
    }

    Some(format!("{}<{}>", declaration_name, names.join(", ")))
}

/// A written argument that is only a name the enclosing instantiation has bound
/// — a type parameter, or an `infer` capture — renders as what it is bound to.
/// The written text alone named the variable, so `Awaited<R>` inside
/// `T extends (...args: any[]) => infer R ? Awaited<R> : T` reached diagnostics
/// as "type 'Awaited<R>'" long after `R` was known.
fn bound_argument_display(
    argument: &ParsedType,
    substitution: &TypeParameterSubstitution,
) -> Option<String> {
    let ParsedType::Named(named) = argument else {
        return None;
    };
    if !named.type_arguments.is_empty() || substitution.is_placeholder(&named.name) {
        return None;
    }
    let bound = substitution.get(&named.name)?;
    (!bound.is_unmodelled()).then(|| bound.name())
}

/// Opt-in (`SURGE_COMPLETE_DEFAULT_ARGS=1`): carry an interface's resolved
/// defaults on its reference. It is what tsc's reference holds, and positional
/// `infer` binding needs it, but it moves zod (+2, `$ZodInternalIssue<T>` loses
/// `path`) and tanstack-query (+1) — a default bound under the wrong
/// substitution somewhere. Off until that is found.
fn complete_default_arguments_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_COMPLETE_DEFAULT_ARGS").as_deref() == Ok("1"))
}

/// Relates a written reference to a library declaration to its parameters'
/// constraints (see `check_written_type_argument_constraints`), under the scope
/// and namespace prefix the declaration's own binding uses.
fn check_library_reference_constraints(
    declaration: &TypeDeclarationInfo,
    named_type: &ParsedNamedType,
    arguments: &[Type],
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
) {
    let (type_parameters, resolution_scope, file_name, declared_name, name) = match declaration {
        TypeDeclarationInfo::Alias(alias) => (
            &alias.body.type_parameters,
            &alias.resolution_scope,
            &alias.file_name,
            alias.declared_name.as_deref(),
            &alias.name,
        ),
        TypeDeclarationInfo::Interface(interface) => (
            &interface.body.type_parameters,
            &interface.resolution_scope,
            &interface.file_name,
            interface.declared_name.as_deref(),
            &interface.name,
        ),
    };
    if type_parameters.iter().all(|parameter| parameter.constraint.is_none()) {
        return;
    }
    // Validate the written key before lazy alias expansion can lose its diagnostic site.
    if name.as_ref() == "Pick"
        && (crate::default_lib::is_physical_default_lib_file_name(file_name)
            || crate::default_lib::is_generated_default_lib_file_name(file_name)
            || file_name.as_ref() == "<built-in>")
        && !ctx.is_library_scoped_file(&ctx.file_name)
        && !named_type.type_arguments.iter().any(|argument| {
            super::substitution::names_instantiated_parameter(argument, substitution)
        })
        && let [source, key] = arguments
    {
        let keys = surge_ts_types::type_variable::literal_keys_of(source).map(|keys| {
            match source.peeled() {
                Type::Object(object) if object.string_index_type.is_some() =>
                    surge_ts_types::union_type(vec![keys, Type::String, Type::Number]),
                Type::Object(object) if object.number_index_type.is_some() =>
                    surge_ts_types::union_type(vec![keys, Type::Number]),
                _ => keys,
            }
        });
        if let Some(keys) = keys
            && !keys.is_unmodelled()
            && crate::infer::types::judgeable_through_type_variable(key)
            && !surge_ts_types::is_assignable_to(key, &keys)
        {
            crate::infer::types::diagnostics::emit_type_argument_constraint(
                &key.name(),
                &format!("keyof {}", source.name()),
                &crate::checks::relation_chain::chain_beneath_head(key, &keys),
                named_type.type_argument_spans.get(1).copied().or(named_type.span),
                ctx,
            );
        }
        return;
    }
    let declaration_scope = resolution_scope.clone().or_else(|| {
        ctx.module_scope_for_file(file_name)
            .filter(|scope| !scope.is_empty())
    });
    let prefix = crate::infer::types::utility::namespace_member_prefix(declared_name, name);
    if let Some(prefix) = prefix.clone() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    check_written_type_argument_constraints(
        type_parameters,
        &named_type.type_arguments,
        &named_type.type_argument_spans,
        arguments,
        named_type.span,
        substitution,
        (&declaration_scope, &**file_name),
        ctx,
        resolving,
    );
    if prefix.is_some() {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
}

/// The written arguments followed by the declaration's resolved defaults, in
/// parameter order. Falls back to the written list when a default cannot be
/// bound; diagnostics the probe emits are rolled back because the authoritative
/// bind reports them.
fn complete_interface_default_arguments(
    interface: &crate::symbols::InterfaceInfo,
    named_type: &ParsedNamedType,
    written: Vec<Type>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<Vec<Type>> {
    let declaration_scope = interface.resolution_scope.clone().or_else(|| {
        ctx.module_scope_for_file(&interface.file_name)
            .filter(|scope| !scope.is_empty())
    });
    let default_prefix = crate::infer::types::utility::namespace_member_prefix(
        interface.declared_name.as_deref(),
        &interface.name,
    );
    if let Some(prefix) = default_prefix.clone() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    let diagnostics_before = ctx.diagnostics().len();
    let bound = bind_type_arguments(
        &interface.body.type_parameters,
        named_type.type_arguments.clone(),
        &interface.name,
        interface.name_span,
        interface.name_span,
        ctx,
        resolving,
        substitution,
        Some(&written),
        None,
        Some((&declaration_scope, &interface.file_name)),
    );
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    if default_prefix.is_some() {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
    let bound = match bound {
        Some(bound) if !bound.had_error => bound,
        _ => return Some(written),
    };
    let mut completed = Vec::with_capacity(interface.body.type_parameters.len());
    for parameter in &interface.body.type_parameters {
        match bound.substitution.get(&parameter.name) {
            Some(ty) => completed.push(ty.clone()),
            None => return Some(written),
        }
    }
    Some(completed)
}

/// Tags a successfully-resolved generic object instantiation with its alias
/// display name for diagnostics. Display-only: no `alias_id` is attached, so
/// nominal assignability is unchanged. Non-object, errored, or already-named
/// resolutions pass through unchanged.
fn tag_generic_object_alias(resolved: ResolvedType, display_name: Option<&str>) -> ResolvedType {
    match (display_name, &resolved.ty) {
        (Some(name), Type::Object(object))
            if !resolved.had_error && object.alias_name.is_none() =>
        {
            ResolvedType {
                ty: Type::Object(object.clone().with_alias_name(name)),
                had_error: resolved.had_error,
            }
        }
        _ => resolved,
    }
}

/// Tags a resolved object type with the interface/type-alias name it came from
/// so diagnostics display the name (tsc behaviour). Non-object resolutions and
/// errored resolutions pass through unchanged.
/// tsc's `getBaseTypeOfEnumLikeType`: an enum member's literal type widens to
/// its enum (`let x = E.A` is `E`). The enum is reached by the name the member
/// was written under (`E.A` → `E`, `N.Color.Red` → `N.Color`), and only once
/// it is not itself being resolved — the members of its own union go without,
/// which also keeps a member from holding the enum that holds it.
fn with_enum_base(
    resolved: ResolvedType,
    declaration: &TypeDeclarationInfo,
    cache_key: &DeclarationResolutionKey,
    written_name: &str,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let TypeDeclarationInfo::Alias(alias) = declaration else {
        return resolved;
    };
    let Some(enum_name) = alias.enum_name.as_deref() else {
        return resolved;
    };
    let is_member = alias
        .name
        .rsplit_once(&format!("{enum_name}."))
        .is_some_and(|(_, member)| !member.is_empty());
    let Type::Reference(reference) = &resolved.ty else {
        return resolved;
    };
    if !is_member || resolved.had_error || reference.enum_base.is_some() {
        return resolved;
    }
    let Some(owner) = reference.enum_owner.clone() else {
        return resolved;
    };
    let Some((prefix, _)) = written_name.rsplit_once('.') else {
        return resolved;
    };
    let Some(handle) = ctx.lookup_type_declaration_handle(prefix) else {
        return resolved;
    };
    let enum_key = type_declaration_resolution_key(handle.get());
    if resolving.contains(&enum_key) || named_type_resolution_in_progress(ctx, &enum_key) {
        return resolved;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let base = resolve_named_type_inner(
        std::sync::Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: prefix.to_string(),
            span: None,
            type_arguments: Vec::new(),
        }),
        ctx,
        resolving,
        substitution,
    );
    ctx.truncate_diagnostics(diagnostics_before);
    if base.had_error {
        return resolved;
    }
    let base = match base.ty {
        Type::Reference(base) if base.enum_owner.as_deref() == Some(&*owner) => base,
        _ => return resolved,
    };
    let Type::Reference(reference) = resolved.ty else {
        return resolved;
    };
    let attached = ResolvedType {
        ty: Type::Reference(reference.with_enum_base(Type::Reference(base))),
        had_error: false,
    };
    cache_named_type_resolution(ctx, cache_key, &attached);
    attached
}

/// Wraps an enum-lowered alias resolution in a nominal reference whose display
/// is tsc's enum form (`import("<module>").Color`). The reference id stays
/// per-member (`Color.Red`), so two members never compare equal to each other.
fn wrap_enum_member_reference(
    resolved: ResolvedType,
    declaration: &TypeDeclarationInfo,
    reference_id: &str,
    decl_key: &DeclarationResolutionKey,
    ctx: &CheckerContext,
) -> ResolvedType {
    let TypeDeclarationInfo::Alias(alias) = declaration else {
        return resolved;
    };
    let Some(enum_name) = alias.enum_name.as_deref() else {
        return resolved;
    };
    if resolved.had_error
        || matches!(
            resolved.ty,
            Type::Reference(_) | Type::Unknown | Type::TypeParameter(_)
        )
    {
        return resolved;
    }
    // tsc qualifies an exported enum's type with the module it came from and
    // names a file-local one bare.
    // A member type prints as `Enum.Member`.
    let own_name = match alias.name.rsplit_once(&format!("{enum_name}.")) {
        Some((_, member)) if !member.is_empty() => format!("{enum_name}.{member}"),
        _ => enum_name.to_string(),
    };
    // An enum a namespace exports is registered under its qualified name
    // (`First.E`) and is not a module export: tsc names it bare.
    let namespace_member = alias.name.as_ref() != own_name.as_str()
        && alias.name.strip_suffix(own_name.as_str()).is_some_and(|prefix| prefix.ends_with('.'));
    let display = if alias.enum_exported && !namespace_member {
        format!(
            "import({:?}).{own_name}",
            module_path_for_display(&alias.file_name)
        )
    } else {
        own_name.clone()
    };
    let numeric = enum_resolution_is_numeric(&resolved.ty);
    let interned = intern_instantiation(ctx, decl_key, &[], resolved.ty.clone());
    let reference = make_type_reference(reference_id.to_string(), display, Vec::new(), interned);
    // The enum is its own declaration's reference id: a member's id less
    // its `.member` suffix, so two same-named enums of different namespaces
    // stay distinct.
    let member_suffix = own_name.strip_prefix(enum_name).filter(|suffix| !suffix.is_empty());
    let owner: std::sync::Arc<str> = match member_suffix {
        Some(suffix) => reference_id.strip_suffix(suffix).unwrap_or(reference_id).into(),
        None => reference_id.into(),
    };
    let members = match member_suffix {
        Some(suffix) => alias.name.strip_suffix(suffix).and_then(|enum_key| match ctx.lookup_type_declaration(enum_key) {
            Some(TypeDeclarationInfo::Alias(enum_alias)) if enum_alias.enum_name.as_deref() == Some(enum_name) => {
                Some(enum_members_of(enum_alias))
            }
            _ => None,
        }),
        None => Some(enum_members_of(alias)),
    };
    let reference = match (numeric, reference) {
        (true, Type::Reference(reference)) => Type::Reference(reference.numeric_enum().with_enum_owner(owner)),
        (false, Type::Reference(reference)) => Type::Reference(reference.with_enum_owner(owner)),
        (_, reference) => reference,
    };
    let reference = match (members, reference) {
        (Some(members), Type::Reference(reference)) => Type::Reference(reference.with_enum_members(members)),
        (_, reference) => reference,
    };
    ResolvedType {
        ty: reference,
        had_error: false,
    }
}

/// An enum's members as `isEnumTypeRelatedTo` reads them: each name with its
/// constant value, or none for a computed member.
fn enum_members_of(alias: &crate::symbols::TypeAliasInfo) -> std::sync::Arc<surge_ts_types::EnumMembers> {
    let members = alias
        .enum_members
        .iter()
        .map(|(name, ty)| {
            let value = match ty {
                ParsedType::NumberLiteral(value) => {
                    Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType { value: value.clone() }))
                }
                ParsedType::StringLiteral(value) => Some(Type::StringLiteral(value.clone())),
                _ => None,
            };
            (std::sync::Arc::<str>::from(name.as_str()), value)
        })
        .collect();
    std::sync::Arc::new(surge_ts_types::EnumMembers { regular: !alias.enum_is_const, members })
}

/// Whether a lowered `enum` body is numeric — every member (or, for the enum
/// type itself, every union arm) is a number. tsc lets any `number` flow into a
/// numeric enum and rejects `string` for a string enum, so the distinction is
/// the whole point of the marker.
fn enum_resolution_is_numeric(ty: &Type) -> bool {
    match ty {
        Type::Number | Type::NumberLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(enum_resolution_is_numeric),
        _ => false,
    }
}

/// The module path tsc writes inside `import("…")`: the declaring file without
/// its declaration extension.
fn module_path_for_display(file_name: &str) -> &str {
    [".d.ts", ".d.mts", ".d.cts", ".tsx", ".ts", ".mts", ".cts"]
        .iter()
        .find_map(|extension| file_name.strip_suffix(extension))
        .unwrap_or(file_name)
}

fn attach_object_alias_name(resolved: ResolvedType, name: &str, alias_id: &str) -> ResolvedType {
    match resolved.ty {
        // Tag the nominal identity even when the resolution errored (a cyclic
        // member may have collapsed to `unknown`): the object is still this named
        // declaration, so assignability can recognise two of its resolutions.
        Type::Object(object) => {
            let object = object.with_alias_id(alias_id);
            // tsc displays a named type by its name even when a deeply cyclic
            // member did not fully resolve (e.g. `URL`, whose `searchParams`
            // cluster is mutually recursive). Keep the display name whenever the
            // object resolved to a real shape; only a collapse to an empty object
            // (no recoverable structure) falls back to the structural form.
            // The `object` keyword is intrinsic and carries no alias
            // (`WeakKey` resolving to `object` prints as `object`).
            let intrinsic_object = object.non_primitive
                && object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature.is_none()
                && object.construct_signature.is_none()
                && !object.is_intersection;
            let object = if intrinsic_object || resolved.had_error && object.properties.is_empty() {
                object
            } else {
                object.with_alias_name(name)
            };
            ResolvedType {
                ty: Type::Object(object),
                had_error: resolved.had_error,
            }
        }
        // A union or function alias (`type Level = "a" | "b"`,
        // `type Fn = (x: string) => void`) is displayed by its name too. The name
        // rides on the handle, so the interned payload stays shared and
        // assignability still sees the plain union/signature.
        Type::Union(union) if !resolved.had_error => ResolvedType {
            ty: Type::Union(union.with_alias_name(name)),
            had_error: resolved.had_error,
        },
        Type::Function(function) if !resolved.had_error => ResolvedType {
            ty: Type::Function(function.with_alias_name(name)),
            had_error: resolved.had_error,
        },
        ty => ResolvedType {
            ty,
            had_error: resolved.had_error,
        },
    }
}

#[cfg(test)]
mod signature_context_cache_tests {
    use crate::program::{SourceFileInput, check_program_with_stats_and_jobs};

    fn fixture(count: usize) -> Vec<SourceFileInput> {
        let mut files = vec![SourceFileInput {
            file_name: "core.ts".to_string(),
            source_text:
                "export interface Internals<O, I> { out: O; inp: I; parse(value: I): O; }\n"
                    .to_string(),
        }];
        files.extend((0..count).map(|i| SourceFileInput {
            file_name: format!("use_{i}.ts"),
            source_text: format!(
                "import {{ Internals }} from \"./core\";\n\
                 export function pick_{i}<T>(seed: T, internals: Internals<string, number>): string {{\n\
                 \x20 return internals.out;\n\
                 }}\n"
            ),
        }));
        files
    }

    /// The zero-hit regression: repeated identical user-generic instantiations
    /// inside generic signatures must produce nonzero signature-context stores
    /// and hits, and hits must dominate once the tuple repeats. The test runs
    /// in its own process, so the global counters are isolated.
    #[test]
    fn repeated_signature_context_instantiations_hit_after_first_store() {
        if crate::test_support::run_in_own_process(
            module_path!(),
            "repeated_signature_context_instantiations_hit_after_first_store",
        ) {
            return;
        }
        // Safety: set before any checker thread is spawned in this isolated,
        // single-test process. `check_program` re-derives the counters gate
        // from this env var at the start of every run.
        unsafe { std::env::set_var("SURGE_TIMINGS", "1") };
        let result =
            check_program_with_stats_and_jobs(fixture(10), crate::CheckerOptions::default(), 1);
        assert!(
            result.diagnostics.is_empty(),
            "fixture is clean: {:?}",
            result.diagnostics
        );
        let counters = crate::metrics::snapshot_program_counters();
        assert!(
            counters.signature_context_generic_store_count >= 1,
            "expected at least one signature-context store, got {}",
            counters.signature_context_generic_store_count
        );
        assert!(
            counters.signature_context_generic_hit_count >= 8,
            "expected repeated tuples to hit (10 modules, 1 unique tuple), got {} hits / {} stores",
            counters.signature_context_generic_hit_count,
            counters.signature_context_generic_store_count
        );
    }

    /// Unknown-carrying tuples (a placeholder-typed argument) must never be
    /// stored in or read from the signature-context tier.
    #[test]
    fn placeholder_argument_tuples_are_never_cached() {
        if crate::test_support::run_in_own_process(
            module_path!(),
            "placeholder_argument_tuples_are_never_cached",
        ) {
            return;
        }
        // Safety: set before any checker thread is spawned in this isolated,
        // single-test process. `check_program` re-derives the counters gate
        // from this env var at the start of every run.
        unsafe { std::env::set_var("SURGE_TIMINGS", "1") };
        let mut files = vec![SourceFileInput {
            file_name: "core.ts".to_string(),
            source_text: "export interface Internals<O, I> { out: O; inp: I; }\n".to_string(),
        }];
        files.extend((0..6).map(|i| SourceFileInput {
            file_name: format!("use_{i}.ts"),
            source_text: format!(
                "import {{ Internals }} from \"./core\";\n\
                 export function poly_{i}<T>(seed: T, internals: Internals<T, T>): T {{\n\
                 \x20 return internals.out;\n\
                 }}\n"
            ),
        }));
        let _ = check_program_with_stats_and_jobs(files, crate::CheckerOptions::default(), 1);
        let counters = crate::metrics::snapshot_program_counters();
        assert_eq!(
            counters.signature_context_generic_store_count, 0,
            "placeholder tuples must not be stored"
        );
        assert_eq!(
            counters.signature_context_generic_hit_count, 0,
            "placeholder tuples must not hit"
        );
    }
}

/// An import type naming what its module does not export: TS2694 on the
/// module (`"path"`, or `"path".export=` for an `export =` module), or TS1340
/// when it names the module itself and only an `export =` of a type would be
/// one. Reported only where the module's types were bound, a module file's
/// import types.
fn report_missing_import_type_member(
    specifier: &str,
    qualifier: &str,
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    let Some(namespace) = ctx.import_type_namespace(&ctx.file_name, specifier) else {
        return false;
    };
    if qualifier.is_empty() {
        // A class assigned to `module.exports` is a type too
        // (`getTypeFromImportTypeNode` takes its type meaning).
        if matches!(namespace.peeled(), Type::Object(object) if object.construct_signature().is_some()) {
            return false;
        }
        let diagnostic = crate::spans::diagnostic_with_syntax_span(
            surge_ts_diagnostics::Diagnostic::ts1340(specifier, ctx.file_name.clone()),
            named_type.span,
        );
        ctx.push(diagnostic);
        return true;
    }
    let path = match &namespace {
        Type::Object(object) => object
            .alias_name
            .as_deref()
            .and_then(|alias| alias.strip_prefix("typeof import(\""))
            .and_then(|rest| rest.strip_suffix("\")"))
            .map(|path| format!("\"{}\"", path.trim_end_matches(".ts").trim_end_matches(".js"))),
        _ => None,
    }
    .unwrap_or_else(|| {
        let module = ctx.import_type_module_path(&ctx.file_name, specifier);
        format!("\"{}\".export=", module.as_deref().unwrap_or(specifier))
    });
    let member = qualifier.split('.').next().unwrap_or(qualifier);
    // tsc reports at the qualifier's first name, which ends the written text
    // when no type arguments follow.
    let member_span = named_type.span.map(|span| match (named_type.type_arguments.is_empty(), span.end.checked_sub(qualifier.len())) {
        (true, Some(start)) if start >= span.start => {
            surge_ts_syntax::TextSpan { start, end: start + member.len() }
        }
        _ => span,
    });
    let diagnostic = crate::spans::diagnostic_with_syntax_span(
        surge_ts_diagnostics::Diagnostic::ts2694(path, member, ctx.file_name.clone()),
        member_span,
    );
    ctx.push(diagnostic);
    true
}

/// tsc's `fillMissingTypeArguments` for a JavaScript reference to a generic
/// class or interface: a missing argument is `any`, and a wrong count is
/// reported only under `noImplicitAny` (`isJsImplicitAny`). A default of
/// `{}` or `unknown` is `any` there too; the arguments are filled at the
/// reference so the instantiation is keyed by them.
fn javascript_filled_type_arguments(
    named_type: std::sync::Arc<ParsedNamedType>,
    declaration: &TypeDeclarationInfo,
    ctx: &mut CheckerContext,
) -> std::sync::Arc<ParsedNamedType> {
    let TypeDeclarationInfo::Interface(interface) = declaration else {
        return named_type;
    };
    if !surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        return named_type;
    }
    let type_parameters = &interface.body.type_parameters;
    // `isJsImplicitAny` skips the arity check both ways, and the arguments
    // past the parameters are dropped.
    if !ctx.options.no_implicit_any
        && !type_parameters.is_empty()
        && named_type.type_arguments.len() > type_parameters.len()
    {
        let mut truncated = (*named_type).clone();
        truncated.type_arguments.truncate(type_parameters.len());
        return std::sync::Arc::new(truncated);
    }
    let min = super::substitution::min_type_argument_count(type_parameters);
    // Only a run of such defaults right after the written arguments can be
    // filled here: a default before one would have to be resolved in the
    // declaring scope, which `bind_type_arguments` does.
    let any_defaults = type_parameters
        .iter()
        .skip(named_type.type_arguments.len().max(min))
        .take_while(|parameter| parameter.default_type.as_ref().is_some_and(is_empty_object_or_unknown_type))
        .count();
    if named_type.type_arguments.len() >= min && any_defaults == 0 {
        return named_type;
    }
    if named_type.type_arguments.len() < min
        && ctx.options.no_implicit_any
        && (!ctx.resolving_class_heritage || crate::infer::types::is_javascript_heritage_reference(ctx))
    {
        report_interface_type_argument_count(interface, &named_type, ctx);
    }
    let mut filled = (*named_type).clone();
    let filled_count = filled.type_arguments.len().max(min) + any_defaults;
    filled.type_arguments.resize(filled_count, ParsedType::Any);
    std::sync::Arc::new(filled)
}

/// A default `fillMissingTypeArguments` turns into `any` in JavaScript: one
/// identical to the empty object type or to `unknown`.
fn is_empty_object_or_unknown_type(ty: &ParsedType) -> bool {
    match ty {
        ParsedType::UnknownKeyword => true,
        ParsedType::Object(object) => {
            object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature.is_none()
                && object.construct_signature.is_none()
                && !object.non_primitive
        }
        _ => false,
    }
}

/// tsc's `getTypeFromClassOrInterfaceReference` arity check, at the reference
/// and naming the declared type with its parameters (`Box<T, U>`).
fn report_interface_type_argument_count(
    interface: &crate::symbols::InterfaceInfo,
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    let type_parameters = &interface.body.type_parameters;
    let min = super::substitution::min_type_argument_count(type_parameters);
    let count = named_type.type_arguments.len();
    if count >= min && count <= type_parameters.len() {
        return false;
    }
    let parameter_names: Vec<&str> = type_parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect();
    let display = format!("{}<{}>", named_type.name, parameter_names.join(", "));
    crate::infer::types::emit_generic_arity(
        &display,
        min,
        type_parameters.len(),
        crate::infer::types::diagnostics::written_reference_span(&named_type.name, named_type.span, &named_type.type_argument_spans),
        ctx,
    );
    true
}

/// Where a reference's argument errors go: the arguments themselves when the
/// reference records them apart from its name (a JS class's `@augments` tag),
/// otherwise the reference.
fn type_argument_report_span(named_type: &ParsedNamedType) -> Option<surge_ts_syntax::TextSpan> {
    match named_type.type_argument_spans.as_slice() {
        [first, ..] if named_type.type_argument_spans.len() == named_type.type_arguments.len() => Some(*first),
        _ => named_type.span,
    }
}
