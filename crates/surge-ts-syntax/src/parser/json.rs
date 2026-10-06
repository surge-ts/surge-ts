//! JSON module lowering.
//!
//! A `.json` import resolves to the JSON value's *type*, not to a program: tsc
//! types `{"version": "1.2.3"}` as `{ version: string }`, widening every scalar
//! and unioning an array's elements. Lowering straight to a [`ParsedType`]
//! rather than to statements keeps the whole feature out of the expression
//! checker — nothing in a JSON file is code.
//!
//! tsc reads a JSON module with the same lenient reader it uses for
//! `tsconfig.json`, so comments and trailing commas are accepted here too;
//! single-quoted strings are not (tsc reports `TS1327` for those).

use std::sync::Arc;

use crate::{ParsedObjectType, ParsedObjectTypeProperty, ParsedType};

pub fn is_json_file_name(file_name: &str) -> bool {
    file_name
        .get(file_name.len().saturating_sub(5)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".json"))
}

/// The type of the value in `source_text`, or `None` when it is not valid JSON.
/// A malformed file is left unmodelled rather than degraded so the caller can
/// keep reporting the import as unresolved.
pub fn parse_json_module_type(source_text: &str) -> Option<ParsedType> {
    let mut parser = JsonParser {
        bytes: source_text.as_bytes(),
        position: 0,
        depth: 0,
    };
    parser.skip_whitespace();
    // A file with no value is a JSON source file without statements, which tsc
    // types as the empty object (`getTypeOfVariableOrParameterOrPropertyWorker`).
    if parser.at_end() {
        return Some(ParsedType::Object(Arc::new(ParsedObjectType {
            properties: Default::default(),
            string_index_type: None,
            number_index_type: None,
            string_index_readonly: false,
            number_index_readonly: false,
            call_signature: None,
            call_signature_overloads: Vec::new(),
            construct_signature: None,
            construct_signature_overloads: Vec::new(),
            non_primitive: false,
            display_name: None,
            abstract_construct_signature: false,
            keyed_indexes: None,
            number_index_span: None,
        })));
    }
    let ty = parser.value()?;
    parser.skip_whitespace();
    parser.at_end().then_some(ty)
}

/// Bounds the recursion a hostile or generated file can force. Deeper values
/// are skipped iteratively and flatten to the degradation sentinel rather than
/// being rejected — rejecting would make the whole import read as unresolved,
/// which is a worse answer than "not modelled past here".
const MAX_DEPTH: u32 = 64;

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
    depth: u32,
}

