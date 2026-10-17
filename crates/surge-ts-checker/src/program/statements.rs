//! Per-statement program checking and unsupported-declaration diagnostics.

use crate::checks::function::body_statements::evolving_arrays;
use std::collections::HashMap;
use std::time::Instant;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedClassDeclaration, ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedFunctionDeclaration,
    ParsedImportKind, ParsedStatement, TextSpan,
};
use surge_ts_types::FunctionType;

use super::*;

use crate::checks::{assign, call, expr, function as check_function, var};
use crate::context::CheckerContext;

/// `shadowed`: the names a script's own declarations do not bind, the global
/// merge having kept another file's declaration for them.
pub(crate) fn check_program_file_statements(
    statements: &[ParsedStatement],
    file_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    shadowed: &[String],
    ctx: &mut CheckerContext,
) {
    let classes = super::forward_references::file_class_declarations(statements);
    let members = super::forward_references::file_member_declarations(statements, &classes);
    ctx.container_assigned_bindings = std::sync::Arc::new(crate::flow::module_assigned_bindings(statements));
    ctx.expando_initializer_members = std::sync::Arc::new(var::module_expando_initializer_members(
        statements,
        surge_ts_syntax::is_javascript_file_name(&ctx.file_name),
    ));
    check_overload_implementation_compatibility(statements, file_index, function_signatures, ctx);
    let losing_variables = variables_losing_to_earlier_declarations(statements, &ctx.file_name);
    for (statement_index, statement) in statements.iter().cloned().enumerate() {
        let statement = expand_module_if_alias(statement, &statements[..statement_index]);
        super::forward_references::check_statement_forward_references(&statement, &classes, &members, ctx);
        let kept = losing_variables
            .get(&statement_index)
            .and_then(|name| Some((name.clone(), ctx.symbols.remove(name)?)));
        let kept_global = declared_value_name(&statement)
            .filter(|name| shadowed.iter().any(|shadowed| shadowed == name))
            .and_then(|name| Some((name.to_string(), ctx.symbols.get_handle(name)?)));
        check_program_statement(
            statement,
            file_index,
            statement_index,
            function_signatures,
            ctx,
        );
        if let Some((name, symbol)) = kept {
            let _ = ctx.symbols.insert_handle(name, symbol);
        }
        if let Some((name, symbol)) = kept_global {
            let _ = ctx.symbols.insert_handle(name, symbol);
        }
    }
    crate::flow::check_module_definite_assignment(statements, ctx);
}

/// The value a top-level statement declares by name.
fn declared_value_name(statement: &ParsedStatement) -> Option<&str> {
    match statement {
        ParsedStatement::VariableDeclaration(variable) if !variable.from_binding_pattern => Some(variable.name.as_str()),
        ParsedStatement::ClassDeclaration(class) => Some(class.name.as_str()),
        ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declared_value_name(declaration),
            _ => None,
        },
        _ => None,
    }
}

/// A module-level variable conflicting with a function or class of the same
/// name (TS2300) binds nothing: tsc's binder gives the conflicting declaration
/// a symbol of its own, and the name keeps resolving to the one bound first —
/// a function wherever it is written, since function declarations are bound
/// before the other statements, and a class only when written earlier. The
/// variable is still checked, apart from the binding it lost to (the binder
/// reports the conflict as TS2300); its statement index maps to the name whose
/// binding is kept.
fn variables_losing_to_earlier_declarations(
    statements: &[ParsedStatement],
    file_name: &str,
) -> HashMap<usize, String> {
    let mut losing = HashMap::new();
    if surge_ts_syntax::is_javascript_file_name(file_name) {
        return losing;
    }
    fn declaration(statement: &ParsedStatement) -> Option<&ParsedStatement> {
        match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => Some(declaration.as_ref()),
                _ => None,
            },
            statement => Some(statement),
        }
    }
    let mut declared: std::collections::HashSet<&str> = statements
        .iter()
        .filter_map(declaration)
        .filter_map(|statement| match statement {
            ParsedStatement::FunctionDeclaration(function) => Some(function.name.as_str()),
            _ => None,
        })
        .collect();
    for (index, statement) in statements.iter().enumerate() {
        let Some(statement) = declaration(statement) else {
            continue;
        };
        match statement {
            ParsedStatement::ClassDeclaration(class) if !class.const_binding => {
                declared.insert(class.name.as_str());
            }
            ParsedStatement::VariableDeclaration(variable)
                if !variable.from_binding_pattern
                    && !variable.is_enum_object
                    && declared.contains(variable.name.as_str()) =>
            {
                losing.insert(index, variable.name.clone());
            }
            _ => {}
        }
    }
    losing
}

/// `resolveExternalModule`: a specifier naming an `@types` package is TS6137,
/// whether or not it resolves.
fn report_type_declaration_import(specifier: &str, span: Option<surge_ts_syntax::TextSpan>, ctx: &mut CheckerContext) {
    if let Some(package) = specifier.strip_prefix("@types/") {
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            Diagnostic::ts6137(package, specifier, ctx.file_name.clone()),
            span,
        ));
    }
}

/// tsc's `isImplementationCompatibleWithOverload` for a module-level function
/// overload group (TS2394 on the first incompatible overload): the return
/// types must be related in either direction unless the overload returns
/// `void`, the implementation may not require more arguments than the overload
/// declares, and each of the overload's parameters must be assignable to the
/// implementation's (strict variance, as for any function declaration). tsc
/// erases a generic signature's type parameters to `any`
/// (`getErasedSignature`); an overload's resolve to placeholders, which relate
/// the same way, and a generic implementation is left unchecked. An
/// implementation without a written return type is judged by the return its
/// body infers when that is a primitive; any type surge could not resolve is
/// left alone.
fn check_overload_implementation_compatibility(
    statements: &[ParsedStatement],
    file_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    fn function_at(statement: &ParsedStatement) -> Option<&ParsedFunctionDeclaration> {
        match statement {
            ParsedStatement::FunctionDeclaration(function) => Some(function),
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                    ParsedStatement::FunctionDeclaration(function) => Some(function),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }
    let mut index = 0;
    while index < statements.len() {
        let Some(first) = function_at(&statements[index]) else {
            index += 1;
            continue;
        };
        let mut end = index + 1;
        while end < statements.len()
            && function_at(&statements[end]).is_some_and(|function| function.name == first.name)
        {
            end += 1;
        }
        let group: Vec<(usize, &ParsedFunctionDeclaration)> = (index..end)
            .filter_map(|position| function_at(&statements[position]).map(|function| (position, function)))
            .collect();
        index = end;
        let Some(&(implementation_index, implementation)) = group.last() else {
            continue;
        };
        let signature_at = |position: usize| {
            function_signatures.get(&FunctionDeclarationLocation {
                file_index,
                statement_index: position,
            })
        };
        if group.len() >= 2
            && implementation.has_body
            && !implementation.is_declare
            && group[..group.len() - 1].iter().all(|(_, overload)| !overload.has_body)
            && let Some(implementation_type) = signature_at(implementation_index)
            && let Some(bound) = ctx.symbols.get(&implementation.name)
        {
            let overloads = bound.ty.clone();
            ctx.symbols
                .record_hidden_function_implementation(&implementation.name, overloads, implementation_type.clone());
        }
        if group.len() < 2
            || !implementation.has_body
            || implementation.is_declare
            || !implementation.type_parameters.is_empty()
        {
            continue;
        }
        let Some(implementation_type) = signature_at(implementation_index) else {
            continue;
        };
        for &(position, overload) in &group[..group.len() - 1] {
            if overload.has_body {
                continue;
            }
            let Some(overload_type) = signature_at(position) else {
                continue;
            };
            if overload_is_compatible(
                implementation_type,
                implementation.return_type.is_some(),
                overload_type,
            ) {
                continue;
            }
            ctx.push(crate::spans::diagnostic_with_syntax_span(
                Diagnostic::ts2394(ctx.file_name.clone()),
                overload.name_span,
            ));
            // `checkFunctionOrConstructorSymbol` stops at the first
            // incompatible overload.
            break;
        }
    }
}

/// A script's function implementation is the body of the default library's
/// same-named declarations (lib.dom's `declare function toString(): string`),
/// which are its overloads: tsc checks them as it checks the lib file and
/// reports an incompatible one there (TS2394). Only a lib function declared
/// once, implemented without overloads of the script's own, is related.
pub(crate) fn report_lib_overload_incompatibility(
    statements: &[ParsedStatement],
    file_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    use super::LibDeclarationKind;
    for (statement_index, statement) in statements.iter().enumerate() {
        let ParsedStatement::FunctionDeclaration(function) = statement else {
            continue;
        };
        if !function.has_body || function.is_declare || !function.type_parameters.is_empty() {
            continue;
        }
        let declarations = statements
            .iter()
            .filter(|other| matches!(other, ParsedStatement::FunctionDeclaration(other) if other.name == function.name))
            .count();
        if declarations > 1 {
            continue;
        }
        let lib_sites = super::program_lib_declaration_sites(&function.name);
        let [(lib_file, lib_span, LibDeclarationKind::Function)] = lib_sites.as_slice() else {
            continue;
        };
        let Some(overload) = ctx.ambient_global_symbols.get(&function.name).and_then(|symbol| match symbol.ty.peeled() {
            surge_ts_types::Type::Function(overload) if overload.overloads().is_none() => Some(overload.clone()),
            _ => None,
        }) else {
            continue;
        };
        let Some(implementation) = function_signatures.get(&FunctionDeclarationLocation {
            file_index,
            statement_index,
        }) else {
            continue;
        };
        if overload_is_compatible(implementation, function.return_type.is_some(), &overload) {
            continue;
        }
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            Diagnostic::ts2394(lib_file.to_string()),
            Some(*lib_span),
        ));
    }
}

fn overload_is_compatible(
    implementation: &FunctionType,
    implementation_returns_written: bool,
    overload: &FunctionType,
) -> bool {
    use surge_ts_types::{Type, is_assignable_to};
    let unresolved = |ty: &Type| ty.is_unknown() || matches!(ty, Type::TypeParameter(_) | Type::ErrorType);
    let related = |left: &Type, right: &Type| {
        unresolved(left) || unresolved(right) || is_assignable_to(left, right) || is_assignable_to(right, left)
    };
    let implementation_return = if implementation_returns_written {
        Some(implementation.return_type().clone())
    } else {
        Some(implementation.return_type().peeled()).filter(is_primitive_return)
    };
    if let Some(implementation_return) = implementation_return
        && !matches!(overload.return_type(), Type::Void)
        && !related(&implementation_return, overload.return_type())
    {
        return false;
    }
    // A tuple rest spreads into positions surge's tuples cannot mark optional.
    let non_array_rest = |signature: &FunctionType| {
        signature.is_variadic()
            && signature
                .parameters()
                .last()
                .is_some_and(|rest| !matches!(rest.peeled(), Type::Array(_)))
    };
    if non_array_rest(implementation) || non_array_rest(overload) {
        return true;
    }
    let overload_count = overload.parameters().len();
    if !overload.is_variadic() && implementation.required_parameter_count() > overload_count {
        return false;
    }
    let parameter_at = |signature: &FunctionType, position: usize| -> Option<Type> {
        let parameters = signature.parameters();
        if signature.is_variadic() && position + 1 >= parameters.len() {
            return parameters.last().map(|rest| match rest.peeled() {
                Type::Array(element) => *element,
                other => other,
            });
        }
        let parameter = parameters.get(position)?;
        Some(if position >= signature.required_parameter_count() {
            surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
        } else {
            parameter.clone()
        })
    };
    let count = implementation.parameters().len().max(overload_count);
    (0..count).all(|position| {
        match (parameter_at(implementation, position), parameter_at(overload, position)) {
            (Some(source), Some(target)) => {
                unresolved(&source) || unresolved(&target) || is_assignable_to(&target, &source)
            }
            _ => true,
        }
    })
}

