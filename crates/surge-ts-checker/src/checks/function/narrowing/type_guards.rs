use surge_ts_syntax::{ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator};
use surge_ts_types::{Type, TypeCopyReason, union_type, with_type_copy_reason};

use crate::context::CheckerContext;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};
use super::guards::*;
use super::{
    ReferenceGuard, narrow_predicate_reference_guards_in_scope, narrow_reference_in_scope,
    narrow_value_guards_in_scope, narrowed_reference_type, reference_equality_narrowings,
    reference_path,
};

/// Applies `typeof x === "tag"` / `typeof o.p === "tag"` narrowing in place to a
/// `ScopeStack` (the if-body and early-return paths). Returns whether the
/// condition was a typeof guard over a reference.
pub(super) fn narrow_typeof_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some((operand, tag, eq)) = parse_typeof_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(operand) else {
        return false;
    };
    let guard = ReferenceGuard::Typeof {
        tag,
        keep_matching: branch_is_true == eq,
    };
    narrow_reference_in_scope(&base, &path, guard, scopes);
    true
}

/// Applies `x instanceof Ctor` / `o.p instanceof Ctor` narrowing in place to a
/// `ScopeStack`. Returns whether the condition was an instanceof guard over a
/// reference.
pub(super) fn narrow_instanceof_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> bool {
    let Some((operand, ctor_name)) = parse_instanceof_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(operand) else {
        return false;
    };
    let constructor_value =
        super::guards::instanceof_constructor_value(condition, scopes.visible_symbols(), ctx);
    if constructs_only_any(constructor_value.as_ref()) {
        return true;
    }
    let instance = resolve_constructor_instance_type(ctor_name, constructor_value.as_ref(), ctx);
    let heritage = instance.as_ref().and_then(|instance| InstanceofHeritage::new(instance, ctx));
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::Instanceof {
            ctor_name,
            instance: instance.as_ref(),
            heritage,
            keep_matching: branch_is_true,
        },
        scopes,
    );
    true
}

/// The instance type a constructor name denotes, for `x instanceof Ctor`. `None`
/// when the name does not resolve to a type — the guard then falls back to
/// nominal union filtering.
pub(super) fn resolve_constructor_instance_type(
    ctor_name: &str,
    constructor_value: Option<&Type>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    resolve_named_constructor_instance_type(ctor_name, ctx)
        .or_else(|| constructor_value.and_then(instance_type_of_constructor_value))
}

/// tsc's `getInstanceType`, for a right operand that is a value rather than a
/// class name: the type of its `prototype` member unless that is `any`, else
/// the union of what its construct signatures return.
fn instance_type_of_constructor_value(constructor: &Type) -> Option<Type> {
    let Type::Object(object) = constructor.peeled() else {
        return None;
    };
    let usable = |ty: &Type| !matches!(ty, Type::Any) && !ty.is_unknown();
    if let Some(prototype) = object.properties.get("prototype")
        && usable(&prototype.ty)
    {
        return Some(prototype.ty.clone());
    }
    let mut signatures = Vec::new();
    object.construct_signature()?.push_overload_members(&mut signatures);
    let returns: Vec<Type> = signatures
        .iter()
        .map(|signature| signature.return_type().clone())
        .collect();
    if !returns.iter().all(usable) {
        return None;
    }
    Some(union_type(returns))
}

