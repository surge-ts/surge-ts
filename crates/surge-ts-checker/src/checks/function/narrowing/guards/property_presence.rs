use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, TypeCopyReason, union_type};

use crate::symbols::{SymbolInfo, SymbolTable};

/// What a union member proves about `"prop" in value`.
pub(super) enum PropertyPresence {
    /// Declared and required: every value of the member has the key.
    Required,
    /// Declared optional: the key may or may not be there at runtime, so the
    /// member survives *both* branches (tsc keeps `{ a?: X }` in the else branch
    /// of `if ("a" in v)`).
    Optional,
    Absent,
    /// Not statically decidable (a string index signature, a non-object member).
    Undecidable,
}

pub(super) fn property_presence_of(member: &Type, property: &str) -> PropertyPresence {
    match member {
        Type::Object(object) => match object.get_property(property) {
            Some(existing) if existing.is_optional() => PropertyPresence::Optional,
            Some(_) => PropertyPresence::Required,
            // A string index answers no symbol key, but an index surge opened
            // over an operand it could not model proves nothing either way.
            None if object.allows_string_index_access()
                && (!property.starts_with("[Symbol.") || object.synthetic_open_index) =>
            {
                PropertyPresence::Undecidable
            }
            None => PropertyPresence::Absent,
        },
        // No key is present on `null` or `undefined` (`isTypePresencePossible`
        // finds neither a property nor an index signature).
        Type::Null | Type::Undefined | Type::Void => PropertyPresence::Absent,
        // `isTypePresencePossible` reads a type variable of the body being
        // checked through its apparent type: its constraint, or `{}`, which
        // has no keys, without one. `"then" in v` on `T | PromiseLike<T>`
        // keeps only the promise.
        Type::TypeParameter(parameter) => {
            match surge_ts_types::type_variable::active_constraint(parameter) {
                Some(None) => PropertyPresence::Absent,
                Some(Some(constraint)) => match constraint.peeled() {
                    Type::TypeParameter(_) => PropertyPresence::Undecidable,
                    apparent => property_presence_of(&apparent, property),
                },
                None => PropertyPresence::Undecidable,
            }
        }
        _ => PropertyPresence::Undecidable,
    }
}

pub(super) enum PresenceNarrowing {
    Kept,
    Removed,
    Narrowed(Type),
}

/// Decides a single union member. Named aliases and interfaces arrive as nominal
/// `Type::Reference` wrappers, and an alias to a union stays a nested union after
/// peeling — both must be looked through, or every member is "undecidable" and
/// the guard narrows nothing. The unpeeled member is what the caller keeps when
/// nothing was dropped, so nominal display names survive in diagnostics.
pub(super) fn narrow_member_by_property_presence(
    member: &Type,
    property: &str,
    keep_present: bool,
) -> PresenceNarrowing {
    let peeled = member.peeled();
    if let Type::Union(inner) = &peeled {
        let mut kept = Vec::new();
        let mut changed = false;
        for constituent in inner.types().iter() {
            match narrow_member_by_property_presence(constituent, property, keep_present) {
                PresenceNarrowing::Kept => kept.push(constituent.clone()),
                PresenceNarrowing::Removed => changed = true,
                PresenceNarrowing::Narrowed(narrowed) => {
                    changed = true;
                    kept.push(narrowed);
                }
            }
        }
        if !changed {
            return PresenceNarrowing::Kept;
        }
        if kept.is_empty() {
            return PresenceNarrowing::Removed;
        }
        return PresenceNarrowing::Narrowed(union_type(kept));
    }

    let survives = match property_presence_of(&peeled, property) {
        PropertyPresence::Required => keep_present,
        PropertyPresence::Absent => !keep_present,
        PropertyPresence::Optional | PropertyPresence::Undecidable => true,
    };
    if survives {
        PresenceNarrowing::Kept
    } else {
        PresenceNarrowing::Removed
    }
}

/// Members every object answers through `Object.prototype`. `"toString" in x` is
/// true for any object, so it proves nothing new, and synthesizing the key would
/// shadow the real member and turn a working `x.toString()` into an error.
pub(super) fn is_object_prototype_member(property: &str) -> bool {
    matches!(
        property,
        "toString"
            | "toLocaleString"
            | "valueOf"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "constructor"
            | "__proto__"
    )
}