/// Primitives, their literals, `null`, `undefined` and `void`, alone or in a
/// union: the body-inferred returns surge relates here.
fn is_primitive_return(ty: &surge_ts_types::Type) -> bool {
    use surge_ts_types::Type;
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_primitive_return),
        _ => false,
    }
}

/// `const ok = typeof v === "string"; if (ok) …` narrows by the condition the
/// alias was written as, as a function body does (tsc's aliased-condition
/// narrowing). Module scope keeps no flow state to record the alias in, so the
/// `const` is looked up among the statements before the `if`.
pub(crate) fn expand_module_if_alias(
    statement: ParsedStatement,
    preceding: &[ParsedStatement],
) -> ParsedStatement {
    let ParsedStatement::If(mut if_statement) = statement else {
        return statement;
    };
    if let Some(expanded) = expand_module_alias_condition(&if_statement.condition, preceding) {
        let written = std::mem::replace(&mut if_statement.condition, expanded);
        if_statement.alias_source = Some(Box::new(written));
    }
    ParsedStatement::If(if_statement)
}

fn expand_module_alias_condition(
    condition: &surge_ts_syntax::ParsedExpression,
    preceding: &[ParsedStatement],
) -> Option<surge_ts_syntax::ParsedExpression> {
    use surge_ts_syntax::ParsedExpression;
    match condition {
        ParsedExpression::Identifier { name, .. } => module_alias_condition(name, preceding),
        ParsedExpression::Unary {
            operator: operator @ surge_ts_syntax::ParsedUnaryOperator::Not,
            operator_span,
            operand,
            operand_span,
        } => Some(ParsedExpression::Unary {
            operator: *operator,
            operator_span: *operator_span,
            operand: Box::new(expand_module_alias_condition(operand, preceding)?),
            operand_span: *operand_span,
        }),
        ParsedExpression::Logical {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
            ..
        } => {
            let expanded_left = expand_module_alias_condition(left, preceding);
            let expanded_right = expand_module_alias_condition(right, preceding);
            if expanded_left.is_none() && expanded_right.is_none() {
                return None;
            }
            Some(ParsedExpression::Logical {
                left: Box::new(expanded_left.unwrap_or_else(|| (**left).clone())),
                left_span: *left_span,
                operator: *operator,
                operator_span: *operator_span,
                right: Box::new(expanded_right.unwrap_or_else(|| (**right).clone())),
                right_span: *right_span,
                truthiness_tests: Vec::new(),
            })
        }
        _ => None,
    }
}