fn resolve_named_constructor_instance_type(
    ctor_name: &str,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // The reference is synthesized from the guard, not written in the source, so
    // it has to carry the arity the declaration expects — a bare generic name
    // would report TS2314 for arguments the guard never writes. `x instanceof
    // GSub` narrows to `GSub<any>` in tsc, which is what filling `any` gives.
    // Through the full resolution surface, not the file's own table: a lib
    // constructor (`Map`, `WeakSet`) is declared in the default lib, so reading
    // arity from the local table alone gave it zero, and `Map` then resolved
    // with no arguments and failed — leaving `x instanceof Map` narrowing
    // nothing at all in project mode.
    let arity = match ctx
        .type_declarations
        .get(ctor_name)
        .or_else(|| ctx.lookup_type_declaration(ctor_name))
    {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => {
            info.body.type_parameters.len()
        }
        Some(crate::symbols::TypeDeclarationInfo::Alias(info)) => info.body.type_parameters.len(),
        None => 0,
    };
    // Nothing this resolution reports belongs to the source, so drop whatever it
    // raised.
    let diagnostics_before = ctx.diagnostics().len();
    let resolved = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        crate::infer::types::resolve_parsed_type(
            surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(
                surge_ts_syntax::ParsedNamedType {
                    type_argument_spans: Vec::new(),
                    name: ctor_name.to_string(),
                    span: None,
                    type_arguments: vec![surge_ts_syntax::ParsedType::Any; arity],
                },
            )),
            ctx,
            &mut Vec::new(),
            &crate::infer::TypeParameterSubstitution::new(),
        )
    });
    ctx.truncate_diagnostics(diagnostics_before);
    if resolved.had_error() {
        return None;
    }
    let instance = resolved.into_ty();
    (!instance.is_unknown()).then_some(instance)
}

/// Applies `Array.isArray(x)` / `Array.isArray(o.p)` narrowing in place to a
/// `ScopeStack`. Returns whether the condition was such a guard over a
/// reference.
pub(super) fn narrow_array_isarray_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some(operand) = parse_array_isarray_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(operand) else {
        return false;
    };
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::Arrayness {
            keep_arrays: branch_is_true,
        },
        scopes,
    );
    true
}

/// Applies `"prop" in x` narrowing in place to a `ScopeStack` (the if-body and
/// early-return paths). Returns whether the condition was such a test over a
/// bare identifier.
pub(super) fn narrow_property_presence_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some((object, property)) = parse_in_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(object) else {
        return false;
    };
    // Shadows in the current frame, not the owning frame — see the note in
    // `narrow_discriminant_in_scope`.
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::PropertyPresence {
            property,
            keep_present: branch_is_true,
        },
        scopes,
    );
    if surge_ts_types::exact_optional_property_types() {
        let mut member = path;
        member.push(property.to_string());
        narrow_reference_in_scope(
            &base,
            &member,
            ReferenceGuard::ExactOptionalPresence {
                keep_present: branch_is_true,
            },
            scopes,
        );
    }
    true
}

/// `o.hasOwnProperty("p")` narrows the read of `o.p` as `"p" in o` does under
/// `exactOptionalPropertyTypes` (`narrowTypeByCallExpression`). It only adds
/// to what the other guards make of the call.
pub(super) fn narrow_has_own_property_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) {
    if !surge_ts_types::exact_optional_property_types() {
        return;
    }
    let ParsedExpression::PropertyCall {
        object,
        property_name,
        arguments,
        ..
    } = condition
    else {
        return;
    };
    let [argument] = arguments.as_slice() else {
        return;
    };
    let ParsedExpression::StringLiteral(property) = &argument.expression else {
        return;
    };
    if property_name != "hasOwnProperty" || argument.spread {
        return;
    }
    let Some((base, mut member)) = reference_path(object) else {
        return;
    };
    member.push(property.clone());
    narrow_reference_in_scope(
        &base,
        &member,
        ReferenceGuard::ExactOptionalPresence {
            keep_present: branch_is_true,
        },
        scopes,
    );
}

/// Applies `x === null` / `x.p === undefined` narrowing in place to a
/// `ScopeStack`, for a bare identifier or one property of one. Returns whether
/// the condition was such a test (handled either way, so the discriminant parse
/// downstream is skipped — `null` is not a discriminant literal).
pub(super) fn narrow_nullish_equality_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some((subject, eq, test)) = parse_nullish_equality_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(subject) else {
        return false;
    };
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::Nullish {
            keep_matching: branch_is_true == eq,
            test,
        },
        scopes,
    );
    true
}

