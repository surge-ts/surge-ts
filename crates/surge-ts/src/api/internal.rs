//! Checker queries TypeScript keeps internal (`getUnionType`,
//! `isArrayLikeType`, a declaration's `symbol`, …), answered from surge's
//! types for tools built on TypeScript's compiler API that call them.

use std::sync::Arc;

use surge_ts_types::{NumberLiteralType, ResolveReference, Type, TypeReference};

use super::checker::TypeChecker;
use super::enums_generated::{SyntaxKind, symbol_flags as ts_sf, type_flags as tf};
use super::handles::{NodeId, SymbolId, TypeId};
use super::symbols::{Meaning, SymbolData, SymbolKey};

/// The structure a `readonly T[]` reference stands for.
struct ReadonlyShape(Type);

impl ResolveReference for ReadonlyShape {
    fn resolve(&self) -> Type {
        self.0.clone()
    }
}

fn readonly_array(element: Type) -> Type {
    let shape = Type::Array(Box::new(element));
    Type::Reference(TypeReference::new(
        surge_ts_types::READONLY_REFERENCE_ID,
        format!("readonly {}", shape.name()),
        vec![shape.clone()],
        Arc::new(ReadonlyShape(shape)),
    ))
}

/// tsc's `TupleType` shape: each element's `ElementFlags`, and whether the
/// tuple is `readonly`.
pub struct TupleShape {
    pub element_flags: Vec<i32>,
    pub readonly: bool,
    /// Each element's label (`[x: number]`), where the tuple was written with them.
    pub labels: Option<Vec<Option<String>>>,
}

/// `ElementFlags.Required` / `Optional` / `Rest`.
const ELEMENT_REQUIRED: i32 = 1;
const ELEMENT_OPTIONAL: i32 = 2;
const ELEMENT_REST: i32 = 4;

impl TypeChecker {
    /// A template literal type's `texts` and `types`: the text around each
    /// placeholder and the placeholders' types.
    pub fn template_literal_parts(&self, ty: TypeId) -> Option<(Vec<String>, Vec<TypeId>)> {
        let value = self.ty(ty);
        let (texts, types) = surge_ts_types::template_literal_parts(&value)?;
        let texts = texts.into_iter().map(str::to_string).collect();
        let types = types.into_iter().map(|member| self.intern(member.clone())).collect();
        Some((texts, types))
    }

    /// A tuple type's `target` shape (`elementFlags`, `readonly`); `None`
    /// for a type that is no tuple.
    pub fn tuple_shape(&self, ty: TypeId) -> Option<TupleShape> {
        let mut value = self.ty(ty);
        let mut readonly = false;
        for _ in 0..16 {
            if let Some((elements, min_length)) = surge_ts_types::fixed_tuple_parts(&value) {
                let element_flags = (0..elements.len())
                    .map(|index| if index < min_length { ELEMENT_REQUIRED } else { ELEMENT_OPTIONAL })
                    .collect();
                let labels = surge_ts_types::fixed_tuple_labels(&value)
                    .map(|labels| labels.into_iter().map(|label| label.map(|label| label.to_string())).collect());
                return Some(TupleShape { element_flags, readonly, labels });
            }
            value = match &value {
                Type::OpenTuple(open) => {
                    let leading_min = surge_ts_types::tuple_min_length(&open.leading);
                    let mut element_flags: Vec<i32> = (0..open.leading.len())
                        .map(|index| if index < leading_min { ELEMENT_REQUIRED } else { ELEMENT_OPTIONAL })
                        .collect();
                    element_flags.push(ELEMENT_REST);
                    element_flags.extend(std::iter::repeat_n(ELEMENT_REQUIRED, open.trailing.len()));
                    let labels = open
                        .labels
                        .as_ref()
                        .map(|labels| labels.iter().map(|label| label.as_ref().map(|label| label.to_string())).collect());
                    return Some(TupleShape { element_flags, readonly, labels });
                }
                Type::Reference(reference) if reference.is_readonly_array() => {
                    readonly = true;
                    reference.arguments.first()?.clone()
                }
                Type::Reference(reference) => reference.resolve(),
                _ => return None,
            };
        }
        None
    }

