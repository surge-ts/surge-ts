//! tsc's `checkEnumMember`, and the computed-member case of
//! `computeConstantEnumMemberValue`: every member initializer is checked as an
//! expression with the enum's members in scope, and one with no constant value
//! must be numeric (TS18033).

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedEnumBody, ParsedExpression, TextSpan};
use surge_ts_types::{Type, is_assignable_to};

use crate::context::{CheckerContext, convert_span};
use crate::infer::InferredExpression;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolKind, SymbolTable};

/// `symbols` is the scope the enum is declared in, after its object is bound.
/// `top_level`: the enum is declared at a file's top level, where a script's
/// enum merges with the same-named enums of the program's other scripts.
pub(crate) fn check_enum_members(
    enum_name: &str,
    bodies: &[ParsedEnumBody],
    symbols: SymbolTable,
    top_level: bool,
    ctx: &mut CheckerContext,
) {
    let Some(Type::Object(object)) = symbols.get(enum_name).map(|symbol| symbol.ty.peeled()) else {
        return;
    };
    let mut scopes = ScopeStack::from_root(symbols);
    scopes.push_child();
    // A bare name in an initializer is first a member of the enum, of any of
    // its declarations; the enum's object carries those of this file.
    for (name, property) in object.properties.iter() {
        let _ = scopes.insert_current(
            name.to_string(),
            SymbolInfo {
                ty: property.ty.clone(),
                kind: SymbolKind::Const,
                function_signature: None,
            },
        );
    }
    if top_level {
        bind_members_of_other_files(enum_name, &object, bodies, &mut scopes, ctx);
    }
    for body in bodies {
        for member in &body.members {
            let diagnostics_before = ctx.diagnostics().len();
            let inferred = crate::checks::expr::evaluate_expression(
                &member.initializer,
                member.initializer_span,
                scopes.visible_symbols(),
                ctx,
            );
            let reported = ctx.diagnostics().len() > diagnostics_before;
            let isolated = ctx.options.isolated_modules || ctx.options.verbatim_module_syntax;
            let constant = constant_kind(&inferred, &member.initializer, scopes.visible_symbols(), ctx);
            if let Some(successor) = member.uninitialized_successor {
                report_uninitialized_successor(constant, successor, isolated, ctx);
            }
            if isolated
                && constant == Some(ConstantKind::String)
                && syntactically_string(&member.initializer, &object, top_level, ctx) == Some(false)
                && let Some(span) = member.initializer_span
            {
                let diagnostic =
                    Diagnostic::ts18055(format!("{enum_name}.{}", member.name), ctx.file_name.clone())
                        .with_span(convert_span(span));
                ctx.push(diagnostic);
            }
            // A reference the checker cannot resolve has no constant value
            // either (`computeConstantEnumMemberValue`). Anything else that is
            // not constant the syntax already reported.
            if body.is_const
                && is_entity_reference(&member.initializer)
                && match inferred {
                    InferredExpression::MissingProperty { .. }
                    | InferredExpression::UnresolvedIdentifier { .. }
                    | InferredExpression::Known(Type::ErrorType) => true,
                    // A literal key the receiver lacks reports and reads on.
                    InferredExpression::Unknown => {
                        reported
                            && matches!(
                                member.initializer,
                                ParsedExpression::IndexAccess { .. }
                                    | ParsedExpression::ElementAccess { .. }
                                    | ParsedExpression::PropertyAccess { is_bracketed: true, .. }
                            )
                    }
                    InferredExpression::Known(_) => false,
                }
                && let Some(span) = member.initializer_span
            {
                ctx.push(Diagnostic::ts2474(ctx.file_name.clone()).with_span(convert_span(span)));
            }
            // A `const enum` reports a computed initializer as TS2474 instead.
            if body.is_const || !member.computed {
                continue;
            }
            let (InferredExpression::Known(ty), Some(span)) = (inferred, member.initializer_span)
            else {
                continue;
            };
            if ty.is_unknown()
                || crate::checks::function::type_contains_degradation(&ty)
                || is_assignable_to(&ty, &Type::Number)
            {
                continue;
            }
            let source = crate::checks::expr::source_display_name(&ty, &Type::Number);
            let diagnostic = Diagnostic::ts18033(source, "number", ctx.file_name.clone())
                .with_span(convert_span(span));
            ctx.push(diagnostic);
        }
    }
}

