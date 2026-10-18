use std::sync::Arc;

use surge_ts_types::{ResolveReference, Type, TypeReference};

use crate::context::{
    CheckerContext, DeclarationEnvironmentHandle,
    DeclarationResolutionKey,
};
use crate::symbols::TypeDeclarationInfo;
use crate::infer::types::*;
use super::{in_flight_degraded_read_epoch, intern_instantiation, lookup_instantiation_probe};

/// Resolver for a lazy [`Type::Reference`] that resolves to an already-computed,
/// program-wide-shared structural expansion. The expansion is computed once per
/// unique instantiation by [`intern_instantiation`] and shared via `Arc`, so
/// resolving the reference never re-expands the declaration body.
#[derive(Debug)]
pub(super) struct InternedInstantiation {
    pub(super) resolved: Arc<Type>,
}

impl ResolveReference for InternedInstantiation {
    fn resolve(&self) -> Type {
        (*self.resolved).clone()
    }

    fn resolve_arc(&self) -> Arc<Type> {
        self.resolved.clone()
    }

    fn peek_resolved(&self) -> Option<Arc<Type>> {
        Some(self.resolved.clone())
    }
}

/// Maximum nesting of in-flight lazy peels before a deeper one degrades to
/// `unknown`. Real reference chains a consumer forces (an event-handler param, an
/// inheritance chain) stay well under this; the bound only trips on a runaway
/// library `extends` cluster.
pub(super) const MAX_LAZY_PEEL_DEPTH: usize = 24;

/// How many times one generic declaration may appear in the in-flight peel stack
/// before a deeper re-entry degrades to `unknown`. Bounds the mutually-recursive
/// library clusters (`A<X>` → `A<f(X)>` → …) while still allowing modest, genuine
/// self-nesting.
pub(super) const MAX_SAME_DECLARATION_PEELS: usize = 3;

