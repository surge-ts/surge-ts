//! tsc's `checkIndexConstraints` (checker.go): every property of a type with
//! an index signature must be assignable to each index signature that applies
//! to its name (TS2411), and a number index to the string index (TS2413).
//!
//! The diagnostic is anchored where the conflict is declared: on the property
//! when this declaration declares it, else on the index signature when this
//! declaration declares that, else — both inherited, from bases none of which
//! holds both — on the interface name.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedInterfaceDeclaration, ParsedNamedType, ParsedObjectType, ParsedType, ParsedTypeParameter,
    TextSpan,
};
use surge_ts_types::{ObjectType, Type, is_assignable_to, union_type};

use crate::context::{CheckerContext, convert_span};
use crate::infer::map_parsed_type;

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexKind {
    String,
    Number,
    Symbol,
}

impl IndexKind {
    fn key_name(self) -> &'static str {
        match self {
            IndexKind::String => "string",
            IndexKind::Number => "number",
            IndexKind::Symbol => "symbol",
        }
    }

    fn value_of(self, object: &ObjectType) -> Option<Type> {
        let value = match self {
            IndexKind::String if !object.synthetic_open_index => object.string_index_type.as_deref(),
            IndexKind::String => None,
            IndexKind::Number => object.number_index_type.as_deref(),
            IndexKind::Symbol => object.symbol_index_type(),
        };
        value.cloned()
    }

    /// `getApplicableIndexInfos`: a string index constrains every property, a
    /// number index only the numerically named ones.
    fn applies_to(self, name: &str) -> bool {
        match self {
            // A computed name surge keeps as written (`[mySymbol]`) may be a
            // unique symbol, which no string index constrains.
            IndexKind::String => !name.starts_with('['),
            IndexKind::Number => is_numeric_literal_name(name),
            IndexKind::Symbol => name.starts_with("[Symbol."),
        }
    }

    /// The value of the index of this kind constraining `name`: keyed string
    /// indexes constrain only the names their keys cover.
    fn constraint_for(self, object: &ObjectType, value: &Type, name: &str) -> Option<Type> {
        match object.active_keyed_indexes() {
            Some(keyed) if self == IndexKind::String => {
                keyed.read(&Type::StringLiteral(name.to_string()))
            }
            _ => self.applies_to(name).then(|| value.clone()),
        }
    }
}

/// tsc's `isNumericLiteralName`: the name reads back unchanged through a
/// number (`ToString(ToNumber(name)) == name`). JavaScript writes a number
/// outside `[1e-6, 1e21)` with an exponent, which no such name can match.
pub(crate) fn is_numeric_literal_name(name: &str) -> bool {
    if matches!(name, "Infinity" | "-Infinity" | "NaN") {
        return true;
    }
    let Ok(value) = name.parse::<f64>() else {
        return false;
    };
    if value == 0.0 {
        return name == "0";
    }
    value.is_finite() && (1e-6..1e21).contains(&value.abs()) && format!("{value}") == name
}

/// What a declaration contributes to the check: which members it declares
/// itself, where its own index signatures are, and the name to fall back on.
pub(crate) struct IndexConstraintDeclaration<'a> {
    pub(crate) name: &'a str,
    pub(crate) name_span: Option<TextSpan>,
    pub(crate) type_parameters: &'a [ParsedTypeParameter],
    pub(crate) own_members: Vec<(&'a str, Option<TextSpan>)>,
    pub(crate) string_index_span: Option<TextSpan>,
    pub(crate) number_index_span: Option<TextSpan>,
    pub(crate) bases: &'a [ParsedNamedType],
    /// Only an interface falls back on its name (`ObjectFlagsInterface`).
    pub(crate) is_interface: bool,
    /// A class's `symbol` index signature, which its instance type does not
    /// carry: keyed indexes keep a type out of the store's fingerprinting.
    pub(crate) symbol_index: Option<&'a ParsedType>,
}

impl IndexConstraintDeclaration<'_> {
    fn own_index_span(&self, kind: IndexKind) -> Option<TextSpan> {
        match kind {
            IndexKind::String => self.string_index_span,
            IndexKind::Number => self.number_index_span,
            IndexKind::Symbol => None,
        }
    }
}

