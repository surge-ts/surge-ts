//! Type alias resolution and built-in utility types (Partial/Record/Pick/Omit/...).

use super::*;

use surge_ts_syntax::{ParsedType, TextSpan};
use surge_ts_types::{ObjectProperty, PropertyMap, Type};

use crate::context::{CheckerContext, DeclarationResolutionKey};
use crate::default_lib::{is_generated_default_lib_file_name, is_physical_default_lib_file_name};
use crate::metrics::alloc_object_type;
use crate::symbols::{TypeAliasInfo, TypeDeclarationHandle};

/// Whether a type alias body introduces a structural boundary that makes a
/// self-reference legal (tsc's rule for recursive type aliases). Object, array,
/// tuple, function, mapped and template-literal bodies all qualify; a union or
/// intersection qualifies when any member does, and so does a conditional. A
/// bare alias reference, indexed access, etc. do not, so `type A = A` stays an
/// error.
fn alias_body_supports_recursion(ty: &ParsedType, ctx: &CheckerContext) -> bool {
    match ty {
        ParsedType::Object(_)
        | ParsedType::Array(_)
        | ParsedType::Tuple(_)
        | ParsedType::Function(_)
        | ParsedType::Mapped(_)
        | ParsedType::TemplateLiteral(_)
        // tsc has allowed a type alias to recurse through a conditional since
        // 4.1 (`type TuplePrefixes<T> = T extends readonly [] ? readonly []
        // : TuplePrefixes<DropLast<T>> | T`). Treating it as an illegal cycle
        // degraded every declaration that reached it — tanstack's
        // `QueryFilters.queryKey` tainted the whole query-core graph.
        | ParsedType::Conditional(_) => true,
        // A reference to a generic class or interface whose arguments may name
        // an alias is created deferred (`isDeferredTypeReferenceNode`), so
        // React's `type ReactNode = … | Iterable<ReactNode> | …` recurses
        // through it legally.
        ParsedType::Named(named) => {
            !named.type_arguments.is_empty()
                && !matches!(
                    ctx.lookup_type_declaration(&named.name),
                    Some(crate::symbols::TypeDeclarationInfo::Alias(_))
                )
                && named.type_arguments.iter().any(may_resolve_type_alias)
        }
        ParsedType::Union(members) | ParsedType::Intersection(members) => members
            .iter()
            .any(|member| alias_body_supports_recursion(member, ctx)),
        _ => false,
    }
}

/// tsc's `mayResolveTypeAlias`, read off the syntax: a type argument that names
/// a type (which may be an alias) or is built from one.
fn may_resolve_type_alias(ty: &ParsedType) -> bool {
    match ty {
        ParsedType::Named(_) | ParsedType::TypeOf(_) => true,
        ParsedType::KeyOf(inner) | ParsedType::Readonly(inner) => may_resolve_type_alias(inner),
        ParsedType::Union(members) | ParsedType::Intersection(members) => {
            members.iter().any(may_resolve_type_alias)
        }
        ParsedType::IndexedAccess(access) => {
            may_resolve_type_alias(&access.object_type) || may_resolve_type_alias(&access.index_type)
        }
        ParsedType::Conditional(conditional) => {
            may_resolve_type_alias(&conditional.check_type)
                || may_resolve_type_alias(&conditional.extends_type)
                || may_resolve_type_alias(&conditional.true_type)
                || may_resolve_type_alias(&conditional.false_type)
        }
        _ => false,
    }
}

/// The namespace a member was declared in (`React` for `React.MouseEvent`),
/// from the *original* declared name so a renamed import still finds its
/// siblings; `None` for a top-level declaration.
pub(crate) fn namespace_member_prefix(declared_name: Option<&str>, name: &str) -> Option<String> {
    declared_name
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|(prefix, _)| prefix.to_string())
}

/// Opt-in (`SURGE_GENERIC_RECURSIVE_ALIAS=1`): give a generic alias's
/// resolution frame the identity of its *instantiation* rather than its
/// declaration, so recursing into itself with different arguments is no longer
/// read as a cycle. Without it a recursive generic record (a router/client
/// proxy built from `{ [K in keyof T]: … Self<T[K]> }`) collapses to the
/// degradation sentinel and every read downstream of it goes silent.
///
/// Off by default: it costs two tanstack-query false positives, where a
/// recursive tuple-prefix union loses its recursive member. See
/// REAL_PROJECT_COMPAT.md. The back-edge itself deliberately stays the
/// sentinel — handing it a lazy self-reference overflows the stack in the
/// intersection merge.
fn generic_recursive_alias_references() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_GENERIC_RECURSIVE_ALIAS").as_deref() == Ok("1"))
}

/// Nested instantiations of the *same* generic declaration that are allowed
/// before the back-edge is treated as a cycle. A router/client proxy nests as
/// deep as its record (`post.listPosts`), never far; the cap is what keeps a
/// genuinely unbounded recursion (`type A<T> = A<T[]>`) from expanding forever
/// once distinct arguments stop it colliding with itself.
const MAX_NESTED_INSTANTIATIONS: usize = 8;

