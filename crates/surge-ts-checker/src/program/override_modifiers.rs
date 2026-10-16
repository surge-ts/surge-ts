//! tsc's `checkMembersForOverrideModifier` (`checker.go`): an `override`
//! modifier needs a base class declaring the member, and under
//! `noImplicitOverride` a member the base declares needs one. A JavaScript
//! file says the same with an `@override` tag, and its messages say so.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    EXPRESSION_HERITAGE_BASE, ParsedBindingName, ParsedClassDeclaration, ParsedClassMember,
    ParsedNamedType, TextSpan,
};
use surge_ts_types::Type;

use crate::context::{CheckerContext, convert_span};
use crate::infer::{InferredExpression, infer_expression, map_parsed_type};
use crate::symbols::TypeDeclarationInfo;

struct Member<'a> {
    name: &'a str,
    name_span: Option<TextSpan>,
    is_static: bool,
    has_override: bool,
    is_abstract: bool,
    is_parameter: bool,
}

/// Whether a type answers a property name (`getPropertyOfType`), as far as
/// surge modelled it.
#[derive(PartialEq, Eq)]
enum Presence {
    Present,
    Absent,
    Unknown,
}

pub(crate) fn check_members_for_override_modifier(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    let members = override_members(class);
    let no_implicit_override = ctx.options.no_implicit_override;
    if !members.iter().any(|member| member.has_override) && !no_implicit_override {
        return;
    }
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    let Some(base) = class.extends.first() else {
        let own_display = super::heritage::type_display(
            &class.name,
            &class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
        );
        for member in members.iter().filter(|member| member.has_override) {
            let diagnostic = if javascript {
                Diagnostic::ts4121(&own_display, ctx.file_name.clone())
            } else {
                Diagnostic::ts4112(&own_display, ctx.file_name.clone())
            };
            push_at(diagnostic, member.name_span, ctx);
        }
        return;
    };
    if class.type_parameters.is_empty() {
        check_against_base(class, base, &members, javascript, ctx);
    } else {
        let type_parameters = super::classes::merged_type_parameters(class, ctx);
        let _type_variables = crate::checks::function::enter_body_type_variables(&type_parameters, ctx);
        crate::checks::function::with_type_parameter_scope(&type_parameters, ctx, |ctx| {
            check_against_base(class, base, &members, javascript, ctx)
        });
    }
}

/// The instance type an `extends` expression constructs: a call's single
/// construct signature, or a class expression's own instance. Neither has
/// a static side surge can name, nor an `abstract` member.
fn expression_base_instance(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) -> Option<Type> {
    let constructor = match class.heritage_expression.as_deref()? {
        surge_ts_syntax::ParsedExpression::ClassExpression(class_expression) => {
            let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            crate::program::class_expression_type(class_expression, &symbols, ctx)
        }
        _ => return super::heritage::expression_base_instance_type(class, ctx),
    };
    let Type::Object(object) = constructor.peeled() else {
        return None;
    };
    let mut signatures = Vec::new();
    object.construct_signature()?.push_overload_members(&mut signatures);
    match signatures.as_slice() {
        [signature] if !signature.return_type().is_unmodelled() => Some(signature.return_type().clone()),
        _ => None,
    }
}

