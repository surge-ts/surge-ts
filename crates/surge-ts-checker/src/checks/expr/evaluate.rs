use super::*;
use surge_ts_syntax::{ParsedLogicalOperator, ParsedType, ParsedUnaryOperator};

/// A computed key is an expression like any other (tsc's
/// `checkComputedPropertyName`); inference only reads its type.
pub(crate) fn check_computed_property_keys(
    properties: &[surge_ts_syntax::ParsedObjectProperty],
    fallback_span: Option<SyntaxTextSpan>,
    contextual: Option<&surge_ts_types::ObjectType>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let mut typed_names: Vec<(usize, Option<String>)> = Vec::new();
    for (index, property) in properties.iter().enumerate() {
        if let Some(key) = property.member_key.as_deref() {
            let key_span = property.name_span.or(property.span).or(fallback_span);
            let key_result = evaluate_expression(key, key_span, symbols, ctx);
            report_invalid_computed_key(&key_result, key_span, ctx);
            if !is_literal_key(key) {
                typed_names.push((index, typed_property_name(key, &key_result)));
            }
        }
        let mut key_type = None;
        if let Some(key) = property.computed_key.as_deref() {
            let key_span = property.name_span.or(property.span).or(fallback_span);
            let key_result = evaluate_expression(key, key_span, symbols, ctx);
            report_invalid_computed_key(&key_result, key_span, ctx);
            if !is_literal_key(key) {
                typed_names.push((index, typed_property_name(key, &key_result)));
            }
            if let InferredExpression::Known(ty) = key_result {
                key_type = Some(ty);
            }
        }
        if let Some(value) = property.unnamed_key_value.as_deref() {
            let span = property.span.or(fallback_span);
            // tsc's `getContextualTypeForObjectLiteralElement`: a member whose
            // name is no literal is contextually typed by the index signature
            // its key applies to.
            let index_type = contextual
                .zip(key_type.as_ref())
                .and_then(|(contextual, key)| applicable_index_type(contextual, key));
            let _ = match index_type {
                Some(index_type) => crate::checks::expected::evaluate_expression_with_expected_type(
                    value,
                    span,
                    Some(&index_type),
                    crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
                    symbols,
                    ctx,
                ),
                None => evaluate_expression(value, span, symbols, ctx),
            };
        }
    }
    if !typed_names.is_empty() {
        report_duplicate_typed_names(properties, &typed_names, ctx);
    }
}

/// A key written as a string or number literal names its property as the
/// literal does; the grammar walk reads those names.
fn is_literal_key(key: &ParsedExpression) -> bool {
    matches!(key, ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_))
}

/// tsc's `getEffectivePropertyNameForPropertyNameNode` for a computed key that
/// is no literal: the name its type gives (`tryGetNameFromType`). surge types
/// a well-known symbol (`Symbol.iterator`) as `symbol`, where the lib declares
/// each as a unique symbol of its own.
fn typed_property_name(key: &ParsedExpression, key_result: &InferredExpression) -> Option<String> {
    if let ParsedExpression::PropertyAccess { object, property_name, .. } = key
        && matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "Symbol")
        && crate::program::WELL_KNOWN_SYMBOLS.contains(&property_name.as_str())
    {
        return Some(format!("__@Symbol.{property_name}"));
    }
    let InferredExpression::Known(ty) = key_result else {
        return None;
    };
    name_of_key_type(ty)
}

/// tsc's `tryGetNameFromType`: a unique symbol's own name, or a string or
/// number literal's value (an enum member's included).
fn name_of_key_type(ty: &Type) -> Option<String> {
    match ty {
        Type::Reference(reference) if reference.is_unique_symbol() => Some(format!("__@{}", reference.id)),
        Type::Reference(reference) => name_of_key_type(&reference.resolve()),
        Type::StringLiteral(value) => Some(value.clone()),
        Type::NumberLiteral(literal) => Some(literal.value.clone()),
        _ => None,
    }
}

/// tsc's `checkGrammarObjectLiteralExpression` duplicate-name rule, for the
/// pairs a typed name takes part in: a second property assignment of a name is
/// TS1117. A method or accessor meeting a property of its name ends the check,
/// as the error tsc reports there does; those errors, and every pair of names
/// written as such, are the grammar walk's.
fn report_duplicate_typed_names(
    properties: &[surge_ts_syntax::ParsedObjectProperty],
    typed_names: &[(usize, Option<String>)],
    ctx: &mut CheckerContext,
) {
    const PROPERTY: u8 = 1;
    const METHOD: u8 = 1 << 1;
    const GET: u8 = 1 << 2;
    const SET: u8 = 1 << 3;
    // A computed method or accessor the parser could not name keeps no kind.
    const UNKNOWN: u8 = 1 << 4;
    let mut typed = typed_names.iter().peekable();
    let mut seen: std::collections::HashMap<&str, (u8, bool)> = std::collections::HashMap::new();
    for (index, property) in properties.iter().enumerate() {
        let typed_name = typed.next_if(|(key_index, _)| *key_index == index).map(|(_, name)| name);
        let (name, is_typed) = match typed_name {
            Some(Some(name)) => (name.as_str(), true),
            Some(None) => continue,
            None if property.is_spread => continue,
            None => (property.name.as_str(), false),
        };
        let kind = if property.is_accessor {
            if property.is_getter { GET } else { SET }
        } else if property.is_method {
            METHOD
        } else if property.is_spread && property.unnamed_key_value.is_none() {
            UNKNOWN
        } else {
            PROPERTY
        };
        let Some(&(existing, first_typed)) = seen.get(name) else {
            seen.insert(name, (kind, is_typed));
            continue;
        };
        if kind == UNKNOWN || existing == UNKNOWN {
            return;
        }
        if kind == METHOD && existing & METHOD != 0 {
            continue;
        }
        if kind == PROPERTY && existing & PROPERTY != 0 {
            if (is_typed || first_typed)
                && let Some(span) = property.span
            {
                let diagnostic = Diagnostic::ts1117(ctx.file_name.clone());
                ctx.push(diagnostic.with_span(crate::context::convert_span(span)));
            }
            continue;
        }
        if kind & (GET | SET) != 0 && existing & (GET | SET) != 0 && existing != GET | SET && kind != existing {
            seen.insert(name, (existing | kind, first_typed));
            continue;
        }
        return;
    }
}

/// tsc's `findApplicableIndexInfo`: a numeric key reads the number index
/// signature, falling back to the string one, and a string key the string one.
fn applicable_index_type(object: &surge_ts_types::ObjectType, key: &Type) -> Option<Type> {
    let is_number = matches!(key, Type::Number | Type::NumberLiteral(_));
    if is_number && let Some(index) = object.number_index_type.as_deref() {
        return Some(index.clone());
    }
    if is_number || matches!(key, Type::String | Type::StringLiteral(_)) {
        return object.string_index_type.as_deref().cloned();
    }
    None
}

/// tsc's `checkComputedPropertyName`: a key must be `null`/`undefined`-free
/// and assignable to `string | number | symbol` — TS2464. A type variable of
/// the body being checked relates through its constraint; a placeholder
/// parameter is left alone.
pub(crate) fn report_invalid_computed_key(
    key: &InferredExpression,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (key, span) else {
        return;
    };
    if !computed_key_type_is_invalid(ty) {
        return;
    }
    let diagnostic = Diagnostic::ts2464(ctx.file_name.clone());
    ctx.push(diagnostic.with_span(crate::context::convert_span(span)));
}

fn computed_key_type_is_invalid(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Null => true,
        _ if has_unmodelled_member(ty) => false,
        _ => !is_assignable_to(ty, &union_type(vec![Type::String, Type::Number, Type::Symbol])),
    }
}

fn has_unmodelled_member(ty: &Type) -> bool {
    match ty {
        Type::Any | Type::Unknown | Type::ErrorType => true,
        Type::TypeParameter(_) => !ty.is_type_variable(),
        Type::Union(union) => union.types().iter().any(has_unmodelled_member),
        Type::Reference(reference) => has_unmodelled_member(&reference.resolve()),
        _ => false,
    }
}

/// tsc ends the flow after a call whose return type is `never`
/// (`util.assertNever(x)`), so a body whose last statement is one has no
/// reachable end point. The verdict is recorded as the call is checked; the
/// flow pass itself sees no types.
fn record_never_returning_call(
    return_type: &surge_ts_types::Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if let Some(span) = span
        && matches!(return_type.peeled(), surge_ts_types::Type::Never)
    {
        ctx.never_returning_calls.insert((span.start, span.end));
    }
}

/// The key [`record_never_returning_call`] would record for the call `body`
/// ends with, read before the body is checked.
pub(crate) fn tail_call_key(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
) -> Option<(usize, usize)> {
    let surge_ts_syntax::ParsedFunctionBodyStatement::Expression(expression) = body.last()? else {
        return None;
    };
    let span = match expression.as_ref() {
        ParsedExpression::Call { callee_span, .. } => *callee_span,
        ParsedExpression::PropertyCall { property_span, .. } => *property_span,
        _ => None,
    }?;
    Some((span.start, span.end))
}

pub(crate) fn evaluate_expression(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let result = crate::checks::function::settle_call_result(
        expression,
        evaluate_expression_unsettled(expression, fallback_span, symbols, ctx),
    );
    if let Some(substitutions) = tagged_template_substitutions(expression) {
        report_symbol_substitutions(substitutions, symbols, ctx);
    }
    if let InferredExpression::Known(ty) = &result {
        crate::semantic::record_expression_type(expression, ty, ctx);
    }
    result
}

/// The substitutions of a tagged template, which lowers to a call whose first
/// argument is the template's strings array.
fn tagged_template_substitutions(expression: &ParsedExpression) -> Option<&[surge_ts_syntax::ParsedCallArgument]> {
    let arguments = match expression {
        ParsedExpression::Call { arguments, .. }
        | ParsedExpression::PropertyCall { arguments, .. }
        | ParsedExpression::ExpressionCall { arguments, .. } => arguments,
        _ => return None,
    };
    match arguments.split_first() {
        Some((first, rest))
            if matches!(first.expression, ParsedExpression::TemplateStringsArray { .. }) =>
        {
            Some(rest)
        }
        _ => None,
    }
}

