//! `strictPropertyInitialization` (TS2564), ported from
//! `checkPropertyInitialization` in `tsc/internal/checker/checker.go`.
//!
//! An instance property with no initializer must be definitely assigned in the
//! constructor. tsc answers "definitely assigned" with the same flow graph it
//! uses everywhere, by asking for the type of a synthesized `this.p` reference
//! at the constructor's end and checking it no longer includes `undefined`.
//! There is no such graph here, so the reachability rules are re-derived over
//! the statement list the same way [`crate::flow`] derives `guarantees_exit`:
//! a statement list assigns the property if some statement assigns it before
//! any statement that cannot complete.

use surge_ts_syntax::{
    ParsedClassDeclaration, ParsedClassMember, ParsedExpression, ParsedFunctionBodyStatement, ParsedType,
};
use surge_ts_types::Type;

use surge_ts_diagnostics::Diagnostic;

use crate::context::{CheckerContext, convert_span};
use crate::flow::{analyze_function_body_flow, condition_never_takes};
use crate::infer::map_parsed_type;

/// Whether `body` assigns `this.<name>` on every path that reaches its end.
pub(super) fn body_assigns(body: &[ParsedFunctionBodyStatement], name: &str) -> bool {
    for statement in body {
        if statement_assigns(statement, name) {
            return true;
        }
        // Nothing after a statement that cannot complete is on the path to the
        // constructor's end, so the property is not assigned along it.
        if statement_guarantees_exit(statement) {
            return false;
        }
    }
    false
}

fn statement_assigns(statement: &ParsedFunctionBodyStatement, name: &str) -> bool {
    match statement {
        ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => {
            assignment.property_name == name || expression_assigns(&assignment.value, name)
        }
        ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
            writes_this_member(&assignment.target, name) || expression_assigns(&assignment.value, name)
        }
        ParsedFunctionBodyStatement::Expression(expression) => expression_assigns(expression, name),
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => variable
            .initializer
            .as_ref()
            .is_some_and(|initializer| expression_assigns(initializer, name)),
        ParsedFunctionBodyStatement::Block(body) => body_assigns(body, name),
        ParsedFunctionBodyStatement::If(if_statement) => {
            // A branch that cannot complete leaves the other one as the only
            // path to the end, so it alone decides.
            let then_decides = body_assigns(&if_statement.then_body, name)
                || body_guarantees_exit(&if_statement.then_body);
            let else_decides = body_assigns(&if_statement.else_body, name)
                || body_guarantees_exit(&if_statement.else_body);
            !if_statement.else_body.is_empty() && then_decides && else_decides
        }
        // A loop whose body is not guaranteed to run assigns nothing; `do … while`
        // lowers with `runs_at_least_once` set and does.
        ParsedFunctionBodyStatement::While(while_statement) => {
            while_statement.runs_at_least_once && body_assigns(&while_statement.body, name)
        }
        // A `try` body may be abandoned part-way, so only `finally` is on every
        // path out of the statement.
        ParsedFunctionBodyStatement::Try(try_statement) => {
            body_assigns(&try_statement.finalizer, name)
        }
        _ => false,
    }
}

/// Whether evaluating `expression` writes `this.<name>` on every path through
/// it (`const r = (this.r = f())`, `this[A] = this[B] = v`).
fn expression_assigns(expression: &ParsedExpression, name: &str) -> bool {
    crate::flow::expression_member_assignments(expression)
        .into_iter()
        .any(|(write, conditional)| {
            !conditional
                && matches!(write, ParsedExpression::MemberAssignment(assignment)
                    if writes_this_member(&assignment.target, name))
        })
}

/// Whether `target` is the member `name` of `this`. `this[k]` writes the
/// member a computed name `[k]` declares when `k` spells the same entity
/// name: tsc matches the two references by the name that expression resolves
/// to (`getAccessedPropertyName`).
fn writes_this_member(target: &ParsedExpression, name: &str) -> bool {
    match target {
        ParsedExpression::PropertyAccess { object, property_name, .. } => {
            matches!(object.as_ref(), ParsedExpression::This { .. }) && property_name == name
        }
        ParsedExpression::ElementAccess { object, index, .. } => {
            matches!(object.as_ref(), ParsedExpression::This { .. })
                && entity_name_path(index).is_some_and(|path| format!("[{path}]") == name)
        }
        _ => false,
    }
}

/// The dotted name `expression` spells (`a`, `E.A`), as a computed member name
/// writes it between its brackets.
fn entity_name_path(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        } => Some(format!("{}.{property_name}", entity_name_path(object)?)),
        _ => None,
    }
}

fn statement_guarantees_exit(statement: &ParsedFunctionBodyStatement) -> bool {
    analyze_function_body_flow(std::slice::from_ref(statement)).guarantees_exit
}

fn body_guarantees_exit(body: &[ParsedFunctionBodyStatement]) -> bool {
    body.iter().any(statement_guarantees_exit)
}

/// Whether some `return` in `body` runs before `this.<name>` is assigned. tsc
/// reads the property at the constructor's `ReturnFlowNode`, which a `return`
/// reaches as well as the end of the body, so one taken early leaves it
/// unassigned there.
fn returns_before_assignment(body: &[ParsedFunctionBodyStatement], name: &str) -> bool {
    for statement in body {
        if statement_returns_before_assignment(statement, name) {
            return true;
        }
        if statement_assigns(statement, name) || statement_guarantees_exit(statement) {
            return false;
        }
    }
    false
}