/// Total depth of the declaration-resolution stack before an instantiation is
/// abandoned as excessively deep. Discriminating frames by their arguments lets
/// a chain that used to stop at the first repeat keep going, so a genuinely
/// unbounded generic (tRPC's builder chain rebuilds its router type per
/// method) needs a ceiling of its own or it exhausts the process stack. This is
/// the same guard tsc spends its `TS2589` on.
const MAX_RESOLUTION_DEPTH: usize = 24;

/// Total generic instantiations one *root* resolution may perform before the
/// alias being resolved is abandoned to the degradation sentinel.
///
/// `MAX_RESOLUTION_DEPTH` and `MAX_NESTED_INSTANTIATIONS` are both path-local:
/// they look only at the frames currently on `resolving`. Once frames are
/// discriminated by their arguments, a type-level program that *fans out* --
/// ts-pattern's `DeepExclude` / `InvertPattern` walk a pattern against a value
/// type and instantiate a fresh argument tuple per member, per level -- never
/// repeats an argument tuple and never nests deeply, so neither guard fires
/// while the total work grows exponentially. This is the breadth ceiling that
/// bounds it, and it is a last-resort safety net: the primary termination
/// model remains repeated-state detection on the argument fingerprint.
///
/// 200 is empirical, and chosen as the largest value that keeps every corpus's
/// diagnostics at or better than their gate-off values. Measured 2026-09-14 on
/// ts-pattern with one frozen binary: the cost is superlinear in the ceiling
/// (200 -> ~5 s, 500 -> 14 s, 1000 -> 35 s, 2000 and 5000 -> still running at
/// 240 s), and dropping to 50 terminates faster but costs zod its exact 21/21
/// parity, so the window is real rather than arbitrary.
const MAX_ROOT_INSTANTIATION_WORK: usize = 200;

/// The breadth ceiling, overridable for tuning (`SURGE_ROOT_INSTANTIATION_WORK`).
fn max_root_instantiation_work() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SURGE_ROOT_INSTANTIATION_WORK")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(MAX_ROOT_INSTANTIATION_WORK)
    })
}

/// Opt-in (`SURGE_ROOT_WORK_TRIP=1`): report which declaration exhausted the
/// breadth budget, once per declaration.
fn report_work_trip(key: &DeclarationResolutionKey) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("SURGE_ROOT_WORK_TRIP").is_some()) {
        return;
    }
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    let label = format!("{}::{}", key.file_name, key.name);
    if let Ok(mut seen) = SEEN.lock() {
        let set = seen.get_or_insert_with(std::collections::HashSet::new);
        if set.insert(label.clone()) {
            eprintln!("[work-trip] {label}");
        }
    }
}

/// Whether the alias recurses only in *tail* position of a conditional
/// (`type Mutate<S, Ms> = … ? S : Ms extends [...] ? Mutate<…, …> : never`).
///
/// Such a recursion terminates by consuming its arguments and produces exactly
/// one type per step, so discriminating its frames by their arguments (rather
/// than reading the back-edge as a cycle) costs one instantiation per step. A
/// self-reference nested inside a structure (a mapped type's member, a tuple,
/// a union arm) fans out instead: ts-pattern's pattern aliases expand 57x
/// slower when those are followed, which is why the general case stays behind
/// [`generic_recursive_alias_references`].
fn alias_is_tail_recursive(alias: &TypeAliasInfo) -> bool {
    fn mentions(ty: &ParsedType, name: &str) -> bool {
        let mut found = false;
        ty.for_each_named_type(&mut |named| found |= named.name == name);
        found
    }

    fn branch_is_tail(ty: &ParsedType, name: &str) -> bool {
        match ty {
            ParsedType::Named(named) if named.name == name => true,
            ParsedType::Conditional(conditional) => {
                !mentions(&conditional.check_type, name)
                    && !mentions(&conditional.extends_type, name)
                    && branch_is_tail(&conditional.true_type, name)
                    && branch_is_tail(&conditional.false_type, name)
            }
            other => !mentions(other, name),
        }
    }

    !alias.body.type_parameters.is_empty()
        && matches!(&alias.body.ty, ParsedType::Conditional(_))
        && branch_is_tail(&alias.body.ty, &alias.name)
        && mentions(&alias.body.ty, &alias.name)
}

/// The `resolving`-stack identity for one *instantiation*.
///
/// The plain declaration key carries no arguments, so `Decorate<{post: …}>` and
/// the `Decorate<{listPosts: …}>` its own body asks for collide and the second
/// is read as a self-cycle — which is why every recursive generic record
/// collapsed to the degradation sentinel. Discriminating the frame by its
/// resolved arguments lets a terminating recursion resolve concretely, and
/// still collides exactly when the arguments repeat, which is the real cycle.
fn instantiation_frame_key(
    declaration_key: &DeclarationResolutionKey,
    is_generic: bool,
    pre_resolved_arguments: Option<&[Type]>,
) -> DeclarationResolutionKey {
    if !is_generic {
        return declaration_key.clone();
    }
    let Some(arguments) = pre_resolved_arguments.filter(|arguments| !arguments.is_empty()) else {
        return declaration_key.clone();
    };
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    for argument in arguments {
        std::hash::Hasher::write_u64(
            &mut hasher,
            crate::speculative::display_type_fingerprint(argument),
        );
    }
    let fingerprint = std::hash::Hasher::finish(&hasher);
    DeclarationResolutionKey {
        fingerprint: fingerprint | (1u64 << 63),
        ..declaration_key.clone()
    }
}

