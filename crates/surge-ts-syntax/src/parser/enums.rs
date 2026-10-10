//! `enum` declarations, lowered to the constructs the checker already models:
//! a type alias holding the union of the member types, plus a same-named
//! ambient `const` whose object type carries one property per member. Enum
//! types are nominal in tsc; the literal-union approximation keeps every member
//! read (`Color.Red`), value use (`z.enum(Color)`), and type position resolving
//! instead of cascading `TS2304`, at the cost of accepting a bare literal where
//! tsc would require the enum member.

use oxc_ast::ast::{Expression, TSEnumDeclaration, TSEnumMemberName};
use oxc_span::GetSpan;

use crate::{
    ParsedEnumBody, ParsedEnumMemberInitializer, ParsedFunctionBodyStatement, ParsedObjectType,
    ParsedObjectTypeProperty, ParsedStatement, ParsedType, ParsedTypeAliasDeclaration,
    ParsedVariableDeclaration, ParsedVariableKind,
};

use super::enum_values::{EnumValue, Evaluated};
use super::text_span_from_oxc_span;

pub(crate) fn parse_enum_declaration(
    declaration: &TSEnumDeclaration<'_>,
    exported: bool,
) -> Vec<ParsedStatement> {
    let (type_alias, value) = lower_enum_declaration(declaration, exported);
    let mut statements: Vec<ParsedStatement> = member_type_aliases(&type_alias, &value)
        .map(|alias| ParsedStatement::TypeAliasDeclaration(Box::new(alias)))
        .collect();
    statements.push(ParsedStatement::TypeAliasDeclaration(Box::new(type_alias)));
    statements.push(ParsedStatement::VariableDeclaration(Box::new(value)));
    statements
}

/// One alias per member, named `Enum.Member`, so an *enum member type* resolves
/// (`type R = Color.Red`, or a discriminant `interface N { kind: Color.Red }`).
/// Without them the annotation misses and the enclosing declaration degrades,
/// which both loses the diagnostic and makes the expansion uncacheable.
///
/// An enum declared inside a `namespace` is registered qualified
/// (`ts.SyntaxKind.SourceFile`), so a bare `SyntaxKind.SourceFile` written inside
/// that namespace still misses — see the note in
/// `docs/perf/NAMESPACE-INTERFACE-MERGE.md`.
fn member_type_aliases<'a>(
    type_alias: &'a ParsedTypeAliasDeclaration,
    value: &'a ParsedVariableDeclaration,
) -> impl Iterator<Item = ParsedTypeAliasDeclaration> + 'a {
    let members: &'a [ParsedObjectTypeProperty] = match value.declared_type.as_ref() {
        Some(ParsedType::Object(object)) => object.properties.as_slice(),
        _ => &[],
    };
    members.iter().map(move |member| ParsedTypeAliasDeclaration {
        is_declare: type_alias.is_declare,
        name: format!("{}.{}", type_alias.name, member.name),
        name_span: member.name_span,
        type_parameters: Vec::new(),
        ty: member.ty.clone(),
        type_span: member.name_span,
        enum_name: Some(type_alias.name.clone()),
        enum_exported: type_alias.enum_exported,
        enum_is_const: type_alias.enum_is_const,
        enum_members: Vec::new(),
    })
}

pub(crate) fn parse_enum_declaration_as_function_body(
    declaration: &TSEnumDeclaration<'_>,
) -> Vec<ParsedFunctionBodyStatement> {
    // A body-local enum is never visible outside its file.
    let (type_alias, value) = lower_enum_declaration(declaration, false);
    vec![
        ParsedFunctionBodyStatement::TypeAlias(Box::new(type_alias)),
        ParsedFunctionBodyStatement::VariableDeclaration(Box::new(value)),
    ]
}

