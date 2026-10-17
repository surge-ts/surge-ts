use std::hash::{Hash, Hasher};

use crate::fx::{FxHasher, PrehashedU64Map};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::clone_reason::{TypeCopyReason, current_type_copy_reason};
use crate::store::{canonical_union_store_enabled, current_program_type_store};
use crate::{Type, TypeListId, UnionTypeId};

static UNION_TYPE_PAYLOAD_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_PAYLOAD_DEEP_CLONE_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_HANDLE_COPY_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT: AtomicU64 = AtomicU64::new(0);
static UNION_TYPE_COPY_UNATTRIBUTED_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnionTypeCounters {
    pub union_type_payload_alloc_count: u64,
    pub union_type_payload_deep_clone_count: u64,
    pub union_type_handle_copy_count: u64,
    pub union_type_copy_from_expression_inference_count: u64,
    pub union_type_copy_from_call_resolution_count: u64,
    pub union_type_copy_from_property_call_resolution_count: u64,
    pub union_type_copy_from_function_body_setup_count: u64,
    pub union_type_copy_from_return_checking_count: u64,
    pub union_type_copy_from_expected_type_count: u64,
    pub union_type_copy_from_symbol_table_count: u64,
    pub union_type_copy_from_module_export_count: u64,
    pub union_type_copy_from_scope_or_context_count: u64,
    pub union_type_copy_from_substitution_unchanged_count: u64,
    pub union_type_copy_from_substitution_changed_count: u64,
    pub union_type_copy_from_diagnostic_formatting_count: u64,
    pub union_type_copy_unattributed_count: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnionTypePayload {
    pub types: Arc<[Type]>,
    pub(crate) list_id: Option<TypeListId>,
    pub(crate) name_memo: crate::name_memo::NameMemo,
}

impl Clone for UnionTypePayload {
    fn clone(&self) -> Self {
        record_union_type_payload_deep_clone_count();
        Self {
            types: self.types.clone(),
            list_id: self.list_id,
            name_memo: self.name_memo.clone(),
        }
    }
}

#[derive(Debug)]
pub struct UnionType {
    payload: Arc<UnionTypePayload>,
    id: Option<UnionTypeId>,
    /// The type-alias name this union was written as (`type Level = "a" | "b"`),
    /// for display only. tsc names such a union by its alias in diagnostics.
    /// Handle-local, like [`crate::FunctionType`]'s parameter names: the payload
    /// stays interned and shared, so rendering never depends on intern order.
    alias_name: Option<Arc<str>>,
}

impl UnionType {
    pub fn new(types: Vec<Type>) -> Self {
        if canonical_union_store_enabled()
            && let Some(store) = current_program_type_store()
        {
            match store.intern_union(types) {
                Ok((payload, id)) => {
                    return Self {
                        payload,
                        id: Some(id),
                        alias_name: None,
                    };
                }
                Err(types) => {
                    record_union_type_payload_alloc_count();
                    return Self {
                        payload: Arc::new(UnionTypePayload {
                            types: types.into(),
                            list_id: None,
                            name_memo: crate::name_memo::NameMemo::default(),
                        }),
                        id: None,
                        alias_name: None,
                    };
                }
            }
        }
        record_union_type_payload_alloc_count();
        Self {
            payload: Arc::new(UnionTypePayload {
                types: types.into(),
                list_id: None,
                name_memo: crate::name_memo::NameMemo::default(),
            }),
            id: None,
            alias_name: None,
        }
    }

    /// Names this union by the alias it was written as, for display only.
    pub fn with_alias_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.alias_name = Some(name.into());
        self
    }

    pub fn alias_name(&self) -> Option<&str> {
        self.alias_name.as_deref()
    }

    pub fn without_alias_name(mut self) -> Self {
        self.alias_name = None;
        self
    }

    /// Like [`Self::new`], but probes the interner through borrowed members so
    /// an interner hit (the overwhelmingly common case: 2.4M hits vs 89k
    /// unique unions on tRPC) never deep-clones the member types. Only a miss
    /// — a genuinely new canonical union — materializes the owned member list.
    fn from_borrowed_members(members: &[&Type]) -> Self {
        if canonical_union_store_enabled()
            && let Some(store) = current_program_type_store()
            && let Some((payload, id)) = store.intern_union_borrowed(members)
        {
            return Self {
                payload,
                id: Some(id),
                alias_name: None,
            };
        }
        Self::new(members.iter().map(|ty| (*ty).clone()).collect())
    }

    pub fn payload(&self) -> &UnionTypePayload {
        &self.payload
    }

    pub fn types(&self) -> &[Type] {
        &self.payload.types
    }

    pub fn name(&self) -> String {
        if let Some(alias_name) = self.alias_name.as_deref() {
            return alias_name.to_string();
        }
        if self.types().is_empty() {
            return "unknown".to_string();
        }
        if crate::tsc_display::active() {
            return crate::tsc_display::memoized(self.payload_address(), || self.render_members());
        }
        self.payload.name_memo.get_or_render(|| self.render_members()).to_string()
    }

    fn render_members(&self) -> String {
        // Distinct members can render to one name — every member of a nominal
        // enum displays as the enum itself — and tsc prints that name once.
        let mut rendered: Vec<String> = Vec::with_capacity(self.types().len());
        let mut members: Vec<&Type> = self.types().iter().collect();
        if crate::tsc_display::active() {
            // `getUnionType` flattens a member union, and `unknown` or `any`
            // absorbs the rest.
            fn flatten<'a>(types: &'a [Type], out: &mut Vec<&'a Type>) {
                for member in types {
                    match member {
                        Type::Union(inner) if inner.alias_name.is_none() => flatten(inner.types(), out),
                        member => out.push(member),
                    }
                }
            }
            let mut flat = Vec::with_capacity(members.len());
            flatten(self.types(), &mut flat);
            if flat.iter().any(|member| matches!(member, Type::Any)) {
                return "any".to_string();
            }
            if flat.iter().any(|member| matches!(member, Type::GenuineUnknown)) {
                return "unknown".to_string();
            }
            members = flat;
        }
        if crate::tsc_display::active() {
            // typescript@6 orders a union by type id: the intrinsics in the
            // order the checker creates them, then every other type in the
            // order it was first created, which the written order stands for.
            members.sort_by_key(|member| crate::tsc_display::intrinsic_rank(member));
            // Its literal reduction drops a literal its primitive covers.
            let has = |primitive: &Type| members.iter().any(|member| *member == primitive);
            let (strings, numbers) = (has(&Type::String), has(&Type::Number));
            members.retain(|member| match member {
                Type::StringLiteral(_) => !strings,
                Type::NumberLiteral(_) => !numbers,
                _ => true,
            });
        } else {
            members.sort_by(|left, right| display_order(left, right));
        }
        // `formatUnionTypes` prints `null`, then `undefined`, last.
        let nullables: Vec<&Type> = [Type::Null, Type::Undefined]
            .iter()
            .filter(|nullable| members.contains(nullable))
            .collect();
        members.retain(|member| !matches!(member, Type::Null | Type::Undefined));
        members.extend(nullables);
        for member in members {
            let name = crate::tsc_display::constituent_name(member);
            if !rendered.iter().any(|existing| *existing == name) {
                rendered.push(name);
            }
        }
        rendered.join(" | ")
    }

    pub fn id(&self) -> Option<UnionTypeId> {
        self.id
    }

    pub fn payload_address(&self) -> usize {
        Arc::as_ptr(&self.payload) as usize
    }

    /// The interned member-list id, if this union's payload was interned.
    /// Participates in the derived payload equality, so equality-faithful
    /// cache identities must include it.
    pub fn list_id(&self) -> Option<TypeListId> {
        self.payload.list_id
    }

    pub fn member_list_address(&self) -> usize {
        self.payload.types.as_ptr() as usize
    }
}

