use std::sync::Arc;

use indexmap::IndexMap;

use crate::store::{canonical_property_map_store_enabled, current_program_type_store};
use crate::{FunctionType, PropertyMapId, Type, union_type};

/// Property map preserving declaration order, which tsc relies on when rendering
/// object types in diagnostics (`{ disabled?: boolean; children?: unknown }`).
/// Equality is order-independent, matching the previous `BTreeMap` semantics, so
/// structural comparisons and type caching are unaffected.
// Insertion-ordered like any IndexMap (iteration order is hasher-independent),
// with the fast workspace hasher for the per-lookup cost.
//
// Keys are `Arc<str>`, not `String`: a derived interface inherits its bases'
// members by cloning the base map's keys (interface member merge), and the base
// maps are `Arc`-shared and reused across every derived type, so an `Arc<str>`
// key makes each inherited-member clone a refcount bump and lets all derived
// types share one allocation per base member name. Equality is by str content
// (order-independent, as before) and `Arc<str>: Hash` matches `String::hash`
// byte-for-byte, so structural comparison, the dedup fingerprint, and the
// canonical property-map store are unchanged.
pub type PropertyMap = IndexMap<Arc<str>, ObjectProperty, crate::fx::FxBuildHasher>;

#[derive(Debug)]
pub struct ObjectType {
    pub properties: Arc<PropertyMap>,
    pub property_map_id: Option<PropertyMapId>,
    pub string_index_type: Option<Arc<Type>>,
    /// `[key: number]: T`. Kept apart from the string index because the two
    /// answer different keys: a numeric key prefers this one and falls back to
    /// the string index (`findApplicableIndexInfo`), while a *string* key a
    /// number-only type cannot answer is TS7015 under `noImplicitAny` — so an
    /// object declaring only this one leaves `string_index_type` empty.
    pub number_index_type: Option<Arc<Type>>,
    /// tsc's `IndexInfo.isReadonly` for the two index signatures above
    /// (`readonly [key: string]: T`): a write that lands on one is TS2542
    /// (`errorIfWritingToReadonlyIndex`). Part of equality, like
    /// `ObjectProperty::readonly`.
    pub string_index_readonly: bool,
    pub number_index_readonly: bool,
    /// Name of the interface or type alias this object was resolved from, used
    /// only for diagnostic display (tsc shows `'StrictObj'`, not the structural
    /// expansion). Deliberately excluded from equality so assignability and
    /// structural comparisons stay name-agnostic.
    pub alias_name: Option<Arc<str>>,
    /// Nominal identity of the non-generic interface/type-alias declaration this
    /// object was resolved from (qualified `file::name`). Two objects resolved
    /// from the same declaration share it; assignability treats them as the same
    /// named type, matching tsc's nominal handling and avoiding spurious failures
    /// when a deeply cyclic library type (e.g. `Buffer`) expands to structurally
    /// different shapes at different sites. Excluded from equality, like
    /// `alias_name`, so structural comparisons stay shape-based.
    pub alias_id: Option<Arc<str>>,
    /// Construct signature for a class value (static side). When present, the
    /// object is callable with `new`, producing the signature's return type (the
    /// instance type). Static members live in `properties`. Part of equality —
    /// see `signatures_equal`.
    pub construct_signature: Option<Arc<FunctionType>>,
    /// Call signature for a callable object (a `declare var Number: NumberConstructor`
    /// style value whose interface has a `(value?: any): number` signature). When
    /// present, the object is callable without `new`, producing the signature's
    /// return type. Part of equality, like `construct_signature`.
    pub call_signature: Option<Arc<FunctionType>>,
    /// Set when this object is the merged surface of an intersection (`A & B`).
    /// Used only to pick the diagnostic tsc reports for a missing required
    /// property (intersections surface the outer assignability code, e.g.
    /// `TS2322`/`TS2345`, rather than the standalone `TS2741`). Excluded from
    /// equality so intersection-merged objects compare structurally.
    pub is_intersection: bool,
    /// Set when `string_index_type` was injected by the checker to keep an
    /// intersection surface open (an operand it could not enumerate), not
    /// declared by the source. The index still widens property *reads*, but
    /// `noPropertyAccessFromIndexSignature` must not fire on it: the property
    /// really lives on the operand surge dropped, not on an index signature the
    /// author wrote. Excluded from equality, like `is_intersection`, so no cache
    /// key, dedup fingerprint, or canonical-store identity changes.
    pub synthetic_open_index: bool,
    /// Set for the `object` keyword. The member surface is the empty object,
    /// but assignability refuses a primitive source. Excluded from equality
    /// like the other markers.
    pub non_primitive: bool,
    /// Set for an object resolved from an `interface` or `class` declaration.
    /// tsc gives only object and type literals an implicit index signature
    /// (`isObjectTypeWithInferableIndex`), so such a source answers a target
    /// index signature with one it declares or not at all. Excluded from
    /// equality like the other markers.
    pub without_inferable_index: bool,
    /// The nominal reference operands of the intersection this object was
    /// merged from (`Matcher<unknown, string> & Omit<…>` keeps the `Matcher`
    /// reference). An `infer` pattern naming that declaration binds its
    /// captures off the reference's arguments, which the merged property map
    /// no longer carries. Provenance only — excluded from equality like the
    /// other markers.
    pub intersection_operands: Option<Arc<[Type]>>,
    /// See [`KeyedIndexes`]. Part of equality.
    pub keyed_indexes: Option<Arc<KeyedIndexes>>,
    /// See [`IndexParameterNames`]. Display only — excluded from equality.
    pub index_parameter_names: Option<Arc<IndexParameterNames>>,
}

