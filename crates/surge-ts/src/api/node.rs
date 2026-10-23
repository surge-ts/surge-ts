//! TypeScript's view of a node: its flags as `NodeFlags`, its named
//! children and scalar properties.

use surge_ts_tsc_syntax::Kind;
use surge_ts_tsc_syntax::flags::NodeFlags as TscNodeFlags;
use surge_ts_tsc_syntax::properties::{self, Slot};

use super::enums_generated::{SyntaxKind, node_flags as ts, syntax_kind_of};
use super::handles::NodeId;
use super::positions::PositionMap;
use super::program::Program;
use surge_ts_tsc_syntax::SyntaxTree;

/// typescript-go numbers `NodeFlags` differently from TypeScript's JavaScript
/// compiler; this is the TypeScript 6 value of every flag the two share. A
/// module declaration's `namespace` and `global` keywords, which
/// typescript-go records as the declaration's keyword, become TypeScript's
/// `Namespace` and `GlobalAugmentation` flags.
pub(crate) fn typescript_node_flags(kind: Kind, flags: TscNodeFlags, keyword: Kind) -> i32 {
    const SHARED: &[(TscNodeFlags, i32)] = &[
        (TscNodeFlags::Let, ts::Let),
        (TscNodeFlags::Const, ts::Const),
        (TscNodeFlags::Using, ts::Using),
        (TscNodeFlags::Synthesized, ts::Synthesized),
        (TscNodeFlags::ExportContext, ts::ExportContext),
        (TscNodeFlags::ContainsThis, ts::ContainsThis),
        (TscNodeFlags::HasImplicitReturn, ts::HasImplicitReturn),
        (TscNodeFlags::HasExplicitReturn, ts::HasExplicitReturn),
        (TscNodeFlags::DisallowInContext, ts::DisallowInContext),
        (TscNodeFlags::YieldContext, ts::YieldContext),
        (TscNodeFlags::DecoratorContext, ts::DecoratorContext),
        (TscNodeFlags::AwaitContext, ts::AwaitContext),
        (TscNodeFlags::DisallowConditionalTypesContext, ts::DisallowConditionalTypesContext),
        (TscNodeFlags::ThisNodeHasError, ts::ThisNodeHasError),
        (TscNodeFlags::JavaScriptFile, ts::JavaScriptFile),
        (TscNodeFlags::ThisNodeOrAnySubNodesHasError, ts::ThisNodeOrAnySubNodesHasError),
        (TscNodeFlags::HasAsyncFunctions, ts::HasAsyncFunctions),
        (TscNodeFlags::PossiblyContainsDynamicImport, ts::PossiblyContainsDynamicImport),
        (TscNodeFlags::PossiblyContainsImportMeta, ts::PossiblyContainsImportMeta),
        (TscNodeFlags::JSDoc, ts::JSDoc),
        (TscNodeFlags::Ambient, ts::Ambient),
        (TscNodeFlags::InWithStatement, ts::InWithStatement),
        (TscNodeFlags::JsonFile, ts::JsonFile),
        (TscNodeFlags::Unreachable, ts::Unreachable),
    ];
    let mut out = 0;
    for &(tsgo, typescript) in SHARED {
        if flags.has(tsgo) {
            out |= typescript;
        }
    }
    // The bit typescript-go shares between `OptionalChain` and a module
    // declaration's `NestedNamespace`.
    if flags.has(TscNodeFlags::OptionalChain) {
        out |= if kind == Kind::ModuleDeclaration { ts::NestedNamespace } else { ts::OptionalChain };
    }
    if kind == Kind::ModuleDeclaration {
        match keyword {
            Kind::NamespaceKeyword => out |= ts::Namespace,
            Kind::GlobalKeyword => out |= ts::GlobalAugmentation,
            _ => {}
        }
    }
    out
}

/// Every node kind's node-valued TypeScript properties, in `forEachChild`
/// order: `(kind, [(property, is a NodeArray)])`.
pub fn node_schema() -> Vec<(SyntaxKind, Vec<(&'static str, bool)>)> {
    Kind::ALL
        .iter()
        .filter_map(|&kind| {
            let properties = properties::child_properties(kind);
            (!properties.is_empty()).then(|| {
                (syntax_kind_of(kind), properties.iter().map(|property| (property.name, property.list)).collect())
            })
        })
        .collect()
}

/// The value of one of a node's TypeScript properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodePropertyValue {
    Node(NodeId),
    /// A `NodeArray`: its nodes, and its `pos`/`end` in UTF-16 units.
    List { nodes: Vec<NodeId>, pos: u32, end: u32 },
}

