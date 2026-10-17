//! tsc's `onFailedToResolveSymbol`: what a name that resolves to nothing is
//! reported as. The checks run in tsc's order, and only when none of them
//! claims the name does it fall through to a missing-lib hint, a spelling
//! suggestion, and finally the name-specific "cannot find" message.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::TextSpan as SyntaxTextSpan;
use surge_ts_types::Type;

use super::suggested_lib_for_nonexistent_name;
use crate::context::CheckerContext;
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::SymbolTable;

use super::inferred::suggested_value_name;

/// Where an unresolved value name was written, as far as tsc's choice of
/// message depends on it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnresolvedNameSite {
    Reference,
    /// The callee of a call: `await(x)` outside an async function.
    Callee,
    /// The name of a shorthand property, `{ name }`.
    ShorthandProperty,
    /// The operand of a type query, `typeof name`.
    TypeQuery,
    /// A name resolved at a node spelled otherwise (a JSX tag's factory), for
    /// which `checkAndReportErrorForMissingPrefix` does not apply.
    Implicit,
}

/// A class whose member bodies enclose the code being checked.
#[derive(Debug, Clone)]
pub(crate) struct EnclosingClassMembers {
    pub(crate) class_name: String,
    pub(crate) instance_type: Type,
    pub(crate) static_type: Type,
}

/// Reports an unresolved value name the way tsc does.
pub(crate) fn report_unresolved_value_name(
    name: &str,
    span: Option<SyntaxTextSpan>,
    site: UnresolvedNameSite,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    // tsc resolves a type-only import or a UMD global here and reports the
    // use itself, so neither reaches the failure path below.
    if site != UnresolvedNameSite::TypeQuery
        && crate::checks::emit_value_position_reference_diagnostic(name, span, ctx)
    {
        return;
    }
    // Where a type-only import is a valid use, it resolves and is not reported.
    if ctx.type_only_alias_use_valid && ctx.type_only_value_reference(name).is_some() {
        return;
    }
    // Suppression goes by file, not by code: where this file's diagnostics
    // never reach the user, the plain form stands in for a message whose
    // spelling suggestion measures every name in scope.
    let plain = diagnostic_with_syntax_span(Diagnostic::ts2304(name, ctx.file_name.clone()), span);
    if !ctx.reports_diagnostic(&plain) || crate::infer::types::diagnostics_discarded() {
        ctx.push(plain);
        return;
    }
    let site = if site == UnresolvedNameSite::Reference && ctx.shorthand_property_depth > 0 {
        UnresolvedNameSite::ShorthandProperty
    } else {
        site
    };
    let diagnostic = invalid_initializer_reference_diagnostic(name, span, site, symbols, ctx)
        .unwrap_or_else(|| failed_value_name_diagnostic(name, site, None, symbols, ctx));
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}

/// Whether an identifier read of `name` resolves, as the identifier
/// inference looks it up: the scope chain, then the module's later bindings.
pub(crate) fn resolves_value_name(name: &str, symbols: &SymbolTable, ctx: &CheckerContext) -> bool {
    symbols.get(name).is_some()
        || ctx
            .module_value_fallback
            .as_ref()
            .is_some_and(|fallback| fallback.get(name).is_some())
}