    /// The value symbols tsc creates without a declaration: `undefined`, the
    /// `arguments` of a function, and `globalThis`. `None` for a name that
    /// is none of them where it is read.
    pub(super) fn builtin_value_symbol(&self, location: NodeId, name: &str) -> Option<SymbolId> {
        // tsc's `argumentsSymbol` has no declared type (`getTypeOfSymbol` is
        // `any`); reading it as an expression is what gives it `IArguments`.
        let (flags, ty) = match name {
            "undefined" => (ts_sf::Property, self.undefined_type()),
            "arguments" if self.reads_function_arguments(location) => (ts_sf::Property, self.any_type()),
            "globalThis" => (ts_sf::ValueModule | ts_sf::NamespaceModule, self.global_this_type(location)),
            _ => return None,
        };
        Some(self.builtin_symbol(name, flags, ty))
    }

    fn builtin_symbol(&self, name: &str, flags: i32, ty: TypeId) -> SymbolId {
        self.symbol_id(SymbolKey::Global(Arc::from(format!("\0{name}"))), || SymbolData::Transient {
            name: Arc::from(name),
            flags,
            ty,
            parent: None,
            declarations: Vec::new(),
        })
    }

    /// tsc's `arguments` lookup: the name reaches a function's own scope
    /// through arrow functions only.
    fn reads_function_arguments(&self, location: NodeId) -> bool {
        let program = self.program();
        let mut current = program.node_parent(location);
        while let Some(node) = current {
            match program.node_kind(node) {
                SyntaxKind::FunctionDeclaration
                | SyntaxKind::FunctionExpression
                | SyntaxKind::MethodDeclaration
                | SyntaxKind::Constructor
                | SyntaxKind::GetAccessor
                | SyntaxKind::SetAccessor => return true,
                SyntaxKind::SourceFile => return false,
                _ => current = program.node_parent(node),
            }
        }
        false
    }

    /// `typeof globalThis`, resolved where `location` is written.
    fn global_this_type(&self, location: NodeId) -> TypeId {
        self.resolved_type_text(location, "typeof globalThis").unwrap_or_else(|| self.error_type())
    }

    /// Whether `symbol` is tsc's `globalThis`, whose exports are the globals.
    pub(super) fn is_global_this_symbol(&self, symbol: SymbolId) -> bool {
        matches!(self.symbol_data(symbol), SymbolData::Transient { name, parent: None, ref declarations, .. }
            if &*name == "globalThis" && declarations.is_empty())
    }

    /// `isUndefinedSymbol`.
    pub fn is_undefined_symbol(&self, symbol: SymbolId) -> bool {
        symbol == self.builtin_symbol("undefined", ts_sf::Property, self.undefined_type())
    }

    /// `isArgumentsSymbol`.
    pub fn is_arguments_symbol(&self, symbol: SymbolId) -> bool {
        symbol == self.builtin_symbol("arguments", ts_sf::Property, self.any_type())
    }

    /// `resolveName(name, location, meaning)`: the symbol `name` means at
    /// `location` (the global scope without one), of a meaning `meaning`
    /// (`SymbolFlags`) allows. An alias matches every meaning, as in tsc.
    pub fn resolve_name_at(&self, location: Option<NodeId>, name: &str, meaning: i32) -> Option<SymbolId> {
        for (bits, kind) in [(ts_sf::Value, Meaning::Value), (ts_sf::Type, Meaning::Type), (ts_sf::Namespace, Meaning::Namespace)] {
            if meaning & bits == 0 {
                continue;
            }
            let found = match location {
                Some(location) => self.resolve_name(location, name, kind),
                None => self.global_symbol(name, kind),
            }
            .or_else(|| match (kind, location) {
                (Meaning::Value, Some(location)) => self.builtin_value_symbol(location, name),
                (Meaning::Value, None) if name != "arguments" => self.builtin_value_symbol_at_top(name),
                _ => None,
            });
            if let Some(symbol) = found
                && self.symbol_flags(symbol) & (meaning | ts_sf::Alias) != 0
            {
                return Some(symbol);
            }
        }
        None
    }

