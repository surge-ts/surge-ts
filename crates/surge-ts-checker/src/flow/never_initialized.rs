//! TS2454 for a `let` read from a flow container nested inside the one that
//! declares it. tsc assumes such an outer binding initialized — some other code
//! may have run first — unless it is `isNeverInitialized`: a mutable local `let`
//! (not exported, not a global script's top-level binding) with no initializer
//! and no definite assignment anywhere in its declaring container. Then every
//! read, from any nested function, arrow, method or property initializer, sees
//! the declared type widened by `undefined` and reports.

use std::collections::HashSet;
use std::sync::Arc;

use surge_ts_syntax::{
    ParsedFunctionBodyStatement, ParsedFunctionParameter,
    ParsedStatement, ParsedType, ParsedVariableDeclaration, ParsedVariableKind,
};

use super::{AssignmentState, FunctionFlowState, type_assumed_initialized};
use crate::context::CheckerContext;

/// Installs the file's definite writes and its top-level never-initialized
/// `let`s as what nested containers inherit.
pub(crate) fn begin_file(
    statements: &[ParsedStatement],
    is_module: bool,
    definite_writes: &[String],
    ctx: &mut CheckerContext,
) {
    ctx.definite_writes = Arc::new(definite_writes.iter().cloned().collect());
    // A global script's top-level `let` is visible to every other file, so tsc
    // never takes it as never-initialized.
    ctx.inherited_never_initialized = if is_module {
        statements
            .iter()
            .filter_map(|statement| match statement {
                ParsedStatement::VariableDeclaration(variable) => Some(variable.as_ref()),
                _ => None,
            })
            .filter(|variable| never_initialized_candidate(variable, ctx))
            .filter(|variable| {
                let declared = ctx
                    .symbols
                    .declared_type(&variable.name)
                    .cloned()
                    .or_else(|| ctx.symbols.get(&variable.name).map(|symbol| symbol.ty.clone()));
                match declared {
                    Some(declared) => !type_assumed_initialized(&declared, ctx),
                    None => variable
                        .declared_type
                        .as_ref()
                        .is_some_and(|annotation| excludes_undefined(annotation, ctx, &[])),
                }
            })
            .map(|variable| variable.name.as_str().into())
            .collect()
    } else {
        Vec::new()
    };
    super::unassigned_reads::record_module_unassigned_reads(statements, ctx);
}

/// Enters a nested flow container: seeds `flow` with the inherited bindings its
/// own parameters and declarations do not shadow, and makes those plus the
/// container's own never-initialized `let`s what *its* nested containers
/// inherit. Returns the list to restore on leaving.
pub(crate) fn enter_container(
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    parameters: &[ParsedFunctionParameter],
    body: &[ParsedFunctionBodyStatement],
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> Vec<Arc<str>> {
    let mut declared = HashSet::new();
    collect_declared_names(body, &mut declared);
    let seed: Vec<Arc<str>> = ctx
        .inherited_never_initialized
        .iter()
        .filter(|name| !super::binds_parameter(parameters, name) && !declared.contains(name.as_ref()))
        .cloned()
        .collect();
    flow.seed_outer_unassigned(seed.clone());
    for name in &seed {
        if ctx.never_initialized_constraint_exempt.contains(name) {
            flow.constraint_exempt.insert(Arc::clone(name));
        }
    }
    let mut inherited = seed;
    collect_body_candidates(body, type_parameters, ctx, &mut inherited);
    flow.set_assigned_bindings(super::assigned_bindings(body));
    std::mem::replace(&mut ctx.inherited_never_initialized, inherited)
}

/// Enters a namespace body as a container: its own top-level `let`s that are
/// never initialized (and not exported) join what its nested bodies inherit,
/// and any outer binding it redeclares drops out. Returns the list to restore.
pub(crate) fn enter_namespace(statements: &[ParsedStatement], ctx: &mut CheckerContext) -> Vec<Arc<str>> {
    let declared: HashSet<&str> = statements
        .iter()
        .filter_map(|statement| {
            let statement = match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                    _ => return None,
                },
                other => other,
            };
            match statement {
                ParsedStatement::VariableDeclaration(variable) => Some(variable.name.as_str()),
                ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
                ParsedStatement::ClassDeclaration(class) => Some(class.name.as_str()),
                _ => None,
            }
        })
        .collect();
    let mut inherited: Vec<Arc<str>> = ctx
        .inherited_never_initialized
        .iter()
        .filter(|name| !declared.contains(name.as_ref()))
        .cloned()
        .collect();
    for statement in statements {
        if let ParsedStatement::VariableDeclaration(variable) = statement
            && never_initialized_candidate(variable, ctx)
            && variable
                .declared_type
                .as_ref()
                .is_some_and(|annotation| excludes_undefined(annotation, ctx, &[]))
        {
            inherited.push(variable.name.as_str().into());
        }
    }
    std::mem::replace(&mut ctx.inherited_never_initialized, inherited)
}

