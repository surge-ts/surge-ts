use std::sync::Arc;

use surge_ts_types::{ResolveReference, Type, TypeReference};

use super::{
    LazyMemberIdentity, intern_instantiation, intern_lazy_member_content, lazy_value_trace_filter,
    lazy_value_trace_shape, lookup_instantiation, parsed_annotation_display,
};
use crate::context::{
    CheckerContext, DeclarationEnvironmentHandle, DeclarationNamespace, DeclarationResolutionKey,
};
use crate::infer::types::*;

thread_local! {
    /// Content ids of the lazy annotations being forced on this thread (see the
    /// re-entry check in `resolve_arc_inner`).
    static LAZY_VALUE_FORCES_IN_PROGRESS: std::cell::RefCell<Vec<Arc<str>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Nested forces past this answer the sentinel. Each instantiation of an
/// expanding generic (`G<T>` reading `G<G<T>>`) has its own content id, so
/// the exact re-entry check alone does not bound the chain.
const MAX_LAZY_FORCE_DEPTH: usize = 32;

/// The capture-site content of a lazy annotation reference, split out from the
/// per-instance handle because none of it depends on WHICH context created the
/// reference: the resolution key, the rendered display, the parsed annotation
/// and the substitution snapshot are functions of the declaration alone. Member
/// references intern this by content, so an interface member reached from N
/// consumer sites renders its display, formats its id and key, and clones its
/// annotation and substitution once rather than N times.
pub(super) struct LazyAnnotationContent {
    pub(super) id: Arc<str>,
    pub(super) key: DeclarationResolutionKey,
    pub(super) display: Arc<str>,
    pub(super) annotation: surge_ts_syntax::ParsedType,
    pub(super) signature_component: Option<LazySignatureComponent>,
    pub(super) signature_environment: Option<LazySignatureEnvironment>,
    /// The namespace-member prefix stack active where the annotation was
    /// captured. A member of `declare namespace ts` writes bare `Symbol`
    /// meaning `ts.Symbol`; the recovered force context has an empty stack, so
    /// without re-installing this the bare name resolves to the GLOBAL Symbol
    /// and every nominal comparison against the namespace type fails.
    pub(super) namespace_prefix_stack: Option<Arc<[String]>>,
}

pub(super) struct LazyDeclarationAnnotation {
    pub(super) content: Arc<LazyAnnotationContent>,
    pub(super) environment: DeclarationEnvironmentHandle,
    pub(super) creation_scope: Option<Arc<crate::symbols::TypeDeclarationScope>>,
    pub(super) memo: std::sync::OnceLock<std::sync::Weak<Type>>,
    /// A degraded (`had_error`/unknown) resolution is never interned into the
    /// shared caches (that would violate the no-degraded-results-program-wide
    /// rule), so the weak `memo` has no keeper and every read would re-run the
    /// full failed resolution — hot on value annotations read once per use
    /// site. Pinning the FIRST answer per annotation instance both bounds the
    /// cost and matches the eager collector's resolve-once semantics.
    pub(super) degraded_memo: std::sync::OnceLock<Arc<Type>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LazySignatureComponent {
    Parameter(usize),
    Return,
    ThisParameter,
    TypePredicate,
}

#[derive(Clone)]
pub(crate) struct LazySignatureEnvironment {
    pub(super) type_parameters: Arc<[surge_ts_syntax::ParsedTypeParameter]>,
    pub(super) substitution: Arc<TypeParameterSubstitution>,
}

impl LazySignatureEnvironment {
    /// Environment for a deferred interface MEMBER annotation: no
    /// type-parameter scope of its own (the interface's parameters are already
    /// bound), just the enclosing instantiation's substitution captured so the
    /// force resolves under the same bindings the eager path would have used.
    pub(crate) fn for_member_substitution(
        substitution: &TypeParameterSubstitution,
    ) -> Option<Self> {
        if substitution.iter().next().is_none() {
            return None;
        }
        Some(Self {
            type_parameters: Arc::from(&[][..]),
            substitution: Arc::new(
                substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
            ),
        })
    }

    pub(crate) fn new(type_parameters: &[surge_ts_syntax::ParsedTypeParameter]) -> Option<Self> {
        if type_parameters.is_empty() {
            return None;
        }
        crate::program::record_program_counter(|c| {
            c.lazy_signature_environment_create_count += 1;
            c.lazy_signature_environment_handle_size_bytes =
                std::mem::size_of::<LazySignatureEnvironment>() as u64;
        });
        let mut substitution = TypeParameterSubstitution::new();
        for type_parameter in type_parameters {
            substitution.insert_placeholder(
                type_parameter.name.clone(),
                Type::type_parameter(&type_parameter.name),
            );
        }
        Some(Self {
            type_parameters: Arc::from(type_parameters),
            substitution: Arc::new(substitution),
        })
    }
}

impl LazySignatureComponent {
    pub(super) fn identity(self) -> String {
        match self {
            Self::Parameter(index) => format!("parameter-{index}"),
            Self::Return => "return".to_string(),
            Self::ThisParameter => "this-parameter".to_string(),
            Self::TypePredicate => "type-predicate".to_string(),
        }
    }

    pub(super) fn peel_reason(self) -> crate::program::DtsExpansionReason {
        match self {
            Self::Parameter(_) => crate::program::DtsExpansionReason::SignatureParameter,
            Self::Return => crate::program::DtsExpansionReason::SignatureReturn,
            Self::ThisParameter => crate::program::DtsExpansionReason::SignatureThisParameter,
            Self::TypePredicate => crate::program::DtsExpansionReason::SignatureTypePredicate,
        }
    }
}

impl ResolveReference for LazyDeclarationAnnotation {
    fn resolve(&self) -> Type {
        (*self.resolve_arc()).clone()
    }

    fn resolve_arc(&self) -> Arc<Type> {
        let resolve = || self.resolve_arc_inner();
        let Some(component) = self.content.signature_component else {
            return resolve();
        };
        if crate::program::current_dts_expansion_reason()
            == crate::program::DtsExpansionReason::Other
        {
            crate::program::with_dts_expansion_reason(component.peel_reason(), resolve)
        } else {
            resolve()
        }
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

    fn captured_census(&self) -> surge_ts_types::ResolverCaptureCensus {
        let content = &self.content;
        let mut content_bytes = std::mem::size_of::<LazyAnnotationContent>() as u64
            + content.annotation.estimated_heap_bytes()
            + content.key.name.len() as u64;
        let mut shared_captures = Vec::new();
        if let Some(environment) = &content.signature_environment {
            shared_captures.push((
                environment.type_parameters.as_ptr() as usize,
                environment
                    .type_parameters
                    .iter()
                    .map(surge_ts_syntax::ParsedTypeParameter::estimated_heap_bytes)
                    .sum(),
            ));
            shared_captures.extend(environment.substitution.census_shared_captures());
            content_bytes += std::mem::size_of::<LazySignatureEnvironment>() as u64;
        }
        // Interned member content is shared by every reference that named the
        // same member under the same substitution; charging it per handle would
        // multiply one allocation across its readers.
        shared_captures.push((Arc::as_ptr(content) as *const () as usize, content_bytes));
        surge_ts_types::ResolverCaptureCensus {
            own_bytes: std::mem::size_of::<Self>() as u64,
            shared_captures,
        }
    }

    fn peek_resolved(&self) -> Option<Arc<Type>> {
        self.memo.get().and_then(std::sync::Weak::upgrade)
    }
}

impl LazyDeclarationAnnotation {
    pub(super) fn resolve_arc_inner(&self) -> Arc<Type> {
        let content = self.content.as_ref();
        crate::program::record_lazy_reference_peel_start(&content.key);
        if content.signature_component.is_some() {
            crate::program::record_program_counter(|c| c.lazy_signature_materialization_count += 1);
        }
        if let Some(resolved) = self.memo.get().and_then(std::sync::Weak::upgrade) {
            crate::program::record_program_counter(|c| c.lazy_reference_memo_hit_count += 1);
            if content.signature_component.is_some() {
                crate::program::record_program_counter(|c| {
                    c.signature_materialization_cache_hit_count += 1
                });
            }
            return resolved;
        }
        if let Some(degraded) = self.degraded_memo.get() {
            crate::program::record_program_counter(|c| c.lazy_reference_memo_hit_count += 1);
            return degraded.clone();
        }
        // A value annotation that reads back into itself while forcing — a
        // `typeof import("m")` namespace whose member is this very annotation
        // (`export declare const x: typeof import("./self").x`) — is a true
        // cycle; the re-entry answers the sentinel and is not memoized, so the
        // outer force completes with its own answer.
        let re_entered = LAZY_VALUE_FORCES_IN_PROGRESS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.len() >= MAX_LAZY_FORCE_DEPTH || stack.iter().any(|id| **id == *content.id) {
                return true;
            }
            stack.push(content.id.clone());
            false
        });
        if re_entered {
            return Arc::new(Type::Unknown);
        }
        struct PopInProgress;
        impl Drop for PopInProgress {
            fn drop(&mut self) {
                LAZY_VALUE_FORCES_IN_PROGRESS.with(|stack| {
                    stack.borrow_mut().pop();
                });
            }
        }
        let _pop_in_progress = PopInProgress;
        let Some(ctx) = self.environment.checker_context() else {
            return Arc::new(Type::Unknown);
        };
        if let Some(entry) = lookup_instantiation(&ctx, &content.key, &[]) {
            crate::program::record_program_counter(|c| c.lazy_reference_interner_hit_count += 1);
            if content.signature_component.is_some() {
                crate::program::record_program_counter(|c| {
                    c.signature_materialization_cache_hit_count += 1
                });
            }
            let _ = self.memo.set(Arc::downgrade(&entry.resolved));
            return entry.resolved;
        }
        if content.signature_component.is_some() {
            crate::program::record_program_counter(|c| {
                c.signature_materialization_cache_miss_count += 1
            });
        }

        let before = crate::program::type_creation_snapshot();
        crate::program::record_lazy_reference_expansion_start(
            &content.key,
            &ctx.file_name,
            &content.display,
            0,
        );
        let mut ctx = Box::new(ctx);
        ctx.set_file_name(content.key.file_name.as_ref().to_string());
        if self.creation_scope.is_some() {
            ctx.type_declaration_scope = self.creation_scope.clone();
        }
        if let Some(stack) = &content.namespace_prefix_stack {
            ctx.namespace_member_prefix_stack = stack.to_vec();
        }
        if let Some(environment) = &content.signature_environment
            && !environment.type_parameters.is_empty()
        {
            // The empty case (a member-substitution environment) must not push
            // a scope: an open type-parameter scope makes every nested
            // resolution non-concrete and de-defers it.
            ctx.push_type_parameter_scope(&environment.type_parameters, None);
        }
        let empty_substitution = TypeParameterSubstitution::new();
        let substitution = content
            .signature_environment
            .as_ref()
            .map_or(&empty_substitution, |environment| {
                environment.substitution.as_ref()
            });
        let resolved = resolve_parsed_type(
            content.annotation.clone(),
            &mut ctx,
            &mut Vec::new(),
            substitution,
        );
        let had_error = resolved.had_error;
        // Signature components historically peel to the structural type. A
        // VALUE annotation (no signature component) must NOT: eager collection
        // stored `map_parsed_type`'s output verbatim, leaving named references
        // nominal so members expand through the live-context peel path — an
        // eager peel here would bake a snapshot of the recovered environment's
        // expansion into the symbol and drift from the eager shape.
        let resolved = Arc::new(match resolved.ty {
            Type::Reference(reference) if content.signature_component.is_some() => {
                reference.resolve().peeled()
            }
            // An annotation that resolves to a reference to itself — a global
            // `let vitest: typeof import("vitest")["vitest"]` whose module
            // export was bound to that same global — would memoize a self-loop
            // that every later member read follows forever.
            Type::Reference(reference) if *reference.id == *content.id => Type::Unknown,
            ty => ty,
        });
        if let Some(filter) = lazy_value_trace_filter()
            && content.key.name.contains(filter)
        {
            let diagnostics: Vec<String> = ctx
                .diagnostics
                .iter()
                .take(4)
                .map(|d| {
                    format!(
                        "{}:{}",
                        d.code,
                        d.message.chars().take(90).collect::<String>()
                    )
                })
                .collect();
            eprintln!(
                "[lazy-value] FORCE {} had_error={had_error} diags={:?} ty={}",
                content.key.name,
                diagnostics,
                lazy_value_trace_shape(&resolved),
            );
        }
        if had_error || resolved.is_unknown() {
            crate::program::note_expansion_degradation();
            crate::program::record_program_counter(|c| {
                c.lazy_reference_degraded_expansion_count += 1;
                if content.signature_component.is_some() {
                    c.degraded_signature_expansion_count += 1;
                }
            });
            if content.signature_component.is_some() {
                crate::program::record_degraded_signature_expansion(&content.key);
            }
            let _ = self.degraded_memo.set(resolved.clone());
            return resolved;
        }
        let resolved = intern_instantiation(&ctx, &content.key, &[], (*resolved).clone());
        let _ = self.memo.set(Arc::downgrade(&resolved));
        crate::program::record_lazy_reference_expansion(
            &content.key,
            &ctx.file_name,
            &content.display,
            0,
            before,
        );
        if content.signature_component.is_some() {
            crate::program::record_program_counter(|c| c.clean_signature_expansion_count += 1);
        }
        resolved
    }
}

pub(crate) fn make_lazy_signature_annotation_reference(
    ctx: &mut CheckerContext,
    declaration_name: &str,
    declaration_start: usize,
    component: LazySignatureComponent,
    annotation: surge_ts_syntax::ParsedType,
    signature_environment: Option<LazySignatureEnvironment>,
) -> Type {
    let display: Arc<str> = Arc::from(parsed_annotation_display(&annotation));
    let component_identity = component.identity();
    let key = DeclarationResolutionKey {
        file_name: ctx.canonical_file_name_arc(),
        name: Arc::from(format!(
            "signature {declaration_name}@{declaration_start}:{component_identity}"
        )),
        namespace: DeclarationNamespace::Type,
        fingerprint: 0,
    };
    crate::program::record_lazy_reference_created(&key);
    crate::program::record_program_counter(|c| {
        match component {
            LazySignatureComponent::Parameter(_)
            | LazySignatureComponent::ThisParameter
            | LazySignatureComponent::TypePredicate => {
                c.lazy_signature_parameter_annotation_create_count += 1
            }
            LazySignatureComponent::Return => c.lazy_signature_return_annotation_create_count += 1,
        }
        c.lazy_signature_annotation_handle_size_bytes =
            std::mem::size_of::<LazyDeclarationAnnotation>() as u64;
        c.lazy_signature_parameter_slot_size_bytes = std::mem::size_of::<Type>() as u64;
        c.lazy_signature_estimated_shallow_retained_bytes +=
            (std::mem::size_of::<LazyDeclarationAnnotation>()
                + std::mem::size_of::<surge_ts_types::TypeReference>()) as u64;
        if signature_environment.is_some() {
            c.lazy_signature_environment_reference_count += 1;
        }
    });
    let id = format!(
        "{}\u{0}signature-annotation\u{0}{declaration_name}\u{0}{declaration_start}\u{0}{component_identity}",
        key.file_name
    );
    lazy_annotation_reference(
        Arc::new(LazyAnnotationContent {
            id: Arc::from(id),
            key,
            display,
            annotation,
            signature_component: Some(component),
            signature_environment,
            namespace_prefix_stack: None,
        }),
        ctx,
    )
}

/// Wraps one lazy annotation content in a fresh per-capture handle: the
/// environment, the creation scope and the memo slots are the only parts that
/// belong to a single capture site.
pub(super) fn lazy_annotation_reference(
    content: Arc<LazyAnnotationContent>,
    ctx: &mut CheckerContext,
) -> Type {
    let environment = ctx.declaration_environment();
    let creation_scope = ctx.type_declaration_scope.clone();
    Type::Reference(TypeReference::new(
        content.id.clone(),
        content.display.clone(),
        Vec::new(),
        Arc::new(LazyDeclarationAnnotation {
            content,
            environment,
            creation_scope,
            memo: std::sync::OnceLock::new(),
            degraded_memo: std::sync::OnceLock::new(),
        }),
    ))
}

/// A lazy reference for a library declaration's VALUE annotation
/// (`declare const x: T` in a dependency `.d.ts`): the annotation maps on
/// first read under the captured declaration environment instead of eagerly
/// during exportable-value collection. Unlike the signature variant, the
/// resolver returns the mapped type UNPEELED (see `resolve_arc_inner`) so the
/// symbol carries exactly the shape eager `map_parsed_type` would have
/// produced — nested named references stay nominal and expand through the
/// normal live-context peel path.
pub(crate) fn make_lazy_value_annotation_reference(
    ctx: &mut CheckerContext,
    declaration_name: &str,
    declaration_start: usize,
    annotation: surge_ts_syntax::ParsedType,
) -> Type {
    make_lazy_value_annotation_reference_under(
        ctx,
        declaration_name,
        declaration_start,
        annotation,
        None,
    )
}

/// [`make_lazy_value_annotation_reference`] for a `declare namespace` value
/// member: the force re-installs `namespace_prefix_stack`, so a bare sibling
/// name in the annotation (`let VariableDeclaration: Type<VariableDeclaration>`)
/// resolves under the namespace it was written in.
pub(crate) fn make_lazy_value_annotation_reference_under(
    ctx: &mut CheckerContext,
    declaration_name: &str,
    declaration_start: usize,
    annotation: surge_ts_syntax::ParsedType,
    namespace_prefix_stack: Option<Arc<[String]>>,
) -> Type {
    let display: Arc<str> = Arc::from(parsed_annotation_display(&annotation));
    let key = DeclarationResolutionKey {
        file_name: ctx.canonical_file_name_arc(),
        name: Arc::from(format!("value {declaration_name}@{declaration_start}")),
        namespace: DeclarationNamespace::Type,
        fingerprint: 0,
    };
    crate::program::record_lazy_reference_created(&key);
    if let Some(filter) = lazy_value_trace_filter()
        && key.name.contains(filter)
    {
        eprintln!("[lazy-value] CREATE {} file={}", key.name, key.file_name);
    }
    let id = format!(
        "{}\u{0}value-annotation\u{0}{declaration_name}\u{0}{declaration_start}",
        key.file_name
    );
    lazy_annotation_reference(
        Arc::new(LazyAnnotationContent {
            id: Arc::from(id),
            key,
            display,
            annotation,
            signature_component: None,
            signature_environment: None,
            namespace_prefix_stack,
        }),
        ctx,
    )
}

/// A lazy reference for a library interface MEMBER annotation (Stage 1 of
/// member-level lazy expansion, `SURGE_LAZY_IFACE_MEMBERS=1`): the property's
/// annotation maps on first read under the captured declaration environment
/// and the enclosing instantiation's substitution, instead of eagerly during
/// every interface expansion. Like the value variant, the force returns the
/// mapped type UNPEELED so nested named references stay nominal. The key's
/// fingerprint carries the substitution's display-inclusive identity so two
/// instantiations of the same generic interface never share an interner
/// entry.
pub(super) fn capture_namespace_prefix_stack(stack: &[String]) -> Option<Arc<[String]>> {
    if stack.is_empty() {
        None
    } else {
        Some(Arc::from(stack))
    }
}

pub(crate) fn make_lazy_member_annotation_reference(
    ctx: &mut CheckerContext,
    identity: LazyMemberIdentity<'_>,
    annotation: &surge_ts_syntax::ParsedType,
    substitution: &TypeParameterSubstitution,
) -> Type {
    let content = intern_lazy_member_content(ctx, identity, |file_name, prefix_stack| {
        let display: Arc<str> = Arc::from(parsed_annotation_display(annotation));
        let key = DeclarationResolutionKey {
            name: Arc::from(identity.key_name("member")),
            file_name,
            namespace: DeclarationNamespace::Type,
            fingerprint: identity.substitution_fingerprint,
        };
        crate::program::record_lazy_reference_created(&key);
        let id = identity.reference_id("member-annotation", &key.file_name);
        Arc::new(LazyAnnotationContent {
            id: Arc::from(id),
            key,
            display,
            annotation: annotation.clone(),
            signature_component: None,
            signature_environment: LazySignatureEnvironment::for_member_substitution(substitution),
            namespace_prefix_stack: capture_namespace_prefix_stack(prefix_stack),
        })
    });
    crate::program::record_program_counter(|c| c.lazy_member_annotation_create_count += 1);
    lazy_annotation_reference(content, ctx)
}

/// A lazy reference for one COMPONENT (parameter or return annotation) of a
/// library interface method (Stage 2 of member-level lazy expansion). The
/// FunctionType shell stays eager; the captured substitution is the method's
/// LOCAL substitution (interface bindings + the method's own extended
/// parameters), so the force resolves exactly as the eager path would.
/// Components carry `signature_component`, so the force peels the resolved
/// reference the way eager structural resolution produced structural shapes.
pub(crate) fn make_lazy_method_component_reference(
    ctx: &mut CheckerContext,
    identity: LazyMemberIdentity<'_>,
    annotation: &surge_ts_syntax::ParsedType,
    local_substitution: &TypeParameterSubstitution,
) -> Type {
    let content = intern_lazy_member_content(ctx, identity, |file_name, prefix_stack| {
        let display: Arc<str> = Arc::from(parsed_annotation_display(annotation));
        let key = DeclarationResolutionKey {
            name: Arc::from(identity.key_name("method")),
            file_name,
            namespace: DeclarationNamespace::Type,
            fingerprint: identity.substitution_fingerprint,
        };
        crate::program::record_lazy_reference_created(&key);
        let id = identity.reference_id("method-component", &key.file_name);
        Arc::new(LazyAnnotationContent {
            id: Arc::from(id),
            key,
            display,
            annotation: annotation.clone(),
            signature_component: identity.component,
            signature_environment: LazySignatureEnvironment::for_member_substitution(
                local_substitution,
            ),
            namespace_prefix_stack: capture_namespace_prefix_stack(prefix_stack),
        })
    });
    crate::program::record_program_counter(|c| c.lazy_member_annotation_create_count += 1);
    lazy_annotation_reference(content, ctx)
}
