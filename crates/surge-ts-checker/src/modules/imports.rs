//! Resolving `import` declarations into local type and value bindings.

use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedExportDeclaration, ParsedImportDeclaration, ParsedImportKind, ParsedStatement, ParsedType,
    ParsedTypeAliasDeclaration, TextSpan,
};
use surge_ts_types::{Type, TypeCopyReason};

use crate::context::{CheckerContext, convert_span};
use crate::program::{ParsedProgramFile, record_program_timing};
use crate::symbols::{
    SymbolInfo, SymbolKind, SymbolTable, TypeAliasInfo, TypeDeclarationInfo, TypeDeclarationScope,
    TypeDeclarationTable,
};

/// Records that an external (non-relative) module specifier failed every
/// resolution path, for the `externalModuleStubs.unresolved` compatibility-report
/// figure. Relative specifiers are not external references and are not counted.
pub(crate) fn record_unresolved_external_module(ctx: &mut CheckerContext, specifier: &str) {
    if is_external_specifier(specifier) {
        ctx.stats.external_modules_unresolved_total += 1;
    }
}

/// Handles an import whose module specifier resolved to nothing: records it
/// against the unresolved-external figure, then emits the unresolved-module
/// diagnostic unless an external specifier is being intentionally stubbed
/// (`stub_external_modules`).
/// The type an import binding takes when its module did not resolve. tsc binds
/// its error type (`any`) there, so a call written against the binding still has
/// its arguments checked and an untyped callback parameter is still TS7006/
/// TS7031. The `Unknown` sentinel means "surge failed to model this" and
/// suppresses those, which is right only when the diagnostic is suppressed too —
/// `stubExternalModules` deliberately hides TS2307 for external specifiers.
fn insert_unresolved_import_binding(
    local_name: &str,
    ctx: &CheckerContext,
    import: &ParsedImportDeclaration,
    symbols: &mut SymbolTable,
) {
    if ctx.options.stub_external_modules && is_external_specifier(&import.module_specifier) {
        insert_unknown_value_import(local_name, symbols);
    } else {
        insert_error_typed_value_import(local_name, symbols);
    }
}

/// tsc binds an import whose module does not resolve to `unknownSymbol`, whose
/// type meaning is the error type as much as its value meaning is. Inside a
/// declaration file an import surge could not follow stays its own gap.
fn insert_unresolved_import_type(
    local_name: &str,
    name_span: Option<TextSpan>,
    type_declarations: &mut TypeDeclarationTable,
    ctx: &mut CheckerContext,
) {
    if !is_declaration_file_name(&ctx.file_name) && type_declarations.get(local_name).is_none() {
        insert_error_type_import(type_declarations, local_name, ctx.file_name_arc(), name_span);
    }
}

pub(crate) fn report_unresolved_module(ctx: &mut CheckerContext, import: &ParsedImportDeclaration) {
    if import.module_specifier == surge_ts_syntax::NON_LITERAL_MODULE_SPECIFIER {
        return;
    }
    record_unresolved_external_module(ctx, &import.module_specifier);
    if !(ctx.options.stub_external_modules && is_external_specifier(&import.module_specifier)) {
        emit_unresolved_module_diagnostic(ctx, import);
    }
}

/// The program file an import specifier resolves to, through the loader's
/// resolution or the relative fallback.
fn resolved_program_file_index(
    ctx: &CheckerContext,
    module_specifier: &str,
    program_files: &[ParsedProgramFile],
) -> Option<usize> {
    ctx.options
        .resolved_module_for(&ctx.file_name, module_specifier)
        .and_then(|resolved| {
            ctx.module_file_index_by_identity
                .get(canonical_file_identity(resolved).as_str())
                .copied()
        })
        .or_else(|| {
            resolve_relative_module(
                &ctx.file_name,
                module_specifier,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .map(|resolution| resolution.resolved_file_index)
        })
}

/// tsc's `File '{0}' is not a module`: the specifier resolves to a program
/// file that is a script. Reported at the specifier; the import then binds
/// nothing usable, as tsc's alias resolves to no symbol.
fn report_non_module_import(
    ctx: &mut CheckerContext,
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
) -> bool {
    let Some(file) = resolved_program_file_index(ctx, &import.module_specifier, program_files)
        .and_then(|index| program_files.get(index))
    else {
        return false;
    };
    if file.is_module || file.file_kind == crate::context::FileKind::DependencyDeclaration {
        return false;
    }
    let mut diagnostic = Diagnostic::ts2306(&file.file_name, ctx.file_name.clone());
    if let Some(span) = import.module_specifier_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push(diagnostic);
    true
}

/// tsc's `mergeModuleAugmentation`: a `declare module "m"` in a module file
/// augments `m`, which must resolve — TS2664 at the name — to a module or a
/// namespace-like `export =` target — TS2671. Target validity is not checked
/// for declaration-file augmentations. Body grammar requires an applied target.
fn check_module_augmentations(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    if !parsed_file.is_module {
        return;
    }
    for statement in &parsed_file.statements {
        // `export declare module "m"` is the same augmentation, with a TS2668
        // of its own.
        let statement = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                _ => continue,
            },
            statement => statement,
        };
        let ParsedStatement::DeclareModuleDeclaration(module) = statement else {
            continue;
        };
        if module.module_specifier != "global" {
            let applied = ambient_module_export_table(ctx, &module.module_specifier).is_some()
                || resolved_program_file_index(ctx, &module.module_specifier, program_files)
                    .and_then(|index| program_files.get(index))
                    .is_some_and(|target| !export_assignment_targets_non_module_entity(target));
            let mut declarations = TypeDeclarationTable::new();
            let mut symbols = SymbolTable::new();
            let mut layers = Vec::new();
            let mut type_only_aliases = Vec::new();
            let mut namespace_imports = NamespaceImports::default();
            for statement in &module.statements {
                // `checkModuleAugmentationElement`: an export that declares
                // nothing (TS2666).
                if applied && let ParsedStatement::ExportDeclaration(export) = statement {
                    use surge_ts_syntax::ParsedExportDeclaration as Export;
                    let span = match export.as_ref() {
                        // `export default interface I {}` is lowered to a
                        // default specifier beside the declaration.
                        Export::Named { specifiers, module_specifier: None, .. }
                            if specifiers.iter().all(|specifier| specifier.exported_name == "default") =>
                        {
                            None
                        }
                        Export::Named { span, .. }
                        | Export::All { span, .. }
                        | Export::Namespace { span, .. }
                        | Export::Equals { span, .. }
                        | Export::EqualsExpression { span, .. } => Some(*span),
                        _ => None,
                    };
                    if let Some(span) = span {
                        let mut diagnostic = Diagnostic::ts2666(ctx.file_name.clone());
                        if let Some(span) = span {
                            diagnostic = diagnostic.with_span(convert_span(TextSpan {
                                start: span.start,
                                end: span.start + "export".len(),
                            }));
                        }
                        ctx.push(diagnostic);
                    }
                }
                // A re-export names a module the augmentation cannot resolve
                // unless the file itself imports it, as an import there does.
                if let ParsedStatement::ExportDeclaration(export) = statement {
                    use surge_ts_syntax::ParsedExportDeclaration as Export;
                    let reexported = match export.as_ref() {
                        Export::Named { module_specifier: Some(specifier), module_specifier_span, .. }
                        | Export::All { module_specifier: specifier, module_specifier_span, .. } => {
                            Some((specifier, *module_specifier_span))
                        }
                        _ => None,
                    };
                    if let Some((specifier, Some(span))) = reexported
                        && specifier.starts_with('.')
                        && !is_declaration_file_name(&parsed_file.file_name)
                    {
                        let recorded = parsed_file.statements.iter().any(|statement| {
                            matches!(statement, ParsedStatement::ImportDeclaration(outer)
                                if outer.module_specifier == *specifier)
                        }) || resolved_module_in_mode(ctx, &ctx.file_name, specifier, None).is_some();
                        if !recorded {
                            let diagnostic = unresolved_module_diagnostic(ctx, specifier);
                            ctx.push(diagnostic.with_span(convert_span(span)));
                        }
                    }
                }
                if let ParsedStatement::ImportDeclaration(import) = statement {
                    if applied && !matches!(import.kind, ParsedImportKind::EntityAlias { .. }) {
                        let mut diagnostic = Diagnostic::ts2667(ctx.file_name.clone());
                        if let Some(span) = import.span {
                            diagnostic = diagnostic.with_span(convert_span(TextSpan {
                                start: span.start,
                                end: span.start + "import".len(),
                            }));
                        }
                        ctx.push(diagnostic);
                    }
                    let recorded = parsed_file.statements.iter().any(|statement| {
                        matches!(statement, ParsedStatement::ImportDeclaration(outer)
                            if outer.module_specifier == import.module_specifier)
                    }) || resolved_module_in_mode(
                        ctx,
                        &ctx.file_name,
                        &import.module_specifier,
                        import_resolution_mode(import),
                    ).is_some();
                    resolve_import_declaration(
                        import,
                        if recorded { program_files } else { &[] },
                        if recorded { module_export_tables } else { &[] },
                        if recorded { module_resolution_scopes } else { &[] },
                        &|_| false,
                        &mut declarations,
                        &mut symbols,
                        &mut layers,
                        &mut type_only_aliases,
                        &mut namespace_imports,
                        ctx,
                    );
                }
            }
        }
        if is_declaration_file_name(&parsed_file.file_name) {
            continue;
        }
        let specifier = module.module_specifier.as_str();
        if specifier == "global" || ambient_module_export_table(ctx, specifier).is_some() {
            continue;
        }
        if let Some(target) = resolved_program_file_index(ctx, specifier, program_files)
            .and_then(|index| program_files.get(index))
        {
            if export_assignment_targets_non_module_entity(target) {
                let mut diagnostic = Diagnostic::ts2671(specifier, ctx.file_name.clone());
                if let Some(span) = module.module_specifier_span {
                    diagnostic = diagnostic.with_span(convert_span(span));
                }
                ctx.push(diagnostic);
            }
            continue;
        }
        // A target that resolves but is not in the program is still TS2664:
        // tsc never loads a file for an augmentation name alone. An untyped
        // JavaScript target is tsc's TS2665 instead, which surge does not port.
        if crate::driver::is_runtime_js_only_module(specifier, ctx)
            || (ctx.options.stub_external_modules && is_external_specifier(specifier))
        {
            continue;
        }
        let mut diagnostic = Diagnostic::ts2664(specifier, ctx.file_name.clone());
        if let Some(span) = module.module_specifier_span {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push(diagnostic);
    }
}

/// Whether `file`'s `export = name` resolves to a local declaration without
/// tsc's namespace meaning (a namespace or an enum), which an augmentation
/// cannot merge into. A name the file does not declare itself (an import) is
/// not followed.
fn export_assignment_targets_non_module_entity(file: &ParsedProgramFile) -> bool {
    let Some(name) = file.statements.iter().find_map(|statement| match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Equals { exported_name, .. } => Some(exported_name.as_str()),
            _ => None,
        },
        _ => None,
    }) else {
        return false;
    };
    let mut declared = false;
    for statement in &file.statements {
        let statement = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                _ => continue,
            },
            other => other,
        };
        match statement {
            ParsedStatement::NamespaceDeclaration(namespace)
                if namespace.name.split('.').next() == Some(name) =>
            {
                return false;
            }
            ParsedStatement::VariableDeclaration(variable) if variable.name == name => {
                if variable.is_enum_object {
                    return false;
                }
                declared = true;
            }
            ParsedStatement::FunctionDeclaration(function) if function.name == name => declared = true,
            ParsedStatement::ClassDeclaration(class) if class.name == name => declared = true,
            ParsedStatement::InterfaceDeclaration(interface) if interface.name == name => {
                declared = true;
            }
            ParsedStatement::TypeAliasDeclaration(alias) if alias.name == name => declared = true,
            ParsedStatement::UnsupportedDeclaration { .. } => return false,
            _ => {}
        }
    }
    declared
}

/// What the namespace-like import bindings of a file are known to resolve to.
#[derive(Default)]
pub(crate) struct NamespaceImports {
    /// `import * as` / `import x = require` bindings whose module's whole
    /// export list surge saw.
    pub(crate) complete: Vec<Arc<str>>,
    /// `import x = require` bindings of a module whose `export =` names a
    /// declaration without namespace meaning.
    pub(crate) value_only: Vec<Arc<str>>,
}

pub(crate) const MEANING_VALUE: u8 = 1 << 0;
pub(crate) const MEANING_TYPE: u8 = 1 << 1;
pub(crate) const MEANING_NAMESPACE: u8 = 1 << 2;

/// Meanings each top-level name is declared with in this file, mirroring the
/// symbol-flag groups tsc's `checkAliasSymbol` builds its excluded meanings
/// from. Only declarations count: an import binding contributes nothing, so a
/// name that appears here alongside an import of the same name is exactly the
/// collision tsc reports as TS2440.
pub(crate) fn local_declaration_meanings(statements: &[ParsedStatement]) -> HashMap<&str, u8> {
    let mut meanings: HashMap<&str, u8> = HashMap::new();
    collect_local_declaration_meanings(statements, &mut meanings);
    meanings
}

fn collect_local_declaration_meanings<'a>(
    statements: &'a [ParsedStatement],
    meanings: &mut HashMap<&'a str, u8>,
) {
    for statement in statements {
        let (name, meaning) = match statement {
            ParsedStatement::VariableDeclaration(variable) => {
                (variable.name.as_str(), MEANING_VALUE)
            }
            ParsedStatement::FunctionDeclaration(function) => {
                (function.name.as_str(), MEANING_VALUE)
            }
            ParsedStatement::ClassDeclaration(class) => {
                (class.name.as_str(), MEANING_VALUE | MEANING_TYPE)
            }
            ParsedStatement::InterfaceDeclaration(interface) => {
                (interface.name.as_str(), MEANING_TYPE)
            }
            ParsedStatement::TypeAliasDeclaration(alias) => (alias.name.as_str(), MEANING_TYPE),
            ParsedStatement::NamespaceDeclaration(namespace) => {
                let name = match namespace.name.split_once('.') {
                    Some((head, _)) => head,
                    None => namespace.name.as_str(),
                };
                // A namespace of types alone is `NamespaceModule`, no value.
                let meaning = if crate::program::is_instantiated_namespace(namespace) {
                    MEANING_VALUE | MEANING_NAMESPACE
                } else {
                    MEANING_NAMESPACE
                };
                (name, meaning)
            }
            ParsedStatement::ExportDeclaration(export) => {
                if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                    collect_local_declaration_meanings(
                        std::slice::from_ref(declaration.as_ref()),
                        meanings,
                    );
                }
                continue;
            }
            _ => continue,
        };
        *meanings.entry(name).or_insert(0) |= meaning;
    }
}