    fn builtin_value_symbol_at_top(&self, name: &str) -> Option<SymbolId> {
        match name {
            "undefined" => Some(self.builtin_symbol(name, ts_sf::Property, self.undefined_type())),
            "globalThis" => {
                // Any file's scope sees the same global scope.
                let file = *self.program().source_files().last()?;
                self.builtin_value_symbol(self.program().root(file), name)
            }
            _ => None,
        }
    }

    /// `getShorthandAssignmentValueSymbol`: the value `{ x }` reads for `x`.
    pub fn shorthand_assignment_value_symbol(&self, assignment: NodeId) -> Option<SymbolId> {
        let program = self.program();
        if program.node_kind(assignment) != SyntaxKind::ShorthandPropertyAssignment {
            return None;
        }
        let name = match program.node_property(assignment, "name")? {
            super::node::NodePropertyValue::Node(name) => name,
            _ => return None,
        };
        let text = program.node_value_text(name)?.to_string();
        self.resolve_name(name, &text, Meaning::Value).or_else(|| self.builtin_value_symbol(name, &text))
    }

    /// A declaration's `node.symbol`: the symbol the binder gave it. A source
    /// file has its module's (a script has none).
    pub fn declaration_symbol(&self, node: NodeId) -> Option<SymbolId> {
        match self.program().node_kind(node) {
            SyntaxKind::SourceFile => self.symbol_at_location(node),
            // tsc's binder gives a function or constructor type the type
            // literal symbol `__type` its `__call`/`__new` signature is a
            // member of, and that is the node's symbol.
            SyntaxKind::FunctionType | SyntaxKind::ConstructorType => {
                let ty = self.type_at_location(node);
                Some(self.symbol_id(SymbolKey::Global(Arc::from(format!("\0__type\0{}\0{}", node.file, node.node))), || SymbolData::Transient {
                    name: Arc::from("__type"),
                    flags: ts_sf::TypeLiteral,
                    ty,
                    parent: None,
                    declarations: vec![node],
                }))
            }
            _ => self.declared_symbol(node),
        }
    }

