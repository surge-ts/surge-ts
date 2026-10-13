//! What declaration emit reports about the types the checker infers for a
//! module's exports. A class expression, or a class declared inside a
//! function, has no name a declaration file could write its type by, so emit
//! writes the type out as a literal (`WriteClassExpressionAsTypeLiteral`) —
//! and a type literal cannot keep a member private or protected (TS4094,
//! `createTypeNodesFromResolvedType`).

use std::collections::BTreeSet;
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedFunctionDeclaration, TextSpan};
use surge_ts_types::fx::FxHashSet;
use surge_ts_types::{FunctionType, ObjectProperty, ObjectType, Type};

use crate::context::{CheckerContext, convert_span};

const MAX_DEPTH: usize = 12;
const WORK_BUDGET: usize = 20_000;

/// Whether declaration emit writes the declarations this file exports from its
/// top level.
pub(crate) fn emits_declarations(ctx: &CheckerContext) -> bool {
    ctx.options.declaration
        && ctx.namespace_member_prefix_stack.is_empty()
        && !surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && !surge_ts_syntax::is_declaration_file_name(&ctx.file_name)
}

/// Where emit reports on the return type it infers for an exported function:
/// its name, when the type is the body's to infer.
pub(crate) fn inferred_return_span(function: &ParsedFunctionDeclaration, ctx: &CheckerContext) -> Option<TextSpan> {
    let infers_return = function.return_type.is_none()
        && function.full_signature.is_none()
        && function.has_body
        && !function.is_declare
        && !function.is_generator
        && !is_overloaded(&function.name, ctx);
    if !(infers_return && emits_declarations(ctx)) {
        return None;
    }
    function.name_span
}

/// An implementation behind overload signatures, which emit writes instead.
fn is_overloaded(name: &str, ctx: &CheckerContext) -> bool {
    ctx.symbols.get(name).is_some_and(|symbol| match &symbol.ty {
        Type::Function(function) => function.overloads().is_some_and(|overloads| !overloads.is_empty()),
        _ => false,
    })
}

/// The type emit writes for an expression it serializes (`export default
/// <expression>`, an `extends` clause that is no entity name), reported at
/// `span`.
pub(crate) fn report_expression_class_literal_members(
    expression: &ParsedExpression,
    span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    if is_entity_name_expression(expression) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if let crate::infer::InferredExpression::Known(ty) = inferred {
        report_class_literal_members(std::slice::from_ref(&ty), span, ctx);
    }
}

fn is_entity_name_expression(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::Identifier { .. } => true,
        ParsedExpression::PropertyAccess { object, is_bracketed: false, .. } => is_entity_name_expression(object),
        _ => false,
    }
}