/// The parameter names the index signatures were declared with
/// (`[index: string]: T`), which tsc prints. One synthesized from keys (a
/// mapped type over `string`) has none and prints as `x`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexParameterNames {
    pub string: Option<Arc<str>>,
    pub number: Option<Arc<str>>,
}

/// Index signatures keyed by a template literal pattern or a branded string
/// (`[k: `data${string}`]: T`, `[k: string & Tag]: T`), one per key
/// constituent. Surge answers them through `string_index_type`, which holds
/// `stand_in`; while it still does, these decide which keys that slot really
/// applies to (`isApplicableIndexType`) and with which value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyedIndexes {
    pub infos: Vec<(Type, Type)>,
    pub stand_in: Option<Type>,
    /// `[k: symbol]: T`, which answers no string or number key.
    pub symbol: Option<Type>,
}

impl KeyedIndexes {
    /// tsc's `getApplicableIndexInfo` for `key` (`isApplicableIndexType`),
    /// distributed over a union key: `None` when a constituent has no
    /// applicable info. Overlapping infos read the first one's value rather
    /// than the intersection of all of them.
    pub fn read(&self, key: &Type) -> Option<Type> {
        let constituents = match crate::peel_to_pattern_literal(key) {
            Type::Union(union) => union.types().to_vec(),
            _ => vec![key.clone()],
        };
        let values = constituents
            .iter()
            .map(|constituent| {
                self.infos
                    .iter()
                    .find(|(info_key, _)| crate::is_assignable_to(constituent, info_key))
                    .map(|(_, value)| value.clone())
            })
            .collect::<Option<Vec<_>>>()?;
        Some(union_type(values))
    }
}

impl ObjectType {
    /// The keyed indexes, while the string slot is still their stand-in: an
    /// intersection merge or a rewrite of the slot leaves them stale.
    pub fn active_keyed_indexes(&self) -> Option<&KeyedIndexes> {
        let keyed = self.keyed_indexes.as_deref()?;
        (!keyed.infos.is_empty()
            && !self.is_intersection
            && !self.synthetic_open_index
            && keyed.stand_in.is_some()
            && self.string_index_type.as_deref() == keyed.stand_in.as_ref())
        .then_some(keyed)
    }

    pub fn symbol_index_type(&self) -> Option<&Type> {
        self.keyed_indexes.as_deref()?.symbol.as_ref()
    }
}

