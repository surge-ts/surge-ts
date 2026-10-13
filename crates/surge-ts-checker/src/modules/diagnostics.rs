//! Module-resolution diagnostic emitters and small structural predicates.

use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedImportDeclaration,
    ParsedImportKind, ParsedStatement, TextSpan,
};

use crate::context::{CheckerContext, FileKind, convert_span};
use crate::paths::{canonicalize_if_exists_string, normalize_path_string};
use crate::program::ParsedProgramFile;

/// Wraps a module specifier in double quotes so TS2305 matches tsc, which
/// renders the specifier from its source text and therefore keeps the quotes
/// (e.g. `Module '"./user"'`). If the specifier is already quoted it is left
/// untouched.
fn quoted_module_specifier(module_specifier: &str) -> String {
    if module_specifier.len() >= 2
        && ((module_specifier.starts_with('"') && module_specifier.ends_with('"'))
            || (module_specifier.starts_with('\'') && module_specifier.ends_with('\'')))
    {
        return module_specifier.to_string();
    }

    format!("\"{module_specifier}\"")
}

/// Diagnostic for an unresolved module specifier, mirroring tsc: a Node
/// built-in name gets the install-@types/node hint via
/// `cannot_resolve_module_name_error_for_specific_module`; anything else falls
/// back to the generic TS2307.
pub(crate) fn unresolved_module_diagnostic(ctx: &CheckerContext, module_specifier: &str) -> Diagnostic {
    unresolved_module_diagnostic_in_mode(ctx, module_specifier, None)
}

/// [`unresolved_module_diagnostic`] for a usage whose resolution mode can
/// differ from its file's: tsc keys the extension hint on the usage's mode.
fn unresolved_module_diagnostic_in_mode(
    ctx: &CheckerContext,
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
) -> Diagnostic {
    unresolved_module_resolution_diagnostic(ctx, module_specifier, resolution_mode)
        .or_else(|| cannot_resolve_module_name_error_for_specific_module(ctx, module_specifier))
        .unwrap_or_else(|| Diagnostic::ts2307(module_specifier, ctx.file_name.clone()))
}

/// What tsc's `resolveExternalModule` reports for an unresolved module ahead
/// of the not-found message its caller chose (TS2307, the Node built-in hint,
/// or a side-effect import's TS2882).
fn unresolved_module_resolution_diagnostic(
    ctx: &CheckerContext,
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
) -> Option<Diagnostic> {
    // A `.d.json.ts` beside a `.json` specifier resolves it whatever
    // `resolveJsonModule` says, so the extension is what is refused.
    if is_relative_specifier(module_specifier)
        && !arbitrary_extension_resolution_allowed(&ctx.file_name)
        && let Some(declaration) = arbitrary_extension_declaration_on_disk(ctx, module_specifier)
    {
        return Some(Diagnostic::ts6263(module_specifier, &declaration, ctx.file_name.clone()));
    }

    // A `.json` specifier under `resolveJsonModule: false` is not a missing
    // module — it is a module the option refuses to resolve, and tsc says so
    // with its own code and hint.
    if !ctx.options.resolve_json_module && surge_ts_syntax::is_json_file_name(module_specifier) {
        return Some(Diagnostic::ts2732(module_specifier, ctx.file_name.clone()));
    }

    if is_relative_specifier(module_specifier)
        && relative_resolution_is_esm(&ctx.file_name, resolution_mode)
        && is_extensionless_relative_specifier(module_specifier)
    {
        return Some(match suggested_import_extension(ctx, module_specifier) {
            Some(extension) => {
                Diagnostic::ts2835(&format!("{module_specifier}{extension}"), ctx.file_name.clone())
            }
            None => Diagnostic::ts2834(ctx.file_name.clone()),
        });
    }

    None
}

