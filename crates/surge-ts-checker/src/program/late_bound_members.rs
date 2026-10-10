//! tsc's late binding of a class's dynamically named members
//! (`lateBindMember`) and the duplicate reports that read its result
//! (`mergeSymbol`, `checkObjectTypeForDuplicateDeclarations`). A computed name
//! written as an entity name whose type is a string or number literal or a
//! unique symbol names a member as a written name does; the grammar pass,
//! which reads names alone, cannot see those.

use std::collections::{HashMap, HashSet};

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedBindingName, ParsedClassDeclaration, ParsedClassMember, ParsedExpression, TextSpan,
};
use surge_ts_types::Type;

use crate::context::{CheckerContext, convert_span};
use crate::infer::{InferredExpression, infer_expression};

const PROPERTY: u8 = 1;
const METHOD: u8 = 1 << 1;
const GET_ACCESSOR: u8 = 1 << 2;
const SET_ACCESSOR: u8 = 1 << 3;
const ACCESSOR: u8 = GET_ACCESSOR | SET_ACCESSOR;
const VALUE: u8 = PROPERTY | METHOD | ACCESSOR;

/// The symbols `Symbol` names that are unique symbols in the lib.
pub(crate) const WELL_KNOWN_SYMBOLS: &[&str] = &[
    "asyncDispose",
    "asyncIterator",
    "dispose",
    "hasInstance",
    "isConcatSpreadable",
    "iterator",
    "match",
    "matchAll",
    "metadata",
    "replace",
    "search",
    "species",
    "split",
    "toPrimitive",
    "toStringTag",
    "unscopables",
];

/// `getExcludedSymbolFlags` over the meanings a class member declares.
fn excluded_flags(flags: u8) -> u8 {
    let mut excluded = 0;
    if flags & METHOD != 0 {
        excluded |= VALUE & !METHOD;
    }
    if flags & GET_ACCESSOR != 0 {
        excluded |= VALUE & !SET_ACCESSOR;
    }
    if flags & SET_ACCESSOR != 0 {
        excluded |= VALUE & !GET_ACCESSOR;
    }
    excluded
}

/// What `checkObjectTypeForDuplicateDeclarations` makes of a member.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DuplicateKind {
    Unchecked,
    Property,
    Accessor,
}

struct Member {
    is_static: bool,
    /// The symbol name: the written name, the literal a late-bound key is
    /// typed as, or a unique symbol's identity.
    name: String,
    display: String,
    flags: u8,
    late: bool,
    duplicate_kind: DuplicateKind,
    span: Option<TextSpan>,
}

pub(crate) fn check_late_bound_members(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    let late = late_bound_members(class, ctx);
    if late.is_empty() {
        return;
    }
    let mut members = early_bound_members(class);
    members.extend(late);
    members.sort_by_key(|member| member.span.map_or(usize::MAX, |span| span.start));
    report_duplicate_members(&members, ctx);
}

/// `lateBindMember` for an interface: a computed name (`[c0]`) whose key is a
/// literal names the member its value does, and one sharing that name with a
/// differently written member is a duplicate at both (TS2300). Members written
/// alike are the interface check's own; methods are overloads.
pub(crate) fn check_interface_late_bound_members(
    members: &[surge_ts_syntax::ParsedInterfaceMember],
    ctx: &mut CheckerContext,
) {
    if !members.iter().any(|member| member.name.starts_with('[')) {
        return;
    }
    let names: Vec<Option<String>> = members
        .iter()
        .map(|member| {
            let Some(path) = member.name.strip_prefix('[').and_then(|name| name.strip_suffix(']')) else {
                return Some(member.name.clone());
            };
            if path.starts_with("Symbol.")
                || path.is_empty()
                || !path.split('.').all(|part| {
                    part.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
                        && part.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                })
            {
                return None;
            }
            let mut segments = path.split('.');
            let mut key = ParsedExpression::Identifier {
                name: segments.next()?.to_string(),
                span: None,
            };
            for segment in segments {
                key = ParsedExpression::PropertyAccess {
                    object: Box::new(key),
                    object_span: None,
                    property_name: segment.to_string(),
                    property_span: None,
                    is_bracketed: false,
                    binding_element: false,
                };
            }
            late_bound_name(&key, path, &member.name, ctx)
                .map(|(name, _)| name)
                .filter(|name| !name.starts_with("__@"))
        })
        .collect();
    let mut reported = HashSet::new();
    for (index, member) in members.iter().enumerate() {
        if !member.name.starts_with('[') {
            continue;
        }
        let Some(name) = &names[index] else {
            continue;
        };
        for (other_index, other) in members.iter().enumerate() {
            if other_index == index
                || other.name == member.name
                || names[other_index].as_ref() != Some(name)
                || (member.is_method && other.is_method)
            {
                continue;
            }
            for duplicate in [member, other] {
                if let Some(span) = duplicate.name_span
                    && reported.insert(span.start)
                {
                    ctx.push(Diagnostic::ts2300(&duplicate.name, ctx.file_name.clone()).with_span(convert_span(span)));
                }
            }
        }
    }
    // Static names are bound before late-bound ones (`lateBindMember`), so a
    // late-bound property is the subsequent declaration of a static name it
    // shares: TS2717 when the types differ.
    let late_bound = |index: usize| members[index].name.starts_with('[') && names[index].is_some();
    let order: Vec<usize> =
        (0..members.len()).filter(|&index| !late_bound(index)).chain((0..members.len()).filter(|&index| late_bound(index))).collect();
    for (position, &index) in order.iter().enumerate() {
        let member = &members[index];
        let Some(name) = &names[index] else {
            continue;
        };
        if member.is_method {
            continue;
        }
        let Some(&earlier) = order[..position]
            .iter()
            .find(|&&other| names[other].as_ref() == Some(name) && !members[other].is_method)
        else {
            continue;
        };
        if !late_bound(index) && !late_bound(earlier) {
            continue;
        }
        let (Some(expected), Some(actual)) = (
            crate::driver::parsed_type_display(&members[earlier].ty),
            crate::driver::parsed_type_display(&member.ty),
        ) else {
            continue;
        };
        if expected != actual
            && let Some(span) = member.name_span
        {
            ctx.push(
                Diagnostic::ts2717(&member.name, expected, actual, ctx.file_name.clone()).with_span(convert_span(span)),
            );
        }
    }
}

