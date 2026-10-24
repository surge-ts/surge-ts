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
    if !on {
        crate::context::CHECK_STAGE.store(0, std::sync::atomic::Ordering::Relaxed);
    }
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

/// How `typeToString` qualifies a non-local type whose id does not say it:
/// its path in its own module where the id it was first read by names it
/// otherwise (`dom.JSX.Element` read as `Element`, `F` read through `import *
/// as A` as `A.F`), and whether it is a global, which no `import("m")` names.
/// Released in the end-of-run teardown.
#[derive(Clone)]
pub(crate) struct DeclaredPath {
    pub(crate) path: Option<Arc<str>>,
    pub(crate) global: bool,
}

static DECLARED_PATHS: std::sync::Mutex<Option<surge_ts_types::fx::FxHashMap<Arc<str>, DeclaredPath>>> =
    std::sync::Mutex::new(None);

pub(crate) fn record_declared_path(id: &Arc<str>, path: Option<&str>, global: bool) {
    if let Ok(mut slot) = DECLARED_PATHS.lock() {
        slot.get_or_insert_with(Default::default)
            .entry(id.clone())
            .or_insert_with(|| DeclaredPath { path: path.map(Arc::from), global });
    }
}

pub(crate) fn declared_path(id: &str) -> Option<DeclaredPath> {
    DECLARED_PATHS.lock().ok()?.as_ref()?.get(id).cloned()
}

/// Program-global copy of the global value table and every script file's own
/// values, published when the check phase starts: a lazy resolution's
/// recovered context holds none of them, so a key naming one (`declare const
/// ck: unique symbol` in a script) would read as naming nothing. Released in
/// the end-of-run teardown.
pub(super) static PROGRAM_SCRIPT_VALUES: std::sync::Mutex<Option<Arc<[Option<Arc<crate::symbols::SymbolTable>>]>>> =
    std::sync::Mutex::new(None);

pub(crate) fn publish_program_script_values(values: &[Option<Arc<crate::symbols::SymbolTable>>]) {
    if let Ok(mut slot) = PROGRAM_SCRIPT_VALUES.lock() {
        *slot = Some(values.into());
    }
}

pub(crate) fn program_script_value_is_known(name: &str) -> bool {
    let Ok(slot) = PROGRAM_SCRIPT_VALUES.lock() else {
        return false;
    };
    slot.as_ref()
        .is_some_and(|tables| tables.iter().flatten().any(|table| table.get(name).is_some()))
}