/// The condition the latest module declaration of `name` aliases, when it is a
/// local, unannotated `const` initialized with one. An exported one resolves to
/// its export symbol, which tsc does not inline.
fn module_alias_condition(
    name: &str,
    preceding: &[ParsedStatement],
) -> Option<surge_ts_syntax::ParsedExpression> {
    let variable = preceding.iter().rev().find_map(|statement| match statement {
        ParsedStatement::VariableDeclaration(variable) if variable.name == name => {
            Some(Some(variable))
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                ParsedStatement::VariableDeclaration(variable) if variable.name == name => {
                    Some(None)
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    })??;
    if !matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Const)
        || variable.is_declare
        || variable.declared_type.is_some()
    {
        return None;
    }
    variable
        .initializer
        .as_ref()
        .filter(|initializer| crate::checks::function::is_condition_shaped(initializer))
        .cloned()
}

/// Runs `check` with every narrowed module binding read at its declared type.
/// A function declaration is a flow container of its own (tsc extends the
/// flow only into function expressions and arrows): hoisted, it may run before
/// the narrowing assignment, guard or assertion, so its body sees neither a
/// binding's narrowing nor a narrowed member of it, `const` or not.
pub(crate) fn with_declared_mutable_module_bindings(
    ctx: &mut CheckerContext,
    check: impl FnOnce(&mut CheckerContext),
) {
    let narrowed: Vec<(std::sync::Arc<str>, crate::symbols::SymbolInfo, surge_ts_types::Type)> = ctx
        .symbols
        .narrowed_names()
        .filter_map(|name| {
            let symbol = ctx.symbols.get(name)?;
            let declared = ctx.symbols.declared_type(name)?;
            (*declared != symbol.ty)
                .then(|| (std::sync::Arc::clone(name), symbol.clone(), declared.clone()))
        })
        .collect();
    // Widened in place, not on a copy: the check records declaration spans on
    // the module table, which must survive it.
    for (name, symbol, declared) in &narrowed {
        let widened = crate::symbols::SymbolInfo {
            ty: declared.clone(),
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        let _ = ctx.symbols.insert(std::sync::Arc::clone(name), widened);
    }
    check(ctx);
    for (name, symbol, declared) in narrowed {
        let _ = ctx.symbols.insert_narrowed(name, symbol, declared);
    }
}

/// A module-scope assignment narrows the binding for the statements after it, as
/// the same assignment does in a block (`x ??= v` included, which lowers to
/// `x = x ?? v`). The assignment check itself is unchanged; the value's type is
/// read again without its diagnostics to decide the narrowing.
pub(crate) fn check_module_assignment(
    assignment: surge_ts_syntax::ParsedAssignment,
    ctx: &mut CheckerContext,
) {
    let _pattern = crate::infer::expression::enter_assignment_pattern_value(assignment.from_pattern);
    let target_name = assignment.target_name.clone();
    let value = assignment.value.clone();
    let value_span = assignment.value_span;
    let compound = crate::flow::is_compound_assignment(&target_name, assignment.target_span, &value);
    if assignment.from_pattern {
        let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        crate::checks::var::check_computed_pattern_key(&value, &symbols, ctx);
    }
    if !assign::check_assignment(assignment, ctx) {
        // tsc checks the value whatever the target turned out to be
        // (`checkBinaryLikeExpression`); a rejected target lends it no
        // contextual type, and nothing is narrowed by the write.
        let symbols = assign::rejected_write_symbols(&ctx.symbols, &target_name, compound);
        let _ = expr::evaluate_expression(&value, value_span, &symbols, ctx);
        return;
    }

    let Some(original) = ctx.symbols.get(&target_name) else {
        return;
    };
    if matches!(original.kind, crate::symbols::SymbolKind::Const) {
        return;
    }
    let original_ty = original.ty.clone();
    let original_kind = original.kind;
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = expr::evaluate_expression(&value, value_span, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);

    if evolving_arrays::assign_module_evolving_array(&target_name, &value, &inferred, ctx) {
        return;
    }
    // An auto-typed binding is whatever it was last assigned: a value surge
    // could not type leaves it unknown, not still `undefined`.
    let auto_typed = ctx
        .symbols
        .auto_array(&target_name)
        .is_some_and(|(binding, _)| binding.module_level && !binding.declared_array);
    if auto_typed && !matches!(&inferred, crate::infer::InferredExpression::Known(ty) if !ty.is_unmodelled()) {
        let narrowed = crate::symbols::SymbolInfo {
            ty: surge_ts_types::Type::Unknown,
            kind: original_kind,
            function_signature: None,
        };
        let _ = ctx.symbols.insert_narrowed(target_name, narrowed, surge_ts_types::Type::Any);
        return;
    }

    let mut scopes = crate::symbols::ScopeStack::from_root(symbols);
    if compound {
        check_function::widen_compound_assigned_symbol_type(&target_name, &mut scopes);
    } else {
        check_function::update_assigned_symbol_type(&target_name, inferred, &mut scopes);
    }
    let Some(updated) = scopes.resolve(&target_name) else {
        return;
    };
    if updated.ty == original_ty {
        return;
    }
    let updated = updated.clone();
    let declared = ctx
        .symbols
        .declared_type(&target_name)
        .cloned()
        .unwrap_or(original_ty);
    let _ = ctx.symbols.insert_narrowed(target_name, updated, declared);
}

/// An annotated module-scope declaration is read at what its initializer
/// narrowed the declared union to, for the statements after it, as a
/// module-scope assignment narrows (`check_module_assignment`).
fn narrow_module_declaration_by_initializer(
    name: String,
    initializer: &surge_ts_syntax::ParsedExpression,
    ctx: &mut CheckerContext,
) {
    let Some(symbol) = ctx.symbols.get(&name) else {
        return;
    };
    let declared = symbol.ty.clone();
    let kind = symbol.kind;
    let enum_declared =
        matches!(&declared, surge_ts_types::Type::Reference(reference) if reference.enum_owner.is_some());
    if !enum_declared && !matches!(declared, surge_ts_types::Type::Union(_) | surge_ts_types::Type::Boolean) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let Some(narrowed) =
        check_function::body_statements::declared_initializer_narrowing(&declared, initializer, &symbols, ctx)
    else {
        return;
    };
    if narrowed == declared {
        return;
    }
    let narrowed = crate::symbols::SymbolInfo {
        ty: narrowed,
        kind,
        function_signature: None,
    };
    let _ = ctx.symbols.insert_narrowed(name, narrowed, declared);
}

/// A module-scope call written as a statement, as the expression an assertion
/// signature is read from, when it could narrow an argument.
pub(crate) fn module_call_expression(
    call: &surge_ts_syntax::ParsedCall,
) -> Option<surge_ts_syntax::ParsedExpression> {
    assertion_arguments_named(&call.arguments).then(|| surge_ts_syntax::ParsedExpression::Call {
        callee_name: call.callee_name.clone(),
        callee_span: call.callee_span,
        type_arguments: call.type_arguments.clone(),
        type_argument_spans: call.type_argument_spans.clone(),
        arguments: call.arguments.clone(),
    })
}

pub(crate) fn assertion_candidate(expression: &surge_ts_syntax::ParsedExpression) -> bool {
    match expression {
        surge_ts_syntax::ParsedExpression::Call { arguments, .. }
        | surge_ts_syntax::ParsedExpression::PropertyCall { arguments, .. } => {
            assertion_arguments_named(arguments)
        }
        _ => false,
    }
}

fn assertion_arguments_named(arguments: &[surge_ts_syntax::ParsedCallArgument]) -> bool {
    arguments.iter().any(|argument| {
        matches!(argument.expression, surge_ts_syntax::ParsedExpression::Identifier { .. })
    })
}

/// `assertIsString(value);` narrows `value` for the rest of the module, as it
/// does for the rest of a block (tsc's assertion signatures). The narrowing runs
/// over a scope rooted at the module's symbols, and each argument binding it
/// changed is carried back.
pub(crate) fn narrow_module_assertion_call(
    expression: Option<surge_ts_syntax::ParsedExpression>,
    ctx: &mut CheckerContext,
) {
    let Some(expression) = expression else {
        return;
    };
    let arguments = match &expression {
        surge_ts_syntax::ParsedExpression::Call { arguments, .. }
        | surge_ts_syntax::ParsedExpression::PropertyCall { arguments, .. } => arguments,
        _ => return,
    };
    let names: Vec<String> = arguments
        .iter()
        .filter_map(|argument| match &argument.expression {
            surge_ts_syntax::ParsedExpression::Identifier { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut scopes = crate::symbols::ScopeStack::from_root(
        ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    crate::checks::function::narrow_assertion_call_in_scope(&expression, &mut scopes, ctx);
    for name in names {
        let (Some(narrowed), Some(original)) = (scopes.resolve(&name), ctx.symbols.get(&name))
        else {
            continue;
        };
        if narrowed.ty == original.ty {
            continue;
        }
        let narrowed = narrowed.clone();
        let declared = original.ty.clone();
        let _ = ctx.symbols.insert_narrowed(name, narrowed, declared);
    }
}

/// The assignments a module-scope expression statement makes inside it
/// (`d ?? (d = v)`, `a = b = c` written as an expression) take effect for the
/// statements after it, as they do in a block (`apply_expression_assignments`).
fn apply_module_expression_assignments(expression: &surge_ts_syntax::ParsedExpression, ctx: &mut CheckerContext) {
    let names: Vec<String> = crate::flow::expression_assignments(expression)
        .into_iter()
        .filter_map(|(assignment, _)| match assignment {
            surge_ts_syntax::ParsedExpression::Assignment { target_name, .. } => Some(target_name.clone()),
            _ => None,
        })
        .collect();
    if names.is_empty() {
        return;
    }
    let mut scopes = crate::symbols::ScopeStack::from_root(
        ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    let mut flow_state = crate::flow::FunctionFlowState::new(false);
    check_function::apply_expression_assignments(expression, &mut scopes, &mut flow_state, ctx);
    for name in names {
        let (Some(updated), Some(original)) = (scopes.resolve(&name), ctx.symbols.get(&name)) else {
            continue;
        };
        if updated.ty == original.ty || matches!(original.kind, crate::symbols::SymbolKind::Const) {
            continue;
        }
        let updated = updated.clone();
        let declared = ctx
            .symbols
            .declared_type(&name)
            .cloned()
            .unwrap_or_else(|| original.ty.clone());
        let _ = ctx.symbols.insert_narrowed(name, updated, declared);
    }
}

/// What a module-scope IIFE's body writes holds once the call returns (see
/// [`check_function::apply_immediately_invoked_assignments`]); each binding it
/// changed is carried back to the module's symbols.
fn apply_module_immediately_invoked_writes(
    writes: check_function::ImmediatelyInvokedAssignments,
    ctx: &mut CheckerContext,
) {
    let names = writes.target_names();
    let mut scopes = crate::symbols::ScopeStack::from_root(
        ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    let mut flow_state = crate::flow::FunctionFlowState::new(false);
    check_function::apply_immediately_invoked_assignments(writes, &mut scopes, &mut flow_state, ctx);
    for name in names {
        let (Some(updated), Some(original)) = (scopes.resolve(&name), ctx.symbols.get(&name)) else {
            continue;
        };
        if updated.ty == original.ty || matches!(original.kind, crate::symbols::SymbolKind::Const) {
            continue;
        }
        let updated = updated.clone();
        let declared = ctx
            .symbols
            .declared_type(&name)
            .cloned()
            .unwrap_or_else(|| original.ty.clone());
        let _ = ctx.symbols.insert_narrowed(name, updated, declared);
    }
}

/// A module-scope `if`: each branch is checked under its own narrowing, and
/// when one branch cannot fall through (`if (isCancel(value)) process.exit(0)`)
/// the statements after it see the other branch's narrowing, exactly as a
/// function body would.
pub(crate) fn check_module_if_statement(
    if_statement: &surge_ts_syntax::ParsedIfStatement,
    ctx: &mut CheckerContext,
) {
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let written = if_statement.alias_source.as_deref().unwrap_or(&if_statement.condition);
    crate::checks::function::report_unreferenced_callable_conditions(
        &if_statement.unreferenced_truthiness_tests,
        written,
        &symbols,
        ctx,
    );
    let condition = crate::checks::expr::evaluate_expression(
        written,
        if_statement.condition_span,
        &symbols,
        ctx,
    );
    crate::checks::expr::report_void_truthiness(&condition, if_statement.condition_span, ctx);
    let mut assigned = Vec::new();
    crate::checks::function::branch_assigned_names(&if_statement.then_body, &mut assigned);
    crate::checks::function::branch_assigned_names(&if_statement.else_body, &mut assigned);
    let mut branch_end_types = Vec::new();
    for (branch, branch_is_true) in [
        (&if_statement.then_body, true),
        (&if_statement.else_body, false),
    ] {
        let branch_symbols = narrowed_module_symbols(if_statement, branch_is_true, ctx)
            .unwrap_or_else(|| {
                ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext)
            });
        let end_types = if branch.is_empty() {
            assigned
                .iter()
                .map(|name| branch_symbols.get(name).map(|symbol| symbol.ty.clone()))
                .collect()
        } else {
            check_statements_over_module_scope(branch.clone(), branch_symbols, &assigned, ctx)
        };
        branch_end_types.push(end_types);
    }
    let scopes = crate::symbols::ScopeStack::from_root(symbols);
    let diverts = |body: &[surge_ts_syntax::ParsedFunctionBodyStatement]| {
        let flow = crate::flow::analyze_function_body_flow(body);
        flow.guarantees_value_return
            || flow.guarantees_exit
            || crate::checks::function::body_ends_in_never_call(body, &scopes, ctx)
    };
    let then_diverts = diverts(&if_statement.then_body);
    let else_diverts = !if_statement.else_body.is_empty() && diverts(&if_statement.else_body);
    if !then_diverts && !else_diverts {
        join_module_branch_assignments(&assigned, &branch_end_types, ctx);
    }
    let surviving_branch = match (then_diverts, else_diverts) {
        (true, false) => false,
        (false, true) => true,
        _ => return,
    };
    if let Some(narrowed) = narrowed_module_symbols(if_statement, surviving_branch, ctx)
    {
        ctx.symbols = narrowed;
    }
}

/// The module symbols as narrowed by `condition` holding (or not), when the
/// condition narrows anything.
fn narrowed_module_symbols(
    if_statement: &surge_ts_syntax::ParsedIfStatement,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) -> Option<crate::symbols::SymbolTable> {
    let base = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let by_source = if_statement
        .alias_source
        .as_deref()
        .and_then(|source| narrowed_module_symbols_over(source, branch_is_true, &base, ctx));
    narrowed_module_symbols_over(
        &if_statement.condition,
        branch_is_true,
        by_source.as_ref().unwrap_or(&base),
        ctx,
    )
    .or(by_source)
}

fn narrowed_module_symbols_over(
    condition: &surge_ts_syntax::ParsedExpression,
    branch_is_true: bool,
    base: &crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<crate::symbols::SymbolTable> {
    let base = base.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let narrowed =
        crate::checks::function::narrow_condition_symbol_table(condition, &base, branch_is_true);
    let narrowed = crate::checks::function::narrow_predicate_guards_symbol_table(
        condition,
        narrowed.as_ref().unwrap_or(&base),
        branch_is_true,
        ctx,
    )
    .or(narrowed);
    let narrowed = crate::checks::function::narrow_element_reference_guards_symbol_table(
        condition,
        branch_is_true,
        narrowed.as_ref().unwrap_or(&base),
        ctx,
    )
    .or(narrowed);
    // As in a function body, a guarded `unknown` stops reading as the `unknown`
    // keyword in the branch where its guard holds.
    crate::checks::expr::downgrade_guarded_genuine_unknown(
        condition,
        narrowed.as_ref().unwrap_or(&base),
        branch_is_true,
    )
    .or(narrowed)
}

/// A module-scope loop, block, `switch` or `try` is checked exactly as the
/// same statement in a function body, over a scope rooted at the module's
/// symbols.
pub(crate) fn check_module_block(
    statements: Vec<surge_ts_syntax::ParsedFunctionBodyStatement>,
    ctx: &mut CheckerContext,
) {
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let _ = check_statements_over_module_scope(statements, symbols, &[], ctx);
}

/// Checks `statements` over a scope rooted at `symbols` and answers the type
/// each of `names` ends with.
fn check_statements_over_module_scope(
    statements: Vec<surge_ts_syntax::ParsedFunctionBodyStatement>,
    symbols: crate::symbols::SymbolTable,
    names: &[String],
    ctx: &mut CheckerContext,
) -> Vec<Option<surge_ts_types::Type>> {
    let mut scopes = crate::symbols::ScopeStack::from_root(symbols);
    let mut var_names = Vec::new();
    crate::flow::collect_var_names(&statements, &mut var_names);
    // A `var` the block redeclares is the module's own binding — one symbol,
    // as tsc binds it — so the redeclaration is checked against it (TS2403).
    for name in &var_names {
        if let Some(existing) = ctx
            .symbols
            .get_own_handle(name)
            .filter(|existing| matches!(existing.kind, crate::symbols::SymbolKind::Var))
        {
            let _ = scopes.insert_current_handle(name.as_str(), existing);
        }
    }
    // The block is a scope of its own: a `const name` in it shadows the module
    // binding or the global of that name instead of redeclaring it. The frame
    // stays pushed so `names` still read what the block narrowed them to.
    scopes.push_child();
    let flow_facts = crate::flow::collect_function_flow_facts(&statements);
    let mut flow_state = crate::flow::FunctionFlowState::new(
        flow_facts.has_let_or_const || flow_facts.has_future_block_scoped_declarations,
    );
    // A binding the block narrows through an aliased condition is constant
    // only if nothing assigns it (`isConstantReference`); read off the block
    // itself, so a same-named `catch (e)` elsewhere in the module stays apart.
    flow_state.set_assigned_bindings(crate::flow::assigned_bindings(&statements));
    crate::checks::function::check_function_body(
        statements,
        None,
        &mut scopes,
        &mut flow_state,
        ctx,
    );
    // A `var` is function-scoped, so one declared in a module-level block,
    // branch or loop is a module binding once the statement has run.
    for name in var_names {
        if let Some(symbol) = scopes.visible_symbols().get(&name)
            && matches!(symbol.kind, crate::symbols::SymbolKind::Var)
            && ctx
                .symbols
                .get(&name)
                .is_none_or(|existing| matches!(existing.kind, crate::symbols::SymbolKind::Var))
        {
            let symbol = symbol.clone();
            let _ = ctx.symbols.insert(name, symbol);
        }
    }
    names
        .iter()
        .map(|name| scopes.resolve(name).map(|symbol| symbol.ty.clone()))
        .collect()
}

/// Both edges of a module-scope `if` reach the code after it, so a binding
/// either branch assigned is the union of what each edge left it as — bounded,
/// as in a function body, by its declaration.
fn join_module_branch_assignments(
    names: &[String],
    branch_end_types: &[Vec<Option<surge_ts_types::Type>>],
    ctx: &mut CheckerContext,
) {
    for (index, name) in names.iter().enumerate() {
        let Some(edges) = branch_end_types
            .iter()
            .map(|types| types.get(index).cloned().flatten())
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some(current) = ctx.symbols.get(name) else {
            continue;
        };
        let joined = surge_ts_types::union_type(edges);
        let current = current.clone();
        let declared = ctx.symbols.declared_type(name).cloned().unwrap_or(current.ty.clone());
        if joined == current.ty || !surge_ts_types::is_assignable_to(&joined, &declared) {
            continue;
        }
        let narrowed = crate::symbols::SymbolInfo {
            ty: joined,
            kind: current.kind,
            function_signature: current.function_signature.clone(),
        };
        let _ = ctx.symbols.insert_narrowed(name.clone(), narrowed, declared);
    }
}

fn with_module_export(ctx: &mut CheckerContext, check: impl FnOnce(&mut CheckerContext)) {
    ctx.module_export_depth += 1;
    check(ctx);
    ctx.module_export_depth -= 1;
}

fn with_module_declared_only(ctx: &mut CheckerContext, check: impl FnOnce(&mut CheckerContext)) {
    ctx.module_declared_only_depth += 1;
    check(ctx);
    ctx.module_declared_only_depth -= 1;
}

/// A module-level binding tsc types by control flow under `noImplicitAny`: a
/// `[]` one as an evolving array (`autoArrayType`), a non-`const` one with no
/// initializer (or an `undefined`/`null` one) as whatever is assigned
/// (`autoType`), starting from `undefined`. An exported one is not flow-typed:
/// other modules can write it, so tsc keeps its declared type.
fn module_auto_binding(
    variable: &surge_ts_syntax::ParsedVariableDeclaration,
    ctx: &CheckerContext,
) -> Option<(std::sync::Arc<str>, crate::symbols::AutoArrayBinding, Option<surge_ts_types::Type>)> {
    let name_span = variable.name_span?;
    if ctx.module_export_depth > 0 {
        return None;
    }
    let auto = evolving_arrays::auto_declaration(variable, ctx)?;
    let is_array = auto == evolving_arrays::AutoDeclaration::Array;
    let initial = (!is_array).then(|| match variable.initializer {
        Some(surge_ts_syntax::ParsedExpression::NullLiteral) => surge_ts_types::Type::Any,
        _ => surge_ts_types::Type::Undefined,
    });
    let is_let = matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Let);
    Some((
        variable.name.as_str().into(),
        crate::symbols::AutoArrayBinding {
            name_span: Some(name_span),
            is_const: matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Const),
            declared_array: is_array,
            is_let,
            initialized: is_array || variable.initializer.is_some(),
            assignments: is_let
                .then(|| ctx.let_assignment(name_span.start))
                .flatten(),
            evolving: is_array,
            elements: surge_ts_types::Type::Never,
            unsettled: false,
            pending_loops: 0,
            declared_only: false,
            module_level: true,
        },
        initial,
    ))
}

pub(crate) fn check_program_statement(
    statement: ParsedStatement,
    file_index: usize,
    statement_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    // Collected before the statement is consumed and applied once it is
    // checked, where tsc's flow places an array mutation.
    let mutations = if ctx.auto_arrays_declared && ctx.symbols.has_auto_arrays() {
        evolving_arrays::collect_module_mutations(&statement, &ctx.symbols)
    } else {
        Vec::new()
    };
    check_program_statement_itself(
        statement,
        file_index,
        statement_index,
        function_signatures,
        ctx,
    );
    if !mutations.is_empty() {
        evolving_arrays::apply_module_mutations(mutations, ctx);
    }
}

fn check_program_statement_itself(
    statement: ParsedStatement,
    file_index: usize,
    statement_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::VariableDeclaration(variable) => {
            let start = Instant::now();
            let auto_binding = module_auto_binding(&variable, ctx);
            let narrowing_initializer = (variable.declared_type.is_some()
                && !variable.is_declare
                && !variable.from_binding_pattern
                && !variable.is_enum_object)
            .then(|| variable.initializer.clone().map(|initializer| (variable.name.clone(), initializer)))
            .flatten();
            let enum_members = variable
                .enum_members
                .clone()
                .map(|bodies| (variable.name.clone(), bodies));
            let emitted_inferred = (variable.declared_type.is_none()
                && !variable.is_declare
                && !variable.is_enum_object
                && ((ctx.module_export_depth > 0 && crate::checks::declaration_emit::emits_declarations(ctx))
                    || crate::checks::declaration_emit::is_painted_variable(&variable.name, ctx)))
            .then(|| (variable.name.clone(), variable.name_span));
            // Emit reuses what a function literal's signature writes
            // (`tryReuseExistingTypeNode`): only what it leaves to inference is
            // serialized.
            let written_signature = match &variable.initializer {
                Some(surge_ts_syntax::ParsedExpression::ArrowFunction(arrow)) if emitted_inferred.is_some() => Some((
                    arrow.parameters.iter().map(|parameter| parameter.declared_type.is_some()).collect::<Vec<_>>(),
                    arrow.return_type.is_some(),
                )),
                _ => None,
            };
            var::check_variable_declaration(*variable, ctx);
            if let Some((name, span)) = emitted_inferred
                && let Some(ty) = ctx.symbols.get(&name).map(|symbol| symbol.ty.clone())
            {
                crate::checks::declaration_emit::report_unnameable_late_bound_keys(&ty, &name, span, ctx);
                let serialized = match (&ty, written_signature) {
                    (surge_ts_types::Type::Function(function), Some((annotated, return_annotated))) if function.overloads().is_none() => {
                        let mut inferred: Vec<surge_ts_types::Type> = function
                            .parameters()
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| !annotated.get(*index).copied().unwrap_or(false))
                            .map(|(_, parameter)| parameter.clone())
                            .collect();
                        if !return_annotated {
                            inferred.push(function.return_type().clone());
                        }
                        inferred
                    }
                    _ => vec![ty],
                };
                crate::checks::declaration_emit::report_inaccessible_names(
                    &serialized,
                    crate::checks::declaration_emit::EmitSite::Variable(&name),
                    span,
                    ctx,
                );
            }
            if let Some((name, initializer)) = narrowing_initializer {
                narrow_module_declaration_by_initializer(name, &initializer, ctx);
            }
            if let Some((name, bodies)) = enum_members {
                let symbols = ctx
                    .symbols
                    .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
                let top_level = ctx.namespace_member_prefix_stack.is_empty();
                crate::checks::enum_members::check_enum_members(
                    &name, &bodies, symbols, top_level, ctx,
                );
            }
            if let Some((name, binding, initial)) = auto_binding {
                ctx.auto_arrays_declared = true;
                if let Some(initial) = initial
                    && let Some(symbol) = ctx.symbols.get(&name).cloned()
                {
                    let narrowed = crate::symbols::SymbolInfo {
                        ty: initial,
                        kind: symbol.kind,
                        function_signature: None,
                    };
                    let _ = ctx.symbols.insert_narrowed(name.clone(), narrowed, surge_ts_types::Type::Any);
                }
                ctx.symbols.set_auto_array(name, Some(binding));
            }
            record_program_timing(ctx.timings.as_ref(), |timings| {
                timings.variable_declaration_checking += start.elapsed()
            });
        }
        ParsedStatement::Assignment(assignment) => {
            check_module_assignment(*assignment, ctx);
        }
        ParsedStatement::MemberAssignment(assignment) => {
            let symbols = ctx
                .symbols
                .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            let mut scopes = crate::symbols::ScopeStack::from_root(symbols);
            let receiver = match &assignment.target {
                surge_ts_syntax::ParsedExpression::PropertyAccess {
                    object,
                    property_name,
                    ..
                } => match object.as_ref() {
                    surge_ts_syntax::ParsedExpression::Identifier { name, .. } => {
                        Some((name.clone(), property_name.clone(), None))
                    }
                    // A write on an expando container member (`o.m.x = v`).
                    surge_ts_syntax::ParsedExpression::PropertyAccess {
                        object: inner,
                        property_name: member,
                        ..
                    } => match inner.as_ref() {
                        surge_ts_syntax::ParsedExpression::Identifier { name, .. }
                            if ctx.javascript_expando_objects.contains(&format!("{name}.{member}")) =>
                        {
                            Some((name.clone(), member.clone(), Some(property_name.clone())))
                        }
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };
            // A module-level write shares its container with the module's
            // declarations, which is where tsc's binder looks an expando
            // container up.
            let statement_span = match (assignment.target_span, assignment.value_span) {
                (Some(target), Some(value)) => Some(surge_ts_syntax::TextSpan {
                    start: target.start,
                    end: value.end,
                }),
                (target, _) => target,
            };
            let emitted_expando = match &receiver {
                Some((name, property_name, None))
                    if crate::checks::declaration_emit::emits_declarations(ctx)
                        && crate::checks::declaration_emit::is_exported_function(name) =>
                {
                    Some((name.clone(), property_name.clone()))
                }
                _ => None,
            };
            ctx.module_level_member_write = true;
            crate::checks::function::check_member_assignment(*assignment, &mut scopes, ctx);
            ctx.module_level_member_write = false;
            if let Some((name, property_name)) = emitted_expando
                && let Some(member) = scopes
                    .resolve(&name)
                    .and_then(|symbol| symbol.ty.get_property_access_type(&property_name))
            {
                crate::checks::declaration_emit::report_inaccessible_names(
                    std::slice::from_ref(&member),
                    crate::checks::declaration_emit::EmitSite::ExpandoProperty(&property_name),
                    statement_span,
                    ctx,
                );
            }
            // The scope stack is this statement's alone. An expando member the
            // write *declared* (`fn.x = v`) belongs to the binding, so it is
            // carried back as its type; what a write to an existing member
            // narrowed is carried as a narrowing, which the statements that
            // follow read and a function body does not.
            if let Some((name, property_name, nested_property)) = receiver
                && let Some(updated) = scopes.resolve(&name)
                && let Some(current) = ctx.symbols.get(&name)
                && updated.ty != current.ty
            {
                let declares_expando = match &nested_property {
                    Some(nested_property) => current
                        .ty
                        .get_property_access_type(&property_name)
                        .is_some_and(|container| container.get_property_access_type(nested_property).is_none()),
                    None => {
                        (matches!(
                            &updated.ty,
                            surge_ts_types::Type::Object(object) if object.call_signature().is_some()
                        ) || ctx.javascript_expando_objects.contains(&name))
                            && current.ty.get_property_access_type(&property_name).is_none()
                    }
                };
                let declared = ctx
                    .symbols
                    .declared_type(&name)
                    .cloned()
                    .unwrap_or_else(|| current.ty.clone());
                let updated = crate::symbols::SymbolInfo {
                    ty: updated.ty.clone(),
                    kind: updated.kind,
                    function_signature: updated.function_signature.clone(),
                };
                if declares_expando {
                    let _ = ctx.symbols.insert(name, updated);
                } else {
                    let _ = ctx.symbols.insert_narrowed(name, updated, declared);
                }
            }
        }
        ParsedStatement::FunctionDeclaration(function) => {
            rebind_type_query_return(&function, ctx);
            with_module_declared_only(ctx, |ctx| {
                with_declared_mutable_module_bindings(ctx, |ctx| {
                    let exported = ctx.module_export_depth > 0;
                    check_program_function_declaration(
                        *function,
                        exported,
                        file_index,
                        statement_index,
                        function_signatures,
                        ctx,
                    );
                });
            });
        }
        ParsedStatement::Call(call) => {
            let assertion = module_call_expression(&call);
            call::check_call(*call, ctx);
            narrow_module_assertion_call(assertion, ctx);
        }
        ParsedStatement::Expression(expression) => {
            let assertion = assertion_candidate(&expression).then(|| (*expression).clone());
            let define_property =
                (!ctx.javascript_expando_objects.is_empty()).then(|| (*expression).clone());
            let immediately_invoked = check_function::immediately_invoked_writes(&expression);
            let assigning = expression.contains_assignment().then(|| (*expression).clone());
            expr::check_expression_statement(*expression, ctx);
            if let Some(expression) = assigning {
                apply_module_expression_assignments(&expression, ctx);
            }
            narrow_module_assertion_call(assertion, ctx);
            if let Some(writes) = immediately_invoked {
                apply_module_immediately_invoked_writes(writes, ctx);
            }
            if let Some(expression) = define_property {
                declare_define_property_member(&expression, ctx);
            }
        }
        ParsedStatement::If(if_statement) => check_module_if_statement(&if_statement, ctx),
        ParsedStatement::Block(statements) => check_module_block(statements, ctx),
        ParsedStatement::TypeAliasDeclaration(alias) => {
            crate::checks::function::check_type_parameter_declarations(&alias.type_parameters, ctx);
            crate::checks::function::check_alias_mapped_type_constraints(&alias.type_parameters, &alias.ty, ctx);
            // A variance annotation on any other alias is TS2637, which the
            // grammar pass reports.
            if matches!(
                alias.ty,
                surge_ts_syntax::ParsedType::Object(_)
                    | surge_ts_syntax::ParsedType::Function(_)
                    | surge_ts_syntax::ParsedType::Mapped(_)
            ) {
                check_variance_annotations(&alias.name, &alias.type_parameters, ctx);
            }
            if let surge_ts_syntax::ParsedType::Object(object) = &alias.ty
                && alias.type_parameters.is_empty()
            {
                super::index_constraints::check_declared_keyed_index_conflicts(
                    object.keyed_indexes.as_deref(),
                    alias.ty.clone(),
                    ctx,
                );
            }
            if let surge_ts_syntax::ParsedType::Object(object) = &alias.ty {
                check_self_indexed_member_annotations(
                    &alias.name,
                    object.properties.iter().map(|property| (property.name.as_str(), property.name_span, &property.ty)),
                    ctx,
                );
            }
        }
        ParsedStatement::InterfaceDeclaration(interface) => {
            crate::checks::function::check_type_parameter_declarations(
                &interface.type_parameters,
                ctx,
            );
            super::heritage::report_circular_base(
                &interface.name,
                interface.name_span,
                &interface.type_parameters,
                false,
                ctx,
            );
            super::heritage::check_interface_heritage(&interface, ctx);
            super::index_constraints::check_interface_index_constraints(&interface, ctx);
            if interface.type_parameters.is_empty() {
                super::index_constraints::check_declared_keyed_index_conflicts(
                    interface.keyed_indexes.as_deref(),
                    surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
                        type_argument_spans: Vec::new(),
                        name: interface.name.clone(),
                        span: None,
                        type_arguments: Vec::new(),
                    })),
                    ctx,
                );
            }
            check_variance_annotations(&interface.name, &interface.type_parameters, ctx);
            if interface.type_parameters.is_empty() {
                report_merged_type_parameter_names(&interface, ctx);
            }
            check_self_indexed_member_annotations(
                &interface.name,
                interface.members.iter().map(|member| (member.name.as_str(), member.name_span, &member.ty)),
                ctx,
            );
            super::late_bound_members::check_interface_late_bound_members(&interface.members, ctx);
            super::late_bound_members::check_interface_computed_key_types(&interface.members, ctx);
            check_interface_setter_parameter_types(&interface, ctx);
        }
        ParsedStatement::ClassDeclaration(class) => {
            with_module_declared_only(ctx, |ctx| super::check_class_declaration(&class, ctx));
            if ctx.module_export_depth > 0 {
                report_emitted_class_base(&class, class.name_span, ctx);
            }
            rebind_late_bound_statics(&class, ctx);
        }
        ParsedStatement::ImportDeclaration(import) => {
            report_type_declaration_import(&import.module_specifier, import.module_specifier_span, ctx);
            check_entity_alias_local_conflict(&import, ctx);
            check_entity_alias_head(&import, ctx)
        }
        ParsedStatement::ExportDeclaration(export) => match *export {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                with_module_export(ctx, |ctx| {
                    check_program_statement(
                        *declaration,
                        file_index,
                        statement_index,
                        function_signatures,
                        ctx,
                    );
                });
            }
            ParsedExportDeclaration::Named { .. } => {}
            ParsedExportDeclaration::Namespace { .. } => {}
            ParsedExportDeclaration::Default { declaration, span } => match declaration {
                ParsedDefaultExportDeclaration::Function(function) => {
                    with_module_declared_only(ctx, |ctx| {
                        check_program_function_declaration(
                            function,
                            true,
                            file_index,
                            statement_index,
                            function_signatures,
                            ctx,
                        );
                    });
                }
                ParsedDefaultExportDeclaration::Expression(expression) => {
                    if !report_export_assignment_in_namespace(false, span, ctx) {
                        let emitted = crate::checks::declaration_emit::emits_declarations(ctx).then(|| expression.clone());
                        check_export_assignment_expression(expression, ctx);
                        if let Some(expression) = emitted {
                            crate::checks::declaration_emit::report_expression_class_literal_members(&expression, span, ctx);
                            crate::checks::declaration_emit::report_default_export_names(&expression, span, ctx);
                        }
                    }
                }
                ParsedDefaultExportDeclaration::Class(class) => {
                    with_module_declared_only(ctx, |ctx| {
                        super::check_class_declaration(&class, ctx);
                    });
                    // An anonymous class (bound as `default`) is reported where
                    // its declaration starts.
                    let report_span = if class.name == "default" { span } else { class.name_span };
                    report_emitted_class_base(&class, report_span, ctx);
                }
                ParsedDefaultExportDeclaration::Unsupported { span } => {
                    let mut diagnostic =
                        Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                    if let Some(span) = span {
                        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                    }

                    ctx.push(diagnostic);
                }
            },
            ParsedExportDeclaration::All { .. } => {}
            ParsedExportDeclaration::Empty { .. } => {}
            ParsedExportDeclaration::Equals {
                exported_name,
                exported_name_span,
                span,
            } => {
                if !report_export_assignment_in_namespace(true, span, ctx) {
                    check_export_assignment_expression(
                        surge_ts_syntax::ParsedExpression::Identifier {
                            name: exported_name,
                            span: exported_name_span,
                        },
                        ctx,
                    );
                }
            }
            ParsedExportDeclaration::EqualsExpression {
                expression, span, ..
            } => {
                if !report_export_assignment_in_namespace(true, span, ctx) {
                    check_export_assignment_expression(*expression, ctx);
                }
            }
            ParsedExportDeclaration::NamespaceExport { .. } => {}
            ParsedExportDeclaration::Unsupported { span } => {
                let mut diagnostic =
                    Diagnostic::surge_unsupported_module_syntax(ctx.file_name.clone());

                if let Some(span) = span {
                    diagnostic = diagnostic.with_span(crate::context::convert_span(span));
                }

                ctx.push(diagnostic);
            }
        },
        ParsedStatement::DeclareModuleDeclaration(module) => {
            report_unresolved_ambient_module_export_equals(&module, ctx);
            // A `declare global` block's namespaces are checked as any other
            // namespace is (`checkModuleDeclaration` over the global scope).
            if module.module_specifier == "global" {
                for statement in &module.statements {
                    if let ParsedStatement::NamespaceDeclaration(namespace) = statement {
                        check_namespace_body(namespace, file_index, ctx);
                    }
                }
            }
        }
        ParsedStatement::NamespaceDeclaration(namespace) => {
            check_namespace_body(&namespace, file_index, ctx);
        }
        ParsedStatement::UnsupportedDeclaration { span } => {
            emit_unsupported_declaration_diagnostic(ctx, span);
        }
    }
}

/// tsc's `checkModuleDeclaration` checks the body as ordinary source
/// elements, in a scope of its own: the body's values (exported or not)
/// shadow the enclosing ones, and its bare type references resolve against the
/// namespace's qualified members.
///
/// The scope mirrors a module file's: hoisted function declarations are bound
/// up front, and the body's other values back later references through the
/// module value fallback, together with what other blocks of the same
/// namespace export.
fn check_namespace_body(
    namespace: &surge_ts_syntax::ParsedNamespaceDeclaration,
    file_index: usize,
    ctx: &mut CheckerContext,
) {
    let ambient_body;
    let namespace = if namespace.is_declare {
        ambient_body = ambient_namespace(namespace);
        &ambient_body
    } else {
        namespace
    };
    let prefix = match ctx.namespace_member_prefix_stack.last() {
        Some(outer) => format!("{outer}.{}", namespace.member_name()),
        None => namespace.name.clone(),
    };
    let block_prefix = crate::driver::namespace_block_prefix(&prefix, &ctx.file_name, namespace);
    ctx.namespace_member_prefix_stack.push(prefix);
    ctx.namespace_block_prefix_stack.push(block_prefix);

    let enclosing = std::sync::Arc::new(
        ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    let type_declarations = ctx.type_declarations.clone();
    let namespace_values = crate::modules::collect_exportable_value_symbols(
        &namespace.statements,
        &type_declarations,
        &enclosing,
        None,
        false,
        ctx,
    );
    let mut body_values = crate::symbols::SymbolTable::new();
    for (name, symbol) in namespace_values.iter_shared() {
        let inherited = enclosing
            .get_handle(name)
            .is_some_and(|outer| std::sync::Arc::ptr_eq(&outer, symbol));
        if !inherited {
            let _ = body_values.insert_shared(name.clone(), symbol.clone());
        }
    }
    // `namespace A.B {}` is `A` holding a block named `A.B`, so the merged `B`
    // is the member the enclosing block took from the merged `A`, not a name
    // of the scope around it.
    let merged = if namespace.name.contains('.') {
        ctx.module_value_fallback
            .as_ref()
            .and_then(|fallback| fallback.get_own_shared(namespace.member_name()))
    } else {
        enclosing.get_handle(&namespace.name).or_else(|| {
            ctx.module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get_handle(&namespace.name))
        })
    };
    let merged_object = merged.and_then(|merged| match &merged.ty {
        surge_ts_types::Type::Object(object) => Some(object.clone()),
        _ => None,
    });
    if let Some(object) = &merged_object {
        for (name, property) in object.properties.iter() {
            let own = body_values.get_own(name).map(|symbol| symbol.ty.clone());
            // Another block's export, or this block's nested namespace, which
            // the merged object carries with every block's members.
            if own.is_none() || matches!(own, Some(surge_ts_types::Type::Object(_))) {
                let _ = body_values.insert(
                    name.to_string(),
                    crate::symbols::SymbolInfo {
                        ty: property.ty.clone(),
                        kind: crate::symbols::SymbolKind::Var,
                        function_signature: None,
                    },
                );
            }
        }
    }
    bind_namespace_require_aliases(&namespace.statements, &mut body_values, ctx);
    let saved_fallback = ctx.module_value_fallback.take();
    let body_values = match saved_fallback.clone() {
        Some(outer) => body_values.with_parent_fallback(outer),
        None => body_values,
    };
    ctx.module_value_fallback = Some(std::sync::Arc::new(body_values));

    let mut symbols = crate::symbols::SymbolTable::declaration_scope(enclosing);
    let mut function_signatures = HashMap::new();
    collect_function_signatures_from_statements(
        &namespace.statements,
        file_index,
        &mut symbols,
        &mut function_signatures,
        ctx,
    );
    // A function this block declares is one symbol with a same-named
    // namespace another block of the namespace exports, so it carries that
    // namespace's members here too.
    if let Some(object) = &merged_object {
        for (name, property) in object.properties.iter() {
            let merged_function = matches!(&property.ty, surge_ts_types::Type::Object(member) if member.call_signature.is_some())
                && symbols
                    .get_own(name)
                    .is_some_and(|symbol| matches!(symbol.ty, surge_ts_types::Type::Function(_)));
            if merged_function && let Some(symbol) = symbols.get_own_shared(name) {
                let _ = symbols.insert(
                    name.to_string(),
                    crate::symbols::SymbolInfo {
                        ty: property.ty.clone(),
                        kind: symbol.kind,
                        function_signature: symbol.function_signature.clone(),
                    },
                );
            }
        }
    }
    let saved_symbols = std::mem::take(&mut ctx.symbols);
    ctx.set_symbols(symbols);
    if let Some(prefix) = ctx.namespace_member_prefix_stack.last().cloned() {
        let file_name = ctx.file_name.clone();
        crate::driver::validate_namespace_type_declarations(
            &namespace.statements,
            &prefix,
            &file_name,
            ctx,
        );
    }
    check_program_file_statements(&namespace.statements, file_index, &function_signatures, &[], ctx);
    ctx.set_symbols(saved_symbols);
    ctx.module_value_fallback = saved_fallback;
    ctx.namespace_member_prefix_stack.pop();
    ctx.namespace_block_prefix_stack.pop();
}

/// tsc never collects a namespace's `import x = require("m")` (it is TS1147),
/// so resolving the alias finds only an ambient `declare module "m"`
/// (`tryFindAmbientModule`) and otherwise reports the module unresolved at the
/// specifier — once something names the alias, which is when tsc resolves it.
/// The alias is declared either way, error-typed when unresolved.
fn bind_namespace_require_aliases(
    statements: &[ParsedStatement],
    body_values: &mut crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    for statement in statements {
        let import = match statement {
            ParsedStatement::ImportDeclaration(import) => import,
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                    ParsedStatement::ImportDeclaration(import) => import,
                    _ => continue,
                },
                _ => continue,
            },
            _ => continue,
        };
        let surge_ts_syntax::ParsedImportKind::Equals { local_name, .. } = &import.kind else {
            continue;
        };
        if body_values.get_own(local_name).is_some() {
            continue;
        }
        let symbol = crate::modules::ambient_module_export_table(ctx, &import.module_specifier).map(
            |table| match table.export_assignment_symbol.clone() {
                Some(symbol) => (*symbol).clone(),
                None => crate::symbols::SymbolInfo {
                    ty: crate::modules::namespace_export_object_type(table),
                    kind: crate::symbols::SymbolKind::Var,
                    function_signature: None,
                },
            },
        );
        match symbol {
            Some(symbol) => {
                let _ = body_values.insert(local_name.clone(), symbol);
            }
            None => {
                if ctx
                    .namespace_require_reads
                    .as_ref()
                    .is_some_and(|reads| reads.contains(local_name.as_str()))
                {
                    crate::modules::emit_unresolvable_module_reference(
                        ctx,
                        &import.module_specifier,
                        import.module_specifier_span.or(import.span),
                    );
                }
                crate::modules::insert_error_typed_value_import(local_name, body_values);
            }
        }
    }
}

