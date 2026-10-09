use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use surge_ts_diagnostics::{Diagnostic, TextSpan as DiagnosticTextSpan};
use surge_ts_syntax::{ParsedType, ParsedTypeParameter, TextSpan as SyntaxTextSpan};
use surge_ts_types::fx::{FxHashMap, FxHashSet};
use surge_ts_types::{FunctionType, ProgramTypeStore, Type, current_program_type_store};

use crate::infer::types::LazyMemberTemplateTable;
use crate::program::ProgramTimings;
use crate::symbols::{
    SymbolTable, TypeDeclarationInfo, TypeDeclarationScope, TypeDeclarationTable,
};

use crate::modules::ModuleExportTable;

mod declaration_environment;
mod interface_keys;
mod options;
mod resolution_keys;
mod substitution;

pub(crate) use declaration_environment::*;
pub(crate) use interface_keys::*;
pub use options::*;
pub(crate) use resolution_keys::*;
pub(crate) use substitution::*;

/// Temporary `SURGE_LV_PROBE=1` probe: which files' local-value tables are
/// actually consulted (post-population). Answers how much of the
/// module_local_values stage a lazy per-file build would skip.
fn local_values_consult_probe() -> Option<&'static Mutex<surge_ts_types::fx::FxHashSet<Arc<str>>>> {
    static PROBE: std::sync::OnceLock<Option<Mutex<surge_ts_types::fx::FxHashSet<Arc<str>>>>> =
        std::sync::OnceLock::new();
    PROBE
        .get_or_init(|| {
            std::env::var_os("SURGE_LV_PROBE")
                .map(|_| Mutex::new(surge_ts_types::fx::FxHashSet::default()))
        })
        .as_ref()
}

/// Default-on (opt-out `SURGE_NS_QUALIFIED_RETRY=0`): retry an already-dotted
/// type name under the active namespace prefixes after the exact lookup
/// misses, so an enum registered inside `declare namespace ts` resolves from
/// its sibling members' bare `SyntaxKind.X` references. Affordable only since
/// the check-phase degraded-peel pin: before it, resolving these names
/// converted fast-fail hash misses into full per-peel re-expansion of the
/// enclosing declaration graph (see docs/perf/NAMESPACE-INTERFACE-MERGE.md).
fn namespace_qualified_retry_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SURGE_NS_QUALIFIED_RETRY").is_none_or(|value| value != "0")
    })
}

fn record_local_values_consult(file_name: &str) {
    if let Some(probe) = local_values_consult_probe()
        && let Ok(mut set) = probe.lock()
        && !set.contains(file_name)
    {
        set.insert(Arc::from(file_name));
    }
}

pub(crate) fn report_local_values_consults(total_entries: usize) {
    if let Some(probe) = local_values_consult_probe()
        && let Ok(set) = probe.lock()
    {
        eprintln!(
            "[lv-probe] consulted_files={} of {} entries",
            set.len(),
            total_entries,
        );
    }
}

/// tsc gates per-return checking on a return-type ANNOTATION. With none, it
/// infers the return type from the returns via `getUnionType` — where a single
/// `any` constituent collapses the whole union to `any` — and checks only the
/// whole-signature relation at the declaration, which `any` trivially passes.
///
/// surge checks each return against the contextual type instead, so a body that
/// mixes an `any` return with a differently-shaped one reported a mismatch tsc
/// never raises (the generated hey-api clients do exactly this: `let data: any`
/// returned alongside object literals).
///
/// The frame records the index of every diagnostic that is a *return mismatch
/// verdict* for one such body. If any return turns out to be `any`, those are
/// dropped — and only those, so an unresolved name or a bad member access inside
/// a return expression still reports, which tsc does regardless of the collapse.
#[derive(Debug, Default, Clone)]
pub(crate) struct ContextualReturnFrame {
    diagnostic_indices: Vec<usize>,
    saw_any_return: bool,
    /// Whether this body is the one being checked against an unannotated
    /// contextual return type. Every body pushes a frame so the innermost one
    /// always belongs to the function being checked — otherwise a nested
    /// function without its own frame would record into, and be silenced by,
    /// an enclosing arrow's.
    active: bool,
    /// Whether some `return <expr>` in this body yielded a `void`- or
    /// `any`-including type. tsc's `noImplicitReturns` check skips a function
    /// whose inferred return type is `void`-like, `any` or exactly `undefined`
    /// (`isUnwrappedReturnTypeUndefinedVoidOrAny`).
    returned_void_like: bool,
    /// Whether some `return <expr>` yielded anything but `undefined`, which
    /// keeps the inferred type from being exactly `undefined`.
    returned_defined: bool,
    /// Whether any `return <expr>` was noted.
    returned_value: bool,
    /// This body's returns have no expectation *by construction*: an
    /// unannotated declaration or class member, which Go checks only against
    /// the annotation (`getReturnTypeFromAnnotation`, nil) and never types
    /// contextually. Distinct from a body whose contextual return merely
    /// degraded, which also reaches the return check with no expectation but
    /// where the missing context is surge's own gap.
    unannotated_declaration: bool,
    /// The types this body's `return <expr>`s produced, for rendering the whole
    /// signature tsc names when a contextually-typed arrow does not fit.
    /// Collected only for an active frame, and capped — the render only needs a
    /// faithful union, not every duplicate.
    returned_types: Vec<surge_ts_types::Type>,
    /// Beside each of `returned_types`, in an unannotated declaration's body:
    /// what that return contributes to the inferred return type.
    returned_forms: Vec<ReturnedForm>,
    /// The declaration whose return type this body is read for: a `return`
    /// calling it directly contributes nothing (`checkAndAggregateReturnExpressionTypes`).
    self_call_name: Option<std::sync::Arc<str>>,
    /// Whether some `return` was such a direct self-call.
    returned_self_call: bool,
}

/// `return f()` (or `return await f()` in an async body) calling the
/// declaration `name` itself, not a binding that shadows it.
fn is_direct_self_call(
    expression: &surge_ts_syntax::ParsedExpression,
    name: &str,
    in_async_body: bool,
    symbols: &crate::symbols::SymbolTable,
) -> bool {
    let expression = match expression {
        surge_ts_syntax::ParsedExpression::Await { operand, .. } if in_async_body => operand.as_ref(),
        other => other,
    };
    matches!(expression, surge_ts_syntax::ParsedExpression::Call { callee_name, .. } if callee_name == name)
        && symbols
            .get(name)
            .is_some_and(|symbol| symbol.kind == crate::symbols::SymbolKind::Function)
}

/// One `return <expr>` of an unannotated declaration as `getReturnTypeFromBody`
/// reads it: the type after `getWidenedType` (a fresh object literal's members
/// and, without `strictNullChecks`, `null`/`undefined`), the literal shape its
/// subtype reduction needs, and whether a literal type in it is fresh — any
/// return but a type assertion's.
#[derive(Debug, Clone)]
pub(crate) struct ReturnedForm {
    pub(crate) widened: surge_ts_types::Type,
    pub(crate) shape: surge_ts_types::LiteralShape,
    pub(crate) fresh: bool,
}

/// The scope pushed for a declaration without type parameters, shared so a
/// push allocates nothing.
pub(crate) fn empty_type_parameter_scope() -> Arc<HashMap<String, Type>> {
    static EMPTY: std::sync::OnceLock<Arc<HashMap<String, Type>>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Arc::default).clone()
}

pub(crate) fn empty_constraint_scope() -> Arc<HashMap<String, ParsedType>> {
    static EMPTY: std::sync::OnceLock<Arc<HashMap<String, ParsedType>>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(Arc::default).clone()
}

fn shared_scope<T>(
    scope: HashMap<String, T>,
    empty: fn() -> Arc<HashMap<String, T>>,
) -> Arc<HashMap<String, T>> {
    if scope.is_empty() { empty() } else { Arc::new(scope) }
}

impl CheckerContext {
    /// Opens a frame for a block body checked against an unannotated contextual
    /// return type. Returns whether one was opened, for the caller to pair with
    /// [`Self::close_contextual_return_frame`].
    pub(crate) fn open_contextual_return_frame(&mut self) {
        let active = std::mem::take(&mut self.next_body_frame_active);
        self.contextual_return_frames.push(ContextualReturnFrame {
            active,
            ..ContextualReturnFrame::default()
        });
    }

    /// Marks the next body opened as the one checked against an unannotated
    /// contextual return type.
    pub(crate) fn activate_next_body_frame(&mut self) {
        self.next_body_frame_active = true;
    }

    /// Whether the body currently being checked owns an active frame.
    /// Marks the innermost body as an unannotated declaration's (see
    /// [`ContextualReturnFrame::unannotated_declaration`]).
    pub(crate) fn mark_unannotated_declaration_body(&mut self) {
        if let Some(frame) = self.contextual_return_frames.last_mut() {
            frame.unannotated_declaration = true;
        }
    }

    pub(crate) fn mark_self_recursive_declaration_body(&mut self, name: std::sync::Arc<str>) {
        if let Some(frame) = self.contextual_return_frames.last_mut() {
            frame.self_call_name = Some(name);
        }
    }

    pub(crate) fn body_returned_self_call(&self) -> bool {
        self.contextual_return_frames
            .last()
            .is_some_and(|frame| frame.returned_self_call)
    }

    pub(crate) fn in_unannotated_declaration_body(&self) -> bool {
        self.contextual_return_frames
            .last()
            .is_some_and(|frame| frame.unannotated_declaration)
    }

    pub(crate) fn in_contextual_return_body(&self) -> bool {
        self.contextual_return_frames
            .last()
            .is_some_and(|frame| frame.active)
    }

    /// Returns whether some return in the closed body yielded a
    /// `void`/`undefined`-including type.
    pub(crate) fn close_contextual_return_frame(&mut self) -> bool {
        let Some(frame) = self.contextual_return_frames.pop() else {
            return false;
        };
        let returned_void_like =
            frame.returned_void_like || (frame.returned_value && !frame.returned_defined);
        if !frame.active || !frame.saw_any_return {
            return returned_void_like;
        }
        // Descending, so earlier indices stay valid as later ones are removed.
        for index in frame.diagnostic_indices.iter().rev() {
            self.remove_diagnostic_at(*index);
        }
        returned_void_like
    }

    /// Records the type a `return <expr>` produced, for the `noImplicitReturns`
    /// decision. Unlike the mismatch bookkeeping this is not gated on the frame
    /// being the contextually-checked one — every body needs its own answer.
    pub(crate) fn note_contextual_return_type(
        &mut self,
        ty: &surge_ts_types::Type,
        expression: Option<&surge_ts_syntax::ParsedExpression>,
        symbols: &crate::symbols::SymbolTable,
    ) {
        fn only_undefined(ty: &surge_ts_types::Type) -> bool {
            match ty {
                surge_ts_types::Type::Undefined => true,
                surge_ts_types::Type::Union(union) => union.types().iter().all(only_undefined),
                surge_ts_types::Type::Reference(reference) => only_undefined(&reference.resolve()),
                _ => false,
            }
        }
        fn admits_undefined(ty: &surge_ts_types::Type) -> bool {
            match ty {
                surge_ts_types::Type::Void => true,
                // tsc asks this question only of a *non-void, non-`any`*
                // function, so `return <any>` suppresses TS7030 where
                // `return <unknown>` does not. `Type::Unknown` is surge's
                // "could not model" sentinel, not the `unknown` keyword
                // (`GenuineUnknown`), and letting it decide turns an unresolved
                // type into a diagnostic — the cascade the sentinel exists to
                // avoid.
                surge_ts_types::Type::Any | surge_ts_types::Type::Unknown => true,
                surge_ts_types::Type::Union(union) => union.types().iter().any(admits_undefined),
                surge_ts_types::Type::Reference(reference) => {
                    admits_undefined(&reference.resolve())
                }
                _ => false,
            }
        }
        // An async body returns the awaited value of what it `return`s: tsc's
        // `getReturnTypeFromBody` awaits each return expression, and
        // `isUnwrappedReturnTypeUndefinedVoidOrAny` tests that awaited type.
        let awaited;
        let ty = if self.in_async_body {
            awaited = crate::checks::call::awaited_type(ty);
            &awaited
        } else {
            ty
        };
        let strict_null_checks = self.options.strict_null_checks;
        let in_async_body = self.in_async_body;
        if let Some(frame) = self.contextual_return_frames.last_mut() {
            if let Some(name) = frame.self_call_name.as_deref()
                && let Some(expression) = expression
                && is_direct_self_call(expression, name, in_async_body, symbols)
            {
                frame.returned_self_call = true;
                return;
            }
            if admits_undefined(ty) {
                frame.returned_void_like = true;
            }
            frame.returned_value = true;
            // Without `strictNullChecks` a lone `null` widens to `any` too.
            let nullish_only = only_undefined(ty)
                || (!strict_null_checks && matches!(ty, surge_ts_types::Type::Null));
            if !nullish_only {
                frame.returned_defined = true;
            }
            // Collected for every frame, not just the contextually-checked
            // one: an unannotated block body infers its return type from these
            // (`getReturnTypeFromBody`), and that body has no expected type by
            // definition, so gating on `active` left it with the sentinel.
            let form = frame.unannotated_declaration.then(|| match expression {
                Some(expression) => ReturnedForm {
                    widened: crate::checks::var::widen_implicit_variable_initializer_type(
                        crate::symbols::SymbolKind::Const,
                        expression,
                        ty,
                        false,
                    ),
                    shape: crate::checks::expr::literal_shape(expression),
                    fresh: returns_fresh_literal(expression, symbols),
                },
                None => ReturnedForm {
                    widened: ty.clone(),
                    shape: surge_ts_types::LiteralShape::Regular,
                    fresh: false,
                },
            });
            match frame.returned_types.iter().position(|existing| existing == ty) {
                Some(index) => {
                    if let (Some(form), Some(recorded)) = (form, frame.returned_forms.get_mut(index)) {
                        recorded.fresh |= form.fresh;
                    }
                }
                None if frame.returned_types.len() < 16 => {
                    frame
                        .returned_types
                        .push(surge_ts_types::with_type_copy_reason(
                            surge_ts_types::TypeCopyReason::ReturnChecking,
                            || ty.clone(),
                        ));
                    if let Some(form) = form {
                        frame.returned_forms.push(form);
                    }
                }
                None => {}
            }
        }

        // An assertion yields its regular type, and so does a parameter read: its
        // type comes from an annotation or a context, and a type parameter a
        // call instantiated it with was written or inferred there, not here.
        fn returns_fresh_literal(
            expression: &surge_ts_syntax::ParsedExpression,
            symbols: &crate::symbols::SymbolTable,
        ) -> bool {
            use surge_ts_syntax::ParsedExpression;
            match expression {
                ParsedExpression::ConstAssertion { .. } | ParsedExpression::TypeAssertion { .. } => false,
                ParsedExpression::NonNullAssertion { expression, .. }
                | ParsedExpression::SatisfiesExpression { expression, .. }
                | ParsedExpression::Await { operand: expression, .. } => {
                    returns_fresh_literal(expression, symbols)
                }
                ParsedExpression::Identifier { name, .. } => !symbols
                    .get(name)
                    .is_some_and(|symbol| symbol.kind == crate::symbols::SymbolKind::Parameter),
                ParsedExpression::Conditional {
                    when_true,
                    when_false,
                    ..
                } => returns_fresh_literal(when_true, symbols) || returns_fresh_literal(when_false, symbols),
                // A call's literal result is fresh only as the inference of a
                // fresh literal argument (`id("x")`); a written type argument or
                // a declared literal return (`lit(): "a"`) is a regular type,
                // which `getWidenedLiteralLikeTypeForContextualReturnTypeIfNeeded`
                // keeps.
                ParsedExpression::Call { type_arguments, arguments, .. }
                | ParsedExpression::PropertyCall { type_arguments, arguments, .. }
                | ParsedExpression::OptionalCall { type_arguments, arguments, .. }
                | ParsedExpression::OptionalPropertyCall { type_arguments, arguments, .. }
                | ParsedExpression::ExpressionCall { type_arguments, arguments, .. } => {
                    type_arguments.is_empty()
                        && arguments.iter().any(|argument| {
                            matches!(
                                argument.expression,
                                ParsedExpression::StringLiteral(_)
                                    | ParsedExpression::NumberLiteral(_)
                                    | ParsedExpression::BooleanLiteral(_)
                                    | ParsedExpression::TemplateLiteral { .. }
                                    | ParsedExpression::ObjectLiteral { .. }
                                    | ParsedExpression::ArrayLiteral { .. }
                            )
                        })
                }
                _ => true,
            }
        }
    }

