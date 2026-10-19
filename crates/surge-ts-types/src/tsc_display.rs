//! Types printed as TypeScript's `typeToString` prints them, for the compiler
//! API. The checker's own display matches tsgo's diagnostic text; inside
//! [`with_tsc_display`] a few renderings follow `typeToString` instead: the
//! degradation sentinel is the error type tsc prints as `any`, and a
//! reference's arguments always re-render in this mode. Diagnostics never
//! render in this mode. [`with_tsc_truncation`] further counts the display
//! as the node builder does and elides what a long one nests.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::fx::FxHashMap;
use crate::{Type, TypeReference};

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// Renders in this mode by payload address. The shared name memos hold the
    /// checker's renderings, and without a memo a type graph that shares
    /// subtrees renders exponentially.
    static RENDERED: RefCell<FxHashMap<(usize, Budget), Rendered>> = RefCell::new(FxHashMap::default());
    /// The node builder's `approximateLength` while a truncating display runs.
    static APPROXIMATE_LENGTH: Cell<Option<usize>> = const { Cell::new(None) };
    static RANKS: RefCell<Option<Arc<CreationRanks>>> = const { RefCell::new(None) };
}

/// The order typescript@6 creates literal and named types in, which its type
/// ids, and so its union order, follow. Its checker creates `""`, `0` and the
/// `typeof` results up front; a program's own literals and names are taken
/// in the order they are written, which is the order a walk over the program
/// asks for their types. A literal expression's type is a fresh literal type
/// with an id of its own, so `""` or `0` a program writes only as an
/// expression is ranked where it is first written rather than up front.
pub struct CreationRanks {
    ranks: FxHashMap<CreationKey, u32>,
    /// The up-front types, and whether a fresh use can stand for one.
    seeds: FxHashMap<CreationKey, (u32, bool)>,
    /// A seed's first expression use, and whether a type writes it too.
    seed_uses: FxHashMap<CreationKey, (Option<u32>, bool)>,
    next: u32,
}

#[derive(PartialEq, Eq, Hash)]
enum CreationKey {
    String(Box<str>),
    Number(u64),
    Name(Box<str>),
}

impl Default for CreationRanks {
    fn default() -> Self {
        let mut seeds = FxHashMap::default();
        seeds.insert(CreationKey::String("".into()), (0, false));
        seeds.insert(CreationKey::Number(number_key(0.0)), (1, false));
        for (index, name) in ["string", "number", "bigint", "boolean", "symbol", "undefined", "object", "function"].into_iter().enumerate() {
            seeds.insert(CreationKey::String(name.into()), (2 + index as u32, true));
        }
        CreationRanks { ranks: FxHashMap::default(), next: seeds.len() as u32, seeds, seed_uses: FxHashMap::default() }
    }
}

impl CreationRanks {
    fn record(&mut self, key: CreationKey, in_type: bool) {
        match self.seeds.get(&key) {
            Some((_, true)) => {}
            Some((_, false)) => {
                let use_ = self.seed_uses.entry(key).or_insert((None, false));
                if in_type {
                    use_.1 = true;
                } else if use_.0.is_none() {
                    self.next += 1;
                    use_.0 = Some(self.next);
                }
            }
            None => {
                if !self.ranks.contains_key(&key) {
                    self.next += 1;
                    self.ranks.insert(key, self.next);
                }
            }
        }
    }

    /// A string literal, written in a type (`in_type`) or as an expression.
    pub fn record_string(&mut self, value: &str, in_type: bool) {
        self.record(CreationKey::String(value.into()), in_type);
    }

    pub fn record_number(&mut self, value: f64, in_type: bool) {
        self.record(CreationKey::Number(number_key(value)), in_type);
    }

    pub fn record_name(&mut self, name: &str) {
        self.record(CreationKey::Name(name.into()), true);
    }

