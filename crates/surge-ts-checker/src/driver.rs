use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedInterfaceDeclaration,
    ParsedNamespaceDeclaration, ParsedStatement, ParsedType, ParsedTypeAliasDeclaration,
    parse_source,
};

use crate::checks::{call, expr, function as check_function, var};
use crate::context::{CheckerContext, DeclarationNamespace, DeclarationResolutionKey, FileKind};
use crate::default_lib::load_generated_default_lib_inputs;
use crate::infer::{report_duplicate_type_parameters, validate_local_type_declaration};
use crate::paths::canonicalize_if_exists_string;
use crate::program::collect_function_signatures_from_statements;
use crate::symbols::{InterfaceInfo, TypeAliasInfo, TypeDeclarationInfo};

pub fn check_source(source_text: &str, file_name: &str) -> Vec<Diagnostic> {
    check_source_with_options(
        source_text,
        file_name,
        crate::context::CheckerOptions::default(),
    )
}

pub fn check_source_with_options(
    source_text: &str,
    file_name: &str,
    options: crate::context::CheckerOptions,
) -> Vec<Diagnostic> {
    let _lock = crate::semantic::engine_lock();
    crate::semantic::claim_thread_caches(crate::semantic::next_program_id());
    let exact = options.exact_optional_property_types;
    surge_ts_types::with_strict_null_checks(options.strict_null_checks, || {
        surge_ts_types::with_exact_optional_property_types(exact, || {
            check_single_source(source_text, file_name, options)
        })
    })
}

fn check_single_source(
    source_text: &str,
    file_name: &str,
    options: crate::context::CheckerOptions,
) -> Vec<Diagnostic> {
    let parsed = parse_source(source_text, file_name);
    let comment_directives = parsed.comment_directives.clone();
    let has_parse_errors = !parsed.parser_errors.is_empty();
    let file_name = parsed.file_name;
    let mut file_kinds = surge_ts_types::fx::FxHashMap::default();
    file_kinds.insert(file_name.clone(), classify_file_kind(&file_name));
    let mut ctx = CheckerContext::new(file_name.clone(), options, file_kinds);
    ctx.declaration_environment_store.mark_program_lifetime();

    let global_this_querying_values = inject_generated_default_libs(&mut ctx);

    let mut merged_td = ctx.ambient_global_type_declarations.as_ref().clone();
    for (k, v) in ctx.type_declarations.iter() {
        let _ = merged_td.insert(k.clone(), v.clone());
    }
    ctx.type_declarations = merged_td;

    let mut merged_sym = ctx.ambient_global_symbols.clone();
    for (k, v) in ctx.symbols.iter() {
        let _ = merged_sym.insert(k.clone(), v.clone());
    }
    ctx.set_symbols(merged_sym);

    let parser_diagnostics: Vec<_> = crate::program::unclaimed_parser_errors(
        &parsed.parser_errors,
        &parsed.grammar_diagnostics,
        &ctx,
    )
    .map(|error| crate::program::diagnostics::parser_error_diagnostic(error, &file_name))
    .collect();
    for diagnostic in parser_diagnostics {
        ctx.push(diagnostic);
    }
    crate::program::emit_grammar_diagnostics(&parsed.grammar_diagnostics, &mut ctx);
    ctx.parenthesized_expressions = parsed.parenthesized_expressions.into();
    if !parsed.is_module && !crate::program::file_is_forced_module(&file_name, parsed.jsx_factory_uses.first_tag.is_some(), &ctx.options) {
        ctx.global_this_starts = parsed.global_this_starts.into();
    } else {
        ctx.module_this_starts = parsed.global_this_starts.into();
    }
    ctx.literal_this_members = parsed.literal_this_members.into();
    ctx.let_assignments = parsed.let_assignments.into();
    ctx.jsx_factory_uses = parsed.jsx_factory_uses.clone();
    if !classify_file_kind(&file_name).is_declaration() {
        let properties =
            crate::program::file_constructor_local_properties(&parsed.statements, &ctx.options);
        if !properties.is_empty() {
            let mut by_file = surge_ts_types::fx::FxHashMap::default();
            by_file.insert(std::sync::Arc::from(file_name.as_str()), properties);
            ctx.constructor_local_properties = std::sync::Arc::new(by_file);
        }
    }

    ctx.merge_script_interfaces_with_globals = !parsed.is_module;
    collect_type_declarations(&parsed.statements, &mut ctx);
    ctx.merge_script_interfaces_with_globals = false;
    crate::program::emit_deferred_grammar_diagnostics(&mut ctx);
    collect_global_augmentations_from_statements(&parsed.statements, &mut ctx);
    // Ambient value lowering (`declare var Document: {…}`) already resolved some
    // of these names against the lib-only declarations. The file's own
    // declarations have since merged into them, so the memoized resolutions are
    // stale — program mode gets this from `begin_file_check`, which the
    // single-file driver does not run.
    ctx.replace_resolved_named_types(0);
    sync_global_this_symbol(&mut ctx);
    if !global_this_querying_values.is_empty() {
        crate::program::relower_global_this_querying_values(&global_this_querying_values, &mut ctx);
        sync_global_this_symbol(&mut ctx);
    }
    let mut merged_sym = ctx.ambient_global_symbols.clone();
    for (k, v) in ctx.symbols.iter() {
        let _ = merged_sym.insert(k.clone(), v.clone());
    }
    ctx.set_symbols(merged_sym);

    seed_class_value_symbols(&parsed.statements, &mut ctx);

    let current_type_declarations = ctx.type_declarations.clone();
    let current_symbols = ctx.symbols.clone();
    let validation_symbols = crate::modules::collect_exportable_value_symbols(
        &parsed.statements,
        &current_type_declarations,
        &current_symbols,
        None,
        parsed.is_module,
        &mut ctx,
    );
    let saved_symbols = std::mem::replace(&mut ctx.symbols, validation_symbols);

    validate_local_type_declarations(&parsed.statements, &file_name, &mut ctx);
    validate_direct_utility_aliases(&parsed.statements, &mut ctx);
    let validation_symbols = std::mem::replace(&mut ctx.symbols, saved_symbols);

    ctx.module_value_fallback = Some(std::sync::Arc::new(validation_symbols));

    crate::flow::begin_never_initialized_file(
        &parsed.statements,
        parsed.is_module,
        &parsed.definite_writes,
        &mut ctx,
    );
    for (index, statement) in parsed.statements.iter().enumerate() {
        let statement =
            crate::program::expand_module_if_alias(statement.clone(), &parsed.statements[..index]);
        check_statement(statement, &mut ctx);
    }
    crate::flow::check_module_definite_assignment(&parsed.statements, &mut ctx);
    ctx.module_value_fallback = None;

    let mut diagnostics = ctx.finish();
    // Program mode applies these in `check_files`; the single-file driver has
    // no such stage.
    crate::program::apply_comment_directives(
        &mut diagnostics,
        &comment_directives,
        has_parse_errors,
        &file_name,
    );
    diagnostics
}

/// Program mode seeds a class's value symbol in its signature pre-pass
/// (`collect_function_signature_from_statement`). The single-file driver has no
/// such pass, so `class A {}` left `A` resolvable as a type only and every
/// value-position reference to it reported TS2693.
fn seed_class_value_symbols(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    // Building a class's value symbol resolves its heritage, and at seeding time
    // the file's own value declarations are not in scope yet — a class extending
    // a `declare const` factory would report the factory as an unresolved name.
    // The check pass walks the same class afterwards with the full scope, so
    // anything raised here is discarded rather than reported twice or early.
    let diagnostics_before = ctx.diagnostics.len();

    let mut symbols = ctx.symbols.clone();
    for statement in statements {
        seed_class_value_symbol(statement, &mut symbols, ctx);
    }
    ctx.set_symbols(symbols);

    ctx.diagnostics.truncate(diagnostics_before);
}

fn seed_class_value_symbol(
    statement: &ParsedStatement,
    symbols: &mut crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    let class = match statement {
        ParsedStatement::ClassDeclaration(class) => class.as_ref(),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                seed_class_value_symbol(declaration.as_ref(), symbols, ctx);
                return;
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Class(class),
                ..
            } => class,
            _ => return,
        },
        _ => return,
    };

    if class.is_declare || symbols.get_own(&class.name).is_some() {
        return;
    }

    let symbol = crate::program::build_class_value_symbol_with_scope(class, Some(symbols), ctx);
    symbols.insert(class.name.clone(), symbol);
}

fn inject_generated_default_libs(
    ctx: &mut CheckerContext,
) -> Vec<crate::program::GlobalThisQueryingValue> {
    let default_lib_inputs = load_generated_default_lib_inputs(ctx.options.no_lib, None);
    if default_lib_inputs.is_empty() {
        return Vec::new();
    }

    let original_file_name = ctx.file_name.clone();
    let mut parser = surge_ts_syntax::ParserWorker::new();
    let parsed_files: Vec<crate::program::ParsedProgramFile> = default_lib_inputs
        .into_iter()
        .map(|input| {
            let parsed = parser.parse(&input.source_text, &input.file_name);
            crate::program::ParsedProgramFile {
                file_name: parsed.file_name,
                has_export_default: input.source_text.contains("export default"),
                contains_typeof: input.source_text.contains("typeof"),
                statements: parsed.statements,
                parser_errors: parsed.parser_errors,
                bind_errors: Vec::new(),
                tsc_bound: false,
                oxc_aborted: false,
                tsc_globals: None,
                no_check: false,
                is_module: parsed.is_module,
                commonjs_module: parsed.commonjs_module,
                import_call_specifiers: parsed.import_call_specifiers,
                import_calls: Vec::new(),
                import_attributes: Vec::new(),
                file_kind: FileKind::GeneratedDeclaration,
                module_reads: parsed.module_reads,
                definite_writes: parsed.definite_writes,
                comment_directives: parsed.comment_directives,
                grammar_diagnostics: Vec::new(),
                parenthesized_expressions: Default::default(),
                global_this_starts: Default::default(),
                literal_this_members: Default::default(),
                let_assignments: Default::default(),
                json_module_type: None,
                jsx_factory_uses: parsed.jsx_factory_uses,
                emit_helper_requests: Vec::new(),
            }
        })
        .collect();

    crate::program::collect_ambient_global_types(&parsed_files, ctx, None);
    let global_this_querying_values = crate::program::lower_ambient_global_values(&parsed_files, ctx);
    ctx.set_file_name(original_file_name);
    global_this_querying_values
}