/// `checkComputedPropertyName` over an interface's computed member names: an
/// entity name key must be a string, number or symbol (TS2464), reported over
/// its brackets.
pub(crate) fn check_interface_computed_key_types(
    members: &[surge_ts_syntax::ParsedInterfaceMember],
    ctx: &mut CheckerContext,
) {
    for member in members {
        let Some(path) = member.name.strip_prefix('[').and_then(|name| name.strip_suffix(']')) else {
            continue;
        };
        let Some(name_span) = member.name_span else {
            continue;
        };
        let Some(key) = entity_name_expression(path) else {
            continue;
        };
        // A well-known symbol member's span is its key's alone.
        let span = if path.starts_with("Symbol.") && name_span.start > 0 {
            TextSpan { start: name_span.start - 1, end: name_span.end + 1 }
        } else {
            name_span
        };
        let checkpoint = ctx.diagnostics().len();
        let symbols = ctx.symbols.clone();
        let key_type = infer_expression(&key, &symbols, ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        crate::checks::expr::report_invalid_computed_key(&key_type, Some(span), ctx);
    }
}

fn entity_name_expression(path: &str) -> Option<ParsedExpression> {
    let is_identifier = |part: &str| {
        part.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
            && part.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    if path.is_empty() || !path.split('.').all(is_identifier) {
        return None;
    }
    let mut segments = path.split('.');
    let mut key = ParsedExpression::Identifier { name: segments.next()?.to_string(), span: None };
    for segment in segments {
        key = ParsedExpression::PropertyAccess {
            object: Box::new(key),
            object_span: None,
            property_name: segment.to_string(),
            property_span: None,
            is_bracketed: false,
            binding_element: false,
        };
    }
    Some(key)
}

/// The members the binder names from what is written. An accessor pair is
/// one parsed member, reported at its first name.
fn early_bound_members(class: &ParsedClassDeclaration) -> Vec<Member> {
    let mut members = Vec::new();
    let mut push = |is_static: bool, name: &str, flags: u8, duplicate_kind: DuplicateKind, span: Option<TextSpan>| {
        members.push(Member {
            is_static,
            name: name.to_string(),
            display: name.to_string(),
            flags,
            late: false,
            duplicate_kind,
            span,
        });
    };
    for member in &class.members {
        match member {
            ParsedClassMember::Property(property) if !is_computed_name(&property.name) => {
                push(property.is_static, property.name.as_str(), PROPERTY, DuplicateKind::Property, property.name_span);
            }
            ParsedClassMember::Method(method) if !is_computed_name(&method.name) => {
                push(method.is_static, method.name.as_str(), METHOD, DuplicateKind::Unchecked, method.name_span);
            }
            ParsedClassMember::Accessor(accessor) if !is_computed_name(&accessor.name) => {
                for declaration in &accessor.declarations {
                    push(
                        accessor.is_static,
                        accessor.name.as_str(),
                        accessor_flags(declaration.is_getter),
                        DuplicateKind::Accessor,
                        accessor.name_span,
                    );
                }
            }
            ParsedClassMember::Constructor(constructor) => {
                for parameter in constructor.parameters.iter().filter(|parameter| parameter.is_parameter_property) {
                    if let ParsedBindingName::Identifier { name, span } = &parameter.binding_name {
                        push(false, name.as_str(), PROPERTY, DuplicateKind::Property, *span);
                    }
                }
            }
            _ => {}
        }
    }
    members
}

/// The parser names a member with an entity-name key by the key as written,
/// in brackets.
fn is_computed_name(name: &str) -> bool {
    name.starts_with('[')
}

fn accessor_flags(is_getter: bool) -> u8 {
    if is_getter { GET_ACCESSOR } else { SET_ACCESSOR }
}

/// The members `lateBindMember` binds: those whose computed key is an entity
/// name (`isLateBindableAST`) typed as something usable as a property name.
fn late_bound_members(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) -> Vec<Member> {
    let mut members = Vec::new();
    let mut next_declarations: HashMap<usize, usize> = HashMap::new();
    for (key, key_span) in &class.computed_keys {
        let Some(path) = entity_name_path(key) else {
            continue;
        };
        let written = format!("[{path}]");
        let Some((is_static, flags, duplicate_kind)) =
            computed_member(class, &written, *key_span, &mut next_declarations)
        else {
            continue;
        };
        let Some((name, display)) = late_bound_name(key, &path, &written, ctx) else {
            continue;
        };
        members.push(Member {
            is_static,
            name,
            display,
            flags,
            late: true,
            duplicate_kind,
            span: *key_span,
        });
    }
    members
}

/// `isEntityNameExpression`: an identifier, or a dotted access on one.
fn entity_name_path(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        } => Some(format!("{}.{property_name}", entity_name_path(object)?)),
        _ => None,
    }
}

