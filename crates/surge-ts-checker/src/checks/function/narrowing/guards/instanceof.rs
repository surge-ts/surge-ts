use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, TypeCopyReason, union_type};

use crate::context::CheckerContext;
use crate::symbols::{InterfaceInfo, SymbolInfo, SymbolTable, TypeDeclarationInfo};

/// tsc's `hasBaseType`, which `isTypeDerivedFrom` decides `instanceof` by:
/// whether a member names a class or interface whose `extends` chain reaches
/// the declaration of the constructor's instance type. Derivation is nominal —
/// an unrelated class is not an instance of an empty one it happens to be
/// assignable to.
#[derive(Clone, Copy)]
pub(crate) struct InstanceofHeritage<'a> {
    target: (&'a str, usize),
    ctx: &'a CheckerContext,
}

impl std::fmt::Debug for InstanceofHeritage<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("InstanceofHeritage").field(&self.target).finish()
    }
}

impl<'a> InstanceofHeritage<'a> {
    /// `None` for the global `Object` and `Function`, which every object or
    /// function type derives from without naming them.
    pub(crate) fn new(instance: &Type, ctx: &'a CheckerContext) -> Option<Self> {
        if super::super::type_guards::is_global_object_or_function(instance) {
            return None;
        }
        let target = named_declaration(instance, ctx)?;
        Some(Self {
            target: declaration_mark(target)?,
            ctx,
        })
    }

    /// `None` when the member is not a class or interface surge can find, or
    /// its chain passes an expression base or a name that does not resolve.
    fn derives(&self, member: &Type) -> Option<bool> {
        let declaration = named_declaration(member, self.ctx)?;
        if declaration_mark(declaration)? == self.target {
            return Some(true);
        }
        self.reaches(declaration, 0)
    }

    fn reaches(&self, declaration: &InterfaceInfo, depth: usize) -> Option<bool> {
        if depth >= 32 {
            return None;
        }
        let mut decided = Some(false);
        for base in &declaration.body.extends {
            if base.name == surge_ts_syntax::EXPRESSION_HERITAGE_BASE {
                return None;
            }
            let found = declaration
                .resolution_scope
                .as_ref()
                .and_then(|scope| scope.get(&base.name))
                .or_else(|| self.ctx.lookup_type_declaration(&base.name));
            let Some(TypeDeclarationInfo::Interface(base)) = found else {
                decided = None;
                continue;
            };
            if declaration_mark(base) == Some(self.target) {
                return Some(true);
            }
            match self.reaches(base, depth + 1) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => decided = None,
            }
        }
        decided
    }
}

/// The class or interface declaration a nominal reference names, found by
/// its own name and confirmed by its declaration identity.
fn named_declaration<'a>(ty: &Type, ctx: &'a CheckerContext) -> Option<&'a InterfaceInfo> {
    let id = match ty {
        Type::Reference(reference) if !reference.id.starts_with('\u{0}') => &reference.id,
        Type::Object(object) => object.alias_id.as_ref()?,
        _ => return None,
    };
    let (_, name) = id.rsplit_once('\u{0}')?;
    let declaration = ctx.lookup_type_declaration(name)?;
    let key = crate::infer::types::type_declaration_resolution_key(declaration);
    if *crate::infer::types::type_declaration_alias_id(declaration, &key) != **id {
        return None;
    }
    match declaration {
        TypeDeclarationInfo::Interface(info) => Some(info),
        TypeDeclarationInfo::Alias(_) => None,
    }
}

/// Where a declaration is written, which an import rename does not change.
fn declaration_mark(declaration: &InterfaceInfo) -> Option<(&str, usize)> {
    declaration.name_span.map(|span| (&*declaration.file_name, span.start))
}

