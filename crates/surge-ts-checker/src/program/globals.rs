//! Script-global type, function-signature, and value-symbol collection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use surge_ts_syntax::{ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedStatement};
use surge_ts_types::FunctionType;

use super::*;

use crate::checks::{expr, function as check_function, var};
use crate::context::{CheckerContext, FileKind};
use crate::driver::collect_type_declarations;
use crate::symbols::SymbolTable;

pub(crate) fn collect_global_type_declarations(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) {
    for parsed_file in parsed_files {
        if parsed_file.file_kind == FileKind::GeneratedDeclaration {
            continue;
        }

        if parsed_file.file_kind == FileKind::DependencyDeclaration || parsed_file.is_module {
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());
        let collect_start = Instant::now();
        ctx.merge_script_interfaces_with_globals = true;
        collect_type_declarations(&parsed_file.statements, ctx);
        ctx.merge_script_interfaces_with_globals = false;
        let lowered_type_declarations = ctx.type_declarations.len() as u64;
        let collect_duration = collect_start.elapsed();
        record_program_file_timing(timings, &parsed_file.file_name, |timings| {
            timings.collect_type_declarations_passes += 1;
            timings.lowered_type_declarations += lowered_type_declarations;
            timings.collect_type_declarations_duration += collect_duration
        });
    }
}

pub(crate) fn collect_global_function_signatures(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &mut SymbolTable,
    function_signatures: &mut HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    let seeded_names = seed_script_values_for_signatures(parsed_files, global_symbols, ctx);
    // A class's member annotations resolve their `typeof` through the
    // context, not the signature scope, so the seed reaches them this way.
    // Behind it sit the lib's globals, which a parameter initializer reads like
    // any other value (`function f(m = Math)`).
    let saved_fallback = ctx.module_value_fallback.take();
    let mut seeded_values = SymbolTable::new();
    for (name, symbol) in &seeded_names {
        let _ = seeded_values.insert_shared(name.clone(), symbol.clone());
    }
    ctx.module_value_fallback = Some(Arc::new(
        seeded_values.with_parent_fallback(Arc::new(ctx.ambient_global_symbols.clone())),
    ));
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !is_script_source(parsed_file) {
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());

        collect_function_signatures_from_statements(
            &parsed_file.statements,
            file_index,
            global_symbols,
            function_signatures,
            ctx,
        );
    }
    ctx.module_value_fallback = saved_fallback;
    // Signature collection declares a script's functions and classes; any other
    // seeded name is still only the seed, even once `apply_expando_members` has
    // replaced it with the members written on it, and the check phase declares
    // it itself — left behind, a `const` read as its own redeclaration (TS2451).
    let declared = signature_declared_names(parsed_files);
    for (name, seeded) in seeded_names {
        let is_seed = global_symbols
            .get_own(&name)
            .is_some_and(|symbol| std::ptr::eq(symbol, seeded.as_ref()));
        if is_seed || !declared.contains(name.as_ref()) {
            global_symbols.remove(&name);
        }
    }
    merge_cross_file_script_namespaces(parsed_files, global_symbols, ctx);
    merge_script_functions_into_ambient_namespaces(parsed_files, global_symbols, ctx);
}

/// A script's function and a declaration file's `declare namespace` of the
/// same name are one global symbol in Go: callable, with the namespace's
/// exports. The ambient table answers the name for every file, so the merged
/// value replaces the namespace object there.
fn merge_script_functions_into_ambient_namespaces(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    for parsed_file in parsed_files.iter().filter(|parsed_file| is_script_source(parsed_file)) {
        for statement in &parsed_file.statements {
            let Some(function) = declared_function(statement) else {
                continue;
            };
            let Some(surge_ts_types::Type::Function(call_signature)) =
                global_symbols.get_own(&function.name).map(|symbol| symbol.ty.clone())
            else {
                continue;
            };
            let Some(namespace) = ctx.ambient_global_symbols.get(&function.name) else {
                continue;
            };
            let surge_ts_types::Type::Object(namespace_object) = &namespace.ty else {
                continue;
            };
            if namespace_object.call_signature.is_some() || namespace_object.construct_signature.is_some() {
                continue;
            }
            let merged = crate::symbols::SymbolInfo {
                ty: surge_ts_types::Type::Object(
                    crate::metrics::alloc_object_type(namespace_object.properties.as_ref().clone(), None)
                        .with_call_signature(call_signature),
                ),
                kind: crate::symbols::SymbolKind::Function,
                function_signature: None,
            };
            let _ = global_symbols.insert(function.name.clone(), merged.clone());
            ctx.ambient_global_symbols.insert(function.name.clone(), merged);
        }
    }
}

