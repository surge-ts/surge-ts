//! Property-call and optional-property-call checking.

use super::*;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedCallArgument, ParsedExpression, ParsedType, TextSpan as SyntaxTextSpan,
};
use surge_ts_types::{FunctionType, Type, union_type};

use crate::checks::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type};
use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::SymbolTable;

/// Whether a member reads as the permissive shape a namespace value object gives
/// every member (`any`, or a function returning `any`). Used as the gate before
/// the qualified `ns.member` lookup, which must not run on every property call.
pub(crate) fn is_permissive_member_type(property_type: &Type) -> bool {
    match property_type {
        Type::Any => true,
        Type::Function(function_type) => *function_type.return_type() == Type::Any,
        _ => false,
    }
}

/// Routes `ns.member(...)` through the qualified `ns.member` binding when one
/// exists. `None` means no such binding — the caller keeps its own answer.
#[allow(clippy::too_many_arguments)]
fn try_qualified_namespace_call(
    object_name: &str,
    property_name: &str,
    callee_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Option<Type>> {
    let qualified_name = format!("{object_name}.{property_name}");
    crate::infer::expression::qualified_namespace_member(&qualified_name, symbols, ctx)?;
    Some(crate::checks::call::check_call_like(
        &qualified_name,
        callee_span,
        call_span,
        type_arguments,
        arguments,
        symbols,
        ctx,
    ))
}

/// tsc still checks a call's argument expressions when the callee is `any`, so
/// their own errors surface (an untyped callback parameter is TS7006/TS7031, a
/// bad reference inside the callback body is still reported). The symmetric
/// call-position arm lives in `check_call_like_with_expected_type`.
///
/// Implicit-any is reported only when the receiver's `any` is the source's, not
/// surge's — see [`receiver_any_is_genuine`]. Everything else in the argument
/// (an unresolved name, a missing member, a mismatched body) is reported either
/// way, since those do not depend on the callback's parameter types.
pub(super) fn evaluate_arguments_context_free(
    object: &ParsedExpression,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let genuine = receiver_any_is_genuine(object, symbols, ctx);
    let saved_depth = ctx.degraded_expected_type_depth;
    if genuine {
        // tsc has no contextual parameter type here either, so it *does* report
        // these parameters. An enclosing suppression — a builder chain surge
        // could not model further out — must not hide that: the provenance of
        // this receiver is decided, and it outranks the ambient guess.
        ctx.degraded_expected_type_depth = 0;
    } else {
        ctx.degraded_expected_type_depth += 1;
    }
    for argument in arguments {
        let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
    }
    ctx.degraded_expected_type_depth = saved_depth;
}

/// `resolveErrorCall`: the type arguments and arguments of a call on tsc's
/// error type are still checked.
pub(super) fn check_error_call_operands(
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    super::check_untyped_call_type_arguments(type_arguments, ctx);
    evaluate_arguments_on_error_type(arguments, symbols, ctx);
}

/// Arguments of a call on tsc's error type. `resolveErrorCall` checks each one
/// with no contextual type, so a callback's parameters are implicit `any`
/// whatever suppression an enclosing expression set up.
pub(super) fn evaluate_arguments_on_error_type(
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let saved_depth = ctx.degraded_expected_type_depth;
    ctx.degraded_expected_type_depth = 0;
    for argument in arguments {
        let _ = evaluate_expression(&argument.expression, argument.span, symbols, ctx);
    }
    ctx.degraded_expected_type_depth = saved_depth;
}

/// Whether an `any`-typed receiver is `any` because the *source* says so, rather
/// than because surge gave up partway along the chain.
///
/// Genuine: a binding stubbed for an import whose module was reported
/// unresolved, which tsc types as its error type. A callback passed to a call on
/// it really has no contextual type, so TS7006/TS7031 on its parameters is what
/// tsc reports.
///
/// Not genuine: a chain that started from a typed binding and collapsed to `any`
/// midway (a generic builder surge could not model). tsc still contextually
/// types those callbacks, so reporting implicit-any there describes surge's gap
/// rather than the source.
pub(crate) fn receiver_any_is_genuine(
    object: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> bool {
    match object {
        ParsedExpression::Identifier { name, .. } => name_is_genuine_any(name, symbols, ctx),
        // A bare call and an index off a bare name both name their base
        // directly rather than nesting an `Identifier` node.
        ParsedExpression::Call { callee_name, .. } => {
            name_is_genuine_any(callee_name, symbols, ctx)
        }
        ParsedExpression::IndexAccess { object_name, .. } => {
            name_is_genuine_any(object_name, symbols, ctx)
        }
        // Walk the chain to the binding it started from: `p.input(x).query(cb)`
        // is genuine exactly when `p` is.
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. }
        | ParsedExpression::PropertyCall { object, .. }
        | ParsedExpression::OptionalPropertyCall { object, .. }
        | ParsedExpression::ElementAccess { object, .. } => {
            receiver_any_is_genuine(object, symbols, ctx)
        }
        // Forms that hand the operand's type straight back: `await any`,
        // `any!`, and an index off one are all still `any` to tsc. A type
        // assertion is deliberately absent — `x as Foo` states a real type.
        ParsedExpression::Await { operand, .. } => {
            receiver_any_is_genuine(operand, symbols, ctx)
        }
        ParsedExpression::NonNullAssertion { expression, .. } => {
            receiver_any_is_genuine(expression, symbols, ctx)
        }
        // A union with `any` in it is `any`, so either arm settles it:
        // `caller.list() ?? []` and `cond ? caller.x : []` both stay `any`.
        ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::Binary { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. } => {
            receiver_any_is_genuine(left, symbols, ctx)
                || receiver_any_is_genuine(right, symbols, ctx)
        }
        ParsedExpression::Conditional {
            when_true,
            when_false,
            ..
        } => {
            receiver_any_is_genuine(when_true, symbols, ctx)
                || receiver_any_is_genuine(when_false, symbols, ctx)
        }
        _ => false,
    }
}

fn name_is_genuine_any(name: &str, symbols: &SymbolTable, ctx: &CheckerContext) -> bool {
    symbols
        .get(name)
        .is_some_and(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::ErrorImport))
        || ctx.genuine_any_bindings.contains(name)
}

/// A member of a call's receiver: surge's own member tables, then the global
/// interface the configured lib declares behind a built-in receiver.
fn receiver_member_type(receiver: &Type, property_name: &str, ctx: &mut CheckerContext) -> Option<Type> {
    (!crate::infer::expression::lib_lacks_builtin_member(receiver, property_name, ctx))
        .then(|| receiver.get_property_access_type(property_name))
        .flatten()
        .or_else(|| crate::infer::expression::lib_builtin_member_type(receiver, property_name, ctx))
        .map(|member| super::bind_member_polymorphic_this(member, receiver, ctx))
}

/// `Array.prototype.filter` narrows its element type when the callback is a type
/// predicate (`rows.filter(isCode)` is `Code[]`). The predicate lives on the
/// argument's collected signature, not on its resolved callable type, so it is
/// read from there; anything else falls through to the ordinary `filter` model.
pub(crate) fn filtered_element_type(
    element: &Type,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let [argument] = arguments else {
        return None;
    };
    match &argument.expression {
        ParsedExpression::Identifier { name, .. } => {
            crate::checks::function::predicate_target_of_value(name, symbols, ctx)
        }
        ParsedExpression::ArrowFunction(arrow) if arrow.return_type.is_none() => {
            inferred_predicate_target(arrow, element, symbols)
        }
        // An inline `(x): x is T => …` carries its predicate on the arrow's
        // written return type.
        ParsedExpression::ArrowFunction(arrow) => {
            let Some(ParsedType::Predicate(predicate)) = &arrow.return_type else {
                return None;
            };
            if predicate.asserts || !arrow.type_parameters.is_empty() {
                return None;
            }
            // Resolving the target here is a query the *call* makes; the arrow
            // checks its own annotation at its own span. Reporting from here is
            // therefore a second diagnostic, and it is made without the arrow's
            // parameters in scope — a predicate written in terms of the tested
            // parameter (`(c): c is Exclude<typeof c, undefined>`) has no `c` to
            // find and reported a false TS2304 on it.
            let diagnostics_before = ctx.diagnostics().len();
            let target = crate::infer::map_parsed_type(predicate.ty.clone()?, ctx);
            ctx.truncate_diagnostics(diagnostics_before);
            (!target.is_unknown()).then_some(target)
        }
        _ => None,
    }
}