fn check_against_base(
    class: &ParsedClassDeclaration,
    base: &ParsedNamedType,
    members: &[Member<'_>],
    javascript: bool,
    ctx: &mut CheckerContext,
) {
    // The lookups are the heritage clause's, whose failures its own check
    // reports.
    let checkpoint = ctx.diagnostics().len();
    let expression_base = base.name == EXPRESSION_HERITAGE_BASE;
    let base_instance = if expression_base {
        expression_base_instance(class, ctx)
    } else {
        Some(crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::InterfaceHeritageResolution,
            || {
                map_parsed_type(
                    super::heritage::named_type_with(&base.name, None, base.type_arguments.clone()),
                    ctx,
                )
            },
        ))
    };
    let base_static = (!expression_base)
        .then(|| ctx.symbols.get(&base.name).map(|symbol| symbol.ty.clone()))
        .flatten();
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let Some(base_instance) = base_instance else {
        return;
    };
    if base_instance.is_unmodelled() || matches!(base_instance, Type::Any | Type::ErrorType) {
        return;
    }
    let base_display = if base.type_arguments.is_empty() && ctx.lookup_type_declaration(&base.name).is_some() {
        base.name.clone()
    } else {
        base_instance.name()
    };
    let ambient = class.is_declare || ctx.file_name.ends_with(".d.ts");
    let no_implicit_override = ctx.options.no_implicit_override;

    for member in members {
        let mut bound_name = None;
        match computed_name(class, member, ctx) {
            ComputedName::NotComputed => {}
            ComputedName::Bindable(name) => bound_name = name,
            ComputedName::Unknown => continue,
            ComputedName::Dynamic(span) => {
                if member.has_override {
                    let diagnostic = if javascript {
                        Diagnostic::ts4128(ctx.file_name.clone())
                    } else {
                        Diagnostic::ts4127(ctx.file_name.clone())
                    };
                    push_at(diagnostic, span, ctx);
                }
                continue;
            }
        }
        if !member.has_override && !no_implicit_override {
            continue;
        }
        let base_side = if member.is_static { base_static.as_ref() } else { Some(&base_instance) };
        // The base's members are named as `lateBindMember` binds them.
        let lookup_name = bound_name.as_deref().unwrap_or(member.name);
        let presence = match base_side {
            Some(base_side) => property_presence(base_side, lookup_name, !member.is_static, ctx),
            None => Presence::Unknown,
        };
        match presence {
            Presence::Unknown => {}
            Presence::Absent if member.has_override => {
                let suggestion = base_side.and_then(|base_side| suggested_base_member(base_side, lookup_name, ctx));
                let diagnostic = match (suggestion, javascript) {
                    (Some(suggestion), false) => Diagnostic::ts4117(&base_display, suggestion, ctx.file_name.clone()),
                    (Some(suggestion), true) => Diagnostic::ts4123(&base_display, suggestion, ctx.file_name.clone()),
                    (None, false) => Diagnostic::ts4113(&base_display, ctx.file_name.clone()),
                    (None, true) => Diagnostic::ts4122(&base_display, ctx.file_name.clone()),
                };
                push_at(diagnostic, member.name_span, ctx);
            }
            Presence::Absent => {}
            Presence::Present => {
                if member.has_override || !no_implicit_override || ambient {
                    continue;
                }
                // A base chain surge cannot walk to the declaration leaves
                // open whether it is abstract, which decides the report.
                let base_abstract = if member.is_static || expression_base {
                    Some(false)
                } else {
                    base_member_is_abstract(&base.name, member.name, ctx)
                };
                let Some(base_abstract) = base_abstract else {
                    continue;
                };
                let diagnostic = if !base_abstract {
                    // A JavaScript file declares no parameter property (TS8012
                    // stops it first), so its JSDoc variant (TS4120) never arises.
                    match (member.is_parameter, javascript) {
                        (true, _) => Diagnostic::ts4115(&base_display, ctx.file_name.clone()),
                        (false, false) => Diagnostic::ts4114(&base_display, ctx.file_name.clone()),
                        (false, true) => Diagnostic::ts4119(&base_display, ctx.file_name.clone()),
                    }
                } else if member.is_abstract {
                    Diagnostic::ts4116(&base_display, ctx.file_name.clone())
                } else {
                    continue;
                };
                push_at(diagnostic, member.name_span, ctx);
            }
        }
    }
}