/// A property initializer or expression-bodied arrow: a container with no
/// declarations of its own.
pub(crate) fn expression_container_flow(
    parameters: &[ParsedFunctionParameter],
    ctx: &CheckerContext,
) -> Option<FunctionFlowState> {
    let seed: Vec<Arc<str>> = ctx
        .inherited_never_initialized
        .iter()
        .filter(|name| !super::binds_parameter(parameters, name))
        .cloned()
        .collect();
    if seed.is_empty() {
        return None;
    }
    let mut flow = FunctionFlowState::new(true);
    for name in &seed {
        if ctx.never_initialized_constraint_exempt.contains(name) {
            flow.constraint_exempt.insert(Arc::clone(name));
        }
    }
    flow.seed_outer_unassigned(seed);
    Some(flow)
}

/// Seeds an immediately invoked body with its caller's bindings still
/// unassigned at the call (see `CheckerContext::immediately_invoked_unassigned`)
/// that the body does not shadow. They stay out of what its own nested
/// containers inherit: to those they are outer variables, assumed initialized.
pub(crate) fn seed_immediately_invoked(
    function_start: Option<usize>,
    own_name: Option<&str>,
    parameters: &[ParsedFunctionParameter],
    body: &[ParsedFunctionBodyStatement],
    flow: &mut FunctionFlowState,
    ctx: &CheckerContext,
) {
    let Some(names) = function_start.and_then(|start| {
        ctx.immediately_invoked_unassigned
            .iter()
            .rev()
            .find(|(function, _)| *function == start)
            .map(|(_, names)| names)
    }) else {
        return;
    };
    let mut shadowed = HashSet::new();
    collect_declared_names(body, &mut shadowed);
    for parameter in parameters {
        shadowed.extend(parameter.binding_name.bound_names().into_iter().map(|bound| bound.name));
    }
    shadowed.extend(own_name.map(str::to_string));
    shadowed.insert("arguments".to_string());
    flow.seed_outer_unassigned(
        names
            .iter()
            .filter(|name| !shadowed.contains(name.as_ref()))
            .cloned()
            .collect(),
    );
}

impl FunctionFlowState {
    pub(crate) fn seed_outer_unassigned(&mut self, names: Vec<Arc<str>>) {
        if names.is_empty() {
            return;
        }
        self.enabled = true;
        self.push_scope(std::collections::HashMap::new());
        for name in names {
            self.declare_current(name, AssignmentState::DeclaredUnassigned);
        }
    }
}

fn never_initialized_candidate(variable: &ParsedVariableDeclaration, ctx: &CheckerContext) -> bool {
    variable.kind == ParsedVariableKind::Let
        && variable.initializer.is_none()
        && !variable.is_declare
        && !variable.has_definite_assertion
        && !variable.from_binding_pattern
        && variable.declared_type.is_some()
        && !ctx.definite_writes.contains(&variable.name)
}

