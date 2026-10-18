use super::*;
use crate::PropertyMap;
use crate::{FunctionType, NumberLiteralType, ObjectProperty, ObjectType, union_type};

fn function_type(
    parameters: Vec<Type>,
    return_type: Type,
    is_variadic: bool,
    required_parameter_count: usize,
) -> Type {
    Type::Function(FunctionType::new(
        parameters,
        return_type,
        is_variadic,
        required_parameter_count,
    ))
}

fn name_target() -> Type {
    let mut properties = PropertyMap::default();
    properties.insert("name".into(), ObjectProperty::required(Type::String));
    Type::Object(ObjectType::new(properties, None))
}

#[test]
fn function_with_any_return_assignable_to_void_returning_target() {
    // A `void`-returning target ignores the source return type, so a function
    // returning a value still satisfies it (e.g. an event handler slot).
    let source = function_type(vec![Type::Unknown], Type::Unknown, false, 1);
    let target = function_type(vec![Type::Unknown], Type::Void, false, 1);
    assert!(is_assignable_to(&source, &target));

    // The reverse does not hold: a non-`void` target still checks the return.
    let source_void = function_type(vec![Type::Unknown], Type::Void, false, 1);
    let target_number = function_type(vec![Type::Unknown], Type::Number, false, 1);
    assert!(!is_assignable_to(&source_void, &target_number));
}

#[test]
fn function_assignable_to_name_object() {
    // A plain function value satisfies `{name: string}` via `Function.name`.
    let function = function_type(vec![], Type::Void, false, 0);
    assert!(is_assignable_to(&function, &name_target()));
}

#[test]
fn function_not_assignable_to_object_with_extra_required_member() {
    let mut properties = PropertyMap::default();
    properties.insert("name".into(), ObjectProperty::required(Type::String));
    properties.insert("nope".into(), ObjectProperty::required(Type::Number));
    let target = Type::Object(ObjectType::new(properties, None));
    let function = function_type(vec![], Type::Void, false, 0);
    assert!(!is_assignable_to(&function, &target));
}

#[test]
fn constructor_object_assignable_to_name_object() {
    // `typeof SomeClass` (a construct-signature object) satisfies
    // `{name: string}` via the synthesized `Function.name` member.
    let constructor =
        Type::Object(
            ObjectType::new(PropertyMap::default(), None)
                .with_construct_signature(FunctionType::new(vec![], Type::Void, false, 0)),
        );
    assert!(is_assignable_to(&constructor, &name_target()));
}

#[test]
fn function_assignable_to_function_interface() {
    // Any function is assignable to the global `Function` interface, even
    // though its structural shape (`prototype`/`arguments`/`caller`) is not
    // exposed by a bare function type. Also covers a `string | Function`
    // target (the DOM `setTimeout` handler).
    struct Resolver(Type);
    impl crate::ResolveReference for Resolver {
        fn resolve(&self) -> Type {
            self.0.clone()
        }
    }
    let mut members = PropertyMap::default();
    members.insert("prototype".into(), ObjectProperty::required(Type::Any));
    members.insert("arguments".into(), ObjectProperty::required(Type::Any));
    let function_interface = Type::Reference(crate::TypeReference::new(
        "lib.es5.d.ts\u{0}Function",
        "Function",
        Vec::new(),
        std::sync::Arc::new(Resolver(Type::Object(ObjectType::new(members, None)))),
    ));
    let function = function_type(vec![Type::Number], Type::Number, false, 1);
    assert!(is_assignable_to(&function, &function_interface));
    assert!(is_assignable_to(
        &function,
        &union_type(vec![Type::String, function_interface])
    ));
}

#[test]
fn plain_object_without_name_not_assignable_to_name_object() {
    // The Function-member fallback must not leak to ordinary objects.
    let source = Type::Object(ObjectType::new(PropertyMap::default(), None));
    assert!(!is_assignable_to(&source, &name_target()));
}