    /// tsc's symbol of a function-like declaration's type: the declaration's
    /// own (`__function` for a function expression), for the type
    /// `getTypeAtLocation` or `getTypeOfSymbol` gave it.
    pub(super) fn function_type_symbol(&self, ty: TypeId) -> Option<SymbolId> {
        let is_function = match self.ty(ty) {
            Type::Function(_) => true,
            Type::Object(object) => object.call_signature().is_some() && object.properties.is_empty(),
            _ => false,
        };
        if !is_function {
            return None;
        }
        let function_like = ts_sf::Function | ts_sf::Method | ts_sf::Accessor;
        if let Some(origin) = self.type_origin(ty) {
            if self.symbol_flags(origin) & function_like != 0 {
                return Some(origin);
            }
            // A property or variable annotated with a function type has the
            // type literal's (`__type`), whatever initializes it.
            if let Some(annotation) = self.function_type_annotation(origin) {
                return self.declaration_symbol(annotation);
            }
            // One initialized with a function expression has the expression's
            // type, whose symbol is the expression's.
            if let Some(function) = self.function_initializer(origin) {
                return self.declared_symbol(function);
            }
        }
        // A function type written through an alias (`type Method = (this:
        // defined) => number`) has the aliased type literal's symbol, however
        // it was reached (`make()`, `callbacks[0]`).
        if let Type::Function(function) = self.ty(ty)
            && let Some(alias) = function.alias_name()
            && let Some(scope) = self.type_scope(ty)
            && let Some(symbol) = self.resolve_name(scope, alias.split('<').next().unwrap_or(alias), Meaning::Type)
            && let Some(declaration) = self
                .symbol_declarations(symbol)
                .into_iter()
                .find(|declaration| self.program().node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)
            && let Some(aliased) = self.declaration_annotation(declaration)
            && matches!(self.program().node_kind(aliased), SyntaxKind::FunctionType | SyntaxKind::ConstructorType)
        {
            return self.declaration_symbol(aliased);
        }
        // A union's member is reached where the union was.
        let location = self.type_location(ty).or_else(|| self.type_scope(ty))?;
        match self.program().node_kind(location) {
            SyntaxKind::FunctionDeclaration
            | SyntaxKind::FunctionExpression
            | SyntaxKind::ArrowFunction
            | SyntaxKind::MethodDeclaration
            | SyntaxKind::MethodSignature
            | SyntaxKind::GetAccessor
            | SyntaxKind::SetAccessor => self.declared_symbol(location),
            // A read of a function (`withThis`, `object.method`) has the type
            // of what it names, whose symbol that is.
            SyntaxKind::Identifier | SyntaxKind::PropertyAccessExpression => {
                let named = self.symbol_at_location(location)?;
                let named = self.aliased_symbol(named).unwrap_or(named);
                if self.symbol_flags(named) & function_like != 0 {
                    Some(named)
                } else if let Some(function) = self.function_initializer(named) {
                    self.declared_symbol(function)
                } else if let Some(annotation) = self.function_type_annotation(named) {
                    self.declaration_symbol(annotation)
                } else {
                    self.annotated_query_function(named)
                }
            }
            // A call of a function returning one it names (`getCallback()`
            // returning `callback`) has that function's type.
            SyntaxKind::CallExpression => {
                let callee = self.symbol_at_location(self.node_child(location, "expression")?)?;
                let callee = self.aliased_symbol(callee).unwrap_or(callee);
                let declaration = self.symbol_value_declaration(callee)?;
                if self.program().node_kind(declaration) != SyntaxKind::FunctionDeclaration
                    || self.node_child(declaration, "type").is_some()
                {
                    return None;
                }
                let returned = self.lone_return_expression(declaration)?;
                if self.program().node_kind(returned) != SyntaxKind::Identifier {
                    return None;
                }
                let named = self.symbol_at_location(returned)?;
                let named = self.aliased_symbol(named).unwrap_or(named);
                (self.symbol_flags(named) & function_like != 0).then_some(named)
            }
            _ => None,
        }
    }

    /// The function a declaration's annotation names by `typeof` (`const
    /// optional: typeof callback | undefined`), whose type is that function's.
    fn annotated_query_function(&self, symbol: SymbolId) -> Option<SymbolId> {
        let declaration = self.symbol_value_declaration(symbol)?;
        // Unannotated, it has what an invoked function expression it is
        // initialized with writes it returns (`((): typeof f => …)()`).
        let annotation = self.node_child(declaration, "type").or_else(|| {
            let initializer = self.node_child(declaration, "initializer")?;
            if self.program().node_kind(initializer) != SyntaxKind::CallExpression {
                return None;
            }
            let mut callee = self.node_child(initializer, "expression")?;
            while self.program().node_kind(callee) == SyntaxKind::ParenthesizedExpression {
                callee = self.node_child(callee, "expression")?;
            }
            matches!(self.program().node_kind(callee), SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression)
                .then(|| self.node_child(callee, "type"))
                .flatten()
        })?;
        let members = match self.program().node_kind(annotation) {
            SyntaxKind::UnionType => self.list_property(annotation, "types"),
            _ => vec![annotation],
        };
        let function_like = ts_sf::Function | ts_sf::Method;
        members.into_iter().find_map(|member| {
            if self.program().node_kind(member) != SyntaxKind::TypeQuery {
                return None;
            }
            let named = self.symbol_at_location(self.node_child(member, "exprName")?)?;
            let named = self.aliased_symbol(named).unwrap_or(named);
            (self.symbol_flags(named) & function_like != 0).then_some(named)
        })
    }

