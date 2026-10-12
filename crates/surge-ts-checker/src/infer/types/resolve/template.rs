use super::*;

use surge_ts_syntax::ParsedTemplateLiteralType;

/// Resolves a template literal type the way tsc's `getTemplateLiteralType`
/// does: literal placeholders fold into the text, a union distributes, and
/// `string`/`number`/`bigint` stay as pattern placeholders
/// (`` `/${string}` ``). A placeholder tsc has no pattern for — and one surge
/// could not resolve — makes the whole template `string`.
pub(crate) fn resolve_template_literal_type(
    template: ParsedTemplateLiteralType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let ParsedTemplateLiteralType {
        quasis,
        interpolations,
        span,
    } = template;

    let mut had_error = false;
    let mut types = Vec::with_capacity(interpolations.len());
    for interpolation in interpolations {
        let resolved = resolve_parsed_type(interpolation, ctx, resolving, substitution);
        had_error |= resolved.had_error;
        // A named placeholder (`${Protocol}`, an enum, a literal alias) is
        // judged by what it names; a nested template keeps its own pattern,
        // and an intersection of patterns each of its members.
        let placeholder = super::intersection::deferred_intersection_members(&resolved.ty)
            .and_then(|members| {
                surge_ts_types::template_placeholder_intersection(
                    members.iter().map(surge_ts_types::peel_to_pattern_literal).collect(),
                )
            })
            .unwrap_or_else(|| surge_ts_types::peel_to_pattern_literal(&resolved.ty));
        types.push(placeholder);
    }

    // `checkCrossProductUnion`: 100,000 or more combinations is an error, and
    // the template is tsc's error type.
    let cross_product = types.iter().fold(1usize, |product, ty| {
        product.saturating_mul(match ty {
            Type::Union(union) => union.types().len().max(1),
            Type::Boolean => 2,
            _ => 1,
        })
    });
    if cross_product >= 100_000 {
        if let Some(span) = span {
            ctx.push(Diagnostic::ts2590(ctx.file_name.clone()).with_span(convert_span(span)));
        }
        return ResolvedType {
            ty: Type::ErrorType,
            had_error,
        };
    }

    ResolvedType {
        ty: surge_ts_types::template_literal_type(&quasis, &types),
        had_error,
    }
}
