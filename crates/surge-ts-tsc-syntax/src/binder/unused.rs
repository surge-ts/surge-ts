//! typescript-go's unused-identifier check over a bound file
//! (`checkUnusedIdentifiers`). The checker marks every symbol it resolves a
//! use of (`NameResolver.Resolve` with `isUse`, `markPropertyAsReferenced`),
//! and a declaration nothing marked is reported under `noUnusedLocals` and
//! `noUnusedParameters`. Uses are found by walking the tree and resolving
//! each name the checker would resolve; a private member is found by its name
//! where the checker goes through the receiver's type, which can only mark
//! more.

use std::collections::HashMap;

use super::{Binder, INTERNAL_PREFIX, SymbolId, Table, sf};
use crate::ast::{self, NodeId};
use crate::flags::NodeFlags;
use crate::kind::Kind;
use crate::messages as diagnostics;
use crate::{Diagnostic, Message, ScriptTarget, UnusedCheck};

const VALUE_USE: u32 = sf::Value | sf::ExportValue;
const MODULE_MEMBER: u32 = sf::Variable
    | sf::Function
    | sf::Class
    | sf::Interface
    | sf::Enum
    | sf::ValueModule
    | sf::NamespaceModule
    | sf::TypeAlias
    | sf::Alias;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
    ReadWrite,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnusedKind {
    Local,
    Parameter,
}

enum Resolved {
    Symbol(SymbolId),
    /// The function's `arguments`, a symbol of no declaration.
    Arguments,
}

#[derive(Clone, Copy)]
struct PrivateMember {
    symbol: SymbolId,
    class: NodeId,
}

impl<'a> Binder<'a> {
    pub(crate) fn unused_diagnostics(&self, check: &UnusedCheck, target: ScriptTarget) -> Vec<Diagnostic> {
        let mut unused = Unused {
            binder: self,
            check,
            target,
            nodes: tree_order(self),
            reference_kinds: vec![0; self.symbols.len()],
            private_members: HashMap::new(),
            diagnostics: Vec::new(),
        };
        unused.collect_private_members();
        unused.mark_references();
        unused.check_nodes();
        unused.diagnostics.sort_by_key(|diagnostic| (diagnostic.start, diagnostic.message.code));
        unused.diagnostics
    }
}

struct Unused<'b, 'a> {
    binder: &'b Binder<'a>,
    check: &'b UnusedCheck,
    target: ScriptTarget,
    /// Every node of the tree, parents before children.
    nodes: Vec<NodeId>,
    /// `symbolReferenceLinks.referenceKinds`, by symbol.
    reference_kinds: Vec<u32>,
    /// A class's `private` or `#`-named members and `private` parameter
    /// properties, by name.
    private_members: HashMap<String, Vec<PrivateMember>>,
    diagnostics: Vec<Diagnostic>,
}

impl Unused<'_, '_> {
    fn node(&self, id: NodeId) -> &crate::ast::Node {
        self.binder.file.node(id)
    }

    fn child(&self, id: NodeId, index: usize) -> Option<NodeId> {
        self.node(id).children.get(index).copied().flatten()
    }

    fn list(&self, id: NodeId, index: usize) -> Vec<NodeId> {
        self.node(id).lists.get(index).and_then(|list| list.as_ref()).map_or_else(Vec::new, |list| list.nodes.clone())
    }

    fn text(&self, id: NodeId) -> &str {
        &self.node(id).text
    }

    fn find_ancestor(&self, node: NodeId, predicate: impl Fn(Kind) -> bool) -> Option<NodeId> {
        let mut current = Some(node);
        while let Some(id) = current {
            if predicate(self.binder.kind(id)) {
                return Some(id);
            }
            current = self.binder.parent(id);
        }
        None
    }

    fn starts_with_underscore(&self, name: NodeId) -> bool {
        self.binder.kind(name) == Kind::Identifier && self.text(name).starts_with('_')
    }

    // --- resolution ----------------------------------------------------------

    /// `Checker.getSymbol`. An import's own flags say nothing of what it
    /// names: the checker reads its target's, which this file cannot see, so
    /// it answers any meaning, as an alias that fails to resolve does.
    fn lookup(&self, table: Table, name: &str, meaning: u32) -> Option<SymbolId> {
        if meaning == 0 {
            return None;
        }
        let symbol = *self.binder.tables.get(&table)?.get(name)?;
        let flags = self.binder.symbols[symbol].flags;
        (flags & meaning != 0 || flags & sf::Alias != 0).then_some(symbol)
    }

    /// `NameResolver.Resolve` for a use at `origin`: the symbol `name` means
    /// there, marked referenced unless the use is a self-reference, one
    /// inside the declaration it names.
    fn resolve_use(&mut self, origin: NodeId, name: &str, meaning: u32) -> Option<SymbolId> {
        let (symbol, counts) = self.resolve(origin, name, meaning)?;
        if counts {
            self.reference_kinds[symbol] |= meaning;
        }
        Some(symbol)
    }

    /// The value an identifier names, a script's own top-level declarations
    /// included: they are globals, which a use never marks, but they are what
    /// the name means.
    fn resolve_value(&self, identifier: NodeId) -> Option<SymbolId> {
        let b = self.binder;
        let name = self.text(identifier);
        self.resolve(identifier, name, VALUE_USE).map(|(symbol, _)| symbol).or_else(|| {
            if b.file.external_module {
                return None;
            }
            self.lookup(Table::Locals(b.file.root), name, VALUE_USE)
        })
    }