    /// The function type a property, variable or parameter declaration (the
    /// symbol's value declaration) is annotated with.
    pub(super) fn function_type_annotation(&self, symbol: SymbolId) -> Option<NodeId> {
        let declaration = self.symbol_value_declaration(symbol)?;
        // A property whose value names another value (`{ implementation }`,
        // `{ run: implementation }`) has that value's type.
        let named = match self.program().node_kind(declaration) {
            SyntaxKind::ShorthandPropertyAssignment => self.shorthand_assignment_value_symbol(declaration),
            SyntaxKind::PropertyAssignment => self
                .declaration_initializer(declaration)
                .filter(|initializer| self.program().node_kind(*initializer) == SyntaxKind::Identifier)
                .and_then(|initializer| self.symbol_at_location(initializer)),
            _ => None,
        };
        if let Some(named) = named.filter(|named| *named != symbol) {
            return self.function_type_annotation(named);
        }
        if !matches!(
            self.program().node_kind(declaration),
            SyntaxKind::PropertySignature | SyntaxKind::PropertyDeclaration | SyntaxKind::VariableDeclaration | SyntaxKind::Parameter
        ) {
            return None;
        }
        let annotation = self.declaration_annotation(declaration)?;
        self.function_type_node(annotation)
    }

    /// A function or constructor type node, or the one a type alias the node
    /// names stands for (`type Method = (this: defined) => number`).
    fn function_type_node(&self, node: NodeId) -> Option<NodeId> {
        match self.program().node_kind(node) {
            SyntaxKind::FunctionType | SyntaxKind::ConstructorType => Some(node),
            SyntaxKind::ParenthesizedType => self.function_type_node(self.declaration_annotation(node)?),
            SyntaxKind::TypeReference => {
                let name = self.type_reference_name(node)?;
                let mut alias = self.symbol_at_location(name)?;
                if self.symbol_flags(alias) & ts_sf::Alias != 0 {
                    alias = self.aliased_symbol(alias)?;
                }
                let declaration = self
                    .symbol_declarations(alias)
                    .into_iter()
                    .find(|declaration| self.program().node_kind(*declaration) == SyntaxKind::TypeAliasDeclaration)?;
                let aliased = self.declaration_annotation(declaration)?;
                matches!(self.program().node_kind(aliased), SyntaxKind::FunctionType | SyntaxKind::ConstructorType).then_some(aliased)
            }
            _ => None,
        }
    }

    /// The function or arrow expression a property, variable or parameter
    /// declaration (the symbol's value declaration) is initialized with.
    pub(super) fn function_initializer(&self, symbol: SymbolId) -> Option<NodeId> {
        let declaration = self.symbol_value_declaration(symbol)?;
        if !matches!(
            self.program().node_kind(declaration),
            SyntaxKind::PropertyAssignment | SyntaxKind::VariableDeclaration | SyntaxKind::PropertyDeclaration
        ) {
            return None;
        }
        let mut initializer = self.declaration_initializer(declaration)?;
        while self.program().node_kind(initializer) == SyntaxKind::ParenthesizedExpression {
            initializer = self.declaration_expression(initializer)?;
        }
        matches!(self.program().node_kind(initializer), SyntaxKind::ArrowFunction | SyntaxKind::FunctionExpression).then_some(initializer)
    }

    /// The binder's name of a destructured parameter: tsc's `__0`, `__1`, …
    /// by position, where typescript-go's port says `__missing`.
    pub(super) fn binding_parameter_name(&self, symbol: SymbolId) -> Option<String> {
        let program = self.program();
        let parameter = *self.symbol_declarations(symbol).first()?;
        if program.node_kind(parameter) != SyntaxKind::Parameter {
            return None;
        }
        let name = match program.node_property(parameter, "name")? {
            super::node::NodePropertyValue::Node(name) => name,
            _ => return None,
        };
        if !matches!(program.node_kind(name), SyntaxKind::ObjectBindingPattern | SyntaxKind::ArrayBindingPattern) {
            return None;
        }
        let owner = program.node_parent(parameter)?;
        let Some(super::node::NodePropertyValue::List { nodes, .. }) = program.node_property(owner, "parameters") else { return None };
        let index = nodes.iter().position(|node| *node == parameter)?;
        Some(format!("__{index}"))
    }