/// The `.d.{extension}.ts` file tsc resolves `./x.{extension}` to, when it
/// exists.
fn arbitrary_extension_declaration_on_disk(ctx: &CheckerContext, module_specifier: &str) -> Option<String> {
    let importer_dir = module_directory(&ctx.file_name);
    let specifier = normalize_path_string(module_specifier);
    let joined = if importer_dir.is_empty() {
        specifier.clone()
    } else {
        normalize_path_string(&format!("{importer_dir}/{specifier}"))
    };
    let declaration = arbitrary_extension_declaration(&joined, &specifier)?;
    Path::new(&declaration)
        .is_file()
        .then(|| canonicalize_if_exists_string(Path::new(&declaration)))
}

/// tsc's `getSuggestedImportExtension`: the output extension of the first
/// file on disk the extensionless path names directly (a directory's index
/// gets none), in tsc's probe order.
fn suggested_import_extension(ctx: &CheckerContext, module_specifier: &str) -> Option<&'static str> {
    let importer_dir = module_directory(&ctx.file_name);
    let specifier = normalize_path_string(module_specifier);
    let base = if importer_dir.is_empty() {
        specifier
    } else {
        normalize_path_string(&format!("{importer_dir}/{specifier}"))
    };
    [
        (".mts", ".mjs"),
        (".ts", ".js"),
        (".cts", ".cjs"),
        (".mjs", ".mjs"),
        (".js", ".js"),
        (".cjs", ".cjs"),
        // tsc suggests `.jsx` here under `jsx: preserve`, which the options
        // do not carry; only the message differs.
        (".tsx", ".js"),
        (".jsx", ".jsx"),
        (".json", ".json"),
    ]
    .into_iter()
    .find(|(source, _)| Path::new(&format!("{base}{source}")).is_file())
    .map(|(_, output)| output)
}

/// The JavaScript file tsc resolves `module_specifier` to when no TypeScript
/// or declaration file answers it; the loader records where the resolver's
/// JavaScript fallback lands, for packages and relative paths alike.
fn javascript_module_resolution(
    ctx: &CheckerContext,
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
) -> Option<String> {
    if is_relative_specifier(module_specifier)
        && relative_resolution_is_esm(&ctx.file_name, resolution_mode)
        && is_extensionless_relative_specifier(module_specifier)
    {
        return None;
    }
    let resolved = ctx
        .options
        .resolved_module_for(&ctx.file_name, module_specifier)?;
    let lower = resolved.to_ascii_lowercase();
    [".js", ".jsx", ".mjs", ".cjs"]
        .into_iter()
        .any(|extension| lower.ends_with(extension))
        .then(|| resolved.clone())
}

