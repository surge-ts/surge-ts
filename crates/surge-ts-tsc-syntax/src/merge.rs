//! The checker's merge of every file's global declarations into one symbol
//! table (`initializeChecker`, `mergeGlobalSymbol`, `mergeSymbol`,
//! `mergeModuleAugmentation`, `reportMergeSymbolError`), with the alias
//! resolution merging reads (`resolveAlias`), kept to the errors they report:
//! declarations in different files that cannot share a name, and aliases
//! whose resolution comes back to themselves.
//!
//! What only the checker's types answer is left out: a named import from a
//! module with `export =` (a property of its type), a default import, and an
//! augmentation of a pattern ambient module.

use std::collections::{HashMap, HashSet};

use crate::messages as diagnostics;
use crate::{Diagnostic, Message, SyntaxDiagnostic};

#[allow(non_upper_case_globals, dead_code)]
mod symbol_flags {
    pub const FunctionScopedVariable: u32 = 1 << 0;
    pub const BlockScopedVariable: u32 = 1 << 1;
    pub const Property: u32 = 1 << 2;
    pub const EnumMember: u32 = 1 << 3;
    pub const Function: u32 = 1 << 4;
    pub const Class: u32 = 1 << 5;
    pub const Interface: u32 = 1 << 6;
    pub const ConstEnum: u32 = 1 << 7;
    pub const RegularEnum: u32 = 1 << 8;
    pub const ValueModule: u32 = 1 << 9;
    pub const NamespaceModule: u32 = 1 << 10;
    pub const TypeLiteral: u32 = 1 << 11;
    pub const ObjectLiteral: u32 = 1 << 12;
    pub const Method: u32 = 1 << 13;
    pub const GetAccessor: u32 = 1 << 15;
    pub const SetAccessor: u32 = 1 << 16;
    pub const TypeParameter: u32 = 1 << 18;
    pub const TypeAlias: u32 = 1 << 19;
    pub const Alias: u32 = 1 << 21;
    pub const Assignment: u32 = 1 << 26;
    pub const ReplaceableByMethod: u32 = 1 << 29;
    pub const All: u32 = (1 << 30) - 1;

    pub const Enum: u32 = RegularEnum | ConstEnum;
    pub const Variable: u32 = FunctionScopedVariable | BlockScopedVariable;
    pub const Value: u32 = Variable
        | Property
        | EnumMember
        | ObjectLiteral
        | Function
        | Class
        | Enum
        | ValueModule
        | Method
        | GetAccessor
        | SetAccessor;
    pub const Type: u32 = Class | Interface | Enum | EnumMember | TypeLiteral | TypeParameter | TypeAlias;
    pub const Accessor: u32 = GetAccessor | SetAccessor;
    pub const Module: u32 = ValueModule | NamespaceModule;
    pub const Namespace: u32 = ValueModule | NamespaceModule | Enum;
    pub const ModuleMember: u32 = Variable | Function | Class | Interface | Enum | Module | TypeAlias | Alias;
}

use symbol_flags as sf;

const DEFAULT: &str = "default";
const EXPORT_EQUALS: &str = "export=";
/// The binder's `__export` symbol, which collects a module's `export *`.
const EXPORT_STAR: &str = "\u{FE}export";

/// The declarations one file contributes to the global scope, as its binder
/// declared them, and what resolving the file's aliases reads.
#[derive(Default, Debug)]
pub struct FileGlobals {
    pub(crate) symbols: Vec<GlobalSymbol>,
    /// A script's locals, which all join the global scope.
    pub(crate) is_script: bool,
    pub(crate) locals: Vec<u32>,
    /// A module's `declare global` blocks: the exports of each.
    pub(crate) augmentations: Vec<Vec<u32>>,
    /// A module's `declare module "…"` blocks: the name each augments, and
    /// the symbol the file's blocks of that name declare.
    pub(crate) module_augmentations: Vec<(String, u32)>,
    /// A module's own symbol (`file.Symbol`), whose exports its importers
    /// resolve against.
    pub(crate) module_symbol: Option<u32>,
    /// A declaration module's `export as namespace` (`file.GlobalExports`).
    pub(crate) global_exports: Vec<u32>,
    /// The containers the names written in the file's alias declarations
    /// resolve through, when any alias names its target by an entity name.
    pub(crate) scopes: Vec<GlobalScope>,
    /// The aliases the checker resolves when it checks the file
    /// (`checkAliasSymbol`), kept to those a cycle of aliases runs through.
    pub(crate) checked_aliases: Vec<u32>,
}

#[derive(Default, Debug)]
pub(crate) struct GlobalSymbol {
    pub name: String,
    pub flags: u32,
    pub declarations: Vec<GlobalDeclaration>,
    pub members: Option<Vec<u32>>,
    pub exports: Option<Vec<u32>>,
    pub link: Option<Box<SymbolLink>>,
}

#[derive(Debug)]
pub(crate) struct GlobalDeclaration {
    /// Where an error about the declaration goes: its name, or the node.
    pub name_range: (usize, usize),
    /// `createDiagnosticForNode(declaration)`.
    pub node_range: (usize, usize),
    pub is_type_declaration: bool,
    /// `rangeOfTypeParameters` of the declaration's type parameter list.
    pub type_parameters_range: Option<(usize, usize)>,
}