pub(crate) fn collect_type_declarations(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    for (index, statement) in statements.iter().enumerate() {
        collect_type_declarations_from_statement(statement, &statements[..index], ctx);
    }
}

/// Merge every `declare global { }` / `declare module "X" { global { } }`
/// augmentation *type* declaration across all files into the ambient global
/// table. A global interface split across files (e.g. `@types/node`'s
/// `BufferConstructor`, which gains `from` in `buffer.buffer.d.ts` and is named by
/// `var Buffer` in `buffer.d.ts`) must be fully assembled here before any value is
/// lowered against it during binding.
pub(crate) fn collect_global_augmentations(
    parsed_files: &[crate::program::ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    let mut block_tables = Vec::new();
    for parsed_file in parsed_files {
        ctx.set_file_name(parsed_file.file_name.clone());
        for_each_global_augmentation_block(
            &parsed_file.statements,
            ctx,
            |block_statements, enclosing_statements, ctx| {
                let block_table = collect_global_augmentation_block_types(
                    block_statements,
                    enclosing_statements,
                    &parsed_file.statements,
                    parsed_files,
                    ctx,
                );
                let block_table = if enclosing_statements.is_none() {
                    crate::symbols::attach_global_declaration_scope(&block_table)
                } else {
                    block_table
                };
                // Its interfaces merge into the global declarations but resolve
                // where they are written: under the module's scope, or the
                // ambient module block's with the block's own imports (`import {
                // URL as _URL } from "url"` inside `declare module "url"`). Both
                // exist only once modules are bound.
                block_tables.push(if parsed_file.is_module || enclosing_statements.is_some() {
                    crate::symbols::mark_module_global_fragments(&block_table)
                } else {
                    block_table
                });
            },
        );
    }
    crate::symbols::merge_shared_tables_into(
        std::sync::Arc::make_mut(&mut ctx.ambient_global_type_declarations),
        &block_tables,
    );
}

/// Lower `declare global` augmentation *values* against the caller's current type
/// environment. Called during binding (where the owning module's import scope is
/// active, so a `declare global { var x: ImportedType }` resolves) after
/// [`collect_global_augmentations`] has merged every augmentation type.
pub(crate) fn lower_global_augmentation_values_from_statements(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    for_each_global_augmentation_block(statements, ctx, |block_statements, enclosing, ctx| {
        lower_global_augmentation_values(block_statements, enclosing.unwrap_or(statements), ctx)
    });
}

/// Single-file driver path: no cross-file split and no module import scope, so the
/// two phases run back-to-back over the one file's statements.
pub(crate) fn collect_global_augmentations_from_statements(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    let mut block_tables = Vec::new();
    for_each_global_augmentation_block(
        statements,
        ctx,
        |block_statements, enclosing_statements, ctx| {
            block_tables.push(collect_global_augmentation_block_types(
                block_statements,
                enclosing_statements,
                statements,
                &[],
                ctx,
            ));
        },
    );
    crate::symbols::merge_shared_tables_into(
        std::sync::Arc::make_mut(&mut ctx.ambient_global_type_declarations),
        &block_tables,
    );
    for_each_global_augmentation_block(statements, ctx, |block_statements, enclosing, ctx| {
        lower_global_augmentation_values(block_statements, enclosing.unwrap_or(statements), ctx)
    });
}

/// Whether any statement carries a `declare global` block (directly or nested
/// in a `declare module "x"`). Modules for which this is true publish global
/// values first-wins in the final analysis pass, which is order-sensitive and
/// must stay on the serial coordinator path.
pub(crate) fn has_global_augmentation_block(statements: &[ParsedStatement]) -> bool {
    statements.iter().any(|statement| {
        let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
            return false;
        };
        module.module_specifier == "global"
            || module.statements.iter().any(|nested| {
                matches!(
                    nested,
                    ParsedStatement::DeclareModuleDeclaration(inner)
                        if inner.module_specifier == "global"
                )
            })
    })
}

fn for_each_global_augmentation_block(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
    mut visit: impl FnMut(&[ParsedStatement], Option<&[ParsedStatement]>, &mut CheckerContext),
) {
    for statement in statements {
        let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
            continue;
        };

        if module.module_specifier == "global" {
            visit(&module.statements, None, ctx);
            continue;
        }

        // `declare module "X" { global { ... } }` also augments the global scope
        // (e.g. `@types/node` declares `var Buffer` inside the "buffer" module).
        // The enclosing block's statements ride along so the augmentation's
        // declarations can see the block's own type aliases and interfaces
        // (`@types/node/events.d.ts` writes `NodeJS.EventEmitter` members
        // against block-local helpers like `Key`/`Listener1`).
        for nested in &module.statements {
            if let ParsedStatement::DeclareModuleDeclaration(inner) = nested {
                if inner.module_specifier == "global" {
                    visit(&inner.statements, Some(&module.statements), ctx);
                }
            }
        }
    }
}

/// Collect one `declare global` block's type declarations into a fresh table, so
/// the result can later be merged into the ambient table by sharing payload
/// handles. The caller merges every block's table together in one pass
/// (see [`collect_global_augmentations`]). `file_statements` are those of the
/// file the block is written in.
fn collect_global_augmentation_block_types(
    block_statements: &[ParsedStatement],
    enclosing_statements: Option<&[ParsedStatement]>,
    file_statements: &[ParsedStatement],
    parsed_files: &[crate::program::ParsedProgramFile],
    ctx: &mut CheckerContext,
) -> crate::symbols::TypeDeclarationTable {
    let saved_type_declarations = std::mem::replace(
        &mut ctx.type_declarations,
        crate::symbols::TypeDeclarationTable::new(),
    );
    let saved_type_declaration_scope = ctx.type_declaration_scope.clone();
    ctx.type_declaration_scope = None;

    collect_type_declarations(block_statements, ctx);
    let mut block_table = std::mem::take(&mut ctx.type_declarations);

    // A `global { … }` block nested in `declare module "X"` lexically sees the
    // enclosing block's type declarations, but this collection runs before
    // `collect_ambient_modules` builds that block's scope. Collect the
    // enclosing declarations here and attach them as the augmentation
    // declarations' resolution scope, so their bodies resolve block-local
    // helpers instead of degrading at every use site.
    if let Some(enclosing) = enclosing_statements {
        collect_type_declarations(enclosing, ctx);
        let enclosing_table = std::mem::take(&mut ctx.type_declarations);
        if enclosing_table.len() > 0 {
            let enclosing_scope =
                std::sync::Arc::new(crate::symbols::TypeDeclarationScope::new(vec![
                    std::sync::Arc::new(enclosing_table),
                ]));
            block_table = crate::program::binding::attach_resolution_scope_to_declarations(
                &block_table,
                enclosing_scope,
            );
        }
    }
    publish_global_augmentation_alias_types(
        block_statements,
        enclosing_statements.unwrap_or(file_statements),
        parsed_files,
        &mut block_table,
        ctx,
    );

    ctx.type_declarations = saved_type_declarations;
    ctx.type_declaration_scope = saved_type_declaration_scope;

    block_table
}