/// tsc merges every script's globals into one table (`mergeSymbolTable`), so a
/// function or class and a namespace of one name written in two scripts are
/// one symbol (TS2433 aside), whose members are the namespace's and the
/// function's expandos or the class's statics. A namespace member is a declaration of its own: an expando write
/// of the same name only assigns it (`SetValueDeclaration` prefers the
/// declaration that is not an assignment), so the member's type replaces what
/// the write declared.
fn merge_cross_file_script_namespaces(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    let mut declaring_files: HashMap<&str, usize> = HashMap::new();
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !is_script_source(parsed_file) {
            continue;
        }
        for statement in &parsed_file.statements {
            if let Some(function) = declared_function(statement) {
                declaring_files.entry(function.name.as_str()).or_insert(file_index);
            }
            if let ParsedStatement::ClassDeclaration(class) = statement {
                declaring_files.entry(class.name.as_str()).or_insert(file_index);
            }
        }
    }
    if declaring_files.is_empty() {
        return;
    }
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !is_script_source(parsed_file) {
            continue;
        }
        let mut names: Vec<&str> = Vec::new();
        for statement in &parsed_file.statements {
            if let ParsedStatement::NamespaceDeclaration(namespace) = statement
                && declaring_files
                    .get(namespace.name.as_str())
                    .is_some_and(|declaring| *declaring != file_index)
                && !names.contains(&namespace.name.as_str())
            {
                names.push(namespace.name.as_str());
            }
        }
        if names.is_empty() {
            continue;
        }
        ctx.set_file_name(parsed_file.file_name.clone());
        for name in names {
            let Some(members) =
                crate::modules::exports::source_namespace_value_members(&parsed_file.statements, name, ctx)
            else {
                continue;
            };
            if members.is_empty() {
                continue;
            }
            let Some(symbol) = global_symbols.get_own_shared(name) else {
                continue;
            };
            let (call_signature, mut properties) = match &symbol.ty {
                surge_ts_types::Type::Function(function) => {
                    (function.clone(), surge_ts_types::PropertyMap::default())
                }
                surge_ts_types::Type::Object(object) => match object.call_signature() {
                    Some(call_signature) => (call_signature.clone(), (*object.properties).clone()),
                    None if object.construct_signature().is_some() => {
                        let merged = crate::modules::exports::object_with_namespace_members(object, &members);
                        let _ = global_symbols.insert(
                            name.to_string(),
                            crate::symbols::SymbolInfo {
                                ty: surge_ts_types::Type::Object(merged),
                                kind: symbol.kind,
                                function_signature: symbol.function_signature.clone(),
                            },
                        );
                        continue;
                    }
                    None => continue,
                },
                _ => continue,
            };
            for (member_name, member) in members.iter() {
                properties.insert(member_name.clone(), member.clone());
            }
            let object = crate::metrics::alloc_object_type(properties, None).with_call_signature(call_signature);
            let _ = global_symbols.insert(
                name.to_string(),
                crate::symbols::SymbolInfo {
                    ty: surge_ts_types::Type::Object(object),
                    kind: symbol.kind,
                    function_signature: symbol.function_signature.clone(),
                },
            );
        }
    }
}

/// The names [`collect_function_signature_from_statement`] declares across the
/// program's scripts.
fn signature_declared_names(parsed_files: &[ParsedProgramFile]) -> std::collections::HashSet<&str> {
    fn collect<'a>(statement: &'a ParsedStatement, names: &mut std::collections::HashSet<&'a str>) {
        match statement {
            ParsedStatement::FunctionDeclaration(function) => {
                names.insert(function.name.as_str());
            }
            ParsedStatement::ClassDeclaration(class) => {
                names.insert(class.name.as_str());
            }
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => collect(declaration, names),
                ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Class(class),
                    ..
                } => {
                    names.insert(class.name.as_str());
                }
                ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Function(function),
                    ..
                } => {
                    names.insert(function.name.as_str());
                }
                _ => {}
            },
            _ => {}
        }
    }
    let mut names = std::collections::HashSet::new();
    for parsed_file in parsed_files.iter().filter(|parsed_file| is_script_source(parsed_file)) {
        for statement in &parsed_file.statements {
            collect(statement, &mut names);
        }
    }
    names
}

/// Each script file's own top-level values, for the other files to see: the
/// binder declares every script's `var`/`let`/`const`/namespace in the one
/// global table, so `let greeting` in `a.ts` is in scope in `b.ts` and in every
/// module. `typeof globalThis` reads them too (`script_global_object_members`),
/// so even a lone script's are collected.
pub(crate) fn collect_script_values(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Vec<Option<Arc<SymbolTable>>> {
    let mut values = vec![None; parsed_files.len()];
    if !parsed_files.iter().any(is_script_source) {
        return values;
    }
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !is_script_source(parsed_file) {
            continue;
        }
        ctx.set_file_name(parsed_file.file_name.clone());
        let collected = crate::modules::collect_exportable_value_symbols(
            &parsed_file.statements,
            &ctx.type_declarations,
            global_symbols,
            None,
            false,
            ctx,
        );
        let mut own = SymbolTable::new();
        for (name, symbol) in collected.iter_shared() {
            let inherited = global_symbols
                .get_own(name)
                .is_some_and(|global| std::ptr::eq(global, symbol.as_ref()));
            if !inherited {
                let _ = own.insert_shared(name.clone(), symbol.clone());
            }
        }
        values[file_index] = Some(Arc::new(own));
    }
    merge_script_namespace_values(parsed_files, &mut values);
    merge_script_namespaces_into_ambient_namespaces(parsed_files, &mut values, ctx);
    values
}

/// A script's `namespace A` and a declaration file's `declare namespace A` are
/// one global namespace in Go, its members merged at every depth; the ambient
/// table answers the name for every file, so the merged value replaces both.
fn merge_script_namespaces_into_ambient_namespaces(
    parsed_files: &[ParsedProgramFile],
    values: &mut [Option<Arc<SymbolTable>>],
    ctx: &mut CheckerContext,
) {
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        if !is_script_source(parsed_file) {
            continue;
        }
        for statement in &parsed_file.statements {
            let ParsedStatement::NamespaceDeclaration(namespace) = statement else {
                continue;
            };
            let Some(script) = values[file_index]
                .as_ref()
                .and_then(|table| table.get_own_shared(&namespace.name))
            else {
                continue;
            };
            let Some(ambient) = ctx.ambient_global_symbols.get_shared(&namespace.name) else {
                continue;
            };
            let (surge_ts_types::Type::Object(ambient_object), surge_ts_types::Type::Object(_)) =
                (&ambient.ty, &script.ty)
            else {
                continue;
            };
            if ambient_object.call_signature.is_some() || ambient_object.construct_signature.is_some() {
                continue;
            }
            let merged = crate::symbols::SymbolInfo {
                ty: crate::modules::merge_namespace_value_objects(&ambient.ty, &script.ty),
                kind: script.kind,
                function_signature: None,
            };
            ctx.ambient_global_symbols.insert(namespace.name.clone(), merged.clone());
            if let Some(table) = values[file_index].as_mut() {
                let _ = Arc::make_mut(table).insert(namespace.name.clone(), merged);
            }
        }
    }
}

