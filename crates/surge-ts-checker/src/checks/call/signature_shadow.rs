//! `SURGE_SIGNATURE_SHADOW=1` (with `SURGE_LAZY_PROBE=1`) sizes reading a
//! generic signature once. At each call that resolves a written signature
//! again under its inferred bindings, the signature is also resolved with its
//! own type parameters as type variables, and that form is instantiated at the
//! type level with the same bindings. The counts say how often the two agree;
//! `SURGE_SIGNATURE_SHADOW_LOG=<path>` appends each disagreement.

use std::io::Write;

use surge_ts_types::{FunctionType, Type, TypeCopyReason, TypeParameterType};

use crate::checks::function::generic_return;
use crate::context::CheckerContext;
use crate::infer::TypeParameterSubstitution;
use crate::symbols::FunctionSignatureInfo;

pub(super) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        surge_ts_types::lazy_probe::enabled() && std::env::var_os("SURGE_SIGNATURE_SHADOW").is_some()
    })
}

thread_local! {
    static IN_SHADOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn label(component: &str, outcome: &str) -> &'static str {
    thread_local! {
        static LABELS: std::cell::RefCell<std::collections::HashMap<String, &'static str>> = Default::default();
    }
    let name = format!("sigshadow.{component}.{outcome}");
    LABELS.with(|labels| *labels.borrow_mut().entry(name.clone()).or_insert_with(|| Box::leak(name.into_boxed_str())))
}

/// Compares the call's resolution of `signature` under `substitution` with
/// the signature read over its own type variables and instantiated with the
/// same bindings.
pub(super) fn compare(
    signature: &FunctionSignatureInfo,
    substitution: &TypeParameterSubstitution,
    instantiated: &FunctionType,
    ctx: &mut CheckerContext,
) {
    use surge_ts_types::lazy_probe::count;
    if IN_SHADOW.with(std::cell::Cell::get) {
        return;
    }
    count("sigshadow.calls");
    if signature.type_parameters.is_empty() {
        count("sigshadow.skip.not_generic");
        return;
    }
    let mut bindings = Vec::with_capacity(signature.type_parameters.len());
    for parameter in &signature.type_parameters {
        match substitution.get(&parameter.name) {
            Some(ty)
                if !substitution.is_placeholder(&parameter.name)
                    && !substitution.is_degraded(&parameter.name)
                    && !ty.is_unknown()
                    && !surge_ts_types::type_variable::mentions_type_variable(ty) =>
            {
                bindings.push((parameter.name.clone(), ty.clone()))
            }
            _ => {
                count("sigshadow.skip.unbound");
                return;
            }
        }
    }
    let Some(variables) = crate::infer::types::SignatureVariables::enter(&signature.type_parameters, ctx) else {
        count("sigshadow.skip.no_spans");
        return;
    };
    if !variables.entered_scope() {
        count("sigshadow.skip.already_bound");
        return;
    }
    let mut generic_substitution = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    let mut mapping: Vec<(TypeParameterType, Type)> = Vec::with_capacity(bindings.len());
    for (name, ty) in &bindings {
        let Some(Type::TypeParameter(variable)) = variables.variable(name) else {
            return;
        };
        generic_substitution.insert(name.clone(), Type::TypeParameter(variable.clone()));
        mapping.push((variable, ty.clone()));
    }
    IN_SHADOW.with(|flag| flag.set(true));
    let diagnostics_before = ctx.diagnostics().len();
    for (index, real) in instantiated.parameters().iter().enumerate() {
        let Some(Some(written)) = signature.parameter_types.get(index) else {
            continue;
        };
        let generic = crate::infer::map_parsed_type_with_substitution(written.clone(), ctx, &generic_substitution);
        classify("param", &generic, real, &mapping, ctx);
    }
    if let Some(written) = signature.return_type.as_ref() {
        let generic = crate::infer::types::with_signature_array_constraints(&signature.type_parameters, || {
            crate::infer::map_parsed_type_with_substitution(written.clone(), ctx, &generic_substitution)
        });
        classify("return", &generic, instantiated.return_type(), &mapping, ctx);
    }
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    IN_SHADOW.with(|flag| flag.set(false));
    drop(variables);
}