/// tsc reports an import whose local name is also declared in the file on the
/// import, not on the declaration, and only when the two overlap in meaning
/// (`checkAliasSymbol`: a value import beside a local `type` alias is legal).
/// The declaration owns the name afterwards, so the colliding value binding is
/// dropped — leaving it would make the declaration look like a redeclaration
/// and report TS2451 on top, which tsc never does.
fn report_import_local_declaration_conflicts(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    let local_meanings = local_declaration_meanings(&parsed_file.statements);
    if local_meanings.is_empty() {
        return;
    }

    for statement in &parsed_file.statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };
        if let ParsedImportKind::EntityAlias { local_name, name_span, target } = &import.kind {
            if let Some(&local_meaning) = local_meanings.get(local_name.as_str())
                && aliases_enum_member(target, parsed_file, program_files, module_export_tables, module_resolution_scopes, ctx)
            {
                report_enum_member_alias_conflict(import, local_name, *name_span, local_meaning, parsed_file, ctx);
            }
            continue;
        }
        let bindings: Vec<(&str, &str, Option<TextSpan>)> = match &import.kind {
            ParsedImportKind::Named { specifiers, .. } => specifiers
                .iter()
                .map(|specifier| {
                    (
                        specifier.local_name.as_str(),
                        specifier.imported_name.as_str(),
                        specifier.name_span,
                    )
                })
                .collect(),
            ParsedImportKind::DefaultAndNamed {
                local_name,
                name_span,
                specifiers,
                ..
            } => std::iter::once((local_name.as_str(), "default", *name_span))
                .chain(specifiers.iter().map(|specifier| {
                    (
                        specifier.local_name.as_str(),
                        specifier.imported_name.as_str(),
                        specifier.name_span,
                    )
                }))
                .collect(),
            ParsedImportKind::Default {
                local_name,
                name_span,
            }
            | ParsedImportKind::TypeOnlyDefault {
                local_name,
                name_span,
            } => vec![(local_name.as_str(), "default", *name_span)],
            ParsedImportKind::Namespace {
                local_name,
                name_span,
                ..
            } => vec![(local_name.as_str(), "*", *name_span)],
            ParsedImportKind::Equals { .. }
            | ParsedImportKind::EntityAlias { .. }
            | ParsedImportKind::SideEffect
            | ParsedImportKind::Unsupported => continue,
        };

        let import_is_type_only = match &import.kind {
            ParsedImportKind::Named { is_type_only, .. }
            | ParsedImportKind::DefaultAndNamed { is_type_only, .. }
            | ParsedImportKind::Namespace { is_type_only, .. } => *is_type_only,
            ParsedImportKind::TypeOnlyDefault { .. } => true,
            _ => false,
        };
        let clause_name = match &import.kind {
            ParsedImportKind::DefaultAndNamed { local_name, .. }
            | ParsedImportKind::Default { local_name, .. }
            | ParsedImportKind::TypeOnlyDefault { local_name, .. }
            | ParsedImportKind::Namespace { local_name, .. } => Some(local_name.as_str()),
            _ => None,
        };

        if !bindings
            .iter()
            .any(|(local_name, ..)| local_meanings.contains_key(local_name))
        {
            continue;
        }

        let Some((export_table, _, resolved_index)) = try_resolve_import_module(
            import,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        ) else {
            continue;
        };

        for (local_name, imported_name, name_span) in bindings {
            let Some(local_meaning) = local_meanings.get(local_name).copied() else {
                continue;
            };
            let target_meaning = if imported_name == "*" {
                let mut meaning = MEANING_NAMESPACE;
                // `import * as x` of an `export =` module is the assigned value
                // (`resolveESModuleSymbol`).
                if export_table.default_symbol.is_some()
                    || export_table.export_assignment_symbol.is_some()
                    || export_table.symbols.iter_shared().next().is_some()
                {
                    meaning |= MEANING_VALUE;
                }
                meaning
            } else {
                let mut meaning = 0;
                let type_export = lookup_type_export(&export_table, imported_name);
                if type_export.is_some() {
                    meaning |= MEANING_TYPE;
                }
                if lookup_value_export(&export_table, imported_name).is_some() {
                    meaning |= MEANING_VALUE;
                }
                // Go's `SymbolFlagsNamespace` covers a namespace and an enum.
                let enum_export = matches!(
                    type_export,
                    Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) if alias.enum_name.is_some()
                );
                if enum_export
                    || crate::modules::module_declares_exported_namespace(
                        resolved_index,
                        program_files,
                        imported_name,
                    )
                {
                    meaning |= MEANING_NAMESPACE;
                }
                meaning
            };

            if local_meaning & target_meaning == 0 {
                // `checkAliasSymbol`: an import a single-file transpiler cannot
                // tell is a type collides with the local value it merged with.
                if ctx.options.isolated_modules
                    && !import_is_type_only
                    && target_meaning != 0
                    && local_meaning & MEANING_VALUE != 0
                {
                    let diagnostic = Diagnostic::ts2865(local_name, ctx.file_name.clone());
                    ctx.push(match name_span {
                        Some(span) => diagnostic.with_span(convert_span(span)),
                        None => diagnostic,
                    });
                }
                continue;
            }

            let diagnostic = Diagnostic::ts2440(local_name, ctx.file_name.clone());
            let diagnostic = match name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            };
            ctx.push(diagnostic);
            // `getDeclarationSpaces`: an import specifier is a value; a default
            // or namespace import has its target's spaces, which for an
            // `export =` target its own declarations decide and the export
            // table does not record.
            let import_meaning = if clause_name != Some(local_name) {
                Some(MEANING_VALUE)
            } else {
                export_table.export_assignment_symbol.is_none().then_some(target_meaning)
            };
            if let Some(import_meaning) = import_meaning {
                report_import_merged_with_exported_declaration(
                    &parsed_file.statements,
                    local_name,
                    import_meaning,
                    name_span,
                    ctx,
                );
            }

            if local_meaning & target_meaning & MEANING_VALUE != 0 {
                symbols.remove(local_name);
            }
        }
    }
}

/// Whether `import x = E.M` names a member of an enum `E` the file declares or
/// imports by name: an enum member, which is a value and a type.
fn aliases_enum_member(
    target: &str,
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) -> bool {
    let Some((head, member)) = target.split_once('.') else {
        return false;
    };
    if member.contains('.') {
        return false;
    }
    let declares_member = |statements: &[ParsedStatement]| {
        statements.iter().any(|statement| {
            let statement = match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                    _ => return false,
                },
                other => other,
            };
            matches!(statement, ParsedStatement::TypeAliasDeclaration(alias)
                if alias.name == target && alias.enum_name.as_deref() == Some(head))
        })
    };
    if declares_member(&parsed_file.statements) {
        return true;
    }
    for statement in &parsed_file.statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };
        let specifiers = match &import.kind {
            ParsedImportKind::Named { specifiers, is_type_only: false }
            | ParsedImportKind::DefaultAndNamed { specifiers, is_type_only: false, .. } => specifiers,
            _ => continue,
        };
        let Some(specifier) = specifiers.iter().find(|specifier| specifier.local_name == head) else {
            continue;
        };
        let Some((export_table, ..)) =
            try_resolve_import_module(import, ctx, program_files, module_export_tables, module_resolution_scopes)
        else {
            return false;
        };
        let member_key = format!("{}.{member}", specifier.imported_name);
        return matches!(
            crate::modules::exports::lookup_type_export(&export_table, &member_key),
            Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) if alias.enum_name.is_some()
        );
    }
    false
}

