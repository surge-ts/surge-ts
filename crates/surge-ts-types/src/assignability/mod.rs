use std::sync::Arc;

use crate::{FunctionType, ObjectType, Type};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectAssignabilityFailure {
    MissingProperty {
        property_name: String,
    },
    PropertyTypeMismatch {
        property_name: String,
        source_type: Type,
        target_type: Type,
    },
    /// A `private` member related to anything but itself, or a `protected`
    /// one to or from a public member (`propertyRelatedTo`).
    AccessibilityMismatch {
        property_name: String,
    },
}

thread_local! {
    /// Recursion depth of the current `is_assignable_to` evaluation. Lazy nominal
    /// `Type::Reference`s can form cyclic structural graphs (interface A whose member
    /// resolves to B whose member resolves back to A); structural comparison would
    /// otherwise recurse forever following them. The bound breaks such a cycle by
    /// treating the over-deep comparison as assignable — the coinductive choice tsc
    /// makes with its relation-in-progress set.
    static ASSIGNABILITY_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };

    /// Object pairs whose assignability is being decided on the current stack,
    /// keyed by their shared property-map `Arc` pointers (stable across the
    /// memoized `resolve()` of a reference). Mutually-recursive library object
    /// graphs (e.g. DOM `Request`/`RequestInit`, whose members cycle back) make
    /// `object_assignability_failure` re-ask the *same* pair while it is still in
    /// progress; without this the comparison re-descends the cycle from every
    /// sibling property, which is exponential. Re-asking an in-progress pair
    /// answers `true` coinductively — the same answer the depth bound gives, but
    /// at the cycle edge instead of after 200 redundant levels. Cleared when the
    /// outermost `is_assignable_to` returns so a freed `Arc` pointer can never be
    /// reused for a stale entry.
    static OBJECT_ASSIGNABILITY_IN_PROGRESS: std::cell::RefCell<crate::fx::FxHashSet<(usize, usize)>> =
        std::cell::RefCell::new(crate::fx::FxHashSet::default());

    /// Completed-result memo for the current outermost `is_assignable_to` query,
    /// keyed on stable `Arc` identities of both sides. The in-progress set above
    /// only catches cycles; on acyclic DAG-shaped types (the same sub-pair
    /// reached from many sibling properties or union arms) every path re-ran the
    /// full comparison, which is exponential in nesting depth. Cleared with the
    /// in-progress set when the outermost call returns, so a freed `Arc` pointer
    /// can never alias a stale entry.
    static ASSIGNABILITY_RELATION_CACHE: std::cell::RefCell<crate::fx::FxHashMap<(u8, RelationKey, RelationKey), bool>> =
        std::cell::RefCell::new(crate::fx::FxHashMap::default());

    /// Bumped whenever a comparison is answered by assumption (depth-cap or
    /// in-progress coinductive `true`) rather than by inspection. A result whose
    /// subtree consumed an assumption is only valid under that assumption, so it
    /// must not be memoized as definitive — mirroring tsc's `Ternary.Maybe`
    /// handling in its relation cache.
    static ASSIGNABILITY_ASSUMPTION_EVENTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Comparisons made under the current outermost `is_assignable_to` query.
    /// The depth bound alone does not bound *work*: a union of generic classes
    /// whose members reference the union again (ast-types' `Type<T>`) fans out
    /// at every level, and every assumption on the way voids the relation memo,
    /// so the comparison is exponential below the cap. Past the budget the
    /// remaining comparisons are answered by assumption, the same coinductive
    /// answer the cap gives.
    static ASSIGNABILITY_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Apparent shapes built for one comparison (see
    /// [`array_like_apparent_object`]). Their member types key the relation
    /// memo by payload address, so they are held until the outermost query
    /// ends: a freed member could otherwise hand its address to the next
    /// shape's and be answered from the stale entry.
    static SYNTHESIZED_TARGETS: std::cell::RefCell<Vec<Type>> =
        const { std::cell::RefCell::new(Vec::new()) };

    /// The relation the current outermost query is being decided under. Constant
    /// for the duration of one query; [`is_comparable_to`] sets and restores it.
    static CURRENT_RELATION: std::cell::Cell<Relation> =
        const { std::cell::Cell::new(Relation::Assignable) };
}

fn current_relation() -> Relation {
    CURRENT_RELATION.with(std::cell::Cell::get)
}

/// Which relation a comparison is being decided under, mirroring tsc's
/// `assignableRelation` / `comparableRelation`. The engine is a set of free
/// functions sharing thread-local state (depth, in-progress set, memo), so the
/// relation lives alongside them rather than being threaded through every
/// signature — the same role `Relater.relation` plays in `relater.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Assignable,
    /// tsc's comparable relation: laxer than assignability, and the relation an
    /// `x as T` conversion is checked under. The difference that matters is a
    /// union *source*, which is related when *some* constituent is rather than
    /// every one (`relater.go`, `someTypeRelatedToType` vs
    /// `eachTypeRelatedToType`).
    Comparable,
}

const MAX_ASSIGNABILITY_DEPTH: u32 = 200;
const MAX_ASSIGNABILITY_STEPS: u64 = 250_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RelationKey {
    tag: u8,
    parts: [usize; 6],
}

/// Stable identity for memoizing a comparison side. Every field that can change
/// the assignability verdict must contribute (properties, both index signatures,
/// call and construct signatures, `alias_id` for the nominal fast path); types without a
/// shared-`Arc` identity return `None` and are simply not memoized.
fn relation_key(ty: &Type) -> Option<RelationKey> {
    match ty {
        // Property maps are interned, so every memberless object shares one:
        // an intersection's operands (`T & object`, `T & 1`) and `object`'s
        // non-primitive marker are what tell such types apart.
        Type::Object(object)
            if object.keyed_indexes.is_some()
                || object.intersection_operands.as_deref().is_some_and(|operands| !operands.is_empty()) =>
        {
            None
        }
        Type::Object(object) => Some(RelationKey {
            tag: if object.non_primitive { 7 } else { 1 },
            parts: [
                Arc::as_ptr(&object.properties) as usize,
                object
                    .string_index_type
                    .as_ref()
                    .map_or(0, |index| Arc::as_ptr(index) as usize),
                object
                    .number_index_type
                    .as_ref()
                    .map_or(0, |index| Arc::as_ptr(index) as usize),
                object
                    .call_signature()
                    .map_or(0, |signature| signature.payload_address() ^ written_shape_address(signature)),
                object.construct_signature().map_or(0, |signature| {
                    signature.payload_address()
                        ^ written_shape_address(signature)
                        ^ usize::from(signature.construct_modifiers().bits())
                }),
                object
                    .alias_id
                    .as_ref()
                    .map_or(0, |id| id.as_ref().as_ptr() as usize),
            ],
        }),
        Type::Union(union) => Some(RelationKey {
            tag: 2,
            parts: [union.payload_address(), 0, 0, 0, 0, 0],
        }),
        Type::Function(function) => Some(RelationKey {
            tag: 3,
            parts: [
                function.payload_address(),
                written_shape_address(function),
                function
                    .type_predicate()
                    .map_or(0, |predicate| predicate as *const crate::TypePredicate as usize),
                usize::from(function.is_method_declaration()),
                function.this_type_address(),
                function.alias_instantiation_address(),
            ],
        }),
        _ => None,
    }
}

/// A signature with a written shape relates through it, and every signature
/// erased to the same payload has a shape of its own, or none.
fn written_shape_address(function: &FunctionType) -> usize {
    function
        .generic_shape()
        .map_or(0, |shape| shape as *const crate::GenericSignatureShape as usize)
}

fn record_assignability_assumption() {
    ASSIGNABILITY_ASSUMPTION_EVENTS.with(|events| events.set(events.get() + 1));
}

/// Widest target union a discriminated distribution is attempted against. The
/// cap keeps a pathological union from turning one failed comparison into a
/// quadratic sweep.
const MAX_DISCRIMINATED_UNION_MEMBERS: usize = 32;

/// A tuple's discriminants are its element properties `"0"`, `"1"`, … and its
/// `length`, so `["a" | "b", number]` relates to `["a", number] | ["b",
/// number]` as an object with a discriminant property does.
fn discriminated_tuple_union_assignable(elements: &[Type], to_union: &crate::UnionType) -> bool {
    let as_object = |elements: &[Type]| {
        let mut properties = crate::PropertyMap::default();
        for (index, element) in elements.iter().enumerate() {
            properties.insert(Arc::from(index.to_string()), crate::ObjectProperty::required(element.clone()));
        }
        properties.insert(Arc::from("length"), crate::ObjectProperty::required(crate::tuple_length_type(elements)));
        Type::Object(ObjectType::new(properties, None))
    };
    let mut members = Vec::with_capacity(to_union.types().len());
    for member in to_union.types() {
        match member.peeled() {
            Type::Tuple(member_elements) => members.push(as_object(&member_elements)),
            _ => return false,
        }
    }
    let Type::Union(member_union) = crate::union_type(members) else {
        return false;
    };
    discriminated_union_assignable(&as_object(elements), &member_union)
}

/// tsc's `typeRelatedToDiscriminatedType`: an object source is related to a
/// union target when every combination of the values its discriminant
/// properties can hold picks target members, and each picked member accepts
/// the rest of the source. `{ done: boolean; value: T }` relates to
/// `{ done: true; value: T } | { done: false; value: T }` although neither
/// member accepts it whole.
///
/// A discriminant is a property the target's members declare with differing
/// types, at least one of them a unit type (a literal, `true`/`false`,
/// `undefined` or `null`). More than 25 combinations is too complex, as in tsc.
///
/// Runs only after the plain member-wise check already failed.
fn discriminated_union_assignable(from: &Type, to_union: &crate::UnionType) -> bool {
    if let Type::Tuple(elements) = from {
        return discriminated_tuple_union_assignable(elements, to_union);
    }
    let Type::Object(from_object) = from else {
        return false;
    };
    let members = to_union.types();
    if members.len() > MAX_DISCRIMINATED_UNION_MEMBERS {
        return false;
    }
    // An enum is the union of its members, each a unit type.
    let enum_union = |ty: &Type| match ty {
        Type::Reference(reference) if reference.enum_owner.is_some() => match reference.resolve() {
            resolved @ Type::Union(_) => Some(resolved),
            _ => None,
        },
        _ => None,
    };
    let is_unit = |ty: &Type| {
        matches!(
            ty,
            Type::StringLiteral(_)
                | Type::NumberLiteral(_)
                | Type::BooleanLiteral(_)
                | Type::Boolean
                | Type::Undefined
                | Type::Null
        ) || matches!(ty, Type::Reference(reference) if reference.enum_owner.is_some()
            && matches!(reference.resolve(), Type::StringLiteral(_) | Type::NumberLiteral(_)))
    };
    let has_unit_part = |ty: &Type| match enum_union(ty).as_ref().unwrap_or(ty) {
        Type::Union(union) => union.types().iter().any(is_unit),
        other => is_unit(other),
    };
    let distributed = |ty: &Type| -> Vec<Type> {
        let mut values = Vec::new();
        let mut push = |ty: &Type| match ty {
            Type::Boolean => {
                values.push(Type::BooleanLiteral(true));
                values.push(Type::BooleanLiteral(false));
            }
            other => values.push(other.clone()),
        };
        match enum_union(ty).as_ref().unwrap_or(ty) {
            Type::Union(union) => union.types().iter().for_each(&mut push),
            other => push(other),
        }
        values
    };

    // Answered from the source alone first: peeling every member is the
    // expensive part, and most failed object-to-union comparisons have no
    // candidate discriminant at all.
    if !from_object.properties.values().any(|property| has_unit_part(&property.ty)) {
        return false;
    }
    let peeled_members: Vec<Type> = members.iter().map(Type::peeled).collect();
    let member_property = |member: &Type, name: &str| -> Option<Type> {
        let Type::Object(object) = member else {
            return None;
        };
        let property = object.properties.get(name)?;
        Some(if property.is_optional() {
            crate::union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        })
    };

    let mut discriminants: Vec<(&str, Vec<Type>)> = Vec::new();
    let mut combinations = 1usize;
    for (name, property) in from_object.properties.iter() {
        let declared: Vec<Type> = peeled_members
            .iter()
            .filter_map(|member| member_property(member, name))
            .collect();
        let uniform = declared.windows(2).all(|pair| pair[0] == pair[1]);
        if declared.is_empty() || uniform || !declared.iter().any(has_unit_part) {
            continue;
        }
        let values = distributed(&property.ty);
        combinations = combinations.saturating_mul(values.len());
        if combinations > 25 || values.is_empty() {
            return false;
        }
        discriminants.push((name.as_ref(), values));
    }
    if discriminants.is_empty() {
        return false;
    }

    let mut matching: Vec<usize> = Vec::new();
    for index in 0..combinations {
        let mut combination = Vec::with_capacity(discriminants.len());
        let mut n = index;
        for (_, values) in discriminants.iter().rev() {
            combination.push(&values[n % values.len()]);
            n /= values.len();
        }
        combination.reverse();
        let mut has_match = false;
        for (member_index, member) in peeled_members.iter().enumerate() {
            let fits = discriminants.iter().zip(&combination).all(|((name, _), value)| {
                member_property(member, name).is_some_and(|declared| is_assignable_to(value, &declared))
            });
            if fits {
                has_match = true;
                if !matching.contains(&member_index) {
                    matching.push(member_index);
                }
            }
        }
        if !has_match {
            return false;
        }
    }

    let excluded: Vec<&str> = discriminants.iter().map(|(name, _)| *name).collect();
    let without_discriminants = |object: &ObjectType| {
        let mut properties = (*object.properties).clone();
        for name in &excluded {
            properties.shift_remove(*name);
        }
        let mut stripped = ObjectType::new(properties, object.string_index_type.as_deref().cloned())
            .with_number_index_type(object.number_index_type.as_deref().cloned());
        if let Some(signature) = object.call_signature() {
            stripped = stripped.with_call_signature(signature.clone());
        }
        if let Some(signature) = object.construct_signature() {
            stripped = stripped.with_construct_signature(signature.clone());
        }
        Type::Object(stripped)
    };
    let source = without_discriminants(from_object);
    matching.iter().all(|&member_index| match &peeled_members[member_index] {
        Type::Object(member) => is_assignable_to(&source, &without_discriminants(member)),
        _ => false,
    })
}

/// tsc's `isTypeComparableTo`. Same engine as [`is_assignable_to`], decided
/// under the comparable relation: `x as T` is legal when the two types overlap
/// in either direction, and overlap is laxer than assignability.
///
/// Nested calls keep whatever relation the outermost query established, exactly
/// as a `Relater` carries one relation through a whole comparison.
pub fn is_comparable_to(from: &Type, to: &Type) -> bool {
    let previous = CURRENT_RELATION.with(|relation| relation.replace(Relation::Comparable));
    let result = is_assignable_to(from, to);
    CURRENT_RELATION.with(|relation| relation.set(previous));
    result
}

pub fn is_assignable_to(from: &Type, to: &Type) -> bool {
    struct DepthGuard;
    impl Drop for DepthGuard {
        fn drop(&mut self) {
            ASSIGNABILITY_DEPTH.with(|depth| {
                let next = depth.get().saturating_sub(1);
                depth.set(next);
                if next == 0 {
                    OBJECT_ASSIGNABILITY_IN_PROGRESS.with(|set| set.borrow_mut().clear());
                    ASSIGNABILITY_RELATION_CACHE.with(|cache| cache.borrow_mut().clear());
                    ASSIGNABILITY_STEPS.with(|steps| steps.set(0));
                    SYNTHESIZED_TARGETS.with(|targets| targets.borrow_mut().clear());
                }
            });
        }
    }
    let depth = ASSIGNABILITY_DEPTH.with(|depth| {
        let next = depth.get() + 1;
        depth.set(next);
        next
    });
    let _guard = DepthGuard;
    if depth > MAX_ASSIGNABILITY_DEPTH {
        record_assignability_assumption();
        return true;
    }
    let steps = ASSIGNABILITY_STEPS.with(|steps| {
        let next = steps.get() + 1;
        steps.set(next);
        next
    });
    if steps > MAX_ASSIGNABILITY_STEPS {
        record_assignability_assumption();
        return true;
    }

    // `isSimpleTypeRelatedTo` (relater.go:211) rejects a `never` target before
    // the rule that relates an `any` source to everything, so `any` and tsc's
    // error type (an `any`) do not relate to `never`. surge's degradation
    // sentinel is `Type::Unknown`, which stays permissive.
    if matches!(from, Type::Any | Type::ErrorType) && matches!(to, Type::Never) {
        return false;
    }

    // The comparable relation is mostly bidirectional: before any structural
    // step, a pair also relates when the target is simply related to the
    // source (`isRelatedTo`, relater.go:2694).
    if current_relation() == Relation::Comparable
        && !matches!(to, Type::Never)
        && is_simple_type_related_to(to, from)
    {
        return true;
    }

    // A class's polymorphic `this` is a type parameter constrained to its
    // instance: nothing but itself (and `any`, `never`) relates to it, while
    // it relates to whatever the instance does.
    if let Type::Reference(reference) = to
        && reference.is_polymorphic_this()
        && from != to
    {
        // A class body has one `this`; a member narrowing of it rebuilds the
        // instance beneath without leaving it.
        return match from {
            Type::Any | Type::Never | Type::Unknown | Type::ErrorType => true,
            Type::Reference(reference) => reference.is_polymorphic_this(),
            Type::Union(union) => union.types().iter().all(|member| is_assignable_to(member, to)),
            _ => false,
        };
    }

    // Only the outermost relation: a promise nested in a member may have been
    // collapsed to its awaited type on one side alone.
    if depth == 1
        && ((promise_reference(to) && definitely_not_thenable(from))
            || (promise_reference(from) && definitely_not_thenable(to)))
    {
        return false;
    }

    // An intersection of string patterns admits what every pattern does
    // (`eachTypeRelatedToType`); its merged object side has nothing to say.
    if let Some(operands) = pattern_intersection_operands(to)
        && operands.iter().all(|operand| {
            crate::template_literal::is_template_literal_type(operand)
                || crate::template_literal::string_mapping_parts(operand).is_some()
        })
        && matches!(from, Type::StringLiteral(_) | Type::String)
    {
        return operands.iter().all(|operand| is_assignable_to(from, operand));
    }

    if from != to
        && current_relation_admits_anything(to)
    {
        return true;
    }
    if from != to
        && let Some(related) = type_variable_related(from, to)
    {
        return related;
    }

    // Predicates and written generic constraints are handle-local; sharing an
    // erased payload does not make two signatures equivalent.
    let same_predicates = match (from, to) {
        (Type::Function(source), Type::Function(target)) => source.type_predicate() == target.type_predicate(),
        _ => true,
    };
    let same_generic_shapes = match (from, to) {
        (Type::Function(source), Type::Function(target)) => {
            written_shape_address(source) == written_shape_address(target)
        }
        _ => true,
    };
    if (from == to && same_predicates && same_generic_shapes)
        || matches!(from, Type::Any)
        || matches!(from, Type::Never)
        || matches!(to, Type::Any)
        || to.is_unknown()
        // Sentinel `Unknown` (NOT the `unknown` keyword, which is
        // `GenuineUnknown`) marks a type surge could not model — e.g.
        // `SubmitEvent.nativeEvent`, whose `NativeSubmitEvent` alias collides
        // with the enclosing declaration and degrades. tsc compares the real
        // type there, so failing on a degraded *source* turns every unmodelled
        // corner into a false-positive cascade. The same leniency already
        // applies to sentinel arguments in the same-generic fast path below.
        // tsc's `errorType` is `any`, so it flows both ways like the sentinel.
        || matches!(from, Type::Unknown | Type::ErrorType | Type::TypeParameter(_))
    {
        return true;
    }
    if matches!(from, Type::Null | Type::Undefined)
        && !crate::strict_null_checks()
        && !matches!(to, Type::Never)
    {
        return true;
    }
    if current_relation() == Relation::Assignable && has_no_common_properties(from, to) {
        return false;
    }

    // An intersection relates when some constituent does
    // (`someTypeRelatedToType`). A merged intersection keeps only its object
    // side's members, so a branded primitive (`"id" & { __brand: "id" }`) is
    // judged through the primitive operand it recorded.
    // A target intersection's weak object side still rejects a constituent
    // sharing none of its members (`isWeakType` per target member).
    let weak_surface_rejects = |operand: &Type| match to {
        Type::Object(target) if target.is_intersection => {
            let mut surface = target.clone();
            surface.is_intersection = false;
            surface.intersection_operands = None;
            has_no_common_properties(operand, &Type::Object(surface))
        }
        _ => false,
    };
    if let Type::Object(object) = from
        && object.is_intersection
        && object.intersection_operands.as_deref().is_some_and(|operands| {
            operands.iter().any(|operand| {
                !matches!(operand, Type::Reference(_) | Type::Object(_))
                    && !weak_surface_rejects(operand)
                    && is_assignable_to(operand, to)
            })
        })
    {
        return true;
    }

    // tsc lets any `number` flow into a numeric `enum` — its own
    // `Flags.A | Flags.B` is typed `number`, and the bitwise combination is the
    // normal way to build a flag argument. The reverse (a string into a string
    // enum) is rejected, which is why only the numeric marker opens this.
    // A number *literal* is not covered: it must match a member's value
    // (`isSimpleTypeRelatedTo`), which the structural comparison against the
    // enum's member union below decides — a computed member resolves to
    // `number` and so still accepts any literal. `number | 0` (from `x ?? 0`)
    // is `number` once tsc's subtype reduction drops the literal.
    if matches!(to, Type::Reference(reference) if reference.numeric_enum)
        && match from {
            Type::Number => true,
            Type::Union(union) => {
                union.types().iter().any(|member| matches!(member, Type::Number))
                    && matches!(from.base_primitive(), Some(Type::Number))
            }
            _ => false,
        }
    {
        return true;
    }

    // A `from` reference is resolved and recursed into below; computing its base
    // primitive here would force a full structural clone of the resolved type
    // (`base_primitive` peels references) only to almost always find it is not a
    // primitive. The recursion re-checks the base primitive on the resolved shape,
    // so skipping references here is behaviour-preserving and avoids the clone —
    // this is the dominant per-peel cost on conditional/mapped-type-heavy programs.
    if !matches!(from, Type::Reference(_))
        && from
            .base_primitive()
            .as_ref()
            .is_some_and(|base| base == to)
    {
        return true;
    }

    let cache_key = match (relation_key(from), relation_key(to)) {
        (Some(from_key), Some(to_key)) => {
            // The relation is part of the key: the same pair legitimately gets
            // different answers under the comparable relation, and a key that
            // omitted it would let one relation answer for the other.
            let pair = (current_relation() as u8, from_key, to_key);
            if let Some(result) =
                ASSIGNABILITY_RELATION_CACHE.with(|cache| cache.borrow().get(&pair).copied())
            {
                return result;
            }
            Some(pair)
        }
        _ => None,
    };
    let assumptions_before = ASSIGNABILITY_ASSUMPTION_EVENTS.with(std::cell::Cell::get);

    let result = assignability_arms(from, to);

    if let Some(pair) = cache_key
        && ASSIGNABILITY_ASSUMPTION_EVENTS.with(std::cell::Cell::get) == assumptions_before
    {
        ASSIGNABILITY_RELATION_CACHE.with(|cache| {
            cache.borrow_mut().insert(pair, result);
        });
    }
    result
}

/// An un-collapsed lib `Promise<T>` / `PromiseLike<T>` reference. It peels to
/// its awaited `T`, which would relate it to every plain `T`.
fn promise_reference(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    reference.enum_owner.is_none()
        && !reference.arguments.is_empty()
        && matches!(
            reference.display.split('<').next(),
            Some("Promise" | "PromiseLike")
        )
}

/// A lib `Promise<T>` / `PromiseLike<T>` reference: whether it is the
/// `PromiseLike`, and its value argument.
fn lib_promise_argument(ty: &Type) -> Option<(bool, &Type)> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    let [argument] = &*reference.arguments else {
        return None;
    };
    if reference.enum_owner.is_some() {
        return None;
    }
    match reference.display.split('<').next() {
        Some("Promise") => Some((false, argument)),
        Some("PromiseLike") => Some((true, argument)),
        _ => None,
    }
}

fn definitely_not_thenable(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(definitely_not_thenable),
        _ => false,
    }
}

/// `isSimpleTypeRelatedTo`'s last rule: under `strictNullChecks` anything is
/// assignable (and comparable) to a union holding `undefined`, `null` and the
/// empty object type — the union `unknown` stands for (`isUnknownLikeUnionType`).
fn current_relation_admits_anything(to: &Type) -> bool {
    let Type::Union(union) = to else {
        return false;
    };
    crate::strict_null_checks()
        && union.types().iter().any(|member| matches!(member, Type::Undefined))
        && union.types().iter().any(|member| matches!(member, Type::Null))
        && union.types().iter().any(|member| match member {
            // A type alias of `{}` names the anonymous empty object type.
            Type::Reference(_) => is_empty_anonymous_object(&member.peeled()),
            other => is_empty_anonymous_object(other),
        })
}