impl PartialEq for UnionType {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload, &other.payload) || self.payload == other.payload
    }
}

impl Eq for UnionType {}

impl Clone for UnionType {
    fn clone(&self) -> Self {
        record_union_type_handle_copy_count();
        record_union_type_copy_count_for_current_reason();
        Self {
            payload: self.payload.clone(),
            id: self.id,
            alias_name: self.alias_name.clone(),
        }
    }
}

pub fn remove_undefined(ty: &Type) -> Type {
    match ty {
        Type::Union(union) => {
            let filtered: Vec<Type> = union
                .types()
                .iter()
                .filter(|t| **t != Type::Undefined)
                .cloned()
                .collect();
            union_type(filtered)
        }
        Type::Undefined => Type::Unknown, // Or whatever makes sense, maybe just return it
        _ => ty.clone(),
    }
}

/// tsc's `getTypeWithFacts(type, Truthy)` over a union: the members a truthy
/// value cannot be (`undefined`, `false`, `0`, `""`) go, and `boolean` keeps
/// only `true`. A non-union is left as [`remove_nullish`] leaves it.
pub fn remove_definitely_falsy(ty: &Type) -> Type {
    // tsc narrows the resolved type; a reference standing for a union (a
    // recursive alias's lazy back-edge, `R<T[K]>` resolving to `string |
    // undefined`) narrows as that union. One that resolves to anything else is
    // kept as written, so its nominal display survives.
    if let Some(flattened) = flatten_reference_unions(ty) {
        return remove_definitely_falsy(&flattened);
    }
    let Type::Union(union) = ty else {
        return match ty {
            Type::Boolean => Type::BooleanLiteral(true),
            Type::BooleanLiteral(false) => Type::Never,
            Type::StringLiteral(value) if value.is_empty() => Type::Never,
            Type::NumberLiteral(literal) if literal.value == "0" => Type::Never,
            _ => remove_nullish(ty),
        };
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter_map(|member| match member {
            Type::Undefined | Type::Void | Type::BooleanLiteral(false) => None,
            member if crate::type_variable::is_nullish_type_variable_intersection(member) => None,
            Type::StringLiteral(value) if value.is_empty() => None,
            Type::NumberLiteral(literal) if literal.value == "0" => None,
            Type::Boolean => Some(Type::BooleanLiteral(true)),
            other => Some(other.clone()),
        })
        .collect();
    union_type(kept)
}

/// `ty` with every reference that resolves to a union replaced by that union —
/// the reference itself, or a member of a union (an optional property typed by
/// one reads as `R<…> | undefined`). A narrowing sorts members by what they
/// are, which a reference standing for several of them does not say. `None`
/// when there is nothing to expand.
pub fn flatten_reference_unions(ty: &Type) -> Option<Type> {
    match ty {
        Type::Reference(reference) if reference_resolves_to_union(ty) => Some(reference.resolve()),
        Type::Union(union) if union.types().iter().any(reference_resolves_to_union) => {
            Some(union_type(
                union
                    .types()
                    .iter()
                    .map(|member| match member {
                        Type::Reference(reference) if reference_resolves_to_union(member) => {
                            reference.resolve()
                        }
                        other => other.clone(),
                    })
                    .collect(),
            ))
        }
        _ => None,
    }
}

fn reference_resolves_to_union(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference)
        if !reference.is_unique_symbol()
            && matches!(reference.resolve_arc().as_ref(), Type::Union(_)))
}

pub fn remove_nullish(ty: &Type) -> Type {
    match ty {
        Type::Union(union) => {
            let filtered: Vec<Type> = union
                .types()
                .iter()
                .filter(|t| !matches!(t, Type::Undefined | Type::Null | Type::Void))
                .filter(|t| !crate::type_variable::is_nullish_type_variable_intersection(t))
                .map(crate::type_variable::non_nullable_type_variable)
                .collect();
            union_type(filtered)
        }
        Type::Undefined | Type::Null | Type::Void => Type::Unknown,
        _ => crate::type_variable::non_nullable_type_variable(ty),
    }
}

pub fn union_type(types: Vec<Type>) -> Type {
    // Flatten, simplify, and dedup over *borrowed* members: the members are
    // only cloned when the result is a genuinely new canonical union (see
    // `UnionType::from_borrowed_members`) or the single surviving member.
    let mut flattened: Vec<&Type> = Vec::with_capacity(types.len());

    for ty in &types {
        match ty {
            Type::Union(union) => flattened.extend(union.types().iter()),
            other => flattened.push(other),
        }
    }

    // tsc's `getUnionType` (checker.go:26006): an `any` member is the whole
    // union, and the error type outranks a written `any`.
    if flattened.iter().any(|ty| matches!(ty, Type::ErrorType)) {
        return Type::ErrorType;
    }
    if flattened.iter().any(|ty| matches!(ty, Type::Any)) {
        return Type::Any;
    }

    // `never` is the identity element of union: `T | never` is `T`. Drop it so
    // distributive conditional results (e.g. `Exclude`) collapse cleanly. If
    // every member was `never`, the union itself is `never`.
    let had_members = !flattened.is_empty();
    flattened.retain(|ty| !matches!(ty, Type::Never));
    absorb_empty_array_members(&mut flattened);
    // Without `strictNullChecks` tsc's `addTypeToUnion` never adds `undefined`
    // or `null` beside another member: they are already in its domain.
    if !crate::strict_null_checks()
        && flattened.iter().any(|ty| !matches!(ty, Type::Null | Type::Undefined))
    {
        flattened.retain(|ty| !matches!(ty, Type::Null | Type::Undefined));
    }

    let unique = order_tuple_members(fold_boolean_literals(remove_string_literals_matched_by_templates(
        dedup_members(flattened),
    )));

    match unique.len() {
        0 if had_members => Type::Never,
        0 => Type::Unknown,
        1 => unique[0].clone(),
        _ => Type::Union(UnionType::from_borrowed_members(&unique)),
    }
}

