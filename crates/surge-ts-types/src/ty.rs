use crate::{FunctionType, ObjectType, TypeReference, UnionType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberLiteralType {
    pub value: String,
}

/// The payload of [`Type::TypeParameter`]: the parameter's declared name.
///
/// Deliberately only the name. A substitution binds a declaration's **own**
/// parameter names, so two references to `QueryBehavior<TQueryFnData, …>` from
/// different consumers produce the same argument tuple — which is why the whole
/// project has 2,343 distinct tuples rather than one per reference site. Adding
/// a per-declaration identity here would make those tuples stop colliding and
/// the cache hit *less*. Any consumer that keys on this must therefore establish
/// that two declarations' same-named parameters cannot meet inside one key.
///
/// `owner` is `0` for such a placeholder. A nonzero `owner` makes it a type
/// variable of the generic body being checked (see [`crate::type_variable`]);
/// those never reach an instantiation key, because a body's resolutions are
/// context-dependent and not interned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeParameterType {
    pub name: std::sync::Arc<str>,
    pub owner: u32,
}

/// A tuple with a rest slot: `[1, ...number[]]`, `[...string[], 'end']`,
/// `[a, ...b[], c]`. `leading` and `trailing` are the fixed slots on either
/// side of the rest, `rest` the rest slot's *element* type. Kept distinct from
/// [`Type::Tuple`] because its length is unknown, and from [`Type::Array`]
/// because it is not: `number[]` does not satisfy `[number, ...number[]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTupleType {
    pub leading: Vec<Type>,
    pub rest: Box<Type>,
    pub trailing: Vec<Type>,
    /// One label per position — `leading`, the rest, then `trailing` — when
    /// the tuple was written named (`[head: A, ...tail: B[]]`).
    pub labels: Option<std::sync::Arc<[crate::TupleLabel]>>,
}

impl OpenTupleType {
    /// The element a literal `index` reads when it falls in the fixed leading
    /// run (`[A, ...B[]]` at `0` is exactly `A`, with no `undefined`).
    pub fn leading_element(&self, index: &str) -> Option<&Type> {
        self.leading.get(index.parse::<usize>().ok()?)
    }

    /// Every element type the tuple can hold, as one union.
    pub fn element_union(&self) -> Type {
        let mut members = self.leading.clone();
        members.push(self.rest.as_ref().clone());
        members.extend(self.trailing.iter().cloned());
        crate::union_type(members)
    }

    pub fn fixed_len(&self) -> usize {
        self.leading.len() + self.trailing.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    String,
    Number,
    Boolean,
    BigInt,
    Symbol,
    Undefined,
    Null,
    Void,
    Any,
    Unknown,
    /// The genuine `unknown` type as written with the `unknown` keyword (or
    /// flowing from a declaration that uses it). Behaves identically to
    /// [`Type::Unknown`] in every type operation — assignability, display,
    /// narrowing — and is matched together with it via [`Type::is_unknown`].
    /// The sole distinction is provenance: `Unknown` is also surge's
    /// graceful-degradation sentinel for types it cannot resolve, whereas
    /// `GenuineUnknown` is only produced by an actual `unknown` annotation. The
    /// checker uses that distinction to emit `TS18046` ('x' is of type
    /// 'unknown') on a genuine-unknown receiver while staying silent on a
    /// degraded one, matching tsc's no-cascade behavior.
    GenuineUnknown,
    /// A type tsc itself has no answer for — an unresolved type name or a type
    /// imported from a module that does not resolve. tsc models this as
    /// `errorType`, which is `any` with its own identity
    /// (`newIntrinsicType(TypeFlagsAny, "error")`), so it stays permissive
    /// everywhere and is only consulted where the *rule* differs.
    ///
    /// Behaves identically to [`Type::Unknown`] in every type operation and is
    /// matched with it via [`Type::is_unknown`]; the distinction is provenance,
    /// exactly as [`Type::GenuineUnknown`] carries it for the written keyword.
    /// `Unknown` means "surge could not model this" and must stay silent to
    /// avoid cascading its own gaps; `ErrorType` means "the source is genuinely
    /// broken", which is where tsc keeps reporting — a callback parameter
    /// contextually typed by it is still an implicit `any` (TS7006).
    ErrorType,
    /// An unsubstituted type parameter standing in for itself, as bound by a
    /// signature or declaration pre-pass (`substitution.insert_placeholder`).
    ///
    /// Behaves identically to [`Type::Unknown`] in every type operation —
    /// assignability, display, narrowing, interning — and is matched together
    /// with it via [`Type::is_unknown`]. Like [`Type::GenuineUnknown`] the sole
    /// distinction is provenance, and it exists for one reason: `Type::Unknown`
    /// doubles as surge's graceful-degradation sentinel, so a cache that sees it
    /// as a type argument cannot tell "this resolution failed" (which the
    /// memory-lifetime rules forbid caching) from "this argument is the
    /// declaration's own parameter" (which is perfectly cacheable). Carrying the
    /// distinction in the type is what lets the interface instantiation key
    /// admit a placeholder argument instead of refusing it.
    TypeParameter(TypeParameterType),
    Never,
    StringLiteral(String),
    NumberLiteral(NumberLiteralType),
    BooleanLiteral(bool),
    Function(FunctionType),
    Object(ObjectType),
    Array(Box<Type>),
    Tuple(crate::TupleType),
    OpenTuple(OpenTupleType),
    Union(UnionType),
    /// A lazy, nominal reference to a named type instantiation (`Box<string>`,
    /// `User`, …). The structural shape is resolved on demand and memoized,
    /// rather than eagerly expanded at every use site. See [`TypeReference`].
    Reference(TypeReference),
}

fn function_type(
    parameters: Vec<Type>,
    return_type: Type,
    is_variadic: bool,
    required_parameter_count: usize,
) -> Type {
    Type::Function(FunctionType::new(
        parameters,
        return_type,
        is_variadic,
        required_parameter_count,
    ))
}

/// A reference resolver that yields a fixed pre-built structural type.
#[derive(Debug)]
struct FixedResolved(Type);

impl crate::ResolveReference for FixedResolved {
    fn resolve(&self) -> Type {
        self.0.clone()
    }
}

/// Builds an `ArrayIterator<T>` reference for the result of `Array.prototype`'s
/// `values()`/`keys()`/`entries()`. It carries the yielded element as its single
/// type argument (so `for…of` derives the element via the iterator-reference
/// path) and resolves structurally to a minimal iterator object exposing
/// `next(): { value, done }`, so direct iterator-protocol use does not report a
/// missing member. `Array`-valued modelling was rejected: it would falsely reject
/// `arr.values().next()`.
fn array_iterator_type(yields: Type) -> Type {
    use crate::{ObjectProperty, PropertyMap};
    let display = format!("ArrayIterator<{}>", yields.name());

    let mut result_props = PropertyMap::default();
    // The protocol's terminal result carries `value: undefined`, so the merged
    // `next()` result type is `T | undefined` — matching tsc and keeping the
    // possibly-absent value sound.
    result_props.insert(
        "value".into(),
        ObjectProperty::required(crate::union_type(vec![yields.clone(), Type::Undefined])),
    );
    result_props.insert("done".into(), ObjectProperty::required(Type::Boolean));
    let result = Type::Object(ObjectType::new(result_props, None));

    let mut props = PropertyMap::default();
    props.insert(
        "next".into(),
        ObjectProperty::required(function_type(vec![], result, false, 0)),
    );
    let body = Type::Object(ObjectType::new(props, None));

    Type::Reference(TypeReference::new(
        "\u{0}ArrayIterator",
        display,
        vec![yields],
        std::sync::Arc::new(FixedResolved(body)),
    ))
}

impl Type {
    /// Whether this is the `unknown` type, regardless of provenance — both the
    /// degradation sentinel [`Type::Unknown`] and the genuine
    /// [`Type::GenuineUnknown`]. Use this in place of `== Type::Unknown` at every
    /// site that cares about unknown-ness rather than provenance.
    /// The placeholder a pre-pass binds an unsubstituted type parameter to.
    /// See [`Type::TypeParameter`] for why this is not just [`Type::Unknown`].
    pub fn type_parameter(name: &str) -> Type {
        Type::TypeParameter(TypeParameterType {
            name: std::sync::Arc::from(name),
            owner: 0,
        })
    }

    /// Surge's own "could not model this": [`Type::is_unknown`] without tsc's
    /// error type, which is a real `any` and keeps flowing — into an inference
    /// candidate, a return type, a binding — where the sentinel must not.
    pub fn is_degraded(&self) -> bool {
        self.is_unknown() && !matches!(self, Type::ErrorType)
    }

    pub fn is_unknown(&self) -> bool {
        matches!(
            self,
            Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_)
        )
    }

    pub fn base_primitive(&self) -> Option<Type> {
        match self {
            Type::String | Type::StringLiteral(_) => Some(Type::String),
            Type::Number | Type::NumberLiteral(_) => Some(Type::Number),
            Type::Boolean | Type::BooleanLiteral(_) => Some(Type::Boolean),
            Type::BigInt => Some(Type::BigInt),
            Type::Symbol => Some(Type::Symbol),
            Type::Reference(reference) => reference.resolve().base_primitive(),
            Type::Object(object) => object.intersected_primitive()?.base_primitive(),
            // A union whose members share one base primitive behaves like that
            // primitive for operand checks. `number | 0` arises from `x ?? 0`
            // (TypeScript reduces it to `number`; surge keeps the literal), and
            // rejecting it made `(x ?? 0) > 0` a false `TS2365`.
            Type::Union(union) => {
                let mut base: Option<Type> = None;
                for member in union.types() {
                    // Every member must share the base — a member without one
                    // (`undefined`) makes the union something else entirely.
                    let member_base = member.base_primitive()?;
                    match &base {
                        Some(existing) if *existing != member_base => return None,
                        Some(_) => {}
                        None => base = Some(member_base),
                    }
                }
                base
            }
            _ => None,
        }
    }

