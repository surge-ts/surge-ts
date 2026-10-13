use std::fmt;
use std::sync::Arc;

use crate::Type;

/// Lazily computes the structural expansion of a [`TypeReference`].
///
/// `surge-ts-types` is a leaf crate with no access to the checker's declaration
/// table, so the checker installs the concrete resolver behind this trait.
/// Implementations are expected to memoize program-wide (one structural
/// instantiation per unique declaration + type-argument tuple) so repeated
/// `resolve` calls are cheap.
/// Census-only estimate of the state a lazy resolver captures. `own_bytes` is
/// the resolver's uniquely-owned heap (parsed annotations, resolved argument
/// vectors, key strings); `shared_captures` lists `(address, bytes)` pairs for
/// `Arc`-shared captures (substitution maps) so a walker can charge each shared
/// payload once.
#[derive(Debug, Default)]
pub struct ResolverCaptureCensus {
    pub own_bytes: u64,
    pub shared_captures: Vec<(usize, u64)>,
}

pub trait ResolveReference: Send + Sync {
    fn resolve(&self) -> Type;

    fn retains_resolution_context(&self) -> bool {
        false
    }

    /// Census-only capture estimate; see [`ResolverCaptureCensus`]. The default
    /// covers resolvers with no interesting captured state.
    fn captured_census(&self) -> ResolverCaptureCensus {
        ResolverCaptureCensus::default()
    }

    /// The memoized structural expansion, if one already exists, without
    /// forcing resolution. Census walks use this to reach retained expansion
    /// graphs; implementations must never compute a new expansion here.
    fn peek_resolved(&self) -> Option<Arc<Type>> {
        None
    }

    fn supports_program_canonicalization(&self) -> bool {
        true
    }

    fn program_canonicalization_discriminator(&self) -> u64 {
        0
    }

    /// Like [`resolve`](Self::resolve) but yields a shared `Arc<Type>`. Resolvers
    /// backed by a memoized/interned `Arc` (the lazy/interned instantiation
    /// resolvers) override this to hand back the shared pointer instead of
    /// deep-cloning the structural type. The assignability checker peels the same
    /// reference millions of times on conditional/mapped-type-heavy programs and
    /// only needs to *borrow* the resolved shape, so avoiding the per-peel `Type`
    /// clone is a large win; the default keeps the old behaviour for resolvers
    /// that own their type by value.
    fn resolve_arc(&self) -> Arc<Type> {
        Arc::new(self.resolve())
    }
}