/// Whether `resolving` already holds `MAX_NESTED_INSTANTIATIONS` frames of this
/// declaration, whatever their arguments.
fn nested_instantiation_limit_reached(
    resolving: &[DeclarationResolutionKey],
    declaration_key: &DeclarationResolutionKey,
) -> bool {
    resolving
        .iter()
        .filter(|frame| {
            frame.file_name == declaration_key.file_name
                && frame.name == declaration_key.name
                && frame.namespace == declaration_key.namespace
        })
        .count()
        >= MAX_NESTED_INSTANTIATIONS
}

pub(crate) fn resolve_type_alias(
    alias: &TypeAliasInfo,
    handle: TypeDeclarationHandle,
    type_arguments: Vec<ParsedType>,
    reference_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
    pre_resolved_arguments: Option<&[Type]>,
    pre_resolved_errors: Option<&[bool]>,
) -> ResolvedType {
    let declaration_key = super::cache::alias_resolution_key(alias);
    // Under the instantiation-aware gate a generic back-edge is a cycle only
    // when its *arguments* repeat, or when the nesting cap is reached.
    let instantiation_frames = generic_recursive_alias_references() || alias_is_tail_recursive(alias);
    let frame_key = if instantiation_frames {
        instantiation_frame_key(
            &declaration_key,
            !alias.body.type_parameters.is_empty(),
            pre_resolved_arguments,
        )
    } else {
        declaration_key.clone()
    };
    // A new root resolution starts with an empty stack; the breadth budget is
    // per root so that the same root always gets the same budget however the
    // program's files are scheduled across workers.
    if resolving.is_empty() {
        ctx.instantiation_work = 0;
    }
    let work_exhausted = if instantiation_frames && frame_key != declaration_key {
        ctx.instantiation_work = ctx.instantiation_work.saturating_add(1);
        let tripped = ctx.instantiation_work > max_root_instantiation_work();
        if tripped {
            report_work_trip(&declaration_key);
        }
        tripped
    } else {
        false
    };
    let nesting_exhausted = instantiation_frames
        && frame_key != declaration_key
        && (work_exhausted
            || resolving.len() + super::cache::lazy_peel_depth() * MAX_RESOLUTION_DEPTH / 4
                >= MAX_RESOLUTION_DEPTH
            || nested_instantiation_limit_reached(resolving, &declaration_key));
    if let Some(index) = resolving
        .iter()
        .position(|name| name == &frame_key)
        .or_else(|| nesting_exhausted.then(|| resolving.len().saturating_sub(1)))
    {
        ctx.note_resolution_cycle(index);
        // tsc only rejects a type alias that references itself *without* an
        // intervening structural type (`type A = A`, `type A = B; type B = A`).
        // Recursion through an object/array/tuple/function (`type Rec = { rest: Rec
        // }`) is valid and tsc reports nothing. For a *non-generic* structural alias
        // resolve the legal back-edge to a lazy nominal reference to the same
        // declaration: forcing it (a member access, an assignability probe) peels one
        // level back to the real recursive shape rather than `unknown`, so a
        // property/assignability check through the self-edge is not silently dropped.
        // The lazy peel stack bounds the re-expansion.
        //
        // A *generic* recursive declaration is left as `unknown`: its lazy peel is
        // bounded mid-instantiation, so forcing the deeply self-instantiating generic
        // clusters (a fluent builder whose every method returns `Builder<…refined…>`)
        // would expose an incomplete shape and over-report member/assignability
        // checks. Keeping `unknown` there preserves the previous (sound, if
        // under-reporting) behaviour. The (suppressed) note marks the degraded
        // resolution; a genuine structureless cycle keeps it as a real error.
        // A re-entry whose path passed through a structural frame (an interface
        // body, or a structural alias body) is legal recursion even when this
        // alias's own body is a bare union of named types — e.g. zod's
        // `type $ZodIssue = … | $ZodIssueInvalidUnion` whose member interface
        // carries `errors: $ZodIssue[][]`. Only a structureless chain
        // (`type A = B; type B = A`) is a genuine tsc error.
        let structural_crossing = ctx
            .structural_resolution_frames
            .iter()
            .any(|&frame| frame > index);
        let legal_recursion =
            alias_body_supports_recursion(&alias.body.ty, ctx) || structural_crossing;
        // A *generic* back-edge stays the degradation sentinel even under the
        // gate. With frames discriminated by their arguments this branch is
        // only reached when the arguments actually repeat — a genuinely
        // infinite type — and handing that back as a lazy self-reference is
        // what the intersection merge then peels forever.
        // A generic back-edge written inside an object type literal's member
        // (`Chainable<p> = p & Omit<{ optional(): Chainable<p, …> }, k>`) sits
        // where tsc resolves lazily, so a nominal self-reference is what it
        // sees there; the reference is only peeled on demand, bounded by the
        // lazy peel stack. A back-edge at the top of the body (`type A<T> =
        // A<T> & X`) opens no such frame and keeps degrading.
        let literal_member_back_edge = !alias.body.type_parameters.is_empty()
            && ctx
                .type_literal_member_frames
                .iter()
                .any(|&frame| frame > index);
        if legal_recursion && (alias.body.type_parameters.is_empty() || literal_member_back_edge)
        {
            // The lazy reference re-expands from these arguments on peel, and
            // answers from the interner by them. A generic back-edge reached
            // inside an open type-parameter scope arrives without them resolved
            // (`D<R[K]>` under `R`, `K`); an empty list would peel `D` with no
            // arguments at all, so they are resolved here under this frame's
            // substitution.
            let resolved_back_edge_arguments;
            let pre_resolved_arguments = match pre_resolved_arguments {
                None if !type_arguments.is_empty() => {
                    resolved_back_edge_arguments = type_arguments
                        .iter()
                        .map(|argument| {
                            resolve_parsed_type(argument.clone(), ctx, resolving, substitution).ty
                        })
                        .collect::<Vec<_>>();
                    Some(resolved_back_edge_arguments.as_slice())
                }
                other => other,
            };
            return ResolvedType {
                ty: make_recursive_cycle_reference(
                    ctx,
                    &alias.name,
                    handle,
                    declaration_key,
                    type_arguments,
                    pre_resolved_arguments,
                    substitution,
                ),
                had_error: false,
            };
        }
        ctx.note_unknown_resolution_cycle(index);
        if !legal_recursion {
            emit_type_alias_cycle(&alias.name, alias.name_span, ctx);
        }
        if !legal_recursion && crate::infer::types::interface::had_error_trace_enabled() {
            eprintln!(
                "[had-error] alias-cycle '{}' cp={}",
                alias.name,
                crate::program::in_check_phase()
            );
        }
        return ResolvedType {
            ty: Type::Unknown,
            // Abandoning a recursion at the nesting/depth cap is *not* a
            // resolved answer: the shape it would have produced is merely
            // unfinished, so the cap degrades rather than handing back a clean
            // sentinel a consumer would then trust. The taint also keeps the
            // truncated shape out of every cache. A genuine repeated-argument
            // cycle keeps the existing `!legal_recursion` answer.
            had_error: !legal_recursion || nesting_exhausted,
        };
    }

    resolving.push(frame_key);
    // See the matching comment in `resolve_interface`: an empty per-file
    // fallback (ambient-module files) must not clobber the installed scope.
    let effective_scope = alias.resolution_scope.clone().or_else(|| {
        ctx.module_scope_for_file(&alias.file_name)
            .filter(|scope| !scope.is_empty())
    });
    // A default is authored inside the declaring namespace and names its
    // siblings bare (express's `Request<P = ParamsDictionary, …>` under
    // `namespace e`), so it binds under the same prefix the body resolves under.
    let default_prefix = namespace_member_prefix(alias.declared_name.as_deref(), &alias.name);
    if let Some(prefix) = default_prefix.clone() {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    let bound = bind_type_arguments(
        &alias.body.type_parameters,
        type_arguments,
        &alias.name,
        reference_span.or(alias.name_span),
        reference_span.or(alias.name_span),
        ctx,
        resolving,
        substitution,
        pre_resolved_arguments,
        pre_resolved_errors,
        Some((&effective_scope, &alias.file_name)),
    );
    if default_prefix.is_some() {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
    let Some(bound_arguments) = bound else {
        resolving.pop();
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    };
    let arguments_had_error = bound_arguments.had_error;
    let local_substitution = bound_arguments.substitution;

    let from_default_lib =
        &*alias.file_name == "<built-in>" || is_generated_default_lib_file_name(&alias.file_name);
    // The physical lib models `Pick`/`Omit` as homomorphic mapped types
    // (`{[P in K]: T[P]}`) that should preserve each source property's optional
    // modifier, but `resolve_mapped_type` forces them required. Resolve them as
    // builtin utilities (which clone the source property, keeping optionality)
    // even from the physical lib.
    let physical_modifier_utility = is_physical_default_lib_file_name(&alias.file_name)
        && matches!(
            &*alias.name,
            "Pick"
                | "Omit"
                | "Required"
                | "Readonly"
                | "NoInfer"
                | "BuiltinIteratorReturn"
                | "Uppercase"
                | "Lowercase"
                | "Capitalize"
                | "Uncapitalize"
        );
    if from_default_lib || physical_modifier_utility {
        if let Some(resolved) = resolve_builtin_utility_alias(
            &alias.name,
            &local_substitution,
            reference_span.or(alias.name_span),
            ctx,
        ) {
            resolving.pop();
            return resolved;
        }
    }

    if &*alias.file_name == "<built-in>"
        && (&*alias.name == "Array" || &*alias.name == "ReadonlyArray")
    {
        resolving.pop();
        let element_type = local_substitution.get("T").cloned().unwrap_or(Type::Any);
        return ResolvedType {
            ty: Type::Array(Box::new(element_type)),
            had_error: false,
        };
    }

    // Derive the namespace prefix from the *original* declared name, not the local
    // binding: a namespace member imported by name (`import { MouseEventHandler }
    // from "react"`) is renamed to its bare form, but its body still references
    // siblings (`EventHandler`, `MouseEvent`) that only resolve under the `React.`
    // prefix. `declared_name` preserves the qualified source name (`React.X`).
    let namespace_prefix = alias
        .declared_name
        .as_deref()
        .unwrap_or(&alias.name)
        .rsplit_once('.')
        .map(|(prefix, _)| prefix.to_string());
    let is_namespace_member = namespace_prefix.is_some();
    // A declaration outside every namespace resolves its body in its own file's
    // top-level scope, not in the namespace whose member led here: React's
    // top-level `type NativeSubmitEvent = SubmitEvent` names the DOM's
    // `SubmitEvent`, not its namespace's `React.SubmitEvent`.
    let enclosing_namespaces = (!is_namespace_member).then(|| {
        (
            std::mem::take(&mut ctx.namespace_member_prefix_stack),
            std::mem::replace(&mut ctx.namespace_member_resolution_depth, 0),
        )
    });
    if let Some(prefix) = namespace_prefix {
        ctx.namespace_member_resolution_depth += 1;
        ctx.namespace_member_prefix_stack.push(prefix);
    }
    // A structural alias body (object/array/function/…) is a structural
    // crossing, like an interface body: a cycle re-entered through it is legal
    // recursion (see `CheckerContext::structural_resolution_frames`).
    let structural_frame = alias_body_supports_recursion(&alias.body.ty, ctx);
    if structural_frame {
        ctx.structural_resolution_frames.push(resolving.len() - 1);
    }
    ctx.push_type_parameter_constraints_only(&alias.body.type_parameters);
    let resolved = with_type_declaration_scope(&effective_scope, ctx, |ctx| {
        with_file_name(ctx, &alias.file_name, |ctx| {
            resolve_parsed_type_with_substitution(
                alias.body.ty.clone(),
                ctx,
                resolving,
                &local_substitution,
            )
        })
    });
    ctx.pop_type_parameter_scope();
    if structural_frame {
        ctx.structural_resolution_frames.pop();
    }
    if is_namespace_member {
        ctx.namespace_member_resolution_depth -= 1;
        ctx.namespace_member_prefix_stack.pop();
    }
    if let Some((stack, depth)) = enclosing_namespaces {
        ctx.namespace_member_prefix_stack = stack;
        ctx.namespace_member_resolution_depth = depth;
    }
    resolving.pop();

    if arguments_had_error
        && !resolved.had_error
        && crate::infer::types::interface::had_error_trace_enabled()
    {
        eprintln!(
            "[had-error] alias-args '{}' cp={}",
            alias.name,
            crate::program::in_check_phase()
        );
    }
    ResolvedType {
        ty: resolved.ty,
        had_error: resolved.had_error || arguments_had_error,
    }
}

/// A homomorphic lib utility (`Partial`, `Required`, `Readonly`: `{ [P in
/// keyof T]: T[P] }` with modifiers) over a type variable of the body being
/// checked stays a generic mapped type (`isGenericMappedType`) instead of
/// resolving members.
fn homomorphic_mapped_variable(
    alias_name: &str,
    source: &Type,
    modifiers: surge_ts_types::type_variable::MappedModifiers,
) -> Option<Type> {
    if !source.is_type_variable() {
        return None;
    }
    let keys = surge_ts_types::type_variable::keyof_variable(source)?;
    surge_ts_types::type_variable::mapped_variable(
        &keys,
        source,
        modifiers,
        Some(source),
        format!("{alias_name}<{}>", source.name()),
    )
}

/// A utility that rebuilds its source's property map must carry the source's
/// *checker-injected* openness marker, not just the index type it produced.
/// Without it the openness is laundered into a declared-looking index
/// signature, and every consumer that reads the marker rather than the index —
/// object spread, `noPropertyAccessFromIndexSignature` — treats a shape surge
/// could not enumerate as fully enumerated.
fn carry_open_marker(
    rebuilt: surge_ts_types::ObjectType,
    source: &surge_ts_types::ObjectType,
) -> surge_ts_types::ObjectType {
    if source.synthetic_open_index {
        return rebuilt.with_open_index_marker();
    }
    rebuilt
}

pub(crate) fn resolve_builtin_utility_alias(
    alias_name: &str,
    substitution: &TypeParameterSubstitution,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) -> Option<ResolvedType> {
    match alias_name {
        // A source that is neither an object nor a type variable maps through
        // the lib's mapped body (`instantiateMappedType`'s primitive, array and
        // tuple constituents).
        "Partial" | "Required"
            if substitution.get("T").is_some_and(|source| {
                !source.is_type_variable() && !source.is_unknown() && !matches!(source.peeled(), Type::Object(_))
            }) =>
        {
            None
        }
        "Partial" => Some(resolve_partial_utility_type(substitution)),
        "Required" => Some(resolve_required_utility_type(substitution)),
        "Readonly" => Some(resolve_readonly_utility_type(substitution)),
        // `NoInfer<T>` is an inference marker only (`type NoInfer<T> =
        // intrinsic` in the lib); in type position it is `T`.
        "NoInfer" => Some(ResolvedType {
            ty: substitution.get("T").cloned().unwrap_or(Type::Unknown),
            had_error: false,
        }),
        // checker.go `getDeclaredTypeOfTypeAlias`: the intrinsic marker of
        // this alias is `getBuiltinIteratorReturnType`.
        "BuiltinIteratorReturn" => Some(ResolvedType {
            ty: if ctx.options.strict_builtin_iterator_return { Type::Undefined } else { Type::Any },
            had_error: false,
        }),
        // `type Uppercase<S extends string> = intrinsic` and its three
        // siblings: tsc's `getStringMappingType`.
        "Uppercase" | "Lowercase" | "Capitalize" | "Uncapitalize" => {
            let kind = surge_ts_types::StringMappingKind::from_name(alias_name)?;
            let argument = substitution.get("S")?.clone();
            // A named argument is judged by what it names; a pattern keeps its
            // own shape rather than resolving to `string`.
            let argument = surge_ts_types::peel_to_pattern_literal(&argument);
            if argument.is_unmodelled() {
                return None;
            }
            Some(ResolvedType {
                ty: surge_ts_types::string_mapping_type(kind, &argument),
                had_error: false,
            })
        }
        "Record" => Some(resolve_record_utility_type(substitution)),
        "Pick" => Some(resolve_pick_utility_type(substitution, name_span, ctx)),
        "Omit" => Some(resolve_omit_utility_type(substitution)),
        "Parameters" => Some(resolve_parameters_utility_type(substitution)),
        "ReturnType" => Some(resolve_return_type_utility_type(substitution)),
        _ => None,
    }
}

pub(crate) fn resolve_partial_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let modifiers = surge_ts_types::type_variable::MappedModifiers {
        readonly: 0,
        optional: 1,
    };
    if let Some(mapped) = homomorphic_mapped_variable("Partial", &source_type, modifiers) {
        return ResolvedType {
            ty: mapped,
            had_error: false,
        };
    }

    let Type::Object(object_type) = source_type.peeled() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let mut properties = PropertyMap::default();
    for (name, property) in object_type.properties.iter() {
        if crate::infer::types::resolve::is_public_key(name, property) {
            properties.insert(name.clone(), ObjectProperty::optional(property.ty.clone()));
        }
    }

    ResolvedType {
        // A homomorphic mapped type preserves its source's index signature.
        ty: Type::Object(carry_open_marker(
            alloc_object_type(
                properties,
                object_type.string_index_type.as_deref().cloned(),
            ),
            &object_type,
        )),
        had_error: false,
    }
}

/// `Required<T>`: every property of `T` becomes required (the inverse of
/// `Partial`). The lib models it as `{ [P in keyof T]-?: T[P] }`, whose `-?`
/// modifier `parse_mapped_type` cannot represent, so resolve it directly here.
pub(crate) fn resolve_required_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let modifiers = surge_ts_types::type_variable::MappedModifiers {
        readonly: 0,
        optional: -1,
    };
    if let Some(mapped) = homomorphic_mapped_variable("Required", &source_type, modifiers) {
        return ResolvedType {
            ty: mapped,
            had_error: false,
        };
    }

    let Type::Object(object_type) = source_type.peeled() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    // `Required<T>` only strips the optional *modifier* (`-?`); it keeps each
    // property's declared type intact, including an explicit `| undefined` member
    // (`jitter?: boolean | … | undefined` stays assignable from `undefined`).
    let mut properties = PropertyMap::default();
    for (name, property) in object_type.properties.iter() {
        if crate::infer::types::resolve::is_public_key(name, property) {
            properties.insert(name.clone(), ObjectProperty::required(property.ty.clone()));
        }
    }

    ResolvedType {
        ty: Type::Object(carry_open_marker(
            alloc_object_type(
                properties,
                object_type.string_index_type.as_deref().cloned(),
            ),
            &object_type,
        )),
        had_error: false,
    }
}

