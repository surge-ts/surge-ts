use super::*;

use std::collections::HashMap;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExportSpecifier, ParsedResolutionModeAttribute, ParsedVariableKind};

use crate::context::convert_span;

/// A `var` nested in a module-level block, loop or `try` belongs to the module
/// scope, so an export clause may name it (`export { hoisted }`).
fn with_exported_nested_vars(
    mut values: SymbolTable,
    parsed_file: &ParsedProgramFile,
    local_type_declarations: &TypeDeclarationTable,
    local_symbols: &SymbolTable,
    imported_symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> SymbolTable {
    let mut named: Vec<&str> = Vec::new();
    for statement in &parsed_file.statements {
        if let ParsedStatement::ExportDeclaration(export) = statement
            && let ParsedExportDeclaration::Named {
                specifiers,
                module_specifier: None,
                ..
            } = export.as_ref()
        {
            named.extend(
                specifiers
                    .iter()
                    .map(|specifier| specifier.local_name.as_str())
                    .filter(|name| values.get_own_shared(name).is_none()),
            );
        }
    }
    if named.is_empty() {
        return values;
    }
    let mut nested = Vec::new();
    for statement in &parsed_file.statements {
        match statement {
            ParsedStatement::Block(body) => nested_vars(body, &named, &mut nested),
            ParsedStatement::If(if_statement) => {
                nested_vars(&if_statement.then_body, &named, &mut nested);
                nested_vars(&if_statement.else_body, &named, &mut nested);
            }
            _ => {}
        }
    }
    if nested.is_empty() {
        return values;
    }
    let typed = collect_exportable_value_symbols(
        &nested,
        local_type_declarations,
        local_symbols,
        Some(imported_symbols),
        parsed_file.is_module,
        ctx,
    );
    for name in named {
        if values.get_own_shared(name).is_none()
            && let Some(symbol) = typed.get_own_shared(name)
        {
            let _ = values.insert_shared(Arc::from(name), symbol);
        }
    }
    values
}

fn nested_vars(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    named: &[&str],
    out: &mut Vec<ParsedStatement>,
) {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    for statement in body {
        match statement {
            Statement::VariableDeclaration(variable)
                if variable.kind == surge_ts_syntax::ParsedVariableKind::Var
                    && named.contains(&variable.name.as_str()) =>
            {
                out.push(ParsedStatement::VariableDeclaration(variable.clone()));
            }
            Statement::Block(block) => nested_vars(block, named, out),
            Statement::If(if_statement) => {
                nested_vars(&if_statement.then_body, named, out);
                nested_vars(&if_statement.else_body, named, out);
            }
            Statement::While(while_statement) => nested_vars(&while_statement.body, named, out),
            Statement::ForOf(for_of_statement) => {
                // A `for (var k in o)` / `for (var v of xs)` head is a hoisted
                // `var` too: a key is a `string`, an element is left unmodelled.
                if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
                    && let surge_ts_syntax::ParsedBindingName::Identifier { name, span } =
                        &for_of_statement.binding_name
                    && named.contains(&name.as_str())
                {
                    out.push(ParsedStatement::VariableDeclaration(Box::new(
                        surge_ts_syntax::ParsedVariableDeclaration {
                            is_declare: false,
                            kind: surge_ts_syntax::ParsedVariableKind::Var,
                            from_binding_pattern: false,
                            has_definite_assertion: false,
                            array_pattern_span: None,
                            is_enum_object: false,
                            enum_members: None,
                            array_rest_start: None,
                            name: name.clone(),
                            name_span: *span,
                            declared_type: Some(if for_of_statement.keys_only {
                                surge_ts_syntax::ParsedType::String
                            } else {
                                surge_ts_syntax::ParsedType::Unknown
                            }),
                            initializer: None,
                            initializer_span: None,
                            declaration_list: None,
                            annotated_pattern: None,
                            pattern_excess_properties: Vec::new(),
                        },
                    )));
                }
                nested_vars(&for_of_statement.body, named, out)
            }
            Statement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    nested_vars(&case.consequent, named, out);
                }
            }
            Statement::Try(try_statement) => {
                nested_vars(&try_statement.block, named, out);
                if let Some(handler) = &try_statement.handler {
                    nested_vars(&handler.body, named, out);
                }
                nested_vars(&try_statement.finalizer, named, out);
            }
            _ => {}
        }
    }
}

pub(crate) fn build_module_export_table(
    parsed_file: &ParsedProgramFile,
    local_type_declarations: &TypeDeclarationTable,
    local_symbols: &SymbolTable,
    imported_symbols: &SymbolTable,
    resolution_scope: Option<Arc<TypeDeclarationScope>>,
    ctx: &mut CheckerContext,
) -> ModuleExportTable {
    if let Some(json_module_type) = &parsed_file.json_module_type {
        return build_json_module_export_table(json_module_type, ctx);
    }

    let split_start = crate::program::binding::analyze_split_enabled()
        .then(std::time::Instant::now);
        let exportable_values = collect_exportable_value_symbols(
        &parsed_file.statements,
        local_type_declarations,
        local_symbols,
        Some(imported_symbols),
        parsed_file.is_module,
        ctx,
    );
    let exportable_values = with_exported_nested_vars(
        exportable_values,
        parsed_file,
        local_type_declarations,
        local_symbols,
        imported_symbols,
        ctx,
    );
    crate::program::binding::analyze_split_record(3, split_start);
    let split_start = crate::program::binding::analyze_split_enabled()
        .then(std::time::Instant::now);

    let mut type_declarations = TypeDeclarationTable::new();
    let mut symbols = SymbolTable::new();
    let mut default_symbol = None;
    let mut export_assignment_symbol = None;
    let mut type_only_exports = surge_ts_types::fx::FxHashMap::default();
    let imported_names = crate::program::import_bound_names(&parsed_file.statements);
    for statement in &parsed_file.statements {
        collect_exports_from_statement(
            statement,
            &exportable_values,
            imported_symbols,
            &imported_names,
            &parsed_file.statements,
            &parsed_file.parenthesized_expressions,
            local_type_declarations,
            local_symbols,
            resolution_scope.as_ref(),
            &mut type_declarations,
            &mut symbols,
            &mut default_symbol,
            &mut export_assignment_symbol,
            &mut type_only_exports,
            ctx,
        );
    }

    crate::program::binding::analyze_split_record(4, split_start);
    ModuleExportTable {
        type_declarations: Arc::new(type_declarations),
        symbols,
        default_symbol,
        export_assignment_symbol,
        writes_export_assignment: parsed_file.statements.iter().any(is_export_assignment),
        export_assignment_names_module: false,
        namespace_export_object_type: None,
        has_unresolved_star_export: false,
        has_incomplete_declaration_surface: module_has_incomplete_declaration_surface(parsed_file),
        shorthand: false,
        type_only_exports: Arc::new(type_only_exports),
    }
}