fn lower_enum_declaration(
    declaration: &TSEnumDeclaration<'_>,
    exported: bool,
) -> (ParsedTypeAliasDeclaration, ParsedVariableDeclaration) {
    let name_span = Some(text_span_from_oxc_span(declaration.id.span));
    let evaluation = super::enum_values::enum_evaluation(declaration);
    // tsc leaves an ambient enum's members without an initializer computed;
    // numbering them keeps each a literal of its own, which is how surge tells
    // the members of such an enum apart.
    let numbers_ambient_members = evaluation.ambient && !declaration.r#const;
    let mut next_auto_value = Some(0.0);
    let mut properties = Vec::with_capacity(declaration.body.members.len());
    let mut member_types = Vec::with_capacity(declaration.body.members.len());
    let mut checked_members = Vec::new();

    for (index, member) in declaration.body.members.iter().enumerate() {
        let value = evaluation.members.get(index).map(|evaluated| &evaluated.value);
        if let Some(initializer) = member.initializer.as_ref()
            && !evaluation.ambient
            && !is_literal_initializer(initializer)
        {
            checked_members.push(ParsedEnumMemberInitializer {
                name: super::enum_values::member_name(&member.id).unwrap_or_default().to_string(),
                initializer: super::expressions::parse_expression(initializer).0,
                initializer_span: Some(text_span_from_oxc_span(initializer.span())),
                computed: matches!(value, Some(Evaluated::Computed)),
                uninitialized_successor: declaration
                    .body
                    .members
                    .get(index + 1)
                    .filter(|next| next.initializer.is_none() && matches!(value, Some(Evaluated::Unresolved)))
                    .map(|next| text_span_from_oxc_span(next.id.span())),
            });
        }
        // A computed member (`A = f()`) is numeric, and so is one whose value
        // depends on another file. A non-finite value stays `number` too:
        // surge's number literal types are not relied on to hold `NaN` or
        // `Infinity`.
        let (member_type, number) = match value {
            Some(Evaluated::Value(EnumValue::Number(number))) if number.is_finite() => {
                (ParsedType::NumberLiteral(format_auto_value(*number)), Some(*number))
            }
            Some(Evaluated::Value(EnumValue::String(text))) => {
                (ParsedType::StringLiteral(text.clone()), None)
            }
            _ if numbers_ambient_members && member.initializer.is_none() => match next_auto_value {
                Some(number) => (ParsedType::NumberLiteral(format_auto_value(number)), Some(number)),
                None => (ParsedType::Number, None),
            },
            _ => (ParsedType::Number, None),
        };
        next_auto_value = number.map(|number| number + 1.0);
        let Some(member_name) = enum_member_name(&member.id) else {
            continue;
        };

        properties.push(ParsedObjectTypeProperty {
            name: member_name,
            name_span: Some(text_span_from_oxc_span(member.span)),
            ty: member_type.clone(),
            optional: false,
            is_method: false,
            readonly: true,
            write_ty: None,
        });
        member_types.push(member_type);
    }

    let number_index_type = reverse_mapping_index_type(&properties);
    let enum_type = match member_types.len() {
        0 => ParsedType::Never,
        1 => member_types.pop().expect("one member type"),
        _ => ParsedType::Union(std::sync::Arc::new(member_types)),
    };

    (
        ParsedTypeAliasDeclaration {
            is_declare: declaration.declare,
            name: declaration.id.name.to_string(),
            name_span,
            type_parameters: Vec::new(),
            ty: enum_type,
            type_span: name_span,
            enum_name: Some(declaration.id.name.to_string()),
            enum_exported: exported,
            enum_is_const: declaration.r#const,
            enum_members: properties.iter().map(|property| (property.name.clone(), property.ty.clone())).collect(),
        },
        ParsedVariableDeclaration {
            // The object side has no written initializer to check, and an `enum`
            // is never subject to the initializer-inference paths.
            is_declare: true,
            from_binding_pattern: false,
            has_definite_assertion: false,
            array_pattern_span: None,
            is_enum_object: true,
            array_rest_start: None,
            kind: ParsedVariableKind::Const,
            name: declaration.id.name.to_string(),
            name_span,
            declared_type: Some(ParsedType::Object(std::sync::Arc::new(ParsedObjectType {
                properties,
                string_index_type: None,
                string_index_readonly: false,
                number_index_readonly: number_index_type.is_some(),
                number_index_type,
                call_signature: None,
            call_signature_overloads: Vec::new(),
                construct_signature: None,
                construct_signature_overloads: Vec::new(),
                non_primitive: false,
                display_name: Some(format!("typeof {}", declaration.id.name)),
                abstract_construct_signature: false,
                keyed_indexes: None,
                number_index_span: None,
            }))),
            initializer: None,
            initializer_span: None,
            declaration_list: None,
            annotated_pattern: None,
            pattern_excess_properties: Vec::new(),
            enum_members: (!checked_members.is_empty()).then(|| {
                std::sync::Arc::new(vec![ParsedEnumBody {
                    is_const: declaration.r#const,
                    members: checked_members,
                }])
            }),
        },
    )
}