/// relater.go `isSimpleTypeRelatedTo` for the assignable and comparable
/// relations: the flag-level rules, decided without looking at structure.
/// Enum members and types are nominal references here, so their literal
/// values are read through the reference rather than by peeling it.
fn is_simple_type_related_to(source: &Type, target: &Type) -> bool {
    if matches!(target, Type::Any | Type::ErrorType) || matches!(source, Type::Never) {
        return true;
    }
    if target.is_unmodelled() {
        return true;
    }
    if matches!(target, Type::Never) {
        return false;
    }
    let related_by_kind = match target {
        Type::String => is_string_like(source),
        Type::Number => is_number_like(source),
        Type::BigInt => matches!(source, Type::BigInt),
        Type::Boolean => matches!(source, Type::Boolean | Type::BooleanLiteral(_)),
        Type::Symbol => {
            matches!(source, Type::Symbol)
                || matches!(source, Type::Reference(reference) if reference.is_unique_symbol())
        }
        _ => false,
    };
    if related_by_kind {
        return true;
    }
    if matches!(target, Type::StringLiteral(_) | Type::NumberLiteral(_))
        && enum_member_value(source).is_some_and(|value| value == *target)
    {
        return true;
    }
    if let (Type::Reference(source_ref), Type::Reference(target_ref)) = (source, target)
        && let (Some(source_enum), Some(target_enum)) = (&source_ref.enum_owner, &target_ref.enum_owner)
        && source_enum == target_enum
        && *source_ref.resolve_arc() == *target_ref.resolve_arc()
    {
        return true;
    }
    let strict = crate::strict_null_checks();
    if matches!(source, Type::Undefined)
        && (!strict && !is_union_or_intersection(target) || matches!(target, Type::Undefined | Type::Void))
    {
        return true;
    }
    if matches!(source, Type::Null) && (!strict && !is_union_or_intersection(target) || matches!(target, Type::Null)) {
        return true;
    }
    if matches!(target, Type::Object(object) if object.non_primitive && !object.is_intersection)
        && is_object_type(source)
    {
        return true;
    }
    if matches!(source, Type::Any | Type::ErrorType) {
        return true;
    }
    let numeric_enum = |ty: &Type| matches!(ty, Type::Reference(reference) if reference.numeric_enum);
    if matches!(source, Type::Number)
        && numeric_enum(target)
        && !matches!(&*resolved_reference(target), Type::Union(_))
    {
        return true;
    }
    if let Type::NumberLiteral(_) = source
        && numeric_enum(target)
        && match &*resolved_reference(target) {
            Type::Number => true,
            literal @ Type::NumberLiteral(_) => literal == source,
            _ => false,
        }
    {
        return true;
    }
    current_relation_admits_anything(target)
}

fn resolved_reference(ty: &Type) -> Arc<Type> {
    match ty {
        Type::Reference(reference) => reference.resolve_arc(),
        other => Arc::new(other.clone()),
    }
}

/// tsc's `StringLike` flags: `string`, a string literal (a string enum
/// member's included), a template literal or a string mapping.
fn is_string_like(ty: &Type) -> bool {
    match ty {
        Type::String | Type::StringLiteral(_) => true,
        Type::Reference(reference) => {
            crate::is_template_literal_type(ty)
                || crate::string_mapping_parts(ty).is_some()
                || reference.enum_owner.is_some()
                    && matches!(&*reference.resolve_arc(), Type::StringLiteral(_))
        }
        _ => false,
    }
}

/// tsc's `NumberLike` flags: `number`, a number literal, a numeric enum
/// member, or an enum whose members are computed (`TypeFlagsEnum`). An enum of
/// literal members is their union, which is not number-like by its flags.
fn is_number_like(ty: &Type) -> bool {
    match ty {
        Type::Number | Type::NumberLiteral(_) => true,
        Type::Reference(reference) => {
            reference.numeric_enum && !matches!(&*reference.resolve_arc(), Type::Union(_))
        }
        _ => false,
    }
}

/// The literal an enum member reference stands for.
fn enum_member_value(ty: &Type) -> Option<Type> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    reference.enum_owner.as_ref()?;
    match &*reference.resolve_arc() {
        literal @ (Type::StringLiteral(_) | Type::NumberLiteral(_)) => Some(literal.clone()),
        _ => None,
    }
}

fn is_union_or_intersection(ty: &Type) -> bool {
    match ty {
        Type::Union(_) => true,
        Type::Object(object) => object.is_intersection,
        Type::Reference(reference) => {
            matches!(&*reference.resolve_arc(), Type::Union(_) | Type::Object(ObjectType { is_intersection: true, .. }))
        }
        _ => false,
    }
}

/// tsc's `TypeFlagsObject`: an object, function, array or tuple type — not
/// the `object` keyword, an intersection, or a primitive a reference names.
fn is_object_type(ty: &Type) -> bool {
    match ty {
        Type::Object(object) => !object.non_primitive && !object.is_intersection,
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Reference(reference) => {
            reference.is_readonly_array()
                || reference.enum_owner.is_none()
                    && !reference.is_unique_symbol()
                    && !crate::is_template_literal_type(ty)
                    && crate::string_mapping_parts(ty).is_none()
                    && is_object_type(&reference.resolve_arc())
        }
        _ => false,
    }
}

/// tsc's `isWeakType` for a plain object: members, every one optional, and no
/// signature or index signature.
fn is_weak_object(object: &ObjectType) -> bool {
    !object.properties.is_empty()
        && object.properties.values().all(crate::ObjectProperty::is_optional)
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
        && !object.is_intersection
        && !object.synthetic_open_index
        && !object.non_primitive
}

/// tsc's `isUnitType` for a source with an apparent type to list members of:
/// a literal, an enum member or a unique symbol (`undefined` and `null` have
/// no members). `boolean` is tsc's `false | true`, two unit types whose
/// apparent type is the same, so it answers as either member does.
fn is_unit_literal(ty: &Type) -> bool {
    match ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) | Type::Boolean => true,
        Type::Reference(reference) => reference.is_unique_symbol() || enum_member_value(ty).is_some(),
        _ => false,
    }
}

fn is_empty_anonymous_object(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if object.properties.is_empty()
            && object.string_index_type.is_none()
            && object.number_index_type.is_none()
            && object.call_signature().is_none()
            && object.construct_signature().is_none()
            && !object.non_primitive
            && !object.is_intersection
            && !object.without_inferable_index)
}

/// The operands of an intersection holding a type variable of the body being
/// checked: `T & X` narrowed (memberless, see
/// `type_variable::intersect_type_variable`) or resolved (the merged members
/// of the other operands, with the variable recorded beside them).
fn type_variable_intersection_operands(ty: &Type) -> Option<&[Type]> {
    let Type::Object(object) = ty else {
        return None;
    };
    let operands = object.intersection_operands.as_deref()?;
    (object.is_intersection && operands.iter().any(Type::is_type_variable)).then_some(operands)
}

/// A memberless `T & X`: its operands are the whole type.
fn is_narrowed_type_variable(ty: &Type) -> bool {
    matches!(ty, Type::Object(object) if object.properties.is_empty() && object.string_index_type.is_none())
}

/// relater.go `getEffectiveConstraintOfIntersection`: the intersection of the
/// variable operands' constraints, examined only against a union target or
/// when an operand belongs to a disjoint domain (`V & number`, `T & {}`), and
/// then cut down by those operands too. `Some(None)` when there is none to
/// examine, `None` when surge cannot compute it.
fn effective_intersection_constraint(operands: &[Type], target_is_union: bool) -> Option<Option<Type>> {
    let mut constraints = Vec::new();
    // Without a union target or a disjoint-domain operand there is no
    // combined constraint to relate, whatever the operands' constraints.
    let has_disjoint_domain_type = operands.iter().any(|operand| {
        !matches!(operand, Type::TypeParameter(_))
            && (disjoint_domain(operand).is_some() || is_empty_anonymous_object(operand))
    });
    if !(target_is_union || has_disjoint_domain_type) {
        return Some(None);
    }
    for operand in operands {
        if let Type::TypeParameter(parameter) = operand {
            let mut constraint = crate::type_variable::active_constraint(parameter)?;
            let mut steps = 0;
            // `getConstraintOfType` is followed through type parameters,
            // index types and conditional types.
            while let Some(Type::TypeParameter(next)) = &constraint {
                if crate::type_variable::deferred_type(next).is_some_and(|kind| {
                    !matches!(
                        kind,
                        crate::type_variable::DeferredType::Keyof(_) | crate::type_variable::DeferredType::Conditional(_)
                    )
                }) || steps > 32
                {
                    return None;
                }
                constraint = crate::type_variable::active_constraint(next)?;
                steps += 1;
            }
            if let Some(constraint) = constraint {
                constraints.push(constraint);
                if target_is_union {
                    constraints.push(operand.clone());
                }
            }
        }
    }
    if constraints.is_empty() {
        return Some(None);
    }
    if has_disjoint_domain_type {
        constraints.extend(
            operands
                .iter()
                .filter(|operand| disjoint_domain(operand).is_some() || is_empty_anonymous_object(operand))
                .cloned(),
        );
    }
    intersect_constraint_types(&constraints).map(Some)
}

/// relater.go's `propertiesRelatedTo` for an intersection source and an object
/// target: failing its constituents one by one, the intersection relates
/// through the members they aggregate, a type variable contributing those of
/// its apparent type (its base constraint; none unconstrained). `None` where
/// the target's shape or an operand's apparent members are beyond what this
/// reads.
fn intersection_members_related(operands: &[Type], to: &Type) -> Option<bool> {
    let Type::Object(target) = to.peeled() else {
        return None;
    };
    if target.string_index_type.is_some()
        || target.number_index_type.is_some()
        || target.call_signature().is_some()
        || target.construct_signature().is_some()
        || target.is_intersection
    {
        return None;
    }
    let mut members = Vec::with_capacity(operands.len());
    for operand in operands {
        let apparent = match operand {
            Type::TypeParameter(parameter) => match crate::type_variable::base_constraint_or_type(operand) {
                Type::TypeParameter(_) if crate::type_variable::active_constraint(parameter) == Some(None) => continue,
                Type::TypeParameter(_) => return None,
                constraint => constraint,
            },
            other => other.clone(),
        };
        match apparent.peeled() {
            Type::Object(object) if !object.synthetic_open_index => members.push(object),
            Type::GenuineUnknown => {}
            _ => return None,
        }
    }
    for (name, property) in target.properties.iter() {
        let sources: Vec<&crate::ObjectProperty> = members.iter().filter_map(|object| object.properties.get(name)).collect();
        if sources.is_empty() {
            if property.optional {
                continue;
            }
            return Some(false);
        }
        if !property.optional && sources.iter().all(|source| source.optional) {
            return Some(false);
        }
        if !sources.iter().any(|source| is_assignable_to(&source.ty, &property.ty)) {
            return Some(false);
        }
    }
    Some(true)
}

/// tsc's `TypeFlagsDisjointDomains` partition for the types surge models as
/// primitives, plus `object` (`NonPrimitive`).
fn disjoint_domain(ty: &Type) -> Option<u8> {
    Some(match ty {
        Type::String | Type::StringLiteral(_) => 0,
        Type::Number | Type::NumberLiteral(_) => 1,
        Type::BigInt => 2,
        Type::Boolean | Type::BooleanLiteral(_) => 3,
        Type::Symbol => 4,
        Type::Undefined | Type::Void => 5,
        Type::Null => 6,
        Type::Object(object)
            if object.non_primitive && object.properties.is_empty() && !object.is_intersection =>
        {
            7
        }
        _ => return None,
    })
}

/// `getIntersectionType` over constraint types, distributed over their union
/// members. `None` when some pair of members has an intersection surge does
/// not represent.
pub(crate) fn intersect_constraint_types(types: &[Type]) -> Option<Type> {
    fn members(ty: &Type) -> Vec<Type> {
        match ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other.clone()],
        }
    }
    let (first, rest) = types.split_first()?;
    let mut result = members(first);
    for ty in rest {
        let mut next = Vec::new();
        for left in &result {
            for right in members(ty) {
                let pair = intersect_constraint_pair(left, &right)?;
                if pair != Type::Never {
                    next.push(pair);
                }
            }
        }
        result = next;
    }
    Some(if result.is_empty() { Type::Never } else { crate::union_type(result) })
}

fn intersect_constraint_pair(left: &Type, right: &Type) -> Option<Type> {
    if left == right {
        return Some(left.clone());
    }
    if matches!(left, Type::Never) || matches!(right, Type::Never) {
        return Some(Type::Never);
    }
    if matches!(left, Type::Any) || matches!(right, Type::Any) {
        return Some(Type::Any);
    }
    if matches!(left, Type::GenuineUnknown) {
        return Some(right.clone());
    }
    if matches!(right, Type::GenuineUnknown) {
        return Some(left.clone());
    }
    if generic_intersection_operands(left).is_some() || generic_intersection_operands(right).is_some() {
        return intersect_with_type_variables(left, right);
    }
    if left.is_unknown() || right.is_unknown() {
        return None;
    }
    if let (Some(left_domain), Some(right_domain)) = (disjoint_domain(left), disjoint_domain(right))
        && left_domain != right_domain
        && !(left_domain == 5 && right_domain == 5)
    {
        return Some(Type::Never);
    }
    let nullable = |ty: &Type| matches!(ty, Type::Null | Type::Undefined);
    if crate::strict_null_checks()
        && ((nullable(left) && is_object_type(right)) || (nullable(right) && is_object_type(left)))
    {
        return Some(Type::Never);
    }
    if is_assignable_to(left, right) {
        return Some(left.clone());
    }
    if is_assignable_to(right, left) {
        return Some(right.clone());
    }
    if is_unit_literal(left) && is_unit_literal(right) && disjoint_domain(left).is_some() {
        return Some(Type::Never);
    }
    None
}

thread_local! {
    static HOISTING: std::cell::RefCell<Vec<(Type, Type)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs a relation through a hoisted constraint unless the same pair is
/// already being related that way: the hoisted constraint holds the source
/// again (`T & 1 | T & 2`), and relater.go's maybe stack answers such a
/// revisit without the depth limit's assumption. `None` on a revisit.
fn with_hoisting(from: &Type, to: &Type, relate: impl FnOnce() -> bool) -> Option<bool> {
    let revisit = HOISTING.with(|stack| stack.borrow().iter().any(|(source, target)| source == from && target == to));
    if revisit {
        // Like a depth-limited step, what depends on the revisit is not cached.
        record_assignability_assumption();
        return None;
    }
    HOISTING.with(|stack| stack.borrow_mut().push((from.clone(), to.clone())));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            HOISTING.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
    let _pop = Pop;
    Some(relate())
}

/// The operands of a type variable or of a memberless intersection holding
/// one, which is how surge keeps `T & X` generic.
fn generic_intersection_operands(ty: &Type) -> Option<Vec<Type>> {
    if ty.is_type_variable() {
        return Some(vec![ty.clone()]);
    }
    let operands = type_variable_intersection_operands(ty)?;
    is_narrowed_type_variable(ty).then(|| operands.to_vec())
}

/// `getIntersectionType` of two operands one of which is generic: the
/// variables are kept (first, as `with_type_variable_operands` orders them)
/// and the other operands reduced among themselves.
fn intersect_with_type_variables(left: &Type, right: &Type) -> Option<Type> {
    let mut variables: Vec<Type> = Vec::new();
    let mut others: Vec<Type> = Vec::new();
    for operand in [left, right]
        .into_iter()
        .flat_map(|ty| generic_intersection_operands(ty).unwrap_or_else(|| vec![ty.clone()]))
    {
        if operand.is_type_variable() {
            if !variables.contains(&operand) {
                variables.push(operand);
            }
        } else {
            others.push(operand);
        }
    }
    let mut reduced: Option<Type> = None;
    for operand in others {
        reduced = Some(match reduced {
            None => operand,
            Some(previous) => intersect_constraint_pair(&previous, &operand)?,
        });
    }
    match reduced {
        Some(Type::Never) => Some(Type::Never),
        Some(Type::Union(_)) => None,
        Some(other) if other.is_unknown() => None,
        Some(other) => {
            variables.push(other);
            Some(crate::type_variable::type_variable_intersection(variables))
        }
        None if variables.len() == 1 => variables.pop(),
        None => Some(crate::type_variable::type_variable_intersection(variables)),
    }
}

/// `TypeFlagsPrimitive`, the targets the comparable relation hoists an
/// intersection's constraints against.
fn is_primitive_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Void
            | Type::Undefined
            | Type::Null
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
    )
}

fn some_type(ty: &Type, predicate: impl Fn(&Type) -> bool) -> bool {
    match ty {
        Type::Union(union) => union.types().iter().any(predicate),
        other => predicate(other),
    }
}

fn active_variable(ty: &Type) -> Option<(&crate::TypeParameterType, Option<Type>)> {
    match ty {
        Type::TypeParameter(parameter) => {
            crate::type_variable::active_constraint(parameter).map(|constraint| (parameter, constraint))
        }
        _ => None,
    }
}

/// tsc's relation for a type variable of the body being checked
/// (`structuredTypeRelatedToWorker`, relater.go): a variable *source* relates
/// through its constraint (`unknown` when it has none) once no target union
/// member is the variable itself; a variable *target* admits nothing but
/// itself, `never`, `any`, and a variable whose constraint leads to it — `T`
/// "could be instantiated with an arbitrary type". `None` when neither side is
/// such a variable. Placeholders and the degradation sentinel stay permissive
/// on either side: they are surge's gaps, not types.
fn type_variable_related(from: &Type, to: &Type) -> Option<bool> {
    // A reference that resolves to a type variable (a generic class's
    // inferred member read under the body's own variables) relates as it.
    let peeled_variable = |ty: &Type| match ty {
        Type::Reference(_) => Some(ty.peeled()).filter(|peeled| generic_intersection_operands(peeled).is_some()),
        _ => None,
    };
    if let Some(peeled) = peeled_variable(to)
        && (from.is_type_variable() || generic_intersection_operands(from).is_some())
    {
        return Some(is_assignable_to(from, &peeled));
    }
    if let Some(peeled) = peeled_variable(from)
        && (to.is_type_variable() || generic_intersection_operands(to).is_some())
    {
        return Some(is_assignable_to(&peeled, to));
    }
    // An intersection target needs every constituent (`typeRelatedToEachType`);
    // a source relates when some constituent does (`someTypeRelatedToType`), or
    // failing that through the constraint the constituents combine to
    // (`getEffectiveConstraintOfIntersection`).
    // A merged intersection also answers structurally through its members, so
    // only a failure on a memberless one is final.
    if let Some(operands) = type_variable_intersection_operands(to) {
        let variables_related = operands
            .iter()
            .filter(|operand| operand.is_type_variable() || is_narrowed_type_variable(to))
            .all(|operand| is_assignable_to(from, operand));
        if !variables_related || is_narrowed_type_variable(to) {
            return Some(variables_related);
        }
        // `typeRelatedToEachType`: the variables are done with, so the source
        // (not its constraint, which no variable admits) meets the rest.
        if let Type::Object(object) = to {
            let rest_operands: Vec<Type> =
                operands.iter().filter(|operand| !operand.is_type_variable()).cloned().collect();
            let mut rest = object.clone();
            rest.intersection_operands = (!rest_operands.is_empty()).then(|| rest_operands.into());
            return Some(is_assignable_to(from, &Type::Object(rest)));
        }
    }
    if let Some(operands) = type_variable_intersection_operands(from) {
        // `typeRelatedToSomeType` finds the source among the target's members.
        if let Type::Union(to_union) = to
            && to_union.types().contains(from)
        {
            return Some(true);
        }
        // relater.go `unionOrIntersectionRelatedTo`: comparing to a primitive,
        // the instantiable operands are replaced by their base constraints,
        // which the other operands may cut down (`T & 1` over `1 | 2`).
        if current_relation() == Relation::Comparable && is_primitive_type(to) && is_narrowed_type_variable(from) {
            // An unconstrained variable is its own base constraint, and an
            // intersection nothing replaces is left to the arms below.
            let constraints: Vec<Type> = operands
                .iter()
                .map(|operand| match operand {
                    Type::TypeParameter(_) if operand.is_type_variable() => {
                        crate::type_variable::base_constraint_or_type(operand)
                    }
                    other => other.clone(),
                })
                .collect();
            match (constraints.as_slice() != operands)
                .then(|| intersect_constraint_types(&constraints))
                .unwrap_or(Some(from.clone()))
            {
                None => return Some(true),
                Some(Type::Never) => return Some(false),
                Some(hoisted) if generic_intersection_operands(&hoisted).is_none() => {
                    return Some(is_assignable_to(&hoisted, to) || is_assignable_to(to, &hoisted));
                }
                Some(_) => {}
            }
        }
        if operands.iter().any(|operand| is_assignable_to(operand, to)) {
            return Some(true);
        }
        match effective_intersection_constraint(operands, matches!(to, Type::Union(_))) {
            None => return Some(true),
            Some(Some(constraint))
                if !some_type(&constraint, |member| member == from)
                    && with_hoisting(from, to, || is_assignable_to(&constraint, to)).unwrap_or(false) =>
            {
                return Some(true)
            }
            Some(_) => {}
        }
        if is_narrowed_type_variable(from) {
            return Some(intersection_members_related(operands, to).unwrap_or(false));
        }
    }
    // relater.go tries the target's own arms before a variable source is
    // related through its constraint.
    let target_deferred = match to {
        Type::TypeParameter(target) => crate::type_variable::deferred_type(target),
        _ => None,
    };
    // Two string mappings relate only through the same mapping's operands
    // (the `StringMapping` source arm of `structuredTypeRelatedToWorker`).
    if let Some(crate::type_variable::DeferredType::StringMapping { kind, operand }) = &target_deferred
        && let Some((source_kind, source_operand)) = crate::type_variable::string_mapping_variable_parts(from)
    {
        return Some(source_kind == *kind && is_assignable_to(&source_operand, operand));
    }
    if matches!(target_deferred, Some(crate::type_variable::DeferredType::StringMapping { .. }))
        && crate::is_member_of_string_mapping(from, to)
    {
        return Some(true);
    }
    // relater.go: for a generic type T and a type U assignable to T, `[...U]`
    // is assignable to T, U to `readonly [...T]`, and U to `[...T]` when U is
    // constrained to a mutable array or tuple type.
    if let Some((element, false)) = single_element_generic_tuple(from)
        && is_assignable_to(&element, to)
    {
        return Some(true);
    }
    if let Some((element, readonly)) = single_element_generic_tuple(to)
        && (readonly || base_is_mutable_array_or_tuple(from, 0))
        && is_assignable_to(from, &element)
    {
        return Some(true);
    }
    if let Some(deferred) = &target_deferred
        && deferred_target_related(from, deferred)
    {
        return Some(true);
    }
    // A generic mapped type is an object type, not a variable: past the target
    // arms above, it relates to a type parameter through its template and to
    // nothing else generic (the apparent-type arm finds no structure there).
    if let Some(crate::type_variable::DeferredType::Mapped {
        keys,
        object,
        modifiers,
        ..
    }) = crate::type_variable::mapped_type(from)
    {
        if target_deferred.is_some() {
            return Some(false);
        }
        if let Type::TypeParameter(target) = to
            && crate::type_variable::active_constraint(target).is_some()
        {
            return Some(mapped_source_related_to_variable(&keys, &object, modifiers, to));
        }
        // Its apparent type is itself unless its operand is constrained to an
        // array (`getResolvedApparentTypeOfMappedType`), an object either way.
        let primitive_target = match to {
            Type::Union(union) => union.types().iter().all(is_primitive_type),
            other => is_primitive_type(other),
        };
        if primitive_target {
            return Some(false);
        }
    }
    let generic_object_source = crate::type_variable::conditional_type(from).is_some()
        || crate::type_variable::mapped_generic_type(from).is_some()
        || crate::type_variable::mapped_constant_type(from).is_some();
    if generic_object_source && matches!(to, Type::Any | Type::Unknown | Type::ErrorType | Type::GenuineUnknown) {
        return Some(true);
    }
    if let Some(conditional) = crate::type_variable::conditional_type(from) {
        return Some(conditional_source_related(from, &conditional, to, target_deferred.as_ref()));
    }
    if let Some(mapped) = crate::type_variable::mapped_generic_type(from) {
        return Some(generic_mapped_source_related(from, &mapped, to, target_deferred.is_some()));
    }
    if let Some(crate::type_variable::DeferredType::MappedConstant { keys, template, modifiers }) =
        crate::type_variable::mapped_constant_type(from)
    {
        return Some(mapped_constant_source_related(from, &keys, &template, modifiers, to, target_deferred.is_some()));
    }
    let source = active_variable(from);
    let target_is_variable = active_variable(to).is_some() || target_deferred.is_some();
    if source.is_none() && !target_is_variable {
        return None;
    }
    if matches!(from, Type::Any | Type::Never | Type::Unknown | Type::ErrorType)
        || matches!(to, Type::Any | Type::Unknown | Type::ErrorType | Type::GenuineUnknown)
        || matches!(from, Type::TypeParameter(_)) && source.is_none()
        || matches!(to, Type::TypeParameter(_)) && !target_is_variable
    {
        return Some(true);
    }
    if matches!(from, Type::Null | Type::Undefined) && !crate::strict_null_checks() {
        return Some(true);
    }
    if let Some((_, constraint)) = source {
        // `typeRelatedToSomeType` before the source's constraint is tried.
        if let Type::Union(to_union) = to
            && to_union
                .types()
                .iter()
                .any(|member| member == from || is_assignable_to(from, member))
        {
            return Some(true);
        }
        // relater.go `structuredTypeRelatedToWorker`: comparability is mostly
        // bidirectional, so a type parameter is comparable to another only
        // through a constraint that itself holds a type parameter. A generic
        // mapped target is an object type, which the carve-out does not cover.
        if current_relation() == Relation::Comparable
            && target_is_variable
            && crate::type_variable::mapped_type(to).is_none()
        {
            return Some(constraint.is_some_and(|constraint| {
                constraint != *from && some_type(&constraint, |member| matches!(member, Type::TypeParameter(_)))
                    && is_assignable_to(&constraint, to)
            }));
        }
        // `T extends T` is a circular constraint tsc reports and drops.
        let constraint = constraint.filter(|constraint| constraint != from).unwrap_or(Type::GenuineUnknown);
        if is_assignable_to(&constraint, to) {
            return Some(true);
        }
        // relater.go `structuredTypeRelatedTo`: against a union, a type
        // parameter's constraint is hoisted into an intersection with it
        // (`T extends 1 | 2` to `T & 1 | T & 2`).
        if matches!(to, Type::Union(_)) {
            return Some(match effective_intersection_constraint(std::slice::from_ref(from), true) {
                None => true,
                Some(Some(hoisted)) if !some_type(&hoisted, |member| member == from) => {
                    with_hoisting(from, to, || is_assignable_to(&hoisted, to)).unwrap_or(false)
                }
                Some(_) => false,
            });
        }
        return Some(false);
    }
    Some(match from {
        Type::Union(from_union) => union_source_related(from_union, to),
        Type::Reference(reference) => is_assignable_to(&reference.resolve_arc(), to),
        _ => false,
    })
}