/// `namespace A` reopened in several scripts is one merged namespace, so every
/// file sees the members each block exports, not the first block's alone.
fn merge_script_namespace_values(
    parsed_files: &[ParsedProgramFile],
    values: &mut [Option<Arc<SymbolTable>>],
) {
    let namespace_names = |parsed_file: &ParsedProgramFile| -> Vec<String> {
        parsed_file
            .statements
            .iter()
            .filter_map(|statement| match statement {
                ParsedStatement::NamespaceDeclaration(namespace) => Some(namespace.name.clone()),
                _ => None,
            })
            .collect()
    };
    let mut merged: HashMap<String, surge_ts_types::Type> = HashMap::new();
    let mut declaring_files: HashMap<String, usize> = HashMap::new();
    for (file_index, parsed_file) in parsed_files.iter().enumerate() {
        let Some(table) = values[file_index].as_ref() else {
            continue;
        };
        for name in namespace_names(parsed_file) {
            let Some(symbol) = table.get_own(&name) else {
                continue;
            };
            *declaring_files.entry(name.clone()).or_default() += 1;
            let combined = match merged.remove(&name) {
                Some(existing) => merge_namespace_value(&existing, &symbol.ty),
                None => symbol.ty.clone(),
            };
            merged.insert(name, combined);
        }
    }
    for (name, ty) in merged {
        if declaring_files.get(&name).copied().unwrap_or(0) < 2 {
            continue;
        }
        for table in values.iter_mut().flatten() {
            let Some(symbol) = table.get_own(&name) else {
                continue;
            };
            let replacement = crate::symbols::SymbolInfo {
                ty: ty.clone(),
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            };
            let _ = Arc::make_mut(table).insert(name.clone(), replacement);
        }
    }
}

fn merge_namespace_value(
    left: &surge_ts_types::Type,
    right: &surge_ts_types::Type,
) -> surge_ts_types::Type {
    use surge_ts_types::Type;
    // A function merges with a namespace of the same name into one callable
    // value carrying the namespace's members.
    let (left_object, right_object) = match (left, right) {
        (Type::Object(left_object), Type::Object(right_object)) => (left_object, right_object),
        (Type::Function(function), Type::Object(object)) | (Type::Object(object), Type::Function(function))
            if object.call_signature.is_none() =>
        {
            return Type::Object(
                crate::metrics::alloc_object_type(object.properties.as_ref().clone(), None)
                    .with_call_signature(function.clone()),
            );
        }
        _ => return left.clone(),
    };
    let call_signature = left_object.call_signature.clone().or_else(|| right_object.call_signature.clone());
    let construct_signature =
        left_object.construct_signature.clone().or_else(|| right_object.construct_signature.clone());
    let mut properties = left_object.properties.as_ref().clone();
    for (name, property) in right_object.properties.iter() {
        match properties.get(name).cloned() {
            Some(existing) => {
                let mut combined = existing;
                combined.ty = merge_namespace_value(&combined.ty, &property.ty);
                properties.insert(name.clone(), combined);
            }
            None => {
                properties.insert(name.clone(), property.clone());
            }
        }
    }
    let mut merged = crate::metrics::alloc_object_type(properties, None);
    merged.call_signature = call_signature;
    merged.construct_signature = construct_signature;
    Type::Object(merged)
}

/// The scripts' values as a module sees them. tsc merges every script's locals
/// into the one global table (`initializeChecker`), which a module's name
/// lookup reaches once its own scope misses. A lib global of the same name is
/// declared first, so the ambient table keeps answering for it, as it does in
/// a script.
pub(crate) fn module_script_globals(
    global_symbols: &SymbolTable,
    script_values: &[Option<Arc<SymbolTable>>],
    ambient_global_symbols: &SymbolTable,
) -> Option<Arc<SymbolTable>> {
    let mut globals = SymbolTable::new();
    let tables = std::iter::once(global_symbols).chain(script_values.iter().flatten().map(Arc::as_ref));
    for table in tables {
        for (name, symbol) in table.iter_handles() {
            if ambient_global_symbols.get(name).is_none() && globals.get_own(name).is_none() {
                let _ = globals.insert_handle(
                    name.clone(),
                    crate::symbols::clone_symbol_info_handle(symbol),
                );
            }
        }
    }
    let empty = globals.iter_handles().next().is_none();
    (!empty).then(|| Arc::new(globals))
}

fn declared_global_names<'a>(
    statements: &'a [ParsedStatement],
    names: &mut std::collections::HashSet<&'a str>,
) {
    for statement in statements {
        match crate::modules::peel_exported_statement(statement) {
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
            _ => {}
        }
    }
}

/// The names declared globally, by syntax alone: a script's top level and a
/// global augmentation (`declare global { … }`, or `global { … }` inside
/// `declare module "x"`), as `(script top level, augmentations)`.
fn global_declaration_names(
    parsed_files: &[ParsedProgramFile],
) -> (std::collections::HashSet<&str>, std::collections::HashSet<&str>) {
    let mut script_top_level = std::collections::HashSet::new();
    let mut augmented = std::collections::HashSet::new();
    for parsed_file in parsed_files {
        for statement in &parsed_file.statements {
            match statement {
                ParsedStatement::DeclareModuleDeclaration(module) if module.module_specifier == "global" => {
                    declared_global_names(&module.statements, &mut augmented);
                }
                ParsedStatement::DeclareModuleDeclaration(module) => {
                    for nested in &module.statements {
                        if let ParsedStatement::DeclareModuleDeclaration(inner) = nested
                            && inner.module_specifier == "global"
                        {
                            declared_global_names(&inner.statements, &mut augmented);
                        }
                    }
                }
                _ if !parsed_file.is_module => {
                    declared_global_names(std::slice::from_ref(statement), &mut script_top_level);
                }
                _ => {}
            }
        }
    }
    (script_top_level, augmented)
}