/// `Readonly<T>`: identity for our purposes — surge does not model the `readonly`
/// modifier, so the type is structurally unchanged. The lib models it as
/// `{ readonly [P in keyof T]: T[P] }`, which `parse_mapped_type` degrades; clone
/// the source object's shape instead.
pub(crate) fn resolve_readonly_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    ResolvedType {
        ty: readonly_shape(&source_type),
        had_error: false,
    }
}

/// `Readonly<T>`: an object's properties become `readonly`; an array or tuple
/// becomes the readonly shape; a union distributes (the mapped type is
/// homomorphic); anything else is itself.
fn readonly_shape(source: &Type) -> Type {
    // A mapped type over a type variable stays deferred in tsc; the variable
    // itself would claim to be it.
    if source.is_type_variable() {
        let modifiers = surge_ts_types::type_variable::MappedModifiers {
            readonly: 1,
            optional: 0,
        };
        return homomorphic_mapped_variable("Readonly", source, modifiers).unwrap_or(Type::Unknown);
    }
    match source.peeled() {
        Type::Object(object_type) => {
            let mut properties = PropertyMap::default();
            for (name, property) in object_type.properties.iter() {
                if !crate::infer::types::resolve::is_public_key(name, property) {
                    continue;
                }
                properties.insert(
                    name.clone(),
                    ObjectProperty {
                        readonly: true,
                        ..property.clone()
                    },
                );
            }
            Type::Object(carry_open_marker(
                alloc_object_type(properties, object_type.string_index_type.as_deref().cloned())
                    .with_readonly_indexes(true, false),
                &object_type,
            ))
        }
        sequence @ (Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)) => {
            crate::infer::types::resolve::readonly_reference(sequence)
        }
        Type::Union(union) => {
            surge_ts_types::union_type(union.types().iter().map(readonly_shape).collect())
        }
        other => other,
    }
}

