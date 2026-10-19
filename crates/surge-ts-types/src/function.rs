use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::clone_reason::{TypeCopyReason, current_type_copy_reason};
use crate::store::{canonical_function_store_enabled, current_program_type_store};
use crate::{FunctionTypeId, Type, TypeListId};

static FUNCTION_TYPE_PAYLOAD_ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_PAYLOAD_ALLOC_BY_REASON: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];
static FUNCTION_TYPE_PAYLOAD_ALLOC_BY_EXPANSION_REASON: [AtomicU64; 42] =
    [const { AtomicU64::new(0) }; 42];
static FUNCTION_TYPE_PAYLOAD_DEEP_CLONE_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_HANDLE_COPY_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_CLONE_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT: AtomicU64 = AtomicU64::new(0);
static FUNCTION_TYPE_COPY_UNATTRIBUTED_COUNT: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static CURRENT_FUNCTION_TYPE_EXPANSION_REASON: Cell<usize> = const { Cell::new(41) };
}

pub fn replace_function_type_expansion_reason(reason: usize) -> usize {
    CURRENT_FUNCTION_TYPE_EXPANSION_REASON.replace(reason)
}

pub fn snapshot_function_type_payload_alloc_by_expansion_reason() -> [u64; 42] {
    std::array::from_fn(|index| {
        FUNCTION_TYPE_PAYLOAD_ALLOC_BY_EXPANSION_REASON[index].load(Ordering::Relaxed)
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FunctionTypeCounters {
    pub function_type_payload_alloc_count: u64,
    pub function_type_payload_alloc_by_reason: [u64; 13],
    pub function_type_payload_deep_clone_count: u64,
    pub function_type_handle_copy_count: u64,
    pub function_type_clone_count: u64,
    pub function_type_copy_from_expression_inference_count: u64,
    pub function_type_copy_from_call_resolution_count: u64,
    pub function_type_copy_from_property_call_resolution_count: u64,
    pub function_type_copy_from_function_body_setup_count: u64,
    pub function_type_copy_from_return_checking_count: u64,
    pub function_type_copy_from_expected_type_count: u64,
    pub function_type_copy_from_symbol_table_count: u64,
    pub function_type_copy_from_module_export_count: u64,
    pub function_type_copy_from_scope_or_context_count: u64,
    pub function_type_copy_from_substitution_unchanged_count: u64,
    pub function_type_copy_from_substitution_changed_count: u64,
    pub function_type_copy_from_diagnostic_formatting_count: u64,
    pub function_type_copy_unattributed_count: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FunctionTypePayload {
    pub parameters: Arc<[Type]>,
    pub(crate) parameter_list_id: Option<TypeListId>,
    pub return_type: Type,
    pub is_variadic: bool,
    pub required_parameter_count: usize,
    pub(crate) name_memo: crate::name_memo::NameMemo,
}

impl Clone for FunctionTypePayload {
    fn clone(&self) -> Self {
        record_function_type_payload_deep_clone_count();
        Self {
            parameters: self.parameters.clone(),
            parameter_list_id: self.parameter_list_id,
            return_type: self.return_type.clone(),
            is_variadic: self.is_variadic,
            required_parameter_count: self.required_parameter_count,
            name_memo: self.name_memo.clone(),
        }
    }
}

/// A generic signature with a constrained type parameter, as written. The
/// resolved signature erases a constrained type parameter to the degradation
/// sentinel, which leaves a relation nothing to relate; here the signature's
/// own type parameters stay placeholders, and each constraint is resolved over
/// them, so a relation can bind them as type variables (`getCanonicalSignature`)
/// or infer them (`instantiateSignatureInContextOf`).
#[derive(Debug)]
pub struct GenericSignatureShape {
    pub type_parameters: Vec<(Arc<str>, Option<Type>)>,
    pub parameters: Vec<Type>,
    pub return_type: Type,
}

pub type GenericSignatureShapeLookup =
    fn(&(dyn std::any::Any + Send + Sync)) -> Option<&GenericSignatureShape>;

static GENERIC_SIGNATURE_SHAPE_LOOKUP: std::sync::OnceLock<GenericSignatureShapeLookup> =
    std::sync::OnceLock::new();

/// Installs how a signature's declaration record answers its
/// [`GenericSignatureShape`]: the records are the checker's, which this crate
/// cannot name.
pub fn install_generic_signature_shape_lookup(lookup: GenericSignatureShapeLookup) {
    let _ = GENERIC_SIGNATURE_SHAPE_LOOKUP.set(lookup);
}

#[derive(Debug)]
pub struct FunctionType {
    pub(crate) payload: Arc<FunctionTypePayload>,
    id: Option<FunctionTypeId>,
    /// Written parameter names, for display only. tsc renders a function type as
    /// `(value: string) => void`, but names play no part in assignability, so
    /// they live on the handle rather than the interned payload: two signatures
    /// that differ only in parameter names still share one canonical payload.
    parameter_names: Option<Arc<[Option<Arc<str>>]>>,
    /// Rendered type-parameter list (`<T extends FieldBag = FieldBag>`), which
    /// tsc prefixes to a generic signature. Display-only, for the same reason.
    type_parameter_head: Option<Arc<str>>,
    /// The type-alias name this signature was written as (`type Fn = (x: string)
    /// => void`), which tsc displays instead of the structural form.
    alias_name: Option<Arc<str>>,
    /// The checker's record of the declaration this value-level signature came
    /// from (`typeof fn` over a generic `declare function fn<T>`), so a call
    /// through a property typed by it can still instantiate the declared type
    /// parameters. Opaque here: this crate has no declaration table. Handle
    /// metadata like the names above — never part of the payload, identity, or
    /// equality.
    declaration: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// The overload group (declaration order) this merged signature stands for,
    /// when it was folded from more than one overload. Call resolution reads it
    /// to pick the return type of the first overload the evaluated arguments
    /// satisfy. Handle metadata: the payload stays the permissive fold, so
    /// identity, interning and equality are unchanged, and two groups folding to
    /// one shape still share a payload. Thin pointer on purpose — a fat one
    /// would grow every `Type`.
    overloads: Option<Arc<Vec<FunctionType>>>,
    /// For a construct signature, the flags relater.go reads off it: see
    /// [`ConstructModifiers`]. Handle metadata like the fields above.
    construct_modifiers: ConstructModifiers,
    /// relater.go `isInstantiatedGenericParameter`: bit `i` is set when this
    /// signature instantiates one whose parameter `i` was a generic type (a
    /// bare type variable). Handle metadata like the fields above.
    instantiated_generic_parameters: u32,
    /// Bit `i` is set when parameter `i` was declared optional by a written
    /// `?`, which tsc prints as `x?: T | undefined`; an initializer's optional
    /// parameter prints as `x?: T`. Display-only handle metadata, kept narrow
    /// so it fits the handle's padding.
    question_optional_parameters: u16,
    /// The signature's written type predicate (`x is T`, `this is T`,
    /// `asserts x is T`), for `compareTypePredicateRelatedTo`. Handle metadata
    /// like the fields above.
    type_predicate: Option<Arc<TypePredicate>>,
    /// The written `this` parameter's type (`(this: C) => void`), for
    /// `getThisTypeOfSignature`. Handle metadata like the fields above.
    this_type: Option<Arc<Type>>,
    /// `this_type` stands in for the polymorphic `this` with the declaring
    /// type, which each receiver instantiates differently, so it does not
    /// decide a relation between signatures.
    polymorphic_this: bool,
    /// The alias instantiation this signature was resolved from, which a
    /// relation compares by its arguments (relater.go's alias variances).
    /// Handle metadata like the fields above, dropped with the payload it
    /// describes when the signature's types are substituted.
    alias_instantiation: Option<Arc<AliasInstantiation>>,
}

/// Prefixes a destructured parameter's name in a signature's parameter names:
/// the binding pattern as `typeToString` prints it (`{ a, b }`). Only the
/// compiler API's tsc display prints it; everywhere else the parameter has
/// no name, as tsc's diagnostics name it by its symbol (`__0`).
pub const PATTERN_PARAMETER_MARKER: char = '\u{1}';

/// A parameter name a diagnostic or relation may use: none for a pattern.
pub fn plain_parameter_name(name: &str) -> Option<&str> {
    (!name.starts_with(PATTERN_PARAMETER_MARKER)).then_some(name)
}

/// The labels tsc gives a tuple built from a parameter list
/// (`getNameableDeclarationAtPosition`): each parameter's name, none for a
/// destructured one.
pub fn parameter_tuple_labels(names: &[Option<Arc<str>>]) -> Vec<crate::TupleLabel> {
    names
        .iter()
        .map(|name| name.as_deref().and_then(plain_parameter_name).map(Arc::from))
        .collect()
}

/// A generic type alias instantiation (`aliasSymbol`/`aliasTypeArguments`).
#[derive(Debug)]
pub struct AliasInstantiation {
    /// The alias's nominal id (`file\0Name`).
    pub id: Arc<str>,
    pub arguments: Arc<[Type]>,
    /// The variances its type parameters measure, packed by
    /// [`crate::declared_variances`].
    pub variances: u32,
}

/// checker.go `TypePredicate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypePredicate {
    pub kind: TypePredicateKind,
    /// The tested parameter's position, for an identifier predicate.
    pub parameter_index: Option<usize>,
    /// `None` for a bare `asserts x`.
    pub ty: Option<Type>,
    /// The subject as written where no parameter position names it (a
    /// binding of a destructured parameter, or a name that is none).
    pub name: Option<Arc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypePredicateKind {
    This,
    Identifier,
    AssertsThis,
    AssertsIdentifier,
}

/// A construct signature's `SignatureFlagsAbstract` and the accessibility of
/// the constructor declaration it came from (`constructorVisibilitiesAreCompatible`).
/// Also carries whether a call signature's declaration is a method, which
/// `compareSignaturesRelated` reads off the target (`strictVariance`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ConstructModifiers(u8);

impl ConstructModifiers {
    const ABSTRACT: u8 = 1;
    const ACCESSIBILITY_SHIFT: u8 = 1;
    const ACCESSIBILITY_MASK: u8 = 0b110;
    const METHOD_DECLARATION: u8 = 0b1000;
    const RETURNS_POLYMORPHIC_THIS: u8 = 0b1_0000;
    const RETURN_MENTIONS_POLYMORPHIC_THIS: u8 = 0b10_0000;

    pub fn new(is_abstract: bool, accessibility: ConstructorAccessibility) -> Self {
        let accessibility = match accessibility {
            ConstructorAccessibility::Undeclared => 0,
            ConstructorAccessibility::Public => 1,
            ConstructorAccessibility::Protected => 2,
            ConstructorAccessibility::Private => 3,
        };
        Self(u8::from(is_abstract) | (accessibility << Self::ACCESSIBILITY_SHIFT))
    }

    pub fn is_abstract(self) -> bool {
        self.0 & Self::ABSTRACT != 0
    }

    pub fn accessibility(self) -> ConstructorAccessibility {
        match (self.0 & Self::ACCESSIBILITY_MASK) >> Self::ACCESSIBILITY_SHIFT {
            1 => ConstructorAccessibility::Public,
            2 => ConstructorAccessibility::Protected,
            3 => ConstructorAccessibility::Private,
            _ => ConstructorAccessibility::Undeclared,
        }
    }

    pub fn with_abstract(self, is_abstract: bool) -> Self {
        Self::new(is_abstract, self.accessibility())
    }

    pub fn bits(self) -> u8 {
        self.0
    }
}

/// `Undeclared` is a signature without a declaration (a class's default
/// constructor), which relates to any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstructorAccessibility {
    Undeclared,
    Public,
    Protected,
    Private,
}

impl FunctionType {
    pub(crate) fn from_canonical_parts(
        payload: Arc<FunctionTypePayload>,
        id: FunctionTypeId,
    ) -> Self {
        Self {
            payload,
            id: Some(id),
            parameter_names: None,
            type_parameter_head: None,
            alias_name: None,
            declaration: None,
            overloads: None,
            construct_modifiers: ConstructModifiers::default(),
            instantiated_generic_parameters: 0,
            question_optional_parameters: 0,
            type_predicate: None,
            this_type: None,
            polymorphic_this: false,
            alias_instantiation: None,
        }
    }

    pub fn new(
        parameters: Vec<Type>,
        return_type: Type,
        is_variadic: bool,
        required_parameter_count: usize,
    ) -> Self {
        if canonical_function_store_enabled()
            && let Some(store) = current_program_type_store()
        {
            match store.intern_function(
                parameters,
                return_type,
                is_variadic,
                required_parameter_count,
            ) {
                Ok((payload, id)) => {
                    return Self {
                        payload,
                        id: Some(id),
                        parameter_names: None,
                        type_parameter_head: None,
                        alias_name: None,
                        declaration: None,
                        overloads: None,
                        construct_modifiers: ConstructModifiers::default(),
                        instantiated_generic_parameters: 0,
                        question_optional_parameters: 0,
                        type_predicate: None,
                        this_type: None,
                        polymorphic_this: false,
                        alias_instantiation: None,
                    };
                }
                Err((parameters, return_type)) => {
                    store.record_function_fallback();
                    record_function_type_payload_alloc_count();
                    return Self {
                        payload: Arc::new(FunctionTypePayload {
                            parameters: parameters.into(),
                            parameter_list_id: None,
                            return_type,
                            is_variadic,
                            required_parameter_count,
                            name_memo: crate::name_memo::NameMemo::default(),
                        }),
                        id: None,
                        parameter_names: None,
                        type_parameter_head: None,
                        alias_name: None,
                        declaration: None,
                        overloads: None,
                        construct_modifiers: ConstructModifiers::default(),
                        instantiated_generic_parameters: 0,
                        question_optional_parameters: 0,
                        type_predicate: None,
                        this_type: None,
                        polymorphic_this: false,
                        alias_instantiation: None,
                    };
                }
            }
        }
        record_function_type_payload_alloc_count();
        Self {
            payload: Arc::new(FunctionTypePayload {
                parameters: parameters.into(),
                parameter_list_id: None,
                return_type,
                is_variadic,
                required_parameter_count,
                name_memo: crate::name_memo::NameMemo::default(),
            }),
            id: None,
            parameter_names: None,
            type_parameter_head: None,
            alias_name: None,
            declaration: None,
            overloads: None,
            construct_modifiers: ConstructModifiers::default(),
            instantiated_generic_parameters: 0,
            question_optional_parameters: 0,
            type_predicate: None,
            this_type: None,
            polymorphic_this: false,
            alias_instantiation: None,
        }
    }

    /// The same signature — names, head, declaration, overloads — over new
    /// parameter and return types.
    pub fn with_signature_types(&self, parameters: Vec<Type>, return_type: Type) -> Self {
        Self {
            parameter_names: self.parameter_names.clone(),
            type_parameter_head: self.type_parameter_head.clone(),
            alias_name: self.alias_name.clone(),
            declaration: self.declaration.clone(),
            overloads: self.overloads.clone(),
            construct_modifiers: self.construct_modifiers,
            instantiated_generic_parameters: self.instantiated_generic_parameters,
            question_optional_parameters: self.question_optional_parameters,
            type_predicate: self.type_predicate.clone(),
            this_type: self.this_type.clone(),
            polymorphic_this: self.polymorphic_this,
            ..Self::new(parameters, return_type, self.is_variadic(), self.required_parameter_count())
        }
    }

    /// The same signature with its first `required` parameters required.
    pub fn with_required_parameter_count(&self, required: usize) -> Self {
        Self {
            parameter_names: self.parameter_names.clone(),
            type_parameter_head: self.type_parameter_head.clone(),
            alias_name: self.alias_name.clone(),
            declaration: self.declaration.clone(),
            overloads: self.overloads.clone(),
            construct_modifiers: self.construct_modifiers,
            instantiated_generic_parameters: self.instantiated_generic_parameters,
            question_optional_parameters: self.question_optional_parameters,
            type_predicate: self.type_predicate.clone(),
            this_type: self.this_type.clone(),
            polymorphic_this: self.polymorphic_this,
            ..Self::new(self.parameters().to_vec(), self.return_type().clone(), self.is_variadic(), required)
        }
    }

    /// Attaches the written parameter names for display. Ignored when the count
    /// does not match the parameter list, and dropped entirely when no position
    /// carries a name, so an unnamed signature keeps the memoized rendering.
    pub fn with_parameter_names(mut self, names: Vec<Option<Arc<str>>>) -> Self {
        if names.len() == self.payload.parameters.len()
            && names.iter().any(std::option::Option::is_some)
        {
            self.parameter_names = Some(names.into());
        }
        self
    }

    pub fn parameter_names(&self) -> Option<&[Option<Arc<str>>]> {
        self.parameter_names.as_deref()
    }

    /// The names this signature declares as its own type parameters, read off
    /// the rendered list (`T = unknown, const K extends keyof T`).
    pub fn type_parameter_names(&self) -> Vec<String> {
        let Some(head) = self.type_parameter_head.as_deref() else {
            return Vec::new();
        };
        let mut names = Vec::new();
        let mut depth = 0i32;
        let mut start = 0;
        let bytes = head.as_bytes();
        for index in 0..=bytes.len() {
            let byte = bytes.get(index).copied();
            match byte {
                Some(b'<' | b'(' | b'[' | b'{') => depth += 1,
                Some(b'>') if index > 0 && bytes[index - 1] == b'=' => {}
                Some(b'>' | b')' | b']' | b'}') => depth -= 1,
                Some(b',') | None if depth <= 0 => {
                    let item = head[start..index.min(head.len())].trim();
                    let name = item
                        .split_whitespace()
                        .find(|word| !matches!(*word, "const" | "in" | "out"))
                        .unwrap_or_default();
                    if !name.is_empty() {
                        names.push(name.to_string());
                    }
                    start = index + 1;
                }
                _ => {}
            }
        }
        names
    }

    /// Attaches the rendered type-parameter list, without the angle brackets.
    /// The rendered type-parameter list of a declared generic signature, if
    /// any. An instantiated signature carries none.
    pub fn type_parameter_head(&self) -> Option<&str> {
        self.type_parameter_head.as_deref()
    }

    pub fn with_type_parameter_head(mut self, head: Option<String>) -> Self {
        self.type_parameter_head = head.filter(|head| !head.is_empty()).map(Arc::from);
        self
    }

    /// Names this signature by the alias it was written as, for display only.
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

    /// Records the alias instantiation this signature was resolved from.
    pub fn with_alias_instantiation(mut self, id: Arc<str>, arguments: Vec<Type>, variances: u32) -> Self {
        self.alias_instantiation = Some(Arc::new(AliasInstantiation { id, arguments: arguments.into(), variances }));
        self
    }

    pub fn alias_instantiation(&self) -> Option<&AliasInstantiation> {
        self.alias_instantiation.as_deref()
    }

    pub(crate) fn alias_instantiation_address(&self) -> usize {
        self.alias_instantiation.as_ref().map_or(0, |alias| Arc::as_ptr(alias) as usize)
    }

    /// Attaches the declaration record this signature was read from.
    pub fn with_declaration(mut self, declaration: Arc<dyn std::any::Any + Send + Sync>) -> Self {
        self.declaration = Some(declaration);
        self
    }

    /// This signature carrying `overloads` as its group. Fewer than two members
    /// carries nothing: a single signature is its own resolution candidate.
    pub fn with_overloads(mut self, overloads: Vec<FunctionType>) -> Self {
        self.overloads = (overloads.len() >= 2).then(|| Arc::new(overloads));
        self
    }

    pub fn with_construct_modifiers(mut self, modifiers: ConstructModifiers) -> Self {
        self.construct_modifiers = modifiers;
        self
    }

    pub fn construct_modifiers(&self) -> ConstructModifiers {
        self.construct_modifiers
    }

    /// Marks the signature as declared by a method (`m(x: T): U`), whose
    /// parameters a relation compares bivariantly.
    pub fn with_method_declaration(mut self, method: bool) -> Self {
        let bits = self.construct_modifiers.0 & !ConstructModifiers::METHOD_DECLARATION;
        self.construct_modifiers =
            ConstructModifiers(bits | if method { ConstructModifiers::METHOD_DECLARATION } else { 0 });
        self
    }

    /// Marks a method declared to return the polymorphic `this`, whose return
    /// (`any` until then) a member read replaces with the receiver.
    pub fn with_polymorphic_this_return(mut self, returns_this: bool) -> Self {
        let bits = self.construct_modifiers.0 & !ConstructModifiers::RETURNS_POLYMORPHIC_THIS;
        self.construct_modifiers = ConstructModifiers(
            bits | if returns_this { ConstructModifiers::RETURNS_POLYMORPHIC_THIS } else { 0 },
        );
        self
    }

    /// Marks a method whose return names the polymorphic `this` inside it
    /// (`optional(): ZodOptional<this>`), re-resolved per receiver.
    pub fn with_return_mentioning_polymorphic_this(mut self, mentions: bool) -> Self {
        let bits = self.construct_modifiers.0 & !ConstructModifiers::RETURN_MENTIONS_POLYMORPHIC_THIS;
        self.construct_modifiers = ConstructModifiers(
            bits | if mentions { ConstructModifiers::RETURN_MENTIONS_POLYMORPHIC_THIS } else { 0 },
        );
        self
    }

    pub fn return_mentions_polymorphic_this(&self) -> bool {
        self.construct_modifiers.0 & ConstructModifiers::RETURN_MENTIONS_POLYMORPHIC_THIS != 0
    }

    pub fn returns_polymorphic_this(&self) -> bool {
        self.construct_modifiers.0 & ConstructModifiers::RETURNS_POLYMORPHIC_THIS != 0
    }

    pub fn is_method_declaration(&self) -> bool {
        self.construct_modifiers.0 & ConstructModifiers::METHOD_DECLARATION != 0
    }

    pub fn with_instantiated_generic_parameters(mut self, mask: u32) -> Self {
        self.instantiated_generic_parameters = mask;
        self
    }

    pub fn is_instantiated_generic_parameter(&self, index: usize) -> bool {
        index < 32 && self.instantiated_generic_parameters & (1 << index) != 0
    }

    pub fn with_question_optional_parameters(mut self, mask: u16) -> Self {
        self.question_optional_parameters = mask;
        self
    }

    fn is_question_optional_parameter(&self, index: usize) -> bool {
        index < 16 && self.question_optional_parameters & (1 << index) != 0
    }

    pub fn with_type_predicate(mut self, predicate: Option<TypePredicate>) -> Self {
        self.type_predicate = predicate.map(Arc::new);
        self
    }

    pub fn type_predicate(&self) -> Option<&TypePredicate> {
        self.type_predicate.as_deref()
    }

    pub fn with_this_type(mut self, this_type: Option<Type>) -> Self {
        self.this_type = this_type.map(Arc::new);
        self
    }

    pub fn this_type(&self) -> Option<&Type> {
        self.this_type.as_deref()
    }

    pub fn with_polymorphic_this(mut self, polymorphic: bool) -> Self {
        self.polymorphic_this = polymorphic;
        self
    }

    pub fn has_polymorphic_this(&self) -> bool {
        self.polymorphic_this
    }

    pub(crate) fn this_type_address(&self) -> usize {
        self.this_type.as_ref().map_or(0, |ty| Arc::as_ptr(ty) as usize)
    }

    pub fn overloads(&self) -> Option<&[FunctionType]> {
        self.overloads.as_deref().map(Vec::as_slice)
    }

    /// The group members in declaration order, or the signature itself when it
    /// is not a group, appended to `out`.
    pub fn push_overload_members(&self, out: &mut Vec<FunctionType>) {
        match self.overloads() {
            Some(members) => out.extend(members.iter().cloned()),
            None => out.push(self.clone()),
        }
    }

    pub fn declaration(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        self.declaration.as_deref()
    }

    pub fn declaration_handle(&self) -> Option<&Arc<dyn std::any::Any + Send + Sync>> {
        self.declaration.as_ref()
    }

    /// The written shape of this signature, when its declaration record
    /// carries one (see [`GenericSignatureShape`]).
    pub fn generic_shape(&self) -> Option<&GenericSignatureShape> {
        let lookup = GENERIC_SIGNATURE_SHAPE_LOOKUP.get()?;
        lookup(self.declaration.as_deref()?)
    }

    pub fn payload(&self) -> &FunctionTypePayload {
        &self.payload
    }

    pub fn parameters(&self) -> &[Type] {
        &self.payload.parameters
    }

    pub fn return_type(&self) -> &Type {
        &self.payload.return_type
    }

    pub fn is_variadic(&self) -> bool {
        self.payload.is_variadic
    }

    pub fn required_parameter_count(&self) -> usize {
        self.payload.required_parameter_count
    }

    pub fn id(&self) -> Option<FunctionTypeId> {
        self.id
    }

    pub fn parameter_list_id(&self) -> Option<TypeListId> {
        self.payload.parameter_list_id
    }

    pub fn payload_address(&self) -> usize {
        Arc::as_ptr(&self.payload) as usize
    }

    /// A weak handle to the shared payload, for identity-keyed memo entries
    /// that must pin the allocation so its address cannot be reused.
    pub fn payload_weak(&self) -> std::sync::Weak<FunctionTypePayload> {
        Arc::downgrade(&self.payload)
    }

    pub fn parameter_list_address(&self) -> usize {
        self.payload.parameters.as_ptr() as usize
    }

    pub fn name(&self) -> String {
        if let Some(alias_name) = self.alias_name.as_deref() {
            return crate::tsc_display::counted(alias_name.to_string());
        }
        // An overload group prints as the object type of its signatures.
        if let Some(members) = self.overloads()
            && members.len() > 1
        {
            if crate::tsc_display::truncating() {
                return "{ ...; }".to_string();
            }
            let signatures: Vec<String> = members
                .iter()
                .map(|member| format!("{};", member.member_name()))
                .collect();
            crate::tsc_display::add_length(2);
            return format!("{{ {} }}", signatures.join(" "));
        }
        // The memo lives on the shared payload, so a handle-local rendering must
        // not be written into it.
        if self.parameter_names.is_some()
            || self.type_parameter_head.is_some()
            || self.this_type.is_some()
            || self.generic_shape().is_some()
            || (self.type_predicate.is_some() && crate::tsc_display::active())
        {
            return self.render_name();
        }
        if crate::tsc_display::active() {
            return crate::tsc_display::memoized(Arc::as_ptr(&self.payload) as usize, || self.render_name());
        }
        self.payload
            .name_memo
            .get_or_render(|| self.render_name())
            .to_string()
    }

    fn render_name(&self) -> String {
        self.render_with_return_separator(" => ")
    }

    /// The signature as a member of an object type, which tsc prints with a
    /// colon: `{ (s: string): number; label: any; }`.
    pub fn member_name(&self) -> String {
        self.render_with_return_separator(": ")
    }

    fn render_with_return_separator(&self, separator: &str) -> String {
        crate::tsc_display::add_length(3);
        let names = self.parameter_names();
        let shape = self.generic_shape();
        let parameter_types = shape.map_or(self.parameters(), |shape| shape.parameters.as_slice());
        let return_type = shape.map_or(self.return_type(), |shape| &shape.return_type);
        // tsc writes the type parameters first.
        let head = match self.type_parameter_head.as_deref() {
            Some(head) => {
                crate::tsc_display::count_type_parameter_head(head);
                format!("<{head}>")
            }
            None => shape.map_or_else(String::new, |shape| {
                let parameters = shape.type_parameters.iter().map(|(name, constraint)| {
                    match constraint {
                        Some(constraint) => format!("{name} extends {}", constraint.name()),
                        None => name.to_string(),
                    }
                }).collect::<Vec<_>>();
                format!("<{}>", parameters.join(", "))
            }),
        };
        // A variadic signature's named last parameter of an array type is its
        // rest parameter, which tsc prints as one (`...items: T[]`).
        let rest = parameter_types
            .len()
            .checked_sub(1)
            .filter(|last| {
                self.is_variadic()
                    && names.is_some_and(|names| names[*last].is_some())
                    && crate::tsc_display::is_rest_parameter_type(&parameter_types[*last])
            });
        let mut parameters = parameter_types
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let optional = index >= self.required_parameter_count();
                let name = names.and_then(|names| names[index].as_deref()).and_then(|name| {
                    match name.strip_prefix(PATTERN_PARAMETER_MARKER) {
                        Some(pattern) => crate::tsc_display::active().then_some(pattern),
                        None => Some(name),
                    }
                });
                let rendered = match name {
                    Some(name) if Some(index) == rest => format!("...{name}: {}", parameter.name()),
                    // A written `?` prints even where a required parameter
                    // follows it (a grammar error), as `isOptionalParameter`
                    // reads the token.
                    Some(name) if self.is_question_optional_parameter(index) => {
                        format!("{name}?: {}", optional_parameter_display(parameter))
                    }
                    Some(name) if optional => format!("{name}?: {}", parameter.name()),
                    Some(name) => format!("{name}: {}", parameter.name()),
                    None => parameter.name(),
                };
                crate::tsc_display::add_length(name.map_or(0, str::len) + 3);
                rendered
            })
            .collect::<Vec<_>>();

        // `getExpandedParameters`: a rest parameter of a tuple type is the
        // tuple's elements, named by their labels or `rest_0`, `rest_1`, …, a
        // variadic one keeping the rest's own name.
        if let Some(rest) = rest
            && crate::tsc_display::active()
            && let Some(name) = names.and_then(|names| names[rest].as_deref()).and_then(plain_parameter_name)
        {
            let expanded = match crate::tsc_display::rest_tuple_shape(&parameter_types[rest]) {
                Some(Type::Tuple(tuple)) => {
                    let labels = expanded_parameter_names(name, tuple.labels(), tuple.len(), None);
                    Some(
                        tuple
                            .iter()
                            .enumerate()
                            .map(|(index, element)| {
                                let optional = if tuple.is_flagged_optional(index) { "?" } else { "" };
                                format!("{}{optional}: {}", labels[index], element.name())
                            })
                            .collect::<Vec<_>>(),
                    )
                }
                Some(Type::OpenTuple(open)) => {
                    let rest_index = open.leading.len();
                    let labels = expanded_parameter_names(name, open.labels.as_deref(), open.fixed_len() + 1, Some(rest_index));
                    let leading_min_length = crate::tuple_min_length(&open.leading);
                    let mut expanded: Vec<String> = open
                        .leading
                        .iter()
                        .enumerate()
                        .map(|(index, element)| {
                            let optional = if index >= leading_min_length { "?" } else { "" };
                            format!("{}{optional}: {}", labels[index], element.name())
                        })
                        .collect();
                    expanded.push(format!("...{}: {}[]", labels[rest_index], open.rest.name()));
                    expanded.extend(
                        open.trailing
                            .iter()
                            .enumerate()
                            .map(|(index, element)| format!("{}: {}", labels[rest_index + 1 + index], element.name())),
                    );
                    Some(expanded)
                }
                _ => None,
            };
            if let Some(expanded) = expanded {
                parameters.truncate(rest);
                parameters.extend(expanded);
            }
        }
        if self.is_variadic() && rest.is_none() {
            crate::tsc_display::add_length("any".len() + "args".len() + 3);
            parameters.push("...args: any[]".to_string());
        }
        if let Some(this_type) = self.this_type() {
            let this_parameter = format!("this: {}", this_type.name());
            crate::tsc_display::add_length("this".len() + 3);
            parameters.insert(0, this_parameter);
        }

        // A method returning the polymorphic `this` no receiver bound yet
        // prints it as written.
        let unbound_this = self.returns_polymorphic_this() && matches!(return_type, Type::Any) && crate::tsc_display::active();
        let return_name = || if unbound_this { "this".to_string() } else { return_type.name() };
        // `typeToString` prints a signature's predicate as its return type.
        let returns = match self.type_predicate().filter(|_| crate::tsc_display::active()) {
            Some(predicate) => {
                let subject = match predicate.kind {
                    TypePredicateKind::This | TypePredicateKind::AssertsThis => Some("this".to_string()),
                    TypePredicateKind::Identifier | TypePredicateKind::AssertsIdentifier => predicate
                        .parameter_index
                        .and_then(|index| names.and_then(|names| names.get(index).cloned().flatten()))
                        .and_then(|name| plain_parameter_name(&name).map(str::to_string))
                        .or_else(|| predicate.name.as_deref().map(str::to_string)),
                };
                let asserts = matches!(predicate.kind, TypePredicateKind::AssertsThis | TypePredicateKind::AssertsIdentifier);
                match (subject, &predicate.ty) {
                    (Some(subject), Some(ty)) if asserts => format!("asserts {subject} is {}", ty.name()),
                    (Some(subject), None) if asserts => format!("asserts {subject}"),
                    (Some(subject), Some(ty)) => format!("{subject} is {}", ty.name()),
                    _ => return_name(),
                }
            }
            None => return_name(),
        };
        format!("{head}({}){separator}{returns}", parameters.join(", "))
    }
}

impl PartialEq for FunctionType {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.payload, &other.payload) || self.payload == other.payload)
            && self.this_type == other.this_type
    }
}

