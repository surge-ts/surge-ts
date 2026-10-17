//! Core ParsedType -> Type resolution (tuples, functions, objects, unions, named, mapped).

mod conditional;
mod indexed_access;
mod intersection;
pub(crate) mod mapped;
mod named;
mod structural;
mod substitution;
mod template;

pub(crate) use conditional::*;
pub(crate) use intersection::*;
pub(crate) use mapped::*;
pub(crate) use named::*;
pub(crate) use structural::*;
pub(crate) use substitution::*;
pub(crate) use template::*;

use super::*;

pub(crate) use indexed_access::check_binding_pattern_reads;
use indexed_access::resolve_indexed_access_type;

use std::sync::Arc;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::ParsedType;

use surge_ts_types::{NumberLiteralType, Type, TypeCopyReason, union_type, with_type_copy_reason};

use crate::context::{CheckerContext, DeclarationResolutionKey, convert_span};

thread_local! {
    static TYPE_EXPANSION_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static TYPE_EXPANSION_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Per-root ceiling on distributive-conditional members and mapped-type keys
/// processed, the checker's analogue of tsc's TS2589 instantiation limit.
/// Nested distribution multiplies (|A|·|B|·… per nesting level) and inline
/// branch ASTs never re-enter a named-declaration frame, so the `resolving`
/// cycle stack cannot see the blowup; this budget is what bounds it. Well-formed
/// code stays orders of magnitude below the limit (a 10k-key mapped type
/// consumes 10k steps).
const TYPE_EXPANSION_STEP_LIMIT: u64 = 500_000;

/// Marks a conditional/mapped expansion on the stack. The step counter resets
/// when the outermost scope begins, so the budget covers one entire (possibly
/// multiplicative) expansion tree while independent sibling expansions each get
/// a fresh budget.
pub(crate) struct TypeExpansionScope;

impl TypeExpansionScope {
    pub(crate) fn enter() -> Self {
        TYPE_EXPANSION_DEPTH.with(|depth| {
            if depth.get() == 0 {
                TYPE_EXPANSION_STEPS.with(|steps| steps.set(0));
            }
            depth.set(depth.get() + 1);
        });
        Self
    }
}

impl Drop for TypeExpansionScope {
    fn drop(&mut self) {
        TYPE_EXPANSION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Returns false once the current expansion tree has exhausted its budget; the
/// caller degrades to `Type::Unknown` (the checker's "cannot model" sentinel,
/// which assignability treats leniently) instead of expanding further.
pub(crate) fn try_consume_type_expansion_step() -> bool {
    TYPE_EXPANSION_STEPS.with(|steps| {
        let next = steps.get().saturating_add(1);
        steps.set(next);
        next <= TYPE_EXPANSION_STEP_LIMIT
    })
}

thread_local! {
    /// `typeof import("spec")` queries on the current resolution stack. A
    /// module whose namespace member is itself annotated through the module
    /// (`export declare const x: typeof import("./self").y`) re-enters the
    /// query while its namespace is being read; the re-entry answers the
    /// sentinel instead of recursing.
    static IMPORT_TYPE_QUERIES_IN_PROGRESS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn resolve_import_type_query(
    specifier: &str,
    members: &[String],
    member_spans: &[surge_ts_syntax::TextSpan],
    ctx: &mut CheckerContext,
) -> ResolvedType {
    let unknown = ResolvedType {
        ty: Type::Unknown,
        had_error: false,
    };
    let file_name = ctx.file_name.clone();
    let key = (file_name.clone(), specifier.to_string());
    let re_entered = IMPORT_TYPE_QUERIES_IN_PROGRESS.with(|stack| {
        let mut stack = stack.borrow_mut();
        if stack.iter().any(|entry| *entry == key) || stack.len() > 16 {
            return true;
        }
        stack.push(key.clone());
        false
    });
    if re_entered {
        return unknown;
    }
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            IMPORT_TYPE_QUERIES_IN_PROGRESS.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
    let _pop = Pop;
    let canonical = ctx.canonical_file_name_arc();
    let found = ctx
        .import_type_namespace(&file_name, specifier)
        .or_else(|| ctx.import_type_namespace(&canonical, specifier));
    let Some(mut ty) = found else {
        return unknown;
    };
    let module_path = match &ty {
        Type::Object(namespace) if !namespace.synthetic_open_index => namespace
            .alias_name
            .as_deref()
            .and_then(|alias| alias.strip_prefix("typeof import(\""))
            .and_then(|rest| rest.strip_suffix("\")"))
            .map(str::to_string),
        _ => None,
    };
    // An `export =` module's symbol is the assigned entity, and its first
    // qualifier is a property of that entity's type (`getTypeFromImportTypeNode`,
    // checker.go:24517).
    let assigned_entity = module_path.is_none()
        && matches!(ty.peeled(), Type::Object(object) if !object.synthetic_open_index)
        && ctx.import_type_namespaces.lock().ok().is_some_and(|namespaces| {
            [Arc::from(file_name.as_str()), canonical.clone()]
                .into_iter()
                .any(|file| {
                    namespaces
                        .get(&(file, specifier.to_string()))
                        .is_some_and(|target| target.export_assignment)
                })
        });
    for (index, member) in members.iter().enumerate() {
        match ty.get_property_access_type(member) {
            Some(member_ty) => ty = member_ty,
            None => {
                // The module namespace object lists every value export, and a
                // namespace the module declares lists its own, so a qualifier
                // either lacks is tsc's TS2694 on it.
                let reported_namespace = module_path
                    .as_deref()
                    .and_then(|path| {
                        if index == 0 {
                            return Some(format!("\"{path}\""));
                        }
                        let inner = members[..index].join(".");
                        ctx.namespace_registry
                            .info_in_module(path, &inner)
                            .filter(|info| info.complete)
                            .map(|_| format!("\"{path}\".{inner}"))
                    })
                    .or_else(|| {
                        (index == 0 && assigned_entity).then(|| {
                            let module = ctx.import_type_module_path(&file_name, specifier);
                            format!("\"{}\".export=", module.as_deref().unwrap_or(specifier))
                        })
                    });
                if let Some(namespace_name) = reported_namespace {
                    let mut diagnostic = Diagnostic::ts2694(
                        namespace_name,
                        member,
                        ctx.file_name.clone(),
                    );
                    if let Some(span) = member_spans.get(index) {
                        diagnostic = diagnostic.with_span(convert_span(*span));
                    }
                    ctx.push_utility_diagnostic_once(diagnostic);
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: true,
                    };
                }
                return unknown;
            }
        }
    }
    ResolvedType {
        ty,
        had_error: false,
    }
}

pub(crate) fn resolve_parsed_type(
    parsed_type: ParsedType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    match parsed_type {
        ParsedType::String => ResolvedType {
            ty: Type::String,
            had_error: false,
        },
        ParsedType::Number => ResolvedType {
            ty: Type::Number,
            had_error: false,
        },
        ParsedType::Boolean => ResolvedType {
            ty: Type::Boolean,
            had_error: false,
        },
        ParsedType::BigInt => ResolvedType {
            ty: Type::BigInt,
            had_error: false,
        },
        ParsedType::Symbol => ResolvedType {
            ty: Type::Symbol,
            had_error: false,
        },
        ParsedType::Undefined => ResolvedType {
            ty: Type::Undefined,
            had_error: false,
        },
        ParsedType::Null => ResolvedType {
            ty: Type::Null,
            had_error: false,
        },
        ParsedType::Any => ResolvedType {
            ty: Type::Any,
            had_error: false,
        },
        ParsedType::Unknown => ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        },
        ParsedType::ErrorType => ResolvedType {
            ty: Type::ErrorType,
            had_error: false,
        },
        ParsedType::UnknownKeyword => ResolvedType {
            ty: Type::GenuineUnknown,
            had_error: false,
        },
        ParsedType::Never => ResolvedType {
            ty: Type::Never,
            had_error: false,
        },
        ParsedType::StringLiteral(value) => ResolvedType {
            ty: Type::StringLiteral(value),
            had_error: false,
        },
        ParsedType::NumberLiteral(value) => ResolvedType {
            ty: Type::NumberLiteral(NumberLiteralType { value }),
            had_error: false,
        },
        ParsedType::BooleanLiteral(value) => ResolvedType {
            ty: Type::BooleanLiteral(value),
            had_error: false,
        },
        ParsedType::Void => ResolvedType {
            ty: Type::Void,
            had_error: false,
        },
        ParsedType::Object(object_type) => {
            resolve_object_type(object_type.as_ref(), ctx, resolving, substitution)
        }
        ParsedType::Array(element_type) => {
            let resolved_element = structural::resolve_in_deferred_position(ctx, resolving, |ctx, resolving| {
                resolve_parsed_type(std::sync::Arc::unwrap_or_clone(element_type), ctx, resolving, substitution)
            });
            ResolvedType {
                ty: Type::Array(Box::new(resolved_element.ty)),
                had_error: resolved_element.had_error,
            }
        }
        ParsedType::Tuple(elements) => resolve_tuple_type(
            std::sync::Arc::unwrap_or_clone(elements),
            ctx,
            resolving,
            substitution,
        ),
        ParsedType::VariadicTuple(elements) => resolve_variadic_tuple_type(
            std::sync::Arc::unwrap_or_clone(elements),
            ctx,
            resolving,
            substitution,
        ),
        ParsedType::Readonly(inner) => {
            let inner = resolve_parsed_type(
                std::sync::Arc::unwrap_or_clone(inner),
                ctx,
                resolving,
                substitution,
            );
            ResolvedType {
                ty: readonly_reference(inner.ty),
                had_error: inner.had_error,
            }
        }
        ParsedType::Union(types) => resolve_union_type(
            std::sync::Arc::unwrap_or_clone(types),
            ctx,
            resolving,
            substitution,
        ),
        ParsedType::Intersection(types) => resolve_intersection_type(
            std::sync::Arc::unwrap_or_clone(types),
            ctx,
            resolving,
            substitution,
        ),
        ParsedType::Function(function_type) => {
            resolve_function_type(function_type, ctx, resolving, substitution)
        }
        ParsedType::Named(named_type) => {
            resolve_named_type(named_type, ctx, resolving, substitution)
        }
        ParsedType::TypeOf(type_of) => {
            if let Some(specifier) = &type_of.import_specifier {
                return resolve_import_type_query(
                    specifier,
                    &type_of.members,
                    &type_of.member_spans,
                    ctx,
                );
            }
            // The name resolver refuses a property's read of a constructor
            // local before looking any further out (TS2844 in a type position).
            if let Some(span) = type_of.name_span
                && let Some(property) =
                    crate::program::property_with_invalid_initializer(&type_of.name, span, ctx)
            {
                let diagnostic =
                    property.invalid_reference_diagnostic(&type_of.name, span, ctx.file_name.clone());
                ctx.push_utility_diagnostic_once(diagnostic.with_span(convert_span(span)));
                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: true,
                };
            }
            // A type query reads the value, so a UMD-global name reports here the
            // same way it would in an expression.
            let umd_global = crate::checks::emit_umd_global_reference_diagnostic(
                &type_of.name,
                type_of.name_span,
                ctx,
            );

            // Inside another file's declaration the name is that module's
            // lexically: its own value wins over anything the consumer binds
            // under the same name (`interface Thing { n: typeof x }` read from
            // a file with its own `x`).
            let declaring_module_symbol = (ctx.cross_file_resolution_depth > 0)
                .then(|| {
                    let file_name = ctx.file_name.clone();
                    ctx.module_local_values_for_file(&file_name)
                        .and_then(|table| table.get(&type_of.name).cloned())
                })
                .flatten();
            // `typeof X` references a value. During type-declaration resolution the
            // file's imported value bindings may not yet be in `ctx.symbols`, so on
            // a miss consult the module's full value table (the same forward-ref
            // fallback used when checking expressions); genuinely-missing names
            // still report TS2304.
            // The file's own declaration shadows a global of the same name
            // (`resolveName` meets module locals first), including while the
            // query is resolved before the declaration is bound in `ctx.symbols`
            // and the global is all `ctx.symbols` has.
            let module_own = || {
                ctx.module_value_fallback
                    .as_ref()
                    .and_then(|table| table.get(&type_of.name).cloned())
                    .or_else(|| {
                        ctx.module_local_values_for_file(&ctx.file_name)
                            .and_then(|table| table.get(&type_of.name).cloned())
                    })
            };
            let visible = ctx.symbols.get(&type_of.name).cloned().and_then(|symbol| {
                let is_global = ctx
                    .ambient_global_symbols
                    .get(&type_of.name)
                    .is_some_and(|global| global.ty == symbol.ty);
                match is_global.then(module_own).flatten() {
                    Some(own) => Some(own),
                    None => Some(symbol),
                }
            });
            // A parameter of the signature being mapped is the innermost
            // scope, and shadows a module value of the same name.
            let parameter = ctx
                .signature_parameter_bindings
                .iter()
                .rev()
                .find(|(name, _)| name == &type_of.name)
                .map(|(_, ty)| crate::symbols::SymbolInfo {
                    ty: ty.clone(),
                    kind: crate::symbols::SymbolKind::Parameter,
                    function_signature: None,
                });
            let symbol = declaring_module_symbol
                .or(parameter)
                .or(visible)
                // `resolveName` meets the module's own declarations before the
                // globals: `typeof stop` for a module's `const stop` is that
                // const, not lib.dom's global `stop()`, even when the query is
                // resolved before the const is bound in `ctx.symbols`.
                .or_else(|| {
                    ctx.module_value_fallback
                        .as_ref()
                        .and_then(|table| table.get(&type_of.name).cloned())
                })
                .or_else(|| {
                    // `typeof X` inside an imported declaration's body is resolved
                    // under the declaring file's name (set by `with_file_name`), but
                    // the consumer's value `symbols`/`module_value_fallback` do not
                    // hold that module's locals. Consult the declaring module's own
                    // value table so a cross-module `Alias<typeof localConst>`
                    // resolves instead of falsely reporting TS2304.
                    let file_name = ctx.file_name.clone();
                    ctx.module_local_values_for_file(&file_name)
                        .and_then(|table| table.get(&type_of.name).cloned())
                })
                .or_else(|| ctx.ambient_global_symbols.get(&type_of.name).cloned())
                // A context recovered from an environment captured during module
                // analysis holds the global table as it stood then, before
                // `globalThis` was built from it; the check phase reads the
                // program's final one.
                .or_else(|| {
                    crate::program::in_check_phase()
                        .then(crate::program::program_ambient_globals)
                        .flatten()
                        .and_then(|globals| globals.get(&type_of.name).cloned())
                });

            // `undefined` is a global value like any other to a type query:
            // `var x: typeof undefined`. Its type is the widening `undefined`,
            // which is `any` once `strictNullChecks` is off.
            if symbol.is_none() && type_of.name == "undefined" && type_of.members.is_empty() {
                return ResolvedType {
                    ty: if surge_ts_types::strict_null_checks() {
                        Type::Undefined
                    } else {
                        Type::Any
                    },
                    had_error: false,
                };
            }

            let Some(symbol) = symbol else {
                // `globalThis` is always a valid built-in, but its value symbol is
                // installed only after every ambient global is collected, so an
                // ambient declaration naming it (`declare var window: Window & typeof
                // globalThis`) resolves it first. Treat the miss as a clean `unknown`
                // (a false TS2304 / `had_error` would otherwise poison the enclosing
                // intersection); the `T & unknown ⇒ T` simplification then keeps
                // `window`/`self` as `Window`.
                // A UMD global resolves for tsc even when surge has no value
                // symbol for it: the reference already reported as TS2686, so a
                // TS2304 on top would be a second diagnostic tsc never emits.
                if type_of.name == "globalThis" || umd_global {
                    // For `globalThis` the miss is a timing gap — the global
                    // object is installed after every ambient global is
                    // collected — so an answer decided now is degraded, not
                    // clean: a clean one would be interned for the whole run.
                    // That covers a member (`typeof globalThis.fetch`) and the
                    // object a conditional or an indexed access reads
                    // (`typeof globalThis extends { onmessage: any } ? … : …`,
                    // bun's `(typeof globalThis)["WritableStream"]`).
                    // `resolve_intersection_type` drops the plain query itself.
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: !type_of.members.is_empty() || type_of.name == "globalThis",
                    };
                }
                // A class reached through `import type { Class }`, or named
                // inside the ambient module that declares it (`http`'s
                // `RequestListener<Request extends typeof IncomingMessage = …>`),
                // has a type declaration but no value symbol in reach; tsc still
                // resolves `typeof Class` there. Stand in a constructor surface
                // over the instance type — enough for `InstanceType<typeof C>`
                // and for a constructor-shaped constraint — kept open so a static
                // member surge cannot see is never reported. Anything else
                // degrades instead of reporting a name the file demonstrably
                // declares.
                if let Some(handle) = ctx.lookup_type_declaration_handle(&type_of.name) {
                    if type_of.members.is_empty()
                        && matches!(
                            handle.get(),
                            crate::symbols::TypeDeclarationInfo::Interface(_)
                        )
                    {
                        // The query names the value, so what reading the class
                        // as a type would report (a generic one's missing
                        // arguments) is not the source's.
                        let checkpoint = ctx.diagnostics().len();
                        let instance = resolve_named_type(
                            std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
                                type_argument_spans: Vec::new(),
                                name: type_of.name.clone(),
                                span: type_of.name_span,
                                type_arguments: Vec::new(),
                            }),
                            ctx,
                            resolving,
                            substitution,
                        );
                        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                        // A tainted expansion is left degraded: standing a
                        // constructor over a shape whose base failed to resolve
                        // was measured to make `Mock<RequestListener>` uncallable
                        // again, because the eager class body it embeds carries
                        // the sentinel into every declared function parameter.
                        if !instance.had_error && !instance.ty.is_unknown() {
                            let mut properties = surge_ts_types::PropertyMap::default();
                            properties.insert(
                                "prototype".into(),
                                surge_ts_types::ObjectProperty::required(instance.ty.clone()),
                            );
                            let constructor = surge_ts_types::FunctionType::new(
                                vec![Type::Any],
                                instance.ty,
                                true,
                                0,
                            );
                            return ResolvedType {
                                ty: Type::Object(
                                    surge_ts_types::ObjectType::new(properties, None)
                                        .with_open_index_marker()
                                        .with_construct_signature(constructor),
                                ),
                                had_error: false,
                            };
                        }
                    }
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: true,
                    };
                }
                // `typeof this.x` where surge binds no `this` (a class member's
                // annotation): tsc reads the instance; surge leaves it open.
                if type_of.name == "this" {
                    // A script's top level owns `this`, which is `typeof
                    // globalThis` (`tryGetThisTypeAt`), and a member it lacks
                    // is read through the index signature it does not have.
                    if let Some(span) = type_of.name_span
                        && u32::try_from(span.start).is_ok_and(|start| ctx.global_this_starts.contains(&start))
                        && let Some(member) = type_of.members.first()
                        && let Some(global) = ctx.symbols.get("globalThis").map(|symbol| symbol.ty.clone())
                        && global.get_property_access_type(member).is_none()
                        && let Some(diagnostic) = crate::checks::expr::global_this_missing_member(member, &global, ctx)
                    {
                        if let Some(diagnostic) = diagnostic {
                            ctx.push(crate::spans::diagnostic_with_syntax_span(
                                diagnostic,
                                type_of.member_spans.first().copied(),
                            ));
                        }
                        return ResolvedType {
                            ty: Type::Any,
                            had_error: false,
                        };
                    }
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: false,
                    };
                }
                let with_name_span = |diagnostic: Diagnostic| match type_of.name_span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                };
                // As for an unknown type name, the plain form stands in where
                // the report is never seen: in a file whose diagnostics are
                // suppressed, or in a lazy peel or probe dropped with its
                // diagnostics.
                // Declaration files re-resolve the same failing query many times.
                let plain = with_name_span(Diagnostic::ts2304(&type_of.name, ctx.file_name.clone()));
                let diagnostic = if !ctx.reports_diagnostic(&plain)
                    || crate::infer::types::cache::lazy_peel_depth() > 0
                    || crate::infer::types::diagnostics_discarded()
                {
                    plain
                } else {
                    with_name_span(crate::checks::expr::unresolved_type_query_diagnostic(
                        &type_of.name,
                        type_of.name_span,
                        ctx,
                    ))
                };
                ctx.push(diagnostic);
                // tsc checks a query's type arguments before its expression
                // (`checkExpressionWithTypeArguments`), so what they name is
                // reported whether or not the value resolves.
                for argument in &type_of.type_arguments {
                    let _ = resolve_parsed_type(argument.clone(), ctx, resolving, substitution);
                }

                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: true,
                };
            };

            // `typeof NS.Root` walks the dotted member path off the base symbol's
            // type. Any segment we cannot model (a non-object base, or a missing
            // property on a namespace whose shape we don't fully reconstruct)
            // degrades to `Unknown` silently rather than emitting a false
            // positive, since the base name itself was resolved.
            let mut ty = symbol.ty;
            // `typeof fn` over a generic declaration keeps the declaration on
            // the function handle: a property typed by it (`vi.fn`) is called
            // through the property path, which otherwise has only the bare
            // resolved signature and cannot bind the type parameters.
            if type_of.members.is_empty()
                && let Some(signature) = symbol.function_signature.as_ref()
                && !signature.type_parameters.is_empty()
                && !signature.overloaded
                && let Type::Function(function) = ty
            {
                let concrete: Arc<crate::symbols::FunctionSignatureInfo> = Arc::clone(signature);
                let declaration: Arc<dyn std::any::Any + Send + Sync> = concrete;
                ty = Type::Function(function.with_declaration(declaration));
            }
            // A base still standing at the degradation sentinel is a value whose
            // type is not known *yet* (a forward reference resolved during an
            // earlier pass), not a value without the member. Reporting a clean
            // `unknown` would let the enclosing declaration intern that answer and
            // pin every later consumer to it — the shape behind a discriminant
            // written as `typeof Codes.invalid_string` losing its literal type.
            let binding_pass = !crate::program::in_check_phase();
            if binding_pass && matches!(ty, Type::Unknown) {
                crate::modules::note_lazy_read_before_check();
            }
            if ty.is_unknown() && !type_of.members.is_empty() {
                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: true,
                };
            }
            // A member the base does not carry yet, or carries at the sentinel, is
            // the same timing gap: a namespace import's members are typed as their
            // module's exports settle, so a clean answer here froze Radix's
            // `typeof DialogPrimitive.Root` — and every props interface built on
            // it — as an empty shape for the whole run.
            // tsc checks the query's name as an expression (`checkNonNullExpression`
            // on each qualifier), judged where the query is written.
            let report_nullable_qualifiers = surge_ts_types::strict_null_checks()
                && !binding_pass
                && ctx.cross_file_resolution_depth == 0
                && !ctx.is_library_scoped_file(&ctx.file_name)
                && substitution.iter().all(|(name, _)| substitution.is_placeholder(name));
            for (index, member) in type_of.members.iter().enumerate() {
                if report_nullable_qualifiers
                    && let Some(span) = type_of.name_span
                    && let Some(diagnostic) =
                        nullable_query_qualifier_diagnostic(&ty, (index == 0).then_some(type_of.name.as_str()), ctx)
                {
                    ctx.push(diagnostic.with_span(convert_span(span)));
                    ty = surge_ts_types::remove_nullish(&ty);
                }
                match ty.get_property_access_type(member) {
                    Some(member_ty) if !matches!(member_ty, Type::Unknown) => ty = member_ty,
                    _ => {
                        if binding_pass {
                            crate::modules::note_lazy_read_before_check();
                        }
                        return ResolvedType {
                            ty: Type::Unknown,
                            had_error: true,
                        };
                    }
                }
            }
            // A value typed from its initializer on first read is not typed at
            // all before the check phase; anything built on it then is degraded.
            if crate::modules::is_lazy_initializer(&ty) {
                if binding_pass {
                    crate::modules::note_lazy_read_before_check();
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: true,
                    };
                }
                ty = crate::checks::function::settle_lazy_read(ty);
            }

            // tsc's `checkExpressionWithTypeArguments` on the query: TS2635 when
            // no signature of the value takes that many type arguments. Judged
            // where the query is written, not where an instantiation of the
            // declaration around it re-resolves it.
            let inapplicable = (!type_of.type_arguments.is_empty()
                && !(type_of.members.is_empty()
                    && crate::checks::call::class_value_lost_its_constructor(&type_of.name, &ty, ctx)))
            .then(|| {
                crate::checks::call::instantiation_expression_diagnostic(
                    &ty,
                    type_of
                        .members
                        .is_empty()
                        .then_some(symbol.function_signature.as_deref())
                        .flatten(),
                    type_of.type_arguments.len(),
                    type_of.type_arguments_span,
                    &ctx.file_name,
                )
            })
            .flatten();
            if let Some(diagnostic) = &inapplicable
                && !binding_pass
                && ctx.cross_file_resolution_depth == 0
                && type_of.name_span.is_some()
                && !ctx.is_library_scoped_file(&ctx.file_name)
                && substitution.iter().all(|(name, _)| substitution.is_placeholder(name))
            {
                ctx.push_utility_diagnostic_once(diagnostic.clone());
            }
            // `getInstantiationExpressionType` keeps only the signatures that
            // take the arguments: with none left, the query is an object with
            // no signatures at all.
            if inapplicable.is_some() && matches!(ty, Type::Function(_)) {
                return ResolvedType {
                    ty: Type::Object(
                        crate::metrics::alloc_object_type(surge_ts_types::PropertyMap::default(), None)
                            .with_alias_name(written_type_query(&type_of)),
                    ),
                    had_error: false,
                };
            }
            if !type_of.type_arguments.is_empty()
                && !binding_pass
                && ctx.cross_file_resolution_depth == 0
                && type_of.name_span.is_some()
                && !ctx.is_library_scoped_file(&ctx.file_name)
                && substitution.iter().all(|(name, _)| substitution.is_placeholder(name))
            {
                let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
                crate::checks::call::report_unsatisfied_instantiation_arguments(
                    &ty,
                    type_of.members.is_empty().then_some(symbol.function_signature.as_ref()).flatten(),
                    &type_of.type_arguments,
                    type_of.type_arguments_span,
                    &symbols,
                    ctx,
                );
            }

            // `typeof f<A, B>` is an instantiation expression: it binds the
            // generic value's type parameters in type position, the same way a
            // call with explicit type arguments does. Without this the query
            // answered with the uninstantiated signature, whose parameters are
            // still the declaration's own — the sentinel — so every utility over
            // it (`ReturnType`, `Parameters`) degraded.
            if !type_of.type_arguments.is_empty()
                && type_of.members.is_empty()
                && let Some(signature) = symbol.function_signature.as_ref()
                && !signature.type_parameters.is_empty()
                && let Type::Function(function) = &ty
            {
                let signature = std::sync::Arc::clone(signature);
                let function = function.clone();
                // The arguments are resolved under the *ambient* substitution,
                // not a fresh one: inside `CreateNext<R, S>` the query reads
                // `typeof withTRPC<R, S>`, whose arguments are the enclosing
                // alias's own parameters and mean nothing on their own.
                let mut argument_substitution = crate::infer::TypeParameterSubstitution::new();
                for (index, type_parameter) in signature.type_parameters.iter().enumerate() {
                    let resolved = match type_of.type_arguments.get(index) {
                        Some(argument) => {
                            resolve_parsed_type(argument.clone(), ctx, resolving, substitution).ty
                        }
                        None => match type_parameter.default_type.clone() {
                            Some(default_type) => {
                                resolve_parsed_type(default_type, ctx, resolving, substitution).ty
                            }
                            None => Type::Unknown,
                        },
                    };
                    argument_substitution.insert(type_parameter.name.clone(), resolved);
                }
                let instantiated =
                    crate::checks::call::instantiate_function_type_with_substitution(
                        &function,
                        &signature,
                        &argument_substitution,
                        ctx,
                    )
                    .into_owned();
                ty = Type::Function(instantiated);
            }

            ResolvedType {
                ty,
                had_error: false,
            }
        }
        ParsedType::KeyOf(inner) => {
            // A written `any` operand is the real `any`, not a modelling
            // placeholder: `getIndexType` answers `keyofConstraintType`.
            if matches!(inner.as_ref(), ParsedType::Any) {
                return ResolvedType {
                    ty: surge_ts_types::union_type(vec![Type::String, Type::Number, Type::Symbol]),
                    had_error: false,
                };
            }
            let resolved_inner = resolve_parsed_type(
                std::sync::Arc::unwrap_or_clone(inner),
                ctx,
                resolving,
                substitution,
            );
            // `keyof T` over a type variable stays deferred (`shouldDeferIndexType`).
            if let Some(keys) = surge_ts_types::type_variable::keyof_variable(&resolved_inner.ty) {
                return ResolvedType {
                    ty: keys,
                    had_error: resolved_inner.had_error,
                };
            }
            // `getIndexType` over a union of variables is the intersection of
            // their keys, and over an intersection of them the union.
            let variable_operands = match &resolved_inner.ty {
                Type::Union(union) => Some((true, union.types())),
                intersection @ Type::Object(object)
                    if surge_ts_types::type_variable::is_narrowed_type_variable(intersection) =>
                {
                    object.intersection_operands.as_deref().map(|operands| (false, operands))
                }
                _ => None,
            };
            // A primitive operand keys its apparent type (`keyof (T & string)`
            // in the true branch of `T extends string ? keyof T : …`).
            if let Some((is_union, operands)) = variable_operands
                && operands
                    .iter()
                    .any(|operand| surge_ts_types::type_variable::keyof_variable(operand).is_some())
                && let Some(keys) = operands
                    .iter()
                    .map(|operand| {
                        surge_ts_types::type_variable::keyof_variable(operand)
                            .or_else(|| primitive_key_type(operand, ctx))
                    })
                    .collect::<Option<Vec<Type>>>()
            {
                return ResolvedType {
                    ty: if is_union {
                        intersection::merge_intersection_members(keys)
                    } else {
                        surge_ts_types::union_type(keys)
                    },
                    had_error: resolved_inner.had_error,
                };
            }
            // `keyof unknown` is `never` (tsc's `getIndexType` over the unknown
            // type), and the *written* keyword is the only `unknown` that means
            // it — surge's `Type::Unknown` doubles as the degradation sentinel.
            // `null`, `undefined` and `void` have no properties to key.
            if matches!(
                resolved_inner.ty,
                Type::GenuineUnknown | Type::Null | Type::Undefined | Type::Void
            ) {
                return ResolvedType {
                    ty: Type::Never,
                    had_error: resolved_inner.had_error,
                };
            }
            // `keyof any` is `string | number | symbol` (the same `getIndexType`
            // arm), and tsc's error type *is* `any` — so a name or module the
            // source does not resolve keys like one. Applying this to every
            // `Type::Any` is what could not be done before: surge's `Any` is
            // also its permissive modelling placeholder, and keying that turned
            // 45 silent degradations into wrong decisions. `ErrorType` carries
            // the provenance, so only the honest error type takes the rule.
            // `keyof never` takes that arm too.
            if matches!(resolved_inner.ty, Type::ErrorType)
                || (matches!(resolved_inner.ty, Type::Never) && !resolved_inner.had_error)
            {
                return ResolvedType {
                    ty: surge_ts_types::union_type(vec![
                        Type::String,
                        Type::Number,
                        Type::Symbol,
                    ]),
                    had_error: resolved_inner.had_error,
                };
            }
            if let Some(keys) = array_key_type(&resolved_inner.ty, ctx)
                .or_else(|| primitive_key_type(&resolved_inner.ty, ctx))
            {
                return ResolvedType {
                    ty: keys,
                    had_error: resolved_inner.had_error,
                };
            }
            let mut keys = Vec::new();
            // `keyof {}` is `never`, not "could not model": the empty-interface
            // escape hatch (`T[keyof DO_NOT_USE_…]` in React's `Key` and
            // `ReactNode`) relies on the `never` arm vanishing from its union.
            // Only a cleanly-resolved object with no index signature is genuinely
            // keyless — a degraded resolution lands on a property-less object too,
            // and calling that `never` would turn an unmodelled type into a closed
            // empty shape (`Omit<Unmodelled, K>` ⇒ `{}`).
            let empty_object_is_never = !resolved_inner.had_error
                && matches!(
                    resolved_inner.ty.peeled(),
                    Type::Object(ref object_type)
                        if object_type.properties.is_empty()
                            && object_type.string_index_type.is_none()
                );
            // Peel a nominal reference (`keyof User`) to read the named type's keys.
            match &resolved_inner.ty.peeled() {
                Type::Object(object_type) => {
                    // A computed-key member other than a well-known symbol's
                    // is kept under a `[…]` name so the shape is not vacuous,
                    // but its key is a symbol no written key can name: `keyof`
                    // must not enumerate it, or a mapped type and `T[keyof T]`
                    // read a property nothing reaches.
                    keys.extend(
                        object_type
                            .properties
                            .iter()
                            .filter(|(key, property)| is_public_key(key, property))
                            .filter_map(|(key, _)| property_name_key_type(key)),
                    );
                    // `getIndexType`: a string index signature keys every
                    // string and number, a number one every number. An open
                    // shape surge could not enumerate keys nothing it knows.
                    if !object_type.synthetic_open_index {
                        if object_type.string_index_type.is_some() {
                            keys.extend([Type::String, Type::Number]);
                        } else if object_type.number_index_type.is_some() {
                            keys.push(Type::Number);
                        }
                    }
                }
                Type::Union(union) if !resolved_inner.had_error => {
                    if let Some(ty) = union_key_type(union.types()) {
                        return ResolvedType { ty, had_error: false };
                    }
                    return ResolvedType {
                        ty: Type::Unknown,
                        had_error: false,
                    };
                }
                _ => {
                    return ResolvedType {
                        ty: Type::Unknown,
                        // A degraded operand yields a degraded key set, not a
                        // clean one. Reporting it clean is what lets an
                        // intersection simplify `keyof A & keyof <degraded>`
                        // down to `keyof A` and a conditional then answer
                        // `extends never` from a key set that never existed —
                        // the `ProtectedIntersection` collision branch fires on
                        // a router surge could not model.
                        had_error: resolved_inner.had_error,
                    };
                }
            }

            ResolvedType {
                ty: if keys.is_empty() {
                    if empty_object_is_never {
                        Type::Never
                    } else {
                        Type::Unknown
                    }
                } else if keys.len() == 1 {
                    keys.into_iter().next().unwrap()
                } else {
                    // tsc keeps the key union's origin, `keyof Shape`, for display.
                    match (union_type(keys), &resolved_inner.ty) {
                        (Type::Union(union), Type::Reference(_)) if !resolved_inner.had_error => {
                            Type::Union(union.with_alias_name(format!("keyof {}", resolved_inner.ty.name())))
                        }
                        (keys, _) => keys,
                    }
                },
                had_error: resolved_inner.had_error,
            }
        }
        ParsedType::Mapped(mapped) => {
            ctx.mark_check_uncertain();
            resolve_mapped_type(std::sync::Arc::unwrap_or_clone(mapped), ctx, resolving, substitution)
        }
        ParsedType::IndexedAccess(indexed_access) => {
            ctx.mark_check_uncertain();
            resolve_indexed_access_type(std::sync::Arc::unwrap_or_clone(indexed_access), ctx, resolving, substitution)
        }
        ParsedType::Conditional(conditional) => {
            ctx.mark_check_uncertain();
            resolve_conditional_type(std::sync::Arc::unwrap_or_clone(conditional), ctx, resolving, substitution)
        }
        ParsedType::TemplateLiteral(template) => resolve_template_literal_type(
            std::sync::Arc::unwrap_or_clone(template),
            ctx,
            resolving,
            substitution,
        ),
        // An `infer X` capture resolves to a permissive `any`: with no real
        // inference, the enclosing `extends` pattern (e.g. `Ctor<infer P>`) stays a
        // concrete shape so a non-matching check type correctly falls through to
        // the conditional's false branch, rather than collapsing to `unknown`
        // (which `is_assignable_to` would treat as matching). See
        // `resolve_conditional_type`.
        ParsedType::Infer(_) => ResolvedType {
            ty: Type::Any,
            had_error: false,
        },
        // A predicate annotation types the function's return value: `boolean`
        // for `x is T`, `void` for an assertion signature. The predicate payload
        // itself is consumed by guard narrowing, not by type resolution.
        ParsedType::UniqueSymbol(name) => ResolvedType {
            ty: surge_ts_types::unique_symbol_type(&ctx.file_name, &name),
            had_error: false,
        },
        ParsedType::InferredMember(member) => ResolvedType {
            ty: crate::checks::function::inferred_member_reference(member, substitution, ctx),
            had_error: false,
        },
        ParsedType::Predicate(predicate) => ResolvedType {
            ty: if predicate.asserts {
                Type::Void
            } else {
                Type::Boolean
            },
            had_error: false,
        },
    }
}