/// The target-side arms of `structuredTypeRelatedToWorker` for a deferred
/// type: `S[K]` relates to `T[J]` when `S` relates to `T` and `K` to `J`, and
/// anything relates to `T[K]` that relates to its base constraint for
/// writing; `keyof S` relates to `keyof T` when `T` relates to `S`, and
/// anything relates to `keyof T` that relates to the keys of `T`'s
/// constraint. A constraint surge cannot compute admits everything.
fn deferred_target_related(from: &Type, target: &crate::type_variable::DeferredType) -> bool {
    use crate::type_variable::{DeferredType, TargetConstraint};
    let source = match from {
        Type::TypeParameter(source) => crate::type_variable::deferred_type(source),
        _ => None,
    };
    let constraint = match target {
        DeferredType::IndexedAccess { object, index } => {
            if let Some(DeferredType::IndexedAccess {
                object: source_object,
                index: source_index,
            }) = &source
                && is_assignable_to(source_object, object)
                && is_assignable_to(source_index, index)
            {
                return true;
            }
            crate::type_variable::indexed_access_write_constraint(object, index)
        }
        DeferredType::Keyof(operand) => {
            if let Some(DeferredType::Keyof(source_operand)) = &source
                && is_assignable_to(operand, source_operand)
            {
                return true;
            }
            crate::type_variable::keyof_constraint_keys(operand)
        }
        DeferredType::Mapped {
            keys,
            object,
            modifiers,
            modifiers_optionality,
        } => {
            return mapped_target_related(from, source.as_ref(), keys, object, *modifiers, *modifiers_optionality);
        }
        DeferredType::TemplateLiteral { texts, types } => {
            let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
            if current_relation() == Relation::Comparable
                && let Some((source_texts, _)) = crate::template_literal::owned_template_parts(from)
            {
                let source_texts: Vec<&str> = source_texts.iter().map(String::as_str).collect();
                return !template_literal_types_definitely_unrelated(&source_texts, &texts);
            }
            let types: Vec<&Type> = types.iter().collect();
            return crate::is_type_matched_by_template_literal(from, &texts, &types);
        }
        // Decided by the caller, which has the target type itself.
        DeferredType::StringMapping { .. } => return false,
        DeferredType::MappedConstant { keys, template, modifiers } => {
            return mapped_constant_target_related(from, source.as_ref(), keys, template, *modifiers);
        }
        DeferredType::Conditional(conditional) => return conditional_target_related(from, conditional),
        // A mapped type's key parameter has no target arm of its own.
        DeferredType::MappedKey { .. } => return false,
        DeferredType::MappedGeneric(mapped) => return generic_mapped_target_related(from, source.as_ref(), mapped),
        DeferredType::Tuple { elements, readonly } => {
            return generic_tuple_target_related(from, source.as_ref(), elements, *readonly);
        }
    };
    match constraint {
        TargetConstraint::Types(types) => types.iter().all(|ty| is_assignable_to(from, ty)),
        TargetConstraint::Absent => false,
        TargetConstraint::Unmodelled => true,
    }
}

/// The element of a generic tuple of one variadic element (`[...T]`), and
/// whether the tuple is `readonly` (tsc's `isSingleElementGenericTupleType`).
fn single_element_generic_tuple(ty: &Type) -> Option<(Type, bool)> {
    let (elements, readonly) = crate::type_variable::generic_tuple(ty)?;
    match elements.as_slice() {
        [(crate::type_variable::TupleElementKind::Variadic, element)] => Some((element.clone(), readonly)),
        _ => None,
    }
}

/// tsc's `isMutableArrayOrTuple(getBaseConstraintOrType(ty))`.
fn base_is_mutable_array_or_tuple(ty: &Type, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if let Some((_, readonly)) = crate::type_variable::generic_tuple(ty) {
        return !readonly;
    }
    match ty {
        Type::TypeParameter(parameter) => match crate::type_variable::active_constraint(parameter) {
            Some(Some(constraint)) => base_is_mutable_array_or_tuple(&constraint, depth + 1),
            _ => false,
        },
        Type::Reference(reference) if reference.is_readonly_array() => false,
        Type::Reference(reference) => base_is_mutable_array_or_tuple(&reference.resolve(), depth + 1),
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        _ => false,
    }
}

/// relater.go `propertiesRelatedTo` for a generic tuple target: the source
/// must be an array or tuple, not readonly unless the target is, and relate
/// element by element — a variadic target element only to a variadic one.
fn generic_tuple_target_related(
    from: &Type,
    source: Option<&crate::type_variable::DeferredType>,
    target: &[(crate::type_variable::TupleElementKind, Type)],
    target_readonly: bool,
) -> bool {
    use crate::type_variable::{DeferredType, TupleElementKind};
    let (source_elements, source_readonly) = match source {
        Some(DeferredType::Tuple { elements, readonly }) => (elements.clone(), *readonly),
        Some(_) => return false,
        None => {
            let (inner, readonly) = match from {
                Type::Reference(reference) if reference.is_readonly_array() => (reference.resolve(), true),
                other => (other.clone(), false),
            };
            let Some(elements) = crate::type_variable::spread_elements(&inner) else {
                return false;
            };
            (elements, readonly)
        }
    };
    if source_readonly && !target_readonly {
        return false;
    }
    let is_variable = |kind: TupleElementKind| matches!(kind, TupleElementKind::Rest | TupleElementKind::Variadic);
    let min_length = |elements: &[(TupleElementKind, Type)]| {
        elements
            .iter()
            .filter(|(kind, _)| matches!(kind, TupleElementKind::Required | TupleElementKind::Variadic))
            .count()
    };
    let source_arity = source_elements.len();
    let target_arity = target.len();
    let source_rest = source_elements.iter().any(|(kind, _)| *kind == TupleElementKind::Rest);
    let target_has_rest = target.iter().any(|(kind, _)| is_variable(*kind));
    let source_min_length = min_length(&source_elements);
    let target_min_length = min_length(target);
    if !source_rest && source_arity < target_min_length {
        return false;
    }
    if !target_has_rest && target_arity < source_min_length {
        return false;
    }
    if !target_has_rest && (source_rest || target_arity < source_arity) {
        return false;
    }
    let target_start_count = target.iter().take_while(|(kind, _)| *kind != TupleElementKind::Rest).count();
    let target_end_count = target.iter().rev().take_while(|(kind, _)| *kind != TupleElementKind::Rest).count();
    for (source_position, (source_kind, source_type)) in source_elements.iter().enumerate() {
        let source_position_from_end = source_arity - 1 - source_position;
        let target_position = if target_has_rest && source_position >= target_start_count {
            target_arity as isize - 1 - source_position_from_end.min(target_end_count) as isize
        } else {
            source_position as isize
        };
        let Some((target_kind, target_type)) = usize::try_from(target_position).ok().and_then(|index| target.get(index))
        else {
            return false;
        };
        if *target_kind == TupleElementKind::Variadic && *source_kind != TupleElementKind::Variadic {
            return false;
        }
        if *source_kind == TupleElementKind::Variadic && !is_variable(*target_kind) {
            return false;
        }
        if *target_kind == TupleElementKind::Required && *source_kind != TupleElementKind::Required {
            return false;
        }
        let target_check_type = if *source_kind == TupleElementKind::Variadic && *target_kind == TupleElementKind::Rest {
            Type::Array(Box::new(target_type.clone()))
        } else {
            target_type.clone()
        };
        if !is_assignable_to(source_type, &target_check_type) {
            return false;
        }
    }
    true
}

/// The generic-mapped-type target arms of `structuredTypeRelatedToWorker`
/// (relater.go) for `{ [P in keys]: object[P] }`. A source `S` relates when
/// the target keeps `?` possible and `S` is `object` itself, or `S` is a type
/// variable whose keys cover the target's (or overlap them, for a `?`
/// mapping) and which relates to `object` — `S[P]` to `object[P]`. A generic
/// mapped source relates through `mappedTypeRelatedTo`, and an empty object
/// type relates to a mapping that adds `?` (`isPartialMappedType`). Nothing
/// else does: a source with structure of its own never relates to `object`,
/// a variable of the body being checked.
fn mapped_target_related(
    from: &Type,
    source: Option<&crate::type_variable::DeferredType>,
    keys: &Type,
    object: &Type,
    modifiers: crate::type_variable::MappedModifiers,
    modifiers_optionality: i8,
) -> bool {
    if modifiers.optional >= 0 && object == from {
        return true;
    }
    if let Some(source_view) = source.and_then(mapped_view) {
        let target = crate::type_variable::DeferredType::Mapped {
            keys: keys.clone(),
            object: object.clone(),
            modifiers,
            modifiers_optionality,
        };
        if let (MappedTemplate::Generic(_) | MappedTemplate::Constant(..), Some(target_view)) =
            (&source_view.template, mapped_view(&target))
        {
            return mapped_views_related(&source_view, &target_view);
        }
    }
    if let Some(crate::type_variable::DeferredType::Mapped {
        keys: source_keys,
        object: source_object,
        modifiers: source_modifiers,
        modifiers_optionality: source_modifiers_optionality,
    }) = source
    {
        return mapped_type_related(
            (source_keys, source_object, *source_modifiers, *source_modifiers_optionality),
            (keys, object, modifiers, modifiers_optionality),
        );
    }
    if modifiers.optional >= 0 && from.is_type_variable() {
        // With `?` a key the source lacks is simply absent; without it every
        // target key must be one of the source's.
        let keys_related = modifiers.optional > 0
            || crate::type_variable::keyof_variable(from).is_some_and(|source_keys| is_assignable_to(keys, &source_keys));
        if keys_related && is_assignable_to(from, object) {
            return true;
        }
    }
    // A shape the checker had to leave open may be the empty object type.
    if matches!(from, Type::Object(object) if object.synthetic_open_index) {
        return true;
    }
    modifiers.optional > 0 && is_empty_object_type(from)
}

/// The conditional-type target arm of `structuredTypeRelatedToWorker`: with
/// no `infer` positions and no branch depending on distribution, a source
/// relates when it relates to each branch the check does not rule out —
/// the true branch is skipped when even a permissive instantiation of the
/// check type fails the extends type (only `never` fails an unconstrained
/// one), the false branch when a restrictive one satisfies it.
fn conditional_target_related(from: &Type, target: &crate::type_variable::DeferredConditional) -> bool {
    if target.has_infer || target.distribution_dependent {
        return false;
    }
    let skip_true = matches!(target.extends, Type::Never);
    let skip_false = !skip_true
        && (matches!(target.extends, Type::Any | Type::GenuineUnknown)
            || target.extends == target.check
            || matches!(&target.extends, Type::Union(union) if union.types().contains(&target.check)));
    (skip_true || is_assignable_to(from, &target.true_type)) && (skip_false || is_assignable_to(from, &target.false_type))
}

/// The conditional-type source arm of `structuredTypeRelatedToWorker`, past
/// the target arms: another conditional with an identical extends type and a
/// check type related either way relates branch by branch; otherwise the
/// source relates through its default constraint (the union of its branches)
/// or, to a target that is not conditional, its distributive constraint. A
/// union target relates when a member does.
fn conditional_source_related(
    from: &Type,
    source: &crate::type_variable::DeferredConditional,
    to: &Type,
    target_deferred: Option<&crate::type_variable::DeferredType>,
) -> bool {
    use crate::type_variable::DeferredType;
    if let Type::Union(union) = to
        && union.types().iter().any(|member| member == from || is_assignable_to(from, member))
    {
        return true;
    }
    let target_conditional = match target_deferred {
        Some(DeferredType::Conditional(target)) => Some(target.as_ref()),
        _ => None,
    };
    // With `infer` positions tsc first infers the source's from the target's
    // extends type; two patterns surge resolved identically bind their
    // `infer` parameters alike, so their branches compare as written.
    if let Some(target) = target_conditional
        && crate::is_type_identical_to(&source.extends, &target.extends)
        && (is_assignable_to(&source.check, &target.check) || is_assignable_to(&target.check, &source.check))
        && is_assignable_to(&source.true_type, &target.true_type)
        && is_assignable_to(&source.false_type, &target.false_type)
    {
        return true;
    }
    match crate::type_variable::default_conditional_constraint(source) {
        Some(constraint) if is_assignable_to(&constraint, to) => return true,
        Some(_) => {}
        None => return true,
    }
    target_conditional.is_none()
        && source
            .distributive_constraint
            .as_ref()
            .is_some_and(|constraint| is_assignable_to(constraint, to))
}

/// A generic mapped type as `mappedTypeRelatedTo` reads it.
struct MappedView {
    /// The key parameter, where the mapping has one of its own.
    key: Option<Type>,
    keys: Type,
    name_type: Option<Type>,
    /// `getCombinedMappedTypeOptionality`.
    optionality: i8,
    template: MappedTemplate,
}

enum MappedTemplate {
    /// `object[P]`, holding `undefined` when the mapping adds `?`.
    Indexed(Type, bool),
    /// A template that does not read `P`.
    Constant(Type),
    Generic(crate::type_variable::DeferredMapped),
}

fn mapped_view(kind: &crate::type_variable::DeferredType) -> Option<MappedView> {
    use crate::type_variable::DeferredType;
    let optionality = |modifiers: crate::type_variable::MappedModifiers, inherited: i8| {
        if modifiers.optional != 0 { modifiers.optional } else { inherited }
    };
    match kind {
        DeferredType::Mapped {
            keys,
            object,
            modifiers,
            modifiers_optionality,
        } => Some(MappedView {
            key: None,
            keys: keys.clone(),
            name_type: None,
            optionality: optionality(*modifiers, *modifiers_optionality),
            template: MappedTemplate::Indexed(object.clone(), modifiers.optional > 0),
        }),
        DeferredType::MappedConstant { keys, template, modifiers } => Some(MappedView {
            key: None,
            keys: keys.clone(),
            name_type: None,
            optionality: modifiers.optional,
            template: MappedTemplate::Constant(mapped_constant_template(template, *modifiers)),
        }),
        DeferredType::MappedGeneric(mapped) => Some(MappedView {
            key: Some(mapped.key.clone()),
            keys: mapped.keys.clone(),
            name_type: mapped.name_type.clone(),
            optionality: optionality(mapped.modifiers, mapped.modifiers_optionality),
            template: MappedTemplate::Generic((**mapped).clone()),
        }),
        _ => None,
    }
}

/// The view's template read at `key`.
fn mapped_view_template(view: &MappedView, key: &Type) -> Option<Type> {
    Some(match &view.template {
        MappedTemplate::Indexed(object, optional) => {
            let read = crate::type_variable::indexed_access_variable(object, key)?;
            if *optional && crate::strict_null_checks() {
                crate::union_type(vec![read, Type::Undefined])
            } else {
                read
            }
        }
        MappedTemplate::Constant(template) => template.clone(),
        MappedTemplate::Generic(mapped) => crate::type_variable::mapped_generic_template(mapped, key),
    })
}

/// relater.go `mappedTypeRelatedTo`: the source may not add `?` the target
/// does not, the target's keys must relate to the source's, the `as`
/// clauses must be the same once the source's key parameter is the
/// target's, and the source's template must relate to the target's there.
fn mapped_views_related(source: &MappedView, target: &MappedView) -> bool {
    let modifiers_related = current_relation() == Relation::Comparable || source.optionality <= target.optionality;
    if !modifiers_related || !is_assignable_to(&target.keys, &source.keys) {
        return false;
    }
    let key = target.key.clone().or_else(|| source.key.clone()).unwrap_or_else(|| target.keys.clone());
    let rename = |view: &MappedView, ty: &Type| match &view.key {
        Some(Type::TypeParameter(parameter)) => crate::type_variable::substitute_variable(ty, parameter, &key),
        _ => ty.clone(),
    };
    let source_name = source.name_type.as_ref().map(|name| rename(source, name));
    let target_name = target.name_type.as_ref().map(|name| rename(target, name));
    if source_name != target_name {
        return false;
    }
    match (mapped_view_template(source, &key), mapped_view_template(target, &key)) {
        (Some(source_template), Some(target_template)) => is_assignable_to(&source_template, &target_template),
        _ => true,
    }
}

/// The generic-mapped-type target arms of `structuredTypeRelatedToWorker`
/// for the general shape. Another generic mapped type relates through
/// `mappedTypeRelatedTo`. Unless the mapping removes `?`, any other source
/// `S` relates when the target's keys (its `as` clause's, where it has one)
/// relate to `keyof S` — for a mapping that adds `?`, when some are among
/// them — and `S` at those keys relates to the template; `S` itself relates
/// to a template `S[P]` and to one reading its keys off an object `S`
/// relates to.
fn generic_mapped_target_related(
    from: &Type,
    source: Option<&crate::type_variable::DeferredType>,
    target: &crate::type_variable::DeferredMapped,
) -> bool {
    use crate::type_variable::DeferredType;
    if let Some(source_view) = source.and_then(mapped_view) {
        let target_view = mapped_view(&DeferredType::MappedGeneric(Box::new(target.clone())));
        return target_view.is_some_and(|target_view| mapped_views_related(&source_view, &target_view));
    }
    if target.modifiers.optional < 0 {
        return false;
    }
    let keys_remapped = target.name_type.is_some();
    let indexed_template = |ty: &Type| match ty {
        Type::TypeParameter(parameter) => match crate::type_variable::deferred_type(parameter) {
            Some(DeferredType::IndexedAccess { object, index }) if index == target.key => Some(object),
            _ => None,
        },
        _ => None,
    };
    if !keys_remapped && indexed_template(&target.template).is_some_and(|object| object == *from) {
        return true;
    }
    if matches!(from, Type::Object(object) if object.synthetic_open_index) {
        return true;
    }
    if target.modifiers.optional > 0 && is_empty_object_type(from) {
        return true;
    }
    let target_keys = target.name_type.clone().unwrap_or_else(|| target.keys.clone());
    let source_keys = if from.is_type_variable() {
        crate::type_variable::keyof_variable(from)
    } else {
        crate::type_variable::literal_keys_of(from)
    };
    let Some(source_keys) = source_keys else {
        return true;
    };
    let filtered = if target.modifiers.optional > 0 {
        if matches!(source_keys, Type::Never) {
            return false;
        }
        Some(crate::type_variable::type_variable_intersection(vec![target_keys.clone(), source_keys]))
    } else {
        if !is_assignable_to(&target_keys, &source_keys) {
            return false;
        }
        None
    };
    let template = crate::type_variable::mapped_generic_template(target, &target.key);
    // `extractTypesOfKind(templateType, ^TypeFlagsNullable)`.
    let non_null_template = match &target.template {
        Type::Union(union) => crate::union_type(
            union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    };
    if !keys_remapped && let Some(object) = indexed_template(&non_null_template) {
        return is_assignable_to(from, &object);
    }
    let indexing = match (&filtered, keys_remapped) {
        (Some(filtered), true) => filtered.clone(),
        (None, true) => target_keys,
        (Some(filtered), false) => {
            crate::type_variable::type_variable_intersection(vec![filtered.clone(), target.key.clone()])
        }
        (None, false) => target.key.clone(),
    };
    crate::type_variable::indexed_access_variable(from, &indexing).is_none_or(|read| is_assignable_to(&read, &template))
}

/// A generic mapped source in the general shape, past the target arms: an
/// object type. It relates to a type parameter `T` when it has no `as`
/// clause, adds no `?`, `keyof T` relates to its keys and its template to
/// `T[P]`; to nothing else generic; to a union when it relates to a member;
/// and to an object type through its members — none, unless the key set is
/// the `keyof` of a constrained variable whose apparent members surge does
/// not enumerate — with a string index signature taking the template.
fn generic_mapped_source_related(
    from: &Type,
    mapped: &crate::type_variable::DeferredMapped,
    to: &Type,
    target_is_deferred: bool,
) -> bool {
    if target_is_deferred {
        return false;
    }
    let template = crate::type_variable::mapped_generic_template(mapped, &mapped.key);
    match to {
        Type::TypeParameter(target) => {
            crate::type_variable::active_constraint(target).is_some()
                && mapped.name_type.is_none()
                && mapped.modifiers.optional <= 0
                && crate::type_variable::keyof_variable(to).is_some_and(|target_keys| is_assignable_to(&target_keys, &mapped.keys))
                && crate::type_variable::indexed_access_variable(to, &mapped.key)
                    .is_some_and(|read| is_assignable_to(&template, &read))
        }
        Type::Union(union) => union.types().iter().any(|member| is_assignable_to(from, member)),
        Type::Reference(_) => match to.peeled() {
            Type::Reference(_) => false,
            peeled => is_assignable_to(from, &peeled),
        },
        Type::Object(object) => {
            let apparent_members_unknown = match &mapped.keys {
                Type::TypeParameter(keys) => match crate::type_variable::deferred_type(keys) {
                    Some(crate::type_variable::DeferredType::Keyof(Type::TypeParameter(operand))) => {
                        !matches!(crate::type_variable::active_constraint(&operand), Some(None))
                    }
                    _ => false,
                },
                _ => false,
            };
            if apparent_members_unknown {
                return true;
            }
            if object.properties.values().any(|property| !property.optional)
                || object.call_signature().is_some()
                || object.construct_signature().is_some()
            {
                return false;
            }
            match (&object.string_index_type, &object.number_index_type) {
                (Some(value), number) => {
                    is_assignable_to(&template, value)
                        && number.as_deref().is_none_or(|value| is_assignable_to(&template, value))
                }
                (None, Some(_)) => false,
                (None, None) => true,
            }
        }
        _ => false,
    }
}

/// `getTemplateTypeFromMappedType`: a mapping that adds `?` reads its
/// template with `undefined`.
fn mapped_constant_template(template: &Type, modifiers: crate::type_variable::MappedModifiers) -> Type {
    if modifiers.optional > 0 && crate::strict_null_checks() {
        crate::union_type(vec![template.clone(), Type::Undefined])
    } else {
        template.clone()
    }
}

/// The generic-mapped-type target arms of `structuredTypeRelatedToWorker` for
/// `{ [P in keys]: template }`, a template that does not read `P`. Another
/// such mapping relates through `mappedTypeRelatedTo`. Any other source that
/// is not a generic mapped type relates, unless the mapping removes `?`, when
/// `keys` relates to its keys (`getIndexType` without index signatures) — or,
/// for a mapping that adds `?`, when some key is among them — and what it
/// holds at those keys relates to the template. An empty object type relates
/// to a mapping that adds `?` (`isPartialMappedType`).
fn mapped_constant_target_related(
    from: &Type,
    source: Option<&crate::type_variable::DeferredType>,
    keys: &Type,
    template: &Type,
    modifiers: crate::type_variable::MappedModifiers,
) -> bool {
    use crate::type_variable::{DeferredType, TargetConstraint};
    if let Some(DeferredType::MappedConstant {
        keys: source_keys,
        template: source_template,
        modifiers: source_modifiers,
    }) = source
    {
        let modifiers_related =
            current_relation() == Relation::Comparable || source_modifiers.optional <= modifiers.optional;
        return modifiers_related
            && is_assignable_to(keys, source_keys)
            && is_assignable_to(
                &mapped_constant_template(source_template, *source_modifiers),
                &mapped_constant_template(template, modifiers),
            );
    }
    if let Some(source_view) = source.and_then(mapped_view) {
        let target = DeferredType::MappedConstant {
            keys: keys.clone(),
            template: template.clone(),
            modifiers,
        };
        return mapped_view(&target).is_some_and(|target_view| mapped_views_related(&source_view, &target_view));
    }
    if modifiers.optional < 0 {
        return false;
    }
    let target_template = mapped_constant_template(template, modifiers);
    if from.is_type_variable() {
        let Some(source_keys) = crate::type_variable::keyof_variable(from) else {
            return false;
        };
        if modifiers.optional == 0 && !is_assignable_to(keys, &source_keys) {
            return false;
        }
        return crate::type_variable::indexed_access_variable(from, keys)
            .is_some_and(|read| is_assignable_to(&read, &target_template));
    }
    if matches!(from, Type::Object(object) if object.synthetic_open_index) {
        return true;
    }
    if modifiers.optional > 0 && is_empty_object_type(from) {
        return true;
    }
    let Some(source_keys) = crate::type_variable::literal_keys_of(from) else {
        return true;
    };
    let read_keys = if modifiers.optional > 0 {
        let key_constraint = crate::type_variable::base_constraint_or_type(keys);
        let common: Vec<Type> = match &source_keys {
            Type::Union(union) => union.types().to_vec(),
            Type::Never => Vec::new(),
            other => vec![other.clone()],
        }
        .into_iter()
        .filter(|key| is_assignable_to(key, &key_constraint))
        .collect();
        if common.is_empty() {
            return false;
        }
        crate::union_type(common)
    } else {
        if !is_assignable_to(keys, &source_keys) {
            return false;
        }
        keys.clone()
    };
    match crate::type_variable::indexed_access_read_types(from, &read_keys) {
        TargetConstraint::Types(types) => types.iter().all(|ty| is_assignable_to(ty, &target_template)),
        TargetConstraint::Absent => false,
        TargetConstraint::Unmodelled => true,
    }
}

/// A generic mapped source `{ [P in keys]: template }` whose template does not
/// read `P`, past the target arms: an object type. It relates to a type
/// parameter `T` when it adds no `?`, `keyof T` relates to `keys` and the
/// template to `T[keys]`; to nothing else generic; to a union when it relates
/// to a member; and to an object type through its members, of which it has
/// none (`resolveMappedTypeMembers` enumerates no generic key), while a
/// string index signature takes the template (`indexSignaturesRelatedTo`).
fn mapped_constant_source_related(
    from: &Type,
    keys: &Type,
    template: &Type,
    modifiers: crate::type_variable::MappedModifiers,
    to: &Type,
    target_is_deferred: bool,
) -> bool {
    if target_is_deferred {
        return false;
    }
    let source_template = mapped_constant_template(template, modifiers);
    match to {
        Type::TypeParameter(target) => {
            crate::type_variable::active_constraint(target).is_some()
                && modifiers.optional <= 0
                && crate::type_variable::keyof_variable(to).is_some_and(|target_keys| is_assignable_to(&target_keys, keys))
                && crate::type_variable::indexed_access_variable(to, keys)
                    .is_some_and(|read| is_assignable_to(&source_template, &read))
        }
        Type::Union(union) => union.types().iter().any(|member| is_assignable_to(from, member)),
        Type::Reference(_) => match to.peeled() {
            Type::Reference(_) => false,
            peeled => is_assignable_to(from, &peeled),
        },
        Type::Object(object) => {
            if object.properties.values().any(|property| !property.optional)
                || object.call_signature().is_some()
                || object.construct_signature().is_some()
            {
                return false;
            }
            match (&object.string_index_type, &object.number_index_type) {
                (Some(value), number) => {
                    is_assignable_to(&source_template, value)
                        && number.as_deref().is_none_or(|value| is_assignable_to(&source_template, value))
                }
                (None, Some(_)) => false,
                (None, None) => true,
            }
        }
        _ => false,
    }
}

/// relater.go `mappedTypeRelatedTo` for two mappings of the deferred shape:
/// the source may not add `?` the target does not
/// (`getCombinedMappedTypeOptionality`), the target's keys must be the
/// source's, and the source template must relate to the target's.
fn mapped_type_related(
    source: (&Type, &Type, crate::type_variable::MappedModifiers, i8),
    target: (&Type, &Type, crate::type_variable::MappedModifiers, i8),
) -> bool {
    let optionality = |modifiers: crate::type_variable::MappedModifiers, modifiers_optionality: i8| {
        if modifiers.optional != 0 { modifiers.optional } else { modifiers_optionality }
    };
    let modifiers_related = current_relation() == Relation::Comparable
        || optionality(source.2, source.3) <= optionality(target.2, target.3);
    if !modifiers_related || !is_assignable_to(target.0, source.0) {
        return false;
    }
    let (source_object, source_optional) = simplified_mapped_template(source.1, source.2);
    let (target_object, target_optional) = simplified_mapped_template(target.1, target.2);
    (!source_optional || target_optional || !crate::strict_null_checks())
        && is_assignable_to(&source_object, &target_object)
}

/// A mapping's template `object[P]` as `getSimplifiedType` leaves it: the
/// object a chain of mapped objects bottoms out in (`substituteIndexedMappedType`
/// at each step), and whether the template holds `undefined` — added by a
/// mapping's own `?` (`getTemplateTypeFromMappedType`) or by a substituted
/// mapping that adds `?`.
fn simplified_mapped_template(object: &Type, modifiers: crate::type_variable::MappedModifiers) -> (Type, bool) {
    let mut object = object.clone();
    let mut optional = modifiers.optional > 0;
    while let Some(crate::type_variable::DeferredType::Mapped {
        object: inner_object,
        modifiers: inner_modifiers,
        modifiers_optionality,
        ..
    }) = crate::type_variable::mapped_type(&object)
    {
        optional |= inner_modifiers.optional > 0 || modifiers_optionality > 0;
        object = inner_object;
    }
    (object, optional)
}

/// A generic mapped source against a type parameter target (relater.go's
/// `TypeFlagsTypeParameter` target arm): `{ [P in Q]: X }` relates to `T`
/// when it adds no `?`, `keyof T` relates to `Q`, and `X` to `T[Q]` — for the
/// template `object[P]`, `object` to `T`.
fn mapped_source_related_to_variable(
    keys: &Type,
    object: &Type,
    modifiers: crate::type_variable::MappedModifiers,
    target: &Type,
) -> bool {
    modifiers.optional <= 0
        && crate::type_variable::keyof_variable(target).is_some_and(|target_keys| is_assignable_to(&target_keys, keys))
        && is_assignable_to(object, target)
}

/// tsc's `isEmptyObjectType` for an object source: no members, signatures or
/// index signatures, or the `object` keyword.
fn is_empty_object_type(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if object.non_primitive
            || (object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !object.is_intersection))
}

