//! Evolving arrays: tsc's control-flow typing of a `[]`-initialized local under
//! `noImplicitAny` (`autoArrayType`, `getTypeAtFlowArrayMutation`). `push`,
//! `unshift` and `x[n] = v` add element types; a read with none yet is an
//! implicit `any[]`, reported by `check_auto_array_read`.
//!
//! Element types only ever grow along the flow, and at a join tsc unions the
//! element types of every incoming evolving array, so recording each mutation
//! as the walk reaches it gives the joined state without tracking branches.

use surge_ts_syntax::{
    ParsedCallArgument, ParsedExpression, ParsedFunctionBodyStatement, ParsedVariableDeclaration,
    ParsedVariableKind,
};
use surge_ts_types::{Type, union_type};

use crate::checks::expr::widen_type;
use crate::context::CheckerContext;
use crate::infer::{InferredExpression, infer_expression};
use crate::symbols::{AutoArrayBinding, ScopeStack, SymbolInfo, SymbolTable};

use super::control_flow::for_of_element_type;

/// One mutation of an evolving array this function declared: the values it
/// adds, each flagged when it is spread.
pub(crate) struct ArrayMutation {
    name: String,
    values: Vec<(ParsedExpression, bool)>,
}

/// How tsc types a local declaration by control flow, if it does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoDeclaration {
    /// `x = []`: an evolving array (`autoArrayType`).
    Array,
    /// `let x;`, `let x = undefined`/`null`: whatever is assigned (`autoType`).
    Value,
}

/// tsc's `getWidenedTypeForVariableLikeDeclaration` for a local it types by
/// control flow: under `noImplicitAny`, an un-annotated, non-ambient,
/// non-destructured variable initialized with `[]`, or a non-`const` one with
/// no initializer or an `undefined`/`null` one.
pub(crate) fn auto_declaration(
    variable: &ParsedVariableDeclaration,
    ctx: &CheckerContext,
) -> Option<AutoDeclaration> {
    if crate::checks::var::is_auto_array_candidate(variable, ctx) {
        return Some(AutoDeclaration::Array);
    }
    (ctx.options.no_implicit_any
        && variable.declared_type.is_none()
        && !variable.is_declare
        && !variable.from_binding_pattern
        && !variable.is_enum_object
        && !variable.has_definite_assertion
        && !matches!(variable.kind, ParsedVariableKind::Const)
        && matches!(
            &variable.initializer,
            None | Some(ParsedExpression::UndefinedLiteral | ParsedExpression::NullLiteral)
        ))
    .then_some(AutoDeclaration::Value)
}

/// Starts tracking a flow-typed declaration, and otherwise hides any enclosing
/// binding of the same name. An array starts as the element-less `any[]`; a
/// value starts as `undefined` (tsc's initial type for it), or `any` for a
/// `null` initializer surge has no type for. Either is declared `any`/`any[]`,
/// which later assignments are checked against.
pub(crate) fn declare_auto_binding(
    variable: &ParsedVariableDeclaration,
    auto: Option<AutoDeclaration>,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let Some(auto) = auto else {
        scopes.declare_auto_array(&variable.name, None);
        return;
    };
    ctx.auto_arrays_declared = true;
    let any_array = Type::Array(Box::new(Type::Any));
    let (declared, initial) = match auto {
        AutoDeclaration::Array => (any_array.clone(), any_array),
        AutoDeclaration::Value => {
            let initial = match variable.initializer {
                Some(ParsedExpression::NullLiteral) => Type::Any,
                _ => Type::Undefined,
            };
            (Type::Any, initial)
        }
    };
    if let Some(symbol) = scopes.resolve(&variable.name) {
        let symbol = SymbolInfo {
            ty: initial,
            kind: symbol.kind,
            function_signature: None,
        };
        scopes.insert_current_narrowed(variable.name.as_str(), symbol, declared);
    }
    let is_let = matches!(variable.kind, ParsedVariableKind::Let);
    let assignments = variable
        .name_span
        .filter(|_| is_let)
        .and_then(|span| ctx.let_assignment(span.start));
    scopes.declare_auto_array(
        &variable.name,
        Some(AutoArrayBinding {
            name_span: variable.name_span,
            is_const: matches!(variable.kind, ParsedVariableKind::Const),
            declared_array: auto == AutoDeclaration::Array,
            is_let,
            initialized: variable.initializer.is_some(),
            assignments,
            evolving: auto == AutoDeclaration::Array,
            elements: Type::Never,
            unsettled: false,
            pending_loops: 0,
            declared_only: false,
            module_level: false,
        }),
    );
}

