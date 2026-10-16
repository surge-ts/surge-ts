//! tsc's subtype and strict subtype relations (relater.go, `subtypeRelation`
//! and `strictSubtypeRelation`), the signature positions they read, and the
//! union reduction built on them (`getUnionType` with `UnionReductionSubtype`,
//! checker.go `removeSubtypes`).
//!
//! Both relations are restrictions of assignability. They are decided here
//! rather than as a mode of the assignability engine, whose leniencies (a
//! sentinel or placeholder relates to anything, surge's `any` stays
//! permissive) would make every reduction remove too much: a shape surge did
//! not model is never a subtype of anything but itself.

use std::sync::Arc;

use crate::{FunctionType, ObjectProperty, ObjectType, Type, union_type};

/// Which part of a type is an object literal expression's own type (tsc's
/// `ObjectFlagsObjectLiteral`, fresh until a union target regularizes it).
/// surge's types do not record it, so a caller that has the expression
/// supplies it.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LiteralShape {
    #[default]
    Regular,
    /// An object literal, with the shapes of the properties written in it.
    Object(Arc<[(Arc<str>, LiteralShape)]>),
    /// Literal types surge cannot line up with the type (the elements of an
    /// array literal): nothing is decided about it.
    Opaque,
}

static REGULAR: LiteralShape = LiteralShape::Regular;

impl LiteralShape {
    fn property(&self, name: &str) -> &LiteralShape {
        match self {
            LiteralShape::Object(properties) => properties
                .iter()
                .find(|(property, _)| &**property == name)
                .map_or(&REGULAR, |(_, shape)| shape),
            _ => &REGULAR,
        }
    }

    fn is_object_literal(&self) -> bool {
        matches!(self, LiteralShape::Object(_))
    }
}

/// `isTypeRelatedTo(source, target, subtypeRelation)`.
pub fn is_subtype_of(source: &Type, target: &Type) -> bool {
    Relater::new(Relation::Subtype).related(source, &REGULAR, false, target, &REGULAR)
}

/// `isTypeRelatedTo(source, target, subtypeRelation)` for an expression's own
/// type, read with the literal shape the expression was written in: an object
/// literal need not have the target's optional properties.
pub fn is_subtype_of_expression(source: &Type, source_shape: &LiteralShape, target: &Type) -> bool {
    Relater::new(Relation::Subtype).related(source, source_shape, source_shape.is_object_literal(), target, &REGULAR)
}

/// `isTypeRelatedTo(source, target, strictSubtypeRelation)`.
pub fn is_strict_subtype_of(source: &Type, target: &Type) -> bool {
    Relater::new(Relation::StrictSubtype).related(source, &REGULAR, false, target, &REGULAR)
}

/// `getUnionType(types, UnionReductionSubtype)`: the union with every member
/// that is a strict subtype of another removed. Each type carries the literal
/// shape of the expression it was read from.
pub fn subtype_reduced_union(types: Vec<(Type, LiteralShape)>) -> Type {
    let types: Vec<(Type, LiteralShape)> = types
        .into_iter()
        .flat_map(|(ty, shape)| match ty {
            Type::Union(union) => union
                .types()
                .iter()
                .map(|member| (member.clone(), shape.clone()))
                .collect::<Vec<_>>(),
            other => vec![(other, shape)],
        })
        .collect();
    let unioned = union_type(types.iter().map(|(ty, _)| ty.clone()).collect());
    let Type::Union(union) = &unioned else {
        return unioned;
    };
    let mut members = union.types().to_vec();
    if members.iter().any(|member| matches!(member, Type::GenuineUnknown)) {
        return Type::GenuineUnknown;
    }
    remove_redundant_literal_types(&mut members, true);
    let mut shapes: Vec<LiteralShape> = members
        .iter()
        .map(|member| {
            types
                .iter()
                .find(|(ty, _)| ty == member)
                .map_or(LiteralShape::Regular, |(_, shape)| shape.clone())
        })
        .collect();
    remove_subtypes(&mut members, &mut shapes);
    union_type(members)
}

/// `getUnionType(types, UnionReductionLiteral)`, the reduction a written union
/// type gets: a literal beside its own primitive is dropped.
pub fn literal_reduced_union(types: Vec<Type>) -> Type {
    let unioned = union_type(types);
    let Type::Union(union) = &unioned else {
        return unioned;
    };
    let mut members = union.types().to_vec();
    let count = members.len();
    remove_redundant_literal_types(&mut members, false);
    if members.len() == count { unioned } else { union_type(members) }
}

/// checker.go `removeRedundantLiteralTypes`: a literal is dropped beside its
/// own primitive, and with `reduce_void_undefined` `undefined` beside `void`.
fn remove_redundant_literal_types(members: &mut Vec<Type>, reduce_void_undefined: bool) {
    let has = |members: &[Type], primitive: &Type| members.iter().any(|member| member == primitive);
    let includes_string = has(members, &Type::String);
    let includes_number = has(members, &Type::Number);
    let includes_symbol = has(members, &Type::Symbol);
    let includes_void = has(members, &Type::Void);
    members.retain(|member| {
        let redundant = match member {
            Type::StringLiteral(_) => includes_string,
            Type::NumberLiteral(_) => includes_number,
            Type::Undefined => reduce_void_undefined && includes_void,
            Type::Reference(reference) if reference.is_unique_symbol() => includes_symbol,
            Type::Reference(_) if is_pattern_literal(member) => includes_string,
            Type::Reference(reference) if reference.enum_owner.is_some() => match &*reference.resolve_arc() {
                Type::StringLiteral(_) => includes_string,
                Type::NumberLiteral(_) => includes_number,
                // An enum's own type is the union of its member literals.
                _ if reference.numeric_enum => includes_number,
                Type::Union(union) => {
                    includes_string && union.types().iter().all(|member| match member {
                        Type::StringLiteral(_) => true,
                        Type::Reference(member) => matches!(&*member.resolve_arc(), Type::StringLiteral(_)),
                        _ => false,
                    })
                }
                _ => false,
            },
            _ => false,
        };
        !redundant
    });
}

