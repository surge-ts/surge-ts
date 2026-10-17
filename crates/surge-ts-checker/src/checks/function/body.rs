//! Function body and statement-level checking (control flow, returns, assignments).

use super::*;

use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedFunctionBodyStatement, ParsedVariableDeclaration, ParsedVariableKind};
use surge_ts_types::{Type, TypeCopyReason, with_type_copy_reason};

use crate::context::CheckerContext;
use crate::context::convert_span;
use crate::flow::{FunctionFlowState, collect_future_block_scoped_declarations};
use crate::program::{
    record_flow_function_count, record_flow_function_skipped_count, record_flow_statement_count,
    record_function_body_check, record_program_timing,
};
use crate::symbols::{ScopeStack, SymbolInfo, SymbolTable};

pub(crate) fn should_check_missing_return(return_type: &Type) -> bool {
    // Peel lazy named references so the exemption sees what the annotation
    // resolves to: `Promise<T>` is modeled as its awaited `T` (implicit await),
    // so an async `(): Promise<void>` body with no `return` is exempt exactly
    // like `(): void`, and a named alias of `void`/`undefined`/`any` is exempt
    // like the keyword itself (tsc treats aliases as transparent here).
    let peeled;
    let return_type = if matches!(return_type, Type::Reference(_)) {
        peeled = return_type.peeled();
        &peeled
    } else {
        return_type
    };
    // tsc's early exit (`checkAllCodePathsInNonVoidFunctionReturnOrThrow`,
    // checker.go:3759): the unwrapped return type *maybe* contains `void` — so a
    // `number | void` union is exempt too — or is exactly `any`/`undefined`. A
    // union merely containing `undefined` is not exempt here; it is the TS2366
    // branch that then finds `undefined` assignable and stays quiet.
    if matches!(return_type, Type::Any | Type::ErrorType | Type::Undefined)
        || return_type_maybe_void(return_type)
    {
        return false;
    }
    // `GenuineUnknown` is a written `unknown` annotation, which tsc does report
    // on; bare `Unknown` is surge's degradation sentinel and must stay silent.
    if matches!(return_type, Type::GenuineUnknown) {
        return true;
    }
    // An object type or a type variable of the body is never `void`-like,
    // whatever its members resolved to.
    if matches!(return_type, Type::Object(_)) || return_type.is_type_variable() {
        return true;
    }
    !matches!(return_type, Type::Unknown | Type::TypeParameter(_))
        && !type_contains_unknown(return_type)
}

/// tsc's `maybeTypeOfKind(t, TypeFlagsVoid)`: `void` itself, or a union with a
/// `void` constituent.
fn return_type_maybe_void(return_type: &Type) -> bool {
    match return_type {
        Type::Void => true,
        Type::Union(union) => union.types().iter().any(return_type_maybe_void),
        Type::Reference(_) => return_type_maybe_void(&return_type.peeled()),
        _ => false,
    }
}

/// tsc's `isTypeAssignableTo(undefinedType, t)` gate on the TS2366 branch
/// (checker.go:3784). `void`/`any` cases have already left through the early
/// exit, so what remains is a union that carries `undefined` (or `unknown`).
fn return_type_admits_undefined(return_type: &Type) -> bool {
    match return_type {
        Type::Undefined | Type::Void | Type::Any | Type::Unknown | Type::GenuineUnknown => true,
        Type::Union(union) => union.types().iter().any(return_type_admits_undefined),
        Type::Reference(_) => return_type_admits_undefined(&return_type.peeled()),
        _ => false,
    }
}

pub(crate) fn type_contains_unknown(ty: &Type) -> bool {
    crate::checks::structural_walk::query(|| contains_unknown(ty, false))
}

/// Like [`type_contains_unknown`], but a written `unknown` is a real type.
pub(crate) fn type_contains_degradation(ty: &Type) -> bool {
    crate::checks::structural_walk::query(|| contains_unknown(ty, true))
}