/// `export import X = N.M` in a `declare global` block merges `X` into the
/// globals as an alias, and an alias carries every meaning of its entity: the
/// entity's type and its namespace members are global types under `X`, as its
/// value is a global value ([`lower_global_augmentation_values`]). The entity
/// resolves from the block outwards the way the value does: the block, then
/// the file or ambient module it is written in, then the globals.
fn publish_global_augmentation_alias_types(
    block_statements: &[ParsedStatement],
    enclosing_statements: &[ParsedStatement],
    parsed_files: &[crate::program::ParsedProgramFile],
    block_table: &mut crate::symbols::TypeDeclarationTable,
    ctx: &mut CheckerContext,
) {
    use crate::symbols::{TypeDeclarationScope, TypeDeclarationTable};

    let aliases: Vec<(&str, &str)> = block_statements
        .iter()
        .filter_map(|statement| match crate::modules::peel_exported_statement(statement) {
            ParsedStatement::ImportDeclaration(import) => match &import.kind {
                surge_ts_syntax::ParsedImportKind::EntityAlias {
                    local_name, target, ..
                } => Some((local_name.as_str(), target.as_str())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    if aliases.is_empty() {
        return;
    }
    let names_entity = |table: &TypeDeclarationTable, target: &str| {
        let prefix = format!("{target}.");
        table.get(target).is_some()
            || table.iter().any(|(key, _)| key.starts_with(prefix.as_str()))
    };
    let mut enclosing: Option<(Arc<TypeDeclarationTable>, Arc<TypeDeclarationScope>)> = None;
    for (local, target) in aliases {
        let entries = if names_entity(block_table, target) {
            entity_alias_type_entries(block_table, target, local, None)
        } else {
            let (enclosing_table, enclosing_scope) = enclosing.get_or_insert_with(|| {
                let saved = std::mem::replace(&mut ctx.type_declarations, TypeDeclarationTable::new());
                let diagnostics_before = ctx.diagnostics().len();
                collect_type_declarations(enclosing_statements, ctx);
                ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
                let table = Arc::new(std::mem::replace(&mut ctx.type_declarations, saved));
                let scope = Arc::new(TypeDeclarationScope::new(vec![table.clone()]));
                (table, scope)
            });
            if names_entity(enclosing_table, target) {
                entity_alias_type_entries(enclosing_table, target, local, Some(&*enclosing_scope))
            } else if let Some(entries) =
                imported_entity_alias_type_entries(enclosing_statements, target, local, parsed_files, ctx)
            {
                entries
            } else {
                entity_alias_type_entries(&ctx.ambient_global_type_declarations, target, local, None)
            }
        };
        for (name, declaration) in entries {
            let _ = block_table.insert(name, declaration);
        }
    }
}

/// [`entity_alias_type_entries`] for an entity whose root the enclosing file
/// imports by name (`import { JSXInternal } from ".."` then `export import JSX =
/// JSXInternal` in its `declare global`): the entity is read from the imported
/// file's own declarations, which is where it resolves.
fn imported_entity_alias_type_entries(
    enclosing_statements: &[ParsedStatement],
    target: &str,
    local: &str,
    parsed_files: &[crate::program::ParsedProgramFile],
    ctx: &mut CheckerContext,
) -> Option<Vec<(String, TypeDeclarationInfo)>> {
    let (root, rest) = match target.split_once('.') {
        Some((root, rest)) => (root, Some(rest)),
        None => (target, None),
    };
    let (module_specifier, imported_name) = enclosing_statements.iter().find_map(|statement| {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            return None;
        };
        let surge_ts_syntax::ParsedImportKind::Named { specifiers, .. } = &import.kind else {
            return None;
        };
        specifiers
            .iter()
            .find(|specifier| specifier.local_name == root)
            .map(|specifier| (import.module_specifier.as_str(), specifier.imported_name.as_str()))
    })?;
    let index = crate::program::module_file_index_for_specifier(
        &ctx.file_name.clone(),
        module_specifier,
        parsed_files,
        ctx,
    )?;
    let imported_file = parsed_files.get(index)?;
    let saved = std::mem::replace(&mut ctx.type_declarations, crate::symbols::TypeDeclarationTable::new());
    let diagnostics_before = ctx.diagnostics().len();
    collect_type_declarations(&imported_file.statements, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    let table = std::mem::replace(&mut ctx.type_declarations, saved);
    let scope = Arc::new(crate::symbols::TypeDeclarationScope::new(vec![Arc::new(table.clone())]));
    let entity = match rest {
        Some(rest) => format!("{imported_name}.{rest}"),
        None => imported_name.to_string(),
    };
    let entries = entity_alias_type_entries(&table, &entity, local, Some(&scope));
    (!entries.is_empty()).then_some(entries)
}

/// The type declarations the entity path `target` names in `source`, keyed
/// under the alias `local`: the entity itself and its qualified members.
fn entity_alias_type_entries(
    source: &crate::symbols::TypeDeclarationTable,
    target: &str,
    local: &str,
    scope: Option<&Arc<crate::symbols::TypeDeclarationScope>>,
) -> Vec<(String, TypeDeclarationInfo)> {
    let mut entries = Vec::new();
    if let Some(declaration) = source.get(target) {
        let declaration =
            crate::modules::exports::attach_type_resolution_scope_if_missing(declaration.clone(), scope);
        entries.push((
            local.to_string(),
            crate::modules::exports::rename_type_declaration(declaration, local.to_string()),
        ));
    }
    let prefix = format!("{target}.");
    for (key, declaration) in source.iter() {
        if let Some(member) = key.strip_prefix(prefix.as_str()) {
            entries.push((
                format!("{local}.{member}"),
                crate::modules::exports::attach_type_resolution_scope_if_missing(
                    declaration.clone(),
                    scope,
                ),
            ));
        }
    }
    entries
}

/// A global function re-declared by another declaration file is an *overload*
/// of the same name, not a shadow: `@types/node` re-declares `clearTimeout` for
/// `NodeJS.Timeout` alongside the DOM's numeric one. First-wins kept only the
/// first, so the other's call sites reported TS2345.
pub(crate) fn merged_global_function_type(
    existing: Option<&crate::symbols::SymbolInfo>,
    incoming: surge_ts_types::FunctionType,
) -> Option<surge_ts_types::FunctionType> {
    match existing {
        None => Some(incoming),
        Some(existing) => match (&existing.ty, existing.kind) {
            (surge_ts_types::Type::Function(existing), crate::symbols::SymbolKind::Function) => {
                Some(crate::infer::types::interface::merge_overload_signatures(
                    existing, &incoming,
                ))
            }
            // A non-function binding of the same name keeps its own answer.
            _ => None,
        },
    }
}

/// `enclosing_statements` are the declarations of the file, or of the ambient
/// module, the block is written in.
fn lower_global_augmentation_values(
    block_statements: &[ParsedStatement],
    enclosing_statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    // Value types resolve against the caller's current type environment (during
    // binding: the module's local declarations plus its import scope) and the
    // ambient table. The block's own types were merged into the ambient table by
    // `merge_global_augmentation_types`, so a split global interface resolves to
    // its fully-merged form rather than this block's partial declaration.
    let saved_symbols = std::mem::take(&mut ctx.symbols);
    let mut current_symbols = crate::symbols::SymbolTable::new();
    let mut local_function_signatures = HashMap::new();
    collect_function_signatures_from_statements(
        block_statements,
        0,
        &mut current_symbols,
        &mut local_function_signatures,
        ctx,
    );
    ctx.symbols = current_symbols;

    for stmt in block_statements {
        let var = match stmt {
            ParsedStatement::VariableDeclaration(var) => Some(var),
            ParsedStatement::ExportDeclaration(export) => {
                if let surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } =
                    export.as_ref()
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

        // A `declare global { namespace awslambda { … } }` block publishes the
        // namespace as a global *value* too, so `awslambda.HttpResponseStream`
        // resolves. Same first-wins discipline the variable arm below uses.
        if let ParsedStatement::NamespaceDeclaration(namespace) = stmt
            && ctx.ambient_global_symbols.get(&namespace.name).is_none()
            && crate::program::is_instantiated_namespace(namespace)
        {
            crate::program::record_augmentation_value_insertion();
            ctx.ambient_global_symbols.insert(
                namespace.name.clone(),
                crate::symbols::SymbolInfo {
                    ty: crate::modules::namespace_value_object_type(namespace),
                    kind: crate::symbols::SymbolKind::Const,
                    function_signature: None,
                },
            );
        }

        if let Some(var) = var {
            // `declare global { let assert: typeof import("vitest")["assert"] }`
            // reads a module namespace that only exists once imports are
            // bound, after this pass; the annotation maps on first read.
            let ty = match var.declared_type.as_ref() {
                Some(annotation)
                    if crate::modules::exports::annotation_contains_import_type_query(
                        annotation,
                    ) =>
                {
                    ctx.register_import_type_global(&var.name);
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
            if ctx.ambient_global_symbols.get(&var.name).is_none() {
                crate::program::record_augmentation_value_insertion();
                if !matches!(var.kind, surge_ts_syntax::ParsedVariableKind::Var) {
                    Arc::make_mut(&mut ctx.block_scoped_globals)
                        .insert(Arc::from(var.name.as_str()));
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
            }
        }
    }

    let mut ordered_function_signatures = local_function_signatures.into_iter().collect::<Vec<_>>();
    ordered_function_signatures
        .sort_by_key(|(location, _)| (location.file_index, location.statement_index));
    for (loc, fun_ty) in ordered_function_signatures {
        let name = match &block_statements[loc.statement_index] {
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

        if let Some(merged) =
            merged_global_function_type(ctx.ambient_global_symbols.get(&name), fun_ty)
        {
            crate::program::record_augmentation_value_insertion();
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

    // A class in the block is a global value as well as a global type, the
    // way a script's `declare class` is.
    for stmt in block_statements {
        if let ParsedStatement::ClassDeclaration(class) =
            crate::modules::peel_exported_statement(stmt)
            && ctx.ambient_global_symbols.get(&class.name).is_none()
        {
            crate::program::record_augmentation_value_insertion();
            let symbol = crate::program::build_class_value_symbol(class, ctx);
            ctx.ambient_global_symbols.insert(class.name.clone(), symbol);
        }
    }

    for stmt in block_statements {
        if let ParsedStatement::ImportDeclaration(import) =
            crate::modules::peel_exported_statement(stmt)
            && let surge_ts_syntax::ParsedImportKind::EntityAlias {
                local_name, target, ..
            } = &import.kind
            && ctx.ambient_global_symbols.get(local_name).is_none()
            && let Some(ty) =
                global_augmentation_alias_value(target, block_statements, enclosing_statements, ctx)
        {
            crate::program::record_augmentation_value_insertion();
            ctx.ambient_global_symbols.insert(
                local_name.clone(),
                crate::symbols::SymbolInfo {
                    ty,
                    kind: crate::symbols::SymbolKind::Const,
                    function_signature: None,
                },
            );
        }
    }

    ctx.symbols = saved_symbols;
}

/// The value an `import X = N.M` in a `declare global` block aliases. The
/// entity's root resolves the way tsc's `resolveEntityName` climbs from the
/// block: its own namespaces, then those of the file or ambient module it is
/// written in, then the globals.
fn global_augmentation_alias_value(
    target: &str,
    block_statements: &[ParsedStatement],
    enclosing_statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) -> Option<surge_ts_types::Type> {
    let mut segments = target.split('.');
    let root = segments.next()?;
    let mut ty = match namespace_value_named(block_statements, root, ctx) {
        Some(ty) => ty,
        None => match namespace_value_named(enclosing_statements, root, ctx) {
            Some(ty) => ty,
            None => ctx.ambient_global_symbols.get(root)?.ty.clone(),
        },
    };
    for member in segments {
        let surge_ts_types::Type::Object(object) = ty.peeled() else {
            return None;
        };
        ty = object.properties.get(member)?.ty.clone();
    }
    Some(ty)
}

fn namespace_value_named(
    statements: &[ParsedStatement],
    name: &str,
    ctx: &mut CheckerContext,
) -> Option<surge_ts_types::Type> {
    let namespace = statements.iter().find_map(|statement| {
        match crate::modules::peel_exported_statement(statement) {
            ParsedStatement::NamespaceDeclaration(namespace)
                if namespace.name == name && crate::program::is_instantiated_namespace(namespace) =>
            {
                Some(namespace)
            }
            _ => None,
        }
    })?;
    Some(crate::modules::namespace_value_object_type_resolved(namespace, ctx))
}

pub(crate) fn sync_global_this_symbol(ctx: &mut CheckerContext) {
    sync_global_this_symbol_with_scripts(ctx, &[]);
}

/// Builds `typeof globalThis` from the ambient globals and `script_members`,
/// the values the program's scripts put on the global object.
pub(crate) fn sync_global_this_symbol_with_scripts(
    ctx: &mut CheckerContext,
    script_members: &[(String, surge_ts_types::Type)],
) {
    use surge_ts_types::PropertyMap;

    let mut properties = PropertyMap::default();
    for (name, symbol) in ctx.ambient_global_symbols.iter() {
        // A block-scoped global (`let`/`const`) is not a property of the
        // global object.
        if name.as_ref() == "globalThis" || ctx.block_scoped_globals.contains(name.as_ref()) {
            continue;
        }

        properties.insert(
            name.clone(),
            surge_ts_types::ObjectProperty::required(symbol.ty.clone()),
        );
    }
    for (name, ty) in script_members {
        if properties.get(name.as_str()).is_none() {
            properties.insert(
                name.as_str().into(),
                surge_ts_types::ObjectProperty::required(ty.clone()),
            );
        }
    }

    // The global object is a nominal reference, not a bare structural object,
    // for the same reason interfaces are: `Window & typeof globalThis` names
    // itself through `window`/`self`, and only nominal identity lets two sites
    // that spell it agree without unfolding it. As a structural operand every
    // resolution site minted a fresh merged surface, so the intersection merge
    // and assignability walked an unbounded unfolding of the same type; a
    // reference keeps `typeof globalThis` deferred inside the intersection and
    // tsc-shaped in diagnostics.
    //
    // Go declares `globalThis` in the globals table it exports, so the global
    // object has itself as a read-only member. The member holds the object
    // weakly: a strong self-reference would be a cycle nothing reclaims.
    let global_object = std::sync::Arc::new_cyclic(|own: &std::sync::Weak<surge_ts_types::Type>| {
        properties.insert(
            "globalThis".into(),
            surge_ts_types::ObjectProperty::required(global_object_reference(std::sync::Arc::new(
                WeakGlobalObjectType(own.clone()),
            )))
            .with_readonly(true),
        );
        surge_ts_types::Type::Object(crate::metrics::alloc_object_type(properties, None))
    });
    ctx.ambient_global_symbols.insert(
        "globalThis".to_string(),
        crate::symbols::SymbolInfo {
            ty: global_object_reference(std::sync::Arc::new(GlobalObjectType(global_object))),
            kind: crate::symbols::SymbolKind::Const,
            function_signature: None,
        },
    );
}

/// Reference id of `typeof globalThis`. Declaration ids are `file\0Name`, so a
/// leading NUL keeps it disjoint from every declaration, like the intersection
/// ids.
pub(crate) const GLOBAL_THIS_REFERENCE_ID: &str = "\u{0}globalThis";

fn global_object_reference(
    resolver: std::sync::Arc<dyn surge_ts_types::ResolveReference>,
) -> surge_ts_types::Type {
    surge_ts_types::Type::Reference(surge_ts_types::TypeReference::new(
        GLOBAL_THIS_REFERENCE_ID,
        "typeof globalThis",
        Vec::new(),
        resolver,
    ))
}

struct GlobalObjectType(std::sync::Arc<surge_ts_types::Type>);

/// `globalThis.globalThis`: the global object, held weakly by its own member.
struct WeakGlobalObjectType(std::sync::Weak<surge_ts_types::Type>);

impl surge_ts_types::ResolveReference for WeakGlobalObjectType {
    fn resolve(&self) -> surge_ts_types::Type {
        self.0.upgrade().map_or(surge_ts_types::Type::Unknown, |object| (*object).clone())
    }

    fn resolve_arc(&self) -> std::sync::Arc<surge_ts_types::Type> {
        self.0
            .upgrade()
            .unwrap_or_else(|| std::sync::Arc::new(surge_ts_types::Type::Unknown))
    }

    fn peek_resolved(&self) -> Option<std::sync::Arc<surge_ts_types::Type>> {
        self.0.upgrade()
    }
}

impl surge_ts_types::ResolveReference for GlobalObjectType {
    fn resolve(&self) -> surge_ts_types::Type {
        (*self.0).clone()
    }

    fn resolve_arc(&self) -> std::sync::Arc<surge_ts_types::Type> {
        self.0.clone()
    }

    fn peek_resolved(&self) -> Option<std::sync::Arc<surge_ts_types::Type>> {
        Some(self.0.clone())
    }
}

pub(crate) fn validate_direct_utility_aliases(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    for statement in statements {
        validate_direct_utility_aliases_from_statement(statement, ctx);
    }
}

pub(crate) fn validate_local_type_declarations(
    statements: &[ParsedStatement],
    file_name: &str,
    ctx: &mut CheckerContext,
) {
    let mut local_declarations = Vec::new();
    let mut seen = HashSet::new();

    collect_local_type_declarations_from_statements(
        statements,
        file_name,
        &mut seen,
        &mut local_declarations,
        ctx,
    );

    for declaration in local_declarations.into_iter().rev() {
        validate_local_type_declaration(&declaration, ctx);
    }
}

/// tsc's `checkModuleDeclaration` checks a namespace body's declarations as it
/// checks a file's. The namespace's own members are registered under their
/// qualified name, `prefix.Name`; nested namespaces validate their own members
/// when their bodies are checked.
pub(crate) fn validate_namespace_type_declarations(
    statements: &[ParsedStatement],
    prefix: &str,
    file_name: &str,
    ctx: &mut CheckerContext,
) {
    let mut local_declarations = Vec::new();
    let mut seen = HashSet::new();
    for statement in statements {
        let declaration = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                _ => continue,
            },
            other => other,
        };
        let (name, kind) = match declaration {
            ParsedStatement::TypeAliasDeclaration(alias) => (&alias.name, LocalDeclarationKind::Alias),
            ParsedStatement::InterfaceDeclaration(interface) => {
                (&interface.name, LocalDeclarationKind::Interface)
            }
            ParsedStatement::ClassDeclaration(class) => (&class.name, LocalDeclarationKind::Interface),
            _ => continue,
        };
        collect_named_local_type_declaration(
            &format!("{prefix}.{name}"),
            file_name,
            kind,
            &mut seen,
            &mut local_declarations,
            ctx,
        );
    }
    for declaration in local_declarations.into_iter().rev() {
        validate_local_type_declaration(&declaration, ctx);
    }
}

fn collect_local_type_declarations_from_statements(
    statements: &[ParsedStatement],
    file_name: &str,
    seen: &mut HashSet<DeclarationResolutionKey>,
    local_declarations: &mut Vec<TypeDeclarationInfo>,
    ctx: &CheckerContext,
) {
    for statement in statements {
        collect_local_type_declarations_from_statement(
            statement,
            file_name,
            seen,
            local_declarations,
            ctx,
        );
    }
}

fn collect_local_type_declarations_from_statement(
    statement: &ParsedStatement,
    file_name: &str,
    seen: &mut HashSet<DeclarationResolutionKey>,
    local_declarations: &mut Vec<TypeDeclarationInfo>,
    ctx: &CheckerContext,
) {
    match statement {
        ParsedStatement::TypeAliasDeclaration(alias) => {
            collect_named_local_type_declaration(
                &alias.name,
                file_name,
                LocalDeclarationKind::Alias,
                seen,
                local_declarations,
                ctx,
            );
        }
        ParsedStatement::InterfaceDeclaration(interface) => {
            collect_named_local_type_declaration(
                &interface.name,
                file_name,
                LocalDeclarationKind::Interface,
                seen,
                local_declarations,
                ctx,
            );
        }
        ParsedStatement::ClassDeclaration(class) => {
            collect_named_local_type_declaration(
                &class.name,
                file_name,
                LocalDeclarationKind::Interface,
                seen,
                local_declarations,
                ctx,
            );
        }
        ParsedStatement::ExportDeclaration(export) => {
            if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                collect_local_type_declarations_from_statement(
                    declaration.as_ref(),
                    file_name,
                    seen,
                    local_declarations,
                    ctx,
                )
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum LocalDeclarationKind {
    Alias,
    Interface,
}

fn collect_named_local_type_declaration(
    name: &str,
    file_name: &str,
    kind: LocalDeclarationKind,
    seen: &mut HashSet<DeclarationResolutionKey>,
    local_declarations: &mut Vec<TypeDeclarationInfo>,
    ctx: &CheckerContext,
) {
    let canonical_file_name =
        crate::paths::canonicalize_if_exists_arc(std::path::Path::new(file_name));
    let key = DeclarationResolutionKey {
        file_name: canonical_file_name.clone(),
        name: std::sync::Arc::from(name),
        namespace: DeclarationNamespace::Type,
        fingerprint: 0,
    };
    if !seen.insert(key) {
        return;
    }

    let matches = |declaration: &TypeDeclarationInfo| match (kind, declaration) {
        (LocalDeclarationKind::Alias, TypeDeclarationInfo::Alias(info)) => {
            &*info.name == name
                && canonicalize_if_exists_string(std::path::Path::new(&*info.file_name))
                    == *canonical_file_name
        }
        (LocalDeclarationKind::Interface, TypeDeclarationInfo::Interface(info)) => {
            &*info.name == name
                && canonicalize_if_exists_string(std::path::Path::new(&*info.file_name))
                    == *canonical_file_name
        }
        _ => false,
    };

    // Local declarations are keyed by their own name, so the O(1) lookup covers
    // essentially every hit; the full scan only runs when the keyed entry was
    // shadowed (e.g. by an import alias) or absent.
    let declaration = ctx
        .type_declarations
        .get(name)
        .filter(|declaration| matches(declaration))
        .or_else(|| {
            ctx.type_declarations
                .iter()
                .map(|(_, declaration)| declaration)
                .find(|declaration| matches(declaration))
        });

    if let Some(declaration) = declaration {
        local_declarations.push(attach_current_type_scope_if_missing(
            declaration.clone(),
            ctx,
        ));
    }
}

fn attach_current_type_scope_if_missing(
    declaration: TypeDeclarationInfo,
    ctx: &CheckerContext,
) -> TypeDeclarationInfo {
    let current_scope = ctx.type_declaration_scope.clone().unwrap_or_else(|| {
        Arc::new(crate::symbols::TypeDeclarationScope::new(vec![Arc::new(
            ctx.type_declarations.clone(),
        )]))
    });

    match declaration {
        TypeDeclarationInfo::Alias(mut alias) => {
            alias.resolution_scope = Some(current_scope);
            TypeDeclarationInfo::Alias(alias)
        }
        TypeDeclarationInfo::Interface(mut interface) => {
            interface.resolution_scope = Some(current_scope);
            TypeDeclarationInfo::Interface(interface)
        }
    }
}

/// `earlier` holds the statements written before a local `statement`; an
/// exported declaration is collected without them.
fn collect_type_declarations_from_statement(
    statement: &ParsedStatement,
    earlier: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::TypeAliasDeclaration(alias) => {
            collect_type_alias(alias, earlier, ctx);
        }
        ParsedStatement::InterfaceDeclaration(interface) => {
            collect_interface(interface, earlier, ctx);
        }
        ParsedStatement::ClassDeclaration(class) => {
            crate::program::collect_class(class, ctx);
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                collect_type_declarations_from_statement(declaration.as_ref(), &[], ctx)
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Class(class),
                ..
            } => {
                crate::program::collect_class(class, ctx);
            }
            _ => {}
        },
        ParsedStatement::NamespaceDeclaration(namespace) => {
            collect_namespace_type_declarations(namespace, ctx);
        }
        _ => {}
    }
}

/// `export { X }` inside `declare namespace ns` re-exports an outer binding as
/// `ns.X` — fastify's entry file publishes `FastifyRequest` this way from an
/// import. Imports live in the module's scope layers, not its local table, so
/// this runs once the scope is assembled and returns the local table with those
/// members added (shared unchanged when there are none). The export-table pass
/// then publishes `ns.X` bare like any declared member, which is what gives a
/// `declare module` augmentation of `X` a real interface to merge into rather
/// than an empty slot to shadow.
pub(crate) fn with_namespace_reexports(
    local: Arc<crate::symbols::TypeDeclarationTable>,
    statements: &[ParsedStatement],
    scope: &crate::symbols::TypeDeclarationScope,
) -> Arc<crate::symbols::TypeDeclarationTable> {
    let mut additions: Vec<(String, TypeDeclarationInfo)> = Vec::new();
    for statement in statements {
        let namespace = match statement {
            ParsedStatement::NamespaceDeclaration(namespace) => namespace,
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => {
                    match declaration.as_ref() {
                        ParsedStatement::NamespaceDeclaration(namespace) => namespace,
                        _ => continue,
                    }
                }
                _ => continue,
            },
            _ => continue,
        };
        for inner in &namespace.statements {
            let ParsedStatement::ExportDeclaration(export) = inner else {
                continue;
            };
            let ParsedExportDeclaration::Named {
                specifiers,
                module_specifier: None,
                ..
            } = export.as_ref()
            else {
                continue;
            };
            for specifier in specifiers {
                let key = format!("{}.{}", namespace.name, specifier.exported_name);
                if local.get(&key).is_some() {
                    continue;
                }
                let Some(declaration) = scope.get(&specifier.local_name) else {
                    continue;
                };
                additions.push((
                    key.clone(),
                    crate::modules::rename_type_declaration(declaration.clone(), key),
                ));
            }
        }
    }
    if additions.is_empty() {
        return local;
    }
    let mut table = local.as_ref().clone();
    for (key, declaration) in additions {
        let _ = table.insert(&key, declaration);
    }
    Arc::new(table)
}

/// Registers a namespace's interfaces and type aliases under qualified names
/// (`JSX.IntrinsicElements`, `JSX.Element`, ...) so the JSX checker can resolve
/// them. Members are not leaked into the unqualified scope. Nested namespaces are
/// flattened by their already-dotted names.
fn collect_namespace_type_declarations(
    namespace: &ParsedNamespaceDeclaration,
    ctx: &mut CheckerContext,
) {
    let ambient =
        namespace.is_declare || crate::modules::is_declaration_file_name(&ctx.file_name);
    let exports_table: Arc<str> = Arc::from(namespace.name.as_str());
    collect_namespace_type_declarations_prefixed(
        namespace,
        &namespace.name,
        &exports_table,
        ambient,
        ctx,
    );
}

/// Default-on (opt-out `SURGE_NS_IFACE_MERGE=0`): fold a namespace's re-opened
/// interfaces into one declaration. Correct and worth **32 fewer false
/// positives on tRPC** (no new ones) — typescript.d.ts splits `Node`, `Type`,
/// `Symbol`, `Identifier`, `SourceFile`, `Signature` and four more into two
/// blocks each, and first-wins drops the block carrying their whole
/// service-method half (`ts.Type.getProperty`, `ts.Symbol.getName`,
/// `Node.getText`, `Identifier.text`).
///
/// The merged blocks are *mutually cyclic* (merged `ts.Node` gains methods
/// returning `SourceFile`, whose merged block returns `Node`), and a degraded
/// expansion is never interned, so before the check-phase degraded-peel pin on
/// `LazyInstantiation` every consumer peel re-expanded the cycle (+223% wall
/// on tRPC, member visits 3.3M -> 26.2M). With the pin the merge measures at
/// baseline wall (27.9s -> 8.1s on tRPC); the pin is what makes this default
/// affordable.
fn namespace_interface_merge_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("SURGE_NS_IFACE_MERGE").is_none_or(|value| value != "0"))
}

/// tsc's `hasExportDeclarations`: an `export { … }`, `export * from …` or
/// export assignment among a block's statements.
fn has_export_declarations(statements: &[ParsedStatement]) -> bool {
    statements.iter().any(|statement| {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            return false;
        };
        !matches!(
            export.as_ref(),
            ParsedExportDeclaration::Statement { .. }
                | ParsedExportDeclaration::NamespaceExport { .. }
                | ParsedExportDeclaration::Unsupported { .. }
                | ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Function(_)
                        | ParsedDefaultExportDeclaration::Class(_),
                    ..
                }
        )
    })
}

/// Declares an interface or class instance side of a namespace in `table`
/// (see [`InterfaceInfo::namespace_member_table`]): it merges into a
/// declaration of the same table, and is first-wins against anything else.
/// Re-collecting the same fragments leaves the entry as it is.
fn register_namespace_member_interface(
    key: String,
    mut info: InterfaceInfo,
    table: &Arc<str>,
    report_conflicts: bool,
    ctx: &mut CheckerContext,
) {
    let Some(handle) = ctx.type_declarations.get_handle(&key) else {
        info.namespace_member_table = Some(table.clone());
        let _ = ctx
            .type_declarations
            .insert(key, TypeDeclarationInfo::Interface(info));
        return;
    };
    let TypeDeclarationInfo::Interface(existing) = handle.get() else {
        return;
    };
    if !namespace_interface_merge_enabled()
        || existing.namespace_member_table.as_deref() != Some(table.as_ref())
        || info
            .body
            .declaration_fragments
            .iter()
            .all(|fragment| existing.body.declaration_fragments.contains(fragment))
    {
        return;
    }
    let incoming = if report_conflicts {
        filter_conflicting_interface_members(existing, info, ctx)
    } else {
        info
    };
    let mut merged = crate::symbols::merge_interface_infos(existing, &incoming);
    merged.namespace_member_table = Some(table.clone());
    ctx.type_declarations
        .upsert(key, TypeDeclarationInfo::Interface(merged));
}

/// `prefix` is the fully-qualified dotted path to `namespace` (`React`, then
/// `React.JSX`, …). Each member is registered under the full path so a qualified
/// consumer reference like `React.JSX.IntrinsicElements` resolves; a nested member
/// is ALSO registered under the bare immediate-namespace key (`JSX.IntrinsicElements`)
/// so the JSX checker's literal lookup and the unqualified sibling references inside
/// the library's own bodies keep resolving.
///
/// `exports_table` identifies the namespace's export table, which all of its
/// blocks share (tsc merges every block into one symbol). A member goes there
/// when it is exported — written `export`, or any member of an ambient block
/// with no export declarations (tsc's `setExportContextFlag`) — and otherwise
/// into the locals of its own block, so re-opened interfaces merge across the
/// blocks of a namespace only when they are exported.
fn collect_namespace_type_declarations_prefixed(
    namespace: &ParsedNamespaceDeclaration,
    prefix: &str,
    exports_table: &Arc<str>,
    ambient: bool,
    ctx: &mut CheckerContext,
) {
    let bare_prefix = namespace.name.as_str();
    let export_context = ambient && !has_export_declarations(&namespace.statements);
    let locals_table: Arc<str> = Arc::from(format!(
        "{}#{}",
        ctx.file_name,
        namespace
            .span
            .or(namespace.name_span)
            .map_or(0, |span| span.start)
    ));
    let member_keys = |name: &str| {
        let mut keys = vec![format!("{prefix}.{name}")];
        if prefix != bare_prefix {
            keys.push(format!("{bare_prefix}.{name}"));
        }
        keys
    };
    for statement in &namespace.statements {
        let (exported, inner) = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => {
                    (true, declaration.as_ref())
                }
                _ => (false, statement),
            },
            other => (false, other),
        };
        let table = if exported || export_context {
            exports_table
        } else {
            &locals_table
        };

        match inner {
            ParsedStatement::InterfaceDeclaration(interface) => {
                for (key_index, key) in member_keys(&interface.name).into_iter().enumerate() {
                    let info = InterfaceInfo::new(
                        key.clone(),
                        ctx.file_name_arc(),
                        interface.name_span,
                        interface.type_parameters.clone(),
                        interface.extends.clone(),
                        interface.members.clone(),
                        interface.string_index_type.clone(),
                        interface.number_index_type.clone(),
                        interface.call_signature.clone(),
                        interface.call_signature_overloads.clone(),
                        interface.construct_signatures.clone(),
                        None,
                    )
                    .with_readonly_indexes(interface.string_index_readonly, interface.number_index_readonly)
                    .with_keyed_indexes(interface.keyed_indexes.clone());
                    register_namespace_member_interface(key, info, table, key_index == 0, ctx);
                }
            }
            ParsedStatement::TypeAliasDeclaration(alias) => {
                for key in member_keys(&alias.name) {
                    let info = TypeAliasInfo::new(
                        key.clone(),
                        ctx.file_name_arc(),
                        alias.name_span,
                        alias.type_parameters.clone(),
                        alias.ty.clone(),
                        None,
                    )
                    .with_enum_name(
                        alias.enum_name.as_deref(),
                        alias.enum_exported,
                        alias.enum_is_const,
                    )
                    .with_enum_members(&alias.enum_members);
                    let _ = ctx
                        .type_declarations
                        .insert(key, TypeDeclarationInfo::Alias(info));
                }
            }
            // A class inside a namespace contributes an instance type under the
            // qualified key just as an interface does; without this a
            // `namespace NS { class C {} }` member is reachable as a value but
            // never as a type. A class and a same-named interface merge just as
            // two interfaces do (`class Duplex …` + `interface Duplex extends
            // Readable, Writable {}` in @types/node's stream namespace is how
            // `destroy` reaches Duplex).
            ParsedStatement::ClassDeclaration(class) => {
                for key in member_keys(&class.name) {
                    let mut info =
                        crate::program::class_instance_interface_info(class, ctx.file_name_arc());
                    // Qualified name with NO declared_name, mirroring the
                    // interface arm above: the namespace-member prefix push in
                    // `resolve_interface` derives from `declared_name.unwrap_or
                    // (name)`, so recording the bare class name as declared_name
                    // kills the prefix and every bare sibling reference in the
                    // class body (`Duplex extends Readable` inside
                    // `namespace Stream`) misses. The `export =` flattening's
                    // bare copy captures the qualified name as declared_name on
                    // rename, exactly as it does for interfaces.
                    info.name = key.as_str().into();
                    register_namespace_member_interface(key, info, table, false, ctx);
                }
            }
            ParsedStatement::NamespaceDeclaration(inner_namespace) => {
                let member = inner_namespace.member_name();
                let inner_prefix = format!("{prefix}.{member}");
                // `namespace A.B {}` exports `B` from `A` without `export`.
                let dotted = inner_namespace
                    .name
                    .strip_prefix(namespace.name.as_str())
                    .is_some_and(|rest| rest.starts_with('.'));
                let enclosing_table = if dotted { exports_table } else { table };
                let inner_exports: Arc<str> = Arc::from(format!("{enclosing_table}.{member}"));
                collect_namespace_type_declarations_prefixed(
                    inner_namespace,
                    &inner_prefix,
                    &inner_exports,
                    ambient || inner_namespace.is_declare,
                    ctx,
                );
            }
            _ => {}
        }
    }
}

fn check_statement(statement: ParsedStatement, ctx: &mut CheckerContext) {
    match statement {
        ParsedStatement::VariableDeclaration(variable) => {
            var::check_variable_declaration(*variable, ctx);
        }
        ParsedStatement::Assignment(assignment) => {
            crate::program::check_module_assignment(*assignment, ctx);
        }
        ParsedStatement::MemberAssignment(assignment) => {
            let symbols = ctx
                .symbols
                .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            let mut scopes = crate::symbols::ScopeStack::from_root(symbols);
            crate::checks::function::check_member_assignment(*assignment, &mut scopes, ctx);
        }
        ParsedStatement::FunctionDeclaration(function) => {
            crate::program::with_declared_mutable_module_bindings(ctx, |ctx| {
                check_function::check_function_declaration(*function, ctx);
            });
        }
        ParsedStatement::Call(call) => {
            let assertion = crate::program::module_call_expression(&call);
            call::check_call(*call, ctx);
            crate::program::narrow_module_assertion_call(assertion, ctx);
        }
        ParsedStatement::Expression(expression) => {
            let assertion = crate::program::assertion_candidate(&expression)
                .then(|| (*expression).clone());
            expr::check_expression_statement(*expression, ctx);
            crate::program::narrow_module_assertion_call(assertion, ctx);
        }
        ParsedStatement::If(if_statement) => {
            crate::program::check_module_if_statement(&if_statement, ctx);
        }
        ParsedStatement::Block(statements) => {
            crate::program::check_module_block(statements, ctx);
        }
        ParsedStatement::TypeAliasDeclaration(_) => {}
        ParsedStatement::InterfaceDeclaration(_) => {}
        ParsedStatement::ClassDeclaration(class) => {
            crate::program::check_class_declaration(&class, ctx);
        }
        ParsedStatement::ImportDeclaration(import) => {
            if crate::modules::is_external_specifier(&import.module_specifier) {
                let suppress_unresolved_diagnostic =
                    matches!(&import.kind, surge_ts_syntax::ParsedImportKind::SideEffect)
                        && (!ctx.options.no_unchecked_side_effect_imports
                            || is_runtime_js_only_module(&import.module_specifier, ctx));

                if !ctx.options.stub_external_modules && !suppress_unresolved_diagnostic {
                    let mut diagnostic = match &import.kind {
                        surge_ts_syntax::ParsedImportKind::SideEffect => {
                            Diagnostic::ts2882(&import.module_specifier, ctx.file_name.clone())
                        }
                        _ => Diagnostic::ts2307(&import.module_specifier, ctx.file_name.clone()),
                    };
                    if let Some(span) = import.module_specifier_span {
                        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                    }
                    ctx.push(diagnostic);
                }

                // A reported TS2307 means the program genuinely has no type
                // here, which is tsc's error type (`any`) — implicit-any and
                // argument checking downstream are real, not surge degradation.
                // Under `stubExternalModules` the diagnostic is suppressed on
                // purpose, so the sentinel stays and keeps the cascade quiet.
                let (binding_ty, binding_kind) = if ctx.options.stub_external_modules {
                    (
                        surge_ts_types::Type::Unknown,
                        crate::symbols::SymbolKind::Var,
                    )
                } else {
                    (
                        surge_ts_types::Type::Any,
                        crate::symbols::SymbolKind::ErrorImport,
                    )
                };
                // Stub the imports to avoid cascades in single-file mode
                match &import.kind {
                    surge_ts_syntax::ParsedImportKind::Named {
                        specifiers,
                        is_type_only,
                    } => {
                        for specifier in specifiers {
                            if *is_type_only {
                                let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                    crate::symbols::TypeAliasInfo::new(
                                        specifier.local_name.to_string(),
                                        ctx.file_name_arc(),
                                        specifier.name_span,
                                        vec![],
                                        surge_ts_syntax::ParsedType::ErrorType,
                                        None,
                                    ),
                                );
                                let _ = ctx
                                    .type_declarations
                                    .insert(specifier.local_name.clone(), declaration);
                            } else {
                                let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                    crate::symbols::TypeAliasInfo::new(
                                        specifier.local_name.to_string(),
                                        ctx.file_name_arc(),
                                        specifier.name_span,
                                        vec![],
                                        surge_ts_syntax::ParsedType::ErrorType,
                                        None,
                                    ),
                                );
                                let _ = ctx
                                    .type_declarations
                                    .insert(specifier.local_name.clone(), declaration);
                                let _ = ctx.symbols.insert(
                                    specifier.local_name.clone(),
                                    crate::symbols::SymbolInfo {
                                        ty: binding_ty.clone(),
                                        kind: binding_kind,
                                        function_signature: None,
                                    },
                                );
                            }
                        }
                    }
                    surge_ts_syntax::ParsedImportKind::DefaultAndNamed {
                        local_name,
                        name_span,
                        is_type_only,
                        specifiers,
                    } => {
                        if *is_type_only {
                            let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                crate::symbols::TypeAliasInfo::new(
                                    local_name.clone(),
                                    ctx.file_name_arc(),
                                    *name_span,
                                    vec![],
                                    surge_ts_syntax::ParsedType::ErrorType,
                                    None,
                                ),
                            );
                            let _ = ctx
                                .type_declarations
                                .insert(local_name.clone(), declaration);
                        } else {
                            let _ = ctx.symbols.insert(
                                local_name.clone(),
                                crate::symbols::SymbolInfo {
                                    ty: binding_ty.clone(),
                                    kind: binding_kind,
                                    function_signature: None,
                                },
                            );
                        }

                        for specifier in specifiers {
                            if *is_type_only {
                                let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                    crate::symbols::TypeAliasInfo::new(
                                        specifier.local_name.to_string(),
                                        ctx.file_name_arc(),
                                        specifier.name_span,
                                        vec![],
                                        surge_ts_syntax::ParsedType::ErrorType,
                                        None,
                                    ),
                                );
                                let _ = ctx
                                    .type_declarations
                                    .insert(specifier.local_name.clone(), declaration);
                            } else {
                                let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                    crate::symbols::TypeAliasInfo::new(
                                        specifier.local_name.to_string(),
                                        ctx.file_name_arc(),
                                        specifier.name_span,
                                        vec![],
                                        surge_ts_syntax::ParsedType::ErrorType,
                                        None,
                                    ),
                                );
                                let _ = ctx
                                    .type_declarations
                                    .insert(specifier.local_name.clone(), declaration);
                                let _ = ctx.symbols.insert(
                                    specifier.local_name.clone(),
                                    crate::symbols::SymbolInfo {
                                        ty: binding_ty.clone(),
                                        kind: binding_kind,
                                        function_signature: None,
                                    },
                                );
                            }
                        }
                    }
                    surge_ts_syntax::ParsedImportKind::Default { local_name, .. } => {
                        let _ = ctx.symbols.insert(
                            local_name.clone(),
                            crate::symbols::SymbolInfo {
                                ty: binding_ty.clone(),
                                kind: binding_kind,
                                function_signature: None,
                            },
                        );
                    }
                    surge_ts_syntax::ParsedImportKind::Namespace {
                        local_name,
                        is_type_only,
                        ..
                    } => {
                        if *is_type_only {
                            let declaration = crate::symbols::TypeDeclarationInfo::Alias(
                                crate::symbols::TypeAliasInfo::new(
                                    local_name.clone(),
                                    ctx.file_name_arc(),
                                    None,
                                    vec![],
                                    surge_ts_syntax::ParsedType::ErrorType,
                                    None,
                                ),
                            );
                            let _ = ctx
                                .type_declarations
                                .insert(local_name.clone(), declaration);
                        } else {
                            let _ = ctx.symbols.insert(
                                local_name.clone(),
                                crate::symbols::SymbolInfo {
                                    ty: binding_ty.clone(),
                                    kind: binding_kind,
                                    function_signature: None,
                                },
                            );
                        }
                    }
                    _ => {}
                }
            } else {
                let mut diagnostic =
                    Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                if let Some(span) = import.span.or(import.module_specifier_span) {
                    diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                }

                ctx.push(diagnostic);
            }
        }
        ParsedStatement::ExportDeclaration(export) => match *export {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                check_statement(*declaration, ctx)
            }
            ParsedExportDeclaration::Named {
                module_specifier: Some(specifier),
                span,
                module_specifier_span,
                ..
            } => {
                if crate::modules::is_external_specifier(&specifier) {
                    if !ctx.options.stub_external_modules {
                        let mut diagnostic = Diagnostic::ts2307(&specifier, ctx.file_name.clone());
                        if let Some(span) = module_specifier_span {
                            diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                        }
                        ctx.push(diagnostic);
                    }
                } else {
                    let mut diagnostic =
                        Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                    if let Some(span) = span {
                        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                    }

                    ctx.push(diagnostic);
                }
            }
            ParsedExportDeclaration::Named { .. } => {}
            ParsedExportDeclaration::Namespace { .. } => {}
            ParsedExportDeclaration::Default { declaration, span } => match declaration {
                ParsedDefaultExportDeclaration::Function(function) => {
                    check_function::check_function_declaration(function, ctx);
                }
                ParsedDefaultExportDeclaration::Class(class) => {
                    crate::program::check_class_declaration(&class, ctx);
                }
                ParsedDefaultExportDeclaration::Expression(expression) => {
                    expr::check_expression_statement(expression, ctx);
                }
                ParsedDefaultExportDeclaration::Unsupported { .. } => {
                    let mut diagnostic =
                        Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                    if let Some(span) = span {
                        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                    }

                    ctx.push(diagnostic);
                }
            },
            ParsedExportDeclaration::All {
                module_specifier,
                span,
                module_specifier_span,
                ..
            } => {
                if crate::modules::is_external_specifier(&module_specifier) {
                    if !ctx.options.stub_external_modules {
                        let mut diagnostic =
                            Diagnostic::ts2307(&module_specifier, ctx.file_name.clone());
                        if let Some(span) = module_specifier_span {
                            diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                        }
                        ctx.push(diagnostic);
                    }
                } else {
                    let mut diagnostic =
                        Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                    if let Some(span) = span {
                        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                    }

                    ctx.push(diagnostic);
                }
            }
            ParsedExportDeclaration::Empty { .. } => {}
            ParsedExportDeclaration::Equals { .. } => {}
            ParsedExportDeclaration::NamespaceExport { .. } => {}
            ParsedExportDeclaration::Unsupported { span }
            | ParsedExportDeclaration::EqualsExpression { span, .. } => {
                let mut diagnostic =
                    Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                if let Some(span) = span {
                    diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                }

                ctx.push(diagnostic);
            }
        },
        ParsedStatement::DeclareModuleDeclaration(_) => {}
        // Namespace members are bound during type-declaration collection; the
        // namespace itself produces no value-level checks here.
        ParsedStatement::NamespaceDeclaration(_) => {}
        ParsedStatement::UnsupportedDeclaration { span } => {
            let mut diag = surge_ts_diagnostics::Diagnostic::surge_unsupported_declaration(
                ctx.file_name.clone(),
            );
            if let Some(s) = span {
                diag = diag.with_span(crate::context::convert_span(s));
            }
            ctx.push(diag);
        }
    }
}

