//! Object/array literal inference.

use super::*;

use std::time::Instant;

use surge_ts_syntax::{
    ParsedArrayElement, ParsedArrowFunctionBody, ParsedExpression, ParsedObjectProperty,
};
use surge_ts_types::{
    ObjectProperty, PropertyMap, Type, TypeCopyReason, union_type, with_type_copy_reason,
};

use crate::checks::function::check_arrow_function_expression;
use crate::context::CheckerContext;
use crate::metrics::alloc_object_type;
use crate::program::{
    record_object_literal_property_check, record_program_timing, record_property_lookup,
};
use crate::symbols::{SymbolInfo, SymbolKind, SymbolTable};

use crate::infer::InferredExpression;

/// tsc names a computed property by its key's type (`checkComputedPropertyName`):
/// a string or number literal key is that property, and a `string`/`number` key
/// adds no named member at all. The written path stays the name only for keys
/// surge cannot type, which keeps well-known symbols (`[Symbol.iterator]`) as
/// they were.
pub(crate) fn resolve_computed_property_names<'a>(
    properties: &'a [ParsedObjectProperty],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> std::borrow::Cow<'a, [ParsedObjectProperty]> {
    if properties.iter().all(|property| property.computed_key.is_none()) {
        return std::borrow::Cow::Borrowed(properties);
    }
    let mut resolved = Vec::with_capacity(properties.len());
    for property in properties {
        let Some(key) = property.computed_key.as_deref() else {
            resolved.push(property.clone());
            continue;
        };
        // The key is re-inferred by every pass that reads the literal; its own
        // diagnostics belong to the pass that walks it, not to this lookup.
        let diagnostics_before = ctx.diagnostics().len();
        let key_type = infer_expression(key, symbols, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        let InferredExpression::Known(key_type) = key_type else {
            resolved.push(property.clone());
            continue;
        };
        // A unique symbol key names the member by the symbol's declaration,
        // whichever alias the key reads it through.
        if let Type::Reference(reference) = &key_type
            && let Some(declaration) = reference.unique_symbol_name()
        {
            let mut renamed = property.clone();
            renamed.name = format!("[{declaration}]");
            resolved.push(renamed);
            continue;
        }
        match key_type.peeled() {
            Type::StringLiteral(name) => {
                let mut renamed = property.clone();
                renamed.name = name;
                resolved.push(renamed);
            }
            Type::NumberLiteral(literal) => {
                let mut renamed = property.clone();
                renamed.name = literal.value;
                resolved.push(renamed);
            }
            Type::String | Type::Number | Type::Any => {}
            _ => resolved.push(property.clone()),
        }
    }
    std::borrow::Cow::Owned(resolved)
}

/// checker.go `checkObjectLiteral`: a computed name typed `string`, or
/// `number` (which `any` is assignable to), adds no member but gives the
/// literal that index signature, whose value is the union of the members
/// after the last spread it applies to (`getObjectLiteralIndexInfo`).
pub(crate) fn computed_key_index_signatures(
    properties: &[ParsedObjectProperty],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Option<Type>, Option<Type>)> {
    if properties.iter().all(|property| property.computed_key.is_none()) {
        return None;
    }
    // A key the parser could not name travels as an empty spread carrying
    // the member's value; it is a member, not a spread.
    let is_spread = |property: &ParsedObjectProperty| property.is_spread && property.unnamed_key_value.is_none();
    let offset = properties.iter().rposition(is_spread).map_or(0, |index| index + 1);
    let diagnostics_before = ctx.diagnostics().len();
    let (mut string_index, mut number_index) = (false, false);
    let (mut string_members, mut number_members) = (Vec::new(), Vec::new());
    for property in &properties[offset..] {
        if is_spread(property) {
            continue;
        }
        // `Some(numeric)` for a member keyed by an index signature, `None` for
        // a named one.
        let (index_key, name) = match property.computed_key.as_deref() {
            None => (None, property.name.clone()),
            Some(key) => match infer_expression(key, symbols, ctx) {
                InferredExpression::Known(key_type) => match key_type.peeled() {
                    Type::String => (Some(false), String::new()),
                    Type::Number | Type::Any => (Some(true), String::new()),
                    Type::StringLiteral(name) => (None, name),
                    Type::NumberLiteral(literal) => (None, literal.value),
                    _ => continue,
                },
                _ => continue,
            },
        };
        if index_key.is_none() && name.starts_with('[') {
            continue;
        }
        let numeric = index_key.unwrap_or_else(|| surge_ts_types::is_numeric_key(&name));
        string_index |= index_key == Some(false);
        number_index |= index_key == Some(true);
        let property_type = match property.unnamed_key_value.as_deref() {
            Some(value) => infer_object_property_value(value, symbols, ctx),
            None => infer_object_property_type(property, None, symbols, ctx),
        };
        if numeric {
            number_members.push(property_type.clone());
        }
        string_members.push(property_type);
    }
    ctx.truncate_diagnostics(diagnostics_before);
    (string_index || number_index).then(|| {
        (
            string_index.then(|| union_type(string_members)),
            number_index.then(|| union_type(number_members)),
        )
    })
}

/// tsc's `checkObjectLiteral`. A member that reads its own `this` is handed
/// the one `getContextualThisParameterType` gives it (see
/// [`literal_member_this`]).
pub(crate) fn infer_object_literal(
    properties: &[ParsedObjectProperty],
    span: Option<surge_ts_syntax::TextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let Some(takers) = literal_this_takers(properties, span, ctx) else {
        return infer_object_literal_members(properties, None, symbols, ctx);
    };
    if !literal_types_member_this(ctx) {
        let member_this = LiteralMemberThis { ty: Type::Any, takers };
        return infer_object_literal_members(properties, Some(&member_this), symbols, ctx);
    }
    // The literal's type needs its members' bodies, which read `this`: it is
    // typed once with `this` unmodelled, what that reports discarded, and its
    // members are then checked against that type.
    let before = ctx.diagnostics().len();
    let sketch = LiteralMemberThis { ty: Type::Unknown, takers };
    let sketched = infer_object_literal_members(properties, Some(&sketch), symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(before);
    let member_this = LiteralMemberThis {
        ty: crate::checks::expr::widen_type(&sketched),
        takers,
    };
    infer_object_literal_members(properties, Some(&member_this), symbols, ctx)
}

/// Which of an object literal's members take the `this` it hands them.
#[derive(Clone, Copy)]
pub(crate) enum LiteralThisTakers {
    /// The members the grammar walk found reading `this` in a literal it saw
    /// has no contextual type.
    Readers,
    /// Every member function without a `this` of its own: the literal is an
    /// assignment declaration's value, which has no contextual type either.
    Members,
}

/// The `this` an object literal hands the members that take it.
pub(crate) struct LiteralMemberThis {
    ty: Type,
    takers: LiteralThisTakers,
}

impl LiteralMemberThis {
    /// Hands `this` to the arrow check about to run on `member` if it takes
    /// it. `method` for a method or accessor, whose `this` parameter the
    /// lowering does not keep.
    pub(crate) fn hand_to(
        &self,
        member: &surge_ts_syntax::ParsedArrowFunction,
        method: bool,
        ctx: &mut CheckerContext,
    ) {
        use surge_ts_syntax::ParsedThisBinding;
        let takes = match self.takers {
            LiteralThisTakers::Readers => reads_literal_this(member, ctx),
            LiteralThisTakers::Members if method => member.this_binding != ParsedThisBinding::Inherited,
            LiteralThisTakers::Members => member.this_binding == ParsedThisBinding::ImplicitAny,
        };
        ctx.next_arrow_this = takes.then(|| self.ty.clone());
    }
}

fn reads_literal_this(member: &surge_ts_syntax::ParsedArrowFunction, ctx: &CheckerContext) -> bool {
    member.this_binding != surge_ts_syntax::ParsedThisBinding::Inherited
        && member
            .span
            .and_then(|span| u32::try_from(span.start).ok())
            .is_some_and(|start| ctx.literal_this_members.binary_search(&start).is_ok())
}

/// Which members of the literal at `span` take its `this`, if any does.
fn literal_this_takers(
    properties: &[ParsedObjectProperty],
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &CheckerContext,
) -> Option<LiteralThisTakers> {
    if ctx.expando_object_literal.is_some()
        && span.and_then(|span| u32::try_from(span.start).ok()) == ctx.expando_object_literal
    {
        return Some(LiteralThisTakers::Members);
    }
    if ctx.literal_this_members.is_empty() {
        return None;
    }
    properties
        .iter()
        .any(|property| {
            matches!(&property.value, ParsedExpression::ArrowFunction(member) if reads_literal_this(member, ctx))
                || property
                    .paired_setter
                    .as_deref()
                    .is_some_and(|setter| reads_literal_this(setter, ctx))
        })
        .then_some(LiteralThisTakers::Readers)
}

/// `getContextualThisParameterType` types an object-literal member's `this`
/// by its literal only under `noImplicitThis`, and always in JavaScript;
/// otherwise it is `any`.
fn literal_types_member_this(ctx: &CheckerContext) -> bool {
    ctx.options.no_implicit_this || surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
}

/// The `this` the object literal at `span`, of type `literal_type`, hands its
/// members: the literal widened (`getWidenedType`) where it has no contextual
/// type, `any` where `this` is not typed by it at all.
pub(crate) fn literal_member_this(
    properties: &[ParsedObjectProperty],
    span: Option<surge_ts_syntax::TextSpan>,
    literal_type: Option<&Type>,
    ctx: &CheckerContext,
) -> Option<LiteralMemberThis> {
    let takers = literal_this_takers(properties, span, ctx)?;
    let ty = if literal_types_member_this(ctx) {
        literal_type.map_or(Type::Unknown, crate::checks::expr::widen_type)
    } else {
        Type::Any
    };
    Some(LiteralMemberThis { ty, takers })
}

#[derive(Default)]
struct SpreadIndex {
    string: Option<Type>,
    number: Option<Type>,
}

impl SpreadIndex {
    fn of(source: &surge_ts_types::ObjectType) -> Self {
        Self {
            string: source.string_index_type.as_deref().cloned(),
            number: source.number_index_type.as_deref().cloned(),
        }
    }

    fn union(self, right: Self) -> Self {
        let both = |left: Option<Type>, right: Option<Type>| Some(union_type(vec![left?, right?]));
        Self {
            string: both(self.string, right.string),
            number: both(self.number, right.number),
        }
    }
}

fn infer_object_literal_members(
    properties: &[ParsedObjectProperty],
    member_this: Option<&LiteralMemberThis>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let computed_index_signatures = computed_key_index_signatures(properties, symbols, ctx);
    let properties = &*resolve_computed_property_names(properties, symbols, ctx);
    let closure_symbols = object_member_closure_symbols(properties, symbols);
    let member_symbols = closure_symbols.as_ref().unwrap_or(symbols);
    let object_literal_start = Instant::now();
    // tsc's `getSpreadType` distributes over a spread union with more than one
    // non-empty object member, so the literal is one object per alternative.
    let mut alternatives: Vec<PropertyMap> = vec![PropertyMap::default()];
    // A spread source that surge could not fully enumerate carries
    // `synthetic_open_index`. The members it stands for are real — the derived
    // interface was kept open precisely because they exist — so the spread
    // result must stay open too; a closed result reports every one of them as a
    // missing property (tanstack's `defaultQueryOptions` spreads
    // `QueryObserverOptions`, whose `extends WithRequired<QueryOptions<…>,
    // 'queryKey'>` base degrades, and read `queryHash`/`networkMode`/`persister`
    // /`queryFn` off the result). A *declared* index signature is deliberately
    // not propagated here; only surge's own openness marker is.
    let mut spread_source_is_open = false;
    let mut spread_source_is_any = false;
    let mut spread_variables: Vec<Type> = Vec::new();
    // `getSpreadType`'s index signatures: the first spread's own, then only
    // those every later operand (a run of written properties included)
    // declares too (`getUnionIndexInfos`). `None` before any spread.
    let mut spread_index: Option<SpreadIndex> = None;
    let mut written_since_spread = false;
    for property in properties {
        record_property_lookup();
        record_object_literal_property_check();

        if property.is_spread {
            let left = if std::mem::take(&mut written_since_spread) {
                Some(SpreadIndex::default())
            } else {
                spread_index.take()
            };
            spread_index = Some(SpreadIndex::default());
            // `{ ...source }` merges `source`'s own properties; later properties
            // (including later spreads) override earlier ones, matching tsc's
            // left-to-right spread semantics. The source is peeled so a nominal
            // reference (`const d: Props = …; { ...d }`) contributes its members.
            // A spread whose type we cannot model as an object is skipped rather
            // than collapsing the whole literal.
            let spread_value = infer_object_property_value(&property.value, symbols, ctx);
            let unspreadable = crate::checks::expr::class_unspreadable_members(&spread_value, ctx);
            match spread_value.peeled() {
                // `{ ...anyValue, k: v }` is `any` in tsc: the spread can carry
                // anything, so the literal has no knowable shape at all. The
                // remaining properties are still walked for their own
                // diagnostics; only the resulting type collapses.
                // tsc's error type is an `any` too.
                Type::Any | Type::ErrorType => {
                    spread_source_is_any = true;
                }
                // Surge's own degradation sentinel: the members it stands for
                // are real but unenumerable, so the result must stay open — as
                // must a type parameter's, which tsc keeps as `T & { … }`.
                Type::Unknown => {
                    spread_source_is_open = true;
                }
                // tsc's `getSpreadType` keeps a generic spread as an
                // intersection (`T & { error: … }`), so the variable stays
                // beside the merged members.
                source @ Type::TypeParameter(_) => {
                    spread_source_is_open = true;
                    if source.is_type_variable() {
                        spread_variables.push(source);
                    }
                }
                Type::Object(source) if surge_ts_types::type_variable::is_narrowed_type_variable(&Type::Object(source.clone())) => {
                    spread_source_is_open = true;
                    spread_variables.extend(
                        source
                            .intersection_operands
                            .as_deref()
                            .unwrap_or_default()
                            .iter()
                            .filter(|operand| operand.is_type_variable())
                            .cloned(),
                    );
                }
                Type::Object(mut source) => {
                    if !unspreadable.is_empty() {
                        let mut properties = (*source.properties).clone();
                        for name in &unspreadable {
                            properties.shift_remove(name.as_str());
                        }
                        source = crate::metrics::alloc_object_type(properties, source.string_index_type.as_deref().cloned())
                            .with_number_index_type(source.number_index_type.as_deref().cloned());
                    }
                    spread_source_is_open |= source.synthetic_open_index;
                    let operand = SpreadIndex::of(&source);
                    spread_index = Some(match left {
                        None => operand,
                        Some(left) => left.union(operand),
                    });
                    // `getSpreadSymbol`: a set-only accessor spreads as
                    // `undefined`. Only a spread literal still says which
                    // members are set-only.
                    let set_only: Vec<&str> = match &property.value {
                        ParsedExpression::ObjectLiteral { properties: spread, .. } => spread
                            .iter()
                            .filter(|member| {
                                member.is_accessor
                                    && !member.is_getter
                                    && !spread.iter().any(|other| other.is_getter && other.name == member.name)
                            })
                            .map(|member| member.name.as_str())
                            .collect(),
                        _ => Vec::new(),
                    };
                    for merged_properties in &mut alternatives {
                        merge_object_spread(&source, merged_properties);
                        for name in &set_only {
                            merged_properties.insert((*name).into(), ObjectProperty::required(Type::Undefined));
                        }
                    }
                }
                // `{ ...(cond ? { list } : {}) }`: spreading a union contributes
                // each member's properties, and a property absent from (or
                // optional in) some member becomes optional — the shape tsc
                // infers for a conditional spread.
                Type::Union(source) => {
                    spread_source_is_open |= source.types().iter().any(|member| {
                        matches!(member.peeled(), Type::Object(object) if object.synthetic_open_index)
                    });
                    match distributed_spread_members(&source) {
                        Some(members)
                            if alternatives.len() * members.len() <= MAX_SPREAD_ALTERNATIVES =>
                        {
                            alternatives = alternatives
                                .iter()
                                .flat_map(|merged_properties| {
                                    members.iter().map(move |member| {
                                        let mut merged_properties = merged_properties.clone();
                                        if let Some(member) = member {
                                            merge_object_spread(member, &mut merged_properties);
                                        }
                                        merged_properties
                                    })
                                })
                                .collect();
                        }
                        _ => {
                            for merged_properties in &mut alternatives {
                                merge_union_spread(&source, merged_properties);
                            }
                        }
                    }
                }
                _ => continue,
            }
            continue;
        }

        written_since_spread = true;
        let readonly = is_get_only_accessor(property, properties);
        let property_symbols = if matches!(property.value, ParsedExpression::ArrowFunction(_)) {
            member_symbols
        } else {
            symbols
        };
        let property_type = infer_object_property_type(property, member_this, property_symbols, ctx);
        for merged_properties in &mut alternatives {
            merged_properties.insert(
                property.name.as_str().into(),
                ObjectProperty::required(property_type.clone()).with_readonly(readonly),
            );
        }
    }

    // tsc's `isJSLiteralType`: without noImplicitAny an object literal written
    // in a JavaScript file is open-ended, and a member it does not declare
    // reads as `any` wherever the object is used.
    let javascript_literal = !ctx.options.no_implicit_any
        && surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    let build = |merged_properties: PropertyMap| {
        if spread_source_is_open || javascript_literal {
            let mut object = alloc_object_type(merged_properties, Some(Type::Any));
            object = object.with_open_index_marker();
            if !spread_variables.is_empty() {
                object = object
                    .with_intersection_marker()
                    .with_intersection_operands(spread_variables.clone());
            }
            Type::Object(object)
        } else if let Some((string_index, number_index)) = computed_index_signatures.clone() {
            Type::Object(alloc_object_type(merged_properties, string_index).with_number_index_type(number_index))
        } else if let Some(index) = spread_index.as_ref().filter(|_| !written_since_spread) {
            Type::Object(
                alloc_object_type(merged_properties, index.string.clone())
                    .with_number_index_type(index.number.clone()),
            )
        } else {
            Type::Object(alloc_object_type(merged_properties, None))
        }
    };
    let result = if spread_source_is_any {
        Type::Any
    } else if alternatives.len() == 1 {
        build(alternatives.pop().unwrap_or_default())
    } else {
        union_type(alternatives.into_iter().map(build).collect())
    };
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.object_literal_checking += object_literal_start.elapsed()
    });
    result
}