/// A lazy, nominal reference to a named type instantiation (`Box<string>`,
/// `User`, …), mirroring tsc's `TypeReference`. Construction is cheap: the
/// structural shape is computed on demand via [`ResolveReference::resolve`] and
/// memoized by the checker, instead of being eagerly expanded at every use site.
///
/// Equality is **nominal** — two references are equal when they come from the
/// same declaration ([`id`](Self::id)) with equal [`arguments`](Self::arguments).
/// The `display` string and the resolver identity are not part of equality, so a
/// reference never compares equal to its own structural expansion (which is the
/// correct nominal semantics) and same-instantiation references compare equal in
/// O(1) without forcing resolution.
#[derive(Clone)]
pub struct TypeReference {
    /// Nominal identity of the source declaration (qualified `file\0Name`).
    pub id: Arc<str>,
    /// Diagnostic display form, e.g. `Box<string>` or `User`.
    pub display: Arc<str>,
    /// Resolved type arguments, for variance-aware comparison and display.
    pub arguments: Arc<[Type]>,
    /// Render the resolved type structurally instead of by `display`. A
    /// conditional alias resolves *to* one of its branches, and tsc shows that
    /// branch's own type. Display-only and a pure function of the declaration,
    /// so it stays out of `nominal_eq` and out of canonical identity.
    pub render_structurally: bool,
    /// This reference is a *numeric* `enum` type (the enum itself or one of its
    /// member types). tsc lets any `number` flow into one — `Flags.A | Flags.B`
    /// is typed `number`, and passing it where `Flags` is expected is legal — so
    /// assignability needs to recognise the target. Display-only provenance, like
    /// `render_structurally`: it stays out of `nominal_eq` and canonical identity.
    pub numeric_enum: bool,
    /// The `enum` this reference is (or is a member type of), by declaring
    /// file and name. Enum types are nominal: a member of one enum never
    /// relates to another enum, whatever the values. Provenance like
    /// `numeric_enum`, outside `nominal_eq` and canonical identity.
    pub enum_owner: Option<Arc<str>>,
    /// The `enum` an enum member's literal type widens to (tsc's
    /// `getBaseTypeOfEnumLikeType`), when the member was resolved where its
    /// enum could be. Provenance like `enum_owner`, outside `nominal_eq`.
    pub enum_base: Option<Arc<Type>>,
    /// The members of the `enum` this reference is (or is a member type of),
    /// which `isEnumTypeRelatedTo` compares between two same-named enums.
    /// Provenance like `enum_owner`, outside `nominal_eq`.
    pub enum_members: Option<Arc<EnumMembers>>,
    /// The type alias this reference was written as the body of (`type NN =
    /// NumberTo<number>`), which tsc displays in its place. Display only.
    pub alias_display: Option<Arc<str>>,
    /// An interface or class with one base and no members of its own, which
    /// tsc relates as that base (`getSingleBaseForNonAugmentingSubtype`).
    /// Display only: it decides which type a report names.
    pub non_augmenting_subtype: bool,
    /// The variances the declaration states for its type parameters, packed
    /// by [`declared_variances`]. A pure function of the declaration, so it
    /// stays out of `nominal_eq` and canonical identity like `numeric_enum`.
    pub declared_variances: u32,
    resolver: Arc<dyn ResolveReference>,
}

/// An `enum`'s members for `isEnumTypeRelatedTo`.
#[derive(Debug, PartialEq, Eq)]
pub struct EnumMembers {
    /// A regular enum (`SymbolFlagsRegularEnum`): not a `const enum`.
    pub regular: bool,
    /// Each member's name and value, `None` for a computed one.
    pub members: Vec<(Arc<str>, Option<Type>)>,
}

/// Nominal id of the synthetic reference that carries the `readonly` array /
/// tuple modifier. The single argument is the mutable shape; resolving the
/// reference yields it, so every structural consumer that peels sees the
/// array, while assignability and identity can still tell the two apart.
pub const READONLY_REFERENCE_ID: &str = "\u{0}readonly";

/// Nominal id of the synthetic reference that carries a written tuple's
/// `minLength` where its element types cannot show it (see
/// [`crate::written_tuple_type`]). The arguments are the tuple and that length
/// as a number literal; resolving the reference yields the tuple.
pub const WRITTEN_TUPLE_REFERENCE_ID: &str = "\u{0}tuple";

/// Nominal id of the synthetic reference that is a class's polymorphic `this`
/// inside its own instance members (tsc's `thisType`, a type parameter
/// constrained to the instance). The single argument is the instance type;
/// resolving the reference yields it, so every structural consumer that
/// peels sees the instance, while a relation can tell `this` apart from it.
pub const POLYMORPHIC_THIS_REFERENCE_ID: &str = "\u{0}this";

impl TypeReference {
    /// Whether this is the synthetic `readonly T[]` / `readonly [A, B]` wrapper.
    pub fn is_readonly_array(&self) -> bool {
        &*self.id == READONLY_REFERENCE_ID
    }

    /// Whether this is a class's polymorphic `this` inside its members.
    pub fn is_polymorphic_this(&self) -> bool {
        &*self.id == POLYMORPHIC_THIS_REFERENCE_ID
    }

    /// The elements and `minLength` of a written tuple wrapper.
    pub fn written_tuple(&self) -> Option<(&[Type], usize)> {
        if &*self.id != WRITTEN_TUPLE_REFERENCE_ID {
            return None;
        }
        let [Type::Tuple(elements), Type::NumberLiteral(min_length)] = &*self.arguments else {
            return None;
        };
        Some((elements.as_slice(), min_length.value.parse().ok()?))
    }

