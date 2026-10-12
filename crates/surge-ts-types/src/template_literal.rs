//! Template literal types (`` `/${string}` ``), ported from tsc's
//! `getTemplateLiteralType`, `inferFromLiteralPartsToTemplateLiteral` and
//! `isValidTypeForTemplateLiteralPlaceholder`.
//!
//! A pattern is carried by a synthetic [`TypeReference`] that resolves to
//! `string`, the way the `readonly` array wrapper resolves to its array: every
//! structural consumer that peels sees a string, while assignability and
//! display can still tell the pattern apart. The arguments alternate
//! `text, placeholder, text, …, text`, texts as string literals, so nominal
//! equality is tsc's interning by `(texts, types)`.

use std::sync::Arc;

use crate::{NumberLiteralType, ResolveReference, Type, TypeReference, is_assignable_to, union_type};

pub const TEMPLATE_LITERAL_REFERENCE_ID: &str = "\u{0}template";

/// tsc refuses a cross product past 100,000 members with an error; surge
/// degrades to `string` well before that, as the finite expansion always did.
const CROSS_PRODUCT_LIMIT: usize = 10_000;

#[derive(Debug)]
struct ResolvesToString;

impl ResolveReference for ResolvesToString {
    fn resolve(&self) -> Type {
        Type::String
    }
}

/// The `(texts, placeholder types)` of a template literal type, `texts` always
/// one longer than the types.
pub fn template_literal_parts(ty: &Type) -> Option<(Vec<&str>, Vec<&Type>)> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    if &*reference.id != TEMPLATE_LITERAL_REFERENCE_ID {
        return None;
    }
    let mut texts = Vec::new();
    let mut types = Vec::new();
    for (index, argument) in reference.arguments.iter().enumerate() {
        if index % 2 == 0 {
            let Type::StringLiteral(text) = argument else {
                return None;
            };
            texts.push(text.as_str());
        } else {
            types.push(argument);
        }
    }
    Some((texts, types))
}

pub const TEMPLATE_PLACEHOLDER_INTERSECTION_ID: &str = "\u{0}template-intersection";

/// A placeholder written as an intersection of patterns
/// (`` `${`a${string}` & `${string}a`}Test` ``), kept whole so each member
/// judges what the placeholder captures. tsc's `isPatternLiteralPlaceholderType`
/// takes an intersection of placeholders, literals and object tags with at
/// least one placeholder.
pub fn template_placeholder_intersection(members: Vec<Type>) -> Option<Type> {
    let literal_or_tag = |ty: &Type| {
        matches!(
            ty,
            Type::StringLiteral(_)
                | Type::NumberLiteral(_)
                | Type::BooleanLiteral(_)
                | Type::Null
                | Type::Undefined
                | Type::Object(_)
        )
    };
    if !members.iter().any(is_pattern_literal_placeholder)
        || !members.iter().all(|member| is_pattern_literal_placeholder(member) || literal_or_tag(member))
    {
        return None;
    }
    let display = members.iter().map(Type::name).collect::<Vec<_>>().join(" & ");
    Some(Type::Reference(TypeReference::new(
        TEMPLATE_PLACEHOLDER_INTERSECTION_ID,
        display,
        members,
        Arc::new(ResolvesToString),
    )))
}

fn template_placeholder_intersection_members(ty: &Type) -> Option<&[Type]> {
    match ty {
        Type::Reference(reference) if &*reference.id == TEMPLATE_PLACEHOLDER_INTERSECTION_ID => {
            Some(&reference.arguments)
        }
        _ => None,
    }
}

pub fn is_template_literal_type(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if &*reference.id == TEMPLATE_LITERAL_REFERENCE_ID)
}

/// Peels lazy references until a pattern literal type (or anything that is not
/// a reference) is reached. A pattern resolves to `string`, so a full peel
/// erases it; an annotation naming a lib alias (`Uppercase<string>`) arrives as
/// a lazy reference with the pattern one level down.
pub fn peel_to_pattern_literal(ty: &Type) -> Type {
    let mut current = ty.clone();
    loop {
        if is_template_literal_type(&current) || string_mapping_parts(&current).is_some() {
            return current;
        }
        let Type::Reference(reference) = &current else {
            return current;
        };
        current = reference.resolve();
    }
}