/// Every name some global declaration binds (see [`global_declaration_names`]).
pub(crate) fn globally_declared_names(parsed_files: &[ParsedProgramFile]) -> std::collections::HashSet<&str> {
    let (mut names, augmented) = global_declaration_names(parsed_files);
    names.extend(augmented);
    names
}

/// The names only a global augmentation declares — a `declare global { … }`,
/// or a `global { … }` block inside `declare module "x"`. tsc merges the
/// augmentations into the globals after every script file's own declarations
/// (`initializeChecker`), so a name a script file also declares at its top
/// level keeps that declaration first: the one whose container
/// `checkExportSpecifier` asks about.
pub(crate) fn global_augmentation_only_names(
    parsed_files: &[ParsedProgramFile],
) -> surge_ts_types::fx::FxHashSet<Arc<str>> {
    let (script_top_level, augmented) = global_declaration_names(parsed_files);
    augmented
        .into_iter()
        .filter(|name| !script_top_level.contains(name))
        .map(Arc::from)
        .collect()
}

fn is_script_source(parsed_file: &ParsedProgramFile) -> bool {
    parsed_file.file_kind != FileKind::GeneratedDeclaration
        && !parsed_file.is_module
        && !parsed_file.file_kind.is_declaration()
}

/// A script's top-level `var`s are globals the binder declares before any
/// signature is resolved, so `function f(x: typeof a)` and `function f(x = a)`
/// read `a` wherever it is written. Signature collection runs before the check
/// phase declares them; seed their values for it, as module analysis does for
/// a module's locals. The seed is removed afterwards so the check phase
/// declares each variable itself.
fn seed_script_values_for_signatures(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) -> Vec<(Arc<str>, crate::symbols::SymbolInfoHandle)> {
    let script_sources = || parsed_files.iter().filter(|parsed_file| is_script_source(parsed_file));
    let mut seeded = Vec::new();
    for parsed_file in script_sources() {
        ctx.set_file_name(parsed_file.file_name.clone());
        let values = crate::modules::collect_exportable_value_symbols(
            &parsed_file.statements,
            &ctx.type_declarations,
            &SymbolTable::new(),
            None,
            false,
            ctx,
        );
        for (name, symbol) in values.iter_shared() {
            if global_symbols.get(name).is_none() {
                let _ = global_symbols.insert_shared(name.clone(), symbol.clone());
                seeded.push((name.clone(), symbol.clone()));
            }
        }
    }
    seeded
}

pub(crate) fn collect_function_signatures_from_statements(
    statements: &[ParsedStatement],
    file_index: usize,
    symbols: &mut SymbolTable,
    function_signatures: &mut HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    let mut declaration_counts = HashMap::<String, usize>::new();
    for statement in statements {
        count_function_declarations(statement, &mut declaration_counts);
    }
    let outer_collecting_signatures = std::mem::replace(&mut ctx.collecting_signatures, true);
    let outer_fallback =
        hoist_function_declarations(statements, file_index, symbols, &declaration_counts, ctx);
    for (statement_index, statement) in statements.iter().enumerate() {
        collect_function_signature_from_statement(
            statement,
            file_index,
            statement_index,
            symbols,
            function_signatures,
            ctx,
            &declaration_counts,
        );
    }
    if let Some(outer_fallback) = outer_fallback {
        ctx.module_value_fallback = outer_fallback;
    }
    // Expando members are hoisted with the function they are written on, so a
    // function declared earlier in the file can already read them. A merged
    // namespace's members come first: they are what a write to the same name
    // assigns rather than declares.
    crate::modules::exports::apply_namespace_members_to_declarations(statements, symbols, ctx);
    crate::modules::exports::apply_expando_members(statements, symbols, ctx);
    ctx.collecting_signatures = outer_collecting_signatures;
}

/// A script's signatures are collected with the program's globals, before any
/// module is bound, so an import type written in one (`import("./m").T`, or
/// JSDoc's `{import("./m").T}`) found no module and resolved to nothing. tsc
/// reads a parameter's type when it is first needed, with every module bound
/// (`getTypeFromImportTypeNode`, checker.go:24483), so a script that names a
/// module collects its function signatures again for its own check, as a module
/// does. They register in a table of their own: `symbols` already declares the
/// script's functions, and a second implementation there reads as a duplicate.
pub(crate) fn recollect_script_function_signatures(
    statements: &[ParsedStatement],
    file_index: usize,
    symbols: Arc<SymbolTable>,
    ctx: &mut CheckerContext,
) -> HashMap<FunctionDeclarationLocation, FunctionType> {
    let mut declaration_counts = HashMap::<String, usize>::new();
    for statement in statements {
        count_function_declarations(statement, &mut declaration_counts);
    }
    let mut scope = SymbolTable::with_parent(symbols);
    let mut function_signatures = HashMap::new();
    let outer_collecting_signatures = std::mem::replace(&mut ctx.collecting_signatures, true);
    for (statement_index, statement) in statements.iter().enumerate() {
        if declared_function(statement).is_none() {
            continue;
        }
        collect_function_signature_from_statement(
            statement,
            file_index,
            statement_index,
            &mut scope,
            &mut function_signatures,
            ctx,
            &declaration_counts,
        );
    }
    ctx.collecting_signatures = outer_collecting_signatures;
    function_signatures
}

fn declared_function(statement: &ParsedStatement) -> Option<&surge_ts_syntax::ParsedFunctionDeclaration> {
    match statement {
        ParsedStatement::FunctionDeclaration(function) => Some(function),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declared_function(declaration),
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Function(function),
                ..
            } => Some(function),
            _ => None,
        },
        _ => None,
    }
}