    fn rank(&self, ty: &Type) -> Option<u32> {
        let named = |display: &str| {
            let head = &display[..display.find('<').unwrap_or(display.len())];
            let name = head.rsplit('.').next().unwrap_or(head);
            self.ranks.get(&CreationKey::Name(name.into())).copied()
        };
        let literal = |key: CreationKey| match self.seeds.get(&key) {
            Some(&(seed, false)) => match self.seed_uses.get(&key) {
                Some(&(Some(fresh), false)) => Some(fresh),
                _ => Some(seed),
            },
            Some(&(seed, true)) => Some(seed),
            None => self.ranks.get(&key).copied(),
        };
        match ty {
            Type::StringLiteral(value) => literal(CreationKey::String(value.as_str().into())),
            Type::NumberLiteral(number) => literal(CreationKey::Number(number_key(number.value.parse().ok()?))),
            Type::Reference(reference)
                if reference.enum_owner.is_none() && !reference.id.starts_with('\u{0}') && !reference.is_unique_symbol() =>
            {
                named(reference.alias_display.as_deref().unwrap_or(&reference.display))
            }
            Type::Union(union) => named(union.alias_name()?),
            Type::Object(object) => named(object.alias_name.as_deref()?),
            _ => None,
        }
    }
}

fn number_key(value: f64) -> u64 {
    if value == 0.0 { 0f64.to_bits() } else { value.to_bits() }
}

/// Runs `render` with `ranks` ordering the unions it prints.
pub fn with_creation_ranks<R>(ranks: Arc<CreationRanks>, render: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<CreationRanks>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            RANKS.with(|ranks| *ranks.borrow_mut() = previous);
        }
    }
    let _restore = Restore(RANKS.with(|current| current.borrow_mut().replace(ranks)));
    render()
}

/// Orders union members after the intrinsics by when tsc created their
/// types, when every one of them has a known creation rank.
pub(crate) fn order_by_creation(members: &mut [&Type]) {
    RANKS.with(|ranks| {
        let ranks = ranks.borrow();
        let Some(ranks) = ranks.as_deref() else { return };
        let start = members.iter().position(|member| intrinsic_rank(member) == 10).unwrap_or(members.len());
        let tail = &mut members[start..];
        let rank = |member: &&Type| match member {
            Type::Null | Type::Undefined => Some(u32::MAX),
            member => ranks.rank(member),
        };
        let Some(keys) = tail.iter().map(rank).collect::<Option<Vec<u32>>>() else { return };
        let mut keyed: Vec<(u32, &Type)> = keys.into_iter().zip(tail.iter().copied()).collect();
        keyed.sort_by_key(|(key, _)| *key);
        for (slot, (_, member)) in tail.iter_mut().zip(keyed) {
            *slot = member;
        }
    });
}

/// Where a render stood against the truncation length, which decides what
/// it elides.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Budget {
    Uncounted,
    /// Started and ended within the length: it elided nothing.
    Within,
    /// Started past the length: every check elided.
    Past,
}

struct Rendered {
    name: Arc<str>,
    /// What the render added to the approximate length.
    added: usize,
}

/// typescript@6's `defaultMaximumTruncationLength`.
const MAXIMUM_TRUNCATION_LENGTH: usize = 160;

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

/// `typeToString`: runs `render` in this mode, eliding what a long display
/// nests as the node builder does once its approximate length passes
/// `defaultMaximumTruncationLength`.
pub fn with_tsc_truncation<R>(render: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            APPROXIMATE_LENGTH.with(|length| length.set(self.0));
        }
    }
    let _restore = Restore(APPROXIMATE_LENGTH.with(|length| length.replace(Some(0))));
    with_tsc_display(render)
}

/// Runs `work` outside the count: resolving a reference mid-display can run
/// the checker, and whatever it renders is no part of the display.
pub(crate) fn uncounted<R>(work: impl FnOnce() -> R) -> R {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            APPROXIMATE_LENGTH.with(|length| length.set(Some(self.0)));
        }
    }
    let Some(length) = APPROXIMATE_LENGTH.with(|length| length.take()) else {
        return work();
    };
    let _restore = Restore(length);
    work()
}

/// Adds to the approximate length of the display being built, as the node
/// builder does for each node it creates.
pub(crate) fn add_length(added: usize) {
    APPROXIMATE_LENGTH.with(|length| {
        if let Some(current) = length.get() {
            length.set(Some(current + added));
        }
    });
}

/// `name`, a display surge keeps as text, counted as the nodes it spells.
pub(crate) fn counted(name: String) -> String {
    if approximate_length().is_some() {
        add_length(text_length(&name));
    }
    name
}

/// A name written with its type arguments (`Promise<T | U>`), the arguments
/// elided as `mapToTypeNodes` elides a list once the display is long.
pub(crate) fn counted_written_name(name: &str) -> String {
    if active()
        && truncating()
        && name.ends_with('>')
        && let Some(open) = name.find('<')
    {
        let head = &name[..open];
        add_length(symbol_name_length(head.rsplit('.').next().unwrap_or(head)));
        return format!("{head}<...>");
    }
    counted(name.to_string())
}