/// tsc's `getTemplateLiteralType`.
pub fn template_literal_type(texts: &[String], types: &[Type]) -> Type {
    let cross_product = types.iter().fold(1usize, |product, ty| {
        product.saturating_mul(match ty {
            Type::Union(union) => union.types().len().max(1),
            Type::Boolean => 2,
            _ => 1,
        })
    });
    if cross_product > CROSS_PRODUCT_LIMIT {
        return Type::String;
    }
    build(texts, types)
}

fn build(texts: &[String], types: &[Type]) -> Type {
    // A union (or `never`) in any placeholder distributes over the template.
    if let Some(index) = types
        .iter()
        .position(|ty| matches!(ty, Type::Union(_) | Type::Never | Type::Boolean))
    {
        let members: Vec<Type> = match &types[index] {
            Type::Union(union) => union.types().to_vec(),
            Type::Boolean => vec![Type::BooleanLiteral(true), Type::BooleanLiteral(false)],
            _ => Vec::new(),
        };
        let distributed: Vec<Type> = members
            .into_iter()
            .map(|member| {
                let mut replaced = types.to_vec();
                replaced[index] = member;
                build(texts, &replaced)
            })
            .collect();
        return if distributed.is_empty() {
            Type::Never
        } else {
            union_type(distributed)
        };
    }

    let mut new_texts: Vec<String> = Vec::new();
    let mut new_types: Vec<Type> = Vec::new();
    let mut text = texts.first().cloned().unwrap_or_default();
    if !add_spans(texts, types, &mut text, &mut new_texts, &mut new_types) {
        return Type::String;
    }
    if new_types.is_empty() {
        return Type::StringLiteral(text);
    }
    new_texts.push(text);
    if new_types.iter().any(crate::type_variable::is_generic_index_type) {
        let display = template_display(&new_texts, &new_types);
        return crate::type_variable::template_literal_variable(new_texts, new_types, display).unwrap_or(Type::String);
    }
    if new_texts.iter().all(String::is_empty) {
        if new_types.iter().all(|ty| *ty == Type::String) {
            return Type::String;
        }
        // `${Uppercase<string>}` is `Uppercase<string>`.
        if let [only] = new_types.as_slice()
            && string_mapping_parts(only).is_some()
        {
            return only.clone();
        }
    }
    intern(new_texts, new_types)
}

/// Folds literal placeholders into the surrounding text, splices nested
/// templates, and keeps `string`/`number`/`bigint`/`any` as placeholders.
/// `false` means a placeholder tsc has no pattern for, which makes the whole
/// template `string`.
fn add_spans(
    texts: &[String],
    types: &[Type],
    text: &mut String,
    new_texts: &mut Vec<String>,
    new_types: &mut Vec<Type>,
) -> bool {
    for (index, ty) in types.iter().enumerate() {
        let following = texts.get(index + 1).map(String::as_str).unwrap_or("");
        if let Some(rendered) = template_string_for_type(ty) {
            text.push_str(&rendered);
            text.push_str(following);
        } else if let Some((nested_texts, nested_types)) = owned_template_parts(ty) {
            text.push_str(&nested_texts[0]);
            if !add_spans(&nested_texts, &nested_types, text, new_texts, new_types) {
                return false;
            }
            text.push_str(following);
        } else if is_pattern_literal_placeholder(ty) || crate::type_variable::is_generic_index_type(ty) {
            new_types.push(ty.clone());
            new_texts.push(std::mem::take(text));
            text.push_str(following);
        } else {
            return false;
        }
    }
    true
}

/// tsc's `getTemplateStringForType`.
fn template_string_for_type(ty: &Type) -> Option<String> {
    match ty {
        Type::StringLiteral(value) => Some(value.clone()),
        Type::NumberLiteral(NumberLiteralType { value }) => Some(value.clone()),
        Type::BooleanLiteral(value) => Some(value.to_string()),
        Type::Null => Some("null".to_string()),
        Type::Undefined => Some("undefined".to_string()),
        _ => None,
    }
}

fn template_display(texts: &[String], types: &[Type]) -> String {
    let mut display = String::from("`");
    for (index, text) in texts.iter().enumerate() {
        display.push_str(text);
        if let Some(ty) = types.get(index) {
            display.push_str("${");
            display.push_str(&ty.name());
            display.push('}');
        }
    }
    display.push('`');
    display
}

