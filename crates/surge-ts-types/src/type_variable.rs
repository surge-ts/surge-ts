//! Type variables of the generic body currently being checked.
//!
//! tsc gives every type parameter an identity and a constraint, and relates a
//! type variable through that constraint (`structuredTypeRelatedToWorker` in
//! relater.go). surge's [`Type::TypeParameter`] placeholder instead stands in
//! for anything it could not instantiate, so it relates like the degradation
//! sentinel. A body's *own* parameters are the exception: while that body is
//! checked they are real variables, bound here with their constraints and
//! marked by a nonzero [`TypeParameterType::owner`]. Once the body is done the
//! entry is gone and any variable that escaped relates like a placeholder
//! again — the constraint lives only as long as the scope that declares it, so
//! no type holds its own constraint alive.
//!
//! The deferred types tsc builds over those variables — `T[K]` and `keyof T`
//! ([`DeferredType`]) — are bound here too, under an owner of their own, for
//! as long as every variable they are built from.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::{Type, TypeParameterType};

struct ActiveVariable {
    owner: u32,
    name: Arc<str>,
    declaration: (Arc<str>, u32),
    constraint: Option<Type>,
    deferred: Option<Deferred>,
}

struct Deferred {
    kind: DeferredType,
    /// The declared variables the type is built from, by owner and name.
    bases: Vec<(u32, Arc<str>)>,
}

/// A generic type tsc leaves unresolved while one of its operands is a type
/// variable: `T[K]` (`shouldDeferIndexedAccessType`) and `keyof T`
/// (`shouldDeferIndexType`). It is a type variable of its own, related through
/// the rules relater.go gives `IndexedAccess` and `Index` types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeferredType {
    IndexedAccess { object: Type, index: Type },
    Keyof(Type),
    /// A generic mapped type (`isGenericMappedType`) in the one template shape
    /// surge defers, `{ [P in keys]: object[P] }`. It is an object type in
    /// tsc, not a type variable: it has no constraint surge computes, and
    /// relates only through the mapped-type arms of relater.go.
    Mapped {
        keys: Type,
        object: Type,
        modifiers: MappedModifiers,
        /// `getCombinedMappedTypeOptionality` of the modifiers type (the `T`
        /// of `keyof T`), which a mapping that leaves `?` alone inherits.
        modifiers_optionality: i8,
    },
    /// A generic mapped type whose template does not read its key,
    /// `{ [P in keys]: template }` (`Record<K, T>` over a variable `K`). An
    /// object type like [`DeferredType::Mapped`].
    MappedConstant {
        keys: Type,
        template: Type,
        modifiers: MappedModifiers,
    },
    /// The key type parameter `P` of a generic mapped type
    /// `{ [P in keys]: … }`, constrained to `keys` and identified by its
    /// declaration.
    MappedKey { keys: Type, declaration: (Arc<str>, u32) },
    /// A generic mapped type in its general shape: a template over its key
    /// parameter, and an optional `as` clause. An object type like
    /// [`DeferredType::Mapped`].
    MappedGeneric(Box<DeferredMapped>),
    /// A conditional type tsc defers because its check or extends type is
    /// generic (`getConditionalType`). `true_type` reads the check type
    /// through the extends type where it names it
    /// (`getConditionalFlowTypeOfType`).
    Conditional(Box<DeferredConditional>),
    /// A template literal type with a type variable placeholder
    /// (`getTemplateLiteralType` keeps `isGenericIndexType` placeholders).
    TemplateLiteral { texts: Vec<String>, types: Vec<Type> },
    /// `Uppercase<T>` and its siblings over a type variable
    /// (`getStringMappingTypeForGenericType`).
    StringMapping { kind: crate::StringMappingKind, operand: Type },
    /// A tuple type with a variadic element over a type variable
    /// (`isGenericTupleType`), `[string, ...T]`. A `Rest` element holds its
    /// element type, a `Variadic` one the variable. An object type in tsc,
    /// related through the tuple arms of relater.go.
    Tuple {
        elements: Vec<(TupleElementKind, Type)>,
        readonly: bool,
    },
}

/// A tuple element's `ElementFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TupleElementKind {
    Required,
    Optional,
    Rest,
    Variadic,
}

/// The parts of a generic mapped type (see [`DeferredType::MappedGeneric`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredMapped {
    /// The key parameter, a [`DeferredType::MappedKey`] variable.
    pub key: Type,
    pub keys: Type,
    pub name_type: Option<Type>,
    /// The template as written, without the `undefined` a `?` adds.
    pub template: Type,
    pub modifiers: MappedModifiers,
    /// `getCombinedMappedTypeOptionality` of the modifiers type.
    pub modifiers_optionality: i8,
}

/// The parts of a deferred conditional type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredConditional {
    pub check: Type,
    pub extends: Type,
    pub true_type: Type,
    pub false_type: Type,
    /// `root.isDistributive`: the written check type is a type parameter.
    pub distributive: bool,
    /// `isDistributionDependent`: distributive, and a branch names the check
    /// type parameter.
    pub distribution_dependent: bool,
    /// The extends type declares `infer` type parameters.
    pub has_infer: bool,
    /// `getConstraintOfDistributiveConditionalType`: the conditional over the
    /// check type's constraint, when that is not `never`.
    pub distributive_constraint: Option<Type>,
}

/// A mapped type's `readonly` and `?` modifiers (`getMappedTypeModifiers`):
/// `1` adds one, `-1` removes it, `0` leaves the source's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappedModifiers {
    pub readonly: i8,
    pub optional: i8,
}

/// What a target-side rule of a deferred type admits: the types a source has
/// to relate to, nothing (`Absent`), or a question surge cannot answer.
pub enum TargetConstraint {
    Types(Vec<Type>),
    Absent,
    Unmodelled,
}

thread_local! {
    static ACTIVE_VARIABLES: RefCell<Vec<ActiveVariable>> = const { RefCell::new(Vec::new()) };
    static NEXT_OWNER: Cell<u32> = const { Cell::new(1) };
    static CONSTRAINT_DEPTH: Cell<u32> = const { Cell::new(0) };
}

fn next_owner() -> u32 {
    NEXT_OWNER.with(|next| {
        let owner = next.get();
        next.set(owner.wrapping_add(1).max(1));
        owner
    })
}

/// The variables one generic body binds. Dropping it unbinds them.
pub struct TypeVariableScope {
    owner: u32,
}

impl TypeVariableScope {
    /// Binds `names` (each with its declaration identity: file and name offset)
    /// as variables of a fresh owner. Constraints are set afterwards with
    /// [`TypeVariableScope::set_constraint`], because resolving one may name the
    /// variables themselves (`U extends T`, `T extends Foo<T>`).
    pub fn enter(parameters: impl IntoIterator<Item = (Arc<str>, (Arc<str>, u32))>) -> Self {
        let owner = next_owner();
        ACTIVE_VARIABLES.with(|active| {
            active.borrow_mut().extend(parameters.into_iter().map(|(name, declaration)| {
                ActiveVariable {
                    owner,
                    name,
                    declaration,
                    constraint: None,
                    deferred: None,
                }
            }))
        });
        Self { owner }
    }

    pub fn set_constraint(&self, name: &str, constraint: Type) {
        ACTIVE_VARIABLES.with(|active| {
            if let Some(variable) = active
                .borrow_mut()
                .iter_mut()
                .find(|variable| variable.owner == self.owner && *variable.name == *name)
            {
                variable.constraint = Some(constraint);
            }
        });
    }

    /// Unbinds `name`: a constraint surge cannot model says nothing about what
    /// the variable admits, so it relates like a placeholder again, and a
    /// variable already built for it turns permissive with it.
    pub fn forget(&self, name: &str) {
        ACTIVE_VARIABLES.with(|active| {
            active.borrow_mut().retain(|variable| {
                !(variable.owner == self.owner && *variable.name == *name)
                    && !builds_on(variable, |owner, base| owner == self.owner && *base == *name)
            })
        });
    }

    pub fn variable(&self, name: &str) -> Type {
        Type::TypeParameter(TypeParameterType {
            name: Arc::from(name),
            owner: self.owner,
        })
    }
}