/// `members` in the order tsgo keeps a union's constituents ([`display_order`]).
pub fn sort_union_members(members: &mut [Type]) {
    members.sort_by(display_order);
}

/// tsgo's `CompareTypes`, the order a union keeps (and prints) its members
/// in: by type-flag rank, then by the name of a named type, then by the
/// type's own data. The sort is stable, so first-seen order stands in for
/// the type ids that settle the remaining ties.
fn display_order(left: &Type, right: &Type) -> std::cmp::Ordering {
    sort_order_flags(left)
        .cmp(&sort_order_flags(right))
        .then_with(|| match (type_name_for_order(left), type_name_for_order(right)) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| compare_type_data(left, right))
}

/// `getSortOrderFlags`: the bit of tsgo's `TypeFlags` a type carries, as a
/// rank (enum-like types share `TypeFlagsEnum`).
fn sort_order_flags(ty: &Type) -> u8 {
    use crate::type_variable::DeferredType;
    match ty {
        Type::Any | Type::Unknown | Type::ErrorType => 0,
        Type::GenuineUnknown => 1,
        Type::Undefined => 2,
        Type::Null => 3,
        Type::Void => 4,
        Type::String => 5,
        Type::Number => 6,
        Type::BigInt => 7,
        Type::Symbol => 9,
        Type::StringLiteral(_) => 10,
        Type::NumberLiteral(_) => 11,
        // `boolean` is its two literals in a union.
        Type::Boolean | Type::BooleanLiteral(_) => 13,
        Type::Reference(reference) if reference.is_unique_symbol() => 14,
        Type::Reference(reference) if reference.enum_owner.is_some() => 16,
        Type::Object(object)
            if object.non_primitive && object.properties.is_empty() && !object.is_intersection =>
        {
            17
        }
        Type::Never => 18,
        Type::TypeParameter(parameter) => match crate::type_variable::deferred_type(parameter) {
            None => 19,
            Some(DeferredType::Keyof(_)) => 21,
            Some(DeferredType::TemplateLiteral { .. }) => 22,
            Some(DeferredType::StringMapping { .. }) => 23,
            Some(DeferredType::IndexedAccess { .. }) => 25,
            Some(DeferredType::Conditional(_)) => 26,
            Some(_) => 20,
        },
        ty if crate::is_template_literal_type(ty) => 22,
        ty if crate::string_mapping_parts(ty).is_some() => 23,
        Type::Reference(reference) if reference.id.starts_with("\u{0}intersection") => 28,
        Type::Object(object)
            if object.is_intersection && object.intersection_operands.as_deref().is_some_and(|operands| operands.len() > 1) =>
        {
            28
        }
        _ => 20,
    }
}

/// `getTypeNameSymbol`'s name: a type alias's, a class or interface's, a
/// type parameter's. Anonymous types have none.
fn type_name_for_order(ty: &Type) -> Option<String> {
    let head = |display: &str| {
        let head = &display[..display.find('<').unwrap_or(display.len())];
        head.rsplit('.').next().unwrap_or(head).to_string()
    };
    let is_name = |display: &str| {
        display.chars().next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
    };
    match ty {
        Type::Reference(reference)
            if reference.enum_owner.is_none() && !reference.id.starts_with('\u{0}') && !reference.is_unique_symbol() =>
        {
            is_name(&reference.display).then(|| head(&reference.display))
        }
        Type::Array(_) => Some("Array".to_string()),
        Type::Object(object) => object.alias_name.as_deref().filter(|alias| is_name(alias)).map(head),
        Type::Function(function) => function.alias_name().filter(|alias| is_name(alias)).map(head),
        Type::TypeParameter(parameter) if crate::type_variable::deferred_type(parameter).is_none() => {
            Some(parameter.name.to_string())
        }
        _ => None,
    }
}

/// The data `CompareTypes` orders same-ranked, same-named types by.
fn compare_type_data(left: &Type, right: &Type) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Type::StringLiteral(left), Type::StringLiteral(right)) => left.as_bytes().cmp(right.as_bytes()),
        (Type::NumberLiteral(left), Type::NumberLiteral(right)) => {
            let value = |literal: &crate::NumberLiteralType| literal.value.parse::<f64>().unwrap_or(f64::NAN);
            value(left).partial_cmp(&value(right)).unwrap_or(Ordering::Equal)
        }
        (Type::BooleanLiteral(left), Type::BooleanLiteral(right)) => left.cmp(right),
        (Type::Array(left), Type::Array(right)) => display_order(left, right),
        (Type::Tuple(left), Type::Tuple(right)) => left.len().cmp(&right.len()),
        (Type::Reference(left), Type::Reference(right))
            if left.enum_owner.is_some() && left.enum_owner == right.enum_owner =>
        {
            let position = |reference: &crate::TypeReference| {
                let members = reference.enum_members.as_deref()?;
                let display = &reference.display;
                let member = display.rsplit('.').next().unwrap_or(display);
                members.members.iter().position(|(name, _)| &**name == member)
            };
            match (position(left), position(right)) {
                (Some(left), Some(right)) => left.cmp(&right),
                _ => Ordering::Equal,
            }
        }
        (Type::Reference(left), Type::Reference(right)) if left.arguments.len() == right.arguments.len() => left
            .arguments
            .iter()
            .zip(right.arguments.iter())
            .map(|(left, right)| display_order(left, right))
            .find(|ordering| ordering.is_ne())
            .unwrap_or(Ordering::Equal),
        (Type::Reference(left), Type::Reference(right)) => left.arguments.len().cmp(&right.arguments.len()),
        _ => Ordering::Equal,
    }
}


