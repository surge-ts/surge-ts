use super::*;

pub(super) fn evaluate_optional_index_access(
    object: &ParsedExpression,
    object_span: Option<SyntaxTextSpan>,
    index: &ParsedExpression,
    index_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let object_result = evaluate_expression(object, object_span.or(fallback_span), symbols, ctx);

    let object_type = match object_result {
        InferredExpression::Known(ty) => ty,
        InferredExpression::UnresolvedIdentifier { name, span } => {
            return InferredExpression::UnresolvedIdentifier { name, span };
        }
        InferredExpression::MissingProperty { .. } | InferredExpression::Unknown => {
            return InferredExpression::Unknown;
        }
    };

    let base_type = surge_ts_types::remove_nullish(&object_type);

    match base_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => InferredExpression::Unknown,
        Type::Tuple(elements) => {
            let index_result =
                evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
            let index_type = match index_result {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };

            if let Some(index_value) = crate::infer::tuple_index_value(&index_type) {
                return match elements.get(index_value).cloned() {
                    Some(element_type) => {
                        InferredExpression::Known(union_type(vec![element_type, Type::Undefined]))
                    }
                    None => {
                        let index_type_name = index_type.name();
                        let object_type_name = Type::Tuple(elements.to_vec()).name();
                        let diagnostic = Diagnostic::ts2339(
                            &index_type_name,
                            &object_type_name,
                            ctx.file_name.clone(),
                        );

                        ctx.push(diagnostic_with_syntax_span(
                            diagnostic,
                            choose_span(index_span, choose_span(object_span, fallback_span)),
                        ));
                        InferredExpression::Unknown
                    }
                };
            }

            if !numeric_index_key(&index_type) && !indexes_as_number(index, symbols) {
                report_non_numeric_index(
                    &index_type,
                    choose_span(index_span, choose_span(object_span, fallback_span)),
                    ctx,
                );
                return InferredExpression::Unknown;
            }

            InferredExpression::Known(union_type(vec![
                union_type(elements.to_vec()),
                Type::Undefined,
            ]))
        }
        Type::Array(element_type) => {
            if element_type.is_unknown() && !element_type.is_type_variable() && *element_type != Type::GenuineUnknown {
                return InferredExpression::Unknown;
            }

            let index_result =
                evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
            let index_type = match index_result {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };
            if let Some(member) =
                well_known_symbol_member(&Type::Array(element_type.clone()), index, symbols)
            {
                return InferredExpression::Known(member);
            }

            if !numeric_index_key(&index_type) && !indexes_as_number(index, symbols) {
                report_non_numeric_index(
                    &index_type,
                    choose_span(index_span, choose_span(object_span, fallback_span)),
                    ctx,
                );
                return InferredExpression::Unknown;
            }

            InferredExpression::Known(union_type(vec![
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                    element_type.as_ref().clone()
                }),
                Type::Undefined,
            ]))
        }
        _ => InferredExpression::Unknown,
    }
}

pub(super) fn evaluate_index_access(
    object_name: &str,
    object_span: Option<SyntaxTextSpan>,
    index: &ParsedExpression,
    index_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let is_write = super::diagnostics::take_element_write_target();
    if super::check_auto_array_read(object_name, object_span, symbols, ctx).is_some() {
        return InferredExpression::Known(Type::Any);
    }
    // tsc's `checkElementAccessExpression` checks the index before it gives
    // up on an error object.
    // As an identifier read resolves it: the module's value table backs a
    // binding a namespace body's scope does not carry.
    let Some(symbol) = symbols.get_handle(object_name).or_else(|| {
        ctx.module_value_fallback
            .as_ref()
            .and_then(|fallback| fallback.get_handle(object_name))
    }) else {
        if object_name != "super" {
            report_unresolved_value_name(
                object_name,
                choose_span(object_span, fallback_span),
                UnresolvedNameSite::Reference,
                symbols,
                ctx,
            );
            let _ = evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
        }
        return InferredExpression::Unknown;
    };
    if symbol.ty == Type::GenuineUnknown {
        let object = ParsedExpression::Identifier {
            name: object_name.to_string(),
            span: object_span,
        };
        super::report_unknown_operand(&object, choose_span(object_span, fallback_span), ctx);
        let _ = evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
        return InferredExpression::Unknown;
    }
    // tsc's `checkElementAccessExpression`: a const enum object indexed by
    // anything but a string literal is TS2476 (a grammar diagnostic) and the
    // access is the error type, so its reverse mapping never answers it.
    if crate::infer::expression::indexes_const_enum_object(object_name, index, &symbol.ty, ctx) {
        let _ = evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
        return InferredExpression::Known(Type::ErrorType);
    }
    if let Some(narrowed) = crate::infer::narrowed_element_read_named(object_name, index, symbols)
    {
        return InferredExpression::Known(narrowed);
    }

    let receiver = ParsedExpression::Identifier {
        name: object_name.to_string(),
        span: object_span,
    };
    let receiver_type =
        strip_reported_undefined_receiver(&receiver, symbol.ty.clone(), object_span, fallback_span, symbols, ctx);
    evaluate_receiver_index_access(
        &receiver,
        receiver_type,
        object_span,
        index,
        index_span,
        fallback_span,
        is_write,
        symbols,
        ctx,
    )
}