fn intern(texts: Vec<String>, types: Vec<Type>) -> Type {
    let display = template_display(&texts, &types);
    let mut arguments = Vec::with_capacity(texts.len() + types.len());
    for (index, text) in texts.iter().enumerate() {
        arguments.push(Type::StringLiteral(text.clone()));
        if let Some(ty) = types.get(index) {
            arguments.push(ty.clone());
        }
    }
    Type::Reference(TypeReference::new(
        TEMPLATE_LITERAL_REFERENCE_ID,
        display,
        arguments,
        Arc::new(ResolvesToString),
    ))
}

/// tsc's `isTypeMatchedByTemplateLiteralType`.
pub fn is_type_matched_by_template_literal(source: &Type, texts: &[&str], types: &[&Type]) -> bool {
    let Some(inferences) = infer_types_from_template_literal(source, texts, types) else {
        return false;
    };
    inferences
        .iter()
        .zip(types)
        .all(|(inference, placeholder)| is_valid_type_for_placeholder(inference, placeholder))
}

/// tsc's `inferTypesFromTemplateLiteralType`.
fn infer_types_from_template_literal(
    source: &Type,
    texts: &[&str],
    types: &[&Type],
) -> Option<Vec<Type>> {
    if let Type::StringLiteral(value) = source {
        return infer_from_literal_parts(&[value.as_str()], &[], texts);
    }
    let (source_texts, source_types) = owned_template_parts(source)?;
    let source_texts: Vec<&str> = source_texts.iter().map(String::as_str).collect();
    let source_types: Vec<&Type> = source_types.iter().collect();
    if source_texts == texts {
        return Some(
            source_types
                .iter()
                .zip(types)
                .map(|(source_type, target_type)| {
                    if is_assignable_to(
                        &crate::type_variable::base_constraint_or_type(source_type),
                        &crate::type_variable::base_constraint_or_type(target_type),
                    ) {
                        (*source_type).clone()
                    } else {
                        string_like_type_for(source_type)
                    }
                })
                .collect(),
        );
    }
    infer_from_literal_parts(&source_texts, &source_types, texts)
}

/// The parts of a template literal type, a pattern or a generic one.
pub fn owned_template_parts(ty: &Type) -> Option<(Vec<String>, Vec<Type>)> {
    match template_literal_parts(ty) {
        Some((texts, types)) => Some((
            texts.into_iter().map(String::from).collect(),
            types.into_iter().cloned().collect(),
        )),
        None => crate::type_variable::template_literal_variable_parts(ty),
    }
}

/// The parts of a string mapping type, a pattern or a generic one.
fn owned_string_mapping_parts(ty: &Type) -> Option<(StringMappingKind, Type)> {
    match string_mapping_parts(ty) {
        Some((kind, inner)) => Some((kind, inner.clone())),
        None => crate::type_variable::string_mapping_variable_parts(ty),
    }
}

/// tsc's `inferTypesFromTemplateLiteralType` against a template written as
/// `texts` around `types`: what each placeholder captures from `source`, or
/// `None` when `source` does not fit the texts.
pub fn infer_template_literal_placeholders(
    source: &Type,
    texts: &[&str],
    types: &[&Type],
) -> Option<Vec<Type>> {
    infer_types_from_template_literal(source, texts, types)
}

