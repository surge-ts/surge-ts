use super::*;
use surge_ts_syntax::ParsedExpression;

/// The shape [`apply_merging_namespace_value_members`] leaves behind when a
/// namespace merges into a function: an object carrying the function's call
/// signature plus the namespace's value members.
fn namespace_merged_callable(ty: &Type) -> bool {
    matches!(ty, Type::Object(object) if object.call_signature.is_some())
}

pub(crate) fn collect_exports_from_statement(
    statement: &ParsedStatement,
    exportable_values: &SymbolTable,
    imported_symbols: &SymbolTable,
    imported_names: &std::collections::HashSet<&str>,
    statement_list: &[ParsedStatement],
    parenthesized_expressions: &[surge_ts_syntax::ParenthesizedExpressionSpan],
    local_type_declarations: &TypeDeclarationTable,
    local_symbols: &SymbolTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
    symbols: &mut SymbolTable,
    default_symbol: &mut Option<Arc<SymbolInfo>>,
    export_assignment_symbol: &mut Option<Arc<SymbolInfo>>,
    type_only_exports: &mut surge_ts_types::fx::FxHashMap<Arc<str>, TypeOnlyAliasKind>,
    ctx: &mut CheckerContext,
) {
    if let ParsedStatement::ExportDeclaration(export) = statement
        && matches!(
            export.as_ref(),
            ParsedExportDeclaration::Equals { .. } | ParsedExportDeclaration::EqualsExpression { .. }
        )
        && surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
    {
        publish_commonjs_type_exports(
            statement_list,
            local_type_declarations,
            resolution_scope,
            type_declarations,
        );
    }
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            // `export import local = N.M` exports every meaning of the entity.
            ParsedExportDeclaration::Statement { declaration, .. }
                if let ParsedStatement::ImportDeclaration(import) = declaration.as_ref()
                    && let surge_ts_syntax::ParsedImportKind::EntityAlias {
                        local_name,
                        target,
                        ..
                    } = &import.kind =>
            {
                let value = entity_alias_value(target, exportable_values, imported_symbols, ctx);
                let has_value = value.is_some();
                if let Some(symbol) = value
                    && symbols.get(local_name).is_none()
                {
                    symbols.insert_shared(local_name.clone(), symbol);
                }
                let types_before = type_declarations.len();
                export_entity_alias_types(
                    target,
                    local_name,
                    local_type_declarations,
                    resolution_scope,
                    type_declarations,
                    ctx,
                );
                // The alias is an export whether or not its entity resolves
                // (Go binds it and resolves it to `unknownSymbol`): an importer
                // of it reports nothing, only the alias's own entity does.
                if !has_value && type_declarations.len() == types_before && symbols.get(local_name).is_none() {
                    insert_unknown_value_import(local_name, symbols);
                    insert_unknown_type_import(type_declarations, local_name, ctx.file_name_arc(), import.span);
                }
            }
            ParsedExportDeclaration::Statement { declaration, .. } => {
                collect_exports_from_statement(
                    declaration.as_ref(),
                    exportable_values,
                    imported_symbols,
                    imported_names,
                    statement_list,
                    parenthesized_expressions,
                    local_type_declarations,
                    local_symbols,
                    resolution_scope,
                    type_declarations,
                    symbols,
                    default_symbol,
                    export_assignment_symbol,
                    type_only_exports,
                    ctx,
                )
            }
            ParsedExportDeclaration::Equals { exported_name, .. } => {
                // `export = identifier` binds the module's single export-assignment
                // value to the named local value symbol. An unresolved target binds
                // nothing and emits no diagnostic here, leaving consumers to bind an
                // unknown placeholder rather than cascade (`import x = require(...)`).
                if let Some(symbol) = exportable_values.get_shared(exported_name) {
                    *export_assignment_symbol = Some(symbol);
                }

                // `export = Emitter` makes the target the module's default for an
                // importer too, type side included — `import Emitter from "events"`
                // then `class X extends Emitter<T>` has to resolve `Emitter` as a
                // type. The default import looks the type up under `default`, which
                // only a literal `export default` would otherwise populate.
                if let Some(declaration) = local_type_declarations.get(exported_name)
                    && type_declarations.get("default").is_none()
                {
                    let _ = type_declarations.insert("default", declaration.clone());
                }

                // When the export target is a `declare namespace <name>`, its type
                // members were collected under `<name>.<member>` keys. Carry them into
                // the export table so a namespace import (`import * as React`) can
                // re-expose them as qualified types (`React.ComponentProps<...>`).
                let prefix = format!("{exported_name}.");
                // The value twin of the qualified type carry below: a namespace
                // member published under `<name>.<member>` holds the member's
                // real signature, while the namespace object only models the
                // member *set* with permissive `any`. Without this, every
                // `import { useState } from "react"` bound React's permissive
                // member and lost the hook's return type.
                for (key, symbol) in exportable_values.iter_shared() {
                    let Some(member_name) = key.strip_prefix(prefix.as_str()) else {
                        continue;
                    };
                    if symbols.get_own_shared(key.as_ref()).is_none() {
                        symbols.insert_shared(key.to_string(), symbol.clone());
                    }
                    if symbols.get_own_shared(member_name).is_none() {
                        symbols.insert_shared(member_name.to_string(), symbol.clone());
                    }
                }
                for (key, declaration) in local_type_declarations.iter() {
                    if key.starts_with(&prefix) {
                        let _ = type_declarations.insert(key.as_ref(), declaration.clone());
                        let exported_member_name = &key[prefix.len()..];
                        let _ = type_declarations.insert(
                            exported_member_name,
                            rename_type_declaration(
                                attach_type_resolution_scope_if_missing(
                                    declaration.clone(),
                                    resolution_scope,
                                ),
                                exported_member_name.to_string(),
                            ),
                        );
                    }
                }
                publish_export_assignment_types(
                    exported_name,
                    local_type_declarations,
                    resolution_scope,
                    type_declarations,
                    ctx,
                );
            }
            ParsedExportDeclaration::EqualsExpression {
                entity_name: Some(entity_name),
                ..
            } => {
                // `getTargetOfAliasLikeExpression`: a dotted entity name is an
                // alias of the entity, resolved with every meaning.
                if let Some(symbol) =
                    entity_alias_value(entity_name, exportable_values, imported_symbols, ctx)
                {
                    *export_assignment_symbol = Some(symbol);
                }
                publish_export_assignment_types(
                    entity_name,
                    local_type_declarations,
                    resolution_scope,
                    type_declarations,
                    ctx,
                );
            }
            ParsedExportDeclaration::EqualsExpression {
                expression,
                entity_name: None,
                ..
            } => {
                // Any other expression is bound as a property, not an alias
                // (`bindExportAssignment`): the module's export is the value of
                // the expression and nothing else. A generic function is typed
                // as its check types it, with its type parameters in scope;
                // the inference sketch leaves its parameters untyped.
                let inferred = match expression.as_ref() {
                    surge_ts_syntax::ParsedExpression::ArrowFunction(function)
                        if !function.type_parameters.is_empty() =>
                    {
                        let diagnostics_before = ctx.diagnostics().len();
                        let evaluated =
                            crate::checks::expr::evaluate_expression(expression, None, exportable_values, ctx);
                        ctx.truncate_diagnostics(diagnostics_before);
                        evaluated
                    }
                    _ => crate::infer::infer_expression(expression, exportable_values, ctx),
                };
                let ty = match inferred {
                    crate::infer::InferredExpression::Known(ty) => ty,
                    crate::infer::InferredExpression::Unknown
                    | crate::infer::InferredExpression::UnresolvedIdentifier { .. }
                    | crate::infer::InferredExpression::MissingProperty { .. } => Type::Unknown,
                };
                *export_assignment_symbol = Some(Arc::new(SymbolInfo {
                    ty,
                    kind: SymbolKind::Const,
                    function_signature: None,
                }));
            }
            ParsedExportDeclaration::Named {
                is_type_only,
                specifiers,
                module_specifier,
                ..
            } => {
                if module_specifier.is_some() {
                    return;
                }

                for specifier in specifiers {
                    // tsc's `checkExportSpecifier` resolves every meaning of the
                    // name; `type` only keeps an importer from reading a value
                    // through it, so a type-only specifier naming a value is
                    // resolved as any other and its value marked.
                    let specifier_is_type_only = *is_type_only || specifier.is_type_only;

                    let mut found = false;

                    // Imported types are not in `local_type_declarations`; the
                    // resolution scope's import layers carry them. Local-first
                    // layer order keeps the own-declaration case unchanged.
                    if let Some(handle) = local_type_declarations
                        .get_handle(&specifier.local_name)
                        .or_else(|| {
                            resolution_scope
                                .and_then(|scope| scope.get_handle(&specifier.local_name))
                        })
                    {
                        export_local_type_declaration(
                            handle.get(),
                            &specifier.exported_name,
                            resolution_scope,
                            type_declarations,
                        );
                        found = true;
                    }

                    // The module's own values only: a global reached through the
                    // table's ambient fallback is not a local to export.
                    if let Some(symbol) = exportable_values
                        .get_own_shared(&specifier.local_name)
                        .or_else(|| imported_symbols.get_shared(&specifier.local_name))
                    {
                        if specifier.exported_name == "default" {
                            // `export { x as default }` — consumers read the
                            // default through `default_symbol`, never `symbols`.
                            if default_symbol.is_none() {
                                *default_symbol = Some(symbol);
                            }
                        } else if symbols.get(&specifier.exported_name).is_none() {
                            symbols.insert_shared(specifier.exported_name.clone(), symbol);
                        }
                        if specifier_is_type_only {
                            type_only_exports
                                .entry(Arc::from(specifier.exported_name.as_str()))
                                .or_insert(TypeOnlyAliasKind::Export);
                        } else if exportable_values.get_own_shared(&specifier.local_name).is_none()
                            && bound_by_type_only_import(statement_list, &specifier.local_name)
                        {
                            // `import type { C }; export { C as D }`: the alias
                            // chain passes through the type-only import.
                            type_only_exports
                                .entry(Arc::from(specifier.exported_name.as_str()))
                                .or_insert(TypeOnlyAliasKind::Import);
                        }
                        found = true;
                    }

                    // A namespace is exportable by its namespace meaning alone
                    // (`export { TypesOnly }`); its type members travel under
                    // the exported name.
                    if ctx
                        .namespace_registry
                        .local_info(&ctx.file_name, &specifier.local_name)
                        .is_some_and(|info| !info.is_enum)
                    {
                        export_namespace_type_members(
                            &specifier.local_name,
                            &specifier.exported_name,
                            local_type_declarations,
                            type_declarations,
                        );
                        found = true;
                    }

                    // `import local = N.M; export { local }` exports every
                    // meaning of the entity the alias names.
                    if !found
                        && let Some(target) = entity_alias_target(statement_list, &specifier.local_name)
                    {
                        if let Some(symbol) =
                            entity_alias_value(target, exportable_values, imported_symbols, ctx)
                        {
                            if specifier.exported_name == "default" {
                                if default_symbol.is_none() {
                                    *default_symbol = Some(symbol);
                                }
                            } else if symbols.get(&specifier.exported_name).is_none() {
                                symbols.insert_shared(specifier.exported_name.clone(), symbol);
                            }
                        }
                        export_entity_alias_types(
                            target,
                            &specifier.exported_name,
                            local_type_declarations,
                            resolution_scope,
                            type_declarations,
                            ctx,
                        );
                        found = true;
                    }

                    // An import binding is a local of the module whatever its
                    // target's meanings are (a namespace-only import has no
                    // value or type entry of its own).
                    if !found && imported_names.contains(specifier.local_name.as_str()) {
                        found = true;
                    }

                    // A global only an augmentation declares is declared in that
                    // block, not in a global source file, so the specifier is a
                    // local export of it (`@types/node` re-exports its buffer
                    // aliases from `node:buffer` this way).
                    if !found && ctx.global_augmentation_only_names.contains(specifier.local_name.as_str()) {
                        if let Some(handle) = ctx.lookup_type_declaration_handle(&specifier.local_name) {
                            export_local_type_declaration(
                                handle.get(),
                                &specifier.exported_name,
                                resolution_scope,
                                type_declarations,
                            );
                        }
                        if let Some(symbol) = ctx.ambient_global_symbols.get_shared(&specifier.local_name) {
                            if specifier.exported_name == "default" {
                                if default_symbol.is_none() {
                                    *default_symbol = Some(symbol);
                                }
                            } else if symbols.get(&specifier.exported_name).is_none() {
                                symbols.insert_shared(specifier.exported_name.clone(), symbol);
                            }
                            if specifier_is_type_only {
                                type_only_exports
                                    .entry(Arc::from(specifier.exported_name.as_str()))
                                    .or_insert(TypeOnlyAliasKind::Export);
                            }
                        }
                        found = true;
                    }

                    if !found {
                        // tsc's `checkExportSpecifier`: a name that resolves to
                        // `undefined`, `globalThis`, or a global declaration is
                        // not a local to export (TS2661), and a primitive type
                        // name that resolves to nothing is reported the same way
                        // (`checkAndReportErrorForExportingPrimitiveType`); only
                        // any other name that resolves to nothing is TS2304.
                        let name = specifier.local_name.as_str();
                        let global = matches!(name, "undefined" | "globalThis")
                            || crate::checks::expr::is_primitive_type_name(name)
                            || ctx.ambient_global_symbols.get(name).is_some()
                            || ctx.lookup_type_declaration_handle(name).is_some()
                            || ctx.namespace_registry.is_global(name);
                        if global {
                            let mut diagnostic =
                                surge_ts_diagnostics::Diagnostic::ts2661(&specifier.local_name, ctx.file_name.clone());
                            if let Some(span) = specifier.name_span {
                                diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                            }
                            ctx.push(diagnostic);
                        } else {
                            push_unresolved_export_diagnostic(
                                ctx,
                                &specifier.local_name,
                                specifier.name_span,
                            );
                        }
                    }
                }
            }
            ParsedExportDeclaration::Default {
                declaration,
                span: _,
            } => match declaration {
                ParsedDefaultExportDeclaration::Function(function) => {
                    if default_symbol.is_some() {
                        // A second default export is reported once, as TS2528,
                        // by the parser's grammar walk; the module pass only
                        // keeps the first symbol.
                    } else if let Some(symbol) = exportable_values
                        .get_own_shared(&function.name)
                        .filter(|symbol| matches!(&symbol.ty, Type::Object(object) if object.call_signature().is_some()))
                    {
                        // The default export is the function's own symbol, with
                        // the members expando writes declared on it.
                        *default_symbol = Some(symbol);
                    } else {
                        let mut signature_symbols =
                            exportable_values.clone_with_reason(TypeCopyReason::ModuleExport);
                        // Only the export's type is wanted here, as for a named
                        // function's signature (`analyze_module`): a local type
                        // the signature names resolves under its preliminary
                        // scope, which holds no imports, and the check phase
                        // reports the signature with them bound.
                        let checkpoint = ctx.diagnostics().len();
                        let mut function_type =
                            check_function::collect_function_declaration_signature(
                                function,
                                &mut signature_symbols,
                                ctx,
                                false,
                            );
                        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                        if let Some(value_type) =
                            promise_value_type(&function.return_type, resolution_scope, ctx)
                        {
                            function_type = FunctionType::new(
                                function_type.parameters().to_vec(),
                                value_type,
                                function_type.is_variadic(),
                                function_type.required_parameter_count(),
                            );
                        }
                        *default_symbol = Some(Arc::new(SymbolInfo {
                            ty: Type::Function(function_type),
                            kind: SymbolKind::Function,
                            function_signature: None,
                        }));
                    }
                }
                ParsedDefaultExportDeclaration::Class(class) => {
                    publish_default_type_export(
                        &class.name,
                        local_type_declarations,
                        resolution_scope,
                        type_declarations,
                        ctx,
                    );
                    if let Some(symbol) = local_symbols.get_shared(&class.name) {
                        if default_symbol.is_none() {
                            *default_symbol = Some(symbol);
                        }
                    }
                }
                ParsedDefaultExportDeclaration::Expression(expression) => {
                    if default_symbol.is_some() {
                        return;
                    }

                    // `export default Dispatcher` re-exports the named
                    // declaration's type side too, not just its value
                    // (`getTargetOfExportAssignment`: an entity name is an
                    // alias of every meaning it has).
                    let entity_name = entity_name_expression(expression, parenthesized_expressions);
                    if let Some(entity_name) = &entity_name {
                        publish_default_type_export(
                            entity_name,
                            local_type_declarations,
                            resolution_scope,
                            type_declarations,
                            ctx,
                        );
                    }
                    // An imported name shadows a global of its name: next's
                    // `import Error from "./_error"; export default Error` is
                    // that class, not the global `Error`. The exportable
                    // values fall back to the globals, so own-ness is read
                    // from their own map.
                    if let Some(name) = entity_name.as_deref().filter(|name| !name.contains('.'))
                        && exportable_values.get_own_shared(name).is_none()
                        && let Some(symbol) = imported_symbols.get_own_shared(name)
                    {
                        *default_symbol = Some(symbol);
                        if bound_by_type_only_import(statement_list, name) {
                            type_only_exports.entry(Arc::from("default")).or_insert(TypeOnlyAliasKind::Import);
                        }
                        return;
                    }

                    // Only the export's type is wanted here: the check phase
                    // checks the expression (`check_export_assignment_expression`)
                    // with the module's imports bound, which a type query in it
                    // (`obj as typeof obj`) does not see from this table.
                    let checkpoint = ctx.diagnostics().len();
                    let ty = crate::infer::infer_expression(expression, exportable_values, ctx);
                    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                    let ty = match ty {
                        crate::infer::InferredExpression::Known(ty) => ty,
                        crate::infer::InferredExpression::Unknown
                        | crate::infer::InferredExpression::UnresolvedIdentifier { .. }
                        | crate::infer::InferredExpression::MissingProperty { .. } => Type::Unknown,
                    };

                    *default_symbol = Some(Arc::new(SymbolInfo {
                        ty,
                        kind: SymbolKind::Const,
                        function_signature: None,
                    }));
                }
                // The expression form is not modelled, but the module
                // demonstrably HAS a default export; recording none would make
                // every consumer report a missing `default` member.
                ParsedDefaultExportDeclaration::Unsupported { .. } => {
                    if default_symbol.is_none() {
                        *default_symbol = Some(Arc::new(SymbolInfo {
                            ty: Type::Unknown,
                            kind: SymbolKind::Const,
                            function_signature: None,
                        }));
                    }
                }
            },
            ParsedExportDeclaration::Namespace { .. } => {}
            ParsedExportDeclaration::All { .. } => {}
            _ => {}
        },
        ParsedStatement::TypeAliasDeclaration(alias) => {
            export_local_type_name(
                &alias.name,
                &alias.name,
                &alias.name_span,
                local_type_declarations,
                resolution_scope,
                type_declarations,
                ctx,
            );
        }
        ParsedStatement::InterfaceDeclaration(interface) => {
            export_local_type_name(
                &interface.name,
                &interface.name,
                &interface.name_span,
                local_type_declarations,
                resolution_scope,
                type_declarations,
                ctx,
            );
        }
        ParsedStatement::FunctionDeclaration(function) => {
            // A same-named `namespace` merges its value members into the
            // function (`fs.realpathSync.native`), and that merge lives in
            // `exportable_values`. The local symbol is the bare function and
            // would drop them, so the merged shape — an object that kept the
            // call signature — wins where it exists.
            let merged = exportable_values
                .get_shared(&function.name)
                .filter(|symbol| namespace_merged_callable(&symbol.ty));
            if let Some(symbol) = merged.or_else(|| local_symbols.get_shared(&function.name)) {
                if symbols.get(&function.name).is_none() {
                    symbols.insert_shared(function.name.clone(), symbol);
                }
            }
        }
        ParsedStatement::ClassDeclaration(class) => {
            // A class exports both an instance type and a constructor/static value.
            if let Some(type_declaration) = local_type_declarations.get(&class.name) {
                export_local_type_declaration(
                    type_declaration,
                    &class.name,
                    resolution_scope,
                    type_declarations,
                );
            }
            // The exportable set carries the class *after* its namespace merge
            // (`namespace EE { export const X }`); the binding-time table does not.
            if let Some(symbol) = exportable_values
                .get_shared(&class.name)
                .or_else(|| local_symbols.get_shared(&class.name))
            {
                if symbols.get(&class.name).is_none() {
                    symbols.insert_shared(class.name.clone(), symbol);
                }
            }
        }
        ParsedStatement::VariableDeclaration(variable) => {
            if let Some(symbol) = exportable_values.get_shared(&variable.name) {
                if symbols.get(&variable.name).is_none() {
                    symbols.insert_shared(variable.name.clone(), symbol);
                }
            }
        }
        ParsedStatement::NamespaceDeclaration(namespace) => {
            // `export namespace ns { … }` exports the namespace's value object
            // and its type members. The members were collected under qualified
            // `ns.<member>` keys (same shape the `export =` path consumes), so
            // carry those keys into the export table for qualified references
            // (`ns.Member`) on the importing side.
            if let Some(symbol) = exportable_values.get_shared(&namespace.name) {
                if symbols.get(&namespace.name).is_none() {
                    symbols.insert_shared(namespace.name.clone(), symbol);
                }
            }

            // The value members are collected under the same qualified
            // `ns.<member>` keys; carry them over so a call through the
            // namespace on the importing side finds the member's real signature.
            let value_prefix = format!("{}.", namespace.name);
            for (key, symbol) in exportable_values.iter_shared() {
                if key.starts_with(&value_prefix) && symbols.get(key.as_ref()).is_none() {
                    symbols.insert_shared(key.as_ref().to_string(), symbol.clone());
                }
            }

            if let Some(type_declaration) = local_type_declarations.get(&namespace.name) {
                export_local_type_declaration(
                    type_declaration,
                    &namespace.name,
                    resolution_scope,
                    type_declarations,
                );
            }

            let prefix = format!("{}.", namespace.name);
            for (key, declaration) in local_type_declarations.iter() {
                if key.starts_with(&prefix) && type_declarations.get(key.as_ref()).is_none() {
                    let _ = type_declarations.insert(
                        key.as_ref(),
                        attach_type_resolution_scope_if_missing(
                            declaration.clone(),
                            resolution_scope,
                        ),
                    );
                }
            }
        }
        _ => {}
    }
}