/// The file's referenced names, when some namespace in it holds an
/// `import x = require()` (see [`bind_namespace_require_aliases`]).
pub(crate) fn namespace_require_reads(
    statements: &[ParsedStatement],
    module_reads: &[String],
) -> Option<std::sync::Arc<surge_ts_types::fx::FxHashSet<String>>> {
    fn has_namespace_require(statements: &[ParsedStatement], in_namespace: bool) -> bool {
        statements.iter().any(|statement| {
            let statement = match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                    _ => return false,
                },
                other => other,
            };
            match statement {
                ParsedStatement::NamespaceDeclaration(namespace) => {
                    has_namespace_require(&namespace.statements, true)
                }
                ParsedStatement::ImportDeclaration(import) => {
                    in_namespace && matches!(import.kind, surge_ts_syntax::ParsedImportKind::Equals { .. })
                }
                _ => false,
            }
        })
    }
    has_namespace_require(statements, false)
        .then(|| std::sync::Arc::new(module_reads.iter().cloned().collect()))
}

/// A `declare namespace` makes every declaration in it ambient, nested
/// namespaces and classes included (a `declare class` has no initializers to
/// check).
fn ambient_namespace(
    namespace: &surge_ts_syntax::ParsedNamespaceDeclaration,
) -> surge_ts_syntax::ParsedNamespaceDeclaration {
    let mut ambient = namespace.clone();
    ambient.statements.iter_mut().for_each(ambient_statement);
    ambient
}

