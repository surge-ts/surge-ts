//! Ambient global and ambient-module (`declare module "..."`) collection passes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use surge_ts_syntax::ParsedStatement;

use super::*;

use crate::context::{CheckerContext, FileKind};
use crate::driver::collect_type_declarations;
use crate::modules::{ModuleExportTable, build_module_export_table};

/// See the comment at the ambient block-import binding phase in
/// [`collect_ambient_modules`]. Default-on (opt-out `SURGE_AMBIENT_BLOCK_IMPORTS=0`):
/// an import written inside `declare module "..."` binds, which is what
/// TypeScript does and the only way a block-internal import is visible from a
/// declaration body.
///
/// It was opt-in while resolving the @types/node graph the bound imports open
/// up cost +36% user time on tRPC. Re-measured 2026-09-14 on an interleaved
/// A/B (three reps, same frozen binary): trpc user time 4.07 s off vs 3.98 s
/// on and peak RSS unchanged, zod within noise, and the diagnostic set is
/// identical on all nine corpora — the memoization work that landed since
/// absorbed the cost, so the reason to keep it off is gone.
fn ambient_block_imports_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("SURGE_AMBIENT_BLOCK_IMPORTS").is_none_or(|v| v != "0"))
}
use crate::symbols::{SymbolTable, TypeDeclarationScope, TypeDeclarationTable};

#[derive(Debug, Clone)]
pub(crate) struct AmbientModuleEntry {
    module_specifier: String,
    file: ParsedProgramFile,
    raw_export_table: ModuleExportTable,
    /// The block's own type declarations, retained so the per-file ambient
    /// scope can be published after every block is registered.
    block_scope: Arc<TypeDeclarationScope>,
}

/// Collects the program's UMD globals: every `export as namespace X` in a file
/// that is itself a module, with that file. `X` is a global naming the module:
/// its types are reachable from every file, and a value reference to it from a
/// module is TS2686, which is what [`CheckerContext::umd_global_names`] drives.
/// A `export as namespace` in a script file declares nothing extra — the file's
/// declarations are already global — so only modules contribute.
pub(crate) fn collect_umd_global_names(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    let mut names: surge_ts_types::fx::FxHashMap<Arc<str>, usize> = Default::default();

    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !parsed_file.is_module {
            continue;
        }

        for statement in &parsed_file.statements {
            if let ParsedStatement::ExportDeclaration(export) = statement
                && let surge_ts_syntax::ParsedExportDeclaration::NamespaceExport {
                    exported_name,
                    ..
                } = export.as_ref()
            {
                names
                    .entry(Arc::from(exported_name.as_str()))
                    .or_insert(file_index);
            }
        }
    }

    ctx.umd_global_names = Arc::new(names);
}

/// tsc's `bindNamespaceExportDeclaration` and its merge into the globals:
/// `export as namespace X` in a declaration module binds `X` globally as an
/// alias of the module (`resolveExternalModuleSymbol`) — the `export =`
/// entity's value, type and namespace members, or the module namespace. A
/// script reads the value freely; a module's value use is TS2686, and its type
/// uses need no import. A global some script or global augmentation declares
/// keeps its own binding, and since tsc reports TS2686 only when every
/// declaration of the merged global is a UMD export, the name then stops being
/// a UMD global.
pub(crate) fn bind_umd_globals(
    parsed_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    let globally_declared = super::globals::globally_declared_names(parsed_files);
    let mut bound: HashSet<&str> = HashSet::new();
    for (index, parsed_file) in parsed_files.iter().enumerate() {
        if !parsed_file.is_module || !super::file_classify::is_declaration_file_name(&parsed_file.file_name) {
            continue;
        }
        for statement in &parsed_file.statements {
            let ParsedStatement::ExportDeclaration(export) = statement else {
                continue;
            };
            let surge_ts_syntax::ParsedExportDeclaration::NamespaceExport { exported_name, .. } =
                export.as_ref()
            else {
                continue;
            };
            let Some(Some(export_table)) = module_export_tables.get(index) else {
                continue;
            };
            let scope = module_resolution_scopes.get(index).and_then(Option::as_ref);
            let merged = !bound.contains(exported_name.as_str())
                && globally_declared.contains(exported_name.as_str());
            if merged {
                Arc::make_mut(&mut ctx.umd_global_names).remove(exported_name.as_str());
            }
            // With no global declaration of its own, the name is the alias
            // alone: its value replaces any entry surge put there some other
            // way (a module's own `declare namespace X` of the same name).
            if bound.insert(exported_name.as_str()) && !merged {
                let value = crate::modules::external_module_value(
                    export_table,
                    Some(index),
                    "",
                    parsed_files,
                    ctx,
                );
                let _ = ctx.ambient_global_symbols.insert_shared(exported_name.as_str(), value);
            }
            let (entity_type, members) = crate::modules::external_module_type_bindings(
                export_table,
                scope,
                Some(index),
                exported_name,
            );
            let globals = Arc::make_mut(&mut ctx.ambient_global_type_declarations);
            if let Some(declaration) = entity_type
                && globals.get(exported_name).is_none()
            {
                crate::modules::exports::insert_type_export(globals, exported_name, scope, declaration);
            }
            for (key, declaration) in members.iter().flat_map(|members| members.iter()) {
                if globals.get(key).is_none() {
                    let _ = globals.insert(key.as_ref(), declaration.clone());
                }
            }
        }
    }
}

/// Every name a file binds at module scope, by syntax alone. A UMD global is
/// shadowed by any such declaration whether or not surge managed to bind it, so
/// this must not be derived from the analysis tables.
pub(crate) fn module_scope_declared_names(statements: &[ParsedStatement]) -> HashSet<&str> {
    fn collect<'a>(statements: &'a [ParsedStatement], names: &mut HashSet<&'a str>) {
        for statement in statements {
            match statement {
                ParsedStatement::VariableDeclaration(variable) => {
                    names.insert(variable.name.as_str());
                }
                ParsedStatement::FunctionDeclaration(function) => {
                    names.insert(function.name.as_str());
                }
                ParsedStatement::ClassDeclaration(class) => {
                    names.insert(class.name.as_str());
                }
                ParsedStatement::InterfaceDeclaration(interface) => {
                    names.insert(interface.name.as_str());
                }
                ParsedStatement::TypeAliasDeclaration(alias) => {
                    names.insert(alias.name.as_str());
                }
                ParsedStatement::NamespaceDeclaration(namespace) => {
                    names.insert(namespace.name.as_str());
                }
                ParsedStatement::ExportDeclaration(export) => {
                    if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref()
                    {
                        collect(std::slice::from_ref(declaration.as_ref()), names);
                    }
                }
                _ => {}
            }
        }
    }

    let mut names = HashSet::new();
    collect(statements, &mut names);
    names.extend(import_bound_names(statements));
    names
}

/// [`module_scope_declared_names`] minus the import bindings: the names the file
/// declares itself. A type-only import of a name the file also declares is a
/// duplicate-identifier error, not a type-only value use, so the declaration
/// wins.
pub(crate) fn module_scope_own_declared_names(statements: &[ParsedStatement]) -> HashSet<&str> {
    let imported = import_bound_names(statements);
    module_scope_declared_names(statements)
        .into_iter()
        .filter(|name| !imported.contains(name))
        .collect()
}

/// Every local name a file's imports bind, type-only imports included. A
/// type-only binding still shadows a same-named global — tsc reports using it
/// as a value as TS1361, not as a UMD-global reference — and an import whose
/// module fails to resolve binds the name just as much, so this reads the
/// syntax rather than the resolved binding tables.
/// The local names of a file's namespace imports (`import * as ns`), whose
/// members are read-only properties of the module namespace object.
pub(crate) fn namespace_import_names(statements: &[ParsedStatement]) -> HashSet<&str> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::ImportDeclaration(import) => match &import.kind {
                surge_ts_syntax::ParsedImportKind::Namespace { local_name, .. } => {
                    Some(local_name.as_str())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(crate) fn import_bound_names(statements: &[ParsedStatement]) -> HashSet<&str> {
    let mut names = HashSet::new();

    for statement in statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };

        match &import.kind {
            surge_ts_syntax::ParsedImportKind::Named { specifiers, .. } => {
                names.extend(
                    specifiers
                        .iter()
                        .map(|specifier| specifier.local_name.as_str()),
                );
            }
            surge_ts_syntax::ParsedImportKind::DefaultAndNamed {
                local_name,
                specifiers,
                ..
            } => {
                names.insert(local_name.as_str());
                names.extend(
                    specifiers
                        .iter()
                        .map(|specifier| specifier.local_name.as_str()),
                );
            }
            surge_ts_syntax::ParsedImportKind::Default { local_name, .. }
            | surge_ts_syntax::ParsedImportKind::Namespace { local_name, .. }
            | surge_ts_syntax::ParsedImportKind::Equals { local_name, .. }
            | surge_ts_syntax::ParsedImportKind::EntityAlias { local_name, .. }
            | surge_ts_syntax::ParsedImportKind::TypeOnlyDefault { local_name, .. } => {
                names.insert(local_name.as_str());
            }
            surge_ts_syntax::ParsedImportKind::SideEffect
            | surge_ts_syntax::ParsedImportKind::Unsupported => {}
        }
    }

    names
}

