//! Property access, index access, calls, and their optional-chaining variants.

use super::*;

use std::time::Instant;

use surge_ts_syntax::{ParsedExpression, TextSpan};
use surge_ts_types::{Type, union_type};

use crate::context::CheckerContext;
use crate::program::{record_program_timing, record_property_lookup};
use crate::symbols::SymbolTable;

use crate::infer::InferredExpression;

pub(crate) fn infer_index_access(
    object_name: &str,
    object_span: &Option<TextSpan>,
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if let Some(narrowed) = narrowed_element_read_named(object_name, index, symbols) {
        return InferredExpression::Known(narrowed);
    }
    let Some(symbol) = symbols.get(object_name) else {
        return InferredExpression::UnresolvedIdentifier {
            name: object_name.to_string(),
            span: *object_span,
        };
    };
    if indexes_const_enum_object(object_name, index, &symbol.ty, ctx) {
        return InferredExpression::Known(Type::ErrorType);
    }

    if let ParsedExpression::NumberLiteral(index_value) = index
        && let Type::OpenTuple(tuple) = symbol.ty.peeled()
        && let Some(element) = tuple.leading_element(index_value)
    {
        return InferredExpression::Known(element.clone());
    }
    let receiver_type = match &symbol.ty {
        Type::Reference(_) => match symbol.ty.peeled() {
            peeled @ (Type::Array(_) | Type::Tuple(_)) => peeled,
            _ => symbol.ty.clone(),
        },
        other => other.clone(),
    };
    if let Type::OpenTuple(tuple) = &receiver_type {
        return infer_open_tuple_index_access(tuple, index, index_span, symbols, ctx);
    }
    match &receiver_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::ErrorType => InferredExpression::Known(Type::ErrorType),
        Type::TypeParameter(_) => deferred_element_read(&receiver_type, index, symbols, ctx),
        Type::Unknown | Type::GenuineUnknown => InferredExpression::Unknown,
        // Lowered to its element array above.
        Type::OpenTuple(_) => InferredExpression::Unknown,
        Type::Union(union_type) => {
            if let ParsedExpression::NumberLiteral(key) = index
                && let crate::checks::expr::UnionKeyRead::Found(member) =
                    crate::checks::expr::union_numeric_key_read(union_type.types(), key, ctx)
            {
                return InferredExpression::Known(member);
            }
            infer_union_index_access(union_type.types(), index, index_span, symbols, ctx)
        }
        Type::Tuple(elements) => {
            infer_tuple_index_access(elements, index, index_span, symbols, ctx)
        }
        Type::Array(element_type) => {
            let element_type = element_type.as_ref();
            if element_type.is_unknown() && !element_type.is_type_variable() && *element_type != Type::GenuineUnknown {
                return InferredExpression::Unknown;
            }

            let index_type = match infer_expression(index, symbols, ctx) {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };

            if !crate::checks::expr::numeric_index_key(&index_type) {
                let _ = index_span;
                return InferredExpression::Unknown;
            }

            InferredExpression::Known(unchecked_index_read(element_type.clone(), ctx))
        }
        // `String` declares `readonly [index: number]: string`, so a numeric
        // index reads a `string` — and under `noUncheckedIndexedAccess` that is
        // `string | undefined` like any other index-signature read.
        Type::String | Type::StringLiteral(_) => {
            let index_type = match infer_expression(index, symbols, ctx) {
                InferredExpression::Known(ty) => ty,
                _ => return InferredExpression::Unknown,
            };
            if !crate::checks::expr::numeric_index_key(&index_type) {
                return InferredExpression::Unknown;
            }
            InferredExpression::Known(unchecked_index_read(Type::String, ctx))
        }
        Type::Function(_)
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Void
        | Type::Never
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Undefined
        | Type::Null => InferredExpression::Unknown,
        Type::Object(_) | Type::Reference(_) => {
            infer_object_element_read(&receiver_type, index, symbols, ctx)
        }
    }
}

/// tsc's `getIndexedAccessType` for an element access with a generic key
/// (`shouldDeferIndexedAccessType`): `obj[key]` with `obj: T` and `key: K` is
/// the deferred `T[K]`, related through its constraint. `None` for a key that
/// is not generic, which tsc resolves eagerly through the apparent type.
pub(crate) fn deferred_element_access(receiver: &Type, index: &Type) -> Option<Type> {
    let generic_index = match index {
        Type::Union(union) => union.types().iter().any(Type::is_type_variable),
        other => other.is_type_variable(),
    };
    if !receiver.is_type_variable() || !generic_index {
        return None;
    }
    surge_ts_types::type_variable::indexed_access_variable(receiver, index)
}

fn deferred_element_read(
    receiver_type: &Type,
    index: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let InferredExpression::Known(index_type) = super::infer_element_access_argument(index, symbols, ctx) else {
        return InferredExpression::Unknown;
    };
    deferred_element_access(receiver_type, &index_type)
        .or_else(|| crate::checks::expr::constraint_element_read(receiver_type, index, &index_type, symbols, ctx))
        .map_or(InferredExpression::Unknown, InferredExpression::Known)
}

fn infer_object_element_read(
    receiver_type: &Type,
    index: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let InferredExpression::Known(index_type) = infer_expression(index, symbols, ctx) else {
        return InferredExpression::Unknown;
    };
    match crate::checks::expr::object_element_read(receiver_type, index, &index_type, symbols, ctx) {
        Some(ty) => InferredExpression::Known(ty),
        None => InferredExpression::Unknown,
    }
}

/// An open tuple (`[number, string, ...boolean[]]`) has fixed positions in
/// front of its rest: `t[0]` is `number`, and anything at or past the rest
/// reads as what the rest and the trailing elements hold.
pub(crate) fn infer_open_tuple_index_access(
    tuple: &surge_ts_types::OpenTupleType,
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let index_type = match infer_expression(index, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => return InferredExpression::Unknown,
    };

    let past_fixed = || {
        let mut members = vec![tuple.rest.as_ref().clone()];
        members.extend(tuple.trailing.iter().cloned());
        union_type(members)
    };
    if let Some(index_value) = tuple_index_value(&index_type) {
        // `getTupleElementTypeOutOfStartCount`: past every fixed element the
        // read may run off the end, which `noUncheckedIndexedAccess` admits.
        return InferredExpression::Known(match tuple.leading.get(index_value) {
            Some(element) => element.clone(),
            None if index_value >= tuple.fixed_len() => unchecked_index_read(past_fixed(), ctx),
            None => past_fixed(),
        });
    }

    if crate::checks::expr::numeric_index_key(&index_type) {
        return InferredExpression::Known(unchecked_index_read(tuple.element_union(), ctx));
    }

    let _ = index_span;
    InferredExpression::Unknown
}

pub(crate) fn infer_tuple_index_access(
    elements: &[Type],
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let index_type = match infer_expression(index, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => return InferredExpression::Unknown,
    };

    // `getTupleElementTypeOutOfStartCount`: past a fixed tuple's end the read
    // is `undefined` (the check reports TS2493).
    if let Some(index_value) = tuple_index_value(&index_type) {
        return InferredExpression::Known(elements.get(index_value).cloned().unwrap_or(Type::Undefined));
    }

    if crate::checks::expr::numeric_index_key(&index_type) {
        return InferredExpression::Known(unchecked_index_read(union_type(elements.to_vec()), ctx));
    }

    let _ = index_span;
    InferredExpression::Unknown
}

/// tsc's `checkElementAccessExpression`: a const enum object indexed by
/// anything but a string literal is TS2476 (a grammar diagnostic) and the
/// access is the error type, so the object's reverse mapping never answers it.
pub(crate) fn indexes_const_enum_object(
    object_name: &str,
    index: &ParsedExpression,
    object_type: &Type,
    ctx: &CheckerContext,
) -> bool {
    let string_literal_like = match index {
        ParsedExpression::StringLiteral(_) => true,
        ParsedExpression::TemplateLiteral { expressions, .. } => expressions.is_empty(),
        _ => false,
    };
    !string_literal_like && is_const_enum_object(object_name, object_type, ctx)
}

/// tsc's `isConstEnumObjectType` for a binding: `object_name` names a const
/// enum and `object_type` is its object, not a value of the same name that
/// shadows it.
pub(crate) fn is_const_enum_object(object_name: &str, object_type: &Type, ctx: &CheckerContext) -> bool {
    let Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) = ctx.lookup_type_declaration(object_name) else {
        return false;
    };
    alias.enum_is_const
        && alias.enum_name.as_deref() == Some(object_name)
        && matches!(object_type, Type::Object(object)
            if object.alias_name.as_deref().and_then(|name| name.strip_prefix("typeof "))
                == Some(object_name))
}