/// checker.go `removeSubtypes`, run from the last member to the first so of
/// two mutual strict subtypes the later one goes. tsc also keeps a class
/// instance that is not derived from the class it is a subtype of; surge's
/// types do not say which objects are class instances, so that exception is
/// not applied.
fn remove_subtypes(members: &mut Vec<Type>, shapes: &mut Vec<LiteralShape>) {
    if members.len() < 2 {
        return;
    }
    let has_empty_object = members.iter().any(|member| {
        matches!(member.peeled(), Type::Object(object)
            if is_empty_object_surface(&object) && !object.non_primitive && !object.is_intersection)
    });
    let mut index = members.len();
    while index > 0 {
        index -= 1;
        let source = members[index].clone();
        if !has_empty_object && !is_structured_or_instantiable(&source) {
            continue;
        }
        if let Type::TypeParameter(parameter) = &source
            && let Some(Some(Type::Union(_))) = crate::type_variable::active_constraint(parameter)
        {
            let others = union_type(
                members
                    .iter()
                    .map(|member| if *member == source { Type::Never } else { member.clone() })
                    .collect(),
            );
            if Relater::new(Relation::StrictSubtype).related(&source, &REGULAR, false, &others, &REGULAR) {
                members.remove(index);
                shapes.remove(index);
            }
            continue;
        }
        let source_shape = shapes[index].clone();
        let removed = (0..members.len()).filter(|other| *other != index).any(|other| {
            Relater::new(Relation::StrictSubtype).related(
                &source,
                &source_shape,
                source_shape.is_object_literal(),
                &members[other],
                &shapes[other],
            )
        });
        if removed {
            members.remove(index);
            shapes.remove(index);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    Subtype,
    StrictSubtype,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallbackCheck {
    None,
    Strict,
    Bivariant,
}

/// relater.go `SignatureCheckMode`, as far as the subtype relations use it.
#[derive(Debug, Clone, Copy)]
struct SignatureCheckMode {
    strict_top_signature: bool,
    strict_arity: bool,
    callback: CallbackCheck,
}

/// Past this nesting a comparison is taken to hold, as tsc answers a deeply
/// nested one `Maybe`.
const MAX_DEPTH: u32 = 48;

struct Relater {
    relation: Relation,
    depth: u32,
    in_progress: Vec<(usize, usize)>,
}

impl Relater {
    fn new(relation: Relation) -> Self {
        Self {
            relation,
            depth: 0,
            in_progress: Vec::new(),
        }
    }

    fn strict(&self) -> bool {
        self.relation == Relation::StrictSubtype
    }

    /// relater.go `isRelatedTo`. `source_fresh` is whether the source is still
    /// a fresh object literal, which only an object-shaped source can be.
    fn related(
        &mut self,
        source: &Type,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &Type,
        target_shape: &LiteralShape,
    ) -> bool {
        if source == target || matches!(target, Type::Any | Type::ErrorType) || matches!(source, Type::Never) {
            return true;
        }
        if matches!(source_shape, LiteralShape::Opaque) || matches!(target_shape, LiteralShape::Opaque) {
            return false;
        }
        if is_unmodelled(source) || is_unmodelled(target) {
            return false;
        }
        if self.depth >= MAX_DEPTH {
            return true;
        }
        self.depth += 1;
        let result = self.related_worker(source, source_shape, source_fresh, target, target_shape);
        self.depth -= 1;
        result
    }

    fn related_worker(
        &mut self,
        source: &Type,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &Type,
        target_shape: &LiteralShape,
    ) -> bool {
        if self.simple_type_related(source, source_fresh, target) {
            return true;
        }
        // `isSimpleTypeRelatedTo` is the only rule an `any`, `unknown` or
        // primitive-to-primitive pair has.
        if matches!(source, Type::Any | Type::ErrorType | Type::GenuineUnknown) {
            return false;
        }
        if !is_structured_or_instantiable(source) && !is_structured_or_instantiable(target) {
            return false;
        }

        if let Type::Union(union) = source {
            if !matches!(source_shape, LiteralShape::Regular) {
                return false;
            }
            return union
                .types()
                .iter()
                .all(|member| self.related(member, &REGULAR, false, target, target_shape));
        }
        if let Type::Union(union) = target {
            // The excess property check sees the whole union; the members are
            // related to the regular form of the literal
            // (`getRegularTypeOfObjectLiteral`).
            if source_fresh
                && let Type::Object(object) = source
                && is_excess_property_check_target(target)
                && object
                    .properties
                    .keys()
                    .any(|name| !is_known_property(target, name))
            {
                return false;
            }
            if union.types().iter().any(|member| member == source) {
                return true;
            }
            return union
                .types()
                .iter()
                .any(|member| self.related(source, source_shape, false, member, &REGULAR));
        }

        // An intersection source is related when one of its operands is, and
        // otherwise through its merged members below (`someTypeRelatedToType`).
        if let Type::Object(object) = source
            && object.is_intersection
            && let Some(operands) = object.intersection_operands.as_deref()
            && operands.iter().any(|operand| {
                !matches!(operand, Type::Object(_)) && self.related(operand, &REGULAR, false, target, target_shape)
            })
        {
            return true;
        }
        if crate::type_variable::is_narrowed_type_variable(target)
            && let Type::Object(object) = target
            && let Some(operands) = object.intersection_operands.as_deref()
        {
            return operands
                .iter()
                .all(|operand| self.related(source, source_shape, source_fresh, operand, &REGULAR));
        }

        if let Type::TypeParameter(parameter) = source {
            let constraint = crate::type_variable::active_constraint(parameter).flatten();
            return match constraint {
                Some(constraint) if constraint != *source => {
                    self.related(&constraint, &REGULAR, false, target, target_shape)
                }
                // An unconstrained type variable is related as `{}` is, to
                // anything but the `object` keyword.
                _ => {
                    let target = without_non_primitive(target);
                    self.related(&empty_object(), &REGULAR, false, &target, target_shape)
                }
            };
        }
        if target.is_type_variable() {
            return false;
        }

        if let Some(related) = self.reference_related(source, source_shape, source_fresh, target, target_shape) {
            return related;
        }

        self.structured_type_related(source, source_shape, source_fresh, target, target_shape)
    }

    /// relater.go `isSimpleTypeRelatedTo` under the subtype relations: the
    /// assignable-only rules (an `any` source, a number into a numeric enum,
    /// the unknown-like union) are exactly what these relations leave out.
    fn simple_type_related(&self, source: &Type, source_fresh: bool, target: &Type) -> bool {
        if matches!(target, Type::Any | Type::ErrorType) || matches!(source, Type::Never) {
            return true;
        }
        if matches!(target, Type::GenuineUnknown)
            && !(self.strict() && matches!(source, Type::Any | Type::ErrorType))
        {
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
        let strict_null_checks = crate::strict_null_checks();
        if matches!(source, Type::Undefined)
            && (!strict_null_checks && !matches!(target, Type::Union(_))
                || matches!(target, Type::Undefined | Type::Void))
        {
            return true;
        }
        if matches!(source, Type::Null)
            && (!strict_null_checks && !matches!(target, Type::Union(_)) || matches!(target, Type::Null))
        {
            return true;
        }
        matches!(target, Type::Object(object) if object.non_primitive && !object.is_intersection)
            && is_object_type(source)
            && !(self.strict() && is_empty_anonymous_object(source) && !source_fresh)
    }

    /// The rules a nominal reference decides before it is peeled. `None`
    /// leaves the pair to the structural comparison.
    fn reference_related(
        &mut self,
        source: &Type,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &Type,
        target_shape: &LiteralShape,
    ) -> Option<bool> {
        if let Type::Reference(target_reference) = target
            && let Some(target_enum) = &target_reference.enum_owner
        {
            // Enum types are nominal (`isEnumTypeRelatedTo`), and a plain
            // literal is a subtype of no enum member, whatever its value.
            let Type::Reference(source_reference) = source else {
                return Some(false);
            };
            if source_reference.enum_owner.as_ref() != Some(target_enum) {
                return Some(false);
            }
            let (Some(source_values), Some(target_values)) = (enum_values(source), enum_values(target)) else {
                return Some(false);
            };
            return Some(source_values.iter().all(|value| target_values.contains(value)));
        }
        if let Type::Reference(reference) = target
            && reference.is_unique_symbol()
        {
            return Some(false);
        }
        match (promise_reference(source), promise_reference(target)) {
            // Two promise instantiations relate through `T`'s measured
            // variance, which is covariant, and a `Promise` is a `PromiseLike`
            // but not the other way round.
            (Some((source_is_like, source_arguments)), Some((target_is_like, target_arguments))) => {
                return Some(
                    (!source_is_like || target_is_like)
                        && source_arguments.len() == target_arguments.len()
                        && source_arguments
                            .iter()
                            .zip(target_arguments.iter())
                            .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR)),
                );
            }
            // A promise's own members are not compared here: only a target
            // asking for none is decided.
            (Some(_), None) => {
                return Some(matches!(target, Type::Object(object)
                    if is_empty_object_surface(object)
                        && !object.non_primitive
                        && !object.synthetic_open_index
                        && !target_shape.is_object_literal()));
            }
            (None, Some(_)) => return Some(false),
            (None, None) => {}
        }
        if is_pattern_literal(target) {
            return Some(crate::is_assignable_to(source, target));
        }
        // A pattern's base constraint is `string` (`getBaseConstraintOfType`).
        if is_pattern_literal(source) {
            return Some(self.related(&Type::String, &REGULAR, false, target, target_shape));
        }

        match (readonly_array_element(source), readonly_array_element(target)) {
            (Some(source_inner), Some(target_inner)) => {
                return Some(self.related(&source_inner, &REGULAR, false, &target_inner, &REGULAR));
            }
            // `readonly T[]` has none of the mutating members a mutable array
            // or a member written against one reads.
            (Some(_), None) => return Some(false),
            (None, Some(target_inner)) => {
                let source = source.peeled();
                return Some(
                    matches!(source, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
                        && self.related(&source, &REGULAR, false, &target_inner, &REGULAR),
                );
            }
            (None, None) => {}
        }

        if let Type::Reference(reference) = source {
            let resolved = reference.resolve_arc();
            return Some(self.related(&resolved, source_shape, source_fresh, target, target_shape));
        }
        if let Type::Reference(reference) = target {
            let resolved = reference.resolve_arc();
            return Some(self.related(source, source_shape, source_fresh, &resolved, target_shape));
        }
        None
    }

    /// relater.go `structuredTypeRelatedToWorker` for the shapes surge models.
    fn structured_type_related(
        &mut self,
        source: &Type,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &Type,
        target_shape: &LiteralShape,
    ) -> bool {
        match (source, target) {
            (Type::Array(source), Type::Array(target)) => {
                self.related(source, &REGULAR, false, target, &REGULAR)
            }
            (Type::Tuple(source), Type::Tuple(target)) => {
                source.len() == target.len()
                    && source
                        .iter()
                        .zip(target.iter())
                        .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR))
            }
            // A mutable tuple is related to an array through its number index
            // type, the union of its elements.
            (Type::Tuple(source), Type::Array(target)) => source
                .iter()
                .all(|source| self.related(source, &REGULAR, false, target, &REGULAR)),
            (Type::OpenTuple(source), Type::Array(target)) => {
                self.related(&source.element_union(), &REGULAR, false, target, &REGULAR)
            }
            (Type::Tuple(source), Type::OpenTuple(target)) => {
                source.len() >= target.fixed_len()
                    && source
                        .iter()
                        .zip(target.leading.iter())
                        .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR))
                    && source[source.len() - target.trailing.len()..]
                        .iter()
                        .zip(target.trailing.iter())
                        .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR))
                    && source[target.leading.len()..source.len() - target.trailing.len()]
                        .iter()
                        .all(|source| self.related(source, &REGULAR, false, &target.rest, &REGULAR))
            }
            (Type::OpenTuple(source), Type::OpenTuple(target)) => {
                source.leading.len() >= target.leading.len()
                    && source.trailing.len() >= target.trailing.len()
                    && source
                        .leading
                        .iter()
                        .zip(target.leading.iter())
                        .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR))
                    && source.leading[target.leading.len()..]
                        .iter()
                        .all(|source| self.related(source, &REGULAR, false, &target.rest, &REGULAR))
                    && self.related(&source.rest, &REGULAR, false, &target.rest, &REGULAR)
                    && source.trailing[..source.trailing.len() - target.trailing.len()]
                        .iter()
                        .all(|source| self.related(source, &REGULAR, false, &target.rest, &REGULAR))
                    && source.trailing[source.trailing.len() - target.trailing.len()..]
                        .iter()
                        .zip(target.trailing.iter())
                        .all(|(source, target)| self.related(source, &REGULAR, false, target, &REGULAR))
            }
            (Type::Function(source), Type::Function(target)) => {
                self.compare_signatures_related(source, target, self.signature_check_mode(), false)
            }
            (Type::Object(source), Type::Object(target)) => {
                self.object_related(source, source_shape, source_fresh, target, target_shape)
            }
            (Type::Object(source), Type::Function(target)) => {
                !source_fresh
                    && self.signatures_related(
                        &signature_list(source.call_signature()),
                        std::slice::from_ref(target),
                        false,
                    )
            }
            // A function's own members are `Function.prototype`'s, which a
            // target member is not compared against here.
            (Type::Function(source), Type::Object(target)) => {
                target.properties.is_empty()
                    && target.construct_signature().is_none()
                    && !target.synthetic_open_index
                    && !target.non_primitive
                    && self.index_signatures_related_to_signatureless(target, false)
                    && self.signatures_related(
                        std::slice::from_ref(source),
                        &signature_list(target.call_signature()),
                        false,
                    )
            }
            // A primitive, array or tuple is related to an object type through
            // its apparent type, whose members are not compared here: only a
            // target that asks for none is decided.
            (source, Type::Object(target))
                if matches!(
                    source,
                    Type::String
                        | Type::Number
                        | Type::Boolean
                        | Type::BigInt
                        | Type::Symbol
                        | Type::StringLiteral(_)
                        | Type::NumberLiteral(_)
                        | Type::BooleanLiteral(_)
                        | Type::Array(_)
                        | Type::Tuple(_)
                        | Type::OpenTuple(_)
                ) =>
            {
                !target.non_primitive
                    && !target.synthetic_open_index
                    && is_empty_object_surface(target)
                    && !target_shape.is_object_literal()
            }
            _ => false,
        }
    }

    fn signature_check_mode(&self) -> SignatureCheckMode {
        SignatureCheckMode {
            strict_top_signature: true,
            strict_arity: self.strict(),
            callback: CallbackCheck::None,
        }
    }

    /// The object arm of `structuredTypeRelatedToWorker`, with the checks
    /// `isRelatedTo` makes of an object source first (excess properties of a
    /// fresh literal, a weak target).
    fn object_related(
        &mut self,
        source: &ObjectType,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &ObjectType,
        target_shape: &LiteralShape,
    ) -> bool {
        if source.synthetic_open_index || target.synthetic_open_index {
            return false;
        }
        if let (Some(source_id), Some(target_id)) = (&source.alias_id, &target.alias_id)
            && source_id == target_id
        {
            return true;
        }
        if source_fresh
            && source
                .properties
                .keys()
                .any(|name| !object_knows_property(target, name))
        {
            return false;
        }
        if is_weak_object(target)
            && (!source.properties.is_empty()
                || source.call_signature().is_some()
                || source.construct_signature().is_some())
            && !source
                .properties
                .keys()
                .any(|name| object_knows_property(target, name))
        {
            return false;
        }
        if target_shape.is_object_literal()
            && is_empty_object_surface(target)
            && !is_empty_object_surface(source)
        {
            return false;
        }

        let key = (
            Arc::as_ptr(&source.properties) as usize,
            Arc::as_ptr(&target.properties) as usize,
        );
        if self.in_progress.contains(&key) {
            return true;
        }
        self.in_progress.push(key);
        let result = self.object_members_related(source, source_shape, source_fresh, target, target_shape);
        self.in_progress.retain(|entry| *entry != key);
        result
    }

    fn object_members_related(
        &mut self,
        source: &ObjectType,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &ObjectType,
        target_shape: &LiteralShape,
    ) -> bool {
        // relater.go `propertiesRelatedTo`: under the subtype relations an
        // optional target member must be present too, unless the source is an
        // object literal.
        let require_optional_properties = !source_shape.is_object_literal();
        for (name, target_property) in target.properties.iter() {
            if target_property.index_slot {
                continue;
            }
            let prototype_member;
            let source_property = match source.properties.get(name.as_ref()) {
                Some(property) => property,
                // `getUnmatchedProperties` passes over a static private name
                // the source lacks.
                None if crate::private_name::is_static(name) => continue,
                // A callable source's apparent type is the global `Function`,
                // whose members a target extending `Function` declares as its
                // own: the source has exactly those members.
                None if (source.call_signature().is_some() || source.construct_signature().is_some())
                    && crate::ty::is_function_member_name(name) =>
                {
                    continue;
                }
                None => match crate::object_prototype_member_type(name) {
                    Some(member) => {
                        prototype_member = ObjectProperty::required(member).with_method(true);
                        &prototype_member
                    }
                    None => {
                        if require_optional_properties || target_property.is_required() {
                            return false;
                        }
                        continue;
                    }
                },
            };
            if !self.property_related(
                source_property,
                source_shape.property(name),
                source_fresh,
                target_property,
                target_shape.property(name),
            ) {
                return false;
            }
        }
        if target_shape.is_object_literal()
            && source
                .properties
                .keys()
                .any(|name| !target.properties.contains_key(name))
        {
            return false;
        }
        self.signatures_related(
            &signature_list(source.call_signature()),
            &signature_list(target.call_signature()),
            false,
        ) && self.signatures_related(
            &signature_list(source.construct_signature()),
            &signature_list(target.construct_signature()),
            false,
        ) && self.index_signatures_related(source, source_fresh, target)
    }

    /// relater.go `propertyRelatedTo` and `isPropertySymbolTypeRelated`.
    fn property_related(
        &mut self,
        source: &ObjectProperty,
        source_shape: &LiteralShape,
        source_fresh: bool,
        target: &ObjectProperty,
        target_shape: &LiteralShape,
    ) -> bool {
        if !restrictions_relate(source.restriction.as_ref(), target.restriction.as_ref()) {
            return false;
        }
        // `{ readonly a: T }` is not a strict subtype of `{ a: T }`, so the
        // reduction does not depend on declaration order.
        if self.strict() && source.readonly && !target.readonly {
            return false;
        }
        let effective_target = with_optionality(&target.ty, target.optional);
        let target_admits_anything = matches!(effective_target, Type::Any | Type::ErrorType)
            || !self.strict() && matches!(effective_target, Type::GenuineUnknown);
        if !target_admits_anything {
            let related = match (target.method, source.ty.peeled(), target.ty.peeled()) {
                // A method is compared with its parameters bivariant, as its
                // declaration kind decides in `compareSignaturesRelated`.
                (true, Type::Function(source_method), Type::Function(target_method)) => {
                    let mut source_signatures = Vec::new();
                    source_method.push_overload_members(&mut source_signatures);
                    let mut target_signatures = Vec::new();
                    target_method.push_overload_members(&mut target_signatures);
                    self.signatures_related(&source_signatures, &target_signatures, true)
                }
                _ => {
                    let effective_source = with_optionality(&source.ty, source.optional);
                    self.related(&effective_source, source_shape, source_fresh, &effective_target, target_shape)
                }
            };
            if !related {
                return false;
            }
        }
        !(source.optional && !target.optional)
    }

    /// relater.go `signaturesRelatedTo`: every target signature needs a
    /// related source signature; a lone pair is related directly.
    fn signatures_related(
        &mut self,
        source: &[FunctionType],
        target: &[FunctionType],
        target_is_method: bool,
    ) -> bool {
        if target.is_empty() {
            return true;
        }
        if source.is_empty() {
            return false;
        }
        let mode = self.signature_check_mode();
        if let ([source], [target]) = (source, target) {
            return self.compare_signatures_related(source, target, mode, target_is_method);
        }
        target.iter().all(|target| {
            source
                .iter()
                .any(|source| self.compare_signatures_related(source, target, mode, target_is_method))
        })
    }

    /// relater.go `compareSignaturesRelated` under the subtype relations.
    fn compare_signatures_related(
        &mut self,
        source: &FunctionType,
        target: &FunctionType,
        mode: SignatureCheckMode,
        target_is_method: bool,
    ) -> bool {
        if source == target {
            return true;
        }
        let source_is_top = is_top_signature(source);
        let target_is_top = is_top_signature(target);
        if !(mode.strict_top_signature && source_is_top) && target_is_top {
            return true;
        }
        if mode.strict_top_signature && source_is_top && !target_is_top {
            return false;
        }
        let target_count = parameter_count(target);
        let source_has_more_parameters = !has_effective_rest_parameter(target)
            && if mode.strict_arity {
                has_effective_rest_parameter(source) || parameter_count(source) > target_count
            } else {
                min_argument_count(source) > target_count
            };
        if source_has_more_parameters {
            return false;
        }
        // A generic source is instantiated in the target's context and a
        // non-array rest compared as a tuple; neither is modelled here.
        if !source.type_parameter_names().is_empty()
            || !target.type_parameter_names().is_empty()
            || non_array_rest_type(source).is_some()
            || non_array_rest_type(target).is_some()
        {
            return false;
        }
        let strict_variance =
            mode.callback == CallbackCheck::None && !target_is_method && crate::strict_function_types();
        let parameter_positions = parameter_count(source).max(target_count);
        let source_min = min_argument_count(source);
        let target_min = min_argument_count(target);
        for position in 0..parameter_positions {
            let (Some(source_type), Some(target_type)) =
                (try_type_at_position(source, position), try_type_at_position(target, position))
            else {
                continue;
            };
            if source_type == target_type && !mode.strict_arity {
                continue;
            }
            let source_callback = (mode.callback == CallbackCheck::None)
                .then(|| single_call_signature(&non_nullable(&source_type)))
                .flatten();
            let target_callback = (mode.callback == CallbackCheck::None)
                .then(|| single_call_signature(&non_nullable(&target_type)))
                .flatten();
            let mut related = match (source_callback, target_callback) {
                (Some(source_callback), Some(target_callback))
                    if nullish_facts(&source_type) == nullish_facts(&target_type) =>
                {
                    let callback_mode = SignatureCheckMode {
                        strict_top_signature: false,
                        strict_arity: mode.strict_arity,
                        callback: if strict_variance {
                            CallbackCheck::Strict
                        } else {
                            CallbackCheck::Bivariant
                        },
                    };
                    self.compare_signatures_related(&target_callback, &source_callback, callback_mode, false)
                }
                _ => {
                    (mode.callback == CallbackCheck::None
                        && !strict_variance
                        && self.related(&source_type, &REGULAR, false, &target_type, &REGULAR))
                        || self.related(&target_type, &REGULAR, false, &source_type, &REGULAR)
                }
            };
            // With strict arity, `(x: number | undefined) => void` is a subtype
            // of `(x?: number | undefined) => void` and not the other way.
            if related
                && mode.strict_arity
                && position >= source_min
                && position < target_min
                && self.related(&source_type, &REGULAR, false, &target_type, &REGULAR)
            {
                related = false;
            }
            if !related {
                return false;
            }
        }
        let target_return = target.return_type();
        if matches!(target_return, Type::Void | Type::Any) {
            return true;
        }
        let source_return = source.return_type();
        (mode.callback == CallbackCheck::Bivariant
            && self.related(target_return, &REGULAR, false, source_return, &REGULAR))
            || self.related(source_return, &REGULAR, false, target_return, &REGULAR)
    }

    /// relater.go `indexSignaturesRelatedTo`.
    fn index_signatures_related(&mut self, source: &ObjectType, source_fresh: bool, target: &ObjectType) -> bool {
        let target_has_string_index = target.string_index_type.is_some();
        let infos = [
            (false, target.string_index_type.as_deref()),
            (true, target.number_index_type.as_deref()),
        ];
        for (numeric, value) in infos {
            let Some(value) = value else {
                continue;
            };
            if !self.strict() && target_has_string_index && matches!(value, Type::Any) {
                continue;
            }
            if let Some(source_value) = source.applicable_index_type(numeric) {
                if !self.related(source_value, &REGULAR, false, value, &REGULAR) {
                    return false;
                }
                continue;
            }
            // In the strict subtype relation only a fresh literal has an
            // implicit index signature, so `{ [x: string]: T }` <: `{}` but not
            // the other way.
            if (!self.strict() || source_fresh) && has_inferable_index(source) {
                if !self.members_related_to_index_info(source, numeric, value) {
                    return false;
                }
                continue;
            }
            return false;
        }
        true
    }

    /// `indexSignaturesRelatedTo` for a source with no index signatures and
    /// no inferable one (a function type).
    fn index_signatures_related_to_signatureless(&self, target: &ObjectType, _source_fresh: bool) -> bool {
        let target_has_string_index = target.string_index_type.is_some();
        [target.string_index_type.as_deref(), target.number_index_type.as_deref()]
            .into_iter()
            .flatten()
            .all(|value| !self.strict() && target_has_string_index && matches!(value, Type::Any))
    }

    /// relater.go `membersRelatedToIndexInfo`.
    fn members_related_to_index_info(&mut self, source: &ObjectType, numeric: bool, value: &Type) -> bool {
        for (name, property) in source.properties.iter() {
            if name.starts_with("[Symbol.") || numeric && !crate::object::is_numeric_key(name) {
                continue;
            }
            let property_type = if numeric || !property.optional {
                with_optionality(&property.ty, property.optional)
            } else {
                strip_undefined(&property.ty)
            };
            if !self.related(&property_type, &REGULAR, false, value, &REGULAR) {
                return false;
            }
        }
        if !numeric
            && let Some(number_index) = source.number_index_type.as_deref()
            && !self.related(number_index, &REGULAR, false, value, &REGULAR)
        {
            return false;
        }
        true
    }
}