pub(crate) fn counted_keyword(keyword: &str) -> String {
    add_length(keyword.len());
    keyword.to_string()
}

/// A written type parameter list (`S extends T, U = never`): the node
/// builder counts each constraint and default, not the names.
pub(crate) fn count_type_parameter_head(head: &str) {
    if approximate_length().is_none() {
        return;
    }
    for parameter in split_top_level(head) {
        let rest = parameter.split_once(" extends ").map_or("", |(_, rest)| rest);
        let (constraint, default) = match rest.split_once(" = ") {
            Some(split) => split,
            None => (rest, parameter.split_once(" = ").map_or("", |(_, default)| default)),
        };
        add_length(text_length(constraint) + text_length(default));
    }
}

/// What `symbolToTypeNode` counts for a name it looks up: the first name of
/// its symbol chain is counted twice, `createAccessFromSymbolChain` adding it
/// both for the initial entity name and as any name.
pub(crate) fn symbol_name_length(name: &str) -> usize {
    2 * (name.len() + 1)
}

/// What the node builder would count for type text: a keyword its length,
/// `unknown` nothing, a member or parameter name one or three more, any other
/// name as a symbol it looks up, a literal with its quotes, two per type
/// literal, union or list operand, three per signature.
fn text_length(text: &str) -> usize {
    const KEYWORDS: [&str; 16] = [
        "string", "number", "boolean", "bigint", "symbol", "object", "any", "void", "undefined", "null", "never",
        "true", "false", "this", "keyof", "readonly",
    ];
    let mut length = 0;
    let mut brackets = Vec::new();
    let mut characters = text.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        match character {
            '(' | '[' => brackets.push(character),
            ')' | ']' | '}' => {
                brackets.pop();
            }
            '"' | '\'' | '`' => {
                let mut end = text.len();
                for (index, next) in characters.by_ref() {
                    if next == character {
                        end = index;
                        break;
                    }
                }
                length += end - start + 1;
            }
            '{' => {
                brackets.push(character);
                length += 2;
            }
            '<' | '|' | '&' | ',' => length += 2,
            '=' if characters.peek().is_some_and(|(_, next)| *next == '>') => length += 3,
            _ if character.is_alphanumeric() || character == '_' || character == '$' => {
                let mut end = start + character.len_utf8();
                while let Some(&(index, next)) = characters.peek() {
                    if !(next.is_alphanumeric() || next == '_' || next == '$' || next == '.') {
                        break;
                    }
                    end = index + next.len_utf8();
                    characters.next();
                }
                let word = &text[start..end];
                let declares = text[end..].trim_start_matches('?').starts_with(':');
                length += match word {
                    _ if declares && brackets.last() == Some(&'(') => word.len() + 3,
                    _ if declares => word.len() + 1,
                    "unknown" => 0,
                    _ if KEYWORDS.contains(&word) || word.starts_with(|first: char| first.is_ascii_digit()) => word.len(),
                    _ => symbol_name_length(word),
                };
            }
            _ => {}
        }
    }
    length
}