/// The member a computed key belongs to: its staticness, its meaning, and
/// what the duplicate check makes of it. A member's name span is its key's
/// without the brackets. The parser folds every accessor of one name into one
/// member that keeps only the first name's span, so a key no member starts at
/// is that accessor's next declaration.
fn computed_member(
    class: &ParsedClassDeclaration,
    written: &str,
    key_span: Option<TextSpan>,
    next_declarations: &mut HashMap<usize, usize>,
) -> Option<(bool, u8, DuplicateKind)> {
    let name_start = key_span?.start + 1;
    let starts_here = |span: Option<TextSpan>| span.is_some_and(|span| span.start == name_start);
    for (index, member) in class.members.iter().enumerate() {
        match member {
            ParsedClassMember::Property(property)
                if property.name == written && starts_here(property.name_span) =>
            {
                return Some((property.is_static, PROPERTY, DuplicateKind::Property));
            }
            ParsedClassMember::Method(method) if method.name == written && starts_here(method.name_span) => {
                return Some((method.is_static, METHOD, DuplicateKind::Unchecked));
            }
            ParsedClassMember::Accessor(accessor)
                if accessor.name == written && starts_here(accessor.name_span) =>
            {
                next_declarations.insert(index, 1);
                let first = accessor.declarations.first()?;
                return Some((accessor.is_static, accessor_flags(first.is_getter), DuplicateKind::Accessor));
            }
            _ => {}
        }
    }
    let mut accessors = class.members.iter().enumerate().filter_map(|(index, member)| match member {
        ParsedClassMember::Accessor(accessor) if accessor.name == written => Some((index, accessor)),
        _ => None,
    });
    let (index, accessor) = accessors.next()?;
    if accessors.next().is_some() {
        return None;
    }
    let position = next_declarations.get_mut(&index)?;
    let declaration = accessor.declarations.get(*position)?;
    *position += 1;
    Some((accessor.is_static, accessor_flags(declaration.is_getter), DuplicateKind::Accessor))
}

