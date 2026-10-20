//! tsc's `getSignatureInstantiation` cache: a declared signature re-resolved
//! under one substitution, in one resolution context, is the same signature
//! every time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use surge_ts_syntax::ParsedType;
use surge_ts_types::{FunctionType, Type};

use crate::context::CheckerContext;
use crate::infer::TypeParameterSubstitution;
use crate::symbols::FunctionSignatureInfo;

/// `SURGE_SIGNATURE_INSTANTIATION_CACHE=0` re-resolves the written signature
/// at every call.
fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_SIGNATURE_INSTANTIATION_CACHE").as_deref() != Ok("0"))
}

#[derive(Clone)]
struct Binding {
    name: Arc<str>,
    ty: Type,
    display: u64,
    placeholder: bool,
    degraded: bool,
    keeps_literal: bool,
}

impl Binding {
    fn same(&self, other: &Self) -> bool {
        self.name == other.name
            && self.display == other.display
            && self.placeholder == other.placeholder
            && self.degraded == other.degraded
            && self.keeps_literal == other.keeps_literal
            && self.ty == other.ty
    }
}

/// What re-resolving a written signature reads besides the signature and the
/// substitution: the caller's installed scope and namespace prefixes, its open
/// type parameter scopes, its polymorphic `this` and the phase.
#[derive(Clone)]
struct Context {
    file: Arc<str>,
    scope: usize,
    prefixes: Vec<String>,
    prefix_depth: usize,
    type_parameter_scopes: Vec<Arc<HashMap<String, Type>>>,
    constraint_scopes: Vec<Arc<HashMap<String, ParsedType>>>,
    polymorphic_this: Option<Type>,
    cross_file_depth: u32,
    check_phase: bool,
    environment: u32,
}

impl Context {
    fn same(&self, other: &Self) -> bool {
        fn same_scopes<V: PartialEq>(left: &[Arc<HashMap<String, V>>], right: &[Arc<HashMap<String, V>>]) -> bool {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| Arc::ptr_eq(left, right) || **left == **right)
        }
        self.file == other.file
            && self.scope == other.scope
            && self.prefixes == other.prefixes
            && self.prefix_depth == other.prefix_depth
            && self.cross_file_depth == other.cross_file_depth
            && self.check_phase == other.check_phase
            && self.environment == other.environment
            && self.polymorphic_this == other.polymorphic_this
            && same_scopes(&self.type_parameter_scopes, &other.type_parameter_scopes)
            && same_scopes(&self.constraint_scopes, &other.constraint_scopes)
    }
}

struct Entry {
    signature: usize,
    /// Pins `signature`: the declaration it was read off holds it.
    function_type: FunctionType,
    bindings: Vec<Binding>,
    context: Context,
    instantiated: FunctionType,
}

/// What storing an instantiation needs once it is resolved.
pub(super) struct Pending {
    digest: u64,
    signature: usize,
    bindings: Vec<Binding>,
    context: Context,
}

thread_local! {
    static ENTRIES: RefCell<surge_ts_types::fx::FxHashMap<u64, Vec<Entry>>> = RefCell::default();
}

const CAP: usize = 8192;

/// Whether `signature` is the declaration `function_type` carries, so the
/// cached copy of `function_type` keeps its address from being reused.
fn pinned_by(function_type: &FunctionType, signature: &FunctionSignatureInfo) -> bool {
    let Some(declaration) = function_type.declaration() else {
        return false;
    };
    declaration
        .downcast_ref::<FunctionSignatureInfo>()
        .is_some_and(|own| std::ptr::eq(own, signature))
        || declaration
            .downcast_ref::<super::DeclaredMemberSignature>()
            .is_some_and(|member| std::ptr::eq(&*member.signature, signature))
}