/// The evolving-array state a mutation in this flow updates. A module-level
/// binding lives in the module table, which outlives the per-statement scope
/// stack; a mutation that crossed a function boundary is not this flow's and
/// updates nothing.
fn flow_binding_mut<'a>(
    name: &str,
    scopes: &'a mut ScopeStack,
    ctx: &'a mut CheckerContext,
) -> Option<&'a mut AutoArrayBinding> {
    match scopes.visible_symbols().auto_array(name) {
        Some((binding, false)) if binding.module_level => ctx.symbols.auto_array_mut(name),
        Some((_, false)) => scopes.auto_array_mut(name),
        _ => None,
    }
}

/// Whether any evolving array is visible here: one this scope declared, or a
/// module-level one still in the module table.
pub(crate) fn auto_arrays_visible(scopes: &ScopeStack, ctx: &CheckerContext) -> bool {
    ctx.auto_arrays_declared
        && (scopes.visible_symbols().has_auto_arrays() || ctx.symbols.has_auto_arrays())
}

/// An assignment to a flow-typed local (`getTypeAtFlowAssignment`): `x = []`
/// (re)starts an evolving array, and any other value is what the binding holds
/// from here, its literals widened — `any[]` for an array local given something
/// that is not an array, which the assignment check has already reported. The
/// new type is a narrowing of the branch it happens in. Returns whether the
/// assignment was handled here.
pub(crate) fn assign_evolving_array(
    name: &str,
    empty: bool,
    value_type: &InferredExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) -> bool {
    let Some((binding, false)) = scopes.visible_symbols().auto_array(name) else {
        return false;
    };
    let declared_array = binding.declared_array;
    let any_array = Type::Array(Box::new(Type::Any));
    let Some(binding) = flow_binding_mut(name, scopes, ctx) else {
        return false;
    };
    let ty = if empty {
        binding.evolving = true;
        binding.elements = Type::Never;
        binding.unsettled = false;
        any_array
    } else {
        binding.evolving = false;
        match value_type {
            InferredExpression::Known(ty)
                if !ty.is_unknown()
                    && (!declared_array || surge_ts_types::is_assignable_to(ty, &any_array)) =>
            {
                widen_type(ty)
            }
            _ if declared_array => any_array,
            _ => Type::Any,
        }
    };
    if let Some(symbol) = scopes.resolve(name) {
        let narrowed = SymbolInfo {
            ty,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        scopes.insert_current(name, narrowed);
    }
    true
}

/// The mutations `statement` itself makes to evolving arrays this function
/// declared — not those in its nested statements, which are collected when
/// they are checked, nor those in nested functions, which tsc's flow for this
/// function never sees.
/// The mutations a module-level statement performs. Module statements are
/// checked one at a time against the module table rather than through a body's
/// scope stack, so they collect and apply their own.
pub(crate) fn collect_module_mutations(
    statement: &surge_ts_syntax::ParsedStatement,
    symbols: &SymbolTable,
) -> Vec<ArrayMutation> {
    use surge_ts_syntax::ParsedStatement;
    let mut mutations = Vec::new();
    let expressions: Vec<&ParsedExpression> = match statement {
        ParsedStatement::Expression(expression) => vec![expression],
        ParsedStatement::Call(call) => {
            call.arguments.iter().map(|argument| &argument.expression).collect()
        }
        ParsedStatement::VariableDeclaration(variable) => variable.initializer.iter().collect(),
        ParsedStatement::Assignment(assignment) => vec![&assignment.value],
        ParsedStatement::MemberAssignment(assignment) => {
            if let Some(name) = element_write_receiver(&assignment.target, symbols) {
                mutations.push(ArrayMutation {
                    name: name.to_string(),
                    values: vec![(assignment.value.clone(), false)],
                });
            }
            vec![&assignment.value]
        }
        _ => Vec::new(),
    };
    for expression in expressions {
        collect_expression_mutations(expression, symbols, &mut mutations);
    }
    mutations
}

/// A module-level assignment to an evolving array, as
/// [`assign_evolving_array`] handles a local one: `xs = []` restarts it and any
/// other value is what it holds from here. Returns whether it was handled.
pub(crate) fn assign_module_evolving_array(
    name: &str,
    value: &ParsedExpression,
    value_type: &InferredExpression,
    ctx: &mut CheckerContext,
) -> bool {
    let Some((binding, _)) = ctx.symbols.auto_array(name) else {
        return false;
    };
    if !binding.module_level {
        return false;
    }
    let declared_array = binding.declared_array;
    let empty = matches!(
        value,
        ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty()
    );
    let any_array = Type::Array(Box::new(Type::Any));
    let Some(binding) = ctx.symbols.auto_array_mut(name) else {
        return false;
    };
    let ty = if empty {
        binding.evolving = true;
        binding.elements = Type::Never;
        binding.unsettled = false;
        any_array
    } else {
        binding.evolving = false;
        match value_type {
            InferredExpression::Known(ty) if !ty.is_unknown() => widen_type(ty),
            _ if declared_array => any_array,
            _ => return false,
        }
    };
    set_module_binding_type(name, ty, ctx);
    true
}

/// Applies module-level mutations to the module table, where the binding lives.
pub(crate) fn apply_module_mutations(mutations: Vec<ArrayMutation>, ctx: &mut CheckerContext) {
    for mutation in mutations {
        // The table is `Arc`-shared by the clone, so this costs no deep copy.
        let symbols = ctx.symbols.clone();
        let element_types = mutation_element_types(&mutation, &symbols, ctx);
        let Some(binding) = ctx.symbols.auto_array_mut(&mutation.name) else {
            continue;
        };
        let Some(ty) = grow_elements(binding, element_types) else {
            continue;
        };
        set_module_binding_type(&mutation.name, ty, ctx);
    }
}

fn set_module_binding_type(name: &str, ty: Type, ctx: &mut CheckerContext) {
    let Some(symbol) = ctx.symbols.get(name) else {
        return;
    };
    if symbol.ty == ty {
        return;
    }
    let updated = SymbolInfo {
        ty,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    let declared = ctx
        .symbols
        .declared_type(name)
        .cloned()
        .unwrap_or_else(|| Type::Array(Box::new(Type::Any)));
    let _ = ctx.symbols.insert_narrowed(std::sync::Arc::<str>::from(name), updated, declared);
}

pub(crate) fn collect_array_mutations(
    statement: &ParsedFunctionBodyStatement,
    symbols: &SymbolTable,
) -> Vec<ArrayMutation> {
    let mut mutations = Vec::new();
    let expressions: Vec<&ParsedExpression> = match statement {
        ParsedFunctionBodyStatement::Expression(expression) => vec![expression],
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            variable.initializer.iter().collect()
        }
        ParsedFunctionBodyStatement::Return(statement) => statement.expression.iter().collect(),
        ParsedFunctionBodyStatement::Throw(statement) => vec![&statement.expression],
        ParsedFunctionBodyStatement::Assignment(assignment) => vec![&assignment.value],
        ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
            if let Some(name) = element_write_receiver(&assignment.target, symbols) {
                mutations.push(ArrayMutation {
                    name: name.to_string(),
                    values: vec![(assignment.value.clone(), false)],
                });
            }
            vec![&assignment.value]
        }
        ParsedFunctionBodyStatement::If(statement) => vec![&statement.condition],
        ParsedFunctionBodyStatement::While(statement) => vec![&statement.condition],
        ParsedFunctionBodyStatement::ForOf(statement) => vec![&statement.iterable],
        ParsedFunctionBodyStatement::Switch(statement) => vec![&statement.discriminant],
        _ => Vec::new(),
    };
    for expression in expressions {
        collect_expression_mutations(expression, symbols, &mut mutations);
    }
    mutations
}