/// The literal a `infer X extends C` placeholder prefers over the captured
/// string `text` (tsc's `inferToTemplateLiteralType`): `"100"` against a
/// `number` constraint is `100`, against `boolean` `"true"` is `true`. `None`
/// keeps the string itself.
pub fn preferred_template_placeholder_inference(text: &str, constraint: &Type) -> Option<Type> {
    let members: Vec<Type> = match constraint {
        Type::Union(union) => union.types().iter().flat_map(distribute_boolean).collect(),
        other => distribute_boolean(other),
    };
    let category = |ty: &Type| PlaceholderCategory::of(ty);
    let mut present: Vec<PlaceholderCategory> = members.iter().filter_map(category).collect();
    if present.contains(&PlaceholderCategory::String) {
        return None;
    }
    let number_value = js_string_to_number(text);
    if !number_value.is_some_and(|value| format_js_number(value) == text) {
        present.retain(|category| {
            !matches!(category, PlaceholderCategory::Number | PlaceholderCategory::NumberLiteral)
        });
    }
    if !is_valid_bigint_string(text) || text.starts_with("0") && text.len() > 1 {
        present.retain(|category| *category != PlaceholderCategory::BigInt);
    }
    let choose = |left: Type, right: &Type| -> Type {
        let Some(right_category) = category(right).filter(|category| present.contains(category)) else {
            return left;
        };
        if let Some(left_category) = category(&left)
            && left_category <= right_category
        {
            return left;
        }
        match (right_category, right) {
            (PlaceholderCategory::Template, _)
                if template_literal_parts(right).is_some_and(|(texts, types)| {
                    is_type_matched_by_template_literal(&Type::StringLiteral(text.to_string()), &texts, &types)
                }) =>
            {
                Type::StringLiteral(text.to_string())
            }
            (PlaceholderCategory::StringMapping, _)
                if string_mapping_parts(right).is_some_and(|(kind, _)| kind.apply(text) == text) =>
            {
                Type::StringLiteral(text.to_string())
            }
            (PlaceholderCategory::StringLiteral, Type::StringLiteral(value)) if value == text => right.clone(),
            (PlaceholderCategory::Number, _) => Type::NumberLiteral(NumberLiteralType {
                value: format_js_number(number_value.unwrap_or_default()),
            }),
            (PlaceholderCategory::NumberLiteral, Type::NumberLiteral(literal))
                if literal.value.parse::<f64>().ok() == number_value =>
            {
                right.clone()
            }
            (PlaceholderCategory::BigInt, _) => Type::BigInt,
            (PlaceholderCategory::BooleanLiteral, Type::BooleanLiteral(value))
                if value.to_string() == text =>
            {
                right.clone()
            }
            (PlaceholderCategory::Undefined, _) if text == "undefined" => right.clone(),
            (PlaceholderCategory::Null, _) if text == "null" => right.clone(),
            _ => left,
        }
    };
    let chosen = members.iter().fold(Type::Never, choose);
    (chosen != Type::Never).then_some(chosen)
}

/// The flag groups tsc's `choose` walks, in its preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PlaceholderCategory {
    String,
    Template,
    StringMapping,
    StringLiteral,
    Number,
    NumberLiteral,
    BigInt,
    BooleanLiteral,
    Undefined,
    Null,
}

impl PlaceholderCategory {
    fn of(ty: &Type) -> Option<Self> {
        Some(match ty {
            Type::String => Self::String,
            _ if is_template_literal_type(ty) => Self::Template,
            _ if string_mapping_parts(ty).is_some() => Self::StringMapping,
            Type::StringLiteral(_) => Self::StringLiteral,
            Type::Number => Self::Number,
            Type::NumberLiteral(_) => Self::NumberLiteral,
            Type::BigInt => Self::BigInt,
            Type::BooleanLiteral(_) => Self::BooleanLiteral,
            Type::Undefined => Self::Undefined,
            Type::Null => Self::Null,
            _ => return None,
        })
    }
}

fn distribute_boolean(ty: &Type) -> Vec<Type> {
    match ty {
        Type::Boolean => vec![Type::BooleanLiteral(false), Type::BooleanLiteral(true)],
        other => vec![other.clone()],
    }
}

/// JavaScript's `Number(text)`, `None` for `NaN` and the infinities.
fn js_string_to_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    let radix = |prefixes: [&str; 2], radix: u32| {
        prefixes
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
            .map(|digits| u64::from_str_radix(digits, radix).ok().map(|value| value as f64))
    };
    if let Some(value) = radix(["0x", "0X"], 16)
        .or_else(|| radix(["0o", "0O"], 8))
        .or_else(|| radix(["0b", "0B"], 2))
    {
        return value;
    }
    if !is_valid_number_string(trimmed) {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// How surge spells a number literal type's value, which is Rust's shortest
/// round-trip form; `-0` reads back as `0`, as JavaScript prints it.
fn format_js_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    value.to_string()
}