thread_local! {
    /// Instantiations whose lazy body is currently being expanded on this thread.
    /// A mutually-recursive library cluster (`HTMLElement` → `Element` → … , the
    /// iterator/typed-array clusters) can peel back into an instantiation while
    /// expanding it; this stack breaks that re-entry with `unknown` instead of
    /// recursing forever. Keyed by declaration + resolved arguments.
    static LAZY_PEEL_STACK: std::cell::RefCell<Vec<(DeclarationResolutionKey, Vec<Type>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Resolver for a deferred library [`Type::Reference`]. Unlike
/// [`InternedInstantiation`] (which holds an already-expanded structural type),
/// this expands the declaration body *on first peel* — one level deep, since the
/// nested named types it references resolve to their own deferred references. This
/// is what keeps resolving a type argument such as `HTMLElement` from eagerly
/// pulling the whole DOM/iterator graph: the bulk shape is materialised only for
/// the instantiations a consumer actually inspects.
pub(super) struct LazyInstantiation {
    pub(super) environment: DeclarationEnvironmentHandle,
    /// The type-declaration scope installed where this reference was created.
    /// The compact environment may be captured before the declaring module
    /// scope is installed, so a peel through it alone cannot see the
    /// declaring module's siblings (a namespace member registered without its
    /// own `resolution_scope`, like the bare `JSX.IntrinsicElements` dual key,
    /// would resolve every member reference to `unknown`). Re-installing the
    /// creation-time scope restores the lexical environment the reference was
    /// formed in; a declaration carrying its own scope still overrides it.
    pub(super) creation_scope: Option<Arc<crate::symbols::TypeDeclarationScope>>,
    pub(super) decl: crate::symbols::TypeDeclarationHandle,
    pub(super) decl_key: DeclarationResolutionKey,
    pub(super) type_arguments: Vec<surge_ts_syntax::ParsedType>,
    pub(super) resolved_arguments: Vec<Type>,
    pub(super) substitution: TypeParameterSubstitution,
    pub(super) display: Arc<str>,
    /// The reference's nominal id and variances, which a signature alias's
    /// expansion carries for a relation to compare its arguments by.
    pub(super) reference_id: Arc<str>,
    pub(super) declared_variances: u32,
    /// Weak so a back-edge reference embedded in an interned expansion cannot
    /// keep its own container alive: memoizing the containing `Arc<Type>`
    /// strongly forms a reference↔expansion cycle that survives even after the
    /// program caches are cleared. The strong ref lives in
    /// `program_instantiations`; if it is gone the resolve falls through to a
    /// fresh peel.
    pub(super) memo: std::sync::OnceLock<std::sync::Weak<Type>>,
    /// A degraded expansion is never interned (see the comment at the degraded
    /// return in `resolve_arc`), so without this every peel of a
    /// still-degraded instantiation re-expands the full member map — and its
    /// equally degraded heritage bases — per consumer read, which is the
    /// re-expansion multiplier behind the namespace-merge blowup. Pin the
    /// first degraded answer per reference instance, but only once the check
    /// phase has begun: before it, module scopes still move between binding
    /// rounds, so an early degraded shape must stay transient for a later
    /// clean peel to supersede. A pinned return still bumps the degradation
    /// epoch and counters, so enclosing expansions stay uncacheable exactly as
    /// before — only the recomputation is skipped. Mirrors
    /// [`LazyDeclarationAnnotation::degraded_memo`].
    pub(super) degraded_memo: std::sync::OnceLock<Arc<Type>>,
}

impl LazyInstantiation {
    /// Whether an expansion is this very instantiation again: a generic alias
    /// whose body reduces to its own back-edge (a cycle reference handed back
    /// for the same arguments) would make `peeled` chase the reference forever.
    fn is_self_reference(&self, ty: &Type) -> bool {
        let Type::Reference(reference) = ty else {
            return false;
        };
        reference.arguments.as_ref() == self.resolved_arguments.as_slice()
            && reference.id.len() == self.decl_key.file_name.len() + 1 + self.decl_key.name.len()
            && reference.id.starts_with(self.decl_key.file_name.as_ref())
            && reference.id.ends_with(self.decl_key.name.as_ref())
    }

    /// An instantiation of a program source's declaration whose arguments name
    /// no type parameter: it reads nothing of the site that wrote it, so a
    /// reference outliving that site's environment (a namespace member bound
    /// before the check phase) still expands under the program's.
    fn concrete_program_instantiation(&self) -> bool {
        let file_name: &str = match self.decl.get() {
            TypeDeclarationInfo::Interface(info) => &info.file_name,
            TypeDeclarationInfo::Alias(info) => &info.file_name,
        };
        let mut budget = 96usize;
        !crate::modules::is_declaration_file_name(file_name)
            && !crate::infer::types::is_dependency_declaration_path(file_name)
            && self.resolved_arguments.len() == self.type_arguments.len()
            && self
                .resolved_arguments
                .iter()
                .all(|argument| crate::infer::types::signature_cache_safe_argument(argument, 0, &mut budget))
    }

    /// A default-lib or dependency declaration no other file augments: a
    /// fragment written elsewhere (`@types/node` reopening `ReadableStream`)
    /// resolves under its own file's scope, which a recovered program context
    /// does not carry. A dependency's reference read after the environment it
    /// was written in is gone (a module value table published before the check
    /// phase) otherwise peels to `unknown`.
    fn declares_in_library(&self) -> bool {
        let library = |file_name: &str| {
            crate::default_lib::is_physical_default_lib_file_name(file_name)
                || crate::default_lib::is_generated_default_lib_file_name(file_name)
                || crate::infer::types::is_dependency_declaration_path(file_name)
        };
        match self.decl.get() {
            TypeDeclarationInfo::Alias(alias) => library(&alias.file_name),
            TypeDeclarationInfo::Interface(interface) => {
                !interface.body.has_foreign_fragments()
                    && library(&interface.file_name)
                    && interface
                        .body
                        .declaration_fragments
                        .iter()
                        .all(|fragment| library(&fragment.file_name))
            }
        }
    }

    /// Whether the expansion is independent of the site that wrote the
    /// reference: a library declaration's body resolves in its own lexical
    /// scope, where the referencing site's type parameters are not visible, and
    /// arguments that name none of them carry nothing of that site either.
    fn resolves_outside_creating_scopes(&self, ctx: &CheckerContext) -> bool {
        let mut budget = 96usize;
        crate::infer::types::declaration_file_is_library_scoped(self.decl.get(), ctx)
            && self.resolved_arguments.iter().all(|argument| {
                crate::infer::types::signature_cache_safe_argument(argument, 0, &mut budget)
            })
    }
}

impl ResolveReference for LazyInstantiation {
    fn resolve(&self) -> Type {
        (*self.resolve_arc()).clone()
    }

    fn reinstantiate(&self, mapping: &[(surge_ts_types::TypeParameterType, Type)]) -> Option<Type> {
        let declaring_file = self.decl_key.file_name.clone();
        let mut ctx = crate::infer::types::origin::durable_context(&declaring_file)
            .or_else(|| self.environment.checker_context())?;
        if self.creation_scope.is_some() {
            ctx.type_declaration_scope = self.creation_scope.clone();
        }
        let arguments = self
            .resolved_arguments
            .iter()
            .map(|argument| crate::checks::function::generic_return::instantiate_type(argument, mapping).ok())
            .collect::<Option<Vec<_>>>()?;
        let substitution = crate::infer::types::origin::instantiated_substitution(&self.substitution, mapping)?;
        let display = crate::checks::function::generic_return::instantiated_display(&self.display, &arguments);
        Some(make_lazy_type_reference(
            &mut ctx,
            &self.reference_id,
            &display,
            self.decl.clone(),
            self.decl_key.clone(),
            self.type_arguments.clone(),
            arguments,
            substitution,
        ))
    }

    fn captured_census(&self) -> surge_ts_types::ResolverCaptureCensus {
        let own_bytes = std::mem::size_of::<Self>() as u64
            + self
                .type_arguments
                .iter()
                .map(surge_ts_syntax::ParsedType::estimated_heap_bytes)
                .sum::<u64>()
            + (self.resolved_arguments.len() * std::mem::size_of::<Type>()) as u64
            + self.decl_key.name.len() as u64;
        surge_ts_types::ResolverCaptureCensus {
            own_bytes,
            shared_captures: self.substitution.census_shared_captures(),
        }
    }

    fn peek_resolved(&self) -> Option<Arc<Type>> {
        self.memo.get().and_then(std::sync::Weak::upgrade)
    }

    fn never_object_keyword(&self) -> bool {
        matches!(self.decl.get(), TypeDeclarationInfo::Interface(_))
    }

    fn resolve_arc(&self) -> Arc<Type> {
        crate::program::record_lazy_reference_peel_start(&self.decl_key);
        if let Some(memoized) = self.memo.get().and_then(std::sync::Weak::upgrade) {
            crate::program::record_program_counter(|c| c.lazy_reference_memo_hit_count += 1);
            if self.is_self_reference(&memoized) {
                crate::program::note_expansion_degradation();
                return Arc::new(Type::Unknown);
            }
            return memoized;
        }
        let Some(ctx) = self.environment.checker_context().or_else(|| {
            crate::context::program_checker_context().filter(|program| {
                self.declares_in_library() && self.resolves_outside_creating_scopes(program)
                    || self.concrete_program_instantiation()
            })
        }) else {
            return Arc::new(Type::Unknown);
        };
        // A peel of the same instantiation elsewhere may have already interned it.
        match lookup_instantiation_probe(&ctx, &self.decl_key, &self.resolved_arguments) {
            crate::speculative::InstantiationProbe::Hit(entry) => {
                crate::program::record_program_counter(|c| {
                    c.lazy_reference_interner_hit_count += 1
                });
                if self.is_self_reference(&entry.resolved) {
                    crate::program::note_expansion_degradation();
                    return Arc::new(Type::Unknown);
                }
                let _ = self.memo.set(Arc::downgrade(&entry.resolved));
                return entry.resolved;
            }
            // Resolution-deferred: this instantiation is owned by an earlier
            // not-yet-committed serial position (a replay reservation). Serial
            // checking here would have *hit* that publisher's interned expansion,
            // so expanding the body now would over-recurse and intern a spurious
            // structural sub-instantiation. Return the nominal reference instead —
            // un-memoized, before touching the peel stack — and let the replay's
            // file-check attempt be discarded and requeued once the publisher
            // commits (see `crate::replay`). The deferral is control flow, not a
            // `Type`: the probe returns `Deferred` at most once per key per attempt
            // (`WorkerOverlay::deferred_once`), so forcing this nominal later
            // resolves the key as a normal miss and terminates `Type::peeled`.
            crate::speculative::InstantiationProbe::Deferred => {
                let mut ctx = ctx;
                return Arc::new(make_recursive_cycle_reference(
                    &mut ctx,
                    &self.display,
                    self.decl.clone(),
                    self.decl_key.clone(),
                    self.type_arguments.clone(),
                    Some(&self.resolved_arguments),
                    &self.substitution,
                ));
            }
            crate::speculative::InstantiationProbe::Miss => {}
        }

        // After the interner probe, so a clean expansion interned by another
        // reference to the same instantiation always wins over the pin.
        if let Some(pinned) = self.degraded_memo.get() {
            // Same taint semantics as the expansion this replaces: the consumer
            // still observes a degradation, only the re-expansion is skipped.
            crate::program::note_expansion_degradation();
            crate::program::record_degraded_resolution();
            crate::program::record_program_counter(|c| {
                c.lazy_reference_degraded_memo_hit_count += 1
            });
            return pinned.clone();
        }

        let guard_key = (self.decl_key.clone(), self.resolved_arguments.clone());
        let outermost_peel = LAZY_PEEL_STACK.with(|stack| stack.borrow().is_empty());
        let blocked = LAZY_PEEL_STACK.with(|stack| {
            let stack = stack.borrow();
            if stack.len() >= MAX_LAZY_PEEL_DEPTH {
                return true;
            }
            // Exact re-entry is a true cycle. The per-declaration count also stops a
            // chain that re-enters the *same* generic declaration with ever-changing
            // arguments (`A<X>` → `A<f(X)>` → …, as the mutually-recursive lib
            // typed-array/iterator clusters do), which the exact-key check misses
            // because every key differs. A few repeats are allowed for legitimate
            // self-nesting before the back-edge degrades to `unknown`.
            let same_decl = stack
                .iter()
                .filter(|entry| entry.0 == self.decl_key)
                .count();
            same_decl >= MAX_SAME_DECLARATION_PEELS || stack.iter().any(|entry| *entry == guard_key)
        });
        if blocked {
            crate::program::note_expansion_degradation();
            crate::program::record_program_counter(|c| c.lazy_reference_blocked_count += 1);
            if crate::infer::types::interface::had_error_trace_enabled() {
                eprintln!(
                    "[had-error] peel-blocked '{}' stack={} cp={}",
                    self.decl_key.name,
                    LAZY_PEEL_STACK.with(|stack| stack.borrow().len()),
                    crate::program::in_check_phase()
                );
            }
            return Arc::new(Type::Unknown);
        }
        let creation_before = crate::program::type_creation_snapshot();
        crate::program::record_lazy_reference_expansion_start(
            &self.decl_key,
            &ctx.file_name,
            &self.display,
            LAZY_PEEL_STACK.with(|stack| stack.borrow().len()),
        );
        LAZY_PEEL_STACK.with(|stack| stack.borrow_mut().push(guard_key.clone()));

        // Box the working context so a nested peel keeps the per-frame stack small
        // — the struct is large and a deep (but bounded) library `extends` chain
        // would otherwise overflow the stack with on-stack clones.
        let mut ctx = Box::new(ctx);
        if self.creation_scope.is_some() {
            ctx.type_declaration_scope = self.creation_scope.clone();
        }
        // The environment keeps the type parameter scopes open where the
        // reference was written. Peeling a library declaration under a generic
        // signature's scopes made every instantiation inside it non-concrete:
        // nested references expanded eagerly and cut their own cycles to
        // `unknown`, and the expansion was interned for every other consumer of
        // the same (declaration, arguments), which a concrete site expands
        // lazily and whole.
        if self.resolves_outside_creating_scopes(&ctx) {
            ctx.type_parameter_scopes.clear();
            ctx.type_parameter_constraint_scopes.clear();
        }
        let in_flight_before = in_flight_degraded_read_epoch();
        let mut resolving = Vec::new();
        let resolved = match self.decl.get() {
            TypeDeclarationInfo::Alias(alias) => resolve_type_alias(
                alias,
                self.decl.clone(),
                self.type_arguments.clone(),
                None,
                &mut ctx,
                &mut resolving,
                &self.substitution,
                Some(&self.resolved_arguments),
                None,
            ),
            TypeDeclarationInfo::Interface(interface) => resolve_interface(
                interface,
                self.decl.clone(),
                self.type_arguments.clone(),
                None,
                None,
                &mut ctx,
                &mut resolving,
                &self.substitution,
                Some(&self.resolved_arguments),
                None,
            ),
        };

        LAZY_PEEL_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if let Some(position) = stack.iter().rposition(|entry| *entry == guard_key) {
                stack.remove(position);
            }
        });

        // A degraded peel (a member that hit an incomplete scope or a bounded
        // re-entry and collapsed to `unknown`) must not be interned: the program
        // instantiation cache is first-wins and program-wide, so a degraded shape
        // computed under a transient incomplete scope (e.g. the binding/signature
        // pass, before `module_scope_by_file` is populated) would permanently
        // shadow the correct expansion every later peel produces. Return the
        // degraded shape transiently instead, leaving the cache for a clean peel to
        // populate, and do not memoize it on this reference.
        if resolved.had_error {
            crate::program::note_expansion_degradation();
            crate::program::record_degraded_resolution();
            crate::program::record_program_counter(|c| {
                c.lazy_reference_degraded_expansion_count += 1
            });
            let degraded = Arc::new(resolved.ty);
            // Pin only an answer every later peel would recompute identically:
            // check phase (module scopes stable), outermost peel (a nested
            // peel's depth/same-declaration blocking is call-site-dependent —
            // at depth zero the subtree had maximal headroom, so no call site
            // can do better), and no in-flight degraded read consumed (that
            // answer depends on which resolutions were mid-flight, see
            // `IN_FLIGHT_DEGRADED_READS`). Blocked-stack `Unknown` and
            // `Deferred` probes return above and are never pinned.
            if crate::program::in_check_phase()
                && outermost_peel
                && in_flight_degraded_read_epoch() == in_flight_before
            {
                let _ = self.degraded_memo.set(degraded.clone());
            }
            return degraded;
        }

        // The eager named-type path tags the resolved object with its declaration
        // name so diagnostics display the nominal form (`Client`, `Box<string>`)
        // instead of the structural expansion. A library-scoped interface is
        // routed here instead, so attach the same display name on peel; otherwise
        // a peeled reference (e.g. the TS2741 target type) renders structurally.
        // Display-only: `alias_name` is excluded from equality, so assignability
        // is unchanged.
        let resolved_ty = match resolved.ty {
            Type::Object(object)
                if object.alias_name.is_none() && !object.properties.is_empty() =>
            {
                Type::Object(object.with_alias_name(Arc::clone(&self.display)))
            }
            Type::Function(function)
                if function.alias_instantiation().is_none()
                    && self.declared_variances != 0
                    && !self.resolved_arguments.is_empty() =>
            {
                Type::Function(function.with_alias_instantiation(
                    self.reference_id.clone(),
                    self.resolved_arguments.clone(),
                    self.declared_variances,
                ))
            }
            other => other,
        };

        if self.is_self_reference(&resolved_ty) {
            crate::program::note_expansion_degradation();
            crate::program::record_degraded_resolution();
            return Arc::new(Type::Unknown);
        }
        let interned =
            intern_instantiation(&ctx, &self.decl_key, &self.resolved_arguments, resolved_ty);
        let _ = self.memo.set(Arc::downgrade(&interned));
        crate::program::record_lazy_reference_expansion(
            &self.decl_key,
            &ctx.file_name,
            &self.display,
            LAZY_PEEL_STACK.with(|stack| stack.borrow().len()),
            creation_before,
        );
        interned
    }

    fn retains_resolution_context(&self) -> bool {
        false
    }

    fn supports_program_canonicalization(&self) -> bool {
        true
    }

    fn program_canonicalization_discriminator(&self) -> u64 {
        self.environment.canonicalization_discriminator()
    }
}