/// A shape this relation cannot judge: surge's degradation sentinel, or a
/// type parameter that is only a placeholder.
fn is_unmodelled(ty: &Type) -> bool {
    matches!(ty, Type::Unknown) || matches!(ty, Type::TypeParameter(_)) && !ty.is_type_variable()
}

fn is_pattern_literal(ty: &Type) -> bool {
    crate::is_template_literal_type(ty) || crate::string_mapping_parts(ty).is_some()
}

fn is_string_like(ty: &Type) -> bool {
    match ty {
        Type::String | Type::StringLiteral(_) => true,
        Type::Reference(_) => {
            is_pattern_literal(ty) || matches!(enum_member_value(ty), Some(Type::StringLiteral(_)))
        }
        _ => false,
    }
}

fn is_number_like(ty: &Type) -> bool {
    match ty {
        Type::Number | Type::NumberLiteral(_) => true,
        Type::Reference(reference) => {
            reference.numeric_enum || matches!(enum_member_value(ty), Some(Type::NumberLiteral(_)))
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

/// The member values an enum type or member reference stands for.
fn enum_values(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Reference(reference) => enum_values(&reference.resolve_arc()),
        Type::Union(union) => {
            let mut values = Vec::new();
            for member in union.types() {
                values.extend(enum_values(member)?);
            }
            Some(values)
        }
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::Number | Type::String => Some(vec![ty.clone()]),
        _ => None,
    }
}

/// A lib `Promise<T>` or `PromiseLike<T>` reference, with whether it is the
/// `PromiseLike` and its type arguments. surge collapses the lib promise to
/// its awaited `T`, so peeling one would relate `Promise<T>` to a plain `T`,
/// which tsc never does: the promise is an interface instantiation.
fn promise_reference(ty: &Type) -> Option<(bool, &[Type])> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    if reference.enum_owner.is_some() || reference.arguments.is_empty() {
        return None;
    }
    match reference.display.split('<').next() {
        Some("Promise") => Some((false, &reference.arguments)),
        Some("PromiseLike") => Some((true, &reference.arguments)),
        _ => None,
    }
}

/// tsc's `TypeFlagsObject`: an object, function, array or tuple type.
fn is_object_type(ty: &Type) -> bool {
    match ty {
        Type::Object(object) => !object.non_primitive && !object.is_intersection,
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Reference(reference) => {
            reference.is_readonly_array()
                || promise_reference(ty).is_some()
                || reference.enum_owner.is_none()
                    && !reference.is_unique_symbol()
                    && !is_pattern_literal(ty)
                    && is_object_type(&reference.resolve_arc())
        }
        _ => false,
    }
}

/// tsc's `TypeFlagsStructuredOrInstantiable`, which is what `removeSubtypes`
/// compares: objects, unions, intersections, type variables and patterns.
fn is_structured_or_instantiable(ty: &Type) -> bool {
    match ty {
        Type::Object(_)
        | Type::Function(_)
        | Type::Array(_)
        | Type::Tuple(_)
        | Type::OpenTuple(_)
        | Type::Union(_) => true,
        Type::TypeParameter(_) => ty.is_type_variable(),
        Type::Reference(reference) => {
            reference.enum_owner.is_none() && !reference.is_unique_symbol()
        }
        _ => false,
    }
}

fn is_empty_object_surface(object: &ObjectType) -> bool {
    object.properties.is_empty()
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
}

/// tsc's `IsEmptyAnonymousObjectType`: `{}` written as a type literal.
fn is_empty_anonymous_object(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if is_empty_object_surface(object)
            && !object.non_primitive
            && !object.is_intersection
            && !object.without_inferable_index)
}

