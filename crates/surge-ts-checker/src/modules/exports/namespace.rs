use super::*;

/// A re-exported import's source module and, for a named import, its original name.
pub(super) fn reexported_import_source(
    parsed_file: &ParsedProgramFile,
    local_name: &str,
) -> Option<(String, Option<String>)> {
    parsed_file.statements.iter().find_map(|statement| {
        let ParsedStatement::ImportDeclaration(import) = statement else {
            return None;
        };
        match &import.kind {
            ParsedImportKind::Namespace {
                local_name: import_local_name,
                ..
            } if import_local_name == local_name => Some((import.module_specifier.clone(), None)),
            ParsedImportKind::Named { specifiers, .. }
            | ParsedImportKind::DefaultAndNamed { specifiers, .. } => specifiers
                .iter()
                .find(|specifier| specifier.local_name == local_name)
                .map(|specifier| {
                    (
                        import.module_specifier.clone(),
                        Some(specifier.imported_name.clone()),
                    )
                }),
            _ => None,
        }
    })
}

/// Whether the module declares a value named `name` beside importing it
/// (`import { A } from "./a"; namespace A { … }`): the merged local value is
/// what the export carries, not the import's.
pub(super) fn declares_own_value(statements: &[ParsedStatement], name: &str) -> bool {
    statements.iter().any(|statement| match statement {
        ParsedStatement::VariableDeclaration(variable) => variable.name == name,
        ParsedStatement::FunctionDeclaration(function) => function.name == name,
        ParsedStatement::ClassDeclaration(class) => class.name == name,
        ParsedStatement::NamespaceDeclaration(namespace) => namespace.name.split('.').next() == Some(name),
        ParsedStatement::ExportDeclaration(export) => matches!(
            export.as_ref(),
            ParsedExportDeclaration::Statement { declaration, .. }
                if declares_own_value(std::slice::from_ref(declaration.as_ref()), name)
        ),
        _ => false,
    })
}

/// Materializes a re-exported namespace's type members into the export table as
/// qualified `<exported_name>.<member>` keys, mirroring how consumers of
/// `import * as` see them through alias scope layers.
pub(super) fn copy_namespace_member_type_exports(
    target_export_table: &ModuleExportTable,
    exported_name: &str,
    resolved_export_table: &mut ModuleExportTable,
) {
    let type_declarations = Arc::make_mut(&mut resolved_export_table.type_declarations);
    for (key, declaration) in target_export_table.type_declarations.iter() {
        if is_export_assignment_key(key) {
            continue;
        }
        let qualified = format!("{exported_name}.{key}");
        if type_declarations.get(&qualified).is_none() {
            let _ = type_declarations.insert(qualified.as_str(), declaration.clone());
        }
    }
}

pub(crate) fn insert_namespace_export(
    symbols: &mut SymbolTable,
    exported_name: &str,
    export_table: &ModuleExportTable,
) {
    let _ = symbols.insert(
        exported_name.to_string(),
        SymbolInfo {
            ty: namespace_export_object_type(export_table),
            kind: SymbolKind::Const,
            function_signature: None,
        },
    );
}

pub(crate) fn namespace_export_object_type(export_table: &ModuleExportTable) -> Type {
    if let Some(namespace_export_object_type) = &export_table.namespace_export_object_type {
        crate::program::record_module_export_borrowed_lookup_count();
        return namespace_export_object_type.clone();
    }

    compute_namespace_export_object_type(export_table)
}

/// A namespace import's members are tsc's *export symbols themselves* —
/// `resolveAnonymousTypeMembers` sets `members = getExportsOfSymbol(symbol)` for
/// a module — so `ns.f(x)` resolves the very signature `f(x)` does, type
/// parameters included. A property here carries only a `Type`, and a resolved
/// generic function handle has its type parameters erased to the sentinel, so
/// without the written signature riding along every `ns.f(...)` call lost its
/// inference: the callee checked, but `T` never bound, from the arguments or
/// from explicit type arguments alike.
fn namespace_member_type(symbol: &SymbolInfo) -> Type {
    let Some(signature) = symbol
        .function_signature
        .as_ref()
        .filter(|signature| !signature.type_parameters.is_empty())
    else {
        return symbol.ty.clone();
    };
    let Type::Function(function_type) = &symbol.ty else {
        return symbol.ty.clone();
    };
    if function_type.declaration().is_some() {
        return symbol.ty.clone();
    }
    Type::Function(function_type.clone().with_declaration(signature.clone()))
}

pub(crate) fn compute_namespace_export_object_type(export_table: &ModuleExportTable) -> Type {
    crate::program::record_module_export_namespace_export_object_materialization_count();
    let mut properties = surge_ts_types::PropertyMap::default();
    let mut property_count = 0u64;

    for (name, symbol) in export_table.symbols.iter() {
        // `export type` keeps a name off the module object.
        if export_table.type_only_exports.contains_key(name.as_ref()) {
            continue;
        }
        property_count += 1;
        // A `const` is read-only through the module object too (`isReadonlySymbol`).
        properties.insert(
            name.clone(),
            surge_ts_types::ObjectProperty::required(namespace_member_type(symbol))
                .with_readonly(matches!(symbol.kind, crate::symbols::SymbolKind::Const)),
        );
    }

    if let Some(default_symbol) = &export_table.default_symbol {
        property_count += 1;
        properties.insert(
            "default".into(),
            surge_ts_types::ObjectProperty::required(default_symbol.ty.clone()),
        );
    }

    // `export = <namespace>` exposes the namespace object as the module's shape;
    // surface its members (e.g. `React.createContext`) on the namespace import.
    // The assigned value can sit behind a lazy nominal reference (an alias-typed
    // `export =`), which the bare `Type::Object` match silently dropped — peel it
    // so the namespace members still surface.
    if let Some(export_assignment_symbol) = &export_table.export_assignment_symbol {
        let peeled_assignment;
        let assignment_ty = match &export_assignment_symbol.ty {
            Type::Reference(_) => {
                peeled_assignment = export_assignment_symbol.ty.peeled();
                &peeled_assignment
            }
            other => other,
        };
        if let Type::Object(object) = assignment_ty {
            for (name, property) in object.properties.iter() {
                property_count += 1;
                properties
                    .entry(name.clone())
                    .or_insert_with(|| property.clone());
            }
        }
    }

    crate::program::record_module_export_namespace_export_object_property_count(property_count);

    Type::Object(crate::metrics::alloc_object_type(properties, None))
}