impl JsonParser<'_> {
    fn at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    /// Whitespace plus `//` and `/* */` comments, which tsc's JSON reader
    /// accepts.
    fn skip_whitespace(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.position += 1;
            }
            if self.bytes[self.position..].starts_with(b"//") {
                self.position += 2;
                while !matches!(self.peek(), None | Some(b'\n')) {
                    self.position += 1;
                }
                continue;
            }
            if self.bytes[self.position..].starts_with(b"/*") {
                self.position += 2;
                while !self.bytes[self.position..].starts_with(b"*/") {
                    if self.at_end() {
                        return;
                    }
                    self.position += 1;
                }
                self.position += 2;
                continue;
            }
            return;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            return true;
        }
        false
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        if self.bytes[self.position..].starts_with(keyword.as_bytes()) {
            self.position += keyword.len();
            return true;
        }
        false
    }

    fn value(&mut self) -> Option<ParsedType> {
        if self.depth >= MAX_DEPTH {
            return self.skip_value().then_some(ParsedType::Unknown);
        }
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(|_| ParsedType::String),
            b'n' => self.eat_keyword("null").then_some(ParsedType::Null),
            b't' => self.eat_keyword("true").then_some(ParsedType::Boolean),
            b'f' => self.eat_keyword("false").then_some(ParsedType::Boolean),
            b'-' | b'0'..=b'9' => self.number().then_some(ParsedType::Number),
            _ => None,
        }
    }

    /// Consumes one value without building a type. Iterative, so the depth cap
    /// it backs cannot itself be defeated by nesting. Past the cap the shape is
    /// no longer modelled, so this only has to find the value's end.
    fn skip_value(&mut self) -> bool {
        let mut nesting: u32 = 0;
        loop {
            self.skip_whitespace();
            let Some(byte) = self.peek() else {
                return false;
            };
            match byte {
                b'{' | b'[' => {
                    self.position += 1;
                    nesting += 1;
                    continue;
                }
                b'}' | b']' => {
                    self.position += 1;
                    let Some(remaining) = nesting.checked_sub(1) else {
                        return false;
                    };
                    nesting = remaining;
                }
                b',' | b':' => {
                    self.position += 1;
                    continue;
                }
                b'"' => {
                    if self.string().is_none() {
                        return false;
                    }
                }
                b'n' if self.eat_keyword("null") => {}
                b't' if self.eat_keyword("true") => {}
                b'f' if self.eat_keyword("false") => {}
                b'-' | b'0'..=b'9' if self.number() => {}
                _ => return false,
            }
            if nesting == 0 {
                return true;
            }
        }
    }

    fn object(&mut self) -> Option<ParsedType> {
        self.position += 1;
        self.depth += 1;
        let mut properties: Vec<ParsedObjectTypeProperty> = Vec::new();
        self.skip_whitespace();
        if !self.eat(b'}') {
            loop {
                self.skip_whitespace();
                let name = self.string()?;
                self.skip_whitespace();
                if !self.eat(b':') {
                    return None;
                }
                self.skip_whitespace();
                let ty = self.value()?;
                // A duplicate key is legal JSON and the last one wins at
                // runtime, which is also what tsc types.
                if let Some(existing) = properties.iter_mut().find(|property| property.name == name)
                {
                    existing.ty = ty;
                } else {
                    properties.push(ParsedObjectTypeProperty {
                        name,
                        name_span: None,
                        ty,
                        optional: false,
                        is_method: false,
                        readonly: false,
                        write_ty: None,
                    });
                }
                self.skip_whitespace();
                if self.eat(b',') {
                    // A trailing comma is allowed, so the closing brace can
                    // follow it directly.
                    self.skip_whitespace();
                    if self.eat(b'}') {
                        break;
                    }
                    continue;
                }
                if self.eat(b'}') {
                    break;
                }
                return None;
            }
        }
        self.depth -= 1;
        Some(ParsedType::Object(Arc::new(ParsedObjectType {
            properties,
            string_index_type: None,
            number_index_type: None,
            string_index_readonly: false,
            number_index_readonly: false,
            call_signature: None,
            call_signature_overloads: Vec::new(),
            construct_signature: None,
            construct_signature_overloads: Vec::new(),
            non_primitive: false,
            display_name: None,
            abstract_construct_signature: false,
            keyed_indexes: None,
            number_index_span: None,
        })))
    }

    fn array(&mut self) -> Option<ParsedType> {
        self.position += 1;
        self.depth += 1;
        let mut elements: Vec<ParsedType> = Vec::new();
        self.skip_whitespace();
        if !self.eat(b']') {
            loop {
                self.skip_whitespace();
                let element = self.value()?;
                if !elements.contains(&element) {
                    elements.push(element);
                }
                self.skip_whitespace();
                if self.eat(b',') {
                    self.skip_whitespace();
                    if self.eat(b']') {
                        break;
                    }
                    continue;
                }
                if self.eat(b']') {
                    break;
                }
                return None;
            }
        }
        self.depth -= 1;
        // `[]` is `never[]` in tsc, and an array of mixed values is an array of
        // their union.
        let element = match elements.len() {
            0 => ParsedType::Never,
            1 => elements.pop().expect("length checked"),
            _ => ParsedType::Union(Arc::new(elements)),
        };
        Some(ParsedType::Array(Arc::new(element)))
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut value = String::new();
        loop {
            let byte = self.peek()?;
            self.position += 1;
            match byte {
                b'"' => return Some(value),
                b'\\' => {
                    let escape = self.peek()?;
                    self.position += 1;
                    match escape {
                        b'"' => value.push('"'),
                        b'\\' => value.push('\\'),
                        b'/' => value.push('/'),
                        b'b' => value.push('\u{8}'),
                        b'f' => value.push('\u{c}'),
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'u' => {
                            let code = self.hex4()?;
                            // A lone surrogate cannot be a Rust `char`; the
                            // replacement keeps the key distinct without
                            // failing the whole file.
                            match char::from_u32(u32::from(code)) {
                                Some(character) => value.push(character),
                                None => value.push('\u{fffd}'),
                            }
                        }
                        _ => return None,
                    }
                }
                // Raw control characters are invalid JSON.
                0x00..=0x1f => return None,
                _ => {
                    let start = self.position - 1;
                    while self.peek().is_some_and(|next| next & 0xc0 == 0x80) {
                        self.position += 1;
                    }
                    value.push_str(std::str::from_utf8(&self.bytes[start..self.position]).ok()?);
                }
            }
        }
    }

    fn hex4(&mut self) -> Option<u16> {
        let end = self.position.checked_add(4)?;
        let digits = self.bytes.get(self.position..end)?;
        let text = std::str::from_utf8(digits).ok()?;
        let value = u16::from_str_radix(text, 16).ok()?;
        self.position = end;
        Some(value)
    }

    fn number(&mut self) -> bool {
        let start = self.position;
        self.eat(b'-');
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.position += 1;
        }
        if self.eat(b'.') {
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        self.position > start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_property_names(ty: &ParsedType) -> Vec<String> {
        let ParsedType::Object(object) = ty else {
            panic!("expected an object type, got {ty:?}");
        };
        object
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect()
    }

    #[test]
    fn scalars_widen() {
        assert_eq!(parse_json_module_type(r#""x""#), Some(ParsedType::String));
        assert_eq!(parse_json_module_type("-1.5e3"), Some(ParsedType::Number));
        assert_eq!(parse_json_module_type("true"), Some(ParsedType::Boolean));
        assert_eq!(parse_json_module_type("null"), Some(ParsedType::Null));
    }

    #[test]
    fn object_keeps_declaration_order_and_last_duplicate() {
        let ty = parse_json_module_type(r#"{"b": 1, "a": "x", "b": true}"#).expect("valid json");
        assert_eq!(object_property_names(&ty), vec!["b", "a"]);
        let ParsedType::Object(object) = &ty else {
            unreachable!()
        };
        assert_eq!(object.properties[0].ty, ParsedType::Boolean);
    }

    #[test]
    fn arrays_union_their_elements() {
        assert_eq!(
            parse_json_module_type("[]"),
            Some(ParsedType::Array(Arc::new(ParsedType::Never)))
        );
        assert_eq!(
            parse_json_module_type("[1, 2]"),
            Some(ParsedType::Array(Arc::new(ParsedType::Number)))
        );
        let mixed = parse_json_module_type(r#"[1, "a"]"#).expect("valid json");
        assert_eq!(
            mixed,
            ParsedType::Array(Arc::new(ParsedType::Union(Arc::new(vec![
                ParsedType::Number,
                ParsedType::String
            ]))))
        );
    }

    #[test]
    fn escapes_and_unicode_keys_survive() {
        let ty = parse_json_module_type(r#"{"aA\n": 1}"#).expect("valid json");
        assert_eq!(object_property_names(&ty), vec!["aA\n"]);
        let ty = parse_json_module_type(r#"{"키": 1}"#).expect("valid json");
        assert_eq!(object_property_names(&ty), vec!["키"]);
    }

    #[test]
    fn malformed_input_is_not_a_json_module() {
        assert_eq!(parse_json_module_type("{"), None);
        assert_eq!(parse_json_module_type("{} trailing"), None);
        assert_eq!(parse_json_module_type("{'a': 1}"), None);
        assert_eq!(parse_json_module_type(""), None);
    }

    #[test]
    fn comments_and_trailing_commas_are_accepted_like_tsc() {
        let ty = parse_json_module_type(
            "{\n  // leading\n  \"a\": 1, /* inline */\n  \"b\": [1, 2,],\n}",
        )
        .expect("valid jsonc");
        assert_eq!(object_property_names(&ty), vec!["a", "b"]);
    }

    #[test]
    fn very_deep_nesting_degrades_instead_of_failing() {
        let source = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        assert!(parse_json_module_type(&source).is_some());
    }

    #[test]
    fn file_names_are_matched_case_insensitively() {
        assert!(is_json_file_name("/a/b/package.JSON"));
        assert!(!is_json_file_name("/a/b/package.jsonx"));
        assert!(!is_json_file_name("json"));
    }
}
