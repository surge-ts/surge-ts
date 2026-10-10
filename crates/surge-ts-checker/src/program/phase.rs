use std::sync::Arc;

// Instrumentation lives in `metrics`; re-export it so existing callers that
// reference `crate::program::record_*` / `ProgramTimings` keep resolving and so
// the bare `record_*` calls throughout this file stay in scope.

/// Program-global check-phase marker. A `CheckerContext` field cannot carry
/// this: most check-phase resolution runs inside environment-RECOVERED
/// contexts (`from_declaration_environment`), which would never see a field
/// set on the driving context. Set when the per-file check dispatch starts,
/// cleared in the end-of-run teardown alongside `clear_program_type_caches`.
pub(super) static IN_CHECK_PHASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_check_phase(on: bool) {
    IN_CHECK_PHASE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn in_check_phase() -> bool {
    IN_CHECK_PHASE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Program-global copy of the authoritative per-file scope map, published when
/// the driving context installs it. Same reason as [`IN_CHECK_PHASE`]: a lazy
/// value/signature annotation resolves inside an environment-RECOVERED context,
/// and an environment captured before the map existed (module analysis takes it
/// away — see `binding.rs`) would otherwise resolve the declaring file's own
/// imports against an empty map. A dependency `.d.ts` that names an imported
/// type (lucide's `import { SVGProps } from 'react'`) then missed at check time.
/// Keyed by file name and rebuilt per program, so consulting it is the same
/// authoritative fallback `module_scope_for_file` already provides. Released in
/// the end-of-run teardown.
pub(super) static PROGRAM_MODULE_SCOPES: std::sync::Mutex<
    Option<Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<crate::symbols::TypeDeclarationScope>>>>,
> = std::sync::Mutex::new(None);

pub(crate) fn publish_program_module_scopes(
    scopes: &Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<crate::symbols::TypeDeclarationScope>>>,
) {
    if let Ok(mut slot) = PROGRAM_MODULE_SCOPES.lock() {
        *slot = Some(scopes.clone());
    }
}

pub(crate) fn program_module_scope_for_file(
    file_name: &str,
) -> Option<Arc<crate::symbols::TypeDeclarationScope>> {
    let slot = PROGRAM_MODULE_SCOPES.lock().ok()?;
    slot.as_ref()?.get(file_name).cloned()
}

/// Program-global copy of the ambient global value table as the check phase
/// sees it, published when that phase starts. A lazy body return resolves in an
/// environment captured during module analysis, before global augmentations
/// were merged (vitest's `declare global { let vi: … }`), and its body names
/// the globals the file's own check does. Released in the end-of-run teardown.
pub(super) static PROGRAM_AMBIENT_GLOBALS: std::sync::Mutex<Option<Arc<crate::symbols::SymbolTable>>> =
    std::sync::Mutex::new(None);

pub(crate) fn publish_program_ambient_globals(globals: Arc<crate::symbols::SymbolTable>) {
    if let Ok(mut slot) = PROGRAM_AMBIENT_GLOBALS.lock() {
        *slot = Some(globals);
    }
}

pub(crate) fn program_ambient_globals() -> Option<Arc<crate::symbols::SymbolTable>> {
    PROGRAM_AMBIENT_GLOBALS.lock().ok()?.clone()
}

pub(crate) fn program_module_scopes()
-> Option<Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<crate::symbols::TypeDeclarationScope>>>> {
    PROGRAM_MODULE_SCOPES.lock().ok()?.clone()
}

/// Each file's JSX pragmas, published when the check phase starts: a value
/// another file's check resolves lazily reads its JSX under the declaring
/// file's `@jsx` factory, not under the reading file's. Released in the
/// end-of-run teardown.
pub(super) static PROGRAM_JSX_FACTORY_USES: std::sync::Mutex<
    Option<Arc<surge_ts_types::fx::FxHashMap<Arc<str>, surge_ts_syntax::JsxFactoryUses>>>,
> = std::sync::Mutex::new(None);

pub(crate) fn publish_program_jsx_factory_uses(
    uses: surge_ts_types::fx::FxHashMap<Arc<str>, surge_ts_syntax::JsxFactoryUses>,
) {
    if let Ok(mut slot) = PROGRAM_JSX_FACTORY_USES.lock() {
        *slot = Some(Arc::new(uses));
    }
}

/// The JSX pragmas `file_name` was parsed with, once the check phase published
/// them; `None` before then.
pub(crate) fn program_jsx_factory_uses(file_name: &str) -> Option<surge_ts_syntax::JsxFactoryUses> {
    let slot = PROGRAM_JSX_FACTORY_USES.lock().ok()?;
    Some(slot.as_ref()?.get(file_name).cloned().unwrap_or_default())
}

/// Installs the JSX pragmas of the file a recovered context was switched to.
pub(crate) fn install_program_jsx_factory_uses(ctx: &mut crate::context::CheckerContext) {
    if let Some(uses) = program_jsx_factory_uses(&ctx.file_name) {
        ctx.jsx_factory_uses = uses;
        ctx.jsx_namespace = None;
    }
}

pub(crate) fn clear_program_module_scopes() {
    if let Ok(mut slot) = PROGRAM_MODULE_SCOPES.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = PROGRAM_AMBIENT_GLOBALS.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = PROGRAM_JSX_FACTORY_USES.lock() {
        *slot = None;
    }
}