    /// Returns the structural form of a [`Type::Reference`] (resolving lazily and
    /// recursively), or a clone of `self` otherwise. Use at sites that
    /// structurally inspect a type — e.g. matching `Type::Object` to read its
    /// properties or index signature — so a nominal reference stays transparent.
    pub fn peeled(&self) -> Type {
        match self {
            Type::Reference(reference) => reference.resolve().peeled(),
            other => other.clone(),
        }
    }

    /// Whether `name` resolves only through a string index signature rather than
    /// a declared property — the condition for TS4111 under
    /// `noPropertyAccessFromIndexSignature`. References are peeled; other type
    /// shapes (no index signature) answer `false`.
    ///
    /// A checker-injected openness index (`synthetic_open_index`) does not count:
    /// there the property really lives on an intersection operand surge could not
    /// enumerate, and tsc — which does model that operand — sees a declared
    /// property and reports nothing.
    pub fn property_only_from_string_index(&self, name: &str) -> bool {
        match self {
            Type::Object(object) => {
                object
                    .get_property(name)
                    .is_none_or(|property| property.index_slot)
                    && object.allows_string_index_access()
                    && !object.synthetic_open_index
            }
            Type::Reference(reference) => reference.resolve().property_only_from_string_index(name),
            // tsc looks the name up on the union's synthetic property set and
            // then for an index signature every member carries; an `a?.b`
            // receiver still carries the chain's `undefined`, which is not a
            // member of the lookup.
            Type::Union(union) => {
                let members: Vec<&Type> = union
                    .types()
                    .iter()
                    .filter(|member| !matches!(member, Type::Undefined | Type::Void))
                    .collect();
                !members.is_empty()
                    && members
                        .iter()
                        .all(|member| member.property_only_from_string_index(name))
            }
            _ => false,
        }
    }

    /// [`Self::property_only_from_string_index`] where no guard has narrowed
    /// the slot yet: the read `noUncheckedIndexedAccess` still widens with
    /// `undefined`. A narrowed slot already holds what its guard proved.
    pub fn reads_unnarrowed_string_index(&self, name: &str) -> bool {
        match self {
            Type::Object(object) => {
                object.get_property(name).is_none() && self.property_only_from_string_index(name)
            }
            Type::Reference(reference) => reference.resolve().reads_unnarrowed_string_index(name),
            Type::Union(union) => {
                self.property_only_from_string_index(name)
                    && union
                        .types()
                        .iter()
                        .filter(|member| !matches!(member, Type::Undefined | Type::Void))
                        .any(|member| member.reads_unnarrowed_string_index(name))
            }
            _ => false,
        }
    }