/// What a symbol stands for beyond its own declarations.
#[derive(Debug)]
pub(crate) enum SymbolLink {
    /// An alias, as its last alias declaration writes it
    /// (`getDeclarationOfAliasSymbol`).
    Alias(AliasDeclaration),
    /// The `__export` symbol: the module specifier of each `export *`.
    ExportStar(Vec<String>),
}

#[derive(Debug)]
pub(crate) struct AliasDeclaration {
    /// The declaration's index in its symbol's `declarations`.
    pub declaration: usize,
    /// `symbolToString` of the alias.
    pub display_name: String,
    /// Declared by an export specifier or `export * as`.
    pub is_export_specifier: bool,
    pub target: AliasTarget,
}

/// What an alias declaration names, as `getTargetOfAliasDeclaration` reads it.
#[derive(Debug)]
pub(crate) enum AliasTarget {
    /// `import x = require("m")`.
    ExternalModule(String),
    /// `import * as x from "m"`, `export * as x from "m"`.
    Namespace(String),
    /// `import { a as x } from "m"`, `export { a as x } from "m"`.
    ModuleMember { specifier: String, name: String },
    /// `import x = N.M`, written in the file's scope `scope`.
    ImportEntity { scope: u32, path: Vec<String> },
    /// `export { a as x }`, `export = a.b`, `export default a`.
    Entity { scope: u32, path: Vec<String> },
    /// `export as namespace x`: the module itself.
    FileModule,
    /// A default import, or an alias of what only types resolve.
    Unresolved,
}

/// A source file or module declaration, as `resolveName` walks it.
#[derive(Debug)]
pub(crate) struct GlobalScope {
    pub parent: Option<u32>,
    pub locals: Vec<u32>,
    /// The module's or namespace's own symbol, whose exports names resolve
    /// through too.
    pub symbol: Option<u32>,
    /// A module source file, or an ambient module or namespace declaration
    /// other than `declare global`: an export specifier's name is not in
    /// scope there.
    pub is_module_root: bool,
    /// A script, whose locals are the globals.
    pub is_global_source_file: bool,
}

impl GlobalSymbol {
    fn is_alias(&self) -> bool {
        matches!(self.link.as_deref(), Some(SymbolLink::Alias(_)))
    }
}

impl FileGlobals {
    /// The aliases a cycle of aliases must run through: what a module or a
    /// namespace exports, `export as namespace`, and `import x = N.M`. An
    /// import only reaches another module through one of the first.
    pub(crate) fn collect_checked_aliases(&mut self) {
        let mut seen = HashSet::new();
        let mut checked = Vec::new();
        for symbol in &self.symbols {
            if symbol.flags & sf::Module == 0 {
                continue;
            }
            for &export in symbol.exports.iter().flatten() {
                if self.symbols[export as usize].is_alias() && seen.insert(export) {
                    checked.push(export);
                }
            }
        }
        for (id, symbol) in self.symbols.iter().enumerate() {
            let id = id as u32;
            if let Some(SymbolLink::Alias(alias)) = symbol.link.as_deref()
                && matches!(alias.target, AliasTarget::ImportEntity { .. })
                && seen.insert(id)
            {
                checked.push(id);
            }
        }
        for &id in &self.global_exports {
            if seen.insert(id) {
                checked.push(id);
            }
        }
        self.checked_aliases = checked;
    }
}

/// A file taking part in the merge, in program order.
pub struct GlobalsInput<'a> {
    pub globals: &'a FileGlobals,
    /// tsc's `IsPlainJSFile`: a merge conflict reports nothing here.
    pub plain_js: bool,
}