/// `checkAliasSymbol` for `import x = E.M` naming an enum member: a local `x`
/// in its value or type space conflicts with it (TS2440, on the declaration),
/// and the import, a local declaration, then merges with any exported one.
fn report_enum_member_alias_conflict(
    import: &surge_ts_syntax::ParsedImportDeclaration,
    local_name: &str,
    name_span: Option<TextSpan>,
    local_meaning: u8,
    parsed_file: &ParsedProgramFile,
    ctx: &mut CheckerContext,
) {
    let target_meaning = MEANING_VALUE | MEANING_TYPE;
    if local_meaning & target_meaning == 0 {
        return;
    }
    let diagnostic = Diagnostic::ts2440(local_name, ctx.file_name.clone());
    ctx.push(match import.span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    report_import_merged_with_exported_declaration(&parsed_file.statements, local_name, target_meaning, name_span, ctx);
}

/// `checkExportsOnMergedDeclarations` over the symbol an import merged into:
/// the import is a local declaration, so an exported declaration of the name
/// in a space the import also occupies is TS2395 at every declaration in that
/// shared space. tsc runs the check from a class, interface, enum, namespace,
/// variable or type alias declaration, never from a function or an import.
fn report_import_merged_with_exported_declaration(
    statements: &[ParsedStatement],
    local_name: &str,
    import_meaning: u8,
    import_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let mut declarations = Vec::new();
    let mut checked = false;
    for statement in statements {
        let (declaration, exported) = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => (declaration.as_ref(), true),
                _ => continue,
            },
            other => (other, false),
        };
        let Some(&meaning) = local_declaration_meanings(std::slice::from_ref(declaration)).get(local_name) else {
            continue;
        };
        declarations.push((meaning, exported, local_declaration_name_span(declaration)));
        checked |= !matches!(declaration, ParsedStatement::FunctionDeclaration(_));
    }
    if !checked {
        return;
    }
    let exported = declarations.iter().filter(|(_, exported, _)| *exported).fold(0, |all, (meaning, ..)| all | meaning);
    let local = declarations
        .iter()
        .filter(|(_, exported, _)| !*exported)
        .fold(import_meaning, |all, (meaning, ..)| all | meaning);
    let common = exported & local;
    if common == 0 {
        return;
    }
    let spans = (import_meaning & common != 0).then_some(import_span).into_iter().chain(
        declarations.iter().filter(|(meaning, ..)| meaning & common != 0).map(|(_, _, span)| *span),
    );
    for span in spans {
        let diagnostic = Diagnostic::ts2395(local_name, ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

fn local_declaration_name_span(declaration: &ParsedStatement) -> Option<TextSpan> {
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

/// tsc's `checkAliasSymbol` under `verbatimModuleSyntax`: an import that is
/// not type-only and names a type (TS1484), or a value reached through a
/// type-only import or export (TS1485), is elided by no transpiler.
fn report_verbatim_type_imports(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    for statement in &parsed_file.statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };
        // (reported name, imported name, span)
        let bindings: Vec<(&str, &str, Option<TextSpan>)> = match &import.kind {
            ParsedImportKind::Named { is_type_only: false, specifiers } => specifiers
                .iter()
                .map(|specifier| (specifier.imported_name.as_str(), specifier.imported_name.as_str(), specifier.name_span))
                .collect(),
            ParsedImportKind::DefaultAndNamed { local_name, name_span, is_type_only: false, specifiers } => {
                std::iter::once((local_name.as_str(), "default", *name_span))
                    .chain(specifiers.iter().map(|specifier| {
                        (specifier.imported_name.as_str(), specifier.imported_name.as_str(), specifier.name_span)
                    }))
                    .collect()
            }
            ParsedImportKind::Default { local_name, name_span } => {
                vec![(local_name.as_str(), "default", *name_span)]
            }
            ParsedImportKind::Equals { local_name, is_type_only: false, .. } => {
                vec![(local_name.as_str(), crate::modules::exports::EXPORT_ASSIGNMENT_KEY, import.span)]
            }
            _ => continue,
        };
        if bindings.is_empty() {
            continue;
        }
        let Some((export_table, _, _)) = try_resolve_import_module(
            import,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        ) else {
            continue;
        };
        if export_table.has_unresolved_star_export || export_table.has_incomplete_declaration_surface {
            continue;
        }
        for (reported_name, imported_name, span) in bindings {
            let is_value = if imported_name == crate::modules::exports::EXPORT_ASSIGNMENT_KEY {
                if !export_table.writes_export_assignment {
                    continue;
                }
                export_table.export_assignment_symbol.is_some()
            } else if imported_name == "default" {
                // A default import of an `export =` module binds the assigned
                // entity under `esModuleInterop`.
                export_table.default_symbol.is_some() || export_table.writes_export_assignment
            } else {
                lookup_value_export(&export_table, imported_name).is_some()
            };
            // `export =` publishes its type under `default`.
            let type_key = if imported_name == crate::modules::exports::EXPORT_ASSIGNMENT_KEY { "default" } else { imported_name };
            let is_type = lookup_type_export(&export_table, type_key).is_some();
            if is_value && is_ambient_const_enum_export(&export_table, type_key) {
                push_ambient_const_enum_alias(span, ctx);
            }
            let diagnostic = if !is_value && is_type {
                Diagnostic::ts1484(reported_name, ctx.file_name.clone())
            } else if is_value && export_table.type_only_exports.contains_key(imported_name) {
                Diagnostic::ts1485(reported_name, ctx.file_name.clone())
            } else {
                continue;
            };
            ctx.push(match span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
    }
}

fn is_ambient_const_enum_export(export_table: &ModuleExportTable, name: &str) -> bool {
    matches!(
        lookup_type_export(export_table, name),
        Some(crate::symbols::TypeDeclarationInfo::Alias(alias))
            if alias.enum_is_const
                && alias.enum_name.is_some()
                && crate::modules::is_declaration_file_name(&alias.file_name)
    )
}

/// `checkAliasSymbol` under `verbatimModuleSyntax`: an alias of an ambient
/// const enum names a value no emitted import or export can carry (TS2748).
fn push_ambient_const_enum_alias(span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let Some(span) = span else {
        return;
    };
    ctx.push(Diagnostic::ts2748("verbatimModuleSyntax", ctx.file_name.clone()).with_span(convert_span(span)));
}

/// tsc's `extendExportStars`: a name two `export *` declarations bring from
/// different modules is ambiguous (TS2308, on the later one) unless the module
/// exports the name itself. Only the names a target declares itself are
/// compared: one it re-exports may be the same symbol on both sides.
fn report_export_star_conflicts(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    let stars: Vec<(&str, Option<TextSpan>)> = parsed_file
        .statements
        .iter()
        .filter_map(|statement| match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::All { module_specifier, span, .. } => Some((module_specifier.as_str(), *span)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    if stars.len() < 2 {
        return;
    }
    let own = exported_names_of(parsed_file, false);
    let mut first_owner: std::collections::HashMap<String, (&str, usize)> = std::collections::HashMap::new();
    for (specifier, span) in stars {
        let Some((_, _, Some(index))) =
            try_resolve_module(specifier, ctx, program_files, module_export_tables, module_resolution_scopes)
        else {
            continue;
        };
        let Some(target) = program_files.get(index) else {
            continue;
        };
        for name in exported_names_of(target, true) {
            if name == "default" || own.contains(&name) {
                continue;
            }
            match first_owner.get(name) {
                Some(&(first_specifier, first_index)) if first_index != index => {
                    let diagnostic = Diagnostic::ts2308(format!("\"{first_specifier}\""), name, ctx.file_name.clone());
                    ctx.push(match span {
                        Some(span) => diagnostic.with_span(convert_span(span)),
                        None => diagnostic,
                    });
                }
                Some(_) => {}
                None => {
                    first_owner.insert(name.to_string(), (specifier, index));
                }
            }
        }
    }
}

/// TS2456 for `type A = B` whose chain of references comes back to `A`
/// through another file's import (`a.ts: type A = B` with `B` imported from
/// `b.ts: type B = A`), namespace members included (`ns2.nested.T`). A cycle
/// within one file is the grammar walk's.
fn report_cross_file_alias_cycles(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    fn reference(alias: &ParsedTypeAliasDeclaration) -> Option<Vec<&str>> {
        match &alias.ty {
            ParsedType::Named(named) if alias.type_parameters.is_empty() && named.type_arguments.is_empty() => {
                Some(named.name.split('.').collect())
            }
            _ => None,
        }
    }
    fn find_alias<'a>(statements: &'a [ParsedStatement], path: &[&str]) -> Option<&'a ParsedTypeAliasDeclaration> {
        let (head, rest) = path.split_first()?;
        statements.iter().find_map(|statement| match crate::modules::exports::peel_exported_statement(statement) {
            ParsedStatement::TypeAliasDeclaration(alias) if rest.is_empty() && alias.name == *head => Some(alias.as_ref()),
            ParsedStatement::NamespaceDeclaration(namespace) if !rest.is_empty() && namespace.member_name() == *head => {
                find_alias(&namespace.statements, rest)
            }
            _ => None,
        })
    }
    fn collect_aliases<'a>(statements: &'a [ParsedStatement], nested: bool, out: &mut Vec<(&'a ParsedTypeAliasDeclaration, bool)>) {
        for statement in statements {
            match crate::modules::exports::peel_exported_statement(statement) {
                ParsedStatement::TypeAliasDeclaration(alias) => out.push((alias.as_ref(), nested)),
                ParsedStatement::NamespaceDeclaration(namespace) => collect_aliases(&namespace.statements, true, out),
                _ => {}
            }
        }
    }
    fn imported<'a>(file: &'a ParsedProgramFile, name: &str) -> Option<(&'a str, &'a str)> {
        file.statements.iter().find_map(|statement| {
            let ParsedStatement::ImportDeclaration(import) = statement else {
                return None;
            };
            let specifiers = match &import.kind {
                ParsedImportKind::Named { specifiers, .. } | ParsedImportKind::DefaultAndNamed { specifiers, .. } => {
                    specifiers
                }
                _ => return None,
            };
            specifiers
                .iter()
                .find(|specifier| specifier.local_name == name)
                .map(|specifier| (import.module_specifier.as_str(), specifier.imported_name.as_str()))
        })
    }
    let mut aliases = Vec::new();
    collect_aliases(&parsed_file.statements, false, &mut aliases);
    let mut reports = Vec::new();
    for (alias, nested) in aliases {
        let Some(mut path) = reference(alias) else {
            continue;
        };
        // A bare name inside a namespace resolves in that namespace first,
        // which this walk does not model.
        if nested && path.len() == 1 && imported(parsed_file, path[0]).is_none() {
            continue;
        }
        let mut file = parsed_file;
        let mut crossed = false;
        for _ in 0..16 {
            if let Some(next) = find_alias(&file.statements, &path) {
                if crossed && file.file_name == parsed_file.file_name && next.name_span == alias.name_span {
                    reports.push((alias.name.clone(), alias.name_span));
                    break;
                }
                let in_namespace = path.len() > 1;
                match reference(next) {
                    Some(next_path) if !(in_namespace && next_path.len() == 1 && imported(file, next_path[0]).is_none()) => {
                        path = next_path;
                        continue;
                    }
                    _ => break,
                }
            }
            let Some((specifier, imported_name)) = imported(file, path[0]) else {
                break;
            };
            let target_index = crate::modules::resolution::resolve_relative_module(
                &file.file_name,
                specifier,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .map(|resolution| resolution.resolved_file_index)
            .or_else(|| {
                let resolved =
                    crate::modules::resolution::resolved_module_in_mode(ctx, &file.file_name, specifier, None)?;
                let identity = crate::modules::resolution::canonical_file_identity(resolved);
                ctx.module_file_index_by_identity.get(identity.as_str()).copied()
            });
            let Some(target) = target_index.and_then(|index| program_files.get(index)) else {
                break;
            };
            file = target;
            path[0] = imported_name;
            crossed = true;
        }
    }
    for (name, span) in reports {
        let diagnostic = Diagnostic::ts2456(name, ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// The names a module exports by its own declarations, and, unless
/// `declared_only`, by its export clauses too.
fn exported_names_of(file: &ParsedProgramFile, declared_only: bool) -> Vec<&str> {
    let mut names = Vec::new();
    for statement in &file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                let mut declared: Vec<&str> =
                    crate::program::module_scope_declared_names(std::slice::from_ref(declaration.as_ref()))
                        .into_iter()
                        .collect();
                declared.sort_unstable();
                names.extend(declared);
            }
            ParsedExportDeclaration::Named { specifiers, .. } if !declared_only => {
                names.extend(specifiers.iter().map(|specifier| specifier.exported_name.as_str()));
            }
            ParsedExportDeclaration::Namespace { exported_name, .. } if !declared_only => {
                names.push(exported_name.as_str());
            }
            _ => {}
        }
    }
    names
}

/// The export half of `checkAliasSymbol` under `isolatedModules` (which
/// `verbatimModuleSyntax` implies): a re-export that is not type-only and
/// names a type is TS1205, and one of a value reached through another file's
/// type-only alias TS1448. A local name is followed through the file's own
/// non-type-only imports; a name it declares itself is not judged.
fn report_verbatim_type_reexports(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    let imported_from = |local: &str| -> Option<(&ParsedImportDeclaration, String)> {
        parsed_file.statements.iter().find_map(|statement| {
            let ParsedStatement::ImportDeclaration(import) = statement else {
                return None;
            };
            match &import.kind {
                ParsedImportKind::Named { is_type_only: false, specifiers }
                | ParsedImportKind::DefaultAndNamed { is_type_only: false, specifiers, .. }
                    if let Some(specifier) = specifiers.iter().find(|specifier| specifier.local_name == local) =>
                {
                    Some((import.as_ref(), specifier.imported_name.clone()))
                }
                _ => None,
            }
        })
    };
    for statement in &parsed_file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        if let ParsedExportDeclaration::Default {
            declaration: surge_ts_syntax::ParsedDefaultExportDeclaration::Expression(surge_ts_syntax::ParsedExpression::Identifier { name, span }),
            ..
        } = export.as_ref()
        {
            report_type_default_export(parsed_file, name, *span, program_files, module_export_tables, module_resolution_scopes, ctx);
            continue;
        }
        if let ParsedExportDeclaration::Equals { exported_name, exported_name_span, .. } = export.as_ref() {
            report_type_export_assignment(parsed_file, exported_name, *exported_name_span, ctx);
            continue;
        }
        let ParsedExportDeclaration::Named {
            is_type_only: false,
            specifiers,
            module_specifier,
            module_specifier_span,
            span,
            resolution_mode,
        } = export.as_ref()
        else {
            continue;
        };
        for specifier in specifiers.iter().filter(|specifier| !specifier.is_type_only) {
            let source = match module_specifier {
                Some(module_specifier) => Some((
                    ParsedImportDeclaration {
                        kind: ParsedImportKind::Named { is_type_only: false, specifiers: Vec::new() },
                        module_specifier: module_specifier.clone(),
                        module_specifier_span: *module_specifier_span,
                        span: *span,
                        resolution_mode: resolution_mode.clone(),
                        inline_type_specifiers: false,
                    },
                    specifier.local_name.clone(),
                )),
                None => imported_from(&specifier.local_name)
                    .map(|(import, imported_name)| (import.clone(), imported_name)),
            };
            let Some((import, imported_name)) = source else {
                continue;
            };
            let Some((export_table, _, _)) = try_resolve_import_module(
                &import,
                ctx,
                program_files,
                module_export_tables,
                module_resolution_scopes,
            ) else {
                continue;
            };
            if export_table.has_unresolved_star_export || export_table.has_incomplete_declaration_surface {
                continue;
            }
            let flag = if ctx.options.verbatim_module_syntax { "verbatimModuleSyntax" } else { "isolatedModules" };
            let is_value = lookup_value_export(&export_table, &imported_name).is_some();
            if ctx.options.verbatim_module_syntax
                && is_value
                && is_ambient_const_enum_export(&export_table, &imported_name)
            {
                push_ambient_const_enum_alias(specifier.name_span, ctx);
            }
            let diagnostic = if !is_value && lookup_type_export(&export_table, &imported_name).is_some() {
                Diagnostic::ts1205(flag, ctx.file_name.clone())
            } else if is_value && export_table.type_only_exports.contains_key(imported_name.as_str()) {
                Diagnostic::ts1448(&specifier.local_name, flag, ctx.file_name.clone())
            } else {
                continue;
            };
            ctx.push(match specifier.name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
    }
}

/// `checkExportAssignment` for `export = T` under `verbatimModuleSyntax`: a
/// name that is only a local type is TS1282, and one bound by `import type T =
/// require(…)` TS1283. Any other binding is left unjudged.
fn report_type_export_assignment(
    parsed_file: &ParsedProgramFile,
    name: &str,
    span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.verbatim_module_syntax || is_declaration_file_name(&ctx.file_name) {
        return;
    }
    let mut declares_value = false;
    let mut declares_type = false;
    let mut type_only_import = false;
    let mut imported = false;
    for statement in &parsed_file.statements {
        match crate::modules::exports::peel_exported_statement(statement) {
            ParsedStatement::VariableDeclaration(variable) if variable.name == name => declares_value = true,
            ParsedStatement::FunctionDeclaration(function) if function.name == name => declares_value = true,
            ParsedStatement::ClassDeclaration(class) if class.name == name => declares_value = true,
            ParsedStatement::NamespaceDeclaration(namespace) if namespace.name == name => {
                if crate::program::is_instantiated_namespace(namespace) {
                    declares_value = true;
                } else {
                    declares_type = true;
                }
            }
            ParsedStatement::TypeAliasDeclaration(alias) if alias.name == name => declares_type = true,
            ParsedStatement::InterfaceDeclaration(interface) if interface.name == name => declares_type = true,
            ParsedStatement::ImportDeclaration(import) => match &import.kind {
                ParsedImportKind::Equals { local_name, is_type_only, .. } if local_name == name => {
                    imported = true;
                    type_only_import = *is_type_only;
                }
                ParsedImportKind::Named { specifiers, .. } | ParsedImportKind::DefaultAndNamed { specifiers, .. }
                    if specifiers.iter().any(|specifier| specifier.local_name == name) =>
                {
                    imported = true;
                }
                ParsedImportKind::Default { local_name, .. }
                | ParsedImportKind::TypeOnlyDefault { local_name, .. }
                | ParsedImportKind::Namespace { local_name, .. }
                | ParsedImportKind::DefaultAndNamed { local_name, .. }
                    if local_name == name =>
                {
                    imported = true;
                }
                _ => {}
            },
            _ => {}
        }
    }
    let diagnostic = if declares_value {
        return;
    } else if imported {
        if !type_only_import {
            return;
        }
        Diagnostic::ts1283(name, ctx.file_name.clone())
    } else if declares_type {
        Diagnostic::ts1282(name, ctx.file_name.clone())
    } else {
        return;
    };
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// `checkExportAssignment` for `export default T` of a name: under
/// `verbatimModuleSyntax` one that is only a type is TS1284, and one bound by
/// a type-only import TS1285; under `isolatedModules` alone an imported name
/// that is only a type cannot be elided by a single-file transpiler (TS1292).
/// A local value of the same name makes it a value.
fn report_type_default_export(
    parsed_file: &ParsedProgramFile,
    name: &str,
    span: Option<TextSpan>,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    let verbatim = ctx.options.verbatim_module_syntax;
    // `export default` in a file emitted as CommonJS is TS1295 instead.
    if verbatim && crate::program::file_emits_commonjs(ctx) {
        return;
    }
    let mut declares_value = false;
    let mut declares_type = false;
    for statement in &parsed_file.statements {
        match crate::modules::exports::peel_exported_statement(statement) {
            ParsedStatement::VariableDeclaration(variable) if variable.name == name => declares_value = true,
            ParsedStatement::FunctionDeclaration(function) if function.name == name => declares_value = true,
            ParsedStatement::ClassDeclaration(class) if class.name == name => declares_value = true,
            ParsedStatement::NamespaceDeclaration(namespace) if namespace.name == name => declares_value = true,
            ParsedStatement::TypeAliasDeclaration(alias) if alias.name == name => declares_type = true,
            ParsedStatement::InterfaceDeclaration(interface) if interface.name == name => declares_type = true,
            _ => {}
        }
    }
    if declares_value {
        return;
    }
    let imported = parsed_file.statements.iter().find_map(|statement| {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            return None;
        };
        match &import.kind {
            ParsedImportKind::Named { is_type_only, specifiers }
            | ParsedImportKind::DefaultAndNamed { is_type_only, specifiers, .. } => specifiers
                .iter()
                .find(|specifier| specifier.local_name == name)
                .map(|specifier| (import.as_ref(), specifier.imported_name.clone(), *is_type_only))
                .or_else(|| match &import.kind {
                    ParsedImportKind::DefaultAndNamed { local_name, is_type_only, .. } if local_name == name => {
                        Some((import.as_ref(), "default".to_string(), *is_type_only))
                    }
                    _ => None,
                }),
            ParsedImportKind::Default { local_name, .. } if local_name == name => {
                Some((import.as_ref(), "default".to_string(), false))
            }
            ParsedImportKind::TypeOnlyDefault { local_name, .. } if local_name == name => {
                Some((import.as_ref(), "default".to_string(), true))
            }
            _ => None,
        }
    });
    let diagnostic = match imported {
        None if verbatim && declares_type => Diagnostic::ts1284(name, ctx.file_name.clone()),
        None => return,
        Some((import, imported_name, type_only)) => {
            // A local type merged into the import adds no value, so under
            // `isolatedModules` its non-local meanings still decide TS1292.
            if declares_type && verbatim {
                return;
            }
            let Some((export_table, _, _)) =
                try_resolve_import_module(import, ctx, program_files, module_export_tables, module_resolution_scopes)
            else {
                return;
            };
            if export_table.has_unresolved_star_export || export_table.has_incomplete_declaration_surface {
                return;
            }
            let is_value = lookup_value_export(&export_table, &imported_name).is_some();
            let is_type = lookup_type_export(&export_table, &imported_name).is_some();
            match (verbatim, is_value, is_type) {
                (true, true, _) if type_only => Diagnostic::ts1285(name, ctx.file_name.clone()),
                (true, false, true) => Diagnostic::ts1284(name, ctx.file_name.clone()),
                (false, false, true) if ctx.options.isolated_modules && !type_only => {
                    Diagnostic::ts1292(name, "isolatedModules", ctx.file_name.clone())
                }
                _ => return,
            }
        }
    };
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

pub(crate) fn resolve_module_imports(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    ctx: &mut CheckerContext,
) -> ModuleImportBindings {
    let mut type_declarations = TypeDeclarationTable::new();
    let mut symbols = SymbolTable::new();
    let mut namespace_alias_layers = Vec::new();
    let mut type_only_aliases = Vec::new();
    let mut namespace_imports = NamespaceImports::default();

    for statement in &parsed_file.statements {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            continue;
        };

        resolve_import_declaration(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            &mut type_declarations,
            &mut symbols,
            &mut namespace_alias_layers,
            &mut type_only_aliases,
            &mut namespace_imports,
            ctx,
        );
        // A module is a value, so a type-only namespace import used as one is
        // TS1361 whatever the module exports, resolved or not.
        if let ParsedImportKind::Namespace {
            local_name,
            is_type_only: true,
            ..
        } = &import.kind
        {
            type_only_aliases.push((Arc::from(local_name.as_str()), TypeOnlyAliasKind::Import));
        }
    }

    register_import_type_namespaces(
        parsed_file,
        program_files,
        module_export_tables,
        module_resolution_scopes,
        ctx,
    );

    // A UMD global (`export as namespace React`) is a global symbol naming its
    // module (Go merges each file's `globalExports` into the globals), so a
    // module that neither declares nor imports the name reads the namespace's
    // types through it — `React.Context` with no import of React — after its
    // own declarations and imports.
    if !ctx.umd_global_names.is_empty() {
        let declared = crate::program::module_scope_declared_names(&parsed_file.statements);
        let mut umd_globals: Vec<(Arc<str>, usize)> = ctx
            .umd_global_names
            .iter()
            .map(|(name, index)| (name.clone(), *index))
            .collect();
        umd_globals.sort();
        for (name, module_index) in umd_globals {
            let declaring_file = program_files.get(module_index);
            if declaring_file.is_none_or(|file| file.file_name == parsed_file.file_name)
                || declared.contains(name.as_ref())
                || local_symbol_exists(&name)
                || symbols.get_handle(&name).is_some()
                || type_declarations.get(&name).is_some()
            {
                continue;
            }
            let Some(export_table) = module_export_tables.get(module_index).and_then(Option::as_ref)
            else {
                continue;
            };
            let scope = module_resolution_scopes
                .get(module_index)
                .and_then(Option::as_ref);
            namespace_alias_layers.push(namespace_alias_table(
                export_table,
                &name,
                scope,
                Some(module_index),
            ));
        }
    }

    report_import_local_declaration_conflicts(
        parsed_file,
        program_files,
        module_export_tables,
        module_resolution_scopes,
        &mut symbols,
        ctx,
    );
    if ctx.options.verbatim_module_syntax {
        report_verbatim_type_imports(parsed_file, program_files, module_export_tables, module_resolution_scopes, ctx);
    }
    if ctx.options.verbatim_module_syntax || ctx.options.isolated_modules {
        report_verbatim_type_reexports(parsed_file, program_files, module_export_tables, module_resolution_scopes, ctx);
    }
    report_export_star_conflicts(parsed_file, program_files, module_export_tables, module_resolution_scopes, ctx);
    report_cross_file_alias_cycles(parsed_file, program_files, ctx);

    check_module_augmentations(
        parsed_file,
        program_files,
        module_export_tables,
        module_resolution_scopes,
        ctx,
    );

    ModuleImportBindings {
        type_declarations: Arc::new(type_declarations),
        symbols,
        namespace_alias_layers,
        type_only_aliases,
        complete_namespace_imports: namespace_imports.complete,
        value_only_import_aliases: namespace_imports.value_only,
    }
}

/// `typeof import("spec")` in this file's type positions reads the module
/// namespace value exactly as `import * as ns` would; the loader already
/// resolved these specifiers, so the namespace shape is registered here and
/// read back when the query resolves under this file's name. A script file
/// (no import/export of its own) can still write the query, so this runs for
/// every file, not only the ones whose imports are bound.
pub(crate) fn register_import_type_namespaces(
    parsed_file: &ParsedProgramFile,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &mut CheckerContext,
) {
    let mut registered: Vec<&str> = Vec::new();
    for specifier in &parsed_file.import_call_specifiers {
        if registered.iter().any(|seen| *seen == specifier.as_str()) {
            continue;
        }
        registered.push(specifier);
        let resolved = try_resolve_module(
            specifier,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        );
        let Some((export_table, _scope, resolved_index)) = resolved else {
            continue;
        };
        // `typeof import("m")` is the type of the module's symbol, which for
        // an `export =` module is the assigned value (`resolveExternalModuleSymbol`).
        // A shorthand ambient module's symbol is typed `any`
        // (`getTypeOfFuncClassEnumModule` on a module without a body).
        let namespace_type = match &export_table.export_assignment_symbol {
            _ if export_table.shorthand => surge_ts_types::Type::Any,
            Some(assignment) => assignment.ty.clone(),
            None => {
                let namespace_type = namespace_export_object_type(&export_table);
                match resolved_index.and_then(|index| program_files.get(index)) {
                    Some(resolved_file) => {
                        tag_namespace_type_with_module_path(namespace_type, &resolved_file.file_name)
                    }
                    None => namespace_type,
                }
            }
        };
        let export_assignment = export_table.export_assignment_symbol.is_some()
            || export_table.type_declarations.get(EXPORT_ASSIGNMENT_NAME).is_some();
        let target = || crate::context::ImportTypeTarget {
            namespace: namespace_type.clone(),
            declarations: export_table.type_declarations.clone(),
            scope: _scope.clone(),
            export_assignment,
            attached: Default::default(),
        };
        let canonical = crate::paths::canonicalize_if_exists_arc(std::path::Path::new(
            &parsed_file.file_name,
        ));
        if *canonical != *parsed_file.file_name {
            ctx.register_import_type_namespace(&canonical, specifier, target());
        }
        ctx.register_import_type_namespace(&parsed_file.file_name, specifier, target());
    }
}

/// Looks up an ambient `declare module "…"` export table, honoring wildcard
/// patterns (`declare module "*.css"`, which Next's `next-env.d.ts` relies on for
/// `import "./globals.css"`). tsc matches a single `*` against any substring and
/// prefers the pattern with the longest matching prefix; an exact declaration
/// always wins.
pub(crate) fn ambient_module_export_table<'a>(
    ctx: &'a CheckerContext,
    module_specifier: &str,
) -> Option<&'a ModuleExportTable> {
    ctx.ambient_modules
        .get(ambient_module_name(ctx, module_specifier)?)
}

/// The name of the ambient module [`ambient_module_export_table`] finds for
/// `module_specifier`: the declaration itself or the wildcard pattern.
fn ambient_module_name<'a>(ctx: &'a CheckerContext, module_specifier: &str) -> Option<&'a str> {
    if let Some((name, _)) = ctx.ambient_modules.get_key_value(module_specifier) {
        return Some(name);
    }

    let mut best: Option<(usize, &str)> = None;
    for pattern in ctx.ambient_modules.keys() {
        let Some((prefix, suffix)) = pattern.split_once('*') else {
            continue;
        };
        if suffix.contains('*')
            || module_specifier.len() < prefix.len() + suffix.len()
            || !module_specifier.starts_with(prefix)
            || !module_specifier.ends_with(suffix)
        {
            continue;
        }
        if best.is_none_or(|(best_prefix, _)| prefix.len() > best_prefix) {
            best = Some((prefix.len(), pattern));
        }
    }

    best.map(|(_, pattern)| pattern)
}