    /// tsc's `errorIfWritingToReadonlyIndex`: whether a write through `self`
    /// keyed by `name` (a literal key; `None` for any other) lands on a
    /// `readonly` index signature. A name the receiver has as a member is a
    /// property write instead. A union answers through its own index
    /// signatures (`getUnionIndexInfos`): one per key kind every constituent
    /// declares, read-only when any constituent's is.
    pub fn writes_readonly_index(&self, name: Option<&str>, key_is_numeric: bool) -> bool {
        match self {
            Type::Object(object) => {
                name.is_none_or(|name| !object_answers_name(object, name))
                    && object.applicable_index_readonly(key_is_numeric)
            }
            Type::Reference(reference) => {
                reference.resolve().writes_readonly_index(name, key_is_numeric)
            }
            Type::Union(union) => {
                let mut string = Some(false);
                let mut number = Some(false);
                for member in union.types() {
                    if matches!(member, Type::Undefined | Type::Null) {
                        continue;
                    }
                    let Some((member_string, member_number)) =
                        union_member_index_readonly(member, name)
                    else {
                        return false;
                    };
                    string = string.zip(member_string).map(|(any, readonly)| any || readonly);
                    number = number.zip(member_number).map(|(any, readonly)| any || readonly);
                }
                match (number, string) {
                    (Some(readonly), _) if key_is_numeric => readonly,
                    (_, Some(readonly)) => readonly,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// `matches!(self.peeled(), Type::Never)`, without expanding a reference
    /// that cannot be `never` (an interface instantiation) and reading an
    /// expansion a reference already holds in place.
    pub fn is_never_peeled(&self) -> bool {
        fn walk(ty: &Type, depth: usize) -> bool {
            match ty {
                Type::Never => true,
                Type::Reference(reference) if depth < 8 => {
                    if reference.instantiates_interface() {
                        return false;
                    }
                    match reference.peek_resolved() {
                        Some(resolved) => walk(&resolved, depth + 1),
                        None => matches!(ty.peeled(), Type::Never),
                    }
                }
                Type::Reference(_) => matches!(ty.peeled(), Type::Never),
                _ => false,
            }
        }
        walk(self, 0)
    }

    pub fn get_property_access_type(&self, name: &str) -> Option<Type> {
        let own = self.own_property_access_type(name);
        // A primitive's, array's or function's apparent type is a lib interface
        // (`Number`, `Array<T>`, `Function`), and every object type answers the
        // global `Object` members its own declaration does not restate —
        // `getPropertyOfType`'s final `globalObjectType` fallback. Without it
        // `var o: Object = 1` was rejected for lacking `hasOwnProperty`.
        match self {
            Type::String
            | Type::StringLiteral(_)
            | Type::Number
            | Type::NumberLiteral(_)
            | Type::BigInt
            | Type::Symbol
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::Function(_)
            | Type::Array(_)
            | Type::Tuple(_)
            | Type::OpenTuple(_) => {
                own.or_else(|| crate::object::object_prototype_member_type(name))
            }
            _ => own,
        }
    }

    fn own_property_access_type(&self, name: &str) -> Option<Type> {
        match self {
            Type::Boolean | Type::BooleanLiteral(_) if name == "valueOf" => {
                Some(function_type(vec![], Type::Boolean, false, 0))
            }
            Type::Object(object) => object
                .get_property_access_type(name)
                .or_else(|| {
                    // A callable or constructable object *is* a `Function`, so it
                    // carries `name`, `length`, `call`/`apply`/`bind` — `typeof C`
                    // and `new (...args: any[]) => T` both answer `.name`.
                    let signature = object
                        .call_signature()
                        .or_else(|| object.construct_signature())?;
                    function_property_access_type(signature, name)
                })
                .or_else(|| object.intersected_primitive()?.get_property_access_type(name))
                .or_else(|| object.intersected_array()?.get_property_access_type(name))
                .or_else(|| narrowed_type_variable_member(object, name)),
            Type::Array(element) => array_property_access_type(name, element.as_ref()),
            // A tuple is an array, so it carries every `Array.prototype` method
            // (`includes`, `map`, …) over the union of its element types — not just
            // `length`. `["get","post"] as const` must answer `.includes`.
            Type::Tuple(elements) => tuple_property_access_type(name, elements),
            // `createTupleTargetType` stops naming element properties at the
            // first variable element, and a rest element makes `length` a
            // plain `number`.
            Type::OpenTuple(tuple) => {
                if name == "length" {
                    return Some(Type::Number);
                }
                if let Some(element) = tuple_element_property(name, &tuple.leading) {
                    return Some(element.clone());
                }
                array_property_access_type(name, &tuple.element_union())
            }
            Type::String | Type::StringLiteral(_) => string_property_access_type(name),
            Type::Number | Type::NumberLiteral(_) => number_property_access_type(name),
            Type::BigInt => bigint_property_access_type(name),
            Type::Symbol => symbol_property_access_type(name),
            Type::Function(function) => function_property_access_type(function, name),
            // Any property of `any` is `any`. A lazy reference can resolve to `any`
            // (e.g. `Promise<any>` collapses to its awaited `any`); without this arm
            // the access falls through to `None` and is misreported as a missing
            // property, where the old eager `any` shape emitted nothing.
            Type::Any => Some(Type::Any),
            Type::Reference(reference) => {
                if reference.is_readonly_array() && MUTATING_ARRAY_MEMBERS.contains(&name) {
                    return None;
                }
                if name == "length"
                    && let Some((elements, min_length)) = reference.written_tuple()
                {
                    return Some(tuple_length_literals(min_length, elements.len()));
                }
                reference.resolve().get_property_access_type(name)
            }
            // Every member must declare the property, and the read is their
            // union. Without this a nominal reference that *resolves* to a union
            // answered `None` for every property, which the callers report as a
            // missing member (zod's `$ZodInternalIssue<T>`, a union of 12
            // `Identity<…>` references that all carry `path`).
            // tsc's `createUnionOrIntersectionProperty` also finds none when a
            // constituent declares it private or protected and another has it
            // from a different declaration.
            Type::Union(union) => {
                let mut members = Vec::with_capacity(union.types().len());
                for member in union.types().iter() {
                    if matches!(member, Type::Undefined) {
                        members.push(Type::Undefined);
                        continue;
                    }
                    members.push(member.get_property_access_type(name)?);
                }
                if Type::union_property_declarations_differ(union.types(), name) {
                    return None;
                }
                (!members.is_empty()).then(|| crate::union_type(members))
            }
            _ => None,
        }
    }

    /// tsc's `createUnionOrIntersectionProperty` for a union: a member one
    /// constituent declares private or protected and another has from a
    /// different declaration is no property of the union.
    pub fn union_property_declarations_differ(members: &[Type], name: &str) -> bool {
        let restrictions: Vec<_> = members
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Null))
            .map(|member| member.property_restriction(name))
            .collect();
        restrictions.iter().any(Option::is_some) && restrictions.windows(2).any(|pair| pair[0] != pair[1])
    }

    /// The private/protected restriction of the member `name` this type
    /// declares, if it is restricted.
    pub fn property_restriction(&self, name: &str) -> Option<crate::object::MemberRestriction> {
        match self {
            Type::Object(object) => object.get_property(name)?.restriction.clone(),
            Type::Reference(reference) => reference.resolve().property_restriction(name),
            _ => None,
        }
    }

    pub fn builtin_constructor_result_type(name: &str) -> Option<Type> {
        match name {
            "Array" => Some(Type::Array(Box::new(Type::Any))),
            "Map" => Some(Type::Object(ObjectType::new(
                {
                    let mut properties = crate::PropertyMap::default();
                    properties.insert(
                        "get".into(),
                        crate::ObjectProperty::required(function_type(
                            vec![Type::Any],
                            Type::Any,
                            false,
                            1,
                        )),
                    );
                    properties.insert(
                        "set".into(),
                        crate::ObjectProperty::required(function_type(
                            vec![Type::Any, Type::Any],
                            Type::Any,
                            false,
                            2,
                        )),
                    );
                    properties.insert(
                        "has".into(),
                        crate::ObjectProperty::required(function_type(
                            vec![Type::Any],
                            Type::Boolean,
                            false,
                            1,
                        )),
                    );
                    properties.insert(
                        "delete".into(),
                        crate::ObjectProperty::required(function_type(
                            vec![Type::Any],
                            Type::Boolean,
                            false,
                            1,
                        )),
                    );
                    properties.insert(
                        "clear".into(),
                        crate::ObjectProperty::required(function_type(
                            vec![],
                            Type::Void,
                            false,
                            0,
                        )),
                    );
                    properties.insert("size".into(), crate::ObjectProperty::required(Type::Number));
                    properties
                },
                None,
            ))),
            _ => None,
        }
    }

    pub fn name(&self) -> String {
        let name = self.render_name();
        // The node builder counts a keyword or a literal as it is written, and
        // a type parameter as a name it looks up; `unknown` it does not count.
        match self {
            Type::TypeParameter(_) => crate::tsc_display::add_length(crate::tsc_display::symbol_name_length(&name)),
            Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Any
            | Type::ErrorType
            | Type::Unknown
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => crate::tsc_display::add_length(name.len()),
            _ => {}
        }
        name
    }

    fn render_name(&self) -> String {
        match self {
            Type::String => "string".to_string(),
            Type::Number => "number".to_string(),
            Type::Boolean => "boolean".to_string(),
            Type::BigInt => "bigint".to_string(),
            Type::Symbol => "symbol".to_string(),
            Type::Undefined => "undefined".to_string(),
            Type::Null => "null".to_string(),
            Type::Void => "void".to_string(),
            // tsc prints its error type as the `any` it is.
            Type::Any | Type::ErrorType => "any".to_string(),
            // The sentinel stands for a type the checker could not compute:
            // tsc's error type.
            Type::Unknown if crate::tsc_display::active() => "any".to_string(),
            Type::Unknown | Type::GenuineUnknown => "unknown".to_string(),
            Type::TypeParameter(parameter) => parameter.name.to_string(),
            Type::Never => "never".to_string(),
            Type::StringLiteral(value) => format!("{value:?}"),
            Type::NumberLiteral(value) => value.value.clone(),
            Type::BooleanLiteral(value) => value.to_string(),
            Type::Function(function) => crate::cycle::as_ancestor(self, || function.name()),
            Type::Object(object) if crate::tsc_display::active() && object.alias_name.is_none() => {
                crate::cycle::as_ancestor(self, || Type::render_object_name(object))
            }
            Type::Object(object) => Type::render_object_name(object),
            Type::Array(element) => format!("{}[]", array_element_name(element)),
            Type::Tuple(tuple) => {
                let indices: Vec<usize> = (0..tuple.len()).collect();
                let elements = crate::tsc_display::render_type_list(&indices, false, |&index| {
                    tuple_element_name(&tuple[index], tuple.label(index), tuple.is_flagged_optional(index), false)
                })
                .join(", ");
                format!("[{elements}]")
            }
            Type::OpenTuple(tuple) => {
                let label = |index: usize| tuple.labels.as_deref().and_then(|labels| labels.get(index)?.as_deref());
                let leading_min_length = tuple_min_length(&tuple.leading);
                let mut parts = tuple
                    .leading
                    .iter()
                    .enumerate()
                    .map(|(index, element)| tuple_element_name(element, label(index), index >= leading_min_length, false))
                    .collect::<Vec<_>>();
                parts.push(tuple_element_name(&tuple.rest, label(tuple.leading.len()), false, true));
                parts.extend(tuple.trailing.iter().enumerate().map(|(index, element)| {
                    tuple_element_name(element, label(tuple.leading.len() + 1 + index), false, false)
                }));
                format!("[{}]", parts.join(", "))
            }
            Type::Union(union) => union.name(),
            Type::Reference(reference) if reference.written_tuple().is_some() => reference.arguments[0].name(),
            Type::Reference(reference) if reference.render_structurally => {
                match reference.resolve() {
                    Type::Object(object) => object_structural_name(&object),
                    peeled => peeled.name(),
                }
            }
            Type::Reference(reference) if let Some(alias) = &reference.alias_display => crate::tsc_display::counted(alias.to_string()),
            Type::Reference(reference) if let Some(operand) = reference.reduced_intersection_operand() => operand.name(),
            // A lazily resolved annotation keeps the text it was written with;
            // tsc prints an `Array<T>` as the `T[]` it is.
            Type::Reference(reference) if crate::tsc_display::is_written_array_reference(reference) => {
                match reference.resolve() {
                    resolved @ Type::Array(_) => resolved.name(),
                    Type::Reference(resolved) if resolved.is_readonly_array() => Type::Reference(resolved).name(),
                    _ => crate::tsc_display::diagnostic_reference_name(reference),
                }
            }
            // `NoInfer<T>` instantiated is its argument (`getSubstitutionType`
            // over a non-generic base); only a type variable keeps the marker.
            Type::Reference(reference) if reference.display.starts_with("NoInfer<") => match reference.resolve() {
                Type::TypeParameter(_) => crate::tsc_display::diagnostic_reference_name(reference),
                resolved => resolved.name(),
            },
            Type::Reference(reference) if crate::tsc_display::active() => crate::tsc_display::reference_name(reference),
            Type::Reference(reference) => crate::tsc_display::diagnostic_reference_name(reference),
        }
    }

    fn render_object_name(object: &crate::ObjectType) -> String {
        if let Some(alias_name) = &object.alias_name {
            return crate::tsc_display::counted_written_name(alias_name);
        }
        // A type variable intersected with members (`T & { extra: number }`,
        // a generic spread) is open over what the variable holds; tsc prints
        // the intersection.
        if crate::tsc_display::active()
            && object.is_intersection
            && object.synthetic_open_index
            && matches!(object.string_index_type.as_deref(), Some(Type::Any))
            && !object.properties.is_empty()
            && let Some(operands) = object.intersection_operands.as_deref()
            && operands.iter().all(|operand| matches!(operand, Type::TypeParameter(_)))
        {
            let mut members = object.clone();
            members.string_index_type = None;
            members.synthetic_open_index = false;
            members.is_intersection = false;
            members.intersection_operands = None;
            let members = object_structural_name(&members);
            let variables = crate::tsc_display::render_type_list(operands, true, Type::name);
            return match object.spread_operands_last {
                true => std::iter::once(members).chain(variables).collect::<Vec<_>>().join(" & "),
                false => variables.into_iter().chain(std::iter::once(members)).collect::<Vec<_>>().join(" & "),
            };
        }
        // A narrowed type variable (`T & string`) has no members of its
        // own; its operands are the whole type.
        if object.is_intersection
            && object.properties.is_empty()
            && object.string_index_type.is_none()
            && object.number_index_type.is_none()
            && let Some(operands) = object.intersection_operands.as_deref()
            && operands.iter().any(|operand| matches!(operand, Type::TypeParameter(_)))
        {
            return crate::tsc_display::render_type_list(operands, true, Type::name).join(" & ");
        }
        object_structural_name(object)
    }
}

thread_local! {
    /// The object literals one top-level rendering may still spell out. A
    /// structure that shares its members (a schema library's expanded
    /// interfaces) otherwise renders exponentially; tsc truncates such a type
    /// as well.
    static STRUCTURAL_NAME_NODES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

const STRUCTURAL_NAME_NODE_BUDGET: usize = 128;

/// Runs `render` outside the rendering in progress's object budget.
pub(crate) fn without_structural_budget<R>(render: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STRUCTURAL_NAME_NODES.with(|nodes| nodes.set(self.0));
        }
    }
    let _restore = Restore(STRUCTURAL_NAME_NODES.with(|nodes| nodes.take()));
    render()
}

fn object_structural_name(object: &crate::ObjectType) -> String {
    let outermost = STRUCTURAL_NAME_NODES.with(|nodes| nodes.get().is_none());
    let remaining = STRUCTURAL_NAME_NODES.with(|nodes| nodes.get()).unwrap_or(STRUCTURAL_NAME_NODE_BUDGET);
    if remaining == 0 {
        return "{ ... }".to_string();
    }
    STRUCTURAL_NAME_NODES.with(|nodes| nodes.set(Some(remaining - 1)));
    let name = crate::name_memo::bounded_name(spelled_object_structural_name(object));
    if outermost {
        STRUCTURAL_NAME_NODES.with(|nodes| nodes.set(None));
    }
    name
}

fn spelled_object_structural_name(object: &crate::ObjectType) -> String {
    let has_index_signatures =
        object.string_index_type.is_some() || object.number_index_type.is_some() || object.symbol_index_type().is_some();
    if object.properties.is_empty() && !has_index_signatures {
        // An object that is nothing but one signature prints as that
        // signature, as tsc prints `new (x: number) => T` and
        // `(x: string) => string`.
        match (object.call_signature(), object.construct_signature()) {
            (Some(call), None) => return Type::Function(call.clone()).name(),
            (None, Some(construct)) => {
                // An overload group prints as an object of its
                // construct signatures.
                if let Some(overloads) = construct.overloads().filter(|overloads| overloads.len() > 1) {
                    if crate::tsc_display::truncating() {
                        return "{ ...; }".to_string();
                    }
                    let members: Vec<String> =
                        overloads.iter().map(|overload| format!("new {};", overload.member_name())).collect();
                    crate::tsc_display::add_length(2);
                    return format!("{{ {} }}", members.join(" "));
                }
                return format!("new {}", Type::Function(construct.clone()).name());
            }
            _ => {}
        }
        if object.non_primitive {
            return crate::tsc_display::counted_keyword("object");
        }
        crate::tsc_display::add_length(2);
        return "{}".to_string();
    }
    // `createTypeNodesFromResolvedType` spells nothing of an object type it
    // reaches once the display is long.
    if crate::tsc_display::truncating() {
        return "{ ...; }".to_string();
    }

    // tsc prints an object type's signatures, then its index signatures,
    // ahead of its properties.
    let signatures = |signature: &FunctionType, prefix: &str| -> Vec<String> {
        match signature.overloads() {
            Some(overloads) => overloads.iter().map(|overload| format!("{prefix}{}", overload.member_name())).collect(),
            None => vec![format!("{prefix}{}", signature.member_name())],
        }
    };
    let mut members = Vec::new();
    if let Some(call) = object.call_signature() {
        members.extend(signatures(call, ""));
    }
    if let Some(construct) = object.construct_signature() {
        members.extend(signatures(construct, "new "));
    }

    let modifier = if object.string_index_readonly { "readonly " } else { "" };
    // tsc prints the parameter name an index signature was declared
    // with, and `x` for one synthesized from keys.
    let names = object.index_parameter_names.as_deref();
    let string_name = names.and_then(|names| names.string.as_deref()).unwrap_or("x");
    let number_name = names.and_then(|names| names.number.as_deref()).unwrap_or("x");
    let index_signature = |modifier: &str, parameter: &str, key: String, value: &Type| {
        let signature = format!("{modifier}[{parameter}: {key}]: {}", value.name());
        crate::tsc_display::add_length(parameter.len() + 4);
        signature
    };
    if let Some(keyed) = object.active_keyed_indexes() {
        members.extend(keyed.infos.iter().map(|(key, value)| index_signature(modifier, string_name, key.name(), value)));
    } else if let Some(index_type) = &object.string_index_type {
        members.push(index_signature(modifier, string_name, crate::tsc_display::counted_keyword("string"), index_type));
    }
    if let Some(index_type) = &object.number_index_type {
        let modifier = if object.number_index_readonly { "readonly " } else { "" };
        members.push(index_signature(modifier, number_name, crate::tsc_display::counted_keyword("number"), index_type));
    }
    if let Some(index_type) = object.symbol_index_type() {
        members.push(index_signature("", string_name, crate::tsc_display::counted_keyword("symbol"), index_type));
    }

    let property_parts = |name: &str, property: &crate::ObjectProperty| -> Vec<String> {
        let name = written_property_key_display(name, property.written_name.as_deref().map(String::as_str));
        crate::tsc_display::add_length(name.len() + 1);
        // tsc prints a member declared with method syntax as
        // one, overload by overload: `f(s: string): number`.
        if property.method
            && let Type::Function(function) = &property.ty
            && function.alias_name().is_none()
        {
            let optional = if property.is_optional() { "?" } else { "" };
            return match function.overloads() {
                Some(overloads) => {
                    overloads.iter().map(|overload| format!("{name}{optional}{}", overload.member_name())).collect()
                }
                None => vec![format!("{name}{optional}{}", function.member_name())],
            };
        }
        let modifier = if property.readonly { "readonly " } else { "" };
        let part = if property.is_optional() {
            format!("{modifier}{name}?: {}", crate::cycle::property_value(|| optional_property_display(&property.ty)))
        } else {
            format!("{modifier}{name}: {}", crate::cycle::property_value(|| property.ty.name()))
        };
        if property.readonly {
            crate::tsc_display::add_length(9);
        }
        vec![part]
    };
    let mut spelled_bytes = 0usize;
    let mut capped = false;
    let count = object.properties.len();
    for (index, (name, property)) in object.properties.iter().enumerate() {
        let position = index + 1;
        // Past the truncation length tsc writes `... N more ...` and the last
        // property in place of the rest.
        if crate::tsc_display::truncating() && position + 2 < count - 1 {
            members.push(format!("... {} more ...", count - position));
            if let Some((name, property)) = object.properties.iter().last() {
                members.extend(property_parts(name, property));
            }
            break;
        }
        for part in property_parts(name, property) {
            capped = spelled_bytes >= crate::name_memo::DISPLAY_CAP_BYTES;
            if capped {
                break;
            }
            spelled_bytes += part.len() + 2;
            members.push(part);
        }
        if capped {
            members.push("...".to_string());
            break;
        }
    }
    if members.is_empty() {
        return if object.non_primitive { "object" } else { "{}" }.to_string();
    }
    crate::tsc_display::add_length(2);
    format!("{{ {}; }}", members.join("; "))
}

fn string_property_access_type(name: &str) -> Option<Type> {
    match name {
        "length" => Some(Type::Number),
        ITERATION_PROTOCOL_MEMBER => Some(iteration_protocol_type(&Type::String)),
        // `searchValue` may be a string or a `RegExp`, so it stays permissive.
        // The replacement is a string *or* a replacer function; spelling that as
        // a union rather than `Any` keeps argument checking just as permissive
        // while giving an inline replacer its contextual parameter types.
        "replace" | "replaceAll" => Some(function_type(
            vec![
                Type::Any,
                crate::union_type(vec![
                    Type::String,
                    function_type(vec![Type::String], Type::String, true, 1),
                ]),
            ],
            Type::String,
            false,
            2,
        )),
        // `(searchString, position?)` — the optional second argument is a number,
        // so declaring only the string parameter made every positional call
        // (`s.includes(v, from)`) a false TS2345.
        "indexOf" | "lastIndexOf" => Some(function_type(
            vec![Type::String, Type::Number],
            Type::Number,
            false,
            1,
        )),
        "search" => Some(function_type(vec![Type::Any], Type::Number, false, 1)),
        "split" => Some(function_type(
            vec![Type::Any, Type::Number],
            Type::Array(Box::new(Type::String)),
            false,
            1,
        )),
        // lib.es5: `slice(start?, end?)`, but `substring(start, end?)` and
        // `substr(from, length?)`.
        "slice" => Some(function_type(vec![Type::Number, Type::Number], Type::String, false, 0)),
        "substring" | "substr" => {
            Some(function_type(vec![Type::Number, Type::Number], Type::String, false, 1))
        }
        "startsWith" | "endsWith" | "includes" => Some(function_type(
            vec![Type::String, Type::Number],
            Type::Boolean,
            false,
            1,
        )),
        "toLowerCase" | "toUpperCase" | "trim" | "trimStart" | "trimEnd" | "trimLeft"
        | "trimRight" => Some(function_type(vec![], Type::String, false, 0)),
        // `(locales?)`, a string, a locale or a list of them.
        "toLocaleLowerCase" | "toLocaleUpperCase" => {
            Some(function_type(vec![Type::Any], Type::String, false, 0))
        }
        // `normalize(form?)` takes the optional Unicode normalization form.
        "normalize" => Some(function_type(vec![Type::String], Type::String, false, 0)),
        "toString" | "valueOf" => Some(function_type(vec![], Type::String, false, 0)),
        "repeat" => Some(function_type(vec![Type::Number], Type::String, false, 1)),
        "concat" => Some(function_type(vec![Type::String], Type::String, true, 0)),
        "charAt" => Some(function_type(vec![Type::Number], Type::String, false, 1)),
        "at" => Some(function_type(
            vec![Type::Number],
            Type::Union(UnionType::new(vec![Type::String, Type::Undefined])),
            false,
            1,
        )),
        "padStart" | "padEnd" => Some(function_type(
            vec![Type::Number, Type::String],
            Type::String,
            false,
            1,
        )),
        "charCodeAt" => Some(function_type(vec![Type::Number], Type::Number, false, 1)),
        "codePointAt" => Some(function_type(
            vec![Type::Number],
            Type::Union(UnionType::new(vec![Type::Number, Type::Undefined])),
            false,
            1,
        )),
        // lib.es2015.core's HTML methods and lib.es2024.string.
        "anchor" | "fontcolor" | "link" => {
            Some(function_type(vec![Type::String], Type::String, false, 1))
        }
        "fontsize" => Some(function_type(vec![Type::Any], Type::String, false, 1)),
        "big" | "blink" | "bold" | "fixed" | "italics" | "small" | "strike" | "sub" | "sup"
        | "toWellFormed" => Some(function_type(vec![], Type::String, false, 0)),
        "isWellFormed" => Some(function_type(vec![], Type::Boolean, false, 0)),
        // `(that, locales?, options?)`.
        "localeCompare" => Some(function_type(
            vec![Type::String, Type::Any, Type::Any],
            Type::Number,
            false,
            1,
        )),
        _ => None,
    }
}

fn number_property_access_type(name: &str) -> Option<Type> {
    match name {
        "toString" => Some(function_type(vec![Type::Number], Type::String, false, 0)),
        "toFixed" | "toPrecision" | "toExponential" => {
            Some(function_type(vec![Type::Number], Type::String, false, 0))
        }
        // `(locales?, options?)`.
        "toLocaleString" => Some(function_type(vec![Type::Any, Type::Any], Type::String, false, 0)),
        "valueOf" => Some(function_type(vec![], Type::Number, false, 0)),
        _ => None,
    }
}

fn bigint_property_access_type(name: &str) -> Option<Type> {
    match name {
        "toString" => Some(function_type(vec![Type::Number], Type::String, false, 0)),
        "toLocaleString" => Some(function_type(vec![Type::Any, Type::Any], Type::String, false, 0)),
        "valueOf" => Some(function_type(vec![], Type::BigInt, false, 0)),
        _ => None,
    }
}

fn symbol_property_access_type(name: &str) -> Option<Type> {
    match name {
        "toString" => Some(function_type(vec![], Type::String, false, 0)),
        "valueOf" => Some(function_type(vec![], Type::Symbol, false, 0)),
        "description" => Some(Type::Union(UnionType::new(vec![
            Type::String,
            Type::Undefined,
        ]))),
        _ => None,
    }
}

/// A tuple element as `typeToTypeNode` writes it: a `NamedTupleMember` when
/// labelled, else an `OptionalType` or `RestType` around the element type.
fn tuple_element_name(element: &Type, label: Option<&str>, optional: bool, rest: bool) -> String {
    match label {
        Some(label) if rest => format!("...{label}: {}[]", array_element_name(element)),
        Some(label) if optional => format!("{label}?: {}", element.name()),
        Some(label) => format!("{label}: {}", element.name()),
        None if rest => format!("...{}[]", array_element_name(element)),
        None if optional => format!("{}?", array_element_name(element)),
        None => element.name(),
    }
}

fn tuple_property_access_type(name: &str, elements: &crate::TupleType) -> Option<Type> {
    if name == "length" {
        return Some(tuple_length_literals(elements.min_length(), elements.len()));
    }
    if let Some(element) = tuple_element_property(name, elements) {
        return Some(element.clone());
    }
    array_property_access_type(name, &tuple_element_union(elements))
}

/// The union of a fixed tuple's element types, `never` for `[]`: its number
/// index type and the `T` of the `Array<T>` members it carries.
pub fn tuple_element_union(elements: &[Type]) -> Type {
    if elements.is_empty() {
        Type::Never
    } else {
        // Flatten and dedup (a tuple element may itself be a union, and
        // repeated literals are common in `as const` tables); a raw nested
        // union fails member-wise assignability against its flat equivalent.
        crate::union_type(elements.to_vec())
    }
}

/// The element property `createTupleTargetType` names by position (`"0"`,
/// `"1"`, …); a non-canonical spelling such as `"01"` names nothing.
fn tuple_element_property<'a>(name: &str, elements: &'a [Type]) -> Option<&'a Type> {
    let index = name.parse::<usize>().ok()?;
    if index.to_string() != name {
        return None;
    }
    elements.get(index)
}