#[test]
fn array_assignable_to_empty_object() {
    // `Object.fromEntries([...])`: an array satisfies a no-required-member
    // object target (`{}`) just like any non-nullish value.
    let empty = Type::Object(ObjectType::new(PropertyMap::default(), None));
    assert!(is_assignable_to(
        &Type::Array(Box::new(Type::Number)),
        &empty
    ));
    assert!(is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Any]),
        &empty
    ));
}

#[test]
fn array_not_assignable_to_object_with_required_member() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::Number)),
        &name_target()
    ));
}

#[test]
fn never_assignable_to_everything() {
    assert!(is_assignable_to(&Type::Never, &Type::String));
    assert!(is_assignable_to(&Type::Never, &Type::Number));
    assert!(is_assignable_to(&Type::Never, &Type::Never));
}

/// A reference whose structural expansion is keyed on its own display name,
/// so any two distinct instantiations are structurally incompatible — a
/// passing assertion proves the nominal argument path was taken rather than
/// the structural fallback.
fn opaque_generic(id: &str, display: &str, argument: Type) -> Type {
    struct Opaque(String);
    impl crate::ResolveReference for Opaque {
        fn resolve(&self) -> Type {
            let mut members = PropertyMap::default();
            members.insert(
                self.0.as_str().into(),
                ObjectProperty::required(Type::Never),
            );
            Type::Object(ObjectType::new(members, None))
        }
    }
    Type::Reference(crate::TypeReference::new(
        id,
        display,
        vec![argument],
        std::sync::Arc::new(Opaque(display.to_string())),
    ))
}

#[test]
fn same_generic_declaration_compares_by_arguments_not_structure() {
    let set_any = opaque_generic("lib\u{0}Set", "Set<any>", Type::Any);
    let set_string = opaque_generic("lib\u{0}Set", "Set<string>", Type::String);
    let set_unknown = opaque_generic("lib\u{0}Set", "Set<unknown>", Type::Unknown);
    let set_number = opaque_generic("lib\u{0}Set", "Set<number>", Type::Number);

    assert!(is_assignable_to(&set_any, &set_string));
    assert!(is_assignable_to(&set_string, &set_any));
    assert!(is_assignable_to(&set_unknown, &set_string));
    assert!(!is_assignable_to(&set_string, &set_number));
}

#[test]
fn different_generic_declarations_do_not_compare_by_arguments() {
    let set = opaque_generic("lib\u{0}Set", "Set<any>", Type::Any);
    let map = opaque_generic("lib\u{0}Map", "Map<any>", Type::Any);
    assert!(!is_assignable_to(&set, &map));
}

#[test]
fn same_generic_reference_assignable_to_union_member() {
    let set_any = opaque_generic("lib\u{0}Set", "Set<any>", Type::Any);
    let set_string = opaque_generic("lib\u{0}Set", "Set<string>", Type::String);
    let target = union_type(vec![set_string, Type::Undefined]);
    assert!(is_assignable_to(&set_any, &target));
}

#[test]
fn nothing_assignable_to_never_except_never() {
    assert!(!is_assignable_to(&Type::String, &Type::Never));
    assert!(!is_assignable_to(&Type::Undefined, &Type::Never));
}

#[test]
fn string_literal_assignable_to_string() {
    assert!(is_assignable_to(
        &Type::StringLiteral("ok".to_string()),
        &Type::String
    ));
}

#[test]
fn string_not_assignable_to_string_literal() {
    assert!(!is_assignable_to(
        &Type::String,
        &Type::StringLiteral("ok".to_string())
    ));
}

#[test]
fn matching_string_literal_assignable_to_same_literal() {
    let literal = Type::StringLiteral("ok".to_string());
    assert!(is_assignable_to(&literal, &literal));
}

#[test]
fn different_string_literal_not_assignable() {
    assert!(!is_assignable_to(
        &Type::StringLiteral("ok".to_string()),
        &Type::StringLiteral("no".to_string())
    ));
}