fn contains_unknown(ty: &Type, sentinel_only: bool) -> bool {
    match crate::checks::structural_walk::visit(ty) {
        crate::checks::structural_walk::Visit::Walk => {}
        crate::checks::structural_walk::Visit::Seen => return false,
        crate::checks::structural_walk::Visit::Exhausted => return true,
    }
    thread_local! {
        // References resolved while walking the current type, to break the cyclic
        // structural graphs lazy nominal references form (interface A whose member
        // resolves to B whose member resolves back to A). Re-entering a reference
        // already on this path means the cycle introduces no *new* `unknown`.
        static VISITING_REFERENCES: std::cell::RefCell<Vec<(std::sync::Arc<str>, std::sync::Arc<[Type]>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    match ty {
        Type::Unknown => true,
        Type::TypeParameter(parameter) => !parameter.is_active_variable(),
        Type::GenuineUnknown => !sentinel_only,
        Type::Array(element) => contains_unknown(element, sentinel_only),
        Type::Tuple(elements) => elements
            .iter()
            .any(|element| contains_unknown(element, sentinel_only)),
        // A generic member signature has its own type parameters erased to
        // the sentinel when it is resolved (only their rendering is kept), and
        // a self-reference instantiated with one (`map<U>(…): Box<U>`) resolves
        // to the sentinel too. Reading those as gaps made every interface with
        // a generic method "contain unknown", which silenced each mismatch
        // against it — `return 1` from a function declared to return
        // `Box<string>`. The sentinel is permissive in a relation, so it is
        // never what makes one fail.
        Type::Function(function) if function.type_parameter_head().is_some() => false,
        Type::Function(function) => {
            !crate::checks::call::is_generic_signature(function)
                && crate::checks::assign::with_signature_type_parameters(function, || {
                    function
                        .parameters()
                        .iter()
                        .any(|parameter| contains_unknown(parameter, sentinel_only))
                        || contains_unknown(function.return_type(), sentinel_only)
                })
        }
        Type::Object(object) => {
            object
                .properties
                .values()
                .any(|property| contains_unknown(&property.ty, sentinel_only))
                || object
                    .string_index_type
                    .as_deref()
                    .is_some_and(|index| contains_unknown(index, sentinel_only))
        }
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| contains_unknown(member, sentinel_only)),
        Type::Reference(reference) => {
            let on_path = VISITING_REFERENCES.with(|visiting| {
                visiting
                    .borrow()
                    .iter()
                    .any(|(id, arguments)| *id == reference.id && *arguments == reference.arguments)
                    // tsc's `isDeeplyNestedType`: a declaration on the path three
                    // times is an expanding recursion (`Deep<T>` reaching
                    // `Deep<T[]>`) whose arguments never repeat.
                    || visiting.borrow().iter().filter(|(id, _)| *id == reference.id).count() >= 3
            });
            if on_path {
                return false;
            }
            VISITING_REFERENCES.with(|visiting| {
                visiting
                    .borrow_mut()
                    .push((reference.id.clone(), reference.arguments.clone()));
            });
            // The arguments are part of what the reference *is*, not just of
            // what it resolves to: a partially-substituted instantiation
            // (`NextComponentType<…, AppPropsType<any, P>>` with `P` still
            // free) can resolve to a structure that no longer mentions `P`,
            // and comparing against it proves nothing about the real type.
            let result = reference
                .arguments
                .iter()
                .any(|argument| contains_unknown(argument, sentinel_only))
                || contains_unknown(&reference.resolve(), sentinel_only);
            VISITING_REFERENCES.with(|visiting| {
                visiting.borrow_mut().pop();
            });
            result
        }
        _ => false,
    }
}

pub(crate) fn emit_missing_return_diagnostic(
    body_flow: crate::flow::FunctionBodyFlow,
    return_type: &Type,
    missing_return_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    let with_span = |diagnostic: Diagnostic| match missing_return_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };

    // tsc runs this check only when the function's end point is reachable
    // (`checkAllCodePathsInNonVoidFunctionReturnOrThrow` is gated on
    // `functionHasImplicitReturn`). A body that always diverges — `while (true)
    // {}`, a returning `if (true)`, a bare `return;` — reports nothing here.
    if body_flow.guarantees_exit {
        return;
    }

    // tsc's switch (checker.go:3777-3800), in order: a `never` return type with a
    // reachable end point is TS2534; no `return` statement at all
    // (`hasExplicitReturn`, which a `throw` does not set) is TS2355; under
    // `strictNullChecks` a type that does not admit `undefined` is TS2366;
    // otherwise `noImplicitReturns` still owes TS7030.
    if matches!(return_type.peeled(), Type::Never) {
        ctx.push(with_span(Diagnostic::ts2534(ctx.file_name.clone())));
        return;
    }

    if !body_flow.contains_return {
        ctx.push(with_span(Diagnostic::ts2355(ctx.file_name.clone())));
        return;
    }

    if body_flow.guarantees_value_return {
        return;
    }

    if ctx.options.strict_null_checks && !return_type_admits_undefined(return_type) {
        ctx.push(with_span(Diagnostic::ts2366(ctx.file_name.clone())));
    } else if ctx.options.no_implicit_returns {
        ctx.push(with_span(Diagnostic::ts7030(ctx.file_name.clone())));
    }
}

/// TS7030 under `noImplicitReturns`: an un-annotated function where some path
/// returns a value but the end point is still reachable. The annotated analogue
/// is [`emit_missing_return_diagnostic`]'s TS2366 branch.
/// The `return;` statements of a body, nested blocks included; a nested
/// function is its own body.
pub(crate) fn bare_return_spans(body: &[ParsedFunctionBodyStatement]) -> Vec<surge_ts_syntax::TextSpan> {
    let mut spans = Vec::new();
    collect_bare_return_spans(body, &mut spans);
    spans
}

fn collect_bare_return_spans(
    body: &[ParsedFunctionBodyStatement],
    spans: &mut Vec<surge_ts_syntax::TextSpan>,
) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Return(statement) => {
                if statement.expression.is_none()
                    && let Some(span) = statement.span
                {
                    spans.push(span);
                }
            }
            ParsedFunctionBodyStatement::Block(statements) => collect_bare_return_spans(statements, spans),
            ParsedFunctionBodyStatement::If(statement) => {
                collect_bare_return_spans(&statement.then_body, spans);
                collect_bare_return_spans(&statement.else_body, spans);
            }
            ParsedFunctionBodyStatement::While(statement) => collect_bare_return_spans(&statement.body, spans),
            ParsedFunctionBodyStatement::ForOf(statement) => collect_bare_return_spans(&statement.body, spans),
            ParsedFunctionBodyStatement::Switch(statement) => {
                for case in &statement.cases {
                    collect_bare_return_spans(&case.consequent, spans);
                }
            }
            ParsedFunctionBodyStatement::Try(statement) => {
                collect_bare_return_spans(&statement.block, spans);
                if let Some(handler) = &statement.handler {
                    collect_bare_return_spans(&handler.body, spans);
                }
                collect_bare_return_spans(&statement.finalizer, spans);
            }
            _ => {}
        }
    }
}

/// tsc's `isUnwrappedReturnTypeUndefinedVoidOrAny`, with surge's sentinel read
/// as `any`.
pub(crate) fn is_undefined_void_or_any(return_type: &Type) -> bool {
    match return_type.peeled() {
        Type::Void | Type::Any | Type::Undefined | Type::GenuineUnknown => true,
        Type::Union(union) => union.types().iter().any(|member| matches!(member, Type::Void)),
        other => other.is_unknown(),
    }
}

pub(crate) fn emit_implicit_return_diagnostic(
    missing_return_span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    let diagnostic = Diagnostic::ts7030(ctx.file_name.clone());
    let diagnostic = match missing_return_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };
    ctx.push(diagnostic);
}