pub(crate) fn is_runtime_js_only_module(module_specifier: &str, ctx: &CheckerContext) -> bool {
    let Some(resolved_path) = ctx
        .options
        .resolved_module_for(&ctx.file_name, module_specifier)
    else {
        return false;
    };

    let lower = resolved_path.to_ascii_lowercase();
    lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
}

fn validate_direct_utility_aliases_from_statement(
    statement: &ParsedStatement,
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::TypeAliasDeclaration(alias) => {
            validate_direct_utility_alias(alias, ctx);
        }
        ParsedStatement::ExportDeclaration(export) => {
            if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                validate_direct_utility_aliases_from_statement(declaration.as_ref(), ctx)
            }
        }
        _ => {}
    }
}

fn validate_direct_utility_alias(alias: &ParsedTypeAliasDeclaration, ctx: &mut CheckerContext) {
    if ctx.current_file_kind == crate::context::FileKind::GeneratedDeclaration {
        return;
    }

    let ParsedType::Named(named_type) = &alias.ty else {
        return;
    };

    if !matches!(
        named_type.name.as_str(),
        "Record" | "Partial" | "Pick" | "Omit"
    ) {
        return;
    }

    // The alias's own type parameters are in scope for its body. Seeding them as
    // placeholders keeps the utility's argument probe from reporting them as
    // unknown names, while the constraints-only push leaves the value scope empty
    // so the concrete-instantiation short-circuit still applies.
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for type_parameter in &alias.type_parameters {
        substitution.insert_placeholder(
            type_parameter.name.clone(),
            surge_ts_types::Type::type_parameter(&type_parameter.name),
        );
    }
    ctx.push_type_parameter_constraints_only(&alias.type_parameters);
    let _ = crate::infer::map_parsed_type_with_substitution(alias.ty.clone(), ctx, &substitution);
    ctx.pop_type_parameter_scope();
}