fn empty_object() -> Type {
    Type::Object(ObjectType::new(Default::default(), None))
}

fn without_non_primitive(ty: &Type) -> Type {
    let is_object_keyword = |ty: &Type| matches!(ty, Type::Object(object) if object.non_primitive);
    match ty {
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .filter(|member| !is_object_keyword(member))
                .cloned()
                .collect(),
        ),
        ty if is_object_keyword(ty) => Type::Never,
        other => other.clone(),
    }
}

fn readonly_array_element(ty: &Type) -> Option<Type> {
    match ty {
        Type::Reference(reference) if reference.is_readonly_array() => reference.arguments.first().cloned(),
        _ => None,
    }
}

/// tsc's `isWeakType` for a plain object.
fn is_weak_object(object: &ObjectType) -> bool {
    !object.properties.is_empty()
        && object.properties.values().all(ObjectProperty::is_optional)
        && object.string_index_type.is_none()
        && object.number_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
        && !object.is_intersection
        && !object.non_primitive
}

/// tsc's `isKnownProperty` for an object target.
fn object_knows_property(object: &ObjectType, name: &str) -> bool {
    object.properties.contains_key(name)
        || object.string_index_type.is_some()
        || object.number_index_type.is_some() && crate::object::is_numeric_key(name)
}