/// tsc's `reportObjectPossiblyNullOrUndefinedError` for a type query's
/// qualifier: the leading name is spelled out, a deeper qualifier is not (a
/// qualified name is no entity name expression).
fn nullable_query_qualifier_diagnostic(ty: &Type, name: Option<&str>, ctx: &CheckerContext) -> Option<Diagnostic> {
    let (undefined, null) = match ty {
        Type::Union(union) => union.types().iter().fold((false, false), |(undefined, null), member| {
            (undefined || matches!(member, Type::Undefined), null || matches!(member, Type::Null))
        }),
        Type::Undefined => (true, false),
        Type::Null => (false, true),
        _ => (false, false),
    };
    let file_name = ctx.file_name.clone();
    Some(match (name, undefined, null) {
        (_, false, false) => return None,
        (Some(name), true, true) => Diagnostic::ts18049(name, file_name),
        (Some(name), true, false) => Diagnostic::ts18048(name, file_name),
        (Some(name), false, true) => Diagnostic::ts18047(name, file_name),
        (None, true, true) => Diagnostic::ts2533(file_name),
        (None, true, false) => Diagnostic::ts2532(file_name),
        (None, false, true) => Diagnostic::ts2531(file_name),
    })
}

pub(crate) fn resolve_parsed_type_with_substitution(
    parsed_type: ParsedType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    with_type_copy_reason(TypeCopyReason::SubstitutionChanged, || {
        resolve_parsed_type(parsed_type, ctx, resolving, substitution)
    })
}