/// A tagged template is still a template expression: a substitution that may
/// be a symbol cannot be converted to a string (TS2731), whatever the tag.
fn report_symbol_substitutions(
    substitutions: &[surge_ts_syntax::ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    for substitution in substitutions {
        if let InferredExpression::Known(ty) = infer_expression(&substitution.expression, symbols, ctx)
            && type_may_be_symbol(&ty)
        {
            ctx.push(diagnostic_with_syntax_span(
                Diagnostic::ts2731(ctx.file_name.clone()),
                substitution.span,
            ));
        }
    }
}

fn evaluate_expression_unsettled(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    record_expression_check();
    crate::checks::check_umd_global_value_reference(expression, fallback_span, symbols, ctx);
    match expression {
        // The inference pass types the literal but does not check it, so a
        // property value's own errors — an unresolved name, a bad member, an
        // untyped callback parameter — never surfaced. Same split the array
        // literal arm below uses: infer for the type, evaluate for diagnostics.
        ParsedExpression::ObjectLiteral { properties, span } => {
            let inferred_expression = infer_expression(expression, symbols, ctx);
            let member_this = crate::infer::expression::literal_member_this(
                properties,
                *span,
                match &inferred_expression {
                    InferredExpression::Known(ty) => Some(ty),
                    _ => None,
                },
                ctx,
            );

            check_computed_property_keys(properties, fallback_span, None, symbols, ctx);
            let mut explicit_properties: Vec<(&str, Option<SyntaxTextSpan>)> = Vec::new();
            for property in properties {
                if !property.is_spread && !property.is_accessor && property.computed_key.is_none() {
                    explicit_properties.push((&property.name, property.name_span));
                }
                // Method and accessor shorthand is checked by the inference pass
                // itself, which must route it through the arrow-checking path to
                // honor its declared signature; evaluating it here as well would
                // duplicate every diagnostic that raised.
                if property.is_method || property.is_accessor {
                    continue;
                }
                if property.is_shorthand {
                    ctx.shorthand_property_depth += 1;
                }
                if let (Some(member_this), ParsedExpression::ArrowFunction(member)) =
                    (&member_this, &property.value)
                {
                    member_this.hand_to(member, false, ctx);
                }
                let property_result = evaluate_expression(
                    &property.value,
                    property.value_span.or(property.span).or(fallback_span),
                    symbols,
                    ctx,
                );
                ctx.next_arrow_this = None;
                if property.is_spread {
                    super::check_object_spread_type(
                        &property_result,
                        property.span.or(property.value_span).or(fallback_span),
                        ctx,
                    );
                    if property.unnamed_key_value.is_none() {
                        report_overwritten_properties(&property_result, &explicit_properties, ctx);
                    }
                }
                if property.is_shorthand {
                    ctx.shorthand_property_depth -= 1;
                }
            }

            inferred_expression
        }
        // A template's interpolations are ordinary expressions and carry their own
        // errors (`${process.env.PORT}` is still a TS4111 index-signature access).
        ParsedExpression::TemplateLiteral {
            expressions,
            span,
            expression_spans,
            ..
        } => {
            for (index, interpolation) in expressions.iter().enumerate() {
                let interpolation_span = expression_spans
                    .get(index)
                    .copied()
                    .flatten()
                    .or(*span)
                    .or(fallback_span);
                let result = evaluate_expression(interpolation, interpolation_span, symbols, ctx);
                let InferredExpression::Known(ty) = &result else {
                    continue;
                };
                // A symbol cannot be converted to a string implicitly (TS2731).
                if type_may_be_symbol(ty) {
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2731(ctx.file_name.clone()),
                        interpolation_span,
                    ));
                }
            }
            infer_expression(expression, symbols, ctx)
        }
        ParsedExpression::ArrayLiteral { elements, .. } => {
            let inferred_expression = infer_expression(expression, symbols, ctx);

            for element in elements {
                let element_result = evaluate_expression(
                    &element.expression,
                    element.span.or(fallback_span),
                    symbols,
                    ctx,
                );
                if element.spread {
                    super::check_iterable_operand(&element_result, element.span, true, ctx);
                    super::check_iteration_next_type(
                        &element_result,
                        element.span,
                        &Type::Undefined,
                        super::IterationSend::Spread,
                        false,
                        ctx,
                    );
                }
            }

            report_inferred_expression(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                    inferred_expression.clone()
                }),
                fallback_span,
                symbols,
                ctx,
            );
            inferred_expression
        }
        ParsedExpression::Call {
            callee_name,
            callee_span,
            type_arguments,
            arguments,
            ..
        } => match check_call_like(
            callee_name,
            *callee_span,
            None,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ) {
            Some(return_type) => {
                record_never_returning_call(&return_type, *callee_span, ctx);
                InferredExpression::Known(return_type)
            }
            None => InferredExpression::Unknown,
        },
        ParsedExpression::New {
            callee,
            callee_span,
            span,
            type_arguments,
            arguments,
        } => match check_new_like(
            callee,
            *callee_span,
            *span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        },
        ParsedExpression::PropertyCall {
            object,
            object_span,
            property_name,
            property_span,
            call_span,
            type_arguments,
            arguments,
            ..
        } => match check_property_call_like(
            object,
            *object_span,
            property_name,
            *property_span,
            *call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        ) {
            Some(return_type) => {
                record_never_returning_call(&return_type, *property_span, ctx);
                InferredExpression::Known(return_type)
            }
            None => InferredExpression::Unknown,
        },
        ParsedExpression::OptionalPropertyCall {
            object,
            object_span,
            property_name,
            property_span,
            call_span,
            type_arguments,
            arguments,
        } => match check_optional_property_call(
            object,
            *object_span,
            property_name,
            *property_span,
            *call_span,
            type_arguments,
            arguments,
            None,
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        },
        ParsedExpression::ExpressionCall {
            callee,
            callee_span,
            type_arguments,
            arguments,
        } => match crate::checks::call::check_expression_call(
            callee,
            *callee_span,
            None,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        },
        ParsedExpression::OptionalCall {
            callee,
            callee_span,
            type_arguments,
            arguments,
        } => match check_optional_call_like(
            callee,
            *callee_span,
            None,
            type_arguments,
            arguments,
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        },
        ParsedExpression::NullishCoalescing {
            left,
            left_span,
            right,
            right_span,
        } => evaluate_nullish_coalescing(
            left,
            left_span,
            right,
            right_span,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::Logical {
            left,
            left_span,
            operator,
            operator_span: _,
            right,
            right_span,
            truthiness_tests,
        } => {
            crate::checks::function::report_unreferenced_callable_conditions(
                truthiness_tests,
                symbols,
                ctx,
            );
            evaluate_logical(
                left,
                left_span,
                operator,
                right,
                right_span,
                fallback_span,
                symbols,
                ctx,
            )
        }
        ParsedExpression::Binary {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
        } => {
            let mut left_result =
                evaluate_expression(left, left_span.or(fallback_span), symbols, ctx);
            let mut right_result =
                evaluate_expression(right, right_span.or(fallback_span), symbols, ctx);
            if binary_operands_must_be_non_null(*operator, left, &left_result, right, &right_result)
            {
                for (operand, result, span) in [
                    (&**left, &mut left_result, left_span.or(fallback_span)),
                    (&**right, &mut right_result, right_span.or(fallback_span)),
                ] {
                    if let InferredExpression::Known(operand_type) = &*result
                        && let Some(non_null) =
                            super::check_non_null_operand(operand, operand_type, span, ctx)
                    {
                        *result = InferredExpression::Known(non_null);
                    }
                }
            }
            let binary_span = match (left_span, right_span) {
                (Some(left_span), Some(right_span)) => Some(SyntaxTextSpan {
                    start: left_span.start,
                    end: right_span.end,
                }),
                _ => fallback_span,
            };
            if let Some(always_false) = equality_result_when_unequal(*operator) {
                let strict = matches!(
                    operator,
                    surge_ts_syntax::ParsedBinaryOperator::StrictEquals | surge_ts_syntax::ParsedBinaryOperator::StrictNotEquals
                );
                report_reference_and_nan_equality(left, right, always_false, strict, binary_span, symbols, ctx);
            }
            if matches!(operator, surge_ts_syntax::ParsedBinaryOperator::In) {
                check_in_operands(
                    &left_result,
                    &right_result,
                    left_span.or(fallback_span),
                    right_span.or(fallback_span),
                    ctx,
                );
            }

            ops::evaluate_binary_expression(
                left_result,
                right_result,
                *operator,
                *left_span,
                *operator_span,
                *right_span,
                binary_span,
                ctx,
            )
        }
        ParsedExpression::Unary {
            operator,
            operand,
            operand_span,
            ..
        } => {
            let operand_result =
                evaluate_expression(operand, operand_span.or(fallback_span), symbols, ctx);
            if *operator == ParsedUnaryOperator::Not {
                report_void_truthiness(&operand_result, operand_span.or(fallback_span), ctx);
            }

            if matches!(
                operator,
                ParsedUnaryOperator::Plus | ParsedUnaryOperator::Minus | ParsedUnaryOperator::BitwiseNot
            ) && let InferredExpression::Known(operand_type) = &operand_result
            {
                check_numeric_unary_operand(
                    *operator,
                    operand,
                    operand_type,
                    operand_span.or(fallback_span),
                    ctx,
                );
            }
            if *operator == ParsedUnaryOperator::Delete {
                super::check_delete_operand(
                    operand,
                    operand_span.or(fallback_span),
                    symbols,
                    ctx,
                );
            }

            ops::evaluate_unary_expression(*operator, operand_result)
        }
        ParsedExpression::Update {
            operand,
            operand_span,
            rejected_target,
        } => {
            let operand_result = super::diagnostics::with_element_write_target(|| {
                evaluate_expression(operand, operand_span.or(fallback_span), symbols, ctx)
            });

            super::check_update_operand(
                operand,
                operand_span.or(fallback_span),
                *rejected_target,
                &operand_result,
                symbols,
                ctx,
            );

            super::update_result_type(&operand_result)
        }
        ParsedExpression::ArrayPatternElement {
            source,
            source_span,
            index,
            rest,
            tuple_literal,
            read,
            pattern_span,
        } => {
            let source_type = crate::infer::array_pattern_source_type(source, *tuple_literal, symbols, ctx);
            // `checkArrayLiteralAssignment` reads the source through
            // `checkIteratedTypeOrElementType` once per pattern: a source with
            // no iteration protocol is TS2488 at the pattern, and every element
            // it binds is `any`.
            if let InferredExpression::Known(source_type) = &source_type
                && super::is_definitely_not_iterable(source_type, true)
            {
                if let Some(pattern_span) = pattern_span {
                    let _ = evaluate_expression(source, source_span.or(fallback_span), symbols, ctx);
                    ctx.push(crate::spans::diagnostic_with_syntax_span(
                        Diagnostic::ts2488(source_type.name(), ctx.file_name.clone()),
                        Some(*pattern_span),
                    ));
                }
                return InferredExpression::Known(Type::Any);
            }
            if pattern_span.is_some()
                && let InferredExpression::Known(ty) = &source_type
                && !super::is_definitely_not_iterable(ty, true)
            {
                super::check_iteration_next_type(
                    &source_type,
                    *pattern_span,
                    &Type::Undefined,
                    super::IterationSend::ArrayDestructuring,
                    false,
                    ctx,
                );
            }
            let element = match &source_type {
                InferredExpression::Known(source_type) => super::array_pattern_element_type(
                    source_type,
                    *index,
                    *rest,
                    ctx.options.strict_builtin_iterator_return,
                ),
                _ if *rest => Some(Type::Unknown),
                _ => None,
            };
            match element {
                Some(element) => {
                    let _ = evaluate_expression(source, source_span.or(fallback_span), symbols, ctx);
                    InferredExpression::Known(element)
                }
                None => evaluate_expression(read, fallback_span, symbols, ctx),
            }
        }
        ParsedExpression::PatternExcessProperties(excess) => {
            for excess in excess {
                crate::checks::function::report_pattern_excess_property(excess, Some(symbols), ctx);
            }
            InferredExpression::Unknown
        }
        ParsedExpression::EmptyBindingPattern {
            pattern,
            source,
            source_span,
            declared_type,
        } => {
            let evaluated = evaluate_expression(source, source_span.or(fallback_span), symbols, ctx);
            let parent = match declared_type {
                Some(declared_type) => {
                    InferredExpression::Known(crate::infer::types::map_parsed_type(declared_type.clone(), ctx))
                }
                None => evaluated.clone(),
            };
            if let InferredExpression::Known(parent) = &parent {
                crate::infer::types::check_binding_pattern_reads(pattern, parent, ctx);
            }
            evaluated
        }
        ParsedExpression::ObjectRest {
            source,
            omitted,
            computed,
            name_span,
        } => match evaluate_expression(source, fallback_span, symbols, ctx) {
            InferredExpression::Known(ty) if crate::checks::function::rest_source_validity(&ty) == Some(false) => {
                ctx.push(crate::spans::diagnostic_with_syntax_span(
                    Diagnostic::ts2700(ctx.file_name.clone()),
                    name_span.or(fallback_span),
                ));
                InferredExpression::Known(Type::ErrorType)
            }
            InferredExpression::Known(ty) if computed.is_empty() => {
                InferredExpression::Known(crate::checks::function::object_rest_type(&ty, omitted))
            }
            InferredExpression::Known(ty) => {
                match crate::checks::function::computed_rest_omission(computed, symbols, ctx) {
                    Some(omission) => InferredExpression::Known(
                        crate::checks::function::object_rest_type_omitting(&ty, omitted, Some(&omission)),
                    ),
                    None => InferredExpression::Unknown,
                }
            }
            other => other,
        },
        ParsedExpression::Sequence { expressions } => {
            let mut result = InferredExpression::Unknown;
            for (expression, span) in expressions {
                result = evaluate_expression(expression, span.or(fallback_span), symbols, ctx);
            }
            result
        }
        ParsedExpression::Yield {
            operand,
            operand_span,
            delegate,
            span,
        } => evaluate_yield_expression(
            operand.as_deref(),
            operand_span.or(fallback_span),
            *delegate,
            span.or(fallback_span),
            symbols,
            ctx,
        ),
        ParsedExpression::Await {
            operand,
            operand_span,
        } => {
            let operand_result =
                evaluate_expression(operand, operand_span.or(fallback_span), symbols, ctx);

            match operand_result {
                InferredExpression::Known(ty) => {
                    InferredExpression::Known(crate::checks::call::awaited_type(&ty))
                }
                other => other,
            }
        }
        ParsedExpression::ImportCall {
            specifier,
            specifier_span,
            options,
            options_span,
            span,
        } => evaluate_import_call(
            specifier,
            specifier_span.or(fallback_span),
            options
                .as_deref()
                .map(|options| (options, options_span.or(fallback_span))),
            span.or(fallback_span),
            symbols,
            ctx,
        ),
        ParsedExpression::Conditional {
            condition,
            condition_span,
            when_true,
            when_true_span,
            when_false,
            when_false_span,
            truthiness_tests,
        } => {
            crate::checks::function::report_unreferenced_callable_conditions(
                truthiness_tests,
                symbols,
                ctx,
            );
            evaluate_conditional(
            condition,
            condition_span,
            when_true,
            when_true_span,
            when_false,
            when_false_span,
            fallback_span,
            symbols,
            ctx,
        )
        }
        ParsedExpression::OptionalPropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
        } => evaluate_optional_property_access(
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
            expression,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::OptionalIndexAccess {
            object,
            object_span,
            index,
            index_span,
        } => evaluate_optional_index_access(
            object,
            *object_span,
            index,
            *index_span,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::IndexAccess {
            object_name,
            object_span,
            index,
            index_span,
        } => evaluate_index_access(
            object_name,
            *object_span,
            index,
            *index_span,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::SatisfiesExpression {
            expression: satisfied_expression,
            span,
            target_type,
            target_span,
            keyword_span,
        } => evaluate_satisfies_expression(
            satisfied_expression,
            span,
            target_type,
            keyword_span.or(*target_span),
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::TypeAssertion {
            expression: asserted_expression,
            expression_span,
            ty,
            type_span: pattern_span,
            annotation: true,
        } => {
            // A destructuring declaration's annotation, or a JSDoc-typed
            // assignment declaration's: the value is checked against it like
            // any declared type, at the pattern or the declaration, and the
            // reads see the declared type itself.
            let declared = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                crate::infer::map_parsed_type(ty.clone(), ctx)
            });
            // A `require` value names no declaration and is reported at
            // itself.
            let Some(pattern_span) = *pattern_span else {
                let _ = crate::checks::expected::evaluate_expression_with_expected_type(
                    asserted_expression,
                    expression_span.or(fallback_span),
                    Some(&declared),
                    crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
                    symbols,
                    ctx,
                );
                return InferredExpression::Known(declared);
            };
            let checked = crate::checks::expected::evaluate_expression_with_expected_type_anchored(
                asserted_expression,
                expression_span.or(fallback_span),
                Some(pattern_span),
                Some(&declared),
                crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
                symbols,
                ctx,
            );
            if let InferredExpression::Known(initializer_type) = &checked {
                crate::checks::var::report_initializer_mismatch(initializer_type, &declared, Some(pattern_span), ctx);
            }
            InferredExpression::Known(declared)
        }
        ParsedExpression::TypeAssertion {
            expression: asserted_expression,
            expression_span,
            ty,
            type_span: _,
            annotation: false,
        } => evaluate_type_assertion(
            asserted_expression,
            expression_span,
            ty,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::ConstAssertion {
            expression: asserted_expression,
            span: expression_span,
        } => evaluate_const_expression(
            asserted_expression,
            expression_span.or(fallback_span),
            symbols,
            ctx,
        ),
        ParsedExpression::ArrowFunction(arrow_function) => {
            let function_type = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                check_arrow_function_expression(arrow_function.as_ref().clone(), symbols, ctx)
            });
            let function_type =
                crate::infer::expression::with_written_predicate(function_type, arrow_function, ctx);
            InferredExpression::Known(Type::Function(function_type))
        }
        ParsedExpression::ClassExpression(class_expression) => InferredExpression::Known(
            crate::program::check_class_expression(class_expression, None, symbols, ctx),
        ),
        ParsedExpression::Instantiation {
            expression: instantiated,
            expression_span,
            type_arguments,
            type_arguments_span,
        } => crate::checks::call::evaluate_instantiation_expression(
            instantiated,
            expression_span.or(fallback_span),
            type_arguments,
            *type_arguments_span,
            symbols,
            ctx,
        ),
        ParsedExpression::NonNullAssertion {
            expression: asserted_expression,
            span: expression_span,
            in_optional_chain,
        } => evaluate_non_null_assertion(
            asserted_expression,
            expression_span,
            in_optional_chain,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::JsxElement {
            tag_name,
            attributes,
            children,
            tag,
            ..
        } => {
            crate::checks::jsx::check_jsx_preconditions(tag.span, fallback_span, ctx);
            crate::checks::jsx::check_jsx_factory_reference(
                false,
                tag.name_span,
                fallback_span,
                symbols,
                ctx,
            );
            crate::checks::jsx::check_jsx_element(
                tag_name,
                tag,
                attributes,
                children,
                fallback_span,
                symbols,
                ctx,
            );

            infer_expression(expression, symbols, ctx)
        }
        ParsedExpression::JsxFragment { children, span } => {
            crate::checks::jsx::check_jsx_preconditions(*span, fallback_span, ctx);
            crate::checks::jsx::check_jsx_factory_reference(true, *span, fallback_span, symbols, ctx);
            crate::checks::jsx::check_jsx_fragment_factory(*span, fallback_span, ctx);
            for child in children {
                evaluate_jsx_child(child, fallback_span, symbols, ctx);
            }

            infer_expression(expression, symbols, ctx)
        }
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
            binding_element,
        } => evaluate_property_access(
            object,
            object_span,
            property_name,
            property_span,
            is_bracketed,
            *binding_element,
            expression,
            fallback_span,
            symbols,
            ctx,
        ),
        ParsedExpression::ElementAccess {
            object,
            object_span,
            index,
            index_span,
        } => {
            // `checkElementAccessExpression` checks the key against whatever the
            // receiver is; `this` answers from the binding a method body has.
            if matches!(object.as_ref(), ParsedExpression::This { .. }) && symbols.get("this").is_some() {
                return evaluate_index_access(
                    "this",
                    *object_span,
                    index,
                    *index_span,
                    fallback_span,
                    symbols,
                    ctx,
                );
            }
            // A write target's receiver is read.
            let _ = super::diagnostics::take_element_write_target();
            let receiver = evaluate_expression(object, object_span.or(fallback_span), symbols, ctx);
            check_property_receiver(object, &receiver, *object_span, fallback_span, symbols, ctx);
            if let InferredExpression::Known(receiver_type) = &receiver
                && let Type::Tuple(elements) = receiver_type.peeled()
                && let ParsedExpression::NumberLiteral(value) = index.as_ref()
                && let Ok(index_value) = value.parse::<i64>()
                && usize::try_from(index_value).map_or(true, |index| index >= elements.len())
            {
                super::index_access::report_tuple_index_out_of_bounds(
                    &elements,
                    index_value,
                    index_span.or(*object_span).or(fallback_span),
                    ctx,
                );
                return InferredExpression::Known(Type::Undefined);
            }
            // tsc checks the index whatever the receiver turned out to be
            // (`checkElementAccessExpression`).
            if let InferredExpression::Known(index_type) = evaluate_expression(
                index,
                index_span.or(*object_span).or(fallback_span),
                symbols,
                ctx,
            ) && super::index_access::report_unusable_index_type(
                &index_type,
                index_span.or(*object_span).or(fallback_span),
                ctx,
            ) {
                return InferredExpression::Unknown;
            }
            let inferred_expression = infer_expression(expression, symbols, ctx);
            report_inferred_expression(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                    inferred_expression.clone()
                }),
                fallback_span,
                symbols,
                ctx,
            );
            inferred_expression
        }
        // An assignment used as a value is checked like the statement form; its
        // effect on the binding is applied by the enclosing statement.
        ParsedExpression::Assignment {
            target_name,
            target_span,
            value,
            value_span,
        } => {
            let assignment = surge_ts_syntax::ParsedAssignment {
                target_name: target_name.clone(),
                target_span: *target_span,
                written_target_span: *target_span,
                value: value.as_ref().clone(),
                value_span: *value_span,
                from_pattern: false,
            };
            let shadowed_locally = symbols.get_own(target_name).is_some();
            if !crate::checks::assign::check_assignment_with_symbols(
                assignment,
                symbols,
                shadowed_locally,
                ctx,
            ) {
                // tsc checks the value whatever the target turned out to be
                // (`checkBinaryLikeExpression`); a rejected target lends it no
                // contextual type.
                return evaluate_expression(value, *value_span, symbols, ctx);
            }
            infer_expression(value, symbols, ctx)
        }
        // A member write used as a value is checked as the statement form is,
        // in a scope of its own: what the write narrows is not seen after it.
        ParsedExpression::MemberAssignment(assignment) => {
            let mut scopes = crate::symbols::ScopeStack::from_root(
                symbols.clone_with_reason(TypeCopyReason::ScopeOrContext),
            );
            crate::checks::function::check_member_assignment(
                assignment.as_ref().clone(),
                &mut scopes,
                ctx,
            );
            infer_expression(&assignment.value, symbols, ctx)
        }
        _ => {
            if let ParsedExpression::Identifier { name, span } = expression
                && let Some(read) = super::check_auto_array_read(name, *span, symbols, ctx)
            {
                return InferredExpression::Known(read);
            }
            let inferred_expression = infer_expression(expression, symbols, ctx);
            // tsc's name resolver runs `checkAndReportErrorForInvalidInitializer`
            // for a property-initializer read of a constructor local whether or
            // not the name also resolves further out, and answers no symbol. A
            // binding the initializer made itself (an arrow's parameter) is
            // found first and is not that read.
            if let ParsedExpression::Identifier {
                name,
                span: Some(span),
            } = expression
                && matches!(inferred_expression, InferredExpression::Known(_))
                && ctx
                    .constructor_local_outer_bindings
                    .iter()
                    .any(|(local, outer)| {
                        local.as_ref() == name.as_str()
                            && match (outer, symbols.get_handle(name)) {
                                (Some(outer), Some(found)) => std::sync::Arc::ptr_eq(outer, &found),
                                (None, None) => true,
                                _ => false,
                            }
                    })
                && let Some(property) =
                    crate::program::property_with_invalid_initializer(name, *span, ctx)
            {
                let diagnostic = property.invalid_reference_diagnostic(name, *span, ctx.file_name.clone());
                ctx.push_utility_diagnostic_once(crate::spans::diagnostic_with_syntax_span(
                    diagnostic,
                    Some(*span),
                ));
                return InferredExpression::Known(Type::ErrorType);
            }
            report_inferred_expression(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                    inferred_expression.clone()
                }),
                fallback_span,
                symbols,
                ctx,
            );
            inferred_expression
        }
    }
}