/// Whether a union member is an instance of the class/interface named `ctor_name`
/// (a nominal name match). `Some(false)` for a member that definitely is not (a
/// primitive, or a differently-named object); `None` when undecidable (`any`/
/// `unknown`), so the member is kept in both branches.
pub(super) fn instanceof_matches(member: &Type, ctor_name: &str) -> Option<bool> {
    // `x instanceof Array` decides array-ness, not a nominal name match: an
    // array member renders as `T[]` and a tuple as `[A, B]`, so the name compare
    // below rejected both and `messageOrMessages instanceof Array ? … : [ … ]`
    // narrowed nothing. Same membership test `Array.isArray` uses.
    if ctor_name == "Array" && !matches!(member, Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_)) {
        return Some(is_array_like_member(member));
    }
    match member {
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => None,
        Type::String
        | Type::StringLiteral(_)
        | Type::Number
        | Type::NumberLiteral(_)
        | Type::Boolean
        | Type::BooleanLiteral(_)
        | Type::Undefined
        | Type::Void
        | Type::Never => Some(false),
        // Match nominally on the member's own name (`Blob`, `URLSearchParams`):
        // a `Type::Reference` reports its referenced name without resolving, so
        // do not peel (peeling would expand to the structural shape). Compare the
        // base name with any type arguments stripped, so a generic member
        // (`Promise<T>`, `Map<K, V>`) matches its bare constructor (`Promise`,
        // `Map`) instead of being treated as a different, undecidable type — the
        // latter left `x instanceof Promise` unable to drop the non-Promise arm.
        other => {
            let name = other.name();
            let base = name.split('<').next().unwrap_or(name.as_str());
            let own_name = base.rsplit('.').next().unwrap_or(base);
            Some(base == ctor_name || own_name == ctor_name)
        }
    }
}

/// [`instanceof_matches`] with the constructor's own instance type as a second
/// opinion. The name compare cannot see heritage — `TRPCClientError` guarded by
/// `instanceof Error` reads as a different type — so a member it rejects is
/// re-tested: through its declared heritage when both sides name declarations
/// whose chain can be followed, otherwise against the instance type.
///
/// The structural re-test is deliberately asymmetric: the member must be
/// assignable to the instance *and* the instance not assignable back. A type
/// that is merely shaped like the constructor's instance (`{ name: string;
/// message: string }` against `Error`) relates in both directions and stays
/// undecided, so the negative branch does not drop it. A real subclass adds
/// members, so it does not.
pub(super) fn instanceof_matches_with_heritage(
    member: &Type,
    ctor_name: &str,
    instance: Option<&Type>,
    heritage: Option<InstanceofHeritage<'_>>,
) -> Option<bool> {
    match instanceof_matches(member, ctor_name) {
        // `isTypeDerivedFrom`: a primitive derives from no class, however
        // little that class declares.
        // An anonymous object type has no base types to derive through.
        Some(false)
            if is_definitely_not_an_object(member)
                || matches!(member, Type::Object(object) if object.alias_id.is_none() && object.alias_name.is_none()) =>
        {
            Some(false)
        }
        Some(false) => {
            let Some(instance) = instance else {
                return Some(false);
            };
            if instance.is_unknown() || member.is_unknown() {
                return Some(false);
            }
            // The member *is* the instance type: a constructor-like value
            // whose `prototype` names it, where no class name can match.
            if member == instance {
                return Some(true);
            }
            if let Some(derived) = heritage.and_then(|heritage| heritage.derives(member)) {
                return Some(derived);
            }
            if !surge_ts_types::is_assignable_to(member, instance) {
                return Some(false);
            }
            (!surge_ts_types::is_assignable_to(instance, member)).then_some(true)
        }
        decided => decided,
    }
}

/// Whether a union member can never be the left operand of a *true*
/// `instanceof` — a primitive, `undefined`/`null`, or `never`.
pub(super) fn is_definitely_not_an_object(member: &Type) -> bool {
    matches!(
        member,
        Type::String
            | Type::StringLiteral(_)
            | Type::Number
            | Type::NumberLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Symbol
            | Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Never
    )
}

/// Whether a union member is an array or tuple, including one written in
/// generic form (`Array<T>` / `ReadonlyArray<T>`), which stays a nominal
/// reference rather than a [`Type::Array`].
pub(super) fn is_array_like_member(member: &Type) -> bool {
    match member {
        Type::Array(_) | Type::Tuple(_) => true,
        Type::Reference(reference) => matches!(
            reference.id.split('\u{0}').next_back(),
            Some("Array" | "ReadonlyArray")
        ),
        _ => false,
    }
}