/// tsc's `getStringLikeTypeForType`.
fn string_like_type_for(ty: &Type) -> Type {
    if matches!(ty, Type::Any | Type::String | Type::StringLiteral(_)) || is_template_literal_type(ty) {
        return ty.clone();
    }
    template_literal_type(&[String::new(), String::new()], std::slice::from_ref(ty))
}

/// tsc's `inferFromLiteralPartsToTemplateLiteral`: the source's first text has
/// to start with the target's, its last has to end with the target's, and each
/// inner target text is found leftmost in what remains. What lies between two
/// matches is the inference for that placeholder — a string literal when it
/// sits inside one source text, a template when it spans several.
fn infer_from_literal_parts(
    source_texts: &[&str],
    source_types: &[&Type],
    target_texts: &[&str],
) -> Option<Vec<Type>> {
    let last_source = source_texts.len() - 1;
    let source_start = source_texts[0];
    let source_end = source_texts[last_source];
    let last_target = target_texts.len() - 1;
    let target_start = target_texts[0];
    let target_end = target_texts[last_target];
    if (last_source == 0 && source_start.len() < target_start.len() + target_end.len())
        || !source_start.starts_with(target_start)
        || !source_end.ends_with(target_end)
    {
        return None;
    }
    let remaining_end = &source_end[..source_end.len() - target_end.len()];
    let source_text = |index: usize| -> &str {
        if index < last_source {
            source_texts[index]
        } else {
            remaining_end
        }
    };

    let mut seg = 0usize;
    let mut pos = target_start.len();
    let mut matches: Vec<Type> = Vec::new();
    let mut add_match = |s: usize, p: usize, seg: &mut usize, pos: &mut usize| {
        let match_type = if s == *seg {
            Type::StringLiteral(source_text(s)[*pos..p].to_string())
        } else {
            let mut match_texts: Vec<String> = Vec::with_capacity(s - *seg + 1);
            match_texts.push(source_texts[*seg][*pos..].to_string());
            match_texts.extend(source_texts[*seg + 1..s].iter().map(|text| (*text).to_string()));
            match_texts.push(source_text(s)[..p].to_string());
            let match_types: Vec<Type> =
                source_types[*seg..s].iter().map(|ty| (*ty).clone()).collect();
            template_literal_type(&match_texts, &match_types)
        };
        matches.push(match_type);
        *seg = s;
        *pos = p;
    };

    for delimiter in &target_texts[1..last_target.max(1)] {
        if !delimiter.is_empty() {
            let mut s = seg;
            let mut p = pos;
            loop {
                if let Some(found) = source_text(s).get(p..).and_then(|rest| rest.find(delimiter)) {
                    p += found;
                    break;
                }
                s += 1;
                if s == source_texts.len() {
                    return None;
                }
                p = 0;
            }
            add_match(s, p, &mut seg, &mut pos);
            pos += delimiter.len();
        } else if pos < source_text(seg).len() {
            let step = source_text(seg)[pos..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
            let (s, p) = (seg, pos + step);
            add_match(s, p, &mut seg, &mut pos);
        } else if seg < last_source {
            let s = seg + 1;
            add_match(s, 0, &mut seg, &mut pos);
        } else {
            return None;
        }
    }
    let end = source_text(last_source).len();
    add_match(last_source, end, &mut seg, &mut pos);
    Some(matches)
}

/// tsc's `isValidTypeForTemplateLiteralPlaceholder`.
fn is_valid_type_for_placeholder(source: &Type, target: &Type) -> bool {
    if let Some(members) = template_placeholder_intersection_members(target) {
        return members.iter().all(|member| {
            matches!(member, Type::Object(object) if object.properties.is_empty()) || is_valid_type_for_placeholder(source, member)
        });
    }
    if *target == Type::String || is_assignable_to(source, target) {
        return true;
    }
    if let Type::StringLiteral(value) = source {
        return match target {
            Type::Number => is_valid_number_string(value),
            Type::BigInt => is_valid_bigint_string(value),
            Type::BooleanLiteral(expected) => *value == expected.to_string(),
            Type::Null => value == "null",
            Type::Undefined => value == "undefined",
            _ if owned_string_mapping_parts(target).is_some() => is_member_of_string_mapping(source, target),
            _ => owned_template_parts(target).is_some_and(|(texts, types)| {
                let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
                let types: Vec<&Type> = types.iter().collect();
                is_type_matched_by_template_literal(source, &texts, &types)
            }),
        };
    }
    if let Some((texts, types)) = owned_template_parts(source) {
        return texts.len() == 2
            && texts[0].is_empty()
            && texts[1].is_empty()
            && is_assignable_to(&types[0], target);
    }
    false
}

/// tsc's `isValidNumberString(s, false)`: a non-empty string `Number(s)` reads
/// as a finite number.
fn is_valid_number_string(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let text = value.trim();
    if text.is_empty() {
        return true;
    }
    let radix_digits = |prefixes: [&str; 2], radix: u32| {
        prefixes.iter().find_map(|prefix| text.strip_prefix(prefix)).map(|digits| {
            !digits.is_empty() && digits.chars().all(|digit| digit.is_digit(radix))
        })
    };
    if let Some(valid) = radix_digits(["0x", "0X"], 16)
        .or_else(|| radix_digits(["0o", "0O"], 8))
        .or_else(|| radix_digits(["0b", "0B"], 2))
    {
        return valid;
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (&unsigned[..index], Some(&unsigned[index + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = match mantissa.find('.') {
        Some(index) => (&mantissa[..index], &mantissa[index + 1..]),
        None => (mantissa, ""),
    };
    let digits = |part: &str| part.chars().all(|digit| digit.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return false;
    }
    exponent.is_none_or(|exponent| {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !exponent.is_empty() && digits(exponent)
    })
}

/// tsc's `isValidBigIntString(s, false)`: `s + "n"` scans as one bigint
/// literal, a leading `-` allowed.
fn is_valid_bigint_string(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let radix_digits = |prefixes: [&str; 2], radix: u32| {
        prefixes.iter().find_map(|prefix| unsigned.strip_prefix(prefix)).map(|digits| {
            !digits.is_empty() && digits.chars().all(|digit| digit.is_digit(radix))
        })
    };
    radix_digits(["0x", "0X"], 16)
        .or_else(|| radix_digits(["0o", "0O"], 8))
        .or_else(|| radix_digits(["0b", "0B"], 2))
        .unwrap_or_else(|| !unsigned.is_empty() && unsigned.chars().all(|digit| digit.is_ascii_digit()))
}

/// tsc's `isPatternLiteralPlaceholderType`, without the intersection form.
fn is_pattern_literal_placeholder(ty: &Type) -> bool {
    matches!(ty, Type::Any | Type::String | Type::Number | Type::BigInt)
        || template_placeholder_intersection_members(ty).is_some()
        || string_mapping_parts(ty).is_some_and(|(_, inner)| is_pattern_literal_placeholder(inner))
        || template_literal_parts(ty)
            .is_some_and(|(_, types)| types.into_iter().all(is_pattern_literal_placeholder))
}

pub const STRING_MAPPING_REFERENCE_ID: &str = "\u{0}stringmapping";

/// The intrinsic string manipulation aliases (`type Uppercase<S> = intrinsic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringMappingKind {
    Uppercase,
    Lowercase,
    Capitalize,
    Uncapitalize,
}

impl StringMappingKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Uppercase" => Some(Self::Uppercase),
            "Lowercase" => Some(Self::Lowercase),
            "Capitalize" => Some(Self::Capitalize),
            "Uncapitalize" => Some(Self::Uncapitalize),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Uppercase => "Uppercase",
            Self::Lowercase => "Lowercase",
            Self::Capitalize => "Capitalize",
            Self::Uncapitalize => "Uncapitalize",
        }
    }

    /// tsc's `applyStringMapping`.
    fn apply(self, text: &str) -> String {
        let first_then_rest = |map: fn(&str) -> String| match text.chars().next() {
            Some(first) => {
                let split = first.len_utf8();
                format!("{}{}", map(&text[..split]), &text[split..])
            }
            None => String::new(),
        };
        match self {
            Self::Uppercase => text.to_uppercase(),
            Self::Lowercase => text.to_lowercase(),
            Self::Capitalize => first_then_rest(str::to_uppercase),
            Self::Uncapitalize => first_then_rest(str::to_lowercase),
        }
    }
}

/// The `(kind, mapped type)` of a generic string mapping type such as
/// `Uppercase<string>`.
pub fn string_mapping_parts(ty: &Type) -> Option<(StringMappingKind, &Type)> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    if &*reference.id != STRING_MAPPING_REFERENCE_ID {
        return None;
    }
    let [Type::StringLiteral(kind), inner] = &*reference.arguments else {
        return None;
    };
    Some((StringMappingKind::from_name(kind)?, inner))
}