/// relater.go `typeArgumentsRelatedTo` for a parameter whose variance the
/// declaration states. An `Unmeasurable` one relates only identical
/// arguments (`compareTypesIdentical`).
fn declared_argument_related(variance: crate::DeclaredVariance, source: &Type, target: &Type) -> bool {
    match variance {
        crate::DeclaredVariance::Covariant => is_assignable_to(source, target),
        crate::DeclaredVariance::Contravariant | crate::DeclaredVariance::UnreliableContravariant => {
            is_assignable_to(target, source)
        }
        crate::DeclaredVariance::Invariant => {
            is_assignable_to(source, target) && is_assignable_to(target, source)
        }
        crate::DeclaredVariance::Unmeasurable => crate::is_type_identical_to(source, target),
        crate::DeclaredVariance::MeasuredContravariant => {
            unmodelled_argument(source) || unmodelled_argument(target) || is_assignable_to(target, source)
        }
        crate::DeclaredVariance::MeasuredInvariant => {
            unmodelled_argument(source)
                || unmodelled_argument(target)
                || (is_assignable_to(source, target) && is_assignable_to(target, source))
        }
    }
}

/// A type argument that stands for something surge could not infer or model,
/// which relates either way under a measured variance.
fn unmodelled_argument(argument: &Type) -> bool {
    matches!(argument, Type::Any | Type::Unknown | Type::ErrorType | Type::GenuineUnknown)
        || (matches!(argument, Type::TypeParameter(_)) && !argument.is_type_variable())
}

/// relater.go `hasCovariantVoidArgument`: a `void` target argument for a
/// covariant parameter lets a failed variance check retry structurally.
fn has_covariant_void_argument(target_arguments: &[Type], variances: u32) -> bool {
    target_arguments.iter().enumerate().any(|(index, argument)| {
        matches!(argument, Type::Void)
            && crate::declared_variance(variances, index) == Some(crate::DeclaredVariance::Covariant)
    })
}

fn assignability_arms(from: &Type, to: &Type) -> bool {
    // relater.go `isRelatedTo`: the comparable relation skips the weak type
    // check (`isPerformingCommonPropertyChecks`) except for a unit source,
    // which must still share a property with an all-optional target.
    if current_relation() == Relation::Comparable
        && let Type::Object(target) = to
        && is_weak_object(target)
        && is_unit_literal(from)
        && !target.properties.keys().any(|name| primitive_member_type(from, name).is_some())
    {
        return false;
    }

    // A string mapping target (`Uppercase<string>`): the same mapping relates
    // by what it maps, anything else has to be a member of it
    // (`isMemberOfStringMapping`).
    if let Some((target_kind, target_inner)) = crate::string_mapping_parts(to) {
        let source = crate::peel_to_pattern_literal(from);
        if let Some((source_kind, source_inner)) = crate::string_mapping_parts(&source) {
            return source_kind == target_kind && is_assignable_to(source_inner, target_inner);
        }
        return match &source {
            Type::Union(union) => union_source_related(union, to),
            Type::Any | Type::Never | Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => true,
            other => crate::is_member_of_string_mapping(other, to),
        };
    }

    // A template literal target relates by pattern (tsc's
    // `isTypeMatchedByTemplateLiteralType`), before either side is peeled: the
    // pattern resolves to `string`, which would accept every string literal and
    // reject nothing.
    if let Some((texts, types)) = crate::template_literal_parts(to) {
        let source = crate::peel_to_pattern_literal(from);
        return match &source {
            Type::Union(union) => union_source_related(union, to),
            Type::Any | Type::Never | Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => true,
            other => match crate::template_literal_parts(other) {
                // relater.go: two template literal types are comparable
                // unless their fixed texts already rule it out.
                Some((source_texts, _)) if current_relation() == Relation::Comparable => {
                    !template_literal_types_definitely_unrelated(&source_texts, &texts)
                }
                _ => crate::is_type_matched_by_template_literal(other, &texts, &types),
            },
        };
    }

    // tsc relates two instantiations of the lib promise through `T`'s measured
    // variance, which is covariant (`then` hands `T` to a callback, an output
    // position under `compareSignaturesRelated`'s callback rule), and a failed
    // variance check is final: the structural retry `hasCovariantVoidArgument`
    // allows for a `void` target argument fails on that same callback. A
    // `Promise` is a `PromiseLike`; the reverse lacks `catch` and is left to
    // the structure.
    if let (Some((source_is_like, source_argument)), Some((target_is_like, target_argument))) =
        (lib_promise_argument(from), lib_promise_argument(to))
        && (!source_is_like || target_is_like)
    {
        return is_assignable_to(source_argument, target_argument);
    }

    // Enum types are nominal (`isEnumTypeRelatedTo`): a member of one enum
    // relates to another enum only when the two are same-named regular enums
    // whose members match, and then by value.
    if let (Type::Reference(from_ref), Type::Reference(to_ref)) = (from, to)
        && let (Some(from_enum), Some(to_enum)) = (&from_ref.enum_owner, &to_ref.enum_owner)
        && from_enum != to_enum
    {
        return different_enums_related(from_ref, to_ref)
            && is_assignable_to(&from_ref.resolve_arc(), &to_ref.resolve_arc());
    }
    // Relate a union source to an enum member by member, before the target
    // is peeled to its values and the members' enum identity is lost.
    if let (Type::Union(from_union), Type::Reference(to_ref)) = (from, to)
        && to_ref.enum_owner.is_some()
    {
        return union_source_related(from_union, to);
    }
    // Nominal identity: two objects resolved from the same non-generic named
    // declaration are the same type, even if one expanded to a structurally
    // different shape (a deeply cyclic library type can resolve to different
    // depths at different sites). This mirrors tsc's named-type handling.
    if let (Type::Object(from_obj), Type::Object(to_obj)) = (from, to) {
        if let (Some(from_id), Some(to_id)) = (&from_obj.alias_id, &to_obj.alias_id) {
            if from_id == to_id {
                return true;
            }
        }
    }

    // Two instantiations of the *same* generic declaration compare by their type
    // arguments rather than by their (often deeply self-referential) structural
    // expansion. tsc treats `Foo<A>` assignable to `Foo<B>` when the arguments are
    // pairwise compatible — an `any` argument matches anything in either
    // direction. A `unknown` *source* argument is also accepted: it is surge's
    // sentinel for a generic the checker could not infer (a method's own type
    // parameter, `pipeThrough<T>(...): ReadableStream<T>`), so failing it
    // structurally would be a false positive. Structural comparison of two
    // expansions that differ only in an `any`/`unknown` argument is exactly where
    // self-referential library generics (`Uint8Array`, `ReadableStream`, `Set`)
    // spuriously diverge.
    if let (Type::Reference(from_ref), Type::Reference(to_ref)) = (from, to) {
        if from_ref.id == to_ref.id && from_ref.arguments.len() == to_ref.arguments.len() {
            let variances = from_ref.declared_variances | to_ref.declared_variances;
            let arguments_compatible =
                from_ref
                    .arguments
                    .iter()
                    .zip(to_ref.arguments.iter())
                    .enumerate()
                    .all(|(index, (from_arg, to_arg))| match crate::declared_variance(variances, index) {
                        Some(variance) => declared_argument_related(variance, from_arg, to_arg),
                        // A type variable of the body being checked is a type,
                        // not a gap: `Foo<U>` is no `Foo<T>` unless `U` is a `T`.
                        None => {
                            matches!(from_arg, Type::Any | Type::Unknown | Type::ErrorType | Type::GenuineUnknown)
                                || (matches!(from_arg, Type::TypeParameter(_)) && !from_arg.is_type_variable())
                                || matches!(to_arg, Type::Any)
                                || is_assignable_to(from_arg, to_arg)
                        }
                    });
            if arguments_compatible {
                return true;
            }
            // relater.go `getVariancesWorker` takes an annotated parameter's
            // variance as written, and `relateVariances` makes a failed check
            // final unless a covariant argument's target is `void`
            // (`hasCovariantVoidArgument`). A parameter it measures may allow
            // the structural retry, so only a declaration annotated throughout
            // is decided here.
            if variances != 0
                && (0..to_ref.arguments.len()).all(|index| {
                    matches!(
                        crate::declared_variance(variances, index),
                        Some(
                            crate::DeclaredVariance::Covariant
                                | crate::DeclaredVariance::Contravariant
                                | crate::DeclaredVariance::Invariant
                        )
                    )
                })
                && !has_covariant_void_argument(&to_ref.arguments, variances)
            {
                return false;
            }
            // tsc relates two instantiations of one alias by the measured
            // variance of its parameters, and `Record<K, T>`'s `K` — a mapped
            // type's key — measures contravariant: `Record<string, X>` is
            // assignable to `Record<"a", X>` although no `a` is declared.
            if from_ref.id.split('\u{0}').next_back() == Some("Record")
                && let ([from_key, from_value], [to_key, to_value]) =
                    (&*from_ref.arguments, &*to_ref.arguments)
                && is_assignable_to(to_key, from_key)
                && (matches!(from_value, Type::Any) || is_assignable_to(from_value, to_value))
            {
                return true;
            }
        }
    }

    // relater.go relates two instantiations of one alias by its variances
    // before their structure. A signature alias whose parameter only reaches
    // a deferred conditional (`QueryFunction<T, K, P>` over
    // `QueryFunctionContext<K, P>`) relates that way where its structure
    // cannot: only measured or declared variances decide here, and anything
    // else is left to the structural comparison.
    if let (Type::Function(from_function), Type::Function(to_function)) = (from, to)
        && let (Some(from_alias), Some(to_alias)) =
            (from_function.alias_instantiation(), to_function.alias_instantiation())
        && from_alias.id == to_alias.id
        && from_alias.arguments.len() == to_alias.arguments.len()
    {
        let variances = from_alias.variances | to_alias.variances;
        let related = from_alias.arguments.iter().zip(to_alias.arguments.iter()).enumerate().all(
            |(index, (from_arg, to_arg))| {
                crate::is_type_identical_to(from_arg, to_arg)
                    || crate::declared_variance(variances, index)
                        .is_some_and(|variance| declared_argument_related(variance, from_arg, to_arg))
            },
        );
        if related {
            return true;
        }
    }

    // A reference assignable to a union member must be tried *before* the source
    // reference is resolved to its structural form below: otherwise `Set<any>`
    // against `Set<string> | undefined` would resolve `Set<any>` structurally and
    // compare it to the `Set<string>` member structurally, losing the nominal
    // `any`-argument shortcut above. Scoped to a `from` reference so a `from` union
    // still flows through the all-members `(Union, _)` arm.
    if let (Type::Reference(_), Type::Union(to_union)) = (from, to) {
        if to_union
            .types()
            .iter()
            .any(|to_ty| is_assignable_to(from, to_ty))
        {
            return true;
        }
    }

    // A unique symbol admits only itself: another unique symbol, or the `symbol`
    // either one widens to, is not it. A union source is left to the
    // every-member arm below.
    if let Type::Reference(target) = to
        && target.is_unique_symbol()
        && !matches!(from, Type::Union(_))
    {
        return matches!(from, Type::Reference(source) if source.id == target.id);
    }

    // Nominal references compare nominally first (same declaration + arguments is
    // handled by the `from == to` fast path above); anything else falls back to
    // comparing the structural expansion, so a reference stays interchangeable
    // with its expanded shape without forcing eager expansion at construction.
    if let Type::Reference(reference) = from {
        // A written tuple relates to another tuple by the element flags it
        // records, which its peeled shape no longer shows.
        if let Some((elements, min_length)) = reference.written_tuple()
            && let Some(related) = tuple_source_related(&fixed_tuple_kinds(elements, min_length), to)
        {
            return related;
        }
        // A pattern source against a target that only *names* a pattern (an
        // annotation's lazy `Capitalize<string>`): resolve the target first,
        // or the source peels to `string` below and the pattern is gone.
        if matches!(to, Type::Reference(_))
            && (crate::is_template_literal_type(from) || crate::string_mapping_parts(from).is_some())
        {
            let target = crate::peel_to_pattern_literal(to);
            if crate::is_template_literal_type(&target)
                || crate::string_mapping_parts(&target).is_some()
            {
                return is_assignable_to(from, &target);
            }
        }
        // A readonly array or tuple is not assignable to a mutable one: the
        // mutable surface has `push`/`splice` the readonly one lacks. A union
        // target's members were each tried against the readonly source above
        // (`typeRelatedToSomeType`); retrying them with the mutable shape it
        // resolves to would accept `readonly T[]` as `T[] | undefined`. A
        // target reference is read first for the same reason, and the lib's
        // `Array<T>` written by name is the mutable array itself.
        if reference.is_readonly_array() {
            match to {
                Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::Union(_) => return false,
                Type::Reference(target) if !target.is_readonly_array() => {
                    if target.arguments.len() == 1 && target.id.split('\u{0}').next_back() == Some("Array") {
                        return false;
                    }
                    return is_assignable_to(from, &target.resolve_arc());
                }
                _ => {}
            }
        }
        // relater.go: a pattern literal's base constraint is itself, so
        // against a primitive it relates by the simple rules alone — `string`
        // takes it and no literal does. Its `string` resolution would let the
        // comparable relation read `string` against a literal.
        if current_relation() == Relation::Comparable
            && (crate::is_template_literal_type(from) || crate::string_mapping_parts(from).is_some())
            && (matches!(
                to,
                Type::String
                    | Type::Number
                    | Type::Boolean
                    | Type::BigInt
                    | Type::Symbol
                    | Type::Undefined
                    | Type::Null
                    | Type::Void
                    | Type::StringLiteral(_)
                    | Type::NumberLiteral(_)
                    | Type::BooleanLiteral(_)
            ) || enum_member_value(to).is_some())
        {
            return is_simple_type_related_to(from, to);
        }
        // `resolve_arc` borrows the memoized/interned expansion instead of
        // deep-cloning it — this arm is peeled millions of times on
        // conditional-heavy programs.
        let resolved = reference.resolve_arc();
        if let Type::Reference(target) = to {
            return relate_reference_expansion(reference, target, || is_assignable_to(&resolved, to));
        }
        return is_assignable_to(&resolved, to);
    }
    if let Type::Reference(reference) = to {
        if reference.is_readonly_array()
            && let Type::Union(from_union) = from
        {
            return union_source_related(from_union, to);
        }
        // Any function (or callable/constructable object) is assignable to the
        // global `Function` interface. Its structural shape carries members a bare
        // function type does not expose (`prototype`, `arguments`, `caller`), so
        // the structural comparison below would wrongly reject it.
        let display = reference.display.as_ref();
        let base = display.split('<').next().unwrap_or(display);
        if base == "Function" && is_function_like(from) {
            return function_interface_indexes_related(from, &reference.resolve_arc());
        }
        // A primitive's apparent type *is* its global wrapper interface
        // (`getApparentType`), so `string` to `String` is an identity, not a
        // member-by-member comparison against a surface surge only partly
        // models.
        if reference.arguments.is_empty()
            && primitive_wrapper_interface(from) == Some(base)
            && is_global_wrapper_interface(reference, base)
        {
            return true;
        }
        let resolved = reference.resolve_arc();
        // A readonly target's apparent members are `ReadonlyArray<T>`'s, which
        // the mutable shape it resolves to would overstate.
        if reference.is_readonly_array()
            && let Type::Object(source) = from
        {
            return match resolved.as_ref() {
                Type::Array(element) => object_related_to_array_like(source, from, element, None, true),
                other => match crate::fixed_tuple_parts(other) {
                    Some(tuple) => object_related_to_array_like(
                        source,
                        from,
                        &crate::tuple_element_union(tuple.0),
                        Some(tuple),
                        true,
                    ),
                    None => false,
                },
            };
        }
        if let Some((elements, min_length)) = reference.written_tuple() {
            match from {
                // A source's own `undefined` elements count as present, as in
                // the tuple-to-tuple arm below: `[n] as const` with `n: number |
                // undefined` has its one element.
                Type::Tuple(source) => {
                    return tuple_related_to_fixed_tuple(
                        &fixed_tuple_kinds(source, source.len()),
                        &fixed_tuple_kinds(elements, min_length),
                    );
                }
                Type::Union(from_union) => return union_source_related(from_union, to),
                Type::Object(source) => {
                    return object_related_to_array_like(
                        source,
                        from,
                        &crate::tuple_element_union(elements),
                        Some((elements, min_length)),
                        false,
                    );
                }
                _ => {}
            }
        }
        return is_assignable_to(from, &resolved);
    }

    match (from, to) {
        (Type::Undefined, Type::Void) => true,
        (Type::Function(source), Type::Function(target)) => {
            is_function_assignable_to(source, target)
        }
        (Type::Array(source), Type::Array(target)) => is_assignable_to(source, target),
        // A source may stop short of the trailing target slots that carry
        // `undefined` — how an optional element (`[string, string?]`) is
        // represented (`tuple_min_length`); a slot that merely accepts it
        // (`unknown`) is still required. A source's own `undefined` elements
        // may be written ones, so they count as present.
        (Type::Tuple(source), Type::Tuple(target)) => {
            tuple_related_to_fixed_tuple(&fixed_tuple_kinds(source, source.len()), &fixed_tuple_elements(target))
        }
        // relater.go relates a mutable tuple to an array through its number
        // index type, the union of its elements (`never` for `[]`).
        (Type::Tuple(source), Type::Array(target)) => match current_relation() {
            Relation::Assignable => source.iter().all(|source_ty| is_assignable_to(source_ty, target)),
            Relation::Comparable => {
                source.is_empty() || source.iter().any(|source_ty| is_assignable_to(source_ty, target))
            }
        },
        (Type::Tuple(source), Type::OpenTuple(target)) => {
            tuple_related_to_open_tuple(&fixed_tuple_elements(source), target)
        }
        (Type::OpenTuple(source), Type::OpenTuple(target)) => {
            tuple_related_to_open_tuple(&open_tuple_elements(source), target)
        }
        (Type::OpenTuple(source), Type::Array(target)) => {
            is_assignable_to(&source.element_union(), target)
        }
        // An array is a tuple of one rest element, which provides no match for
        // a required target element (relater.go `propertiesRelatedTo`). An
        // array literal against a tuple is typed as one, so it never gets here.
        (Type::Array(source), Type::OpenTuple(target)) => {
            tuple_related_to_open_tuple(&[(ElementKind::Rest, source.as_ref())], target)
        }
        // relater.go `structuredTypeRelatedToWorker`: an object that is not an
        // array or tuple reaches an array or fixed-tuple target through the
        // structural comparison, so `interface StrNum extends Array<string |
        // number> { 0: string; 1: number; length: 2 }` satisfies `[string,
        // number]`. A tuple with a rest element admits no such source
        // (`propertiesRelatedTo`'s `ElementFlagsVariable` check).
        (Type::Object(source), Type::Array(_) | Type::Tuple(_))
            if let Some(array) = source.intersected_array() =>
        {
            is_assignable_to(array, to)
        }
        (Type::Object(source), Type::Array(element)) => {
            object_related_to_array_like(source, from, element, None, false)
        }
        (Type::Object(source), Type::Tuple(elements)) => object_related_to_array_like(
            source,
            from,
            &crate::tuple_element_union(elements),
            Some((elements.as_slice(), crate::tuple_min_length(elements))),
            false,
        ),
        (Type::Union(from_union), Type::Union(_)) => {
            // Check each source member against the whole target union rather
            // than `any` single target member: a source member that is itself a
            // union (surge builds nested unions in a few synthesized spots)
            // fits the target member-wise, not as one atom.
            union_source_related(from_union, to)
        }
        (Type::Union(from_union), to_ty) => union_source_related(from_union, to_ty),
        (from_ty, Type::Union(to_union)) => {
            to_union
                .types()
                .iter()
                .any(|to_ty| is_assignable_to(from_ty, to_ty))
                || matches!(from_ty, Type::Object(_) | Type::Tuple(_))
                    && discriminated_union_assignable(from_ty, to_union)
        }
        (Type::Object(from_obj), Type::Object(to_obj)) => {
            object_assignable(from_obj, to_obj, from, to)
        }
        // An object type carrying a call signature (e.g. `BooleanConstructor`,
        // or any `typeof fn` whose value also has properties) is assignable to a
        // function type when its call signature is. tsc treats such objects as
        // callable; without this an idiom like `arr.filter(Boolean)` is rejected.
        (Type::Object(source), Type::Function(target)) => source
            .call_signature()
            .is_some_and(|call_signature| is_function_assignable_to(call_signature, target)),
        // A function satisfies an object target when it matches the target's call
        // signature (if any) and supplies its required members. A callable interface
        // such as React's `ForwardRefRenderFunction` is the call-signature case; a
        // plain object whose members are all drawn from `Function.prototype` (`name`,
        // `length`, `call`/`apply`/`bind`, …) is the no-call-signature case — e.g. the
        // cross-realm `cls: {name: string}` idiom that accepts `typeof SomeClass`. A
        // construct-signature or index-signature target is left to the dedicated arms
        // above (or rejected), since a plain function value models neither.
        // A function has no index signature of its own, nor an inferable one
        // (`isObjectTypeWithInferableIndex` excludes a type with signatures), so
        // only `indexSignaturesRelatedTo`'s exemption for an `any` value under a
        // target string index lets one through.
        (Type::Function(source), Type::Object(target)) => {
            let target_has_string_index = target.string_index_type.is_some();
            target.construct_signature().is_none()
                && target
                    .string_index_type
                    .as_deref()
                    .is_none_or(|index| matches!(index, Type::Any))
                && target
                    .number_index_type
                    .as_deref()
                    .is_none_or(|index| target_has_string_index && matches!(index, Type::Any))
                && match target.call_signature() {
                    Some(call_signature) => is_function_assignable_to(source, call_signature),
                    None => true,
                }
                && target.properties.iter().all(|(name, target_property)| {
                    match from.get_property_access_type(name).or_else(|| function_interface_member(name)) {
                        Some(source_ty) => is_assignable_to(&source_ty, &target_property.ty),
                        None => target_property.is_optional(),
                    }
                })
        }
        // A primitive structurally satisfies an object type that requires no
        // members — `{}`, all-optional shapes, and crucially the `T & {}` lib idiom
        // (e.g. `HTMLInputTypeAttribute = "button" | … | (string & {})`, where the
        // `string & {}` branch is what accepts an arbitrary `string`). tsc treats
        // any non-nullish value as assignable to such a type. Arrays and tuples are
        // objects too, so they likewise satisfy a no-required-member target — this
        // is what makes `Object.fromEntries(entries: [...][])` accept its argument
        // when the parameter degrades to `{}`.
        // The `object` keyword is the one no-member target a primitive does
        // not satisfy; arrays and tuples are objects and still do.
        (
            Type::String
            | Type::StringLiteral(_)
            | Type::Number
            | Type::NumberLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Symbol,
            Type::Object(target),
        ) if target.non_primitive => false,
        (
            Type::String
            | Type::StringLiteral(_)
            | Type::Number
            | Type::NumberLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Symbol
            | Type::Array(_)
            | Type::Tuple(_)
            | Type::OpenTuple(_),
            Type::Object(target),
        ) => {
            // A required member is satisfied by whatever the source's own
            // intrinsic surface answers for it — `length` on an array or
            // string, `[Symbol.iterator]` on either. surge models these as
            // their own `Type` variants rather than as `Array`/`String`
            // interface instances, so without the lookup every lib interface an
            // array satisfies in tsc (`ArrayLike<T>`, `Iterable<T>`, a bare
            // `{ length: number }`) was rejected wholesale.
            target
                .properties
                .iter()
                .all(|(name, property)| match primitive_member_type(from, name) {
                    // A member declared with method syntax compares its
                    // parameters bivariantly, as it does between two objects:
                    // `ConcatArray<T>.join(separator?: string)` takes the
                    // array's own `join`.
                    Some(Type::Function(source_method)) if property.method => {
                        match property.ty.peeled() {
                            Type::Function(target_method) => {
                                is_signature_assignable_to(&source_method, &target_method, true)
                            }
                            _ => is_assignable_to(&Type::Function(source_method), &property.ty),
                        }
                    }
                    Some(source_ty) => is_assignable_to(&source_ty, &property.ty),
                    None => property.is_optional(),
                })
                // Only an index signature the source *declared* rejects here. A
                // checker-injected openness marker records an intersection operand
                // surge could not enumerate (`T & {}` where `T` stayed generic), so
                // treating it as a declared `[key: string]: T` turns surge's own
                // modelling loss into a false rejection of an array against `{}`.
                // A string index of type `any` takes every non-primitive source
                // (`indexSignaturesRelatedTo`).
                && (!target.declares_string_index_access()
                    || matches!(from, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
                        && target.string_index_type.as_deref().is_some_and(|index| matches!(index, Type::Any)))
                && target.number_index_type.as_deref().is_none_or(|index| {
                    target.synthetic_open_index
                        || index.is_unmodelled()
                        || intrinsic_number_index_type(from)
                            .is_none_or(|source| source.is_unmodelled() || is_assignable_to(&source, index))
                })
                && target.call_signature().is_none()
                && target.construct_signature().is_none()
        }
        _ => false,
    }
}

/// The number index signature of an array's, a tuple's or a string's apparent
/// type, which `indexSignaturesRelatedTo` relates to a target's.
fn intrinsic_number_index_type(ty: &Type) -> Option<Type> {
    match ty {
        Type::Array(element) => Some((**element).clone()),
        Type::Tuple(elements) => Some(crate::tuple_element_union(elements)),
        Type::String | Type::StringLiteral(_) => Some(Type::String),
        _ => None,
    }
}

/// relater.go `templateLiteralTypesDefinitelyUnrelated`: the fixed texts
/// disagree where both templates start or where both end.
fn template_literal_types_definitely_unrelated(source_texts: &[&str], target_texts: &[&str]) -> bool {
    let (Some(source_start), Some(target_start), Some(source_end), Some(target_end)) = (
        source_texts.first().map(|text| text.as_bytes()),
        target_texts.first().map(|text| text.as_bytes()),
        source_texts.last().map(|text| text.as_bytes()),
        target_texts.last().map(|text| text.as_bytes()),
    ) else {
        return false;
    };
    let start = source_start.len().min(target_start.len());
    let end = source_end.len().min(target_end.len());
    source_start[..start] != target_start[..start]
        || source_end[source_end.len() - end..] != target_end[target_end.len() - end..]
}

/// Whether `ty` is a function or an object carrying a call/construct signature —
/// i.e. something assignable to the global `Function` interface.
/// A union source is related when *every* constituent is under the assignable
/// relation, but when *some* constituent is under the comparable one
/// (`relater.go`: `eachTypeRelatedToType` vs `someTypeRelatedToType`). This is
/// the only place the two relations diverge, and it applies at every level of a
/// structural comparison — which is why `[string, string | undefined]` overlaps
/// `string[]` even though it is not assignable to it.
fn union_source_related(from_union: &crate::UnionType, to: &Type) -> bool {
    // tsc's `containsType` shortcut: a source member that is itself a target
    // member relates without a comparison. Probing each one against every
    // target member instead costs n·m relation steps, which on two large
    // literal unions exhausts `MAX_ASSIGNABILITY_STEPS` and answers `true`.
    let target_index = match to {
        Type::Union(to_union) => crate::union::UnionMemberIndex::for_large(to_union.types()),
        _ => None,
    };
    let related = |from_ty: &Type| {
        target_index.as_ref().is_some_and(|index| index.contains(from_ty))
            || is_assignable_to(from_ty, to)
    };
    let mut members = from_union.types().iter();
    match current_relation() {
        Relation::Assignable => members.all(related),
        Relation::Comparable => members.any(related),
    }
}

/// A member of a value's intrinsic surface; for a primitive, past surge's
/// built-in tables, its apparent type is the global wrapper interface with
/// whatever the program declares into it (`interface Boolean { doStuff(): string }`).
fn primitive_member_type(from: &Type, name: &str) -> Option<Type> {
    from.get_property_access_type(name).or_else(|| {
        let wrapper = primitive_wrapper_interface(from)?;
        crate::current_program_type_store()?
            .augmented_wrapper(wrapper)?
            .peeled()
            .get_property_access_type(name)
    })
}

thread_local! {
    /// The recursive generic reference pairs whose expansions are being related
    /// on this stack (relater.go's `sourceStack`/`targetStack`).
    static EXPANDING_REFERENCE_PAIRS: std::cell::RefCell<Vec<(Arc<str>, Arc<str>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// relater.go `isDeeplyNestedType` on both sides: the third nested expansion
/// of one recursive generic declaration against another is assumed related
/// (`Ternary.Maybe`), so a path type (`NestedRecord<"x.y.z.a.b.c", V>`) is not
/// compared to its last level. Only instantiations over arguments that are no
/// objects count: tsc identifies a homomorphic mapping over an object
/// (`Id<T> = { [K in keyof T]: Id<T[K]> }`) by that object, which differs at
/// every level. tsc's identity is that of the type the reference expands to,
/// so an expansion into another alias (`Record<K, V>`) is that alias's.
fn relate_reference_expansion(
    source: &crate::TypeReference,
    target: &crate::TypeReference,
    relate: impl FnOnce() -> bool,
) -> bool {
    let recursion_identity = |reference: &crate::TypeReference| -> Option<Arc<str>> {
        if reference.arguments.is_empty()
            || reference.arguments.iter().any(|argument| {
                matches!(
                    argument.peeled(),
                    Type::Object(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::Function(_)
                )
            })
        {
            return None;
        }
        let base = |display: &str| display.split('<').next().unwrap_or(display).to_string();
        match reference.resolve_arc().as_ref() {
            Type::Reference(expansion) => Some(Arc::clone(&expansion.id)),
            Type::Object(object) => match object.alias_name.as_deref() {
                Some(alias) if base(alias) != base(&reference.display) => Some(Arc::from(base(alias))),
                _ => Some(Arc::clone(&reference.id)),
            },
            _ => None,
        }
    };
    let (Some(source_id), Some(target_id)) = (recursion_identity(source), recursion_identity(target)) else {
        return relate();
    };
    let deeply_nested = EXPANDING_REFERENCE_PAIRS.with(|pairs| {
        let pairs = pairs.borrow();
        pairs.iter().filter(|(expanding, _)| *expanding == source_id).count() >= 2
            && pairs.iter().filter(|(_, expanding)| *expanding == target_id).count() >= 2
    });
    if deeply_nested {
        ASSIGNABILITY_ASSUMPTION_EVENTS.with(|events| events.set(events.get() + 1));
        return true;
    }
    EXPANDING_REFERENCE_PAIRS.with(|pairs| pairs.borrow_mut().push((source_id, target_id)));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            EXPANDING_REFERENCE_PAIRS.with(|pairs| {
                pairs.borrow_mut().pop();
            });
        }
    }
    let _pop = Pop;
    relate()
}

/// The global interface a primitive's apparent type is.
fn primitive_wrapper_interface(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::String | Type::StringLiteral(_) => Some("String"),
        Type::Number | Type::NumberLiteral(_) => Some("Number"),
        Type::Boolean | Type::BooleanLiteral(_) => Some("Boolean"),
        Type::Symbol => Some("Symbol"),
        Type::BigInt => Some("BigInt"),
        _ => None,
    }
}

/// Whether `reference` names the lib's own wrapper interface rather than a
/// user type that happens to share the name: the lib's carries `valueOf`.
fn is_global_wrapper_interface(reference: &crate::TypeReference, name: &str) -> bool {
    reference.id.split('\u{0}').next_back() == Some(name)
        && matches!(
            &*reference.resolve_arc(),
            Type::Object(object) if object.properties.contains_key("valueOf")
        )
}

fn is_function_like(ty: &Type) -> bool {
    match ty {
        Type::Function(_) => true,
        Type::Object(object) => {
            object.call_signature().is_some() || object.construct_signature().is_some()
        }
        _ => false,
    }
}

/// Whether a parameter type carries the `unknown` degradation sentinel (NOT the
/// `unknown` keyword, which is `GenuineUnknown`) in an argument, member, union
/// arm, or signature position. Depth-bounded and reference-arguments-only (no
/// peel), so cyclic library reference graphs cannot loop.
/// Whether a *declared* parameter type still carries holes surge could not
/// fill — an unsubstituted type parameter, the degradation sentinel, or a
/// reference to a generic declaration written without arguments, whose members
/// were therefore built from the declaration's own parameters. Such an
/// expectation cannot reject an argument: the mismatch describes the hole, not
/// the source.
pub fn parameter_type_is_degraded(ty: &Type) -> bool {
    parameter_carries_degraded_unknown(ty, 0)
}

fn parameter_carries_degraded_unknown(ty: &Type, depth: usize) -> bool {
    carries_degraded_unknown(ty, depth, true)
}

/// A signature's parameter compared with another's: a type variable of the
/// body being checked is a real type there, not a hole.
fn signature_parameter_carries_hole(ty: &Type) -> bool {
    carries_degraded_unknown(ty, 0, false)
}

/// [`parameter_type_is_degraded`] for a type the body being checked declares,
/// where its own type variables are the real types they are there.
pub fn declared_type_is_degraded(ty: &Type) -> bool {
    carries_degraded_unknown(ty, 0, false)
}

fn carries_degraded_unknown(ty: &Type, depth: usize, active_variables_are_holes: bool) -> bool {
    if depth > 3 {
        return false;
    }
    let parameter_carries_degraded_unknown =
        |ty: &Type, depth: usize| carries_degraded_unknown(ty, depth, active_variables_are_holes);
    match ty {
        Type::TypeParameter(parameter) => active_variables_are_holes || !parameter.is_active_variable(),
        // tsc's error type is an `any` the relation reads through, so what
        // else the type says (a callback's return) is still compared.
        Type::Unknown => true,
        Type::Reference(reference) => {
            reference
                .arguments
                .iter()
                .any(|argument| parameter_carries_degraded_unknown(argument, depth + 1))
                // A lazily-deferred instantiation may carry its unresolved holes
                // only in the expanded body (its `arguments` can be empty for an
                // un-substituted parameter, e.g. `SubmitEvent<T>` in a library
                // signature resolved outside its generic context). Peel one level;
                // the depth bound keeps cyclic reference graphs from looping.
                || parameter_carries_degraded_unknown(&ty.peeled(), depth + 1)
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| parameter_carries_degraded_unknown(&property.ty, depth + 1))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(|index| parameter_carries_degraded_unknown(index, depth + 1))
        }
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| parameter_carries_degraded_unknown(member, depth + 1)),
        // A generic member signature carries the sentinel where its own type
        // parameters were erased; that is a bound name, not a hole (see the
        // same arm in the checker's return walker).
        Type::Function(function) if function.type_parameter_head().is_some() => false,
        Type::Function(function) => {
            function
                .parameters()
                .iter()
                .any(|parameter| parameter_carries_degraded_unknown(parameter, depth + 1))
                || parameter_carries_degraded_unknown(function.return_type(), depth + 1)
        }
        Type::Array(element) => parameter_carries_degraded_unknown(element, depth + 1),
        _ => false,
    }
}