    /// Beside [`Self::body_return_types`], in an unannotated declaration's
    /// body: what each return contributes to the inferred return type.
    pub(crate) fn body_return_forms(&self) -> &[ReturnedForm] {
        self.contextual_return_frames
            .last()
            .map_or(&[], |frame| frame.returned_forms.as_slice())
    }

    /// The types the body currently being checked returned, for inferring an
    /// unannotated block body's return type. Valid until the frame is closed.
    pub(crate) fn body_return_types(&self) -> &[surge_ts_types::Type] {
        self.contextual_return_frames
            .last()
            .map_or(&[], |frame| frame.returned_types.as_slice())
    }

    /// Takes the return-mismatch verdicts an active frame recorded, removing the
    /// leaf diagnostics and handing back the types the body actually returned.
    /// tsc reports one whole-signature mismatch on the assignment instead, so the
    /// caller renders that from the join of these.
    pub(crate) fn take_contextual_return_mismatch(&mut self) -> Option<Vec<surge_ts_types::Type>> {
        let frame = self.contextual_return_frames.last_mut()?;
        if !frame.active || frame.saw_any_return || frame.diagnostic_indices.is_empty() {
            return None;
        }
        let indices = std::mem::take(&mut frame.diagnostic_indices);
        let returned_types = std::mem::take(&mut frame.returned_types);
        // Descending, so earlier indices stay valid as later ones are removed.
        for index in indices.iter().rev() {
            self.remove_diagnostic_at(*index);
        }
        Some(returned_types)
    }

    pub(crate) fn note_contextual_return_is_any(&mut self) {
        if let Some(frame) = self.contextual_return_frames.last_mut()
            && frame.active
        {
            frame.saw_any_return = true;
        }
    }