/// tsc's `getTypePredicateFromBody`: a callback with no return annotation
/// whose body is one boolean expression is a type predicate when that
/// expression narrows its first parameter in its true branch and the narrowed
/// type cannot survive the false branch (`(op) => op.type === 'state'`), so
/// `xs.filter(x => x !== null)` yields `number[]` (TS 5.5). Only the first
/// parameter is read, which is the one `filter`'s predicate overload tests.
/// The predicate's target is the true-branch type.
fn inferred_predicate_target(
    arrow: &surge_ts_syntax::ParsedArrowFunction,
    element: &Type,
    symbols: &SymbolTable,
) -> Option<Type> {
    if arrow.return_type.is_some()
        || arrow.is_async
        || arrow.is_generator
        || !arrow.type_parameters.is_empty()
    {
        return None;
    }
    let parameter = arrow.parameters.first()?;
    if parameter.rest {
        return None;
    }
    let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } = &parameter.binding_name
    else {
        return None;
    };
    let mut condition = single_returned_expression(&arrow.body)?;
    // `narrowType` sees through `satisfies` as it does through parentheses.
    while let ParsedExpression::SatisfiesExpression { expression, .. } = condition {
        condition = expression;
    }
    // tsc requires the body to be boolean-typed: `(x) => x` narrows `x` in its
    // true branch but returns `x`, not a boolean, so it is no predicate.
    if !is_boolean_shaped(condition) {
        return None;
    }
    let true_type = narrow_by_branch(condition, element, name, true, symbols)?;
    if true_type == *element || true_type.is_unknown() {
        return None;
    }
    let false_type = narrow_by_branch(condition, &true_type, name, false, symbols)?;
    if is_never(&false_type) {
        return Some(true_type);
    }
    // The shared narrowers only split unions, so a lone `{ a: 1 }` that the
    // false branch of `x !== undefined` cannot reach comes back unchanged.
    // Narrowing the element instead asks the same question through the union
    // split: the predicate holds when no true-branch member survives the false
    // branch.
    let element_false = narrow_by_branch(condition, element, name, false, symbols)?;
    let false_members = union_members(&element_false);
    union_members(&true_type)
        .iter()
        .all(|member| !false_members.contains(member))
        .then_some(true_type)
}

/// The one expression a predicate body can consist of: an expression body, or a
/// block whose only statement is a `return`. tsc refuses to infer from anything
/// with more than one return.
fn single_returned_expression(
    body: &surge_ts_syntax::ParsedArrowFunctionBody,
) -> Option<&ParsedExpression> {
    match body {
        surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => Some(expression),
        surge_ts_syntax::ParsedArrowFunctionBody::Block(statements) => match statements.as_slice() {
            [surge_ts_syntax::ParsedFunctionBodyStatement::Return(statement)] => {
                statement.expression.as_ref()
            }
            _ => None,
        },
    }
}

fn is_boolean_shaped(expression: &ParsedExpression) -> bool {
    use surge_ts_syntax::{ParsedBinaryOperator as Op, ParsedLogicalOperator, ParsedUnaryOperator};
    match expression {
        ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::Call { .. }
        | ParsedExpression::PropertyCall { .. }
        | ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            ..
        } => true,
        ParsedExpression::Binary { operator, .. } => matches!(
            operator,
            Op::StrictEquals
                | Op::StrictNotEquals
                | Op::Equals
                | Op::NotEquals
                | Op::LessThan
                | Op::LessThanEquals
                | Op::GreaterThan
                | Op::GreaterThanEquals
                | Op::In
                | Op::Instanceof
        ),
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::And | ParsedLogicalOperator::Or,
            right,
            ..
        } => is_boolean_shaped(left) && is_boolean_shaped(right),
        _ => false,
    }
}

fn union_members(ty: &Type) -> Vec<Type> {
    match ty {
        Type::Union(union) => union.types().to_vec(),
        _ if is_never(ty) => Vec::new(),
        _ => vec![ty.clone()],
    }
}

fn is_never(ty: &Type) -> bool {
    match ty {
        Type::Never => true,
        Type::Union(union) => union.types().is_empty(),
        _ => false,
    }
}

/// `name.prop === literal` (or `!==`) decided per union member, as tsc's
/// `narrowTypeByDiscriminant` does: a member whose `prop` is a literal is kept
/// exactly when the comparison can hold, and one whose `prop` is anything wider
/// stays. Unlike the scope narrower this answers `never` when nothing survives,
/// which is what tells a predicate apart from a plain boolean.
fn narrow_by_discriminant_leaf(
    condition: &ParsedExpression,
    ty: &Type,
    name: &str,
    branch_is_true: bool,
) -> Option<Type> {
    use surge_ts_syntax::ParsedBinaryOperator;
    let ParsedExpression::Binary {
        left,
        operator,
        right,
        ..
    } = condition
    else {
        return None;
    };
    let equality = match operator {
        ParsedBinaryOperator::StrictEquals => true,
        ParsedBinaryOperator::StrictNotEquals => false,
        _ => return None,
    };
    let (access, literal) = match (left.as_ref(), right.as_ref()) {
        (access @ ParsedExpression::PropertyAccess { .. }, literal)
        | (literal, access @ ParsedExpression::PropertyAccess { .. }) => (access, literal),
        _ => return None,
    };
    let ParsedExpression::PropertyAccess {
        object,
        property_name,
        ..
    } = access
    else {
        return None;
    };
    if !matches!(object.as_ref(), ParsedExpression::Identifier { name: base, .. } if base == name) {
        return None;
    }
    let literal = match literal {
        ParsedExpression::StringLiteral(value) => Type::StringLiteral(value.clone()),
        ParsedExpression::BooleanLiteral(value) => Type::BooleanLiteral(*value),
        _ => return None,
    };
    let keep_equal = equality == branch_is_true;
    let flattened = surge_ts_types::flatten_reference_unions(ty).unwrap_or_else(|| ty.clone());
    let members: Vec<Type> = match &flattened {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let kept: Vec<Type> = members
        .into_iter()
        .filter(|member| {
            let Type::Object(object) = member.peeled() else {
                return true;
            };
            match object
                .properties
                .get(property_name.as_str())
                .map(|property| property.ty.peeled())
            {
                Some(value @ (Type::StringLiteral(_) | Type::BooleanLiteral(_))) => {
                    (value == literal) == keep_equal
                }
                _ => true,
            }
        })
        .collect();
    Some(if kept.is_empty() {
        Type::Never
    } else {
        union_type(kept)
    })
}

/// `ty` narrowed by `condition` holding (or failing), composed over `&&`, `||`
/// and `!` the way control flow composes them: `a && b` fails where `a` fails,
/// or where `a` holds and `b` fails. A leaf condition is left to the scope
/// narrower; one it cannot read leaves the type as it was.
fn narrow_by_branch(
    condition: &ParsedExpression,
    ty: &Type,
    name: &str,
    branch_is_true: bool,
    symbols: &SymbolTable,
) -> Option<Type> {
    use surge_ts_syntax::{ParsedLogicalOperator, ParsedUnaryOperator};
    if is_never(ty) {
        return Some(Type::Never);
    }
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => narrow_by_branch(operand, ty, name, !branch_is_true, symbols),
        ParsedExpression::Logical {
            left,
            operator,
            right,
            ..
        } if matches!(
            operator,
            ParsedLogicalOperator::And | ParsedLogicalOperator::Or
        ) =>
        {
            let conjunction = matches!(operator, ParsedLogicalOperator::And);
            // The side on which the left operand decides the outcome.
            let left_decides = narrow_by_branch(left, ty, name, !conjunction, symbols)?;
            let left_passes = narrow_by_branch(left, ty, name, conjunction, symbols)?;
            let right = narrow_by_branch(right, &left_passes, name, branch_is_true, symbols)?;
            if branch_is_true == conjunction {
                return Some(right);
            }
            let reachable: Vec<Type> = [left_decides, right]
                .into_iter()
                .filter(|member| !is_never(member))
                .collect();
            Some(if reachable.is_empty() {
                Type::Never
            } else {
                union_type(reachable)
            })
        }
        _ if let Some(narrowed) =
            narrow_by_discriminant_leaf(condition, ty, name, branch_is_true) =>
        {
            Some(narrowed)
        }
        // A test of one of the parameter's properties narrows the parameter
        // itself only as a union's discriminant (`narrowTypeByDiscriminant`).
        _ if !matches!(ty, Type::Union(_)) && !condition_tests_reference(condition, name) => {
            Some(ty.clone())
        }
        _ => {
            let mut scope =
                symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            scope.insert(
                name.to_string(),
                crate::symbols::SymbolInfo {
                    ty: ty.clone(),
                    kind: crate::symbols::SymbolKind::Parameter,
                    function_signature: None,
                },
            );
            match crate::checks::function::narrow_condition_symbol_table(
                condition,
                &scope,
                branch_is_true,
            ) {
                Some(narrowed) => Some(narrowed.get(name)?.ty.clone()),
                None => Some(ty.clone()),
            }
        }
    }
}

/// Whether a condition tests the reference `name` itself rather than one of
/// its properties: `name`, `typeof name`, `name <op> x`, `"p" in name`, or a
/// call handed `name`.
fn condition_tests_reference(condition: &ParsedExpression, name: &str) -> bool {
    let is_reference =
        |expression: &ParsedExpression| matches!(expression, ParsedExpression::Identifier { name: found, .. } if found == name);
    match condition {
        ParsedExpression::Identifier { .. } => is_reference(condition),
        ParsedExpression::Unary { operand, .. } => {
            is_reference(operand) || condition_tests_reference(operand, name)
        }
        ParsedExpression::Binary { left, right, .. } => {
            is_reference(left)
                || is_reference(right)
                || condition_tests_reference(left, name)
                || condition_tests_reference(right, name)
        }
        ParsedExpression::Call { arguments, .. } | ParsedExpression::PropertyCall { arguments, .. } => {
            arguments.iter().any(|argument| is_reference(&argument.expression))
        }
        _ => false,
    }
}