/// tsc's binder declares every function of a scope before any signature is
/// resolved, so a signature may read its own function or one declared after it
/// (`function f(n: typeof f)`, `function f(o = defaults())`). Signatures are
/// collected in source order, so when one reads ahead like that, a first pass
/// over the declarations — each of them standing in as the degradation
/// sentinel meanwhile — gives every function the signature the second,
/// reporting pass reads it by until its own turn comes. Returns the fallback to
/// restore once the pass is over.
fn hoist_function_declarations(
    statements: &[ParsedStatement],
    file_index: usize,
    symbols: &SymbolTable,
    declaration_counts: &HashMap<String, usize>,
    ctx: &mut CheckerContext,
) -> Option<Option<Arc<SymbolTable>>> {
    let functions: Vec<&surge_ts_syntax::ParsedFunctionDeclaration> =
        statements.iter().filter_map(declared_function).collect();
    let local_types: Vec<(&str, Vec<&surge_ts_syntax::ParsedType>)> = statements
        .iter()
        .filter_map(|statement| match crate::modules::peel_exported_statement(statement) {
            ParsedStatement::TypeAliasDeclaration(alias) => Some((alias.name.as_str(), vec![&alias.ty])),
            ParsedStatement::InterfaceDeclaration(interface) => Some((
                interface.name.as_str(),
                check_function::interface_written_types(interface),
            )),
            _ => None,
        })
        .collect();
    if !check_function::signatures_read_ahead(&functions, &local_types) {
        return None;
    }
    let outer_fallback = ctx.module_value_fallback.clone();
    let layered = |table: SymbolTable| {
        Arc::new(match &outer_fallback {
            Some(outer) => table.with_parent_fallback(outer.clone()),
            None => table,
        })
    };
    let mut sentinels = SymbolTable::new();
    for function in &functions {
        let _ = sentinels.insert(
            function.name.clone(),
            crate::symbols::SymbolInfo {
                ty: surge_ts_types::Type::Unknown,
                kind: crate::symbols::SymbolKind::Function,
                function_signature: None,
            },
        );
    }
    ctx.module_value_fallback = Some(layered(sentinels));
    let mut first_pass = symbols.clone();
    let mut discarded = HashMap::new();
    let diagnostics_before = ctx.diagnostics().len();
    let function_statements: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter(|(_, statement)| declared_function(statement).is_some())
        .map(|(statement_index, _)| statement_index)
        .collect();
    for position in check_function::signature_collection_order(&functions, &local_types) {
        let statement_index = function_statements[position];
        collect_function_signature_from_statement(
            &statements[statement_index],
            file_index,
            statement_index,
            &mut first_pass,
            &mut discarded,
            ctx,
            declaration_counts,
        );
    }
    ctx.truncate_diagnostics(diagnostics_before);
    let mut hoisted = SymbolTable::new();
    for function in &functions {
        if let Some(symbol) = first_pass.get_handle(&function.name) {
            let _ = hoisted.insert_handle(function.name.clone(), symbol);
        }
    }
    ctx.module_value_fallback = Some(layered(hoisted));
    Some(outer_fallback)
}

fn count_function_declarations(
    statement: &ParsedStatement,
    declaration_counts: &mut HashMap<String, usize>,
) {
    match statement {
        ParsedStatement::FunctionDeclaration(function) => {
            *declaration_counts.entry(function.name.clone()).or_default() += 1;
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                count_function_declarations(declaration, declaration_counts)
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Function(function),
                ..
            } => {
                *declaration_counts.entry(function.name.clone()).or_default() += 1;
            }
            _ => {}
        },
        _ => {}
    }
}

pub(crate) fn collect_function_signature_from_statement(
    statement: &ParsedStatement,
    file_index: usize,
    statement_index: usize,
    symbols: &mut SymbolTable,
    function_signatures: &mut HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
    declaration_counts: &HashMap<String, usize>,
) {
    match statement {
        ParsedStatement::FunctionDeclaration(function) => {
            let function_type = check_function::collect_function_declaration_signature(
                function,
                symbols,
                ctx,
                declaration_counts.get(&function.name) == Some(&1),
            );
            function_signatures.insert(
                FunctionDeclarationLocation {
                    file_index,
                    statement_index,
                },
                function_type,
            );
        }
        ParsedStatement::ClassDeclaration(class) => {
            let symbol = super::build_class_value_symbol_with_scope(class, Some(symbols), ctx);
            symbols.insert(class.name.clone(), symbol);
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Class(class),
                ..
            } => {
                let symbol = super::build_class_value_symbol_with_scope(class, Some(symbols), ctx);
                symbols.insert(class.name.clone(), symbol);
            }
            ParsedExportDeclaration::Statement { declaration, .. } => {
                collect_function_signature_from_statement(
                    declaration.as_ref(),
                    file_index,
                    statement_index,
                    symbols,
                    function_signatures,
                    ctx,
                    declaration_counts,
                )
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Function(function),
                ..
            } => {
                let function_type = check_function::collect_function_declaration_signature(
                    function,
                    symbols,
                    ctx,
                    declaration_counts.get(&function.name) == Some(&1),
                );
                function_signatures.insert(
                    FunctionDeclarationLocation {
                        file_index,
                        statement_index,
                    },
                    function_type,
                );
            }
            ParsedExportDeclaration::Default { .. } => {}
            _ => {}
        },
        _ => {}
    }
}