/// Whether a resolved file must yield to an ambient `declare module
/// "<specifier>"` of the same name. A file with no top-level import/export is a
/// global script, not an external module, so its (empty) export table must not
/// shadow the ambient declaration — packages whose `types` entry is a wrapper
/// containing only `declare module "pkg" { … }` depend on this. Only an exact
/// specifier match counts; a wildcard pattern never outranks a resolved file.
pub(crate) fn resolved_file_yields_to_ambient_module(
    ctx: &CheckerContext,
    program_files: &[ParsedProgramFile],
    resolved_index: usize,
    module_specifier: &str,
) -> bool {
    // Go's `resolveExternalModule` (checker.go:15369) consults
    // `tryFindAmbientModule` *before* any file resolution, so an ambient
    // `declare module "querystring"` wins over a node_modules package of that
    // name whether or not the package's entry point is a module. Requiring the
    // resolved file to be a script bound every Node builtin type import in a
    // project that also installs the userland polyfill (`querystring@0.2.1`
    // ships `decode.d.ts` as a module) to the polyfill, which declares none of
    // the types — so `ParsedUrlQuery` read as "no exported member" and
    // `NextRouter.query` degraded with it. An augmentation is not an ambient
    // module and is applied separately (`module_augmentations`).
    let _ = (program_files, resolved_index);
    ctx.ambient_modules.contains_key(module_specifier)
}

pub(crate) fn try_resolve_module(
    module_specifier: &str,
    ctx: &CheckerContext,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
) -> Option<(
    ModuleExportTable,
    Option<Arc<TypeDeclarationScope>>,
    Option<usize>,
)> {
    try_resolve_module_in_mode(
        module_specifier,
        None,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    )
}

/// [`try_resolve_module`] for an import declaration, whose syntax can pick
/// the mode its specifier resolves in (see [`import_resolution_mode`]).
fn try_resolve_import_module(
    import: &ParsedImportDeclaration,
    ctx: &CheckerContext,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
) -> Option<(
    ModuleExportTable,
    Option<Arc<TypeDeclarationScope>>,
    Option<usize>,
)> {
    try_resolve_module_in_mode(
        &import.module_specifier,
        import_resolution_mode(import),
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    )
}

pub(crate) fn try_resolve_module_in_mode(
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
    ctx: &CheckerContext,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
) -> Option<(
    ModuleExportTable,
    Option<Arc<TypeDeclarationScope>>,
    Option<usize>,
)> {
    let resolution_start = Instant::now();
    if let Some(resolved_file_name) =
        resolved_module_in_mode(ctx, &ctx.file_name, module_specifier, resolution_mode)
    {
        let resolved_file_name = canonical_file_identity(resolved_file_name);
        if let Some(resolved_index) = ctx
            .module_file_index_by_identity
            .get(resolved_file_name.as_str())
        {
            let yields_to_ambient = resolved_file_yields_to_ambient_module(
                ctx,
                program_files,
                *resolved_index,
                module_specifier,
            );
            if let Some(Some(export_table)) = module_export_tables
                .get(*resolved_index)
                .filter(|_| !yields_to_ambient)
            {
                let scope = module_resolution_scopes
                    .get(*resolved_index)
                    .and_then(|scope| scope.clone());
                record_program_timing(ctx.timings.as_ref(), |timings| {
                    timings.export_table_lookup += resolution_start.elapsed();
                    timings.package_export_lookup += resolution_start.elapsed();
                    timings.import_specifier_resolution += resolution_start.elapsed();
                });
                let mut export_table = export_table.clone_with_reason(TypeCopyReason::ModuleExport);
                if let Some(augmentation) = ctx.module_augmentations.get(module_specifier) {
                    crate::program::apply_module_augmentation(&mut export_table, augmentation);
                }
                // A relative `declare module "./sibling"` files under the target's
                // identity, not under any string a consumer writes.
                crate::program::apply_file_keyed_module_augmentation(
                    &mut export_table,
                    resolved_file_name.as_str(),
                    ctx,
                );
                return Some((export_table, scope, Some(*resolved_index)));
            }
        }
    }

    if let Some(export_table) = ambient_module_export_table(ctx, module_specifier) {
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.package_export_lookup += resolution_start.elapsed();
            timings.import_specifier_resolution += resolution_start.elapsed();
        });
        let mut export_table = export_table.clone_with_reason(TypeCopyReason::ModuleExport);
        if let Some(augmentation) = ctx.module_augmentations.get(module_specifier) {
            crate::program::apply_module_augmentation(&mut export_table, augmentation);
        }
        return Some((
            export_table,
            Some(Arc::new(TypeDeclarationScope::new(vec![Arc::new(
                ctx.type_declarations.clone(),
            )]))),
            None,
        ));
    }

    if let Some(resolved) = resolve_relative_module_in_mode(
        &ctx.file_name,
        module_specifier,
        resolution_mode,
        program_files,
        &ctx.module_file_index_by_identity,
    ) {
        if let Some(Some(export_table)) = module_export_tables.get(resolved.resolved_file_index) {
            let scope = module_resolution_scopes
                .get(resolved.resolved_file_index)
                .and_then(|scope| scope.clone());
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.export_table_lookup += resolution_start.elapsed();
                timings.import_specifier_resolution += resolution_start.elapsed();
            });
            let mut export_table = export_table.clone_with_reason(TypeCopyReason::ModuleExport);
            crate::program::apply_file_keyed_module_augmentation(
                &mut export_table,
                &canonical_file_identity(&resolved.resolved_file_name),
                ctx,
            );
            return Some((export_table, scope, Some(resolved.resolved_file_index)));
        }
    }

    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.import_specifier_resolution += resolution_start.elapsed()
    });
    None
}