pub(crate) fn check_property_call_like(
    object: &ParsedExpression,
    object_span: Option<SyntaxTextSpan>,
    property_name: &str,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_property_call_like_unrecorded(object, object_span, property_name, property_span, call_span, type_arguments, arguments, expected_return_type, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

const OBJECT_MEMBER_NAMES: &[&str] = &[
    "constructor",
    "toString",
    "toLocaleString",
    "valueOf",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
];

fn check_property_call_like_unrecorded(
    object: &ParsedExpression,
    object_span: Option<SyntaxTextSpan>,
    property_name: &str,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if matches!(property_name, "push" | "unshift") {
        crate::checks::expr::mark_evolving_array_operation(object, ctx);
    }
    let object_ty =
        match crate::checks::expr::evaluate_expression(object, object_span, symbols, ctx) {
            // `checkNonNullExpression`: an `unknown` receiver is reported and
            // the call goes on as the error type.
            crate::infer::InferredExpression::Known(Type::GenuineUnknown)
                if crate::checks::expr::report_unknown_operand(object, object_span.or(call_span), ctx) =>
            {
                evaluate_arguments_on_error_type(arguments, symbols, ctx);
                return Some(Type::ErrorType);
            }
            // `Promise<T>` is modelled as its awaited `T` (see the `.then`
            // arm below), so a `void`-resolving promise looks `undefined` to
            // the receiver check; the promise itself is never nullish.
            crate::infer::InferredExpression::Known(ty)
                if matches!(property_name, "then" | "catch" | "finally") =>
            {
                ty
            }
            crate::infer::InferredExpression::Known(ty) => {
                crate::checks::expr::strip_reported_undefined_receiver(
                    object,
                    ty,
                    object_span,
                    call_span,
                    symbols,
                    ctx,
                )
            }
            // The receiver did not resolve, but the arguments are still code:
            // an unresolved name, a missing member or an implicit-any parameter
            // inside them is reported by tsc regardless of what the callee is.
            // Dropping the call here left whole callback bodies — JSX subtrees
            // included — unchecked. `evaluate_arguments_context_free` still
            // decides *implicit-any* by receiver provenance, so a chain that
            // merely collapsed to `any` does not start reporting parameters tsc
            // contextually types.
            failed @ (crate::infer::InferredExpression::MissingProperty { .. }
            | crate::infer::InferredExpression::UnresolvedIdentifier { .. }) => {
                if surge_ts_types::private_name::is_private_name_key(property_name) {
                    let _ = crate::checks::expr::check_private_name_access(
                        crate::checks::expr::PrivateNameReceiver::Error,
                        property_name,
                        property_span,
                        symbols,
                        ctx,
                    );
                }
                check_error_call_operands(type_arguments, arguments, symbols, ctx);
                return failed.flowing_type();
            }
            _ => {
                super::check_untyped_call_type_arguments(type_arguments, ctx);
                evaluate_arguments_context_free(object, arguments, symbols, ctx);
                return None;
            }
        };
    let object_ty = type_variable_call_receiver(object_ty, property_name);
    let _this_argument = super::CallThisArgument::set(object_ty.clone(), object_span.or(call_span));

    if surge_ts_types::private_name::is_private_name_key(property_name)
        && let Some(ty) = crate::checks::expr::check_private_name_access(
            crate::checks::expr::PrivateNameReceiver::Type(&object_ty),
            property_name,
            property_span,
            symbols,
            ctx,
        )
    {
        evaluate_arguments_on_error_type(arguments, symbols, ctx);
        return Some(ty);
    }

    crate::checks::expr::check_member_accessibility(
        object,
        &object_ty,
        property_name,
        property_span,
        false,
        symbols,
        ctx,
    );

    // Computed before the dispatch below so the narrowed element can be matched
    // on: `filter` with a type-predicate callback yields that predicate's type.
    let filtered_element = if property_name == "filter"
        && let Type::Array(element) = &object_ty
    {
        filtered_element_type(element, arguments, symbols, ctx)
    } else {
        None
    };

    if property_name == "all" && is_promise_all_receiver(&object_ty) {
        return check_promise_all_call(
            arguments,
            call_span.or(property_span),
            expected_return_type,
            symbols,
            ctx,
        );
    }
    // Only with the promise kept as a type: collapsed, the result would be a
    // bare `T`, which the "a primitive is no promise" rule then rejects where
    // a `Promise<T>` is expected (`return Promise.resolve("")`).
    if property_name == "resolve"
        && promise_nominal_enabled()
        && arguments.len() <= 1
        && type_arguments.is_empty()
        && !arguments.iter().any(|argument| argument.spread)
        && is_promise_all_receiver(&object_ty)
    {
        return check_promise_resolve_call(arguments, expected_return_type, symbols, ctx);
    }
    if property_name == "race"
        && type_arguments.is_empty()
        && is_promise_all_receiver(&object_ty)
        && let Some(result) = check_promise_race_call(arguments, symbols, ctx)
    {
        return Some(result);
    }
    // `Promise.reject(reason)` is a `Promise<never>`. Only with the promise
    // kept as a type: collapsed, it would be a bare `never`, and a call typed
    // `never` ends the flow it sits in.
    if property_name == "reject"
        && promise_nominal_enabled()
        && arguments.len() <= 1
        && type_arguments.is_empty()
        && is_promise_all_receiver(&object_ty)
    {
        evaluate_arguments_context_free(object, arguments, symbols, ctx);
        return Some(promise_of(&Type::Never, ctx));
    }
    // `Promise<T>` is modeled as its awaited `T`, so a `.then`/`.catch`/`.finally`
    // chained on a promise-returning call lands on the value type and would be
    // reported as a missing member. Treat the receiver as the awaited value
    // instead — the chain keeps its collapsed result so further links resolve.
    if matches!(property_name, "then" | "catch" | "finally")
        && !matches!(object_ty, Type::Unknown | Type::TypeParameter(_))
        && !declares_own_property(&object_ty, property_name)
        && !lib_promise_lacks(property_name, ctx)
    {
        return if property_name == "then" {
            check_promise_then_call(object_ty, arguments, symbols, ctx)
        } else {
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            Some(object_ty)
        };
    }

    // A namespace binding that has not resolved in this round reads as `any` or
    // as the sentinel; the qualified `ns.member` entry still carries the member's
    // real signature, so try it before answering from the degraded receiver.
    if matches!(object_ty, Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_))
        && let ParsedExpression::Identifier { name, .. } = object
        && let Some(result) = try_qualified_namespace_call(
            name,
            property_name,
            property_span.or(object_span),
            call_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        )
    {
        return result;
    }

    if let Some(result) = check_bind_call_apply(
        &object_ty,
        property_name,
        property_span,
        call_span,
        type_arguments,
        arguments,
        symbols,
        ctx,
    ) {
        return result;
    }

    // Under strictNullChecks a type variable's apparent type is `unknown`,
    // which lacks even `Object`'s members (`x.hasOwnProperty()` on `T`, or on
    // `T | { data: T }`). Only those are looked up on a variable here: surge
    // does not narrow one by `typeof x === "function"`.
    let object_member_on_variable = surge_ts_types::strict_null_checks()
        && OBJECT_MEMBER_NAMES.contains(&property_name);
    if object_member_on_variable
        && let Type::Union(union) = &object_ty
        && union.types().iter().any(Type::is_type_variable)
        && let missing @ crate::infer::InferredExpression::MissingProperty { .. } =
            crate::infer::expression::infer_property_access(object, &object_span, property_name, &property_span, symbols, ctx)
    {
        crate::checks::expr::report_inferred_expression(missing, property_span, symbols, ctx);
        evaluate_arguments_context_free(object, arguments, symbols, ctx);
        return Some(Type::ErrorType);
    }
    match object_ty {
        Type::ErrorType => {
            check_error_call_operands(type_arguments, arguments, symbols, ctx);
            Some(Type::ErrorType)
        }
        // `resolveUntypedCall` still checks the written type arguments.
        Type::Any => {
            super::check_untyped_call_type_arguments(type_arguments, ctx);
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            Some(Type::Any)
        }
        // The receiver degraded, but the arguments are still code — same
        // reasoning as the unresolved-receiver arm above. `evaluate_arguments_
        // context_free` keeps implicit-any gated on receiver provenance, so a
        // callback whose contextual type surge lost is still not reported.
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
            // A type variable's member is still looked up on its apparent
            // type, as a plain read of it is.
            if object_member_on_variable
                && object_ty.is_type_variable()
                && let missing @ crate::infer::InferredExpression::MissingProperty { .. } =
                    crate::infer::expression::infer_property_access(object, &object_span, property_name, &property_span, symbols, ctx)
            {
                crate::checks::expr::report_inferred_expression(missing, property_span, symbols, ctx);
            }
            super::check_untyped_call_type_arguments(type_arguments, ctx);
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            None
        }
        // The predicate decides the element type; the call is still checked
        // like any other `filter`, which is what gives an inline arrow its
        // contextual parameter types.
        Type::Array(_) if filtered_element.is_some() => {
            match object_ty.get_property_access_type("filter") {
                Some(Type::Function(filter)) => {
                    check_function_type_call(
                        &filter,
                        property_span,
                        call_span,
                        type_arguments,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    )?;
                }
                _ => {
                    for argument in arguments {
                        let _ =
                            evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                    }
                }
            }
            filtered_element.map(|element| Type::Array(Box::new(element)))
        }
        Type::Array(element_type) if property_name == "map" => check_array_map_call(
            element_type.as_ref(),
            property_span,
            call_span,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        ),
        Type::Array(element_type)
            if property_name == "flat"
                && super::builtins::array_flat_result(&element_type, arguments).is_some()
                && !crate::infer::expression::lib_lacks_builtin_member(
                    &Type::Array(element_type.clone()),
                    property_name,
                    ctx,
                ) =>
        {
            super::builtins::array_flat_result(&element_type, arguments)
        }
        Type::Array(element_type) if property_name == "flatMap" && !arguments.is_empty() => {
            super::builtins::check_array_flat_map_call(
                element_type.as_ref(),
                property_span,
                call_span,
                arguments,
                symbols,
                ctx,
            )
        }
        Type::Array(element_type)
            if matches!(property_name, "reduce" | "reduceRight")
                && matches!(arguments.len(), 1 | 2)
                && type_arguments.len() <= 1
                && !arguments.iter().any(|argument| argument.spread) =>
        {
            Some(check_array_reduce_call(
                element_type.as_ref(),
                type_arguments,
                arguments,
                symbols,
                ctx,
            ))
        }
        Type::Array(element_type) if property_name == "find" => check_array_find_call(
            element_type.as_ref(),
            property_span,
            call_span,
            arguments,
            symbols,
            ctx,
        ),
        Type::Union(union_type) => {
            // A sentinel member means part of the receiver is unmodelled, so a
            // miss on any *other* member says nothing about the source — the
            // same no-cascade rule a wholly-sentinel receiver gets.
            if union_type.types().iter().any(Type::is_unknown) {
                return None;
            }
            // The property is looked up on the union before anything is called:
            // a member lacking it is the error, whatever the others hold.
            let lacks_member = |ty: &Type, ctx: &mut CheckerContext| {
                !matches!(ty, Type::Undefined | Type::Null)
                    && receiver_member_type(ty, property_name, ctx).is_none()
                    && !ty.peeled().is_unknown()
                    && !matches!(ty, Type::Array(_) | Type::Tuple(_))
            };
            if union_type.types().iter().any(|ty| lacks_member(ty, ctx))
                && !union_type
                    .types()
                    .iter()
                    .any(|ty| crate::checks::expr::carries_leaked_type_parameter(ty, ctx))
            {
                ctx.push(diagnostic_with_syntax_span(
                    crate::checks::expr::missing_property_diagnostic(
                        property_name,
                        &Type::Union(union_type.clone()),
                        symbols,
                        ctx,
                    ),
                    crate::spans::choose_span(property_span, object_span),
                ));
                check_error_call_operands(type_arguments, arguments, symbols, ctx);
                return Some(Type::ErrorType);
            }
            if let Some(callee) = union_receiver_callee(&union_type, property_name) {
                return super::check_callable_union_call(
                    &callee,
                    property_span,
                    crate::checks::expr::element_access_span(object_span, property_span)
                        .map(|span| SyntaxTextSpan { end: span.end - 1, ..span }),
                    call_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                );
            }
            let mut result_types = vec![];
            // An empty array (`x ?? []`) beside other members adds no signature
            // of its own: tsc combines the members' signatures, where its
            // `never` callback parameters fall away in the union
            // (`getIntersectedSignatures`), so it is not called on its own.
            let empty_array = |ty: &Type| matches!(ty, Type::Tuple(elements) if elements.is_empty())
                || matches!(ty, Type::Array(element) if matches!(**element, Type::Never));
            let has_other_member = union_type
                .types()
                .iter()
                .any(|ty| !empty_array(ty) && !matches!(ty, Type::Undefined | Type::Null));
            for ty in union_type.types() {
                if matches!(ty, Type::Undefined | Type::Null) {
                    result_types.push(Type::Undefined);
                    continue;
                }
                if has_other_member && empty_array(ty) {
                    continue;
                }

                if property_name == "map"
                    && let Type::Array(element_type) = ty
                {
                    let mapped = check_array_map_call(
                        element_type.as_ref(),
                        property_span,
                        call_span,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    )?;
                    result_types.push(mapped);
                    continue;
                }

                if property_name == "find"
                    && let Type::Array(element_type) = ty
                {
                    let found = check_array_find_call(
                        element_type.as_ref(),
                        property_span,
                        call_span,
                        arguments,
                        symbols,
                        ctx,
                    )?;
                    result_types.push(found);
                    continue;
                }

                if property_name == "find"
                    && let Type::Array(element_type) = ty
                {
                    let found = check_array_find_call(
                        element_type.as_ref(),
                        property_span,
                        call_span,
                        arguments,
                        symbols,
                        ctx,
                    )?;
                    result_types.push(found);
                    continue;
                }

                let Some(property_type) = receiver_member_type(ty, property_name, ctx) else {
                    // A member whose reference peels to the sentinel is a shape
                    // surge could not reconstruct, not a type without the member.
                    if ty.peeled().is_unknown()
                        || crate::checks::expr::carries_leaked_type_parameter(ty, ctx)
                    {
                        return None;
                    }
                    if no_lib_array_member(ty, ctx) {
                        result_types.push(Type::Any);
                        continue;
                    }
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2339(
                            property_name,
                            &Type::Union(union_type.clone()).name(),
                            ctx.file_name.clone(),
                        ),
                        crate::spans::choose_span(property_span, object_span),
                    ));
                    check_error_call_operands(type_arguments, arguments, symbols, ctx);
                    return Some(Type::ErrorType);
                };

                let declared_member = property_type.clone();
                match callable_property_signature(property_type) {
                    Type::Function(function_type) => {
                        if super::report_type_argument_arity(
                            &Type::Function(function_type.clone()),
                            type_arguments,
                            super::type_arguments_start(property_span),
                            false,
                            ctx,
                        ) {
                            evaluate_arguments_context_free(object, arguments, symbols, ctx);
                            return None;
                        }
                        let function_type = instantiate_declared_member_signature(
                            &function_type,
                            Some(&declared_member),
                            type_arguments,
                            property_span,
                            arguments,
                            expected_return_type,
                            symbols,
                            ctx,
                        );
                        let return_type = check_function_type_call(
                            &function_type,
                            property_span,
                            call_span,
                            type_arguments,
                            arguments,
                            expected_return_type,
                            symbols,
                            ctx,
                        )?;
                        result_types.push(return_type);
                    }
                    Type::Any => result_types.push(Type::Any),
                    Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => return None,
                    // A member whose own type is a union of callables sharing
                    // one signature is callable, and one carrying the
                    // degradation sentinel proves nothing — the same rules the
                    // bare-call path applies.
                    Type::Union(union) => {
                        let return_type = super::check_callable_union_call(
                            &union,
                            property_span,
                            crate::checks::expr::element_access_span(object_span, property_span)
                                .map(|span| SyntaxTextSpan { end: span.end - 1, ..span }),
                            call_span,
                            type_arguments,
                            arguments,
                            symbols,
                            ctx,
                        )?;
                        result_types.push(return_type);
                    }
                    // tsc's `isUntypedFunctionCall`: no signatures, but a `Function`.
                    other if surge_ts_types::is_untyped_function_callee(&other) => {
                        for argument in arguments {
                            let _ = crate::checks::expr::evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                        }
                        result_types.push(Type::Any);
                    }
                    other => {
                        ctx.push(diagnostic_with_syntax_span(
                            super::not_callable_diagnostic(&other, ctx),
                            crate::spans::choose_span(
                                property_span,
                                crate::spans::choose_span(call_span, object_span),
                            ),
                        ));
                        return None;
                    }
                }
            }

            Some(surge_ts_types::union_type(result_types))
        }
        _ => {
            if property_name == "map"
                && let Type::Array(element_type) = &object_ty
            {
                return check_array_map_call(
                    element_type.as_ref(),
                    property_span,
                    call_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                );
            }

            let Some(property_type) = receiver_member_type(&object_ty, property_name, ctx) else {
                if no_lib_array_member(&object_ty, ctx) {
                    return Some(Type::Any);
                }
                // See the matching guard in the property-access path: a nominal
                // reference peeling to the sentinel is an unreconstructed shape,
                // not a type without the member.
                if object_ty.peeled().is_unknown()
                    || crate::checks::expr::carries_leaked_type_parameter(&object_ty, ctx)
                {
                    return None;
                }
                if let Some(diagnostic) =
                    crate::checks::expr::global_this_missing_member(property_name, &object_ty, ctx)
                {
                    if let Some(diagnostic) = diagnostic {
                        ctx.push(diagnostic_with_syntax_span(
                            diagnostic,
                            crate::spans::choose_span(property_span, object_span),
                        ));
                    }
                    return Some(Type::Any);
                }
                // tsc's `reportNonexistentProperty`, as for any member read.
                let diagnostic = crate::checks::expr::missing_property_diagnostic(
                    property_name,
                    &object_ty,
                    symbols,
                    ctx,
                );
                ctx.push(diagnostic_with_syntax_span(
                    diagnostic,
                    crate::spans::choose_span(property_span, object_span),
                ));
                check_error_call_operands(type_arguments, arguments, symbols, ctx);
                return Some(Type::ErrorType);
            };

            // A generic namespace member is published under a qualified
            // `ns.member` key carrying its real signature; the namespace object
            // models only the member *set*, so the member itself reads as the
            // permissive `any`. Routing the call through the qualified binding
            // restores arity, return type, and generic inference. Gated on that
            // permissive shape — already in hand — so no ordinary property call
            // pays for the lookup.
            if let ParsedExpression::Identifier { name, .. } = object
                && is_permissive_member_type(&property_type)
                && let Some(result) = try_qualified_namespace_call(
                    name,
                    property_name,
                    property_span.or(object_span),
                    call_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                )
            {
                return result;
            }

            let declared_member = property_type.clone();
            match callable_property_signature(property_type) {
                Type::Function(function_type) => {
                    if super::report_type_argument_arity(
                        &Type::Function(function_type.clone()),
                        type_arguments,
                        super::type_arguments_start(property_span),
                        false,
                        ctx,
                    ) {
                        evaluate_arguments_context_free(object, arguments, symbols, ctx);
                        return None;
                    }
                    let function_type = instantiate_declared_member_signature(
                        &function_type,
                        Some(&declared_member),
                        type_arguments,
                        property_span,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    );
                    check_function_type_call(
                        &function_type,
                        property_span,
                        call_span,
                        type_arguments,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    )
                }
                Type::ErrorType => {
                    evaluate_arguments_on_error_type(arguments, symbols, ctx);
                    Some(Type::ErrorType)
                }
                Type::Any => {
                    evaluate_arguments_context_free(object, arguments, symbols, ctx);
                    Some(Type::Any)
                }
                Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
                    evaluate_arguments_context_free(object, arguments, symbols, ctx);
                    None
                },
                // See the union arm in the multi-receiver loop above: a property
                // typed as a union of callables is callable, and one carrying the
                // degradation sentinel is not a source error.
                Type::Union(union) => super::check_callable_union_call(
                    &union,
                    property_span,
                    crate::checks::expr::element_access_span(object_span, property_span)
                        .map(|span| SyntaxTextSpan { end: span.end - 1, ..span }),
                    call_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                ),
                // tsc's `isUntypedFunctionCall`: no signatures, but a `Function`.
                other if surge_ts_types::is_untyped_function_callee(&other) => {
                    for argument in arguments {
                        let _ = crate::checks::expr::evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                    }
                    Some(Type::Any)
                }
                other => {
                    let get_accessor_call = arguments.is_empty()
                        && crate::checks::expr::member_is_get_accessor(
                            object,
                            &object_ty,
                            property_name,
                            symbols,
                            ctx,
                        );
                    ctx.push(diagnostic_with_syntax_span(
                        super::not_callable_member_diagnostic(&other, get_accessor_call, ctx),
                        crate::spans::choose_span(
                            property_span,
                            crate::spans::choose_span(call_span, object_span),
                        ),
                    ));
                    None
                }
            }
        }
    }
}

