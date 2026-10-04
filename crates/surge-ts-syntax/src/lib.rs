//! Parsed TypeScript syntax and parser entrypoint.

mod ast;
pub mod clone_census;
mod parser;

pub use ast::*;
pub use parser::{
    EmitHelperOptions, EmitHelperRequest, emit_helper_requests, external_emit_helpers,
    DeclarationEmitDiagnostic, IsolatedDeclarationDiagnostic, JsxRuntimeOptions, ParserWorker, extract_check_directive, extract_reference_path_directives,
    extract_reference_type_directives, is_declaration_file_name, is_javascript_file_name,
    is_json_file_name, isolated_declaration_diagnostics, declaration_emit_diagnostics, js_number_to_string, jsx_entity_root,
    jsx_runtime_import, merge_parsed_call_signatures, parse_json_module_type, parse_source,
};

#[cfg(test)]
mod tests;