pub(crate) fn resolve_record_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(key_type) = substitution.get("K").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    if key_type.is_type_variable() {
        let value_type = substitution.get("T").cloned().unwrap_or(Type::Unknown);
        let modifiers = surge_ts_types::type_variable::MappedModifiers {
            readonly: 0,
            optional: 0,
        };
        let name = format!("Record<{}, {}>", key_type.name(), value_type.name());
        if !value_type.is_unmodelled()
            && let Some(mapped) =
                surge_ts_types::type_variable::mapped_constant_variable(&key_type, &value_type, modifiers, name)
        {
            return ResolvedType {
                ty: mapped,
                had_error: false,
            };
        }
    }

    if record_key_is_open(&key_type) {
        return ResolvedType {
            ty: Type::Object(alloc_object_type(
                PropertyMap::default(),
                Some(substitution.get("T").cloned().unwrap_or(Type::Unknown)),
            )),
            had_error: false,
        };
    }

    let Some(keys) = record_literal_keys(&key_type) else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let value_type = substitution.get("T").cloned().unwrap_or(Type::Unknown);
    let mut properties = PropertyMap::default();

    for key in keys {
        properties.insert(key.into(), ObjectProperty::required(value_type.clone()));
    }

    ResolvedType {
        ty: Type::Object(alloc_object_type(properties, None)),
        had_error: false,
    }
}