impl Program {
    /// The node's node-valued TypeScript properties (`expression`, `name`,
    /// `arguments`, ...), in the order `forEachChild` visits them. A property
    /// the node does not have is absent.
    pub fn node_properties(&self, node: NodeId) -> Vec<(&'static str, NodePropertyValue)> {
        let kind = self.tree(node).kind(node.node);
        properties::child_properties(kind)
            .iter()
            .filter_map(|property| Some((property.name, self.slot_value(node, kind, property.slot)?)))
            .collect()
    }

    /// One node-valued property of a node.
    pub fn node_property(&self, node: NodeId, name: &str) -> Option<NodePropertyValue> {
        let kind = self.tree(node).kind(node.node);
        properties::child_properties(kind)
            .iter()
            .filter(|property| property.name == name)
            .find_map(|property| self.slot_value(node, kind, property.slot))
    }

    /// Appends the node's children, as [`Self::node_properties`] lists them.
    pub(crate) fn extend_with_children(&self, node: NodeId, children: &mut Vec<NodeId>) {
        for_each_child(self.tree(node), node.node, |child| children.push(NodeId { node: child, ..node }));
    }

    fn slot_value(&self, node: NodeId, kind: Kind, slot: Slot) -> Option<NodePropertyValue> {
        let tree = self.tree(node);
        match properties::resolve(tree, node.node, slot) {
            properties::SlotValue::Node(Some(child)) => Some(NodePropertyValue::Node(NodeId { node: child, ..node })),
            properties::SlotValue::List(Some(list)) => {
                let nodes: Vec<NodeId> = list
                    .nodes
                    .iter()
                    .filter(|&&child| !is_reparsed_modifier(tree, kind, slot, child))
                    .map(|&child| NodeId { node: child, ..node })
                    .collect();
                // Only the synthesized `export` was there: TypeScript has no
                // modifiers at all.
                if nodes.is_empty() && !list.nodes.is_empty() {
                    return None;
                }
                let positions = self.positions(node.source_file());
                Some(NodePropertyValue::List { nodes, pos: positions.to_utf16(list.pos), end: positions.to_utf16(list.end) })
            }
            _ => None,
        }
    }

    /// The text an identifier, private identifier or literal holds (tsc's
    /// `Identifier.text`, `StringLiteral.text`, ...); `None` for other nodes.
    pub fn node_value_text(&self, node: NodeId) -> Option<&str> {
        let tree = self.tree(node);
        let kind = syntax_kind_of(tree.kind(node.node));
        let has_text = matches!(
            kind,
            SyntaxKind::Identifier
                | SyntaxKind::PrivateIdentifier
                | SyntaxKind::StringLiteral
                | SyntaxKind::NumericLiteral
                | SyntaxKind::BigIntLiteral
                | SyntaxKind::RegularExpressionLiteral
                | SyntaxKind::NoSubstitutionTemplateLiteral
                | SyntaxKind::TemplateHead
                | SyntaxKind::TemplateMiddle
                | SyntaxKind::TemplateTail
                | SyntaxKind::JsxText
        );
        has_text.then(|| tree.text(node.node))
    }

    /// The operator or keyword a node records as a `SyntaxKind`: a prefix or
    /// postfix operator's `operator`, a heritage clause's `token`, a type
    /// operator's `operator`, a meta-property's `keywordToken`.
    pub fn node_operator(&self, node: NodeId) -> Option<SyntaxKind> {
        let tree = self.tree(node);
        let data = tree.node(node.node);
        matches!(
            data.kind,
            Kind::PrefixUnaryExpression
                | Kind::PostfixUnaryExpression
                | Kind::HeritageClause
                | Kind::TypeOperator
                | Kind::MetaProperty
        )
        .then(|| syntax_kind_of(data.op))
    }

    /// `isTypeOnly` of an import or export clause or specifier.
    pub fn node_is_type_only(&self, node: NodeId) -> bool {
        self.tree(node).node(node.node).is_type_only
    }