/// [`evaluate_index_access`] past its receiver, whose type the access goes on
/// with once a possibly-`undefined` one was reported: `receiver[index]`.
#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_receiver_index_access(
    receiver: &ParsedExpression,
    receiver_type: Type,
    object_span: Option<SyntaxTextSpan>,
    index: &ParsedExpression,
    index_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    is_write: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let site = super::diagnostics::ElementAccessSite::of(receiver, is_write);
    // A user-defined thenable indexes like the value it resolves to: `await` is
    // erased at parse time, so `const rows = await db.execute(...)` reaches here
    // as the query object itself, exactly as a `Promise<T>` reaches it as `T`.
    let receiver_type = crate::checks::call::thenable_awaited_type(&receiver_type)
        .unwrap_or(receiver_type);
    // A class's polymorphic `this` indexes like the instance it stands for.
    let receiver_type = surge_ts_types::without_polymorphic_this(receiver_type);

    if let ParsedExpression::NumberLiteral(index_value) = index
        && let Type::OpenTuple(tuple) = receiver_type.peeled()
        && let Some(element) = tuple.leading_element(index_value)
    {
        return InferredExpression::Known(element.clone());
    }

    // A nominal array reference (`Array<number>`, `ReadonlyArray<string>`)
    // indexes like the array it names; left unpeeled it fell through to the
    // object arm and reported the *receiver* as a missing property.
    let receiver_type = match &receiver_type {
        // A reference to the library `Array<T>` interface itself (a `Array<string>`
        // return annotation reaching the caller through a package re-export)
        // peels to the interface's member object, which has no numeric index
        // signature to answer `key[0]`; it indexes like `T[]` all the same.
        Type::Reference(reference)
            if reference.arguments.len() == 1
                && matches!(
                    reference.id.split('\u{0}').next_back(),
                    Some("Array" | "ReadonlyArray")
                ) =>
        {
            Type::Array(Box::new(reference.arguments[0].clone()))
        }
        Type::Reference(_) => match receiver_type.peeled() {
            peeled @ (Type::Array(_) | Type::Tuple(_) | Type::Any) => peeled,
            Type::OpenTuple(tuple) => Type::Array(Box::new(tuple.element_union())),
            _ => receiver_type,
        },
        _ => receiver_type,
    };

    // `getPropertyOfType` on a union of fixed tuples: a literal index past
    // every member's end is no property of the union (TS2339).
    if let ParsedExpression::NumberLiteral(index_value) = index
        && let Ok(index_value) = index_value.parse::<usize>()
        && let Type::Union(union) = receiver_type.peeled()
        && union
            .types()
            .iter()
            .all(|member| matches!(member.peeled(), Type::Tuple(elements) if index_value >= elements.len()))
    {
        let span = choose_span(index_span, choose_span(object_span, fallback_span));
        if span.is_some_and(|span| ctx.allow_missing_tuple_elements.contains(&span)) {
            return InferredExpression::Known(Type::Undefined);
        }
        let diagnostic = Diagnostic::ts2339(index_value.to_string(), receiver_type.name(), ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, span));
        return InferredExpression::Unknown;
    }

    if let Type::OpenTuple(tuple) = &receiver_type {
        return crate::infer::expression::infer_open_tuple_index_access(
            tuple,
            index,
            &index_span,
            symbols,
            ctx,
        );
    }

    match &receiver_type {
        Type::Any => InferredExpression::Known(Type::Any),
        Type::TypeParameter(_) => {
            let InferredExpression::Known(index_type) =
                crate::infer::expression::infer_element_access_argument(index, symbols, ctx)
            else {
                return InferredExpression::Unknown;
            };
            let Some(deferred) = crate::infer::expression::deferred_element_access(&receiver_type, &index_type)
            else {
                return constraint_element_read(&receiver_type, index, &index_type, symbols, ctx)
                    .map_or(InferredExpression::Unknown, InferredExpression::Known);
            };
            if report_invalid_deferred_index(
                &receiver_type,
                &index_type,
                element_access_span(object_span, index_span).or(fallback_span),
                ctx,
            ) {
                return InferredExpression::Known(Type::ErrorType);
            }
            InferredExpression::Known(deferred)
        }
        Type::Unknown | Type::GenuineUnknown | Type::ErrorType => InferredExpression::Unknown,
        // Lowered to its element array above.
        Type::OpenTuple(_) => InferredExpression::Unknown,
        Type::Tuple(elements) => {
            let index_result =
                evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
            let index_type = match index_result {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };
            if let Some(member) = well_known_symbol_member(&receiver_type, index, symbols) {
                return InferredExpression::Known(member);
            }

            // tsc's `getPropertyTypeForIndexType`: a literal index past a fixed
            // tuple's end is TS2493 (a negative one TS2514) and reads `undefined`,
            // unless the access allows a missing element — a destructured
            // binding with a default.
            if let Some(index_value) = tuple_literal_index_value(&index_type) {
                if let Some(element_type) =
                    usize::try_from(index_value).ok().and_then(|index| elements.get(index))
                {
                    return InferredExpression::Known(element_type.clone());
                }
                report_tuple_index_out_of_bounds(
                    elements,
                    index_value,
                    choose_span(index_span, choose_span(object_span, fallback_span)),
                    ctx,
                );
                return InferredExpression::Known(Type::Undefined);
            }

            if !numeric_index_key(&index_type) && !indexes_as_number(index, symbols) {
                report_non_numeric_index(
                    &index_type,
                    choose_span(index_span, choose_span(object_span, fallback_span)),
                    ctx,
                );
                return InferredExpression::Unknown;
            }

            InferredExpression::Known(crate::infer::unchecked_index_read(
                union_type(elements.to_vec()),
                ctx,
            ))
        }
        Type::Array(element_type) => {
            if element_type.is_unknown() && !element_type.is_type_variable() && **element_type != Type::GenuineUnknown {
                return InferredExpression::Unknown;
            }

            let index_result =
                evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
            let index_type = match index_result {
                InferredExpression::Known(ty) => ty,
                InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Unknown => return InferredExpression::Unknown,
            };
            if let Some(member) =
                well_known_symbol_member(&Type::Array(element_type.clone()), index, symbols)
            {
                return InferredExpression::Known(member);
            }

            if !numeric_index_key(&index_type) && !indexes_as_number(index, symbols) {
                report_non_numeric_index(
                    &index_type,
                    choose_span(index_span, choose_span(object_span, fallback_span)),
                    ctx,
                );
                return InferredExpression::Unknown;
            }

            InferredExpression::Known(crate::infer::unchecked_index_read(
                with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
                    element_type.as_ref().clone()
                }),
                ctx,
            ))
        }
        Type::Object(_)
        | Type::Function(_)
        | Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Void
        | Type::Never
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Undefined
        | Type::Null
        | Type::Reference(_)
        | Type::Union(_) => {
            let index_result =
                evaluate_expression(index, index_span.or(fallback_span), symbols, ctx);
            let index_type = match index_result {
                InferredExpression::Known(ty) => ty,
                // An unresolved key is tsc's error type, which a non-string
                // index signature answers first, then a string one
                // (`findApplicableIndexInfo`); with neither the read is `any`.
                InferredExpression::UnresolvedIdentifier { .. } => {
                    return match receiver_type.peeled() {
                        Type::Object(object)
                            if !object.synthetic_open_index
                                && !crate::checks::function::type_contains_degradation(&receiver_type) =>
                        {
                            object
                                .number_index_type
                                .as_deref()
                                .or(object.string_index_type.as_deref())
                                .cloned()
                                .map_or(InferredExpression::Unknown, InferredExpression::Known)
                        }
                        _ => InferredExpression::Unknown,
                    };
                }
                _ => return InferredExpression::Unknown,
            };
            let index_type = well_known_symbol_key_type(index, symbols).unwrap_or(index_type);
            if report_unusable_index_type(
                &index_type,
                choose_span(index_span, choose_span(object_span, fallback_span)),
                ctx,
            ) {
                return InferredExpression::Unknown;
            }
            if report_invalid_union_variable_index(
                &receiver_type,
                &index_type,
                element_access_span(object_span, index_span).or(fallback_span),
                ctx,
            ) {
                return InferredExpression::Known(Type::ErrorType);
            }
            if let Some(read) = narrowed_variable_element_read(&receiver_type, index, &index_type, symbols, ctx) {
                return InferredExpression::Known(read);
            }

            // A literal index names a concrete property that must exist on the
            // receiver; a missing one is a real TS2339. A *non-literal* computed
            // key (plain `string`/`number`, `keyof T`, a type parameter, `symbol`,
            // …) resolves to an indexed-access type (`T[K]`, an index-signature
            // value, …) and is NOT a missing-property error — emitting one here was
            // a false positive that mis-named the receiver as the absent property.
            //
            // Only an object-like receiver can be missing a literal-named member.
            // Primitives carry an apparent type with index signatures (`string`'s
            // numeric index returns `string`, `string["length"]` is a real member,
            // …), so a literal index there is never a TS2339 — emitting one was a
            // false positive (`path[0]` reported as `Property 'path' ... 'string'`).
            let receiver_is_object_like = matches!(
                receiver_type,
                Type::Object(_) | Type::Function(_) | Type::Reference(_)
            );
            // A key that is not a literal still resolves through an index
            // signature: `record[k]` with `k: string` reads the string index,
            // and a numeric key prefers the number one
            // (`findApplicableIndexInfo`). A number-only receiver cannot answer
            // a string key at all — under `noImplicitAny` that is TS7015, tsc's
            // "index expression is not of type 'number'".
            let index_is_numeric = is_assignable_to(&index_type, &Type::Number)
                || indexes_as_number(index, symbols);
            let Some(key) = literal_index_key(&index_type) else {
                if let Type::Object(object_type) = receiver_type.peeled() {
                    // A unique symbol names the member declared under it,
                    // `[s]` (tsc's late-bound name). One written through another
                    // path to the same symbol cannot be told apart here.
                    if let Type::Reference(reference) = &index_type
                        && let Some(name) = reference.unique_symbol_name()
                    {
                        let own = format!("[{name}]");
                        let qualified = format!(".{name}]");
                        if let Some(property) = object_type
                            .properties
                            .iter()
                            .find(|(key, _)| key.as_ref() == own.as_str() || key.ends_with(&qualified))
                            .map(|(_, property)| property)
                        {
                            return InferredExpression::Known(property.ty.clone());
                        }
                        if object_type
                            .properties
                            .keys()
                            .any(|key| key.starts_with('[') && !key.starts_with("[Symbol."))
                        {
                            return InferredExpression::Unknown;
                        }
                    }
                    if let Some(keyed) = object_type.active_keyed_indexes()
                        && !index_type.is_type_variable()
                        && !index_type.peeled().is_unknown()
                        && !matches!(index_type.peeled(), Type::Any)
                    {
                        return keyed_index_read(
                            keyed,
                            &index_type,
                            &receiver_type,
                            element_access_span(object_span, index_span).or(fallback_span),
                            ctx,
                        );
                    }
                    // `getIndexedAccessType` distributes a union of literal keys,
                    // each reading the member it names; a generic key is read
                    // through its constraint, which is what its deferred
                    // `T[K]` relates by.
                    // surge reads a generic key eagerly rather than deferring
                    // `T[K]`; its constraint is read only where an index
                    // signature would otherwise answer for declared members.
                    let key_union = if !index_type.is_type_variable() {
                        index_type.clone()
                    } else if object_type.applicable_index_type(index_is_numeric).is_some() {
                        surge_ts_types::type_variable::base_constraint_or_type(&index_type)
                    } else {
                        Type::Never
                    };
                    // A write through a union of keys must suit every member
                    // it may land on (`AccessFlagsWriting` intersects them); a
                    // generic key's write stays the deferred `T[K]`, which
                    // surge does not model.
                    if let Some(members) = declared_literal_union_members(&object_type, &key_union) {
                        if !is_write {
                            return InferredExpression::Known(union_type(members));
                        }
                        if !index_type.is_type_variable() {
                            return InferredExpression::Known(crate::infer::types::merge_intersection_members(members));
                        }
                    }
                    // `getIndexedAccessType` distributes a union key over the
                    // index signatures too: `both[string | number]` reads the
                    // string index and the number index.
                    if let Type::Union(keys) = &index_type
                        && keys.types().iter().all(|key| matches!(key, Type::String | Type::Number))
                        && let Some(reads) = keys
                            .types()
                            .iter()
                            .map(|key| object_type.applicable_index_type(*key == Type::Number).cloned())
                            .collect::<Option<Vec<Type>>>()
                        && reads.iter().any(|read| *read != reads[0])
                    {
                        let read = if is_write {
                            crate::infer::types::merge_intersection_members(reads)
                        } else {
                            union_type(reads)
                        };
                        return InferredExpression::Known(crate::infer::unchecked_index_read(read, ctx));
                    }
                    if let Some(index_value) = object_type.applicable_index_type(index_is_numeric) {
                        return InferredExpression::Known(crate::infer::unchecked_index_read(
                            index_value.clone(),
                            ctx,
                        ));
                    }
                    if let Some(member_union) = union_literal_index_read(&object_type, &index_type)
                    {
                        return InferredExpression::Known(member_union);
                    }
                    if object_type.number_index_type.is_some() && ctx.options.no_implicit_any {
                        ctx.push(diagnostic_with_syntax_span(
                            Diagnostic::ts7015(ctx.file_name.clone()),
                            choose_span(index_span, choose_span(object_span, fallback_span)),
                        ));
                        return InferredExpression::Unknown;
                    }
                    // `getPropertyTypeForIndexType`: a key the receiver answers
                    // neither as a member nor through an index signature makes
                    // the whole access an implicit `any`, exactly as a literal
                    // key that misses does.
                    //
                    // Held to a member set surge wrote down itself: a nominal
                    // reference's index signature can come from a base, a
                    // mapped type or a `Record` surge has not expanded here
                    // (zod's `$ZodShape`, `RegExpExecArray`), and an empty
                    // object type is a modelling gap rather than a receiver
                    // that answers nothing.
                    // A class's own instance is a member set surge wrote down
                    // too, judged by a unique symbol key it does not declare.
                    let class_instance_symbol_key = matches!(&index_type, Type::Reference(key) if key.is_unique_symbol())
                        && object_type.string_index_type.is_none()
                        && object_type.number_index_type.is_none()
                        && !object_type.synthetic_open_index
                        && matches!(&receiver_type, Type::Reference(reference) if reference.arguments.is_empty()
                            && reference.id.rsplit('\u{0}').next().and_then(|name| ctx.lookup_type_declaration(name)).is_some_and(
                                |info| matches!(info, crate::symbols::TypeDeclarationInfo::Interface(info) if info.is_class_instance),
                            ));
                    if ctx.options.no_implicit_any
                        && index_key_is_concrete(&index_type)
                        && !enum_key_names_members(&index_type, &object_type)
                        && (matches!(&receiver_type, Type::Object(written) if !written.properties.is_empty())
                            || class_instance_symbol_key)
                    {
                        ctx.push(diagnostic_with_syntax_span(
                            crate::checks::expr::ts7053_for_index(
                                &index_type,
                                receiver_type.name(),
                                ctx.file_name.clone(),
                            ),
                            element_access_span(object_span, index_span).or(fallback_span),
                        ));
                    }
                } else if let Type::Union(union) = receiver_type.peeled()
                    && ctx.options.no_implicit_any
                    && let Type::Reference(key) = &index_type
                    && let Some(name) = key.unique_symbol_name()
                {
                    // A late-bound name reads no index signature, so a union
                    // whose members do not all declare it has no such member
                    // (`createUnionOrIntersectionProperty`, a read partial).
                    let own = format!("[{name}]");
                    let qualified = format!(".{name}]");
                    let declares = |member: &surge_ts_types::ObjectType| {
                        member.properties.keys().any(|key| key.as_ref() == own.as_str() || key.ends_with(&qualified))
                    };
                    let members: Vec<Type> = union.types().iter().map(Type::peeled).collect();
                    let objects: Option<Vec<&surge_ts_types::ObjectType>> = members
                        .iter()
                        .map(|member| match member {
                            Type::Object(object)
                                if !object.synthetic_open_index
                                    && object.active_keyed_indexes().is_none()
                                    && !crate::checks::function::type_contains_degradation(member) =>
                            {
                                Some(object)
                            }
                            _ => None,
                        })
                        .collect();
                    // The union's own index signatures are those every member
                    // has, so one member without any leaves the read nothing.
                    if let Some(objects) = objects
                        && objects.iter().any(|object| declares(object))
                        && !objects.iter().all(|object| declares(object))
                        && objects.iter().any(|object| {
                            object.string_index_type.is_none() && object.number_index_type.is_none()
                        })
                    {
                        ctx.push(diagnostic_with_syntax_span(
                            crate::checks::expr::ts7053_for_index(&index_type, receiver_type.name(), ctx.file_name.clone()),
                            element_access_span(object_span, index_span).or(fallback_span),
                        ));
                        return InferredExpression::Unknown;
                    }
                }
                if report_global_this_element_access(
                    &receiver_type,
                    &index_type,
                    element_access_span(object_span, index_span).or(fallback_span),
                    ctx,
                ) {
                    return InferredExpression::Unknown;
                }
                // A `string` answers a numeric index and its own members and
                // nothing else, so any other key is the implicit `any` an
                // array's non-numeric index already is.
                if reads_string_by_number(&receiver_type) {
                    if index_is_numeric {
                        return InferredExpression::Known(crate::infer::unchecked_index_read(
                            Type::String,
                            ctx,
                        ));
                    }
                    if index_key_is_concrete(&index_type) {
                        report_non_numeric_index(
                            &index_type,
                            choose_span(index_span, choose_span(object_span, fallback_span)),
                            ctx,
                        );
                    }
                }
                return InferredExpression::Unknown;
            };

            // A nominal alias can peel to a tuple, or to a union distributing
            // over them (`type ResultTuple<T> = [undefined, T] | [Err, undefined]`).
            // The `Type::Tuple` arm above only sees an *unpeeled* tuple, so a
            // literal index on the alias fell through to here and was reported as
            // a property missing from the receiver — named after the receiver
            // binding, since that is what this arm has in hand.
            if let Some(element_type) = literal_tuple_element(&receiver_type.peeled(), &key) {
                return InferredExpression::Known(element_type);
            }
            if let Type::Union(union) = receiver_type.peeled() {
                match union_numeric_key_read(union.types(), &key, ctx) {
                    UnionKeyRead::Found(member) => return InferredExpression::Known(member),
                    UnionKeyRead::Missing => {
                        if ctx.options.no_implicit_any {
                            ctx.push(diagnostic_with_syntax_span(
                                crate::checks::expr::ts7053_for_index(&index_type, receiver_type.name(), ctx.file_name.clone()),
                                element_access_span(object_span, index_span).or(fallback_span),
                            ));
                        }
                        return InferredExpression::Unknown;
                    }
                    UnionKeyRead::Undecided => {}
                }
            }

            // A literal key names a member. An index signature answers it too —
            // a numeric key is converted to a string, which is why
            // `record[1]` reads a `Record<number, T>` and a `Record<string, T>`
            // alike. Only a receiver that declares neither is a real TS2339.
            if let Type::Object(object_type) = receiver_type.peeled() {
                if let Some(member) = literal_key_member_read(&object_type, &key, ctx) {
                    return InferredExpression::Known(member);
                }
                if let Some(keyed) = object_type.active_keyed_indexes() {
                    return keyed_index_read(
                        keyed,
                        &Type::StringLiteral(key.clone()),
                        &receiver_type,
                        element_access_span(object_span, index_span).or(fallback_span),
                        ctx,
                    );
                }
                if let Some(index_value) = object_type.applicable_index_type(index_is_numeric) {
                    return InferredExpression::Known(crate::infer::unchecked_index_read(
                        index_value.clone(),
                        ctx,
                    ));
                }
                // `getPropertyTypeForIndexType` skips the implicit-`any` reports
                // for a const enum's object (`isConstEnumObjectType`): a literal
                // key it lacks is TS2339 on the index, whatever the options.
                if site
                    .receiver_text
                    .as_deref()
                    .is_some_and(|receiver| crate::infer::expression::is_const_enum_object(receiver, &receiver_type, ctx))
                {
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts2339(&key, receiver_type.name(), ctx.file_name.clone()),
                        choose_span(index_span, choose_span(object_span, fallback_span)),
                    ));
                    return InferredExpression::Known(Type::ErrorType);
                }
                // A literal key a number-only receiver cannot answer is the same
                // implicit `any` as a non-literal one.
                if object_type.number_index_type.is_some() && ctx.options.no_implicit_any {
                    ctx.push(diagnostic_with_syntax_span(
                        Diagnostic::ts7015(ctx.file_name.clone()),
                        choose_span(index_span, choose_span(object_span, fallback_span)),
                    ));
                    return InferredExpression::Unknown;
                }
            }

            // Same numeric index signature `String` declares, reached with a
            // literal key (`text[0]`).
            if reads_string_by_number(&receiver_type) && index_is_numeric {
                return InferredExpression::Known(crate::infer::unchecked_index_read(
                    Type::String,
                    ctx,
                ));
            }

            if receiver_is_object_like {
                report_missing_element(
                    &key,
                    &index_type,
                    &receiver_type,
                    index_span,
                    element_access_span(object_span, index_span).or(fallback_span),
                    &site,
                    symbols,
                    ctx,
                );
            }
            InferredExpression::Unknown
        }
    }
}