/// The `minLength` of a fixed tuple read off its element types alone. surge
/// lowers an optional element (`[A, B?]`) to a slot whose type carries
/// `undefined`, and optional elements can only trail the required ones, so
/// the elements that may be absent are the trailing run of such slots. A
/// written tuple this reading gets wrong is a [`written_tuple_type`].
pub fn tuple_min_length(elements: &[Type]) -> usize {
    let optional = elements
        .iter()
        .rev()
        .take_while(|element| match element {
            Type::Undefined => true,
            Type::Union(union) => union.types().contains(&Type::Undefined),
            _ => false,
        })
        .count();
    elements.len() - optional
}

/// The `length` member `createTupleTargetType` gives a fixed tuple: the union
/// of the number literals from its `minLength` to its arity.
pub fn tuple_length_type(elements: &[Type]) -> Type {
    tuple_length_literals(tuple_min_length(elements), elements.len())
}

pub(crate) fn tuple_length_literals(min_length: usize, arity: usize) -> Type {
    crate::union_type((min_length..=arity).map(number_literal).collect())
}

fn number_literal(value: usize) -> Type {
    Type::NumberLiteral(NumberLiteralType {
        value: value.to_string(),
    })
}

/// A tuple type as written, with the `minLength` tsc counts from its element
/// flags. The plain [`Type::Tuple`] stands for it where [`tuple_min_length`]
/// reads the same length off the element types; where it cannot —
/// `[a?: any]`, whose `any` absorbs the optional `undefined`, a required
/// `[a: T | undefined]`, any optional element once `strictNullChecks` drops
/// the `undefined` — the tuple is wrapped in a [`WRITTEN_TUPLE_REFERENCE_ID`]
/// reference that records it, and a consumer that peels still sees the tuple.
///
/// [`WRITTEN_TUPLE_REFERENCE_ID`]: crate::WRITTEN_TUPLE_REFERENCE_ID
/// A class's polymorphic `this` over its instance type (see
/// [`crate::POLYMORPHIC_THIS_REFERENCE_ID`]).
pub fn polymorphic_this_type(instance: Type) -> Type {
    Type::Reference(TypeReference::new(
        crate::POLYMORPHIC_THIS_REFERENCE_ID,
        "this",
        vec![instance.clone()],
        std::sync::Arc::new(FixedResolved(instance)),
    ))
}