pub(crate) fn object_member_closure_symbols(
    properties: &[ParsedObjectProperty],
    symbols: &SymbolTable,
) -> Option<SymbolTable> {
    let mut written = Vec::new();
    for property in properties {
        let ParsedExpression::ArrowFunction(arrow) = &property.value else {
            continue;
        };
        if let ParsedArrowFunctionBody::Block(body) = &arrow.body {
            let parameters: Vec<_> = arrow
                .parameters
                .iter()
                .flat_map(|parameter| parameter.binding_name.bound_names())
                .map(|bound| bound.name)
                .collect();
            written.extend(
                crate::checks::function::deep_assigned_names(&[body])
                    .into_iter()
                    .filter(|name| {
                        !parameters.contains(name) && !object_member_body_binds(body, name)
                    }),
            );
        }
    }
    let mut widened = None;
    for name in written {
        let (Some(symbol), Some(declared)) = (symbols.get(&name), symbols.declared_type(&name)) else {
            continue;
        };
        if !matches!(symbol.kind, SymbolKind::Let | SymbolKind::Var) || symbol.ty == *declared {
            continue;
        }
        let table = widened
            .get_or_insert_with(|| symbols.clone_with_reason(TypeCopyReason::ExpressionInference));
        table.insert(
            name,
            SymbolInfo {
                ty: declared.clone(),
                kind: symbol.kind,
                function_signature: symbol.function_signature.clone(),
            },
        );
    }
    widened
}

