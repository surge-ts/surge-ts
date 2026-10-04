use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedArrowFunction, ParsedArrowFunctionBody, ParsedFunctionDeclaration, ParsedTypeParameter,
};
use surge_ts_types::{FunctionType, Type, TypeCopyReason, with_type_copy_reason};

use super::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::context::convert_span;
use crate::flow::{FunctionFlowState, analyze_function_body_flow, collect_function_flow_facts};
use crate::infer::InferredExpression;
use crate::metrics::alloc_function_type;
use crate::program::record_program_timing;
use crate::symbols::{ScopeStack, SymbolTable};

mod body;
pub(crate) mod body_statements;
mod narrowing;
mod signature;

pub(crate) use body::*;
pub(crate) use body_statements::*;
pub(crate) use narrowing::*;
pub(crate) use signature::*;
/// A context for inferring a body during *signature collection* whose cache
/// writes cannot reach the check phase.
///
/// The inference resolves expressions with no instantiation in scope, so it
/// produces types the check phase must never see: on tRPC's `provider.ts` the
/// `forEach` callback parameter came out as an uninstantiated `NodePath<N, N>`
/// instead of `ASTPath<namedTypes.VariableDeclaration>`, and because the memo
/// serving it is keyed without the type-parameter scope, the check phase read
/// that back — `path.node` went unknown and 16 diagnostics both checkers report
/// disappeared.
///
/// So the shadow shares only what resolution needs to *read*: the declaration
/// tables and scopes, the ambient globals, the per-file scope and value maps. It
/// deliberately does not inherit the memo window (its own ordinal) or the
/// physical-interface instantiation caches. Modelled on the shadow
/// `collect_exportable_value_symbols` builds for the same reason.
///
/// **It does not fix that leak**, and the following were each measured not to
/// either (gate limited to the one file, so the whole -16/+9 delta is that
/// file's own inference):
///
/// * discarding the inferred type entirely — the delta survives, so it is the
///   act of inferring, not the type that gets published;
/// * a fresh program type store around the walk (`with_program_type_store`);
/// * `SURGE_DISABLE_CANONICAL_TYPE_STORE=1`;
/// * `SURGE_IFACE_MEMO_PROGRAM=0`, `SURGE_IFACE_MODULE_MEMO=0`,
///   `SURGE_DISABLE_SIG_CONTEXT_CACHE=1`;
/// * withholding *every* field this function shares, the scope and both
///   `Arc<Mutex<…>>` sets included.
///
/// Building the shadow but skipping the walk is neutral, so the carrier is
/// something the walk reaches that none of the above covers — most likely an
/// ordering or first-wins effect in the analysis pipeline rather than a cache.
/// Finding it needs a trace of where `ASTPath<namedTypes.VariableDeclaration>`
/// becomes `NodePath<N, N>`, not another isolation attempt.
fn body_inference_shadow_context(ctx: &CheckerContext) -> CheckerContext {
    let mut file_kinds = surge_ts_types::fx::FxHashMap::default();
    file_kinds.insert(ctx.file_name.to_string(), ctx.current_file_kind);
    let mut shadow = CheckerContext::new_with_shared_options(
        ctx.file_name.to_string(),
        std::sync::Arc::clone(&ctx.options),
        file_kinds,
    );
    shadow.timings = ctx.timings.clone();
    // Environment identity stays content-stable, as it must for any context that
    // resolves the same declarations: the deterministic stage counter and attempt
    // tag are inherited, and the memo map gets its own window ordinal so no entry
    // it writes can be served to the caller.
    shadow.resolution_stage_counter = ctx.resolution_stage_counter;
    shadow.environment_attempt = ctx.environment_attempt;
    shadow.replace_resolved_named_types(3);
    shadow.type_declarations = ctx.type_declarations.clone();
    shadow.type_declaration_scope = ctx.type_declaration_scope.clone();
    shadow.ambient_global_type_declarations = ctx.ambient_global_type_declarations.clone();
    shadow.ambient_global_symbols = ctx
        .ambient_global_symbols
        .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    shadow.module_scope_by_file = ctx.module_scope_by_file.clone();
    shadow.module_local_values_by_file = ctx.module_local_values_by_file.clone();
    shadow.import_type_namespaces = ctx.import_type_namespaces.clone();
    shadow.import_type_globals = ctx.import_type_globals.clone();
    shadow.namespace_member_prefix_stack = ctx.namespace_member_prefix_stack.clone();
    shadow.type_parameter_scopes = ctx.type_parameter_scopes.clone();
    shadow.type_parameter_constraint_scopes = ctx.type_parameter_constraint_scopes.clone();
    shadow
}

thread_local! {
    /// Body returns being forced on this thread, outermost first. Go's
    /// `getReturnTypeOfSignature` guards the same re-entry with
    /// `pushTypeResolution` and answers `any` for the cycle.
    static BODY_RETURNS_IN_PROGRESS: std::cell::RefCell<Vec<std::sync::Arc<str>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The next arrow checked takes its expected type only as its contextual
    /// type: the value is related where `elaborateError` does not descend into
    /// it — the operand of `x as T` / `<T>x` (compared by
    /// `checkAssertionDeferred`) or a branch of a conditional.
    pub(crate) static NEXT_ARROW_NOT_ELABORATED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// An unannotated declaration's return type, read from its body on first demand.
///
/// Go resolves a signature's return type lazily (`getReturnTypeOfSignature`,
/// checker.go:20321): the first read runs `getReturnTypeFromBody` and the answer
/// is cached on the signature, in the one environment the checker has. Walking
/// the body eagerly during surge's signature collection instead ran it under
/// module analysis's incomplete scopes, and that walk alone — its result
/// discarded — deleted diagnostics the check phase reports correctly. A
/// declaration nothing reads, like a codemod's default export, is now never
/// walked at all, which is also what Go does.
struct LazyBodyReturn {
    id: std::sync::Arc<str>,
    function: std::sync::Arc<ParsedFunctionDeclaration>,
    parameter_types: std::sync::Arc<[Type]>,
    file_name: std::sync::Arc<str>,
    environment: crate::context::DeclarationEnvironmentHandle,
    creation_scope: Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>,
    memo: std::sync::OnceLock<Type>,
    analysis_memo: std::sync::Mutex<Option<AnalysisForce>>,
}

type ModuleValueTables = surge_ts_types::fx::FxHashMap<std::sync::Arc<str>, std::sync::Arc<SymbolTable>>;
type ProgramModuleScopes = surge_ts_types::fx::FxHashMap<
    std::sync::Arc<str>,
    std::sync::Arc<crate::symbols::TypeDeclarationScope>,
>;

/// A body return forced during module analysis, with the program state the body
/// was read against: the module value tables and module scopes as published
/// then. They are held weakly so no later publication can take their address.
struct AnalysisForce {
    values: Option<std::sync::Weak<ModuleValueTables>>,
    scopes: Option<std::sync::Weak<ProgramModuleScopes>>,
    resolved: Type,
}

impl AnalysisForce {
    fn read_against(
        &self,
        values: &Option<std::sync::Arc<ModuleValueTables>>,
        scopes: &Option<std::sync::Arc<ProgramModuleScopes>>,
    ) -> bool {
        fn same<T>(held: &Option<std::sync::Weak<T>>, current: &Option<std::sync::Arc<T>>) -> bool {
            match (held, current) {
                (None, None) => true,
                (Some(held), Some(current)) => std::ptr::eq(held.as_ptr(), std::sync::Arc::as_ptr(current)),
                _ => false,
            }
        }
        same(&self.values, values) && same(&self.scopes, scopes)
    }
}

impl surge_ts_types::ResolveReference for LazyBodyReturn {
    fn resolve(&self) -> Type {
        if let Some(resolved) = self.memo.get() {
            return resolved.clone();
        }
        let re_entered = BODY_RETURNS_IN_PROGRESS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.iter().any(|id| **id == *self.id) {
                return true;
            }
            stack.push(self.id.clone());
            false
        });
        if re_entered {
            crate::program::note_expansion_degradation();
            return Type::Unknown;
        }
        struct PopInProgress;
        impl Drop for PopInProgress {
            fn drop(&mut self) {
                BODY_RETURNS_IN_PROGRESS.with(|stack| {
                    stack.borrow_mut().pop();
                });
            }
        }
        let _pop = PopInProgress;
        // Module analysis forces the same return over and over (every
        // `ReturnType<typeof f>` it resolves, every render of `f`'s type) while
        // the tables the body is read against stay put; only a republication of
        // them can change the answer.
        let analysis_state = (!crate::program::in_check_phase()).then(|| {
            (
                self.environment.published_module_local_values(),
                crate::program::program_module_scopes(),
            )
        });
        if let Some((values, scopes)) = &analysis_state {
            let held = self.analysis_memo.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(force) = held.as_ref()
                && force.read_against(values, scopes)
            {
                return force.resolved.clone();
            }
        }
        let epoch = crate::program::expansion_degradation_epoch();
        let Some(mut ctx) = self.environment.checker_context() else {
            return Type::Unknown;
        };
        ctx.set_file_name(self.file_name.to_string());
        if self.creation_scope.is_some() {
            ctx.type_declaration_scope = self.creation_scope.clone();
        }
        // An environment holds no value table, so the body's calls into its own
        // module (`prepareUrl` awaiting the imported `resultOf`) resolve against
        // the declaring module's values as the program holds them now.
        let scope = match self
            .environment
            .current_module_local_values(&self.file_name)
        {
            Some(values) => module_body_scope(&values, &ctx),
            None => module_body_scope(&SymbolTable::new(), &ctx),
        };
        let resolved =
            checked_body_return(&self.function, &self.parameter_types, scope, body_check_context(&ctx))
            .map(|returned| promised_if_async(returned, self.function.is_async, &mut ctx))
            .unwrap_or(Type::Unknown);
        // A force during module analysis runs with its scopes still incomplete,
        // so only the check phase's answer is the one every later read may keep.
        // Module analysis keeps its own answer only for the state it read, and
        // never one a recursion cut short or that fell back to the sentinel.
        if crate::program::in_check_phase() {
            let _ = self.memo.set(resolved.clone());
            *self.analysis_memo.lock().unwrap_or_else(|error| error.into_inner()) = None;
        } else if let Some((values, scopes)) = analysis_state
            && crate::program::expansion_degradation_epoch() == epoch
            && !resolved.is_unknown()
        {
            *self.analysis_memo.lock().unwrap_or_else(|error| error.into_inner()) = Some(AnalysisForce {
                values: values.as_ref().map(std::sync::Arc::downgrade),
                scopes: scopes.as_ref().map(std::sync::Arc::downgrade),
                resolved: resolved.clone(),
            });
        }
        resolved
    }
}

/// An unannotated class member's type, read from its initializer or getter
/// body on first demand, with `this` bound to the class instance — tsc's
/// `getWidenedTypeForVariableLikeDeclaration` for a property and the getter's
/// `getReturnTypeFromBody`. Whatever cannot be typed cleanly stays `any`, which
/// is what the member was before this inference existed.
struct LazyInferredMember {
    id: std::sync::Arc<str>,
    member: std::sync::Arc<surge_ts_syntax::ParsedInferredMember>,
    file_name: std::sync::Arc<str>,
    environment: crate::context::DeclarationEnvironmentHandle,
    creation_scope: Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>,
    memo: std::sync::OnceLock<Type>,
}

impl surge_ts_types::ResolveReference for LazyInferredMember {
    fn resolve(&self) -> Type {
        if let Some(resolved) = self.memo.get() {
            return resolved.clone();
        }
        let re_entered = BODY_RETURNS_IN_PROGRESS.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.iter().any(|id| **id == *self.id) {
                return true;
            }
            stack.push(self.id.clone());
            false
        });
        // tsc answers a member whose type depends on itself with `any`.
        if re_entered {
            return Type::Any;
        }
        struct PopInProgress;
        impl Drop for PopInProgress {
            fn drop(&mut self) {
                BODY_RETURNS_IN_PROGRESS.with(|stack| {
                    stack.borrow_mut().pop();
                });
            }
        }
        let _pop = PopInProgress;
        let Some(mut ctx) = self.environment.checker_context() else {
            return Type::Any;
        };
        ctx.set_file_name(self.file_name.to_string());
        if self.creation_scope.is_some() {
            ctx.type_declaration_scope = self.creation_scope.clone();
        }
        let resolved =
            infer_member_type(&self.member, &self.file_name, &self.environment, &mut ctx)
                .unwrap_or(Type::Any);
        if crate::program::in_check_phase() {
            let _ = self.memo.set(resolved.clone());
        }
        resolved
    }
}