impl Drop for TypeVariableScope {
    fn drop(&mut self) {
        ACTIVE_VARIABLES.with(|active| {
            active
                .borrow_mut()
                .retain(|variable| variable.owner != self.owner && !builds_on(variable, |owner, _| owner == self.owner))
        });
    }
}

fn builds_on(variable: &ActiveVariable, base: impl Fn(u32, &str) -> bool) -> bool {
    variable
        .deferred
        .as_ref()
        .is_some_and(|deferred| deferred.bases.iter().any(|(owner, name)| base(*owner, name)))
}

/// The active variable a type-parameter *declaration* is bound to, if its
/// body is being checked.
pub fn variable_for_declaration(file: &str, name_offset: u32) -> Option<Type> {
    ACTIVE_VARIABLES.with(|active| {
        active
            .borrow()
            .iter()
            .rev()
            .find(|variable| {
                variable.deferred.is_none() && *variable.declaration.0 == *file && variable.declaration.1 == name_offset
            })
            .map(|variable| {
                Type::TypeParameter(TypeParameterType {
                    name: variable.name.clone(),
                    owner: variable.owner,
                })
            })
    })
}

/// `Some(constraint)` when `parameter` is a variable whose body is being
/// checked (`None` inside for an unconstrained one); `None` for a placeholder.
/// A deferred type answers its base constraint, or `None` when surge cannot
/// compute it.
pub fn active_constraint(parameter: &TypeParameterType) -> Option<Option<Type>> {
    if parameter.owner == 0 {
        return None;
    }
    let (constraint, deferred) = ACTIVE_VARIABLES.with(|active| {
        active
            .borrow()
            .iter()
            .find(|variable| variable.owner == parameter.owner && *variable.name == *parameter.name)
            .map(|variable| {
                (
                    variable.constraint.clone(),
                    variable.deferred.as_ref().map(|deferred| deferred.kind.clone()),
                )
            })
    })?;
    match deferred {
        None => Some(constraint),
        Some(kind) => deferred_constraint(&kind),
    }
}

/// What `parameter` is built from when it is a deferred type.
pub fn deferred_type(parameter: &TypeParameterType) -> Option<DeferredType> {
    if parameter.owner == 0 {
        return None;
    }
    ACTIVE_VARIABLES.with(|active| {
        active
            .borrow()
            .iter()
            .find(|variable| variable.owner == parameter.owner && *variable.name == *parameter.name)
            .and_then(|variable| variable.deferred.as_ref().map(|deferred| deferred.kind.clone()))
    })
}

/// tsc's deferred `object[index]` over a type variable `object`, or `None`
/// when an operand is not something the deferred type can be built from (a
/// placeholder, the sentinel, a key that is no key type).
pub fn indexed_access_variable(object: &Type, index: &Type) -> Option<Type> {
    // `getSimplifiedIndexedAccessType` over a generic mapped object:
    // `substituteIndexedMappedType` reads `{ [P in K]: X[P] }[I]` as `X[I]`,
    // optional when the mapping adds `?` or its modifiers type does.
    if let Some(DeferredType::Mapped {
        object: template_object,
        modifiers,
        modifiers_optionality,
        ..
    }) = mapped_type(object)
    {
        let substituted = indexed_access_variable(&template_object, index)?;
        let optional = modifiers.optional > 0 || modifiers_optionality > 0;
        return Some(if optional && crate::strict_null_checks() {
            crate::union_type(vec![substituted, Type::Undefined])
        } else {
            substituted
        });
    }
    // `getSimplifiedIndexedAccessType` substitutes into a generic mapped type
    // unless its `as` clause remaps keys (`getMappedTypeNameTypeKind`: a
    // clause whose names are all keys only filters them).
    if let Some(mapped) = mapped_generic_type(object)
        && mapped
            .name_type
            .as_ref()
            .is_none_or(|name_type| crate::is_assignable_to(name_type, &mapped.key))
    {
        return Some(mapped_generic_template(&mapped, index));
    }
    if let Some(DeferredType::MappedConstant { template, modifiers, .. }) = mapped_constant_type(object) {
        return Some(if modifiers.optional > 0 && crate::strict_null_checks() {
            crate::union_type(vec![template, Type::Undefined])
        } else {
            template
        });
    }
    // `getIndexedAccessTypeOrUndefined`: an object with nothing but a string
    // index signature reads that signature at any string or number key.
    if let Type::Object(object_type) = object.peeled()
        && object_type.properties.is_empty()
        && object_type.number_index_type.is_none()
        && object_type.call_signature().is_none()
        && object_type.construct_signature().is_none()
        && !object_type.synthetic_open_index
        && !object_type.is_intersection
        && let Some(value) = object_type.string_index_type.as_deref()
        && crate::is_assignable_to(index, &crate::union_type(vec![Type::String, Type::Number]))
    {
        return Some(value.clone());
    }
    // A generic key over an object that is not generic defers too
    // (`shouldDeferIndexedAccessType`, `isGenericIndexType`).
    let mut bases = if matches!(object, Type::TypeParameter(_)) {
        variable_bases(object)?
    } else if object.is_unknown() || matches!(object, Type::Any) {
        return None;
    } else if is_generic_index_type(object) {
        mentioned_bases(object)
    } else {
        Vec::new()
    };
    bases.extend(index_bases(index)?);
    let object_name = object.name();
    let object_name = if object_name.starts_with("keyof ") {
        format!("({object_name})")
    } else {
        object_name
    };
    deferred_variable(
        DeferredType::IndexedAccess {
            object: object.clone(),
            index: index.clone(),
        },
        format!("{object_name}[{}]", index.name()),
        bases,
    )
}

/// tsc's deferred `keyof operand` over a type variable `operand`. The keys of
/// a generic mapped type are not deferred: `getIndexTypeForMappedType`
/// answers its constraint.
pub fn keyof_variable(operand: &Type) -> Option<Type> {
    if let Some(DeferredType::Mapped { keys, .. } | DeferredType::MappedConstant { keys, .. }) =
        mapped_type(operand).or_else(|| mapped_constant_type(operand))
    {
        return Some(keys);
    }
    if let Some(mapped) = mapped_generic_type(operand)
        && mapped.name_type.is_none()
    {
        return Some(mapped.keys);
    }
    if let Some(bases) = variable_bases(operand) {
        return deferred_variable(DeferredType::Keyof(operand.clone()), format!("keyof {}", operand.name()), bases);
    }
    // `keyof (T & {})`: an intersection with a type variable is a generic
    // object type (`shouldDeferIndexType`), kept as the deferred `keyof` of
    // the whole intersection.
    let bases = intersection_variable_bases(operand)?;
    deferred_variable(DeferredType::Keyof(operand.clone()), format!("keyof ({})", operand.name()), bases)
}

/// The variables of an intersection of type variables with `{}` (`T & {}`).
/// Only that shape: a `keyof` over any other operand would have to read its
/// keys too, which the deferred `keyof` of the whole does not relate through.
fn intersection_variable_bases(ty: &Type) -> Option<Vec<(u32, Arc<str>)>> {
    let Type::Object(object) = ty else {
        return None;
    };
    if !object.is_intersection {
        return None;
    }
    let is_empty_object = |operand: &Type| {
        matches!(operand, Type::Object(empty) if empty.properties.is_empty()
            && empty.string_index_type.is_none()
            && empty.number_index_type.is_none()
            && empty.call_signature().is_none()
            && empty.construct_signature().is_none()
            && empty.intersection_operands.is_none())
    };
    let mut bases = Vec::new();
    let mut has_empty_object = false;
    for operand in object.intersection_operands.as_deref()? {
        match operand {
            Type::TypeParameter(_) => bases.extend(variable_bases(operand)?),
            other if is_empty_object(other) => has_empty_object = true,
            _ => return None,
        }
    }
    // Without an empty object operand the `keyof` distributes
    // (`keyof (T & U)` is `keyof T | keyof U`).
    (has_empty_object && !bases.is_empty()).then_some(bases)
}