fn evaluate_nullish_coalescing(
    left: &Box<ParsedExpression>,
    left_span: &Option<SyntaxTextSpan>,
    right: &Box<ParsedExpression>,
    right_span: &Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let left_result = evaluate_expression(left, left_span.or(fallback_span), symbols, ctx);
    // The right operand runs only when the left is nullish, which narrows
    // the left's reference there as `left == null` would.
    let nullish_test = (!left.contains_assignment()).then(|| ParsedExpression::Binary {
        left: left.clone(),
        left_span: *left_span,
        operator: surge_ts_syntax::ParsedBinaryOperator::Equals,
        operator_span: None,
        right: Box::new(ParsedExpression::NullLiteral),
        right_span: None,
    });
    let right_symbols = nullish_test
        .as_ref()
        .and_then(|test| crate::checks::function::narrow_truthy_operand_symbol_table(test, symbols));
    let symbols = right_symbols.as_ref().unwrap_or(symbols);
    // With no outer contextual type, tsc contextually types the right
    // operand from the left's — that is what gives `opts.uri ?? ((id) =>
    // id)`'s parameter a type instead of a false TS7006. It is a contextual
    // type only (`getContextualTypeForBinaryOperand`): the operand is not
    // required to fit it.
    let right_result = match contextual_default_operand_type(&left_result) {
        Some(contextual) => match empty_object_fallback_type(right, &contextual) {
            Some(fallback) => InferredExpression::Known(fallback),
            None => crate::checks::expected::evaluate_expression_with_expected_type(
                right,
                right_span.or(fallback_span),
                Some(&contextual),
                crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
                symbols,
                ctx,
            ),
        },
        None => evaluate_expression(right, right_span.or(fallback_span), symbols, ctx),
    };

    match (left_result, right_result) {
        (InferredExpression::Known(left_type), InferredExpression::Known(right_type)) => {
            InferredExpression::Known(crate::infer::expression::nullish_coalescing_result(
                left_type, right_type,
            ))
        }
        (
            InferredExpression::Known(Type::Unknown)
            | InferredExpression::Known(Type::GenuineUnknown)
            | InferredExpression::Unknown,
            _,
        )
        | (
            _,
            InferredExpression::Known(Type::Unknown)
            | InferredExpression::Known(Type::GenuineUnknown)
            | InferredExpression::Unknown,
        ) => InferredExpression::Unknown,
        _ => InferredExpression::Unknown,
    }
}