/// tsc's `checkAndReportErrorForInvalidInitializer`, which the name resolver
/// runs ahead of `onFailedToResolveSymbol` for a reference inside a property
/// whose constructor declares the name: a missing `this.`/`C.` prefix, and
/// otherwise the reference the property cannot make.
fn invalid_initializer_reference_diagnostic(
    name: &str,
    span: Option<SyntaxTextSpan>,
    site: UnresolvedNameSite,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Diagnostic> {
    let span = span?;
    let property = crate::program::property_with_invalid_initializer(name, span, ctx)?;
    if site != UnresolvedNameSite::TypeQuery
        && let Some(diagnostic) = missing_member_prefix_diagnostic(name, symbols, ctx)
    {
        return Some(diagnostic);
    }
    Some(property.invalid_reference_diagnostic(name, span, ctx.file_name.clone()))
}

/// The operand of `typeof name` that resolves to nothing. The callers have
/// already settled the names tsc resolves (UMD globals, type-only classes).
pub(crate) fn unresolved_type_query_diagnostic(
    name: &str,
    span: Option<SyntaxTextSpan>,
    ctx: &CheckerContext,
) -> Diagnostic {
    if let Some(span) = span
        && let Some(property) = crate::program::property_with_invalid_initializer(name, span, ctx)
    {
        return property.invalid_reference_diagnostic(name, span, ctx.file_name.clone());
    }
    // A query's name is an expression (`checkTypeQuery` checks it), so it
    // fails to resolve as any value does: a name only a type has is TS2693.
    failed_value_name_diagnostic(name, UnresolvedNameSite::TypeQuery, None, &ctx.symbols, ctx)
}

/// tsc's `onFailedToResolveSymbol` for a value: `not_found` is the resolver's
/// `nameNotFoundMessage` when it has its own, which replaces only the plain
/// "Cannot find name" reports.
pub(crate) fn failed_value_name_diagnostic(
    name: &str,
    site: UnresolvedNameSite,
    not_found: Option<Diagnostic>,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Diagnostic {
    let file_name = ctx.file_name.clone();
    if !matches!(site, UnresolvedNameSite::TypeQuery | UnresolvedNameSite::Implicit)
        && let Some(diagnostic) = missing_member_prefix_diagnostic(name, symbols, ctx)
    {
        return diagnostic;
    }
    // `checkAndReportErrorForUsingNamespaceAsTypeOrValue`.
    if ctx.namespace_meaning(name) == Some(false) {
        return Diagnostic::ts2708(name, file_name);
    }
    // `checkAndReportErrorForUsingTypeAsValue`.
    if is_primitive_type_name(name) {
        return Diagnostic::ts2693(name, file_name);
    }
    let is_type_parameter = ctx
        .type_parameter_scopes
        .iter()
        .any(|scope| scope.contains_key(name));
    if is_type_parameter || ctx.lookup_type_declaration(name).is_some() {
        return if is_es2015_or_later_constructor_name(name) {
            Diagnostic::ts2585(name, file_name)
        } else {
            Diagnostic::ts2693(name, file_name)
        };
    }

    let message = cannot_find_name_message(name, site, ctx);
    if let Some(lib) = suggested_lib_for_nonexistent_name(name) {
        return not_found.unwrap_or_else(|| message.render(name, lib, file_name));
    }
    if let Some(suggestion) = suggested_value_name(name, symbols, ctx) {
        return Diagnostic::ts2552(name, suggestion, file_name);
    }
    not_found.unwrap_or_else(|| message.render(name, "", file_name))
}

/// tsc's `checkAndReportErrorForMissingPrefix`: a static member of any
/// enclosing class, then an instance member of the innermost one — the latter
/// only when that class's member is the `this` container, which surge reads off
/// the `this` binding (an arrow keeps it, a `function` rebinds it).
fn missing_member_prefix_diagnostic(
    name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<Diagnostic> {
    for (index, class) in ctx.enclosing_class_members.iter().enumerate().rev() {
        // A class surge could not model answers every name, which would turn
        // any unresolved name in its body into "did you mean the static
        // member".
        if is_degraded_class_type(&class.static_type)
            || is_degraded_class_type(&class.instance_type)
        {
            continue;
        }
        if has_property_of_type(&class.static_type, name, ctx) {
            return Some(Diagnostic::ts2662(
                name,
                &class.class_name,
                ctx.file_name.clone(),
            ));
        }
        let innermost = index + 1 == ctx.enclosing_class_members.len();
        if innermost
            && symbols
                .get("this")
                .is_some_and(|this| surge_ts_types::without_polymorphic_this(this.ty.clone()) == class.instance_type)
            && has_property_of_type(&class.instance_type, name, ctx)
        {
            return Some(Diagnostic::ts2663(name, ctx.file_name.clone()));
        }
    }
    None
}

fn is_degraded_class_type(ty: &Type) -> bool {
    matches!(ty, Type::Any | Type::ErrorType) || ty.is_unknown()
}

/// tsc's `getPropertyOfType`: a member the type declares or inherits, or one
/// its apparent type adds — `Function`'s for a callable or constructable
/// object, `Object`'s for every object — but never an index signature, which
/// a property read also answers from.
fn has_property_of_type(ty: &Type, name: &str, ctx: &CheckerContext) -> bool {
    match ty.peeled() {
        // Checker-injected openness stands in for members surge could not
        // enumerate (an expression base), any of which may be this one.
        Type::Object(object) if object.synthetic_open_index => {
            ty.get_property_access_type(name).is_some()
        }
        // The lib decides what `Function` carries: `name` is es2015's.
        Type::Object(object) => {
            object
                .get_property(name)
                .is_some_and(|property| !property.index_slot)
                || (object.call_signature().is_some() || object.construct_signature().is_some())
                    && crate::program::global_interface_declares("Function", name, ctx)
                || surge_ts_types::object_prototype_member_type(name).is_some()
        }
        other => other.get_property_access_type(name).is_some(),
    }
}

/// The message tsc's `getCannotFindNameDiagnosticForName` picks for `name`.
#[derive(Clone, Copy)]
pub(crate) enum CannotFindNameMessage {
    Plain,
    ShorthandProperty,
    AsyncFunction,
    Dom,
    NewerLib,
    Node { wildcard_types: bool },
    Jquery { wildcard_types: bool },
    TestRunner { wildcard_types: bool },
    Bun { wildcard_types: bool },
}

pub(crate) fn cannot_find_name_message(
    name: &str,
    site: UnresolvedNameSite,
    ctx: &CheckerContext,
) -> CannotFindNameMessage {
    let wildcard_types = ctx.options.types_uses_wildcard();
    match name {
        "document" | "console" => CannotFindNameMessage::Dom,
        "$" => CannotFindNameMessage::Jquery { wildcard_types },
        "beforeEach" | "describe" | "suite" | "it" | "test" => {
            CannotFindNameMessage::TestRunner { wildcard_types }
        }
        "process" | "require" | "Buffer" | "module" | "NodeJS" => {
            CannotFindNameMessage::Node { wildcard_types }
        }
        "Bun" => CannotFindNameMessage::Bun { wildcard_types },
        // tsc's list reads `"ast.Symbol"` (an artifact of its Go port), so
        // `Symbol` is not on it and keeps the plain message.
        "Map"
        | "Set"
        | "Promise"
        | "WeakMap"
        | "WeakSet"
        | "Iterator"
        | "AsyncIterator"
        | "SharedArrayBuffer"
        | "Atomics"
        | "AsyncIterable"
        | "AsyncIterableIterator"
        | "AsyncGenerator"
        | "AsyncGeneratorFunction"
        | "BigInt"
        | "Reflect"
        | "BigInt64Array"
        | "BigUint64Array" => CannotFindNameMessage::NewerLib,
        "await" if site == UnresolvedNameSite::Callee => CannotFindNameMessage::AsyncFunction,
        _ if site == UnresolvedNameSite::ShorthandProperty => {
            CannotFindNameMessage::ShorthandProperty
        }
        _ => CannotFindNameMessage::Plain,
    }
}

impl CannotFindNameMessage {
    /// tsc passes the suggested lib as a second argument to whichever message
    /// it picked; only TS2583 has a slot for it.
    pub(crate) fn render(self, name: &str, lib: &str, file_name: String) -> Diagnostic {
        match self {
            Self::Plain => Diagnostic::ts2304(name, file_name),
            Self::ShorthandProperty => Diagnostic::ts18004(name, file_name),
            Self::AsyncFunction => Diagnostic::ts2311(name, file_name),
            Self::Dom => Diagnostic::ts2584(name, file_name),
            Self::NewerLib => Diagnostic::ts2583(name, lib, file_name),
            Self::Node {
                wildcard_types: true,
            } => Diagnostic::ts2580(name, file_name),
            Self::Node {
                wildcard_types: false,
            } => Diagnostic::ts2591(name, file_name),
            Self::Jquery {
                wildcard_types: true,
            } => Diagnostic::ts2581(name, file_name),
            Self::Jquery {
                wildcard_types: false,
            } => Diagnostic::ts2592(name, file_name),
            Self::TestRunner {
                wildcard_types: true,
            } => Diagnostic::ts2582(name, file_name),
            Self::TestRunner {
                wildcard_types: false,
            } => Diagnostic::ts2593(name, file_name),
            Self::Bun {
                wildcard_types: true,
            } => Diagnostic::ts2867(name, file_name),
            Self::Bun {
                wildcard_types: false,
            } => Diagnostic::ts2868(name, file_name),
        }
    }
}

/// tsc's `isExportAssignmentExpressionName`: an `export =` or `export default`
/// target may legitimately name a type or a namespace, so the root name of one
/// is neither reported as used-as-a-value nor as missing
/// (`checkAndReportErrorForUsingTypeAsValue`,
/// `checkAndReportErrorForUsingNamespaceAsTypeOrValue`). A primitive type name
/// is still reported.
pub(crate) fn export_assignment_target_is_exempt(
    expression: &surge_ts_syntax::ParsedExpression,
    ctx: &CheckerContext,
) -> bool {
    // `IsValidTypeOnlyAliasUseSite`: the bare identifier of an export
    // assignment is not an expression node, so naming a type-only alias there
    // is not a use of its value (TS1361/TS1362). Parenthesized, it is one.
    if let surge_ts_syntax::ParsedExpression::Identifier { name, span } = expression
        && span.is_none_or(|span| ctx.parenthesized_outer_span(span).is_none())
        && ctx.type_only_value_reference(name).is_some()
    {
        return true;
    }
    let mut root = expression;
    let name = loop {
        match root {
            surge_ts_syntax::ParsedExpression::PropertyAccess { object, .. } => root = object,
            surge_ts_syntax::ParsedExpression::Identifier { name, .. } => break name.as_str(),
            _ => return false,
        }
    };
    if ctx.symbols.get(name).is_some() || is_primitive_type_name(name) {
        return false;
    }
    ctx.namespace_meaning(name).is_some()
        || ctx
            .type_parameter_scopes
            .iter()
            .any(|scope| scope.contains_key(name))
        || ctx.lookup_type_declaration(name).is_some()
}

pub(crate) fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "any" | "string" | "number" | "boolean" | "never" | "unknown"
    )
}

pub(crate) fn is_es2015_or_later_constructor_name(name: &str) -> bool {
    matches!(
        name,
        "Promise" | "Symbol" | "Map" | "WeakMap" | "Set" | "WeakSet"
    )
}