impl PartialEq for ObjectType {
    fn eq(&self, other: &Self) -> bool {
        // Object types resolved from the same memoized reference share a property
        // map `Arc`; comparing their pointers short-circuits the O(size) structural
        // compare, which dominates checking projects with large library object
        // graphs (DOM `Request`/`Response`, …) reached through nominal references.
        let properties_equal =
            Arc::ptr_eq(&self.properties, &other.properties) || self.properties == other.properties;
        properties_equal
            && self.string_index_type == other.string_index_type
            && self.number_index_type == other.number_index_type
            && self.string_index_readonly == other.string_index_readonly
            && self.number_index_readonly == other.number_index_readonly
            && signatures_equal(&self.construct_signature, &other.construct_signature)
            && signatures_equal(&self.call_signature, &other.call_signature)
            && self.keyed_indexes == other.keyed_indexes
            && (!(self.is_memberless_intersection() || other.is_memberless_intersection())
                || (self.intersection_operands == other.intersection_operands
                    && self.non_primitive == other.non_primitive))
    }
}

impl ObjectType {
    /// An intersection with no members of its own (`T & null`, `string & {}`):
    /// its operands are what it is, so they decide equality — `(T & object) |
    /// (T & null)` must not collapse to one member.
    fn is_memberless_intersection(&self) -> bool {
        self.is_intersection
            && self.properties.is_empty()
            && self.string_index_type.is_none()
            && self.number_index_type.is_none()
            && self.intersection_operands.is_some()
    }
}

/// Call/construct signatures participate in equality: two class *static* sides
/// (`typeof AppendCommand` vs `typeof PExpireAtCommand`) usually carry identical
/// (often empty) static property maps and differ only in their constructor, so
/// leaving signatures out made them compare equal — and any cache keyed on
/// resolved type arguments (the generic-instantiation interner) then served one
/// class's expansion for another's.
fn signatures_equal(left: &Option<Arc<FunctionType>>, right: &Option<Arc<FunctionType>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right) || left == right,
        _ => false,
    }
}

impl Eq for ObjectType {}

#[derive(Debug, Clone)]
pub struct ObjectProperty {
    pub ty: Type,
    pub optional: bool,
    /// Declared with method syntax (`m(): T`) rather than as a property holding
    /// a function type (`m: () => T`). tsc checks a method's parameters
    /// bivariantly even under `strictFunctionTypes`, so the distinction is
    /// load-bearing for assignability.
    pub method: bool,
    /// Declared `readonly`, or a getter with no matching setter. tsc's
    /// `isReadonlySymbol`: a write to such a member is TS2540 whatever its
    /// type. Like `method`, it participates in equality but not in the property
    /// fingerprint — two objects differing only here land in one bucket and are
    /// told apart by equality.
    pub readonly: bool,
    /// A class member declared `private` or `protected`, with the declaration
    /// it belongs to. tsc relates such a member only to itself (private) or to
    /// a restricted member (protected), whatever its type. Participates in
    /// equality but not in the property fingerprint, like `readonly`.
    pub restriction: Option<MemberRestriction>,
    /// Not a declaration: the slot of the string index signature this name
    /// reads, recorded by flow narrowing (`if (env.URL) env.URL`). It answers
    /// reads like a property but still *comes from* the index signature, which
    /// is what `noPropertyAccessFromIndexSignature` asks.
    pub index_slot: bool,
    /// The declaration's name as written where that is not the key
    /// (`'1.0'`, `"a b"`), which tsc names the member by. Display only: it
    /// takes no part in equality.
    pub written_name: Option<Arc<String>>,
    /// Where an object literal declares the member, which the compiler API
    /// answers as its declaration. Takes no part in equality.
    pub declaration: Option<Arc<PropertyDeclarationSite>>,
}

/// An object literal member's name: its file and byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDeclarationSite {
    pub file: Arc<str>,
    pub start: u32,
    pub end: u32,
}

impl PartialEq for ObjectProperty {
    fn eq(&self, other: &Self) -> bool {
        self.ty == other.ty
            && self.optional == other.optional
            && self.method == other.method
            && self.readonly == other.readonly
            && self.restriction == other.restriction
            && self.index_slot == other.index_slot
    }
}