/// tsc's `checkSpreadPropOverrides`: a property written before a spread whose
/// type always has it (neither optional nor missing from some union member)
/// is overwritten — TS2783, under `strictNullChecks` only.
pub(crate) fn report_overwritten_properties(
    spread: &InferredExpression,
    explicit_properties: &[(&str, Option<SyntaxTextSpan>)],
    ctx: &mut CheckerContext,
) {
    if !ctx.options.strict_null_checks || explicit_properties.is_empty() {
        return;
    }
    let InferredExpression::Known(ty) = spread else {
        return;
    };
    // A type parameter spreads its constraint's properties (tsc reads them
    // through `getPropertiesOfType`, which goes to the apparent type).
    let constrained;
    let ty = match ty {
        Type::TypeParameter(parameter) => match ctx.type_parameter_constraint(&parameter.name).cloned() {
            Some(constraint) => {
                constrained = crate::infer::map_parsed_type(constraint, ctx);
                &constrained
            }
            None => return,
        },
        other => other,
    };
    for (name, span) in explicit_properties {
        if let Some(span) = span
            && spread_always_writes(ty, name)
        {
            let diagnostic = Diagnostic::ts2783(name, ctx.file_name.clone());
            ctx.push(diagnostic.with_span(crate::context::convert_span(*span)));
        }
    }
}

pub(crate) fn spread_always_writes(ty: &Type, name: &str) -> bool {
    match ty {
        Type::Object(object) => object
            .properties
            .get(name)
            .is_some_and(|property| !property.optional),
        Type::Reference(reference) => spread_always_writes(&reference.resolve(), name),
        Type::Union(union) => union.types().iter().all(|member| spread_always_writes(member, name)),
        _ => false,
    }
}

/// tsc's `checkTruthinessOfType` for a `void` operand — TS1345.
pub(crate) fn report_void_truthiness(
    result: &InferredExpression,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if let InferredExpression::Known(Type::Void) = result
        && let Some(span) = span
    {
        let diagnostic = Diagnostic::ts1345(ctx.file_name.clone());
        ctx.push(diagnostic.with_span(crate::context::convert_span(span)));
    }
}

fn evaluate_logical(
    left: &Box<ParsedExpression>,
    left_span: &Option<SyntaxTextSpan>,
    operator: &ParsedLogicalOperator,
    right: &Box<ParsedExpression>,
    right_span: &Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    evaluate_logical_in_context(
        left,
        left_span,
        operator,
        right,
        right_span,
        fallback_span,
        None,
        symbols,
        ctx,
    )
}

/// `a && b` / `a || b` where `contextual` is the whole expression's contextual
/// type, which tsc's `getContextualTypeForBinaryOperand` hands the right
/// operand of `&&` (as context only: the result also carries the left's falsy
/// part).
#[allow(clippy::too_many_arguments)]
/// The scope the right operand of `a && b` or `a || b` is checked in: after
/// the left's assignments, and narrowed by the left holding (`&&`) or failing
/// (`||`).
pub(crate) fn logical_right_operand_symbols(
    left: &ParsedExpression,
    operator: &ParsedLogicalOperator,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    // `a && b && c` reaches `c` through `a` and then `b` (`bindLogicalLikeExpression`),
    // so each operand narrows what the one before it left: `isU(x) && x.kind
    // === "a" && x.a` discriminates the union the predicate proved.
    if let ParsedExpression::Logical {
        left: first,
        operator: inner,
        right: second,
        ..
    } = left
        && inner == operator
    {
        let after_first = logical_right_operand_symbols(first, operator, symbols, ctx);
        let base = after_first.as_ref().unwrap_or(symbols);
        return logical_right_operand_symbols(second, operator, base, ctx).or(after_first);
    }
    // The right operand runs after the left's assignments, and is narrowed by
    // what they assigned: `(next = it.next()) && !next.done`.
    let after_assignments = symbols_after_assignments(left, symbols, ctx);
    let symbols = after_assignments.as_ref().unwrap_or(symbols);
    let tested = left
        .contains_assignment()
        .then(|| left.with_assignments_as_reads());
    let left: &ParsedExpression = tested.as_ref().unwrap_or(left);
    // `a && b` only evaluates `b` when `a` is truthy, so narrow `b` by the
    // `a` guard: a structured guard (`x.kind === "k" && x.k`, `"p" in x &&
    // x.p`) plus each identifier/property the `&&` chain proves non-nullish
    // (`a.b && a.b > c`).
    let narrowed = match operator {
        surge_ts_syntax::ParsedLogicalOperator::And => {
            let guarded =
                crate::checks::function::narrow_truthy_operand_symbol_table(left, symbols);
            // That path only knows the syntactic guards; a user-defined
            // predicate call in the chain still has to stop its subject
            // from reading as a genuine-unknown receiver on the right.
            let downgraded = downgrade_predicate_guarded_genuine_unknown(
                left,
                guarded.as_ref().unwrap_or(symbols),
            );
            downgraded.or(guarded)
        }
        // `a || b` only evaluates `b` when `a` is falsy, so the guards
        // `a`'s falsity proves hold on the right (`typeof x !== "object"
        // || !("p" in x) || typeof x.p !== "string"`).
        surge_ts_syntax::ParsedLogicalOperator::Or => {
            crate::checks::function::narrow_falsy_operand_symbol_table(left, symbols)
        }
    };
    // A user-defined predicate in the chain narrows its subject for the
    // right operand too (`ts.isImportDeclaration(n) && n.moduleSpecifier`).
    // `a || b` only evaluates `b` when `a` is falsy, so a negated
    // predicate narrows there the same way (`!isA(x) || x.av`).
    let predicate_branch = match operator {
        surge_ts_syntax::ParsedLogicalOperator::And => true,
        surge_ts_syntax::ParsedLogicalOperator::Or => false,
    };
    let narrowed = crate::checks::function::narrow_predicate_guards_symbol_table(
        left,
        narrowed.as_ref().unwrap_or(symbols),
        predicate_branch,
        ctx,
    )
    .or(narrowed);
    // A guard on an element access (`xs[i] && xs[i].x`) narrows the
    // access itself rather than any binding.
    let narrowed = crate::checks::function::narrow_element_reference_guards_symbol_table(
        left,
        predicate_branch,
        narrowed.as_ref().unwrap_or(symbols),
        ctx,
    )
    .or(narrowed);
    narrowed.or(after_assignments)
}

pub(crate) fn evaluate_logical_in_context(
    left: &Box<ParsedExpression>,
    left_span: &Option<SyntaxTextSpan>,
    operator: &ParsedLogicalOperator,
    right: &Box<ParsedExpression>,
    right_span: &Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    contextual: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let left_result = evaluate_expression(left, left_span.or(fallback_span), symbols, ctx);
    report_void_truthiness(&left_result, left_span.or(fallback_span), ctx);
    let right_symbols = logical_right_operand_symbols(left, operator, symbols, ctx);
    let right_symbols = right_symbols.as_ref().unwrap_or(symbols);
    // `a || b` hands `b` the same contextual type `a ?? b` does.
    let (right_contextual, left_typed) = match operator {
        surge_ts_syntax::ParsedLogicalOperator::Or => {
            (contextual_default_operand_type(&left_result), true)
        }
        surge_ts_syntax::ParsedLogicalOperator::And => (contextual.cloned(), false),
    };
    let right_result = match right_contextual {
        Some(contextual) => match left_typed
            .then(|| empty_object_fallback_type(right, &contextual))
            .flatten()
        {
            Some(fallback) => InferredExpression::Known(fallback),
            None => crate::checks::expected::evaluate_expression_with_expected_type(
                right,
                right_span.or(fallback_span),
                Some(&contextual),
                crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
                right_symbols,
                ctx,
            ),
        },
        None => evaluate_expression(right, right_span.or(fallback_span), right_symbols, ctx),
    };

    ops::evaluate_logical_expression(*operator, left_result, right_result)
}

fn evaluate_conditional(
    condition: &Box<ParsedExpression>,
    condition_span: &Option<SyntaxTextSpan>,
    when_true: &Box<ParsedExpression>,
    when_true_span: &Option<SyntaxTextSpan>,
    when_false: &Box<ParsedExpression>,
    when_false_span: &Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let condition_result =
        evaluate_expression(condition, condition_span.or(fallback_span), symbols, ctx);
    report_void_truthiness(&condition_result, condition_span.or(fallback_span), ctx);
    // Narrow a discriminated union per branch so `x.kind === "a" ? x.a :
    // x.b` checks `x.a` against the `"a"` member only.
    let true_symbols =
        crate::checks::function::narrow_condition_symbol_table(condition, symbols, true);
    // The branch where the guard holds also sees a guarded `unknown` as
    // narrowed, exactly as the `if` form does.
    let true_symbols = downgrade_guarded_genuine_unknown(
        condition,
        true_symbols.as_ref().unwrap_or(symbols),
        true,
    )
    .or(true_symbols);
    let true_symbols = crate::checks::function::narrow_predicate_guards_symbol_table(
        condition,
        true_symbols.as_ref().unwrap_or(symbols),
        true,
        ctx,
    )
    .or(true_symbols);
    let true_symbols = crate::checks::function::narrow_element_reference_guards_symbol_table(
        condition,
        true,
        true_symbols.as_ref().unwrap_or(symbols),
        ctx,
    )
    .or(true_symbols);
    let false_symbols =
        crate::checks::function::narrow_condition_symbol_table(condition, symbols, false);
    let false_symbols = downgrade_guarded_genuine_unknown(
        condition,
        false_symbols.as_ref().unwrap_or(symbols),
        false,
    )
    .or(false_symbols);
    let false_symbols = crate::checks::function::narrow_predicate_guards_symbol_table(
        condition,
        false_symbols.as_ref().unwrap_or(symbols),
        false,
        ctx,
    )
    .or(false_symbols);
    let false_symbols = crate::checks::function::narrow_element_reference_guards_symbol_table(
        condition,
        false,
        false_symbols.as_ref().unwrap_or(symbols),
        ctx,
    )
    .or(false_symbols);
    let true_result = evaluate_expression(
        when_true,
        when_true_span.or(fallback_span),
        true_symbols.as_ref().unwrap_or(symbols),
        ctx,
    );
    let false_result = evaluate_expression(
        when_false,
        when_false_span.or(fallback_span),
        false_symbols.as_ref().unwrap_or(symbols),
        ctx,
    );

    let branch_types = [
        (when_true.as_ref(), ops::inferred_type(&true_result).cloned()),
        (when_false.as_ref(), ops::inferred_type(&false_result).cloned()),
    ];
    subtype_reduced_conditional(
        ops::evaluate_conditional_expression(condition_result, true_result, false_result),
        branch_types,
    )
}