fn classify_file_kind(file_name: &str) -> FileKind {
    if surge_ts_syntax::is_declaration_file_name(file_name) {
        return FileKind::RootDeclaration;
    }

    FileKind::RootSource
}

/// The binder declares an exported declaration's local symbol with none of
/// the declaration's meanings (`declareModuleMember`), so a local type alias or
/// interface written after exported declarations of its name merges with them
/// (TS2395 reports the pair) unless an earlier local declaration already gave
/// the name a type meaning.
fn merges_with_exported_declaration(earlier: &[ParsedStatement], name: &str) -> bool {
    let declares_type = |statement: &ParsedStatement| match statement {
        ParsedStatement::TypeAliasDeclaration(alias) => alias.name == name,
        ParsedStatement::InterfaceDeclaration(interface) => interface.name == name,
        ParsedStatement::ClassDeclaration(class) => class.name == name,
        _ => false,
    };
    let mut exported = false;
    for statement in earlier {
        match statement {
            ParsedStatement::ExportDeclaration(export) => {
                if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref()
                    && declares_type(declaration.as_ref())
                {
                    exported = true;
                }
            }
            other if declares_type(other) => return false,
            _ => {}
        }
    }
    exported
}

pub(crate) fn collect_type_alias(
    alias: &ParsedTypeAliasDeclaration,
    earlier: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    report_duplicate_type_parameters(&alias.type_parameters, ctx);

    let info = TypeAliasInfo::new(
        alias.name.clone(),
        ctx.file_name_arc(),
        alias.name_span,
        alias.type_parameters.clone(),
        alias.ty.clone(),
        None,
    )
    .with_enum_name(
        alias.enum_name.as_deref(),
        alias.enum_exported,
        alias.enum_is_const,
    )
    .with_enum_members(&alias.enum_members);

    let previous = ctx
        .type_declarations
        .insert(alias.name.clone(), TypeDeclarationInfo::Alias(info));

    // Enums lower to aliases, and two `enum E` blocks are a legal merge rather
    // than a redeclaration.
    let merging_enums = alias.enum_name.is_some()
        && matches!(
            previous.as_ref(),
            Some(TypeDeclarationInfo::Alias(previous)) if previous.enum_name.is_some()
        );

    if previous.is_some() && !merging_enums && !merges_with_exported_declaration(earlier, &alias.name) {
        let mut diagnostic = Diagnostic::ts2300(&alias.name, ctx.file_name.clone());

        if let Some(span) = alias.name_span {
            diagnostic = diagnostic.with_span(crate::context::convert_span(span));
        }

        ctx.push(diagnostic);
    }
}