/// The evolving array `x` of an element write `x[n] = v`. tsc takes the write
/// only with a number-like index; any other key makes `x` an ordinary read.
pub(super) fn element_write_receiver<'a>(
    target: &'a ParsedExpression,
    symbols: &SymbolTable,
) -> Option<&'a str> {
    let (name, index) = match target {
        ParsedExpression::ElementAccess { object, index, .. } => match object.as_ref() {
            ParsedExpression::Identifier { name, .. } => (name.as_str(), index.as_ref()),
            _ => return None,
        },
        ParsedExpression::IndexAccess { object_name, index, .. } => {
            (object_name.as_str(), index.as_ref())
        }
        _ => return None,
    };
    let (binding, false) = symbols.auto_array(name)? else {
        return None;
    };
    (binding.evolving && is_number_like_index(index, symbols)).then_some(name)
}

fn is_number_like_index(index: &ParsedExpression, symbols: &SymbolTable) -> bool {
    match index {
        ParsedExpression::NumberLiteral(_) => true,
        // `isTypeAssignableToKind(…, NumberLike)` reads a type variable through
        // its constraint: `a[v]` with `v: T extends number` evolves `a`.
        ParsedExpression::Identifier { name, .. } => symbols.get(name).is_some_and(|symbol| {
            let ty = surge_ts_types::type_variable::base_constraint_or_type(&symbol.ty);
            matches!(ty.base_primitive(), Some(Type::Number)) || ty == Type::Any
        }),
        ParsedExpression::PropertyAccess { property_name, .. } => property_name == "length",
        ParsedExpression::Binary { .. } | ParsedExpression::Update { .. } => true,
        _ => false,
    }
}