fn infer_member_type(
    member: &surge_ts_syntax::ParsedInferredMember,
    file_name: &str,
    environment: &crate::context::DeclarationEnvironmentHandle,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // A script's declarations are globals, which lookup reaches without a table.
    let mut scope = match environment.current_module_local_values(file_name) {
        Some(values) => values.as_ref().clone(),
        None => ctx
            .symbols
            .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    };
    let instance = crate::infer::types::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: member.class_name.clone(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    scope.insert(
        "this".to_string(),
        crate::symbols::SymbolInfo {
            ty: instance.clone(),
            kind: crate::symbols::SymbolKind::Const,
            function_signature: None,
        },
    );
    match &member.source {
        // tsc's `getReturnTypeFromBody` widens what the body returns.
        surge_ts_syntax::ParsedInferredMemberSource::GetterBody(body) => {
            infer_statements_return(body, scope, ctx)
                .map(|ty| crate::checks::var::widen_nullable_type(&ty))
        }
        surge_ts_syntax::ParsedInferredMemberSource::MethodBody(method) => {
            let this_type = match &method.this_parameter_type {
                Some(written) => Some(crate::infer::types::map_parsed_type(written.clone(), ctx)),
                None if method.is_static => scope.get(&member.class_name).map(|class| class.ty.clone()),
                None => Some(instance),
            };
            let parameter_types: Vec<Type> = method
                .parameter_types
                .iter()
                .map(|parameter| crate::infer::types::map_parsed_type(parameter.clone(), ctx))
                .collect();
            let scope = match environment.current_module_local_values(file_name) {
                Some(values) => module_body_scope(&values, ctx),
                None => module_body_scope(&SymbolTable::new(), ctx),
            };
            checked_body_return_of(
                BodyParts {
                    parameters: &method.parameters,
                    body: &method.body,
                    is_generator: false,
                    is_async: method.is_async,
                    has_this_parameter: method.this_parameter_type.is_some(),
                    this_type,
                },
                &parameter_types,
                scope,
                body_check_context(ctx),
            )
            .map(|ty| crate::checks::var::widen_nullable_type(&ty))
            .map(|ty| promised_if_async(ty, method.is_async, ctx))
        }
        surge_ts_syntax::ParsedInferredMemberSource::Initializer(initializer) => {
            let mut shadow = body_inference_shadow_context(ctx);
            let crate::infer::InferredExpression::Known(ty) =
                crate::infer::infer_expression(initializer, &scope, &mut shadow)
            else {
                return None;
            };
            let ty = if member.keep_literal {
                ty
            } else {
                crate::checks::expr::widen_type(&ty)
            };
            let ty = crate::checks::var::widen_nullable_type(&ty);
            type_is_deeply_concrete(&ty).then_some(ty)
        }
        // tsc's `getWidenedTypeForAssignmentDeclaration` for `this.x = v`: the
        // union of what is assigned, widened, with `undefined` for a member
        // only methods assign; an empty array is `any[]`, and a member
        // assigned nothing but `null` or `undefined` is `any`.
        surge_ts_syntax::ParsedInferredMemberSource::ThisAssignments(assignments) => {
            // A member only methods assign is looked up in the base class
            // first (`getTypeOfPropertyInBaseClass`).
            if !assignments.in_constructor
                && let Some(base_type) = base_class_property_type(&member.class_name, &member.member_name, ctx)
            {
                return Some(base_type);
            }
            let mut shadow = body_inference_shadow_context(ctx);
            let mut types = Vec::with_capacity(assignments.values.len() + 1);
            for value in &assignments.values {
                if matches!(value, surge_ts_syntax::ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty()) {
                    types.push(Type::Array(Box::new(Type::Any)));
                    continue;
                }
                let crate::infer::InferredExpression::Known(ty) =
                    crate::infer::infer_expression(value, &scope, &mut shadow)
                else {
                    return None;
                };
                types.push(crate::checks::expr::widen_type(&ty));
            }
            if types.iter().all(|ty| matches!(ty, Type::Null | Type::Undefined)) {
                return Some(Type::Any);
            }
            if !assignments.in_constructor && surge_ts_types::strict_null_checks() {
                types.push(Type::Undefined);
            }
            let ty = surge_ts_types::union_type(types);
            type_is_deeply_concrete(&ty).then_some(ty)
        }
    }
}

/// The type of `member_name` on the first base of the class `class_name`.
fn base_class_property_type(class_name: &str, member_name: &str, ctx: &mut CheckerContext) -> Option<Type> {
    let Some(crate::symbols::TypeDeclarationInfo::Interface(class)) = ctx.lookup_type_declaration(class_name) else {
        return None;
    };
    if !class.is_class_instance {
        return None;
    }
    let base = class.body.extends.first()?.clone();
    let diagnostics_before = ctx.diagnostics().len();
    let base_type = crate::infer::types::map_parsed_type(surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(base)), ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    base_type.get_property_access_type(member_name).filter(|ty| !ty.is_unknown())
}

pub(crate) fn inferred_member_reference(
    member: std::sync::Arc<surge_ts_syntax::ParsedInferredMember>,
    ctx: &mut CheckerContext,
) -> Type {
    let id: std::sync::Arc<str> = std::sync::Arc::from(format!(
        "{}{INFERRED_MEMBER_ID_TAG}{}.{}\u{0}{}",
        ctx.file_name, member.class_name, member.member_name, member.member_start
    ));
    let display = format!("{}[\"{}\"]", member.class_name, member.member_name);
    let reference = surge_ts_types::TypeReference::new(
        id.clone(),
        display,
        Vec::new(),
        std::sync::Arc::new(LazyInferredMember {
            id,
            member,
            file_name: std::sync::Arc::from(ctx.file_name.as_ref()),
            environment: ctx.declaration_environment(),
            creation_scope: ctx.type_declaration_scope.clone(),
            memo: std::sync::OnceLock::new(),
        }),
    );
    Type::Reference(reference.rendered_structurally())
}

const BODY_RETURN_ID_TAG: &str = "\u{0}body-return\u{0}";
const INFERRED_MEMBER_ID_TAG: &str = "\u{0}inferred-member\u{0}";

/// What a call through an unannotated declaration evaluates to: the resolved
/// return type, never the lazy reference standing in for it. Go's
/// `getReturnTypeOfSignature` only ever hands back a settled type, and a
/// reference that resolves to the sentinel is not recognised as one by the many
/// sites that test the unpeeled type — destructuring such a result indexed a
/// receiver rendered `unknown` (TS7053) instead of staying silent.
pub(crate) fn settle_call_result(
    expression: &surge_ts_syntax::ParsedExpression,
    result: InferredExpression,
) -> InferredExpression {
    use surge_ts_syntax::ParsedExpression as E;
    // A read of an inferred class member or a lazily typed variable settles it
    // the same way.
    if !matches!(
        expression,
        E::Identifier { .. }
            | E::Call { .. }
            | E::PropertyCall { .. }
            | E::OptionalPropertyCall { .. }
            | E::ExpressionCall { .. }
            | E::OptionalCall { .. }
            | E::PropertyAccess { .. }
            | E::OptionalPropertyAccess { .. }
    ) {
        return result;
    }
    let InferredExpression::Known(ty) = result else {
        return result;
    };
    InferredExpression::Known(settle_body_return(ty))
}

/// Whether a function's return was read from its body — a declaration's or a
/// class method's — and so carries the fresh literal types its `return`s
/// produced.
pub(crate) fn is_body_inferred_return(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference)
        if reference.id.contains(BODY_RETURN_ID_TAG) || reference.id.contains(INFERRED_MEMBER_ID_TAG))
}

fn is_settled_on_read(reference: &surge_ts_types::TypeReference) -> bool {
    reference.id.contains(BODY_RETURN_ID_TAG)
        || reference.id.contains(INFERRED_MEMBER_ID_TAG)
        || reference.id.contains(crate::modules::LAZY_INITIALIZER_ID_TAG)
}

/// A member or body-return type as a read of it sees it: resolved, never the
/// lazy reference standing in for it.
pub(crate) fn settle_lazy_read(ty: Type) -> Type {
    settle_body_return(ty)
}

fn settle_body_return(ty: Type) -> Type {
    match &ty {
        Type::Reference(reference) if is_settled_on_read(reference) => reference.resolve(),
        Type::Union(union)
            if union.types().iter().any(|member| {
                matches!(member, Type::Reference(reference) if is_settled_on_read(reference))
            }) =>
        {
            surge_ts_types::union_type(
                union.types().iter().cloned().map(settle_body_return).collect(),
            )
        }
        _ => ty,
    }
}

fn lazy_body_return_reference(
    function: &ParsedFunctionDeclaration,
    parameter_types: &[Type],
    ctx: &mut CheckerContext,
) -> Type {
    let start = function.name_span.map_or(0, |span| span.start);
    let id: std::sync::Arc<str> = std::sync::Arc::from(format!(
        "{}{BODY_RETURN_ID_TAG}{}\u{0}{start}",
        ctx.file_name, function.name
    ));
    let display = format!("ReturnType<typeof {}>", function.name);
    let reference = surge_ts_types::TypeReference::new(
        id.clone(),
        display,
        Vec::new(),
        std::sync::Arc::new(LazyBodyReturn {
            id,
            function: std::sync::Arc::new(function.clone()),
            parameter_types: std::sync::Arc::from(parameter_types),
            file_name: std::sync::Arc::from(ctx.file_name.as_ref()),
            environment: ctx.declaration_environment(),
            creation_scope: ctx.type_declaration_scope.clone(),
            memo: std::sync::OnceLock::new(),
            analysis_memo: std::sync::Mutex::new(None),
        }),
    );
    // tsc renders the inferred type, not a name for it.
    Type::Reference(reference.rendered_structurally())
}

/// Whether a declaration's return type comes from its body at all: it is
/// unannotated, has a body to read, and is not a generator (whose result is a
/// `Generator`, not what its body completes with).
pub(crate) fn return_type_comes_from_body(function: &ParsedFunctionDeclaration) -> bool {
    function.return_type.is_none()
        && !function.is_declare
        && function.has_body
        && !function.is_generator
        && !function.body.is_empty()
}

/// A generic declaration's unannotated return for one call: the body read with
/// the declaration's type parameters bound to the call's type arguments and its
/// parameters at their instantiated types. Go computes the return once over the
/// type parameters and instantiates it per call (`instantiateType(
/// getReturnTypeOfSignature(sig.target), sig.mapper)`); a body surge reads
/// lazily has no substitutable form, so each call reads it under its own
/// bindings, which is the same type. `None` keeps the sentinel: a type argument
/// the call left unresolved, a recursive call, or a body not typed cleanly.
pub(crate) fn instantiated_body_return(
    source: &crate::symbols::BodyReturnSource,
    substitution: &crate::infer::TypeParameterSubstitution,
    parameter_types: &[Type],
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let function = &source.function;
    let start = function.name_span.map_or(0, |span| span.start);
    let id: std::sync::Arc<str> = std::sync::Arc::from(format!(
        "{}{BODY_RETURN_ID_TAG}{}\u{0}{start}",
        ctx.file_name, function.name
    ));
    let re_entered = BODY_RETURNS_IN_PROGRESS.with(|stack| {
        let mut stack = stack.borrow_mut();
        if stack.iter().any(|entry| **entry == *id) {
            return true;
        }
        stack.push(id.clone());
        false
    });
    if re_entered {
        crate::program::note_expansion_degradation();
        return None;
    }
    struct PopInProgress;
    impl Drop for PopInProgress {
        fn drop(&mut self) {
            BODY_RETURNS_IN_PROGRESS.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
    let _pop = PopInProgress;

    let mut bindings = std::collections::HashMap::new();
    for type_parameter in &function.type_parameters {
        let bound = substitution
            .get(&type_parameter.name)
            .filter(|ty| !ty.is_unknown() && !substitution.is_placeholder(&type_parameter.name))?;
        bindings.insert(type_parameter.name.clone(), bound.clone());
    }
    // The body names what its own module has in scope, not the caller's.
    let file_name = ctx.file_name.clone();
    let latest_values = ctx
        .declaration_environment_store
        .published_module_local_values()
        .and_then(|values| values.get(file_name.as_str()).cloned());
    let scope = match latest_values.or_else(|| ctx.module_local_values_for_file(&file_name)) {
        Some(values) => module_body_scope(&values, ctx),
        None => ctx
            .symbols
            .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    };
    let saved_declarations = std::mem::take(&mut ctx.type_declarations);
    let saved_scope = std::mem::replace(
        &mut ctx.type_declaration_scope,
        crate::program::program_module_scope_for_file(&file_name),
    );
    ctx.type_parameter_scopes.push(std::sync::Arc::new(bindings));
    ctx.type_parameter_constraint_scopes
        .push(crate::context::empty_constraint_scope());
    // The type-parameter scopes are the whole environment the body is read in
    // beyond its own module; a constraint scope in play is not captured by
    // them, and only the check phase's answer is final.
    let cache_key = (crate::program::in_check_phase()
        && ctx
            .type_parameter_constraint_scopes
            .iter()
            .all(|scope| scope.is_empty()))
    .then(|| {
        ctx.type_parameter_scopes
            .iter()
            .map(|scope| {
                let mut entries: Vec<(String, Type)> = scope
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.clone()))
                    .collect();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                entries
            })
            .collect::<Vec<_>>()
    });
    let cached = cache_key.as_ref().and_then(|key| {
        source
            .instantiations
            .lock()
            .ok()?
            .iter()
            .find(|(bound, _)| bound == key)
            .map(|(_, ty)| ty.clone())
    });
    let inferred = match cached {
        Some(cached) => Some(cached),
        None => {
            let epoch = crate::program::expansion_degradation_epoch();
            let inferred =
                checked_body_return(function, parameter_types, scope, body_check_context(ctx));
            if let (Some(key), Some(ty)) = (cache_key, inferred.as_ref())
                && crate::program::expansion_degradation_epoch() == epoch
                && let Ok(mut instantiations) = source.instantiations.lock()
            {
                instantiations.push((key, ty.clone()));
            }
            inferred
        }
    };
    ctx.pop_type_parameter_scope();
    ctx.type_declaration_scope = saved_scope;
    ctx.type_declarations = saved_declarations;
    inferred.map(|returned| promised_if_async(returned, function.is_async, ctx))
}