pub(crate) fn collect_interface(
    interface: &ParsedInterfaceDeclaration,
    earlier: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    report_duplicate_type_parameters(&interface.type_parameters, ctx);
    report_conflicting_members_within(&interface.members, ctx);

    let info = InterfaceInfo::new(
        interface.name.clone(),
        ctx.file_name_arc(),
        interface.name_span,
        interface.type_parameters.clone(),
        interface.extends.clone(),
        interface.members.clone(),
        interface.string_index_type.clone(),
        interface.number_index_type.clone(),
        interface.call_signature.clone(),
        interface.call_signature_overloads.clone(),
        interface.construct_signatures.clone(),
        None,
    )
    .with_readonly_indexes(interface.string_index_readonly, interface.number_index_readonly)
    .with_keyed_indexes(interface.keyed_indexes.clone());

    enum Existing {
        None,
        NonInterface,
        Interface(crate::symbols::TypeDeclarationHandle),
    }

    // Classify the existing entry through an `Arc` handle so the common
    // declaration-merging case reads the previously accumulated interface in
    // place instead of deep-cloning its (often large) member list. The handle
    // owns its payload and is decoupled from `ctx`, so the borrowed interface
    // stays valid while `ctx` is borrowed mutably below.
    let existing = match ctx.type_declarations.get_handle(&interface.name) {
        None => Existing::None,
        Some(handle) => match handle.get() {
            TypeDeclarationInfo::Interface(_) => Existing::Interface(handle),
            _ => Existing::NonInterface,
        },
    };

    match existing {
        Existing::None => {
            // Declaration merging with the global scope: a script file's
            // top-level `interface X` re-opens a same-named global interface.
            // Modules shadow instead, and are never collected under this flag.
            let ambient = ctx
                .merge_script_interfaces_with_globals
                .then(|| {
                    ctx.ambient_global_type_declarations
                        .get_handle(&interface.name)
                })
                .flatten()
                .filter(|handle| matches!(handle.get(), TypeDeclarationInfo::Interface(_)));
            match ambient {
                Some(handle) => {
                    let TypeDeclarationInfo::Interface(ambient) = handle.get() else {
                        unreachable!("handle filtered to interfaces above")
                    };
                    let incoming = filter_conflicting_interface_members(ambient, info, ctx);
                    let merged = crate::symbols::merge_interface_infos(ambient, &incoming);
                    let _ = ctx.type_declarations.insert(
                        interface.name.clone(),
                        TypeDeclarationInfo::Interface(merged),
                    );
                }
                None => {
                    let _ = ctx
                        .type_declarations
                        .insert(interface.name.clone(), TypeDeclarationInfo::Interface(info));
                }
            }
        }
        Existing::Interface(handle) => {
            let TypeDeclarationInfo::Interface(existing) = handle.get() else {
                unreachable!("handle classified as interface above")
            };
            let incoming = filter_conflicting_interface_members(existing, info, ctx);
            let merged = crate::symbols::merge_interface_infos(existing, &incoming);
            ctx.type_declarations.upsert(
                interface.name.clone(),
                TypeDeclarationInfo::Interface(merged),
            );
        }
        Existing::NonInterface if merges_with_exported_declaration(earlier, &interface.name) => {}
        Existing::NonInterface => {
            let mut diagnostic = Diagnostic::ts2300(&interface.name, ctx.file_name.clone());

            if let Some(span) = interface.name_span {
                diagnostic = diagnostic.with_span(crate::context::convert_span(span));
            }

            ctx.push(diagnostic);
        }
    }
}

