use super::*;

#[test]
fn parse_object_type_literal_accepts_commas_semicolons_and_trailing_separator() {
    let cases = [
        "let user: { name: string; age: number } = { name: \"Ada\", age: 36 };",
        "let user: { name: string, age: number } = { name: \"Ada\", age: 36 };",
        "let user: { name: string; age: number; } = { name: \"Ada\", age: 36 };",
    ];

    for source in cases {
        let parsed = parse_source(source, "example.ts");
        assert!(
            parsed.parser_errors.is_empty(),
            "unexpected parser errors for {source}"
        );
        assert_eq!(parsed.statements.len(), 1);

        let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
            panic!("expected a variable declaration");
        };

        let Some(ParsedType::Object(object_type)) = variable.declared_type.as_ref() else {
            panic!("expected an object type annotation");
        };

        assert_eq!(object_type.properties.len(), 2);
        assert_eq!(object_type.properties[0].name, "name");
        assert_eq!(object_type.properties[1].name, "age");
    }
}

#[test]
fn parse_object_type_literal_keeps_optional_property_metadata() {
    let parsed = parse_source("let user: { name?: string } = {};", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Object(object_type)) = variable.declared_type.as_ref() else {
        panic!("expected an object type annotation");
    };

    assert_eq!(object_type.properties.len(), 1);
    assert!(object_type.properties[0].optional);
    assert_eq!(object_type.properties[0].name, "name");
}

#[test]
fn parse_object_type_literal_accepts_mixed_optional_separators() {
    let cases = [
        (
            "let user: { name?: string; age: number } = { age: 1 };",
            vec![("name", true), ("age", false)],
        ),
        (
            "let user: { name?: string, age?: number } = {};",
            vec![("name", true), ("age", true)],
        ),
        (
            "let user: { name?: string; age?: number; } = {};",
            vec![("name", true), ("age", true)],
        ),
    ];

    for (source, expected_properties) in cases {
        let parsed = parse_source(source, "example.ts");
        assert!(
            parsed.parser_errors.is_empty(),
            "unexpected parser errors for {source}"
        );
        assert_eq!(parsed.statements.len(), 1);

        let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
            panic!("expected a variable declaration");
        };

        let Some(ParsedType::Object(object_type)) = variable.declared_type.as_ref() else {
            panic!("expected an object type annotation");
        };

        assert_eq!(object_type.properties.len(), expected_properties.len());

        for ((expected_name, expected_optional), property) in expected_properties
            .into_iter()
            .zip(object_type.properties.iter())
        {
            assert_eq!(property.name, expected_name);
            assert_eq!(property.optional, expected_optional);
        }
    }
}

#[test]
fn parse_union_type_literal_keeps_constituents_and_metadata() {
    let parsed = parse_source(
        "let value: { name?: string } | undefined = { name: \"Ada\" };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let ParsedType::Union(types) = variable.declared_type.as_ref().unwrap() else {
        panic!("expected a union type");
    };
    assert_eq!(types.len(), 2);
}

#[test]
fn parse_type_query_identifier() {
    let parsed = parse_source("type Config = typeof config;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::TypeOf(type_of) = &alias.ty else {
        panic!("expected a typeof type");
    };
    assert_eq!(type_of.name, "config");
}

#[test]
fn parse_type_operator_keyof() {
    let parsed = parse_source("type Keys = keyof Config;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::KeyOf(inner) = &alias.ty else {
        panic!("expected a keyof type");
    };

    let ParsedType::Named(named) = &**inner else {
        panic!("expected a named inner type");
    };
    assert_eq!(named.name, "Config");
}

#[test]
fn parse_type_operator_keyof_typeof() {
    let parsed = parse_source("type ConfigKeys = keyof typeof config;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::KeyOf(inner) = &alias.ty else {
        panic!("expected a keyof type");
    };

    let ParsedType::TypeOf(type_of) = &**inner else {
        panic!("expected a typeof inner type");
    };
    assert_eq!(type_of.name, "config");
}

#[test]
fn parse_union_types_accepts_multiple_constituents() {
    let parsed = parse_source(
        "let value: string | number | undefined = \"ok\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Union(types)) = variable.declared_type.as_ref() else {
        panic!("expected a union type annotation");
    };

    assert_eq!(types.len(), 3);
    assert!(matches!(types[0], ParsedType::String));
    assert!(matches!(types[1], ParsedType::Number));
    assert!(matches!(types[2], ParsedType::Undefined));
}

#[test]
fn parse_string_literal_type() {
    let parsed = parse_source("let value: \"ok\" = \"ok\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::StringLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a string literal type");
    };

    assert_eq!(value, "ok");
}

#[test]
fn parse_single_quoted_string_literal_type() {
    let parsed = parse_source("let value: 'ok' = 'ok';", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::StringLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a string literal type");
    };

    assert_eq!(value, "ok");
}

#[test]
fn parse_tsx_source_uses_typescript_entry_without_jsx_syntax() {
    let parsed = parse_source("const value: string = 1;", "example.tsx");
    assert!(
        parsed.parser_errors.is_empty(),
        "tsx source should parse TypeScript syntax without parser errors"
    );
    assert_eq!(parsed.statements.len(), 1);
}

fn first_initializer(source: &str) -> ParsedExpression {
    let parsed = parse_source(source, "example.tsx");
    assert!(
        parsed.parser_errors.is_empty(),
        "unexpected parser errors for {source}: {:?}",
        parsed.parser_errors
    );
    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration for {source}");
    };
    variable
        .initializer
        .clone()
        .unwrap_or_else(|| panic!("expected an initializer for {source}"))
}

#[test]
fn parse_tsx_jsx_element_variants_without_parser_errors() {
    for source in [
        "const a = <div />;",
        "const b = <div></div>;",
        "const c = <div id=\"x\">hello</div>;",
        "const d = <div>{value}</div>;",
        "const e = <><span /></>;",
        "const f = <section><span>{name}</span></section>;",
        "const g = <UI.Button />;",
        "const h = <Button value={count} enabled />;",
    ] {
        let parsed = parse_source(source, "example.tsx");
        assert!(
            parsed.parser_errors.is_empty(),
            "tsx source should parse without parser errors for {source}: {:?}",
            parsed.parser_errors
        );
        assert_eq!(parsed.statements.len(), 1, "for {source}");
    }
}

#[test]
fn parse_tsx_intrinsic_element_has_no_component_reference() {
    let ParsedExpression::JsxElement {
        tag_name,
        component_name,
        ..
    } = first_initializer("const a = <div id=\"root\">hello</div>;")
    else {
        panic!("expected a JSX element");
    };
    assert_eq!(tag_name, "div");
    // Lowercase intrinsic tags are not value references.
    assert_eq!(component_name, None);
}

#[test]
fn parse_tsx_component_element_records_value_reference() {
    let ParsedExpression::JsxElement {
        tag_name,
        component_name,
        ..
    } = first_initializer("const a = <Button value={count} enabled />;")
    else {
        panic!("expected a JSX element");
    };
    assert_eq!(tag_name, "Button");
    // Capitalized components resolve as value references so missing names report TS2304.
    assert_eq!(component_name, Some("Button".to_string()));
}

#[test]
fn parse_tsx_member_element_resolves_head_identifier() {
    let ParsedExpression::JsxElement {
        tag_name,
        component_name,
        ..
    } = first_initializer("const a = <UI.Button />;")
    else {
        panic!("expected a JSX element");
    };
    assert_eq!(tag_name, "UI.Button");
    // The head of a member tag is the value that must resolve in scope.
    assert_eq!(component_name, Some("UI".to_string()));
}

#[test]
fn parse_tsx_expression_child_preserves_inner_expression() {
    let ParsedExpression::JsxElement { children, .. } =
        first_initializer("const a = <div>{value}</div>;")
    else {
        panic!("expected a JSX element");
    };
    let expression_child = children
        .iter()
        .find_map(|child| match child {
            ParsedJsxChild::Expression {
                expression: Some(expression),
                ..
            } => Some(expression),
            _ => None,
        })
        .expect("expected an expression container child");
    assert!(matches!(
        expression_child,
        ParsedExpression::Identifier { name, .. } if name == "value"
    ));
}

#[test]
fn parse_tsx_fragment_walks_children() {
    let ParsedExpression::JsxFragment { children, .. } =
        first_initializer("const a = <><span /></>;")
    else {
        panic!("expected a JSX fragment");
    };
    assert!(
        children
            .iter()
            .any(|child| matches!(child, ParsedJsxChild::Element(_))),
        "fragment should retain its nested element child"
    );
}

#[test]
fn parse_ts_angle_brackets_are_not_parsed_as_jsx() {
    // In `.ts` mode `<T>` is a type assertion / generic, never a JSX element.
    let parsed = parse_source("const x = identity<number>(1);", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };
    assert!(
        !matches!(
            variable.initializer,
            Some(ParsedExpression::JsxElement { .. } | ParsedExpression::JsxFragment { .. })
        ),
        "angle brackets in .ts must not lower to JSX"
    );
}

