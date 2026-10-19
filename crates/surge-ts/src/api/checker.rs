//! TypeScript's `TypeChecker`, answering from what surge's checker computed.
//!
//! Symbols come from the binder (see [`super::symbols`]). Types come from the
//! checked program: an expression's or declaration's type is the one the
//! checker recorded while checking its file, a named type is resolved with
//! the checker's own resolution in the file's scope. Where the checker has no
//! answer, the result is the error type (`any`, as tsc's `errorType` is),
//! never a guess.

use std::sync::Arc;

use surge_ts_checker::lowlevel::semantic::{PropertyInfo, Query};
use surge_ts_tsc_syntax::Kind;
use surge_ts_tsc_syntax::symbol_flags as sf;
use surge_ts_types::fx::FxHashMap;
use surge_ts_types::{FunctionType, Type};

use super::enums_generated::{SyntaxKind, object_flags as of, symbol_flags as ts_sf, type_flags as tf};
use super::handles::{NodeId, SignatureId, SymbolId, TypeId};
use super::node::NodePropertyValue;
use super::program::Program;
use super::symbols::{GlobalScope, Meaning, SymbolData, SymbolKey};

/// How tsc names a type alias's declared type.
enum Attach {
    /// By the alias itself.
    Alias,
    /// As the type its body writes, without the alias.
    None,
    /// By the instantiation of another alias its body writes.
    Target(String),
}

/// Whether a type mentions a type parameter anywhere it shows one.
fn contains_type_parameter(ty: &Type) -> bool {
    match ty {
        Type::TypeParameter(_) => true,
        Type::Union(union) => union.types().iter().any(contains_type_parameter),
        Type::Array(element) => contains_type_parameter(element),
        Type::Tuple(elements) => elements.iter().any(contains_type_parameter),
        Type::Reference(reference) => reference.arguments.iter().any(contains_type_parameter),
        Type::Function(function) => {
            function.parameters().iter().any(contains_type_parameter) || contains_type_parameter(function.return_type())
        }
        Type::Object(object) => object.properties.values().any(|property| contains_type_parameter(&property.ty)),
        _ => false,
    }
}

/// A `declare module "name"` symbol's name: the quoted module name.
fn is_ambient_module_name(name: &str) -> bool {
    name.len() >= 2 && ((name.starts_with('"') && name.ends_with('"')) || (name.starts_with('\'') && name.ends_with('\'')))
}

/// Whether a type's text joins operands with `separator` outside any
/// brackets or quotes.
fn has_top_level(text: &str, separator: &str) -> bool {
    let mut depth = 0usize;
    let mut quote = None;
    let bytes = text.as_bytes();
    for (index, character) in text.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(character),
            (None, '(' | '[' | '{' | '<') => depth += 1,
            (None, '>') if index > 0 && bytes[index - 1] == b'=' => {}
            (None, ')' | ']' | '}' | '>') => depth = depth.saturating_sub(1),
            (None, _) if depth == 0 && text[index..].starts_with(separator) => return true,
            _ => {}
        }
    }
    false
}

/// Whether `text` mentions one of `names` as a whole identifier.
fn mentions_name(text: &str, names: &[String]) -> bool {
    let identifier = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    names.iter().any(|name| {
        text.match_indices(name.as_str()).any(|(at, _)| {
            !text[..at].chars().next_back().is_some_and(identifier)
                && !text[at + name.len()..].chars().next().is_some_and(identifier)
        })
    })
}

/// The type a generic alias's declared type stands for.
#[derive(Debug)]
struct AliasBody(Type);

impl surge_ts_types::ResolveReference for AliasBody {
    fn resolve(&self) -> Type {
        self.0.clone()
    }
}

/// `SignatureKind`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SignatureKind {
    Call = 0,
    Construct = 1,
}

#[derive(Default)]
pub(crate) struct SemanticTables {
    pub(crate) types: Vec<TypeEntry>,
    type_by_key: FxHashMap<TypeKey, u32>,
    pub(crate) symbols: Vec<SymbolData>,
    pub(crate) symbol_by_key: FxHashMap<SymbolKey, u32>,
    signatures: Vec<SignatureEntry>,
    signature_by_key: FxHashMap<(TypeId, SignatureKind, u32), u32>,
    pub(crate) globals: Option<GlobalScope>,
    /// How tsc prints an error type that stands for a type reference
    /// nothing declares: the reference as written.
    error_displays: FxHashMap<u32, String>,
    /// Symbols whose declared type is being computed: one reached again is
    /// circular, tsc's error type.
    declared_in_progress: surge_ts_types::fx::FxHashSet<SymbolId>,
    creation_ranks: Option<Arc<surge_ts_types::CreationRanks>>,
    /// Each module's own export table: the bound tables it is read from never
    /// change, and every alias step reads it again.
    module_exports: FxHashMap<SymbolId, Arc<[SymbolId]>>,
}

pub(crate) struct TypeEntry {
    pub(crate) ty: Type,
    /// The symbol whose type this is, when a query started from one: its
    /// declarations are the signatures' declarations.
    origin: Option<SymbolId>,
    /// The node whose type this is, when a query started from one.
    location: Option<NodeId>,
    /// Where a type derived from another (a union's member, a type
    /// argument, a signature's return) was reached: the names written in it
    /// resolve there (a type parameter's declaration).
    scope: Option<NodeId>,
}

#[derive(Clone)]
struct SignatureEntry {
    function: FunctionType,
    declaration: Option<NodeId>,
    /// Where the type the signature belongs to was reached.
    scope: Option<NodeId>,
}

/// A location's type: a symbol's own type handle, or a type computed for
/// the location.
#[derive(Clone)]
enum Located {
    Handle(TypeId),
    Computed(Type),
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum TypeKey {
    Intrinsic(&'static str),
    Location(NodeId),
    /// An optional chain link's type without the `undefined` the chain adds.
    NonOptional(NodeId),
    SymbolType(SymbolId),
    DeclaredType(SymbolId),
    Apparent(TypeId),
    UnresolvedReference(String),
}

/// A query surge does not answer yet: the API says so rather than answering
/// with a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "surge-ts: {} is not supported yet", self.0)
    }
}

impl std::error::Error for Unsupported {}

/// `TypeChecker`. Cheap to clone; shares its program.
#[derive(Clone)]
pub struct TypeChecker {
    pub(crate) program: Program,
}

/// How a type names its constituents, for [`TypeChecker::type_constituents`].
fn intrinsic_name(ty: &Type) -> Option<&'static str> {
    Some(match ty {
        Type::Any => "any",
        Type::ErrorType | Type::Unknown => "error",
        Type::GenuineUnknown => "unknown",
        Type::String => "string",
        Type::Number => "number",
        Type::Boolean => "boolean",
        Type::BigInt => "bigint",
        Type::Symbol => "symbol",
        Type::Undefined => "undefined",
        Type::Null => "null",
        Type::Void => "void",
        Type::Never => "never",
        _ => return None,
    })
}

impl TypeChecker {
    pub(crate) fn new(program: Program) -> Self {
        TypeChecker { program }
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// The type handle with this index, if this checker handed one out.
    pub fn type_handle(&self, index: u32) -> Option<TypeId> {
        let count = self.with_tables(|tables| tables.types.len());
        ((index as usize) < count).then_some(TypeId { program: self.program.inner().identity, index })
    }

    /// The symbol handle with this index, if this checker handed one out.
    pub fn symbol_handle(&self, index: u32) -> Option<SymbolId> {
        let count = self.with_tables(|tables| tables.symbols.len());
        ((index as usize) < count).then_some(SymbolId { program: self.program.inner().identity, index })
    }

    /// The signature handle with this index, if this checker handed one out.
    pub fn signature_handle(&self, index: u32) -> Option<SignatureId> {
        let count = self.with_tables(|tables| tables.signatures.len());
        ((index as usize) < count).then_some(SignatureId { program: self.program.inner().identity, index })
    }

    /// The file an import of `specifier` in `file` resolved to in this
    /// program (`getResolvedModule`).
    pub fn resolved_module(&self, file: super::handles::SourceFileId, specifier: &str) -> Option<super::handles::SourceFileId> {
        self.resolved_module_file(file, specifier)
    }

    fn query<R>(&self, f: impl FnOnce(&mut Query<'_>) -> R) -> Option<R> {
        Some(self.program.inner().checked.as_ref()?.query(f))
    }

    fn type_id(&self, key: Option<TypeKey>, origin: Option<SymbolId>, ty: impl FnOnce() -> Type) -> TypeId {
        let program = self.program.inner().identity;
        if let Some(key) = &key
            && let Some(index) = self.with_tables(|tables| tables.type_by_key.get(key).copied())
        {
            return TypeId { program, index };
        }
        // A body-inferred return is the type it resolves to (`Generator<Y, R, N>`,
        // a class instance), not the lazy reference standing in for it, whose
        // peel would skip past that type's own reference and symbol.
        let ty = surge_ts_checker::lowlevel::semantic::settled_read(ty());
        // A read the checker marked as cycling back to a declaration is that
        // declaration's type, as tsc's same type object is.
        if let Some(referent) = surge_ts_types::cycle_marker_key(&ty).and_then(|marker| self.cycle_referent(marker)) {
            if let Some(key) = key {
                self.with_tables(|tables| tables.type_by_key.insert(key, referent.index));
            }
            return referent;
        }
        let location = match &key {
            Some(TypeKey::Location(node) | TypeKey::NonOptional(node)) => Some(*node),
            _ => None,
        };
        let key = key.or_else(|| intrinsic_name(&ty).map(TypeKey::Intrinsic));
        self.with_tables(|tables| {
            if let Some(key) = &key
                && let Some(&index) = tables.type_by_key.get(key)
            {
                return TypeId { program, index };
            }
            let index = tables.types.len() as u32;
            tables.types.push(TypeEntry { ty, origin, location, scope: None });
            if let Some(key) = key {
                tables.type_by_key.insert(key, index);
            }
            TypeId { program, index }
        })
    }

    /// The type of the declaration a cycle marker names: its name's symbol's,
    /// or an object literal's (the object its methods' `this` is).
    fn cycle_referent(&self, marker: &str) -> Option<TypeId> {
        thread_local! {
            static RESOLVING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        if RESOLVING.with(|resolving| resolving.borrow().iter().any(|key| key == marker)) {
            return None;
        }
        let mut parts = marker.split('\u{3}');
        let file = self.program.source_file(parts.next()?)?;
        let start = parts.next()?.parse::<usize>().ok()?;
        let end = parts.next()?.parse::<usize>().ok()?;
        let node = self.node_at_byte_span(self.program.root(file), start, end)?;
        RESOLVING.with(|resolving| resolving.borrow_mut().push(marker.to_string()));
        let referent = match self.program.node_kind(node) {
            SyntaxKind::Identifier => self.symbol_at_location(node).map(|symbol| self.type_of_symbol(symbol)),
            // `this` is the literal's regular type, not the fresh one its
            // expression has.
            _ => Some(self.intern(self.ty(self.type_at_location(node)))),
        };
        RESOLVING.with(|resolving| resolving.borrow_mut().retain(|key| key != marker));
        referent
    }

    /// The innermost node spanning exactly `start..end`.
    fn node_at_byte_span(&self, root: NodeId, start: usize, end: usize) -> Option<NodeId> {
        let program = &self.program;
        let mut node = root;
        'descend: loop {
            for (_, value) in program.node_properties(node) {
                let children = match value {
                    NodePropertyValue::Node(child) => vec![child],
                    NodePropertyValue::List { nodes, .. } => nodes,
                };
                for child in children {
                    let (child_start, child_end) = program.node_byte_span(child);
                    if child_start <= start && end <= child_end {
                        node = child;
                        continue 'descend;
                    }
                }
            }
            return (node != root && program.node_byte_span(node) == (start, end)).then_some(node);
        }
    }

    pub(crate) fn intern(&self, ty: Type) -> TypeId {
        self.type_id(None, None, || ty)
    }

    /// A type derived from `parent`, reached where `parent` was.
    fn intern_within(&self, parent: TypeId, ty: Type) -> TypeId {
        self.intern_in_scope(self.reached_at(parent), ty)
    }

    /// A contextual type's handle: reached at `node`, and the type of the
    /// annotated declaration `node` initializes, when it initializes one.
    pub(super) fn contextual_type_handle(&self, node: NodeId, ty: Type) -> TypeId {
        let declaration = self.program.node_parent(node).filter(|parent| {
            matches!(
                self.program.node_kind(*parent),
                SyntaxKind::VariableDeclaration | SyntaxKind::PropertyDeclaration | SyntaxKind::Parameter
            ) && self.node_child(*parent, "initializer") == Some(node)
                && self.node_child(*parent, "type").is_some()
        });
        match declaration.and_then(|declaration| self.declared_symbol(declaration)) {
            Some(symbol) => self.type_id(None, Some(symbol), || ty),
            None => self.intern_in_scope(Some(node), ty),
        }
    }

    pub(super) fn intern_in_scope(&self, scope: Option<NodeId>, ty: Type) -> TypeId {
        let handle = self.intern(ty);
        if scope.is_some() {
            self.with_tables(|tables| {
                let entry = &mut tables.types[handle.index as usize];
                if entry.location.is_none() && entry.origin.is_none() {
                    entry.scope = scope;
                }
            });
        }
        handle
    }

    pub(crate) fn ty(&self, ty: TypeId) -> Type {
        assert_eq!(ty.program, self.program.inner().identity, "type handle belongs to another program");
        self.with_tables(|tables| tables.types[ty.index as usize].ty.clone())
    }

    fn origin(&self, ty: TypeId) -> Option<SymbolId> {
        self.with_tables(|tables| tables.types[ty.index as usize].origin)
    }

    pub(super) fn type_origin(&self, ty: TypeId) -> Option<SymbolId> {
        self.origin(ty)
    }

    fn location(&self, ty: TypeId) -> Option<NodeId> {
        self.with_tables(|tables| tables.types[ty.index as usize].location)
    }

    pub(super) fn type_location(&self, ty: TypeId) -> Option<NodeId> {
        self.location(ty)
    }

    /// Where a type was reached, for resolving the names written in it.
    pub(super) fn type_scope(&self, ty: TypeId) -> Option<NodeId> {
        self.reached_at(ty)
    }

    /// tsc's `unresolvedType`: the declared type of a type reference nothing
    /// declares, an error type printed as `/*unresolved*/ any`.
    fn unresolved_type(&self) -> TypeId {
        self.type_id(Some(TypeKey::Intrinsic("unresolved")), None, || Type::ErrorType)
    }

    /// tsc's `errorType`: `any`, standing for a type there is no answer for.
    pub fn error_type(&self) -> TypeId {
        self.intern(Type::ErrorType)
    }

    pub fn any_type(&self) -> TypeId {
        self.intern(Type::Any)
    }
    pub fn unknown_type(&self) -> TypeId {
        self.intern(Type::GenuineUnknown)
    }
    pub fn string_type(&self) -> TypeId {
        self.intern(Type::String)
    }
    pub fn number_type(&self) -> TypeId {
        self.intern(Type::Number)
    }
    pub fn boolean_type(&self) -> TypeId {
        self.intern(Type::Boolean)
    }
    pub fn bigint_type(&self) -> TypeId {
        self.intern(Type::BigInt)
    }
    pub fn es_symbol_type(&self) -> TypeId {
        self.intern(Type::Symbol)
    }
    pub fn undefined_type(&self) -> TypeId {
        self.intern(Type::Undefined)
    }
    pub fn null_type(&self) -> TypeId {
        self.intern(Type::Null)
    }
    pub fn void_type(&self) -> TypeId {
        self.intern(Type::Void)
    }
    pub fn never_type(&self) -> TypeId {
        self.intern(Type::Never)
    }

    // --- locations -----------------------------------------------------------

    /// `getSymbolAtLocation`, with tsc's dispatch: a declaration's name is
    /// its symbol, a name or property access is what it resolves to, a module
    /// specifier is its module, `this` is its type's symbol.
    pub fn symbol_at_location(&self, node: NodeId) -> Option<SymbolId> {
        let program = &self.program;
        let kind = program.node_kind(node);
        if kind == SyntaxKind::SourceFile {
            return if program.is_external_module(node.source_file()) { self.module_symbol(node.source_file()) } else { None };
        }
        let parent = program.node_parent(node)?;
        let parent_kind = program.node_kind(parent);
        if program.is_declaration_name_or_import_property_name(node) {
            let declared = self.declared_symbol(parent)?;
            if matches!(parent_kind, SyntaxKind::ImportSpecifier | SyntaxKind::ExportSpecifier)
                && self.node_child(parent, "propertyName") == Some(node)
            {
                return self.alias_target(declared).filter(|target| *target != declared);
            }
            return Some(declared);
        }
        match kind {
            SyntaxKind::Identifier | SyntaxKind::PrivateIdentifier => self.symbol_of_name(node),
            SyntaxKind::PropertyAccessExpression => self.symbol_of_name(self.node_child(node, "name")?),
            SyntaxKind::QualifiedName => self.symbol_of_name(self.node_child(node, "right")?),
            SyntaxKind::ThisKeyword | SyntaxKind::SuperKeyword => {
                // An explicit `this` parameter is `this`'s symbol.
                if kind == SyntaxKind::ThisKeyword
                    && let Some(parameter) = self.explicit_this_parameter(node)
                {
                    return self.declared_symbol(parameter);
                }
                let ty = self.type_at_location(node);
                self.type_symbol(ty).or_else(|| {
                    if kind == SyntaxKind::ThisKeyword {
                        let receiver = self.this_receiver_type(node)?;
                        return self.type_symbol(receiver).or_else(|| self.origin(receiver));
                    }
                    self.super_class_symbol(node)
                })
            }
            // A `this` type is the class or interface it is written in.
            SyntaxKind::ThisType => {
                let mut current = program.node_parent(node);
                while let Some(owner) = current {
                    if matches!(
                        program.node_kind(owner),
                        SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression | SyntaxKind::InterfaceDeclaration
                    ) {
                        return self.declared_symbol(owner);
                    }
                    current = program.node_parent(owner);
                }
                None
            }
            // The right operand's `[Symbol.hasInstance]` method, else its
            // type's symbol (`getSymbolHasInstanceMethodOfObjectType`).
            SyntaxKind::InstanceOfKeyword if parent_kind == SyntaxKind::BinaryExpression => {
                let right = self.node_child(parent, "right")?;
                let ty = self.type_at_location(right);
                const HAS_INSTANCE: &str = "[Symbol.hasInstance]";
                let method = (self.type_flags(ty) & (tf::Object | tf::NonPrimitive | tf::Intersection) != 0)
                    .then(|| {
                        self.property_symbol_of_expression(right, HAS_INSTANCE).or_else(|| {
                            self.global_function_interface(ty)
                                .and_then(|interface| self.interface_member_in_hierarchy(interface, HAS_INSTANCE))
                        })
                    })
                    .flatten()
                    .filter(|method| {
                        let method_type = self.type_of_symbol(*method);
                        !self.signatures_of_type(method_type, SignatureKind::Call).is_empty()
                    });
                method.or_else(|| self.type_symbol(ty))
            }
            SyntaxKind::ConstructorKeyword if parent_kind == SyntaxKind::Constructor => {
                self.declared_symbol(program.node_parent(parent)?)
            }
            SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral => self
                .module_symbol_of_specifier(node)
                .or_else(|| self.element_access_property(node)),
            SyntaxKind::NumericLiteral => self.element_access_property(node),
            SyntaxKind::DefaultKeyword
            | SyntaxKind::FunctionKeyword
            | SyntaxKind::EqualsGreaterThanToken
            | SyntaxKind::ClassKeyword => self.declared_symbol(parent),
            SyntaxKind::ExportKeyword if parent_kind == SyntaxKind::ExportAssignment => self.declared_symbol(parent),
            SyntaxKind::ImportType => {
                let argument = self.node_child(node, "argument")?;
                self.symbol_at_location(self.node_child(argument, "literal")?)
            }
            _ => None,
        }
    }

    /// tsc's `getSymbolOfNameOrPropertyAccessExpression` for a name that is
    /// not a declaration's.
    fn symbol_of_name(&self, node: NodeId) -> Option<SymbolId> {
        let program = &self.program;
        let parent = program.node_parent(node)?;
        let parent_kind = program.node_kind(parent);
        let name = program.node_value_text(node)?.to_string();
        // A name the parser recovered resolves to nothing.
        if name.is_empty() {
            return None;
        }
        // A private name is the member the enclosing class that declares it
        // has (`lookupSymbolForPrivateIdentifierDeclaration`).
        if name.starts_with('#') && parent_kind == SyntaxKind::PropertyAccessExpression {
            return self.enclosing_private_member(node, &name);
        }
        if parent_kind == SyntaxKind::PropertyAccessExpression && self.node_child(parent, "name") == Some(node) {
            let object = self.node_child(parent, "expression")?;
            return self.property_symbol_of_expression(object, &name);
        }
        // An intrinsic element's tag is a member of `JSX.IntrinsicElements`
        // (`getIntrinsicTagSymbol`).
        if self.is_intrinsic_tag_name(node) {
            return self.intrinsic_tag_symbol(node, &name);
        }
        if parent_kind == SyntaxKind::QualifiedName && self.node_child(parent, "right") == Some(node) {
            let left = self.node_child(parent, "left")?;
            let left_symbol = self.symbol_at_location(left);
            let resolved = left_symbol.and_then(|namespace| self.export_of_symbol(namespace, &name));
            // `typeof value.member` reads a property of the value's type.
            let in_type_query = {
                let mut entity = parent;
                while let Some(outer) = program.node_parent(entity)
                    && program.node_kind(outer) == SyntaxKind::QualifiedName
                {
                    entity = outer;
                }
                program.node_parent(entity).is_some_and(|query| program.node_kind(query) == SyntaxKind::TypeQuery)
            };
            if resolved.is_none()
                && in_type_query
                && let Some(left_symbol) = left_symbol
                && self.symbol_flags(left_symbol) & (ts_sf::Variable | ts_sf::Property) != 0
            {
                return self.property_symbol_of_expression(left, &name);
            }
            return resolved.or_else(|| self.unresolved_type_reference_symbol(parent));
        }
        if parent_kind == SyntaxKind::BindingElement
            && self.node_child(parent, "propertyName") == Some(node)
            && let Some(pattern) = program.node_parent(parent)
            && program.node_kind(pattern) == SyntaxKind::ObjectBindingPattern
        {
            // An object literal the pattern destructures declares the member.
            if let Some(member) =
                self.pattern_source_literal(pattern).and_then(|literal| self.object_literal_member(literal, &name))
            {
                return Some(member);
            }
            let owner = self.type_at_location(pattern);
            return self.property_of_type(owner, &name);
        }
        // `export =` and `export default` name an entity of any meaning,
        // resolved at the nearest scope that declares one (`resolveEntityName`
        // with `Value | Type | Namespace`): a file's own namespace before a
        // global value of its name (its own UMD global among them).
        if parent_kind == SyntaxKind::ExportAssignment {
            let candidates: Vec<SymbolId> = [Meaning::Value, Meaning::Namespace, Meaning::Type]
                .into_iter()
                .filter_map(|meaning| self.resolve_name(node, &name, meaning))
                .collect();
            // A UMD global lives in the global scope even when the file
            // declaring it is this one.
            let file = node.source_file();
            let local = |candidate: &SymbolId| {
                self.symbol_declarations(*candidate).iter().any(|declaration| {
                    declaration.source_file() == file
                        && program.node_kind(*declaration) != SyntaxKind::NamespaceExportDeclaration
                })
            };
            return candidates.iter().copied().find(local).or_else(|| candidates.first().copied());
        }
        let meaning = meaning_of_location(program, node);
        let resolved = self
            .resolve_name(node, &name, meaning)
            .or_else(|| (meaning == Meaning::Namespace).then(|| self.resolve_name(node, &name, Meaning::Value)).flatten())
            // `resolveName` reads an alias by its target's meanings: an import
            // of a module names no type.
            .filter(|resolved| {
                meaning != Meaning::Type
                    || !(parent_kind == SyntaxKind::TypeReference && self.node_child(parent, "typeName") == Some(node))
                    || !self.is_alias(*resolved)
                    || self.aliased_symbol(*resolved).is_none_or(|target| {
                        !self.is_module_symbol(target) || self.symbol_flags(target) & ts_sf::Type != 0
                    })
            });
        if resolved.is_none() {
            let unresolved = self.unresolved_type_reference_symbol(node);
            if unresolved.is_some() {
                return unresolved;
            }
        }
        if resolved.is_none() && meaning == Meaning::Value {
            return self.builtin_value_symbol(node, &name);
        }
        resolved
    }

    /// tsc's `getUnresolvedSymbolForEntityName`, for a type reference's name
    /// nothing declares (the `const` of `as const` among them): a type alias
    /// placeholder per written path, `Foo.Bar` a child of `Foo`'s.
    fn unresolved_type_reference_symbol(&self, name: NodeId) -> Option<SymbolId> {
        let mut reference = name;
        while let Some(parent) = self.program.node_parent(reference)
            && self.program.node_kind(parent) == SyntaxKind::QualifiedName
        {
            reference = parent;
        }
        let parent = self.program.node_parent(reference)?;
        if self.program.node_kind(parent) != SyntaxKind::TypeReference {
            return None;
        }
        self.unresolved_symbol_with_path(name).map(|(symbol, _)| symbol)
    }

    fn unresolved_symbol_with_path(&self, name: NodeId) -> Option<(SymbolId, String)> {
        let program = &self.program;
        let (identifier, parent) = match program.node_kind(name) {
            SyntaxKind::QualifiedName => {
                let left = self.unresolved_symbol_with_path(self.node_child(name, "left")?)?;
                (self.node_child(name, "right")?, Some(left))
            }
            SyntaxKind::Identifier => (name, None),
            _ => return None,
        };
        let text = program.node_value_text(identifier)?.to_string();
        let path = match &parent {
            Some((_, parent_path)) => format!("{parent_path}.{text}"),
            None => text.clone(),
        };
        let ty = self.error_type();
        let symbol = self.symbol_id(SymbolKey::Global(Arc::from(format!("\0unresolved\0{path}"))), || SymbolData::Transient {
            name: Arc::from(text.as_str()),
            flags: ts_sf::TypeAlias,
            ty,
            parent: parent.map(|(symbol, _)| symbol),
            declarations: Vec::new(),
        });
        Some((symbol, path))
    }

    /// The property `obj["key"]` (or `obj[0]`) reads: tsc's answer for a
    /// literal argument of an element access.
    fn element_access_property(&self, literal: NodeId) -> Option<SymbolId> {
        let parent = self.program.node_parent(literal)?;
        // `T["k"]` in a type names the property of its object type.
        if self.program.node_kind(parent) == SyntaxKind::LiteralType
            && let Some(access) = self.program.node_parent(parent)
            && self.program.node_kind(access) == SyntaxKind::IndexedAccessType
            && self.node_child(access, "indexType") == Some(parent)
        {
            let object = self.node_child(access, "objectType")?;
            let name = self.program.node_value_text(literal)?.to_string();
            let property = self.property_of_type(self.type_at_location(object), &name);
            // An anonymous object type's member is declared where its type
            // is written.
            if property.is_none_or(|property| self.symbol_declarations(property).is_empty())
                && let Some(written) = self.written_member_of_type(object, &name, 0)
            {
                return Some(written);
            }
            return property;
        }
        if self.program.node_kind(parent) != SyntaxKind::ElementAccessExpression
            || self.node_child(parent, "argumentExpression") != Some(literal)
        {
            return None;
        }
        let object = self.node_child(parent, "expression")?;
        let name = self.program.node_value_text(literal)?.to_string();
        // `E["A"]` names the enum's member, as `E.A` does; `C["s"]` (and
        // `super["s"]` in a static member) a class's static member, its own
        // or a base class's.
        if let Some(container) = self.member_container_symbol(object) {
            if self.symbol_flags(container) & (ts_sf::Enum | ts_sf::ValueModule) != 0
                && let Some(member) = self.export_of_symbol(container, &name)
            {
                return Some(member);
            }
            let class = if self.is_alias(container) { self.aliased_symbol(container).unwrap_or(container) } else { container };
            if self.symbol_flags(class) & ts_sf::Class != 0
                && self.reads_class_value(object)
                && let Some(member) = self.static_member_in_hierarchy(class, &name, &mut Vec::new())
            {
                return Some(member);
            }
        }
        let object_type = self.type_at_location(object);
        let property = self.property_of_type(object_type, &name)?;
        // `getPropertyOfType` has no member an index signature answers; a
        // tuple's elements and an array's numeric names are surge's own.
        if !matches!(self.ty(object_type).peeled(), Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
            && matches!(self.symbol_data(property), SymbolData::Transient { .. })
            && self.symbol_declarations(property).is_empty()
            && self.index_signature_member(object_type, object, &name).is_some()
        {
            return None;
        }
        Some(property)
    }

    pub(super) fn type_reference_name(&self, reference: NodeId) -> Option<NodeId> {
        self.node_child(reference, "typeName")
    }

    pub(super) fn declaration_annotation(&self, declaration: NodeId) -> Option<NodeId> {
        self.node_child(declaration, "type")
    }

    pub(super) fn declaration_initializer(&self, declaration: NodeId) -> Option<NodeId> {
        self.node_child(declaration, "initializer")
    }

    pub(super) fn declaration_expression(&self, node: NodeId) -> Option<NodeId> {
        self.node_child(node, "expression")
    }

    pub(super) fn node_child(&self, node: NodeId, property: &str) -> Option<NodeId> {
        match self.program.node_property(node, property)? {
            super::node::NodePropertyValue::Node(child) => Some(child),
            _ => None,
        }
    }

    pub(super) fn declared_symbol(&self, declaration: NodeId) -> Option<SymbolId> {
        let symbol = self.program.declared_symbol_index(declaration)?;
        Some(self.bound_symbol(declaration.file, symbol))
    }

    /// `getTypeAtLocation`.
    pub fn type_at_location(&self, node: NodeId) -> TypeId {
        let key = Some(TypeKey::Location(node));
        if let Some(index) = self.with_tables(|tables| tables.type_by_key.get(key.as_ref().unwrap()).copied()) {
            return TypeId { program: node.program, index };
        }
        let located = match self.compute_type_at_location(node) {
            Located::Handle(ty) => Located::Handle(self.expanded_self_reference(node, ty).unwrap_or(ty)),
            Located::Computed(ty) => match self.expanded_self_reference_of(node, &ty) {
                Some(own) => Located::Handle(own),
                None => Located::Computed(ty),
            },
        };
        match located {
            // The very type of the symbol, as tsc answers with the same object.
            Located::Handle(ty) => {
                let ty = match self.ty(ty) {
                    Type::Any | Type::ErrorType | Type::Unknown if !self.keeps_error_type(node) => {
                        self.unresolved_expression_type(node).unwrap_or(ty)
                    }
                    _ => ty,
                };
                self.with_tables(|tables| tables.type_by_key.insert(key.unwrap(), ty.index));
                ty
            }
            Located::Computed(ty) => {
                if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown)
                    && !self.keeps_error_type(node)
                    && let Some(error) = self.unresolved_expression_type(node)
                {
                    self.with_tables(|tables| tables.type_by_key.insert(key.unwrap(), error.index));
                    return error;
                }
                let ty = match ty {
                    Type::Any | Type::ErrorType => self.no_lib_array_member_access(node).unwrap_or(ty),
                    ty => ty,
                };
                let origin = self.accessed_function_member(node, &ty);
                self.type_id(key, origin, || ty)
            }
        }
    }

    /// `getNonOptionalType` of an optional chain link's type: the type without
    /// the `undefined` the chain adds. A member read (`values?.map`) is then
    /// the member's own type, whose symbol is the member's, as in tsc.
    pub fn non_optional_type_at_location(&self, node: NodeId) -> TypeId {
        let ty = self.type_at_location(node);
        let Type::Union(union) = self.ty(ty) else { return ty };
        let members: Vec<Type> = union.types().iter().filter(|member| !matches!(member, Type::Undefined)).cloned().collect();
        if members.len() == union.types().len() {
            return ty;
        }
        let stripped = surge_ts_types::union_type(members);
        let origin = self.accessed_function_member(node, &stripped);
        self.type_id(Some(TypeKey::NonOptional(node)), origin, || stripped)
    }

    /// Whether `ty` is `module`'s own namespace object: an object of its
    /// exports alone (a synthetic `default` makes another object).
    fn is_namespace_object_of(&self, module: SymbolId, ty: &Type) -> bool {
        let Type::Object(object) = ty else { return false };
        let exports: std::collections::HashSet<String> =
            self.exports_of_module(module).into_iter().map(|export| self.symbol_name(export)).collect();
        object.properties.keys().all(|name| exports.contains(name.as_ref()))
    }

    /// The checker's one-pass-earlier signature of a function `f` that a
    /// signature reads (named `typeof f` for where tsc's printer meets `f`
    /// again) is, printed by itself, `f`'s own type.
    fn expanded_self_reference(&self, location: NodeId, ty: TypeId) -> Option<TypeId> {
        self.expanded_self_reference_of(location, &self.ty(ty))
    }

    fn expanded_self_reference_of(&self, location: NodeId, ty: &Type) -> Option<TypeId> {
        let Type::Function(function) = ty else { return None };
        let name = function.alias_name()?.strip_prefix("typeof ")?;
        if name.contains(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')) {
            return None;
        }
        let symbol = self.resolve_name(location, name, Meaning::Value)?;
        if self.symbol_flags(symbol) & ts_sf::Function == 0 {
            return None;
        }
        let own = self.type_of_symbol(symbol);
        matches!(self.ty(own), Type::Function(own) if own.alias_name().is_none()).then_some(own)
    }

    /// A property access on an array that a `noLib` program's own `Array`
    /// answers, where surge's array model has no such member.
    fn no_lib_array_member_access(&self, node: NodeId) -> Option<Type> {
        if self.program.node_kind(node) != SyntaxKind::PropertyAccessExpression {
            return None;
        }
        let receiver = self.ty(self.type_at_location(self.node_child(node, "expression")?));
        let name = self.program.node_text(self.node_child(node, "name")?).to_string();
        let checker_index = self.program.file(node.source_file()).checker_index?;
        self.query(|query| query.no_lib_array_member(checker_index, &receiver, &name)).flatten()
    }

    /// The method or function a property access reads, when its type is that
    /// member's signature: tsc's type of `values.pop` is the method's type,
    /// whose symbol is the method (tools key members by it).
    fn accessed_function_member(&self, node: NodeId, ty: &Type) -> Option<SymbolId> {
        // The member's name: a property access's, or a literal element
        // access's argument (`game["FindFirstChild"]`).
        let name = match self.program.node_kind(node) {
            SyntaxKind::PropertyAccessExpression => self.node_child(node, "name")?,
            SyntaxKind::ElementAccessExpression => self.node_child(node, "argumentExpression")?,
            _ => return None,
        };
        let is_function = match ty {
            Type::Function(_) => true,
            Type::Object(object) => object.call_signature().is_some() && object.properties.is_empty(),
            _ => false,
        };
        if !is_function {
            return None;
        }
        // An optional element access's key names no symbol (its receiver's
        // type has `undefined`); the member is the non-nullable receiver's.
        let member = self.symbol_at_location(name).or_else(|| {
            if self.program.node_kind(node) != SyntaxKind::ElementAccessExpression {
                return None;
            }
            let key = self.program.node_value_text(name)?.to_string();
            let receiver = self.non_nullable_type(self.type_at_location(self.node_child(node, "expression")?));
            self.property_of_type(receiver, &key)
        })?;
        // An instantiation's member (`Array<number>`'s `pop`) is a transient
        // copy; the type's symbol is the declaration's own.
        let declared = self
            .symbol_declarations(member)
            .first()
            .and_then(|declaration| self.declared_symbol(*declaration))
            .unwrap_or(member);
        // A property naming a value (`{ callback }`) has that value's type,
        // whose symbol and declarations are the value's.
        let declared = self.property_named_value(declared).unwrap_or(declared);
        // A method or function; a property a function expression initializes
        // (`no: function () {}`), whose type is the expression's; or one a
        // function type annotates (`throw: (search?: string) => void`), whose
        // type is the type literal's.
        (self.symbol_flags(declared) & (ts_sf::Method | ts_sf::Function) != 0
            || self.function_initializer(declared).is_some()
            || self.function_type_annotation(declared).is_some())
        .then_some(declared)
    }

    /// tsc's `getTypeOfNode`: a type node's type, an expression's type as
    /// the checker computed it, a declaration's or declaration name's
    /// symbol's type, a type declaration's declared type.
    fn compute_type_at_location(&self, node: NodeId) -> Located {
        let program = &self.program;
        let kind = program.node_kind(node);
        let error = Located::Computed(Type::ErrorType);
        if kind == SyntaxKind::SourceFile && !program.is_external_module(node.source_file()) {
            return error;
        }
        // tsc's `getTypeOfNode` for a class's `extends Base<A>`: the base's
        // instance type, which the clause's text names as a type.
        if kind == SyntaxKind::ExpressionWithTypeArguments && self.is_class_extends_heritage(node) {
            return Located::Computed(self.type_of_type_node(node).unwrap_or(Type::ErrorType));
        }
        if program.is_part_of_type_node(node) {
            if kind == SyntaxKind::TypeReference
                && let Some(ty) = self.unresolved_reference_type(node)
            {
                return Located::Handle(ty);
            }
            if kind == SyntaxKind::TypeQuery
                && let Some(ty) = self.queried_entity_type(node)
            {
                return Located::Handle(ty);
            }
            // `typeof x`'s name is the value's type.
            if kind == SyntaxKind::Identifier
                && program.node_parent(node).is_some_and(|parent| {
                    program.node_kind(parent) == SyntaxKind::TypeQuery && self.node_child(parent, "exprName") == Some(node)
                })
                && let Some(ty) = self.queried_value_type(node)
            {
                return Located::Handle(ty);
            }
            if matches!(kind, SyntaxKind::Identifier | SyntaxKind::QualifiedName | SyntaxKind::PropertyAccessExpression) {
                return self.symbol_at_location(node).map_or(error, |symbol| Located::Handle(self.declared_type_at(symbol, Some(node))));
            }
            return Located::Computed(self.type_of_type_node(node).unwrap_or(Type::ErrorType));
        }
        // A literal naming a declaration (`{ "key": v }`) is no expression
        // there (`isInExpressionContext`): it has its symbol's type.
        if matches!(kind, SyntaxKind::StringLiteral | SyntaxKind::NumericLiteral | SyntaxKind::NoSubstitutionTemplateLiteral)
            && program.is_declaration_name_or_import_property_name(node)
            && program.node_parent(node).is_some_and(|parent| {
                !matches!(program.node_kind(parent), SyntaxKind::ImportSpecifier | SyntaxKind::ExportSpecifier)
            })
            && let Some(symbol) = self.symbol_at_location(node)
        {
            return Located::Handle(self.type_of_symbol(symbol));
        }
        if program.is_expression_node(node) || self.is_import_equals_entity_name(node) {
            return self.type_of_expression(node, kind);
        }
        if program.is_type_declaration(node) {
            return self.declared_symbol(node).map_or(error, |symbol| Located::Handle(self.declared_type_of_symbol(symbol)));
        }
        if program.is_type_declaration_name(node) {
            return self.symbol_at_location(node).map_or(error, |symbol| Located::Handle(self.declared_type_of_symbol(symbol)));
        }
        if super::classify::is_declaration_kind(kind) {
            return self.declared_symbol(node).map_or(error, |symbol| Located::Handle(self.type_of_symbol(symbol)));
        }
        if program.is_declaration_name_or_import_property_name(node) {
            let Some(symbol) = self.symbol_at_location(node) else { return error };
            return Located::Handle(self.type_of_symbol(symbol));
        }
        if matches!(kind, SyntaxKind::ObjectBindingPattern | SyntaxKind::ArrayBindingPattern) {
            return Located::Computed(self.binding_pattern_source_type(node).unwrap_or(Type::ErrorType));
        }
        // `isInRightSideOfImportOrExportAssignment`: `export = C` names the
        // declared type, else the value.
        if self.is_export_assignment_entity_name(node)
            && let Some(symbol) = self.symbol_at_location(node)
        {
            let declared = self.declared_type_of_symbol(symbol);
            if !matches!(self.ty(declared), Type::ErrorType) {
                return Located::Handle(declared);
            }
            return Located::Handle(self.type_of_symbol(symbol));
        }
        error
    }

    fn is_export_assignment_entity_name(&self, node: NodeId) -> bool {
        let program = &self.program;
        if !matches!(program.node_kind(node), SyntaxKind::Identifier | SyntaxKind::QualifiedName) {
            return false;
        }
        let mut current = node;
        while let Some(parent) = program.node_parent(current) {
            match program.node_kind(parent) {
                SyntaxKind::QualifiedName => current = parent,
                SyntaxKind::ExportAssignment => return self.node_child(parent, "expression") == Some(current),
                _ => return false,
            }
        }
        false
    }

    /// A name in `import x = A.B`'s entity, which tsc types as the value it
    /// names.
    fn is_import_equals_entity_name(&self, node: NodeId) -> bool {
        let program = &self.program;
        if !matches!(program.node_kind(node), SyntaxKind::Identifier | SyntaxKind::QualifiedName) {
            return false;
        }
        let mut current = node;
        while let Some(parent) = program.node_parent(current) {
            match program.node_kind(parent) {
                SyntaxKind::QualifiedName => current = parent,
                SyntaxKind::ImportEqualsDeclaration => return self.node_child(parent, "moduleReference") == Some(current),
                _ => return false,
            }
        }
        false
    }

    fn located_type(&self, located: Located) -> Type {
        match located {
            Located::Handle(ty) => self.ty(ty),
            Located::Computed(ty) => ty,
        }
    }

    /// The type a binding pattern destructures: its declaration's annotation,
    /// else its initializer's type (tsc's `getTypeForVariableLikeDeclaration`).
    fn binding_pattern_source_type(&self, pattern: NodeId) -> Option<Type> {
        let declaration = self.program.node_parent(pattern)?;
        if let Some(annotation) = self.node_child(declaration, "type") {
            return self.type_of_type_node(annotation);
        }
        if let Some(initializer) = self.node_child(declaration, "initializer") {
            let kind = self.program.node_kind(initializer);
            return Some(self.located_type(self.type_of_expression(initializer, kind)));
        }
        match self.program.node_kind(declaration) {
            SyntaxKind::BindingElement => {
                let outer = self.program.node_parent(declaration)?;
                let source = self.binding_pattern_source_type(outer)?;
                self.binding_element_of(declaration, outer, &source)
            }
            _ => None,
        }
    }

    /// What a binding element of `pattern` reads off the pattern's `source`:
    /// an array pattern's element by position, a rest element what follows,
    /// an object pattern's by property.
    fn binding_element_of(&self, element: NodeId, pattern: NodeId, source: &Type) -> Option<Type> {
        if self.program.node_kind(pattern) == SyntaxKind::ArrayBindingPattern {
            let Some(NodePropertyValue::List { nodes, .. }) = self.program.node_property(pattern, "elements") else {
                return None;
            };
            let index = nodes.iter().position(|candidate| *candidate == element)?;
            if self.node_child(element, "dotDotDotToken").is_some() {
                return match source.peeled() {
                    Type::Tuple(elements) => Some(Type::Tuple(elements.iter().skip(index).cloned().collect())),
                    array @ Type::Array(_) => Some(array),
                    _ => None,
                };
            }
            return self.query(|query| query.property(source, &index.to_string())).flatten().map(|property| property.ty);
        }
        // An object rest has what the elements beside it do not take.
        if self.node_child(element, "dotDotDotToken").is_some() {
            let Type::Object(object) = source.peeled() else { return None };
            let Some(NodePropertyValue::List { nodes, .. }) = self.program.node_property(pattern, "elements") else {
                return None;
            };
            let taken: Vec<String> = nodes
                .iter()
                .filter(|sibling| **sibling != element)
                .filter_map(|sibling| {
                    let name = self.node_child(*sibling, "propertyName").or_else(|| self.node_child(*sibling, "name"))?;
                    self.program.node_value_text(name).map(str::to_string)
                })
                .collect();
            let mut properties = (*object.properties).clone();
            properties.retain(|name, _| !taken.iter().any(|taken| **taken == **name));
            return Some(Type::Object(surge_ts_types::ObjectType::new(properties, object.string_index_type.as_deref().cloned())));
        }
        let name = self
            .node_child(element, "propertyName")
            .or_else(|| self.node_child(element, "name"))
            .and_then(|name| self.program.node_value_text(name).map(str::to_string))?;
        self.query(|query| query.property(source, &name)).flatten().map(|property| property.ty)
    }

    /// tsc's `getTypeForBindingElement` where the checker recorded nothing (a
    /// signature without a body): what the pattern's source has there, else
    /// the element's initializer widened, else what a rest element implies.
    fn binding_element_type(&self, element: NodeId) -> Option<Type> {
        let pattern = self.program.node_parent(element)?;
        if let Some(source) = self.binding_pattern_source_type(pattern)
            && let Some(ty) = self.binding_element_of(element, pattern, &source)
        {
            return Some(ty);
        }
        if let Some(initializer) = self.node_child(element, "initializer") {
            let ty = self.located_type(self.type_of_expression(initializer, self.program.node_kind(initializer)));
            return self.query(|query| query.widened_literal(&ty));
        }
        if self.node_child(element, "dotDotDotToken").is_some() {
            return Some(match self.program.node_kind(pattern) {
                SyntaxKind::ArrayBindingPattern => Type::Array(Box::new(Type::Any)),
                _ => Type::Object(surge_ts_types::ObjectType::new(Default::default(), Some(Type::Any))),
            });
        }
        None
    }

    /// tsc's `getRegularTypeOfExpression`, answered from what the checker
    /// recorded for the expression.
    fn type_of_expression(&self, node: NodeId, kind: SyntaxKind) -> Located {
        let located = self.type_of_expression_unbound(node, kind);
        if kind == SyntaxKind::CallExpression {
            return match self.optional_call_type(node, self.located_type(located.clone())) {
                Some(ty) => Located::Computed(ty),
                None => located,
            };
        }
        if kind != SyntaxKind::PropertyAccessExpression {
            return located;
        }
        match self.bind_this_return(node, self.located_type(located.clone())) {
            Some(bound) => Located::Computed(bound),
            None => located,
        }
    }

    /// `getTypeOfSymbolAtLocation` of a link of an optional chain: the link's
    /// type, the outermost one's with the `undefined` the chain adds, an
    /// inner one's without the marker (`removeOptionalTypeMarker`).
    fn optional_chain_link_type(&self, access: NodeId, symbol: SymbolId) -> Option<Type> {
        let program = &self.program;
        if program.node_kind(access) != SyntaxKind::PropertyAccessExpression
            || program.node_flags(access) & super::enums_generated::node_flags::OptionalChain == 0
        {
            return None;
        }
        let ty = self.ty(self.type_at_location(access));
        if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown) {
            return None;
        }
        let continues = program.node_parent(access).is_some_and(|parent| {
            program.node_flags(parent) & super::enums_generated::node_flags::OptionalChain != 0
                && self.node_child(parent, "expression") == Some(access)
        });
        if !continues {
            return Some(ty);
        }
        let declared_undefined = match self.ty(self.type_of_symbol(symbol)).peeled() {
            Type::Undefined => true,
            Type::Union(union) => union.types().contains(&Type::Undefined),
            _ => false,
        };
        match ty.peeled() {
            Type::Union(union) if !declared_undefined && union.types().contains(&Type::Undefined) => Some(
                surge_ts_types::union_type(union.types().iter().filter(|member| !matches!(member, Type::Undefined)).cloned().collect()),
            ),
            _ => Some(ty),
        }
    }

    /// tsc's `unique symbol`: `Symbol()` (or `Symbol.for(...)`) initializing
    /// a `const` or a `static readonly` property (`getESSymbolLikeTypeForNode`),
    /// and a member declared `: unique symbol` (`Symbol.iterator`).
    fn unique_symbol_type_at(&self, node: NodeId, kind: SyntaxKind) -> Option<Type> {
        let program = &self.program;
        let unique = |declaration: NodeId| {
            let file = program.file_name(declaration.source_file()).to_string();
            let (start, _) = program.node_byte_span(declaration);
            surge_ts_types::unique_symbol_type(&file, &format!("symbol\u{0}{start}\u{0}member"))
        };
        match kind {
            SyntaxKind::CallExpression => {
                let callee = self.node_child(node, "expression")?;
                let is_symbol = |name: NodeId| program.node_value_text(name) == Some("Symbol");
                let names_symbol = match program.node_kind(callee) {
                    SyntaxKind::Identifier => is_symbol(callee),
                    SyntaxKind::PropertyAccessExpression => {
                        self.node_child(callee, "expression").is_some_and(is_symbol)
                            && self.node_child(callee, "name").and_then(|name| program.node_value_text(name)) == Some("for")
                    }
                    _ => false,
                };
                if !names_symbol {
                    return None;
                }
                let mut parent = program.node_parent(node)?;
                while program.node_kind(parent) == SyntaxKind::ParenthesizedExpression {
                    parent = program.node_parent(parent)?;
                }
                let valid = match program.node_kind(parent) {
                    SyntaxKind::VariableDeclaration => {
                        self.node_child(parent, "name").is_some_and(|name| program.node_kind(name) == SyntaxKind::Identifier)
                            && program.node_parent(parent).is_some_and(|list| {
                                program.node_flags(list) & super::enums_generated::node_flags::Const != 0
                            })
                    }
                    SyntaxKind::PropertyDeclaration => {
                        self.has_readonly_modifier(parent)
                            && matches!(program.node_property(parent, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
                                if nodes.iter().any(|modifier| program.node_kind(*modifier) == SyntaxKind::StaticKeyword))
                    }
                    _ => false,
                };
                valid.then(|| unique(parent))
            }
            SyntaxKind::PropertyAccessExpression => {
                let symbol = self.symbol_at_location(self.node_child(node, "name")?)?;
                self.declared_unique_symbol(symbol)
            }
            _ => None,
        }
    }

    /// The names a member may have in the checker's type of its owner: a
    /// computed name's literal key, then its written key (which surge keeps
    /// where the key is not a literal type), a private name as written, or
    /// the bound name.
    fn member_names(&self, declaration: NodeId, bound_name: &str) -> Vec<String> {
        let program = &self.program;
        let mut names = Vec::new();
        if let Some(name) = self.node_child(declaration, "name")
            && program.node_kind(name) == SyntaxKind::ComputedPropertyName
        {
            names.extend(self.computed_member_name(declaration));
            if let Some(key) = self.node_child(name, "expression") {
                let written: String = program.node_text(key).split_whitespace().collect();
                names.push(format!("[{written}]"));
                names.push(written);
            }
        }
        names.push(private_member_name(bound_name).unwrap_or(bound_name).to_string());
        names
    }

    /// The property name a member with a computed name (`[key]`) has: the
    /// key's string or number literal type (`getPropertyNameFromType`).
    fn computed_member_name(&self, declaration: NodeId) -> Option<String> {
        let program = &self.program;
        let name = self.node_child(declaration, "name")?;
        if program.node_kind(name) != SyntaxKind::ComputedPropertyName {
            return None;
        }
        let key = self.node_child(name, "expression")?;
        match self.located_type(self.type_of_expression(key, program.node_kind(key))).peeled() {
            Type::StringLiteral(value) => Some(value),
            Type::NumberLiteral(value) => Some(value.value.to_string()),
            _ => None,
        }
    }

    /// Whether a symbol is declared inside `declare module "name" { … }`.
    fn is_in_ambient_module_block(&self, symbol: SymbolId) -> bool {
        let program = &self.program;
        self.symbol_declarations(symbol).first().is_some_and(|declaration| {
            let mut current = program.node_parent(*declaration);
            while let Some(node) = current {
                if program.node_kind(node) == SyntaxKind::ModuleDeclaration
                    && self.node_child(node, "name").is_some_and(|name| program.node_kind(name) == SyntaxKind::StringLiteral)
                {
                    return true;
                }
                current = program.node_parent(node);
            }
            false
        })
    }

    /// The `unique symbol` a member declared `: unique symbol` has.
    fn declared_unique_symbol(&self, symbol: SymbolId) -> Option<Type> {
        let program = &self.program;
        let declaration = *self.symbol_declarations(symbol).first()?;
        // `isValidESSymbolDeclaration`: a readonly property, static in a class.
        let is_static = || {
            matches!(program.node_property(declaration, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
                if nodes.iter().any(|modifier| program.node_kind(*modifier) == SyntaxKind::StaticKeyword))
        };
        let valid = match program.node_kind(declaration) {
            SyntaxKind::PropertySignature => self.has_readonly_modifier(declaration),
            SyntaxKind::PropertyDeclaration => self.has_readonly_modifier(declaration) && is_static(),
            _ => false,
        };
        if !valid {
            return None;
        }
        let annotation = self.node_child(declaration, "type")?;
        if program.node_kind(annotation) != SyntaxKind::TypeOperator
            || program.node_operator(annotation) != Some(SyntaxKind::UniqueKeyword)
        {
            return None;
        }
        let file = program.file_name(declaration.source_file()).to_string();
        let (start, _) = program.node_byte_span(declaration);
        Some(surge_ts_types::unique_symbol_type(&file, &format!("symbol\u{0}{start}\u{0}member")))
    }

    /// `checkCallExpression` of a call in an optional chain whose callee may
    /// be nullish: its result is possibly `undefined`.
    fn optional_call_type(&self, call: NodeId, ty: Type) -> Option<Type> {
        let program = &self.program;
        if program.node_flags(call) & super::enums_generated::node_flags::OptionalChain == 0
            || matches!(ty, Type::Any | Type::ErrorType | Type::Unknown | Type::Undefined)
        {
            return None;
        }
        let has_undefined = |ty: &Type| match ty.peeled() {
            Type::Undefined | Type::Null => true,
            Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Undefined | Type::Null)),
            _ => false,
        };
        if has_undefined(&ty) {
            return None;
        }
        let callee = self.node_child(call, "expression")?;
        let callee_type = self.located_type(self.type_of_expression(callee, program.node_kind(callee)));
        has_undefined(&callee_type).then(|| surge_ts_types::union_type(vec![ty, Type::Undefined]))
    }

    /// A method read off its object whose return is the polymorphic `this`
    /// returns that object's type (tsc instantiates `this` with the receiver).
    fn bind_this_return(&self, access: NodeId, ty: Type) -> Option<Type> {
        let Type::Function(function) = &ty else { return None };
        let mut overloads = vec![function.clone()];
        overloads.extend(function.overloads().into_iter().flatten().cloned());
        if !overloads.iter().any(FunctionType::returns_polymorphic_this) {
            return None;
        }
        let object = self.node_child(access, "expression")?;
        let receiver = self.located_type(self.type_of_expression(object, self.program.node_kind(object)));
        if matches!(receiver, Type::Any | Type::ErrorType | Type::Unknown) {
            return None;
        }
        Some(surge_ts_types::bind_polymorphic_this_return(ty, &receiver))
    }

    fn type_of_expression_unbound(&self, node: NodeId, kind: SyntaxKind) -> Located {
        let program = &self.program;
        // The name of an access is typed as the access (tsc's
        // `isRightSideOfQualifiedNameOrPropertyAccess`).
        if let Some(parent) = program.node_parent(node)
            && matches!(
                (program.node_kind(parent), self.node_child(parent, "name").or_else(|| self.node_child(parent, "right"))),
                (SyntaxKind::PropertyAccessExpression | SyntaxKind::QualifiedName | SyntaxKind::MetaProperty, Some(name)) if name == node
            )
        {
            return self.type_of_expression(parent, program.node_kind(parent));
        }
        if kind == SyntaxKind::ParenthesizedExpression
            && let Some(inner) = self.node_child(node, "expression")
        {
            return self.type_of_expression(inner, program.node_kind(inner));
        }
        // `checkNonNullAssertion`: the operand's type without `null` and
        // `undefined`.
        if kind == SyntaxKind::NonNullExpression
            && let Some(inner) = self.node_child(node, "expression")
        {
            let operand = self.located_type(self.type_of_expression(inner, program.node_kind(inner)));
            if matches!(operand, Type::Null | Type::Undefined | Type::Void) {
                return Located::Computed(Type::Never);
            }
            return Located::Computed(surge_ts_types::remove_nullish(&operand));
        }
        // `checkIdentifier` and `checkPropertyAccessExpression` report a write
        // to what cannot be written and type the target as the error type.
        if matches!(kind, SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression | SyntaxKind::ElementAccessExpression)
            && self.is_assignment_target(node)
            && self.is_invalid_write_target(node, kind)
        {
            return Located::Computed(Type::ErrorType);
        }
        // An intrinsic element's tag is no expression: `getTypeOfNode` has
        // the error type there.
        if kind == SyntaxKind::Identifier && self.is_intrinsic_tag_name(node) {
            return Located::Computed(Type::ErrorType);
        }
        // `checkIdentifier`: the target of a definite assignment is the
        // variable's declared type, not the type flow narrowed it to.
        if kind == SyntaxKind::Identifier
            && self.is_definite_assignment_target(node)
            && let Some(symbol) = self.symbol_at_location(node)
            && self.symbol_flags(symbol) & ts_sf::Variable != 0
        {
            return Located::Handle(self.type_of_symbol(symbol));
        }
        // A reference to a module alias is the module's namespace object,
        // named by its module (`typeof import("m")`), as its alias is.
        if kind == SyntaxKind::Identifier
            && let Some(symbol) = self.symbol_at_location(node)
            && self.is_alias(symbol)
            && let Some(target) = self.aliased_symbol(symbol).filter(|target| *target != symbol && self.is_module_symbol(*target))
            && self.recorded_type(node, kind).is_some_and(|recorded| self.is_namespace_object_of(target, &recorded))
        {
            let ty = self.type_of_symbol(symbol);
            if self.origin(ty) == Some(target) {
                return Located::Handle(ty);
            }
        }
        if kind == SyntaxKind::NewExpression && self.instantiates_abstract_class(node) {
            return Located::Computed(Type::ErrorType);
        }
        if let Some(literal) = literal_type(program, node, kind) {
            return Located::Computed(literal);
        }
        if let Some(located) = self.commonjs_type(node, kind) {
            return located;
        }
        if kind == SyntaxKind::PropertyAccessExpression
            && let Some(symbol) = self.symbol_at_location(node)
            && self.symbol_flags(symbol) & ts_sf::EnumMember != 0
            && let Some(&member) = self.symbol_declarations(symbol).first()
            && let Some(ty) = self.local_enum_member_type(member, true)
        {
            return Located::Computed(ty);
        }
        // A member an enum's own initializer reads by name is its enum
        // literal, which the checker folds to the value.
        if kind == SyntaxKind::Identifier
            && let Some(symbol) = self.symbol_at_location(node)
            && self.symbol_flags(symbol) & ts_sf::EnumMember != 0
            && !self.is_declaration_name(node)
        {
            let member = self.type_of_symbol(symbol);
            if matches!(self.ty(member), Type::Reference(_)) {
                return Located::Handle(member);
            }
        }
        if let Some(own) = self.own_value_type(node, kind) {
            return Located::Handle(own);
        }
        if let Some(unique) = self.unique_symbol_type_at(node, kind) {
            return Located::Computed(unique);
        }
        // An array's method is the lib `Array<T>`'s; the checker reads it off
        // its own approximation of the array type.
        if kind == SyntaxKind::PropertyAccessExpression
            && let Some(ty) = self.array_method_type(node)
        {
            return Located::Computed(ty);
        }
        // A plain assignment's target is its declared property, whatever the
        // checker recorded reading it as.
        if kind == SyntaxKind::PropertyAccessExpression
            && program.node_parent(node).is_some_and(|parent| {
                program.node_kind(parent) == SyntaxKind::BinaryExpression
                    && self.node_child(parent, "left") == Some(node)
                    && self
                        .node_child(parent, "operatorToken")
                        .is_some_and(|operator| program.node_kind(operator) == SyntaxKind::EqualsToken)
            })
            && let Some(ty) = self.unnarrowed_access_type(node)
        {
            return Located::Computed(ty);
        }
        // `a = () => …` is the function's own type (`checkBinaryLikeExpression`
        // answers the right operand's), which the checker records as `a`'s.
        if kind == SyntaxKind::BinaryExpression
            && let Some(right) = self.node_child(node, "right")
            && matches!(program.node_kind(right), SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression)
            && let Some(operator) = self.node_child(node, "operatorToken").map(|operator| program.node_kind(operator))
        {
            if operator == SyntaxKind::EqualsToken {
                return self.type_of_expression(right, program.node_kind(right));
            }
            // A logical assignment is the part of the target it keeps and the
            // assigned value: `a &&= f` is `a`'s falsy part or `f`, `a ||= f`
            // its truthy part or `f`, `a ??= f` its non-nullish part or `f`.
            if matches!(
                operator,
                SyntaxKind::AmpersandAmpersandEqualsToken | SyntaxKind::BarBarEqualsToken | SyntaxKind::QuestionQuestionEqualsToken
            ) && let Some(left) = self.node_child(node, "left")
                && let Some(symbol) = self.symbol_at_location(left)
            {
                let declared = self.ty(self.type_of_symbol(symbol));
                let members = match declared.peeled() {
                    Type::Union(union) => union.types().to_vec(),
                    other => vec![other],
                };
                let falsy = |member: &Type| match member {
                    Type::Undefined | Type::Null | Type::Void | Type::BooleanLiteral(false) => true,
                    Type::NumberLiteral(number) => number.value == "0",
                    Type::StringLiteral(text) => text.is_empty(),
                    _ => false,
                };
                let kept: Vec<Type> = members
                    .into_iter()
                    .filter(|member| match operator {
                        SyntaxKind::AmpersandAmpersandEqualsToken => falsy(member),
                        SyntaxKind::BarBarEqualsToken => !falsy(member),
                        _ => !matches!(member, Type::Undefined | Type::Null | Type::Void),
                    })
                    .collect();
                let assigned = self.located_type(self.type_of_expression(right, program.node_kind(right)));
                let mut union = kept;
                union.push(assigned);
                return Located::Computed(surge_ts_types::union_type(union));
            }
        }
        let recorded = self.recorded_type(node, kind);
        // A literal the checker left unrecorded (or recorded only as the
        // `any` of a failed assignment) is typed as `checkObjectLiteral` and
        // `checkArrayLiteral` type it.
        if matches!(kind, SyntaxKind::ObjectLiteralExpression | SyntaxKind::ArrayLiteralExpression)
            && recorded.as_ref().is_none_or(|ty| matches!(ty, Type::Any | Type::ErrorType | Type::Unknown) || ty.is_degraded())
            && let Some(ty) = self.unrecorded_literal_type(node, kind)
        {
            return Located::Computed(ty);
        }
        if kind == SyntaxKind::ObjectLiteralExpression
            && let Some(recorded) = &recorded
            && let Some(ty) = self.with_recovered_members(node, recorded)
        {
            return Located::Computed(ty);
        }
        // A name or member the checker typed with its stand-in signature has
        // its declaration's type; a call through one, that signature's return.
        let stand_in = recorded.as_ref().is_some_and(is_stand_in_signature)
            && matches!(kind, SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression);
        let through_stand_in = kind == SyntaxKind::CallExpression
            && matches!(recorded, Some(Type::Any))
            && self.node_child(node, "expression").is_some_and(|callee| {
                let member = (program.node_kind(callee) == SyntaxKind::PropertyAccessExpression)
                    .then(|| {
                        let object = self.node_child(callee, "expression")?;
                        let name = program.node_value_text(self.node_child(callee, "name")?)?.to_string();
                        let object_type = self.located_type(self.type_of_expression(object, program.node_kind(object)));
                        self.query(|query| query.property(&object_type, &name)).flatten().map(|property| property.ty)
                    })
                    .flatten();
                member
                    .or_else(|| self.recorded_type(callee, program.node_kind(callee)))
                    .as_ref()
                    .is_some_and(is_stand_in_signature)
            });
        if through_stand_in && let Some(ty) = self.unrecorded_call_type(node) {
            return Located::Computed(ty);
        }
        // An import the checker read as `any` has the type its alias has
        // (what the importing file bound, where its target is out of reach).
        if matches!(recorded, Some(Type::Any))
            && kind == SyntaxKind::Identifier
            && let Some(symbol) = self.symbol_at_location(node)
            && self.is_alias(symbol)
        {
            let aliased = self.type_of_symbol(symbol);
            if !matches!(self.ty(aliased), Type::Any | Type::ErrorType | Type::Unknown) {
                return Located::Handle(aliased);
            }
        }
        if let Some(ty) = recorded.filter(|_| !stand_in) {
            if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown)
                && let Some(recovered) = self.recovered_expression_type(node, kind)
            {
                return Located::Computed(recovered);
            }
            return Located::Computed(ty);
        }
        if let Some(ty) = self.operator_type(node, kind) {
            return Located::Computed(ty);
        }
        // tsc types `a = b` as `b`, a JavaScript assignment declaration
        // (`module.exports = …`, `this.x = …`) included.
        if kind == SyntaxKind::BinaryExpression
            && self.is_plain_assignment(node)
            && let Some(right) = self.node_child(node, "right")
        {
            return self.type_of_expression(right, program.node_kind(right));
        }
        if matches!(
            kind,
            SyntaxKind::Identifier | SyntaxKind::PrivateIdentifier | SyntaxKind::PropertyAccessExpression | SyntaxKind::QualifiedName
        ) && let Some(symbol) = self.symbol_at_location(node)
        {
            return Located::Handle(self.type_of_symbol(symbol));
        }
        // `o["k"]` reads the member its literal key names, as `o.k` does.
        if kind == SyntaxKind::ElementAccessExpression
            && let Some(argument) = self.node_child(node, "argumentExpression")
            && matches!(
                program.node_kind(argument),
                SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral | SyntaxKind::NumericLiteral
            )
            && let Some(symbol) = self.symbol_at_location(argument)
        {
            return Located::Handle(self.type_of_symbol(symbol));
        }
        Located::Computed(self.recovered_expression_type(node, kind).unwrap_or(Type::ErrorType))
    }

    /// `getAssignmentTargetKind(node) === Definite`: the target of `=` (or a
    /// destructuring assignment's element), not of a compound assignment.
    fn is_definite_assignment_target(&self, node: NodeId) -> bool {
        let program = &self.program;
        let Some(parent) = program.node_parent(node) else { return false };
        match program.node_kind(parent) {
            // `&&=`, `||=` and `??=` are definite too
            // (`isLogicalOrCoalescingAssignmentOperator`).
            SyntaxKind::BinaryExpression => {
                self.node_child(parent, "left") == Some(node)
                    && self.node_child(parent, "operatorToken").is_some_and(|operator| {
                        matches!(
                            program.node_kind(operator),
                            SyntaxKind::EqualsToken
                                | SyntaxKind::AmpersandAmpersandEqualsToken
                                | SyntaxKind::BarBarEqualsToken
                                | SyntaxKind::QuestionQuestionEqualsToken
                        )
                    })
            }
            SyntaxKind::ParenthesizedExpression => self.is_definite_assignment_target(parent),
            SyntaxKind::PrefixUnaryExpression | SyntaxKind::PostfixUnaryExpression => false,
            _ => self.is_assignment_target(node),
        }
    }

    /// `new C()` of an abstract class: `resolveNewExpression` reports it and
    /// answers the error call.
    fn instantiates_abstract_class(&self, new_expression: NodeId) -> bool {
        let program = &self.program;
        let Some(callee) = self.node_child(new_expression, "expression") else { return false };
        let Some(symbol) = self.symbol_at_location(callee) else { return false };
        let class = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
        self.symbol_flags(class) & ts_sf::Class != 0
            && self.symbol_declarations(class).iter().any(|declaration| {
                program.node_kind(*declaration) == SyntaxKind::ClassDeclaration
                    && matches!(program.node_property(*declaration, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
                        if nodes.iter().any(|modifier| program.node_kind(*modifier) == SyntaxKind::AbstractKeyword))
            })
    }

    /// tsc's `getAssignmentTargetKind(node) !== None`: the target of an
    /// assignment operator or `++`/`--`, or of a destructuring assignment.
    fn is_assignment_target(&self, node: NodeId) -> bool {
        let program = &self.program;
        let Some(parent) = program.node_parent(node) else { return false };
        match program.node_kind(parent) {
            SyntaxKind::BinaryExpression => {
                self.node_child(parent, "left") == Some(node)
                    && self.node_child(parent, "operatorToken").is_some_and(|operator| {
                        matches!(
                            program.node_kind(operator),
                            SyntaxKind::EqualsToken
                                | SyntaxKind::PlusEqualsToken
                                | SyntaxKind::MinusEqualsToken
                                | SyntaxKind::AsteriskEqualsToken
                                | SyntaxKind::AsteriskAsteriskEqualsToken
                                | SyntaxKind::SlashEqualsToken
                                | SyntaxKind::PercentEqualsToken
                                | SyntaxKind::LessThanLessThanEqualsToken
                                | SyntaxKind::GreaterThanGreaterThanEqualsToken
                                | SyntaxKind::GreaterThanGreaterThanGreaterThanEqualsToken
                                | SyntaxKind::AmpersandEqualsToken
                                | SyntaxKind::BarEqualsToken
                                | SyntaxKind::CaretEqualsToken
                                | SyntaxKind::AmpersandAmpersandEqualsToken
                                | SyntaxKind::BarBarEqualsToken
                                | SyntaxKind::QuestionQuestionEqualsToken
                        )
                    })
            }
            SyntaxKind::PrefixUnaryExpression | SyntaxKind::PostfixUnaryExpression => {
                matches!(program.node_operator(parent), Some(SyntaxKind::PlusPlusToken | SyntaxKind::MinusMinusToken))
            }
            SyntaxKind::ParenthesizedExpression | SyntaxKind::NonNullExpression => self.is_assignment_target(parent),
            SyntaxKind::ArrayLiteralExpression => self.is_destructuring_target(parent),
            SyntaxKind::SpreadElement => program.node_parent(parent).is_some_and(|outer| {
                program.node_kind(outer) == SyntaxKind::ArrayLiteralExpression && self.is_destructuring_target(outer)
            }),
            SyntaxKind::PropertyAssignment => {
                self.node_child(parent, "initializer") == Some(node)
                    && program.node_parent(parent).is_some_and(|object| self.is_destructuring_target(object))
            }
            SyntaxKind::ShorthandPropertyAssignment => {
                program.node_parent(parent).is_some_and(|object| self.is_destructuring_target(object))
            }
            SyntaxKind::ForInStatement | SyntaxKind::ForOfStatement => self.node_child(parent, "initializer") == Some(node),
            _ => false,
        }
    }

    /// A write target `checkIdentifier` / `isAssignmentToReadonlyEntity`
    /// rejects: a name that is no variable, or is a constant; a readonly or
    /// getter-only property outside the constructor that initializes it; a
    /// member read through a namespace import.
    fn is_invalid_write_target(&self, node: NodeId, kind: SyntaxKind) -> bool {
        let program = &self.program;
        if kind == SyntaxKind::Identifier {
            let Some(symbol) = self.symbol_at_location(node) else { return false };
            let flags = self.symbol_flags(symbol);
            if flags & ts_sf::Variable == 0 {
                let script_kind = program.script_kind(node.source_file());
                let javascript = script_kind == super::enums_generated::script_kind::JS
                    || script_kind == super::enums_generated::script_kind::JSX;
                return !(javascript && flags & ts_sf::ValueModule != 0) && !self.is_unresolved_symbol(symbol);
            }
            return self.is_constant_variable(symbol);
        }
        let Some(object) = self.node_child(node, "expression") else { return false };
        let mut object_node = object;
        while program.node_kind(object_node) == SyntaxKind::ParenthesizedExpression {
            let Some(inner) = self.node_child(object_node, "expression") else { break };
            object_node = inner;
        }
        // An element access names its member by a literal argument.
        let name = match kind {
            SyntaxKind::ElementAccessExpression => match self.node_child(node, "argumentExpression") {
                Some(argument)
                    if matches!(
                        program.node_kind(argument),
                        SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral | SyntaxKind::NumericLiteral
                    ) =>
                {
                    argument
                }
                _ => return false,
            },
            _ => match self.node_child(node, "name") {
                Some(name) => name,
                None => return false,
            },
        };
        let Some(symbol) = self.symbol_at_location(name) else { return false };
        let flags = self.symbol_flags(symbol);
        let declarations = self.symbol_declarations(symbol);
        // A mapped or instantiated member's readonly is the mapping's, not
        // its declaration's.
        let written_readonly = flags & ts_sf::Property != 0
            && !matches!(self.symbol_data(symbol), SymbolData::Transient { .. })
            && declarations.iter().any(|declaration| self.has_readonly_modifier(*declaration));
        let computed_readonly = !written_readonly
            && flags & ts_sf::Property != 0
            && program.node_value_text(name).is_some_and(|name| {
                let object_type = self.located_type(self.type_of_expression(object, program.node_kind(object)));
                self.query(|query| query.property(&object_type, name)).flatten().is_some_and(|property| property.readonly)
            });
        let readonly = written_readonly
            || computed_readonly
            || flags & ts_sf::Variable != 0 && self.is_constant_variable(symbol)
            || flags & ts_sf::Accessor != 0 && flags & ts_sf::SetAccessor == 0
            || flags & ts_sf::EnumMember != 0;
        if readonly {
            // A class's own readonly property is written by its constructor.
            if flags & ts_sf::Property != 0 && program.node_kind(object_node) == SyntaxKind::ThisKeyword {
                let mut container = program.node_parent(node);
                while let Some(current) = container {
                    match program.node_kind(current) {
                        SyntaxKind::ArrowFunction => container = program.node_parent(current),
                        SyntaxKind::Constructor => {
                            let class = program.node_parent(current);
                            return !declarations.iter().any(|declaration| {
                                program.node_parent(*declaration) == class || program.node_parent(*declaration) == Some(current)
                            });
                        }
                        kind if super::classify::is_declaration_kind(kind) && kind != SyntaxKind::VariableDeclaration => return true,
                        _ => container = program.node_parent(current),
                    }
                }
            }
            return true;
        }
        program.node_kind(object_node) == SyntaxKind::Identifier
            && self.symbol_at_location(object_node).is_some_and(|object_symbol| {
                self.is_alias(object_symbol)
                    && self
                        .symbol_declarations(object_symbol)
                        .first()
                        .is_some_and(|declaration| program.node_kind(*declaration) == SyntaxKind::NamespaceImport)
            })
    }

    /// `getDeclarationNodeFlagsFromSymbol(symbol) & Constant`: a `const`,
    /// `using` or `await using` variable.
    fn is_constant_variable(&self, symbol: SymbolId) -> bool {
        let program = &self.program;
        self.symbol_value_declaration(symbol).is_some_and(|declaration| {
            program.node_kind(declaration) == SyntaxKind::VariableDeclaration
                && program.node_parent(declaration).is_some_and(|list| {
                    program.node_kind(list) == SyntaxKind::VariableDeclarationList
                        && program.node_flags(list)
                            & (super::enums_generated::node_flags::Const | super::enums_generated::node_flags::Using)
                            != 0
                })
        })
    }

    pub(super) fn list_property(&self, node: NodeId, property: &str) -> Vec<NodeId> {
        match self.program.node_property(node, property) {
            Some(NodePropertyValue::List { nodes, .. }) => nodes,
            _ => Vec::new(),
        }
    }

    fn is_plain_assignment(&self, binary: NodeId) -> bool {
        self.program.node_kind(binary) == SyntaxKind::BinaryExpression
            && self
                .node_child(binary, "operatorToken")
                .is_some_and(|operator| self.program.node_kind(operator) == SyntaxKind::EqualsToken)
    }

    /// tsc's `isAssignmentTarget` for an object or array literal: the target
    /// of a plain assignment, or an element or property value of one.
    fn is_destructuring_target(&self, literal: NodeId) -> bool {
        let program = &self.program;
        let Some(parent) = program.node_parent(literal) else { return false };
        match program.node_kind(parent) {
            SyntaxKind::BinaryExpression => self.is_plain_assignment(parent) && self.node_child(parent, "left") == Some(literal),
            SyntaxKind::ArrayLiteralExpression => self.is_destructuring_target(parent),
            SyntaxKind::PropertyAssignment => {
                self.node_child(parent, "initializer") == Some(literal)
                    && program.node_parent(parent).is_some_and(|object| self.is_destructuring_target(object))
            }
            SyntaxKind::SpreadElement | SyntaxKind::SpreadAssignment => {
                program.node_parent(parent).is_some_and(|outer| self.is_destructuring_target(outer))
            }
            _ => false,
        }
    }

    /// What types a literal contextually where the API can tell: a written
    /// type, or a destructuring pattern (which gives an array literal a
    /// tuple context and no literal types to keep).
    fn literal_context(&self, literal: NodeId) -> LiteralContext {
        let program = &self.program;
        let Some(parent) = program.node_parent(literal) else { return LiteralContext::None };
        match program.node_kind(parent) {
            SyntaxKind::ParenthesizedExpression => self.literal_context(parent),
            SyntaxKind::VariableDeclaration | SyntaxKind::PropertyDeclaration | SyntaxKind::Parameter
                if self.node_child(parent, "initializer") == Some(literal) =>
            {
                if let Some(annotation) = self.node_child(parent, "type") {
                    return self.type_of_type_node(annotation).map_or(LiteralContext::Unknown, LiteralContext::Type);
                }
                match self.node_child(parent, "name") {
                    Some(name)
                        if matches!(
                            program.node_kind(name),
                            SyntaxKind::ObjectBindingPattern | SyntaxKind::ArrayBindingPattern
                        ) =>
                    {
                        LiteralContext::Pattern(name)
                    }
                    _ => LiteralContext::None,
                }
            }
            SyntaxKind::BinaryExpression
                if self.is_plain_assignment(parent) && self.node_child(parent, "right") == Some(literal) =>
            {
                let Some(left) = self.node_child(parent, "left") else { return LiteralContext::Unknown };
                match program.node_kind(left) {
                    SyntaxKind::ObjectLiteralExpression | SyntaxKind::ArrayLiteralExpression => LiteralContext::Pattern(left),
                    _ => LiteralContext::Unknown,
                }
            }
            SyntaxKind::ArrayLiteralExpression => {
                let Some(index) = self.list_property(parent, "elements").iter().position(|element| *element == literal) else {
                    return LiteralContext::Unknown;
                };
                match self.literal_context(parent) {
                    LiteralContext::Type(ty) => match ty.peeled() {
                        Type::Array(element) => LiteralContext::Type(*element),
                        Type::Tuple(elements) => elements.get(index).cloned().map_or(LiteralContext::Unknown, LiteralContext::Type),
                        _ => LiteralContext::Unknown,
                    },
                    LiteralContext::Pattern(pattern) => self.pattern_element(pattern, index),
                    LiteralContext::None => LiteralContext::None,
                    LiteralContext::Unknown => LiteralContext::Unknown,
                }
            }
            SyntaxKind::PropertyAssignment if self.node_child(parent, "initializer") == Some(literal) => {
                let Some(object) = program.node_parent(parent) else { return LiteralContext::Unknown };
                let Some(key) = self.node_child(parent, "name").and_then(|name| self.literal_property_name(name)) else {
                    return LiteralContext::Unknown;
                };
                match self.literal_context(object) {
                    LiteralContext::Type(ty) => self
                        .query(|query| query.property(&ty, &key))
                        .flatten()
                        .map_or(LiteralContext::Unknown, |property| LiteralContext::Type(property.ty)),
                    LiteralContext::Pattern(pattern) => self.pattern_property(pattern, &key),
                    LiteralContext::None => LiteralContext::None,
                    LiteralContext::Unknown => LiteralContext::Unknown,
                }
            }
            _ => LiteralContext::Unknown,
        }
    }

    fn literal_property_name(&self, name: NodeId) -> Option<String> {
        match self.program.node_kind(name) {
            SyntaxKind::Identifier | SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral => {
                self.program.node_value_text(name).map(str::to_string)
            }
            SyntaxKind::NumericLiteral => self.program.node_value_text(name).map(normalize_number),
            _ => None,
        }
    }

    /// The pattern a destructuring pattern's element `index` nests, if any.
    fn pattern_element(&self, pattern: NodeId, index: usize) -> LiteralContext {
        let Some(&element) = self.list_property(pattern, "elements").get(index) else { return LiteralContext::None };
        let nested = match self.program.node_kind(element) {
            SyntaxKind::BindingElement => self.node_child(element, "name"),
            _ => Some(element),
        };
        self.nested_pattern(nested)
    }

    /// The pattern a destructuring pattern's property `key` nests, if any.
    fn pattern_property(&self, pattern: NodeId, key: &str) -> LiteralContext {
        let program = &self.program;
        let members = match program.node_kind(pattern) {
            SyntaxKind::ObjectBindingPattern => self.list_property(pattern, "elements"),
            SyntaxKind::ObjectLiteralExpression => self.list_property(pattern, "properties"),
            _ => return LiteralContext::None,
        };
        for member in members {
            let name = self.node_child(member, "propertyName").or_else(|| self.node_child(member, "name"));
            if name.and_then(|name| self.literal_property_name(name)).as_deref() != Some(key) {
                continue;
            }
            let nested = match program.node_kind(member) {
                SyntaxKind::BindingElement => self.node_child(member, "name"),
                SyntaxKind::PropertyAssignment => self.node_child(member, "initializer"),
                _ => None,
            };
            return self.nested_pattern(nested);
        }
        LiteralContext::None
    }

    fn nested_pattern(&self, node: Option<NodeId>) -> LiteralContext {
        match node.map(|node| (node, self.program.node_kind(node))) {
            Some((
                node,
                SyntaxKind::ObjectBindingPattern
                | SyntaxKind::ArrayBindingPattern
                | SyntaxKind::ObjectLiteralExpression
                | SyntaxKind::ArrayLiteralExpression,
            )) => LiteralContext::Pattern(node),
            _ => LiteralContext::None,
        }
    }

    /// Whether an expression's type is a fresh literal (`getWidenedLiteralType`
    /// widens only those): a literal written there, or a `const` its literal
    /// initializer types.
    fn is_fresh_literal_expression(&self, expression: NodeId) -> bool {
        let program = &self.program;
        match program.node_kind(expression) {
            SyntaxKind::StringLiteral
            | SyntaxKind::NumericLiteral
            | SyntaxKind::BigIntLiteral
            | SyntaxKind::NoSubstitutionTemplateLiteral
            | SyntaxKind::TrueKeyword
            | SyntaxKind::FalseKeyword
            | SyntaxKind::TemplateExpression => true,
            SyntaxKind::PrefixUnaryExpression => {
                self.node_child(expression, "operand").is_some_and(|operand| self.is_fresh_literal_expression(operand))
            }
            SyntaxKind::ParenthesizedExpression => {
                self.node_child(expression, "expression").is_some_and(|inner| self.is_fresh_literal_expression(inner))
            }
            SyntaxKind::ConditionalExpression => ["whenTrue", "whenFalse"].iter().all(|branch| {
                self.node_child(expression, branch).is_some_and(|branch| self.is_fresh_literal_expression(branch))
            }),
            SyntaxKind::Identifier => {
                let shorthand = program
                    .node_parent(expression)
                    .is_some_and(|parent| program.node_kind(parent) == SyntaxKind::ShorthandPropertyAssignment);
                let symbol = if shorthand {
                    program.node_value_text(expression).and_then(|name| self.resolve_name(expression, name, Meaning::Value))
                } else {
                    self.symbol_at_location(expression)
                };
                symbol
                .and_then(|symbol| self.symbol_value_declaration(symbol))
                .is_some_and(|declaration| {
                    program.node_kind(declaration) == SyntaxKind::VariableDeclaration
                        && self.node_child(declaration, "type").is_none()
                        && program.node_parent(declaration).is_some_and(|list| {
                            program.node_flags(list) & super::enums_generated::node_flags::Const != 0
                        })
                        && self
                            .node_child(declaration, "initializer")
                            .is_some_and(|initializer| self.is_fresh_literal_expression(initializer))
                })
            }
            _ => false,
        }
    }

    /// `checkExpressionForMutableLocation`: an element's or property's type,
    /// its fresh literal widened unless the contextual type keeps literals of
    /// its kind (`isLiteralOfContextualType`).
    fn mutable_location_type(&self, expression: NodeId, contextual: Option<&Type>) -> Option<Type> {
        let ty = self.located_type(self.type_of_expression(expression, self.program.node_kind(expression)));
        self.widened_for_mutable_location(expression, ty, contextual)
    }

    fn widened_for_mutable_location(&self, expression: NodeId, ty: Type, contextual: Option<&Type>) -> Option<Type> {
        let ty = match ty {
            // An element the checker could not type is the error type tsc
            // gives a failed expression.
            Type::ErrorType | Type::Unknown => Type::Any,
            ty if ty.is_degraded() => return None,
            ty => ty,
        };
        if !self.is_fresh_literal_expression(expression)
            || contextual.is_some_and(|contextual| keeps_literal(contextual, &ty))
        {
            return Some(ty);
        }
        self.query(|query| query.widened_literal(&ty))
    }

    fn unrecorded_literal_type(&self, literal: NodeId, kind: SyntaxKind) -> Option<Type> {
        if self.is_destructuring_target(literal) {
            return self.target_literal_type(literal);
        }
        let context = self.literal_context(literal);
        let contextual = match &context {
            LiteralContext::Type(ty) => Some(ty.clone()),
            _ => None,
        };
        if kind == SyntaxKind::ArrayLiteralExpression {
            let in_tuple_context = match &context {
                LiteralContext::Pattern(pattern) => matches!(
                    self.program.node_kind(*pattern),
                    SyntaxKind::ArrayBindingPattern | SyntaxKind::ArrayLiteralExpression
                ),
                LiteralContext::Type(ty) => is_tuple_like(ty),
                _ => false,
            };
            let mut elements = Vec::new();
            let mut spread = false;
            for (index, element) in self.list_property(literal, "elements").into_iter().enumerate() {
                let element_context = match contextual.as_ref().map(Type::peeled) {
                    Some(Type::Array(element)) => Some(*element),
                    Some(Type::Tuple(types)) => types.get(index).cloned(),
                    _ => None,
                };
                elements.push(match self.program.node_kind(element) {
                    SyntaxKind::OmittedExpression => Type::Undefined,
                    SyntaxKind::SpreadElement if !in_tuple_context => {
                        spread = true;
                        self.spread_element_type(self.node_child(element, "expression")?)?
                    }
                    SyntaxKind::SpreadElement => return None,
                    _ => self.mutable_location_type(element, element_context.as_ref())?,
                });
            }
            if spread {
                return Some(Type::Array(Box::new(surge_ts_types::union_type(elements))));
            }
            if in_tuple_context {
                return Some(Type::tuple(elements));
            }
            if elements.is_empty() {
                return Some(Type::Array(Box::new(Type::Never)));
            }
            return Some(Type::Array(Box::new(surge_ts_types::union_type(elements))));
        }
        let mut properties = surge_ts_types::PropertyMap::default();
        for member in self.list_property(literal, "properties") {
            let key = self.node_child(member, "name").and_then(|name| self.literal_property_name(name))?;
            let property_context = contextual
                .as_ref()
                .and_then(|contextual| self.query(|query| query.property(contextual, &key)).flatten())
                .map(|property| property.ty);
            let ty = match self.program.node_kind(member) {
                SyntaxKind::PropertyAssignment => {
                    self.mutable_location_type(self.node_child(member, "initializer")?, property_context.as_ref())?
                }
                // The name reads the variable it names, not the property it
                // declares.
                SyntaxKind::ShorthandPropertyAssignment => {
                    let name = self.node_child(member, "name")?;
                    let value = self.resolve_name(name, self.program.node_value_text(name)?, Meaning::Value);
                    let ty = value.map_or(Type::ErrorType, |value| self.ty(self.type_of_symbol(value)));
                    self.widened_for_mutable_location(name, ty, property_context.as_ref())?
                }
                _ => return None,
            };
            properties.insert(Arc::from(key.as_str()), surge_ts_types::ObjectProperty::required(ty));
        }
        Some(Type::Object(surge_ts_types::ObjectType::new(properties, None)))
    }

    /// The elements a spread adds to an array literal: an array's element, or
    /// the error type of what is not iterable (`checkIteratedTypeOrElementType`).
    fn spread_element_type(&self, operand: NodeId) -> Option<Type> {
        let ty = self.located_type(self.type_of_expression(operand, self.program.node_kind(operand)));
        let not_iterable = |ty: &Type| match ty {
            Type::Number
            | Type::NumberLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Unknown
            | Type::ErrorType => true,
            Type::Object(object) => {
                object.call_signature.is_none()
                    && object.construct_signature.is_none()
                    && object.alias_name.is_none()
                    && !object.properties.keys().any(|key| key.contains("iterator"))
            }
            _ => false,
        };
        match ty.peeled() {
            Type::Array(element) => Some(*element),
            Type::Any => Some(Type::Any),
            Type::Union(union) if union.types().iter().any(not_iterable) => Some(Type::Any),
            ty if not_iterable(&ty) => Some(Type::Any),
            _ => None,
        }
    }

    /// What a CommonJS JavaScript file names without declaring: `require(m)`
    /// is module `m`, `exports` and `module.exports` the file's own module,
    /// `module` an object holding it (tsc's `getTypeOfSymbol` of the
    /// file's `module`/`exports` symbols).
    fn commonjs_type(&self, node: NodeId, kind: SyntaxKind) -> Option<Located> {
        let program = &self.program;
        let script_kind = program.script_kind(node.source_file());
        if script_kind != super::enums_generated::script_kind::JS && script_kind != super::enums_generated::script_kind::JSX {
            return None;
        }
        let free = |name: NodeId, text: &str| {
            program.node_kind(name) == SyntaxKind::Identifier
                && program.node_text(name) == text
                && self.symbol_at_location(name).is_none_or(|symbol| self.is_unresolved_symbol(symbol))
        };
        // `module.exports = value` makes the module that value; otherwise it
        // is the file's namespace, named by the file.
        let own_module = || -> Option<Located> {
            let checker_index = program.file(node.source_file()).checker_index?;
            if let Some(assigned) = self
                .query(|query| query.module_export_assignment_type(checker_index))
                .flatten()
                .filter(|ty| !matches!(ty, Type::Any | Type::ErrorType | Type::Unknown))
            {
                return Some(Located::Computed(assigned));
            }
            let Type::Object(namespace) = self.query(|query| query.module_namespace_type(checker_index)).flatten()? else {
                return None;
            };
            let file_name = program.file_name(node.source_file());
            let module = file_name.rsplit_once('.').map_or(file_name, |(module, _)| module);
            Some(Located::Computed(Type::Object(namespace.with_alias_name(format!("typeof import({module:?})")))))
        };
        match kind {
            SyntaxKind::CallExpression => {
                let callee = self.node_child(node, "expression")?;
                let [argument] = self.list_property(node, "arguments")[..] else { return None };
                if !free(callee, "require") || program.node_kind(argument) != SyntaxKind::StringLiteral {
                    return None;
                }
                self.module_type_of_specifier(node, program.node_value_text(argument)?).map(Located::Computed)
            }
            SyntaxKind::Identifier if free(node, "exports") && !self.is_access_name(node) => own_module(),
            SyntaxKind::Identifier if free(node, "module") && !self.is_access_name(node) => {
                let exports = self.located_type(own_module()?);
                let mut properties = surge_ts_types::PropertyMap::default();
                properties.insert(Arc::from("exports"), surge_ts_types::ObjectProperty::required(exports));
                Some(Located::Computed(Type::Object(surge_ts_types::ObjectType::new(properties, None))))
            }
            SyntaxKind::PropertyAccessExpression => {
                let object = self.node_child(node, "expression")?;
                let name = self.node_child(node, "name")?;
                (free(object, "module") && program.node_text(name) == "exports").then(own_module).flatten()
            }
            _ => None,
        }
    }

    fn module_type_of_specifier(&self, at: NodeId, specifier: &str) -> Option<Type> {
        let checker_index = self.program.file(at.source_file()).checker_index?;
        let text = format!("typeof import({specifier:?})");
        self.query(|query| query.resolve_type_text(checker_index, &text, &[], &[]))
            .flatten()
            .filter(|ty| !matches!(ty, Type::Any | Type::ErrorType | Type::Unknown))
    }

    fn is_access_name(&self, node: NodeId) -> bool {
        self.program.node_parent(node).is_some_and(|parent| {
            self.program.node_kind(parent) == SyntaxKind::PropertyAccessExpression && self.node_child(parent, "name") == Some(node)
        })
    }

    /// An object literal the checker recorded with `any` members where it gave
    /// up on their values, with the types tsc still gives those values.
    fn with_recovered_members(&self, literal: NodeId, recorded: &Type) -> Option<Type> {
        let Type::Object(object) = recorded else { return None };
        let contextual = match self.literal_context(literal) {
            LiteralContext::Type(ty) => Some(ty),
            _ => None,
        };
        let mut properties = None::<surge_ts_types::PropertyMap>;
        for member in self.list_property(literal, "properties") {
            if self.program.node_kind(member) != SyntaxKind::PropertyAssignment {
                continue;
            }
            let Some(key) = self.node_child(member, "name").and_then(|name| self.literal_property_name(name)) else { continue };
            if !object.get_property(&key).is_some_and(|property| matches!(property.ty, Type::Any | Type::ErrorType | Type::Unknown)) {
                continue;
            }
            let property_context = contextual
                .as_ref()
                .and_then(|contextual| self.query(|query| query.property(contextual, &key)).flatten())
                .map(|property| property.ty);
            let Some(ty) = self.node_child(member, "initializer").and_then(|value| {
                let ty = self.ty(self.type_at_location(value));
                // `getWidenedLiteralLikeTypeForContextualType` widens literals
                // alone; a nested literal's members were widened as its own.
                if matches!(ty, Type::Object(_))
                    || property_context.as_ref().is_some_and(|contextual| keeps_literal(contextual, &ty))
                {
                    return Some(ty);
                }
                self.query(|query| query.widened_literal(&ty))
            }) else {
                continue;
            };
            if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown) {
                continue;
            }
            let properties = properties.get_or_insert_with(|| (*object.properties).clone());
            if let Some(property) = properties.get_mut(key.as_str()) {
                property.ty = ty;
            }
        }
        let mut recovered = object.clone();
        recovered.properties = Arc::new(properties?);
        recovered.property_map_id = None;
        Some(Type::Object(recovered))
    }

    /// A destructuring assignment's target literal, typed by the targets it
    /// assigns (`checkArrayLiteral` makes an array target a tuple).
    fn target_literal_type(&self, literal: NodeId) -> Option<Type> {
        let program = &self.program;
        let target_type = |target: NodeId| -> Option<Type> {
            match program.node_kind(target) {
                // A default (`[x = 1] = …`) is checked as the assignment it
                // is, typed as its widened value.
                SyntaxKind::BinaryExpression if self.is_plain_assignment(target) => {
                    let value = self.ty(self.type_at_location(self.node_child(target, "right")?));
                    self.query(|query| query.widened_literal(&value))
                        .filter(|ty| !matches!(ty, Type::Unknown | Type::ErrorType) && !ty.is_degraded())
                }
                SyntaxKind::ObjectLiteralExpression | SyntaxKind::ArrayLiteralExpression => self.target_literal_type(target),
                SyntaxKind::Identifier => {
                    let symbol = self.symbol_at_location(target)?;
                    let ty = self.ty(self.type_of_symbol(symbol));
                    (!matches!(ty, Type::Unknown) && !ty.is_degraded()).then_some(ty)
                }
                // What cannot be assigned (`[f()] = …`) is still checked as
                // the expression it is.
                _ => {
                    let ty = self.ty(self.type_at_location(target));
                    (!matches!(ty, Type::Unknown | Type::ErrorType) && !ty.is_degraded()).then_some(ty)
                }
            }
        };
        if program.node_kind(literal) == SyntaxKind::ArrayLiteralExpression {
            let written = self.list_property(literal, "elements");
            // `[...rest] = value` is the tuple `[...T[]]`, which is `T[]`.
            if let [only] = written.as_slice()
                && program.node_kind(*only) == SyntaxKind::SpreadElement
            {
                let rest = target_type(self.node_child(*only, "expression")?)?;
                return matches!(rest.peeled(), Type::Array(_)).then_some(rest);
            }
            let mut elements = Vec::new();
            let mut rest = None;
            for element in written {
                let ty = match program.node_kind(element) {
                    SyntaxKind::OmittedExpression => Type::Undefined,
                    // `[...rest, last] = value` is `[...T[], U]`.
                    SyntaxKind::SpreadElement if rest.is_none() => {
                        let Type::Array(element) = target_type(self.node_child(element, "expression")?)?.peeled() else { return None };
                        rest = Some((elements.len(), element));
                        continue;
                    }
                    _ => target_type(element)?,
                };
                elements.push(ty);
            }
            if let Some((at, element)) = rest {
                let trailing = elements.split_off(at);
                return Some(Type::OpenTuple(surge_ts_types::OpenTupleType { leading: elements, rest: element, trailing, labels: None }));
            }
            return Some(Type::tuple(elements));
        }
        let mut properties = surge_ts_types::PropertyMap::default();
        for member in self.list_property(literal, "properties") {
            let key = self.node_child(member, "name").and_then(|name| self.literal_property_name(name))?;
            let property = match program.node_kind(member) {
                SyntaxKind::PropertyAssignment => {
                    surge_ts_types::ObjectProperty::required(target_type(self.node_child(member, "initializer")?)?)
                }
                SyntaxKind::ShorthandPropertyAssignment => {
                    // The name assigns the variable it names, not the
                    // property it declares.
                    let name = self.node_child(member, "name")?;
                    let target = self.resolve_name(name, program.node_value_text(name)?, Meaning::Value)?;
                    let ty = self.ty(self.type_of_symbol(target));
                    if matches!(ty, Type::Unknown) || ty.is_degraded() {
                        return None;
                    }
                    if self.node_child(member, "objectAssignmentInitializer").is_some() {
                        surge_ts_types::ObjectProperty::optional(ty)
                    } else {
                        surge_ts_types::ObjectProperty::required(ty)
                    }
                }
                _ => return None,
            };
            properties.insert(Arc::from(key.as_str()), property);
        }
        Some(Type::Object(surge_ts_types::ObjectType::new(properties, None)))
    }

    /// An operator expression the checker left unrecorded (a condition it
    /// narrows by rather than evaluates), typed as tsc's
    /// `checkPrefixUnaryExpression` and `checkBinaryLikeExpression` type it
    /// whatever their operands are, or from an operand's type it does know.
    fn operator_type(&self, node: NodeId, kind: SyntaxKind) -> Option<Type> {
        let program = &self.program;
        let operand_type = |operand: NodeId| {
            let ty = self.located_type(self.type_of_expression(operand, program.node_kind(operand)));
            (!matches!(ty, Type::ErrorType | Type::Unknown)).then_some(ty)
        };
        match kind {
            SyntaxKind::PrefixUnaryExpression => {
                let operand = self.node_child(node, "operand")?;
                let operator = program.node_operator(node)?;
                let operand_kind = program.node_kind(operand);
                if matches!(operator, SyntaxKind::MinusToken | SyntaxKind::PlusToken) {
                    if operand_kind == SyntaxKind::NumericLiteral {
                        let value = normalize_number(program.node_value_text(operand)?);
                        let value = match (operator, value.as_str()) {
                            (SyntaxKind::PlusToken, _) | (_, "0") => value,
                            (_, _) => format!("-{value}"),
                        };
                        return Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType { value }));
                    }
                    if operand_kind == SyntaxKind::BigIntLiteral {
                        return Some(Type::BigInt);
                    }
                }
                match operator {
                    SyntaxKind::PlusToken => Some(Type::Number),
                    // `getUnaryResultType`: a number, a bigint for a bigint
                    // operand, either for one that may be both.
                    SyntaxKind::MinusToken | SyntaxKind::TildeToken => {
                        let operand = operand_type(operand).unwrap_or(Type::Any);
                        let kinds = |ty: &Type| match ty.peeled() {
                            Type::Union(union) => union.types().iter().filter_map(Type::base_primitive).collect::<Vec<_>>(),
                            other => other.base_primitive().into_iter().collect(),
                        };
                        let kinds = kinds(&operand);
                        Some(match (kinds.contains(&Type::BigInt), kinds.contains(&Type::Number)) {
                            (true, true) => surge_ts_types::union_type(vec![Type::Number, Type::BigInt]),
                            (true, false) => Type::BigInt,
                            _ => Type::Number,
                        })
                    }
                    // `checkPrefixUnaryExpression`: `++x` is a number, or a
                    // bigint for a bigint operand.
                    SyntaxKind::PlusPlusToken | SyntaxKind::MinusMinusToken => Some(match operand_type(operand) {
                        Some(Type::BigInt) => Type::BigInt,
                        _ => Type::Number,
                    }),
                    // An operand typed as the error type is `any`, whose
                    // negation is `boolean`.
                    SyntaxKind::ExclamationToken => match operand_type(operand) {
                        Some(operand) => self.query(|query| query.logical_not_type(&operand)),
                        None => Some(Type::Boolean),
                    },
                    _ => None,
                }
            }
            // `checkPostfixUnaryExpression`: `x++` is a number, or a bigint for
            // a bigint operand, as `++x` is.
            SyntaxKind::PostfixUnaryExpression => {
                let operand = self.node_child(node, "operand")?;
                Some(match operand_type(operand) {
                    Some(Type::BigInt) => Type::BigInt,
                    _ => Type::Number,
                })
            }
            // `x as T` is `T`; `x as const` the operand's literal type, which
            // the checker records as the operand's.
            SyntaxKind::AsExpression | SyntaxKind::TypeAssertionExpression => {
                let asserted = self.node_child(node, "type")?;
                let is_const = program.node_kind(asserted) == SyntaxKind::TypeReference
                    && self.node_child(asserted, "typeName").is_some_and(|name| program.node_text(name) == "const");
                if is_const {
                    operand_type(self.node_child(node, "expression")?)
                } else {
                    self.type_of_type_node(asserted).filter(|ty| !matches!(ty, Type::ErrorType | Type::Unknown))
                }
            }
            SyntaxKind::AwaitExpression => {
                let operand = operand_type(self.node_child(node, "expression")?)?;
                self.query(|query| query.awaited_type(&operand)).filter(|ty| !ty.is_degraded())
            }
            SyntaxKind::TypeOfExpression => self.query(|query| query.typeof_type()),
            SyntaxKind::VoidExpression => Some(Type::Undefined),
            SyntaxKind::DeleteExpression => Some(Type::Boolean),
            SyntaxKind::BinaryExpression => {
                // A compound assignment is typed as its operator's result.
                let operator = match program.node_kind(self.node_child(node, "operatorToken")?) {
                    SyntaxKind::PlusEqualsToken => SyntaxKind::PlusToken,
                    SyntaxKind::MinusEqualsToken => SyntaxKind::MinusToken,
                    SyntaxKind::AsteriskEqualsToken => SyntaxKind::AsteriskToken,
                    SyntaxKind::AsteriskAsteriskEqualsToken => SyntaxKind::AsteriskAsteriskToken,
                    SyntaxKind::SlashEqualsToken => SyntaxKind::SlashToken,
                    SyntaxKind::PercentEqualsToken => SyntaxKind::PercentToken,
                    SyntaxKind::LessThanLessThanEqualsToken => SyntaxKind::LessThanLessThanToken,
                    SyntaxKind::GreaterThanGreaterThanEqualsToken => SyntaxKind::GreaterThanGreaterThanToken,
                    SyntaxKind::GreaterThanGreaterThanGreaterThanEqualsToken => SyntaxKind::GreaterThanGreaterThanGreaterThanToken,
                    SyntaxKind::AmpersandEqualsToken => SyntaxKind::AmpersandToken,
                    SyntaxKind::BarEqualsToken => SyntaxKind::BarToken,
                    SyntaxKind::CaretEqualsToken => SyntaxKind::CaretToken,
                    operator => operator,
                };
                if operator == SyntaxKind::CommaToken {
                    let right = self.node_child(node, "right")?;
                    return operand_type(right);
                }
                let operands = || Some((operand_type(self.node_child(node, "left")?)?, operand_type(self.node_child(node, "right")?)?));
                let primitive = |ty: &Type| ty.base_primitive();
                match operator {
                    SyntaxKind::BarBarToken | SyntaxKind::AmpersandAmpersandToken => {
                        let (left, right) = operands()?;
                        return self.query(|query| query.logical_type(operator == SyntaxKind::BarBarToken, &left, &right)).flatten();
                    }
                    SyntaxKind::QuestionQuestionToken => {
                        let (left, right) = operands()?;
                        return self.query(|query| query.nullish_coalescing_type(&left, &right));
                    }
                    // `checkBinaryLikeExpression`'s `+`: numbers add, bigints add,
                    // a string on either side concatenates, `any` stays `any`.
                    SyntaxKind::PlusToken => {
                        let (left, right) = operands()?;
                        return match (primitive(&left), primitive(&right)) {
                            (Some(Type::Number), Some(Type::Number)) => Some(Type::Number),
                            (Some(Type::BigInt), Some(Type::BigInt)) => Some(Type::BigInt),
                            (Some(Type::String), _) | (_, Some(Type::String)) => Some(Type::String),
                            _ if matches!(left, Type::Any) || matches!(right, Type::Any) => Some(Type::Any),
                            _ => None,
                        };
                    }
                    SyntaxKind::MinusToken
                    | SyntaxKind::AsteriskToken
                    | SyntaxKind::AsteriskAsteriskToken
                    | SyntaxKind::SlashToken
                    | SyntaxKind::PercentToken
                    | SyntaxKind::LessThanLessThanToken
                    | SyntaxKind::GreaterThanGreaterThanToken
                    | SyntaxKind::GreaterThanGreaterThanGreaterThanToken
                    | SyntaxKind::AmpersandToken
                    | SyntaxKind::BarToken
                    | SyntaxKind::CaretToken => {
                        // A number unless a side may be a bigint; two bigints
                        // are one; a mix is the error type.
                        let (left, right) = operands()?;
                        let maybe_bigint = |ty: &Type| match ty.peeled() {
                            Type::Union(union) => union.types().iter().any(|member| primitive(member) == Some(Type::BigInt)),
                            other => primitive(&other) == Some(Type::BigInt),
                        };
                        return Some(if !maybe_bigint(&left) && !maybe_bigint(&right) {
                            Type::Number
                        } else if primitive(&left) == Some(Type::BigInt) && primitive(&right) == Some(Type::BigInt) {
                            Type::BigInt
                        } else {
                            Type::ErrorType
                        });
                    }
                    _ => {}
                }
                matches!(
                    operator,
                    SyntaxKind::LessThanToken
                        | SyntaxKind::GreaterThanToken
                        | SyntaxKind::LessThanEqualsToken
                        | SyntaxKind::GreaterThanEqualsToken
                        | SyntaxKind::EqualsEqualsToken
                        | SyntaxKind::ExclamationEqualsToken
                        | SyntaxKind::EqualsEqualsEqualsToken
                        | SyntaxKind::ExclamationEqualsEqualsToken
                        | SyntaxKind::InstanceOfKeyword
                        | SyntaxKind::InKeyword
                )
                .then_some(Type::Boolean)
            }
            SyntaxKind::PropertyAccessExpression => self.unnarrowed_access_type(node),
            SyntaxKind::CallExpression => self.unrecorded_call_type(node),
            SyntaxKind::ElementAccessExpression => self.element_write_target_type(node),
            _ => None,
        }
    }

    /// `o["key"] = value` or `t[0] = value`: the target is the property the
    /// literal names, at its declared type.
    fn element_write_target_type(&self, access: NodeId) -> Option<Type> {
        let program = &self.program;
        let parent = program.node_parent(access)?;
        let is_target = program.node_kind(parent) == SyntaxKind::BinaryExpression
            && self.node_child(parent, "left") == Some(access)
            && self
                .node_child(parent, "operatorToken")
                .is_some_and(|operator| program.node_kind(operator) == SyntaxKind::EqualsToken);
        if !is_target {
            return None;
        }
        let argument = self.node_child(access, "argumentExpression")?;
        if !matches!(program.node_kind(argument), SyntaxKind::StringLiteral | SyntaxKind::NumericLiteral) {
            return None;
        }
        let name = program.node_value_text(argument)?.to_string();
        let object = self.node_child(access, "expression")?;
        let object_type = self.located_type(self.type_of_expression(object, program.node_kind(object)));
        if matches!(object_type, Type::ErrorType | Type::Unknown) {
            return None;
        }
        if let Some(property) = self.query(|query| query.property(&object_type, &name)).flatten() {
            return Some(self.property_type(&property));
        }
        let numeric = program.node_kind(argument) == SyntaxKind::NumericLiteral;
        match object_type.peeled() {
            Type::Array(element) if numeric => Some(*element),
            _ => self.query(|query| query.index_type(&object_type, numeric)).flatten(),
        }
    }

    /// A call the checker recorded no result for, whose callee has one
    /// signature that nothing instantiates: that signature's return type.
    fn unrecorded_call_type(&self, call: NodeId) -> Option<Type> {
        let program = &self.program;
        let callee = self.node_child(call, "expression")?;
        if program.node_kind(callee) == SyntaxKind::SuperKeyword {
            return Some(Type::Void);
        }
        if matches!(program.node_property(call, "typeArguments"), Some(NodePropertyValue::List { nodes, .. }) if !nodes.is_empty()) {
            return None;
        }
        let callee_type = self.located_type(self.type_of_expression(callee, program.node_kind(callee)));
        let Type::Function(function) = callee_type.peeled() else { return None };
        if function.type_parameter_head().is_some()
            || function.generic_shape().is_some()
            || function.overloads().is_some()
            || function.this_type().is_some()
        {
            return None;
        }
        let returns = function.return_type();
        (!returns.is_degraded() && !contains_type_parameter(returns)).then(|| returns.clone())
    }

    fn array_method_type(&self, access: NodeId) -> Option<Type> {
        let program = &self.program;
        let object = self.node_child(access, "expression")?;
        let name = program.node_value_text(self.node_child(access, "name")?)?.to_string();
        let object_type = self.located_type(self.type_of_expression(object, program.node_kind(object)));
        let (object_type, nullable) = self.chain_object_type(access, object_type);
        match object_type.peeled() {
            Type::Array(_) => {}
            // A tuple's `length` is its own literal, not `Array<T>`'s.
            Type::Tuple(_) if name != "length" => {}
            _ => return None,
        }
        let property = self.query(|query| query.property(&object_type, &name)).flatten()?;
        if property.ty.is_degraded() {
            return None;
        }
        Some(if nullable { surge_ts_types::union_type(vec![property.ty, Type::Undefined]) } else { property.ty })
    }

    /// A property access in a position tsc never narrows, typed as the
    /// property its object declares: a call's callee (a method read off its
    /// object) and the target of a plain assignment (`getFlowTypeOfAccessExpression`
    /// answers a definite assignment target with the declared type).
    fn unnarrowed_access_type(&self, access: NodeId) -> Option<Type> {
        let program = &self.program;
        let parent = program.node_parent(access)?;
        let unnarrowed = match program.node_kind(parent) {
            SyntaxKind::CallExpression => self.node_child(parent, "expression") == Some(access),
            SyntaxKind::BinaryExpression => {
                self.node_child(parent, "left") == Some(access)
                    && self
                        .node_child(parent, "operatorToken")
                        .is_some_and(|operator| program.node_kind(operator) == SyntaxKind::EqualsToken)
            }
            _ => false,
        };
        if !unnarrowed {
            return None;
        }
        let object = self.node_child(access, "expression")?;
        let name = program.node_value_text(self.node_child(access, "name")?)?.to_string();
        // The checker narrows a reference's properties into its object type,
        // where tsc keeps the declared property: an object a declared
        // non-union variable holds is read from that declared type.
        let declared_object = (program.node_kind(object) == SyntaxKind::Identifier)
            .then(|| self.symbol_at_location(object))
            .flatten()
            .filter(|symbol| self.symbol_flags(*symbol) & (ts_sf::Variable | ts_sf::Property) != 0)
            .map(|symbol| self.ty(self.type_of_symbol(symbol)))
            .filter(|declared| !matches!(declared.peeled(), Type::Union(_) | Type::ErrorType | Type::Unknown));
        let mut object_type =
            declared_object.unwrap_or_else(|| self.located_type(self.type_of_expression(object, program.node_kind(object))));
        if matches!(object_type, Type::ErrorType) && program.node_kind(object) == SyntaxKind::ThisKeyword {
            object_type = self.ty(self.this_receiver_type(object)?);
        }
        if matches!(object_type, Type::ErrorType | Type::Unknown) {
            return None;
        }
        // An optional chain reads the member off the object's non-nullable
        // part; `?.` itself makes the read possibly `undefined`.
        let (object_type, nullable) = self.chain_object_type(access, object_type);
        let Some(property) = self.query(|query| query.property(&object_type, &name)).flatten() else {
            // A member no declaration names is its index signature's, written
            // or not (`checkPropertyAccessExpression` reports a readonly one
            // and keeps its type).
            return self
                .query(|query| query.index_type(&object_type, surge_ts_types::is_numeric_key(&name)))
                .flatten()
                .filter(|ty| !matches!(ty, Type::ErrorType | Type::Unknown));
        };
        let ty = self.property_type(&property);
        if is_stand_in_signature(&ty) {
            return None;
        }
        Some(if nullable { surge_ts_types::union_type(vec![ty, Type::Undefined]) } else { ty })
    }

    /// The object type a property access in an optional chain reads from,
    /// and whether its own `?.` met a possibly nullish object.
    fn chain_object_type(&self, access: NodeId, object_type: Type) -> (Type, bool) {
        let program = &self.program;
        if program.node_flags(access) & super::enums_generated::node_flags::OptionalChain == 0 {
            return (object_type, false);
        }
        let nullish = |member: &Type| matches!(member, Type::Undefined | Type::Null | Type::Void);
        let stripped = match object_type.peeled() {
            Type::Union(union) if union.types().iter().any(nullish) => Some(surge_ts_types::union_type(
                union.types().iter().filter(|member| !nullish(member)).cloned().collect(),
            )),
            _ => None,
        };
        // `?.` strips the object's nullability, a later link the `undefined`
        // the chain carries; either way the chain stays possibly `undefined`.
        match stripped {
            Some(stripped) => (stripped, true),
            None => (object_type, false),
        }
    }

    /// An unannotated `const` initialized with a class, enum, namespace or
    /// expando value: its type is that value's, the same type.
    fn initializing_own_value(&self, symbol: SymbolId) -> Option<TypeId> {
        if self.symbol_flags(symbol) & ts_sf::BlockScopedVariable == 0 {
            return None;
        }
        let declaration = self.symbol_value_declaration(symbol)?;
        if self.program.node_kind(declaration) != SyntaxKind::VariableDeclaration
            || self.node_child(declaration, "type").is_some()
            || self.program.node_flags(self.program.node_parent(declaration)?) & super::enums_generated::node_flags::Const == 0
        {
            return None;
        }
        let mut initializer = self.node_child(declaration, "initializer")?;
        while self.program.node_kind(initializer) == SyntaxKind::ParenthesizedExpression {
            initializer = self.node_child(initializer, "expression")?;
        }
        // One level only: a `const` read from another is not followed, which
        // keeps `const a = b; const b = a;` from looping.
        let referenced = self.referenced_value_symbol(initializer, self.program.node_kind(initializer))?;
        let target = if self.is_alias(referenced) { self.aliased_symbol(referenced).unwrap_or(referenced) } else { referenced };
        self.is_typeof_named(target).then(|| self.type_of_symbol(referenced))
    }

    fn referenced_value_symbol(&self, node: NodeId, kind: SyntaxKind) -> Option<SymbolId> {
        if !matches!(kind, SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression | SyntaxKind::QualifiedName) {
            return None;
        }
        self.symbol_at_location(node)
    }

    /// A reference to a class, enum, namespace or expando value, whose type
    /// is its symbol's own however it is reached.
    fn own_value_type(&self, node: NodeId, kind: SyntaxKind) -> Option<TypeId> {
        let symbol = self.referenced_value_symbol(node, kind)?;
        let target = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
        if self.is_typeof_named(target) {
            return Some(self.type_of_symbol(symbol));
        }
        self.initializing_own_value(target)
    }

    /// The type checking the node's file checked it against: its
    /// contextual type.
    pub(super) fn recorded_contextual_type(&self, node: NodeId) -> Option<Type> {
        let checker_index = self.program.file(node.source_file()).checker_index?;
        let (start, end) = self.program.node_byte_span(node);
        self.query(|query| query.semantic_index(checker_index).contextual_type(start, end).cloned()).flatten()
    }

    /// What checking the node's file recorded for it.
    fn recorded_type(&self, node: NodeId, kind: SyntaxKind) -> Option<Type> {
        let recorded = self.recorded_type_unbridged(node, kind)?;
        // A reference to a function reads the type the checker narrowed it
        // to there, without the predicate its declaration infers.
        if kind == SyntaxKind::Identifier
            && let Type::Function(function) = &recorded
            && function.type_predicate().is_none()
            && !self.is_declaration_name(node)
            && let Some(symbol) = self.symbol_at_location(node)
            && self.symbol_flags(symbol) & (ts_sf::Function | ts_sf::Variable) != 0
            && let Type::Function(declared) = self.ty(self.type_of_symbol(symbol))
            && declared.type_predicate().is_some()
            && declared.parameters().len() == function.parameters().len()
        {
            return Some(Type::Function(declared));
        }
        Some(recorded)
    }

    fn recorded_type_unbridged(&self, node: NodeId, kind: SyntaxKind) -> Option<Type> {
        let checker_index = self.program.file(node.source_file()).checker_index?;
        let (start, end) = self.program.node_byte_span(node);
        // The checker keys a call's result by its callee without the
        // parentheses an immediately invoked function is written in.
        let callee = (kind == SyntaxKind::CallExpression)
            .then(|| self.node_child(node, "expression"))
            .flatten()
            .map(|mut callee| {
                while self.program.node_kind(callee) == SyntaxKind::ParenthesizedExpression
                    && let Some(inner) = self.node_child(callee, "expression")
                {
                    callee = inner;
                }
                self.program.node_byte_span(callee)
            });
        let element = (kind == SyntaxKind::ElementAccessExpression)
            .then(|| self.node_child(node, "argumentExpression"))
            .flatten()
            .map(|argument| (start, self.program.node_byte_span(argument).1));
        let declaration_name = self
            .program
            .node_parent(node)
            .filter(|parent| {
                self.program.node_property(*parent, "name") == Some(super::node::NodePropertyValue::Node(node))
            })
            .is_some();
        self.query(|query| {
            let index = query.semantic_index(checker_index);
            if declaration_name && let Some(ty) = index.declaration_type(start, end) {
                return Some(ty.clone());
            }
            if let Some(ty) = index.expression_type(start, end) {
                return Some(ty.clone());
            }
            if let Some((callee_start, callee_end)) = callee
                && let Some(ty) = index.call_result_type(callee_start, callee_end)
            {
                return Some(ty.clone());
            }
            if let Some((object_start, argument_end)) = element
                && let Some(ty) = index.element_access_type(object_start, argument_end)
            {
                return Some(ty.clone());
            }
            index.declaration_type(start, end).cloned()
        })
        .flatten()
    }

    /// An `extends` clause entry of a class declaration or expression.
    fn is_class_extends_heritage(&self, node: NodeId) -> bool {
        let Some(clause) = self.program.node_parent(node) else { return false };
        self.program.node_operator(clause) == Some(SyntaxKind::ExtendsKeyword)
            && self.program.node_parent(clause).is_some_and(|owner| {
                matches!(self.program.node_kind(owner), SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression)
            })
    }

    /// A type node's type: its source text resolved by the checker where it
    /// is written, with the type parameters in scope there standing for
    /// themselves.
    fn type_of_type_node(&self, node: NodeId) -> Option<Type> {
        let kind = self.program.node_kind(node);
        if let Some(keyword) = keyword_type(kind) {
            return Some(keyword);
        }
        // `infer U` is the type parameter it declares.
        if kind == SyntaxKind::InferType
            && let Some(name) = self.node_child(node, "typeParameter").and_then(|parameter| self.node_child(parameter, "name"))
        {
            return self.type_of_type_node(name);
        }
        let checker_index = self.program.file(node.source_file()).checker_index?;
        let text = self.program.node_text(node).to_string();
        let type_parameters = self.type_parameters_in_scope(node);
        let namespaces = self.namespace_scope(node);
        let resolved = self.query(|query| query.resolve_type_text(checker_index, &text, &type_parameters, &namespaces)).flatten();
        if resolved.as_ref().is_none_or(|ty| matches!(ty, Type::Any | Type::Unknown | Type::ErrorType))
            && let Some(deferred) = self.deferred_type_of_node(node)
        {
            return Some(deferred);
        }
        resolved
    }

    // --- symbols -------------------------------------------------------------

    /// `getTypeOfSymbol` / `getTypeOfSymbolAtLocation`: the type a value
    /// symbol declares; at a reference, what the checker narrowed it to there.
    pub fn type_of_symbol_at_location(&self, symbol: SymbolId, location: NodeId) -> TypeId {
        let ty = self.type_of_symbol_at_location_unexpanded(symbol, location);
        self.expanded_self_reference(location, ty).unwrap_or(ty)
    }

    fn type_of_symbol_at_location_unexpanded(&self, symbol: SymbolId, location: NodeId) -> TypeId {
        let symbol = self.export_symbol_of_value(symbol);
        let kind = self.program.node_kind(location);
        // A body-local enum's member is its fresh enum literal type wherever
        // it is read.
        if self.symbol_flags(symbol) & ts_sf::EnumMember != 0
            && let Some(&member) = self.symbol_declarations(symbol).first()
            && self.local_enum_member_type(member, false).is_some()
        {
            return self.type_of_symbol(symbol);
        }
        // At an intrinsic tag, a member of `IntrinsicElements` has its
        // declared type; the index signature or the interface no value type.
        if kind == SyntaxKind::Identifier && self.is_intrinsic_tag_name(location) {
            return match self.symbol_flags(symbol) & (ts_sf::Property | ts_sf::Method) {
                0 => self.error_type(),
                _ => self.type_of_symbol(symbol),
            };
        }
        let access = match kind {
            SyntaxKind::Identifier => self.program.node_parent(location).filter(|parent| {
                self.program.node_kind(*parent) == SyntaxKind::PropertyAccessExpression
                    && self.node_child(*parent, "name") == Some(location)
            }),
            SyntaxKind::PropertyAccessExpression => Some(location),
            _ => None,
        };
        // At the access itself tsc answers the member's declared type; at its
        // name, the read there, narrowed (`t.length` after `t.length === 1`).
        if let Some(access) = access
            && !self.is_assignment_target(access)
            && let Some(declared) = self.fixed_tuple_length(access)
        {
            return match kind {
                SyntaxKind::PropertyAccessExpression => self.type_id(None, Some(symbol), || declared),
                _ => self.type_at_location(access),
            };
        }
        if matches!(kind, SyntaxKind::Identifier | SyntaxKind::PrivateIdentifier)
            && self.symbol_at_location(location) == Some(symbol)
            && !self.is_declaration_name(location)
        {
            let reference = match self.program.node_parent(location) {
                Some(parent)
                    if (self.program.node_kind(parent) == SyntaxKind::PropertyAccessExpression
                        && self.node_child(parent, "name") == Some(location))
                        || (self.program.node_kind(parent) == SyntaxKind::QualifiedName
                            && self.node_child(parent, "right") == Some(location)) =>
                {
                    parent
                }
                _ => location,
            };
            let reference_kind = self.program.node_kind(reference);
            if self.is_assignment_target(reference) && self.is_invalid_write_target(reference, reference_kind) {
                return self.error_type();
            }
            if reference_kind == SyntaxKind::Identifier
                && self.is_definite_assignment_target(reference)
                && self.symbol_flags(symbol) & ts_sf::Variable != 0
            {
                return self.type_of_symbol(symbol);
            }
            if let Some(ty) = self.optional_chain_link_type(reference, symbol) {
                return self.type_id(None, Some(symbol), || ty);
            }
            if let Some(own) = self.own_value_type(reference, reference_kind) {
                return own;
            }
            if let Some(ty) = self.recorded_type(reference, reference_kind) {
                if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown)
                    && let Some(declared) = self
                        .named_error_type_of(symbol)
                        .or_else(|| self.constructor_typed_read(symbol))
                        .or_else(|| self.initialized_constant_read(symbol))
                        .or_else(|| {
                            self.annotated_type_of(symbol).filter(|ty| {
                                !self.keeps_error_type(reference)
                                    && (reference_kind != SyntaxKind::PropertyAccessExpression
                                        || !surge_ts_types::mentions_type_variable(&self.ty(*ty), 6))
                            })
                        })
                {
                    return declared;
                }
                let ty = match reference_kind {
                    SyntaxKind::PropertyAccessExpression => self.bind_this_return(reference, ty.clone()).unwrap_or(ty),
                    _ => ty,
                };
                return self.type_id(Some(TypeKey::Location(reference)), Some(symbol), || ty);
            }
        }
        // An index signature's symbol has no type of its own; at the name it
        // is read by, it is what the signature declares.
        if kind == SyntaxKind::Identifier
            && self.symbol_flags(symbol) & ts_sf::Signature != 0
            && self.symbol_name(symbol) == "__index"
            && let Some(declaration) = self.symbol_declarations(symbol).first().copied()
            && let Some(annotation) = self.node_child(declaration, "type")
            && let Some(ty) = self.type_of_type_node(annotation)
        {
            return self.type_id(None, Some(symbol), || ty);
        }
        let ty = self.type_of_symbol(symbol);
        // A method read off an object returns that object for `this`.
        let access = match kind {
            SyntaxKind::PropertyAccessExpression => Some(location),
            SyntaxKind::Identifier | SyntaxKind::PrivateIdentifier => self.program.node_parent(location).filter(|parent| {
                self.program.node_kind(*parent) == SyntaxKind::PropertyAccessExpression
                    && self.node_child(*parent, "name") == Some(location)
            }),
            _ => None,
        };
        if let Some(access) = access
            && let Some(bound) = self.bind_this_return(access, self.ty(ty))
        {
            return self.type_id(Some(TypeKey::Location(access)), Some(symbol), || bound);
        }
        ty
    }

    /// A fixed tuple's `length` is a member of its own tuple target, typed by
    /// the element flags (`1 | 2`), where surge resolves the name to
    /// `Array<T>`'s `number`.
    fn fixed_tuple_length(&self, access: NodeId) -> Option<Type> {
        if self.node_child(access, "name").and_then(|name| self.program.node_value_text(name)) != Some("length") {
            return None;
        }
        let object = self.node_child(access, "expression")?;
        let object_type = self.located_type(self.type_of_expression(object, self.program.node_kind(object)));
        if !matches!(object_type.peeled(), Type::Tuple(_)) {
            return None;
        }
        object_type.get_property_access_type("length")
    }

    /// `getExportSymbolOfValueSymbolIfExported`: an exported declaration's
    /// local symbol, which carries only `ExportValue`, reads as its export.
    pub(super) fn export_symbol_of_value(&self, symbol: SymbolId) -> SymbolId {
        let SymbolData::Bound { file, symbol: index } = self.symbol_data(symbol) else { return symbol };
        let syntax = self.program.syntax(self.program.file_id(file));
        let bound = &syntax.bound.symbols[index];
        match bound.export_symbol {
            Some(export) if bound.flags & sf::ExportValue != 0 => self.bound_symbol(file, export),
            _ => symbol,
        }
    }

    fn is_declaration_name(&self, node: NodeId) -> bool {
        self.program.node_parent(node).is_some_and(|parent| {
            declaration_kinds().contains(&self.program.node_kind(parent))
                && self.node_child(parent, "name") == Some(node)
        })
    }

    /// `getTypeOfSymbol`.
    pub fn type_of_symbol(&self, symbol: SymbolId) -> TypeId {
        let key = TypeKey::SymbolType(symbol);
        if let Some(index) = self.with_tables(|tables| tables.type_by_key.get(&key).copied()) {
            return TypeId { program: symbol.program, index };
        }
        // `getTypeOfSymbol` reads a symbol's own value meaning before its
        // alias one: a local `const a` merged with `export { a } from` (an
        // error tsc reports) is still the variable.
        let own_value = ts_sf::Variable
            | ts_sf::Property
            | ts_sf::Function
            | ts_sf::Method
            | ts_sf::Class
            | ts_sf::Enum
            | ts_sf::ValueModule
            | ts_sf::EnumMember
            | ts_sf::Accessor;
        if self.is_alias(symbol) && self.symbol_flags(symbol) & own_value == 0 {
            // An alias has the very type of what it aliases (`getTypeOfAlias`).
            let target = self.aliased_symbol(symbol).filter(|target| *target != symbol);
            // A value declared inside an ambient module block is out of this
            // API's reach: the importing file bound its type.
            if let Some(target) = target
                && self.symbol_flags(target) & ts_sf::Value != 0
                && self.is_in_ambient_module_block(target)
                && let Some(bound) = self.imported_value_type(symbol)
            {
                self.with_tables(|tables| tables.type_by_key.insert(key, bound.index));
                return bound;
            }
            let ty = match target {
                Some(target) => self.type_of_symbol(target),
                None => self.error_type(),
            };
            // A value target this API cannot type or reach (declared inside an
            // ambient module) has the type the importing file bound; a
            // type-only target has none.
            let ty = match self.ty(ty) {
                Type::ErrorType if target.is_none_or(|target| self.symbol_flags(target) & ts_sf::Value != 0) => {
                    self.imported_value_type(symbol).unwrap_or(ty)
                }
                // A resolved target this API cannot type (declared inside an
                // ambient module) has the type the importing file bound.
                Type::Any | Type::ErrorType if target.is_some_and(|target| self.symbol_flags(target) & ts_sf::Value != 0) => {
                    self.imported_value_type(symbol).unwrap_or(ty)
                }
                _ => ty,
            };
            // A module this API cannot type is named by its module, as tsc
            // prints a module's namespace object (`typeof import("m")`); tsc
            // leaves one an ambient module block imports unresolved.
            let ty = match target {
                Some(target)
                    if self.is_module_symbol(target)
                        && matches!(self.ty(ty), Type::Any | Type::ErrorType)
                        && !self.is_in_ambient_module_block(symbol) =>
                {
                    self.type_id(None, Some(target), || Type::Any)
                }
                // Any other namespace object a module alias reads (`import *
                // as ns`, `export * as ns`) is named by its module too; one
                // with members the module does not export (a synthetic
                // `default`) is not that module's.
                Some(target)
                    if self.is_module_symbol(target)
                        && self.origin(ty) != Some(target)
                        && !self.is_in_ambient_module_block(symbol)
                        && self.is_namespace_object_of(target, &self.ty(ty)) =>
                {
                    let namespace = self.ty(ty);
                    self.type_id(None, Some(target), || namespace)
                }
                _ => ty,
            };
            let ty = match (self.ty(ty), target) {
                (Type::Any | Type::ErrorType | Type::Unknown, Some(target)) => self.annotated_type_of(target).unwrap_or(ty),
                _ => ty,
            };
            self.with_tables(|tables| tables.type_by_key.insert(key, ty.index));
            return ty;
        }
        if let Some(own) = self.initializing_own_value(symbol) {
            self.with_tables(|tables| tables.type_by_key.insert(key, own.index));
            return own;
        }
        let mut ty = self.compute_type_of_symbol(symbol).unwrap_or(Type::ErrorType);
        // A variable an abstract class's `new` initializes is that error.
        if let Some(declaration) = self.symbol_value_declaration(symbol)
            && self.program.node_kind(declaration) == SyntaxKind::VariableDeclaration
            && self.node_child(declaration, "type").is_none()
            && let Some(initializer) = self.node_child(declaration, "initializer")
            && self.program.node_kind(initializer) == SyntaxKind::NewExpression
            && self.instantiates_abstract_class(initializer)
        {
            ty = Type::ErrorType;
        }
        // `addOptionality`: an optional method's type includes `undefined`.
        if self.symbol_flags(symbol) & (ts_sf::Method | ts_sf::Optional) == ts_sf::Method | ts_sf::Optional
            && self.program.inner().strict_null_checks
            && matches!(ty, Type::Function(_))
        {
            ty = surge_ts_types::union_type(vec![ty, Type::Undefined]);
        }
        if matches!(ty, Type::ErrorType | Type::Any)
            && let Some(initialized) = self.unannotated_initializer_type(symbol, matches!(ty, Type::Any))
        {
            ty = initialized;
        }
        if matches!(ty, Type::ErrorType | Type::Any | Type::Unknown)
            && let Some(error) = self.annotated_unresolved_type(symbol)
        {
            self.with_tables(|tables| tables.type_by_key.insert(key, error.index));
            return error;
        }
        self.type_id(Some(key), Some(symbol), || ty)
    }

    /// An unannotated variable the checker typed `any` (a failed assignment
    /// of an object or array literal) or never checked (a file whose parse
    /// it gave up on): its initializer's type, a `let`'s fresh literal widened
    /// (`getWidenedTypeForVariableLikeDeclaration`).
    fn unannotated_initializer_type(&self, symbol: SymbolId, literals_only: bool) -> Option<Type> {
        let declaration = self.symbol_value_declaration(symbol)?;
        let program = &self.program;
        if program.node_kind(declaration) != SyntaxKind::VariableDeclaration
            || self.node_child(declaration, "type").is_some()
            || self.node_child(declaration, "name").is_none_or(|name| program.node_kind(name) != SyntaxKind::Identifier)
        {
            return None;
        }
        // tsc checks nothing in a `with` statement's body.
        let mut ancestor = program.node_parent(declaration);
        while let Some(node) = ancestor {
            if program.node_kind(node) == SyntaxKind::WithStatement {
                return None;
            }
            ancestor = program.node_parent(node);
        }
        let initializer = self.node_child(declaration, "initializer")?;
        let kind = program.node_kind(initializer);
        let literal = matches!(kind, SyntaxKind::ObjectLiteralExpression | SyntaxKind::ArrayLiteralExpression);
        if literals_only && !literal {
            return None;
        }
        let ty = self.located_type(self.type_of_expression(initializer, kind));
        if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown) || ty.is_degraded() {
            return None;
        }
        let is_const = program
            .node_parent(declaration)
            .is_some_and(|list| program.node_flags(list) & super::enums_generated::node_flags::Const != 0);
        if is_const || !self.is_fresh_literal_expression(initializer) {
            return Some(ty);
        }
        self.query(|query| query.widened_literal(&ty))
    }

    /// The error type of a declaration annotated with a reference nothing
    /// declares, which tsc prints as written (`let x: Missing`). An optional
    /// one's `| undefined` leaves plain `any`.
    fn annotated_unresolved_type(&self, symbol: SymbolId) -> Option<TypeId> {
        let declaration = *self.symbol_declarations(symbol).first()?;
        if !matches!(
            self.program.node_kind(declaration),
            SyntaxKind::VariableDeclaration | SyntaxKind::Parameter | SyntaxKind::PropertyDeclaration | SyntaxKind::PropertySignature
        ) || self.node_child(declaration, "questionToken").is_some()
        {
            return None;
        }
        let annotation = self.node_child(declaration, "type")?;
        if let Some(error) = self.unresolved_type_node(annotation) {
            return Some(error);
        }
        // An alias of such a reference prints as the reference.
        let annotation = self.unwrap_type_node(annotation);
        if self.program.node_kind(annotation) != SyntaxKind::TypeReference {
            return None;
        }
        let alias = self.symbol_at_location(self.node_child(annotation, "typeName")?)?;
        let declared = self.declared_type_of_symbol(alias);
        self.with_tables(|tables| tables.error_displays.contains_key(&declared.index)).then_some(declared)
    }

    /// tsc's error type for a type reference whose name resolves to nothing,
    /// named by the reference with its type arguments
    /// (`getTypeFromTypeAliasReference` of an unresolved symbol).
    fn unresolved_reference_type(&self, reference: NodeId) -> Option<TypeId> {
        let program = &self.program;
        let reference = self.unwrap_type_node(reference);
        if program.node_kind(reference) != SyntaxKind::TypeReference {
            return None;
        }
        let name = self.node_child(reference, "typeName")?;
        if !self.reference_names_nothing(reference, name) {
            return None;
        }
        let mut display: String = program.node_text(name).split_whitespace().collect();
        if let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(reference, "typeArguments")
            && !nodes.is_empty()
        {
            let arguments: Vec<String> = nodes.iter().map(|argument| self.type_to_string(self.type_at_location(*argument))).collect();
            display = format!("{display}<{}>", arguments.join(", "));
        }
        let ty = self.type_id(Some(TypeKey::UnresolvedReference(display.clone())), None, || Type::ErrorType);
        self.with_tables(|tables| tables.error_displays.insert(ty.index, display));
        Some(ty)
    }

    /// What tsc still types an expression the checker gave up on (`any` where
    /// it reported an error): an operator's result, a conditional's two
    /// branches, a template's `string`.
    fn recovered_expression_type(&self, node: NodeId, kind: SyntaxKind) -> Option<Type> {
        let known = |ty: Type| (!matches!(ty, Type::Any | Type::ErrorType | Type::Unknown)).then_some(ty);
        let child = |property: &str| {
            let child = self.node_child(node, property)?;
            known(self.ty(self.type_at_location(child)))
        };
        match kind {
            SyntaxKind::PrefixUnaryExpression
            | SyntaxKind::PostfixUnaryExpression
            | SyntaxKind::TypeOfExpression
            | SyntaxKind::VoidExpression
            | SyntaxKind::DeleteExpression
            | SyntaxKind::BinaryExpression => known(self.operator_type(node, kind)?),
            SyntaxKind::ConditionalExpression => {
                Some(surge_ts_types::union_type(vec![child("whenTrue")?, child("whenFalse")?]))
            }
            SyntaxKind::TemplateExpression => Some(Type::String),
            SyntaxKind::BinaryExpression if self.is_plain_assignment(node) => child("right"),
            // `import()` without a specifier still promises a module.
            SyntaxKind::CallExpression
                if self.node_child(node, "expression").is_some_and(|callee| self.program.node_kind(callee) == SyntaxKind::ImportKeyword)
                    && self.list_property(node, "arguments").is_empty() =>
            {
                let checker_index = self.program.file(node.source_file()).checker_index?;
                self.query(|query| query.resolve_type_text(checker_index, "Promise<any>", &[], &[])).flatten()
            }
            _ => None,
        }
    }

    /// tsc's type for a class property declared with neither a type nor an
    /// initializer (`getFlowTypeInConstructor`): the widened union of what the
    /// constructor assigns it (`this.x = …`, `this["x"] = …`), or a static
    /// block for a static one.
    fn constructor_assigned_type(&self, property: NodeId) -> Option<Type> {
        let program = &self.program;
        if self.node_child(property, "type").is_some() || self.node_child(property, "initializer").is_some() {
            return None;
        }
        let name = program.node_value_text(self.node_child(property, "name")?)?.to_string();
        let is_static = matches!(program.node_property(property, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
            if nodes.iter().any(|modifier| program.node_kind(*modifier) == SyntaxKind::StaticKeyword));
        let class = program.node_parent(property)?;
        let Some(NodePropertyValue::List { nodes: members, .. }) = program.node_property(class, "members") else { return None };
        let containers = members.into_iter().filter(|member| match program.node_kind(*member) {
            SyntaxKind::Constructor => !is_static,
            SyntaxKind::ClassStaticBlockDeclaration => is_static,
            _ => false,
        });
        let mut assigned = Vec::new();
        for container in containers {
            let Some(body) = self.node_child(container, "body") else { continue };
            for statement in self.list_property(body, "statements") {
                if program.node_kind(statement) != SyntaxKind::ExpressionStatement {
                    continue;
                }
                let Some(expression) = self.node_child(statement, "expression") else { continue };
                if program.node_kind(expression) != SyntaxKind::BinaryExpression || !self.is_plain_assignment(expression) {
                    continue;
                }
                let Some(target) = self.node_child(expression, "left") else { continue };
                let names_property = match program.node_kind(target) {
                    SyntaxKind::PropertyAccessExpression => self
                        .node_child(target, "name")
                        .and_then(|member| program.node_value_text(member))
                        .is_some_and(|member| member == name),
                    SyntaxKind::ElementAccessExpression => self
                        .node_child(target, "argumentExpression")
                        .and_then(|argument| program.node_value_text(argument))
                        .is_some_and(|member| member == name),
                    _ => false,
                };
                let on_this = self
                    .node_child(target, "expression")
                    .is_some_and(|object| program.node_kind(object) == SyntaxKind::ThisKeyword);
                if !names_property || !on_this {
                    continue;
                }
                let Some(right) = self.node_child(expression, "right") else { continue };
                let ty = self.located_type(self.type_of_expression(right, program.node_kind(right)));
                if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown) {
                    return None;
                }
                assigned.push(self.query(|query| query.widened_literal(&ty)).unwrap_or(ty));
            }
        }
        (!assigned.is_empty()).then(|| surge_ts_types::union_type(assigned))
    }

    /// The type a declaration states (`const limit: number`, a property or
    /// parameter annotation) where the checker bound the value as `any`
    /// (inside an ambient module block); tsc never narrows a stated
    /// non-`any` type to `any`.
    fn annotated_type_of(&self, symbol: SymbolId) -> Option<TypeId> {
        let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol)? } else { symbol };
        // Every import of a module also declared without a body is `any`.
        if let Some(module) = self.symbol_parent(symbol)
            && self.symbol_declarations(module).iter().any(|declaration| {
                self.program.node_kind(*declaration) == SyntaxKind::ModuleDeclaration && self.node_child(*declaration, "body").is_none()
            })
        {
            return None;
        }
        let declaration = *self.symbol_declarations(symbol).first()?;
        if !matches!(
            self.program.node_kind(declaration),
            SyntaxKind::VariableDeclaration | SyntaxKind::PropertyDeclaration | SyntaxKind::PropertySignature | SyntaxKind::Parameter
        ) || self.node_child(declaration, "type").is_none()
            // An annotated catch variable is an error tsc types as `any`.
            || self.program.node_parent(declaration).is_some_and(|parent| self.program.node_kind(parent) == SyntaxKind::CatchClause)
        {
            return None;
        }
        let written = self.written_type_of_symbol(symbol)?;
        (!matches!(written, Type::Any | Type::ErrorType | Type::Unknown)).then(|| self.type_id(None, Some(symbol), || written))
    }

    /// An unannotated variable's type from its initializer
    /// (`getWidenedTypeForVariableLikeDeclaration`), where the checker gave
    /// up on the initializer it reported an error in.
    fn initialized_variable_type(&self, declaration: NodeId) -> Option<Type> {
        thread_local! {
            static INITIALIZING: std::cell::RefCell<Vec<NodeId>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        if self.node_child(declaration, "type").is_some()
            || INITIALIZING.with(|initializing| initializing.borrow().contains(&declaration))
        {
            return None;
        }
        // A JavaScript variable may be typed by its JSDoc.
        let script_kind = self.program.script_kind(declaration.source_file());
        if script_kind == super::enums_generated::script_kind::JS || script_kind == super::enums_generated::script_kind::JSX {
            return None;
        }
        let initializer = self.node_child(declaration, "initializer")?;
        INITIALIZING.with(|initializing| initializing.borrow_mut().push(declaration));
        let ty = self.ty(self.type_at_location(initializer));
        INITIALIZING.with(|initializing| initializing.borrow_mut().retain(|current| *current != declaration));
        match &ty {
            Type::Any | Type::ErrorType | Type::Unknown | Type::Null | Type::Undefined | Type::Void => return None,
            Type::Array(element) if matches!(**element, Type::Never) => return None,
            _ if ty.is_degraded() => return None,
            _ => {}
        }
        let constant = self
            .program
            .node_parent(declaration)
            .is_some_and(|list| self.program.node_flags(list) & super::enums_generated::node_flags::Const != 0);
        if constant {
            return Some(ty);
        }
        self.query(|query| query.widened_literal(&ty))
    }

    /// A read of a constant typed by its initializer where the checker gave up
    /// on it (see [`Self::initialized_variable_type`]): what the constant
    /// holds, which no flow changes.
    fn initialized_constant_read(&self, symbol: SymbolId) -> Option<TypeId> {
        let declaration = self.symbol_declarations(symbol).into_iter().next()?;
        if self.program.node_kind(declaration) != SyntaxKind::VariableDeclaration
            || !self
                .program
                .node_parent(declaration)
                .is_some_and(|list| self.program.node_flags(list) & super::enums_generated::node_flags::Const != 0)
        {
            return None;
        }
        self.initialized_variable_type(declaration)?;
        let declared = self.type_of_symbol(symbol);
        (!matches!(self.ty(declared), Type::Any | Type::ErrorType | Type::Unknown)).then_some(declared)
    }

    /// Where the checker's `any` is tsc's too: a write to what cannot be
    /// written, an intrinsic tag, a name in a type (a predicate's parameter, a
    /// `typeof` query).
    fn keeps_error_type(&self, node: NodeId) -> bool {
        let program = &self.program;
        let kind = program.node_kind(node);
        if kind == SyntaxKind::ParenthesizedExpression {
            return self.node_child(node, "expression").is_some_and(|inner| self.keeps_error_type(inner));
        }
        let Some(parent) = program.node_parent(node) else { return false };
        if kind == SyntaxKind::Identifier
            && matches!(program.node_kind(parent), SyntaxKind::PropertyAccessExpression | SyntaxKind::QualifiedName)
            && self.node_child(parent, "name").or_else(|| self.node_child(parent, "right")) == Some(node)
        {
            return self.keeps_error_type(parent);
        }
        if matches!(kind, SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression | SyntaxKind::ElementAccessExpression)
            && self.is_assignment_target(node)
            && self.is_invalid_write_target(node, kind)
        {
            return true;
        }
        if kind == SyntaxKind::Identifier && self.is_intrinsic_tag_name(node) {
            return true;
        }
        let mut ancestor = Some(node);
        while let Some(current) = ancestor {
            match program.node_kind(current) {
                SyntaxKind::TypePredicate | SyntaxKind::TypeQuery => return true,
                SyntaxKind::Identifier | SyntaxKind::QualifiedName => ancestor = program.node_parent(current),
                _ => break,
            }
        }
        false
    }

    /// A read of a class property the constructor types (see
    /// [`Self::constructor_assigned_type`]), where the checker recorded `any`.
    fn constructor_typed_read(&self, symbol: SymbolId) -> Option<TypeId> {
        let declaration = self.symbol_value_declaration(symbol)?;
        if self.program.node_kind(declaration) != SyntaxKind::PropertyDeclaration {
            return None;
        }
        self.constructor_assigned_type(declaration)?;
        Some(self.type_of_symbol(symbol))
    }

    /// The named error type an expression has where the checker answered
    /// `any`: an assertion to a type over an unresolved reference, an
    /// assignment of one, or a read of a binding typed by one.
    fn unresolved_expression_type(&self, node: NodeId) -> Option<TypeId> {
        let program = &self.program;
        if let Some(parent) = program.node_parent(node)
            && program.node_kind(parent) == SyntaxKind::PropertyAccessExpression
            && self.node_child(parent, "name") == Some(node)
        {
            return self.unresolved_expression_type(parent);
        }
        match program.node_kind(node) {
            SyntaxKind::AsExpression | SyntaxKind::TypeAssertionExpression => {
                self.unresolved_type_node(self.node_child(node, "type")?)
            }
            SyntaxKind::ParenthesizedExpression => self.unresolved_expression_type(self.node_child(node, "expression")?),
            SyntaxKind::BinaryExpression if self.is_plain_assignment(node) => {
                self.unresolved_expression_type(self.node_child(node, "right")?)
            }
            SyntaxKind::Identifier if program.is_expression_node(node) => {
                let symbol = self.symbol_at_location(node)?;
                self.named_error_type_of(symbol)
                    .or_else(|| self.constructor_typed_read(symbol))
                    .or_else(|| self.initialized_constant_read(symbol))
                    .or_else(|| self.annotated_type_of(symbol))
            }
            // A member's written type holds its owner's type parameters,
            // which only the owner's instantiation could replace.
            SyntaxKind::PropertyAccessExpression => {
                let symbol = self.symbol_at_location(node)?;
                self.constructor_typed_read(symbol).or_else(|| {
                    self.annotated_type_of(symbol)
                        .filter(|ty| !surge_ts_types::mentions_type_variable(&self.ty(*ty), 6))
                })
            }
            SyntaxKind::ElementAccessExpression => [self.node_child(node, "argumentExpression"), Some(node)]
                .into_iter()
                .flatten()
                .filter_map(|part| self.symbol_at_location(part))
                .find_map(|symbol| self.constructor_typed_read(symbol)),
            _ => None,
        }
    }

    /// `symbol`'s type where it is an error type named by an unresolved
    /// reference; not while that type is being computed (a binding read in
    /// its own initializer).
    fn named_error_type_of(&self, symbol: SymbolId) -> Option<TypeId> {
        thread_local! {
            static CONSULTING: std::cell::RefCell<Vec<SymbolId>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        if CONSULTING.with(|consulting| consulting.borrow().contains(&symbol)) {
            return None;
        }
        CONSULTING.with(|consulting| consulting.borrow_mut().push(symbol));
        let declared = self.type_of_symbol(symbol);
        CONSULTING.with(|consulting| consulting.borrow_mut().retain(|consulted| *consulted != symbol));
        self.with_tables(|tables| tables.error_displays.contains_key(&declared.index)).then_some(declared)
    }

    /// Whether a type reference's name resolves to nothing for tsc: the
    /// placeholder for a name nothing declares, an import of what a module
    /// does not export or of a module that does not resolve, or, where the
    /// checker resolved nothing either, a member no namespace declares
    /// (`N.Missing`).
    fn reference_names_nothing(&self, reference: NodeId, name: NodeId) -> bool {
        match self.symbol_at_location(name) {
            Some(symbol) if self.is_unresolved_symbol(symbol) => true,
            Some(symbol) if self.is_alias(symbol) => self.aliased_symbol(symbol).is_none(),
            Some(_) => false,
            None => self
                .type_of_type_node(reference)
                .is_none_or(|ty| matches!(ty, Type::ErrorType | Type::Any | Type::Unknown)),
        }
    }

    /// The error type tsc gives a type node over a reference that resolves to
    /// nothing, named by that reference: the reference itself, an alias of
    /// one, or an indexed access on one (`getIndexedAccessType` answers its
    /// `any` operand, the object's first).
    fn unresolved_type_node(&self, node: NodeId) -> Option<TypeId> {
        let node = self.unwrap_type_node(node);
        match self.program.node_kind(node) {
            SyntaxKind::TypeReference => self.unresolved_reference_type(node).or_else(|| {
                let alias = self.symbol_at_location(self.node_child(node, "typeName")?)?;
                let alias = if self.is_alias(alias) { self.aliased_symbol(alias)? } else { alias };
                if self.symbol_flags(alias) & ts_sf::TypeAlias == 0 || self.is_unresolved_symbol(alias) {
                    return None;
                }
                let declared = self.declared_type_of_symbol(alias);
                self.with_tables(|tables| tables.error_displays.contains_key(&declared.index)).then_some(declared)
            }),
            SyntaxKind::IndexedAccessType => self
                .node_child(node, "objectType")
                .and_then(|object| self.unresolved_type_node(object))
                .or_else(|| self.node_child(node, "indexType").and_then(|index| self.unresolved_type_node(index))),
            _ => None,
        }
    }

    /// A placeholder symbol of [`Self::unresolved_type_reference_symbol`]:
    /// the only transient type alias.
    fn is_unresolved_symbol(&self, symbol: SymbolId) -> bool {
        matches!(self.symbol_data(symbol), SymbolData::Transient { flags, .. } if flags == ts_sf::TypeAlias)
    }

    /// The type an import (or export) binding has in the file that declares
    /// it.
    fn imported_value_type(&self, alias: SymbolId) -> Option<TypeId> {
        let declaration = *self.symbol_declarations(alias).first()?;
        if !matches!(
            self.program.node_kind(declaration),
            SyntaxKind::ImportSpecifier
                | SyntaxKind::ImportClause
                | SyntaxKind::NamespaceImport
                | SyntaxKind::ImportEqualsDeclaration
                | SyntaxKind::ExportSpecifier
        ) {
            return None;
        }
        let checker_index = self.program.file(declaration.source_file()).checker_index?;
        let name = self.symbol_name(alias);
        let ty = self.query(|query| query.file_value_type(checker_index, &name)).flatten()?;
        (!matches!(ty, Type::Unknown | Type::ErrorType)).then(|| self.intern(ty))
    }

    fn is_alias(&self, symbol: SymbolId) -> bool {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].flags & sf::Alias != 0
            }
            _ => false,
        }
    }

    fn compute_type_of_symbol(&self, symbol: SymbolId) -> Option<Type> {
        if let Some(unique) = self.declared_unique_symbol(symbol) {
            return Some(unique);
        }
        let data = self.symbol_data(symbol);
        let (file, index) = match &data {
            SymbolData::Transient { ty, .. } => return Some(self.ty(*ty)),
            SymbolData::Global { name, .. } => return self.query(|query| query.global_value_type(name)).flatten(),
            SymbolData::Bound { file, symbol } => (*file, *symbol),
        };
        let file_id = self.program.file_id(file);
        let syntax = self.program.syntax(file_id).clone();
        let bound = &syntax.bound.symbols[index];
        if bound.flags & (sf::Value | sf::ValueModule) == 0 {
            return None;
        }
        let declaration = bound.value_declaration.or_else(|| bound.declarations.first().copied())?;
        let declaration_id = NodeId { program: symbol.program, file, node: declaration };
        if syntax.tree.kind(declaration) == Kind::SourceFile {
            let checker_index = self.program.file(file_id).checker_index?;
            return self.query(|query| query.module_namespace_type(checker_index)).flatten();
        }
        let declaration_kind = syntax.tree.kind(declaration);
        if declaration_kind == Kind::EnumMember
            && let Some(ty) = self.enum_member_type(declaration_id)
        {
            return Some(ty);
        }
        // An expando member (`f.p = v`) has the widened type of what its
        // assignments assign (`getWidenedTypeForAssignmentDeclaration`).
        if declaration_kind == Kind::PropertyAccessExpression {
            let mut assigned = Vec::new();
            for declaration in &bound.declarations {
                let assignment = syntax.tree.parent(*declaration).filter(|parent| syntax.tree.kind(*parent) == Kind::BinaryExpression);
                let right = assignment.and_then(|assignment| self.node_child(NodeId { node: assignment, ..declaration_id }, "right"));
                let Some(right) = right else { continue };
                let ty = self.ty(self.type_at_location(right));
                assigned.push(self.query(|query| query.widened_literal(&ty)).unwrap_or(ty));
            }
            if !assigned.is_empty() {
                return Some(surge_ts_types::union_type(assigned));
            }
        }
        // A function or class expression's symbol has the expression's type.
        if matches!(declaration_kind, Kind::ArrowFunction | Kind::FunctionExpression | Kind::ClassExpression)
            && let Some(ty) = self.recorded_type(declaration_id, self.program.node_kind(declaration_id))
        {
            return Some(ty);
        }
        if declaration_kind == Kind::PropertyDeclaration
            && let Some(assigned) = self.constructor_assigned_type(declaration_id)
        {
            return Some(assigned);
        }
        if let Some(name) = syntax.tree.node(declaration).name {
            let name_id = NodeId { node: name, ..declaration_id };
            // The checker's stand-in for a signature it could not model is
            // no answer; the written signature is.
            let computed = syntax.tree.kind(name) == Kind::ComputedPropertyName;
            if let Some(ty) = self.recorded_type(name_id, SyntaxKind::Identifier)
                && !(is_stand_in_signature(&ty) && self.written_type_of_symbol(symbol).is_some())
                && !(computed && matches!(ty, Type::Any | Type::ErrorType))
            {
                // The name of an overload is typed by that one signature; the
                // symbol has every overload's.
                if matches!(&ty, Type::Function(function) if function.overloads().is_none())
                    && self.overload_count(symbol) > 1
                    && let Some(group) = self.written_type_of_symbol(symbol)
                {
                    return Some(group);
                }
                if matches!(ty, Type::Any | Type::ErrorType | Type::Unknown)
                    && declaration_kind == Kind::VariableDeclaration
                    && let Some(initialized) = self.initialized_variable_type(declaration_id)
                {
                    return Some(initialized);
                }
                if matches!(ty, Type::Any | Type::Unknown)
                    && matches!(declaration_kind, Kind::Parameter | Kind::PropertySignature | Kind::PropertyDeclaration | Kind::VariableDeclaration)
                    && self.node_child(declaration_id, "questionToken").is_none()
                    && let Some(deferred) = self.node_child(declaration_id, "type").and_then(|annotation| self.deferred_type_of_node(annotation))
                {
                    return Some(deferred);
                }
                return Some(ty);
            }
        }
        if matches!(declaration_kind, Kind::PropertyAssignment | Kind::ShorthandPropertyAssignment | Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor)
            && let Some(object) = syntax.tree.parent(declaration)
            && syntax.tree.kind(object) == Kind::ObjectLiteralExpression
        {
            let object_type = self.ty(self.type_at_location(NodeId { node: object, ..declaration_id }));
            return self
                .member_names(declaration_id, &bound.name)
                .iter()
                .find_map(|name| self.query(|query| query.property(&object_type, name)).flatten())
                .map(|property| self.property_type(&property));
        }
        // A member with a computed name is late-bound: no table holds it, so
        // its owner is the declaration it is written in.
        let late_bound_owner = bound.parent.is_none().then(|| syntax.tree.parent(declaration)).flatten().and_then(|owner| {
            matches!(
                syntax.tree.kind(owner),
                Kind::ClassDeclaration | Kind::ClassExpression | Kind::InterfaceDeclaration | Kind::TypeLiteral
            )
            .then(|| syntax.bound.node_symbol.get(&owner).copied())
            .flatten()
        });
        if let Some(parent) = bound.parent.or(late_bound_owner) {
            let parent_symbol = self.bound_symbol(file, parent);
            let parent_flags = syntax.bound.symbols[parent].flags;
            let owner = if parent_flags & (sf::Class | sf::Interface | sf::TypeLiteral) != 0
                && bound.flags & sf::ClassMember != 0
                && !is_static_member(&syntax.tree, declaration)
            {
                self.declared_type_of_symbol(parent_symbol)
            } else {
                self.type_of_symbol(parent_symbol)
            };
            let owner = self.ty(owner);
            // An owner this API cannot type (a body-local class's constructor)
            // reads every member as `any`; the member's own written signature
            // below answers instead.
            let property = (!matches!(owner, Type::Any))
                .then(|| {
                    self.member_names(declaration_id, &bound.name)
                        .iter()
                        .find_map(|name| self.query(|query| query.property(&owner, name)).flatten())
                })
                .flatten();
            if let Some(property) = property.filter(|property| !matches!(property.ty, Type::ErrorType | Type::Unknown)) {
                if matches!(property.ty, Type::Any)
                    && let Some(inferred) = self.written_infer_type(declaration_id)
                {
                    return Some(inferred);
                }
                if is_stand_in_signature(&property.ty)
                    && let Some(written) = self.written_type_of_symbol(symbol)
                {
                    return Some(written);
                }
                return Some(self.property_type(&property));
            }
        }
        if let Some(written) = self.written_type_of_symbol(symbol) {
            return Some(written);
        }
        if declaration_kind == Kind::BindingElement
            && let Some(ty) = self.binding_element_type(declaration_id)
        {
            return Some(ty);
        }
        // An unannotated, mutable class property is its initializer's type,
        // widened (`getWidenedTypeForVariableLikeDeclaration`).
        if declaration_kind == Kind::PropertyDeclaration
            && self.node_child(declaration_id, "type").is_none()
            && !self.has_readonly_modifier(declaration_id)
            && let Some(initializer) = self.node_child(declaration_id, "initializer")
        {
            let ty = self.located_type(self.type_of_expression(initializer, self.program.node_kind(initializer)));
            if !matches!(ty, Type::ErrorType | Type::Unknown) {
                return self.query(|query| query.widened_literal(&ty));
            }
        }
        let checker_index = self.program.file(file_id).checker_index?;
        let name = bound.name.clone();
        let is_top_level = bound.parent.is_none_or(|parent| {
            syntax.tree.kind(syntax.bound.symbols[parent].declarations.first().copied().unwrap_or(0)) == Kind::SourceFile
        });
        if is_top_level {
            let found = self
                .query(|query| {
                    query
                        .module_value_type(checker_index, &name)
                        .or_else(|| query.module_export_type(checker_index, &name))
                })
                .flatten();
            if found.is_some() {
                return found;
            }
            if !syntax.tree.is_external_module() {
                return self.query(|query| query.global_value_type(&name)).flatten();
            }
        }
        None
    }

    /// An enum member's type: its enum literal type, which the enum's scope
    /// names `E.Member`.
    /// A type written as `text`, resolved by the checker in the scope of
    /// `location`'s file.
    pub(super) fn resolved_type_text(&self, location: NodeId, text: &str) -> Option<TypeId> {
        let checker_index = self.program.file(location.source_file()).checker_index?;
        let namespaces = self.namespace_scope(location);
        let ty = self.query(|query| query.resolve_type_text(checker_index, text, &[], &namespaces)).flatten()?;
        Some(self.intern(ty))
    }

    fn enum_member_type(&self, member: NodeId) -> Option<Type> {
        let program = &self.program;
        let enum_name = self.node_child(program.node_parent(member)?, "name")?;
        let member_name = self.node_child(member, "name")?;
        if program.node_kind(enum_name) != SyntaxKind::Identifier || program.node_kind(member_name) != SyntaxKind::Identifier {
            return None;
        }
        let text = format!("{}.{}", program.node_text(enum_name), program.node_text(member_name));
        let checker_index = program.file(member.source_file()).checker_index?;
        let namespaces = self.namespace_scope(member);
        let resolved = self
            .query(|query| query.resolve_type_text(checker_index, &text, &[], &namespaces))
            .flatten()
            .filter(|ty| !matches!(ty, Type::Any | Type::Unknown | Type::ErrorType));
        match resolved {
            Some(Type::Reference(reference)) => Some(Type::Reference(reference)),
            value => self.synthesized_enum_member_type(member, value, false),
        }
    }

    fn local_enum_member_type(&self, member: NodeId, regular: bool) -> Option<Type> {
        let declaration = self.program.node_parent(member)?;
        if !self.is_local_declaration(declaration) {
            return None;
        }
        self.synthesized_enum_member_type(member, None, regular)
    }

    /// An enum member the checker typed by its value alone (a body-local
    /// enum it lowers to an object, a member it folded to a literal): tsc's
    /// enum literal type (`Local.A`), named by the enum and holding the value,
    /// computed or not. A read's regular literal type is the declared type of
    /// an enum of one member, which tsc names by the enum.
    fn synthesized_enum_member_type(&self, member: NodeId, value: Option<Type>, regular: bool) -> Option<Type> {
        let program = &self.program;
        let declaration = program.node_parent(member)?;
        if program.node_kind(declaration) != SyntaxKind::EnumDeclaration {
            return None;
        }
        let enum_name = program.node_text(self.node_child(declaration, "name")?).to_string();
        let member_name = program.node_value_text(self.node_child(member, "name")?)?.to_string();
        let value = match value {
            Some(value) => value,
            None => {
                let enum_symbol = self.declared_symbol(declaration)?;
                let object = self.ty(self.type_of_symbol(enum_symbol));
                self.query(|query| query.property(&object, &member_name)).flatten()?.ty
            }
        };
        let numeric = match value {
            Type::NumberLiteral(_) | Type::Number => true,
            Type::StringLiteral(_) | Type::String => false,
            _ => return None,
        };
        let (start, _) = program.node_byte_span(declaration);
        let owner = format!("\0local-enum\0{}\0{start}\0{enum_name}", declaration.file);
        let display = if regular && self.list_property(declaration, "members").len() == 1 {
            enum_name.clone()
        } else {
            format!("{enum_name}.{member_name}")
        };
        let reference =
            surge_ts_types::TypeReference::new(format!("{owner}.{member_name}"), display, Vec::new(), Arc::new(AliasBody(value)))
                .with_enum_owner(Arc::from(owner.as_str()));
        Some(Type::Reference(if numeric { reference.numeric_enum() } else { reference }))
    }

    /// How many overload signatures (bodiless function or method
    /// declarations) a symbol declares.
    fn overload_count(&self, symbol: SymbolId) -> usize {
        self.symbol_declarations(symbol)
            .into_iter()
            .filter(|declaration| {
                matches!(
                    self.program.node_kind(*declaration),
                    SyntaxKind::FunctionDeclaration | SyntaxKind::MethodDeclaration | SyntaxKind::MethodSignature
                ) && self.node_child(*declaration, "body").is_none()
            })
            .count()
    }

    /// A symbol's type as its declarations write it: an annotation, or the
    /// signatures of a function-like declaration (an overload group as a type
    /// literal of call signatures), resolved by the checker where they are
    /// written. `None` for a declaration whose type is inferred.
    fn written_type_of_symbol(&self, symbol: SymbolId) -> Option<Type> {
        let declarations = self.symbol_declarations(symbol);
        let first = *declarations.first()?;
        let program = &self.program;
        let kind = program.node_kind(first);
        if let Some(inferred) = self.written_infer_type(first) {
            return Some(inferred);
        }
        // `typeof x` naming a sibling parameter, which no file scope sees.
        if kind == SyntaxKind::Parameter
            && let Some(annotation) = self.node_child(first, "type").map(|annotation| self.unwrap_type_node(annotation))
            && program.node_kind(annotation) == SyntaxKind::TypeQuery
            && let Some(name) = self.node_child(annotation, "exprName")
            && let Some(queried) = self.queried_value_type(name)
        {
            return Some(self.ty(queried));
        }
        let text = match kind {
            SyntaxKind::Parameter => {
                let annotation = program.node_text(self.node_child(first, "type")?).to_string();
                // An initializer makes a parameter optional to callers, not
                // possibly undefined in the body.
                let optional = self.node_child(first, "questionToken").is_some();
                if optional && self.program.inner().strict_null_checks && self.node_child(first, "dotDotDotToken").is_none() {
                    format!("({annotation}) | undefined")
                } else {
                    annotation
                }
            }
            SyntaxKind::PropertyDeclaration | SyntaxKind::PropertySignature | SyntaxKind::VariableDeclaration => {
                let annotation = program.node_text(self.node_child(first, "type")?).to_string();
                if self.node_child(first, "questionToken").is_some() && self.program.inner().strict_null_checks {
                    format!("({annotation}) | undefined")
                } else {
                    annotation
                }
            }
            SyntaxKind::GetAccessor => program.node_text(self.node_child(first, "type")?).to_string(),
            SyntaxKind::SetAccessor => {
                let accessor = declarations.iter().copied().find(|declaration| program.node_kind(*declaration) == SyntaxKind::GetAccessor);
                match accessor.and_then(|getter| self.node_child(getter, "type")) {
                    Some(annotation) => program.node_text(annotation).to_string(),
                    None => match program.node_property(first, "parameters") {
                        Some(NodePropertyValue::List { nodes, .. }) => program.node_text(self.node_child(*nodes.first()?, "type")?).to_string(),
                        _ => return None,
                    },
                }
            }
            SyntaxKind::FunctionDeclaration | SyntaxKind::MethodDeclaration | SyntaxKind::MethodSignature => {
                let signatures: Vec<NodeId> = declarations
                    .iter()
                    .copied()
                    .filter(|declaration| program.node_kind(*declaration) == kind)
                    .collect();
                let overloads: Vec<NodeId> = if signatures.len() > 1 {
                    signatures.iter().copied().filter(|declaration| self.node_child(*declaration, "body").is_none()).collect()
                } else {
                    signatures
                };
                let parts = overloads.iter().map(|declaration| self.signature_parts(*declaration)).collect::<Option<Vec<_>>>()?;
                match parts.as_slice() {
                    [(head, parameters, returns)] => format!("{head}({parameters}) => {returns}"),
                    _ => format!(
                        "{{ {}; }}",
                        parts
                            .iter()
                            .map(|(head, parameters, returns)| format!("{head}({parameters}): {returns}"))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
            SyntaxKind::ClassDeclaration | SyntaxKind::EnumDeclaration | SyntaxKind::ModuleDeclaration => {
                let name = self.node_child(first, "name")?;
                if program.node_kind(name) != SyntaxKind::Identifier {
                    return None;
                }
                format!("typeof {}", program.node_text(name))
            }
            _ => return None,
        };
        let checker_index = program.file(first.source_file()).checker_index?;
        let type_parameters = self.type_parameters_in_scope(first);
        let namespaces = self.namespace_scope(first);
        let resolved = self.query(|query| query.resolve_type_text(checker_index, &text, &type_parameters, &namespaces)).flatten();
        if resolved.as_ref().is_none_or(|ty| matches!(ty, Type::Any | Type::Unknown | Type::ErrorType))
            && matches!(kind, SyntaxKind::Parameter | SyntaxKind::PropertyDeclaration | SyntaxKind::PropertySignature | SyntaxKind::VariableDeclaration)
            && self.node_child(first, "questionToken").is_none()
            && let Some(deferred) = self.node_child(first, "type").and_then(|annotation| self.deferred_type_of_node(annotation))
        {
            return Some(deferred);
        }
        resolved
    }

    /// A function-like declaration's signature as source text: its type
    /// parameter list, its parameter list (`a: A, b?: B`) and its return type.
    /// `None` when a parameter's or the return type is not written.
    fn signature_parts(&self, declaration: NodeId) -> Option<(String, String, String)> {
        let program = &self.program;
        let type_parameters = match program.node_property(declaration, "typeParameters") {
            Some(NodePropertyValue::List { nodes, .. }) if !nodes.is_empty() => {
                format!("<{}>", nodes.iter().map(|parameter| program.node_text(*parameter)).collect::<Vec<_>>().join(", "))
            }
            _ => String::new(),
        };
        let parameters = match program.node_property(declaration, "parameters") {
            Some(NodePropertyValue::List { nodes, .. }) => nodes,
            _ => Vec::new(),
        };
        let mut written = Vec::new();
        for (position, parameter) in parameters.iter().enumerate() {
            let name = self.node_child(*parameter, "name")?;
            let name = match program.node_kind(name) {
                SyntaxKind::Identifier => program.node_text(name).to_string(),
                _ => format!("__{position}"),
            };
            let rest = if self.node_child(*parameter, "dotDotDotToken").is_some() { "..." } else { "" };
            let optional = if self.node_child(*parameter, "questionToken").is_some() || self.node_child(*parameter, "initializer").is_some() {
                "?"
            } else {
                ""
            };
            let annotation = program.node_text(self.node_child(*parameter, "type")?);
            // A `?` parameter's type includes `undefined` (tsc's `addOptionality`).
            if rest.is_empty() && self.node_child(*parameter, "questionToken").is_some() && program.inner().strict_null_checks {
                written.push(format!("{name}?: ({annotation}) | undefined"));
            } else {
                written.push(format!("{rest}{name}{optional}: {annotation}"));
            }
        }
        let return_type = match self.node_child(declaration, "type") {
            Some(annotation) => program.node_text(annotation).to_string(),
            // Unwritten, a plain body that returns no value returns `void`
            // (`getReturnTypeFromBody` with no return expressions).
            None if self.node_child(declaration, "asteriskToken").is_none()
                && !self.has_async_modifier(declaration)
                && self.node_child(declaration, "body").is_some()
                && !self.returns_a_value(declaration) =>
            {
                "void".to_string()
            }
            None => return None,
        };
        Some((type_parameters, written.join(", "), return_type))
    }

    /// Whether a function-like declaration's body returns a value anywhere
    /// outside its nested functions.
    fn returns_a_value(&self, function: NodeId) -> bool {
        let program = &self.program;
        let Some(body) = self.node_child(function, "body") else { return false };
        let mut pending = vec![body];
        while let Some(node) = pending.pop() {
            match program.node_kind(node) {
                SyntaxKind::FunctionDeclaration
                | SyntaxKind::FunctionExpression
                | SyntaxKind::ArrowFunction
                | SyntaxKind::MethodDeclaration
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor
                | SyntaxKind::ClassDeclaration
                | SyntaxKind::ClassExpression => {}
                SyntaxKind::ReturnStatement => {
                    if self.node_child(node, "expression").is_some() {
                        return true;
                    }
                }
                _ => pending.extend(program.node_children(node)),
            }
        }
        false
    }

    /// The type parameters of every declaration `node` is written inside.
    fn type_parameters_in_scope(&self, node: NodeId) -> Vec<String> {
        let mut names = Vec::new();
        let mut current = Some(node);
        while let Some(owner) = current {
            if let Some(NodePropertyValue::List { nodes, .. }) = self.program.node_property(owner, "typeParameters") {
                for parameter in nodes {
                    if let Some(name) = self.node_child(parameter, "name") {
                        names.push(self.program.node_text(name).to_string());
                    }
                }
            }
            current = self.program.node_parent(owner);
            // A conditional's `infer` declarations are in scope in its
            // `extends` clause and its true branch.
            if let Some(conditional) = current
                && self.program.node_kind(conditional) == SyntaxKind::ConditionalType
                && let Some(extends) = self.node_child(conditional, "extendsType")
                && (extends == owner || self.node_child(conditional, "trueType") == Some(owner))
            {
                self.collect_infer_names(extends, &mut names);
            }
        }
        names
    }

    /// A type tsc defers over a type parameter in scope (`Parameters<T>`,
    /// `K[1]`, `T extends string ? T : never`), which the checker reads as
    /// `unknown`: a reference displayed as tsc prints the deferred type.
    fn deferred_type_of_node(&self, node: NodeId) -> Option<Type> {
        let node = self.unwrap_type_node(node);
        let kind = self.program.node_kind(node);
        let generic = match kind {
            SyntaxKind::TypeReference => self.node_property(node, "typeArguments").is_some(),
            SyntaxKind::IndexedAccessType | SyntaxKind::ConditionalType => true,
            _ => false,
        };
        if !generic {
            return None;
        }
        let in_scope = self.type_parameters_in_scope(node);
        if in_scope.is_empty() || !self.mentions_type_parameter(node, &in_scope) {
            return None;
        }
        let display = self.render_type_node(node)?;
        Some(Type::Reference(surge_ts_types::TypeReference::new(
            format!("\0deferred\0{display}"),
            display,
            Vec::new(),
            Arc::new(AliasBody(Type::Unknown)),
        )))
    }

    fn mentions_type_parameter(&self, node: NodeId, names: &[String]) -> bool {
        if self.program.node_kind(node) == SyntaxKind::TypeReference
            && let Some(name) = self.node_child(node, "typeName")
            && names.iter().any(|parameter| parameter == self.program.node_text(name))
        {
            return true;
        }
        self.program.node_children(node).into_iter().any(|child| self.mentions_type_parameter(child, names))
    }

    /// tsc's `typeToString` of a written type built from references, indexed
    /// accesses, conditionals, operators and literals; `None` for any other.
    fn render_type_node(&self, node: NodeId) -> Option<String> {
        let program = &self.program;
        let list = |property: &str, separator: &str| -> Option<String> {
            let rendered: Option<Vec<String>> =
                self.list_property(node, property).into_iter().map(|member| self.render_type_node(member)).collect();
            Some(rendered?.join(separator))
        };
        let child = |property: &str| self.render_type_node(self.node_child(node, property)?);
        // An operand that binds looser than its position is parenthesized.
        let operand = |property: &str| -> Option<String> {
            let inner = self.unwrap_type_node(self.node_child(node, property)?);
            let rendered = self.render_type_node(inner)?;
            Some(match program.node_kind(inner) {
                SyntaxKind::UnionType | SyntaxKind::IntersectionType | SyntaxKind::ConditionalType | SyntaxKind::FunctionType => {
                    format!("({rendered})")
                }
                _ => rendered,
            })
        };
        let kind = program.node_kind(node);
        if keyword_type(kind).is_some()
            || matches!(kind, SyntaxKind::ObjectKeyword | SyntaxKind::NullKeyword | SyntaxKind::ThisType)
        {
            return Some(program.node_text(node).to_string());
        }
        Some(match kind {
            SyntaxKind::ParenthesizedType => child("type")?,
            SyntaxKind::TypeReference => {
                let name: String = program.node_text(self.node_child(node, "typeName")?).split_whitespace().collect();
                match self.node_property(node, "typeArguments") {
                    Some(_) => format!("{name}<{}>", list("typeArguments", ", ")?),
                    None => name,
                }
            }
            SyntaxKind::IndexedAccessType => format!("{}[{}]", operand("objectType")?, child("indexType")?),
            SyntaxKind::ConditionalType => format!(
                "{} extends {} ? {} : {}",
                operand("checkType")?,
                operand("extendsType")?,
                child("trueType")?,
                child("falseType")?
            ),
            SyntaxKind::UnionType => list("types", " | ")?,
            SyntaxKind::IntersectionType => list("types", " & ")?,
            SyntaxKind::ArrayType => format!("{}[]", operand("elementType")?),
            SyntaxKind::TupleType => format!("[{}]", list("elements", ", ")?),
            SyntaxKind::TypeOperator => {
                let operator = match program.node_operator(node)? {
                    SyntaxKind::KeyOfKeyword => "keyof",
                    SyntaxKind::ReadonlyKeyword => "readonly",
                    SyntaxKind::UniqueKeyword => "unique",
                    _ => return None,
                };
                format!("{operator} {}", operand("type")?)
            }
            SyntaxKind::InferType => {
                let parameter = self.node_child(node, "typeParameter")?;
                let name = program.node_text(self.node_child(parameter, "name")?).to_string();
                match self.node_child(parameter, "constraint") {
                    Some(constraint) => format!("infer {name} extends {}", self.render_type_node(constraint)?),
                    None => format!("infer {name}"),
                }
            }
            SyntaxKind::LiteralType => {
                let literal = self.node_child(node, "literal")?;
                match program.node_kind(literal) {
                    SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral => {
                        format!("{:?}", program.node_value_text(literal)?)
                    }
                    _ => program.node_text(literal).split_whitespace().collect(),
                }
            }
            _ => return None,
        })
    }

    /// The type parameter a member or parameter annotated `infer U` declares,
    /// which the checker reads off its conditional's match alone.
    fn written_infer_type(&self, declaration: NodeId) -> Option<Type> {
        let annotation = self.unwrap_type_node(self.node_child(declaration, "type")?);
        if self.program.node_kind(annotation) != SyntaxKind::InferType {
            return None;
        }
        let inferred = self.type_of_type_node(annotation).filter(|ty| !matches!(ty, Type::Any | Type::ErrorType | Type::Unknown))?;
        if self.node_child(declaration, "questionToken").is_some() && self.program.inner().strict_null_checks {
            return Some(surge_ts_types::union_type(vec![inferred, Type::Undefined]));
        }
        Some(inferred)
    }

    fn collect_infer_names(&self, node: NodeId, names: &mut Vec<String>) {
        match self.program.node_kind(node) {
            SyntaxKind::InferType => {
                if let Some(name) = self.node_child(node, "typeParameter").and_then(|parameter| self.node_child(parameter, "name")) {
                    names.push(self.program.node_text(name).to_string());
                }
            }
            // A nested conditional's own `extends` clause declares its own.
            SyntaxKind::ConditionalType => {
                if let Some(check) = self.node_child(node, "checkType") {
                    self.collect_infer_names(check, names);
                }
            }
            _ => {
                for child in self.program.node_children(node) {
                    self.collect_infer_names(child, names);
                }
            }
        }
    }

    /// `getDeclaredTypeOfSymbol`: the type a class, interface, type alias,
    /// enum or type parameter declares.
    pub fn declared_type_of_symbol(&self, symbol: SymbolId) -> TypeId {
        self.declared_type_at(symbol, None)
    }

    /// [`Self::declared_type_of_symbol`], resolved preferably where
    /// `location` names the symbol.
    fn declared_type_at(&self, symbol: SymbolId, location: Option<NodeId>) -> TypeId {
        if self.is_unresolved_symbol(symbol) {
            return self.unresolved_type();
        }
        let key = Some(TypeKey::DeclaredType(symbol));
        if let Some(index) = self.with_tables(|tables| tables.type_by_key.get(key.as_ref().unwrap()).copied()) {
            return TypeId { program: symbol.program, index };
        }
        if !self.with_tables(|tables| tables.declared_in_progress.insert(symbol)) {
            return self.error_type();
        }
        // An alias's declared type is its target's, the same type; a type
        // meaning of its own comes first (`tryGetDeclaredTypeOfSymbol`).
        if self.is_alias(symbol)
            && self.symbol_flags(symbol) & (ts_sf::Class | ts_sf::Interface | ts_sf::TypeAlias | ts_sf::TypeParameter | ts_sf::Enum | ts_sf::EnumMember) == 0
            && let Some(target) = self.aliased_symbol(symbol).filter(|target| *target != symbol)
        {
            let target_type = self.declared_type_at(target, location);
            self.with_tables(|tables| {
                tables.declared_in_progress.remove(&symbol);
                tables.type_by_key.insert(TypeKey::DeclaredType(symbol), target_type.index);
            });
            return target_type;
        }
        let ty = self.compute_declared_type(symbol, location).unwrap_or(Type::ErrorType);
        self.with_tables(|tables| tables.declared_in_progress.remove(&symbol));
        // `type Uppercase<S> = intrinsic` declares tsc's `intrinsicMarkerType`,
        // an `any` named `intrinsic`.
        if self.symbol_declarations(symbol).into_iter().any(|declaration| {
            self.program.node_kind(declaration) == SyntaxKind::TypeAliasDeclaration
                && self
                    .node_child(declaration, "type")
                    .is_some_and(|body| self.program.node_kind(body) == SyntaxKind::IntrinsicKeyword)
        }) {
            let marker = self.type_id(Some(TypeKey::UnresolvedReference("\0intrinsic".to_string())), None, || Type::ErrorType);
            self.with_tables(|tables| {
                tables.error_displays.insert(marker.index, "intrinsic".to_string());
                tables.type_by_key.insert(key.clone().unwrap(), marker.index);
            });
            return marker;
        }
        // An alias of `typeof Entity` is that entity's own type.
        if let Some(declaration) = self
            .symbol_declarations(symbol)
            .into_iter()
            .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
            && let Some(body) = self.node_child(declaration, "type").map(|body| self.unwrap_type_node(body))
            && self.program.node_kind(body) == SyntaxKind::TypeQuery
            && let Some(entity) = self.queried_entity_type(body)
        {
            self.with_tables(|tables| tables.type_by_key.insert(key.clone().unwrap(), entity.index));
            return entity;
        }
        // An alias of a reference nothing declares is that reference's error
        // type; one that reads itself back (`type A = A["x"]`) is no such alias.
        if matches!(ty, Type::ErrorType | Type::Any | Type::Unknown)
            && let Some(declaration) = self
                .symbol_declarations(symbol)
                .into_iter()
                .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
            && let Some(body) = self.node_child(declaration, "type")
            && self.with_tables(|tables| tables.declared_in_progress.insert(symbol))
            && let Some(error) = {
                let error = self.unresolved_type_node(body);
                self.with_tables(|tables| tables.declared_in_progress.remove(&symbol));
                error
            }
        {
            self.with_tables(|tables| tables.type_by_key.insert(key.unwrap(), error.index));
            return error;
        }
        let ty = match ty {
            Type::Any | Type::Unknown | Type::ErrorType => self.named_declared_type(symbol).unwrap_or(ty),
            ty => ty,
        };
        self.type_id(key, Some(symbol), || ty)
    }

    /// A class, interface, enum or object-like alias the checker could not
    /// resolve, named as tsc names the type it declares (`Box<T>`).
    fn named_declared_type(&self, symbol: SymbolId) -> Option<Type> {
        let declarations = self.symbol_declarations(symbol);
        let named = declarations.iter().copied().find(|declaration| match self.program.node_kind(*declaration) {
            SyntaxKind::ClassDeclaration | SyntaxKind::InterfaceDeclaration | SyntaxKind::EnumDeclaration => true,
            SyntaxKind::TypeAliasDeclaration => self.node_child(*declaration, "type").is_some_and(|body| {
                matches!(
                    self.program.node_kind(self.unwrap_type_node(body)),
                    SyntaxKind::TypeLiteral | SyntaxKind::MappedType | SyntaxKind::FunctionType | SyntaxKind::ConstructorType
                )
            }),
            _ => false,
        })?;
        let name = self.node_child(named, "name")?;
        if self.program.node_kind(name) != SyntaxKind::Identifier || self.program.node_text(name).is_empty() {
            return None;
        }
        // A module augmentation's declarations merge into the module only;
        // the checker resolving nothing there means no module took them.
        if self.program.is_external_module(named.source_file()) {
            let mut ancestor = self.program.node_parent(named);
            while let Some(current) = ancestor {
                if self.program.node_kind(current) == SyntaxKind::ModuleDeclaration
                    && self.node_child(current, "name").is_some_and(|name| self.program.node_kind(name) == SyntaxKind::StringLiteral)
                {
                    return None;
                }
                ancestor = self.program.node_parent(current);
            }
        }
        // Merged declarations append the type parameters they name anew
        // (`appendTypeParameters`).
        let mut parameters: Vec<String> = Vec::new();
        for declaration in &declarations {
            if let Some(NodePropertyValue::List { nodes, .. }) = self.program.node_property(*declaration, "typeParameters") {
                for parameter in nodes {
                    if let Some(name) = self.node_child(parameter, "name").map(|name| self.program.node_text(name).to_string())
                        && !parameters.contains(&name)
                    {
                        parameters.push(name);
                    }
                }
            }
        }
        let name = self.program.node_text(name);
        let display = if parameters.is_empty() { name.to_string() } else { format!("{name}<{}>", parameters.join(", ")) };
        let (start, _) = self.program.node_byte_span(named);
        Some(Type::Reference(surge_ts_types::TypeReference::new(
            format!("\0named\0{}\0{start}\0{display}", named.file),
            display,
            Vec::new(),
            Arc::new(AliasBody(Type::Unknown)),
        )))
    }

    fn compute_declared_type(&self, symbol: SymbolId, location: Option<NodeId>) -> Option<Type> {
        let (file, name, flags) = match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                let syntax = self.program.syntax(self.program.file_id(file)).clone();
                let bound = &syntax.bound.symbols[symbol];
                (file, bound.name.clone(), bound.flags)
            }
            SymbolData::Global { name, parts } => {
                let (file, symbol) = *parts.first()?;
                let syntax = self.program.syntax(self.program.file_id(file)).clone();
                let flags = parts
                    .iter()
                    .map(|&(file, symbol)| self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].flags)
                    .fold(0, |all, flags| all | flags);
                let _ = symbol;
                drop(syntax);
                (file, name.to_string(), flags)
            }
            SymbolData::Transient { .. } => return None,
        };
        if flags & sf::Alias != 0 && flags & (sf::Class | sf::Interface | sf::TypeAlias | sf::TypeParameter | sf::Enum | sf::EnumMember) == 0 {
            let target = self.aliased_symbol(symbol)?;
            return (target != symbol).then(|| self.ty(self.declared_type_at(target, location)));
        }
        if flags & sf::TypeParameter != 0 {
            return Some(Type::type_parameter(&name));
        }
        if flags & sf::EnumMember != 0 {
            let member = *self.symbol_declarations(symbol).first()?;
            return self.enum_member_type(member);
        }
        if flags & (sf::Class | sf::Interface | sf::TypeAlias | sf::Enum) == 0 {
            return None;
        }
        // `export default class A` declares `default`; its scope names it `A`.
        let name = if name == "default" { self.symbol_name_as_written(symbol) } else { name };
        let type_parameters = self.declared_type_parameter_names(symbol);
        // A declaration inside namespaces is named by its path from the file;
        // one inside an ambient module, by its name where it is imported.
        let (path, outermost) = self.namespace_path(symbol).unwrap_or_else(|| (name.clone(), symbol));
        let text = if type_parameters.is_empty() { path.clone() } else { format!("{path}<{}>", type_parameters.join(", ")) };
        let head = path.split('.').next().unwrap_or(&path).to_string();
        let with_parameters = |name: &str| {
            if type_parameters.is_empty() { name.to_string() } else { format!("{name}<{}>", type_parameters.join(", ")) }
        };
        let resolve = |scope: super::handles::SourceFileId, text: &str, namespaces: &[String]| {
            let checker_index = self.program.file(scope).checker_index?;
            self.query(|query| query.resolve_type_text(checker_index, text, &type_parameters, namespaces))
                .flatten()
                .filter(|ty| !matches!(ty, Type::Unknown | Type::ErrorType))
        };
        // A class or interface is named by its symbol (`getDeclaredTypeOfClassOrInterface`),
        // however its base resolved: one the checker opened over a base it
        // could not resolve (`class C extends Array {}`) is still `C`.
        let class_or_interface = flags & (sf::Class | sf::Interface) != 0 && flags & sf::TypeAlias == 0;
        let aliased = |ty: Type| match self.alias_type(symbol, &name, &type_parameters, ty) {
            Type::Object(object) if class_or_interface && object.alias_name.is_none() => {
                Type::Object(object.with_alias_name(with_parameters(&name)))
            }
            ty => ty,
        };
        // A class expression is named by no scope: its instance is what the
        // constructor recorded at the expression constructs, its `prototype`.
        if flags & sf::Class != 0
            && let Some(&declaration) = self.symbol_declarations(symbol).first()
            && self.program.node_kind(declaration) == SyntaxKind::ClassExpression
            && let Some(constructor) = self.recorded_type(declaration, SyntaxKind::ClassExpression)
            && let Some(prototype) = self.query(|query| query.property(&constructor, "prototype")).flatten()
        {
            return Some(self.property_type(&prototype));
        }
        // A function body's own type, where that body declares it.
        if let Some(&declaration) = self.symbol_declarations(symbol).first()
            && self.is_local_declaration(declaration)
            && let Some(name_node) = self.node_child(declaration, "name")
            && let Some(checker_index) = self.program.file(declaration.source_file()).checker_index
        {
            let span = self.program.node_byte_span(name_node);
            let text = with_parameters(&name);
            if let Some(ty) = self
                .query(|query| query.resolve_local_type_text(checker_index, &text, &type_parameters, span))
                .flatten()
                .filter(|ty| !matches!(ty, Type::Unknown | Type::ErrorType))
            {
                return Some(aliased(ty));
            }
        }
        // The name as the reference writes it, where it is written: what the
        // checker resolved there, which reaches an import of a member of an
        // `export =` namespace no path from a file does.
        if let Some(location) = location
            && let Some(written) = self.written_entity_name(location)
            && let Some(ty) = resolve(location.source_file(), &with_parameters(&written), &self.namespace_scope(location))
        {
            return Some(aliased(ty));
        }
        // Its bare name inside the namespaces that declare it, which reaches
        // a member they do not export.
        let declaring = self.program.file_id(file);
        if let Some(&declaration) = self.symbol_declarations(symbol).first()
            && let Some(ty) = resolve(declaring, &with_parameters(&name), &self.namespace_scope(declaration))
        {
            return Some(aliased(ty));
        }
        // An alias's body as written, where it is written: a local alias's
        // or one the checker cannot instantiate by name.
        if flags & sf::TypeAlias != 0
            && let Some(declaration) = self
                .symbol_declarations(symbol)
                .into_iter()
                .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
            && let Some(body) = self.node_child(declaration, "type")
            && let Some(ty) = resolve(declaration.source_file(), self.program.node_text(body), &self.namespace_scope(declaration))
        {
            return Some(aliased(ty));
        }
        // Else a file that sees the same declaration by its path (the
        // declaring file may be a default library the checker kept no scope
        // for), the referencing one first.
        let mut scopes = vec![declaring];
        scopes.extend(location.map(NodeId::source_file));
        scopes.extend(
            self.program
                .root_file_names()
                .iter()
                .filter_map(|root| self.program.source_file(root)),
        );
        scopes.dedup();
        for scope in scopes {
            if scope != declaring {
                let meaning = if head == path { Meaning::Type } else { Meaning::Namespace };
                let seen = self
                    .resolve_name(self.program.root(scope), &head, meaning)
                    .map(|found| if self.is_alias(found) { self.aliased_symbol(found).unwrap_or(found) } else { found });
                if seen != Some(outermost) {
                    continue;
                }
            }
            if let Some(ty) = resolve(scope, &text, &[]) {
                return Some(aliased(ty));
            }
        }
        // A generic alias is named by its instantiation however little of its
        // body the checker resolves.
        // A generic alias is named by its instantiation however little of its
        // body the checker resolves, unless the body names something nothing
        // declares (tsc's error type then).
        let declaration = self
            .symbol_declarations(symbol)
            .into_iter()
            .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)?;
        let body = self.node_child(declaration, "type")?;
        if self.names_unresolved_type(body) {
            return None;
        }
        Some(aliased(Type::ErrorType)).filter(|ty| matches!(ty, Type::Reference(_)))
    }

    /// A type alias's declared type as tsc names it. tsc attaches the alias
    /// to a type its body creates (`getAliasSymbolForTypeNode`): an object,
    /// function, mapped, array or tuple type, a union or intersection that
    /// stays one, a deferred conditional or indexed access (or one that
    /// distributes into a new union), an instantiation of a generic
    /// interface, class or alias. A generic alias's is then its
    /// instantiation with its own type parameters, printed `Name<T>`. An
    /// instantiation of a homomorphic mapped alias (`Partial<User>`) keeps
    /// that alias, as does one a local alias names; a resolved conditional,
    /// `keyof`, `typeof` or a template literal is printed as itself.
    fn alias_type(&self, symbol: SymbolId, name: &str, type_parameters: &[String], body: Type) -> Type {
        let program = &self.program;
        let Some(declaration) = self
            .symbol_declarations(symbol)
            .into_iter()
            .find(|declaration| program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
        else {
            return body;
        };
        let Some(node) = self.node_child(declaration, "type").map(|node| self.unwrap_type_node(node)) else { return body };
        // A bare reference to another alias is that alias's declared type.
        if program.node_kind(node) == SyntaxKind::TypeReference
            && !matches!(program.node_property(node, "typeArguments"), Some(NodePropertyValue::List { nodes, .. }) if !nodes.is_empty())
            && let Some(target) = self
                .node_child(node, "typeName")
                .and_then(|name| self.symbol_at_location(name))
                .map(|target| if self.is_alias(target) { self.aliased_symbol(target).unwrap_or(target) } else { target })
            && target != symbol
            && self.symbol_flags(target) & ts_sf::TypeAlias != 0
            && !self.is_unresolved_symbol(target)
            && self.declared_type_parameter_names(target).is_empty()
        {
            let declared = self.ty(self.declared_type_of_symbol(target));
            if !matches!(declared, Type::ErrorType) {
                return declared;
            }
        }
        let generic_in = |node: NodeId, parameters: &[String]| mentions_name(program.node_text(node), parameters);
        let body_text = || {
            let structure = without_alias_display(body.clone(), name);
            self.query(|query| surge_ts_types::with_tsc_display(|| query.type_to_string(&structure))).unwrap_or_default()
        };
        let resolves_to_union = |node: NodeId| {
            let Some(checker_index) = program.file(declaration.source_file()).checker_index else { return false };
            let namespaces = self.namespace_scope(declaration);
            self.query(|query| query.resolve_type_text(checker_index, program.node_text(node), type_parameters, &namespaces))
                .flatten()
                .is_some_and(|ty| matches!(ty.peeled(), Type::Union(_)))
        };
        let attaches = match program.node_kind(node) {
            SyntaxKind::TypeLiteral
            | SyntaxKind::FunctionType
            | SyntaxKind::ConstructorType
            | SyntaxKind::MappedType
            | SyntaxKind::ArrayType => Attach::Alias,
            // A tuple spreading another is normalized into a new one.
            SyntaxKind::TupleType if !self.spreads_tuple(node) => Attach::Alias,
            SyntaxKind::UnionType if !type_parameters.is_empty() || has_top_level(&body_text(), " | ") => Attach::Alias,
            // `A & {}` reduces to `A`, which keeps its own name; `string &
            // number` to `never`.
            SyntaxKind::IntersectionType
                if !type_parameters.is_empty() || (self.intersection_operand_count(node) > 1 && !matches!(body.peeled(), Type::Never)) =>
            {
                Attach::Alias
            }
            SyntaxKind::ConditionalType if generic_in(node, type_parameters) => Attach::Alias,
            // An access by a union of keys unions the properties anew; one
            // property's type is that property's own.
            SyntaxKind::IndexedAccessType
                if generic_in(node, type_parameters)
                    || self.node_child(node, "indexType").is_some_and(|index| resolves_to_union(index)) =>
            {
                Attach::Alias
            }
            SyntaxKind::TypeReference => self.reference_alias_attachment(declaration, node, type_parameters),
            _ => Attach::None,
        };
        match attaches {
            Attach::None => without_alias_display(body, name),
            Attach::Alias if type_parameters.is_empty() => {
                with_alias_display(body, name, format!("\0api-alias\0{}\0{name}", declaration.source_file().index))
            }
            Attach::Alias => {
                let arguments: Vec<Type> = type_parameters.iter().map(|parameter| Type::type_parameter(parameter)).collect();
                Type::Reference(surge_ts_types::TypeReference::new(
                    format!("\0api-alias\0{}\0{name}", declaration.source_file().index),
                    format!("{name}<{}>", type_parameters.join(", ")),
                    arguments,
                    Arc::new(AliasBody(body)),
                ))
            }
            Attach::Target(display) => Type::Reference(surge_ts_types::TypeReference::new(
                format!("\0api-alias\0{}\0{name}", declaration.source_file().index),
                display,
                Vec::new(),
                Arc::new(AliasBody(without_alias_display(body, name))),
            )),
        }
    }

    /// Whether the alias declared by `declaration` names the instantiation
    /// its body `reference` writes (`getTypeFromTypeAliasReference`,
    /// `instantiateMappedType`).
    fn reference_alias_attachment(&self, declaration: NodeId, reference: NodeId, type_parameters: &[String]) -> Attach {
        let program = &self.program;
        let arguments = match program.node_property(reference, "typeArguments") {
            Some(NodePropertyValue::List { nodes, .. }) if !nodes.is_empty() => nodes,
            // A bare reference is the referenced type itself.
            _ => return Attach::None,
        };
        let Some(target) = self
            .node_child(reference, "typeName")
            .and_then(|name| self.symbol_at_location(name))
            .map(|target| if self.is_alias(target) { self.aliased_symbol(target).unwrap_or(target) } else { target })
        else {
            return Attach::None;
        };
        if self.symbol_flags(target) & ts_sf::TypeAlias == 0 {
            return Attach::Alias;
        }
        let Some(target_declaration) = self
            .symbol_declarations(target)
            .into_iter()
            .find(|declaration| program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
        else {
            return Attach::None;
        };
        let target_parameters = self.declared_type_parameter_names(target);
        let Some(target_body) = self.node_child(target_declaration, "type").map(|node| self.unwrap_type_node(node)) else {
            return Attach::None;
        };
        let generic_arguments = arguments.iter().any(|argument| mentions_name(program.node_text(*argument), type_parameters));
        let target_display = || {
            let checker_index = program.file(declaration.source_file()).checker_index?;
            let namespaces = self.namespace_scope(declaration);
            let rendered = arguments
                .iter()
                .map(|argument| {
                    let ty = self
                        .query(|query| query.resolve_type_text(checker_index, program.node_text(*argument), type_parameters, &namespaces))
                        .flatten()?;
                    Some(self.type_to_string(self.intern(ty)))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{}<{}>", self.symbol_name(target), rendered.join(", ")))
        };
        match program.node_kind(target_body) {
            // A conditional resolves for arguments that settle it.
            SyntaxKind::ConditionalType if !generic_arguments => Attach::None,
            // A homomorphic mapped type maps a union argument constituent-wise
            // under the new alias; any other argument keeps its own.
            SyntaxKind::MappedType if self.homomorphic_parameter(target_body, &target_parameters).is_some() => {
                let position = self.homomorphic_parameter(target_body, &target_parameters).unwrap();
                let union_argument = arguments.get(position).is_some_and(|argument| {
                    program.node_kind(self.unwrap_type_node(*argument)) == SyntaxKind::UnionType
                });
                if union_argument {
                    Attach::Alias
                } else {
                    target_display().map_or(Attach::None, Attach::Target)
                }
            }
            // A local alias does not name a top-level alias's instantiation.
            _ if self.is_local_declaration(declaration) && !self.is_local_declaration(target_declaration) => {
                target_display().map_or(Attach::None, Attach::Target)
            }
            _ => Attach::Alias,
        }
    }

    /// The position of the type parameter a mapped type `{ [K in keyof T]: … }`
    /// is homomorphic in.
    fn homomorphic_parameter(&self, mapped: NodeId, parameters: &[String]) -> Option<usize> {
        let program = &self.program;
        let constraint = self.node_child(self.node_child(mapped, "typeParameter")?, "constraint")?;
        let constraint = self.unwrap_type_node(constraint);
        if program.node_kind(constraint) != SyntaxKind::TypeOperator || program.node_operator(constraint) != Some(SyntaxKind::KeyOfKeyword) {
            return None;
        }
        let operand = self.unwrap_type_node(self.node_child(constraint, "type")?);
        if program.node_kind(operand) != SyntaxKind::TypeReference {
            return None;
        }
        let text = program.node_text(operand);
        parameters.iter().position(|parameter| parameter == text)
    }

    /// An intersection type's operands but `{}` and `unknown`, which it drops.
    fn intersection_operand_count(&self, intersection: NodeId) -> usize {
        let program = &self.program;
        let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(intersection, "types") else { return 0 };
        nodes
            .into_iter()
            .map(|operand| self.unwrap_type_node(operand))
            .filter(|operand| match program.node_kind(*operand) {
                SyntaxKind::UnknownKeyword => false,
                SyntaxKind::TypeLiteral => {
                    !matches!(program.node_property(*operand, "members"), Some(NodePropertyValue::List { nodes, .. }) if nodes.is_empty())
                }
                _ => true,
            })
            .count()
    }

    /// Whether a tuple type spreads anything but an array (`[...A, B]` for a
    /// tuple `A`).
    fn spreads_tuple(&self, tuple: NodeId) -> bool {
        let program = &self.program;
        let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(tuple, "elements") else { return false };
        nodes.into_iter().any(|element| {
            program.node_kind(element) == SyntaxKind::RestType
                && self
                    .node_child(element, "type")
                    .is_some_and(|inner| program.node_kind(self.unwrap_type_node(inner)) != SyntaxKind::ArrayType)
        })
    }

    fn unwrap_type_node(&self, mut node: NodeId) -> NodeId {
        let program = &self.program;
        while program.node_kind(node) == SyntaxKind::ParenthesizedType
            || (program.node_kind(node) == SyntaxKind::TypeOperator && program.node_operator(node) == Some(SyntaxKind::ReadonlyKeyword))
        {
            match self.node_child(node, "type") {
                Some(inner) => node = inner,
                None => break,
            }
        }
        node
    }

    /// A declaration inside a function or block rather than a file or
    /// namespace.
    fn is_local_declaration(&self, declaration: NodeId) -> bool {
        let mut current = self.program.node_parent(declaration);
        while let Some(owner) = current {
            match self.program.node_kind(owner) {
                SyntaxKind::SourceFile | SyntaxKind::ModuleBlock => return false,
                SyntaxKind::Block => return true,
                _ => current = self.program.node_parent(owner),
            }
        }
        false
    }

    /// `typeof Entity` of a class, enum, namespace or expando value: that
    /// value's own type, which tsc names by the entity.
    /// The type of the value a `typeof` names by a plain identifier (a
    /// parameter or binding of the same signature), not while that value's
    /// own type is being computed.
    fn queried_value_type(&self, name: NodeId) -> Option<TypeId> {
        thread_local! {
            static QUERYING: std::cell::RefCell<Vec<SymbolId>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        if self.program.node_kind(name) != SyntaxKind::Identifier {
            return None;
        }
        let symbol = self.symbol_at_location(name)?;
        if self.symbol_flags(symbol) & (ts_sf::FunctionScopedVariable | ts_sf::BlockScopedVariable) == 0
            || QUERYING.with(|querying| querying.borrow().contains(&symbol))
        {
            return None;
        }
        QUERYING.with(|querying| querying.borrow_mut().push(symbol));
        let ty = self.type_of_symbol(symbol);
        QUERYING.with(|querying| querying.borrow_mut().retain(|queried| *queried != symbol));
        (!matches!(self.ty(ty), Type::Any | Type::ErrorType | Type::Unknown)).then_some(ty)
    }

    fn queried_entity_type(&self, query: NodeId) -> Option<TypeId> {
        if self.node_property(query, "typeArguments").is_some_and(|arguments| matches!(arguments, NodePropertyValue::List { nodes, .. } if !nodes.is_empty())) {
            return None;
        }
        let symbol = self.symbol_at_location(self.node_child(query, "exprName")?)?;
        let target = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
        self.is_typeof_named(target).then(|| self.type_of_symbol(symbol))
    }

    fn node_property(&self, node: NodeId, property: &str) -> Option<NodePropertyValue> {
        self.program.node_property(node, property)
    }

    /// Whether the type `node` writes is tsc's error type: a reference that
    /// names nothing, or a union or intersection with one among its operands.
    fn names_unresolved_type(&self, node: NodeId) -> bool {
        let program = &self.program;
        let node = self.unwrap_type_node(node);
        match program.node_kind(node) {
            SyntaxKind::TypeReference => self
                .node_child(node, "typeName")
                .is_none_or(|name| self.symbol_at_location(name).is_none_or(|symbol| self.is_unresolved_symbol(symbol))),
            SyntaxKind::UnionType | SyntaxKind::IntersectionType => match program.node_property(node, "types") {
                Some(NodePropertyValue::List { nodes, .. }) => nodes.into_iter().any(|operand| self.names_unresolved_type(operand)),
                _ => false,
            },
            _ => false,
        }
    }

    /// The entity name a type-position name belongs to, as written: `A.B`
    /// for either of its identifiers.
    fn written_entity_name(&self, name: NodeId) -> Option<String> {
        let mut entity = name;
        while let Some(parent) = self.program.node_parent(entity)
            && self.program.node_kind(parent) == SyntaxKind::QualifiedName
            && self.node_child(parent, "right") == Some(entity)
        {
            entity = parent;
        }
        matches!(self.program.node_kind(entity), SyntaxKind::Identifier | SyntaxKind::QualifiedName)
            .then(|| self.program.node_text(entity).to_string())
    }

    /// The checker's namespace member prefixes for code written at `node`:
    /// each enclosing namespace's dotted path, outermost first.
    fn namespace_scope(&self, node: NodeId) -> Vec<String> {
        let program = &self.program;
        let mut names = Vec::new();
        let mut current = program.node_parent(node);
        while let Some(owner) = current {
            if program.node_kind(owner) == SyntaxKind::ModuleDeclaration {
                if program.node_flags(owner) & super::enums_generated::node_flags::GlobalAugmentation != 0 {
                    break;
                }
                match self.node_child(owner, "name") {
                    Some(name) if program.node_kind(name) == SyntaxKind::Identifier => {
                        names.push(program.node_text(name).to_string());
                    }
                    // Inside `declare module "m"` the checker keeps no prefix.
                    _ => break,
                }
            }
            current = program.node_parent(owner);
        }
        names.reverse();
        let mut prefixes: Vec<String> = Vec::with_capacity(names.len());
        for name in names {
            let prefix = match prefixes.last() {
                Some(outer) => format!("{outer}.{name}"),
                None => name,
            };
            prefixes.push(prefix);
        }
        prefixes
    }

    /// The dotted path naming a declaration from its file's scope (its own
    /// name behind the namespaces it is declared in), with the outermost of
    /// them. `None` inside an ambient module (`declare module "m"`), which no
    /// path from the file reaches.
    fn namespace_path(&self, symbol: SymbolId) -> Option<(String, SymbolId)> {
        let mut path = vec![self.symbol_name(symbol)];
        let mut current = symbol;
        while let Some(parent) = self.symbol_parent(current) {
            let declaration = *self.symbol_declarations(parent).first()?;
            match self.program.node_kind(declaration) {
                SyntaxKind::SourceFile => break,
                // `declare global { … }` declares globals.
                SyntaxKind::ModuleDeclaration
                    if self.program.node_flags(declaration) & super::enums_generated::node_flags::GlobalAugmentation != 0 =>
                {
                    break;
                }
                SyntaxKind::ModuleDeclaration => {
                    let name = self.node_child(declaration, "name")?;
                    if self.program.node_kind(name) != SyntaxKind::Identifier {
                        return None;
                    }
                    path.push(self.program.node_text(name).to_string());
                }
                _ => return None,
            }
            current = parent;
        }
        path.reverse();
        Some((path.join("."), current))
    }

    fn declared_type_parameter_names(&self, symbol: SymbolId) -> Vec<String> {
        let (file, symbol) = match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => (file, symbol),
            SymbolData::Global { parts, .. } => match parts.first() {
                Some(&(file, symbol)) => (file, symbol),
                None => return Vec::new(),
            },
            SymbolData::Transient { .. } => return Vec::new(),
        };
        let syntax = self.program.syntax(self.program.file_id(file)).clone();
        let Some(&declaration) = syntax.bound.symbols[symbol].declarations.first() else { return Vec::new() };
        syntax
            .tree
            .node(declaration)
            .type_parameters
            .as_ref()
            .map(|list| {
                list.nodes
                    .iter()
                    .filter_map(|&parameter| syntax.tree.node(parameter).name.map(|name| syntax.tree.text(name).to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `Symbol.name`.
    pub fn symbol_name(&self, symbol: SymbolId) -> String {
        let symbol_id = symbol;
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                let name = self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].name.clone();
                if name == "__module" {
                    return format!("\"{}\"", strip_extension(self.program.file_name(self.program.file_id(file))));
                }
                if name == "__missing"
                    && let Some(parameter) = self.binding_parameter_name(symbol_id)
                {
                    return parameter;
                }
                // tsc's `symbolName` names a private member (`__#1@#x`) as
                // written.
                match private_member_name(&name) {
                    Some(written) => written.to_string(),
                    None => name,
                }
            }
            SymbolData::Global { name, .. } | SymbolData::Transient { name, .. } => name.to_string(),
        }
    }

    /// `Symbol.escapedName`: the name with a leading `__` escaped as tsc's
    /// `escapeLeadingUnderscores` does, internal names as they are.
    pub fn symbol_escaped_name(&self, symbol: SymbolId) -> String {
        if let SymbolData::Bound { file, symbol } = self.symbol_data(symbol) {
            let name = &self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].name;
            if private_member_name(name).is_some() {
                return name.clone();
            }
        }
        let name = self.symbol_name(symbol);
        let internal = matches!(
            name.as_str(),
            "__call" | "__constructor" | "__new" | "__index" | "__export" | "__global" | "__missing" | "__type"
                | "__object" | "__jsxAttributes" | "__class" | "__function" | "__computed"
        );
        // A destructured parameter's `__0` is the binder's own name.
        let parameter = name.len() > 2 && name.bytes().skip(2).all(|byte| byte.is_ascii_digit()) && self.binding_parameter_name(symbol).is_some();
        if name.starts_with("__") && !internal && !parameter { format!("_{name}") } else { name }
    }

    /// `Symbol.flags`, as TypeScript's `SymbolFlags`.
    pub fn symbol_flags(&self, symbol: SymbolId) -> i32 {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].flags as i32
            }
            SymbolData::Global { parts, .. } => parts
                .iter()
                .map(|&(file, symbol)| self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].flags as i32)
                .fold(0, |all, flags| all | flags),
            SymbolData::Transient { flags, .. } => flags | ts_sf::Transient,
        }
    }

    /// `Symbol.declarations`.
    pub fn symbol_declarations(&self, symbol: SymbolId) -> Vec<NodeId> {
        let program = symbol.program;
        let parts = match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => vec![(file, symbol)],
            SymbolData::Global { parts, .. } => parts,
            SymbolData::Transient { declarations, .. } => return declarations,
        };
        parts
            .into_iter()
            .flat_map(|(file, symbol)| {
                self.program.syntax(self.program.file_id(file)).bound.symbols[symbol]
                    .declarations
                    .iter()
                    .map(move |&node| NodeId { program, file, node })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// `Symbol.valueDeclaration`.
    pub fn symbol_value_declaration(&self, symbol: SymbolId) -> Option<NodeId> {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol: index } => {
                let node = self.program.syntax(self.program.file_id(file)).bound.symbols[index].value_declaration?;
                Some(NodeId { program: symbol.program, file, node })
            }
            SymbolData::Global { parts, .. } => parts.iter().find_map(|&(file, index)| {
                let node = self.program.syntax(self.program.file_id(file)).bound.symbols[index].value_declaration?;
                Some(NodeId { program: symbol.program, file, node })
            }),
            // tsc's union property keeps a value declaration only when every
            // constituent's is the same one.
            SymbolData::Transient { declarations, .. } => {
                declarations.first().copied().filter(|first| declarations.iter().all(|declaration| declaration == first))
            }
        }
    }

    /// `Symbol.parent`: the symbol whose exports or members hold this one.
    pub fn symbol_parent(&self, symbol: SymbolId) -> Option<SymbolId> {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                let parent = self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].parent?;
                Some(self.bound_symbol(file, parent))
            }
            SymbolData::Transient { parent, .. } => parent,
            SymbolData::Global { .. } => None,
        }
    }

    fn table_symbols(&self, symbol: SymbolId, members: bool) -> Vec<SymbolId> {
        let parts = match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => vec![(file, symbol)],
            SymbolData::Global { parts, .. } => parts,
            SymbolData::Transient { .. } => return Vec::new(),
        };
        let mut out = Vec::new();
        for (file, index) in parts {
            let syntax = self.program.syntax(self.program.file_id(file)).clone();
            let table = if members { syntax.bound.members.get(&index) } else { syntax.bound.exports.get(&index) };
            let Some(table) = table else { continue };
            let mut entries: Vec<(&String, &usize)> = table.iter().collect();
            // The order tsc binds them in: a block's function declarations
            // first (`bindEachFunctionsFirst`), then everything in source order.
            let hoisted = |declaration: Option<u32>| {
                !members
                    && declaration.is_some_and(|declaration| {
                        syntax.tree.kind(declaration) == surge_ts_tsc_syntax::Kind::FunctionDeclaration
                            && syntax.tree.parent(declaration).is_some_and(|parent| {
                                matches!(
                                    syntax.tree.kind(parent),
                                    surge_ts_tsc_syntax::Kind::SourceFile
                                        | surge_ts_tsc_syntax::Kind::Block
                                        | surge_ts_tsc_syntax::Kind::ModuleBlock
                                )
                            })
                    })
            };
            entries.sort_by_key(|(_, member)| {
                let declaration = syntax.bound.symbols[**member].declarations.first().copied();
                (!hoisted(declaration), declaration)
            });
            out.extend(entries.into_iter().map(|(_, &member)| self.bound_symbol(file, member)));
        }
        out
    }

    /// `Symbol.members`, in declaration order.
    pub fn symbol_members(&self, symbol: SymbolId) -> Vec<SymbolId> {
        self.table_symbols(symbol, true)
    }

    /// `Symbol.exports`, in declaration order.
    pub fn symbol_exports(&self, symbol: SymbolId) -> Vec<SymbolId> {
        self.table_symbols(symbol, false)
    }

    fn export_of_symbol(&self, symbol: SymbolId, name: &str) -> Option<SymbolId> {
        // `typeof globalThis` has the globals a script declares with `var`
        // and `function`, not its block-scoped ones (tsc's `varsOnly`).
        if self.is_global_this_symbol(symbol) {
            return self.global_symbol(name, Meaning::Value).filter(|global| self.symbol_flags(*global) & ts_sf::BlockScoped == 0);
        }
        let symbol = self.aliased_symbol(symbol).unwrap_or(symbol);
        self.exports_of_module(symbol).into_iter().find(|export| self.symbol_name(*export) == name)
    }

    /// `symbolToString`: the symbol's name.
    pub fn symbol_to_string(&self, symbol: SymbolId) -> String {
        self.symbol_name(symbol)
    }

    // --- modules and aliases -------------------------------------------------

    fn module_symbol(&self, file: super::handles::SourceFileId) -> Option<SymbolId> {
        let syntax = self.program.syntax(file).clone();
        let &symbol = syntax.bound.node_symbol.get(&syntax.tree.root())?;
        Some(self.bound_symbol(file.index, symbol))
    }

    fn module_symbol_of_specifier(&self, specifier: NodeId) -> Option<SymbolId> {
        let parent = self.program.node_parent(specifier)?;
        let parent_kind = self.program.node_kind(parent);
        let is_specifier = match parent_kind {
            SyntaxKind::ImportDeclaration | SyntaxKind::ExportDeclaration | SyntaxKind::JSDocImportTag => {
                self.node_child(parent, "moduleSpecifier") == Some(specifier)
            }
            SyntaxKind::ExternalModuleReference => true,
            SyntaxKind::LiteralType => self.program.node_parent(parent).is_some_and(|import| self.program.node_kind(import) == SyntaxKind::ImportType),
            SyntaxKind::ModuleDeclaration => {
                return self.declared_symbol(parent).or_else(|| self.ambient_module_symbol(specifier));
            }
            _ => false,
        };
        if !is_specifier {
            return None;
        }
        // `resolveExternalModule` tries an ambient `declare module "name"`
        // before the file the name resolves to; one written in a module file
        // is an augmentation of that file, not an ambient module.
        let ambient = self.ambient_module_symbol(specifier).filter(|&ambient| {
            self.symbol_declarations(ambient)
                .iter()
                .any(|declaration| !self.program.is_external_module(declaration.source_file()))
        });
        if ambient.is_some() {
            return ambient;
        }
        let text = self.program.node_value_text(specifier)?;
        match self.resolved_module_file(specifier.source_file(), text) {
            Some(target) => self.module_symbol(target),
            None => self.ambient_module_symbol(specifier),
        }
    }

    /// The `declare module "name"` a specifier names, merged across files. A
    /// relative name never names one.
    fn ambient_module_symbol(&self, specifier: NodeId) -> Option<SymbolId> {
        let text = self.program.node_value_text(specifier)?;
        if text.starts_with('.') || text.starts_with('/') {
            return None;
        }
        self.global_symbol(&format!("\"{text}\""), Meaning::Namespace)
            .or_else(|| self.global_symbol(&format!("\"{text}\""), Meaning::Value))
    }

    /// The file an import in `file` of `specifier` resolved to, as the
    /// program resolved it.
    pub(crate) fn resolved_module_file(&self, file: super::handles::SourceFileId, specifier: &str) -> Option<super::handles::SourceFileId> {
        let checker_index = self.program.file(file).checker_index?;
        let resolved = self.query(|query| query.resolved_module(checker_index, specifier)).flatten()?;
        self.program.source_file(&resolved)
    }

    /// `getExportsOfModule`: a module's exports, `export *` re-exports
    /// included; a module whose `export =` names a namespace exports that
    /// namespace's (`resolveExternalModuleSymbol`).
    pub fn exports_of_module(&self, module: SymbolId) -> Vec<SymbolId> {
        let exports = self.own_module_exports(module);
        if let Some(assignment) = exports.iter().copied().find(|export| self.symbol_name(*export) == "export=")
            && let Some(assigned) = self.aliased_symbol(assignment)
            && assigned != module
            && self.symbol_flags(assigned) & (ts_sf::ValueModule | ts_sf::NamespaceModule) != 0
        {
            return self.own_module_exports(assigned);
        }
        exports
    }

    /// A module's own export table, `export *` re-exports included.
    fn own_module_exports(&self, module: SymbolId) -> Vec<SymbolId> {
        if let Some(exports) = self.with_tables(|tables| tables.module_exports.get(&module).cloned()) {
            return exports.to_vec();
        }
        let mut out: Vec<SymbolId> = Vec::new();
        let mut seen_names = std::collections::HashSet::new();
        let mut visited = std::collections::HashSet::new();
        self.collect_module_exports(module, &mut out, &mut seen_names, &mut visited);
        self.with_tables(|tables| tables.module_exports.insert(module, Arc::from(out.as_slice())));
        out
    }

    fn collect_module_exports(
        &self,
        module: SymbolId,
        out: &mut Vec<SymbolId>,
        seen_names: &mut std::collections::HashSet<String>,
        visited: &mut std::collections::HashSet<SymbolId>,
    ) {
        if !visited.insert(module) {
            return;
        }
        let exports = self.symbol_exports(module);
        let mut stars = Vec::new();
        for export in exports {
            let name = self.symbol_name(export);
            if name == "__export" {
                stars.push(export);
                continue;
            }
            if seen_names.insert(name) {
                out.push(export);
            }
        }
        for star in stars {
            for declaration in self.symbol_declarations(star) {
                let Some(specifier) = self.node_child(declaration, "moduleSpecifier") else { continue };
                // `export * from "m"` names an ambient `declare module "m"` too.
                if let Some(target_module) = self.module_symbol_of_specifier(specifier) {
                    let mut star_names = std::collections::HashSet::new();
                    let mut star_exports = Vec::new();
                    self.collect_module_exports(target_module, &mut star_exports, &mut star_names, visited);
                    for export in star_exports {
                        let name = self.symbol_name(export);
                        if name != "default" && seen_names.insert(name) {
                            out.push(export);
                        }
                    }
                }
            }
        }
    }

    /// `getAliasedSymbol`: what an import or export alias finally names. A
    /// symbol that is not an alias is its own target.
    pub fn aliased_symbol(&self, symbol: SymbolId) -> Option<SymbolId> {
        let mut current = symbol;
        for _ in 0..64 {
            match self.alias_target(current) {
                Some(next) if next != current => current = next,
                Some(_) => return Some(current),
                None => return (current != symbol).then_some(current),
            }
        }
        Some(current)
    }

    /// One step of alias resolution; `Some(symbol)` itself for a non-alias.
    pub(super) fn alias_target(&self, symbol: SymbolId) -> Option<SymbolId> {
        if self.symbol_flags(symbol) & ts_sf::Alias == 0 {
            return Some(symbol);
        }
        let program = &self.program;
        // `getDeclarationOfAliasSymbol`: the last alias declaration, where an
        // alias merges with a declaration of its own (`export type a`
        // beside `export { a } from`).
        let declarations = self.symbol_declarations(symbol);
        let declaration = declarations
            .iter()
            .rev()
            .find(|declaration| {
                matches!(
                    program.node_kind(**declaration),
                    SyntaxKind::ImportEqualsDeclaration
                        | SyntaxKind::NamespaceExportDeclaration
                        | SyntaxKind::ImportClause
                        | SyntaxKind::NamespaceImport
                        | SyntaxKind::NamespaceExport
                        | SyntaxKind::ImportSpecifier
                        | SyntaxKind::ExportSpecifier
                        | SyntaxKind::ExportAssignment
                )
            })
            .or(declarations.first())
            .copied()?;
        let kind = program.node_kind(declaration);
        // A file the program resolved the specifier to, or the ambient
        // `declare module "name"` it names.
        let module_of = |statement: NodeId| -> Option<SymbolId> {
            let specifier = self.node_child(statement, "moduleSpecifier")?;
            self.module_symbol_of_specifier(specifier)
        };
        let export_named = |module: SymbolId, name: &str| -> Option<SymbolId> {
            self.own_module_exports(module).into_iter().find(|export| self.symbol_name(*export) == name)
        };
        match kind {
            SyntaxKind::ImportClause => {
                let import = program.node_parent(declaration)?;
                let module = module_of(import)?;
                export_named(module, "default").or_else(|| export_named(module, "export="))
            }
            // A UMD global is the module of the file declaring it, or what
            // its `export =` names (`resolveExternalModuleSymbol`).
            SyntaxKind::NamespaceExportDeclaration => {
                thread_local! {
                    static RESOLVING: std::cell::RefCell<Vec<SymbolId>> = const { std::cell::RefCell::new(Vec::new()) };
                }
                // `export = N` naming the global back is circular
                // (`resolveAlias` answers the unknown symbol).
                if RESOLVING.with(|resolving| resolving.borrow().contains(&symbol)) {
                    return None;
                }
                RESOLVING.with(|resolving| resolving.borrow_mut().push(symbol));
                let module = self.module_symbol(declaration.source_file());
                let target = module.and_then(|module| match export_named(module, "export=") {
                    Some(assigned) => self.aliased_symbol(assigned).or(Some(assigned)),
                    None => Some(module),
                });
                RESOLVING.with(|resolving| resolving.borrow_mut().retain(|resolving| *resolving != symbol));
                target
            }
            // `import * as ns` of a module whose `export =` names an entity is
            // that entity (`resolveESModuleSymbol`).
            SyntaxKind::NamespaceImport => {
                let import = program.node_parent(program.node_parent(declaration)?)?;
                let module = module_of(import)?;
                export_named(module, "export=").or(Some(module))
            }
            SyntaxKind::ImportSpecifier => {
                let named = program.node_parent(declaration)?;
                let clause = program.node_parent(named)?;
                let import = program.node_parent(clause)?;
                let module = module_of(import)?;
                let imported = self.node_child(declaration, "propertyName").or_else(|| self.node_child(declaration, "name"))?;
                let imported = program.node_value_text(imported)?;
                // A module whose `export =` names a namespace exports that
                // namespace's members (`resolveExternalModuleSymbol`).
                export_named(module, imported).or_else(|| {
                    let assigned = self.aliased_symbol(export_named(module, "export=")?)?;
                    self.symbol_exports(assigned).into_iter().find(|export| self.symbol_name(*export) == imported)
                })
            }
            SyntaxKind::ExportSpecifier => {
                let named = program.node_parent(declaration)?;
                let export = program.node_parent(named)?;
                let local = self.node_child(declaration, "propertyName").or_else(|| self.node_child(declaration, "name"))?;
                let local_name = program.node_value_text(local)?.to_string();
                match module_of(export) {
                    Some(module) => export_named(module, &local_name),
                    None => self
                        .resolve_name(local, &local_name, Meaning::Value)
                        .or_else(|| self.resolve_name(local, &local_name, Meaning::Type))
                        .filter(|target| *target != symbol),
                }
            }
            SyntaxKind::ImportEqualsDeclaration => {
                let reference = self.node_child(declaration, "moduleReference")?;
                if program.node_kind(reference) == SyntaxKind::ExternalModuleReference {
                    let specifier = self.node_child(reference, "expression")?;
                    let module = self.module_symbol_of_specifier(specifier)?;
                    return export_named(module, "export=").or(Some(module));
                }
                self.symbol_at_location(reference)
            }
            SyntaxKind::NamespaceExport => {
                let export = program.node_parent(declaration)?;
                module_of(export)
            }
            SyntaxKind::ExportAssignment => {
                let expression = self.node_child(declaration, "expression")?;
                self.symbol_at_location(expression)
            }
            _ => None,
        }
    }

    // --- types ---------------------------------------------------------------

    /// `typeToString`.
    pub fn type_to_string(&self, ty: TypeId) -> String {
        if ty == self.unresolved_type() {
            return "/*unresolved*/ any".to_string();
        }
        if let Some(display) = self.with_tables(|tables| tables.error_displays.get(&ty.index).cloned()) {
            return display;
        }
        // The checker's type for a module's namespace object carries no
        // symbol; tsc names that type by its module.
        if let Some(origin) = self.origin(ty)
            && self.is_module_symbol(origin)
        {
            let name = self.symbol_name(origin);
            // `symbolToTypeNode`: under node16/nodenext an ESM file's module,
            // printed outside any file, says how it resolves.
            let esm = self.symbol_declarations(origin).first().is_some_and(|declaration| {
                self.program.node_kind(*declaration) == SyntaxKind::SourceFile
                    && self
                        .query(|query| query.is_esm_under_node_resolution(self.program.file_name(declaration.source_file())))
                        .unwrap_or(false)
            });
            let attributes = if esm { ", { with: { \"resolution-mode\": \"import\" } }" } else { "" };
            return format!("typeof import(\"{}\"{attributes})", name.trim_matches(|c| c == '"' || c == '\''));
        }
        // tsc names a class's, enum's or namespace's value by its symbol
        // (`createAnonymousTypeNode`), and so a function or object with
        // members of its own.
        if let Some(origin) = self.origin(ty)
            && self.is_typeof_named(origin)
            && self.with_tables(|tables| tables.type_by_key.get(&TypeKey::SymbolType(origin)).copied()) == Some(ty.index)
        {
            return format!("typeof {}", self.symbol_name_as_written(origin));
        }
        let value = self.ty(ty);
        let fresh = self
            .location(ty)
            .is_some_and(|node| self.program.node_kind(node) == SyntaxKind::ObjectLiteralExpression);
        // A cycle marker prints by the type of the declaration it reads back;
        // the ones a printing meets are resolved and it is printed again.
        let mut referents = std::collections::HashMap::new();
        let mut tried = std::collections::HashSet::new();
        let text = loop {
            let (text, missing) = surge_ts_types::with_cycle_referents(referents.clone(), || {
                surge_ts_types::with_fresh_cycle_root(fresh, || {
                    self.tsc_display(|| {
                        surge_ts_types::with_tsc_truncation(|| {
                            self.query(|query| {
                                let shown = query.shown_type(&value);
                                query.type_to_string(shown.as_ref().unwrap_or(&value))
                            })
                            .unwrap_or_else(|| value.name())
                        })
                    })
                })
            });
            let missing: Vec<String> = missing.into_iter().filter(|marker| tried.insert(marker.clone())).collect();
            if missing.is_empty() {
                break text;
            }
            for marker in missing {
                if let Some(referent) = self.cycle_referent(&marker) {
                    referents.insert(marker, self.ty(referent));
                }
            }
        };
        truncated(text)
    }

    /// Runs `render` printing types as `typeToString` does, unions ordered
    /// as this program's types would be created.
    fn tsc_display<R>(&self, render: impl FnOnce() -> R) -> R {
        let ranks = self.creation_ranks();
        surge_ts_types::with_creation_ranks(ranks, || surge_ts_types::with_tsc_display(render))
    }

    /// The order typescript@6 would create this program's literal and named
    /// types in, walking its files as a tool does: its sources, then its
    /// declaration files.
    fn creation_ranks(&self) -> Arc<surge_ts_types::CreationRanks> {
        if let Some(ranks) = self.with_tables(|tables| tables.creation_ranks.clone()) {
            return ranks;
        }
        let program = &self.program;
        let mut files = program.source_files();
        files.retain(|file| !program.is_source_file_default_library(*file));
        files.sort_by_key(|file| program.is_declaration_file(*file));
        let mut ranks = surge_ts_types::CreationRanks::default();
        for file in files {
            let mut pending = vec![(program.root(file), None, "")];
            while let Some((node, parent, property)) = pending.pop() {
                self.record_creation(node, parent, property, &mut ranks);
                let kind = program.node_kind(node);
                for (property, value) in program.node_properties(node).into_iter().rev() {
                    match value {
                        NodePropertyValue::Node(child) => pending.push((child, Some(kind), property)),
                        NodePropertyValue::List { nodes, .. } => {
                            pending.extend(nodes.into_iter().rev().map(|child| (child, Some(kind), property)))
                        }
                    }
                }
            }
        }
        let ranks = Arc::new(ranks);
        self.with_tables(|tables| tables.creation_ranks = Some(ranks.clone()));
        ranks
    }

    /// Records the type `node` creates when a tool asks for its type: a
    /// literal expression's or literal type's, and a name's. A property name
    /// or a module specifier creates none.
    fn record_creation(
        &self,
        node: NodeId,
        parent: Option<SyntaxKind>,
        property: &str,
        ranks: &mut surge_ts_types::CreationRanks,
    ) {
        let program = &self.program;
        let specifier = matches!(
            parent,
            Some(SyntaxKind::ImportDeclaration | SyntaxKind::ExportDeclaration | SyntaxKind::ExternalModuleReference)
        ) || parent == Some(SyntaxKind::ModuleDeclaration) && property == "name";
        let member_name = property == "name" && parent != Some(SyntaxKind::ModuleDeclaration);
        match program.node_kind(node) {
            SyntaxKind::Identifier => {
                if let Some(text) = program.node_value_text(node) {
                    ranks.record_name(text);
                }
            }
            SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral if !specifier && !member_name => {
                if let Some(text) = program.node_value_text(node) {
                    ranks.record_string(text, parent == Some(SyntaxKind::LiteralType));
                }
            }
            SyntaxKind::NumericLiteral if !member_name => {
                let Some(value) = program.node_value_text(node).and_then(|text| normalize_number(text).parse::<f64>().ok())
                else {
                    return;
                };
                let prefix = program.node_parent(node).filter(|_| parent == Some(SyntaxKind::PrefixUnaryExpression));
                let negated = prefix.and_then(|prefix| program.node_operator(prefix)) == Some(SyntaxKind::MinusToken);
                let in_type = match prefix {
                    Some(prefix) => program.node_parent(prefix).is_some_and(|outer| program.node_kind(outer) == SyntaxKind::LiteralType),
                    None => parent == Some(SyntaxKind::LiteralType),
                };
                ranks.record_number(if negated { -value } else { value }, in_type);
            }
            _ => {}
        }
    }

    /// A value symbol whose type tsc prints as `typeof Name`: a class, an
    /// enum, a namespace with values, and a function or variable given value
    /// members of its own.
    fn is_typeof_named(&self, symbol: SymbolId) -> bool {
        let flags = self.symbol_flags(symbol);
        // A variable's expando namespace (`const f = () => …; f.p = …`) is
        // the variable's; its type's symbol is the function it is
        // initialized with, which tsc writes as a literal.
        if flags & ts_sf::Variable != 0 && flags & ts_sf::Assignment != 0 && flags & (ts_sf::Class | ts_sf::Enum) == 0 {
            return false;
        }
        if flags & (ts_sf::Class | ts_sf::Enum | ts_sf::ValueModule) != 0 {
            return !self.is_module_symbol(symbol);
        }
        // An expando member alone makes no namespace: tsc writes the
        // function's type as a literal with it.
        flags & (ts_sf::Function | ts_sf::Variable) != 0
            && self.symbol_exports(symbol).into_iter().any(|export| {
                self.symbol_flags(export) & ts_sf::Value != 0 && self.symbol_flags(export) & ts_sf::Assignment == 0
            })
    }

    /// tsc's `getNameOfSymbolAsWritten`: the name its declaration writes (a
    /// default export's own name), `(Missing)` for a name the parser
    /// recovered.
    fn symbol_name_as_written(&self, symbol: SymbolId) -> String {
        for declaration in self.symbol_declarations(symbol) {
            if let Some(name) = self.node_child(declaration, "name") {
                let text = self.program.node_text(name);
                return if text.is_empty() { "(Missing)".to_string() } else { text.to_string() };
            }
        }
        self.symbol_name(symbol)
    }

    fn is_module_symbol(&self, symbol: SymbolId) -> bool {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                let syntax = self.program.syntax(self.program.file_id(file)).clone();
                let bound = &syntax.bound.symbols[symbol];
                bound.declarations.first().is_some_and(|&declaration| syntax.tree.kind(declaration) == Kind::SourceFile)
                    || is_ambient_module_name(&bound.name)
            }
            SymbolData::Global { name, .. } => is_ambient_module_name(&name),
            _ => false,
        }
    }

    /// `Type.flags`, as TypeScript's `TypeFlags`. Only flags surge models are
    /// ever set: a type surge could not resolve is `Any`, as tsc's error type.
    pub fn type_flags(&self, ty: TypeId) -> i32 {
        let value = self.ty(ty);
        self.query(|query| flags_of(query, &value)).unwrap_or_else(|| shallow_flags(&value))
    }

    /// `ObjectType.objectFlags`: `Reference` for arrays, tuples and generic
    /// instantiations, `Tuple` for tuples, `Anonymous` for object and
    /// function literals, `Interface`/`Class` for named object types.
    pub fn object_flags(&self, ty: TypeId) -> i32 {
        let value = self.ty(ty);
        match &value {
            Type::Array(_) => of::Reference,
            Type::Tuple(_) | Type::OpenTuple(_) => of::Reference | of::Tuple,
            Type::Function(_) => of::Anonymous,
            Type::Object(object) if object.alias_id.is_some() => of::Interface,
            Type::Object(_) => of::Anonymous,
            Type::Reference(reference) if !reference.arguments.is_empty() => of::Reference,
            Type::Reference(_) => match value.peeled() {
                Type::Object(object) if object.alias_id.is_some() => of::Interface,
                Type::Object(_) | Type::Function(_) => of::Anonymous,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// The literal a literal type holds (`LiteralType.value`).
    pub fn literal_value(&self, ty: TypeId) -> Option<LiteralValue> {
        // An enum member's literal type is a reference to its value.
        match self.ty(ty).peeled() {
            Type::StringLiteral(value) => Some(LiteralValue::String(value)),
            Type::NumberLiteral(value) => value.value.parse().ok().map(LiteralValue::Number),
            _ => None,
        }
    }

    /// `UnionOrIntersectionType.types`.
    pub fn type_constituents(&self, ty: TypeId) -> Vec<TypeId> {
        let value = self.ty(ty);
        let members = match value.peeled() {
            Type::Union(union) => union.types().to_vec(),
            Type::Boolean => vec![Type::BooleanLiteral(false), Type::BooleanLiteral(true)],
            Type::Object(object) if object.is_intersection => object.intersection_operands.as_deref().map(<[Type]>::to_vec).unwrap_or_default(),
            _ => Vec::new(),
        };
        members.into_iter().map(|member| self.intern_within(ty, member)).collect()
    }

    /// `Type.symbol`: the symbol of the declaration a named type came from.
    pub fn type_symbol(&self, ty: TypeId) -> Option<SymbolId> {
        let Some(symbol) = self.named_symbol(ty) else { return self.function_type_symbol(ty) };
        if self.symbol_flags(symbol) & ts_sf::TypeAlias == 0 {
            return Some(symbol);
        }
        // An alias names its type without being its symbol: an object type
        // written as a type literal has the literal's own (`__type`).
        let declaration = self
            .symbol_declarations(symbol)
            .into_iter()
            .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)?;
        let written = self.node_child(declaration, "type")?;
        if matches!(self.program.node_kind(written), SyntaxKind::TypeLiteral | SyntaxKind::MappedType) {
            self.declared_symbol(written)
        } else {
            None
        }
    }

    /// `Type.aliasSymbol`: the type alias a type was written through, when
    /// surge's type keeps one. An object type keeps its alias's declaration;
    /// a union keeps only the alias's name, resolved where the type was
    /// reached.
    pub fn alias_symbol(&self, ty: TypeId) -> Result<Option<SymbolId>, Unsupported> {
        let is_type_alias = |symbol: &SymbolId| self.symbol_flags(*symbol) & ts_sf::TypeAlias != 0;
        let value = self.ty(ty);
        let Type::Union(union) = &value else {
            return Ok(self.named_symbol(ty).filter(is_type_alias));
        };
        let Some(alias) = union.alias_name() else { return Ok(None) };
        let name = alias.split('<').next().unwrap_or(alias);
        let scope = self
            .origin(ty)
            .and_then(|origin| self.symbol_declarations(origin).first().copied())
            .or_else(|| self.location(ty));
        scope
            .and_then(|node| self.resolve_name(node, name, Meaning::Type))
            .or_else(|| self.global_symbol(name, Meaning::Type))
            .map(|symbol| if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol })
            .filter(is_type_alias)
            .map(Some)
            .ok_or(Unsupported("Type.aliasSymbol for a union type alias out of scope where the type was reached"))
    }

    /// The class, interface or type alias a type's declaration identity names.
    fn named_symbol(&self, ty: TypeId) -> Option<SymbolId> {
        let value = self.ty(ty);
        let id = match &value {
            Type::Reference(reference) => Some(reference.id.to_string()),
            Type::Object(object) => object.alias_id.as_deref().map(str::to_string),
            _ => None,
        }
        .or_else(|| match value.peeled() {
            Type::Object(object) => object.alias_id.as_deref().map(str::to_string),
            _ => None,
        });
        let Some(id) = id else {
            // An interface instantiated and expanded keeps only the name it
            // was written with (`Failure<{ code: number; }>`); it names the
            // interface where the type was reached.
            let Type::Object(object) = value.peeled() else { return None };
            let written = object.alias_name.as_deref()?;
            let name = written.split('<').next().unwrap_or(written);
            let scope = self.reached_at(ty)?;
            return self
                .resolve_name(scope, name, Meaning::Type)
                .map(|symbol| if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol })
                .filter(|symbol| self.symbol_flags(*symbol) & (ts_sf::Interface | ts_sf::Class) != 0);
        };
        let (file_name, name) = id.split_once('\0')?;
        let file = self.program.source_file(file_name)?;
        // A class declared in a body is identified by its name and position
        // (`Counter@466`) and named where its type was reached (`new
        // Counter()` inside the function declaring it).
        if let Some((local, _)) = name.split_once('@') {
            return self.reached_at(ty).and_then(|node| self.resolve_name(node, local, Meaning::Type));
        }
        let root = self.program.root(file);
        self.resolve_name(root, name, Meaning::Type).or_else(|| self.global_symbol(name, Meaning::Type))
    }

    /// `getPropertiesOfType`.
    pub fn properties_of_type(&self, ty: TypeId) -> Vec<SymbolId> {
        let value = self.ty(ty);
        let properties = self.query(|query| query.properties(&value)).unwrap_or_default();
        properties.into_iter().map(|property| self.property_symbol(ty, property)).collect()
    }

    /// `getPropertyOfType`.
    pub fn property_of_type(&self, ty: TypeId, name: &str) -> Option<SymbolId> {
        let value = self.ty(ty);
        let property = self.query(|query| query.property(&value, name)).flatten()?;
        Some(self.property_symbol(ty, property))
    }

    /// A class's or interface's member named `name`, its own or one its
    /// `extends` clauses inherit (`getPropertyOfType` over resolved bases).
    /// The static member `name` a class or the classes it extends declare.
    fn static_member_in_hierarchy(&self, class: SymbolId, name: &str, visited: &mut Vec<SymbolId>) -> Option<SymbolId> {
        if visited.contains(&class) || visited.len() > 32 {
            return None;
        }
        visited.push(class);
        if let Some(member) = self.symbol_exports(class).into_iter().find(|member| self.symbol_name(*member) == name) {
            return Some(member);
        }
        let program = &self.program;
        for declaration in self.symbol_declarations(class) {
            for clause in self.list_property(declaration, "heritageClauses") {
                if program.node_operator(clause) != Some(SyntaxKind::ExtendsKeyword) {
                    continue;
                }
                for base in self.list_property(clause, "types") {
                    let Some(base) = self
                        .node_child(base, "expression")
                        .and_then(|expression| self.symbol_at_location(expression))
                        .map(|base| if self.is_alias(base) { self.aliased_symbol(base).unwrap_or(base) } else { base })
                    else {
                        continue;
                    };
                    if let Some(member) = self.static_member_in_hierarchy(base, name, visited) {
                        return Some(member);
                    }
                }
            }
        }
        None
    }

    /// The symbol whose exports or statics a member read off `object` may be:
    /// what `object` names, or the class a class expression declares.
    fn member_container_symbol(&self, object: NodeId) -> Option<SymbolId> {
        let mut inner = object;
        while self.program.node_kind(inner) == SyntaxKind::ParenthesizedExpression {
            inner = self.node_child(inner, "expression")?;
        }
        match self.program.node_kind(inner) {
            SyntaxKind::ClassExpression => self.declared_symbol(inner),
            _ => self.symbol_at_location(object),
        }
    }

    /// Whether an expression whose symbol is a class reads the class itself,
    /// whose members are its statics: `this` and `super` do only in a static
    /// member, elsewhere they are an instance.
    fn reads_class_value(&self, object: NodeId) -> bool {
        let program = &self.program;
        if !matches!(program.node_kind(object), SyntaxKind::ThisKeyword | SyntaxKind::SuperKeyword) {
            return true;
        }
        let mut current = program.node_parent(object);
        while let Some(node) = current {
            match program.node_kind(node) {
                SyntaxKind::ClassStaticBlockDeclaration => return true,
                SyntaxKind::MethodDeclaration
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor
                | SyntaxKind::PropertyDeclaration
                | SyntaxKind::Constructor => return is_static_member(&program.syntax(node.source_file()).tree, node.node),
                SyntaxKind::FunctionDeclaration
                | SyntaxKind::FunctionExpression
                | SyntaxKind::ClassDeclaration
                | SyntaxKind::ClassExpression
                | SyntaxKind::SourceFile => return false,
                _ => current = program.node_parent(node),
            }
        }
        false
    }

    /// Whose expando members (`f.p = v`) its type has: a `function`'s, or an
    /// unannotated `const`'s a function initializes (`getSymbolOfExpando`
    /// takes no `let`, though the binder declares its members too).
    fn is_expando_owner(&self, symbol: SymbolId) -> bool {
        let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
        self.symbol_flags(symbol) & ts_sf::Function != 0
            || self.symbol_value_declaration(symbol).is_some_and(|declaration| {
                // A written type is the variable's type, expando or not.
                self.program.node_kind(declaration) == SyntaxKind::VariableDeclaration
                    && self.node_child(declaration, "type").is_none()
                    && self.program.node_parent(declaration).is_some_and(|list| {
                        self.program.node_flags(list) & super::enums_generated::node_flags::Const != 0
                    })
            })
    }

    /// The class whose instance an expression's one construct signature
    /// returns.
    fn constructed_class_symbol(&self, expression: NodeId) -> Option<SymbolId> {
        let ty = self.type_at_location(expression);
        let [signature] = self.signatures_of_type(ty, SignatureKind::Construct)[..] else { return None };
        self.type_symbol(self.return_type_of_signature(signature))
            .filter(|class| self.symbol_flags(*class) & (ts_sf::Class | ts_sf::Interface) != 0)
            .or_else(|| self.mixin_class_symbol(expression))
    }

    /// The class expression a mixin call returns (`Timestamped(Root)` for
    /// `function Timestamped(Base) { return class extends Base {…} }`), whose
    /// instance no name reaches.
    fn mixin_class_symbol(&self, expression: NodeId) -> Option<SymbolId> {
        if self.program.node_kind(expression) != SyntaxKind::CallExpression {
            return None;
        }
        let callee = self.symbol_at_location(self.node_child(expression, "expression")?)?;
        let function = self.symbol_value_declaration(callee)?;
        let mut returned = self.lone_return_expression(function)?;
        while self.program.node_kind(returned) == SyntaxKind::ParenthesizedExpression {
            returned = self.node_child(returned, "expression")?;
        }
        (self.program.node_kind(returned) == SyntaxKind::ClassExpression).then(|| self.declared_symbol(returned)).flatten()
    }

    fn member_in_hierarchy(&self, symbol: SymbolId, name: &str, visited: &mut Vec<SymbolId>) -> Option<SymbolId> {
        if visited.contains(&symbol) || visited.len() > 32 {
            return None;
        }
        visited.push(symbol);
        if let Some(member) = self
            .symbol_members(symbol)
            .into_iter()
            .chain(self.symbol_exports(symbol))
            .find(|member| {
                self.symbol_name(*member) == name
                    && (self.symbol_flags(*member) & ts_sf::Assignment == 0 || self.is_expando_owner(symbol))
            })
        {
            return Some(member);
        }
        if self.symbol_flags(symbol) & (ts_sf::Class | ts_sf::Interface) == 0 {
            return None;
        }
        let program = &self.program;
        for declaration in self.symbol_declarations(symbol) {
            let Some(NodePropertyValue::List { nodes: clauses, .. }) = program.node_property(declaration, "heritageClauses") else {
                continue;
            };
            for clause in clauses {
                if program.node_operator(clause) != Some(SyntaxKind::ExtendsKeyword) {
                    continue;
                }
                let Some(NodePropertyValue::List { nodes: bases, .. }) = program.node_property(clause, "types") else { continue };
                for base in bases {
                    let expression = self.node_child(base, "expression");
                    // A base the clause does not name (`extends Mixin(Root)`,
                    // `extends Base` for a constructor parameter) is the class
                    // its expression constructs.
                    let Some(base) = expression
                        .and_then(|expression| self.symbol_at_location(expression))
                        .map(|base| if self.is_alias(base) { self.aliased_symbol(base).unwrap_or(base) } else { base })
                        .filter(|base| self.symbol_flags(*base) & (ts_sf::Class | ts_sf::Interface) != 0)
                        .or_else(|| expression.and_then(|expression| self.constructed_class_symbol(expression)))
                    else {
                        continue;
                    };
                    if let Some(member) = self.member_in_hierarchy(base, name, visited) {
                        return Some(member);
                    }
                }
            }
        }
        None
    }

    /// The member named `name` of the type an expression's declaration
    /// writes, or of the object literal that initializes it: the symbol tsc
    /// keeps on an anonymous object type's property. `None` where the write
    /// does not settle it (a union several of whose members declare it).
    fn written_member_of_expression(&self, object: NodeId, name: &str, depth: u32) -> Option<SymbolId> {
        let program = &self.program;
        if depth > 8 {
            return None;
        }
        let mut object = object;
        loop {
            match program.node_kind(object) {
                SyntaxKind::ParenthesizedExpression | SyntaxKind::NonNullExpression | SyntaxKind::SatisfiesExpression => {}
                SyntaxKind::AsExpression | SyntaxKind::TypeAssertionExpression => {
                    let asserted = self.node_child(object, "type")?;
                    let is_const = program.node_kind(asserted) == SyntaxKind::TypeReference
                        && self.node_child(asserted, "typeName").and_then(|name| program.node_value_text(name)).as_deref() == Some("const");
                    if !is_const {
                        return self.written_member_of_type(asserted, name, depth + 1);
                    }
                }
                _ => break,
            }
            object = self.node_child(object, "expression")?;
        }
        match program.node_kind(object) {
            SyntaxKind::ObjectLiteralExpression => self.object_literal_member(object, name),
            SyntaxKind::ThisKeyword => {
                let receiver = self.this_receiver_type(object)?;
                self.member_in_hierarchy(self.type_symbol(receiver)?, name, &mut Vec::new())
            }
            SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression => {
                let symbol = self.symbol_at_location(object)?;
                let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
                // A member read through a mapping or an instantiation has a
                // type its declaration does not write.
                if matches!(self.symbol_data(symbol), SymbolData::Transient { .. }) {
                    return None;
                }
                let declaration = self.symbol_value_declaration(symbol)?;
                if !matches!(
                    program.node_kind(declaration),
                    SyntaxKind::VariableDeclaration
                        | SyntaxKind::Parameter
                        | SyntaxKind::PropertyDeclaration
                        | SyntaxKind::PropertySignature
                        | SyntaxKind::PropertyAssignment
                ) {
                    return None;
                }
                if let Some(annotation) = self.node_child(declaration, "type") {
                    return self.written_member_of_type(annotation, name, depth + 1);
                }
                let initializer = self.node_child(declaration, "initializer")?;
                self.written_member_of_expression(initializer, name, depth + 1)
            }
            // A call of a non-generic function returns what its declaration
            // writes it returns.
            SyntaxKind::CallExpression => {
                let callee = self.node_child(object, "expression")?;
                let symbol = self.symbol_at_location(callee)?;
                let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
                let declaration = self.symbol_value_declaration(symbol)?;
                if !matches!(
                    program.node_kind(declaration),
                    SyntaxKind::FunctionDeclaration | SyntaxKind::MethodDeclaration | SyntaxKind::MethodSignature
                ) || !self.list_property(declaration, "typeParameters").is_empty()
                {
                    return None;
                }
                match self.node_child(declaration, "type") {
                    Some(returns) => self.written_member_of_type(returns, name, depth + 1),
                    // An unannotated one returns what its lone `return` writes
                    // (`return { implementation };`).
                    None => self.written_member_of_expression(self.lone_return_expression(declaration)?, name, depth + 1),
                }
            }
            _ => None,
        }
    }

    /// The expression of a function's only `return`, its nested functions'
    /// own returns aside.
    pub(super) fn lone_return_expression(&self, function: NodeId) -> Option<NodeId> {
        let program = &self.program;
        let mut pending = vec![self.node_child(function, "body")?];
        let mut found = None;
        while let Some(node) = pending.pop() {
            match program.node_kind(node) {
                SyntaxKind::FunctionDeclaration
                | SyntaxKind::FunctionExpression
                | SyntaxKind::ArrowFunction
                | SyntaxKind::MethodDeclaration
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor
                | SyntaxKind::ClassDeclaration
                | SyntaxKind::ClassExpression => continue,
                SyntaxKind::ReturnStatement => {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(self.node_child(node, "expression")?);
                }
                _ => pending.extend(program.node_children(node)),
            }
        }
        found
    }

    /// The object literal a binding pattern destructures, written as its
    /// declaration's initializer or as the matching member of an outer
    /// pattern's literal.
    fn pattern_source_literal(&self, pattern: NodeId) -> Option<NodeId> {
        let program = &self.program;
        let parent = program.node_parent(pattern)?;
        let source = match program.node_kind(parent) {
            SyntaxKind::VariableDeclaration | SyntaxKind::Parameter => self.node_child(parent, "initializer")?,
            SyntaxKind::BindingElement => {
                let outer = self.pattern_source_literal(program.node_parent(parent)?)?;
                let key = self.node_child(parent, "propertyName").or_else(|| self.node_child(parent, "name"))?;
                let key = program.node_value_text(key)?;
                self.list_property(outer, "properties").into_iter().rev().find_map(|member| {
                    (program.node_kind(member) == SyntaxKind::PropertyAssignment
                        && self.node_child(member, "name").and_then(|name| program.node_value_text(name)) == Some(key))
                    .then(|| self.node_child(member, "initializer"))
                    .flatten()
                })?
            }
            _ => return None,
        };
        let mut source = source;
        while program.node_kind(source) == SyntaxKind::ParenthesizedExpression {
            source = self.node_child(source, "expression")?;
        }
        (program.node_kind(source) == SyntaxKind::ObjectLiteralExpression).then_some(source)
    }

    fn object_literal_member(&self, literal: NodeId, name: &str) -> Option<SymbolId> {
        let program = &self.program;
        let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(literal, "properties") else { return None };
        nodes.into_iter().rev().find_map(|member| {
            let member_name = self.node_child(member, "name")?;
            (program.node_value_text(member_name)? == name).then(|| self.declared_symbol(member)).flatten()
        })
    }

    fn typed_union_operands(&self, object: NodeId) -> Vec<(NodeId, Option<Type>)> {
        self.written_union_operands(object).into_iter().map(|node| (node, self.type_of_type_node(node))).collect()
    }

    /// The member `name` of the operand of `object`'s written union that
    /// `ty` is.
    fn written_union_member(&self, object: NodeId, ty: &Type, name: &str) -> Option<SymbolId> {
        let operands = self.typed_union_operands(object);
        if operands.len() < 2 {
            return None;
        }
        let (node, _) = operands.iter().find(|(_, operand)| operand.as_ref() == Some(ty))?;
        self.written_member_of_type(*node, name, 1)
    }

    /// `member` as `owner`'s `property` has it: read through an
    /// instantiation or a mapping, it keeps its declaration with the type and
    /// optionality the read gives it (tsc's instantiated and mapped symbols).
    fn member_as_read(&self, owner: TypeId, member: SymbolId, property: &PropertyInfo) -> SymbolId {
        let Some(declaration) = self.symbol_declarations(member).first().copied() else { return member };
        let read_type = self.property_type(property);
        let flags = self.symbol_flags(member);
        let optional = if property.optional { ts_sf::Optional } else { 0 };
        if flags & ts_sf::Optional == optional
            && self.has_readonly_modifier(declaration) == property.readonly
            && self.ty(self.type_of_symbol(member)) == read_type
        {
            return member;
        }
        let name = property.name.clone();
        let flags = (flags & !ts_sf::Optional) | optional;
        let ty = self.intern(read_type);
        let parent = self.symbol_parent(member);
        self.symbol_id(SymbolKey::Property(owner, name.clone()), || SymbolData::Transient {
            name,
            flags,
            ty,
            parent,
            declarations: vec![declaration],
        })
    }

    /// The object literal or type literal member `name` whose name the
    /// checker recorded at `site`.
    fn object_literal_member_at(&self, site: &surge_ts_types::PropertyDeclarationSite, name: &str) -> Option<SymbolId> {
        let program = &self.program;
        let file = program.source_file(&site.file)?;
        let (start, end) = (site.start as usize, site.end as usize);
        let mut node = program.root(file);
        'descend: loop {
            for child in program.node_children(node) {
                let (child_start, child_end) = program.node_byte_span(child);
                if (child_start, child_end) == (start, end) {
                    let declaration = program.node_parent(child)?;
                    let declares = matches!(
                        program.node_kind(declaration),
                        SyntaxKind::PropertyAssignment
                            | SyntaxKind::ShorthandPropertyAssignment
                            | SyntaxKind::MethodDeclaration
                            | SyntaxKind::GetAccessor
                            | SyntaxKind::SetAccessor
                            | SyntaxKind::PropertySignature
                            | SyntaxKind::MethodSignature
                    ) && self.node_child(declaration, "name") == Some(child);
                    let symbol = declares.then(|| self.declared_symbol(declaration)).flatten()?;
                    return (self.symbol_name(symbol) == name).then_some(symbol);
                }
                if child_start <= start && end <= child_end {
                    node = child;
                    continue 'descend;
                }
            }
            return None;
        }
    }

    /// The operands of the union the declaration of the reference `object`
    /// writes as its type (`m: M` with `type M = { … } | { … }`), through
    /// the aliases it names.
    fn written_union_operands(&self, object: NodeId) -> Vec<NodeId> {
        let program = &self.program;
        let mut object = object;
        while matches!(program.node_kind(object), SyntaxKind::ParenthesizedExpression | SyntaxKind::NonNullExpression) {
            match self.node_child(object, "expression") {
                Some(inner) => object = inner,
                None => return Vec::new(),
            }
        }
        // `o["k"]` reads what `o`'s written type declares `k` with, a
        // member or the index signature answering it.
        let symbol = match program.node_kind(object) {
            SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression => self.symbol_at_location(object),
            SyntaxKind::ElementAccessExpression => self
                .node_child(object, "argumentExpression")
                .filter(|key| matches!(program.node_kind(*key), SyntaxKind::StringLiteral | SyntaxKind::NumericLiteral))
                .and_then(|key| program.node_value_text(key).map(str::to_string))
                .zip(self.node_child(object, "expression"))
                .and_then(|(key, base)| {
                    self.written_member_of_expression(base, &key, 0)
                        .or_else(|| self.written_member_of_expression(base, "__index", 0))
                }),
            _ => None,
        };
        let Some(symbol) = symbol else { return Vec::new() };
        let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
        // An index signature's symbol declares no value.
        let Some(declaration) = self.symbol_value_declaration(symbol).or_else(|| self.symbol_declarations(symbol).first().copied())
        else {
            return Vec::new();
        };
        if !matches!(
            program.node_kind(declaration),
            SyntaxKind::VariableDeclaration
                | SyntaxKind::Parameter
                | SyntaxKind::PropertyDeclaration
                | SyntaxKind::PropertySignature
                | SyntaxKind::IndexSignature
        ) {
            return Vec::new();
        }
        let Some(annotation) = self.node_child(declaration, "type") else { return Vec::new() };
        let mut operands = Vec::new();
        self.push_union_operands(annotation, &mut operands, 0);
        operands
    }

    fn push_union_operands(&self, node: NodeId, operands: &mut Vec<NodeId>, depth: u32) {
        let program = &self.program;
        if depth > 8 {
            return;
        }
        let node = self.unwrap_type_node(node);
        match program.node_kind(node) {
            SyntaxKind::UnionType => {
                if let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(node, "types") {
                    for operand in nodes {
                        self.push_union_operands(operand, operands, depth + 1);
                    }
                }
            }
            SyntaxKind::TypeReference if self.list_property(node, "typeArguments").is_empty() => {
                let alias = self
                    .node_child(node, "typeName")
                    .and_then(|name| self.symbol_at_location(name))
                    .map(|target| if self.is_alias(target) { self.aliased_symbol(target).unwrap_or(target) } else { target })
                    .filter(|target| self.symbol_flags(*target) & ts_sf::TypeAlias != 0)
                    .and_then(|target| {
                        self.symbol_declarations(target)
                            .into_iter()
                            .find(|declaration| program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
                    })
                    .and_then(|declaration| self.node_child(declaration, "type"));
                match alias {
                    Some(aliased) => self.push_union_operands(aliased, operands, depth + 1),
                    None => operands.push(node),
                }
            }
            _ => operands.push(node),
        }
    }

    fn written_member_of_type(&self, node: NodeId, name: &str, depth: u32) -> Option<SymbolId> {
        let program = &self.program;
        if depth > 8 {
            return None;
        }
        let node = self.unwrap_type_node(node);
        match program.node_kind(node) {
            SyntaxKind::TypeLiteral => {
                let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(node, "members") else { return None };
                nodes.into_iter().find_map(|member| {
                    // An index signature is the literal's `__index`.
                    if name == "__index" {
                        return (program.node_kind(member) == SyntaxKind::IndexSignature)
                            .then(|| self.declared_symbol(member))
                            .flatten();
                    }
                    let member_name = self.node_child(member, "name")?;
                    // A computed name: a well-known symbol's
                    // (`[Symbol.hasInstance]`), or a literal key's.
                    let written = if program.node_kind(member_name) == SyntaxKind::ComputedPropertyName {
                        let key = self.node_child(member_name, "expression")?;
                        if program.node_kind(key) == SyntaxKind::PropertyAccessExpression {
                            format!("[{}]", program.node_text(key))
                        } else {
                            self.computed_member_name(member)?
                        }
                    } else {
                        program.node_value_text(member_name)?.to_string()
                    };
                    (written == name).then(|| self.declared_symbol(member)).flatten()
                })
            }
            SyntaxKind::TypeReference => {
                let target = self.symbol_at_location(self.node_child(node, "typeName")?)?;
                let target = if self.is_alias(target) { self.aliased_symbol(target).unwrap_or(target) } else { target };
                if self.symbol_flags(target) & ts_sf::TypeAlias != 0 {
                    let declaration = self
                        .symbol_declarations(target)
                        .into_iter()
                        .find(|declaration| program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)?;
                    return self.written_member_of_type(self.node_child(declaration, "type")?, name, depth + 1);
                }
                self.member_in_hierarchy(target, name, &mut Vec::new())
            }
            SyntaxKind::IntersectionType | SyntaxKind::UnionType => {
                let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(node, "types") else { return None };
                let found: Vec<SymbolId> =
                    nodes.into_iter().filter_map(|operand| self.written_member_of_type(operand, name, depth + 1)).collect();
                match found.as_slice() {
                    [only] => Some(*only),
                    [first, ..] if program.node_kind(node) == SyntaxKind::IntersectionType => Some(*first),
                    _ => None,
                }
            }
            // `typeof value`: what the value's declaration writes.
            SyntaxKind::TypeQuery => {
                let entity = self.node_child(node, "exprName")?;
                if program.node_kind(entity) != SyntaxKind::Identifier {
                    return None;
                }
                self.written_member_of_expression(entity, name, depth + 1)
            }
            // `T["k"]`: what the member `k` of `T` is declared with.
            SyntaxKind::IndexedAccessType => {
                let index = self.node_child(node, "indexType")?;
                if program.node_kind(index) != SyntaxKind::LiteralType {
                    return None;
                }
                let key = program.node_value_text(self.node_child(index, "literal")?)?.to_string();
                let member = self.written_member_of_type(self.node_child(node, "objectType")?, &key, depth + 1)?;
                let declaration = self.symbol_value_declaration(member)?;
                match self.node_child(declaration, "type") {
                    Some(annotation) => self.written_member_of_type(annotation, name, depth + 1),
                    None => self.written_member_of_expression(self.node_child(declaration, "initializer")?, name, depth + 1),
                }
            }
            _ => None,
        }
    }

    /// A property's type as `getTypeOfSymbol` gives it: an optional
    /// property's includes `undefined` under `strictNullChecks`.
    fn property_type(&self, property: &PropertyInfo) -> Type {
        if property.optional && self.program.inner().strict_null_checks {
            surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        }
    }

    fn property_symbol(&self, owner: TypeId, property: PropertyInfo) -> SymbolId {
        self.property_symbol_written(owner, property, None)
    }

    /// [`Self::property_symbol`] read off `object`, whose written type or
    /// initializer can declare a member its type keeps no symbol for.
    fn property_symbol_written(&self, owner: TypeId, property: PropertyInfo, object: Option<NodeId>) -> SymbolId {
        let declared_by_type = self
            .declaring_symbol(owner)
            .and_then(|symbol| self.member_in_hierarchy(symbol, &property.name, &mut Vec::new()));
        let declared = declared_by_type
            .or_else(|| object.and_then(|object| self.written_member_of_expression(object, &property.name, 0)));
        if let Some(declared) = declared {
            // A generic declaration's member is read through its
            // instantiation: tsc's instantiated symbol keeps the member's
            // declaration with the instantiated type.
            let declaration = self.symbol_declarations(declared).first().copied();
            if let Some(declaration) = declaration
                && !self.type_parameters_in_scope(declaration).is_empty()
            {
                let name = property.name.clone();
                let flags = self.symbol_flags(declared);
                let ty = self.intern(self.property_type(&property));
                let parent = self.symbol_parent(declared);
                return self.symbol_id(SymbolKey::Property(owner, name.clone()), || SymbolData::Transient {
                    name,
                    flags,
                    ty,
                    parent,
                    declarations: vec![declaration],
                });
            }
            return declared;
        }
        if let Some(symbol) = self.union_or_intersection_property(owner, &property, object) {
            return symbol;
        }
        // A member an object literal declares (an inferred return, an
        // argument a generic call infers from).
        if let Some(member) =
            property.declaration.as_deref().and_then(|site| self.object_literal_member_at(site, &property.name))
        {
            return self.member_as_read(owner, member, &property);
        }
        // An anonymous object narrowed out of a union is that union's
        // written member.
        if let Some(object) = object
            && let Some(member) = self.written_union_member(object, &self.ty(owner), &property.name)
        {
            return member;
        }
        let name = property.name.clone();
        let mut flags = if property.method { ts_sf::Method } else { ts_sf::Property };
        if property.optional {
            flags |= ts_sf::Optional;
        }
        let ty = self.intern(self.property_type(&property));
        let parent = self.type_symbol(owner);
        // A member typed by its own `unique symbol` is that one member however
        // its object is reached (`LuaTuple`'s `_nominal_LuaTuple` on every
        // instantiation), as tsc's single declared symbol is.
        let key = match &property.ty {
            Type::Reference(reference) if reference.is_unique_symbol() => {
                SymbolKey::Global(Arc::from(format!("\0unique-member\0{}\0{name}", reference.id)))
            }
            _ => SymbolKey::Property(owner, name.clone()),
        };
        self.symbol_id(key, || SymbolData::Transient {
            name,
            flags,
            ty,
            parent,
            declarations: Vec::new(),
        })
    }

    /// The class or interface declaring a type's members: its own symbol, or
    /// the global interface tsc's `getApparentType` reads a primitive's or an
    /// array's members from.
    fn declaring_symbol(&self, owner: TypeId) -> Option<SymbolId> {
        if let Some(symbol) = self.type_symbol(owner) {
            return Some(symbol);
        }
        let flags = self.type_flags(owner);
        let name = if flags & tf::StringLike != 0 {
            "String"
        } else if flags & tf::NumberLike != 0 {
            "Number"
        } else if flags & tf::BooleanLike != 0 {
            "Boolean"
        } else if flags & tf::BigIntLike != 0 {
            "BigInt"
        } else if flags & tf::ESSymbolLike != 0 {
            "Symbol"
        } else {
            match self.ty(owner) {
                Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => "Array",
                Type::Reference(reference) if reference.is_readonly_array() => "ReadonlyArray",
                _ => return None,
            }
        };
        self.global_symbol(name, Meaning::Type)
    }

    /// tsc's `createUnionOrIntersectionProperty`: a property one symbol
    /// gives every constituent declaring it is that symbol; several make a
    /// synthetic property carrying each one's declarations.
    fn union_or_intersection_property(&self, owner: TypeId, property: &PropertyInfo, object: Option<NodeId>) -> Option<SymbolId> {
        let is_union = self.type_flags(owner) & tf::Union != 0;
        if !is_union && self.type_flags(owner) & tf::Intersection == 0 {
            return None;
        }
        let mut found: Vec<SymbolId> = Vec::new();
        let mut operands: Option<Vec<(NodeId, Option<Type>)>> = None;
        for constituent in self.type_constituents(owner) {
            let Some(mut symbol) = self.property_of_type(constituent, &property.name) else { continue };
            // A member of an anonymous constituent is declared in the type
            // literal written for it in the union the reference declares.
            if is_union
                && let Some(object) = object
                && self.symbol_declarations(symbol).is_empty()
            {
                let operands = operands.get_or_insert_with(|| self.typed_union_operands(object));
                let constituent_type = self.ty(constituent);
                if let Some((node, _)) = operands.iter().find(|(_, ty)| ty.as_ref().is_some_and(|ty| *ty == constituent_type))
                    && let Some(member) = self.written_member_of_type(*node, &property.name, 1)
                {
                    symbol = member;
                }
            }
            if !found.contains(&symbol) {
                found.push(symbol);
            }
        }
        if let [only] = found.as_slice() {
            return Some(*only);
        }
        // Every constituent reading the one declaration as the same type is
        // reading that declaration's own symbol, as tsc's union property
        // returns a lone `singleProp` (`LuaTuple`'s `_nominal_LuaTuple`).
        if let Some(first) = found.first()
            && let [declaration] = self.symbol_declarations(*first).as_slice()
            && let first_type = self.ty(self.type_of_symbol(*first))
            && found.iter().all(|symbol| {
                self.symbol_declarations(*symbol).as_slice() == [*declaration]
                    && self.ty(self.type_of_symbol(*symbol)) == first_type
            })
            && let Some(declared) = self.declared_symbol(*declaration)
        {
            return Some(declared);
        }
        let declarations: Vec<NodeId> = found.iter().flat_map(|symbol| self.symbol_declarations(*symbol)).collect();
        if declarations.is_empty() {
            return None;
        }
        let is_optional = |symbol: &SymbolId| self.symbol_flags(*symbol) & ts_sf::Optional != 0;
        let optional = if is_union { found.iter().any(is_optional) } else { found.iter().all(is_optional) };
        let name = property.name.clone();
        let flags = ts_sf::Property | if optional { ts_sf::Optional } else { 0 };
        let ty = self.intern(self.property_type(property));
        Some(self.symbol_id(SymbolKey::Property(owner, name.clone()), || SymbolData::Transient {
            name,
            flags,
            ty,
            parent: None,
            declarations,
        }))
    }

    fn property_symbol_of_expression(&self, object: NodeId, name: &str) -> Option<SymbolId> {
        if let Some(symbol) = self.member_container_symbol(object)
            && self.symbol_flags(symbol) & (ts_sf::ValueModule | ts_sf::NamespaceModule | ts_sf::Alias | ts_sf::Enum | ts_sf::Class) != 0
            && self.reads_class_value(object)
        {
            if let Some(export) = self.export_of_symbol(symbol, name)
                && (self.symbol_flags(export) & ts_sf::Assignment == 0 || self.is_expando_owner(symbol))
            {
                return Some(export);
            }
            // A class's static member, its own or a base class's.
            let class = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
            if self.symbol_flags(class) & ts_sf::Class != 0
                && let Some(member) = self.static_member_in_hierarchy(class, name, &mut Vec::new())
            {
                return Some(member);
            }
        }
        // A function's expando member (`f.p = v`), which its type has too.
        if let Some(symbol) = self.symbol_at_location(object)
            && self.is_expando_owner(symbol)
            && let Some(member) = self
                .symbol_exports(symbol)
                .into_iter()
                .find(|member| self.symbol_flags(*member) & ts_sf::Assignment != 0 && self.symbol_name(*member) == name)
        {
            return Some(member);
        }
        let object_type = self.type_at_location(object);
        let object_type = match self.ty(object_type) {
            Type::ErrorType if self.program.node_kind(object) == SyntaxKind::ThisKeyword => {
                self.this_receiver_type(object).unwrap_or(object_type)
            }
            _ => object_type,
        };
        let value = self.ty(object_type);
        if let Some(property) = self.query(|query| query.property(&value, name)).flatten() {
            let found = self.property_symbol_written(object_type, property, Some(object));
            // A member surge reads off an index signature as a property of
            // its own is that signature's.
            if matches!(self.symbol_data(found), SymbolData::Transient { .. })
                && self.symbol_declarations(found).is_empty()
                && let Some(signature) = self.index_signature_member(object_type, object, name)
            {
                return Some(signature);
            }
            return Some(found);
        }
        // A name no member declares, read through an index signature, is
        // that signature's symbol (`__index`).
        if let Some(signature) = self.index_signature_member(object_type, object, name) {
            return Some(signature);
        }
        if let Some(interface) = self.global_function_interface(object_type)
            && let Some(member) = self.interface_member_in_hierarchy(interface, name)
        {
            return Some(member);
        }
        // An array member surge's array model lacks: the one the program's
        // own `Array` declares or inherits (a `noLib` lib's `ArrayLike.size`),
        // a tuple's too (`LuaTuple<[A, B]>`'s through its tuple operand).
        if is_array_like(&value)
            && let Some(member) = self.interface_member_in_hierarchy("Array", name)
        {
            return Some(member);
        }
        // `checkNonNullExpression`: a member is read off the object's
        // non-nullable type (an optional chain's `?.` strips it too).
        let nullish = |member: &Type| matches!(member, Type::Undefined | Type::Null | Type::Void);
        let Type::Union(union) = value.peeled() else { return None };
        if !union.types().iter().any(nullish) {
            return None;
        }
        let stripped = surge_ts_types::union_type(union.types().iter().filter(|member| !nullish(member)).cloned().collect());
        let property = self.query(|query| query.property(&stripped, name)).flatten()?;
        let ty = self.intern(self.property_type(&property));
        // The member is the non-nullable type's: its declaring interface's,
        // or the written union member it is.
        let found = self.property_symbol_written(self.intern_within(object_type, stripped), property, Some(object));
        if !matches!(self.symbol_data(found), SymbolData::Transient { .. }) {
            return Some(found);
        }
        // Keyed by the written object type, the member stays one symbol
        // across queries; it keeps what declares it.
        let declarations = self.symbol_declarations(found);
        let flags = self.symbol_flags(found) & !ts_sf::Transient;
        let parent = self.symbol_parent(found);
        let name: Arc<str> = Arc::from(name);
        Some(self.symbol_id(SymbolKey::Property(object_type, Arc::from(format!("\0nonnull\0{name}"))), || {
            SymbolData::Transient { name, flags, ty, parent, declarations }
        }))
    }

    /// The global interface `getPropertyOfType` reads a callable or
    /// constructable type's missing members off: `Function`, or under
    /// `strictBindCallApply` `CallableFunction` / `NewableFunction`
    /// (`getApparentType`'s `globalCallableFunctionType`).
    fn global_function_interface(&self, ty: TypeId) -> Option<&'static str> {
        let callable = !self.signatures_of_type(ty, SignatureKind::Call).is_empty();
        if !callable && self.signatures_of_type(ty, SignatureKind::Construct).is_empty() {
            return None;
        }
        let options = &self.program.inner().options;
        let strict = |name: &str| options.get(name).and_then(serde_json::Value::as_bool);
        if !strict("strictBindCallApply").or_else(|| strict("strict")).unwrap_or(false) {
            return Some("Function");
        }
        Some(if callable { "CallableFunction" } else { "NewableFunction" })
    }

    /// A JSX element's tag that names an intrinsic element
    /// (`isIntrinsicJsxName`: lowercase first, or a dash).
    fn is_intrinsic_tag_name(&self, node: NodeId) -> bool {
        let program = &self.program;
        program.node_parent(node).is_some_and(|parent| {
            matches!(
                program.node_kind(parent),
                SyntaxKind::JsxOpeningElement | SyntaxKind::JsxSelfClosingElement | SyntaxKind::JsxClosingElement
            ) && self.node_child(parent, "tagName") == Some(node)
        }) && program
            .node_value_text(node)
            .is_some_and(|name| name.starts_with(|first: char| first.is_ascii_lowercase()) || name.contains('-'))
    }

    /// The last `@name value` pragma the file's leading comments write
    /// (`processCommentPragmas`), its name matched case-insensitively.
    fn file_pragma(&self, file: super::handles::SourceFileId, name: &str) -> Option<String> {
        let text = self.program.text(file).clone();
        let mut rest = text.trim_start_matches('\u{feff}');
        let mut found = None;
        loop {
            rest = rest.trim_start();
            let comment = if let Some(line) = rest.strip_prefix("//") {
                let end = line.find('\n').unwrap_or(line.len());
                let (comment, after) = line.split_at(end);
                rest = after;
                comment
            } else if let Some(block) = rest.strip_prefix("/*") {
                let Some(end) = block.find("*/") else { break };
                let (comment, after) = block.split_at(end);
                rest = &after[2..];
                comment
            } else {
                break;
            };
            let mut search = comment;
            while let Some(at) = search.find('@') {
                let after = &search[at + 1..];
                let tag_end = after.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(after.len());
                let (tag, value) = after.split_at(tag_end);
                if tag.eq_ignore_ascii_case(name) && value.starts_with(|c: char| c.is_whitespace()) {
                    let value = value.trim_start();
                    let value_end = value.find(|c: char| c.is_whitespace() || c == '*').unwrap_or(value.len());
                    if value_end > 0 {
                        found = Some(value[..value_end].to_string());
                    }
                }
                search = after;
            }
        }
        found
    }

    /// The `JSX` namespace a JSX element at `location` reads its types from
    /// (`getJsxNamespaceAt`): the implicit runtime import's, the factory
    /// namespace's, else the global one.
    fn jsx_namespace_at(&self, location: NodeId) -> Option<SymbolId> {
        let file = location.source_file();
        let options = &self.program.inner().options;
        let option_text = |name: &str| options.get(name).and_then(serde_json::Value::as_str).map(str::to_string);
        let jsx = options.get("jsx").and_then(serde_json::Value::as_i64).unwrap_or(0);
        let runtime = self.file_pragma(file, "jsxRuntime");
        let import_source = self.file_pragma(file, "jsxImportSource");
        // `getJSXImplicitImportBase`.
        let implicit_base = (runtime.as_deref() != Some("classic")
            && (jsx == 4 || jsx == 5 || option_text("jsxImportSource").is_some() || import_source.is_some() || runtime.as_deref() == Some("automatic")))
        .then(|| import_source.clone().or_else(|| option_text("jsxImportSource")).unwrap_or_else(|| "react".to_string()));
        let jsx_export = |container: SymbolId| {
            let container = self.aliased_symbol(container).unwrap_or(container);
            self.export_of_symbol(container, "JSX").map(|jsx| self.aliased_symbol(jsx).unwrap_or(jsx))
        };
        if let Some(base) = implicit_base {
            let runtime_module = format!("{base}/{}", if jsx == 5 { "jsx-dev-runtime" } else { "jsx-runtime" });
            if let Some(target) = self.resolved_module_file(file, &runtime_module)
                && let Some(module) = self.module_symbol(target)
                && let Some(namespace) = jsx_export(module)
            {
                return Some(namespace);
            }
        }
        // `getJsxNamespace`: the `@jsx` pragma's or `jsxFactory`'s first
        // name, else `reactNamespace`, else `React`.
        let factory = self
            .file_pragma(file, "jsx")
            .or_else(|| option_text("jsxFactory"))
            .map(|factory| factory.split('.').next().unwrap_or_default().to_string())
            .or_else(|| option_text("reactNamespace"))
            .unwrap_or_else(|| "React".to_string());
        if let Some(container) = self.resolve_name(location, &factory, Meaning::Namespace)
            && let Some(namespace) = jsx_export(container)
        {
            return Some(namespace);
        }
        self.global_symbol("JSX", Meaning::Namespace).map(|jsx| self.aliased_symbol(jsx).unwrap_or(jsx))
    }

    /// `getIntrinsicTagSymbol`: the `JSX.IntrinsicElements` member the tag
    /// names, else the interface's own index signature, else the interface
    /// when an inherited one answers.
    fn intrinsic_tag_symbol(&self, tag: NodeId, name: &str) -> Option<SymbolId> {
        let namespace = self.jsx_namespace_at(tag)?;
        let elements = self.export_of_symbol(namespace, "IntrinsicElements")?;
        let elements = self.aliased_symbol(elements).unwrap_or(elements);
        if let Some(member) = self.member_in_hierarchy(elements, name, &mut Vec::new()) {
            return Some(member);
        }
        if let Some(index) = self.symbol_members(elements).into_iter().find(|member| self.symbol_name(*member) == "__index") {
            return Some(index);
        }
        let declared = self.declared_type_of_symbol(elements);
        let value = self.ty(declared);
        self.query(|query| query.index_type(&value, false).is_some()).unwrap_or(false).then_some(elements)
    }

    /// The index signature a read of `name` off `owner` goes through: the
    /// declaring type's `__index` member, or the one `object`'s declaration
    /// writes.
    fn index_signature_member(&self, owner: TypeId, object: NodeId, name: &str) -> Option<SymbolId> {
        const INDEX: &str = "__index";
        let program = &self.program;
        // `getApplicableIndexSymbol`: the `__index` of the type's own
        // members (a class's static side has its statics), never one it
        // inherits.
        let own = |symbol: SymbolId, statics: bool| {
            let table = if statics { self.symbol_exports(symbol) } else { self.symbol_members(symbol) };
            table.into_iter().find(|member| self.symbol_name(*member) == INDEX)
        };
        let object_symbol = self.symbol_at_location(object).map(|symbol| self.aliased_symbol(symbol).unwrap_or(symbol));
        let table = match (object_symbol, self.declaring_symbol(owner)) {
            (Some(class), _) if self.symbol_flags(class) & ts_sf::Class != 0 => own(class, true),
            // A class instance's members are its declared properties, which
            // hold no `__index`.
            (_, Some(symbol)) if self.symbol_flags(symbol) & ts_sf::Class != 0 => None,
            (_, Some(symbol)) => own(symbol, false),
            (_, None) => self.written_member_of_expression(object, INDEX, 0),
        }?;
        // A type literal written in a merged interface or class is read
        // through an instantiation (its `this` type has more than one
        // declaration, so `isTypeParameterPossiblyReferenced` holds), whose
        // members keep the literal's properties and drop its `__index`.
        let declared_in_merged = self.symbol_declarations(table).first().is_some_and(|declaration| {
            let mut current = program.node_parent(*declaration);
            while let Some(node) = current {
                if matches!(
                    program.node_kind(node),
                    SyntaxKind::InterfaceDeclaration | SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression
                ) {
                    return self.declared_symbol(node).is_some_and(|owner| self.symbol_declarations(owner).len() > 1);
                }
                current = program.node_parent(node);
            }
            false
        });
        let in_type_literal = self
            .symbol_declarations(table)
            .first()
            .and_then(|declaration| program.node_parent(*declaration))
            .is_some_and(|parent| program.node_kind(parent) == SyntaxKind::TypeLiteral);
        if in_type_literal && declared_in_merged {
            return None;
        }
        // The signatures a string name reads through: a string key's, and a
        // number key's too for a numeric name.
        let numeric = name.parse::<f64>().is_ok();
        let declarations = self.symbol_declarations(table);
        let applicable: Vec<NodeId> = declarations
            .iter()
            .copied()
            .filter(|declaration| {
                let Some(key) = self
                    .list_property(*declaration, "parameters")
                    .first()
                    .and_then(|parameter| self.node_child(*parameter, "type"))
                else {
                    return false;
                };
                let keys = match program.node_kind(key) {
                    SyntaxKind::UnionType => self.list_property(key, "types"),
                    _ => vec![key],
                };
                keys.iter().any(|key| match program.node_kind(*key) {
                    SyntaxKind::StringKeyword => true,
                    SyntaxKind::NumberKeyword => numeric,
                    _ => false,
                })
            })
            .collect();
        if applicable.is_empty() {
            return None;
        }
        if applicable.len() == declarations.len() {
            return Some(table);
        }
        let parent = self.symbol_parent(table);
        let ty = self.type_of_symbol(table);
        Some(self.symbol_id(SymbolKey::Property(owner, Arc::from(format!("\0index\0{numeric}"))), || SymbolData::Transient {
            name: Arc::from(INDEX),
            flags: ts_sf::Signature,
            ty,
            parent,
            declarations: applicable,
        }))
    }

    /// The member `name` of the global interface `interface`, or of the
    /// interfaces it extends, nearest first: the symbol its declaration has.
    fn interface_member_in_hierarchy(&self, interface: &str, name: &str) -> Option<SymbolId> {
        let mut queue = vec![interface.to_string()];
        let mut seen = Vec::new();
        while let Some(current) = queue.first().cloned() {
            queue.remove(0);
            if seen.contains(&current) || seen.len() > 16 {
                continue;
            }
            seen.push(current.clone());
            let Some(symbol) = self.global_symbol(&current, Meaning::Type) else { continue };
            for declaration in self.symbol_declarations(symbol) {
                if self.program.node_kind(declaration) != SyntaxKind::InterfaceDeclaration {
                    continue;
                }
                if let Some(NodePropertyValue::List { nodes, .. }) = self.program.node_property(declaration, "members") {
                    for member in nodes {
                        if self.node_child(member, "name").is_some_and(|member_name| self.program.node_text(member_name) == name) {
                            return self.declared_symbol(member);
                        }
                    }
                }
                if let Some(NodePropertyValue::List { nodes: clauses, .. }) = self.program.node_property(declaration, "heritageClauses") {
                    for clause in clauses {
                        if let Some(NodePropertyValue::List { nodes: bases, .. }) = self.program.node_property(clause, "types") {
                            for base in bases {
                                if let Some(expression) = self.node_child(base, "expression") {
                                    queue.push(self.program.node_text(expression).to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn has_readonly_modifier(&self, declaration: NodeId) -> bool {
        matches!(self.program.node_property(declaration, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
            if nodes.iter().any(|modifier| self.program.node_kind(*modifier) == SyntaxKind::ReadonlyKeyword))
    }

    fn has_async_modifier(&self, declaration: NodeId) -> bool {
        matches!(self.program.node_property(declaration, "modifiers"), Some(NodePropertyValue::List { nodes, .. })
            if nodes.iter().any(|modifier| self.program.node_kind(*modifier) == SyntaxKind::AsyncKeyword))
    }

    /// tsc's `checkPrivateIdentifierPropertyAccess`: the member the innermost
    /// class declaring the name has, read off an object that is not `any`.
    /// Outside every class that declares it a private name resolves to
    /// nothing.
    fn enclosing_private_member(&self, node: NodeId, name: &str) -> Option<SymbolId> {
        let program = &self.program;
        let mut current = program.node_parent(node);
        let mut member = None;
        while let Some(owner) = current {
            if matches!(program.node_kind(owner), SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression)
                && let Some(class) = self.declared_symbol(owner)
                && let Some(found) = self.symbol_members(class).into_iter().chain(self.symbol_exports(class)).find(|member| {
                    // The binder keys a private name by its class (`__#1@#x`).
                    self.symbol_data_name(*member).as_deref().and_then(private_member_name) == Some(name)
                })
            {
                member = Some(found);
                break;
            }
            current = program.node_parent(owner);
        }
        let member = member?;
        let access = program.node_parent(node)?;
        let object = self.node_child(access, "expression")?;
        let object_type = self.located_type(self.type_of_expression(object, program.node_kind(object)));
        (!matches!(object_type, Type::Any)).then_some(member)
    }

    fn symbol_data_name(&self, symbol: SymbolId) -> Option<String> {
        match self.symbol_data(symbol) {
            SymbolData::Bound { file, symbol } => {
                Some(self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].name.clone())
            }
            _ => None,
        }
    }

    /// A function-like's written `this` parameter.
    fn own_this_parameter(&self, function: NodeId) -> Option<NodeId> {
        let program = &self.program;
        if !matches!(
            program.node_kind(function),
            SyntaxKind::FunctionDeclaration | SyntaxKind::FunctionExpression | SyntaxKind::MethodDeclaration
        ) {
            return None;
        }
        let Some(NodePropertyValue::List { nodes, .. }) = program.node_property(function, "parameters") else { return None };
        let first = *nodes.first()?;
        self.node_child(first, "name").is_some_and(|name| program.node_text(name) == "this").then_some(first)
    }

    /// The `this` parameter the function binding `this` at `this_node`
    /// declares; arrow functions keep their container's.
    fn explicit_this_parameter(&self, this_node: NodeId) -> Option<NodeId> {
        let program = &self.program;
        let mut current = program.node_parent(this_node)?;
        loop {
            match program.node_kind(current) {
                SyntaxKind::FunctionDeclaration | SyntaxKind::FunctionExpression | SyntaxKind::MethodDeclaration => {
                    return self.own_this_parameter(current);
                }
                SyntaxKind::Constructor
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor
                | SyntaxKind::ClassDeclaration
                | SyntaxKind::ClassExpression
                | SyntaxKind::SourceFile
                | SyntaxKind::ModuleBlock => return None,
                _ => current = program.node_parent(current)?,
            }
        }
    }

    /// The base class `super` names inside a class that extends one.
    fn super_class_symbol(&self, node: NodeId) -> Option<SymbolId> {
        let program = &self.program;
        let mut current = program.node_parent(node)?;
        while !matches!(program.node_kind(current), SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression) {
            current = program.node_parent(current)?;
        }
        let Some(NodePropertyValue::List { nodes: clauses, .. }) = program.node_property(current, "heritageClauses") else { return None };
        let clause = clauses.into_iter().find(|clause| program.node_operator(*clause) == Some(SyntaxKind::ExtendsKeyword))?;
        let Some(NodePropertyValue::List { nodes: bases, .. }) = program.node_property(clause, "types") else { return None };
        let base = self.symbol_at_location(self.node_child(*bases.first()?, "expression")?)?;
        Some(if self.is_alias(base) { self.aliased_symbol(base).unwrap_or(base) } else { base })
    }

    /// The type `this` reads members from in a class member the checker left
    /// unrecorded: the class's instance type, or its constructor in a static
    /// member. An arrow function keeps its container's `this`; any other
    /// function binds its own.
    fn this_receiver_type(&self, this: NodeId) -> Option<TypeId> {
        let program = &self.program;
        let mut current = program.node_parent(this)?;
        loop {
            if let Some(first) = self.own_this_parameter(current) {
                // An explicit `this` parameter declares the receiver.
                let annotation = self.node_child(first, "type")?;
                return Some(self.intern(self.type_of_type_node(annotation)?));
            }
            match program.node_kind(current) {
                SyntaxKind::ArrowFunction => {}
                SyntaxKind::FunctionDeclaration | SyntaxKind::FunctionExpression | SyntaxKind::SourceFile | SyntaxKind::ModuleBlock => {
                    return None;
                }
                SyntaxKind::MethodDeclaration
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor
                | SyntaxKind::PropertyDeclaration
                | SyntaxKind::Constructor
                | SyntaxKind::ClassStaticBlockDeclaration => {
                    let class = program.node_parent(current)?;
                    if !matches!(program.node_kind(class), SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression) {
                        return None;
                    }
                    let symbol = self.declared_symbol(class)?;
                    let is_static = program.node_kind(current) == SyntaxKind::ClassStaticBlockDeclaration
                        || is_static_member(&program.syntax(current.source_file()).tree, current.node);
                    return Some(if is_static { self.type_of_symbol(symbol) } else { self.declared_type_of_symbol(symbol) });
                }
                _ => {}
            }
            current = program.node_parent(current)?;
        }
    }

    /// `getApparentType`.
    pub fn apparent_type(&self, ty: TypeId) -> TypeId {
        let value = self.ty(ty);
        let key = Some(TypeKey::Apparent(ty));
        self.type_id(key, None, || self.query(|query| query.apparent_type(&value)).unwrap_or(value))
    }

    /// `getBaseConstraintOfType`: a type parameter's constraint, a union's
    /// or template literal's own type, and no constraint for any other type.
    /// surge's type for a type parameter keeps no constraint once the body
    /// declaring it is checked, so the constraint is resolved from the
    /// declaration as written (tsc resolves it on through other type
    /// parameters' constraints).
    pub fn base_constraint_of_type(&self, ty: TypeId) -> Result<Option<TypeId>, Unsupported> {
        let flags = self.type_flags(ty);
        if flags & tf::TypeParameter != 0 {
            return self.type_parameter_constraint(ty);
        }
        if flags & (tf::Union | tf::Intersection) != 0 {
            let value = self.ty(ty);
            if !has_type_parameter_member(&value) {
                return Ok(Some(ty));
            }
            // tsc's `computeBaseConstraint` of a union: each member's base
            // constraint, a member without one left out.
            let Type::Union(union) = &value else {
                return Err(Unsupported("getBaseConstraintOfType for an intersection with a type parameter"));
            };
            let mut members = Vec::new();
            for member in union.types() {
                match member {
                    Type::TypeParameter(parameter) => {
                        let declaration = self
                            .type_parameter_named(&parameter.name, self.reached_at(ty))
                            .ok_or(Unsupported("getBaseConstraintOfType for a union with a type parameter out of scope where it was reached"))?;
                        if let Some(constraint) = self.declared_constraint(declaration)? {
                            members.push(constraint);
                        }
                    }
                    other => members.push(other.clone()),
                }
            }
            return Ok(Some(self.intern(surge_ts_types::union_type(members))));
        }
        Ok((flags & (tf::TemplateLiteral | tf::StringMapping) != 0).then_some(ty))
    }

    fn type_parameter_constraint(&self, ty: TypeId) -> Result<Option<TypeId>, Unsupported> {
        let unsupported = Unsupported("getBaseConstraintOfType for a type parameter not reached from its declaration");
        // The declaration the handle came from, or the type parameter its
        // name resolves to where the type was reached.
        let declaration = self
            .origin(ty)
            .and_then(|symbol| self.type_parameter_declaration(symbol))
            .or_else(|| match self.ty(ty) {
                Type::TypeParameter(parameter) => self.type_parameter_named(&parameter.name, self.reached_at(ty)),
                _ => None,
            })
            .ok_or(unsupported)?;
        Ok(self.declared_constraint(declaration)?.map(|constraint| self.intern(constraint)))
    }

    /// Where a type was reached: the node it is the type of, or the first
    /// declaration of the symbol it is the type of.
    fn reached_at(&self, ty: TypeId) -> Option<NodeId> {
        self.location(ty)
            .or_else(|| self.origin(ty).and_then(|symbol| self.symbol_declarations(symbol).first().copied()))
            .or_else(|| self.with_tables(|tables| tables.types[ty.index as usize].scope))
            // A synthesized symbol's type was reached where the type it was
            // made with was (a contextual `this`).
            .or_else(|| match self.origin(ty).map(|symbol| self.symbol_data(symbol)) {
                Some(SymbolData::Transient { ty: made, .. }) if made != ty => self.reached_at(made),
                _ => None,
            })
    }

    fn type_parameter_declaration(&self, symbol: SymbolId) -> Option<NodeId> {
        self.symbol_declarations(symbol)
            .into_iter()
            .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeParameter)
    }

    /// The type parameter `name` declares in scope at `location`.
    fn type_parameter_named(&self, name: &str, location: Option<NodeId>) -> Option<NodeId> {
        let symbol = self.resolve_name(location?, name, Meaning::Type)?;
        if self.symbol_flags(symbol) & ts_sf::TypeParameter == 0 {
            return None;
        }
        self.type_parameter_declaration(symbol)
    }

    /// A type parameter declaration's constraint as written, resolved where
    /// it is written; `None` for an unconstrained one.
    fn declared_constraint(&self, declaration: NodeId) -> Result<Option<Type>, Unsupported> {
        let unsupported = Unsupported("getBaseConstraintOfType for a constraint surge could not resolve");
        let Some(constraint) = self.node_child(declaration, "constraint") else { return Ok(None) };
        let text = self.program.node_text(constraint).to_string();
        let checker_index = self.program.file(declaration.source_file()).checker_index.ok_or(unsupported)?;
        let type_parameters = self.type_parameters_in_scope(declaration);
        let namespaces = self.namespace_scope(declaration);
        self.query(|query| query.resolve_type_text(checker_index, &text, &type_parameters, &namespaces))
            .flatten()
            .map(Some)
            .ok_or(unsupported)
    }

    /// `getNonNullableType`.
    pub fn non_nullable_type(&self, ty: TypeId) -> TypeId {
        let value = self.ty(ty);
        let non_nullable = surge_ts_types::remove_nullish(&value);
        // A type with nothing to remove is itself, the handle keeping where
        // it was reached.
        if non_nullable == value {
            return ty;
        }
        self.intern_within(ty, non_nullable)
    }

    /// `getBaseTypeOfLiteralType`.
    pub fn base_type_of_literal_type(&self, ty: TypeId) -> TypeId {
        match base_of_literal(&self.ty(ty)) {
            Some(base) => self.intern(base),
            None => ty,
        }
    }

    /// `getWidenedType`. tsc widens only `null` and `undefined` outside
    /// `strictNullChecks` and an object literal's freshness, which surge's
    /// types do not carry: with `strictNullChecks` every type is its own
    /// widened type. Without it surge cannot tell a widening `null` from a
    /// declared one.
    pub fn widened_type(&self, ty: TypeId) -> Result<TypeId, Unsupported> {
        if self.program.inner().strict_null_checks {
            Ok(ty)
        } else {
            Err(Unsupported("getWidenedType without strictNullChecks"))
        }
    }

    /// `isArrayType`.
    pub fn is_array_type(&self, ty: TypeId) -> bool {
        matches!(self.ty(ty).peeled(), Type::Array(_))
    }

    /// `isTupleType`.
    pub fn is_tuple_type(&self, ty: TypeId) -> bool {
        matches!(self.ty(ty).peeled(), Type::Tuple(_) | Type::OpenTuple(_))
    }

    /// `getTypeArguments`: an array's element, a tuple's elements, a generic
    /// instantiation's arguments.
    pub fn type_arguments(&self, ty: TypeId) -> Vec<TypeId> {
        let value = self.ty(ty);
        let arguments = match &value {
            Type::Reference(reference) if !reference.arguments.is_empty() && !reference.is_readonly_array() => {
                reference.arguments.to_vec()
            }
            _ => match value.peeled() {
                Type::Array(element) => vec![*element],
                Type::OpenTuple(open) => open.leading.into_iter().chain([*open.rest]).chain(open.trailing).collect(),
                Type::Tuple(elements) => elements.into_elements(),
                _ => Vec::new(),
            },
        };
        arguments.into_iter().map(|argument| self.intern_within(ty, argument)).collect()
    }

    /// `getStringIndexType` / `getNumberIndexType`.
    pub fn index_type_of_type(&self, ty: TypeId, number: bool) -> Option<TypeId> {
        let value = self.ty(ty);
        let index = self.query(|query| query.index_type(&value, number)).flatten()?;
        Some(self.intern_within(ty, index))
    }

    /// `isTypeAssignableTo`.
    pub fn is_type_assignable_to(&self, source: TypeId, target: TypeId) -> bool {
        let (source, target) = (self.ty(source), self.ty(target));
        self.query(|query| query.is_assignable(&source, &target)).unwrap_or(false)
    }

    // --- signatures ----------------------------------------------------------

    /// `getSignaturesOfType`.
    pub fn signatures_of_type(&self, ty: TypeId, kind: SignatureKind) -> Vec<SignatureId> {
        let value = self.ty(ty);
        let construct = kind == SignatureKind::Construct;
        let functions = self.query(|query| query.signatures(&value, construct)).unwrap_or_default();
        let declarations = self.signature_declarations(ty, kind);
        let program = self.program.inner().identity;
        let scope = self.reached_at(ty);
        functions
            .into_iter()
            .enumerate()
            .map(|(position, function)| {
                let key = (ty, kind, position as u32);
                let declaration = if declarations.len() == 1 { declarations.first().copied() } else { declarations.get(position).copied() };
                self.with_tables(|tables| {
                    if let Some(&index) = tables.signature_by_key.get(&key) {
                        return SignatureId { program, index };
                    }
                    let index = tables.signatures.len() as u32;
                    tables.signatures.push(SignatureEntry { function, declaration, scope });
                    tables.signature_by_key.insert(key, index);
                    SignatureId { program, index }
                })
            })
            .collect()
    }

    /// The signature-bearing declarations of the symbol a type came from, in
    /// declaration order, an implementation that follows overloads dropped.
    fn signature_declarations(&self, ty: TypeId, kind: SignatureKind) -> Vec<NodeId> {
        let Some(origin) = self.origin(ty) else {
            // A function expression's own type: the expression declares it.
            return self
                .location(ty)
                .filter(|location| {
                    kind == SignatureKind::Call
                        && matches!(self.program.node_kind(*location), SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression)
                })
                .into_iter()
                .collect();
        };
        // A value annotated with a function or constructor type: the type
        // node declares its signature, whatever initializes the value.
        if let Some(annotation) = self.function_type_annotation(origin) {
            let written = match kind {
                SignatureKind::Call => SyntaxKind::FunctionType,
                SignatureKind::Construct => SyntaxKind::ConstructorType,
            };
            if self.program.node_kind(annotation) == written {
                return vec![annotation];
            }
        }
        if kind == SignatureKind::Call
            && let Some(function) = self.function_initializer(origin)
        {
            return vec![function];
        }
        let declarations: Vec<NodeId> = self
            .symbol_declarations(origin)
            .into_iter()
            .filter(|declaration| {
                let declaration_kind = self.program.node_kind(*declaration);
                match kind {
                    SignatureKind::Call => matches!(
                        declaration_kind,
                        SyntaxKind::FunctionDeclaration | SyntaxKind::MethodDeclaration | SyntaxKind::MethodSignature
                            | SyntaxKind::FunctionExpression | SyntaxKind::ArrowFunction | SyntaxKind::CallSignature
                    ),
                    SignatureKind::Construct => matches!(declaration_kind, SyntaxKind::Constructor | SyntaxKind::ConstructSignature),
                }
            })
            .collect();
        if declarations.len() > 1
            && let Some(last) = declarations.last()
            && self.node_child(*last, "body").is_some()
        {
            return declarations[..declarations.len() - 1].to_vec();
        }
        declarations
    }

    fn signature(&self, signature: SignatureId) -> SignatureEntry {
        assert_eq!(signature.program, self.program.inner().identity, "signature handle belongs to another program");
        self.with_tables(|tables| tables.signatures[signature.index as usize].clone())
    }

    /// `getReturnTypeOfSignature`.
    pub fn return_type_of_signature(&self, signature: SignatureId) -> TypeId {
        let entry = self.signature(signature);
        self.intern_in_scope(entry.declaration, entry.function.return_type().clone())
    }

    /// `Signature.getDeclaration()`.
    pub fn signature_declaration(&self, signature: SignatureId) -> Option<NodeId> {
        self.signature(signature).declaration
    }

    /// `Signature.getParameters()`: the declared parameters' symbols when the
    /// declaration is known, else symbols synthesized from the signature.
    pub fn signature_parameters(&self, signature: SignatureId) -> Vec<SymbolId> {
        let entry = self.signature(signature);
        if let Some(declaration) = entry.declaration
            && let Some(super::node::NodePropertyValue::List { nodes, .. }) = self.program.node_property(declaration, "parameters")
        {
            // A `this` parameter is the signature's `thisParameter`, not one of its parameters.
            let nodes = match nodes.first() {
                Some(first) if self.is_this_parameter(*first) => &nodes[1..],
                _ => &nodes[..],
            };
            let symbols: Vec<SymbolId> = nodes.iter().filter_map(|parameter| self.declared_symbol(*parameter)).collect();
            if symbols.len() == nodes.len() {
                return symbols;
            }
        }
        let names = entry.function.parameter_names().map(<[_]>::to_vec).unwrap_or_default();
        entry
            .function
            .parameters()
            .iter()
            .enumerate()
            .map(|(position, parameter_type)| {
                let name: Arc<str> = names
                    .get(position)
                    .cloned()
                    .flatten()
                    .unwrap_or_else(|| Arc::from(format!("arg{position}")));
                let ty = self.intern_in_scope(entry.declaration, parameter_type.clone());
                self.symbol_id(SymbolKey::Parameter(signature.index, position as u32), || SymbolData::Transient {
                    name,
                    flags: ts_sf::FunctionScopedVariable,
                    ty,
                    parent: None,
                    declarations: Vec::new(),
                })
            })
            .collect()
    }

    /// `Signature.thisParameter`: the declared `this` parameter's symbol, or
    /// one standing for the signature's `this` type.
    pub fn signature_this_parameter(&self, signature: SignatureId) -> Option<SymbolId> {
        let entry = self.signature(signature);
        let declared = entry.declaration.and_then(|declaration| match self.program.node_property(declaration, "parameters") {
            Some(super::node::NodePropertyValue::List { nodes, .. }) => {
                nodes.first().copied().filter(|first| self.is_this_parameter(*first))
            }
            _ => None,
        });
        // An instantiation's `this` is its own (`Callable<void>`'s `void` for a
        // written `this: T`), carried by a symbol of its own.
        let instantiated = entry.function.this_type().filter(|this_type| {
            declared
                .and_then(|parameter| self.declared_symbol(parameter))
                .is_some_and(|symbol| self.ty(self.type_of_symbol(symbol)) != **this_type)
        });
        if instantiated.is_none()
            && let Some(parameter) = declared
            && let Some(symbol) = self.declared_symbol(parameter)
        {
            return Some(symbol);
        }
        let this_type = match entry.function.this_type() {
            Some(this_type) => this_type.clone(),
            // A context-sensitive arrow or function expression takes its
            // contextual signature's `this` (`assignContextualParameterTypes`).
            None => {
                let declaration = entry
                    .declaration
                    .filter(|declaration| declared.is_none() && self.has_context_sensitive_parameters(*declaration))?;
                let contextual = match self.ty(self.contextual_type(declaration)?).peeled() {
                    Type::Function(function) => function,
                    Type::Object(object) => object.call_signature()?.clone(),
                    _ => return None,
                };
                contextual.this_type()?.clone()
            }
        };
        let ty = self.intern_in_scope(entry.declaration.or(entry.scope), this_type);
        Some(self.symbol_id(SymbolKey::Parameter(signature.index, u32::MAX), || SymbolData::Transient {
            name: Arc::from("this"),
            flags: ts_sf::FunctionScopedVariable,
            ty,
            parent: None,
            declarations: declared.into_iter().collect(),
        }))
    }

    /// tsc's `hasContextSensitiveParameters` for an arrow or function
    /// expression: not generic, and a parameter left unannotated (or, for a
    /// function expression, no `this` parameter of its own).
    fn has_context_sensitive_parameters(&self, function: NodeId) -> bool {
        let kind = self.program.node_kind(function);
        if !matches!(kind, SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression)
            || !self.list_property(function, "typeParameters").is_empty()
        {
            return false;
        }
        let parameters = self.list_property(function, "parameters");
        parameters.iter().any(|parameter| self.node_child(*parameter, "type").is_none())
            || kind == SyntaxKind::FunctionExpression && !parameters.first().is_some_and(|first| self.is_this_parameter(*first))
    }

    /// A parameter declaration named `this`.
    fn is_this_parameter(&self, parameter: NodeId) -> bool {
        self.program.node_kind(parameter) == SyntaxKind::Parameter
            && self.node_child(parameter, "name").is_some_and(|name| self.program.node_text(name) == "this")
    }

    /// `Signature.getTypeParameters()`.
    pub fn signature_type_parameters(&self, signature: SignatureId) -> Vec<TypeId> {
        let entry = self.signature(signature);
        entry.function.type_parameter_names().iter().map(|name| self.intern(Type::type_parameter(name))).collect()
    }

    /// `signatureToString`: `(a: T, b?: U): R`, as tsc writes a signature.
    pub fn signature_to_string(&self, signature: SignatureId) -> String {
        self.tsc_display(|| self.render_signature(signature))
    }

    fn render_signature(&self, signature: SignatureId) -> String {
        let entry = self.signature(signature);
        let function = &entry.function;
        let names = function.parameter_names().map(<[_]>::to_vec).unwrap_or_default();
        let count = function.parameters().len();
        let parameters: Vec<String> = function
            .parameters()
            .iter()
            .enumerate()
            .map(|(position, ty)| {
                let name = names.get(position).cloned().flatten().map(|name| name.to_string()).unwrap_or_else(|| format!("arg{position}"));
                let rest = function.is_variadic() && position + 1 == count;
                let optional = !rest && position >= function.required_parameter_count();
                format!("{}{name}{}: {}", if rest { "..." } else { "" }, if optional { "?" } else { "" }, ty.name())
            })
            .collect();
        let head = match function.type_parameter_head() {
            Some(head) if !head.is_empty() && !head.starts_with('<') => format!("<{head}>"),
            head => head.unwrap_or_default().to_string(),
        };
        format!("{head}({}): {}", parameters.join(", "), function.return_type().name())
    }
}

/// tsc's `typeToString` cuts a printed type at 320 UTF-16 units
/// (`defaultMaximumTruncationLength * 2`), ending it with `...`.
fn truncated(text: String) -> String {
    const LIMIT: usize = 320;
    if text.encode_utf16().count() < LIMIT {
        return text;
    }
    let mut units = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        if units + character.len_utf16() > LIMIT - 3 {
            break;
        }
        units += character.len_utf16();
        end = index + character.len_utf8();
    }
    format!("{}...", &text[..end])
}

/// A literal type's value.
#[derive(Clone, Debug, PartialEq)]
pub enum LiteralValue {
    String(String),
    Number(f64),
}

/// The signature surge stands in for one it did not model (an ambient
/// namespace's function): `(any[], ...args: any[]) => any`.
fn is_stand_in_signature(ty: &Type) -> bool {
    match ty {
        Type::Function(function) => {
            function.is_variadic()
                && matches!(function.parameters(), [Type::Array(element)] if matches!(element.as_ref(), Type::Any))
                && matches!(function.return_type(), Type::Any)
        }
        _ => false,
    }
}

/// [`TypeChecker::literal_context`]: no contextual type, one the API cannot
/// tell, a written type, or a destructuring pattern.
enum LiteralContext {
    None,
    Unknown,
    Type(Type),
    Pattern(NodeId),
}

/// `isTupleLikeType` of a contextual type, or of one of its union members.
fn is_tuple_like(ty: &Type) -> bool {
    match ty.peeled() {
        Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Union(union) => union.types().iter().any(is_tuple_like),
        _ => false,
    }
}

/// `isLiteralOfContextualType`: the contextual type keeps a literal of the
/// literal's own kind (or is a type variable).
fn keeps_literal(contextual: &Type, literal: &Type) -> bool {
    let kind = |ty: &Type| match ty {
        Type::StringLiteral(_) => Some(0),
        Type::NumberLiteral(_) => Some(1),
        Type::BooleanLiteral(_) => Some(2),
        _ => None,
    };
    let Some(wanted) = kind(literal) else { return false };
    match contextual.peeled() {
        Type::TypeParameter(_) => true,
        Type::Union(union) => union.types().iter().any(|member| keeps_literal(member, literal)),
        other => kind(&other) == Some(wanted),
    }
}

/// The written `#x` of the binder's key for a private member, `__#1@#x`.
fn private_member_name(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("__#")?;
    let (_, written) = rest.split_once('@')?;
    written.starts_with('#').then_some(written)
}

fn strip_extension(file_name: &str) -> &str {
    for extension in [".d.ts", ".d.mts", ".d.cts", ".tsx", ".ts", ".mts", ".cts", ".jsx", ".js", ".mjs", ".cjs"] {
        if let Some(stripped) = file_name.strip_suffix(extension) {
            return stripped;
        }
    }
    file_name
}

/// An array or tuple, or an intersection with one: what reads its members
/// off the program's `Array`.
fn is_array_like(ty: &Type) -> bool {
    match ty.peeled() {
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Object(object) if object.is_intersection => {
            object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(is_array_like))
        }
        _ => false,
    }
}

fn is_static_member(tree: &surge_ts_tsc_syntax::SyntaxTree, declaration: u32) -> bool {
    tree.node(declaration).modifier_nodes().iter().any(|&modifier| tree.kind(modifier) == Kind::StaticKeyword)
}

fn declaration_kinds() -> &'static [SyntaxKind] {
    &[
        SyntaxKind::VariableDeclaration,
        SyntaxKind::Parameter,
        SyntaxKind::BindingElement,
        SyntaxKind::FunctionDeclaration,
        SyntaxKind::ClassDeclaration,
        SyntaxKind::ClassExpression,
        SyntaxKind::InterfaceDeclaration,
        SyntaxKind::TypeAliasDeclaration,
        SyntaxKind::EnumDeclaration,
        SyntaxKind::EnumMember,
        SyntaxKind::ModuleDeclaration,
        SyntaxKind::TypeParameter,
        SyntaxKind::PropertyDeclaration,
        SyntaxKind::PropertySignature,
        SyntaxKind::MethodDeclaration,
        SyntaxKind::MethodSignature,
        SyntaxKind::GetAccessor,
        SyntaxKind::SetAccessor,
        SyntaxKind::PropertyAssignment,
        SyntaxKind::ShorthandPropertyAssignment,
        SyntaxKind::ImportSpecifier,
        SyntaxKind::ExportSpecifier,
        SyntaxKind::ImportClause,
        SyntaxKind::NamespaceImport,
        SyntaxKind::NamespaceExport,
        SyntaxKind::ImportEqualsDeclaration,
        SyntaxKind::FunctionExpression,
        SyntaxKind::JsxAttribute,
    ]
}

fn meaning_of_location(program: &Program, node: NodeId) -> Meaning {
    let mut current = node;
    // Left of a heritage clause's dotted name (`Ambient` in `extends
    // Ambient.Config`) is a namespace.
    let mut qualifies = false;
    while let Some(parent) = program.node_parent(current) {
        match program.node_kind(parent) {
            SyntaxKind::TypeQuery => return Meaning::Value,
            SyntaxKind::QualifiedName => {
                if program.node_property(parent, "left") == Some(super::node::NodePropertyValue::Node(current)) {
                    return Meaning::Namespace;
                }
                current = parent;
            }
            SyntaxKind::TypeReference => return Meaning::Type,
            // `getSymbolOfPartOfRightHandSideOfImportEquals`: `import x = N`
            // names a namespace.
            SyntaxKind::ImportEqualsDeclaration if current == node => return Meaning::Namespace,
            SyntaxKind::ExpressionWithTypeArguments => {
                let clause = program.node_parent(parent);
                let in_interface = clause
                    .and_then(|clause| program.node_parent(clause))
                    .is_some_and(|owner| program.node_kind(owner) == SyntaxKind::InterfaceDeclaration);
                let implements = clause.is_some_and(|clause| program.node_operator(clause) == Some(SyntaxKind::ImplementsKeyword));
                return match (in_interface || implements, qualifies) {
                    (true, true) => Meaning::Namespace,
                    (true, false) => Meaning::Type,
                    (false, _) => Meaning::Value,
                };
            }
            SyntaxKind::PropertyAccessExpression
                if matches!(program.node_kind(current), SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression) =>
            {
                qualifies |= program.node_property(parent, "expression") == Some(super::node::NodePropertyValue::Node(current));
                current = parent;
            }
            _ => return Meaning::Value,
        }
    }
    Meaning::Value
}

fn literal_type(program: &Program, node: NodeId, kind: SyntaxKind) -> Option<Type> {
    Some(match kind {
        SyntaxKind::StringLiteral | SyntaxKind::NoSubstitutionTemplateLiteral => {
            Type::StringLiteral(program.node_value_text(node)?.to_string())
        }
        SyntaxKind::NumericLiteral => Type::NumberLiteral(surge_ts_types::NumberLiteralType {
            value: normalize_number(program.node_value_text(node)?),
        }),
        SyntaxKind::TrueKeyword => Type::BooleanLiteral(true),
        SyntaxKind::FalseKeyword => Type::BooleanLiteral(false),
        SyntaxKind::NullKeyword => Type::Null,
        SyntaxKind::BigIntLiteral => Type::BigInt,
        _ => return None,
    })
}

fn normalize_number(text: &str) -> String {
    let cleaned = text.replace('_', "");
    let value = if let Some(hex) = cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok().map(|value| value as f64)
    } else if let Some(binary) = cleaned.strip_prefix("0b").or_else(|| cleaned.strip_prefix("0B")) {
        i64::from_str_radix(binary, 2).ok().map(|value| value as f64)
    } else if let Some(octal) = cleaned.strip_prefix("0o").or_else(|| cleaned.strip_prefix("0O")) {
        i64::from_str_radix(octal, 8).ok().map(|value| value as f64)
    } else {
        cleaned.parse::<f64>().ok()
    };
    match value {
        Some(value) if value.fract() == 0.0 && value.abs() < 1e21 => format!("{}", value as i64),
        Some(value) => format!("{value}"),
        None => text.to_string(),
    }
}

fn keyword_type(kind: SyntaxKind) -> Option<Type> {
    Some(match kind {
        SyntaxKind::StringKeyword => Type::String,
        SyntaxKind::NumberKeyword => Type::Number,
        SyntaxKind::BooleanKeyword => Type::Boolean,
        SyntaxKind::BigIntKeyword => Type::BigInt,
        SyntaxKind::SymbolKeyword => Type::Symbol,
        SyntaxKind::AnyKeyword => Type::Any,
        SyntaxKind::UnknownKeyword => Type::GenuineUnknown,
        SyntaxKind::NeverKeyword => Type::Never,
        SyntaxKind::VoidKeyword => Type::Void,
        SyntaxKind::UndefinedKeyword => Type::Undefined,
        _ => return None,
    })
}

fn has_type_parameter_member(ty: &Type) -> bool {
    let is_parameter = |member: &Type| matches!(member, Type::TypeParameter(_));
    match ty {
        Type::Union(union) => union.types().iter().any(is_parameter),
        Type::Object(object) => object.intersection_operands.as_ref().is_some_and(|operands| operands.iter().any(is_parameter)),
        _ => false,
    }
}

/// tsc's `getBaseTypeOfLiteralType`: a literal's primitive, an enum member's
/// enum, a union's members' bases; `None` for a type it leaves as it is.
fn base_of_literal(ty: &Type) -> Option<Type> {
    match ty {
        Type::StringLiteral(_) => Some(Type::String),
        Type::NumberLiteral(_) => Some(Type::Number),
        Type::BooleanLiteral(_) => Some(Type::Boolean),
        Type::Reference(reference)
            if reference.id.as_ref() == surge_ts_types::TEMPLATE_LITERAL_REFERENCE_ID
                || reference.id.as_ref() == surge_ts_types::STRING_MAPPING_REFERENCE_ID =>
        {
            Some(Type::String)
        }
        Type::Reference(reference) => reference.enum_base.as_deref().cloned(),
        Type::Union(union) => Some(surge_ts_types::union_type(
            union.types().iter().map(|member| base_of_literal(member).unwrap_or_else(|| member.clone())).collect(),
        )),
        _ => None,
    }
}

fn shallow_flags(ty: &Type) -> i32 {
    match ty {
        Type::String => tf::String,
        Type::Number => tf::Number,
        Type::Boolean => tf::Boolean | tf::Union,
        Type::BigInt => tf::BigInt,
        Type::Symbol => tf::ESSymbol,
        Type::Undefined => tf::Undefined,
        Type::Null => tf::Null,
        Type::Void => tf::Void,
        Type::Any | Type::ErrorType | Type::Unknown => tf::Any,
        Type::GenuineUnknown => tf::Unknown,
        Type::TypeParameter(_) => tf::TypeParameter,
        Type::Never => tf::Never,
        Type::StringLiteral(_) => tf::StringLiteral,
        Type::NumberLiteral(_) => tf::NumberLiteral,
        Type::BooleanLiteral(_) => tf::BooleanLiteral,
        Type::Union(_) => tf::Union,
        Type::Object(object) if object.is_intersection && object.intersection_operands.is_some() => tf::Intersection,
        Type::Function(_) | Type::Object(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => tf::Object,
        Type::Reference(_) => 0,
    }
}

fn flags_of(query: &mut Query<'_>, ty: &Type) -> i32 {
    match ty {
        Type::Reference(reference) => {
            if reference.id.as_ref() == surge_ts_types::TEMPLATE_LITERAL_REFERENCE_ID {
                return tf::TemplateLiteral;
            }
            if reference.id.as_ref() == surge_ts_types::STRING_MAPPING_REFERENCE_ID {
                return tf::StringMapping;
            }
            let peeled = query.peel(ty);
            if matches!(peeled, Type::Reference(_)) {
                return tf::Object;
            }
            let flags = flags_of(query, &peeled);
            if reference.enum_owner.is_some() && flags & (tf::NumberLiteral | tf::StringLiteral) != 0 {
                flags | tf::EnumLiteral
            } else {
                flags
            }
        }
        Type::Union(union) => {
            let members = union.types();
            let is_boolean = members.len() == 2
                && members.contains(&Type::BooleanLiteral(true))
                && members.contains(&Type::BooleanLiteral(false));
            if is_boolean { tf::Boolean | tf::Union } else { tf::Union }
        }
        _ => shallow_flags(ty),
    }
}

/// `ty` named by the alias `name`, as tsc prints a type it attached the alias
/// to; an array or tuple, which carries no display of its own, behind a
/// reference with the nominal identity `id`.
fn with_alias_display(ty: Type, name: &str, id: String) -> Type {
    match ty {
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => {
            Type::Reference(surge_ts_types::TypeReference::new(id, name, Vec::new(), Arc::new(AliasBody(ty))))
        }
        Type::Union(union) => Type::Union(union.with_alias_name(name)),
        Type::Object(object) => Type::Object(object.with_alias_name(name)),
        Type::Function(function) => Type::Function(function.with_alias_name(name)),
        Type::Reference(mut reference) => {
            reference.alias_display = Some(Arc::from(name));
            Type::Reference(reference)
        }
        other => other,
    }
}

/// `ty` without the display the checker names it by when that is the alias
/// `name` (bare or qualified by its namespaces), which tsc did not attach.
fn without_alias_display(ty: Type, name: &str) -> Type {
    let names_alias = |display: &str| {
        display == name || display.strip_suffix(name).is_some_and(|qualifier| qualifier.ends_with('.'))
    };
    match ty {
        Type::Union(union) if union.alias_name().is_some_and(names_alias) => Type::Union(union.without_alias_name()),
        Type::Object(mut object) if object.alias_name.as_deref().is_some_and(names_alias) => {
            object.alias_name = None;
            Type::Object(object)
        }
        Type::Function(function) if function.alias_name().is_some_and(names_alias) => Type::Function(function.without_alias_name()),
        Type::Reference(mut reference) if reference.alias_display.as_deref().is_some_and(names_alias) => {
            reference.alias_display = None;
            Type::Reference(reference)
        }
        // A lazy reference to the alias itself stands for its body.
        Type::Reference(reference) if reference.arguments.is_empty() && names_alias(&reference.display) => {
            without_alias_display(reference.resolve(), name)
        }
        other => other,
    }
}