fn collect_expression_mutations(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    mutations: &mut Vec<ArrayMutation>,
) {
    let recurse = |expression: &ParsedExpression, mutations: &mut Vec<ArrayMutation>| {
        collect_expression_mutations(expression, symbols, mutations);
    };
    let arguments_of = |arguments: &[ParsedCallArgument], mutations: &mut Vec<ArrayMutation>| {
        for argument in arguments {
            collect_expression_mutations(&argument.expression, symbols, mutations);
        }
    };
    match expression {
        ParsedExpression::PropertyCall {
            object,
            property_name,
            arguments,
            ..
        }
        | ParsedExpression::OptionalPropertyCall {
            object,
            property_name,
            arguments,
            ..
        } => {
            arguments_of(arguments, mutations);
            if matches!(property_name.as_str(), "push" | "unshift")
                && let ParsedExpression::Identifier { name, .. } = object.as_ref()
                && let Some((binding, false)) = symbols.auto_array(name)
                && binding.evolving
            {
                mutations.push(ArrayMutation {
                    name: name.clone(),
                    values: arguments
                        .iter()
                        .map(|argument| (argument.expression.clone(), argument.spread))
                        .collect(),
                });
            } else {
                recurse(object, mutations);
            }
        }
        ParsedExpression::Call { arguments, .. } => arguments_of(arguments, mutations),
        ParsedExpression::New { callee, arguments, .. }
        | ParsedExpression::ExpressionCall { callee, arguments, .. }
        | ParsedExpression::OptionalCall { callee, arguments, .. } => {
            recurse(callee, mutations);
            arguments_of(arguments, mutations);
        }
        ParsedExpression::Binary { left, right, .. }
        | ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. } => {
            recurse(left, mutations);
            recurse(right, mutations);
        }
        ParsedExpression::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => {
            recurse(condition, mutations);
            recurse(when_true, mutations);
            recurse(when_false, mutations);
        }
        ParsedExpression::Unary { operand, .. }
        | ParsedExpression::Update { operand, .. }
        | ParsedExpression::Await { operand, .. } => recurse(operand, mutations),
        ParsedExpression::ObjectRest { source, .. }
        | ParsedExpression::EmptyBindingPattern { source, .. } => recurse(source, mutations),
        ParsedExpression::ArrayPatternElement { read, .. } => recurse(read, mutations),
        ParsedExpression::Sequence { expressions } => {
            for (expression, _) in expressions {
                recurse(expression, mutations);
            }
        }
        ParsedExpression::TypeAssertion { expression, .. }
        | ParsedExpression::SatisfiesExpression { expression, .. }
        | ParsedExpression::NonNullAssertion { expression, .. }
        | ParsedExpression::ConstAssertion { expression, .. } => recurse(expression, mutations),
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. } => recurse(object, mutations),
        ParsedExpression::ElementAccess { object, index, .. }
        | ParsedExpression::OptionalIndexAccess { object, index, .. } => {
            recurse(object, mutations);
            recurse(index, mutations);
        }
        ParsedExpression::IndexAccess { index, .. } => recurse(index, mutations),
        ParsedExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                recurse(&element.expression, mutations);
            }
        }
        ParsedExpression::ObjectLiteral { properties, .. } => {
            for property in properties {
                recurse(&property.value, mutations);
            }
        }
        ParsedExpression::TemplateLiteral { expressions, .. } => {
            for expression in expressions {
                recurse(expression, mutations);
            }
        }
        _ => {}
    }
}