/// The rest of tsc's `checkConstEnumAccess` for a const enum declared in a
/// declaration file: no single-file transpiler can inline its members
/// (TS2748). `verbatimModuleSyntax` leaves an imported one to the import
/// (`checkAliasSymbol`); the file's own enums are the grammar walk's.
pub(crate) fn report_ambient_const_enum_access(
    object_name: &str,
    span: Option<surge_ts_syntax::TextSpan>,
    object_type: &Type,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.isolated_modules
        && !(ctx.options.verbatim_module_syntax && !ctx.is_import_binding(object_name))
    {
        return;
    }
    if ctx.type_only_alias_use_valid || !is_const_enum_object(object_name, object_type, ctx) {
        return;
    }
    let Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) = ctx.lookup_type_declaration(object_name) else {
        return;
    };
    if *alias.file_name == *ctx.file_name || !crate::modules::is_declaration_file_name(&alias.file_name) {
        return;
    }
    let Some(span) = span else {
        return;
    };
    let flag = if ctx.options.verbatim_module_syntax { "verbatimModuleSyntax" } else { "isolatedModules" };
    ctx.push(
        surge_ts_diagnostics::Diagnostic::ts2748(flag, ctx.file_name.clone())
            .with_span(crate::context::convert_span(span)),
    );
}

/// `E.A` read off an enum's object is the enum member type `E.A`, not the
/// bare value it holds, so it keeps the enum's nominal identity: `F.X` is not
/// an `E` even when both are `0`. Only a member whose `E.A` alias resolves to
/// an enum reference is answered here; anything else falls back to the object.
pub(crate) fn enum_member_value_type(
    object: &ParsedExpression,
    property_name: &str,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let name = match object {
        ParsedExpression::Identifier { name, .. } => {
            // A local value of the same name shadows the enum.
            if symbols.get(name).is_some_and(|symbol| !matches!(symbol.kind, crate::symbols::SymbolKind::Const)) {
                return None;
            }
            name.clone()
        }
        // `M.E.A` names the namespace's enum the way `M.E` does as a type.
        // Inside a namespace block an enum merged across blocks resolves per
        // block, so its members are not read this way there.
        ParsedExpression::PropertyAccess {
            is_bracketed: false, ..
        } if ctx.namespace_block_prefix_stack.is_empty() => {
            let qualified = dotted_identifier_name(object)?;
            let head = qualified.split('.').next()?;
            if symbols.get(head).is_some_and(|symbol| {
                !matches!(symbol.kind, crate::symbols::SymbolKind::Const | crate::symbols::SymbolKind::Var)
            }) {
                return None;
            }
            qualified
        }
        _ => return None,
    };
    let name = &name;
    let crate::symbols::TypeDeclarationInfo::Alias(alias) = ctx.lookup_type_declaration(name)? else {
        return None;
    };
    let declared_enum = alias.enum_name.as_deref();
    if declared_enum != Some(name.as_str()) && declared_enum != name.rsplit('.').next() {
        return None;
    }
    let member_name = format!("{name}.{property_name}");
    ctx.lookup_type_declaration(&member_name)?;
    let member = crate::infer::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: member_name,
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    if !matches!(&member, Type::Reference(reference) if reference.enum_owner.is_some()) {
        return None;
    }
    // An enum merged across namespace blocks resolves its members per block,
    // so a qualified member may not belong to the enum the same name reads.
    if name.contains('.') {
        let enum_type = crate::infer::map_parsed_type(
            surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: name.clone(),
                span: None,
                type_arguments: Vec::new(),
            })),
            ctx,
        );
        if !surge_ts_types::is_assignable_to(&member, &enum_type) {
            return None;
        }
    }
    Some(member)
}

fn dotted_identifier_name(expression: &ParsedExpression) -> Option<String> {
    match expression {
        ParsedExpression::Identifier { name, .. } => Some(name.clone()),
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            is_bracketed: false,
            ..
        } => Some(format!("{}.{property_name}", dotted_identifier_name(object)?)),
        _ => None,
    }
}

/// tsc reads a dotted member of a const enum's object with
/// `skipObjectFunctionPropertyAugment` (`isConstEnumObjectType` in
/// `checkPropertyAccessExpressionOrQualifiedName`), so the global `Object`
/// members every other object answers are not there: `E.toString` is TS2339
/// on `typeof E`. The enum object's type when the read misses for that reason.
pub(crate) fn const_enum_object_lacks_member(
    object: &ParsedExpression,
    property_name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Type> {
    surge_ts_types::object_prototype_member_type(property_name)?;
    let ParsedExpression::Identifier { name, .. } = object else {
        return None;
    };
    let object_type = &symbols.get(name)?.ty;
    let Type::Object(enum_object) = object_type else {
        return None;
    };
    if enum_object.contains_property(property_name) {
        return None;
    }
    let crate::symbols::TypeDeclarationInfo::Alias(alias) = ctx.lookup_type_declaration(name)? else {
        return None;
    };
    // A value of the same name that is not the enum's object shadows it.
    let is_enum_object = enum_object
        .alias_name
        .as_deref()
        .and_then(|display| display.strip_prefix("typeof "))
        == Some(name.as_str());
    (alias.enum_is_const && alias.enum_name.as_deref() == Some(name.as_str()) && is_enum_object)
        .then(|| object_type.clone())
}

pub(crate) fn infer_property_access(
    object: &ParsedExpression,
    _object_span: &Option<TextSpan>,
    property_name: &str,
    property_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    record_property_lookup();
    let property_access_start = Instant::now();
    if let Some(member) = enum_member_value_type(object, property_name, symbols, ctx) {
        return InferredExpression::Known(member);
    }
    let object_type = match infer_expression(object, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        // tsc types a member the receiver lacks with its error type, and
        // everything read off that is the error type again.
        InferredExpression::MissingProperty { .. } => {
            return InferredExpression::Known(Type::ErrorType);
        }
        InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };

    let result = crate::program::with_dts_expansion_reason(
        crate::program::DtsExpansionReason::PropertyLookup,
        || match &object_type {
            Type::Any => InferredExpression::Known(Type::Any),
            Type::ErrorType => InferredExpression::Known(Type::ErrorType),
            Type::TypeParameter(parameter) if object_type.is_type_variable() => {
                match type_variable_member(parameter, &object_type, property_name, *property_span, 0) {
                    InferredExpression::Known(member) => InferredExpression::Known(widen_index_signature_read(
                        &object_type,
                        property_name,
                        crate::checks::function::bind_this_member(&object_type, member),
                        ctx,
                    )),
                    other => other,
                }
            }
            Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
                InferredExpression::Unknown
            }
            // The check pass reports the receiver; the access itself has no
            // type tsc would continue with.
            Type::Undefined | Type::Null => InferredExpression::Unknown,
            Type::Union(union_type) => {
                // A sentinel member means part of the receiver is unmodelled, so
                // a miss on any *other* member says nothing about the source —
                // the same no-cascade rule a wholly-sentinel receiver gets.
                if union_type
                    .types()
                    .iter()
                    .any(surge_ts_types::Type::is_unmodelled)
                {
                    return InferredExpression::Unknown;
                }
                if Type::union_property_declarations_differ(union_type.types(), property_name) {
                    return InferredExpression::MissingProperty {
                        property_name: property_name.to_string(),
                        object_type: object_type.clone(),
                        span: *property_span,
                    };
                }
                // `a?.b.c` keeps the chain's `undefined`; a plain `x.c` on a
                // possibly-`undefined` `x` is an error the check pass reports,
                // and tsc then types the access from the defined part alone.
                let keeps_undefined = object.continues_optional_chain();
                let mut result_types = vec![];
                for ty in union_type.types() {
                    // An optional chain short-circuits a `null` receiver to
                    // `undefined` too.
                    if matches!(ty, Type::Undefined | Type::Null) {
                        if keeps_undefined {
                            result_types.push(Type::Undefined);
                        }
                        continue;
                    }
                    // A type variable member reads through its apparent type.
                    if let Type::TypeParameter(parameter) = ty {
                        match type_variable_member(parameter, ty, property_name, *property_span, 0) {
                            InferredExpression::Known(member) => {
                                result_types.push(widen_index_signature_read(ty, property_name, member, ctx));
                                continue;
                            }
                            InferredExpression::MissingProperty { .. } => {
                                return InferredExpression::MissingProperty {
                                    property_name: property_name.to_string(),
                                    object_type: object_type.clone(),
                                    span: *property_span,
                                };
                            }
                            _ => return InferredExpression::Unknown,
                        }
                    }
                    match ty.get_property_access_type(property_name) {
                        Some(property_type) => result_types.push(widen_index_signature_read(
                            ty,
                            property_name,
                            crate::checks::function::bind_this_member(ty, property_type),
                            ctx,
                        )),
                        None if no_lib_array_member(ty, ctx) => result_types.push(no_lib_array_member_or_any(ty, property_name, ctx)),
                        // Same rule as the single-receiver arm below: a member
                        // whose reference peels to the sentinel is a shape surge
                        // could not reconstruct, not a type without the member.
                        None if ty.peeled().is_unknown()
                            || crate::checks::expr::carries_leaked_type_parameter(ty, ctx) =>
                        {
                            return InferredExpression::Unknown;
                        }
                        // tsc names the union the property was looked up on,
                        // not the member that happens to lack it.
                        None => {
                            return InferredExpression::MissingProperty {
                                property_name: property_name.to_string(),
                                object_type: object_type.clone(),
                                span: *property_span,
                            };
                        }
                    }
                }
                InferredExpression::Known(surge_ts_types::union_type(result_types))
            }
            _ => (!lib_member_over_table(&object_type, property_name, ctx))
                .then(|| {
                    global_object_member(&object_type, property_name, ctx)
                        .or_else(|| object_type.get_property_access_type(property_name))
                })
                .flatten()
                .or_else(|| lib_builtin_member_type(&object_type, property_name, ctx))
                .map(|property_type| {
                    InferredExpression::Known(widen_index_signature_read(
                        &object_type,
                        property_name,
                        crate::checks::function::bind_this_member(&object_type, property_type),
                        ctx,
                    ))
                })
                .unwrap_or_else(|| {
                    if no_lib_array_member(&object_type, ctx) {
                        InferredExpression::Known(no_lib_array_member_or_any(&object_type, property_name, ctx))
                    // A nominal reference that peels to the sentinel is a shape
                    // surge could not reconstruct (a cross-module `Set<string>`
                    // annotation whose lazy environment is gone), not a type
                    // without the member.
                    } else if object_type.peeled().is_unknown()
                        || crate::checks::expr::carries_leaked_type_parameter(&object_type, ctx)
                    {
                        InferredExpression::Unknown
                    } else {
                        InferredExpression::MissingProperty {
                            property_name: property_name.to_string(),
                            object_type: object_type.clone(),
                            span: *property_span,
                        }
                    }
                }),
        },
    );
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.property_access_checking += property_access_start.elapsed()
    });
    result
}