/// The class's own members in the order tsc visits them, a constructor's
/// parameter properties in its place. An ambient (`declare`) property only
/// redeclares, and a JavaScript `this.x = v` is no class element.
fn override_members(class: &ParsedClassDeclaration) -> Vec<Member<'_>> {
    let mut members = Vec::new();
    for member in &class.members {
        match member {
            ParsedClassMember::Method(method) => members.push(Member {
                name: &method.name,
                name_span: method.computed_name_span.or(method.name_span),
                is_static: method.is_static,
                has_override: method.is_override,
                is_abstract: method.is_abstract,
                is_parameter: false,
            }),
            ParsedClassMember::Property(property) if property.is_declare || property.this_assignments.is_some() => {}
            ParsedClassMember::Property(property) => members.push(Member {
                name: &property.name,
                name_span: property.computed_name_span.or(property.name_span),
                is_static: property.is_static,
                has_override: property.is_override,
                is_abstract: property.is_abstract,
                is_parameter: false,
            }),
            ParsedClassMember::Accessor(accessor) => members.push(Member {
                name: &accessor.name,
                name_span: accessor.computed_name_span.or(accessor.name_span),
                is_static: accessor.is_static,
                has_override: accessor.is_override,
                is_abstract: accessor.is_abstract,
                is_parameter: false,
            }),
            ParsedClassMember::Constructor(constructor) => {
                for parameter in constructor.parameters.iter().filter(|parameter| parameter.is_parameter_property) {
                    let ParsedBindingName::Identifier { name, span } = &parameter.binding_name else {
                        continue;
                    };
                    members.push(Member {
                        name,
                        name_span: *span,
                        is_static: false,
                        has_override: parameter.is_override_parameter_property,
                        is_abstract: false,
                        is_parameter: true,
                    });
                }
            }
            ParsedClassMember::StaticBlock(_) => {}
        }
    }
    members
}

enum ComputedName {
    NotComputed,
    /// Bound to a member; the name it binds to when that differs from the
    /// written one.
    Bindable(Option<String>),
    /// A name no member is bound under, with where tsc reports it: from the
    /// `[`.
    Dynamic(Option<TextSpan>),
    Unknown,
}

