//! `SURGE_DEFERRED_SHADOW=1` (with `SURGE_LAZY_PROBE=1`) sizes type-level
//! instantiation against today's per-substitution resolution. A conditional or
//! mapped node resolved under concrete bindings is read again with those
//! bindings replaced by type variables, and the read is instantiated at the
//! type level with the bindings: how often that reproduces the resolution is
//! how far reading a declaration once and instantiating it would carry.
//! `SURGE_DEFERRED_SHADOW_LOG=<path>` appends each disagreement.

use std::cell::Cell;
use std::io::Write;
use std::sync::Arc;

use surge_ts_syntax::ParsedType;
use surge_ts_types::type_variable::{TypeVariableScope, mentions_type_variable};
use surge_ts_types::{Type, TypeCopyReason, TypeParameterType};

use super::{ResolvedType, TypeParameterSubstitution};
use crate::checks::function::generic_return;
use crate::context::{CheckerContext, DeclarationResolutionKey};

pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        surge_ts_types::lazy_probe::enabled() && std::env::var_os("SURGE_DEFERRED_SHADOW").is_some()
    })
}

thread_local! {
    static IN_SHADOW: Cell<bool> = const { Cell::new(false) };
    static SEEN: std::cell::RefCell<surge_ts_types::fx::FxHashSet<u64>> = Default::default();
    /// Why a resolution inside the current shadow read gave up: the first
    /// exit to the sentinel and the last one.
    static DEGRADED_FIRST: Cell<Option<&'static str>> = const { Cell::new(None) };
    static DEGRADED_LAST: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Records why a resolution degraded, while a shadow read is under way.
pub(crate) fn note_degrade(reason: &'static str) {
    if IN_SHADOW.with(Cell::get) {
        DEGRADED_FIRST.with(|first| {
            if first.get().is_none() {
                first.set(Some(reason));
            }
        });
        DEGRADED_LAST.with(|last| last.set(Some(reason)));
    }
}

fn written_kind(ty: &ParsedType) -> &'static str {
    match ty {
        ParsedType::Named(named) if named.type_arguments.is_empty() => "name",
        ParsedType::Named(_) => "reference",
        ParsedType::IndexedAccess(_) => "indexed",
        ParsedType::KeyOf(_) => "keyof",
        ParsedType::Tuple(..) | ParsedType::VariadicTuple(..) => "tuple",
        ParsedType::Array(_) | ParsedType::Readonly(_) => "array",
        ParsedType::Object(_) => "object",
        ParsedType::Function(_) => "function",
        ParsedType::Union(_) => "union",
        ParsedType::Intersection(_) => "intersection",
        ParsedType::Conditional(_) => "conditional",
        ParsedType::Mapped(_) => "mapped",
        ParsedType::TemplateLiteral(_) => "template",
        ParsedType::TypeOf(_) => "typeof",
        ParsedType::Infer(_) => "infer",
        ParsedType::StringLiteral(_) | ParsedType::NumberLiteral(_) | ParsedType::BooleanLiteral(_) => "literal",
        _ => "keyword",
    }
}

/// The written shape of a shadowed node: check and extends kinds for a
/// conditional, the key constraint's kind for a mapped type.
fn written_shape(node: &ParsedType, infer: bool) -> String {
    match node {
        ParsedType::Conditional(conditional) => format!(
            "{}>{}{}",
            written_kind(&conditional.check_type),
            written_kind(&conditional.extends_type),
            if infer { "+infer" } else { "" }
        ),
        ParsedType::Mapped(mapped) => match mapped.constraint.as_ref() {
            ParsedType::KeyOf(operand) => match operand.as_ref() {
                ParsedType::Named(named) => format!("in:keyof({}{})", named.name, if named.type_arguments.is_empty() { "" } else { "<…>" }),
                other => format!("in:keyof({})", written_kind(other)),
            },
            other => format!("in:{}", written_kind(other)),
        },
        other => written_kind(other).to_string(),
    }
}

/// Shadows per run: each one resolves its node a second time.
fn cap() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SURGE_DEFERRED_SHADOW_CAP").ok().and_then(|cap| cap.parse().ok()).unwrap_or(150_000)
    })
}