/// tsc's generic mapped type `{ [P in keys]: object[P] }` over a generic key
/// set, deferred for as long as the variables it is built from. `None` when
/// `keys` or `object` is not something it can be built from.
pub fn mapped_variable(
    keys: &Type,
    object: &Type,
    modifiers: MappedModifiers,
    modifiers_type: Option<&Type>,
    name: String,
) -> Option<Type> {
    let mut bases = variable_bases(keys)?;
    bases.extend(variable_bases(object)?);
    let modifiers_optionality = modifiers_type.map_or(0, combined_mapped_optionality);
    deferred_variable(
        DeferredType::Mapped {
            keys: keys.clone(),
            object: object.clone(),
            modifiers,
            modifiers_optionality,
        },
        name,
        bases,
    )
}

/// tsc's `getExtractStringType(getIndexType(operand))` over a type variable:
/// `Extract<keyof operand, string>`, the key a `for…in` over it binds.
pub fn extract_string_keys(operand: &Type) -> Option<Type> {
    let keyof = keyof_variable(operand)?;
    let mut extract = DeferredConditional {
        check: keyof.clone(),
        extends: Type::String,
        true_type: keyof.clone(),
        false_type: Type::Never,
        distributive: true,
        distribution_dependent: true,
        has_infer: false,
        distributive_constraint: None,
    };
    extract.distributive_constraint =
        distributive_conditional_at(&extract, &key_constraint()).filter(|constraint| *constraint != Type::Never);
    conditional_variable(extract, format!("Extract<{}, string>", keyof.name()))
}

/// tsc's `Omit<source, keys>` over a type variable `source`, as `getRestType`
/// builds it: `Pick<source, Exclude<keyof source, keys>>`. `None` when it
/// cannot be built.
pub fn omit_variable(source: &Type, omitted: &[String]) -> Option<Type> {
    let keyof = keyof_variable(source)?;
    let omitted_keys = crate::union_type(omitted.iter().map(|key| Type::StringLiteral(key.clone())).collect());
    let mut exclude = DeferredConditional {
        check: keyof.clone(),
        extends: omitted_keys.clone(),
        true_type: Type::Never,
        false_type: keyof.clone(),
        distributive: true,
        distribution_dependent: true,
        has_infer: false,
        distributive_constraint: None,
    };
    exclude.distributive_constraint =
        distributive_conditional_at(&exclude, &key_constraint()).filter(|constraint| *constraint != Type::Never);
    let keys = conditional_variable(exclude, format!("Exclude<{}, {}>", keyof.name(), omitted_keys.name()))?;
    mapped_variable(
        &keys,
        source,
        MappedModifiers { readonly: 0, optional: 0 },
        Some(source),
        format!("Omit<{}, {}>", source.name(), omitted_keys.name()),
    )
}

/// The key parameter of a generic mapped type over `keys`.
pub fn mapped_key_variable(keys: &Type, declaration: (Arc<str>, u32), name: &str) -> Option<Type> {
    let mut bases = variable_bases(keys).unwrap_or_default();
    bases.extend(mentioned_bases(keys));
    deferred_variable(
        DeferredType::MappedKey {
            keys: keys.clone(),
            declaration,
        },
        name.to_string(),
        bases,
    )
}

/// A generic mapped type in its general shape.
pub fn mapped_generic_variable(mapped: DeferredMapped, name: String) -> Option<Type> {
    let mut bases = mentioned_bases(&mapped.keys);
    bases.extend(mentioned_bases(&mapped.template));
    if let Some(name_type) = &mapped.name_type {
        bases.extend(mentioned_bases(name_type));
    }
    deferred_variable(DeferredType::MappedGeneric(Box::new(mapped)), name, bases)
}

/// The parts of a generic mapped type in its general shape.
pub fn mapped_generic_type(ty: &Type) -> Option<DeferredMapped> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match deferred_type(parameter)? {
        DeferredType::MappedGeneric(mapped) => Some(*mapped),
        _ => None,
    }
}

/// `getTemplateTypeFromMappedType` of a generic mapped type at `key`: the
/// template with its key parameter replaced, holding `undefined` when the
/// mapping adds `?`.
pub fn mapped_generic_template(mapped: &DeferredMapped, key: &Type) -> Type {
    let template = match &mapped.key {
        Type::TypeParameter(parameter) if *key != mapped.key => substitute_variable(&mapped.template, parameter, key),
        _ => mapped.template.clone(),
    };
    if mapped.modifiers.optional > 0 && crate::strict_null_checks() {
        crate::union_type(vec![template, Type::Undefined])
    } else {
        template
    }
}

/// tsc's `instantiateType` with a mapper replacing the one variable `from`,
/// over what surge builds deferred types from. A type it cannot rebuild — a
/// reference whose arguments name `from` — is unmodelled.
pub fn substitute_variable(ty: &Type, from: &TypeParameterType, to: &Type) -> Type {
    fn go(ty: &Type, from: &TypeParameterType, to: &Type, depth: usize) -> Type {
        if depth > 16 || !mentions_variable(ty, from, 0) {
            return ty.clone();
        }
        let sub = |ty: &Type| go(ty, from, to, depth + 1);
        match ty {
            Type::TypeParameter(parameter) if parameter == from => to.clone(),
            Type::TypeParameter(parameter) => match deferred_type(parameter) {
                Some(DeferredType::IndexedAccess { object, index }) => {
                    let (object, index) = (sub(&object), sub(&index));
                    indexed_access_variable(&object, &index)
                        .or_else(|| match indexed_access_lookup(&object.peeled(), &index, false) {
                            Some(Some(types)) => Some(crate::union_type(types)),
                            _ => None,
                        })
                        .unwrap_or(Type::Unknown)
                }
                Some(DeferredType::Keyof(operand)) => {
                    let operand = sub(&operand);
                    keyof_variable(&operand).or_else(|| keys_of(&operand)).unwrap_or(Type::Unknown)
                }
                Some(DeferredType::TemplateLiteral { texts, types }) => {
                    let types: Vec<Type> = types.iter().map(sub).collect();
                    crate::template_literal_type(&texts, &types)
                }
                Some(DeferredType::StringMapping { kind, operand }) => crate::string_mapping_type(kind, &sub(&operand)),
                Some(DeferredType::Conditional(conditional)) => {
                    let name = parameter.name.to_string();
                    conditional_variable(
                        DeferredConditional {
                            check: sub(&conditional.check),
                            extends: sub(&conditional.extends),
                            true_type: sub(&conditional.true_type),
                            false_type: sub(&conditional.false_type),
                            distributive_constraint: conditional.distributive_constraint.as_ref().map(sub),
                            ..*conditional
                        },
                        name,
                    )
                    .unwrap_or(Type::Unknown)
                }
                _ => Type::Unknown,
            },
            Type::Union(union) => crate::union_type(union.types().iter().map(sub).collect()),
            Type::Array(element) => Type::Array(Box::new(sub(element))),
            Type::Tuple(elements) => Type::Tuple(elements.iter().map(sub).collect()),
            Type::Object(object) if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none() =>
            {
                match object.intersection_operands.as_deref() {
                    Some(operands) => type_variable_intersection(operands.iter().map(sub).collect()),
                    None => ty.clone(),
                }
            }
            _ => Type::Unknown,
        }
    }
    go(ty, from, to, 0)
}