/// tsc's `checkConditionalExpression` unions the branch types with subtype
/// reduction (`getUnionTypeEx(…, UnionReductionSubtype)`), so a branch whose
/// type is a strict subtype of the other's is absorbed: `c ? () => {} : (x?:
/// string) => {}` is `(x?: string) => void`.
pub(crate) fn subtype_reduced_conditional(
    result: InferredExpression,
    branches: [(&ParsedExpression, Option<Type>); 2],
) -> InferredExpression {
    if !matches!(result, InferredExpression::Known(Type::Union(_))) {
        return result;
    }
    let [(when_true, Some(true_type)), (when_false, Some(false_type))] = branches else {
        return result;
    };
    InferredExpression::Known(surge_ts_types::subtype_reduced_union(vec![
        (true_type, literal_shape(when_true)),
        (false_type, literal_shape(when_false)),
    ]))
}

/// Which part of an expression's type is an object literal's own, fresh type
/// (tsc's `ObjectFlagsObjectLiteral`), which surge's types do not record. A
/// literal that ends up inside a union or an array's element type, or behind a
/// function's inferred return, cannot be lined up with the type and is opaque.
pub(crate) fn literal_shape(expression: &ParsedExpression) -> surge_ts_types::LiteralShape {
    use surge_ts_types::LiteralShape;
    let regular = |shape: &LiteralShape| matches!(shape, LiteralShape::Regular);
    match expression {
        ParsedExpression::ObjectLiteral { properties, .. } => LiteralShape::Object(
            properties
                .iter()
                .filter(|property| {
                    !property.is_spread
                        && !property.is_method
                        && !property.is_accessor
                        && property.computed_key.is_none()
                        && property.unnamed_key_value.is_none()
                })
                .map(|property| (std::sync::Arc::from(property.name.as_str()), literal_shape(&property.value)))
                .collect(),
        ),
        ParsedExpression::SatisfiesExpression { expression, .. }
        | ParsedExpression::NonNullAssertion { expression, .. }
        | ParsedExpression::Await { operand: expression, .. }
        | ParsedExpression::Assignment { value: expression, .. } => literal_shape(expression),
        ParsedExpression::MemberAssignment(assignment) => literal_shape(&assignment.value),
        ParsedExpression::Sequence { expressions } => expressions
            .last()
            .map_or(LiteralShape::Regular, |(expression, _)| literal_shape(expression)),
        ParsedExpression::ConstAssertion { expression, .. } if !regular(&literal_shape(expression)) => {
            LiteralShape::Opaque
        }
        ParsedExpression::ArrayLiteral { elements, .. }
            if elements.iter().any(|element| !regular(&literal_shape(&element.expression))) =>
        {
            LiteralShape::Opaque
        }
        ParsedExpression::Conditional { when_true: left, when_false: right, .. }
        | ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. }
            if !regular(&literal_shape(left)) || !regular(&literal_shape(right)) =>
        {
            LiteralShape::Opaque
        }
        ParsedExpression::ArrowFunction(function) if function.return_type.is_none() => LiteralShape::Opaque,
        _ => LiteralShape::Regular,
    }
}

/// tsc's `checkYieldExpression`: in a generator with a return type
/// annotation, a `yield` operand is contextually typed by the declared yield
/// type and must be assignable to it (an async generator's once awaited; a
/// bare `yield` yields `undefined`, a `yield*` what its operand iterates).
/// Outside a generator the operand is not checked. What the expression
/// produces is the declared next type (`any` without an annotation), and for a
/// `yield*` what its operand's iterator returns.
fn evaluate_yield_expression(
    operand: Option<&ParsedExpression>,
    operand_span: Option<SyntaxTextSpan>,
    delegate: bool,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if !ctx.in_generator_function {
        return InferredExpression::Known(Type::Any);
    }
    if let Some(span) = span
        && ctx.implicit_any_yields.contains(&span.start)
    {
        ctx.push(diagnostic_with_syntax_span(Diagnostic::ts7057(ctx.file_name.clone()), Some(span)));
    }
    let yield_type = ctx
        .generator_yield_type
        .clone()
        .filter(|ty| !matches!(ty, Type::Any) && !ty.is_unmodelled());
    // tsc still types an operand from the generator's contextual return type
    // (and a `yield*` operand's elements from the yield type), which surge
    // does not model; the mismatch of what it yields is related below.
    let expected = match &yield_type {
        Some(yield_type) if !delegate && !ctx.in_async_body => yield_type.clone(),
        _ => Type::Unknown,
    };
    // A `yield` evaluates to the annotated generator's next type
    // (`getIterationTypeOfGeneratorFunctionReturnType(Next)`), `any` without one.
    let next_type = ctx
        .generator_next_type
        .clone()
        .filter(is_modelled_iteration_type)
        .unwrap_or(Type::Any);
    let Some(operand) = operand else {
        if let Some(yield_type) = yield_type
            && surge_ts_types::strict_null_checks()
            && !is_assignable_to(&Type::Undefined, &yield_type)
        {
            let diagnostic = Diagnostic::ts2322("undefined", yield_type.name(), ctx.file_name.clone());
            ctx.push(diagnostic_with_syntax_span(diagnostic, span));
        }
        crate::checks::function::record_generator_yield(Some(Type::Undefined), None);
        return InferredExpression::Known(next_type);
    };
    let evaluated = crate::checks::expected::evaluate_expression_with_expected_type(
        operand,
        operand_span,
        Some(&expected),
        crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
        symbols,
        ctx,
    );
    // `getYieldedTypeOfYieldExpression` iterates a `yield*` operand whether or
    // not the generator is annotated; an async generator's may be async or
    // sync iterable.
    if delegate {
        if ctx.in_async_generator {
            super::check_async_iterable_operand(&evaluated, operand_span, false, ctx);
        } else if ctx.ambient_global_type_declarations.get("Iterable").is_some() {
            super::check_iterable_operand(&evaluated, operand_span, true, ctx);
        }
        // What the delegate is resumed with is what this generator is; with
        // no annotation that is `any`, which every `next` accepts.
        if let Some(sent) = ctx.generator_next_type.clone().filter(is_modelled_iteration_type) {
            let allow_async = ctx.in_async_generator;
            super::check_iteration_next_type(
                &evaluated,
                operand_span,
                &sent,
                super::IterationSend::YieldStar,
                allow_async,
                ctx,
            );
        }
    }
    record_yielded_type(operand, &evaluated, delegate, ctx);
    // A `yield*` evaluates to what its operand's iterator returns
    // (`getIterationTypeOfIterable(Return)`), annotated generator or not.
    let delegated_return = match &evaluated {
        InferredExpression::Known(operand_type) if delegate => {
            crate::checks::function::generator_return_type_argument(operand_type)
                .filter(is_modelled_iteration_type)
                .map(|ty| if ctx.in_async_body { crate::checks::call::awaited_type(&ty) } else { ty })
                .or_else(|| {
                    let strict = ctx.options.strict_builtin_iterator_return;
                    let usage = if ctx.in_async_generator {
                        super::IterationUse::async_generator_delegate(strict)
                    } else {
                        super::IterationUse::sync(strict)
                    };
                    super::iteration_types_of_iterable(operand_type, usage)
                        .map(|types| types.return_type)
                        .filter(|ty| is_modelled_iteration_type(ty) && !ty.is_unmodelled())
                })
        }
        _ => None,
    };
    if let (Some(yield_type), InferredExpression::Known(operand_type)) = (yield_type, evaluated) {
        let yielded = if delegate { iterated_element_type(&operand_type) } else { Some(operand_type) };
        if let Some(yielded) = yielded {
            let yielded = if ctx.in_async_body { crate::checks::call::awaited_type(&yielded) } else { yielded };
            crate::checks::var::report_initializer_mismatch(&yielded, &yield_type, operand_span, ctx);
        }
    }
    if delegate {
        return InferredExpression::Known(delegated_return.unwrap_or(Type::Any));
    }
    InferredExpression::Known(next_type)
}

/// tsc's `getYieldedTypeOfYieldExpression` for the generator body collecting
/// its yields: a `yield*` yields its operand's elements, and an async
/// generator yields awaited values. A fresh object or array literal widens
/// as the aggregate's `getWidenedType` widens it.
fn record_yielded_type(
    operand: &ParsedExpression,
    evaluated: &InferredExpression,
    delegate: bool,
    ctx: &CheckerContext,
) {
    let InferredExpression::Known(operand_type) = evaluated else {
        crate::checks::function::record_generator_yield(None, None);
        return;
    };
    let yielded = if delegate {
        iterated_element_type(operand_type)
    } else if matches!(operand, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }) {
        Some(super::widen_type(operand_type))
    } else {
        Some(operand_type.clone())
    };
    let yielded = yielded
        .map(|ty| if ctx.in_async_generator { crate::checks::call::awaited_type(&ty) } else { ty })
        .filter(|ty| !ty.is_unmodelled());
    let sent = delegate.then(|| {
        crate::checks::function::generator_next_type_argument(operand_type).unwrap_or(Type::GenuineUnknown)
    });
    crate::checks::function::record_generator_yield(yielded, sent);
}

/// An iteration type argument surge can hand on as a value's type: not the
/// degradation sentinel, an error, or a type parameter. A written `unknown`
/// is kept.
fn is_modelled_iteration_type(ty: &Type) -> bool {
    !matches!(ty, Type::Unknown | Type::ErrorType | Type::TypeParameter(_))
}

/// tsc's `checkImportCallExpression`: the specifier must be a `string` — a
/// bare `null` or `undefined` never is (TS7036) — the options an
/// `ImportCallOptions`, and the call needs the global `Promise` constructor
/// (`createPromiseReturnType`, TS2712). The call's own type is not modelled.
fn evaluate_import_call(
    specifier: &ParsedExpression,
    specifier_span: Option<SyntaxTextSpan>,
    options: Option<(&ParsedExpression, Option<SyntaxTextSpan>)>,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let specifier_result = evaluate_expression(specifier, specifier_span, symbols, ctx);
    let options_result = options.map(|(options, options_span)| {
        (evaluate_expression(options, options_span, symbols, ctx), options_span)
    });
    if let InferredExpression::Known(specifier_type) = &specifier_result
        && (matches!(specifier_type, Type::Undefined | Type::Null)
            || !(specifier_type.is_unmodelled()
                || crate::checks::assign::type_contains_unknown(specifier_type)
                || is_assignable_to(specifier_type, &Type::String)))
    {
        let diagnostic = Diagnostic::ts7036(specifier_type.name(), ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, specifier_span));
    }
    if let Some((InferredExpression::Known(options_type), options_span)) = &options_result
        && ctx.lookup_type_declaration("ImportCallOptions").is_some()
    {
        let import_call_options = crate::infer::map_parsed_type(
            ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: "ImportCallOptions".to_string(),
                span: None,
                type_arguments: Vec::new(),
            })),
            ctx,
        );
        let target = union_type(vec![import_call_options, Type::Undefined]);
        crate::checks::var::report_initializer_mismatch(options_type, &target, *options_span, ctx);
    }
    // `getGlobalPromiseConstructorSymbol`: the lib declares the `Promise`
    // interface from ES5 on, and its constructor from ES2015.
    if ctx.lookup_type_declaration("Promise").is_some()
        && symbols.get("Promise").is_none()
        && ctx.ambient_global_symbols.get("Promise").is_none()
    {
        let diagnostic = Diagnostic::ts2712(ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, span));
    }
    // `checkImportCallExpression`: a promise of the resolved module's type.
    // An `export =` module also gains a synthetic `default`
    // (`getTypeWithSyntheticDefaultImportType`), which is not modelled here.
    if let ParsedExpression::StringLiteral(specifier) = specifier
        && let Some(namespace) = import_call_module_namespace(specifier, ctx)
    {
        return InferredExpression::Known(crate::checks::call::promise_of(&namespace, ctx));
    }
    InferredExpression::Unknown
}