/// TS2717 for a property one interface body declares twice with different
/// types, on the later declaration, as for a later merged declaration.
fn report_conflicting_members_within(members: &[surge_ts_syntax::ParsedInterfaceMember], ctx: &mut CheckerContext) {
    let is_method = |ty: &ParsedType| matches!(ty, ParsedType::Function(_));
    let mut duplicate_computed: Vec<usize> = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let Some(previous_index) = members[..index].iter().position(|previous| previous.name == member.name) else {
            continue;
        };
        let previous = &members[previous_index];
        if is_method(&previous.ty) || is_method(&member.ty) {
            continue;
        }
        // A computed name is late-bound (`lateBindMember`): a second property
        // of the name is a duplicate whatever its type, on every declaration.
        if member.name.starts_with('[') {
            if !duplicate_computed.contains(&previous_index) {
                duplicate_computed.push(previous_index);
            }
            duplicate_computed.push(index);
            continue;
        }
        if previous.ty == member.ty {
            continue;
        }
        // Parsed types carry their spans; the written text decides identity.
        if let (Some(expected), Some(actual)) = (parsed_type_display(&previous.ty), parsed_type_display(&member.ty))
            && expected != actual
        {
            let mut diagnostic = Diagnostic::ts2717(&member.name, expected, actual, ctx.file_name.clone());
            if let Some(span) = member.name_span {
                diagnostic = diagnostic.with_span(crate::context::convert_span(span));
            }
            ctx.push(diagnostic);
        }
    }
    for index in duplicate_computed {
        let member = &members[index];
        if let Some(span) = member.name_span {
            ctx.push(
                Diagnostic::ts2300(&member.name, ctx.file_name.clone())
                    .with_span(crate::context::convert_span(span)),
            );
        }
    }
}