/// Whether `ty` is built from the variable `from`, through the deferred types
/// it is made of.
fn mentions_variable(ty: &Type, from: &TypeParameterType, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    let nested = |ty: &Type| mentions_variable(ty, from, depth + 1);
    match ty {
        Type::TypeParameter(parameter) if parameter == from => true,
        Type::TypeParameter(parameter) => match deferred_type(parameter) {
            Some(DeferredType::IndexedAccess { object, index }) => nested(&object) || nested(&index),
            Some(DeferredType::Keyof(operand)) | Some(DeferredType::StringMapping { operand, .. }) => nested(&operand),
            Some(DeferredType::TemplateLiteral { types, .. }) => types.iter().any(nested),
            Some(DeferredType::Conditional(conditional)) => {
                nested(&conditional.check)
                    || nested(&conditional.extends)
                    || nested(&conditional.true_type)
                    || nested(&conditional.false_type)
            }
            Some(DeferredType::Mapped { keys, object, .. }) => nested(&keys) || nested(&object),
            Some(DeferredType::MappedConstant { keys, template, .. }) => nested(&keys) || nested(&template),
            Some(DeferredType::MappedKey { keys, .. }) => nested(&keys),
            Some(DeferredType::MappedGeneric(mapped)) => {
                nested(&mapped.keys) || nested(&mapped.template) || mapped.name_type.as_ref().is_some_and(nested)
            }
            Some(DeferredType::Tuple { elements, .. }) => elements.iter().any(|(_, ty)| nested(ty)),
            None => false,
        },
        Type::Union(union) => union.types().iter().any(nested),
        Type::Array(element) => nested(element),
        Type::Tuple(elements) => elements.iter().any(nested),
        Type::Function(function) => function.parameters().iter().any(nested) || nested(function.return_type()),
        Type::Object(object) => {
            object.properties.values().any(|property| nested(&property.ty))
                || object.string_index_type.as_deref().is_some_and(nested)
                || object.number_index_type.as_deref().is_some_and(nested)
                || object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(nested))
        }
        Type::Reference(reference) => reference.arguments.iter().any(nested),
        _ => false,
    }
}

/// A deferred conditional type, for as long as the variables it mentions.
pub fn conditional_variable(conditional: DeferredConditional, name: String) -> Option<Type> {
    let mut bases = mentioned_bases(&conditional.check);
    bases.extend(mentioned_bases(&conditional.extends));
    bases.extend(mentioned_bases(&conditional.true_type));
    bases.extend(mentioned_bases(&conditional.false_type));
    deferred_variable(DeferredType::Conditional(Box::new(conditional)), name, bases)
}

/// The parts of a deferred conditional type.
pub fn conditional_type(ty: &Type) -> Option<DeferredConditional> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match deferred_type(parameter)? {
        DeferredType::Conditional(conditional) => Some(*conditional),
        _ => None,
    }
}

/// `getDefaultConstraintOfConditionalType`: the union of the branches, or the
/// other branch when one is `any`. `None` when a branch is unmodelled.
pub fn default_conditional_constraint(conditional: &DeferredConditional) -> Option<Type> {
    let unmodelled = |ty: &Type| ty.is_unmodelled() && !matches!(ty, Type::GenuineUnknown | Type::ErrorType);
    if unmodelled(&conditional.true_type) || unmodelled(&conditional.false_type) {
        return None;
    }
    Some(match (&conditional.true_type, &conditional.false_type) {
        (Type::Any, false_type) => false_type.clone(),
        (true_type, Type::Any) => true_type.clone(),
        (true_type, false_type) => crate::union_type(vec![true_type.clone(), false_type.clone()]),
    })
}

/// tsc's generic mapped type `{ [P in keys]: template }` whose template does
/// not name `P`, deferred for as long as the variables it is built from.
pub fn mapped_constant_variable(keys: &Type, template: &Type, modifiers: MappedModifiers, name: String) -> Option<Type> {
    let mut bases = variable_bases(keys)?;
    bases.extend(mentioned_bases(template));
    deferred_variable(
        DeferredType::MappedConstant {
            keys: keys.clone(),
            template: template.clone(),
            modifiers,
        },
        name,
        bases,
    )
}

/// What `ty` is built from when it is a generic mapped type with a template
/// that does not read its key.
pub fn mapped_constant_type(ty: &Type) -> Option<DeferredType> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    deferred_type(parameter).filter(|kind| matches!(kind, DeferredType::MappedConstant { .. }))
}

/// The bases of every active variable `ty` mentions.
fn mentioned_bases(ty: &Type) -> Vec<(u32, Arc<str>)> {
    fn walk(ty: &Type, depth: usize, bases: &mut Vec<(u32, Arc<str>)>) {
        if depth > 8 {
            return;
        }
        match ty {
            Type::TypeParameter(_) => bases.extend(variable_bases(ty).unwrap_or_default()),
            Type::Array(element) => walk(element, depth + 1, bases),
            Type::Tuple(elements) => elements.iter().for_each(|element| walk(element, depth + 1, bases)),
            Type::Union(union) => union.types().iter().for_each(|member| walk(member, depth + 1, bases)),
            Type::Function(function) => {
                function.parameters().iter().for_each(|parameter| walk(parameter, depth + 1, bases));
                walk(function.return_type(), depth + 1, bases);
            }
            Type::Object(object) => {
                object.properties.values().for_each(|property| walk(&property.ty, depth + 1, bases));
                object.string_index_type.as_deref().into_iter().for_each(|index| walk(index, depth + 1, bases));
                object.number_index_type.as_deref().into_iter().for_each(|index| walk(index, depth + 1, bases));
                object
                    .intersection_operands
                    .as_deref()
                    .into_iter()
                    .flatten()
                    .for_each(|operand| walk(operand, depth + 1, bases));
            }
            Type::Reference(reference) => reference.arguments.iter().for_each(|argument| walk(argument, depth + 1, bases)),
            _ => {}
        }
    }
    let mut bases = Vec::new();
    walk(ty, 0, &mut bases);
    bases
}

/// The literal keys of a non-generic type without its index signatures
/// (`getIndexType` with `IndexFlags.NoIndexSignatures`), or `None` when surge
/// cannot enumerate them.
pub fn literal_keys_of(ty: &Type) -> Option<Type> {
    keys_of_with(ty, true)
}

/// What reading `object[index]` yields through the base constraint of a
/// generic `index` (the source side of an `IndexedAccess` type).
pub fn indexed_access_read_types(object: &Type, index: &Type) -> TargetConstraint {
    let Some(base_index) = base_constraint_or_self(index) else {
        return TargetConstraint::Unmodelled;
    };
    if is_generic(&base_index) || is_generic(object) {
        return TargetConstraint::Unmodelled;
    }
    match indexed_access_lookup(object, &base_index, false) {
        None => TargetConstraint::Unmodelled,
        Some(None) => TargetConstraint::Absent,
        Some(Some(types)) => TargetConstraint::Types(types),
    }
}

/// tsc's `isGenericTypeWithUnionConstraint` for a type variable of the body
/// being checked: its base constraint is a union or nullable. A reference of
/// such a type under a contextual type with no type variables is read through
/// that constraint narrowed by flow (`getNarrowableTypeForReference`).
pub fn is_generic_with_union_constraint(ty: &Type) -> bool {
    ty.is_type_variable()
        && matches!(
            base_constraint(ty),
            Some(Some(Type::Union(_) | Type::Null | Type::Undefined | Type::Void))
        )
}

/// `isGenericIndexType` for what surge builds: a type variable of the body
/// being checked, or an intersection holding one (`P & string`).
pub fn is_generic_index_type(ty: &Type) -> bool {
    ty.is_type_variable()
        || matches!(ty, Type::Object(object)
            if object.properties.is_empty()
                && object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(Type::is_type_variable)))
}

/// A template literal type whose placeholders include a type variable of the
/// body being checked. `None` when none of them is one.
pub fn template_literal_variable(texts: Vec<String>, types: Vec<Type>, name: String) -> Option<Type> {
    let mut bases = Vec::new();
    for ty in &types {
        if ty.is_type_variable() {
            bases.extend(variable_bases(ty)?);
        } else if is_generic_index_type(ty) {
            bases.extend(mentioned_bases(ty));
        }
    }
    deferred_variable(DeferredType::TemplateLiteral { texts, types }, name, bases)
}

/// A string mapping over a type variable of the body being checked.
pub fn string_mapping_variable(kind: crate::StringMappingKind, operand: &Type, name: String) -> Option<Type> {
    let bases = variable_bases(operand)?;
    deferred_variable(
        DeferredType::StringMapping {
            kind,
            operand: operand.clone(),
        },
        name,
        bases,
    )
}

/// The parts of a deferred template literal type.
pub fn template_literal_variable_parts(ty: &Type) -> Option<(Vec<String>, Vec<Type>)> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match deferred_type(parameter)? {
        DeferredType::TemplateLiteral { texts, types } => Some((texts, types)),
        _ => None,
    }
}