/// tsc's `resolveExternalModule` for a module that resolves to a JavaScript
/// file outside the program. The module exists, so it is never TS2307:
/// `GetResolutionDiagnostic` rejects a `.jsx` file while `jsx` is unset
/// (TS6142), and otherwise the module is untyped — TS7016 under
/// `noImplicitAny`, nothing without it. Under `allowJs`, tsc makes a
/// JavaScript file outside `node_modules` a program file, so its import
/// reports nothing either.
fn push_untyped_javascript_module_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    resolution_mode: Option<surge_ts_syntax::ResolutionModeOverride>,
    span: Option<TextSpan>,
) -> bool {
    let Some(resolved) = javascript_module_resolution(ctx, module_specifier, resolution_mode)
    else {
        return false;
    };
    let is_program_file_in_tsc = ctx.options.allow_js
        && !resolved.contains("/node_modules/")
        && !resolved.contains("\\node_modules\\");
    let diagnostic = if resolved.to_ascii_lowercase().ends_with(".jsx") && !ctx.options.jsx_configured
    {
        Some(Diagnostic::ts6142(module_specifier, &resolved, ctx.file_name.clone()))
    } else if ctx.options.no_implicit_any && !is_program_file_in_tsc {
        Some(Diagnostic::ts7016(module_specifier, &resolved, ctx.file_name.clone()))
    } else {
        None
    };
    if let Some(mut diagnostic) = diagnostic {
        if let Some(span) = span {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push(diagnostic);
    }
    true
}

/// tsc's `resolveExternalModuleName` for an `import("…")` a checked file
/// writes: a dynamic import (`checkImportCallExpression`), which resolves in
/// ESM mode (`getEmitSyntaxForUsageLocation`), or an import type
/// (`getTypeFromImportTypeNode`), which resolves in its file's. Reported at
/// the literal like a declaration's specifier: unresolved, or a script
/// (TS2306).
pub(crate) fn report_unresolved_import_call(
    ctx: &mut CheckerContext,
    import_call: &surge_ts_syntax::ParsedImportCall,
    program_files: &[ParsedProgramFile],
) {
    let specifier = import_call.specifier.as_str();
    if ambient_module_export_table(ctx, specifier).is_some() {
        return;
    }
    let resolution_mode = match import_call.kind {
        surge_ts_syntax::ParsedImportCallKind::Expression => {
            Some(surge_ts_syntax::ResolutionModeOverride::Import)
        }
        surge_ts_syntax::ParsedImportCallKind::Type | surge_ts_syntax::ParsedImportCallKind::Require => None,
    };
    // The loader resolved the specifier in the importer's mode, where an
    // extensionless relative path can still find a file ESM mode does not.
    let extensionless_esm = is_relative_specifier(specifier)
        && relative_resolution_is_esm(&ctx.file_name, resolution_mode)
        && is_extensionless_relative_specifier(specifier);
    let resolved_index = resolved_module_in_mode(ctx, &ctx.file_name, specifier, resolution_mode)
        .filter(|_| !extensionless_esm)
        .and_then(|resolved| {
            ctx.module_file_index_by_identity
                .get(canonical_file_identity(resolved).as_str())
                .copied()
        })
        .or_else(|| {
            let recorded_key = match resolution_mode {
                Some(mode) => crate::modules::candidates::resolution_mode_override_key(specifier, mode),
                None => specifier.to_string(),
            };
            if ctx.options.resolution_recorded(&ctx.file_name, &recorded_key) {
                return None;
            }
            resolve_relative_module_in_mode(
                &ctx.file_name,
                specifier,
                resolution_mode,
                program_files,
                &ctx.module_file_index_by_identity,
            )
            .map(|resolution| resolution.resolved_file_index)
        });
    if let Some(file) = resolved_index.and_then(|index| program_files.get(index)) {
        if (!file.is_module || file.jsdoc_import_module) && file.file_kind != FileKind::DependencyDeclaration {
            let diagnostic = Diagnostic::ts2306(&file.file_name, ctx.file_name.clone());
            ctx.push(diagnostic.with_span(convert_span(import_call.specifier_span)));
        } else if import_call.kind == surge_ts_syntax::ParsedImportCallKind::Type {
            crate::modules::imports::report_synchronous_import_of_esm(
                specifier,
                Some(import_call.specifier_span),
                crate::modules::imports::ModuleImportSyntax::ImportType,
                None,
                false,
                program_files,
                ctx,
            );
        }
        return;
    }
    record_unresolved_external_module(ctx, specifier);
    if ctx.options.stub_external_modules && is_external_specifier(specifier) {
        return;
    }
    if push_untyped_javascript_module_diagnostic(
        ctx,
        specifier,
        resolution_mode,
        Some(import_call.specifier_span),
    ) {
        return;
    }
    let diagnostic = unresolved_module_diagnostic_in_mode(ctx, specifier, resolution_mode);
    ctx.push(diagnostic.with_span(convert_span(import_call.specifier_span)));
}

/// The unresolved-module diagnostic for a specifier the program never
/// resolved, with no untyped-JavaScript reading of it.
pub(crate) fn emit_unresolvable_module_reference(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    span: Option<TextSpan>,
) {
    let mut diagnostic = unresolved_module_diagnostic(ctx, module_specifier);
    if let Some(span) = span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push(diagnostic);
}

pub(crate) fn emit_unresolved_export_module_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    module_specifier_span: Option<TextSpan>,
) {
    if push_untyped_javascript_module_diagnostic(ctx, module_specifier, None, module_specifier_span)
    {
        return;
    }
    let mut diagnostic = unresolved_module_diagnostic(ctx, module_specifier);

    if let Some(span) = module_specifier_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push(diagnostic);
}