/// The written node's identity: the allocation its declaration shares.
fn node_address(node: &ParsedType) -> usize {
    match node {
        ParsedType::Conditional(conditional) => Arc::as_ptr(conditional) as usize,
        ParsedType::Mapped(mapped) => Arc::as_ptr(mapped) as usize,
        other => std::ptr::from_ref(other) as usize,
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Conditional,
    Mapped,
}

fn label(kind: Kind, outcome: &str) -> &'static str {
    thread_local! {
        static LABELS: std::cell::RefCell<std::collections::HashMap<String, &'static str>> = Default::default();
    }
    let name = format!(
        "shadow.{}.{outcome}",
        match kind {
            Kind::Conditional => "cond",
            Kind::Mapped => "mapped",
        }
    );
    LABELS.with(|labels| {
        *labels
            .borrow_mut()
            .entry(name.clone())
            .or_insert_with(|| Box::leak(name.into_boxed_str()))
    })
}

fn holds_sentinel(ty: &Type, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    match ty {
        Type::Unknown => true,
        Type::Union(union) => union.types().iter().any(|member| holds_sentinel(member, budget)),
        Type::Array(element) => holds_sentinel(element, budget),
        Type::Tuple(elements) => elements.iter().any(|element| holds_sentinel(element, budget)),
        Type::Function(function) => {
            function.parameters().iter().any(|parameter| holds_sentinel(parameter, budget))
                || holds_sentinel(function.return_type(), budget)
        }
        Type::Object(object) => {
            object.properties.values().any(|property| holds_sentinel(&property.ty, budget))
                || object.string_index_type.as_deref().is_some_and(|index| holds_sentinel(index, budget))
                || object.number_index_type.as_deref().is_some_and(|index| holds_sentinel(index, budget))
                || object
                    .intersection_operands
                    .as_deref()
                    .is_some_and(|operands| operands.iter().any(|operand| holds_sentinel(operand, budget)))
        }
        Type::Reference(reference) => reference.arguments.iter().any(|argument| holds_sentinel(argument, budget)),
        _ => false,
    }
}