/// Applies `x === "lit"` / `x !== 3` narrowing in place to a `ScopeStack`.
/// Composite conditions reach the same leaf through
/// [`narrow_single_guard_for_identifier`]; this is the bare form, which never
/// enters that path. Returns whether the subject narrowed.
pub(super) fn narrow_literal_equality_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some((name, literal, eq)) =
        parse_identifier_literal_equality(condition, scopes.visible_symbols())
    else {
        return false;
    };
    let Some(symbol) = scopes.resolve(name) else {
        return false;
    };
    let declared = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();
    let keep_matching = branch_is_true == eq;
    let narrowed = match narrow_by_literal_equality(&declared, &literal, keep_matching) {
        Some(narrowed) => narrowed,
        // The branch that excludes the one literal a variable is already
        // narrowed to is dead: tsc's flow type there is `never`. Leaving it at
        // the literal made the following `else if` compare two disjoint
        // literals and report a false TS2367 (`let m: "x" | "y" = "x"` reaches
        // the `else` narrowed to `"x"` by its initializer). Only an identifier
        // whose *whole* type is that literal is narrowed this way — a
        // discriminant property keeps the shared helper's behaviour, where the
        // object union carries the narrowing instead.
        None if !keep_matching && declared == literal => Type::Never,
        None => return false,
    };
    let _ = scopes.insert_current_narrowed(
        name.to_string(),
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        declared,
    );
    true
}

/// Applies `x === y` narrowing against a compared value no literal spells
/// ([`reference_equality_narrowings`]) in place to a `ScopeStack`. Both
/// operands' types are read before either narrows. Returns whether anything
/// narrowed.
pub(super) fn narrow_reference_equality_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let narrowings = reference_equality_narrowings(
        condition,
        branch_is_true,
        &|name| scopes.resolve(name).cloned(),
        &|name| scopes.resolve(name).map(|symbol| symbol.ty.clone()),
    );
    let narrowed = !narrowings.is_empty();
    for (name, symbol, declared) in narrowings {
        let _ = scopes.insert_current_narrowed(name, symbol, declared);
    }
    narrowed
}

/// [`narrow_reference_equality_in_scope`] over a plain symbol table.
pub(super) fn narrow_reference_equality_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let narrowings = reference_equality_narrowings(
        condition,
        branch_is_true,
        &|name| symbols.get(name).cloned(),
        &|name| symbols.get(name).map(|symbol| symbol.ty.clone()),
    );
    if narrowings.is_empty() {
        return None;
    }
    let mut narrowed = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    for (name, symbol, declared) in narrowings {
        narrowed.insert_narrowed(name, symbol, declared);
    }
    Some(narrowed)
}

/// Applies `ArrayBuffer.isView(x)` / `ArrayBuffer.isView(o.p)` narrowing in place
/// to a `ScopeStack`. Returns whether the condition was such a guard over a
/// reference.
pub(super) fn narrow_arraybuffer_isview_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) -> bool {
    let Some(operand) = parse_arraybuffer_isview_condition(condition) else {
        return false;
    };
    let Some((base, path)) = reference_path(operand) else {
        return false;
    };
    narrow_reference_in_scope(
        &base,
        &path,
        ReferenceGuard::ArrayBufferView {
            keep_views: branch_is_true,
        },
        scopes,
    );
    true
}

/// `x instanceof Ctor` / `o.p instanceof Ctor` on a plain symbol table. Split
/// from the syntactic guards in [`narrow_condition_symbol_table`] for the same
/// reason the predicates are: deciding whether a union member *derives from*
/// the constructor needs the constructor's instance type, and resolving that
/// needs the checker context. The name-only test those guards run leaves
/// `TRPCClientError | Envelope` untouched under `instanceof Error`, so the
/// false branch of `props.result instanceof Error || …` still read the error
/// arm.
pub(super) fn narrow_instanceof_heritage_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    let (operand, ctor_name) = parse_instanceof_condition(condition)?;
    let (base, path) = reference_path(operand)?;
    let symbol = symbols.get(&base)?;
    let declared = symbol.ty.clone();
    let kind = symbol.kind;
    let function_signature = symbol.function_signature.clone();

    let constructor_value = super::guards::instanceof_constructor_value(condition, symbols, ctx);
    if super::guards::constructs_only_any(constructor_value.as_ref()) {
        return None;
    }
    let instance = resolve_constructor_instance_type(ctor_name, constructor_value.as_ref(), ctx)?;
    let heritage = InstanceofHeritage::new(&instance, ctx);
    let narrowed = with_type_copy_reason(TypeCopyReason::ScopeOrContext, || {
        if path.is_empty() {
            // A non-union subject narrows down its subtype edge instead, the
            // same fallback the scope-based single-guard path takes: without it
            // only an `if` narrowed `valueA instanceof Map`, and the same test
            // written as the condition of a `?:` left the subject alone.
            narrow_union_by_instanceof(&declared, ctor_name, Some(&instance), heritage, branch_is_true)
                .or_else(|| {
                    narrow_to_instanceof_subclass(&declared, Some(&instance), branch_is_true)
                })
        } else {
            narrowed_reference_type(
                &declared,
                &path,
                ReferenceGuard::Instanceof {
                    ctor_name,
                    instance: Some(&instance),
                    heritage,
                    keep_matching: branch_is_true,
                },
            )
        }
    })?;
    if narrowed == declared {
        return None;
    }

    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        base,
        SymbolInfo {
            ty: narrowed,
            kind,
            function_signature,
        },
        declared,
    );
    Some(narrowed_symbols)
}