fn is_export_assignment(statement: &ParsedStatement) -> bool {
    matches!(
        statement,
        ParsedStatement::ExportDeclaration(export)
            if matches!(
                export.as_ref(),
                ParsedExportDeclaration::Equals { .. } | ParsedExportDeclaration::EqualsExpression { .. }
            )
    )
}

/// The export surface of a `.json` module: the value itself as the default
/// export and as the namespace object, plus one named export per top-level
/// property — which is what tsc gives an object-valued JSON file. A JSON module
/// exports no types.
///
/// A non-object value (a top-level array or scalar) has no named exports here.
/// tsc resolves a named import against the value's own members in that case
/// (`import { length } from "./list.json"`), which surge does not model.
fn build_json_module_export_table(
    json_module_type: &ParsedType,
    ctx: &mut CheckerContext,
) -> ModuleExportTable {
    let value_type = crate::infer::map_parsed_type(json_module_type.clone(), ctx);
    let value_symbol = |ty: Type| SymbolInfo {
        ty,
        kind: SymbolKind::Const,
        function_signature: None,
    };

    let mut symbols = SymbolTable::new();
    if let Type::Object(object) = &value_type {
        for (name, property) in object.properties.iter() {
            let _ = symbols.insert(name.as_ref(), value_symbol(property.ty.clone()));
        }
    }

    ModuleExportTable {
        type_declarations: Arc::new(TypeDeclarationTable::new()),
        symbols,
        default_symbol: Some(Arc::new(value_symbol(value_type.clone()))),
        export_assignment_symbol: Some(Arc::new(value_symbol(value_type.clone()))),
        writes_export_assignment: true,
        export_assignment_names_module: false,
        has_unresolved_star_export: false,
        namespace_export_object_type: Some(value_type),
        has_incomplete_declaration_surface: false,
        shorthand: false,
        type_only_exports: Default::default(),
    }
}

/// Stores a re-exported value under its exported name. `default` lives in its
/// own slot (`ModuleExportTable::get_shared_value` reads only that slot for the
/// name), so `export { default } from './m'` must land there rather than in
/// `symbols` where no consumer would ever find it.
fn republish_value_export(
    resolved_export_table: &mut ModuleExportTable,
    exported_name: &str,
    value_export: Arc<SymbolInfo>,
) {
    if exported_name == "default" {
        if resolved_export_table.default_symbol.is_none() {
            resolved_export_table.default_symbol = Some(value_export);
        }
        return;
    }

    if resolved_export_table.symbols.get(exported_name).is_none() {
        let _ = resolved_export_table
            .symbols
            .insert_shared(exported_name.to_string(), value_export);
    }
}

/// What an `export = <local>` aliases when `<local>` is an import binding rather
/// than a declaration of this module, which the local pass binds nothing for.
/// `@types/node` builds its `node:*` modules this way: `import path =
/// require("path"); export = path;`, and `import { strict } from "node:assert";
/// export = strict;` for `node:assert/strict`.
enum ExportEqualsImportAlias {
    /// `import <local> = require("<specifier>")`: the whole module.
    Module(String),
    /// `import { <imported> as <local> } from "<specifier>"`: that one export,
    /// as `resolveAlias` takes an import specifier to its target.
    Member { specifier: String, imported_name: String },
}

fn export_equals_import_alias(parsed_file: &ParsedProgramFile) -> Option<ExportEqualsImportAlias> {
    let exported_name = parsed_file.statements.iter().find_map(|statement| {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            return None;
        };
        match export.as_ref() {
            ParsedExportDeclaration::Equals { exported_name, .. } => Some(exported_name.as_str()),
            _ => None,
        }
    })?;

    parsed_file.statements.iter().find_map(|statement| {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            return None;
        };
        match &import.kind {
            ParsedImportKind::Equals { local_name, .. } if local_name == exported_name => {
                Some(ExportEqualsImportAlias::Module(import.module_specifier.clone()))
            }
            ParsedImportKind::Named { is_type_only: false, specifiers }
            | ParsedImportKind::DefaultAndNamed { is_type_only: false, specifiers, .. } => specifiers
                .iter()
                .find(|specifier| specifier.local_name == exported_name)
                .map(|specifier| ExportEqualsImportAlias::Member {
                    specifier: import.module_specifier.clone(),
                    imported_name: specifier.imported_name.clone(),
                }),
            _ => None,
        }
    })
}

/// Adopts the aliased module's whole export surface. Every slot is filled only
/// when empty, so anything this module declared itself still wins.
fn adopt_export_assignment_alias(
    resolved_export_table: &mut ModuleExportTable,
    target_export_table: &ModuleExportTable,
) {
    if resolved_export_table.export_assignment_symbol.is_none() {
        resolved_export_table.export_assignment_symbol =
            target_export_table.export_assignment_symbol.clone();
        resolved_export_table.export_assignment_names_module = target_export_table
            .export_assignment_symbol
            .is_none()
            && (!target_export_table.writes_export_assignment
                || target_export_table.export_assignment_names_module);
    }

    if resolved_export_table.default_symbol.is_none() {
        resolved_export_table.default_symbol = target_export_table.default_symbol.clone();
    }

    // Shares the target's payload handles: an `export =` namespace the size of
    // `typescript.d.ts` is adopted by every importer, and deep-cloning each
    // declaration into a first-wins insert was a clone-and-drop per entry.
    let type_declarations = Arc::make_mut(&mut resolved_export_table.type_declarations);
    for (name, _) in target_export_table.type_declarations.iter() {
        let _ = type_declarations.insert_shared_from(
            name.as_ref(),
            &target_export_table.type_declarations,
            name.as_ref(),
        );
    }

    for (name, symbol) in target_export_table.symbols.iter_shared() {
        if resolved_export_table.symbols.get(name).is_none() {
            resolved_export_table
                .symbols
                .insert_shared(name.clone(), symbol.clone());
        }
    }
}

