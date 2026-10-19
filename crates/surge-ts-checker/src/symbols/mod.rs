mod scopes;
mod type_declarations;
mod values;

pub(crate) use scopes::ScopeStack;
pub(crate) use type_declarations::{
    clear_declared_variances,
    ClassMemberSymbol, InterfaceBody, InterfaceDeclarationFragmentId, InterfaceInfo,
    TypeAliasInfo, TypeDeclarationHandle, TypeDeclarationInfo, TypeDeclarationScope,
    TypeDeclarationMapWeak, TypeDeclarationTable, attach_global_declaration_scope,
    is_global_declaration_scope, mark_module_global_fragments,
    merge_augmentation_type_declaration_into_table, merge_interface_infos,
    merge_shared_table_into, merge_shared_tables_into, merge_type_declaration_into_table,
    reads_members_of,
};
pub(crate) use values::{
    AutoArrayBinding, BodyReturnEnclosing, BodyReturnSource, ConstructSignature, DestructureKey, FunctionSignatureInfo,
    InferredPredicate, SymbolInfo, SymbolInfoHandle, SymbolKind, SymbolTable,
    TupleDestructureBinding, clone_symbol_info_handle, map_symbol_kind,
};