/// Narrows `ty` for `var_name` under a single (non-composite) guard condition.
/// `x instanceof Sub` on a subject that is not a union narrows it *down* to the
/// constructor's instance type when that type is a subtype of the subject's
/// (`declare const s: Base; if (s instanceof Sub) s.onlyOnSub()`). Union subjects
/// are handled by member filtering in `narrow_union_by_instanceof`; this is the
/// single-type case, which filtering cannot express.
///
/// Only the matching branch narrows: the `else` branch of `s instanceof Sub`
/// keeps `Base`, since every non-`Sub` `Base` still inhabits it.
pub(super) fn narrow_to_instanceof_subclass(
    ty: &Type,
    instance: Option<&Type>,
    keep_matching: bool,
) -> Option<Type> {
    if !keep_matching || matches!(ty.peeled(), Type::Union(_)) {
        return None;
    }
    // `unknown` and `any` narrow to the candidate itself (tsc's
    // `getNarrowedType`), except that `any` stays `any` under `instanceof
    // Object` and `instanceof Function` (`narrowTypeByInstanceof`).
    if matches!(ty, Type::GenuineUnknown)
        || matches!(ty, Type::Any)
            && !instance.is_some_and(|instance| is_global_object_or_function(instance))
    {
        return instance.filter(|instance| !instance.is_unknown()).cloned();
    }
    // A subject that is unresolved says nothing to narrow.
    if ty.is_unknown() || matches!(ty, Type::Any) {
        return None;
    }
    let instance = instance?;
    if instance.is_unknown() || instance == ty {
        return None;
    }
    // `getNarrowedTypeWorker` keeps a subject derived from the candidate
    // (`isTypeDerivedFrom`): an instantiation of the candidate's own generic
    // declaration (`B<number>` under `instanceof B`, whose instance is `B<any>`).
    if let (Type::Reference(subject), Type::Reference(candidate)) = (ty, instance)
        && subject.id == candidate.id
    {
        return None;
    }
    if surge_ts_types::is_assignable_to(instance, ty) {
        return Some(instance.clone());
    }
    // `getNarrowedType` intersects what does not relate (`X & Y`); a weak
    // subject a class instance shares no member with is the case surge used
    // to relate before it checked weak types, and its members are the union
    // of both sides'.
    if surge_ts_types::is_weak_type(ty)
        && let (Type::Object(subject), Type::Object(candidate)) = (ty.peeled(), instance.peeled())
        && !candidate.properties.values().any(|property| property.ty.is_unknown())
    {
        let mut properties = (*candidate.properties).clone();
        for (name, property) in subject.properties.iter() {
            if !properties.contains_key(name.as_ref()) {
                properties.insert(name.clone(), property.clone());
            }
        }
        return Some(Type::Object(crate::metrics::alloc_object_type(
            properties,
            candidate.string_index_type.as_deref().cloned(),
        )));
    }
    // `getNarrowedTypeWorker`'s last resort for an object the candidate does
    // not relate to either way, as a predicate guard narrows one.
    if !surge_ts_types::is_assignable_to(ty, instance)
        && matches!(ty.peeled(), Type::Object(_))
        && matches!(instance.peeled(), Type::Object(_))
        && !instance.is_unmodelled()
    {
        return Some(crate::infer::types::merge_intersection_members(vec![ty.clone(), instance.clone()]));
    }
    None
}

