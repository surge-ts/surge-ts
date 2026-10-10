//! Interface declaration resolution and instance type construction.

use super::*;

use surge_ts_syntax::{ParsedFunctionType, ParsedInterfaceMember, ParsedNamedType, ParsedType};
use surge_ts_types::{FunctionType, ObjectProperty, PropertyMap, Type, current_program_type_store};

use crate::context::{CheckerContext, DeclarationResolutionKey};
use crate::default_lib::{is_generated_default_lib_file_name, is_physical_default_lib_file_name};
use crate::metrics::{alloc_function_type, alloc_object_type};
use crate::symbols::{InterfaceInfo, TypeDeclarationHandle};

/// Opt-in `SURGE_IFACE_CACHE_ALL=1`: extend the instantiation cache beyond the
/// physical default lib to every interface declaration, check phase only.
///
/// Kept off (re-measured 2026-09-11). It looked like a 7% win, but only because
/// `InterfaceInstantiationKey` carried no environment component — two run-wide
/// booleans, so user interfaces were shared across resolution environments,
/// which the memory-lifetime rules forbid. With the environment discriminator
/// now in the key the cache stops hitting on this corpus and the gate costs
/// **+1.2%** on tanstack-query. Reach for the module-instantiation memo, whose
/// fingerprint was designed for cross-environment sharing, not for this.
fn extended_interface_cache_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_IFACE_CACHE_ALL").is_some())
}

/// Opt-in `SURGE_LIB_MEMBER_CACHE=1`: extend the per-MEMBER instantiation
/// cache (method/overload signatures) beyond the physical default lib to
/// every library-scoped declaration file, check phase only — before it,
/// module scopes and augmentations still move between binding rounds. Unlike
/// `SURGE_IFACE_CACHE_ALL` this does NOT extend the whole-interface cache;
/// only member keys and declaration templates are formed for these files.
///
/// Sealed off-by-default (2026-08-28): diagnostics stay 5-corpus identical
/// and tRPC method-cache hits rise 2.5k -> 120k, but user time is ~+2% — key
/// formation, composite-key hashing, and value validation cost more than the
/// avoided member resolutions now that the check-phase degraded peel is
/// pinned. Re-measure only alongside a cheaper member-key identity.
fn extended_member_cache_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_LIB_MEMBER_CACHE").is_some())
}

/// Opt-in `SURGE_LAZY_IFACE_MEMBERS=1`: defer a library interface's structured
/// PROPERTY member annotations to lazy references resolved on first read (see
/// docs/perf/MEMBER-LAZY-EXPANSION.md). Method members stay eager in Stage 1.
fn lazy_interface_members_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_LAZY_IFACE_MEMBERS").is_some())
}

/// The member-annotation deferral tier: structured shapes whose eager
/// resolution recurses. Primitives, literals, and keywords resolve in
/// nanoseconds and deferring them exposes unforced references to structural
/// checks; `ParsedType::Named` already defers itself through the library
/// reference tiers; anything containing `typeof` resolves against value
/// tables the captured environment drops.
fn defer_interface_member_annotation(annotation: &ParsedType, optional: bool) -> bool {
    match annotation {
        // A union stays eager when the OPTIONAL read path would re-wrap it
        // (`property.ty | undefined`): the eager path normalizes the combined
        // union on construction, while a deferred union resolves too late for
        // the outer dedup and leaks `T | undefined | undefined` shapes into
        // verdicts and renders.
        ParsedType::Union(members) => {
            !optional
                && !members.iter().any(|member| {
                    matches!(member, ParsedType::Undefined)
                        || matches!(member, ParsedType::Named(named) if named.name == "null")
                })
                && !crate::modules::annotation_contains_typeof(annotation)
        }
        ParsedType::Object(_)
        | ParsedType::Tuple(_)
        | ParsedType::Intersection(_)
        | ParsedType::KeyOf(_)
        | ParsedType::IndexedAccess(_)
        | ParsedType::Mapped(_)
        | ParsedType::Conditional(_)
        | ParsedType::TemplateLiteral(_) => !crate::modules::annotation_contains_typeof(annotation),
        ParsedType::Array(element) => defer_interface_member_annotation(element, false),
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleInstantiationMemoMode {
    Off,
    /// `SURGE_IFACE_MODULE_MEMO=analysis`: the module-analysis phase only,
    /// where 99% of the repeated expansions are (measured on zod: 1.14M of
    /// 1.15M degradation events).
    Analysis,
    /// Default since 2026-09-11: also memoize during the check phase, worth
    /// -6.1% instructions on tanstack-query. The earlier note here recorded this
    /// as diagnostic-affecting on trpc; that no longer reproduces — ky, ofetch,
    /// zod, trpc and tanstack-query are byte-identical with and without it, and
    /// the full oracle sweep is unchanged, measured in one binary on both arms.
    /// (The original measurement compared two binaries built from a tree that
    /// moved between them.) `SURGE_IFACE_MODULE_MEMO=analysis` restores the
    /// analysis-phase-only behaviour.
    All,
}

fn module_instantiation_memo_mode() -> ModuleInstantiationMemoMode {
    static MODE: std::sync::OnceLock<ModuleInstantiationMemoMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(
        || match std::env::var("SURGE_IFACE_MODULE_MEMO").as_deref().ok() {
            Some("0") => ModuleInstantiationMemoMode::Off,
            Some("analysis") => ModuleInstantiationMemoMode::Analysis,
            _ => ModuleInstantiationMemoMode::All,
        },
    )
}

fn module_instantiation_memo_active() -> bool {
    match module_instantiation_memo_mode() {
        ModuleInstantiationMemoMode::Off => false,
        ModuleInstantiationMemoMode::Analysis => !crate::program::in_check_phase(),
        ModuleInstantiationMemoMode::All => true,
    }
}

/// Identity of everything outside `(declaration, substitution)` that a body
/// expansion can read. The components mirror `DeclarationEnvironmentKey`, which
/// is the checker's existing statement of what makes two resolution contexts
/// interchangeable:
///
/// * the phase and stage/attempt counters;
/// * whether any type-parameter scope is open — that flips
///   `concrete_instantiation` for every nested reference, and so whether they
///   defer or expand;
/// * the versioned identity of the live declaration table and of every layer of
///   the scope the body actually resolves under. An interface merged by a later
///   augmentation, or a scope layer that gained declarations between binding
///   rounds, bumps a version and lands in a different bucket — without this,
///   `NodeJS.ProcessEnv` (merged from several files, its index signature
///   inherited through `extends Dict<string>`) reused a pre-merge expansion;
/// * the per-file module-scope map, which `module_scope_for_file` falls back to
///   when a declaration has no pre-attached scope, and which
///   `with_type_declaration_scope` leaves as the *caller's* scope when the
///   declaration's own is `None`.
/// The per-module table instance id is dropped from the fingerprint by default
/// (`SURGE_IFACE_MEMO_TABLE=1` restores it) where the body provably does not
/// read the consumer's table — the condition that lets the program-lifetime
/// memo in `cache.rs` share one expansion across the modules of a type graph.
/// The module-scoped memo is wiped per module regardless, so the narrower key
/// changes nothing there (measured: 290,880 -> 290,879 expansions).
/// EXPERIMENT KNOB (`SURGE_IFACE_MEMO_FRAGMENTS=1`).
fn interface_memo_fragment_identity_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_IFACE_MEMO_FRAGMENTS").as_deref() == Ok("1"))
}

fn interface_memo_table_identity_dropped() -> bool {
    static DROPPED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DROPPED.get_or_init(|| std::env::var("SURGE_IFACE_MEMO_TABLE").as_deref() != Ok("1"))
}

fn module_instantiation_memo_fingerprint(
    interface: &InterfaceInfo,
    local_substitution: &TypeParameterSubstitution,
    declaration_effective_scope: &Option<Arc<crate::symbols::TypeDeclarationScope>>,
    ctx: &CheckerContext,
) -> u64 {
    use std::hash::Hasher;
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    hasher.write_u8(u8::from(crate::program::in_check_phase()));
    hasher.write_u8(u8::from(
        ctx.type_parameter_scopes
            .iter()
            .all(|scope| scope.is_empty()),
    ));
    hasher.write_u64(ctx.resolution_stage_counter);
    hasher.write_u64(ctx.environment_attempt);
    // The consumer's local declaration table is part of the fingerprint only
    // when the body can actually read it. `binding.rs` clones the table per
    // module and `TypeDeclarationTable::clone` mints a fresh instance id, so
    // including it unconditionally makes every consumer of one type graph
    // disagree on the key — which is exactly what `lookup_ignores_local_table`
    // already calls out as "it makes expansions depend on which module
    // triggered them (defeating cross-module expansion reuse)".
    //
    // The predicate has to describe the state the *body* resolves under, not the
    // caller's: this runs before `with_file_name` / `with_type_declaration_scope`
    // are entered below, so it predicts what they will install — a cross-file
    // resolution (already crossed, or about to cross into the interface's own
    // file) with a scope installed (the declaration's own, or an inherited one).
    // The two cases write different discriminants so they cannot collide.
    let body_crosses_file =
        ctx.cross_file_resolution_depth > 0 || *ctx.file_name != *interface.file_name;
    let body_has_scope =
        declaration_effective_scope.is_some() || ctx.type_declaration_scope.is_some();
    let drop_table_identity =
        interface_memo_table_identity_dropped() && body_crosses_file && body_has_scope;
    let (table_instance, table_version) = ctx.type_declarations.snapshot_identity();
    if !drop_table_identity {
        hasher.write_u64(table_instance);
    }
    // The mutation counter is kept even when the instance id is dropped.
    // `snapshot_identity` bundles two different things: `instance_id`, the
    // per-clone identity that `binding.rs` mints fresh for every module (the
    // consumer-local shadowing the predicate above proves is bypassed), and
    // `version`, which `TypeDeclarationTable::upsert` bumps on every
    // declaration-merging write. Merging is *not* bypassed by resolving under
    // the declaration's own scope — an interface augmented after one consumer
    // expanded it has a different body for the next one. Clones inherit the
    // version, so it stays comparable across the per-module clones.
    hasher.write_u64(table_version);
    // EXPERIMENT: a merged interface's body depends on which fragments the
    // consumer can see — `declare module` augmentation lands in a copy of the
    // export table, so two consumers legitimately disagree about the same
    // declaration. Hash the fragment list so a merged declaration can only
    // share with a consumer that sees the same fragments.
    if interface_memo_fragment_identity_enabled() {
        hasher.write_usize(interface.body.declaration_fragments.len());
        for fragment in &interface.body.declaration_fragments {
            hasher.write(fragment.file_name.as_bytes());
            hasher.write_usize(fragment.declaration_start);
        }
    }
    hasher.write_usize(if ctx.module_scope_by_file.is_empty() {
        0
    } else {
        Arc::as_ptr(&ctx.module_scope_by_file) as *const () as usize
    });
    let effective_scope = declaration_effective_scope
        .as_ref()
        .or(ctx.type_declaration_scope.as_ref());
    match effective_scope {
        None => hasher.write_u8(0),
        Some(scope) => {
            hasher.write_u8(1);
            for layer in scope.layers() {
                let (instance, version) = layer.snapshot_identity();
                hasher.write_u64(instance);
                hasher.write_u64(version);
            }
        }
    }
    for (name, ty) in local_substitution.iter() {
        hasher.write(name.as_bytes());
        hasher.write_u8(u8::from(local_substitution.is_placeholder(name)));
        // Display-inclusive: `Type` equality compares references by
        // (id, arguments), so two nominally equal argument tuples that *render*
        // differently must not share an entry (the canonical-store
        // display-substitution class).
        hasher.write_u64(crate::speculative::display_type_fingerprint(ty));
    }
    hasher.write(interface.name.as_bytes());
    hasher.write(interface.file_name.as_bytes());
    hasher.finish()
}

/// Opt-in (`SURGE_TRACE_HAD_ERROR=1`) per-origin trace of every `had_error`
/// creation site: lookup misses (with scope/phase provenance), interface
/// base/member/argument taint, alias cycles/arguments, conditional failures,
/// and unbound `infer` captures. Read once; zero cost when unset (every
/// probe short-circuits on its error condition before consulting the gate).
pub(crate) fn had_error_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_TRACE_HAD_ERROR").is_some())
}