#[derive(Clone, Copy)]
struct ExportedDeclaration {
    meaning: u8,
    name_span: Option<TextSpan>,
    counts_for_redeclare: bool,
    is_block_scoped_variable: bool,
    /// A JavaScript module's `@typedef`/`@callback`, which the binder declares
    /// straight into the module's exports: `export { T }` resolves to that
    /// export itself rather than conflicting with it.
    is_jsdoc_alias: bool,
}

/// Two `export { … }` specifiers publishing one name are duplicate exported
/// identifiers. tsc reports every one of them, on the exported name rather than
/// on the specifier, and words it as a block-scoped redeclaration when the name
/// is also an exported `let`/`const` — the merged symbol is then the variable's.
fn report_duplicate_export_specifiers(
    specifiers_by_exported_name: &HashMap<&str, Vec<&ParsedExportSpecifier>>,
    exported_declarations: &HashMap<&str, ExportedDeclaration>,
    ctx: &mut CheckerContext,
) {
    // tsc's binder words a second `default` as TS2528, which it reports itself.
    let mut duplicated: Vec<&&str> = specifiers_by_exported_name
        .keys()
        .filter(|name| **name != "default" && specifiers_by_exported_name[**name].len() > 1)
        .collect();
    duplicated.sort_unstable();
    for exported_name in duplicated {
        let specifiers = &specifiers_by_exported_name[*exported_name];
        let declaration = exported_declarations.get(*exported_name);
        let block_scoped = declaration.is_some_and(|declaration| declaration.is_block_scoped_variable);
        let spans = declaration
            .and_then(|declaration| declaration.name_span)
            .into_iter()
            .chain(specifiers.iter().filter_map(|specifier| specifier.exported_name_span));
        for span in spans {
            let diagnostic = if block_scoped {
                Diagnostic::ts2451(*exported_name, ctx.file_name.clone())
            } else {
                Diagnostic::ts2300(*exported_name, ctx.file_name.clone())
            };
            ctx.push(diagnostic.with_span(convert_span(span)));
        }
    }
}

/// Whether a declaration of this kind counts toward tsc's exported-declaration
/// tally (`checkExternalModuleExports`). Interfaces, type aliases, namespaces
/// and enums legally merge with another declaration of the same exported name,
/// so only a variable, function or class makes a second export a redeclaration.
fn export_declaration_counts_for_redeclare(declaration: &ParsedStatement) -> bool {
    matches!(
        declaration,
        ParsedStatement::VariableDeclaration(_)
            | ParsedStatement::FunctionDeclaration(_)
            | ParsedStatement::ClassDeclaration(_)
    )
}

fn exported_declaration_name_span(declaration: &ParsedStatement) -> Option<TextSpan> {
    match declaration {
        ParsedStatement::VariableDeclaration(variable) => variable.name_span,
        ParsedStatement::FunctionDeclaration(function) => function.name_span,
        ParsedStatement::ClassDeclaration(class) => class.name_span,
        ParsedStatement::InterfaceDeclaration(interface) => interface.name_span,
        ParsedStatement::TypeAliasDeclaration(alias) => alias.name_span,
        ParsedStatement::NamespaceDeclaration(namespace) => namespace.name_span,
        _ => None,
    }
}

/// An exported name carried by both a local `export`ed declaration and an
/// `export { … }` specifier is two declarations of one export. tsc reports the
/// pair on every declaration as TS2323 unless the local one legally merges
/// (`checkExternalModuleExports`), and separately reports the specifier as
/// TS2484 when its target overlaps the local declaration in meaning
/// (`checkAliasSymbol` with an export specifier).
/// `checkAliasSymbol`: a JavaScript `export { T }` of a name that is only a
/// type — the file's `@typedef` or `@callback` — is TS18043 at the local name.
fn report_type_export_specifiers_in_javascript(parsed_file: &ParsedProgramFile, ctx: &mut CheckerContext) {
    fn declared(statement: &ParsedStatement) -> Option<&ParsedStatement> {
        match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => Some(declaration.as_ref()),
                _ => None,
            },
            other => Some(other),
        }
    }
    let mut meanings: HashMap<&str, u8> = HashMap::new();
    let mut aliases: Vec<&str> = Vec::new();
    for declaration in parsed_file.statements.iter().filter_map(declared) {
        if let ParsedStatement::TypeAliasDeclaration(alias) = declaration {
            aliases.push(alias.name.as_str());
        }
        for (name, meaning) in local_declaration_meanings(std::slice::from_ref(declaration)) {
            *meanings.entry(name).or_insert(0) |= meaning;
        }
    }
    if aliases.is_empty() {
        return;
    }
    let Some(descriptor) = surge_ts_diagnostics::emitted_descriptor_for_number_with_arity(18043, 0) else {
        return;
    };
    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::Named { is_type_only: false, specifiers, module_specifier: None, .. } =
            export.as_ref()
        else {
            continue;
        };
        for specifier in specifiers {
            let local = specifier.local_name.as_str();
            if specifier.is_type_only
                || !aliases.contains(&local)
                || meanings.get(local).is_some_and(|meaning| meaning & MEANING_VALUE != 0)
            {
                continue;
            }
            let diagnostic = Diagnostic::from_descriptor(descriptor, Vec::new(), ctx.file_name.clone());
            ctx.push(match specifier.name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
    }
}