fn classify(
    component: &'static str,
    generic: &Type,
    real: &Type,
    mapping: &[(TypeParameterType, Type)],
    ctx: &CheckerContext,
) {
    use surge_ts_types::lazy_probe::count;
    let outcome = if crate::infer::types::holds_sentinel(real, &mut 2_000) {
        // Where the call's own read degraded, whether the generic read
        // instantiates to a type that holds no sentinel.
        let clean = !generic.is_unmodelled()
            && !crate::infer::types::holds_sentinel(generic, &mut 2_000)
            && generic_return::instantiate_type(generic, mapping)
                .is_ok_and(|instantiated| !crate::infer::types::holds_sentinel(&instantiated, &mut 2_000));
        if clean { "real_degraded.generic_clean" } else { "real_degraded" }
    } else if generic.is_unmodelled() || crate::infer::types::holds_sentinel(generic, &mut 2_000) {
        "generic_degraded"
    } else {
        match generic_return::instantiate_type(generic, mapping) {
            Err(generic_return::Unsupported::Deferred) => "undecided.deferred",
            Err(generic_return::Unsupported::Reference(_)) => "undecided.reference",
            Err(generic_return::Unsupported::Depth) => "undecided.depth",
            Ok(instantiated) => {
                let outcome = if generic_return::captures_variable(&instantiated, mapping, 1) {
                    "captured"
                } else if instantiated == *real
                    && crate::speculative::display_type_fingerprint(&instantiated)
                        == crate::speculative::display_type_fingerprint(real)
                {
                    "agree"
                } else {
                    match same_structure(real, &instantiated, 0, &mut 50_000) {
                        Some(true) if instantiated.name() == real.name() => "agree_interning",
                        Some(true) => "display_differs",
                        Some(false) => "disagree",
                        None => "undecided.budget",
                    }
                };
                if matches!(outcome, "disagree" | "display_differs" | "captured") {
                    log(component, outcome, ctx, real, &instantiated, generic);
                }
                outcome
            }
        }
    };
    count(label(component, outcome));
}