fn split_top_level(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut previous = ' ';
    for (index, character) in list.char_indices() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if previous == '=' => {}
            '>' | ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(list[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
        previous = character;
    }
    parts.push(list[start..].trim());
    parts
}

/// `T | undefined`, the optional member's type tsc prints as a union.
pub(crate) fn with_undefined(ty: &Type) -> String {
    // `undefined` joins a union as one more member, which a long display
    // elides with the rest.
    if approximate_length().is_some()
        && let Type::Union(union) = ty
        && union.alias_name().is_none()
    {
        return crate::union_type(vec![ty.clone(), Type::Undefined]).name();
    }
    add_length(2);
    let name = constituent_name(ty);
    add_length(2 + "undefined".len());
    format!("{name} | undefined")
}

/// `checkTruncationLength`: the display has grown past where nested
/// structure is still spelled out.
pub(crate) fn truncating() -> bool {
    APPROXIMATE_LENGTH.with(|length| length.get().is_some_and(|length| length > MAXIMUM_TRUNCATION_LENGTH))
}

fn approximate_length() -> Option<usize> {
    APPROXIMATE_LENGTH.with(Cell::get)
}

/// `mapToTypeNodes`: `render` over `types`, the middle elided as `... N more
/// ...` once the display is long. A bare list (union or intersection
/// operands) keeps its first and last type; a list of type arguments that is
/// already long prints as `...`.
pub(crate) fn render_type_list<T>(types: &[T], bare: bool, render: impl Fn(&T) -> String) -> Vec<String> {
    if approximate_length().is_none() {
        return types.iter().map(render).collect();
    }
    if types.is_empty() {
        return Vec::new();
    }
    if truncating() {
        if !bare {
            return vec!["...".to_string()];
        }
        if types.len() > 2 {
            return vec![render(&types[0]), format!("... {} more ...", types.len() - 2), render(&types[types.len() - 1])];
        }
    }
    let mut rendered = Vec::with_capacity(types.len());
    for (index, ty) in types.iter().enumerate() {
        let position = index + 1;
        if truncating() && position + 2 < types.len() - 1 {
            rendered.push(format!("... {} more ...", types.len() - position));
            rendered.push(render(&types[types.len() - 1]));
            break;
        }
        add_length(2);
        rendered.push(render(ty));
    }
    rendered
}

/// `render()` for the payload at `address`, rendered once per display. A
/// counted render is reused where it elides the same: within the length
/// throughout, or past it from the start.
pub(crate) fn memoized(address: usize, render: impl FnOnce() -> String) -> String {
    memoized_optional(address, || Some(render())).unwrap_or_default()
}

/// Like [`memoized`], for a rendering that may decline.
pub(crate) fn memoized_optional(address: usize, render: impl FnOnce() -> Option<String>) -> Option<String> {
    if crate::cycle::canonical() {
        return render();
    }
    let start = approximate_length();
    let budget = match start {
        None => Budget::Uncounted,
        Some(start) if start > MAXIMUM_TRUNCATION_LENGTH => Budget::Past,
        Some(_) => Budget::Within,
    };
    let cached = RENDERED.with(|rendered| {
        let rendered = rendered.borrow();
        let entry = rendered.get(&(address, budget))?;
        let fits = match start {
            Some(start) if budget == Budget::Within => start + entry.added <= MAXIMUM_TRUNCATION_LENGTH,
            _ => true,
        };
        fits.then(|| (entry.name.clone(), entry.added))
    });
    if let Some((name, added)) = cached {
        add_length(added);
        return Some(name.to_string());
    }
    let markers_before = crate::cycle::markers_rendered();
    let name = render()?;
    let added = start.zip(approximate_length()).map_or(0, |(start, end)| end - start);
    // A cycle marker prints by what encloses it.
    let reusable = crate::cycle::markers_rendered() == markers_before
        && match start {
            Some(start) if budget == Budget::Within => start + added <= MAXIMUM_TRUNCATION_LENGTH,
            _ => true,
        };
    if reusable {
        RENDERED.with(|rendered| {
            rendered.borrow_mut().insert((address, budget), Rendered { name: Arc::from(name.as_str()), added })
        });
    }
    Some(name)
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
    let name = ty.name();
    // A signature type an alias names (`FnAlias<T>`) prints as the alias.
    let bare_signature = is_signature_type(ty) && (name.starts_with('(') || name.starts_with('<') || name.starts_with("new "));
    if bare_signature || is_unnamed_intersection(ty) { format!("({name})") } else { name }
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
        Some(display) => has_top_level(display, '&'),
    }
}

/// An alias name as `typeToString` prints it: by the symbol, not the
/// qualified name it was written with (`ReactNode` for `React.ReactNode`).
pub(crate) fn bare_alias_name(alias: &str) -> &str {
    let head = &alias[..alias.find('<').unwrap_or(alias.len())];
    let is_path = head.split('.').all(|part| {
        let mut characters = part.chars();
        characters.next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
            && characters.all(|character| character.is_alphanumeric() || character == '_' || character == '$')
    });
    match head.rfind('.') {
        Some(dot) if is_path => &alias[dot + 1..],
        _ => alias,
    }
}

/// Whether `operator` (`&` or `|`) joins operands at the top level of
/// `display`.
pub(crate) fn has_top_level(display: &str, operator: char) -> bool {
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
            (None, found) if found == operator && depth == 0 => return true,
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
        || matches!(ty, Type::Reference(reference) if reference.is_readonly_array() || reference.written_tuple().is_some())
}