/// The parts of a deferred string mapping type.
pub fn string_mapping_variable_parts(ty: &Type) -> Option<(crate::StringMappingKind, Type)> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match deferred_type(parameter)? {
        DeferredType::StringMapping { kind, operand } => Some((kind, operand)),
        _ => None,
    }
}

/// What `ty` is built from when it is a generic mapped type.
pub fn mapped_type(ty: &Type) -> Option<DeferredType> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    deferred_type(parameter).filter(|kind| matches!(kind, DeferredType::Mapped { .. }))
}

/// `getCombinedMappedTypeOptionality`: the `?` a mapping adds (`1`) or
/// removes (`-1`), or, when it leaves `?` alone, its modifiers type's.
pub fn combined_mapped_optionality(ty: &Type) -> i8 {
    match mapped_type(ty) {
        Some(DeferredType::Mapped {
            modifiers,
            modifiers_optionality,
            ..
        }) => {
            if modifiers.optional != 0 {
                modifiers.optional
            } else {
                modifiers_optionality
            }
        }
        _ => 0,
    }
}

/// The modifiers type of a mapping over `keys` (`getModifiersTypeFromMappedType`):
/// the `T` of `keyof T`, directly or as the constraint of a key parameter.
pub fn mapped_modifiers_type(keys: &Type) -> Option<Type> {
    let Type::TypeParameter(parameter) = keys else {
        return None;
    };
    match deferred_type(parameter) {
        Some(DeferredType::Keyof(operand)) => Some(operand),
        Some(_) => None,
        None => match active_constraint(parameter)?? {
            Type::TypeParameter(constraint) => match deferred_type(&constraint)? {
                DeferredType::Keyof(operand) => Some(operand),
                _ => None,
            },
            _ => None,
        },
    }
}

fn deferred_variable(kind: DeferredType, name: String, bases: Vec<(u32, Arc<str>)>) -> Option<Type> {
    if bases.is_empty() {
        return None;
    }
    ACTIVE_VARIABLES.with(|active| {
        let mut active = active.borrow_mut();
        if let Some(existing) = active
            .iter()
            .find(|variable| variable.deferred.as_ref().is_some_and(|deferred| deferred.kind == kind))
        {
            return Some(Type::TypeParameter(TypeParameterType {
                name: existing.name.clone(),
                owner: existing.owner,
            }));
        }
        let owner = next_owner();
        let name: Arc<str> = Arc::from(name);
        active.push(ActiveVariable {
            owner,
            name: name.clone(),
            declaration: (Arc::from(""), 0),
            constraint: None,
            deferred: Some(Deferred { kind, bases }),
        });
        Some(Type::TypeParameter(TypeParameterType { name, owner }))
    })
}

/// The declared variables `ty` is built from, when it is an active variable.
fn variable_bases(ty: &Type) -> Option<Vec<(u32, Arc<str>)>> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    if parameter.owner == 0 {
        return None;
    }
    ACTIVE_VARIABLES.with(|active| {
        active
            .borrow()
            .iter()
            .find(|variable| variable.owner == parameter.owner && *variable.name == *parameter.name)
            .map(|variable| match &variable.deferred {
                Some(deferred) => deferred.bases.clone(),
                None => vec![(variable.owner, variable.name.clone())],
            })
    })
}

fn index_bases(index: &Type) -> Option<Vec<(u32, Arc<str>)>> {
    match index {
        Type::TypeParameter(_) => variable_bases(index),
        Type::String | Type::Number | Type::Symbol | Type::StringLiteral(_) | Type::NumberLiteral(_) => {
            Some(Vec::new())
        }
        Type::Union(union) => {
            let mut bases = Vec::new();
            for member in union.types() {
                bases.extend(index_bases(member)?);
            }
            Some(bases)
        }
        Type::Object(object) if object.properties.is_empty() => {
            let operands = object.intersection_operands.as_deref()?;
            let mut bases = Vec::new();
            for operand in operands {
                if matches!(operand, Type::TypeParameter(_)) {
                    bases.extend(variable_bases(operand)?);
                }
            }
            (!bases.is_empty()).then_some(bases)
        }
        _ => None,
    }
}

/// Bounds the constraint walks below: a chain of constraints naming one
/// another is circular (tsc's `circularConstraintType`).
struct ConstraintDepth;

impl ConstraintDepth {
    fn enter() -> Option<Self> {
        CONSTRAINT_DEPTH.with(|depth| {
            let current = depth.get();
            (current < 32).then(|| {
                depth.set(current + 1);
                ConstraintDepth
            })
        })
    }
}