/// A read through [`surge_ts_types::KeyedIndexes`]: a key no info applies to is
/// tsc's implicit-`any` element access (TS7053), a literal one included —
/// `getApplicableIndexInfoForName` reads it as its string literal.
fn keyed_index_read(
    keyed: &surge_ts_types::KeyedIndexes,
    key: &Type,
    receiver: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if let Some(value) = keyed.read(key) {
        return InferredExpression::Known(crate::infer::unchecked_index_read(value, ctx));
    }
    if ctx.options.no_implicit_any {
        ctx.push(diagnostic_with_syntax_span(
            crate::checks::expr::ts7053_for_index(&key, receiver.name(), ctx.file_name.clone()),
            span,
        ));
    }
    InferredExpression::Unknown
}

/// tsc's `checkIndexedAccessIndexType` for an access deferred over a type
/// variable: its key has to be assignable to `keyof` the receiver (TS2536),
/// and the access is the error type when it is not.
pub(crate) fn report_invalid_deferred_index(
    receiver: &Type,
    index: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let Some(keys) = surge_ts_types::type_variable::keyof_variable(receiver) else {
        return false;
    };
    if is_assignable_to(index, &keys) {
        return false;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2536(index.name(), receiver.name(), ctx.file_name.clone()),
        span,
    ));
    true
}