fn check_promise_then_call(
    value_type: Type,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Some(callback) = arguments.first() else {
        return Some(Type::Unknown);
    };

    let callback_type =
        Type::Function(FunctionType::new(vec![value_type], Type::Unknown, false, 1));
    let inferred_callback = evaluate_expression_with_expected_type(
        &callback.expression,
        callback.span,
        Some(&callback_type),
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );

    // The lib's `onrejected` takes `(reason: any)`; every argument is still
    // checked (`resolveCall`).
    let rejected_type = Type::Function(FunctionType::new(vec![Type::Any], Type::Unknown, false, 1));
    for (index, argument) in arguments.iter().enumerate().skip(1) {
        let _ = evaluate_expression_with_expected_type(
            &argument.expression,
            argument.span,
            (index == 1).then_some(&rejected_type),
            ExpectedTypeDiagnostic::ArgumentNotAssignable,
            symbols,
            ctx,
        );
    }

    let next_value = match inferred_callback {
        InferredExpression::Known(Type::Function(function_type)) => {
            promise_like_awaited_type(function_type.return_type())
        }
        InferredExpression::Known(ty) => promise_like_awaited_type(&ty),
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => Type::Unknown,
    };

    // A `then` answers a promise; only with `Promise<T>` collapsed is that its
    // value. The receiver can reach here as a promise too, when its interface
    // did not resolve where `then` was looked up.
    // `TResult1` is inferred from the callback's return, which widens its
    // fresh literals (`() => ({ data: 5 })` is `Promise<{ data: number }>`).
    let next_value = if promise_nominal_enabled() {
        crate::checks::expr::widen_type(&next_value)
    } else {
        next_value
    };
    Some(promise_of(&next_value, ctx))
}

