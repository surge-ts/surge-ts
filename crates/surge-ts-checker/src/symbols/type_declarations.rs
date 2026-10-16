use std::collections::HashMap;
use std::sync::Arc;

use surge_ts_syntax::{
    ParsedFunctionType, ParsedInterfaceMember, ParsedNamedType, ParsedType, ParsedTypeParameter,
    TextSpan,
};

/// The heavy, immutable payload of a type alias declaration.
///
/// Held behind an `Arc` in [`TypeAliasInfo`] so that re-export/import rebinding,
/// which only rewrites the per-binding header (`name`, `resolution_scope`), shares
/// the parsed alias tree by pointer instead of deep-cloning it. The body is never
/// mutated after declaration collection; declaration merging builds a fresh body.
#[derive(Debug)]
pub(crate) struct TypeAliasBody {
    pub(crate) type_parameters: Vec<ParsedTypeParameter>,
    pub(crate) ty: ParsedType,
}

impl Clone for TypeAliasBody {
    fn clone(&self) -> Self {
        crate::program::record_type_declaration_payload_deep_clone_count();
        Self {
            type_parameters: self.type_parameters.clone(),
            ty: self.ty.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct TypeAliasInfo {
    pub(crate) name: Arc<str>,
    /// The name this declaration was originally declared under at its source,
    /// captured the first time it is renamed by a re-export/import (`import type
    /// { Box as ABox }`). Used only for diagnostic display so messages show the
    /// original name (`Box<string>`) rather than the local binding (`ABox`).
    pub(crate) declared_name: Option<Arc<str>>,
    pub(crate) file_name: Arc<str>,
    pub(crate) name_span: Option<TextSpan>,
    pub(crate) resolution_scope: Option<Arc<TypeDeclarationScope>>,
    pub(crate) body: Arc<TypeAliasBody>,
    /// The enum this alias stands for, when it was synthesized by lowering an
    /// `enum` (see [`surge_ts_syntax::ParsedTypeAliasDeclaration::enum_name`]).
    pub(crate) enum_name: Option<Arc<str>>,
    /// Whether that enum was exported; tsc qualifies only an exported one.
    pub(crate) enum_exported: bool,
    /// Whether that enum is a `const enum`, whose object only a string
    /// literal may index (`isConstEnumObjectType`).
    pub(crate) enum_is_const: bool,
    /// On an enum's own alias, its members' names and lowered types (see
    /// [`surge_ts_syntax::ParsedTypeAliasDeclaration::enum_members`]).
    pub(crate) enum_members: Arc<[(String, ParsedType)]>,
    /// Memoized resolution-cache key (canonical file name + declared name).
    /// Built on first request — key construction canonicalizes the path and
    /// allocates, and resolution asks for it millions of times per run. Carried
    /// across clones (the key is a pure function of `file_name`/`name`) and
    /// reset by `rename_type_declaration`, the only place that rewrites `name`.
    pub(crate) cached_resolution_key: std::sync::OnceLock<crate::context::DeclarationResolutionKey>,
    /// Memoized nominal reference id (`"{canonical file}\0{name}"`), the other
    /// per-resolution allocation on the named-type hot path. Same lifecycle as
    /// [`Self::cached_resolution_key`].
    pub(crate) cached_alias_id: std::sync::OnceLock<Arc<str>>,
}

impl TypeAliasInfo {
    pub(crate) fn new(
        name: impl Into<Arc<str>>,
        file_name: impl Into<Arc<str>>,
        name_span: Option<TextSpan>,
        type_parameters: Vec<ParsedTypeParameter>,
        ty: ParsedType,
        resolution_scope: Option<Arc<TypeDeclarationScope>>,
    ) -> Self {
        Self {
            name: name.into(),
            declared_name: None,
            file_name: file_name.into(),
            name_span,
            resolution_scope,
            body: Arc::new(TypeAliasBody {
                type_parameters,
                ty,
            }),
            enum_name: None,
            enum_exported: false,
            enum_is_const: false,
            enum_members: Arc::from([]),
            cached_resolution_key: std::sync::OnceLock::new(),
            cached_alias_id: std::sync::OnceLock::new(),
        }
    }

    /// Marks this alias as standing for an enum, so its resolution can carry the
    /// enum's nominal display.
    pub(crate) fn with_enum_name(
        mut self,
        enum_name: Option<&str>,
        exported: bool,
        is_const: bool,
    ) -> Self {
        self.enum_name = enum_name.map(Arc::from);
        self.enum_exported = exported;
        self.enum_is_const = is_const;
        self
    }

    pub(crate) fn with_enum_members(mut self, members: &[(String, ParsedType)]) -> Self {
        if !members.is_empty() {
            self.enum_members = Arc::from(members);
        }
        self
    }
}

impl Clone for TypeAliasInfo {
    fn clone(&self) -> Self {
        crate::program::record_type_declaration_header_copy_count();
        Self {
            name: self.name.clone(),
            declared_name: self.declared_name.clone(),
            file_name: self.file_name.clone(),
            name_span: self.name_span,
            resolution_scope: self.resolution_scope.clone(),
            body: self.body.clone(),
            enum_name: self.enum_name.clone(),
            enum_exported: self.enum_exported,
            enum_is_const: self.enum_is_const,
            enum_members: self.enum_members.clone(),
            cached_resolution_key: self.cached_resolution_key.clone(),
            cached_alias_id: self.cached_alias_id.clone(),
        }
    }
}

/// The heavy, immutable payload of an interface declaration. See [`TypeAliasBody`]
/// for the body/header sharing rationale.
#[derive(Debug)]
pub(crate) struct InterfaceBody {
    pub(crate) type_parameters: Vec<ParsedTypeParameter>,
    pub(crate) extends: Vec<ParsedNamedType>,
    pub(crate) members: Vec<ParsedInterfaceMember>,
    pub(crate) string_index_type: Option<ParsedType>,
    /// See [`surge_ts_syntax::ParsedObjectType::number_index_type`].
    pub(crate) number_index_type: Option<ParsedType>,
    /// See [`surge_ts_syntax::ParsedObjectType::string_index_readonly`].
    pub(crate) string_index_readonly: bool,
    pub(crate) number_index_readonly: bool,
    /// See [`surge_ts_syntax::ParsedKeyedIndexes`].
    pub(crate) keyed_indexes: Option<Arc<surge_ts_syntax::ParsedKeyedIndexes>>,
    pub(crate) index_parameter_names: Option<Arc<surge_ts_syntax::ParsedIndexParameterNames>>,
    pub(crate) call_signature: Option<ParsedFunctionType>,
    /// See [`surge_ts_syntax::ParsedInterfaceDeclaration::call_signature_overloads`].
    pub(crate) call_signature_overloads: Vec<ParsedFunctionType>,
    pub(crate) construct_signatures: Vec<ParsedFunctionType>,
    pub(crate) declaration_fragments: Vec<InterfaceDeclarationFragmentId>,
    pub(crate) member_fragments: Vec<InterfaceDeclarationFragmentId>,
    /// The fragment each `extends` entry was written in, parallel to `extends`.
    pub(crate) extends_fragments: Vec<InterfaceDeclarationFragmentId>,
    /// Fragments written in a `declare global` block of a module or of an
    /// ambient module declaration. They merge into the global declaration but
    /// resolve where they are written, under that file's module scope:
    /// `@types/node` reopens the global `EventTarget` with `extends
    /// __EventTarget`, a module-local alias, and `URL` with `extends _URL`, an
    /// import inside `declare module "url"`.
    pub(crate) module_global_fragments: Vec<InterfaceDeclarationFragmentId>,
    /// Resolution scopes for fragments written in another file — a `declare
    /// module` augmentation. The merged declaration resolves under the
    /// *augmented* file's scope, where a name the augmenting file declared
    /// locally does not exist, so those members carry their own scope here.
    /// Empty for every declaration that was never augmented.
    pub(crate) fragment_scopes: Vec<(InterfaceDeclarationFragmentId, Arc<TypeDeclarationScope>)>,
    /// The `private`/`protected` members of the class this is the instance side
    /// of, static ones included; empty for a plain interface. Only
    /// accessibility checks read it — the shape never depends on it — and it
    /// lives in the shared body rather than the often-copied header.
    pub(crate) restricted_members: Vec<surge_ts_syntax::ParsedRestrictedMember>,
    /// Every member the body of the class this is the instance side of
    /// writes, static ones included; empty for a plain interface. Like
    /// `restricted_members`, read only by checks that ask about a member's
    /// declaration rather than its type.
    pub(crate) class_members: Vec<ClassMemberSymbol>,
    /// The `symbol` index signature of the class this is the instance side
    /// of, which the instance type does not carry: only the index-constraint
    /// check of a derived class reads it.
    pub(crate) class_symbol_index: Option<ParsedType>,
}

/// A member a class body writes, with the binder's symbol flags for it that
/// the member's type does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassMemberSymbol {
    pub(crate) name: String,
    pub(crate) is_static: bool,
    /// `SymbolFlagsGetAccessor`: a `get` accessor, alone or paired.
    pub(crate) is_get_accessor: bool,
    /// A property declaration (`isClassInstanceProperty`), as opposed to a
    /// method or accessor.
    pub(crate) is_field: bool,
}

impl InterfaceBody {
    /// Whether some fragment resolves under a scope other than the merged
    /// declaration's own.
    pub(crate) fn has_foreign_fragments(&self) -> bool {
        !self.fragment_scopes.is_empty() || !self.module_global_fragments.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct InterfaceDeclarationFragmentId {
    /// Shared: every member fragment of one declaration points at the same
    /// file string (a per-member owned copy multiplied a path across every
    /// interface member of every dependency `.d.ts`).
    pub(crate) file_name: Arc<str>,
    pub(crate) declaration_start: usize,
}

impl Clone for InterfaceBody {
    fn clone(&self) -> Self {
        crate::program::record_type_declaration_payload_deep_clone_count();
        Self {
            type_parameters: self.type_parameters.clone(),
            extends: self.extends.clone(),
            members: self.members.clone(),
            string_index_type: self.string_index_type.clone(),
            number_index_type: self.number_index_type.clone(),
            string_index_readonly: self.string_index_readonly,
            number_index_readonly: self.number_index_readonly,
            keyed_indexes: self.keyed_indexes.clone(),
            index_parameter_names: self.index_parameter_names.clone(),
            call_signature: self.call_signature.clone(),
            call_signature_overloads: self.call_signature_overloads.clone(),
            construct_signatures: self.construct_signatures.clone(),
            declaration_fragments: self.declaration_fragments.clone(),
            member_fragments: self.member_fragments.clone(),
            extends_fragments: self.extends_fragments.clone(),
            module_global_fragments: self.module_global_fragments.clone(),
            fragment_scopes: self.fragment_scopes.clone(),
            restricted_members: self.restricted_members.clone(),
            class_members: self.class_members.clone(),
            class_symbol_index: self.class_symbol_index.clone(),
        }
    }
}

/// Every call signature a body declares: the overload list when it has one,
/// otherwise the single signature it folded to.
fn declared_call_signatures(body: &InterfaceBody) -> Vec<ParsedFunctionType> {
    if body.call_signature_overloads.is_empty() {
        body.call_signature.clone().into_iter().collect()
    } else {
        body.call_signature_overloads.clone()
    }
}

#[derive(Debug)]
pub(crate) struct InterfaceInfo {
    pub(crate) name: Arc<str>,
    /// See [`TypeAliasInfo::declared_name`]. Original source name, captured on the
    /// first re-export/import rename, for diagnostic display only.
    pub(crate) declared_name: Option<Arc<str>>,
    pub(crate) file_name: Arc<str>,
    pub(crate) name_span: Option<TextSpan>,
    pub(crate) resolution_scope: Option<Arc<TypeDeclarationScope>>,
    /// Set when this "interface" is the instance side of an `abstract class`.
    /// Only the abstract-instantiation check reads it; nothing about the shape
    /// depends on it.
    pub(crate) is_abstract_class: bool,
    /// The class body writes a constructor, and the modifier it carries. A
    /// class without one inherits its base's construct signatures, modifier
    /// included, which is what `new`/`extends` accessibility walks up to.
    pub(crate) declares_constructor: bool,
    pub(crate) constructor_accessibility: Option<surge_ts_syntax::ParsedMemberAccessibility>,
    /// Set when this is the instance side of a class. A class base is an
    /// expression, so its heritage names are reported by the class's own check
    /// rather than wherever the instance is first expanded.
    pub(crate) is_class_instance: bool,
    /// For a namespace member, the symbol table tsc's binder declares it in
    /// (`declareModuleMember`): the namespace's exports, which every block of
    /// the namespace shares, or its own block's locals. Declarations merge
    /// only within one table.
    pub(crate) namespace_member_table: Option<Arc<str>>,
    pub(crate) body: Arc<InterfaceBody>,
    /// See [`TypeAliasInfo::cached_resolution_key`].
    pub(crate) cached_resolution_key: std::sync::OnceLock<crate::context::DeclarationResolutionKey>,
    /// See [`TypeAliasInfo::cached_alias_id`].
    pub(crate) cached_alias_id: std::sync::OnceLock<Arc<str>>,
    /// Memoized stable declaration identity for the instantiation cache
    /// (`None` = the declaration is unstable and never cacheable). Depends on
    /// `file_name`, `name_span`, the declared name, and every merged fragment,
    /// so it must be reset wherever `declaration_fragments` grows on a cloned
    /// accumulator and wherever the declaration is renamed.
    pub(crate) cached_stable_id:
        std::sync::OnceLock<Option<crate::context::StableInterfaceDeclarationId>>,
    /// See [`InterfaceInfo::reads_own_members`]: the answer, with the member
    /// count it was decided for, since merging appends members in place.
    pub(crate) cached_self_reference: std::sync::OnceLock<(usize, bool)>,
}

impl InterfaceInfo {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: impl Into<Arc<str>>,
        file_name: impl Into<Arc<str>>,
        name_span: Option<TextSpan>,
        type_parameters: Vec<ParsedTypeParameter>,
        extends: Vec<ParsedNamedType>,
        members: Vec<ParsedInterfaceMember>,
        string_index_type: Option<ParsedType>,
        number_index_type: Option<ParsedType>,
        call_signature: Option<ParsedFunctionType>,
        call_signature_overloads: Vec<ParsedFunctionType>,
        construct_signatures: Vec<ParsedFunctionType>,
        resolution_scope: Option<Arc<TypeDeclarationScope>>,
    ) -> Self {
        let file_name = file_name.into();
        let declaration_fragment = InterfaceDeclarationFragmentId {
            file_name: file_name.clone(),
            declaration_start: name_span.map_or(0, |span| span.start),
        };
        let member_fragments = vec![declaration_fragment.clone(); members.len()];
        let extends_fragments = vec![declaration_fragment.clone(); extends.len()];
        Self {
            name: name.into(),
            declared_name: None,
            file_name,
            name_span,
            resolution_scope,
            is_abstract_class: false,
            declares_constructor: false,
            constructor_accessibility: None,
            is_class_instance: false,
            namespace_member_table: None,
            body: Arc::new(InterfaceBody {
                type_parameters,
                extends,
                members,
                string_index_type,
                number_index_type,
                string_index_readonly: false,
                number_index_readonly: false,
                keyed_indexes: None,
                index_parameter_names: None,
                call_signature,
                call_signature_overloads,
                construct_signatures,
                declaration_fragments: vec![declaration_fragment],
                member_fragments,
                extends_fragments,
                module_global_fragments: Vec::new(),
                fragment_scopes: Vec::new(),
                restricted_members: Vec::new(),
                class_members: Vec::new(),
                class_symbol_index: None,
            }),
            cached_resolution_key: std::sync::OnceLock::new(),
            cached_alias_id: std::sync::OnceLock::new(),
            cached_stable_id: std::sync::OnceLock::new(),
            cached_self_reference: std::sync::OnceLock::new(),
        }
    }

    /// The name a member annotation refers to this declaration by.
    pub(crate) fn own_name(&self) -> &str {
        let name = self.declared_name.as_deref().unwrap_or(&self.name);
        name.rsplit('.').next().unwrap_or(name)
    }

    /// Whether a property annotation reads members of this declaration
    /// (`$inferSelect: InferSelectModel<Table<T>>` inside `Table<T>`), which an
    /// eager expansion would read before it is built.
    pub(crate) fn reads_own_members(&self) -> bool {
        let member_count = self.body.members.len();
        if let Some(&(count, answer)) = self.cached_self_reference.get()
            && count == member_count
        {
            return answer;
        }
        let own_name = self.own_name();
        let answer = self.body.members.iter().any(|member| {
            !matches!(member.ty, ParsedType::Function(_))
                && reads_members_of(&member.ty, own_name, &member.name, &|_| true)
        });
        let _ = self.cached_self_reference.set((member_count, answer));
        answer
    }

    /// Marks the declaration's string and number index signatures `readonly`;
    /// see [`InterfaceBody::string_index_readonly`].
    pub(crate) fn with_readonly_indexes(mut self, string: bool, number: bool) -> Self {
        if string || number {
            let body = Arc::make_mut(&mut self.body);
            body.string_index_readonly = string;
            body.number_index_readonly = number;
        }
        self
    }

    pub(crate) fn with_keyed_indexes(
        mut self,
        keyed_indexes: Option<Arc<surge_ts_syntax::ParsedKeyedIndexes>>,
    ) -> Self {
        if keyed_indexes.is_some() {
            Arc::make_mut(&mut self.body).keyed_indexes = keyed_indexes;
        }
        self
    }

    pub(crate) fn with_index_parameter_names(
        mut self,
        names: Option<Arc<surge_ts_syntax::ParsedIndexParameterNames>>,
    ) -> Self {
        if names.is_some() {
            Arc::make_mut(&mut self.body).index_parameter_names = names;
        }
        self
    }
}

/// Whether `ty`, the annotation of the property `member`, reads members of
/// the declaration `name` — indexes it, takes its keys, or hands it to a type
/// alias — rather than naming it whole. Only the annotation's own shape
/// counts: a nested object or signature resolves along with the expansion.
/// Reading the property itself (`x: C['x']`) is a circularity, not a read.
pub(crate) fn reads_members_of(
    ty: &ParsedType,
    name: &str,
    member: &str,
    is_alias: &dyn Fn(&str) -> bool,
) -> bool {
    let names_it = |ty: &ParsedType| {
        let mut found = false;
        ty.for_each_named_type(&mut |named| found |= named.name == name);
        found
    };
    match ty {
        ParsedType::IndexedAccess(access) => {
            let mut innermost = access.as_ref();
            while let ParsedType::IndexedAccess(inner) = innermost.object_type.as_ref() {
                innermost = inner;
            }
            match (innermost.object_type.as_ref(), innermost.index_type.as_ref()) {
                (ParsedType::Named(named), ParsedType::StringLiteral(key)) if named.name == name => key != member,
                _ => reads_members_of(&innermost.object_type, name, member, is_alias),
            }
        }
        ParsedType::KeyOf(operand) => names_it(operand),
        ParsedType::Named(named) => {
            named.name != name && named.type_arguments.iter().any(names_it) && is_alias(&named.name)
        }
        ParsedType::Array(element) | ParsedType::Readonly(element) => {
            reads_members_of(element, name, member, is_alias)
        }
        ParsedType::Tuple(elements) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
            elements.iter().any(|element| reads_members_of(element, name, member, is_alias))
        }
        _ => false,
    }
}

/// The merged index of each key kind is the first declaration's that has one,
/// and so is the name it prints with.
fn merged_index_parameter_names(
    existing: &InterfaceBody,
    incoming: &InterfaceBody,
) -> Option<Arc<surge_ts_syntax::ParsedIndexParameterNames>> {
    let (Some(existing_names), Some(incoming_names)) =
        (&existing.index_parameter_names, &incoming.index_parameter_names)
    else {
        return existing.index_parameter_names.clone().or_else(|| incoming.index_parameter_names.clone());
    };
    let pick = |own: &Option<Arc<str>>, own_index: bool, other: &Option<Arc<str>>| {
        if own_index { own.clone() } else { other.clone() }
    };
    Some(Arc::new(surge_ts_syntax::ParsedIndexParameterNames {
        string: pick(&existing_names.string, existing.string_index_type.is_some(), &incoming_names.string),
        number: pick(&existing_names.number, existing.number_index_type.is_some(), &incoming_names.number),
    }))
}

/// Keyed indexes survive a merge only while at most one side declares a
/// string index, which the merged string slot could not tell apart.
fn merged_keyed_indexes(
    existing: &InterfaceBody,
    incoming: &InterfaceBody,
) -> Option<Arc<surge_ts_syntax::ParsedKeyedIndexes>> {
    match (&existing.string_index_type, &incoming.string_index_type) {
        (Some(_), None) => existing.keyed_indexes.clone(),
        (None, Some(_)) => incoming.keyed_indexes.clone(),
        (None, None) => existing.keyed_indexes.clone().or_else(|| incoming.keyed_indexes.clone()),
        (Some(_), Some(_)) => None,
    }
}

impl Clone for InterfaceInfo {
    fn clone(&self) -> Self {
        crate::program::record_type_declaration_header_copy_count();
        Self {
            name: self.name.clone(),
            declared_name: self.declared_name.clone(),
            file_name: self.file_name.clone(),
            name_span: self.name_span,
            resolution_scope: self.resolution_scope.clone(),
            is_abstract_class: self.is_abstract_class,
            declares_constructor: self.declares_constructor,
            constructor_accessibility: self.constructor_accessibility,
            is_class_instance: self.is_class_instance,
            namespace_member_table: self.namespace_member_table.clone(),
            body: self.body.clone(),
            cached_resolution_key: self.cached_resolution_key.clone(),
            cached_alias_id: self.cached_alias_id.clone(),
            cached_stable_id: self.cached_stable_id.clone(),
            cached_self_reference: self.cached_self_reference.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum TypeDeclarationInfo {
    Alias(TypeAliasInfo),
    Interface(InterfaceInfo),
}

impl Clone for TypeDeclarationInfo {
    fn clone(&self) -> Self {
        match self {
            Self::Alias(info) => Self::Alias(info.clone()),
            Self::Interface(info) => Self::Interface(info.clone()),
        }
    }
}

impl TypeDeclarationInfo {
    /// The name this declaration was originally declared under at its source,
    /// falling back to its current binding name when it was never renamed. Used
    /// for diagnostic display so an import-renamed type shows its original name
    /// (`Box`) rather than the local binding (`ABox`).
    pub(crate) fn declared_name(&self) -> &str {
        match self {
            Self::Alias(info) => info.declared_name.as_deref().unwrap_or(&info.name),
            Self::Interface(info) => info.declared_name.as_deref().unwrap_or(&info.name),
        }
    }

    /// The variances the declaration states for its type parameters, packed
    /// for its references: the `in`/`out` modifiers of every declaration
    /// (`getTypeParameterModifiers`). relater.go reads an alias's variances
    /// only for an object-typed instantiation, which an object literal,
    /// function or mapped body gives; a conditional body resolves to a branch
    /// that carries no alias.
    ///
    /// An unannotated parameter a `-?` mapping names in its key constraint
    /// measures `Unmeasurable`: `mappedTypeRelatedTo` instantiates that
    /// constraint with `reportUnmeasurableMapper` for such a source, and with
    /// `reportUnreliableMapper` otherwise. It relates the target's constraint
    /// to the source's and the source's template to the target's, so a
    /// parameter that is the whole constraint of a mapping without an `as`
    /// clause measures contravariant (and unreliable), and one that is the
    /// whole template covariant.
    ///
    /// A parameter passed on to another alias (`QueryFunctionContext<K, P>`)
    /// takes that alias's polarity at its position, found in the declaration's
    /// own resolution scope or, without one, by `lookup` in the scope it is
    /// resolved in.
    pub(crate) fn declared_variances(&self, lookup: DeclarationLookup<'_>) -> u32 {
        let (type_parameters, mapping) = match self {
            Self::Alias(info) => match &info.body.ty {
                ParsedType::Object(_) | ParsedType::Function(_) => (&info.body.type_parameters, None),
                ParsedType::Mapped(mapped) => (&info.body.type_parameters, Some(mapped)),
                _ => return 0,
            },
            Self::Interface(info) => (&info.body.type_parameters, None),
        };
        let names = |ty: &ParsedType, name: &str| {
            let mut named = false;
            ty.for_each_named_type(&mut |reference| {
                named |= reference.name == name && reference.type_arguments.is_empty();
            });
            named
        };
        let is_bare = |ty: &ParsedType, name: &str| {
            matches!(ty, ParsedType::Named(reference) if reference.name == name && reference.type_arguments.is_empty())
        };
        let measured = |name: &str| {
            let mapped = mapping?;
            let in_constraint = names(&mapped.constraint, name);
            if matches!(mapped.optional, surge_ts_syntax::MappedOptionality::Remove) && in_constraint {
                return Some(surge_ts_types::DeclaredVariance::Unmeasurable);
            }
            if mapped.name_type.is_some() {
                return None;
            }
            let in_template = names(&mapped.value_type, name);
            if is_bare(&mapped.constraint, name) && !in_template {
                Some(surge_ts_types::DeclaredVariance::UnreliableContravariant)
            } else if is_bare(&mapped.value_type, name) && !in_constraint {
                Some(surge_ts_types::DeclaredVariance::Covariant)
            } else {
                None
            }
        };
        let polarity = |name: &str| {
            if !surge_ts_types::strict_null_checks() {
                return None;
            }
            let (scope, root) = match self {
                Self::Alias(info) => (info.resolution_scope.clone(), info.name.clone()),
                Self::Interface(info) => (info.resolution_scope.clone(), info.name.clone()),
            };
            let mut polarity = Polarity::within(scope, lookup, 0);
            polarity.root = Some(root);
            match self {
                Self::Alias(info) => polarity.walk(&info.body.ty, name, true)?,
                Self::Interface(info) => polarity.interface(&info.body, name)?,
            }
            polarity.variance()
        };
        surge_ts_types::declared_variances(type_parameters.iter().map(|parameter| {
            surge_ts_types::DeclaredVariance::annotated(parameter.is_in, parameter.is_out)
                .or_else(|| measured(parameter.name.as_str()))
                .or_else(|| polarity(parameter.name.as_str()))
        }))
    }
}

/// Finds a type declaration by name in the scope being resolved in.
pub(crate) type DeclarationLookup<'a> = &'a dyn Fn(&str) -> Option<crate::symbols::TypeDeclarationHandle>;

/// Whether `ty` holds a conditional whose extends type names `name`.
fn conditional_extends_names(ty: &ParsedType, name: &str) -> bool {
    match ty {
        ParsedType::Conditional(conditional) => {
            parsed_type_names(&conditional.extends_type, name)
                || conditional_extends_names(&conditional.true_type, name)
                || conditional_extends_names(&conditional.false_type, name)
        }
        ParsedType::Union(members) | ParsedType::Intersection(members) | ParsedType::Tuple(members) => {
            members.iter().any(|member| conditional_extends_names(member, name))
        }
        ParsedType::Array(element) | ParsedType::Readonly(element) => conditional_extends_names(element, name),
        _ => false,
    }
}

/// How many aliases deep a parameter is followed through the arguments of
/// the references it is passed to.
const POLARITY_REFERENCE_DEPTH: usize = 3;

/// Where a type parameter occurs in a declaration's written shape, a
/// syntactic stand-in for tsc's marker-type variance measurement
/// (`getVariancesWorker`). A method's parameters relate bivariantly and
/// constrain nothing; a parameter passed to another alias takes that alias's
/// polarity at its position; anything else whose variance depends on another
/// declaration or on a type operator makes the walk give up (`None`).
struct Polarity<'a> {
    covariant: bool,
    contravariant: bool,
    /// Passed to a mapping whose own measurement is `Unmeasurable` (a `-?`
    /// over `keyof T`), which makes every reference relate structurally.
    unmeasurable: bool,
    /// The scope the walked declaration resolves its references in.
    scope: Option<Arc<TypeDeclarationScope>>,
    lookup: DeclarationLookup<'a>,
    depth: usize,
    /// The declaration being measured. A reference back to it is the
    /// in-progress measurement (`getVariances` answering `emptyArray`), which
    /// relates either way and constrains nothing.
    root: Option<Arc<str>>,
}

impl<'a> Polarity<'a> {
    fn within(scope: Option<Arc<TypeDeclarationScope>>, lookup: DeclarationLookup<'a>, depth: usize) -> Self {
        Self { covariant: false, contravariant: false, unmeasurable: false, scope, lookup, depth, root: None }
    }

    fn nested(&self) -> Self {
        let mut nested = Self::within(self.scope.clone(), self.lookup, self.depth);
        nested.root = self.root.clone();
        nested
    }

    fn variance(&self) -> Option<surge_ts_types::DeclaredVariance> {
        if self.unmeasurable {
            return Some(surge_ts_types::DeclaredVariance::Unmeasurable);
        }
        match (self.covariant, self.contravariant) {
            (true, true) => Some(surge_ts_types::DeclaredVariance::MeasuredInvariant),
            (false, true) => Some(surge_ts_types::DeclaredVariance::MeasuredContravariant),
            _ => None,
        }
    }

    fn interface(&mut self, body: &InterfaceBody, name: &str) -> Option<()> {
        if body.extends.iter().any(|base| {
            base.type_arguments
                .iter()
                .any(|argument| parsed_type_names(argument, name))
        }) {
            return None;
        }
        for member in &body.members {
            self.member(&member.ty, member.is_method, member.write_ty.as_ref(), name)?;
        }
        for index in [&body.string_index_type, &body.number_index_type].into_iter().flatten() {
            self.walk(index, name, true)?;
        }
        for signature in declared_call_signatures(body).iter().chain(&body.construct_signatures) {
            self.function(signature, name, true)?;
        }
        Some(())
    }

    fn member(
        &mut self,
        ty: &ParsedType,
        is_method: bool,
        write_ty: Option<&ParsedType>,
        name: &str,
    ) -> Option<()> {
        if let Some(write_ty) = write_ty {
            self.walk(write_ty, name, false)?;
        }
        match ty {
            ParsedType::Function(method) if is_method => {
                if method.type_parameters.iter().any(|parameter| parameter.name == name) {
                    return None;
                }
                self.walk(&method.return_type, name, true)
            }
            _ => self.walk(ty, name, true),
        }
    }

    fn function(&mut self, function: &ParsedFunctionType, name: &str, positive: bool) -> Option<()> {
        if function.type_parameters.iter().any(|parameter| {
            parameter.name == name
                || parameter.constraint.as_ref().is_some_and(|constraint| parsed_type_names(constraint, name))
                || parameter.default_type.as_ref().is_some_and(|default| parsed_type_names(default, name))
        }) {
            return None;
        }
        for parameter in &function.parameters {
            if parameter.is_this {
                if parsed_type_names(&parameter.ty, name) {
                    return None;
                }
                continue;
            }
            self.walk(&parameter.ty, name, !positive)?;
        }
        self.walk(&function.return_type, name, positive)
    }

    fn walk(&mut self, ty: &ParsedType, name: &str, positive: bool) -> Option<()> {
        match ty {
            ParsedType::Named(reference) if reference.name == name && reference.type_arguments.is_empty() => {
                if positive {
                    self.covariant = true;
                } else {
                    self.contravariant = true;
                }
                Some(())
            }
            ParsedType::Array(element) | ParsedType::Readonly(element) => self.walk(element, name, positive),
            // `keyof S` relates to `keyof T` when `T` relates to `S`. Over
            // anything but the bare parameter the keys may not depend on it
            // (`keyof T[]`), so that still gives up.
            ParsedType::KeyOf(operand)
                if matches!(operand.as_ref(), ParsedType::Named(reference)
                    if reference.name == name && reference.type_arguments.is_empty()) =>
            {
                self.walk(operand, name, !positive)
            }
            ParsedType::Tuple(elements) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
                elements.iter().try_for_each(|element| self.walk(element, name, positive))
            }
            ParsedType::VariadicTuple(elements) => elements.iter().try_for_each(|element| match element {
                surge_ts_syntax::ParsedTupleElement::Fixed(ty) | surge_ts_syntax::ParsedTupleElement::Rest(ty, _) => {
                    self.walk(ty, name, positive)
                }
            }),
            ParsedType::Function(function) => self.function(function, name, positive),
            ParsedType::Object(object) => {
                for property in &object.properties {
                    if positive {
                        self.member(&property.ty, property.is_method, property.write_ty.as_ref(), name)?;
                    } else {
                        let mut inner = self.nested();
                        inner.member(&property.ty, property.is_method, property.write_ty.as_ref(), name)?;
                        self.covariant |= inner.contravariant;
                        self.contravariant |= inner.covariant;
                    }
                }
                for index in [&object.string_index_type, &object.number_index_type].into_iter().flatten() {
                    self.walk(index, name, positive)?;
                }
                let calls = if object.call_signature_overloads.is_empty() {
                    object.call_signature.as_deref().into_iter().collect::<Vec<_>>()
                } else {
                    object.call_signature_overloads.iter().collect()
                };
                let constructs = if object.construct_signature_overloads.is_empty() {
                    object.construct_signature.as_deref().into_iter().collect::<Vec<_>>()
                } else {
                    object.construct_signature_overloads.iter().collect()
                };
                calls
                    .into_iter()
                    .chain(constructs)
                    .try_for_each(|signature| self.function(signature, name, positive))
            }
            ParsedType::Named(reference) if !reference.type_arguments.is_empty() && parsed_type_names(ty, name) => {
                self.reference(reference, name, positive)
            }
            // Two instantiations of one deferred conditional relate when their
            // check types relate either way and their branches relate
            // (`structuredTypeRelatedToWorker`), so only the branches give the
            // parameter a direction — unless the check type alone is
            // invariant in it. The extends types must be identical.
            ParsedType::Conditional(conditional) if parsed_type_names(ty, name) => {
                if parsed_type_names(&conditional.extends_type, name) {
                    return None;
                }
                if parsed_type_names(&conditional.check_type, name) {
                    let mut check = self.nested();
                    check.walk(&conditional.check_type, name, true)?;
                    if check.covariant && check.contravariant {
                        self.covariant = true;
                        self.contravariant = true;
                    }
                }
                self.walk(&conditional.true_type, name, positive)?;
                self.walk(&conditional.false_type, name, positive)
            }
            // `mappedTypeRelatedTo` relates a mapping's constraint through
            // `reportUnreliableMapper`, and a conditional under it relates only
            // with identical extends types: over `keyof name` with a template
            // testing `… extends name`, the measured variance cannot decide,
            // and two instantiations relate structurally.
            ParsedType::Mapped(mapped)
                if parsed_type_names(&mapped.constraint, name) && conditional_extends_names(&mapped.value_type, name) =>
            {
                self.unmeasurable = true;
                Some(())
            }
            _ if parsed_type_names(ty, name) => None,
            _ => Some(()),
        }
    }

    /// `Alias<…, name, …>`: each argument naming the parameter is walked in
    /// the direction the alias's own parameter at that position takes.
    fn reference(&mut self, reference: &ParsedNamedType, name: &str, positive: bool) -> Option<()> {
        if self.root.as_deref() == Some(reference.name.as_str()) {
            return Some(());
        }
        if self.depth >= POLARITY_REFERENCE_DEPTH {
            return None;
        }
        // A module scope holds no globals (`Required`, `Partial`).
        let handle = self
            .scope
            .as_ref()
            .and_then(|scope| scope.get_handle(&reference.name))
            .or_else(|| (self.lookup)(&reference.name))?;
        let alias = match handle.get() {
            TypeDeclarationInfo::Alias(alias) => alias,
            // An interface or class parameter written `in`/`out` relates the
            // way it is annotated (`getVariancesWorker` takes the annotation
            // as written); an unannotated one gives up as before.
            TypeDeclarationInfo::Interface(interface) => {
                for (index, argument) in reference.type_arguments.iter().enumerate() {
                    if !parsed_type_names(argument, name) {
                        continue;
                    }
                    let parameter = interface.body.type_parameters.get(index)?;
                    if !parameter.is_in && !parameter.is_out {
                        return None;
                    }
                    if parameter.is_out {
                        self.walk(argument, name, positive)?;
                    }
                    if parameter.is_in {
                        self.walk(argument, name, !positive)?;
                    }
                }
                return Some(());
            }
        };
        // A mapping is measured as a whole (`Required<T>`'s `-?` over `keyof T`
        // is `Unmeasurable`), not walked.
        let mapped_variances = matches!(alias.body.ty, ParsedType::Mapped(_))
            .then(|| handle.get().declared_variances(self.lookup));
        for (index, argument) in reference.type_arguments.iter().enumerate() {
            if !parsed_type_names(argument, name) {
                continue;
            }
            if let Some(variances) = mapped_variances {
                match surge_ts_types::declared_variance(variances, index) {
                    Some(surge_ts_types::DeclaredVariance::Unmeasurable) => self.unmeasurable = true,
                    Some(surge_ts_types::DeclaredVariance::Covariant) => self.walk(argument, name, positive)?,
                    Some(surge_ts_types::DeclaredVariance::UnreliableContravariant) => {
                        self.walk(argument, name, !positive)?
                    }
                    _ => return None,
                }
                continue;
            }
            let parameter = alias.body.type_parameters.get(index)?;
            let mut inner = Polarity::within(alias.resolution_scope.clone(), self.lookup, self.depth + 1);
            inner.root = self.root.clone();
            inner.walk(&alias.body.ty, &parameter.name, true)?;
            if inner.covariant {
                self.walk(argument, name, positive)?;
            }
            if inner.contravariant {
                self.walk(argument, name, !positive)?;
            }
        }
        Some(())
    }
}

fn parsed_type_names(ty: &ParsedType, name: &str) -> bool {
    let mut named = false;
    ty.for_each_named_type(&mut |reference| named |= reference.name == name);
    named
}

/// Merge two interface declarations following TypeScript declaration merging:
/// the existing members come first, then the incoming members. `extends` clauses
/// concatenate. Same-named method members are preserved (overloads); a property
/// already declared by the existing interface wins (matching TypeScript, which
/// keeps the first declaration's type), so a later conflicting property is
/// dropped here. Property conflict *reporting* is the caller's responsibility.
pub(crate) fn merge_interface_infos(
    existing: &InterfaceInfo,
    incoming: &InterfaceInfo,
) -> InterfaceInfo {
    // The same physical declarations folded in twice duplicate every member and
    // every heritage clause: `declaration_fragments` chains without dedup, and
    // an augmentation can now reach a consumer both through the target's own
    // declaration table and through the specifier-keyed export-table patch.
    if !incoming.body.declaration_fragments.is_empty()
        && incoming
            .body
            .declaration_fragments
            .iter()
            .all(|fragment| existing.body.declaration_fragments.contains(fragment))
    {
        return existing.clone();
    }

    let is_method = |member: &ParsedInterfaceMember| matches!(member.ty, ParsedType::Function(_));
    let existing_property_names: std::collections::HashSet<&str> = existing
        .body
        .members
        .iter()
        .filter(|member| !is_method(member))
        .map(|member| member.name.as_str())
        .collect();

    let mut members = existing.body.members.clone();
    let mut member_fragments = existing.body.member_fragments.clone();
    for (member, fragment) in incoming
        .body
        .members
        .iter()
        .zip(incoming.body.member_fragments.iter())
    {
        if !is_method(member) && existing_property_names.contains(member.name.as_str()) {
            continue;
        }
        members.push(member.clone());
        member_fragments.push(fragment.clone());
    }
    let mut extends = existing.body.extends.clone();
    extends.extend(incoming.body.extends.iter().cloned());
    let mut type_parameters = existing.body.type_parameters.clone();
    merge_type_parameters(&mut type_parameters, &incoming.body.type_parameters);
    let mut merged_info = InterfaceInfo::new(
        existing.name.clone(),
        existing.file_name.clone(),
        existing.name_span,
        type_parameters,
        extends,
        members,
        existing
            .body
            .string_index_type
            .clone()
            .or_else(|| incoming.body.string_index_type.clone()),
        existing
            .body
            .number_index_type
            .clone()
            .or_else(|| incoming.body.number_index_type.clone()),
        // One permissive fold over every declaration's signatures, as the
        // parser folds one declaration's (`merge_parsed_call_signatures`); the
        // first declaration's fold alone made a call matching a later
        // declaration's signature a false TS2554.
        match (&existing.body.call_signature, &incoming.body.call_signature) {
            (Some(existing), Some(incoming)) => {
                Some(surge_ts_syntax::merge_parsed_call_signatures(existing, incoming))
            }
            (existing, incoming) => existing.clone().or_else(|| incoming.clone()),
        },
        {
            // Merged declarations contribute their call signatures in
            // declaration order, so a call resolves against any of them.
            let mut merged = declared_call_signatures(&existing.body);
            merged.extend(declared_call_signatures(&incoming.body));
            if merged.len() > 1 { merged } else { Vec::new() }
        },
        {
            let mut merged = existing.body.construct_signatures.clone();
            merged.extend(incoming.body.construct_signatures.iter().cloned());
            merged
        },
        existing
            .resolution_scope
            .clone()
            .or_else(|| incoming.resolution_scope.clone()),
    );
    Arc::make_mut(&mut merged_info.body).declaration_fragments = existing
        .body
        .declaration_fragments
        .iter()
        .chain(incoming.body.declaration_fragments.iter())
        .cloned()
        .collect();
    Arc::make_mut(&mut merged_info.body).member_fragments = member_fragments;
    Arc::make_mut(&mut merged_info.body).extends_fragments = existing
        .body
        .extends_fragments
        .iter()
        .chain(incoming.body.extends_fragments.iter())
        .cloned()
        .collect();
    let mut module_global_fragments = existing.body.module_global_fragments.clone();
    for fragment in &incoming.body.module_global_fragments {
        if !module_global_fragments.contains(fragment) {
            module_global_fragments.push(fragment.clone());
        }
    }
    Arc::make_mut(&mut merged_info.body).module_global_fragments = module_global_fragments;
    // Each index signature keeps the modifier of the declaration it came from.
    {
        let body = Arc::make_mut(&mut merged_info.body);
        body.string_index_readonly = if existing.body.string_index_type.is_some() {
            existing.body.string_index_readonly
        } else {
            incoming.body.string_index_readonly
        };
        body.number_index_readonly = if existing.body.number_index_type.is_some() {
            existing.body.number_index_readonly
        } else {
            incoming.body.number_index_readonly
        };
        body.keyed_indexes = merged_keyed_indexes(&existing.body, &incoming.body);
        body.index_parameter_names = merged_index_parameter_names(&existing.body, &incoming.body);
    }
    // The merged symbol keeps tsc's `SymbolFlagsClass` when either side is one.
    merged_info.is_class_instance = existing.is_class_instance || incoming.is_class_instance;
    // `new` reads the modifiers of the symbol's value declaration, its first
    // class (`resolveNewExpression`): an interface merged with an `abstract
    // class` in either order leaves it abstract, and a later class of the same
    // name is a duplicate the name does not denote.
    let class_side = if existing.is_class_instance { existing } else { incoming };
    merged_info.is_abstract_class = class_side.is_abstract_class;
    merged_info.declares_constructor = class_side.declares_constructor;
    merged_info.constructor_accessibility = class_side.constructor_accessibility;
    let mut fragment_scopes = existing.body.fragment_scopes.clone();
    for entry in &incoming.body.fragment_scopes {
        if !fragment_scopes
            .iter()
            .any(|(fragment, _)| fragment == &entry.0)
        {
            fragment_scopes.push(entry.clone());
        }
    }
    Arc::make_mut(&mut merged_info.body).fragment_scopes = fragment_scopes;
    if !existing.body.restricted_members.is_empty() || !incoming.body.restricted_members.is_empty() {
        Arc::make_mut(&mut merged_info.body).restricted_members = existing
            .body
            .restricted_members
            .iter()
            .chain(incoming.body.restricted_members.iter())
            .cloned()
            .collect();
    }
    if !existing.body.class_members.is_empty() || !incoming.body.class_members.is_empty() {
        Arc::make_mut(&mut merged_info.body).class_members = existing
            .body
            .class_members
            .iter()
            .chain(incoming.body.class_members.iter())
            .cloned()
            .collect();
    }
    merged_info
}

/// tsc's binder declares each declaration's type parameters in the merged
/// symbol's members, so a name an earlier declaration declared is the same
/// parameter and any other name is one more: the merged type's parameters are
/// every name in order of first appearance
/// (`appendLocalTypeParametersOfClassOrInterfaceOrTypeAlias`). A parameter is
/// then read off all of its declarations: its constraint and its default are
/// the first ones written (`getConstraintDeclaration`,
/// `getResolvedTypeParameterDefault`), so `interface I<T>` merged with
/// `interface I<T = number>` takes `I` with no argument, and a modifier on any
/// declaration applies (`getTypeParameterModifiers`).
fn merge_type_parameters(
    merged: &mut Vec<ParsedTypeParameter>,
    incoming: &[ParsedTypeParameter],
) {
    for parameter in incoming {
        let Some(known) = merged.iter_mut().find(|known| known.name == parameter.name) else {
            merged.push(parameter.clone());
            continue;
        };
        if known.constraint.is_none() {
            known.constraint = parameter.constraint.clone();
        }
        if known.default_type.is_none() {
            known.default_type = parameter.default_type.clone();
        }
        known.is_const |= parameter.is_const;
        known.is_in |= parameter.is_in;
        known.is_out |= parameter.is_out;
    }
}

/// The scope a global declaration resolves under. It has nothing of its own, so
/// a name falls through to the globals — the lexical scope tsc gives a global
/// declaration — instead of to whichever module's scope the reference that
/// reached the declaration was written in: a module's local
/// `IteratorYieldResult` must not become the lib's `IteratorResult` member.
pub(crate) fn global_declaration_scope() -> Arc<TypeDeclarationScope> {
    GLOBAL_DECLARATION_SCOPE
        .get_or_init(|| Arc::new(TypeDeclarationScope::new(Vec::new())))
        .clone()
}

static GLOBAL_DECLARATION_SCOPE: std::sync::OnceLock<Arc<TypeDeclarationScope>> = std::sync::OnceLock::new();

pub(crate) fn is_global_declaration_scope(scope: &Arc<TypeDeclarationScope>) -> bool {
    GLOBAL_DECLARATION_SCOPE
        .get()
        .is_some_and(|global| Arc::ptr_eq(global, scope))
}

/// Gives every declaration of a global table that has no resolution scope of
/// its own the [`global_declaration_scope`].
pub(crate) fn attach_global_declaration_scope(table: &TypeDeclarationTable) -> TypeDeclarationTable {
    let scope = global_declaration_scope();
    let mut attached = TypeDeclarationTable::new();
    for (name, declaration) in table.iter() {
        let declaration = match declaration.clone() {
            TypeDeclarationInfo::Alias(mut alias) => {
                alias.resolution_scope.get_or_insert_with(|| scope.clone());
                TypeDeclarationInfo::Alias(alias)
            }
            TypeDeclarationInfo::Interface(mut interface) => {
                interface.resolution_scope.get_or_insert_with(|| scope.clone());
                TypeDeclarationInfo::Interface(interface)
            }
        };
        let _ = attached.insert(name.as_ref(), declaration);
    }
    attached
}

/// Marks every interface of a `declare global` block as resolving under its
/// file's module scope wherever it merges.
pub(crate) fn mark_module_global_fragments(table: &TypeDeclarationTable) -> TypeDeclarationTable {
    let mut marked = TypeDeclarationTable::new();
    for (name, declaration) in table.iter() {
        let declaration = match declaration.clone() {
            TypeDeclarationInfo::Interface(mut interface) => {
                let body = Arc::make_mut(&mut interface.body);
                body.module_global_fragments = body.declaration_fragments.clone();
                TypeDeclarationInfo::Interface(interface)
            }
            alias => alias,
        };
        let _ = marked.insert(name.as_ref(), declaration);
    }
    marked
}

/// Declaration-merge an interface contributed by a `declare module` block in
/// another file. Identical to [`merge_type_declaration_into_table`] except that
/// the incoming fragments keep their own resolution scope: the merged
/// declaration resolves under the augmented file's scope, where a type the
/// augmenting file declared locally (`type WithPersist<S> = …`, named by the
/// member it adds) does not exist.
pub(crate) fn merge_augmentation_type_declaration_into_table(
    table: &mut TypeDeclarationTable,
    name: &str,
    incoming: &TypeDeclarationInfo,
) {
    let augmented_unscoped = matches!(
        table.get(name),
        Some(TypeDeclarationInfo::Interface(existing)) if existing.resolution_scope.is_none()
    );
    merge_type_declaration_into_table(table, name, incoming);
    let (TypeDeclarationInfo::Interface(incoming), Some(scope)) = (
        incoming,
        match incoming {
            TypeDeclarationInfo::Interface(info) => info.resolution_scope.clone(),
            TypeDeclarationInfo::Alias(_) => None,
        },
    ) else {
        return;
    };
    let Some(TypeDeclarationInfo::Interface(merged)) = table.get(name) else {
        return;
    };
    if merged
        .body
        .declaration_fragments
        .iter()
        .all(|fragment| incoming.body.declaration_fragments.contains(fragment))
    {
        // The augmentation is the only fragment, so the declaration already
        // resolves under its own scope.
        return;
    }
    let mut merged = merged.clone();
    // The augmented declaration's own fragments (and its `extends`) resolve
    // where they are written; the augmentation's scope reaches only its own
    // fragments, through `fragment_scopes`. Keeping it as the merged scope
    // made `extends BaseNode` in the augmented file name the augmentation's
    // `BaseNode` fragment alone.
    if augmented_unscoped
        && merged
            .resolution_scope
            .as_ref()
            .is_some_and(|merged_scope| Arc::ptr_eq(merged_scope, &scope))
    {
        merged.resolution_scope = None;
    }
    let body = Arc::make_mut(&mut merged.body);
    for fragment in &incoming.body.declaration_fragments {
        if !body
            .fragment_scopes
            .iter()
            .any(|(existing, _)| existing == fragment)
        {
            body.fragment_scopes
                .push((fragment.clone(), scope.clone()));
        }
    }
    table.upsert(name, TypeDeclarationInfo::Interface(merged));
}

/// Insert `incoming` into `table`, merging into an existing interface of the same
/// name (declaration merging) rather than dropping the later declaration.
/// Non-interface collisions keep the first declaration (first-wins), matching a
/// `var`/interface pair where the interface supplies the type.
pub(crate) fn merge_type_declaration_into_table(
    table: &mut TypeDeclarationTable,
    name: &str,
    incoming: &TypeDeclarationInfo,
) {
    enum Action {
        Merge(Box<InterfaceInfo>),
        KeepFirst,
        Insert,
    }

    let action = match (table.get(name), incoming) {
        (
            Some(TypeDeclarationInfo::Interface(existing)),
            TypeDeclarationInfo::Interface(incoming),
        ) => Action::Merge(Box::new(merge_interface_infos(existing, incoming))),
        (Some(_), _) => Action::KeepFirst,
        (None, _) => Action::Insert,
    };

    match action {
        Action::Merge(merged) => table.upsert(name, TypeDeclarationInfo::Interface(*merged)),
        Action::KeepFirst => {}
        Action::Insert => {
            let _ = table.insert(name, incoming.clone());
        }
    }
}

/// Merge every declaration of `source` into `dest`, sharing unchanged payloads
/// and allocating a fresh payload only for merged interfaces.
pub(crate) fn merge_shared_table_into(
    dest: &mut TypeDeclarationTable,
    source: &TypeDeclarationTable,
) {
    enum Action {
        Merge(Box<InterfaceInfo>),
        KeepFirst,
        Share(Arc<TypeDeclarationInfo>),
    }

    for (name, incoming) in source.declarations.iter() {
        let name_ref = name.as_ref();
        let action = match (dest.get(name_ref), incoming.as_ref()) {
            (
                Some(TypeDeclarationInfo::Interface(existing)),
                TypeDeclarationInfo::Interface(incoming),
            ) => Action::Merge(Box::new(merge_interface_infos(existing, incoming))),
            (Some(_), _) => Action::KeepFirst,
            (None, _) => Action::Share(incoming.clone()),
        };

        match action {
            Action::Merge(merged) => dest.upsert(name_ref, TypeDeclarationInfo::Interface(*merged)),
            Action::KeepFirst => {}
            Action::Share(declaration) => dest.insert_shared_handle(name.clone(), declaration),
        }
    }
}

/// Bulk variant of [`merge_shared_table_into`] that merges many sources into
/// `dest` in one pass. Applying the single-source merge once per source rebuilds a
/// growing interface's member list on every merge, so a global interface split
/// across N files (`declare global { interface X { ... } }`) was O(N^2). Here each
/// interface is folded into one owned accumulator whose property set is maintained
/// incrementally, making the whole merge O(total members).
pub(crate) fn merge_shared_tables_into(
    dest: &mut TypeDeclarationTable,
    sources: &[TypeDeclarationTable],
) {
    let is_method = |member: &ParsedInterfaceMember| matches!(member.ty, ParsedType::Function(_));
    // name -> (accumulated interface, its non-method property names)
    let mut interfaces: HashMap<String, (InterfaceInfo, std::collections::HashSet<String>)> =
        HashMap::new();

    for source in sources {
        for (name, incoming) in source.declarations.iter() {
            let name_ref = name.as_ref();

            let TypeDeclarationInfo::Interface(incoming_interface) = incoming.as_ref() else {
                // A later non-interface never overrides an earlier binding (first
                // wins), and an interface already accumulated for this name keeps it.
                if dest.get(name_ref).is_some() || interfaces.contains_key(name_ref) {
                    continue;
                }
                dest.insert_shared_handle(name.clone(), incoming.clone());
                continue;
            };

            if let Some((accumulator, seen)) = interfaces.get_mut(name_ref) {
                fold_interface_declaration(accumulator, seen, incoming_interface, &is_method);
                continue;
            }

            match dest.get(name_ref) {
                Some(TypeDeclarationInfo::Interface(existing)) => {
                    let mut accumulator = existing.clone();
                    let mut seen: std::collections::HashSet<String> = existing
                        .body
                        .members
                        .iter()
                        .filter(|member| !is_method(member))
                        .map(|member| member.name.clone())
                        .collect();
                    fold_interface_declaration(
                        &mut accumulator,
                        &mut seen,
                        incoming_interface,
                        &is_method,
                    );
                    interfaces.insert(name_ref.to_string(), (accumulator, seen));
                }
                // An existing non-interface binding wins over a later interface.
                Some(_) => {}
                None => {
                    let seen: std::collections::HashSet<String> = incoming_interface
                        .body
                        .members
                        .iter()
                        .filter(|member| !is_method(member))
                        .map(|member| member.name.clone())
                        .collect();
                    interfaces.insert(name_ref.to_string(), (incoming_interface.clone(), seen));
                }
            }
        }
    }

    for (name, (accumulator, _)) in interfaces {
        dest.upsert(&name, TypeDeclarationInfo::Interface(accumulator));
    }
}

/// Append `incoming`'s members to `accumulator` in place, mirroring
/// [`merge_interface_infos`] (methods always kept as overloads; a property already
/// declared wins) but reusing the accumulator's member list instead of cloning it,
/// and consulting an incrementally maintained `seen` set instead of rebuilding it.
fn fold_interface_declaration(
    accumulator: &mut InterfaceInfo,
    seen: &mut std::collections::HashSet<String>,
    incoming: &InterfaceInfo,
    is_method: &impl Fn(&ParsedInterfaceMember) -> bool,
) {
    // The accumulator is a clone of an existing declaration, so a memoized
    // stable id would go stale as the fragment list grows below.
    accumulator.cached_stable_id = std::sync::OnceLock::new();
    let body = Arc::make_mut(&mut accumulator.body);
    for (member, fragment) in incoming
        .body
        .members
        .iter()
        .zip(incoming.body.member_fragments.iter())
    {
        if !is_method(member) {
            if seen.contains(member.name.as_str()) {
                continue;
            }
            seen.insert(member.name.clone());
        }
        body.members.push(member.clone());
        body.member_fragments.push(fragment.clone());
    }
    body.extends.extend(incoming.body.extends.iter().cloned());
    body.extends_fragments
        .extend(incoming.body.extends_fragments.iter().cloned());
    for fragment in &incoming.body.module_global_fragments {
        if !body.module_global_fragments.contains(fragment) {
            body.module_global_fragments.push(fragment.clone());
        }
    }
    merge_type_parameters(&mut body.type_parameters, &incoming.body.type_parameters);
    if body.number_index_type.is_none() {
        body.number_index_type = incoming.body.number_index_type.clone();
        body.number_index_readonly = incoming.body.number_index_readonly;
    }
    body.keyed_indexes = merged_keyed_indexes(body, &incoming.body);
    body.index_parameter_names = merged_index_parameter_names(body, &incoming.body);
    if body.string_index_type.is_none() {
        body.string_index_type = incoming.body.string_index_type.clone();
        body.string_index_readonly = incoming.body.string_index_readonly;
    }
    if body.call_signature.is_none() {
        body.call_signature = incoming.body.call_signature.clone();
    }
    body.construct_signatures
        .extend(incoming.body.construct_signatures.iter().cloned());
    body.declaration_fragments
        .extend(incoming.body.declaration_fragments.iter().cloned());
    for entry in &incoming.body.fragment_scopes {
        if !body
            .fragment_scopes
            .iter()
            .any(|(fragment, _)| fragment == &entry.0)
        {
            body.fragment_scopes.push(entry.clone());
        }
    }
    if accumulator.resolution_scope.is_none() {
        accumulator.resolution_scope = incoming.resolution_scope.clone();
    }
}

#[derive(Clone)]
pub(crate) struct TypeDeclarationHandle {
    declaration: Arc<TypeDeclarationInfo>,
}

impl TypeDeclarationHandle {
    pub(crate) fn new(declaration: TypeDeclarationInfo) -> Self {
        Self { declaration: Arc::new(declaration) }
    }

    pub(crate) fn get(&self) -> &TypeDeclarationInfo {
        self.declaration.as_ref()
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.declaration, &other.declaration)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TypeDeclarationScope {
    layers: Vec<Arc<TypeDeclarationTable>>,
    /// A scope built before the file's imports were bound — it carries the
    /// file's own declarations only. A declaration exported with one of these
    /// attached resolves every imported name in its body to `unknown`, so the
    /// attachment is replaceable: see `attach_type_resolution_scope`.
    preliminary: bool,
    /// How many leading layers are lexically inside the file's own
    /// declarations — a function body's local types, and the placeholders of
    /// its enclosing type parameters — which a lookup reads before the file's
    /// table, as tsc's `resolveName` walks the inner scopes first.
    lexical_layers: usize,
}

impl TypeDeclarationScope {
    pub(crate) fn new(layers: Vec<Arc<TypeDeclarationTable>>) -> Self {
        Self {
            layers,
            preliminary: false,
            lexical_layers: 0,
        }
    }

    /// Marks the first `count` layers as lexical (see `lexical_layers`).
    pub(crate) fn with_lexical_layers(mut self, count: usize) -> Self {
        self.lexical_layers = count.min(self.layers.len());
        self
    }

    pub(crate) fn lexical_layer_count(&self) -> usize {
        self.lexical_layers
    }

    /// The declaration a lexical layer binds `name` to.
    pub(crate) fn lexical_get(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        self.layers[..self.lexical_layers]
            .iter()
            .find_map(|layer| layer.get_without_lookup_record(name))
    }

    /// [`Self::lexical_get`] as a handle.
    pub(crate) fn lexical_get_handle(&self, name: &str) -> Option<TypeDeclarationHandle> {
        self.layers[..self.lexical_layers]
            .iter()
            .find_map(|layer| layer.get_handle(name))
    }

    /// Marks a scope as built before import binding.
    pub(crate) fn preliminary(mut self) -> Self {
        self.preliminary = true;
        self
    }

    pub(crate) fn is_preliminary(&self) -> bool {
        self.preliminary
    }

    /// Census-only: layer table identities, for the scope↔table cycle probe.
    pub(crate) fn census_layer_addresses(&self) -> impl Iterator<Item = usize> + '_ {
        self.layers.iter().map(|layer| layer.identity_address())
    }

    pub(crate) fn get(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        let mut layers_visited = 0;

        for layer in &self.layers {
            layers_visited += 1;
            if let Some(declaration) = layer.get_without_lookup_record(name) {
                crate::program::record_declaration_lookup(layers_visited);
                return Some(declaration);
            }
        }

        crate::program::record_declaration_lookup(layers_visited);
        None
    }

    /// Debug-only (`SURGE_TRACE_HAD_ERROR`): per-layer sizes and a name sample.
    pub(crate) fn debug_layer_summary(&self) -> String {
        self.layers
            .iter()
            .map(|layer| {
                let names: Vec<&str> = layer.iter().map(|(name, _)| name.as_ref()).take(6).collect();
                format!("{}[{}]", layer.len(), names.join(","))
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    pub(crate) fn get_handle(&self, name: &str) -> Option<TypeDeclarationHandle> {
        let mut layers_visited = 0;

        for layer in &self.layers {
            layers_visited += 1;
            if let Some(handle) = layer.get_handle(name) {
                crate::program::record_declaration_lookup(layers_visited);
                return Some(handle);
            }
        }

        crate::program::record_declaration_lookup(layers_visited);
        None
    }

    #[allow(dead_code)]
    pub(crate) fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub(crate) fn layers(&self) -> &[Arc<TypeDeclarationTable>] {
        &self.layers
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.layers.iter().all(|layer| layer.len() == 0)
    }
}

#[derive(Debug)]
/// Shared top-level type-declaration namespace for aliases and interfaces.
///
/// The first declaration wins; later duplicates are reported by the caller and
/// must not replace the original entry.
pub(crate) struct TypeDeclarationTable {
    declarations:
        Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<TypeDeclarationInfo>>>,
    /// Instance identity + mutation counter. `(instance_id, version)` equality
    /// proves this exact table instance is bytewise-unchanged since a previous
    /// observation, letting the declaration-environment capture reuse one
    /// immutable snapshot across the many environments interned between
    /// mutations. Clones get a fresh identity (they diverge independently).
    instance_id: u64,
    version: u64,
}

fn next_table_instance_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for TypeDeclarationTable {
    fn default() -> Self {
        Self {
            declarations: Arc::new(Default::default()),
            instance_id: next_table_instance_id(),
            version: 0,
        }
    }
}

impl Clone for TypeDeclarationTable {
    fn clone(&self) -> Self {
        Self {
            declarations: self.declarations.clone(),
            instance_id: next_table_instance_id(),
            version: 0,
        }
    }
}

impl TypeDeclarationTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// See the `instance_id` field: equal pairs prove an unchanged instance.
    pub(crate) fn snapshot_identity(&self) -> (u64, u64) {
        (self.instance_id, self.version)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        crate::program::record_declaration_lookup(1);
        self.get_without_lookup_record(name)
    }

    fn get_without_lookup_record(&self, name: &str) -> Option<&TypeDeclarationInfo> {
        self.declarations.get(name).map(Arc::as_ref)
    }

    pub(crate) fn get_handle(&self, name: &str) -> Option<TypeDeclarationHandle> {
        Some(TypeDeclarationHandle {
            declaration: self.declarations.get(name)?.clone(),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.declarations.len()
    }

    /// Census-only identity of the shared COW map backing this table.
    pub(crate) fn identity_address(&self) -> usize {
        Arc::as_ptr(&self.declarations) as usize
    }

    /// Census-only estimate of this map backing's owned index memory.
    pub(crate) fn index_heap_bytes(&self) -> u64 {
        (self.declarations.capacity()
            * (std::mem::size_of::<Arc<str>>()
                + std::mem::size_of::<Arc<TypeDeclarationInfo>>()
                + 16)) as u64
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &TypeDeclarationInfo)> + '_ {
        self.declarations
            .iter()
            .map(|(name, declaration)| (name, declaration.as_ref()))
    }

    pub(crate) fn insert(
        &mut self,
        name: impl AsRef<str>,
        declaration: TypeDeclarationInfo,
    ) -> Option<TypeDeclarationInfo> {
        let name_ref = name.as_ref();
        if self.declarations.contains_key(name_ref) {
            return Some(declaration);
        }

        let key = declaration_key(name_ref, &declaration);
        Arc::make_mut(&mut self.declarations).insert(key, Arc::new(declaration));
        self.version += 1;
        None
    }

    /// Insert `declaration`, replacing any existing entry for `name`.
    ///
    /// Unlike [`insert`](Self::insert) (which is first-wins), this overwrites the
    /// existing payload. Used by declaration-merging paths.
    pub(crate) fn upsert(&mut self, name: impl AsRef<str>, declaration: TypeDeclarationInfo) {
        let name_ref = name.as_ref();
        let declarations = Arc::make_mut(&mut self.declarations);
        if let Some(existing) = declarations.get_mut(name_ref) {
            *existing = Arc::new(declaration);
        } else {
            let key = declaration_key(name_ref, &declaration);
            declarations.insert(key, Arc::new(declaration));
        }
        self.version += 1;
    }

    /// Shares `handle`'s payload under `name`, first-wins.
    pub(crate) fn insert_handle(&mut self, name: &str, handle: &TypeDeclarationHandle) {
        self.insert_shared_handle(Arc::from(name), handle.declaration.clone());
    }

    fn insert_shared_handle(
        &mut self,
        name: Arc<str>,
        declaration: Arc<TypeDeclarationInfo>,
    ) {
        if self.declarations.contains_key(name.as_ref()) {
            return;
        }
        Arc::make_mut(&mut self.declarations).insert(name, declaration);
        self.version += 1;
    }

    /// Share `source`'s payload for `source_name` under `name`, first-wins,
    /// without cloning the declaration. This is the per-importer qualified
    /// namespace binding path, where the declaration content is byte-identical.
    pub(crate) fn insert_shared_from(
        &mut self,
        name: &str,
        source: &TypeDeclarationTable,
        source_name: &str,
    ) -> bool {
        if self.declarations.contains_key(name) {
            return false;
        }
        let Some(declaration) = source.declarations.get(source_name).cloned() else {
            return false;
        };
        self.insert_shared_handle(Arc::from(name), declaration);
        true
    }
}

fn declaration_key(name: &str, declaration: &TypeDeclarationInfo) -> Arc<str> {
    let declared_name = match declaration {
        TypeDeclarationInfo::Alias(alias) => &alias.name,
        TypeDeclarationInfo::Interface(interface) => &interface.name,
    };
    if declared_name.as_ref() == name {
        declared_name.clone()
    } else {
        Arc::from(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_declaration_table_is_first_wins_across_kinds() {
        let mut table = TypeDeclarationTable::new();

        let alias = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            "User".to_string(),
            "a.ts".to_string(),
            None,
            vec![],
            ParsedType::String,
            None,
        ));
        let interface = TypeDeclarationInfo::Interface(InterfaceInfo::new(
            "User".to_string(),
            "b.ts".to_string(),
            None,
            vec![],
            vec![],
            vec![],
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
        ));

        assert!(table.insert("User", alias.clone()).is_none());
        assert!(table.insert("User", interface.clone()).is_some());
        assert!(matches!(
            table.get("User"),
            Some(TypeDeclarationInfo::Alias(_))
        ));

        let mut table = TypeDeclarationTable::new();
        assert!(table.insert("User", interface.clone()).is_none());
        assert!(table.insert("User", alias).is_some());
        assert!(matches!(
            table.get("User"),
            Some(TypeDeclarationInfo::Interface(_))
        ));
    }

    fn sample_alias(name: &str, file: &str) -> TypeDeclarationInfo {
        TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            name.to_string(),
            file.to_string(),
            None,
            vec![],
            ParsedType::String,
            None,
        ))
    }

    #[test]
    fn snapshot_identity_tracks_mutations_and_never_aliases_instances() {
        let mut table = TypeDeclarationTable::new();
        let unmodified = table.snapshot_identity();

        let _ = table.insert("User", sample_alias("User", "a.ts"));
        let mutated = table.snapshot_identity();
        assert_eq!(unmodified.0, mutated.0);
        assert_ne!(unmodified.1, mutated.1);

        let mut clone = table.clone();
        assert_ne!(clone.snapshot_identity().0, table.snapshot_identity().0);
        let _ = clone.insert("Other", sample_alias("Other", "b.ts"));
        assert_eq!(table.snapshot_identity(), mutated);
    }

    #[test]
    fn shared_payload_stays_valid_after_source_table_drops() {
        let mut source = TypeDeclarationTable::new();
        let _ = source.insert("User", sample_alias("User", "exporter.d.ts"));

        let mut importer = TypeDeclarationTable::new();
        assert!(importer.insert_shared_from("Renamed", &source, "User"));
        assert!(!importer.insert_shared_from("Renamed", &source, "User"));

        drop(source);
        assert!(matches!(
            importer.get("Renamed"),
            Some(TypeDeclarationInfo::Alias(_))
        ));

        let handle = importer.get_handle("Renamed").expect("shared entry");
        drop(importer);
        assert!(matches!(handle.get(), TypeDeclarationInfo::Alias(_)));
    }

    #[test]
    fn type_declaration_scope_honors_layer_precedence() {
        let mut ambient = TypeDeclarationTable::new();
        let ambient_alias = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            "User".to_string(),
            "ambient.d.ts".to_string(),
            None,
            vec![],
            ParsedType::String,
            None,
        ));
        let _ = ambient.insert("User", ambient_alias.clone());

        let mut local = TypeDeclarationTable::new();
        let local_interface = TypeDeclarationInfo::Interface(InterfaceInfo::new(
            "User".to_string(),
            "local.ts".to_string(),
            None,
            vec![],
            vec![],
            vec![],
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
        ));
        let _ = local.insert("User", local_interface.clone());

        let scope = TypeDeclarationScope::new(vec![Arc::new(local), Arc::new(ambient)]);

        assert_eq!(scope.layer_count(), 2);
        assert!(matches!(
            scope.get("User"),
            Some(TypeDeclarationInfo::Interface(_))
        ));

        let mut ambient_only = TypeDeclarationTable::new();
        let ambient_only_alias = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            "Buffer".to_string(),
            "ambient.d.ts".to_string(),
            None,
            vec![],
            ParsedType::Unknown,
            None,
        ));
        let _ = ambient_only.insert("Buffer", ambient_only_alias);
        let scope = TypeDeclarationScope::new(vec![
            Arc::new(TypeDeclarationTable::new()),
            Arc::new(ambient_only),
        ]);
        assert!(matches!(
            scope.get("Buffer"),
            Some(TypeDeclarationInfo::Alias(_))
        ));
    }
}