    /// A `unique symbol` declared by some `const` (see [`unique_symbol_type`]).
    pub fn is_unique_symbol(&self) -> bool {
        self.id.starts_with(UNIQUE_SYMBOL_ID_PREFIX)
    }

    /// The declaration name of a unique symbol (`s` in `const s = Symbol()`).
    pub fn unique_symbol_name(&self) -> Option<&str> {
        self.id.strip_prefix(UNIQUE_SYMBOL_ID_PREFIX)?.rsplit('\u{0}').next()
    }

    pub fn new(
        id: impl Into<Arc<str>>,
        display: impl Into<Arc<str>>,
        arguments: impl Into<Arc<[Type]>>,
        resolver: Arc<dyn ResolveReference>,
    ) -> Self {
        Self {
            id: id.into(),
            display: crate::name_memo::bounded_display(display.into()),
            arguments: arguments.into(),
            render_structurally: false,
            numeric_enum: false,
            enum_owner: None,
            enum_base: None,
            enum_members: None,
            alias_display: None,
            non_augmenting_subtype: false,
            declared_variances: 0,
            resolver,
        }
    }


    /// Marks this reference as rendering its resolved type structurally.
    pub fn rendered_structurally(mut self) -> Self {
        self.render_structurally = true;
        self
    }

    /// Marks this reference as a numeric `enum` type.
    pub fn numeric_enum(mut self) -> Self {
        self.numeric_enum = true;
        self
    }

    /// Marks this reference as the `enum` `owner`, or one of its member types.
    pub fn with_enum_owner(mut self, owner: Arc<str>) -> Self {
        self.enum_owner = Some(owner);
        self
    }

    /// Records the `enum` this member type widens to.
    pub fn with_enum_base(mut self, base: Type) -> Self {
        self.enum_base = Some(Arc::new(base));
        self
    }

    /// Records the members of this reference's `enum`.
    pub fn with_enum_members(mut self, members: Arc<EnumMembers>) -> Self {
        self.enum_members = Some(members);
        self
    }

    /// Records the type alias this reference is displayed as.
    pub fn with_alias_display(mut self, display: impl Into<Arc<str>>) -> Self {
        self.alias_display = Some(crate::name_memo::bounded_display(display.into()));
        self
    }

    /// Marks this reference as a non-augmenting subtype of its single base.
    pub fn as_non_augmenting_subtype(mut self) -> Self {
        self.non_augmenting_subtype = true;
        self
    }

    /// Records the variances the declaration states.
    pub fn with_declared_variances(mut self, variances: u32) -> Self {
        self.declared_variances = variances;
        self
    }

    /// Computes (or returns the memoized) structural expansion of this reference.
    pub fn resolve(&self) -> Type {
        self.resolver.resolve()
    }

    /// The structural expansion as a shared `Arc<Type>`, avoiding a deep clone
    /// when the underlying resolver is `Arc`-backed. See
    /// [`ResolveReference::resolve_arc`].
    pub fn resolve_arc(&self) -> Arc<Type> {
        self.resolver.resolve_arc()
    }

    pub fn retains_resolution_context(&self) -> bool {
        self.resolver.retains_resolution_context()
    }

    pub fn supports_program_canonicalization(&self) -> bool {
        self.resolver.supports_program_canonicalization()
    }

    pub fn program_canonicalization_discriminator(&self) -> u64 {
        self.resolver.program_canonicalization_discriminator()
    }

    pub fn resolver_address(&self) -> usize {
        Arc::as_ptr(&self.resolver) as *const () as usize
    }

    /// Census-only capture estimate of the resolver. See
    /// [`ResolveReference::captured_census`].
    pub fn captured_census(&self) -> ResolverCaptureCensus {
        self.resolver.captured_census()
    }

    /// The already-memoized expansion, if any, without forcing resolution.
    pub fn peek_resolved(&self) -> Option<Arc<Type>> {
        self.resolver.peek_resolved()
    }