/// The instance a class's polymorphic `this` stands for; any other type as it
/// is.
pub fn without_polymorphic_this(ty: Type) -> Type {
    match ty {
        Type::Reference(ref reference) if reference.is_polymorphic_this() => reference.arguments[0].clone(),
        other => other,
    }
}

pub fn written_tuple_type(elements: Vec<Type>, min_length: usize) -> Type {
    written_labeled_tuple_type(elements, min_length, None)
}

/// [`written_tuple_type`] for a tuple written with element labels (or built
/// from a parameter list, whose names tsc keeps as the labels).
pub fn written_labeled_tuple_type(elements: Vec<Type>, min_length: usize, labels: Option<Vec<crate::TupleLabel>>) -> Type {
    let min_length = min_length.min(elements.len());
    let wrapped = min_length != tuple_min_length(&elements);
    let tuple = Type::Tuple(crate::TupleType::with_shape(elements, min_length, labels));
    if !wrapped {
        return tuple;
    }
    Type::Reference(TypeReference::new(
        crate::WRITTEN_TUPLE_REFERENCE_ID,
        tuple.name(),
        vec![tuple.clone(), number_literal(min_length)],
        std::sync::Arc::new(FixedResolved(tuple)),
    ))
}

pub fn is_written_tuple(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if reference.written_tuple().is_some())
}

/// A fixed tuple's elements and `minLength`: a written tuple's recorded one,
/// a plain tuple's as [`tuple_min_length`] reads it.
pub fn fixed_tuple_parts(ty: &Type) -> Option<(&[Type], usize)> {
    match ty {
        Type::Tuple(elements) => Some((elements.as_slice(), elements.min_length())),
        Type::Reference(reference) => reference.written_tuple(),
        _ => None,
    }
}

/// The element labels of a fixed tuple, written or plain.
pub fn fixed_tuple_labels(ty: &Type) -> Option<Vec<crate::TupleLabel>> {
    match ty {
        Type::Tuple(tuple) => tuple.labels().map(<[_]>::to_vec),
        Type::Reference(reference) if reference.written_tuple().is_some() => fixed_tuple_labels(&reference.arguments[0]),
        _ => None,
    }
}

/// Whether `name` is a member of `object` rather than a key its index
/// signatures answer: a declared property (not a flow-narrowed index slot), a
/// private name, a global `Object` member, or — on a callable or constructable
/// object — a `Function` member. `getPropertyOfType` consults all of them
/// before any index signature.
fn object_answers_name(object: &crate::ObjectType, name: &str) -> bool {
    object.get_property(name).is_some_and(|property| !property.index_slot)
        || crate::private_name::is_private_name_key(name)
        || crate::object::object_prototype_member_type(name).is_some()
        || (object.call_signature().is_some() || object.construct_signature().is_some())
            && FUNCTION_MEMBER_NAMES.contains(&name)
}

pub(crate) fn is_function_member_name(name: &str) -> bool {
    FUNCTION_MEMBER_NAMES.contains(&name)
}

/// The members of lib.es5.d.ts's `interface Function`, lib.es2015.core's
/// `name`, and lib.esnext.decorators' `[Symbol.metadata]`.
const FUNCTION_MEMBER_NAMES: &[&str] = &[
    "apply", "arguments", "bind", "call", "caller", "length", "name", "prototype", "toString",
    "[Symbol.hasInstance]", "[Symbol.metadata]",
];

/// The index signatures a union constituent contributes to the union's own
/// (`getUnionIndexInfos` reads each constituent's apparent type): the string
/// and the number one, `Some(readonly)` where it declares it. `None` when the
/// constituent has `name` as a member — the write is then to the union's
/// property — or has a shape whose index signatures surge does not know.
fn union_member_index_readonly(
    member: &Type,
    name: Option<&str>,
) -> Option<(Option<bool>, Option<bool>)> {
    match member {
        Type::Object(object) => {
            if object.synthetic_open_index || name.is_some_and(|name| object_answers_name(object, name)) {
                return None;
            }
            Some((
                object.string_index_type.is_some().then_some(object.string_index_readonly),
                object.number_index_type.is_some().then_some(object.number_index_readonly),
            ))
        }
        // lib.es5.d.ts: `interface String { readonly [index: number]: string; }`.
        Type::String | Type::StringLiteral(_) => {
            let has_member = name.is_some_and(|name| {
                string_property_access_type(name).is_some()
                    || crate::object::object_prototype_member_type(name).is_some()
            });
            (!has_member).then_some((None, Some(true)))
        }
        Type::Array(element) => {
            let has_member = name.is_some_and(|name| {
                array_property_access_type(name, element).is_some()
                    || crate::object::object_prototype_member_type(name).is_some()
            });
            (!has_member).then_some((None, Some(false)))
        }
        // `ReadonlyArray<T>` declares `readonly [n: number]: T`.
        Type::Reference(reference) if reference.is_readonly_array() => {
            let has_member = name.is_some_and(|name| {
                reference.resolve().get_property_access_type(name).is_some()
            });
            (!has_member).then_some((None, Some(true)))
        }
        Type::Reference(reference) => union_member_index_readonly(&reference.resolve(), name),
        _ => None,
    }
}

/// The members every function value carries via `Function`/`CallableFunction`
/// (tsc reports none of these as missing). Under `strictBindCallApply` `call`
/// and `apply` yield the function's own return type; modelled as variadic with
/// no required parameters so an arbitrary `thisArg`/args list is accepted. `bind`
/// yields a bound function, approximated by the original function type so a later
/// call still resolves.
fn function_property_access_type(function: &FunctionType, name: &str) -> Option<Type> {
    match name {
        "call" | "apply" => Some(function_type(
            vec![],
            function.return_type().clone(),
            true,
            0,
        )),
        "bind" => Some(function_type(
            vec![],
            Type::Function(function.clone()),
            true,
            0,
        )),
        "length" => Some(Type::Number),
        "name" => Some(Type::String),
        "prototype" => Some(Type::Any),
        "toString" | "toLocaleString" => Some(function_type(vec![], Type::String, false, 0)),
        _ => None,
    }
}