/// [`report_invalid_deferred_index`] over a union of type variables, whose
/// keys are the intersection of each member's (`getIndexType`).
fn report_invalid_union_variable_index(
    receiver: &Type,
    index: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let Type::Union(union) = receiver else {
        return false;
    };
    let Some(keys) = union
        .types()
        .iter()
        .map(|member| {
            matches!(member, Type::TypeParameter(_))
                .then(|| surge_ts_types::type_variable::keyof_variable(member))
                .flatten()
        })
        .collect::<Option<Vec<Type>>>()
    else {
        return false;
    };
    let keys = crate::infer::types::merge_intersection_members(keys);
    if is_assignable_to(index, &keys) {
        return false;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2536(index.name(), receiver.name(), ctx.file_name.clone()),
        span,
    ));
    true
}

/// tsc's `isForInVariableForNumericPropertyNames`: the key of a `for…in` over
/// an object whose only index signature is numeric indexes as a `number`, even
/// though the binding itself is a `string`.
/// A key an array-like receiver indexes by. tsc converts a numeric-like string
/// literal to its number, so `xs["0"]` reads element 0 and is not an error.
/// The written index of a literal tuple key, including the numeric-like string
/// literal tsc converts to a number. Negative values stay signed: an out-of-range
/// index is reported, not silently read as a member.
fn tuple_literal_index_value(index_type: &Type) -> Option<i64> {
    match index_type {
        Type::NumberLiteral(NumberLiteralType { value }) | Type::StringLiteral(value) => {
            value.parse::<i64>().ok()
        }
        _ => None,
    }
}