/// The scope a module declaration's body is checked in from outside its own
/// check: the module's values over the ambient globals, rooted the way the
/// check phase roots the module file (`check_program_file`).
fn module_body_scope(module_values: &SymbolTable, ctx: &CheckerContext) -> SymbolTable {
    let globals = crate::program::program_ambient_globals()
        .filter(|_| crate::program::in_check_phase())
        .unwrap_or_else(|| {
            std::sync::Arc::new(
                ctx.ambient_global_symbols
                    .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
            )
        });
    let mut scope = SymbolTable::file_check_root(globals);
    for (name, symbol) in module_values.iter_shared() {
        let _ = scope.insert_shared(name.clone(), symbol.clone());
    }
    scope
}

/// A context the body checker can run a declaration's body in from outside
/// that declaration's own check: [`body_inference_shadow_context`] — its own
/// caches, stores and resolution memo, so nothing resolved under this read's
/// bindings reaches another — plus the program's declaration tables a body
/// check reads and a type-only inference did not (ambient modules,
/// augmentations, the JSX and namespace registries).
fn body_check_context(ctx: &CheckerContext) -> CheckerContext {
    let mut shadow = body_inference_shadow_context(ctx);
    if let Some(latest) = ctx.declaration_environment_store.published_module_local_values() {
        shadow.module_local_values_by_file = latest;
    }
    shadow.current_file_kind = ctx.current_file_kind;
    shadow.ambient_modules = ctx.ambient_modules.clone();
    shadow.ambient_file_type_scopes = ctx.ambient_file_type_scopes.clone();
    shadow.module_augmentations = ctx.module_augmentations.clone();
    shadow.module_file_index_by_identity = ctx.module_file_index_by_identity.clone();
    shadow.module_value_fallback = ctx.module_value_fallback.clone();
    shadow.jsx_namespace_modules = ctx.jsx_namespace_modules.clone();
    shadow.namespace_registry = ctx.namespace_registry.clone();
    shadow.block_scoped_globals = ctx.block_scoped_globals.clone();
    shadow.umd_global_names = ctx.umd_global_names.clone();
    shadow
}

/// tsc's `getReturnTypeFromBody` over the real body check: the union of what
/// the body's `return`s produced, literals widened, `undefined` when its end is
/// reachable, and `void` when it returns nothing — the rule an unannotated
/// block-bodied arrow already follows. The check runs in `shadow`, a context of
/// its own, so none of its diagnostics or state reach the caller. A return that
/// is the degradation sentinel keeps the whole answer at it.
fn checked_body_return(
    function: &ParsedFunctionDeclaration,
    parameter_types: &[Type],
    scope: SymbolTable,
    shadow: CheckerContext,
) -> Option<Type> {
    checked_body_return_of(
        BodyParts {
            parameters: &function.parameters,
            body: &function.body,
            is_generator: function.is_generator,
            is_async: function.is_async,
            has_this_parameter: function.has_this_parameter,
            this_type: None,
        },
        parameter_types,
        scope,
        shadow,
    )
}

/// The parts of a function-like declaration its body is checked from.
struct BodyParts<'a> {
    parameters: &'a [surge_ts_syntax::ParsedFunctionParameter],
    body: &'a [surge_ts_syntax::ParsedFunctionBodyStatement],
    is_generator: bool,
    is_async: bool,
    has_this_parameter: bool,
    this_type: Option<Type>,
}

fn checked_body_return_of(
    parts: BodyParts<'_>,
    parameter_types: &[Type],
    scope: SymbolTable,
    mut shadow: CheckerContext,
) -> Option<Type> {
    shadow.set_symbols(scope);
    let function_type = FunctionType::new(
        parameter_types.to_vec(),
        Type::Unknown,
        parts.parameters.last().is_some_and(|parameter| parameter.rest),
        signature::required_parameter_count(parts.parameters),
    );
    let ((), captured) = signature::capture_declaration_body_returns(|| {
        signature::check_function_body_with_signature_and_this(
            None,
            parts.parameters.to_vec(),
            parts.body.to_vec(),
            &function_type,
            &[],
            None,
            false,
            None,
            parts.this_type,
            false,
            parts.is_generator,
            parts.is_async,
            parts.has_this_parameter,
            &mut shadow,
        )
    });
    let captured = captured?;
    if captured.returned.iter().any(Type::is_degraded) {
        return None;
    }
    if captured.returned.is_empty() {
        return Some(Type::Void);
    }
    Some(declaration_return_type(captured))
}

/// `getReturnTypeFromBody` over an unannotated declaration's returns: each one
/// widened (`getWidenedType`), `undefined` for a reachable end, the union
/// subtype-reduced, and a literal widened only when the union is that one
/// fresh unit type (`getWidenedLiteralLikeTypeForContextualReturnTypeIfNeeded`
/// with no contextual type) — a union of literals stays one.
fn declaration_return_type(captured: signature::CapturedReturns) -> Type {
    let fresh = captured.forms.iter().any(|form| form.fresh);
    let mut members: Vec<(Type, surge_ts_types::LiteralShape)> =
        if captured.forms.len() == captured.returned.len() {
            captured.forms.into_iter().map(|form| (form.widened, form.shape)).collect()
        } else {
            captured
                .returned
                .into_iter()
                .map(|ty| (ty, surge_ts_types::LiteralShape::Regular))
                .collect()
        };
    if captured.falls_through {
        members.push((Type::Undefined, surge_ts_types::LiteralShape::Regular));
    }
    // The reduction relates the members, resolving what they reference; a read
    // before the check phase is transient (only the check phase's answer is
    // kept) and runs where those names may not resolve yet.
    let union = if crate::program::in_check_phase() {
        surge_ts_types::subtype_reduced_union(members)
    } else {
        surge_ts_types::union_type(members.into_iter().map(|(ty, _)| ty).collect())
    };
    let unit_literal = match &union {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => true,
        Type::Reference(reference) => reference.enum_base.is_some(),
        _ => false,
    };
    if unit_literal && fresh {
        crate::checks::expr::widen_type(&union)
    } else {
        union
    }
}

fn inferred_declaration_return_type(
    function: &ParsedFunctionDeclaration,
    function_type: &FunctionType,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !return_type_comes_from_body(function) {
        return None;
    }
    let scope = ctx
        .symbols
        .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    infer_body_return(function, function_type.parameters(), scope, ctx)
        .map(|returned| promised_if_async(returned, function.is_async, ctx))
}

/// `getReturnTypeFromBody` for a declaration none of whose `return`s carries
/// a value: `void` (a declaration never `mayReturnNever`), promised when async.
fn no_value_return_type(function: &ParsedFunctionDeclaration, ctx: &mut CheckerContext) -> Option<Type> {
    let reads_body =
        function.return_type.is_none() && !function.is_declare && function.has_body && !function.is_generator;
    (reads_body && !crate::flow::analyze_function_body_flow(&function.body).contains_return_with_value)
        .then(|| promised_if_async(Type::Void, function.is_async, ctx))
}

/// tsc's `createPromiseReturnType`: an async function's inferred return is a
/// promise of what its body's `return`s await to. With the lib promise
/// collapsed to its value the awaited value is the answer as it is.
fn promised_if_async(returned: Type, is_async: bool, ctx: &mut CheckerContext) -> Type {
    if !is_async || !crate::checks::call::promise_nominal_enabled() {
        return returned;
    }
    crate::checks::call::promise_of(&crate::checks::call::awaited_type(&returned), ctx)
}

/// The body walk behind [`inferred_declaration_return_type`] and
/// [`LazyBodyReturn`]: every reachable `return`, unioned and widened, published
/// only when it is concrete at every depth.
fn infer_body_return(
    function: &ParsedFunctionDeclaration,
    parameter_types: &[Type],
    mut scope: SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    for (parameter, parameter_type) in function.parameters.iter().zip(parameter_types.iter()) {
        if let Some(name) = signature::written_binding_names(std::slice::from_ref(parameter))
            .into_iter()
            .flatten()
            .next()
        {
            scope.insert(
                name,
                crate::symbols::SymbolInfo {
                    ty: parameter_type.clone(),
                    kind: crate::symbols::SymbolKind::Parameter,
                    function_signature: None,
                },
            );
        }
    }
    infer_statements_return(&function.body, scope, ctx)
}

/// [`infer_body_return`] over a body with its bindings already in `scope`.
fn infer_statements_return(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    mut scope: SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    use surge_ts_syntax::ParsedFunctionBodyStatement as BodyStatement;

    // Deeply concrete: a shallow check passes an object whose *member* is the
    // degradation sentinel, and publishing that as a declaration's return type
    // hands every consumer a shape that silences its own checks. tsc has no such
    // condition — it has no sentinel — so this is surge's own guard, and it is
    // what makes inferring a branchy body affordable.
    let usable_type = |ty: &Type| type_is_deeply_concrete(ty);
    // The whole walk runs against a shadow, so nothing it resolves is published
    // where the check phase can read it. Its diagnostics die with it.
    let mut shadow = body_inference_shadow_context(ctx);
    let mut returned: Vec<Type> = Vec::new();
    let mut usable = true;

    // Every `return` the body can reach, branches included: tsc's
    // `getReturnTypeFromBody` unions them all, and stopping at the first
    // statement that carries control flow left an early return
    // (`if (!opts.connectionParams) return url;`) — the shape tRPC's
    // `prepareUrl` has — with no inferred type at all.
    fn collect(
        body: &[BodyStatement],
        scope: &mut crate::symbols::SymbolTable,
        returned: &mut Vec<Type>,
        usable: &mut bool,
        usable_type: &impl Fn(&Type) -> bool,
        ctx: &mut CheckerContext,
    ) {
        for statement in body {
            if !*usable {
                return;
            }
            match statement {
                BodyStatement::VariableDeclaration(variable) => {
                    if variable.declared_type.is_none()
                        && let Some(initializer) = variable.initializer.as_ref()
                        && let crate::infer::InferredExpression::Known(ty) =
                            crate::infer::infer_expression(initializer, scope, ctx)
                    {
                        scope.insert(
                            variable.name.clone(),
                            crate::symbols::SymbolInfo {
                                ty,
                                kind: crate::symbols::SymbolKind::Const,
                                function_signature: None,
                            },
                        );
                    }
                }
                BodyStatement::Return(statement) => match statement.expression.as_ref() {
                    Some(expression) => {
                        match crate::infer::infer_expression(expression, scope, ctx) {
                            crate::infer::InferredExpression::Known(ty) if usable_type(&ty) => {
                                returned.push(ty)
                            }
                            _ => *usable = false,
                        }
                    }
                    None => returned.push(Type::Undefined),
                },
                BodyStatement::Block(block) => {
                    collect(block, scope, returned, usable, usable_type, ctx)
                }
                BodyStatement::If(if_statement) => {
                    collect(
                        &if_statement.then_body,
                        scope,
                        returned,
                        usable,
                        usable_type,
                        ctx,
                    );
                    collect(
                        &if_statement.else_body,
                        scope,
                        returned,
                        usable,
                        usable_type,
                        ctx,
                    );
                }
                BodyStatement::While(while_statement) => collect(
                    &while_statement.body,
                    scope,
                    returned,
                    usable,
                    usable_type,
                    ctx,
                ),
                BodyStatement::ForOf(for_of) => {
                    collect(&for_of.body, scope, returned, usable, usable_type, ctx)
                }
                BodyStatement::Switch(switch_statement) => {
                    for case in &switch_statement.cases {
                        collect(&case.consequent, scope, returned, usable, usable_type, ctx);
                    }
                }
                BodyStatement::Try(try_statement) => {
                    collect(
                        &try_statement.block,
                        scope,
                        returned,
                        usable,
                        usable_type,
                        ctx,
                    );
                    if let Some(handler) = &try_statement.handler {
                        collect(&handler.body, scope, returned, usable, usable_type, ctx);
                    }
                    collect(
                        &try_statement.finalizer,
                        scope,
                        returned,
                        usable,
                        usable_type,
                        ctx,
                    );
                }
                BodyStatement::Throw(_)
                | BodyStatement::Assignment(_)
                | BodyStatement::ThisPropertyAssignment(_)
                | BodyStatement::MemberAssignment(_)
                | BodyStatement::Expression(_)
                | BodyStatement::Function(_)
                | BodyStatement::TypeAlias(_)
                | BodyStatement::Interface(_)
                | BodyStatement::Class(_)
                | BodyStatement::Continue
                | BodyStatement::Break => {}
            }
        }
    }

    collect(
        body,
        &mut scope,
        &mut returned,
        &mut usable,
        &usable_type,
        &mut shadow,
    );
    drop(shadow);
    // A body that returns nothing is `void` (`getReturnTypeFromBody`).
    if usable && returned.is_empty() {
        return Some(Type::Void);
    }
    // tsc widens the fresh literals a returned expression carries
    // (`getReturnTypeFromBody` runs the result through the widening machinery),
    // so `return { importName: "trpc" }` is `{ importName: string }`. Freezing
    // the literal instead publishes a type far narrower than the declaration's,
    // and every consumer compares against the wrong one.
    let inferred = usable
        .then(|| crate::checks::expr::widen_type(&surge_ts_types::union_type(returned)))
        .filter(usable_type)?;
    Some(inferred)
}