/// The `(element, index, array)` callback signature shared by the array
/// iteration methods. Modelling all three parameters (rather than just the
/// element) lets a `(v, i) => …` callback type its index parameter as `number`
/// and stay assignable, instead of cascading into `TS7006`/`TS2345`.
fn array_iteration_callback(element: &Type, return_type: Type) -> Type {
    function_type(
        vec![
            element.clone(),
            Type::Number,
            Type::Array(Box::new(element.clone())),
        ],
        return_type,
        false,
        // All three are required in the lib's signature; the target passes
        // every one, and a callback is free to declare fewer.
        3,
    )
}

/// The member name `[Symbol.iterator]` parses to; see `computed_key_name` in the
/// parser, which renders a computed key under a name no written identifier can
/// spell.
pub const ITERATION_PROTOCOL_MEMBER: &str = "[Symbol.iterator]";

/// The iteration protocol an array, tuple or string carries through the lib's
/// `[Symbol.iterator](): ArrayIterator<T>` / `StringIterator<T>`. surge models
/// these as their own [`Type`] variants rather than as `Array`/`String`
/// interface instances, so nothing supplied that member and no array ever
/// satisfied `Iterable<T>`: every `take([1, 2, 3])` against
/// `take<T>(items: Iterable<T>)` was a false `TS2345`.
///
/// Only what the relation reads is modelled. `next()` yields `{ value: E }`,
/// the least `IteratorYieldResult<T>` accepts — `done` is optional there — so
/// the element is still checked and `number[]` stays rejected against
/// `Iterable<string>`.
fn iteration_protocol_type(element: &Type) -> Type {
    let mut yielded = crate::PropertyMap::default();
    yielded.insert(
        std::sync::Arc::from("value"),
        crate::ObjectProperty::required(element.clone()),
    );
    let yielded = Type::Object(ObjectType::new(yielded, None));

    let mut iterator = crate::PropertyMap::default();
    iterator.insert(
        std::sync::Arc::from("next"),
        crate::ObjectProperty::required(function_type(vec![], yielded, false, 0)),
    );
    let iterator = Type::Object(ObjectType::new(iterator, None));

    function_type(vec![], iterator, false, 0)
}

fn element_or_undefined(element: &Type) -> Type {
    crate::union_type(vec![element.clone(), Type::Undefined])
}

/// Every member name [`array_property_access_type`] answers. An interface that
/// `extends Array<T>`/`ReadonlyArray<T>` inherits a structural member set, and
/// the array surface is modelled by a name lookup rather than a property map —
/// this is what lets the heritage merge materialize it.
/// `Array.prototype` members `ReadonlyArray` does not declare.
pub const MUTATING_ARRAY_MEMBERS: &[&str] = &[
    "push", "pop", "shift", "unshift", "splice", "sort", "reverse", "fill", "copyWithin",
];

pub fn array_property_names() -> &'static [&'static str] {
    &[
        "length", "map", "find", "findLast", "findIndex", "findLastIndex", "filter", "some",
        "every", "forEach", "flatMap", "flat", "reduce", "reduceRight", "join", "concat", "slice",
        "sort", "reverse", "fill", "splice", "push", "pop", "shift", "unshift", "at", "indexOf",
        "lastIndexOf", "includes", "values", "keys", "entries", "toString", "toLocaleString",
        "toSorted", "toReversed", "toSpliced", "with",
    ]
}

/// lib.es5's `ConcatArray<T>`: what `concat` reads of each array it joins.
fn concat_array_type(element: &Type) -> Type {
    let mut properties = crate::PropertyMap::default();
    properties.insert("length".into(), crate::ObjectProperty::required(Type::Number).with_readonly(true));
    properties.insert(
        "join".into(),
        crate::ObjectProperty::required(function_type(vec![Type::String], Type::String, false, 0)).with_method(true),
    );
    properties.insert(
        "slice".into(),
        crate::ObjectProperty::required(function_type(
            vec![Type::Number, Type::Number],
            Type::Array(Box::new(element.clone())),
            false,
            0,
        ))
        .with_method(true),
    );
    Type::Object(
        ObjectType::new(properties, None)
            .with_number_index_type(Some(element.clone()))
            .with_readonly_indexes(false, true)
            .with_alias_name(format!("ConcatArray<{}>", element.name())),
    )
}

pub fn array_member_type(name: &str, element: &Type) -> Option<Type> {
    array_property_access_type(name, element)
}

fn array_property_access_type(name: &str, element: &Type) -> Option<Type> {
    // Every iteration method takes the lib's optional `thisArg?: any` after
    // its callback.
    match name {
        "length" => Some(Type::Number),
        ITERATION_PROTOCOL_MEMBER => Some(iteration_protocol_type(element)),
        "map" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Array(Box::new(Type::Any)),
            false,
            1,
        )),
        // Like `filter` below, the lib types these predicates as returning
        // `unknown` (the non-narrowing overload), so any truthy value is accepted.
        "find" | "findLast" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            element_or_undefined(element),
            false,
            1,
        )),
        "findIndex" | "findLastIndex" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Number,
            false,
            1,
        )),
        // The predicate's return is `unknown` in the lib, not `boolean`: any
        // truthy value filters (`lines.filter((x) => x)`), so demanding `boolean`
        // reports a `TS2345` tsc never emits.
        "filter" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Array(Box::new(element.clone())),
            false,
            1,
        )),
        "some" | "every" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Boolean,
            false,
            1,
        )),
        "forEach" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Void,
            false,
            1,
        )),
        "flatMap" => Some(function_type(
            vec![array_iteration_callback(element, Type::Any), Type::Any],
            Type::Array(Box::new(Type::Any)),
            false,
            1,
        )),
        "flat" => Some(function_type(
            vec![Type::Number],
            Type::Array(Box::new(Type::Any)),
            true,
            0,
        )),
        // `reduce`/`reduceRight` carry an accumulator type we do not infer, so
        // the accumulator and the result stay `Any` — but the callback's *shape*
        // is known, and modelling it is what contextually types an inline
        // reducer instead of reporting every parameter as an implicit any.
        "reduce" | "reduceRight" => Some(function_type(
            vec![
                function_type(
                    vec![
                        Type::Any,
                        element.clone(),
                        Type::Number,
                        Type::Array(Box::new(element.clone())),
                    ],
                    Type::Any,
                    false,
                    // All four are required in the lib's signature.
                    4,
                ),
                Type::Any,
            ],
            Type::Any,
            false,
            1,
        )),
        "join" => Some(function_type(vec![Type::String], Type::String, false, 0)),
        "toString" => Some(function_type(vec![], Type::String, false, 0)),
        // lib.es2015.core adds `toLocaleString(locales, options?)`.
        "toLocaleString" => Some(function_type(vec![Type::Any, Type::Any], Type::String, false, 0)),
        // `concat(...items: ConcatArray<T>[]): T[]` and
        // `concat(...items: (T | ConcatArray<T>)[]): T[]`.
        "concat" => {
            let items = concat_array_type(element);
            let either = crate::union_type(vec![element.clone(), items.clone()]);
            let returns = Type::Array(Box::new(element.clone()));
            let arrays = FunctionType::new(vec![Type::Array(Box::new(items))], returns.clone(), true, 0);
            let mixed = FunctionType::new(vec![Type::Array(Box::new(either.clone()))], returns.clone(), true, 0);
            Some(Type::Function(
                FunctionType::new(vec![Type::Array(Box::new(either))], returns, true, 0)
                    .with_overloads(vec![arrays, mixed]),
            ))
        }
        "slice" => Some(function_type(
            vec![Type::Number, Type::Number],
            Type::Array(Box::new(element.clone())),
            false,
            0,
        )),
        // `sort`'s optional comparator is `(a, b) => number`; modelling both
        // parameters lets `(left, right) => …` type them as the element type
        // instead of cascading into `TS7006`.
        // ES2023's copying twins return a new array but take what the
        // mutating originals take.
        "sort" | "toSorted" => Some(function_type(
            vec![function_type(
                vec![element.clone(), element.clone()],
                Type::Number,
                false,
                2,
            )],
            Type::Array(Box::new(element.clone())),
            false,
            0,
        )),
        "reverse" | "toReversed" => Some(function_type(
            vec![],
            Type::Array(Box::new(element.clone())),
            false,
            0,
        )),
        "with" => Some(function_type(
            vec![Type::Number, element.clone()],
            Type::Array(Box::new(element.clone())),
            false,
            2,
        )),
        // `fill(value: T, start?: number, end?: number)` — positional, not
        // variadic; modelling it as variadic checked `start`/`end` against `T`.
        "fill" => Some(function_type(
            vec![element.clone(), Type::Number, Type::Number],
            Type::Array(Box::new(element.clone())),
            false,
            1,
        )),
        // `splice(start: number, deleteCount?: number, ...items: T[])`.
        "splice" | "toSpliced" => Some(function_type(
            vec![
                Type::Number,
                Type::Number,
                Type::Array(Box::new(element.clone())),
            ],
            Type::Array(Box::new(element.clone())),
            true,
            1,
        )),
        // `push`/`unshift` take `...items: T[]`: the rest parameter is stored as
        // the array it is written as, so an element type that is itself an
        // array or tuple (`string[][]`, `[string, string][]`) is not unwrapped
        // a second time by variadic argument checking.
        "push" | "unshift" => Some(function_type(
            vec![Type::Array(Box::new(element.clone()))],
            Type::Number,
            true,
            0,
        )),
        "pop" | "shift" => Some(function_type(
            vec![],
            element_or_undefined(element),
            false,
            0,
        )),
        "at" => Some(function_type(
            vec![Type::Number],
            element_or_undefined(element),
            false,
            1,
        )),
        "indexOf" | "lastIndexOf" => Some(function_type(
            vec![element.clone(), Type::Number],
            Type::Number,
            false,
            1,
        )),
        "includes" => Some(function_type(
            vec![element.clone(), Type::Number],
            Type::Boolean,
            false,
            1,
        )),
        // Iterator-producing methods. The result carries the yielded element so
        // `for…of arr.values()` / `.entries()` / `.keys()` derive the loop
        // variable type instead of degrading to `unknown`.
        "values" => Some(function_type(
            vec![],
            array_iterator_type(element.clone()),
            false,
            0,
        )),
        "keys" => Some(function_type(
            vec![],
            array_iterator_type(Type::Number),
            false,
            0,
        )),
        "entries" => Some(function_type(
            vec![],
            array_iterator_type(Type::tuple(vec![Type::Number, element.clone()])),
            false,
            0,
        )),
        _ => None,
    }
}