    /// `ExportAssignment.isExportEquals`.
    pub fn node_is_export_equals(&self, node: NodeId) -> bool {
        self.tree(node).node(node.node).is_export_equals
    }

}

/// Every child of `node` that [`Program::node_properties`] lists, in order.
fn for_each_child(tree: &SyntaxTree, node: u32, mut visit: impl FnMut(u32)) {
    let kind = tree.kind(node);
    for property in properties::child_properties(kind) {
        match properties::resolve(tree, node, property.slot) {
            properties::SlotValue::Node(Some(child)) => visit(child),
            properties::SlotValue::List(Some(list)) => list
                .nodes
                .iter()
                .filter(|&&child| !is_reparsed_modifier(tree, kind, property.slot, child))
                .for_each(|&child| visit(child)),
            _ => {}
        }
    }
}

/// `Node.flags`, as TypeScript's `NodeFlags` values.
pub(crate) fn node_flags(tree: &SyntaxTree, node: u32) -> i32 {
    let kind = tree.kind(node);
    let mut flags = typescript_node_flags(kind, tree.flags(node), tree.node(node).op);
    // typescript-go marks `B` in `namespace A.B` with a synthesized
    // `export` modifier; TypeScript flags it `NestedNamespace`.
    if kind == Kind::ModuleDeclaration && tree.parent(node).is_some_and(|parent| tree.kind(parent) == Kind::ModuleDeclaration) {
        flags |= ts::NestedNamespace;
    }
    flags
}

/// A file's nodes as columns, indexed by node: a binding mirrors the tree
/// from it in one call. A node index no node holds has kind 0 and no
/// children; node `i`'s children are
/// `children[child_offsets[i]..child_offsets[i + 1]]`.
pub struct NodeTable {
    pub root: u32,
    pub kinds: Vec<u16>,
    pub pos: Vec<u32>,
    pub end: Vec<u32>,
    pub start: Vec<u32>,
    pub parent: Vec<i32>,
    pub flags: Vec<i32>,
    pub child_offsets: Vec<u32>,
    pub children: Vec<u32>,
}

impl NodeTable {
    pub(crate) fn new(tree: &SyntaxTree, text: &str, positions: &PositionMap) -> NodeTable {
        let count = tree.node_count();
        let mut table = NodeTable {
            root: tree.root(),
            kinds: vec![0; count],
            pos: vec![0; count],
            end: vec![0; count],
            start: vec![0; count],
            parent: vec![-1; count],
            flags: vec![0; count],
            child_offsets: vec![0; count + 1],
            children: Vec::new(),
        };
        for node in 0..count as u32 {
            let index = node as usize;
            table.child_offsets[index] = table.children.len() as u32;
            if !tree.is_attached(node) {
                continue;
            }
            table.kinds[index] = typescript_kind(tree, node).0;
            table.pos[index] = positions.to_utf16(tree.pos(node));
            table.end[index] = positions.to_utf16(tree.end(node));
            table.start[index] = positions.to_utf16(tree.start(text, node));
            table.flags[index] = node_flags(tree, node);
            if let Some(parent) = tree.parent(node) {
                table.parent[index] = parent as i32;
            }
            for_each_child(tree, node, |child| table.children.push(child));
        }
        table.child_offsets[count] = table.children.len() as u32;
        table
    }
}

/// typescript-go gives the body of `namespace A.B` a zero-width `export`
/// modifier of its own making; TypeScript gives it none.
pub(crate) fn is_reparsed_modifier(tree: &surge_ts_tsc_syntax::SyntaxTree, kind: Kind, slot: Slot, child: u32) -> bool {
    kind == Kind::ModuleDeclaration
        && slot == Slot::Modifiers
        && tree.flags(child).has(TscNodeFlags::Reparsed)
        && tree.pos(child) == tree.end(child)
}

/// The TypeScript kind of a node: its own, except that the hole in `[, a]`,
/// which this port builds as an empty `BindingElement`, is TypeScript's
/// `OmittedExpression`.
pub(crate) fn typescript_kind(tree: &surge_ts_tsc_syntax::SyntaxTree, id: u32) -> SyntaxKind {
    let data = tree.node(id);
    if data.kind == Kind::BindingElement && data.name.is_none() && data.initializer.is_none() && data.children.iter().all(Option::is_none) {
        return SyntaxKind::OmittedExpression;
    }
    syntax_kind_of(data.kind)
}
