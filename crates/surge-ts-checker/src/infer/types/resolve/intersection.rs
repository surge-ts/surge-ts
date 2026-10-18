use super::*;

use surge_ts_types::{ObjectType, PropertyMap};

use super::indexed_access::ParsedTypeSpan;

use crate::metrics::{alloc_function_type, alloc_object_type};

/// Reference-only intersections remain nominal during declaration indexing.
/// Without this companion to dependency-alias deferral, constructing
/// `ComponentProps<...> & RefAttributes<...>` immediately peels both deferred
/// operands and recreates the eager expansion the references were meant to
/// avoid. The escape hatch is for paired before/after profiling only.
fn defer_reference_intersections() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("SURGE_EAGER_REFERENCE_INTERSECTIONS").as_deref() != Ok("1"))
}

/// Resolves an intersection `A & B`. Object-like operands are merged into a
/// single object exposing every member's property surface, which lets the
/// existing object machinery (property access, assignability, object-literal
/// checking) handle intersections without a dedicated runtime type. The merged
/// object is tagged via [`with_intersection_marker`] so a missing required
/// property surfaces the outer assignability code tsc reports for intersections.
///
/// Simplification follows the existing `any`/`unknown` policy: `T & any` is
/// `any`, `T & unknown` is `T`. Conflicting properties keep the left operand
/// (full `string & number -> never` reduction is a non-goal). If any operand is
/// unresolved the whole intersection degrades to `Unknown` after the root
/// diagnostic is reported, so reads stay conservative and never cascade.
fn names_uninstalled_global_this(ty: &ParsedType, ctx: &CheckerContext) -> bool {
    matches!(ty, ParsedType::TypeOf(query)
        if query.name == "globalThis"
            && query.members.is_empty()
            && query.import_specifier.is_none())
        && ctx.symbols.get("globalThis").is_none()
        && ctx.ambient_global_symbols.get("globalThis").is_none()
        && !(crate::program::in_check_phase()
            && crate::program::program_ambient_globals()
                .is_some_and(|globals| globals.get("globalThis").is_some()))
}

pub(crate) fn resolve_intersection_type(
    types: Vec<ParsedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let mut resolved_types = Vec::new();
    let mut had_error = false;
    let span = match (types.first().and_then(|first| first.span()), types.last().and_then(|last| last.span())) {
        (Some(first), Some(last)) => Some(surge_ts_syntax::TextSpan { start: first.start, end: last.end }),
        _ => None,
    };

    for ty in types {
        // `declare var window: Window & typeof globalThis` is lowered before the
        // global object exists. The query adds nothing the intersection could
        // use yet (`T & unknown` is `T`), and its degraded answer must not taint
        // the operands that did resolve.
        if names_uninstalled_global_this(&ty, ctx) {
            continue;
        }
        let resolved = resolve_parsed_type(ty, ctx, resolving, substitution);
        had_error |= resolved.had_error;
        resolved_types.push(resolved.ty);
    }
    if resolved_types.is_empty() {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }
    // `checkCrossProductUnion`: an intersection distributes over its union
    // operands, and 100,000 or more combinations is an error. tsc intersects
    // three or more operands half by half, dropping the combinations that
    // reduce to `never` as it goes, so the count is only exact when no member
    // of one operand can conflict with a member of another. Operands are not
    // peeled: that would expand the deferred references this merge keeps.
    let cross_product = resolved_types.iter().fold(1usize, |product, ty| {
        product.saturating_mul(match ty {
            Type::Union(union) => union.types().len().max(1),
            _ => 1,
        })
    });
    if cross_product >= 100_000 && union_operands_share_no_property(&resolved_types) {
        if let Some(span) = span {
            ctx.push(Diagnostic::ts2590(ctx.file_name.clone()).with_span(convert_span(span)));
        }
        return ResolvedType {
            ty: Type::ErrorType,
            had_error,
        };
    }

    // One failed operand must not erase the others: `ComponentProps<"button"> &
    // VariantProps<…>` with an unmodelled second operand still has a fully usable
    // first operand, and collapsing the whole intersection to `unknown` is what
    // strips contextual typing from every prop that flows through it. Merge the
    // usable members (the merge already drops `unknown` operands) but keep
    // `had_error` — the taint still gates every cache/bail exactly as before, so
    // no degraded shape is interned or re-expanded.
    if had_error {
        if resolved_types.iter().all(Type::is_unknown) {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: true,
            };
        }
        // The failed operand's members are unknown, not absent: keep the
        // merge open so a read of one is not reported missing.
        let lost_operand = resolved_types.iter().any(Type::is_unknown);
        let merged = merge_intersection_members(resolved_types);
        return ResolvedType {
            ty: if lost_operand {
                open_object_arms(merged)
            } else {
                merged
            },
            had_error: true,
        };
    }

    ResolvedType {
        ty: merge_intersection_members(resolved_types),
        had_error: false,
    }
}

fn union_operands_share_no_property(operands: &[Type]) -> bool {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, operand) in operands.iter().enumerate() {
        let Type::Union(union) = operand else {
            continue;
        };
        for member in union.types() {
            let Type::Object(object) = member else {
                return false;
            };
            for name in object.properties.keys() {
                if *seen.entry(name.to_string()).or_insert(index) != index {
                    return false;
                }
            }
        }
    }
    true
}

fn open_object_arms(ty: Type) -> Type {
    match ty {
        Type::Object(mut object) => {
            if object.string_index_type.is_none() {
                object.string_index_type = Some(std::sync::Arc::new(Type::Unknown));
            }
            Type::Object(object.with_open_index_marker())
        }
        Type::Union(union) => surge_ts_types::union_type(
            union
                .types()
                .iter()
                .cloned()
                .map(open_object_arms)
                .collect(),
        ),
        other => other,
    }
}

/// Arity ceiling for distributing a union operand across an intersection. Each
/// arm rebuilds the whole merged property map, so the bound keeps the worst case
/// at 8x a shape that is rare in practice; wider unions keep the single merge.
const MAX_DISTRIBUTED_UNION_ARITY: usize = 8;

/// Nesting bound for union distribution. Each arm's merge peels its operands,
/// and peeling a deferred intersection reference re-enters the merge — a
/// self-referential shape recursed until the stack gave out once *every* union
/// operand distributed rather than only a lone one. Beyond this depth the merge
/// falls through to the single open form, which is what it did for these shapes
/// before distribution reached them at all.
const MAX_DISTRIBUTION_DEPTH: u32 = 2;

/// Backstop nesting bound for the property-level merge. A property declared by
/// more than one operand is merged as its own intersection, so a shape that
/// grows a *fresh* operand at every level (a generic re-instantiated through its
/// own property) would otherwise walk until the stack gave out. A cycle whose
/// operands repeat is caught exactly by [`MergeStack`] before this bound
/// matters; real intersections nest a handful of levels.
const MAX_MERGE_DEPTH: u32 = 24;