impl Eq for ObjectProperty {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRestriction {
    pub private: bool,
    /// Identity of the declaring class: its file and the offset of its name.
    /// A thin pointer, so the property stays small.
    pub owner: Arc<String>,
}

impl ObjectProperty {
    pub fn required(ty: Type) -> Self {
        Self {
            ty,
            optional: false,
            method: false,
            readonly: false,
            restriction: None,
            index_slot: false,
            written_name: None,
            declaration: None,
        }
    }

    pub fn optional(ty: Type) -> Self {
        Self {
            ty,
            optional: true,
            method: false,
            readonly: false,
            restriction: None,
            index_slot: false,
            written_name: None,
            declaration: None,
        }
    }

    pub fn with_method(mut self, method: bool) -> Self {
        self.method = method;
        // The member's signature is declared by the method, as tsc's
        // `Signature.declaration` records it wherever the type is read.
        if method && let Type::Function(function) = &self.ty {
            self.ty = Type::Function(function.clone().with_method_declaration(true));
        }
        self
    }

    pub fn with_readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn with_written_name(mut self, written_name: Option<Arc<String>>) -> Self {
        self.written_name = written_name;
        self
    }

    pub fn with_declaration(mut self, declaration: Option<Arc<PropertyDeclarationSite>>) -> Self {
        self.declaration = declaration;
        self
    }

    pub fn is_optional(&self) -> bool {
        self.optional
    }

    pub fn is_required(&self) -> bool {
        !self.optional
    }

    pub fn is_method(&self) -> bool {
        self.method
    }
}

impl ObjectType {
    pub fn new(properties: PropertyMap, string_index_type: Option<Type>) -> Self {
        let owned = |mut properties: PropertyMap| {
            properties.shrink_to_fit();
            Arc::new(properties)
        };
        let (properties, property_map_id) = if canonical_property_map_store_enabled()
            && let Some(store) = current_program_type_store()
        {
            match store.intern_property_map(properties) {
                Ok((map, id)) => (map, Some(id)),
                Err(properties) => (owned(properties), None),
            }
        } else {
            (owned(properties), None)
        };
        Self {
            properties,
            property_map_id,
            string_index_type: string_index_type.map(Arc::new),
            number_index_type: None,
            string_index_readonly: false,
            number_index_readonly: false,
            alias_name: None,
            alias_id: None,
            construct_signature: None,
            call_signature: None,
            is_intersection: false,
            synthetic_open_index: false,
            non_primitive: false,
            without_inferable_index: false,
            intersection_operands: None,
            keyed_indexes: None,
            index_parameter_names: None,
        }
    }

    /// The array or tuple operand of the intersection this object was merged
    /// from (`string[]` in `string[] & { extra: number }`): the value is that
    /// array, with the object's members on top.
    pub fn intersected_array(&self) -> Option<&Type> {
        self.intersection_operands
            .as_deref()?
            .iter()
            .find(|operand| matches!(operand, Type::Array(_) | Type::Tuple(_)))
    }

    /// The primitive operand of the intersection this object was merged from
    /// (`number` in `number & { __brand: true }`): the value *is* that
    /// primitive, with the object's members on top.
    pub fn intersected_primitive(&self) -> Option<&Type> {
        self.intersection_operands.as_deref()?.iter().find(|operand| {
            matches!(
                operand,
                Type::String
                    | Type::Number
                    | Type::Boolean
                    | Type::BigInt
                    | Type::Symbol
                    | Type::StringLiteral(_)
                    | Type::NumberLiteral(_)
                    | Type::BooleanLiteral(_)
            )
        })
    }

    /// Declares `[key: number]: T` on this object; see `number_index_type`.
    pub fn with_number_index_type(mut self, number_index_type: Option<Type>) -> Self {
        self.number_index_type = number_index_type.map(Arc::new);
        self
    }

