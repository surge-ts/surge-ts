//! Origins of deferred types: the written node and the substitution a deferred
//! conditional or mapped type was resolved under, so an instantiation resolves
//! the node again over the instantiated substitution.

use std::sync::Arc;

use surge_ts_syntax::ParsedType;
use surge_ts_types::{Type, TypeCopyReason, TypeParameterType};

use super::{TypeParameterSubstitution, resolve_parsed_type};
use crate::context::{CheckerContext, DeclarationEnvironmentHandle};
use crate::symbols::TypeDeclarationScope;

pub(crate) struct ParsedTypeOrigin {
    node: ParsedType,
    substitution: TypeParameterSubstitution,
    environment: DeclarationEnvironmentHandle,
    creation_scope: Option<Arc<TypeDeclarationScope>>,
    file_name: Arc<str>,
}

impl std::fmt::Debug for ParsedTypeOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ParsedTypeOrigin")
    }
}

impl ParsedTypeOrigin {
    pub(crate) fn record(deferred: &Type, node: ParsedType, substitution: &TypeParameterSubstitution, ctx: &CheckerContext) {
        Self::install(deferred, node, substitution, ctx, false);
    }

    /// [`Self::record`] over an inner origin already recorded.
    pub(crate) fn replace(deferred: &Type, node: ParsedType, substitution: &TypeParameterSubstitution, ctx: &CheckerContext) {
        Self::install(deferred, node, substitution, ctx, true);
    }

    fn install(deferred: &Type, node: ParsedType, substitution: &TypeParameterSubstitution, ctx: &CheckerContext, replace: bool) {
        if !crate::checks::function::in_generic_body_read() || !matches!(deferred, Type::TypeParameter(_)) {
            return;
        }
        let install = if replace {
            surge_ts_types::type_variable::replace_deferred_origin
        } else {
            surge_ts_types::type_variable::set_deferred_origin
        };
        install(
            deferred,
            Arc::new(Self {
                node,
                substitution: substitution.clone_with_reason(TypeCopyReason::SubstitutionUnchanged),
                environment: ctx.declaration_environment(),
                creation_scope: ctx.type_declaration_scope.clone(),
                file_name: Arc::from(ctx.file_name.as_str()),
            }),
        );
    }
}

/// `substitution` with every value instantiated by `mapping`, and each mapped
/// variable's own name bound to its type: a node written in the generic body
/// names the body's type parameter directly.
pub(crate) fn instantiated_substitution(
    substitution: &TypeParameterSubstitution,
    mapping: &[(TypeParameterType, Type)],
) -> Option<TypeParameterSubstitution> {
    let mut instantiated = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    let entries: Vec<(Arc<str>, Type)> = substitution.iter().map(|(name, ty)| (name.clone(), ty.clone())).collect();
    for (name, ty) in entries {
        let ty = crate::checks::function::generic_return::instantiate_type(&ty, mapping).ok()?;
        instantiated.insert(name.to_string(), ty);
    }
    for (variable, to) in mapping {
        if substitution.get(&variable.name).is_none() {
            instantiated.insert(variable.name.to_string(), to.clone());
        }
    }
    Some(instantiated)
}

impl surge_ts_types::type_variable::Reinstantiate for ParsedTypeOrigin {
    fn reinstantiate(&self, mapping: &[(TypeParameterType, Type)]) -> Option<Type> {
        let mut ctx = durable_context(&self.file_name).or_else(|| self.environment.checker_context())?;
        // A node is written in its file, whose module scope names what it
        // reads; the scope installed when it was resolved can be a caller's
        // (an instantiated signature swaps only the file name).
        ctx.type_declaration_scope = crate::program::program_module_scope_for_file(&self.file_name)
            .or_else(|| self.creation_scope.clone());
        let substitution = instantiated_substitution(&self.substitution, mapping)?;
        let diagnostics_before = ctx.diagnostics().len();
        let resolved = resolve_parsed_type(self.node.clone(), &mut ctx, &mut Vec::new(), &substitution);
        ctx.truncate_diagnostics(diagnostics_before);
        // A degraded part is kept, as the per-call resolution keeps it: only
        // its caching depends on `had_error`.
        Some(resolved.ty)
    }
}

/// The reference `id` (a declaration's `file\0name`) at `arguments`, built from
/// its declaration the way a written reference to it is: the lazy reference a
/// consumer peels. `None` when the declaration is not reachable by its name in
/// its own file's scope.
pub(crate) fn reference_from_declaration(id: &str, display: &str, arguments: Vec<Type>) -> Option<Type> {
    let (file_name, name) = id.split_once('\0')?;
    let mut ctx = crate::context::program_checker_context()?;
    ctx.set_file_name(file_name.to_string());
    ctx.type_declaration_scope = crate::program::program_module_scope_for_file(file_name);
    let handle = ctx.lookup_type_declaration_handle(name)?;
    let decl_key = super::type_declaration_resolution_key(handle.get());
    if *super::type_declaration_alias_id(handle.get(), &decl_key) != *id {
        return None;
    }
    Some(super::make_lazy_type_reference(
        &mut ctx,
        &Arc::from(id),
        display,
        handle,
        decl_key,
        Vec::new(),
        arguments,
        TypeParameterSubstitution::new(),
    ))
}

/// The program context over `file_name`'s module scope. What an instantiation
/// builds outlives the body read whose shadow context made the deferred type
/// (its store is gone by the time a lazy part is peeled), so it is built in
/// the program's.
pub(crate) fn durable_context(file_name: &str) -> Option<CheckerContext> {
    let mut ctx = INSTANTIATION_CONTEXT
        .with(|slot| slot.borrow().clone())
        .or_else(crate::context::program_checker_context)?;
    ctx.set_file_name(file_name.to_string());
    ctx.type_declaration_scope = crate::program::program_module_scope_for_file(file_name);
    Some(ctx)
}

thread_local! {
    static INSTANTIATION_CONTEXT: std::cell::RefCell<Option<CheckerContext>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `ctx` as the context instantiations resolve their origins in:
/// the file check that asked for the instantiation, which holds the imports and
/// module scopes the program context captured before they existed.
pub(crate) fn with_instantiation_context<R>(ctx: CheckerContext, f: impl FnOnce() -> R) -> R {
    let outer = INSTANTIATION_CONTEXT.with(|slot| slot.replace(Some(ctx)));
    let result = f();
    INSTANTIATION_CONTEXT.with(|slot| *slot.borrow_mut() = outer);
    result
}