/// A guard on this exact element access (`if (xs[i])`) recorded a narrowed
/// read under the access's rendered key; see
/// `narrow_element_reference_guards_in_scope`.
pub(crate) fn narrowed_element_read(
    object: &ParsedExpression,
    index: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<Type> {
    let key = crate::checks::function::element_reference_key(object, index)?;
    symbols.get(&key).map(|symbol| symbol.ty.clone())
}

pub(crate) fn narrowed_element_read_named(
    object_name: &str,
    index: &ParsedExpression,
    symbols: &SymbolTable,
) -> Option<Type> {
    let key =
        crate::checks::function::element_reference_key_named(object_name, &[], index)?;
    symbols.get(&key).map(|symbol| symbol.ty.clone())
}

/// Under `noUncheckedIndexedAccess` a member that exists only through a string
/// index signature reads as `T | undefined`, like the element access it is.
fn widen_index_signature_read(
    receiver: &Type,
    property_name: &str,
    property_type: Type,
    ctx: &CheckerContext,
) -> Type {
    // A type variable is read through its apparent type, its constraint.
    let mut apparent = receiver.clone();
    for _ in 0..8 {
        match &apparent {
            Type::TypeParameter(parameter) if surge_ts_types::type_variable::deferred_type(parameter).is_none() => {
                match surge_ts_types::type_variable::active_constraint(parameter) {
                    Some(Some(constraint)) => apparent = constraint,
                    _ => break,
                }
            }
            _ => break,
        }
    }
    if ctx.options.no_unchecked_indexed_access
        && apparent.reads_unnarrowed_string_index(property_name)
    {
        unchecked_index_read(property_type, ctx)
    } else {
        property_type
    }
}

/// Under `noUncheckedIndexedAccess` an array or index-signature element read is
/// `T | undefined`; a tuple element at a literal index is exact and never
/// widened.
pub(crate) fn unchecked_index_read(element_type: Type, ctx: &CheckerContext) -> Type {
    // `any`/`unknown` absorb `undefined`; the degradation sentinel must stay
    // the sentinel rather than become a union the checks would read as typed.
    if !ctx.options.no_unchecked_indexed_access
        || matches!(
            element_type,
            Type::Any | Type::GenuineUnknown | Type::Unknown | Type::TypeParameter(_)
        )
    {
        element_type
    } else {
        union_type(vec![element_type, Type::Undefined])
    }
}

/// The symbol bound to a qualified `ns.member` value key, using the same lookup
/// chain a bare call resolves through.
pub(crate) fn qualified_namespace_member(
    qualified_name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<std::sync::Arc<crate::symbols::SymbolInfo>> {
    if let Some(symbol) = symbols.get_shared(qualified_name) {
        return Some(symbol);
    }
    if let Some(symbol) = ctx
        .module_value_fallback
        .as_ref()
        .and_then(|fallback| fallback.get_shared(qualified_name))
    {
        return Some(symbol);
    }
    let file_name = ctx.file_name.clone();
    ctx.module_local_values_for_file(&file_name)
        .and_then(|table| table.get_shared(qualified_name))
}

/// Inference-side twin of `try_qualified_namespace_call`: instantiates the
/// member's return type through its qualified `ns.member` signature. `None` when
/// no such binding exists.
fn infer_qualified_namespace_call(
    object_name: &str,
    property_name: &str,
    property_span: Option<TextSpan>,
    type_arguments: &[surge_ts_syntax::ParsedType],
    arguments: &[surge_ts_syntax::ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    let qualified_name = format!("{object_name}.{property_name}");
    let symbol = qualified_namespace_member(&qualified_name, symbols, ctx)?;
    // The qualified binding is authoritative once it exists: answering from the
    // namespace object's permissive `any` member instead would bake that into
    // whatever declaration is being resolved.
    let Type::Function(function_type) = &symbol.ty else {
        return Some(InferredExpression::Unknown);
    };
    let function_type = function_type.clone();
    // An overload group answers through `chooseOverload`, as the checked call
    // does; the signature recorded for it is only its first member's.
    if function_type.overloads().is_some()
        && type_arguments.is_empty()
        && let Some(selected) = crate::checks::call::select_overload_return_type_for_inferred_call(
            &function_type,
            arguments,
            symbols,
            ctx,
        )
    {
        return Some(InferredExpression::Known(selected));
    }
    let function_signature = symbol.function_signature.clone();
    Some(InferredExpression::Known(
        crate::checks::call::instantiate_function_return_type_for_call(
            &function_type,
            function_signature.as_deref(),
            type_arguments,
            property_span,
            arguments,
            symbols,
            ctx,
        ),
    ))
}

pub(crate) fn infer_property_call(
    object: &ParsedExpression,
    _object_span: &Option<TextSpan>,
    property_name: &str,
    property_span: &Option<TextSpan>,
    type_arguments: &[surge_ts_syntax::ParsedType],
    arguments: &[surge_ts_syntax::ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    record_property_lookup();
    let property_call_start = Instant::now();
    // Taken before the receiver is inferred: a `map` there is not the one the
    // context was given to.
    let mapped_context = take_mapped_argument_context();
    let object_type = match infer_expression(object, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        // tsc types a member the receiver lacks with its error type, and
        // everything read off that is the error type again.
        InferredExpression::MissingProperty { .. } => {
            return InferredExpression::Known(Type::ErrorType);
        }
        InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };
    let object_type = crate::checks::call::type_variable_call_receiver(object_type, property_name);

    // `filter` with a type-predicate callback yields the predicate's element,
    // as the checked path answers it: an argument (`new Set(xs.filter(isY))`)
    // is inferred, not checked.
    if property_name == "filter"
        && let Type::Array(element) = &object_type
        && let Some(filtered) = crate::checks::call::filtered_element_type(element, arguments, symbols, ctx)
    {
        return InferredExpression::Known(Type::Array(Box::new(filtered)));
    }

    if matches!(
        object_type,
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_)
    ) && let ParsedExpression::Identifier { name, .. } = object
        && let Some(inferred) = infer_qualified_namespace_call(
            name,
            property_name,
            *property_span,
            type_arguments,
            arguments,
            symbols,
            ctx,
        )
    {
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.property_access_checking += property_call_start.elapsed()
        });
        return inferred;
    }

    // `Promise.all` / `Promise.resolve` are typed by their own rules on the
    // checked path; the inferred path — a destructuring initializer takes it —
    // has to answer the same type, without reporting anything itself.
    if (matches!(property_name, "all" | "race")
        || (property_name == "resolve" && crate::checks::call::promise_nominal_enabled()))
        && type_arguments.is_empty()
        && !arguments.iter().any(|argument| argument.spread)
        && crate::checks::call::is_promise_all_receiver(&object_type)
    {
        let reported = ctx.diagnostics().len();
        let result = if property_name == "race" {
            crate::checks::call::check_promise_race_call(arguments, symbols, ctx)
        } else if property_name == "all" {
            (!arguments.is_empty())
                .then(|| crate::checks::call::check_promise_all_call(arguments, None, None, symbols, ctx))
                .flatten()
        } else if arguments.len() <= 1 {
            crate::checks::call::check_promise_resolve_call(arguments, None, symbols, ctx)
        } else {
            None
        };
        ctx.truncate_diagnostics(reported);
        if let Some(result) = result {
            return InferredExpression::Known(result);
        }
    }

    // `Promise<T>` is modelled as its awaited `T`, so a `.then`/`.catch`/`.finally`
    // chained on a promise-returning call lands on the value type, which declares
    // no such member. The checking path answers these from the chain; without the
    // same answer here the inference-only path degrades, and a generic call whose
    // argument is `() => sleep(10).then(() => 'data')` then binds no type
    // argument at all (vitest's `vi.fn`).
    if matches!(property_name, "then" | "catch" | "finally")
        && !matches!(
            object_type,
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_)
        )
        && !crate::checks::call::declares_own_property(&object_type, property_name)
        && !crate::checks::call::lib_promise_lacks(property_name, ctx)
    {
        let chained = if property_name == "then" {
            match arguments.first() {
                Some(callback) => match infer_expression(&callback.expression, symbols, ctx) {
                    InferredExpression::Known(Type::Function(function_type)) => {
                        crate::checks::call::promise_like_awaited_type(function_type.return_type())
                    }
                    InferredExpression::Known(ty) => {
                        crate::checks::call::promise_like_awaited_type(&ty)
                    }
                    _ => Type::Unknown,
                },
                None => Type::Unknown,
            }
        } else {
            object_type.clone()
        };
        // The same promise the checking path answers (`check_promise_then_call`):
        // an inferred callback that returned the bare value handed `vi.fn` a
        // `T` of `() => string` for `() => sleep(10).then(() => 'data')`.
        let chained = if property_name == "then" && crate::checks::call::promise_nominal_enabled() {
            let widened = crate::checks::expr::widen_type(&chained);
            crate::checks::call::promise_of(&widened, ctx)
        } else {
            chained
        };
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.property_access_checking += property_call_start.elapsed()
        });
        return InferredExpression::Known(chained);
    }

    if let Some(result) = crate::checks::call::bind_call_apply_result(
        &object_type,
        property_name,
        type_arguments,
        arguments,
        ctx,
    ) {
        record_program_timing(ctx.timings.as_ref(), |timings| {
            timings.property_access_checking += property_call_start.elapsed()
        });
        return InferredExpression::Known(result);
    }

    let object_type = crate::checks::call::inherited_array_receiver(&object_type, property_name).unwrap_or(object_type);
    let result = match &object_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::ErrorType => InferredExpression::Known(Type::ErrorType),
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
            InferredExpression::Unknown
        }
        Type::Array(element_type) if property_name == "find" => {
            InferredExpression::Known(surge_ts_types::union_type(vec![
                element_type.as_ref().clone(),
                Type::Undefined,
            ]))
        }
        // `map`'s `U` is its callback's return, read with the callback's
        // parameters typed by the element as the checked call reads it; the
        // member table's own `map` answers `any[]`, which every argument
        // (`new Set(xs.map(f))`), property and element position inferred.
        Type::Array(element_type) if property_name == "map" && !arguments.is_empty() => {
            let reported = ctx.diagnostics().len();
            let mapped = crate::checks::call::check_array_map_call(
                element_type,
                None,
                None,
                arguments,
                mapped_context.as_ref(),
                symbols,
                ctx,
            );
            ctx.truncate_diagnostics(reported);
            match mapped {
                Some(mapped) => InferredExpression::Known(mapped),
                None => InferredExpression::Unknown,
            }
        }
        // The overload the checked call picks, and its `U`.
        Type::Array(element_type)
            if matches!(property_name, "reduce" | "reduceRight")
                && matches!(arguments.len(), 1 | 2)
                && type_arguments.len() <= 1
                && !arguments.iter().any(|argument| argument.spread) =>
        {
            let reported = ctx.diagnostics().len();
            let reduced = crate::checks::call::check_array_reduce_call(element_type, type_arguments, arguments, symbols, ctx);
            ctx.truncate_diagnostics(reported);
            InferredExpression::Known(reduced)
        }
        Type::Array(element_type)
            if property_name == "flat"
                && crate::checks::call::array_flat_result(element_type, arguments).is_some()
                && !lib_lacks_builtin_member(&object_type, property_name, ctx) =>
        {
            InferredExpression::Known(
                crate::checks::call::array_flat_result(element_type, arguments).unwrap_or(Type::Unknown),
            )
        }
        Type::Array(element_type) if property_name == "flatMap" && !arguments.is_empty() => {
            let reported = ctx.diagnostics().len();
            let mapped =
                crate::checks::call::check_array_flat_map_call(element_type, None, None, arguments, symbols, ctx);
            ctx.truncate_diagnostics(reported);
            match mapped {
                Some(mapped) => InferredExpression::Known(mapped),
                None => InferredExpression::Unknown,
            }
        }
        Type::Union(union_type) => {
            let mut result_types = vec![];
            for ty in union_type.types() {
                if *ty == Type::Undefined {
                    // `a?.b.c()` keeps the chain's `undefined`; a plain call on
                    // a possibly-`undefined` receiver is an error the check
                    // pass reports, and tsc types the call from the defined
                    // part alone (`checkNonNullExpression`).
                    if object.continues_optional_chain() {
                        result_types.push(ty.clone());
                    }
                    continue;
                }
                if property_name == "find"
                    && let Type::Array(element_type) = ty
                {
                    result_types.push(surge_ts_types::union_type(vec![
                        element_type.as_ref().clone(),
                        Type::Undefined,
                    ]));
                    continue;
                }
                match ty
                    .get_property_access_type(property_name)
                    .map(|member| crate::checks::call::bind_member_polymorphic_this(member, ty, ctx))
                {
                    Some(Type::Function(function_type)) => {
                        result_types.push(function_type.return_type().clone());
                    }
                    Some(Type::Any) => result_types.push(Type::Any),
                    Some(_) | None => return InferredExpression::Unknown,
                }
            }
            if result_types.is_empty() {
                return InferredExpression::Unknown;
            }
            InferredExpression::Known(surge_ts_types::union_type(result_types))
        }
        _ => match (!lib_member_over_table(&object_type, property_name, ctx))
            .then(|| object_type.get_property_access_type(property_name))
            .flatten()
            // A built-in member surge's tables do not list (`s.matchAll(re)`)
            // is the configured lib's, as the property read takes it.
            .or_else(|| lib_builtin_member_type(&object_type, property_name, ctx))
            .map(|member| crate::checks::call::bind_member_polymorphic_this(member, &object_type, ctx))
        {
            // A generic namespace member call resolves through its qualified
            // `ns.member` binding, which carries the real signature the namespace
            // object does not — the inference-side twin of the routing in
            // `check_property_call_like`, gated the same way on the permissive
            // member shape so the lookup stays off the ordinary path.
            Some(member_type)
                if crate::checks::call::is_permissive_member_type(&member_type)
                    && matches!(object, ParsedExpression::Identifier { .. }) =>
            {
                let ParsedExpression::Identifier { name, .. } = object else {
                    unreachable!("guarded by the match arm")
                };
                match infer_qualified_namespace_call(
                    name,
                    property_name,
                    *property_span,
                    type_arguments,
                    arguments,
                    symbols,
                    ctx,
                ) {
                    Some(inferred) => inferred,
                    None => match member_type {
                        Type::Function(function_type) => InferredExpression::Known(
                            crate::checks::call::select_overload_return_type_for_inferred_call(
                                &function_type,
                                arguments,
                                symbols,
                                ctx,
                            )
                            .unwrap_or_else(|| function_type.return_type().clone()),
                        ),
                        _ => InferredExpression::Known(Type::Any),
                    },
                }
            }
            Some(Type::Any) => InferredExpression::Known(Type::Any),
            None if no_lib_array_member(&object_type, ctx) => {
                InferredExpression::Known(match no_lib_array_member_or_any(&object_type, property_name, ctx) {
                    Type::Function(function) if function.type_parameter_head().is_none() => function.return_type().clone(),
                    _ => Type::Any,
                })
            }
            // A member typed by a generic call signature answers its return type
            // only once the call's type arguments are bound, and a reference to a
            // callable interface has to be peeled before it looks callable at all.
            Some(Type::Function(function_type))
                if !crate::checks::call::written_call_signature_recovery_enabled() =>
            {
                InferredExpression::Known(function_type.return_type().clone())
            }
            Some(_) if !crate::checks::call::written_call_signature_recovery_enabled() => {
                InferredExpression::Unknown
            }
            Some(member_type) => {
                match crate::checks::call::callable_member_call_return_type(
                    &member_type,
                    type_arguments,
                    *property_span,
                    arguments,
                    symbols,
                    ctx,
                ) {
                    Some(return_type) => InferredExpression::Known(return_type),
                    None => InferredExpression::Unknown,
                }
            }
            None => InferredExpression::Unknown,
        },
    };
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.property_access_checking += property_call_start.elapsed()
    });
    result
}