fn ambient_statement(statement: &mut ParsedStatement) {
    match statement {
        ParsedStatement::ClassDeclaration(class) => class.is_declare = true,
        ParsedStatement::NamespaceDeclaration(inner) => {
            inner.is_declare = true;
            inner.statements.iter_mut().for_each(ambient_statement);
        }
        ParsedStatement::ExportDeclaration(export) => {
            if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_mut() {
                ambient_statement(declaration);
            }
        }
        _ => {}
    }
}

/// Everything a declaration file declares is ambient whether or not it is
/// written `declare`, so its top-level variables, functions and classes are
/// checked as `declare` ones are, and its namespaces as `declare namespace`.
pub(crate) fn make_declaration_file_ambient(statements: &mut [ParsedStatement]) {
    for statement in statements {
        match statement {
            ParsedStatement::VariableDeclaration(variable) => variable.is_declare = true,
            ParsedStatement::FunctionDeclaration(function) => function.is_declare = true,
            ParsedStatement::ExportDeclaration(export) => match export.as_mut() {
                ParsedExportDeclaration::Statement { declaration, .. } => {
                    make_declaration_file_ambient(std::slice::from_mut(declaration.as_mut()));
                }
                ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Class(class),
                    ..
                } => class.is_declare = true,
                ParsedExportDeclaration::Default {
                    declaration: ParsedDefaultExportDeclaration::Function(function),
                    ..
                } => function.is_declare = true,
                _ => {}
            },
            other => ambient_statement(other),
        }
    }
}