/// The tuple a rest parameter's type is, through the `readonly` and
/// written-tuple wrappers (tsc's `isTupleType` holds for both).
pub(crate) fn rest_tuple_shape(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Tuple(_) | Type::OpenTuple(_) => Some(ty),
        Type::Reference(reference) if reference.is_readonly_array() || reference.written_tuple().is_some() => {
            rest_tuple_shape(reference.arguments.first()?)
        }
        _ => None,
    }
}

/// A reference as `typeToString` prints it with no enclosing declaration: by
/// its type's own name (`Element` for `JSX.Element`), its arguments rendered
/// in this mode. An enum member keeps its enum (`Color.Red`), and a display
/// that is no name with arguments stays as it is.
pub(crate) fn reference_name(reference: &TypeReference) -> String {
    // `typeToString` allows the unique symbol type (`AllowUniqueESSymbolType`)
    // rather than naming it by its declaration.
    if reference.is_unique_symbol() {
        return counted_keyword("unique symbol");
    }
    // A deferred intersection's display was written from its operands before
    // instantiation (`Expectation<T> & …`) and substitution leaves it as it
    // was; its arguments are the operands as instantiated.
    if reference.id.starts_with("\u{0}intersection") && !reference.arguments.is_empty() {
        return render_type_list(&reference.arguments, true, constituent_name).join(" & ");
    }
    // The mapping's kind rides along as an argument the type never had.
    if &*reference.id == crate::template_literal::STRING_MAPPING_REFERENCE_ID
        && let [Type::StringLiteral(kind), inner] = &*reference.arguments
    {
        add_length(symbol_name_length(kind) + 2);
        return format!("{kind}<{}>", inner.name());
    }
    // A lazily resolved annotation keeps the text it was written with; one
    // that is not a type's name (`{ retries: number }`, `A | B`) prints as
    // the type it resolves to, as tsc prints every type it has. A type query
    // stays written: tsc names a module, function or class by it.
    if is_lazy_annotation(reference)
        && !is_written_type_name(&reference.display)
        && !reference.display.starts_with("typeof ")
        && !reference.display.starts_with("import(")
    {
        return reference.resolve().name();
    }
    bare_reference_name(reference, true)
}

fn is_lazy_annotation(reference: &TypeReference) -> bool {
    ["\u{0}value-annotation\u{0}", "\u{0}signature-annotation\u{0}", "\u{0}member-annotation"]
        .iter()
        .any(|marker| reference.id.contains(marker))
}

/// A (qualified) name with type arguments or none: `Box<T>`, `ns.A`.
fn is_written_type_name(display: &str) -> bool {
    let head = &display[..display.find('<').unwrap_or(display.len())];
    (head.len() == display.len() || display.ends_with('>'))
        && !head.is_empty()
        && head.split('.').all(|part| {
            let mut characters = part.chars();
            characters.next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
                && characters.all(|character| character.is_alphanumeric() || character == '_' || character == '$')
        })
}

/// A reference as a diagnostic prints it: tsc names the symbol, not the
/// qualified name the annotation was written with (`A<number>` for
/// `Generics.A<number>`). `name()` runs on hot paths, so only a qualified head
/// leaves the stored display, and its arguments re-render only when one of
/// them is qualified.
pub(crate) fn diagnostic_reference_name(reference: &TypeReference) -> String {
    // An alias of the `object` keyword is the intrinsic itself, which carries
    // no alias.
    // Forcing every argument-less reference here expanded interfaces that
    // nothing else reads, only to print their names.
    if reference.arguments.is_empty()
        && reference.enum_owner.is_none()
        && !reference.never_object_keyword()
        && matches!(&*reference.resolve_arc(), Type::Object(object)
            if object.non_primitive
                && object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !object.is_intersection)
    {
        return counted_keyword("object");
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
        return counted(display.to_string());
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
            return counted(name.to_string());
        }
        return counted(display.to_string());
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
        return counted(display.to_string());
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
        let rendered = render_type_list(&reference.arguments, false, Type::name);
        add_length(symbol_name_length(bare));
        return format!("{bare}<{}>", rendered.join(", "));
    }
    // Written arguments are a list `mapToTypeNodes` elides once the display
    // is long, as it does the rendered ones; a deferred type is the written
    // node tsc reuses whole.
    if written_count.is_some() && truncating() && !reference.id.starts_with("\u{0}deferred\u{0}") {
        add_length(symbol_name_length(bare));
        return format!("{bare}<...>");
    }
    counted(format!("{bare}{arguments}"))
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