pub(crate) fn collect_global_variables(
    parsed_files: &[ParsedProgramFile],
    global_symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    for parsed_file in parsed_files {
        if parsed_file.file_kind == FileKind::GeneratedDeclaration {
            continue;
        }

        if parsed_file.file_kind == FileKind::DependencyDeclaration || parsed_file.is_module {
            continue;
        }

        ctx.set_file_name(parsed_file.file_name.clone());
        for statement in &parsed_file.statements {
            let var = match statement {
                ParsedStatement::VariableDeclaration(var) => Some(var),
                ParsedStatement::ExportDeclaration(export) => {
                    if let surge_ts_syntax::ParsedExportDeclaration::Statement {
                        declaration, ..
                    } = export.as_ref()
                    {
                        if let ParsedStatement::VariableDeclaration(var) = declaration.as_ref() {
                            Some(var)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(var) = var {
                if var.is_declare || parsed_file.file_kind.is_declaration() {
                    // `let assert: typeof import("vitest")["assert"]` reads a
                    // module namespace that only exists once imports are bound,
                    // after this pass; the annotation maps on first read instead.
                    let ty = match var.declared_type.as_ref() {
                        Some(annotation)
                            if crate::modules::exports::annotation_contains_import_type_query(
                                annotation,
                            ) =>
                        {
                            {
                            ctx.register_import_type_global(&var.name);
                            crate::infer::make_lazy_value_annotation_reference(
                                ctx,
                                &var.name,
                                var.name_span.map_or(0, |span| span.start),
                                annotation.clone(),
                            )
                        }
                        }
                        // `declare let b: { v: typeof b }` queries itself, which
                        // tsc resolves on demand; mapped now, the name is unbound.
                        Some(annotation)
                            if super::queries_pending_value(
                                var,
                                &std::iter::once(var.name.as_str()).collect(),
                            ) =>
                        {
                            crate::infer::make_lazy_value_annotation_reference(
                                ctx,
                                &var.name,
                                var.name_span.map_or(0, |span| span.start),
                                annotation.clone(),
                            )
                        }
                        Some(annotation) => crate::infer::map_parsed_type(annotation.clone(), ctx),
                        None => surge_ts_types::Type::Unknown,
                    };
                    if !parsed_file.file_kind.is_declaration()
                        || global_symbols.get(&var.name).is_none()
                    {
                        global_symbols.insert(
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
        }
    }
}

#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) fn collect_local_value_symbols(
    statements: &[ParsedStatement],
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    for statement in statements {
        collect_local_value_symbols_from_statement(statement, symbols, ctx);
    }
}

#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) fn collect_local_value_symbols_from_statement(
    statement: &ParsedStatement,
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::VariableDeclaration(var) => {
            if var.is_declare {
                return;
            }

            let symbol_kind = if matches!(var.kind, surge_ts_syntax::ParsedVariableKind::Const) {
                crate::symbols::SymbolKind::Const
            } else {
                crate::symbols::SymbolKind::Let
            };

            let ty = if let Some(declared_type) = var.declared_type.as_ref() {
                crate::infer::map_parsed_type(declared_type.clone(), ctx)
            } else if let Some(initializer) = var.initializer.as_ref() {
                let inferred =
                    expr::evaluate_expression(initializer, var.initializer_span, symbols, ctx);

                match inferred {
                    crate::infer::InferredExpression::Known(inferred_ty)
                        if !matches!(
                            inferred_ty,
                            surge_ts_types::Type::Unknown
                                | surge_ts_types::Type::TypeParameter(_)
                        ) =>
                    {
                        var::widen_variable_declaration_initializer_type(
                            symbol_kind,
                            initializer,
                            &inferred_ty,
                            var::is_auto_array_candidate(var, ctx),
                            ctx.options.no_implicit_any,
                        )
                    }
                    _ => surge_ts_types::Type::Unknown,
                }
            } else {
                surge_ts_types::Type::Unknown
            };

            symbols.insert(
                var.name.clone(),
                crate::symbols::SymbolInfo {
                    ty,
                    kind: symbol_kind,
                    function_signature: None,
                },
            );
        }
        ParsedStatement::ExportDeclaration(export) => {
            if let surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } =
                export.as_ref()
            {
                collect_local_value_symbols_from_statement(declaration.as_ref(), symbols, ctx)
            }
        }
        _ => {}
    }
}

/// A script's top-level declarations share the global scope with the ambient
/// globals of every other file, the default library's included: `class
/// Symbol {}` collides with lib's `var Symbol` (TS2300), `let name` with
/// lib.dom's `const name` (TS2451), a class with a global function's overloads
/// (TS2813). A plain JavaScript file's side is not reported, the other's
/// still is (`reportMergeSymbolError`). A name the script declares more than
/// once is left to the file's own merge checks.
pub(crate) fn report_script_global_conflicts(statements: &[ParsedStatement], plain_js: bool, ctx: &mut CheckerContext) {
    use crate::symbols::SymbolKind;
    use surge_ts_diagnostics::Diagnostic;
    use surge_ts_syntax::ParsedVariableKind;
    enum Declared {
        Class,
        Function,
        BlockScoped,
        Var,
        Namespace,
    }
    let declarations: Vec<(&str, Option<surge_ts_syntax::TextSpan>, Declared)> = statements
        .iter()
        .filter_map(|statement| match statement {
            // An ambient declaration is itself registered as a global.
            ParsedStatement::ClassDeclaration(class) if !class.is_declare => {
                Some((class.name.as_str(), class.name_span, Declared::Class))
            }
            ParsedStatement::FunctionDeclaration(function) if function.has_body => {
                Some((function.name.as_str(), function.name_span, Declared::Function))
            }
            ParsedStatement::VariableDeclaration(variable)
                if !variable.is_declare && !variable.is_enum_object && !variable.from_binding_pattern =>
            {
                let declared = match variable.kind {
                    ParsedVariableKind::Var => Declared::Var,
                    _ => Declared::BlockScoped,
                };
                Some((variable.name.as_str(), variable.name_span, declared))
            }
            ParsedStatement::NamespaceDeclaration(namespace)
                if !namespace.is_declare && !namespace.name.contains('.') && super::is_instantiated_namespace(namespace) =>
            {
                Some((namespace.name.as_str(), namespace.name_span, Declared::Namespace))
            }
            _ => None,
        })
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for statement in statements {
        let name = match statement {
            ParsedStatement::ClassDeclaration(class) => class.name.as_str(),
            ParsedStatement::FunctionDeclaration(function) => function.name.as_str(),
            ParsedStatement::VariableDeclaration(variable) => variable.name.as_str(),
            ParsedStatement::NamespaceDeclaration(namespace) => namespace.name.as_str(),
            ParsedStatement::InterfaceDeclaration(_) | ParsedStatement::TypeAliasDeclaration(_) => continue,
            _ => continue,
        };
        *counts.entry(name).or_default() += 1;
    }
    for (name, span, declared) in declarations {
        if counts.get(name).copied().unwrap_or(0) > 1 {
            continue;
        }
        // A namespace merges with a class, function or namespace.
        if ctx.namespace_meaning(name).is_some() {
            continue;
        }
        // `globalThis` is no lib declaration; redeclaring it is TS2397.
        if name == "globalThis" {
            continue;
        }
        let Some(global) = ctx.ambient_global_symbols.get(name) else {
            continue;
        };
        let file_name = ctx.file_name.clone();
        let diagnostic = match (declared, global.kind) {
            (Declared::Class, SymbolKind::Function) => Diagnostic::ts2813(name, file_name),
            (Declared::Class | Declared::Namespace, SymbolKind::Var | SymbolKind::Let | SymbolKind::Const) => {
                Diagnostic::ts2300(name, file_name)
            }
            // An ambient `declare var` is registered as `Let`.
            (Declared::Function, SymbolKind::Const) => Diagnostic::ts2451(name, file_name),
            (Declared::Function, SymbolKind::Var | SymbolKind::Let) => Diagnostic::ts2300(name, file_name),
            (Declared::BlockScoped, SymbolKind::Var | SymbolKind::Let | SymbolKind::Const | SymbolKind::Function) => {
                Diagnostic::ts2451(name, file_name)
            }
            (Declared::Var, SymbolKind::Function) => Diagnostic::ts2300(name, file_name),
            (Declared::Var, SymbolKind::Const) => Diagnostic::ts2451(name, file_name),
            _ => continue,
        };
        // tsc reports a conflicting merge at every declaration of both
        // symbols (`addDuplicateDeclarationErrorsForSymbols`), the lib's too.
        for (lib_file, lib_span, _) in super::program_lib_declaration_sites(name) {
            let mut duplicate = diagnostic.clone();
            duplicate.file_name = lib_file.to_string();
            ctx.push(crate::spans::diagnostic_with_syntax_span(duplicate, Some(lib_span)));
        }
        if !plain_js {
            ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
        }
    }
    // A JavaScript `var` is typed by its JSDoc or its initializer, which this
    // syntactic comparison does not read.
    if !surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        report_script_var_redeclarations_of_globals(statements, ctx);
    }
}

/// A script's types, and a module's `declare global` ones, merge with the
/// globals the default library declares. An interface cannot merge with a
/// type alias, nor a type alias with any type, and tsc reports the conflict at
/// every declaration of both (`reportMergeSymbolError`).
pub(crate) fn report_lib_global_type_conflicts(statements: &[ParsedStatement], is_module: bool, ctx: &mut CheckerContext) {
    use super::LibDeclarationKind as Kind;
    use surge_ts_diagnostics::Diagnostic;
    fn report(name: &str, span: Option<surge_ts_syntax::TextSpan>, conflicts: fn(Kind) -> bool, ctx: &mut CheckerContext) {
        let sites = super::program_lib_declaration_sites(name);
        if !sites.iter().any(|(_, _, kind)| conflicts(*kind)) {
            return;
        }
        let diagnostic = Diagnostic::ts2300(name, ctx.file_name.clone());
        for (lib_file, lib_span, _) in sites {
            let mut duplicate = diagnostic.clone();
            duplicate.file_name = lib_file.to_string();
            ctx.push(crate::spans::diagnostic_with_syntax_span(duplicate, Some(lib_span)));
        }
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
    }
    fn visit(statement: &ParsedStatement, ctx: &mut CheckerContext) {
        match statement {
            ParsedStatement::TypeAliasDeclaration(alias) if alias.enum_name.is_none() => report(
                &alias.name,
                alias.name_span,
                |kind| matches!(kind, Kind::Class | Kind::Interface | Kind::TypeAlias),
                ctx,
            ),
            ParsedStatement::InterfaceDeclaration(interface) => {
                report(&interface.name, interface.name_span, |kind| kind == Kind::TypeAlias, ctx);
                report_lib_duplicate_index_signatures(interface, ctx);
            }
            ParsedStatement::ExportDeclaration(export) => {
                if let surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                    visit(declaration, ctx);
                }
            }
            _ => {}
        }
    }
    for statement in statements {
        match statement {
            ParsedStatement::DeclareModuleDeclaration(module) if is_module && module.module_specifier == "global" => {
                for statement in &module.statements {
                    visit(statement, ctx);
                }
            }
            _ if !is_module => visit(statement, ctx),
            _ => {}
        }
    }
}

/// `checkTypeForDuplicateIndexSignatures` over a lib interface an augmentation
/// reopens: an index signature whose key the lib's declarations already index
/// by is a duplicate at each of them.
fn report_lib_duplicate_index_signatures(interface: &surge_ts_syntax::ParsedInterfaceDeclaration, ctx: &mut CheckerContext) {
    for (number, span) in [(false, interface.string_index_span), (true, interface.number_index_span)] {
        let Some(span) = span else {
            continue;
        };
        let lib_sites: Vec<_> = super::program_lib_index_sites(&interface.name)
            .into_iter()
            .filter(|(_, lib_number, _)| *lib_number == number)
            .collect();
        if lib_sites.is_empty() {
            continue;
        }
        let diagnostic = surge_ts_diagnostics::Diagnostic::ts2374(if number { "number" } else { "string" }, ctx.file_name.clone());
        for (lib_file, _, lib_span) in lib_sites {
            let mut duplicate = diagnostic.clone();
            duplicate.file_name = lib_file.to_string();
            ctx.push(crate::spans::diagnostic_with_syntax_span(duplicate, Some(lib_span)));
        }
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, Some(span)));
    }
}