/// A function-local `let` is not in any resolved table when a nested body is
/// checked, so its annotation is judged as written (see
/// [`excludes_undefined`]).
fn collect_body_candidates(
    body: &[ParsedFunctionBodyStatement],
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    ctx: &mut CheckerContext,
    names: &mut Vec<Arc<str>>,
) {
    let mut found = Vec::new();
    let local_types = body_local_types(body);
    for_each_declaration(body, &mut |variable| {
        let Some(annotation) = variable.declared_type.as_ref() else {
            return;
        };
        if never_initialized_candidate(variable, ctx)
            && annotation_excludes(annotation, ctx, type_parameters, &local_types, &[], 0)
            && !names.iter().any(|name| name.as_ref() == variable.name)
        {
            let exempt = annotation_constraint_substitutes(annotation, ctx, type_parameters);
            found.push((Arc::<str>::from(variable.name.as_str()), exempt));
        }
    });
    for (name, exempt) in found {
        if exempt {
            ctx.never_initialized_constraint_exempt.insert(Arc::clone(&name));
        }
        names.push(name);
    }
}

/// The types `body` declares for itself, which its annotations may name: an
/// interface or class type (`None`) never admits `undefined`, a non-generic
/// alias — an enum's included — only through its body.
fn body_local_types(body: &[ParsedFunctionBodyStatement]) -> Vec<(String, Option<ParsedType>)> {
    fn collect(body: &[ParsedFunctionBodyStatement], types: &mut Vec<(String, Option<ParsedType>)>) {
        for statement in body {
            match statement {
                ParsedFunctionBodyStatement::TypeAlias(alias) if alias.type_parameters.is_empty() => {
                    types.push((alias.name.clone(), Some(alias.ty.clone())));
                }
                ParsedFunctionBodyStatement::Interface(interface) => types.push((interface.name.clone(), None)),
                ParsedFunctionBodyStatement::Class(class) => types.push((class.name.clone(), None)),
                _ => {}
            }
            for_each_child_body(statement, &mut |child| collect(child, types));
        }
    }
    let mut types = Vec::new();
    collect(body, &mut types);
    types
}

pub(crate) fn collect_declared_names(body: &[ParsedFunctionBodyStatement], names: &mut HashSet<String>) {
    for_each_declaration(body, &mut |variable| {
        names.insert(variable.name.clone());
    });
    fn others(body: &[ParsedFunctionBodyStatement], names: &mut HashSet<String>) {
        for statement in body {
            match statement {
                ParsedFunctionBodyStatement::Function(function) => {
                    names.insert(function.name.clone());
                }
                ParsedFunctionBodyStatement::Class(class) => {
                    names.insert(class.name.clone());
                }
                ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                    if let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } =
                        &for_of_statement.binding_name
                    {
                        names.insert(name.clone());
                    }
                    others(&for_of_statement.body, names);
                }
                ParsedFunctionBodyStatement::Try(try_statement) => {
                    others(&try_statement.block, names);
                    if let Some(handler) = &try_statement.handler {
                        if let Some(surge_ts_syntax::ParsedBindingName::Identifier { name, .. }) =
                            &handler.binding_name
                        {
                            names.insert(name.clone());
                        }
                        others(&handler.body, names);
                    }
                    others(&try_statement.finalizer, names);
                }
                _ => for_each_child_body(statement, &mut |child| others(child, names)),
            }
        }
    }
    others(body, names);
}

fn for_each_declaration(
    body: &[ParsedFunctionBodyStatement],
    visit: &mut dyn FnMut(&ParsedVariableDeclaration),
) {
    for statement in body {
        if let ParsedFunctionBodyStatement::VariableDeclaration(variable) = statement {
            visit(variable);
        }
        for_each_child_body(statement, &mut |child| for_each_declaration(child, visit));
    }
}

/// The nested statement lists of `statement` that share its flow container.
fn for_each_child_body(
    statement: &ParsedFunctionBodyStatement,
    visit: &mut dyn FnMut(&[ParsedFunctionBodyStatement]),
) {
    match statement {
        ParsedFunctionBodyStatement::Block(block) => visit(block),
        ParsedFunctionBodyStatement::If(if_statement) => {
            visit(&if_statement.then_body);
            visit(&if_statement.else_body);
        }
        ParsedFunctionBodyStatement::While(while_statement) => visit(&while_statement.body),
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => visit(&for_of_statement.body),
        ParsedFunctionBodyStatement::Switch(switch_statement) => {
            for case in &switch_statement.cases {
                visit(&case.consequent);
            }
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            visit(&try_statement.block);
            if let Some(handler) = &try_statement.handler {
                visit(&handler.body);
            }
            visit(&try_statement.finalizer);
        }
        _ => {}
    }
}