fn report_duplicate_export_declarations(
    parsed_file: &ParsedProgramFile,
    parsed_files: &[ParsedProgramFile],
    local_module_export_tables: &[Option<ModuleExportTable>],
    resolved_module_export_tables: &mut [Option<ModuleExportTable>],
    resolving: &mut [bool],
    ctx: &mut CheckerContext,
) {
    let javascript = surge_ts_syntax::is_javascript_file_name(&parsed_file.file_name);
    if javascript {
        report_type_export_specifiers_in_javascript(parsed_file, ctx);
    }
    let mut exported_declarations: HashMap<&str, ExportedDeclaration> = HashMap::new();
    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
            continue;
        };
        let meanings = local_declaration_meanings(std::slice::from_ref(declaration.as_ref()));
        let Some((name, meaning)) = meanings.into_iter().next() else {
            continue;
        };
        exported_declarations.insert(
            name,
            ExportedDeclaration {
                meaning,
                name_span: exported_declaration_name_span(declaration.as_ref()),
                counts_for_redeclare: export_declaration_counts_for_redeclare(declaration.as_ref()),
                is_block_scoped_variable: matches!(
                    declaration.as_ref(),
                    ParsedStatement::VariableDeclaration(variable)
                        if !matches!(variable.kind, ParsedVariableKind::Var)
                ),
                is_jsdoc_alias: javascript && matches!(declaration.as_ref(), ParsedStatement::TypeAliasDeclaration(_)),
            },
        );
    }

    let mut specifiers_by_exported_name: HashMap<&str, Vec<&ParsedExportSpecifier>> =
        HashMap::new();
    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::Named { specifiers, .. } = export.as_ref() else {
            continue;
        };
        for specifier in specifiers {
            specifiers_by_exported_name
                .entry(specifier.exported_name.as_str())
                .or_default()
                .push(specifier);
        }
    }

    report_duplicate_export_specifiers(&specifiers_by_exported_name, &exported_declarations, ctx);

    if exported_declarations.is_empty() {
        return;
    }

    // Go's binder gives an exported declaration's local symbol only
    // `ExportValue` (`declareModuleMember`), which no name lookup matches, so a
    // local `export { A }` resolves to what the file declares as `A` without
    // `export`, and only when there is none to the module's export `A` itself.
    let mut unexported_meanings = HashMap::new();
    let mut exported_meanings = HashMap::new();
    for statement in &parsed_file.statements {
        let exported = matches!(
            statement,
            ParsedStatement::ExportDeclaration(export)
                if matches!(export.as_ref(), ParsedExportDeclaration::Statement { .. })
        );
        let meanings = if exported {
            &mut exported_meanings
        } else {
            &mut unexported_meanings
        };
        for (name, meaning) in local_declaration_meanings(std::slice::from_ref(statement)) {
            *meanings.entry(name).or_insert(0) |= meaning;
        }
    }
    let local_meaning = |name: &str| {
        unexported_meanings
            .get(name)
            .copied()
            .filter(|meaning| *meaning != 0)
            .or_else(|| exported_meanings.get(name).copied())
            .unwrap_or(0)
    };

    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::Named {
            specifiers,
            module_specifier,
            resolution_mode,
            ..
        } = export.as_ref()
        else {
            continue;
        };
        if !specifiers
            .iter()
            .any(|specifier| exported_declarations.contains_key(specifier.exported_name.as_str()))
        {
            continue;
        }

        let target_export_table = match module_specifier {
            Some(module_specifier) => {
                let resolved = try_resolve_module_export_table_in_mode(
                    module_specifier,
                    ParsedResolutionModeAttribute::resolution_override(*resolution_mode),
                    ctx,
                    parsed_files,
                    local_module_export_tables,
                    resolved_module_export_tables,
                    resolving,
                    &parsed_file.file_name,
                );
                ctx.set_file_name(parsed_file.file_name.clone());
                let Some((target_export_table, _)) = resolved else {
                    continue;
                };
                Some(target_export_table)
            }
            None => None,
        };

        for specifier in specifiers {
            let Some(declaration) = exported_declarations.get(specifier.exported_name.as_str())
            else {
                continue;
            };
            // A second specifier exporting the same name is a fresh symbol in
            // tsc's binder, not another declaration of this export, so only the
            // first one merges with the declaration and is reported here.
            if specifiers_by_exported_name
                .get(specifier.exported_name.as_str())
                .and_then(|specifiers| specifiers.first())
                .is_none_or(|first| !std::ptr::eq(*first, specifier))
            {
                continue;
            }
            let ExportedDeclaration {
                meaning: declaration_meaning,
                name_span: declaration_span,
                counts_for_redeclare,
                is_jsdoc_alias,
                ..
            } = *declaration;
            if is_jsdoc_alias
                && target_export_table.is_none()
                && unexported_meanings.get(specifier.local_name.as_str()).copied().unwrap_or(0) == 0
            {
                continue;
            }

            let target_meaning = match &target_export_table {
                Some(target_export_table) => {
                    let mut meaning = 0;
                    if lookup_type_export(target_export_table, &specifier.local_name).is_some() {
                        meaning |= MEANING_TYPE;
                    }
                    if lookup_value_export(target_export_table, &specifier.local_name).is_some() {
                        meaning |= MEANING_VALUE;
                    }
                    meaning
                }
                None => local_meaning(specifier.local_name.as_str()),
            };

            if counts_for_redeclare {
                for span in [declaration_span, specifier.name_span] {
                    let diagnostic =
                        Diagnostic::ts2323(&specifier.exported_name, ctx.file_name.clone());
                    let diagnostic = match span {
                        Some(span) => diagnostic.with_span(convert_span(span)),
                        None => diagnostic,
                    };
                    ctx.push(diagnostic);
                }
            }

            if declaration_meaning & target_meaning != 0 {
                let diagnostic =
                    Diagnostic::ts2484(&specifier.exported_name, ctx.file_name.clone());
                let diagnostic = match specifier.name_span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                };
                ctx.push(diagnostic);
            }
        }
    }
}

pub(crate) fn resolve_module_export_tables(
    parsed_files: &[ParsedProgramFile],
    local_module_export_tables: &[Option<ModuleExportTable>],
    ctx: &mut CheckerContext,
) -> Vec<Option<ModuleExportTable>> {
    let mut resolved_module_export_tables = vec![None; parsed_files.len()];
    let mut resolving = vec![false; parsed_files.len()];

    for file_index in 0..parsed_files.len() {
        let _ = resolve_module_export_table(
            file_index,
            parsed_files,
            local_module_export_tables,
            &mut resolved_module_export_tables,
            &mut resolving,
            ctx,
        );
    }

    resolved_module_export_tables
}

