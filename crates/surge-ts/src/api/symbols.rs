//! Symbols: what the binder declared in each file, the global scope those
//! declarations merge into, and name resolution over both — tsc's
//! `resolveName`, run over this port's binder tables.

use std::sync::Arc;

use surge_ts_tsc_syntax::symbol_flags as sf;
use surge_ts_tsc_syntax::{Kind, SyntaxTree};
use surge_ts_types::fx::FxHashMap;

use super::checker::{SemanticTables, TypeChecker};
use super::handles::{NodeId, SymbolId, TypeId};
use super::program::Program;

/// What a [`SymbolId`] names.
#[derive(Clone, Debug)]
pub(crate) enum SymbolData {
    /// A symbol a file's binder declared.
    Bound { file: u32, symbol: usize },
    /// A global name, merged across every file that declares it.
    Global { name: Arc<str>, parts: Vec<(u32, usize)> },
    /// A symbol no binder made: a property the checker computed, a
    /// signature's parameter. An instantiated member keeps its declaration;
    /// a union's property keeps one per constituent declaring it.
    Transient {
        name: Arc<str>,
        flags: i32,
        ty: TypeId,
        parent: Option<SymbolId>,
        declarations: Vec<NodeId>,
    },
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum SymbolKey {
    Bound(u32, usize),
    Global(Arc<str>),
    Property(TypeId, Arc<str>),
    Parameter(u32, u32),
}

/// The program's global scope: every script's top-level declarations, every
/// `declare global` block's, every ambient module's.
#[derive(Default)]
pub(crate) struct GlobalScope {
    names: FxHashMap<Arc<str>, Vec<(u32, usize)>>,
}

impl GlobalScope {
    fn build(program: &Program) -> Self {
        let mut names: FxHashMap<Arc<str>, Vec<(u32, usize)>> = FxHashMap::default();
        let mut merged_flags: FxHashMap<Arc<str>, u32> = FxHashMap::default();
        for file in program.source_files() {
            let syntax = program.syntax(file);
            let tree = &syntax.tree;
            let bound = &syntax.bound;
            let mut add = |name: &str, symbol: usize| {
                // tsc's `mergeSymbol` leaves a declaration the global it would
                // merge into excludes (`let x` in two scripts) out of it, a
                // symbol of its own.
                let flags = bound.symbols[symbol].flags;
                let merged = merged_flags.entry(Arc::from(name)).or_default();
                if *merged != 0
                    && *merged & surge_ts_tsc_syntax::excluded_symbol_flags(flags) != 0
                    && (*merged | flags) & sf::Assignment == 0
                {
                    return;
                }
                *merged |= flags;
                names.entry(Arc::from(name)).or_default().push((file.index, symbol));
            };
            if !tree.is_external_module() {
                if let Some(locals) = bound.locals.get(&tree.root()) {
                    for (name, &symbol) in locals {
                        add(name, symbol);
                    }
                }
                continue;
            }
            for statement in statements(tree, tree.root()) {
                if tree.kind(statement) != Kind::ModuleDeclaration {
                    continue;
                }
                let Some(&symbol) = bound.node_symbol.get(&statement) else { continue };
                let name_node = tree.node(statement).name;
                let is_global = name_node.is_some_and(|name| tree.kind(name) == Kind::Identifier && tree.text(name) == "global");
                if is_global {
                    if let Some(exports) = bound.exports.get(&symbol) {
                        for (name, &member) in exports {
                            add(name, member);
                        }
                    }
                } else if name_node.is_some_and(|name| tree.kind(name) == Kind::StringLiteral) {
                    add(&bound.symbols[symbol].name, symbol);
                }
            }
        }
        GlobalScope { names }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&[(u32, usize)]> {
        self.names.get(name).map(Vec::as_slice)
    }
}

pub(crate) fn statements(tree: &SyntaxTree, container: u32) -> Vec<u32> {
    tree.node(container)
        .lists
        .first()
        .and_then(|list| list.as_ref())
        .map(|list| list.nodes.clone())
        .unwrap_or_default()
}

/// tsc's `getMeaningFromLocation`, reduced to the three meanings name
/// resolution distinguishes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Meaning {
    Value,
    Type,
    Namespace,
}

impl Meaning {
    fn flags(self) -> u32 {
        match self {
            Meaning::Value => sf::Value,
            Meaning::Type => sf::Type,
            Meaning::Namespace => sf::Namespace,
        }
    }
}

impl TypeChecker {
    pub(crate) fn with_tables<R>(&self, f: impl FnOnce(&mut SemanticTables) -> R) -> R {
        let mut tables = self.program.inner().semantics.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut tables)
    }