/// TS4094 at `span` for each private or protected member of a class emit
/// writes as a type literal while it writes `types`.
pub(crate) fn report_class_literal_members(types: &[Type], span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let names = {
        let mut walk = ClassLiteralWalk {
            ctx: &*ctx,
            names: BTreeSet::new(),
            expanded: FxHashSet::default(),
            work: 0,
        };
        for ty in types {
            walk.visit(ty, 0);
        }
        walk.names
    };
    for name in names {
        let diagnostic = Diagnostic::ts4094(name, ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// tsc's node builder over one inferred type, reduced to what decides TS4094:
/// a named type is written by its name (its type arguments with it), an
/// anonymous one by its members, and a class with no name to write is
/// expanded with every member it has, inherited ones included.
struct ClassLiteralWalk<'a> {
    ctx: &'a CheckerContext,
    names: BTreeSet<String>,
    expanded: FxHashSet<Arc<str>>,
    work: usize,
}

impl ClassLiteralWalk<'_> {
    fn visit(&mut self, ty: &Type, depth: usize) {
        self.work += 1;
        if depth > MAX_DEPTH || self.work > WORK_BUDGET {
            return;
        }
        let depth = depth + 1;
        match ty {
            Type::Union(union) => {
                for member in union.types() {
                    self.visit(member, depth);
                }
            }
            Type::Array(element) => self.visit(element, depth),
            Type::Tuple(elements) => {
                for element in elements {
                    self.visit(element, depth);
                }
            }
            Type::OpenTuple(tuple) => {
                for element in tuple.leading.iter().chain(std::iter::once(tuple.rest.as_ref())).chain(&tuple.trailing) {
                    self.visit(element, depth);
                }
            }
            Type::Function(function) => self.visit_signature(function, depth),
            Type::Reference(reference) if self.is_class_literal_instance(&reference.id) => {
                if let Type::Object(object) = reference.resolve() {
                    self.expand_instance(&reference.id, &object, depth);
                }
            }
            Type::Reference(reference) if reference.render_structurally => self.visit(&reference.resolve(), depth),
            Type::Reference(reference) => {
                for argument in reference.arguments.iter() {
                    self.visit(argument, depth);
                }
            }
            Type::Object(object) => self.visit_object(object, depth),
            _ => {}
        }
    }

    fn visit_signature(&mut self, function: &FunctionType, depth: usize) {
        let overloads = function.overloads().unwrap_or_default();
        for signature in std::iter::once(function).chain(overloads) {
            for parameter in signature.parameters() {
                self.visit(parameter, depth);
            }
            self.visit(signature.return_type(), depth);
        }
    }

    fn visit_object(&mut self, object: &ObjectType, depth: usize) {
        if let Some(signature) = object.construct_signature()
            && object.alias_name.as_deref().is_some_and(|name| name.starts_with("typeof "))
        {
            // `typeof C` is written by name unless C is a class with none.
            if self.instance_id(signature.return_type()).is_some_and(|id| self.is_class_literal_instance(&id)) {
                for (name, property) in object.properties.iter() {
                    if name.as_ref() == "prototype" {
                        continue;
                    }
                    self.note(name, property);
                    self.visit(&property.ty, depth);
                }
                self.visit_signature(signature, depth);
                for operand in object.intersection_operands.iter().flat_map(|operands| operands.iter()) {
                    self.visit(operand, depth);
                }
            }
            return;
        }
        if let Some(id) = object.alias_id.as_deref() {
            if self.is_class_literal_instance(id) {
                self.expand_instance(id, object, depth);
            }
            return;
        }
        if object.alias_name.is_some() {
            return;
        }
        for property in object.properties.values() {
            self.visit(&property.ty, depth);
        }
        for signature in object.call_signature().into_iter().chain(object.construct_signature()) {
            self.visit_signature(signature, depth);
        }
        for index in object.string_index_type.iter().chain(&object.number_index_type) {
            self.visit(index, depth);
        }
        for operand in object.intersection_operands.iter().flat_map(|operands| operands.iter()) {
            self.visit(operand, depth);
        }
    }

    fn expand_instance(&mut self, id: &str, object: &ObjectType, depth: usize) {
        if !self.expanded.insert(Arc::from(id)) {
            return;
        }
        for (name, property) in object.properties.iter() {
            self.note(name, property);
            self.visit(&property.ty, depth);
        }
    }

    fn note(&mut self, name: &str, property: &ObjectProperty) {
        if property.restriction.is_some() || name.starts_with('#') {
            self.names.insert(name.to_string());
        }
    }

    fn instance_id(&self, instance: &Type) -> Option<Arc<str>> {
        match instance {
            Type::Object(object) => object.alias_id.clone(),
            Type::Reference(reference) => Some(reference.id.clone()),
            _ => None,
        }
    }

    /// A class declaration emit cannot name: a class expression (bound by a
    /// `const` of this file, or anywhere in a body), or a class a function
    /// body declares. Body-local declarations are keyed `name@offset`.
    fn is_class_literal_instance(&self, id: &str) -> bool {
        let Some((file, name)) = id.split_once('\0') else {
            return false;
        };
        name.contains('@')
            || name == "(Anonymous class)"
            || (file == self.ctx.file_name.as_str() && self.ctx.file_const_class_names.contains(name))
    }
}

/// TS4023 at `span` for each late-bound member key (`[SYMBOL]`, `[NS.sym]`)
/// emit writes into an anonymous type of the exported variable `name` whose
/// leading name this file does not bind: a computed property name is an
/// expression, which no import type can stand in for.
pub(crate) fn report_unnameable_late_bound_keys(ty: &Type, name: &str, span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let mut heads = BTreeSet::new();
    let mut work = 0;
    collect_late_bound_key_heads(ty, 0, &mut work, &mut heads);
    for head in heads {
        if ctx.symbols.get(&head).is_some()
            || ctx.module_value_fallback.as_ref().is_some_and(|table| table.get(&head).is_some())
        {
            continue;
        }
        let diagnostic = Diagnostic::ts4023(name, &head, "\"\"", ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// The names leading the late-bound keys of the anonymous types emit writes
/// out member by member; a named type is written by its name alone.
fn collect_late_bound_key_heads(ty: &Type, depth: usize, work: &mut usize, heads: &mut BTreeSet<String>) {
    *work += 1;
    if depth > MAX_DEPTH || *work > WORK_BUDGET {
        return;
    }
    let depth = depth + 1;
    match ty {
        Type::Union(union) => {
            for member in union.types() {
                collect_late_bound_key_heads(member, depth, work, heads);
            }
        }
        Type::Array(element) => collect_late_bound_key_heads(element, depth, work, heads),
        Type::Tuple(elements) => {
            for element in elements {
                collect_late_bound_key_heads(element, depth, work, heads);
            }
        }
        Type::Reference(reference) => {
            for argument in reference.arguments.iter() {
                collect_late_bound_key_heads(argument, depth, work, heads);
            }
        }
        Type::Object(object) if object.alias_name.is_none() && object.alias_id.is_none() => {
            for (key, property) in object.properties.iter() {
                if let Some(inner) = key.strip_prefix('[').and_then(|key| key.strip_suffix(']'))
                    && let Some(head) = inner.split('.').next()
                    && !head.is_empty()
                    && head.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
                {
                    heads.insert(head.to_string());
                }
                collect_late_bound_key_heads(&property.ty, depth, work, heads);
            }
        }
        _ => {}
    }
}