/// tsc checks a merged interface once, over the members of every fragment
/// (`checkInterfaceDeclaration`), and reports a property on its first
/// declaration (`checkIndexConstraintForProperty`, `prop.ValueDeclaration`).
/// Checked here fragment by fragment: one without an index signature of its
/// own still declares members another fragment's signature constrains, and a
/// member an earlier fragment already declared is that fragment's to report.
pub(crate) fn check_interface_index_constraints(
    interface: &ParsedInterfaceDeclaration,
    ctx: &mut CheckerContext,
) {
    let merged = match ctx.lookup_type_declaration(&interface.name) {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info)) => Some(info.body.clone()),
        _ => None,
    };
    let merged_has_index = merged
        .as_ref()
        .is_some_and(|body| body.string_index_type.is_some() || body.number_index_type.is_some());
    if interface.string_index_type.is_none()
        && interface.number_index_type.is_none()
        && interface.keyed_indexes.is_none()
        && interface.extends.is_empty()
        && !merged_has_index
    {
        return;
    }
    let declaration_start = interface.name_span.map_or(0, |span| span.start);
    let first_declared_here = |name: &str| {
        merged.as_ref().is_none_or(|body| {
            body.members
                .iter()
                .zip(body.member_fragments.iter())
                .find(|(member, _)| member.name == name)
                .is_none_or(|(_, fragment)| {
                    fragment.declaration_start == declaration_start && *fragment.file_name == *ctx.file_name
                })
        })
    };
    let declaration = IndexConstraintDeclaration {
        name: &interface.name,
        name_span: interface.name_span,
        type_parameters: &interface.type_parameters,
        own_members: interface
            .members
            .iter()
            .filter(|member| first_declared_here(&member.name))
            .map(|member| (member.name.as_str(), member.name_span))
            .collect(),
        string_index_span: interface.string_index_span,
        number_index_span: interface.number_index_span,
        bases: &interface.extends,
        is_interface: true,
        symbol_index: None,
    };
    check_index_constraints(&declaration, ctx);
}

pub(crate) fn check_class_index_constraints(
    class: &surge_ts_syntax::ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    if class.string_index_type.is_none()
        && class.number_index_type.is_none()
        && class.keyed_indexes.is_none()
        && class.extends.is_empty()
    {
        return;
    }
    let members = super::heritage::class_instance_members(class);
    let declaration = IndexConstraintDeclaration {
        name: &class.name,
        name_span: class.name_span,
        type_parameters: &class.type_parameters,
        own_members: members
            .iter()
            .map(|member| (member.name.as_str(), member.name_span))
            .collect(),
        string_index_span: class.string_index_span,
        number_index_span: class.number_index_span,
        bases: &class.extends,
        is_interface: false,
        symbol_index: class.keyed_indexes.as_deref().and_then(|keyed| keyed.symbol.as_ref()),
    };
    check_index_constraints(&declaration, ctx);
}

/// `checkIndexConstraints(staticType, symbol, true)`: a class's own static
/// members against its static index signatures, `prototype` excepted.
pub(crate) fn check_class_static_index_constraints(
    class: &surge_ts_syntax::ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    use surge_ts_syntax::ParsedClassMember;
    let checkpoint = ctx.diagnostics().len();
    let indexes: Vec<(IndexKind, Type)> = [
        (IndexKind::String, class.static_string_index_type.as_ref()),
        (IndexKind::Number, class.static_number_index_type.as_ref()),
    ]
    .into_iter()
    .filter_map(|(kind, ty)| Some((kind, map_parsed_type(ty?.clone(), ctx))))
    .filter(|(_, value)| !crate::checks::function::type_contains_degradation(value))
    .collect();
    ctx.truncate_diagnostics(checkpoint);
    if indexes.is_empty() {
        return;
    }
    // `checkIndexConstraintForIndexSignature` on the static side: its own
    // number index against its own string index, reported on the former.
    if let [(IndexKind::String, string_value), (IndexKind::Number, number_value)] = indexes.as_slice()
        && let Some(span) = class.static_number_index_span
        && !is_assignable_to(number_value, string_value)
    {
        let diagnostic =
            Diagnostic::ts2413("number", number_value.name(), "string", string_value.name(), ctx.file_name.clone());
        ctx.push(diagnostic.with_span(convert_span(span)));
    }
    let Some(Type::Object(statics)) = ctx.symbols.get(&class.name).map(|symbol| symbol.ty.peeled()) else {
        return;
    };
    for member in &class.members {
        let (name, name_span) = match member {
            ParsedClassMember::Property(property) if property.is_static => (&property.name, property.name_span),
            ParsedClassMember::Method(method) if method.is_static => (&method.name, method.name_span),
            _ => continue,
        };
        // A private name has no key an index signature could constrain
        // (`getLiteralTypeFromPropertyName` is `never` for it).
        if surge_ts_types::private_name::is_private_name_key(name) {
            continue;
        }
        let Some(property) = statics.properties.get(name.as_str()) else {
            continue;
        };
        // A method whose inferred return degraded is still a function, which no
        // primitive index type admits.
        let degraded = crate::checks::function::type_contains_degradation(&property.ty);
        if degraded && !matches!(property.ty.peeled(), Type::Function(_)) {
            continue;
        }
        for (kind, value) in &indexes {
            if degraded && !matches!(value.peeled(), Type::String | Type::Number | Type::Boolean | Type::BigInt) {
                continue;
            }
            if kind.applies_to(name) && !is_assignable_to(&property.ty, value) {
                let diagnostic = Diagnostic::ts2411(
                    name.as_str(),
                    property.ty.name(),
                    kind.key_name(),
                    value.name(),
                    ctx.file_name.clone(),
                );
                ctx.push(match name_span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                });
            }
        }
    }
}