/// Whether a type carries no permissive or unresolved part anywhere: neither
/// surge's degradation sentinel nor `any`, at any depth.
///
/// Both halves are load-bearing for publishing an *inferred* return type. The
/// sentinel silences a consumer's own checks, and so does a nested `any`: on
/// tRPC's `packages/upgrade` the shallow check let through object returns whose
/// members were `any`, and 16 diagnostics surge reported correctly — and tsc
/// reports too — disappeared, because the consumer's receiver started answering
/// every member. When the body cannot be typed this cleanly the declaration keeps
/// the sentinel, which is the behavior without this inference at all.
fn type_is_deeply_concrete(ty: &Type) -> bool {
    fn permissive(ty: &Type, depth: usize) -> bool {
        if depth > 8 {
            return true;
        }
        match ty {
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::ErrorType => true,
            Type::TypeParameter(_) => true,
            Type::Array(element) => permissive(element, depth + 1),
            Type::Tuple(elements) => elements
                .iter()
                .any(|element| permissive(element, depth + 1)),
            Type::Union(union) => union
                .types()
                .iter()
                .any(|member| permissive(member, depth + 1)),
            Type::Reference(reference) => reference
                .arguments
                .iter()
                .any(|argument| permissive(argument, depth + 1)),
            Type::Function(function) => {
                function
                    .parameters()
                    .iter()
                    .any(|parameter| permissive(parameter, depth + 1))
                    || permissive(function.return_type(), depth + 1)
            }
            Type::Object(object) => {
                object
                    .properties
                    .values()
                    .any(|property| permissive(&property.ty, depth + 1))
                    || object
                        .string_index_type
                        .as_deref()
                        .is_some_and(|index| permissive(index, depth + 1))
            }
            _ => false,
        }
    }
    !permissive(ty, 0)
}

/// How an unannotated declaration's return type (and an unannotated class
/// member's type) is read from its body, by `SURGE_INFER_DECLARATION_RETURN_TYPES`:
///
/// - unset, empty or `lazy` (the default): a [`LazyBodyReturn`] /
///   [`LazyInferredMember`] resolved on first demand, as Go's
///   `getReturnTypeOfSignature` does. `lazy:<substring>` limits it to files
///   whose name contains the substring, to bisect which declaration moves a
///   diagnostic.
/// - `0` or `off`: no inference; the declaration keeps the sentinel and the
///   member keeps `any`.
/// - `1`, or any other value as a file substring: the eager walk during
///   signature collection. Kept for comparison only — running `infer_expression`
///   there, under module analysis's incomplete scopes, deleted 16 trpc
///   diagnostics surge otherwise reports (measured 2026-09-21), which is what the
///   lazy form exists to avoid.
pub(crate) fn lazy_body_returns(file_name: &str) -> bool {
    static SETTING: std::sync::OnceLock<Option<Option<String>>> = std::sync::OnceLock::new();
    let setting = SETTING.get_or_init(|| {
        match std::env::var("SURGE_INFER_DECLARATION_RETURN_TYPES")
            .ok()
            .as_deref()
        {
            None | Some("") | Some("lazy") => Some(None),
            Some(value) => value
                .strip_prefix("lazy:")
                .map(|filter| Some(filter.to_string())),
        }
    });
    match setting {
        None => false,
        Some(None) => true,
        Some(Some(filter)) => file_name.contains(filter.as_str()),
    }
}

fn infer_declaration_return_types(file_name: &str) -> bool {
    static SETTING: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    match SETTING
        .get_or_init(|| std::env::var("SURGE_INFER_DECLARATION_RETURN_TYPES").ok())
        .as_deref()
    {
        None | Some("") | Some("lazy") | Some("0") | Some("off") => false,
        Some(value) if value.starts_with("lazy:") => false,
        Some("1") => true,
        Some(filter) => file_name.contains(filter),
    }
}

pub(crate) fn collect_function_declaration_signature(
    function: &ParsedFunctionDeclaration,
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
    allow_lazy_dependency_signature: bool,
) -> FunctionType {
    // The signature resolves against `symbols`; whatever table the context
    // held goes back afterwards, or every declaration collected after this one
    // would resolve names against an empty table.
    let temp_symbols = std::mem::take(symbols);
    let outer_symbols = std::mem::take(&mut ctx.symbols);
    ctx.set_symbols(temp_symbols);

    // Establish the function's own type-parameter scope (with constraints) while
    // mapping the signature, so a constrained parameter such as `K extends keyof T`
    // is visible when its body resolves a `T[K]` indexed access (otherwise a false
    // TS2536, e.g. the lib `addEventListener<K extends keyof WindowEventMap>(…:
    // WindowEventMap[K])`).
    //
    // Scoped to *generic* `declare` functions only. For a `declare` function this
    // collected signature is the authoritative resolution (no body is checked
    // afterwards), so it must resolve its constrained indexed accesses here. A
    // non-`declare` function is re-checked authoritatively by
    // `check_function_declaration` under its own scope; resolving its (often
    // cross-module) signature concretely *here* instead changed how generic
    // instantiations were collected and surfaced assignability false positives, so
    // its pre-pass signature is left as-is. The empty-scope case is also skipped: a
    // pushed empty scope makes `type_parameter_scopes` non-empty and flips the
    // `concrete_instantiation` short-circuit.
    crate::program::record_program_counter(|c| c.function_signatures_indexed_count += 1);
    let map_signature = |ctx: &mut CheckerContext| {
        static LAZY_DEPENDENCY_SIGNATURES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let lazy_dependency_signatures = *LAZY_DEPENDENCY_SIGNATURES
            .get_or_init(|| std::env::var_os("SURGE_EAGER_DEPENDENCY_SIGNATURES").is_none());
        // Deferral stays scoped to installed-package declarations. Extending it
        // to the physical `lib.*.d.ts` set looks attractive (largest
        // declaration surface, small slice of it used) and is diagnostically
        // free, but it is a measured loss: building lazy references for the
        // whole lib surface costs more than the deferred mapping saves.
        // See docs/perf/LOADER-PARSE-HANDOFF.md § Not taken.
        let signature = if ctx.current_file_kind == crate::context::FileKind::DependencyDeclaration
            && allow_lazy_dependency_signature
            && lazy_dependency_signatures
        {
            map_lazy_dependency_function_signature(function, ctx)
        } else {
            map_function_signature(
                &function.parameters,
                function.return_type.as_ref(),
                &function.type_parameters,
                None,
                ctx,
            )
        };
        signature::with_written_this_type(
            signature,
            function.this_parameter_type.as_ref(),
            &function.type_parameters,
            ctx,
        )
    };
    let full_signature = full_signature_of(function.full_signature.as_ref(), ctx);
    let function_type = if let Some(signature) = &full_signature {
        signature.clone()
    } else if function.is_declare && !function.type_parameters.is_empty() {
        with_type_parameter_scope(&function.type_parameters, ctx, map_signature)
    } else {
        map_signature(ctx)
    };

    // A declaration whose body can mention a type parameter is left out — its
    // own, or an enclosing generic's. Go computes such a return once and
    // instantiates it per call (`instantiateType(getReturnTypeOfSignature(
    // sig.target), sig.mapper)`); a `LazyBodyReturn` is opaque to that
    // substitution. `createDeferred<TValue>()` called where the context supplies
    // `TValue` came back with a bare `TValue` and assignment narrowing dropped
    // the member it should have kept; a component declared inside
    // `createHydrationStreamProvider<TShape>` kept a bare `TShape` in its props.
    let lazy_return = full_signature.is_none()
        && lazy_body_returns(&ctx.file_name)
        && function.type_parameters.is_empty()
        && ctx
            .type_parameter_scopes
            .iter()
            .all(|scope| scope.is_empty())
        && return_type_comes_from_body(function);
    let function_type = match full_signature
        .is_none()
        .then(|| {
            lazy_return
                .then(|| lazy_body_return_reference(function, function_type.parameters(), ctx))
                .or_else(|| {
                    infer_declaration_return_types(&ctx.file_name)
                        .then(|| inferred_declaration_return_type(function, &function_type, ctx))
                        .flatten()
                })
                .or_else(|| no_value_return_type(function, ctx))
        })
        .flatten()
    {
        Some(return_type) => FunctionType::new(
            function_type.parameters().to_vec(),
            return_type,
            function_type.is_variadic(),
            function_type.required_parameter_count(),
        )
        .with_parameter_names(signature::written_binding_names(&function.parameters))
        .with_this_type(function_type.this_type().cloned()),
        None => function_type,
    };

    *symbols = std::mem::take(&mut ctx.symbols);
    ctx.set_symbols(outer_symbols);

    let signature_info =
        function_declaration_signature_info(function, &function_type, symbols, ctx);
    let duplicate = register_function_signature(
        function.name.clone(),
        with_type_copy_reason(TypeCopyReason::FunctionBodySetup, || function_type.clone()),
        Some(signature_info),
        symbols,
        false,
        function.has_body,
        // `allow_lazy_dependency_signature` is "the file declares this name
        // once"; a body beside other declarations is their implementation.
        function.has_body && !allow_lazy_dependency_signature,
    );

    if duplicate {
        let diagnostic = Diagnostic::ts2393(ctx.file_name.clone());
        let diagnostic = match function.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };

        ctx.push(diagnostic);

        if let Some(first_span) = symbols.take_declaration_span(&function.name) {
            ctx.push(Diagnostic::ts2393(ctx.file_name.clone()).with_span(convert_span(first_span)));
        }
    } else if let Some(span) = function.name_span {
        symbols.record_declaration_span(&function.name, span);
    }

    function_type
}

/// tsc's `getSignatureOfFullSignatureType`: the single call signature of a
/// JavaScript declaration's `@type` full signature, which is the
/// declaration's own signature. The tag's type is reported where JSDoc types
/// are checked, not here.
pub(crate) fn full_signature_of(
    full_signature: Option<&surge_ts_syntax::ParsedType>,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    let full_signature = full_signature?;
    if !surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        return None;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let ty = crate::infer::map_parsed_type(full_signature.clone(), ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    single_call_signature_of(&ty)
}

fn single_call_signature_of(ty: &Type) -> Option<FunctionType> {
    let single = |function: &FunctionType| {
        function.overloads().is_none_or(<[FunctionType]>::is_empty).then(|| function.clone())
    };
    match ty {
        Type::Function(function) => single(function),
        Type::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.construct_signature().is_none() =>
        {
            object.call_signature().and_then(single)
        }
        Type::Reference(_) => match ty.peeled() {
            peeled @ (Type::Function(_) | Type::Object(_)) => single_call_signature_of(&peeled),
            _ => None,
        },
        _ => None,
    }
}

/// `getParameterTypeOfFullSignature`: each declared parameter is the full
/// signature's type at its position, `any` past its end.
fn full_signature_parameter_types(signature: &FunctionType, parameter_count: usize) -> Vec<Type> {
    let mut types = contextual_parameter_types(signature, parameter_count);
    types.resize(parameter_count, Type::Any);
    types
}

pub(crate) fn check_function_declaration(
    function: ParsedFunctionDeclaration,
    ctx: &mut CheckerContext,
) {
    let start = Instant::now();
    let ParsedFunctionDeclaration {
        has_this_parameter,
        this_parameter_type,
        is_declare,
        name,
        name_span,
        type_parameters,
        parameters,
        return_type,
        return_type_span,
        body,
        has_body,
        is_generator,
        is_async,
        full_signature,
        ..
    } = function;
    check_type_parameter_declarations(&type_parameters, ctx);

    with_type_parameter_scope(&type_parameters, ctx, |ctx| {
        let signature_info = function_signature_info(
            &type_parameters,
            &parameters,
            return_type.as_ref(),
            &ctx.file_name,
        );
        let full_signature = full_signature_of(full_signature.as_ref(), ctx);
        let full_signature_parameters = full_signature
            .as_ref()
            .map(|signature| full_signature_parameter_types(signature, parameters.len()));
        let declared_type = signature::with_written_this_type(
            map_function_signature(
                &parameters,
                return_type.as_ref(),
                &type_parameters,
                full_signature_parameters.as_deref(),
                ctx,
            ),
            this_parameter_type.as_ref(),
            &type_parameters,
            ctx,
        );
        // Callers see the full signature itself (`getSignaturesOfSymbol`); the
        // body sees its parameters at their positions.
        let (function_type, body_function_type) = match (&full_signature, full_signature_parameters) {
            (Some(signature), Some(parameter_types)) => (
                signature.clone(),
                declared_type.with_signature_types(parameter_types, signature.return_type().clone()),
            ),
            _ => (declared_type.clone(), declared_type),
        };

        // The first registration for this name replaces what the collection
        // pre-pass installed — the check pass resolves the signature under the
        // right scope, so its answer is the authoritative one. A *later*
        // declaration of the same name is an overload, and replacing again would
        // leave only the last one: `declare function f(v: number): string`
        // followed by `declare function f(v: number, r: number): string` made
        // `f(1)` a false TS2554.
        let first_registration = ctx
            .checked_function_declaration_names
            .insert(std::sync::Arc::from(name.as_str()));
        let duplicate = {
            let symbols = &mut ctx.symbols;
            register_function_signature(
                name.clone(),
                with_type_copy_reason(TypeCopyReason::FunctionBodySetup, || function_type.clone()),
                Some(signature_info.clone()),
                symbols,
                first_registration,
                has_body,
                has_body && !first_registration,
            )
        };

        if duplicate {
            let diagnostic = Diagnostic::ts2393(ctx.file_name.clone());
            let diagnostic = match name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            };

            ctx.push(diagnostic);

            if let Some(first_span) = ctx.symbols.take_declaration_span(&name) {
                ctx.push(
                    Diagnostic::ts2393(ctx.file_name.clone()).with_span(convert_span(first_span)),
                );
            }
        } else if let Some(span) = name_span {
            ctx.symbols.record_declaration_span(&name, span);
        }

        // A bodyless `function` declaration is an ambient declaration or an
        // overload signature: there is no body to check, and the implementation
        // (or none, for ambient) carries the real body. Running body checks here
        // would falsely flag the signature (e.g. TS2355 for a non-void return type
        // with no `return`), which tsc never does for an overload signature.
        if is_declare || !has_body {
            return;
        }

        check_function_body_with_signature(
            name,
            parameters,
            body,
            &body_function_type,
            &type_parameters,
            Some(signature_info),
            return_type.is_some() || full_signature.is_some(),
            return_type_span.or(name_span),
            is_generator,
            is_async,
            has_this_parameter,
            this_parameter_type,
            ctx,
        );
    });
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.function_declaration_checking += start.elapsed()
    });
}