/// The type declarations of modules that no module exports, by
/// `file\0name`: declaration emit cannot name them from another file.
/// Published before the resolved export tables are released; cleared in the
/// end-of-run teardown.
#[derive(Default)]
pub(crate) struct UnexportedTypeDeclarations {
    pub(crate) unexported: surge_ts_types::fx::FxHashMap<Arc<str>, UnexportedTypeKind>,
    /// Package declarations no portable specifier reaches, with the module
    /// path a report names: a package nested in another's `node_modules`, or
    /// a file its package's `exports` does not expose and whose entry points
    /// do not re-export the declaration.
    pub(crate) unportable: surge_ts_types::fx::FxHashMap<Arc<str>, Arc<str>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnexportedTypeKind {
    Interface,
    Alias,
    Class,
}

pub(super) static PROGRAM_UNEXPORTED_TYPES: std::sync::Mutex<Option<Arc<UnexportedTypeDeclarations>>> =
    std::sync::Mutex::new(None);

pub(crate) fn publish_program_unexported_types(names: UnexportedTypeDeclarations) {
    if let Ok(mut slot) = PROGRAM_UNEXPORTED_TYPES.lock() {
        *slot = Some(Arc::new(names));
    }
}

pub(crate) fn program_unexported_types() -> Option<Arc<UnexportedTypeDeclarations>> {
    PROGRAM_UNEXPORTED_TYPES.lock().ok()?.clone()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibDeclarationKind {
    Value,
    Function,
    Class,
    Interface,
    TypeAlias,
    Namespace,
}

/// A default-lib declaration: its file, its name's span and what it declares.
pub(crate) type LibDeclarationSite = (Arc<str>, surge_ts_syntax::TextSpan, LibDeclarationKind);

/// A default-lib interface's index signature: its file, whether its key is
/// `number` rather than `string`, and its span.
pub(crate) type LibIndexSite = (Arc<str>, bool, surge_ts_syntax::TextSpan);

#[derive(Default)]
pub(crate) struct LibDeclarationSites {
    pub(crate) declarations: surge_ts_types::fx::FxHashMap<Arc<str>, Vec<LibDeclarationSite>>,
    pub(crate) indexes: surge_ts_types::fx::FxHashMap<Arc<str>, Vec<LibIndexSite>>,
}

/// Where each default-lib file declares a top-level name, published when the
/// check phase starts: a script declaration that collides with a lib global
/// is a duplicate at the lib's declarations too. Released in the end-of-run
/// teardown.
pub(super) static PROGRAM_LIB_DECLARATION_SITES: std::sync::Mutex<Option<Arc<LibDeclarationSites>>> =
    std::sync::Mutex::new(None);

pub(crate) fn publish_program_lib_declaration_sites(sites: LibDeclarationSites) {
    if let Ok(mut slot) = PROGRAM_LIB_DECLARATION_SITES.lock() {
        *slot = Some(Arc::new(sites));
    }
}

/// The default-lib files' top-level declarations of `name`.
pub(crate) fn program_lib_declaration_sites(name: &str) -> Vec<LibDeclarationSite> {
    PROGRAM_LIB_DECLARATION_SITES
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref()?.declarations.get(name).cloned())
        .unwrap_or_default()
}

/// The index signatures the default-lib declarations of interface `name`
/// declare.
pub(crate) fn program_lib_index_sites(name: &str) -> Vec<LibIndexSite> {
    PROGRAM_LIB_DECLARATION_SITES
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref()?.indexes.get(name).cloned())
        .unwrap_or_default()
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
    if let Ok(mut slot) = PROGRAM_SCRIPT_VALUES.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = DECLARED_PATHS.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = PROGRAM_JSX_FACTORY_USES.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = PROGRAM_LIB_DECLARATION_SITES.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = PROGRAM_UNEXPORTED_TYPES.lock() {
        *slot = None;
    }
}

/// What the slots above hold for one program. A retained program skips the
/// end-of-run teardown, so it takes this after its check and puts it back
/// each time it is entered: another program's check replaces it in between.
#[derive(Clone, Default)]
pub(crate) struct PublishedProgramState {
    module_scopes: Option<Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<crate::symbols::TypeDeclarationScope>>>>,
    ambient_globals: Option<Arc<crate::symbols::SymbolTable>>,
    script_values: Option<Arc<[Option<Arc<crate::symbols::SymbolTable>>]>>,
    jsx_factory_uses: Option<Arc<surge_ts_types::fx::FxHashMap<Arc<str>, surge_ts_syntax::JsxFactoryUses>>>,
    lib_declaration_sites: Option<Arc<LibDeclarationSites>>,
    unexported_types: Option<Arc<UnexportedTypeDeclarations>>,
}

pub(crate) fn published_program_state() -> PublishedProgramState {
    fn read<T: Clone>(slot: &std::sync::Mutex<Option<T>>) -> Option<T> {
        slot.lock().ok()?.clone()
    }
    PublishedProgramState {
        module_scopes: read(&PROGRAM_MODULE_SCOPES),
        ambient_globals: read(&PROGRAM_AMBIENT_GLOBALS),
        script_values: read(&PROGRAM_SCRIPT_VALUES),
        jsx_factory_uses: read(&PROGRAM_JSX_FACTORY_USES),
        lib_declaration_sites: read(&PROGRAM_LIB_DECLARATION_SITES),
        unexported_types: read(&PROGRAM_UNEXPORTED_TYPES),
    }
}

pub(crate) fn install_published_program_state(state: &PublishedProgramState) {
    fn write<T: Clone>(slot: &std::sync::Mutex<Option<T>>, value: &Option<T>) {
        if let Ok(mut slot) = slot.lock() {
            *slot = value.clone();
        }
    }
    write(&PROGRAM_MODULE_SCOPES, &state.module_scopes);
    write(&PROGRAM_AMBIENT_GLOBALS, &state.ambient_globals);
    write(&PROGRAM_SCRIPT_VALUES, &state.script_values);
    write(&PROGRAM_JSX_FACTORY_USES, &state.jsx_factory_uses);
    write(&PROGRAM_LIB_DECLARATION_SITES, &state.lib_declaration_sites);
    write(&PROGRAM_UNEXPORTED_TYPES, &state.unexported_types);
}