#[test]
fn parse_number_literal_type() {
    let parsed = parse_source("let value: 1 = 1;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::NumberLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a number literal type");
    };

    assert_eq!(value, "1");
}

#[test]
fn parse_decimal_number_literal_type() {
    let parsed = parse_source("let value: 1.5 = 1.5;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::NumberLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a decimal number literal type");
    };

    assert_eq!(value, "1.5");
}

#[test]
fn parse_boolean_literal_type() {
    let parsed = parse_source("let value: true = true;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::BooleanLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a boolean literal type");
    };

    assert!(*value);
}

#[test]
fn parse_boolean_false_literal_type() {
    let parsed = parse_source("let value: false = false;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::BooleanLiteral(value)) = variable.declared_type.as_ref() else {
        panic!("expected a boolean literal type");
    };

    assert!(!*value);
}

#[test]
fn parse_literal_union_type() {
    let parsed = parse_source(
        "type Status = \"idle\" | \"loading\" | \"done\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Union(types) = &alias.ty else {
        panic!("expected a union alias");
    };

    assert!(matches!(types[0], ParsedType::StringLiteral(_)));
    assert!(matches!(types[1], ParsedType::StringLiteral(_)));
    assert!(matches!(types[2], ParsedType::StringLiteral(_)));
}

#[test]
fn parse_intersection_type() {
    let parsed = parse_source("type AB = A & B & C;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Intersection(types) = &alias.ty else {
        panic!("expected an intersection alias");
    };

    assert_eq!(types.len(), 3);
    assert!(types.iter().all(|ty| matches!(ty, ParsedType::Named(_))));
}

#[test]
fn parse_void_type() {
    let parsed = parse_source("let value: void = undefined;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.declared_type.as_ref(),
        Some(ParsedType::Void)
    ));
}