    /// The symbol `name` means at `origin`, and whether a use there counts
    /// as one.
    fn resolve(&self, origin: NodeId, name: &str, meaning: u32) -> Option<(SymbolId, bool)> {
        let b = self.binder;
        let mut location = Some(origin);
        let mut last_location: Option<NodeId> = None;
        let mut last_self_reference: Option<NodeId> = None;
        let mut result = None;
        'walk: while let Some(mut loc) = location {
            if matches!(b.kind(loc), Kind::ModuleDeclaration | Kind::EnumDeclaration)
                && last_location.is_some()
                && b.name_of(loc) == last_location
            {
                last_location = Some(loc);
                match b.parent(loc) {
                    Some(parent) => loc = parent,
                    None => break,
                }
            }
            let is_global_source_file = b.kind(loc) == Kind::SourceFile && !b.file.external_module;
            if !is_global_source_file && let Some(found) = self.lookup(Table::Locals(loc), name, meaning) {
                let flags = b.symbols[found].flags;
                let mut use_result = true;
                if ast::is_function_like_kind(b.kind(loc))
                    && let Some(last) = last_location
                    && Some(last) != self.node(loc).body
                {
                    // Type parameters are in scope over the whole signature,
                    // parameters only in the body and the parameter list,
                    // and local types only in the body.
                    if meaning & flags & sf::Type != 0 {
                        use_result = flags & sf::TypeParameter != 0
                            && (Some(last) == self.node(loc).ty
                                || matches!(b.kind(last), Kind::Parameter | Kind::TypeParameter));
                    }
                    if meaning & flags & sf::Variable != 0 {
                        if self.use_outer_variable_scope_in_parameter(found, loc, last) {
                            use_result = false;
                        } else if flags & sf::FunctionScopedVariable != 0 {
                            use_result = b.kind(last) == Kind::Parameter
                                || Some(last) == self.node(loc).ty
                                    && b.symbols[found].value_declaration.is_some_and(|declaration| {
                                        self.find_ancestor(declaration, |kind| kind == Kind::Parameter).is_some()
                                    });
                        }
                    }
                } else if b.kind(loc) == Kind::ConditionalType {
                    // `infer T` is in scope only in the true branch.
                    use_result = last_location.is_some() && last_location == self.child(loc, 2);
                }
                if use_result {
                    result = Some(Resolved::Symbol(found));
                    break 'walk;
                }
            }
            match b.kind(loc) {
                Kind::SourceFile | Kind::ModuleDeclaration
                    if b.kind(loc) == Kind::ModuleDeclaration || b.file.external_module =>
                {
                    if let Some(found) = self.lookup_module_member(loc, name, meaning) {
                        result = Some(Resolved::Symbol(found));
                        break 'walk;
                    }
                }
                Kind::EnumDeclaration => {
                    if let Some(symbol) = b.symbol_of(loc)
                        && let Some(found) = self.lookup(Table::Exports(symbol), name, meaning & sf::EnumMember)
                    {
                        result = Some(Resolved::Symbol(found));
                        break 'walk;
                    }
                }
                Kind::ClassDeclaration | Kind::ClassExpression | Kind::InterfaceDeclaration => {
                    if let Some(symbol) = b.symbol_of(loc) {
                        if let Some(found) = self.lookup(Table::Members(symbol), name, meaning & sf::Type) {
                            if self.is_type_parameter_declared_in(found, loc) {
                                // A static member does not see its class's
                                // type parameters.
                                if last_location.is_some_and(|last| b.is_static(last)) {
                                    return None;
                                }
                                result = Some(Resolved::Symbol(found));
                                break 'walk;
                            }
                        } else if b.kind(loc) == Kind::ClassExpression
                            && meaning & sf::Class != 0
                            && b.name_of(loc).is_some_and(|class_name| self.text(class_name) == name)
                        {
                            result = Some(Resolved::Symbol(symbol));
                            break 'walk;
                        }
                    }
                }
                Kind::ExpressionWithTypeArguments => {
                    // A base class expression does not see its class's type
                    // parameters.
                    if last_location.is_some()
                        && last_location == self.node(loc).expression
                        && let Some(clause) = b.parent(loc)
                        && b.kind(clause) == Kind::HeritageClause
                        && self.node(clause).op == Kind::ExtendsKeyword
                        && let Some(container) = b.parent(clause)
                        && b.is_class_like(container)
                        && let Some(symbol) = b.symbol_of(container)
                        && self.lookup(Table::Members(symbol), name, meaning & sf::Type).is_some()
                    {
                        return None;
                    }
                }
                Kind::ComputedPropertyName => {
                    if let Some(grandparent) = b.parent(loc).and_then(|parent| b.parent(parent))
                        && (b.is_class_like(grandparent) || b.kind(grandparent) == Kind::InterfaceDeclaration)
                        && let Some(symbol) = b.symbol_of(grandparent)
                        && self.lookup(Table::Members(symbol), name, meaning & sf::Type).is_some()
                    {
                        return None;
                    }
                }
                Kind::MethodDeclaration
                | Kind::Constructor
                | Kind::GetAccessor
                | Kind::SetAccessor
                | Kind::FunctionDeclaration => {
                    if meaning & sf::Variable != 0 && name == "arguments" {
                        result = Some(Resolved::Arguments);
                        break 'walk;
                    }
                }
                Kind::FunctionExpression => {
                    if meaning & sf::Variable != 0 && name == "arguments" {
                        result = Some(Resolved::Arguments);
                        break 'walk;
                    }
                    if meaning & sf::Function != 0
                        && b.name_of(loc).is_some_and(|function_name| self.text(function_name) == name)
                    {
                        result = b.symbol_of(loc).map(Resolved::Symbol);
                        break 'walk;
                    }
                }
                Kind::Decorator => {
                    // A decorator is resolved where its class, or the member
                    // or parameter it decorates, is declared.
                    if let Some(parent) = b.parent(loc)
                        && b.kind(parent) == Kind::Parameter
                    {
                        loc = parent;
                    }
                    if let Some(parent) = b.parent(loc)
                        && (b.is_class_element(parent) || b.kind(parent) == Kind::ClassDeclaration)
                    {
                        loc = parent;
                    }
                }
                Kind::InferType => {
                    if meaning & sf::TypeParameter != 0
                        && let Some(type_parameter) = self.child(loc, 0)
                        && b.name_of(type_parameter).is_some_and(|parameter_name| self.text(parameter_name) == name)
                    {
                        result = b.symbol_of(type_parameter).map(Resolved::Symbol);
                        break 'walk;
                    }
                }
                _ => {}
            }
            if self.is_self_reference_location(loc, last_location) {
                last_self_reference = Some(loc);
            }
            last_location = Some(loc);
            location = b.parent(loc);
        }
        let Some(Resolved::Symbol(symbol)) = result else { return None };
        let counts = last_self_reference.is_none_or(|declaration| b.symbol_of(declaration) != Some(symbol));
        Some((symbol, counts))
    }

    /// The exports of a module (or of an external module file) in scope in
    /// its body: its default export under the local name it declares, and
    /// any member it exports, but not a name an export specifier alone puts
    /// in the table.
    fn lookup_module_member(&self, module: NodeId, name: &str, meaning: u32) -> Option<SymbolId> {
        let b = self.binder;
        let symbol = b.symbol_of(module)?;
        let exports = Table::Exports(symbol);
        let is_external = b.kind(module) == Kind::SourceFile
            || b.flags(module).has(NodeFlags::Ambient) && !b.is_global_scope_augmentation(module);
        if is_external {
            let table = b.tables.get(&exports);
            if let Some(&default) = table.and_then(|table| table.get(super::DEFAULT)) {
                if b.symbols[default].flags & meaning != 0 && self.local_name_of_export_default(default) == Some(name) {
                    return Some(default);
                }
            }
            if let Some(&export) = table.and_then(|table| table.get(name))
                && b.symbols[export].flags == sf::Alias
                && b.symbols[export]
                    .declarations
                    .iter()
                    .any(|&declaration| matches!(b.kind(declaration), Kind::ExportSpecifier | Kind::NamespaceExport))
            {
                return None;
            }
        }
        if name == super::DEFAULT {
            return None;
        }
        self.lookup(exports, name, meaning & MODULE_MEMBER)
    }

    /// `GetLocalSymbolForExportDefault`, by the name its local is declared
    /// under.
    fn local_name_of_export_default(&self, default: SymbolId) -> Option<&str> {
        let b = self.binder;
        let declarations = &b.symbols[default].declarations;
        let first = *declarations.first()?;
        if !b.has_modifier(first, Kind::DefaultKeyword) {
            return None;
        }
        declarations
            .iter()
            .find(|declaration| b.local_symbol.contains_key(declaration))
            .and_then(|&declaration| b.name_of(declaration))
            .map(|name| self.text(name))
    }

    fn is_type_parameter_declared_in(&self, symbol: SymbolId, container: NodeId) -> bool {
        let b = self.binder;
        b.symbols[symbol]
            .declarations
            .iter()
            .any(|&declaration| b.kind(declaration) == Kind::TypeParameter && b.parent(declaration) == Some(container))
    }

    fn is_self_reference_location(&self, node: NodeId, last_location: Option<NodeId>) -> bool {
        let b = self.binder;
        match b.kind(node) {
            Kind::Parameter => last_location.is_some() && last_location == b.name_of(node),
            Kind::FunctionDeclaration
            | Kind::ClassDeclaration
            | Kind::InterfaceDeclaration
            | Kind::EnumDeclaration
            | Kind::TypeAliasDeclaration
            | Kind::ModuleDeclaration => true,
            _ => false,
        }
    }

    /// A name in a parameter's initializer or binding pattern resolves past a
    /// variable its function's body declares, unless the parameters must be
    /// moved into the body when emitted (`useOuterVariableScopeInParameter`).
    fn use_outer_variable_scope_in_parameter(&self, symbol: SymbolId, function: NodeId, last_location: NodeId) -> bool {
        let b = self.binder;
        if b.kind(last_location) != Kind::Parameter {
            return false;
        }
        let Some(body) = self.node(function).body else { return false };
        let Some(declaration) = b.symbols[symbol].value_declaration else { return false };
        let (body, declaration) = (self.node(body), self.node(declaration));
        if declaration.pos < body.pos || declaration.end > body.end {
            return false;
        }
        let parameters =
            self.node(function).parameters.as_ref().map_or_else(Vec::new, |parameters| parameters.nodes.clone());
        !parameters.iter().any(|&parameter| {
            let p = self.node(parameter);
            p.name.is_some_and(|name| self.requires_scope_change(name))
                || p.initializer.is_some_and(|initializer| self.requires_scope_change(initializer))
        })
    }

    fn requires_scope_change(&self, node: NodeId) -> bool {
        let b = self.binder;
        let n = self.node(node);
        match b.kind(node) {
            Kind::ArrowFunction | Kind::FunctionExpression | Kind::FunctionDeclaration | Kind::Constructor => false,
            Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor | Kind::PropertyAssignment => {
                n.name.is_some_and(|name| self.requires_scope_change(name))
            }
            Kind::PropertyDeclaration => {
                if b.has_modifier(node, Kind::StaticKeyword) {
                    !self.check.emit_standard_class_fields
                } else {
                    n.name.is_some_and(|name| self.requires_scope_change(name))
                }
            }
            kind => {
                let nullish_coalesce = kind == Kind::BinaryExpression && n.op == Kind::QuestionQuestionToken;
                let optional_chain = matches!(
                    kind,
                    Kind::PropertyAccessExpression
                        | Kind::ElementAccessExpression
                        | Kind::CallExpression
                        | Kind::NonNullExpression
                ) && n.flags.has(NodeFlags::OptionalChain);
                if nullish_coalesce || optional_chain {
                    return self.target < ScriptTarget::ES2020;
                }
                if kind == Kind::BindingElement
                    && self.child(node, 0).is_some()
                    && b.parent(node).is_some_and(|parent| b.kind(parent) == Kind::ObjectBindingPattern)
                {
                    return self.target < ScriptTarget::ES2017;
                }
                if (Kind::FirstTypeNode..=Kind::LastTypeNode).contains(&kind) {
                    return false;
                }
                b.file.children(node).into_iter().any(|child| self.requires_scope_change(child))
            }
        }
    }

    // --- marking -------------------------------------------------------------

    fn collect_private_members(&mut self) {
        let b = self.binder;
        for class in self.nodes.clone() {
            if !b.is_class_like(class) {
                continue;
            }
            for member in self.list(class, 1) {
                let declarations: Vec<(NodeId, Option<NodeId>)> = match b.kind(member) {
                    Kind::MethodDeclaration | Kind::PropertyDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
                        vec![(member, b.name_of(member))]
                    }
                    Kind::Constructor => self
                        .node(member)
                        .parameters
                        .as_ref()
                        .map_or_else(Vec::new, |parameters| parameters.nodes.clone())
                        .into_iter()
                        .filter(|&parameter| b.has_modifier(parameter, Kind::PrivateKeyword))
                        .map(|parameter| (parameter, b.name_of(parameter)))
                        .collect(),
                    _ => Vec::new(),
                };
                for (declaration, name) in declarations {
                    let Some(name) = name else { continue };
                    let is_private = b.kind(name) == Kind::PrivateIdentifier
                        || b.kind(declaration) == Kind::Parameter
                        || b.has_modifier(declaration, Kind::PrivateKeyword);
                    let Some(symbol) = b.symbol_of(declaration).filter(|_| is_private) else { continue };
                    let key = match b.kind(name) {
                        Kind::ComputedPropertyName => match self.node(name).expression {
                            Some(expression) if b.kind(expression) == Kind::Identifier => {
                                computed_key(self.text(expression))
                            }
                            _ => continue,
                        },
                        _ => self.text(name).to_string(),
                    };
                    self.private_members.entry(key).or_default().push(PrivateMember { symbol, class });
                }
            }
        }
    }

    fn mark_references(&mut self) {
        let b = self.binder;
        // A link names no location this port can resolve from, so every
        // declaration of the name counts as linked.
        if !self.check.jsdoc_link_names.is_empty() {
            let links: std::collections::HashSet<&str> =
                self.check.jsdoc_link_names.iter().map(String::as_str).collect();
            for entries in b.tables.values() {
                for (name, &symbol) in entries {
                    if links.contains(name.as_str()) {
                        self.reference_kinds[symbol] |= sf::All;
                    }
                }
            }
        }
        for node in self.nodes.clone() {
            match b.kind(node) {
                Kind::Identifier => self.identifier(node),
                Kind::PrivateIdentifier => self.private_identifier(node),
                Kind::ElementAccessExpression => {
                    if let Some(argument) = self.child(node, 1) {
                        self.element_access(node, argument);
                    }
                }
                Kind::BindingElement
                    if b.parent(node).is_some_and(|parent| b.kind(parent) == Kind::ObjectBindingPattern) =>
                {
                    // The property a destructuring reads is never a
                    // write-only reference.
                    let property = self.child(node, 1).or_else(|| b.name_of(node));
                    if let Some(property) = property
                        && matches!(b.kind(property), Kind::Identifier | Kind::StringLiteral | Kind::NumericLiteral)
                    {
                        let name = self.text(property).to_string();
                        let source = self.destructured_source(node);
                        let receiver = self.receiver_class(source);
                        self.mark_private_members_of(receiver, node, &name, None, false);
                    }
                }
                Kind::PropertyAssignment | Kind::ShorthandPropertyAssignment => {
                    if let Some(object) = b.parent(node)
                        && b.kind(object) == Kind::ObjectLiteralExpression
                        && self.access_kind(object) != Access::Read
                        && let Some(property) = b.name_of(node)
                        && matches!(b.kind(property), Kind::Identifier | Kind::StringLiteral | Kind::NumericLiteral)
                    {
                        let name = self.text(property).to_string();
                        // `({ x } = this)` reads `x` through `this`
                        // (`checkObjectLiteralDestructuringAssignment`'s
                        // `rightIsThis`).
                        let this = b
                            .parent(object)
                            .filter(|&assignment| {
                                b.kind(assignment) == Kind::BinaryExpression
                                    && self.node(assignment).op == Kind::EqualsToken
                                    && self.child(assignment, 0) == Some(object)
                            })
                            .and_then(|assignment| self.child(assignment, 2))
                            .filter(|&right| b.kind(right) == Kind::ThisKeyword);
                        let receiver = self.receiver_class(this);
                        self.mark_private_members_of(receiver, node, &name, this, false);
                    }
                }
                Kind::JsxOpeningElement | Kind::JsxSelfClosingElement => {
                    if let Some(tag) = self.node(node).name {
                        for read in &self.check.jsx_element_reads {
                            self.mark_jsx_factory(tag, read);
                        }
                    }
                }
                Kind::JsxOpeningFragment => {
                    for read in &self.check.jsx_fragment_reads {
                        self.mark_jsx_factory(node, read);
                    }
                }
                _ => {}
            }
        }
    }

    /// `markJsxAliasReferenced`: the factory the element compiles to is used
    /// wherever it resolves from the tag.
    fn mark_jsx_factory(&mut self, location: NodeId, name: &str) {
        if let Some(symbol) = self.resolve_use(location, name, sf::Value) {
            self.reference_kinds[symbol] |= sf::All;
        }
    }

    /// An identifier the checker resolves while checking: the meaning its
    /// place asks for, or none for a name a declaration introduces, a
    /// property's name, or a label.
    fn identifier(&mut self, node: NodeId) {
        let b = self.binder;
        let Some(parent) = b.parent(node) else { return };
        let is_name = self.node(parent).name == Some(node);
        let name = self.text(node).to_string();
        match b.kind(parent) {
            Kind::VariableDeclaration
            | Kind::Parameter
            | Kind::BindingElement
            | Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ClassDeclaration
            | Kind::ClassExpression
            | Kind::InterfaceDeclaration
            | Kind::TypeAliasDeclaration
            | Kind::EnumDeclaration
            | Kind::EnumMember
            | Kind::ModuleDeclaration
            | Kind::TypeParameter
            | Kind::ImportClause
            | Kind::NamespaceImport
            | Kind::NamespaceExport
            | Kind::NamespaceExportDeclaration
            | Kind::ImportEqualsDeclaration
            | Kind::PropertyDeclaration
            | Kind::PropertySignature
            | Kind::MethodDeclaration
            | Kind::MethodSignature
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::PropertyAssignment
            | Kind::JsxAttribute
            | Kind::MetaProperty
            | Kind::NamedTupleMember
            | Kind::ImportAttribute
                if is_name => {}
            Kind::BindingElement if self.child(parent, 1) == Some(node) => {}
            Kind::QualifiedName if self.child(parent, 1) == Some(node) => {}
            Kind::TypePredicate if self.child(parent, 1) == Some(node) => {}
            Kind::ImportSpecifier
            | Kind::LabeledStatement
            | Kind::BreakStatement
            | Kind::ContinueStatement
            | Kind::JsxNamespacedName
            | Kind::ImportType => {}
            Kind::PropertyAccessExpression if is_name => {
                let object = self.node(parent).expression;
                self.mark_private_members(parent, &name, object, true);
            }
            Kind::ExportSpecifier => self.export_specifier(parent, node, &name),
            Kind::ExportAssignment if self.node(parent).expression == Some(node) => {
                self.resolve_use(node, &name, sf::All);
            }
            Kind::JsxOpeningElement | Kind::JsxSelfClosingElement | Kind::JsxClosingElement if is_name => {
                if !is_intrinsic_tag_name(&name) {
                    self.resolve_use(node, &name, VALUE_USE);
                }
            }
            Kind::TypeReference if is_name => {
                self.resolve_use(node, &name, sf::Type);
            }
            _ => self.entity_or_expression(node, &name),
        }
    }

    /// The head of an entity name, or an identifier expression. `a` in
    /// `a.B` names a namespace in a type or import, a value in `typeof`.
    fn entity_or_expression(&mut self, node: NodeId, name: &str) {
        let b = self.binder;
        let mut top = node;
        while let Some(parent) = b.parent(top) {
            let continues = match b.kind(parent) {
                Kind::QualifiedName => self.child(parent, 0) == Some(top),
                Kind::PropertyAccessExpression => self.node(parent).expression == Some(top),
                _ => false,
            };
            if !continues {
                break;
            }
            top = parent;
        }
        let context = b.parent(top);
        let qualified = top != node;
        let meaning = match context.map(|context| b.kind(context)) {
            Some(Kind::TypeReference) if qualified => sf::Namespace,
            Some(Kind::ImportEqualsDeclaration) => sf::Namespace,
            Some(Kind::TypeQuery) => VALUE_USE,
            Some(Kind::ImportType) => return,
            Some(Kind::ExpressionWithTypeArguments) if !context.is_some_and(|context| self.is_class_extends(context)) => {
                if qualified { sf::Namespace } else { sf::Type }
            }
            _ => {
                // `getResolvedSymbol` resolves a write-only access without
                // counting it as a use.
                if self.access_kind(node) == Access::Write {
                    return;
                }
                VALUE_USE
            }
        };
        self.resolve_use(node, name, meaning);
    }

    fn is_class_extends(&self, expression_with_type_arguments: NodeId) -> bool {
        let b = self.binder;
        b.parent(expression_with_type_arguments).is_some_and(|clause| {
            b.kind(clause) == Kind::HeritageClause
                && self.node(clause).op == Kind::ExtendsKeyword
                && b.parent(clause).is_some_and(|container| b.is_class_like(container))
        })
    }

    /// `export { a as b }` resolves the local `a` it exports, unless it
    /// re-exports from another module.
    fn export_specifier(&mut self, specifier: NodeId, node: NodeId, name: &str) {
        let b = self.binder;
        let local = self.child(specifier, 0).or(self.node(specifier).name);
        if local != Some(node) {
            return;
        }
        let declaration = b.parent(specifier).and_then(|exports| b.parent(exports));
        if declaration.is_some_and(|declaration| self.child(declaration, 1).is_some()) {
            return;
        }
        self.resolve_use(node, name, sf::Value | sf::Type | sf::Namespace | sf::Alias);
    }

    fn private_identifier(&mut self, node: NodeId) {
        let b = self.binder;
        let Some(parent) = b.parent(node) else { return };
        let is_name = self.node(parent).name == Some(node);
        match b.kind(parent) {
            Kind::PropertyDeclaration | Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor if is_name => {}
            Kind::PropertyAccessExpression if is_name => {
                let object = self.node(parent).expression;
                if let Some(member) = self.lexical_private_member(node) {
                    self.mark_private_member(member, parent, object, true);
                }
            }
            // `#x in value` (`checkPrivateIdentifierExpression`).
            _ => {
                if let Some(member) = self.lexical_private_member(node) {
                    self.reference_kinds[member.symbol] |= sf::All;
                }
            }
        }
    }

    /// `lookupSymbolForPrivateIdentifierDeclaration`: the innermost class
    /// declaring the name.
    fn lexical_private_member(&self, node: NodeId) -> Option<PrivateMember> {
        let b = self.binder;
        let name = self.text(node);
        let mut class = b.containing_class(node);
        while let Some(current) = class {
            if let Some(symbol) = b.symbol_of(current) {
                let key = format!("{INTERNAL_PREFIX}#{symbol}@{name}");
                let found = [Table::Members(symbol), Table::Exports(symbol)]
                    .into_iter()
                    .find_map(|table| b.tables.get(&table).and_then(|table| table.get(&key)).copied());
                if let Some(member) = found {
                    return Some(PrivateMember { symbol: member, class: current });
                }
            }
            class = b.containing_class(current);
        }
        None
    }

    fn mark_private_members(&mut self, access: NodeId, name: &str, object: Option<NodeId>, may_be_write_only: bool) {
        let receiver = self.receiver_class(object);
        self.mark_private_members_of(receiver, access, name, object, may_be_write_only);
    }

    /// The members named `name` of the class the receiver is an instance or
    /// the constructor of, or of every class when that is unknown.
    fn mark_private_members_of(
        &mut self,
        receiver: Option<NodeId>,
        access: NodeId,
        name: &str,
        object: Option<NodeId>,
        may_be_write_only: bool,
    ) {
        let Some(members) = self.private_members.get(name).cloned() else { return };
        for member in members {
            if receiver.is_none_or(|class| class == member.class) {
                self.mark_private_member(member, access, object, may_be_write_only);
            }
        }
    }

    /// The class whose members a receiver's properties are: `this` in its
    /// class's members, or the class a name declares. A derived class sees
    /// its bases' members, which a name alone does not tell apart.
    fn receiver_class(&self, object: Option<NodeId>) -> Option<NodeId> {
        let b = self.binder;
        let mut object = object?;
        while b.kind(object) == Kind::ParenthesizedExpression {
            object = self.node(object).expression?;
        }
        let class = match b.kind(object) {
            Kind::ThisKeyword => self.this_class(object)?,
            Kind::Identifier => {
                let symbol = self.resolve_value(object)?;
                b.symbols[symbol]
                    .declarations
                    .iter()
                    .copied()
                    .find(|&declaration| b.kind(declaration) == Kind::ClassDeclaration)?
            }
            _ => return None,
        };
        self.list(class, 0).is_empty().then_some(class)
    }

    /// The class whose members' bodies `this` is read in.
    fn this_class(&self, node: NodeId) -> Option<NodeId> {
        let b = self.binder;
        let mut current = b.parent(node);
        while let Some(id) = current {
            match b.kind(id) {
                Kind::MethodDeclaration
                | Kind::GetAccessor
                | Kind::SetAccessor
                | Kind::Constructor
                | Kind::PropertyDeclaration
                | Kind::ClassStaticBlockDeclaration => {
                    return b.parent(id).filter(|&parent| b.is_class_like(parent));
                }
                Kind::FunctionDeclaration
                | Kind::FunctionExpression
                | Kind::ClassDeclaration
                | Kind::ClassExpression
                | Kind::ModuleDeclaration
                | Kind::SourceFile => return None,
                _ => {}
            }
            current = b.parent(id);
        }
        None
    }

    /// What a binding element destructures: the initializer of the
    /// declaration its pattern binds.
    fn destructured_source(&self, element: NodeId) -> Option<NodeId> {
        let b = self.binder;
        let pattern = b.parent(element)?;
        let declaration = b.parent(pattern)?;
        match b.kind(declaration) {
            Kind::VariableDeclaration | Kind::Parameter => self.node(declaration).initializer,
            _ => None,
        }
    }

    /// `x[key]` reads the members its key's literal types name
    /// (`getIndexedAccessType`): a literal key, a unique symbol that keys a
    /// computed member, a key whose declared type is a union of literals, or
    /// a `const` initialized with one. A key of a type this port cannot read
    /// may name any member.
    fn element_access(&mut self, access: NodeId, argument: NodeId) {
        let b = self.binder;
        let object = self.node(access).expression;
        if matches!(b.kind(argument), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral | Kind::NumericLiteral) {
            let name = self.text(argument).to_string();
            self.mark_private_members(access, &name, object, true);
            return;
        }
        if b.kind(argument) == Kind::Identifier {
            let key = computed_key(self.text(argument));
            if self.private_members.contains_key(&key) {
                self.mark_private_members(access, &key, object, true);
                return;
            }
        }
        let names = match self.literal_keys(argument) {
            Some(names) => names,
            None => {
                let receiver = self.receiver_class(object);
                let names: Vec<String> = self
                    .private_members
                    .iter()
                    .filter(|(_, members)| {
                        members.iter().any(|member| receiver.is_none_or(|class| class == member.class))
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                names
            }
        };
        for name in names {
            self.mark_private_members(access, &name, object, true);
        }
    }

    /// The literal names a key can take, when its declaration says.
    fn literal_keys(&self, argument: NodeId) -> Option<Vec<String>> {
        let b = self.binder;
        if b.kind(argument) != Kind::Identifier {
            return None;
        }
        let symbol = self.resolve_value(argument)?;
        let declaration = b.symbols[symbol].value_declaration?;
        if !matches!(b.kind(declaration), Kind::VariableDeclaration | Kind::Parameter) {
            return None;
        }
        let d = self.node(declaration);
        if let Some(annotation) = d.ty {
            return self.literal_type_keys(annotation);
        }
        let is_const = b.kind(declaration) == Kind::VariableDeclaration
            && b.combined_node_flags(declaration) & NodeFlags::BlockScoped == NodeFlags::Const;
        let initializer = d.initializer?;
        (is_const && matches!(b.kind(initializer), Kind::StringLiteral | Kind::NumericLiteral))
            .then(|| vec![self.text(initializer).to_string()])
    }

    fn literal_type_keys(&self, annotation: NodeId) -> Option<Vec<String>> {
        let b = self.binder;
        match b.kind(annotation) {
            Kind::LiteralType => {
                let literal = self.child(annotation, 0)?;
                matches!(b.kind(literal), Kind::StringLiteral | Kind::NumericLiteral)
                    .then(|| vec![self.text(literal).to_string()])
            }
            Kind::UnionType => {
                let mut keys = Vec::new();
                for member in self.list(annotation, 0) {
                    keys.extend(self.literal_type_keys(member)?);
                }
                Some(keys)
            }
            Kind::ParenthesizedType => self.node(annotation).ty.and_then(|inner| self.literal_type_keys(inner)),
            // No literal: an index of these types names no member.
            Kind::StringKeyword | Kind::NumberKeyword | Kind::SymbolKeyword | Kind::AnyKeyword => Some(Vec::new()),
            _ => None,
        }
    }

    /// `markPropertyAsReferenced`: a write that is only a write reads
    /// nothing, unless the member is a setter, and a method reaching itself
    /// through `this` or its class does not use itself.
    fn mark_private_member(&mut self, member: PrivateMember, access: NodeId, object: Option<NodeId>, may_be_write_only: bool) {
        let b = self.binder;
        if may_be_write_only
            && self.access_kind(access) == Access::Write
            && b.symbols[member.symbol].flags & sf::SetAccessor == 0
        {
            return;
        }
        if object.is_some_and(|object| self.is_self_type_access(object, member.class))
            && let Some(containing) = self.find_ancestor(access, ast::is_function_like_declaration_kind)
            && b.symbol_of(containing) == Some(member.symbol)
        {
            return;
        }
        self.reference_kinds[member.symbol] |= sf::All;
    }

    fn is_self_type_access(&self, object: NodeId, class: NodeId) -> bool {
        let b = self.binder;
        if b.kind(object) == Kind::ThisKeyword {
            return true;
        }
        if !b.is_entity_name_expression(object) {
            return false;
        }
        let mut first = object;
        while b.kind(first) == Kind::PropertyAccessExpression {
            let Some(expression) = self.node(first).expression else { return false };
            first = expression;
        }
        b.name_of(class).is_some_and(|class_name| self.text(class_name) == self.text(first))
    }

    /// `ast.accessKind`.
    fn access_kind(&self, node: NodeId) -> Access {
        let b = self.binder;
        let Some(parent) = b.parent(node) else { return Access::Read };
        let p = self.node(parent);
        match b.kind(parent) {
            Kind::ParenthesizedExpression | Kind::ArrayLiteralExpression => self.access_kind(parent),
            Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression => {
                if matches!(p.op, Kind::PlusPlusToken | Kind::MinusMinusToken) {
                    Access::ReadWrite
                } else {
                    Access::Read
                }
            }
            Kind::BinaryExpression => {
                if self.child(parent, 0) == Some(node) && ast::is_assignment_operator(p.op) {
                    if p.op == Kind::EqualsToken { Access::Write } else { Access::ReadWrite }
                } else {
                    Access::Read
                }
            }
            Kind::PropertyAccessExpression => {
                if p.name == Some(node) {
                    self.access_kind(parent)
                } else {
                    Access::Read
                }
            }
            Kind::PropertyAssignment => {
                let object = b.parent(parent).map_or(Access::Read, |object| self.access_kind(object));
                // In `({ x: target } = value)` the property is read and the
                // target written.
                if p.name == Some(node) {
                    match object {
                        Access::Read => Access::Write,
                        Access::Write => Access::Read,
                        Access::ReadWrite => Access::ReadWrite,
                    }
                } else {
                    object
                }
            }
            Kind::ShorthandPropertyAssignment => {
                if p.initializer == Some(node) {
                    Access::Read
                } else {
                    b.parent(parent).map_or(Access::Read, |object| self.access_kind(object))
                }
            }
            Kind::ForInStatement | Kind::ForOfStatement => {
                if p.initializer == Some(node) {
                    Access::Write
                } else {
                    Access::Read
                }
            }
            _ => Access::Read,
        }
    }

    // --- checking ------------------------------------------------------------

    /// The nodes the checker registers while checking the file
    /// (`registerForUnusedIdentifiersCheck`), each checked as
    /// `checkUnusedIdentifiers` does.
    fn check_nodes(&mut self) {
        let b = self.binder;
        for node in self.nodes.clone() {
            let has_locals = b.tables.get(&Table::Locals(node)).is_some_and(|locals| !locals.is_empty());
            match b.kind(node) {
                Kind::SourceFile => {
                    if b.file.external_module {
                        self.check_locals_and_parameters(node);
                    }
                }
                Kind::ModuleDeclaration => {
                    if self.node(node).body.is_some() && !b.is_global_scope_augmentation(node) {
                        self.check_locals_and_parameters(node);
                    }
                }
                Kind::Block | Kind::CaseBlock | Kind::ForStatement | Kind::ForInStatement | Kind::ForOfStatement => {
                    if has_locals {
                        self.check_locals_and_parameters(node);
                    }
                }
                Kind::Constructor
                | Kind::FunctionExpression
                | Kind::FunctionDeclaration
                | Kind::ArrowFunction
                | Kind::MethodDeclaration
                | Kind::GetAccessor
                | Kind::SetAccessor => {
                    // An overload's parameters are not reported, only the
                    // implementation's.
                    if self.node(node).body.is_some() {
                        self.check_locals_and_parameters(node);
                    }
                    self.check_type_parameters(node);
                }
                Kind::MethodSignature
                | Kind::CallSignature
                | Kind::ConstructSignature
                | Kind::FunctionType
                | Kind::ConstructorType
                | Kind::TypeAliasDeclaration
                | Kind::InterfaceDeclaration => self.check_type_parameters(node),
                Kind::ClassDeclaration | Kind::ClassExpression => {
                    self.check_class_members(node);
                    self.check_type_parameters(node);
                }
                Kind::InferType => self.check_infer_type_parameter(node),
                _ => {}
            }
        }
    }

    fn report(&mut self, location: NodeId, kind: UnusedKind, range: (usize, usize), message: &'static Message, args: Vec<String>) {
        let flags = self.binder.flags(location);
        if flags.has(NodeFlags::Ambient) || flags.has(NodeFlags::ThisNodeOrAnySubNodesHasError) {
            return;
        }
        let enabled = match kind {
            UnusedKind::Local => self.check.locals,
            UnusedKind::Parameter => self.check.parameters,
        };
        if enabled {
            self.diagnostics.push(Diagnostic { start: range.0, end: range.1, message, args });
        }
    }

    fn report_on(&mut self, location: NodeId, kind: UnusedKind, node: NodeId, message: &'static Message, args: Vec<String>) {
        let range = self.binder.error_range_for_node(node);
        self.report(location, kind, range, message, args);
    }

    fn check_class_members(&mut self, class: NodeId) {
        let b = self.binder;
        for member in self.list(class, 1) {
            match b.kind(member) {
                Kind::MethodDeclaration | Kind::PropertyDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
                    let Some(symbol) = b.symbol_of(member) else { continue };
                    // A getter already reported the pair.
                    if b.kind(member) == Kind::SetAccessor && b.symbols[symbol].flags & sf::GetAccessor != 0 {
                        continue;
                    }
                    let Some(name) = b.name_of(member) else { continue };
                    let is_private =
                        b.has_modifier(member, Kind::PrivateKeyword) || b.kind(name) == Kind::PrivateIdentifier;
                    if self.reference_kinds[symbol] == 0 && is_private && !b.flags(member).has(NodeFlags::Ambient) {
                        let display = self.member_display_name(name);
                        self.report_on(
                            member,
                            UnusedKind::Local,
                            name,
                            diagnostics::X_0_is_declared_but_its_value_is_never_read,
                            vec![display],
                        );
                    }
                }
                Kind::Constructor => {
                    let parameters = self
                        .node(member)
                        .parameters
                        .as_ref()
                        .map_or_else(Vec::new, |parameters| parameters.nodes.clone());
                    for parameter in parameters {
                        let Some(symbol) = b.symbol_of(parameter) else { continue };
                        if self.reference_kinds[symbol] == 0
                            && b.has_modifier(parameter, Kind::PrivateKeyword)
                            && let Some(name) = b.name_of(parameter)
                        {
                            let display = self.text(name).to_string();
                            self.report_on(
                                parameter,
                                UnusedKind::Local,
                                name,
                                diagnostics::Property_0_is_declared_but_its_value_is_never_read,
                                vec![display],
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// `symbolToString` of a member: a string-literal name keeps its quotes.
    fn member_display_name(&self, name: NodeId) -> String {
        match self.binder.kind(name) {
            Kind::Identifier | Kind::PrivateIdentifier | Kind::NumericLiteral => self.text(name).to_string(),
            Kind::StringLiteral => format!("\"{}\"", self.text(name)),
            _ => self.binder.declaration_name_to_string(Some(name)),
        }
    }

    fn check_locals_and_parameters(&mut self, node: NodeId) {
        let b = self.binder;
        let Some(locals) = b.tables.get(&Table::Locals(node)) else { return };
        let mut locals: Vec<(&String, SymbolId)> = locals.iter().map(|(name, &symbol)| (name, symbol)).collect();
        locals.sort_by_key(|&(_, symbol)| symbol);
        let mut variable_parents: Vec<NodeId> = Vec::new();
        let mut import_clauses: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
        for (name, local) in locals {
            let symbol = &b.symbols[local];
            let reference_kinds = self.reference_kinds[local];
            let skip = if symbol.flags & sf::TypeParameter != 0 {
                // A type parameter is checked with its list, unless it is
                // merged with a parameter nothing reads.
                symbol.flags & sf::Variable == 0 || reference_kinds & sf::Variable != 0
            } else {
                reference_kinds != 0 || symbol.export_symbol.is_some()
            };
            if skip {
                continue;
            }
            for &declaration in &symbol.declarations {
                match b.kind(declaration) {
                    Kind::VariableDeclaration | Kind::Parameter | Kind::BindingElement => {
                        if let Some(parent) = b.parent(b.root_declaration(declaration))
                            && !variable_parents.contains(&parent)
                        {
                            variable_parents.push(parent);
                        }
                    }
                    Kind::ImportClause | Kind::ImportSpecifier | Kind::NamespaceImport => {
                        if b.name_of(declaration).is_some_and(|name| self.starts_with_underscore(name)) {
                            continue;
                        }
                        let Some(clause) = self.import_clause_of(declaration) else { continue };
                        match import_clauses.iter_mut().find(|(existing, _)| *existing == clause) {
                            Some((_, unused)) => unused.push(declaration),
                            None => import_clauses.push((clause, vec![declaration])),
                        }
                    }
                    Kind::TypeParameter => {}
                    _ => {
                        if !b.is_ambient_module(declaration) {
                            self.report_unused_local(declaration, name);
                        }
                    }
                }
            }
        }
        for parent in variable_parents {
            if b.kind(parent) == Kind::VariableDeclarationList {
                self.report_unused_variables(parent);
            } else {
                let parameters =
                    self.node(parent).parameters.as_ref().map_or_else(Vec::new, |parameters| parameters.nodes.clone());
                self.report_unused_variable_declarations(&parameters);
            }
        }
        for (clause, unused) in import_clauses {
            self.report_unused_imports(clause, &unused);
        }
    }

    fn import_clause_of(&self, declaration: NodeId) -> Option<NodeId> {
        let b = self.binder;
        match b.kind(declaration) {
            Kind::ImportClause => Some(declaration),
            Kind::NamespaceImport => b.parent(declaration),
            _ => b.parent(declaration).and_then(|named| b.parent(named)),
        }
    }

    fn report_unused_local(&mut self, declaration: NodeId, name: &str) {
        let b = self.binder;
        let message = if b.is_type_declaration(declaration) {
            diagnostics::X_0_is_declared_but_never_used
        } else {
            diagnostics::X_0_is_declared_but_its_value_is_never_read
        };
        let node = b.name_of(declaration).unwrap_or(declaration);
        self.report_on(declaration, UnusedKind::Local, node, message, vec![name.to_string()]);
    }

    fn report_unused_variables(&mut self, list: NodeId) {
        let declarations = self.list(list, 0);
        if declarations.len() > 1 && declarations.iter().all(|&declaration| self.is_unreferenced_variable(declaration)) {
            self.report_unused_variable(list, list, diagnostics::All_variables_are_unused, Vec::new());
        } else {
            self.report_unused_variable_declarations(&declarations);
        }
    }

    fn report_unused_binding_elements(&mut self, pattern: NodeId) {
        let elements = self.list(pattern, 0);
        if elements.len() > 1 && elements.iter().all(|&element| self.is_unreferenced_variable(element)) {
            self.report_unused_variable(pattern, pattern, diagnostics::All_destructured_elements_are_unused, Vec::new());
        } else {
            self.report_unused_variable_declarations(&elements);
        }
    }

    fn report_unused_variable_declarations(&mut self, declarations: &[NodeId]) {
        let b = self.binder;
        for &declaration in declarations {
            let Some(name) = b.name_of(declaration) else { continue };
            if self.is_parameter_property(declaration) || self.is_this_parameter(declaration) {
                continue;
            }
            if matches!(b.kind(name), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern) {
                self.report_unused_binding_elements(name);
            } else if self.is_unreferenced_variable(declaration) {
                let text = self.text(name).to_string();
                self.report_unused_variable(
                    declaration,
                    name,
                    diagnostics::X_0_is_declared_but_its_value_is_never_read,
                    vec![text],
                );
            }
        }
    }

    /// Reported as a parameter when it is (part of) one.
    fn report_unused_variable(&mut self, location: NodeId, node: NodeId, message: &'static Message, args: Vec<String>) {
        let b = self.binder;
        let mut owner = location;
        while matches!(b.kind(owner), Kind::BindingElement | Kind::ObjectBindingPattern | Kind::ArrayBindingPattern) {
            match b.parent(owner) {
                Some(parent) => owner = parent,
                None => break,
            }
        }
        let kind = if b.kind(owner) == Kind::Parameter { UnusedKind::Parameter } else { UnusedKind::Local };
        self.report_on(owner, kind, node, message, args);
    }

    fn is_unreferenced_variable(&self, node: NodeId) -> bool {
        let b = self.binder;
        let Some(name) = b.name_of(node) else { return true };
        if matches!(b.kind(name), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern) {
            return self.list(name, 0).iter().all(|&element| self.is_unreferenced_variable(element));
        }
        if b.symbol_of(node).is_some_and(|symbol| self.reference_kinds[symbol] & sf::Variable != 0) {
            return false;
        }
        let parent = b.parent(node);
        let in_object_pattern = parent.is_some_and(|parent| b.kind(parent) == Kind::ObjectBindingPattern);
        if b.kind(node) == Kind::BindingElement && in_object_pattern {
            // In `{ a, ...b }`, `a` is used: it keeps its property out of `b`.
            let elements = parent.map_or_else(Vec::new, |parent| self.list(parent, 0));
            if let Some(&last) = elements.last()
                && last != node
                && self.child(last, 0).is_some()
            {
                return false;
            }
        }
        let underscore_exempt = match b.kind(node) {
            Kind::Parameter => true,
            Kind::VariableDeclaration => {
                parent
                    .and_then(|list| b.parent(list))
                    .is_some_and(|statement| matches!(b.kind(statement), Kind::ForInStatement | Kind::ForOfStatement))
                    || b.combined_node_flags(node).has(NodeFlags::Using)
            }
            Kind::BindingElement => !(in_object_pattern && self.child(node, 1).is_none()),
            _ => false,
        };
        !(underscore_exempt && self.starts_with_underscore(name))
    }

    fn is_parameter_property(&self, node: NodeId) -> bool {
        let b = self.binder;
        b.kind(node) == Kind::Parameter
            && b.parent(node).is_some_and(|parent| b.kind(parent) == Kind::Constructor)
            && self
                .node(node)
                .modifier_nodes()
                .iter()
                .any(|&modifier| ast::is_parameter_property_modifier(b.kind(modifier)))
    }

    fn is_this_parameter(&self, node: NodeId) -> bool {
        let b = self.binder;
        b.kind(node) == Kind::Parameter
            && b.name_of(node).is_some_and(|name| b.kind(name) == Kind::Identifier && self.text(name) == "this")
    }

    fn report_unused_imports(&mut self, clause: NodeId, unused: &[NodeId]) {
        let b = self.binder;
        let mut declarations = usize::from(b.name_of(clause).is_some());
        if let Some(bindings) = self.child(clause, 0) {
            declarations +=
                if b.kind(bindings) == Kind::NamespaceImport { 1 } else { self.list(bindings, 0).len() };
        }
        if declarations > 1 && declarations == unused.len() {
            if let Some(import) = b.parent(clause) {
                self.report_on(import, UnusedKind::Local, import, diagnostics::All_imports_in_import_declaration_are_unused, Vec::new());
            }
            return;
        }
        for &declaration in unused {
            let name = b.name_of(declaration).map(|name| self.text(name).to_string()).unwrap_or_default();
            self.report_unused_local(declaration, &name);
        }
    }

    fn is_unreferenced_type_parameter(&self, type_parameter: NodeId) -> bool {
        let b = self.binder;
        b.symbol_of(type_parameter).is_none_or(|symbol| self.reference_kinds[symbol] & sf::TypeParameter == 0)
            && !b.name_of(type_parameter).is_some_and(|name| self.starts_with_underscore(name))
    }

    fn check_type_parameters(&mut self, node: NodeId) {
        let Some(list) = self.node(node).type_parameters.clone() else { return };
        let type_parameters = list.nodes.clone();
        if type_parameters.is_empty() {
            return;
        }
        if type_parameters.len() > 1 && type_parameters.iter().all(|&parameter| self.is_unreferenced_type_parameter(parameter)) {
            let range = self.binder.range_of_type_parameters(&list);
            self.report(node, UnusedKind::Parameter, range, diagnostics::All_type_parameters_are_unused, Vec::new());
            return;
        }
        for parameter in type_parameters {
            if self.is_unreferenced_type_parameter(parameter)
                && let Some(name) = self.binder.name_of(parameter)
            {
                let text = self.text(name).to_string();
                self.report_on(node, UnusedKind::Parameter, parameter, diagnostics::X_0_is_declared_but_never_used, vec![text]);
            }
        }
    }

    fn check_infer_type_parameter(&mut self, node: NodeId) {
        let Some(type_parameter) = self.child(node, 0) else { return };
        if self.is_unreferenced_type_parameter(type_parameter)
            && let Some(name) = self.binder.name_of(type_parameter)
        {
            let text = self.text(name).to_string();
            self.report_on(node, UnusedKind::Parameter, name, diagnostics::X_0_is_declared_but_never_used, vec![text]);
        }
    }
}

fn tree_order(binder: &Binder<'_>) -> Vec<NodeId> {
    let mut order = Vec::new();
    let mut stack = vec![binder.file.root];
    while let Some(node) = stack.pop() {
        order.push(node);
        let mut children = binder.file.children(node);
        children.reverse();
        stack.extend(children);
    }
    order
}

/// How a computed member named by a unique symbol is keyed: `[key]`.
fn computed_key(expression: &str) -> String {
    format!("[{expression}]")
}

/// `isIntrinsicJsxName`: a lower-case or dashed tag names an element, not a
/// value.
fn is_intrinsic_tag_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase()) || name.contains('-')
}