/// tsc's `getStringMappingType`.
pub fn string_mapping_type(kind: StringMappingKind, ty: &Type) -> Type {
    match ty {
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| string_mapping_type(kind, member))
                .collect(),
        ),
        Type::Never => Type::Never,
        Type::StringLiteral(value) => Type::StringLiteral(kind.apply(value)),
        Type::Any | Type::String => generic_string_mapping(kind, ty),
        Type::Number | Type::BigInt => generic_string_mapping(
            kind,
            &template_literal_type(&[String::new(), String::new()], std::slice::from_ref(ty)),
        ),
        _ => {
            if let Some((texts, types)) = template_literal_parts(ty) {
                let (texts, types) = apply_template_string_mapping(kind, &texts, &types);
                return template_literal_type(&texts, &types);
            }
            if let Some((texts, types)) = crate::type_variable::template_literal_variable_parts(ty) {
                let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
                let types: Vec<&Type> = types.iter().collect();
                let (texts, types) = apply_template_string_mapping(kind, &texts, &types);
                return template_literal_type(&texts, &types);
            }
            if let Some((existing, _)) = crate::type_variable::string_mapping_variable_parts(ty)
                && existing == kind
            {
                return ty.clone();
            }
            if ty.is_type_variable() {
                return crate::type_variable::string_mapping_variable(kind, ty, format!("{}<{}>", kind.name(), ty.name()))
                    .unwrap_or_else(|| ty.clone());
            }
            match string_mapping_parts(ty) {
                Some((existing, _)) if existing == kind => ty.clone(),
                Some(_) => generic_string_mapping(kind, ty),
                None => ty.clone(),
            }
        }
    }
}