/// Every non-arrow function body binds `arguments` (tsc's `argumentsSymbol`,
/// typed by the global `IArguments`); an arrow sees its enclosing one.
pub(crate) fn bind_arguments_object(scopes: &mut ScopeStack, ctx: &mut CheckerContext) {
    let ty = crate::infer::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: "IArguments".to_string(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    // `argumentsSymbol` is a property, not a constant: a write is checked
    // against `IArguments`. It is no declaration either, so a local
    // `var arguments` is the first declaration of its name, not a redeclaration.
    scopes.insert_current(
        "arguments",
        crate::symbols::SymbolInfo {
            ty,
            kind: crate::symbols::SymbolKind::Parameter,
            function_signature: None,
        },
    );
}

/// A `function` declared inside another function's body, checked the way a
/// module-level one is but over the scope that encloses it. It binds its own
/// `this`, so the enclosing method's does not leak in.
pub(crate) fn check_nested_function_declaration(
    function: ParsedFunctionDeclaration,
    enclosing: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    if function.is_declare || !function.has_body {
        return;
    }
    let mut symbols = enclosing.clone_with_reason(TypeCopyReason::FunctionBodySetup);
    if !function.has_this_parameter {
        let _ = symbols.insert(
            "this",
            crate::symbols::SymbolInfo {
                ty: Type::Any,
                kind: crate::symbols::SymbolKind::Const,
                function_signature: None,
            },
        );
    }
    let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols.clone());
    let ParsedFunctionDeclaration {
        has_this_parameter,
        this_parameter_type,
        name,
        name_span,
        type_parameters,
        parameters,
        return_type,
        return_type_span,
        body,
        is_generator,
        is_async,
        ..
    } = function;
    check_type_parameter_declarations(&type_parameters, ctx);
    with_type_parameter_scope(&type_parameters, ctx, |ctx| {
        let signature_info = function_signature_info(
            &type_parameters,
            &parameters,
            return_type.as_ref(),
            &ctx.file_name,
        );
        let function_type = signature::with_written_this_type(
            map_function_signature(&parameters, return_type.as_ref(), &type_parameters, None, ctx),
            this_parameter_type.as_ref(),
            &type_parameters,
            ctx,
        );
        ctx.nested_function_scope = Some(std::sync::Arc::new(symbols));
        check_function_body_with_signature(
            name,
            parameters,
            body,
            &function_type,
            &type_parameters,
            Some(signature_info),
            return_type.is_some(),
            return_type_span.or(name_span),
            is_generator,
            is_async,
            has_this_parameter,
            this_parameter_type,
            ctx,
        );
    });
    ctx.symbols = saved_symbols;
}

pub(crate) fn check_function_declaration_body(
    function: ParsedFunctionDeclaration,
    function_type: &FunctionType,
    type_parameters: &[ParsedTypeParameter],
    ctx: &mut CheckerContext,
) {
    // A return type read from the body is not an annotation, and Go checks a
    // return statement only against the annotation (`checkReturnStatement` via
    // `getReturnTypeFromAnnotation`, nil here). Handing the body its own
    // `LazyBodyReturn` as the expectation lifted the degraded-expectation
    // suppression the plain sentinel gives: a component's returned JSX then
    // reported its callback props as implicit `any`.
    let settled_signature;
    let full_signature = full_signature_of(function.full_signature.as_ref(), ctx);
    let function_type = if let Some(signature) = &full_signature {
        // The collected signature is the full signature itself; the body
        // binds each parameter to its type at the parameter's position.
        settled_signature = FunctionType::new(
            full_signature_parameter_types(signature, function.parameters.len()),
            signature.return_type().clone(),
            function.parameters.last().is_some_and(|parameter| parameter.rest),
            signature::required_parameter_count(&function.parameters),
        )
        .with_parameter_names(signature::written_binding_names(&function.parameters));
        &settled_signature
    } else if function.return_type.is_none()
        && matches!(
            function_type.return_type(),
            Type::Reference(reference) if reference.id.contains(BODY_RETURN_ID_TAG)
        ) {
        settled_signature = FunctionType::new(
            function_type.parameters().to_vec(),
            Type::Unknown,
            function_type.is_variadic(),
            function_type.required_parameter_count(),
        )
        .with_parameter_names(signature::written_binding_names(&function.parameters));
        &settled_signature
    } else {
        function_type
    };
    let start = Instant::now();
    let ParsedFunctionDeclaration {
        has_this_parameter,
        this_parameter_type,
        is_declare,
        name,
        name_span,
        parameters,
        return_type,
        return_type_span,
        body,
        has_body,
        is_generator,
        is_async,
        ..
    } = function;

    // A bodyless declaration is ambient or an overload signature; checking the
    // absent body would report TS2355 on a non-void return type that the
    // implementation below actually satisfies. Mirrors the guard in
    // `check_function_declaration`.
    if is_declare || !has_body {
        return;
    }

    let signature_info = function_signature_info(
        type_parameters,
        &parameters,
        return_type.as_ref(),
        &ctx.file_name,
    );
    check_function_body_with_signature(
        name,
        parameters,
        body,
        function_type,
        type_parameters,
        Some(signature_info),
        return_type.is_some() || full_signature.is_some(),
        return_type_span.or(name_span),
        is_generator,
        is_async,
        has_this_parameter,
        this_parameter_type,
        ctx,
    );
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.function_declaration_checking += start.elapsed()
    });
}

/// The contextual type a written parameter takes at each position of
/// `expected_type`. A rest parameter is not one positional slot: a tuple-typed
/// one (`(...args: [value: number, message?: string]) => void`, the shape TS
/// variadic-tuple reconstruction produces) declares those elements positionally,
/// and an array-typed one supplies its element type at every trailing position.
/// Reading `parameters()` by index instead leaves every position past the rest
/// slot uncontextualized, which is a false TS7006/TS7031 on the callback.
/// The tuple expansion mirrors `expanded_signature` in the assignability
/// relation, so a callback typed here still compares against its slot.
/// The contextual type of a written rest parameter at `position`: tsc's
/// `getRestTypeAtPosition` (relater.go:1829-1858). It is the *collection* the
/// rest binds — the context's own rest type when the positions line up, an array
/// of its element past that, and otherwise a tuple of the remaining positions —
/// never the element type the positional expansion hands every other slot.
/// `(...a) => …` against `(...args: string[]) => void` typed `a` as `string`.
///
/// `None` where tsc's answer needs an optional tuple element, which surge's
/// tuple types cannot say; those positions keep the positional expansion.
fn contextual_rest_parameter_type(expected_type: &FunctionType, position: usize) -> Option<Type> {
    let parameters = expected_type.parameters();
    let count = parameters.len();
    // Surge stores a rest slot either as the written array or already unwrapped
    // to its element (see `contextual_parameter_types`); tsc's `restType` is the
    // collection, and its element is what an index by `number` reads.
    let rest_and_element = expected_type.is_variadic().then(|| {
        let stored = match &parameters[count - 1] {
            Type::Reference(reference) => reference.resolve().peeled(),
            other => other.clone(),
        };
        match stored {
            Type::Array(element) => (Type::Array(element.clone()), *element),
            Type::Tuple(elements) => {
                let element = surge_ts_types::union_type(elements.clone());
                (Type::Tuple(elements), element)
            }
            Type::OpenTuple(open) => {
                let element = open.element_union();
                (Type::OpenTuple(open), element)
            }
            element => (Type::Array(Box::new(element.clone())), element),
        }
    });

    if let Some((rest, element)) = &rest_and_element
        && position + 1 >= count
    {
        return Some(if position + 1 == count {
            rest.clone()
        } else {
            Type::Array(Box::new(element.clone()))
        });
    }

    let fixed_end = if rest_and_element.is_some() {
        count - 1
    } else {
        count
    };
    if position >= fixed_end {
        return Some(Type::Tuple(Vec::new()));
    }
    // surge's tuples have no optional elements, so an optional parameter
    // contributes its `T | undefined` slot.
    let required = expected_type.required_parameter_count();
    let leading = parameters[position..fixed_end]
        .iter()
        .enumerate()
        .map(|(offset, parameter)| {
            if position + offset >= required
                && !surge_ts_types::is_assignable_to(&Type::Undefined, parameter)
            {
                surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
            } else {
                parameter.clone()
            }
        })
        .collect::<Vec<_>>();
    Some(match rest_and_element {
        None => Type::Tuple(leading),
        Some((Type::Tuple(elements), _)) => {
            Type::Tuple(leading.into_iter().chain(elements).collect())
        }
        Some((Type::OpenTuple(open), _)) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading: leading.into_iter().chain(open.leading).collect(),
            rest: open.rest,
            trailing: open.trailing,
        }),
        Some((_, element)) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading,
            rest: Box::new(element),
            trailing: Vec::new(),
        }),
    })
}

fn without_undefined(ty: Type) -> Type {
    match &ty {
        Type::Union(union) if union.types().contains(&Type::Undefined) => {
            surge_ts_types::union_type(
                union
                    .types()
                    .iter()
                    .filter(|member| !matches!(member, Type::Undefined))
                    .cloned()
                    .collect(),
            )
        }
        _ => ty,
    }
}