/// tsc's `checkExportAssignment`: an export assignment directly in a namespace
/// body is TS1063 (`export =`) or TS1319 (`export default`), and nothing else
/// about it is checked, its expression included.
fn report_export_assignment_in_namespace(
    is_export_equals: bool,
    span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if ctx.namespace_member_prefix_stack.is_empty() {
        return false;
    }
    let diagnostic = if is_export_equals {
        Diagnostic::ts1063(ctx.file_name.clone())
    } else {
        Diagnostic::ts1319(ctx.file_name.clone())
    };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
    true
}

/// The target of `export =` / `export default <expression>`, which tsc checks
/// as an ordinary expression except that its root name may name a type or a
/// namespace.
fn check_export_assignment_expression(
    expression: surge_ts_syntax::ParsedExpression,
    ctx: &mut CheckerContext,
) {
    if crate::checks::expr::export_assignment_target_is_exempt(&expression, ctx) {
        return;
    }
    expr::check_expression_statement(expression, ctx);
}

pub(crate) fn emit_unsupported_declaration_diagnostics(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    for statement in statements {
        emit_unsupported_declaration_diagnostic_from_statement(statement, ctx);
    }
}

pub(crate) fn emit_unsupported_declaration_diagnostic_from_statement(
    statement: &ParsedStatement,
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::UnsupportedDeclaration { span } => {
            emit_unsupported_declaration_diagnostic(ctx, *span);
        }
        ParsedStatement::ImportDeclaration(import)
            if matches!(
                import.kind,
                ParsedImportKind::Unsupported | ParsedImportKind::TypeOnlyDefault { .. }
            ) =>
        {
            emit_unsupported_declaration_diagnostic(
                ctx,
                import.span.or(import.module_specifier_span),
            );
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Unsupported { span }
            | ParsedExportDeclaration::EqualsExpression { span, .. } => {
                emit_unsupported_declaration_diagnostic(ctx, *span);
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Unsupported { span },
                span: declaration_span,
            } => {
                emit_unsupported_declaration_diagnostic(ctx, (*span).or(*declaration_span));
            }
            _ => {}
        },
        ParsedStatement::DeclareModuleDeclaration(module) => {
            emit_unsupported_declaration_diagnostics(&module.statements, ctx);
        }
        _ => {}
    }
}

pub(crate) fn emit_unsupported_declaration_diagnostic(
    ctx: &mut CheckerContext,
    span: Option<TextSpan>,
) {
    let mut diagnostic = Diagnostic::surge_unsupported_declaration(ctx.file_name.clone());

    if let Some(span) = span {
        diagnostic = diagnostic.with_span(crate::context::convert_span(span));
    }

    ctx.push(diagnostic);
}

pub(crate) fn check_program_function_declaration(
    function: ParsedFunctionDeclaration,
    exported: bool,
    file_index: usize,
    statement_index: usize,
    function_signatures: &HashMap<FunctionDeclarationLocation, FunctionType>,
    ctx: &mut CheckerContext,
) {
    let declaration_location = FunctionDeclarationLocation {
        file_index,
        statement_index,
    };
    // Declaration emit writes the return type this body infers.
    let emitted_return_span = exported
        .then(|| crate::checks::declaration_emit::inferred_return_span(&function, ctx))
        .flatten();

    let saved_symbols = std::mem::take(&mut ctx.symbols);
    let body_root_symbols =
        saved_symbols.clone_with_reason(surge_ts_types::TypeCopyReason::FunctionBodySetup);
    ctx.symbols = body_root_symbols;
    let Some(function_type) = function_signatures.get(&declaration_location) else {
        let captured = capturing_returns(emitted_return_span.is_some(), || {
            check_function::check_function_declaration(function, ctx)
        });
        ctx.symbols = saved_symbols;
        report_emitted_return(captured, emitted_return_span, ctx);
        return;
    };

    check_function::report_named_full_signature(function.full_signature.as_ref(), ctx);
    check_function::report_signature_qualified_names(&function, ctx);
    let type_parameters = function.type_parameters.clone();
    check_function::check_type_parameter_declarations(&type_parameters, ctx);
    let circular_candidate = check_function::circular_body_return_candidate(&function, ctx);
    let circular = check_function::returns_reading_itself(&function);
    let name = function.name.clone();
    let captured = capturing_returns(emitted_return_span.is_some(), || {
        check_function::with_circular_body(&name, circular, || {
            check_function::check_function_declaration_body(function, function_type, &type_parameters, ctx)
        })
    });
    ctx.symbols = saved_symbols;
    check_function::report_circular_body_return(circular_candidate, ctx);
    report_emitted_return(captured, emitted_return_span, ctx);
}

fn capturing_returns(capture: bool, check: impl FnOnce()) -> Option<check_function::CapturedReturns> {
    if capture {
        return check_function::capture_declaration_body_returns(check).1;
    }
    check();
    None
}