/// Publishes `local_name`'s type declaration under the `default` export key.
///
/// A default-exported class (or an identifier naming one) contributes a type as
/// well as a value, and a consumer's `import D from "./m"` binds both. The name
/// may itself be imported here (`import D from "./d"; export default D`), which
/// puts it in the resolution scope's import layers rather than this file's own
/// declaration table.
fn publish_default_type_export(
    local_name: &str,
    local_type_declarations: &TypeDeclarationTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
    ctx: &CheckerContext,
) {
    if type_declarations.get("default").is_some() {
        return;
    }

    if let Some(handle) = local_type_declarations
        .get_handle(local_name)
        .or_else(|| resolution_scope.and_then(|scope| scope.get_handle(local_name)))
    {
        export_local_type_declaration(handle.get(), "default", resolution_scope, type_declarations);
    } else if let Some(declaration) = ctx.ambient_global_type_declarations.get(local_name) {
        export_local_type_declaration(declaration, "default", None, type_declarations);
    }
}

/// tsc's `isEntityNameExpression`: an identifier, or a property access naming
/// an identifier on one, with no parentheses anywhere in it — `export default
/// (A.B)` exports a value, not an alias. The parsed tree drops parentheses, so
/// they are read back from the file's parenthesized-expression spans.
fn entity_name_expression(
    expression: &ParsedExpression,
    parenthesized_expressions: &[surge_ts_syntax::ParenthesizedExpressionSpan],
) -> Option<String> {
    let (name, span) = match expression {
        ParsedExpression::Identifier { name, span } => (name.clone(), *span),
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed: false,
            ..
        } => {
            let object_name = entity_name_expression(object, parenthesized_expressions)?;
            let span = object_span
                .zip(*property_span)
                .map(|(object_span, property_span)| TextSpan {
                    start: object_span.start,
                    end: property_span.end,
                });
            (format!("{object_name}.{property_name}"), span)
        }
        _ => return None,
    };
    let parenthesized = span.is_some_and(|span| {
        parenthesized_expressions
            .binary_search_by_key(&(span.start, span.end), |parenthesized| {
                (parenthesized.inner.start, parenthesized.inner.end)
            })
            .is_ok()
    });
    (!parenthesized).then_some(name)
}