fn statement_returns_before_assignment(statement: &ParsedFunctionBodyStatement, name: &str) -> bool {
    match statement {
        ParsedFunctionBodyStatement::Return(_) => true,
        ParsedFunctionBodyStatement::Block(body) => returns_before_assignment(body, name),
        // A branch a literal condition never takes is unreachable.
        ParsedFunctionBodyStatement::If(if_statement) => {
            (!condition_never_takes(&if_statement.condition, true)
                && returns_before_assignment(&if_statement.then_body, name))
                || (!condition_never_takes(&if_statement.condition, false)
                    && returns_before_assignment(&if_statement.else_body, name))
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            (while_statement.runs_at_least_once || !condition_never_takes(&while_statement.condition, true))
                && returns_before_assignment(&while_statement.body, name)
        }
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
            returns_before_assignment(&for_of_statement.body, name)
        }
        // Every case can be entered straight from the discriminant.
        ParsedFunctionBodyStatement::Switch(switch_statement) => switch_statement
            .cases
            .iter()
            .any(|case| returns_before_assignment(&case.consequent, name)),
        // A `return` in the `try` block or the handler runs the `finally` on
        // its way out.
        ParsedFunctionBodyStatement::Try(try_statement) => {
            (!body_assigns(&try_statement.finalizer, name)
                && (returns_before_assignment(&try_statement.block, name)
                    || try_statement
                        .handler
                        .as_ref()
                        .is_some_and(|handler| returns_before_assignment(&handler.body, name))))
                || returns_before_assignment(&try_statement.finalizer, name)
        }
        _ => false,
    }
}

/// The constructor that carries a body. Overload signatures declare no
/// statements, so only the implementation can initialize anything.
fn constructor_body(class: &ParsedClassDeclaration) -> Option<&[ParsedFunctionBodyStatement]> {
    class.members.iter().find_map(|member| match member {
        ParsedClassMember::Constructor(constructor) if !constructor.body.is_empty() => {
            Some(constructor.body.as_slice())
        }
        _ => None,
    })
}

/// tsc exempts a property whose type already admits the uninitialized value:
/// `any`, `unknown`, or anything including `undefined`. A type surge failed to
/// model is exempt too — an unresolved annotation is not evidence of a missing
/// initializer.
fn type_exempts_initialization(declared: &ParsedType, ctx: &mut CheckerContext) -> bool {
    // A type parameter is a real declared type, not a gap: tsc reports the
    // uninitialized `p: T` like any other. Surge resolves one to `Unknown`
    // inside the class's scope, which the wide-type exemption below would
    // otherwise swallow.
    if let ParsedType::Named(named) = declared
        && named.type_arguments.is_empty()
        && ctx.type_parameter_in_scope(&named.name)
    {
        return false;
    }
    // Nor is a conditional type over one, which `getConditionalType` defers
    // whatever its branches are.
    if let ParsedType::Conditional(conditional) = declared
        && let ParsedType::Named(checked) = conditional.check_type.as_ref()
        && checked.type_arguments.is_empty()
        && ctx.type_parameter_in_scope(&checked.name)
    {
        return false;
    }
    let ty = map_parsed_type(declared.clone(), ctx);
    contains_undefined_or_wide(&ty)
}

fn contains_undefined_or_wide(ty: &Type) -> bool {
    match ty {
        Type::Any
        | Type::GenuineUnknown
        | Type::Unknown
        | Type::ErrorType
        | Type::Undefined
        | Type::Void => true,
        Type::Union(union) => union.types().iter().any(contains_undefined_or_wide),
        _ => false,
    }
}

pub(crate) fn check_property_initialization(
    class: &ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    // tsc checks property initialization only together with `strictNullChecks`.
    if !ctx.options.strict_property_initialization
        || !ctx.options.strict_null_checks
        || class.is_declare
    {
        return;
    }

    let constructor = constructor_body(class).map(<[_]>::to_vec);

    // A class type parameter is in scope for its members' annotations. Without
    // this the annotation is resolved in the enclosing scope, where the
    // parameter is not a name at all, and `map_parsed_type` reports the
    // *resolution* failure (TS2304 "Cannot find name 'T'") — a diagnostic about
    // this check's own lookup, on a class that is perfectly well typed.
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        check_declared_properties(class, constructor.as_deref(), ctx)
    });
}

fn check_declared_properties(
    class: &ParsedClassDeclaration,
    constructor: Option<&[ParsedFunctionBodyStatement]>,
    ctx: &mut CheckerContext,
) {
    for member in &class.members {
        let ParsedClassMember::Property(property) = member else {
            continue;
        };
        if property.is_static
            || property.has_literal_name
            || property.is_abstract
            || property.is_declare
            || property.has_definite_assertion
            || property.optional
            || property.initializer.is_some()
        {
            continue;
        }
        // An unannotated property has no declared type to inspect; surge types
        // it `any`, which is exempt in tsc for the same reason.
        let Some(declared) = property.initialization_type.as_ref().or(property.declared_type.as_ref()) else {
            continue;
        };
        if type_exempts_initialization(declared, ctx) {
            continue;
        }
        if constructor.is_some_and(|body| {
            body_assigns(body, &property.name) && !returns_before_assignment(body, &property.name)
        }) {
            continue;
        }

        let diagnostic = Diagnostic::ts2564(
            surge_ts_types::private_name::display(&property.name),
            ctx.file_name.clone(),
        );
        ctx.push(match property.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}
