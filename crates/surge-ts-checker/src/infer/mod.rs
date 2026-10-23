pub(crate) mod expression;
pub(crate) mod types;

pub(crate) use expression::{
    array_pattern_source_type, falsy_part, infer_expression, narrowed_element_read_named, truthy_part, tuple_index_value,
    unchecked_index_read,
};
pub(crate) use types::{
    LazySignatureComponent, LazySignatureEnvironment, TypeParameterSubstitution,
    make_lazy_signature_annotation_reference, make_lazy_value_annotation_reference,
    map_parsed_type, map_parsed_type_with_substitution, report_duplicate_type_parameters,
    string_literal_union_keys, substitute_parsed_type_parameters_deep,
    try_map_parsed_type_with_substitution, validate_local_type_declaration,
    with_type_declaration_scope,
};

use surge_ts_syntax::TextSpan;
use surge_ts_types::Type;

#[derive(Debug, Clone)]
pub(crate) enum InferredExpression {
    Known(Type),
    UnresolvedIdentifier {
        name: String,
        span: Option<TextSpan>,
    },
    MissingProperty {
        property_name: String,
        object_type: Type,
        span: Option<TextSpan>,
    },
    Unknown,
}

/// tsc's error type for a *name* that did not resolve — but only where the
/// failure is the source's. During module analysis an import that is not bound
/// *yet* fails the same lookup, and publishing the error type from there
/// poisons the export every consumer reads; those passes keep the sentinel. A
/// missing *member* has no such ordering: its receiver already resolved.
pub(crate) fn unresolved_name_error_type() -> Option<Type> {
    crate::program::in_check_phase().then_some(Type::error_type())
}

impl InferredExpression {
    /// The type the expression contributes where only a type can flow on: a
    /// return, an inference candidate, a binding. `None` is surge's own "could
    /// not model this".
    pub(crate) fn flowing_type(self) -> Option<Type> {
        match self {
            Self::Known(ty) => Some(ty),
            Self::MissingProperty { .. } => Some(Type::error_type()),
            Self::UnresolvedIdentifier { .. } => unresolved_name_error_type(),
            Self::Unknown => None,
        }
    }
}