/// `checkTypeLiteral`'s `checkIndexConstraints`, for a type literal written as
/// a variable's annotation and already resolved to `resolved`. Every member of
/// a type literal is its own (`getParentOfSymbol(prop) == t.symbol`), so a
/// property that conflicts with an index signature applying to its name is
/// reported on the property.
pub(crate) fn check_type_literal_index_constraints(
    literal: &ParsedObjectType,
    resolved: &Type,
    ctx: &mut CheckerContext,
) {
    if literal.string_index_type.is_none() && literal.number_index_type.is_none() && literal.keyed_indexes.is_none() {
        return;
    }
    let Type::Object(object) = resolved.peeled() else {
        return;
    };
    let indexes: Vec<(IndexKind, Type)> = [IndexKind::String, IndexKind::Number, IndexKind::Symbol]
        .into_iter()
        .filter_map(|kind| Some((kind, kind.value_of(&object)?)))
        .filter(|(_, value)| !crate::checks::function::type_contains_degradation(value))
        .collect();
    if indexes.is_empty() {
        return;
    }
    for member in &literal.properties {
        // A computed key (`[Symbol.iterator]`) carries no span; it names no
        // string or number the index signatures above apply to.
        let Some(span) = member.name_span else {
            continue;
        };
        let Some(property) = object.properties.get(member.name.as_str()) else {
            continue;
        };
        // As for an interface: only a member surge could not model at all is
        // skipped. A library type (`String`) carries degraded members deep
        // inside while its relation to the index type is still decided.
        if property.ty.is_unmodelled() {
            continue;
        }
        let property_type = if property.optional
            && surge_ts_types::strict_null_checks()
            && !surge_ts_types::exact_optional_property_types()
        {
            union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        };
        for (kind, value) in &indexes {
            let Some(value) = &kind.constraint_for(&object, value, &member.name) else {
                continue;
            };
            if !is_assignable_to(&property_type, value) {
                let diagnostic = Diagnostic::ts2411(
                    member.name.as_str(),
                    property_type.name(),
                    kind.key_name(),
                    value.name(),
                    ctx.file_name.clone(),
                );
                ctx.push(diagnostic.with_span(convert_span(span)));
            }
        }
    }
}