#[test]
fn number_literal_assignability() {
    let literal = Type::NumberLiteral(NumberLiteralType {
        value: "1".to_string(),
    });

    assert!(is_assignable_to(&literal, &Type::Number));
    assert!(!is_assignable_to(
        &Type::Number,
        &Type::NumberLiteral(NumberLiteralType {
            value: "1".to_string(),
        })
    ));
}

#[test]
fn boolean_literal_assignability() {
    assert!(is_assignable_to(
        &Type::BooleanLiteral(true),
        &Type::Boolean
    ));
    assert!(!is_assignable_to(
        &Type::Boolean,
        &Type::BooleanLiteral(true)
    ));
    assert!(!is_assignable_to(
        &Type::BooleanLiteral(true),
        &Type::BooleanLiteral(false)
    ));
}

#[test]
fn literal_union_assignability() {
    assert!(is_assignable_to(
        &union_type(vec![
            Type::StringLiteral("ok".to_string()),
            Type::StringLiteral("no".to_string())
        ]),
        &Type::String
    ));
    assert!(!is_assignable_to(
        &Type::String,
        &union_type(vec![
            Type::StringLiteral("ok".to_string()),
            Type::StringLiteral("no".to_string())
        ])
    ));
}

#[test]
fn literal_object_assignability() {
    let mut source_properties = PropertyMap::default();
    source_properties.insert(
        "kind".into(),
        ObjectProperty::required(Type::StringLiteral("click".to_string())),
    );

    let mut target_properties = PropertyMap::default();
    target_properties.insert("kind".into(), ObjectProperty::required(Type::String));

    assert!(is_assignable_to(
        &Type::Object(ObjectType::new(source_properties, None)),
        &Type::Object(ObjectType::new(target_properties, None))
    ));
}

#[test]
fn void_assignability() {
    assert!(is_assignable_to(&Type::Undefined, &Type::Void));
    assert!(is_assignable_to(&Type::Void, &Type::Any));
    assert!(is_assignable_to(&Type::Any, &Type::Void));
    assert!(!is_assignable_to(&Type::Void, &Type::Undefined));
}

#[test]
fn undefined_assignable_to_void() {
    assert!(is_assignable_to(&Type::Undefined, &Type::Void));
}

#[test]
fn void_assignable_to_void() {
    assert!(is_assignable_to(&Type::Void, &Type::Void));
}

#[test]
fn any_assignable_to_void() {
    assert!(is_assignable_to(&Type::Any, &Type::Void));
}

#[test]
fn void_assignable_to_any() {
    assert!(is_assignable_to(&Type::Void, &Type::Any));
}

#[test]
fn void_not_assignable_to_undefined() {
    assert!(!is_assignable_to(&Type::Void, &Type::Undefined));
}

#[test]
fn void_not_assignable_to_string() {
    assert!(!is_assignable_to(&Type::Void, &Type::String));
}

#[test]
fn void_union_assignability() {
    let void_union = union_type(vec![Type::Void, Type::Undefined]);

    assert!(is_assignable_to(&Type::Undefined, &void_union));
    assert!(is_assignable_to(&Type::Void, &void_union));
    assert!(is_assignable_to(&void_union, &Type::Void));
    assert!(!is_assignable_to(&void_union, &Type::Undefined));
}

#[test]
fn function_type_assignable_same_signature() {
    let source = function_type(vec![Type::String], Type::Number, false, 1);
    let target = function_type(vec![Type::String], Type::Number, false, 1);

    assert!(is_assignable_to(&source, &target));
}

#[test]
fn function_type_assignable_fewer_source_parameters() {
    // A callback declaring fewer parameters than the target expects is
    // assignable: the surplus arguments are simply ignored (e.g. `(v) => …`
    // for an `(element, index) => …` slot).
    let source = function_type(vec![Type::String], Type::Number, false, 1);
    let target = function_type(vec![Type::String, Type::Number], Type::Number, false, 2);

    assert!(is_assignable_to(&source, &target));
}