/// An element an alias names prints bare (`Route[]`, `Fn[]`); a written
/// union, signature or intersection is parenthesized (`(A & B)[]`).
fn array_element_name(element: &Type) -> String {
    match element {
        Type::Union(_) => {
            let name = element.name();
            if crate::tsc_display::has_top_level(&name, '|') { format!("({name})") } else { name }
        }
        _ => crate::tsc_display::constituent_name(element),
    }
}

/// A property key as tsc's `createPropertyNameNodeForIdentifierOrLiteral`
/// writes it: an identifier or a canonical non-negative number as it is, any
/// other text as a string literal (`"data-foo"`). Keys surge spells itself —
/// computed (`[Symbol.iterator]`), private (`#x`), internal — stay as they are.
/// `createPropertyNameNodeForIdentifierOrLiteral`: an identifier prints bare
/// however it was written; any other name declared as a string literal keeps
/// that literal's quotes.
fn written_property_key_display<'a>(name: &'a str, written: Option<&str>) -> std::borrow::Cow<'a, str> {
    let name = crate::private_name::display(name);
    let is_identifier = name.chars().next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && name.chars().all(|character| character.is_alphanumeric() || character == '_' || character == '$');
    match written {
        Some(written) if !is_identifier && written.starts_with('\'') => std::borrow::Cow::Owned(format!("'{name}'")),
        Some(written) if !is_identifier && written.starts_with('"') => std::borrow::Cow::Owned(format!("{name:?}")),
        _ => property_key_display(name),
    }
}

fn property_key_display(name: &str) -> std::borrow::Cow<'_, str> {
    let is_identifier = name.chars().next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && name.chars().all(|character| character.is_alphanumeric() || character == '_' || character == '$');
    let is_numeric = is_canonical_number_text(name);
    let spelled_by_surge = name.starts_with(['[', '#', '@']) || name.contains('\0') || name.is_empty();
    if is_identifier || is_numeric || spelled_by_surge {
        return std::borrow::Cow::Borrowed(name);
    }
    std::borrow::Cow::Owned(format!("{name:?}"))
}

/// `isNumericLiteralName`: `(+name).toString() === name`, for the decimal and
/// exponent forms a JavaScript number prints in (`26`, `1.5`, `9.67e+24`).
fn is_canonical_number_text(name: &str) -> bool {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let decimal = |text: &str| {
        let (integer, fraction) = text.split_once('.').map_or((text, None), |(integer, fraction)| (integer, Some(fraction)));
        digits(integer)
            && (integer == "0" || !integer.starts_with('0'))
            && fraction.is_none_or(|fraction| digits(fraction) && !fraction.ends_with('0'))
    };
    match name.split_once('e') {
        Some((mantissa, exponent)) => {
            let exponent = exponent.strip_prefix(['+', '-']);
            decimal(mantissa)
                && !mantissa.starts_with('0')
                && mantissa.split('.').next().is_some_and(|integer| integer.len() == 1)
                && exponent.is_some_and(|exponent| digits(exponent) && !exponent.starts_with('0'))
        }
        None => decimal(name) && name.split('.').next().is_some_and(|integer| integer.len() <= 21),
    }
}