/// tsc's `getLiteralTypeFromProperty` without `includeNonPublic`: a private
/// or protected member, or a `#private` one, is no key of its type — not for
/// `keyof`, and so not for any homomorphic mapped type over it.
pub(crate) fn is_public_key(name: &str, property: &surge_ts_types::ObjectProperty) -> bool {
    property.restriction.is_none() && !name.starts_with('#')
}

/// The key type of a member name for `getLiteralTypeFromProperty`: a string
/// key's literal, or the unique symbol of a well-known `[Symbol.<name>]`
/// member. Any other computed name is a symbol surge cannot spell, and keys
/// nothing.
pub(crate) fn property_name_key_type(name: &str) -> Option<Type> {
    if !name.starts_with('[') {
        return Some(Type::StringLiteral(name.to_string()));
    }
    let symbol = name.strip_prefix('[')?.strip_suffix(']')?;
    symbol
        .starts_with("Symbol.")
        .then(|| surge_ts_types::unique_symbol_type("", symbol))
}

/// `getIndexType` over an array or tuple: `number` for its numeric index, the
/// numeric-string name of each fixed tuple position, and the members it
/// inherits from the lib `Array` (`ReadonlyArray` when readonly). `None`
/// without a lib interface to enumerate.
fn array_key_type(ty: &Type, ctx: &CheckerContext) -> Option<Type> {
    let readonly = matches!(ty, Type::Reference(reference) if reference.is_readonly_array());
    let positions = match ty.peeled() {
        Type::Array(_) => 0,
        Type::Tuple(elements) => elements.len(),
        Type::OpenTuple(tuple) => tuple.leading.len(),
        _ => return None,
    };
    let interface = if readonly { "ReadonlyArray" } else { "Array" };
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) =
        ctx.lookup_type_declaration(interface)
    else {
        return None;
    };
    if info.body.members.is_empty() {
        return None;
    }
    let mut keys = vec![Type::Number];
    keys.extend((0..positions).map(|position| Type::StringLiteral(position.to_string())));
    let mut seen = std::collections::HashSet::new();
    for member in &info.body.members {
        if seen.insert(member.name.as_str())
            && let Some(key) = property_name_key_type(&member.name)
        {
            keys.push(key);
        }
    }
    Some(union_type(keys))
}