/// The local names a file's imports bind **in type space only**
/// (`import type X from …`, `import type { X } from …`,
/// `import type * as X from …`, `import type X = require(…)`). Referencing one
/// as a value is TS1361.
pub(crate) fn type_only_import_bound_names(statements: &[ParsedStatement]) -> HashSet<&str> {
    let mut names = HashSet::new();

    for statement in statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };

        match &import.kind {
            surge_ts_syntax::ParsedImportKind::Named {
                is_type_only: true,
                specifiers,
            } => {
                names.extend(
                    specifiers
                        .iter()
                        .map(|specifier| specifier.local_name.as_str()),
                );
            }
            surge_ts_syntax::ParsedImportKind::DefaultAndNamed {
                local_name,
                is_type_only: true,
                specifiers,
                ..
            } => {
                names.insert(local_name.as_str());
                names.extend(
                    specifiers
                        .iter()
                        .map(|specifier| specifier.local_name.as_str()),
                );
            }
            surge_ts_syntax::ParsedImportKind::Namespace {
                local_name,
                is_type_only: true,
                ..
            }
            | surge_ts_syntax::ParsedImportKind::Equals {
                local_name,
                is_type_only: true,
                ..
            }
            | surge_ts_syntax::ParsedImportKind::TypeOnlyDefault { local_name, .. } => {
                names.insert(local_name.as_str());
            }
            _ => {}
        }
    }

    names
}