/// Whether the sentinel (or an unsubstituted parameter) stands anywhere a
/// relation would read: every member, index and signature, through
/// 16 levels. A self-referential annotation (`var h: new () => typeof h`) is
/// expanded only so far, and the cut-off sits that deep.
pub fn type_carries_hole_anywhere(ty: &Type) -> bool {
    carries_hole(ty, 0, 16, false)
}

/// [`type_carries_hole_anywhere`] over the members a value shows directly and
/// theirs, without expanding references: cheap enough to ask of every value.
pub fn type_carries_hole_near_surface(ty: &Type) -> bool {
    carries_hole(ty, 0, 2, false)
}

fn carries_hole(ty: &Type, depth: usize, max_depth: usize, expand_references: bool) -> bool {
    if depth > max_depth {
        return false;
    }
    let next = |ty: &Type| carries_hole(ty, depth + 1, max_depth, expand_references);
    let signature = |function: &crate::FunctionType| {
        function.type_parameter_head().is_none()
            && (function.parameters().iter().any(next) || next(function.return_type()))
    };
    match ty {
        Type::TypeParameter(parameter) => !parameter.is_active_variable(),
        Type::Unknown | Type::ErrorType => true,
        Type::Reference(reference) => {
            reference.arguments.iter().any(next)
                || (expand_references && depth < max_depth && next(&ty.peeled()))
        }
        Type::Object(object) => {
            object.properties.values().any(|property| next(&property.ty))
                || object.string_index_type.as_deref().is_some_and(next)
                || object.number_index_type.as_deref().is_some_and(next)
                || object.call_signature().is_some_and(signature)
                || object.construct_signature().is_some_and(signature)
        }
        Type::Union(union) => union.types().iter().any(next),
        Type::Function(function) => signature(function),
        Type::Array(element) => next(element),
        Type::Tuple(elements) => elements.iter().any(next),
        Type::OpenTuple(tuple) => {
            tuple.leading.iter().chain(std::iter::once(tuple.rest.as_ref())).chain(&tuple.trailing).any(next)
        }
        _ => false,
    }
}

/// Whether two types are identical once surge's holes are filled in: a hole
/// on either side ([`type_carries_hole_anywhere`]'s) stands for whatever the
/// other side has there, and every other position must match exactly.
pub fn types_identical_modulo_holes(left: &Type, right: &Type) -> bool {
    identical_modulo_holes(left, right, 0)
}

fn identical_modulo_holes(left: &Type, right: &Type, depth: usize) -> bool {
    let is_hole = |ty: &Type| match ty {
        Type::Unknown | Type::ErrorType => true,
        Type::TypeParameter(parameter) => !parameter.is_active_variable(),
        _ => false,
    };
    if depth > 16 || is_hole(left) || is_hole(right) || crate::is_type_identical_to(left, right) {
        return true;
    }
    let next = |left: &Type, right: &Type| identical_modulo_holes(left, right, depth + 1);
    let signatures = |left: Option<&FunctionType>, right: Option<&FunctionType>| match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.type_parameter_head().is_none()
                && right.type_parameter_head().is_none()
                && left.is_variadic() == right.is_variadic()
                && left.required_parameter_count() == right.required_parameter_count()
                && left.parameters().len() == right.parameters().len()
                && left.parameters().iter().zip(right.parameters()).all(|(left, right)| next(left, right))
                && next(left.return_type(), right.return_type())
        }
        _ => false,
    };
    let indexes = |left: Option<&Type>, right: Option<&Type>| match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => next(left, right),
        _ => false,
    };
    match (left, right) {
        (Type::Array(left), Type::Array(right)) => next(left, right),
        (Type::Tuple(left), Type::Tuple(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(left, right)| next(left, right))
        }
        (Type::Union(left), Type::Union(right)) => {
            let (left, right) = (left.types(), right.types());
            left.len() == right.len()
                && left.iter().all(|member| right.iter().any(|other| next(member, other)))
                && right.iter().all(|member| left.iter().any(|other| next(other, member)))
        }
        (Type::Function(left), Type::Function(right)) => signatures(Some(left), Some(right)),
        (Type::Object(left), Type::Object(right)) => {
            left.properties.len() == right.properties.len()
                && left.properties.iter().all(|(name, property)| {
                    right.properties.get(name).is_some_and(|other| {
                        property.optional == other.optional
                            && property.readonly == other.readonly
                            && next(&property.ty, &other.ty)
                    })
                })
                && indexes(left.string_index_type.as_deref(), right.string_index_type.as_deref())
                && indexes(left.number_index_type.as_deref(), right.number_index_type.as_deref())
                && signatures(left.call_signature(), right.call_signature())
                && signatures(left.construct_signature(), right.construct_signature())
        }
        (Type::Reference(left), Type::Reference(right)) => {
            left.id == right.id
                && left.arguments.len() == right.arguments.len()
                && left.arguments.iter().zip(right.arguments.iter()).all(|(left, right)| next(left, right))
        }
        _ => false,
    }
}

/// A tuple-typed rest parameter (`(...args: [a: A, b?: B]) => R`) *is* a
/// positional parameter list in tsc, so it must compare against a plainly
/// declared `(a: A, b?: B) => R`. Expands that trailing tuple into its elements;
/// every other signature is returned unchanged.
fn expanded_signature(function: &FunctionType) -> (std::borrow::Cow<'_, [Type]>, usize, bool) {
    let parameters = function.parameters();
    if function.is_variadic()
        && let Some(rest) = parameters.last()
        && let Type::Tuple(elements) = rest_slot_shape(rest)
    {
        let leading = parameters.len() - 1;
        let mut expanded = parameters[..leading].to_vec();
        expanded.extend(elements.iter().cloned());
        let required = function.required_parameter_count().min(leading)
            + written_tuple_min_length(rest).unwrap_or_else(|| crate::tuple_min_length(&elements));
        return (std::borrow::Cow::Owned(expanded), required, false);
    }
    // `(...args: [A, ...R[]])` takes `A`, then any number of `R`.
    if function.is_variadic()
        && let Some(rest) = parameters.last()
        && let Type::OpenTuple(tuple) = rest.peeled()
        && tuple.trailing.is_empty()
    {
        let leading = parameters.len() - 1;
        let mut expanded = parameters[..leading].to_vec();
        expanded.extend(tuple.leading.iter().cloned());
        expanded.push(Type::Array(tuple.rest.clone()));
        let required = function.required_parameter_count().min(leading) + tuple.leading.len();
        return (std::borrow::Cow::Owned(expanded), required, true);
    }
    (
        std::borrow::Cow::Borrowed(parameters),
        function.required_parameter_count(),
        function.is_variadic(),
    )
}

/// Adds `undefined` to every optional, non-rest parameter under
/// `strictNullChecks`. Parameters surge could not type are left alone.
fn with_parameter_optionality(
    parameters: std::borrow::Cow<'_, [Type]>,
    required: usize,
    variadic: bool,
) -> std::borrow::Cow<'_, [Type]> {
    let rest_index = variadic.then(|| parameters.len().saturating_sub(1));
    let optional = |index: usize, parameter: &Type| {
        index >= required
            && Some(index) != rest_index
            && !parameter.is_unmodelled()
            && !matches!(parameter, Type::Any)
            && !type_includes_undefined(parameter)
    };
    if !crate::strict_null_checks()
        || !parameters
            .iter()
            .enumerate()
            .any(|(index, parameter)| optional(index, parameter))
    {
        return parameters;
    }
    std::borrow::Cow::Owned(
        parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                if optional(index, parameter) {
                    crate::union_type(vec![parameter.clone(), Type::Undefined])
                } else {
                    parameter.clone()
                }
            })
            .collect(),
    )
}

/// The structural shape behind a rest slot. A rest annotation resolves to the
/// array or tuple it is written as, but written as a deferred alias
/// (`...args: Parameters<F>`, vitest's `MockParameters<T>`) it arrives as a
/// lazy reference, and matching the variant on it directly compared the whole
/// rest as one positional parameter — every such mock read as not assignable
/// to any callback. The peel happens here, at comparison time, and not when the
/// signature is built: a declared generic signature holds that reference bound
/// to a placeholder, and forcing its memo there froze the placeholder
/// expansion for every later instantiation.
fn rest_slot_shape(rest: &Type) -> Type {
    match rest {
        Type::Reference(_) => rest.peeled(),
        other => other.clone(),
    }
}

/// The recorded `minLength` of the written tuple a rest slot names, through
/// the references in front of it.
fn written_tuple_min_length(rest: &Type) -> Option<usize> {
    let Type::Reference(reference) = rest else {
        return None;
    };
    match reference.written_tuple() {
        Some((_, min_length)) => Some(min_length),
        None => written_tuple_min_length(&reference.resolve_arc()),
    }
}

/// Widens a variadic signature's parameter list so a positional comparison
/// reaches every slot the rest parameter covers. A rest parameter is stored as
/// the array it is written as (`...items: T[]` -> `T[]`), so comparing it
/// positionally against a plainly declared `(a: T, b: T) => …` needs the array
/// expanded into `width` copies of its element.
fn widen_variadic_parameters<'a>(
    parameters: std::borrow::Cow<'a, [Type]>,
    is_variadic: bool,
    width: usize,
) -> std::borrow::Cow<'a, [Type]> {
    if !is_variadic {
        return parameters;
    }
    let Some(rest) = parameters.last() else {
        return parameters;
    };
    let leading = parameters.len() - 1;
    let mut widened = parameters[..leading].to_vec();
    match rest_slot_shape(rest) {
        Type::Array(element) => widened.resize(width.max(leading + 1), *element),
        // tsc's `getTypeAtPosition`: a rest parameter that is not itself a
        // tuple is indexed at each position, which distributes over a union of
        // tuples; a fixed tuple too short for the position reads `undefined`.
        Type::Union(union)
            if union
                .types()
                .iter()
                .all(|member| matches!(member.peeled(), Type::Tuple(_) | Type::OpenTuple(_))) =>
        {
            for position in 0..width.max(leading + 1) - leading {
                let elements = union
                    .types()
                    .iter()
                    .map(|member| match member.peeled() {
                        Type::Tuple(elements) => {
                            elements.get(position).cloned().unwrap_or(Type::Undefined)
                        }
                        Type::OpenTuple(open) => match open.leading.get(position) {
                            Some(element) => element.clone(),
                            None => {
                                let mut tail = vec![open.rest.as_ref().clone()];
                                tail.extend(open.trailing.iter().cloned());
                                crate::union_type(tail)
                            }
                        },
                        _ => unreachable!("every member is a tuple"),
                    })
                    .collect();
                widened.push(crate::union_type(elements));
            }
        }
        _ => return parameters,
    }
    std::borrow::Cow::Owned(widened)
}

fn type_includes_undefined(ty: &Type) -> bool {
    match ty {
        Type::Undefined => true,
        Type::Union(union) => union.types().iter().any(type_includes_undefined),
        _ => false,
    }
}

/// Whether a parameter slot accepts `void`, which makes tsc treat it as
/// optional for arity purposes (`getMinArgumentCount` walks the trailing
/// parameters back while they accept `void`). This is what lets a
/// `Promise<void>` executor's `resolve` — `(value: void | PromiseLike<void>)
/// => void` — be stored in a `() => void` slot.
fn parameter_accepts_void(ty: &Type) -> bool {
    match ty {
        Type::Void => true,
        Type::Union(union) => union.types().iter().any(parameter_accepts_void),
        _ => false,
    }
}

/// `required` with the trailing run of `void`-accepting parameters dropped.
fn required_count_ignoring_trailing_void(parameters: &[Type], required: usize) -> usize {
    let mut required = required.min(parameters.len());
    while required > 0 && parameter_accepts_void(&parameters[required - 1]) {
        required -= 1;
    }
    required
}

fn is_function_assignable_to(source: &FunctionType, target: &FunctionType) -> bool {
    // relater.go `signaturesRelatedTo`: the comparable relation erases the
    // type parameters of both signatures instead of instantiating the source
    // in the target's context, and an unsubstituted placeholder already
    // relates like the `any` erasure leaves.
    if current_relation() == Relation::Comparable {
        return is_signature_assignable_to(source, target, false);
    }
    // `signaturesRelatedTo` relates an overload group member by member with
    // every signature's type parameters erased too; the fold surge holds in
    // its place is related the same erased way.
    // Against one target signature some source overload must relate on its
    // own; the fold accepts what no single member does.
    if let Some(members) = source.overloads()
        && target.overloads().is_none()
        && !members.is_empty()
    {
        // `signaturesRelatedTo` relates each pair with the generics erased.
        return members
            .iter()
            .any(|member| is_signature_assignable_to(member, target, false));
    }
    // relater.go `signaturesRelatedTo` with several target signatures: each
    // needs some source signature related to it. Only the arity half of that is
    // decided per signature (`compareSignaturesRelated`: a source needing more
    // arguments than a target without a rest supplies); the types go through
    // the fold, as a source typed from its contextual signature can read the
    // members of the one it was typed under.
    if let Some(target_members) = target.overloads()
        && !target_members.is_empty()
    {
        let source_members = match source.overloads().filter(|members| !members.is_empty()) {
            Some(members) => members,
            None => std::slice::from_ref(source),
        };
        let arity_fits = |source: &FunctionType, target: &FunctionType| {
            target.is_variadic() || source.required_parameter_count() <= target.parameters().len()
        };
        if !target_members.iter().all(|target_member| {
            source_members.iter().any(|source_member| arity_fits(source_member, target_member))
        }) {
            return false;
        }
    }
    if source.overloads().is_some() || target.overloads().is_some() {
        return is_signature_assignable_to(source, target, false);
    }
    generic_signatures_related(source, target, false)
}

/// `compareSignaturesRelated` for a member declared with method syntax, whose
/// parameters compare bivariantly; a generic side is related as
/// [`is_function_assignable_to`] relates it.
pub fn is_method_assignable_to(source: &FunctionType, target: &FunctionType) -> bool {
    if source.overloads().is_some() || target.overloads().is_some() {
        return is_signature_assignable_to(source, target, true);
    }
    generic_signatures_related(source, target, true)
}

