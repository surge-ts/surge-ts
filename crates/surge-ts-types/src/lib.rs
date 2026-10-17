//! Core TypeScript-like type representation and assignability helpers.

mod assignability;
mod clone_reason;
mod function;
pub mod fx;
pub mod leaf_probe;
mod name_memo;
pub use name_memo::clear_name_intern_table;
mod object;
pub mod private_name;
mod reference;
mod store;
mod identity;
mod subtype;
mod template_literal;
mod tuple;
pub use tuple::{TupleLabel, TupleType, open_tuple_labels};
mod tsc_display;
pub use tsc_display::{CreationRanks, with_creation_ranks, with_tsc_display, with_tsc_truncation};
pub mod type_variable;
mod ty;
mod union;

pub use assignability::*;
pub use clone_reason::{TypeCopyReason, with_type_copy_reason};
pub use function::*;
pub use object::*;
pub use reference::*;
pub use store::*;
pub use identity::is_type_identical_to;
pub use subtype::{
    LiteralShape, has_effective_rest_parameter, is_strict_subtype_of, is_subtype_of, is_subtype_of_expression, min_argument_count,
    literal_reduced_union, parameter_count, subtype_reduced_union, try_type_at_position, type_at_position,
};
pub use template_literal::{
    STRING_MAPPING_REFERENCE_ID, StringMappingKind, TEMPLATE_LITERAL_REFERENCE_ID,
    infer_template_literal_placeholders, is_member_of_string_mapping, is_template_literal_type, peel_to_pattern_literal,
    template_placeholder_intersection,
    is_type_matched_by_template_literal, preferred_template_placeholder_inference,
    string_mapping_parts, string_mapping_type, template_literal_parts, template_literal_type,
};
pub use ty::*;
pub use union::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PropertyMap;

    #[test]
    fn crate_root_reexports_still_work() {
        let mut properties = PropertyMap::default();
        properties.insert("name".into(), ObjectProperty::required(Type::String));

        let ty = Type::Object(ObjectType::new(properties, None));

        assert_eq!(ty.name(), "{ name: string; }");
        assert!(is_assignable_to(&Type::String, &Type::Any));
    }

    #[test]
    fn optional_property_access_widens_to_undefined() {
        let mut properties = PropertyMap::default();
        properties.insert("name".into(), ObjectProperty::optional(Type::String));

        let ty = ObjectType::new(properties, None);

        assert_eq!(
            ty.get_property_access_type("name"),
            Some(union_type(vec![Type::String, Type::Undefined]))
        );
    }
}