/// Builds a deferred library [`Type::Reference`] whose structural body is expanded
/// lazily on peel (see [`LazyInstantiation`]). `resolved_arguments` carry the
/// nominal identity and display; `decl`/`substitution` drive the one-level
/// expansion when forced.
pub(crate) fn make_lazy_type_reference(
    ctx: &mut CheckerContext,
    reference_id: &Arc<str>,
    display: &str,
    decl: crate::symbols::TypeDeclarationHandle,
    decl_key: DeclarationResolutionKey,
    type_arguments: Vec<surge_ts_syntax::ParsedType>,
    resolved_arguments: Vec<Type>,
    substitution: TypeParameterSubstitution,
) -> Type {
    crate::program::record_lazy_reference_created(&decl_key);
    let environment = ctx.declaration_environment();
    let creation_scope = ctx.type_declaration_scope.clone();
    let declared_variances = decl.get().declared_variances(&|name| ctx.lookup_type_declaration_handle(name));
    let intersection_alias = crate::infer::types::resolve::is_intersection_alias(decl.get());
    let display: Arc<str> = Arc::from(display);
    let reference = TypeReference::new(
        reference_id.clone(),
        display.clone(),
        resolved_arguments.clone(),
        Arc::new(LazyInstantiation {
            environment,
            creation_scope,
            decl,
            decl_key,
            type_arguments,
            resolved_arguments,
            substitution,
            display,
            reference_id: reference_id.clone(),
            declared_variances,
            memo: std::sync::OnceLock::new(),
            degraded_memo: std::sync::OnceLock::new(),
        }),
    )
    .with_declared_variances(declared_variances);
    Type::Reference(if intersection_alias { reference.as_intersection_alias() } else { reference })
}