pub(crate) fn resolve_pick_utility_type(
    substitution: &TypeParameterSubstitution,
    _name_span: Option<TextSpan>,
    _ctx: &mut CheckerContext,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    // `Pick<T, K>` is `{ [P in K]: T[P] }`: over generic keys it is a generic
    // mapped type, whose modifiers type is `T` (the declared `K extends
    // keyof T`).
    if let Some(key_type) = substitution.get("K")
        && key_type.is_type_variable()
        && source_type.is_type_variable()
        && let Some(mapped) = surge_ts_types::type_variable::mapped_variable(
            key_type,
            &source_type,
            surge_ts_types::type_variable::MappedModifiers {
                readonly: 0,
                optional: 0,
            },
            Some(&source_type),
            format!("Pick<{}, {}>", source_type.name(), key_type.name()),
        )
    {
        return ResolvedType {
            ty: mapped,
            had_error: false,
        };
    }

    let peeled_source = source_type.peeled();
    // A tuple or array picks off its own surface (`Pick<[number], 'length'>`
    // is `{ length: 1 }`).
    if matches!(peeled_source, Type::Tuple(_) | Type::Array(_) | Type::OpenTuple(_))
        && let Some(keys) = substitution.get("K").and_then(string_literal_union_keys)
    {
        let mut properties = PropertyMap::default();
        for key in keys {
            let Some(ty) = peeled_source.get_property_access_type(&key).filter(|ty| !ty.is_unmodelled()) else {
                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: false,
                };
            };
            properties.insert(key.into(), surge_ts_types::ObjectProperty::required(ty));
        }
        return ResolvedType {
            ty: Type::Object(alloc_object_type(properties, None)),
            had_error: false,
        };
    }
    let Type::Object(object_type) = peeled_source else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Some(key_type) = substitution.get("K").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Some(keys) = string_literal_union_keys(&key_type) else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let mut properties = PropertyMap::default();
    for key in keys {
        let Some(property) = object_type
            .properties
            .get(key.as_str())
            .filter(|property| crate::infer::types::resolve::is_public_key(&key, property))
        else {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: true,
            };
        };

        properties.insert(key.into(), property.clone());
    }

    ResolvedType {
        ty: Type::Object(alloc_object_type(properties, None)),
        had_error: false,
    }
}