    pub(crate) fn symbol_id(&self, key: SymbolKey, data: impl FnOnce() -> SymbolData) -> SymbolId {
        let program = self.program.inner().identity;
        self.with_tables(|tables| {
            if let Some(&index) = tables.symbol_by_key.get(&key) {
                return SymbolId { program, index };
            }
            let index = tables.symbols.len() as u32;
            tables.symbols.push(data());
            tables.symbol_by_key.insert(key, index);
            SymbolId { program, index }
        })
    }

    pub(crate) fn symbol_data(&self, symbol: SymbolId) -> SymbolData {
        assert_eq!(symbol.program, self.program.inner().identity, "symbol handle belongs to another program");
        self.with_tables(|tables| tables.symbols[symbol.index as usize].clone())
    }

    /// The handle for a symbol a file's binder declared; a global symbol
    /// several files contribute to is its merged global symbol.
    pub(crate) fn bound_symbol(&self, file: u32, symbol: usize) -> SymbolId {
        let syntax = self.program.syntax(self.program.file_id(file));
        let bound = &syntax.bound.symbols[symbol];
        let is_global_declaration = bound.parent.is_none()
            && bound.declarations.first().is_some_and(|&declaration| {
                let tree = &syntax.tree;
                let container = enclosing_container(tree, declaration);
                (container == tree.root() && !tree.is_external_module()) || self.is_global_augmentation(tree, container)
            });
        if is_global_declaration {
            let name: Arc<str> = Arc::from(bound.name.as_str());
            if let Some(parts) = self.global_scope_entry(&name)
                && parts.len() > 1
                && parts.contains(&(file, symbol))
            {
                return self.symbol_id(SymbolKey::Global(name.clone()), || SymbolData::Global { name, parts });
            }
        }
        self.symbol_id(SymbolKey::Bound(file, symbol), || SymbolData::Bound { file, symbol })
    }

    fn is_global_augmentation(&self, tree: &SyntaxTree, node: u32) -> bool {
        tree.kind(node) == Kind::ModuleDeclaration
            && tree.node(node).name.is_some_and(|name| tree.kind(name) == Kind::Identifier && tree.text(name) == "global")
    }

    pub(crate) fn global_scope_entry(&self, name: &str) -> Option<Vec<(u32, usize)>> {
        let built = self.with_tables(|tables| tables.globals.is_some());
        if !built {
            let scope = GlobalScope::build(&self.program);
            self.with_tables(|tables| {
                tables.globals.get_or_insert(scope);
            });
        }
        self.with_tables(|tables| tables.globals.as_ref().and_then(|globals| globals.get(name)).map(<[_]>::to_vec))
    }

    pub(crate) fn global_symbol(&self, name: &str, meaning: Meaning) -> Option<SymbolId> {
        let parts = self.global_scope_entry(name)?;
        let flags = parts
            .iter()
            .map(|&(file, symbol)| self.program.syntax(self.program.file_id(file)).bound.symbols[symbol].flags)
            .fold(0, |all, flags| all | flags);
        if flags & (meaning.flags() | sf::Alias) == 0 {
            return None;
        }
        Some(match parts.as_slice() {
            [(file, symbol)] => self.symbol_id(SymbolKey::Bound(*file, *symbol), || SymbolData::Bound { file: *file, symbol: *symbol }),
            _ => {
                let name: Arc<str> = Arc::from(name);
                self.symbol_id(SymbolKey::Global(name.clone()), || SymbolData::Global { name, parts })
            }
        })
    }