/// Reads `node` again over variables for the bindings it reads and compares
/// the type-level instantiation with `resolved`. `excluded` names are bound by
/// the node itself (`infer` captures, a mapped key).
#[allow(clippy::too_many_arguments)]
pub(super) fn compare(
    kind: Kind,
    node: &ParsedType,
    excluded: &[String],
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
    resolved: &ResolvedType,
    resolve: impl FnOnce(&mut CheckerContext, &mut Vec<DeclarationResolutionKey>, &TypeParameterSubstitution) -> ResolvedType,
) {
    use surge_ts_types::lazy_probe::count;
    if IN_SHADOW.with(Cell::get) {
        return;
    }
    count(label(kind, "calls"));
    let kind_has_infer = matches!(kind, Kind::Conditional) && !excluded.is_empty();
    if resolved.had_error || resolved.ty.is_unmodelled() {
        count(label(kind, "skip.degraded"));
        return;
    }
    // A result that is degraded inside (a member left `unknown`) degraded
    // without any variable; a read over variables cannot reproduce it.
    if holds_sentinel(&resolved.ty, &mut 2_000) {
        count(label(kind, "skip.degraded_inside"));
        return;
    }
    if crate::checks::function::in_generic_body_read() || crate::semantic::serves_api() {
        count(label(kind, "skip.mode"));
        return;
    }
    if super::mapped::body_alias_display_pending() {
        count(label(kind, "alias_display_pending"));
    }
    let mut names = Vec::new();
    node.for_each_named_type(&mut |named| {
        if named.type_arguments.is_empty() && !excluded.contains(&named.name) {
            names.push(named.name.clone());
        }
    });
    names.sort();
    names.dedup();
    let bound: Vec<(String, Type)> = names
        .into_iter()
        .filter_map(|name| {
            let ty = substitution.get(&name)?;
            (!substitution.is_placeholder(&name)
                && !substitution.is_degraded(&name)
                && !ty.is_unknown()
                && !mentions_type_variable(ty))
            .then(|| (name, ty.clone()))
        })
        .collect();
    if bound.is_empty() {
        count(label(kind, "skip.no_bindings"));
        return;
    }
    // A second resolution on top of a deep one can overflow the stack.
    if resolving.len() >= 12 || crate::infer::types::cache::lazy_peel_depth() > 2 {
        count(label(kind, "skip.deep"));
        return;
    }
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = surge_ts_types::fx::FxHasher::default();
        node_address(node).hash(&mut hasher);
        for (name, ty) in &bound {
            name.hash(&mut hasher);
            surge_ts_types::type_conflict_digest(ty).hash(&mut hasher);
        }
        hasher.finish()
    };
    let fresh = SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.len() < cap() && seen.insert(key)
    });
    if !fresh {
        count(label(kind, "skip.repeat_or_cap"));
        return;
    }
    IN_SHADOW.with(|flag| flag.set(true));
    let declaration_file: Arc<str> = Arc::from("\u{0}deferred-shadow");
    let first = node_address(node) as u32;
    let scope = TypeVariableScope::enter(
        bound
            .iter()
            .enumerate()
            .map(|(index, (name, _))| (Arc::from(name.as_str()), (declaration_file.clone(), first.wrapping_add(index as u32)))),
    );
    let mut generic_substitution = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    for (name, _) in &bound {
        generic_substitution.insert(name.clone(), scope.variable(name));
    }
    let diagnostics_before = ctx.diagnostics().len();
    DEGRADED_FIRST.with(|first| first.set(None));
    DEGRADED_LAST.with(|last| last.set(None));
    let generic = resolve(ctx, resolving, &generic_substitution);
    let degraded = (
        DEGRADED_FIRST.with(Cell::get).unwrap_or("none"),
        DEGRADED_LAST.with(Cell::get).unwrap_or("none"),
    );
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    let mapping: Vec<(TypeParameterType, Type)> = bound
        .iter()
        .filter_map(|(name, ty)| match scope.variable(name) {
            Type::TypeParameter(variable) => Some((variable, ty.clone())),
            _ => None,
        })
        .collect();
    let shape = written_shape(node, kind_has_infer);
    let class = if mentions_type_variable(&generic.ty) {
        "deferred"
    } else if generic.ty.is_unmodelled() {
        "generic_unmodelled"
    } else {
        "concrete"
    };
    let outcome = if generic.had_error {
        "generic_degraded"
    } else {
        match generic_return::instantiate_type(&generic.ty, &mapping) {
            Err(generic_return::Unsupported::Deferred) => "undecided.deferred",
            Err(generic_return::Unsupported::Reference(_)) => "undecided.reference",
            Err(generic_return::Unsupported::Depth) => "undecided.depth",
            Ok(instantiated) => {
                let outcome = if generic_return::captures_variable(&instantiated, &mapping, 1) {
                    "captured"
                } else if instantiated == resolved.ty
                    && crate::speculative::display_type_fingerprint(&instantiated)
                        == crate::speculative::display_type_fingerprint(&resolved.ty)
                {
                    "agree"
                } else if generic_return::first_difference(&resolved.ty, &instantiated).is_none() {
                    if instantiated.name() == resolved.ty.name() { "agree_interning" } else { "display_differs" }
                } else {
                    "disagree"
                };
                if matches!(outcome, "disagree" | "display_differs" | "captured") || class == "generic_unmodelled" {
                    log(kind, class, outcome, ctx, &resolved.ty, &instantiated, &generic.ty, &shape, degraded);
                }
                outcome
            }
        }
    };
    drop(scope);
    IN_SHADOW.with(|flag| flag.set(false));
    count(label(kind, &format!("{class}.{outcome}")));
    if class == "generic_unmodelled" || outcome == "generic_degraded" {
        count(label(kind, &format!("degraded_last.{}", degraded.1)));
        count(label(kind, &format!("degraded_first.{}", degraded.0)));
        count(label(kind, &format!("degraded_shape.{shape}")));
    }
}

#[allow(clippy::too_many_arguments)]
fn log(
    kind: Kind,
    class: &str,
    outcome: &str,
    ctx: &CheckerContext,
    resolved: &Type,
    instantiated: &Type,
    generic: &Type,
    shape: &str,
    degraded: (&str, &str),
) {
    let Some(path) = std::env::var_os("SURGE_DEFERRED_SHADOW_LOG") else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let clip = |text: String| text.chars().take(300).collect::<String>();
    let _ = writeln!(
        file,
        "{} {class}.{outcome} shape={shape} first={} last={} file={} || resolved={} || instantiated={} || generic={} || DIFF {}",
        match kind {
            Kind::Conditional => "cond",
            Kind::Mapped => "mapped",
        },
        degraded.0,
        degraded.1,
        ctx.file_name,
        clip(resolved.name()),
        clip(instantiated.name()),
        clip(generic.name()),
        clip(
            generic_return::first_difference(resolved, instantiated)
                .or_else(|| generic_return::first_display_difference(resolved, instantiated))
                .unwrap_or_default()
        ),
    );
}
