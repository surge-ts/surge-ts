//! Operand rules for `delete` and for `++`/`--`, which tsc checks in
//! `checkDeleteExpression` and `checkPre/PostfixUnaryExpression`
//! (`tsc/internal/checker/checker.go`). Both operators write through their
//! operand, so both reject a `readonly` or `const` target; `delete`
//! additionally requires a property reference whose type admits `undefined`.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, TextSpan as SyntaxTextSpan};
use surge_ts_types::{Type, union_type};

use crate::checks::function::{ReadonlyElementWrite, property_write_is_readonly, readonly_element_write};
use crate::context::{CheckerContext, convert_span};
use crate::infer::{InferredExpression, infer_expression};
use crate::symbols::SymbolTable;

/// The member a `delete` or update operand names, with the receiver it is read
/// from. `None` for anything that is not a property reference.
fn operand_property<'a>(
    operand: &'a ParsedExpression,
) -> Option<(&'a ParsedExpression, PropertyKey<'a>)> {
    match operand {
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => Some((object, PropertyKey::Name(property_name))),
        ParsedExpression::ElementAccess { object, index, .. } => {
            Some((object, PropertyKey::Computed(index)))
        }
        _ => None,
    }
}

enum PropertyKey<'a> {
    Name(&'a str),
    Computed(&'a ParsedExpression),
}

impl PropertyKey<'_> {
    /// The literal member name this key resolves to, when it has one. A
    /// computed key that is not a literal names no single member, so the
    /// member-level rules cannot apply to it.
    fn literal_name(&self, symbols: &SymbolTable, ctx: &mut CheckerContext) -> Option<String> {
        match self {
            PropertyKey::Name(name) => Some((*name).to_string()),
            PropertyKey::Computed(index) => match infer_expression(index, symbols, ctx) {
                InferredExpression::Known(Type::StringLiteral(value)) => Some(value.to_string()),
                InferredExpression::Known(Type::NumberLiteral(value)) => Some(value.value),
                _ => None,
            },
        }
    }
}

/// An `IndexAccess` names its object by bare identifier rather than carrying an
/// expression, so it needs its own receiver lookup.
fn index_access_receiver(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Type, String)> {
    let ParsedExpression::IndexAccess {
        object_name, index, ..
    } = operand
    else {
        return None;
    };
    let receiver = symbols.get(object_name)?.ty.clone();
    let name = match infer_expression(index, symbols, ctx) {
        InferredExpression::Known(Type::StringLiteral(value)) => value.to_string(),
        InferredExpression::Known(Type::NumberLiteral(value)) => value.value,
        _ => return None,
    };
    Some((receiver, name))
}

/// The receiver type and member name a write through `operand` targets.
fn write_target(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Type, String)> {
    if let Some(target) = index_access_receiver(operand, symbols, ctx) {
        return Some(target);
    }
    let (object, key) = operand_property(operand)?;
    let name = key.literal_name(symbols, ctx)?;
    let receiver = match infer_expression(object, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        _ => return None,
    };
    Some((receiver, name))
}