#[test]
fn function_type_not_assignable_more_required_source_parameters() {
    // The source requires two parameters but the target only ever supplies
    // one, so it cannot be called safely.
    let source = function_type(vec![Type::String, Type::Number], Type::Number, false, 2);
    let target = function_type(vec![Type::String], Type::Number, false, 1);

    assert!(!is_assignable_to(&source, &target));
}

#[test]
fn function_type_not_assignable_parameter_mismatch() {
    let source = function_type(vec![Type::Number], Type::Number, false, 1);
    let target = function_type(vec![Type::String], Type::Number, false, 1);

    assert!(!is_assignable_to(&source, &target));
}

#[test]
fn function_type_not_assignable_return_mismatch() {
    let source = function_type(vec![Type::String], Type::String, false, 1);
    let target = function_type(vec![Type::String], Type::Number, false, 1);

    assert!(!is_assignable_to(&source, &target));
}

#[test]
fn function_type_return_covariance_basic() {
    let source = function_type(
        vec![Type::String],
        Type::StringLiteral("ok".to_string()),
        false,
        1,
    );
    let target = function_type(vec![Type::String], Type::String, false, 1);

    assert!(is_assignable_to(&source, &target));
}

#[test]
fn function_type_literal_parameter_conservative() {
    let source = function_type(
        vec![Type::StringLiteral("ok".to_string())],
        Type::Void,
        false,
        1,
    );
    let target = function_type(vec![Type::String], Type::Void, false, 1);

    assert!(!is_assignable_to(&source, &target));
}

#[test]
fn function_type_union_parameter_compatible_when_same() {
    let source = function_type(
        vec![union_type(vec![Type::String, Type::Number])],
        Type::Void,
        false,
        1,
    );
    let target = function_type(
        vec![union_type(vec![Type::String, Type::Number])],
        Type::Void,
        false,
        1,
    );

    assert!(is_assignable_to(&source, &target));
}

#[test]
fn function_type_void_return_assignable_to_void_return() {
    let source = function_type(vec![Type::String], Type::Void, false, 1);
    let target = function_type(vec![Type::String], Type::Void, false, 1);

    assert!(is_assignable_to(&source, &target));
}

#[test]
fn function_type_void_return_not_assignable_to_string_return() {
    let source = function_type(vec![Type::String], Type::Void, false, 1);
    let target = function_type(vec![Type::String], Type::String, false, 1);

    assert!(!is_assignable_to(&source, &target));
}

#[test]
fn array_type_name_string() {
    assert_eq!(Type::Array(Box::new(Type::String)).name(), "string[]");
}

#[test]
fn array_type_name_number() {
    assert_eq!(Type::Array(Box::new(Type::Number)).name(), "number[]");
}

#[test]
fn array_type_name_literal() {
    assert_eq!(
        Type::Array(Box::new(Type::StringLiteral("ok".to_string()))).name(),
        r#""ok"[]"#
    );
}

#[test]
fn array_type_name_union() {
    assert_eq!(
        Type::Array(Box::new(union_type(vec![Type::String, Type::Number]))).name(),
        "(string | number)[]"
    );
}

#[test]
fn array_type_name_function() {
    assert_eq!(
        Type::Array(Box::new(function_type(vec![], Type::String, false, 0))).name(),
        "(() => string)[]"
    );
}

#[test]
fn array_assignable_same_element() {
    assert!(is_assignable_to(
        &Type::Array(Box::new(Type::String)),
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn array_assignable_literal_element_to_base_element() {
    assert!(is_assignable_to(
        &Type::Array(Box::new(Type::StringLiteral("ok".to_string()))),
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn array_not_assignable_base_element_to_literal_element() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::String)),
        &Type::Array(Box::new(Type::StringLiteral("ok".to_string())))
    ));
}

#[test]
fn array_not_assignable_different_element() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::Number)),
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn array_any_element_assignability() {
    assert!(is_assignable_to(
        &Type::Array(Box::new(Type::Any)),
        &Type::Array(Box::new(Type::String))
    ));
    assert!(is_assignable_to(
        &Type::Array(Box::new(Type::String)),
        &Type::Array(Box::new(Type::Any))
    ));
}