/// [`CheckerContext::script_function_implementations`]: the global functions
/// scripts implement, JavaScript ones included.
pub(crate) fn script_function_implementations(
    parsed_files: &[ParsedProgramFile],
) -> Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Vec<Arc<str>>>> {
    let mut implementations: surge_ts_types::fx::FxHashMap<Arc<str>, Vec<Arc<str>>> = Default::default();
    for file in parsed_files.iter().filter(|file| !file.is_module && !file.file_kind.is_declaration()) {
        let file_name: Arc<str> = Arc::from(file.file_name.as_str());
        for statement in &file.statements {
            if let ParsedStatement::FunctionDeclaration(function) = statement
                && function.has_body
            {
                implementations.entry(Arc::from(function.name.as_str())).or_default().push(file_name.clone());
            }
        }
    }
    implementations.retain(|_, files| {
        files.dedup();
        files.len() > 1
    });
    Arc::new(implementations)
}

/// tsc's `checkFunctionOrConstructorSymbol`: a global function implemented in
/// more than one script is TS2393 at each implementation; a file's own
/// duplicates are its body check's.
pub(crate) fn report_duplicate_script_function_implementations(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    if ctx.script_function_implementations.is_empty() {
        return;
    }
    let implementations = ctx.script_function_implementations.clone();
    for statement in statements {
        let ParsedStatement::FunctionDeclaration(function) = statement else {
            continue;
        };
        if !function.has_body
            || !implementations
                .get(function.name.as_str())
                .is_some_and(|files| files.iter().any(|file| **file != *ctx.file_name))
        {
            continue;
        }
        let diagnostic = surge_ts_diagnostics::Diagnostic::ts2393(ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, function.name_span));
    }
}