pub(crate) fn try_resolve_module_export_table(
    module_specifier: &str,
    ctx: &mut CheckerContext,
    parsed_files: &[ParsedProgramFile],
    local_module_export_tables: &[Option<ModuleExportTable>],
    resolved_module_export_tables: &mut [Option<ModuleExportTable>],
    resolving: &mut [bool],
    file_name: &str,
) -> Option<(ModuleExportTable, Option<usize>)> {
    try_resolve_module_export_table_in_mode(
        module_specifier,
        None,
        ctx,
        parsed_files,
        local_module_export_tables,
        resolved_module_export_tables,
        resolving,
        file_name,
    )
}

/// [`try_resolve_module_export_table`] for a re-export whose
/// `resolution-mode` attribute picks the mode its specifier resolves in.
pub(crate) fn try_resolve_module_export_table_in_mode(
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
    ctx: &mut CheckerContext,
    parsed_files: &[ParsedProgramFile],
    local_module_export_tables: &[Option<ModuleExportTable>],
    resolved_module_export_tables: &mut [Option<ModuleExportTable>],
    resolving: &mut [bool],
    file_name: &str,
) -> Option<(ModuleExportTable, Option<usize>)> {
    let resolution_start = Instant::now();
    if let Some(resolved_file_name) =
        resolved_module_in_mode(ctx, file_name, module_specifier, resolution_mode)
    {
        let resolved_file_name = canonical_file_identity(resolved_file_name);
        if let Some(resolved_index) = ctx
            .module_file_index_by_identity
            .get(resolved_file_name.as_str())
            .copied()
        {
            if !crate::modules::imports::resolved_file_yields_to_ambient_module(
                ctx,
                parsed_files,
                resolved_index,
                module_specifier,
            ) && let Some(export_table) = resolve_module_export_table(
                resolved_index,
                parsed_files,
                local_module_export_tables,
                resolved_module_export_tables,
                resolving,
                ctx,
            ) {
                record_program_timing(ctx.timings.as_ref(), |timings| {
                    timings.export_table_lookup += resolution_start.elapsed();
                    timings.package_export_lookup += resolution_start.elapsed();
                });
                let mut export_table = export_table;
                crate::program::apply_file_keyed_module_augmentation(
                    &mut export_table,
                    resolved_file_name.as_str(),
                    ctx,
                );
                return Some((export_table, Some(resolved_index)));
            }
        }
    }

    if let Some(export_table) =
        crate::modules::imports::ambient_module_export_table(ctx, module_specifier)
    {
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.package_export_lookup += resolution_start.elapsed();
        });
        return Some((export_table.clone(), None));
    }

    if let Some(resolved) = resolve_relative_module(
        file_name,
        module_specifier,
        parsed_files,
        &ctx.module_file_index_by_identity,
    ) {
        if let Some(export_table) = resolve_module_export_table(
            resolved.resolved_file_index,
            parsed_files,
            local_module_export_tables,
            resolved_module_export_tables,
            resolving,
            ctx,
        ) {
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.export_table_lookup += resolution_start.elapsed();
            });
            let mut export_table = export_table;
            crate::program::apply_file_keyed_module_augmentation(
                &mut export_table,
                &canonical_file_identity(&resolved.resolved_file_name),
                ctx,
            );
            return Some((export_table, Some(resolved.resolved_file_index)));
        }
    }

    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.import_specifier_resolution += resolution_start.elapsed()
    });
    None
}

/// `hasExportAssignmentSymbol`: the module's exports are its `export =`.
fn exports_through_assignment(table: &ModuleExportTable) -> bool {
    table.export_assignment_symbol.is_some()
        || table
            .type_declarations
            .iter()
            .any(|(name, _)| crate::modules::is_export_assignment_key(name))
}

fn reexports_name_from_module(statements: &[ParsedStatement], name: &str) -> bool {
    statements.iter().any(|statement| match statement {
        ParsedStatement::ExportDeclaration(export) => matches!(
            export.as_ref(),
            ParsedExportDeclaration::Named { specifiers, module_specifier: Some(_), .. }
                if specifiers.iter().any(|specifier| specifier.exported_name == name)
        ),
        _ => false,
    })
}