/// Under `noLib` the array member surface comes from the configured replacement
/// lib (roblox-ts's `Array` adds `size`/`push`/`pop`/… that the standard JS array
/// surface lacks). surge collapses that interface to `Type::Array` for
/// assignability, discarding its member set, so a member the std surface does not
/// know is not a real typo here — stay permissive instead of over-reporting
/// TS2339. Without `noLib` the std array surface is authoritative and a miss is a
/// genuine error.
/// A member of a built-in receiver that surge's own member tables do not list
/// but the configured lib declares on the global interface behind it
/// (`copyWithin` on `Array<T>`, `anchor` on `String`). What the lib says
/// follows `target`/`lib`, which the tables cannot. A member whose type does
/// not resolve, or that is overloaded, still exists, so it reads `any` rather
/// than as missing.
/// Whether surge's own member tables offer `name` on this receiver although
/// the configured lib does not declare it (`xs.at(0)` under `target: es2015`).
/// Only a member tsc ties to a lib version is judged, and only against a lib
/// that declares the global interface at all.
pub(crate) fn lib_lacks_builtin_member(receiver: &Type, name: &str, ctx: &CheckerContext) -> bool {
    // Only the receivers surge answers from its own tables; one resolved from
    // the lib (`ObjectConstructor`) already lacks what the lib lacks.
    let interface_name = match receiver {
        Type::Array(_) | Type::Tuple(_) => "Array",
        Type::String | Type::StringLiteral(_) => "String",
        Type::Number | Type::NumberLiteral(_) => "Number",
        Type::BigInt => "BigInt",
        Type::Symbol => "Symbol",
        _ => return false,
    };
    if ctx.options.no_lib
        || crate::checks::expr::lib_feature_of_missing_member(receiver, name).is_none()
    {
        return false;
    }
    match ctx.lookup_type_declaration(interface_name) {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => {
            ctx.is_library_scoped_file(&info.file_name)
                && !info.body.members.is_empty()
                && !info.body.members.iter().any(|member| member.name == name)
        }
        _ => false,
    }
}