/// Narrows a union by an `x instanceof Ctor` guard. `keep_matching` keeps the
/// members that are instances of `Ctor` (the `=== true` branch); otherwise
/// removes them. Members whose membership is undecidable are kept either way.
pub(crate) fn narrow_union_by_instanceof(
    ty: &Type,
    ctor_name: &str,
    instance: Option<&Type>,
    heritage: Option<InstanceofHeritage<'_>>,
    keep_matching: bool,
) -> Option<Type> {
    // Peel a lazy/nominal reference to its structural form first: a deferred
    // generic alias such as `MaybeAsync<T>` (= `T | Promise<T>`) reaches here as a
    // `Type::Reference`, and matching `Type::Union` directly would miss the union
    // it resolves to, leaving `x instanceof Promise` unable to drop the non-Promise
    // arm.
    let peeled = ty.peeled();
    if let Some(narrowed) = narrow_type_variables_by_instanceof(&peeled, ctor_name, instance, heritage, keep_matching) {
        return narrowed;
    }
    let Type::Union(union) = &peeled else {
        // A lone type that is an instance of the constructor cannot reach the
        // `else` branch (`isTypeDerivedFrom` filters it out), so an exhaustive
        // chain of `instanceof` checks ends at `never`.
        return (!keep_matching
            && instanceof_matches_with_heritage(&peeled, ctor_name, instance, heritage) == Some(true))
        .then_some(Type::Never);
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(
            |member| match instanceof_matches_with_heritage(member, ctor_name, instance, heritage) {
                Some(is_instance) => is_instance == keep_matching,
                None => true,
            },
        )
        .cloned()
        .collect();

    // tsc's `getNarrowedType`: with no member derived from the candidate, a
    // candidate that is itself a subtype of the subject is what the value is.
    if kept.is_empty()
        && keep_matching
        && let Some(instance) = instance
        && !instance.is_unknown()
        && union
            .types()
            .iter()
            .any(|member| !member.is_unknown() && surge_ts_types::is_assignable_to(instance, member))
        && !union
            .types()
            .iter()
            .any(|member| surge_ts_types::is_assignable_to(member, instance))
    {
        return Some(instance.clone());
    }

    if kept.is_empty() && keep_matching {
        // No member *names* the constructor, but `x instanceof C` still proves
        // the value is an object: a nominal member may be a subclass whose
        // heritage this name-based test cannot see (`Maybe<TRPCClientError>`
        // guarded by `instanceof Error`), while a primitive or nullish member
        // definitely is not. Dropping only the definitely-not members is what
        // keeps `error.message` from staying `string | undefined`.
        let objects: Vec<Type> = union
            .types()
            .iter()
            .filter(|member| !is_definitely_not_an_object(member))
            .cloned()
            .collect();
        if !objects.is_empty() && objects.len() < union.types().len() {
            return Some(union_type(objects));
        }
        return None;
    }

    if kept.is_empty() || kept.len() == union.types().len() {
        return None;
    }
    Some(union_type(kept))
}

/// tsc's `getNarrowedType` with `checkDerived` where the subject holds a type
/// variable of the body being checked. A variable is derived from the
/// constructor when its constraint is; the true branch keeps derived members,
/// and a variable that is not derived becomes `T & C` unless some other member
/// already matched (then it is dropped, like any unrelated member). The false
/// branch drops derived members. `None` when no member is such a variable.
fn narrow_type_variables_by_instanceof(
    ty: &Type,
    ctor_name: &str,
    instance: Option<&Type>,
    heritage: Option<InstanceofHeritage<'_>>,
    keep_matching: bool,
) -> Option<Option<Type>> {
    let members: Vec<Type> = match ty {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let derived = |member: &Type| -> Option<bool> {
        let Type::TypeParameter(parameter) = member else {
            return None;
        };
        let constraint = surge_ts_types::type_variable::active_constraint(parameter)?;
        Some(constraint.is_some_and(|constraint| {
            constraint.is_type_variable()
                || instanceof_matches_with_heritage(&constraint, ctor_name, instance, heritage) == Some(true)
        }))
    };
    if members.iter().all(|member| derived(member).is_none()) {
        return None;
    }
    let other_matched = members.iter().any(|member| match derived(member) {
        Some(derived) => derived,
        None => instanceof_matches_with_heritage(member, ctor_name, instance, heritage) == Some(true),
    });
    let instance = instance.filter(|instance| !instance.is_unknown());
    let kept: Vec<Type> = members
        .iter()
        .filter_map(|member| match (derived(member), keep_matching) {
            (Some(true), true) => Some(member.clone()),
            (Some(false), true) if other_matched => None,
            (Some(false), true) => Some(match instance {
                Some(instance) => surge_ts_types::type_variable::intersect_type_variable(member, instance.clone()),
                None => member.clone(),
            }),
            (Some(true), false) => None,
            (Some(false), false) => Some(member.clone()),
            (None, _) => match instanceof_matches_with_heritage(member, ctor_name, instance, heritage) {
                Some(is_instance) => (is_instance == keep_matching).then(|| member.clone()),
                None => Some(member.clone()),
            },
        })
        .collect();
    if kept.is_empty() {
        return Some(Some(Type::Never));
    }
    if kept.len() == members.len() && kept.iter().zip(&members).all(|(kept, member)| kept == member) {
        return Some(None);
    }
    Some(Some(union_type(kept)))
}

/// Parses an `x instanceof Ctor` guard, returning the operand expression and the
/// constructor identifier name. (`instanceof` has no equality polarity — the
/// then-branch always keeps the matching members.)
pub(crate) fn parse_instanceof_condition(
    condition: &ParsedExpression,
) -> Option<(&ParsedExpression, &str)> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        left,
        operator: ParsedBinaryOperator::Instanceof,
        right,
        ..
    } = condition
    else {
        return None;
    };
    // A namespace-qualified constructor (`x instanceof schemas.$ZodType`) is
    // matched by its own name, the last segment.
    let ctor_name = match right.as_ref() {
        ParsedExpression::Identifier { name, .. } => name,
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } if matches!(object.as_ref(), ParsedExpression::Identifier { .. }) => property_name,
        _ => return None,
    };
    Some((left.as_ref(), ctor_name.as_str()))
}