pub(crate) fn resolve_omit_utility_type(substitution: &TypeParameterSubstitution) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    // `Omit<any, K>` is `Pick<any, Exclude<string | number | symbol, K>>`: a
    // mapped type over the non-literal keys, so every member reads `any`
    // through an index signature. Only the error type (an unresolved name) is
    // known to be that `any`; surge's `Any` doubles as a modelling placeholder,
    // which `keyof` does not key either.
    if matches!(source_type, Type::ErrorType) {
        let mut object = alloc_object_type(PropertyMap::default(), Some(Type::Any));
        object.number_index_type = Some(std::sync::Arc::new(Type::Any));
        return ResolvedType {
            ty: Type::Object(object),
            had_error: false,
        };
    }

    let Type::Object(object_type) = source_type.peeled() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Some(key_type) = substitution.get("K").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Some(keys) = string_literal_union_keys(&key_type) else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let keys: surge_ts_types::fx::FxHashSet<&str> = keys.iter().map(String::as_str).collect();
    let mut properties = PropertyMap::default();
    for (key, property) in object_type.properties.iter() {
        if keys.contains(key.as_ref()) || !crate::infer::types::resolve::is_public_key(key, property) {
            continue;
        }

        properties.insert(key.clone(), property.clone());
    }

    ResolvedType {
        // `Omit<T, K>` is `Pick<T, Exclude<keyof T, K>>`, and `keyof T` contains
        // `string` whenever `T` has a string index signature — so the result stays
        // open. Dropping it made every unlisted member of an open source read as
        // missing.
        ty: Type::Object(carry_open_marker(
            alloc_object_type(
                properties,
                object_type.string_index_type.as_deref().cloned(),
            ),
            &object_type,
        )),
        had_error: false,
    }
}