/// Whether `ty` answers `name` from a member of its own, as opposed to from an
/// index signature. A `.d.ts` class whose base could not be resolved is left open
/// with a permissive string index, which would otherwise make every value look
/// like a thenable.
pub(crate) fn declares_own_property(ty: &Type, name: &str) -> bool {
    if let Type::Object(object_type) = ty.peeled() {
        return object_type.get_property_type(name).is_some();
    }
    ty.get_property_access_type(name).is_some()
}

pub(crate) fn promise_like_awaited_type(ty: &Type) -> Type {
    if let Type::Reference(reference) = ty {
        let base = reference
            .display
            .split('<')
            .next()
            .unwrap_or(&reference.display);
        if matches!(base, "Promise" | "PromiseLike")
            && let Some(value_type) = reference.arguments.first()
        {
            return value_type.clone();
        }
    }
    // An instantiation already resolved to its object: its `then` says what
    // it resolves to, and nothing lazy is forced reading it.
    if let Type::Object(object) = ty
        && object
            .alias_name
            .as_deref()
            .is_some_and(|name| matches!(name.split('<').next(), Some("Promise" | "PromiseLike")))
        && let Some(value_type) = thenable_awaited_type(ty)
    {
        return value_type;
    }

    ty.clone()
}

/// `Promise<value>` — what an async function returns for a body that
/// completes with `value` (tsc's `createPromiseReturnType`). The awaited value
/// is what gets wrapped, so a body returning a promise does not nest. Without a
/// lib `Promise` the value is kept as it is.
pub(crate) fn promise_of(value: &Type, ctx: &mut CheckerContext) -> Type {
    if !promise_nominal_enabled() {
        return value.clone();
    }
    // Unwrapped by its rendering alone: forcing a lazy value (`awaited_type`
    // peels for a thenable) resolves it here, outside the scope that binds
    // the names it was written in (`core.output<typeof schema>` inside the
    // arrow whose parameter `schema` is).
    let value = promise_like_awaited_type(value);
    if value.is_unknown() && !matches!(value, Type::GenuineUnknown) {
        return value;
    }
    // The slot is named after the value: a reference renders its written
    // argument, and the instantiation is cached by its resolved arguments, so
    // any other name would be the display every later `Promise<value>` gets.
    let slot = value.name();
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    substitution.insert(slot.clone(), value.clone());
    let named = |name: &str, type_arguments| {
        ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments,
        }))
    };
    let reported = ctx.diagnostics().len();
    let promise = crate::infer::map_parsed_type_with_substitution(
        named("Promise", vec![named(&slot, Vec::new())]),
        ctx,
        &substitution,
    );
    ctx.truncate_diagnostics(reported);
    match promise {
        // The reference renders its written argument, which is the slot name.
        Type::Reference(mut reference) => {
            reference.display = format!("Promise<{}>", value.name()).into();
            Type::Reference(reference)
        }
        promise if promise.is_unknown() => value,
        promise => promise,
    }
}

/// Keep the lib's `Promise<T>` / `PromiseLike<T>` as the interface it is, as
/// tsc does, instead of collapsing it to its awaited `T`. The collapse dates
/// from when `await` was erased at parse time; it makes `Promise<T> |
/// undefined` read as `T | undefined` and hides every misuse of a promise as
/// its value. On by default; `SURGE_PROMISE_NOMINAL=0` restores the collapse.
pub(crate) fn promise_nominal_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_PROMISE_NOMINAL").as_deref() != Ok("0"))
}

/// The type `await ty` produces, following tsc's `getAwaitedTypeNoAlias`:
/// `any` awaits to itself, a union awaits constituent-wise, and a promise is
/// unwrapped repeatedly (`Promise<Promise<T>>` awaits to `T`). Anything that is
/// not promise-like awaits to itself.
pub(crate) fn awaited_type(ty: &Type) -> Type {
    awaited_type_at_depth(ty, 0)
}

fn awaited_type_at_depth(ty: &Type, depth: usize) -> Type {
    // tsc pushes each type onto `awaitedTypeStack` to stop mutually recursive
    // thenables (`BadPromiseA`/`BadPromiseB`); a depth cap ends the same cycles.
    const MAX_UNWRAP_DEPTH: usize = 10;
    if depth >= MAX_UNWRAP_DEPTH || matches!(ty, Type::Any) || ty.is_unknown() {
        return ty.clone();
    }

    // tsc's `getAwaitedTypeNoAlias` awaits each member of a union — including
    // one reached through an alias (`MaybePromise<T> = T | Promise<T>`), whose
    // type in tsc simply is that union.
    let union = match ty {
        Type::Union(union) => Some(union.clone()),
        Type::Reference(_) => match ty.peeled() {
            Type::Union(union) => Some(union),
            _ => None,
        },
        _ => None,
    };
    if let Some(union) = union {
        return union_type(
            union
                .types()
                .iter()
                .map(|member| awaited_type_at_depth(member, depth + 1))
                .collect(),
        );
    }

    let promised = promise_like_awaited_type(ty);
    if promised != *ty {
        return awaited_type_at_depth(&promised, depth + 1);
    }
    if let Some(promised) = thenable_awaited_type(ty)
        && promised != *ty
    {
        return awaited_type_at_depth(&promised, depth + 1);
    }

    ty.clone()
}