/// A receiver that answers a numeric index with a `string`: `String` declares
/// `readonly [index: number]: string`, and a union of string literals answers
/// from every member (`('all' | 'active')[0]`).
fn reads_string_by_number(receiver_type: &Type) -> bool {
    match receiver_type {
        Type::String | Type::StringLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(reads_string_by_number),
        Type::Reference(_) => matches!(
            receiver_type.peeled(),
            Type::String | Type::StringLiteral(_)
        ),
        _ => false,
    }
}

/// tsc's `isApplicableIndexType` against a number index: a key assignable to
/// `number`, `${number}`, or a numeric string literal. A union key is looked
/// up member by member.
fn numeric_index_key(index_type: &Type) -> bool {
    match index_type {
        Type::StringLiteral(value) => value.parse::<f64>().is_ok(),
        Type::Union(union) => union.types().iter().all(numeric_index_key),
        other if is_numeric_string_type(other) => true,
        other => is_assignable_to(other, &Type::Number),
    }
}

fn is_numeric_string_type(ty: &Type) -> bool {
    surge_ts_types::template_literal_parts(ty).is_some_and(|(texts, types)| {
        texts.iter().all(|text| text.is_empty()) && matches!(types.as_slice(), [Type::Number])
    })
}

/// A receiver whose only index is numeric cannot answer any other key. tsc
/// reports a key that could still name a property as the implicit `any` the
/// access reads (TS7015, silent without `noImplicitAny`), and one that cannot
/// name a property at all as TS2538.
fn report_non_numeric_index(
    index_type: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if report_unusable_index_type(index_type, span, ctx) {
        return;
    }
    if !ctx.options.no_implicit_any {
        return;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts7015(ctx.file_name.clone()),
        span,
    ));
}

