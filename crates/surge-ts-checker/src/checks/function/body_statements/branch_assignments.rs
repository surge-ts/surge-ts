
use surge_ts_syntax::{
    ParsedExpression,
    ParsedFunctionBodyStatement,
};
use surge_ts_types::{Type, TypeCopyReason, is_assignable_to, union_type, with_type_copy_reason};

use crate::context::CheckerContext;
use crate::symbols::{ScopeStack, SymbolInfo};
use super::super::narrow_discriminant_in_scope;

/// The bindings a branch body assigns at its own statement level. Deeper
/// assignments are discarded with their own inner frame before the branch ends,
/// so they cannot reach the join.
/// The binding a reference path starts from: `a` for `a.b.c` and `a[0].b`.
/// `None` for a path rooted in anything that is not a plain binding.
fn reference_root_name(expression: &ParsedExpression) -> Option<&str> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name),
        // `this` is bound in scope under its own name, like any root.
        ParsedExpression::This { .. } => Some("this"),
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. }
        | ParsedExpression::ElementAccess { object, .. } => reference_root_name(object),
        ParsedExpression::IndexAccess { object_name, .. } => Some(object_name),
        _ => None,
    }
}

pub(crate) fn branch_assigned_names(body: &[ParsedFunctionBodyStatement], names: &mut Vec<String>) {
    for statement in body {
        // An assignment used as a value (`if ((m = f()))`) assigns too.
        let values: Vec<&surge_ts_syntax::ParsedExpression> = match statement {
            ParsedFunctionBodyStatement::Expression(expression) => vec![expression],
            ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                variable.initializer.iter().collect()
            }
            ParsedFunctionBodyStatement::Return(statement) => statement.expression.iter().collect(),
            ParsedFunctionBodyStatement::Assignment(assignment) => vec![&assignment.value],
            ParsedFunctionBodyStatement::MemberAssignment(assignment) => vec![&assignment.value],
            ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => vec![&assignment.value],
            ParsedFunctionBodyStatement::If(statement) => vec![&statement.condition],
            ParsedFunctionBodyStatement::While(statement) => vec![&statement.condition],
            _ => Vec::new(),
        };
        for value in values {
            for (assignment, _) in crate::flow::expression_assignments(value) {
                if let surge_ts_syntax::ParsedExpression::Assignment { target_name, .. } = assignment
                    && !names.iter().any(|name| name == target_name)
                {
                    names.push(target_name.clone());
                }
            }
        }
        match statement {
            ParsedFunctionBodyStatement::Assignment(assignment) => {
                if !names.iter().any(|name| *name == assignment.target_name) {
                    names.push(assignment.target_name.clone());
                }
            }
            // `o.p = v` narrows by rewriting the *base* symbol's type
            // (`narrow_reference_in_scope`), so the name to carry across the
            // branch join is the root of the reference path, not the member.
            ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
                if let Some(base) = reference_root_name(&assignment.target)
                    && !names.iter().any(|name| name == base)
                {
                    names.push(base.to_string());
                }
            }
            ParsedFunctionBodyStatement::ThisPropertyAssignment(_) => {
                if !names.iter().any(|name| name == "this") {
                    names.push("this".to_string());
                }
            }
            ParsedFunctionBodyStatement::Block(block) => branch_assigned_names(block, names),
            _ => {}
        }
    }
}

/// The bindings a loop's exit joins: an assignment nested in a branch of the
/// body still reaches the exit, and the inner join has already written it to
/// the declaring frame.
pub(super) fn loop_join_names(body: &[ParsedFunctionBodyStatement]) -> Vec<String> {
    let mut names = Vec::new();
    branch_assigned_names(body, &mut names);
    loop_assigned_names(body, &mut names);
    names
}

