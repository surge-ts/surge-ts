//! tsc's deferred conditional types (`ConditionalType`).
//!
//! surge's checker keeps a deferred conditional only while the generic body
//! that made it is checked; afterwards its type is a type parameter named by
//! its text. The conditional is read back from where it was written: an
//! annotation that writes it, or a reference to a type alias whose body does
//! (tsc's `root` and the `mapper` the reference instantiated it with).

use std::sync::Arc;

use surge_ts_types::Type;

use super::super::enums_generated::{SyntaxKind, symbol_flags as ts_sf, type_flags as tf};
use super::super::handles::{NodeId, SymbolId, TypeId};
use super::super::node::NodePropertyValue;
use super::{TypeChecker, Unsupported, mentions_name};

/// Where a deferred conditional type was written.
pub(crate) struct ConditionalSite {
    /// `root.node`: the conditional type node.
    root: NodeId,
    /// The arguments the alias references leading to the root instantiated
    /// it with, by the alias's type parameter names (`mapper`).
    mapper: Vec<(String, TypeId)>,
    /// The outermost alias the conditional was written through, and its
    /// arguments (`aliasSymbol`, `aliasTypeArguments`).
    alias: Option<(SymbolId, Vec<TypeId>)>,
    /// The annotation that wrote it: the names its arguments write resolve
    /// there.
    written: NodeId,
}

/// A `ConditionalType`'s `root` and its instantiated `checkType` and
/// `extendsType`.
pub struct ConditionalParts {
    /// `root.node`.
    pub root: NodeId,
    pub root_check_type: TypeId,
    pub root_extends_type: TypeId,
    pub is_distributive: bool,
    /// `root.inferTypeParameters`.
    pub infer_type_parameters: Vec<TypeId>,
    /// `root.aliasSymbol`: the alias the root is the body of.
    pub root_alias_symbol: Option<SymbolId>,
    pub check_type: TypeId,
    pub extends_type: TypeId,
}

const ALIAS_DEPTH: u32 = 8;

impl TypeChecker {
    /// The conditional a type handle stands for, when its type is a deferred
    /// conditional the checker left as a type parameter.
    pub(super) fn conditional_site(&self, ty: TypeId) -> Option<Arc<ConditionalSite>> {
        if let Some(known) = self.with_tables(|tables| tables.conditional_sites.get(&ty.index).cloned()) {
            return known;
        }
        let site = self.find_conditional_site(ty).map(Arc::new);
        self.with_tables(|tables| tables.conditional_sites.insert(ty.index, site.clone()));
        site
    }

    fn find_conditional_site(&self, ty: TypeId) -> Option<ConditionalSite> {
        if !matches!(self.ty(ty), Type::TypeParameter(_)) {
            return None;
        }
        let written = self.written_type_node(ty)?;
        self.conditional_site_at(written, written, Vec::new(), 0)
    }

    /// The type node a handle's type was written as: the node it is the type
    /// of, or the annotation of the declaration whose type it is.
    fn written_type_node(&self, ty: TypeId) -> Option<NodeId> {
        let symbol = match self.location(ty) {
            Some(location) if self.program.is_part_of_type_node(location) => return Some(location),
            Some(location) => self.symbol_at_location(location)?,
            None => self.origin(ty)?,
        };
        self.symbol_declarations(symbol).into_iter().find_map(|declaration| self.declaration_annotation(declaration))
    }

    fn conditional_site_at(&self, node: NodeId, written: NodeId, mapper: Vec<(String, TypeId)>, depth: u32) -> Option<ConditionalSite> {
        let node = self.unwrap_parentheses(node);
        match self.program.node_kind(node) {
            SyntaxKind::ConditionalType => Some(ConditionalSite { root: node, mapper, alias: None, written }),
            SyntaxKind::TypeReference if depth < ALIAS_DEPTH => {
                let symbol = self.symbol_at_location(self.node_child(node, "typeName")?)?;
                let symbol = if self.is_alias(symbol) { self.aliased_symbol(symbol).unwrap_or(symbol) } else { symbol };
                if self.symbol_flags(symbol) & ts_sf::TypeAlias == 0 {
                    return None;
                }
                let declaration = self
                    .symbol_declarations(symbol)
                    .into_iter()
                    .find(|declaration| self.program.node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)?;
                let parameters = self.list_property(declaration, "typeParameters");
                let arguments = self.list_property(node, "typeArguments");
                if arguments.len() > parameters.len() {
                    return None;
                }
                let mut instantiation = Vec::with_capacity(parameters.len());
                for (position, parameter) in parameters.iter().enumerate() {
                    let name = self.program.node_text(self.node_child(*parameter, "name")?).to_string();
                    let argument = match arguments.get(position) {
                        Some(argument) => self.instantiate_type_node(*argument, &mapper, written)?,
                        None => self.instantiate_type_node(self.node_child(*parameter, "default")?, &instantiation, written)?,
                    };
                    instantiation.push((name, argument));
                }
                let alias_arguments = instantiation.iter().map(|(_, argument)| *argument).collect();
                let body = self.node_child(declaration, "type")?;
                let mut site = self.conditional_site_at(body, written, instantiation, depth + 1)?;
                site.alias = Some((symbol, alias_arguments));
                Some(site)
            }
            _ => None,
        }
    }