/// tsc's `enumNumberIndexInfo` (`resolveAnonymousTypeMembers`): the object of
/// an enum with no members, or with any numeric member, carries the reverse
/// mapping `readonly [n: number]: string`. A string-only enum has none, so
/// `S[0]` stays an implicit `any`.
fn reverse_mapping_index_type(properties: &[ParsedObjectTypeProperty]) -> Option<Box<ParsedType>> {
    let numeric = properties.is_empty()
        || properties
            .iter()
            .any(|property| matches!(property.ty, ParsedType::NumberLiteral(_) | ParsedType::Number));
    numeric.then(|| Box::new(ParsedType::String))
}

fn enum_member_name(name: &TSEnumMemberName<'_>) -> Option<String> {
    match name {
        // The parser's nameless placeholder for a computed name (TS1164).
        TSEnumMemberName::Identifier(identifier) if identifier.name.is_empty() => None,
        TSEnumMemberName::Identifier(identifier) => Some(identifier.name.to_string()),
        TSEnumMemberName::String(literal) | TSEnumMemberName::ComputedString(literal) => {
            Some(literal.value.to_string())
        }
        TSEnumMemberName::ComputedTemplateString(_) => None,
    }
}

/// A literal initializer (`1`, `-1`, `"a"`, `` `a` ``) is constant and reports
/// nothing when checked, so the checker is not handed one.
fn is_literal_initializer(initializer: &Expression<'_>) -> bool {
    match initializer {
        Expression::StringLiteral(_) | Expression::NumericLiteral(_) => true,
        Expression::TemplateLiteral(template) => template.expressions.is_empty(),
        Expression::UnaryExpression(unary) => {
            matches!(
                unary.operator,
                oxc_syntax::operator::UnaryOperator::UnaryNegation
                    | oxc_syntax::operator::UnaryOperator::UnaryPlus
            ) && matches!(unary.argument, Expression::NumericLiteral(_))
        }
        _ => false,
    }
}

fn format_auto_value(value: f64) -> String {
    super::number_text::js_number_to_string(value)
}