/// `computeEnumMemberValue` for a member with no initializer after one whose
/// value another file holds: a string leaves nothing to number it from
/// (TS1061), and a number is one a single-file transpiler cannot see (TS18056).
fn report_uninitialized_successor(
    constant: Option<ConstantKind>,
    span: TextSpan,
    isolated: bool,
    ctx: &mut CheckerContext,
) {
    let diagnostic = match constant {
        Some(ConstantKind::String) => Diagnostic::ts1061(ctx.file_name.clone()),
        Some(ConstantKind::Number) if isolated => Diagnostic::ts18056(ctx.file_name.clone()),
        _ => return,
    };
    ctx.push(diagnostic.with_span(convert_span(span)));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConstantKind {
    Number,
    String,
}

/// What the evaluator's value for an initializer is, where surge can tell it
/// has one: a literal type, or a plain `string` or `number` folded from
/// operands that are literals or read one.
fn constant_kind(
    inferred: &InferredExpression,
    initializer: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<ConstantKind> {
    match inferred {
        InferredExpression::Known(Type::NumberLiteral(_)) => Some(ConstantKind::Number),
        InferredExpression::Known(Type::StringLiteral(_)) => Some(ConstantKind::String),
        InferredExpression::Known(Type::Number) if folds_to_constant(initializer, symbols, ctx) => {
            Some(ConstantKind::Number)
        }
        InferredExpression::Known(Type::String) if folds_to_constant(initializer, symbols, ctx) => {
            Some(ConstantKind::String)
        }
        _ => None,
    }
}

/// The evaluator's value for an initializer surge types as plain `string` or
/// `number`: it has one when every operand is a literal or reads one.
fn folds_to_constant(expression: &ParsedExpression, symbols: &SymbolTable, ctx: &mut CheckerContext) -> bool {
    match expression {
        ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_) => true,
        ParsedExpression::TemplateLiteral { expressions, .. } => {
            expressions.iter().all(|expression| folds_to_constant(expression, symbols, ctx))
        }
        ParsedExpression::Unary { operator, operand, .. } => {
            matches!(
                operator,
                surge_ts_syntax::ParsedUnaryOperator::Plus
                    | surge_ts_syntax::ParsedUnaryOperator::Minus
                    | surge_ts_syntax::ParsedUnaryOperator::BitwiseNot
            ) && folds_to_constant(operand, symbols, ctx)
        }
        ParsedExpression::Binary { left, right, .. } => {
            folds_to_constant(left, symbols, ctx) && folds_to_constant(right, symbols, ctx)
        }
        ParsedExpression::Identifier { .. } | ParsedExpression::PropertyAccess { .. } => {
            let checkpoint = ctx.diagnostics().len();
            let inferred = crate::infer::infer_expression(expression, symbols, ctx);
            ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
            matches!(inferred, InferredExpression::Known(Type::StringLiteral(_) | Type::NumberLiteral(_)))
        }
        _ => false,
    }
}

/// A name, or a member of one read by a name or a literal key: what the
/// enum evaluator resolves rather than computes.
fn is_entity_reference(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::Identifier { .. } => true,
        ParsedExpression::PropertyAccess { object, .. } => is_entity_reference(object),
        ParsedExpression::IndexAccess { index, .. } => {
            matches!(index.as_ref(), ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_))
        }
        ParsedExpression::ElementAccess { object, index, .. } => {
            is_entity_reference(object)
                && matches!(index.as_ref(), ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_))
        }
        _ => false,
    }
}