    /// Marks the string and number index signatures `readonly`; see
    /// `string_index_readonly`. Set after the index types: a flag for a
    /// signature the object does not have is dropped.
    pub fn with_readonly_indexes(mut self, string: bool, number: bool) -> Self {
        self.string_index_readonly = string && self.string_index_type.is_some();
        self.number_index_readonly = number && self.number_index_type.is_some();
        self
    }

    /// Whether the index signature [`Self::applicable_index_type`] picks is
    /// `readonly`. A checker-injected openness index declares nothing.
    pub fn applicable_index_readonly(&self, key_is_numeric: bool) -> bool {
        if key_is_numeric && self.number_index_type.is_some() {
            return self.number_index_readonly;
        }
        self.string_index_readonly && !self.synthetic_open_index
    }

    /// The index signature that answers `key`, following tsc's
    /// `findApplicableIndexInfo`: a numeric key prefers the number index and
    /// falls back to the string one; every other key can only use the string
    /// index.
    pub fn applicable_index_type(&self, key_is_numeric: bool) -> Option<&Type> {
        if key_is_numeric && let Some(number_index) = self.number_index_type.as_deref() {
            return Some(number_index);
        }
        self.string_index_type.as_deref()
    }

    /// tsc's `hasNumericPropertyNames`: exactly one index signature, and it is
    /// the numeric one. A `for…in` over such a type binds its key as `number`.
    pub fn has_numeric_property_names(&self) -> bool {
        self.number_index_type.is_some() && self.string_index_type.is_none()
    }

    /// Records the nominal reference operands of the intersection this object
    /// merges; see `intersection_operands`.
    pub fn with_intersection_operands(mut self, operands: Vec<Type>) -> Self {
        self.intersection_operands = (!operands.is_empty()).then(|| Arc::from(operands));
        self
    }

    /// Returns a copy tagged with the interface/type-alias name it was resolved
    /// from, for diagnostic display only.
    pub fn with_alias_name(mut self, alias_name: impl Into<Arc<str>>) -> Self {
        self.alias_name = Some(alias_name.into());
        self
    }

    /// Returns a copy tagged with the nominal identity of its source declaration.
    pub fn with_alias_id(mut self, alias_id: impl Into<Arc<str>>) -> Self {
        self.alias_id = Some(alias_id.into());
        self
    }

    /// Marks this object as the merged surface of an intersection type.
    pub fn with_intersection_marker(mut self) -> Self {
        self.is_intersection = true;
        self
    }

    pub fn with_index_parameter_names(mut self, names: Option<Arc<IndexParameterNames>>) -> Self {
        self.index_parameter_names = names;
        self
    }

    /// Marks this object's string index signature as checker-injected openness
    /// rather than a declared `[key: string]: T`.
    pub fn with_open_index_marker(mut self) -> Self {
        self.synthetic_open_index = true;
        self
    }

    /// Marks this object as resolved from an `interface` or `class` declaration.
    pub fn with_nominal_declaration_marker(mut self) -> Self {
        self.without_inferable_index = true;
        self
    }

    /// Marks this object as the `object` keyword: no primitive satisfies it.
    pub fn with_non_primitive_marker(mut self) -> Self {
        self.non_primitive = true;
        self
    }

    /// Returns a copy carrying a construct signature, marking this object as the
    /// static/value side of a class that can be invoked with `new`.
    pub fn with_construct_signature(mut self, construct_signature: FunctionType) -> Self {
        self.construct_signature = Some(Arc::new(construct_signature));
        self
    }

    pub fn construct_signature(&self) -> Option<&FunctionType> {
        self.construct_signature.as_deref()
    }

    /// Returns a copy carrying a call signature, marking this object as callable
    /// without `new` (e.g. `Number(value)`).
    pub fn with_call_signature(mut self, call_signature: FunctionType) -> Self {
        self.call_signature = Some(Arc::new(call_signature));
        self
    }

    pub fn call_signature(&self) -> Option<&FunctionType> {
        self.call_signature.as_deref()
    }

    pub fn get_property(&self, name: &str) -> Option<&ObjectProperty> {
        self.properties.get(name)
    }