pub(crate) fn resolve_module_export_table(
    file_index: usize,
    parsed_files: &[ParsedProgramFile],
    local_module_export_tables: &[Option<ModuleExportTable>],
    resolved_module_export_tables: &mut [Option<ModuleExportTable>],
    resolving: &mut [bool],
    ctx: &mut CheckerContext,
) -> Option<ModuleExportTable> {
    // `file_index` may originate from a different vector domain than the slices
    // passed here. The relative/package resolvers and `module_file_index_by_identity`
    // index the full project file list, but some callers (ambient module binding)
    // pass a narrow local vector. A resolved index from the global domain can then
    // exceed these slices, so every access is bounds-checked and an out-of-domain
    // index yields a conservative unresolved result instead of panicking.
    if let Some(Some(resolved)) = resolved_module_export_tables.get(file_index) {
        return Some(resolved.clone());
    }

    let Some(parsed_file) = parsed_files.get(file_index) else {
        return None;
    };

    let Some(local_export_table) = local_module_export_tables
        .get(file_index)
        .and_then(|slot| slot.clone())
    else {
        return None;
    };

    match resolving.get(file_index) {
        Some(true) => return Some(local_export_table),
        Some(false) => {}
        None => return None,
    }

    if let Some(slot) = resolving.get_mut(file_index) {
        *slot = true;
    }
    ctx.set_file_name(parsed_file.file_name.clone());

    let mut resolved_export_table = local_export_table;

    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        match export.as_ref() {
            ParsedExportDeclaration::Named {
                is_type_only,
                specifiers,
                module_specifier: Some(module_specifier),
                module_specifier_span,
                resolution_mode,
                ..
            } => {
                report_synchronous_import_of_esm(
                    module_specifier,
                    *module_specifier_span,
                    ModuleImportSyntax::Other,
                    ParsedResolutionModeAttribute::resolution_override(*resolution_mode),
                    resolution_mode.is_some(),
                    parsed_files,
                    ctx,
                );
                let Some((target_export_table, resolved_index)) =
                    try_resolve_module_export_table_in_mode(
                        module_specifier,
                        ParsedResolutionModeAttribute::resolution_override(*resolution_mode),
                        ctx,
                        parsed_files,
                        local_module_export_tables,
                        resolved_module_export_tables,
                        resolving,
                        &parsed_file.file_name,
                    )
                else {
                    if resolve_relative_module(
                        &parsed_file.file_name,
                        module_specifier,
                        parsed_files,
                        &ctx.module_file_index_by_identity,
                    )
                    .is_none()
                    {
                        record_unresolved_external_module(ctx, module_specifier);
                        if !(ctx.options.stub_external_modules
                            && is_external_specifier(module_specifier))
                        {
                            emit_unresolved_export_module_diagnostic(
                                ctx,
                                module_specifier,
                                *module_specifier_span,
                            );
                        }
                    }

                    for specifier in specifiers {
                        let specifier_is_type_only = *is_type_only || specifier.is_type_only;
                        insert_unknown_type_import(
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                            &specifier.exported_name,
                            ctx.file_name_arc(),
                            specifier.name_span,
                        );

                        if !specifier_is_type_only {
                            insert_unknown_value_import(
                                &specifier.exported_name,
                                &mut resolved_export_table.symbols,
                            );
                        }
                    }

                    continue;
                };

                ctx.set_file_name(parsed_file.file_name.clone());

                // A shorthand ambient module's every member is the module
                // itself: `any`, with no type of its own.
                if target_export_table.shorthand {
                    for specifier in specifiers {
                        insert_error_type_import(
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                            &specifier.exported_name,
                            ctx.file_name_arc(),
                            specifier.name_span,
                        );
                        if !(*is_type_only || specifier.is_type_only) {
                            insert_value_import(
                                &specifier.exported_name,
                                Type::Any,
                                &mut resolved_export_table.symbols,
                            );
                        }
                    }
                    continue;
                }

                // The names the target publishes qualified `NS.Member` keys
                // under: a namespace, which the alias re-exports beside any
                // type or value of the same name.
                let mut namespace_heads: Option<surge_ts_types::fx::FxHashSet<Arc<str>>> = None;
                let mut is_namespace = |name: &str| {
                    namespace_heads
                        .get_or_insert_with(|| {
                            target_export_table
                                .type_declarations
                                .iter()
                                .filter_map(|(key, _)| {
                                    key.split_once('.').map(|(head, _)| head.into())
                                })
                                .collect()
                        })
                        .contains(name)
                };
                for specifier in specifiers {
                    let specifier_is_type_only = *is_type_only || specifier.is_type_only;
                    // `getTargetOfExportSpecifier` sends `default` through
                    // `getTargetOfModuleDefault`, where a synthetic default wins.
                    if specifier.local_name == "default"
                        && can_have_synthetic_default(
                            ctx,
                            resolved_index.and_then(|index| parsed_files.get(index)),
                            &target_export_table,
                        )
                    {
                        let value = external_module_value(
                            &target_export_table,
                            resolved_index,
                            module_specifier,
                            parsed_files,
                            ctx,
                        );
                        republish_value_export(
                            &mut resolved_export_table,
                            &specifier.exported_name,
                            value,
                        );
                        if specifier_is_type_only {
                            resolved_export_table.mark_type_only_export(
                                &specifier.exported_name,
                                TypeOnlyAliasKind::Export,
                            );
                        }
                        if let Some(type_export) =
                            lookup_type_export(&target_export_table, EXPORT_ASSIGNMENT_NAME)
                        {
                            export_local_type_declaration(
                                type_export,
                                &specifier.exported_name,
                                None,
                                Arc::make_mut(&mut resolved_export_table.type_declarations),
                            );
                        }
                        continue;
                    }
                    let type_export =
                        lookup_type_export(&target_export_table, &specifier.local_name);
                    let value_export =
                        lookup_value_export(&target_export_table, &specifier.local_name);
                    let target_type_only_kind = target_export_table
                        .type_only_exports
                        .get(specifier.local_name.as_str())
                        .copied();

                    if specifier_is_type_only {
                        // `export type` re-exports every meaning of the name
                        // (tsc's alias resolves them all); it only keeps an
                        // importer from using the value.
                        let republishes_value = value_export.is_some();
                        if let Some(value_export) = value_export {
                            republish_value_export(
                                &mut resolved_export_table,
                                &specifier.exported_name,
                                value_export,
                            );
                            resolved_export_table.mark_type_only_export(
                                &specifier.exported_name,
                                TypeOnlyAliasKind::Export,
                            );
                        }

                        if let Some(type_export) = type_export {
                            export_local_type_declaration(
                                type_export,
                                &specifier.exported_name,
                                None,
                                Arc::make_mut(&mut resolved_export_table.type_declarations),
                            );
                            if is_namespace(&specifier.local_name) {
                                copy_qualified_type_exports(
                                    &target_export_table,
                                    &specifier.local_name,
                                    &specifier.exported_name,
                                    Arc::make_mut(&mut resolved_export_table.type_declarations),
                                );
                            }
                            continue;
                        }

                        // A type-only namespace publishes only qualified
                        // `NS.Member` keys, so the direct lookup misses; the
                        // import path already compensates the same way. Run the
                        // scan only on that miss, where it belongs.
                        if copy_qualified_type_exports(
                            &target_export_table,
                            &specifier.local_name,
                            &specifier.exported_name,
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                        ) {
                            continue;
                        }

                        // `export type { f } from './m'` over a value-only
                        // export: the name is legal in type position through
                        // `typeof f`.
                        if republishes_value {
                            continue;
                        }

                        // A name the target re-exports while the target is
                        // still being resolved goes round a cycle of aliases:
                        // `resolveAlias` reports TS2303 for it and nothing more.
                        if !resolved_index.is_some_and(|index| {
                            resolving.get(index).copied() == Some(true)
                                && parsed_files.get(index).is_some_and(|target| {
                                    reexports_name_from_module(&target.statements, &specifier.local_name)
                                })
                        }) {
                            emit_missing_export_diagnostic(
                                ctx,
                                module_specifier,
                                &specifier.local_name,
                                specifier.name_span,
                            );
                        }
                        insert_unknown_type_import(
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                            &specifier.exported_name,
                            ctx.file_name_arc(),
                            specifier.name_span,
                        );
                        continue;
                    }

                    let mut found = false;

                    if let Some(type_export) = type_export {
                        export_local_type_declaration(
                            type_export,
                            &specifier.exported_name,
                            None,
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                        );
                        found = true;
                    }

                    if let Some(value_export) = value_export {
                        republish_value_export(
                            &mut resolved_export_table,
                            &specifier.exported_name,
                            value_export,
                        );
                        if let Some(kind) = target_type_only_kind {
                            resolved_export_table
                                .mark_type_only_export(&specifier.exported_name, kind);
                        }
                        found = true;
                    }

                    if (!found || is_namespace(&specifier.local_name))
                        && copy_qualified_type_exports(
                            &target_export_table,
                            &specifier.local_name,
                            &specifier.exported_name,
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                        )
                    {
                        found = true;
                    }

                    if !found
                        && crate::modules::module_declares_exported_namespace(
                            resolved_index,
                            parsed_files,
                            &specifier.local_name,
                        )
                    {
                        continue;
                    }

                    if !found {
                        if target_export_table.has_unresolved_star_export
                            || resolved_index
                                .map(|i| {
                                    module_has_unresolved_star_export(
                                        i,
                                        parsed_files,
                                        &ctx.module_file_index_by_identity,
                                    )
                                })
                                .unwrap_or(false)
                        {
                            insert_unknown_type_import(
                                Arc::make_mut(&mut resolved_export_table.type_declarations),
                                &specifier.exported_name,
                                ctx.file_name_arc(),
                                specifier.name_span,
                            );
                            if !specifier_is_type_only {
                                insert_unknown_value_import(
                                    &specifier.exported_name,
                                    &mut resolved_export_table.symbols,
                                );
                            }
                            continue;
                        }

                        if should_bind_unknown_for_missing_export(
                            &target_export_table,
                            resolved_index,
                            parsed_files,
                        ) {
                            insert_unknown_type_import(
                                Arc::make_mut(&mut resolved_export_table.type_declarations),
                                &specifier.exported_name,
                                ctx.file_name_arc(),
                                specifier.name_span,
                            );
                            if !specifier_is_type_only {
                                insert_unknown_value_import(
                                    &specifier.exported_name,
                                    &mut resolved_export_table.symbols,
                                );
                            }
                            continue;
                        }

                        emit_missing_export_diagnostic(
                            ctx,
                            module_specifier,
                            &specifier.local_name,
                            specifier.name_span,
                        );
                        insert_unknown_type_import(
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                            &specifier.exported_name,
                            ctx.file_name_arc(),
                            specifier.name_span,
                        );
                        if !specifier_is_type_only {
                            insert_unknown_value_import(
                                &specifier.exported_name,
                                &mut resolved_export_table.symbols,
                            );
                        }
                    }
                }
            }
            ParsedExportDeclaration::Namespace {
                exported_name,
                module_specifier,
                module_specifier_span,
                is_type_only,
                ..
            } => {
                if *is_type_only {
                    resolved_export_table.mark_type_only_export(exported_name, TypeOnlyAliasKind::Export);
                }
                report_synchronous_import_of_esm(
                    module_specifier,
                    *module_specifier_span,
                    ModuleImportSyntax::Other,
                    None,
                    false,
                    parsed_files,
                    ctx,
                );
                let Some((target_export_table, resolved_index)) = try_resolve_module_export_table(
                    module_specifier,
                    ctx,
                    parsed_files,
                    local_module_export_tables,
                    resolved_module_export_tables,
                    resolving,
                    &parsed_file.file_name,
                ) else {
                    if resolve_relative_module(
                        &parsed_file.file_name,
                        module_specifier,
                        parsed_files,
                        &ctx.module_file_index_by_identity,
                    )
                    .is_none()
                    {
                        record_unresolved_external_module(ctx, module_specifier);
                        if !(ctx.options.stub_external_modules
                            && is_external_specifier(module_specifier))
                        {
                            emit_unresolved_export_module_diagnostic(
                                ctx,
                                module_specifier,
                                *module_specifier_span,
                            );
                        }
                    }

                    insert_unknown_value_import(exported_name, &mut resolved_export_table.symbols);
                    continue;
                };

                ctx.set_file_name(parsed_file.file_name.clone());
                if exports_through_assignment(&target_export_table) {
                    let diagnostic = Diagnostic::ts2498(format!("\"{module_specifier}\""), ctx.file_name.clone());
                    ctx.push(match module_specifier_span {
                        Some(span) => diagnostic.with_span(crate::context::convert_span(*span)),
                        None => diagnostic,
                    });
                }
                if exported_name == "default" {
                    // `export * as default` is the module's default export
                    // (`isSyntacticDefault` counts a namespace export).
                    if resolved_export_table.default_symbol.is_none() {
                        let namespace_type = namespace_export_object_type(&target_export_table);
                        let namespace_type =
                            match resolved_index.and_then(|index| parsed_files.get(index)) {
                                Some(file) => {
                                    tag_namespace_type_with_module_path(namespace_type, &file.file_name)
                                }
                                None => namespace_type,
                            };
                        resolved_export_table.default_symbol = Some(Arc::new(SymbolInfo {
                            ty: namespace_type,
                            kind: SymbolKind::Const,
                            function_signature: None,
                        }));
                    }
                } else {
                    insert_namespace_export(
                        &mut resolved_export_table.symbols,
                        exported_name,
                        &target_export_table,
                    );
                }
                copy_namespace_member_type_exports(
                    &target_export_table,
                    exported_name,
                    &mut resolved_export_table,
                );
            }
            // `import * as z from "./m"; export { z }` re-exports the namespace
            // binding (zod's `z`). The value symbol is carried by the local
            // export pass; the namespace's TYPE side lives only in the importing
            // file's alias scope layers, so the qualified `z.<member>` keys must
            // be materialized into the export table here for the consumer-side
            // `copy_qualified_type_exports` to find.
            ParsedExportDeclaration::Named {
                specifiers,
                module_specifier: None,
                ..
            } => {
                for specifier in specifiers {
                    let Some((namespace_module_specifier, imported_name)) =
                        reexported_import_source(parsed_file, &specifier.local_name)
                    else {
                        continue;
                    };
                    let Some((target_export_table, _resolved_index)) =
                        try_resolve_module_export_table(
                            &namespace_module_specifier,
                            ctx,
                            parsed_files,
                            local_module_export_tables,
                            resolved_module_export_tables,
                            resolving,
                            &parsed_file.file_name,
                        )
                    else {
                        continue;
                    };
                    ctx.set_file_name(parsed_file.file_name.clone());
                    if let Some(imported_name) = imported_name {
                        copy_qualified_type_exports(
                            &target_export_table,
                            &imported_name,
                            &specifier.exported_name,
                            Arc::make_mut(&mut resolved_export_table.type_declarations),
                        );
                        // The value side was bound from the preliminary table,
                        // where a namespace object carries permissive members;
                        // the target's final symbol is what the consumer reads.
                        if let Some(symbol) = target_export_table
                            .symbols
                            .get_shared(&imported_name)
                            .filter(|symbol| {
                                matches!(&symbol.ty, Type::Object(object)
                                    if object.call_signature().is_none()
                                        && object.construct_signature().is_none())
                            })
                        {
                            resolved_export_table
                                .symbols
                                .insert_shared(specifier.exported_name.clone(), symbol);
                        }
                    } else {
                        copy_namespace_member_type_exports(
                            &target_export_table,
                            &specifier.exported_name,
                            &mut resolved_export_table,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    let re_export_start = Instant::now();
    // `getExportsOfModuleWorker`: a name some `export *` reaches without a
    // type-only star is not made type-only by another star, so the values an
    // `export type *` carries are taken after every other star's.
    let mut type_only_star_targets = Vec::new();
    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::All {
            module_specifier,
            module_specifier_span,
            is_type_only,
            resolution_mode,
            ..
        } = export.as_ref()
        else {
            continue;
        };
        report_synchronous_import_of_esm(
            module_specifier,
            *module_specifier_span,
            ModuleImportSyntax::Other,
            ParsedResolutionModeAttribute::resolution_override(*resolution_mode),
            resolution_mode.is_some(),
            parsed_files,
            ctx,
        );

        let Some((target_export_table, _resolved_index)) = try_resolve_module_export_table_in_mode(
            module_specifier,
            ParsedResolutionModeAttribute::resolution_override(*resolution_mode),
            ctx,
            parsed_files,
            local_module_export_tables,
            resolved_module_export_tables,
            resolving,
            &parsed_file.file_name,
        ) else {
            record_unresolved_external_module(ctx, module_specifier);
            if !(ctx.options.stub_external_modules && is_external_specifier(module_specifier)) {
                emit_unresolved_export_module_diagnostic(
                    ctx,
                    module_specifier,
                    *module_specifier_span,
                );
            }
            resolved_export_table.has_unresolved_star_export = true;
            continue;
        };

        ctx.set_file_name(parsed_file.file_name.clone());

        // `checkExportDeclaration`: a module whose exports are its `export =`
        // has no named exports to re-export.
        if exports_through_assignment(&target_export_table) {
            let diagnostic = Diagnostic::ts2498(
                format!("\"{module_specifier}\""),
                ctx.file_name.clone(),
            );
            ctx.push(match module_specifier_span {
                Some(span) => diagnostic.with_span(crate::context::convert_span(*span)),
                None => diagnostic,
            });
        }

        let resolved_type_declarations =
            Arc::make_mut(&mut resolved_export_table.type_declarations);
        // Payload sharing (`insert_shared_from`) was tried here and reverted:
        // collapsing the re-exported clone into the source payload changes
        // which first-wins expansion later consumers observe (zod message
        // drift). Re-export entries keep their per-table copies.
        for (name, declaration) in target_export_table.type_declarations.iter() {
            if !is_export_assignment_key(name)
                && resolved_type_declarations.get(name.as_ref()).is_none()
            {
                let _ = resolved_type_declarations.insert(name.clone(), declaration.clone());
            }
        }

        if *is_type_only {
            type_only_star_targets.push(target_export_table);
            continue;
        }
        for (name, symbol) in target_export_table.symbols.iter_shared() {
            if resolved_export_table.symbols.get(name).is_none() {
                crate::program::record_module_export_symbol_handle_copy_count(1);
                resolved_export_table
                    .symbols
                    .insert_shared(name.clone(), symbol.clone());
                if let Some(kind) = target_export_table.type_only_exports.get(name.as_ref()) {
                    resolved_export_table.mark_type_only_export(name, *kind);
                }
            }
        }
    }
    for target_export_table in type_only_star_targets {
        for (name, symbol) in target_export_table.symbols.iter_shared() {
            if resolved_export_table.symbols.get(name).is_none() {
                crate::program::record_module_export_symbol_handle_copy_count(1);
                resolved_export_table
                    .symbols
                    .insert_shared(name.clone(), symbol.clone());
                resolved_export_table.mark_type_only_export(name, TypeOnlyAliasKind::Export);
            }
        }
    }
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.re_export_expansion += re_export_start.elapsed()
    });

    if let Some(alias) = export_equals_import_alias(parsed_file) {
        let specifier = match &alias {
            ExportEqualsImportAlias::Module(specifier)
            | ExportEqualsImportAlias::Member { specifier, .. } => specifier,
        };
        if let Some((target_export_table, _resolved_index)) = try_resolve_module_export_table(
            specifier,
            ctx,
            parsed_files,
            local_module_export_tables,
            resolved_module_export_tables,
            resolving,
            &parsed_file.file_name,
        ) {
            ctx.set_file_name(parsed_file.file_name.clone());
            match &alias {
                ExportEqualsImportAlias::Module(_) => {
                    adopt_export_assignment_alias(&mut resolved_export_table, &target_export_table);
                }
                ExportEqualsImportAlias::Member { imported_name, .. } => {
                    if resolved_export_table.export_assignment_symbol.is_none() {
                        resolved_export_table.export_assignment_symbol =
                            lookup_value_export(&target_export_table, imported_name);
                    }
                }
            }
        }
    }

    report_duplicate_export_declarations(
        parsed_file,
        parsed_files,
        local_module_export_tables,
        resolved_module_export_tables,
        resolving,
        ctx,
    );
    ctx.set_file_name(parsed_file.file_name.clone());

    if let Some(slot) = resolving.get_mut(file_index) {
        *slot = false;
    }
    resolved_export_table.namespace_export_object_type =
        Some(compute_namespace_export_object_type(&resolved_export_table));
    if let Some(slot) = resolved_module_export_tables.get_mut(file_index) {
        *slot = Some(resolved_export_table.clone_with_reason(TypeCopyReason::ModuleExport));
    }
    Some(resolved_export_table)
}