/// The qualified type members of the local namespace `local` (`local.T`),
/// published under `exported` (`exported.T`) for an importer's qualified names.
fn export_namespace_type_members(
    local: &str,
    exported: &str,
    local_type_declarations: &TypeDeclarationTable,
    type_declarations: &mut TypeDeclarationTable,
) {
    let prefix = format!("{local}.");
    for (key, declaration) in local_type_declarations.iter() {
        if let Some(member) = key.strip_prefix(prefix.as_str()) {
            let _ = type_declarations.insert(format!("{exported}.{member}"), declaration.clone());
        }
    }
}

/// The entity `local` aliases when `statements` declare `import local = N.M`.
fn entity_alias_target<'s>(statements: &'s [ParsedStatement], local: &str) -> Option<&'s str> {
    statements.iter().find_map(|statement| {
        let import = match statement {
            ParsedStatement::ImportDeclaration(import) => import,
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                    ParsedStatement::ImportDeclaration(import) => import,
                    _ => return None,
                },
                _ => return None,
            },
            _ => return None,
        };
        match &import.kind {
            surge_ts_syntax::ParsedImportKind::EntityAlias {
                local_name, target, ..
            } if local_name == local => Some(target.as_str()),
            _ => None,
        }
    })
}

/// The value an alias's entity path names, read the way the path resolves
/// from the module: its first name in the module (or `globalThis`), then each
/// member.
fn entity_alias_value(
    target: &str,
    exportable_values: &SymbolTable,
    imported_symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Arc<SymbolInfo>> {
    if let Some(symbol) = exportable_values.get_shared(target) {
        return Some(symbol);
    }
    // `import g = globalThis` names the global object, which is installed once
    // module binding is done; until then the entity exists, its type unknown.
    if target == "globalThis" {
        return Some(ctx.ambient_global_symbols.get_shared(target).unwrap_or_else(|| {
            Arc::new(SymbolInfo {
                ty: Type::Unknown,
                kind: SymbolKind::Const,
                function_signature: None,
            })
        }));
    }
    let mut segments = target.split('.');
    let mut first = segments.next()?;
    let root = if first == "globalThis" {
        first = segments.next()?;
        ctx.ambient_global_symbols.get_shared(first)
    } else {
        exportable_values
            .get_shared(first)
            .or_else(|| imported_symbols.get_shared(first))
    }?;
    let mut members = segments.peekable();
    if members.peek().is_none() {
        return Some(root);
    }
    let mut ty = root.ty.clone();
    for member in members {
        let Type::Object(object) = &ty else {
            return None;
        };
        ty = object.properties.get(member)?.ty.clone();
    }
    Some(Arc::new(SymbolInfo {
        ty,
        kind: SymbolKind::Const,
        function_signature: None,
    }))
}

/// The type declarations an alias's entity path names: the entity itself and
/// its qualified members, published under `exported`.
fn export_entity_alias_types(
    target: &str,
    exported: &str,
    local_type_declarations: &TypeDeclarationTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
    ctx: &CheckerContext,
) {
    // The entity resolves the way tsc's `resolveEntityName` reads it from the
    // module: its own declarations, then its imports, then the globals —
    // `export import JSX = Internal` over `import { Internal } from ".."`.
    let (target, globals_only) = match target.strip_prefix("globalThis.") {
        Some(global) => (global, true),
        None => (target, false),
    };
    let prefix = format!("{target}.");
    let declares = |table: &TypeDeclarationTable| {
        table.get(target).is_some() || table.iter().any(|(key, _)| key.starts_with(&prefix))
    };
    let module_tables = std::iter::once(local_type_declarations).chain(
        resolution_scope
            .into_iter()
            .flat_map(|scope| scope.layers().iter().map(Arc::as_ref)),
    );
    let table = match module_tables
        .filter(|_| !globals_only)
        .find(|table| declares(table))
    {
        Some(table) => table,
        None => ctx.ambient_global_type_declarations.as_ref(),
    };
    if let Some(declaration) = table.get(target) {
        export_local_type_declaration(declaration, exported, resolution_scope, type_declarations);
    }
    for (key, declaration) in table.iter() {
        if let Some(member) = key.strip_prefix(prefix.as_str()) {
            let _ = type_declarations.insert(format!("{exported}.{member}"), declaration.clone());
        }
    }
}

/// `bindCommonJSTypeExports`: a CommonJS module's JSDoc types are its
/// exports, and with `module.exports` replaced they become members of the
/// replacement, which an import type or a named type import of the module
/// reaches (`import("./m").T`).
fn publish_commonjs_type_exports(
    statement_list: &[ParsedStatement],
    local_type_declarations: &TypeDeclarationTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
) {
    for statement in statement_list {
        let ParsedStatement::TypeAliasDeclaration(alias) = statement else {
            continue;
        };
        let Some(declaration) = local_type_declarations.get(&alias.name) else {
            continue;
        };
        let _ = type_declarations.insert(
            format!("{EXPORT_ASSIGNMENT_NAME}.{}", alias.name),
            attach_type_resolution_scope_if_missing(declaration.clone(), resolution_scope),
        );
        if type_declarations.get(&alias.name).is_none() {
            export_local_type_declaration(declaration, &alias.name, resolution_scope, type_declarations);
        }
    }
}

/// The type and namespace meanings of an `export =` entity, resolved the way
/// tsc's `resolveEntityName` resolves it from the export assignment: the
/// module's own declarations, then its imports, then the globals. The entity's
/// type is published as [`EXPORT_ASSIGNMENT_NAME`] and each namespace member
/// both under `export=.<member>` and bare — `getExportsOfModule` resolves the
/// `export =` first, so the members are what a named import reaches.
fn publish_export_assignment_types(
    entity: &str,
    local_type_declarations: &TypeDeclarationTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
    ctx: &CheckerContext,
) {
    let (entity, globals_only) = match entity.strip_prefix("globalThis.") {
        Some(global) => (global, true),
        None => (entity, false),
    };
    let prefix = format!("{entity}.");
    let declares = |table: &TypeDeclarationTable| {
        table.get(entity).is_some() || table.iter().any(|(key, _)| key.starts_with(&prefix))
    };
    let module_tables = std::iter::once(local_type_declarations).chain(
        resolution_scope
            .into_iter()
            .flat_map(|scope| scope.layers().iter().map(Arc::as_ref)),
    );
    let (table, scope) = match module_tables
        .filter(|_| !globals_only)
        .find(|table| declares(table))
    {
        Some(table) => (table, resolution_scope),
        None if declares(ctx.ambient_global_type_declarations.as_ref()) => {
            (ctx.ambient_global_type_declarations.as_ref(), None)
        }
        None => return,
    };

    if let Some(declaration) = table.get(entity)
        && type_declarations.get(EXPORT_ASSIGNMENT_NAME).is_none()
    {
        let _ = type_declarations.insert(
            EXPORT_ASSIGNMENT_NAME,
            attach_type_resolution_scope_if_missing(declaration.clone(), scope),
        );
    }
    for (key, declaration) in table.iter() {
        let Some(member) = key.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let _ = type_declarations.insert(
            format!("{EXPORT_ASSIGNMENT_NAME}.{member}"),
            attach_type_resolution_scope_if_missing(declaration.clone(), scope),
        );
        if type_declarations.get(member).is_none() {
            export_local_type_declaration(declaration, member, scope, type_declarations);
        }
    }
}

fn bound_by_type_only_import(statements: &[ParsedStatement], local: &str) -> bool {
    statements.iter().any(|statement| {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            return false;
        };
        match &import.kind {
            surge_ts_syntax::ParsedImportKind::Named { is_type_only: true, specifiers } => {
                specifiers.iter().any(|specifier| specifier.local_name == local)
            }
            surge_ts_syntax::ParsedImportKind::DefaultAndNamed { is_type_only: true, local_name, specifiers, .. } => {
                local_name == local || specifiers.iter().any(|specifier| specifier.local_name == local)
            }
            surge_ts_syntax::ParsedImportKind::TypeOnlyDefault { local_name, .. } => local_name == local,
            surge_ts_syntax::ParsedImportKind::Namespace { is_type_only: true, local_name, .. } => local_name == local,
            _ => false,
        }
    })
}