fn object_member_body_binds(body: &[surge_ts_syntax::ParsedFunctionBodyStatement], name: &str) -> bool {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    body.iter().any(|statement| match statement {
        Statement::VariableDeclaration(variable) => variable.name == name,
        Statement::Function(function) => function.name == name,
        Statement::Class(class) => class.name == name,
        Statement::Block(body) => object_member_body_binds(body, name),
        Statement::If(statement) => {
            object_member_body_binds(&statement.then_body, name)
                || object_member_body_binds(&statement.else_body, name)
        }
        Statement::While(statement) => object_member_body_binds(&statement.body, name),
        Statement::ForOf(statement) => {
            statement.binding_name.bound_names().iter().any(|bound| bound.name == name)
                || object_member_body_binds(&statement.body, name)
        }
        Statement::Switch(statement) => statement.cases.iter()
            .any(|case| object_member_body_binds(&case.consequent, name)),
        Statement::Try(statement) => {
            object_member_body_binds(&statement.block, name)
                || object_member_body_binds(&statement.finalizer, name)
                || statement.handler.as_ref().is_some_and(|handler| {
                    handler.binding_name.as_ref().is_some_and(|binding| {
                        binding.bound_names().iter().any(|bound| bound.name == name)
                    }) || object_member_body_binds(&handler.body, name)
                })
        }
        _ => false,
    })
}