/// Every plain binding a loop body assigns, at any depth. Unlike a branch, a
/// loop's nested assignments reach its head through the back edge.
fn loop_assigned_names(body: &[ParsedFunctionBodyStatement], names: &mut Vec<String>) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Assignment(assignment) => {
                if !names.iter().any(|name| *name == assignment.target_name) {
                    names.push(assignment.target_name.clone());
                }
            }
            ParsedFunctionBodyStatement::Block(block) => loop_assigned_names(block, names),
            ParsedFunctionBodyStatement::If(if_statement) => {
                loop_assigned_names(&if_statement.then_body, names);
                loop_assigned_names(&if_statement.else_body, names);
            }
            ParsedFunctionBodyStatement::While(while_statement) => {
                loop_assigned_names(&while_statement.body, names);
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                loop_assigned_names(&for_of_statement.body, names);
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    loop_assigned_names(&case.consequent, names);
                }
            }
            ParsedFunctionBodyStatement::Try(try_statement) => {
                loop_assigned_names(&try_statement.block, names);
                if let Some(handler) = &try_statement.handler {
                    loop_assigned_names(&handler.body, names);
                }
                loop_assigned_names(&try_statement.finalizer, names);
            }
            _ => {}
        }
    }
}

thread_local! {
    static IN_LOOP_PREPASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn in_loop_prepass() -> bool {
    IN_LOOP_PREPASS.with(std::cell::Cell::get)
}

/// tsc types a binding at a loop head as the union of the entry edge and every
/// back edge, so `let min: number | null = null` does not read as `null`
/// throughout a loop that assigns it.
pub(super) fn widen_loop_assigned_bindings(
    body: &[ParsedFunctionBodyStatement],
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut crate::flow::FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let names = deep_assigned_names(&[body]);
    if names.is_empty() {
        return;
    }
    // A loop nested inside the body being pre-checked settles for the
    // declarations, so nesting costs one extra pass, not one per level.
    if IN_LOOP_PREPASS.with(std::cell::Cell::get) {
        widen_assigned_bindings(&[body], scopes);
        return;
    }
    // The back edge carries what the body leaves each binding as: check the
    // body once with nothing reported, read those types, and start the real
    // pass at the union of the entry and back-edge types.
    let entry_types = branch_assignment_types(&names, scopes);
    let saved_scopes = scopes.clone();
    let saved_flow = flow_state.clone();
    let diagnostics_before = ctx.diagnostics().len();
    IN_LOOP_PREPASS.with(|flag| flag.set(true));
    scopes.push_child();
    crate::checks::function::check_function_body(body.to_vec(), return_type, scopes, flow_state, ctx);
    let back_edge_types = branch_assignment_types(&names, scopes);
    IN_LOOP_PREPASS.with(|flag| flag.set(false));
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    *scopes = saved_scopes;
    *flow_state = saved_flow;
    let head_types: Vec<(String, Type)> = entry_types
        .iter()
        .filter_map(|(name, entry)| {
            let back_edge = back_edge_types.iter().find(|(other, _)| other == name)?;
            let joined = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
                join_flow_types(vec![entry.clone(), back_edge.1.clone()])
            });
            (joined != *entry).then(|| (name.clone(), joined))
        })
        .collect();
    adopt_branch_assignments(&head_types, scopes);
}

/// Every plain binding any of `bodies` assigns, at any depth.
pub(crate) fn deep_assigned_names(bodies: &[&[ParsedFunctionBodyStatement]]) -> Vec<String> {
    let mut names = Vec::new();
    for body in bodies {
        loop_assigned_names(body, &mut names);
    }
    names
}

/// Widens each binding `bodies` assign to its declared type, for code that
/// can be reached from any point inside them — a loop head through its back
/// edge, a `catch` or `finally` from wherever the `try` threw.
pub(super) fn widen_assigned_bindings(
    bodies: &[&[ParsedFunctionBodyStatement]],
    scopes: &mut ScopeStack,
) {
    for name in deep_assigned_names(bodies) {
        let Some(symbol) = scopes.resolve(&name) else {
            continue;
        };
        let Some(declared) = scopes.visible_symbols().declared_type(&name).cloned() else {
            continue;
        };
        if declared == symbol.ty || declared.is_unknown() {
            continue;
        }
        let widened = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            union_type(vec![symbol.ty.clone(), declared])
        });
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let _ = scopes.update_visible(
            &name,
            SymbolInfo {
                ty: widened,
                kind,
                function_signature,
            },
        );
    }
}

