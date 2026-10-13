use super::*;

pub(crate) fn export_local_type_name(
    local_name: &str,
    exported_name: &str,
    name_span: &Option<TextSpan>,
    local_type_declarations: &TypeDeclarationTable,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
    ctx: &mut CheckerContext,
) {
    // Read the local declaration through an `Arc` handle so re-export
    // binding hands `export_local_type_declaration` a borrow instead of a deep
    // clone. The rename/scope rewrite there still takes one owned copy; this
    // removes the redundant second clone this path previously paid per
    // re-exported type.
    // `export { X }` / `export type { X }` may name an IMPORTED type, which
    // lives in the resolution scope's import layers rather than this file's own
    // declaration table. The scope is layer-ordered local-first, so this is a
    // pure miss-path fallback: a name declared here still resolves identically.
    let Some(handle) = local_type_declarations
        .get_handle(local_name)
        .or_else(|| resolution_scope.and_then(|scope| scope.get_handle(local_name)))
    else {
        push_unresolved_export_diagnostic(ctx, local_name, *name_span);
        return;
    };

    export_local_type_declaration(
        handle.get(),
        exported_name,
        resolution_scope,
        type_declarations,
    );
}

pub(crate) fn export_local_type_declaration(
    declaration: &TypeDeclarationInfo,
    exported_name: &str,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    type_declarations: &mut TypeDeclarationTable,
) {
    let declaration = rename_type_declaration(
        attach_type_resolution_scope_if_missing(declaration.clone(), resolution_scope),
        exported_name.to_string(),
    );
    let _ = type_declarations.insert(exported_name.to_string(), declaration);
}

pub(crate) fn rename_type_declaration(
    declaration: TypeDeclarationInfo,
    exported_name: String,
) -> TypeDeclarationInfo {
    match declaration {
        TypeDeclarationInfo::Alias(mut alias) => {
            if alias.declared_name.is_none() {
                alias.declared_name = Some(alias.name.clone());
            }
            alias.name = exported_name.into();
            alias.cached_resolution_key = std::sync::OnceLock::new();
            alias.cached_alias_id = std::sync::OnceLock::new();
            TypeDeclarationInfo::Alias(alias)
        }
        TypeDeclarationInfo::Interface(mut interface) => {
            if interface.declared_name.is_none() {
                interface.declared_name = Some(interface.name.clone());
            }
            interface.name = exported_name.into();
            interface.cached_resolution_key = std::sync::OnceLock::new();
            interface.cached_alias_id = std::sync::OnceLock::new();
            interface.cached_stable_id = std::sync::OnceLock::new();
            TypeDeclarationInfo::Interface(interface)
        }
    }
}

pub(crate) fn insert_type_export(
    type_declarations: &mut TypeDeclarationTable,
    local_name: &str,
    resolution_scope: Option<&Arc<TypeDeclarationScope>>,
    declaration: TypeDeclarationInfo,
) {
    let declaration = rename_type_declaration(
        attach_type_resolution_scope_if_missing(declaration, resolution_scope),
        local_name.to_string(),
    );
    let _ = type_declarations.insert(local_name.to_string(), declaration);
}

pub(crate) fn insert_unknown_type_import(
    type_declarations: &mut TypeDeclarationTable,
    local_name: &str,
    file_name: Arc<str>,
    name_span: Option<TextSpan>,
) {
    let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
        local_name.to_string(),
        file_name,
        name_span,
        vec![],
        ParsedType::Unknown,
        None,
    ));

    let _ = type_declarations.insert(local_name.to_string(), declaration);
}

/// The type side of an import from a module that does not resolve. tsc has no
/// answer for it either, so it binds the *error* type: permissive like the
/// degradation sentinel, but still reported through (a callback parameter
/// contextually typed by it is an implicit `any`). A module that *did* resolve
/// but whose export surge could not find is a modelling gap, not an error, and
/// keeps [`insert_unknown_type_import`].
pub(crate) fn insert_error_type_import(
    type_declarations: &mut TypeDeclarationTable,
    local_name: &str,
    file_name: Arc<str>,
    name_span: Option<TextSpan>,
) {
    let declaration = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
        local_name.to_string(),
        file_name,
        name_span,
        vec![],
        ParsedType::ErrorType,
        None,
    ));

    let _ = type_declarations.insert(local_name.to_string(), declaration);
}

pub(crate) fn insert_unknown_value_import(local_name: &str, symbols: &mut SymbolTable) {
    insert_value_import(local_name, Type::Unknown, symbols);
}

/// Binds a name whose import module was reported unresolved to tsc's error
/// type. [`SymbolKind::ErrorImport`] records that it is the source's, not a
/// surge modelling failure.
pub(crate) fn insert_error_typed_value_import(local_name: &str, symbols: &mut SymbolTable) {
    let _ = symbols.insert(
        local_name.to_string(),
        SymbolInfo {
            ty: Type::ErrorType,
            kind: SymbolKind::ErrorImport,
            function_signature: None,
        },
    );
}

pub(crate) fn insert_value_import(local_name: &str, ty: Type, symbols: &mut SymbolTable) {
    let _ = symbols.insert(
        local_name.to_string(),
        SymbolInfo {
            ty,
            kind: SymbolKind::Var,
            function_signature: None,
        },
    );
}

pub(crate) fn lookup_type_export<'a>(
    export_table: &'a ModuleExportTable,
    local_name: &'a str,
) -> Option<&'a TypeDeclarationInfo> {
    crate::program::record_module_export_borrowed_lookup_count();
    export_table.type_declarations.get(local_name)
}