/// Whether the configured lib's member stands over surge's own table for
/// `name` on this receiver: one the lib does not declare, or an array's
/// `values()`, `keys()` or `entries()`, whose lib `ArrayIterator<T>` inherits
/// `[Symbol.iterator]` and the rest of `IteratorObject` where the table's
/// stand-in models only `next()` (and so is no `Iterable<T>`).
pub(crate) fn lib_member_over_table(receiver: &Type, name: &str, ctx: &CheckerContext) -> bool {
    lib_lacks_builtin_member(receiver, name, ctx) || lib_array_iterator_method(receiver, name, ctx).is_some()
}

/// The array an iterator method (`values`, `keys`, `entries`) is read off
/// when the configured lib declares the `ArrayIterator<T>` it returns.
fn lib_array_iterator_method(receiver: &Type, name: &str, ctx: &CheckerContext) -> Option<Type> {
    if !matches!(name, "values" | "keys" | "entries")
        || ctx.options.no_lib
        || !matches!(
            ctx.lookup_type_declaration("ArrayIterator"),
            Some(crate::symbols::TypeDeclarationInfo::Interface(info)) if ctx.is_library_scoped_file(&info.file_name)
        )
    {
        return None;
    }
    match receiver {
        Type::Array(_) | Type::Tuple(_) => Some(receiver.clone()),
        Type::Reference(_) => match receiver.peeled() {
            array @ (Type::Array(_) | Type::Tuple(_)) => Some(array),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn lib_builtin_member_type(
    receiver: &Type,
    name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if let Type::Reference(reference) = receiver
        && reference.id.as_ref() == "\u{0}ArrayIterator"
    {
        return lib_array_iterator(reference.arguments.first()?, ctx)?.get_property_access_type(name);
    }
    if let Type::Reference(_) = receiver
        && let Some(array) = lib_array_iterator_method(receiver, name, ctx)
    {
        return lib_builtin_member_type(&array, name, ctx);
    }
    let (interface_name, element) = match receiver {
        Type::Array(element) => ("Array", Some(element.as_ref().clone())),
        Type::Tuple(elements) => ("Array", Some(surge_ts_types::union_type(elements.to_vec()))),
        Type::String | Type::StringLiteral(_) => ("String", None),
        Type::Number | Type::NumberLiteral(_) => ("Number", None),
        Type::Boolean | Type::BooleanLiteral(_) => ("Boolean", None),
        Type::BigInt => ("BigInt", None),
        Type::Symbol => ("Symbol", None),
        Type::Function(_) => ("Function", None),
        // tsc's `getPropertyOfType`: an object type with call or construct
        // signatures reads the members it lacks from the global `Function`.
        _ if matches!(receiver.peeled(), Type::Object(object)
            if object.call_signature().is_some() || object.construct_signature().is_some()) =>
        {
            ("Function", None)
        }
        _ => return None,
    };
    lib_interface_member(interface_name, element, name, ctx)
}

/// checker.go `getPropertyOfType`: an object type without call or construct
/// signatures reads a member it does not declare from the global `Object`
/// type (`constructor: Function`), before any index signature answers.
fn global_object_member(receiver: &Type, name: &str, ctx: &mut CheckerContext) -> Option<Type> {
    surge_ts_types::object_prototype_member_type(name)?;
    let Type::Object(object) = receiver.peeled() else {
        return None;
    };
    if object.properties.contains_key(name)
        || object.synthetic_open_index
        || object.call_signature().is_some()
        || object.construct_signature().is_some()
    {
        return None;
    }
    lib_interface_member("Object", None, name, ctx)
}

pub(crate) fn lib_interface_member(
    interface_name: &str,
    element: Option<Type>,
    name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let (member_type, optional, scope) = match ctx.lookup_type_declaration(interface_name)? {
        crate::symbols::TypeDeclarationInfo::Interface(info) => {
            let mut declared = info.body.members.iter().filter(|member| member.name == name);
            let member = declared.next()?;
            // An overloaded member is not one signature: a non-generic
            // interface answers it from its resolved type, where the overloads
            // form one group; otherwise it exists, and that is all this says.
            if declared.next().is_some() {
                return Some(
                    element
                        .is_none()
                        .then(|| overloaded_lib_interface_member(interface_name, name, ctx))
                        .flatten()
                        .unwrap_or(Type::Any),
                );
            }
            (member.ty.clone(), member.optional, info.resolution_scope.clone())
        }
        crate::symbols::TypeDeclarationInfo::Alias(_) => return None,
    };
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    if let Some(element) = element {
        substitution.set("T".to_string(), element, false);
    }
    let diagnostics_before = ctx.diagnostics().len();
    let resolved = crate::infer::with_type_declaration_scope(&scope, ctx, |ctx| {
        crate::infer::types::resolve_parsed_type(member_type, ctx, &mut Vec::new(), &substitution)
    });
    ctx.truncate_diagnostics(diagnostics_before);
    let had_error = resolved.had_error();
    let ty = resolved.into_ty();
    Some(if had_error || ty.is_unknown() {
        Type::Any
    } else if optional {
        surge_ts_types::union_type(vec![ty, Type::Undefined])
    } else {
        ty
    })
}

/// An overloaded member of the lib's `Array<T>` with `T` bound to `element`:
/// each declared signature resolved, the first carrying the group, as the
/// interface's own resolution groups them. For the compiler API, which
/// shows the member where the checker's own table answers it `any`.
pub(crate) fn lib_array_overloaded_member(element: Type, name: &str, ctx: &mut CheckerContext) -> Option<Type> {
    let crate::symbols::TypeDeclarationInfo::Interface(info) = ctx.lookup_type_declaration("Array")? else {
        return None;
    };
    let declared: Vec<_> = info.body.members.iter().filter(|member| member.name == name).map(|member| member.ty.clone()).collect();
    if declared.len() < 2 {
        return None;
    }
    let scope = info.resolution_scope.clone();
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    substitution.set("T".to_string(), element, false);
    let diagnostics_before = ctx.diagnostics().len();
    let mut signatures = Vec::with_capacity(declared.len());
    let mut had_error = false;
    for member_type in declared {
        // The checker stands a constrained type parameter of the signature
        // in with the unknown sentinel; shown, it is itself, its constraint
        // instantiated for the element (`<S extends Row>`).
        let (member_type, head) = match &member_type {
            surge_ts_syntax::ParsedType::Function(function)
                if function.type_parameters.iter().any(|parameter| parameter.constraint.is_some()) =>
            {
                let mut rendered = Vec::with_capacity(function.type_parameters.len());
                for parameter in &function.type_parameters {
                    let mut text = parameter.name.clone();
                    for (keyword, written) in [(" extends ", &parameter.constraint), (" = ", &parameter.default_type)] {
                        let Some(written) = written else { continue };
                        let resolved = crate::infer::with_type_declaration_scope(&scope, ctx, |ctx| {
                            crate::infer::types::resolve_parsed_type(written.clone(), ctx, &mut Vec::new(), &substitution)
                        });
                        had_error |= resolved.had_error();
                        text.push_str(keyword);
                        text.push_str(&resolved.into_ty().name());
                    }
                    rendered.push(text);
                }
                let mut unconstrained = function.as_ref().clone();
                for parameter in &mut unconstrained.type_parameters {
                    parameter.constraint = None;
                    parameter.default_type = None;
                }
                (surge_ts_syntax::ParsedType::Function(std::sync::Arc::new(unconstrained)), Some(rendered.join(", ")))
            }
            _ => (member_type, None),
        };
        let resolved = crate::infer::with_type_declaration_scope(&scope, ctx, |ctx| {
            crate::infer::types::resolve_parsed_type(member_type, ctx, &mut Vec::new(), &substitution)
        });
        had_error |= resolved.had_error();
        match resolved.into_ty() {
            Type::Function(function) => signatures.push(match head {
                Some(head) => function.with_type_parameter_head(Some(head)),
                None => function,
            }),
            _ => had_error = true,
        }
    }
    ctx.truncate_diagnostics(diagnostics_before);
    if had_error {
        return None;
    }
    let first = signatures.first()?.clone();
    Some(Type::Function(first.with_overloads(signatures)))
}

fn overloaded_lib_interface_member(interface_name: &str, name: &str, ctx: &mut CheckerContext) -> Option<Type> {
    let diagnostics_before = ctx.diagnostics().len();
    let interface = crate::infer::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: interface_name.to_string(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    ctx.truncate_diagnostics(diagnostics_before);
    let member = interface.peeled().get_property_access_type(name)?;
    // An overload group's own head folds its members' parameters; what it
    // returns is what decides whether surge modelled it.
    let decided = match &member {
        Type::Function(function) => function.return_type(),
        other => other,
    };
    (!crate::checks::function::type_contains_degradation(decided)).then_some(member)
}

/// The lib's `ArrayIterator<T>`, which `Array.prototype.values()` and its
/// kin return: surge's own stand-in models only `next()`, while the lib's
/// inherits every `IteratorObject` member the configured lib declares
/// (`map`, `filter`, `toArray`, …). `None` when the lib declares none.
fn lib_array_iterator(yields: &Type, ctx: &mut CheckerContext) -> Option<Type> {
    if !matches!(
        ctx.lookup_type_declaration("ArrayIterator"),
        Some(crate::symbols::TypeDeclarationInfo::Interface(_))
    ) {
        return None;
    }
    const SLOT: &str = "\u{0}yields";
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    substitution.insert(SLOT.to_string(), yields.clone());
    let named = |name: &str, type_arguments| {
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments,
        }))
    };
    let reported = ctx.diagnostics().len();
    let iterator = crate::infer::map_parsed_type_with_substitution(
        named("ArrayIterator", vec![named(SLOT, Vec::new())]),
        ctx,
        &substitution,
    );
    ctx.truncate_diagnostics(reported);
    (!iterator.is_unknown()).then_some(iterator)
}

fn no_lib_array_member(object_type: &Type, ctx: &CheckerContext) -> bool {
    ctx.options.no_lib && crate::infer::types::no_lib_array_element(object_type).is_some()
}

/// The member a `noLib` program's own `Array` declares, which surge's array
/// model lacks; `any` when the program declares none either.
fn no_lib_array_member_or_any(array: &Type, property_name: &str, ctx: &mut CheckerContext) -> Type {
    let Some(element) = crate::infer::types::no_lib_array_element(array) else { return Type::Any };
    crate::infer::types::no_lib_array_member_type(&element, property_name, ctx).unwrap_or(Type::Any)
}

/// The tuple a union member is, through a reference or an intersection with
/// a tuple operand.
fn branded_tuple(ty: &Type) -> Option<Type> {
    match ty.peeled() {
        tuple @ Type::Tuple(_) => Some(tuple),
        Type::Object(object) if object.is_intersection => object
            .intersection_operands
            .as_deref()?
            .iter()
            .map(Type::peeled)
            .find(|operand| matches!(operand, Type::Tuple(_))),
        _ => None,
    }
}

/// Non-optional element access on an arbitrary object expression (`expr[index]`).
/// Mirrors tuple/array indexing without the `| undefined` that optional access
/// adds, so a destructured `const [, setX] = useState()` reads the exact element
/// type (the setter) rather than `setter | undefined`.
/// An element read off each member of a union receiver, joined.
fn infer_union_index_access(
    members: &[Type],
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let mut result_types = vec![];
    for ty in members {
        if matches!(ty, Type::Undefined | Type::Null) {
            result_types.push(Type::Undefined);
            continue;
        }
        let peeled;
        let ty = match ty {
            Type::Reference(reference) if reference.is_readonly_array() => {
                peeled = reference.resolve();
                &peeled
            }
            // A tuple reached through a reference or a brand (`LuaTuple<[A,
            // B]>`, a tuple intersected with an object) is indexed as the tuple.
            Type::Reference(_) | Type::Object(_) if let Some(tuple) = branded_tuple(ty) => {
                peeled = tuple;
                &peeled
            }
            other => other,
        };
        match ty {
            Type::Tuple(elements) => {
                let res =
                    infer_tuple_index_access(elements, index, index_span, symbols, ctx);
                if let InferredExpression::Known(ty) = res {
                    result_types.push(ty);
                } else {
                    return InferredExpression::Unknown;
                }
            }
            Type::Array(element_type) => {
                let element_type = element_type.as_ref();
                if element_type.is_unknown() && !element_type.is_type_variable() && *element_type != Type::GenuineUnknown {
                    return InferredExpression::Unknown;
                }

                let index_type = match infer_expression(index, symbols, ctx) {
                    InferredExpression::Known(ty) => ty,
                    _ => return InferredExpression::Unknown,
                };

                if !crate::checks::expr::numeric_index_key(&index_type) {
                    return InferredExpression::Unknown;
                }
                result_types.push(unchecked_index_read(element_type.clone(), ctx));
            }
            // Same numeric index signature `String` declares, per
            // member: `('all' | 'active')[0]` reads a `string` from
            // either side.
            Type::String | Type::StringLiteral(_) => {
                let index_type = match infer_expression(index, symbols, ctx) {
                    InferredExpression::Known(ty) => ty,
                    _ => return InferredExpression::Unknown,
                };
                if !crate::checks::expr::numeric_index_key(&index_type) {
                    return InferredExpression::Unknown;
                }
                result_types.push(unchecked_index_read(Type::String, ctx));
            }
            _ => return InferredExpression::Unknown,
        }
    }
    InferredExpression::Known(surge_ts_types::union_type(result_types))
}

thread_local! {
    static ASSIGNMENT_PATTERN_VALUE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Restores what [`enter_assignment_pattern_value`] replaced.
pub(crate) struct AssignmentPatternValue(bool);

impl Drop for AssignmentPatternValue {
    fn drop(&mut self) {
        ASSIGNMENT_PATTERN_VALUE.set(self.0);
    }
}

/// Marks whether the assignment being handled was lowered from an assignment
/// pattern, whose defaults run before the patterns they default
/// (`bindDestructuringAssignmentFlow`): a computed key there reads what the
/// default assigned. A binding pattern's default may not run
/// (`bindInitializer`), so it is not marked.
pub(crate) fn enter_assignment_pattern_value(from_pattern: bool) -> AssignmentPatternValue {
    AssignmentPatternValue(ASSIGNMENT_PATTERN_VALUE.replace(from_pattern))
}

pub(crate) fn infer_element_access(
    object: &ParsedExpression,
    object_span: &Option<TextSpan>,
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if let Some(narrowed) = narrowed_element_read(object, index, symbols) {
        return InferredExpression::Known(narrowed);
    }
    let source = symbols;
    // A pattern's lowered read writes its key before its source.
    let after_source = (ASSIGNMENT_PATTERN_VALUE.get()
        && matches!((object_span, index_span), (Some(object), Some(key)) if key.start < object.start))
    .then(|| crate::checks::expr::symbols_after_all_assignments(object, symbols, ctx))
    .flatten();
    let symbols = after_source.as_ref().unwrap_or(symbols);
    let object_type = match infer_expression(object, source, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        // tsc types a member the receiver lacks with its error type, and
        // everything read off that is the error type again.
        InferredExpression::MissingProperty { .. } => {
            return InferredExpression::Known(Type::ErrorType);
        }
        InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };

    match &object_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::ErrorType => InferredExpression::Known(Type::ErrorType),
        Type::Tuple(elements) => {
            infer_tuple_index_access(elements, index, index_span, symbols, ctx)
        }
        Type::OpenTuple(tuple) => infer_open_tuple_index_access(tuple, index, index_span, symbols, ctx),
        Type::Array(element_type) => {
            if element_type.is_unknown() && !element_type.is_type_variable() && **element_type != Type::GenuineUnknown {
                return InferredExpression::Unknown;
            }
            match infer_expression(index, symbols, ctx) {
                InferredExpression::Known(Type::NumberLiteral(_))
                | InferredExpression::Known(Type::Number) => {
                    InferredExpression::Known(unchecked_index_read((**element_type).clone(), ctx))
                }
                // `${number}` and numeric names apply to the number index too
                // (`isApplicableIndexType`): `{ [key]: [value] }` of an array.
                InferredExpression::Known(key)
                    if (matches!(key, Type::StringLiteral(_) | Type::Union(_))
                        || surge_ts_types::template_literal_parts(&key).is_some())
                        && crate::checks::expr::numeric_index_key(&key) =>
                {
                    InferredExpression::Known(unchecked_index_read((**element_type).clone(), ctx))
                }
                _ => InferredExpression::Unknown,
            }
        }
        Type::Object(_) | Type::Reference(_) => {
            infer_object_element_read(&object_type, index, symbols, ctx)
        }
        Type::TypeParameter(_) => deferred_element_read(&object_type, index, symbols, ctx),
        Type::Union(union_type) => {
            if let ParsedExpression::NumberLiteral(key) = index
                && let crate::checks::expr::UnionKeyRead::Found(member) =
                    crate::checks::expr::union_numeric_key_read(union_type.types(), key, ctx)
            {
                return InferredExpression::Known(member);
            }
            infer_union_index_access(union_type.types(), index, index_span, symbols, ctx)
        }
        _ => InferredExpression::Unknown,
    }
}

pub(crate) fn infer_optional_index_access(
    object: &ParsedExpression,
    _object_span: &Option<TextSpan>,
    index: &ParsedExpression,
    index_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let object_type = match infer_expression(object, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        // tsc types a member the receiver lacks with its error type, and
        // everything read off that is the error type again.
        InferredExpression::MissingProperty { .. } => {
            return InferredExpression::Known(Type::ErrorType);
        }
        InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };

    let base_type = surge_ts_types::remove_nullish(&object_type);

    match &base_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::ErrorType => InferredExpression::Known(Type::ErrorType),
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => {
            InferredExpression::Unknown
        }
        Type::Tuple(elements) => {
            let result = infer_tuple_index_access(elements, index, index_span, symbols, ctx);
            match result {
                InferredExpression::Known(ty) => {
                    InferredExpression::Known(union_type(vec![ty, Type::Undefined]))
                }
                _ => result,
            }
        }
        Type::Array(element_type) => {
            let element_type = element_type.as_ref();
            if element_type.is_unknown() && !element_type.is_type_variable() && *element_type != Type::GenuineUnknown {
                return InferredExpression::Unknown;
            }

            let index_type = match infer_expression(index, symbols, ctx) {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };

            if let Type::NumberLiteral(_) | Type::Number = index_type {
                InferredExpression::Known(union_type(vec![element_type.clone(), Type::Undefined]))
            } else {
                InferredExpression::Unknown
            }
        }
        _ => InferredExpression::Unknown,
    }
}