/// tsc's `applyTemplateStringMapping`.
fn apply_template_string_mapping(
    kind: StringMappingKind,
    texts: &[&str],
    types: &[&Type],
) -> (Vec<String>, Vec<Type>) {
    let mut new_texts: Vec<String> = texts.iter().map(|text| (*text).to_string()).collect();
    let mut new_types: Vec<Type> = types.iter().map(|ty| (*ty).clone()).collect();
    match kind {
        StringMappingKind::Uppercase | StringMappingKind::Lowercase => {
            new_texts = new_texts.iter().map(|text| kind.apply(text)).collect();
            new_types = new_types.iter().map(|ty| string_mapping_type(kind, ty)).collect();
        }
        StringMappingKind::Capitalize | StringMappingKind::Uncapitalize => {
            if !new_texts[0].is_empty() {
                new_texts[0] = kind.apply(&new_texts[0]);
            } else if let Some(first) = new_types.first_mut() {
                *first = string_mapping_type(kind, first);
            }
        }
    }
    (new_texts, new_types)
}

fn generic_string_mapping(kind: StringMappingKind, inner: &Type) -> Type {
    Type::Reference(TypeReference::new(
        STRING_MAPPING_REFERENCE_ID,
        format!("{}<{}>", kind.name(), inner.name()),
        vec![Type::StringLiteral(kind.name().to_string()), inner.clone()],
        Arc::new(ResolvesToString),
    ))
}

/// tsc's `isMemberOfStringMapping`.
pub fn is_member_of_string_mapping(source: &Type, target: &Type) -> bool {
    if matches!(target, Type::Any) {
        return true;
    }
    if *target == Type::String || owned_template_parts(target).is_some() {
        return is_assignable_to(source, target);
    }
    if owned_string_mapping_parts(target).is_some() {
        // Applying the target's own mappings must leave the source unchanged,
        // and the source must belong to what is being mapped.
        let (mapped, inner) = apply_target_string_mapping_to_source(source, target);
        return mapped == *source && is_member_of_string_mapping(source, &inner);
    }
    false
}

