//! Heritage-clause member compatibility, ported from
//! `tsc/internal/checker/checker.go`.
//!
//! tsc's `issueMemberSpecificError` runs before it reports that a class as a
//! whole is incompatible with something it extends or implements: it walks the
//! class's own non-static members and reports each one that is not assignable
//! to the same-named base member (TS2416). Only when that walk reports nothing
//! does the broad diagnostic fire — which is what makes a *missing* member
//! TS2415/TS2420 and a *mistyped* one TS2416.
//!
//! An interface has no member-specific pass: extending an interface can only
//! fail by overriding a member incompatibly, so that single condition is the
//! whole rule and tsc reports it as TS2430 on the interface name.

use surge_ts_syntax::{
    ParsedClassDeclaration, ParsedClassMember, ParsedInterfaceDeclaration,
    ParsedMemberAccessibility, ParsedNamedType, ParsedType, TextSpan,
};

use crate::symbols::{InterfaceInfo, TypeDeclarationInfo};
use surge_ts_types::{Type, is_assignable_to};

use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;

use crate::context::{CheckerContext, convert_span};
use crate::infer::map_parsed_type;

/// A member a heritage walk compares: its name, and where tsc anchors an error
/// about it.
pub(crate) struct DeclaredMember {
    pub(crate) name: String,
    pub(crate) name_span: Option<TextSpan>,
}

pub(crate) fn class_instance_members(class: &ParsedClassDeclaration) -> Vec<DeclaredMember> {
    class
        .members
        .iter()
        .filter_map(|member| match member {
            ParsedClassMember::Property(property) if !property.is_static => Some(DeclaredMember {
                name: property.name.clone(),
                name_span: property.name_span,
            }),
            ParsedClassMember::Method(method) if !method.is_static => Some(DeclaredMember {
                name: method.name.clone(),
                name_span: method.name_span,
            }),
            ParsedClassMember::Accessor(accessor) if !accessor.is_static => Some(DeclaredMember {
                name: accessor.name.clone(),
                name_span: accessor.name_span,
            }),
            _ => None,
        })
        .collect()
}

fn named_type(name: &str, span: Option<TextSpan>) -> ParsedType {
    ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span,
        type_arguments: Vec::new(),
    }))
}

/// The member's type as the receiver's own surface answers it. `None` when the
/// receiver is not an object surface surge resolved, which is what keeps a
/// heritage name it could not model from reading as a mismatch.
fn property_type(receiver: &Type, name: &str) -> Option<Type> {
    match receiver.peeled() {
        Type::Object(object) => object.properties.get(name).map(|property| {
            if property.optional {
                surge_ts_types::union_type(vec![property.ty.clone(), Type::Undefined])
            } else {
                property.ty.clone()
            }
        }),
        // An array or tuple base: only its `length` and element slots are
        // modelled closely enough to relate a redeclaration to.
        peeled @ (Type::Array(_) | Type::Tuple(_))
            if name == "length" || super::is_numeric_literal_name(name) =>
        {
            peeled.get_property_access_type(name)
        }
        _ => None,
    }
}

/// Whether the two types are comparable at all. A modelling failure on either
/// side is not evidence of an incompatible override, and `any` relates to
/// everything, so both are left alone rather than reported. A type variable of
/// the declaration being checked is a type like any other.
fn comparable(source: &Type, target: &Type) -> bool {
    // surge merges an intersection's object operands and drops a tuple or
    // array one (`TType & { [tag]: V }`), so a merged intersection is not the
    // type tsc relates.
    let modelled = |ty: &Type| {
        !ty.is_unmodelled()
            && !matches!(ty, Type::ErrorType | Type::Any)
            && !matches!(ty.peeled(), Type::Object(object) if object.is_intersection)
    };
    modelled(source) && modelled(target)
}

/// The member itself, as a one-member object: a method compares its
/// parameters bivariantly (tsc's `compareSignaturesRelated` for a method
/// declaration), which only the member, not its function type, records. Only
/// the types are related (`issueMemberSpecificError`), so neither a
/// `private`/`protected` restriction nor `readonly` takes part.
fn member_alone(receiver: &Type, name: &str) -> Option<Type> {
    let Type::Object(object) = receiver.peeled() else {
        return None;
    };
    let mut property = object.properties.get(name)?.clone();
    property.restriction = None;
    property.readonly = false;
    let mut properties = surge_ts_types::PropertyMap::default();
    properties.insert(name.into(), property);
    Some(Type::Object(crate::metrics::alloc_object_type(properties, None)))
}