pub(crate) fn infer_optional_property_access(
    object: &ParsedExpression,
    _object_span: &Option<TextSpan>,
    property_name: &str,
    property_span: &Option<TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    record_property_lookup();
    let property_access_start = Instant::now();
    let object_type = match infer_expression(object, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        // tsc types a member the receiver lacks with its error type, and
        // everything read off that is the error type again.
        InferredExpression::MissingProperty { .. } => {
            return InferredExpression::Known(Type::ErrorType);
        }
        InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };

    // `a?.b` only widens to `| undefined` when `a` can actually be nullish. After
    // a guard proves it is not (`if (!a.result) throw;` then `a.result?.mean`),
    // tsc types the access exactly as `a.b`, so an unconditional widen turned
    // guarded arithmetic into TS2362/TS2363.
    let object_is_nullish = optional_chain_can_short_circuit(&object_type);
    let base_type = surge_ts_types::remove_nullish(&object_type);

    let result_type = match base_type {
        // A type variable of the body being checked reads its member off its
        // apparent type, as the non-optional access does.
        Type::TypeParameter(ref parameter) if base_type.is_type_variable() => {
            type_variable_member(parameter, &base_type, property_name, *property_span, 0)
        }
        Type::Unknown
        | Type::GenuineUnknown
        | Type::TypeParameter(_)
        | Type::Any
        | Type::ErrorType => InferredExpression::Known(base_type.clone()),
        Type::Union(ref union_type) => {
            let mut result_types = Vec::new();
            let mut saw_known = false;

            for ty in union_type.types() {
                if let Type::TypeParameter(parameter) = ty
                    && ty.is_type_variable()
                {
                    match type_variable_member(parameter, ty, property_name, *property_span, 0) {
                        InferredExpression::Known(member) => {
                            saw_known = true;
                            result_types.push(member);
                            continue;
                        }
                        InferredExpression::MissingProperty { .. } => {
                            return InferredExpression::MissingProperty {
                                property_name: property_name.to_string(),
                                object_type: base_type.clone(),
                                span: *property_span,
                            };
                        }
                        _ => return InferredExpression::Unknown,
                    }
                }
                if *ty == Type::Undefined || ty.is_unknown() {
                    continue;
                }

                saw_known = true;
                match ty.get_property_access_type(property_name) {
                    Some(property_type) => result_types.push(property_type),
                    None if no_lib_array_member(ty, ctx) => result_types.push(no_lib_array_member_or_any(ty, property_name, ctx)),
                    // Same rule the non-optional access applies: a receiver that
                    // peels to the sentinel, or one still carrying a leaked type
                    // parameter, is a shape surge could not reconstruct, not a
                    // type without the member. Without it `a?.b` reported a
                    // member that `a!.b` resolved.
                    None if ty.peeled().is_unknown()
                        || crate::checks::expr::carries_leaked_type_parameter(ty, ctx) =>
                    {
                        return InferredExpression::Unknown;
                    }
                    None => {
                        return InferredExpression::MissingProperty {
                            property_name: property_name.to_string(),
                            object_type: base_type.clone(),
                            span: *property_span,
                        };
                    }
                }
            }

            if !saw_known || result_types.is_empty() {
                InferredExpression::Unknown
            } else {
                InferredExpression::Known(surge_ts_types::union_type(result_types))
            }
        }
        _ => base_type
            .get_property_access_type(property_name)
            .map(InferredExpression::Known)
            .unwrap_or_else(|| {
                if no_lib_array_member(&base_type, ctx) {
                    InferredExpression::Known(no_lib_array_member_or_any(&base_type, property_name, ctx))
                } else if base_type.peeled().is_unknown()
                    || crate::checks::expr::carries_leaked_type_parameter(&base_type, ctx)
                {
                    InferredExpression::Unknown
                } else {
                    InferredExpression::MissingProperty {
                        property_name: property_name.to_string(),
                        object_type: base_type.clone(),
                        span: *property_span,
                    }
                }
            }),
    };

    let result = match result_type {
        // `unknown` and `any` absorb the chain's `undefined`, as every union
        // with them does.
        InferredExpression::Known(ty @ (Type::GenuineUnknown | Type::Any)) => {
            InferredExpression::Known(ty)
        }
        InferredExpression::Known(ty) if object_is_nullish => {
            InferredExpression::Known(union_type(vec![ty, Type::Undefined]))
        }
        other => other,
    };
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.property_access_checking += property_access_start.elapsed()
    });
    result
}