/// tsc's `getAwaitedType`, where [`awaited_type`] is `getAwaitedTypeNoAlias`:
/// an `await` (or `for await`) of a type variable whose constraint does not
/// rule out a thenable (`isAwaitedTypeNeeded`) produces `Awaited<T>`, a type
/// of its own that relates to `T` but that `T` does not relate to.
pub(crate) fn awaited_type_with_alias(ty: &Type, ctx: &mut CheckerContext) -> Type {
    let awaited = awaited_type(ty);
    match &awaited {
        Type::Union(union) if union.types().iter().any(awaited_alias_needed) => {
            union_type(union.types().iter().map(|member| awaited_alias(member, ctx)).collect())
        }
        _ => awaited_alias(&awaited, ctx),
    }
}

fn awaited_alias(ty: &Type, ctx: &mut CheckerContext) -> Type {
    if !awaited_alias_needed(ty) {
        return ty.clone();
    }
    const OPERAND: &str = "__surge_awaited_operand";
    let named = |name: &str, type_arguments: Vec<ParsedType>| {
        ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments,
        }))
    };
    let mut substitution = crate::infer::types::TypeParameterSubstitution::default();
    substitution.insert(OPERAND.to_string(), ty.clone());
    let checkpoint = ctx.diagnostics().len();
    let resolved = crate::infer::types::resolve_parsed_type(
        named("Awaited", vec![named(OPERAND, Vec::new())]),
        ctx,
        &mut Vec::new(),
        &substitution,
    );
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if resolved.had_error() {
        return ty.clone();
    }
    let resolved = resolved.into_ty();
    if resolved.is_unmodelled() { ty.clone() } else { resolved }
}

fn awaited_alias_needed(ty: &Type) -> bool {
    let Type::TypeParameter(parameter) = ty else {
        return false;
    };
    if !ty.is_type_variable() || surge_ts_types::type_variable::deferred_type(parameter).is_some() {
        return false;
    }
    match surge_ts_types::type_variable::active_constraint(parameter) {
        Some(None) => true,
        Some(Some(constraint)) => constraint_admits_thenable(&constraint),
        None => false,
    }
}

/// The base-constraint arm of `isAwaitedTypeNeeded`: `any`, `unknown`, `{}`,
/// `object`, or something promise-like.
fn constraint_admits_thenable(constraint: &Type) -> bool {
    let thenable = |ty: &Type| promise_like_awaited_type(ty) != *ty || thenable_awaited_type(ty).is_some();
    match constraint.peeled() {
        Type::Any | Type::GenuineUnknown => true,
        Type::Object(object)
            if object.non_primitive
                || (object.properties.is_empty()
                    && !object.is_intersection
                    && object.string_index_type.is_none()
                    && object.call_signature().is_none()
                    && object.construct_signature().is_none()) =>
        {
            true
        }
        Type::Union(union) => union.types().iter().any(thenable),
        other => thenable(&other),
    }
}

/// The value a user-defined thenable resolves to: the parameter of the callback
/// `then` takes first.
///
/// `await` is erased at parse time, so a promise-typed value is modelled as the
/// value it resolves to everywhere — which is why `Promise<T>` reads as `T`
/// above. A class that implements `Promise<T>` rather than being one
/// (drizzle's `QueryPromise`, and every `PgRaw`/`SQLiteRaw` that extends it) got
/// no such treatment, so `const rows = await db.execute(...)` read as the raw
/// query object and indexing it reported a missing property.
pub(crate) fn thenable_awaited_type(ty: &Type) -> Option<Type> {
    let Type::Object(object) = ty.peeled() else {
        return None;
    };
    let then = object.get_property_access_type("then")?;
    let on_fulfilled = callable_first_parameter(&then)?;
    callable_first_parameter(&on_fulfilled)
}

/// tsc's `isThenableType` for a type `getPromisedTypeOfPromise` cannot
/// unwrap: a callable `then` whose first parameter is no callback. Awaiting
/// one is TS1320, returning one from an async function TS1058. A `then` or a
/// first parameter surge could not model is left alone.
pub(crate) fn is_non_promise_thenable(ty: &Type) -> bool {
    fn call_signatures(ty: &Type, out: &mut Vec<surge_ts_types::FunctionType>) {
        match ty.peeled() {
            Type::Function(function) => function.push_overload_members(out),
            Type::Union(union) => {
                for member in union.types() {
                    if !matches!(member, Type::Undefined | Type::Null) {
                        call_signatures(member, out);
                    }
                }
            }
            Type::Object(object) => {
                if let Some(signature) = object.call_signature() {
                    signature.push_overload_members(out);
                }
            }
            _ => {}
        }
    }
    match ty.peeled() {
        Type::Union(union) => union.types().iter().any(is_non_promise_thenable),
        Type::Object(object) => {
            if promise_like_awaited_type(ty) != *ty {
                return false;
            }
            let Some(then) = object.get_property_access_type("then") else {
                return false;
            };
            let mut signatures = Vec::new();
            call_signatures(&then, &mut signatures);
            !signatures.is_empty()
                && signatures.iter().all(|signature| match signature.parameters().first() {
                    None => true,
                    Some(on_fulfilled) => {
                        let mut callbacks = Vec::new();
                        call_signatures(on_fulfilled, &mut callbacks);
                        !on_fulfilled.is_unmodelled() && !matches!(on_fulfilled.peeled(), Type::Any) && callbacks.is_empty()
                    }
                })
        }
        _ => false,
    }
}

/// The first parameter of `ty` read as a callable — looking through a union, the
/// way an optional callback parameter is written
/// (`((value: T) => R) | undefined | null`).
fn callable_first_parameter(ty: &Type) -> Option<Type> {
    match ty.peeled() {
        Type::Function(function) => function.parameters().first().cloned(),
        Type::Union(union) => union
            .types()
            .iter()
            .find_map(|member| match member.peeled() {
                Type::Function(function) => function.parameters().first().cloned(),
                _ => None,
            }),
        _ => None,
    }
}

pub(crate) fn check_optional_property_call(
    object: &surge_ts_syntax::ParsedExpression,
    object_span: Option<SyntaxTextSpan>,
    property_name: &str,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = check_optional_property_call_unrecorded(object, object_span, property_name, property_span, call_span, type_arguments, arguments, expected_return_type, symbols, ctx);
    crate::semantic::record_span_type(call_span, result.as_ref(), ctx);
    result
}