/// tsc's `isNonBindableDynamicName`: a computed name whose type is not a
/// literal or a unique symbol names no member at all — its declaration has no
/// property to look up, with or without `override`. A well-known symbol
/// (`Symbol.iterator`) is always a unique one.
fn computed_name(class: &ParsedClassDeclaration, member: &Member<'_>, ctx: &mut CheckerContext) -> ComputedName {
    if !member.name.starts_with('[') {
        return ComputedName::NotComputed;
    }
    if member.name.starts_with("[Symbol.") {
        return ComputedName::Bindable(None);
    }
    let Some(span) = member.name_span else {
        return ComputedName::Unknown;
    };
    let Some((key, key_span)) = class.computed_keys.iter().find(|(_, key_span)| {
        key_span.is_some_and(|key_span| key_span.start <= span.start && span.end <= key_span.end)
    }) else {
        return ComputedName::Unknown;
    };
    let checkpoint = ctx.diagnostics().len();
    let symbols = ctx.symbols.clone();
    let key_type = infer_expression(key, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let InferredExpression::Known(key_type) = key_type else {
        return ComputedName::Unknown;
    };
    let is_name = |ty: &Type| {
        matches!(ty, Type::StringLiteral(_) | Type::NumberLiteral(_))
            || matches!(ty, Type::Reference(reference) if reference.is_unique_symbol())
    };
    let is_wide = |ty: &Type| matches!(ty, Type::String | Type::Number | Type::Symbol);
    match &key_type {
        Type::StringLiteral(value) => ComputedName::Bindable(Some(value.clone())),
        Type::NumberLiteral(literal) => ComputedName::Bindable(Some(literal.value.clone())),
        Type::Reference(reference) if reference.is_unique_symbol() => {
            ComputedName::Bindable(reference.unique_symbol_name().map(|name| format!("[{name}]")))
        }
        ty if is_wide(ty) => ComputedName::Dynamic(*key_span),
        Type::Union(union) if union.types().iter().all(|member| is_name(member) || is_wide(member)) => {
            ComputedName::Dynamic(*key_span)
        }
        _ => ComputedName::Unknown,
    }
}

/// `getPropertyOfType`: the type's own members, then the global `Function`
/// members for a callable or constructable one, then the global `Object`
/// members.
fn property_presence(ty: &Type, name: &str, instance_side: bool, ctx: &CheckerContext) -> Presence {
    let ty = ty.peeled();
    if ty.is_unmodelled() || matches!(ty, Type::Any | Type::ErrorType) {
        return Presence::Unknown;
    }
    match &ty {
        Type::Object(object) => {
            if object.get_property(name).is_some() {
                return Presence::Present;
            }
            let callable = object.call_signature().is_some() || object.construct_signature().is_some();
            if callable && global_interface_declares("Function", name, ctx) {
                return Presence::Present;
            }
            if global_interface_declares("Object", name, ctx) {
                return Presence::Present;
            }
            if object.synthetic_open_index || object.intersected_primitive().is_some() {
                return Presence::Unknown;
            }
            // A static side surge built without its lib `Function` surface
            // cannot tell an absent member from an unmodelled one.
            if !instance_side && !callable {
                return Presence::Unknown;
            }
            Presence::Absent
        }
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::Function(_) => {
            if ty.get_property_access_type(name).is_some() || global_interface_declares("Object", name, ctx) {
                Presence::Present
            } else {
                Presence::Absent
            }
        }
        _ => Presence::Unknown,
    }
}

pub(crate) fn global_interface_declares(interface: &str, name: &str, ctx: &CheckerContext) -> bool {
    match ctx.lookup_type_declaration(interface) {
        Some(TypeDeclarationInfo::Interface(info)) => {
            info.body.members.iter().any(|member| member.name == name)
                || info.body.extends.iter().any(|parent| global_interface_declares(&parent.name, name, ctx))
        }
        _ => match interface {
            "Object" => surge_ts_types::object_prototype_member_type(name).is_some(),
            _ => false,
        },
    }
}

/// `getSuggestedSymbolForNonexistentClassMember`: the spelling suggestion over
/// the base type's own properties — an array's are its lib `Array`'s.
fn suggested_base_member(base: &Type, name: &str, ctx: &CheckerContext) -> Option<String> {
    let candidates: Vec<String> = match base.peeled() {
        Type::Object(object) => object.properties.keys().map(|key| key.to_string()).collect(),
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => match ctx.lookup_type_declaration("Array") {
            Some(TypeDeclarationInfo::Interface(info)) => {
                info.body.members.iter().map(|member| member.name.clone()).collect()
            }
            _ => return None,
        },
        _ => return None,
    };
    crate::checks::expr::spelling_suggestion(name, candidates.iter().map(String::as_str), 0).map(str::to_string)
}

/// Whether the member `name` the base chain resolves to is declared
/// `abstract` — the nearest declaration wins, as `getPropertyOfType` finds it.
/// A base that is a value rather than a class (`extends Mixed`) takes its
/// members from construct signatures, which declare none abstract. `None`
/// when the chain leaves what this file can name before reaching it.
fn base_member_is_abstract(base_name: &str, name: &str, ctx: &CheckerContext) -> Option<bool> {
    if ctx.lookup_type_declaration(base_name).is_none() && ctx.symbols.get(base_name).is_some() {
        return Some(false);
    }
    let mut next = Some(base_name.to_string());
    let mut visited = std::collections::HashSet::new();
    while let Some(current) = next.take() {
        if !visited.insert(current.clone()) {
            break;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&current) else {
            return None;
        };
        if let Some(member) = info.body.members.iter().find(|member| member.name == name) {
            return Some(member.is_abstract);
        }
        next = info.body.extends.first().map(|parent| parent.name.clone());
    }
    Some(false)
}

fn push_at(diagnostic: Diagnostic, span: Option<TextSpan>, ctx: &mut CheckerContext) {
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}