pub(crate) fn infer_new_expression(
    callee: &ParsedExpression,
    type_arguments: &[surge_ts_syntax::ParsedType],
    arguments: &[surge_ts_syntax::ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // `new Map<string, V>()` is the instance its written type arguments name,
    // as the checked `new` builds it, not the member table's `any` stand-in.
    // A bare `new Map()` takes `MapConstructor`'s `new (): Map<any, any>`.
    let bare_map = type_arguments.is_empty()
        && arguments.is_empty()
        && matches!(callee, ParsedExpression::Identifier { name, .. } if name == "Map");
    if (!type_arguments.is_empty() || bare_map)
        && let ParsedExpression::Identifier { name, .. } = callee
        && matches!(ctx.lookup_type_declaration(name), Some(crate::symbols::TypeDeclarationInfo::Interface(info))
            if crate::default_lib::is_physical_default_lib_file_name(&info.file_name)
                && info.body.type_parameters.len() >= type_arguments.len().max(usize::from(bare_map) * 2))
    {
        let named = surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            name: name.clone(),
            span: None,
            type_arguments: if bare_map {
                vec![surge_ts_syntax::ParsedType::Any; 2]
            } else {
                type_arguments.to_vec()
            },
            type_argument_spans: Vec::new(),
        }));
        let saved_symbols = std::mem::replace(
            &mut ctx.symbols,
            symbols.clone_with_reason(surge_ts_types::TypeCopyReason::CallResolution),
        );
        let instance = crate::infer::map_parsed_type(named, ctx);
        ctx.symbols = saved_symbols;
        if !instance.is_unknown() {
            return InferredExpression::Known(instance);
        }
    }
    // The stand-in instance of a lib constructor (`new Array` is `any[]`)
    // answers only for the default lib's own: a program declaring its own
    // `Array` under `noLib` (roblox-ts's) constructs what that declares.
    let builtin = match callee {
        ParsedExpression::Identifier { name, .. }
            if !matches!(ctx.lookup_type_declaration(name), Some(crate::symbols::TypeDeclarationInfo::Interface(info))
                if !crate::default_lib::is_physical_default_lib_file_name(&info.file_name)) =>
        {
            surge_ts_types::Type::builtin_constructor_result_type(name)
        }
        _ => None,
    };
    // Arguments a generic lib constructor infers from (`new Set(ids)`) are
    // read through its construct signatures first.
    if let Some(result_type) = builtin.as_ref()
        && (arguments.is_empty() || !type_arguments.is_empty())
    {
        return InferredExpression::Known(result_type.clone());
    }

    // `resolveNewExpression` reads the construct signatures of the callee's
    // apparent type, through a named reference (`DateConstructor`).
    let callee_type = match infer_expression(callee, symbols, ctx) {
        InferredExpression::Known(ty @ Type::Reference(_)) => match ty.peeled() {
            Type::Object(object)
                if object
                    .construct_signature()
                    .is_some_and(|signature| signature.type_parameter_head().is_none()) =>
            {
                InferredExpression::Known(Type::Object(object))
            }
            _ => InferredExpression::Known(ty),
        },
        other => other,
    };
    match callee_type {
        // `checkCallExpression`: `new` of a function that only has a call
        // signature (TS7009/TS2350) is `any`, whatever the function returns.
        InferredExpression::Known(Type::Function(_)) => InferredExpression::Known(Type::Any),
        InferredExpression::Known(Type::Object(object))
            if object.construct_signature().is_some() =>
        {
            let construct_signature = object.construct_signature().expect("construct signature present");
            // Written type arguments instantiate the signature that takes
            // that many and fits the arguments (`new Box<string>()`).
            if !type_arguments.is_empty() {
                let mut members = Vec::new();
                construct_signature.push_overload_members(&mut members);
                if let Some(member) = members.iter().find(|member| {
                    member.type_parameter_names().len() == type_arguments.len()
                        && crate::checks::call::overload_arity_fits(member, arguments.len())
                }) {
                    return InferredExpression::Known(crate::checks::call::instantiate_function_return_type_for_call(
                        member,
                        None,
                        type_arguments,
                        None,
                        arguments,
                        symbols,
                        ctx,
                    ));
                }
            }
            // An overloaded generic construct signature is resolved for the
            // arguments, as the checked `new` resolves it.
            if type_arguments.is_empty()
                && !arguments.is_empty()
                && construct_signature.overloads().is_some()
                && let Some(member) = crate::checks::call::select_overload_member_for_inferred_call(
                    construct_signature,
                    arguments,
                    symbols,
                    ctx,
                )
            {
                return InferredExpression::Known(member.return_type().clone());
            }
            match builtin {
                Some(result_type) => InferredExpression::Known(result_type),
                None => InferredExpression::Known(construct_signature.return_type().clone()),
            }
        }
        InferredExpression::Known(Type::Any) => InferredExpression::Known(Type::Any),
        _ if builtin.is_some() => InferredExpression::Known(builtin.expect("builtin result present")),
        InferredExpression::UnresolvedIdentifier { name, span } => {
            InferredExpression::UnresolvedIdentifier { name, span }
        }
        _ => InferredExpression::Unknown,
    }
}