/// See the call site in [`check_function_body`]. Returns the previous
/// `module_value_fallback` when one was installed, for the caller to restore.
pub(crate) fn install_deferred_block_value_fallback(
    body: &[ParsedFunctionBodyStatement],
    ctx: &mut CheckerContext,
) -> Option<Option<std::sync::Arc<crate::symbols::SymbolTable>>> {
    let mut annotated = crate::symbols::SymbolTable::new();
    let mut found = false;
    for statement in body {
        let ParsedFunctionBodyStatement::VariableDeclaration(variable) = statement else {
            continue;
        };
        if !matches!(
            variable.kind,
            surge_ts_syntax::ParsedVariableKind::Let | surge_ts_syntax::ParsedVariableKind::Const
        ) {
            continue;
        }
        // The sentinel, never the declared type: the name *is* in scope for the
        // deferred body (tsc resolves it), surge simply has no usable type for it
        // before the declaration's own statement runs, and the sentinel says
        // exactly that — it reports nothing and cascades nothing. Resolving the
        // annotation here instead would be worse than useless: it runs out of
        // statement position, and the alias cache would keep that first answer,
        // swallowing the one-time diagnostic the real check owes.
        found = true;
        let ty = Type::Unknown;
        let _ = annotated.insert(
            variable.name.clone(),
            SymbolInfo {
                ty,
                kind: match variable.kind {
                    surge_ts_syntax::ParsedVariableKind::Let => {
                        crate::symbols::SymbolKind::Let
                    }
                    _ => crate::symbols::SymbolKind::Const,
                },
                function_signature: None,
            },
        );
    }
    // A `var` is declared in its function's locals, not at its statement
    // (`declareSymbolAndAddToSymbolTable`): a closure or a hoisted function's
    // parameter default evaluated ahead of the statement resolves it too.
    let mut var_names = Vec::new();
    crate::flow::collect_var_names(body, &mut var_names);
    for name in var_names {
        if annotated.get_own(&name).is_some() {
            continue;
        }
        found = true;
        let _ = annotated.insert(
            name,
            SymbolInfo {
                ty: Type::Unknown,
                kind: crate::symbols::SymbolKind::Var,
                function_signature: None,
            },
        );
    }
    if !found {
        return None;
    }
    let annotated = match ctx.module_value_fallback.clone() {
        Some(parent) => annotated.with_parent_fallback(parent),
        None => annotated,
    };
    Some(std::mem::replace(
        &mut ctx.module_value_fallback,
        Some(std::sync::Arc::new(annotated)),
    ))
}

pub(crate) fn check_function_body(
    body: Vec<ParsedFunctionBodyStatement>,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    record_function_body_check();
    record_flow_function_count();

    let mut pushed_scope = false;
    if flow_state.is_enabled() {
        let future_block_scoped_declarations = collect_future_block_scoped_declarations(&body);
        if !future_block_scoped_declarations.is_empty() {
            flow_state.push_scope(future_block_scoped_declarations);
            flow_state.record_enum_objects(&body);
            pushed_scope = true;
        }
    } else {
        record_flow_function_skipped_count();
    }

    // Body-local `type`/`interface`/`class` declarations bind a block-scoped
    // type: install them as the innermost type-declaration layer for the whole
    // body (they are visible before their statement position, like the function
    // hoist below).
    let saved_type_declaration_scope = install_body_local_type_declarations(&body, scopes, ctx);

    // A nested function body is deferred: it runs after the enclosing block has
    // finished evaluating, so it may legally name a `const`/`let` declared later
    // in that block. Recursive schemas are written exactly this way
    // (`const A: Schema = lazy(() => object({ b: B })); const B: Schema = ...`).
    // Publish those bindings as a fallback rather than into the scope itself, so
    // straight-line resolution order — and the flow state built from it — is
    // unchanged.
    //
    // Installed before the hoist below, not after: a hoisted signature's
    // annotation is mapped there, and `function g(...args:
    // ConstructorParameters<typeof E>)` under a sibling `const E = Error`
    // otherwise reported a false TS2304 for a name that is plainly in scope.
    let saved_module_value_fallback = install_deferred_block_value_fallback(&body, ctx);

    // Hoist nested `function` declarations into the current scope so a sibling
    // closure can call them (function declarations are function-scoped and
    // callable before their statement position). A signature that reads its own
    // function or a later one gets the one-pass-earlier signature, as at the
    // top level (`hoist_function_declarations`).
    let nested_functions: Vec<&surge_ts_syntax::ParsedFunctionDeclaration> = body
        .iter()
        .filter_map(|statement| match statement {
            ParsedFunctionBodyStatement::Function(function) => Some(function.as_ref()),
            _ => None,
        })
        .collect();
    let local_types: Vec<(&str, Vec<&surge_ts_syntax::ParsedType>)> = body
        .iter()
        .filter_map(|statement| match statement {
            ParsedFunctionBodyStatement::TypeAlias(alias) => Some((alias.name.as_str(), vec![&alias.ty])),
            ParsedFunctionBodyStatement::Interface(interface) => Some((
                interface.name.as_str(),
                crate::checks::function::interface_written_types(interface),
            )),
            _ => None,
        })
        .collect();
    if crate::checks::function::signatures_read_ahead(&nested_functions, &local_types) {
        for function in &nested_functions {
            scopes.insert_current(
                function.name.as_str(),
                SymbolInfo {
                    ty: Type::Unknown,
                    kind: crate::symbols::SymbolKind::Function,
                    function_signature: None,
                },
            );
        }
        let collection_order: Vec<&surge_ts_syntax::ParsedFunctionDeclaration> =
            crate::checks::function::signature_collection_order(&nested_functions, &local_types)
                .into_iter()
                .map(|position| nested_functions[position])
                .collect();
        let diagnostics_before = ctx.diagnostics().len();
        hoist_nested_functions(&collection_order, scopes, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
    }
    hoist_nested_functions(&nested_functions, scopes, ctx);
    crate::modules::exports::apply_body_expando_members(&body, scopes, ctx);
    let saved_expando_objects = extend_javascript_expando_objects(&body, ctx);
    let saved_expando_initializer_members =
        crate::checks::var::install_expando_initializer_members(&body, ctx);


    // A body-local type declaration's own body may name a body-local *value*
    // (`type Schema = Infer<typeof schema>`), and the `typeof` arm resolves
    // names through `ctx.symbols` — a file-level table that never holds function
    // locals. Republish the body's visible value scope there for the duration of
    // the statement loop so the declaration resolves wherever it is forced from.
    // Restricted to bodies that actually declare a local type: the republished
    // table is shared with the scope stack, so every subsequent local binding
    // copies the (small) locals map.
    let saved_symbols = saved_type_declaration_scope
        .is_some()
        .then(|| std::mem::take(&mut ctx.symbols));

    let mut nested_functions = Vec::new();
    let mut nested_classes = Vec::new();
    for (statement_index, statement) in body.into_iter().enumerate() {
        if let ParsedFunctionBodyStatement::Function(function) = statement {
            nested_functions.push(function);
            continue;
        }
        if let ParsedFunctionBodyStatement::Class(class) = &statement {
            nested_classes.push(class.clone());
        }
        if saved_symbols.is_some() {
            ctx.symbols = scopes.visible_symbols().clone();
        }
        check_function_body_statement(
            statement,
            statement_index,
            with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
            scopes,
            flow_state,
            ctx,
        );
    }

    if let Some(saved) = saved_expando_objects {
        ctx.javascript_expando_objects = saved;
    }
    if let Some(saved) = saved_expando_initializer_members {
        ctx.expando_initializer_members = saved;
    }

    // A nested `function` is hoisted: it may read a binding declared after it
    // (called only once that binding exists), so its body is checked once the
    // whole block is bound. It sees every binding at its declared type — tsc
    // carries no narrowing into a function declaration.
    if !nested_functions.is_empty() || !nested_classes.is_empty() {
        let enclosing = declared_type_view(scopes.visible_symbols());
        for function in nested_functions {
            crate::checks::function::check_nested_function_declaration(*function, &enclosing, ctx);
        }
        for class in nested_classes {
            crate::program::check_nested_class_declaration(&class, &enclosing, ctx);
        }
    }

    if pushed_scope {
        flow_state.pop_scope();
    }

    if let Some(saved) = saved_symbols {
        ctx.symbols = saved;
    }

    if let Some(saved) = saved_type_declaration_scope {
        ctx.type_declaration_scope = saved;
    }

    if let Some(saved) = saved_module_value_fallback {
        ctx.module_value_fallback = saved;
    }
}

/// tsc's binder looks an expando write's container up in the write's own
/// container (`lookupEntity`), so a JavaScript body's variables initialized
/// with a function or an empty object literal (`IsExpandoInitializer`) are
/// containers for the writes in that body, as the module's are for the
/// module's. Returns the file's set, to restore once the body's statements
/// are checked.
fn extend_javascript_expando_objects(
    body: &[ParsedFunctionBodyStatement],
    ctx: &mut CheckerContext,
) -> Option<std::sync::Arc<std::collections::HashSet<String>>> {
    if !surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        return None;
    }
    let mut names = body
        .iter()
        .filter_map(|statement| match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable)
                if variable.declared_type.is_none()
                    && match &variable.initializer {
                        Some(surge_ts_syntax::ParsedExpression::ObjectLiteral { properties, .. }) => {
                            properties.is_empty()
                        }
                        Some(surge_ts_syntax::ParsedExpression::ArrowFunction(_)) => true,
                        _ => false,
                    } =>
            {
                Some(variable.name.clone())
            }
            _ => None,
        })
        .peekable();
    names.peek()?;
    let mut extended = (*ctx.javascript_expando_objects).clone();
    extended.extend(names);
    Some(std::mem::replace(
        &mut ctx.javascript_expando_objects,
        std::sync::Arc::new(extended),
    ))
}