pub(crate) fn resolve_parameters_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Type::Function(function_type) = source_type.peeled() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    // `Parameters<(...args: A[]) => R>` is `A[]`, not a one-tuple holding the
    // array; a rest parameter spreads into the parameter list. A rest behind
    // fixed parameters (`[a: string, ...rest: number[]]`) has no tuple shape
    // surge can express and keeps the array as its last element.
    let parameters = function_type.parameters();
    let ty = match parameters {
        [rest] if function_type.is_variadic() => rest.clone(),
        _ => optional_parameter_tuple(
            parameters,
            function_type.required_parameter_count(),
            function_type.is_variadic(),
        ),
    };
    ResolvedType {
        ty,
        had_error: false,
    }
}

/// The parameter list as a tuple. An optional parameter is an optional tuple
/// element, spelled as a slot that accepts `undefined`; so is a trailing rest,
/// whose slot may be left out entirely.
pub(crate) fn optional_parameter_tuple(
    parameters: &[Type],
    required: usize,
    trailing_rest: bool,
) -> Type {
    let last = parameters.len().saturating_sub(1);
    Type::Tuple(
        parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                let optional = index >= required || (trailing_rest && index == last);
                if optional && !surge_ts_types::is_assignable_to(&Type::Undefined, parameter) {
                    surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
                } else {
                    parameter.clone()
                }
            })
            .collect(),
    )
}

pub(crate) fn resolve_return_type_utility_type(
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let Some(source_type) = substitution.get("T").cloned() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    let Type::Function(function_type) = source_type.peeled() else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    ResolvedType {
        ty: function_type.return_type().clone(),
        had_error: false,
    }
}

/// Whether a `Record` key admits arbitrary members, which makes the record an
/// index signature rather than a fixed property set. `number` and `symbol` are
/// as open as `string`; so is a union that contains one of them.
fn record_key_is_open(key_type: &Type) -> bool {
    match key_type {
        Type::String | Type::Number | Type::Symbol => true,
        Type::Union(union) => union.types().iter().any(record_key_is_open),
        // A pattern key (`` `data-${string}` ``) is a string index restricted
        // to the pattern; the open string index is the nearest thing surge
        // models.
        other => {
            surge_ts_types::is_template_literal_type(other)
                || surge_ts_types::string_mapping_parts(other).is_some()
        }
    }
}

/// The property names a literal `Record` key enumerates. A numeric key names the
/// member by its text, the same way an object literal's numeric key does.
fn record_literal_keys(key_type: &Type) -> Option<Vec<String>> {
    match key_type {
        Type::StringLiteral(value) => Some(vec![value.clone()]),
        Type::NumberLiteral(literal) => Some(vec![literal.value.clone()]),
        Type::Reference(reference) if reference.enum_owner.is_some() => match reference.resolve() {
            resolved @ (Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::Union(_)) => {
                record_literal_keys(&resolved)
            }
            _ => None,
        },
        Type::Union(union) => {
            let mut keys = Vec::new();
            for variant in union.types() {
                keys.extend(record_literal_keys(variant)?);
            }
            Some(keys)
        }
        _ => None,
    }
}

pub(crate) fn string_literal_union_keys(ty: &Type) -> Option<Vec<String>> {
    match ty {
        // `never` is the empty key set: `Omit<T, never>` is `T` (ts-pattern's
        // `Chainable<p, omitted = never>` is written exactly that way) and
        // `Pick<T, never>` is `{}`. Refusing it degraded both to the sentinel.
        Type::Never => Some(Vec::new()),
        Type::StringLiteral(value) => Some(vec![value.clone()]),
        Type::Union(union) => {
            let mut keys = Vec::new();
            for variant in union.types() {
                match variant {
                    Type::StringLiteral(value) => keys.push(value.clone()),
                    _ => return None,
                }
            }
            Some(keys)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use surge_ts_types::ObjectType;

    fn optional_object() -> Type {
        let mut props = PropertyMap::default();
        props.insert("a".into(), ObjectProperty::optional(Type::Number));
        props.insert("b".into(), ObjectProperty::optional(Type::String));
        Type::Object(ObjectType::new(props, None))
    }

    #[test]
    fn omit_preserves_source_property_optionality() {
        let mut sub = TypeParameterSubstitution::new();
        sub.insert("T".to_string(), optional_object());
        sub.insert("K".to_string(), Type::StringLiteral("b".to_string()));

        let resolved = resolve_omit_utility_type(&sub);
        let Type::Object(object) = resolved.ty else {
            panic!("Omit must resolve to an object");
        };
        let a = object.properties.get("a").expect("`a` is kept");
        assert!(
            a.is_optional(),
            "Omit must keep `a` optional, not force it required"
        );
        assert!(object.properties.get("b").is_none(), "`b` is omitted");
    }
}