/// `o[k]` written through a key no literal names, with whether the index
/// signature it lands on is the numeric one. An unresolved key is tsc's error
/// type, which a non-string index signature answers first
/// (`findApplicableIndexInfo`).
fn non_literal_index_write(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Type, bool)> {
    let (receiver, index) = match operand {
        ParsedExpression::IndexAccess { object_name, index, .. } => (symbols.get(object_name)?.ty.clone(), index),
        ParsedExpression::ElementAccess { object, index, .. } => match infer_expression(object, symbols, ctx) {
            InferredExpression::Known(ty) => (ty, index),
            _ => return None,
        },
        _ => return None,
    };
    let checkpoint = ctx.diagnostics().len();
    let key = infer_expression(index, symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    let key_is_numeric = match key {
        InferredExpression::Known(Type::Number) => true,
        InferredExpression::Known(Type::String) => false,
        InferredExpression::UnresolvedIdentifier { .. } => true,
        _ => return None,
    };
    Some((receiver, key_is_numeric))
}

/// Where tsc anchors a member-level diagnostic about `operand`: the property
/// name alone, not the whole access expression.
fn property_name_span(operand: &ParsedExpression) -> Option<SyntaxTextSpan> {
    match operand {
        ParsedExpression::PropertyAccess { property_span, .. }
        | ParsedExpression::OptionalPropertyAccess { property_span, .. } => *property_span,
        ParsedExpression::IndexAccess { index_span, .. }
        | ParsedExpression::ElementAccess { index_span, .. } => *index_span,
        _ => None,
    }
}

/// `delete o.p`. Mirrors `checkDeleteExpression`: the operand must be a property
/// reference, must not be a private identifier, and must be either `readonly`
/// (rejected) or optional.
pub(crate) fn check_delete_operand(
    operand: &ParsedExpression,
    operand_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let Some(span) = operand_span else {
        return;
    };

    // An operand surge did not model arrives as `Unknown`: reporting a
    // *shape* rule on it would be a false positive about what may well be a
    // property reference.
    if matches!(operand, ParsedExpression::Unknown) {
        return;
    }

    let is_property_reference = matches!(
        operand,
        ParsedExpression::PropertyAccess { .. }
            | ParsedExpression::OptionalPropertyAccess { .. }
            | ParsedExpression::IndexAccess { .. }
            | ParsedExpression::ElementAccess { .. }
    );
    if !is_property_reference {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts2703(file_name).with_span(convert_span(span)));
        return;
    }
    // The member is still checked: tsc goes on to the read-only and optional
    // rules.
    if let ParsedExpression::PropertyAccess { property_name, is_bracketed: false, .. } = operand
        && surge_ts_types::private_name::is_private_name_key(property_name)
    {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts18011(file_name).with_span(convert_span(span)));
    }

    // A delete target is written through (`isDeleteTarget`): a `readonly`
    // tuple element is TS2704 like any read-only property, and a numeric index
    // into a `readonly` array lands on its index signature, TS2542 on the
    // whole access (`errorIfWritingToReadonlyIndex`).
    if let Some(write) = deleted_readonly_element(operand, symbols, ctx) {
        let file_name = ctx.file_name.clone();
        let diagnostic = match write {
            ReadonlyElementWrite::Property(_) => Diagnostic::ts2704(file_name),
            ReadonlyElementWrite::IndexSignature(name) => Diagnostic::ts2542(name, file_name),
        };
        ctx.push(diagnostic.with_span(convert_span(span)));
        return;
    }

    let Some((receiver, property_name)) = write_target(operand, symbols, ctx) else {
        return;
    };

    if property_write_is_readonly(&receiver, &property_name, ctx) {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts2704(file_name).with_span(convert_span(span)));
        return;
    }

    // `errorIfWritingToReadonlyIndex`: a delete target is written, and a key
    // no member answers goes through the receiver's index signature.
    if receiver.writes_readonly_index(
        Some(property_name.as_str()),
        surge_ts_types::is_numeric_key(&property_name),
    ) {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts2542(receiver.name(), file_name).with_span(convert_span(span)));
        return;
    }

    // tsc's `checkDeleteExpressionMustBeOptional` runs under `strictNullChecks`
    // only: without it `undefined` is in every type's domain.
    if !ctx.options.strict_null_checks {
        return;
    }
    let unnarrowed = unnarrowed_receiver(operand, symbols, ctx).unwrap_or(receiver);
    // A member only the apparent `Object` has (`toString`) is a method, never
    // optional.
    let apparent_member = || match unnarrowed.peeled() {
        Type::Object(object)
            if !object.synthetic_open_index
                && object.string_index_type.is_none()
                && object.get_property(&property_name).is_none() =>
        {
            surge_ts_types::object_prototype_member_type(&property_name)
        }
        _ => None,
    };
    let Some(property_type) = declared_property_type(&unnarrowed, &property_name).or_else(apparent_member) else {
        return;
    };
    // `any`, `unknown` and `never` are exempt, as is anything surge failed to
    // model — the operand rule must not fire on a type we did not resolve.
    if matches!(
        property_type,
        Type::Any | Type::GenuineUnknown | Type::Never | Type::Unknown
    ) {
        return;
    }
    // `exactOptionalPropertyTypes` asks for the optional modifier itself: a
    // written `undefined` no longer makes a member deletable.
    let optional = if surge_ts_types::exact_optional_property_types() {
        declared_property_optional(&unnarrowed, &property_name)
    } else {
        type_includes_undefined(&property_type)
    };
    if !optional {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts2790(file_name).with_span(convert_span(span)));
    }
}

/// Whether the member the receiver declares is optional; a union's is when
/// any constituent's is, as tsc's synthetic union property records.
fn declared_property_optional(receiver: &Type, property_name: &str) -> bool {
    match receiver.peeled() {
        Type::Object(object) => object
            .properties
            .get(property_name)
            .is_none_or(|property| property.optional),
        Type::Union(union) => union
            .types()
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Null))
            .any(|member| declared_property_optional(member, property_name)),
        _ => true,
    }
}