impl Eq for FunctionType {}

impl Clone for FunctionType {
    fn clone(&self) -> Self {
        record_function_type_handle_copy_count();
        record_function_type_copy_count_for_current_reason();
        record_function_type_clone_count();
        Self {
            payload: self.payload.clone(),
            id: self.id,
            parameter_names: self.parameter_names.clone(),
            type_parameter_head: self.type_parameter_head.clone(),
            alias_name: self.alias_name.clone(),
            declaration: self.declaration.clone(),
            overloads: self.overloads.clone(),
            construct_modifiers: self.construct_modifiers,
            instantiated_generic_parameters: self.instantiated_generic_parameters,
            question_optional_parameters: self.question_optional_parameters,
            type_predicate: self.type_predicate.clone(),
            this_type: self.this_type.clone(),
            polymorphic_this: self.polymorphic_this,
            alias_instantiation: self.alias_instantiation.clone(),
        }
    }
}

pub fn snapshot_function_type_counters() -> FunctionTypeCounters {
    FunctionTypeCounters {
        function_type_payload_alloc_count: FUNCTION_TYPE_PAYLOAD_ALLOC_COUNT
            .load(Ordering::Relaxed),
        function_type_payload_alloc_by_reason: std::array::from_fn(|index| {
            FUNCTION_TYPE_PAYLOAD_ALLOC_BY_REASON[index].load(Ordering::Relaxed)
        }),
        function_type_payload_deep_clone_count: FUNCTION_TYPE_PAYLOAD_DEEP_CLONE_COUNT
            .load(Ordering::Relaxed),
        function_type_handle_copy_count: FUNCTION_TYPE_HANDLE_COPY_COUNT.load(Ordering::Relaxed),
        function_type_clone_count: FUNCTION_TYPE_CLONE_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_expression_inference_count:
            FUNCTION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_call_resolution_count:
            FUNCTION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_property_call_resolution_count:
            FUNCTION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_function_body_setup_count:
            FUNCTION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_return_checking_count:
            FUNCTION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_expected_type_count: FUNCTION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT
            .load(Ordering::Relaxed),
        function_type_copy_from_symbol_table_count: FUNCTION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT
            .load(Ordering::Relaxed),
        function_type_copy_from_module_export_count: FUNCTION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT
            .load(Ordering::Relaxed),
        function_type_copy_from_scope_or_context_count:
            FUNCTION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_substitution_unchanged_count:
            FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_substitution_changed_count:
            FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT.load(Ordering::Relaxed),
        function_type_copy_from_diagnostic_formatting_count:
            FUNCTION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT.load(Ordering::Relaxed),
        function_type_copy_unattributed_count: FUNCTION_TYPE_COPY_UNATTRIBUTED_COUNT
            .load(Ordering::Relaxed),
    }
}