thread_local! {
    static DISTRIBUTION_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static MERGE_STACK: std::cell::RefCell<Vec<Vec<MergeOperandKey>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Identity of a merge operand: the shared payload pointers for a structural
/// object, the declaration id plus argument list for a reference. Two operands
/// with the same key merge to the same surface, so a merge that meets its own
/// key while still in progress is a cycle, not a deeper shape.
#[derive(Clone, PartialEq, Eq)]
enum MergeOperandKey {
    Object {
        properties: usize,
        string_index: usize,
        call_signature: usize,
        construct_signature: usize,
    },
    Reference {
        id: std::sync::Arc<str>,
        arguments: std::sync::Arc<[Type]>,
    },
    Other(std::mem::Discriminant<Type>),
}

impl MergeOperandKey {
    fn of(ty: &Type) -> Self {
        fn arc_addr<T: ?Sized>(arc: &std::sync::Arc<T>) -> usize {
            std::sync::Arc::as_ptr(arc) as *const u8 as usize
        }
        match ty {
            Type::Object(object) => Self::Object {
                properties: arc_addr(&object.properties),
                string_index: object.string_index_type.as_ref().map_or(0, arc_addr),
                call_signature: object.call_signature.as_ref().map_or(0, arc_addr),
                construct_signature: object.construct_signature.as_ref().map_or(0, arc_addr),
            },
            Type::Reference(reference) => Self::Reference {
                id: reference.id.clone(),
                arguments: reference.arguments.clone(),
            },
            other => Self::Other(std::mem::discriminant(other)),
        }
    }

    fn is_structural(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

/// The merges currently in progress on this thread, outermost first. The
/// operands of every frame stay alive for the frame's duration (the caller owns
/// them), so the payload addresses in the keys cannot be reused underneath it.
struct MergeStack;

enum MergeEntry {
    Frame(MergeStack),
    /// The same operands are already being merged further up the stack.
    Cycle,
    DepthExceeded,
}

impl MergeStack {
    fn enter(members: &[Type]) -> MergeEntry {
        let key: Vec<MergeOperandKey> = members.iter().map(MergeOperandKey::of).collect();
        MERGE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.len() as u32 >= MAX_MERGE_DEPTH {
                return MergeEntry::DepthExceeded;
            }
            if key.iter().any(MergeOperandKey::is_structural) && stack.contains(&key) {
                return MergeEntry::Cycle;
            }
            stack.push(key);
            MergeEntry::Frame(Self)
        })
    }
}

impl Drop for MergeStack {
    fn drop(&mut self) {
        MERGE_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// The surface handed back when a merge re-enters itself. `Window & typeof
/// globalThis` is the canonical shape: `Window.window` is that very
/// intersection, already merged once, so the property is the enclosing surface
/// and tsc renders it as the same type. An operand that is itself a merged
/// intersection is that surface; otherwise fall back to the open first operand.
fn cyclic_merge_result(members: Vec<Type>) -> Type {
    if let Some(merged) = members
        .iter()
        .find(|ty| matches!(ty, Type::Object(object) if object.is_intersection))
    {
        return merged.clone();
    }
    open_merge_fallback(members)
}

/// The surface returned when the merge bound is hit: the first operand, forced
/// OPEN so the members contributed by the operands that were not merged do not
/// read as excess properties. Same policy as an undistributable union operand.
fn open_merge_fallback(members: Vec<Type>) -> Type {
    let Some(first) = members.into_iter().next() else {
        return Type::Unknown;
    };
    match first.peeled() {
        Type::Object(object) if object.string_index_type.is_none() => {
            let mut object = object.with_open_index_marker();
            object.string_index_type = Some(std::sync::Arc::new(Type::Any));
            Type::Object(object)
        }
        other => other,
    }
}

struct DistributionDepth;

impl DistributionDepth {
    fn enter() -> Option<Self> {
        DISTRIBUTION_DEPTH.with(|depth| {
            if depth.get() >= MAX_DISTRIBUTION_DEPTH {
                return None;
            }
            depth.set(depth.get() + 1);
            Some(Self)
        })
    }
}

impl Drop for DistributionDepth {
    fn drop(&mut self) {
        DISTRIBUTION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Reference-id prefixes of the two deferred intersection forms below. Both
/// carry their operands as the reference arguments, which is what lets a
/// nested deferred intersection be flattened instead of merged as an opaque
/// operand.
const DEFERRED_INTERSECTION_ID_PREFIX: &str = "\u{0}intersection\u{0}";

/// The operands of a closed deferred intersection reference.
pub(super) fn deferred_intersection_members(ty: &Type) -> Option<&[Type]> {
    match ty {
        Type::Reference(reference) if reference.id.starts_with(DEFERRED_INTERSECTION_ID_PREFIX) => {
            Some(&reference.arguments)
        }
        _ => None,
    }
}
const OPEN_DEFERRED_INTERSECTION_ID_PREFIX: &str = "\u{0}intersection-open\u{0}";

/// `A & (B & C) ⇒ A & B & C`. Without this a property both operands declare as
/// a deferred intersection nested one level deeper at every merge: `window` on
/// `Window & typeof globalThis` became `(Window & typeof globalThis) & Window`,
/// then `((Window & typeof globalThis) & Window) & Window`, … — every level a
/// new nominal identity, so no cycle guard downstream ever saw the same type
/// twice and assignability unfolded it to its depth cap with an exponential
/// fan-out. Flattened and deduplicated, every level is the same reference.
/// Also reports whether an open wrapper was unwrapped, so the merged result
/// stays open.
fn flatten_deferred_intersections(members: Vec<Type>) -> (Vec<Type>, bool) {
    let mut flat = Vec::with_capacity(members.len());
    let mut unwrapped_open = false;
    let mut pending: Vec<Type> = members.into_iter().rev().collect();
    while let Some(member) = pending.pop() {
        match &member {
            Type::Reference(reference)
                if reference.id.starts_with(DEFERRED_INTERSECTION_ID_PREFIX) =>
            {
                pending.extend(reference.arguments.iter().rev().cloned());
            }
            Type::Reference(reference)
                if reference
                    .id
                    .starts_with(OPEN_DEFERRED_INTERSECTION_ID_PREFIX) =>
            {
                unwrapped_open = true;
                pending.extend(reference.arguments.iter().rev().cloned());
            }
            _ => flat.push(member),
        }
    }
    (flat, unwrapped_open)
}

/// `T & T ⇒ T` for operands with the same identity. A property both operands
/// declare with one type (`Window.document` and the global `document`, both
/// `Document`) would otherwise become a fresh `Document & Document` reference
/// at every merge, and each fresh reference is a new identity for every cycle
/// guard downstream. Only identity-bearing operands (objects, references) are
/// deduplicated; two functions or unions are compared by nothing here.
fn dedup_identical_operands(members: Vec<Type>) -> Vec<Type> {
    if members.len() < 2 {
        return members;
    }
    let mut seen: Vec<MergeOperandKey> = Vec::with_capacity(members.len());
    members
        .into_iter()
        .filter(|ty| {
            let key = MergeOperandKey::of(ty);
            if !key.is_structural() || !seen.contains(&key) {
                seen.push(key);
                true
            } else {
                false
            }
        })
        .collect()
}

/// `SURGE_DISTRIBUTE_REFERENCE_UNIONS=1`: distribute an intersection over a
/// union that sits behind a nominal reference, as tsc's `getIntersectionType`
/// cross product does (`X & (A | B)` becomes `X & A | X & B`). Matching only a
/// bare `Type::Union` operand leaves `ComponentType<P> & { getInitialProps? }`
/// with the object operand alone — no call signature — so every
/// `const App: AppType = (props) => …` is a false TS2322.
///
/// Off because a *downstream* divergence makes it a net loss (measured
/// 2026-09-16, trpc FN 55 -> 70, FP 2 -> 4): with it on, jscodeshift's
/// `JSCodeshift = Core & typeof namedTypes & typeof builders` merges down to the
/// `namedTypes` operand alone — `Object(call=false, props=[Printable, …])` — so
/// `j(file.source)` is a false TS2349 and the 15 diagnostics behind it go
/// silent. `Core`'s *overloaded* call signatures are what get lost; surge's
/// `ObjectType` carries a single `call_signature`. The instantiation budget is
/// not the cause (`SURGE_ROOT_WORK_TRIP` reports zero trips), and the union call
/// path is not either (`check_callable_union_call` never bails on this corpus).
/// Fix the merge's handling of an overloaded operand first, then turn this on.
fn distribute_reference_unions() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("SURGE_DISTRIBUTE_REFERENCE_UNIONS").as_deref() == Ok("1"))
}

pub(crate) fn merge_intersection_members(members: Vec<Type>) -> Type {
    // `getIntersectionType` distributes over a union operand; one holding a
    // type variable has to, or the variable's arm loses the other operands
    // (`NonNullable<T[K] | undefined>` is `T[K] & {}`, not `T[K]`).
    let variable_unions: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, member)| matches!(member, Type::Union(union) if union.types().iter().any(Type::is_type_variable)))
        .map(|(index, _)| index)
        .collect();
    if let [index] = variable_unions.as_slice()
        && let Type::Union(union) = &members[*index]
    {
        let arms = union
            .types()
            .iter()
            .map(|arm| {
                let mut operands = members.clone();
                operands[*index] = arm.clone();
                merge_intersection_members(operands)
            })
            .collect();
        return surge_ts_types::union_type(arms);
    }
    let mut variables: Vec<Type> = Vec::new();
    for member in members.iter().filter(|ty| ty.is_type_variable()) {
        if !variables.contains(member) {
            variables.push(member.clone());
        }
    }
    if variables.is_empty() {
        return merge_intersection_member_types(members);
    }
    let mut others: Vec<Type> = members
        .iter()
        .filter(|ty| !ty.is_type_variable() && !matches!(ty, Type::GenuineUnknown))
        .cloned()
        .collect();
    // `getIntersectionType` removes an empty anonymous object type from an
    // intersection holding an object type — a generic mapped or tuple type is
    // one, where a type parameter is not (`T & {}` stays).
    if variables.iter().any(is_generic_object_type)
        && let Some(index) = others.iter().position(is_empty_anonymous_object_type)
    {
        others.remove(index);
    }
    if others.is_empty() {
        return match <[Type; 1]>::try_from(variables) {
            Ok([variable]) => variable,
            Err(variables) => surge_ts_types::type_variable::type_variable_intersection(variables),
        };
    }
    if !others.iter().any(Type::is_unmodelled)
        && let Some(kept) = with_type_variable_operands(&merge_intersection_member_types(others), &variables)
    {
        return kept;
    }
    // The merge drops a type variable operand like an unmodelled one; the
    // relation still needs it (`T & {}` is assignable to `T`), so it is kept
    // beside the merged members.
    match merge_intersection_member_types(members) {
        Type::Object(object) => {
            let mut operands: Vec<Type> = object.intersection_operands.as_deref().unwrap_or_default().to_vec();
            operands.extend(variables);
            Type::Object(object.with_intersection_marker().with_intersection_operands(operands))
        }
        other => other,
    }
}

fn is_generic_object_type(ty: &Type) -> bool {
    use surge_ts_types::type_variable as tv;
    tv::mapped_type(ty).is_some()
        || tv::mapped_generic_type(ty).is_some()
        || tv::mapped_constant_type(ty).is_some()
        || tv::generic_tuple(ty).is_some()
}

fn is_empty_anonymous_object_type(ty: &Type) -> bool {
    matches!(ty, Type::Object(object)
        if object.properties.is_empty()
            && object.string_index_type.is_none()
            && object.number_index_type.is_none()
            && object.call_signature().is_none()
            && object.construct_signature().is_none()
            && !object.non_primitive
            && !object.is_intersection)
}

/// `merged & variables` kept generic, as tsc's `getIntersectionType` keeps it,
/// with a union distributed over (`T & (A | B)` is `(T & A) | (T & B)`). An
/// object with members stays merged and open, since the variables' members
/// are not enumerated. `None` for an operand kind surge relates only merged.
fn with_type_variable_operands(merged: &Type, variables: &[Type]) -> Option<Type> {
    let narrowed = |operand: &Type| {
        let mut operands = variables.to_vec();
        operands.push(operand.clone());
        surge_ts_types::type_variable::type_variable_intersection(operands)
    };
    match merged {
        Type::Never => Some(Type::Never),
        Type::Union(union) => {
            let arms: Option<Vec<Type>> = union
                .types()
                .iter()
                .map(|member| with_type_variable_operands(member, variables))
                .collect();
            Some(surge_ts_types::union_type(arms?))
        }
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => Some(narrowed(merged)),
        Type::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature.is_none()
                && object.construct_signature.is_none()
                && !object.is_intersection =>
        {
            Some(narrowed(merged))
        }
        Type::Object(object) if !object.is_intersection || object.intersection_operands.is_none() => {
            let mut object = object.clone();
            if object.string_index_type.is_none() {
                object = object.with_open_index_marker();
                object.string_index_type = Some(std::sync::Arc::new(Type::Any));
            }
            Some(Type::Object(
                object.with_intersection_marker().with_intersection_operands(variables.to_vec()),
            ))
        }
        _ => None,
    }
}

/// tsc's `getIntersectionType` over primitive operands alone: two from
/// disjoint domains (`string & number`, `string & null`) or two distinct unit
/// types (`"a" & "b"`, `true & null`) are `never`; otherwise the one unit
/// type, or the primitive itself (`string & "a"` is `"a"`). `None` when some
/// operand is not a primitive.
fn reduce_primitive_intersection(members: &[Type]) -> Option<Type> {
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum Domain {
        String,
        Number,
        BigInt,
        Boolean,
        Symbol,
        Void,
        Null,
    }
    let domain = |ty: &Type| match ty {
        Type::String | Type::StringLiteral(_) => Some(Domain::String),
        Type::Number | Type::NumberLiteral(_) => Some(Domain::Number),
        Type::BigInt => Some(Domain::BigInt),
        Type::Boolean | Type::BooleanLiteral(_) => Some(Domain::Boolean),
        Type::Symbol => Some(Domain::Symbol),
        Type::Void | Type::Undefined => Some(Domain::Void),
        Type::Null => Some(Domain::Null),
        _ => None,
    };
    if members.len() < 2 {
        return None;
    }
    let domains: Vec<Domain> = members.iter().map(domain).collect::<Option<_>>()?;
    if domains.iter().any(|other| *other != domains[0]) {
        return Some(Type::Never);
    }
    let is_unit = |ty: &Type| {
        matches!(ty, Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) | Type::Undefined | Type::Null)
    };
    let mut unit: Option<&Type> = None;
    for member in members.iter().filter(|member| is_unit(member)) {
        match unit {
            Some(existing) if existing != member => return Some(Type::Never),
            _ => unit = Some(member),
        }
    }
    Some(unit.unwrap_or(&members[0]).clone())
}