pub(crate) fn resolve_import_declaration(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    type_only_aliases: &mut Vec<(Arc<str>, TypeOnlyAliasKind)>,
    namespace_imports: &mut NamespaceImports,
    ctx: &mut CheckerContext,
) {
    report_ts_extension_import(import, program_files, ctx);
    report_jsx_resolution(import, program_files, ctx);
    // A JavaScript file's `require` and JSDoc `@import` lower to import
    // declarations tsc does not count as one.
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    let syntax = match &import.kind {
        ParsedImportKind::EntityAlias { .. } | ParsedImportKind::Unsupported => None,
        ParsedImportKind::Equals { .. } if !javascript => Some(ModuleImportSyntax::ImportEquals),
        kind if import_is_type_only(kind) && !import.inline_type_specifiers && !javascript => {
            Some(ModuleImportSyntax::TypeOnlyImportClause)
        }
        _ => Some(ModuleImportSyntax::Other),
    };
    if let Some(syntax) = syntax {
        report_synchronous_import_of_esm(
            &import.module_specifier,
            import.module_specifier_span,
            syntax,
            import_resolution_mode(import),
            import.resolution_mode.is_some(),
            program_files,
            ctx,
        );
    }
    if !matches!(
        import.kind,
        ParsedImportKind::EntityAlias { .. } | ParsedImportKind::SideEffect
    ) && resolves_to_shorthand_ambient_module(
        &import.module_specifier,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    ) {
        bind_shorthand_module_import(import, local_symbol_exists, type_declarations, symbols, ctx);
        return;
    }
    if let Some(named) =
        module_dot_exports_import(import, program_files, module_export_tables, module_resolution_scopes, ctx)
    {
        resolve_named_import(
            &named,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            type_only_aliases,
            ctx,
        );
        return;
    }
    // A namespace import or `import x = require(…)` of a module whose `export =`
    // names an `import type` binding inherits the type-only alias.
    // tsc resolves such a binding to the assigned target, which surge's empty
    // module object is not; after the TS1361 its uses report nothing more.
    let mut type_only_namespace = None;
    if let ParsedImportKind::Namespace { local_name, is_type_only: false, .. }
    | ParsedImportKind::Equals { local_name, is_type_only: false, .. } = &import.kind
        && let Some((export_table, _, _)) =
            try_resolve_import_module(import, ctx, program_files, module_export_tables, module_resolution_scopes)
        && export_table.type_only_exports.contains_key(crate::modules::exports::EXPORT_ASSIGNMENT_KEY)
    {
        record_type_only_alias(
            &export_table,
            crate::modules::exports::EXPORT_ASSIGNMENT_KEY,
            local_name,
            type_only_aliases,
        );
        type_only_namespace = Some(local_name.clone());
    }
    match &import.kind {
        ParsedImportKind::EntityAlias { .. } => return,
        ParsedImportKind::Unsupported => {
            if !is_declaration_file_name(&ctx.file_name) {
                emit_unsupported_module_syntax_diagnostic(ctx, import);
            }
            return;
        }
        ParsedImportKind::DefaultAndNamed { .. } => resolve_default_and_named_import(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            type_only_aliases,
            ctx,
        ),
        ParsedImportKind::Default { .. } | ParsedImportKind::TypeOnlyDefault { .. } => resolve_default_import(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            type_only_aliases,
            ctx,
        ),
        ParsedImportKind::Namespace { .. } => resolve_namespace_import(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            namespace_imports,
            ctx,
        ),
        ParsedImportKind::Equals { .. } => resolve_import_equals(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            namespace_imports,
            ctx,
        ),
        ParsedImportKind::SideEffect => {
            if ambient_module_export_table(ctx, &import.module_specifier).is_some() {
                return;
            }
            if ctx
                .options
                .resolved_module_for(&ctx.file_name, &import.module_specifier)
                .is_some()
            {
                return;
            }
            if resolve_relative_module(
                &ctx.file_name,
                &import.module_specifier,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .is_none()
            {
                report_unresolved_module(ctx, import);
            }
            return;
        }
        ParsedImportKind::Named { .. } => resolve_named_import(
            import,
            program_files,
            module_export_tables,
            module_resolution_scopes,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            type_only_aliases,
            ctx,
        ),
    };
    if let Some(local_name) = type_only_namespace {
        symbols.insert(
            local_name,
            crate::symbols::SymbolInfo {
                ty: Type::Any,
                kind: crate::symbols::SymbolKind::Const,
                function_signature: None,
            },
        );
    }
}

/// Under `node20`/`nodenext` an ES module's `module.exports` named export is
/// what `require()` of it returns, so an import that reads the module through
/// `require` binds that export as `import { "module.exports" as local }` would:
/// `import x = require()` and a JavaScript `require` always
/// (`getTargetOfImportEqualsDeclaration`), a default or namespace import only
/// when written in a CommonJS-format file importing an ESM-format one
/// (`getTargetOfModuleDefault`, `resolveESModuleSymbol`).
fn module_dot_exports_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    ctx: &CheckerContext,
) -> Option<ParsedImportDeclaration> {
    const MODULE_EXPORTS: &str = "module.exports";
    if !matches!(
        ctx.options.module_emit,
        crate::context::ModuleEmitKind::Node20 | crate::context::ModuleEmitKind::NodeNext
    ) {
        return None;
    }
    let (local_name, name_span, is_type_only, transpiled_default) = match &import.kind {
        ParsedImportKind::Equals {
            local_name,
            name_span,
            is_type_only,
            ..
        } => (local_name, *name_span, *is_type_only, false),
        ParsedImportKind::Default { local_name, name_span } => (local_name, *name_span, false, true),
        ParsedImportKind::Namespace {
            local_name,
            name_span,
            is_type_only,
        } => (local_name, *name_span, *is_type_only, true),
        _ => return None,
    };
    let (export_table, _, resolved_index) =
        try_resolve_import_module(import, ctx, program_files, module_export_tables, module_resolution_scopes)?;
    if transpiled_default {
        let target = program_files.get(resolved_index?)?;
        if declaration_emit_syntax(ctx) != Some(ModuleFormat::CommonJs)
            || implied_format_for_emit(ctx, &target.file_name) != Some(ModuleFormat::Esm)
        {
            return None;
        }
    }
    if export_table.symbols.get(MODULE_EXPORTS).is_none()
        && crate::modules::exports::lookup_type_export(&export_table, MODULE_EXPORTS).is_none()
    {
        return None;
    }
    Some(ParsedImportDeclaration {
        kind: ParsedImportKind::Named {
            is_type_only,
            specifiers: vec![surge_ts_syntax::ParsedImportSpecifier {
                imported_name: MODULE_EXPORTS.to_string(),
                local_name: local_name.clone(),
                name_span,
            }],
        },
        ..import.clone()
    })
}

/// Whether `module_specifier` lands on a shorthand ambient module
/// (`declare module "x";`) by the precedence [`try_resolve_module`] applies.
fn resolves_to_shorthand_ambient_module(
    module_specifier: &str,
    ctx: &CheckerContext,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
) -> bool {
    ambient_module_export_table(ctx, module_specifier).is_some_and(|table| table.shorthand)
        && try_resolve_module(
            module_specifier,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        )
        .is_some_and(|(table, _, _)| table.shorthand)
}

/// Every binding a shorthand ambient module gives — default, named, namespace
/// or `import =` — is the module symbol itself (`getExternalModuleMember`
/// returns the module for a named import): `any` as a value, and a namespace
/// with no members as a type, which surge leaves unresolved rather than
/// misreport as a value.
fn bind_shorthand_module_import(
    import: &ParsedImportDeclaration,
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    ctx: &mut CheckerContext,
) {
    let (value_locals, type_locals): (Vec<&str>, Vec<&str>) = match &import.kind {
        ParsedImportKind::Default { local_name, .. } | ParsedImportKind::Equals { local_name, .. } => {
            (vec![local_name], vec![])
        }
        ParsedImportKind::TypeOnlyDefault { local_name, .. } => (vec![], vec![local_name]),
        ParsedImportKind::Namespace {
            local_name,
            is_type_only,
            ..
        } => {
            if *is_type_only {
                (vec![], vec![local_name])
            } else {
                (vec![local_name], vec![])
            }
        }
        ParsedImportKind::Named {
            is_type_only,
            specifiers,
        } => {
            let locals = specifiers.iter().map(|specifier| specifier.local_name.as_str()).collect();
            if *is_type_only { (vec![], locals) } else { (locals, vec![]) }
        }
        ParsedImportKind::DefaultAndNamed {
            local_name,
            is_type_only,
            specifiers,
            ..
        } => {
            let locals = std::iter::once(local_name.as_str())
                .chain(specifiers.iter().map(|specifier| specifier.local_name.as_str()))
                .collect();
            if *is_type_only { (vec![], locals) } else { (locals, vec![]) }
        }
        ParsedImportKind::EntityAlias { .. }
        | ParsedImportKind::SideEffect
        | ParsedImportKind::Unsupported => (vec![], vec![]),
    };
    for local in value_locals.iter().chain(&type_locals) {
        if type_declarations.get(local).is_none() {
            crate::modules::exports::insert_error_type_import(
                type_declarations,
                local,
                ctx.file_name_arc(),
                None,
            );
        }
    }
    for local in value_locals {
        if !local_symbol_exists(local) {
            crate::modules::exports::insert_value_import(local, Type::Any, symbols);
        }
    }
}

fn resolve_default_and_named_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    type_only_aliases: &mut Vec<(Arc<str>, TypeOnlyAliasKind)>,
    ctx: &mut CheckerContext,
) {
    let ParsedImportKind::DefaultAndNamed {
        local_name,
        name_span,
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        return;
    };
    let Some((export_table, default_scope, resolved_index)) = try_resolve_import_module(
        import,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    ) else {
        if resolve_relative_module(
            &ctx.file_name,
            &import.module_specifier,
            program_files,
            &ctx.module_file_index_by_identity,
        )
        .is_none()
        {
            report_unresolved_module(ctx, import);
        } else if !report_non_module_import(ctx, import, program_files) {
            emit_no_default_export_diagnostic(
                ctx,
                local_name,
                *name_span,
                resolve_relative_module(
                    &ctx.file_name,
                    &import.module_specifier,
                    program_files,
                    &ctx.module_file_index_by_identity,
                )
                .map(|resolution| resolution.resolved_file_index)
                .filter(|&index| {
                    program_files
                        .get(index)
                        .is_some_and(|file| !file.file_kind.is_declaration())
                }),
                program_files,
            );
        }

        if *is_type_only {
            let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
                local_name.clone(),
                ctx.file_name_arc(),
                *name_span,
                vec![],
                ParsedType::ErrorType,
                None,
            ));
            if type_declarations.get(local_name).is_none() {
                let _ = type_declarations.insert(local_name.clone(), declaration);
            }
        } else {
            insert_unresolved_import_type(local_name, *name_span, type_declarations, ctx);
            insert_unresolved_import_binding(local_name, ctx, import, symbols);
        }

        for specifier in specifiers {
            if *is_type_only {
                let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
                    specifier.local_name.to_string(),
                    ctx.file_name_arc(),
                    specifier.name_span,
                    vec![],
                    ParsedType::ErrorType,
                    None,
                ));
                if type_declarations.get(&specifier.local_name).is_none() {
                    let _ = type_declarations.insert(specifier.local_name.clone(), declaration);
                }
            } else {
                let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
                    specifier.local_name.to_string(),
                    ctx.file_name_arc(),
                    specifier.name_span,
                    vec![],
                    ParsedType::ErrorType,
                    None,
                ));
                if type_declarations.get(&specifier.local_name).is_none() {
                    let _ = type_declarations.insert(specifier.local_name.clone(), declaration);
                }
                insert_unresolved_import_binding(&specifier.local_name, ctx, import, symbols);
            }
        }
        return;
    };

    // Bind the default specifier through the same default-import resolution it
    // would take on its own. A missing default emits TS2305 (unless the module
    // is an incomplete declaration surface) and binds an unknown placeholder,
    // but never returns early: the named specifiers below must still bind so a
    // missing default does not cascade into TS2304 on their usages.
    let synthetic_default = can_have_synthetic_default(
        ctx,
        resolved_index.and_then(|index| program_files.get(index)),
        &export_table,
    );
    if synthetic_default {
        bind_external_module_symbol(
            &export_table,
            default_scope.as_ref(),
            resolved_index,
            &import.module_specifier,
            local_name,
            !*is_type_only,
            program_files,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            ctx,
        );
    } else {
        bind_default_type_import(
            &export_table,
            default_scope.as_ref(),
            local_name,
            type_declarations,
        );

        match export_table.get_shared_value("default") {
            Some(default_symbol) => {
                if *is_type_only {
                    let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
                        local_name.clone(),
                        ctx.file_name_arc(),
                        *name_span,
                        vec![],
                        ParsedType::ErrorType,
                        None,
                    ));
                    if type_declarations.get(local_name).is_none() {
                        let _ = type_declarations.insert(local_name.clone(), declaration);
                    }
                } else if !local_symbol_exists(local_name) {
                    symbols.insert_shared(local_name.clone(), default_symbol);
                }
            }
            None => {
                if export_table.type_declarations.get("default").is_none()
                    && !should_bind_unknown_for_missing_export(&export_table, resolved_index, program_files)
                {
                    emit_no_default_export_diagnostic(
                        ctx,
                        local_name,
                        *name_span,
                        resolved_index,
                        program_files,
                    );
                }
                if *is_type_only {
                    let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
                        local_name.clone(),
                        ctx.file_name_arc(),
                        *name_span,
                        vec![],
                        ParsedType::ErrorType,
                        None,
                    ));
                    if type_declarations.get(local_name).is_none() {
                        let _ = type_declarations.insert(local_name.clone(), declaration);
                    }
                } else {
                    insert_unknown_value_import(local_name, symbols);
                }
            }
        }
    }

    for specifier in specifiers {
        if synthetic_default && specifier.imported_name == "default" {
            bind_external_module_symbol(
                &export_table,
                default_scope.as_ref(),
                resolved_index,
                &import.module_specifier,
                &specifier.local_name,
                !*is_type_only,
                program_files,
                local_symbol_exists,
                type_declarations,
                symbols,
                namespace_alias_layers,
                ctx,
            );
            continue;
        }
        if emit_export_equals_member_import_diagnostic(
            ctx,
            &export_table,
            &import.module_specifier,
            &specifier.imported_name,
            specifier.name_span,
            resolved_index,
            program_files,
        ) {
            insert_unknown_type_import(type_declarations, &specifier.local_name, ctx.file_name_arc(), specifier.name_span);
            insert_unknown_value_import(&specifier.local_name, symbols);
            continue;
        }
        if imported_name_is_unexported_local(resolved_index, program_files, &specifier.imported_name) {
            emit_unexported_local_import_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                module_has_explicit_default_export(
                    &import.module_specifier,
                    resolved_index,
                    program_files,
                    ctx,
                ),
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
            continue;
        }
        // Named specifiers resolve against the module export by their imported
        // name; the local name is only the binding target (e.g. `helper as h`).
        let type_export = lookup_type_export(&export_table, &specifier.imported_name);
        let value_export = lookup_value_export(&export_table, &specifier.imported_name);

        let has_qualified_type_exports = copy_qualified_type_exports(
            &export_table,
            &specifier.imported_name,
            &specifier.local_name,
            type_declarations,
        );
        copy_qualified_value_exports(
            &export_table,
            &specifier.imported_name,
            &specifier.local_name,
            symbols,
        );

        if *is_type_only {
            if let Some(type_export) = type_export {
                export_local_type_declaration(
                    type_export,
                    &specifier.local_name,
                    None,
                    type_declarations,
                );
                // `import type` binds the whole symbol: the value side of a
                // class or enum is what `typeof C` reads in a type position.
                if let Some(value_export) = value_export
                    && symbols.get(&specifier.local_name).is_none()
                {
                    symbols.insert_shared(specifier.local_name.clone(), value_export);
                }
                continue;
            }

            // A type-only namespace (`export namespace enumUtil { export type … }`)
            // has no direct export entry, only qualified `ns.Member` ones; the
            // import is still valid.
            if has_qualified_type_exports {
                continue;
            }

            // A value-only export is a legal `import type` target (`typeof f`).
            if let Some(value_export) = value_export {
                if symbols.get(&specifier.local_name).is_none() {
                    symbols.insert_shared(specifier.local_name.clone(), value_export);
                }
                continue;
            }

            emit_missing_named_import_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                module_has_explicit_default_export(
                    &import.module_specifier,
                    resolved_index,
                    program_files,
                    ctx,
                ),
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            continue;
        }

        let mut found = has_qualified_type_exports;

        if let Some(type_export) = type_export {
            export_local_type_declaration(
                type_export,
                &specifier.local_name,
                None,
                type_declarations,
            );
            found = true;
        }

        if let Some(value_export) = value_export {
            if symbols.get(&specifier.local_name).is_none() {
                symbols.insert_shared(specifier.local_name.clone(), value_export);
            }
            record_type_only_alias(
                &export_table,
                &specifier.imported_name,
                &specifier.local_name,
                type_only_aliases,
            );
            found = true;
        }

        if !found {
            if should_bind_unknown_for_missing_export(&export_table, resolved_index, program_files)
            {
                insert_unknown_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                insert_unknown_value_import(&specifier.local_name, symbols);
                continue;
            }

            emit_missing_named_import_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                module_has_explicit_default_export(
                    &import.module_specifier,
                    resolved_index,
                    program_files,
                    ctx,
                ),
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
        }
    }
    return;
}