/// Signatures are collected before the module's values are bound, so a return
/// written `typeof x` over a value inferred from a call
/// (`const all = make(() => …)`) degraded there. tsc resolves it lazily, at
/// first use; read again here, it is the value's type.
fn rebind_type_query_return(function: &ParsedFunctionDeclaration, ctx: &mut CheckerContext) {
    let Some(return_type) = function.return_type.as_ref() else {
        return;
    };
    if !function.type_parameters.is_empty()
        || function.full_signature.is_some()
        || !crate::modules::annotation_contains_typeof(return_type)
    {
        return;
    }
    let Some(symbol) = ctx.symbols.get(&function.name) else {
        return;
    };
    let surge_ts_types::Type::Function(bound) = &symbol.ty else {
        return;
    };
    if bound.overloads().is_some() || !crate::checks::function::type_contains_degradation(bound.return_type()) {
        return;
    }
    let (bound, kind, function_signature) = (bound.clone(), symbol.kind, symbol.function_signature.clone());
    let diagnostics_before = ctx.diagnostics().len();
    let fresh = crate::infer::map_parsed_type(return_type.clone(), ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    if crate::checks::function::type_contains_degradation(&fresh) {
        return;
    }
    ctx.symbols.insert(
        function.name.clone(),
        crate::symbols::SymbolInfo {
            ty: surge_ts_types::Type::Function(bound.with_signature_types(bound.parameters().to_vec(), fresh)),
            kind,
            function_signature,
        },
    );
}

/// The class value was bound before the module's values were: a static keyed
/// by another of them (`static [s2]: boolean` with `s2: typeof s0`) kept its
/// written name. Read again now, it binds the member its key names.
fn rebind_late_bound_statics(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    let late_bound = |name: &str, is_static: bool| is_static && name.starts_with('[') && !name.starts_with("[Symbol.");
    let has_late_bound_static = class.members.iter().any(|member| match member {
        surge_ts_syntax::ParsedClassMember::Property(property) => late_bound(&property.name, property.is_static),
        surge_ts_syntax::ParsedClassMember::Method(method) => late_bound(&method.name, method.is_static),
        _ => false,
    });
    if !has_late_bound_static || ctx.symbols.get(&class.name).is_none() {
        return;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let mut symbol = super::build_class_value_symbol(class, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    // A namespace merged into the class (`namespace Sql { class Aliased }`)
    // put its members on the bound value; the rebuilt one has only the class's.
    if let (Some(previous), surge_ts_types::Type::Object(rebuilt)) = (ctx.symbols.get(&class.name), &mut symbol.ty)
        && let surge_ts_types::Type::Object(previous) = &previous.ty
    {
        let missing: Vec<_> = previous
            .properties
            .iter()
            .filter(|(name, _)| !name.starts_with('[') && !rebuilt.properties.contains_key(*name))
            .map(|(name, property)| (name.clone(), property.clone()))
            .collect();
        if !missing.is_empty() {
            let mut properties = (*rebuilt.properties).clone();
            properties.extend(missing);
            rebuilt.properties = std::sync::Arc::new(properties);
            rebuilt.property_map_id = None;
        }
    }
    ctx.symbols.insert(class.name.clone(), symbol);
}

/// Declaration emit writes an `extends` clause that is no entity name as a
/// `_base` variable typed by the expression.
fn report_emitted_class_base(
    class: &ParsedClassDeclaration,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    if let Some(expression) = class.heritage_expression.as_deref()
        && crate::checks::declaration_emit::emits_declarations(ctx)
    {
        crate::checks::declaration_emit::report_expression_class_literal_members(expression, span, ctx);
    }
}

fn report_emitted_return(
    captured: Option<check_function::CapturedReturns>,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    if let Some(captured) = captured {
        crate::checks::declaration_emit::report_class_literal_members(&captured.returned, span, ctx);
        crate::checks::declaration_emit::report_inaccessible_names(
            &captured.returned,
            crate::checks::declaration_emit::EmitSite::FunctionReturn,
            span,
            ctx,
        );
    }
}

/// A declaration of a merged interface that declares no type parameters does
/// not see another declaration's: its members naming one name nothing
/// (TS2304), unless an outer type of that name answers.
fn report_merged_type_parameter_names(interface: &surge_ts_syntax::ParsedInterfaceDeclaration, ctx: &mut CheckerContext) {
    let Some(crate::symbols::TypeDeclarationInfo::Interface(merged)) = ctx.lookup_type_declaration(&interface.name)
    else {
        return;
    };
    let foreign: Vec<String> = merged
        .body
        .type_parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .filter(|name| ctx.lookup_type_declaration(name).is_none())
        .collect();
    if foreign.is_empty() {
        return;
    }
    let mut unresolved = Vec::new();
    let mut visit = |named: &surge_ts_syntax::ParsedNamedType| {
        if named.type_arguments.is_empty()
            && foreign.contains(&named.name)
            && let Some(span) = named.span
        {
            unresolved.push((named.name.clone(), span));
        }
    };
    let written = interface
        .members
        .iter()
        .map(|member| &member.ty)
        .chain(interface.string_index_type.iter())
        .chain(interface.number_index_type.iter());
    for ty in written.filter(|ty| !introduces_type_names(ty)) {
        ty.for_each_named_type(&mut visit);
    }
    for (name, span) in unresolved {
        let diagnostic = Diagnostic::ts2304(&name, ctx.file_name.clone()).with_span(crate::context::convert_span(span));
        ctx.push_utility_diagnostic_once(diagnostic);
    }
}

/// Whether a written type declares type names of its own somewhere (a generic
/// signature, a mapped type's key, a conditional's `infer`), which would
/// shadow an outer name.
fn introduces_type_names(ty: &surge_ts_syntax::ParsedType) -> bool {
    use surge_ts_syntax::ParsedType;
    let signature = |function: &surge_ts_syntax::ParsedFunctionType| {
        !function.type_parameters.is_empty()
            || function.parameters.iter().any(|parameter| introduces_type_names(&parameter.ty))
            || introduces_type_names(&function.return_type)
    };
    match ty {
        ParsedType::Mapped(_) | ParsedType::Conditional(_) | ParsedType::Infer(_) => true,
        ParsedType::Function(function) => signature(function),
        ParsedType::Object(object) => {
            object.properties.iter().any(|property| introduces_type_names(&property.ty))
                || object.call_signature.as_deref().is_some_and(signature)
                || object.construct_signature.as_deref().is_some_and(signature)
        }
        ParsedType::Named(named) => named.type_arguments.iter().any(introduces_type_names),
        ParsedType::Array(element) | ParsedType::Readonly(element) | ParsedType::KeyOf(element) => {
            introduces_type_names(element)
        }
        ParsedType::Tuple(elements, ..) | ParsedType::Union(elements) | ParsedType::Intersection(elements) => {
            elements.iter().any(introduces_type_names)
        }
        ParsedType::IndexedAccess(access) => {
            introduces_type_names(&access.object_type) || introduces_type_names(&access.index_type)
        }
        _ => false,
    }
}

pub(crate) fn count_local_type_declarations_in_statements(statements: &[ParsedStatement]) -> usize {
    statements
        .iter()
        .map(count_local_type_declarations_in_statement)
        .sum()
}

pub(crate) fn count_local_type_declarations_in_statement(statement: &ParsedStatement) -> usize {
    match statement {
        ParsedStatement::TypeAliasDeclaration(_) => 1,
        ParsedStatement::InterfaceDeclaration(_) => 1,
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                count_local_type_declarations_in_statement(declaration.as_ref())
            }
            _ => 0,
        },
        _ => 0,
    }
}

/// `Object.defineProperty(o, "name", descriptor)` on a JavaScript expando
/// object declares `name` on it for the statements that follow.
fn declare_define_property_member(expression: &surge_ts_syntax::ParsedExpression, ctx: &mut CheckerContext) {
    let Some((name, member, descriptor)) = crate::modules::exports::define_property_declaration(expression) else {
        return;
    };
    if !ctx.javascript_expando_objects.contains(name) {
        return;
    }
    let Some(current) = ctx.symbols.get(name).cloned() else {
        return;
    };
    let surge_ts_types::Type::Object(object) = &current.ty else {
        return;
    };
    let symbols = ctx.symbols.clone();
    let Some((member_type, readonly)) = crate::modules::exports::expando_member_type(
        &crate::modules::exports::ExpandoValue::Descriptor(descriptor),
        &symbols,
        ctx,
    ) else {
        return;
    };
    let mut properties = (*object.properties).clone();
    properties.insert(
        member.into(),
        surge_ts_types::ObjectProperty::required(member_type).with_readonly(readonly),
    );
    let _ = ctx.symbols.insert(
        name.to_string(),
        crate::symbols::SymbolInfo {
            ty: surge_ts_types::Type::Object(crate::metrics::alloc_object_type(properties, None)),
            kind: current.kind,
            function_signature: current.function_signature.clone(),
        },
    );
}

/// tsc's `checkTypeParameterDeferred`: a declaration whose type parameter is
/// written `in` or `out` (not both) must relate as that variance says,
/// instantiated with a marker subtype and supertype for the parameter
/// (TS2636). surge's markers are a string literal and `string`, so a
/// parameter with a constraint they may not meet is left unchecked.
/// TS2502 for a member annotated as its own declaration indexed by its own
/// name (`x: T["x"]`, `x: T<U>["x"]`): resolving the annotation needs the
/// member's type first (`getTypeOfVariableOrParameterOrProperty`).
/// `checkAccessorDeclaration`: a `set` accessor's parameter type is checked
/// like any written type, read or not. The interface's own types resolve
/// lazily and leave it out.
fn check_interface_setter_parameter_types(
    interface: &surge_ts_syntax::ParsedInterfaceDeclaration,
    ctx: &mut CheckerContext,
) {
    let write_types: Vec<&surge_ts_syntax::ParsedType> =
        interface.members.iter().filter_map(|member| member.write_ty.as_ref()).collect();
    if write_types.is_empty() {
        return;
    }
    crate::checks::function::with_type_parameter_scope(&interface.type_parameters, ctx, |ctx| {
        for write_type in write_types {
            let _ = crate::infer::map_parsed_type(write_type.clone(), ctx);
        }
    });
}

pub(crate) fn check_self_indexed_member_annotations<'a>(
    declaration: &str,
    members: impl IntoIterator<Item = (&'a str, Option<surge_ts_syntax::TextSpan>, &'a surge_ts_syntax::ParsedType)>,
    ctx: &mut CheckerContext,
) {
    for (name, name_span, ty) in members {
        let surge_ts_syntax::ParsedType::IndexedAccess(access) = ty else {
            continue;
        };
        let names_declaration = matches!(access.object_type.as_ref(), surge_ts_syntax::ParsedType::Named(object)
            if object.name == declaration);
        let names_member = matches!(access.index_type.as_ref(), surge_ts_syntax::ParsedType::StringLiteral(key)
            if key == name);
        if names_declaration && names_member {
            let diagnostic = Diagnostic::ts2502(name, ctx.file_name.clone());
            ctx.push(match name_span {
                Some(span) => diagnostic.with_span(crate::context::convert_span(span)),
                None => diagnostic,
            });
        }
    }
}

fn check_variance_annotations(
    name: &str,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    ctx: &mut CheckerContext,
) {
    if !type_parameters.iter().any(|parameter| parameter.is_in != parameter.is_out) {
        return;
    }
    let _variables = crate::checks::function::enter_body_type_variables(type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(type_parameters, ctx, |ctx| {
        for (index, parameter) in type_parameters.iter().enumerate() {
            if parameter.is_in == parameter.is_out || parameter.constraint.is_some() {
                continue;
            }
            let instantiate = |marker: surge_ts_syntax::ParsedType, ctx: &mut CheckerContext| {
                let arguments = type_parameters
                    .iter()
                    .enumerate()
                    .map(|(position, other)| {
                        if position == index {
                            marker.clone()
                        } else {
                            super::heritage::named_type_with(&other.name, None, Vec::new())
                        }
                    })
                    .collect();
                crate::infer::map_parsed_type(super::heritage::named_type_with(name, None, arguments), ctx)
            };
            let checkpoint = ctx.diagnostics().len();
            let sub = instantiate(surge_ts_syntax::ParsedType::StringLiteral("__surge_variance_sub__".to_string()), ctx);
            let sup = instantiate(surge_ts_syntax::ParsedType::String, ctx);
            ctx.truncate_diagnostics(checkpoint);
            if crate::checks::function::type_contains_degradation(&sub)
                || crate::checks::function::type_contains_degradation(&sup)
            {
                continue;
            }
            // Related structurally: a reference would relate its arguments by
            // the very annotation being checked.
            let (sub, sup) = (sub.peeled(), sup.peeled());
            let (source, target) = if parameter.is_out { (&sub, &sup) } else { (&sup, &sub) };
            if surge_ts_types::is_assignable_to(source, target) {
                continue;
            }
            let display = |marker: &str| {
                let arguments: Vec<String> = type_parameters
                    .iter()
                    .enumerate()
                    .map(|(position, other)| {
                        if position == index { format!("{marker}-{}", other.name) } else { other.name.clone() }
                    })
                    .collect();
                format!("{name}<{}>", arguments.join(", "))
            };
            let (source_name, target_name) = if parameter.is_out {
                (display("sub"), display("super"))
            } else {
                (display("super"), display("sub"))
            };
            let diagnostic = Diagnostic::ts2636(source_name, target_name, ctx.file_name.clone());
            ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, parameter.span));
        }
    });
}

/// Whether the current namespace block itself declares `name` as a value that
/// is no namespace.
fn block_declares_plain_value(name: &str, ctx: &CheckerContext) -> bool {
    ctx.module_value_fallback.as_ref().is_some_and(|block| {
        block.get_own(name).is_some_and(|symbol| {
            matches!(
                symbol.kind,
                crate::symbols::SymbolKind::Var
                    | crate::symbols::SymbolKind::Let
                    | crate::symbols::SymbolKind::Const
                    | crate::symbols::SymbolKind::Function
            )
                // An `export import` member of another block stands in as `any`.
                && !matches!(symbol.ty, surge_ts_types::Type::Any)
                // A namespace's value object is bound as a `const`.
                && !(matches!(symbol.kind, crate::symbols::SymbolKind::Const)
                    && matches!(symbol.ty, surge_ts_types::Type::Object(ref object) if object.alias_name.is_none())
                    && ctx.namespace_meaning(name).is_some())
        })
    })
}

/// `checkAliasSymbol`: inside a namespace block, `import x = N.M` merges with
/// the block's own `x`, and a value declaration there conflicts with a target
/// that is a value too (TS2440, on the import declaration).
fn check_entity_alias_local_conflict(import: &surge_ts_syntax::ParsedImportDeclaration, ctx: &mut CheckerContext) {
    let surge_ts_syntax::ParsedImportKind::EntityAlias { local_name, target, .. } = &import.kind else {
        return;
    };
    if ctx.namespace_member_prefix_stack.is_empty() {
        return;
    }
    if !block_declares_plain_value(local_name, ctx) {
        return;
    }
    let mut segments = target.split('.').map(str::trim);
    let Some(head) = segments.next() else {
        return;
    };
    let mut expression = surge_ts_syntax::ParsedExpression::Identifier { name: head.to_string(), span: None };
    for segment in segments {
        expression = surge_ts_syntax::ParsedExpression::PropertyAccess {
            object: Box::new(expression),
            object_span: None,
            property_name: segment.to_string(),
            property_span: None,
            is_bracketed: false,
            binding_element: false,
        };
    }
    let checkpoint = ctx.diagnostics().len();
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let target_value = crate::infer::infer_expression(&expression, &symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    if !matches!(target_value, crate::infer::InferredExpression::Known(ref ty) if !ty.is_unmodelled()) {
        return;
    }
    let diagnostic = Diagnostic::ts2440(local_name, ctx.file_name.clone());
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, import.span));
}

