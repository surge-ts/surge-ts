//! `SURGE_TRACE_RESOLUTION=<path>`: what each program file's module
//! specifiers resolve to, written as tsc's `--traceResolution` summary lines
//! so the two can be compared.

use std::collections::HashSet;
use std::fmt::Write;

use surge_ts_syntax::{ParsedExportDeclaration, ParsedStatement, ResolutionModeOverride};

use super::ParsedProgramFile;
use crate::context::CheckerContext;

pub(super) fn write_resolution_trace(parsed_files: &[ParsedProgramFile], ctx: &CheckerContext) {
    let Some(path) = std::env::var_os("SURGE_TRACE_RESOLUTION") else {
        return;
    };
    let mut out = String::new();
    for file in parsed_files {
        let mut specifiers = Vec::new();
        collect_specifiers(&file.statements, &mut specifiers);
        specifiers.extend(file.import_call_specifiers.iter().map(|specifier| (specifier.clone(), None)));
        let mut seen = HashSet::new();
        let importer = absolute(&file.file_name);
        for (specifier, mode) in specifiers {
            if !seen.insert((specifier.clone(), mode)) {
                continue;
            }
            let resolved = crate::modules::resolved_module_in_mode(ctx, &file.file_name, &specifier, mode)
                .cloned()
                .or_else(|| {
                    crate::modules::resolve_relative_module_in_mode(
                        &file.file_name,
                        &specifier,
                        mode,
                        parsed_files,
                        &ctx.module_file_index_by_identity,
                    )
                    .map(|resolution| resolution.resolved_file_name)
                });
            let mode = match mode {
                Some(ResolutionModeOverride::Import) => " (import)",
                Some(ResolutionModeOverride::Require) => " (require)",
                None => "",
            };
            let _ = writeln!(out, "======== Resolving module '{specifier}' from '{importer}'.{mode} ========");
            let _ = match resolved {
                Some(resolved) => writeln!(
                    out,
                    "======== Module name '{specifier}' was successfully resolved to '{}'. ========",
                    absolute(&resolved)
                ),
                None => writeln!(out, "======== Module name '{specifier}' was not resolved. ========"),
            };
        }
    }
    let _ = std::fs::write(path, out);
}

fn absolute(file_name: &str) -> String {
    std::path::absolute(file_name).map_or_else(|_| file_name.to_string(), |path| path.display().to_string())
}

fn collect_specifiers(statements: &[ParsedStatement], out: &mut Vec<(String, Option<ResolutionModeOverride>)>) {
    for statement in statements {
        match statement {
            ParsedStatement::ImportDeclaration(import) => {
                out.push((import.module_specifier.clone(), crate::modules::import_resolution_mode(import)));
            }
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Named { module_specifier: Some(specifier), .. }
                | ParsedExportDeclaration::All { module_specifier: specifier, .. }
                | ParsedExportDeclaration::Namespace { module_specifier: specifier, .. } => {
                    out.push((specifier.clone(), None));
                }
                ParsedExportDeclaration::Statement { declaration, .. } => {
                    collect_specifiers(std::slice::from_ref(declaration), out);
                }
                _ => {}
            },
            ParsedStatement::DeclareModuleDeclaration(module) => collect_specifiers(&module.statements, out),
            ParsedStatement::NamespaceDeclaration(namespace) => collect_specifiers(&namespace.statements, out),
            _ => {}
        }
    }
}