/// The cached instantiation of `signature` under `substitution` here, or what
/// storing one takes. `None` when the call is not cached: under the compiler
/// API (which records reads), inside a generic body read, or over type
/// variables, whose activity the cached form would not carry.
pub(super) fn lookup(
    function_type: &FunctionType,
    signature: &FunctionSignatureInfo,
    substitution: &TypeParameterSubstitution,
    file: &str,
    ctx: &CheckerContext,
) -> Result<FunctionType, Option<Pending>> {
    surge_ts_types::lazy_probe::count("sigcache.lookup");
    if !enabled() || crate::semantic::serves_api() || crate::checks::function::in_generic_body_read() {
        surge_ts_types::lazy_probe::count("sigcache.skip.mode");
        return Err(None);
    }
    if !pinned_by(function_type, signature) {
        surge_ts_types::lazy_probe::count("sigcache.skip.unpinned");
        return Err(None);
    }
    let mut bindings = Vec::new();
    for (name, ty) in substitution.iter() {
        if surge_ts_types::type_variable::mentions_type_variable(ty) {
            surge_ts_types::lazy_probe::count("sigcache.skip.variables");
            return Err(None);
        }
        bindings.push(Binding {
            name: name.clone(),
            ty: ty.clone(),
            display: crate::speculative::display_type_fingerprint(ty),
            placeholder: substitution.is_placeholder(name),
            degraded: substitution.is_degraded(name),
            keeps_literal: substitution.keeps_literal(name),
        });
    }
    if ctx
        .type_parameter_scopes
        .iter()
        .any(|scope| scope.values().any(surge_ts_types::type_variable::mentions_type_variable))
    {
        surge_ts_types::lazy_probe::count("sigcache.skip.scope_variables");
        return Err(None);
    }
    bindings.sort_by(|left, right| left.name.cmp(&right.name));
    let context = Context {
        file: Arc::from(file),
        scope: ctx.type_declaration_scope.as_ref().map_or(0, |scope| Arc::as_ptr(scope) as usize),
        prefixes: ctx.namespace_member_prefix_stack.clone(),
        prefix_depth: ctx.namespace_member_resolution_depth,
        type_parameter_scopes: ctx.type_parameter_scopes.clone(),
        constraint_scopes: ctx.type_parameter_constraint_scopes.clone(),
        polymorphic_this: ctx.polymorphic_this.clone(),
        cross_file_depth: ctx.cross_file_resolution_depth,
        check_phase: crate::program::in_check_phase(),
        environment: ctx.declaration_environment_store.owner_id(),
    };
    let signature_address = std::ptr::from_ref(signature) as usize;
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    signature_address.hash(&mut hasher);
    (std::ptr::from_ref(function_type.payload()) as usize).hash(&mut hasher);
    for binding in &bindings {
        binding.name.hash(&mut hasher);
        binding.display.hash(&mut hasher);
        surge_ts_types::type_conflict_digest(&binding.ty).hash(&mut hasher);
    }
    context.file.hash(&mut hasher);
    context.scope.hash(&mut hasher);
    context.prefixes.hash(&mut hasher);
    context.type_parameter_scopes.len().hash(&mut hasher);
    context.check_phase.hash(&mut hasher);
    context.environment.hash(&mut hasher);
    let digest = hasher.finish();
    let hit = ENTRIES.with(|entries| {
        entries.borrow().get(&digest).and_then(|bucket| {
            bucket
                .iter()
                .find(|entry| {
                    entry.signature == signature_address
                        && entry.bindings.len() == bindings.len()
                        && entry.bindings.iter().zip(&bindings).all(|(left, right)| left.same(right))
                        && entry.context.same(&context)
                        && entry.function_type == *function_type
                })
                .map(|entry| entry.instantiated.clone())
        })
    });
    surge_ts_types::lazy_probe::count_if(hit.is_some(), "sigcache.hit");
    match hit {
        Some(instantiated) => Ok(instantiated),
        None => Err(Some(Pending {
            digest,
            signature: signature_address,
            bindings,
            context,
        })),
    }
}

pub(super) fn store(pending: Pending, function_type: &FunctionType, instantiated: &FunctionType) {
    ENTRIES.with(|entries| {
        let mut entries = entries.borrow_mut();
        if entries.len() >= CAP {
            entries.clear();
        }
        entries.entry(pending.digest).or_default().push(Entry {
            signature: pending.signature,
            function_type: function_type.clone(),
            bindings: pending.bindings,
            context: pending.context,
            instantiated: instantiated.clone(),
        });
    });
}

/// Per-file state: the next file check resolves under its own scopes.
pub(crate) fn clear_signature_instantiations() {
    ENTRIES.with(|entries| entries.borrow_mut().clear());
}