pub(crate) fn record_function_type_payload_alloc_count() {
    FUNCTION_TYPE_PAYLOAD_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
    FUNCTION_TYPE_PAYLOAD_ALLOC_BY_REASON[type_copy_reason_index(current_type_copy_reason())]
        .fetch_add(1, Ordering::Relaxed);
    let expansion_reason = CURRENT_FUNCTION_TYPE_EXPANSION_REASON.get();
    FUNCTION_TYPE_PAYLOAD_ALLOC_BY_EXPANSION_REASON[expansion_reason]
        .fetch_add(1, Ordering::Relaxed);
}

fn type_copy_reason_index(reason: TypeCopyReason) -> usize {
    match reason {
        TypeCopyReason::Other => 0,
        TypeCopyReason::ExpressionInference => 1,
        TypeCopyReason::CallResolution => 2,
        TypeCopyReason::PropertyCallResolution => 3,
        TypeCopyReason::FunctionBodySetup => 4,
        TypeCopyReason::ReturnChecking => 5,
        TypeCopyReason::ExpectedType => 6,
        TypeCopyReason::SymbolTable => 7,
        TypeCopyReason::ModuleExport => 8,
        TypeCopyReason::ScopeOrContext => 9,
        TypeCopyReason::SubstitutionUnchanged => 10,
        TypeCopyReason::SubstitutionChanged => 11,
        TypeCopyReason::DiagnosticFormatting => 12,
    }
}

