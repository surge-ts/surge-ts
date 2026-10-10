//! tsc's `lateBindMember` for the members of a type literal, interface or
//! class: a computed name written as an entity name (`[c0]`, `[N.s1]`) names
//! the member by its key's type when that type is usable as a property name
//! (`isTypeUsableAsPropertyName`). The parser keeps such a member under its
//! written key in brackets; a string or number literal key becomes the literal
//! property, and a unique symbol the `[declaration]` name its element
//! accesses look up.

use std::cell::Cell;
use std::sync::Arc;

use surge_ts_syntax::{ParsedType, ParsedTypeOfType};
use surge_ts_types::{PropertyMap, Type};

use crate::context::CheckerContext;

thread_local! {
    /// A key whose type reads the declaration being bound (`[rI.x]` inside
    /// `interface RI` with `declare const rI: RI<"a">`) must not late-bind
    /// that declaration's members again while it is resolved.
    static LATE_BINDING: Cell<bool> = const { Cell::new(false) };
}

/// Renames the late-bindable members of `properties`. `true` when a key's
/// value could not be resolved yet (a class built while signatures are
/// collected, before the module's values are bound): the shape keeps the
/// written name and must not be cached as final. Only the collected file's own
/// values are still to be bound: a key that does not resolve outside that
/// phase, or in another file's declaration, never will here, so its written
/// name is final. Degrading it kept every shape naming it uncached, and
/// drizzle's `[TableName]` re-resolved its whole declaration graph without end.
pub(crate) fn late_bind_member_names(properties: &mut PropertyMap, ctx: &mut CheckerContext) -> bool {
    if !properties.keys().any(|name| is_late_bindable_name(name)) || LATE_BINDING.with(Cell::get) {
        return false;
    }
    LATE_BINDING.with(|active| active.set(true));
    let mut renamed: Vec<(Arc<str>, Arc<str>)> = Vec::new();
    let mut unresolved = false;
    for name in properties.keys().filter(|name| is_late_bindable_name(name)) {
        match late_bound_name(name, ctx) {
            Ok(Some(bound)) if bound.as_str() != name.as_ref() => renamed.push((name.clone(), bound.into())),
            Ok(_) => {}
            Err(Unresolved) => {
                unresolved |= ctx.collecting_signatures && ctx.cross_file_resolution_depth == 0;
            }
        }
    }
    LATE_BINDING.with(|active| active.set(false));
    if renamed.is_empty() {
        return unresolved;
    }
    let previous = std::mem::take(properties);
    for (name, property) in previous {
        let name = renamed
            .iter()
            .find(|(written, _)| *written == name)
            .map_or(name, |(_, bound)| bound.clone());
        // A name already declared keeps its first declaration, as the
        // symbol it merges into does.
        properties.entry(name).or_insert(property);
    }
    unresolved
}

/// The `string` and `number` index signatures (value, `readonly`) a class's
/// members imply (see [`late_bind_index_signatures`]).
#[derive(Default)]
pub(crate) struct ImpliedIndexes {
    pub(crate) string: Option<(Type, bool)>,
    pub(crate) number: Option<(Type, bool)>,
}

/// tsc's `getIndexInfosOfIndexSymbol` for a class: a `[k]` member whose key
/// is a `string` or `number` rather than a literal (`lateBindIndexSignature`)
/// is no property but an index of its key's kind, over every member that
/// index applies to. Removes such members from `properties`.
pub(crate) fn late_bind_index_signatures(properties: &mut PropertyMap, ctx: &mut CheckerContext) -> ImpliedIndexes {
    let mut implied = ImpliedIndexes::default();
    if !properties.keys().any(|name| is_late_bindable_name(name)) || LATE_BINDING.with(Cell::get) {
        return implied;
    }
    LATE_BINDING.with(|active| active.set(true));
    let keyed: Vec<(Arc<str>, bool)> = properties
        .keys()
        .filter(|name| is_late_bindable_name(name))
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|name| index_signature_key_is_numeric(&name, ctx).map(|numeric| (name, numeric)))
        .collect();
    LATE_BINDING.with(|active| active.set(false));
    if keyed.is_empty() {
        return implied;
    }
    let value_type = |property: &surge_ts_types::ObjectProperty| {
        if property.optional && surge_ts_types::strict_null_checks() {
            surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        }
    };
    let (mut string_values, mut number_values) = (Vec::new(), Vec::new());
    let (mut string_readonly, mut number_readonly) = (None, None);
    for (name, numeric) in &keyed {
        let Some(property) = properties.shift_remove(name) else {
            continue;
        };
        let readonly = if *numeric { &mut number_readonly } else { &mut string_readonly };
        *readonly = Some(readonly.unwrap_or(true) && property.readonly);
        if *numeric {
            number_values.push(value_type(&property));
        }
        string_values.push(value_type(&property));
    }
    // A member named by a symbol answers neither index.
    for (name, property) in properties.iter().filter(|(name, _)| !name.starts_with('[')) {
        if crate::program::is_numeric_literal_name(name) {
            number_values.push(value_type(property));
        }
        string_values.push(value_type(property));
    }
    implied.string = string_readonly.map(|readonly| (surge_ts_types::union_type(string_values), readonly));
    implied.number = number_readonly.map(|readonly| (surge_ts_types::union_type(number_values), readonly));
    implied
}