/// An `enum` declared more than once in one scope is one enum: tsc merges the
/// bodies. The lowering runs per declaration, so the object sides (and the
/// union alias holding the member types) are folded together here — otherwise
/// one declaration's members silently replace the other's.
pub(crate) fn merge_lowered_enum_declarations(statements: &mut Vec<ParsedStatement>) {
    use std::collections::HashMap;

    let mut objects: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut aliases: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, statement) in statements.iter().enumerate() {
        match peel_exported(statement) {
            ParsedStatement::VariableDeclaration(variable) if variable.is_enum_object => {
                objects.entry(variable.name.as_str()).or_default().push(index);
            }
            ParsedStatement::TypeAliasDeclaration(alias)
                if alias.enum_name.as_deref() == Some(alias.name.as_str()) =>
            {
                aliases.entry(alias.name.as_str()).or_default().push(index);
            }
            _ => {}
        }
    }

    let duplicated: Vec<Vec<usize>> = objects
        .into_values()
        .chain(aliases.into_values())
        .filter(|indices| indices.len() > 1)
        .collect();
    if duplicated.is_empty() {
        return;
    }

    let mut dropped: Vec<usize> = Vec::new();
    for indices in duplicated {
        let (first, rest) = indices.split_first().expect("non-empty group");
        let mut properties: Vec<ParsedObjectTypeProperty> = Vec::new();
        let mut reverse_mapping = None;
        let mut member_types: Vec<ParsedType> = Vec::new();
        let mut named_members: Vec<(String, ParsedType)> = Vec::new();
        let mut enum_members: Vec<ParsedEnumBody> = Vec::new();
        for index in rest {
            match peel_exported(&statements[*index]) {
                ParsedStatement::VariableDeclaration(variable) => {
                    if let Some(ParsedType::Object(object)) = variable.declared_type.as_ref() {
                        properties.extend(object.properties.iter().cloned());
                        reverse_mapping = reverse_mapping.or_else(|| object.number_index_type.clone());
                    }
                    if let Some(bodies) = variable.enum_members.as_ref() {
                        enum_members.extend(bodies.iter().cloned());
                    }
                }
                ParsedStatement::TypeAliasDeclaration(alias) => {
                    push_union_members(&alias.ty, &mut member_types);
                    named_members.extend(alias.enum_members.iter().cloned());
                }
                _ => {}
            }
            dropped.push(*index);
        }

        let Some(target) = exported_enum_statement_mut(&mut statements[*first]) else {
            continue;
        };
        match target {
            ParsedStatement::VariableDeclaration(variable) => {
                if let Some(ParsedType::Object(object)) = variable.declared_type.as_mut() {
                    let object = std::sync::Arc::make_mut(object);
                    if object.number_index_type.is_none() {
                        object.number_index_type = reverse_mapping;
                    }
                    for property in properties {
                        if !object.properties.iter().any(|kept| kept.name == property.name) {
                            object.properties.push(property);
                        }
                    }
                    object.number_index_type = reverse_mapping_index_type(&object.properties);
                }
                if !enum_members.is_empty() {
                    let bodies = variable.enum_members.get_or_insert_with(Default::default);
                    std::sync::Arc::make_mut(bodies).extend(enum_members);
                }
            }
            ParsedStatement::TypeAliasDeclaration(alias) => {
                for (name, ty) in named_members {
                    if !alias.enum_members.iter().any(|(kept, _)| *kept == name) {
                        alias.enum_members.push((name, ty));
                    }
                }
                let mut merged = Vec::new();
                push_union_members(&alias.ty, &mut merged);
                for member in member_types {
                    if !merged.contains(&member) {
                        merged.push(member);
                    }
                }
                alias.ty = match merged.len() {
                    1 => merged.pop().expect("one member"),
                    _ => ParsedType::Union(std::sync::Arc::new(merged)),
                };
            }
            _ => {}
        }
    }

    dropped.sort_unstable();
    let mut index = 0;
    statements.retain(|_| {
        let keep = dropped.binary_search(&index).is_err();
        index += 1;
        keep
    });
}

fn push_union_members(ty: &ParsedType, members: &mut Vec<ParsedType>) {
    match ty {
        ParsedType::Union(union) => members.extend(union.iter().cloned()),
        other => members.push(other.clone()),
    }
}

fn peel_exported(statement: &ParsedStatement) -> &ParsedStatement {
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            crate::ParsedExportDeclaration::Statement { declaration, .. } => {
                peel_exported(declaration.as_ref())
            }
            _ => statement,
        },
        other => other,
    }
}

fn exported_enum_statement_mut(statement: &mut ParsedStatement) -> Option<&mut ParsedStatement> {
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_mut() {
            crate::ParsedExportDeclaration::Statement { declaration, .. } => {
                exported_enum_statement_mut(declaration.as_mut())
            }
            _ => None,
        },
        other => Some(other),
    }
}