/// Joins a then-branch's end types with the fall-through (condition-false) types
/// for the bindings it assigned. tsc types the code after `if (!x) { x = … }`
/// from both incoming edges; surge's branch scope discards the assignment
/// narrowing on `pop_child`, which otherwise leaves the declared union in place
/// for every later use. The join only ever removes union members both edges
/// rule out — a widening result is a modelling artifact and is dropped.
pub(super) fn join_branch_assignments(
    branch_types: &[(String, Type)],
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    if branch_types.is_empty() {
        return;
    }

    scopes.push_child();
    narrow_discriminant_in_scope(condition, scopes, false, ctx);
    let fallthrough_types: Vec<Option<Type>> = branch_types
        .iter()
        .map(|(name, _)| {
            scopes.resolve(name).map(|symbol| {
                with_type_copy_reason(TypeCopyReason::ScopeOrContext, || symbol.ty.clone())
            })
        })
        .collect();
    scopes.pop_child();

    for ((name, branch_ty), fallthrough_ty) in branch_types.iter().zip(fallthrough_types) {
        let Some(fallthrough_ty) = fallthrough_ty else {
            continue;
        };
        let joined = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            join_flow_types(vec![branch_ty.clone(), fallthrough_ty])
        });
        let Some(symbol) = scopes.resolve(name) else {
            continue;
        };
        // Bound by the *declaration*, not by whatever narrowing survives the
        // branch: with both edges narrowed (`let s: Wide = "a"; if (c) s = "b";`)
        // the join is legitimately wider than either, and comparing against the
        // fall-through narrowing alone would drop it.
        let bound = scopes
            .visible_symbols()
            .declared_type(name)
            .unwrap_or(&symbol.ty)
            .clone();
        if joined == symbol.ty || !is_assignable_to(&joined, &bound) {
            continue;
        }
        let joined_symbol = SymbolInfo {
            ty: joined,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        // Written to the owning frame, not shadowed in the current one: the join
        // describes the binding from the `if` onward, and a block-local shadow
        // would be dropped before a `break`/loop-exit edge that carries it. A
        // later narrowing reads the binding's declaration, not this join.
        let _ = scopes.update_visible_narrowed(name, joined_symbol, bound);
    }
}

/// Joins the two edges of an `if`/`else` for the bindings either branch assigns.
/// Both branch frames have popped, so each side's end type is supplied as a
/// snapshot; the result is bounded by the declaration, never by whatever
/// narrowing survives the statement.
pub(super) fn join_branch_pair(
    then_types: &[(String, Type)],
    else_types: &[(String, Type)],
    scopes: &mut ScopeStack,
) {
    for (name, then_ty) in then_types {
        let Some((_, else_ty)) = else_types.iter().find(|(other, _)| other == name) else {
            continue;
        };
        let joined = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            join_flow_types(vec![then_ty.clone(), else_ty.clone()])
        });
        let Some(symbol) = scopes.resolve(name) else {
            continue;
        };
        let current = symbol.ty.clone();
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let bound = scopes
            .visible_symbols()
            .declared_type(name)
            .cloned()
            .unwrap_or_else(|| current.clone());
        if joined == current || !is_assignable_to(&joined, &bound) {
            continue;
        }
        let _ = scopes.update_visible_narrowed(
            name,
            SymbolInfo {
                ty: joined,
                kind,
                function_signature,
            },
            bound,
        );
    }
}