fn generic_signatures_related(source: &FunctionType, target: &FunctionType, bivariant_parameters: bool) -> bool {
    if source.generic_shape().is_some() || target.generic_shape().is_some() {
        return written_generic_signatures_related(source, target, bivariant_parameters);
    }
    // `compareSignaturesRelated`: a generic target keeps its own type
    // parameters (`getCanonicalSignature`), and a generic source is
    // instantiated in the context of that canonical signature.
    if let Some(canonical_target) = opaque_generic_target(target) {
        let instantiated = related_generic_source_in_context_of(source, &canonical_target);
        return is_signature_assignable_to(
            instantiated.as_ref().unwrap_or(source),
            &canonical_target,
            bivariant_parameters,
        );
    }
    if let Some(instantiated) = related_generic_source_in_context_of(source, target) {
        return is_signature_assignable_to(&instantiated, target, bivariant_parameters);
    }
    is_signature_assignable_to(source, target, bivariant_parameters)
}

/// The two signatures `compareSignaturesRelated` compares once generics are
/// settled: the target's canonical signature and the source instantiated in
/// its context, as [`generic_signatures_related`] relates them. The target's
/// type variables stay bound while the value is held. `None` when neither
/// side is generic.
pub struct CanonicalSignatures {
    _scope: Option<crate::type_variable::TypeVariableScope>,
    pub source: FunctionType,
    pub target: FunctionType,
}

pub fn canonical_signatures(source: &FunctionType, target: &FunctionType) -> Option<CanonicalSignatures> {
    if source.generic_shape().is_some() || target.generic_shape().is_some() {
        let canonical = target.generic_shape().map(|shape| canonical_signature(target, shape));
        let (scope, canonical_target) = match canonical {
            Some((scope, canonical_target)) => (Some(scope), canonical_target),
            None if target.type_parameter_head().is_some() => (None, opaque_generic_target(target)?),
            None => (None, target.clone()),
        };
        let instantiated_source = match source.generic_shape() {
            Some(shape) => shape_in_context_of(source, shape, &canonical_target),
            None => related_generic_source_in_context_of(source, &canonical_target).unwrap_or_else(|| source.clone()),
        };
        return Some(CanonicalSignatures { _scope: scope, source: instantiated_source, target: canonical_target });
    }
    if let Some(canonical_target) = opaque_generic_target(target) {
        let instantiated =
            related_generic_source_in_context_of(source, &canonical_target).unwrap_or_else(|| source.clone());
        return Some(CanonicalSignatures { _scope: None, source: instantiated, target: canonical_target });
    }
    related_generic_source_in_context_of(source, target)
        .map(|instantiated| CanonicalSignatures { _scope: None, source: instantiated, target: target.clone() })
}

/// `compareSignaturesRelated` (relater.go:1485) where a side was written with
/// a constrained type parameter. The target keeps its own type parameters
/// (`getCanonicalSignature`, checker.go:19314): each is a type variable
/// related through its constraint, and nothing but itself is assignable to
/// it. A generic source is then instantiated in the target's context
/// (`instantiateSignatureInContextOf`, checker.go:19371). A constrained target
/// without a written shape keeps the erased comparison.
fn written_generic_signatures_related(
    source: &FunctionType,
    target: &FunctionType,
    bivariant_parameters: bool,
) -> bool {
    let canonical = target.generic_shape().map(|shape| canonical_signature(target, shape));
    let canonical_target = match &canonical {
        Some((_, canonical_target)) => canonical_target.clone(),
        None if target.type_parameter_head().is_some() => match opaque_generic_target(target) {
            Some(opaque) => opaque,
            None => return is_signature_assignable_to(source, target, bivariant_parameters),
        },
        None => target.clone(),
    };
    let instantiated_source = match source.generic_shape() {
        Some(shape) => shape_in_context_of(source, shape, &canonical_target),
        None => related_generic_source_in_context_of(source, &canonical_target).unwrap_or_else(|| source.clone()),
    };
    let related = is_signature_assignable_to(&instantiated_source, &canonical_target, bivariant_parameters);
    // Their payloads key the relation memo until the outermost query ends.
    SYNTHESIZED_TARGETS.with(|targets| {
        let mut targets = targets.borrow_mut();
        targets.push(Type::Function(canonical_target));
        targets.push(Type::Function(instantiated_source));
    });
    related
}

/// `getCanonicalSignature` over a written shape: the signature's own type
/// parameters bound as the variables of a fresh scope, each with its
/// constraint, for as long as the scope is held.
fn canonical_signature(
    function: &FunctionType,
    shape: &crate::GenericSignatureShape,
) -> (crate::type_variable::TypeVariableScope, FunctionType) {
    let variables = crate::type_variable::TypeVariableScope::enter(
        shape
            .type_parameters
            .iter()
            .map(|(name, _)| (name.clone(), (Arc::<str>::from(""), 0u32))),
    );
    let names: Vec<String> = shape.type_parameters.iter().map(|(name, _)| name.to_string()).collect();
    let variable = |name: &str| variables.variable(name);
    let mut changed = false;
    for (name, constraint) in &shape.type_parameters {
        if let Some(constraint) = constraint {
            variables.set_constraint(name, substitute_type_parameters(constraint, &names, &variable, &mut changed));
        }
    }
    let parameters = shape
        .parameters
        .iter()
        .map(|parameter| substitute_type_parameters(parameter, &names, &variable, &mut changed))
        .collect();
    let return_type = substitute_type_parameters(&shape.return_type, &names, &variable, &mut changed);
    let canonical = FunctionType::new(
        parameters,
        return_type,
        function.is_variadic(),
        function.required_parameter_count(),
    );
    (variables, canonical)
}

/// `instantiateSignatureInContextOf` for a source written with a constrained
/// type parameter: the inference runs over its written shape.
fn shape_in_context_of(
    source: &FunctionType,
    shape: &crate::GenericSignatureShape,
    target: &FunctionType,
) -> FunctionType {
    let written = FunctionType::new(
        shape.parameters.clone(),
        shape.return_type.clone(),
        source.is_variadic(),
        source.required_parameter_count(),
    );
    let names: Vec<String> = shape.type_parameters.iter().map(|(name, _)| name.to_string()).collect();
    let constraints: Vec<Option<Type>> =
        shape.type_parameters.iter().map(|(_, constraint)| constraint.clone()).collect();
    instantiate_in_context_of(&written, &names, &constraints, target, true).unwrap_or(written)
}

/// `ty` with the type parameters named `names` read as the sentinel, which is
/// what a written generic signature resolves its own to.
/// `signature` with its own type parameters `names` read as `replacement`:
/// `unknown` for tsc's `getBaseSignature` of unconstrained ones, as inference
/// reads a generic source signature (`inferFromSignatures`).
pub fn signature_with_type_parameters_as(signature: &FunctionType, names: &[String], replacement: &Type) -> FunctionType {
    let mut changed = false;
    let replace = |_: &str| replacement.clone();
    let parameters = signature
        .parameters()
        .iter()
        .map(|parameter| substitute_type_parameters(parameter, names, &replace, &mut changed))
        .collect();
    let return_type = substitute_type_parameters(signature.return_type(), names, &replace, &mut changed);
    FunctionType::new(parameters, return_type, signature.is_variadic(), signature.required_parameter_count())
}

pub(crate) fn erase_type_parameters(ty: &Type, names: &[String]) -> Type {
    if names.is_empty() {
        return ty.clone();
    }
    let mut changed = false;
    substitute_type_parameters(ty, names, &|_: &str| Type::Unknown, &mut changed)
}

/// tsc's `instantiateSignatureInContextOf`: a generic source compared with a
/// non-generic target is first instantiated with what the target's parameters
/// infer for its type parameters, so `<T>(x: T) => T[]` against
/// `(x: number) => string[]` compares `number[]` with `string[]` and fails.
/// surge's placeholders relate like `unknown`, which accepted every such pair.
/// A constrained parameter stays a placeholder: the head is display text and
/// carries no resolved constraint to fall back to when the inference misses it.
pub fn generic_source_in_context_of(source: &FunctionType, target: &FunctionType) -> Option<FunctionType> {
    in_context_of(source, target, false)
}

/// [`generic_source_in_context_of`] for a relation, which has no inference
/// context of its own: every type parameter is inferred from the return types
/// too (`applyToReturnTypes` when `inferenceContext` is nil), so `<T, U>(x: T)
/// => U` against `(x: boolean) => boolean` reads `U` as `boolean`.
fn related_generic_source_in_context_of(source: &FunctionType, target: &FunctionType) -> Option<FunctionType> {
    in_context_of(source, target, true)
}

fn in_context_of(source: &FunctionType, target: &FunctionType, from_return_types: bool) -> Option<FunctionType> {
    if target.type_parameter_head().is_some() {
        return None;
    }
    let head = source.type_parameter_head()?;
    let names: Vec<String> = head_segments(head)
        .into_iter()
        .filter(|segment| !segment.is_empty() && !segment.contains(' ') && !segment.contains('<'))
        .map(String::from)
        .collect();
    if names.is_empty() {
        return None;
    }
    instantiate_in_context_of(source, &names, &vec![None; names.len()], target, from_return_types)
}

/// [`generic_source_in_context_of`] over `names`, each with its constraint.
/// `getInferredType` (inference.go:1283) keeps an inference that satisfies
/// the constraint instantiated with the other inferences; a pure return type
/// inference keeps the union members that do; failing that the other
/// variance's inference stands if it satisfies it, and failing that the
/// constraint itself. A constrained parameter the parameters infer nothing for
/// is inferred from the return type (`applyToReturnTypes`, at the lower
/// `InferencePriorityReturnType`).
fn instantiate_in_context_of(
    source: &FunctionType,
    names: &[String],
    constraints: &[Option<Type>],
    target: &FunctionType,
    from_return_types: bool,
) -> Option<FunctionType> {
    let empty_candidates = || -> Vec<InferenceCandidates> {
        names
            .iter()
            .map(|name| InferenceCandidates {
                name: name.clone(),
                covariant: Vec::new(),
                contravariant: Vec::new(),
            })
            .collect()
    };
    let mut candidates = empty_candidates();
    // `getTypeAtPosition` reads an optional parameter with its `undefined`,
    // a type parameter's too, which the target's `undefined` then matches.
    let with_optionality = |function: &FunctionType, index: usize, ty: Type| {
        if crate::strict_null_checks()
            && index >= function.required_parameter_count()
            && !(function.is_variadic() && index + 1 >= function.parameters().len())
            && (!ty.is_unmodelled() || matches!(ty, Type::TypeParameter(_)))
            && !type_includes_undefined(&ty)
        {
            crate::union_type(vec![ty, Type::Undefined])
        } else {
            ty
        }
    };
    for index in 0..source.parameters().len() {
        let (Some(source_parameter), Some(target_parameter)) =
            (parameter_type_at(source, index), parameter_type_at(target, index))
        else {
            continue;
        };
        let source_parameter = with_optionality(source, index, source_parameter);
        let target_parameter = with_optionality(target, index, target_parameter);
        infer_to_type_parameters(&source_parameter, &target_parameter, &mut candidates, false, 0);
    }
    let constrained = constraints.iter().any(Option::is_some);
    let mut return_candidates = empty_candidates();
    if constrained || from_return_types {
        infer_to_type_parameters(source.return_type(), target.return_type(), &mut return_candidates, false, 0);
    }
    // Per name: the inference `getInferredType` prefers, the other variance's
    // as its fallback, and whether the return type alone supplied it.
    let inferences: Vec<Option<(Type, Option<Type>, bool)>> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let constraint = constraints.get(index).and_then(Option::as_ref);
            preferred_inference(&candidates[index], source, name, constraint)
                .map(|(inferred, fallback)| (inferred, fallback, false))
                .or_else(|| {
                    if constraint.is_none() && !from_return_types {
                        return None;
                    }
                    preferred_inference(&return_candidates[index], source, name, constraint)
                        .map(|(inferred, fallback)| (inferred, fallback, true))
                })
        })
        .collect();
    let mut inferred: Vec<Type> = names
        .iter()
        .zip(&inferences)
        .map(|(name, inference)| match inference {
            Some((inferred, _, _)) => inferred.clone(),
            None => Type::type_parameter(name),
        })
        .collect();
    for index in 0..names.len() {
        let Some(constraint) = constraints.get(index).and_then(Option::as_ref) else {
            continue;
        };
        let instantiated_constraint = {
            let current = |name: &str| match names.iter().position(|own| own == name) {
                Some(position) => inferred[position].clone(),
                None => Type::type_parameter(name),
            };
            substitute_type_parameters(constraint, names, &current, &mut false)
        };
        let satisfies = |ty: &Type| is_assignable_to(ty, &instantiated_constraint);
        let chosen = match &inferences[index] {
            Some((candidate, _, _)) if satisfies(candidate) => candidate.clone(),
            Some((Type::Union(union), _, true)) if union.types().iter().any(|member| satisfies(member)) => {
                crate::union_type(union.types().iter().filter(|&member| satisfies(member)).cloned().collect())
            }
            Some((_, Some(fallback), _)) if satisfies(fallback) => fallback.clone(),
            _ => instantiated_constraint.clone(),
        };
        inferred[index] = chosen;
    }
    let resolved = |name: &str| match names.iter().position(|own| own == name) {
        Some(position) => inferred[position].clone(),
        None => Type::type_parameter(name),
    };
    let mut changed = false;
    let parameters: Vec<Type> = source
        .parameters()
        .iter()
        .map(|parameter| substitute_type_parameters(parameter, names, &resolved, &mut changed))
        .collect();
    let return_type = substitute_type_parameters(source.return_type(), names, &resolved, &mut changed);
    let resolved_any = constrained
        || candidates
            .iter()
            .chain(return_candidates.iter().filter(|_| from_return_types))
            .any(|inference| !inference.covariant.is_empty() || !inference.contravariant.is_empty());
    (changed && resolved_any).then(|| {
        FunctionType::new(
            parameters,
            return_type,
            source.is_variadic(),
            source.required_parameter_count(),
        )
    })
}

/// `getInferredType` before its constraint check: the covariant inference
/// when it fits some contravariant candidate, else the contravariant one,
/// with the other as the fallback. `None` without a candidate.
fn preferred_inference(
    inference: &InferenceCandidates,
    source: &FunctionType,
    name: &str,
    constraint: Option<&Type>,
) -> Option<(Type, Option<Type>)> {
    // `getCovariantInference` widens literal candidates of a parameter the
    // return type does not expose at its top level, unless its constraint
    // admits primitives.
    let widened: Vec<Type>;
    let covariant = if type_parameter_at_top_level(source.return_type(), name)
        || constraint.is_some_and(has_primitive_constraint)
    {
        inference.covariant.as_slice()
    } else {
        widened = inference.covariant.iter().map(widen_literal_candidate).collect();
        widened.as_slice()
    };
    // `getCommonSupertype`: literals of one primitive combine into their
    // union (`literalTypesWithSameBaseType`); otherwise the leftmost candidate
    // every other one is assignable to, and with none the first stands and the
    // comparison reports the disagreement.
    let literal_union = constraint.and_then(|_| literal_candidates_union(covariant));
    let supertype = literal_union.or_else(|| {
        covariant
            .iter()
            .find(|candidate| covariant.iter().all(|other| is_assignable_to(other, candidate)))
            .or_else(|| covariant.first())
            .cloned()
    });
    // `getCommonSubtype`: the leftmost candidate no later one is a subtype of.
    let subtype = inference.contravariant.iter().fold(None::<&Type>, |subtype, candidate| match subtype {
        Some(subtype) if !is_assignable_to(candidate, subtype) => Some(subtype),
        _ => Some(candidate),
    });
    match (supertype, subtype) {
        (Some(supertype), Some(subtype)) => {
            let prefer_covariant = !matches!(supertype, Type::Never | Type::Any)
                && inference
                    .contravariant
                    .iter()
                    .any(|candidate| is_assignable_to(&supertype, candidate));
            Some(if prefer_covariant {
                (supertype, Some(subtype.clone()))
            } else {
                (subtype.clone(), Some(supertype))
            })
        }
        (Some(inferred), None) => Some((inferred, None)),
        (None, Some(inferred)) => Some((inferred.clone(), None)),
        (None, None) => None,
    }
}

/// `literalTypesWithSameBaseType`: every candidate a literal of one primitive.
fn literal_candidates_union(candidates: &[Type]) -> Option<Type> {
    let base = |ty: &Type| match ty {
        Type::StringLiteral(_) => Some(Type::String),
        Type::NumberLiteral(_) => Some(Type::Number),
        Type::BooleanLiteral(_) => Some(Type::Boolean),
        _ => None,
    };
    let first = base(candidates.first()?)?;
    (candidates.len() > 1 && candidates.iter().all(|candidate| base(candidate).as_ref() == Some(&first)))
        .then(|| crate::union_type(candidates.to_vec()))
}

/// `hasPrimitiveConstraint`: the constraint admits a primitive or literal type,
/// so a literal inference for it is kept as it is. One surge could not model
/// keeps it too.
fn has_primitive_constraint(constraint: &Type) -> bool {
    match constraint {
        Type::Union(union) => union.types().iter().any(has_primitive_constraint),
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::Unknown
        | Type::ErrorType => true,
        other => other.base_primitive().is_some(),
    }
}

/// `isTypeParameterAtTopLevel`: the type is the parameter or a union with it
/// as a member.
fn type_parameter_at_top_level(ty: &Type, name: &str) -> bool {
    match ty {
        Type::TypeParameter(parameter) => *parameter.name == *name && !ty.is_type_variable(),
        Type::Union(union) => union.types().iter().any(|member| type_parameter_at_top_level(member, name)),
        _ => false,
    }
}

/// `getWidenedLiteralType` of an inference candidate.
fn widen_literal_candidate(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Union(union) => crate::union_type(union.types().iter().map(widen_literal_candidate).collect()),
        other => other.clone(),
    }
}

/// tsc's `getTypeAtPosition`: a rest parameter answers for every position it
/// covers — its element for an array, the element at that offset for a tuple.
fn parameter_type_at(function: &FunctionType, index: usize) -> Option<Type> {
    let parameters = function.parameters();
    let last = parameters.len().checked_sub(1)?;
    if !function.is_variadic() || index < last {
        return parameters.get(index).cloned();
    }
    let offset = index - last;
    match parameters[last].peeled() {
        Type::Array(element) => Some(*element),
        Type::Tuple(elements) => elements.get(offset).cloned(),
        Type::OpenTuple(tuple) => Some(
            tuple
                .leading
                .get(offset)
                .cloned()
                .unwrap_or_else(|| tuple.rest.as_ref().clone()),
        ),
        other => Some(other),
    }
}

/// What one named type parameter of a generic source has been inferred from,
/// split by the variance of the position (`InferenceInfo.candidates` and
/// `contraCandidates`).
struct InferenceCandidates {
    name: String,
    covariant: Vec<Type>,
    contravariant: Vec<Type>,
}

/// Collects, for each named type parameter, the target types standing where
/// the source writes it. A parameter position of a nested signature flips the
/// variance (`inferFromContravariantTypesIfStrictFunctionTypes`).
fn infer_to_type_parameters(
    source: &Type,
    target: &Type,
    candidates: &mut Vec<InferenceCandidates>,
    contravariant: bool,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    match (source, target) {
        (Type::TypeParameter(parameter), target) if !source.is_type_variable() => {
            if target.is_unmodelled() || matches!(target, Type::Any) {
                return;
            }
            if let Some(inference) = candidates
                .iter_mut()
                .find(|inference| *inference.name == *parameter.name)
            {
                let types = if contravariant {
                    &mut inference.contravariant
                } else {
                    &mut inference.covariant
                };
                if !types.contains(target) {
                    types.push(target.clone());
                }
            }
        }
        (Type::Array(source), Type::Array(target)) => {
            infer_to_type_parameters(source, target, candidates, contravariant, depth + 1);
        }
        (Type::Tuple(source), Type::Tuple(target)) => {
            for (source, target) in source.iter().zip(target) {
                infer_to_type_parameters(source, target, candidates, contravariant, depth + 1);
            }
        }
        // A nested signature that declares one of the names itself shadows it
        // (`getErasedSignature` leaves nothing there to infer to).
        (Type::Function(source), Type::Function(target))
            if !source
                .type_parameter_names()
                .iter()
                .any(|own| candidates.iter().any(|inference| inference.name == *own)) =>
        {
            for (source, target) in source.parameters().iter().zip(target.parameters()) {
                infer_to_type_parameters(source, target, candidates, !contravariant, depth + 1);
            }
            infer_to_type_parameters(source.return_type(), target.return_type(), candidates, contravariant, depth + 1);
        }
        (Type::Reference(source), Type::Reference(target))
            if source.id == target.id && source.arguments.len() == target.arguments.len() =>
        {
            for (source, target) in source.arguments.iter().zip(target.arguments.iter()) {
                infer_to_type_parameters(source, target, candidates, contravariant, depth + 1);
            }
        }
        (Type::Object(source), Type::Object(target)) => {
            infer_from_object_members(source, target, candidates, contravariant, depth + 1);
        }
        // `inferFromObjectTypes` reads a reference's resolved members when the
        // other side is not the same reference.
        (Type::Reference(reference), Type::Object(_)) if !reference.is_unique_symbol() => {
            if let Type::Object(source) = reference.resolve()
                && let Type::Object(target) = target
            {
                infer_from_object_members(&source, target, candidates, contravariant, depth + 1);
            }
        }
        // `inferFromMatchingTypes`: members both unions share are matched
        // off, and what remains of the target infers to the source's naked
        // type parameter.
        (Type::Union(source_union), target) => {
            let target_members: Vec<Type> = match target {
                Type::Union(target_union) => target_union.types().to_vec(),
                other => vec![other.clone()],
            };
            let (naked, fixed): (Vec<&Type>, Vec<&Type>) = source_union
                .types()
                .iter()
                .partition(|member| matches!(member, Type::TypeParameter(_)) && !member.is_type_variable());
            let [naked] = naked.as_slice() else {
                return;
            };
            let remaining: Vec<Type> =
                target_members.into_iter().filter(|member| !fixed.iter().any(|fixed| *fixed == member)).collect();
            if !remaining.is_empty() {
                infer_to_type_parameters(naked, &crate::union_type(remaining), candidates, contravariant, depth + 1);
            }
        }
        _ => {}
    }
}

/// `inferFromObjectTypes` between two object types: same-named properties,
/// then call and construct signatures, then index signatures — unless each
/// declares a required property the other lacks (`typesDefinitelyUnrelated`).
fn infer_from_object_members(
    source: &ObjectType,
    target: &ObjectType,
    candidates: &mut Vec<InferenceCandidates>,
    contravariant: bool,
    depth: usize,
) {
    let lacks_required_member_of = |object: &ObjectType, other: &ObjectType| {
        other
            .properties
            .iter()
            .any(|(name, property)| !property.optional && !object.properties.contains_key(name.as_ref()))
    };
    if lacks_required_member_of(source, target) && lacks_required_member_of(target, source) {
        return;
    }
    // `getTypeOfSymbol` reads an optional property with its `undefined`.
    let property_type = |property: &crate::ObjectProperty| {
        if property.optional
            && crate::strict_null_checks()
            && !property.ty.is_unmodelled()
            && !type_includes_undefined(&property.ty)
        {
            crate::union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        }
    };
    for (name, source_property) in source.properties.iter() {
        if let Some(target_property) = target.properties.get(name.as_ref()) {
            infer_to_type_parameters(
                &property_type(source_property),
                &property_type(target_property),
                candidates,
                contravariant,
                depth,
            );
        }
    }
    for (source_signature, target_signature) in [
        (source.call_signature(), target.call_signature()),
        (source.construct_signature(), target.construct_signature()),
    ] {
        if let (Some(source_signature), Some(target_signature)) = (source_signature, target_signature) {
            infer_to_type_parameters(
                &Type::Function(source_signature.clone()),
                &Type::Function(target_signature.clone()),
                candidates,
                contravariant,
                depth,
            );
        }
    }
    if let (Some(source_index), Some(target_index)) =
        (source.string_index_type.as_deref(), target.string_index_type.as_deref())
    {
        infer_to_type_parameters(source_index, target_index, candidates, contravariant, depth);
    }
    if let (Some(source_index), Some(target_index)) =
        (source.number_index_type.as_deref(), target.applicable_index_type(true))
    {
        infer_to_type_parameters(source_index, target_index, candidates, contravariant, depth);
    }
}