#[test]
fn array_not_assignable_to_primitive() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::String)),
        &Type::String
    ));
}

#[test]
fn primitive_not_assignable_to_array() {
    assert!(!is_assignable_to(
        &Type::String,
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn array_union_assignability_valid() {
    assert!(is_assignable_to(
        &Type::Array(Box::new(union_type(vec![Type::String, Type::Number]))),
        &Type::Array(Box::new(union_type(vec![Type::String, Type::Number])))
    ));
}

#[test]
fn array_union_assignability_mismatch() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::Boolean)),
        &Type::Array(Box::new(union_type(vec![Type::String, Type::Number])))
    ));
}

#[test]
fn array_function_element_assignability_valid() {
    assert!(is_assignable_to(
        &Type::Array(Box::new(function_type(vec![], Type::Void, false, 0))),
        &Type::Array(Box::new(function_type(vec![], Type::Void, false, 0)))
    ));
}

#[test]
fn array_function_element_assignability_mismatch() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(function_type(
            vec![Type::String],
            Type::Void,
            false,
            1
        ))),
        &Type::Array(Box::new(function_type(vec![], Type::Void, false, 0)))
    ));
}

#[test]
fn tuple_type_name_empty_if_supported() {
    assert_eq!(Type::tuple(vec![]).name(), "[]");
}

#[test]
fn tuple_type_name_one_element() {
    assert_eq!(Type::tuple(vec![Type::String]).name(), "[string]");
}

#[test]
fn tuple_type_name_two_elements() {
    assert_eq!(
        Type::tuple(vec![Type::String, Type::Number]).name(),
        "[string, number]"
    );
}

#[test]
fn tuple_type_name_literal_element() {
    assert_eq!(
        Type::tuple(vec![Type::StringLiteral("ok".to_string()), Type::Number]).name(),
        r#"["ok", number]"#
    );
}

#[test]
fn tuple_type_name_union_element() {
    assert_eq!(
        Type::tuple(vec![
            union_type(vec![Type::String, Type::Number]),
            Type::Boolean
        ])
        .name(),
        "[string | number, boolean]"
    );
}

#[test]
fn tuple_type_name_function_element() {
    assert_eq!(
        Type::tuple(vec![
            function_type(vec![], Type::Void, false, 0),
            Type::String,
        ])
        .name(),
        "[() => void, string]"
    );
}

#[test]
fn tuple_type_name_object_element() {
    let mut properties = PropertyMap::default();
    properties.insert("name".into(), ObjectProperty::required(Type::String));

    assert_eq!(
        Type::tuple(vec![
            Type::Object(ObjectType::new(properties, None)),
            Type::Number
        ])
        .name(),
        "[{ name: string; }, number]"
    );
}

#[test]
fn tuple_type_name_array_element() {
    assert_eq!(
        Type::tuple(vec![Type::Array(Box::new(Type::String)), Type::Number]).name(),
        "[string[], number]"
    );
}

#[test]
fn tuple_type_name_nested_tuple() {
    assert_eq!(
        Type::tuple(vec![
            Type::tuple(vec![Type::String, Type::Number]),
            Type::Boolean,
        ])
        .name(),
        "[[string, number], boolean]"
    );
}

#[test]
fn tuple_assignable_same_shape() {
    assert!(is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &Type::tuple(vec![Type::String, Type::Number])
    ));
}

#[test]
fn tuple_assignable_literal_to_base() {
    assert!(is_assignable_to(
        &Type::tuple(vec![
            Type::StringLiteral("ok".to_string()),
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            })
        ]),
        &Type::tuple(vec![Type::String, Type::Number])
    ));
}