    /// Records `index` as a return-mismatch verdict of the innermost frame.
    fn note_contextual_return_mismatch(&mut self, index: usize) {
        if let Some(frame) = self.contextual_return_frames.last_mut()
            && frame.active
        {
            frame.diagnostic_indices.push(index);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CheckerContext {
    pub(crate) file_name: String,
    /// Lazily built `Arc` copy of `file_name`, invalidated by `set_file_name`.
    /// Declaration headers collected for one file share this single allocation
    /// instead of owning a path copy per header.
    file_name_arc: Option<Arc<str>>,
    /// Lazily built canonical form of `file_name` (raw, canonical), revalidated
    /// against the raw name on every call for the same reason as
    /// [`Self::file_name_arc`]. Lazy annotation references key on the canonical
    /// path per creation; this collapses the repeated canonicalize-cache probes.
    canonical_file_name_memo: Option<(Arc<str>, Arc<str>)>,
    pub(crate) current_file_kind: FileKind,
    pub(crate) options: Arc<CheckerOptions>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// The argument whose TS2345 the call being checked has already stopped
    /// reporting at: tsc names only the first inapplicable argument of a call.
    /// Scoped to that argument's own span, so a call nested inside it still
    /// reports its own first mismatch.
    pub(crate) suppressed_argument_mismatch_span: Option<DiagnosticTextSpan>,
    /// Spans the grammar pass has already answered for, with the codes tsc
    /// does not also report there: a renamed binding in a bodyless signature
    /// (TS2842) is not an implicit `any` (TS7031).
    pub(crate) grammar_answered_spans: Vec<(DiagnosticTextSpan, &'static [u32])>,
    /// The file's unreachable statement runs, as the binder marks them (the
    /// TS7027 findings, reported or not).
    pub(crate) unreachable_statement_runs: Vec<SyntaxTextSpan>,
    /// Grammar findings held until the file's type declarations are installed
    /// (`emit_deferred_grammar_diagnostics`).
    pub(crate) deferred_grammar_findings: Vec<surge_ts_syntax::ParsedGrammarDiagnostic>,
    /// Parameters already bound while a signature's annotations are being
    /// mapped, so a later annotation's `typeof <earlier parameter>` resolves the
    /// way tsc's parameter scope does. Empty outside signature mapping.
    pub(crate) signature_parameter_bindings: Vec<(String, Type)>,
    // Dedup index for `push`, mirroring the keys of `diagnostics`. `push` rejected
    // duplicates by scanning the whole `diagnostics` vec (re-rendering every code
    // to a `String` per comparison), so a context that emits D diagnostics was
    // O(D^2) — e.g. a single file with thousands of unresolved-name reports. The
    // set makes the check O(1); `diagnostic_keys_len` lets `push` detect when
    // `diagnostics` was mutated directly (clear/take) and rebuild lazily.
    //
    // The shrinking paths on this type release their keys instead of relying on
    // that rebuild (see `release_dedup_keys_for_tail`): they are speculative
    // probes that run per generic call, and rebuilding per probe reinstated the
    // same O(D^2) the index exists to remove.
    diagnostic_keys: HashSet<
        (
            String,
            String,
            String,
            Option<surge_ts_diagnostics::TextSpan>,
        ),
        surge_ts_types::fx::FxBuildHasher,
    >,
    diagnostic_keys_len: usize,
    pub(crate) stats: CompatibilityStats,
    /// File-region overlay of `push_utility_diagnostic_once` keys: entries
    /// recorded since the last `begin_file_check`. Keys inherited from before
    /// the check phase live in the shared baseline below, so the per-file
    /// reset is an O(overlay) clear instead of a full-set clone.
    pub(crate) utility_diagnostic_keys:
        HashSet<UtilityDiagnosticKey, surge_ts_types::fx::FxBuildHasher>,
    /// The utility-key set this worker context entered the check phase with,
    /// captured (not cloned) on the first `begin_file_check`. Suppression
    /// consults baseline then overlay, which is exactly the pre-region
    /// semantics where both lived in one set. See [`Self::begin_file_check`].
    utility_diagnostic_keys_baseline:
        Option<Arc<HashSet<UtilityDiagnosticKey, surge_ts_types::fx::FxBuildHasher>>>,
    pub(crate) symbols: SymbolTable,
    pub(crate) type_declarations: TypeDeclarationTable,
    pub(crate) type_declaration_scope: Option<Arc<TypeDeclarationScope>>,
    pub(crate) program_type_store: Arc<ProgramTypeStore>,
    pub(crate) substitution_store: Arc<SubstitutionStore>,
    pub(crate) declaration_environment_store: Arc<DeclarationEnvironmentStore>,
    declaration_environment_generation: u64,
    /// See [`EnvironmentMapIdentity`]: content-stable identity of the current
    /// `resolved_named_types` instance, updated at every replacement site.
    pub(crate) resolved_named_types_identity: EnvironmentMapIdentity,
    /// Deterministic count of main-thread resolution stages entered
    /// ([`Self::begin_resolution_stage`]); identical in serial and parallel
    /// modes because stage transitions happen on the linear driver path.
    pub(crate) resolution_stage_counter: u64,
    /// 0 for first-attempt work, 1 while re-running a file/module in the STC
    /// commit walk, so recheck environments never collide with the discarded
    /// speculative attempt's.
    pub(crate) environment_attempt: u64,
    /// See `DeclarationEnvironmentKey::visit`: bumped on every file switch,
    /// reset at every memo-map replacement and stage boundary.
    environment_visit_counter: u64,
    pub(crate) resolved_named_types:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, DeclarationResolutionState>>>,
    /// Program-scoped cache for context-free *generic* library/dependency
    /// instantiations, keyed by declaration and the resolved type arguments. The
    /// real `lib*.d.ts` typed-array/iterator cluster (`Uint8Array`,
    /// `ArrayIterator`, `IteratorObject`, …) is mutually recursive and generic, so
    /// every signature mentioning it would otherwise re-expand the entire tree.
    /// Each bucket is a small list of `(resolved args, resolution)` checked by
    /// structural `Type` equality, so a fingerprint collision can never return a
    /// wrong type. Only top-level (`resolving` empty) instantiations are stored, so
    /// the cached value matches a standalone resolution. Never reset; shared via
    /// `Arc` across all `CheckerContext` clones and jobs.
    pub(crate) program_resolved_generic_types:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, Vec<GenericInstantiationCacheEntry>>>>,
    /// Program-wide instantiation interner backing lazy/nominal `Type::Reference`
    /// resolution. Maps a declaration + resolved type arguments to the shared
    /// structural expansion, so a reference resolves (and the body expands) at
    /// most once per unique instantiation rather than at every use site. Shared
    /// via `Arc` across all `CheckerContext` clones and jobs.
    pub(crate) program_instantiations:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, Vec<InstantiationCacheEntry>>>>,
    /// Completed, clean physical-default-lib interface expansions keyed without
    /// structural `Type` equality. Unlike `program_instantiations`, this index is
    /// eligible inside an enclosing generic scope when every actual argument has
    /// a stable nominal/literal identity.
    pub(crate) physical_interface_instantiations:
        Arc<Mutex<FxHashMap<InterfaceInstantiationKey, Arc<Type>>>>,
    pub(crate) physical_interface_declaration_templates:
        Arc<Mutex<FxHashMap<StableInterfaceDeclarationId, Arc<InterfaceDeclarationTemplate>>>>,
    pub(crate) physical_interface_method_instantiations:
        Arc<Mutex<FxHashMap<InterfaceMemberInstantiationKey, FunctionType>>>,
    pub(crate) physical_interface_overload_instantiations:
        Arc<Mutex<FxHashMap<InterfaceOverloadInstantiationKey, FunctionType>>>,
    /// Interned capture-site content for deferred interface member
    /// annotations (`SURGE_LAZY_IFACE_MEMBERS`). Pure content keyed by
    /// declaration identity and substitution fingerprint, so it is shared
    /// across environments the way the parsed AST is.
    pub(crate) lazy_member_annotation_templates: Arc<Mutex<LazyMemberTemplateTable>>,
    pub(crate) ambient_modules: Arc<FxHashMap<String, ModuleExportTable>>,
    /// Namespace value types behind `typeof import("spec")`, keyed by the
    /// querying file and the specifier; registered when that file's imports
    /// are bound, read when the query resolves under that file's name.
    pub(crate) import_type_namespaces: Arc<Mutex<FxHashMap<(Arc<str>, String), ImportTypeTarget>>>,
    /// Ambient globals declared through `typeof import("spec")…`: they are
    /// derived from a module's export, so that module's own declaration of the
    /// same name must bind ahead of the global (see `collect_exportable_value_symbols_from_statement`).
    pub(crate) import_type_globals: Arc<Mutex<FxHashSet<String>>>,
    /// Per-file resolution scopes for files whose declarations live in ambient
    /// `declare module "…"` blocks: the blocks' own type declarations plus
    /// their block-internal import bindings. Merged into `module_scope_by_file`
    /// so the layered lookup's per-file fallback can see block imports
    /// (`import { Socket } from "node:net"` inside `declare module "http"`),
    /// which no other scope carries.
    pub(crate) ambient_file_type_scopes: Arc<FxHashMap<Arc<str>, Arc<TypeDeclarationScope>>>,
    /// Module augmentations (`declare module "x"` in a file that is itself a
    /// module). Unlike ambient module declarations, these only merge into an
    /// already-resolved target; they do not make `"x"` resolvable on their own.
    pub(crate) module_augmentations: Arc<FxHashMap<String, ModuleExportTable>>,
    pub(crate) ambient_global_symbols: SymbolTable,
    /// Names an in-program module declares as a UMD global (`export as namespace
    /// X`), with the index of that module's file. Referencing one as a value
    /// from a module file is TS2686; from a script file it is legal; its types
    /// are reachable from every file. Program-lifetime, written once before
    /// checking.
    pub(crate) umd_global_names: Arc<FxHashMap<Arc<str>, usize>>,
    /// Every namespace in the program, by scope. See
    /// [`crate::program::NamespaceRegistry`].
    pub(crate) namespace_registry: Arc<crate::program::NamespaceRegistry>,
    /// Global `let`/`const` names: bindings in the global scope that are not
    /// properties of the global object.
    pub(crate) block_scoped_globals: Arc<FxHashSet<Arc<str>>>,
    /// Names only a global augmentation declares. See
    /// [`crate::program::global_augmentation_only_names`].
    pub(crate) global_augmentation_only_names: Arc<FxHashSet<Arc<str>>>,
    /// The subset of [`Self::umd_global_names`] the file under check actually
    /// reaches through the global scope: the file is a module and nothing local
    /// or imported shadows the name. Empty for script files, for files that
    /// bind every UMD name themselves, and under `allowUmdGlobalAccess`.
    pub(crate) file_umd_global_names: FxHashSet<Arc<str>>,
    /// Set only while collecting a *script* file's top-level type declarations.
    /// A script's `interface X` re-opens a same-named global interface
    /// (declaration merging); a module's shadows it. Without the merge a script
    /// that re-opens a DOM interface silently lost the lib's members — and, in
    /// the other direction, the lib's shape replaced the file's own.
    pub(crate) merge_script_interfaces_with_globals: bool,
    /// Local names the file under check binds through a type-only import
    /// (`import type React from "react"`). Referencing one as a value is
    /// TS1361 — including the implicit factory reference every JSX tag makes
    /// under `jsx: react`.
    pub(crate) file_type_only_import_names: FxHashSet<Arc<str>>,
    /// Local names the file under check imports through an export whose alias
    /// chain passes through a type-only declaration (`export type { A }`
    /// upstream of a plain `import { A }`), with that declaration's kind.
    /// Owned like `file_type_only_import_names`.
    pub(crate) file_type_only_alias_names: FxHashMap<Arc<str>, crate::modules::TypeOnlyAliasKind>,
    /// Every name the current file's imports bind, for TS2632. Owned by the
    /// file that set it, like `file_type_only_import_names`.
    pub(crate) file_import_names: FxHashSet<Arc<str>>,
    /// The subset of `file_import_names` bound by `import * as`.
    pub(crate) file_namespace_import_names: FxHashSet<Arc<str>>,
    /// The subset of `file_namespace_import_names` whose module's whole export
    /// list surge saw, so a member missing from it is the source's error.
    pub(crate) file_complete_namespace_import_names: FxHashSet<Arc<str>>,
    /// The `import x = require` bindings of the file under check that have no
    /// namespace meaning (`export =` of a variable, class, ...).
    pub(crate) file_value_only_import_alias_names: FxHashSet<Arc<str>>,
    /// Top-level classes the file under check binds through `const X = class
    /// {}`. Per-file: cleared by `begin_file_check`.
    pub(crate) file_const_class_names: FxHashSet<Arc<str>>,
    pub(crate) file_type_only_import_names_owner: Option<String>,
    /// Set while an expression is checked where a type-only import is a valid
    /// use (a computed name of an `abstract` or `declare` member, or in an
    /// ambient class): tsc's `isValidTypeOnlyAliasUseSite`.
    pub(crate) type_only_alias_use_valid: bool,
    /// The value bindings the file's imports made, by name: a read that
    /// resolves to anything else is a local shadowing the import.
    pub(crate) file_type_only_import_handles: FxHashMap<Arc<str>, crate::symbols::SymbolInfoHandle>,
    /// Function names the authoritative check pass has already registered in the
    /// file under check. The first registration *replaces* the signature the
    /// collection pre-pass installed (the check pass resolves it under the right
    /// scope); every later one is an overload of the same name and must merge
    /// into it, or only the last declaration would survive.
    pub(crate) checked_function_declaration_names: FxHashSet<Arc<str>>,
    /// The file [`Self::file_umd_global_names`] was computed for. Type
    /// resolution re-enters under a *declaring* file's name, and that file's
    /// shadowing is not the checked file's, so the set only applies while the
    /// two agree.
    pub(crate) file_umd_global_names_owner: Option<String>,
    pub(crate) ambient_global_type_declarations: Arc<TypeDeclarationTable>,
    pub(crate) module_file_index_by_identity: Arc<FxHashMap<Arc<str>, usize>>,
    /// Each module's resolution scope keyed by its source `file_name`, mirroring
    /// `shared_state.module_resolution_scopes` but addressable by name. A type
    /// alias/interface imported across a module *import cycle* can lose its
    /// pre-attached `resolution_scope` when the multi-pass binding fixpoint rebinds
    /// it (the source module's scope is not yet available in that pass), leaving it
    /// `None`. Resolving such a declaration's body must still happen in its
    /// declaring module's scope, so resolution falls back to this map keyed by the
    /// declaration's `file_name`. Refreshed before each module-analysis round with
    /// the freshest resolution scopes (signature collection resolves parameter
    /// types through local aliases whose attached scope lacks import layers), then
    /// set a final time before the check phase.
    pub(crate) module_scope_by_file: Arc<FxHashMap<Arc<str>, Arc<TypeDeclarationScope>>>,
    /// Each module's local value symbols, keyed by `file_name`. The value analogue
    /// of [`Self::module_scope_by_file`]: when an imported type alias's body is
    /// resolved while checking a *consumer* file, a `typeof <localValue>` in that
    /// body must resolve against the *declaring* module's values, not the
    /// consumer's `symbols`. Populated once (read-only) before the check phase and
    /// shared across jobs. Consulted via `get` only.
    pub(crate) module_local_values_by_file: Arc<FxHashMap<Arc<str>, Arc<SymbolTable>>>,
    /// While set (on the value-collection shadow context of a library
    /// declaration file), annotated initializer-less variable declarations get
    /// a lazy annotation reference instead of an eagerly mapped type. See
    /// `infer::types::cache::make_lazy_value_annotation_reference`.
    pub(crate) lazy_library_value_annotations: bool,
    /// Set on the export-collection shadow context: an initializer's arrow whose
    /// return type is written has a signature fixed by its annotations, and the
    /// shadow discards diagnostics, so checking its body there is pure cost.
    pub(crate) skip_annotated_function_bodies: bool,
    /// While set, exportable-value collection runs THIN (`Unknown` types, no
    /// annotation/initializer resolution). Set only around the superseded
    /// analysis rounds' collection calls; see `modules::exports::values::
    /// thin_prelim_enabled` for the soundness argument.
    pub(crate) thin_superseded_value_collection: bool,
    /// The modules JSX namespaces are read from that no lexical scope exposes
    /// (each file's implicit `jsx-runtime` import, each UMD global), located
    /// once after module binding.
    pub(crate) jsx_namespace_modules: Arc<crate::checks::jsx::JsxNamespaceModules>,
    /// The current file's JSX pragmas ([`surge_ts_syntax::ParsedSource::jsx_factory_uses`]).
    /// Per-file: reset by [`Self::begin_file_check`].
    pub(crate) jsx_factory_uses: surge_ts_syntax::JsxFactoryUses,
    /// The JSX namespace resolved for the current file (tsc's
    /// `getJsxNamespaceAt`), keyed by the file it was resolved for. Per-file:
    /// reset by [`Self::begin_file_check`].
    pub(crate) jsx_namespace: Option<(Arc<str>, crate::checks::jsx::JsxNamespace)>,
    /// tsc's per-file `jsxFragmentType`: the fragment factory is resolved,
    /// and a failure reported, at the file's first fragment only. Per-file:
    /// reset by [`Self::begin_file_check`].
    pub(crate) jsx_fragment_factory_resolved: bool,
    /// Each scope is shared, never edited in place: a declaration environment
    /// captures the stack by cloning the `Arc`s, where a deep copy of every map
    /// (and of every constraint's parsed type) cost ~1 KB per environment.
    pub(crate) type_parameter_scopes: Vec<Arc<HashMap<String, Type>>>,
    /// The current file's definite-assignment targets
    /// ([`surge_ts_syntax::ParsedSource::definite_writes`]).
    pub(crate) definite_writes: Arc<HashSet<String>>,
    /// Never-initialized `let` bindings of enclosing flow containers that a
    /// nested container reading them reports as TS2454 (tsc's
    /// `isNeverInitialized`); see `flow::never_initialized`.
    pub(crate) inherited_never_initialized: Vec<Arc<str>>,
    /// The bindings the function body being checked assigns anywhere, which
    /// tsc's `isConstantReference` reads: an element key named by one is not a
    /// reference its guards narrow.
    pub(crate) container_assigned_bindings: Arc<std::collections::HashSet<Arc<str>>>,
    /// The expando writes of the container being checked, by the annotated
    /// declaration whose function they declare members on (see
    /// [`crate::checks::var::install_expando_initializer_members`]).
    pub(crate) expando_initializer_members:
        Arc<HashMap<usize, Vec<crate::checks::var::ExpandoInitializerMember>>>,
    /// The never-initialized bindings typed by a type parameter whose
    /// constraint carries `undefined`: a read of one where tsc substitutes the
    /// constraint is not reported (see `FunctionFlowState::constraint_exempt`).
    pub(crate) never_initialized_constraint_exempt: HashSet<Arc<str>>,
    // Parallel to `type_parameter_scopes`: the declared constraint (if any) for
    // each in-scope type parameter, used to recognize `K extends keyof T` so a
    // generic `T[K]` is not falsely reported as an invalid index (TS2536).
    pub(crate) type_parameter_constraint_scopes: Vec<Arc<HashMap<String, ParsedType>>>,
    pub(crate) timings: Option<std::sync::Arc<std::sync::Mutex<ProgramTimings>>>,
    /// Nonzero while resolving the body of a namespace-qualified type member
    /// (e.g. `React.ComponentProps`). Unresolved names encountered here are
    /// internal references into a namespace surface we only partially model, so
    /// they resolve to `unknown` without a TS2304 cascade — tsc resolves them
    /// against the full `@types/*`/generated namespace and reports nothing.
    pub(crate) namespace_member_resolution_depth: usize,
    /// Generic instantiations performed under the *current root* resolution
    /// (reset whenever the declaration-resolution stack empties). The
    /// `resolving`-stack guards bound how deep a recursion goes and how many
    /// frames of one declaration may nest, but both are path-local: a
    /// type-level program that fans out — many distinct argument tuples across
    /// several levels, none repeating and none deep — does unbounded *total*
    /// work without tripping either. This is the breadth ceiling.
    pub(crate) instantiation_work: usize,
    /// Nested *generic* declaration instantiations currently being resolved.
    /// tsc keeps the same counter on the checker and yields the error type at
    /// 100 (`instantiateTypeWithAlias`); this is the single ceiling every
    /// instantiation passes, whatever declaration kind it goes through.
    pub(crate) instantiation_depth: usize,
    /// Nonzero while checking the attributes/children of a JSX element whose
    /// component props type could not be modelled (the `unknown` sentinel).
    /// Without a props type there is no contextual type to hand an inline
    /// callback, so an implicit-any report there says nothing about the source —
    /// it only reflects surge's own modelling gap. Same no-cascade rule the
    /// property/index checks apply to a sentinel receiver.
    pub(crate) unmodelled_jsx_props_depth: usize,
    /// Nonzero while checking a value against an expected type surge degraded to
    /// the `unknown` sentinel (the self-recursive generic interface cycle, most
    /// of zod's builder surface). The written value has a contextual type in
    /// tsc; surge just lost it, so an implicit-any report inside it describes
    /// surge's modelling gap rather than the source. Same rule as
    /// [`Self::unmodelled_jsx_props_depth`].
    pub(crate) degraded_expected_type_depth: usize,
    /// Names bound in this file whose `any` is the *source's*, reached through a
    /// binding rather than named directly: `const q = trpc.post.all.useQuery()`
    /// where `trpc` is an import whose module was reported unresolved. tsc types
    /// the whole chain as its error type, so a callback passed to a call on `q`
    /// really has no contextual type. Without this the provenance stopped at the
    /// import and every downstream call read as a chain surge merely failed to
    /// model. Per-file: cleared by `begin_file_check`.
    pub(crate) genuine_any_bindings: HashSet<String>,
    /// `var`s whose first declaration is unannotated and inferred `any`, which
    /// may be surge's inference failing rather than the source's type — a later
    /// declaration is not compared with it (TS2403). Per-file: cleared by
    /// `begin_file_check`.
    pub(crate) inferred_any_vars: HashSet<String>,
    /// `var`s whose first declaration is `= this` in an instance member: their
    /// type is the polymorphic `this`, identical to no class type.
    pub(crate) this_initialized_vars: HashSet<String>,
    /// Nonzero while checking the value of a shorthand object-literal property
    /// (`{ value }`). An unresolved name there is TS18004 to tsc — the property
    /// has no initializer to fall back on — rather than a plain missing name.
    /// Whether a `this` reached here would have no annotated type: the body
    /// being checked is a plain `function` with no `this` parameter. An arrow
    /// inherits it (that is what makes `function f() { return () => this }`
    /// report), while a class method or an object-literal method clears it.
    pub(crate) this_is_implicitly_any: bool,
    /// Inside a `function` expression whose contextual signature is known and
    /// declares no `this` (TS2683 on each `this` read under `noImplicitThis`).
    pub(crate) contextual_this_missing: bool,
    /// The current JavaScript file's module-level variables initialized with
    /// a function or an empty object literal, which a member write declares
    /// members on (`IsExpandoInitializer`).
    pub(crate) javascript_expando_objects: Arc<HashSet<String>>,
    /// The member assignment about to be checked is a module-level statement,
    /// in the container of the module's own declarations. Taken by that check.
    pub(crate) module_level_member_write: bool,
    /// The members the constructor being checked may initialize even when they
    /// are `readonly`: its class's own instance properties and parameter
    /// properties. `None` everywhere else — tsc allows the write only when the
    /// control-flow container is that constructor, so a nested function, an
    /// arrow included, clears it.
    pub(crate) constructor_writable_members: Option<Vec<String>>,
    /// The base class's constructor value while a derived constructor is
    /// checked: what a `super(…)` call resolves its arguments against. The
    /// `super` symbol itself is the base *instance*, for `super.member`.
    pub(crate) super_constructor_type: Option<Type>,
    /// The `extends` reference of the class whose constructor is checked.
    pub(crate) super_class_reference: Option<surge_ts_syntax::ParsedNamedType>,
    /// Set while a file's function and class signatures are collected, which
    /// runs before its `const`s are bound. A class base that names no type may
    /// be one of those values (`class D extends Mixed`), so the miss is not
    /// reported from here; the class's own check reports a base that names
    /// nothing at all.
    pub(crate) collecting_signatures: bool,
    /// Whether the declaration whose heritage is being resolved is a class
    /// instance. See [`Self::collecting_signatures`].
    pub(crate) resolving_class_heritage: bool,
    /// The union an object literal is being checked against, while it is
    /// evaluated against the one member picked for it. The literal's evaluator
    /// takes it: a missing property is then the union's assignability failure,
    /// not the member's TS2741.
    pub(crate) union_literal_target: Option<Type>,
    pub(crate) shorthand_property_depth: usize,
    /// How deep the per-property union-member probe is nested. It types a
    /// literal's properties against a candidate union, and a nested literal
    /// probes again, so the depth is what keeps that from multiplying.
    pub(crate) union_member_probe_depth: usize,
    /// One frame per block-bodied function being checked against a CONTEXTUAL
    /// return type it did not annotate. See
    /// `ContextualReturnFrame`.
    pub(crate) contextual_return_frames: Vec<ContextualReturnFrame>,
    /// Set while a return statement's value is being checked against that
    /// contextual type, so only the mismatch verdicts get recorded — every other
    /// diagnostic raised inside the expression is unrelated and must survive.
    pub(crate) in_contextual_return_check: bool,
    /// Set while a contextually typed function's returned value is related to
    /// that contextual return type. tsc relates the function's inferred return,
    /// which `getReturnTypeFromBody` widens out of freshness, so an excess
    /// property in a returned literal is no error.
    pub(crate) relating_unfresh_return: bool,
    /// The body being checked belongs to an `async` function, whose returns
    /// relate awaited value to awaited return type (tsc's `unwrapReturnType`).
    pub(crate) in_async_body: bool,
    /// The body being checked belongs to a generator, whose returns relate to
    /// its declared type's return type argument (tsc's `unwrapReturnType`).
    pub(crate) in_generator_body: bool,
    /// Where the `yield`s of the unannotated generator declaration being
    /// checked start whose value is used with no contextual type, each an
    /// implicit `any` under `noImplicitAny` (TS7057).
    pub(crate) implicit_any_yields: Vec<usize>,
    /// The generator being checked is unannotated under `noImplicitAny`, so a
    /// `yield` whose contextual type is `any` is TS7057 as well.
    pub(crate) implicit_any_yield_generator: bool,
    /// Name starts of the `var` declarations typed `any` for a circular
    /// initializer (TS7022), whose reads there are neither early nor unassigned.
    pub(crate) circular_var_starts: Vec<usize>,
    /// The yield type of the annotated generator whose body is being checked
    /// (`getIterationTypesOfGeneratorFunctionReturnType`), which each `yield`
    /// operand is contextually typed by and checked against.
    pub(crate) generator_yield_type: Option<Type>,
    /// The next type of the annotated generator whose body is being checked:
    /// what each of its `yield`s evaluates to.
    pub(crate) generator_next_type: Option<Type>,
    /// The body being checked is a generator's, annotated or not: a `yield`
    /// anywhere else is a grammar error whose operand tsc does not check.
    pub(crate) in_generator_function: bool,
    /// The generator being checked is an `async function*`, whose `yield*`
    /// operand may be async or sync iterable.
    pub(crate) in_async_generator: bool,
    /// The arrow about to be checked is a call argument, so a whole-signature
    /// mismatch is an argument error (TS2345). Taken by that arrow's check.
    pub(crate) next_arrow_is_argument: bool,
    /// The arrow about to be checked is only contextually typed by its
    /// expectation (the right operand of `&&`, `||`, `??`), so a return that
    /// does not fit is no error of the arrow's own. Taken by that arrow's check.
    pub(crate) next_arrow_context_only: bool,
    /// The names the file being checked references anywhere, installed only
    /// while a file with a namespace-local `import x = require()` is checked:
    /// tsc resolves (and reports) such an alias only once something names it.
    pub(crate) namespace_require_reads: Option<Arc<FxHashSet<String>>>,
    /// Set by a return-value check whose value is a conditional, consumed by
    /// that conditional: tsc checks each branch of a returned conditional
    /// against the return type on its own (`checkReturnExpression`).
    pub(crate) split_returned_conditional: bool,
    /// The element reads of the defaulted binding elements on the binding
    /// being evaluated: tsc reads each with `AccessFlagsAllowMissing`, so a
    /// tuple too short for it is not TS2493. Other reads on the same path
    /// still report.
    pub(crate) allow_missing_tuple_elements: Vec<SyntaxTextSpan>,
    /// Spans of the `default`-less switches checked in this file whose cases do
    /// not cover their discriminant, which the missing-return check needs.
    pub(crate) non_exhaustive_switches: Vec<(usize, usize)>,
    /// `default`-less switches whose cases are known to cover the discriminant.
    pub(crate) exhaustive_switches: Vec<(usize, usize)>,
    /// The subset of `exhaustive_switches` whose cases name every member of
    /// a discriminant surge fully typed, which is what may make the code
    /// after one unreachable (TS7027).
    pub(crate) proven_exhaustive_switches: Vec<(usize, usize)>,
    /// Set by the arrow path just before it checks a block body, consumed by
    /// the frame that body opens.
    pub(crate) next_body_frame_active: bool,
    /// Depth of `with_file_name` frames whose file differs from the enclosing
    /// one — nonzero exactly while a declaration from another file is being
    /// resolved. See [`Self::lookup_ignores_local_table`].
    pub(crate) cross_file_resolution_depth: u32,
    /// Stack of namespace prefixes for the member bodies currently being
    /// resolved (e.g. `"React"` while expanding `React.ChangeEventHandler`).
    /// Namespace members are stored under qualified names but reference their
    /// siblings unqualified (`EventHandler<…>` inside `React.ChangeEventHandler`),
    /// so a bare name that does not resolve is retried against these prefixes.
    pub(crate) namespace_member_prefix_stack: Vec<String>,
    /// The namespace blocks being checked, innermost last, as
    /// `driver::namespace_block_prefix` keys a block's colliding locals.
    pub(crate) namespace_block_prefix_stack: Vec<String>,
    /// The classes whose bodies enclose the code being checked, innermost last.
    /// Each entry is the class's declaration identity followed by its bases',
    /// so a `private` member is accessible when its declaring class heads an
    /// entry and a `protected` one when it appears anywhere in one.
    pub(crate) enclosing_classes: Vec<Vec<crate::checks::expr::ClassIdentity>>,
    /// The classes whose member bodies enclose the code being checked,
    /// innermost last, with the types an unresolved name is looked up on for
    /// tsc's "did you mean `this.x` / `C.x`" hint.
    pub(crate) enclosing_class_members: Vec<crate::checks::expr::EnclosingClassMembers>,
    /// The calls whose return type is `never`, keyed by the call's own span:
    /// tsc ends the flow after one (`util.assertNever(x)`), so the function's
    /// end point is not reachable through it.
    pub(crate) never_returning_calls: FxHashSet<(usize, usize)>,
    /// The class type parameters that are out of scope because a *static*
    /// member is being checked: the member's source range, the depth of the
    /// scope that binds them (a nearer scope — a static method's own
    /// parameters — shadows them), and their names.
    pub(crate) static_member_type_parameters:
        Option<(Option<surge_ts_syntax::TextSpan>, usize, Vec<String>)>,
    /// Each source file's properties whose class constructor declares locals
    /// (see [`crate::program::ConstructorLocalProperty`]), keyed by file name.
    pub(crate) constructor_local_properties:
        Arc<FxHashMap<Arc<str>, Arc<[crate::program::ConstructorLocalProperty]>>>,
    /// Each global function name implemented at the top level of a script,
    /// with the files that implement it (TS2393 in every one of them).
    pub(crate) script_function_implementations: Arc<FxHashMap<Arc<str>, Vec<Arc<str>>>>,
    /// While a property initializer is checked: the binding each of its
    /// constructor's locals resolves to *outside* the property, so a read that
    /// lands on it (rather than on a binding the initializer itself made) is
    /// the read tsc's name resolver refuses.
    pub(crate) constructor_local_outer_bindings:
        Vec<(Arc<str>, Option<crate::symbols::SymbolInfoHandle>)>,
    /// The scope enclosing a nested `function` declaration whose body is
    /// about to be checked. It already chains to the module and the ambient
    /// globals, so it replaces the usual module-over-ambient body root.
    pub(crate) nested_function_scope: Option<Arc<SymbolTable>>,
    /// The file's parenthesized expressions (see
    /// [`surge_ts_syntax::ParsedSource::parenthesized_expressions`]). Per-file:
    /// cleared by `begin_file_check`, set when the file's check starts.
    pub(crate) parenthesized_expressions: Arc<[surge_ts_syntax::ParenthesizedExpressionSpan]>,
    /// Where each `this` the current script file's top level owns starts
    /// (see [`surge_ts_syntax::ParsedSource::global_this_starts`]); empty for
    /// a module. Per-file: cleared by `begin_file_check`.
    pub(crate) global_this_starts: Arc<[u32]>,
    /// Where each `this` the current module file's top level owns starts, which
    /// `tryGetThisTypeAtEx` types as `undefined`; empty for a script. Per-file:
    /// cleared by `begin_file_check`.
    pub(crate) module_this_starts: Arc<[u32]>,
    /// The file's object-literal members whose `this` is the literal itself
    /// (see [`surge_ts_syntax::ParsedSource::literal_this_members`]).
    /// Per-file: cleared by `begin_file_check`.
    pub(crate) literal_this_members: Arc<[u32]>,
    /// The `this` an object literal hands the member whose arrow check runs
    /// next (tsc's `getContextualThisParameterType`). Taken by that check.
    pub(crate) next_arrow_this: Option<Type>,
    /// The start of the object literal being evaluated as an assignment
    /// declaration's value (`F.prototype = { … }`), which has no contextual
    /// type: its members' `this` is the literal itself.
    pub(crate) expando_object_literal: Option<u32>,
    /// The receiver of a `push`/`unshift`/`length`/`x[n] = v` about to be
    /// evaluated: an evolving array read there is typed `any[]` and is not a
    /// read tsc reports.
    pub(crate) evolving_array_operation_target: Option<SyntaxTextSpan>,
    /// Whether this file declared an evolving array at all, so every other
    /// file skips the per-identifier lookup. Per-file: cleared by
    /// `begin_file_check`.
    pub(crate) auto_arrays_declared: bool,
    /// The file's flow-typed `let` assignment summaries (see
    /// [`surge_ts_syntax::ParsedSource::let_assignments`]). Per-file.
    pub(crate) let_assignments: Arc<[surge_ts_syntax::LetAssignmentSummary]>,
    /// Nesting depth of top-level function and class declaration checks,
    /// which read module-level auto arrays by their declared type.
    pub(crate) module_declared_only_depth: u32,
    /// Nesting depth of `export` declaration checks, whose bindings other
    /// modules can reach, so tsc does not type them by this module's flow.
    pub(crate) module_export_depth: u32,
    /// Lowest `resolving`-stack index that any cycle truncation has re-entered
    /// since this field was last reset. A resolution that pushed its declaration
    /// at stack depth `floor` is independent of the enclosing `resolving` context
    /// — and therefore safe to memoize — only if every cycle it triggered
    /// re-entered a frame at `floor` or deeper (an *internal* self/mutual cycle).
    /// A cycle reaching below `floor` means the result depends on an outer frame.
    /// See the generic instantiation cache in `resolve_named_type`.
    pub(crate) lowest_cycle_target_index: usize,
    /// The subset of `lowest_cycle_target_index` whose re-entry handed back
    /// the `unknown` fallback (a generic declaration re-entered mid-resolution)
    /// instead of a lazy nominal reference that peels to the real shape later.
    pub(crate) lowest_unknown_cycle_target_index: usize,
    /// `resolving`-stack indices of the frames that cross a structural type —
    /// an interface body, or a type alias whose body is itself structural
    /// (object/array/function/…). A type-alias cycle whose re-entry path passes
    /// through one of these frames is tsc-legal recursion (`type Issue = A |
    /// InvalidUnion` where `InvalidUnion.errors: Issue[][]`), not a
    /// structureless `type A = B; type B = A` cycle. Frames are recorded by
    /// `resolve_interface`/`resolve_type_alias` for the duration of their body
    /// resolution.
    pub(crate) structural_resolution_frames: Vec<usize>,
    /// The subset of `structural_resolution_frames` opened by an object *type
    /// literal* (`Omit<{ optional(): Chainable<…> }, k>`), whose members tsc
    /// resolves lazily. A generic alias re-entered through one of these is
    /// handed a lazy self-reference instead of the sentinel; a re-entry through
    /// an interface or alias body is not (forcing those exposed incomplete
    /// shapes on tanstack-query).
    pub(crate) type_literal_member_frames: Vec<usize>,
    file_kinds: Arc<FxHashMap<String, FileKind>>,
    /// All module-scope value bindings of the file currently being checked,
    /// inferred up front. Consulted only when a bare identifier misses the
    /// positional scope, so a function body may reference a `const`/`let`/`class`
    /// declared *after* it (legal — the body runs after the module finishes).
    /// `Arc`-shared so cloning the context stays cheap.
    pub(crate) module_value_fallback: Option<Arc<SymbolTable>>,
}

impl CheckerContext {
    pub(crate) fn new(
        file_name: String,
        options: CheckerOptions,
        file_kinds: FxHashMap<String, FileKind>,
    ) -> Self {
        Self::new_with_shared_options(file_name, Arc::new(options), file_kinds)
    }

    /// Like [`Self::new`], but shares an existing options handle. The options
    /// carry the project-wide module-resolution tables
    /// (`resolved_modules_by_importer`), so per-module shadow contexts must not
    /// deep-clone them.
    pub(crate) fn new_with_shared_options(
        file_name: String,
        options: Arc<CheckerOptions>,
        file_kinds: FxHashMap<String, FileKind>,
    ) -> Self {
        let current_file_kind = file_kinds
            .get(&file_name)
            .copied()
            .unwrap_or(FileKind::RootSource);

        Self {
            file_name,
            file_name_arc: None,
            canonical_file_name_memo: None,
            current_file_kind,
            options,
            diagnostics: Vec::new(),
            suppressed_argument_mismatch_span: None,
            grammar_answered_spans: Vec::new(),
            deferred_grammar_findings: Vec::new(),
            signature_parameter_bindings: Vec::new(),
            diagnostic_keys: HashSet::default(),
            diagnostic_keys_len: 0,
            stats: CompatibilityStats::default(),
            utility_diagnostic_keys: HashSet::default(),
            utility_diagnostic_keys_baseline: None,
            symbols: SymbolTable::new(),
            type_declarations: TypeDeclarationTable::new(),
            type_declaration_scope: None,
            program_type_store: current_program_type_store().unwrap_or_else(ProgramTypeStore::new),
            substitution_store: SubstitutionStore::new(),
            declaration_environment_store: DeclarationEnvironmentStore::new(),
            declaration_environment_generation: 0,
            resolved_named_types_identity: EnvironmentMapIdentity::initial(),
            resolution_stage_counter: 0,
            environment_attempt: 0,
            environment_visit_counter: 0,
            resolved_named_types: Arc::new(Mutex::new(FxHashMap::default())),
            program_resolved_generic_types: Arc::new(Mutex::new(FxHashMap::default())),
            program_instantiations: Arc::new(Mutex::new(FxHashMap::default())),
            physical_interface_instantiations: Arc::new(Mutex::new(FxHashMap::default())),
            physical_interface_declaration_templates: Arc::new(Mutex::new(FxHashMap::default())),
            physical_interface_method_instantiations: Arc::new(Mutex::new(FxHashMap::default())),
            physical_interface_overload_instantiations: Arc::new(Mutex::new(FxHashMap::default())),
            lazy_member_annotation_templates: Arc::new(Mutex::new(FxHashMap::default())),
            ambient_modules: Arc::new(FxHashMap::default()),
            import_type_namespaces: Arc::new(Mutex::new(FxHashMap::default())),
            import_type_globals: Arc::new(Mutex::new(FxHashSet::default())),
            ambient_file_type_scopes: Arc::new(FxHashMap::default()),
            module_augmentations: Arc::new(FxHashMap::default()),
            ambient_global_symbols: SymbolTable::new(),
            umd_global_names: Arc::new(FxHashMap::default()),
            namespace_registry: Arc::default(),
            block_scoped_globals: Arc::default(),
            global_augmentation_only_names: Arc::default(),
            file_umd_global_names: FxHashSet::default(),
            file_umd_global_names_owner: None,
            merge_script_interfaces_with_globals: false,
            definite_writes: Arc::default(),
            inherited_never_initialized: Vec::new(),
            container_assigned_bindings: Arc::default(),
            never_initialized_constraint_exempt: HashSet::new(),
            file_type_only_import_names: FxHashSet::default(),
            file_type_only_alias_names: FxHashMap::default(),
            file_import_names: FxHashSet::default(),
            file_namespace_import_names: FxHashSet::default(),
            file_complete_namespace_import_names: FxHashSet::default(),
            file_value_only_import_alias_names: FxHashSet::default(),
            file_const_class_names: FxHashSet::default(),
            checked_function_declaration_names: FxHashSet::default(),
            file_type_only_import_names_owner: None,
            type_only_alias_use_valid: false,
            file_type_only_import_handles: FxHashMap::default(),
            ambient_global_type_declarations: Arc::new(TypeDeclarationTable::new()),
            module_file_index_by_identity: Arc::new(FxHashMap::default()),
            module_scope_by_file: Arc::new(FxHashMap::default()),
            module_local_values_by_file: Arc::new(FxHashMap::default()),
            thin_superseded_value_collection: false,
            lazy_library_value_annotations: false,
            skip_annotated_function_bodies: false,
            jsx_namespace_modules: Default::default(),
            jsx_factory_uses: Default::default(),
            jsx_namespace: None,
            jsx_fragment_factory_resolved: false,
            type_parameter_scopes: Vec::new(),
            type_parameter_constraint_scopes: Vec::new(),
            timings: None,
            expando_initializer_members: Arc::default(),
            namespace_member_resolution_depth: 0,
            instantiation_work: 0,
            instantiation_depth: 0,
            unmodelled_jsx_props_depth: 0,
            this_is_implicitly_any: false,
            contextual_this_missing: false,
            javascript_expando_objects: Arc::default(),
            module_level_member_write: false,
            constructor_writable_members: None,
            super_constructor_type: None,
            super_class_reference: None,
            collecting_signatures: false,
            resolving_class_heritage: false,
            union_literal_target: None,
            shorthand_property_depth: 0,
            degraded_expected_type_depth: 0,
            genuine_any_bindings: HashSet::default(),
            inferred_any_vars: HashSet::default(),
            this_initialized_vars: HashSet::default(),
            union_member_probe_depth: 0,
            contextual_return_frames: Vec::new(),
            in_contextual_return_check: false,
            relating_unfresh_return: false,
            in_async_body: false,
            in_generator_body: false,
            implicit_any_yields: Vec::new(),
            implicit_any_yield_generator: false,
            circular_var_starts: Vec::new(),
            generator_yield_type: None,
            generator_next_type: None,
            in_generator_function: false,
            in_async_generator: false,
            next_arrow_is_argument: false,
            next_arrow_context_only: false,
            namespace_require_reads: None,
            split_returned_conditional: false,
            allow_missing_tuple_elements: Vec::new(),
            non_exhaustive_switches: Vec::new(),
            exhaustive_switches: Vec::new(),
            proven_exhaustive_switches: Vec::new(),
            next_body_frame_active: false,
            cross_file_resolution_depth: 0,
            namespace_member_prefix_stack: Vec::new(),
            namespace_block_prefix_stack: Vec::new(),
            enclosing_classes: Vec::new(),
            enclosing_class_members: Vec::new(),
            never_returning_calls: FxHashSet::default(),
            static_member_type_parameters: None,
            constructor_local_properties: Arc::default(),
            script_function_implementations: Arc::default(),
            constructor_local_outer_bindings: Vec::new(),
            nested_function_scope: None,
            parenthesized_expressions: Arc::from([]),
            global_this_starts: Arc::from([]),
            module_this_starts: Arc::from([]),
            literal_this_members: Arc::from([]),
            next_arrow_this: None,
            expando_object_literal: None,
            evolving_array_operation_target: None,
            auto_arrays_declared: false,
            let_assignments: Arc::from([]),
            module_declared_only_depth: 0,
            module_export_depth: 0,
            lowest_cycle_target_index: usize::MAX,
            lowest_unknown_cycle_target_index: usize::MAX,
            structural_resolution_frames: Vec::new(),
            type_literal_member_frames: Vec::new(),
            file_kinds: Arc::new(file_kinds),
            module_value_fallback: None,
            unreachable_statement_runs: Vec::new(),
        }
    }

    /// Replaces the named-resolution memo with a fresh map and records its
    /// content-stable identity: (current file, current resolution stage,
    /// `ordinal`, current attempt). Every site that swaps the map must go
    /// through here so environment identity stays pointer-free.
    pub(crate) fn replace_resolved_named_types(&mut self, ordinal: u32) {
        self.resolved_named_types = Arc::new(Mutex::new(FxHashMap::default()));
        self.resolved_named_types_identity = EnvironmentMapIdentity {
            creator: Arc::from(self.file_name.as_str()),
            stage: self.resolution_stage_counter,
            ordinal,
            attempt: self.environment_attempt,
        };
        self.environment_visit_counter = 0;
    }

    /// Marks a main-thread resolution stage boundary: bumps the deterministic
    /// stage counter so map identities created in different stages never
    /// collide. The carried memo map itself is deliberately left in place —
    /// stage-time resolution hitting the previous stage's memo is observable
    /// serial behavior (one tRPC display depends on it), so parallel drivers
    /// must instead reproduce the serial carry (see the last-committed-module
    /// hand-off in `collect_module_analyses_with_bindings_parallel`).
    pub(crate) fn begin_resolution_stage(&mut self) {
        self.resolution_stage_counter += 1;
        self.environment_visit_counter = 0;
    }

    pub(crate) fn note_resolution_cycle(&mut self, target_index: usize) {
        self.lowest_cycle_target_index = self.lowest_cycle_target_index.min(target_index);
    }

    pub(crate) fn note_unknown_resolution_cycle(&mut self, target_index: usize) {
        self.lowest_unknown_cycle_target_index =
            self.lowest_unknown_cycle_target_index.min(target_index);
    }

    /// A per-file scratch clone that does not copy the run's accumulated
    /// diagnostics or their dedup index — the caller clears both right away,
    /// and copying then discarding them was O(diagnostics so far) per file.
    pub(crate) fn clone_without_diagnostics(&mut self) -> Self {
        let diagnostics = std::mem::take(&mut self.diagnostics);
        let diagnostic_keys = std::mem::take(&mut self.diagnostic_keys);
        let diagnostic_keys_len = std::mem::replace(&mut self.diagnostic_keys_len, 0);
        let clone = self.clone();
        self.diagnostics = diagnostics;
        self.diagnostic_keys = diagnostic_keys;
        self.diagnostic_keys_len = diagnostic_keys_len;
        clone
    }

    pub(crate) fn declaration_environment(&self) -> DeclarationEnvironmentHandle {
        self.declaration_environment_store.intern(self)
    }

    fn from_declaration_environment(
        data: &DeclarationEnvironmentData,
        declaration_environment_store: Arc<DeclarationEnvironmentStore>,
    ) -> Self {
        Self {
            file_name: data.file_name.clone(),
            file_name_arc: None,
            canonical_file_name_memo: None,
            current_file_kind: data.current_file_kind,
            options: data.options.clone(),
            diagnostics: Vec::new(),
            suppressed_argument_mismatch_span: None,
            grammar_answered_spans: Vec::new(),
            deferred_grammar_findings: Vec::new(),
            signature_parameter_bindings: Vec::new(),
            diagnostic_keys: HashSet::default(),
            diagnostic_keys_len: 0,
            stats: CompatibilityStats::default(),
            utility_diagnostic_keys: HashSet::default(),
            utility_diagnostic_keys_baseline: None,
            symbols: data.symbols.clone(),
            type_declarations: data.type_declarations.as_ref().clone(),
            type_declaration_scope: data.type_declaration_scope.clone(),
            program_type_store: data.program_type_store.clone(),
            substitution_store: data.substitution_store.clone(),
            declaration_environment_store,
            declaration_environment_generation: 0,
            resolved_named_types_identity: data.resolved_named_types_identity.clone(),
            resolution_stage_counter: data.resolution_stage_counter,
            environment_attempt: data.environment_attempt,
            environment_visit_counter: data.environment_visit_counter,
            resolved_named_types: data.resolved_named_types.clone(),
            program_resolved_generic_types: data.program_resolved_generic_types.clone(),
            program_instantiations: data.program_instantiations.clone(),
            physical_interface_instantiations: data.physical_interface_instantiations.clone(),
            lazy_member_annotation_templates: data.lazy_member_annotation_templates.clone(),
            physical_interface_declaration_templates: data
                .physical_interface_declaration_templates
                .clone(),
            physical_interface_method_instantiations: data
                .physical_interface_method_instantiations
                .clone(),
            physical_interface_overload_instantiations: data
                .physical_interface_overload_instantiations
                .clone(),
            ambient_modules: data.ambient_modules.clone(),
            import_type_namespaces: data.import_type_namespaces.clone(),
            import_type_globals: data.import_type_globals.clone(),
            ambient_file_type_scopes: data.ambient_file_type_scopes.clone(),
            module_augmentations: data.module_augmentations.clone(),
            ambient_global_symbols: data.ambient_global_symbols.clone(),
            // A declaration environment only re-resolves types; it never runs the
            // expression checks that report TS2686, so it carries no UMD state.
            umd_global_names: Arc::new(FxHashMap::default()),
            namespace_registry: Arc::default(),
            block_scoped_globals: Arc::default(),
            global_augmentation_only_names: Arc::default(),
            file_umd_global_names: FxHashSet::default(),
            file_umd_global_names_owner: None,
            merge_script_interfaces_with_globals: false,
            definite_writes: Arc::default(),
            inherited_never_initialized: Vec::new(),
            container_assigned_bindings: Arc::default(),
            never_initialized_constraint_exempt: HashSet::new(),
            file_type_only_import_names: FxHashSet::default(),
            file_type_only_alias_names: FxHashMap::default(),
            file_import_names: FxHashSet::default(),
            file_namespace_import_names: FxHashSet::default(),
            file_complete_namespace_import_names: FxHashSet::default(),
            file_value_only_import_alias_names: FxHashSet::default(),
            file_const_class_names: FxHashSet::default(),
            checked_function_declaration_names: FxHashSet::default(),
            file_type_only_import_names_owner: None,
            type_only_alias_use_valid: false,
            file_type_only_import_handles: FxHashMap::default(),
            ambient_global_type_declarations: data.ambient_global_type_declarations.clone(),
            module_file_index_by_identity: data.module_file_index_by_identity.clone(),
            module_scope_by_file: data.module_scope_by_file.clone(),
            module_local_values_by_file: data.module_local_values_by_file.clone(),
            thin_superseded_value_collection: false,
            lazy_library_value_annotations: false,
            skip_annotated_function_bodies: false,
            jsx_namespace_modules: data.jsx_namespace_modules.clone(),
            jsx_factory_uses: Default::default(),
            jsx_namespace: None,
            jsx_fragment_factory_resolved: false,
            type_parameter_scopes: data.type_parameter_scopes.clone(),
            type_parameter_constraint_scopes: data.type_parameter_constraint_scopes.clone(),
            timings: data.timings.clone(),
            expando_initializer_members: Arc::default(),
            namespace_member_resolution_depth: 0,
            instantiation_work: 0,
            instantiation_depth: 0,
            unmodelled_jsx_props_depth: 0,
            this_is_implicitly_any: false,
            contextual_this_missing: false,
            javascript_expando_objects: Arc::default(),
            module_level_member_write: false,
            constructor_writable_members: None,
            super_constructor_type: None,
            super_class_reference: None,
            collecting_signatures: false,
            resolving_class_heritage: false,
            union_literal_target: None,
            shorthand_property_depth: 0,
            degraded_expected_type_depth: 0,
            genuine_any_bindings: HashSet::default(),
            inferred_any_vars: HashSet::default(),
            this_initialized_vars: HashSet::default(),
            union_member_probe_depth: 0,
            contextual_return_frames: Vec::new(),
            in_contextual_return_check: false,
            relating_unfresh_return: false,
            in_async_body: false,
            in_generator_body: false,
            implicit_any_yields: Vec::new(),
            implicit_any_yield_generator: false,
            circular_var_starts: Vec::new(),
            generator_yield_type: None,
            generator_next_type: None,
            in_generator_function: false,
            in_async_generator: false,
            next_arrow_is_argument: false,
            next_arrow_context_only: false,
            namespace_require_reads: None,
            split_returned_conditional: false,
            allow_missing_tuple_elements: Vec::new(),
            non_exhaustive_switches: Vec::new(),
            exhaustive_switches: Vec::new(),
            proven_exhaustive_switches: Vec::new(),
            next_body_frame_active: false,
            cross_file_resolution_depth: u32::from(data.crossed_file),
            namespace_member_prefix_stack: Vec::new(),
            namespace_block_prefix_stack: Vec::new(),
            enclosing_classes: Vec::new(),
            enclosing_class_members: Vec::new(),
            never_returning_calls: FxHashSet::default(),
            static_member_type_parameters: None,
            constructor_local_properties: Arc::default(),
            script_function_implementations: Arc::default(),
            constructor_local_outer_bindings: Vec::new(),
            nested_function_scope: None,
            parenthesized_expressions: Arc::from([]),
            global_this_starts: Arc::from([]),
            module_this_starts: Arc::from([]),
            literal_this_members: Arc::from([]),
            next_arrow_this: None,
            expando_object_literal: None,
            evolving_array_operation_target: None,
            auto_arrays_declared: false,
            let_assignments: Arc::from([]),
            module_declared_only_depth: 0,
            module_export_depth: 0,
            lowest_cycle_target_index: usize::MAX,
            lowest_unknown_cycle_target_index: usize::MAX,
            structural_resolution_frames: Vec::new(),
            type_literal_member_frames: Vec::new(),
            file_kinds: data.file_kinds.clone(),
            module_value_fallback: data.module_value_fallback.clone(),
            unreachable_statement_runs: Vec::new(),
        }
    }

    /// End-of-run sizes of the shared program type caches, sampled before
    /// [`Self::clear_program_type_caches`] tears them down.
    pub(crate) fn program_cache_stats(&self) -> crate::metrics::ProgramCacheStats {
        let (generic_type_buckets, generic_type_entries) = self
            .program_resolved_generic_types
            .lock()
            .map(|cache| {
                (
                    cache.len() as u64,
                    cache.values().map(|bucket| bucket.len() as u64).sum(),
                )
            })
            .unwrap_or_default();
        let (instantiation_buckets, instantiation_entries) = self
            .program_instantiations
            .lock()
            .map(|cache| {
                (
                    cache.len() as u64,
                    cache.values().map(|bucket| bucket.len() as u64).sum(),
                )
            })
            .unwrap_or_default();
        let physical_interface_entries = self
            .physical_interface_instantiations
            .lock()
            .map(|cache| cache.len() as u64)
            .unwrap_or_default();
        crate::metrics::ProgramCacheStats {
            generic_type_buckets,
            generic_type_entries,
            instantiation_buckets,
            instantiation_entries,
            physical_interface_entries,
        }
    }

    pub(crate) fn clear_program_type_caches(&self) {
        crate::checks::function::clear_circular_body_returns();
        if let Ok(mut cache) = self.resolved_named_types.lock() {
            cache.clear();
        }
        if let Ok(mut namespaces) = self.import_type_namespaces.lock() {
            namespaces.clear();
        }
        if let Ok(mut cache) = self.program_resolved_generic_types.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.program_instantiations.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.physical_interface_instantiations.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.physical_interface_declaration_templates.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.physical_interface_method_instantiations.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.physical_interface_overload_instantiations.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.lazy_member_annotation_templates.lock() {
            cache.clear();
        }
        crate::infer::types::cache::clear_program_module_instantiation_memo();
        self.substitution_store.clear();
        if let Ok(mut environments) = self.declaration_environment_store.entries.lock() {
            environments.by_key.clear();
            environments.by_id.clear();
        }
        crate::program::clear_program_module_scopes();
        crate::infer::types::clear_deferred_merges();
        surge_ts_types::clear_name_intern_table();
    }

    /// Whether unresolved type names should be silently treated as `unknown`
    /// rather than emitting TS2304 — true while expanding a namespace-qualified
    /// member body. See [`Self::namespace_member_resolution_depth`].
    pub(crate) fn suppress_unknown_type_name(&self) -> bool {
        self.namespace_member_resolution_depth > 0
    }

    pub(crate) fn register_import_type_namespace(
        &self,
        file_name: &str,
        specifier: &str,
        target: ImportTypeTarget,
    ) {
        if let Ok(mut namespaces) = self.import_type_namespaces.lock() {
            namespaces.insert((Arc::from(file_name), specifier.to_string()), target);
        }
    }

    /// The declaration an import type (`import("m").T`) written in the
    /// current file names: the module's type export, resolving in the
    /// module's own scope.
    pub(crate) fn import_type_declaration_handle(
        &self,
        name: &str,
    ) -> Option<crate::symbols::TypeDeclarationHandle> {
        let (specifier, qualifier) = surge_ts_syntax::split_import_type_name(name)?;
        let mut namespaces = self.import_type_namespaces.lock().ok()?;
        let target = namespaces.get_mut(&(Arc::from(self.file_name.as_str()), specifier.to_string()))?;
        if let Some(handle) = target.attached.get(qualifier) {
            return Some(handle.clone());
        }
        let key = match (target.export_assignment, qualifier.is_empty()) {
            (true, true) => crate::modules::EXPORT_ASSIGNMENT_NAME.to_string(),
            (true, false) => format!("{}.{qualifier}", crate::modules::EXPORT_ASSIGNMENT_NAME),
            (false, _) => qualifier.to_string(),
        };
        let declaration = target.declarations.get(&key)?.clone();
        let handle = crate::symbols::TypeDeclarationHandle::new(
            crate::modules::exports::attach_type_resolution_scope_if_missing(
                declaration,
                target.scope.as_ref(),
            ),
        );
        target.attached.insert(qualifier.to_string(), handle.clone());
        Some(handle)
    }

    pub(crate) fn register_import_type_global(&self, name: &str) {
        if let Ok(mut globals) = self.import_type_globals.lock() {
            globals.insert(name.to_string());
        }
    }

    pub(crate) fn is_import_type_global(&self, name: &str) -> bool {
        self.import_type_globals
            .lock()
            .ok()
            .is_some_and(|globals| globals.contains(name))
    }

    pub(crate) fn import_type_namespace(&self, file_name: &str, specifier: &str) -> Option<Type> {
        self.import_type_namespaces
            .lock()
            .ok()
            .and_then(|namespaces| {
                namespaces
                    .get(&(Arc::from(file_name), specifier.to_string()))
                    .map(|target| target.namespace.clone())
            })
    }

    /// tsc's name resolver (`nameresolver.go`): a class type parameter is not
    /// in scope in a static member, unless a nearer declaration rebinds the
    /// name.
    pub(crate) fn names_static_forbidden_type_parameter(
        &self,
        name: &str,
        span: Option<surge_ts_syntax::TextSpan>,
        written_here: bool,
    ) -> bool {
        let Some((member_span, depth, names)) = self.static_member_type_parameters.as_ref() else {
            return false;
        };
        // Only a reference written inside the member is out of scope: resolving
        // its annotations expands the class's own declaration, whose other
        // members name the parameter legitimately. A reference that carries no
        // span is judged by whether it is being resolved on its own rather
        // than beneath such an expansion.
        match member_span.zip(span) {
            Some((member_span, span)) => {
                if span.start < member_span.start || span.end > member_span.end {
                    return false;
                }
            }
            None if !written_here => return false,
            None => {}
        }
        names.iter().any(|forbidden| forbidden == name)
            && !self.type_parameter_scopes[(*depth + 1).min(self.type_parameter_scopes.len())..]
                .iter()
                .any(|scope| scope.contains_key(name))
    }

    /// Whether an enclosing declaration currently binds `name` as a type
    /// parameter; a `Type::TypeParameter` that none does is one that leaked
    /// out of an instantiation.
    pub(crate) fn type_parameter_in_scope(&self, name: &str) -> bool {
        self.type_parameter_scopes
            .iter()
            .any(|scope| scope.contains_key(name))
    }

    pub(crate) fn push_type_parameter_scope(
        &mut self,
        type_parameters: &[ParsedTypeParameter],
        substitution: Option<HashMap<String, Type>>,
    ) {
        let mut scope = substitution.unwrap_or_default();
        let mut constraint_scope = HashMap::new();
        for type_parameter in type_parameters {
            let variable = type_parameter.name_span.and_then(|span| {
                surge_ts_types::type_variable::variable_for_declaration(&self.file_name, span.start as u32)
            });
            match variable {
                Some(variable) => {
                    scope.insert(type_parameter.name.clone(), variable);
                }
                None => {
                    scope.entry(type_parameter.name.clone()).or_insert(Type::Unknown);
                }
            }
            if let Some(constraint) = type_parameter.constraint.clone() {
                constraint_scope.insert(type_parameter.name.clone(), constraint);
            }
        }
        self.type_parameter_scopes.push(shared_scope(scope, empty_type_parameter_scope));
        self.type_parameter_constraint_scopes
            .push(shared_scope(constraint_scope, empty_constraint_scope));
    }

    pub(crate) fn pop_type_parameter_scope(&mut self) {
        self.type_parameter_scopes
            .pop()
            .expect("type parameter scope stack must not underflow");
        self.type_parameter_constraint_scopes
            .pop()
            .expect("type parameter constraint scope stack must not underflow");
    }

    /// Registers only the *constraints* of `type_parameters` (an empty value
    /// scope is pushed alongside to keep the stacks aligned). Used while
    /// resolving a generic interface/alias body so `Internals["def"]` with
    /// `Internals extends $ZodTypeInternals<…>` is recognised as
    /// constraint-validated instead of cascading into TS2536 — without marking
    /// the resolution non-concrete (an entry in `type_parameter_scopes` would
    /// disable instantiation interning for everything resolved inside).
    pub(crate) fn push_type_parameter_constraints_only(
        &mut self,
        type_parameters: &[ParsedTypeParameter],
    ) {
        let mut constraint_scope = HashMap::new();
        for type_parameter in type_parameters {
            if let Some(constraint) = type_parameter.constraint.clone() {
                constraint_scope.insert(type_parameter.name.clone(), constraint);
            }
        }
        self.type_parameter_scopes.push(empty_type_parameter_scope());
        self.type_parameter_constraint_scopes
            .push(shared_scope(constraint_scope, empty_constraint_scope));
    }

    /// When the in-scope type parameter `name` is declared as `name extends keyof X`,
    /// return the referenced type-parameter name `X`. Used to keep generic `T[K]`
    /// indexed access valid when `K extends keyof T`.
    pub(crate) fn type_parameter_keyof_constraint_target(&self, name: &str) -> Option<&str> {
        for scope in self.type_parameter_constraint_scopes.iter().rev() {
            if let Some(constraint) = scope.get(name) {
                if let ParsedType::KeyOf(inner) = constraint
                    && let ParsedType::Named(named) = inner.as_ref()
                {
                    return Some(named.name.as_str());
                }
                return None;
            }
        }
        None
    }

    /// Whether the in-scope type parameter `name` was declared with any
    /// `extends` constraint. A constrained parameter's valid index keys depend
    /// on its (often complex, library-generated) constraint, which we do not
    /// fully resolve; tsc validates the access against that constraint, so an
    /// indexed access through a constrained parameter must not cascade into a
    /// `TS2536`/`TS2538` false positive.
    pub(crate) fn type_parameter_constraint(&self, name: &str) -> Option<&ParsedType> {
        self.type_parameter_constraint_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
    }

    pub(crate) fn type_parameter_has_constraint(&self, name: &str) -> bool {
        self.type_parameter_constraint_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(name))
    }

    pub(crate) fn set_file_name(&mut self, file_name: String) {
        if self.file_name != file_name {
            self.declaration_environment_generation =
                self.declaration_environment_generation.wrapping_add(1);
            self.environment_visit_counter = self.environment_visit_counter.wrapping_add(1);
            self.file_name_arc = None;
            self.canonical_file_name_memo = None;
        }
        self.current_file_kind = self
            .file_kinds
            .get(&file_name)
            .copied()
            .unwrap_or(FileKind::RootSource);
        self.file_name = file_name;
    }

    /// The canonical twin of [`Self::file_name_arc`], with the same
    /// revalidation rule.
    pub(crate) fn canonical_file_name_arc(&mut self) -> Arc<str> {
        if let Some((raw, canonical)) = &self.canonical_file_name_memo
            && **raw == *self.file_name
        {
            return canonical.clone();
        }
        let raw = self.file_name_arc();
        let canonical =
            crate::paths::canonicalize_if_exists_arc(std::path::Path::new(&*self.file_name));
        self.canonical_file_name_memo = Some((raw, canonical.clone()));
        canonical
    }

    /// The memo is revalidated against `file_name` on every call because a few
    /// callers rebind `file_name` directly instead of through `set_file_name`.
    pub(crate) fn file_name_arc(&mut self) -> Arc<str> {
        if let Some(arc) = &self.file_name_arc
            && **arc == *self.file_name
        {
            return arc.clone();
        }
        let arc: Arc<str> = Arc::from(self.file_name.as_str());
        self.file_name_arc = Some(arc.clone());
        arc
    }

    /// Retained-capacity bound for the per-file utility-key overlay. A typical
    /// file records at most a handful of keys; one pathological file must not
    /// pin a huge table on the worker for the rest of the run.
    const UTILITY_KEY_OVERLAY_RETAINED_CAPACITY: usize = 1024;

    /// File-region reset for a worker context that is reused across files.
    /// Serial checking clones a fresh context per file, so each file starts
    /// from the pre-check key set; a parallel worker reuses one context, and
    /// without this reset its utility keys accumulate every checked file's
    /// entries for the worker's lifetime (and can suppress diagnostics serial
    /// checking emits). The first call moves the inherited keys into the
    /// shared baseline; later calls clear only the per-file overlay, so the
    /// reused context behaves byte-identically to a fresh clone without
    /// cloning the key set per file.
    /// `resolved_named_types` must be a fresh map rather than cleared in place:
    /// resolutions depend on the consumer file's environment, and retained
    /// declaration environments may still refer to the previous file's map.
    /// Whether `name` reaches the file under check only as a UMD global, which
    /// makes a value reference to it TS2686. Reads the per-file set computed at
    /// file entry, so shadowing by a local or imported binding is already
    /// resolved.
    pub(crate) fn is_umd_global_value_reference(&self, name: &str) -> bool {
        if self.file_umd_global_names.is_empty()
            || self.file_umd_global_names_owner.as_deref() != Some(self.file_name.as_str())
            || !self.file_umd_global_names.contains(name)
        {
            return false;
        }

        // The per-file set only rules out shadowing that is visible from the
        // file's own imports and module-level tables. A binding introduced in an
        // enclosing scope — or a module-level one those tables missed — still
        // shadows the global, so the name must either resolve to nothing or
        // resolve to the ambient global entry itself.
        match self.symbols.get_handle(name) {
            None => true,
            Some(resolved) => self
                .ambient_global_symbols
                .get_handle(name)
                .is_some_and(|global| Arc::ptr_eq(&resolved, &global)),
        }
    }

    /// Narrows the program's UMD global names to the ones the file under check
    /// reaches through the global scope. `is_module` is the referencing file's
    /// module-ness (a script may reference UMD globals freely) and `is_shadowed`
    /// reports whether the file binds the name itself, as a value or a type —
    /// tsc reports a different diagnostic for a shadowing `import type`, so a
    /// bound name is left alone here either way.
    /// Whether `name` reaches the file under check only through a type-only
    /// import, which makes a value reference to it TS1361.
    ///
    /// The import must actually name something with a value meaning: importing
    /// a *type* with `import type` and using it as a value is TS2693 in tsc
    /// ("only refers to a type"), not TS1361.
    /// A type-only import whose target has a value side: tsc resolves the
    /// alias and reports the use (TS1361). A target that is only a type is the
    /// plain type-as-value error instead; a class is both.
    pub(crate) fn is_type_only_import_value_reference(&self, name: &str) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
            && self.file_type_only_import_names.contains(name)
            && match self.lookup_type_declaration(name) {
                None => true,
                Some(TypeDeclarationInfo::Interface(info)) => info.is_class_instance,
                // An enum is lowered to an alias of its name, and has a value.
                Some(TypeDeclarationInfo::Alias(alias)) => alias.enum_name.is_some(),
            }
    }

    /// tsc's `getTypeOnlyAliasDeclarationEx` for a value use of `name`: the
    /// kind of type-only declaration its import passes through, the file's own
    /// `import type` first.
    pub(crate) fn type_only_value_reference(
        &self,
        name: &str,
    ) -> Option<crate::modules::TypeOnlyAliasKind> {
        if self.is_type_only_import_value_reference(name) {
            return Some(crate::modules::TypeOnlyAliasKind::Import);
        }
        if self.file_type_only_import_names_owner.as_deref() != Some(self.file_name.as_str()) {
            return None;
        }
        self.file_type_only_alias_names.get(name).copied()
    }

    /// `Some(instantiated)` when `name` is a namespace visible here, looking
    /// through the namespaces whose members are being resolved.
    pub(crate) fn namespace_meaning(&self, name: &str) -> Option<bool> {
        let registry = &self.namespace_registry;
        self.namespace_member_prefix_stack
            .iter()
            .rev()
            .find_map(|prefix| registry.lookup(&self.file_name, &format!("{prefix}.{name}")))
            .or_else(|| registry.lookup(&self.file_name, name))
            .or_else(|| self.import_alias_namespace_meaning(name))
    }

    /// An import alias whose target has a namespace meaning (`import N =
    /// require("./m")` over `export = N`): the members the import binds are keyed
    /// `N.<member>` in the file's import layers. An enum's members are keyed the
    /// same way, but an enum is not a namespace. Instantiated when the alias
    /// binds a value too.
    fn import_alias_namespace_meaning(&self, name: &str) -> Option<bool> {
        if !self.is_import_binding(name) {
            return None;
        }
        let heads_member = self.type_declaration_scope.as_ref()?.layers().iter().any(|layer| {
            layer.iter().any(|(key, declaration)| {
                key.strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with('.'))
                    && !matches!(declaration, TypeDeclarationInfo::Alias(alias)
                        if alias.enum_name.is_some())
            })
        });
        heads_member.then(|| self.symbols.get(name).is_some())
    }

    /// The namespace `name` names here, looking through the namespaces whose
    /// members are being resolved.
    pub(crate) fn namespace_info(
        &self,
        name: &str,
    ) -> Option<(String, &crate::program::NamespaceInfo)> {
        let registry = &self.namespace_registry;
        self.namespace_member_prefix_stack
            .iter()
            .rev()
            .find_map(|prefix| {
                let qualified = format!("{prefix}.{name}");
                registry
                    .info(&self.file_name, &qualified)
                    .map(|info| (qualified, info))
            })
            .or_else(|| {
                registry
                    .info(&self.file_name, name)
                    .map(|info| (name.to_string(), info))
            })
    }

    /// Whether the per-file import sets describe the file under check. While a
    /// declaration from another file is being resolved they do not.
    pub(crate) fn knows_file_imports(&self) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
    }