/// tsc's `getNarrowedTypeOfSymbol`: the parameters of a callback whose
/// contextual signature is a lone rest parameter of a union of tuples depend on
/// each other the way the names of one destructuring do — `kind === 'A'` picks
/// the `payload` of the tuples it leaves — unless one of them is written.
fn record_dependent_parameters(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    expected_type: &FunctionType,
    body: &surge_ts_syntax::ParsedArrowFunctionBody,
    arrow_span: Option<surge_ts_syntax::TextSpan>,
    scopes: &mut ScopeStack,
) {
    let (Some(span), [rest]) = (arrow_span, expected_type.parameters()) else {
        return;
    };
    if parameters.len() < 2 || !expected_type.is_variadic() {
        return;
    }
    let rest = rest.peeled();
    let Type::Union(union) = &rest else {
        return;
    };
    if !union
        .types()
        .iter()
        .all(|member| matches!(member.peeled(), Type::Tuple(_) | Type::OpenTuple(_)))
    {
        return;
    }
    let assigned = match body {
        surge_ts_syntax::ParsedArrowFunctionBody::Block(statements) => {
            deep_assigned_names(&[statements.as_slice()])
        }
        surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => {
            crate::flow::expression_assignments(expression)
                .into_iter()
                .filter_map(|(assignment, _)| match assignment {
                    surge_ts_syntax::ParsedExpression::Assignment { target_name, .. } => {
                        Some(target_name.clone())
                    }
                    _ => None,
                })
                .collect()
        }
    };
    let names: Vec<&str> = parameters
        .iter()
        .filter_map(|parameter| match &parameter.binding_name {
            surge_ts_syntax::ParsedBindingName::Identifier { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if names.iter().any(|name| assigned.iter().any(|assigned| assigned == name)) {
        return;
    }
    let source: std::sync::Arc<str> = format!("\0rest@{}", span.start).into();
    for (index, parameter) in parameters.iter().enumerate() {
        if let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } = &parameter.binding_name
            && parameter.declared_type.is_none()
            && parameter.initializer.is_none()
            && !parameter.rest
        {
            scopes.record_tuple_destructure(
                name,
                Some(crate::symbols::TupleDestructureBinding {
                    source: source.clone(),
                    key: crate::symbols::DestructureKey::Index(index),
                    source_type: Some(rest.clone()),
                }),
            );
        }
    }
}

fn contextual_parameter_types(expected_type: &FunctionType, parameter_count: usize) -> Vec<Type> {
    // tsc's `getTypeOfParameter`: an optional parameter of the contextual
    // signature is `T | undefined`, and that is the type an unannotated
    // callback parameter takes from it — the caller may well omit it.
    let required = expected_type.required_parameter_count();
    let with_optionality = |index: usize, parameter: &Type| {
        if index >= required
            && surge_ts_types::strict_null_checks()
            && !parameter.is_unknown()
            && !matches!(parameter, Type::Any)
        {
            surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
        } else {
            parameter.clone()
        }
    };
    let parameters = expected_type.parameters();
    if !expected_type.is_variadic() {
        return parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| with_optionality(index, parameter))
            .collect();
    }

    let Some((rest, leading)) = parameters.split_last() else {
        return Vec::new();
    };
    let leading: Vec<Type> = leading
        .iter()
        .enumerate()
        .map(|(index, parameter)| with_optionality(index, parameter))
        .collect();
    let leading = leading.as_slice();
    let peeled;
    let rest = match rest {
        Type::Reference(reference) => {
            peeled = reference.resolve().peeled();
            &peeled
        }
        other => other,
    };

    let mut expanded = leading.to_vec();
    match rest {
        Type::Tuple(elements) => expanded.extend(elements.iter().cloned()),
        // tsc's `tryGetTypeAtPosition`: a rest parameter that is not itself a
        // tuple is indexed at each position, which distributes over a union of
        // tuples (`(...args: ['A', number] | ['B', string])` gives `'A' | 'B'`
        // then `number | string`); a fixed tuple too short for the position
        // reads `undefined`.
        Type::Union(union) if union.types().iter().all(|member| {
            matches!(member.peeled(), Type::Tuple(_) | Type::OpenTuple(_))
        }) =>
        {
            for position in 0..parameter_count.saturating_sub(leading.len()) {
                let elements = union
                    .types()
                    .iter()
                    .map(|member| match member.peeled() {
                        Type::Tuple(elements) => {
                            elements.get(position).cloned().unwrap_or(Type::Undefined)
                        }
                        Type::OpenTuple(open) => match open.leading.get(position) {
                            Some(element) => element.clone(),
                            None => {
                                let mut tail = vec![open.rest.as_ref().clone()];
                                tail.extend(open.trailing.iter().cloned());
                                surge_ts_types::union_type(tail)
                            }
                        },
                        _ => unreachable!("every member is a tuple"),
                    })
                    .collect();
                expanded.push(surge_ts_types::union_type(elements));
            }
        }
        // The resolver already unwraps an array rest annotation to its element
        // type, but a signature mapped from source keeps the array; accept both.
        other => {
            let element = match other {
                Type::Array(element) => element.as_ref(),
                other => other,
            };
            let fill_to = parameter_count.max(leading.len() + 1);
            while expanded.len() < fill_to {
                expanded.push(element.clone());
            }
        }
    }
    expanded
}

pub(crate) fn check_arrow_function_expression(
    arrow: ParsedArrowFunction,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> FunctionType {
    check_arrow_function_expression_with_expected_type(arrow, None, symbols, ctx)
}

/// Emits the whole-signature mismatch tsc reports when a contextually-typed
/// arrow's returns do not fit: TS2322 anchored on the assignment target, or
/// TS2345 when the arrow is a call argument.
#[allow(clippy::too_many_arguments)]
fn emit_contextual_signature_mismatch(
    parameter_types: &[Type],
    returned_types: &[Type],
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    is_variadic: bool,
    required_parameter_count: usize,
    expected_type: &FunctionType,
    span: Option<surge_ts_syntax::TextSpan>,
    is_argument: bool,
    ctx: &mut CheckerContext,
) {
    if returned_types.is_empty() {
        return;
    }
    // tsc widens the fresh literals a returned object literal carries, so
    // `return { ok: "nope" }` renders as `{ ok: string; }`.
    let returned =
        crate::checks::expr::widen_type(&surge_ts_types::union_type(returned_types.to_vec()));
    let source = alloc_function_type(
        parameter_types.to_vec(),
        returned,
        is_variadic,
        required_parameter_count,
    )
    .with_parameter_names(signature::written_binding_names(parameters));
    let source = Type::Function(source);
    let target = Type::Function(expected_type.clone());
    let (source_name, target_name) = crate::checks::expr::disambiguated_pair(
        &source,
        source.name(),
        &target,
        target.name(),
        &ctx.file_name,
    );
    let diagnostic = if is_argument {
        crate::checks::expr::assignability_mismatch_diagnostic(
            &source,
            &target,
            &source_name,
            &target_name,
            true,
            ctx.file_name.clone(),
        )
    } else {
        crate::checks::expr::type_not_assignable_diagnostic(
            &source,
            &target,
            &source_name,
            &target_name,
            ctx.file_name.clone(),
        )
    };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
}

pub(crate) fn check_arrow_function_expression_with_expected_type(
    arrow: ParsedArrowFunction,
    expected_type: Option<&FunctionType>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> FunctionType {
    check_arrow_function_expression_anchored(arrow, expected_type, None, symbols, ctx)
}

/// [`check_arrow_function_expression_with_expected_type`] with the span tsc
/// anchors a whole-signature mismatch on — the assignment target, not the
/// failing return.
/// An unannotated arrow or function expression with a *block* body takes its
/// return type from its `return` statements, as tsc's `getReturnTypeFromBody`
/// does. On by default since 2026-09-22; `SURGE_INFER_BLOCK_RETURN_TYPES=0`
/// turns it off. Before it, every `const f = () => { return … }` was the
/// sentinel and silenced each use of `f`.
///
/// Turning it on first opened 31 trpc false positives, each a missing tsc rule
/// the sentinel had hidden: a function against an `any` index signature,
/// candidate widening that dropped surge's openness marker, filter callbacks'
/// inferred type predicates, and array elements whose declared literal members
/// were widened. Only an arrow with no contextual type is inferred here; a
/// contextually typed block callback still keeps the sentinel.
fn infer_block_body_return_types() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_INFER_BLOCK_RETURN_TYPES").as_deref() != Ok("0"))
}

/// tsc's `getReturnTypeFromBody` widens a single literal return type unless the
/// contextual return type is literal-like for it (`isLiteralOfContextualType`),
/// so `() => 1` returns `number` while `(): 1 => 1` and `c ? "a" : "b"` keep
/// their literals. A contextual type surge could not settle keeps the literal.
pub(crate) fn widen_unit_return_type(
    body_type: Type,
    contextual_return_type: Option<&Type>,
) -> Type {
    let unit_literal = match &body_type {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => true,
        Type::Reference(reference) => reference.enum_base.is_some(),
        _ => false,
    };
    if !unit_literal {
        return body_type;
    }
    if contextual_return_type
        .is_some_and(|contextual| is_literal_of_contextual_type(&body_type, contextual))
    {
        return body_type;
    }
    crate::checks::expr::widen_type(&body_type)
}

/// tsc's `getReturnTypeFromBody` for a generator: `Generator<Y, R, N>` from
/// what the body yields, returns and is resumed with. `None` when a yield or
/// a returned value could not be typed, or the returns were not collected.
pub(crate) fn inferred_generator_type(
    yields: Option<&GeneratorYields>,
    returned: Option<&[Type]>,
    body_flow: &crate::flow::FunctionBodyFlow,
    contextual_return: Option<&Type>,
    is_async: bool,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let yields = yields.filter(|yields| !yields.unmodelled)?;
    let return_type = if body_flow.contains_return_with_value {
        let returned = returned.filter(|returned| !returned.is_empty())?;
        if returned.iter().any(|ty| ty.is_unmodelled()) {
            return None;
        }
        let mut members: Vec<Type> = returned
            .iter()
            .map(|ty| if is_async { crate::checks::call::awaited_type(ty) } else { ty.clone() })
            .collect();
        if surge_ts_types::strict_null_checks() && !body_flow.guarantees_exit {
            members.push(Type::Undefined);
        }
        let contextual_return_value = contextual_return.and_then(|ty| contextual_iteration_type(ty, 1));
        match members.as_slice() {
            [single] => widen_unit_return_type(single.clone(), contextual_return_value.as_ref()),
            _ => surge_ts_types::union_type(members),
        }
    } else if body_flow.guarantees_exit && !body_flow.contains_return {
        // `checkAndAggregateReturnExpressionTypes`: a bare `return;` is a return
        // of `void`; only a body that never completes at all returns `never`.
        Type::Never
    } else {
        Type::Void
    };
    let contextual_yield = contextual_return.and_then(|ty| contextual_iteration_type(ty, 0));
    let yield_type = match yields.yielded.as_slice() {
        [] => Type::Never,
        [single] => widen_unit_return_type(single.clone(), contextual_yield.as_ref()),
        many => surge_ts_types::union_type(many.to_vec()),
    };
    // `getWidenedType`: without `strictNullChecks` a yielded `null` or
    // `undefined` widens to `any`, which absorbs the union.
    let widens_to_any = !surge_ts_types::strict_null_checks()
        && match yield_type.peeled() {
            Type::Undefined | Type::Null => true,
            Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Undefined | Type::Null)),
            _ => false,
        };
    let yield_type = if widens_to_any { Type::Any } else { yield_type };
    let resumed: Vec<&Type> = yields.sent.iter().filter(|ty| !matches!(ty, Type::GenuineUnknown)).collect();
    let next_type = match resumed.as_slice() {
        [] if !yields.sent.is_empty() => Type::GenuineUnknown,
        [] => contextual_return
            .and_then(|ty| contextual_iteration_type(ty, 2))
            .unwrap_or(Type::GenuineUnknown),
        [single] => (*single).clone(),
        _ => return None,
    };
    generator_type_of(&yield_type, &return_type, &next_type, is_async, ctx)
}

/// tsc's `getIterationTypeOfGeneratorFunctionReturnType` over a contextual
/// return type: a union's members each describe the generator, so its
/// iteration type is the union of theirs (yield, return, next by `index`).
fn contextual_iteration_type(contextual: &Type, index: usize) -> Option<Type> {
    let argument = |ty: &Type| match index {
        0 => generator_yield_type_argument(ty),
        1 => generator_return_type_argument(ty),
        _ => generator_next_type_argument(ty),
    };
    match contextual {
        Type::Union(union) => union
            .types()
            .iter()
            .map(argument)
            .collect::<Option<Vec<_>>>()
            .map(surge_ts_types::union_type),
        _ => argument(contextual),
    }
}

pub(crate) fn is_literal_of_contextual_type(candidate: &Type, contextual: &Type) -> bool {
    // An enum member is the number or string literal it stands for.
    if let Type::Reference(reference) = candidate
        && reference.enum_base.is_some()
    {
        return is_literal_of_contextual_type(&reference.resolve(), contextual);
    }
    match contextual {
        Type::Unknown | Type::TypeParameter(_) => true,
        Type::Reference(reference) => {
            is_literal_of_contextual_type(candidate, &reference.resolve())
        }
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| is_literal_of_contextual_type(candidate, member)),
        Type::StringLiteral(_) => matches!(candidate, Type::StringLiteral(_)),
        Type::NumberLiteral(_) => matches!(candidate, Type::NumberLiteral(_)),
        Type::BooleanLiteral(_) | Type::Boolean => matches!(candidate, Type::BooleanLiteral(_)),
        _ => false,
    }
}

/// tsc's `elaborateArrowFunction`: an expression-bodied arrow with no
/// annotated parameters whose body does not fit the contextual return type is
/// reported at the body, as the return types rather than the whole signatures.
fn report_contextual_body_mismatch(
    body_type: &Type,
    contextual_return_type: &Type,
    expected_type: &FunctionType,
    body_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let Some(body_span) = body_span else {
        return false;
    };
    if matches!(
        contextual_return_type,
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) | Type::Void
    ) || surge_ts_types::is_assignable_to(body_type, contextual_return_type)
        // The same degradation test the enclosing relation applies, which
        // reports the whole value when this does not report the body.
        || crate::checks::assign::type_contains_unknown(body_type)
        || crate::checks::assign::type_contains_unknown(contextual_return_type)
    {
        return false;
    }
    // `elaborateError` tries `elaborateDidYouMeanToCallOrConstruct` before the
    // arrow elaboration: when calling the arrow would produce something that fits
    // the whole expected type, the mismatch stays on the arrow itself.
    if !matches!(body_type, Type::Any | Type::Never)
        && surge_ts_types::is_assignable_to(body_type, &Type::Function(expected_type.clone()))
    {
        return false;
    }
    let reported_target =
        crate::checks::expr::reported_relation_target(body_type, contextual_return_type);
    let source_name = crate::checks::expr::source_display_name(body_type, &reported_target);
    let target_name = reported_target.name();
    let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
        body_type,
        &reported_target,
        &source_name,
        &target_name,
        ctx.file_name.clone(),
    )
    .with_span(crate::context::convert_span(body_span));
    ctx.push(diagnostic);
    true
}

