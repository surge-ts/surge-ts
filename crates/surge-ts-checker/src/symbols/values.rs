use std::collections::{HashMap, HashSet};

use std::sync::Arc;

use surge_ts_syntax::ParsedExpression;
use surge_ts_types::fx::FxBuildHasher;

use surge_ts_syntax::{ParsedType, ParsedTypeParameter, TextSpan};
use surge_ts_types::{FunctionType, Type, TypeCopyReason, with_type_copy_reason};

use crate::program::{
    record_symbol_info_handle_copy_count, record_symbol_info_payload_deep_clone_count,
    record_symbol_table_clone_count, record_symbol_table_entry_handle_copy_count,
    record_type_name_lookup_string_count,
};

pub(crate) type SymbolInfoHandle = Arc<SymbolInfo>;

#[derive(Debug)]
pub(crate) struct SymbolInfo {
    pub(crate) ty: Type,
    pub(crate) kind: SymbolKind,
    pub(crate) function_signature: Option<Arc<FunctionSignatureInfo>>,
}

impl Clone for SymbolInfo {
    fn clone(&self) -> Self {
        record_symbol_info_payload_deep_clone_count();
        Self {
            ty: self.ty.clone(),
            kind: self.kind,
            function_signature: self.function_signature.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionSignatureInfo {
    /// Set when this signature is the instantiation template of an *overload
    /// group*: the group's value type is the permissive fold of every overload
    /// and only the first declaration's parsed signature is kept, so a call
    /// instantiated through it against a later overload's argument shape binds
    /// the wrong parameters (`vi.spyOn(obj, "method")` against the `"get"`
    /// accessor overload). Paths that instantiate on their own initiative
    /// consult this and leave the folded type as it is.
    pub(crate) overloaded: bool,
    pub(crate) type_parameters: Vec<ParsedTypeParameter>,
    pub(crate) parameter_types: Vec<Option<ParsedType>>,
    /// Declared parameter names (`None` for binding patterns), parallel to
    /// `parameter_types`. A `x is T` return predicate names its tested
    /// parameter, so guard narrowing maps that name back to a call-argument
    /// position through this list.
    pub(crate) parameter_names: Vec<Option<String>>,
    /// The last parameter is a `...rest`. Its written type is what the rest
    /// arguments are inferred against as a whole (a tuple for a bare `E`, the
    /// element for `T[]`), not what each argument is matched to by position.
    pub(crate) rest: bool,
    pub(crate) return_type: Option<ParsedType>,
    /// File the signature was declared in. Instantiation re-resolves the parsed
    /// parameter/return annotations, whose names (an imported generic's
    /// module-local types, e.g. react-hook-form's `UseFormProps`) are visible in
    /// the declaring file's per-file scope but not the caller's; the
    /// `module_scope_by_file` fallback keys on the active file name, so
    /// instantiation runs under this one.
    pub(crate) declaring_file: Option<Arc<str>>,
    /// Namespace the signature was declared in (`React` for `React.useState`),
    /// when it was published as a qualified namespace member. Its annotations
    /// name siblings unqualified (`Dispatch<SetStateAction<S>>`), which are
    /// stored under qualified keys, so instantiation re-resolves them under this
    /// prefix.
    pub(crate) namespace_prefix: Option<Arc<str>>,
    /// The overload of this group that declares a `x is T` return, when the
    /// signature kept for the group does not. Only guard narrowing reads it:
    /// the folded value type and every instantiation path are untouched, so
    /// carrying it cannot change which parameters a call binds.
    pub(crate) predicate_overload: Option<Arc<FunctionSignatureInfo>>,
    /// The group's *later* overloads, in declaration order, when this signature
    /// is the one kept for an overload group. A generic group re-resolves its
    /// parameter annotations at every call, which discards the folded parameter
    /// union and leaves only this signature's shape; instantiating these
    /// alongside restores the fold at the instantiated level, so an argument
    /// written for a later overload is not reported against the first.
    pub(crate) overload_alternatives: Vec<Arc<FunctionSignatureInfo>>,
    /// tsc's `getTypePredicateFromBody`: a function with no return annotation
    /// whose body is one `return` of a condition that splits a parameter's
    /// type exactly in two *is* a type predicate over that parameter (TS 5.5).
    /// Computed where the signature is collected, which is where the
    /// parameter types are known; guard narrowing reads it as it reads a
    /// written `x is T`.
    pub(crate) inferred_predicate: Option<InferredPredicate>,
    /// A generic declaration with a body and no written return type: tsc reads
    /// its return from the body (`getReturnTypeFromBody`) and instantiates it
    /// per call, so each call infers the body under its own type arguments.
    pub(crate) body_return: Option<Arc<BodyReturnSource>>,
    /// A generic class's construct signatures, which `new` resolves against:
    /// the class's value side is `any` (`generic_class_value_symbol`), so they
    /// ride on the signature that value carries.
    pub(crate) construct_signatures: Option<Arc<Vec<ConstructSignature>>>,
}

/// One construct signature of a generic class, written over the class's type
/// parameters. `template` holds what `signature` does not say: the parameter
/// count, the minimum argument count, the rest flag, and the type of each
/// parameter `signature` leaves unwritten.
#[derive(Debug, Clone)]
pub(crate) struct ConstructSignature {
    pub(crate) signature: Arc<FunctionSignatureInfo>,
    pub(crate) template: FunctionType,
}

/// The body behind [`FunctionSignatureInfo::body_return`], with the returns
/// read from it so far, one per set of type-parameter bindings — Go keeps a
/// signature's instantiations on the signature (`sig.instantiations`), so each
/// is computed once however many calls reach it.
#[derive(Debug)]
pub(crate) struct BodyReturnSource {
    pub(crate) function: surge_ts_syntax::ParsedFunctionDeclaration,
    pub(crate) instantiations: std::sync::Mutex<Vec<(Vec<Vec<(String, Type)>>, Type)>>,
    /// For a function declared in another's body, the scope it is read in:
    /// that body's bindings and local types (`interface Box` beside it), not
    /// only its module's.
    pub(crate) enclosing: Option<BodyReturnEnclosing>,
}

#[derive(Debug)]
pub(crate) struct BodyReturnEnclosing {
    /// The body's [`SymbolTable`], erased: a table holds the signatures that
    /// hold this, a cycle the auto-trait check cannot prove `Send` through.
    symbols: Arc<dyn std::any::Any + Send + Sync>,
    pub(crate) type_declaration_scope: Option<Arc<crate::symbols::TypeDeclarationScope>>,
}

impl BodyReturnEnclosing {
    pub(crate) fn new(symbols: SymbolTable, type_declaration_scope: Option<Arc<crate::symbols::TypeDeclarationScope>>) -> Self {
        Self { symbols: Arc::new(symbols), type_declaration_scope }
    }

    pub(crate) fn symbols(&self) -> Option<&SymbolTable> {
        self.symbols.downcast_ref::<SymbolTable>()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InferredPredicate {
    pub(crate) parameter_index: usize,
    pub(crate) target: Type,
}


/// One name of `const [error, value] = source`: element `index` of `source`.
/// `source` names a binding, whose current type is read when narrowing; a
/// pattern over any other expression keys its group by the pattern and keeps
/// the type that expression had.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TupleDestructureBinding {
    pub(crate) source: Arc<str>,
    pub(crate) key: DestructureKey,
    pub(crate) source_type: Option<Type>,
}

/// What a destructured binding reads off its source: a tuple element, or a
/// property of an object pattern (`const { kind, payload } = action`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DestructureKey {
    Index(usize),
    Property(Arc<str>),
}

impl DestructureKey {
    /// The type this key reads off one member of the source union.
    pub(crate) fn read(&self, member: &Type) -> Option<Type> {
        // A member reached through a reference or a brand (`LuaTuple<[A, B]>`,
        // a tuple intersected with an object) reads as the type it is.
        let member = match member {
            Type::Tuple(_) | Type::Object(_) => member.clone(),
            _ => member.peeled(),
        };
        let member = match &member {
            Type::Object(object) if object.is_intersection => object
                .intersection_operands
                .as_deref()
                .and_then(|operands| operands.iter().map(Type::peeled).find(|operand| matches!(operand, Type::Tuple(_))))
                .filter(|_| matches!(self, DestructureKey::Index(_)))
                .unwrap_or(member),
            _ => member,
        };
        match (self, &member) {
            // A fixed tuple too short for the index reads `undefined`
            // (`getBindingElementTypeFromParentType` with no bounds check).
            (DestructureKey::Index(index), Type::Tuple(elements)) => {
                Some(elements.get(*index).cloned().unwrap_or(Type::Undefined))
            }
            (DestructureKey::Property(name), Type::Object(object)) => {
                let property = object.properties.get(name)?;
                Some(if property.is_optional() {
                    surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
                } else {
                    property.ty.clone()
                })
            }
            _ => None,
        }
    }
}

/// A local tsc types by control flow under `noImplicitAny` — `let x;`,
/// `let x = undefined` (`autoType`) or `x = []` (`autoArrayType`). Its type
/// follows the assignments reaching each read; an evolving array's elements
/// grow with `push`, `unshift` and `x[n] = v`. A read that finds no type yet —
/// an element-less array, or a closure that cannot see the flow — is an
/// implicit `any`: TS7005, with TS7034 on the declaration. An array's element
/// types only ever grow along the flow, so the state lives beside the binding
/// instead of in branch frames.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AutoArrayBinding {
    pub(crate) name_span: Option<TextSpan>,
    pub(crate) is_const: bool,
    /// Declared `= []` (`autoArrayType`) rather than uninitialized (`autoType`),
    /// which is what a read that cannot see the flow is typed as.
    pub(crate) declared_array: bool,
    /// A `let` (tsc's mutable local), whose closures may continue its flow.
    pub(crate) is_let: bool,
    /// Declared with an initializer (`undefined`, `null` or `[]`), so never
    /// "never initialized".
    pub(crate) initialized: bool,
    /// How the parser saw the binding assigned, when it recorded it.
    pub(crate) assignments: Option<surge_ts_syntax::LetAssignmentSummary>,
    /// Holding an evolving array (`[]`, then its mutations) rather than
    /// whatever was last assigned.
    pub(crate) evolving: bool,
    /// What the mutations seen so far added; `never` until the first.
    pub(crate) elements: Type,
    /// A mutation whose element type surge could not settle. A read is then not
    /// provably element-less, so it is not reported.
    pub(crate) unsettled: bool,
    /// Loops being checked whose body mutates the binding with an element type
    /// that could not be settled before the body ran. tsc's loop fixed point
    /// sees those elements at every read in the body; surge does not report
    /// there.
    pub(crate) pending_loops: u32,
    /// Seen from a nested `function` declaration, which never continues the
    /// enclosing flow.
    pub(crate) declared_only: bool,
    /// Declared at the top level of a module, whose flow surge does not walk:
    /// a read there is element-less unless the parser saw a top-level mutation
    /// reach it (see [`surge_ts_syntax::ModuleArrayMutations`]).
    pub(crate) module_level: bool,
}

impl AutoArrayBinding {
    /// Whether a read here finds no element type yet — tsc's `autoArrayType`.
    pub(crate) fn is_element_less(&self) -> bool {
        self.evolving && self.elements == Type::Never && !self.unsettled && self.pending_loops == 0
    }

    /// Whether a closure reading the binding at `position` continues the
    /// enclosing flow (tsc: a `let` past its last assignment). Without the
    /// parser's record it is assumed to, which reports nothing.
    pub(crate) fn closure_sees_flow(&self, position: usize) -> bool {
        if self.declared_only || !self.is_let {
            return false;
        }
        match self.assignments {
            None => true,
            Some(summary) => summary
                .last_assignment
                .is_none_or(|last| last != u32::MAX && (last as usize) < position),
        }
    }

    /// A `let` nothing ever definitely assigns, read before any assignment
    /// could have run: tsc reads it as `undefined`, not as an implicit `any`.
    pub(crate) fn never_initialized(&self) -> bool {
        self.is_let
            && !self.initialized
            && self
                .assignments
                .is_some_and(|summary| !summary.definitely_assigned)
    }
}

/// A function implementation that its overload signatures hide from callers.
#[derive(Debug, Clone)]
pub(crate) struct HiddenImplementation {
    group: Type,
    signature: FunctionType,
}

#[derive(Debug, Default)]
pub(crate) struct SymbolTable {
    // Copy-on-write: clones share this map through `Arc` and only pay for a
    // deep copy if a *shared* table is later mutated. The multi-pass
    // module-binding fixpoint clones symbol tables thousands of times but
    // mutates almost none of those clones, so sharing makes the clones
    // effectively free and the deep copy is deferred to the rare mutate path.
    symbols: Arc<HashMap<Arc<str>, SymbolInfoHandle, FxBuildHasher>>,
    // Name spans of the *first* block-scoped declaration (let/const or function
    // implementation) registered in this scope, so a later redeclaration can
    // back-emit the duplicate diagnostic (TS2451/TS2393) at the original site
    // too — tsc flags every conflicting declaration, not just the latest. Shares
    // the same copy-on-write discipline as `symbols`; empty in nearly every
    // table, so the extra `Arc` clone is effectively free.
    declaration_spans: Arc<HashMap<Arc<str>, TextSpan, FxBuildHasher>>,
    // Names that already have a body-bearing function *implementation* registered
    // in this scope. A second implementation is the only thing that yields
    // TS2393 ("Duplicate function implementation"); bodyless declarations
    // (overload signatures, ambient `declare function`s) merge as overloads and
    // must not trip it. Shares the copy-on-write discipline of `symbols`.
    function_implementations: Arc<HashSet<Arc<str>, FxBuildHasher>>,
    // The implementations that overload signatures hide from callers, by name;
    // misses fall through to `parent`.
    hidden_implementations: Arc<HashMap<Arc<str>, HiddenImplementation, FxBuildHasher>>,
    // Declared type of a name whose entry in `symbols` currently holds a
    // *narrowed* type. An assignment is checked against the declared type, not
    // the flow-narrowed one (`let v: string | undefined = undefined; if (v ===
    // undefined) { v = "x" }` is legal), so narrowing records the pre-narrowing
    // type here. Populated only by the narrowing paths and dropped with the
    // narrowed table, so it is empty in nearly every table and its `Arc` clone
    // is effectively free. Shares the copy-on-write discipline of `symbols`.
    declared_types: Option<Arc<HashMap<Arc<str>, Type, FxBuildHasher>>>,
    // The condition a boolean `const` guard was written as (`const ok = a &&
    // isError(a)`), so a conditional expression tested on `ok` narrows by that
    // condition exactly as an `if (ok)` does. Written only by the function-body
    // declaration path; empty in nearly every table.
    alias_conditions: Option<Arc<HashMap<Arc<str>, Arc<ParsedExpression>, FxBuildHasher>>>,
    // `const [error, value] = tuple` binds dependent names: each records the
    // `(source, index)` it was destructured from, so a truthiness test on one
    // retypes the others. Same discipline as `alias_conditions`.
    // `None` marks a name redeclared since, which hides a parent's binding.
    tuple_destructures:
        Option<Arc<HashMap<Arc<str>, Option<TupleDestructureBinding>, FxBuildHasher>>>,
    // Function-local `[]`-initialized bindings tsc types by control flow
    // (`autoArrayType`); see [`AutoArrayBinding`]. Empty in nearly every table.
    auto_arrays: Option<Arc<HashMap<Arc<str>, AutoArrayBinding, FxBuildHasher>>>,
    // A function body's table: a binding found past it belongs to an enclosing
    // function or the module, not to this flow.
    function_boundary: bool,
    // Optional read-only fallback consulted by lookups (`get`, `get_handle`,
    // `contains_let_or_const`) when a name is absent from `symbols`. A function
    // body's root scope sets this to the module/ambient environment instead of
    // copying every visible symbol into its own map, so opening a scope inside a
    // file with N module-level symbols stays O(1) rather than O(N) (and the COW
    // on the first local insert clones only the small local map, not the parent).
    // Mutations and `iter*` operate on `symbols` alone; `parent` is only ever set
    // on transient, lookup-only scope roots that are never iterated.
    parent: Option<Arc<SymbolTable>>,
    // A block-scoped declaration boundary over `parent`: a namespace body is
    // its own scope, so an enclosing `let`/`const` of the same name is shadowed
    // rather than redeclared.
    declaration_scope_root: bool,
}

impl Clone for SymbolTable {
    fn clone(&self) -> Self {
        record_symbol_table_clone_count();
        // Handle/entry copies are now recorded only when a shared table is
        // actually mutated (see `symbols_mut`), since the clone itself copies
        // nothing.
        Self {
            symbols: Arc::clone(&self.symbols),
            declaration_spans: Arc::clone(&self.declaration_spans),
            function_implementations: Arc::clone(&self.function_implementations),
            hidden_implementations: Arc::clone(&self.hidden_implementations),
            declared_types: self.declared_types.clone(),
            alias_conditions: self.alias_conditions.clone(),
            tuple_destructures: self.tuple_destructures.clone(),
            auto_arrays: self.auto_arrays.clone(),
            function_boundary: self.function_boundary,
            parent: self.parent.clone(),
            declaration_scope_root: self.declaration_scope_root,
        }
    }
}

impl SymbolTable {
    pub(crate) fn clone_with_reason(&self, reason: TypeCopyReason) -> Self {
        with_type_copy_reason(reason, || self.clone())
    }

    /// Clone for a declaration-environment capture: shares the symbol map (and
    /// parent) but drops the declaration-span / function-implementation maps.
    /// Environment-recovered contexts only resolve types — they never run the
    /// duplicate-declaration checks that read those maps — and capturing the
    /// span map would both retain a snapshot per environment and force the live
    /// table's next `record_declaration_span` into a full copy-on-write copy.
    pub(crate) fn clone_for_environment_capture(&self) -> Self {
        record_symbol_table_clone_count();
        let empty = Self::empty_for_environment_capture();
        Self {
            symbols: Arc::clone(&self.symbols),
            declaration_spans: empty.declaration_spans,
            function_implementations: empty.function_implementations,
            hidden_implementations: empty.hidden_implementations,
            declared_types: self.declared_types.clone(),
            alias_conditions: self.alias_conditions.clone(),
            tuple_destructures: self.tuple_destructures.clone(),
            auto_arrays: None,
            function_boundary: false,
            parent: self.parent.clone(),
            declaration_scope_root: self.declaration_scope_root,
        }
    }

    /// An empty table whose maps are shared by every environment capture, so a
    /// capture allocates none (tRPC interns ~74k environments).
    pub(crate) fn empty_for_environment_capture() -> Self {
        static EMPTY: std::sync::OnceLock<SymbolTable> = std::sync::OnceLock::new();
        EMPTY.get_or_init(SymbolTable::default).clone()
    }

    /// Build a lookup-only scope whose own map is empty and whose misses fall
    /// through to `parent`. See the `parent` field. Used for function-body roots.
    pub(crate) fn with_parent(parent: Arc<SymbolTable>) -> Self {
        Self {
            symbols: Arc::new(HashMap::default()),
            declaration_spans: Arc::new(HashMap::default()),
            function_implementations: Arc::new(HashSet::default()),
            hidden_implementations: Arc::new(HashMap::default()),
            declared_types: None,
            alias_conditions: None,
            tuple_destructures: None,
            auto_arrays: None,
            function_boundary: false,
            parent: Some(parent),
            declaration_scope_root: false,
        }
    }

    /// Return this table (sharing its own map) with `parent` attached as the
    /// lookup fallback. The own entries keep precedence over `parent`.
    pub(crate) fn with_parent_fallback(mut self, parent: Arc<SymbolTable>) -> Self {
        self.parent = Some(parent);
        self
    }

    /// A per-file check root: empty own symbol map with `parent` (the ambient
    /// globals) as the lookup fallback, sharing the parent's declaration-span
    /// and function-implementation maps copy-on-write so the own-map-only
    /// duplicate checks see exactly what a full clone would have seen. Unlike
    /// a clone, the first file-local insert copies only the small own map,
    /// not every global.
    pub(crate) fn file_check_root(parent: Arc<SymbolTable>) -> Self {
        record_symbol_table_clone_count();
        Self {
            symbols: Arc::new(HashMap::default()),
            declaration_spans: Arc::clone(&parent.declaration_spans),
            function_implementations: Arc::clone(&parent.function_implementations),
            hidden_implementations: Arc::clone(&parent.hidden_implementations),
            declared_types: parent.declared_types.clone(),
            alias_conditions: parent.alias_conditions.clone(),
            tuple_destructures: parent.tuple_destructures.clone(),
            auto_arrays: None,
            function_boundary: false,
            parent: Some(parent),
            declaration_scope_root: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolKind {
    Var,
    Let,
    Const,
    Function,
    Parameter,
    /// The key of a `for…in` over an object whose only index signature is
    /// numeric. Its type is `string` like any other key; the kind records what
    /// tsc's `isForInVariableForNumericPropertyNames` asks at an access through
    /// it — that the key indexes as a `number`.
    ForInNumericKey,
    /// A binding stubbed for an import whose module was reported unresolved.
    /// Its type is tsc's error type (`any`), and unlike an inferred `any` that
    /// records a surge modelling failure, this one is what the source really
    /// says — so a callback passed to a call on it genuinely has no contextual
    /// type and its parameters are implicit-any.
    ErrorImport,
}

impl SymbolTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns a uniquely-owned mutable view of the underlying map, performing
    /// the copy-on-write deep copy only if this table currently shares its map
    /// with a clone. The deep copy is what the per-entry handle-copy counters
    /// now measure.
    fn symbols_mut(&mut self) -> &mut HashMap<Arc<str>, SymbolInfoHandle, FxBuildHasher> {
        if Arc::strong_count(&self.symbols) > 1 {
            let entry_count = self.symbols.len() as u64;
            record_symbol_table_entry_handle_copy_count(entry_count);
            record_symbol_info_handle_copy_count(entry_count);
        }
        Arc::make_mut(&mut self.symbols)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&SymbolInfo> {
        record_type_name_lookup_string_count(1);
        match self.symbols.get(name) {
            Some(symbol) => Some(symbol.as_ref()),
            None => self.parent.as_ref().and_then(|parent| parent.get(name)),
        }
    }

    pub(crate) fn get_handle(&self, name: &str) -> Option<SymbolInfoHandle> {
        record_type_name_lookup_string_count(1);
        if let Some(symbol) = self.symbols.get(name) {
            record_symbol_info_handle_copy_count(1);
            return Some(Arc::clone(symbol));
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.get_handle(name))
    }

    pub(crate) fn get_shared(&self, name: &str) -> Option<SymbolInfoHandle> {
        self.get_handle(name)
    }

    /// Own-map-only lookup; never consults `parent`.
    ///
    /// Export collection attaches the ambient globals as a `parent` fallback, so
    /// `get`/`get_shared` answer "is this name visible here?", not "did this module
    /// declare it?". An own-ness test built on the visible-name question lets a
    /// same-named global suppress the module's own declaration, after which the
    /// module exports the global instead (`next/font`'s `Content` displacing
    /// radix's `declare const Content`).
    /// [`Self::get_own`] as a shared handle.
    pub(crate) fn get_own_handle(&self, name: &str) -> Option<SymbolInfoHandle> {
        self.symbols.get(name).cloned()
    }

    pub(crate) fn get_own(&self, name: &str) -> Option<&SymbolInfo> {
        record_type_name_lookup_string_count(1);
        self.symbols.get(name).map(|symbol| symbol.as_ref())
    }

    pub(crate) fn get_own_shared(&self, name: &str) -> Option<SymbolInfoHandle> {
        record_type_name_lookup_string_count(1);
        let symbol = self.symbols.get(name)?;
        record_symbol_info_handle_copy_count(1);
        Some(Arc::clone(symbol))
    }

    pub(crate) fn insert(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
    ) -> Option<SymbolInfoHandle> {
        self.symbols_mut().insert(name.into(), Arc::new(symbol))
    }

    /// Install a flow-narrowed type for `name`, remembering the type it was
    /// narrowed *from* so assignments keep checking against the declaration. A
    /// name narrowed twice (nested guards) keeps the outermost record.
    pub(crate) fn insert_narrowed(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfo,
        declared: Type,
    ) -> Option<SymbolInfoHandle> {
        let name = name.into();
        // Recorded on the table that holds the narrowed symbol: a lookup stops
        // at the first table holding the name, so a record left on a parent is
        // lost under a nested guard.
        let outermost = self.declared_type(&name).cloned().unwrap_or(declared);
        let declared_types = self.declared_types.get_or_insert_with(Default::default);
        if !declared_types.contains_key(&name) {
            Arc::make_mut(declared_types).insert(Arc::clone(&name), outermost);
        }
        self.insert(name, symbol)
    }

    /// Record (or clear) the pre-narrowing type of `name` on this table's own
    /// map, overwriting any existing entry. `insert_narrowed` is the
    /// keep-the-outermost variant; scope-stack narrowing needs the explicit form
    /// so `pop_child` can restore what the branch shadowed.
    pub(crate) fn set_declared_type(&mut self, name: Arc<str>, declared: Option<Type>) {
        match declared {
            Some(declared) => {
                let declared_types = self.declared_types.get_or_insert_with(Default::default);
                Arc::make_mut(declared_types).insert(name, declared);
            }
            None => {
                if let Some(declared_types) = self.declared_types.as_mut()
                    && declared_types.contains_key(&name)
                {
                    Arc::make_mut(declared_types).remove(&name);
                }
            }
        }
    }

    /// The pre-narrowing type of `name`, when a narrowing path replaced its
    /// entry. `None` means the symbol's own type *is* its declared type.
    ///
    /// Walks the parent chain in lockstep with `get`, so a scope that declares
    /// its own `name` (an inner function shadowing a narrowed outer binding)
    /// stops the search rather than inheriting the outer declaration's type.
    /// The names this table itself narrowed, i.e. the ones whose entry holds a
    /// narrowed type with the pre-narrowing type recorded beside it.
    pub(crate) fn narrowed_names(&self) -> impl Iterator<Item = &Arc<str>> {
        self.declared_types
            .as_ref()
            .into_iter()
            .flat_map(|declared_types| declared_types.keys())
    }

    pub(crate) fn set_alias_condition(
        &mut self,
        name: Arc<str>,
        condition: Option<Arc<ParsedExpression>>,
    ) {
        match condition {
            Some(condition) => {
                let conditions = self.alias_conditions.get_or_insert_with(Default::default);
                Arc::make_mut(conditions).insert(name, condition);
            }
            None => {
                if let Some(conditions) = self.alias_conditions.as_mut()
                    && conditions.contains_key(&name)
                {
                    Arc::make_mut(conditions).remove(&name);
                }
            }
        }
    }

    pub(crate) fn set_tuple_destructure(
        &mut self,
        name: Arc<str>,
        binding: Option<TupleDestructureBinding>,
    ) {
        let bindings = self.tuple_destructures.get_or_insert_with(Default::default);
        Arc::make_mut(bindings).insert(name, binding);
    }

    pub(crate) fn tuple_destructure(&self, name: &str) -> Option<TupleDestructureBinding> {
        if let Some(binding) = self
            .tuple_destructures
            .as_ref()
            .and_then(|bindings| bindings.get(name))
        {
            return binding.clone();
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.tuple_destructure(name))
    }

    /// Every binding destructured out of `source`, with its index.
    pub(crate) fn tuple_destructure_siblings(&self, source: &str) -> Vec<(Arc<str>, DestructureKey)> {
        let mut names: Vec<Arc<str>> = Vec::new();
        let mut table = Some(self);
        while let Some(current) = table {
            for name in current.tuple_destructures.iter().flat_map(|bindings| bindings.keys()) {
                if !names.contains(name) {
                    names.push(Arc::clone(name));
                }
            }
            table = current.parent.as_deref();
        }
        let mut siblings: Vec<(Arc<str>, DestructureKey)> = names
            .into_iter()
            .filter_map(|name| {
                let binding = self.tuple_destructure(&name)?;
                (&*binding.source == source).then_some((name, binding.key))
            })
            .collect();
        siblings.sort_by(|left, right| left.0.cmp(&right.0));
        siblings
    }

    /// The evolving-array state of `name`, and whether it belongs to an
    /// enclosing function (or the module) rather than this one. Like
    /// `declared_type`, a scope that declares its own `name` stops the search.
    pub(crate) fn auto_array(&self, name: &str) -> Option<(&AutoArrayBinding, bool)> {
        if self.symbols.contains_key(name) {
            return self
                .auto_arrays
                .as_ref()
                .and_then(|bindings| bindings.get(name))
                .map(|binding| (binding, false));
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.auto_array(name))
            .map(|(binding, crossed)| (binding, crossed || self.function_boundary))
    }

    pub(crate) fn mark_function_boundary(&mut self) {
        self.function_boundary = true;
    }

    /// Whether `name` is bound by an enclosing function (or the module) rather
    /// than this one; `None` when nothing binds it.
    pub(crate) fn binding_crosses_function_boundary(&self, name: &str) -> Option<bool> {
        if self.symbols.contains_key(name) {
            return Some(false);
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.binding_crosses_function_boundary(name))
            .map(|crossed| crossed || self.function_boundary)
    }

    /// Whether `name` is bound by the outermost table (the file's or the
    /// program's globals) rather than a nested scope; `None` when nothing
    /// binds it.
    pub(crate) fn binding_is_root(&self, name: &str) -> Option<bool> {
        if self.symbols.contains_key(name) {
            return Some(self.parent.is_none());
        }
        self.parent.as_ref().and_then(|parent| parent.binding_is_root(name))
    }

    pub(crate) fn has_auto_arrays(&self) -> bool {
        self.auto_arrays
            .as_ref()
            .is_some_and(|bindings| !bindings.is_empty())
    }

    /// Installs (or, with `None`, clears) this table's own entry for `name`,
    /// returning the entry it replaced.
    pub(crate) fn set_auto_array(
        &mut self,
        name: Arc<str>,
        binding: Option<AutoArrayBinding>,
    ) -> Option<AutoArrayBinding> {
        match binding {
            Some(binding) => {
                let bindings = self.auto_arrays.get_or_insert_with(Default::default);
                Arc::make_mut(bindings).insert(name, binding)
            }
            None => match self.auto_arrays.as_mut() {
                Some(bindings) if bindings.contains_key(&name) => {
                    Arc::make_mut(bindings).remove(&name)
                }
                _ => None,
            },
        }
    }

    /// Marks this table's flow-typed bindings as seen only at their declared
    /// types, as a nested `function` declaration sees them.
    pub(crate) fn mark_auto_arrays_declared_only(&mut self) {
        if let Some(bindings) = self.auto_arrays.as_mut() {
            for binding in Arc::make_mut(bindings).values_mut() {
                binding.declared_only = true;
            }
        }
    }

    pub(crate) fn auto_array_mut(&mut self, name: &str) -> Option<&mut AutoArrayBinding> {
        let bindings = self.auto_arrays.as_mut()?;
        if !bindings.contains_key(name) {
            return None;
        }
        Arc::make_mut(bindings).get_mut(name)
    }

    pub(crate) fn alias_condition(&self, name: &str) -> Option<Arc<ParsedExpression>> {
        if self.symbols.contains_key(name) {
            return self
                .alias_conditions
                .as_ref()
                .and_then(|conditions| conditions.get(name))
                .cloned();
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.alias_condition(name))
    }

    pub(crate) fn declared_type(&self, name: &str) -> Option<&Type> {
        if self.symbols.contains_key(name) {
            return self
                .declared_types
                .as_ref()
                .and_then(|declared_types| declared_types.get(name));
        }
        self.parent
            .as_ref()
            .and_then(|parent| parent.declared_type(name))
    }

    pub(crate) fn insert_handle(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfoHandle,
    ) -> Option<SymbolInfoHandle> {
        self.symbols_mut().insert(name.into(), symbol)
    }

    pub(crate) fn insert_shared(
        &mut self,
        name: impl Into<Arc<str>>,
        symbol: SymbolInfoHandle,
    ) -> Option<SymbolInfoHandle> {
        self.insert_handle(name, symbol)
    }

    pub(crate) fn remove(&mut self, name: &str) -> Option<SymbolInfoHandle> {
        self.symbols_mut().remove(name)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &SymbolInfo)> {
        self.symbols
            .iter()
            .map(|(name, symbol)| (name, symbol.as_ref()))
    }

    /// Every name a lookup through this table can reach, parents included.
    /// A name shadowed by a nearer table appears more than once.
    pub(crate) fn visible_names(&self) -> impl Iterator<Item = &Arc<str>> {
        std::iter::successors(Some(self), |table| table.parent.as_deref())
            .flat_map(|table| table.symbols.keys())
    }

    pub(crate) fn iter_handles(&self) -> impl Iterator<Item = (&Arc<str>, &SymbolInfoHandle)> {
        self.symbols.iter()
    }

    pub(crate) fn iter_shared(&self) -> impl Iterator<Item = (&Arc<str>, &SymbolInfoHandle)> {
        self.iter_handles()
    }

    /// Census-only identity/footprint probes. Addresses identify the shared
    /// copy-on-write maps so pointer-deduplicating walks charge each map once.
    pub(crate) fn symbols_map_address(&self) -> usize {
        Arc::as_ptr(&self.symbols) as usize
    }

    pub(crate) fn declaration_spans_map_address(&self) -> usize {
        Arc::as_ptr(&self.declaration_spans) as usize
    }

    pub(crate) fn declaration_spans_footprint(&self) -> (u64, u64) {
        let entries = self.declaration_spans.len() as u64;
        let bytes = entries
            * (std::mem::size_of::<Arc<str>>() + std::mem::size_of::<TextSpan>() + 16) as u64;
        (entries, bytes)
    }

    pub(crate) fn parent_table(&self) -> Option<&SymbolTable> {
        self.parent.as_deref()
    }

    pub(crate) fn contains_let_or_const(&self, name: &str) -> bool {
        record_type_name_lookup_string_count(1);
        if let Some(existing) = self.symbols.get(name) {
            return matches!(existing.as_ref().kind, SymbolKind::Let | SymbolKind::Const);
        }
        !self.declaration_scope_root
            && self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.contains_let_or_const(name))
    }

    /// A lookup-only scope over `parent` that starts a new block-scoped
    /// declaration space (see `declaration_scope_root`).
    pub(crate) fn declaration_scope(parent: Arc<SymbolTable>) -> Self {
        let mut table = Self::with_parent(parent);
        table.declaration_scope_root = true;
        table
    }

    /// Records the name span of the first declaration of `name` in this scope so
    /// a later redeclaration can back-emit its duplicate diagnostic at the
    /// original site. Only the first recording is kept.
    pub(crate) fn record_declaration_span(&mut self, name: &str, span: TextSpan) {
        if self.declaration_spans.contains_key(name) {
            return;
        }
        Arc::make_mut(&mut self.declaration_spans).insert(name.into(), span);
    }

    /// Removes and returns the recorded first-declaration span for `name`, if
    /// any. Removing ensures a third+ redeclaration does not re-emit at a site
    /// already flagged.
    pub(crate) fn take_declaration_span(&mut self, name: &str) -> Option<TextSpan> {
        if !self.declaration_spans.contains_key(name) {
            return None;
        }
        Arc::make_mut(&mut self.declaration_spans).remove(name)
    }

    /// Whether a body-bearing function implementation for `name` was already
    /// registered in this scope (see [`Self::mark_function_implementation`]).
    pub(crate) fn has_function_implementation(&self, name: &str) -> bool {
        self.function_implementations.contains(name)
    }

    /// Marks that a body-bearing function implementation for `name` exists in
    /// this scope, so a *second* implementation can be reported as TS2393.
    pub(crate) fn mark_function_implementation(&mut self, name: &str) {
        if self.function_implementations.contains(name) {
            return;
        }
        Arc::make_mut(&mut self.function_implementations).insert(name.into());
    }

    /// Records the signature of `name`'s implementation, which the overload
    /// group `group` hides from callers.
    pub(crate) fn record_hidden_function_implementation(&mut self, name: &str, group: Type, signature: FunctionType) {
        Arc::make_mut(&mut self.hidden_implementations).insert(name.into(), HiddenImplementation { group, signature });
    }

    /// The implementation behind the overloads `group` that `name` is bound to
    /// here, from the nearest scope recording one for that name.
    pub(crate) fn hidden_function_implementation(&self, name: &str, group: &Type) -> Option<&FunctionType> {
        match self.hidden_implementations.get(name) {
            Some(hidden) => (hidden.group == *group).then_some(&hidden.signature),
            None => self
                .parent
                .as_ref()
                .and_then(|parent| parent.hidden_function_implementation(name, group)),
        }
    }
}

pub(crate) fn clone_symbol_info_handle(symbol: &SymbolInfoHandle) -> SymbolInfoHandle {
    record_symbol_info_handle_copy_count(1);
    Arc::clone(symbol)
}

pub(crate) fn map_symbol_kind(parsed_kind: surge_ts_syntax::ParsedVariableKind) -> SymbolKind {
    match parsed_kind {
        surge_ts_syntax::ParsedVariableKind::Var => SymbolKind::Var,
        surge_ts_syntax::ParsedVariableKind::Let => SymbolKind::Let,
        surge_ts_syntax::ParsedVariableKind::Const => SymbolKind::Const,
    }
}