/// Names every declared member of `members` whose type is not assignable to the
/// same-named member of `base_type`, with the span tsc anchors on.
fn incompatible_members(
    members: &[DeclaredMember],
    own_type: &Type,
    base_type: &Type,
) -> Vec<(String, Option<TextSpan>)> {
    let mut incompatible = Vec::new();
    for member in members {
        let (Some(own), Some(base)) = (
            property_type(own_type, &member.name),
            property_type(base_type, &member.name),
        ) else {
            continue;
        };
        if !comparable(&own, &base) {
            continue;
        }
        let assignable = match (
            member_alone(own_type, &member.name),
            member_alone(base_type, &member.name),
        ) {
            (Some(own_member), Some(base_member)) => is_assignable_to(&own_member, &base_member),
            _ => is_assignable_to(&own, &base),
        };
        if !assignable {
            incompatible.push((member.name.clone(), member.name_span));
        }
    }
    incompatible
}

/// TS2416 for every member of `class` that overrides a same-named member of
/// `base` incompatibly. Returns whether anything was reported, which is what
/// gates the broad diagnostic. A generic class is related through its own type
/// variables, and a base written with type arguments through them.
pub(crate) fn report_incompatible_heritage_members(
    class: &ParsedClassDeclaration,
    base: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    if class.is_declare {
        return false;
    }
    if class.type_parameters.is_empty() {
        return report_incompatible_heritage_members_in_scope(class, base, ctx);
    }
    let _type_variables = crate::checks::function::enter_body_type_variables(&class.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        report_incompatible_heritage_members_in_scope(class, base, ctx)
    })
}