fn hoist_nested_functions(
    functions: &[&surge_ts_syntax::ParsedFunctionDeclaration],
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    for function in functions {
        // The signature mapper seeds parameter-default evaluation from
        // `ctx.symbols`, a file-level table that never holds function locals,
        // so a default referring to an enclosing local read as unresolved.
        // The visible scope is already in hand two lines below.
        let saved_symbols = std::mem::replace(&mut ctx.symbols, scopes.visible_symbols().clone());
        let function_type = crate::checks::function::signature::map_function_signature(
            &function.parameters,
            function.return_type.as_ref(),
            &function.type_parameters,
            None,
            ctx,
        );
        ctx.symbols = saved_symbols;
        let signature_info = crate::checks::function::signature::function_declaration_signature_info(
            function,
            &function_type,
            scopes.visible_symbols(),
            ctx,
        );
        let function_type = with_instantiated_outer_bindings(function_type, &signature_info, ctx);
        scopes.insert_current_handle(
            function.name.as_str(),
            std::sync::Arc::new(SymbolInfo {
                ty: Type::Function(function_type),
                kind: crate::symbols::SymbolKind::Function,
                function_signature: Some(signature_info),
            }),
        );
    }
}

/// A generic function declared in a body read for a call keeps its written
/// signature and the bindings the body was read under, so a call through the
/// value it returns checks `V` of `function bar<V extends U>` against `U` as
/// bound there: `outer<U>`'s body for `outer("")` (`instantiated_body_return`),
/// or a method's per instantiation. A body checked on its own binds nothing.
fn with_instantiated_outer_bindings(
    function_type: surge_ts_types::FunctionType,
    signature: &std::sync::Arc<crate::symbols::FunctionSignatureInfo>,
    ctx: &CheckerContext,
) -> surge_ts_types::FunctionType {
    if signature.type_parameters.is_empty() {
        return function_type;
    }
    let merged = crate::infer::types::merged_type_parameter_substitution(
        ctx,
        &crate::infer::TypeParameterSubstitution::new(),
    );
    let own: Vec<&str> = signature.type_parameters.iter().map(|parameter| parameter.name.as_str()).collect();
    let outer_type_arguments: Vec<(String, Type)> = merged
        .iter()
        .filter(|(name, _)| !own.contains(&name.as_ref()) && !merged.is_placeholder(name))
        .map(|(name, ty)| (name.to_string(), ty.clone()))
        .collect();
    if outer_type_arguments.is_empty() {
        return function_type;
    }
    function_type.with_declaration(std::sync::Arc::new(crate::checks::call::DeclaredMemberSignature {
        signature: signature.clone(),
        outer_type_arguments,
        generic_shape: None,
    }))
}