/// `never[]` — the type of an empty array literal — is a subtype of every array
/// type, so tsc's subtype reduction drops it beside another array member
/// (`list || []` is `string[]`, `c ? [] : [1]` is `number[]`). Applied to every
/// union rather than only the subtype-reducing sites: both forms relate to the
/// same targets, so only the display of a written `never[] | T[]` differs.
fn absorb_empty_array_members(members: &mut Vec<&Type>) {
    // Without `strictNullChecks` the literal is `undefined[]`, which every
    // array type is equally a supertype of.
    fn is_empty_array(ty: &Type) -> bool {
        matches!(ty, Type::Array(element)
            if **element == Type::Never
                || **element == Type::Undefined && !crate::strict_null_checks())
    }
    if !members.iter().any(|ty| is_empty_array(ty)) {
        return;
    }
    if members.iter().any(|ty| matches!(ty, Type::Array(_)) && !is_empty_array(ty)) {
        members.retain(|ty| !is_empty_array(ty));
    }
}

/// tsc's `removeStringLiteralsMatchedByTemplateLiterals`: a string literal a
/// pattern template member already admits is redundant (`` `${number}` | '0' ``).
fn remove_string_literals_matched_by_templates(members: Vec<&Type>) -> Vec<&Type> {
    if !members.iter().any(|ty| matches!(ty, Type::StringLiteral(_))) {
        return members;
    }
    let templates: Vec<(Vec<&str>, Vec<&Type>)> = members
        .iter()
        .filter_map(|ty| crate::template_literal::template_literal_parts(ty))
        .filter(|(_, types)| types.iter().all(|ty| matches!(ty, Type::String | Type::Number | Type::BigInt)))
        .collect();
    if templates.is_empty() {
        return members;
    }
    let redundant: Vec<bool> = members
        .iter()
        .map(|ty| {
            matches!(ty, Type::StringLiteral(_))
                && templates.iter().any(|(texts, types)| {
                    crate::template_literal::is_type_matched_by_template_literal(ty, texts, types)
                })
        })
        .collect();
    members
        .into_iter()
        .zip(redundant)
        .filter_map(|(ty, redundant)| (!redundant).then_some(ty))
        .collect()
}

/// `true | false` is `boolean` — tsc folds the pair back into the primitive, so
/// `Equal<boolean, false | true>` holds and the union prints as `boolean`. The
/// primitive takes the first literal's position; a `boolean` member already
/// present absorbs both literals.
fn fold_boolean_literals(members: Vec<&Type>) -> Vec<&Type> {
    static BOOLEAN: Type = Type::Boolean;
    let has_true = members.iter().any(|ty| matches!(ty, Type::BooleanLiteral(true)));
    let has_false = members.iter().any(|ty| matches!(ty, Type::BooleanLiteral(false)));
    let has_boolean = members.iter().any(|ty| matches!(ty, Type::Boolean));
    if !(has_boolean || (has_true && has_false)) {
        return members;
    }
    let mut folded = Vec::with_capacity(members.len());
    let mut placed = false;
    for ty in members {
        match ty {
            Type::BooleanLiteral(_) | Type::Boolean => {
                if !placed {
                    folded.push(&BOOLEAN);
                    placed = true;
                }
            }
            other => folded.push(other),
        }
    }
    folded
}

/// TypeScript 7 orders a union's tuple constituents after everything else and
/// by ascending arity, keeping first-seen order within one arity — measured on
/// the 7.0.2 oracle (`[1, 2, 3] | [1, 2] | [] | [1]` reads back as `[[], [1],
/// [1, 2], [1, 2, 3]]` whichever way it is written, and `{ o: 1 } | [1] | { q: 2 }
/// | [2, 3]` as `[{ o: 1 }, { q: 2 }, [1], [2, 3]]`). The order is observable:
/// `UnionToTuple` peels a union one constituent at a time, so a type-level list
/// built from a union of tuples (ts-pattern's `FindUnions`) comes out in this
/// order and nothing else. Every other constituent keeps its first-seen order.
fn order_tuple_members(members: Vec<&Type>) -> Vec<&Type> {
    if members
        .iter()
        .filter(|ty| matches!(ty, Type::Tuple(_) | Type::OpenTuple(_)))
        .count()
        < 2
        && !members
            .iter()
            .any(|ty| matches!(ty, Type::Tuple(_) | Type::OpenTuple(_)))
    {
        return members;
    }
    let (mut tuples, mut others): (Vec<&Type>, Vec<&Type>) = members
        .into_iter()
        .partition(|ty| matches!(ty, Type::Tuple(_) | Type::OpenTuple(_)));
    tuples.sort_by_key(|ty| match ty {
        Type::Tuple(elements) => (elements.len(), 0),
        Type::OpenTuple(tuple) => (tuple.fixed_len(), 1),
        _ => (0, 0),
    });
    others.append(&mut tuples);
    others
}

/// Below this size, pairwise `contains` beats the per-member hashing overhead;
/// most unions the checker builds have a handful of members.
const LINEAR_DEDUP_LIMIT: usize = 16;

fn dedup_members<'a>(flattened: Vec<&'a Type>) -> Vec<&'a Type> {
    let mut unique: Vec<&'a Type> = Vec::with_capacity(flattened.len().min(64));
    if flattened.len() <= LINEAR_DEDUP_LIMIT {
        for ty in flattened {
            if !unique.iter().any(|existing| *existing == ty) {
                unique.push(ty);
            }
        }
        return unique;
    }

    // Inline-first buckets: the overflow `Vec` only allocates on a fingerprint
    // collision between distinct members (vanishingly rare with 64-bit keys),
    // so a large union's dedup costs one map allocation instead of one `Vec`
    // per distinct member.
    let mut buckets: PrehashedU64Map<(usize, Vec<usize>)> =
        PrehashedU64Map::with_capacity_and_hasher(flattened.len(), Default::default());
    for ty in flattened {
        match buckets.entry(dedup_key(ty)) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((unique.len(), Vec::new()));
                unique.push(ty);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let (first, overflow) = entry.get_mut();
                if unique[*first] == ty || overflow.iter().any(|&index| unique[index] == ty) {
                    continue;
                }
                overflow.push(unique.len());
                unique.push(ty);
            }
        }
    }
    unique
}

/// A large union's members keyed by [`dedup_key`], so repeated membership
/// probes against one union cost a hash lookup each instead of a scan.
pub(crate) struct UnionMemberIndex<'a> {
    members: &'a [Type],
    buckets: PrehashedU64Map<(usize, Vec<usize>)>,
}

impl<'a> UnionMemberIndex<'a> {
    /// `None` below [`LINEAR_DEDUP_LIMIT`], where scanning is the cheaper probe.
    pub(crate) fn for_large(members: &'a [Type]) -> Option<Self> {
        if members.len() <= LINEAR_DEDUP_LIMIT {
            return None;
        }
        let mut buckets: PrehashedU64Map<(usize, Vec<usize>)> =
            PrehashedU64Map::with_capacity_and_hasher(members.len(), Default::default());
        for (index, member) in members.iter().enumerate() {
            match buckets.entry(dedup_key(member)) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((index, Vec::new()));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => entry.get_mut().1.push(index),
            }
        }
        Some(Self { members, buckets })
    }

    pub(crate) fn contains(&self, ty: &Type) -> bool {
        self.buckets.get(&dedup_key(ty)).is_some_and(|(first, overflow)| {
            self.members[*first] == *ty || overflow.iter().any(|&index| self.members[index] == *ty)
        })
    }
}