/// `lateBindMember`'s name for a key `checkComputedPropertyName` types as a
/// string or number literal or a unique symbol; a well-known symbol, which
/// surge types as `symbol`, is one.
fn late_bound_name(
    key: &ParsedExpression,
    path: &str,
    written: &str,
    ctx: &mut CheckerContext,
) -> Option<(String, String)> {
    if let Some(member) = path.strip_prefix("Symbol.") {
        return WELL_KNOWN_SYMBOLS
            .contains(&member)
            .then(|| (format!("__@{path}"), written.to_string()));
    }
    let checkpoint = ctx.diagnostics().len();
    let symbols = ctx.symbols.clone();
    let key_type = infer_expression(key, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let InferredExpression::Known(key_type) = key_type else {
        return None;
    };
    match key_type {
        Type::StringLiteral(value) => Some((value.clone(), value)),
        Type::NumberLiteral(literal) => Some((literal.value.clone(), literal.value)),
        Type::Reference(reference) if reference.is_unique_symbol() => {
            Some((format!("__@{}", reference.id), written.to_string()))
        }
        _ => None,
    }
}

/// The binder's symbols for the written names, `lateBindMember` over the
/// late-bound members, `combineSymbolTables`' merge of the two, and then
/// `checkObjectTypeForDuplicateDeclarations`. A name only written members
/// share is the grammar pass's to report.
fn report_duplicate_members(members: &[Member], ctx: &mut CheckerContext) {
    let mut symbols: Vec<(u8, Vec<usize>)> = Vec::new();
    let mut symbol_of = vec![0usize; members.len()];

    let mut early: HashMap<(bool, &str), usize> = HashMap::new();
    for (index, member) in members.iter().enumerate().filter(|(_, member)| !member.late) {
        let key = (member.is_static, member.name.as_str());
        let symbol = match early.get(&key).copied() {
            Some(symbol) if symbols[symbol].0 & excluded_flags(member.flags) == 0 => symbol,
            // The binder's own conflict, which it reports: the member keeps a
            // symbol of its own.
            Some(_) => {
                symbols.push((0, Vec::new()));
                symbols.len() - 1
            }
            None => {
                symbols.push((0, Vec::new()));
                early.insert(key, symbols.len() - 1);
                symbols.len() - 1
            }
        };
        symbols[symbol].0 |= member.flags;
        symbols[symbol].1.push(index);
        symbol_of[index] = symbol;
    }

    let mut late: HashMap<(bool, &str), usize> = HashMap::new();
    let mut late_names: Vec<(bool, &str)> = Vec::new();
    for (index, member) in members.iter().enumerate().filter(|(_, member)| member.late) {
        let key = (member.is_static, member.name.as_str());
        let symbol = match late.get(&key).copied() {
            Some(symbol) => symbol,
            None => {
                symbols.push((0, Vec::new()));
                late.insert(key, symbols.len() - 1);
                late_names.push(key);
                symbols.len() - 1
            }
        };
        if symbols[symbol].0 & excluded_flags(member.flags) == 0 {
            symbols[symbol].0 |= member.flags;
            symbols[symbol].1.push(index);
            symbol_of[index] = symbol;
            continue;
        }
        let mut declarations = early.get(&key).map(|&early_symbol| symbols[early_symbol].1.clone()).unwrap_or_default();
        declarations.extend(symbols[symbol].1.iter().copied());
        declarations.push(index);
        for declaration in declarations {
            report(&members[declaration], &member.display, ctx);
        }
        let accessors = symbols[symbol].0 & ACCESSOR;
        if accessors != 0 && accessors != member.flags & ACCESSOR {
            symbols[symbol].0 |= ACCESSOR;
        }
        symbols.push((member.flags, vec![index]));
        symbol_of[index] = symbols.len() - 1;
    }

    for key in &late_names {
        let (Some(&early_symbol), Some(&late_symbol)) = (early.get(key), late.get(key)) else {
            continue;
        };
        if symbols[early_symbol].0 & excluded_flags(symbols[late_symbol].0) != 0 {
            // `reportMergeSymbolError`, at every declaration of both.
            let display = members[symbols[late_symbol].1[0]].display.clone();
            let declarations: Vec<usize> =
                symbols[late_symbol].1.iter().chain(&symbols[early_symbol].1).copied().collect();
            for declaration in declarations {
                report(&members[declaration], &display, ctx);
            }
            continue;
        }
        let late_declarations = std::mem::take(&mut symbols[late_symbol].1);
        let late_flags = symbols[late_symbol].0;
        symbols[early_symbol].0 |= late_flags;
        for &declaration in &late_declarations {
            symbol_of[declaration] = early_symbol;
        }
        symbols[early_symbol].1.extend(late_declarations);
    }

    let late_keys: HashSet<(bool, &str)> = late_names.iter().copied().collect();
    let mut states: HashMap<(bool, &str), DuplicateKind> = HashMap::new();
    let mut reported: HashSet<(bool, &str)> = HashSet::new();
    for (index, member) in members.iter().enumerate() {
        let key = (member.is_static, member.name.as_str());
        if member.duplicate_kind == DuplicateKind::Unchecked
            || !late_keys.contains(&key)
            || reported.contains(&key)
            || symbols[symbol_of[index]].1.len() < 2
        {
            continue;
        }
        let duplicate = match states.get(&key).copied() {
            None => {
                states.insert(key, member.duplicate_kind);
                false
            }
            Some(DuplicateKind::Property) => true,
            Some(_) => member.duplicate_kind != DuplicateKind::Accessor,
        };
        if !duplicate {
            continue;
        }
        reported.insert(key);
        for other in members.iter().filter(|other| other.is_static == member.is_static && other.name == member.name) {
            report(other, &other.display, ctx);
        }
    }
}

fn report(member: &Member, display: &str, ctx: &mut CheckerContext) {
    let diagnostic = Diagnostic::ts2300(display, ctx.file_name.clone());
    ctx.push(match member.span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}