/// `symbols` with every narrowed binding, in it or a parent, back at its
/// declared type.
pub(crate) fn declared_type_view(symbols: &SymbolTable) -> SymbolTable {
    let mut view = symbols.clone_with_reason(TypeCopyReason::FunctionBodySetup);
    // A function declaration never continues the enclosing flow.
    view.mark_auto_arrays_declared_only();
    let mut narrowed: Vec<std::sync::Arc<str>> = Vec::new();
    let mut table = Some(symbols);
    while let Some(current) = table {
        narrowed.extend(current.narrowed_names().cloned());
        table = current.parent_table();
    }
    for name in narrowed {
        let (Some(declared), Some(symbol)) = (symbols.declared_type(&name), symbols.get(&name))
        else {
            continue;
        };
        let restored = SymbolInfo {
            ty: declared.clone(),
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        let _ = view.insert(name, restored);
    }
    view
}

/// Binds the body's own `type`/`interface`/`class` declarations as an inner
/// type-declaration layer (and, for classes, a local value symbol), returning
/// the scope to restore once the body has been checked.
///
/// Returns `None` — installing nothing — for the overwhelmingly common body
/// that declares no local types.
fn install_body_local_type_declarations(
    body: &[ParsedFunctionBodyStatement],
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) -> Option<Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>> {
    if !body.iter().any(|statement| {
        matches!(
            statement,
            ParsedFunctionBodyStatement::TypeAlias(_)
                | ParsedFunctionBodyStatement::Interface(_)
                | ParsedFunctionBodyStatement::Class(_)
        )
    }) {
        return None;
    }

    let declarations = collect_body_local_type_declarations(body, ctx);

    let outer_layers: Vec<std::sync::Arc<crate::symbols::TypeDeclarationTable>> = ctx
        .type_declaration_scope
        .as_ref()
        .map(|scope| scope.layers().to_vec())
        .unwrap_or_default();
    // The body's layers, and an enclosing body's, are inner scopes: a name
    // they bind shadows the file's own declaration of it.
    let outer_lexical_layers = ctx
        .type_declaration_scope
        .as_ref()
        .map_or(0, |scope| scope.lexical_layer_count());

    // Each declaration's own body resolves against the declarations that
    // precede it plus a placeholder layer, never against a scope that transitively
    // contains the declaration itself: a scope that reached back into the layer
    // holding it would close an `Arc` cycle and leak the layer for the run.
    // Forward/self references and the enclosing function's type parameters land
    // on the placeholder layer instead, degrading to `unknown` rather than
    // reporting an unresolved name.
    let placeholder_layer = std::sync::Arc::new(body_local_placeholder_table(&declarations, ctx));
    let mut prefix = crate::symbols::TypeDeclarationTable::new();
    let mut body_layer = crate::symbols::TypeDeclarationTable::new();
    for (name, declaration) in declarations {
        let mut layers = Vec::with_capacity(outer_layers.len() + 2);
        layers.push(std::sync::Arc::new(prefix.clone()));
        layers.push(placeholder_layer.clone());
        layers.extend(outer_layers.iter().cloned());
        let scope = std::sync::Arc::new(
            crate::symbols::TypeDeclarationScope::new(layers).with_lexical_layers(2 + outer_lexical_layers),
        );

        let declaration = with_resolution_scope(declaration, scope);
        let _ = prefix.insert(name.as_str(), declaration.clone());
        let _ = body_layer.insert(name.as_str(), declaration);
    }

    let mut layers = Vec::with_capacity(outer_layers.len() + 1);
    layers.push(std::sync::Arc::new(body_layer));
    layers.extend(outer_layers);

    let saved = ctx.type_declaration_scope.take();
    let body_scope = std::sync::Arc::new(
        crate::symbols::TypeDeclarationScope::new(layers).with_lexical_layers(1 + outer_lexical_layers),
    );
    if crate::semantic::recording() {
        for statement in body {
            let name_span = match statement {
                ParsedFunctionBodyStatement::TypeAlias(alias) => alias.name_span,
                ParsedFunctionBodyStatement::Interface(interface) => interface.name_span,
                ParsedFunctionBodyStatement::Class(class) => class.name_span,
                _ => None,
            };
            crate::semantic::record_local_type_scope(name_span, &body_scope, ctx);
        }
    }
    ctx.type_declaration_scope = Some(body_scope);

    // The real static type is built at the class's own statement position, once
    // the values its heritage clause may name (`class D extends Parent {}` over a
    // preceding `const Parent = …`) are bound. Reserve the name here so a read
    // from an earlier closure degrades to `any` instead of reporting TS2304.
    for statement in body {
        if let ParsedFunctionBodyStatement::Class(class) = statement {
            scopes.insert_current_handle(
                class.name.as_str(),
                std::sync::Arc::new(SymbolInfo {
                    ty: Type::Any,
                    kind: crate::symbols::SymbolKind::Const,
                    function_signature: None,
                }),
            );
        }
    }

    Some(saved)
}

/// Binds a class expression's instance type under the class's name, as
/// `install_body_local_type_declarations` binds a body-local class's, and
/// returns the scope to restore once the class has been read.
pub(crate) fn install_class_expression_type_declaration(
    class: &surge_ts_syntax::ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) -> Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>> {
    let mut info = crate::program::class_instance_interface_info(class, ctx.file_name_arc());
    info.name = body_local_declaration_name(&class.name, class.name_span).into();
    let mut declaration = crate::symbols::TypeDeclarationInfo::Interface(info);
    set_declared_name(&mut declaration, &class.name);
    let declarations = vec![(class.name.clone(), declaration)];

    let outer_layers: Vec<std::sync::Arc<crate::symbols::TypeDeclarationTable>> = ctx
        .type_declaration_scope
        .as_ref()
        .map(|scope| scope.layers().to_vec())
        .unwrap_or_default();
    let outer_lexical_layers = ctx
        .type_declaration_scope
        .as_ref()
        .map_or(0, |scope| scope.lexical_layer_count());
    // The class's own members resolve against the placeholder layer, never a
    // scope holding the class itself — see `install_body_local_type_declarations`.
    let placeholder_layer = std::sync::Arc::new(body_local_placeholder_table(&declarations, ctx));
    let mut declaration_layers = Vec::with_capacity(outer_layers.len() + 1);
    declaration_layers.push(placeholder_layer);
    declaration_layers.extend(outer_layers.iter().cloned());
    let declaration_scope = std::sync::Arc::new(
        crate::symbols::TypeDeclarationScope::new(declaration_layers)
            .with_lexical_layers(1 + outer_lexical_layers),
    );

    let mut class_layer = crate::symbols::TypeDeclarationTable::new();
    for (name, declaration) in declarations {
        let _ = class_layer.insert(name.as_str(), with_resolution_scope(declaration, declaration_scope.clone()));
    }
    let mut layers = Vec::with_capacity(outer_layers.len() + 1);
    layers.push(std::sync::Arc::new(class_layer));
    layers.extend(outer_layers);
    std::mem::replace(
        &mut ctx.type_declaration_scope,
        Some(std::sync::Arc::new(
            crate::symbols::TypeDeclarationScope::new(layers).with_lexical_layers(1 + outer_lexical_layers),
        )),
    )
}

/// Opt-in (`SURGE_LOCAL_TYPE_DECLARATION_CHECKS=1`): check a body-local type
/// declaration at its statement. Off by default: on ts-pattern it turns 63
/// genuine surge/tsc type-level divergences (`FindSelected`, `InvertPattern`,
/// `DeepExclude` assertions) into `TS2344` reports on a corpus that otherwise
/// stands at one. Flip it once those are burned down.
fn body_local_declaration_checks_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SURGE_LOCAL_TYPE_DECLARATION_CHECKS").as_deref() == Ok("1")
    })
}