pub(crate) fn import_call_module_namespace(specifier: &str, ctx: &mut CheckerContext) -> Option<Type> {
    let files = [std::sync::Arc::from(ctx.file_name.as_str()), ctx.canonical_file_name_arc()];
    let namespaces = ctx.import_type_namespaces.lock().ok()?;
    let target = files
        .into_iter()
        .find_map(|file| namespaces.get(&(file, specifier.to_string())))?;
    (!target.export_assignment).then(|| target.namespace.clone())
}

/// The element type a `yield*` operand iterates: an array's or tuple's
/// elements, or the yield type argument of a lib iterable or generator.
pub(crate) fn iterated_element_type(iterable: &Type) -> Option<Type> {
    match iterable {
        Type::Array(element) => Some((**element).clone()),
        Type::Tuple(elements) => Some(union_type(elements.clone())),
        Type::Reference(_) => crate::checks::function::generator_yield_type_argument(iterable)
            .or_else(|| structural_yield_type(iterable)),
        Type::Object(_) => structural_yield_type(iterable),
        _ => None,
    }
}

fn structural_yield_type(iterable: &Type) -> Option<Type> {
    super::iteration_types_of_iterable(iterable, super::IterationUse::sync(false))
        .map(|types| types.yield_type)
        .filter(|ty| !ty.is_unmodelled())
}

fn evaluate_optional_property_access(
    object: &Box<ParsedExpression>,
    object_span: &Option<SyntaxTextSpan>,
    property_name: &String,
    property_span: &Option<SyntaxTextSpan>,
    is_bracketed: &bool,
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let is_write = super::diagnostics::take_element_write_target();
    if property_name == "length" && !is_bracketed {
        super::mark_evolving_array_operation(object, ctx);
    }
    let receiver = evaluate_expression(object, object_span.or(fallback_span), symbols, ctx);
    // An optional chain skips `checkNonNullType`; the member is looked up on
    // `unknown` with `null` and `undefined` removed, which is `{}` — or on
    // `unknown` itself when those are in every type's domain.
    if !*is_bracketed && matches!(receiver, InferredExpression::Known(Type::GenuineUnknown)) {
        let receiver_name = if surge_ts_types::strict_null_checks() {
            "{}"
        } else {
            "unknown"
        };
        report_property_of_unknown(property_name, receiver_name, *property_span, ctx);
        return InferredExpression::Unknown;
    }
    if !*is_bracketed && let InferredExpression::Known(receiver_type) = &receiver {
        check_member_accessibility(
            object,
            receiver_type,
            property_name,
            *property_span,
            super::is_property_write_target(*property_span),
            symbols,
            ctx,
        );
    }
    let inferred_expression = infer_expression(expression, symbols, ctx);
    if *is_bracketed
        && let InferredExpression::MissingProperty { object_type, .. } = &inferred_expression
    {
        report_missing_element(
            property_name,
            &Type::StringLiteral(property_name.clone()),
            object_type,
            *property_span,
            element_access_span(*object_span, *property_span).or(fallback_span),
            &super::diagnostics::ElementAccessSite::of(object, is_write),
            symbols,
            ctx,
        );
        return InferredExpression::Unknown;
    }
    if let InferredExpression::MissingProperty {
        property_name,
        object_type,
        span,
    } = &inferred_expression
    {
        let diagnostic = match global_this_missing_member(property_name, object_type, ctx) {
            Some(diagnostic) => diagnostic,
            None => Some(missing_property_diagnostic(
                property_name,
                object_type,
                symbols,
                ctx,
            )),
        };
        if let Some(diagnostic) = diagnostic {
            ctx.push(diagnostic_with_syntax_span(
                diagnostic,
                choose_span(*span, fallback_span),
            ));
        }
    }
    if !is_bracketed && matches!(inferred_expression, InferredExpression::Known(_)) {
        maybe_emit_index_signature_access(
            object,
            property_name,
            *property_span,
            fallback_span,
            symbols,
            ctx,
        );
    }
    inferred_expression
}

fn evaluate_satisfies_expression(
    satisfied_expression: &Box<ParsedExpression>,
    span: &Option<SyntaxTextSpan>,
    target_type: &ParsedType,
    anchor_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let temp_symbols = symbols.clone_with_reason(TypeCopyReason::ExpressionInference);
    let saved_symbols = std::mem::replace(&mut ctx.symbols, temp_symbols);
    let resolved_target_type = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
        crate::infer::map_parsed_type(target_type.clone(), ctx)
    });
    ctx.symbols = saved_symbols;

    // Evaluate the left expression contextually against the target type
    // This pushes contextual diagnostics (like excess properties, missing properties).
    // tsc reports a mismatch of the operand as a whole on the satisfies
    // expression, whose error range is its `satisfies` keyword, or the tag
    // of a JSDoc `@satisfies` (`GetErrorRangeForNode`): `anchor_span` is the
    // keyword when one is written, else the target's line.
    let mismatch_span = anchor_span.or(*span).or(fallback_span);
    let contextual_inferred = crate::checks::expected::evaluate_expression_with_expected_type_anchored(
        satisfied_expression,
        span.or(fallback_span),
        mismatch_span,
        Some(&resolved_target_type),
        crate::checks::expected::ExpectedTypeDiagnostic::SatisfiesNotAssignable,
        symbols,
        ctx,
    );

    // We must also perform a top-level assignability check for things that don't do it
    // contextually (e.g. primitives, identifiers). However, if contextual checking already
    // failed and returned Unknown, we might get false cascades. Let's do a clean check
    // against the original inferred type.
    // The operand was checked above under its contextual type; this second,
    // context-free pass only reads its type, and what it reports (a method
    // parameter without the target's type, say) is not tsc's.
    let checked_diagnostics = ctx.diagnostics().len();
    let original_inferred = crate::infer::infer_expression(satisfied_expression, symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checked_diagnostics);
    // `checkSatisfiesExpression` types its operand under the target as the
    // contextual type, which is what gives a function's parameters theirs;
    // the context-free sketch would leave them `any`.
    let original_inferred = match (satisfied_expression.as_ref(), &contextual_inferred) {
        (ParsedExpression::ArrowFunction(_), crate::infer::InferredExpression::Known(Type::Function(_))) => {
            contextual_inferred.clone()
        }
        _ => original_inferred,
    };

    // Check if contextual check already failed (meaning it returned Unknown when actual wasn't Unknown).
    let contextual_failed = matches!(
        contextual_inferred,
        crate::infer::InferredExpression::Unknown
    );
    let mut top_level_failed = false;

    if let crate::infer::InferredExpression::Known(actual_type) = &original_inferred {
        if *actual_type != surge_ts_types::Type::Unknown
            && resolved_target_type != surge_ts_types::Type::Unknown
        {
            let needs_top_level_check = !matches!(
                satisfied_expression.as_ref(),
                surge_ts_syntax::ParsedExpression::ObjectLiteral { .. }
                    | surge_ts_syntax::ParsedExpression::ArrayLiteral { .. }
                    | surge_ts_syntax::ParsedExpression::ConstAssertion { .. }
                    | surge_ts_syntax::ParsedExpression::Conditional { .. }
            );

            // Either side reaching the degradation sentinel anywhere —
            // not just at the top — means surge failed to model part of
            // the shape, so a mismatch says nothing about the source.
            // Same no-cascade rule the assignment checks apply, and
            // checked only after assignability already failed: the deep
            // walk forces lazy references, which perturbs later
            // resolutions when run on every clean check.
            if needs_top_level_check
                && !surge_ts_types::is_assignable_to(actual_type, &resolved_target_type)
                && !crate::checks::function::type_contains_unknown(actual_type)
                && !crate::checks::function::type_contains_unknown(&resolved_target_type)
            {
                top_level_failed = true;
                let actual_type_name = actual_type.name();
                let target_type_name = resolved_target_type.name();
                let diagnostic = crate::checks::expr::satisfies_mismatch_diagnostic(
                    actual_type,
                    &resolved_target_type,
                    &actual_type_name,
                    &target_type_name,
                    ctx.file_name.clone(),
                );
                let diagnostic = match mismatch_span {
                    Some(span) => diagnostic.with_span(crate::context::convert_span(span)),
                    None => diagnostic,
                };
                ctx.push(diagnostic);
            }
        }
    }

    let final_inferred = match original_inferred {
        crate::infer::InferredExpression::Known(ty) => {
            match ctx.options.diagnostic_profile {
                crate::context::DiagnosticProfile::Tsc => {
                    if let ParsedExpression::ConstAssertion {
                        expression: const_inner,
                        span: const_span,
                    } = &**satisfied_expression
                    {
                        // `x as const satisfies T` keeps the
                        // const-asserted literal type; `infer_expression`
                        // widens literal members. Re-derive it with the
                        // const-aware evaluator, gated to pure literal
                        // trees so the re-evaluation cannot re-emit
                        // expression diagnostics.
                        if is_pure_literal_tree(const_inner) {
                            match evaluate_const_expression(
                                const_inner,
                                const_span.or(fallback_span),
                                symbols,
                                ctx,
                            ) {
                                crate::infer::InferredExpression::Known(const_ty) => {
                                    crate::infer::InferredExpression::Known(const_ty)
                                }
                                _ => crate::infer::InferredExpression::Known(ty),
                            }
                        } else {
                            crate::infer::InferredExpression::Known(ty)
                        }
                    } else {
                        crate::infer::InferredExpression::Known(
                            super::diagnostics::widen_type_for_contextual_type(
                                &ty,
                                &resolved_target_type,
                            ),
                        )
                    }
                }
                crate::context::DiagnosticProfile::Native => {
                    crate::infer::InferredExpression::Known(ty)
                }
            }
        }
        other => other,
    };

    // `checkSatisfiesExpression` returns the operand's type whatever the
    // relation found; a literal's failure is a reported excess or missing
    // member, not a type surge could not model.
    let literal_operand = matches!(
        satisfied_expression.as_ref(),
        ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
    );
    if contextual_failed && !(literal_operand && matches!(final_inferred, crate::infer::InferredExpression::Known(_))) {
        crate::infer::InferredExpression::Unknown
    } else if top_level_failed {
        match ctx.options.diagnostic_profile {
            crate::context::DiagnosticProfile::Tsc => final_inferred,
            crate::context::DiagnosticProfile::Native => crate::infer::InferredExpression::Unknown,
        }
    } else {
        final_inferred
    }
}

