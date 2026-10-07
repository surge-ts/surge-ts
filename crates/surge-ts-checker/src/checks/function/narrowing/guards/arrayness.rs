use surge_ts_syntax::ParsedExpression;
use surge_ts_types::{Type, TypeCopyReason, union_type};

use crate::symbols::{SymbolInfo, SymbolTable};

/// Parses an `Array.isArray(x)` guard, returning the argument expression.
pub(crate) fn parse_array_isarray_condition(
    condition: &ParsedExpression,
) -> Option<&ParsedExpression> {
    let ParsedExpression::PropertyCall {
        object,
        property_name,
        arguments,
        ..
    } = condition
    else {
        return None;
    };
    let ParsedExpression::Identifier { name, .. } = object.as_ref() else {
        return None;
    };
    if name != "Array" || property_name != "isArray" || arguments.len() != 1 {
        return None;
    }
    Some(&arguments[0].expression)
}

/// Narrows a union by `Array.isArray(x)`. `keep_arrays` keeps the array/tuple
/// members (the `=== true` branch); otherwise removes them. `any`/`unknown`
/// members are kept either way.
pub(crate) fn narrow_union_by_arrayness(ty: &Type, keep_arrays: bool) -> Option<Type> {
    // `arg is any[]` over a type variable, bare or already narrowed, is
    // `getNarrowedType`'s `T & any[]`: it keeps its identity and reads as an array.
    if keep_arrays && (ty.is_type_variable() || surge_ts_types::type_variable::is_narrowed_type_variable(ty)) {
        let mut operands = match ty {
            Type::Object(object) => object.intersection_operands.as_deref().unwrap_or_default().to_vec(),
            other => vec![other.clone()],
        };
        operands.push(Type::Array(Box::new(Type::Any)));
        return Some(surge_ts_types::type_variable::type_variable_intersection(operands));
    }
    if keep_arrays && supertype_of_any_array(ty) {
        return Some(Type::Array(Box::new(Type::Any)));
    }
    let Type::Union(union) = ty else {
        return None;
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .map(|member| {
            if keep_arrays && supertype_of_any_array(member) {
                Type::Array(Box::new(Type::Any))
            } else {
                member.clone()
            }
        })
        .filter(|member| match member {
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => true,
            Type::Array(_) | Type::Tuple(_) => keep_arrays,
            // `Array<T>` / `ReadonlyArray<T>` written in generic form stays a
            // nominal reference rather than `Type::Array`, so match by name too.
            Type::Reference(reference)
                if matches!(
                    reference.id.split('\u{0}').next_back(),
                    Some("Array" | "ReadonlyArray")
                ) =>
            {
                keep_arrays
            }
            _ => !keep_arrays,
        })
        .collect();

    if kept.is_empty() {
        // No member can be an array, so the true branch is unreachable. tsc
        // narrows to the predicate's own type (`Array.isArray(arg: any): arg is
        // any[]`) rather than leaving the union alone, which is why reading
        // `.map` there is legal in tsc and was a false TS2339 here.
        return keep_arrays.then(|| Type::Array(Box::new(Type::Any)));
    }
    if kept.as_slice() == union.types() {
        return None;
    }
    Some(union_type(kept))
}

/// `unknown`, `{}` or `object`: `getNarrowedType` against `any[]` is the array
/// itself, which those all admit.
fn supertype_of_any_array(ty: &Type) -> bool {
    match ty {
        Type::GenuineUnknown => true,
        Type::Object(object) => {
            object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !object.is_intersection
        }
        _ => false,
    }
}

/// Builds a symbol table narrowed by an `Array.isArray(x)` guard for the branch.
pub(crate) fn narrow_array_isarray_symbol_table(
    condition: &ParsedExpression,
    symbols: &SymbolTable,
    branch_is_true: bool,
) -> Option<SymbolTable> {
    let operand = parse_array_isarray_condition(condition)?;
    let ParsedExpression::Identifier { name, .. } = operand else {
        return None;
    };
    let symbol = symbols.get(name)?;
    let narrowed = narrow_union_by_arrayness(&symbol.ty, branch_is_true)?;
    let mut narrowed_symbols = symbols.clone_with_reason(TypeCopyReason::ScopeOrContext);
    narrowed_symbols.insert_narrowed(
        name.clone(),
        SymbolInfo {
            ty: narrowed,
            kind: symbol.kind,
            function_signature: symbol.function_signature.clone(),
        },
        symbol.ty.clone(),
    );
    Some(narrowed_symbols)
}

/// Concrete `ArrayBufferView` types: the typed arrays and `DataView`. The
/// `ArrayBuffer.isView(x)` predicate is `x is ArrayBufferView`, and a union may
/// carry either the `ArrayBufferView` interface itself or a concrete view.
pub(super) const ARRAY_BUFFER_VIEW_NAMES: &[&str] = &[
    "ArrayBufferView",
    "DataView",
    "Int8Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
];

/// Whether a union member is an `ArrayBufferView` (matched nominally on the base
/// name, ignoring any generic arguments, e.g. `ArrayBufferView<ArrayBuffer>`).
pub(super) fn is_array_buffer_view_type(member: &Type) -> bool {
    let name = member.name();
    let base = name.split('<').next().unwrap_or(name.as_str());
    ARRAY_BUFFER_VIEW_NAMES.contains(&base)
}

/// Parses an `ArrayBuffer.isView(x)` guard, returning the argument expression.
pub(crate) fn parse_arraybuffer_isview_condition(
    condition: &ParsedExpression,
) -> Option<&ParsedExpression> {
    let ParsedExpression::PropertyCall {
        object,
        property_name,
        arguments,
        ..
    } = condition
    else {
        return None;
    };
    let ParsedExpression::Identifier { name, .. } = object.as_ref() else {
        return None;
    };
    if name != "ArrayBuffer" || property_name != "isView" || arguments.len() != 1 {
        return None;
    }
    Some(&arguments[0].expression)
}

/// Narrows a union by `ArrayBuffer.isView(x)`. `keep_views` keeps the
/// `ArrayBufferView` members (the `=== true` branch); otherwise removes them.
/// `any`/`unknown` members are kept either way.
pub(crate) fn narrow_union_by_arraybufferview(ty: &Type, keep_views: bool) -> Option<Type> {
    let Type::Union(union) = ty else {
        return None;
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| match member {
            Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => true,
            _ => is_array_buffer_view_type(member) == keep_views,
        })
        .cloned()
        .collect();

    if kept.is_empty() || kept.len() == union.types().len() {
        return None;
    }
    Some(union_type(kept))
}