    /// `getImmediateAliasedSymbol`: one step of an alias chain.
    pub fn immediate_aliased_symbol(&self, symbol: SymbolId) -> Option<SymbolId> {
        if self.symbol_flags(symbol) & ts_sf::Alias == 0 {
            return None;
        }
        self.alias_target(symbol).filter(|target| *target != symbol)
    }

    /// `getContextualType`: the type the checker checked the expression
    /// against. A parenthesized expression's is the expression's inside, as
    /// surge checks that one.
    pub fn contextual_type(&self, node: NodeId) -> Option<TypeId> {
        let program = self.program();
        let mut candidates = vec![node];
        let mut inner = node;
        while program.node_kind(inner) == SyntaxKind::ParenthesizedExpression {
            match program.node_property(inner, "expression") {
                Some(super::node::NodePropertyValue::Node(expression)) => inner = expression,
                _ => break,
            }
            candidates.push(inner);
        }
        let mut outer = node;
        while let Some(parent) = program.node_parent(outer)
            && program.node_kind(parent) == SyntaxKind::ParenthesizedExpression
        {
            outer = parent;
            candidates.push(outer);
        }
        let ty = candidates.into_iter().find_map(|candidate| self.recorded_contextual_type(candidate))?;
        // Reached at the node, where the names it was written with resolve
        // (`Callable<void>`'s alias, whose type literal is its symbol); an
        // initializer's is its declaration's annotation, whose symbol it has.
        Some(self.contextual_type_handle(node, ty))
    }

    /// `getUnionType`.
    pub fn union_type(&self, types: &[TypeId]) -> TypeId {
        let members = types.iter().map(|ty| self.ty(*ty)).collect();
        self.intern(surge_ts_types::union_type(members))
    }

    /// `createArrayType`.
    pub fn array_type(&self, element: TypeId, readonly: bool) -> TypeId {
        let element = self.ty(element);
        self.intern(if readonly { readonly_array(element) } else { Type::Array(Box::new(element)) })
    }

    /// `getStringLiteralType`.
    pub fn string_literal_type(&self, value: &str) -> TypeId {
        self.intern(Type::StringLiteral(value.to_string()))
    }

    /// `getNumberLiteralType`, with the value as JavaScript prints it.
    pub fn number_literal_type(&self, value: &str) -> TypeId {
        self.intern(Type::NumberLiteral(NumberLiteralType { value: value.to_string() }))
    }

    /// `getTrueType` and `getFalseType`.
    pub fn boolean_literal_type(&self, value: bool) -> TypeId {
        self.intern(Type::BooleanLiteral(value))
    }

    /// `isArrayLikeType`: an array or tuple, or a type other than `null` or
    /// `undefined` assignable to `readonly any[]`.
    pub fn is_array_like_type(&self, ty: TypeId) -> bool {
        if self.is_array_type(ty) || self.is_tuple_type(ty) {
            return true;
        }
        let flags = self.type_flags(ty);
        if flags & (tf::Undefined | tf::Null) != 0 {
            return false;
        }
        // Assignability to `readonly any[]`: a union's when every member's,
        // an intersection's when one operand's (`T[] & { _nominal: … }`).
        if flags & tf::Union != 0 {
            return self.type_constituents(ty).into_iter().all(|member| self.is_array_like_type(member));
        }
        if flags & tf::Intersection != 0 && self.type_constituents(ty).into_iter().any(|operand| self.is_array_like_type(operand)) {
            return true;
        }
        let target = self.intern(readonly_array(Type::Any));
        self.is_type_assignable_to(ty, target)
    }
}