/// Checks the body-local declaration `name` binds (installed by
/// `install_body_local_type_declarations`) the way a top-level one is checked
/// by `validate_local_type_declarations`.
fn validate_body_local_declaration(name: &str, ctx: &mut CheckerContext) {
    if !body_local_declaration_checks_enabled() {
        return;
    }
    let Some(declaration) = ctx
        .type_declaration_scope
        .as_ref()
        .and_then(|scope| scope.get(name))
        .cloned()
    else {
        return;
    };
    crate::infer::types::validate_local_type_declaration(&declaration, ctx);
}

/// The body-local declarations of `body`, paired with the name each binds.
///
/// A declaration is registered under a synthetic, declaration-site-unique
/// internal name (`T@<offset>`) with the source name kept as its display name:
/// the program-wide resolution caches are keyed on `(file, declaration name)`,
/// and two sibling function bodies in one file may legitimately declare the same
/// local type with different bodies (zod's test files do exactly that).
fn collect_body_local_type_declarations(
    body: &[ParsedFunctionBodyStatement],
    ctx: &mut CheckerContext,
) -> Vec<(String, crate::symbols::TypeDeclarationInfo)> {
    use crate::symbols::{InterfaceInfo, TypeAliasInfo, TypeDeclarationInfo};

    let file_name = ctx.file_name_arc();
    let mut declarations = Vec::new();

    for statement in body {
        let declaration = match statement {
            ParsedFunctionBodyStatement::TypeAlias(alias) => {
                let info = TypeAliasInfo::new(
                    body_local_declaration_name(&alias.name, alias.name_span),
                    file_name.clone(),
                    alias.name_span,
                    alias.type_parameters.clone(),
                    alias.ty.clone(),
                    None,
                )
                .with_enum_name(
                    alias.enum_name.as_deref(),
                    alias.enum_exported,
                    alias.enum_is_const,
                )
                .with_enum_members(&alias.enum_members);
                (alias.name.clone(), TypeDeclarationInfo::Alias(info))
            }
            ParsedFunctionBodyStatement::Interface(interface) => {
                let info = InterfaceInfo::new(
                    body_local_declaration_name(&interface.name, interface.name_span),
                    file_name.clone(),
                    interface.name_span,
                    interface.type_parameters.clone(),
                    interface.extends.clone(),
                    interface.members.clone(),
                    interface.string_index_type.clone(),
                    interface.number_index_type.clone(),
                    interface.call_signature.clone(),
                    interface.call_signature_overloads.clone(),
                    interface.construct_signatures.clone(),
                    None,
                )
                .with_readonly_indexes(interface.string_index_readonly, interface.number_index_readonly)
                .with_keyed_indexes(interface.keyed_indexes.clone())
                .with_index_parameter_names(interface.index_parameter_names.clone());
                (interface.name.clone(), TypeDeclarationInfo::Interface(info))
            }
            ParsedFunctionBodyStatement::Class(class) => {
                let mut info =
                    crate::program::class_instance_interface_info(class, file_name.clone());
                info.name = body_local_declaration_name(&class.name, class.name_span).into();
                (class.name.clone(), TypeDeclarationInfo::Interface(info))
            }
            _ => continue,
        };

        let (name, mut info) = declaration;
        set_declared_name(&mut info, &name);
        declarations.push((name, info));
    }

    declarations
}

/// The degradation layer a body-local declaration's own body resolves against:
/// the enclosing function's type parameters plus every body-local name,
/// each bound to the `unknown` sentinel. Without it a body-local alias over an
/// outer type parameter (`type TError = ClientError<TRouter>`) would merely move
/// the unresolved-name report from the use sites to the declaration.
fn body_local_placeholder_table(
    declarations: &[(String, crate::symbols::TypeDeclarationInfo)],
    ctx: &mut CheckerContext,
) -> crate::symbols::TypeDeclarationTable {
    use crate::symbols::{TypeAliasInfo, TypeDeclarationInfo};

    let file_name = ctx.file_name_arc();
    let anchor = declarations
        .iter()
        .find_map(|(_, declaration)| body_local_declaration_anchor(declaration));
    let mut table = crate::symbols::TypeDeclarationTable::new();

    let placeholder = |name: &str,
                       type_parameters: Vec<surge_ts_syntax::ParsedTypeParameter>,
                       table: &mut crate::symbols::TypeDeclarationTable| {
        let internal_name = with_body_instantiation(match anchor {
            Some(anchor) => format!("{name}@{anchor}"),
            None => name.to_string(),
        });
        // The placeholder keeps the declaration's own type parameters so a
        // forward reference written with arguments (`Some<number>` before
        // `type Some<T> = …`) binds them instead of reporting the placeholder
        // as not generic.
        let mut info = TypeDeclarationInfo::Alias(TypeAliasInfo::new(
            internal_name,
            file_name.clone(),
            None,
            type_parameters,
            surge_ts_syntax::ParsedType::Unknown,
            None,
        ));
        set_declared_name(&mut info, name);
        let _ = table.insert(name, info);
    };

    for (name, declaration) in declarations {
        let type_parameters = match declaration {
            TypeDeclarationInfo::Alias(alias) => alias.body.type_parameters.clone(),
            TypeDeclarationInfo::Interface(interface) => interface.body.type_parameters.clone(),
        };
        placeholder(name, type_parameters, &mut table);
    }
    for scope in &ctx.type_parameter_scopes {
        for name in scope.keys() {
            placeholder(name, Vec::new(), &mut table);
        }
    }

    table
}