/// The errors the global merge reports, per file of `files`.
pub fn merge_globals(files: &[GlobalsInput<'_>]) -> Vec<Vec<SyntaxDiagnostic>> {
    merge_globals_report(files).diagnostics
}

/// What the global merge decides for each file of `files`.
pub struct MergeReport {
    /// The errors it reports.
    pub diagnostics: Vec<Vec<SyntaxDiagnostic>>,
    /// The type parameter lists of declarations whose symbol another file
    /// declares too: tsc checks a list for unused type parameters only when
    /// every declaration of its symbol is in one file
    /// (`allDeclarationsInSameSourceFile`).
    pub shared_type_parameters: Vec<Vec<(usize, usize)>>,
    /// The names of a script's locals that conflict with a global an earlier
    /// file declares: they stay out of the global scope, which resolves the
    /// name to the earlier declaration, the script's own reads included.
    pub shadowed: Vec<Vec<String>>,
}

/// [`merge_globals_report_with`] resolving no module specifier to a file:
/// only ambient modules resolve.
pub fn merge_globals_report(files: &[GlobalsInput<'_>]) -> MergeReport {
    merge_globals_report_with(files, &|_: u32, _: &str| -> Option<u32> { None }, &|_: u32| true)
}

/// The merge, with the program's module resolution: `resolve_module(file,
/// specifier)` is the index in `files` of the file `specifier` resolves to
/// from `files[file]`. `is_checked(file)` says whether the checker checks
/// `files[file]`, resolving its aliases.
pub fn merge_globals_report_with(
    files: &[GlobalsInput<'_>],
    resolve_module: &dyn Fn(u32, &str) -> Option<u32>,
    is_checked: &dyn Fn(u32) -> bool,
) -> MergeReport {
    let mut merger = Merger {
        files,
        resolve_module,
        globals: Table::default(),
        merged: Vec::new(),
        merged_symbols: HashMap::new(),
        reported: Vec::new(),
        seen: HashSet::new(),
        alias_targets: HashMap::new(),
        resolving: Vec::new(),
        indexes: HashMap::new(),
        shadowed: Vec::new(),
    };
    let mut ambient_modules = Vec::new();
    for (file, input) in files.iter().enumerate() {
        if input.globals.is_script {
            for &local in &input.globals.locals {
                let global = &input.globals.symbols[local as usize];
                if global.name == "globalThis" {
                    for declaration in &global.declarations {
                        merger.report(
                            file as u32,
                            declaration.node_range,
                            diagnostics::Declaration_name_conflicts_with_built_in_global_identifier_0,
                            vec!["globalThis".into()],
                        );
                    }
                }
                let symbol = Sym::File(file as u32, local);
                // Global ambient module declarations merge after the rest.
                if global.flags & sf::Module != 0 && global.name.starts_with('"') {
                    ambient_modules.push(symbol);
                    continue;
                }
                merger.merge_global_symbol(symbol);
            }
        }
    }
    // The first `export as namespace` of a name wins over any later.
    // port: tsc adds a file's after the scripts before it, in an order where
    // a file follows the files it references and imports; this program's
    // order is not that one, so every script's globals come first, as the
    // script declaring a global a module's `export =` names does in tsc's.
    for (file, input) in files.iter().enumerate() {
        for &id in &input.globals.global_exports {
            let name = &input.globals.symbols[id as usize].name;
            if merger.globals.get(name).is_none() {
                merger.globals.set(name.clone(), Sym::File(file as u32, id));
            }
        }
    }
    for (file, input) in files.iter().enumerate() {
        for exports in &input.globals.augmentations {
            let source: Vec<(String, Sym)> = exports
                .iter()
                .map(|&id| (input.globals.symbols[id as usize].name.clone(), Sym::File(file as u32, id)))
                .collect();
            merger.merge_into_table(TableRef::Globals, source, false);
        }
    }
    if let Some(undefined) = merger.globals.get("undefined") {
        let declarations = merger.declarations(undefined);
        for (file, declaration) in declarations {
            if !declaration.is_type_declaration {
                merger.report(
                    file,
                    declaration.node_range,
                    diagnostics::Declaration_name_conflicts_with_built_in_global_identifier_0,
                    vec!["undefined".into()],
                );
            }
        }
    }
    for symbol in ambient_modules {
        merger.merge_global_symbol(symbol);
    }
    for (file, input) in files.iter().enumerate() {
        for (module_name, id) in &input.globals.module_augmentations {
            // port: an augmentation of a pattern ambient module is not merged.
            let Some(main_module) = merger.resolve_external_module_name(file as u32, module_name) else { continue };
            let Some(main_module) = merger.resolve_external_module_symbol(main_module, false) else { continue };
            if merger.flags(main_module) & sf::Namespace != 0 {
                let augmentation = Sym::File(file as u32, *id);
                // An augmenting export the module only re-exports through
                // `export *` merges into the symbol that re-export resolves to.
                if merger.export_of(main_module, EXPORT_STAR).is_some()
                    && let Some(exports) = merger.exports(augmentation)
                {
                    for (name, value) in exports.entries {
                        if merger.export_of(main_module, &name).is_none()
                            && let Some(resolved) = merger.module_export(main_module, &name)
                        {
                            merger.merge_symbol(resolved, value, false);
                        }
                    }
                }
                merger.merge_symbol(main_module, augmentation, false);
            }
        }
    }
    for (file, input) in files.iter().enumerate() {
        if !is_checked(file as u32) {
            continue;
        }
        for &id in &input.globals.checked_aliases {
            merger.resolve_alias(Sym::File(file as u32, id));
        }
    }
    let mut shared_type_parameters = vec![Vec::new(); files.len()];
    for symbol in &merger.merged {
        let first_file = symbol.declarations.first().map(|&(file, _, _)| file);
        if symbol.declarations.iter().all(|&(file, _, _)| Some(file) == first_file) {
            continue;
        }
        for &(file, id, index) in &symbol.declarations {
            if let Some(range) = merger.file_symbol(file, id).declarations[index].type_parameters_range {
                shared_type_parameters[file as usize].push(range);
            }
        }
    }
    let mut diagnostics = vec![Vec::new(); files.len()];
    for (file, diagnostic) in merger.reported {
        diagnostics[file as usize].push(diagnostic.render());
    }
    let mut shadowed = vec![Vec::new(); files.len()];
    for (file, name) in merger.shadowed {
        if !name.starts_with('"') {
            shadowed[file as usize].push(name);
        }
    }
    MergeReport { diagnostics, shared_type_parameters, shadowed }
}

/// A symbol of one file, or one the merge made (tsc's transient clone).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Sym {
    File(u32, u32),
    Merged(u32),
}

/// A symbol table that keeps its insertion order.
#[derive(Default, Clone)]
struct Table {
    entries: Vec<(String, Sym)>,
    index: HashMap<String, usize>,
}

impl Table {
    fn get(&self, name: &str) -> Option<Sym> {
        self.index.get(name).map(|&i| self.entries[i].1)
    }

    fn set(&mut self, name: String, symbol: Sym) {
        match self.index.get(&name) {
            Some(&i) => self.entries[i].1 = symbol,
            None => {
                self.index.insert(name.clone(), self.entries.len());
                self.entries.push((name, symbol));
            }
        }
    }
}

struct MergedSymbol {
    name: String,
    flags: u32,
    declarations: Vec<(u32, u32, usize)>,
    members: Option<Table>,
    exports: Option<Table>,
}

#[derive(Clone, Copy)]
enum TableRef {
    Globals,
    Members(u32),
    Exports(u32),
}

/// A file table [`Merger`] looks names up in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum IndexedTable {
    Exports(u32),
    Locals(u32),
}

struct Merger<'a> {
    files: &'a [GlobalsInput<'a>],
    resolve_module: &'a dyn Fn(u32, &str) -> Option<u32>,
    globals: Table,
    merged: Vec<MergedSymbol>,
    merged_symbols: HashMap<Sym, Sym>,
    reported: Vec<(u32, Diagnostic)>,
    seen: HashSet<(u32, usize, usize, u32, Vec<String>)>,
    /// `aliasSymbolLinks.aliasTarget`, `None` for `unknownSymbol`.
    alias_targets: HashMap<Sym, Option<Sym>>,
    /// The aliases being resolved (`pushTypeResolution`), each with whether
    /// its resolution was found circular.
    resolving: Vec<(Sym, bool)>,
    /// Name indexes of the files' tables, built on a table's first lookup.
    indexes: HashMap<(u32, IndexedTable), HashMap<&'a str, u32>>,
    /// The script locals a conflict left out of the globals, by file.
    shadowed: Vec<(u32, String)>,
}