/// For a `[path]` key whose type is a `string` or `number` rather than a
/// literal: whether it is numeric (`any` is). `None` for any other key.
fn index_signature_key_is_numeric(written: &str, ctx: &mut CheckerContext) -> Option<bool> {
    let path = &written[1..written.len() - 1];
    let mut segments = path.split('.');
    let name = segments.next()?.to_string();
    let members: Vec<String> = segments.map(str::to_string).collect();
    if name.is_empty() || members.iter().any(String::is_empty) {
        return None;
    }
    let query = ParsedType::TypeOf(Arc::new(ParsedTypeOfType {
        name,
        name_span: None,
        members,
        import_specifier: None,
        member_spans: Vec::new(),
        type_arguments: Vec::new(),
        type_arguments_span: None,
    }));
    let checkpoint = ctx.diagnostics().len();
    let key_type = super::map_parsed_type(query, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    fn numeric(ty: &Type) -> Option<bool> {
        match ty {
            Type::String => Some(false),
            Type::Number | Type::Any => Some(true),
            Type::Union(union) => {
                let kinds: Option<Vec<bool>> = union
                    .types()
                    .iter()
                    .map(|member| match member {
                        Type::StringLiteral(_) => Some(false),
                        Type::NumberLiteral(_) => Some(true),
                        other => numeric(other),
                    })
                    .collect();
                kinds.map(|kinds| kinds.iter().all(|numeric| *numeric))
            }
            _ => None,
        }
    }
    numeric(&key_type)
}

fn is_late_bindable_name(name: &str) -> bool {
    name.starts_with('[') && name.ends_with(']') && !name.starts_with("[Symbol.")
}

struct Unresolved;

/// The member name a key written as `[path]` binds to, `None` when its type is
/// not usable as a property name.
fn late_bound_name(written: &str, ctx: &mut CheckerContext) -> Result<Option<String>, Unresolved> {
    let path = &written[1..written.len() - 1];
    let mut segments = path.split('.');
    let Some(name) = segments.next().map(str::to_string) else {
        return Ok(None);
    };
    let members: Vec<String> = segments.map(str::to_string).collect();
    if name.is_empty() || members.iter().any(String::is_empty) {
        return Ok(None);
    }
    let query = ParsedType::TypeOf(Arc::new(ParsedTypeOfType {
        name,
        name_span: None,
        members,
        import_specifier: None,
        member_spans: Vec::new(),
        type_arguments: Vec::new(),
        type_arguments_span: None,
    }));
    let checkpoint = ctx.diagnostics().len();
    let key_type = super::map_parsed_type(query, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if key_type.is_unknown() || matches!(key_type, Type::ErrorType) {
        return Err(Unresolved);
    }
    Ok(match &key_type {
        Type::StringLiteral(value) => Some(value.clone()),
        Type::NumberLiteral(literal) => Some(literal.value.clone()),
        Type::Reference(reference) if reference.enum_base.is_some() => match key_type.peeled() {
            Type::StringLiteral(value) => Some(value),
            Type::NumberLiteral(literal) => Some(literal.value),
            _ => None,
        },
        Type::Reference(reference) => reference
            .unique_symbol_name()
            .map(|declaration| format!("[{declaration}]")),
        _ => None,
    })
}