/// Whether a class's `extends` names a non-class generic declaration with a
/// type-argument count that declaration does not take as a type reference.
fn class_base_arguments_select_constructors(base: &ParsedNamedType, ctx: &CheckerContext) -> bool {
    let Some(crate::symbols::TypeDeclarationInfo::Interface(interface)) =
        ctx.lookup_type_declaration(&base.name)
    else {
        return false;
    };
    let type_parameters = &interface.body.type_parameters;
    let min_type_argument_count = super::resolve::min_type_argument_count(type_parameters);
    !interface.is_class_instance
        && !type_parameters.is_empty()
        && (base.type_arguments.len() < min_type_argument_count
            || base.type_arguments.len() > type_parameters.len())
}

fn is_concrete_back_edge_argument(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Any
            | Type::GenuineUnknown
            | Type::Never
            | Type::String
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
    )
}

/// Whether a resolved type argument is free of the degradation sentinel as far
/// as its structure shows without peeling a reference. Past the budget it is
/// not.
fn argument_free_of_sentinel(ty: &Type, depth: usize, budget: &mut usize) -> bool {
    if depth >= 16 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    match ty {
        Type::Unknown => false,
        Type::Array(element) => argument_free_of_sentinel(element, depth + 1, budget),
        Type::Tuple(elements) => elements
            .iter()
            .all(|element| argument_free_of_sentinel(element, depth + 1, budget)),
        Type::OpenTuple(tuple) => tuple
            .leading
            .iter()
            .chain(std::iter::once(tuple.rest.as_ref()))
            .chain(tuple.trailing.iter())
            .all(|element| argument_free_of_sentinel(element, depth + 1, budget)),
        Type::Union(union) => union
            .types()
            .iter()
            .all(|member| argument_free_of_sentinel(member, depth + 1, budget)),
        Type::Function(function) => {
            function
                .parameters()
                .iter()
                .all(|parameter| argument_free_of_sentinel(parameter, depth + 1, budget))
                && argument_free_of_sentinel(function.return_type(), depth + 1, budget)
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .all(|property| argument_free_of_sentinel(&property.ty, depth + 1, budget))
                && object
                    .string_index_type
                    .as_deref()
                    .is_none_or(|index| argument_free_of_sentinel(index, depth + 1, budget))
        }
        Type::Reference(reference) => reference
            .arguments
            .iter()
            .all(|argument| argument_free_of_sentinel(argument, depth + 1, budget)),
        _ => true,
    }
}

fn generic_interface_display_name(interface: &InterfaceInfo) -> String {
    let name = interface.declared_name.as_deref().unwrap_or(&interface.name);
    let parameters = &interface.body.type_parameters;
    if parameters.is_empty() {
        return name.to_string();
    }
    let parameters = parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}<{parameters}>")
}

/// The bare name a merged interface's fragments declare it by: a declaration
/// reached through an import or a namespace carries a qualified or renamed
/// name of its own.
fn fragment_declared_name(interface: &InterfaceInfo) -> &str {
    let name = interface.declared_name.as_deref().unwrap_or(&interface.name);
    name.rsplit('.').next().unwrap_or(name)
}

/// The scope a declaration without one of its own resolves its body under:
/// its file's module scope, unless that scope does not declare it. A
/// declaration in a `declare module` block's own table carries no scope (the
/// table and the block scope would hold each other) and is reached through the
/// block scope installed where it is referenced; the names around it are that
/// scope's from the layer declaring it outward, as Go resolves a declaration's
/// names where it is written.
fn unscoped_declaration_scope(
    file_name: &str,
    name: &str,
    handle: &TypeDeclarationHandle,
    ctx: &CheckerContext,
) -> Option<Arc<crate::symbols::TypeDeclarationScope>> {
    let module_scope = ctx.module_scope_for_file(file_name).filter(|scope| !scope.is_empty())?;
    if module_scope
        .get_handle(name)
        .is_some_and(|declared| same_declaration(&declared, handle))
    {
        return Some(module_scope);
    }
    let installed = ctx.type_declaration_scope.as_ref()?;
    let layers = installed.layers();
    match layers
        .iter()
        .position(|layer| layer.get_handle(name).is_some_and(|declared| declared.ptr_eq(handle)))
    {
        Some(position) => {
            let scope = crate::symbols::TypeDeclarationScope::new(layers[position..].to_vec())
                .with_lexical_layers(installed.lexical_layer_count().saturating_sub(position));
            Some(Arc::new(if installed.is_preliminary() { scope.preliminary() } else { scope }))
        }
        None => Some(module_scope),
    }
}

/// Whether two handles name one declaration. The module scope can hold its
/// own copy of a declaration (rescoped or merged), so payload identity alone
/// misses it; a declaration is its file and the span of its name.
fn same_declaration(a: &TypeDeclarationHandle, b: &TypeDeclarationHandle) -> bool {
    if a.ptr_eq(b) {
        return true;
    }
    let site = |handle: &TypeDeclarationHandle| match handle.get() {
        crate::symbols::TypeDeclarationInfo::Interface(info) => (info.file_name.clone(), info.name_span),
        crate::symbols::TypeDeclarationInfo::Alias(info) => (info.file_name.clone(), info.name_span),
    };
    let (a_file, a_span) = site(a);
    let (b_file, b_span) = site(b);
    a_span.is_some() && a_span == b_span && a_file == b_file
}

/// An object-typed member whose properties read the declaration they sit in
/// (Drizzle's `_: { columns: T['columns']; inferSelect: InferSelectModel<Table<T>> }`):
/// those properties are deferred as a self-reading member is, so reading
/// `Table<C>['_']['columns']` does not resolve `inferSelect`, which re-enters
/// this very expansion and caches the cycle's degraded answer. tsc reads each
/// property's type on demand. `None` when no property reads the declaration.
#[allow(clippy::too_many_arguments)]
fn resolve_object_member_deferring_self_reads(
    object: &surge_ts_syntax::ParsedObjectType,
    member_name: &str,
    reading: SelfReading<'_>,
    member_index: usize,
    substitution_fingerprint: u64,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<ResolvedType> {
    let deferred: Vec<&surge_ts_syntax::ParsedObjectTypeProperty> = {
        let is_alias = |name: &str| {
            matches!(ctx.lookup_type_declaration(name), Some(crate::symbols::TypeDeclarationInfo::Alias(_)))
        };
        object
            .properties
            .iter()
            .filter(|property| {
                !property.is_method
                    && crate::symbols::reads_members_of(&property.ty, reading.own_name, &property.name, &is_alias)
            })
            .collect()
    };
    if deferred.is_empty() {
        return None;
    }
    let mut reduced = object.clone();
    reduced
        .properties
        .retain(|property| !deferred.iter().any(|deferred| deferred.name == property.name));
    let resolved = resolve_parsed_type(ParsedType::Object(Arc::new(reduced)), ctx, resolving, substitution);
    let Type::Object(mut resolved_object) = resolved.ty else {
        return None;
    };
    let mut properties = surge_ts_types::PropertyMap::default();
    for property in &object.properties {
        let entry = if let Some(deferred) = deferred.iter().find(|deferred| deferred.name == property.name) {
            let key = format!("{member_name}.{}", deferred.name);
            let ty = super::cache::make_lazy_member_annotation_reference(
                ctx,
                super::cache::LazyMemberIdentity {
                    interface_name: reading.interface_name,
                    declaration_start: reading.declaration_start,
                    member_index,
                    member_name: &key,
                    component: None,
                    substitution_fingerprint,
                },
                &deferred.ty,
                substitution,
            );
            let entry = if deferred.optional {
                surge_ts_types::ObjectProperty::optional(ty)
            } else {
                surge_ts_types::ObjectProperty::required(ty)
            };
            entry.with_readonly(deferred.readonly)
        } else {
            match resolved_object.properties.get(property.name.as_str()) {
                Some(resolved) => resolved.clone(),
                None => continue,
            }
        };
        properties.insert(property.name.as_str().into(), entry);
    }
    for (name, resolved) in resolved_object.properties.iter() {
        if !properties.contains_key(name.as_ref()) {
            properties.insert(name.clone(), resolved.clone());
        }
    }
    let rebuilt = surge_ts_types::ObjectType::new(properties, None);
    resolved_object.properties = rebuilt.properties;
    resolved_object.property_map_id = rebuilt.property_map_id;
    Some(ResolvedType {
        ty: Type::Object(resolved_object),
        had_error: resolved.had_error,
    })
}

/// An expansion whose properties read the declaration itself, which defers
/// them the way tsc resolves members only when read.
#[derive(Clone, Copy)]
pub(crate) struct SelfReading<'a> {
    interface_name: &'a str,
    declaration_start: usize,
    own_name: &'a str,
}

