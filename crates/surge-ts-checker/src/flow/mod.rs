use std::collections::HashMap;
use std::sync::Arc;

use surge_ts_syntax::{ParsedExpression, ParsedFunctionBodyStatement, TextSpan as SyntaxTextSpan};

use crate::context::CheckerContext;
use crate::program::{
    record_flow_branch_changed_local_count, record_flow_branch_empty_delta_count,
    record_flow_read_lookup_count, record_flow_scope_pop_count, record_flow_scope_push_count,
    record_flow_state_clone_count, record_flow_state_full_clone_avoided_count,
};

mod assigned;
mod branch;
mod expr;
mod facts;
mod guards;
mod module_scope;
mod never_initialized;
mod unassigned_reads;

pub(crate) use assigned::{assigned_bindings, module_assigned_bindings};
pub(crate) use branch::*;
pub(crate) use expr::*;
pub(crate) use facts::*;
pub(crate) use guards::{condition_defined_names, condition_never_takes};
pub(crate) use module_scope::{
    check_class_member_flow, check_module_definite_assignment, walk_class,
};
pub(crate) use never_initialized::{
    begin_file as begin_never_initialized_file, collect_declared_names, enter_container,
    excludes_undefined, expression_container_flow, is_plainly_defined,
};
pub(crate) use unassigned_reads::is_unassigned_read;
use unassigned_reads::record_unassigned_read;

/// Marks what `condition` proves defined on its `when` edge, for the code the
/// edge leads to.
pub(crate) fn mark_condition_defined(
    condition: &ParsedExpression,
    when: bool,
    flow_state: &mut FunctionFlowState,
    ctx: &CheckerContext,
) {
    for name in condition_defined_names(condition, when, &|callee| predicate_parameter(callee, ctx)) {
        flow_state.mark_assigned(name);
    }
}

/// The argument position a callee's `param is T` predicate tests, when the
/// callee in scope declares one (not an `asserts` form, and not `this`).
pub(crate) fn predicate_parameter(callee: &str, ctx: &CheckerContext) -> Option<usize> {
    let symbol = ctx.symbols.get(callee).cloned().or_else(|| {
        ctx.module_value_fallback
            .as_ref()
            .and_then(|fallback| fallback.get(callee).cloned())
    })?;
    let signature = symbol.function_signature?;
    let signature = match &signature.return_type {
        Some(surge_ts_syntax::ParsedType::Predicate(_)) => signature,
        _ => signature.predicate_overload.clone()?,
    };
    let Some(surge_ts_syntax::ParsedType::Predicate(predicate)) = &signature.return_type else {
        return None;
    };
    if predicate.asserts || predicate.ty.is_none() || predicate.parameter_name == "this" {
        return None;
    }
    signature
        .parameter_names
        .iter()
        .position(|name| name.as_deref() == Some(predicate.parameter_name.as_str()))
}

/// A block-scoped binding whose own declaration is being evaluated (see
/// [`FunctionFlowState::initializing`]).
#[derive(Debug, Clone)]
pub(crate) struct InitializingBinding {
    name: Arc<str>,
    /// A read is also unassigned: the declared type does not assume the
    /// binding initialized.
    unassigned: bool,
    /// Where the declaration names the binding, when nothing but the
    /// initializer types it: a read then makes its type circular, which tsc's
    /// `reportCircularityError` reports there (TS7022).
    circular_at: Option<SyntaxTextSpan>,
    /// A circular `var`, which tsc types `any`: a read is neither early nor
    /// unassigned.
    circular_var: bool,
}

impl InitializingBinding {
    /// A `let`/`const` declaration: typed by its annotation when it has one
    /// (`unassigned` when that type excludes `undefined`), by its initializer
    /// otherwise. A binding a destructuring pattern declares keeps no trace of
    /// the pattern's annotation, so it is never taken for circular.
    pub(crate) fn declaration(
        variable: &surge_ts_syntax::ParsedVariableDeclaration,
        annotation_excludes_undefined: Option<bool>,
    ) -> Self {
        Self {
            name: Arc::from(variable.name.as_str()),
            unassigned: annotation_excludes_undefined.unwrap_or(false),
            circular_at: (annotation_excludes_undefined.is_none() && !variable.from_binding_pattern)
                .then_some(variable.name_span)
                .flatten(),
            circular_var: false,
        }
    }