    pub fn get_property_type(&self, name: &str) -> Option<&Type> {
        self.properties.get(name).map(|property| &property.ty)
    }

    pub fn get_property_access_type(&self, name: &str) -> Option<Type> {
        if let Some(property) = self.properties.get(name) {
            if property.is_optional() {
                return Some(union_type(vec![property.ty.clone(), Type::Undefined]));
            }

            return Some(property.ty.clone());
        }

        // No index signature and no `Object` member answers a private name
        // (tsc reads it with `getPropertyOfType` alone). Surge's own openness
        // marker still does: it stands for members it could not enumerate.
        if crate::private_name::is_private_name_key(name) {
            return self
                .synthetic_open_index
                .then(|| self.string_index_type.as_deref().cloned())
                .flatten();
        }

        // Every object type's apparent type includes the global `Object`
        // interface members, so `error.toString()` / `value.hasOwnProperty(k)`
        // resolve even when the declared interface (e.g. lib's `Error`) never
        // restates them. tsc's `getPropertyOfType` finds them before any index
        // signature is consulted: `record.hasOwnProperty` on
        // `Record<string, unknown>` is the method, not `unknown`.
        if let Some(member) = object_prototype_member_type(name) {
            return Some(member);
        }
        if let Some(indexed) = self.keyed_index_for_name(name) {
            return indexed;
        }
        // A numeric member name is answered by the numeric index signature
        // first — `record[1]` on `{ [k: number]: T }` — and by the string one
        // otherwise, exactly as `findApplicableIndexInfo` orders them.
        self.applicable_index_type(is_numeric_key(name)).cloned()
    }

    pub fn contains_property(&self, name: &str) -> bool {
        self.properties.contains_key(name)
            || (!crate::private_name::is_private_name_key(name)
                && match self.keyed_index_for_name(name) {
                    Some(indexed) => indexed.is_some(),
                    None => self.applicable_index_type(is_numeric_key(name)).is_some(),
                })
    }

    /// `getApplicableIndexInfoForName` while keyed indexes are active (`Some`):
    /// the name is read as its string literal, and a numeric one may still
    /// fall to the number index.
    pub fn keyed_index_for_name(&self, name: &str) -> Option<Option<Type>> {
        let keyed = self.active_keyed_indexes()?;
        Some(
            keyed
                .read(&Type::StringLiteral(name.to_string()))
                .or_else(|| {
                    is_numeric_key(name)
                        .then(|| self.number_index_type.as_deref().cloned())
                        .flatten()
                }),
        )
    }

    /// Like [`Self::contains_property`], but counts only members the SOURCE
    /// declared — a checker-injected openness index does not make every name a
    /// known property. See [`Self::declares_string_index_access`].
    pub fn declares_property(&self, name: &str) -> bool {
        self.properties.contains_key(name)
            || (!crate::private_name::is_private_name_key(name) && self.declares_string_index_access())
    }

    pub fn allows_string_index_access(&self) -> bool {
        self.string_index_type.is_some()
    }

    /// Like [`Self::allows_string_index_access`], but only for an index the
    /// SOURCE declared. Checker-injected openness (`synthetic_open_index`, set
    /// when an intersection dropped an unmodelled operand) means "reads of
    /// unknown members are not errors here" — it does not mean the type declares
    /// those members, so it must not silence excess-property checking the way a
    /// real `[key: string]: T` does.
    pub fn declares_string_index_access(&self) -> bool {
        self.string_index_type.is_some() && !self.synthetic_open_index
    }

    pub fn required_properties(&self) -> impl Iterator<Item = (&Arc<str>, &ObjectProperty)> + '_ {
        self.properties.iter().filter(|entry| entry.1.is_required())
    }

    pub fn optional_properties(&self) -> impl Iterator<Item = (&Arc<str>, &ObjectProperty)> + '_ {
        self.properties.iter().filter(|entry| entry.1.is_optional())
    }
}

