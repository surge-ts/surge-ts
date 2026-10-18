//! Where a node sits, as tsc's utilities classify it (`isPartOfTypeNode`,
//! `isExpressionNode`, `isDeclarationName`, ...): the dispatch
//! `getTypeAtLocation` and `getSymbolAtLocation` take on.

use super::enums_generated::SyntaxKind;
use super::handles::NodeId;
use super::node::NodePropertyValue;
use super::program::Program;

impl Program {
    pub(crate) fn child(&self, node: NodeId, property: &str) -> Option<NodeId> {
        match self.node_property(node, property)? {
            NodePropertyValue::Node(child) => Some(child),
            NodePropertyValue::List { .. } => None,
        }
    }

    fn list_contains(&self, node: NodeId, property: &str, child: NodeId) -> bool {
        matches!(self.node_property(node, property), Some(NodePropertyValue::List { nodes, .. }) if nodes.contains(&child))
    }

    fn is_child(&self, node: NodeId, property: &str, child: NodeId) -> bool {
        self.child(node, property) == Some(child)
    }

    fn parent_kind(&self, node: NodeId) -> Option<(NodeId, SyntaxKind)> {
        let parent = self.node_parent(node)?;
        Some((parent, self.node_kind(parent)))
    }

    /// tsc's `isPartOfTypeNode`.
    pub(crate) fn is_part_of_type_node(&self, node: NodeId) -> bool {
        let kind = self.node_kind(node);
        if is_type_node_kind(kind) {
            return true;
        }
        match kind {
            SyntaxKind::AnyKeyword
            | SyntaxKind::UnknownKeyword
            | SyntaxKind::NumberKeyword
            | SyntaxKind::BigIntKeyword
            | SyntaxKind::StringKeyword
            | SyntaxKind::BooleanKeyword
            | SyntaxKind::SymbolKeyword
            | SyntaxKind::ObjectKeyword
            | SyntaxKind::UndefinedKeyword
            | SyntaxKind::NullKeyword
            | SyntaxKind::NeverKeyword => true,
            SyntaxKind::VoidKeyword => self.parent_kind(node).is_none_or(|(_, parent)| parent != SyntaxKind::VoidExpression),
            SyntaxKind::ExpressionWithTypeArguments => self.is_part_of_type_expression_with_type_arguments(node),
            SyntaxKind::TypeParameter => self
                .parent_kind(node)
                .is_some_and(|(_, parent)| matches!(parent, SyntaxKind::MappedType | SyntaxKind::InferType)),
            SyntaxKind::Identifier | SyntaxKind::QualifiedName | SyntaxKind::PropertyAccessExpression | SyntaxKind::ThisKeyword => {
                let mut node = node;
                if kind == SyntaxKind::Identifier
                    && let Some((parent, parent_kind)) = self.parent_kind(node)
                    && ((parent_kind == SyntaxKind::QualifiedName && self.is_child(parent, "right", node))
                        || (parent_kind == SyntaxKind::PropertyAccessExpression && self.is_child(parent, "name", node)))
                {
                    node = parent;
                }
                let Some((parent, parent_kind)) = self.parent_kind(node) else { return false };
                if parent_kind == SyntaxKind::TypeQuery {
                    return false;
                }
                if parent_kind == SyntaxKind::ImportType {
                    return !self.node_is_type_only(parent);
                }
                if is_type_node_kind(parent_kind) {
                    return true;
                }
                match parent_kind {
                    SyntaxKind::ExpressionWithTypeArguments => self.is_part_of_type_expression_with_type_arguments(parent),
                    SyntaxKind::TypeParameter => self.is_child(parent, "constraint", node),
                    SyntaxKind::PropertyDeclaration
                    | SyntaxKind::PropertySignature
                    | SyntaxKind::Parameter
                    | SyntaxKind::VariableDeclaration
                    | SyntaxKind::FunctionDeclaration
                    | SyntaxKind::FunctionExpression
                    | SyntaxKind::ArrowFunction
                    | SyntaxKind::Constructor
                    | SyntaxKind::MethodDeclaration
                    | SyntaxKind::MethodSignature
                    | SyntaxKind::GetAccessor
                    | SyntaxKind::SetAccessor
                    | SyntaxKind::CallSignature
                    | SyntaxKind::ConstructSignature
                    | SyntaxKind::IndexSignature
                    | SyntaxKind::TypeAssertionExpression => self.is_child(parent, "type", node),
                    SyntaxKind::CallExpression | SyntaxKind::NewExpression | SyntaxKind::TaggedTemplateExpression => {
                        self.list_contains(parent, "typeArguments", node)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// tsc's `isPartOfTypeExpressionWithTypeArguments`: a heritage clause's
    /// type, except a class's `extends`, which names a value.
    fn is_part_of_type_expression_with_type_arguments(&self, node: NodeId) -> bool {
        let Some((clause, clause_kind)) = self.parent_kind(node) else { return false };
        if clause_kind != SyntaxKind::HeritageClause {
            return false;
        }
        let in_class = self
            .parent_kind(clause)
            .is_some_and(|(_, owner)| matches!(owner, SyntaxKind::ClassDeclaration | SyntaxKind::ClassExpression));
        !(in_class && self.node_operator(clause) == Some(SyntaxKind::ExtendsKeyword))
    }

    /// tsc's `isExpressionNode`.
    pub(crate) fn is_expression_node(&self, node: NodeId) -> bool {
        let kind = self.node_kind(node);
        match kind {
            SyntaxKind::SuperKeyword
            | SyntaxKind::NullKeyword
            | SyntaxKind::TrueKeyword
            | SyntaxKind::FalseKeyword
            | SyntaxKind::RegularExpressionLiteral
            | SyntaxKind::ArrayLiteralExpression
            | SyntaxKind::ObjectLiteralExpression
            | SyntaxKind::PropertyAccessExpression
            | SyntaxKind::ElementAccessExpression
            | SyntaxKind::CallExpression
            | SyntaxKind::NewExpression
            | SyntaxKind::TaggedTemplateExpression
            | SyntaxKind::AsExpression
            | SyntaxKind::TypeAssertionExpression
            | SyntaxKind::SatisfiesExpression
            | SyntaxKind::NonNullExpression
            | SyntaxKind::ParenthesizedExpression
            | SyntaxKind::FunctionExpression
            | SyntaxKind::ClassExpression
            | SyntaxKind::ArrowFunction
            | SyntaxKind::VoidExpression
            | SyntaxKind::DeleteExpression
            | SyntaxKind::TypeOfExpression
            | SyntaxKind::PrefixUnaryExpression
            | SyntaxKind::PostfixUnaryExpression
            | SyntaxKind::BinaryExpression
            | SyntaxKind::ConditionalExpression
            | SyntaxKind::SpreadElement
            | SyntaxKind::TemplateExpression
            | SyntaxKind::OmittedExpression
            | SyntaxKind::JsxElement
            | SyntaxKind::JsxSelfClosingElement
            | SyntaxKind::JsxFragment
            | SyntaxKind::YieldExpression
            | SyntaxKind::AwaitExpression
            | SyntaxKind::MetaProperty => true,
            SyntaxKind::ExpressionWithTypeArguments => {
                self.parent_kind(node).is_none_or(|(_, parent)| parent != SyntaxKind::HeritageClause)
            }
            SyntaxKind::QualifiedName => {
                let mut current = node;
                while let Some((parent, SyntaxKind::QualifiedName)) = self.parent_kind(current) {
                    current = parent;
                }
                self.parent_kind(current).is_some_and(|(_, parent)| parent == SyntaxKind::TypeQuery)
                    || self.is_jsx_tag_name(current)
            }
            SyntaxKind::PrivateIdentifier => self.parent_kind(node).is_some_and(|(parent, parent_kind)| {
                parent_kind == SyntaxKind::BinaryExpression
                    && self.is_child(parent, "left", node)
                    && self.child(parent, "operatorToken").is_some_and(|operator| self.node_kind(operator) == SyntaxKind::InKeyword)
            }),
            SyntaxKind::Identifier
                if self.parent_kind(node).is_some_and(|(_, parent)| parent == SyntaxKind::TypeQuery) || self.is_jsx_tag_name(node) =>
            {
                true
            }
            SyntaxKind::Identifier
            | SyntaxKind::NumericLiteral
            | SyntaxKind::BigIntLiteral
            | SyntaxKind::StringLiteral
            | SyntaxKind::NoSubstitutionTemplateLiteral
            | SyntaxKind::ThisKeyword => self.is_in_expression_context(node),
            _ => false,
        }
    }

    fn is_jsx_tag_name(&self, node: NodeId) -> bool {
        self.parent_kind(node).is_some_and(|(parent, parent_kind)| {
            matches!(
                parent_kind,
                SyntaxKind::JsxOpeningElement | SyntaxKind::JsxSelfClosingElement | SyntaxKind::JsxClosingElement
            ) && self.is_child(parent, "tagName", node)
        })
    }

    /// tsc's `isInExpressionContext`.
    fn is_in_expression_context(&self, node: NodeId) -> bool {
        let Some((parent, parent_kind)) = self.parent_kind(node) else { return false };
        match parent_kind {
            SyntaxKind::VariableDeclaration
            | SyntaxKind::Parameter
            | SyntaxKind::PropertyDeclaration
            | SyntaxKind::PropertySignature
            | SyntaxKind::EnumMember
            | SyntaxKind::PropertyAssignment
            | SyntaxKind::BindingElement => self.is_child(parent, "initializer", node),
            SyntaxKind::ExpressionStatement
            | SyntaxKind::IfStatement
            | SyntaxKind::DoStatement
            | SyntaxKind::WhileStatement
            | SyntaxKind::ReturnStatement
            | SyntaxKind::WithStatement
            | SyntaxKind::SwitchStatement
            | SyntaxKind::CaseClause
            | SyntaxKind::ThrowStatement => self.is_child(parent, "expression", node),
            SyntaxKind::ForStatement => {
                (self.is_child(parent, "initializer", node) && self.node_kind(node) != SyntaxKind::VariableDeclarationList)
                    || self.is_child(parent, "condition", node)
                    || self.is_child(parent, "incrementor", node)
            }
            SyntaxKind::ForInStatement | SyntaxKind::ForOfStatement => {
                (self.is_child(parent, "initializer", node) && self.node_kind(node) != SyntaxKind::VariableDeclarationList)
                    || self.is_child(parent, "expression", node)
            }
            SyntaxKind::TypeAssertionExpression | SyntaxKind::AsExpression | SyntaxKind::SatisfiesExpression => {
                self.is_child(parent, "expression", node)
            }
            SyntaxKind::TemplateSpan | SyntaxKind::ComputedPropertyName => self.is_child(parent, "expression", node),
            SyntaxKind::Decorator | SyntaxKind::JsxExpression | SyntaxKind::JsxSpreadAttribute | SyntaxKind::SpreadAssignment => true,
            SyntaxKind::ExpressionWithTypeArguments => {
                self.is_child(parent, "expression", node) && !self.is_part_of_type_node(parent)
            }
            SyntaxKind::ShorthandPropertyAssignment => self.is_child(parent, "objectAssignmentInitializer", node),
            _ => self.is_expression_node(parent),
        }
    }

    /// tsc's `isDeclarationName`.
    pub(crate) fn is_declaration_name(&self, node: NodeId) -> bool {
        let kind = self.node_kind(node);
        if matches!(kind, SyntaxKind::SourceFile | SyntaxKind::ObjectBindingPattern | SyntaxKind::ArrayBindingPattern) {
            return false;
        }
        self.parent_kind(node)
            .is_some_and(|(parent, parent_kind)| is_declaration_kind(parent_kind) && self.is_child(parent, "name", node))
    }

    /// tsc's `isDeclarationNameOrImportPropertyName`.
    pub(crate) fn is_declaration_name_or_import_property_name(&self, node: NodeId) -> bool {
        match self.parent_kind(node) {
            Some((parent, SyntaxKind::ImportSpecifier | SyntaxKind::ExportSpecifier)) => {
                matches!(self.node_kind(node), SyntaxKind::Identifier | SyntaxKind::StringLiteral)
                    && (self.is_child(parent, "name", node) || self.is_child(parent, "propertyName", node))
            }
            _ => self.is_declaration_name(node),
        }
    }

    /// tsc's `isTypeDeclaration`.
    pub(crate) fn is_type_declaration(&self, node: NodeId) -> bool {
        match self.node_kind(node) {
            SyntaxKind::TypeParameter
            | SyntaxKind::ClassDeclaration
            | SyntaxKind::InterfaceDeclaration
            | SyntaxKind::TypeAliasDeclaration
            | SyntaxKind::EnumDeclaration => true,
            SyntaxKind::ImportClause => self.node_is_type_only(node),
            SyntaxKind::ImportSpecifier => self
                .node_parent(node)
                .and_then(|named| self.node_parent(named))
                .is_some_and(|clause| self.node_is_type_only(clause)),
            SyntaxKind::ExportSpecifier => self
                .node_parent(node)
                .and_then(|named| self.node_parent(named))
                .is_some_and(|export| self.node_is_type_only(export)),
            _ => false,
        }
    }

    /// tsc's `isTypeDeclarationName`.
    pub(crate) fn is_type_declaration_name(&self, node: NodeId) -> bool {
        self.node_kind(node) == SyntaxKind::Identifier
            && self
                .node_parent(node)
                .is_some_and(|parent| self.is_type_declaration(parent) && self.is_child(parent, "name", node))
    }
}

pub(crate) fn is_type_node_kind(kind: SyntaxKind) -> bool {
    kind.0 >= SyntaxKind::FirstTypeNode.0 && kind.0 <= SyntaxKind::LastTypeNode.0
}

/// tsc's `isDeclarationKind`.
pub(crate) fn is_declaration_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ArrowFunction
            | SyntaxKind::BindingElement
            | SyntaxKind::ClassDeclaration
            | SyntaxKind::ClassExpression
            | SyntaxKind::ClassStaticBlockDeclaration
            | SyntaxKind::Constructor
            | SyntaxKind::EnumDeclaration
            | SyntaxKind::EnumMember
            | SyntaxKind::ExportSpecifier
            | SyntaxKind::FunctionDeclaration
            | SyntaxKind::FunctionExpression
            | SyntaxKind::GetAccessor
            | SyntaxKind::ImportClause
            | SyntaxKind::ImportEqualsDeclaration
            | SyntaxKind::ImportSpecifier
            | SyntaxKind::InterfaceDeclaration
            | SyntaxKind::JsxAttribute
            | SyntaxKind::MethodDeclaration
            | SyntaxKind::MethodSignature
            | SyntaxKind::ModuleDeclaration
            | SyntaxKind::NamespaceExportDeclaration
            | SyntaxKind::NamespaceImport
            | SyntaxKind::NamespaceExport
            | SyntaxKind::Parameter
            | SyntaxKind::PropertyAssignment
            | SyntaxKind::PropertyDeclaration
            | SyntaxKind::PropertySignature
            | SyntaxKind::SetAccessor
            | SyntaxKind::ShorthandPropertyAssignment
            | SyntaxKind::TypeAliasDeclaration
            | SyntaxKind::TypeParameter
            | SyntaxKind::VariableDeclaration
            | SyntaxKind::NamedTupleMember
    )
}