fn resolve_default_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    type_only_aliases: &mut Vec<(Arc<str>, TypeOnlyAliasKind)>,
    ctx: &mut CheckerContext,
) {
    // `import type D from "m"` binds the same alias in type space only; a
    // value use of it is TS1361, reported where the value is read.
    let (local_name, name_span, type_only) = match &import.kind {
        ParsedImportKind::Default {
            local_name,
            name_span,
        } => (local_name, name_span, false),
        ParsedImportKind::TypeOnlyDefault {
            local_name,
            name_span,
        } => (local_name, name_span, true),
        _ => return,
    };
    let Some((export_table, scope, resolved_index)) = try_resolve_import_module(
        import,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    ) else {
        if resolve_relative_module(
            &ctx.file_name,
            &import.module_specifier,
            program_files,
            &ctx.module_file_index_by_identity,
        )
        .is_none()
        {
            report_unresolved_module(ctx, import);
        } else if !report_non_module_import(ctx, import, program_files) {
            emit_no_default_export_diagnostic(
                ctx,
                local_name,
                *name_span,
                resolve_relative_module(
                    &ctx.file_name,
                    &import.module_specifier,
                    program_files,
                    &ctx.module_file_index_by_identity,
                )
                .map(|resolution| resolution.resolved_file_index)
                .filter(|&index| {
                    program_files
                        .get(index)
                        .is_some_and(|file| !file.file_kind.is_declaration())
                }),
                program_files,
            );
        }
        if type_only {
            insert_error_type_import(type_declarations, local_name, ctx.file_name_arc(), *name_span);
        } else {
            insert_unresolved_import_type(local_name, *name_span, type_declarations, ctx);
            insert_unresolved_import_binding(local_name, ctx, import, symbols);
        }
        return;
    };
    if !type_only {
        record_type_only_alias(&export_table, "default", local_name, type_only_aliases);
    }

    // `getTargetOfModuleDefault`: a synthetic default overrides a real
    // `default` export.
    if can_have_synthetic_default(
        ctx,
        resolved_index.and_then(|index| program_files.get(index)),
        &export_table,
    ) {
        bind_external_module_symbol(
            &export_table,
            scope.as_ref(),
            resolved_index,
            &import.module_specifier,
            local_name,
            !type_only,
            program_files,
            local_symbol_exists,
            type_declarations,
            symbols,
            namespace_alias_layers,
            ctx,
        );
        return;
    }

    bind_default_type_import(&export_table, scope.as_ref(), local_name, type_declarations);

    let Some(default_symbol) = export_table.get_shared_value("default") else {
        let type_default = export_table.type_declarations.get("default").is_some();
        // A `default` with a type meaning only (`export default interface`) is
        // an export all the same.
        if !type_default && !should_bind_unknown_for_missing_export(&export_table, resolved_index, program_files) {
            emit_no_default_export_diagnostic(
                ctx,
                local_name,
                *name_span,
                resolved_index,
                program_files,
            );
        }
        // `checkAliasSymbol`: a JavaScript file imports values only.
        if !type_only && type_default && surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
            let import_text = format!("import(\"{}\")", import.module_specifier);
            let diagnostic = Diagnostic::ts18042(local_name, import_text, ctx.file_name.clone());
            ctx.push(match name_span {
                Some(span) => diagnostic.with_span(convert_span(*span)),
                None => diagnostic,
            });
        }
        if !type_only {
            insert_unknown_value_import(local_name, symbols);
        }
        return;
    };

    if !type_only && !local_symbol_exists(local_name) {
        symbols.insert_shared(local_name.clone(), default_symbol);
    }
}

/// A default-exported class contributes a type as well as a value, so
/// `import D from "./m"` must bind both — otherwise `type X = D` and any
/// re-export of `D` from this module lose the type side entirely.
fn bind_default_type_import(
    export_table: &ModuleExportTable,
    scope: Option<&Arc<TypeDeclarationScope>>,
    local_name: &str,
    type_declarations: &mut TypeDeclarationTable,
) {
    copy_qualified_type_exports(export_table, "default", local_name, type_declarations);
    let Some(type_export) = lookup_type_export(export_table, "default") else {
        return;
    };
    if type_declarations.get(local_name).is_some() {
        return;
    }
    insert_type_export(type_declarations, local_name, scope, type_export.clone());
}

fn resolve_import_equals(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    namespace_imports: &mut NamespaceImports,
    ctx: &mut CheckerContext,
) {
    let ParsedImportKind::Equals {
        local_name,
        name_span,
        ..
    } = &import.kind
    else {
        return;
    };

    let Some((export_table, scope, resolved_index)) = try_resolve_import_module(
        import,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    ) else {
        if resolve_relative_module_in_mode(
            &ctx.file_name,
            &import.module_specifier,
            import_resolution_mode(import),
            program_files,
            &ctx.module_file_index_by_identity,
        )
        .is_none()
        {
            report_unresolved_module(ctx, import);
        } else {
            report_non_module_import(ctx, import, program_files);
        }
        // The alias resolves to tsc's `unknownSymbol`, whose every meaning is
        // the error type.
        insert_error_type_import(
            type_declarations,
            local_name,
            ctx.file_name_arc(),
            *name_span,
        );
        insert_unresolved_import_binding(local_name, ctx, import, symbols);
        return;
    };

    if export_table.export_assignment_symbol.is_none()
        && module_export_surface_is_complete(&export_table, resolved_index, program_files, ctx)
    {
        namespace_imports.complete.push(Arc::from(local_name.as_str()));
    }
    if export_table.export_assignment_symbol.is_some()
        && resolved_index
            .and_then(|index| program_files.get(index))
            .is_some_and(|file| {
                !surge_ts_syntax::is_javascript_file_name(&file.file_name)
                    && export_assignment_targets_non_module_entity(file)
            })
    {
        namespace_imports.value_only.push(Arc::from(local_name.as_str()));
    }
    // `getTargetOfImportEqualsDeclaration` → `resolveExternalModuleSymbol`.
    bind_external_module_symbol(
        &export_table,
        scope.as_ref(),
        resolved_index,
        &import.module_specifier,
        local_name,
        true,
        program_files,
        local_symbol_exists,
        type_declarations,
        symbols,
        namespace_alias_layers,
        ctx,
    );
}

/// tsc's `resolveExternalModuleSymbol` as an import binds it: the entity the
/// module's `export =` names, with its value, its type and its namespace
/// members alike, or without an `export =` the module namespace.
/// `import x = require("m")` binds it, and so does a default import of a
/// module with a synthetic default.
fn bind_external_module_symbol(
    export_table: &ModuleExportTable,
    scope: Option<&Arc<TypeDeclarationScope>>,
    resolved_index: Option<usize>,
    module_specifier: &str,
    local_name: &str,
    binds_value: bool,
    program_files: &[ParsedProgramFile],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    ctx: &CheckerContext,
) {
    let assignment_type = lookup_type_export(export_table, EXPORT_ASSIGNMENT_NAME).cloned();
    let assignment_members = export_assignment_member_table(export_table, local_name, scope);
    if export_table.export_assignment_symbol.is_some()
        || assignment_type.is_some()
        || assignment_members.is_some()
    {
        if binds_value
            && let Some(symbol) = export_table.export_assignment_symbol.clone()
            && !local_symbol_exists(local_name)
        {
            symbols.insert_shared(local_name, symbol);
        }
        if let Some(declaration) = assignment_type
            && type_declarations.get(local_name).is_none()
        {
            insert_type_export(type_declarations, local_name, scope, declaration);
        }
        if let Some(members) = assignment_members {
            namespace_alias_layers.push(members);
        }
        return;
    }

    // A module that *writes* `export = target` but whose target surge could not
    // resolve keeps the unknown placeholder: its real shape is that value, not
    // the module namespace, and standing the (empty) namespace in its place
    // turns every use into a cascade.
    if export_table.writes_export_assignment && !export_table.export_assignment_names_module {
        if binds_value {
            insert_unknown_value_import(local_name, symbols);
        }
        return;
    }

    // Without an export assignment the target is an ordinary module, and the
    // import binds its namespace — the same object `import * as x` binds.
    // Binding an unknown placeholder instead left every `x.member` and every
    // `typeof x.member` silent, which is what opened the whole jscodeshift
    // surface in tRPC's `upgrade` transforms: its `JSCodeshift` is an
    // intersection over `typeof recast.types.namedTypes`, reached through
    // `import recast = require("recast")`. An ambient `declare module "m"` is
    // such a module too (`resolveExternalModuleSymbol` returns the module
    // symbol when it has no `export =`).
    namespace_alias_layers.push(namespace_alias_table(
        export_table,
        local_name,
        scope,
        resolved_index,
    ));
    if !binds_value {
        return;
    }
    crate::modules::exports::copy_namespace_alias_value_exports(export_table, local_name, symbols);

    if local_symbol_exists(local_name) {
        return;
    }

    symbols.insert(
        local_name,
        SymbolInfo {
            ty: module_namespace_type(export_table, resolved_index, module_specifier, program_files, ctx),
            kind: SymbolKind::Const,
            function_signature: None,
        },
    );
}

/// The type side [`bind_external_module_symbol`] binds under `local_name`: the
/// `export =` entity's type and its namespace members, or without an
/// `export =` the module namespace's members.
pub(crate) fn external_module_type_bindings(
    export_table: &ModuleExportTable,
    scope: Option<&Arc<TypeDeclarationScope>>,
    resolved_index: Option<usize>,
    local_name: &str,
) -> (Option<TypeDeclarationInfo>, Option<Arc<TypeDeclarationTable>>) {
    let assignment_type = lookup_type_export(export_table, EXPORT_ASSIGNMENT_NAME).cloned();
    let assignment_members = export_assignment_member_table(export_table, local_name, scope);
    if export_table.export_assignment_symbol.is_some()
        || assignment_type.is_some()
        || assignment_members.is_some()
    {
        return (assignment_type, assignment_members);
    }
    if export_table.writes_export_assignment && !export_table.export_assignment_names_module {
        return (None, None);
    }
    (
        None,
        Some(namespace_alias_table(export_table, local_name, scope, resolved_index)),
    )
}