impl Drop for ConstraintDepth {
    fn drop(&mut self) {
        CONSTRAINT_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// `keyofConstraintType`: what every `keyof T` is constrained to.
fn key_constraint() -> Type {
    crate::union_type(vec![Type::String, Type::Number, Type::Symbol])
}

fn deferred_constraint(kind: &DeferredType) -> Option<Option<Type>> {
    match kind {
        // `computeBaseConstraint` for an `Index` type: the names an `as`
        // clause gives the keys of a generic mapped type, and otherwise
        // `keyofConstraintType`.
        DeferredType::Keyof(operand) => match mapped_generic_type(operand) {
            Some(DeferredMapped {
                key: Type::TypeParameter(key),
                keys,
                name_type: Some(name_type),
                ..
            }) => Some(Some(substitute_variable(&name_type, &key, &keys))),
            _ => Some(Some(key_constraint())),
        },
        // `computeBaseConstraint` for an `IndexedAccess` type: the property
        // the key's base constraint selects out of the object's.
        DeferredType::IndexedAccess { object, index } => {
            // A generic mapped object is its own base constraint, and reading
            // it at a key's base constraint defers again: the access has no
            // constraint to relate through.
            if mapped_generic_type(object).is_some()
                || mapped_type(object).is_some()
                || mapped_constant_type(object).is_some()
            {
                return Some(None);
            }
            let _depth = ConstraintDepth::enter()?;
            let Some(object) = base_constraint(object)? else {
                return Some(None);
            };
            let Some(index) = base_constraint(index)? else {
                return Some(None);
            };
            if is_generic(&object) || is_generic(&index) {
                return None;
            }
            Some(indexed_access_lookup(&object, &index, false)?.map(crate::union_type))
        }
        // An object type, whose apparent members come from the modifiers
        // type's constraint (`resolveMappedTypeMembers`); surge does not
        // resolve them.
        DeferredType::Mapped { .. } | DeferredType::MappedConstant { .. } | DeferredType::MappedGeneric(_) => None,
        DeferredType::MappedKey { keys, .. } => Some(Some(keys.clone())),
        // `getBaseConstraintOfType` of a generic tuple: the tuple over its
        // variadic elements' constraints, a union constraint distributing
        // (`createNormalizedTupleType` maps a union element).
        DeferredType::Tuple { elements, .. } => {
            let _depth = ConstraintDepth::enter()?;
            let mut alternatives: Vec<Vec<(TupleElementKind, Type)>> = vec![Vec::new()];
            for (kind, ty) in elements {
                if *kind != TupleElementKind::Variadic {
                    for alternative in &mut alternatives {
                        alternative.push((*kind, ty.clone()));
                    }
                    continue;
                }
                let constraint = base_constraint(ty)??;
                let members = match &constraint {
                    Type::Union(union) => union.types().to_vec(),
                    other => vec![other.clone()],
                };
                if alternatives.len() * members.len() > 8 {
                    return None;
                }
                let mut next = Vec::with_capacity(alternatives.len() * members.len());
                for alternative in &alternatives {
                    for member in &members {
                        let mut spread = alternative.clone();
                        push_spread_elements(&mut spread, member)?;
                        next.push(spread);
                    }
                }
                alternatives = next;
            }
            let tuples: Option<Vec<Type>> = alternatives.into_iter().map(normalized_tuple_type).collect();
            Some(Some(crate::union_type(tuples?)))
        }
        // `getConstraintOfConditionalType`.
        DeferredType::Conditional(conditional) => match &conditional.distributive_constraint {
            Some(constraint) => Some(Some(constraint.clone())),
            None => default_conditional_constraint(conditional).map(Some),
        },
        // `computeBaseConstraint`: the template over its placeholders' base
        // constraints, or `string` when one has none.
        DeferredType::TemplateLiteral { texts, types } => {
            let _depth = ConstraintDepth::enter()?;
            let mut constraints = Vec::with_capacity(types.len());
            for ty in types {
                match base_constraint(ty)? {
                    Some(constraint) if !is_generic(&constraint) => constraints.push(constraint),
                    Some(_) => return None,
                    None => return Some(Some(Type::String)),
                }
            }
            Some(Some(crate::template_literal_type(texts, &constraints)))
        }
        // `computeBaseConstraint`: the mapping of the operand's base
        // constraint, or `string` when it has none.
        DeferredType::StringMapping { kind, operand } => {
            let _depth = ConstraintDepth::enter()?;
            match base_constraint(operand)? {
                Some(constraint) if constraint != *operand && !is_generic(&constraint) => {
                    Some(Some(crate::string_mapping_type(*kind, &constraint)))
                }
                Some(_) => None,
                None => Some(Some(Type::String)),
            }
        }
    }
}

/// tsc's `getBaseConstraintOfType`: `Some(None)` for a variable without a
/// constraint, `None` for one surge cannot resolve.
fn base_constraint(ty: &Type) -> Option<Option<Type>> {
    let _depth = ConstraintDepth::enter()?;
    match ty {
        Type::TypeParameter(parameter) => match active_constraint(parameter)? {
            None => Some(None),
            Some(constraint) if constraint == *ty => Some(None),
            Some(constraint) => base_constraint(&constraint),
        },
        Type::Union(union) => {
            let mut members = Vec::with_capacity(union.types().len());
            for member in union.types() {
                match base_constraint(member)? {
                    Some(member) => members.push(member),
                    None => return Some(None),
                }
            }
            Some(Some(crate::union_type(members)))
        }
        // `computeBaseConstraint` of an intersection holding a variable: the
        // intersection of its operands' constraints, an unconstrained one
        // contributing none.
        Type::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object
                    .intersection_operands
                    .as_deref()
                    .is_some_and(|operands| operands.iter().any(|operand| matches!(operand, Type::TypeParameter(_)))) =>
        {
            let mut constraints = Vec::new();
            for operand in object.intersection_operands.as_deref().unwrap_or_default() {
                if let Some(constraint) = base_constraint(operand)? {
                    constraints.push(constraint);
                }
            }
            if constraints.is_empty() {
                return Some(None);
            }
            Some(Some(crate::assignability::intersect_constraint_types(&constraints)?))
        }
        Type::GenuineUnknown => Some(Some(Type::GenuineUnknown)),
        other if other.is_unmodelled() => None,
        other => Some(Some(other.clone())),
    }
}

/// tsc's `getBaseConstraintOrType`.
fn base_constraint_or_self(ty: &Type) -> Option<Type> {
    Some(base_constraint(ty)?.unwrap_or_else(|| ty.clone()))
}

/// tsc's `getBaseConstraintOrType`, with a type whose constraint surge cannot
/// compute standing for itself.
pub fn base_constraint_or_type(ty: &Type) -> Type {
    if !ty.is_type_variable() {
        return ty.clone();
    }
    base_constraint_or_self(ty).unwrap_or_else(|| ty.clone())
}

/// `isGenericObjectType` / `isGenericIndexType` for what a base constraint
/// can leave behind. An intersection holding a variable is merged, not
/// generic here: the lookup reads it as unmodelled.
fn is_generic(ty: &Type) -> bool {
    match ty {
        Type::TypeParameter(_) => true,
        Type::Union(union) => union.types().iter().any(is_generic),
        _ => false,
    }
}

enum Lookup {
    Found(Type),
    Missing,
    Unmodelled,
}

/// tsc's `getIndexedAccessTypeOrUndefined` with no access node, over a
/// non-generic object and key: the type each key constituent selects,
/// `Some(None)` when one selects nothing, `None` when surge cannot tell.
fn indexed_access_lookup(object: &Type, index: &Type, no_index_signatures: bool) -> Option<Option<Vec<Type>>> {
    let keys = match index {
        Type::Union(union) => union.types(),
        other => std::slice::from_ref(other),
    };
    let mut types = Vec::with_capacity(keys.len());
    let mut unmodelled = false;
    for key in keys {
        match property_type_for_index(object, key, no_index_signatures) {
            Lookup::Found(ty) => types.push(ty),
            Lookup::Missing => return Some(None),
            Lookup::Unmodelled => unmodelled = true,
        }
    }
    (!unmodelled).then_some(Some(types))
}

/// tsc's `getPropertyTypeForIndexType`: a literal key names a property, and
/// any key falls back to the index signature that applies to it — a symbol
/// key to the string one. `NoIndexSignatures` (a type variable's constraint
/// written through) admits only a number index.
fn property_type_for_index(object: &Type, key: &Type, no_index_signatures: bool) -> Lookup {
    let object = object.peeled();
    if matches!(object, Type::Any | Type::Never) {
        return Lookup::Found(object);
    }
    let (name, numeric) = match key {
        Type::StringLiteral(value) => (Some(value.as_str()), crate::is_numeric_key(value)),
        Type::NumberLiteral(literal) => (Some(literal.value.as_str()), true),
        Type::String | Type::Symbol => (None, false),
        Type::Number => (None, true),
        _ => return Lookup::Unmodelled,
    };
    match &object {
        Type::Object(object) => {
            if object.synthetic_open_index || object.intersection_operands.is_some() {
                return Lookup::Unmodelled;
            }
            if let Some(name) = name {
                if let Some(property) = object.properties.get(name) {
                    if property.restriction.is_some() || property.index_slot {
                        return Lookup::Unmodelled;
                    }
                    return Lookup::Found(if property.optional {
                        crate::union_type(vec![property.ty.clone(), Type::Undefined])
                    } else {
                        property.ty.clone()
                    });
                }
                if crate::object_prototype_member_type(name).is_some() {
                    return Lookup::Unmodelled;
                }
            }
            if numeric && let Some(value) = object.number_index_type.as_deref() {
                return Lookup::Found(value.clone());
            }
            match object.string_index_type.as_deref() {
                Some(_) if no_index_signatures => Lookup::Missing,
                Some(value) => Lookup::Found(value.clone()),
                None => Lookup::Missing,
            }
        }
        Type::Array(element) if numeric => Lookup::Found(element.as_ref().clone()),
        Type::String | Type::StringLiteral(_) if name.is_none() => {
            if numeric {
                Lookup::Found(Type::String)
            } else {
                Lookup::Missing
            }
        }
        Type::Array(_)
        | Type::Number
        | Type::NumberLiteral(_)
        | Type::Boolean
        | Type::BooleanLiteral(_)
        | Type::BigInt
        | Type::Symbol
            if name.is_none() =>
        {
            Lookup::Missing
        }
        Type::Null | Type::Undefined | Type::Void | Type::GenuineUnknown => Lookup::Missing,
        _ => Lookup::Unmodelled,
    }
}

/// The base constraint of `object[index]` for writing, the target side of
/// `structuredTypeRelatedToWorker`'s `IndexedAccess` arm: absent while either
/// base is still generic, and read without the index signatures of a type
/// variable's constraint (`AccessFlags.NoIndexSignatures`).
pub fn indexed_access_write_constraint(object: &Type, index: &Type) -> TargetConstraint {
    let _depth = match ConstraintDepth::enter() {
        Some(depth) => depth,
        None => return TargetConstraint::Unmodelled,
    };
    let (Some(base_object), Some(base_index)) = (base_constraint_or_self(object), base_constraint_or_self(index))
    else {
        return TargetConstraint::Unmodelled;
    };
    if is_generic(&base_object) || is_generic(&base_index) {
        return TargetConstraint::Absent;
    }
    match indexed_access_lookup(&base_object, &base_index, base_object != *object) {
        None => TargetConstraint::Unmodelled,
        Some(None) => TargetConstraint::Absent,
        Some(Some(types)) => TargetConstraint::Types(types),
    }
}

/// The keys a source of `keyof operand` may be: `keyof C` for the constraint
/// `C` of `operand` (the target side of the `Index` arm; relater.go reads the
/// simplified type or constraint of the operand).
pub fn keyof_constraint_keys(operand: &Type) -> TargetConstraint {
    let _depth = match ConstraintDepth::enter() {
        Some(depth) => depth,
        None => return TargetConstraint::Unmodelled,
    };
    let Type::TypeParameter(parameter) = operand else {
        return TargetConstraint::Unmodelled;
    };
    // `getKnownKeysOfTupleType`: the indexes of the leading fixed elements
    // and the keys of `Array`.
    if let Some((elements, _)) = generic_tuple(operand) {
        let fixed = elements
            .iter()
            .take_while(|(kind, _)| matches!(kind, TupleElementKind::Required | TupleElementKind::Optional))
            .count();
        let mut keys: Vec<Type> = (0..fixed).map(|index| Type::StringLiteral(index.to_string())).collect();
        keys.push(Type::Number);
        keys.extend(crate::array_property_names().iter().map(|name| Type::StringLiteral((*name).to_string())));
        return TargetConstraint::Types(vec![crate::union_type(keys)]);
    }
    if deferred_type(parameter).is_some() {
        return TargetConstraint::Unmodelled;
    }
    match active_constraint(parameter) {
        None => TargetConstraint::Unmodelled,
        Some(None) => TargetConstraint::Absent,
        // `T extends { [K in keyof T]: X }`: the constraint's keys are `keyof T`
        // again, which tsc relates only as `Ternary.Maybe` — not enough here.
        Some(Some(constraint @ Type::TypeParameter(_)))
            if keyof_variable(&constraint).is_some_and(|keys| {
                matches!(
                    &keys,
                    Type::TypeParameter(keys) if matches!(
                        deferred_type(keys),
                        Some(DeferredType::Keyof(keyed)) if keyed == *operand
                    )
                )
            }) =>
        {
            TargetConstraint::Absent
        }
        Some(Some(constraint @ Type::TypeParameter(_))) => keyof_constraint_keys(&constraint),
        Some(Some(constraint)) => match keys_of(&constraint) {
            Some(keys) => TargetConstraint::Types(vec![keys]),
            None => TargetConstraint::Unmodelled,
        },
    }
}

/// tsc's `getIndexType` for a non-generic type surge models member by member.
fn keys_of(ty: &Type) -> Option<Type> {
    keys_of_with(ty, false)
}

fn keys_of_with(ty: &Type, no_index_signatures: bool) -> Option<Type> {
    match ty.peeled() {
        Type::Any | Type::Never => Some(key_constraint()),
        Type::GenuineUnknown | Type::Null | Type::Undefined | Type::Void => Some(Type::Never),
        Type::Object(object) => {
            if object.synthetic_open_index || object.intersection_operands.is_some() {
                return None;
            }
            let mut keys = Vec::new();
            for (name, property) in object.properties.iter() {
                if crate::private_name::is_private_name_key(name) || property.restriction.is_some() {
                    continue;
                }
                // A computed member's key is a symbol, and a numeric name is a
                // number literal key or a string one depending on how it was
                // written; neither is recorded.
                if name.starts_with('[') || crate::is_numeric_key(name) || property.index_slot {
                    return None;
                }
                keys.push(Type::StringLiteral(name.to_string()));
            }
            if !no_index_signatures {
                if object.string_index_type.is_some() {
                    keys.extend([Type::String, Type::Number]);
                } else if object.number_index_type.is_some() {
                    keys.push(Type::Number);
                }
            }
            Some(if keys.is_empty() { Type::Never } else { crate::union_type(keys) })
        }
        _ => None,
    }
}

/// tsc's `getLowerBoundOfKeyType`: the keys a generic mapped type over `keys`
/// is known to have (`resolveMappedTypeMembers`). `None` when surge cannot
/// compute them.
pub fn lower_bound_of_key_type(ty: &Type) -> Option<Type> {
    let _depth = ConstraintDepth::enter()?;
    match ty {
        Type::TypeParameter(parameter) => match deferred_type(parameter) {
            Some(DeferredType::Keyof(operand)) => {
                let apparent = base_constraint_or_type(&operand);
                if is_generic(&apparent) {
                    return None;
                }
                keys_of(&apparent)
            }
            Some(DeferredType::Conditional(conditional)) if conditional.distributive => {
                let check = lower_bound_of_key_type(&conditional.check)?;
                if check == conditional.check {
                    return Some(ty.clone());
                }
                distributive_conditional_at(&conditional, &check)
            }
            _ => Some(ty.clone()),
        },
        Type::Union(union) => {
            let members: Option<Vec<Type>> = union.types().iter().map(lower_bound_of_key_type).collect();
            Some(crate::union_type(members?))
        }
        _ => Some(ty.clone()),
    }
}

/// `getConditionalTypeInstantiation` of a distributive conditional with its
/// check type replaced by the non-generic `check`, distributed over its union
/// members. `None` when a member's branch cannot be decided or rebuilt.
fn distributive_conditional_at(conditional: &DeferredConditional, check: &Type) -> Option<Type> {
    let Type::TypeParameter(check_parameter) = &conditional.check else {
        return None;
    };
    if conditional.has_infer || mentions_type_variable(&conditional.extends) || mentions_type_variable(check) {
        return None;
    }
    let members = match check {
        Type::Union(union) => union.types().to_vec(),
        Type::Never => return Some(Type::Never),
        other => vec![other.clone()],
    };
    let mut results = Vec::with_capacity(members.len());
    for member in members {
        if member.is_unknown() || matches!(member, Type::Any) {
            return None;
        }
        let branch = if crate::is_assignable_to(&member, &conditional.extends) {
            &conditional.true_type
        } else {
            &conditional.false_type
        };
        let instantiated = substitute_variable(branch, check_parameter, &member);
        results.push(resolve_instantiated_substitution(instantiated)?);
    }
    Some(crate::union_type(results))
}

/// An instantiated substitution type (`check & extends` in a true branch) is
/// its base type once that is no longer a type variable and is assignable to
/// the constraint.
fn resolve_instantiated_substitution(ty: Type) -> Option<Type> {
    let Type::Object(object) = &ty else {
        return (!ty.is_unmodelled()).then_some(ty);
    };
    let Some(operands) = object.intersection_operands.as_deref() else {
        return Some(ty);
    };
    let [base, constraints @ ..] = operands else {
        return None;
    };
    if base.is_type_variable() || base.is_unmodelled() || constraints.iter().any(mentions_type_variable) {
        return None;
    }
    constraints
        .iter()
        .all(|constraint| crate::is_assignable_to(base, constraint))
        .then(|| base.clone())
}

impl TypeParameterType {
    pub fn is_active_variable(&self) -> bool {
        self.owner != 0
            && ACTIVE_VARIABLES.with(|active| {
                active
                    .borrow()
                    .iter()
                    .any(|variable| variable.owner == self.owner && *variable.name == *self.name)
            })
    }
}

impl Type {
    /// A type variable of the generic body being checked: a real type, related
    /// through its constraint, not an unmodelled hole.
    pub fn is_type_variable(&self) -> bool {
        matches!(self, Type::TypeParameter(parameter) if parameter.is_active_variable())
    }

    /// [`Type::is_unknown`] without the type variables of the body being
    /// checked: what a relation must not judge.
    pub fn is_unmodelled(&self) -> bool {
        self.is_unknown() && !self.is_type_variable()
    }
}

/// Whether `ty` mentions a type variable of a body being checked. Such a
/// variable is identified by its owner, which a key that renders the type by
/// name does not see: two bodies' `T` look alike.
pub fn mentions_type_variable(ty: &Type) -> bool {
    fn walk(ty: &Type, depth: usize) -> bool {
        if depth > 8 {
            return false;
        }
        let nested = |ty: &Type| walk(ty, depth + 1);
        match ty {
            Type::TypeParameter(parameter) => parameter.owner != 0,
            Type::Array(element) => nested(element),
            Type::Tuple(elements) => elements.iter().any(nested),
            Type::Union(union) => union.types().iter().any(nested),
            Type::Function(function) => function.parameters().iter().any(nested) || nested(function.return_type()),
            Type::Object(object) => {
                object.properties.values().any(|property| nested(&property.ty))
                    || object.string_index_type.as_deref().is_some_and(nested)
                    || object.number_index_type.as_deref().is_some_and(nested)
                    || object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(nested))
            }
            Type::Reference(reference) => reference.arguments.iter().any(nested),
            _ => false,
        }
    }
    walk(ty, 0)
}

/// `variable & with`, as narrowing a type variable produces it (`T & string`
/// under `typeof`, `T & C` under `instanceof`): a memberless intersection
/// whose operands are the whole type. The relation reads the operands, and a
/// primitive operand answers member reads.
pub fn intersect_type_variable(variable: &Type, with: Type) -> Type {
    type_variable_intersection(vec![variable.clone(), with])
}

/// An intersection with at least one type variable operand, kept as its
/// operands rather than merged: tsc keeps `T & X` generic, and merging would
/// drop the variable (surge's merge treats it as an unmodelled operand).
pub fn type_variable_intersection(operands: Vec<Type>) -> Type {
    Type::Object(
        crate::ObjectType::new(Default::default(), None)
            .with_intersection_marker()
            .with_intersection_operands(operands),
    )
}

/// A type variable narrowed to `T & X` (see [`intersect_type_variable`]).
pub fn is_narrowed_type_variable(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if object.is_intersection
            && object.properties.is_empty()
            && object.string_index_type.is_none()
            && object
                .intersection_operands
                .as_deref()
                .is_some_and(|operands| operands.iter().any(|operand| matches!(operand, Type::TypeParameter(_)))))
}