#[test]
fn parse_function_type_no_params() {
    let parsed = parse_source("type Fn = () => string;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    assert!(function_type.parameters.is_empty());
    assert!(matches!(&*function_type.return_type, ParsedType::String));
}

#[test]
fn parse_function_type_one_param() {
    let parsed = parse_source("type Fn = (value: string) => number;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    assert_eq!(function_type.parameters.len(), 1);
    assert_eq!(function_type.parameters[0].name.as_deref(), Some("value"));
    assert!(matches!(function_type.parameters[0].ty, ParsedType::String));
    assert!(matches!(&*function_type.return_type, ParsedType::Number));
}

#[test]
fn parse_function_type_multiple_params() {
    let parsed = parse_source(
        "type Fn = (value: string, count: number) => boolean;",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    assert_eq!(function_type.parameters.len(), 2);
    assert_eq!(function_type.parameters[0].name.as_deref(), Some("value"));
    assert_eq!(function_type.parameters[1].name.as_deref(), Some("count"));
    assert!(matches!(&*function_type.return_type, ParsedType::Boolean));
}

#[test]
fn parse_function_type_literal_param() {
    let parsed = parse_source("type Fn = (value: \"idle\") => void;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    let ParsedType::StringLiteral(value) = &function_type.parameters[0].ty else {
        panic!("expected a string literal parameter type");
    };

    assert_eq!(value, "idle");
    assert!(matches!(&*function_type.return_type, ParsedType::Void));
}

#[test]
fn parse_function_type_union_param() {
    let parsed = parse_source(
        "type Fn = (value: \"idle\" | \"done\") => void;",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    let ParsedType::Union(types) = &function_type.parameters[0].ty else {
        panic!("expected a union parameter type");
    };

    assert_eq!(types.len(), 2);
}

#[test]
fn parse_function_type_nested_param() {
    let parsed = parse_source("type Fn = (callback: () => string) => void;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    let ParsedType::Function(callback_type) = &function_type.parameters[0].ty else {
        panic!("expected a nested function type parameter");
    };

    assert!(callback_type.parameters.is_empty());
    assert!(matches!(&*callback_type.return_type, ParsedType::String));
}

#[test]
fn parse_function_type_function_return() {
    let parsed = parse_source("type Fn = (value: string) => () => string;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type alias");
    };

    let ParsedType::Function(return_type) = &*function_type.return_type else {
        panic!("expected a nested return function type");
    };

    assert!(return_type.parameters.is_empty());
    assert!(matches!(&*return_type.return_type, ParsedType::String));
}

#[test]
fn parse_function_type_in_type_alias() {
    let parsed = parse_source("type Mapper = (value: string) => number;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(alias.ty, ParsedType::Function(_)));
}

#[test]
fn parse_function_type_in_object_property() {
    let parsed = parse_source(
        "let store: { getState: () => string } = { getState: getState };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let ParsedType::Object(object_type) = variable.declared_type.as_ref().unwrap() else {
        panic!("expected an object type");
    };

    assert!(matches!(
        object_type.properties[0].ty,
        ParsedType::Function(_)
    ));
}

#[test]
fn parse_function_type_in_interface_member() {
    let parsed = parse_source("interface Store { getState: () => string; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert!(matches!(interface.members[0].ty, ParsedType::Function(_)));
}

#[test]
fn parse_function_type_malformed_no_panic() {
    let parsed = parse_source("type Fn = (value: string) => ;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_missing_arrow_no_panic() {
    let parsed = parse_source("type Fn = (value: string) string;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_missing_return_no_panic() {
    let parsed = parse_source("type Fn = (value: string) =>", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_missing_parameter_type_no_panic() {
    let parsed = parse_source("type Fn = (value) => string;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_optional_param_unsupported_no_panic() {
    let parsed = parse_source("type Fn = (value?: string) => void;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_rest_param_unsupported_no_panic() {
    let parsed = parse_source("type Fn = (...args: string[]) => void;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_generic_type_parameters_parsed() {
    let parsed = parse_source("type Fn = <T>(value: T) => T;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert_eq!(function_type.type_parameters.len(), 1);
    assert_eq!(function_type.parameters.len(), 1);
}

#[test]
fn parse_function_type_parameter_one() {
    let parsed = parse_source("type Fn = <T>(value: T) => T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert_eq!(function_type.type_parameters.len(), 1);
    assert_eq!(function_type.type_parameters[0].name, "T");
}

#[test]
fn parse_function_type_parameter_multiple() {
    let parsed = parse_source("type Fn = <A, B>(value: A) => B;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert_eq!(function_type.type_parameters.len(), 2);
}

#[test]
fn parse_function_type_parameter_constraint() {
    let parsed = parse_source("type Fn = <T extends string>(value: T) => T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert_eq!(function_type.type_parameters[0].name, "T");
    assert!(matches!(
        function_type.type_parameters[0].constraint,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_function_type_parameter_default() {
    let parsed = parse_source("type Fn = <T = string>(value: T) => T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert!(matches!(
        function_type.type_parameters[0].default_type,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_function_type_alias_outer_type_parameter() {
    let parsed = parse_source("type Fn<T> = <U>(value: T) => U;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Function(function_type) = &alias.ty else {
        panic!("expected a function type");
    };

    assert_eq!(alias.type_parameters.len(), 1);
    assert_eq!(function_type.type_parameters.len(), 1);
}

#[test]
fn parse_type_parameter_missing_name_no_panic() {
    let parsed = parse_source("type Box<> = string;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_type_parameter_missing_close_angle_no_panic() {
    let parsed = parse_source("type Box<T = string = { value: T };", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_type_parameter_missing_default_no_panic() {
    let parsed = parse_source("type Box<T = > = string;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_type_parameter_missing_constraint_no_panic() {
    let parsed = parse_source("type Box<T extends> = string;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_duplicate_type_parameter_names_parser_safe() {
    let parsed = parse_source("type Pair<T, T> = [T, T];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_declaration_generic_type_parameters_parsed() {
    let parsed = parse_source(
        "function identity<T>(value: T): T { return value; }",
        "example.ts",
    );

    assert_eq!(parsed.file_name, "example.ts");
    let ParsedStatement::FunctionDeclaration(function) = &parsed.statements[0] else {
        panic!("expected a function declaration");
    };

    assert_eq!(function.type_parameters.len(), 1);
    assert_eq!(function.parameters.len(), 1);
}

#[test]
fn parse_function_declaration_object_binding_parameter_parsed() {
    let parsed = parse_source(
        "function identity({ id: userId, name }) { return userId + name; }",
        "example.ts",
    );

    assert_eq!(parsed.file_name, "example.ts");
    let ParsedStatement::FunctionDeclaration(function) = &parsed.statements[0] else {
        panic!("expected a function declaration");
    };

    assert_eq!(function.parameters.len(), 1);
    let ParsedBindingName::ObjectPattern(pattern) = &function.parameters[0].binding_name else {
        panic!("expected an object binding pattern");
    };

    assert_eq!(pattern.elements.len(), 2);
    assert_eq!(pattern.elements[0].property_name, "id");
    assert!(matches!(
        pattern.elements[0].binding_name,
        ParsedBindingName::Identifier { ref name, .. } if name == "userId"
    ));
    assert_eq!(pattern.elements[1].property_name, "name");
    assert!(matches!(
        pattern.elements[1].binding_name,
        ParsedBindingName::Identifier { ref name, .. } if name == "name"
    ));
}

#[test]
fn parse_function_type_default_param_unsupported_no_panic() {
    let parsed = parse_source("type Fn = (value = \"ok\") => string;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_this_param_unsupported_no_panic() {
    let parsed = parse_source("type Fn = (this: string) => void;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_call_signature_unsupported_no_panic() {
    let parsed = parse_source("type Fn = { (): string };", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_construct_signature_unsupported_no_panic() {
    let parsed = parse_source("type Fn = { new (): string };", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_function_type_parenthesized_non_function_type_no_panic() {
    let parsed = parse_source("type Fn = (string);", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(alias.ty, ParsedType::String));
}

#[test]
fn parse_literal_type_alias() {
    let parsed = parse_source("type Status = \"idle\" | 1 | true;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Union(types) = &alias.ty else {
        panic!("expected a union alias");
    };

    assert!(matches!(types[0], ParsedType::StringLiteral(_)));
    assert!(matches!(types[1], ParsedType::NumberLiteral(_)));
    assert!(matches!(types[2], ParsedType::BooleanLiteral(true)));
}

#[test]
fn parse_literal_object_property_type() {
    let parsed = parse_source(
        "let event: { kind: \"click\" } = { kind: \"click\" };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Object(object_type)) = variable.declared_type.as_ref() else {
        panic!("expected an object type annotation");
    };

    let ParsedType::StringLiteral(value) = &object_type.properties[0].ty else {
        panic!("expected a string literal property type");
    };

    assert_eq!(value, "click");
}

#[test]
fn parse_literal_interface_property_type() {
    let parsed = parse_source("interface ClickEvent { kind: \"click\"; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    let ParsedType::StringLiteral(value) = &interface.members[0].ty else {
        panic!("expected a string literal member type");
    };

    assert_eq!(value, "click");
}

#[test]
fn parse_malformed_union_type_literal_does_not_panic() {
    let parsed = parse_source("let value: string | = \"ok\";", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_negative_literal_type_unsupported_no_panic() {
    let parsed = parse_source("type Value = -1;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_template_literal_type_unsupported_no_panic() {
    let parsed = parse_source("type Value = `x`;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_bigint_literal_type_unsupported_no_panic() {
    let parsed = parse_source("type Value = 1n;", "example.ts");

    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_undefined_literal_expression() {
    let parsed = parse_source("let value = undefined;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::UndefinedLiteral)
    ));
}

#[test]
fn parse_type_alias_primitive() {
    let parsed = parse_source("type Name = string;", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert_eq!(parsed.statements.len(), 1);

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert_eq!(alias.name, "Name");
    assert!(matches!(alias.ty, ParsedType::String));
}

#[test]
fn parse_type_alias_object() {
    let parsed = parse_source("type User = { name: string; age?: number };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Object(object_type) = &alias.ty else {
        panic!("expected an object alias");
    };

    assert_eq!(object_type.properties.len(), 2);
    assert_eq!(object_type.properties[0].name, "name");
    assert_eq!(object_type.properties[1].name, "age");
}

#[test]
fn parse_type_alias_union() {
    let parsed = parse_source("type MaybeName = string | undefined;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Union(types) = &alias.ty else {
        panic!("expected a union alias");
    };

    assert_eq!(types.len(), 2);
    assert!(matches!(types[0], ParsedType::String));
    assert!(matches!(types[1], ParsedType::Undefined));
}

#[test]
fn parse_type_alias_named_reference() {
    let parsed = parse_source(
        "type Name = string; let value: Name = \"ok\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.name, "Name");
}

#[test]
fn parse_type_reference_type_arguments_parsed() {
    let parsed = parse_source(
        "type Box<T> = T; let value: Box<string> = \"ok\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.type_arguments.len(), 1);
    assert!(matches!(named.type_arguments[0], ParsedType::String));
}

#[test]
fn parse_type_alias_named_reference_inside_object_property() {
    let parsed = parse_source(
        "type Name = string; type User = { name: Name };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[1] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Object(object_type) = &alias.ty else {
        panic!("expected an object alias");
    };

    let ParsedType::Named(named) = &object_type.properties[0].ty else {
        panic!("expected a named property type");
    };

    assert_eq!(named.name, "Name");
}

#[test]
fn parse_type_alias_named_reference_inside_union() {
    let parsed = parse_source(
        "type Name = string; type MaybeName = Name | undefined;",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[1] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Union(types) = &alias.ty else {
        panic!("expected a union alias");
    };

    assert!(matches!(types[0], ParsedType::Named(_)));
}

#[test]
fn parse_type_alias_generic_type_parameters_parsed() {
    let parsed = parse_source("type Box<T> = T;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert_eq!(alias.type_parameters.len(), 1);
    let ParsedType::Named(named) = &alias.ty else {
        panic!("expected a named type");
    };
    assert_eq!(named.name, "T");
}

#[test]
fn parse_type_alias_type_parameter_one() {
    let parsed = parse_source("type Box<T> = T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert_eq!(alias.type_parameters.len(), 1);
}

#[test]
fn parse_type_alias_type_parameter_multiple() {
    let parsed = parse_source("type Pair<A, B> = [A, B];", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert_eq!(alias.type_parameters.len(), 2);
}

#[test]
fn parse_type_alias_type_parameter_trailing_comma() {
    let parsed = parse_source("type Pair<T,> = [T, T];", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert_eq!(alias.type_parameters.len(), 1);
}

#[test]
fn parse_type_alias_type_parameter_constraint() {
    let parsed = parse_source("type Box<T extends string> = T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(
        alias.type_parameters[0].constraint,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_type_alias_type_parameter_default() {
    let parsed = parse_source("type Box<T = string> = T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(
        alias.type_parameters[0].default_type,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_type_alias_type_parameter_constraint_and_default() {
    let parsed = parse_source("type Box<T extends string = string> = T;", "example.ts");
    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(
        alias.type_parameters[0].constraint,
        Some(ParsedType::String)
    ));
    assert!(matches!(
        alias.type_parameters[0].default_type,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_type_alias_malformed_no_panic() {
    let parsed = parse_source("type Name = ;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_required_property() {
    let parsed = parse_source("interface User { name: string; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert_eq!(parsed.statements.len(), 1);

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.name, "User");
    assert_eq!(interface.members.len(), 1);
    assert_eq!(interface.members[0].name, "name");
    assert!(!interface.members[0].optional);
    assert!(matches!(interface.members[0].ty, ParsedType::String));
}

#[test]
fn parse_interface_optional_property() {
    let parsed = parse_source("interface User { name?: string; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.members.len(), 1);
    assert!(interface.members[0].optional);
}

#[test]
fn parse_interface_multiple_properties_semicolon() {
    let parsed = parse_source(
        "interface User { name: string; age: number; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.members.len(), 2);
    assert_eq!(interface.members[0].name, "name");
    assert_eq!(interface.members[1].name, "age");
}

#[test]
fn parse_interface_multiple_properties_comma() {
    let parsed = parse_source("interface User { name: string, age: number }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.members.len(), 2);
}

#[test]
fn parse_interface_multiple_properties_newline_or_current_separator() {
    let parsed = parse_source(
        "interface User {\n  name: string\n  age: number\n}",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.members.len(), 2);
}

#[test]
fn parse_interface_property_union_type() {
    let parsed = parse_source("interface User { name: string | undefined; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert!(matches!(interface.members[0].ty, ParsedType::Union(_)));
}

#[test]
fn parse_interface_property_named_type() {
    let parsed = parse_source(
        "type Name = string; interface User { name: Name; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[1] else {
        panic!("expected an interface declaration");
    };

    assert!(matches!(interface.members[0].ty, ParsedType::Named(_)));
}

#[test]
fn parse_interface_top_level_statement_order() {
    let parsed = parse_source(
        "let user: User = { name: \"Ada\" }; interface User { name: string; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        parsed.statements[0],
        ParsedStatement::VariableDeclaration(_)
    ));
    assert!(matches!(
        parsed.statements[1],
        ParsedStatement::InterfaceDeclaration(_)
    ));
}

#[test]
fn parse_interface_malformed_no_panic() {
    let parsed = parse_source("interface User { name }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_extends_unsupported_no_panic() {
    let parsed = parse_source(
        "interface User extends Base { name: string; }",
        "example.ts",
    );
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_generic_type_parameters_parsed() {
    let parsed = parse_source("interface Box<T> { value: T; }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert_eq!(parsed.statements.len(), 1);

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.type_parameters.len(), 1);
    let ParsedType::Named(named) = &interface.members[0].ty else {
        panic!("expected a named member type");
    };
    assert_eq!(named.name, "T");
}

#[test]
fn parse_interface_type_parameter_one() {
    let parsed = parse_source("interface Box<T> { value: T; }", "example.ts");
    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.type_parameters.len(), 1);
}

#[test]
fn parse_interface_type_parameter_multiple() {
    let parsed = parse_source(
        "interface Pair<A, B> { first: A; second: B; }",
        "example.ts",
    );
    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.type_parameters.len(), 2);
}

#[test]
fn parse_interface_type_parameter_constraint() {
    let parsed = parse_source(
        "interface Box<T extends string> { value: T; }",
        "example.ts",
    );
    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert!(matches!(
        interface.type_parameters[0].constraint,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_interface_type_parameter_default() {
    let parsed = parse_source("interface Box<T = string> { value: T; }", "example.ts");
    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert!(matches!(
        interface.type_parameters[0].default_type,
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_interface_method_unsupported_no_panic() {
    let parsed = parse_source(
        "interface User { greet(): string; name: string; }",
        "example.ts",
    );
    assert_eq!(parsed.file_name, "example.ts");

    let ParsedStatement::InterfaceDeclaration(interface) = &parsed.statements[0] else {
        panic!("expected an interface declaration");
    };

    assert_eq!(interface.members.len(), 2);
    assert_eq!(interface.members[0].name, "greet");
    assert_eq!(interface.members[1].name, "name");
}

#[test]
fn parse_interface_call_signature_unsupported_no_panic() {
    let parsed = parse_source("interface Callable { (): string; }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_index_signature_unsupported_no_panic() {
    let parsed = parse_source(
        "interface MapLike { [key: string]: number; value: number; }",
        "example.ts",
    );
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_readonly_property_unsupported_no_panic() {
    let parsed = parse_source("interface User { readonly name: string; }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_missing_body_no_panic() {
    let parsed = parse_source("interface User", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_missing_member_type_no_panic() {
    let parsed = parse_source("interface User { name; }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_interface_malformed_member_no_panic() {
    let parsed = parse_source("interface User { name: ; }", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_no_args() {
    let parsed = parse_source("store.getState();", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall {
        object,
        property_name,
        arguments,
        ..
    } = expression.as_ref()
    else {
        panic!("expected a property call expression");
    };

    let ParsedExpression::Identifier {
        name: object_name, ..
    } = &**object
    else {
        panic!("expected an identifier");
    };

    assert_eq!(object_name, "store");
    assert_eq!(property_name, "getState");
    assert!(arguments.is_empty());
}

#[test]
fn parse_call_expression_type_arguments_parsed() {
    let parsed = parse_source("identity<string>(\"ok\");", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    match &parsed.statements[0] {
        ParsedStatement::Call(call) => {
            assert_eq!(call.callee_name, "identity");
            assert_eq!(call.type_arguments.len(), 1);
            assert_eq!(call.arguments.len(), 1);
        }
        ParsedStatement::Expression(expression)
            if matches!(expression.as_ref(), ParsedExpression::Call { .. }) =>
        {
            let ParsedExpression::Call {
                callee_name,
                type_arguments,
                arguments,
                ..
            } = expression.as_ref()
            else {
                unreachable!()
            };
            assert_eq!(callee_name, "identity");
            assert_eq!(type_arguments.len(), 1);
            assert_eq!(arguments.len(), 1);
        }
        _ => panic!("expected a call statement or call expression"),
    }
}

#[test]
fn parse_named_type_reference_type_argument_one() {
    let parsed = parse_source(
        "type Box<T> = T; let value: Box<string> = \"ok\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.type_arguments.len(), 1);
}

#[test]
fn parse_named_type_reference_type_argument_multiple() {
    let parsed = parse_source(
        "type Pair<A, B> = [A, B]; let value: Pair<string, number> = [\"a\", 1];",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.type_arguments.len(), 2);
}

#[test]
fn parse_named_type_reference_type_argument_nested() {
    let parsed = parse_source(
        "type Box<T> = { value: T }; let value: Box<Box<string>> = { value: { value: \"ok\" } };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.type_arguments.len(), 1);
    assert!(matches!(named.type_arguments[0], ParsedType::Named(_)));
}

#[test]
fn parse_named_type_reference_type_argument_union() {
    let parsed = parse_source(
        "type Box<T> = { value: T }; let value: Box<string | number> = { value: 1 };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert!(matches!(named.type_arguments[0], ParsedType::Union(_)));
}

#[test]
fn parse_named_type_reference_type_argument_tuple() {
    let parsed = parse_source(
        "type Box<T> = { value: T }; let value: Box<[string, number]> = { value: [\"a\", 1] };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert!(matches!(named.type_arguments[0], ParsedType::Tuple(..)));
}

#[test]
fn parse_named_type_reference_type_argument_function() {
    let parsed = parse_source(
        "type Box<T> = { value: T }; let value: Box<() => string> = { value: () => \"ok\" };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert!(matches!(named.type_arguments[0], ParsedType::Function(_)));
}

#[test]
fn parse_named_type_reference_type_argument_object() {
    let parsed = parse_source(
        "type Box<T> = { value: T }; let value: Box<{ name: string }> = { value: { name: \"ok\" } };",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert!(matches!(named.type_arguments[0], ParsedType::Object(_)));
}

#[test]
fn parse_named_type_reference_type_argument_trailing_comma() {
    let parsed = parse_source(
        "type Box<T> = T; let value: Box<string,> = \"ok\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[1] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Named(named)) = variable.declared_type.as_ref() else {
        panic!("expected a named type reference");
    };

    assert_eq!(named.type_arguments.len(), 1);
}

#[test]
fn parse_named_type_reference_missing_close_angle_no_panic() {
    let parsed = parse_source(
        "type Box<T> = T; let value: Box<string = \"ok\";",
        "example.ts",
    );
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_named_type_reference_missing_type_argument_no_panic() {
    let parsed = parse_source("type Box<T> = T; let value: Box<> = \"ok\";", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_call_expression_type_arguments_multiple() {
    let parsed = parse_source("pair<string, number>(\"a\", 1);", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Call(call) = &parsed.statements[0] else {
        panic!("expected a call statement");
    };

    assert_eq!(call.type_arguments.len(), 2);
}

#[test]
fn parse_call_expression_type_arguments_nested() {
    let parsed = parse_source("identity<Box<string>>(value);", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Call(call) = &parsed.statements[0] else {
        panic!("expected a call statement");
    };

    assert_eq!(call.type_arguments.len(), 1);
    assert!(matches!(call.type_arguments[0], ParsedType::Named(_)));
}

#[test]
fn parse_call_expression_type_arguments_missing_close_angle_no_panic() {
    let parsed = parse_source("identity<string(value);", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_expression_type_argument_parser_safe_or_pinned() {
    let parsed = parse_source("store.getState<string>();", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { type_arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert_eq!(type_arguments.len(), 1);
}

#[test]
fn parse_property_call_one_arg() {
    let parsed = parse_source("store.setState(\"next\");", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert_eq!(arguments.len(), 1);
}

#[test]
fn parse_property_call_multiple_args() {
    let parsed = parse_source("store.setState(\"next\", count, true);", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert_eq!(arguments.len(), 3);
}

#[test]
fn parse_property_call_argument_is_property_call() {
    let parsed = parse_source("store.setState(store.getState());", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert!(matches!(
        arguments[0].expression,
        ParsedExpression::PropertyCall { .. }
    ));
}

#[test]
fn parse_property_call_argument_is_call() {
    let parsed = parse_source("store.setState(getState());", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert!(matches!(
        arguments[0].expression,
        ParsedExpression::Call { .. }
    ));
}

#[test]
fn parse_property_call_argument_is_conditional() {
    let parsed = parse_source("store.setState(true ? \"next\" : \"prev\");", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Expression(expression) = &parsed.statements[0] else {
        panic!("expected a property call expression");
    };
    let ParsedExpression::PropertyCall { arguments, .. } = expression.as_ref() else {
        panic!("expected a property call expression");
    };

    assert!(matches!(
        arguments[0].expression,
        ParsedExpression::Conditional { .. }
    ));
}

#[test]
fn parse_property_call_in_variable_initializer() {
    let parsed = parse_source("let value = store.getState();", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::PropertyCall { .. })
    ));
}

#[test]
fn parse_property_call_in_return_statement() {
    let parsed = parse_source("function read() { return store.getState(); }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::FunctionDeclaration(function) = &parsed.statements[0] else {
        panic!("expected a function declaration");
    };

    let ParsedFunctionBodyStatement::Return(return_statement) = &function.body[0] else {
        panic!("expected a return statement");
    };

    assert!(matches!(
        return_statement.expression.as_ref(),
        Some(ParsedExpression::PropertyCall { .. })
    ));
}

#[test]
fn parse_property_call_in_assignment() {
    let parsed = parse_source("value = store.getState();", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::Assignment(assignment) = &parsed.statements[0] else {
        panic!("expected an assignment");
    };

    assert!(matches!(
        assignment.value,
        ParsedExpression::PropertyCall { .. }
    ));
}

#[test]
fn parse_arrow_function_expression_parsed() {
    let parsed = parse_source("const fn = ({ ctx, input }) => input;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedExpression::ArrowFunction(arrow)) = variable.initializer.as_ref() else {
        panic!("expected an arrow function expression");
    };

    assert_eq!(arrow.parameters.len(), 1);
    let ParsedBindingName::ObjectPattern(pattern) = &arrow.parameters[0].binding_name else {
        panic!("expected an object binding pattern");
    };

    assert_eq!(pattern.elements.len(), 2);
    assert!(matches!(
        &arrow.body,
        ParsedArrowFunctionBody::Expression(expression)
        if matches!(expression.as_ref(), ParsedExpression::Identifier { name, .. } if name == "input")
    ));
}

#[test]
fn parse_property_call_in_conditional_branch() {
    let parsed = parse_source(
        "let value = true ? store.getState() : \"fallback\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedExpression::Conditional { when_true, .. }) = variable.initializer.as_ref()
    else {
        panic!("expected a conditional expression");
    };

    assert!(matches!(
        when_true.as_ref(),
        ParsedExpression::PropertyCall { .. }
    ));
}

#[test]
fn parse_property_call_method_shorthand_unsupported_no_panic() {
    let parsed = parse_source(
        "({ getState() { return \"ok\"; } }).getState();",
        "example.ts",
    );
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_missing_close_paren_no_panic() {
    let parsed = parse_source("store.getState(", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_missing_property_name_no_panic() {
    let parsed = parse_source("store.();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_missing_object_no_panic() {
    let parsed = parse_source(".getState();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_nested_object_unsupported_no_panic() {
    let parsed = parse_source("app.store.getState();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_chained_unsupported_no_panic() {
    let parsed = parse_source("store.getApi().getState();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_optional_chaining_unsupported_no_panic() {
    let parsed = parse_source("store.getState?.();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_call_bracket_unsupported_no_panic() {
    let parsed = parse_source("store[\"getState\"]();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_array_type_string() {
    let parsed = parse_source("let value: string[] = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Array(element)) = variable.declared_type.as_ref() else {
        panic!("expected an array type");
    };

    assert!(matches!(&**element, ParsedType::String));
}

#[test]
fn parse_array_type_number() {
    let parsed = parse_source("let value: number[] = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.declared_type.as_ref(),
        Some(ParsedType::Array(_))
    ));
}

#[test]
fn parse_array_type_boolean() {
    let parsed = parse_source("let value: boolean[] = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.declared_type.as_ref(),
        Some(ParsedType::Array(_))
    ));
}

#[test]
fn parse_array_type_undefined() {
    let parsed = parse_source("let value: undefined[] = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.declared_type.as_ref(),
        Some(ParsedType::Array(_))
    ));
}

#[test]
fn parse_array_type_void() {
    let parsed = parse_source("let value: void[] = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.declared_type.as_ref(),
        Some(ParsedType::Array(_))
    ));
}

#[test]
fn parse_array_type_literal_element() {
    let parsed = parse_source("let value: \"ok\"[] = [\"ok\"];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedType::Array(element)) = variable.declared_type.as_ref() else {
        panic!("expected an array type");
    };

    assert!(matches!(&**element, ParsedType::StringLiteral(_)));
}

#[test]
fn parse_array_type_named_element() {
    let parsed = parse_source("type Status = string[];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(alias.ty, ParsedType::Array(_)));
}

#[test]
fn parse_array_type_object_element() {
    let parsed = parse_source("type Store = { name: string }[];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(alias.ty, ParsedType::Array(_)));
}

#[test]
fn parse_array_type_function_element() {
    let parsed = parse_source("type Listeners = (() => void)[];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let Some(ParsedType::Array(element)) = Some(&alias.ty) else {
        panic!("expected an array type");
    };

    assert!(matches!(&**element, ParsedType::Function(_)));
}

#[test]
fn parse_array_type_union_element_if_supported() {
    let parsed = parse_source("type Status = (\"idle\" | \"done\")[];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let Some(ParsedType::Array(element)) = Some(&alias.ty) else {
        panic!("expected an array type");
    };

    assert!(matches!(&**element, ParsedType::Union(types) if types.len() == 2));
}

#[test]
fn parse_array_type_nested_array() {
    let parsed = parse_source("type Matrix = string[][];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    assert!(matches!(alias.ty, ParsedType::Array(_)));
}

#[test]
fn parse_array_type_malformed_missing_close_bracket_no_panic() {
    let parsed = parse_source("let value: string[ = [];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_array_type_empty_brackets_without_element_no_panic() {
    let parsed = parse_source("let value: [] = [];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_array_type_array_generic_unsupported_no_panic() {
    let parsed = parse_source("let value: Array<string> = [];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_one_element() {
    let parsed = parse_source("type Pair = [string];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert_eq!(elements.len(), 1);
    assert!(matches!(elements[0], ParsedType::String));
}

#[test]
fn parse_tuple_type_empty_tuple_behavior_pinned() {
    let parsed = parse_source("type Empty = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(elements.is_empty());
}

#[test]
fn parse_tuple_type_two_elements() {
    let parsed = parse_source("type Pair = [string, number];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert_eq!(elements.len(), 2);
    assert!(matches!(elements[0], ParsedType::String));
    assert!(matches!(elements[1], ParsedType::Number));
}

#[test]
fn parse_tuple_type_three_elements() {
    let parsed = parse_source("type Trio = [string, number, boolean];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert_eq!(elements.len(), 3);
}

#[test]
fn parse_tuple_type_string_number() {
    let parsed = parse_source("let value: [string, number] = [\"Ada\", 36];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let ParsedType::Tuple(elements, ..) = variable.declared_type.as_ref().unwrap() else {
        panic!("expected a tuple type annotation");
    };

    assert!(matches!(elements[0], ParsedType::String));
    assert!(matches!(elements[1], ParsedType::Number));
}

#[test]
fn parse_tuple_type_literal_element() {
    let parsed = parse_source("type StatusPair = [\"idle\", number];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(elements[0], ParsedType::StringLiteral(_)));
}

#[test]
fn parse_tuple_type_union_element_if_supported() {
    let parsed = parse_source(
        "type StatusPair = [\"idle\" | \"done\", number];",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(&elements[0], ParsedType::Union(types) if types.len() == 2));
}

#[test]
fn parse_tuple_type_union_element() {
    let parsed = parse_source(
        "type StatusPair = [\"idle\" | \"done\", number];",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(&elements[0], ParsedType::Union(types) if types.len() == 2));
}

#[test]
fn parse_tuple_type_object_element() {
    let parsed = parse_source("type Pair = [{ name: string }, number];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(elements[0], ParsedType::Object(_)));
}

#[test]
fn parse_tuple_type_function_element() {
    let parsed = parse_source("type Pair = [() => void, string];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(elements[0], ParsedType::Function(_)));
}

#[test]
fn parse_tuple_type_array_element() {
    let parsed = parse_source("type Pair = [string[], number[]];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(elements[0], ParsedType::Array(_)));
}

#[test]
fn parse_tuple_type_nested_tuple() {
    let parsed = parse_source("type Pair = [[string, number], boolean];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(matches!(elements[0], ParsedType::Tuple(..)));
}

#[test]
fn parse_tuple_type_trailing_comma() {
    let parsed = parse_source("type Pair = [string, number,];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert_eq!(elements.len(), 2);
}

#[test]
fn parse_tuple_type_empty_tuple() {
    let parsed = parse_source("type Empty = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Tuple(elements, ..) = &alias.ty else {
        panic!("expected a tuple type");
    };

    assert!(elements.is_empty());
}

#[test]
fn parse_tuple_type_missing_close_bracket_no_panic() {
    let parsed = parse_source("type Pair = [string, number;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_missing_element_after_comma_no_panic() {
    let parsed = parse_source("type Pair = [string, ];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_readonly_unsupported_no_panic() {
    let parsed = parse_source("type Pair = readonly [string, number];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_optional_element_unsupported_no_panic() {
    let parsed = parse_source("type Pair = [string?];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_labeled_element_unsupported_no_panic() {
    let parsed = parse_source("type Pair = [name: string, age: number];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_rest_element_unsupported_no_panic() {
    let parsed = parse_source("type Pair = [string, ...number[]];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_type_variadic_unsupported_no_panic() {
    let parsed = parse_source("type Pair = [string, ...number[], boolean];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_nested_index_access_unsupported_no_panic() {
    let parsed = parse_source("let value = values[0][1];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_tuple_index_call_unsupported_no_panic() {
    let parsed = parse_source("let value = values[0]();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_array_literal_empty() {
    let parsed = parse_source("let values = [];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. }) if elements.is_empty()
    ));
}

#[test]
fn parse_array_literal_one_element() {
    let parsed = parse_source("let values = [\"Ada\"];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    let Some(ParsedExpression::ArrayLiteral { elements, .. }) = variable.initializer.as_ref()
    else {
        panic!("expected an array literal");
    };

    assert_eq!(elements.len(), 1);
}

#[test]
fn parse_array_literal_multiple_elements() {
    let parsed = parse_source("let values = [\"Ada\", \"Grace\"];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. }) if elements.len() == 2
    ));
}

#[test]
fn parse_array_literal_trailing_comma() {
    let parsed = parse_source("let values = [\"Ada\",];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. }) if elements.len() == 1
    ));
}

#[test]
fn parse_array_literal_identifier_element() {
    let parsed = parse_source("let values = [name];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. })
            if matches!(elements[0].expression, ParsedExpression::Identifier { .. })
    ));
}

#[test]
fn parse_array_literal_call_element() {
    let parsed = parse_source("let values = [getState()];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. })
            if matches!(elements[0].expression, ParsedExpression::Call { .. })
    ));
}

#[test]
fn parse_array_literal_property_call_element() {
    let parsed = parse_source("let values = [store.getState()];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. })
            if matches!(elements[0].expression, ParsedExpression::PropertyCall { .. })
    ));
}

#[test]
fn parse_array_literal_conditional_element() {
    let parsed = parse_source("let values = [true ? \"a\" : \"b\"];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. })
            if matches!(elements[0].expression, ParsedExpression::Conditional { .. })
    ));
}

#[test]
fn parse_array_literal_nested_array() {
    let parsed = parse_source("let values = [[1]];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::ArrayLiteral { elements, .. })
            if matches!(elements[0].expression, ParsedExpression::ArrayLiteral { .. })
    ));
}

#[test]
fn parse_array_literal_missing_close_bracket_no_panic() {
    let parsed = parse_source("let values = [\"Ada\";", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_array_literal_spread_unsupported_no_panic() {
    let parsed = parse_source("let values = [...items];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_index_access_number_literal() {
    let parsed = parse_source("let value = values[0];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::IndexAccess { .. })
    ));
}

#[test]
fn parse_index_access_identifier_index() {
    let parsed = parse_source("let value = values[index];", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::VariableDeclaration(variable) = &parsed.statements[0] else {
        panic!("expected a variable declaration");
    };

    assert!(matches!(
        variable.initializer.as_ref(),
        Some(ParsedExpression::IndexAccess { .. })
    ));
}

#[test]
fn parse_index_access_in_variable_initializer() {
    let parsed = parse_source("let value = values[0];", "example.ts");
    assert!(parsed.parser_errors.is_empty());
}

#[test]
fn parse_index_access_in_return_statement() {
    let parsed = parse_source("function read() { return values[0]; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());
}

#[test]
fn parse_index_access_in_assignment() {
    let parsed = parse_source("value = values[0];", "example.ts");
    assert!(parsed.parser_errors.is_empty());
}

#[test]
fn parse_index_access_missing_close_bracket_no_panic() {
    let parsed = parse_source("let value = values[0;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_index_access_string_key_unsupported_or_no_panic() {
    let parsed = parse_source("let value = values[\"0\"];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_nested_index_access_unsupported_no_panic() {
    let parsed = parse_source("let value = values[0][1];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_property_index_access_unsupported_no_panic() {
    let parsed = parse_source("let value = store.values[0];", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_index_call_unsupported_no_panic() {
    let parsed = parse_source("let value = values[0]();", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
}

#[test]
fn parse_import_named_one() {
    let parsed = parse_source("import { User } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named {
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        panic!("expected a named import");
    };

    assert!(!*is_type_only);
    assert_eq!(import.module_specifier, "./user");
    assert_eq!(specifiers.len(), 1);
    assert_eq!(specifiers[0].imported_name, "User");
    assert_eq!(specifiers[0].local_name, "User");
}

#[test]
fn parse_import_named_multiple() {
    let parsed = parse_source("import { User, Role } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert_eq!(specifiers.len(), 2);
    assert_eq!(specifiers[0].imported_name, "User");
    assert_eq!(specifiers[1].imported_name, "Role");
}

#[test]
fn parse_import_named_trailing_comma() {
    let parsed = parse_source("import { User, Role, } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert_eq!(specifiers.len(), 2);
}

#[test]
fn parse_import_named_empty_list() {
    let parsed = parse_source("import {} from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert!(specifiers.is_empty());
}

#[test]
fn parse_import_named_alias() {
    let parsed = parse_source(
        "import { User as UserModel } from \"./user\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert_eq!(specifiers.len(), 1);
    assert_eq!(specifiers[0].imported_name, "User");
    assert_eq!(specifiers[0].local_name, "UserModel");
}

#[test]
fn parse_import_type_named_one() {
    let parsed = parse_source("import type { User } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named {
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        panic!("expected a named import");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers.len(), 1);
    assert_eq!(specifiers[0].imported_name, "User");
}

#[test]
fn parse_import_type_named_multiple() {
    let parsed = parse_source("import type { User, Role } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { is_type_only, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert!(*is_type_only);
}

#[test]
fn parse_import_type_named_alias() {
    let parsed = parse_source(
        "import type { User as UserModel } from \"./user\";",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named {
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        panic!("expected a named import");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers[0].imported_name, "User");
    assert_eq!(specifiers[0].local_name, "UserModel");
}

#[test]
fn parse_import_type_named_trailing_comma() {
    let parsed = parse_source("import type { User, Role, } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named {
        is_type_only,
        specifiers,
    } = &import.kind
    else {
        panic!("expected a named import");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers.len(), 2);
}

#[test]
fn parse_import_side_effect() {
    let parsed = parse_source("import \"./setup\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(parsed.is_module);

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    assert!(matches!(import.kind, ParsedImportKind::SideEffect));
    assert_eq!(import.module_specifier, "./setup");
}

#[test]
fn parse_import_side_effect_marks_module() {
    let parsed = parse_source("import \"./setup\";", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn parse_import_missing_from_no_panic() {
    let parsed = parse_source("import { User } \"./user\";", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_import_missing_module_specifier_no_panic() {
    let parsed = parse_source("import { User } from ;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_import_missing_close_brace_no_panic() {
    let parsed = parse_source("import { User from \"./user\";", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_import_trailing_comma() {
    let parsed = parse_source("import { User, Role, } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert_eq!(specifiers.len(), 2);
}

#[test]
fn parse_import_empty_named_list() {
    let parsed = parse_source("import {} from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    let ParsedImportKind::Named { specifiers, .. } = &import.kind else {
        panic!("expected a named import");
    };

    assert!(specifiers.is_empty());
}

#[test]
fn parse_import_default() {
    let parsed = parse_source("import User from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    assert!(matches!(
        import.kind,
        ParsedImportKind::Default { ref local_name, .. } if local_name == "User"
    ));
}

#[test]
fn parse_import_namespace() {
    let parsed = parse_source("import * as user from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    assert!(matches!(
        import.kind,
        ParsedImportKind::Namespace { ref local_name, .. } if local_name == "user"
    ));
}

#[test]
fn parse_import_default_plus_named_parsed_no_panic() {
    let parsed = parse_source("import User, { Role } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ImportDeclaration(import) = &parsed.statements[0] else {
        panic!("expected an import declaration");
    };

    assert!(matches!(
        import.kind,
        ParsedImportKind::DefaultAndNamed {
            ref local_name,
            ref specifiers,
            ..
        } if local_name == "User" && specifiers.len() == 1 && specifiers[0].imported_name == "Role"
    ));
}

#[test]
fn parse_import_equals_require_unsupported_no_panic() {
    let parsed = parse_source("import user = require(\"./user\");", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(matches!(
        parsed.statements.first(),
        Some(ParsedStatement::ImportDeclaration(_))
    ));
}

#[test]
fn parse_dynamic_import_expression() {
    let parsed = parse_source("import(\"./user\");", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    let Some(ParsedStatement::Expression(expression)) = parsed.statements.first() else {
        panic!("expected an expression statement");
    };
    let ParsedExpression::ImportCall { specifier, options: None, .. } = expression.as_ref() else {
        panic!("expected an import call");
    };
    assert!(matches!(specifier.as_ref(), ParsedExpression::StringLiteral(value) if value == "./user"));
}

#[test]
fn parse_export_interface_declaration() {
    let parsed = parse_source("export interface User { name: string; }", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::InterfaceDeclaration(_)
    ));
}

#[test]
fn parse_export_type_alias_declaration() {
    let parsed = parse_source("export type UserId = string;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::TypeAliasDeclaration(_)
    ));
}

#[test]
fn parse_export_function_declaration() {
    let parsed = parse_source(
        "export function getName(): string { return \"Ada\"; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::FunctionDeclaration(_)
    ));
}

#[test]
fn parse_export_variable_const_declaration() {
    let parsed = parse_source("export const name: string = \"Ada\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_const_declaration() {
    let parsed = parse_source("export const name: string = \"Ada\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_variable_let_declaration() {
    let parsed = parse_source("export let count: number = 1;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_let_declaration() {
    let parsed = parse_source("export let count: number = 1;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_variable_var_declaration() {
    let parsed = parse_source("export var value: boolean = true;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_var_declaration() {
    let parsed = parse_source("export var value: boolean = true;", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected an exported declaration");
    };
    let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() else {
        panic!("expected an exported declaration");
    };

    assert!(matches!(
        declaration.as_ref(),
        ParsedStatement::VariableDeclaration(_)
    ));
}

#[test]
fn parse_export_named_one() {
    let parsed = parse_source("export { User };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named {
        specifiers,
        module_specifier,
        ..
    } = export.as_ref()
    else {
        panic!("expected a named export");
    };

    assert!(module_specifier.is_none());
    assert_eq!(specifiers.len(), 1);
    assert_eq!(specifiers[0].local_name, "User");
    assert_eq!(specifiers[0].exported_name, "User");
}

#[test]
fn parse_export_named_multiple() {
    let parsed = parse_source("export { User, Role };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named { specifiers, .. } = export.as_ref() else {
        panic!("expected a named export");
    };

    assert_eq!(specifiers.len(), 2);
}

#[test]
fn parse_export_named_alias() {
    let parsed = parse_source("export { User as UserModel };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named { specifiers, .. } = export.as_ref() else {
        panic!("expected a named export");
    };

    assert_eq!(specifiers.len(), 1);
    assert_eq!(specifiers[0].local_name, "User");
    assert_eq!(specifiers[0].exported_name, "UserModel");
}

#[test]
fn parse_export_type_named_one() {
    let parsed = parse_source("export type { User };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named {
        is_type_only,
        specifiers,
        ..
    } = export.as_ref()
    else {
        panic!("expected a named export");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers.len(), 1);
}

#[test]
fn parse_export_type_named_alias() {
    let parsed = parse_source("export type { User as UserModel };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named {
        is_type_only,
        specifiers,
        ..
    } = export.as_ref()
    else {
        panic!("expected a named export");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers[0].local_name, "User");
    assert_eq!(specifiers[0].exported_name, "UserModel");
}

#[test]
fn parse_export_type_named_multiple() {
    let parsed = parse_source("export type { User, Role };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named {
        is_type_only,
        specifiers,
        ..
    } = export.as_ref()
    else {
        panic!("expected a named export");
    };

    assert!(*is_type_only);
    assert_eq!(specifiers.len(), 2);
}

#[test]
fn parse_export_mixed_type_and_value_specifiers() {
    let parsed = parse_source("export { type User, value as renamedValue };", "example.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::ExportDeclaration(export) = &parsed.statements[0] else {
        panic!("expected a named export");
    };
    let ParsedExportDeclaration::Named { specifiers, .. } = export.as_ref() else {
        panic!("expected a named export");
    };

    assert_eq!(specifiers.len(), 2);
    assert!(specifiers[0].is_type_only);
    assert!(!specifiers[1].is_type_only);
    assert_eq!(specifiers[0].local_name, "User");
    assert_eq!(specifiers[0].exported_name, "User");
    assert_eq!(specifiers[1].local_name, "value");
    assert_eq!(specifiers[1].exported_name, "renamedValue");
}

#[test]
fn parse_export_type_named_marks_module() {
    let parsed = parse_source("export type { User };", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn parse_export_empty() {
    let parsed = parse_source("export {};", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(parsed.is_module);

    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(export.as_ref(), ParsedExportDeclaration::Empty { .. })
    ));
}

#[test]
fn parse_export_empty_marks_module() {
    let parsed = parse_source("export {};", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn parse_export_missing_brace_no_panic() {
    let parsed = parse_source("export { User;", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_export_missing_name_no_panic() {
    let parsed = parse_source("export { as UserModel };", "example.ts");
    assert_eq!(parsed.file_name, "example.ts");
    assert!(!parsed.parser_errors.is_empty());
}

#[test]
fn parse_export_default_expression() {
    let parsed = parse_source("export default 1;", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Expression(ParsedExpression::NumberLiteral(value)),
                ..
            } if value == "1"
        )
    ));
}

#[test]
fn parse_export_default_function() {
    let parsed = parse_source(
        "export default function getName(): string { return \"Ada\"; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Function(_),
                ..
            }
        )
    ));
}

#[test]
fn parse_export_default_class_no_panic() {
    let parsed = parse_source("export default class Foo {}", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Class(_),
                ..
            }
        )
    ));
}

#[test]
fn parse_export_named_from() {
    let parsed = parse_source("export { User } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Named {
                module_specifier: Some(module_specifier),
                ..
            } if module_specifier == "./user"
        )
    ));
}

#[test]
fn parse_export_type_named_from() {
    let parsed = parse_source("export type { User } from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Named {
                is_type_only: true,
                module_specifier: Some(module_specifier),
                ..
            } if module_specifier == "./user"
        )
    ));
}

#[test]
fn parse_export_star_from() {
    let parsed = parse_source("export * from \"./user\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::All {
                module_specifier,
                ..
            } if module_specifier == "./user"
        )
    ));
}

#[test]
fn parse_export_star_as_from_unsupported_no_panic() {
    let parsed = parse_source("export * as Foo from \"./foo\";", "example.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::ExportDeclaration(export) if matches!(
            export.as_ref(),
            ParsedExportDeclaration::Namespace {
                exported_name,
                module_specifier,
                ..
            } if exported_name == "Foo" && module_specifier == "./foo"
        )
    ));
}

#[test]
fn parse_declare_namespace_yields_namespace_declaration() {
    let parsed = parse_source(
        "declare namespace JSX { interface Element {} }",
        "example.d.ts",
    );
    assert!(parsed.parser_errors.is_empty());
    let ParsedStatement::NamespaceDeclaration(namespace) = &parsed.statements[0] else {
        panic!(
            "expected namespace declaration, got {:?}",
            parsed.statements
        );
    };
    assert_eq!(namespace.name, "JSX");
    assert!(matches!(
        &namespace.statements[0],
        ParsedStatement::InterfaceDeclaration(interface) if interface.name == "Element"
    ));
}

#[test]
fn parse_declare_global_is_supported() {
    let parsed = parse_source("declare global {}", "example.d.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::DeclareModuleDeclaration(declaration) if matches!(
            declaration.as_ref(),
            ParsedDeclareModuleDeclaration {
                module_specifier,
                ..
            } if module_specifier == "global"
        )
    ));
}

#[test]
// A wildcard module declaration keeps its pattern: the checker matches it
// against an importer's specifier (`declare module "*.css"` covers
// `import "./globals.css"`).
fn parse_declare_module_wildcard_keeps_pattern() {
    let parsed = parse_source("declare module \"*.css\" {}", "example.d.ts");
    assert!(parsed.parser_errors.is_empty());
    assert!(matches!(
        &parsed.statements[0],
        ParsedStatement::DeclareModuleDeclaration(declaration)
            if declaration.module_specifier == "*.css"
    ));
}

#[test]
fn module_detection_plain_script_false() {
    let parsed = parse_source("let value: string = \"ok\";", "example.ts");
    assert!(!parsed.is_module);
}

#[test]
fn module_detection_import_named_true() {
    let parsed = parse_source("import { User } from \"./user\";", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_import_type_true() {
    let parsed = parse_source("import type { User } from \"./user\";", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_import_side_effect_true() {
    let parsed = parse_source("import \"./setup\";", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_import_unsupported_true() {
    let parsed = parse_source("import User from \"./user\";", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_interface_true() {
    let parsed = parse_source("export interface User { name: string; }", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_type_alias_true() {
    let parsed = parse_source("export type Name = string;", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_function_true() {
    let parsed = parse_source(
        "export function getName(): string { return \"Ada\"; }",
        "example.ts",
    );
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_variable_true() {
    let parsed = parse_source("export const value: number = 1;", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_named_true() {
    let parsed = parse_source("export { User };", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_type_named_true() {
    let parsed = parse_source("export type { User };", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_empty_true() {
    let parsed = parse_source("export {};", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_export_unsupported_true() {
    let parsed = parse_source("export default 1;", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn module_detection_oxc_source_type_module_true() {
    let parsed = parse_source("export {};", "example.ts");
    assert!(parsed.is_module);
}

#[test]
fn parse_indexed_access_type() {
    let source = "type T = Obj[\"key\"]; type U = Arr[0]; type V = Obj[keyof Obj];";
    let parsed = parse_source(source, "test.ts");

    let stmt1 = &parsed.statements[0];
    let stmt2 = &parsed.statements[1];
    let stmt3 = &parsed.statements[2];

    assert!(matches!(
        stmt1,
        ParsedStatement::TypeAliasDeclaration(alias) if matches!(
            &alias.ty,
            ParsedType::IndexedAccess(indexed_access) if matches!(
                (indexed_access.object_type.as_ref(), indexed_access.index_type.as_ref()),
                (ParsedType::Named(obj), ParsedType::StringLiteral(key)) if obj.name == "Obj" && key == "key"
            )
        )
    ));

    assert!(matches!(
        stmt2,
        ParsedStatement::TypeAliasDeclaration(alias) if matches!(
            &alias.ty,
            ParsedType::IndexedAccess(indexed_access) if matches!(
                (indexed_access.object_type.as_ref(), indexed_access.index_type.as_ref()),
                (ParsedType::Named(arr), ParsedType::NumberLiteral(num)) if arr.name == "Arr" && num == "0"
            )
        )
    ));

    assert!(matches!(
        stmt3,
        ParsedStatement::TypeAliasDeclaration(alias) if matches!(
            &alias.ty,
            ParsedType::IndexedAccess(indexed_access) if matches!(
                (indexed_access.object_type.as_ref(), indexed_access.index_type.as_ref()),
                (ParsedType::Named(obj1), ParsedType::KeyOf(keyof_inner)) if obj1.name == "Obj" && matches!(
                    keyof_inner.as_ref(),
                    ParsedType::Named(obj2) if obj2.name == "Obj"
                )
            )
        )
    ));
}

#[test]
fn parse_mapped_type_required() {
    let source = "type Clone<T> = { [K in keyof T]: T[K] };";
    let parsed = parse_source(source, "test.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Mapped(mapped) = &alias.ty else {
        panic!("expected a mapped type");
    };

    assert_eq!(mapped.key_name, "K");
    assert_eq!(mapped.optional, MappedOptionality::Keep);
    assert!(matches!(*mapped.constraint, ParsedType::KeyOf(_)));
    assert!(matches!(*mapped.value_type, ParsedType::IndexedAccess(_)));
}

#[test]
fn parse_mapped_type_optional() {
    let source = "type OptionalClone<T> = { [K in keyof T]?: T[K] };";
    let parsed = parse_source(source, "test.ts");
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Mapped(mapped) = &alias.ty else {
        panic!("expected a mapped type");
    };

    assert_eq!(mapped.key_name, "K");
    assert_eq!(mapped.optional, MappedOptionality::Add);
    assert!(matches!(*mapped.constraint, ParsedType::KeyOf(_)));
    assert!(matches!(*mapped.value_type, ParsedType::IndexedAccess(_)));
}

#[test]
fn parse_mapped_type_carries_its_remapping() {
    let source = "type Remapped<T> = { [K in keyof T as string]: T[K] };";
    let parsed = parse_source(source, "test.ts");

    let ParsedStatement::TypeAliasDeclaration(alias) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };

    let ParsedType::Mapped(mapped) = &alias.ty else {
        panic!("expected a mapped type");
    };
    assert_eq!(mapped.key_name, "K");
    assert!(matches!(
        mapped.name_type.as_deref(),
        Some(ParsedType::String)
    ));
}

#[test]
fn parse_mapped_type_readonly_and_optionality_modifiers_are_modelled() {
    let source = "type ReadonlyClone<T> = { readonly [K in keyof T]: T[K] }; type MinusOpt<T> = { [K in keyof T]-?: T[K] }; type PlusOpt<T> = { [K in keyof T]+?: T[K] };";
    let parsed = parse_source(source, "test.ts");

    let ParsedStatement::TypeAliasDeclaration(alias1) = &parsed.statements[0] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Mapped(readonly_clone) = &alias1.ty else {
        panic!("expected a mapped type");
    };
    assert_eq!(readonly_clone.optional, MappedOptionality::Keep);

    let ParsedStatement::TypeAliasDeclaration(alias2) = &parsed.statements[1] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Mapped(minus_opt) = &alias2.ty else {
        panic!("expected `-?` to keep the mapping rather than degrade the alias");
    };
    assert_eq!(minus_opt.optional, MappedOptionality::Remove);

    let ParsedStatement::TypeAliasDeclaration(alias3) = &parsed.statements[2] else {
        panic!("expected a type alias declaration");
    };
    let ParsedType::Mapped(plus_opt) = &alias3.ty else {
        panic!("expected `+?` to keep the mapping rather than degrade the alias");
    };
    assert_eq!(plus_opt.optional, MappedOptionality::Add);
}

#[test]
fn parse_function_body_type_class_and_interface_declarations() {
    let parsed = parse_source(
        "function outer() { type T = string; interface I { a: number } class C {} return 1; }",
        "example.ts",
    );
    assert!(parsed.parser_errors.is_empty());

    let ParsedStatement::FunctionDeclaration(function) = &parsed.statements[0] else {
        panic!("expected a function declaration");
    };

    let ParsedFunctionBodyStatement::TypeAlias(alias) = &function.body[0] else {
        panic!("expected a body-local type alias");
    };
    assert_eq!(alias.name, "T");
    assert_eq!(alias.ty, ParsedType::String);

    let ParsedFunctionBodyStatement::Interface(interface) = &function.body[1] else {
        panic!("expected a body-local interface");
    };
    assert_eq!(interface.name, "I");
    assert_eq!(interface.members.len(), 1);

    let ParsedFunctionBodyStatement::Class(class) = &function.body[2] else {
        panic!("expected a body-local class");
    };
    assert_eq!(class.name, "C");

    assert!(matches!(
        function.body[3],
        ParsedFunctionBodyStatement::Return(_)
    ));
}

fn grammar_kinds(source: &str, file_name: &str) -> Vec<ParsedGrammarDiagnosticKind> {
    parse_source(source, file_name)
        .grammar_diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}

#[test]
fn grammar_walk_reports_a_const_without_an_initializer() {
    let parsed = parse_source("const pending: number;", "example.ts");

    assert_eq!(
        grammar_kinds("const pending: number;", "example.ts"),
        vec![ParsedGrammarDiagnosticKind::ConstNotInitialized]
    );
    let span = parsed.grammar_diagnostics[0].span;
    assert_eq!(&"const pending: number;"[span.start..span.end], "pending");
}

#[test]
fn grammar_walk_exempts_ambient_and_loop_head_bindings() {
    for source in [
        "declare const value: number;",
        "declare namespace Ambient { const value: number; }",
        "for (const value of [1]) { void value; }",
        "for (const key in { a: 1 }) { void key; }",
    ] {
        assert!(
            grammar_kinds(source, "example.ts").is_empty(),
            "unexpected grammar diagnostics for {source}"
        );
    }
}

#[test]
fn grammar_walk_reports_each_repeated_object_literal_property() {
    assert_eq!(
        grammar_kinds("const o = { a: 1, a: 2, a: 3 };", "example.ts"),
        vec![
            ParsedGrammarDiagnosticKind::DuplicateObjectLiteralProperty,
            ParsedGrammarDiagnosticKind::DuplicateObjectLiteralProperty,
        ]
    );
    assert!(
        grammar_kinds(
            "const o = { get a() { return 1; }, set a(v: number) { void v; } };",
            "example.ts"
        )
        .is_empty()
    );
}

#[test]
fn grammar_walk_reports_an_overload_group_without_an_implementation() {
    assert_eq!(
        grammar_kinds(
            "function f(a: string): void;\nfunction f(a: number): void;",
            "example.ts"
        ),
        vec![ParsedGrammarDiagnosticKind::FunctionImplementationMissing]
    );
    assert!(
        grammar_kinds(
            "function f(a: string): void;\nfunction f(a: string | number): void {}",
            "example.ts"
        )
        .is_empty()
    );
    assert!(grammar_kinds("declare function f(a: string): void;", "example.ts").is_empty());
}

#[test]
fn grammar_walk_reports_a_derived_constructor_without_a_super_call() {
    assert_eq!(
        grammar_kinds("class D extends B { constructor() {} }", "example.ts"),
        vec![ParsedGrammarDiagnosticKind::MissingSuperCall]
    );
    assert!(
        grammar_kinds(
            "class D extends B { constructor(f: boolean) { if (f) { super(1); } else { super(2); } } }",
            "example.ts"
        )
        .is_empty()
    );
}

#[test]
fn grammar_walk_is_skipped_for_declaration_and_non_typescript_files() {
    // A declaration file's top-level declaration still needs `declare` or
    // `export` (TS1046), which the reduced declaration-file pass reports.
    assert_eq!(
        grammar_kinds("const pending: number;", "example.d.ts"),
        vec![ParsedGrammarDiagnosticKind::Ts(1046)]
    );
    for file_name in ["example.js", "example.jsx"] {
        assert!(
            grammar_kinds("const pending: number;", file_name).is_empty(),
            "unexpected grammar diagnostics for {file_name}"
        );
    }
}