    /// Nominal identity test: same declaration and same type arguments.
    pub fn nominal_eq(&self, other: &Self) -> bool {
        // Interned references share one id allocation, and ids are long
        // qualified strings with a common file-path prefix — the byte compare
        // never short-circuits early, so the pointer check is the whole win.
        let same_id = Arc::ptr_eq(&self.id, &other.id) || self.id == other.id;
        same_id && self.arguments == other.arguments
    }
}

/// What a declaration states about a type parameter's variance, which the
/// relation then need not measure (`getVariancesWorker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredVariance {
    /// `out`.
    Covariant,
    /// `in`.
    Contravariant,
    /// `in out`.
    Invariant,
    /// Unannotated, but measuring it reports `Unmeasurable`: only identical
    /// arguments relate without a structural comparison.
    Unmeasurable,
    /// Unannotated, measured contravariant with `VarianceFlagsUnreliable`: a
    /// failed argument check still falls back to the structural comparison.
    UnreliableContravariant,
    /// Unannotated, read contravariant (invariant) off the declaration's
    /// written shape: arguments surge could not model still relate, and a
    /// failed check falls back to the structural comparison.
    MeasuredContravariant,
    MeasuredInvariant,
}

impl DeclaredVariance {
    /// The variance `in`/`out` modifiers annotate.
    pub fn annotated(is_in: bool, is_out: bool) -> Option<Self> {
        match (is_in, is_out) {
            (true, true) => Some(Self::Invariant),
            (false, true) => Some(Self::Covariant),
            (true, false) => Some(Self::Contravariant),
            (false, false) => None,
        }
    }
}

/// Parameters past this many read as stating nothing.
const DECLARED_VARIANCE_LIMIT: usize = 10;
const DECLARED_VARIANCE_BITS: usize = 3;

/// Packs each type parameter's declared variance, in declaration order.
pub fn declared_variances(variances: impl IntoIterator<Item = Option<DeclaredVariance>>) -> u32 {
    variances
        .into_iter()
        .take(DECLARED_VARIANCE_LIMIT)
        .enumerate()
        .fold(0, |packed, (index, variance)| {
            let code: u32 = match variance {
                None => 0,
                Some(DeclaredVariance::Covariant) => 1,
                Some(DeclaredVariance::Contravariant) => 2,
                Some(DeclaredVariance::Invariant) => 3,
                Some(DeclaredVariance::Unmeasurable) => 4,
                Some(DeclaredVariance::UnreliableContravariant) => 5,
                Some(DeclaredVariance::MeasuredContravariant) => 6,
                Some(DeclaredVariance::MeasuredInvariant) => 7,
            };
            packed | (code << (index * DECLARED_VARIANCE_BITS))
        })
}

/// The variance `packed` declares for the parameter at `index`.
pub fn declared_variance(packed: u32, index: usize) -> Option<DeclaredVariance> {
    if index >= DECLARED_VARIANCE_LIMIT {
        return None;
    }
    match (packed >> (index * DECLARED_VARIANCE_BITS)) & 0b111 {
        1 => Some(DeclaredVariance::Covariant),
        2 => Some(DeclaredVariance::Contravariant),
        3 => Some(DeclaredVariance::Invariant),
        4 => Some(DeclaredVariance::Unmeasurable),
        5 => Some(DeclaredVariance::UnreliableContravariant),
        6 => Some(DeclaredVariance::MeasuredContravariant),
        7 => Some(DeclaredVariance::MeasuredInvariant),
        _ => None,
    }
}

impl fmt::Debug for TypeReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeReference")
            .field("id", &self.id)
            .field("display", &self.display)
            .field("arguments", &self.arguments)
            .finish_non_exhaustive()
    }
}

impl PartialEq for TypeReference {
    fn eq(&self, other: &Self) -> bool {
        self.nominal_eq(other)
    }
}

impl Eq for TypeReference {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::is_assignable_to;

    struct Fixed(Type);