/// Builds the lazy nominal [`Type::Reference`] a recursive declaration's
/// self-edge resolves to when a resolution cycle is detected (see
/// `resolve_type_alias` / `resolve_interface`). It carries the declaration's
/// nominal identity and defers re-expansion to [`LazyInstantiation`], so forcing
/// the back-edge peels one level to the real recursive shape (bounded by the lazy
/// peel stack) instead of collapsing to `unknown`.
/// How many lazy references are currently being peeled. A resolution that runs
/// inside a peel pays that peel's stack on top of its own, so the two depths
/// have to be bounded together rather than separately.
pub(crate) fn lazy_peel_depth() -> usize {
    LAZY_PEEL_STACK.with(|stack| stack.borrow().len())
}

pub(crate) fn make_recursive_cycle_reference(
    ctx: &mut CheckerContext,
    name: &str,
    handle: crate::symbols::TypeDeclarationHandle,
    decl_key: DeclarationResolutionKey,
    type_arguments: Vec<surge_ts_syntax::ParsedType>,
    pre_resolved_arguments: Option<&[Type]>,
    substitution: &TypeParameterSubstitution,
) -> Type {
    let reference_id: Arc<str> = Arc::from(format!("{}\u{0}{}", decl_key.file_name, decl_key.name));
    let resolved_arguments = pre_resolved_arguments
        .map(<[Type]>::to_vec)
        .unwrap_or_default();
    make_lazy_type_reference(
        ctx,
        &reference_id,
        name,
        handle,
        decl_key,
        type_arguments,
        resolved_arguments,
        substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
    )
}