fn report_incompatible_heritage_members_in_scope(
    class: &ParsedClassDeclaration,
    base: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    let own_arguments: Vec<ParsedType> =
        class.type_parameters.iter().map(|parameter| named_type(&parameter.name, None)).collect();
    // Both are the clause's own lookups, whose failures its own check reports.
    let checkpoint = ctx.diagnostics().len();
    let own_type = map_parsed_type(named_type_with(&class.name, class.name_span, own_arguments), ctx);
    // Resolved as the heritage position resolves it, where a base that names a
    // value (`extends Mixed`) takes its instance type from the constructor.
    let base_type = if base.name == surge_ts_syntax::EXPRESSION_HERITAGE_BASE {
        expression_base_instance_type(class, ctx)
    } else {
        Some(crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::InterfaceHeritageResolution,
            || map_parsed_type(named_type_with(&base.name, None, base.type_arguments.clone()), ctx),
        ))
    };
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let Some(base_type) = base_type else {
        return false;
    };
    let members = class_instance_members(class);
    let incompatible = incompatible_members(&members, &own_type, &base_type);
    if incompatible.is_empty() {
        return false;
    }

    // tsc names the base *type*; for a value base that is the constructed
    // instance, not the identifier in the `extends` clause.
    let base_display = if base.type_arguments.is_empty() && ctx.lookup_type_declaration(&base.name).is_some() {
        base.name.clone()
    } else {
        base_type.name()
    };
    let own_display = type_display(
        &class.name,
        &class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
    );
    for (name, name_span) in incompatible {
        let diagnostic = Diagnostic::ts2416(&name, &own_display, &base_display, ctx.file_name.clone());
        ctx.push(match name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
    true
}

/// The instance type an `extends f(...)` call base constructs: the return of
/// its one construct signature. A method call is left out, as the class check
/// leaves its arguments (`extends obj.method()`); so is an overloaded
/// constructor, whose instance tsc picks by the class's type arguments.
pub(crate) fn expression_base_instance_type(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) -> Option<Type> {
    let expression = class.heritage_expression.as_deref()?;
    if !matches!(expression, surge_ts_syntax::ParsedExpression::Call { .. }) {
        return None;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let crate::infer::InferredExpression::Known(constructor) = crate::infer::infer_expression(expression, &symbols, ctx)
    else {
        return None;
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

/// TS2509 for an `extends f(...)` base that constructs `never`: an
/// intersection of mixin constructors whose instances conflict on a private
/// member reduces to it (`isConflictingPrivateProperty`), and `isValidBaseType`
/// rejects it.
pub(crate) fn report_never_mixin_base(class: &ParsedClassDeclaration, base: &ParsedNamedType, ctx: &mut CheckerContext) {
    let Some(expression) = class.heritage_expression.as_deref() else {
        return;
    };
    if !matches!(expression, surge_ts_syntax::ParsedExpression::Call { .. }) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let crate::infer::InferredExpression::Known(constructor) = inferred else {
        return;
    };
    let Type::Object(object) = constructor.peeled() else {
        return;
    };
    let Some(signature) = object.construct_signature() else {
        return;
    };
    let mut signatures = Vec::new();
    signature.push_overload_members(&mut signatures);
    if signatures.iter().all(|signature| matches!(signature.return_type().peeled(), Type::Never)) {
        let diagnostic = Diagnostic::ts2509("never", ctx.file_name.clone());
        ctx.push(match base.span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// `getInstantiatedConstructorsForTypeArguments` over an `extends f(...)`
/// base: TS2508 when no construct signature takes the written number of type
/// arguments, and TS2510 when the non-generic ones construct different types
/// (`checkClassLikeDeclaration`).
pub(crate) fn report_call_base_constructors(class: &ParsedClassDeclaration, base: &ParsedNamedType, ctx: &mut CheckerContext) {
    let Some(expression) = class.heritage_expression.as_deref() else {
        return;
    };
    if !matches!(expression, surge_ts_syntax::ParsedExpression::Call { .. }) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let crate::infer::InferredExpression::Known(constructor) = inferred else {
        return;
    };
    let Type::Object(object) = constructor.peeled() else {
        return;
    };
    let Some(signature) = object.construct_signature() else {
        return;
    };
    let mut signatures = Vec::new();
    signature.push_overload_members(&mut signatures);
    let span = base.span.map(convert_span);
    let push = |diagnostic: Diagnostic, ctx: &mut CheckerContext| {
        ctx.push(match span {
            Some(span) => diagnostic.with_span(span),
            None => diagnostic,
        });
    };
    let written = base.type_arguments.len();
    if written > 0 {
        if !signatures.iter().any(|signature| signature.type_parameter_names().len() == written) {
            push(Diagnostic::ts2508(ctx.file_name.clone()), ctx);
        }
        return;
    }
    if signatures.len() > 1
        && signatures.iter().all(|signature| signature.type_parameter_names().is_empty())
        && signatures.iter().all(|signature| !signature.return_type().peeled().is_unknown())
        && signatures[1..]
            .iter()
            .any(|signature| !surge_ts_types::is_type_identical_to(signature.return_type(), signatures[0].return_type()))
    {
        push(Diagnostic::ts2510(ctx.file_name.clone()), ctx);
    }
}

/// The index half of tsc's `checkTypeAssignableTo(typeWithThis, baseWithThis)`
/// for a class declaring an index signature of its own (an inherited one is
/// the base's and relates): the broad diagnostic `report` builds from the two
/// displays, at the class name. Returns whether it reported.
pub(crate) fn report_incompatible_index_signatures(
    class: &ParsedClassDeclaration,
    base: &ParsedNamedType,
    report: impl FnOnce(&str, &str, String) -> Diagnostic,
    ctx: &mut CheckerContext,
) -> bool {
    if class.is_declare || (class.string_index_type.is_none() && class.number_index_type.is_none()) {
        return false;
    }
    let _type_variables = crate::checks::function::enter_body_type_variables(&class.type_parameters, ctx);
    let failed = crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        let own_arguments: Vec<ParsedType> =
            class.type_parameters.iter().map(|parameter| named_type(&parameter.name, None)).collect();
        let checkpoint = ctx.diagnostics().len();
        let own_type = map_parsed_type(named_type_with(&class.name, class.name_span, own_arguments), ctx);
        let base_type = crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::InterfaceHeritageResolution,
            || map_parsed_type(named_type_with(&base.name, None, base.type_arguments.clone()), ctx),
        );
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        let (Type::Object(own), Type::Object(base_object)) = (own_type.peeled(), base_type.peeled()) else {
            return None;
        };
        if own.synthetic_open_index || base_object.synthetic_open_index {
            return None;
        }
        let modelled = |ty: &Type| !crate::checks::function::type_contains_degradation(ty);
        let unrelated = |own_index: Option<&Type>, base_index: Option<&Type>| match (own_index, base_index) {
            (Some(own_index), Some(base_index)) => {
                modelled(own_index) && modelled(base_index) && !is_assignable_to(own_index, base_index)
            }
            _ => false,
        };
        let own_numeric = own.number_index_type.as_deref().or(own.string_index_type.as_deref());
        let incompatible = unrelated(own.string_index_type.as_deref(), base_object.string_index_type.as_deref())
            || unrelated(own_numeric, base_object.number_index_type.as_deref());
        incompatible.then(|| base_type.name())
    });
    let Some(base_display) = failed else {
        return false;
    };
    let base_display = if base.type_arguments.is_empty() && ctx.lookup_type_declaration(&base.name).is_some() {
        base.name.clone()
    } else {
        base_display
    };
    let own_display = type_display(
        &class.name,
        &class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
    );
    let diagnostic = report(&own_display, &base_display, ctx.file_name.clone());
    ctx.push(match class.name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
    true
}

/// `propertyRelatedTo`'s modifier rules for a class against a type it
/// implements, which carries the `private` and `protected` members of every
/// class that type extends: a private member relates only to its own
/// declaration, a protected target only to a member declared in a class
/// derived from the target's declaring class (`isValidOverrideOf`), and a
/// protected source never to a public target.
pub(crate) fn implements_unrelated_accessibility(
    class: &ParsedClassDeclaration,
    implemented: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    if class.type_parameters.is_empty() {
        return implements_unrelated_accessibility_in_scope(class, implemented, ctx);
    }
    let _type_variables = crate::checks::function::enter_body_type_variables(&class.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        implements_unrelated_accessibility_in_scope(class, implemented, ctx)
    })
}

fn implements_unrelated_accessibility_in_scope(
    class: &ParsedClassDeclaration,
    implemented: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    let own_arguments: Vec<ParsedType> =
        class.type_parameters.iter().map(|parameter| named_type(&parameter.name, None)).collect();
    let checkpoint = ctx.diagnostics().len();
    let own_type = map_parsed_type(named_type_with(&class.name, class.name_span, own_arguments), ctx);
    let implemented_type = crate::program::with_dts_expansion_reason(
        crate::program::DtsExpansionReason::InterfaceHeritageResolution,
        || map_parsed_type(named_type_with(&implemented.name, None, implemented.type_arguments.clone()), ctx),
    );
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let (Type::Object(own), Type::Object(implemented_object)) = (own_type.peeled(), implemented_type.peeled()) else {
        return false;
    };
    let lineage = class_lineage(class, ctx);
    implemented_object.properties.iter().any(|(name, target_property)| {
        let Some(own_property) = own.properties.get(&**name) else {
            return false;
        };
        let source = own_property.restriction.as_ref();
        let target = target_property.restriction.as_ref();
        if source.is_some_and(|restriction| restriction.private) || target.is_some_and(|restriction| restriction.private) {
            return source != target;
        }
        match (source, target) {
            (_, Some(target)) => {
                let declaring = match source {
                    Some(source) => Some(source.owner.to_string()),
                    None => public_member_owner(&lineage, name),
                };
                declaring.is_some_and(|declaring| !derives_from(&lineage, &declaring, &target.owner))
            }
            (Some(_), None) => true,
            (None, None) => false,
        }
    })
}

/// The class's instance declaration and every class its `extends` chain
/// reaches, nearest first.
fn class_lineage(class: &ParsedClassDeclaration, ctx: &CheckerContext) -> Vec<InterfaceInfo> {
    let mut lineage: Vec<InterfaceInfo> = Vec::new();
    let mut current = match ctx.lookup_type_declaration(&class.name) {
        Some(TypeDeclarationInfo::Interface(info)) if info.is_class_instance => Some(info.clone()),
        _ => None,
    };
    while let Some(info) = current.take() {
        if lineage.len() >= 32 || lineage.iter().any(|seen| owner_mark(seen) == owner_mark(&info)) {
            break;
        }
        current = crate::checks::expr::base_interface(&info, ctx).filter(|base| base.is_class_instance);
        lineage.push(info);
    }
    lineage
}

/// The declaring-class mark a restricted member of `info`'s instance type
/// carries (see `attach_member_restrictions`).
fn owner_mark(info: &InterfaceInfo) -> Option<String> {
    info.name_span.map(|span| format!("{}\0{}", info.file_name, span.start))
}

/// The class a public member is declared in: the nearest one on the lineage
/// that declares it.
fn public_member_owner(lineage: &[InterfaceInfo], name: &str) -> Option<String> {
    lineage
        .iter()
        .find(|info| info.body.members.iter().any(|member| member.name == name))
        .and_then(owner_mark)
}

/// `hasBaseType` between two declaring-class marks, read along the checked
/// class's lineage. A declaring class off the lineage cannot be placed, and
/// counts as derived.
fn derives_from(lineage: &[InterfaceInfo], declaring: &str, base: &str) -> bool {
    let Some(position) = lineage.iter().position(|info| owner_mark(info).as_deref() == Some(declaring)) else {
        return true;
    };
    lineage[position..].iter().any(|info| owner_mark(info).as_deref() == Some(base))
}

/// tsc's base-type circularity: a class whose `extends` chain leads back to
/// itself cannot resolve its base constructor (`getBaseConstructorTypeOfClass`,
/// TS2506), and an interface's base types cannot resolve at all
/// (`reportCircularBaseType`, TS2310, at each of its declarations). Each base
/// name is resolved where the declaration naming it is written.
pub(crate) fn report_circular_base(
    name: &str,
    name_span: Option<TextSpan>,
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    is_class: bool,
    ctx: &mut CheckerContext,
) {
    let Some(TypeDeclarationInfo::Interface(start)) = ctx.lookup_type_declaration(name) else {
        return;
    };
    let start = start.clone();
    if !base_chain_reaches(&start, ctx) {
        return;
    }
    let diagnostic = if is_class {
        Diagnostic::ts2506(name, ctx.file_name.clone())
    } else if type_parameters.is_empty() {
        Diagnostic::ts2310(name, ctx.file_name.clone())
    } else {
        let parameters: Vec<&str> = type_parameters.iter().map(|parameter| parameter.name.as_str()).collect();
        Diagnostic::ts2310(format!("{name}<{}>", parameters.join(", ")), ctx.file_name.clone())
    };
    ctx.push(match name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

fn base_chain_reaches(start: &InterfaceInfo, ctx: &mut CheckerContext) -> bool {
    let key = |info: &InterfaceInfo| (info.file_name.clone(), info.name_span.map(|span| span.start));
    let target = key(start);
    // A class has one base, its first `extends` type (`getEffectiveBaseTypeNode`).
    let bases = |info: &InterfaceInfo| {
        let count = if info.is_class_instance { 1 } else { info.body.extends.len() };
        info.body
            .extends
            .iter()
            .take(count)
            .map(|base| (info.resolution_scope.clone(), base.name.clone()))
            .collect::<Vec<_>>()
    };
    let mut pending = bases(start);
    let mut visited = Vec::new();
    while let Some((scope, name)) = pending.pop() {
        let found = crate::infer::types::with_type_declaration_scope(&scope, ctx, |ctx| {
            ctx.lookup_type_declaration(&name).cloned()
        });
        let Some(TypeDeclarationInfo::Interface(info)) = found else { continue };
        let info_key = key(&info);
        if info_key == target {
            return true;
        }
        if visited.contains(&info_key) {
            continue;
        }
        visited.push(info_key);
        pending.extend(bases(&info));
    }
    false
}

/// tsc's `checkInterfaceDeclaration` heritage checks, on the interface as its
/// own type parameters instantiate it: bases that supply the same member must
/// supply identical ones (`checkInheritedPropertiesAreIdentical`, TS2320),
/// and only then may the interface override an inherited member with nothing
/// but a type assignable to it (TS2430). Both report on the interface's name.
pub(crate) fn check_interface_heritage(
    interface: &ParsedInterfaceDeclaration,
    ctx: &mut CheckerContext,
) {
    // A declaration merged within this file is checked once, at its first
    // declaration, over every declaration's bases and members.
    let merged = match ctx.lookup_type_declaration(&interface.name) {
        Some(crate::symbols::TypeDeclarationInfo::Interface(info))
            if info.body.declaration_fragments.len() > 1
                && !info.body.has_foreign_fragments()
                && info
                    .body
                    .declaration_fragments
                    .iter()
                    .all(|fragment| *fragment.file_name == *ctx.file_name) =>
        {
            Some(info.body.clone())
        }
        _ => None,
    };
    let (extends, members) = match &merged {
        Some(body) => {
            let declaration_start = interface.name_span.map_or(0, |span| span.start);
            if body.declaration_fragments[0].declaration_start != declaration_start {
                return;
            }
            (
                body.extends.clone(),
                body.members.iter().map(|member| (member.name.clone(), member.name_span)).collect(),
            )
        }
        None => (
            interface.extends.clone(),
            interface.members.iter().map(|member| (member.name.clone(), member.name_span)).collect::<Vec<_>>(),
        ),
    };
    if extends.is_empty() {
        return;
    }
    let _variables = crate::checks::function::enter_body_type_variables(&interface.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(&interface.type_parameters, ctx, |ctx| {
        check_interface_heritage_in_scope(interface, &extends, &members, ctx);
    });
}

/// A script's interfaces, and a module's `declare global` ones, reopen the
/// default library's, and tsc checks the lib's own interfaces against the
/// augmented bases: an index signature added to `Function` breaks
/// `CallableFunction`'s `this` types. Reported at the lib's declaration; only
/// the direct subinterfaces of an augmented interface are rechecked.
pub(crate) fn check_augmented_lib_subinterfaces(
    statements: &[surge_ts_syntax::ParsedStatement],
    is_module: bool,
    ctx: &mut CheckerContext,
) {
    use surge_ts_syntax::ParsedStatement;
    fn collect<'a>(statement: &'a ParsedStatement, names: &mut Vec<&'a str>) {
        match statement {
            ParsedStatement::InterfaceDeclaration(interface) => names.push(interface.name.as_str()),
            ParsedStatement::ExportDeclaration(export) => {
                if let surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                    collect(declaration, names);
                }
            }
            _ => {}
        }
    }
    if ctx.options.skip_lib_check {
        return;
    }
    let mut augmented = Vec::new();
    for statement in statements {
        match statement {
            ParsedStatement::DeclareModuleDeclaration(module) if is_module && module.module_specifier == "global" => {
                for statement in &module.statements {
                    collect(statement, &mut augmented);
                }
            }
            _ if !is_module => collect(statement, &mut augmented),
            _ => {}
        }
    }
    augmented.retain(|name| {
        crate::program::program_lib_declaration_sites(name)
            .iter()
            .any(|(_, _, kind)| *kind == crate::program::LibDeclarationKind::Interface)
    });
    if augmented.is_empty() {
        return;
    }
    let is_lib = |file_name: &str| {
        crate::default_lib::is_physical_default_lib_file_name(file_name)
            || crate::default_lib::is_generated_default_lib_file_name(file_name)
    };
    let subinterfaces: Vec<InterfaceInfo> = ctx
        .ambient_global_type_declarations
        .iter()
        .filter_map(|(_, declaration)| match declaration {
            TypeDeclarationInfo::Interface(info)
                if is_lib(&info.file_name)
                    && info.body.extends.iter().any(|base| augmented.contains(&base.name.as_str())) =>
            {
                Some(info.clone())
            }
            _ => None,
        })
        .collect();
    for info in subinterfaces {
        let body = &info.body;
        let interface = ParsedInterfaceDeclaration {
            is_declare: true,
            name: info.name.to_string(),
            name_span: info.name_span,
            type_parameters: body.type_parameters.clone(),
            extends: body.extends.clone(),
            members: Vec::new(),
            string_index_type: body.string_index_type.clone(),
            number_index_type: body.number_index_type.clone(),
            string_index_span: None,
            number_index_span: None,
            string_index_readonly: body.string_index_readonly,
            number_index_readonly: body.number_index_readonly,
            call_signature: None,
            call_signature_overloads: Vec::new(),
            construct_signatures: Vec::new(),
            keyed_indexes: None,
        };
        let members: Vec<(String, Option<TextSpan>)> =
            body.members.iter().map(|member| (member.name.clone(), member.name_span)).collect();
        let checkpoint = ctx.diagnostics().len();
        {
            let _variables = crate::checks::function::enter_body_type_variables(&interface.type_parameters, ctx);
            crate::checks::function::with_type_parameter_scope(&interface.type_parameters, ctx, |ctx| {
                check_interface_heritage_in_scope(&interface, &interface.extends, &members, ctx);
            });
        }
        let reports: Vec<Diagnostic> = ctx.diagnostics()[checkpoint..]
            .iter()
            .filter(|diagnostic| {
                matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(2320 | 2430))
            })
            .cloned()
            .collect();
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        for mut diagnostic in reports {
            diagnostic.file_name = info.file_name.to_string();
            ctx.push(diagnostic);
        }
    }
}

fn check_interface_heritage_in_scope(
    interface: &ParsedInterfaceDeclaration,
    extends: &[ParsedNamedType],
    declared_members: &[(String, Option<TextSpan>)],
    ctx: &mut CheckerContext,
) {
    let members: Vec<DeclaredMember> = declared_members
        .iter()
        .map(|(name, name_span)| DeclaredMember {
            name: name.clone(),
            name_span: *name_span,
        })
        .collect();
    let own_arguments: Vec<ParsedType> = interface
        .type_parameters
        .iter()
        .map(|parameter| named_type(&parameter.name, None))
        .collect();
    let own_type = map_parsed_type(named_type_with(&interface.name, interface.name_span, own_arguments), ctx);
    let own_display = type_display(&interface.name, &interface.type_parameters.iter().map(|p| p.name.clone()).collect::<Vec<_>>());

    let mut bases = Vec::new();
    for base in extends {
        if base.type_arguments.is_empty()
            && interface.type_parameters.iter().any(|parameter| parameter.name == base.name)
        {
            let diagnostic = Diagnostic::ts2312(ctx.file_name.clone());
            ctx.push(match base.span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
            continue;
        }
        // A base that names no type is already reported, at its own span.
        if !base.name.contains('.') && ctx.lookup_type_declaration(&base.name).is_none() {
            continue;
        }
        // The reference reports its own errors where it is written.
        let checkpoint = ctx.diagnostics().len();
        let base_type = map_parsed_type(named_type_with(&base.name, None, base.type_arguments.clone()), ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        // `isValidBaseType`: an object type, or an intersection of them.
        if !is_valid_interface_base(&base_type) {
            let diagnostic = Diagnostic::ts2312(ctx.file_name.clone());
            ctx.push(match base.span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
            continue;
        }
        let display = if base.type_arguments.is_empty() { base.name.clone() } else { base_type.name() };
        bases.push((display, base_type));
    }

    if bases.len() > 1 && report_non_identical_inherited_members(interface, &own_display, &members, &bases, ctx) {
        return;
    }
    for (display, base_type) in &bases {
        if incompatible_members(&members, &own_type, base_type).is_empty()
            && !redeclares_restricted_member(&members, base_type)
            && !declared_index_signatures_incompatible(interface, &own_type, base_type)
        {
            continue;
        }
        let diagnostic = Diagnostic::ts2430(&own_display, display, ctx.file_name.clone());
        ctx.push(match interface.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// Whether tsc's `isValidBaseType` holds, judged only where surge's type says
/// so for certain: a union, a type parameter or a primitive is no base.
fn is_valid_interface_base(base: &Type) -> bool {
    !matches!(
        base.peeled(),
        Type::Union(_)
            | Type::TypeParameter(_)
            | Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Never
    )
}

/// `propertyRelatedTo`'s modifier rules for a member the interface redeclares
/// over a class it extends: a private base member relates only to its own
/// declaration, and a protected one only to a member of a derived class,
/// which an interface member never is.
fn redeclares_restricted_member(members: &[DeclaredMember], base_type: &Type) -> bool {
    let Type::Object(base) = base_type.peeled() else {
        return false;
    };
    members.iter().any(|member| {
        base.properties
            .get(member.name.as_str())
            .is_some_and(|property| property.restriction.is_some())
    })
}

/// The index half of the interface's `checkTypeAssignableTo` against a base
/// (`indexSignaturesRelatedTo`): a numeric target index reads the source's
/// number index, else its string one. Only an index the interface declares
/// can fail — an inherited one is the base's own.
fn declared_index_signatures_incompatible(
    interface: &ParsedInterfaceDeclaration,
    own_type: &Type,
    base_type: &Type,
) -> bool {
    if interface.string_index_type.is_none() && interface.number_index_type.is_none() {
        return false;
    }
    let (Type::Object(own), Type::Object(base)) = (own_type.peeled(), base_type.peeled()) else {
        return false;
    };
    if own.synthetic_open_index || base.synthetic_open_index {
        return false;
    }
    let unrelated = |own_index: Option<&Type>, base_index: Option<&Type>| match (own_index, base_index) {
        (Some(own_index), Some(base_index)) => {
            comparable(own_index, base_index) && !is_assignable_to(own_index, base_index)
        }
        _ => false,
    };
    let own_numeric = own.number_index_type.as_deref().or(own.string_index_type.as_deref());
    unrelated(own.string_index_type.as_deref(), base.string_index_type.as_deref())
        || unrelated(own_numeric, base.number_index_type.as_deref())
}

/// `checkInheritedPropertiesAreIdentical`: a member the interface does not
/// declare itself, supplied by two bases, must be the same member in both.
fn report_non_identical_inherited_members(
    interface: &ParsedInterfaceDeclaration,
    own_display: &str,
    members: &[DeclaredMember],
    bases: &[(String, Type)],
    ctx: &mut CheckerContext,
) -> bool {
    let mut seen: Vec<(Arc<str>, surge_ts_types::ObjectProperty, usize)> = Vec::new();
    let mut identical = true;
    for (index, (_, base_type)) in bases.iter().enumerate() {
        let Type::Object(object) = base_type.peeled() else {
            continue;
        };
        for (name, property) in object.properties.iter() {
            if members.iter().any(|member| member.name == name.as_ref()) {
                continue;
            }
            match seen.iter().find(|(seen_name, _, _)| seen_name == name) {
                None => seen.push((name.clone(), property.clone(), index)),
                Some((_, existing, existing_index)) => {
                    if properties_identical(existing, property) {
                        continue;
                    }
                    identical = false;
                    let diagnostic = Diagnostic::ts2320(
                        own_display,
                        &bases[*existing_index].0,
                        &bases[index].0,
                        ctx.file_name.clone(),
                    );
                    ctx.push(match interface.name_span {
                        Some(span) => diagnostic.with_span(convert_span(span)),
                        None => diagnostic,
                    });
                }
            }
        }
    }
    !identical
}

/// `compareProperties` under the identity relation. A member surge could not
/// model is not evidence of a conflict.
fn properties_identical(left: &surge_ts_types::ObjectProperty, right: &surge_ts_types::ObjectProperty) -> bool {
    if !comparable(&left.ty, &right.ty)
        || surge_ts_types::parameter_type_is_degraded(&left.ty)
        || surge_ts_types::parameter_type_is_degraded(&right.ty)
    {
        return true;
    }
    left.restriction == right.restriction
        && (left.restriction.is_some() || left.optional == right.optional)
        && left.readonly == right.readonly
        && surge_ts_types::is_type_identical_to(&left.ty, &right.ty)
}

pub(crate) fn named_type_with(name: &str, span: Option<TextSpan>, type_arguments: Vec<ParsedType>) -> ParsedType {
    ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span,
        type_arguments,
    }))
}

/// `typeToString` of a generic declaration's own type: `I<T, U>`.
pub(crate) fn type_display(name: &str, type_parameters: &[String]) -> String {
    if type_parameters.is_empty() {
        name.to_string()
    } else {
        format!("{name}<{}>", type_parameters.join(", "))
    }
}

/// The `extends` relation tsc checks once the member walk found nothing:
/// `typeWithThis` against `baseWithThis` (TS2415), then, only when that
/// holds, the static sides (TS2417). Two of `propertyRelatedTo`'s failures
/// are visible without types — a `private` or `protected` member on one side
/// that is not on the other — and the static side adds the type comparison
/// the instance walk already did. What is not modelled here: an inherited
/// index signature the class's own members violate, and `#private` names,
/// which the parser drops.
pub(crate) fn check_base_class_relation(
    class: &ParsedClassDeclaration,
    base_name: &str,
    ctx: &mut CheckerContext,
) {
    if class.is_declare || !class.type_parameters.is_empty() {
        return;
    }
    let Some(TypeDeclarationInfo::Interface(base)) = ctx.lookup_type_declaration(base_name) else {
        return;
    };
    if !base.is_class_instance {
        return;
    }
    let base = base.clone();
    let base_display = base
        .declared_name
        .as_deref()
        .unwrap_or(&base.name)
        .to_string();

    let own_accessibility = |name: &str, is_static: bool| {
        class
            .restricted_members
            .iter()
            .find(|member| member.name == name && member.is_static == is_static)
            .map(|member| member.accessibility)
    };
    let instance_members: Vec<String> = class_instance_members(class)
        .into_iter()
        .map(|member| member.name)
        .chain(
            super::classes::constructor_parameter_property_members(class)
                .into_iter()
                .map(|member| member.name),
        )
        .collect();
    let instance_incompatible = instance_members.iter().any(|name| {
        let Some(base_accessibility) = base_member_accessibility(&base, name, false, ctx) else {
            return false;
        };
        !accessibility_related(own_accessibility(name, false), base_accessibility)
    });
    if instance_incompatible {
        let diagnostic = Diagnostic::ts2415(&class.name, &base_display, ctx.file_name.clone());
        ctx.push(match class.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
        return;
    }

    let static_members: Vec<String> = class
        .members
        .iter()
        .filter_map(|member| match member {
            ParsedClassMember::Property(property) if property.is_static => Some(property.name.clone()),
            ParsedClassMember::Method(method) if method.is_static => Some(method.name.clone()),
            ParsedClassMember::Accessor(accessor) if accessor.is_static => Some(accessor.name.clone()),
            _ => None,
        })
        .collect();
    if static_members.is_empty() {
        return;
    }
    let static_type = |name: &str| ctx.symbols.get(name).map(|symbol| symbol.ty.peeled());
    let (own_static, base_static) = (static_type(&class.name), static_type(base_name));
    let static_incompatible = static_members.iter().any(|name| {
        let Some(base_accessibility) = base_member_accessibility(&base, name, true, ctx) else {
            return false;
        };
        if !accessibility_related(own_accessibility(name, true), base_accessibility) {
            return true;
        }
        let (Some(own), Some(base)) = (
            own_static.as_ref().and_then(|ty| property_type(ty, name)),
            base_static.as_ref().and_then(|ty| property_type(ty, name)),
        ) else {
            return false;
        };
        comparable(&own, &base) && !is_assignable_to(&own, &base)
    });
    if static_incompatible {
        let diagnostic = Diagnostic::ts2417(
            format!("typeof {}", class.name),
            format!("typeof {base_display}"),
            ctx.file_name.clone(),
        );
        ctx.push(match class.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// The modifier the base side of the relation sees for `name`: `Some(None)`
/// for a public member the base chain declares, `None` when no class in the
/// chain declares it at all.
fn base_member_accessibility(
    base: &InterfaceInfo,
    name: &str,
    is_static: bool,
    ctx: &CheckerContext,
) -> Option<Option<ParsedMemberAccessibility>> {
    let mut current = base.clone();
    for _ in 0..32 {
        if let Some(restricted) = current
            .body
            .restricted_members
            .iter()
            .find(|member| member.name == name && member.is_static == is_static)
        {
            return Some(Some(restricted.accessibility));
        }
        if !is_static && current.body.members.iter().any(|member| member.name == name) {
            return Some(None);
        }
        // The class's own statics only: its static type also carries the
        // ones it inherits, whose modifiers belong to the class declaring
        // them further up the chain.
        if is_static
            && current.body.class_members.iter().any(|member| member.is_static && member.name == name)
        {
            return Some(None);
        }
        current = crate::checks::expr::base_interface(&current, ctx)?;
    }
    None
}

/// tsc's `propertyRelatedTo` on modifiers, for a derived class against its
/// base: a private member must be the very same declaration on both sides,
/// which a redeclaration never is; a protected base member may be widened by
/// the subclass; a protected derived member may not hide a public one.
fn accessibility_related(
    own: Option<ParsedMemberAccessibility>,
    base: Option<ParsedMemberAccessibility>,
) -> bool {
    match (own, base) {
        (Some(ParsedMemberAccessibility::Private), _) | (_, Some(ParsedMemberAccessibility::Private)) => false,
        (_, Some(ParsedMemberAccessibility::Protected)) => true,
        (Some(ParsedMemberAccessibility::Protected), None) => false,
        (None, None) => true,
    }
}