#[test]
fn tuple_not_assignable_base_to_literal() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &Type::tuple(vec![
            Type::StringLiteral("ok".to_string()),
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            })
        ])
    ));
}

#[test]
fn tuple_not_assignable_different_length_too_short() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String]),
        &Type::tuple(vec![Type::String, Type::Number])
    ));
}

#[test]
fn tuple_not_assignable_different_length_too_long() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &Type::tuple(vec![Type::String])
    ));
}

#[test]
fn tuple_not_assignable_element_mismatch() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String, Type::String]),
        &Type::tuple(vec![Type::String, Type::Number])
    ));
}

#[test]
fn tuple_assignable_to_array_when_elements_compatible() {
    assert!(is_assignable_to(
        &Type::tuple(vec![Type::String, Type::StringLiteral("ok".to_string())]),
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn tuple_not_assignable_to_array_when_element_mismatch() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &Type::Array(Box::new(Type::String))
    ));
}

#[test]
fn array_not_assignable_to_tuple() {
    assert!(!is_assignable_to(
        &Type::Array(Box::new(Type::String)),
        &Type::tuple(vec![Type::String, Type::String])
    ));
}

#[test]
fn tuple_any_assignability() {
    assert!(is_assignable_to(
        &Type::Any,
        &Type::tuple(vec![Type::String, Type::Number])
    ));
    assert!(is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &Type::Any
    ));
}

#[test]
fn tuple_union_assignability_valid() {
    assert!(is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &union_type(vec![
            Type::tuple(vec![Type::String, Type::Number]),
            Type::String
        ])
    ));
}

#[test]
fn tuple_union_assignability_mismatch() {
    assert!(!is_assignable_to(
        &Type::tuple(vec![Type::String, Type::Number]),
        &union_type(vec![Type::tuple(vec![Type::String, Type::String])])
    ));
}

fn plain_object(entries: Vec<(&str, Type)>) -> Type {
    let mut properties = PropertyMap::default();
    for (name, ty) in entries {
        properties.insert(name.into(), ObjectProperty::required(ty));
    }
    Type::Object(ObjectType {
        properties: Arc::new(properties),
        property_map_id: None,
        string_index_type: None,
        number_index_type: None,
        string_index_readonly: false,
        number_index_readonly: false,
        alias_name: None,
        alias_id: None,
        construct_signature: None,
        call_signature: None,
        is_intersection: false,
        synthetic_open_index: false,
        keyed_indexes: None,
        index_parameter_names: None,
        non_primitive: false,
        without_inferable_index: false,
        spread_operands_last: false,
                intersection_operands: None,
    })
}

/// Diamond-shaped nesting: each level's `a` and `b` share the same child type
/// (one `Arc`), and the source carries an extra `tag` so no identity/equality
/// fast path fires. Without the relation cache each sibling re-compares the
/// same child pair, which is 2^depth — at depth 64 this test only terminates
/// because completed sub-results are memoized.
fn diamond(depth: usize, leaf: Type, tagged: bool) -> Type {
    let mut current = if tagged {
        plain_object(vec![("leaf", leaf), ("tag", Type::Number)])
    } else {
        plain_object(vec![("leaf", leaf)])
    };
    for _ in 0..depth {
        let mut entries = vec![("a", current.clone()), ("b", current)];
        if tagged {
            entries.push(("tag", Type::Number));
        }
        current = plain_object(entries);
    }
    current
}

#[test]
fn diamond_object_assignability_terminates_and_accepts() {
    let source = diamond(64, Type::String, true);
    let target = diamond(64, Type::String, false);
    assert!(is_assignable_to(&source, &target));
}

#[test]
fn diamond_object_assignability_terminates_and_rejects_leaf_mismatch() {
    let source = diamond(64, Type::String, true);
    let target = diamond(64, Type::Number, false);
    assert!(!is_assignable_to(&source, &target));
}