pub(crate) fn resolve_interface(
    interface: &InterfaceInfo,
    handle: TypeDeclarationHandle,
    type_arguments: Vec<ParsedType>,
    reference_span: Option<surge_ts_syntax::TextSpan>,
    argument_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
    pre_resolved_arguments: Option<&[Type]>,
    pre_resolved_errors: Option<&[bool]>,
) -> ResolvedType {
    crate::program::record_interface_resolution_attempt();
    let declaration_key = super::cache::interface_resolution_key(interface);
    if let Some(index) = resolving.iter().position(|name| name == &declaration_key) {
        // A recursive interface (`interface Node { next: Node }`) is always valid in
        // tsc: a reference to it is `createTypeReference(target, typeArguments)`,
        // whose members are instantiated only when read
        // (`resolveTypeReferenceMembers`). The self-edge resolves to a lazy nominal
        // reference to the same declaration, so a member/assignability check through
        // it peels back to the real shape instead of silently passing on `unknown`;
        // the lazy peel stack bounds re-expansion.
        //
        // A generic self-edge carries the arguments the caller resolved under this
        // frame's substitution (`next: List<T>` inside `List<number>` is
        // `List<number>`, `then(): IPromise<any>` is `IPromise<any>`). Arguments
        // that did not resolve, or that hold the degradation sentinel, give the
        // reference no identity, so that back-edge stays `unknown` with a
        // (suppressed) note.
        ctx.note_resolution_cycle(index);
        if interface.body.type_parameters.is_empty() {
            return ResolvedType {
                ty: make_recursive_cycle_reference(
                    ctx,
                    &interface.name,
                    handle,
                    declaration_key,
                    type_arguments,
                    pre_resolved_arguments,
                    substitution,
                ),
                had_error: false,
            };
        }
        let mut budget = 96usize;
        // Arguments that name no type parameter (`Router<any>` behind `type
        // AnyRouter = Router<any>`) mean the same under every frame's
        // substitution, so a back-edge to a frame further out keeps them too.
        let immediate_parent = index + 1 == resolving.len();
        let concrete_arguments = pre_resolved_arguments
            .is_some_and(|arguments| arguments.iter().all(is_concrete_back_edge_argument));
        if (immediate_parent || concrete_arguments)
            && let Some(arguments) = pre_resolved_arguments.filter(|arguments| {
            arguments
                .iter()
                .all(|argument| argument_free_of_sentinel(argument, 0, &mut budget))
        }) {
            let name = interface.declared_name.as_deref().unwrap_or(&interface.name);
            let display = if arguments.is_empty() {
                name.to_string()
            } else {
                let arguments = arguments.iter().map(Type::name).collect::<Vec<_>>().join(", ");
                format!("{name}<{arguments}>")
            };
            // The reference peels under the scope installed where it is made,
            // which past the immediate parent is another declaration's; the
            // frame it re-enters resolved its body under the declaration's own.
            let own_scope = if immediate_parent {
                None
            } else {
                interface.resolution_scope.clone().or_else(|| {
                    unscoped_declaration_scope(&interface.file_name, &interface.name, &handle, ctx)
                })
            };
            let installed_scope = own_scope.map(|scope| ctx.type_declaration_scope.replace(scope));
            let reference = make_recursive_cycle_reference(
                ctx,
                &display,
                handle,
                declaration_key,
                type_arguments,
                Some(arguments),
                substitution,
            );
            if let Some(installed_scope) = installed_scope {
                ctx.type_declaration_scope = installed_scope;
            }
            return ResolvedType {
                ty: reference,
                had_error: false,
            };
        }
        ctx.note_unknown_resolution_cycle(index);
        emit_type_declaration_cycle(&interface.name, interface.name_span, ctx);
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }

    resolving.push(declaration_key.clone());
    // An EMPTY per-file fallback must not clobber the caller's installed scope:
    // an ambient-module file (`declare module "http" { … }` in @types/node)
    // registers its declarations in the block's export table, so its per-file
    // scope is empty. A sibling declaration reached through the installed block
    // scope carries no resolution_scope of its own; replacing the block scope
    // with the empty fallback made every name in the sibling's body miss.
    let declaration_effective_scope = interface
        .resolution_scope
        .clone()
        .or_else(|| unscoped_declaration_scope(&interface.file_name, &interface.name, &handle, ctx));
    // See `resolve_type_alias`: a default names its namespace siblings bare.
    let default_prefix = crate::infer::types::utility::namespace_member_prefix(
        interface.declared_name.as_deref(),
        &interface.name,
    );
    if let Some(prefix) = default_prefix.clone() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    // `getTypeFromClassOrInterfaceReference` reports a wrong argument count on
    // the reference, naming the generic type with its parameters (`C<T>`); a
    // lookup surge synthesized has no reference to report it on.
    let generic_display_name = generic_interface_display_name(interface);
    let bound = bind_type_arguments(
        &interface.body.type_parameters,
        type_arguments,
        &generic_display_name,
        reference_span,
        argument_span,
        ctx,
        resolving,
        substitution,
        pre_resolved_arguments,
        pre_resolved_errors,
        Some((&declaration_effective_scope, &interface.file_name)),
    );
    if default_prefix.is_some() {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
    let Some(bound_arguments) = bound else {
        resolving.pop();
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    };
    let arguments_had_error = bound_arguments.had_error;
    let local_substitution = bound_arguments.substitution;

    if is_generated_default_lib_file_name(&interface.file_name) {
        match &*interface.name {
            "Array" | "ReadonlyArray" => {
                let element_type = local_substitution.get("T").cloned().unwrap_or(Type::Any);
                resolving.pop();
                let array = Type::Array(Box::new(element_type));
                return ResolvedType {
                    ty: if &*interface.name == "ReadonlyArray"
                        && crate::infer::types::resolve::readonly_arrays_enabled()
                    {
                        crate::infer::types::resolve::readonly_reference(array)
                    } else {
                        array
                    },
                    had_error: false,
                };
            }
            "Uint8Array" => {
                resolving.pop();
                return ResolvedType {
                    ty: Type::Array(Box::new(Type::Number)),
                    had_error: false,
                };
            }
            "Map" => {
                resolving.pop();
                return ResolvedType {
                    ty: generated_default_lib_map_instance_type(),
                    had_error: false,
                };
            }
            "Promise" | "PromiseLike" if !crate::checks::call::promise_nominal_enabled() => {
                let ty = local_substitution
                    .get("T")
                    .cloned()
                    .unwrap_or(Type::Unknown);
                resolving.pop();
                return ResolvedType {
                    ty,
                    had_error: false,
                };
            }
            _ => {}
        }
    }

    if is_physical_default_lib_file_name(&interface.file_name) {
        match &*interface.name {
            // The lib declares `Array`/`ReadonlyArray` as interfaces, but they
            // model the same structure as the `T[]` syntax (which lowers to
            // `Type::Array`). Collapse them so an `Array<T>` annotation and a
            // `T[]` annotation are the same type and compare assignable; their
            // members (`map`, `concat`, `length`, …) are served by the array
            // apparent-type path. This mirrors the generated-lib behaviour.
            "Array" | "ReadonlyArray" => {
                let element_type = local_substitution.get("T").cloned().unwrap_or(Type::Any);
                resolving.pop();
                let array = Type::Array(Box::new(element_type));
                return ResolvedType {
                    ty: if &*interface.name == "ReadonlyArray"
                        && crate::infer::types::resolve::readonly_arrays_enabled()
                    {
                        crate::infer::types::resolve::readonly_reference(array)
                    } else {
                        array
                    },
                    had_error: false,
                };
            }
            // `await` is stripped at parse time, so model `Promise<T>` /
            // `PromiseLike<T>` as their resolved value `T` (an implicit await
            // everywhere). `.then()`-style chaining on a raw promise remains a
            // documented limitation.
            "Promise" | "PromiseLike" if !crate::checks::call::promise_nominal_enabled() => {
                let ty = local_substitution
                    .get("T")
                    .cloned()
                    .unwrap_or(Type::Unknown);
                resolving.pop();
                return ResolvedType {
                    ty,
                    had_error: false,
                };
            }
            _ => {}
        }
    }

    // Under `noLib`, a configured `types` package may replace the standard lib
    // (roblox-ts's `@rbxts/compiler-types` redeclares `Array`/`ReadonlyArray`
    // with the same structural shape as `T[]`). Collapse them exactly like the
    // physical lib so `Array<T>` unifies with `T[]` and the self-referential
    // generic interface never re-enters its own resolution (which would
    // otherwise degrade to `unknown` via the generic cycle path above).
    //
    // `Promise` is intentionally NOT collapsed here: roblox-ts code uses the
    // Promise object surface (`.then`/`.catch`) directly, so mapping it to its
    // awaited value (as the physical-lib path does for implicit-await) would
    // strip those members and over-report.
    if ctx.options.no_lib
        && matches!(&*interface.name, "Array" | "ReadonlyArray")
        && crate::program::is_configured_types_global_file(&interface.file_name, &ctx.options.types)
    {
        let element_type = local_substitution.get("T").cloned().unwrap_or(Type::Any);
        resolving.pop();
        return ResolvedType {
            ty: Type::Array(Box::new(element_type)),
            had_error: false,
        };
    }

    let physical_default_lib = is_physical_default_lib_file_name(&interface.file_name);
    // Opt-in `SURGE_IFACE_CACHE_ALL=1`: extend the instantiation cache beyond
    // the physical default lib to every interface declaration, but only during
    // the check phase — before it, module scopes and augmentations still move
    // between binding rounds, so the canonical key (declaration + arguments +
    // options) does not pin the resolution inputs. The phase marker is
    // program-global because most check-phase resolution runs in
    // environment-recovered contexts. Every existing gate (clean-only
    // interning, value validation, display-tagged argument identity) applies.
    let cache_eligible = physical_default_lib
        || (extended_interface_cache_enabled() && crate::program::in_check_phase());
    let member_cache_extension = !physical_default_lib
        && extended_member_cache_enabled()
        && physical_interface_member_cache_enabled()
        && {
            let check_phase = crate::program::in_check_phase();
            let library_scoped = check_phase
                && (ctx.is_library_scoped_file(&interface.file_name)
                    || crate::program::is_library_classified_file_name(&interface.file_name));
            crate::program::record_program_counter(|c| {
                if !check_phase {
                    c.member_cache_ext_not_check_phase_count += 1;
                } else if !library_scoped {
                    c.member_cache_ext_not_library_scoped_count += 1;
                }
            });
            if check_phase
                && !library_scoped
                && std::env::var_os("SURGE_MEMBER_CACHE_TRACE").is_some()
            {
                eprintln!("[member-cache-ext] not-lib file={}", interface.file_name);
            }
            library_scoped
        };
    let collect_all_interface_identities = crate::program::dts_expansion_trace_enabled();
    let form_interface_key =
        cache_eligible || member_cache_extension || collect_all_interface_identities;
    let stable_declaration = if form_interface_key {
        stable_interface_declaration_id(interface).ok()
    } else {
        None
    };
    let interface_key = if form_interface_key {
        // Reuse the stable id computed above instead of rebuilding it inside
        // the key constructor.
        let key = match stable_declaration.clone() {
            Some(declaration) => canonical_physical_interface_key_with_declaration(
                interface,
                &local_substitution,
                ctx,
                declaration,
                (cache_eligible || member_cache_extension) && !physical_default_lib,
            ),
            None => Err(InterfaceCacheSkipReason::UnstableDeclaration),
        };
        match key {
            Ok(key) => Some(key),
            Err(reason) => {
                record_interface_cache_skip(reason);
                None
            }
        }
    } else {
        None
    };
    let declaration_template = if (physical_default_lib || member_cache_extension)
        && physical_interface_member_cache_enabled()
        && let Some(declaration) = stable_declaration.as_ref()
    {
        physical_interface_declaration_template(ctx, interface, declaration)
    } else {
        None
    };
    if member_cache_extension {
        crate::program::record_program_counter(|c| {
            if declaration_template.is_none() || interface_key.is_none() {
                c.member_cache_ext_template_unavailable_count += 1;
            } else {
                c.member_cache_ext_keyed_count += 1;
            }
        });
    }
    let creation_before = crate::program::type_creation_snapshot();
    if cache_eligible {
        if !physical_interface_cache_enabled() {
            record_interface_cache_skip(InterfaceCacheSkipReason::Disabled);
        } else if let Some(key) = interface_key.as_ref() {
            if let Some(cached) = lookup_physical_interface_instantiation(ctx, key) {
                crate::program::record_program_counter(|c| {
                    c.physical_interface_cache_hit_count += 1
                });
                resolving.pop();
                crate::program::record_interface_resolution_result(
                    stable_declaration,
                    Some(key),
                    true,
                    true,
                    interface.body.extends.len(),
                    interface.body.members.len(),
                    creation_before,
                );
                return ResolvedType {
                    ty: (*cached).clone(),
                    had_error: false,
                };
            }
            crate::program::record_program_counter(|c| c.physical_interface_cache_miss_count += 1);
        }
    }

    // A body's own type variables differ by owner, not by name, and the memo's
    // fingerprint renders them by name.
    let instantiated_with_variables = local_substitution
        .iter()
        .any(|(_, ty)| surge_ts_types::type_variable::mentions_type_variable(ty));
    let module_memo_key = (module_instantiation_memo_active() && !instantiated_with_variables).then(|| {
        module_instantiation_memo_key(
            &declaration_key,
            module_instantiation_memo_fingerprint(
                interface,
                &local_substitution,
                &declaration_effective_scope,
                ctx,
            ),
        )
    });
    if let Some(key) = module_memo_key.as_ref()
        && let Some(memoized) = get_module_instantiation_memo(ctx, key)
    {
        resolving.pop();
        return memoized;
    }
    let in_flight_reads_before = in_flight_degraded_read_epoch();

    // Derive the namespace prefix from the *original* declared name, not the
    // local binding (see the matching comment in `resolve_type_alias`). A member
    // rebound under a differently-spelled namespace alias — lucide's
    // `import * as react from "react"` registers React's
    // `ForwardRefExoticComponent` as `react.ForwardRefExoticComponent` — would
    // otherwise look its siblings up under the alias prefix, so
    // `extends NamedExoticComponent` missed and the interface silently lost the
    // inherited call signature that makes it a component type.
    let namespace_prefix = interface
        .declared_name
        .as_deref()
        .unwrap_or(&interface.name)
        .rsplit_once('.')
        .map(|(prefix, _)| prefix.to_string());
    let is_namespace_member = namespace_prefix.is_some();
    // See `resolve_type_alias`: a top-level declaration's body does not see the
    // members of the namespace whose member led here.
    let enclosing_namespaces = (!is_namespace_member).then(|| {
        (
            std::mem::take(&mut ctx.namespace_member_prefix_stack),
            std::mem::replace(&mut ctx.namespace_member_resolution_depth, 0),
        )
    });
    if let Some(prefix) = namespace_prefix {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    // An interface body is a structural crossing: a type-alias cycle re-entered
    // through this frame is legal recursion (see
    // `CheckerContext::structural_resolution_frames`).
    ctx.structural_resolution_frames.push(resolving.len() - 1);
    ctx.push_type_parameter_constraints_only(&interface.body.type_parameters);
    // Extended-tier interning requires a body resolution that touched NO cycle
    // machinery at all (self-cycles included): a recursive re-entry embeds a
    // nominal cycle reference whose ARGUMENTS carry the resolving context's
    // substitution, so a cached body would substitute the first caller's
    // rendering into every consumer (observed as zod message drift). The
    // physical default-lib tier keeps its historical behavior.
    let cycle_floor = resolving.len();
    let saved_lowest_cycle = ctx.lowest_cycle_target_index;
    ctx.lowest_cycle_target_index = usize::MAX;
    let diagnostics_before = ctx.diagnostics().len();
    let utility_keys_before = ctx.utility_diagnostic_keys.len();
    let degradation_before = crate::program::expansion_degradation_epoch();
    let expansion_reason = if physical_default_lib {
        crate::program::DtsExpansionReason::DefaultLibInterfaceInstantiation
    } else if interface.file_name.contains("node_modules") {
        crate::program::DtsExpansionReason::DependencyInterfaceInstantiation
    } else {
        crate::program::DtsExpansionReason::InterfaceResolution
    };
    let lazy_member_context = (lazy_interface_members_enabled()
        && ctx.is_library_scoped_file(&interface.file_name))
    .then(|| {
        (
            interface.name.as_ref(),
            interface.name_span.map_or(0, |span| span.start),
        )
    });
    let outer_class_heritage =
        std::mem::replace(&mut ctx.resolving_class_heritage, interface.is_class_instance);
    let self_reading = (!physical_default_lib
        && !is_generated_default_lib_file_name(&interface.file_name)
        && interface.reads_own_members())
    .then(|| SelfReading {
        interface_name: interface.name.as_ref(),
        declaration_start: interface.name_span.map_or(0, |span| span.start),
        own_name: interface.own_name(),
    });
    let resolved = crate::program::with_dts_expansion_reason(expansion_reason, || {
        with_type_declaration_scope(&declaration_effective_scope, ctx, |ctx| {
            with_file_name(ctx, &interface.file_name, |ctx| {
                resolve_interface_declaration(
                    &interface.body.extends,
                    &interface.body.members,
                    interface.body.string_index_type.as_ref(),
                    interface.body.number_index_type.as_ref(),
                    interface.body.string_index_readonly,
                    interface.body.number_index_readonly,
                    interface.body.keyed_indexes.as_deref(),
                    interface.body.call_signature.as_ref(),
                    &interface.body.call_signature_overloads,
                    &interface.body.construct_signatures,
                    ctx,
                    resolving,
                    &local_substitution,
                    stable_declaration.as_ref(),
                    declaration_template.as_deref(),
                    interface_key.as_ref(),
                    lazy_member_context,
                    interface.body.has_foreign_fragments().then(|| &*interface.body),
                    Some((fragment_declared_name(interface), &handle)),
                    self_reading,
                )
            })
        })
    });
    ctx.resolving_class_heritage = outer_class_heritage;
    ctx.pop_type_parameter_scope();
    ctx.structural_resolution_frames.pop();
    let subtree_lowest_cycle = ctx.lowest_cycle_target_index;
    ctx.lowest_cycle_target_index = saved_lowest_cycle.min(subtree_lowest_cycle);
    let cycle_free = subtree_lowest_cycle >= cycle_floor;
    // A member annotation that re-enters an in-progress frame embeds a cycle
    // reference carrying its own resolved arguments — a nominal handle any later
    // reader peels on demand, so the body is still a function of its key. What
    // a re-entry does poison is HERITAGE: a base that is mid-resolution on the
    // stack contributes no inherited members, and that incomplete surface
    // depends on which frames the caller had open (`QueryObserverOptions`
    // spread without `queryKey` when its `WithRequired<QueryOptions<…>>` base
    // was an outer frame). Only a heritage re-entry into an outer frame keeps
    // the body out of the module memo.
    let heritage_lowest_cycle = LAST_EXPANSION_HERITAGE_LOWEST_CYCLE.with(std::cell::Cell::get);
    let heritage_outer_cycle_free = heritage_lowest_cycle.saturating_add(1) >= cycle_floor;
    if is_namespace_member {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
    if let Some((stack, depth)) = enclosing_namespaces {
        ctx.namespace_member_prefix_stack = stack;
        ctx.namespace_member_resolution_depth = depth;
    }
    resolving.pop();

    let had_error = resolved.had_error || arguments_had_error;
    if arguments_had_error && !resolved.had_error && had_error_trace_enabled() {
        eprintln!(
            "[had-error] iface-args '{}' cp={}",
            interface.name,
            crate::program::in_check_phase()
        );
    }
    let emitted_diagnostics = ctx.diagnostics().len() != diagnostics_before
        || ctx.utility_diagnostic_keys.len() != utility_keys_before;
    let degraded_during_expansion =
        crate::program::expansion_degradation_epoch() != degradation_before;
    let clean = !had_error && !emitted_diagnostics && !degraded_during_expansion;
    let mut ty = attach_member_restrictions(resolved.ty, &interface);
    if cache_eligible
        && (physical_default_lib || cycle_free)
        && physical_interface_cache_enabled()
        && let Some(key) = interface_key.clone()
    {
        let value_rejection = clean
            .then(|| validate_physical_interface_cache_value(&ty).err())
            .flatten();
        if clean && value_rejection.is_none() {
            ty = (*intern_physical_interface_instantiation(ctx, key, ty)).clone();
        } else {
            crate::program::record_program_counter(|c| {
                c.physical_interface_cache_reject_had_error_count += u64::from(had_error);
                c.physical_interface_cache_reject_diagnostics_count +=
                    u64::from(emitted_diagnostics);
                c.physical_interface_cache_reject_degradation_count +=
                    u64::from(degraded_during_expansion);
                c.physical_interface_cache_reject_unknown_count += u64::from(matches!(
                    value_rejection,
                    Some(InterfaceCacheValueRejection::Unknown)
                ));
                c.physical_interface_cache_reject_context_count += u64::from(matches!(
                    value_rejection,
                    Some(InterfaceCacheValueRejection::ResolutionContext)
                ));
                c.physical_interface_cache_reject_traversal_count += u64::from(matches!(
                    value_rejection,
                    Some(InterfaceCacheValueRejection::TraversalLimit)
                ));
            });
        }
    }
    crate::program::record_interface_resolution_result(
        stable_declaration,
        interface_key.as_ref(),
        clean,
        false,
        interface.body.extends.len(),
        interface.body.members.len(),
        creation_before,
    );

    let resolved = ResolvedType { ty, had_error };
    // Store only an expansion that is a pure function of its own inputs:
    //   - `cycle_free` — no re-entry below this frame, so nothing on the
    //     caller's `resolving` stack shaped the result;
    //   - no diagnostics (a reused entry cannot re-emit them);
    //   - the expansion-degradation epoch is unchanged, so no nested lazy peel
    //     was cut short by *this consumer's* peel-stack depth (the transient
    //     `unknown` class that drifted zod diagnostics when it was interned);
    //   - no named-type memo entry was read while it was mid-resolution on
    //     another frame.
    // `had_error` on its own is allowed: a name that is absent from this
    // declaration's own resolution scope (an unmodelled `enum` member type, a
    // dotted namespace reference) misses identically at every site inside the
    // region this map covers, and the miss emits nothing.
    if let Some(key) = module_memo_key {
        let in_flight_clean = in_flight_degraded_read_epoch() == in_flight_reads_before;
        let base_is_open = LAST_EXPANSION_BASE_IS_OPEN.with(std::cell::Cell::get);
        if heritage_outer_cycle_free
            && !emitted_diagnostics
            && !degraded_during_expansion
            && in_flight_clean
        {
            store_module_instantiation_memo(ctx, key, &resolved, base_is_open);
        } else if super::cache::program_memo_dump_enabled() {
            eprintln!(
                "[program-memo-skip] {} heritage_outer_cycle={} diagnostics={} degraded={} in_flight={} had_error={} base_open={} check={}",
                interface.name,
                !heritage_outer_cycle_free,
                emitted_diagnostics,
                degraded_during_expansion,
                !in_flight_clean,
                resolved.had_error,
                base_is_open,
                crate::program::in_check_phase()
            );
        }
    }
    resolved
}

thread_local! {
    /// `base_is_open` of the most recently completed
    /// [`resolve_interface_declaration`]. Read by `resolve_interface` at its
    /// memo-store site, which runs immediately after the call returns, so the
    /// last write is always the outermost expansion's own.
    pub(crate) static LAST_EXPANSION_BASE_IS_OPEN: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    /// `ctx.lowest_cycle_target_index` as it stood once the most recently
    /// completed `resolve_interface_declaration` had resolved its heritage,
    /// before any member annotation. Read at the same memo-store site.
    pub(crate) static LAST_EXPANSION_HERITAGE_LOWEST_CYCLE: std::cell::Cell<usize> =
        const { std::cell::Cell::new(usize::MAX) };
    /// `ctx.lowest_unknown_cycle_target_index` over the heritage of the most
    /// recently completed `resolve_interface_declaration`, published at its end
    /// so that it is that frame's own: the outermost frame whose re-entry left
    /// a base, or a base's argument, `unknown`. Read by `resolve_named_type`.
    pub(crate) static LAST_EXPANSION_HERITAGE_UNKNOWN_CYCLE: std::cell::Cell<usize> =
        const { std::cell::Cell::new(usize::MAX) };
}

/// A member's own resolution context, installed while its annotation resolves.
struct InstalledMemberScope {
    saved_scope: Option<Arc<crate::symbols::TypeDeclarationScope>>,
    saved_file_name: String,
    crossed_file: bool,
}

fn install_member_scope(
    ctx: &mut CheckerContext,
    scope: &Arc<crate::symbols::TypeDeclarationScope>,
    file_name: &str,
) -> InstalledMemberScope {
    let saved_scope = ctx.type_declaration_scope.clone();
    let saved_file_name = ctx.file_name.clone();
    let crossed_file = saved_file_name != file_name;
    ctx.type_declaration_scope = Some(scope.clone());
    ctx.set_file_name(file_name.to_string());
    if crossed_file {
        ctx.cross_file_resolution_depth += 1;
    }
    InstalledMemberScope {
        saved_scope,
        saved_file_name,
        crossed_file,
    }
}

/// The scope a fragment of a merged declaration resolves under when it was
/// written where the declaration's own scope does not reach, as tsc resolves
/// every declaration where it is written: a `declare module` augmentation keeps
/// the scope it was collected with, and a `declare global` block in a module
/// takes that module's scope once the program has bound it.
fn foreign_fragment_scope(
    body: &crate::symbols::InterfaceBody,
    fragment: &crate::symbols::InterfaceDeclarationFragmentId,
    merged: Option<(&str, &TypeDeclarationHandle)>,
    ctx: &CheckerContext,
) -> Option<(Arc<crate::symbols::TypeDeclarationScope>, Arc<str>)> {
    if let Some((_, scope)) = body
        .fragment_scopes
        .iter()
        .find(|(candidate, _)| candidate == fragment)
    {
        // The augmentation's own declaration of the interface is the merged
        // symbol in Go (`mergeModuleAugmentation`), so its name reads the merged
        // declaration from inside the fragment, not the fragment alone.
        if let Some((name, handle)) = merged
            && scope.get_handle(name).is_some_and(|declared| !declared.ptr_eq(handle))
        {
            let mut own = crate::symbols::TypeDeclarationTable::new();
            own.insert_handle(name, handle);
            let mut layers = vec![Arc::new(own)];
            layers.extend(scope.layers().iter().cloned());
            return Some((
                Arc::new(crate::symbols::TypeDeclarationScope::new(layers)),
                fragment.file_name.clone(),
            ));
        }
        return Some((scope.clone(), fragment.file_name.clone()));
    }
    if *fragment.file_name == *ctx.file_name || !body.module_global_fragments.contains(fragment) {
        return None;
    }
    let scope = ctx
        .module_scope_by_file
        .get(&*fragment.file_name)
        .cloned()
        .or_else(|| crate::program::program_module_scope_for_file(&fragment.file_name))?;
    Some((scope, fragment.file_name.clone()))
}

fn restore_member_scope(ctx: &mut CheckerContext, saved: InstalledMemberScope) {
    if saved.crossed_file {
        ctx.cross_file_resolution_depth -= 1;
    }
    ctx.set_file_name(saved.saved_file_name);
    ctx.type_declaration_scope = saved.saved_scope;
}

pub(crate) fn resolve_interface_declaration(
    extends: &[ParsedNamedType],
    members: &[ParsedInterfaceMember],
    string_index_type: Option<&ParsedType>,
    number_index_type: Option<&ParsedType>,
    string_index_readonly: bool,
    number_index_readonly: bool,
    keyed_indexes: Option<&surge_ts_syntax::ParsedKeyedIndexes>,
    call_signature: Option<&ParsedFunctionType>,
    call_signature_overloads: &[ParsedFunctionType],
    construct_signatures: &[ParsedFunctionType],
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
    interface_declaration: Option<&crate::context::StableInterfaceDeclarationId>,
    declaration_template: Option<&crate::context::InterfaceDeclarationTemplate>,
    interface_key: Option<&crate::context::InterfaceInstantiationKey>,
    // `Some((interface name, declaration start))` opts this expansion into
    // member-level laziness (library-scoped declarations only). The local
    // validation pass passes `None`: it exists to EMIT member diagnostics,
    // and diagnostics produced inside a lazy force are dropped with the
    // recovered context.
    lazy_member_context: Option<(&str, usize)>,
    // The declaration body, when some of its fragments were written in another
    // file and must resolve under that file's scope rather than this
    // declaration's (`foreign_fragment_scope`).
    augmented_body: Option<&crate::symbols::InterfaceBody>,
    // The merged declaration being resolved, under the name its fragments
    // declare it by.
    merged_self: Option<(&str, &TypeDeclarationHandle)>,
    // Defers a property that reads members of the declaration being expanded.
    self_reading: Option<SelfReading<'_>>,
) -> ResolvedType {
    crate::program::record_interface_member_declaration_visits(members.len());
    crate::program::record_program_counter(|c| {
        c.interface_own_property_map_alloc_count += 1;
        c.interface_method_signature_group_alloc_count += members
            .iter()
            .filter(|member| matches!(member.ty, ParsedType::Function(_)))
            .count() as u64;
        c.interface_call_signature_array_alloc_count += u64::from(call_signature.is_some());
        c.interface_construct_signature_array_alloc_count += construct_signatures.len() as u64;
        c.interface_index_signature_alloc_count += u64::from(string_index_type.is_some());
    });
    let mut properties = PropertyMap::default();
    let mut had_error = false;
    let mut inherited_index_type: Option<Type> = None;
    let mut inherited_number_index_type: Option<Type> = None;
    let mut inherited_index_readonly = false;
    let mut inherited_number_index_readonly = false;
    let mut inherited_call_signature: Option<FunctionType> = None;
    let mut inherited_construct_signature: Option<FunctionType> = None;
    // A base that resolves to `any` (e.g. a mixin) leaves the derived member set
    // unknown; tsc keeps the type open. A base that fails to resolve is only
    // treated as open inside declaration files, where the real base is assumed to
    // be an unmodelled lib type (DOM/Node `Request`) that tsc resolves under
    // `skipLibCheck`; in user source an unresolved base is a genuine error and
    // tsc still flags missing-member access, so it must stay closed there.
    let in_declaration_file = is_declaration_file_name(&ctx.file_name);
    let mut base_is_open = false;
    let outer_unknown_cycle =
        std::mem::replace(&mut ctx.lowest_unknown_cycle_target_index, usize::MAX);
    for (base_index, base) in extends.iter().enumerate() {
        if ctx.resolving_class_heritage && crate::program::base_value_is_no_constructor(base, None, ctx) {
            continue;
        }
        let installed_base_scope = augmented_body
            .and_then(|body| Some((body, body.extends_fragments.get(base_index)?)))
            .and_then(|(body, fragment)| foreign_fragment_scope(body, fragment, merged_self, ctx))
            .map(|(scope, file_name)| install_member_scope(ctx, &scope, &file_name));
        // `resolveBaseTypesOfClass`: a class whose base is not a class derives
        // from a constructor function, and its type arguments select among the
        // construct signatures rather than instantiate a type reference, so a
        // count no reference would take is no error. surge does not select a
        // signature, so such a base stays open.
        let resolved_base = if ctx.resolving_class_heritage
            && class_base_arguments_select_constructors(base, ctx)
        {
            ResolvedType {
                ty: Type::Unknown,
                had_error: true,
            }
        } else {
            crate::program::with_dts_expansion_reason(
                crate::program::DtsExpansionReason::InterfaceHeritageResolution,
                || {
                    resolve_named_type(
                        std::sync::Arc::new(base.clone()),
                        ctx,
                        resolving,
                        substitution,
                    )
                },
            )
        };
        if resolved_base.had_error && had_error_trace_enabled() {
            eprintln!(
                "[had-error] base '{}' cp={} in file {}",
                base.name,
                crate::program::in_check_phase(),
                ctx.file_name
            );
        }
        if let Some(saved) = installed_base_scope {
            restore_member_scope(ctx, saved);
        }
        had_error |= resolved_base.had_error;
        // A base that resolved with errors may be missing members surge could
        // not model; the derived member set is incomplete, so keep it open
        // rather than flagging every inherited access.
        base_is_open |= resolved_base.had_error;

        // A generic base (`extends Dict<string>`) resolves to a nominal
        // `Type::Reference`; peel it so its inherited members and index signature
        // are merged structurally.
        match resolved_base.ty.peeled() {
            // `interface RequestHandler<…> extends core.RequestHandler<…> {}`
            // (express) extends an alias of a *function type*: the derived
            // interface is callable with exactly that signature and declares
            // nothing else, so treat the base like an object whose only member
            // is its call signature.
            Type::Function(function_type) => {
                if inherited_call_signature.is_none() {
                    inherited_call_signature = Some(function_type);
                }
            }
            Type::Object(object_type) => {
                for (name, property) in object_type.properties.iter() {
                    // Derived members shadow inherited ones; probe before
                    // cloning so an already-present name costs no key or
                    // property copy.
                    if properties.contains_key(name) {
                        continue;
                    }
                    let reason = if matches!(property.ty, Type::Function(_)) {
                        crate::program::DtsExpansionReason::InheritedMethodMerge
                    } else {
                        crate::program::DtsExpansionReason::InheritedPropertyMerge
                    };
                    crate::program::with_dts_expansion_reason(reason, || {
                        properties.insert(name.clone(), property.clone());
                    });
                }
                if let Some(declaration) = interface_declaration {
                    let inherited_methods = object_type
                        .properties
                        .values()
                        .filter(|property| matches!(property.ty, Type::Function(_)))
                        .count();
                    crate::program::record_inherited_member_merge(
                        declaration,
                        &base.name,
                        object_type.properties.len(),
                        inherited_methods,
                    );
                }
                // A base's checker-injected openness is not a declared index
                // signature: inheriting it as one made every undeclared member
                // of the derived type a false TS4111 instead of staying open.
                if object_type.synthetic_open_index {
                    base_is_open = true;
                } else if inherited_index_type.is_none()
                    && let Some(index_type) = &object_type.string_index_type
                {
                    inherited_index_type = Some(index_type.as_ref().clone());
                    inherited_index_readonly = object_type.string_index_readonly;
                }
                if inherited_number_index_type.is_none()
                    && let Some(index_type) = &object_type.number_index_type
                {
                    inherited_number_index_type = Some(index_type.as_ref().clone());
                    inherited_number_index_readonly = object_type.number_index_readonly;
                }
                // Call/construct signatures are inherited like members: React's
                // `ForwardRefExoticComponent extends ExoticComponent` carries its
                // callability entirely from the base, and dropping it here strips
                // the component's call signature (breaking `ComponentProps<typeof
                // forwardRefComponent>` and JSX prop checking).
                if inherited_call_signature.is_none() {
                    inherited_call_signature = object_type.call_signature().cloned();
                }
                if inherited_construct_signature.is_none() {
                    inherited_construct_signature = object_type.construct_signature().cloned();
                }
                // An empty-object base inside a declaration file is, in this
                // checker, an unmodelled lib/dependency stub (e.g. the generated
                // `interface Request {}` placeholder for the DOM type). tsc has the
                // real, populated base under `skipLibCheck`, so keep the derived
                // type open instead of flagging every inherited access.
                if in_declaration_file
                    && object_type.properties.is_empty()
                    && object_type.string_index_type.is_none()
                {
                    base_is_open = true;
                }
            }
            // `interface NodeArray<T> extends ReadonlyArray<T>` — the array
            // surface is a name lookup, not a property map, so materialize it
            // here or every inherited `forEach`/`length` is a false TS2339.
            Type::Array(element) => {
                if inherited_number_index_type.is_none() {
                    inherited_number_index_type = Some(element.as_ref().clone());
                    // `ReadonlyArray<T>` declares `readonly [n: number]: T`.
                    inherited_number_index_readonly = matches!(&resolved_base.ty,
                        Type::Reference(reference) if reference.is_readonly_array()
                            || reference.arguments.len() == 1
                                && reference.id.split('\u{0}').next_back() == Some("ReadonlyArray"));
                }
                for name in surge_ts_types::array_property_names() {
                    if properties.contains_key(*name) {
                        continue;
                    }
                    let Some(member_ty) = surge_ts_types::array_member_type(name, element.as_ref())
                    else {
                        continue;
                    };
                    // Every function-valued member of the lib's `Array<T>` is
                    // declared with method syntax, so it relates bivariantly.
                    let is_method = matches!(member_ty, Type::Function(_));
                    properties.insert(
                        (*name).into(),
                        surge_ts_types::ObjectProperty::required(member_ty).with_method(is_method),
                    );
                }
            }
            // `interface MyEmptyTuple extends EmptyTuple` — a tuple's members are
            // its element properties, its `length` and the `Array` members over
            // its element union, read off the same surface a tuple value has.
            base @ (Type::Tuple(_) | Type::OpenTuple(_)) => {
                let (positions, min_length, element) = match &base {
                    Type::Tuple(elements) => (
                        elements.len(),
                        surge_ts_types::tuple_min_length(elements),
                        surge_ts_types::tuple_element_union(elements),
                    ),
                    Type::OpenTuple(tuple) => (tuple.leading.len(), tuple.leading.len(), tuple.element_union()),
                    _ => unreachable!("matched a tuple"),
                };
                if inherited_number_index_type.is_none() {
                    inherited_number_index_type = Some(element);
                }
                let element_names = (0..positions).map(|position| position.to_string());
                let member_names = std::iter::once("length")
                    .chain(surge_ts_types::array_property_names().iter().copied())
                    .map(str::to_string);
                for (position, name) in element_names.chain(member_names).enumerate() {
                    if properties.contains_key(name.as_str()) {
                        continue;
                    }
                    let Some(member_ty) = base.get_property_access_type(&name) else {
                        continue;
                    };
                    let is_element = position < positions;
                    let is_method = !is_element && matches!(member_ty, Type::Function(_));
                    let property = if is_element && position >= min_length {
                        surge_ts_types::ObjectProperty::optional(member_ty)
                    } else {
                        surge_ts_types::ObjectProperty::required(member_ty)
                    };
                    properties.insert(name.as_str().into(), property.with_method(is_method));
                }
            }
            Type::Any => base_is_open = true,
            // A degraded (sentinel-`Unknown`) base is surge's own resolution
            // failure — e.g. a base inside a cyclic module cluster — not a user
            // error; tsc has the full base, so the derived type must stay open
            // in user source too or every inherited member access is a false
            // TS2339. Only the *genuine* `unknown` keyword keeps the derived
            // type closed outside declaration files.
            Type::Unknown | Type::TypeParameter(_) => base_is_open = true,
            Type::GenuineUnknown => base_is_open |= in_declaration_file,
            _ => {}
        }
    }
    LAST_EXPANSION_HERITAGE_LOWEST_CYCLE.with(|cell| cell.set(ctx.lowest_cycle_target_index));
    let heritage_unknown_cycle = ctx.lowest_unknown_cycle_target_index;
    ctx.lowest_unknown_cycle_target_index = outer_unknown_cycle.min(heritage_unknown_cycle);

    let mut own_method_group_contaminated =
        surge_ts_types::fx::FxHashMap::<String, bool>::default();
    let mut own_method_group_clean = surge_ts_types::fx::FxHashMap::<String, bool>::default();
    // Names this body has already contributed. Same-named members *within* one
    // body (or across declaration-merged bodies) are overloads and fold together;
    // an *inherited* same-named member is instead replaced, since a derived
    // class/interface member shadows the base's. Folding across the heritage
    // boundary blended `Bench.addEventListener<K, T = EventsMap[K]>` with the
    // `EventTarget` one it overrides and cost the callback its contextual type.
    let mut own_member_names =
        surge_ts_types::fx::FxHashSet::<&str>::with_hasher(Default::default());
    // Each folded method group's members in declaration order, attached to the
    // fold once the members are all in, so a call can pick its overload's
    // return the way a free function's group does (tsc's `chooseOverload`).
    let mut method_overload_members =
        surge_ts_types::fx::FxHashMap::<&str, Vec<FunctionType>>::with_hasher(Default::default());
    // Stage 2 gate: a method's components defer only when it has NO overloads
    // — `merge_overload_signatures` compares parameter slots by equality and
    // must not peel, so distinct lazy-ref ids in an overload group would widen
    // previously-equal slots to `any`.
    // One substitution per expansion, so its fingerprint is computed at most
    // once here rather than per deferred member.
    let mut lazy_substitution_fingerprint: Option<u64> = None;
    let lazy_method_group_counts = lazy_member_context.map(|_| {
        let mut counts =
            surge_ts_types::fx::FxHashMap::<&str, u32>::with_hasher(Default::default());
        for member in members {
            if matches!(member.ty, ParsedType::Function(_)) {
                *counts.entry(member.name.as_str()).or_insert(0) += 1;
            }
        }
        counts
    });
    for (member_index, member) in members.iter().enumerate() {
        let is_method = matches!(member.ty, ParsedType::Function(_));
        let reason = if is_method {
            crate::program::DtsExpansionReason::InterfaceMethodMapping
        } else {
            crate::program::DtsExpansionReason::InterfaceOwnPropertyMapping
        };
        let member_template = declaration_template.and_then(|template| {
            let member_template = template.members.get(member_index)?;
            (member_template.declaration.declared_name.as_ref() == member.name
                && member_template.declaration.declaration_kind
                    == if is_method {
                        crate::context::InterfaceMemberDeclarationKind::Method
                    } else {
                        crate::context::InterfaceMemberDeclarationKind::Property
                    })
            .then_some(member_template)
        });
        let method_key = is_method
            .then(|| {
                Some(interface_member_instantiation_key(
                    &member_template?.declaration,
                    interface_key?,
                ))
            })
            .flatten();
        let diagnostics_before = ctx.diagnostics().len();
        let utility_keys_before = ctx.utility_diagnostic_keys.len();
        let degradation_before = crate::program::expansion_degradation_epoch();
        let cached_method = method_key
            .as_ref()
            .and_then(|key| lookup_physical_interface_method(ctx, key));
        let method_cache_hit = cached_method.is_some();
        if is_method && method_key.is_some() {
            crate::program::record_program_counter(|c| {
                if method_cache_hit {
                    c.interface_method_cache_hit_count += 1;
                    c.interface_method_function_payload_avoided_count += 1;
                } else {
                    c.interface_method_cache_miss_count += 1;
                }
            });
        }
        let mut deferred_method_components = false;
        let installed_member_scope = augmented_body
            .and_then(|body| Some((body, body.member_fragments.get(member_index)?)))
            .and_then(|(body, fragment)| foreign_fragment_scope(body, fragment, merged_self, ctx))
            .map(|(scope, file_name)| install_member_scope(ctx, &scope, &file_name));
        let mut property_type = if let Some(function) = cached_method {
            ResolvedType {
                ty: Type::Function(function),
                had_error: false,
            }
        } else if let Some((interface_name, declaration_start)) = (!is_method)
            .then(|| {
                lazy_member_context
                    .filter(|_| defer_interface_member_annotation(&member.ty, member.optional))
                    .or_else(|| {
                        self_reading
                            .filter(|reading| {
                                crate::symbols::reads_members_of(
                                    &member.ty,
                                    reading.own_name,
                                    &member.name,
                                    &|name| {
                                        matches!(
                                            ctx.lookup_type_declaration(name),
                                            Some(crate::symbols::TypeDeclarationInfo::Alias(_))
                                        )
                                    },
                                )
                            })
                            .map(|reading| (reading.interface_name, reading.declaration_start))
                    })
            })
            .flatten()
        {
            ResolvedType {
                ty: super::cache::make_lazy_member_annotation_reference(
                    ctx,
                    super::cache::LazyMemberIdentity {
                        interface_name,
                        declaration_start,
                        member_index,
                        member_name: &member.name,
                        component: None,
                        substitution_fingerprint: *lazy_substitution_fingerprint
                            .get_or_insert_with(|| {
                                super::cache::member_substitution_fingerprint(substitution)
                            }),
                    },
                    &member.ty,
                    substitution,
                ),
                had_error: false,
            }
        } else if let Some((interface_name, declaration_start)) = lazy_member_context
            && is_method
            // Check phase only: during analysis, scopes still move, so a
            // forced component cannot intern — and signature assignability
            // then re-resolves the components per comparison pair, grinding
            // the analysis rounds the way the pre-pin degraded re-expansion
            // did.
            && crate::program::in_check_phase()
            && lazy_method_group_counts
                .as_ref()
                .is_some_and(|counts| counts.get(member.name.as_str()) == Some(&1))
            && let ParsedType::Function(function_type) = &member.ty
        {
            deferred_method_components = true;
            let function_type = function_type.clone();
            crate::program::with_dts_expansion_reason(reason, || {
                super::resolve::resolve_function_type_lazy_components(
                    function_type,
                    ctx,
                    resolving,
                    substitution,
                    interface_name,
                    declaration_start,
                    member_index,
                    &member.name,
                )
            })
        } else {
            crate::program::with_dts_expansion_reason(reason, || {
                let deferring = match (&member.ty, self_reading) {
                    (ParsedType::Object(object), Some(reading)) if !is_method => {
                        let fingerprint = *lazy_substitution_fingerprint
                            .get_or_insert_with(|| super::cache::member_substitution_fingerprint(substitution));
                        resolve_object_member_deferring_self_reads(
                            object,
                            &member.name,
                            reading,
                            member_index,
                            fingerprint,
                            ctx,
                            resolving,
                            substitution,
                        )
                    }
                    _ => None,
                };
                deferring.unwrap_or_else(|| resolve_parsed_type(member.ty.clone(), ctx, resolving, substitution))
            })
        };
        if let Some(saved) = installed_member_scope {
            restore_member_scope(ctx, saved);
        }
        let emitted_diagnostics = ctx.diagnostics().len() != diagnostics_before
            || ctx.utility_diagnostic_keys.len() != utility_keys_before;
        let degraded = crate::program::expansion_degradation_epoch() != degradation_before;
        let value_rejection =
            if is_method && !property_type.had_error && !emitted_diagnostics && !degraded {
                validate_physical_interface_cache_value(&property_type.ty).err()
            } else {
                None
            };
        // A deferred method's callables hide behind lazy component refs the
        // resolved-side classifier cannot see, so classify from the PARSED
        // annotation — semantically identical (the resolved-side check has no
        // Reference arm, so named callables already classified as false).
        let contextual_typing_dependency = is_method
            && if deferred_method_components {
                if let ParsedType::Function(function_type) = &member.ty {
                    super::cache::parsed_method_has_contextual_typing_dependency(function_type)
                } else {
                    false
                }
            } else {
                physical_interface_method_has_contextual_typing_dependency(&property_type.ty)
            };
        let method_clean = is_method
            && !property_type.had_error
            && !emitted_diagnostics
            && !degraded
            && !contextual_typing_dependency
            && value_rejection.is_none();
        if is_method {
            let degradation_reason = if method_clean {
                None
            } else if emitted_diagnostics {
                Some(crate::program::InterfaceDegradationReason::DiagnosticProduced)
            } else if matches!(value_rejection, Some(InterfaceCacheValueRejection::Unknown))
                || matches!(property_type.ty, Type::Unknown | Type::TypeParameter(_))
                || degraded
            {
                Some(crate::program::InterfaceDegradationReason::UnknownFallback)
            } else if matches!(
                value_rejection,
                Some(InterfaceCacheValueRejection::ResolutionContext)
            ) {
                Some(crate::program::InterfaceDegradationReason::ContextRetainingReference)
            } else if matches!(
                value_rejection,
                Some(InterfaceCacheValueRejection::TraversalLimit)
            ) {
                Some(crate::program::InterfaceDegradationReason::TraversalLimit)
            } else if contextual_typing_dependency {
                Some(crate::program::InterfaceDegradationReason::ContextualTypingDependency)
            } else if property_type.had_error {
                Some(crate::program::InterfaceDegradationReason::MethodSignatureFailure)
            } else {
                Some(crate::program::InterfaceDegradationReason::Other)
            };
            crate::program::record_interface_method_mapping(
                method_key.as_ref(),
                method_clean,
                degradation_reason,
            );
            if method_key.is_some() {
                crate::program::record_program_counter(|c| {
                    c.interface_method_cache_reject_had_error_count +=
                        u64::from(property_type.had_error);
                    c.interface_method_cache_reject_diagnostics_count +=
                        u64::from(emitted_diagnostics);
                    c.interface_method_cache_reject_degradation_count += u64::from(degraded);
                    c.interface_method_cache_reject_unknown_count += u64::from(matches!(
                        value_rejection,
                        Some(InterfaceCacheValueRejection::Unknown)
                    ));
                    c.interface_method_cache_reject_context_count += u64::from(matches!(
                        value_rejection,
                        Some(InterfaceCacheValueRejection::ResolutionContext)
                    ));
                    c.interface_method_cache_reject_contextual_typing_count +=
                        u64::from(contextual_typing_dependency);
                    c.interface_method_cache_reject_traversal_count += u64::from(matches!(
                        value_rejection,
                        Some(InterfaceCacheValueRejection::TraversalLimit)
                    ));
                });
            }
            if !method_cache_hit
                && method_clean
                && let (Some(key), Type::Function(function)) =
                    (method_key.clone(), &property_type.ty)
            {
                property_type.ty =
                    Type::Function(intern_physical_interface_method(ctx, key, function.clone()));
            }
        }
        had_error |= property_type.had_error;
        if property_type.had_error && had_error_trace_enabled() {
            eprintln!(
                "[had-error] member '{}' cp={} in file {}",
                member.name,
                crate::program::in_check_phase(),
                ctx.file_name
            );
        }

        if is_method {
            let inherited_function = !own_method_group_contaminated.contains_key(&member.name)
                && properties
                    .get(member.name.as_str())
                    .is_some_and(|property| matches!(property.ty, Type::Function(_)));
            own_method_group_contaminated
                .entry(member.name.clone())
                .or_insert(inherited_function);
            own_method_group_clean
                .entry(member.name.clone())
                .and_modify(|clean| *clean &= method_clean)
                .or_insert(method_clean);
        }

        // Same-named function members are overloads (within one interface, or
        // merged across declaration-merged interfaces such as `ArrayConstructor`
        // gaining `from` overloads in lib.es2015.core). Collapse them into one
        // permissive signature so a call matching any overload's arity is
        // accepted, rather than last-wins dropping every overload but one.
        if own_member_names.contains(member.name.as_str())
            && let (Some(existing), Type::Function(incoming)) =
                (properties.get(member.name.as_str()), &property_type.ty)
            && let Type::Function(existing_fn) = &existing.ty
        {
            let overload_key = member_template
                .filter(|member| member.overload_position != 0)
                .and_then(|member| {
                    let clean = own_method_group_clean
                        .get(member.declaration.declared_name.as_ref())
                        .copied()
                        .unwrap_or(false);
                    let contaminated = own_method_group_contaminated
                        .get(member.declaration.declared_name.as_ref())
                        .copied()
                        .unwrap_or(true);
                    let group = declaration_template?
                        .method_groups
                        .get(member.overload_group? as usize)?;
                    if !clean || contaminated {
                        return None;
                    }
                    Some(interface_overload_instantiation_key(
                        interface_declaration?,
                        group,
                        member.overload_position + 1,
                        interface_key?,
                    ))
                });
            method_overload_members
                .entry(member.name.as_str())
                .or_insert_with(|| vec![existing_fn.clone()])
                .push(incoming.clone());
            let cached_overload = overload_key
                .as_ref()
                .and_then(|key| lookup_physical_interface_overload(ctx, key));
            let overload_cache_hit = cached_overload.is_some();
            if overload_key.is_some() {
                crate::program::record_program_counter(|c| {
                    if overload_cache_hit {
                        c.interface_overload_cache_hit_count += 1;
                        c.interface_overload_function_payload_avoided_count += 1;
                    } else {
                        c.interface_overload_cache_miss_count += 1;
                    }
                });
            }
            crate::program::record_interface_overload_construction(
                overload_key.as_ref(),
                overload_cache_hit,
            );
            let merged = if let Some(cached) = cached_overload {
                cached
            } else {
                let merged = crate::program::with_dts_expansion_reason(
                    crate::program::DtsExpansionReason::OverloadArrayMerge,
                    || merge_overload_signatures(existing_fn, incoming),
                );
                match overload_key {
                    Some(key)
                        if validate_physical_interface_cache_value(&Type::Function(
                            merged.clone(),
                        ))
                        .is_ok() =>
                    {
                        intern_physical_interface_overload(ctx, key, merged)
                    }
                    Some(_) => {
                        crate::program::record_program_counter(|c| {
                            c.interface_overload_cache_reject_count += 1
                        });
                        merged
                    }
                    None => merged,
                }
            };
            let optional = existing.optional && member.optional;
            properties.insert(
                member.name.as_str().into(),
                if optional {
                    ObjectProperty::optional(Type::Function(merged))
                } else {
                    ObjectProperty::required(Type::Function(merged))
                }
                .with_method(existing.method || member.is_method),
            );
            own_member_names.insert(member.name.as_str());
            continue;
        }

        let object_property = if member.optional {
            ObjectProperty::optional(property_type.ty)
        } else {
            ObjectProperty::required(property_type.ty)
        }
        .with_method(member.is_method)
        .with_readonly(member.readonly);

        properties.insert(member.name.as_str().into(), object_property);
        own_member_names.insert(member.name.as_str());
    }

    // The fold answers every consumer that wants one signature; the members
    // ride along, as they do for the interface's own call signatures, so a call
    // resolves against the candidate whose arity fits and reports TS2769 when
    // several fit and none accepts it. Attached once per group here: copying
    // the growing list at every fold step was quadratic in the overload count.
    for (name, overloads) in method_overload_members {
        if let Some(property) = properties.get_mut(name)
            && let Type::Function(merged) = &property.ty
        {
            property.ty = Type::Function(merged.clone().with_overloads(overloads));
        }
    }

    // An own index signature takes precedence; otherwise inherit one from a
    // base interface (e.g. `interface ProcessEnv extends Dict<string>`).
    let openness_is_synthetic =
        string_index_type.is_none() && inherited_index_type.is_none() && base_is_open;
    // Publish the raw flag for the program-lifetime memo. `openness_is_synthetic`
    // is not a usable proxy: a declared or inherited index signature suppresses
    // it while `base_is_open` is still true, and it is exactly those expansions
    // whose shape depended on a base the resolver could not pin down.
    LAST_EXPANSION_BASE_IS_OPEN.with(|flag| flag.set(base_is_open));
    // Published last, like the flag above: the members resolved since the
    // heritage loop run nested expansions that publish their own.
    LAST_EXPANSION_HERITAGE_UNKNOWN_CYCLE.with(|cell| cell.set(heritage_unknown_cycle));
    let resolved_index_type = match string_index_type {
        Some(parsed) => {
            let resolved = crate::program::with_dts_expansion_reason(
                crate::program::DtsExpansionReason::InterfaceIndexSignatureMapping,
                || resolve_parsed_type(parsed.clone(), ctx, resolving, substitution),
            );
            had_error |= resolved.had_error;
            Some(resolved.ty)
        }
        None => inherited_index_type.or(if base_is_open { Some(Type::Any) } else { None }),
    };

    // A numeric index signature is resolved alongside the string one: a numeric
    // key prefers it, and its presence alone is what makes a *string* key an
    // implicit `any` rather than a resolved member.
    let resolved_number_index_type = number_index_type
        .map(|parsed| {
            let resolved = crate::program::with_dts_expansion_reason(
                crate::program::DtsExpansionReason::InterfaceIndexSignatureMapping,
                || resolve_parsed_type(parsed.clone(), ctx, resolving, substitution),
            );
            had_error |= resolved.had_error;
            resolved.ty
        })
        .or(inherited_number_index_type);

    let resolved_index_readonly = if string_index_type.is_some() {
        string_index_readonly
    } else {
        inherited_index_readonly
    };
    let resolved_number_index_readonly = if number_index_type.is_some() {
        number_index_readonly
    } else {
        inherited_number_index_readonly
    };
    had_error |= super::late_bind_member_names(&mut properties, ctx);
    // A class's own index signature of a key kind wins over one its members
    // imply, which wins over an inherited one.
    let implied = if ctx.resolving_class_heritage {
        super::late_bind_index_signatures(&mut properties, ctx)
    } else {
        super::ImpliedIndexes::default()
    };
    let (resolved_index_type, resolved_index_readonly) = match (string_index_type, implied.string) {
        (None, Some((value, readonly))) => (Some(value), readonly),
        _ => (resolved_index_type, resolved_index_readonly),
    };
    let (resolved_number_index_type, resolved_number_index_readonly) = match (number_index_type, implied.number) {
        (None, Some((value, readonly))) => (Some(value), readonly),
        _ => (resolved_number_index_type, resolved_number_index_readonly),
    };
    let keyed_indexes = super::resolve::resolve_keyed_indexes(
        keyed_indexes,
        string_index_type.and(resolved_index_type.as_ref()),
        ctx,
        resolving,
        substitution,
    );
    let mut object_type = alloc_object_type(properties, resolved_index_type)
        .with_number_index_type(resolved_number_index_type)
        .with_readonly_indexes(resolved_index_readonly, resolved_number_index_readonly)
        .with_nominal_declaration_marker();
    object_type.keyed_indexes = keyed_indexes;
    if openness_is_synthetic {
        object_type = object_type.with_open_index_marker();
    }
    if let Some(call_signature) = call_signature {
        let resolved = crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::InterfaceCallSignatureMapping,
            || {
                resolve_parsed_type(
                    ParsedType::Function(std::sync::Arc::new(call_signature.clone())),
                    ctx,
                    resolving,
                    substitution,
                )
            },
        );
        had_error |= resolved.had_error;
        if let Type::Function(function_type) = resolved.ty {
            // The fold answers every consumer that wants one signature; the
            // overloads ride along so a call resolves against the candidate
            // whose arity and arguments fit, as tsc's `chooseOverload` does.
            let mut resolved_overloads: Vec<FunctionType> = Vec::new();
            for overload in call_signature_overloads {
                let resolved = crate::program::with_dts_expansion_reason(
                    crate::program::DtsExpansionReason::InterfaceCallSignatureMapping,
                    || {
                        resolve_parsed_type(
                            ParsedType::Function(std::sync::Arc::new(overload.clone())),
                            ctx,
                            resolving,
                            substitution,
                        )
                    },
                );
                had_error |= resolved.had_error;
                if let Type::Function(overload) = resolved.ty {
                    resolved_overloads.push(overload);
                }
            }
            let function_type = if resolved_overloads.len() > 1 {
                function_type.with_overloads(resolved_overloads)
            } else {
                function_type
            };
            object_type = object_type.with_call_signature(function_type);
        }
    } else if let Some(inherited) = inherited_call_signature {
        object_type = object_type.with_call_signature(inherited);
    }

    // Resolve every construct-signature overload and fold them into one permissive
    // signature (matching how method overloads are merged), so a call matching any
    // overload's arity/arguments is accepted (`new Uint8Array(8)` and
    // `new Uint8Array([1,2,3])` both work).
    let mut merged_construct: Option<FunctionType> = None;
    let mut construct_members: Vec<FunctionType> = Vec::new();
    for construct_signature in construct_signatures {
        let resolved = crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::InterfaceConstructSignatureMapping,
            || {
                resolve_parsed_type(
                    ParsedType::Function(std::sync::Arc::new(construct_signature.clone())),
                    ctx,
                    resolving,
                    substitution,
                )
            },
        );
        had_error |= resolved.had_error;
        if let Type::Function(function_type) = resolved.ty {
            construct_members.push(function_type.clone());
            merged_construct = Some(match merged_construct {
                Some(existing) => crate::program::with_dts_expansion_reason(
                    crate::program::DtsExpansionReason::OverloadArrayMerge,
                    || merge_overload_signatures(&existing, &function_type),
                ),
                None => function_type,
            });
        }
    }
    if let Some(construct_signature) = merged_construct {
        // The group is kept beside the fold, as a method overload group is: what
        // `infer` reads off a constructor is its *last* signature.
        let construct_signature = construct_signature.with_overloads(construct_members);
        object_type = object_type.with_construct_signature(construct_signature);
    } else if let Some(inherited) = inherited_construct_signature {
        object_type = object_type.with_construct_signature(inherited);
    }

    ResolvedType {
        ty: Type::Object(object_type),
        had_error,
    }
}

/// Whether exactly one side of an overload parameter slot is a function type and
/// the other declares no call signature of its own. Deliberately does NOT peel:
/// this runs for every merged slot, and forcing a nominal reference's expansion
/// here is the hazard the interface-resolution counters pin.
fn exactly_one_callback_slot(left: &Type, right: &Type) -> bool {
    let left_is_function = matches!(left, Type::Function(_));
    let right_is_function = matches!(right, Type::Function(_));
    if left_is_function == right_is_function {
        return false;
    }
    let other = if left_is_function { right } else { left };
    !matches!(other, Type::Object(object) if object.call_signature.is_some())
}

/// Collapse two function overloads into a single permissive signature: the
/// required-parameter count is the smaller of the two (a call matching the
/// shorter overload's arity is accepted), the parameter list is the longer of
/// the two with positions widened to the degradation sentinel where the
/// overloads disagree (so the
/// merge never rejects an argument valid under either overload), and the result
/// is variadic if either overload is. The shorter overload's return type is kept
/// as the representative, matching the most basic form (e.g. `Array.from`'s
/// `T[]`).
/// surge models a resolved `Promise<T>` as its awaited `T` (the implicit-await
/// modelling in `resolve_interface`), so one overload whose return reached that
/// collapse and one that still holds the `Promise<T>` reference denote the same
/// type — `tsc` keeps a signature per declaration and never sees a difference.
/// Degrading the merged return over that silenced every check downstream of a
/// global declared in two files: `fetch`, declared by both `lib.dom` and
/// `@types/node`, answered the sentinel for its own result. Returns the wrapped
/// form, so the merged signature still displays the way `tsc` prints it.
fn promise_collapsed_pair_return(left: &Type, right: &Type) -> Option<Type> {
    let left_awaited = crate::checks::call::promise_like_awaited_type(left);
    let right_awaited = crate::checks::call::promise_like_awaited_type(right);
    if left_awaited != right_awaited {
        return None;
    }
    if &left_awaited != left {
        return Some(left.clone());
    }
    if &right_awaited != right {
        return Some(right.clone());
    }
    None
}

pub(crate) fn merge_overload_signatures(a: &FunctionType, b: &FunctionType) -> FunctionType {
    let canonical_merge = a
        .id()
        .zip(b.id())
        .zip(current_program_type_store())
        .map(|((left, right), store)| (left, right, store));
    if let Some((left, right, store)) = canonical_merge.as_ref()
        && let Some(merged) = store.lookup_overload_merge(*left, *right)
    {
        return merged;
    }
    let (longer, shorter) = if a.parameters().len() >= b.parameters().len() {
        (a, b)
    } else {
        (b, a)
    };

    let parameters = longer
        .parameters()
        .iter()
        .enumerate()
        .map(|(index, ty)| match shorter.parameters().get(index) {
            Some(other) if other == ty => ty.clone(),
            // Two callback parameters that differ only in their *return* fold
            // the same way the overloads themselves do. Collapsing them to `any`
            // instead threw away the callback's own parameter types, so an arrow
            // written at the call site (`schema.refine((d) => …)`, whose
            // overloads are `(arg: Output) => arg is R` and `(arg: Output) =>
            // unknown`) lost its contextual type and every parameter became an
            // implicit any. Differing *parameter* lists still collapse: merging
            // those changes which arguments the slot accepts, which measured as
            // a new false TS2741 on ofetch.
            Some(Type::Function(other))
                if matches!(ty, Type::Function(current)
                    if current.parameters() == other.parameters()
                        && current.is_variadic() == other.is_variadic()
                        && current.required_parameter_count()
                            == other.required_parameter_count()) =>
            {
                let Type::Function(current) = ty else {
                    unreachable!("guarded above")
                };
                Type::Function(merge_overload_signatures(current, other))
            }
            // Exactly one overload declares a callback in this slot: widening to
            // `any` throws the callback's parameter types away, so every arrow
            // written at the call site becomes an implicit any (JSON.stringify's
            // replacer, addEventListener's listener). A union keeps both
            // overloads' arguments assignable AND leaves a signature for
            // contextual typing to find.
            Some(other) if exactly_one_callback_slot(ty, other) => {
                surge_ts_types::union_type(vec![ty.clone(), other.clone()])
            }
            // The degradation sentinel rather than `any`: the overloads do
            // constrain this slot, surge just cannot say how, and `any` would
            // claim the source omitted a contextual type — turning a callback
            // nested in the argument into a false implicit-any report.
            Some(_) => Type::Unknown,
            None => ty.clone(),
        })
        .collect::<Vec<_>>();

    let is_variadic = a.is_variadic() || b.is_variadic();
    let required_parameter_count = a
        .required_parameter_count()
        .min(b.required_parameter_count());
    // Which overload's return applies depends on the arguments, which a single
    // merged signature cannot express, so *any* disagreement degrades the merged
    // return rather than committing to one overload's answer: picking the
    // shorter one reported `err.format(mapper)` (whose overload returns
    // `$ZodFormattedError<T, U>`) against the no-argument overload's
    // `$ZodFormattedError<T>`. Extending this from the one-unresolved-return case
    // to every disagreement cost no false negatives on any corpus.
    let return_type = if a.return_type() == b.return_type() {
        shorter.return_type().clone()
    } else if let Some(wrapped) = promise_collapsed_pair_return(a.return_type(), b.return_type()) {
        wrapped
    } else {
        Type::Unknown
    };
    let merged = [a, b]
        .into_iter()
        .find(|candidate| {
            candidate.parameters() == parameters.as_slice()
                && candidate.return_type() == &return_type
                && candidate.is_variadic() == is_variadic
                && candidate.required_parameter_count() == required_parameter_count
        })
        .cloned()
        .unwrap_or_else(|| {
            crate::program::record_program_counter(|c| c.overload_array_alloc_count += 1);
            alloc_function_type(
                parameters,
                return_type,
                is_variadic,
                required_parameter_count,
            )
        });
    match canonical_merge {
        Some((left, right, store)) => store.record_overload_merge(left, right, merged),
        None => merged,
    }
}

fn record_interface_cache_skip(reason: InterfaceCacheSkipReason) {
    crate::program::record_program_counter(|c| match reason {
        InterfaceCacheSkipReason::Disabled => c.physical_interface_cache_skip_disabled_count += 1,
        InterfaceCacheSkipReason::UnstableDeclaration => {
            c.physical_interface_cache_skip_unstable_declaration_count += 1
        }
        InterfaceCacheSkipReason::UnresolvedTypeArgument => {
            c.physical_interface_cache_skip_unresolved_argument_count += 1
        }
        InterfaceCacheSkipReason::UnsupportedTypeArgument => {
            c.physical_interface_cache_skip_unsupported_argument_count += 1
        }
    });
}

fn is_declaration_file_name(file_name: &str) -> bool {
    surge_ts_syntax::is_declaration_file_name(file_name)
}

pub(crate) fn generated_default_lib_map_instance_type() -> Type {
    let mut properties = PropertyMap::default();
    properties.insert(
        "get".into(),
        ObjectProperty::required(Type::Function(alloc_function_type(
            vec![Type::Any],
            Type::Any,
            false,
            1,
        ))),
    );
    properties.insert(
        "set".into(),
        ObjectProperty::required(Type::Function(alloc_function_type(
            vec![Type::Any, Type::Any],
            Type::Any,
            false,
            2,
        ))),
    );
    properties.insert(
        "has".into(),
        ObjectProperty::required(Type::Function(alloc_function_type(
            vec![Type::Any],
            Type::Boolean,
            false,
            1,
        ))),
    );
    properties.insert(
        "delete".into(),
        ObjectProperty::required(Type::Function(alloc_function_type(
            vec![Type::Any],
            Type::Boolean,
            false,
            1,
        ))),
    );
    properties.insert(
        "clear".into(),
        ObjectProperty::required(Type::Function(alloc_function_type(
            vec![],
            Type::Void,
            false,
            0,
        ))),
    );
    properties.insert("size".into(), ObjectProperty::required(Type::Number));

    Type::Object(alloc_object_type(properties, None))
}

/// Marks the instance members a class declares `private` or `protected` with
/// the class they belong to, for the relation's modifier rules. Inherited
/// members already carry their own declaring class's mark. A member whose
/// accessor pair splits its modifiers is left unmarked.
fn attach_member_restrictions(ty: Type, interface: &InterfaceInfo) -> Type {
    if interface.body.restricted_members.is_empty() {
        return ty;
    }
    let Some(name_span) = interface.name_span else {
        return ty;
    };
    let Type::Object(mut object) = ty else {
        return ty;
    };
    let owner: std::sync::Arc<str> = format!("{}\0{}", interface.file_name, name_span.start).into();
    let mut properties = None;
    for restricted in &interface.body.restricted_members {
        if restricted.is_static || restricted.accessor_side.is_some() {
            continue;
        }
        let Some(property) = object.properties.get(restricted.name.as_str()) else {
            continue;
        };
        let restriction = surge_ts_types::MemberRestriction {
            private: restricted.accessibility == surge_ts_syntax::ParsedMemberAccessibility::Private,
            owner: owner.clone(),
        };
        if property.restriction.as_ref() == Some(&restriction) {
            continue;
        }
        let mut property = property.clone();
        property.restriction = Some(restriction);
        properties
            .get_or_insert_with(|| (*object.properties).clone())
            .insert(restricted.name.as_str().into(), property);
    }
    if let Some(properties) = properties {
        object.properties = std::sync::Arc::new(properties);
        object.property_map_id = None;
    }
    Type::Object(object)
}