fn is_known_property(ty: &Type, name: &str) -> bool {
    match ty.peeled() {
        Type::Object(object) => object_knows_property(&object, name),
        Type::Union(union) => union.types().iter().any(|member| is_known_property(member, name)),
        _ => false,
    }
}

/// tsc's `isExcessPropertyCheckTarget`.
fn is_excess_property_check_target(ty: &Type) -> bool {
    match ty.peeled() {
        Type::Object(_) | Type::Function(_) => true,
        Type::Union(union) => union.types().iter().all(is_excess_property_check_target),
        _ => false,
    }
}

/// tsc's `isObjectTypeWithInferableIndex`: an object or type literal, not an
/// interface or class instance, and nothing callable.
fn has_inferable_index(object: &ObjectType) -> bool {
    !object.without_inferable_index
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
}

/// The modifier rules of `propertyRelatedTo`, as surge's members carry them.
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

/// An optional member's or parameter's type as `getTypeOfSymbol` reads it:
/// with `undefined` under `strictNullChecks`.
fn with_optionality(ty: &Type, optional: bool) -> Type {
    if optional && crate::strict_null_checks() && !includes_undefined(ty) {
        union_type(vec![ty.clone(), Type::Undefined])
    } else {
        ty.clone()
    }
}

fn includes_undefined(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Any | Type::GenuineUnknown | Type::Unknown | Type::ErrorType => true,
        Type::Union(union) => union.types().iter().any(includes_undefined),
        _ => false,
    }
}