/// The value [`bind_external_module_symbol`] binds, for a re-export of it.
pub(crate) fn external_module_value(
    export_table: &ModuleExportTable,
    resolved_index: Option<usize>,
    module_specifier: &str,
    program_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Arc<SymbolInfo> {
    if let Some(symbol) = &export_table.export_assignment_symbol {
        return symbol.clone();
    }
    let ty = if export_table.writes_export_assignment && !export_table.export_assignment_names_module {
        Type::Unknown
    } else {
        module_namespace_type(export_table, resolved_index, module_specifier, program_files, ctx)
    };
    Arc::new(SymbolInfo {
        ty,
        kind: SymbolKind::Const,
        function_signature: None,
    })
}

fn module_namespace_type(
    export_table: &ModuleExportTable,
    resolved_index: Option<usize>,
    module_specifier: &str,
    program_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> Type {
    let namespace_type = namespace_export_object_type(export_table);
    let module_name = match resolved_index {
        Some(index) => program_files.get(index).map(|file| file.file_name.as_str()),
        None => ambient_module_name(ctx, module_specifier),
    };
    match module_name {
        Some(module_name) => tag_namespace_type_with_module_path(namespace_type, module_name),
        None => namespace_type,
    }
}

thread_local! {
    // `import * as ns from "m"` re-keys every type `m` exports under `ns.<member>`.
    // The result depends only on the resolved module and the alias, so a barrel
    // namespace-imported by many files (or the same module imported repeatedly)
    // would otherwise rebuild an O(exports) table per importer. Cached by
    // (resolved module index, alias) and cleared per run.
    static NAMESPACE_ALIAS_TABLE_CACHE: RefCell<HashMap<(usize, String), Arc<TypeDeclarationTable>>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn clear_namespace_alias_table_cache() {
    NAMESPACE_ALIAS_TABLE_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// Build the `ns.<member>` type-declaration table for a namespace import. Mirrors
/// the per-member `insert_type_export` the eager path used, but produces a
/// standalone table that is appended to the resolution scope as a shared layer.
fn build_namespace_alias_table(
    export_table: &ModuleExportTable,
    local_name: &str,
    namespace_scope: Option<&Arc<TypeDeclarationScope>>,
) -> Arc<TypeDeclarationTable> {
    let mut table = TypeDeclarationTable::new();
    for (key, declaration) in export_table.type_declarations.iter() {
        if is_export_assignment_key(key) {
            continue;
        }
        // A member of an exported namespace is keyed `ns.Member`, and under a
        // namespace import its tsc-visible name keeps that qualifier
        // (`local.ns.Member`). Registering only the last segment leaves the real
        // name unresolvable, so the reference silently degrades to an open type.
        let local_key = format!("{local_name}.{key}");
        if table.get(&local_key).is_none() {
            crate::modules::exports::insert_type_export(
                &mut table,
                &local_key,
                namespace_scope,
                declaration.clone(),
            );
        }
    }
    Arc::new(table)
}

/// The `local.<member>` type layer an `import local = require(...)` binds for
/// the namespace members of the module's `export =` entity; `None` when the
/// entity has none.
fn export_assignment_member_table(
    export_table: &ModuleExportTable,
    local_name: &str,
    scope: Option<&Arc<TypeDeclarationScope>>,
) -> Option<Arc<TypeDeclarationTable>> {
    let prefix = format!("{EXPORT_ASSIGNMENT_NAME}.");
    let mut table = TypeDeclarationTable::new();
    for (key, declaration) in export_table.type_declarations.iter() {
        let Some(member) = key.strip_prefix(prefix.as_str()) else {
            continue;
        };
        crate::modules::exports::insert_type_export(
            &mut table,
            &format!("{local_name}.{member}"),
            scope,
            declaration.clone(),
        );
    }
    (table.len() > 0).then(|| Arc::new(table))
}

fn namespace_alias_table(
    export_table: &ModuleExportTable,
    local_name: &str,
    namespace_scope: Option<&Arc<TypeDeclarationScope>>,
    resolved_index: Option<usize>,
) -> Arc<TypeDeclarationTable> {
    let Some(index) = resolved_index else {
        return build_namespace_alias_table(export_table, local_name, namespace_scope);
    };

    let cache_key = (index, local_name.to_string());
    if let Some(cached) =
        NAMESPACE_ALIAS_TABLE_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned())
    {
        return cached;
    }
    let table = build_namespace_alias_table(export_table, local_name, namespace_scope);
    NAMESPACE_ALIAS_TABLE_CACHE.with(|cache| {
        cache.borrow_mut().insert(cache_key, table.clone());
    });
    table
}

/// Whether surge saw the module's whole export list: tsc's `resolveQualifiedName`
/// reports a member the list lacks, which surge can only repeat for a module
/// whose `export *` targets all resolved and whose surface it lowered in full.
/// `export =` modules expose their entity's members instead, and a JavaScript
/// module's exports come from assignments surge only partly models.
fn module_export_surface_is_complete(
    export_table: &ModuleExportTable,
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> bool {
    let Some(index) = resolved_index else {
        return false;
    };
    let Some(file) = program_files.get(index) else {
        return false;
    };
    !export_table.has_unresolved_star_export
        && !export_table.has_incomplete_declaration_surface
        && !export_table.shorthand
        && !export_table.writes_export_assignment
        && file.json_module_type.is_none()
        && !surge_ts_syntax::is_javascript_file_name(&file.file_name)
        && !should_bind_unknown_for_missing_export(export_table, resolved_index, program_files)
        && !module_has_unresolved_star_export(index, program_files, &ctx.module_file_index_by_identity)
}

fn resolve_namespace_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    namespace_imports: &mut NamespaceImports,
    ctx: &mut CheckerContext,
) {
    let ParsedImportKind::Namespace {
        local_name,
        name_span,
        is_type_only,
    } = &import.kind
    else {
        return;
    };
    if *is_type_only {
        // `import type * as ns` still exposes the module's exported types under
        // the qualified alias (`ns.Member`), and `typeof ns.Member` stays legal
        // in type positions — only emitting the binding at runtime is elided. So
        // the namespace value shape is registered too, otherwise
        // `ComponentProps<typeof LabelPrimitive.Root>` reports a false TS2304.
        if let Some((export_table, scope, resolved_index)) = try_resolve_import_module(
            import,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        ) {
            if module_export_surface_is_complete(&export_table, resolved_index, program_files, ctx) {
                namespace_imports.complete.push(Arc::from(local_name.as_str()));
            }
            namespace_alias_layers.push(namespace_alias_table(
                &export_table,
                local_name,
                scope.as_ref(),
                resolved_index,
            ));

            crate::modules::exports::copy_namespace_alias_value_exports(
                &export_table,
                local_name,
                symbols,
            );

            if !local_symbol_exists(local_name) {
                let namespace_type = namespace_export_object_type(&export_table);
                let namespace_type = match resolved_index.and_then(|index| program_files.get(index))
                {
                    Some(resolved_file) => {
                        tag_namespace_type_with_module_path(namespace_type, &resolved_file.file_name)
                    }
                    None => namespace_type,
                };
                symbols.insert(
                    local_name.clone(),
                    SymbolInfo {
                        ty: namespace_type,
                        kind: SymbolKind::Const,
                        function_signature: None,
                    },
                );
            }
        } else if resolve_relative_module(
            &ctx.file_name,
            &import.module_specifier,
            program_files,
            &ctx.module_file_index_by_identity,
        )
        .is_none()
        {
            report_unresolved_module(ctx, import);
        } else {
            report_non_module_import(ctx, import, program_files);
        }

        let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            local_name.clone(),
            ctx.file_name_arc(),
            None,
            vec![],
            ParsedType::ErrorType,
            None,
        ));
        if type_declarations.get(local_name).is_none() {
            let _ = type_declarations.insert(local_name.clone(), declaration);
        }
        return;
    }

    let (namespace_type, namespace_export_table, namespace_scope, namespace_resolved_index) =
        if let Some((export_table, scope, resolved_index)) = try_resolve_import_module(
            import,
            ctx,
            program_files,
            module_export_tables,
            module_resolution_scopes,
        ) {
            let synthetic_default = namespace_import_has_synthetic_default(
                ctx,
                resolved_index.and_then(|index| program_files.get(index)),
                &export_table,
            );
            // `resolveESModuleSymbol`: the import is the `export =` target
            // itself, and a target with only a type meaning (`export =
            // Interface`) binds no value, as `import x = require` does.
            if !synthetic_default
                && export_table.export_assignment_symbol.is_none()
                && export_table.writes_export_assignment
                && !export_table.export_assignment_names_module
                && lookup_type_export(&export_table, EXPORT_ASSIGNMENT_NAME).is_some()
            {
                bind_external_module_symbol(
                    &export_table,
                    scope.as_ref(),
                    resolved_index,
                    &import.module_specifier,
                    local_name,
                    true,
                    program_files,
                    local_symbol_exists,
                    type_declarations,
                    symbols,
                    namespace_alias_layers,
                    ctx,
                );
                return;
            }
            let namespace_type = namespace_export_object_type(&export_table);
            let namespace_type = if synthetic_default {
                let default_value = external_module_value(
                    &export_table,
                    resolved_index,
                    &import.module_specifier,
                    program_files,
                    ctx,
                );
                with_synthetic_default_member(namespace_type, default_value.ty.clone())
            } else if let Some(assigned) = export_table
                .export_assignment_symbol
                .as_ref()
                .filter(|symbol| !matches!(symbol.ty.peeled(), Type::Object(_)))
            {
                // `resolveESModuleSymbol`: with no synthetic default the import
                // is the `export =` target itself, whose value is no namespace
                // object to surface members from.
                assigned.ty.clone()
            } else {
                namespace_type
            };
            // tsc displays a namespace import object as `typeof import("<path>")`
            // (absolute, without the source extension) rather than the structural
            // shape. Tag the object with that display form when we know the file.
            let namespace_type = match resolved_index.and_then(|index| program_files.get(index)) {
                Some(resolved_file) if matches!(namespace_type, Type::Object(_)) => {
                    tag_namespace_type_with_module_path(namespace_type, &resolved_file.file_name)
                }
                _ => namespace_type,
            };
            (namespace_type, Some(export_table), scope, resolved_index)
        } else {
            if resolve_relative_module(
                &ctx.file_name,
                &import.module_specifier,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .is_none()
            {
                report_unresolved_module(ctx, import);
            } else {
                report_non_module_import(ctx, import, program_files);
            }
            insert_unresolved_import_type(local_name, *name_span, type_declarations, ctx);
            insert_unresolved_import_binding(local_name, ctx, import, symbols);
            return;
        };

    // Re-expose the module's exported types under the namespace alias so qualified
    // type references resolve (`React.ComponentProps<...>`, `M.SomeType`). Members
    // of an `export = <namespace>` keep their `<ns>.<member>` keys here; the first
    // segment is replaced with the local alias. Built once per (module, alias) and
    // appended as a shared scope layer rather than copied into every importer's
    // table, so `import * as` of a large barrel stays O(1) per importer.
    if let Some(export_table) = &namespace_export_table {
        if module_export_surface_is_complete(
            export_table,
            namespace_resolved_index,
            program_files,
            ctx,
        ) {
            namespace_imports.complete.push(Arc::from(local_name.as_str()));
        }
        namespace_alias_layers.push(namespace_alias_table(
            export_table,
            local_name,
            namespace_scope.as_ref(),
            namespace_resolved_index,
        ));
        crate::modules::exports::copy_namespace_alias_value_exports(
            export_table,
            local_name,
            symbols,
        );
    }

    if !local_symbol_exists(local_name) {
        symbols.insert(
            local_name.clone(),
            SymbolInfo {
                ty: namespace_type,
                kind: SymbolKind::Const,
                function_signature: None,
            },
        );
    }
}

/// tsc's `resolveIndirectionAlias`: an import of an export whose alias chain
/// passes through a type-only declaration inherits it, so the import's own
/// value uses are reported against that declaration.
fn record_type_only_alias(
    export_table: &ModuleExportTable,
    imported_name: &str,
    local_name: &str,
    type_only_aliases: &mut Vec<(Arc<str>, TypeOnlyAliasKind)>,
) {
    if let Some(kind) = export_table.type_only_exports.get(imported_name) {
        type_only_aliases.push((Arc::from(local_name), *kind));
    }
}

fn resolve_named_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    module_export_tables: &[Option<ModuleExportTable>],
    module_resolution_scopes: &[Option<Arc<TypeDeclarationScope>>],
    local_symbol_exists: &dyn Fn(&str) -> bool,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    namespace_alias_layers: &mut Vec<Arc<TypeDeclarationTable>>,
    type_only_aliases: &mut Vec<(Arc<str>, TypeOnlyAliasKind)>,
    ctx: &mut CheckerContext,
) {
    let ParsedImportKind::Named {
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        return;
    };
    let Some((export_table, scope, resolved_index)) = try_resolve_import_module(
        import,
        ctx,
        program_files,
        module_export_tables,
        module_resolution_scopes,
    ) else {
        let Some(resolved) = resolve_relative_module(
            &ctx.file_name,
            &import.module_specifier,
            program_files,
            &ctx.module_file_index_by_identity,
        ) else {
            report_unresolved_module(ctx, import);
            for specifier in specifiers {
                if *is_type_only {
                    insert_error_type_import(
                        type_declarations,
                        &specifier.local_name,
                        ctx.file_name_arc(),
                        specifier.name_span,
                    );
                    continue;
                }

                insert_error_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                insert_unresolved_import_binding(&specifier.local_name, ctx, import, symbols);
            }
            return;
        };

        if report_non_module_import(ctx, import, program_files) {
            for specifier in specifiers {
                insert_error_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                if !*is_type_only {
                    insert_unresolved_import_binding(&specifier.local_name, ctx, import, symbols);
                }
            }
            return;
        }

        let local_scope = module_resolution_scopes
            .get(resolved.resolved_file_index)
            .and_then(|scope| scope.clone());

        for specifier in specifiers {
            if let Some(local_scope) = &local_scope {
                if let Some(local_declaration) = local_scope.get(&specifier.imported_name).cloned()
                {
                    if *is_type_only {
                        insert_type_export(
                            type_declarations,
                            &specifier.local_name,
                            Some(&local_scope),
                            local_declaration,
                        );
                        continue;
                    }

                    insert_type_export(
                        type_declarations,
                        &specifier.local_name,
                        Some(&local_scope),
                        local_declaration,
                    );
                    continue;
                }
            }

            emit_missing_named_import_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                module_has_explicit_default_export(
                    &import.module_specifier,
                    None,
                    program_files,
                    ctx,
                ),
            );

            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
        }
        return;
    };

    let Some(scope) = scope else {
        let local_scope = module_resolution_scopes
            .get(resolved_index.unwrap_or(usize::MAX))
            .and_then(|scope| scope.clone());

        for specifier in specifiers {
            if let Some(local_scope) = &local_scope {
                if let Some(local_declaration) = local_scope.get(&specifier.imported_name).cloned()
                {
                    if *is_type_only {
                        insert_type_export(
                            type_declarations,
                            &specifier.local_name,
                            Some(&local_scope),
                            local_declaration,
                        );
                        continue;
                    }

                    insert_type_export(
                        type_declarations,
                        &specifier.local_name,
                        Some(&local_scope),
                        local_declaration,
                    );
                    continue;
                }
            }

            emit_missing_export_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
            );

            if *is_type_only {
                insert_unknown_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                continue;
            }

            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
        }
        return;
    };

    let has_unresolved_star_export = export_table.has_unresolved_star_export
        || resolved_index
            .map(|i| {
                module_has_unresolved_star_export(
                    i,
                    program_files,
                    &ctx.module_file_index_by_identity,
                )
            })
            .unwrap_or(false);

    // `getTargetOfImportSpecifier` sends a `default` specifier through
    // `getTargetOfModuleDefault`, where a synthetic default wins.
    let synthetic_default = specifiers
        .iter()
        .any(|specifier| specifier.imported_name == "default")
        && can_have_synthetic_default(
            ctx,
            resolved_index.and_then(|index| program_files.get(index)),
            &export_table,
        );
    for specifier in specifiers {
        if synthetic_default && specifier.imported_name == "default" {
            bind_external_module_symbol(
                &export_table,
                Some(&scope),
                resolved_index,
                &import.module_specifier,
                &specifier.local_name,
                !*is_type_only,
                program_files,
                local_symbol_exists,
                type_declarations,
                symbols,
                namespace_alias_layers,
                ctx,
            );
            continue;
        }
        if emit_export_equals_member_import_diagnostic(
            ctx,
            &export_table,
            &import.module_specifier,
            &specifier.imported_name,
            specifier.name_span,
            resolved_index,
            program_files,
        ) {
            insert_unknown_type_import(type_declarations, &specifier.local_name, ctx.file_name_arc(), specifier.name_span);
            insert_unknown_value_import(&specifier.local_name, symbols);
            continue;
        }
        if imported_name_is_unexported_local(resolved_index, program_files, &specifier.imported_name) {
            emit_unexported_local_import_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                module_has_explicit_default_export(
                    &import.module_specifier,
                    resolved_index,
                    program_files,
                    ctx,
                ),
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
            continue;
        }
        let type_export = lookup_type_export(&export_table, &specifier.imported_name);
        let value_export = lookup_value_export(&export_table, &specifier.imported_name);
        let has_qualified_type_exports = copy_qualified_type_exports(
            &export_table,
            &specifier.imported_name,
            &specifier.local_name,
            type_declarations,
        );
        copy_qualified_value_exports(
            &export_table,
            &specifier.imported_name,
            &specifier.local_name,
            symbols,
        );
        if *is_type_only {
            if let Some(type_export) = type_export {
                insert_type_export(
                    type_declarations,
                    &specifier.local_name,
                    Some(&scope),
                    type_export.clone(),
                );
                // `import type` binds the whole symbol: the value side of a
                // class or enum is what `typeof C` reads in a type position.
                if let Some(value_export) = value_export {
                    symbols.insert_shared(specifier.local_name.clone(), value_export);
                }
                continue;
            }

            if let Some(local_declaration) = scope.get(&specifier.imported_name).cloned() {
                insert_type_export(
                    type_declarations,
                    &specifier.local_name,
                    Some(&scope),
                    local_declaration,
                );
                continue;
            }

            // `import type { f }` imports the SYMBOL, so a value-only export
            // (function/const) is a legal target — it is usable in type
            // position through `typeof f`. Binding the value here is what makes
            // that query resolve instead of cascading TS2304. That holds for an
            // `export * as ns` namespace too, whose types are only qualified
            // `ns.Member` entries: `typeof ns.member` still reads its value side.
            if let Some(value_export) = value_export {
                symbols.insert_shared(specifier.local_name.clone(), value_export);
                continue;
            }

            if has_qualified_type_exports
                || crate::modules::module_declares_exported_namespace(
                    resolved_index,
                    program_files,
                    &specifier.imported_name,
                )
            {
                continue;
            }

            emit_missing_import_member_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                resolved_index,
                program_files,
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            continue;
        }

        let mut found = has_qualified_type_exports;

        if let Some(type_export) = type_export {
            insert_type_export(
                type_declarations,
                &specifier.local_name,
                Some(&scope),
                type_export.clone(),
            );
            found = true;
        } else if let Some(local_declaration) = scope.get(&specifier.imported_name).cloned() {
            insert_type_export(
                type_declarations,
                &specifier.local_name,
                Some(&scope),
                local_declaration,
            );
            found = true;
        }

        if let Some(value_export) = value_export {
            symbols.insert_shared(specifier.local_name.clone(), value_export);
            record_type_only_alias(
                &export_table,
                &specifier.imported_name,
                &specifier.local_name,
                type_only_aliases,
            );
            found = true;
        }

        if !found
            && crate::modules::module_declares_exported_namespace(
                resolved_index,
                program_files,
                &specifier.imported_name,
            )
        {
            continue;
        }

        if !found {
            if has_unresolved_star_export {
                if *is_type_only {
                    insert_unknown_type_import(
                        type_declarations,
                        &specifier.local_name,
                        ctx.file_name_arc(),
                        specifier.name_span,
                    );
                    continue;
                }

                insert_unknown_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                insert_unknown_value_import(&specifier.local_name, symbols);
                continue;
            }

            if should_bind_unknown_for_missing_export(&export_table, resolved_index, program_files)
            {
                insert_unknown_type_import(
                    type_declarations,
                    &specifier.local_name,
                    ctx.file_name_arc(),
                    specifier.name_span,
                );
                insert_unknown_value_import(&specifier.local_name, symbols);
                continue;
            }

            emit_missing_import_member_diagnostic(
                ctx,
                &import.module_specifier,
                &specifier.imported_name,
                specifier.name_span,
                resolved_index,
                program_files,
            );
            insert_unknown_type_import(
                type_declarations,
                &specifier.local_name,
                ctx.file_name_arc(),
                specifier.name_span,
            );
            insert_unknown_value_import(&specifier.local_name, symbols);
        }
    }
    return;
}