    fn unwrap_parentheses(&self, mut node: NodeId) -> NodeId {
        while self.program.node_kind(node) == SyntaxKind::ParenthesizedType {
            match self.node_child(node, "type") {
                Some(inner) => node = inner,
                None => break,
            }
        }
        node
    }

    /// The type a node of an alias's body writes, its type parameters
    /// replaced by `mapper`'s arguments (tsc's `instantiateType` of the
    /// node's type). A name the mapper binds is that very argument.
    fn instantiate_type_node(&self, node: NodeId, mapper: &[(String, TypeId)], scope: NodeId) -> Option<TypeId> {
        if mapper.is_empty() {
            return Some(self.type_at_location(node));
        }
        let unwrapped = self.unwrap_parentheses(node);
        if self.program.node_kind(unwrapped) == SyntaxKind::TypeReference && self.list_property(unwrapped, "typeArguments").is_empty() {
            let text = self.program.node_text(unwrapped);
            if let Some((_, argument)) = mapper.iter().find(|(name, _)| name == text) {
                return Some(*argument);
            }
        }
        let names: Vec<String> = mapper.iter().map(|(name, _)| name.clone()).collect();
        if !mentions_name(self.program.node_text(node), &names) {
            return Some(self.type_at_location(node));
        }
        let ty = self.resolve_bound_type_node(node, &self.mapper_bindings(mapper), false)?;
        Some(self.intern_in_scope(Some(scope), ty))
    }

    fn mapper_bindings(&self, mapper: &[(String, TypeId)]) -> Vec<(String, Type)> {
        mapper.iter().map(|(name, argument)| (name.clone(), self.ty(*argument))).collect()
    }

    /// A type node's type with some type parameters in scope bound.
    fn resolve_bound_type_node(&self, node: NodeId, bindings: &[(String, Type)], for_constraint: bool) -> Option<Type> {
        let checker_index = self.program.file(node.source_file()).checker_index?;
        let text = self.type_node_text(node);
        let type_parameters = self.type_parameters_in_scope(node);
        let namespaces = self.namespace_scope(node);
        let local = self.local_type_anchor(node);
        self.query(|query| {
            query.resolve_bound_type_text(checker_index, &text, &type_parameters, &namespaces, local, bindings, for_constraint)
        })
        .flatten()
    }

    /// The name of a local type declared in the innermost block around
    /// `node` that declares one: the checker keeps that body's type scope
    /// under it.
    pub(super) fn local_type_anchor(&self, node: NodeId) -> Option<(usize, usize)> {
        let mut current = self.program.node_parent(node);
        while let Some(owner) = current {
            match self.program.node_kind(owner) {
                SyntaxKind::SourceFile | SyntaxKind::ModuleBlock => return None,
                SyntaxKind::Block => {
                    let declared = self.list_property(owner, "statements").into_iter().find_map(|statement| {
                        matches!(
                            self.program.node_kind(statement),
                            SyntaxKind::TypeAliasDeclaration | SyntaxKind::InterfaceDeclaration | SyntaxKind::ClassDeclaration
                        )
                        .then(|| self.node_child(statement, "name"))
                        .flatten()
                    });
                    if let Some(name) = declared {
                        return Some(self.program.node_byte_span(name));
                    }
                }
                _ => {}
            }
            current = self.program.node_parent(owner);
        }
        None
    }

    /// `root.isDistributive`: the check type is a naked type parameter,
    /// named here.
    fn distributive_parameter(&self, root: NodeId) -> Option<String> {
        let check = self.unwrap_parentheses(self.node_child(root, "checkType")?);
        if self.program.node_kind(check) != SyntaxKind::TypeReference || !self.list_property(check, "typeArguments").is_empty() {
            return None;
        }
        let symbol = self.symbol_at_location(self.node_child(check, "typeName")?)?;
        (self.symbol_flags(symbol) & ts_sf::TypeParameter != 0).then(|| self.program.node_text(check).to_string())
    }

    /// The `infer` type parameter declarations of a conditional's extends
    /// type.
    pub(super) fn infer_declarations(&self, root: NodeId) -> Vec<NodeId> {
        fn walk(checker: &TypeChecker, node: NodeId, found: &mut Vec<NodeId>) {
            if checker.program.node_kind(node) == SyntaxKind::InferType
                && let Some(parameter) = checker.node_child(node, "typeParameter")
            {
                found.push(parameter);
            }
            for (_, value) in checker.program.node_properties(node) {
                match value {
                    NodePropertyValue::Node(child) => walk(checker, child, found),
                    NodePropertyValue::List { nodes, .. } => nodes.into_iter().for_each(|child| walk(checker, child, found)),
                }
            }
        }
        let mut found = Vec::new();
        if let Some(extends) = self.node_child(root, "extendsType") {
            walk(self, extends, &mut found);
        }
        found
    }