/// tsc's `getPropertyTypeForIndexType` looks a key up only when it is not
/// nullable and is assignable to `string`, `number` or `symbol`
/// (`isTypeAssignableToKind`). Any other key — an object, a function, a class,
/// `bigint`, `boolean` — is TS2538 on the key whatever the receiver, and the
/// access reads the error type. Nullish and `void` keys are left alone, and so
/// is a key surge could not fully model.
pub(crate) fn report_unusable_index_type(
    index_type: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if !is_unusable_index_type(index_type) {
        return false;
    }
    ctx.push(diagnostic_with_syntax_span(
        Diagnostic::ts2538(index_type.name(), ctx.file_name.clone()),
        span,
    ));
    true
}

fn is_unusable_index_type(index_type: &Type) -> bool {
    match &index_type.peeled() {
        Type::Any
        | Type::Unknown
        | Type::GenuineUnknown
        | Type::ErrorType
        | Type::TypeParameter(_)
        | Type::Never
        | Type::Void
        | Type::Null
        | Type::Undefined
        // tsc resolves a union key one member at a time.
        | Type::Union(_) => false,
        // A branded key (`string & Tag1 & Tag2`) is an intersection surge
        // relates operand by operand, which does not see its primitive.
        Type::Object(object) if object.is_intersection => false,
        Type::Reference(_) => false,
        other => {
            !crate::checks::function::type_contains_degradation(other)
                && !is_assignable_to(other, &Type::String)
                && !is_assignable_to(other, &Type::Number)
                && !is_assignable_to(other, &Type::Symbol)
        }
    }
}