    /// The binding a declaration holds while its initializer runs, if any:
    /// a `let`/`const` always, a `var` only once its check found the
    /// initializer circular (`circular_var_starts`).
    pub(crate) fn for_initializer(
        variable: &surge_ts_syntax::ParsedVariableDeclaration,
        annotation_excludes_undefined: impl FnOnce() -> Option<bool>,
        circular_var_starts: &[usize],
    ) -> Option<Self> {
        if variable.kind != surge_ts_syntax::ParsedVariableKind::Var {
            return Some(Self::declaration(variable, annotation_excludes_undefined()));
        }
        let circular = variable
            .name_span
            .is_some_and(|span| circular_var_starts.contains(&span.start));
        circular.then(|| Self {
            name: Arc::from(variable.name.as_str()),
            unassigned: false,
            circular_at: None,
            circular_var: true,
        })
    }

    fn for_head(name: &str, span: Option<SyntaxTextSpan>) -> Self {
        Self {
            name: Arc::from(name),
            unassigned: false,
            circular_at: span,
            circular_var: false,
        }
    }
}

/// A `for…in`/`for…of` head's own bindings, which its declaration holds while
/// the expression it iterates runs (tsc's
/// `isImmediatelyUsedInInitializerOfBlockScopedVariable`). A head takes no
/// annotation, so each is typed by that expression.
pub(crate) fn for_head_initializing(
    for_of_statement: &surge_ts_syntax::ParsedForOfStatement,
) -> Vec<InitializingBinding> {
    fn collect(binding: &surge_ts_syntax::ParsedBindingName, out: &mut Vec<InitializingBinding>) {
        use surge_ts_syntax::ParsedBindingName;
        match binding {
            ParsedBindingName::Identifier { name, span } => {
                out.push(InitializingBinding::for_head(name, *span));
            }
            ParsedBindingName::ObjectPattern(pattern) => {
                for element in &pattern.elements {
                    collect(&element.binding_name, out);
                }
                if let Some(rest) = &pattern.rest {
                    collect(rest, out);
                }
            }
            ParsedBindingName::ArrayPattern(pattern) => {
                for element in pattern.elements.iter().flatten() {
                    collect(element, out);
                }
                if let Some(rest) = &pattern.rest {
                    collect(rest, out);
                }
            }
            ParsedBindingName::Unsupported { .. } => {}
        }
    }
    let mut bindings = Vec::new();
    if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::BlockScoped {
        collect(&for_of_statement.binding_name, &mut bindings);
    }
    bindings
}