/// The value an `instanceof` test's right operand names: its binding, or for
/// a namespace-qualified constructor (`x instanceof core.$ZodType`) the
/// member's type, which no binding of the last segment's name carries.
pub(crate) fn instanceof_constructor_value(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut crate::context::CheckerContext,
) -> Option<Type> {
    let ParsedExpression::Binary { right, .. } = condition else {
        return None;
    };
    match right.as_ref() {
        ParsedExpression::Identifier { name, .. } => symbols.get(name).map(|symbol| symbol.ty.clone()),
        access @ ParsedExpression::PropertyAccess { .. } => {
            let diagnostics_before = ctx.diagnostics().len();
            let inferred = crate::infer::infer_expression(access, symbols, ctx);
            ctx.truncate_diagnostics(diagnostics_before);
            match inferred {
                crate::infer::InferredExpression::Known(ty) => Some(ty),
                _ => None,
            }
        }
        _ => None,
    }
}

/// tsc's `narrowTypeByInstanceof` through `getInstanceType`: a constructor
/// value without a `prototype` whose construct signatures all return `any`
/// has the instance type `any`, which narrows nothing — not even by the name
/// of a type the constructor shares.
pub(crate) fn constructs_only_any(constructor: Option<&Type>) -> bool {
    let Some(Type::Object(object)) = constructor.map(Type::peeled) else {
        return false;
    };
    if object.properties.contains_key("prototype") {
        return false;
    }
    let Some(construct) = object.construct_signature() else {
        return false;
    };
    let mut signatures = Vec::new();
    construct.push_overload_members(&mut signatures);
    !signatures.is_empty() && signatures.iter().all(|signature| matches!(signature.return_type(), Type::Any))
}

/// Builds a symbol table narrowed by an `x instanceof Ctor` guard for the branch.
pub(crate) fn narrow_instanceof_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let (operand, ctor_name) = parse_instanceof_condition(condition)?;
    let ParsedExpression::Identifier { name, .. } = operand else {
        return None;
    };
    let symbol = symbols.get(name)?;
    if constructs_only_any(symbols.get(ctor_name).map(|constructor| &constructor.ty)) {
        return None;
    }
    // No checker context on this path, so the constructor's instance type is
    // not available: the name compare decides alone, as it always has.
    let narrowed = narrow_union_by_instanceof(&symbol.ty, ctor_name, None, None, branch_is_true)?;
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        name.clone(),
        SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        },
        symbol.ty.clone(),
    );
    Some(narrowed_symbols)
}