/// TS2403 for a script's `var` (or `declare var`) that redeclares an ambient
/// global `var` with another type: the global's first declaration — the
/// default library's, which is registered first — gives the variable its
/// type. Only an annotated declaration, or a bare one (`any`), is compared.
/// TS2403 for a `declare var` redeclaring a top-level `var` of the file. An
/// ambient declaration is registered before any statement is checked, so the
/// statement check sees it as its own first declaration; the types are read
/// off the declarations here instead. Only types read without inference are
/// compared: an annotation, `any` for neither annotation nor initializer, or
/// a literal initializer's widened type.
pub(crate) fn report_ambient_var_redeclarations(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    use surge_ts_syntax::ParsedExpression;
    let vars = || {
        statements.iter().filter_map(|statement| match statement {
            ParsedStatement::VariableDeclaration(variable)
                if variable.kind == surge_ts_syntax::ParsedVariableKind::Var
                    && !variable.from_binding_pattern
                    && !variable.is_enum_object =>
            {
                Some(variable.as_ref())
            }
            _ => None,
        })
    };
    // Only names a later `declare var` redeclares are read at all.
    let mut seen = std::collections::HashSet::new();
    let redeclared: std::collections::HashSet<&str> = vars()
        .filter(|variable| !seen.insert(variable.name.as_str()) && variable.is_declare)
        .map(|variable| variable.name.as_str())
        .collect();
    if redeclared.is_empty() {
        return;
    }
    let mut first: std::collections::HashMap<&str, Option<surge_ts_types::Type>> = std::collections::HashMap::new();
    for variable in vars() {
        if !redeclared.contains(variable.name.as_str()) {
            continue;
        }
        let declared = match (&variable.declared_type, &variable.initializer) {
            (Some(annotation), _) => {
                let checkpoint = ctx.diagnostics().len();
                let declared = crate::infer::map_parsed_type(annotation.clone(), ctx);
                ctx.truncate_diagnostics(checkpoint);
                (!declared.is_degraded() && !declared.is_unknown()).then_some(declared)
            }
            (None, None) => Some(surge_ts_types::Type::Any),
            (None, Some(ParsedExpression::NumberLiteral(_))) => Some(surge_ts_types::Type::Number),
            (None, Some(ParsedExpression::StringLiteral(_))) => Some(surge_ts_types::Type::String),
            (None, Some(ParsedExpression::BooleanLiteral(_))) => Some(surge_ts_types::Type::Boolean),
            (None, Some(_)) => None,
        };
        let Some(previous) = first.get(variable.name.as_str()) else {
            first.insert(&variable.name, declared);
            continue;
        };
        if !variable.is_declare {
            continue;
        }
        let (Some(previous), Some(declared)) = (previous, declared) else {
            continue;
        };
        if surge_ts_types::is_type_identical_to(previous, &declared) {
            continue;
        }
        let diagnostic =
            surge_ts_diagnostics::Diagnostic::ts2403(&variable.name, previous.name(), declared.name(), ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, variable.name_span));
    }
}

fn report_script_var_redeclarations_of_globals(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    use crate::symbols::SymbolKind;
    for statement in statements {
        let ParsedStatement::VariableDeclaration(variable) = statement else {
            continue;
        };
        if variable.kind != surge_ts_syntax::ParsedVariableKind::Var
            || variable.from_binding_pattern
            || variable.is_enum_object
            || (variable.declared_type.is_none() && variable.initializer.is_some())
        {
            continue;
        }
        // An ambient `declare var` is registered as `Let`.
        let Some(global) = ctx.ambient_global_symbols.get(&variable.name) else {
            continue;
        };
        if !matches!(global.kind, SymbolKind::Let) || global.ty.is_degraded() || global.ty.is_unknown() {
            continue;
        }
        let previous = global.ty.clone();
        let checkpoint = ctx.diagnostics().len();
        let declared = match &variable.declared_type {
            Some(annotation) => crate::infer::map_parsed_type(annotation.clone(), ctx),
            None => surge_ts_types::Type::Any,
        };
        ctx.truncate_diagnostics(checkpoint);
        if declared.is_degraded() || declared.is_unknown() || surge_ts_types::is_type_identical_to(&previous, &declared) {
            continue;
        }
        let diagnostic =
            surge_ts_diagnostics::Diagnostic::ts2403(&variable.name, previous.name(), declared.name(), ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, variable.name_span));
    }
}