fn merge_intersection_member_types(members: Vec<Type>) -> Type {
    let (members, unwrapped_open) = flatten_deferred_intersections(members);
    // tsc's `getIntersectionType` (checker.go:26443): an `any` operand is the
    // whole intersection, and the error type outranks a written `any`.
    if members.iter().any(|ty| matches!(ty, Type::ErrorType)) {
        return Type::ErrorType;
    }
    if members.iter().any(|ty| matches!(ty, Type::Any)) {
        return Type::Any;
    }
    if let Some(reduced) = reduce_primitive_intersection(&members) {
        return reduced;
    }

    // A dropped `Type::Unknown` operand is surge's degradation sentinel for an
    // operand it could not model (`ComponentProps<typeof UnmodelledValue> & {…}`),
    // NOT the `unknown` keyword (`GenuineUnknown`). The failed operand may have
    // contributed members we never saw, so a surviving *inline object* surface
    // must stay OPEN — a closed merge would flag every dropped member's use as an
    // excess property. A surviving nominal reference is returned untouched (see
    // the lone-survivor comment below).
    // `Type::Void` counts as a loss too: `PromiseLike<T>` is modelled as its
    // awaited `T`, so `PromiseLike<void> & { pull(): void }` reaches here as
    // `void & {…}` with the promise's own `then` gone. Leaving the object
    // surface closed reported `then` as an excess property on every value
    // written against such a type.
    let dropped_unmodelled_operand = unwrapped_open
        || members
            .iter()
            .any(|ty| matches!(ty, Type::Unknown | Type::TypeParameter(_) | Type::Void));
    // A survivor with an index signature of its own keeps it, and is marked
    // open only beside a dropped sentinel: `Partial<Record<K, V>>` next to a
    // `SafeExtendShape<S, U>` whose `U` surge could not bind is not the whole
    // intersection either, but beside a type variable it is read as declared
    // (TS4111 still names its index-signature members).
    let dropped_sentinel = members.iter().any(|ty| matches!(ty, Type::Unknown));
    let open_if_unmodelled = |ty: Type| -> Type {
        match ty {
            Type::Object(object)
                if dropped_unmodelled_operand && (object.string_index_type.is_none() || dropped_sentinel) =>
            {
                let mut object = object.with_open_index_marker();
                if object.string_index_type.is_none() {
                    object.string_index_type = Some(std::sync::Arc::new(Type::Any));
                }
                Type::Object(object)
            }
            other => other,
        }
    };

    let members: Vec<Type> = members.into_iter().filter(|ty| !ty.is_unknown()).collect();
    let mut members = dedup_identical_operands(members);
    // `getIntersectionType` removes an empty anonymous object type from an
    // intersection holding an object type: `Simplify<T>`'s `{ … } & {}` is
    // the object alone, and reports as one.
    let is_object_type = |ty: &Type| match ty {
        Type::Object(_) => !is_empty_anonymous_object_type(ty),
        Type::Reference(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::Function(_) => true,
        _ => false,
    };
    if members.iter().any(is_object_type)
        && let Some(index) = members.iter().position(is_empty_anonymous_object_type)
    {
        members.remove(index);
    }

    // `T & unknown ⇒ T`: with the `unknown` operands dropped, a lone survivor is
    // returned unchanged. Peeling and re-merging it (below) would force a lazy
    // library reference's bounded structural expansion and discard its nominal
    // identity — e.g. `Window & typeof globalThis` would otherwise corrupt the
    // shared `Window` apparent type.
    if members.len() == 1 {
        let survivor = members.into_iter().next().unwrap();
        // A NAMED lone survivor must become open too — `T & Other` where `T`'s
        // binding degraded leaves `Other` alone, and a closed `Other` reports
        // every member that lived on `T`'s constraint as missing. Peeling it
        // here would force the reference's structural expansion at resolution
        // time (the hazard the comment above describes, and the one the
        // `any`-member degradation counter pins), so defer instead: the merge
        // below peels and opens on first consumer peel.
        if dropped_unmodelled_operand && let Type::Reference(reference) = &survivor {
            crate::program::record_program_counter(|c| c.lazy_intersection_create_count += 1);
            let display = survivor.name();
            let id = format!(
                "{OPEN_DEFERRED_INTERSECTION_ID_PREFIX}{}",
                reference.id.as_ref()
            );
            let members = vec![survivor.clone()];
            return Type::Reference(surge_ts_types::TypeReference::new(
                id,
                display,
                members.clone(),
                deferred_merge(members, true),
            ));
        }
        return open_if_unmodelled(survivor);
    }

    // `X[] & Y[]` reads, indexes and iterates as the intersection of its
    // elements (each operand's members intersect), which an array of that
    // intersection models; the object merge below would keep one operand.
    if members.len() > 1 && members.iter().all(|member| matches!(member, Type::Array(_))) {
        let elements = members
            .into_iter()
            .filter_map(|member| match member {
                Type::Array(element) => Some(*element),
                _ => None,
            })
            .collect();
        return Type::Array(Box::new(merge_intersection_members(elements)));
    }

    // `(A | B) & C` is `(A & C) | (B & C)`. The object merge below only reads
    // `Type::Object` operands, so an undistributed union operand contributes
    // nothing: the merged surface keeps only `C`'s properties and every use of an
    // `A`/`B` member is reported as an excess property.
    //
    // Every union operand distributes, not just a lone one — tRPC's server-side
    // helper options are `(queryClient | queryClientConfig) & (external |
    // internal)`, two unions, and keeping only the first one's members made
    // every `router`/`ctx`/`client` read a false TS2339. The bound is on the
    // *product*: each arm rebuilds the whole merged property map, so the cap
    // holds the worst case at `MAX_DISTRIBUTED_UNION_ARITY` merges however the
    // operands split. Anything wider falls through to the single merge marked
    // open, which suppresses the excess-property report for the names surge did
    // not enumerate.
    let union_operands: Vec<(usize, Vec<Type>)> = members
        .iter()
        .enumerate()
        .filter_map(|(index, ty)| match ty {
            Type::Union(union) => Some((index, union.types().to_vec())),
            // tsc's `getIntersectionType` builds the cross product from the
            // *resolved* constituents, so a union behind a nominal reference
            // distributes like a bare one. See
            // [`distribute_reference_unions`] for why this is opt-in.
            Type::Reference(_) if distribute_reference_unions() => match ty.peeled() {
                Type::Union(union) => Some((index, union.types().to_vec())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let unenumerated_union_operand = !union_operands.is_empty();
    let product = union_operands
        .iter()
        .try_fold(1usize, |product, (_, arms)| {
            (!arms.is_empty()).then(|| product.saturating_mul(arms.len()))
        });
    if let Some(product) = product
        && !union_operands.is_empty()
        && product <= MAX_DISTRIBUTED_UNION_ARITY
        && let Some(_guard) = DistributionDepth::enter()
    {
        let operand_names: Vec<String> = members.iter().map(intersection_operand_name).collect();
        let mut distributed = Vec::with_capacity(product);
        for selection in 0..product {
            let mut operands = members.clone();
            let mut names = operand_names.clone();
            let mut remaining = selection;
            for (index, arms) in &union_operands {
                let arm = &arms[remaining % arms.len()];
                remaining /= arms.len();
                names[*index] = intersection_operand_name(arm);
                operands[*index] = arm.clone();
            }
            if let Some(reduced) = reduce_primitive_intersection(&operands) {
                distributed.push(reduced);
                continue;
            }
            // tsc builds each arm through the same ordered set as any other
            // intersection, so `(Date | undefined) & Date` has a `Date` arm, not
            // a merged `Date & Date` surface.
            let operands = dedup_identical_operands(operands);
            if operands.len() == 1 && !dropped_unmodelled_operand {
                distributed.extend(operands);
                continue;
            }
            distributed.push(merge_intersection_members_now(
                operands,
                Some(names.join(" & ")),
                dropped_unmodelled_operand,
            ));
        }
        return surge_ts_types::union_type(distributed);
    }

    let display_name = (!members.is_empty()).then(|| {
        members
            .iter()
            .map(intersection_operand_name)
            .collect::<Vec<_>>()
            .join(" & ")
    });

    // An intersection whose operands are all lazy/nominal references
    // (`CheckboxProps & RefAttributes<HTMLButtonElement>`) defers its merge:
    // peeling the operands here forces each library reference's structural
    // expansion at *resolution* time, which is what pulls the React/DOM graph
    // while dependency `.d.ts` export tables are being collected. The merge
    // runs instead when a consumer peels the intersection reference. Operands
    // that already carry structure (inline objects, primitives) keep the eager
    // merge so non-reference intersections are unchanged.
    if defer_reference_intersections()
        && members.len() > 1
        && members.iter().all(|ty| matches!(ty, Type::Reference(_)))
    {
        crate::program::record_program_counter(|c| c.lazy_intersection_create_count += 1);
        let display = display_name.unwrap_or_default();
        // Identity from the operands' module-qualified reference ids, not the
        // display form: same-named types from different modules must not
        // collapse into one nominal intersection.
        let id = members
            .iter()
            .filter_map(|ty| match ty {
                Type::Reference(reference) => Some(reference.id.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\u{0}&\u{0}");
        // The operands double as the reference arguments so nominal identity
        // (`same_reference`: id + arguments) distinguishes `A & Ref<X>` from
        // `A & Ref<Y>` — the operand ids alone erase the instantiation.
        // An intersection that dropped an unmodelled operand is open, and must
        // not share its identity with the closed one over the same operands.
        let prefix = if dropped_unmodelled_operand {
            OPEN_DEFERRED_INTERSECTION_ID_PREFIX
        } else {
            DEFERRED_INTERSECTION_ID_PREFIX
        };
        return Type::Reference(surge_ts_types::TypeReference::new(
            format!("{prefix}{id}"),
            display,
            members.clone(),
            deferred_merge(members, dropped_unmodelled_operand),
        ));
    }

    merge_intersection_members_now(
        members,
        display_name,
        dropped_unmodelled_operand || unenumerated_union_operand,
    )
}

thread_local! {
    /// Deferred merges already handed out on this thread, keyed by the operand
    /// resolvers. A property both operands declare is merged afresh for every
    /// surface that carries it, and every fresh deferred reference owned its own
    /// memo, so `window` on one `Window & typeof globalThis` surface re-merged
    /// the same ~1000 members each time a consumer peeled it — that repetition,
    /// not any single merge, is what assignability's structural walk turned into
    /// gigabytes. Sharing the resolver shares the memo. Held weakly: an entry
    /// lives exactly as long as some reference still points at it, and a live
    /// entry keeps its operands (hence the keyed addresses) alive, so a dead
    /// entry is the only way an address can be reused and it fails to upgrade.
    /// Cleared with the program type caches.
    static DEFERRED_MERGES: std::cell::RefCell<
        surge_ts_types::fx::FxHashMap<DeferredMergeKey, std::sync::Weak<LazyIntersectionMerge>>,
    > = std::cell::RefCell::new(surge_ts_types::fx::FxHashMap::default());
}

thread_local! {
    static MERGES_IN_PROGRESS: std::cell::RefCell<Vec<usize>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(PartialEq, Eq, Hash)]
struct DeferredMergeKey {
    resolvers: Vec<usize>,
    dropped_unmodelled_operand: bool,
}

const DEFERRED_MERGES_PRUNE_INTERVAL: usize = 4096;

pub(crate) fn clear_deferred_merges() {
    DEFERRED_MERGES.with(|merges| merges.borrow_mut().clear());
}

/// The resolver for merging `members` (all references) later, shared with every
/// earlier deferred intersection of the same operand instances.
fn deferred_merge(
    members: Vec<Type>,
    dropped_unmodelled_operand: bool,
) -> std::sync::Arc<LazyIntersectionMerge> {
    let key = DeferredMergeKey {
        resolvers: members
            .iter()
            .filter_map(|ty| match ty {
                Type::Reference(reference) => Some(reference.resolver_address()),
                _ => None,
            })
            .collect(),
        dropped_unmodelled_operand,
    };
    DEFERRED_MERGES.with(|merges| {
        let mut merges = merges.borrow_mut();
        if let Some(shared) = merges.get(&key).and_then(std::sync::Weak::upgrade) {
            crate::program::record_program_counter(|c| c.lazy_intersection_share_count += 1);
            return shared;
        }
        if merges.len() % DEFERRED_MERGES_PRUNE_INTERVAL == DEFERRED_MERGES_PRUNE_INTERVAL - 1 {
            merges.retain(|_, weak| weak.strong_count() > 0);
        }
        let resolver = std::sync::Arc::new(LazyIntersectionMerge {
            members,
            dropped_unmodelled_operand,
            memo: std::sync::OnceLock::new(),
        });
        merges.insert(key, std::sync::Arc::downgrade(&resolver));
        resolver
    })
}

/// Resolver for a deferred all-reference intersection: the member peel + merge
/// runs on first consumer peel instead of at resolution time.
struct LazyIntersectionMerge {
    members: Vec<Type>,
    dropped_unmodelled_operand: bool,
    memo: std::sync::OnceLock<std::sync::Arc<Type>>,
}

impl surge_ts_types::ResolveReference for LazyIntersectionMerge {
    fn resolve(&self) -> Type {
        (*self.resolve_arc()).clone()
    }

    fn resolve_arc(&self) -> std::sync::Arc<Type> {
        if let Some(merged) = self.memo.get() {
            return merged.clone();
        }
        // A member can reach this same intersection again while it is being
        // merged (`Readonly<Omit<X, K>>` over an operand whose members name the
        // intersection). Initializing the memo re-entrantly would block on the
        // `OnceLock` forever, so the back-edge reads the sentinel like a blocked
        // lazy peel, and anything that consumed it is not memoized.
        let address = self as *const Self as usize;
        if MERGES_IN_PROGRESS.with(|merges| merges.borrow().contains(&address)) {
            crate::program::note_expansion_degradation();
            crate::program::record_degraded_resolution();
            crate::infer::types::cache::note_in_flight_degraded_read();
            return std::sync::Arc::new(Type::Unknown);
        }
        crate::program::record_program_counter(|c| c.lazy_intersection_peel_count += 1);
        let in_flight_before = crate::infer::types::cache::in_flight_degraded_read_epoch();
        MERGES_IN_PROGRESS.with(|merges| merges.borrow_mut().push(address));
        let display_name = (!self.members.is_empty()).then(|| {
            self.members
                .iter()
                .map(intersection_operand_name)
                .collect::<Vec<_>>()
                .join(" & ")
        });
        let merged = std::sync::Arc::new(crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::IntersectionMerge,
            || {
                merge_intersection_members_now(
                    self.members.clone(),
                    display_name,
                    self.dropped_unmodelled_operand,
                )
            },
        ));
        MERGES_IN_PROGRESS.with(|merges| {
            let mut merges = merges.borrow_mut();
            if let Some(position) = merges.iter().rposition(|entry| *entry == address) {
                merges.remove(position);
            }
        });
        if crate::infer::types::cache::in_flight_degraded_read_epoch() != in_flight_before {
            return merged;
        }
        self.memo.get_or_init(|| merged).clone()
    }
}

fn merge_intersection_members_now(
    members: Vec<Type>,
    display_name: Option<String>,
    dropped_unmodelled_operand: bool,
) -> Type {
    // A type that names itself through one of its own properties re-enters the
    // property merge below with the operands it is already merging. `Window &
    // typeof globalThis` alternates `window`/`self` with a period of two; left
    // alone the walk never terminates, and bounding it by depth alone still
    // fans out exponentially (two recursive properties per level, each level
    // re-merging ~1000 members) — that shape peaked at 55 GB RSS.
    let _frame = match MergeStack::enter(&members) {
        MergeEntry::Frame(frame) => frame,
        MergeEntry::Cycle => return cyclic_merge_result(members),
        MergeEntry::DepthExceeded => return open_merge_fallback(members),
    };

    // Also the primitive operands, which the merge below drops from the
    // surface but assignability still relates through, and the `object`
    // keyword, whose presence keeps the intersection from being weak.
    let nominal_operands: Vec<Type> = members
        .iter()
        .filter(|member| {
            matches!(member, Type::Object(object)
                if object.non_primitive && object.properties.is_empty() && !object.is_intersection)
                || matches!(
                member,
                Type::Reference(_)
                    | Type::Array(_)
                    | Type::Tuple(_)
                    | Type::String
                    | Type::Number
                    | Type::Boolean
                    | Type::BigInt
                    | Type::Symbol
                    | Type::StringLiteral(_)
                    | Type::NumberLiteral(_)
                    | Type::BooleanLiteral(_)
            )
        })
        .cloned()
        .collect();
    // `typeof globalThis`'s members are variables, which do not take part in
    // an intersection property's optionality (`createUnionOrIntersectionProperty`
    // reads it off class members only): `Window & typeof globalThis` keeps
    // Window's optional `ontouchstart` optional beside the global variable.
    let global_object_operands: Vec<bool> = members
        .iter()
        .map(|member| {
            matches!(member, Type::Reference(reference) if &*reference.id == crate::driver::GLOBAL_THIS_REFERENCE_ID)
        })
        .collect();
    // Peel reference operands (`StudentBulkImportRow & { … }`) so a named object
    // member contributes its properties to the merged intersection surface.
    let members: Vec<Type> = members.iter().map(Type::peeled).collect();

    // A union operand the caller could not distribute contributes no properties
    // to the merge below, so the result must stay OPEN or every use of one of its
    // members reads as an excess property. The caller's check runs *before* this
    // peel, so it misses a union hidden behind a nominal reference — zod's
    // `ZodIssue = ZodIssueOptionalMessage & { fatal?; message }` merged down to
    // `{ fatal, message }` and made `path` a false TS2353.
    let undistributed_union_operand = members.iter().any(|ty| matches!(ty, Type::Union(_)));
    // So does a reference operand that peels to the degradation sentinel: the
    // caller saw a reference and could not know it would fail to expand.
    let peeled_to_sentinel = members.iter().any(|ty| matches!(ty, Type::Unknown));
    let dropped_unmodelled_operand =
        dropped_unmodelled_operand || undistributed_union_operand || peeled_to_sentinel;

    // A survivor with an index signature of its own keeps it, and is marked
    // open only beside a dropped sentinel: `Partial<Record<K, V>>` next to a
    // `SafeExtendShape<S, U>` whose `U` surge could not bind is not the whole
    // intersection either, but beside a type variable it is read as declared
    // (TS4111 still names its index-signature members).
    let dropped_sentinel = members.iter().any(|ty| matches!(ty, Type::Unknown));
    let open_if_unmodelled = |ty: Type| -> Type {
        match ty {
            Type::Object(object)
                if dropped_unmodelled_operand && (object.string_index_type.is_none() || dropped_sentinel) =>
            {
                let mut object = object.with_open_index_marker();
                if object.string_index_type.is_none() {
                    object.string_index_type = Some(std::sync::Arc::new(Type::Any));
                }
                Type::Object(object)
            }
            other => other,
        }
    };

    let object_members: Vec<_> = members
        .iter()
        .filter_map(|ty| match ty {
            Type::Object(object) => Some(object),
            _ => None,
        })
        .collect();
    let object_members_from_global_object: Vec<bool> = members
        .iter()
        .zip(&global_object_operands)
        .filter(|(ty, _)| matches!(ty, Type::Object(_)))
        .map(|(_, from_global_object)| *from_global_object)
        .collect();

    // An object operand excludes `null`/`undefined`, so `undefined & {}` is
    // `never`. That reduction is what makes `NonNullable<T>` — defined as
    // `T & {}` — actually drop the nullish arms once the union is distributed;
    // without it the brand-collapse below handed `undefined` back and every
    // `NonNullable<…>` kept its `| undefined`.
    //
    // `void` is deliberately excluded even though tsc reduces it the same way:
    // surge models `PromiseLike<void>` as its awaited `void`, so
    // `PromiseLike<void> & { pull(): void }` reaches here as `void & {…}` and
    // reducing it to `never` would strip the contextual type off every method
    // written against it. Skipped, too, when an operand was dropped as
    // unmodelled: the surviving object may not be the whole story.
    if !dropped_unmodelled_operand
        && !object_members.is_empty()
        && members.iter().any(|ty| matches!(ty, Type::Undefined | Type::Null))
    {
        return Type::Never;
    }

    // tsc's `getIntersectionTypeEx` empties an intersection holding `never` or
    // operands from two disjoint primitive domains. Without it a property
    // declared `string | undefined` on one operand and `string` on another
    // distributed to `string | undefined` (`undefined & string` kept its first
    // operand) instead of `string`. A dropped unmodelled operand cannot make a
    // disjoint pair inhabited, so this holds regardless.
    if members.iter().any(|ty| matches!(ty, Type::Never))
        || has_disjoint_primitive_domains(&members)
    {
        return Type::Never;
    }

    // Brand idiom: `string & { _?: never }` (and other `Base & {…all-optional…}`
    // shapes, e.g. `LiteralUnion<L, B> = L | (B & { _?: never })`). When every
    // object operand only contributes optional members, the object side is a
    // phantom "brand" and the intersection is structurally just the non-object
    // side — tsc treats `string & {}` as assignable both to and from `string`.
    // Collapsing to the non-object member keeps that bidirectional behavior;
    // falling through to the object-merge below would keep only `{ _?: never }`
    // and wrongly reject `(string & brand) → string`.
    if !object_members.is_empty()
        && object_members
            .iter()
            .all(|object| is_brand_like_object(object))
    {
        let mut non_object = members.iter().filter(|ty| !matches!(ty, Type::Object(_)));
        if let Some(first) = non_object.next() {
            if non_object.next().is_none() {
                return first.clone();
            }
        }
    }

    if !object_members.is_empty() {
        let mut properties: PropertyMap = PropertyMap::default();
        let mut string_index_type: Option<Type> = None;
        // An operand that was itself opened by a nested merge contributes an
        // index the author never wrote; re-merging must not launder it into a
        // declared one, or `noPropertyAccessFromIndexSignature` fires on the
        // outer surface instead.
        let mut string_index_is_synthetic = false;
        // `resolveIntersectionTypeMembers` keeps every operand's index infos, so
        // a number index any operand declares answers for the intersection
        // (`Window & typeof globalThis` reads `window["x"]` as TS7015).
        let mut number_index_type: Option<std::sync::Arc<Type>> = None;
        // A callable operand (`F & { … }`, or an interface with a call signature)
        // keeps the merged intersection callable. An intersection of *several*
        // function types is TypeScript's overload spelling (execa's
        // `ExecaMethod` is four call signatures intersected), so they fold into
        // one permissive signature rather than the first winning — otherwise
        // every call matching a later overload is a false TS2554.
        let mut call_signature = members
            .iter()
            .filter_map(|ty| match ty {
                Type::Function(function_type) => Some(function_type.clone()),
                _ => None,
            })
            .reduce(|merged, function_type| {
                crate::infer::types::interface::merge_overload_signatures(&merged, &function_type)
            })
            .map(std::sync::Arc::new);
        let mut construct_signature: Option<std::sync::Arc<surge_ts_types::FunctionType>> = None;
        // checker.go `isDiscriminantWithNeverType`: a required property two
        // operands declare with differing types, one with a literal part and
        // none `never`, whose types intersect to `never` makes the whole
        // intersection `never` (`getReducedType`). Each operand contributes its
        // property's read type (`getTypeOfSymbol`), which for an optional one
        // includes `undefined`.
        let mut shared_property_types: Vec<(std::sync::Arc<str>, Vec<Type>)> = Vec::new();
        let read_type = |property: &surge_ts_types::ObjectProperty| {
            if property.is_optional()
                && surge_ts_types::strict_null_checks()
                && !surge_ts_types::exact_optional_property_types()
            {
                surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
            } else {
                property.ty.clone()
            }
        };

        let mut variable_only_properties: std::collections::HashSet<std::sync::Arc<str>> =
            std::collections::HashSet::new();
        // checker.go `isConflictingPrivateProperty`: a property two class
        // declarations contribute, one of them `private`, reduces the whole
        // intersection to `never`.
        let mut conflicting_private = false;
        for (object, from_global_object) in object_members.iter().zip(&object_members_from_global_object) {
            for (name, property) in object.properties.iter() {
                if let Some(existing) = properties.get(name) {
                    let private = |restriction: &Option<surge_ts_types::MemberRestriction>| {
                        restriction.as_ref().is_some_and(|restriction| restriction.private)
                    };
                    let same_owner = matches!(
                        (&existing.restriction, &property.restriction),
                        (Some(left), Some(right)) if left.owner == right.owner
                    );
                    conflicting_private |=
                        (private(&existing.restriction) || private(&property.restriction)) && !same_owner;
                }
                // A property declared by more than one operand is the
                // *intersection* of what they declare, not the first one:
                // `Row & { identity: Code }` narrows `identity` from
                // `Code | NameOnly` down to `Code`. Keeping the first made every
                // type-predicate narrowing through an intersection read the
                // unnarrowed union.
                match properties.get(name) {
                    Some(existing) => {
                        match shared_property_types.iter_mut().find(|(shared, _)| shared == name) {
                            Some((_, types)) => types.push(read_type(property)),
                            None => shared_property_types
                                .push((name.clone(), vec![read_type(existing), read_type(property)])),
                        }
                        let existing_is_variable = variable_only_properties.contains(name);
                        let optional = match (existing_is_variable, *from_global_object) {
                            (true, true) => existing.is_optional() && property.is_optional(),
                            (true, false) => property.is_optional(),
                            (false, true) => existing.is_optional(),
                            (false, false) => existing.is_optional() && property.is_optional(),
                        };
                        if existing_is_variable && !*from_global_object {
                            variable_only_properties.remove(name);
                        }
                        let merged_property = surge_ts_types::ObjectProperty {
                            ty: merge_intersection_members(vec![
                                existing.ty.clone(),
                                property.ty.clone(),
                            ]),
                            optional,
                            method: existing.method,
                            readonly: false,
                            restriction: existing.restriction.clone(),
                            index_slot: existing.index_slot,
                            written_name: existing.written_name.clone(),
                        };
                        properties.insert(name.clone(), merged_property);
                    }
                    None => {
                        if *from_global_object {
                            variable_only_properties.insert(name.clone());
                        }
                        properties.insert(name.clone(), property.clone());
                    }
                }
            }
            if string_index_type.is_none()
                && let Some(index) = object.string_index_type.as_deref()
            {
                string_index_type = Some(index.clone());
                string_index_is_synthetic = object.synthetic_open_index;
            }
            if number_index_type.is_none() {
                number_index_type = object.number_index_type.clone();
            }
            if let Some(object_call_signature) = object.call_signature.as_deref() {
                call_signature = Some(match call_signature.take() {
                    Some(existing) => std::sync::Arc::new(
                        crate::infer::types::interface::merge_overload_signatures(
                            &existing,
                            object_call_signature,
                        ),
                    ),
                    None => std::sync::Arc::new(object_call_signature.clone()),
                });
            }
        }
        if conflicting_private {
            return Type::Never;
        }
        if shared_property_types.into_iter().any(|(name, types)| {
            properties.get(&name).is_some_and(|property| !property.is_optional())
                && types.iter().any(has_literal_part)
                && types.iter().any(|ty| *ty != types[0])
                && !types.iter().any(|ty| matches!(ty, Type::Never))
                && matches!(merge_intersection_members(types), Type::Never)
        }) {
            return Type::Never;
        }
        let constructors: Vec<&surge_ts_types::FunctionType> = object_members
            .iter()
            .filter_map(|object| object.construct_signature.as_deref())
            .collect();
        if let Some(mixed) = mixin_construct_signature(&constructors) {
            construct_signature = Some(std::sync::Arc::new(mixed));
        } else if let Some(first) = constructors.first() {
            construct_signature = Some(std::sync::Arc::new((*first).clone()));
        }

        // An array operand indexes the value by number (`T[] & { … }`), and
        // so does a tuple's, by the union of its elements (`LuaTuple<T> = T &
        // { readonly LUA_TUPLE: never }`).
        if number_index_type.is_none()
            && let Some(element) = nominal_operands.iter().find_map(|operand| match operand.peeled() {
                Type::Array(element) => Some(*element),
                Type::Tuple(elements) if !elements.is_empty() => Some(surge_ts_types::union_type(elements.to_vec())),
                Type::OpenTuple(tuple) => Some(tuple.element_union()),
                _ => None,
            })
        {
            number_index_type = Some(std::sync::Arc::new(element));
        }
        // The dropped operand may add to a member too: `{ passkey: Actions } &
        // InferClientAPI<O>` with the API unmodelled reads `passkey` as
        // `Actions & …`, so an inline object member is opened like the surface.
        if dropped_unmodelled_operand {
            for property in properties.values_mut() {
                if let Some(opened) = opened_object_member(&property.ty) {
                    property.ty = opened;
                }
            }
        }
        let mut merged = alloc_object_type(properties, string_index_type)
            .with_intersection_marker()
            .with_intersection_operands(nominal_operands.clone());
        if string_index_is_synthetic {
            merged = merged.with_open_index_marker();
        }
        if let Some(display_name) = display_name {
            merged = merged.with_alias_name(display_name);
        }
        merged.call_signature = call_signature;
        merged.construct_signature = construct_signature;
        merged.number_index_type = number_index_type;
        return open_if_unmodelled(Type::Object(merged));
    }

    // An intersection of *only* function types is TypeScript's overload
    // spelling (execa's `ExecaMethod` intersects four call signatures). Folding
    // them into one permissive signature keeps every overload's arity callable;
    // taking the first made calls matching a later one a false TS2554.
    let mut function_members = members.iter().filter_map(|ty| match ty {
        Type::Function(function_type) => Some(function_type),
        _ => None,
    });
    if let (Some(first), Some(second)) = (function_members.next(), function_members.next()) {
        let merged = function_members.fold(
            crate::infer::types::interface::merge_overload_signatures(first, second),
            |merged, function_type| {
                crate::infer::types::interface::merge_overload_signatures(&merged, function_type)
            },
        );
        // Keep the operands in written order alongside the fold. The fold is what
        // stays callable at every arity, but inference has to see the individual
        // signatures: tsc infers from the *last* one, which is what makes
        // `UnionToTuple` peel a union one member at a time.
        let operands = members
            .iter()
            .filter_map(|ty| match ty {
                Type::Function(function_type) => Some(function_type.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        return Type::Function(merged.with_overloads(operands));
    }

    // Two distinct literals have no common inhabitant, so their intersection is
    // `never`. Without this the tail was first-operand-wins, which inverts a
    // guard written as `keyof A & keyof B extends never ? … : …` — trpc's
    // `ProtectedIntersection` — because the disjoint key sets reduced to the
    // first operand instead of `never`. Only literal operands participate;
    // full `string & number -> never` reduction stays a non-goal, as above.
    if let Some(reduced) = reduce_disjoint_literals(&members) {
        return reduced;
    }

    match members.into_iter().next() {
        Some(member) => member,
        None => Type::Unknown,
    }
}

/// checker.go `CheckFlagsHasLiteralPart`: `isLiteralType` (a unit type,
/// `boolean`, or a union of units) or a unique symbol.
fn has_literal_part(ty: &Type) -> bool {
    fn is_unit(ty: &Type) -> bool {
        match ty {
            Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Undefined
            | Type::Null
            | Type::Void => true,
            Type::Reference(reference) => {
                reference.is_unique_symbol() || (reference.enum_owner.is_some() && is_unit(&reference.resolve()))
            }
            _ => false,
        }
    }
    match ty {
        Type::Boolean => true,
        Type::Union(union) => union.types().iter().all(is_unit),
        other => is_unit(other),
    }
}

/// tsc's `TypeFlagsDisjointDomains` partition (types.go:494), for the operands
/// surge models as primitives. Object types belong to no domain: `string & {…}`
/// is a brand, not `never`.
fn primitive_domain(ty: &Type) -> Option<u8> {
    match ty {
        Type::String | Type::StringLiteral(_) => Some(0),
        Type::Number | Type::NumberLiteral(_) => Some(1),
        Type::BigInt => Some(2),
        Type::Boolean | Type::BooleanLiteral(_) => Some(3),
        Type::Symbol => Some(4),
        Type::Void | Type::Undefined => Some(5),
        // The `object` keyword (`TypeFlagsNonPrimitive`).
        Type::Object(object) if object.non_primitive && !object.is_intersection => Some(6),
        _ => None,
    }
}

fn has_disjoint_primitive_domains(members: &[Type]) -> bool {
    let mut domains = members.iter().filter_map(primitive_domain);
    domains
        .next()
        .is_some_and(|first| domains.any(|domain| domain != first))
}

/// The set of literals an operand admits, when the operand is a scalar literal,
/// a union of scalar literals, or the primitive that spells one of their
/// domains. `None` for anything else, which leaves the caller's existing
/// behavior alone.
enum LiteralDomain<'a> {
    /// Every literal of one primitive domain (`string`, `number`, …).
    Primitive(&'a Type),
    Members(Vec<&'a Type>),
}

fn literal_primitive_domain(literal: &Type) -> Option<Type> {
    match literal {
        Type::StringLiteral(_) => Some(Type::String),
        Type::NumberLiteral(_) => Some(Type::Number),
        Type::BooleanLiteral(_) => Some(Type::Boolean),
        _ => None,
    }
}

fn literal_domain(ty: &Type) -> Option<LiteralDomain<'_>> {
    match ty {
        Type::String | Type::Number | Type::Boolean => Some(LiteralDomain::Primitive(ty)),
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => {
            Some(LiteralDomain::Members(vec![ty]))
        }
        Type::Union(union) => {
            let mut members = Vec::with_capacity(union.types().len());
            for member in union.types() {
                match member {
                    Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => {
                        members.push(member);
                    }
                    _ => return None,
                }
            }
            (!members.is_empty()).then_some(LiteralDomain::Members(members))
        }
        _ => None,
    }
}

/// `Some` when every operand is literal-like: the set intersection of their
/// admitted literals, `never` when that is empty. Without this a `keyof A &
/// keyof B` guard — trpc's `ProtectedIntersection`, whose disjoint key sets must
/// reduce to `never` — took the first operand and inverted the conditional. Two
/// union operands never reach the distribution path above (its product is
/// quadratic), so the reduction has to happen here.
fn reduce_disjoint_literals(members: &[Type]) -> Option<Type> {
    let mut domains = members.iter().map(literal_domain);
    let mut reduced = domains.next()??;

    for domain in domains {
        reduced = match (reduced, domain?) {
            (LiteralDomain::Primitive(left), LiteralDomain::Primitive(right)) => {
                if left == right {
                    LiteralDomain::Primitive(left)
                } else {
                    return Some(Type::Never);
                }
            }
            (LiteralDomain::Primitive(primitive), LiteralDomain::Members(members))
            | (LiteralDomain::Members(members), LiteralDomain::Primitive(primitive)) => {
                LiteralDomain::Members(
                    members
                        .into_iter()
                        .filter(|member| {
                            literal_primitive_domain(member).as_ref() == Some(primitive)
                        })
                        .collect(),
                )
            }
            (LiteralDomain::Members(left), LiteralDomain::Members(right)) => {
                LiteralDomain::Members(
                    left.into_iter()
                        .filter(|member| right.contains(member))
                        .collect(),
                )
            }
        };
        if matches!(&reduced, LiteralDomain::Members(members) if members.is_empty()) {
            return Some(Type::Never);
        }
    }

    Some(match reduced {
        LiteralDomain::Primitive(primitive) => primitive.clone(),
        LiteralDomain::Members(members) => {
            surge_ts_types::union_type(members.into_iter().cloned().collect())
        }
    })
}

/// Whether an object contributes no required structure to an intersection — all
/// properties optional, no index signature, no call/construct signature. Such an
/// operand is a phantom "brand" (`{ _?: never }`), so `Base & brand` is
/// structurally just `Base`.
fn is_brand_like_object(object: &ObjectType) -> bool {
    object.string_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
        && object
            .properties
            .values()
            .all(|property| property.is_optional() && is_phantom_member_type(&property.ty))
}

/// A brand's member carries no payload: `never` (`{ _?: never }`) or an empty
/// object (`WithRequired<T, K> = T & { [_ in K]: {} }`). A member with a real
/// type is one the value is asked for — next's `AppType` is
/// `ComponentType<P> & { getInitialProps?(context): IP | Promise<IP> }`, and
/// collapsing that away made every `MyApp.getInitialProps = …` a false TS2339
/// on a type whose own display still showed the member.
fn is_phantom_member_type(ty: &Type) -> bool {
    match ty {
        Type::Never => true,
        Type::Object(object) => {
            object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
        }
        _ => false,
    }
}

/// An intersection operand as tsc prints it: a function or union type is
/// parenthesized, since `() => void & T` would read as a function returning
/// the intersection.
pub(crate) fn intersection_operand_name(ty: &Type) -> String {
    let name = ty.name();
    match ty {
        // A signature an alias names (`FnAlias<P>`) prints as the alias.
        Type::Function(_) if name.starts_with('(') || name.starts_with('<') || name.starts_with("new ") => format!("({name})"),
        // A named union prints as its alias and needs no parentheses.
        Type::Union(_) if name.contains(" | ") => format!("({name})"),
        _ => name,
    }
}

/// tsc's `isMixinConstructorType`: one construct signature taking only
/// `...args: any[]`.
fn is_mixin_constructor(signature: &surge_ts_types::FunctionType) -> bool {
    signature.overloads().is_none()
        && signature.type_parameter_head().is_none()
        && signature.is_variadic()
        && matches!(
            signature.parameters(),
            [Type::Any] | [Type::Array(_)]
        )
        && match signature.parameters() {
            [Type::Array(element)] => matches!(element.as_ref(), Type::Any),
            _ => true,
        }
}

/// The construct signature of an intersection that mixes constructors in
/// (tsc's `resolveIntersectionTypeMembers`): what the first constituent that is
/// not a mixin takes, returning its own instance type intersected with every
/// mixin's. With nothing but mixins the first one stands in for it. `None`
/// when no constituent is a mixin.
fn mixin_construct_signature(
    constructors: &[&surge_ts_types::FunctionType],
) -> Option<surge_ts_types::FunctionType> {
    let mut is_mixin: Vec<bool> = constructors
        .iter()
        .map(|signature| is_mixin_constructor(signature))
        .collect();
    if constructors.len() < 2 || !is_mixin.contains(&true) {
        return None;
    }
    if is_mixin.iter().all(|mixin| *mixin) {
        is_mixin[0] = false;
    }
    let base_index = is_mixin.iter().position(|mixin| !mixin)?;
    let base = constructors[base_index];
    let with_mixins = |own_return: &Type| -> Type {
        let mut instances: Vec<Type> = Vec::with_capacity(constructors.len());
        for (index, signature) in constructors.iter().enumerate() {
            if index == base_index {
                instances.push(own_return.clone());
            } else if is_mixin[index] {
                instances.push(signature.return_type().clone());
            }
        }
        merge_intersection_members(instances)
    };
    let rebuild = |signature: &surge_ts_types::FunctionType| {
        // `cloneSignature` keeps the base's flags: an abstract base stays one.
        let rebuilt = alloc_function_type(
            signature.parameters().to_vec(),
            with_mixins(signature.return_type()),
            signature.is_variadic(),
            signature.required_parameter_count(),
        )
        .with_type_parameter_head(signature.type_parameter_head().map(str::to_string))
        .with_construct_modifiers(signature.construct_modifiers());
        match signature.parameter_names() {
            Some(names) => rebuilt.with_parameter_names(names.to_vec()),
            None => rebuilt,
        }
    };
    let mixed = rebuild(base);
    Some(match base.overloads() {
        Some(overloads) => mixed.with_overloads(overloads.iter().map(rebuild).collect()),
        None => mixed,
    })
}

fn opened_object_member(ty: &Type) -> Option<Type> {
    let Type::Object(object) = ty else {
        return None;
    };
    if object.properties.is_empty()
        || object.string_index_type.is_some()
        || object.synthetic_open_index
        || object.call_signature().is_some()
        || object.construct_signature().is_some()
    {
        return None;
    }
    let mut object = object.clone().with_open_index_marker();
    object.string_index_type = Some(std::sync::Arc::new(Type::Any));
    Some(Type::Object(object))
}