pub(crate) fn record_function_type_payload_deep_clone_count() {
    FUNCTION_TYPE_PAYLOAD_DEEP_CLONE_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_function_type_handle_copy_count() {
    FUNCTION_TYPE_HANDLE_COPY_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_function_type_clone_count() {
    FUNCTION_TYPE_CLONE_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn record_function_type_copy_count_for_current_reason() {
    match current_type_copy_reason() {
        TypeCopyReason::ExpressionInference => {
            FUNCTION_TYPE_COPY_FROM_EXPRESSION_INFERENCE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::CallResolution => {
            FUNCTION_TYPE_COPY_FROM_CALL_RESOLUTION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::PropertyCallResolution => {
            FUNCTION_TYPE_COPY_FROM_PROPERTY_CALL_RESOLUTION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::FunctionBodySetup => {
            FUNCTION_TYPE_COPY_FROM_FUNCTION_BODY_SETUP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ReturnChecking => {
            FUNCTION_TYPE_COPY_FROM_RETURN_CHECKING_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ExpectedType => {
            FUNCTION_TYPE_COPY_FROM_EXPECTED_TYPE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SymbolTable => {
            FUNCTION_TYPE_COPY_FROM_SYMBOL_TABLE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ModuleExport => {
            FUNCTION_TYPE_COPY_FROM_MODULE_EXPORT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::ScopeOrContext => {
            FUNCTION_TYPE_COPY_FROM_SCOPE_OR_CONTEXT_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SubstitutionUnchanged => {
            FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_UNCHANGED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::SubstitutionChanged => {
            FUNCTION_TYPE_COPY_FROM_SUBSTITUTION_CHANGED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::DiagnosticFormatting => {
            FUNCTION_TYPE_COPY_FROM_DIAGNOSTIC_FORMATTING_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        TypeCopyReason::Other => {
            FUNCTION_TYPE_COPY_UNATTRIBUTED_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_rest_parameters_take_the_tuple_labels() {
        assert_eq!(expanded_parameter_names("args", None, 2, None), ["args_0", "args_1"]);
        let labels = [Some(Arc::from("a")), None, Some(Arc::from("a")), Some(Arc::from("rest"))];
        assert_eq!(expanded_parameter_names("args", Some(&labels), 4, None), ["a", "args_1", "a_1", "rest"]);
        assert_eq!(expanded_parameter_names("args", Some(&[None, None]), 2, Some(1)), ["args_0", "args"]);
    }

    #[test]
    fn function_type_name_no_params() {
        let ty = FunctionType::new(vec![], Type::String, false, 0);

        assert_eq!(ty.name(), "() => string");
    }

    #[test]
    fn function_type_name_one_param() {
        let ty = FunctionType::new(vec![Type::String], Type::Number, false, 1);

        assert_eq!(ty.name(), "(string) => number");
    }

    #[test]
    fn function_type_name_multiple_params() {
        let ty = FunctionType::new(
            vec![Type::String, Type::Number, Type::Boolean],
            Type::Void,
            false,
            3,
        );

        assert_eq!(ty.name(), "(string, number, boolean) => void");
    }

    #[test]
    fn function_type_name_nested_parameter() {
        let ty = FunctionType::new(
            vec![Type::Function(FunctionType::new(
                vec![Type::String],
                Type::Number,
                false,
                1,
            ))],
            Type::Void,
            false,
            1,
        );

        assert_eq!(ty.name(), "((string) => number) => void");
    }

    #[test]
    fn function_type_name_nested_return() {
        let ty = FunctionType::new(
            vec![Type::String],
            Type::Function(FunctionType::new(
                vec![Type::Number],
                Type::Boolean,
                false,
                1,
            )),
            false,
            1,
        );

        assert_eq!(ty.name(), "(string) => (number) => boolean");
    }
}

/// A method read off `receiver` whose return is the polymorphic `this` returns
/// the receiver itself, overload members included.
pub fn bind_polymorphic_this_return(member: Type, receiver: &Type) -> Type {
    let Type::Function(function) = &member else {
        return member;
    };
    let mut members = Vec::new();
    function.push_overload_members(&mut members);
    if !members.iter().any(FunctionType::returns_polymorphic_this) {
        return member;
    }
    let rebind = |function: &FunctionType| {
        if function.returns_polymorphic_this() {
            function
                .with_signature_types(function.parameters().to_vec(), receiver.clone())
                .with_polymorphic_this_return(false)
        } else {
            function.clone()
        }
    };
    let rebound = rebind(function);
    Type::Function(match function.overloads() {
        Some(overloads) => rebound.with_overloads(overloads.iter().map(rebind).collect()),
        None => rebound,
    })
}

/// A `?` parameter's type as tsc prints it: under `strictNullChecks` its
/// declared type gains `undefined`, which `any`, `unknown`, and a type already
/// including it absorb.
/// `getUniqAssociatedNamesFromTupleType`: the parameter names a rest
/// parameter `rest` of a tuple expands to — each element's label, else
/// `rest_<index>` (the variadic element keeps `rest` itself), a repeated
/// label numbered apart.
fn expanded_parameter_names(rest: &str, labels: Option<&[crate::TupleLabel]>, len: usize, variadic: Option<usize>) -> Vec<String> {
    let unlabeled = |index: usize| if Some(index) == variadic { rest.to_string() } else { format!("{rest}_{index}") };
    let Some(labels) = labels.filter(|labels| labels.len() == len) else {
        return (0..len).map(unlabeled).collect();
    };
    let mut names: Vec<String> =
        labels.iter().enumerate().map(|(index, label)| label.as_deref().map_or_else(|| unlabeled(index), str::to_string)).collect();
    let mut seen = std::collections::HashSet::new();
    let duplicates: Vec<usize> = (0..names.len()).filter(|&index| !seen.insert(names[index].clone())).collect();
    let mut counters: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for index in duplicates {
        let base = names[index].clone();
        let mut counter = counters.get(&base).copied().unwrap_or(1);
        let name = loop {
            let candidate = format!("{base}_{counter}");
            if seen.insert(candidate.clone()) {
                break candidate;
            }
            counter += 1;
        };
        counters.insert(name.clone(), counter + 1);
        names[index] = name;
    }
    names
}

fn optional_parameter_display(ty: &Type) -> String {
    if !crate::strict_null_checks() {
        return ty.name();
    }
    match ty {
        Type::Any | Type::ErrorType | Type::Unknown | Type::GenuineUnknown | Type::Undefined => ty.name(),
        Type::Union(union) if union.types().iter().any(|member| matches!(member.peeled(), Type::Undefined)) => ty.name(),
        _ => crate::tsc_display::with_undefined(ty),
    }
}