/// `checkCrossProductUnion` stops tsc at 100,000 spread alternatives; surge
/// folds far earlier into the merged approximation below.
const MAX_SPREAD_ALTERNATIVES: usize = 64;

/// One object spread into the members merged so far.
fn merge_object_spread(source: &surge_ts_types::ObjectType, merged_properties: &mut PropertyMap) {
    for (name, source_property) in source.properties.iter() {
        // `isSpreadableProperty`: a private name stays behind.
        if surge_ts_types::private_name::is_private_name_key(name) {
            continue;
        }
        // An *optional* source property may not be carried at
        // all, so an earlier property of the same name survives:
        // tsc types the result as the union of both (with
        // `undefined` dropped from the spread side) and keeps
        // the earlier property's optionality. Replacing it
        // outright made zustand's `let options = { partialize:
        // (s) => s, …, ...baseOptions }` read `partialize` as
        // possibly-undefined and uncallable.
        let merged = match merged_properties.get(name.as_ref()) {
            Some(existing) if source_property.optional => surge_ts_types::ObjectProperty {
                ty: surge_ts_types::union_type(vec![
                    existing.ty.clone(),
                    surge_ts_types::remove_undefined(&source_property.ty),
                ]),
                optional: existing.optional,
                method: existing.method || source_property.method,
                readonly: false,
                restriction: None,
                index_slot: false,
            },
            // `getSpreadSymbol`: outside a const context a
            // spread member is writable whatever it was on the
            // source.
            _ => source_property.clone().with_readonly(false),
        };
        merged_properties.insert(name.clone(), merged);
    }
}