/// Whether `instance` is the global `Object` or `Function` interface — the
/// instance types `instanceof` does not narrow `any` to.
fn is_global_object_or_function(instance: &Type) -> bool {
    if surge_ts_types::is_global_function_interface(instance) {
        return true;
    }
    let Type::Reference(reference) = instance else {
        return false;
    };
    reference.id.split('\u{0}').next_back() == Some("Object")
        && matches!(instance.peeled(), Type::Object(object)
            if ["hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable"]
                .iter()
                .all(|member| object.properties.get(*member).is_some()))
}

/// Narrows `symbols` by a bare `x === "lit"` / `x !== 3` test.
pub(super) fn narrow_literal_equality_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let (name, literal, eq) = parse_identifier_literal_equality(condition, symbols)?;
    let symbol = symbols.get(name)?;
    let narrowed = narrow_by_literal_equality(&symbol.ty, &literal, branch_is_true == eq)?;
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        name.to_string(),
        SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        },
        symbol.ty.clone(),
    );
    Some(narrowed_symbols)
}

/// Collects the identifiers tested by equality guards — a discriminant test
/// (`x.kind === "a"` → `x`) or a nullish test (`x === null`) — within a
/// (possibly `||`/`&&`/`!`-composed) condition. Kept out of
/// [`collect_guard_operand_identifiers`], whose results also drive the
/// genuine-`unknown` downgrade — testing a *property* proves nothing about the
/// whole value there.
pub(super) fn collect_equality_guard_subjects(
    condition: &ParsedExpression,
    scopes: &ScopeStack,
    names: &mut Vec<String>,
) {
    match condition {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect_equality_guard_subjects(operand, scopes, names),
        ParsedExpression::Logical {
            left,
            operator: ParsedLogicalOperator::Or | ParsedLogicalOperator::And,
            right,
            ..
        } => {
            collect_equality_guard_subjects(left, scopes, names);
            collect_equality_guard_subjects(right, scopes, names);
        }
        _ => {
            // The compared value may be a `const` (`x?.version === CACHE_VERSION`);
            // it resolves through the visible table like the single-guard path.
            let subject = match parse_discriminant_condition_with(condition, &|expression| {
                const_member_literal_value(expression, scopes.visible_symbols())
            }) {
                Some((ParsedExpression::Identifier { name, .. }, _, _, _)) => Some(name.as_str()),
                _ => match parse_nullish_equality_condition(condition) {
                    Some((ParsedExpression::Identifier { name, .. }, _, _)) => Some(name.as_str()),
                    _ => parse_identifier_literal_equality(condition, scopes.visible_symbols())
                        .map(|(name, _, _)| name),
                },
            };
            if let Some(name) = subject {
                if !names.iter().any(|existing| existing == name) {
                    names.push(name.to_string());
                }
                return;
            }
            // `y === z`: both operands narrow, each by the other's type.
            let Some(test) = parse_equality_test(condition) else {
                return;
            };
            for (subject, value) in test.operand_pairs() {
                if let ParsedExpression::Identifier { name, .. } = subject
                    && reference_path(value).is_some()
                    && !names.iter().any(|existing| existing == name)
                {
                    names.push(name.clone());
                }
            }
        }
    }
}

/// Like [`narrow_discriminant_symbol_table`] but applies the narrowing in place
/// to a `ScopeStack`, for narrowing a discriminated union inside an `if` branch
/// (or after an early-returning `if`).
pub(crate) fn narrow_discriminant_in_scope(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    narrow_value_guards_in_scope(condition, scopes, branch_is_true, ctx);
    // A predicate over a *property* (`ts.isStringLiteral(node.moduleSpecifier)`)
    // narrows that path, which the value-guard dispatch above does not reach —
    // and it has to run after it, since the property only exists once the base
    // itself is narrowed (`isImportDeclaration(node) && isStringLiteral(node.m)`).
    narrow_predicate_reference_guards_in_scope(condition, scopes, branch_is_true, ctx);
}