/// Drop members of a later interface declaration whose property type conflicts
/// with the existing declaration and report TS2717 for each. The earlier
/// declaration's type wins (matching TypeScript), so assignability still checks
/// against the first-declared type. Same-named methods are kept as overloads.
fn filter_conflicting_interface_members(
    existing: &InterfaceInfo,
    mut incoming: InterfaceInfo,
    ctx: &mut CheckerContext,
) -> InterfaceInfo {
    // First declaration of each name wins, matching the `find` this replaces.
    // A script's own declaration already sits in the global interface
    // (`collect_ambient_global_types` publishes it), and is no earlier
    // declaration of its own members.
    let mut existing_by_name = HashMap::new();
    for (index, member) in existing.body.members.iter().enumerate() {
        let own = existing
            .body
            .member_fragments
            .get(index)
            .is_some_and(|fragment| incoming.body.declaration_fragments.contains(fragment));
        if own {
            continue;
        }
        existing_by_name
            .entry(member.name.as_str())
            .or_insert(member);
    }

    std::sync::Arc::make_mut(&mut incoming.body)
        .members
        .retain(|member| {
            let Some(previous) = existing_by_name.get(member.name.as_str()).copied() else {
                return true;
            };

            let is_method = |ty: &ParsedType| matches!(ty, ParsedType::Function(_));
            if is_method(&previous.ty) || is_method(&member.ty) || previous.ty == member.ty {
                return true;
            }

            let (Some(expected), Some(actual)) = (
                parsed_type_display(&previous.ty),
                parsed_type_display(&member.ty),
            ) else {
                return false;
            };
            // Parsed types carry their spans; the written text decides
            // identity, and an identical redeclaration merges silently.
            if expected == actual {
                return false;
            }
            {
                let mut diagnostic =
                    Diagnostic::ts2717(&member.name, expected, actual, ctx.file_name.clone());
                if let Some(span) = member.name_span {
                    diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                }
                ctx.push(diagnostic);
            }

            false
        });

    incoming
}

/// A best-effort TypeScript-style rendering of a parsed type for conflict
/// messages. Returns `None` for shapes whose display would not match the
/// compiler exactly; the caller then keeps the first declaration silently.
pub(crate) fn parsed_type_display(ty: &ParsedType) -> Option<String> {
    let rendered = match ty {
        ParsedType::String => "string".to_string(),
        ParsedType::Number => "number".to_string(),
        ParsedType::Boolean => "boolean".to_string(),
        ParsedType::Undefined => "undefined".to_string(),
        ParsedType::Null => "null".to_string(),
        ParsedType::Void => "void".to_string(),
        ParsedType::Any => "any".to_string(),
        ParsedType::Unknown | ParsedType::UnknownKeyword => "unknown".to_string(),
        ParsedType::Never => "never".to_string(),
        ParsedType::StringLiteral(value) => format!("\"{value}\""),
        ParsedType::NumberLiteral(value) => value.clone(),
        ParsedType::BooleanLiteral(value) => value.to_string(),
        ParsedType::Named(named) if named.type_arguments.is_empty() => named.name.clone(),
        _ => return None,
    };
    Some(rendered)
}