/// tsc's post-`if` join of two branches that both complete, for the bindings
/// each narrowed by a guard (`if (c) { if (!x) return; } else { if (x === null)
/// return; }`): the binding leaves as the union of the two, where the frames
/// alone would drop both narrowings at the join. Assigned bindings are
/// [`join_branch_pair`]'s.
pub(super) fn join_branch_narrowings(
    then_narrowings: &[(std::sync::Arc<str>, crate::symbols::SymbolInfoHandle, Type)],
    else_narrowings: &[(std::sync::Arc<str>, crate::symbols::SymbolInfoHandle, Type)],
    assigned: &[String],
    scopes: &mut ScopeStack,
) {
    for (name, then_symbol, declared) in then_narrowings {
        if assigned.iter().any(|assigned| assigned.as_str() == name.as_ref()) {
            continue;
        }
        let Some((_, else_symbol, _)) = else_narrowings.iter().find(|(other, _, _)| other == name) else {
            continue;
        };
        let joined = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
            join_flow_types(vec![then_symbol.ty.clone(), else_symbol.ty.clone()])
        });
        let Some(current) = scopes.resolve(name) else {
            continue;
        };
        if joined == current.ty || !is_assignable_to(&joined, &current.ty) {
            continue;
        }
        let symbol = SymbolInfo {
            ty: joined,
            ..(**then_symbol).clone()
        };
        let _ = scopes.insert_current_narrowed(name.clone(), symbol, declared.clone());
    }
}

/// Adopts a branch's end types outright, for a join the branch is the *only*
/// incoming edge of. A `try` whose `catch` returns or throws is that shape:
/// reaching the statement's end means the block ran to completion, so the
/// assignments it made hold from there on. Without this the narrowing dies with
/// the branch frame and a `let x: T | undefined` assigned inside the `try` reads
/// as possibly-undefined for the rest of the function.
pub(super) fn adopt_branch_assignments(branch_types: &[(String, Type)], scopes: &mut ScopeStack) {
    for (name, branch_ty) in branch_types {
        let Some(symbol) = scopes.resolve(name) else {
            continue;
        };
        if *branch_ty == symbol.ty {
            continue;
        }
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let bound = scopes
            .visible_symbols()
            .declared_type(name)
            .cloned()
            .unwrap_or_else(|| symbol.ty.clone());
        if !is_assignable_to(branch_ty, &bound) {
            continue;
        }
        let _ = scopes.update_visible_narrowed(
            name,
            SymbolInfo {
                ty: branch_ty.clone(),
                kind,
                function_signature,
            },
            bound,
        );
    }
}

/// Joins every edge reaching the code after a statement with more than two
/// (`switch` cases, a `try` and its `catch`): each binding an edge assigned
/// becomes the union of what the edges left it as, bounded by its declaration
/// like [`join_branch_pair`].
pub(super) fn join_branch_edges(edges: &[Vec<(String, Type)>], scopes: &mut ScopeStack) {
    let Some(first) = edges.first() else {
        return;
    };
    for (name, _) in first {
        let types: Vec<Type> = edges
            .iter()
            .filter_map(|edge| edge.iter().find(|(other, _)| other == name))
            .map(|(_, ty)| ty.clone())
            .collect();
        if types.len() != edges.len() {
            continue;
        }
        let joined = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || join_flow_types(types));
        let Some(symbol) = scopes.resolve(name) else {
            continue;
        };
        let current = symbol.ty.clone();
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let bound = scopes
            .visible_symbols()
            .declared_type(name)
            .cloned()
            .unwrap_or_else(|| current.clone());
        if joined == current || !is_assignable_to(&joined, &bound) {
            continue;
        }
        let _ = scopes.update_visible_narrowed(
            name,
            SymbolInfo {
                ty: joined,
                kind,
                function_signature,
            },
            bound,
        );
    }
}

/// The type a binding holds where edges meet. surge keeps a member narrowing
/// (`o.p = v`, `if (!o.p)`) on the binding's object, where tsc narrows only the
/// `o.p` reference, so edges that narrowed members of one object join member
/// by member — each member reads the union of what the edges left it as —
/// instead of as a union of objects a later member narrowing cannot see into.
pub(super) fn join_flow_types(types: Vec<Type>) -> Type {
    join_member_narrowings(&types, 0).unwrap_or_else(|| union_type(types))
}