/// The members a spread union distributes over (`getSpreadType` after
/// `tryMergeUnionOfObjectTypeAndEmptyObject`): only when at least two of them
/// are objects with members; an empty object, `null`, `undefined` or a
/// primitive spreads nothing (`None`). A union with anything surge cannot see
/// through is left to the merged approximation.
fn distributed_spread_members(
    source: &surge_ts_types::UnionType,
) -> Option<Vec<Option<surge_ts_types::ObjectType>>> {
    let mut members = Vec::new();
    let mut objects = 0;
    for member in source.types().iter() {
        match member.peeled() {
            Type::Object(object)
                if object.synthetic_open_index
                    || surge_ts_types::type_variable::is_narrowed_type_variable(&Type::Object(
                        object.clone(),
                    )) =>
            {
                return None;
            }
            Type::Object(object) => {
                let empty = object.properties.is_empty()
                    && object.string_index_type.is_none()
                    && object.number_index_type.is_none()
                    && object.call_signature().is_none()
                    && object.construct_signature().is_none();
                if empty {
                    members.push(None);
                } else {
                    objects += 1;
                    members.push(Some(object));
                }
            }
            Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::Number
            | Type::NumberLiteral(_)
            | Type::String
            | Type::StringLiteral(_)
            | Type::BigInt => members.push(None),
            _ => return None,
        }
    }
    (objects >= 2).then_some(members)
}

fn merge_union_spread(source: &surge_ts_types::UnionType, merged: &mut PropertyMap) {
    // A member written as a named type (`IdleResult<T> | ErrorResult<T>`)
    // spreads the members it names.
    let peeled: Vec<Type> = source.types().iter().map(Type::peeled).collect();
    let members: Vec<Option<&surge_ts_types::ObjectType>> = peeled
        .iter()
        .map(|member| match member {
            Type::Object(object) => Some(object),
            // `undefined`/`null` (and anything else with no own properties)
            // contributes nothing but still makes the other members' properties
            // optional.
            _ => None,
        })
        .collect();

    if members.iter().all(Option::is_none) {
        return;
    }

    let mut names: Vec<std::sync::Arc<str>> = Vec::new();
    for member in members.iter().flatten() {
        for (name, _) in member.properties.iter() {
            if !names.contains(name) && !surge_ts_types::private_name::is_private_name_key(name) {
                names.push(name.clone());
            }
        }
    }

    for name in names {
        let mut present_types = Vec::new();
        let mut missing_or_optional = false;
        for member in &members {
            match member.and_then(|object| object.properties.get(&name)) {
                Some(property) => {
                    present_types.push(property.ty.clone());
                    missing_or_optional |= property.is_optional();
                }
                None => missing_or_optional = true,
            }
        }

        // An earlier required property is not weakened by a later conditional
        // spread: `{ a: 1, ...(cond ? { a: 2 } : {}) }` still always has `a`.
        let already_required = merged.get(&name).is_some_and(ObjectProperty::is_required);
        let ty = union_type(present_types);
        merged.insert(
            name,
            if missing_or_optional && !already_required {
                ObjectProperty::optional(ty)
            } else {
                ObjectProperty::required(ty)
            },
        );
    }
}