/// Renders the type of an optional property as tsc does in diagnostics: under
/// `strictNullChecks` and outside `exactOptionalPropertyTypes`, an optional
/// property's printed type gains `| undefined`. `unknown`/`any` already absorb `undefined`, and a union that
/// already includes it needs no addition.
fn optional_property_display(ty: &Type) -> String {
    // tsc prints a member's `getNonMissingTypeOfSymbol`, which keeps the
    // declared type as it is under `exactOptionalPropertyTypes`.
    if crate::exact_optional_property_types() || !crate::strict_null_checks() {
        return ty.name();
    }
    match ty {
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) | Type::Undefined => ty.name(),
        // A member can be a reference that resolves to `undefined` (a
        // distributed conditional's branch).
        Type::Union(union) if union.types().iter().any(|m| matches!(m.peeled(), Type::Undefined)) => {
            ty.name()
        }
        _ => crate::tsc_display::with_undefined(ty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectProperty;

    #[test]
    fn a_property_key_is_quoted_unless_an_identifier_or_a_canonical_number() {
        for bare in ["plain", "$x", "_y", "10", "0", "1.5", "9.671406556917009e+24", "1e-7", "[Symbol.iterator]", "#private"] {
            assert_eq!(property_key_display(bare), bare, "{bare}");
        }
        for quoted in ["data-foo", "ns:attr", "literal property", "1.", "1.0", "01", "1e+07"] {
            assert_eq!(property_key_display(quoted), format!("{quoted:?}"), "{quoted}");
        }
    }

    #[test]
    fn string_literal_type_name_quotes_value() {
        assert_eq!(Type::StringLiteral("ok".to_string()).name(), r#""ok""#);
    }

    // `toString`/`toLocaleString` come from `Object.prototype` and every array
    // answers them; leaving them out of the surface reported TS2339.
    #[test]
    fn array_answers_the_object_string_conversions() {
        for name in ["toString", "toLocaleString"] {
            assert!(array_property_names().contains(&name), "{name}");
            let member = array_member_type(name, &Type::String).expect(name);
            let Type::Function(function_type) = member else {
                panic!("{name} must be callable");
            };
            assert_eq!(function_type.return_type(), &Type::String, "{name}");
        }
        // `toLocaleString` carries lib.es2015.core's `(locales, options?)`.
        let Some(Type::Function(to_string)) = array_member_type("toString", &Type::String) else {
            panic!("toString must be callable");
        };
        assert!(to_string.parameters().is_empty());
    }

    #[test]
    fn number_literal_type_name_is_stable() {
        assert_eq!(
            Type::NumberLiteral(NumberLiteralType {
                value: "1".to_string()
            })
            .name(),
            "1"
        );
    }

    #[test]
    fn boolean_literal_type_name_is_true_false() {
        assert_eq!(Type::BooleanLiteral(true).name(), "true");
        assert_eq!(Type::BooleanLiteral(false).name(), "false");
    }

    #[test]
    fn void_type_name_is_void() {
        assert_eq!(Type::Void.name(), "void");
    }

    #[test]
    fn array_type_name_string() {
        assert_eq!(Type::Array(Box::new(Type::String)).name(), "string[]");
    }

    #[test]
    fn array_length_property_is_number() {
        assert_eq!(
            Type::Array(Box::new(Type::String)).get_property_access_type("length"),
            Some(Type::Number)
        );
    }

    #[test]
    fn tuple_length_property_is_its_arity() {
        assert_eq!(
            Type::tuple(vec![Type::String, Type::Number]).get_property_access_type("length"),
            Some(number_literal(2))
        );
        assert_eq!(
            Type::tuple(vec![]).get_property_access_type("length"),
            Some(number_literal(0))
        );
    }

    #[test]
    fn tuple_length_counts_trailing_optional_elements_as_absent() {
        let optional = crate::union_type(vec![Type::Number, Type::Undefined]);
        assert_eq!(
            Type::tuple(vec![Type::String, optional]).get_property_access_type("length"),
            Some(crate::union_type(vec![number_literal(1), number_literal(2)]))
        );
    }

    #[test]
    fn tuple_elements_are_properties_by_position() {
        let tuple = Type::tuple(vec![Type::String, Type::Number]);
        assert_eq!(tuple.get_property_access_type("0"), Some(Type::String));
        assert_eq!(tuple.get_property_access_type("1"), Some(Type::Number));
        assert_eq!(tuple.get_property_access_type("2"), None);
        assert_eq!(tuple.get_property_access_type("01"), None);
    }

    #[test]
    fn open_tuple_names_only_its_leading_elements() {
        let tuple = Type::OpenTuple(OpenTupleType {
            leading: vec![Type::String],
            rest: Box::new(Type::Number),
            trailing: vec![Type::Boolean],
            labels: None,
        });
        assert_eq!(tuple.get_property_access_type("0"), Some(Type::String));
        assert_eq!(tuple.get_property_access_type("1"), None);
        assert_eq!(tuple.get_property_access_type("length"), Some(Type::Number));
    }

    #[test]
    fn written_tuple_records_what_its_element_types_cannot_show() {
        let optional = crate::union_type(vec![Type::Number, Type::Undefined]);

        let optional_any = written_tuple_type(vec![Type::Any], 0);
        assert!(is_written_tuple(&optional_any));
        assert_eq!(optional_any.name(), "[any?]");
        assert_eq!(
            optional_any.get_property_access_type("length"),
            Some(crate::union_type(vec![number_literal(0), number_literal(1)]))
        );
        assert_eq!(optional_any.get_property_access_type("0"), Some(Type::Any));

        let required = written_tuple_type(vec![Type::String, optional.clone()], 2);
        assert!(is_written_tuple(&required));
        assert_eq!(required.get_property_access_type("length"), Some(number_literal(2)));

        let plain = written_tuple_type(vec![Type::String, optional.clone()], 1);
        assert!(matches!(plain, Type::Tuple(_)));
        assert_eq!(plain.name(), "[string, (number | undefined)?]");
        // A tuple built from element types alone flags nothing.
        assert_eq!(Type::tuple(vec![Type::String, optional]).name(), "[string, number | undefined]");
    }

    #[test]
    fn tuple_labels_print_as_named_members() {
        let optional = crate::union_type(vec![Type::Number, Type::Undefined]);
        let labels = Some(vec![Some("a".into()), Some("b".into())]);
        let named = written_labeled_tuple_type(vec![Type::String, optional.clone()], 1, labels.clone());
        assert_eq!(named.name(), "[a: string, b?: number | undefined]");
        assert_eq!(
            named.get_property_access_type("length"),
            Some(crate::union_type(vec![number_literal(1), number_literal(2)]))
        );
        let unlabeled = written_tuple_type(vec![Type::String, optional], 1);
        assert_ne!(named, unlabeled);

        let open = Type::OpenTuple(OpenTupleType {
            leading: vec![Type::String],
            rest: Box::new(Type::Number),
            trailing: Vec::new(),
            labels: crate::open_tuple_labels(vec![Some("head".into()), Some("tail".into())]),
        });
        assert_eq!(open.name(), "[head: string, ...tail: number[]]");
    }

    #[test]
    fn tuple_unknown_property_is_unsupported() {
        assert_eq!(
            Type::tuple(vec![Type::String, Type::Number])
                .get_property_access_type("definitelyNotAnArrayMember"),
            None
        );
    }

    #[test]
    fn tuple_exposes_array_methods_over_element_union() {
        let tuple = Type::tuple(vec![Type::String, Type::Number]);
        // A tuple is an array, so array methods resolve over the element union.
        assert!(tuple.get_property_access_type("includes").is_some());
        assert!(tuple.get_property_access_type("map").is_some());
    }

    #[test]
    fn array_type_name_number() {
        assert_eq!(Type::Array(Box::new(Type::Number)).name(), "number[]");
    }

    #[test]
    fn array_type_name_boolean() {
        assert_eq!(Type::Array(Box::new(Type::Boolean)).name(), "boolean[]");
    }

    #[test]
    fn array_type_name_undefined() {
        assert_eq!(Type::Array(Box::new(Type::Undefined)).name(), "undefined[]");
    }

    #[test]
    fn array_type_name_void() {
        assert_eq!(Type::Array(Box::new(Type::Void)).name(), "void[]");
    }

    #[test]
    fn array_type_name_literal() {
        assert_eq!(
            Type::Array(Box::new(Type::StringLiteral("ok".to_string()))).name(),
            r#""ok"[]"#
        );
    }

    #[test]
    fn array_type_name_union() {
        assert_eq!(
            Type::Array(Box::new(Type::Union(crate::UnionType::new(vec![
                Type::String,
                Type::Number,
            ]))))
            .name(),
            "(string | number)[]"
        );
    }

    #[test]
    fn array_type_name_function() {
        assert_eq!(
            Type::Array(Box::new(Type::Function(FunctionType::new(
                vec![],
                Type::String,
                false,
                0,
            ))))
            .name(),
            "(() => string)[]"
        );
    }

    #[test]
    fn array_type_name_object() {
        let mut properties = crate::PropertyMap::default();
        properties.insert("name".into(), ObjectProperty::required(Type::String));

        assert_eq!(
            Type::Array(Box::new(Type::Object(ObjectType::new(properties, None)))).name(),
            "{ name: string; }[]"
        );
    }

    #[test]
    fn array_type_name_nested_array() {
        assert_eq!(
            Type::Array(Box::new(Type::Array(Box::new(Type::String)))).name(),
            "string[][]"
        );
    }

    #[test]
    fn tuple_type_name_empty() {
        assert_eq!(Type::tuple(vec![]).name(), "[]");
    }

    #[test]
    fn tuple_type_name_one_element() {
        assert_eq!(Type::tuple(vec![Type::String]).name(), "[string]");
    }

    #[test]
    fn tuple_type_name_two_elements() {
        assert_eq!(
            Type::tuple(vec![Type::String, Type::Number]).name(),
            "[string, number]"
        );
    }

    #[test]
    fn tuple_type_name_literal_element() {
        assert_eq!(
            Type::tuple(vec![Type::StringLiteral("ok".to_string()), Type::Number]).name(),
            r#"["ok", number]"#
        );
    }

    #[test]
    fn tuple_type_name_union_element() {
        assert_eq!(
            Type::tuple(vec![
                Type::Union(crate::UnionType::new(vec![Type::String, Type::Number,])),
                Type::Boolean,
            ])
            .name(),
            "[string | number, boolean]"
        );
    }

    #[test]
    fn tuple_type_name_function_element() {
        assert_eq!(
            Type::tuple(vec![
                Type::Function(FunctionType::new(vec![], Type::Void, false, 0)),
                Type::String,
            ])
            .name(),
            "[() => void, string]"
        );
    }

    #[test]
    fn tuple_type_name_object_element() {
        let mut properties = crate::PropertyMap::default();
        properties.insert("name".into(), ObjectProperty::required(Type::String));

        assert_eq!(
            Type::tuple(vec![
                Type::Object(ObjectType::new(properties, None)),
                Type::Number,
            ])
            .name(),
            "[{ name: string; }, number]"
        );
    }

    #[test]
    fn tuple_type_name_array_element() {
        assert_eq!(
            Type::tuple(vec![Type::Array(Box::new(Type::String)), Type::Number]).name(),
            "[string[], number]"
        );
    }

    #[test]
    fn tuple_type_name_nested_tuple() {
        assert_eq!(
            Type::tuple(vec![
                Type::tuple(vec![Type::String, Type::Number]),
                Type::Boolean,
            ])
            .name(),
            "[[string, number], boolean]"
        );
    }

    fn iterator_element(method_result: &Type) -> Type {
        let Type::Function(function) = method_result else {
            panic!("expected a function-typed array method, got {method_result:?}");
        };
        let Type::Reference(reference) = function.return_type() else {
            panic!("expected an iterator reference return type");
        };
        assert_eq!(reference.id.as_ref(), "\u{0}ArrayIterator");
        reference.arguments[0].clone()
    }

    #[test]
    fn array_values_iterator_yields_element() {
        let result = array_property_access_type("values", &Type::Number).unwrap();
        assert_eq!(iterator_element(&result), Type::Number);
    }

    #[test]
    fn array_keys_iterator_yields_number() {
        let result = array_property_access_type("keys", &Type::String).unwrap();
        assert_eq!(iterator_element(&result), Type::Number);
    }

    #[test]
    fn array_entries_iterator_yields_index_value_tuple() {
        let result = array_property_access_type("entries", &Type::String).unwrap();
        assert_eq!(
            iterator_element(&result),
            Type::tuple(vec![Type::Number, Type::String])
        );
    }

    #[test]
    fn array_iterator_next_value_is_element_or_undefined() {
        let result = array_property_access_type("values", &Type::Number).unwrap();
        let Type::Function(function) = &result else {
            panic!("expected function");
        };
        let next = function
            .return_type()
            .get_property_access_type("next")
            .expect("iterator exposes next()");
        let Type::Function(next_fn) = next else {
            panic!("next is callable");
        };
        let value = next_fn
            .return_type()
            .get_property_access_type("value")
            .expect("iterator result has value");
        assert_eq!(
            value,
            crate::union_type(vec![Type::Number, Type::Undefined])
        );
    }
}

/// Whether `ty` is the global `Function` interface. tsc treats it as callable
/// with any arguments (yielding `any`) and constructible, even though
/// `lib.d.ts` declares no call signature on it — so every consumer that asks
/// "is this callable?" has to special-case it. The `apply`/`call`/`bind` check
/// pins this to the real lib interface rather than any same-named user type.
pub fn is_global_function_interface(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    if reference.id.split('\u{0}').next_back() != Some("Function") {
        return false;
    }
    match ty.peeled() {
        Type::Object(object) => {
            object.call_signature().is_none()
                && ["apply", "call", "bind"]
                    .iter()
                    .all(|member| object.properties.get(*member).is_some())
        }
        _ => false,
    }
}

/// tsc's `isUntypedFunctionCall` for a callee that is not `any`: no call or
/// construct signature, not a union, and assignable to the global `Function`
/// interface, which surge reads off `Function`'s own `apply`, `call` and
/// `bind` as [`is_global_function_interface`] does. Such a call is untyped.
pub fn is_untyped_function_callee(ty: &Type) -> bool {
    if is_global_function_interface(ty) {
        return true;
    }
    match ty.peeled() {
        Type::Object(object) => {
            object.call_signature().is_none()
                && object.construct_signature().is_none()
                && ["apply", "call", "bind"]
                    .iter()
                    .all(|member| object.properties.get(*member).is_some())
        }
        _ => false,
    }
}

/// A member of a type variable narrowed to `T & X` (see
/// `type_variable::intersect_type_variable`): the variable's constraint and the
/// other operands answer it, as the intersection's apparent members do in tsc.
fn narrowed_type_variable_member(object: &crate::ObjectType, name: &str) -> Option<Type> {
    if !object.is_intersection || !object.properties.is_empty() {
        return None;
    }
    let operands = object.intersection_operands.as_deref()?;
    // The member tsc reads is every operand's member intersected; the type a
    // guard narrowed to is the one that refines the variable's constraint, so
    // it answers first.
    operands
        .iter()
        .filter(|operand| !matches!(operand, Type::TypeParameter(_)))
        .find_map(|operand| operand.get_property_access_type(name))
        .or_else(|| {
            // An intersection is never nullish: `T & {}` (`NonNullable<T>`)
            // reads `T`'s constraint without its nullish members.
            operands.iter().find_map(|operand| match operand {
                Type::TypeParameter(parameter) => {
                    crate::remove_nullish(&crate::type_variable::active_constraint(parameter)??)
                        .get_property_access_type(name)
                }
                _ => None,
            })
        })
}