/// Whether a written annotation excludes `undefined`, `any`, `unknown` and
/// `void`, following names through the declaration table without evaluating
/// anything: an interface or class type never admits `undefined` whatever its
/// arguments, an alias does so only through its body (one level of argument
/// substitution), and a type parameter only through its constraint's
/// substitution, which the caller tracks. Anything unresolved answers no.
pub(crate) fn excludes_undefined(
    annotation: &ParsedType,
    ctx: &CheckerContext,
    local_type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
) -> bool {
    annotation_excludes(annotation, ctx, local_type_parameters, &[], &[], 0)
}

fn annotation_excludes(
    annotation: &ParsedType,
    ctx: &CheckerContext,
    local_type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    local_types: &[(String, Option<ParsedType>)],
    substitution: &[(&str, &ParsedType)],
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    match annotation {
        ParsedType::Named(named) => {
            if named.type_arguments.is_empty() {
                if let Some((_, argument)) =
                    substitution.iter().find(|(parameter, _)| *parameter == named.name)
                {
                    return annotation_excludes(argument, ctx, local_type_parameters, local_types, &[], depth + 1);
                }
                if local_type_parameters.iter().any(|parameter| parameter.name == named.name)
                    || ctx.type_parameter_in_scope(&named.name)
                {
                    return true;
                }
                if let Some((_, local)) = local_types.iter().find(|(name, _)| *name == named.name) {
                    return match local {
                        Some(body) => annotation_excludes(body, ctx, local_type_parameters, local_types, &[], depth + 1),
                        None => true,
                    };
                }
            }
            let declaration = ctx
                .type_declarations
                .get(&named.name)
                .or_else(|| ctx.ambient_global_type_declarations.get(&named.name));
            match declaration {
                Some(crate::symbols::TypeDeclarationInfo::Interface(_)) => true,
                Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) => {
                    let parameters = &alias.body.type_parameters;
                    if parameters.len() != named.type_arguments.len() {
                        return false;
                    }
                    let inner: Vec<(&str, &ParsedType)> = parameters
                        .iter()
                        .map(|parameter| parameter.name.as_str())
                        .zip(named.type_arguments.iter())
                        .collect();
                    annotation_excludes(&alias.body.ty, ctx, local_type_parameters, &[], &inner, depth + 1)
                }
                None => false,
            }
        }
        ParsedType::Union(members) => members.iter().all(|member| {
            annotation_excludes(member, ctx, local_type_parameters, local_types, substitution, depth)
        }),
        other => is_plainly_defined(other),
    }
}

/// Whether a never-initialized binding annotated as a type parameter escapes
/// TS2454 where tsc substitutes its constraint (see
/// `FunctionFlowState::constraint_exempt`).
pub(crate) fn annotation_constraint_substitutes(
    annotation: &ParsedType,
    ctx: &CheckerContext,
    local_type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
) -> bool {
    let ParsedType::Named(named) = annotation else {
        return false;
    };
    if !named.type_arguments.is_empty() {
        return false;
    }
    match local_type_parameters.iter().find(|parameter| parameter.name == named.name) {
        Some(parameter) => parameter.constraint.as_ref().is_some_and(|constraint| {
            matches!(constraint, ParsedType::Undefined)
                || matches!(constraint, ParsedType::Union(members)
                    if members.iter().any(|member| matches!(member, ParsedType::Undefined)))
        }),
        None => super::constraint_substitutes(&named.name, ctx),
    }
}

pub(crate) fn is_plainly_defined(annotation: &ParsedType) -> bool {
    match annotation {
        // `null` is not `undefined`: `containsUndefinedType` does not see it.
        ParsedType::Null
        | ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_)
        | ParsedType::Object(_)
        | ParsedType::Array(_)
        | ParsedType::Tuple(..)
        | ParsedType::Function(_) => true,
        ParsedType::Union(members) => members.iter().all(is_plainly_defined),
        _ => false,
    }
}