/// An expansion made over type variables, kept with the declaration it
/// instantiates so an instantiation of the variables resolves the declaration
/// again ([`ResolveReference::reinstantiate`]) rather than rebuilding the
/// expansion.
struct OriginBackedInstantiation {
    resolved: Arc<Type>,
    origin: LazyInstantiation,
}

impl std::fmt::Debug for OriginBackedInstantiation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OriginBackedInstantiation")
    }
}

impl ResolveReference for OriginBackedInstantiation {
    fn resolve(&self) -> Type {
        (*self.resolved).clone()
    }

    fn resolve_arc(&self) -> Arc<Type> {
        self.resolved.clone()
    }

    fn peek_resolved(&self) -> Option<Arc<Type>> {
        Some(self.resolved.clone())
    }

    fn reinstantiate(&self, mapping: &[(surge_ts_types::TypeParameterType, Type)]) -> Option<Type> {
        self.origin.reinstantiate(mapping)
    }
}

/// `reference` (an expansion of `decl` at `resolved_arguments`) able to
/// re-resolve its declaration when the variables it names are instantiated.
pub(crate) fn origin_backed_reference(
    reference: TypeReference,
    ctx: &CheckerContext,
    decl: crate::symbols::TypeDeclarationHandle,
    decl_key: DeclarationResolutionKey,
    type_arguments: Vec<surge_ts_syntax::ParsedType>,
    resolved_arguments: Vec<Type>,
    substitution: TypeParameterSubstitution,
) -> Type {
    let declared_variances = decl.get().declared_variances(&|name| ctx.lookup_type_declaration_handle(name));
    let resolved = reference.resolve_shared();
    let origin = LazyInstantiation {
        environment: ctx.declaration_environment(),
        creation_scope: ctx.type_declaration_scope.clone(),
        decl,
        decl_key,
        type_arguments,
        resolved_arguments,
        substitution,
        display: reference.display.clone(),
        reference_id: reference.id.clone(),
        declared_variances,
        memo: std::sync::OnceLock::new(),
        degraded_memo: std::sync::OnceLock::new(),
    };
    Type::Reference(reference.with_resolver(Arc::new(OriginBackedInstantiation { resolved, origin })))
}