/// Collects and merges every ambient global *type* declaration across all
/// declaration files. The default lib graph splits a single global interface
/// across files (e.g. `ArrayConstructor` gains `isArray` in lib.es5 and
/// `from`/`of` in lib.es2015.core), and a `declare var Array: ArrayConstructor`
/// would otherwise freeze the variable's type against whatever members were
/// merged when its own file was processed, dropping members contributed by files
/// processed later.
///
/// Runs before [`collect_global_augmentation_types`][crate::driver::collect_global_augmentations]
/// so the *ambient* declaration is the merge base. A merged interface takes its
/// declaring file and resolution scope from whichever fragment merged first, and
/// only the ambient one can resolve the member annotations: letting a
/// `declare global` block win that role resolved `NodeJS.Process`'s members
/// under the augmenting module's scope, where none of their type names exist.
pub(crate) fn collect_ambient_global_types(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) {
    for parsed_file in parsed_files {
        if !(is_ambient_global_declaration_file(parsed_file, ctx)
            || is_root_source_script(parsed_file))
            || !publishes_ambient_globals(parsed_file)
        {
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
        ctx.type_declaration_scope = None;

        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
        let collect_start = Instant::now();
        collect_type_declarations(&parsed_file.statements, ctx);
        let ambient_td =
            crate::symbols::attach_global_declaration_scope(&std::mem::take(&mut ctx.type_declarations));
        let lowered_type_declarations = ambient_td.len() as u64;
        let collect_duration = collect_start.elapsed();
        record_program_file_timing(timings, &parsed_file.file_name, |metrics| {
            metrics.collect_type_declarations_passes += 1;
            metrics.lowered_type_declarations += lowered_type_declarations;
            metrics.collect_type_declarations_duration += collect_duration;
        });
        record_program_timing(timings, |timings| {
            timings.dependency_declaration_collection += collect_duration;
            timings.dependency_declaration_lower_time += collect_duration;
        });

        // Declaration merging across global declaration files: the same
        // interface (a default lib's `Window`, or a project's split global
        // `interface Env`) contributes members from every declaration rather
        // than being dropped first-wins.
        crate::symbols::merge_shared_table_into(
            Arc::make_mut(&mut ctx.ambient_global_type_declarations),
            &ambient_td,
        );

        ctx.type_declarations = saved_type_declarations;
        ctx.type_declaration_scope = saved_type_declaration_scope;
    }
}

/// Lowers ambient value symbols (functions, `declare var`s, and `declare class`
/// constructors) against the fully-merged type table, so a variable typed by a
/// split global interface sees every member.
///
/// Runs *after* the `declare global` augmentation types are merged:
/// @types/node's `globals.d.ts` (a script) declares `var process:
/// NodeJS.Process` while the interface's members are re-opened from
/// `process.d.ts` inside a `declare global`, so lowering the value before that
/// merge froze `process` against whatever partial `NodeJS.Process` existed.
pub(crate) fn lower_ambient_global_values(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) -> Vec<GlobalThisQueryingValue> {
    let lowered_files: Vec<&ParsedProgramFile> = parsed_files
        .iter()
        .filter(|parsed_file| {
            is_ambient_global_declaration_file(parsed_file, ctx) && publishes_ambient_globals(parsed_file)
        })
        .collect();
    // tsc types a value on demand, so a type query in an ambient annotation
    // reads a variable declared after it (`declare var S: typeof A;
    // declare const A: number;`) — `isBlockScopedNameDeclaredBeforeUse` holds
    // for any use in a type query or ambient context. A variable whose
    // annotation queries one not lowered yet waits until that one is.
    let mut pending: HashSet<&str> = lowered_files
        .iter()
        .flat_map(|parsed_file| parsed_file.statements.iter().filter_map(ambient_variable))
        .map(|var| var.name.as_str())
        .collect();
    let mut deferred: Vec<(&ParsedProgramFile, &surge_ts_syntax::ParsedVariableDeclaration)> = Vec::new();
    let mut global_this_querying = Vec::new();
    let global_this_pending: HashSet<&str> = ctx
        .ambient_global_symbols
        .get("globalThis")
        .is_none()
        .then_some("globalThis")
        .into_iter()
        .collect();
    let mut lower = |parsed_file: &ParsedProgramFile,
                     var: &surge_ts_syntax::ParsedVariableDeclaration,
                     ctx: &mut CheckerContext| {
        if lower_ambient_variable(var, ctx) && queries_pending_value(var, &global_this_pending) {
            global_this_querying.push(GlobalThisQueryingValue {
                file_name: parsed_file.file_name.clone(),
                var: var.clone(),
            });
        }
    };
    for &parsed_file in &lowered_files {
        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
        ctx.type_declaration_scope = None;
        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());

        let mut local_function_signatures = HashMap::new();
        let mut current_symbols = std::mem::take(&mut ctx.symbols);
        collect_function_signatures_from_statements(
            &parsed_file.statements,
            0,
            &mut current_symbols,
            &mut local_function_signatures,
            ctx,
        );
        ctx.symbols = current_symbols;

        for var in parsed_file.statements.iter().filter_map(ambient_variable) {
            let waits = deferred.iter().any(|(_, earlier)| earlier.name == var.name)
                || queries_pending_value(var, &pending);
            if waits {
                deferred.push((parsed_file, var));
            } else {
                lower(parsed_file, var, ctx);
                pending.remove(var.name.as_str());
            }
        }

        let mut ordered_function_signatures =
            local_function_signatures.into_iter().collect::<Vec<_>>();
        ordered_function_signatures
            .sort_by_key(|(location, _)| (location.file_index, location.statement_index));
        for (loc, fun_ty) in ordered_function_signatures {
            let name = match &parsed_file.statements[loc.statement_index] {
                ParsedStatement::FunctionDeclaration(f) => f.name.clone(),
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    surge_ts_syntax::ParsedExportDeclaration::Default {
                        declaration: surge_ts_syntax::ParsedDefaultExportDeclaration::Function(f),
                        ..
                    } => f.name.clone(),
                    surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => {
                        if let ParsedStatement::FunctionDeclaration(f) = declaration.as_ref() {
                            f.name.clone()
                        } else {
                            "unknown".to_string()
                        }
                    }
                    _ => "unknown".to_string(),
                },
                _ => "unknown".to_string(),
            };

            if let Some(merged) = crate::driver::merged_global_function_type(
                ctx.ambient_global_symbols.get(&name),
                fun_ty,
            ) {
                ctx.ambient_global_symbols.insert(
                    name,
                    crate::symbols::SymbolInfo {
                        ty: surge_ts_types::Type::Function(merged),
                        kind: crate::symbols::SymbolKind::Function,
                        function_signature: None,
                    },
                );
            }
        }

        // `declare class` contributes a global constructor/static value. The
        // instance interface is already in `ambient_global_type_declarations`
        // above, so the value's construct signature and member types resolve.
        for stmt in &parsed_file.statements {
            let class = match stmt {
                ParsedStatement::ClassDeclaration(class) => Some(class),
                ParsedStatement::ExportDeclaration(export) => {
                    if let surge_ts_syntax::ParsedExportDeclaration::Statement {
                        declaration, ..
                    } = export.as_ref()
                    {
                        if let ParsedStatement::ClassDeclaration(class) = declaration.as_ref() {
                            Some(class)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(class) = class {
                if ctx.ambient_global_symbols.get(&class.name).is_none() {
                    let symbol = super::build_class_value_symbol(class, ctx);
                    ctx.ambient_global_symbols
                        .insert(class.name.clone(), symbol);
                }
            }
        }

        ctx.type_declarations = saved_type_declarations;
        ctx.type_declaration_scope = saved_type_declaration_scope;
    }

    while !deferred.is_empty() {
        let ready = deferred.iter().position(|(_, var)| {
            let mut others = pending.clone();
            others.remove(var.name.as_str());
            !queries_pending_value(var, &others)
        });
        // A cycle has no ready variable; its members lower in source order.
        let (parsed_file, var) = deferred.remove(ready.unwrap_or(0));
        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.take();
        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
        lower(parsed_file, var, ctx);
        ctx.type_declarations = saved_type_declarations;
        ctx.type_declaration_scope = saved_type_declaration_scope;
        if !deferred.iter().any(|(_, later)| later.name == var.name) {
            pending.remove(var.name.as_str());
        }
    }

    lower_ambient_namespace_values(parsed_files, ctx);
    global_this_querying
}

/// An ambient variable whose annotation queried `typeof globalThis` before the
/// global object existed: lib.dom's `declare var window: Window & typeof
/// globalThis`. Lowering dropped the query (`T & unknown` is `T`), so the
/// value lacks the script globals tsc reads through it (`window.a` for a
/// script's `var a`) until [`relower_global_this_querying_values`] runs.
pub(crate) struct GlobalThisQueryingValue {
    file_name: String,
    var: surge_ts_syntax::ParsedVariableDeclaration,
}

/// Re-lowers `values` once `typeof globalThis` is installed, keeping each
/// symbol's kind.
pub(crate) fn relower_global_this_querying_values(
    values: &[GlobalThisQueryingValue],
    ctx: &mut CheckerContext,
) {
    if values.is_empty() || ctx.ambient_global_symbols.get("globalThis").is_none() {
        return;
    }
    let original_file_name = ctx.file_name.clone();
    for value in values {
        let Some(declared_type) = value.var.declared_type.as_ref() else {
            continue;
        };
        let Some(mut symbol) = ctx.ambient_global_symbols.get(&value.var.name).cloned() else {
            continue;
        };
        ctx.set_file_name(value.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.take();
        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
        symbol.ty = crate::infer::map_parsed_type(declared_type.clone(), ctx);
        ctx.type_declarations = saved_type_declarations;
        ctx.type_declaration_scope = saved_type_declaration_scope;
        ctx.ambient_global_symbols.insert(value.var.name.clone(), symbol);
    }
    ctx.set_file_name(original_file_name);
}

fn ambient_variable(statement: &ParsedStatement) -> Option<&surge_ts_syntax::ParsedVariableDeclaration> {
    match crate::modules::peel_exported_statement(statement) {
        ParsedStatement::VariableDeclaration(var) => Some(var),
        _ => None,
    }
}

/// Whether `var`'s annotation queries the value of a variable in `pending`.
pub(crate) fn queries_pending_value(
    var: &surge_ts_syntax::ParsedVariableDeclaration,
    pending: &HashSet<&str>,
) -> bool {
    fn queries(ty: &surge_ts_syntax::ParsedType, pending: &HashSet<&str>) -> bool {
        use surge_ts_syntax::ParsedType;
        let signature = |function: &surge_ts_syntax::ParsedFunctionType| {
            function.parameters.iter().any(|parameter| queries(&parameter.ty, pending))
                || queries(&function.return_type, pending)
        };
        match ty {
            ParsedType::TypeOf(type_of) => {
                type_of.import_specifier.is_none() && pending.contains(type_of.name.as_str())
            }
            ParsedType::Named(named) => named.type_arguments.iter().any(|argument| queries(argument, pending)),
            ParsedType::Array(element) | ParsedType::Readonly(element) | ParsedType::KeyOf(element) => {
                queries(element, pending)
            }
            ParsedType::Tuple(elements) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
                elements.iter().any(|element| queries(element, pending))
            }
            ParsedType::Object(object) => {
                object.properties.iter().any(|property| queries(&property.ty, pending))
                    || object.call_signature.as_deref().is_some_and(signature)
            }
            ParsedType::Function(function) => signature(function),
            ParsedType::IndexedAccess(indexed) => {
                queries(&indexed.object_type, pending) || queries(&indexed.index_type, pending)
            }
            ParsedType::Conditional(conditional) => {
                queries(&conditional.check_type, pending)
                    || queries(&conditional.extends_type, pending)
                    || queries(&conditional.true_type, pending)
                    || queries(&conditional.false_type, pending)
            }
            _ => false,
        }
    }
    var.declared_type.as_ref().is_some_and(|ty| queries(ty, pending))
}

/// Declares an ambient variable in the global table unless an earlier
/// declaration of the name already did.
/// Returns whether `var` installed the symbol (an earlier declaration wins).
fn lower_ambient_variable(var: &surge_ts_syntax::ParsedVariableDeclaration, ctx: &mut CheckerContext) -> bool {
    let ty = var
        .declared_type
        .as_ref()
        .map(|ty| crate::infer::map_parsed_type(ty.clone(), ctx))
        .unwrap_or(surge_ts_types::Type::Unknown);
    if ctx.ambient_global_symbols.get(&var.name).is_none() {
        if !matches!(var.kind, surge_ts_syntax::ParsedVariableKind::Var) {
            Arc::make_mut(&mut ctx.block_scoped_globals).insert(Arc::from(var.name.as_str()));
        }
        ctx.ambient_global_symbols.insert(
            var.name.clone(),
            crate::symbols::SymbolInfo {
                ty,
                kind: if matches!(var.kind, surge_ts_syntax::ParsedVariableKind::Const) {
                    crate::symbols::SymbolKind::Const
                } else {
                    crate::symbols::SymbolKind::Let
                },
                function_signature: None,
            },
        );
        return true;
    }
    false
}

/// `declare namespace X { ... }` contributes a global value object whose members
/// are the namespace's declarations. roblox-ts's lib uses this heavily (`math`,
/// `task`, `utf8`, `buffer`, `vector`, `os`); user code accesses them as values
/// (`math.floor`, `task.wait`). Without this the name resolves only as a type
/// (TS2693) or not at all (TS2304).
///
/// Namespaces merge across blocks and files exactly like interfaces — roblox-ts's
/// `math` is split across declarations — so members are accumulated across every
/// ambient block of the same name before a single value symbol is inserted. A
/// real value symbol (`declare const`/`function`/`class` of the same name) takes
/// precedence and is left untouched.
fn lower_ambient_namespace_values(parsed_files: &[ParsedProgramFile], ctx: &mut CheckerContext) {
    use surge_ts_types::PropertyMap;

    let mut merged: HashMap<String, PropertyMap> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    let original_file_name = ctx.file_name.clone();
    for parsed_file in parsed_files {
        if !is_ambient_global_declaration_file(parsed_file, ctx) {
            continue;
        }
        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.take();
        let saved_type_declarations = std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());

        for stmt in &parsed_file.statements {
            let namespace = match stmt {
                ParsedStatement::NamespaceDeclaration(namespace) => Some(namespace),
                ParsedStatement::ExportDeclaration(export) => {
                    if let surge_ts_syntax::ParsedExportDeclaration::Statement {
                        declaration, ..
                    } = export.as_ref()
                    {
                        if let ParsedStatement::NamespaceDeclaration(namespace) =
                            declaration.as_ref()
                        {
                            Some(namespace)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            // Only an instantiated block gives the namespace a value side.
            if let Some(namespace) = namespace
                && super::is_instantiated_namespace(namespace)
            {
                let entry = merged.entry(namespace.name.clone()).or_insert_with(|| {
                    order.push(namespace.name.clone());
                    PropertyMap::default()
                });
                crate::modules::fill_namespace_value_properties(namespace, entry);
                crate::modules::resolve_ambient_namespace_value_annotations(namespace, entry, ctx);
            }
        }
        ctx.type_declarations = saved_type_declarations;
        ctx.type_declaration_scope = saved_type_declaration_scope;
    }
    ctx.set_file_name(original_file_name);

    let global_augmentation_value_names = global_augmentation_value_names(parsed_files);

    for name in order {
        let properties = merged.remove(&name).unwrap_or_default();
        if let Some(existing) = ctx.ambient_global_symbols.get(&name) {
            // A function or class the namespace merges with keeps its call or
            // construct signature and gains the namespace's exports.
            let merged_value = match &existing.ty {
                surge_ts_types::Type::Function(function) if !properties.is_empty() => Some(
                    crate::metrics::alloc_object_type(properties, None)
                        .with_call_signature(function.clone()),
                ),
                surge_ts_types::Type::Object(object)
                    if !properties.is_empty()
                        && (object.call_signature.is_some() || object.construct_signature.is_some()) =>
                {
                    let mut extended = object.clone();
                    let mut members = (*object.properties).clone();
                    for (member, property) in properties.iter() {
                        members.entry(member.clone()).or_insert_with(|| property.clone());
                    }
                    extended.properties = Arc::new(members);
                    extended.property_map_id = None;
                    Some(extended)
                }
                _ => None,
            };
            if let Some(merged_value) = merged_value {
                let symbol = crate::symbols::SymbolInfo {
                    ty: surge_ts_types::Type::Object(merged_value),
                    kind: existing.kind,
                    function_signature: existing.function_signature.clone(),
                };
                ctx.ambient_global_symbols.insert(name, symbol);
            }
            continue;
        }
        // A namespace with no value members contributes nothing but an empty
        // object, and this pass runs before `declare global` blocks are lowered
        // (`collect_global_augmentations`) — claiming the name here would freeze
        // it at `{}` and turn every member access into TS2339. bun-types is
        // exactly this shape: `declare namespace Bun { type … }` in one file,
        // `declare global { var Bun: typeof import("bun") }` in another.
        if properties.is_empty() && global_augmentation_value_names.contains(&name) {
            continue;
        }
        let alias_name = format!("typeof {name}");
        ctx.ambient_global_symbols.insert(
            name,
            crate::symbols::SymbolInfo {
                ty: surge_ts_types::Type::Object(
                    crate::metrics::alloc_object_type(properties, None).with_alias_name(alias_name),
                ),
                kind: crate::symbols::SymbolKind::Const,
                function_signature: None,
            },
        );
    }
}

/// Names bound as a global *value* by a `declare global { … }` block anywhere in
/// the program (including the `declare module "x" { global { … } }` nesting
/// `@types/node` uses). Those blocks are lowered after this pass, so their
/// values would otherwise lose the first-wins race against a same-named
/// ambient namespace.
fn global_augmentation_value_names(parsed_files: &[ParsedProgramFile]) -> HashSet<String> {
    fn collect_block_value_names(
        block_statements: &[ParsedStatement],
        names: &mut HashSet<String>,
    ) {
        for statement in block_statements {
            let inner = match statement {
                ParsedStatement::ExportDeclaration(export) => {
                    if let surge_ts_syntax::ParsedExportDeclaration::Statement {
                        declaration, ..
                    } = export.as_ref()
                    {
                        declaration.as_ref()
                    } else {
                        statement
                    }
                }
                other => other,
            };
            match inner {
                ParsedStatement::VariableDeclaration(var) => {
                    names.insert(var.name.clone());
                }
                ParsedStatement::FunctionDeclaration(function) => {
                    names.insert(function.name.clone());
                }
                ParsedStatement::ClassDeclaration(class) => {
                    names.insert(class.name.clone());
                }
                _ => {}
            }
        }
    }

    let mut names = HashSet::new();
    for parsed_file in parsed_files {
        for statement in &parsed_file.statements {
            let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
                continue;
            };
            if module.module_specifier == "global" {
                collect_block_value_names(&module.statements, &mut names);
                continue;
            }
            for nested in &module.statements {
                if let ParsedStatement::DeclareModuleDeclaration(inner) = nested
                    && inner.module_specifier == "global"
                {
                    collect_block_value_names(&inner.statements, &mut names);
                }
            }
        }
    }
    names
}

/// Whether a parsed file contributes to the ambient global scope. Declaration
/// files do, except dependency declarations that are not part of a configured
/// `@types/*` package (those reach the program only through module resolution).
fn is_ambient_global_declaration_file(
    parsed_file: &ParsedProgramFile,
    ctx: &CheckerContext,
) -> bool {
    if !parsed_file.file_kind.is_declaration() {
        return false;
    }

    if parsed_file.file_kind == FileKind::DependencyDeclaration
        && !is_configured_types_global_file(&parsed_file.file_name, &ctx.options.types)
    {
        return false;
    }

    true
}

/// A source file with no top-level import or export: Go's `initializeChecker`
/// merges its locals into the globals table, so a module names its types as it
/// names a declaration file's. A JavaScript script's JSDoc types are left to
/// the script pass: published this early, an `import("./m")` typedef resolves
/// before modules are bound and loses the TS1340 its use reports.
fn is_root_source_script(parsed_file: &ParsedProgramFile) -> bool {
    parsed_file.file_kind == FileKind::RootSource
        && !parsed_file.is_module
        && !surge_ts_syntax::is_javascript_file_name(&parsed_file.file_name)
}

/// Whether the file's own top-level declarations reach the *global* scope. A
/// declaration file carrying a top-level `import`/`export` is a module: its
/// declarations are module-scoped no matter how the file was reached, and it
/// augments the global scope only through `declare global` /
/// `declare module "x"` blocks, which are collected separately. Publishing a
/// module's declarations globally made the whole
/// `@types/express-serve-static-core` surface global (a `/// <reference types>`
/// directive in a dependency puts it in the effective `types` list), clobbering
/// the real global `Response` with express's `status(code)` and resolving bare
/// `NextFunction`/`ParamsDictionary` that tsc reports as unknown names.
///
/// Neither pass below can serve a module: both run before binding, so a module's
/// import scope does not exist yet and its annotations would resolve against the
/// global table alone. A module declaration file is therefore skipped outright.
fn publishes_ambient_globals(parsed_file: &ParsedProgramFile) -> bool {
    !parsed_file.is_module
}

/// Whether `file_name` belongs to one of the configured `compilerOptions.types`
/// packages. Two layouts contribute global declarations: DefinitelyTyped stubs
/// under `node_modules/@types/<mangled>` (scoped names map like TypeScript:
/// `@scope/pkg` -> `scope__pkg`), and packages that ship their own ambient
/// declarations directly under `node_modules/<name>` (e.g. roblox-ts's
/// `@rbxts/types` / `@rbxts/compiler-types`, which replace the default lib). The
/// `types` list passed here already includes packages discovered through
/// `/// <reference types="..." />` closure, so a referenced package is covered
/// even when only its referrer is named explicitly.
pub(crate) fn is_configured_types_global_file(file_name: &str, types: &[String]) -> bool {
    let normalized = file_name.replace('\\', "/");
    types.iter().any(|type_name| {
        if type_name == "*" {
            return false;
        }
        let mangled = mangle_types_package_name(type_name);
        let at_types = format!("/@types/{mangled}/");
        let direct = format!("/node_modules/{type_name}/");
        normalized.contains(&at_types) || normalized.contains(&direct)
    })
}

fn mangle_types_package_name(type_name: &str) -> String {
    type_name
        .strip_prefix('@')
        .map(|name| name.replace('/', "__"))
        .unwrap_or_else(|| type_name.to_string())
}

type AmbientBlockImports =
    surge_ts_types::fx::FxHashMap<(usize, usize), crate::modules::ModuleImportBindings>;

fn ambient_blocks_have_imports(parsed_files: &[ParsedProgramFile]) -> bool {
    parsed_files
        .iter()
        .flat_map(|parsed_file| &parsed_file.statements)
        .any(|statement| {
            matches!(
                statement,
                ParsedStatement::DeclareModuleDeclaration(module)
                    if module.module_specifier != "global"
                        && module.statements.iter().any(|statement| {
                            matches!(statement, ParsedStatement::ImportDeclaration(_))
                        })
            )
        })
}

/// Binds the imports written inside each file's `declare module`
/// blocks against the ambient tables registered so far. Resolution
/// diagnostics are dropped: an unresolvable block import stays silent, as it
/// always has.
fn bind_ambient_block_imports(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) -> AmbientBlockImports {
    let mut bindings = AmbientBlockImports::default();
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        for (statement_index, statement) in parsed_file.statements.iter().enumerate() {
            let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
                continue;
            };
            if module.module_specifier == "global"
                || !module
                    .statements
                    .iter()
                    .any(|statement| matches!(statement, ParsedStatement::ImportDeclaration(_)))
            {
                continue;
            }
            ctx.set_file_name(parsed_file.file_name.clone());
            let mut block_file = parsed_file.clone();
            block_file.statements = module.statements.clone();
            let diagnostics_before = ctx.diagnostics().len();
            let block_bindings =
                crate::modules::resolve_module_imports(&block_file, &[], &[], &[], &|_| false, ctx);
            ctx.truncate_diagnostics(diagnostics_before);
            bindings.insert((file_index, statement_index), block_bindings);
        }
    }
    bindings
}

fn register_ambient_blocks(
    parsed_files: &[ParsedProgramFile],
    block_imports: Option<&AmbientBlockImports>,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> Vec<AmbientModuleEntry> {
    let mut ambient_module_entries = Vec::<AmbientModuleEntry>::new();
    let mut ambient_module_indexes = HashMap::<String, usize>::new();
    let augmentation_types = syntactic_ambient_augmentation_types(parsed_files, block_imports, ctx);

    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        ctx.set_file_name(parsed_file.file_name.clone());
        let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
        ctx.type_declaration_scope = None;
        for (statement_index, statement) in parsed_file.statements.iter().enumerate() {
            let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
                continue;
            };
            let bound_imports = block_imports
                .and_then(|block_imports| block_imports.get(&(file_index, statement_index)));

            if module.module_specifier == "global" {
                continue;
            }

            let saved_type_declarations =
                std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
            let saved_symbols = std::mem::replace(&mut ctx.symbols, SymbolTable::new());

            // `declare module "buffer" { global { var Buffer: … } export { Buffer }; }`
            // resolves its own `export { … }` against the values its nested
            // `global` block declares, so those join the block's local scope.
            // Only the values: the block's *types* are merged program-wide by
            // `collect_global_augmentations`, and a module-local copy would
            // shadow that merge with one file's half of a split interface
            // (`BufferConstructor` spans buffer.d.ts and buffer.buffer.d.ts).
            // Nothing here reaches the export table on its own — only an
            // explicit `export` clause pulls a name out.
            let nested_global_statements: Vec<ParsedStatement> = module
                .statements
                .iter()
                .filter_map(|statement| match statement {
                    ParsedStatement::DeclareModuleDeclaration(nested)
                        if nested.module_specifier == "global" =>
                    {
                        Some(nested.statements.iter().cloned().map(|statement| {
                            // A `var` inside `declare global` is ambient by
                            // context, not by an explicit `declare` keyword.
                            match statement {
                                ParsedStatement::VariableDeclaration(mut declaration) => {
                                    declaration.is_declare = true;
                                    ParsedStatement::VariableDeclaration(declaration)
                                }
                                other => other,
                            }
                        }))
                    }
                    _ => None,
                })
                .flatten()
                .collect();

            let collect_start = Instant::now();
            collect_type_declarations(&module.statements, ctx);
            if !parsed_file.is_module
                && let Some(augmentation) = augmentation_types.get(&module.module_specifier)
            {
                for (name, declaration) in augmentation.iter() {
                    if ctx.type_declarations.get(name.as_ref()).is_some() {
                        crate::symbols::merge_augmentation_type_declaration_into_table(
                            &mut ctx.type_declarations,
                            name.as_ref(),
                            declaration,
                        );
                    }
                }
            }
            record_type_declaration_table_clone(
                timings,
                ctx.type_declarations.len(),
                TableCloneKind::General,
            );
            let block_declarations = Arc::new(ctx.type_declarations.clone());
            let block_scope = Arc::new(TypeDeclarationScope::new(vec![block_declarations.clone()]));
            let current_type_declarations_scope = match bound_imports {
                Some(bindings) => {
                    let mut layers = vec![block_declarations];
                    layers.extend(bindings.scope_layers());
                    Arc::new(TypeDeclarationScope::new(layers))
                }
                None => block_scope.clone(),
            };
            ctx.type_declaration_scope = Some(current_type_declarations_scope.clone());
            let mut local_function_signatures = HashMap::new();
            let mut current_symbols = std::mem::take(&mut ctx.symbols);
            collect_function_signatures_from_statements(
                &module.statements,
                0,
                &mut current_symbols,
                &mut local_function_signatures,
                ctx,
            );
            ctx.symbols = current_symbols;

            // A block's imports are its locals (Go's binder declares them in
            // the module's table), so `export var v: typeof lib` reads the value
            // `import lib = require("lib")` binds.
            let saved_value_fallback = bound_imports.map(|bindings| {
                std::mem::replace(
                    &mut ctx.module_value_fallback,
                    Some(Arc::new(bindings.symbols.clone())),
                )
            });
            for stmt in module.statements.iter().chain(&nested_global_statements) {
                match stmt {
                    ParsedStatement::VariableDeclaration(var) => {
                        if var.is_declare && ctx.symbols.get(&var.name).is_none() {
                            let ty = var
                                .declared_type
                                .as_ref()
                                .map(|ty| crate::infer::map_parsed_type(ty.clone(), ctx))
                                .unwrap_or(surge_ts_types::Type::Unknown);
                            ctx.symbols.insert(
                                var.name.clone(),
                                crate::symbols::SymbolInfo {
                                    kind: if matches!(
                                        var.kind,
                                        surge_ts_syntax::ParsedVariableKind::Const
                                    ) {
                                        crate::symbols::SymbolKind::Const
                                    } else {
                                        crate::symbols::SymbolKind::Let
                                    },
                                    ty,
                                    function_signature: None,
                                },
                            );
                        }
                    }
                    ParsedStatement::ExportDeclaration(export) => {
                        if let surge_ts_syntax::ParsedExportDeclaration::Statement {
                            declaration,
                            ..
                        } = export.as_ref()
                            && let ParsedStatement::VariableDeclaration(var) = declaration.as_ref()
                        {
                            if ctx.symbols.get(&var.name).is_none() {
                                let ty = var
                                    .declared_type
                                    .as_ref()
                                    .map(|ty| crate::infer::map_parsed_type(ty.clone(), ctx))
                                    .unwrap_or(surge_ts_types::Type::Unknown);
                                ctx.symbols.insert(
                                    var.name.clone(),
                                    crate::symbols::SymbolInfo {
                                        kind: if matches!(
                                            var.kind,
                                            surge_ts_syntax::ParsedVariableKind::Const
                                        ) {
                                            crate::symbols::SymbolKind::Const
                                        } else {
                                            crate::symbols::SymbolKind::Let
                                        },
                                        ty,
                                        function_signature: None,
                                    },
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(saved_value_fallback) = saved_value_fallback {
                ctx.module_value_fallback = saved_value_fallback;
            }

            let mut temp_file = parsed_file.clone();
            temp_file.statements = module.statements.clone();
            let current_type_declarations = std::mem::take(&mut ctx.type_declarations);
            let current_symbols = std::mem::take(&mut ctx.symbols);
            // A block's own imports (`import { EventEmitter } from "node:events"`
            // inside `declare module "stream"`) are the values its classes
            // extend; without them a derived class lost every inherited static
            // (`import { EventEmitter } from "stream"` was a false TS2305).
            // Only the ambient modules registered so far answer, and an
            // unresolvable one keeps missing silently as it always has.
            let imported_symbols = if let Some(bindings) = bound_imports {
                bindings.symbols.clone()
            } else if temp_file
                .statements
                .iter()
                .any(|statement| matches!(statement, ParsedStatement::ImportDeclaration(_)))
            {
                let diagnostics_before = ctx.diagnostics().len();
                let bindings = crate::modules::resolve_module_imports(
                    &temp_file,
                    &[],
                    &[],
                    &[],
                    &|_| false,
                    ctx,
                );
                ctx.truncate_diagnostics(diagnostics_before);
                bindings.symbols
            } else {
                SymbolTable::new()
            };
            let mut raw_export_table = build_module_export_table(
                &temp_file,
                &current_type_declarations,
                &current_symbols,
                &imported_symbols,
                Some(current_type_declarations_scope.clone()),
                ctx,
            );
            raw_export_table.shorthand = module.is_shorthand;
            let lowered_type_declarations = current_type_declarations.len() as u64;
            ctx.type_declarations = current_type_declarations;
            ctx.symbols = current_symbols;

            if !parsed_file.is_module {
                register_nested_ambient_module_augmentations(
                    parsed_file,
                    module,
                    &current_type_declarations_scope,
                    &imported_symbols,
                    ctx,
                );
            }

            if parsed_file.is_module {
                // `declare module "x"` inside a module file augments an existing
                // module rather than declaring a new ambient one. It is merged
                // into the resolved target on import, never made resolvable here.
                let key = module_augmentation_key(
                    &parsed_file.file_name,
                    &module.module_specifier,
                    parsed_files,
                    ctx,
                );
                // Go's `mergeModuleAugmentation` merges an export the target only
                // re-exports (`export *`) into the symbol it resolves to, so a
                // relative augmentation reaches the declaring file too; the target
                // file's own declarations already take the file-keyed entry.
                let target_identity = key.strip_prefix(MODULE_AUGMENTATION_FILE_KEY_PREFIX);
                let declaring_file_contributions: Vec<_> =
                    augmentation_contributions_by_declaring_file(
                        &parsed_file.file_name,
                        &module.module_specifier,
                        &raw_export_table,
                        parsed_files,
                        ctx,
                    )
                    .into_iter()
                    .filter(|(file_identity, _)| Some(file_identity.as_str()) != target_identity)
                    .collect();
                match Arc::make_mut(&mut ctx.module_augmentations).get_mut(&key) {
                    Some(existing) => merge_module_augmentation_tables(existing, &raw_export_table),
                    None => {
                        Arc::make_mut(&mut ctx.module_augmentations).insert(key, raw_export_table);
                    }
                }
                for (file_identity, contribution) in declaring_file_contributions {
                    let file_key = module_augmentation_file_key(&file_identity);
                    match Arc::make_mut(&mut ctx.module_augmentations).get_mut(&file_key) {
                        Some(existing) => merge_module_augmentation_tables(existing, &contribution),
                        None => {
                            Arc::make_mut(&mut ctx.module_augmentations)
                                .insert(file_key, contribution);
                        }
                    }
                }
            } else if let Some(existing_index) = ambient_module_indexes
                .get(&module.module_specifier)
                .copied()
            {
                merge_module_export_tables(
                    &mut ambient_module_entries[existing_index].raw_export_table,
                    &raw_export_table,
                );
                if let Some(existing_table) =
                    Arc::make_mut(&mut ctx.ambient_modules).get_mut(&module.module_specifier)
                {
                    merge_module_export_tables(existing_table, &raw_export_table);
                }
            } else {
                Arc::make_mut(&mut ctx.ambient_modules)
                    .insert(module.module_specifier.clone(), raw_export_table.clone());
                ambient_module_indexes.insert(
                    module.module_specifier.clone(),
                    ambient_module_entries.len(),
                );
                ambient_module_entries.push(AmbientModuleEntry {
                    module_specifier: module.module_specifier.clone(),
                    file: temp_file,
                    raw_export_table,
                    block_scope,
                });
            }

            ctx.type_declarations = saved_type_declarations;
            ctx.symbols = saved_symbols;
            let collect_duration = collect_start.elapsed();
            record_program_file_timing(timings, &parsed_file.file_name, |metrics| {
                metrics.collect_type_declarations_passes += 1;
                metrics.lowered_type_declarations += lowered_type_declarations;
                metrics.collect_type_declarations_duration += collect_duration;
            });
        }
        ctx.type_declaration_scope = saved_type_declaration_scope;
    }
    ambient_module_entries
}

fn resolve_ambient_export_tables(ambient_module_entries: &[AmbientModuleEntry], ctx: &mut CheckerContext) {
    let ambient_files = ambient_module_entries
        .iter()
        .map(|entry| entry.file.clone())
        .collect::<Vec<_>>();
    let local_module_export_tables = ambient_module_entries
        .iter()
        .map(|entry| Some(entry.raw_export_table.clone()))
        .collect::<Vec<_>>();

    let mut resolved_module_export_tables = vec![None; ambient_module_entries.len()];
    let mut resolving = vec![false; ambient_module_entries.len()];

    for (file_index, entry) in ambient_module_entries.iter().enumerate() {
        if let Some(resolved_export_table) = crate::modules::resolve_module_export_table(
            file_index,
            &ambient_files,
            &local_module_export_tables,
            &mut resolved_module_export_tables,
            &mut resolving,
            ctx,
        ) {
            Arc::make_mut(&mut ctx.ambient_modules)
                .insert(entry.module_specifier.clone(), resolved_export_table);
        }
    }
}

/// Go merges every augmentation into its module before anything resolves
/// (`mergeModuleAugmentation` runs in `initializeChecker`), so a class an
/// ambient module declares, and every signature naming it, sees the augmented
/// members. A block is lowered and resolved as it is registered, before the
/// augmentations written after it are, so the augmentations of script ambient
/// modules are collected from syntax first: a module file's `declare module
/// "x"`, and a `module "x"` nested in a script's ambient module, which resolves
/// through the enclosing block and that block's imports.
fn syntactic_ambient_augmentation_types(
    parsed_files: &[ParsedProgramFile],
    block_imports: Option<&AmbientBlockImports>,
    ctx: &mut CheckerContext,
) -> HashMap<String, TypeDeclarationTable> {
    let mut augmentations: HashMap<String, TypeDeclarationTable> = HashMap::new();
    let ambient: HashSet<&str> = parsed_files
        .iter()
        .filter(|parsed_file| !parsed_file.is_module)
        .flat_map(|parsed_file| &parsed_file.statements)
        .filter_map(|statement| match statement {
            ParsedStatement::DeclareModuleDeclaration(module) if module.module_specifier != "global" => {
                Some(module.module_specifier.as_str())
            }
            _ => None,
        })
        .collect();
    if ambient.is_empty() {
        return augmentations;
    }
    let saved_file_name = ctx.file_name.clone();
    let saved_type_declarations = std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
    let saved_scope = ctx.type_declaration_scope.take();
    let diagnostics_before = ctx.diagnostics().len();
    let collect = |statements: &[ParsedStatement], ctx: &mut CheckerContext| {
        collect_type_declarations(statements, ctx);
        std::mem::take(&mut ctx.type_declarations)
    };
    let mut add = |specifier: &str, table: TypeDeclarationTable, layers: Vec<Arc<TypeDeclarationTable>>| {
        let scope = Arc::new(TypeDeclarationScope::new(layers));
        let scoped = super::binding::attach_resolution_scope_to_declarations(&table, scope);
        let entry = augmentations.entry(specifier.to_string()).or_default();
        for (name, declaration) in scoped.iter() {
            crate::symbols::merge_augmentation_type_declaration_into_table(entry, name.as_ref(), declaration);
        }
    };
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        for (statement_index, statement) in parsed_file.statements.iter().enumerate() {
            let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
                continue;
            };
            if module.module_specifier == "global" {
                continue;
            }
            if parsed_file.is_module {
                if !ambient.contains(module.module_specifier.as_str()) {
                    continue;
                }
                ctx.set_file_name(parsed_file.file_name.clone());
                let table = collect(&module.statements, ctx);
                let layers = vec![Arc::new(table.clone())];
                add(&module.module_specifier, table, layers);
                continue;
            }
            for nested in &module.statements {
                let ParsedStatement::DeclareModuleDeclaration(nested) = nested else {
                    continue;
                };
                if !ambient.contains(nested.module_specifier.as_str()) {
                    continue;
                }
                ctx.set_file_name(parsed_file.file_name.clone());
                let enclosing = collect(&module.statements, ctx);
                let table = collect(&nested.statements, ctx);
                let mut layers = vec![Arc::new(table.clone()), Arc::new(enclosing)];
                if let Some(bindings) =
                    block_imports.and_then(|block_imports| block_imports.get(&(file_index, statement_index)))
                {
                    layers.extend(bindings.scope_layers());
                }
                add(&nested.module_specifier, table, layers);
            }
        }
    }
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    ctx.type_declarations = saved_type_declarations;
    ctx.type_declaration_scope = saved_scope;
    ctx.set_file_name(saved_file_name);
    augmentations
}

/// `declare module "Map" { module "Observable" { … } }` in a script file
/// augments "Observable": Go's `isModuleAugmentationExternal` treats a
/// string-named module directly inside a top-level ambient module of a
/// non-module file as an augmentation. Its body resolves names through the
/// enclosing block (that block's declarations and imports).
fn register_nested_ambient_module_augmentations(
    parsed_file: &ParsedProgramFile,
    module: &surge_ts_syntax::ParsedDeclareModuleDeclaration,
    enclosing_scope: &Arc<TypeDeclarationScope>,
    enclosing_imports: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    for statement in &module.statements {
        let ParsedStatement::DeclareModuleDeclaration(nested) = statement else {
            continue;
        };
        if nested.module_specifier == "global" {
            continue;
        }
        let saved_type_declarations =
            std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
        let saved_symbols = std::mem::replace(&mut ctx.symbols, SymbolTable::new());
        let saved_scope = ctx.type_declaration_scope.clone();

        collect_type_declarations(&nested.statements, ctx);
        let mut layers = vec![Arc::new(ctx.type_declarations.clone())];
        layers.extend(enclosing_scope.layers().iter().cloned());
        let scope = Arc::new(TypeDeclarationScope::new(layers));
        ctx.type_declaration_scope = Some(scope.clone());
        let mut local_function_signatures = HashMap::new();
        let mut nested_symbols = std::mem::take(&mut ctx.symbols);
        collect_function_signatures_from_statements(
            &nested.statements,
            0,
            &mut nested_symbols,
            &mut local_function_signatures,
            ctx,
        );

        let mut temp_file = parsed_file.clone();
        temp_file.statements = nested.statements.clone();
        let nested_type_declarations = std::mem::take(&mut ctx.type_declarations);
        let table = build_module_export_table(
            &temp_file,
            &nested_type_declarations,
            &nested_symbols,
            enclosing_imports,
            Some(scope),
            ctx,
        );

        ctx.type_declarations = saved_type_declarations;
        ctx.symbols = saved_symbols;
        ctx.type_declaration_scope = saved_scope;

        let augmentations = Arc::make_mut(&mut ctx.module_augmentations);
        match augmentations.get_mut(&nested.module_specifier) {
            Some(existing) => merge_module_augmentation_tables(existing, &table),
            None => {
                augmentations.insert(nested.module_specifier.clone(), table);
            }
        }
    }
}

pub(crate) fn collect_ambient_modules(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) {
    let ambient_binding_start = Instant::now();

    // An import inside `declare module "fs/promises"` is one of the block's
    // locals (Go's binder declares it in the module's symbol table), so
    // `function access(path: PathLike)` resolves `PathLike` through it no
    // matter where the imported block sits in the program. Its target can be
    // any other block, registered later, so the blocks are registered once
    // silently to give every import a table to bind against, then registered
    // for real with each block's bindings in its scope.
    let block_imports = ambient_blocks_have_imports(parsed_files).then(|| {
        let diagnostics_start = ctx.diagnostics().len();
        let saved_ambient_modules = ctx.ambient_modules.clone();
        let saved_module_augmentations = ctx.module_augmentations.clone();
        let saved_resolved_named_types = ctx
            .resolved_named_types
            .lock()
            .map(|memo| memo.clone())
            .unwrap_or_default();
        let preliminary_entries = register_ambient_blocks(parsed_files, None, ctx, timings);
        resolve_ambient_export_tables(&preliminary_entries, ctx);
        let bindings = bind_ambient_block_imports(parsed_files, ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_start);
        ctx.ambient_modules = saved_ambient_modules;
        ctx.module_augmentations = saved_module_augmentations;
        if let Ok(mut memo) = ctx.resolved_named_types.lock() {
            *memo = saved_resolved_named_types;
        }
        bindings
    });

    let ambient_module_entries =
        register_ambient_blocks(parsed_files, block_imports.as_ref(), ctx, timings);
    if ambient_module_entries.is_empty() {
        return;
    }
    resolve_ambient_export_tables(&ambient_module_entries, ctx);

    // A class in one block that extends a class imported from another
    // (`class Stream extends EventEmitter` in `declare module "stream"`) took
    // its statics from the raw table the import resolved to while the blocks
    // were being registered — before `export *` chains (`node:events` →
    // `events`) were resolved, so a base reached that way contributed nothing
    // and `import { EventEmitter } from "stream"` was a false TS2305. With
    // every table resolved, bind the block's imports once more and re-merge.
    for entry in &ambient_module_entries {
        let has_imports = entry
            .file
            .statements
            .iter()
            .any(|statement| matches!(statement, ParsedStatement::ImportDeclaration(_)));
        let has_derived_class = entry.file.statements.iter().any(|statement| {
            matches!(
                crate::modules::peel_exported_statement(statement),
                ParsedStatement::ClassDeclaration(class) if !class.extends.is_empty()
            )
        });
        if !has_imports || !has_derived_class {
            continue;
        }
        ctx.set_file_name(entry.file.file_name.clone());
        let diagnostics_before = ctx.diagnostics().len();
        let bindings =
            crate::modules::resolve_module_imports(&entry.file, &[], &[], &[], &|_| false, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        let export_assigned = entry
            .file
            .statements
            .iter()
            .find_map(|statement| match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    surge_ts_syntax::ParsedExportDeclaration::Equals { exported_name, .. } => {
                        Some(exported_name.clone())
                    }
                    _ => None,
                },
                _ => None,
            });
        let Some(table) = Arc::make_mut(&mut ctx.ambient_modules).get_mut(&entry.module_specifier)
        else {
            continue;
        };
        for statement in &entry.file.statements {
            let ParsedStatement::ClassDeclaration(class) =
                crate::modules::peel_exported_statement(statement)
            else {
                continue;
            };
            let Some(base) = class.extends.first() else {
                continue;
            };
            let base_type = bindings
                .symbols
                .get(&base.name)
                .map(|symbol| symbol.ty.peeled());
            let Some(base_type) = base_type else {
                continue;
            };
            if let Some(merged) =
                crate::modules::statics_merged_into(&class.name, &base_type, &table.symbols)
            {
                let _ = table.symbols.insert(class.name.clone(), merged);
            }
            if export_assigned.as_deref() == Some(class.name.as_str())
                && let Some(assigned) = table.export_assignment_symbol.as_deref()
                && let Some(merged) =
                    crate::modules::statics_merged_into_symbol(assigned, &base_type)
            {
                table.export_assignment_symbol = Some(Arc::new(merged));
                table.namespace_export_object_type = None;
            }
        }
    }

    // Bind each block's own imports (`import { Socket } from "node:net"`
    // inside `declare module "http"`) now that every ambient specifier is
    // registered, and publish a per-file scope of block declarations + import
    // bindings. The layered lookup's per-file fallback consults it when the
    // installed block scope misses, which is the only way a block-internal
    // import can be seen from a declaration body. Import-resolution
    // diagnostics are dropped: these imports were never resolved before, and
    // an unresolvable one must keep missing silently exactly as it always
    // has.
    if !ambient_block_imports_enabled() {
        record_program_timing(timings, |timings| {
            timings.ambient_module_binding += ambient_binding_start.elapsed()
        });
        return;
    }
    let mut ambient_file_layers: surge_ts_types::fx::FxHashMap<
        Arc<str>,
        Vec<Arc<crate::symbols::TypeDeclarationTable>>,
    > = surge_ts_types::fx::FxHashMap::default();
    for entry in &ambient_module_entries {
        let layers = ambient_file_layers
            .entry(Arc::from(entry.file.file_name.as_str()))
            .or_default();
        layers.extend(entry.block_scope.layers().iter().cloned());
        let has_imports = entry
            .file
            .statements
            .iter()
            .any(|statement| matches!(statement, ParsedStatement::ImportDeclaration(_)));
        if !has_imports {
            continue;
        }
        ctx.set_file_name(entry.file.file_name.clone());
        let diagnostics_before = ctx.diagnostics().len();
        let bindings =
            crate::modules::resolve_module_imports(&entry.file, &[], &[], &[], &|_| false, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        layers.extend(bindings.scope_layers());
    }
    ctx.ambient_file_type_scopes = Arc::new(
        ambient_file_layers
            .into_iter()
            .map(|(file_name, layers)| {
                (
                    file_name,
                    Arc::new(crate::symbols::TypeDeclarationScope::new(layers)),
                )
            })
            .collect(),
    );

    record_program_timing(timings, |timings| {
        timings.ambient_module_binding += ambient_binding_start.elapsed()
    });
}

/// Resolves `specifier` written in `from_file` to a program file index, taking
/// the package resolver's answer for a bare specifier and the relative resolver
/// otherwise.
pub(crate) fn module_file_index_for_specifier(
    from_file: &str,
    specifier: &str,
    parsed_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Option<usize> {
    if let Some(resolved) = ctx.options.resolved_module_for(from_file, specifier) {
        let identity = crate::modules::canonical_file_identity(resolved);
        if let Some(index) = ctx.module_file_index_by_identity.get(identity.as_str()) {
            return Some(*index);
        }
    }
    crate::modules::resolve_relative_module(
        from_file,
        specifier,
        parsed_files,
        &ctx.module_file_index_by_identity,
    )
    .map(|resolution| resolution.resolved_file_index)
}

/// The file that *declares* `type_name`, following the entry module's
/// `export *` / `export { … } from` chain.
///
/// `declare module "zod/v4" { interface ZodType { … } }` names the package
/// entry point, but `ZodType` is declared in `classic/schemas.ts` and only
/// re-exported from the entry — and `interface ZodString extends …` resolves
/// its heritage in *that* file's scope. Patching the entry module's export
/// table alone left every subtype blind to the added member, so a member an
/// augmentation contributed existed on the augmented interface and on nothing
/// that extended it. Only top-level declarations are followed: a name reached
/// through a namespace re-export (`declare namespace fx { export { Req } }`)
/// has no single declaring file here and keeps the specifier-keyed path.
fn declaring_file_for_exported_type(
    entry_file_index: usize,
    type_name: &str,
    parsed_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Option<String> {
    const MAX_REEXPORT_HOPS: usize = 64;
    let mut visited = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(entry_file_index);
    while let Some(index) = queue.pop_front() {
        if visited.len() >= MAX_REEXPORT_HOPS || !visited.insert(index) {
            continue;
        }
        let Some(file) = parsed_files.get(index) else {
            continue;
        };
        if export_assignment_namespace(&file.statements).is_some_and(|namespace| {
            namespace_reexported_local(&file.statements, namespace, type_name).is_none()
        }) {
            return Some(crate::modules::canonical_file_identity(&file.file_name));
        }
        let declares = |statement: &ParsedStatement| match statement {
            ParsedStatement::InterfaceDeclaration(declaration) => declaration.name == type_name,
            ParsedStatement::ClassDeclaration(declaration) => declaration.name == type_name,
            _ => false,
        };
        for statement in &file.statements {
            match statement {
                statement if declares(statement) => {
                    return Some(crate::modules::canonical_file_identity(&file.file_name));
                }
                ParsedStatement::ExportDeclaration(export) => match &**export {
                    ParsedExportDeclaration::Statement { declaration, .. }
                        if declares(declaration) =>
                    {
                        return Some(crate::modules::canonical_file_identity(&file.file_name));
                    }
                    ParsedExportDeclaration::All {
                        module_specifier, ..
                    } => {
                        if let Some(next) = module_file_index_for_specifier(
                            &file.file_name,
                            module_specifier,
                            parsed_files,
                            ctx,
                        ) {
                            queue.push_back(next);
                        }
                    }
                    ParsedExportDeclaration::Named {
                        specifiers,
                        module_specifier: Some(module_specifier),
                        ..
                    } if specifiers
                        .iter()
                        .any(|specifier| specifier.exported_name == type_name) =>
                    {
                        if let Some(next) = module_file_index_for_specifier(
                            &file.file_name,
                            module_specifier,
                            parsed_files,
                            ctx,
                        ) {
                            queue.push_back(next);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    None
}

/// Splits an augmentation into the per-file contributions that
/// [`declaring_file_for_exported_type`] can place, so each augmented interface
/// also merges into the declaration table its subtypes resolve against. The
/// specifier-keyed entry stays registered alongside: names with no resolvable
/// declaring file still reach consumers only through it, and the ones that do
/// resolve merge idempotently (see `merge_interface_infos`).
fn augmentation_contributions_by_declaring_file(
    augmenting_file: &str,
    module_specifier: &str,
    augmentation: &ModuleExportTable,
    parsed_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Vec<(String, ModuleExportTable)> {
    if augmentation.type_declarations.len() == 0 {
        return Vec::new();
    }
    let Some(entry_file_index) =
        module_file_index_for_specifier(augmenting_file, module_specifier, parsed_files, ctx)
    else {
        return Vec::new();
    };
    let mut by_file: Vec<(String, ModuleExportTable)> = Vec::new();
    for (name, declaration) in augmentation.type_declarations.iter() {
        let Some(declaring_file) =
            declaring_file_for_exported_type(entry_file_index, name.as_ref(), parsed_files, ctx)
        else {
            continue;
        };
        let entry = match by_file
            .iter_mut()
            .find(|(file, _)| *file == declaring_file)
        {
            Some(entry) => entry,
            None => {
                by_file.push((declaring_file, ModuleExportTable::default()));
                by_file.last_mut().expect("just pushed")
            }
        };
        crate::symbols::merge_type_declaration_into_table(
            Arc::make_mut(&mut entry.1.type_declarations),
            name.as_ref(),
            declaration,
        );
    }
    by_file
}


/// Merge a module augmentation into an already-resolved target export table.
///
/// Augmented interfaces merge their members into the target's existing exports
/// (declaration merging); new exported values and types are added. The target's
/// namespace export shape is preserved, since the augmentation only extends it.
/// Augmentation keys are the specifier a *consumer* would write, so a bare
/// specifier files under itself. A relative one (`declare module
/// "./generated"`, the shape `@typescript-eslint/types` uses to hang `parent`
/// on every AST node) names a file relative to the augmenting file, and no
/// consumer outside that directory writes the same string — it files under the
/// target's canonical identity instead, which is what the import path already
/// has in hand. The prefix keeps the two key spaces apart.
pub(crate) const MODULE_AUGMENTATION_FILE_KEY_PREFIX: &str = "\0augmented-file\0";

pub(crate) fn module_augmentation_file_key(resolved_file_identity: &str) -> String {
    format!("{MODULE_AUGMENTATION_FILE_KEY_PREFIX}{resolved_file_identity}")
}

fn module_augmentation_key(
    augmenting_file: &str,
    module_specifier: &str,
    parsed_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> String {
    // Only importer-scoped *package* resolutions reach `resolved_module_for`, so
    // a relative target is resolved the way an import of it is.
    match crate::modules::resolve_relative_module(
        augmenting_file,
        module_specifier,
        parsed_files,
        &ctx.module_file_index_by_identity,
    ) {
        Some(resolution) => module_augmentation_file_key(&crate::modules::canonical_file_identity(
            &resolution.resolved_file_name,
        )),
        None => module_specifier.to_string(),
    }
}

/// Merges in the augmentation filed under a resolved target file, if any. The
/// specifier-keyed lookup stays alongside it: a bare specifier files under
/// itself.
pub(crate) fn apply_file_keyed_module_augmentation(
    export_table: &mut ModuleExportTable,
    resolved_file_identity: &str,
    ctx: &CheckerContext,
) {
    if let Some(augmentation) = ctx
        .module_augmentations
        .get(&module_augmentation_file_key(resolved_file_identity))
    {
        // The target's own declaration table already carries the merged
        // interfaces (`merge_file_keyed_module_augmentation_into_declarations`
        // runs before its export table is built), so only the augmentation's
        // values and brand-new types are still missing here. Merging the
        // interface bodies a second time would duplicate methods and heritage.
        for (name, declaration) in augmentation.type_declarations.iter() {
            if export_table.type_declarations.get(name.as_ref()).is_none() {
                let _ = Arc::make_mut(&mut export_table.type_declarations)
                    .insert(name.as_ref(), declaration.clone());
            }
        }
        for (name, symbol) in augmentation.symbols.iter_shared() {
            match export_table.symbols.get(name) {
                None => {
                    let _ = export_table
                        .symbols
                        .insert_shared(name.clone(), symbol.clone());
                    // The namespace object was computed before the augmentation's
                    // values joined the table.
                    export_table.namespace_export_object_type = None;
                }
                // A namespace augmenting a class (`namespace Observable { let
                // x }`) adds its values to the class's static side, as the
                // same merge does within one file.
                Some(existing)
                    if matches!(existing.ty, surge_ts_types::Type::Object(_))
                        && matches!(symbol.ty, surge_ts_types::Type::Object(_))
                        && !name.contains('.') =>
                {
                    let merged = crate::symbols::SymbolInfo {
                        ty: crate::modules::merge_namespace_value_objects(&existing.ty, &symbol.ty),
                        kind: existing.kind,
                        function_signature: existing.function_signature.clone(),
                    };
                    let _ = export_table.symbols.insert_shared(name.clone(), Arc::new(merged));
                    export_table.namespace_export_object_type = None;
                }
                Some(_) => {}
            }
        }
        merge_augmentation_values_into_export_assignment(export_table, augmentation);
    }
}

/// Go merges an augmentation into what `export =` resolves to when that is a
/// namespace (`mergeModuleAugmentation`), so the augmentation's values are
/// members of the assigned value — a nested namespace merging member by member.
fn merge_augmentation_values_into_export_assignment(
    export_table: &mut ModuleExportTable,
    augmentation: &ModuleExportTable,
) {
    let Some(assignment) = export_table.export_assignment_symbol.as_ref() else {
        return;
    };
    if !matches!(assignment.ty, surge_ts_types::Type::Object(_)) {
        return;
    }
    let mut members = surge_ts_types::PropertyMap::default();
    for (name, symbol) in augmentation.symbols.iter_shared() {
        if !name.contains('.') {
            members.insert(
                name.as_ref().into(),
                surge_ts_types::ObjectProperty::required(symbol.ty.clone()),
            );
        }
    }
    if members.is_empty() {
        return;
    }
    let merged = crate::modules::merge_namespace_value_objects(
        &assignment.ty,
        &surge_ts_types::Type::Object(crate::metrics::alloc_object_type(members, None)),
    );
    export_table.export_assignment_symbol = Some(Arc::new(crate::symbols::SymbolInfo {
        ty: merged,
        kind: assignment.kind,
        function_signature: assignment.function_signature.clone(),
    }));
    export_table.namespace_export_object_type = None;
}

pub(crate) fn has_file_keyed_module_augmentations(ctx: &CheckerContext) -> bool {
    ctx.module_augmentations
        .keys()
        .any(|key| key.starts_with(MODULE_AUGMENTATION_FILE_KEY_PREFIX))
}

/// Merges a relative `declare module "./target"` augmentation into the target
/// file's own declaration table, where the file's declarations resolve their
/// heritage. Applying it only to the export-table copy each importer receives
/// left `interface Identifier extends BaseNode` — resolved under the target's
/// own scope — blind to a `parent` hung on `BaseNode` by a sibling file, so the
/// member existed on `BaseNode` but not on anything extending it, and which
/// consumer triggered the expansion decided what an importer saw.
pub(crate) fn merge_file_keyed_module_augmentation_into_declarations(
    table: &mut crate::symbols::TypeDeclarationTable,
    file_identity: &str,
    statements: &[ParsedStatement],
    ctx: &CheckerContext,
) {
    if let Some(augmentation) = ctx
        .module_augmentations
        .get(&module_augmentation_file_key(file_identity))
    {
        // Go merges an augmentation into what `export =` resolves to when that
        // is a namespace (`mergeModuleAugmentation`), so its types are the
        // namespace's members.
        let namespace = export_assignment_namespace(statements);
        for (name, declaration) in augmentation.type_declarations.iter() {
            // A member the namespace re-exports (`export type { Req }`) is an
            // alias: Go merges into what it resolves to, not into a new
            // namespace member that would shadow it.
            let name = match namespace {
                Some(namespace) => match namespace_reexported_local(statements, namespace, name) {
                    Some(local) => local.to_string(),
                    None => format!("{namespace}.{name}"),
                },
                None => name.to_string(),
            };
            crate::symbols::merge_augmentation_type_declaration_into_table(
                table,
                &name,
                declaration,
            );
        }
    }
}

/// The namespace a module's `export = X` names, when the module declares one.
fn export_assignment_namespace(statements: &[ParsedStatement]) -> Option<&str> {
    let exported = statements.iter().find_map(|statement| match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Equals { exported_name, .. } => Some(exported_name.as_str()),
            _ => None,
        },
        _ => None,
    })?;
    statements
        .iter()
        .any(|statement| {
            matches!(
                crate::modules::peel_exported_statement(statement),
                ParsedStatement::NamespaceDeclaration(namespace) if namespace.name == exported
            )
        })
        .then_some(exported)
}

/// The local name a top-level `namespace` re-exports as `name` through an
/// export specifier (`export { Local as name }`).
fn namespace_reexported_local<'a>(
    statements: &'a [ParsedStatement],
    namespace: &str,
    name: &str,
) -> Option<&'a str> {
    statements.iter().find_map(|statement| {
        let ParsedStatement::NamespaceDeclaration(declaration) =
            crate::modules::peel_exported_statement(statement)
        else {
            return None;
        };
        if declaration.name != namespace {
            return None;
        }
        declaration.statements.iter().find_map(|member| match member {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Named {
                    specifiers,
                    module_specifier: None,
                    ..
                } => specifiers
                    .iter()
                    .find(|specifier| specifier.exported_name == name)
                    .map(|specifier| specifier.local_name.as_str()),
                _ => None,
            },
            _ => None,
        })
    })
}

pub(crate) fn apply_module_augmentation(
    base: &mut ModuleExportTable,
    augmentation: &ModuleExportTable,
) {
    for (name, declaration) in augmentation.type_declarations.iter() {
        crate::symbols::merge_augmentation_type_declaration_into_table(
            Arc::make_mut(&mut base.type_declarations),
            name.as_ref(),
            declaration,
        );
    }

    for (name, symbol) in augmentation.symbols.iter_shared() {
        if base.symbols.get(name).is_none() {
            let _ = base.symbols.insert_shared(name.clone(), symbol.clone());
        }
    }
}

/// [`merge_module_export_tables`] for two augmentations of one module: each
/// interface fragment keeps the scope of the block that wrote it, as it does
/// once merged into the target (`merge_augmentation_type_declaration_into_table`).
fn merge_module_augmentation_tables(target: &mut ModuleExportTable, source: &ModuleExportTable) {
    for (name, declaration) in source.type_declarations.iter() {
        crate::symbols::merge_augmentation_type_declaration_into_table(
            Arc::make_mut(&mut target.type_declarations),
            name.as_ref(),
            declaration,
        );
    }
    merge_module_export_tables(target, source);
}

pub(crate) fn merge_module_export_tables(
    target: &mut ModuleExportTable,
    source: &ModuleExportTable,
) {
    record_type_declaration_table_merge(
        None,
        source.type_declarations.len(),
        TableMergeKind::General,
    );
    for (name, declaration) in source.type_declarations.iter() {
        crate::symbols::merge_type_declaration_into_table(
            Arc::make_mut(&mut target.type_declarations),
            name.as_ref(),
            declaration,
        );
    }

    for (name, symbol) in source.symbols.iter_shared() {
        if target.symbols.get(name).is_none() {
            let _ = target.symbols.insert_shared(name.clone(), symbol.clone());
        }
    }

    if target.default_symbol.is_none() {
        target.default_symbol = source.default_symbol.clone();
    }

    target.namespace_export_object_type = None;
    target.writes_export_assignment |= source.writes_export_assignment;
    target.export_assignment_names_module |= source.export_assignment_names_module;
    target.has_unresolved_star_export |= source.has_unresolved_star_export;
    target.has_incomplete_declaration_surface |= source.has_incomplete_declaration_surface;
}