/// A type variable narrowed to `T & null` or `T & undefined`: definitely falsy.
pub fn is_nullish_type_variable_intersection(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if object.is_intersection
            && object.properties.is_empty()
            && object.intersection_operands.as_deref().is_some_and(|operands| {
                operands.iter().any(|operand| matches!(operand, Type::TypeParameter(_)))
                    && operands.iter().any(|operand| matches!(operand, Type::Null | Type::Undefined | Type::Void))
            }))
}

/// tsc's `getGlobalNonNullableTypeInstantiation` for a type variable whose
/// constraint may be nullish: `T` proven non-nullish (truthy, `!`, `?.`) is
/// `T & {}`, which keeps its identity and relates through the constraint left
/// once nullish members are gone. Anything else is returned as it is.
pub fn non_nullable_type_variable(ty: &Type) -> Type {
    if !matches!(ty, Type::TypeParameter(_)) {
        return ty.clone();
    }
    // `hasTypeFacts` of an instantiable type reads its base constraint.
    let Some(constraint) = base_constraint(ty) else {
        return ty.clone();
    };
    let may_be_nullish = match &constraint {
        None => true,
        Some(constraint) if constraint.is_unknown() => true,
        Some(Type::Union(union)) => union
            .types()
            .iter()
            .any(|member| matches!(member, Type::Null | Type::Undefined | Type::Void) || member.is_unknown()),
        Some(other) => matches!(other, Type::Null | Type::Undefined | Type::Void),
    };
    if !may_be_nullish || !crate::strict_null_checks() {
        return ty.clone();
    }
    intersect_type_variable(ty, Type::Object(crate::ObjectType::new(Default::default(), None)))
}