fn deleted_readonly_element(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<ReadonlyElementWrite> {
    let (receiver, index) = match operand {
        ParsedExpression::ElementAccess { object, index, .. } => match infer_expression(object, symbols, ctx) {
            InferredExpression::Known(receiver) => (receiver, index),
            _ => return None,
        },
        ParsedExpression::IndexAccess { object_name, index, .. } => (symbols.get(object_name)?.ty.clone(), index),
        _ => return None,
    };
    let InferredExpression::Known(index_type) = infer_expression(index, symbols, ctx) else {
        return None;
    };
    if index_type.is_unknown() || !surge_ts_types::is_assignable_to(&index_type, &Type::Number) {
        return None;
    }
    readonly_element_write(&receiver, &index_type)
}

/// The receiver type as *declared*, undoing flow narrowing along the access
/// path. `checkDeleteExpressionMustBeOptional` reads `getTypeOfSymbol(symbol)`,
/// which is the member's declaration, so the idiomatic `if (o.p) delete o.p`
/// must still see `p?: T` as optional even though the guard narrowed it.
/// `None` where nothing was narrowed, leaving the flow receiver in place.
fn unnarrowed_receiver(
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let (object, _) = operand_property(operand)?;
    declared_expression_type(object, symbols, ctx)
}

fn declared_expression_type(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    match expression {
        ParsedExpression::Identifier { name, .. } => symbols.declared_type(name).cloned(),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        }
        | ParsedExpression::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => {
            let receiver = declared_expression_type(object, symbols, ctx)?;
            let Type::Object(object_type) = receiver.peeled() else {
                return None;
            };
            Some(object_type.properties.get(property_name.as_str())?.ty.clone())
        }
        _ => None,
    }
}

fn declared_property_type(receiver: &Type, property_name: &str) -> Option<Type> {
    match receiver.peeled() {
        Type::Object(object) => {
            let property = object.properties.get(property_name)?;
            Some(if property.optional {
                union_type(vec![property.ty.clone(), Type::Undefined])
            } else {
                property.ty.clone()
            })
        }
        // The member of what the receiver is once it is not nullish (an
        // optional chain's short-circuit included), declared by every member.
        Type::Union(union) => {
            let members: Vec<&Type> = union
                .types()
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null))
                .collect();
            if members.is_empty() || members.len() == union.types().len() && members.len() > 1 {
                return None;
            }
            let types = members
                .into_iter()
                .map(|member| declared_property_type(member, property_name))
                .collect::<Option<Vec<_>>>()?;
            Some(union_type(types))
        }
        _ => None,
    }
}

fn type_includes_undefined(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Any | Type::Void => true,
        Type::Union(union) => union.types().iter().any(type_includes_undefined),
        _ => false,
    }
}

/// tsc's `isAssignmentToReadonlyEntity` for a member write target: TS2540 on
/// a read-only member, and on every member of a namespace import
/// (`import * as ns`), which the module object never lets anyone write. A
/// member the module does not export is not a read-only write (its read
/// reports it); nor is a member of a local that shadows the import.
pub(crate) fn report_readonly_member_write(
    target: &ParsedExpression,
    span: SyntaxTextSpan,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let Some((receiver, property_name)) = write_target(target, symbols, ctx) else {
        return false;
    };
    let through_namespace_import = operand_property(target).is_some_and(|(object, _)| {
        matches!(object, ParsedExpression::Identifier { name, .. }
            if ctx.is_namespace_import_binding(name)
                && symbols.get(name).is_some_and(|visible| {
                    // A module-level statement is evaluated against the module
                    // table itself, lent out of the context meanwhile.
                    ctx.symbols.get(name).is_none_or(|module| visible.ty == module.ty)
                }))
    }) && receiver.get_property_access_type(&property_name).is_some();
    if !through_namespace_import && !property_write_is_readonly(&receiver, &property_name, ctx) {
        return false;
    }
    let file_name = ctx.file_name.clone();
    let anchor = property_name_span(target).unwrap_or(span);
    let property_name = surge_ts_types::private_name::display(&property_name);
    ctx.push(Diagnostic::ts2540(property_name, file_name).with_span(convert_span(anchor)));
    true
}