/// Tags a namespace import object with tsc's `typeof import("<path>")` display
/// form. The path is the resolved module file made absolute and stripped of its
/// TypeScript extension (e.g. `…/pkg/index.d.ts` -> `…/pkg/index`).
pub(crate) fn tag_namespace_type_with_module_path(
    namespace_type: Type,
    resolved_file_name: &str,
) -> Type {
    match namespace_type {
        Type::Object(object) => {
            let path = strip_typescript_extension(resolved_file_name);
            Type::Object(object.with_alias_name(format!("typeof import(\"{path}\")")))
        }
        other => other,
    }
}

/// Strips a TypeScript source/declaration extension, matching the module name
/// tsc prints inside `typeof import(...)`. Declaration extensions are checked
/// first so `index.d.ts` becomes `index`, not `index.d`.
pub(crate) fn strip_typescript_extension(file_name: &str) -> &str {
    for extension in [".d.ts", ".d.mts", ".d.cts", ".ts", ".tsx", ".mts", ".cts"] {
        if let Some(stripped) = file_name.strip_suffix(extension) {
            return stripped;
        }
    }

    file_name
}

/// tsc's `resolveExternalModule` for an emittable import (`IsEmittableImport`:
/// one with an import clause that is not type-only) that resolves *by* its
/// written TypeScript extension: a `.d.*` path is TS2846 wherever it is
/// written, and any other TypeScript extension needs
/// `allowImportingTsExtensions` (TS5097), which a declaration file always has.
/// `getResolutionDiagnostic`: a specifier resolving to a `.tsx` program file
/// while `jsx` is unset is TS6142, which `resolveExternalModule` reports
/// before anything else, the file a module or not. A `.jsx` file is reported
/// on the JavaScript module path.
fn report_jsx_resolution(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    if ctx.options.jsx_configured || matches!(import.kind, ParsedImportKind::EntityAlias { .. }) {
        return;
    }
    let Some(file) = resolved_program_file_index(ctx, &import.module_specifier, program_files)
        .and_then(|index| program_files.get(index))
    else {
        return;
    };
    if !file.file_name.to_ascii_lowercase().ends_with(".tsx") {
        return;
    }
    let mut diagnostic = Diagnostic::ts6142(&import.module_specifier, &file.file_name, ctx.file_name.clone());
    if let Some(span) = import.module_specifier_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push(diagnostic);
}

fn report_ts_extension_import(
    import: &ParsedImportDeclaration,
    program_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    if import_is_type_only(&import.kind) || matches!(import.kind, ParsedImportKind::SideEffect) {
        return;
    }
    let Some(extension) = written_ts_extension(&import.module_specifier) else {
        return;
    };
    let declaration_specifier = is_declaration_file_name(&import.module_specifier);
    if !declaration_specifier
        && (ctx.options.allow_importing_ts_extensions || is_declaration_file_name(&ctx.file_name))
    {
        return;
    }
    let Some(resolved) = resolve_relative_module(
        &ctx.file_name,
        &import.module_specifier,
        program_files,
        &ctx.module_file_index_by_identity,
    ) else {
        return;
    };
    // tsc's `ResolvedUsingTsExtension`: `tryAddingExtensions` found the file
    // among the written extension's substitutes (`./a.d.ts` is `./a.ts`
    // first), not by appending extensions to the whole path or as a
    // directory, as a CommonJS-mode lookup can.
    let named_file = relative_specifier_path(&ctx.file_name, &import.module_specifier);
    let resolved_identity = canonical_file_identity(&resolved.resolved_file_name);
    let substituted = super::candidates::relative_import_candidates(&named_file, &import.module_specifier)
        .unwrap_or_default()
        .iter()
        .any(|candidate| canonical_file_identity(candidate) == resolved_identity);
    if !substituted {
        return;
    }
    let mut diagnostic = if declaration_specifier {
        let suggestion = suggested_import_source(&import.module_specifier, extension, ctx);
        Diagnostic::ts2846(suggestion, ctx.file_name.clone())
    } else {
        Diagnostic::ts5097(extension, ctx.file_name.clone())
    };
    if let Some(span) = import.module_specifier_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push(diagnostic);
}

/// tsc's `TryExtractTSExtension`: declaration extensions are tried first.
fn written_ts_extension(specifier: &str) -> Option<&'static str> {
    [".d.ts", ".d.cts", ".d.mts", ".ts", ".tsx", ".mts", ".cts"]
        .into_iter()
        .find(|extension| specifier.ends_with(extension))
}

/// tsc's `getSuggestedImportSource`: an ES module output imports the emitted
/// JavaScript (or, with `allowImportingTsExtensions`, the TypeScript source a
/// declaration specifier stands for); anything else imports extensionless.
fn suggested_import_source(specifier: &str, extension: &str, ctx: &CheckerContext) -> String {
    let stem = &specifier[..specifier.len() - extension.len()];
    if !ctx.options.module_emit.is_ecmascript() && !resolves_in_node_esm_mode(&ctx.file_name) {
        return stem.to_string();
    }
    let prefer_ts = is_declaration_file_name(specifier) && ctx.options.allow_importing_ts_extensions;
    let suffix = match extension {
        ".mts" | ".d.mts" => if prefer_ts { ".mts" } else { ".mjs" },
        ".cts" | ".d.cts" => if prefer_ts { ".cts" } else { ".cjs" },
        _ => if prefer_ts { ".ts" } else { ".js" },
    };
    format!("{stem}{suffix}")
}

/// How an import or export reaches the module it names, which picks the
/// error [`report_synchronous_import_of_esm`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleImportSyntax {
    /// `import x = require("…")` in a TypeScript file.
    ImportEquals,
    /// `import type … from "…"`.
    TypeOnlyImportClause,
    Other,
}

/// tsc's `resolveExternalModule` under `module: node16`/`node18`: an import
/// that compiles to `require` (any in a CommonJS file, and every
/// `import x = require()`) cannot load a file whose format is ESM, unless a
/// `resolution-mode` attribute says how to resolve it.
pub(crate) fn report_synchronous_import_of_esm(
    module_specifier: &str,
    module_specifier_span: Option<TextSpan>,
    syntax: ModuleImportSyntax,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
    has_resolution_mode_attribute: bool,
    program_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    if !matches!(
        ctx.options.module_emit,
        crate::context::ModuleEmitKind::Node16 | crate::context::ModuleEmitKind::Node18
    ) || has_resolution_mode_attribute
        || ctx.ambient_modules.contains_key(module_specifier)
    {
        return;
    }
    let synchronous = syntax == ModuleImportSyntax::ImportEquals
        || implied_format_for_emit(ctx, &ctx.file_name) == Some(ModuleFormat::CommonJs);
    if !synchronous {
        return;
    }
    let resolved = resolved_module_in_mode(ctx, &ctx.file_name, module_specifier, resolution_mode)
        .cloned()
        .or_else(|| {
            resolve_relative_module_in_mode(
                &ctx.file_name,
                module_specifier,
                resolution_mode,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .map(|resolution| resolution.resolved_file_name)
        });
    let Some(resolved) = resolved else {
        return;
    };
    if implied_format_for_emit(ctx, &resolved) != Some(ModuleFormat::Esm) {
        return;
    }
    // `sourceFile.Symbol`: only a module is imported at all. The index names
    // the whole program's file list, which a caller's slice need not be.
    let identity = canonical_file_identity(&resolved);
    let is_module = ctx
        .module_file_index_by_identity
        .get(identity.as_str())
        .and_then(|&index| program_files.get(index))
        .filter(|file| canonical_file_identity(&file.file_name) == identity)
        .is_none_or(|file| file.is_module);
    if !is_module {
        return;
    }
    let diagnostic = match syntax {
        ModuleImportSyntax::ImportEquals => Diagnostic::ts1471(module_specifier, ctx.file_name.clone()),
        ModuleImportSyntax::TypeOnlyImportClause => Diagnostic::ts1541(ctx.file_name.clone()),
        ModuleImportSyntax::Other => Diagnostic::ts1479(module_specifier, ctx.file_name.clone()),
    };
    ctx.push(match module_specifier_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

fn import_is_type_only(kind: &ParsedImportKind) -> bool {
    match kind {
        ParsedImportKind::Named { is_type_only, .. }
        | ParsedImportKind::DefaultAndNamed { is_type_only, .. }
        | ParsedImportKind::Namespace { is_type_only, .. }
        | ParsedImportKind::Equals { is_type_only, .. } => *is_type_only,
        ParsedImportKind::TypeOnlyDefault { .. } => true,
        _ => false,
    }
}