    /// `ConditionalType`'s `root`, `checkType` and `extendsType`.
    pub fn conditional_parts(&self, ty: TypeId) -> Option<ConditionalParts> {
        let site = self.conditional_site(ty)?;
        let check = self.node_child(site.root, "checkType")?;
        let extends = self.node_child(site.root, "extendsType")?;
        let infer_type_parameters = self
            .infer_declarations(site.root)
            .into_iter()
            .filter_map(|declaration| self.declared_symbol(declaration))
            .map(|symbol| self.declared_type_of_symbol(symbol))
            .collect();
        let root_alias_symbol = self
            .program
            .node_parent(site.root)
            .filter(|parent| self.program.node_kind(*parent) == SyntaxKind::TypeAliasDeclaration)
            .and_then(|declaration| self.declared_symbol(declaration));
        Some(ConditionalParts {
            root: site.root,
            root_check_type: self.type_at_location(check),
            root_extends_type: self.type_at_location(extends),
            is_distributive: self.distributive_parameter(site.root).is_some(),
            infer_type_parameters,
            root_alias_symbol,
            check_type: self.instantiate_type_node(check, &site.mapper, site.written)?,
            extends_type: self.instantiate_type_node(extends, &site.mapper, site.written)?,
        })
    }

    /// The alias a conditional was written through, and its arguments.
    pub(super) fn conditional_alias(&self, ty: TypeId) -> Option<(SymbolId, Vec<TypeId>)> {
        self.conditional_site(ty)?.alias.clone()
    }

    /// `getBaseConstraintOfType` of a conditional (`computeBaseConstraint`):
    /// the conditional over its check type's constraint when it distributes
    /// and that is not `never` (`getConstraintOfDistributiveConditionalType`),
    /// else the union of its branches (`getDefaultConstraintOfConditionalType`),
    /// each then constrained in turn.
    pub(super) fn conditional_constraint(&self, ty: TypeId) -> Result<Option<TypeId>, Unsupported> {
        let unsupported = Unsupported("getBaseConstraintOfType for a conditional type whose branches surge could not resolve");
        let site = self.conditional_site(ty).ok_or(unsupported)?;
        let degraded = |ty: &Type| matches!(ty, Type::Unknown | Type::ErrorType);
        let bindings = self.mapper_bindings(&site.mapper);
        if let Some(parameter) = self.distributive_parameter(site.root) {
            let check_node = self.node_child(site.root, "checkType").ok_or(unsupported)?;
            let check = self.instantiate_type_node(check_node, &site.mapper, site.written).ok_or(unsupported)?;
            let constraint = match self.ty(check) {
                Type::TypeParameter(_) if self.type_flags(check) & tf::TypeParameter != 0 => self.base_constraint_of_type(check)?,
                _ => None,
            };
            if let Some(constraint) = constraint.map(|constraint| self.ty(constraint)).filter(|constraint| *constraint != self.ty(check)) {
                let mut bound = bindings.clone();
                bind(&mut bound, parameter, constraint);
                if let Some(instantiated) = self
                    .resolve_bound_type_node(site.root, &bound, true)
                    .filter(|instantiated| !matches!(instantiated, Type::Never) && !degraded(instantiated))
                {
                    return self.constraint_of_resolved(site.written, instantiated);
                }
            }
        }
        let mut bound = bindings;
        for declaration in self.infer_declarations(site.root) {
            if let Some(name) = self.node_child(declaration, "name") {
                bind(&mut bound, self.program.node_text(name).to_string(), Type::GenuineUnknown);
            }
        }
        let branch = |property: &str| {
            let node = self.node_child(site.root, property)?;
            self.resolve_bound_type_node(node, &bound, false).filter(|ty| !degraded(ty))
        };
        let true_type = branch("trueType").ok_or(unsupported)?;
        let false_type = branch("falseType").ok_or(unsupported)?;
        let combined = match (true_type, false_type) {
            (Type::Any, false_type) => false_type,
            (true_type, Type::Any) => true_type,
            (true_type, false_type) => surge_ts_types::union_type(vec![true_type, false_type]),
        };
        self.constraint_of_resolved(site.written, combined)
    }

    /// `getBaseConstraint` of a constraint resolved where `scope` writes it:
    /// a type variable's or union's own base constraint, any other type
    /// itself.
    fn constraint_of_resolved(&self, scope: NodeId, ty: Type) -> Result<Option<TypeId>, Unsupported> {
        let handle = self.intern_in_scope(Some(scope), ty);
        if self.type_flags(handle) & (tf::TypeParameter | tf::Conditional | tf::Union | tf::Intersection) != 0 {
            self.base_constraint_of_type(handle)
        } else {
            Ok(Some(handle))
        }
    }
}

fn bind(bindings: &mut Vec<(String, Type)>, name: String, ty: Type) {
    match bindings.iter_mut().find(|(bound, _)| *bound == name) {
        Some(binding) => binding.1 = ty,
        None => bindings.push((name, ty)),
    }
}