/// Whether a property name is a numeric one — the form a numeric index
/// signature answers (tsc: `isNumericLiteralName`).
pub fn is_numeric_key(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

/// The members of the global `Object` interface (lib.es5), shared by the apparent
/// type of every non-nullish value. Parameter types are approximated as `any`
/// (the real signatures take `PropertyKey`/`Object`) since only arity and the
/// return type matter for the diagnostics surge emits.
pub fn object_prototype_member_type(name: &str) -> Option<Type> {
    let member = match name {
        "toString" | "toLocaleString" => FunctionType::new(vec![], Type::String, false, 0),
        "valueOf" => FunctionType::new(vec![], Type::Any, false, 0),
        "hasOwnProperty" | "isPrototypeOf" | "propertyIsEnumerable" => {
            FunctionType::new(vec![Type::Any], Type::Boolean, false, 1)
        }
        "constructor" => return Some(Type::Any),
        _ => return None,
    };
    Some(Type::Function(member))
}

impl Clone for ObjectType {
    fn clone(&self) -> Self {
        Self {
            properties: self.properties.clone(),
            property_map_id: self.property_map_id,
            string_index_type: self.string_index_type.clone(),
            number_index_type: self.number_index_type.clone(),
            string_index_readonly: self.string_index_readonly,
            number_index_readonly: self.number_index_readonly,
            alias_name: self.alias_name.clone(),
            alias_id: self.alias_id.clone(),
            construct_signature: self.construct_signature.clone(),
            call_signature: self.call_signature.clone(),
            is_intersection: self.is_intersection,
            synthetic_open_index: self.synthetic_open_index,
            non_primitive: self.non_primitive,
            without_inferable_index: self.without_inferable_index,
            intersection_operands: self.intersection_operands.clone(),
            keyed_indexes: self.keyed_indexes.clone(),
            index_parameter_names: self.index_parameter_names.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FunctionType;

    fn constructor(parameter: Type) -> FunctionType {
        FunctionType::new(vec![parameter], Type::Number, false, 1)
    }

    #[test]
    fn static_sides_differing_only_in_their_constructor_are_not_equal() {
        // Two class static sides carry the same (here empty) static property map
        // and differ only in the constructor, so leaving signatures out of
        // equality made every `typeof SomeClass` interchangeable — and caches
        // keyed on resolved type arguments served one class's expansion for
        // another's.
        let append = ObjectType::new(PropertyMap::default(), None)
            .with_construct_signature(constructor(Type::String));
        let expire = ObjectType::new(PropertyMap::default(), None)
            .with_construct_signature(constructor(Type::Number));
        assert_ne!(append, expire);

        let same = ObjectType::new(PropertyMap::default(), None)
            .with_construct_signature(constructor(Type::String));
        assert_eq!(append, same);
    }

    #[test]
    fn the_synthetic_open_index_marker_stays_out_of_equality() {
        // Like `is_intersection`: the marker only steers diagnostic selection, so
        // it must not split cache keys, dedup fingerprints, or store identity.
        let declared = ObjectType::new(PropertyMap::default(), Some(Type::Any));
        let synthetic =
            ObjectType::new(PropertyMap::default(), Some(Type::Any)).with_open_index_marker();
        assert_eq!(declared, synthetic);
        assert!(!declared.synthetic_open_index);
        assert!(synthetic.synthetic_open_index);
        assert!(synthetic.clone().synthetic_open_index);
    }

    #[test]
    fn a_synthetic_open_index_is_not_an_index_signature_source() {
        let declared = Type::Object(ObjectType::new(PropertyMap::default(), Some(Type::Any)));
        let synthetic = Type::Object(
            ObjectType::new(PropertyMap::default(), Some(Type::Any)).with_open_index_marker(),
        );
        assert!(declared.property_only_from_string_index("path"));
        assert!(!synthetic.property_only_from_string_index("path"));
    }

    #[test]
    fn a_callable_object_is_not_equal_to_a_plain_one() {
        let plain = ObjectType::new(PropertyMap::default(), None);
        let callable = ObjectType::new(PropertyMap::default(), None)
            .with_call_signature(constructor(Type::String));
        assert_ne!(plain, callable);
    }
}