fn body_local_declaration_anchor(
    declaration: &crate::symbols::TypeDeclarationInfo,
) -> Option<usize> {
    match declaration {
        crate::symbols::TypeDeclarationInfo::Alias(alias) => alias.name_span,
        crate::symbols::TypeDeclarationInfo::Interface(interface) => interface.name_span,
    }
    .map(|span| span.start)
}

fn body_local_declaration_name(name: &str, name_span: Option<surge_ts_syntax::TextSpan>) -> String {
    with_body_instantiation(match name_span {
        Some(span) => format!("{name}@{}", span.start),
        None => name.to_string(),
    })
}

/// A body-local declaration read for one generic call is that call's own
/// (see `body_instantiation_tag`).
fn with_body_instantiation(internal_name: String) -> String {
    match super::body_instantiation_tag() {
        Some(tag) => format!("{internal_name}#{tag:x}"),
        None => internal_name,
    }
}

fn set_declared_name(declaration: &mut crate::symbols::TypeDeclarationInfo, name: &str) {
    match declaration {
        crate::symbols::TypeDeclarationInfo::Alias(alias) => {
            alias.declared_name = Some(std::sync::Arc::from(name));
        }
        crate::symbols::TypeDeclarationInfo::Interface(interface) => {
            interface.declared_name = Some(std::sync::Arc::from(name));
        }
    }
}

fn with_resolution_scope(
    declaration: crate::symbols::TypeDeclarationInfo,
    scope: std::sync::Arc<crate::symbols::TypeDeclarationScope>,
) -> crate::symbols::TypeDeclarationInfo {
    match declaration {
        crate::symbols::TypeDeclarationInfo::Alias(mut alias) => {
            alias.resolution_scope = Some(scope);
            crate::symbols::TypeDeclarationInfo::Alias(alias)
        }
        crate::symbols::TypeDeclarationInfo::Interface(mut interface) => {
            interface.resolution_scope = Some(scope);
            crate::symbols::TypeDeclarationInfo::Interface(interface)
        }
    }
}

pub(crate) fn check_function_body_statement(
    statement: ParsedFunctionBodyStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    // Collected before the statement is consumed; applied once it is checked,
    // where tsc's flow places an array mutation.
    let mutations = if super::body_statements::evolving_arrays::auto_arrays_visible(scopes, ctx) {
        collect_array_mutations(&statement, scopes.visible_symbols())
    } else {
        Vec::new()
    };
    // Assignments inside the statement's own expressions take effect once it
    // has run; a condition's are applied by its statement before the branches.
    let assigning: Vec<surge_ts_syntax::ParsedExpression> = statement_value_expressions(&statement)
        .into_iter()
        .filter(|expression| expression.contains_assignment() || expression.contains_member_assignment())
        .cloned()
        .collect();
    let immediately_invoked = match &statement {
        ParsedFunctionBodyStatement::Expression(expression) => {
            super::body_statements::immediately_invoked_writes(expression)
        }
        _ => None,
    };
    let seeded = ctx.immediately_invoked_unassigned.len();
    let seeds = immediately_invoked_unassigned(&statement_value_expressions(&statement), flow_state);
    ctx.immediately_invoked_unassigned.extend(seeds);
    check_function_body_statement_itself(
        statement,
        statement_index,
        return_type,
        scopes,
        flow_state,
        ctx,
    );
    ctx.immediately_invoked_unassigned.truncate(seeded);
    if let Some(writes) = immediately_invoked {
        super::body_statements::apply_immediately_invoked_assignments(writes, scopes, flow_state, ctx);
    }
    if !mutations.is_empty() {
        apply_array_mutations(mutations, scopes, ctx);
    }
    for expression in &assigning {
        apply_expression_assignments(expression, scopes, flow_state, ctx);
    }
}

/// The bindings each function the statement calls where it is written reads
/// as still unassigned: those unassigned when the statement starts, less any
/// its own expressions write, which may run before the call.
pub(crate) fn immediately_invoked_unassigned(
    expressions: &[&surge_ts_syntax::ParsedExpression],
    flow_state: &FunctionFlowState,
) -> Vec<(usize, Vec<std::sync::Arc<str>>)> {
    fn collect(
        expression: &surge_ts_syntax::ParsedExpression,
        functions: &mut Vec<usize>,
        written: &mut Vec<String>,
    ) {
        match expression {
            surge_ts_syntax::ParsedExpression::ExpressionCall { callee, .. } => {
                if let surge_ts_syntax::ParsedExpression::ArrowFunction(function) = callee.as_ref()
                    && let Some(span) = function.span
                {
                    functions.push(span.start);
                }
            }
            surge_ts_syntax::ParsedExpression::Assignment { target_name, .. } => {
                written.push(target_name.clone());
            }
            _ => {}
        }
        expression.for_each_child(&mut |child| collect(child, functions, written));
    }
    let mut functions = Vec::new();
    let mut written = Vec::new();
    for expression in expressions {
        collect(expression, &mut functions, &mut written);
    }
    if functions.is_empty() {
        return Vec::new();
    }
    let unassigned: Vec<std::sync::Arc<str>> = flow_state
        .unassigned_bindings()
        .into_iter()
        .filter(|name| !written.iter().any(|written| written.as_str() == name.as_ref()))
        .collect();
    if unassigned.is_empty() {
        return Vec::new();
    }
    functions.into_iter().map(|start| (start, unassigned.clone())).collect()
}