fn entity_has_type_meaning(target: &str, ctx: &mut CheckerContext) -> bool {
    let checkpoint = ctx.diagnostics().len();
    let resolved = crate::infer::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: target.to_string(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    ctx.truncate_diagnostics(checkpoint);
    !resolved.is_unknown() && !matches!(resolved, surge_ts_types::Type::ErrorType)
}

/// `import x = N.M`: tsc resolves the entity's first name as a namespace
/// (`getSymbolOfPartOfRightHandSideOfImportEquals`). A name that is only a
/// type is TS2702; a name with no meaning at all is TS2503. A name with a
/// value meaning but no namespace (tsc's TS2503 as well) is left alone: the
/// namespaces surge registers are not every one a program can see.
fn check_entity_alias_head(import: &surge_ts_syntax::ParsedImportDeclaration, ctx: &mut CheckerContext) {
    // `checkAliasSymbol`: `export import x = require(…)` of a module whose
    // `export =` is only a type.
    if let surge_ts_syntax::ParsedImportKind::Equals { local_name, is_type_only: false, exported: true, .. } = &import.kind
        && (ctx.options.isolated_modules || ctx.options.verbatim_module_syntax)
        && ctx.symbols.get(local_name).is_none()
        && ctx.lookup_type_declaration(local_name).is_some()
    {
        let flag = if ctx.options.verbatim_module_syntax { "verbatimModuleSyntax" } else { "isolatedModules" };
        let diagnostic = Diagnostic::ts1269(flag, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, import.span));
        return;
    }
    let surge_ts_syntax::ParsedImportKind::EntityAlias { target, local_name, name_span } = &import.kind else {
        return;
    };
    // `checkTypeNameIsReserved`: an alias whose target has a type meaning
    // cannot take a predefined type's name.
    if matches!(
        local_name.as_str(),
        "any" | "unknown" | "never" | "number" | "bigint" | "boolean" | "string" | "symbol" | "void" | "object" | "undefined"
    ) && entity_has_type_meaning(target, ctx)
    {
        let diagnostic = Diagnostic::ts2438(local_name, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, *name_span));
    }
    let head = target.split('.').next().unwrap_or(target);
    let head_span = import.module_specifier_span.map(|span| surge_ts_syntax::TextSpan {
        start: span.start,
        end: span.start + head.len(),
    });
    report_entity_alias_through_type_only(head, target, import, ctx);
    // `checkImportEqualsDeclaration`: an instantiated namespace head that a
    // value of the block itself hides is TS2437.
    if ctx.namespace_meaning(head) == Some(true)
        && let Some(prefix) = ctx.namespace_member_prefix_stack.last()
        && ctx.namespace_registry.lookup(&ctx.file_name, &format!("{prefix}.{head}")).is_none()
        && block_declares_plain_value(head, ctx)
    {
        let diagnostic = Diagnostic::ts2437(head, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
        return;
    }
    // A class binds a value and a type but no namespace.
    if ctx.namespace_meaning(head).is_none()
        && matches!(
            ctx.lookup_type_declaration(head),
            Some(crate::symbols::TypeDeclarationInfo::Interface(info)) if info.is_class_instance
        )
    {
        let diagnostic = Diagnostic::ts2702(head, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
        return;
    }
    // `checkAliasSymbol`: `export import x = N` of a namespace with no value
    // side is elided by no single-file transpiler.
    if ctx.module_export_depth > 0
        && !target.contains('.')
        && (ctx.options.isolated_modules || ctx.options.verbatim_module_syntax)
        && ctx.namespace_meaning(head) == Some(false)
        && ctx.symbols.get(head).is_none()
    {
        let flag = if ctx.options.verbatim_module_syntax { "verbatimModuleSyntax" } else { "isolatedModules" };
        let diagnostic = Diagnostic::ts1269(flag, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, import.span));
        return;
    }
    if ctx.namespace_meaning(head).is_some() {
        report_verbatim_type_only_entity_alias(head, target, import, ctx);
        // A module namespace's members are not in the namespace registry.
        if !ctx.is_import_binding(head) {
            let exported = ctx.module_export_depth > 0;
            check_entity_alias_namespace_path(target, head_span, exported, ctx);
        }
        return;
    }
    if ctx.is_import_binding(head)
        || ctx.is_umd_global_value_reference(head)
        || ctx.ambient_global_symbols.get(head).is_some()
    {
        return;
    }
    // `getSymbolOfPartOfRightHandSideOfImportEquals` resolves the head with
    // `SymbolFlagsNamespace` alone, which a `var` or a `function` lacks.
    if let Some(symbol) = ctx.symbols.get(head) {
        let plain_value = matches!(
            symbol.kind,
            crate::symbols::SymbolKind::Var
                | crate::symbols::SymbolKind::Let
                | crate::symbols::SymbolKind::Const
                | crate::symbols::SymbolKind::Function
        ) && ctx.lookup_type_declaration(head).is_none();
        if plain_value {
            let diagnostic = Diagnostic::ts2503(head, ctx.file_name.clone());
            ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
        }
        return;
    }
    let diagnostic = match ctx.lookup_type_declaration(head) {
        Some(crate::symbols::TypeDeclarationInfo::Alias(alias)) if alias.enum_name.is_some() => return,
        Some(_) => Diagnostic::ts2702(head, ctx.file_name.clone()),
        None => Diagnostic::ts2503(head, ctx.file_name.clone()),
    };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
}

/// Every name between the head and the last of `import x = N.M.K` is a
/// namespace tsc looks the next one up in; one the registry does not know
/// under a known namespace is TS2694. The last name may carry any meaning, so
/// it is judged only against a namespace whose whole export list the
/// registry saw: exported with no meaning at all, the alias resolves to
/// `unknownSymbol`, which `markAliasSymbolAsReferenced` takes for a value, so
/// an `export import` also reads its head as one (TS2708 for a namespace with
/// no value side).
fn check_entity_alias_namespace_path(
    target: &str,
    head_span: Option<surge_ts_syntax::TextSpan>,
    exported: bool,
    ctx: &mut CheckerContext,
) {
    let segments: Vec<&str> = target.split('.').collect();
    let mut left = segments[0].to_string();
    for segment in segments.iter().take(segments.len().saturating_sub(1)).skip(1) {
        let path = format!("{left}.{segment}");
        if ctx.namespace_meaning(&path).is_none() {
            let diagnostic = Diagnostic::ts2694(&left, segment, ctx.file_name.clone());
            let span = head_span.map(|span| surge_ts_syntax::TextSpan {
                start: span.start + left.len() + 1,
                end: span.start + path.len(),
            });
            ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
            return;
        }
        left = path;
    }
    let [head, .., last] = segments.as_slice() else {
        return;
    };
    let missing = ctx
        .namespace_registry
        .info(&ctx.file_name, &left)
        .is_some_and(|info| info.complete && info.exports.get(*last).copied().unwrap_or(0) == 0);
    if !missing {
        return;
    }
    if exported && ctx.namespace_meaning(head) == Some(false) {
        let diagnostic = Diagnostic::ts2708(head, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
    }
    let diagnostic = Diagnostic::ts2694(&left, last, ctx.file_name.clone());
    let span = head_span.map(|span| surge_ts_syntax::TextSpan {
        start: span.start + left.len() + 1,
        end: span.start + target.len(),
    });
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
}

/// tsc's `checkAndReportErrorForResolvingImportAliasToTypeOnlySymbol`: an
/// entity alias whose head was bound by `import type`, or by an import of an
/// `export type`, or that names such an export of a namespace import
/// (`ns.member`), is TS1380 or TS1379 at the whole reference.
fn report_entity_alias_through_type_only(
    head: &str,
    target: &str,
    import: &surge_ts_syntax::ParsedImportDeclaration,
    ctx: &mut CheckerContext,
) {
    let qualified = target
        .match_indices('.')
        .map(|(end, _)| &target[..end])
        .chain(std::iter::once(target))
        .skip(1)
        .collect::<Vec<_>>();
    let kind = if let Some(kind) =
        qualified.iter().rev().find_map(|prefix| ctx.type_only_value_reference(prefix))
    {
        kind
    } else if ctx.is_import_binding(head) && ctx.file_type_only_import_names.contains(head) {
        crate::modules::TypeOnlyAliasKind::Import
    } else {
        match ctx.type_only_value_reference(head) {
            Some(kind) => kind,
            None => return,
        }
    };
    let diagnostic = match kind {
        crate::modules::TypeOnlyAliasKind::Import => Diagnostic::ts1380(ctx.file_name.clone()),
        crate::modules::TypeOnlyAliasKind::Export => Diagnostic::ts1379(ctx.file_name.clone()),
    };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, import.module_specifier_span));
}

/// `declare module "m" { export = A; }`: tsc resolves `A` in the module block
/// and outward (`resolveEntityName`), and a name nothing declares is TS2304.
/// The block's own declarations are its scope; one that imports anything is
/// left alone.
fn report_unresolved_ambient_module_export_equals(
    module: &surge_ts_syntax::ParsedDeclareModuleDeclaration,
    ctx: &mut CheckerContext,
) {
    fn declared_name(statement: &ParsedStatement) -> Option<&str> {
        Some(match statement {
            ParsedStatement::VariableDeclaration(variable) => variable.name.as_str(),
            ParsedStatement::FunctionDeclaration(function) => function.name.as_str(),
            ParsedStatement::ClassDeclaration(class) => class.name.as_str(),
            ParsedStatement::InterfaceDeclaration(interface) => interface.name.as_str(),
            ParsedStatement::TypeAliasDeclaration(alias) => alias.name.as_str(),
            ParsedStatement::NamespaceDeclaration(namespace) => namespace.name.split('.').next()?,
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => return declared_name(declaration),
                _ => return None,
            },
            _ => return None,
        })
    }
    if module.is_shorthand
        || module.statements.iter().any(|statement| matches!(statement, ParsedStatement::ImportDeclaration(_)))
    {
        return;
    }
    for statement in &module.statements {
        let ParsedStatement::ExportDeclaration(export) = statement else {
            continue;
        };
        let ParsedExportDeclaration::Equals { exported_name, exported_name_span, .. } = export.as_ref() else {
            continue;
        };
        let name = exported_name.as_str();
        if name.contains('.')
            || module.statements.iter().any(|other| declared_name(other) == Some(name))
            || ctx.symbols.get(name).is_some()
            || ctx.ambient_global_symbols.get(name).is_some()
            || ctx.lookup_type_declaration(name).is_some()
            || ctx.namespace_meaning(name).is_some()
        {
            continue;
        }
        let diagnostic = Diagnostic::ts2304(name, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, *exported_name_span));
    }
}

/// `checkAliasSymbol` under `verbatimModuleSyntax`: `import x = N.T` whose
/// target has no value side (a type alias, an interface, an uninstantiated
/// namespace) is TS1288 on the declaration.
fn report_verbatim_type_only_entity_alias(
    head: &str,
    target: &str,
    import: &surge_ts_syntax::ParsedImportDeclaration,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.verbatim_module_syntax
        || crate::modules::is_declaration_file_name(&ctx.file_name)
        || ctx.is_import_binding(head)
    {
        return;
    }
    let Some(member) = target.strip_prefix(head).and_then(|rest| rest.strip_prefix('.')) else {
        return;
    };
    if member.contains('.') {
        return;
    }
    let Some((_, info)) = ctx.namespace_info(head) else {
        return;
    };
    let Some(meaning) = info.exports.get(member).copied().filter(|_| info.complete) else {
        return;
    };
    if meaning & super::namespaces::MEANING_VALUE == 0 {
        let diagnostic = Diagnostic::ts1288(ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, import.span));
    }
}