pub(crate) fn check_expression_flow(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    flow_state: &FunctionFlowState,
    statement_index: usize,
    ctx: &mut CheckerContext,
) -> FlowCheck {
    check_expression_flow_impl(expression, fallback_span, flow_state, statement_index, ctx)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FunctionBodyFlow {
    /// tsc's `hasExplicitReturn`: see [`ReturnFlowSummary::contains_return`].
    pub(crate) contains_return: bool,
    pub(crate) contains_return_with_value: bool,
    pub(crate) guarantees_value_return: bool,
    pub(crate) guarantees_exit: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FunctionFlowFacts {
    pub(crate) has_let_or_const: bool,
    pub(crate) has_uninitialized_let_or_const: bool,
    pub(crate) has_assignments: bool,
    pub(crate) has_identifier_reads: bool,
    pub(crate) has_future_block_scoped_declarations: bool,
    pub(crate) has_return_or_throw: bool,
    pub(crate) has_branching: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssignmentState {
    DeclaredUnassigned,
    Assigned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlowCheck {
    Clear,
    Blocked,
}

impl FlowCheck {
    pub(crate) fn is_blocked(self) -> bool {
        matches!(self, Self::Blocked)
    }
}

#[derive(Debug)]
pub(crate) struct FunctionFlowState {
    enabled: bool,
    tracked_local_count: usize,
    scopes: Vec<FlowScope>,
    branch_captures: Vec<FlowBranchCapture>,
    /// Maps a boolean `const`/`let` alias to the value identifiers its
    /// initializer guards (`const ok = error && isError(error) && …` →
    /// `["error"]`). An early-exit `if (!ok) return;` then narrows those
    /// identifiers in the fall-through, the way tsc tracks aliased conditions.
    /// Used only to drop a guarded genuine-`unknown` to the degradation
    /// sentinel so a later access is not a spurious `TS18046`.
    alias_guard_targets: HashMap<String, Vec<String>>,
    /// Maps a boolean `const` alias to the condition expression it was
    /// initialized from (`const ok = name !== null && name.length > 0`), so a
    /// later `if (ok)` narrows exactly as the written condition would. Only
    /// `const` bindings with a plainly boolean initializer are recorded, so the
    /// stored expression cannot go stale and the clone stays proportional to
    /// guard aliases rather than to every local.
    alias_guard_conditions: HashMap<String, Arc<ParsedExpression>>,
    /// Maps a `const` bound to a property reference (`const { direction } =
    /// opts`, `const kind = node.kind`) to that reference, so testing the alias
    /// narrows the object it came from — tsc's aliased-discriminant narrowing.
    discriminant_aliases: HashMap<String, Arc<ParsedExpression>>,
    /// Bindings typed by a type parameter whose constraint is a union or
    /// nullable. tsc substitutes that constraint for a read in a constraint
    /// position or under a non-generic contextual type
    /// (`getNarrowableTypeForReference`), where it may admit `undefined` and
    /// so is not reported; any other read is.
    pub(crate) constraint_exempt: std::collections::HashSet<Arc<str>>,
    /// A body surge does not otherwise type-check (a generic class member, a
    /// namespace function), walked for definite assignment alone: its own
    /// annotated `let`s are tracked from their written types.
    pub(crate) detached: bool,
    /// Every nested block is walked at the index of the top-level statement
    /// that holds it (the module-level pass), so a declaration still pending in
    /// an enclosing scope is compared against that index directly: a read of it
    /// from a nested block or a class static block is in its temporal dead
    /// zone too.
    pub(crate) shared_statement_index: bool,
    /// Locals a guard on the way to the expression being walked proves are
    /// not `undefined` (`x` in `typeof x === "string" && x`; see `guards`).
    guarded_defined: std::cell::RefCell<Vec<Arc<str>>>,
    /// How many enclosing branches no flow reaches (`b` in `true ? a : b`),
    /// where tsc reads a local as its declared type.
    unreachable_depth: std::cell::Cell<u32>,
    /// The body's enums, lowered to `const` objects, and whether each is a
    /// `const enum`. Reading one before its declaration is tsc's TS2450, not a
    /// variable's TS2448 and TS2454 (`checkResolvedBlockScopedVariable`).
    enum_objects: HashMap<Arc<str>, bool>,
    /// The block-scoped bindings whose own declaration is being evaluated — a
    /// `let`/`const` initializer, a `for…in`/`for…of` head's expression. A read
    /// of one there is a use before the declaration (tsc's
    /// `isImmediatelyUsedInInitializerOfBlockScopedVariable`); TS2454 goes with
    /// it only when the binding's declared type does not assume it initialized,
    /// and an unannotated one is circular, so `any`.
    initializing: Vec<InitializingBinding>,
    /// The bindings the container assigns anywhere, nested functions included
    /// (tsc's `isSymbolAssigned`); a parameter or `let` among them is not a
    /// constant reference.
    assigned_bindings: Arc<std::collections::HashSet<Arc<str>>>,
    /// The container's hoisted `var`s (see [`FunctionFlowState::hoist_vars`]).
    hoisted_vars: std::collections::HashSet<Arc<str>>,
    /// The container is a global script's top level, whose bindings are
    /// never mutable locals.
    script_top_level: bool,
    /// Walking a computed method or accessor name, whose flow container is the
    /// member (see [`FunctionFlowState::assumed_initialized_in_member_key`]).
    pub(crate) in_member_key: std::cell::Cell<bool>,
}

/// What [`FunctionFlowState::begin_initializer`] changed, for
/// [`FunctionFlowState::end_initializer`] to take back.
pub(crate) struct InitializerMark {
    initializing: usize,
    enabled: bool,
}

impl Clone for FunctionFlowState {
    fn clone(&self) -> Self {
        record_flow_state_clone_count(
            self.scopes
                .iter()
                .map(|scope| scope.locals.len())
                .sum::<usize>(),
        );

        Self {
            enabled: self.enabled,
            tracked_local_count: self.tracked_local_count,
            scopes: self.scopes.clone(),
            branch_captures: self.branch_captures.clone(),
            alias_guard_targets: self.alias_guard_targets.clone(),
            alias_guard_conditions: self.alias_guard_conditions.clone(),
            discriminant_aliases: self.discriminant_aliases.clone(),
            constraint_exempt: self.constraint_exempt.clone(),
            detached: self.detached,
            shared_statement_index: self.shared_statement_index,
            guarded_defined: self.guarded_defined.clone(),
            unreachable_depth: self.unreachable_depth.clone(),
            enum_objects: self.enum_objects.clone(),
            initializing: self.initializing.clone(),
            assigned_bindings: Arc::clone(&self.assigned_bindings),
            hoisted_vars: self.hoisted_vars.clone(),
            script_top_level: self.script_top_level,
            in_member_key: self.in_member_key.clone(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct FlowBranchDelta {
    changes: HashMap<usize, HashMap<Arc<str>, AssignmentState>>,
    changed_local_count: usize,
    pub(crate) continues: bool,
}

impl FlowBranchDelta {
    fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    fn get(&self, scope_index: usize, name: &str) -> Option<AssignmentState> {
        self.changes
            .get(&scope_index)
            .and_then(|scope_changes| scope_changes.get(name))
            .copied()
    }
}

#[derive(Debug, Default, Clone)]
struct FlowBranchCapture {
    scope_count: usize,
    tracked_local_count: usize,
    previous_states: HashMap<usize, HashMap<Arc<str>, Option<AssignmentState>>>,
}

#[derive(Debug, Default)]
struct FlowScope {
    locals: HashMap<Arc<str>, AssignmentState>,
    future_block_scoped_declarations: HashMap<Arc<str>, FutureDeclaration>,
    tracked_count: usize,
}

impl Clone for FlowScope {
    fn clone(&self) -> Self {
        Self {
            locals: self.locals.clone(),
            future_block_scoped_declarations: self.future_block_scoped_declarations.clone(),
            tracked_count: self.tracked_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowReadOutcome {
    Unresolved,
    UseBeforeDeclaration {
        unassigned: bool,
        circular_at: Option<SyntaxTextSpan>,
    },
    Declared(AssignmentState),
}

pub(crate) fn analyze_function_body_flow(body: &[ParsedFunctionBodyStatement]) -> FunctionBodyFlow {
    let summary = summarize_function_body_flow(body);
    FunctionBodyFlow {
        contains_return: summary.contains_return,
        contains_return_with_value: summary.contains_return_with_value,
        guarantees_value_return: summary.guarantees_value_return,
        guarantees_exit: summary.guarantees_exit,
    }
}

pub(crate) fn collect_function_flow_facts(
    body: &[ParsedFunctionBodyStatement],
) -> FunctionFlowFacts {
    let mut facts = FunctionFlowFacts::default();
    collect_function_flow_facts_from_body(body, &mut facts);
    facts
}

pub(crate) fn mark_assignment_state(target_name: &str, flow_state: &mut FunctionFlowState) {
    flow_state.mark_assigned(target_name);
}

impl FunctionFlowState {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            tracked_local_count: 0,
            scopes: Vec::new(),
            branch_captures: Vec::new(),
            alias_guard_targets: HashMap::new(),
            alias_guard_conditions: HashMap::new(),
            discriminant_aliases: HashMap::new(),
            constraint_exempt: std::collections::HashSet::new(),
            detached: false,
            shared_statement_index: false,
            guarded_defined: std::cell::RefCell::new(Vec::new()),
            unreachable_depth: std::cell::Cell::new(0),
            enum_objects: HashMap::new(),
            initializing: Vec::new(),
            assigned_bindings: Arc::default(),
            hoisted_vars: std::collections::HashSet::new(),
            script_top_level: false,
            in_member_key: std::cell::Cell::new(false),
        }
    }

    /// Records the enums `body` declares at its own level.
    pub(crate) fn record_enum_objects(&mut self, body: &[surge_ts_syntax::ParsedFunctionBodyStatement]) {
        use surge_ts_syntax::ParsedFunctionBodyStatement;
        for statement in body {
            if let ParsedFunctionBodyStatement::VariableDeclaration(variable) = statement
                && variable.is_enum_object
            {
                let is_const = body.iter().any(|statement| {
                    matches!(
                        statement,
                        ParsedFunctionBodyStatement::TypeAlias(alias)
                            if alias.enum_name.as_deref() == Some(variable.name.as_str()) && alias.enum_is_const
                    )
                });
                self.enum_objects.insert(variable.name.as_str().into(), is_const);
            }
        }
    }

    /// Whether `name` is one of the body's enums, and if so whether it is a
    /// `const enum`.
    pub(crate) fn enum_object(&self, name: &str) -> Option<bool> {
        self.enum_objects.get(name).copied()
    }

    pub(crate) fn set_assigned_bindings(&mut self, names: std::collections::HashSet<Arc<str>>) {
        self.assigned_bindings = Arc::new(names);
    }

    pub(crate) fn assigned_bindings(&self) -> Arc<std::collections::HashSet<Arc<str>>> {
        Arc::clone(&self.assigned_bindings)
    }

    pub(crate) fn is_binding_assigned(&self, name: &str) -> bool {
        self.assigned_bindings.contains(name)
    }

    /// Marks `names` as declared by the initializer walked next (see
    /// [`FunctionFlowState::initializing`]). The walk runs even where nothing
    /// else is tracked, so a state that tracks nothing is enabled until
    /// [`Self::end_initializer`].
    pub(crate) fn begin_initializer(
        &mut self,
        names: impl IntoIterator<Item = InitializingBinding>,
    ) -> InitializerMark {
        let mark = InitializerMark {
            initializing: self.initializing.len(),
            enabled: self.enabled,
        };
        self.initializing.extend(names);
        self.enabled = true;
        self.tracked_local_count += self.initializing.len() - mark.initializing;
        mark
    }

    pub(crate) fn end_initializer(&mut self, mark: InitializerMark) {
        self.tracked_local_count -= self.initializing.len() - mark.initializing;
        self.initializing.truncate(mark.initializing);
        self.enabled = mark.enabled;
    }

    /// Adds the locals a guard proves defined for the expression walked next;
    /// returns the mark [`Self::restore_guarded_defined`] takes back.
    pub(crate) fn push_guarded_defined(&self, names: &[&str]) -> usize {
        let mut guarded = self.guarded_defined.borrow_mut();
        let mark = guarded.len();
        guarded.extend(names.iter().map(|name| Arc::<str>::from(*name)));
        mark
    }

    pub(crate) fn restore_guarded_defined(&self, mark: usize) {
        self.guarded_defined.borrow_mut().truncate(mark);
    }

    pub(crate) fn enter_unreachable(&self, unreachable: bool) {
        if unreachable {
            self.unreachable_depth.set(self.unreachable_depth.get() + 1);
        }
    }

    pub(crate) fn exit_unreachable(&self, unreachable: bool) {
        if unreachable {
            self.unreachable_depth.set(self.unreachable_depth.get() - 1);
        }
    }

    /// Whether tsc's flow type for `name` here cannot hold the `undefined` an
    /// unassigned local starts with: a guard stripped it, or no flow reaches.
    fn reads_as_defined(&self, name: &str) -> bool {
        self.unreachable_depth.get() > 0
            || self.guarded_defined.borrow().iter().any(|guarded| &**guarded == name)
    }

    /// Opens the container's own scope holding its hoisted `var`s as declared
    /// but unassigned; their declarations later mark them assigned rather than
    /// redeclaring them in whatever block they sit in.
    pub(crate) fn hoist_vars(&mut self, names: Vec<Arc<str>>) {
        if names.is_empty() {
            return;
        }
        self.enabled = true;
        self.push_scope(HashMap::new());
        for name in names {
            self.hoisted_vars.insert(Arc::clone(&name));
            self.declare_current(name, AssignmentState::DeclaredUnassigned);
        }
    }

    /// tsc's `checkIdentifier` for a read in a computed method or accessor
    /// name: the member is its flow container, so an enclosing binding is an
    /// outer variable, assumed initialized unless the analysis climbs to its
    /// own container — a `const`, or a `let`/parameter past its last
    /// assignment — or it is a `let` never assigned at all. A `var` is
    /// neither, nor is anything a global script declares at its top level; a
    /// `let` assigned anywhere is taken as assigned after the read.
    pub(crate) fn assumed_initialized_in_member_key(&self, name: &str) -> bool {
        self.in_member_key.get()
            && (self.script_top_level
                || self.hoisted_vars.contains(name)
                || self.assigned_bindings.contains(name))
    }

    /// The assignment state of every binding in the outermost scope — the
    /// container's own bindings, which outlive any branch.
    pub(crate) fn root_states(&self) -> Vec<(Arc<str>, AssignmentState)> {
        self.scopes
            .first()
            .map(|scope| {
                scope
                    .locals
                    .iter()
                    .map(|(name, state)| (Arc::clone(name), *state))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn root_state(&self, name: &str) -> Option<AssignmentState> {
        self.scopes
            .first()
            .and_then(|scope| scope.locals.get(name).copied())
    }

    pub(crate) fn set_root_state(&mut self, name: &str, state: AssignmentState) {
        if let Some(slot) = self
            .scopes
            .first_mut()
            .and_then(|scope| scope.locals.get_mut(name))
        {
            *slot = state;
        }
    }

    pub(crate) fn tracked_local_count(&self) -> usize {
        self.tracked_local_count
    }

    /// Records that the boolean alias `name` guards `targets` (see
    /// [`FunctionFlowState::alias_guard_targets`] field docs).
    pub(crate) fn record_alias_guard_targets(&mut self, name: String, targets: Vec<String>) {
        if targets.is_empty() {
            return;
        }
        self.alias_guard_targets.insert(name, targets);
    }

    pub(crate) fn alias_guard_targets(&self, name: &str) -> Option<&[String]> {
        self.alias_guard_targets.get(name).map(Vec::as_slice)
    }

    /// Records the condition a boolean `const` alias was initialized from (see
    /// [`FunctionFlowState::alias_guard_conditions`] field docs).
    pub(crate) fn record_alias_guard_condition(
        &mut self,
        name: String,
        condition: Arc<ParsedExpression>,
    ) {
        self.alias_guard_conditions.insert(name, condition);
    }

    pub(crate) fn alias_guard_condition(&self, name: &str) -> Option<Arc<ParsedExpression>> {
        self.alias_guard_conditions.get(name).cloned()
    }

    /// Records the property reference a `const` alias was bound to (see
    /// [`FunctionFlowState::discriminant_aliases`] field docs).
    pub(crate) fn record_discriminant_alias(
        &mut self,
        name: String,
        reference: Arc<ParsedExpression>,
    ) {
        self.discriminant_aliases.insert(name, reference);
    }

    pub(crate) fn discriminant_alias(&self, name: &str) -> Option<&ParsedExpression> {
        self.discriminant_aliases.get(name).map(Arc::as_ref)
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn begin_branch_capture(&mut self) {
        if !self.enabled {
            return;
        }

        record_flow_state_full_clone_avoided_count();
        self.branch_captures.push(FlowBranchCapture {
            scope_count: self.scopes.len(),
            tracked_local_count: self.tracked_local_count,
            previous_states: HashMap::new(),
        });
    }

    pub(crate) fn finish_branch_capture(&mut self) -> FlowBranchDelta {
        if !self.enabled {
            return FlowBranchDelta::default();
        }

        let Some(capture) = self.branch_captures.pop() else {
            return FlowBranchDelta::default();
        };

        let mut delta = FlowBranchDelta::default();

        for (scope_index, scope_changes) in &capture.previous_states {
            if *scope_index >= capture.scope_count {
                continue;
            }

            let Some(scope) = self.scopes.get(*scope_index) else {
                continue;
            };

            for name in scope_changes.keys() {
                if let Some(current_state) = scope.locals.get(name).copied() {
                    delta
                        .changes
                        .entry(*scope_index)
                        .or_default()
                        .insert(name.clone(), current_state);
                    delta.changed_local_count += 1;
                }
            }
        }

        if delta.is_empty() {
            record_flow_branch_empty_delta_count();
        } else {
            record_flow_branch_changed_local_count(delta.changed_local_count);
        }

        self.restore_branch_capture(capture);
        delta
    }

    pub(crate) fn push_scope(
        &mut self,
        future_block_scoped_declarations: HashMap<Arc<str>, FutureDeclaration>,
    ) {
        if !self.enabled {
            return;
        }

        let tracked_count = future_block_scoped_declarations.len();
        self.tracked_local_count += tracked_count;
        record_flow_scope_push_count();
        self.scopes.push(FlowScope {
            locals: HashMap::new(),
            future_block_scoped_declarations,
            tracked_count,
        });
    }

    pub(crate) fn pop_scope(&mut self) {
        if !self.enabled {
            return;
        }

        if let Some(scope) = self.scopes.pop() {
            self.tracked_local_count = self.tracked_local_count.saturating_sub(scope.tracked_count);
            record_flow_scope_pop_count();
        }
    }

    pub(crate) fn declare_current(&mut self, name: impl Into<Arc<str>>, state: AssignmentState) {
        if !self.enabled {
            return;
        }

        let name = name.into();
        let scope_index = self
            .scopes
            .len()
            .checked_sub(1)
            .expect("flow state must contain at least one scope");
        let scope = self
            .scopes
            .last_mut()
            .expect("flow state must contain at least one scope");
        let previous_state = scope.locals.get(&name).copied();
        if previous_state == Some(state) {
            return;
        }

        let record_name = self
            .branch_captures
            .last()
            .is_some_and(|capture| scope_index < capture.scope_count)
            .then(|| name.clone());

        scope.locals.insert(name, state);

        if let Some(capture) = self.branch_captures.last_mut() {
            if scope_index < capture.scope_count {
                Self::record_branch_change(
                    capture,
                    scope_index,
                    record_name.expect("branch capture name should be recorded"),
                    previous_state,
                );
            }
        }

        if previous_state.is_none() {
            scope.tracked_count += 1;
            self.tracked_local_count += 1;
        }
    }

    pub(crate) fn mark_assigned(&mut self, name: &str) {
        if !self.enabled || self.tracked_local_count == 0 {
            return;
        }

        let mut found_scope_index = None;
        for (scope_index, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(current_state) = scope.locals.get(name).copied() {
                if current_state == AssignmentState::Assigned {
                    return;
                }
                found_scope_index = Some(scope_index);
                break;
            }
        }

        let Some(scope_index) = found_scope_index else {
            return;
        };

        let owned_name: Arc<str> = name.into();
        let record_name = self
            .branch_captures
            .last()
            .is_some_and(|capture| scope_index < capture.scope_count)
            .then(|| owned_name.clone());

        if let Some(scope) = self.scopes.get_mut(scope_index) {
            let previous_state = scope.locals.insert(owned_name, AssignmentState::Assigned);
            if let Some(capture) = self.branch_captures.last_mut() {
                if scope_index < capture.scope_count {
                    Self::record_branch_change(
                        capture,
                        scope_index,
                        record_name.expect("branch capture name should be recorded"),
                        previous_state,
                    );
                }
            }
        }
    }

    fn restore_branch_capture(&mut self, capture: FlowBranchCapture) {
        self.restore_scope_count(capture.scope_count);

        for (scope_index, scope_changes) in capture.previous_states {
            if scope_index >= self.scopes.len() {
                continue;
            }

            let Some(scope) = self.scopes.get_mut(scope_index) else {
                continue;
            };

            for (name, previous_state) in scope_changes {
                match previous_state {
                    Some(previous_state) => {
                        scope.locals.insert(name, previous_state);
                    }
                    None => {
                        if scope.locals.remove(&name).is_some() {
                            scope.tracked_count = scope.tracked_count.saturating_sub(1);
                        }
                    }
                }
            }
        }

        self.tracked_local_count = capture.tracked_local_count;
    }

    fn restore_scope_count(&mut self, scope_count: usize) {
        while self.scopes.len() > scope_count {
            let Some(scope) = self.scopes.pop() else {
                break;
            };
            self.tracked_local_count = self.tracked_local_count.saturating_sub(scope.tracked_count);
        }
    }

    fn record_branch_change(
        capture: &mut FlowBranchCapture,
        scope_index: usize,
        name: Arc<str>,
        previous_state: Option<AssignmentState>,
    ) {
        capture
            .previous_states
            .entry(scope_index)
            .or_default()
            .entry(name)
            .or_insert(previous_state);
    }

    fn read_identifier(&self, name: &str, statement_index: usize) -> FlowReadOutcome {
        if !self.enabled || self.tracked_local_count == 0 {
            return FlowReadOutcome::Unresolved;
        }

        if let Some(binding) = self
            .initializing
            .iter()
            .rev()
            .find(|binding| &*binding.name == name)
        {
            if binding.circular_var {
                return FlowReadOutcome::Declared(AssignmentState::Assigned);
            }
            return FlowReadOutcome::UseBeforeDeclaration {
                unassigned: binding.unassigned,
                circular_at: binding.circular_at,
            };
        }

        let Some(current_scope) = self.scopes.last() else {
            return FlowReadOutcome::Unresolved;
        };

        let mut lookup_steps = 1usize;

        if let Some(state) = current_scope.locals.get(name).copied() {
            record_flow_read_lookup_count(lookup_steps);
            return FlowReadOutcome::Declared(state);
        }

        if let Some(declaration) = current_scope
            .future_block_scoped_declarations
            .get(name)
            .filter(|declaration| statement_index < declaration.index)
        {
            record_flow_read_lookup_count(lookup_steps);
            return FlowReadOutcome::UseBeforeDeclaration {
                unassigned: declaration.unassigned,
                circular_at: None,
            };
        }

        for scope in self.scopes.iter().rev().skip(1) {
            lookup_steps += 1;
            if let Some(state) = scope.locals.get(name).copied() {
                record_flow_read_lookup_count(lookup_steps);
                return FlowReadOutcome::Declared(state);
            }
            if self.shared_statement_index
                && let Some(declaration) = scope
                    .future_block_scoped_declarations
                    .get(name)
                    .filter(|declaration| statement_index < declaration.index)
            {
                record_flow_read_lookup_count(lookup_steps);
                return FlowReadOutcome::UseBeforeDeclaration {
                    unassigned: declaration.unassigned,
                    circular_at: None,
                };
            }
        }

        record_flow_read_lookup_count(lookup_steps);
        FlowReadOutcome::Unresolved
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReturnFlowSummary {
    /// A `return` statement some flow reaches, with or without a value; a
    /// `throw` is not one (tsc's `hasExplicitReturn`, which only
    /// `bindReturnStatement` sets).
    contains_return: bool,
    /// A `return <expr>` appears on some path, and a `throw` does *not* set
    /// this. Distinguishes a function that genuinely returns a value (subject
    /// to `noImplicitReturns`/TS7030) from one that only throws (whose inferred
    /// return type is `void`).
    contains_return_with_value: bool,
    contains_throw: bool,
    guarantees_value_return: bool,
    /// Every path through the body leaves the current straight-line flow without
    /// falling through — via `return`/`throw` (exits the function) or
    /// `continue`/`break` (exits the enclosing loop/switch). Strictly weaker than
    /// `guarantees_value_return`: a bare `return;` or a `continue;` sets this but
    /// not that. Used to narrow code after `if (cond) <diverge>;` guards.
    guarantees_exit: bool,
}