/// `x++` / `--o.p`. Mirrors the `++`/`--` arms of `checkPre/PostfixUnaryExpression`:
/// the write is rejected first (tsc's `checkIdentifier` reports it and hands back
/// the error type, which then satisfies the arithmetic check), and only an
/// accepted target has its operand type checked. An operand that is no
/// reference at all (`rejected_target`) is reported by `checkReferenceExpression`
/// only once that arithmetic check passes.
pub(crate) fn check_update_operand(
    operand: &ParsedExpression,
    operand_span: Option<SyntaxTextSpan>,
    rejected_target: bool,
    operand_result: &InferredExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let Some(span) = operand_span else {
        return;
    };
    let report_rejected_target = |ctx: &mut CheckerContext| {
        if rejected_target {
            let file_name = ctx.file_name.clone();
            let diagnostic = if is_optional_chain(operand) {
                Diagnostic::ts2777(file_name)
            } else {
                Diagnostic::ts2357(file_name)
            };
            ctx.push(diagnostic.with_span(convert_span(span)));
        }
    };

    if let ParsedExpression::Identifier { name, .. } = operand {
        if let Some(symbol) = symbols.get(name) {
            if let Some(diagnostic) =
                crate::checks::assign::unwritable_binding_diagnostic(name, &symbol, ctx)
            {
                ctx.push(diagnostic.with_span(convert_span(span)));
                return;
            }
        }
    } else if report_readonly_member_write(operand, span, symbols, ctx) {
        return;
    } else if let Some((receiver, property_name)) = write_target(operand, symbols, ctx) {
        if receiver.writes_readonly_index(
            Some(property_name.as_str()),
            surge_ts_types::is_numeric_key(&property_name),
        ) {
            // `errorIfWritingToReadonlyIndex`: the operand is written; the
            // arithmetic check below still runs on what it reads.
            let file_name = ctx.file_name.clone();
            ctx.push(Diagnostic::ts2542(receiver.name(), file_name).with_span(convert_span(span)));
        }
    } else if let Some((receiver, key_is_numeric)) = non_literal_index_write(operand, symbols, ctx)
        && receiver.writes_readonly_index(None, key_is_numeric)
    {
        let file_name = ctx.file_name.clone();
        ctx.push(Diagnostic::ts2542(receiver.name(), file_name).with_span(convert_span(span)));
    }

    let InferredExpression::Known(operand_type) = operand_result else {
        report_rejected_target(ctx);
        return;
    };

    // tsc runs `checkNonNullType` on the operand first, so an `unknown` or
    // possibly-undefined operand is reported as that and never as a bad
    // arithmetic operand.
    if *operand_type == Type::GenuineUnknown && super::report_unknown_operand(operand, Some(span), ctx) {
        report_rejected_target(ctx);
        return;
    }
    if super::maybe_emit_possibly_undefined_receiver(
        operand,
        operand_type,
        Some(span),
        None,
        symbols,
        ctx,
    ) {
        report_rejected_target(ctx);
        return;
    }

    if operand_type.is_unknown() || is_arithmetic_operand(operand_type) {
        report_rejected_target(ctx);
        return;
    }

    let file_name = ctx.file_name.clone();
    ctx.push(Diagnostic::ts2356(file_name).with_span(convert_span(span)));
}

/// tsc accepts an operand assignable to `number | bigint`, which `any` and a
/// numeric enum member both are. A union qualifies only if every constituent
/// does, ignoring `null`/`undefined`: tsc runs `checkNonNullType` first, so
/// `number | undefined` is reported as possibly-undefined and not as a bad
/// arithmetic operand.
fn is_arithmetic_operand(ty: &Type) -> bool {
    match ty {
        Type::Any => true,
        Type::Union(union) => union
            .types()
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Void))
            .all(is_arithmetic_operand),
        other => matches!(
            other.base_primitive(),
            Some(Type::Number) | Some(Type::BigInt)
        ),
    }
}

/// tsc's `getUnaryResultType`: a bigint operand keeps `bigint`, everything else
/// coerces to `number`.
pub(crate) fn update_result_type(operand_result: &InferredExpression) -> InferredExpression {
    match operand_result {
        InferredExpression::Known(ty) if matches!(ty.base_primitive(), Some(Type::BigInt)) => {
            InferredExpression::Known(Type::BigInt)
        }
        _ => InferredExpression::Known(Type::Number),
    }
}

/// tsc's `isOptionalChain`: the operand is, or continues, a `?.` chain.
fn is_optional_chain(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::OptionalPropertyAccess { .. }
        | ParsedExpression::OptionalPropertyCall { .. }
        | ParsedExpression::OptionalIndexAccess { .. }
        | ParsedExpression::OptionalCall { .. } => true,
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::ElementAccess { object, .. }
        | ParsedExpression::PropertyCall { object, .. } => is_optional_chain(object),
        _ => false,
    }
}