pub(crate) fn emit_unresolved_module_diagnostic(
    ctx: &mut CheckerContext,
    import: &ParsedImportDeclaration,
) {
    // Go resolves a side-effect import only under `noUncheckedSideEffectImports`.
    if matches!(import.kind, ParsedImportKind::SideEffect)
        && !ctx.options.no_unchecked_side_effect_imports
    {
        return;
    }
    let resolution_mode = import_resolution_mode(import);
    if !matches!(import.kind, ParsedImportKind::SideEffect)
        && push_untyped_javascript_module_diagnostic(
            ctx,
            &import.module_specifier,
            resolution_mode,
            import.module_specifier_span.or(import.span),
        )
    {
        return;
    }
    let mut diagnostic = match &import.kind {
        ParsedImportKind::SideEffect => {
            unresolved_module_resolution_diagnostic(ctx, &import.module_specifier, resolution_mode)
                .unwrap_or_else(|| {
                    Diagnostic::ts2882(&import.module_specifier, ctx.file_name.clone())
                })
        }
        _ => unresolved_module_diagnostic_in_mode(ctx, &import.module_specifier, resolution_mode),
    };

    if let Some(span) = import.module_specifier_span.or(import.span) {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push(diagnostic);
}

thread_local! {
    // Whether a module has any `export * from "X"` whose target does not resolve.
    // This depends only on the (fixed-per-run) file set, but the import/export
    // binding fixpoint queries it once per importing specifier across several
    // passes. A barrel that re-exports N modules is imported by O(N) files, so the
    // uncached scan (O(re-exports) per query, each rebuilding candidate paths and
    // probing them) made the binding phase O(N^2). Keyed by the global file index;
    // cleared per run alongside the relative-module cache.
    static STAR_EXPORT_UNRESOLVED_CACHE: RefCell<HashMap<usize, bool>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn clear_star_export_unresolved_cache() {
    STAR_EXPORT_UNRESOLVED_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub(crate) fn module_has_unresolved_star_export(
    file_index: usize,
    parsed_files: &[ParsedProgramFile],
    file_index_by_identity: &surge_ts_types::fx::FxHashMap<Arc<str>, usize>,
) -> bool {
    if let Some(cached) =
        STAR_EXPORT_UNRESOLVED_CACHE.with(|cache| cache.borrow().get(&file_index).copied())
    {
        return cached;
    }

    let result = parsed_files[file_index].statements.iter().any(|statement| {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            return false;
        };
        let ParsedExportDeclaration::All {
            module_specifier, ..
        } = export.as_ref()
        else {
            return false;
        };

        !crate::modules::resolution::is_relative_specifier(module_specifier)
            && resolve_relative_module(
                &parsed_files[file_index].file_name,
                module_specifier,
                parsed_files,
                file_index_by_identity,
            )
            .is_none()
    });

    STAR_EXPORT_UNRESOLVED_CACHE.with(|cache| {
        cache.borrow_mut().insert(file_index, result);
    });
    result
}

pub(crate) fn should_bind_unknown_for_missing_export(
    export_table: &ModuleExportTable,
    resolved_index: Option<usize>,
    parsed_files: &[ParsedProgramFile],
) -> bool {
    let Some(file_index) = resolved_index else {
        return false;
    };
    let Some(file) = parsed_files.get(file_index) else {
        return false;
    };

    // A `.json` file whose contents did not parse is a module whose export
    // names are unknowable, not one that is missing the name asked for. surge
    // reports no JSON syntax errors, so the alternative would be a TS2305 on
    // every import of it.
    if matches!(
        file.json_module_type,
        Some(surge_ts_syntax::ParsedType::Unknown)
    ) {
        return true;
    }

    matches!(file.file_kind, FileKind::DependencyDeclaration)
        && export_table.has_incomplete_declaration_surface
}

pub(crate) fn module_has_incomplete_declaration_surface(parsed_file: &ParsedProgramFile) -> bool {
    if !parsed_file.file_kind.is_declaration() {
        return false;
    }

    parsed_file
        .statements
        .iter()
        .any(statement_has_unsupported_declaration_surface)
}

pub(crate) fn statement_has_unsupported_declaration_surface(statement: &ParsedStatement) -> bool {
    match statement {
        ParsedStatement::UnsupportedDeclaration { .. } => true,
        ParsedStatement::ImportDeclaration(import) => {
            matches!(
                import.kind,
                ParsedImportKind::Unsupported | ParsedImportKind::TypeOnlyDefault { .. }
            )
        }
        ParsedStatement::ExportDeclaration(export) => matches!(
            export.as_ref(),
            ParsedExportDeclaration::Unsupported { .. }
                | ParsedExportDeclaration::EqualsExpression { .. }
                | ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Unsupported { .. },
                    ..
                }
        ),
        ParsedStatement::DeclareModuleDeclaration(module) => module
            .statements
            .iter()
            .any(statement_has_unsupported_declaration_surface),
        _ => false,
    }
}

pub(crate) fn emit_unsupported_module_syntax_diagnostic(
    ctx: &mut CheckerContext,
    import: &ParsedImportDeclaration,
) {
    let mut diagnostic = Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

    if let Some(span) = import.span.or(import.module_specifier_span) {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push(diagnostic);
}

pub(crate) fn emit_missing_export_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    export_name: &str,
    name_span: Option<TextSpan>,
) {
    // tsc renders the module specifier from its source text, which keeps the
    // surrounding quotes (e.g. Module '"./user"'). The checker stores it
    // unquoted, so re-wrap it here.
    let specifier = quoted_module_specifier(module_specifier);
    let mut diagnostic = Diagnostic::ts2305(specifier, export_name, ctx.file_name.clone());

    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push(diagnostic);
}

/// An import naming something the resolved source module declares at its top
/// level without exporting it (TS2459, or TS2614 when the module has a default
/// export). Decided from the module's syntax only when that is conclusive: a
/// source file whose export list has no `export *`, `export =` or form surge
/// does not parse. surge's export tables also carry a module's local values, so
/// without this the import silently bound the local.
pub(crate) fn imported_name_is_unexported_local(
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
    name: &str,
) -> bool {
    let Some(exported) = syntactic_export_names(resolved_index, program_files) else {
        return false;
    };
    let Some(file) = resolved_index.and_then(|index| program_files.get(index)) else {
        return false;
    };
    !exported.contains(&name)
        && crate::program::module_scope_declared_names(&file.statements).contains(name)
}

/// The names a TypeScript source module exports, in declaration order, when
/// its syntax says so conclusively: no `export *`, `export =` or export form
/// surge does not parse.
pub(crate) fn syntactic_export_names(
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
) -> Option<Vec<&str>> {
    let file = resolved_index.and_then(|index| program_files.get(index))?;
    if file.file_kind.is_declaration()
        || ![".ts", ".tsx", ".mts", ".cts"]
            .iter()
            .any(|extension| file.file_name.ends_with(extension))
    {
        return None;
    }
    let mut exported: Vec<&str> = Vec::new();
    for statement in &file.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                let mut names: Vec<&str> = crate::program::module_scope_declared_names(
                    std::slice::from_ref(declaration.as_ref()),
                )
                .into_iter()
                .collect();
                names.sort_unstable();
                exported.extend(names);
            }
            ParsedExportDeclaration::Named { specifiers, .. } => {
                exported.extend(specifiers.iter().map(|specifier| specifier.exported_name.as_str()));
            }
            ParsedExportDeclaration::Default { .. } => exported.push("default"),
            ParsedExportDeclaration::Namespace { exported_name, .. } => {
                exported.push(exported_name.as_str());
            }
            ParsedExportDeclaration::Empty { .. } => {}
            ParsedExportDeclaration::All { .. }
            | ParsedExportDeclaration::NamespaceExport { .. }
            | ParsedExportDeclaration::Equals { .. }
            | ParsedExportDeclaration::EqualsExpression { .. }
            | ParsedExportDeclaration::Unsupported { .. } => return None,
        }
    }
    Some(exported)
}