    /// tsc's `resolveName` from `location` for `name` with `meaning`.
    pub(crate) fn resolve_name(&self, location: NodeId, name: &str, meaning: Meaning) -> Option<SymbolId> {
        let file = location.source_file();
        let syntax = self.program.syntax(file).clone();
        let tree = &syntax.tree;
        let bound = &syntax.bound;
        let wanted = meaning.flags();
        let lookup = |table: Option<&std::collections::HashMap<String, usize>>, mask: u32| -> Option<usize> {
            let &symbol = table?.get(name)?;
            let flags = bound.symbols[symbol].flags;
            (flags & mask != 0 || flags & sf::Alias != 0).then_some(symbol)
        };
        let mut node = Some(location.node);
        let mut previous: Option<u32> = None;
        while let Some(current) = node {
            let kind = tree.kind(current);
            let is_global_file = current == tree.root() && !tree.is_external_module();
            if !is_global_file
                && let Some(symbol) = lookup(bound.locals.get(&current), wanted)
            {
                return Some(self.bound_symbol(file.index, symbol));
            }
            match kind {
                Kind::SourceFile if tree.is_external_module() => {
                    let exports = bound.node_symbol.get(&current).and_then(|symbol| bound.exports.get(symbol));
                    if name != "default"
                        && let Some(symbol) = lookup(exports, wanted & (sf::Value | sf::Type | sf::Namespace))
                    {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                }
                Kind::ModuleDeclaration => {
                    let exports = bound.node_symbol.get(&current).and_then(|symbol| bound.exports.get(symbol));
                    if name != "default"
                        && let Some(symbol) = lookup(exports, wanted & (sf::Value | sf::Type | sf::Namespace))
                    {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                }
                Kind::EnumDeclaration => {
                    let exports = bound.node_symbol.get(&current).and_then(|symbol| bound.exports.get(symbol));
                    if let Some(symbol) = lookup(exports, wanted & sf::EnumMember) {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                }
                Kind::ClassDeclaration | Kind::ClassExpression | Kind::InterfaceDeclaration => {
                    // Type parameters of a class or interface live in its
                    // members; only a use outside its own members' names
                    // sees them.
                    let members = bound.node_symbol.get(&current).and_then(|symbol| bound.members.get(symbol));
                    if let Some(symbol) = lookup(members, wanted & sf::Type)
                        && bound.symbols[symbol].flags & sf::TypeParameter != 0
                    {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                    if kind == Kind::ClassExpression
                        && wanted & sf::Class != 0
                        && let Some(class_name) = tree.node(current).name
                        && tree.text(class_name) == name
                        && let Some(&symbol) = bound.node_symbol.get(&current)
                    {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                }
                Kind::FunctionExpression => {
                    if wanted & sf::Function != 0
                        && let Some(function_name) = tree.node(current).name
                        && tree.text(function_name) == name
                        && let Some(&symbol) = bound.node_symbol.get(&current)
                    {
                        return Some(self.bound_symbol(file.index, symbol));
                    }
                }
                _ => {}
            }
            previous = Some(current);
            node = tree.parent(current);
        }
        let _ = previous;
        self.global_symbol(name, meaning)
    }
}

/// The node whose locals, exports or members a declaration was bound into:
/// its nearest container.
pub(crate) fn enclosing_container(tree: &SyntaxTree, declaration: u32) -> u32 {
    let mut node = tree.parent(declaration);
    while let Some(current) = node {
        if matches!(
            tree.kind(current),
            Kind::SourceFile
                | Kind::ModuleBlock
                | Kind::ModuleDeclaration
                | Kind::ClassDeclaration
                | Kind::ClassExpression
                | Kind::InterfaceDeclaration
                | Kind::TypeLiteral
                | Kind::EnumDeclaration
                | Kind::Block
                | Kind::FunctionDeclaration
                | Kind::FunctionExpression
                | Kind::ArrowFunction
                | Kind::MethodDeclaration
                | Kind::Constructor
                | Kind::GetAccessor
                | Kind::SetAccessor
        ) {
            return if tree.kind(current) == Kind::ModuleBlock { tree.parent(current).unwrap_or(current) } else { current };
        }
        node = tree.parent(current);
    }
    tree.root()
}

impl Program {
    /// The binder symbol a declaration node was added to (tsc's
    /// `node.symbol`).
    pub(crate) fn declared_symbol_index(&self, node: NodeId) -> Option<usize> {
        self.syntax(node.source_file()).bound.node_symbol.get(&node.node).copied()
    }
}