fn dedup_key(ty: &Type) -> u64 {
    let mut hasher = FxHasher::default();
    dedup_key_into(ty, &mut hasher, 0);
    hasher.finish()
}

/// Equality-consistent structural digest of a type: `a == b` (under `Type`'s
/// `PartialEq`) implies equal digests, and the digest is never pointer-based
/// (see [`dedup_key_into`]'s invariant). Used by the checker's speculative-check
/// conflict tracking, where a digest collision only causes a spurious (sound)
/// serial recheck, never a missed conflict.
///
/// Much finer than [`dedup_key`]: that key's depth-3 cutoff, name-only object
/// hashing, and empty function arm collide deep-but-distinct instantiation
/// arguments, and in the speculative commit walk each collision falsely marks a
/// valid replay stale (its miss digest matches a published entry whose
/// arguments differ, i.e. one serial checking would also have missed).
/// Measured on tRPC: 212 of 226 stale-replay offender digests were exactly this
/// class. This walker keeps the same consistency invariant — hash only
/// (a subset of) equality-participating fields, structurally, never by pointer.
pub fn type_conflict_digest(ty: &Type) -> u64 {
    let mut hasher = FxHasher::default();
    let mut budget: u32 = FINE_KEY_NODE_BUDGET;
    fine_key_into(ty, &mut hasher, 0, &mut budget);
    hasher.finish()
}

const FINE_KEY_NODE_BUDGET: u32 = 64;
const FINE_KEY_MAX_DEPTH: u8 = 8;
/// Independent per-property budget for object members. Object equality is
/// order-independent (`IndexMap`), so each property's contribution must not
/// depend on iteration order — a shared budget would truncate different
/// properties depending on insertion order and break equality consistency.
const FINE_KEY_PROPERTY_BUDGET: u32 = 32;

thread_local! {
    /// [`fine_key_properties`] by property map and depth. Each property is
    /// walked under its own fixed budget, so the fold depends on nothing else;
    /// the `Weak` pins the map, so its address is not reused while the entry
    /// lives (and a map mutated through `Arc::make_mut` moves away from it).
    static FINE_KEY_PROPERTY_MEMO: std::cell::RefCell<
        crate::fx::FxHashMap<(usize, u8), (std::sync::Weak<crate::PropertyMap>, u64)>,
    > = std::cell::RefCell::new(Default::default());
}

const FINE_KEY_PROPERTY_MEMO_CAP: usize = 262_144;

fn fine_key_properties(properties: &Arc<crate::PropertyMap>, depth: u8) -> u64 {
    let key = (Arc::as_ptr(properties) as usize, depth);
    if let Some(combined) = FINE_KEY_PROPERTY_MEMO.with(|memo| memo.borrow().get(&key).map(|(_, combined)| *combined)) {
        return combined;
    }
    let mut combined: u64 = 0;
    for (name, property) in properties.iter() {
        let mut property_hasher = FxHasher::default();
        name.hash(&mut property_hasher);
        property.optional.hash(&mut property_hasher);
        let mut property_budget = FINE_KEY_PROPERTY_BUDGET;
        fine_key_into(&property.ty, &mut property_hasher, depth + 1, &mut property_budget);
        combined = combined.wrapping_add(property_hasher.finish());
    }
    FINE_KEY_PROPERTY_MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        if memo.len() >= FINE_KEY_PROPERTY_MEMO_CAP {
            memo.clear();
        }
        memo.insert(key, (Arc::downgrade(properties), combined));
    });
    combined
}

fn fine_key_into(ty: &Type, hasher: &mut FxHasher, depth: u8, budget: &mut u32) {
    std::mem::discriminant(ty).hash(hasher);
    if depth >= FINE_KEY_MAX_DEPTH || *budget == 0 {
        return;
    }
    *budget -= 1;
    match ty {
        Type::StringLiteral(value) => value.hash(hasher),
        Type::NumberLiteral(value) => value.value.hash(hasher),
        Type::BooleanLiteral(value) => value.hash(hasher),
        Type::Reference(reference) => {
            // `display` is excluded: `TypeReference` equality is nominal
            // (id + arguments only).
            reference.id.hash(hasher);
            reference.arguments.len().hash(hasher);
            for argument in reference.arguments.iter() {
                fine_key_into(argument, hasher, depth + 1, budget);
            }
        }
        Type::Array(element) => fine_key_into(element, hasher, depth + 1, budget),
        Type::Tuple(elements) => {
            elements.len().hash(hasher);
            for element in elements {
                fine_key_into(element, hasher, depth + 1, budget);
            }
        }
        Type::Union(union) => {
            // `list_id` participates in the derived payload equality; interned
            // structurally, it doubles as a full-depth member-list fingerprint.
            union.payload.list_id.hash(hasher);
            union.types().len().hash(hasher);
            for member in union.types() {
                fine_key_into(member, hasher, depth + 1, budget);
            }
        }
        Type::Object(object) => {
            object.properties.len().hash(hasher);
            // Order-independent map equality: fold per-property digests with a
            // commutative combiner, each walked under its own fixed budget.
            let combined = fine_key_properties(&object.properties, depth);
            combined.hash(hasher);
            // `call_signature` and `construct_signature` do participate in
            // `ObjectType` equality but are left unhashed anyway: omitting an
            // equality field only costs collisions, which are safe here, while
            // walking two more signatures per object is not worth it.
            // `is_intersection` is genuinely excluded from equality and must
            // stay unhashed.
            match &object.number_index_type {
                Some(index_type) => {
                    1u8.hash(hasher);
                    fine_key_into(index_type, hasher, depth + 1, budget);
                }
                None => 0u8.hash(hasher),
            }
            match &object.string_index_type {
                Some(index) => {
                    1u8.hash(hasher);
                    fine_key_into(index, hasher, depth + 1, budget);
                }
                None => 0u8.hash(hasher),
            }
        }
        Type::Function(function) => {
            let payload = &function.payload;
            payload.parameter_list_id.hash(hasher);
            payload.parameters.len().hash(hasher);
            for parameter in payload.parameters.iter() {
                fine_key_into(parameter, hasher, depth + 1, budget);
            }
            fine_key_into(&payload.return_type, hasher, depth + 1, budget);
            payload.is_variadic.hash(hasher);
            payload.required_parameter_count.hash(hasher);
        }
        _ => {}
    }
}