/// Copy an exported namespace's qualified type members (`ns.Member`) into the
/// importer's scope under the local binding name (`local.Member`), so qualified
/// type references through a named namespace import resolve.
/// Copies every `<imported_name>.<member>` qualified type export under the
/// local binding name. Returns whether any member was copied — a `true` means
/// the imported name exists as a (type-only) namespace even when it has no
/// direct type/value export entry of its own.
pub(crate) fn copy_qualified_type_exports(
    export_table: &ModuleExportTable,
    imported_name: &str,
    local_name: &str,
    type_declarations: &mut TypeDeclarationTable,
) -> bool {
    let prefix = format!("{imported_name}.");
    let mut copied_any = false;
    for (key, _) in export_table.type_declarations.iter() {
        if let Some(member) = key.strip_prefix(&prefix) {
            copied_any = true;
            let local_key = format!("{local_name}.{member}");
            let _ = type_declarations.insert_shared_from(
                &local_key,
                &export_table.type_declarations,
                key.as_ref(),
            );
        }
    }
    copied_any
}

/// Value-side twin of [`copy_qualified_type_exports`]: copies an exported
/// namespace's `ns.member` value entries under the importer's local binding
/// name, so a qualified call (`util.arrayToEnum(…)`) can find the member's real
/// signature through the local alias.
pub(crate) fn copy_qualified_value_exports(
    export_table: &ModuleExportTable,
    imported_name: &str,
    local_name: &str,
    symbols: &mut SymbolTable,
) {
    let prefix = format!("{imported_name}.");
    for (key, symbol) in export_table.symbols.iter_shared() {
        if let Some(member) = key.strip_prefix(&prefix) {
            let local_key = format!("{local_name}.{member}");
            if symbols.get(&local_key).is_none() {
                symbols.insert_shared(local_key, symbol.clone());
            }
        }
    }
}

/// Re-exposes a module's qualified value members under a namespace alias
/// (`import * as ts` -> `ts.isImportDeclaration`), the value twin of
/// [`crate::modules::imports::build_namespace_alias_table`]. Only members that
/// carry a published signature are copied: the namespace object models the
/// member *set* with permissive `any`, so a call or a type-predicate guard
/// routed through it would otherwise lose the signature. An `export =` module's
/// members are also keyed bare (see the `Equals` arm in `statements.rs`), which
/// is what makes `<alias>.<member>` land on the right name.
pub(crate) fn copy_namespace_alias_value_exports(
    export_table: &ModuleExportTable,
    local_name: &str,
    symbols: &mut SymbolTable,
) {
    let has_export_assignment = export_table.export_assignment_symbol.is_some();
    for (key, symbol) in export_table.symbols.iter_shared() {
        if symbol.function_signature.is_none() {
            continue;
        }
        let head = key.split_once('.').map_or(key.as_ref(), |(head, _)| head);
        if export_table.type_only_exports.contains_key(head) {
            continue;
        }
        if !has_export_assignment && !key.contains('.') {
            continue;
        }
        let local_key = format!("{local_name}.{key}");
        if symbols.get(&local_key).is_none() {
            symbols.insert_shared(local_key, symbol.clone());
        }
    }
}

pub(crate) fn lookup_value_export(
    export_table: &ModuleExportTable,
    local_name: &str,
) -> Option<Arc<SymbolInfo>> {
    if local_name == "default" {
        return export_table.get_shared_value("default");
    }

    export_table
        .get_shared_value(local_name)
        .or_else(|| lookup_export_assignment_member(export_table, local_name))
}

/// A module whose surface is `export = X` publishes no named exports, but tsc
/// lets a named import reach the members of the assigned value's type
/// (`import { join } from "path"` over `export = path`). Peel the assignment —
/// it can sit behind a lazy nominal reference — and read the member off the
/// object shape, mirroring what `compute_namespace_export_object_type` already
/// does for namespace imports.
///
/// Miss path only: the module's own named exports always win, and nothing is
/// materialized into `symbols` (that would make the members visible to
/// `export *`, which tsc does not do).
fn lookup_export_assignment_member(
    export_table: &ModuleExportTable,
    local_name: &str,
) -> Option<Arc<SymbolInfo>> {
    let export_assignment_symbol = export_table.export_assignment_symbol.as_ref()?;
    let assignment_ty = export_assignment_symbol.ty.peeled();
    let ty = match &assignment_ty {
        // A generic class's value side is modelled as `any`, so its statics are
        // unavailable; `any` answers every member, which is also what an
        // `export = <any>` surface means.
        Type::Any => Type::Any,
        // The value's type is not known yet (an `export =` of a declared
        // constant reads its annotation once the module's scope is bound):
        // nothing says the member is missing.
        Type::Unknown => Type::Unknown,
        // Declared properties only — `Object.prototype` members are not module
        // exports, so `import { toString } from "path"` must stay TS2305. A
        // static side left open (its base was modelled as `any`) answers like
        // `any` does.
        Type::Object(object) => match object.properties.get(local_name) {
            Some(property) => property.ty.clone(),
            None if object.synthetic_open_index => Type::Any,
            None => return None,
        },
        // A union's property is one every member declares
        // (`getUnionOrIntersectionProperty`), typed by their union.
        Type::Union(union) => {
            let mut member_types = Vec::with_capacity(union.types().len());
            for member in union.types().iter() {
                let Type::Object(object) = member.peeled() else {
                    return None;
                };
                member_types.push(object.properties.get(local_name)?.ty.clone());
            }
            surge_ts_types::union_type(member_types)
        }
        _ => return None,
    };

    Some(Arc::new(SymbolInfo {
        ty,
        kind: SymbolKind::Const,
        function_signature: None,
    }))
}