/// tsc's `narrowTypeByInKeyword` (flow.go). When some constituent can have
/// the key (`isTypePresencePossible`), the type is filtered by whether each
/// constituent can have it in this branch — which leaves `never` when none can,
/// a non-union type included. Otherwise only the true branch learns the key,
/// as tsc's intersection with `Record<"p", unknown>`.
pub(crate) fn narrow_union_by_property_presence(
    ty: &Type,
    property: &str,
    keep_present: bool,
) -> Option<Type> {
    let peeled = ty.peeled();
    if presence_possible(&peeled, property) {
        return match narrow_member_by_property_presence(ty, property, keep_present) {
            PresenceNarrowing::Kept => None,
            PresenceNarrowing::Removed => Some(Type::Never),
            PresenceNarrowing::Narrowed(narrowed) => Some(narrowed),
        };
    }
    // The key is a required literal, never an index signature, which would make
    // every other read off the value a permissive hit. A member every object
    // answers through `Object.prototype` is left alone: synthesizing it would
    // shadow the real member.
    if !keep_present || is_object_prototype_member(property) {
        return None;
    }
    match &peeled {
        Type::Union(union) => {
            let members: Vec<Type> = union
                .types()
                .iter()
                .map(|member| {
                    with_unknown_property(member, property).unwrap_or_else(|| member.clone())
                })
                .collect();
            let narrowed = union_type(members);
            (narrowed != peeled).then_some(narrowed)
        }
        // `narrowTypeByInKeyword` intersects the type with `Record<"p",
        // unknown>`, and it prints as that intersection.
        other => with_unknown_property(other, property).map(|narrowed| match narrowed {
            Type::Object(object)
                if !property.contains(['"', '\\'])
                    && !surge_ts_types::is_numeric_key(property)
                    && object.alias_name.is_none() =>
            {
                let record = format!("Record<\"{property}\", unknown>");
                // `{}` adds nothing to the intersection, which is the record alone.
                let name = match ty {
                    Type::Object(empty) if empty.properties.is_empty() && !empty.non_primitive && empty.alias_name.is_none()
                        && empty.string_index_type.is_none() && empty.number_index_type.is_none()
                        && empty.call_signature.is_none() && empty.construct_signature.is_none() => record,
                    Type::Union(_) | Type::Function(_) => format!("({}) & {record}", ty.name()),
                    _ => format!("{} & {record}", ty.name()),
                };
                Type::Object(object.with_alias_name(name))
            }
            narrowed => narrowed,
        }),
    }
}

/// Whether some constituent of `ty` can have `property`: it declares it, or an
/// index signature answers it (`isTypePresencePossible` assuming presence).
fn presence_possible(ty: &Type, property: &str) -> bool {
    match ty.peeled() {
        Type::Union(union) => {
            union.types().iter().any(|member| presence_possible(member, property))
        }
        member @ Type::Object(_) => !matches!(
            property_presence_of(&member, property),
            PropertyPresence::Absent
        ),
        member @ Type::TypeParameter(_) => matches!(
            property_presence_of(&member, property),
            PropertyPresence::Required | PropertyPresence::Optional
        ),
        _ => false,
    }
}

/// An object that learned `property` is present, typed `unknown`.
fn with_unknown_property(member: &Type, property: &str) -> Option<Type> {
    // `narrowTypeByInKeyword`: a type variable that declares no such property
    // is intersected with `Record<property, unknown>`, staying a `T`.
    if member.is_type_variable() {
        let mut properties = surge_ts_types::PropertyMap::default();
        properties.insert(
            std::sync::Arc::from(property),
            surge_ts_types::ObjectProperty::required(Type::GenuineUnknown),
        );
        return Some(surge_ts_types::type_variable::intersect_type_variable(
            member,
            Type::Object(crate::metrics::alloc_object_type(properties, None)),
        ));
    }
    let Type::Object(object) = member.peeled() else {
        return None;
    };
    let mut properties = (*object.properties).clone();
    properties.insert(
        std::sync::Arc::from(property),
        surge_ts_types::ObjectProperty::required(Type::GenuineUnknown),
    );
    Some(Type::Object(crate::metrics::alloc_object_type(
        properties,
        object.string_index_type.as_deref().cloned(),
    )))
}