fn join_member_narrowings(types: &[Type], depth: usize) -> Option<Type> {
    if types.len() < 2 || depth > 8 {
        return None;
    }
    // An edge that left the binding as declared, a nominal reference, absorbs
    // the edges that only narrowed its members: each member's union is its
    // declared type again.
    if let Some(declared) = types.iter().find(|ty| matches!(ty, Type::Reference(_)))
        && let Type::Object(shape) = declared.peeled()
        && types.iter().all(|ty| {
            ty == declared
                || matches!(ty, Type::Object(object)
                    if object.alias_name == shape.alias_name
                        && object.properties.len() == shape.properties.len()
                        && object.properties.keys().all(|name| shape.properties.contains_key(name)))
        })
    {
        return Some(declared.clone());
    }
    let objects: Vec<&surge_ts_types::ObjectType> = types
        .iter()
        .map(|ty| match ty {
            Type::Object(object) => Some(object),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let first = objects[0];
    let same_object = objects.iter().all(|object| {
        object.alias_name == first.alias_name
            && object.properties.len() == first.properties.len()
            && object.properties.keys().all(|name| first.properties.contains_key(name))
            && object.string_index_type == first.string_index_type
            && object.number_index_type == first.number_index_type
            && object.call_signature == first.call_signature
            && object.construct_signature == first.construct_signature
    });
    if !same_object || objects.iter().all(|object| *object == first) {
        return None;
    }
    let mut merged = first.clone();
    let properties = std::sync::Arc::make_mut(&mut merged.properties);
    for (name, property) in first.properties.iter() {
        let members: Vec<Type> = objects
            .iter()
            .filter_map(|object| object.properties.get(name).map(|member| member.ty.clone()))
            .collect();
        let optional = objects
            .iter()
            .any(|object| object.properties.get(name).is_some_and(|member| member.optional));
        let ty = join_member_narrowings(&members, depth + 1).unwrap_or_else(|| union_type(members));
        properties.insert(
            name.clone(),
            surge_ts_types::ObjectProperty {
                ty,
                optional,
                ..property.clone()
            },
        );
    }
    Some(Type::Object(merged))
}

/// Every binding a branch may write, however deeply nested: the joins inside
/// it write to the binding's owning frame, which outlives the branch's own.
pub(super) fn nested_assigned_names(body: &[ParsedFunctionBodyStatement], names: &mut Vec<String>) {
    branch_assigned_names(body, names);
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Block(block) => nested_assigned_names(block, names),
            ParsedFunctionBodyStatement::If(if_statement) => {
                nested_assigned_names(&if_statement.then_body, names);
                nested_assigned_names(&if_statement.else_body, names);
            }
            ParsedFunctionBodyStatement::While(while_statement) => {
                nested_assigned_names(&while_statement.body, names)
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                nested_assigned_names(&for_of_statement.body, names)
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    nested_assigned_names(&case.consequent, names);
                }
            }
            ParsedFunctionBodyStatement::Try(try_statement) => {
                nested_assigned_names(&try_statement.block, names);
                if let Some(handler) = &try_statement.handler {
                    nested_assigned_names(&handler.body, names);
                }
                nested_assigned_names(&try_statement.finalizer, names);
            }
            _ => {}
        }
    }
}

/// Puts each binding back to what it held where a branch began, for the
/// sibling branch or join that starts from that point: tsc's flow reaches an
/// `else` or a later `case` from the branch's entry, never through a sibling.
pub(super) fn restore_branch_entry(entry_types: &[(String, Type)], scopes: &mut ScopeStack) {
    for (name, entry_ty) in entry_types {
        let Some(symbol) = scopes.resolve(name) else {
            continue;
        };
        if symbol.ty == *entry_ty {
            continue;
        }
        let kind = symbol.kind;
        let function_signature = symbol.function_signature.clone();
        let _ = scopes.update_visible(
            name,
            SymbolInfo {
                ty: entry_ty.clone(),
                kind,
                function_signature,
            },
        );
    }
}

/// Snapshots the current type of each assigned binding at a branch's end, before
/// its scope frame pops.
pub(super) fn branch_assignment_types(names: &[String], scopes: &ScopeStack) -> Vec<(String, Type)> {
    names
        .iter()
        .filter_map(|name| {
            scopes.resolve(name).map(|symbol| {
                (
                    name.clone(),
                    with_type_copy_reason(TypeCopyReason::ScopeOrContext, || symbol.ty.clone()),
                )
            })
        })
        .collect()
}