    impl ResolveReference for Fixed {
        fn resolve(&self) -> Type {
            self.0.clone()
        }
    }

    fn reference(id: &str, display: &str, arguments: Vec<Type>, resolved: Type) -> Type {
        Type::Reference(TypeReference::new(
            id,
            display,
            arguments,
            Arc::new(Fixed(resolved)),
        ))
    }

    #[test]
    fn name_uses_display_not_structural_expansion() {
        let ty = reference(
            "box.ts\u{0}Box",
            "Box<string>",
            vec![Type::String],
            Type::String,
        );
        assert_eq!(ty.name(), "Box<string>");
    }

    #[test]
    fn oversized_displays_are_abbreviated_distinctly() {
        let shape = |member: &str| format!("ZodObject<{{ {} }}>", member.repeat(20_000));
        let a = reference("zod.ts\u{0}ZodObject", &shape("a: string; "), Vec::new(), Type::String);
        let b = reference("zod.ts\u{0}ZodObject", &shape("b: string; "), Vec::new(), Type::String);
        assert!(a.name().len() < 512);
        assert!(a.name().starts_with("ZodObject<{ a: string;"));
        assert_ne!(a.name(), b.name());
    }

    #[test]
    fn same_declaration_and_arguments_are_nominally_equal() {
        let a = reference(
            "box.ts\u{0}Box",
            "Box<string>",
            vec![Type::String],
            Type::String,
        );
        let b = reference(
            "box.ts\u{0}Box",
            "Box<string> (other display)",
            vec![Type::String],
            Type::Number,
        );
        // Equality is nominal: identical id + arguments, regardless of display or
        // resolver identity.
        assert_eq!(a, b);
        assert!(is_assignable_to(&a, &b));
    }

    #[test]
    fn different_arguments_are_not_nominally_equal() {
        let a = reference(
            "box.ts\u{0}Box",
            "Box<string>",
            vec![Type::String],
            Type::String,
        );
        let b = reference(
            "box.ts\u{0}Box",
            "Box<number>",
            vec![Type::Number],
            Type::Number,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn reference_falls_back_to_structural_expansion_for_assignability() {
        // `type Id = string;` — a reference whose structural form is `string`.
        let id = reference("ids.ts\u{0}Id", "Id", vec![], Type::String);
        assert!(is_assignable_to(&id, &Type::String));
        assert!(is_assignable_to(&Type::StringLiteral("x".to_string()), &id));
        assert!(!is_assignable_to(&id, &Type::Number));
    }

    #[test]
    fn reference_resolves_for_base_primitive() {
        let id = reference("ids.ts\u{0}Id", "Id", vec![], Type::String);
        assert_eq!(id.base_primitive(), Some(Type::String));
    }
}

const UNIQUE_SYMBOL_ID_PREFIX: &str = "\u{0}unique-symbol\u{0}";

struct UniqueSymbol;

impl ResolveReference for UniqueSymbol {
    fn resolve(&self) -> Type {
        Type::Symbol
    }
}

/// The type of `const name: unique symbol` declared in `file_name`: a `symbol`
/// whose identity is its declaration, so two unique symbols are unrelated
/// (tsc's `TypeFlagsUniqueESSymbol`) while each still flows into `symbol`.
/// `name` may carry the declaration's offset after a NUL (`s\0120`), which
/// keeps same-named declarations of one file apart, and then `\0member` for a
/// property signature's symbol, which no value name spells: tsc prints it as
/// `unique symbol`.
pub fn unique_symbol_type(file_name: &str, name: &str) -> Type {
    let (plain, position) = name.split_once('\u{0}').unwrap_or((name, ""));
    let display = if position.ends_with("\u{0}member") {
        "unique symbol".to_string()
    } else {
        format!("typeof {plain}")
    };
    Type::Reference(TypeReference::new(
        format!("{UNIQUE_SYMBOL_ID_PREFIX}{file_name}\u{0}{position}\u{0}{plain}"),
        display,
        Vec::new(),
        Arc::new(UniqueSymbol),
    ))
}