/// tsc's `getCanonicalSignature`: a generic target keeps its type parameters
/// as they are, whatever the source: `T` in `<T>(x: T) => T[]` is a type of
/// its own that `number` does not satisfy, so neither `(x: number) =>
/// number[]` nor `<U>(x: U) => string[]` is assignable to it. surge's
/// type-parameter placeholders relate like `unknown`, so for this comparison
/// each of the target's parameters is replaced with an opaque type only it
/// can inhabit. Constrained parameters are left alone: the head is display
/// text and does not carry a resolved constraint to relate through.
/// A rendered type-parameter list split at its top-level commas: a constraint
/// may itself hold a comma (`T extends Record<string, X>`).
fn head_segments(head: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (index, character) in head.char_indices() {
        match character {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if head[..index].ends_with('=') => {}
            '>' | ')' | ']' | '}' => depth -= 1,
            ',' if depth <= 0 => {
                segments.push(head[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    segments.push(head[start..].trim());
    segments
}

fn opaque_generic_target(target: &FunctionType) -> Option<FunctionType> {
    let head = target.type_parameter_head()?;
    let mut names = Vec::new();
    for segment in head_segments(head) {
        if segment.is_empty() || segment.contains(' ') || segment.contains('<') {
            return None;
        }
        names.push(segment.to_string());
    }
    let opaque = |name: &str| {
        let mut properties = crate::PropertyMap::default();
        properties.insert(
            format!("\u{0}type parameter {name}").into(),
            crate::ObjectProperty::required(Type::Never),
        );
        Type::Object(ObjectType::new(properties, None))
    };
    let mut changed = false;
    let substitute = |ty: &Type, changed: &mut bool| substitute_type_parameters(ty, &names, &opaque, changed);
    let parameters: Vec<Type> = target
        .parameters()
        .iter()
        .map(|parameter| substitute(parameter, &mut changed))
        .collect();
    let return_type = substitute(target.return_type(), &mut changed);
    changed.then(|| {
        FunctionType::new(
            parameters,
            return_type,
            target.is_variadic(),
            target.required_parameter_count(),
        )
    })
}

/// `getSignatureInstantiation` for a signature whose own type parameters are
/// known only by name: each placeholder named in `names` is replaced by the
/// type at the same position.
pub fn instantiate_named_type_parameters(function: &FunctionType, names: &[String], types: &[Type]) -> FunctionType {
    // The signature's own parameters shadow any outer one of the same name,
    // bound as a type variable or not.
    fn replace(ty: &Type, names: &[String], types: &[Type]) -> Type {
        match ty {
            Type::TypeParameter(parameter) => names
                .iter()
                .position(|name| **name == *parameter.name)
                .and_then(|index| types.get(index).cloned())
                .unwrap_or_else(|| ty.clone()),
            Type::Array(element) => Type::Array(Box::new(replace(element, names, types))),
            Type::Tuple(elements) => Type::Tuple(elements.iter().map(|element| replace(element, names, types)).collect()),
            Type::Union(union) => crate::union_type(union.types().iter().map(|member| replace(member, names, types)).collect()),
            Type::Function(function) => Type::Function(replace_in_signature(function, names, types)),
            other => other.clone(),
        }
    }
    fn replace_in_signature(function: &FunctionType, names: &[String], types: &[Type]) -> FunctionType {
        FunctionType::new(
            function.parameters().iter().map(|parameter| replace(parameter, names, types)).collect(),
            replace(function.return_type(), names, types),
            function.is_variadic(),
            function.required_parameter_count(),
        )
    }
    replace_in_signature(function, names, types)
}

fn substitute_type_parameters(
    ty: &Type,
    names: &[String],
    opaque: &dyn Fn(&str) -> Type,
    changed: &mut bool,
) -> Type {
    match ty {
        Type::TypeParameter(parameter)
            if !ty.is_type_variable() && names.iter().any(|name| **name == *parameter.name) =>
        {
            *changed = true;
            opaque(&parameter.name)
        }
        Type::Array(element) => Type::Array(Box::new(substitute_type_parameters(element, names, opaque, changed))),
        Type::Tuple(elements) => Type::Tuple(
            elements
                .iter()
                .map(|element| substitute_type_parameters(element, names, opaque, changed))
                .collect(),
        ),
        Type::Reference(reference) => {
            let mut arguments_changed = false;
            let arguments: Vec<Type> = reference.arguments.iter().map(|argument| {
                substitute_type_parameters(argument, names, opaque, &mut arguments_changed)
            }).collect();
            if !arguments_changed {
                return ty.clone();
            }
            *changed = true;
            let resolver = SignatureReferenceSubstitution {
                original: reference.clone(),
                names: names.to_vec().into(),
                replacements: names.iter().map(|name| opaque(name)).collect::<Vec<_>>().into(),
            };
            let mut substituted = crate::TypeReference::new(
                reference.id.clone(),
                reference.display.clone(),
                arguments,
                Arc::new(resolver),
            );
            substituted.render_structurally = reference.render_structurally;
            substituted.numeric_enum = reference.numeric_enum;
            substituted.enum_owner = reference.enum_owner.clone();
            substituted.enum_base = reference.enum_base.clone();
            substituted.enum_members = reference.enum_members.clone();
            substituted.alias_display = reference.alias_display.clone();
            substituted.non_augmenting_subtype = reference.non_augmenting_subtype;
            substituted.intersection_alias = reference.intersection_alias;
            substituted.declared_variances = reference.declared_variances;
            Type::Reference(substituted)
        }
        Type::Union(union) => crate::union_type(
            union
                .types()
                .iter()
                .map(|member| substitute_type_parameters(member, names, opaque, changed))
                .collect(),
        ),
        Type::Function(function) => {
            // A nested generic signature's own type parameters shadow the
            // outer ones of the same name.
            let own = function.type_parameter_names();
            let unshadowed: Vec<String>;
            let names = if own.is_empty() {
                names
            } else {
                unshadowed = names.iter().filter(|name| !own.contains(name)).cloned().collect();
                unshadowed.as_slice()
            };
            Type::Function(FunctionType::new(
                function
                    .parameters()
                    .iter()
                    .map(|parameter| substitute_type_parameters(parameter, names, opaque, changed))
                    .collect(),
                substitute_type_parameters(function.return_type(), names, opaque, changed),
                function.is_variadic(),
                function.required_parameter_count(),
            ))
        }
        // A written object type (`{ a: T; b: T }`). An intersection surface,
        // an open one, a nominal declaration or the `object` keyword is left
        // as it is: rebuilding it here would drop what makes it that.
        Type::Object(object)
            if object.alias_id.is_none()
                && !object.is_intersection
                && !object.synthetic_open_index
                && !object.non_primitive
                && object.intersection_operands.is_none() =>
        {
            let mut object_changed = false;
            let properties: crate::PropertyMap = object
                .properties
                .iter()
                .map(|(name, property)| {
                    let mut property = property.clone();
                    property.ty = substitute_type_parameters(&property.ty, names, opaque, &mut object_changed);
                    (name.clone(), property)
                })
                .collect();
            let string_index = object
                .string_index_type
                .as_deref()
                .map(|index| substitute_type_parameters(index, names, opaque, &mut object_changed));
            let number_index = object
                .number_index_type
                .as_deref()
                .map(|index| substitute_type_parameters(index, names, opaque, &mut object_changed));
            if !object_changed {
                return ty.clone();
            }
            *changed = true;
            let mut substituted = ObjectType::new(properties, string_index).with_number_index_type(number_index);
            if object.without_inferable_index {
                substituted = substituted.with_nominal_declaration_marker();
            }
            if let Some(signature) = object.call_signature() {
                substituted = substituted.with_call_signature(signature.clone());
            }
            if let Some(signature) = object.construct_signature() {
                substituted = substituted.with_construct_signature(signature.clone());
            }
            Type::Object(substituted)
        }
        other => other.clone(),
    }
}

struct SignatureReferenceSubstitution {
    original: crate::TypeReference,
    names: Arc<[String]>,
    replacements: Arc<[Type]>,
}

impl crate::ResolveReference for SignatureReferenceSubstitution {
    fn resolve(&self) -> Type {
        let resolved = self.original.resolve_arc();
        let structural = match resolved.as_ref() {
            Type::Object(object) => {
                let mut object = object.clone();
                object.alias_id = None;
                Type::Object(object)
            }
            other => other.clone(),
        };
        let replacement = |name: &str| {
            self.names.iter().position(|own| own == name)
                .map(|index| self.replacements[index].clone())
                .unwrap_or_else(|| Type::type_parameter(name))
        };
        substitute_type_parameters(&structural, &self.names, &replacement, &mut false)
    }

    fn retains_resolution_context(&self) -> bool {
        true
    }

    fn captured_census(&self) -> crate::ResolverCaptureCensus {
        self.original.captured_census()
    }

    fn supports_program_canonicalization(&self) -> bool {
        // These bindings belong to one signature comparison, not the program.
        false
    }
}

/// A member value against the member it fills: one declared with method
/// syntax compares its parameters bivariantly (`compareSignaturesRelated`
/// for a method declaration), even under `strictFunctionTypes`.
pub fn is_member_assignable_to(source: &Type, target: &Type, method: bool) -> bool {
    if method
        && let (Type::Function(source_signature), Type::Function(target_signature)) =
            (source.peeled(), target.peeled())
    {
        return is_signature_assignable_to(&source_signature, &target_signature, true);
    }
    is_assignable_to(source, target)
}

/// `bivariant_parameters` relaxes the contravariant parameter test to accept
/// either direction. tsc applies exactly that relaxation to members declared
/// with method syntax (`m(x: T): U`), even under `strictFunctionTypes` — an
/// override whose parameter is a *subtype* of the base's still satisfies it.
fn is_signature_assignable_to(
    source: &FunctionType,
    target: &FunctionType,
    bivariant_parameters: bool,
) -> bool {
    // `compareSignaturesRelated`'s `strictVariance` is off for a target
    // declared by a method, however the signature was reached, and for every
    // target without `strictFunctionTypes`.
    signature_related_in_mode(
        source,
        target,
        bivariant_parameters || target.is_method_declaration() || !crate::strict_function_types(),
        CallbackMode::None,
    )
}

/// The index signatures an augmentation adds to `Function`, which a function's
/// apparent members do not supply (`indexSignaturesRelatedTo`): a function or
/// class type has no inferable index, so only its own index signatures can.
fn function_interface_indexes_related(from: &Type, function_interface: &Type) -> bool {
    let Type::Object(target) = function_interface.peeled() else {
        return true;
    };
    if target.string_index_type.is_none() && target.number_index_type.is_none() {
        return true;
    }
    let Type::Object(source) = from.peeled() else {
        return false;
    };
    let related = |source_index: Option<&Type>, target_index: &Type| {
        source_index.is_some_and(|source_index| is_assignable_to(source_index, target_index))
    };
    target
        .string_index_type
        .as_deref()
        .is_none_or(|index| related(source.string_index_type.as_deref(), index))
        && target.number_index_type.as_deref().is_none_or(|index| {
            related(source.number_index_type.as_deref().or(source.string_index_type.as_deref()), index)
        })
}

/// relater.go `SignatureCheckModeStrictCallback` / `BivariantCallback`: the
/// mode two callback parameters are compared in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CallbackMode {
    None,
    Strict,
    Bivariant,
}

/// relater.go `isEnumTypeRelatedTo` for two different enums: the same name,
/// both regular, and every source member in the target with the same value —
/// or, where one value is unknown (computed), neither a string.
fn different_enums_related(source: &crate::TypeReference, target: &crate::TypeReference) -> bool {
    let enum_name = |owner: &str| owner.rsplit(['\0', '.']).next().unwrap_or(owner).to_string();
    let (Some(source_owner), Some(target_owner)) = (&source.enum_owner, &target.enum_owner) else {
        return false;
    };
    let (Some(source_members), Some(target_members)) = (&source.enum_members, &target.enum_members) else {
        return false;
    };
    if enum_name(source_owner) != enum_name(target_owner) || !source_members.regular || !target_members.regular {
        return false;
    }
    let is_string = |value: &Option<Type>| matches!(value, Some(Type::StringLiteral(_)));
    source_members.members.iter().all(|(name, source_value)| {
        let Some((_, target_value)) = target_members.members.iter().find(|(target_name, _)| target_name == name)
        else {
            return false;
        };
        match (source_value, target_value) {
            (Some(source_value), Some(target_value)) => source_value == target_value,
            _ => !is_string(source_value) && !is_string(target_value),
        }
    })
}

/// relater.go `compareTypePredicateRelatedTo`.
fn type_predicates_related(source: &crate::TypePredicate, target: &crate::TypePredicate) -> bool {
    use crate::TypePredicateKind::{AssertsIdentifier, Identifier};
    if source.kind != target.kind {
        return false;
    }
    if matches!(source.kind, Identifier | AssertsIdentifier) && source.parameter_index != target.parameter_index {
        return false;
    }
    match (&source.ty, &target.ty) {
        (None, None) => true,
        (Some(source_type), Some(target_type)) => source_type == target_type || is_assignable_to(source_type, target_type),
        _ => false,
    }
}

/// `getSingleCallSignature` of `getNonNullableType(ty)`.
fn single_call_signature(ty: &Type) -> Option<FunctionType> {
    let non_nullable = match ty.peeled() {
        Type::Union(union) => {
            let members: Vec<Type> = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null))
                .cloned()
                .collect();
            match members.len() {
                1 => members.into_iter().next()?,
                _ => return None,
            }
        }
        other => other.clone(),
    };
    match non_nullable.peeled() {
        Type::Function(function) if function.overloads().is_none() => Some(function.clone()),
        _ => None,
    }
}

/// `getTypeFacts(ty, TypeFactsIsUndefinedOrNull)`.
fn undefined_or_null_facts(ty: &Type) -> (bool, bool) {
    match ty.peeled() {
        Type::Undefined => (true, false),
        Type::Null => (false, true),
        Type::Union(union) => union.types().iter().fold((false, false), |facts, member| {
            let (undefined, null) = undefined_or_null_facts(member);
            (facts.0 || undefined, facts.1 || null)
        }),
        _ => (false, false),
    }
}

fn signature_related_in_mode(
    source: &FunctionType,
    target: &FunctionType,
    bivariant_parameters: bool,
    mode: CallbackMode,
) -> bool {
    if let Some(related) = tuple_union_rest_related(source, target, bivariant_parameters, mode) {
        return related && signature_returns_related(source, target, mode);
    }
    let (source_parameters, source_required, source_variadic) = expanded_signature(source);
    let (target_parameters, target_required, target_variadic) = expanded_signature(target);
    // tsc's `getTypeOfParameter`: an optional parameter's type includes
    // `undefined`, which is what the target passes when it omits the argument.
    // `(x: number) => void` therefore does not fit `(x?: number) => void`.
    let source_parameters =
        with_parameter_optionality(source_parameters, source_required, source_variadic);
    let target_parameters =
        with_parameter_optionality(target_parameters, target_required, target_variadic);
    let width = source_parameters.len().max(target_parameters.len());
    let source_parameters = widen_variadic_parameters(source_parameters, source_variadic, width);
    let target_parameters = widen_variadic_parameters(target_parameters, target_variadic, width);

    // A source function may declare fewer parameters than the target expects —
    // the surplus arguments the target would pass are simply ignored — but it
    // must not *require* more parameters than the target can ever supply. This
    // mirrors how tsc accepts `(v) => …` and `(v, i) => …` for an
    // `(element, index, array) => …` callback slot. The shared parameter prefix
    // is still checked bivariantly.
    let source_required = required_count_ignoring_trailing_void(&source_parameters, source_required);
    if !target_variadic && source_required > target_parameters.len() {
        return false;
    }

    let parameters_compatible = source_parameters.iter().zip(target_parameters.iter()).enumerate().all(
        |(index, (source_parameter, target_parameter))| {
            // A source parameter typed `unknown`/`any` accepts whatever argument
            // the target would supply, so it is contravariantly compatible with
            // any target parameter. This is what makes a generic call signature
            // whose unconstrained type parameter collapsed to `unknown` (e.g.
            // `BooleanConstructor`'s `<T>(value?: T) => boolean`) usable as a
            // typed callback such as an array predicate.
            // Contravariant, matching tsc under `strictFunctionTypes`: the
            // parameter the target would supply must be acceptable to the
            // source. Requiring covariance as well (the previous both-directions
            // rule) rejected a handler whose declared event type relates to the
            // slot's in only one direction, while pure covariance would wrongly
            // accept a literal-narrowed source (`(v: "idle") => void` as
            // `(v: string) => void`, TS2322 in tsc). A *degraded* target
            // parameter (one carrying the `unknown` sentinel in an argument or
            // member — e.g. a slot whose `KeyboardEvent<T>` kept an unresolved
            // `T`) cannot support the contravariant test (its `unknown` holes are
            // not assignable to the source's concrete members), so it falls back
            // to the covariant direction rather than flagging a handler tsc
            // accepts.
            if source_parameter == target_parameter
                || source_parameter.is_unmodelled()
                || matches!(source_parameter, Type::Any)
            {
                return true;
            }
            // `compareSignaturesRelated`: two callback parameters relate by
            // their signatures, target against source, so a type used only in
            // callback parameter positions is covariant.
            if mode == CallbackMode::None
                && !source.is_instantiated_generic_parameter(index)
                && !target.is_instantiated_generic_parameter(index)
                && let (Some(source_callback), Some(target_callback)) =
                    (single_call_signature(source_parameter), single_call_signature(target_parameter))
                && undefined_or_null_facts(source_parameter) == undefined_or_null_facts(target_parameter)
            {
                let callback_mode =
                    if bivariant_parameters { CallbackMode::Bivariant } else { CallbackMode::Strict };
                return signature_related_in_mode(&target_callback, &source_callback, false, callback_mode);
            }
            is_assignable_to(target_parameter, source_parameter)
                || (((bivariant_parameters && mode == CallbackMode::None)
                    || signature_parameter_carries_hole(target_parameter))
                    && is_assignable_to(source_parameter, target_parameter))
        },
    );

    // `compareSignaturesRelated`: a source's `this` other than `void` relates
    // to the target's either way round. A polymorphic `this` is whatever the
    // receiver is, which only a `void` on the other side is sure to reject.
    if let (Some(source_this), Some(target_this)) = (source.this_type(), target.this_type())
        && (!(source.has_polymorphic_this() || target.has_polymorphic_this()) || matches!(target_this, Type::Void))
        && !matches!(source_this, Type::Void)
        && !source_this.is_unmodelled()
        && !target_this.is_unmodelled()
        && !is_assignable_to(source_this, target_this)
        && !is_assignable_to(target_this, source_this)
    {
        return false;
    }

    parameters_compatible && signature_returns_related(source, target, mode)
}

/// A `void`-returning target ignores whatever the source returns: tsc accepts
/// any function as a `() => void` slot (`Array.prototype.forEach` callbacks,
/// event handlers, etc.). Outside that case the source return must be
/// assignable to the target's, or, between callbacks of a bivariant
/// parameter, either way.
/// Two predicates relate by `compareTypePredicateRelatedTo` alone. tsc also
/// rejects a source without a predicate against one with, but surge does
/// not attach the predicates tsc infers from a body
/// (`getTypePredicateFromBody`), so such a source still relates by its
/// return type.
fn signature_returns_related(source: &FunctionType, target: &FunctionType, mode: CallbackMode) -> bool {
    matches!(target.return_type(), Type::Void | Type::Any)
        || match (source.type_predicate(), target.type_predicate()) {
            (Some(source_predicate), Some(target_predicate)) => {
                type_predicates_related(source_predicate, target_predicate)
            }
            _ => {
                (mode == CallbackMode::Bivariant && is_assignable_to(target.return_type(), source.return_type()))
                    || is_assignable_to(source.return_type(), target.return_type())
            }
        }
}

/// relater.go `compareSignaturesRelated` beside a rest that is no array: only
/// the shorter list's positions are compared, its last one as the rest of each
/// list from there (`getRestOrAnyTypeAtPosition`). A target whose rest is a
/// union of tuples (`(...args: readonly [1, "1"] | readonly [2, "2"]) => …`)
/// or a type variable (`(x: number, ...args: T) => …`) against a source with
/// at least as many parameters relates that rest to what the source takes from
/// the rest's position on, as a tuple: `(a, b) => …` fits the mutable union and
/// not the readonly one, `(a) => …` neither, and `(a, b, ...x)` no `T`, which
/// may be empty. `None` outside that shape.
fn tuple_union_rest_related(
    source: &FunctionType,
    target: &FunctionType,
    bivariant_parameters: bool,
    mode: CallbackMode,
) -> Option<bool> {
    if !target.is_variadic() {
        return None;
    }
    let target_parameters = target.parameters();
    let (target_rest, target_fixed) = target_parameters.split_last()?;
    let tuple_like = |member: &Type| match member {
        Type::Reference(reference) if reference.is_readonly_array() => {
            matches!(reference.resolve(), Type::Tuple(_) | Type::OpenTuple(_))
        }
        other => matches!(other.peeled(), Type::Tuple(_) | Type::OpenTuple(_)),
    };
    let non_array_rest = match target_rest {
        Type::Union(union) => union.types().iter().all(tuple_like),
        Type::TypeParameter(_) => true,
        _ => false,
    };
    let source_parameters = source.parameters();
    if !non_array_rest || source_parameters.len() < target_parameters.len() {
        return None;
    }
    let rest_index = target_fixed.len();
    let source_types = with_parameter_optionality(
        std::borrow::Cow::Borrowed(source_parameters),
        source.required_parameter_count(),
        source.is_variadic(),
    );
    let target_types = with_parameter_optionality(
        std::borrow::Cow::Borrowed(target_parameters),
        target.required_parameter_count(),
        true,
    );
    if source_types.iter().chain(target_types.iter()).any(Type::is_unmodelled) {
        return None;
    }
    let related = |target_type: &Type, source_type: &Type| {
        is_assignable_to(target_type, source_type)
            || (bivariant_parameters && mode == CallbackMode::None && is_assignable_to(source_type, target_type))
    };
    let source_rest_is_written = |rest: &Type| match rest {
        Type::Union(members) => members.types().iter().all(tuple_like),
        Type::Reference(reference) if reference.is_readonly_array() => true,
        other => tuple_like(other) || matches!(rest_slot_shape(other), Type::Array(_)),
    };
    let source_rest = if source.is_variadic() {
        let (source_tail, source_fixed) = source_types.split_last()?;
        if !source_rest_is_written(source_tail) {
            return None;
        }
        if rest_index == source_fixed.len() {
            // `getRestOrAnyTypeAtPosition`: a rest of `any[]` reads as `any`.
            match rest_slot_shape(source_tail) {
                Type::Array(element) if matches!(*element, Type::Any) => Type::Any,
                _ => source_tail.clone(),
            }
        } else {
            let Type::Array(element) = rest_slot_shape(source_tail) else {
                return None;
            };
            Type::OpenTuple(crate::OpenTupleType {
                leading: source_fixed[rest_index..].to_vec(),
                rest: element,
                trailing: Vec::new(),
            })
        }
    } else {
        Type::Tuple(source_types[rest_index..].to_vec())
    };
    Some(
        (0..rest_index).all(|index| related(&target_types[index], &source_types[index]))
            && related(target_rest, &source_rest),
    )
}

/// tsc's `isWeakType`: an object whose members are all optional, with at
/// least one, and no call, construct or index signature.
pub fn is_weak_type(ty: &Type) -> bool {
    match ty.peeled() {
        // `object` is not weak, and an intersection is weak only when every
        // operand is: the merged surface answers for its inline members, the
        // recorded operands for the rest (`object & { call?: never }` is not).
        Type::Object(object) if object.non_primitive && !object.is_intersection => false,
        Type::Object(object) => {
            object
                .intersection_operands
                .as_deref()
                .filter(|_| object.is_intersection)
                .is_none_or(|operands| operands.iter().all(is_weak_type))
                && !object.properties.is_empty()
                && object.properties.values().all(|property| property.optional)
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature.is_none()
                && object.construct_signature.is_none()
                && !object.synthetic_open_index
        }
        _ => false,
    }
}

/// The source of a weak-type failure whose first call or construct
/// signature returns something the target accepts: tsc then asks "Did you
/// mean to call it?" (TS2560).
pub fn weak_type_source_returns_target(source: &Type, target: &Type) -> bool {
    let signature = match source.peeled() {
        Type::Function(function) => Some(function),
        Type::Object(object) => object
            .call_signature()
            .or_else(|| object.construct_signature())
            .cloned(),
        _ => None,
    };
    signature.is_some_and(|signature| is_assignable_to(signature.return_type(), target))
}

/// The weak-type check of relater.go `isRelatedTo`: a source with members of
/// its own (or signatures), none of which the weak target declares, relates
/// to nothing but itself. A source surge did not model whole is not judged.
pub fn has_no_common_properties(source: &Type, target: &Type) -> bool {
    if !is_weak_type(target) || is_global_object_type(source) {
        return false;
    }
    let Type::Object(target) = target.peeled() else {
        return false;
    };
    match source.peeled() {
        Type::Object(source) => {
            !source.synthetic_open_index
                && source.string_index_type.as_deref().is_none_or(|index| !index.is_unknown())
                && (!source.properties.is_empty()
                    || source.call_signature.is_some()
                    || source.construct_signature.is_some())
                && !source.properties.values().any(|property| property.ty.is_unknown())
                && !source.properties.keys().any(|name| target.properties.contains_key(name.as_ref()))
        }
        // A function type has signatures and no properties of its own.
        Type::Function(_) => true,
        // A primitive's properties are its apparent type's.
        primitive @ (Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)) => {
            !target.properties.keys().any(|name| primitive_member_type(&primitive, name).is_some())
        }
        // An array's properties are `Array<T>`'s, and a tuple's add its element
        // indexes. surge does not read them off the lib, so a name any lib
        // version declares on `Array`, or any index, counts as shared.
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => !target.properties.keys().any(|name| {
            crate::array_property_names().contains(&name.as_ref())
                || crate::MUTATING_ARRAY_MEMBERS.contains(&name.as_ref())
                || name.parse::<usize>().is_ok()
        }),
        _ => false,
    }
}

/// tsc's `globalObjectType`, which `isRelatedTo` exempts from the
/// common-property check: the lib's `Object` interface.
fn is_global_object_type(ty: &Type) -> bool {
    const OBJECT_MEMBERS: [&str; 7] = [
        "constructor",
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ];
    let named_object = match ty {
        Type::Reference(reference) => reference.id.split('\u{0}').next_back() == Some("Object"),
        Type::Object(object) => object.alias_name.as_deref() == Some("Object"),
        _ => false,
    };
    matches!(ty.peeled(), Type::Object(object)
        if OBJECT_MEMBERS.iter().all(|member| object.properties.contains_key(*member))
            && (named_object || object.properties.len() == OBJECT_MEMBERS.len()))
}

fn object_assignable(from_obj: &ObjectType, to_obj: &ObjectType, from: &Type, to: &Type) -> bool {
    let key = (
        Arc::as_ptr(&from_obj.properties) as usize,
        Arc::as_ptr(&to_obj.properties) as usize,
    );
    let newly_inserted = OBJECT_ASSIGNABILITY_IN_PROGRESS.with(|set| set.borrow_mut().insert(key));
    if !newly_inserted {
        record_assignability_assumption();
        return true;
    }

    let result = object_assignability_failure(from, to).is_none()
        && object_signatures_related(from_obj, to_obj)
        && index_signatures_related(from_obj, to_obj);
    OBJECT_ASSIGNABILITY_IN_PROGRESS.with(|set| {
        set.borrow_mut().remove(&key);
    });
    result
}

/// A tuple element's `ElementFlags`, as far as surge's shapes carry them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ElementKind {
    Required,
    Optional,
    Rest,
}

fn fixed_tuple_elements(elements: &[Type]) -> Vec<(ElementKind, &Type)> {
    fixed_tuple_kinds(elements, crate::tuple_min_length(elements))
}

fn fixed_tuple_kinds(elements: &[Type], min_length: usize) -> Vec<(ElementKind, &Type)> {
    elements
        .iter()
        .enumerate()
        .map(|(index, element)| {
            let kind = if index < min_length { ElementKind::Required } else { ElementKind::Optional };
            (kind, element)
        })
        .collect()
}