pub(crate) fn check_index_constraints(declaration: &IndexConstraintDeclaration<'_>, ctx: &mut CheckerContext) {
    // A generic declaration is checked as itself instantiated over its own
    // type parameters, bound as type variables for the duration.
    let _type_variables =
        crate::checks::function::enter_body_type_variables(declaration.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(declaration.type_parameters, ctx, |ctx| {
        check_index_constraints_in_scope(declaration, ctx)
    });
}

fn check_index_constraints_in_scope(declaration: &IndexConstraintDeclaration<'_>, ctx: &mut CheckerContext) {
    let checkpoint = ctx.diagnostics().len();
    let own_type = map_parsed_type(
        named(declaration.name, declaration.name_span, declaration.type_parameters),
        ctx,
    );
    ctx.truncate_diagnostics(checkpoint);
    let Type::Object(object) = own_type.peeled() else {
        return;
    };
    let mut indexes: Vec<(IndexKind, Type)> = [IndexKind::String, IndexKind::Number, IndexKind::Symbol]
        .into_iter()
        .filter_map(|kind| Some((kind, kind.value_of(&object)?)))
        .filter(|(_, value)| !value.is_unmodelled())
        .collect();
    if let Some(symbol_index) = declaration.symbol_index
        && !indexes.iter().any(|(kind, _)| *kind == IndexKind::Symbol)
    {
        let checkpoint = ctx.diagnostics().len();
        let value = map_parsed_type(symbol_index.clone(), ctx);
        ctx.truncate_diagnostics(checkpoint);
        if !value.is_unmodelled() {
            indexes.push((IndexKind::Symbol, value));
        }
    }
    if indexes.is_empty() {
        return;
    }
    let checkpoint = ctx.diagnostics().len();
    let bases: Vec<Type> = declaration
        .bases
        .iter()
        .map(|base| map_parsed_type(ParsedType::Named(std::sync::Arc::new(base.clone())), ctx))
        .collect();
    ctx.truncate_diagnostics(checkpoint);

    let mut reports = Vec::new();
    for (name, property) in object.properties.iter() {
        if property.index_slot
            || name.starts_with('\u{0}')
            || surge_ts_types::private_name::is_private_name_key(name)
        {
            continue;
        }
        let own_member = declaration
            .own_members
            .iter()
            .find(|(member, _)| **member == **name)
            .map(|(_, span)| *span);
        let property_type = if property.optional
            && surge_ts_types::strict_null_checks()
            && !surge_ts_types::exact_optional_property_types()
        {
            union_type(vec![property.ty.clone(), Type::Undefined])
        } else {
            property.ty.clone()
        };
        if property.ty.is_unmodelled() {
            continue;
        }
        for (kind, value) in &indexes {
            let Some(value) = &kind.constraint_for(&object, value, name) else {
                continue;
            };
            let Some(span) = error_span(declaration, own_member, *kind, &bases, |base| {
                matches!(base.peeled(), Type::Object(object) if object.properties.contains_key(name.as_ref()))
            }) else {
                continue;
            };
            if !is_assignable_to(&property_type, value) {
                reports.push((
                    span,
                    Diagnostic::ts2411(
                        name.as_ref(),
                        property_type.name(),
                        kind.key_name(),
                        value.name(),
                        ctx.file_name.clone(),
                    ),
                ));
            }
        }
    }

    // `checkIndexConstraintForIndexSignature`: the number index against the
    // string one, when both apply.
    if let [(IndexKind::String, string_value), (IndexKind::Number, number_value), ..] = indexes.as_slice()
        && object.active_keyed_indexes().is_none()
        && let Some(span) = error_span(
            declaration,
            declaration.own_index_span(IndexKind::Number).map(Some),
            IndexKind::String,
            &bases,
            |base| index_of(base, IndexKind::Number).is_some(),
        )
        && !is_assignable_to(number_value, string_value)
    {
        reports.push((
            span,
            Diagnostic::ts2413("number", number_value.name(), "string", string_value.name(), ctx.file_name.clone()),
        ));
    }

    for (span, diagnostic) in reports {
        ctx.push(diagnostic.with_span(convert_span(span)));
    }
}

/// Where a conflict between a member (a property, or the number index) and an
/// applicable index signature is reported: on the member when this
/// declaration declares it, else on the index signature when this declaration
/// declares that, else — an interface whose bases bring them separately — on
/// its name. `None` when some base holds both (it reports the conflict), and
/// when either comes from another fragment of a merged declaration rather
/// than from a base: tsc anchors that on the other fragment, which is checked
/// on its own or lives in a library.
fn error_span(
    declaration: &IndexConstraintDeclaration<'_>,
    own_member: Option<Option<TextSpan>>,
    kind: IndexKind,
    bases: &[Type],
    base_has_member: impl Fn(&Type) -> bool,
) -> Option<TextSpan> {
    if let Some(member_span) = own_member {
        return member_span;
    }
    if !bases.iter().any(&base_has_member) {
        return None;
    }
    if let Some(index_span) = declaration.own_index_span(kind) {
        return Some(index_span);
    }
    if !bases.iter().any(|base| index_of(base, kind).is_some()) {
        return None;
    }
    let some_base_holds_both = bases
        .iter()
        .any(|base| base_has_member(base) && index_of(base, kind).is_some());
    if !declaration.is_interface || some_base_holds_both {
        return None;
    }
    declaration.name_span
}

fn index_of(ty: &Type, kind: IndexKind) -> Option<Type> {
    match ty.peeled() {
        Type::Object(object) => kind.value_of(&object),
        _ => None,
    }
}

fn named(name: &str, span: Option<TextSpan>, type_parameters: &[ParsedTypeParameter]) -> ParsedType {
    ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span,
        type_arguments: type_parameters
            .iter()
            .map(|parameter| named(&parameter.name, None, &[]))
            .collect(),
    }))
}