/// Whether `real` and `instantiated` are the same type up to interning and
/// display: a reference either side kept lazy where the other expanded it is
/// compared through its expansion, at any depth, and a union's members match
/// in any order. `None` when the walk runs out of budget.
fn same_structure(real: &Type, instantiated: &Type, depth: usize, budget: &mut usize) -> Option<bool> {
    if real == instantiated {
        return Some(true);
    }
    if *budget == 0 || depth > 48 {
        return None;
    }
    *budget -= 1;
    let mut nested = |real: &Type, instantiated: &Type| same_structure(real, instantiated, depth + 1, budget);
    match (real, instantiated) {
        (Type::Reference(left), Type::Reference(right)) if left.id == right.id && left.arguments.len() == right.arguments.len() => {
            for (left, right) in left.arguments.iter().zip(right.arguments.iter()) {
                if !nested(left, right)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        // The instantiation keeps an alias or conditional reference lazy where
        // the call resolved it, so its side expands first.
        (_, Type::Reference(reference)) if expands(reference) => nested(real, &reference.resolve()),
        (Type::Reference(reference), _) if expands(reference) => nested(&reference.resolve(), instantiated),
        (Type::Tuple(left), Type::Tuple(right)) => {
            if left.len() != right.len() {
                return Some(false);
            }
            for (left, right) in left.iter().zip(right) {
                if !nested(left, right)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        (Type::OpenTuple(left), Type::OpenTuple(right)) => {
            if left.leading.len() != right.leading.len() || left.trailing.len() != right.trailing.len() {
                return Some(false);
            }
            for (left, right) in left
                .leading
                .iter()
                .zip(&right.leading)
                .chain(std::iter::once((&*left.rest, &*right.rest)))
                .chain(left.trailing.iter().zip(&right.trailing))
            {
                if !nested(left, right)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        (Type::Array(left), Type::Array(right)) => nested(left, right),
        (Type::Union(left), Type::Union(right)) => {
            let (left, right) = (left.types(), right.types());
            if left.len() != right.len() {
                return Some(false);
            }
            let mut matched = vec![false; right.len()];
            'members: for member in left.iter() {
                for (index, candidate) in right.iter().enumerate() {
                    if !matched[index] && nested(member, candidate)? {
                        matched[index] = true;
                        continue 'members;
                    }
                }
                return Some(false);
            }
            Some(true)
        }
        (Type::Function(left), Type::Function(right)) => same_function(left, right, &mut nested),
        (Type::Object(left), Type::Object(right)) => {
            if left.properties.len() != right.properties.len()
                || left.string_index_readonly != right.string_index_readonly
                || left.number_index_readonly != right.number_index_readonly
                || left.non_primitive != right.non_primitive
                || left.is_intersection != right.is_intersection
            {
                return Some(false);
            }
            for (name, property) in left.properties.iter() {
                let Some(other) = right.properties.get(name) else {
                    return Some(false);
                };
                if property.optional != other.optional || property.readonly != other.readonly || property.method != other.method {
                    return Some(false);
                }
                if !nested(&property.ty, &other.ty)? {
                    return Some(false);
                }
            }
            for (left, right) in [
                (left.string_index_type.as_deref(), right.string_index_type.as_deref()),
                (left.number_index_type.as_deref(), right.number_index_type.as_deref()),
            ] {
                match (left, right) {
                    (Some(left), Some(right)) => {
                        if !nested(left, right)? {
                            return Some(false);
                        }
                    }
                    (None, None) => {}
                    _ => return Some(false),
                }
            }
            for (left, right) in [
                (left.call_signature.as_deref(), right.call_signature.as_deref()),
                (left.construct_signature.as_deref(), right.construct_signature.as_deref()),
            ] {
                match (left, right) {
                    (Some(left), Some(right)) => {
                        if !same_function(left, right, &mut nested)? {
                            return Some(false);
                        }
                    }
                    (None, None) => {}
                    _ => return Some(false),
                }
            }
            match (left.intersection_operands.as_deref(), right.intersection_operands.as_deref()) {
                (Some(left), Some(right)) if left.len() == right.len() => {
                    for (left, right) in left.iter().zip(right) {
                        if !nested(left, right)? {
                            return Some(false);
                        }
                    }
                    Some(true)
                }
                (None, None) => Some(true),
                _ => Some(false),
            }
        }
        _ => Some(false),
    }
}

fn expands(reference: &surge_ts_types::TypeReference) -> bool {
    !matches!(reference.resolve(), Type::Reference(again) if again.id == reference.id)
}

fn same_function(
    left: &FunctionType,
    right: &FunctionType,
    nested: &mut impl FnMut(&Type, &Type) -> Option<bool>,
) -> Option<bool> {
    let (flags_left, flags_right) = (left.payload(), right.payload());
    if left.parameters().len() != right.parameters().len()
        || flags_left.is_variadic != flags_right.is_variadic
        || flags_left.required_parameter_count != flags_right.required_parameter_count
        || left.this_type().is_some() != right.this_type().is_some()
    {
        return Some(false);
    }
    for (left, right) in left.parameters().iter().zip(right.parameters()) {
        if !nested(left, right)? {
            return Some(false);
        }
    }
    if let (Some(left), Some(right)) = (left.this_type(), right.this_type())
        && !nested(left, right)?
    {
        return Some(false);
    }
    nested(left.return_type(), right.return_type())
}

fn log(component: &str, outcome: &str, ctx: &CheckerContext, real: &Type, instantiated: &Type, generic: &Type) {
    let Some(path) = std::env::var_os("SURGE_SIGNATURE_SHADOW_LOG") else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let clip = |text: String| text.chars().take(300).collect::<String>();
    let _ = writeln!(
        file,
        "{component} {outcome} file={} || real={} || instantiated={} || generic={} || DIFF {}",
        ctx.file_name,
        clip(real.name()),
        clip(instantiated.name()),
        clip(generic.name()),
        clip(
            generic_return::first_difference(real, instantiated)
                .or_else(|| generic_return::first_display_difference(real, instantiated))
                .unwrap_or_default()
        ),
    );
}