/// Coarse structural key for union dedup. Invariant: `a == b` (under `Type`'s
/// `PartialEq`) must imply `dedup_key(a) == dedup_key(b)`, so only fields that
/// participate in equality are hashed, and always structurally — never by
/// pointer, since `ObjectType`/`FunctionType`/`UnionType` equality accepts
/// structurally-equal values behind distinct `Arc`s.
///
/// Hashing *fewer* equality fields is always safe — it only produces
/// collisions, which [`dedup_members`] resolves with a structural compare.
/// But every colliding pair costs one such compare, so a key that collapses a
/// whole shape class puts the hashed path back at O(n^2): this arm hashed
/// nothing at all for functions, and only property *names* for objects, so a
/// union of distinct signatures — or of objects that share a property name set
/// and differ in the types — landed entirely in one bucket.
fn dedup_key_into(ty: &Type, hasher: &mut FxHasher, depth: u8) {
    std::mem::discriminant(ty).hash(hasher);
    if depth >= 3 {
        return;
    }
    match ty {
        Type::StringLiteral(value) => value.hash(hasher),
        Type::NumberLiteral(value) => value.value.hash(hasher),
        Type::BooleanLiteral(value) => value.hash(hasher),
        Type::Reference(reference) => {
            reference.id.hash(hasher);
            reference.arguments.len().hash(hasher);
            for argument in reference.arguments.iter() {
                dedup_key_into(argument, hasher, depth + 1);
            }
        }
        Type::Array(element) => dedup_key_into(element, hasher, depth + 1),
        Type::OpenTuple(tuple) => {
            tuple.leading.len().hash(hasher);
            tuple.trailing.len().hash(hasher);
            for element in tuple.leading.iter().chain(tuple.trailing.iter()) {
                dedup_key_into(element, hasher, depth + 1);
            }
            dedup_key_into(&tuple.rest, hasher, depth + 1);
        }
        Type::Tuple(elements) => {
            elements.len().hash(hasher);
            for element in elements {
                dedup_key_into(element, hasher, depth + 1);
            }
        }
        Type::Union(union) => {
            union.types().len().hash(hasher);
            for member in union.types() {
                dedup_key_into(member, hasher, depth + 1);
            }
        }
        Type::Object(object) => {
            object.properties.len().hash(hasher);
            // IndexMap equality is order-independent, so each property's
            // contribution is mixed with a commutative combiner rather than
            // hashed in iteration order. Each property walks under its own
            // fresh hasher for the same reason.
            let mut properties: u64 = 0;
            for (name, property) in object.properties.iter() {
                let mut property_hasher = FxHasher::default();
                name.hash(&mut property_hasher);
                property.optional.hash(&mut property_hasher);
                dedup_key_into(&property.ty, &mut property_hasher, depth + 1);
                properties = properties.wrapping_add(property_hasher.finish());
            }
            properties.hash(hasher);
            match &object.number_index_type {
                Some(index) => {
                    1u8.hash(hasher);
                    dedup_key_into(index, hasher, depth + 1);
                }
                None => 0u8.hash(hasher),
            }
            match &object.string_index_type {
                Some(index) => {
                    1u8.hash(hasher);
                    dedup_key_into(index, hasher, depth + 1);
                }
                None => 0u8.hash(hasher),
            }
        }
        Type::Function(function) => {
            // Every field here participates in `FunctionTypePayload`'s derived
            // equality. `id`, `parameter_names`, `type_parameter_head` and
            // `alias_name` live on the handle, not the payload, and must stay
            // unhashed: they do not participate, so hashing one would make the
            // key finer than equality and dedup would keep a duplicate.
            let payload = &function.payload;
            payload.parameters.len().hash(hasher);
            payload.is_variadic.hash(hasher);
            payload.required_parameter_count.hash(hasher);
            for parameter in payload.parameters.iter() {
                dedup_key_into(parameter, hasher, depth + 1);
            }
            dedup_key_into(&payload.return_type, hasher, depth + 1);
        }
        _ => {}
    }
}

pub fn snapshot_union_type_counters() -> UnionTypeCounters {
    UnionTypeCounters {
        union_type_payload_alloc_count: UNION_TYPE_PAYLOAD_ALLOC_COUNT.load(Ordering::Relaxed),
        union_type_payload_deep_clone_count: UNION_TYPE_PAYLOAD_DEEP_CLONE_COUNT
            .load(Ordering::Relaxed),
        union_type_handle_copy_count: UNION_TYPE_HANDLE_COPY_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_expression_inference_count:
            UNION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_call_resolution_count: UNION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_property_call_resolution_count:
            UNION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_function_body_setup_count:
            UNION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_return_checking_count: UNION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_expected_type_count: UNION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_symbol_table_count: UNION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_module_export_count: UNION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_scope_or_context_count: UNION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT
            .load(Ordering::Relaxed),
        union_type_copy_from_substitution_unchanged_count:
            UNION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_substitution_changed_count:
            UNION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT.load(Ordering::Relaxed),
        union_type_copy_from_diagnostic_formatting_count:
            UNION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT.load(Ordering::Relaxed),
        union_type_copy_unattributed_count: UNION_TYPE_COPY_UNATTRIBUTED_COUNT
            .load(Ordering::Relaxed),
    }
}