/// tsc's `applyTargetStringMappingToSource`.
fn apply_target_string_mapping_to_source(source: &Type, target: &Type) -> (Type, Type) {
    let Some((kind, inner)) = owned_string_mapping_parts(target) else {
        return (source.clone(), target.clone());
    };
    let (source, inner) = if owned_string_mapping_parts(&inner).is_some() {
        apply_target_string_mapping_to_source(source, &inner)
    } else {
        (source.clone(), inner)
    };
    (string_mapping_type(kind, &source), inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(texts: &[&str], types: &[Type]) -> Type {
        let texts: Vec<String> = texts.iter().map(|text| (*text).to_string()).collect();
        template_literal_type(&texts, types)
    }

    fn matched(source: &Type, target: &Type) -> bool {
        let (texts, types) = template_literal_parts(target).expect("target is a template");
        is_type_matched_by_template_literal(source, &texts, &types)
    }

    #[test]
    fn literal_placeholders_fold_into_a_string_literal() {
        assert_eq!(
            template(&["/", "/", ""], &[Type::StringLiteral("a".into()), Type::BooleanLiteral(true)]),
            Type::StringLiteral("/a/true".into())
        );
    }

    #[test]
    fn a_union_placeholder_distributes() {
        let ty = template(
            &["", "://", ""],
            &[
                union_type(vec![Type::StringLiteral("http".into()), Type::StringLiteral("ftp".into())]),
                Type::String,
            ],
        );
        assert_eq!(ty.name(), "`http://${string}` | `ftp://${string}`");
    }

    #[test]
    fn a_string_literal_matches_by_its_fixed_texts() {
        let slash = template(&["/", ""], &[Type::String]);
        assert!(matched(&Type::StringLiteral("/bin".into()), &slash));
        assert!(!matched(&Type::StringLiteral("no slash".into()), &slash));
    }

    #[test]
    fn a_number_placeholder_takes_numeric_strings_only() {
        let px = template(&["", "px"], &[Type::Number]);
        assert!(matched(&Type::StringLiteral("12px".into()), &px));
        assert!(matched(&Type::StringLiteral("1e3px".into()), &px));
        assert!(!matched(&Type::StringLiteral("abpx".into()), &px));
        assert!(!matched(&Type::StringLiteral("px".into()), &px));
    }

    #[test]
    fn a_template_source_matches_across_its_own_placeholders() {
        let target = template(&["<", ".", ">"], &[Type::String, Type::String]);
        let source = template(&["<<", ">.<", "-", ">>"], &[Type::String, Type::Number, Type::Number]);
        assert!(matched(&source, &target));
    }

    #[test]
    fn a_string_mapping_folds_literals_and_keeps_patterns() {
        assert_eq!(
            string_mapping_type(StringMappingKind::Uppercase, &Type::StringLiteral("abc".into())),
            Type::StringLiteral("ABC".into())
        );
        let upper = string_mapping_type(StringMappingKind::Uppercase, &Type::String);
        assert_eq!(upper.name(), "Uppercase<string>");
        assert!(is_member_of_string_mapping(&Type::StringLiteral("ABC".into()), &upper));
        assert!(!is_member_of_string_mapping(&Type::StringLiteral("aBC".into()), &upper));
        let shout = template(&["on", ""], &[upper]);
        assert_eq!(shout.name(), "`on${Uppercase<string>}`");
        assert!(matched(&Type::StringLiteral("onCLICK".into()), &shout));
        assert!(!matched(&Type::StringLiteral("onClick".into()), &shout));
    }

    #[test]
    fn a_template_is_a_member_of_the_mapping_it_already_satisfies() {
        let capitalized = string_mapping_type(StringMappingKind::Capitalize, &Type::String);
        let a_template = template(&["A", ""], &[Type::String]);
        assert_eq!(
            string_mapping_type(StringMappingKind::Capitalize, &a_template),
            a_template
        );
        assert!(is_member_of_string_mapping(&a_template, &capitalized));
        assert!(is_assignable_to(&a_template, &capitalized));
        let lower_template = template(&["a", ""], &[Type::String]);
        assert!(!is_assignable_to(&lower_template, &capitalized));
    }

    #[test]
    fn an_all_string_template_is_string() {
        assert_eq!(template(&["", ""], &[Type::String]), Type::String);
    }
}