/// tsc's generic tuple type over `elements`, deferred for as long as its
/// variadic elements' variables. `None` when a variadic element is not a type
/// variable of the body being checked.
pub fn generic_tuple_variable(elements: Vec<(TupleElementKind, Type)>, readonly: bool) -> Option<Type> {
    let mut bases = Vec::new();
    for (kind, ty) in &elements {
        if *kind == TupleElementKind::Variadic {
            bases.extend(variable_bases(ty)?);
        }
    }
    let parts: Vec<String> = elements
        .iter()
        .map(|(kind, ty)| match kind {
            TupleElementKind::Required => ty.name(),
            TupleElementKind::Optional => format!("{}?", tuple_element_name(ty)),
            TupleElementKind::Rest => format!("...{}[]", tuple_element_name(ty)),
            TupleElementKind::Variadic => format!("...{}", ty.name()),
        })
        .collect();
    let name = format!("{}[{}]", if readonly { "readonly " } else { "" }, parts.join(", "));
    deferred_variable(DeferredType::Tuple { elements, readonly }, name, bases)
}

/// The elements and readonly-ness of a generic tuple type.
pub fn generic_tuple(ty: &Type) -> Option<(Vec<(TupleElementKind, Type)>, bool)> {
    let Type::TypeParameter(parameter) = ty else {
        return None;
    };
    match deferred_type(parameter)? {
        DeferredType::Tuple { elements, readonly } => Some((elements, readonly)),
        _ => None,
    }
}

/// `readonly` over a generic tuple type.
pub fn readonly_generic_tuple(ty: &Type) -> Option<Type> {
    let (elements, _) = generic_tuple(ty)?;
    generic_tuple_variable(elements, true)
}

fn tuple_element_name(ty: &Type) -> String {
    let name = ty.name();
    if matches!(ty, Type::Union(_) | Type::Function(_)) {
        format!("({name})")
    } else {
        name
    }
}

/// The elements a spread of the non-generic array-like `ty` contributes,
/// with their `ElementFlags`, or `None` for anything else.
pub fn spread_elements(ty: &Type) -> Option<Vec<(TupleElementKind, Type)>> {
    let mut elements = Vec::new();
    push_spread_elements(&mut elements, ty)?;
    Some(elements)
}

fn push_spread_elements(elements: &mut Vec<(TupleElementKind, Type)>, ty: &Type) -> Option<()> {
    if let Some((fixed, min_length)) = crate::fixed_tuple_parts(ty) {
        for (index, element) in fixed.iter().enumerate() {
            let kind = if index < min_length { TupleElementKind::Required } else { TupleElementKind::Optional };
            elements.push((kind, element.clone()));
        }
        return Some(());
    }
    if let Type::Reference(reference) = ty
        && reference.is_readonly_array()
    {
        return push_spread_elements(elements, &reference.resolve());
    }
    match ty.peeled() {
        Type::Array(element) => elements.push((TupleElementKind::Rest, *element)),
        Type::OpenTuple(open) => {
            elements.extend(open.leading.into_iter().map(|ty| (TupleElementKind::Required, ty)));
            elements.push((TupleElementKind::Rest, *open.rest));
            elements.extend(open.trailing.into_iter().map(|ty| (TupleElementKind::Required, ty)));
        }
        tuple @ Type::Tuple(_) => return push_spread_elements(elements, &tuple),
        Type::Any => elements.push((TupleElementKind::Rest, Type::Any)),
        _ => return None,
    }
    Some(())
}

/// `createNormalizedTupleType` for elements with no variadic one left: the
/// rest elements and everything between them merge into one rest.
fn normalized_tuple_type(elements: Vec<(TupleElementKind, Type)>) -> Option<Type> {
    let first_rest = elements.iter().position(|(kind, _)| *kind == TupleElementKind::Rest);
    let Some(first_rest) = first_rest else {
        let min_length = elements
            .iter()
            .rposition(|(kind, _)| *kind == TupleElementKind::Required)
            .map_or(0, |index| index + 1);
        let types = elements.into_iter().map(|(_, ty)| ty).collect();
        return Some(crate::written_tuple_type(types, min_length));
    };
    let last_rest = elements.iter().rposition(|(kind, _)| *kind == TupleElementKind::Rest)?;
    let leading: Vec<Type> = elements[..first_rest].iter().map(|(_, ty)| ty.clone()).collect();
    let rest = crate::union_type(elements[first_rest..=last_rest].iter().map(|(_, ty)| ty.clone()).collect());
    let trailing: Vec<Type> = elements[last_rest + 1..].iter().map(|(_, ty)| ty.clone()).collect();
    if leading.is_empty() && trailing.is_empty() {
        return Some(Type::Array(Box::new(rest)));
    }
    Some(Type::OpenTuple(crate::OpenTupleType {
        leading,
        rest: Box::new(rest),
        trailing,
    }))
}