impl<'a> Merger<'a> {
    fn file_globals(&self, file: u32) -> &'a FileGlobals {
        self.files[file as usize].globals
    }

    fn file_symbol(&self, file: u32, id: u32) -> &'a GlobalSymbol {
        &self.files[file as usize].globals.symbols[id as usize]
    }

    fn flags(&self, symbol: Sym) -> u32 {
        match symbol {
            Sym::File(file, id) => self.file_symbol(file, id).flags,
            Sym::Merged(id) => self.merged[id as usize].flags,
        }
    }

    fn name(&self, symbol: Sym) -> String {
        match symbol {
            Sym::File(file, id) => self.file_symbol(file, id).name.clone(),
            Sym::Merged(id) => self.merged[id as usize].name.clone(),
        }
    }

    fn declaration_refs(&self, symbol: Sym) -> Vec<(u32, u32, usize)> {
        match symbol {
            Sym::File(file, id) => (0..self.file_symbol(file, id).declarations.len()).map(|i| (file, id, i)).collect(),
            Sym::Merged(id) => self.merged[id as usize].declarations.clone(),
        }
    }

    fn declarations(&self, symbol: Sym) -> Vec<(u32, &'a GlobalDeclaration)> {
        self.declaration_refs(symbol)
            .into_iter()
            .map(|(file, id, i)| (file, &self.file_symbol(file, id).declarations[i]))
            .collect()
    }

    fn file_table(&self, file: u32, ids: &Option<Vec<u32>>) -> Option<Table> {
        let ids = ids.as_ref()?;
        let mut table = Table::default();
        for &id in ids {
            table.set(self.file_symbol(file, id).name.clone(), Sym::File(file, id));
        }
        Some(table)
    }

    fn members(&self, symbol: Sym) -> Option<Table> {
        match symbol {
            Sym::File(file, id) => self.file_table(file, &self.file_symbol(file, id).members),
            Sym::Merged(id) => self.merged[id as usize].members.clone(),
        }
    }

    fn exports(&self, symbol: Sym) -> Option<Table> {
        match symbol {
            Sym::File(file, id) => self.file_table(file, &self.file_symbol(file, id).exports),
            Sym::Merged(id) => self.merged[id as usize].exports.clone(),
        }
    }

    fn merged_symbol(&self, symbol: Sym) -> Sym {
        self.merged_symbols.get(&symbol).copied().unwrap_or(symbol)
    }

    /// `symbol.Exports[name]`.
    fn export_of(&mut self, symbol: Sym, name: &str) -> Option<Sym> {
        match symbol {
            Sym::Merged(id) => self.merged[id as usize].exports.as_ref().and_then(|table| table.get(name)),
            Sym::File(file, id) => {
                let globals = self.file_globals(file);
                let exports = globals.symbols[id as usize].exports.as_ref()?;
                let index = self.indexes.entry((file, IndexedTable::Exports(id))).or_insert_with(|| {
                    exports.iter().map(|&export| (globals.symbols[export as usize].name.as_str(), export)).collect()
                });
                index.get(name).map(|&export| Sym::File(file, export))
            }
        }
    }

    /// `scope.Locals()[name]`.
    fn local_of(&mut self, file: u32, scope: u32, name: &str) -> Option<Sym> {
        let globals = self.file_globals(file);
        let locals = &globals.scopes.get(scope as usize)?.locals;
        let index = self.indexes.entry((file, IndexedTable::Locals(scope))).or_insert_with(|| {
            locals.iter().map(|&local| (globals.symbols[local as usize].name.as_str(), local)).collect()
        });
        index.get(name).map(|&local| Sym::File(file, local))
    }

    fn merge_global_symbol(&mut self, symbol: Sym) {
        let name = self.name(symbol);
        let merged = match self.globals.get(&name) {
            Some(global) => {
                let merged = self.merge_symbol(global, symbol, false);
                // `mergeSymbol` returns the global it could not merge into, and
                // the name resolves to that one wherever it is written.
                if merged == global
                    && !self.merged_symbols.contains_key(&symbol)
                    && let Sym::File(file, _) = symbol
                {
                    self.shadowed.push((file, name.clone()));
                }
                merged
            }
            None => self.merged_symbol(symbol),
        };
        self.globals.set(name, merged);
    }

    fn table_get(&self, table: TableRef, name: &str) -> Option<Sym> {
        match table {
            TableRef::Globals => self.globals.get(name),
            TableRef::Members(id) => self.merged[id as usize].members.as_ref().and_then(|t| t.get(name)),
            TableRef::Exports(id) => self.merged[id as usize].exports.as_ref().and_then(|t| t.get(name)),
        }
    }

    fn table_set(&mut self, table: TableRef, name: String, symbol: Sym) {
        match table {
            TableRef::Globals => self.globals.set(name, symbol),
            TableRef::Members(id) => self.merged[id as usize].members.get_or_insert_with(Table::default).set(name, symbol),
            TableRef::Exports(id) => self.merged[id as usize].exports.get_or_insert_with(Table::default).set(name, symbol),
        }
    }

    /// `mergeSymbolTable` into a table this merge owns (the globals, or a
    /// merged symbol's members or exports).
    fn merge_into_table(&mut self, target: TableRef, source: Vec<(String, Sym)>, unidirectional: bool) {
        for (name, source_symbol) in source {
            let merged = match self.table_get(target, &name) {
                Some(target_symbol) => self.merge_symbol(target_symbol, source_symbol, unidirectional),
                None => self.merged_symbol(source_symbol),
            };
            self.table_set(target, name, merged);
        }
    }

    fn clone_symbol(&mut self, symbol: Sym) -> u32 {
        let clone = MergedSymbol {
            name: self.name(symbol),
            flags: self.flags(symbol),
            declarations: self.declaration_refs(symbol),
            members: self.members(symbol),
            exports: self.exports(symbol),
        };
        let id = self.merged.len() as u32;
        self.merged.push(clone);
        self.merged_symbols.insert(symbol, Sym::Merged(id));
        id
    }

    fn merge_symbol(&mut self, target: Sym, source: Sym, unidirectional: bool) -> Sym {
        let target_flags = self.flags(target);
        let source_flags = self.flags(source);
        if target_flags & excluded_symbol_flags(source_flags) == 0 || (source_flags | target_flags) & sf::Assignment != 0 {
            if source == target {
                return target;
            }
            let target_id = match target {
                Sym::Merged(id) => id,
                Sym::File(..) => {
                    // What an alias resolves to decides the merge.
                    let Some(resolved) = self.resolve_symbol(target) else {
                        return source;
                    };
                    let resolved_flags = self.flags(resolved);
                    if resolved_flags & excluded_symbol_flags(source_flags) != 0
                        && (source_flags | resolved_flags) & sf::Assignment == 0
                    {
                        self.report_merge_symbol_error(target, source);
                        return source;
                    }
                    self.clone_symbol(resolved)
                }
            };
            self.merged[target_id as usize].flags |= source_flags;
            let declarations = self.declaration_refs(source);
            self.merged[target_id as usize].declarations.extend(declarations);
            if let Some(members) = self.members(source) {
                self.merged[target_id as usize].members.get_or_insert_with(Table::default);
                self.merge_into_table(TableRef::Members(target_id), members.entries, unidirectional);
            }
            if let Some(exports) = self.exports(source) {
                self.merged[target_id as usize].exports.get_or_insert_with(Table::default);
                self.merge_into_table(TableRef::Exports(target_id), exports.entries, unidirectional);
            }
            if !unidirectional {
                self.merged_symbols.insert(source, Sym::Merged(target_id));
            }
            return Sym::Merged(target_id);
        }
        if target_flags & sf::NamespaceModule != 0 {
            let name = self.name(target);
            if name != "globalThis"
                && let Some(&(file, declaration)) = self.declarations(source).first()
                && !self.files[file as usize].plain_js
            {
                self.report(
                    file,
                    declaration.name_range,
                    diagnostics::Cannot_augment_module_0_with_value_exports_because_it_resolves_to_a_non_module_entity,
                    vec![symbol_to_string(&name)],
                );
            }
            return target;
        }
        self.report_merge_symbol_error(target, source);
        target
    }

    fn report_merge_symbol_error(&mut self, target: Sym, source: Sym) {
        let target_flags = self.flags(target);
        let source_flags = self.flags(source);
        let message = if target_flags & sf::Enum != 0 || source_flags & sf::Enum != 0 {
            diagnostics::Enum_declarations_can_only_merge_with_namespace_or_other_enum_declarations
        } else if target_flags & sf::BlockScopedVariable != 0 || source_flags & sf::BlockScopedVariable != 0 {
            diagnostics::Cannot_redeclare_block_scoped_variable_0
        } else {
            diagnostics::Duplicate_identifier_0
        };
        let symbol_name = symbol_to_string(&self.name(source));
        let first_file = |merger: &Self, symbol: Sym| merger.declaration_refs(symbol).first().map(|&(file, _, _)| file);
        let source_plain_js = first_file(self, source).is_some_and(|file| self.files[file as usize].plain_js);
        let target_plain_js = first_file(self, target).is_some_and(|file| self.files[file as usize].plain_js);
        let args = if message.text.contains("{0}") { vec![symbol_name] } else { Vec::new() };
        if !source_plain_js {
            for (file, declaration) in self.declarations(source) {
                self.report(file, declaration.name_range, message, args.clone());
            }
        }
        if !target_plain_js {
            for (file, declaration) in self.declarations(target) {
                self.report(file, declaration.name_range, message, args.clone());
            }
        }
    }

    /// `lookupOrIssueError`: one report per place and message.
    fn report(&mut self, file: u32, (start, end): (usize, usize), message: &'static Message, args: Vec<String>) {
        if self.seen.insert((file, start, end, message.code, args.clone())) {
            self.reported.push((file, Diagnostic { start, end, message, args }));
        }
    }

    // --- alias resolution ----------------------------------------------------

    /// `ast.IsNonLocalAlias(symbol, Value|Type|Namespace)`: an alias and
    /// nothing else.
    fn is_non_local_alias(&self, symbol: Sym) -> bool {
        self.flags(symbol) & (sf::Alias | sf::Value | sf::Type | sf::Namespace) == sf::Alias
    }

    /// `resolveSymbol`.
    fn resolve_symbol(&mut self, symbol: Sym) -> Option<Sym> {
        if self.is_non_local_alias(symbol) { self.resolve_alias(symbol) } else { Some(symbol) }
    }

    /// `getDeclarationOfAliasSymbol`: the last alias declaration, with its
    /// file and the range an error about it takes.
    fn alias_declaration(&self, symbol: Sym) -> Option<(u32, (usize, usize), &'a AliasDeclaration)> {
        self.declaration_refs(symbol).into_iter().rev().find_map(|(file, id, index)| {
            let global = self.file_symbol(file, id);
            match global.link.as_deref() {
                Some(SymbolLink::Alias(alias)) if alias.declaration == index => {
                    Some((file, global.declarations[index].node_range, alias))
                }
                _ => None,
            }
        })
    }

    /// `resolveAlias`: `None` is `unknownSymbol`. A resolution that reaches an
    /// alias whose own resolution is in progress fails every resolution from
    /// that alias's on (`pushTypeResolution`), and each of those aliases is
    /// TS2303 at its declaration.
    fn resolve_alias(&mut self, symbol: Sym) -> Option<Sym> {
        if let Some(&target) = self.alias_targets.get(&symbol) {
            return target;
        }
        if let Some(start) = self.resolving.iter().position(|&(entry, _)| entry == symbol) {
            for entry in &mut self.resolving[start..] {
                entry.1 = true;
            }
            return None;
        }
        self.resolving.push((symbol, false));
        let declaration = self.alias_declaration(symbol);
        let mut target = declaration.and_then(|(file, _, alias)| self.target_of_alias_declaration(file, alias));
        if let Some(immediate) = target
            && self.is_non_local_alias(immediate)
        {
            target = self.resolve_alias(immediate).map(|resolved| self.merged_symbol(resolved));
        }
        let circular = self.resolving.pop().is_some_and(|(_, circular)| circular);
        if circular {
            if let Some((file, node_range, alias)) = declaration {
                self.report(
                    file,
                    node_range,
                    diagnostics::Circular_definition_of_import_alias_0,
                    vec![alias.display_name.clone()],
                );
            }
            target = None;
        }
        self.alias_targets.insert(symbol, target);
        target
    }

    /// `getTargetOfAliasDeclaration`, which leaves an alias it reaches
    /// unresolved.
    fn target_of_alias_declaration(&mut self, file: u32, alias: &'a AliasDeclaration) -> Option<Sym> {
        let any_meaning = sf::Value | sf::Type | sf::Namespace;
        match &alias.target {
            AliasTarget::ExternalModule(specifier) => {
                let module = self.resolve_external_module_name(file, specifier)?;
                self.resolve_external_module_symbol(module, true)
            }
            AliasTarget::Namespace(specifier) => {
                let module = self.resolve_external_module_name(file, specifier)?;
                self.resolve_es_module_symbol(module)
            }
            AliasTarget::ModuleMember { specifier, name } => self.external_module_member(file, specifier, name),
            AliasTarget::ImportEntity { scope, path } => {
                let meaning = if path.len() == 1 { sf::Namespace } else { any_meaning };
                self.resolve_entity_name(file, *scope, path, meaning, true)
            }
            AliasTarget::Entity { scope, path } => self.resolve_entity_name(file, *scope, path, any_meaning, true),
            AliasTarget::FileModule => {
                let module = self.file_globals(file).module_symbol?;
                self.resolve_external_module_symbol(Sym::File(file, module), true)
            }
            AliasTarget::Unresolved => None,
        }
    }

    /// `resolveExternalModuleName`: an ambient module of the name
    /// (`tryFindAmbientModule`), then the module file the program resolves
    /// it to.
    fn resolve_external_module_name(&self, file: u32, specifier: &str) -> Option<Sym> {
        if !is_external_module_name_relative(specifier)
            && let Some(symbol) = self.globals.get(&format!("\"{specifier}\""))
            && self.flags(symbol) & sf::ValueModule != 0
        {
            return Some(self.merged_symbol(symbol));
        }
        let target = (self.resolve_module)(file, specifier)?;
        let module = self.files.get(target as usize)?.globals.module_symbol?;
        Some(self.merged_symbol(Sym::File(target, module)))
    }

    /// `resolveExternalModuleSymbol`: what a module's `export =` names, or
    /// the module.
    fn resolve_external_module_symbol(&mut self, module: Sym, dont_resolve_alias: bool) -> Option<Sym> {
        let Some(export_equals) = self.export_of(module, EXPORT_EQUALS) else {
            return Some(module);
        };
        let resolved = if dont_resolve_alias { Some(export_equals) } else { self.resolve_symbol(export_equals) };
        resolved.map(|symbol| self.merged_symbol(symbol))
    }

    /// `resolveESModuleSymbol`. The synthetic-default wrapper it may build
    /// keeps the flags this merge reads.
    fn resolve_es_module_symbol(&mut self, module: Sym) -> Option<Sym> {
        let symbol = self.resolve_external_module_symbol(module, true)?;
        if self.is_non_local_alias(symbol) {
            return self.resolve_alias(symbol).map(|resolved| self.merged_symbol(resolved));
        }
        Some(symbol)
    }

    /// `getExternalModuleMember`, which leaves the export unresolved.
    fn external_module_member(&mut self, file: u32, specifier: &str, name: &str) -> Option<Sym> {
        let module = self.resolve_external_module_name(file, specifier)?;
        // port: a member of a module with `export =` is a property of its type.
        if self.export_of(module, EXPORT_EQUALS).is_some() {
            return None;
        }
        let target = self.resolve_es_module_symbol(module)?;
        if self.flags(target) & sf::Module == 0 {
            return None;
        }
        self.module_export(target, name)
    }

    /// `getExportsOfModule(module)[name]`: a module with `export =` exports
    /// what its target does, and an `export *` adds what the re-exported
    /// module exports and the module does not.
    fn module_export(&mut self, module: Sym, name: &str) -> Option<Sym> {
        let module = self.resolve_external_module_symbol(module, false)?;
        let mut visited = Vec::new();
        self.visit_module_exports(module, name, &mut visited)
    }

    fn visit_module_exports(&mut self, module: Sym, name: &str, visited: &mut Vec<Sym>) -> Option<Sym> {
        if visited.contains(&module) {
            return None;
        }
        visited.push(module);
        if let Some(symbol) = self.export_of(module, name) {
            return Some(symbol);
        }
        if name == DEFAULT {
            return None;
        }
        let stars = self.export_of(module, EXPORT_STAR)?;
        for (file, specifier) in self.export_star_specifiers(stars) {
            if let Some(target) = self.resolve_external_module_name(file, specifier)
                && let Some(symbol) = self.visit_module_exports(target, name, visited)
            {
                return Some(symbol);
            }
        }
        None
    }

    /// The module specifiers of an `__export` symbol's declarations, with
    /// the file each is written in.
    fn export_star_specifiers(&self, stars: Sym) -> Vec<(u32, &'a str)> {
        let mut seen = HashSet::new();
        let mut specifiers = Vec::new();
        for (file, id, _) in self.declaration_refs(stars) {
            if !seen.insert((file, id)) {
                continue;
            }
            if let Some(SymbolLink::ExportStar(names)) = self.file_symbol(file, id).link.as_deref() {
                specifiers.extend(names.iter().map(|name| (file, name.as_str())));
            }
        }
        specifiers
    }

    /// `resolveEntityName` of an identifier or qualified name written in the
    /// file's scope `scope`.
    fn resolve_entity_name(
        &mut self,
        file: u32,
        scope: u32,
        path: &[String],
        meaning: u32,
        dont_resolve_alias: bool,
    ) -> Option<Sym> {
        let (last, left) = path.split_last()?;
        let mut symbol = if left.is_empty() {
            let found = self.resolve_name(file, scope, last, meaning)?;
            self.merged_symbol(found)
        } else {
            let namespace = self.resolve_entity_name(file, scope, left, sf::Namespace, false)?;
            match self.exported_symbol(namespace, last, meaning) {
                Some(found) => self.merged_symbol(found),
                None if self.flags(namespace) & sf::Alias != 0 => {
                    let resolved = self.resolve_alias(namespace)?;
                    let found = self.exported_symbol(resolved, last, meaning)?;
                    self.merged_symbol(found)
                }
                None => return None,
            }
        };
        while self.flags(symbol) & meaning == 0 && !dont_resolve_alias && self.flags(symbol) & sf::Alias != 0 {
            symbol = self.resolve_alias(symbol)?;
        }
        Some(symbol)
    }

    /// `getSymbol(getExportsOfSymbol(namespace), name, meaning)`.
    fn exported_symbol(&mut self, namespace: Sym, name: &str, meaning: u32) -> Option<Sym> {
        let export = if self.flags(namespace) & sf::Module != 0 {
            self.module_export(namespace, name)
        } else {
            self.export_of(namespace, name)
        };
        self.get_symbol(export, meaning)
    }

    /// `resolveName` from a declaration in the file's scope `scope`: each
    /// enclosing container's locals, then the exports of a module or
    /// namespace, then the globals.
    fn resolve_name(&mut self, file: u32, scope: u32, name: &str, meaning: u32) -> Option<Sym> {
        let globals = self.file_globals(file);
        let scopes = &globals.scopes;
        let mut current = Some(scope);
        while let Some(index) = current {
            let Some(container) = scopes.get(index as usize) else { break };
            if !container.is_global_source_file {
                let local = self.local_of(file, index, name);
                if let Some(symbol) = self.get_symbol(local, meaning) {
                    return Some(symbol);
                }
            }
            if let Some(owner) = container.symbol {
                let owner = self.merged_symbol(Sym::File(file, owner));
                let export = self.export_of(owner, name);
                // A name a module exports through an export specifier is not
                // one its own declarations see.
                let exported_by_specifier = container.is_module_root
                    && export.is_some_and(|export| self.flags(export) == sf::Alias && self.is_export_specifier(export));
                if !exported_by_specifier
                    && name != DEFAULT
                    && let Some(symbol) = self.get_symbol(export, meaning & sf::ModuleMember)
                {
                    return Some(symbol);
                }
            }
            current = container.parent;
        }
        let global = self.globals.get(name);
        self.get_symbol(global, meaning)
    }

    fn is_export_specifier(&self, symbol: Sym) -> bool {
        self.alias_declaration(symbol).is_some_and(|(_, _, alias)| alias.is_export_specifier)
    }

    /// `getSymbol`: the symbol when it has the meaning, or is an alias whose
    /// resolution has it.
    fn get_symbol(&mut self, symbol: Option<Sym>, meaning: u32) -> Option<Sym> {
        if meaning & sf::All == 0 {
            return None;
        }
        let symbol = self.merged_symbol(symbol?);
        if self.flags(symbol) & meaning != 0
            || (self.flags(symbol) & sf::Alias != 0 && self.symbol_flags(symbol) & meaning != 0)
        {
            return Some(symbol);
        }
        None
    }

    /// `getSymbolFlags`: an alias's flags with those of everything it
    /// resolves through; all of them when its resolution fails.
    fn symbol_flags(&mut self, symbol: Sym) -> u32 {
        let mut flags = self.flags(symbol);
        let mut symbol = symbol;
        let mut seen: Vec<Sym> = Vec::new();
        while self.flags(symbol) & sf::Alias != 0 {
            let Some(target) = self.resolve_alias(symbol) else {
                return sf::All;
            };
            if self.flags(target) & sf::Alias != 0 {
                if target == symbol || seen.contains(&target) {
                    break;
                }
                if seen.is_empty() {
                    seen.push(symbol);
                }
                seen.push(target);
            }
            flags |= self.flags(target);
            symbol = target;
        }
        flags
    }
}