/// `getIndexType` over a primitive: the keys of its apparent type, the lib's
/// global interface for it (`keyof string` is `number | "charAt" | …`).
fn primitive_key_type(ty: &Type, ctx: &CheckerContext) -> Option<Type> {
    let interface = match ty {
        Type::String | Type::StringLiteral(_) => "globalThis.String",
        Type::Number | Type::NumberLiteral(_) => "globalThis.Number",
        Type::Boolean | Type::BooleanLiteral(_) => "globalThis.Boolean",
        Type::BigInt => "globalThis.BigInt",
        Type::Symbol => "globalThis.Symbol",
        _ => return None,
    };
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(interface) else {
        return None;
    };
    if info.body.members.is_empty() {
        return None;
    }
    let mut keys = Vec::new();
    if info.body.string_index_type.is_some() {
        keys.extend([Type::String, Type::Number]);
    } else if info.body.number_index_type.is_some() {
        keys.push(Type::Number);
    }
    let mut seen = std::collections::HashSet::new();
    for member in &info.body.members {
        if seen.insert(member.name.as_str())
            && let Some(key) = property_name_key_type(&member.name)
        {
            keys.push(key);
        }
    }
    Some(union_type(keys))
}

/// `keyof (A | B)` is the intersection of the members' key sets (tsc's
/// `getIndexType` over a union): a named key survives when every member has it
/// or answers it through a string index. `None` when a member is not an object
/// surge fully models, or when every member carries a string index — the key
/// set is then `string | number`, which the mapped-type and indexed-access
/// resolvers only accept as literal keys of the source.
fn union_key_type(members: &[Type]) -> Option<Type> {
    let objects = members
        .iter()
        .map(|member| match member.peeled() {
            Type::Object(object) if !object.synthetic_open_index => Some(object),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if objects.iter().all(|object| object.string_index_type.is_some()) {
        return None;
    }
    let mut named: Vec<&str> = objects
        .iter()
        .find(|object| object.string_index_type.is_none())?
        .properties
        .iter()
        .filter(|(key, property)| is_public_key(key, property))
        .map(|(key, _)| key.as_ref())
        .collect();
    named.retain(|key| {
        objects
            .iter()
            .all(|object| object.string_index_type.is_some() || object.properties.contains_key(*key))
    });
    let mut keys: Vec<Type> = named.into_iter().filter_map(property_name_key_type).collect();
    Some(match keys.len() {
        0 => Type::Never,
        1 => keys.remove(0),
        _ => union_type(keys),
    })
}

/// `typeof a.b<X, Y>` as written, for display.
fn written_type_query(type_of: &surge_ts_syntax::ParsedTypeOfType) -> String {
    let mut written = format!("typeof {}", std::iter::once(type_of.name.as_str()).chain(type_of.members.iter().map(String::as_str)).collect::<Vec<_>>().join("."));
    let arguments: Vec<String> = type_of
        .type_arguments
        .iter()
        .map(|argument| crate::driver::parsed_type_display(argument).unwrap_or_else(|| "?".to_string()))
        .collect();
    written.push_str(&format!("<{}>", arguments.join(", ")));
    written
}