fn evaluate_type_assertion(
    asserted_expression: &Box<ParsedExpression>,
    expression_span: &Option<SyntaxTextSpan>,
    ty: &ParsedType,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let temp_symbols = symbols.clone_with_reason(TypeCopyReason::ExpressionInference);
    let saved_symbols = std::mem::replace(&mut ctx.symbols, temp_symbols);
    let resolved_type = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
        crate::infer::map_parsed_type(ty.clone(), ctx)
    });
    ctx.symbols = saved_symbols;

    // Evaluate the inner expression so it participates in checking. An
    // asserted function expression is contextually typed by the
    // assertion target (`((arg) => …) as Ctor["create"]`), so its
    // parameters are not implicit-any — resolve the target first and
    // pass it down.
    match asserted_expression.as_ref() {
        ParsedExpression::ArrowFunction(arrow) => {
            let contextual = match resolved_type.peeled() {
                surge_ts_types::Type::Function(function_type) => Some(function_type),
                _ => None,
            };
            // A target surge could not reduce to a signature still gives
            // tsc one, so an implicit-any report here would describe that
            // gap rather than the source.
            let degraded = contextual.is_none();
            if degraded {
                ctx.degraded_expected_type_depth += 1;
            }
            crate::checks::function::NEXT_ARROW_NOT_ELABORATED.set(true);
            let arrow_type = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                crate::checks::function::check_arrow_function_expression_with_expected_type(
                    arrow.as_ref().clone(),
                    contextual.as_ref(),
                    symbols,
                    ctx,
                )
            });
            crate::checks::function::NEXT_ARROW_NOT_ELABORATED.set(false);
            if degraded {
                ctx.degraded_expected_type_depth -= 1;
            }
            let source_type = Type::Function(arrow_type);
            if !degraded && super::assertion::assertion_sides_modelled(&source_type, &resolved_type) {
                super::assertion::check_assertion_overlap(
                    &crate::infer::InferredExpression::Known(source_type),
                    &resolved_type,
                    fallback_span,
                    ctx,
                );
            }
        }
        _ => {
            // An object literal takes the asserted type as its contextual type
            // (Go's `getContextualType`, checker.go:29718), so a method in
            // `{ handle(x) { … } } as Ctx` is not an implicit `any`. The
            // expectation only types; the assertion relates nothing. Go gives
            // every operand that context, but surge's array and tuple
            // expectations report element mismatches whatever the mode, so
            // routing those here would add errors an assertion never raises.
            let source = if matches!(
                asserted_expression.as_ref(),
                ParsedExpression::ObjectLiteral { .. }
            ) {
                crate::checks::expected::evaluate_expression_with_expected_type(
                    asserted_expression,
                    expression_span.or(fallback_span),
                    Some(&resolved_type),
                    crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
                    symbols,
                    ctx,
                )
            } else {
                evaluate_expression(
                    asserted_expression,
                    expression_span.or(fallback_span),
                    symbols,
                    ctx,
                )
            };
            // tsc relates every assertion. surge judges the pairs it models:
            // primitives and their literals, a `null` or `undefined` operand
            // against a modelled target, and any two types free of degradation
            // and of the body's type variables. An array literal is inferred
            // here as the array tsc's contextual typing would have made a
            // tuple, so it is only judged as a primitive pair.
            let asserted_source = match asserted_expression.as_ref() {
                ParsedExpression::ObjectLiteral { .. } => {
                    super::assertion::object_literal_own_type(asserted_expression, symbols, ctx)
                }
                ParsedExpression::Identifier { name, .. } => match &source {
                    InferredExpression::Known(narrowed) => symbols
                        .declared_type(name)
                        .and_then(|declared| super::assertion::member_narrowing_undone(narrowed, declared))
                        .map_or(source, InferredExpression::Known),
                    _ => source,
                },
                _ => source,
            };
            if let InferredExpression::Known(source_type) = &asserted_source
                && ((is_primitive_assertion_side(source_type) && is_primitive_assertion_side(&resolved_type))
                    || (surge_ts_types::strict_null_checks()
                        && matches!(source_type, Type::Null | Type::Undefined)
                        && !crate::checks::function::type_contains_degradation(&resolved_type))
                    || (!matches!(asserted_expression.as_ref(), ParsedExpression::ArrayLiteral { .. })
                        && super::assertion::assertion_sides_modelled(source_type, &resolved_type)))
            {
                super::assertion::check_assertion_overlap(&asserted_source, &resolved_type, fallback_span, ctx);
            }
        }
    }

    // If the type is unresolved (e.g. unknown named type), map_parsed_type
    // already emits TS2304 and returns Type::Unknown.
    // We just return it as the assertion result.
    InferredExpression::Known(resolved_type)
}

fn evaluate_non_null_assertion(
    asserted_expression: &Box<ParsedExpression>,
    expression_span: &Option<SyntaxTextSpan>,
    in_optional_chain: &bool,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let inferred = evaluate_expression(
        asserted_expression,
        expression_span.or(fallback_span),
        symbols,
        ctx,
    );

    match inferred {
        InferredExpression::Known(ty) => {
            let filtered = surge_ts_types::remove_nullish(&ty);
            if *in_optional_chain {
                InferredExpression::Known(surge_ts_types::union_type(vec![
                    filtered,
                    surge_ts_types::Type::Undefined,
                ]))
            } else {
                InferredExpression::Known(filtered)
            }
        }
        other => other,
    }
}

fn evaluate_property_access(
    object: &Box<ParsedExpression>,
    object_span: &Option<SyntaxTextSpan>,
    property_name: &String,
    property_span: &Option<SyntaxTextSpan>,
    is_bracketed: &bool,
    binding_element: bool,
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let is_write = super::diagnostics::take_element_write_target();
    if property_name == "length" && !is_bracketed {
        super::mark_evolving_array_operation(object, ctx);
    }
    // The receiver is code too: a call's bad argument, a missing member
    // deeper in the chain, a possibly-`undefined` link — inferring it
    // types the access but reports none of that.
    let receiver = evaluate_expression(object, object_span.or(fallback_span), symbols, ctx);
    if let ParsedExpression::Identifier { name, span } = object.as_ref()
        && let InferredExpression::Known(receiver_type) = &receiver
        && (ctx.options.isolated_modules || ctx.options.verbatim_module_syntax)
    {
        crate::infer::expression::report_ambient_const_enum_access(name, *span, receiver_type, ctx);
    }
    if !binding_element {
        check_property_receiver(object, &receiver, *object_span, fallback_span, symbols, ctx);
    } else if let Some(parent) = destructured_parent_without_members(object, &receiver, symbols, ctx) {
        report_property_of_unknown(property_name, &parent, *property_span, ctx);
        return InferredExpression::Unknown;
    }
    if !*is_bracketed && surge_ts_types::private_name::is_private_name_key(property_name) {
        let Some(private_receiver) = super::PrivateNameReceiver::of(&receiver) else {
            return InferredExpression::Unknown;
        };
        if let Some(ty) =
            super::check_private_name_access(private_receiver, property_name, *property_span, symbols, ctx)
        {
            return InferredExpression::Known(ty);
        }
    }
    if !*is_bracketed && matches!(receiver, InferredExpression::Known(Type::GenuineUnknown)) {
        if !surge_ts_types::strict_null_checks() {
            report_property_of_unknown(property_name, "unknown", *property_span, ctx);
        }
        return InferredExpression::Unknown;
    }
    // `c["x"]` is tsc's deliberate escape hatch: element access skips the
    // accessibility check that `c.x` gets. A destructured name is checked
    // (`checkVariableLikeDeclaration` on the binding element).
    if (!*is_bracketed || binding_element)
        && let InferredExpression::Known(receiver_type) = &receiver
    {
        check_member_accessibility(
            object,
            receiver_type,
            property_name,
            *property_span,
            super::is_property_write_target(*property_span),
            symbols,
            ctx,
        );
    }
    let inferred_expression = infer_expression(expression, symbols, ctx);
    if *is_bracketed
        && !binding_element
        && let InferredExpression::MissingProperty { object_type, .. } = &inferred_expression
    {
        report_missing_element(
            property_name,
            &Type::StringLiteral(property_name.clone()),
            object_type,
            *property_span,
            element_access_span(*object_span, *property_span).or(fallback_span),
            &super::diagnostics::ElementAccessSite::of(object, is_write),
            symbols,
            ctx,
        );
        return InferredExpression::Unknown;
    }
    report_inferred_expression(
        with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
            inferred_expression.clone()
        }),
        fallback_span,
        symbols,
        ctx,
    );
    if !is_bracketed && matches!(inferred_expression, InferredExpression::Known(_)) {
        maybe_emit_index_signature_access(
            object,
            property_name,
            *property_span,
            fallback_span,
            symbols,
            ctx,
        );
    }
    inferred_expression
}

/// tsc reads a destructured name as an indexed access on the parent type
/// (`getBindingElementTypeFromParentType`), never through
/// `checkNonNullExpression`: a parent that is `unknown`, or admits `null`,
/// `undefined` or `void`, has no such property, and TS2339 names the whole
/// parent. Returns that name.
fn destructured_parent_without_members(
    object: &ParsedExpression,
    receiver: &InferredExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<String> {
    let InferredExpression::Known(parent) = receiver else {
        return None;
    };
    let strict = surge_ts_types::strict_null_checks();
    let parent = match parent {
        Type::TypeParameter(_) => type_parameter_base_constraint(parent)?.unwrap_or(Type::GenuineUnknown),
        other => other.clone(),
    };
    if parent == Type::GenuineUnknown {
        // Without `strictNullChecks` the apparent type of `unknown` is `{}`.
        return Some(if strict { "unknown" } else { "{}" }.to_string());
    }
    let parent = match object {
        // An initializer that cannot be `undefined` takes it off the annotated
        // parent; `null` stays.
        ParsedExpression::TypeAssertion {
            annotation: true,
            expression,
            ..
        } if strict && !initializer_may_be_undefined(expression, symbols, ctx) => {
            surge_ts_types::remove_undefined(&parent)
        }
        _ => parent,
    };
    let admits_void = match parent.peeled() {
        Type::Void => true,
        Type::Union(union) => union.types().iter().any(|member| *member == Type::Void),
        _ => false,
    };
    (admits_void || receiver_nullability(&parent).is_some()).then(|| parent.name())
}

/// tsc's `getBaseConstraintOfType` for a type variable: `Some(None)` when it
/// has no constraint, `None` when surge cannot resolve one.
fn type_parameter_base_constraint(ty: &Type) -> Option<Option<Type>> {
    let mut current = ty.clone();
    for _ in 0..16 {
        let Type::TypeParameter(parameter) = &current else {
            return Some(Some(current));
        };
        match surge_ts_types::type_variable::active_constraint(parameter)? {
            None => return Some(None),
            Some(constraint) if constraint == current => return Some(None),
            Some(constraint) => current = constraint,
        }
    }
    None
}

/// Whether an initializer's own type has tsc's `EQUndefined` fact. An
/// initializer surge cannot type is taken not to.
fn initializer_may_be_undefined(
    initializer: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let InferredExpression::Known(ty) = infer_expression(initializer, symbols, ctx) else {
        return false;
    };
    let may_be_undefined = |ty: &Type| {
        matches!(ty, Type::Undefined | Type::Void | Type::Any | Type::GenuineUnknown | Type::TypeParameter(_))
    };
    match ty.peeled() {
        Type::Union(union) => union.types().iter().any(may_be_undefined),
        other => may_be_undefined(&other),
    }
}

/// Walks a JSX child for ordinary diagnostics. Text is inert; `{expression}`
/// containers and nested elements are evaluated so diagnostics such as unresolved
/// names inside `{...}` are still reported.
fn evaluate_jsx_child(
    child: &ParsedJsxChild,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    match child {
        ParsedJsxChild::Text => {}
        ParsedJsxChild::Expression {
            expression,
            span,
            spread,
        } => {
            if let Some(expression) = expression {
                let inferred = evaluate_expression(expression, span.or(fallback_span), symbols, ctx);
                if *spread {
                    crate::checks::jsx::check_jsx_spread_child(&inferred, span.or(fallback_span), ctx);
                }
            }
        }
        ParsedJsxChild::Element(element) => {
            let _ = evaluate_expression(element, fallback_span, symbols, ctx);
        }
    }
}

/// Whether an expression is a tree of literals and array/object literals only —
/// re-evaluating such a tree cannot resolve names or push diagnostics, so the
/// `satisfies` result path may safely re-derive its const-asserted type.
fn is_pure_literal_tree(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::StringLiteral(_)
        | ParsedExpression::NumberLiteral(_)
        | ParsedExpression::BigIntLiteral(_)
        | ParsedExpression::BooleanLiteral(_) => true,
        ParsedExpression::ArrayLiteral { elements, .. } => elements
            .iter()
            .all(|element| is_pure_literal_tree(&element.expression)),
        ParsedExpression::ObjectLiteral { properties, .. } => properties.iter().all(|property| {
            !property.is_spread && !property.is_method && is_pure_literal_tree(&property.value)
        }),
        _ => false,
    }
}

