//! tsc's synthetic default: a default import of a module that can have one
//! binds the module itself — its `export =` entity, else its namespace —
//! rather than a `default` export (`getTargetOfModuleDefault`).

use super::*;
use crate::context::{CheckerContext, ModuleEmitKind};
use crate::program::ParsedProgramFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleFormat {
    CommonJs,
    Esm,
}

/// tsc's `GetImpliedNodeFormatForEmit`: under a node module kind the file's
/// implied format, under any other the one its extension fixes, or ESM for a
/// dependency under a `"type": "module"` package (`esm_package_files`). A
/// project file's package.json `"type"` is read under node resolution alone,
/// where `esm_module_files` records it.
pub(crate) fn implied_format_for_emit(ctx: &CheckerContext, file_name: &str) -> Option<ModuleFormat> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".mts") || lower.ends_with(".mjs") {
        return Some(ModuleFormat::Esm);
    }
    if lower.ends_with(".cts") || lower.ends_with(".cjs") {
        return Some(ModuleFormat::CommonJs);
    }
    if !ctx.options.module_emit.is_node() {
        return ctx.options.esm_package_files.contains(file_name).then_some(ModuleFormat::Esm);
    }
    if ![".ts", ".tsx", ".js", ".jsx"].iter().any(|extension| lower.ends_with(extension)) {
        return None;
    }
    Some(if ctx.options.esm_module_files.contains(file_name) {
        ModuleFormat::Esm
    } else {
        ModuleFormat::CommonJs
    })
}

/// tsc's `getEmitSyntaxForUsageLocation` for an import or export declaration
/// in the file being checked: the file's emit format.
pub(crate) fn declaration_emit_syntax(ctx: &CheckerContext) -> Option<ModuleFormat> {
    if let Some(format) = implied_format_for_emit(ctx, &ctx.file_name) {
        return Some(format);
    }
    match ctx.options.module_emit {
        ModuleEmitKind::CommonJS => Some(ModuleFormat::CommonJs),
        kind if kind.is_ecmascript() || kind == ModuleEmitKind::Preserve => Some(ModuleFormat::Esm),
        _ => None,
    }
}

fn is_javascript_file(file: &ParsedProgramFile) -> bool {
    let lower = file.file_name.to_ascii_lowercase();
    file.json_module_type.is_some()
        || [".js", ".jsx", ".mjs", ".cjs"]
            .iter()
            .any(|extension| lower.ends_with(extension))
}

/// tsc's `isSyntacticDefault` over the module's `default` export: written as
/// `export default`, a `default` modifier or an export specifier. A module
/// with `export =` reads `default` off the assigned entity, never syntactic.
fn has_syntactic_default(export_table: &ModuleExportTable) -> bool {
    !export_table.writes_export_assignment
        && (export_table.default_symbol.is_some()
            || export_table.type_declarations.get("default").is_some()
            // `export default N` over a namespace of types alone.
            || export_table.type_declarations.iter().any(|(key, _)| key.starts_with("default.")))
}

/// tsc's `canHaveSyntheticDefault` for a default import, read from the file
/// being checked, of the module `export_table` describes: `target` is its
/// file, `None` for an ambient module.
pub(crate) fn can_have_synthetic_default(
    ctx: &CheckerContext,
    target: Option<&ParsedProgramFile>,
    export_table: &ModuleExportTable,
) -> bool {
    if let Some(file) = target
        && declaration_emit_syntax(ctx) == Some(ModuleFormat::Esm)
    {
        match implied_format_for_emit(ctx, &file.file_name) {
            // In Node.js an ESM import of a CommonJS module always has one.
            Some(ModuleFormat::CommonJs) if ctx.options.module_emit.is_node() => return true,
            // Between two ESM files there is none, whatever `module` says.
            Some(ModuleFormat::Esm) => return false,
            _ => {}
        }
    }
    let exports_es_module_marker = || export_table.symbols.get("__esModule").is_some();
    match target {
        Some(file) if !file.file_kind.is_declaration() => {
            if is_javascript_file(file) {
                // `ExternalModuleIndicator == nil`: no ECMAScript module
                // syntax, which a CommonJS module has none of.
                (!file.is_module || file.commonjs_module) && !exports_es_module_marker()
            } else {
                // A TypeScript file is emitted with an `__esModule` marker
                // unless it writes `export =`.
                export_table.writes_export_assignment
            }
        }
        // A declaration file or ambient module may describe CommonJS without
        // saying so; only its own default or `__esModule` export rules one out.
        _ => !has_syntactic_default(export_table) && !exports_es_module_marker(),
    }
}

/// tsc's `resolveESModuleSymbol` for `import * as ns`: the namespace object
/// gains the synthetic default as a `default` member when the module is
/// callable or constructable, already has a `default`, or is a CommonJS file
/// an ESM file imports — and it can have a synthetic default at all.
pub(crate) fn namespace_import_has_synthetic_default(
    ctx: &CheckerContext,
    target: Option<&ParsedProgramFile>,
    export_table: &ModuleExportTable,
) -> bool {
    let assignment = export_table
        .export_assignment_symbol
        .as_ref()
        .map(|symbol| symbol.ty.peeled());
    let (has_signatures, has_default) = match &assignment {
        Some(Type::Function(_)) => (true, false),
        Some(Type::Object(object)) => (
            object.call_signature.is_some() || object.construct_signature.is_some(),
            object.properties.get("default").is_some(),
        ),
        Some(_) => (false, false),
        None => (false, export_table.default_symbol.is_some()),
    };
    let esm_imports_commonjs = target.is_some_and(|file| {
        declaration_emit_syntax(ctx) == Some(ModuleFormat::Esm)
            && implied_format_for_emit(ctx, &file.file_name) == Some(ModuleFormat::CommonJs)
    });
    (has_signatures || has_default || esm_imports_commonjs)
        && can_have_synthetic_default(ctx, target, export_table)
}

/// The namespace object with `default` set to the synthetic default, which
/// overrides a `default` member of its own.
pub(crate) fn with_synthetic_default_member(namespace_type: Type, default_type: Type) -> Type {
    let Type::Object(object) = namespace_type else {
        return namespace_type;
    };
    let mut properties = (*object.properties).clone();
    properties.insert(
        "default".into(),
        surge_ts_types::ObjectProperty::required(default_type),
    );
    Type::Object(crate::metrics::alloc_object_type(properties, None))
}
