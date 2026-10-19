use std::hash::Hash;
use std::sync::Arc;

use surge_ts_types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DeclarationNamespace {
    Type,
    /// Instantiations expanded *inside* an open type-parameter scope (a generic
    /// signature or body). Their nested references defer differently than at a
    /// concrete site (`concrete_instantiation` is false for everything they
    /// contain), so their interned expansions must never share a bucket with
    /// the concrete tier's — same declaration, same arguments, different
    /// representation.
    TypeSignatureContext,
    /// Generic instantiations deferred because a type argument is a signature
    /// *placeholder* (`QueryBehavior<TQueryFnData, …>` inside
    /// `function f<TQueryFnData>(…)`). The placeholder resolves to `unknown`
    /// but also marks the declaration's own parameter, which changes how the
    /// body resolves (deferred conditionals, indexed accesses), so the peeled
    /// expansion must never share a bucket with a literal `Foo<unknown>`. The
    /// key's `fingerprint` carries the placeholder argument mask.
    PlaceholderInstantiation,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DeclarationResolutionKey {
    pub(crate) file_name: Arc<str>,
    pub(crate) name: Arc<str>,
    pub(crate) namespace: DeclarationNamespace,
    /// Discriminator for the entries whose identity is a *hash* rather than a
    /// name — the module-instantiation memo and the display-tagged signature
    /// context. Zero for every ordinary declaration key. Kept as its own field
    /// so the hot resolution paths do not format it into `name`, which cost a
    /// `String` plus an `Arc<str>` allocation per interface/alias resolution.
    pub(crate) fingerprint: u64,
}

#[derive(Debug, Clone)]
pub(crate) enum DeclarationResolutionState {
    Resolving,
    Resolved { ty: Type, had_error: bool },
}

#[derive(Debug, Clone)]
pub(crate) struct GenericInstantiationCacheEntry {
    pub(crate) arguments: Vec<Type>,
    pub(crate) arguments_digest: u64,
    pub(crate) ty: Type,
    pub(crate) had_error: bool,
}

/// One declaration's interned instantiations in insertion order, indexed by
/// argument digest. A lookup walks only the entries carrying its digest, where
/// a scan read every instantiation of the declaration on every lookup: zod's,
/// trpc's and drizzle-orm's generics hold thousands, which made the scans
/// 0.19-0.32 billion entry reads per run.
#[derive(Debug, Clone)]
pub(crate) struct DigestBucket<E> {
    entries: Vec<E>,
    /// Digest to the first and last entry carrying it.
    ends: surge_ts_types::fx::FxHashMap<u64, (u32, u32)>,
    /// Per entry, the next entry with the same digest.
    next: Vec<u32>,
}

impl<E> Default for DigestBucket<E> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            ends: Default::default(),
            next: Vec::new(),
        }
    }
}

const NO_ENTRY: u32 = u32::MAX;

impl<E> std::ops::Deref for DigestBucket<E> {
    type Target = [E];

    fn deref(&self) -> &[E] {
        &self.entries
    }
}

impl<'a, E> IntoIterator for &'a DigestBucket<E> {
    type Item = &'a E;
    type IntoIter = std::slice::Iter<'a, E>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<E> DigestBucket<E> {
    /// The first entry, in insertion order, that has `digest` and satisfies
    /// `matches`; `matches` must imply the digest, as the entries' own
    /// `matches` does.
    pub(crate) fn find(&self, digest: u64, mut matches: impl FnMut(&E) -> bool) -> Option<&E> {
        if !digest_buckets_enabled() {
            return self.entries.iter().find(|entry| matches(entry));
        }
        let mut index = self.ends.get(&digest)?.0;
        while index != NO_ENTRY {
            let entry = &self.entries[index as usize];
            if matches(entry) {
                return Some(entry);
            }
            index = self.next[index as usize];
        }
        None
    }

    pub(crate) fn push(&mut self, digest: u64, entry: E) {
        let index = self.entries.len() as u32;
        self.entries.push(entry);
        self.next.push(NO_ENTRY);
        match self.ends.get_mut(&digest) {
            Some((_, last)) => {
                self.next[*last as usize] = index;
                *last = index;
            }
            None => {
                self.ends.insert(digest, (index, index));
            }
        }
    }
}

/// `SURGE_DIGEST_BUCKETS=0` restores the linear bucket scan, for A/B
/// measurement.
fn digest_buckets_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_DIGEST_BUCKETS").as_deref() != Ok("0"))
}