/// What `receiver[index]` reads off an object receiver, for the
/// diagnostic-free inference path: the member a literal key names, or the
/// applicable index signature. `None` wherever the checked path would report.
pub(crate) fn object_element_read(
    receiver_type: &Type,
    index: &ParsedExpression,
    index_type: &Type,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Type> {
    let peeled = receiver_type.peeled();
    let index_is_numeric =
        is_assignable_to(index_type, &Type::Number) || indexes_as_number(index, symbols);
    let well_known = well_known_symbol_key_type(index, symbols);
    let index_type = well_known.as_ref().unwrap_or(index_type);
    let Some(key) = literal_index_key(index_type) else {
        let Type::Object(object_type) = &peeled else {
            return None;
        };
        let key_union = if !index_type.is_type_variable() {
            index_type.clone()
        } else if object_type.applicable_index_type(index_is_numeric).is_some() {
            surge_ts_types::type_variable::base_constraint_or_type(index_type)
        } else {
            Type::Never
        };
        if let Some(members) = declared_literal_union_members(object_type, &key_union) {
            return Some(union_type(members));
        }
        return match object_type.applicable_index_type(index_is_numeric) {
            Some(index_value) => Some(crate::infer::unchecked_index_read(index_value.clone(), ctx)),
            None => union_literal_index_read(object_type, index_type),
        };
    };
    if let Some(element_type) = literal_tuple_element(&peeled, &key) {
        return Some(element_type);
    }
    let Type::Object(object_type) = &peeled else {
        return None;
    };
    literal_key_member_read(object_type, &key, ctx)
        .or_else(|| {
            object_type
                .applicable_index_type(index_is_numeric)
                .map(|index_value| crate::infer::unchecked_index_read(index_value.clone(), ctx))
        })
        .or_else(|| narrowed_variable_element_read(&peeled, index, index_type, symbols, ctx))
}

/// `getPropertyTypeForIndexType` over a narrowed `T & X`. An element access
/// expression defers only on a generic key or a variadic tuple
/// (`shouldDeferIndexedAccessType`), so tsc reads the key off the
/// intersection's apparent type, whose members and index signatures are its
/// operands'. As for a member read, the operand a guard narrowed to answers
/// before the variable's constraint.
fn narrowed_variable_element_read(
    receiver_type: &Type,
    index: &ParsedExpression,
    index_type: &Type,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Type> {
    if !surge_ts_types::type_variable::is_narrowed_type_variable(receiver_type) {
        return None;
    }
    let Type::Object(object) = receiver_type else {
        return None;
    };
    let operands = object.intersection_operands.as_deref()?;
    let index_is_numeric = is_assignable_to(index_type, &Type::Number)
        || indexes_as_number(index, symbols)
        || literal_index_key(index_type).is_some_and(|key| surge_ts_types::is_numeric_key(&key));
    operands
        .iter()
        .filter(|operand| !matches!(operand, Type::TypeParameter(_)))
        .find_map(|operand| match operand.peeled() {
            Type::Array(element) => {
                index_is_numeric.then(|| crate::infer::unchecked_index_read(*element, ctx))
            }
            peeled => object_element_read(&peeled, index, index_type, symbols, ctx),
        })
        .or_else(|| {
            operands
                .iter()
                .find_map(|operand| constraint_element_read(operand, index, index_type, symbols, ctx))
        })
}

/// A literal key's member, which under `noUncheckedIndexedAccess` includes
/// `undefined` when only an index signature answers it
/// (`getPropertyTypeForIndexType` with `AccessFlagsIncludeUndefined`).
fn literal_key_member_read(object_type: &surge_ts_types::ObjectType, key: &str, ctx: &CheckerContext) -> Option<Type> {
    let member = object_type.get_property_access_type(key)?;
    let from_index_signature = object_type.properties.get(key).is_none()
        && !object_type.synthetic_open_index
        && !surge_ts_types::private_name::is_private_name_key(key)
        && object_type.applicable_index_type(surge_ts_types::is_numeric_key(key)).is_some();
    Some(if from_index_signature {
        crate::infer::unchecked_index_read(member, ctx)
    } else {
        member
    })
}

/// tsc resolves `x["k"]` on a type variable `x` eagerly through its apparent
/// type (only a generic index defers the access).
pub(crate) fn constraint_element_read(
    receiver_type: &Type,
    index: &ParsedExpression,
    index_type: &Type,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Type> {
    if !receiver_type.is_type_variable() || index_type.is_type_variable() {
        return None;
    }
    let constraint = crate::checks::call::type_variable_apparent_type(receiver_type)?;
    object_element_read(&constraint, index, index_type, symbols, ctx)
}

fn indexes_as_number(index: &ParsedExpression, symbols: &SymbolTable) -> bool {
    let ParsedExpression::Identifier { name, .. } = index else {
        return false;
    };
    symbols
        .get(name)
        .is_some_and(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::ForInNumericKey))
}

/// The element a literal numeric key selects out of a tuple, distributing over a
/// union of them. `None` when the receiver is not tuple-shaped, or when any
/// member lacks the index — an out-of-range key is a real error, left to the
/// caller.
pub(crate) enum UnionKeyRead {
    Found(Type),
    Missing,
    Undecided,
}

/// tsc's `getPropertyOfType` on a union for a literal numeric key, then its
/// union index infos. A member declaring the key, or answering it through an
/// applicable index signature, contributes its type; the property exists when
/// some member declares it and none lacks it altogether (`ReadPartial`).
/// Failing that, only an index signature of the key's own kind that every
/// member declares answers (`getUnionIndexInfos` matches key types exactly).
/// A member surge did not model, or a nullish one, decides nothing.
pub(crate) fn union_numeric_key_read(members: &[Type], key: &str, ctx: &CheckerContext) -> UnionKeyRead {
    if !surge_ts_types::is_numeric_key(key) {
        return UnionKeyRead::Undecided;
    }
    let mut declared = false;
    let mut partial = false;
    let mut answers = Vec::with_capacity(members.len());
    let mut number_indexes = Some(Vec::with_capacity(members.len()));
    let mut string_indexes = Some(Vec::with_capacity(members.len()));
    for member in members {
        let peeled = member.peeled();
        let (property, number_index, string_index) = match &peeled {
            Type::Object(object) => {
                if object.synthetic_open_index || object.keyed_indexes.is_some() {
                    return UnionKeyRead::Undecided;
                }
                let property = object.properties.contains_key(key).then(|| object.get_property_access_type(key)).flatten();
                (
                    property,
                    object.number_index_type.as_deref().cloned(),
                    object.string_index_type.as_deref().cloned(),
                )
            }
            Type::Tuple(elements) => {
                let element = key.parse::<usize>().ok().and_then(|index| elements.get(index).cloned());
                (element, Some(union_type(elements.clone())), None)
            }
            Type::Array(element) => (None, Some(element.as_ref().clone()), None),
            Type::String | Type::StringLiteral(_) => (None, Some(Type::String), None),
            Type::Number
            | Type::NumberLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Symbol => (None, None, None),
            _ => return UnionKeyRead::Undecided,
        };
        match (&property, &number_index, &string_index) {
            (Some(property), _, _) => {
                declared = true;
                answers.push(property.clone());
            }
            (None, Some(index), _) | (None, None, Some(index)) => {
                answers.push(crate::infer::unchecked_index_read(index.clone(), ctx));
            }
            (None, None, None) => partial = true,
        }
        match (number_indexes.as_mut(), number_index) {
            (Some(indexes), Some(index)) => indexes.push(index),
            _ => number_indexes = None,
        }
        match (string_indexes.as_mut(), string_index) {
            (Some(indexes), Some(index)) => indexes.push(index),
            _ => string_indexes = None,
        }
    }
    if declared && !partial {
        return UnionKeyRead::Found(union_type(answers));
    }
    match number_indexes.or(string_indexes) {
        Some(indexes) => UnionKeyRead::Found(crate::infer::unchecked_index_read(union_type(indexes), ctx)),
        None => UnionKeyRead::Missing,
    }
}

fn literal_tuple_element(ty: &Type, key: &str) -> Option<Type> {
    match ty {
        Type::Tuple(elements) => elements.get(key.parse::<usize>().ok()?).cloned(),
        Type::Union(union) => {
            let members = union.types();
            if members.is_empty() {
                return None;
            }
            let mut selected = Vec::with_capacity(members.len());
            for member in members {
                selected.push(literal_tuple_element(&member.peeled(), key)?);
            }
            Some(union_type(selected))
        }
        // An intersection with a tuple operand reads the element off it
        // (`LuaTuple<T> = T & { readonly LUA_TUPLE: never }`), which the
        // merged property map does not carry.
        Type::Object(object) if object.is_intersection => object
            .intersection_operands
            .as_deref()?
            .iter()
            .find_map(|operand| literal_tuple_element(&operand.peeled(), key)),
        _ => None,
    }
}

/// The member name a literal computed key stands for. A string index signature
/// is keyed by the *string* form, so `1` and `"1"` name the same member.
/// A union of literal keys indexes like each of them: `o[k]` with `k: "a" | "b"`
/// reads `o["a"] | o["b"]`, and is only a missing key when one of them is. The
/// union never reaches [`literal_index_key`], so without this it would look like
/// a non-literal key the receiver cannot answer.
/// The members a union of literal keys names when every one of them is a
/// declared member, none read through an index signature.
fn declared_literal_union_members(object_type: &surge_ts_types::ObjectType, index_type: &Type) -> Option<Vec<Type>> {
    let Type::Union(union) = index_type else {
        return None;
    };
    let mut member_types = Vec::with_capacity(union.types().len());
    for member in union.types() {
        let key = literal_index_key(member)?;
        if object_type.properties.get(key.as_str()).is_none_or(|property| property.index_slot) {
            return None;
        }
        member_types.push(object_type.get_property_access_type(&key)?);
    }
    Some(member_types)
}

fn union_literal_index_read(
    object_type: &surge_ts_types::ObjectType,
    index_type: &Type,
) -> Option<Type> {
    let Type::Union(union) = index_type else {
        return None;
    };
    let members = union.types();
    if members.is_empty() {
        return None;
    }
    let mut member_types = Vec::with_capacity(members.len());
    for member in members {
        let key = literal_index_key(member)?;
        member_types.push(object_type.get_property_access_type(&key)?);
    }
    Some(union_type(member_types))
}

/// Whether an index type is specific enough to hold the receiver to it. A
/// degraded or still-open key (`any` because surge could not model the
/// expression, an unresolved reference, an uninstantiated type parameter)
/// answers nothing about which member is read.
/// Whether every member of an enum key names a member of `object` — or the
/// member's value could not be read, which says nothing either way.
fn enum_key_names_members(index_type: &Type, object: &surge_ts_types::ObjectType) -> bool {
    let is_enum_member = |ty: &Type| matches!(ty, Type::Reference(member) if member.enum_owner.is_some());
    let members = match index_type.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    if !is_enum_member(index_type) && !members.iter().all(is_enum_member) {
        return false;
    }
    members.iter().all(|member| match member.peeled() {
        Type::StringLiteral(value) => object.properties.contains_key(value.as_str()),
        Type::NumberLiteral(NumberLiteralType { value }) => object.properties.contains_key(value.as_str()),
        _ => !matches!(member, Type::Reference(reference) if reference.enum_owner.is_some()),
    })
}

/// tsc's `getPropertyTypeForIndexType` on `typeof globalThis`, which has no
/// index signature: a `string` or `number` key names no global, so under
/// `noImplicitAny` the access is an implicit `any` (TS7053).
pub(crate) fn report_global_this_element_access(
    receiver_type: &Type,
    index_type: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    if !ctx.options.no_implicit_any
        || !matches!(index_type, Type::String | Type::Number)
        || !matches!(receiver_type, Type::Reference(reference)
            if &*reference.id == crate::driver::GLOBAL_THIS_REFERENCE_ID)
    {
        return false;
    }
    ctx.push(diagnostic_with_syntax_span(
        crate::checks::expr::ts7053_for_index(&index_type, "typeof globalThis", ctx.file_name.clone()),
        span,
    ));
    true
}

fn index_key_is_concrete(index_type: &Type) -> bool {
    let is_enum_member = |ty: &Type| matches!(ty, Type::Reference(member) if member.enum_owner.is_some());
    match index_type {
        // An enum's own type is the union of its members.
        Type::Reference(reference) => {
            reference.is_unique_symbol()
                || reference.enum_owner.is_some()
                || matches!(index_type.peeled(), Type::Union(members) if members.types().iter().all(is_enum_member))
        }
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_) => false,
        _ => true,
    }
}

fn literal_index_key(index_type: &Type) -> Option<String> {
    match index_type {
        Type::StringLiteral(value) => Some(value.clone()),
        Type::NumberLiteral(NumberLiteralType { value }) => Some(value.clone()),
        Type::BooleanLiteral(value) => Some(value.to_string()),
        // Only a well-known symbol's key names its member as written
        // (`[Symbol.iterator]`); a declared unique symbol's members are named
        // by the expression each declaration wrote.
        Type::Reference(reference) if reference.is_unique_symbol() => reference
            .unique_symbol_name()
            .filter(|name| name.starts_with("Symbol."))
            .map(|name| format!("[{name}]")),
        _ => None,
    }
}

/// The type of a `Symbol.<name>` key naming one of the lib's well-known
/// symbols: a unique symbol, which names the member a class or interface
/// declares as `[Symbol.<name>]`. surge types the lib's `unique symbol`
/// members as `symbol`, so the key is recognised by what it reads.
/// The member a well-known symbol key reads off an array or tuple's apparent
/// `Array<T>`, which declares it under the same `[Symbol.<name>]`.
fn well_known_symbol_member(receiver: &Type, index: &ParsedExpression, symbols: &SymbolTable) -> Option<Type> {
    let Type::Reference(key) = well_known_symbol_key_type(index, symbols)? else {
        return None;
    };
    receiver.get_property_access_type(&format!("[{}]", key.unique_symbol_name()?))
}

pub(crate) fn well_known_symbol_key_type(index: &ParsedExpression, symbols: &SymbolTable) -> Option<Type> {
    let ParsedExpression::PropertyAccess {
        object,
        property_name,
        is_bracketed: false,
        ..
    } = index
    else {
        return None;
    };
    let ParsedExpression::Identifier { name, .. } = object.as_ref() else {
        return None;
    };
    (name == "Symbol"
        && symbols.get(name).is_some()
        && crate::program::WELL_KNOWN_SYMBOLS.contains(&property_name.as_str()))
    .then(|| surge_ts_types::unique_symbol_type("", &format!("Symbol.{property_name}")))
}

pub(super) fn report_tuple_index_out_of_bounds(
    elements: &[Type],
    index_value: i64,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    if span.is_some_and(|span| ctx.allow_missing_tuple_elements.contains(&span)) {
        return;
    }
    let diagnostic = if index_value < 0 {
        Diagnostic::ts2514(ctx.file_name.clone())
    } else {
        Diagnostic::ts2493(
            Type::Tuple(elements.to_vec()).name(),
            elements.len(),
            index_value,
            ctx.file_name.clone(),
        )
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}
