//! Types printed as TypeScript's `typeToString` prints them, for the compiler
//! API. The checker's own display matches tsgo's diagnostic text; inside
//! [`with_tsc_display`] a few renderings follow `typeToString` instead: the
//! degradation sentinel is the error type tsc prints as `any`, and a
//! reference's arguments always re-render in this mode. Diagnostics never
//! render in this mode.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::fx::FxHashMap;
use crate::{Type, TypeReference};

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Renders in this mode by payload address. The shared name memos hold the
    /// checker's renderings, and without a memo a type graph that shares
    /// subtrees renders exponentially.
    static RENDERED: RefCell<FxHashMap<usize, Arc<str>>> = RefCell::new(FxHashMap::default());
}

/// Runs `render` with types printed as `typeToString` prints them.
pub fn with_tsc_display<R>(render: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(self.0));
            if !self.0 {
                RENDERED.with(|rendered| rendered.borrow_mut().clear());
            }
        }
    }
    let _restore = Restore(ACTIVE.with(|active| active.replace(true)));
    render()
}

pub(crate) fn active() -> bool {
    ACTIVE.with(Cell::get)
}

/// `render()` for the payload at `address`, rendered once per display.
pub(crate) fn memoized(address: usize, render: impl FnOnce() -> String) -> String {
    if let Some(name) = RENDERED.with(|rendered| rendered.borrow().get(&address).cloned()) {
        return name.to_string();
    }
    let name = render();
    RENDERED.with(|rendered| rendered.borrow_mut().insert(address, Arc::from(name.as_str())));
    name
}

/// A union member's position by typescript@6's intrinsic type ids (`string`,
/// `number`, `bigint`, `false`, `true`, `symbol`, `void`); any other type
/// keeps its place after them.
pub(crate) fn intrinsic_rank(ty: &Type) -> u8 {
    match ty {
        Type::Any | Type::ErrorType | Type::Unknown => 0,
        Type::GenuineUnknown => 1,
        Type::String => 2,
        Type::Number => 3,
        Type::BigInt => 4,
        Type::BooleanLiteral(false) => 5,
        Type::BooleanLiteral(true) => 6,
        Type::Boolean => 5,
        Type::Symbol => 7,
        Type::Void => 8,
        Type::Never => 9,
        _ => 10,
    }
}

/// A union or optional member's constituent: tsc parenthesizes a function or
/// constructor type there (`(() => void) | undefined`).
pub(crate) fn constituent_name(ty: &Type) -> String {
    if is_signature_type(ty) || is_unnamed_intersection(ty) { format!("({})", ty.name()) } else { ty.name() }
}

/// An intersection printed as its operands, which a union parenthesizes. The
/// display an annotation leaves on it is its written operands, not an alias.
fn is_unnamed_intersection(ty: &Type) -> bool {
    let object = match ty {
        Type::Object(object) => object,
        Type::Reference(reference) => {
            return reference.id.starts_with("\u{0}intersection")
                && reference.alias_display.is_none()
                && reference.arguments.len() > 1;
        }
        _ => return false,
    };
    if !object.is_intersection {
        return false;
    }
    match object.alias_name.as_deref() {
        None => object.intersection_operands.as_deref().is_some_and(|operands| operands.len() > 1),
        Some(display) => has_top_level_ampersand(display),
    }
}

fn has_top_level_ampersand(display: &str) -> bool {
    let mut depth = 0usize;
    let mut quote = None;
    let mut previous = ' ';
    for character in display.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(character),
            (None, '(' | '[' | '{' | '<') => depth += 1,
            (None, '>') if previous == '=' => {}
            (None, ')' | ']' | '}' | '>') => depth = depth.saturating_sub(1),
            (None, '&') if depth == 0 => return true,
            _ => {}
        }
        previous = character;
    }
    false
}

/// A type that prints as one bare signature, `(x: T) => U` or `new () => T`.
fn is_signature_type(ty: &Type) -> bool {
    match ty {
        Type::Function(function) => function.alias_name().is_none() && function.overloads().is_none(),
        Type::Object(object) => {
            object.alias_name.is_none()
                && object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_some() != object.construct_signature().is_some()
        }
        _ => false,
    }
}

/// A parameter type a rest parameter can have.
pub(crate) fn is_rest_parameter_type(ty: &Type) -> bool {
    matches!(ty, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::TypeParameter(_) | Type::Any)
        || matches!(ty, Type::Reference(reference) if reference.is_readonly_array())
}