/// A mutable tuple source against the tuple `to` names — through an alias,
/// and a readonly target takes it as it is — or `None` when `to` is no tuple.
fn tuple_source_related(source: &[(ElementKind, &Type)], to: &Type) -> Option<bool> {
    match to {
        Type::OpenTuple(target) => Some(tuple_related_to_open_tuple(source, target)),
        Type::Reference(reference) if reference.written_tuple().is_none() => {
            tuple_source_related(source, &reference.resolve_arc())
        }
        other => {
            let (elements, min_length) = crate::fixed_tuple_parts(other)?;
            Some(tuple_related_to_fixed_tuple(source, &fixed_tuple_kinds(elements, min_length)))
        }
    }
}

/// relater.go `propertiesRelatedTo` for a tuple source against a tuple target
/// without a rest element: the source must reach the target's `minLength` and
/// the target must hold the source's longest length, and each element is
/// related to its counterpart, which must not be required where the source's
/// may be missing.
fn tuple_related_to_fixed_tuple(source: &[(ElementKind, &Type)], target: &[(ElementKind, &Type)]) -> bool {
    let min_length = |elements: &[(ElementKind, &Type)]| {
        elements
            .iter()
            .filter(|(kind, _)| *kind == ElementKind::Required)
            .count()
    };
    let source_rest = source.iter().any(|(kind, _)| *kind == ElementKind::Rest);
    if !source_rest && source.len() < min_length(target)
        || target.len() < min_length(source)
        || source_rest
        || target.len() < source.len()
    {
        return false;
    }
    source
        .iter()
        .zip(target)
        .all(|((source_kind, source_type), (target_kind, target_type))| {
            (*target_kind != ElementKind::Required || *source_kind == ElementKind::Required)
                && is_assignable_to(source_type, target_type)
        })
}

fn open_tuple_elements(tuple: &crate::OpenTupleType) -> Vec<(ElementKind, &Type)> {
    let mut elements = fixed_tuple_elements(&tuple.leading);
    elements.push((ElementKind::Rest, tuple.rest.as_ref()));
    elements.extend(tuple.trailing.iter().map(|element| (ElementKind::Required, element)));
    elements
}

/// relater.go `propertiesRelatedTo` for a tuple source against a tuple target
/// with a rest element: a fixed source must reach the target's `minLength`,
/// and each source position is related to the target position it lands on —
/// counted from the start within the target's leading elements, from the end
/// past them — which must not be a required element the source may lack.
fn tuple_related_to_open_tuple(source: &[(ElementKind, &Type)], target: &crate::OpenTupleType) -> bool {
    let target_elements = open_tuple_elements(target);
    let target_arity = target_elements.len();
    let target_min_length = target_elements
        .iter()
        .filter(|(kind, _)| *kind == ElementKind::Required)
        .count();
    let source_rest = source.iter().any(|(kind, _)| *kind == ElementKind::Rest);
    if !source_rest && source.len() < target_min_length {
        return false;
    }
    let source_arity = source.len();
    source
        .iter()
        .enumerate()
        .all(|(source_position, (source_kind, source_type))| {
            let from_end = source_arity - 1 - source_position;
            let target_position = if source_position >= target.leading.len() {
                target_arity - 1 - from_end.min(target.trailing.len())
            } else {
                source_position
            };
            let (target_kind, target_type) = target_elements[target_position];
            (target_kind != ElementKind::Required || *source_kind == ElementKind::Required)
                && is_assignable_to(source_type, target_type)
        })
}

/// An object source against an array or fixed-tuple target, related as
/// against the object type the target's apparent type is (see
/// [`array_like_apparent_object`]). Most objects lack the array surface, so
/// the member names are checked before any member type is built.
fn object_related_to_array_like(
    source: &ObjectType,
    from: &Type,
    element: &Type,
    tuple: Option<(&[Type], usize)>,
    readonly: bool,
) -> bool {
    let has_array_members = crate::array_property_names()
        .iter()
        .filter(|name| !readonly || !crate::MUTATING_ARRAY_MEMBERS.contains(name))
        .all(|name| supplies_required_member(source, name));
    let has_elements = tuple.is_none_or(|(_, min_length)| {
        (0..min_length).all(|index| supplies_required_member(source, &index.to_string()))
    });
    if !has_array_members || !has_elements {
        return false;
    }
    let target = Type::Object(array_like_apparent_object(element, tuple, readonly));
    let Type::Object(target_object) = &target else {
        return false;
    };
    let related = object_assignable(source, target_object, from, &target);
    SYNTHESIZED_TARGETS.with(|targets| targets.borrow_mut().push(target));
    related
}

/// The members of an array's or a fixed tuple's apparent type: `Array<T>`'s
/// over the element type (`ReadonlyArray<T>`'s for a readonly one, which lacks
/// the mutators), and for a tuple the element properties and the `length`
/// `createTupleTargetType` declares, beside the number index signature.
fn array_like_apparent_object(element: &Type, tuple: Option<(&[Type], usize)>, readonly: bool) -> ObjectType {
    let mut properties = crate::PropertyMap::default();
    for name in crate::array_property_names() {
        if readonly && crate::MUTATING_ARRAY_MEMBERS.contains(name) {
            continue;
        }
        let property = if *name == "length" {
            let length = match tuple {
                Some((elements, min_length)) => crate::ty::tuple_length_literals(min_length, elements.len()),
                None => Type::Number,
            };
            crate::ObjectProperty::required(length)
        } else {
            let Some(member) = crate::array_member_type(name, element) else {
                continue;
            };
            crate::ObjectProperty::required(member).with_method(true)
        };
        properties.insert((*name).into(), property);
    }
    if let Some((elements, min_length)) = tuple {
        for (index, element) in elements.iter().enumerate() {
            let property = if index < min_length {
                crate::ObjectProperty::required(element.clone())
            } else {
                crate::ObjectProperty::optional(element.clone())
            };
            properties.insert(index.to_string().into(), property);
        }
    }
    // Built directly rather than through `ObjectType::new`: this shape lives
    // for one comparison and has no business in the canonical property-map
    // store.
    ObjectType {
        properties: Arc::new(properties),
        property_map_id: None,
        string_index_type: None,
        number_index_type: Some(Arc::new(element.clone())),
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
    }
}

/// Whether `source` answers a required target member `name` the way
/// [`object_assignability_failure`] looks it up: a property, an index
/// signature standing for members surge could not enumerate, the `Function`
/// surface of a callable object, or the global `Object` members.
fn supplies_required_member(source: &ObjectType, name: &str) -> bool {
    source.properties.contains_key(name)
        || source
            .applicable_index_type(crate::object::is_numeric_key(name))
            .is_some_and(|index| {
                // An index signature never supplies a required member; only one
                // surge could not model (the sentinel) is let through. A type
                // variable is a modelled value type.
                source.synthetic_open_index || index.is_unmodelled()
            })
        || callable_object_function_member(source, name).is_some()
        || object_prototype_member(name).is_some()
}

/// relater.go `reportUnmatchedProperty` for an object source against a fixed
/// tuple target: the one member the source lacks, which TS2741 names. With
/// more than one missing, `tryElaborateArrayLikeErrors` declines to list them
/// for a source that is not an array, so the plain assignability head stands.
pub fn tuple_target_missing_property(source: &Type, elements: &[Type]) -> Option<String> {
    let Type::Object(source) = source else {
        return None;
    };
    // `shouldReportUnmatchedPropertyError`: a source that is only a signature
    // is not reported by its members.
    if source.properties.is_empty()
        && (source.call_signature().is_some() || source.construct_signature().is_some())
    {
        return None;
    }
    let target = array_like_apparent_object(
        &crate::tuple_element_union(elements),
        Some((elements, crate::tuple_min_length(elements))),
        false,
    );
    let mut missing = target
        .required_properties()
        .filter(|(name, _)| !supplies_required_member(source, name))
        .map(|(name, _)| name.to_string());
    let first = missing.next()?;
    missing.next().is_none().then_some(first)
}

/// tsc's `indexSignaturesRelatedTo`. A target index signature is satisfied by
/// the source's applicable one, or — the source having none — by every source
/// member it would cover (the implicit index signature of an object type).
/// An `any`-valued signature asks for nothing once the target has a string
/// index at all. Only an object or type literal has that implicit signature
/// (`isObjectTypeWithInferableIndex`): an interface, a class instance or a
/// callable object answers with a signature it declares, or not at all.
/// A type variable of the body being checked is a type here like anywhere
/// else; only what surge could not model relates unconditionally.
fn index_signatures_related(source: &ObjectType, target: &ObjectType) -> bool {
    if target.synthetic_open_index || source.synthetic_open_index {
        return true;
    }
    // `typeRelatedToEachType`: an intersection target is related member by
    // member, so one member's `[x: string]: any` does not excuse another
    // member's number index signature.
    if target.is_intersection
        && (target.string_index_type.is_some() || target.number_index_type.is_some())
        && let Some(operands) = target.intersection_operands.as_deref()
    {
        let members: Vec<ObjectType> = operands
            .iter()
            .filter_map(|operand| match operand.peeled() {
                Type::Object(object) => Some(object),
                _ => None,
            })
            .collect();
        if members.len() == operands.len() && members.len() > 1 {
            return members.iter().all(|member| index_signatures_related(source, member));
        }
    }
    let target_has_string_index = target.string_index_type.is_some();
    let exempt = |value: &Type| target_has_string_index && matches!(value, Type::Any);
    let related = |value: &Type, numeric_only: bool| {
        // A written `unknown` is a type like any other: an interface still
        // lacks the index signature it would satisfy.
        if exempt(value) || (value.is_unmodelled() && !matches!(value, Type::GenuineUnknown)) {
            return true;
        }
        if let Some(source_index) = source.applicable_index_type(numeric_only) {
            return source_index.is_unmodelled() || is_assignable_to(source_index, value);
        }
        if source.without_inferable_index
            || source.call_signature().is_some()
            || source.construct_signature().is_some()
        {
            return false;
        }
        // With no signature of the target's own kind, a numeric one still
        // covers keys a string signature answers (`membersRelatedToIndexer`).
        if !numeric_only
            && let Some(number_index) = source.number_index_type.as_deref()
            && !number_index.is_unmodelled()
            && !is_assignable_to(number_index, value)
        {
            return false;
        }
        // A computed member's key is a symbol, which no string or number index
        // signature applies to (`membersRelatedToIndexInfo`,
        // `isApplicableIndexType`).
        source
            .properties
            .iter()
            .filter(|(name, _)| {
                !name.starts_with('[')
                    && (!numeric_only || crate::object::is_numeric_key(name.as_ref()))
            })
            .all(|(_, property)| {
                property.ty.is_unmodelled()
                    || is_assignable_to(&indexed_member_type(property, numeric_only), value)
            })
    };
    let string_related = match target.active_keyed_indexes() {
        Some(keyed) => keyed
            .infos
            .iter()
            .all(|(key, value)| keyed_index_info_related(source, key, value)),
        None => target
            .string_index_type
            .as_deref()
            .is_none_or(|value| related(value, false)),
    };
    // A `symbol` index signature covers the source's symbol-keyed members.
    let symbol_related = |value: &Type| {
        if value.is_unmodelled() {
            return true;
        }
        if let Some(source_index) = source.symbol_index_type() {
            return source_index.is_unmodelled() || is_assignable_to(source_index, value);
        }
        if source.without_inferable_index
            || source.call_signature().is_some()
            || source.construct_signature().is_some()
        {
            return false;
        }
        source
            .properties
            .iter()
            .filter(|(name, _)| name.starts_with('['))
            .all(|(_, property)| {
                property.ty.is_unmodelled() || is_assignable_to(&indexed_member_type(property, false), value)
            })
    };
    string_related
        && target
            .number_index_type
            .as_deref()
            .is_none_or(|value| related(value, true))
        && target.symbol_index_type().is_none_or(symbol_related)
}

/// tsc's `typeRelatedToIndexInfo` for a target info keyed by a pattern or a
/// branded string: the source's applicable info answers it, or — the source
/// having none — every member and info whose key the target key covers
/// (`membersRelatedToIndexer`).
fn keyed_index_info_related(source: &ObjectType, key: &Type, value: &Type) -> bool {
    if value.is_unmodelled() {
        return true;
    }
    let source_keyed = source.active_keyed_indexes();
    let applicable = match source_keyed {
        Some(source_keyed) => source_keyed
            .infos
            .iter()
            .find(|(source_key, _)| is_assignable_to(key, source_key))
            .map(|(_, source_value)| source_value),
        None => source.string_index_type.as_deref(),
    };
    if let Some(source_value) = applicable {
        return source_value.is_unmodelled() || is_assignable_to(source_value, value);
    }
    if source.without_inferable_index
        || source.call_signature().is_some()
        || source.construct_signature().is_some()
    {
        return false;
    }
    let infos_related = source_keyed.is_none_or(|source_keyed| {
        source_keyed
            .infos
            .iter()
            .filter(|(source_key, _)| is_assignable_to(source_key, key))
            .all(|(_, source_value)| is_assignable_to(source_value, value))
    });
    infos_related
        && source
            .properties
            .iter()
            .filter(|(name, _)| {
                !name.starts_with('[') && is_assignable_to(&Type::StringLiteral(name.to_string()), key)
            })
            .all(|(_, property)| {
                property.ty.is_unmodelled()
                    || is_assignable_to(&indexed_member_type(property, false), value)
            })
}

/// The type `membersRelatedToIndexInfo` relates a member as: an optional one
/// keeps the `undefined` its read adds against a number index signature and
/// loses every `undefined` against a string one, unless it is only
/// `undefined`; under `exactOptionalPropertyTypes` it is its declared type.
fn indexed_member_type(property: &crate::ObjectProperty, numeric_index: bool) -> Type {
    if !property.is_optional() || !crate::strict_null_checks() || crate::exact_optional_property_types() {
        return property.ty.clone();
    }
    if numeric_index {
        with_optionality(&property.ty, true)
    } else {
        strip_undefined_member(&property.ty).unwrap_or(Type::Undefined)
    }
}

/// tsc's `signaturesRelatedTo`, for both kinds: every call (construct)
/// signature the target declares has to be matched by one of the source's. A
/// target with none asks for nothing; a source with none matches nothing.
fn object_signatures_related(source: &ObjectType, target: &ObjectType) -> bool {
    signatures_of_kind_related(source.call_signature(), target.call_signature())
        && construct_signature_modifiers_related(source.construct_signature(), target.construct_signature())
        && signatures_of_kind_related(source.construct_signature(), target.construct_signature())
}

/// relater.go `signaturesRelatedTo` for construct signatures: an abstract
/// constructor type is not assignable to a non-abstract one, and
/// `constructorVisibilitiesAreCompatible`.
fn construct_signature_modifiers_related(source: Option<&FunctionType>, target: Option<&FunctionType>) -> bool {
    use crate::ConstructorAccessibility::{Private, Protected, Public, Undeclared};
    let (Some(source), Some(target)) = (source, target) else {
        return true;
    };
    let (source, target) = (source.construct_modifiers(), target.construct_modifiers());
    if source.is_abstract() && !target.is_abstract() {
        return false;
    }
    match (source.accessibility(), target.accessibility()) {
        (Undeclared, _) | (_, Undeclared) | (_, Private) => true,
        (source, Protected) => source != Private,
        (source, Public) => source == Public,
    }
}

fn signatures_of_kind_related(source: Option<&FunctionType>, target: Option<&FunctionType>) -> bool {
    let Some(target) = target else {
        return true;
    };
    let Some(source) = source else {
        return false;
    };
    let overloads = |signature: &FunctionType| -> Vec<FunctionType> {
        match signature.overloads() {
            Some(members) if !members.is_empty() => members.to_vec(),
            _ => vec![signature.clone()],
        }
    };
    let sources = overloads(source);
    let targets = overloads(target);
    // A member surge could not type says nothing about the group. A single
    // pair relates through written shapes, which hold the constrained type
    // parameters their signatures erased.
    let single_pair = sources.len() == 1 && targets.len() == 1;
    let unmodelled = |signature: &FunctionType| {
        let (parameters, return_type) = match signature.generic_shape().filter(|_| single_pair) {
            Some(shape) => (shape.parameters.as_slice(), &shape.return_type),
            None => (signature.parameters(), signature.return_type()),
        };
        parameters.iter().any(|parameter| matches!(parameter, Type::Unknown))
            || matches!(return_type, Type::Unknown)
    };
    if sources.iter().any(unmodelled) || targets.iter().any(unmodelled) {
        return true;
    }
    // relater.go `signaturesRelatedTo`: a single pair instantiates a generic
    // source in the target's context; with an overload group on either side
    // every signature's type parameters are erased, and each target member
    // needs some source member.
    if let ([source_signature], [target_signature]) = (sources.as_slice(), targets.as_slice()) {
        return is_function_assignable_to(source_signature, target_signature);
    }
    targets.iter().all(|target_signature| {
        sources
            .iter()
            .any(|source_signature| is_signature_assignable_to(source_signature, target_signature, false))
    })
}

/// Function/constructor objects (those carrying a call or construct signature)
/// also expose `Function.prototype` members. When such a source object lacks an
/// explicit property, fall back to these so a `typeof SomeClass` value satisfies
/// targets like `{name: string}`. Mirrors `function_property_access_type` in
/// `ty.rs`, but keyed off the object's call signature for `call`/`apply`/`bind`.
/// tsc's `getPropertyOfType` falls back to the global `Object` type's members
/// for every object type, so `{}` has `toString` and is assignable to
/// `Object`. The members' lib signatures, with `valueOf`'s `Object` as the
/// empty object type (no primitive, and it has these same members) and
/// `constructor`'s `Function` left as `any`.
fn object_prototype_member(name: &str) -> Option<Type> {
    let method = |parameters: Vec<Type>, return_type: Type| {
        let required = parameters.len();
        Some(Type::Function(FunctionType::new(parameters, return_type, false, required)))
    };
    match name {
        "toString" | "toLocaleString" => method(vec![], Type::String),
        "valueOf" => method(vec![], Type::Object(ObjectType::new(Default::default(), None))),
        "hasOwnProperty" | "propertyIsEnumerable" | "isPrototypeOf" => {
            method(vec![Type::Any], Type::Boolean)
        }
        "constructor" => Some(Type::Any),
        _ => None,
    }
}

fn callable_object_function_member(source: &ObjectType, name: &str) -> Option<Type> {
    let signature = source
        .call_signature()
        .or_else(|| source.construct_signature())?;
    match name {
        "length" => Some(Type::Number),
        "name" => Some(Type::String),
        "toString" | "toLocaleString" => Some(Type::Function(FunctionType::new(
            vec![],
            Type::String,
            false,
            0,
        ))),
        "call" | "apply" => Some(Type::Function(FunctionType::new(
            vec![],
            signature.return_type().clone(),
            true,
            0,
        ))),
        "bind" => Some(Type::Function(FunctionType::new(
            vec![],
            Type::Function(signature.clone()),
            true,
            0,
        ))),
        _ => function_interface_member(name),
    }
}

/// The global `Function` members a function value's surface above does not
/// model, as the relation needs them to satisfy a target that extends
/// `Function`: lib.es5.d.ts's `prototype` and `arguments` (`any`) and `caller`
/// (the global `Function`, which this crate cannot name), and
/// lib.es2015.symbol.wellknown.d.ts's `[Symbol.hasInstance]`.
fn function_interface_member(name: &str) -> Option<Type> {
    match name {
        "prototype" | "arguments" | "caller" | "[Symbol.metadata]" => Some(Type::Any),
        "[Symbol.hasInstance]" => Some(Type::Function(FunctionType::new(vec![Type::Any], Type::Boolean, false, 1))),
        _ => None,
    }
}

/// Drops the `undefined` member a value may carry when the target property is
/// optional. `None` means the value was *only* `undefined`, which such a target
/// always accepts.
fn strip_undefined_member(ty: &Type) -> Option<Type> {
    match ty {
        Type::Undefined => None,
        Type::Union(union)
            if union
                .types()
                .iter()
                .any(|member| *member == Type::Undefined) =>
        {
            let kept: Vec<Type> = union
                .types()
                .iter()
                .filter(|member| **member != Type::Undefined)
                .cloned()
                .collect();
            if kept.is_empty() {
                None
            } else {
                Some(crate::union_type(kept))
            }
        }
        _ => Some(ty.clone()),
    }
}

/// relater.go `propertyRelatedTo` under the comparable relation. Both members
/// are read with their optionality (`getNonMissingTypeOfSymbol`), so two
/// optional members always overlap in `undefined`, and an optional source is
/// not held to a required target (`skipOptional`). A method target still
/// compares its parameters bivariantly.
fn comparable_property_related(source_ty: &Type, source_optional: bool, target: &crate::ObjectProperty) -> bool {
    let effective_source = with_optionality(source_ty, source_optional);
    let effective_target = with_optionality(&target.ty, target.is_optional());
    if target.is_method()
        && let (Type::Function(source_signature), Type::Function(target_signature)) = (source_ty, &target.ty)
    {
        return type_includes_undefined(&effective_source) && type_includes_undefined(&effective_target)
            || is_signature_assignable_to(source_signature, target_signature, true);
    }
    is_assignable_to(&effective_source, &effective_target)
}

/// An optional member's declared type as `getNonMissingTypeOfSymbol` reads it:
/// with `undefined` under `strictNullChecks`, as declared under
/// `exactOptionalPropertyTypes`.
fn with_optionality(ty: &Type, optional: bool) -> Type {
    if optional
        && crate::strict_null_checks()
        && !crate::exact_optional_property_types()
        && !type_includes_undefined(ty)
    {
        crate::union_type(vec![ty.clone(), Type::Undefined])
    } else {
        ty.clone()
    }
}

/// tsc's `propertyRelatedTo` modifier rules. A private member on either side
/// relates only to the same declaration. A protected target needs a protected
/// source; tsc also requires the source's class to derive from the target's,
/// which the member alone cannot tell, so any protected source is accepted.
/// A protected source never relates to a public target.
fn restrictions_relate(
    source: Option<&crate::MemberRestriction>,
    target: Option<&crate::MemberRestriction>,
) -> bool {
    match (source, target) {
        (None, None) => true,
        (Some(source), Some(target)) if source.private || target.private => source == target,
        (Some(_), Some(_)) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

pub fn object_assignability_failure(
    source: &Type,
    target: &Type,
) -> Option<ObjectAssignabilityFailure> {
    let (Type::Object(source), Type::Object(target)) = (source, target) else {
        return None;
    };
    let comparable = current_relation() == Relation::Comparable;

    for (property_name, target_property) in target.properties.iter() {
        let source_property = source.properties.get(property_name.as_ref());
        // A declared index signature supplies no target property (tsc's
        // `propertiesRelatedTo` reads `getPropertyOfType` only): `{ [k: string]:
        // any }` is missing `hello` from `{ hello: string }`, and an optional
        // target property the source does not declare has nothing to relate.
        // Checker-injected openness still answers, since it stands for members
        // surge could not enumerate — and so does an `any`-valued signature,
        // which is also how surge spells an object it could not model (the
        // stand-in a generic body's own type parameter is evaluated with).
        let source_property_ty = source_property.map(|property| &property.ty).or_else(|| {
            let index = source
                .applicable_index_type(crate::object::is_numeric_key(property_name.as_ref()))?;
            (source.synthetic_open_index || index.is_unmodelled())
                .then_some(index)
        });

        let source_property_ty = source_property_ty
            .cloned()
            .or_else(|| callable_object_function_member(source, property_name.as_ref()))
            .or_else(|| object_prototype_member(property_name.as_ref()));

        let Some(source_property_ty) = source_property_ty.as_ref() else {
            // `getUnmatchedProperties` passes over a static private name the
            // source lacks.
            if target_property.is_optional() || crate::private_name::is_static(property_name) {
                continue;
            }

            return Some(ObjectAssignabilityFailure::MissingProperty {
                property_name: property_name.to_string(),
            });
        };

        if let Some(source_property) = source_property
            && !restrictions_relate(
                source_property.restriction.as_ref(),
                target_property.restriction.as_ref(),
            )
        {
            return Some(ObjectAssignabilityFailure::AccessibilityMismatch {
                property_name: property_name.to_string(),
            });
        }

        if comparable {
            let source_optional = source_property.is_some_and(|property| property.is_optional());
            if !comparable_property_related(source_property_ty, source_optional, target_property) {
                return Some(ObjectAssignabilityFailure::PropertyTypeMismatch {
                    property_name: property_name.to_string(),
                    source_type: source_property_ty.clone(),
                    target_type: target_property.ty.clone(),
                });
            }
            continue;
        }

        if source_property.is_some()
            && source_property.is_some_and(|p| p.is_optional())
            && target_property.is_required()
        {
            return Some(ObjectAssignabilityFailure::MissingProperty {
                property_name: property_name.to_string(),
            });
        }

        // Without `exactOptionalPropertyTypes` an optional target property accepts
        // an explicit `undefined`, so a required source property read as
        // `T | undefined` (typically itself an optional property's read type)
        // satisfies a `p?: T` target. With it the target is related as declared
        // (`getNonMissingTypeOfSymbol`).
        let stripped_source_ty;
        let comparable_source_ty = if target_property.is_optional() && !crate::exact_optional_property_types() {
            match strip_undefined_member(source_property_ty) {
                Some(stripped) => {
                    stripped_source_ty = stripped;
                    &stripped_source_ty
                }
                None => continue,
            }
        } else {
            source_property_ty
        };

        let property_assignable = match (
            target_property.is_method(),
            comparable_source_ty,
            &target_property.ty,
        ) {
            (true, Type::Function(source_signature), Type::Function(target_signature)) => {
                is_signature_assignable_to(source_signature, target_signature, true)
            }
            _ => is_assignable_to(comparable_source_ty, &target_property.ty),
        };

        if !property_assignable {
            return Some(ObjectAssignabilityFailure::PropertyTypeMismatch {
                property_name: property_name.to_string(),
                source_type: source_property_ty.clone(),
                target_type: target_property.ty.clone(),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests;

/// The operands of a deferred intersection (`A & B` kept as a reference whose
/// arguments are its operands) or a merged one.
fn pattern_intersection_operands(ty: &Type) -> Option<&[Type]> {
    let operands = match ty {
        Type::Reference(reference) if reference.id.starts_with("\u{0}intersection") => &reference.arguments[..],
        Type::Object(object) if object.is_intersection => object.intersection_operands.as_deref()?,
        _ => return None,
    };
    (operands.len() > 1).then_some(operands)
}