/// A `[…] as const` argument, typed the way the assertion says: an array literal
/// is a tuple and a nested one is a nested tuple, all the way down. The inference
/// sketch previously forwarded straight through the assertion, so
/// `flatMap(range, (x) => [[x, x, x]] as const)` handed `number[][]` to the
/// inference of `readonly B[]` and `B` came out as `number[]` instead of the inner
/// tuple. Leaves go through ordinary inference, so their literal types survive.
pub(crate) fn infer_const_expression(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let result = infer_const_expression_unrecorded(expression, symbols, ctx);
    if let InferredExpression::Known(ty) = &result {
        crate::semantic::record_const_operand_type(expression, ty, ctx);
    }
    result
}

fn infer_const_expression_unrecorded(
    expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match expression {
        ParsedExpression::ArrayLiteral { elements, .. } => {
            let mut element_types = Vec::with_capacity(elements.len());
            for element in elements {
                match infer_const_expression(&element.expression, symbols, ctx) {
                    InferredExpression::Known(ty) if !ty.is_unknown() => element_types.push(ty),
                    _ => return InferredExpression::Unknown,
                }
            }
            // `as const`: a readonly tuple, exactly as the annotation form.
            InferredExpression::Known(crate::infer::types::readonly_reference(
                Type::Tuple(element_types),
            ))
        }
        ParsedExpression::ObjectLiteral { properties, .. }
            if !properties.iter().any(|property| property.is_spread) =>
        {
            let mut members = surge_ts_types::PropertyMap::default();
            for property in properties {
                match infer_const_expression(&property.value, symbols, ctx) {
                    InferredExpression::Known(ty) if !ty.is_unknown() => {
                        members.insert(
                            property.name.as_str().into(),
                            // `as const`: every property is read-only.
                            surge_ts_types::ObjectProperty::required(ty).with_readonly(true),
                        );
                    }
                    _ => return InferredExpression::Unknown,
                }
            }
            InferredExpression::Known(Type::Object(crate::metrics::alloc_object_type(
                members, None,
            )))
        }
        // tsc's `checkTemplateExpression` in a const context: the template is
        // typed by its own pattern, `` `*${s}*` `` rather than `string`.
        ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        } if !expressions.is_empty() => {
            let inferred = infer_expression(expression, symbols, ctx);
            match template_expression_pattern_type(expressions, quasis, symbols, ctx) {
                Some(pattern) => InferredExpression::Known(pattern),
                None => inferred,
            }
        }
        _ => infer_expression(expression, symbols, ctx),
    }
}

pub(crate) fn infer_array_literal(
    elements: &[ParsedArrayElement],
    tuple_context: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if tuple_context && elements.iter().all(|element| !element.spread) {
        return infer_tuple_literal(elements, symbols, ctx);
    }
    // `[]` is `never[]`, or `undefined[]` when `undefined` is in every type's
    // domain (the implicit element type tsc widens to `any`).
    if elements.is_empty() {
        let element = if surge_ts_types::strict_null_checks() {
            Type::Never
        } else {
            Type::Undefined
        };
        return InferredExpression::Known(Type::Array(Box::new(element)));
    }

    let mut element_types = Vec::new();
    let mut spread_element_types = Vec::new();

    for element in elements {
        match infer_expression(&element.expression, symbols, ctx) {
            InferredExpression::Known(Type::Any) => {
                return InferredExpression::Known(Type::Array(Box::new(Type::Any)));
            }
            InferredExpression::Known(Type::Unknown)
            | InferredExpression::Known(Type::GenuineUnknown)
            | InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => {
                return InferredExpression::Unknown;
            }
            // `[...xs]` contributes what iterating `xs` yields, not `xs`. A
            // shape surge cannot iterate leaves the whole literal untyped
            // rather than claiming an element type it did not derive.
            InferredExpression::Known(ty) if element.spread => {
                let yielded = crate::checks::function::for_of_element_type(&ty);
                if yielded.is_unknown() {
                    return InferredExpression::Unknown;
                }
                spread_element_types.push(yielded);
            }
            // Only a fresh literal widens through its members. An element read
            // from a declared type (`[envelope.result]`) keeps the literal
            // members its declaration wrote — `{ type: 'started' }` stays a
            // discriminant — and only a top-level literal widens, which is what
            // a `const c = 'a'` element does in tsc.
            InferredExpression::Known(ty) if !is_fresh_literal_expression(&element.expression) => {
                spread_element_types.push(widen_top_level_literals(&ty));
            }
            InferredExpression::Known(ty) => element_types.push(ty),
        }
    }

    // A bare array literal widens its element literals like tsc does
    // (`["a", "b"]` -> `string[]`, not `("a" | "b")[]`), so methods such as
    // `["a","b"].includes(someString)` accept a widened argument. Contextual
    // typing against a literal-union target goes through a different path and is
    // unaffected.
    //
    // A spread contributes an *existing* type, not a fresh literal, so it is
    // not widened: `[...combination]` off a `('a' | 'b')[]` stays that union
    // where widening made it `string[]` and rejected every use of the copy.
    let element_type = if spread_element_types.is_empty() {
        widen_outside_literal_context(element_types)
    } else {
        if !element_types.is_empty() {
            spread_element_types.push(widen_outside_literal_context(element_types));
        }
        union_type(spread_element_types)
    };
    InferredExpression::Known(Type::Array(Box::new(element_type)))
}