/// The contextual type a `??`/`||` default operand inherits from the operand it
/// falls back from. Only a function-typed left operand qualifies: that is the
/// case whose parameters would otherwise become implicit `any`, and widening it
/// further would start excess-property-checking object-literal defaults against
/// a type the author never wrote.
fn contextual_default_operand_type(left: &InferredExpression) -> Option<Type> {
    let InferredExpression::Known(left_type) = left else {
        return None;
    };
    let stripped = surge_ts_types::remove_nullish(left_type);
    matches!(stripped.peeled(), Type::Function(_) | Type::Object(_)).then_some(stripped)
}

/// The type tsc gives the `{}` in the `opts ?? {}` idiom. An object literal is
/// contextually typed by the binding pattern it initializes, so the empty
/// fallback carries the left operand's property *names* with type `undefined`;
/// reads off the joined union then answer `T | undefined` rather than reporting
/// the name as missing from a bare `{}`. `None` for any other right operand.
pub(crate) fn empty_object_fallback_type(
    right: &ParsedExpression,
    contextual: &Type,
) -> Option<Type> {
    let ParsedExpression::ObjectLiteral { properties, .. } = right else {
        return None;
    };
    if !properties.is_empty() {
        return None;
    }
    let Type::Object(contextual_object) = contextual.peeled() else {
        return None;
    };
    let mut absent = surge_ts_types::PropertyMap::default();
    for name in contextual_object.properties.keys() {
        absent.insert(
            name.clone(),
            surge_ts_types::ObjectProperty::optional(Type::Undefined),
        );
    }
    // tsc subtype-reduces `x ?? {}` to `x`'s type, so an index signature on it
    // survives: `(schema.properties ?? {})['data']` still reads the element.
    let string_index_type = contextual_object.string_index_type.as_deref().cloned();
    Some(Type::Object(surge_ts_types::ObjectType::new(
        absent,
        string_index_type,
    )))
}

/// For an equality operator, the result it has when its operands differ:
/// `false` for `==`/`===`, `true` for `!=`/`!==`.
fn equality_result_when_unequal(operator: surge_ts_syntax::ParsedBinaryOperator) -> Option<bool> {
    match operator {
        surge_ts_syntax::ParsedBinaryOperator::StrictEquals | surge_ts_syntax::ParsedBinaryOperator::Equals => Some(false),
        surge_ts_syntax::ParsedBinaryOperator::StrictNotEquals | surge_ts_syntax::ParsedBinaryOperator::NotEquals => Some(true),
        _ => None,
    }
}

/// tsc's equality checks that need only syntax: comparing against an object,
/// array or `function` literal can never be true, since objects compare by
/// reference (TS2839), and comparing against the global `NaN` never either
/// (TS2845).
fn report_reference_and_nan_equality(
    left: &ParsedExpression,
    right: &ParsedExpression,
    unequal_result: bool,
    strict: bool,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let result = if unequal_result { "true" } else { "false" };
    let is_object_literal = |expression: &ParsedExpression| match expression {
        ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. } => true,
        // A `function` expression lowers to the arrow shape but binds its own
        // `this`; tsc exempts a real arrow.
        ParsedExpression::ArrowFunction(arrow) => {
            !matches!(arrow.this_binding, surge_ts_syntax::ParsedThisBinding::Inherited)
        }
        _ => false,
    };
    // A JavaScript file is checked only for strict equality.
    if (is_object_literal(left) || is_object_literal(right))
        && (strict || !surge_ts_syntax::is_javascript_file_name(&ctx.file_name))
    {
        ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2839(result, ctx.file_name.clone()), span));
    }
    // The lib's `declare var NaN` binds as a `let`-like global; a parameter or
    // block-scoped constant of the same name is not the global.
    let is_global_nan = |expression: &ParsedExpression| {
        matches!(expression, ParsedExpression::Identifier { name, .. } if name == "NaN")
            && symbols
                .get("NaN")
                .is_some_and(|symbol| {
                    matches!(
                        symbol.kind,
                        crate::symbols::SymbolKind::Var | crate::symbols::SymbolKind::Let
                    )
                })
    };
    if is_global_nan(left) || is_global_nan(right) {
        ctx.push(diagnostic_with_syntax_span(Diagnostic::ts2845(result, ctx.file_name.clone()), span));
    }
}

/// Whether `checkBinaryLikeExpression` runs `checkNonNullType` on both operands:
/// always for arithmetic, bitwise and `in`; for `+` only when neither side is
/// string-like (concatenation takes `null`); for a relational comparison only
/// when no operand is a symbol (that is TS2469 instead).
fn binary_operands_must_be_non_null(
    operator: surge_ts_syntax::ParsedBinaryOperator,
    left: &ParsedExpression,
    left_result: &InferredExpression,
    right: &ParsedExpression,
    right_result: &InferredExpression,
) -> bool {
    use surge_ts_syntax::ParsedBinaryOperator as Op;
    // tsc's `isTypeAssignableToKind(t, StringLike)` is the non-strict check, so
    // without `strictNullChecks` the `null` keyword is string-like (surge types
    // the keyword as `any`, which would read as string-like under it too). An
    // operand surge could not type is tsc's error type, an `any`.
    let string_like = |operand: &ParsedExpression, result: &InferredExpression| {
        if matches!(operand, ParsedExpression::NullLiteral) {
            return !surge_ts_types::strict_null_checks();
        }
        match result {
            InferredExpression::Known(ty) => surge_ts_types::is_assignable_to(ty, &Type::String),
            _ => true,
        }
    };
    let symbol_like = |result: &InferredExpression| matches!(result, InferredExpression::Known(ty) if type_may_be_symbol(&ty.peeled()));
    match operator {
        Op::Subtract
        | Op::Multiply
        | Op::Divide
        | Op::Remainder
        | Op::Exponential
        | Op::ShiftLeft
        | Op::ShiftRight
        | Op::ShiftRightZeroFill
        | Op::BitwiseAnd
        | Op::BitwiseOR
        | Op::BitwiseXOR
        | Op::In => true,
        Op::Add => !string_like(left, left_result) && !string_like(right, right_result),
        Op::LessThan | Op::LessThanEquals | Op::GreaterThan | Op::GreaterThanEquals => {
            !symbol_like(left_result) && !symbol_like(right_result)
        }
        Op::StrictEquals | Op::StrictNotEquals | Op::Equals | Op::NotEquals | Op::Instanceof => {
            false
        }
    }
}

/// tsc's `checkInExpression`: the key must be assignable to
/// `string | number | symbol`, and the right operand must be non-nullable and
/// assignable to `object`. Operands surge could not settle are not judged.
fn check_in_operands(
    left_result: &InferredExpression,
    right_result: &InferredExpression,
    left_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let judgeable = |ty: &Type| {
        !crate::checks::function::type_contains_unknown(ty)
            && !matches!(ty, Type::Any | Type::GenuineUnknown | Type::ErrorType)
    };
    let property_key = surge_ts_types::union_type(vec![Type::String, Type::Number, Type::Symbol]);
    let key_assignable = |ty: &Type| surge_ts_types::is_assignable_to(ty, &property_key);
    if let InferredExpression::Known(left_type) = left_result
        && judgeable(left_type)
        && !key_assignable(left_type)
    {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2322(
                &source_display_name(left_type, &Type::String),
                "string | number | symbol",
                ctx.file_name.clone(),
            ),
            left_span,
        ));
    }
    let InferredExpression::Known(right_type) = right_result else {
        return;
    };
    if !judgeable(right_type) {
        return;
    }
    let non_primitive = Type::Object(surge_ts_types::ObjectType::new(Default::default(), None).with_non_primitive_marker());
    if !surge_ts_types::is_assignable_to(right_type, &non_primitive) {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2322(&right_type.name(), "object", ctx.file_name.clone()),
            right_span,
        ));
    } else if crate::checks::function::is_unknown_empty_object(right_type) {
        // `hasEmptyObjectIntersection`: `{}` (a narrowed `unknown`) may be a
        // primitive.
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2638(right_type.name(), ctx.file_name.clone()),
            right_span,
        ));
    }
}


/// tsc's `+`/`-`/`~` arm of `checkPrefixUnaryExpression`: a possibly nullish
/// operand is reported (`checkNonNullType`), a symbol operand is TS2469, and
/// unary `+` on a bigint is TS2736.
fn check_numeric_unary_operand(
    operator: ParsedUnaryOperator,
    operand: &ParsedExpression,
    operand_type: &Type,
    operand_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if matches!(operand, ParsedExpression::NullLiteral) || *operand_type == Type::GenuineUnknown {
        super::check_non_null_operand(operand, operand_type, operand_span, ctx);
        return;
    }
    if crate::checks::function::type_contains_unknown(operand_type)
        || matches!(operand_type, Type::Any | Type::GenuineUnknown | Type::ErrorType)
    {
        return;
    }
    let operator_text = match operator {
        ParsedUnaryOperator::Plus => "+",
        ParsedUnaryOperator::Minus => "-",
        _ => "~",
    };
    super::check_non_null_operand(operand, operand_type, operand_span, ctx);
    fn some_member(ty: &Type, test: &dyn Fn(&Type) -> bool) -> bool {
        match ty {
            Type::Union(union) => union.types().iter().any(|member| some_member(member, test)),
            other => test(other),
        }
    }
    if some_member(operand_type, &|ty| matches!(ty, Type::Symbol)) {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2469(operator_text, ctx.file_name.clone()),
            operand_span,
        ));
    }
    if matches!(operator, ParsedUnaryOperator::Plus)
        && some_member(operand_type, &|ty| matches!(ty, Type::BigInt))
    {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2736("+", &widen_type(operand_type).name(), ctx.file_name.clone()),
            operand_span,
        ));
    }
}

fn type_may_be_symbol(ty: &Type) -> bool {
    match ty {
        Type::Symbol => true,
        Type::Union(union) => union.types().iter().any(type_may_be_symbol),
        _ => false,
    }
}

fn is_primitive_assertion_side(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_primitive_assertion_side),
        _ => false,
    }
}

fn report_property_of_unknown(
    property_name: &str,
    receiver_name: &str,
    property_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2339(property_name, receiver_name, ctx.file_name.clone()),
        property_span,
    ));
}

/// `symbols` once `expression` has run the assignments it certainly performs:
/// each target holds the type it was assigned. `None` when it assigns nothing.
fn symbols_after_assignments(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<SymbolTable> {
    if !expression.contains_assignment() {
        return None;
    }
    let mut after = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    for (assignment, conditional) in crate::flow::expression_assignments(expression) {
        let ParsedExpression::Assignment {
            target_name, value, ..
        } = assignment
        else {
            continue;
        };
        if conditional {
            continue;
        }
        let (InferredExpression::Known(assigned), Some(symbol)) =
            (infer_expression(value, &after, ctx), after.get(target_name))
        else {
            continue;
        };
        if assigned.is_unknown() {
            continue;
        }
        // tsc's `getTypeAtFlowAssignment` narrows the declared type by what
        // was assigned, and a later write in the right operand is still
        // checked against the declaration: `(x = "") && (x = 0)`.
        let declared = after.declared_type(target_name).unwrap_or(&symbol.ty).clone();
        let updated = crate::symbols::SymbolInfo {
            ty: crate::checks::function::body_statements::assignment_reduced_type(Some(&declared), assigned),
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        };
        let _ = after.insert_narrowed(target_name.clone(), updated, declared);
    }
    Some(after)
}