fn strip_undefined(ty: &Type) -> Type {
    match ty {
        Type::Union(union) if union.types().iter().any(|member| matches!(member, Type::Undefined)) => union_type(
            union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// tsc's `getNonNullableType`.
fn non_nullable(ty: &Type) -> Type {
    match ty {
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `getTypeFacts(t, TypeFactsIsUndefinedOrNull)`: whether a type may be
/// `undefined`, and whether it may be `null`.
fn nullish_facts(ty: &Type) -> (bool, bool) {
    match ty {
        Type::Undefined | Type::Void => (true, false),
        Type::Null => (false, true),
        Type::Any | Type::ErrorType | Type::GenuineUnknown | Type::Unknown => (true, true),
        Type::Union(union) => union.types().iter().fold((false, false), |facts, member| {
            let member = nullish_facts(member);
            (facts.0 || member.0, facts.1 || member.1)
        }),
        _ => (false, false),
    }
}

/// tsc's `getSingleCallSignature`: a type whose only member is one call
/// signature.
fn single_call_signature(ty: &Type) -> Option<FunctionType> {
    match ty.peeled() {
        Type::Function(function) if function.overloads().is_none() => Some(function),
        Type::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.construct_signature().is_none() =>
        {
            object.call_signature().filter(|signature| signature.overloads().is_none()).cloned()
        }
        _ => None,
    }
}

fn signature_list(signature: Option<&FunctionType>) -> Vec<FunctionType> {
    let mut signatures = Vec::new();
    if let Some(signature) = signature {
        signature.push_overload_members(&mut signatures);
    }
    signatures
}

/// relater.go `isTopSignature`: `(...args: any[]) => any` and its `never` and
/// `unknown` spellings.
fn is_top_signature(signature: &FunctionType) -> bool {
    if !signature.type_parameter_names().is_empty()
        || signature.parameters().len() != 1
        || !signature.is_variadic()
    {
        return false;
    }
    let rest = match signature.parameters()[0].peeled() {
        Type::Array(element) => *element,
        other => other,
    };
    matches!(rest, Type::Any | Type::ErrorType | Type::Never)
        && matches!(signature.return_type(), Type::Any | Type::ErrorType | Type::GenuineUnknown)
}

fn rest_parameter(signature: &FunctionType) -> Option<&Type> {
    signature
        .is_variadic()
        .then(|| signature.parameters().last())
        .flatten()
}

/// The fixed tuple a rest parameter spells (`...args: [a: string]`), which
/// counts as the positions it lists.
fn fixed_tuple_rest(rest: &Type) -> Option<Vec<Type>> {
    match rest.peeled() {
        Type::Tuple(elements) => Some(elements),
        _ => None,
    }
}

/// relater.go `getParameterCount`: a rest parameter counts once, a tuple-typed
/// one as the fixed positions it spells.
pub fn parameter_count(signature: &FunctionType) -> usize {
    let length = signature.parameters().len();
    match rest_parameter(signature).map(Type::peeled) {
        Some(Type::Tuple(elements)) => length - 1 + elements.len(),
        Some(Type::OpenTuple(tuple)) => length + tuple.leading.len(),
        _ => length,
    }
}

/// relater.go `getMinArgumentCount`: the declared minimum (or the required
/// elements a rest tuple spells), less the trailing `void` parameters a call
/// may omit.
pub fn min_argument_count(signature: &FunctionType) -> usize {
    let length = signature.parameters().len();
    let from_rest = match rest_parameter(signature).map(Type::peeled) {
        Some(Type::Tuple(elements)) if !elements.is_empty() => Some(length - 1 + elements.len()),
        Some(Type::OpenTuple(tuple)) if !tuple.leading.is_empty() => Some(length - 1 + tuple.leading.len()),
        _ => None,
    };
    let mut minimum = from_rest.unwrap_or_else(|| signature.required_parameter_count());
    while minimum > 0 {
        let accepts_void = try_type_at_position(signature, minimum - 1).is_some_and(|ty| match ty {
            Type::Void => true,
            Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Void)),
            _ => false,
        });
        if !accepts_void {
            break;
        }
        minimum -= 1;
    }
    minimum
}

/// relater.go `hasEffectiveRestParameter`: a rest parameter that is not a
/// fixed tuple.
pub fn has_effective_rest_parameter(signature: &FunctionType) -> bool {
    rest_parameter(signature).is_some_and(|rest| fixed_tuple_rest(rest).is_none())
}

/// relater.go `getNonArrayRestType`: a rest parameter typed by something other
/// than an array (a variadic tuple, a union of tuples).
fn non_array_rest_type(signature: &FunctionType) -> Option<Type> {
    let rest = rest_parameter(signature)?;
    if readonly_array_element(rest).is_some() {
        return None;
    }
    match rest.peeled() {
        Type::Array(_) | Type::Tuple(_) | Type::Any | Type::ErrorType => None,
        other => Some(other),
    }
}

/// relater.go `tryGetTypeAtPosition`: the type an argument at `position`
/// meets — an optional parameter's with `undefined`, a rest parameter's
/// element — or `None` past the last position.
pub fn try_type_at_position(signature: &FunctionType, position: usize) -> Option<Type> {
    let parameters = signature.parameters();
    let fixed = parameters.len() - usize::from(rest_parameter(signature).is_some());
    if position < fixed {
        let optional = position >= signature.required_parameter_count();
        return Some(with_optionality(&parameters[position], optional));
    }
    let rest = rest_parameter(signature)?;
    let index = position - fixed;
    if let Some(elements) = fixed_tuple_rest(rest) {
        return elements.get(index).cloned();
    }
    Some(rest_element_at(rest, index))
}

/// `getTypeAtPosition`: [`try_type_at_position`], `any` past the end.
pub fn type_at_position(signature: &FunctionType, position: usize) -> Type {
    try_type_at_position(signature, position).unwrap_or(Type::Any)
}

/// The element a rest parameter's type holds at `index` past the fixed
/// parameters (`getIndexedAccessType(restType, index)`).
fn rest_element_at(rest: &Type, index: usize) -> Type {
    if let Some(element) = readonly_array_element(rest) {
        return rest_element_at(&element, index);
    }
    match rest.peeled() {
        Type::Array(element) => *element,
        Type::Tuple(elements) => elements.get(index).cloned().unwrap_or(Type::Undefined),
        Type::OpenTuple(tuple) => tuple.leading.get(index).cloned().unwrap_or_else(|| {
            let mut members = vec![tuple.rest.as_ref().clone()];
            members.extend(tuple.trailing.iter().cloned());
            union_type(members)
        }),
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| rest_element_at(member, index))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PropertyMap;

    fn function(parameters: Vec<Type>, required: usize) -> FunctionType {
        FunctionType::new(parameters, Type::Void, false, required)
    }

    fn method_object(signature: FunctionType) -> Type {
        let mut properties = PropertyMap::default();
        properties.insert("f".into(), ObjectProperty::required(Type::Function(signature)).with_method(true));
        Type::Object(ObjectType::new(properties, None))
    }

    #[test]
    fn fewer_parameters_is_the_strict_subtype() {
        let none = Type::Function(function(vec![], 0));
        let optional = Type::Function(function(vec![Type::String], 0));
        assert!(is_strict_subtype_of(&none, &optional));
        assert!(!is_strict_subtype_of(&optional, &none));
        assert_eq!(subtype_reduced_union(vec![(none, LiteralShape::Regular), (optional.clone(), LiteralShape::Regular)]), optional);
    }

    #[test]
    fn a_required_parameter_is_the_strict_subtype_of_an_optional_one() {
        let required = function(vec![union_type(vec![Type::String, Type::Undefined])], 1);
        let optional = function(vec![Type::String], 0);
        let required_method = method_object(required);
        let optional_method = method_object(optional);
        assert!(is_strict_subtype_of(&required_method, &optional_method));
        assert!(!is_strict_subtype_of(&optional_method, &required_method));
    }

    #[test]
    fn any_is_a_subtype_only_of_any_and_unknown() {
        assert!(!is_subtype_of(&Type::Any, &Type::String));
        assert!(is_subtype_of(&Type::Any, &Type::GenuineUnknown));
        assert!(!is_strict_subtype_of(&Type::Any, &Type::GenuineUnknown));
        assert!(is_subtype_of(&Type::StringLiteral("a".into()), &Type::String));
    }

    #[test]
    fn a_fresh_literal_with_an_excess_property_is_no_subtype() {
        let mut narrow = PropertyMap::default();
        narrow.insert("a".into(), ObjectProperty::required(Type::Number));
        let mut wide = narrow.clone();
        wide.insert("b".into(), ObjectProperty::required(Type::Number));
        let narrow = Type::Object(ObjectType::new(narrow, None));
        let wide = Type::Object(ObjectType::new(wide, None));
        let literal = LiteralShape::Object(Arc::from(vec![]));
        assert!(is_strict_subtype_of(&wide, &narrow));
        let reduced = subtype_reduced_union(vec![(narrow, literal.clone()), (wide, literal)]);
        assert!(matches!(reduced, Type::Union(union) if union.types().len() == 2));
    }
}