/// tsc's `checkArrayLiteral` in a tuple context: a tuple of the elements'
/// types, each literal widened (`checkExpressionForMutableLocation`) since the
/// context a destructuring pattern gives an element is never a literal type.
fn infer_tuple_literal(
    elements: &[ParsedArrayElement],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let mut element_types = Vec::with_capacity(elements.len());
    for element in elements {
        match infer_expression(&element.expression, symbols, ctx) {
            InferredExpression::Known(ty) if ty.is_unknown() => return InferredExpression::Unknown,
            InferredExpression::Known(ty) if is_fresh_literal_expression(&element.expression) => {
                element_types.push(crate::checks::expr::widen_type(&ty));
            }
            InferredExpression::Known(ty) => element_types.push(ty),
            InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => return InferredExpression::Unknown,
        }
    }
    InferredExpression::Known(Type::Tuple(element_types))
}

fn is_fresh_literal_expression(expression: &ParsedExpression) -> bool {
    matches!(
        expression,
        ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::TemplateLiteral { .. }
            | ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
    )
}

fn widen_top_level_literals(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => {
            crate::checks::expr::widen_type(ty)
        }
        Type::Reference(reference) if reference.enum_base.is_some() => {
            crate::checks::expr::widen_type(ty)
        }
        Type::Union(union) => {
            union_type(union.types().iter().map(widen_top_level_literals).collect())
        }
        other => other.clone(),
    }
}

pub(crate) fn infer_object_property_value(
    parsed_expression: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    match infer_expression(parsed_expression, symbols, ctx) {
        InferredExpression::Known(ty) => ty,
        _ => Type::Unknown,
    }
}

/// Infers the type of an object literal property. Method shorthand is lowered to an arrow
/// function whose declared parameter and return types must be honored, so it is routed through
/// the arrow-function checking path (which also checks the body, consistent with function
/// declarations) rather than the inference path that widens inline parameters to `any`.
fn infer_object_property_type(
    property: &ParsedObjectProperty,
    member_this: Option<&LiteralMemberThis>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    if (property.is_method || property.is_accessor)
        && let ParsedExpression::ArrowFunction(arrow) = &property.value
    {
        let checked = if property.is_getter {
            getter_with_return_annotation(arrow, property)
        } else {
            arrow.as_ref().clone()
        };
        if let Some(member_this) = member_this {
            member_this.hand_to(arrow, true, ctx);
        }
        let function_type = with_type_copy_reason(TypeCopyReason::ExpressionInference, || {
            check_arrow_function_expression(checked, symbols, ctx)
        });
        ctx.next_arrow_this = None;
        // `getTypeOfAccessors`: the getter's annotation, else the setter's, else
        // what the getter's body returns — all of which the getter's own
        // return type now is. A lone setter's property is its parameter's type.
        if property.is_getter {
            if let (Some(member_this), Some(setter)) = (member_this, property.paired_setter.as_deref()) {
                member_this.hand_to(setter, true, ctx);
            }
            check_setter_of_getter(property, arrow, function_type.return_type(), symbols, ctx);
            ctx.next_arrow_this = None;
            return function_type.return_type().clone();
        }
        if property.is_accessor {
            return function_type.parameters().first().cloned().unwrap_or(Type::Any);
        }
        return Type::Function(super::with_written_predicate(function_type, arrow, ctx));
    }

    infer_object_property_value(&property.value, symbols, ctx)
}

/// tsc's `isReadonlySymbol` for a literal member: an accessor with a getter and
/// no setter of the same name.
fn is_get_only_accessor(property: &ParsedObjectProperty, properties: &[ParsedObjectProperty]) -> bool {
    property.is_getter
        && property.paired_setter.is_none()
        && !properties.iter().any(|other| {
            other.is_accessor && !other.is_getter && other.name == property.name
        })
}

/// tsc's `getReturnTypeFromAnnotation` for a get accessor: without an
/// annotation of its own it takes its setter's parameter annotation, which then
/// types its `return` statements too.
fn getter_with_return_annotation(
    getter: &surge_ts_syntax::ParsedArrowFunction,
    property: &ParsedObjectProperty,
) -> surge_ts_syntax::ParsedArrowFunction {
    let mut getter = getter.clone();
    if getter.return_type.is_none() {
        getter.return_type = property
            .paired_setter
            .as_deref()
            .and_then(|setter| setter.parameters.first())
            .and_then(|parameter| parameter.declared_type.clone());
    }
    getter
}