/// A reference as `typeToString` prints it with no enclosing declaration: by
/// its type's own name (`Element` for `JSX.Element`), its arguments rendered
/// in this mode. An enum member keeps its enum (`Color.Red`), and a display
/// that is no name with arguments stays as it is.
pub(crate) fn reference_name(reference: &TypeReference) -> String {
    // `typeToString` allows the unique symbol type (`AllowUniqueESSymbolType`)
    // rather than naming it by its declaration.
    if reference.is_unique_symbol() {
        return "unique symbol".to_string();
    }
    bare_reference_name(reference, true)
}

/// A reference as a diagnostic prints it: tsc names the symbol, not the
/// qualified name the annotation was written with (`A<number>` for
/// `Generics.A<number>`). `name()` runs on hot paths, so only a qualified head
/// leaves the stored display, and its arguments re-render only when one of
/// them is qualified.
pub(crate) fn diagnostic_reference_name(reference: &TypeReference) -> String {
    // An alias of the `object` keyword is the intrinsic itself, which carries
    // no alias.
    if reference.arguments.is_empty()
        && reference.enum_owner.is_none()
        && matches!(&*reference.resolve_arc(), Type::Object(object)
            if object.non_primitive
                && object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !object.is_intersection)
    {
        return "object".to_string();
    }
    let display = reference.display.as_ref();
    // A reference written without its defaulted arguments prints them all.
    // A synthetic reference (a deferred intersection, a string mapping) keeps
    // arguments of its own.
    if !reference.id.starts_with('\u{0}')
        && let Some(open) = display.find('<')
        && display.ends_with('>')
        && top_level_count(&display[open + 1..display.len() - 1]) < reference.arguments.len()
    {
        return bare_reference_name(reference, true);
    }
    if !display[..display.find('<').unwrap_or(display.len())].contains('.') {
        return display.to_string();
    }
    bare_reference_name(reference, false)
}

fn bare_reference_name(reference: &TypeReference, always_render_arguments: bool) -> String {
    let display = reference.display.as_ref();
    if reference.enum_owner.is_some() || reference.enum_base.is_some() {
        // `typeToString` names an exported enum bare; only a diagnostic
        // qualifies it with its module.
        if active()
            && let Some(rest) = display.strip_prefix("import(\"")
            && let Some((_, name)) = rest.split_once("\").")
        {
            return name.to_string();
        }
        return display.to_string();
    }
    let head_end = display.find('<').unwrap_or(display.len());
    let head = &display[..head_end];
    let is_name = !head.is_empty()
        && head.split('.').all(|part| {
            let mut characters = part.chars();
            characters.next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
                && characters.all(|character| character.is_alphanumeric() || character == '_' || character == '$')
        });
    if !is_name {
        return display.to_string();
    }
    let bare = head.rsplit('.').next().unwrap_or(head);
    let arguments = &display[head_end..];
    let written_count = (arguments.len() >= 2 && arguments.ends_with('>'))
        .then(|| top_level_count(&arguments[1..arguments.len() - 1]));
    let renders = match written_count {
        Some(count) => {
            (always_render_arguments && count <= reference.arguments.len())
                || (arguments.contains('.') && count == reference.arguments.len())
        }
        None => false,
    };
    if renders && !reference.arguments.is_empty() {
        let rendered: Vec<String> = reference.arguments.iter().map(Type::name).collect();
        return format!("{bare}<{}>", rendered.join(", "));
    }
    format!("{bare}{arguments}")
}

/// A reference written as the lib's array interface, `Array<T>` or
/// `ReadonlyArray<T>`.
pub(crate) fn is_written_array_reference(reference: &TypeReference) -> bool {
    !reference.is_readonly_array()
        && (reference.display.starts_with("Array<") || reference.display.starts_with("ReadonlyArray<"))
}

/// How many comma-separated items `list` holds outside brackets.
fn top_level_count(list: &str) -> usize {
    let mut depth = 0i32;
    let mut count = 1;
    let mut previous = ' ';
    for character in list.chars() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            // The `>` of an arrow closes nothing.
            '>' if previous == '=' => {}
            '>' | ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => count += 1,
            _ => {}
        }
        previous = character;
    }
    count
}