fn check_optional_property_call_unrecorded(
    object: &surge_ts_syntax::ParsedExpression,
    object_span: Option<SyntaxTextSpan>,
    property_name: &str,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    type_arguments: &[ParsedType],
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if matches!(property_name, "push" | "unshift") {
        crate::checks::expr::mark_evolving_array_operation(object, ctx);
    }
    let object_result = evaluate_expression(object, object_span, symbols, ctx);

    let object_type = match object_result {
        InferredExpression::Known(ty) => ty,
        InferredExpression::MissingProperty { .. }
        | InferredExpression::UnresolvedIdentifier { .. } => {
            evaluate_arguments_on_error_type(arguments, symbols, ctx);
            return Some(Type::ErrorType);
        }
        InferredExpression::Unknown => return None,
    };

    // No early return for a degraded receiver: the `Unknown` arm below walks the
    // arguments, and returning here skipped it — everything inside a callback on
    // an unmodelled receiver (`messages?.map((item) => <article>…</article>)`)
    // went unchecked, which is where trpc's missing UMD-global reports live.
    let base_type = surge_ts_types::remove_nullish(&object_type);
    crate::checks::expr::check_member_accessibility(
        object,
        &base_type,
        property_name,
        property_span,
        false,
        symbols,
        ctx,
    );

    // Same awaited-value modelling as the non-optional path: `result?.catch(...)`
    // on a `Promise<void> | undefined` lands on `void`, not on a missing member.
    if matches!(property_name, "then" | "catch" | "finally")
        && !base_type.is_unknown()
        && !declares_own_property(&base_type, property_name)
        && !lib_promise_lacks(property_name, ctx)
    {
        let continued = if property_name == "then" {
            check_promise_then_call(base_type, arguments, symbols, ctx)
        } else {
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            Some(base_type)
        };
        return continued.map(|ty| super::with_chain_undefined(ty, &object_type));
    }

    match base_type {
        Type::ErrorType => {
            evaluate_arguments_on_error_type(arguments, symbols, ctx);
            Some(Type::ErrorType)
        }
        Type::Any => {
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            Some(Type::Any)
        }
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
            evaluate_arguments_context_free(object, arguments, symbols, ctx);
            None
        },
        Type::Array(element_type) if property_name == "map" => check_array_map_call(
            element_type.as_ref(),
            property_span,
            call_span,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        )
        .map(|ret| super::with_chain_undefined(ret, &object_type)),
        Type::Array(element_type) if property_name == "find" => check_array_find_call(
            element_type.as_ref(),
            property_span,
            call_span,
            arguments,
            symbols,
            ctx,
        )
        .map(|ret| super::with_chain_undefined(ret, &object_type)),
        Type::Union(union_type) => {
            let mut result_types = vec![];
            for ty in union_type.types() {
                if matches!(ty, Type::Undefined | Type::Null) {
                    result_types.push(Type::Undefined);
                    continue;
                }

                if property_name == "map"
                    && let Type::Array(element_type) = ty
                {
                    let mapped = check_array_map_call(
                        element_type.as_ref(),
                        property_span,
                        call_span,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    )?;
                    result_types.push(mapped);
                    continue;
                }

                let Some(property_type) = receiver_member_type(ty, property_name, ctx) else {
                    // A member whose reference peels to the sentinel is a shape
                    // surge could not reconstruct, not a type without the member.
                    if ty.peeled().is_unknown()
                        || crate::checks::expr::carries_leaked_type_parameter(ty, ctx)
                    {
                        return None;
                    }
                    if no_lib_array_member(ty, ctx) {
                        result_types.push(Type::Any);
                        continue;
                    }
                    let diagnostic =
                        Diagnostic::ts2339(
                            property_name,
                            &Type::Union(union_type.clone()).name(),
                            ctx.file_name.clone(),
                        );
                    ctx.push(diagnostic_with_syntax_span(
                        diagnostic,
                        crate::spans::choose_span(property_span, object_span),
                    ));
                    return None;
                };

                let property_type_base = surge_ts_types::remove_nullish(&property_type);

                let declared_member = property_type_base.clone();
                match callable_property_signature(property_type_base) {
                    Type::Function(function_type) => {
                        if super::report_type_argument_arity(
                            &Type::Function(function_type.clone()),
                            type_arguments,
                            super::type_arguments_start(property_span),
                            false,
                            ctx,
                        ) {
                            evaluate_arguments_context_free(object, arguments, symbols, ctx);
                            return None;
                        }
                        let function_type = instantiate_declared_member_signature(
                            &function_type,
                            Some(&declared_member),
                            type_arguments,
                            property_span,
                            arguments,
                            expected_return_type,
                            symbols,
                            ctx,
                        );
                        let return_type = check_function_type_call(
                            &function_type,
                            property_span,
                            call_span,
                            type_arguments,
                            arguments,
                            expected_return_type,
                            symbols,
                            ctx,
                        )?;
                        result_types.push(return_type);
                    }
                    Type::Any => result_types.push(Type::Any),
                    Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => return None,
                    // tsc's `isUntypedFunctionCall`: no signatures, but a `Function`.
                    other if surge_ts_types::is_untyped_function_callee(&other) => {
                        for argument in arguments {
                            let _ = crate::checks::expr::evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                        }
                        result_types.push(Type::Any);
                    }
                    other => {
                        ctx.push(diagnostic_with_syntax_span(
                            super::not_callable_diagnostic(&other, ctx),
                            crate::spans::choose_span(
                                property_span,
                                crate::spans::choose_span(call_span, object_span),
                            ),
                        ));
                        return None;
                    }
                }
            }

            Some(super::with_chain_undefined(
                surge_ts_types::union_type(result_types),
                &object_type,
            ))
        }
        _ => {
            if property_name == "map"
                && let Type::Array(element_type) = &base_type
            {
                return check_array_map_call(
                    element_type.as_ref(),
                    property_span,
                    call_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                )
                .map(|ret| super::with_chain_undefined(ret, &object_type));
            }

            if property_name == "find"
                && let Type::Array(element_type) = &base_type
            {
                return check_array_find_call(
                    element_type.as_ref(),
                    property_span,
                    call_span,
                    arguments,
                    symbols,
                    ctx,
                )
                .map(|ret| super::with_chain_undefined(ret, &object_type));
            }

            let Some(property_type) = receiver_member_type(&base_type, property_name, ctx) else {
                if no_lib_array_member(&base_type, ctx) {
                    return Some(Type::Any);
                }
                // Same rule as the plain-call arms: a reference that peels to the
                // sentinel is a shape surge could not reconstruct, not a type
                // without the member.
                if base_type.peeled().is_unknown()
                    || crate::checks::expr::carries_leaked_type_parameter(&base_type, ctx)
                {
                    return None;
                }
                if let Some(diagnostic) =
                    crate::checks::expr::global_this_missing_member(property_name, &base_type, ctx)
                {
                    if let Some(diagnostic) = diagnostic {
                        ctx.push(diagnostic_with_syntax_span(
                            diagnostic,
                            crate::spans::choose_span(property_span, object_span),
                        ));
                    }
                    return Some(Type::Any);
                }
                // `reportNonexistentProperty`, as a plain call reports it: a
                // static member, a later lib's member (TS2550), then a spelling.
                let diagnostic =
                    crate::checks::expr::missing_property_diagnostic(property_name, &base_type, symbols, ctx);
                ctx.push(diagnostic_with_syntax_span(
                    diagnostic,
                    crate::spans::choose_span(property_span, object_span),
                ));
                check_error_call_operands(type_arguments, arguments, symbols, ctx);
                return Some(Type::ErrorType);
            };

            let property_type_base = surge_ts_types::remove_nullish(&property_type);

            let declared_member = property_type_base.clone();
            match callable_property_signature(property_type_base) {
                Type::Function(function_type) => {
                    if super::report_type_argument_arity(
                        &Type::Function(function_type.clone()),
                        type_arguments,
                        super::type_arguments_start(property_span),
                        false,
                        ctx,
                    ) {
                        evaluate_arguments_context_free(object, arguments, symbols, ctx);
                        return None;
                    }
                    let function_type = instantiate_declared_member_signature(
                        &function_type,
                        Some(&declared_member),
                        type_arguments,
                        property_span,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    );
                    check_function_type_call(
                        &function_type,
                        property_span,
                        call_span,
                        type_arguments,
                        arguments,
                        expected_return_type,
                        symbols,
                        ctx,
                    )
                    .map(|ret| super::with_chain_undefined(ret, &object_type))
                }
                Type::ErrorType => {
                    evaluate_arguments_on_error_type(arguments, symbols, ctx);
                    Some(Type::ErrorType)
                }
                Type::Any => {
                    evaluate_arguments_context_free(object, arguments, symbols, ctx);
                    Some(Type::Any)
                }
                Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
                    evaluate_arguments_context_free(object, arguments, symbols, ctx);
                    None
                },
                // tsc's `isUntypedFunctionCall`: no signatures, but a `Function`.
                other if surge_ts_types::is_untyped_function_callee(&other) => {
                    for argument in arguments {
                        let _ = crate::checks::expr::evaluate_expression(&argument.expression, argument.span, symbols, ctx);
                    }
                    Some(Type::Any)
                }
                other => {
                    ctx.push(diagnostic_with_syntax_span(
                        super::not_callable_diagnostic(&other, ctx),
                        crate::spans::choose_span(
                            property_span,
                            crate::spans::choose_span(call_span, object_span),
                        ),
                    ));
                    None
                }
            }
        }
    }
}

/// A member typed by `typeof fn` over a generic declaration carries that
/// declaration (see the `typeof` arm of `resolve_parsed_type`); the call
/// instantiates its type parameters from the arguments exactly as a call on the
/// declared symbol would, so `vi.fn()` binds `T` (to its default) instead of
/// returning `Mock<T>` with the parameter bare. Any other function-typed member
/// is used as resolved.
pub(super) fn instantiate_declared_member_signature<'a>(
    function_type: &'a surge_ts_types::FunctionType,
    declared_member: Option<&Type>,
    type_arguments: &[ParsedType],
    type_argument_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> std::borrow::Cow<'a, surge_ts_types::FunctionType> {
    // An overload group is resolved per candidate by the call (tsc's
    // `chooseOverload`); a signature recovered for the group is its first
    // member's, and instantiating that answered every call with the first
    // member's return (`queryOptions()` against an overload requiring options).
    if function_type.overloads().is_some() {
        return std::borrow::Cow::Borrowed(function_type);
    }
    if let Some(member) = function_type
        .declaration()
        .and_then(|declaration| declaration.downcast_ref::<super::DeclaredMemberSignature>())
    {
        // A written signature that names something the capture did not bind
        // (an enclosing parameter the body was resolved under by scope, not by
        // substitution) reports it here as an unresolved name. That report is
        // about the recovery, not the source: drop it and keep the resolved
        // handle, exactly as an unattached signature behaves.
        let diagnostics_before = ctx.diagnostics().len();
        let instantiated = surge_ts_types::with_type_copy_reason(
            surge_ts_types::TypeCopyReason::CallResolution,
            || {
                super::instantiate::instantiate_function_type(
                    function_type,
                    Some(&member.signature),
                    &member.outer_type_arguments,
                    type_arguments,
                    type_argument_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                )
            },
        );
        if ctx.diagnostics().len() != diagnostics_before {
            ctx.truncate_diagnostics(diagnostics_before);
            return std::borrow::Cow::Borrowed(function_type);
        }
        return instantiated;
    }
    let declared_signature = function_type
        .declaration()
        .and_then(|declaration| declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>())
        .filter(|signature| !signature.type_parameters.is_empty());
    if let Some(signature) = declared_signature {
        return surge_ts_types::with_type_copy_reason(
            surge_ts_types::TypeCopyReason::CallResolution,
            || {
                super::instantiate::instantiate_function_type(
                    function_type,
                    Some(signature),
                    &[],
                    type_arguments,
                    type_argument_span,
                    arguments,
                    expected_return_type,
                    symbols,
                    ctx,
                )
            },
        );
    }
    // A member typed by an interface or alias carrying a generic call signature
    // has no declaration on its handle at all: `resolve_function_type` erases
    // the signature's type parameters to the degradation sentinel and keeps
    // only their rendering. The written signature is read back off the
    // declaration the member names, the same recovery the bare-call path runs.
    let Some(written) = declared_member
        .filter(|_| super::written_call_signature_recovery_enabled())
        .and_then(|declared| super::interface_call_signature_info(declared, ctx))
    else {
        return std::borrow::Cow::Borrowed(function_type);
    };
    surge_ts_types::with_type_copy_reason(surge_ts_types::TypeCopyReason::CallResolution, || {
        super::instantiate::instantiate_function_type(
            function_type,
            Some(&written.signature),
            &written.outer_type_arguments,
            type_arguments,
            type_argument_span,
            arguments,
            expected_return_type,
            symbols,
            ctx,
        )
    })
}