/// The evaluator's `IsSyntacticallyString`, as far as surge can tell: a string
/// literal or template is one, `+` is one when either side is, and a value
/// read from another file (an import, another script's `const`) is not.
/// `None` where the answer depends on a declaration not followed here (a
/// member, a local `const`).
fn syntactically_string(
    expression: &ParsedExpression,
    enum_object: &surge_ts_types::ObjectType,
    top_level: bool,
    ctx: &CheckerContext,
) -> Option<bool> {
    match expression {
        ParsedExpression::StringLiteral(_) | ParsedExpression::TemplateLiteral { .. } => Some(true),
        ParsedExpression::NumberLiteral(_) => Some(false),
        ParsedExpression::Unary { operand, .. } => {
            syntactically_string(operand, enum_object, top_level, ctx).map(|_| false)
        }
        ParsedExpression::Binary { left, operator, right, .. } => {
            let left = syntactically_string(left, enum_object, top_level, ctx);
            let right = syntactically_string(right, enum_object, top_level, ctx);
            if *operator != surge_ts_syntax::ParsedBinaryOperator::Add {
                return left.and(right).map(|_| false);
            }
            match (left, right) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            }
        }
        ParsedExpression::Identifier { name, .. } if !enum_object.properties.contains_key(name.as_str()) => {
            (ctx.is_import_binding(name) || (top_level && is_another_files_const(name, ctx))).then_some(false)
        }
        _ => None,
    }
}

/// A global `const` another file declares (`evaluateEntityNameExpression`
/// reads it as no syntactic string whatever its initializer).
fn is_another_files_const(name: &str, ctx: &CheckerContext) -> bool {
    !ctx.file_top_level_names.contains(name)
        && ctx.ambient_global_symbols.get(name).is_none()
        && ctx.symbols.get(name).is_some_and(|symbol| matches!(symbol.kind, SymbolKind::Const))
}

/// The members other files' declarations of the enum give it, which tsc's
/// `Resolve` finds in the merged enum's exports (`case ast.KindEnumDeclaration`).
/// Under `isolatedModules` a reference to one is TS1281: a single-file
/// transpiler cannot tell the name is a member.
fn bind_members_of_other_files(
    enum_name: &str,
    object: &surge_ts_types::ObjectType,
    bodies: &[ParsedEnumBody],
    scopes: &mut ScopeStack,
    ctx: &mut CheckerContext,
) {
    let mut references = Vec::new();
    for body in bodies {
        for member in &body.members {
            collect_bare_names(&member.initializer, &mut references);
        }
    }
    let isolated_flag = if ctx.options.verbatim_module_syntax {
        Some("verbatimModuleSyntax")
    } else if ctx.options.isolated_modules {
        Some("isolatedModules")
    } else {
        None
    };
    for (name, span) in references {
        if object.properties.contains_key(name) {
            continue;
        }
        let qualified = format!("{enum_name}.{name}");
        let declared_elsewhere = matches!(
            ctx.lookup_type_declaration(&qualified),
            Some(crate::symbols::TypeDeclarationInfo::Alias(alias))
                if alias.enum_name.as_deref() == Some(enum_name)
                    && alias.file_name.as_ref() != ctx.file_name.as_str()
        );
        if !declared_elsewhere {
            continue;
        }
        let _ = scopes.insert_current(
            name,
            SymbolInfo {
                ty: Type::Unknown,
                kind: SymbolKind::Const,
                function_signature: None,
            },
        );
        if let (Some(flag), Some(span)) = (isolated_flag, span) {
            let diagnostic = Diagnostic::ts1281(name, flag, &qualified, ctx.file_name.clone())
                .with_span(convert_span(span));
            ctx.push(diagnostic);
        }
    }
}

/// The names an initializer reads bare, outside any function it contains.
fn collect_bare_names<'a>(
    expression: &'a ParsedExpression,
    names: &mut Vec<(&'a str, Option<TextSpan>)>,
) {
    if let ParsedExpression::Identifier { name, span } = expression {
        names.push((name.as_str(), *span));
        return;
    }
    expression.for_each_child(&mut |child| collect_bare_names(child, names));
}