pub(crate) fn check_arrow_function_expression_anchored(
    arrow: ParsedArrowFunction,
    expected_type: Option<&FunctionType>,
    target_span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> FunctionType {
    let ParsedArrowFunction {
        is_generator,
        this_binding,
        this_parameter_type,
        name,
        type_parameters,
        parameters,
        return_type,
        return_type_span,
        is_async,
        body,
        body_reads: _,
        body_span,
        span: arrow_span,
    } = arrow;
    // tsc's `isUntypedSignatureInJSFile` holds only for a function with no
    // contextual type; a contextually typed one requires its parameters.
    let parameters = match expected_type {
        Some(_) if parameters.iter().any(|parameter| parameter.untyped_javascript) => parameters
            .into_iter()
            .map(|mut parameter| {
                if parameter.untyped_javascript {
                    parameter.optional = parameter.initializer.is_some();
                    parameter.untyped_javascript = false;
                }
                parameter
            })
            .collect(),
        _ => parameters,
    };
    let is_argument = std::mem::take(&mut ctx.next_arrow_is_argument);
    let context_only = std::mem::take(&mut ctx.next_arrow_context_only);
    let not_elaborated = NEXT_ARROW_NOT_ELABORATED.replace(false);
    // What an object literal hands its member (tsc's
    // `getContextualThisParameterType`); an arrow has no `this` to take it.
    let literal_this = ctx
        .next_arrow_this
        .take()
        .filter(|_| !matches!(this_binding, surge_ts_syntax::ParsedThisBinding::Inherited));
    check_type_parameter_declarations(&type_parameters, ctx);
    // A generic arrow's, function expression's or object-literal method's own
    // type parameters are type variables while its signature and body are
    // checked, exactly as a declaration's are; the signature is mapped inside
    // the scope, so it resolves over them rather than the placeholders.
    let _type_variables = enter_body_type_variables(&type_parameters, ctx);

    // An arrow does not bind `this`, so it keeps whatever the enclosing function
    // established; a `function` expression and an object-literal method both
    // lower to this shape but bind their own.
    let outer_this_is_implicitly_any = ctx.this_is_implicitly_any;
    let outer_constructor_writable_members = ctx.constructor_writable_members.take();
    match this_binding {
        surge_ts_syntax::ParsedThisBinding::Inherited => {}
        surge_ts_syntax::ParsedThisBinding::Own => ctx.this_is_implicitly_any = false,
        surge_ts_syntax::ParsedThisBinding::ImplicitAny => {
            ctx.this_is_implicitly_any = literal_this.is_none();
        }
    }
    let _this_class = (!matches!(this_binding, surge_ts_syntax::ParsedThisBinding::Inherited)).then(|| {
        crate::checks::expr::ThisParameterClassScope::enter(this_parameter_type.as_ref().and_then(|written| {
            crate::checks::expr::this_parameter_class(Some(written), &type_parameters, ctx)
        }))
    });
    let written_this = this_parameter_type.filter(|_| !matches!(this_binding, surge_ts_syntax::ParsedThisBinding::Inherited)).map(|written| {
        let checkpoint = ctx.diagnostics().len();
        let ty = with_type_parameter_scope(&type_parameters, ctx, |ctx| crate::infer::map_parsed_type(written, ctx));
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        ty
    });

    let mut expanded_contextual_parameter_types = expected_type
        .map(|expected_type| contextual_parameter_types(expected_type, parameters.len()));
    // tsc's `getContextuallyTypedParameterType`: a rest parameter takes the
    // rest of the contextual signature (`getRestTypeAtPosition`), which is the
    // empty tuple when nothing is left — still a contextual type, so the
    // parameter is no implicit `any[]`.
    if let (Some(expected_type), Some(types)) =
        (expected_type, expanded_contextual_parameter_types.as_mut())
        && let Some(last) = parameters.len().checked_sub(1)
        && parameters[last].rest
        && parameters[last].declared_type.is_none()
        && types.len() == last
        && let Some(rest) = contextual_rest_parameter_type(expected_type, last)
    {
        types.push(rest);
    }
    let contextual_parameter_types = expanded_contextual_parameter_types.as_deref();
    let result = with_type_parameter_scope(&type_parameters, ctx, |ctx| {
        // Resolve the arrow's annotations against the value symbols visible at
        // the arrow site, mirroring `check_variable_declaration_against_symbols`:
        // `(x: typeof localConst) => …` must see the enclosing function body's
        // locals, which live in the scope stack and never reach `ctx.symbols`.
        // A named `function` expression's own name is in scope there too (tsc
        // resolves it at the expression, past its parameters); its signature is
        // what is being resolved, so the name stands at the sentinel meanwhile.
        let signature_symbols = match &name {
            Some(name) => {
                let mut scope = SymbolTable::with_parent(std::sync::Arc::new(symbols.clone()));
                let _ = scope.insert(
                    name.as_str(),
                    crate::symbols::SymbolInfo {
                        ty: Type::Unknown,
                        kind: crate::symbols::SymbolKind::Function,
                        function_signature: None,
                    },
                );
                scope
            }
            None => symbols.clone(),
        };
        let saved_symbols = std::mem::replace(&mut ctx.symbols, signature_symbols);
        let function_type = map_function_signature(
            &parameters,
            return_type.as_ref(),
            &type_parameters,
            contextual_parameter_types,
            ctx,
        );
        ctx.symbols = saved_symbols;
        let source_type_name = function_type.name();
        let has_explicit_return_type = return_type.is_some();
        let mut parameter_types = function_type.parameters().to_vec();
        let mut return_type = with_type_copy_reason(TypeCopyReason::ExpectedType, || {
            function_type.return_type().clone()
        });
        // tsc's `checkSignatureDeclaration`: a generator's annotation describes
        // the generator object, which `void` never does.
        if is_generator && has_explicit_return_type && matches!(return_type, Type::Void) {
            ctx.push(crate::spans::diagnostic_with_syntax_span(
                Diagnostic::ts2505(ctx.file_name.clone()),
                return_type_span,
            ));
        } else if is_generator && has_explicit_return_type {
            check_generator_instantiation(&return_type, is_async, return_type_span, ctx);
        }

        if let Some(expected_type) = expected_type {
            for (index, parameter_type) in contextual_parameter_types
                .unwrap_or_default()
                .iter()
                .cloned()
                .enumerate()
            {
                if index < parameter_types.len() && parameters[index].declared_type.is_none() {
                    // A default initializer supplies the value whenever the
                    // caller passes `undefined`, so inside the function the
                    // parameter no longer includes it
                    // (`(fn, options = {}) => …` reads `options` as the
                    // object).
                    parameter_types[index] = if parameters[index].initializer.is_some() {
                        without_undefined(parameter_type)
                    } else {
                        parameter_type
                    };
                }
            }

            if !has_explicit_return_type {
                return_type = with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                    expected_type.return_type().clone()
                });
            }

            if has_contextual_unknown_binding_pattern(
                &parameters,
                contextual_parameter_types,
            ) {
                let target_type_name = expected_type.name();
                let diagnostic =
                    Diagnostic::ts2345(&source_type_name, &target_type_name, ctx.file_name.clone());
                let diagnostic = match arrow_span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                };
                ctx.push(diagnostic);
            }
        }

        if ctx.skip_annotated_function_bodies && has_explicit_return_type {
            return alloc_function_type(
                parameter_types,
                return_type,
                function_type.is_variadic(),
                function_type.required_parameter_count(),
            )
            .with_parameter_names(signature::written_binding_names(&parameters));
        }

        let mut scopes =
            ScopeStack::from_root(symbols.clone_with_reason(TypeCopyReason::FunctionBodySetup));
        scopes.mark_function_boundary();
        if let Some(name) = &name {
            scopes.insert_current(
                name.as_str(),
                crate::symbols::SymbolInfo {
                    ty: Type::Function(
                        alloc_function_type(
                            parameter_types.clone(),
                            return_type.clone(),
                            function_type.is_variadic(),
                            function_type.required_parameter_count(),
                        )
                        .with_parameter_names(signature::written_binding_names(&parameters)),
                    ),
                    kind: crate::symbols::SymbolKind::Function,
                    function_signature: None,
                },
            );
        }
        scopes.push_function_scope();
        if !matches!(this_binding, surge_ts_syntax::ParsedThisBinding::Inherited) {
            bind_arguments_object(&mut scopes, ctx);
        }
        // A `function` expression binds its own `this`; without a `this`
        // parameter it has no type, so the enclosing scope's `this` must not
        // leak in. tsc takes `this` from the contextual signature here — surge
        // does not model that, and reading the enclosing class instead reported
        // its members missing (`ws.addEventListener('message', function () {
        // this.send(…) })` inside a class). An object-literal member takes the
        // `this` its literal hands it.
        // `getContextualThisParameterType`: the contextual signature's `this`
        // comes before the literal's.
        let contextual_this = expected_type
            .and_then(|expected| expected.this_type())
            .filter(|this_type| {
                !matches!(this_binding, surge_ts_syntax::ParsedThisBinding::Inherited) && !this_type.is_unmodelled()
            })
            .cloned();
        if contextual_this.is_some() || written_this.is_some() {
            ctx.this_is_implicitly_any = false;
        }
        let own_this = match (&literal_this, this_binding) {
            _ if written_this.is_some() => written_this.clone(),
            _ if contextual_this.is_some() => contextual_this,
            (Some(literal_this), _) => Some(literal_this.clone()),
            (None, surge_ts_syntax::ParsedThisBinding::ImplicitAny) => Some(Type::Any),
            (None, _) => None,
        };
        if let Some(own_this) = own_this {
            scopes.insert_current(
                "this",
                crate::symbols::SymbolInfo {
                    ty: own_this,
                    kind: crate::symbols::SymbolKind::Const,
                    function_signature: None,
                },
            );
        }
        for (index, parameter) in parameters.iter().enumerate() {
            // The signature keeps surge's variadic slot, but the binding a
            // contextually typed `...rest` introduces holds the collection.
            let rest_binding_type = expected_type
                .filter(|_| parameter.rest && parameter.declared_type.is_none())
                .filter(|_| index + 1 == parameters.len())
                .and_then(|expected_type| contextual_rest_parameter_type(expected_type, index));
            let parameter_type = rest_binding_type
                .as_ref()
                .unwrap_or_else(|| parameter_types.get(index).unwrap_or(&Type::Any));
            insert_parameter_bindings(parameter, parameter_type, &mut scopes);
            signature::record_parameter_binding(parameter, parameter_type, &scopes, ctx);
        }
        if let Some(expected_type) = expected_type {
            record_dependent_parameters(&parameters, expected_type, &body, arrow_span, &mut scopes);
        }
        crate::checks::function::with_type_parameter_scope(&type_parameters, ctx, |ctx| {
            for (index, parameter) in parameters.iter().enumerate() {
                if let Some(parameter_type) = parameter_types.get(index) {
                    check_annotated_parameter_initializer(
                        parameter,
                        parameter_type,
                        scopes.visible_symbols(),
                        ctx,
                    );
                    check_annotated_binding_pattern_reads(parameter, parameter_type, ctx);
                }
                signature::check_binding_pattern_keys(&parameter.binding_name, scopes.visible_symbols(), ctx);
                check_binding_pattern_defaults(
                    &parameter.binding_name,
                    parameter_types.get(index),
                    parameter.declared_type.is_some(),
                    scopes.visible_symbols(),
                    ctx,
                );
                if let Some(parameter_type) = parameter_types.get(index) {
                    signature::check_parameter_pattern_accessibility(
                        &parameter.binding_name,
                        parameter_type,
                        scopes.visible_symbols(),
                        ctx,
                    );
                }
            }
        });

        let visible_symbols = visible_symbols(&scopes);
        // tsc's `unwrapReturnType`: an async body is typed against the awaited
        // return type (`async (): Promise<R> => ({ … })` reads the literal
        // against `R`).
        let body_return_type = if is_async && !is_generator {
            crate::checks::call::awaited_type(&return_type)
        } else {
            return_type.clone()
        };
        let saved_never_initialized = ctx.inherited_never_initialized.clone();
        match body {
            ParsedArrowFunctionBody::Expression(expression) => {
                // An expression body is a flow container of its own; only a
                // never-initialized outer `let` can be unassigned in it.
                if let Some(own_flow) = crate::flow::expression_container_flow(&parameters, ctx) {
                    crate::flow::check_parameter_default_flow(&parameters, &own_flow, ctx);
                    let _ =
                        crate::flow::check_expression_flow(&expression, None, &own_flow, 0, ctx);
                }
                ctx.inherited_never_initialized
                    .retain(|name| !crate::flow::binds_parameter(&parameters, name));
                let return_type_for_body = match &body_return_type {
                    // An annotation naming the arrow's own type variable is a type
                    // its returns are related to, as a declaration's are
                    // (`checkReturnExpression`).
                    Type::TypeParameter(_)
                        if has_explicit_return_type && body_return_type.is_type_variable() =>
                    {
                        Some(&body_return_type)
                    }
                    Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
                        None
                    }
                    // A *contextual* `void` accepts any return (`cb: () => void`
                    // takes `() => 1`); a written `: void` is an annotation like
                    // any other and tsc checks the returned value against it.
                    Type::Void if !has_explicit_return_type => None,
                    ty => Some(ty),
                };
                let inferred_body = match return_type_for_body {
                    // Like a `return` operand, the body anchors what its own
                    // node has no span for (TS2352 on `() => <T>null`).
                    None => evaluate_expression(&expression, body_span, &visible_symbols, ctx),
                    // Only an annotated return type is checked through
                    // `checkReturnExpression`; a contextual one is related by the
                    // enclosing assignment, which does not split a conditional.
                    Some(return_type_for_body) if !has_explicit_return_type => {
                        super::expected::evaluate_expression_with_expected_type(
                            &expression,
                            None,
                            Some(return_type_for_body),
                            super::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
                            &visible_symbols,
                            ctx,
                        )
                    }
                    Some(return_type_for_body) => {
                        // The body is the async function's returned value, so
                        // a conditional's branches are awaited as a `return`'s.
                        let outer_async_body =
                            std::mem::replace(&mut ctx.in_async_body, is_async && !is_generator);
                        let inferred =
                            crate::checks::expected::evaluate_return_expression_with_expected_type(
                                &expression,
                                body_span,
                                None,
                                return_type_for_body,
                                &visible_symbols,
                                ctx,
                            );
                        ctx.in_async_body = outer_async_body;
                        // tsc's `checkReturnExpression` relates an expression body
                        // to the annotation as a whole, at the body — awaited on
                        // both sides for an async one (`unwrapReturnType`).
                        let awaited_sides = match &inferred {
                            InferredExpression::Known(body_type) if is_async => Some((
                                crate::checks::call::awaited_type(body_type),
                                crate::checks::call::awaited_type(return_type_for_body),
                            )),
                            _ => None,
                        };
                        let related_sides = match (&awaited_sides, &inferred) {
                            (Some((body_type, return_type)), _) => Some((body_type, return_type)),
                            (None, InferredExpression::Known(body_type)) => {
                                Some((body_type, return_type_for_body))
                            }
                            _ => None,
                        };
                        if let Some((body_type, return_type_for_body)) = related_sides
                            && !body_type.is_unmodelled()
                            && !surge_ts_types::is_assignable_to(body_type, return_type_for_body)
                            && !type_contains_unknown(body_type)
                            && !type_contains_unknown(return_type_for_body)
                        {
                            let reported_target = crate::checks::expr::reported_relation_target(
                                body_type,
                                return_type_for_body,
                            );
                            let source_name = crate::checks::expr::source_display_name(
                                body_type,
                                &reported_target,
                            );
                            let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
                                body_type,
                                &reported_target,
                                &source_name,
                                &reported_target.name(),
                                ctx.file_name.clone(),
                            );
                            ctx.push(crate::spans::diagnostic_with_syntax_span(
                                diagnostic, body_span,
                            ));
                        }
                        inferred
                    }
                };

                if !has_explicit_return_type {
                    if let Some(body_type) = inferred_body.flowing_type() {
                        if matches!(body_type, Type::ErrorType) {
                            return_type = Type::ErrorType;
                        } else if !body_type.is_unmodelled() {
                            // An argument after a call's first failing one is
                            // related to nothing, its body included.
                            let withheld_argument = not_elaborated
                                || context_only
                                    && ctx.suppressed_argument_mismatch_span.is_some();
                            if expected_type.is_some()
                                && !is_async
                                && !withheld_argument
                                && parameters
                                    .iter()
                                    .all(|parameter| parameter.declared_type.is_none())
                                && let Some(expected_type) = expected_type
                                && report_contextual_body_mismatch(
                                    &body_type,
                                    &return_type,
                                    expected_type,
                                    body_span,
                                    ctx,
                                )
                            {
                                // Reported on the body: the arrow keeps the
                                // contextual return so the enclosing relation
                                // does not report it again.
                            } else {
                                // `getReturnTypeFromBody` ends in `getWidenedType`: a
                                // fresh object or array literal's members widen as
                                // `checkExpressionForMutableLocation` widened them.
                                let body_type = if matches!(
                                    &*expression,
                                    surge_ts_syntax::ParsedExpression::ObjectLiteral { .. }
                                        | surge_ts_syntax::ParsedExpression::ArrayLiteral { .. }
                                ) {
                                    crate::checks::expr::widen_fresh_literal_expression_type(
                                        &expression,
                                        &body_type,
                                        expected_type.map(|expected| expected.return_type()),
                                    )
                                } else {
                                    body_type
                                };
                                if is_async && !is_generator {
                                    // `getReturnTypeFromBody` awaits an async body
                                    // (`getAwaitedTypeNoAlias`) before promising it.
                                    let contextual = expected_type.map(|expected| {
                                        crate::checks::call::awaited_type(expected.return_type())
                                    });
                                    return_type = widen_unit_return_type(
                                        crate::checks::call::awaited_type(&body_type),
                                        contextual.as_ref(),
                                    );
                                    return_type =
                                        crate::checks::call::promise_of(&return_type, ctx);
                                } else {
                                    return_type = widen_unit_return_type(
                                        body_type,
                                        expected_type.map(|expected| expected.return_type()),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            ParsedArrowFunctionBody::Block(statements) => {
                let flow_facts = collect_function_flow_facts(&statements);
                let mut flow_state = FunctionFlowState::new(
                    flow_facts.has_let_or_const || flow_facts.has_future_block_scoped_declarations,
                );
                let _ = crate::flow::enter_container(
                    &type_parameters,
                    &parameters,
                    &statements,
                    &mut flow_state,
                    ctx,
                );
                crate::flow::check_parameter_default_flow(&parameters, &flow_state, ctx);
                flow_state.hoist_vars(
                    crate::flow::collect_hoisted_vars(&statements)
                        .into_iter()
                        .filter(|name| !crate::flow::binds_parameter(&parameters, name))
                        .collect(),
                );
                let body_flow = analyze_function_body_flow(&statements);
                // Whether a `default`-less switch covers its discriminant is only
                // known once the body is checked.
                let recheck_body = (body_flow.guarantees_value_return || body_flow.guarantees_exit)
                    .then(|| body_has_defaultless_switch(&statements).then(|| statements.clone()))
                    .flatten();
                let tail_call = crate::checks::expr::tail_call_key(&statements);
                let return_type_for_body = match &return_type {
                    // An unannotated generator's returns make its `TReturn`;
                    // the contextual type describes the generator object.
                    _ if is_generator && !has_explicit_return_type => None,
                    // An annotation naming the arrow's own type variable is a type
                    // its returns are related to, as a declaration's are
                    // (`checkReturnExpression`).
                    Type::TypeParameter(_)
                        if has_explicit_return_type && return_type.is_type_variable() =>
                    {
                        Some(&return_type)
                    }
                    Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
                        None
                    }
                    // A *contextual* `void` accepts any return (`cb: () => void`
                    // takes `() => 1`); a written `: void` is an annotation like
                    // any other and tsc checks the returned value against it.
                    Type::Void if !has_explicit_return_type => None,
                    ty => Some(ty),
                };
                // A contextual return type this arrow did not annotate is not a
                // per-return assignability target for tsc — see
                // `ContextualReturnFrame`.
                // One frame per FUNCTION body — `check_function_body` also runs
                // for every nested block, so the frame cannot live there or an
                // `if` would shadow its own function's.
                if !has_explicit_return_type && expected_type.is_some() {
                    ctx.activate_next_body_frame();
                }
                ctx.open_contextual_return_frame();
                let _constructor_body = body_statements::ConstructorBody::enter(false);
                // tsc relates a generator's returns to its `TReturn` only under a
                // return type annotation (`getReturnTypeFromAnnotation`); a
                // contextually typed one is related by its whole signature.
                let annotated_generator = is_generator && has_explicit_return_type;
                // tsc's `reportErrorsFromWidening` for the yield type of a
                // nameless generator with no contextual signature (TS7025).
                if is_generator
                    && name.is_none()
                    && !has_explicit_return_type
                    && expected_type.is_none()
                    && ctx.degraded_expected_type_depth == 0
                    && ctx.options.no_implicit_any
                    && !ctx.options.strict_null_checks
                    && yields_only_widening_nullish(&statements)
                {
                    ctx.push(crate::spans::diagnostic_with_syntax_span(
                        Diagnostic::ts7025("any", ctx.file_name.clone()),
                        arrow_span,
                    ));
                }
                let outer_async_body = std::mem::replace(
                    &mut ctx.in_async_body,
                    is_async && (!is_generator || annotated_generator),
                );
                let outer_generator_body =
                    std::mem::replace(&mut ctx.in_generator_body, annotated_generator);
                let outer_yield_type = std::mem::replace(
                    &mut ctx.generator_yield_type,
                    annotated_generator
                        .then(|| generator_yield_type_argument(&return_type))
                        .flatten(),
                );
                let outer_next_type = std::mem::replace(
                    &mut ctx.generator_next_type,
                    annotated_generator
                        .then(|| generator_next_type_argument(&return_type))
                        .flatten(),
                );
                let outer_generator_function = std::mem::replace(&mut ctx.in_generator_function, is_generator);
                let outer_async_generator =
                    std::mem::replace(&mut ctx.in_async_generator, is_generator && is_async);
                let ((), generator_yields) =
                    collecting_generator_yields(is_generator && !has_explicit_return_type, || {
                        check_function_body(
                            statements,
                            return_type_for_body,
                            &mut scopes,
                            &mut flow_state,
                            ctx,
                        )
                    });
                ctx.in_async_body = outer_async_body;
                ctx.in_generator_body = outer_generator_body;
                ctx.generator_yield_type = outer_yield_type;
                ctx.generator_next_type = outer_next_type;
                ctx.in_generator_function = outer_generator_function;
                ctx.in_async_generator = outer_async_generator;
                let body_flow = match recheck_body {
                    Some(body)
                        if !ctx.non_exhaustive_switches.is_empty()
                            || !ctx.exhaustive_switches.is_empty() =>
                    {
                        crate::flow::with_non_exhaustive_switches(
                            &ctx.non_exhaustive_switches,
                            &ctx.exhaustive_switches,
                            || analyze_function_body_flow(&body),
                        )
                    }
                    _ => body_flow,
                };
                // The flow verdict is checked first: the return-type gate walks
                // the whole type, which on a large annotation is far costlier
                // than the body it guards.
                if has_explicit_return_type
                    && !is_generator
                    && !body_flow.guarantees_exit
                    && !body_flow.guarantees_value_return
                    && !tail_call.is_some_and(|key| ctx.never_returning_calls.contains(&key))
                    && should_check_missing_return(&body_return_type)
                {
                    emit_missing_return_diagnostic(
                        body_flow,
                        &body_return_type,
                        return_type_span.or(arrow_span),
                        ctx,
                    );
                }
                // tsc reports a contextually-typed arrow whose returns do not fit
                // as one whole-signature mismatch on the assignment, with the leaf
                // as nested elaboration. Take the leaf verdicts back and render
                // that instead.
                let contextual_return_mismatch = ctx.take_contextual_return_mismatch();
                // Not elaborated, the mismatch is the enclosing relation's to
                // report, so the arrow keeps the return its body gives.
                // Taking the verdicts took the returned types with them.
                let unelaborated_returns = (not_elaborated || context_only)
                    .then(|| contextual_return_mismatch.clone())
                    .flatten();
                let unelaborated_mismatch = unelaborated_returns.is_some();
                if let Some(returned_types) = contextual_return_mismatch
                    && let Some(expected_type) = expected_type
                    && !context_only
                    && !not_elaborated
                {
                    emit_contextual_signature_mismatch(
                        &parameter_types,
                        &returned_types,
                        &parameters,
                        function_type.is_variadic(),
                        function_type.required_parameter_count(),
                        expected_type,
                        target_span.or(arrow_span),
                        is_argument,
                        ctx,
                    );
                }
                // tsc infers an unannotated block body's return type from its
                // `return` statements (`getReturnTypeFromBody`). surge did that
                // only for an expression body, so an object-literal method
                // (`{ config() { return opts; } }`, which lowers to this shape)
                // handed back the sentinel — and every type parameter inferred
                // through that return died with it, which is what left tRPC's
                // router unmodelled at `createTRPCNext({ config() { … } })`.
                // A degraded return stays the sentinel: inferring from it would
                // turn "could not model" into a decision.
                // A contextual return that says nothing (`any` from a lib helper,
                // the sentinel, an uninstantiated parameter) leaves the body's
                // returns to decide, as tsc's `getReturnTypeFromBody` always does.
                let contextual_return =
                    expected_type.map(|expected| expected.return_type().clone());
                let contextual_return_is_open = contextual_return.as_ref().is_none_or(|ty| {
                    matches!(ty, Type::Any | Type::Unknown | Type::TypeParameter(_))
                });
                if infer_block_body_return_types()
                    && !has_explicit_return_type
                    && !is_generator
                    && (contextual_return_is_open || unelaborated_mismatch)
                {
                    let returned =
                        unelaborated_returns.unwrap_or_else(|| ctx.body_return_types().to_vec());
                    if !returned.is_empty()
                        && returned
                            .iter()
                            .all(|ty| !ty.is_unmodelled() || matches!(ty, Type::ErrorType))
                    {
                        // With no contextual return type, a returned literal
                        // widens (`() => { return 1; }` is `() => number`).
                        let mut members: Vec<Type> = returned
                            .into_iter()
                            .map(|member| {
                                widen_unit_return_type(member, contextual_return.as_ref())
                            })
                            .collect();
                        if !body_flow.guarantees_exit {
                            members.push(Type::Undefined);
                        }
                        return_type = surge_ts_types::union_type(members);
                        // The returns were recorded awaited; an async body
                        // returns a promise of them (`createPromiseReturnType`).
                        if is_async && !is_generator {
                            return_type = crate::checks::call::promise_of(&return_type, ctx);
                        }
                    }
                }
                // `getReturnTypeFromBody` with no `return <expr>`: `never` when
                // the end is unreachable and nothing returns (a function
                // expression or object-literal method `mayReturnNever`), else
                // `void`, or `undefined` for a contextual return admitting it.
                if !has_explicit_return_type
                    && !is_generator
                    && !body_flow.contains_return_with_value
                    && ctx.body_return_types().is_empty()
                {
                    let empty_return = if body_flow.guarantees_exit && !body_flow.contains_return {
                        Type::Never
                    } else {
                        let contextual_admits_undefined = contextual_return.as_ref().is_some_and(|ty| {
                            let unwrapped =
                                if is_async { crate::checks::call::awaited_type(ty) } else { ty.clone() };
                            match unwrapped.peeled() {
                                Type::Undefined => true,
                                Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Undefined)),
                                _ => false,
                            }
                        });
                        if contextual_admits_undefined { Type::Undefined } else { Type::Void }
                    };
                    return_type = if is_async {
                        crate::checks::call::promise_of(&empty_return, ctx)
                    } else {
                        empty_return
                    };
                }
                let returned = ctx.body_return_types().to_vec();
                if is_generator
                    && !has_explicit_return_type
                    && let Some(generator) = inferred_generator_type(
                        generator_yields.as_ref(),
                        Some(&returned),
                        &body_flow,
                        expected_type.map(|expected| expected.return_type()),
                        is_async,
                        ctx,
                    )
                {
                    return_type = generator;
                }
                let returned_void_like = ctx.close_contextual_return_frame();

                let contextually_void = expected_type
                    .is_some_and(|expected_type| matches!(expected_type.return_type(), Type::Void));
                if !has_explicit_return_type
                    && !is_generator
                    && !contextually_void
                    && !returned_void_like
                    && ctx.options.no_implicit_returns
                    && body_flow.contains_return_with_value
                    && !body_flow.guarantees_exit
                {
                    emit_implicit_return_diagnostic(arrow_span, ctx);
                }
            }
        }
        ctx.inherited_never_initialized = saved_never_initialized;

        if expected_type
            .is_some_and(|expected_type| matches!(expected_type.return_type(), Type::Void))
            && !has_explicit_return_type
        {
            return_type = Type::Void;
        }

        alloc_function_type(
            parameter_types,
            return_type,
            function_type.is_variadic(),
            function_type.required_parameter_count(),
        )
        .with_parameter_names(signature::written_binding_names(&parameters))
    });

    ctx.this_is_implicitly_any = outer_this_is_implicitly_any;
    ctx.constructor_writable_members = outer_constructor_writable_members;
    result
}