/// The return type of calling a callable member, instantiated. The inference
/// side reads a member's return type straight off the resolved handle, which for
/// a generic call signature still carries the erased type parameters — an object
/// literal holding a nested builder call (`t.router({ post: t.router({…}) })`)
/// is inferred there, so without this the outer call sees an unresolved argument
/// and abandons its own instantiation.
pub(crate) fn callable_member_call_return_type(
    member_type: &Type,
    type_arguments: &[ParsedType],
    property_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Type::Function(declared) = callable_property_signature(member_type.clone()) else {
        return None;
    };
    let function_type = instantiate_declared_member_signature(
        &declared,
        Some(member_type),
        type_arguments,
        property_span,
        arguments,
        None,
        symbols,
        ctx,
    );
    Some(
        overloaded_member_call_return_type(
            &declared,
            type_arguments,
            property_span,
            arguments,
            symbols,
            ctx,
        )
        .unwrap_or_else(|| function_type.return_type().clone()),
    )
}

/// The return type of the overload an inferred member call lands on: tsc's
/// `chooseOverload`, as a checked call runs it — the candidates in declaration
/// order, each with the right arity, a generic one instantiated for this call.
/// Reading only non-generic picks left a generic group at its first member's
/// return, which a call too short for it cannot resolve to
/// (`queryOptions()` against a first overload that requires its options).
pub(crate) fn overloaded_member_call_return_type(
    declared: &surge_ts_types::FunctionType,
    type_arguments: &[ParsedType],
    property_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let argument_types = super::inferred_argument_shapes(declared, arguments, symbols, ctx)?;
    super::choose_overload_return_type(
        declared,
        &argument_types,
        type_arguments,
        property_span,
        arguments,
        None,
        Vec::new(),
        symbols,
        ctx,
    )
}

/// Under `noLib` the array member surface comes from the configured replacement
/// lib (roblox-ts's `Array` adds `size`/`push`/`pop`/… absent from the standard
/// JS array surface). surge collapses that interface to `Type::Array` for
/// assignability, discarding its member set, so a method the std surface does not
/// know is not a real typo here — resolve it permissively instead of emitting
/// TS2339. Without `noLib` the std array surface is authoritative.
/// A property typed as a callable object — a type literal carrying call
/// signatures, like expect-type's `toEqualTypeOf: { <E>(v: E): true; <E>(): true }`
/// — is invoked like a function. Surface its call signature so the property-call
/// match treats it as callable instead of a false TS2349.
/// tsc reads `z.f` off a union receiver as the union of the members' `f`
/// (`getPropertyOfType` on the union) and calls that union, whose signatures
/// are `getUnionSignatures` of the members'. `None` leaves the call to the
/// member-by-member walk: a nullish member, an array (whose members tsc reads
/// off the element union, `getArrayMemberCallSignatures`), a member that is no
/// plain signature, or a generic one, which is instantiated per member there.
fn union_receiver_callee(receiver: &surge_ts_types::UnionType, property_name: &str) -> Option<surge_ts_types::UnionType> {
    let mut members = Vec::with_capacity(receiver.types().len());
    for ty in receiver.types() {
        if matches!(ty, Type::Undefined | Type::Null)
            || matches!(ty.peeled(), Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
        {
            return None;
        }
        let property = ty.get_property_access_type(property_name)?;
        let Type::Function(signature) = callable_property_signature(property.clone()) else {
            return None;
        };
        let mut signatures = Vec::new();
        signature.push_overload_members(&mut signatures);
        if signatures
            .iter()
            .any(|signature| !super::own_type_parameter_names(signature).is_empty())
        {
            return None;
        }
        members.push(property);
    }
    match union_type(members) {
        Type::Union(union) => Some(union),
        _ => None,
    }
}

fn callable_property_signature(ty: Type) -> Type {
    let signature = match &ty {
        Type::Object(object) => object.call_signature().cloned(),
        Type::Reference(_) => match ty.peeled() {
            Type::Object(object) => object.call_signature().cloned(),
            // A reference resolving to a function IS the callable — surface it
            // like the object call-signature case, or the caller's variant
            // match sees the unpeeled reference and misreports TS2349 (lazy
            // value annotations wrap plain function-typed `declare const`s).
            Type::Function(function) => Some(function),
            // A reference resolving to `any`, the sentinel, or a union of
            // signatures is judged by what it resolves to, as the unwrapped
            // types are (an inferred class member is such a reference).
            resolved @ (Type::Any
            | Type::Unknown
            | Type::ErrorType
            | Type::TypeParameter(_)
            | Type::Union(_)) => return resolved,
            _ => None,
        },
        _ => None,
    };
    match signature {
        Some(signature) => Type::Function(signature),
        None => ty,
    }
}

fn no_lib_array_member(object_type: &Type, ctx: &CheckerContext) -> bool {
    ctx.options.no_lib && matches!(object_type, Type::Array(_))
}

/// The receiver a method call on a type variable is resolved against: its
/// apparent type (the constraint), which reports a member it lacks there. tsc
/// instantiates the constraint's `this` with the variable
/// (`getTypeWithThisArgument`), which surge cannot, so a member that reads the
/// polymorphic `this` is left to the variable and its call result unknown
/// rather than the constraint's own type.
pub(crate) fn type_variable_call_receiver(receiver: Type, property_name: &str) -> Type {
    match type_variable_apparent_type(&receiver) {
        Some(apparent)
            if !apparent
                .get_property_access_type(property_name)
                .is_some_and(|member| member_reads_polymorphic_this(&member)) =>
        {
            apparent
        }
        _ => match non_nullable_array_variable(&receiver) {
            Some(array) => array,
            None => receiver,
        },
    }
}

fn member_reads_polymorphic_this(member: &Type) -> bool {
    let mut signatures = Vec::new();
    match member.peeled() {
        Type::Function(function) => function.push_overload_members(&mut signatures),
        Type::Object(object) => match object.call_signature() {
            Some(signature) => signature.push_overload_members(&mut signatures),
            None => return false,
        },
        _ => return false,
    }
    signatures.iter().any(|signature| {
        signature.returns_polymorphic_this()
            || signature.return_mentions_polymorphic_this()
            || signature.has_polymorphic_this()
    })
}

/// `T & {}` (a type variable proven non-nullish, `values!`) whose constraint
/// is an array once its nullish members are gone: the array its methods are
/// called on.
fn non_nullable_array_variable(receiver: &Type) -> Option<Type> {
    let Type::Object(object) = receiver else {
        return None;
    };
    let [Type::TypeParameter(parameter), Type::Object(empty)] = object.intersection_operands.as_deref()? else {
        return None;
    };
    if !surge_ts_types::type_variable::is_narrowed_type_variable(receiver) || !empty.properties.is_empty() {
        return None;
    }
    let constraint = surge_ts_types::type_variable::active_constraint(parameter)??;
    match surge_ts_types::remove_nullish(&constraint).peeled() {
        array @ (Type::Array(_) | Type::Tuple(_)) => Some(array),
        _ => None,
    }
}

/// The constraint a type variable of the body being checked is read through,
/// followed through variables it names. `None` for anything else, or when
/// the chain ends without a constraint surge models. A union constraint is
/// narrowed where tsgo reads it (`getNarrowableTypeForReference`), which
/// surge does not follow, so it answers nothing either.
pub(crate) fn type_variable_apparent_type(ty: &Type) -> Option<Type> {
    if !ty.is_type_variable() {
        return None;
    }
    let mut current = ty.clone();
    for _ in 0..8 {
        let Type::TypeParameter(parameter) = &current else {
            return (!current.is_unknown()
                && !matches!(current, Type::Any)
                && !matches!(current.peeled(), Type::Union(_)))
            .then_some(current);
        };
        if surge_ts_types::type_variable::deferred_type(parameter).is_some() {
            return None;
        }
        current = surge_ts_types::type_variable::active_constraint(parameter)??;
    }
    None
}

/// Whether the configured lib's `Promise` lacks `name` (`finally` before
/// es2018), which the awaited-value modelling must not answer then.
pub(crate) fn lib_promise_lacks(name: &str, ctx: &CheckerContext) -> bool {
    if name != "finally" || ctx.options.no_lib {
        return false;
    }
    match ctx.lookup_type_declaration("Promise") {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => {
            ctx.is_library_scoped_file(&info.file_name)
                && !info.body.members.is_empty()
                && !info.body.members.iter().any(|member| member.name == name)
        }
        _ => false,
    }
}