/// A setter's parameter written without a type is typed by the getter's
/// return type, annotated or inferred (`getTypeForVariableLikeDeclaration`:
/// "use the type of the get accessor if one is present").
fn check_setter_of_getter(
    property: &ParsedObjectProperty,
    getter: &surge_ts_syntax::ParsedArrowFunction,
    getter_return_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let Some(setter) = property.paired_setter.as_deref() else {
        return;
    };
    let untyped_value = setter
        .parameters
        .first()
        .is_some_and(|parameter| parameter.declared_type.is_none());
    if !untyped_value || getter.return_type.is_some() {
        check_paired_setter(property, getter, symbols, ctx);
        return;
    }
    let value_signature = surge_ts_types::FunctionType::new(
        vec![getter_return_type.clone()],
        Type::Void,
        false,
        1,
    );
    let _ = crate::checks::function::check_arrow_function_expression_with_expected_type(
        setter.clone(),
        Some(&value_signature),
        symbols,
        ctx,
    );
}

/// A getter's `set` partner is checked for its body alone. Its parameter,
/// written without a type, takes the getter's annotation (or `any`).
pub(crate) fn check_paired_setter(
    property: &ParsedObjectProperty,
    getter: &surge_ts_syntax::ParsedArrowFunction,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    let Some(setter) = property.paired_setter.as_deref() else {
        return;
    };
    let mut setter = setter.clone();
    if let Some(parameter) = setter.parameters.first_mut()
        && parameter.declared_type.is_none()
    {
        parameter.declared_type = Some(getter.return_type.clone().unwrap_or(surge_ts_syntax::ParsedType::Any));
    }
    let _ = check_arrow_function_expression(setter, symbols, ctx);
}

/// The template literal type a template expression has where tsc types it by
/// pattern (a const context, or a template-literal contextual type): each
/// interpolation contributes its own type when that is within
/// `string | number | boolean | bigint | null | undefined`
/// (`templateConstraintType`), and `string` otherwise.
pub(crate) fn template_expression_pattern_type(
    expressions: &[ParsedExpression],
    quasis: &[Option<String>],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if quasis.len() != expressions.len() + 1 {
        return None;
    }
    let texts: Vec<String> = quasis.iter().cloned().collect::<Option<_>>()?;
    let before = ctx.diagnostics().len();
    let types: Vec<Type> = expressions
        .iter()
        .map(|interpolation| match infer_expression(interpolation, symbols, ctx) {
            InferredExpression::Known(ty) if is_template_constraint(&ty) => ty,
            // A type variable of the body being checked is kept when it is a
            // `templateConstraintType` (`checkTemplateExpression`).
            InferredExpression::Known(ty)
                if surge_ts_types::type_variable::is_generic_index_type(&ty)
                    && surge_ts_types::is_assignable_to(
                        &ty,
                        &surge_ts_types::union_type(vec![
                            Type::String,
                            Type::Number,
                            Type::BigInt,
                            Type::Boolean,
                            Type::Null,
                            Type::Undefined,
                        ]),
                    ) =>
            {
                ty
            }
            // A type parameter is a generic placeholder tsc relates through
            // its constraint, which surge's placeholder does not carry; like a
            // value surge could not type, it stands as `any`, the placeholder
            // that matches whatever the target asks for.
            InferredExpression::Known(ty) if !ty.is_unknown() && ty != Type::Any => Type::String,
            _ => Type::Any,
        })
        .collect();
    ctx.truncate_diagnostics_releasing_utility_keys(before);
    Some(surge_ts_types::template_literal_type(&texts, &types))
}

fn is_template_constraint(ty: &Type) -> bool {
    let primitive = |ty: &Type| {
        matches!(
            ty,
            Type::String
                | Type::Number
                | Type::Boolean
                | Type::BigInt
                | Type::Null
                | Type::Undefined
                | Type::StringLiteral(_)
                | Type::NumberLiteral(_)
                | Type::BooleanLiteral(_)
        ) || surge_ts_types::is_template_literal_type(ty)
    };
    match ty {
        Type::Union(union) => union.types().iter().all(primitive),
        other => primitive(other),
    }
}

/// Which literal kinds an array literal's elements keep, per tsc's
/// `isLiteralOfContextualType`: an element whose contextual type is a type
/// variable constrained to `string` stays a string literal, and likewise for
/// `number`. Set by generic inference around the one argument it applies to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LiteralElementContext {
    pub(crate) string: bool,
    pub(crate) number: bool,
}

thread_local! {
    static LITERAL_ELEMENT_CONTEXT: std::cell::Cell<LiteralElementContext> =
        const { std::cell::Cell::new(LiteralElementContext { string: false, number: false }) };
}

pub(crate) fn with_literal_element_context<R>(
    context: LiteralElementContext,
    run: impl FnOnce() -> R,
) -> R {
    let previous = LITERAL_ELEMENT_CONTEXT.with(|cell| cell.replace(context));
    let result = run();
    LITERAL_ELEMENT_CONTEXT.with(|cell| cell.set(previous));
    result
}

fn widen_outside_literal_context(element_types: Vec<Type>) -> Type {
    let context = LITERAL_ELEMENT_CONTEXT.with(std::cell::Cell::get);
    if context == LiteralElementContext::default() {
        return crate::checks::expr::widen_type(&union_type(element_types));
    }
    let members: Vec<Type> = element_types
        .into_iter()
        .flat_map(|ty| match ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other],
        })
        .map(|member| match &member {
            Type::StringLiteral(_) if context.string => member,
            Type::NumberLiteral(_) if context.number => member,
            _ => crate::checks::expr::widen_type(&member),
        })
        .collect();
    union_type(members)
}
