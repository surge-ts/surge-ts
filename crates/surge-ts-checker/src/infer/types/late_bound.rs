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
/// `drop_unresolved` lets a key whose name resolves nowhere bind no member; it
/// holds only where every member was written in the scope resolving it, which
/// an inherited one (drizzle's `Table` members, late-bound under a derived
/// class's file) is not.
pub(crate) fn late_bind_member_names(
    properties: &mut PropertyMap,
    drop_unresolved: bool,
    ctx: &mut CheckerContext,
) -> bool {
    if !properties.keys().any(|name| is_late_bindable_name(name)) || LATE_BINDING.with(Cell::get) {
        return false;
    }
    LATE_BINDING.with(|active| active.set(true));
    let mut renamed: Vec<(Arc<str>, Arc<str>)> = Vec::new();
    let mut invalid: Vec<Arc<str>> = Vec::new();
    let mut unresolved = false;
    for name in properties.keys().filter(|name| is_late_bindable_name(name)) {
        match late_bound_name(name, drop_unresolved, ctx) {
            Ok(LateBound::Name(bound)) if bound.as_str() != name.as_ref() => renamed.push((name.clone(), bound.into())),
            Ok(LateBound::Invalid) => invalid.push(name.clone()),
            Ok(_) => {}
            Err(Unresolved) => {
                unresolved |= ctx.collecting_signatures && ctx.cross_file_resolution_depth == 0;
            }
        }
    }
    LATE_BINDING.with(|active| active.set(false));
    // A key no property name can come from (an enum object, a name that does
    // not resolve) binds no member at all (`isLateBindableName`).
    for name in &invalid {
        properties.shift_remove(name);
    }
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
/// literal: whether it is numeric. `None` for any other key — `any` too,
/// which tsc takes as a numeric index but surge also reads where a key's
/// declaration did not resolve (a unique symbol it could not follow).
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
            Type::Number => Some(true),
            // A pattern key is a string one (`isTypeAssignableTo(keyType,
            // stringType)` in `getIndexInfosOfIndexSymbol`).
            other if surge_ts_types::is_template_literal_type(other)
                || surge_ts_types::string_mapping_parts(other).is_some() =>
            {
                Some(false)
            }
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

enum LateBound {
    /// The property name the key's type binds the member to.
    Name(String),
    /// A key of a type no property name comes from but an index may (a
    /// `string` or `number`): the member stays for the index to read.
    Unusable,
    /// A key that is no property key at all (TS2464) or names nothing.
    Invalid,
}

/// The member name a key written as `[path]` binds to.
fn late_bound_name(written: &str, drop_unresolved: bool, ctx: &mut CheckerContext) -> Result<LateBound, Unresolved> {
    let path = &written[1..written.len() - 1];
    let mut segments = path.split('.');
    let Some(name) = segments.next().map(str::to_string) else {
        return Ok(LateBound::Unusable);
    };
    let head = name.clone();
    let members: Vec<String> = segments.map(str::to_string).collect();
    if name.is_empty() || members.iter().any(String::is_empty) {
        return Ok(LateBound::Unusable);
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
    // An unresolved name comes back `unknown`, which is otherwise a key still
    // to be resolved. Before the check phase a lookup can miss a value not yet
    // bound, so only the check phase's miss names nothing.
    let names_nothing = drop_unresolved
        && crate::program::in_check_phase()
        && ctx.diagnostics()[checkpoint..].iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                surge_ts_diagnostics::DiagnosticCode::TypeScript(2304 | 2552 | 2503)
            )
        })
        && !value_name_is_known(&head, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    // Outside the collection pass a name that does not resolve never will.
    if (matches!(key_type, Type::ErrorType) || names_nothing)
        && !(ctx.collecting_signatures && ctx.cross_file_resolution_depth == 0)
    {
        return Ok(LateBound::Invalid);
    }
    if key_type.is_unknown() || matches!(key_type, Type::ErrorType) {
        return Err(Unresolved);
    }
    let bound = match &key_type {
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
    };
    Ok(match bound {
        Some(name) => LateBound::Name(name),
        // An object (an enum's or a namespace's value) is never a key.
        None if matches!(key_type.peeled(), Type::Object(_)) => LateBound::Invalid,
        None => LateBound::Unusable,
    })
}

fn value_name_is_known(name: &str, ctx: &CheckerContext) -> bool {
    ctx.symbols.get(name).is_some()
        || ctx.module_value_fallback.as_ref().is_some_and(|table| table.get(name).is_some())
        || ctx.module_local_values_for_file(&ctx.file_name).is_some_and(|table| table.get(name).is_some())
        || ctx.ambient_global_symbols.get(name).is_some()
        || crate::program::program_ambient_globals().is_some_and(|globals| globals.get(name).is_some())
}