/// `tspath.IsExternalModuleNameRelative`: a relative path, or a rooted one.
pub(crate) fn is_external_module_name_relative(name: &str) -> bool {
    let bytes = name.as_bytes();
    name == "."
        || name == ".."
        || name.starts_with("./")
        || name.starts_with(".\\")
        || name.starts_with("../")
        || name.starts_with("..\\")
        || matches!(bytes.first(), Some(b'/' | b'\\'))
        || bytes.len() >= 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes.len() == 2 || matches!(bytes[2], b'/' | b'\\'))
        || name.starts_with("^/")
        || name.contains("://")
}

/// `symbolToString` for a global's name.
fn symbol_to_string(name: &str) -> String {
    match name.strip_prefix('\u{FE}') {
        Some(rest) => format!("__{rest}"),
        None => name.to_string(),
    }
}

/// `getExcludedSymbolFlags`.
fn excluded_symbol_flags(flags: u32) -> u32 {
    let mut result = 0;
    let pairs: [(u32, u32); 16] = [
        (sf::BlockScopedVariable, sf::Value),
        (sf::FunctionScopedVariable, sf::Value & !sf::FunctionScopedVariable),
        (sf::Property, sf::Value & !(sf::Property | sf::Accessor)),
        (sf::EnumMember, sf::Value | sf::Type),
        (sf::Function, sf::Value & !(sf::Function | sf::ValueModule | sf::Class)),
        (sf::Class, (sf::Value | sf::Type) & !(sf::ValueModule | sf::Interface | sf::Function)),
        (sf::Interface, sf::Type & !(sf::Interface | sf::Class)),
        (sf::RegularEnum, (sf::Value | sf::Type) & !(sf::RegularEnum | sf::ValueModule)),
        (sf::ConstEnum, (sf::Value | sf::Type) & !sf::ConstEnum),
        (sf::ValueModule, sf::Value & !(sf::Function | sf::Class | sf::RegularEnum | sf::ValueModule)),
        (sf::Method, sf::Value & !sf::Method),
        (sf::GetAccessor, sf::Value & !(sf::SetAccessor | sf::Property)),
        (sf::SetAccessor, sf::Value & !(sf::GetAccessor | sf::Property)),
        (sf::TypeParameter, sf::Type & !sf::TypeParameter),
        (sf::TypeAlias, sf::Type),
        (sf::Alias, sf::Alias),
    ];
    for (flag, excludes) in pairs {
        if flags & flag != 0 {
            result |= excludes;
        }
    }
    if flags & sf::ReplaceableByMethod != 0 {
        result &= !sf::Method;
    }
    result
}