/// Whether the module file exports a namespace `name` by declaration
/// (`export namespace N {}`). The binder declares that export whatever the
/// body holds, while surge's tables only carry a namespace's members, so an
/// empty one leaves no trace there.
pub(crate) fn module_declares_exported_namespace(
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
    name: &str,
) -> bool {
    let Some(file) = resolved_index.and_then(|index| program_files.get(index)) else {
        return false;
    };
    file.statements.iter().any(|statement| match statement {
        ParsedStatement::ExportDeclaration(export) => matches!(
            export.as_ref(),
            ParsedExportDeclaration::Statement { declaration, .. }
                if matches!(declaration.as_ref(), ParsedStatement::NamespaceDeclaration(namespace)
                    if namespace.name.split('.').next() == Some(name))
        ),
        _ => false,
    })
}

/// tsc's `errorNoModuleMemberSymbol` for a named import the module does not
/// export: the closest exported name (TS2724), else TS2614 when the module
/// has a default export, else TS2305.
pub(crate) fn emit_missing_import_member_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    name: &str,
    name_span: Option<TextSpan>,
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
) {
    let suggestion = syntactic_export_names(resolved_index, program_files).and_then(|exported| {
        crate::checks::expr::spelling_suggestion(
            name,
            exported.into_iter().filter(|export| *export != "default"),
            0,
        )
        .map(str::to_string)
    });
    let Some(suggestion) = suggestion else {
        emit_missing_named_import_diagnostic(
            ctx,
            module_specifier,
            name,
            name_span,
            module_has_explicit_default_export(module_specifier, resolved_index, program_files, ctx),
        );
        return;
    };
    let diagnostic = Diagnostic::ts2724(
        quoted_module_specifier(module_specifier),
        name,
        suggestion,
        ctx.file_name.clone(),
    );
    ctx.push(match name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// tsc's `reportInvalidImportEqualsExportMember`: a named import of the very
/// local a module assigns with `export =` (`class Foo {}; export = Foo`) can
/// only be reached through a default import, `require`, or `import = require`.
pub(crate) fn emit_export_equals_member_import_diagnostic(
    ctx: &mut CheckerContext,
    export_table: &crate::modules::ModuleExportTable,
    module_specifier: &str,
    name: &str,
    name_span: Option<TextSpan>,
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
) -> bool {
    let Some(file) = resolved_index.and_then(|index| program_files.get(index)) else {
        return false;
    };
    // A namespace merged into the assigned target may export a member of the
    // same name, which the import then reaches.
    if file.file_kind.is_declaration()
        || export_table.symbols.get(&format!("{name}.{name}")).is_some()
    {
        return false;
    }
    let assigns_name = file.statements.iter().any(|statement| {
        matches!(statement, ParsedStatement::ExportDeclaration(export)
            if matches!(export.as_ref(), ParsedExportDeclaration::Equals { exported_name, .. } if exported_name == name))
    });
    if !assigns_name || !crate::program::module_scope_declared_names(&file.statements).contains(name) {
        return false;
    }
    let file_name = ctx.file_name.clone();
    let diagnostic = if !matches!(ctx.options.module_emit, crate::context::ModuleEmitKind::CommonJS) {
        Diagnostic::ts2595(name, file_name)
    } else if surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        Diagnostic::ts2597(name, file_name)
    } else {
        Diagnostic::ts2616(name, name, quoted_module_specifier(module_specifier), file_name)
    };
    ctx.push(match name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    true
}

pub(crate) fn emit_unexported_local_import_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    name: &str,
    name_span: Option<TextSpan>,
    has_explicit_default_export: bool,
) {
    let specifier = quoted_module_specifier(module_specifier);
    let diagnostic = if has_explicit_default_export {
        Diagnostic::ts2614(specifier, name, ctx.file_name.clone())
    } else {
        Diagnostic::ts2459(specifier, name, ctx.file_name.clone())
    };
    ctx.push(match name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// tsc's `reportNonDefaultExport` for a default import from a module with no
/// default export: TS2613 when the module exports a member of the imported
/// name, TS1192 otherwise, naming the module by its resolved file (without
/// extension) when it is known. A declaration file can always be imported
/// synthetically (`canHaveSyntheticDefault`), so importing from one, or from
/// an ambient module surge has no file for, is not an error.
pub(crate) fn emit_no_default_export_diagnostic(
    ctx: &mut CheckerContext,
    local_name: &str,
    name_span: Option<TextSpan>,
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
) {
    let Some(file) = resolved_index.and_then(|index| program_files.get(index)) else {
        return;
    };
    let path = file.file_name.as_str();
    let stem = [".d.ts", ".d.mts", ".d.cts", ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx"]
        .iter()
        .find_map(|extension| path.strip_suffix(extension))
        .unwrap_or(path);
    let module_name = format!("\"{stem}\"");
    let exports_local_name = syntactic_export_names(resolved_index, program_files)
        .is_some_and(|names| names.contains(&local_name));
    let diagnostic = if exports_local_name {
        // The catalog has no brace escape, so the import's braces ride in the
        // argument.
        Diagnostic::ts2613(&module_name, format!("{{ {local_name} }}"), ctx.file_name.clone())
    } else {
        Diagnostic::ts1192(module_name, ctx.file_name.clone())
    };
    ctx.push(match name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

pub(crate) fn emit_missing_named_import_diagnostic(
    ctx: &mut CheckerContext,
    module_specifier: &str,
    export_name: &str,
    name_span: Option<TextSpan>,
    has_explicit_default_export: bool,
) {
    // See emit_missing_export_diagnostic: tsc keeps the specifier's quotes.
    let specifier = quoted_module_specifier(module_specifier);
    let mut diagnostic = if has_explicit_default_export {
        Diagnostic::ts2614(specifier, export_name, ctx.file_name.clone())
    } else {
        Diagnostic::ts2305(specifier, export_name, ctx.file_name.clone())
    };

    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push(diagnostic);
}

pub(crate) fn module_has_explicit_default_export(
    module_specifier: &str,
    resolved_index: Option<usize>,
    program_files: &[ParsedProgramFile],
    ctx: &CheckerContext,
) -> bool {
    if program_files.iter().any(|file| {
        file.file_kind == FileKind::DependencyDeclaration
            && file.file_name.contains(module_specifier)
            && file.has_export_default
    }) {
        return true;
    }

    if let Some(index) = resolved_index
        && program_files
            .get(index)
            .is_some_and(|file| file_has_explicit_default_export(file))
    {
        return true;
    }

    let Some(resolved_file_name) = ctx
        .options
        .resolved_module_for(&ctx.file_name, module_specifier)
    else {
        return false;
    };

    let canonical_file_name = canonicalize_if_exists_string(Path::new(resolved_file_name));
    program_files
        .iter()
        .find(|file| file.file_name == canonical_file_name)
        .is_some_and(|file| file_has_explicit_default_export(file))
}

pub(crate) fn file_has_explicit_default_export(file: &ParsedProgramFile) -> bool {
    file.has_export_default
}

/// Whether `export { name }` would resolve `name` in the global scope, or name
/// a primitive type.
pub(crate) fn is_global_scope_name(ctx: &CheckerContext, name: &str) -> bool {
    matches!(
        name,
        "any" | "string" | "number" | "boolean" | "never" | "unknown" | "undefined" | "globalThis"
    ) || ctx.ambient_global_symbols.get(name).is_some()
        || ctx.ambient_global_type_declarations.get(name).is_some()
        || ctx.namespace_registry.is_global(name)
}

/// tsc's `checkExportSpecifier` on `export { x }` with no module specifier:
/// a name that resolves to a global-scope declaration (a lib or script
/// global, `undefined`, `globalThis`) is TS2661, as is a primitive type name
/// (`checkAndReportErrorForExportingPrimitiveType`); otherwise the name is
/// simply not found.
pub(crate) fn push_unresolved_export_diagnostic(
    ctx: &mut CheckerContext,
    local_name: &str,
    name_span: Option<TextSpan>,
) {
    let mut diagnostic = if is_global_scope_name(ctx, local_name) {
        Diagnostic::ts2661(local_name, ctx.file_name.clone())
    } else {
        Diagnostic::ts2304(local_name, ctx.file_name.clone())
    };

    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    // `CheckerContext::push` already dedups by (code, file, message, span) via
    // its O(1) key index, so no pre-scan of the accumulated diagnostics is
    // needed here.
    ctx.push(diagnostic);
}