pub(crate) fn record_union_type_payload_alloc_count() {
    UNION_TYPE_PAYLOAD_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_union_type_payload_deep_clone_count() {
    UNION_TYPE_PAYLOAD_DEEP_CLONE_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_union_type_handle_copy_count() {
    UNION_TYPE_HANDLE_COPY_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn record_union_type_copy_count_for_current_reason() {
    match current_type_copy_reason() {
        TypeCopyReason::ExpressionInference => {
            UNION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::CallResolution => {
            UNION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::PropertyCallResolution => {
            UNION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::FunctionBodySetup => {
            UNION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ReturnChecking => {
            UNION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ExpectedType => {
            UNION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SymbolTable => {
            UNION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ModuleExport => {
            UNION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ScopeOrContext => {
            UNION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SubstitutionUnchanged => {
            UNION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SubstitutionChanged => {
            UNION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::DiagnosticFormatting => {
            UNION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::Other => {
            UNION_TYPE_COPY_UNATTRIBUTED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FunctionType, NumberLiteralType, Type};

    #[test]
    fn literal_union_dedupes_exact_duplicates() {
        let ty = union_type(vec![
            Type::StringLiteral("ok".to_string()),
            Type::StringLiteral("ok".to_string()),
        ]);

        assert_eq!(ty, Type::StringLiteral("ok".to_string()));
    }

    #[test]
    fn literal_union_display_stable() {
        let ty = union_type(vec![
            Type::StringLiteral("ok".to_string()),
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            }),
            Type::BooleanLiteral(true),
        ]);

        assert_eq!(ty.name(), r#""ok" | 1 | true"#);
    }

    #[test]
    fn literal_union_dedupes_string_literals() {
        let ty = union_type(vec![
            Type::StringLiteral("idle".to_string()),
            Type::StringLiteral("idle".to_string()),
        ]);

        assert_eq!(ty, Type::StringLiteral("idle".to_string()));
    }

    #[test]
    fn literal_union_dedupes_number_literals() {
        let ty = union_type(vec![
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            }),
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            }),
        ]);

        assert_eq!(
            ty,
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            })
        );
    }

    #[test]
    fn literal_union_dedupes_boolean_literals() {
        let ty = union_type(vec![Type::BooleanLiteral(true), Type::BooleanLiteral(true)]);

        assert_eq!(ty, Type::BooleanLiteral(true));
    }

    #[test]
    fn literal_union_preserves_first_seen_order() {
        let ty = union_type(vec![
            Type::StringLiteral("idle".to_string()),
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            }),
            Type::BooleanLiteral(true),
            Type::StringLiteral("idle".to_string()),
        ]);

        assert_eq!(ty.name(), r#""idle" | 1 | true"#);
    }

    #[test]
    fn literal_union_does_not_collapse_to_primitive() {
        let ty = union_type(vec![Type::StringLiteral("ok".to_string()), Type::String]);

        assert!(matches!(ty, Type::Union(_)));
        assert_eq!(ty.name(), r#"string | "ok""#);
    }

    #[test]
    fn literal_union_does_not_collapse_string_literal_with_string() {
        let ty = union_type(vec![Type::StringLiteral("idle".to_string()), Type::String]);

        assert!(matches!(ty, Type::Union(_)));
        assert_eq!(ty.name(), r#"string | "idle""#);
    }

    #[test]
    fn literal_union_does_not_collapse_number_literal_with_number() {
        let ty = union_type(vec![
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string(),
            }),
            Type::Number,
        ]);

        assert!(matches!(ty, Type::Union(_)));
        assert_eq!(ty.name(), "number | 1");
    }

    #[test]
    fn literal_union_folds_boolean_literal_into_boolean() {
        // `boolean` is `true | false`, so tsc reduces the union to `boolean`.
        let ty = union_type(vec![Type::BooleanLiteral(true), Type::Boolean]);

        assert_eq!(ty, Type::Boolean);
        assert_eq!(ty.name(), "boolean");
    }

    #[test]
    fn union_drops_never_members() {
        let ty = union_type(vec![Type::StringLiteral("b".to_string()), Type::Never]);
        assert_eq!(ty, Type::StringLiteral("b".to_string()));
    }

    #[test]
    fn tuple_members_sort_after_others_by_ascending_arity() {
        let three = Type::Tuple(vec![Type::Number, Type::Number, Type::Number]);
        let one = Type::Tuple(vec![Type::Number]);
        let empty = Type::Tuple(Vec::new());
        let object = Type::Object(crate::ObjectType::new(crate::PropertyMap::default(), None));
        let Type::Union(union) = union_type(vec![
            three.clone(),
            object.clone(),
            empty.clone(),
            one.clone(),
        ]) else {
            panic!("expected a union");
        };
        assert_eq!(union.types(), &[object, empty, one, three]);
    }

    #[test]
    fn union_of_only_never_is_never() {
        let ty = union_type(vec![Type::Never, Type::Never]);
        assert_eq!(ty, Type::Never);
    }

    #[test]
    fn empty_union_stays_unknown() {
        assert_eq!(union_type(vec![]), Type::Unknown);
    }

    #[test]
    fn literal_union_with_any_collapses_to_any() {
        let ty = union_type(vec![Type::StringLiteral("ok".to_string()), Type::Any]);

        assert_eq!(ty, Type::Any);
    }

    #[test]
    fn large_union_dedup_matches_linear_semantics() {
        // Exceeds LINEAR_DEDUP_LIMIT so the hashed path runs; every member is
        // duplicated once and first-seen order must survive.
        let members: Vec<Type> = (0..40)
            .map(|index| Type::StringLiteral(format!("member-{index}")))
            .collect();
        let mut doubled = members.clone();
        doubled.extend(members.clone());

        let ty = union_type(doubled);
        match &ty {
            Type::Union(union) => assert_eq!(union.types(), members.as_slice()),
            other => panic!("expected union, got {other:?}"),
        }
    }

    #[test]
    fn large_union_dedups_structurally_equal_objects() {
        use crate::{ObjectProperty, ObjectType, PropertyMap};
        use std::sync::Arc;

        // Structurally equal objects behind distinct Arcs must still dedup on
        // the hashed path, matching `Type::eq`.
        let object = || {
            let mut properties = PropertyMap::default();
            properties.insert("value".into(), ObjectProperty::required(Type::String));
            Type::Object(ObjectType {
                properties: Arc::new(properties),
                property_map_id: None,
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                alias_name: None,
                alias_id: None,
                construct_signature: None,
                call_signature: None,
                is_intersection: false,
                synthetic_open_index: false,
                keyed_indexes: None,
                index_parameter_names: None,
                non_primitive: false,
                without_inferable_index: false,
                intersection_operands: None,
            })
        };

        let mut members: Vec<Type> = (0..40)
            .map(|index| Type::StringLiteral(format!("member-{index}")))
            .collect();
        members.push(object());
        members.push(object());

        let ty = union_type(members);
        match &ty {
            Type::Union(union) => {
                assert_eq!(union.types().len(), 41);
                assert!(matches!(union.types().last(), Some(Type::Object(_))));
            }
            other => panic!("expected union, got {other:?}"),
        }
    }

    /// A union of distinct function types used to land every member in one
    /// `dedup_key` bucket (the arm hashed nothing but the discriminant), so the
    /// hashed path degenerated to an O(n^2) sweep of structural compares. The
    /// dedup *result* was always correct; this pins that it stays correct now
    /// that the key discriminates, and `distinct_function_members_do_not_collide`
    /// pins the bucketing itself.
    #[test]
    fn large_union_of_distinct_functions_keeps_every_member() {
        let members: Vec<Type> = (0..40)
            .map(|index| {
                Type::Function(FunctionType::new(
                    vec![Type::NumberLiteral(NumberLiteralType {
                        value: index.to_string(),
                    })],
                    Type::String,
                    false,
                    1,
                ))
            })
            .collect();
        let mut doubled = members.clone();
        doubled.extend(members.clone());

        let ty = union_type(doubled);
        match &ty {
            Type::Union(union) => assert_eq!(union.types(), members.as_slice()),
            other => panic!("expected union, got {other:?}"),
        }
    }

    /// The consistency direction of the invariant: two structurally equal
    /// signatures behind distinct handles must still share a key, or dedup would
    /// keep a duplicate.
    #[test]
    fn equal_functions_share_a_dedup_key() {
        let signature = || {
            Type::Function(FunctionType::new(
                vec![Type::String],
                Type::Number,
                false,
                1,
            ))
        };
        assert_eq!(signature(), signature());
        assert_eq!(dedup_key(&signature()), dedup_key(&signature()));
    }

    /// The discrimination direction: distinct signatures must land in distinct
    /// buckets. Without this the hashed path is quadratic in the union's width.
    #[test]
    fn distinct_function_members_do_not_collide() {
        let by_return = Type::Function(FunctionType::new(vec![Type::String], Type::Number, false, 1));
        let by_parameter =
            Type::Function(FunctionType::new(vec![Type::Number], Type::Number, false, 1));
        let by_arity = Type::Function(FunctionType::new(vec![], Type::Number, false, 0));

        assert_ne!(dedup_key(&by_return), dedup_key(&by_parameter));
        assert_ne!(dedup_key(&by_return), dedup_key(&by_arity));
    }

    /// The bucket-occupancy property the fix is actually for, asserted directly
    /// rather than through a timing: every distinct signature in a wide union
    /// must get its own key. With the old arm all 64 of these shared one bucket,
    /// so inserting them cost 64*63/2 structural compares; a machine-independent
    /// count is the honest way to pin that, since wall-clock here is dominated
    /// by whatever else the host is running.
    #[test]
    fn wide_union_of_signatures_occupies_distinct_buckets() {
        let members: Vec<Type> = (0..64)
            .map(|index| {
                Type::Function(FunctionType::new(
                    vec![Type::StringLiteral(format!("p{index}"))],
                    Type::Number,
                    false,
                    1,
                ))
            })
            .collect();

        let keys: std::collections::HashSet<u64> = members.iter().map(dedup_key).collect();
        assert_eq!(
            keys.len(),
            members.len(),
            "each distinct signature must land in its own dedup bucket"
        );
    }

    /// The same property for the object shape class: a discriminated union whose
    /// members share a property name set and differ in the discriminant's type.
    #[test]
    fn wide_union_of_same_shaped_objects_occupies_distinct_buckets() {
        use crate::{ObjectProperty, ObjectType, PropertyMap};
        use std::sync::Arc;

        let members: Vec<Type> = (0..64)
            .map(|index| {
                let mut properties = PropertyMap::default();
                properties.insert(
                    "kind".into(),
                    ObjectProperty::required(Type::StringLiteral(format!("k{index}"))),
                );
                properties.insert("value".into(), ObjectProperty::required(Type::Number));
                Type::Object(ObjectType {
                    properties: Arc::new(properties),
                    property_map_id: None,
                    string_index_type: None,
                    number_index_type: None,
                    string_index_readonly: false,
                    number_index_readonly: false,
                    alias_name: None,
                    alias_id: None,
                    construct_signature: None,
                    call_signature: None,
                    is_intersection: false,
                    synthetic_open_index: false,
                    keyed_indexes: None,
                    index_parameter_names: None,
                    non_primitive: false,
                    without_inferable_index: false,
                intersection_operands: None,
                })
            })
            .collect();

        let keys: std::collections::HashSet<u64> = members.iter().map(dedup_key).collect();
        assert_eq!(
            keys.len(),
            members.len(),
            "each discriminant literal must land in its own dedup bucket"
        );
    }

    /// Objects that share a property *name* set and differ only in the property
    /// types were the other collapsed shape class.
    #[test]
    fn objects_differing_only_in_property_types_do_not_collide() {
        use crate::{ObjectProperty, ObjectType, PropertyMap};
        use std::sync::Arc;

        let object = |ty: Type| {
            let mut properties = PropertyMap::default();
            properties.insert("value".into(), ObjectProperty::required(ty));
            Type::Object(ObjectType {
                properties: Arc::new(properties),
                property_map_id: None,
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                alias_name: None,
                alias_id: None,
                construct_signature: None,
                call_signature: None,
                is_intersection: false,
                synthetic_open_index: false,
                keyed_indexes: None,
                index_parameter_names: None,
                non_primitive: false,
                without_inferable_index: false,
                intersection_operands: None,
            })
        };

        assert_ne!(
            dedup_key(&object(Type::String)),
            dedup_key(&object(Type::Number))
        );
        assert_eq!(
            dedup_key(&object(Type::String)),
            dedup_key(&object(Type::String))
        );
    }

    /// Property order must not reach the key: `PropertyMap` equality is
    /// order-independent, so two equal objects built in different insertion
    /// orders have to hash the same or dedup keeps a duplicate.
    #[test]
    fn property_order_does_not_change_the_dedup_key() {
        use crate::{ObjectProperty, ObjectType, PropertyMap};
        use std::sync::Arc;

        let object = |reversed: bool| {
            let mut properties = PropertyMap::default();
            let mut entries = vec![
                ("a", Type::String),
                ("b", Type::Number),
                ("c", Type::Boolean),
            ];
            if reversed {
                entries.reverse();
            }
            for (name, ty) in entries {
                properties.insert(name.into(), ObjectProperty::required(ty));
            }
            Type::Object(ObjectType {
                properties: Arc::new(properties),
                property_map_id: None,
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                alias_name: None,
                alias_id: None,
                construct_signature: None,
                call_signature: None,
                is_intersection: false,
                synthetic_open_index: false,
                keyed_indexes: None,
                index_parameter_names: None,
                non_primitive: false,
                without_inferable_index: false,
                intersection_operands: None,
            })
        };

        assert_eq!(object(false), object(true));
        assert_eq!(dedup_key(&object(false)), dedup_key(&object(true)));
    }

    #[test]
    fn literal_union_flattens_nested_literal_unions() {
        let ty = union_type(vec![
            Type::Union(UnionType::new(vec![
                Type::StringLiteral("idle".to_string()),
                Type::NumberLiteral(NumberLiteralType {
                    value: "1".to_string(),
                }),
            ])),
            Type::Union(UnionType::new(vec![
                Type::BooleanLiteral(true),
                Type::StringLiteral("idle".to_string()),
            ])),
        ]);

        assert_eq!(ty.name(), r#""idle" | 1 | true"#);
    }
}