/// Adds each mutation's element types to its array, as tsc's
/// `addEvolvingArrayElementType` does: the context-free type of the value, its
/// literals widened (a spread adds what it iterates).
pub(crate) fn apply_array_mutations(
    mutations: Vec<ArrayMutation>,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    for mutation in mutations {
        let element_types = mutation_element_types(&mutation, scopes.visible_symbols(), ctx);
        add_elements(&mutation.name, element_types, scopes, ctx);
    }
}

fn mutation_element_types(
    mutation: &ArrayMutation,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Vec<Type>> {
    mutation
        .values
        .iter()
        .map(|(value, spread)| match infer_expression(value, symbols, ctx) {
            InferredExpression::Known(ty) if !ty.is_unknown() => {
                let element = if *spread { for_of_element_type(&ty.peeled()) } else { ty };
                (!element.is_unknown()).then(|| widen_type(&element))
            }
            _ => None,
        })
        .collect()
}

/// `None` is a value whose type surge could not settle.
fn add_elements(
    name: &str,
    element_types: Option<Vec<Type>>,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let Some(binding) = flow_binding_mut(name, scopes, ctx) else {
        return;
    };
    let Some(ty) = grow_elements(binding, element_types) else {
        return;
    };
    set_binding_type(name, ty, scopes, ctx);
}

/// Adds a mutation's element types to what the array holds, and returns the
/// type a read of it now has — `None` while it stays element-less or unsettled.
fn grow_elements(binding: &mut AutoArrayBinding, element_types: Option<Vec<Type>>) -> Option<Type> {
    if !binding.evolving {
        return None;
    }
    match element_types {
        Some(element_types) => {
            let mut members = vec![binding.elements.clone()];
            members.extend(element_types);
            binding.elements = union_type(members);
            (!binding.unsettled && binding.elements != Type::Never)
                .then(|| Type::Array(Box::new(binding.elements.clone())))
        }
        None => {
            binding.unsettled = true;
            Some(Type::Array(Box::new(Type::Any)))
        }
    }
}

fn set_binding_type(name: &str, ty: Type, scopes: &mut ScopeStack, ctx: &mut CheckerContext) {
    let module_level = matches!(
        scopes.visible_symbols().auto_array(name),
        Some((binding, false)) if binding.module_level
    );
    let Some(symbol) = scopes.resolve(name) else {
        return;
    };
    if symbol.ty == ty {
        return;
    }
    let updated = SymbolInfo {
        ty,
        kind: symbol.kind,
        function_signature: symbol.function_signature.clone(),
    };
    if module_level {
        let declared = ctx
            .symbols
            .declared_type(name)
            .cloned()
            .unwrap_or_else(|| Type::Array(Box::new(Type::Any)));
        let _ = ctx.symbols.insert_narrowed(std::sync::Arc::<str>::from(name), updated, declared);
        return;
    }
    let _ = scopes.update_visible(name, updated);
}

/// A loop body's mutations reach every read in it through the back edge (tsc's
/// loop fixed point), including reads that precede them. Their element types
/// are added before the body is checked; an array with a mutation whose value
/// cannot be typed yet (it names something the body declares) is held
/// unreported for the length of the loop. Returns those arrays.
pub(crate) fn prime_loop_mutations(
    body: &[ParsedFunctionBodyStatement],
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) -> Vec<String> {
    if !auto_arrays_visible(scopes, ctx) {
        return Vec::new();
    }
    let mut mutations = Vec::new();
    collect_body_mutations(body, scopes.visible_symbols(), &mut mutations);
    let mut pending = Vec::new();
    for mutation in mutations {
        match mutation_element_types(&mutation, scopes.visible_symbols(), ctx) {
            Some(element_types) => add_elements(&mutation.name, Some(element_types), scopes, ctx),
            None => {
                if let Some(binding) = flow_binding_mut(&mutation.name, scopes, ctx) {
                    binding.pending_loops += 1;
                    pending.push(mutation.name);
                }
            }
        }
    }
    pending
}

pub(crate) fn release_loop_mutations(
    pending: Vec<String>,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    for name in pending {
        if let Some(binding) = flow_binding_mut(&name, scopes, ctx) {
            binding.pending_loops = binding.pending_loops.saturating_sub(1);
        }
    }
}

fn collect_body_mutations(
    body: &[ParsedFunctionBodyStatement],
    symbols: &SymbolTable,
    mutations: &mut Vec<ArrayMutation>,
) {
    for statement in body {
        mutations.extend(collect_array_mutations(statement, symbols));
        match statement {
            ParsedFunctionBodyStatement::Block(block) => {
                collect_body_mutations(block, symbols, mutations);
            }
            ParsedFunctionBodyStatement::If(statement) => {
                collect_body_mutations(&statement.then_body, symbols, mutations);
                collect_body_mutations(&statement.else_body, symbols, mutations);
            }
            ParsedFunctionBodyStatement::While(statement) => {
                collect_body_mutations(&statement.body, symbols, mutations);
            }
            ParsedFunctionBodyStatement::ForOf(statement) => {
                collect_body_mutations(&statement.body, symbols, mutations);
            }
            ParsedFunctionBodyStatement::Switch(statement) => {
                for case in &statement.cases {
                    collect_body_mutations(&case.consequent, symbols, mutations);
                }
            }
            ParsedFunctionBodyStatement::Try(statement) => {
                collect_body_mutations(&statement.block, symbols, mutations);
                if let Some(handler) = &statement.handler {
                    collect_body_mutations(&handler.body, symbols, mutations);
                }
                collect_body_mutations(&statement.finalizer, symbols, mutations);
            }
            _ => {}
        }
    }
}