    pub(crate) fn is_import_binding(&self, name: &str) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
            && self.file_import_names.contains(name)
    }

    pub(crate) fn is_namespace_import_binding(&self, name: &str) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
            && self.file_namespace_import_names.contains(name)
    }

    pub(crate) fn is_complete_namespace_import_binding(&self, name: &str) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
            && self.file_complete_namespace_import_names.contains(name)
    }

    pub(crate) fn is_value_only_import_alias(&self, name: &str) -> bool {
        self.file_type_only_import_names_owner.as_deref() == Some(self.file_name.as_str())
            && self.file_value_only_import_alias_names.contains(name)
    }

    pub(crate) fn set_file_value_only_import_alias_names<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a Arc<str>>,
    ) {
        self.file_value_only_import_alias_names.clear();
        self.file_value_only_import_alias_names.extend(names.into_iter().cloned());
    }

    pub(crate) fn set_file_complete_namespace_import_names<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a Arc<str>>,
    ) {
        self.file_complete_namespace_import_names.clear();
        self.file_complete_namespace_import_names
            .extend(names.into_iter().cloned());
    }

    pub(crate) fn set_file_import_names<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a str>,
        namespace_names: impl IntoIterator<Item = &'a str>,
    ) {
        self.file_import_names.clear();
        self.file_import_names.extend(names.into_iter().map(Arc::from));
        self.file_namespace_import_names.clear();
        self.file_namespace_import_names
            .extend(namespace_names.into_iter().map(Arc::from));
    }

    pub(crate) fn set_file_type_only_import_names<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a str>,
    ) {
        self.file_type_only_import_names.clear();
        self.file_type_only_import_names_owner = Some(self.file_name.clone());
        for name in names {
            self.file_type_only_import_names.insert(Arc::from(name));
        }
    }

    pub(crate) fn set_file_type_only_alias_names(
        &mut self,
        names: impl IntoIterator<Item = (Arc<str>, crate::modules::TypeOnlyAliasKind)>,
    ) {
        self.file_type_only_alias_names.clear();
        self.file_type_only_alias_names.extend(names);
    }

    pub(crate) fn set_file_umd_global_names(
        &mut self,
        is_module: bool,
        mut is_shadowed: impl FnMut(&str) -> bool,
    ) {
        self.file_umd_global_names.clear();
        self.file_umd_global_names_owner = None;
        self.degraded_expected_type_depth = 0;
        self.union_member_probe_depth = 0;
        self.contextual_return_frames.clear();
        self.in_contextual_return_check = false;
        self.relating_unfresh_return = false;
        self.in_async_body = false;
        self.in_generator_body = false;
        self.implicit_any_yields.clear();
        self.implicit_any_yield_generator = false;
        self.generator_yield_type = None;
        self.generator_next_type = None;
        self.in_generator_function = false;
        self.in_async_generator = false;
        self.next_arrow_is_argument = false;
        self.next_arrow_context_only = false;
        self.next_body_frame_active = false;
        if !is_module || self.options.allow_umd_global_access || self.umd_global_names.is_empty() {
            return;
        }
        self.file_umd_global_names_owner = Some(self.file_name.clone());
        for name in self.umd_global_names.keys() {
            if !is_shadowed(name) {
                self.file_umd_global_names.insert(name.clone());
            }
        }
    }

    /// The outermost parentheses around the expression spanning `inner`, which
    /// is where tsc anchors a diagnostic on that operand.
    pub(crate) fn parenthesized_outer_span(&self, inner: SyntaxTextSpan) -> Option<SyntaxTextSpan> {
        let key = (inner.start, inner.end);
        self.parenthesized_expressions
            .binary_search_by_key(&key, |span| (span.inner.start, span.inner.end))
            .ok()
            .map(|index| self.parenthesized_expressions[index].outer)
    }

    /// How the flow-typed `let` whose name starts at `name_start` is assigned.
    pub(crate) fn let_assignment(&self, name_start: usize) -> Option<surge_ts_syntax::LetAssignmentSummary> {
        let key = u32::try_from(name_start).ok()?;
        self.let_assignments
            .binary_search_by_key(&key, |summary| summary.name_start)
            .ok()
            .map(|index| self.let_assignments[index])
    }

    pub(crate) fn begin_file_check(&mut self, file_name: String) {
        self.set_file_name(file_name);
        self.circular_var_starts.clear();
        self.type_declaration_scope = None;
        if self.utility_diagnostic_keys_baseline.is_none() {
            self.utility_diagnostic_keys_baseline =
                Some(Arc::new(std::mem::take(&mut self.utility_diagnostic_keys)));
        } else if self.utility_diagnostic_keys.capacity()
            > Self::UTILITY_KEY_OVERLAY_RETAINED_CAPACITY
        {
            self.utility_diagnostic_keys = HashSet::default();
        } else {
            self.utility_diagnostic_keys.clear();
        }
        self.diagnostic_keys.clear();
        self.diagnostic_keys_len = 0;
        self.file_umd_global_names.clear();
        self.file_umd_global_names_owner = None;
        self.file_type_only_import_names.clear();
        self.file_type_only_alias_names.clear();
        self.file_type_only_import_handles.clear();
        self.file_type_only_import_names_owner = None;
        self.checked_function_declaration_names.clear();
        self.grammar_answered_spans.clear();
        self.unreachable_statement_runs.clear();
        self.deferred_grammar_findings.clear();
        crate::checks::call::clear_overload_callback_memo();
        self.definite_writes = Arc::default();
        self.inherited_never_initialized.clear();
        self.container_assigned_bindings = Arc::default();
        self.expando_initializer_members = Arc::default();
        self.never_initialized_constraint_exempt.clear();
        self.non_exhaustive_switches.clear();
        self.exhaustive_switches.clear();
        self.proven_exhaustive_switches.clear();
        self.genuine_any_bindings.clear();
        self.inferred_any_vars.clear();
        self.this_initialized_vars.clear();
        self.this_is_implicitly_any = false;
        self.javascript_expando_objects = Arc::default();
        self.module_level_member_write = false;
        self.constructor_writable_members = None;
        self.super_constructor_type = None;
        self.super_class_reference = None;
        self.collecting_signatures = false;
        self.resolving_class_heritage = false;
        self.union_literal_target = None;
        self.shorthand_property_depth = 0;
        self.enclosing_classes.clear();
        self.enclosing_class_members.clear();
        self.static_member_type_parameters = None;
        self.never_returning_calls.clear();
        self.nested_function_scope = None;
        self.parenthesized_expressions = Default::default();
        self.global_this_starts = Default::default();
        self.module_this_starts = Default::default();
        self.literal_this_members = Default::default();
        self.next_arrow_this = None;
        self.expando_object_literal = None;
        self.file_const_class_names.clear();
        self.jsx_factory_uses = Default::default();
        self.jsx_namespace = None;
        self.jsx_fragment_factory_resolved = false;
        self.evolving_array_operation_target = None;
        self.auto_arrays_declared = false;
        self.let_assignments = Default::default();
        self.module_declared_only_depth = 0;
        self.module_export_depth = 0;
        self.namespace_require_reads = None;
        debug_assert!(
            self.diagnostics.is_empty(),
            "begin_file_check: previous file's diagnostics were not taken"
        );
        self.replace_resolved_named_types(0);
    }

    /// Empties both the overlay and the baseline — the equivalent of clearing
    /// the whole pre-region key set. Used by the per-file signature-collection
    /// context, which intentionally re-reports keys recorded elsewhere.
    pub(crate) fn reset_utility_diagnostic_keys(&mut self) {
        self.utility_diagnostic_keys.clear();
        self.utility_diagnostic_keys_baseline = None;
    }

    /// Moves the inherited utility-key overlay into the shared baseline (the
    /// same split [`Self::begin_file_check`] performs on its first call), so a
    /// per-module speculative clone starts with an empty overlay: suppression
    /// still consults the full inherited set, and after the module's analysis
    /// the overlay holds exactly that module's key additions.
    /// Drops the push-dedup index (it lazily rebuilds from `diagnostics` on
    /// the next deduplicated push), so cheap clones don't deep-copy a stale
    /// key set.
    pub(crate) fn clear_diagnostic_keys(&mut self) {
        self.diagnostic_keys = HashSet::default();
        self.diagnostic_keys_len = 0;
    }

    pub(crate) fn snapshot_utility_keys_into_baseline(&mut self) {
        let inherited = std::mem::take(&mut self.utility_diagnostic_keys);
        self.utility_diagnostic_keys_baseline = match self.utility_diagnostic_keys_baseline.take() {
            None => Some(Arc::new(inherited)),
            Some(existing) => {
                let mut merged = (*existing).clone();
                merged.extend(inherited);
                Some(Arc::new(merged))
            }
        };
    }

    pub(crate) fn set_symbols(&mut self, symbols: SymbolTable) {
        self.symbols = symbols;
        self.declaration_environment_generation =
            self.declaration_environment_generation.wrapping_add(1);
    }

    /// Whether `file_name` is a trusted upstream library/dependency declaration
    /// file. Resolutions of declarations in such files are context-free (their
    /// bodies reference only the global ambient surface) and emit no use-site
    /// diagnostics under `skipLibCheck`, so they are safe to memoize program-wide.
    pub(crate) fn is_library_scoped_file(&self, file_name: &str) -> bool {
        // Probe `file_kinds` first: the hot callers ask about dependency
        // `.d.ts` files that are registered there, and the name-based
        // default-lib checks each cost a thread-local string-hash lookup.
        if matches!(
            self.file_kinds.get(file_name),
            Some(
                FileKind::DependencyDeclaration
                    | FileKind::GeneratedDeclaration
                    | FileKind::PhysicalDefaultLib
            )
        ) {
            return true;
        }
        crate::default_lib::is_physical_default_lib_file_name(file_name)
            || crate::default_lib::is_generated_default_lib_file_name(file_name)
    }

    pub(crate) fn lookup_type_declaration(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        // A namespace member's own siblings shadow any outer/global declaration of
        // the same name: inside `namespace React` a bare `MouseEvent` is
        // `React.MouseEvent` (a generic interface), not the non-generic DOM global.
        // Resolving the qualified candidates first is what keeps generic React event
        // types (`MouseEventHandler<T> = EventHandler<MouseEvent<T>>`) from degrading
        // to the arity-0 global and losing their function shape.
        // `globalThis.X` explicitly targets the global scope, bypassing every
        // local shadow — `interface Iterator<T> extends globalThis.Iterator<T>`
        // must reach the lib Iterator, not re-find its own shadowing
        // declaration. Only the ambient global table is consulted.
        if let Some(global_name) = name.strip_prefix("globalThis.") {
            crate::program::record_type_declaration_lookup(3);
            return self.ambient_global_type_declarations.get(global_name);
        }
        for candidate in self.namespace_qualified_candidates(name) {
            if let Some(declaration) = self.lookup_type_declaration_exact(&candidate) {
                return Some(declaration);
            }
        }
        if let Some(declaration) = self.lookup_type_declaration_exact(name) {
            return Some(declaration);
        }
        for candidate in self.namespace_dotted_retry_candidates(name) {
            if let Some(declaration) = self.lookup_type_declaration_exact(&candidate) {
                return Some(declaration);
            }
        }
        None
    }

    /// Opt-in (`SURGE_NS_QUALIFIED_RETRY=1`) fallback for an already-dotted name
    /// written inside a namespace body: an enum declared inside
    /// `declare namespace ts` registers as `ts.SyntaxKind.X`, so the bare
    /// `SyntaxKind.X` its sibling members write misses the exact lookup. The
    /// retry runs strictly after the exact lookup, so it can never shadow an
    /// existing resolution.
    fn namespace_dotted_retry_candidates(&self, name: &str) -> Vec<String> {
        if !namespace_qualified_retry_enabled() || !name.contains('.') {
            return Vec::new();
        }
        self.namespace_prefix_candidates(name)
    }

    /// Whether name lookups for the file currently being resolved must ignore
    /// the consumer module's own declaration table (`self.type_declarations`).
    ///
    /// While a dependency `.d.ts` declaration body is being expanded from
    /// another file (a `with_file_name` frame whose file differs from the one
    /// being analyzed), `self.type_declarations` still holds the *consuming*
    /// module's local table. Consulting it would let a consumer-local type
    /// name shadow the dependency's own lexical scope inside the dependency's
    /// body — wrong per tsc, and it makes expansions depend on which module
    /// triggered them (defeating cross-module expansion reuse). The dependency
    /// file's own declarations travel in its installed resolution scope, so
    /// scope lookup serves the body's own names. Same-file resolution and
    /// windows with no installed scope keep the local-table consult.
    fn lookup_ignores_local_table(&self) -> bool {
        // A global declaration sees no module's locals, however the context
        // resolving it was recovered.
        self.type_declaration_scope.as_ref().is_some_and(|scope| {
            self.cross_file_resolution_depth > 0 || crate::symbols::is_global_declaration_scope(scope)
        })
    }

    /// A global interface re-opened by several declarations lives fully merged in
    /// the ambient table. A narrower layer can hold one of those declarations on
    /// its own — a dependency's own declaration table becomes a scope layer for
    /// every file that resolves through it — and answering from that layer
    /// silently drops every other contributor. `NodeJS.ProcessEnv` is the case
    /// that motivated this: `next` re-opens it with just `NODE_ENV`, so a file
    /// resolving through `next` lost `@types/node`'s `extends Dict<string>` and
    /// with it the index signature that makes `process.env.ANYTHING` legal.
    ///
    /// Only supersede when the ambient entry is provably the same declaration
    /// plus more: every fragment of the narrower entry must appear in it. A
    /// module-local interface that merely shares a qualified name is not a
    /// contributor and keeps its own answer.
    fn ambient_supersedes(&self, name: &str, found: &TypeDeclarationInfo) -> bool {
        // Only namespace-qualified keys. An *unqualified* name in a narrower
        // layer is module-local and is supposed to shadow a same-named global —
        // node-fetch's `class Response` must keep its `json(): Promise<unknown>`
        // rather than answering from the DOM's `interface Response`.
        if !name.contains('.') {
            return false;
        }
        let TypeDeclarationInfo::Interface(found) = found else {
            return false;
        };
        let Some(TypeDeclarationInfo::Interface(ambient)) =
            self.ambient_global_type_declarations.get(name)
        else {
            return false;
        };
        if ambient.body.declaration_fragments.len() <= found.body.declaration_fragments.len() {
            return false;
        }
        found
            .body
            .declaration_fragments
            .iter()
            .all(|fragment| ambient.body.declaration_fragments.contains(fragment))
    }

    fn lookup_type_declaration_exact(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        if let Some(declaration) = self
            .type_declaration_scope
            .as_ref()
            .and_then(|scope| scope.lexical_get(name))
        {
            return Some(declaration);
        }
        if !self.lookup_ignores_local_table()
            && let Some(declaration) = self.type_declarations.get(name)
        {
            crate::program::record_type_declaration_lookup(1);
            if self.ambient_supersedes(name, declaration) {
                return self.ambient_global_type_declarations.get(name);
            }
            return Some(declaration);
        }

        if let Some(scope) = self.type_declaration_scope.as_ref() {
            if let Some(declaration) = scope.get(name) {
                crate::program::record_type_declaration_lookup(2);
                if self.ambient_supersedes(name, declaration) {
                    return self.ambient_global_type_declarations.get(name);
                }
                return Some(declaration);
            }
        }

        crate::program::record_type_declaration_lookup(3);
        let found = self.ambient_global_type_declarations.get(name);
        found
    }

    /// Candidate qualified names for a bare reference made inside a namespace
    /// member body: the innermost active prefix and each enclosing one, joined to
    /// `name` (`React.X`; `A.B.X` then `A.X`). Empty unless a namespace member is
    /// being resolved, or when `name` is already qualified.
    fn namespace_qualified_candidates(&self, name: &str) -> Vec<String> {
        if name.contains('.') {
            return Vec::new();
        }
        self.namespace_prefix_candidates(name)
    }

    /// The prefix walk shared by [`Self::namespace_qualified_candidates`] and
    /// [`namespace_qualified_retry_enabled`]'s dotted retry: each active
    /// namespace prefix, outermost last, joined to `name`.
    fn namespace_prefix_candidates(&self, name: &str) -> Vec<String> {
        let Some(prefix) = self.namespace_member_prefix_stack.last() else {
            return Vec::new();
        };

        let mut candidates = Vec::new();
        if let Some(block) = self.namespace_block_prefix_stack.last()
            && block
                .strip_prefix(prefix.as_str())
                .is_some_and(|rest| rest.starts_with(crate::driver::NAMESPACE_BLOCK_MARKER))
        {
            candidates.push(format!("{block}.{name}"));
        }
        let mut remaining = prefix.as_str();
        loop {
            candidates.push(format!("{remaining}.{name}"));
            // A block local's own prefix falls back to its namespace's.
            let segment_start = remaining.rfind('.').map_or(0, |dot| dot + 1);
            if let Some(marker) = remaining[segment_start..].find(crate::driver::NAMESPACE_BLOCK_MARKER) {
                remaining = &remaining[..segment_start + marker];
                continue;
            }
            match remaining.rsplit_once('.') {
                Some((outer, _)) => remaining = outer,
                None => break,
            }
        }
        candidates
    }

    /// Like [`lookup_type_declaration`](Self::lookup_type_declaration) but returns
    /// a [`TypeDeclarationHandle`] whose borrow is decoupled from `self`, so
    /// resolution can read the declaration while `self` is borrowed mutably
    /// without deep-cloning the payload.
    pub(crate) fn lookup_type_declaration_handle(
        &self,
        name: &str,
    ) -> Option<crate::symbols::TypeDeclarationHandle> {
        // Namespace siblings shadow outer/global declarations of the same name — see
        // [`lookup_type_declaration`] for why this ordering matters.
        // See the `globalThis.` comment in `lookup_type_declaration`.
        if let Some(global_name) = name.strip_prefix("globalThis.") {
            crate::program::record_type_declaration_lookup(3);
            return self
                .ambient_global_type_declarations
                .get_handle(global_name);
        }
        for candidate in self.namespace_qualified_candidates(name) {
            if let Some(handle) = self.lookup_type_declaration_handle_exact(&candidate) {
                return Some(handle);
            }
        }
        if let Some(handle) = self.lookup_type_declaration_handle_exact(name) {
            return Some(handle);
        }
        for candidate in self.namespace_dotted_retry_candidates(name) {
            if let Some(handle) = self.lookup_type_declaration_handle_exact(&candidate) {
                return Some(handle);
            }
        }
        None
    }

    /// The declaration `name` resolves to in another file's own module scope
    /// (local declarations plus its resolved imports). Read-only and pointed at
    /// one file: a caller that needs to read a declaration's members without
    /// re-pointing the whole context at its file — which bumps the
    /// declaration-environment generation every later resolution is keyed on —
    /// asks this instead.
    pub(crate) fn lookup_type_declaration_handle_in_file(
        &self,
        name: &str,
        file: &str,
    ) -> Option<crate::symbols::TypeDeclarationHandle> {
        if let Some(scope) = self.module_scope_by_file.get(file) {
            return scope.get_handle(name);
        }
        if self.module_scope_by_file.is_empty() {
            return crate::program::program_module_scope_for_file(file)
                .and_then(|scope| scope.get_handle(name));
        }
        None
    }

    fn lookup_type_declaration_handle_exact(
        &self,
        name: &str,
    ) -> Option<crate::symbols::TypeDeclarationHandle> {
        if let Some(handle) = self
            .type_declaration_scope
            .as_ref()
            .and_then(|scope| scope.lexical_get_handle(name))
        {
            return Some(handle);
        }
        if !self.lookup_ignores_local_table()
            && let Some(handle) = self.type_declarations.get_handle(name)
        {
            crate::program::record_type_declaration_lookup(1);
            if self.ambient_supersedes(name, handle.get())
                && let Some(ambient) = self.ambient_global_type_declarations.get_handle(name)
            {
                return Some(ambient);
            }
            return Some(handle);
        }

        if let Some(scope) = self.type_declaration_scope.as_ref() {
            if let Some(handle) = scope.get_handle(name) {
                crate::program::record_type_declaration_lookup(2);
                if self.ambient_supersedes(name, handle.get())
                    && let Some(ambient) = self.ambient_global_type_declarations.get_handle(name)
                {
                    return Some(ambient);
                }
                return Some(handle);
            }
        }

        // The active `type_declaration_scope` can be incomplete — a declaration's
        // pre-attached `resolution_scope` may carry only its local layer (no
        // imports), and a lazy reference re-expanded outside its originating frame
        // may carry no scope at all. The authoritative per-file scope (local
        // declarations + resolved imports) lives in `module_scope_by_file`, keyed
        // by the file currently being resolved (`with_file_name` tracks it through
        // declaration bodies). Consulting it on a miss makes a type referenced from
        // file X resolvable whenever X can legitimately see it, independent of which
        // partial scope happened to be installed. This is what stabilizes mutually
        // imported clusters (ky's `Options`/`Hooks`/`NormalizedOptions` across the
        // circular `options.ts`/`hooks.ts` imports), whose resolution order would
        // otherwise leave a member degraded to `unknown`.
        crate::program::record_scope_fallback_consult();
        if let Some(scope) = self.module_scope_by_file.get(self.file_name.as_str()) {
            if let Some(handle) = scope.get_handle(name) {
                crate::program::record_type_declaration_lookup(2);
                if self.ambient_supersedes(name, handle.get())
                    && let Some(ambient) = self.ambient_global_type_declarations.get_handle(name)
                {
                    return Some(ambient);
                }
                return Some(handle);
            }
        } else if self.module_scope_by_file.is_empty()
            && let Some(scope) = crate::program::program_module_scope_for_file(&self.file_name)
            && let Some(handle) = scope.get_handle(name)
        {
            if self.ambient_supersedes(name, handle.get())
                && let Some(ambient) = self.ambient_global_type_declarations.get_handle(name)
            {
                crate::program::record_type_declaration_lookup(2);
                return Some(ambient);
            }
            // This context was recovered from an environment captured before the
            // map existed (a lazy annotation created during module analysis), so
            // the declaring file's own imports are invisible to it. The published
            // program map is the same authoritative per-file scope.
            crate::program::record_type_declaration_lookup(2);
            return Some(handle);
        }

        crate::program::record_type_declaration_lookup(3);
        let found = self.ambient_global_type_declarations.get_handle(name);
        found
    }

    pub(crate) fn set_module_file_index_by_identity(
        &mut self,
        module_file_index_by_identity: FxHashMap<Arc<str>, usize>,
    ) {
        self.module_file_index_by_identity = Arc::new(module_file_index_by_identity);
        self.declaration_environment_generation =
            self.declaration_environment_generation.wrapping_add(1);
    }

    pub(crate) fn set_module_scope_by_file(
        &mut self,
        module_scope_by_file: FxHashMap<Arc<str>, Arc<TypeDeclarationScope>>,
    ) {
        self.module_scope_by_file = Arc::new(module_scope_by_file);
        crate::program::publish_program_module_scopes(&self.module_scope_by_file);
        self.declaration_environment_generation =
            self.declaration_environment_generation.wrapping_add(1);
    }

    /// The resolution scope of the module that declared `file_name`, used as a
    /// fallback when a declaration's pre-attached `resolution_scope` was dropped
    /// across the cyclic-import binding fixpoint. See [`Self::module_scope_by_file`].
    pub(crate) fn module_scope_for_file(
        &self,
        file_name: &str,
    ) -> Option<Arc<TypeDeclarationScope>> {
        crate::program::record_scope_fallback_consult();
        self.module_scope_by_file.get(file_name).cloned()
    }

    pub(crate) fn set_module_local_values_by_file(
        &mut self,
        module_local_values_by_file: FxHashMap<Arc<str>, Arc<SymbolTable>>,
    ) {
        self.module_local_values_by_file = Arc::new(module_local_values_by_file);
        self.declaration_environment_store
            .publish_module_local_values(&self.module_local_values_by_file);
        self.declaration_environment_generation =
            self.declaration_environment_generation.wrapping_add(1);
    }

    /// The local value symbols of the module that declared `file_name`, used to
    /// resolve a `typeof <localValue>` inside an imported declaration's body
    /// (resolved under the declaring file's name via `with_file_name`, but against
    /// the consumer's value `symbols`). See [`Self::module_local_values_by_file`].
    pub(crate) fn module_local_values_for_file(&self, file_name: &str) -> Option<Arc<SymbolTable>> {
        let entry = self.module_local_values_by_file.get(file_name).cloned();
        if entry.is_some() {
            record_local_values_consult(file_name);
        } else if !self.module_local_values_by_file.is_empty()
            && local_values_consult_probe().is_some()
        {
            // Under SURGE_LV_PROBE, a consult that misses the populated map
            // would indicate the typeof-filter skipped a file that IS
            // consulted — a violation of the consult ⇒ contains-typeof
            // invariant the filter relies on.
            eprintln!("[lv-probe] MISS {file_name}");
        }
        entry
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        if let (surge_ts_diagnostics::DiagnosticCode::TypeScript(code), Some(span)) =
            (&diagnostic.code, diagnostic.span)
            && self
                .grammar_answered_spans
                .iter()
                .any(|(answered, codes)| *answered == span && codes.contains(code))
        {
            return;
        }
        if self.should_suppress(&diagnostic) {
            self.record_suppressed(&diagnostic);
            return;
        }
        if matches!(
            diagnostic.code,
            surge_ts_diagnostics::DiagnosticCode::TypeScript(2345)
        ) && diagnostic.span.is_some()
            && diagnostic.span == self.suppressed_argument_mismatch_span
        {
            return;
        }
        // An assignability verdict raised while checking a return value against a
        // contextual type belongs to the frame — it is the thing tsc does not
        // check. Everything else the expression reports (an unresolved name, a
        // missing member, an implicit-any parameter) is unrelated and is left
        // alone, which is what keeps this narrower than draining the range.
        let is_return_verdict = self.in_contextual_return_check
            && matches!(
                diagnostic.code,
                // The four shapes a return-value mismatch takes: not assignable,
                // excess property, and the two missing-property renderings.
                surge_ts_diagnostics::DiagnosticCode::TypeScript(2322 | 2353 | 2739 | 2741)
            );
        let before = self.diagnostics.len();
        self.push_deduplicated(diagnostic);
        if is_return_verdict && self.diagnostics.len() > before {
            let index = self.diagnostics.len() - 1;
            self.note_contextual_return_mismatch(index);
        }
    }

    /// Merges a diagnostic that already passed suppression in the context it
    /// was emitted from (a parallel analysis worker's, whose `current_file_kind`
    /// matched the emitting module). Must not re-run [`Self::should_suppress`]:
    /// this context's current file is unrelated to the diagnostic's origin at
    /// merge time, only the order-preserving dedup applies.
    pub(crate) fn push_collected(&mut self, diagnostic: Diagnostic) {
        self.push_deduplicated(diagnostic);
    }

    fn push_deduplicated(&mut self, diagnostic: Diagnostic) {
        // `diagnostics` is also mutated directly elsewhere (clear / mem::take /
        // truncate); if its length no longer matches what the index reflects, the
        // index is stale, so rebuild it from the current diagnostics before use.
        if self.diagnostic_keys_len != self.diagnostics.len() {
            self.diagnostic_keys = self
                .diagnostics
                .iter()
                .map(Self::diagnostic_dedup_key)
                .collect();
            self.diagnostic_keys_len = self.diagnostics.len();
        }

        if !self
            .diagnostic_keys
            .insert(Self::diagnostic_dedup_key(&diagnostic))
        {
            return;
        }

        self.diagnostics.push(diagnostic);
        self.diagnostic_keys_len = self.diagnostics.len();
    }

    fn diagnostic_dedup_key(
        diagnostic: &Diagnostic,
    ) -> (
        String,
        String,
        String,
        Option<surge_ts_diagnostics::TextSpan>,
    ) {
        (
            diagnostic.code.to_string(),
            diagnostic.file_name.clone(),
            diagnostic.message.clone(),
            diagnostic.span,
        )
    }

    pub(crate) fn push_utility_diagnostic_once(&mut self, diagnostic: Diagnostic) {
        let key = UtilityDiagnosticKey {
            code: diagnostic.code.to_string(),
            file_name: diagnostic.file_name.clone(),
            span: diagnostic.span.map(|span| (span.start, span.end)),
            message: diagnostic.message.clone(),
        };

        if self
            .utility_diagnostic_keys_baseline
            .as_ref()
            .is_some_and(|baseline| baseline.contains(&key))
        {
            return;
        }
        if self.utility_diagnostic_keys.insert(key) {
            self.push(diagnostic);
        }
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn truncate_diagnostics(&mut self, len: usize) {
        self.release_dedup_keys_for_tail(len);
        self.diagnostics.truncate(len);
    }

    /// Releases the push-dedup keys of `diagnostics[from..]` ahead of dropping
    /// them, so the index stays in sync instead of going stale.
    ///
    /// A stale index makes the next `push` rebuild it from every remaining
    /// diagnostic, allocating three `String`s apiece. The callers that shrink
    /// `diagnostics` are speculative probes that discard what they emitted, and
    /// they run per generic call and per named-type resolution — so leaving the
    /// index stale turned a file's diagnostic count into a per-probe cost.
    ///
    /// A no-op when the index is already stale (`clear_diagnostic_keys`, or a
    /// direct mutation of the field outside this type): claiming freshness there
    /// would suppress diagnostics whose keys the set never held.
    fn release_dedup_keys_for_tail(&mut self, from: usize) {
        if from >= self.diagnostics.len() || self.diagnostic_keys_len != self.diagnostics.len() {
            return;
        }
        for diagnostic in &self.diagnostics[from..] {
            self.diagnostic_keys
                .remove(&Self::diagnostic_dedup_key(diagnostic));
        }
        self.diagnostic_keys_len = from;
    }

    /// Drops the diagnostic at `index`, releasing its dedup key when the index
    /// is in sync. `indices` in the contextual-return frames are ascending, so
    /// callers walk them in reverse and earlier positions stay valid.
    fn remove_diagnostic_at(&mut self, index: usize) {
        if index >= self.diagnostics.len() {
            return;
        }
        if self.diagnostic_keys_len == self.diagnostics.len() {
            let key = Self::diagnostic_dedup_key(&self.diagnostics[index]);
            self.diagnostic_keys.remove(&key);
            self.diagnostic_keys_len -= 1;
        }
        self.diagnostics.remove(index);
    }

    /// Like [`truncate_diagnostics`] but also releases the
    /// `push_utility_diagnostic_once` keys recorded for the discarded diagnostics.
    /// Used by a speculative probe (e.g. resolving a generic's arguments to form a
    /// key) that discards its diagnostics: without releasing the once-guard, an
    /// authoritative re-resolution of the same type would be suppressed as a
    /// duplicate. Scoped narrowly so general truncation keeps its behavior.
    pub(crate) fn truncate_diagnostics_releasing_utility_keys(&mut self, len: usize) {
        if len < self.diagnostics.len() {
            for diagnostic in &self.diagnostics[len..] {
                let key = UtilityDiagnosticKey {
                    code: diagnostic.code.to_string(),
                    file_name: diagnostic.file_name.clone(),
                    span: diagnostic.span.map(|span| (span.start, span.end)),
                    message: diagnostic.message.clone(),
                };
                self.utility_diagnostic_keys.remove(&key);
            }
        }
        self.release_dedup_keys_for_tail(len);
        self.diagnostics.truncate(len);
    }

    pub(crate) fn finish(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub(crate) fn finish_with_stats(self) -> (Vec<Diagnostic>, CompatibilityStats) {
        (self.diagnostics, self.stats)
    }

    /// Whether `diagnostic` would reach the user from the file being checked.
    pub(crate) fn reports_diagnostic(&self, diagnostic: &Diagnostic) -> bool {
        !self.should_suppress(diagnostic)
    }

    fn should_suppress(&self, diagnostic: &Diagnostic) -> bool {
        if self.options.diagnostic_profile == DiagnosticProfile::Native {
            return false;
        }

        if self.current_file_kind == FileKind::GeneratedDeclaration {
            return diagnostic.code.to_string() != "surge::parser-error";
        }

        // Physical default-lib files are trusted upstream declarations: never
        // surface diagnostics that originate inside them, so unsupported lib
        // syntax cannot flood normal user diagnostics.
        if self.current_file_kind == FileKind::PhysicalDefaultLib {
            return true;
        }

        let code = diagnostic.code.to_string();
        if code.starts_with("surge::") {
            return true;
        }

        if self.options.skip_lib_check && self.current_file_kind.is_declaration() {
            return true;
        }

        false
    }

    fn record_suppressed(&mut self, diagnostic: &Diagnostic) {
        self.stats.suppressed_diagnostics_total += 1;

        if self.current_file_kind.is_declaration() {
            self.stats.suppressed_declaration_diagnostics_total += 1;
        }

        if is_rust_only_compat_diagnostic(&diagnostic.code.to_string()) {
            self.stats.suppressed_rust_only_diagnostics_total += 1;
        }
    }
}

pub(crate) fn convert_span(span: SyntaxTextSpan) -> DiagnosticTextSpan {
    DiagnosticTextSpan {
        start: span.start,
        end: span.end,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct UtilityDiagnosticKey {
    code: String,
    file_name: String,
    span: Option<(usize, usize)>,
    message: String,
}

fn is_rust_only_compat_diagnostic(code: &str) -> bool {
    code.starts_with("surge::")
}

#[cfg(test)]
mod diagnostic_dedup_index_tests {
    use super::{CheckerContext, CheckerOptions};
    use surge_ts_diagnostics::Diagnostic;

    fn context() -> CheckerContext {
        CheckerContext::new(
            "a.ts".to_string(),
            CheckerOptions::default(),
            surge_ts_types::fx::FxHashMap::default(),
        )
    }

    /// The shrinking paths release their dedup keys rather than invalidating the
    /// index. If a release ever misses a key, the index would claim to be in sync
    /// while holding a key for a diagnostic that is gone, and the re-emission a
    /// speculative probe's discard is supposed to allow would be swallowed —
    /// silently, as a missing diagnostic rather than a failure.
    #[test]
    fn truncated_diagnostic_can_be_pushed_again() {
        let mut ctx = context();
        ctx.push(Diagnostic::ts2304("Missing", "a.ts"));
        assert_eq!(ctx.diagnostics().len(), 1);

        ctx.truncate_diagnostics(0);
        assert_eq!(ctx.diagnostics().len(), 0);

        ctx.push(Diagnostic::ts2304("Missing", "a.ts"));
        assert_eq!(ctx.diagnostics().len(), 1);
    }

    /// Truncation to a non-zero length must release only the discarded tail's
    /// keys: the surviving prefix stays deduplicated.
    #[test]
    fn truncation_keeps_the_surviving_prefix_deduplicated() {
        let mut ctx = context();
        ctx.push(Diagnostic::ts2304("Kept", "a.ts"));
        ctx.push(Diagnostic::ts2304("Dropped", "a.ts"));
        assert_eq!(ctx.diagnostics().len(), 2);

        ctx.truncate_diagnostics(1);

        ctx.push(Diagnostic::ts2304("Kept", "a.ts"));
        assert_eq!(
            ctx.diagnostics().len(),
            1,
            "the surviving diagnostic's key must still suppress a repeat"
        );

        ctx.push(Diagnostic::ts2304("Dropped", "a.ts"));
        assert_eq!(
            ctx.diagnostics().len(),
            2,
            "the discarded diagnostic's key must have been released"
        );
    }

    /// `truncate_diagnostics_releasing_utility_keys` releases the push-dedup key
    /// alongside the once-guard key it is named for.
    #[test]
    fn utility_key_truncation_also_releases_the_push_dedup_key() {
        let mut ctx = context();
        ctx.push(Diagnostic::ts2304("Probe", "a.ts"));
        ctx.truncate_diagnostics_releasing_utility_keys(0);

        ctx.push(Diagnostic::ts2304("Probe", "a.ts"));
        assert_eq!(ctx.diagnostics().len(), 1);
    }

    /// A context whose index was dropped (`clear_diagnostic_keys`) is stale, and a
    /// shrink must leave it stale so the next push rebuilds — claiming freshness
    /// there would suppress diagnostics whose keys the set never held.
    #[test]
    fn shrinking_a_stale_index_still_rebuilds() {
        let mut ctx = context();
        ctx.push(Diagnostic::ts2304("Kept", "a.ts"));
        ctx.push(Diagnostic::ts2304("Dropped", "a.ts"));
        ctx.clear_diagnostic_keys();

        ctx.truncate_diagnostics(1);

        ctx.push(Diagnostic::ts2304("Kept", "a.ts"));
        assert_eq!(
            ctx.diagnostics().len(),
            1,
            "the rebuild must re-derive the surviving diagnostic's key"
        );
    }
}

/// What an import type's module (`import("m")`) offers the file that writes
/// it: its value (`typeof import("m")`) and its type exports.
pub(crate) struct ImportTypeTarget {
    pub(crate) namespace: Type,
    pub(crate) declarations: Arc<crate::symbols::TypeDeclarationTable>,
    pub(crate) scope: Option<Arc<crate::symbols::TypeDeclarationScope>>,
    /// The module is `export =`: its types are the assigned entity's members.
    pub(crate) export_assignment: bool,
    /// Declarations already given the module's scope, by qualifier.
    pub(crate) attached: FxHashMap<String, crate::symbols::TypeDeclarationHandle>,
}

impl std::fmt::Debug for ImportTypeTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImportTypeTarget")
            .field("namespace", &self.namespace)
            .field("export_assignment", &self.export_assignment)
            .finish_non_exhaustive()
    }
}