fn statement_value_expressions(
    statement: &ParsedFunctionBodyStatement,
) -> Vec<&surge_ts_syntax::ParsedExpression> {
    match statement {
        ParsedFunctionBodyStatement::Expression(expression) => vec![expression],
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            variable.initializer.iter().collect()
        }
        ParsedFunctionBodyStatement::Return(statement) => statement.expression.iter().collect(),
        ParsedFunctionBodyStatement::Throw(statement) => vec![&statement.expression],
        ParsedFunctionBodyStatement::Assignment(assignment) => vec![&assignment.value],
        ParsedFunctionBodyStatement::MemberAssignment(assignment) => vec![&assignment.value],
        ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => vec![&assignment.value],
        _ => Vec::new(),
    }
}

fn check_function_body_statement_itself(
    statement: ParsedFunctionBodyStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    record_flow_statement_count();
    match statement {
        // Checked by `check_function_body` once its block is fully bound.
        ParsedFunctionBodyStatement::Function(_) => {}
        // The type side is bound ahead of the statement loop by
        // `install_body_local_type_declarations`; the declaration is *checked*
        // here, at its own statement, once the values before it are in scope —
        // its body may query one (`type t = Expect<Equal<typeof x, string>>`),
        // and a body nothing reads was never resolved at all, so every
        // assertion a type-level test suite is made of reported nothing. A
        // class's members are checked with the block's nested functions, once
        // the block is bound.
        ParsedFunctionBodyStatement::TypeAlias(alias) => {
            validate_body_local_declaration(&alias.name, ctx);
        }
        ParsedFunctionBodyStatement::Interface(interface) => {
            validate_body_local_declaration(&interface.name, ctx);
        }
        ParsedFunctionBodyStatement::Class(class) => {
            if flow_state.tracked_local_count() > 0 {
                crate::flow::walk_class(&class, statement_index, flow_state, ctx);
            }
            let visible_symbols = visible_symbols(scopes);
            crate::program::check_class_head_expressions(&class, &visible_symbols, ctx);
            crate::flow::check_class_member_flow(&class, ctx);
            let symbol = crate::program::build_class_value_symbol(&class, ctx);
            scopes.insert_current_handle(class.name.as_str(), std::sync::Arc::new(symbol));
        }
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            let start = Instant::now();
            let enum_members = variable
                .enum_members
                .clone()
                .map(|bodies| (variable.name.clone(), bodies));
            check_function_variable_declaration(
                *variable,
                statement_index,
                scopes,
                flow_state,
                ctx,
            );
            if let Some((name, bodies)) = enum_members {
                let symbols = visible_symbols(scopes)
                    .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
                crate::checks::enum_members::check_enum_members(
                    &name, &bodies, symbols, false, ctx,
                );
            }
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.variable_declaration_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::Block(block_body) => {
            check_function_block(block_body, return_type, scopes, flow_state, ctx);
        }
        ParsedFunctionBodyStatement::Return(return_statement) => {
            let start = Instant::now();
            let visible_symbols = visible_symbols(scopes);
            check_function_return_statement(
                *return_statement,
                statement_index,
                return_type,
                flow_state,
                &visible_symbols,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.return_statement_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::Throw(throw_statement) => {
            check_function_throw_statement(
                *throw_statement,
                statement_index,
                scopes,
                flow_state,
                ctx,
            );
        }
        ParsedFunctionBodyStatement::Continue => super::body_statements::record_continue_edge(scopes),
        ParsedFunctionBodyStatement::Break
        | ParsedFunctionBodyStatement::BreakOut
        | ParsedFunctionBodyStatement::LabelEnd => {}
        ParsedFunctionBodyStatement::Assignment(assignment) => {
            let start = Instant::now();
            check_function_assignment(*assignment, statement_index, scopes, flow_state, ctx);
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.assignability_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => {
            let start = Instant::now();
            check_this_property_assignment(*assignment, scopes, ctx);
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.assignability_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
            let start = Instant::now();
            // `o.p = v` reads `o` (and `v`); only the member is written.
            if flow_state.tracked_local_count() > 0 {
                let _ = crate::flow::check_member_target_flow(
                    &assignment.target,
                    assignment.target_span,
                    flow_state,
                    statement_index,
                    ctx,
                );
                let _ = crate::flow::check_expression_flow(
                    &assignment.value,
                    assignment.value_span,
                    flow_state,
                    statement_index,
                    ctx,
                );
                flow_state.mark_expando_member_assigned(&assignment.target);
            }
            check_member_assignment(*assignment, scopes, ctx);
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.assignability_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::Expression(expression) => {
            let start = Instant::now();
            check_function_expression_statement(
                *expression,
                statement_index,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.expression_statement_checking += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::If(if_statement) => {
            let start = Instant::now();
            check_function_if_statement(
                *if_statement,
                statement_index,
                return_type,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.flow_narrowing += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            let start = Instant::now();
            check_function_while_statement(
                *while_statement,
                statement_index,
                return_type,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.flow_narrowing += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
            let start = Instant::now();
            check_function_for_of_statement(
                *for_of_statement,
                statement_index,
                return_type,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.flow_narrowing += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::Switch(switch_statement) => {
            let start = Instant::now();
            check_function_switch_statement(
                *switch_statement,
                statement_index,
                return_type,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.flow_narrowing += start.elapsed()
            });
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            let start = Instant::now();
            check_function_try_statement(
                *try_statement,
                statement_index,
                return_type,
                scopes,
                flow_state,
                ctx,
            );
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.flow_narrowing += start.elapsed()
            });
        }
    }
}

pub(crate) fn visible_symbols(scopes: &ScopeStack) -> &SymbolTable {
    scopes.visible_symbols()
}

pub(crate) fn check_local_duplicate_declaration(
    variable: &ParsedVariableDeclaration,
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) {
    if matches!(
        variable.kind,
        ParsedVariableKind::Let | ParsedVariableKind::Const
    ) && scopes.current_contains_let_or_const(&variable.name)
    {
        let diagnostic = Diagnostic::ts2451(&variable.name, ctx.file_name.clone());
        let diagnostic = match variable.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };

        ctx.push(diagnostic);
    }
}