/// An equality-consistent digest of an instantiation's arguments
/// (`type_conflict_digest` per argument): equal argument lists always share it,
/// so a bucket scan compares it before the deep `Type` equality.
pub(crate) fn arguments_digest(arguments: &[Type]) -> u64 {
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    arguments.len().hash(&mut hasher);
    for argument in arguments {
        surge_ts_types::type_conflict_digest(argument).hash(&mut hasher);
    }
    std::hash::Hasher::finish(&hasher)
}

impl GenericInstantiationCacheEntry {
    pub(crate) fn matches(&self, digest: u64, arguments: &[Type]) -> bool {
        self.arguments_digest == digest && self.arguments == arguments
    }
}

/// One memoized structural expansion of a named declaration at a fixed set of
/// resolved type arguments, shared via `Arc` so a `Type::Reference` can resolve
/// to it without re-expanding the declaration body. Backs the lazy/nominal type
/// reference machinery (see `infer::types` instantiation interner).
#[derive(Debug, Clone)]
pub(crate) struct InstantiationCacheEntry {
    pub(crate) arguments: Vec<Type>,
    pub(crate) arguments_digest: u64,
    pub(crate) resolved: std::sync::Arc<Type>,
}

impl InstantiationCacheEntry {
    pub(crate) fn matches(&self, digest: u64, arguments: &[Type]) -> bool {
        self.arguments_digest == digest && self.arguments == arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StableInterfaceDeclarationFragmentId {
    pub(crate) canonical_file: Arc<str>,
    pub(crate) declaration_start: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StableInterfaceDeclarationId {
    pub(crate) canonical_file: Arc<str>,
    pub(crate) declaration_start: u32,
    pub(crate) declaration_name: Arc<str>,
    pub(crate) merged_fragments: Arc<[StableInterfaceDeclarationFragmentId]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CanonicalTypeIdentity {
    String,
    Number,
    Boolean,
    BigInt,
    Symbol,
    Undefined,
    Null,
    Void,
    Any,
    Never,
    StringLiteral(Arc<str>),
    NumberLiteral(Arc<str>),
    BooleanLiteral(bool),
    Array(Box<Self>),
    Tuple(Arc<[Self]>),
    Reference {
        declaration: Arc<str>,
        arguments: Arc<[Self]>,
    },
    NamedObject(Arc<str>),
    /// An unsubstituted type parameter, identified by its declared name.
    ///
    /// Admitting this is the point of [`surge_ts_types::Type::TypeParameter`]:
    /// before it existed a placeholder argument arrived as `Type::Unknown`,
    /// indistinguishable from a degraded resolution, and the key builder had to
    /// refuse it (287,082 refusals on tanstack-query). The name is carried
    /// rather than collapsing every parameter to one identity, so that two
    /// declarations whose parameters are not interchangeable cannot share a key
    /// — a missed hit is cheap, a wrong hit is not.
    TypeParameter(Arc<str>),
    /// A structural identity paired with the argument's display fingerprint.
    /// Structurally-equal arguments can carry different rendered display forms
    /// (the canonical-store display-substitution class); a cache keyed without
    /// the display would bake the first winner's rendering into every
    /// consumer. Wraps each top-level instantiation argument.
    DisplayTagged(Box<Self>, u64),
    /// Widened identities for the extended interface-instantiation tier:
    /// equality-faithful encodings of argument shapes the compact identities
    /// above cannot express. Each mirrors exactly the fields the type's
    /// derived `PartialEq` compares — interned list ids included — so
    /// identity equality implies argument equality and the flat cache key
    /// stays injective without a bucket-plus-`==`-confirm tier. Object
    /// properties are sorted by name (`IndexMap` equality is
    /// order-independent); `call_signature`/`is_intersection`/`alias_id` are
    /// excluded from `ObjectType` equality and stay unencoded.
    UnionArg {
        list_id: Option<surge_ts_types::TypeListId>,
        members: Arc<[Self]>,
    },
    FunctionArg {
        parameter_list_id: Option<surge_ts_types::TypeListId>,
        parameters: Arc<[Self]>,
        return_type: Box<Self>,
        is_variadic: bool,
        required_parameter_count: usize,
    },
    ObjectArg {
        properties: Arc<[(Arc<str>, bool, Self)]>,
        string_index: Option<Box<Self>>,
    },
}