/// Parses a `"property" in object` test, returning the object expression and the
/// property name.
pub(crate) fn parse_in_condition(
    condition: &ParsedExpression,
) -> Option<(&ParsedExpression, &str)> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        left,
        operator: ParsedBinaryOperator::In,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let property = match left.as_ref() {
        ParsedExpression::StringLiteral(property) => property.as_str(),
        // A template without substitutions has the string literal type it spells.
        ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        } if expressions.is_empty() => quasis.first()?.as_deref()?,
        // A number names the property its canonical spelling does
        // (`getPropertyNameFromType`: `1 in x` tests `"1"`); a literal written
        // any other way is left unparsed.
        ParsedExpression::NumberLiteral(value)
            if value
                .parse::<f64>()
                .is_ok_and(|number| surge_ts_syntax::js_number_to_string(number) == *value) =>
        {
            value.as_str()
        }
        // `Symbol.iterator in heads`: a well-known symbol is a property name
        // too (tsc's `isTypeUsableAsPropertyName`), keyed the way a computed
        // `[Symbol.iterator]` member is.
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } if matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "Symbol") => {
            well_known_symbol_key(property_name)?
        }
        _ => return None,
    };
    Some((right.as_ref(), property))
}

/// A `key in object` test whose key is no literal but an expression typed as a
/// property name (`isTypeUsableAsPropertyName`): a string or number literal
/// type, or a unique symbol (`const sym = Symbol(); sym in x`).
pub(crate) fn parse_typed_key_in_condition<'a>(
    condition: &'a ParsedExpression,
    key_type: &dyn Fn(&ParsedExpression) -> Option<Type>,
) -> Option<(&'a ParsedExpression, String)> {
    let ParsedExpression::Binary {
        left,
        operator: surge_ts_syntax::ParsedBinaryOperator::In,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let key = match key_type(left)? {
        Type::StringLiteral(value) => value.to_string(),
        Type::NumberLiteral(number) => number.value.to_string(),
        Type::Reference(reference) if reference.is_unique_symbol() => {
            format!("[{}]", reference.unique_symbol_name()?)
        }
        _ => return None,
    };
    Some((right.as_ref(), key))
}

fn well_known_symbol_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "asyncDispose" => "[Symbol.asyncDispose]",
        "asyncIterator" => "[Symbol.asyncIterator]",
        "dispose" => "[Symbol.dispose]",
        "hasInstance" => "[Symbol.hasInstance]",
        "isConcatSpreadable" => "[Symbol.isConcatSpreadable]",
        "iterator" => "[Symbol.iterator]",
        "match" => "[Symbol.match]",
        "matchAll" => "[Symbol.matchAll]",
        "replace" => "[Symbol.replace]",
        "search" => "[Symbol.search]",
        "species" => "[Symbol.species]",
        "split" => "[Symbol.split]",
        "toPrimitive" => "[Symbol.toPrimitive]",
        "toStringTag" => "[Symbol.toStringTag]",
        "unscopables" => "[Symbol.unscopables]",
        _ => return None,
    })
}

/// Builds a symbol table narrowed by a `"prop" in obj` test for the given branch.
pub(crate) fn narrow_property_presence_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let typed_key;
    let (object, property) = match parse_in_condition(condition) {
        Some(parsed) => parsed,
        None => {
            let (object, key) = parse_typed_key_in_condition(condition, &|key| match key {
                ParsedExpression::Identifier { name, .. } => symbols.get(name).map(|symbol| symbol.ty.clone()),
                _ => None,
            })?;
            typed_key = key;
            (object, typed_key.as_str())
        }
    };
    let (name, path) = super::super::reference_path(object)?;
    if !path.is_empty() {
        return None;
    }
    let symbol = symbols.get(&name)?;
    let narrowed = narrow_union_by_property_presence(&symbol.ty, property, branch_is_true)?;
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        name,
        SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        },
        symbol.ty.clone(),
    );
    Some(narrowed_symbols)
}