/// Whether an optional chain over a receiver of this type can short-circuit, and
/// so contributes `undefined` to the access's type. The degradation sentinel and
/// `any` are left alone: their result is already the sentinel/`any`.
pub(crate) fn optional_chain_can_short_circuit(object_type: &Type) -> bool {
    match object_type {
        Type::Undefined | Type::Null | Type::Void | Type::GenuineUnknown => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| matches!(member, Type::Undefined | Type::Null | Type::Void)),
        _ => false,
    }
}

/// Whether `resolveMappedTypeMembers` gives a generic mapped type over keys
/// whose lower bound is `keys` a member readable as `name`: a property for a
/// literal key, an index signature for `string`/`number`. A key still generic
/// adds nothing. `None` when a key is unmodelled.
fn mapped_key_admits(keys: &Type, name: &str) -> Option<bool> {
    let members = match keys {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let mut admits = false;
    for key in &members {
        match key {
            Type::StringLiteral(literal) => admits |= literal == name,
            Type::NumberLiteral(_) => admits |= key.name() == name,
            Type::String | Type::Any => admits = true,
            Type::Number => admits |= surge_ts_types::is_numeric_key(name),
            Type::Never | Type::Symbol => {}
            _ if key.is_type_variable() => {}
            _ => return None,
        }
    }
    Some(admits)
}

/// A member of a type variable of the body being checked, read off its
/// apparent type (`getApparentType`): the constraint, or `{}` without one. A
/// constraint surge did not model whole answers only what it has.
fn type_variable_member(
    parameter: &surge_ts_types::TypeParameterType,
    variable: &Type,
    property_name: &str,
    property_span: Option<TextSpan>,
    depth: usize,
) -> InferredExpression {
    const OBJECT_MEMBERS: &[&str] = &[
        "constructor",
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ];
    if let Some(surge_ts_types::type_variable::DeferredType::Mapped { keys, object, modifiers, .. }) =
        surge_ts_types::type_variable::mapped_type(variable)
        && let Type::TypeParameter(keys) = keys
        && matches!(surge_ts_types::type_variable::deferred_type(&keys),
            Some(surge_ts_types::type_variable::DeferredType::Keyof(_)))
    {
        let apparent = surge_ts_types::type_variable::base_constraint_or_type(&object);
        if let Some(member) = apparent.get_property_access_type(property_name) {
            return InferredExpression::Known(if modifiers.optional > 0 && surge_ts_types::strict_null_checks() {
                surge_ts_types::union_type(vec![member, Type::Undefined])
            } else { member });
        }
    }
    let mapped_member = match surge_ts_types::type_variable::deferred_type(parameter) {
        Some(surge_ts_types::type_variable::DeferredType::Mapped { keys, object, modifiers, .. }) => {
            Some((keys, Some(object), None, modifiers))
        }
        Some(surge_ts_types::type_variable::DeferredType::MappedConstant { keys, template, modifiers }) => {
            Some((keys, None, Some(template), modifiers))
        }
        _ => None,
    };
    if let Some((keys, object, template, modifiers)) = mapped_member {
        let Some(lower_bound) = surge_ts_types::type_variable::lower_bound_of_key_type(&keys) else {
            return InferredExpression::Unknown;
        };
        match mapped_key_admits(&lower_bound, property_name) {
            None => return InferredExpression::Unknown,
            Some(true) => {
                let member = match (object, template) {
                    (_, Some(template)) => template,
                    (Some(object), None) => {
                        let key = Type::StringLiteral(property_name.to_string());
                        match surge_ts_types::type_variable::indexed_access_variable(&object, &key) {
                            Some(access) => access,
                            None => match surge_ts_types::type_variable::base_constraint_or_type(&object)
                                .get_property_access_type(property_name)
                            {
                                Some(member) => member,
                                None => return InferredExpression::Unknown,
                            },
                        }
                    }
                    (None, None) => return InferredExpression::Unknown,
                };
                return InferredExpression::Known(if modifiers.optional > 0 && surge_ts_types::strict_null_checks() {
                    surge_ts_types::union_type(vec![member, Type::Undefined])
                } else {
                    member
                });
            }
            Some(false) if !OBJECT_MEMBERS.contains(&property_name) => {
                return InferredExpression::MissingProperty {
                    property_name: property_name.to_string(),
                    object_type: variable.clone(),
                    span: property_span,
                };
            }
            Some(false) => {}
        }
    }
    let Some(constraint) = surge_ts_types::type_variable::active_constraint(parameter) else {
        return InferredExpression::Unknown;
    };
    let missing = || InferredExpression::MissingProperty {
        property_name: property_name.to_string(),
        object_type: variable.clone(),
        span: property_span,
    };
    // `getApparentType`: an unconstrained variable is `unknown`, which only
    // without `strictNullChecks` reads as `{}` and so has `Object`'s members.
    let Some(constraint) = constraint else {
        return if OBJECT_MEMBERS.contains(&property_name) && !surge_ts_types::strict_null_checks() {
            InferredExpression::Unknown
        } else {
            missing()
        };
    };
    // `getApparentType` follows a constraint that is itself a variable
    // (`U extends T, T extends A`).
    if let Type::TypeParameter(next) = &constraint
        && constraint.is_type_variable()
        && surge_ts_types::type_variable::deferred_type(next).is_none()
    {
        if depth >= 8 {
            return InferredExpression::Unknown;
        }
        return match type_variable_member(next, &constraint, property_name, property_span, depth + 1) {
            InferredExpression::MissingProperty { .. } => missing(),
            other => other,
        };
    }
    // A reference typed by a variable with a union constraint is read as that
    // union (tsgo's `getNarrowableTypeForReference`); a narrowing guard has
    // already left it `T & C'` (narrowing/reference.rs), so here it is whole.
    // Its nullish members are the receiver check's to report.
    if let Type::Union(union) = &constraint {
        let members: Vec<&Type> = union
            .types()
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void))
            .collect();
        if members.iter().any(|member| member.is_unmodelled()) {
            return InferredExpression::Unknown;
        }
        // Only a member surge closed off proves the property absent; what an
        // unannotated method of a class member returns is beside the point.
        let closed = |member: &Type| match member.peeled() {
            Type::Object(object) => {
                !object.synthetic_open_index && object.string_index_type.is_none() && !object.is_intersection
            }
            peeled => !peeled.is_unknown() && !matches!(peeled, Type::Any | Type::Union(_)),
        };
        let mut found = Vec::with_capacity(members.len());
        for member in &members {
            // A member that is itself a type variable (a deferred conditional)
            // reads through its own apparent type.
            if let Type::TypeParameter(member_parameter) = member
                && member.is_type_variable()
            {
                if depth >= 8 {
                    return InferredExpression::Unknown;
                }
                match type_variable_member(member_parameter, member, property_name, property_span, depth + 1) {
                    InferredExpression::Known(member_type) => {
                        found.push(member_type);
                        continue;
                    }
                    InferredExpression::MissingProperty { .. } => {
                        return InferredExpression::MissingProperty {
                            property_name: property_name.to_string(),
                            object_type: constraint.clone(),
                            span: property_span,
                        };
                    }
                    _ => return InferredExpression::Unknown,
                }
            }
            match member.get_property_access_type(property_name) {
                Some(member_type) => found.push(member_type),
                None if closed(member) && !OBJECT_MEMBERS.contains(&property_name) => {
                    return InferredExpression::MissingProperty {
                        property_name: property_name.to_string(),
                        object_type: constraint.clone(),
                        span: property_span,
                    };
                }
                None => return InferredExpression::Unknown,
            }
        }
        return InferredExpression::Known(surge_ts_types::union_type(found));
    }
    match constraint.get_property_access_type(property_name) {
        Some(member) => InferredExpression::Known(member),
        None if !constraint.is_unknown()
            && !OBJECT_MEMBERS.contains(&property_name)
            && !crate::checks::function::type_contains_degradation(&constraint) =>
        {
            missing()
        }
        None => InferredExpression::Unknown,
    }
}
