//! Type-resolution diagnostic emitters (unknown name, arity, cycles).

use surge_ts_diagnostics::Diagnostic;
use std::sync::Arc;

use surge_ts_syntax::{ParsedNamedType, TextSpan};

use crate::context::{CheckerContext, convert_span};
use crate::symbols::TypeDeclarationInfo;

/// Returns whether the failure is one surge reports — the only case where the
/// name is known to be the source's error rather than a gap in what surge
/// resolved.
pub(crate) fn emit_unknown_type_name(
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    if ctx.suppress_unknown_type_name() {
        return false;
    }
    // Suppression goes by file, not by code. Where this file's diagnostics
    // never reach the user (`skipLibCheck` declarations, the lib), the plain
    // form stands in for the message, whose spelling suggestion measures every
    // declared name: a declaration file re-resolves the same unresolvable
    // reference thousands of times.
    let mut plain = Diagnostic::ts2304(&named_type.name, ctx.file_name.clone());
    if let Some(span) = named_type.span {
        plain = plain.with_span(convert_span(span));
    }
    if !ctx.reports_diagnostic(&plain) {
        ctx.push_utility_diagnostic_once(plain);
        return false;
    }
    // A lazy peel resolves in a context of its own that is dropped with its
    // diagnostics, so the suggestion would be measured for nothing.
    if crate::infer::types::cache::lazy_peel_depth() > 0 {
        ctx.push_utility_diagnostic_once(plain);
        return true;
    }
    let mut diagnostic = unknown_type_name_diagnostic(&named_type.name, ctx);
    if let Some(span) = named_type.span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    let reportable = ctx.reports_diagnostic(&diagnostic);
    ctx.push_utility_diagnostic_once(diagnostic);
    reportable
}

/// tsc's `onFailedToResolveSymbol` for a type reference.
fn unknown_type_name_diagnostic(name: &str, ctx: &CheckerContext) -> Diagnostic {
    use crate::checks::expr::{UnresolvedNameSite, cannot_find_name_message};

    let file_name = ctx.file_name.clone();
    // `checkAndReportErrorForUsingNamespaceAsTypeOrValue`, then
    // `checkAndReportErrorForUsingValueAsType`.
    if is_namespace_like(name, ctx) {
        return Diagnostic::ts2709(name, file_name);
    }
    if names_plain_value(name, ctx) {
        return Diagnostic::ts2749(name, file_name);
    }
    let message = cannot_find_name_message(name, UnresolvedNameSite::Reference, ctx);
    if let Some(lib) = crate::checks::expr::suggested_lib_for_nonexistent_name(name) {
        return message.render(name, lib, file_name);
    }
    if let Some(suggestion) = suggested_type_name(name, ctx) {
        return Diagnostic::ts2552(name, suggestion, file_name);
    }
    message.render(name, "", file_name)
}

/// Whether `name` resolves with a namespace meaning. surge binds every
/// namespace as a value and publishes its members under qualified `ns.member`
/// keys, so a name heading such a key — or a namespace import — is one. An
/// import alias of a module keys its members in the scope's import layers
/// (`import lib = require("lib")` inside a `declare module` block).
fn is_namespace_like(name: &str, ctx: &CheckerContext) -> bool {
    if ctx.is_namespace_import_binding(name)
        || ctx.namespace_meaning(name).is_some()
        || is_module_alias(name, ctx)
    {
        return true;
    }
    let heads_key = |candidate: &Arc<str>| {
        candidate
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('.'))
    };
    ctx.type_declarations.iter().any(|(key, _)| heads_key(key))
        || ctx.type_declaration_scope.as_ref().is_some_and(|scope| {
            scope
                .layers()
                .iter()
                .any(|layer| layer.iter().any(|(key, _)| heads_key(key)))
        })
        || ctx.ambient_global_type_declarations.iter().any(|(key, _)| heads_key(key))
        || ctx.symbols.iter().any(|(key, _)| heads_key(key))
        || ctx.ambient_global_symbols.iter().any(|(key, _)| heads_key(key))
}

/// An import alias of a module itself (`import m = require("./m")` over a
/// module with no `export =`), which tsc's `resolveName(…, Module)` finds: the
/// binding's value is the module's namespace object.
fn is_module_alias(name: &str, ctx: &CheckerContext) -> bool {
    ctx.is_import_binding(name)
        && ctx.symbols.get(name).is_some_and(|symbol| {
            matches!(&symbol.ty, surge_ts_types::Type::Object(object)
                if object
                    .alias_name
                    .as_deref()
                    .is_some_and(|alias| alias.starts_with("typeof import(\"")))
        })
}

/// `checkAndReportErrorForUsingValueAsType`: the name is a value and not a
/// namespace.
fn names_plain_value(name: &str, ctx: &CheckerContext) -> bool {
    // A declaration's own annotation is resolved before its value is bound
    // (`var x: x`); the module's value table already has it.
    ctx.symbols.get(name).is_some()
        || ctx.ambient_global_symbols.get(name).is_some()
        || ctx.module_value_fallback.as_ref().is_some_and(|table| table.get(name).is_some())
        || ctx
            .module_local_values_for_file(&ctx.file_name)
            .is_some_and(|table| table.get(name).is_some())
}

/// tsc's `resolveEntityName` for the head of a qualified type name
/// (`Head.Member…`) that surge could not resolve as a whole. The head is
/// looked up with a namespace meaning; only when nothing by that name could be
/// one is the failure reported. What the members of a real namespace are is
/// not modelled precisely enough to report anything past the head.
pub(crate) fn emit_unresolved_qualified_type_head(
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    if ctx.suppress_unknown_type_name()
        || !ctx.knows_file_imports()
        || !is_written_type_position(named_type, ctx)
    {
        return false;
    }
    let mut segments = named_type.name.split('.');
    let (Some(head), Some(member)) = (segments.next(), segments.next()) else {
        return false;
    };
    // A declaration file is judged only where its diagnostics reach the user
    // (a root file, not the lib or a `skipLibCheck` dependency).
    if crate::modules::is_declaration_file_name(&ctx.file_name)
        && !ctx.reports_diagnostic(&Diagnostic::ts2503(head, ctx.file_name.clone()))
    {
        return false;
    }
    if let Some((qualified_head, _)) = ctx.namespace_info(head) {
        return emit_missing_namespace_member(named_type, &qualified_head, ctx);
    }
    if ctx.is_complete_namespace_import_binding(head) {
        return emit_missing_module_namespace_member(named_type, head, member, ctx);
    }
    // A UMD global is its module's namespace (`export as namespace Foo`).
    if ctx.umd_global_names.contains_key(head) && ctx.symbols.get(head).is_some() {
        return emit_missing_module_namespace_member(named_type, head, member, ctx);
    }
    // `globalThis` and a UMD global are namespaces whose members surge does
    // not resolve here. `import x = require(…)` of an `export =` without
    // namespace meaning resolves to nothing as a namespace.
    let value_only_alias = ctx.is_value_only_import_alias(head);
    if ((is_namespace_like(head, ctx) || ctx.is_import_binding(head)) && !value_only_alias)
        || head == "globalThis"
        || ctx.umd_global_names.contains_key(head)
        || ctx
            .namespace_member_prefix_stack
            .iter()
            .any(|prefix| is_namespace_like(&format!("{prefix}.{head}"), ctx))
    {
        return false;
    }
    let file_name = ctx.file_name.clone();
    let is_type_parameter = ctx
        .type_parameter_scopes
        .iter()
        .any(|scope| scope.contains_key(head));
    // TS2713 underlines the qualified name; the rest underline its head.
    let (diagnostic, whole_name) =
        if is_type_parameter || ctx.lookup_type_declaration(head).is_some() {
            // `checkAndReportErrorForUsingTypeAsNamespace`.
            match declared_type_has_property(head, member, ctx) {
                Some(true) => (Diagnostic::ts2713(head, member, file_name), true),
                Some(false) => (Diagnostic::ts2702(head, file_name), false),
                None => return false,
            }
        } else if let Some(suggestion) = suggested_namespace_name(head, ctx) {
            (Diagnostic::ts2833(head, suggestion, file_name), false)
        } else {
            (Diagnostic::ts2503(head, file_name), false)
        };
    let mut diagnostic = diagnostic;
    if let Some(span) = named_type.span {
        let span = if whole_name {
            TextSpan {
                start: span.start,
                end: span.start + head.len() + 1 + member.len(),
            }
        } else {
            TextSpan {
                start: span.start,
                end: span.start + head.len(),
            }
        };
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    let reportable = ctx.reports_diagnostic(&diagnostic);
    ctx.push_utility_diagnostic_once(diagnostic);
    reportable
}

/// tsc's `resolveQualifiedName` failure past a namespace head: the first
/// segment the namespace's exports do not carry with the meaning needed there
/// (a namespace for a middle segment, a type for the last). Only a namespace
/// whose whole export list surge saw is judged.
fn emit_missing_namespace_member(
    named_type: &ParsedNamedType,
    qualified_head: &str,
    ctx: &mut CheckerContext,
) -> bool {
    use crate::program::{MEANING_NAMESPACE, MEANING_TYPE, MEANING_VALUE};

    if !is_written_type_position(named_type, ctx) {
        return false;
    }

    let segments: Vec<&str> = named_type.name.split('.').collect();
    let mut namespace = qualified_head.to_string();
    // Byte offset of the current segment within the written name.
    let mut offset = segments[0].len() + 1;
    for (index, segment) in segments.iter().enumerate().skip(1) {
        let is_last = index + 1 == segments.len();
        let Some(info) = ctx.namespace_registry.info(&ctx.file_name, &namespace) else {
            return false;
        };
        if !info.complete {
            return false;
        }
        let needed = if is_last { MEANING_TYPE } else { MEANING_NAMESPACE };
        let meanings = info.exports.get(*segment).copied().unwrap_or(0);
        if meanings & needed != 0 {
            namespace = format!("{namespace}.{segment}");
            offset += segment.len() + 1;
            continue;
        }
        let file_name = ctx.file_name.clone();
        let segment_span = named_type.span.map(|span| TextSpan {
            start: span.start + offset,
            end: span.start + offset + segment.len(),
        });
        let suggestion = crate::checks::expr::spelling_suggestion(
            segment,
            info.exports
                .iter()
                .filter(|(_, meanings)| **meanings & needed != 0)
                .map(|(name, _)| name.as_ref()),
            0,
        )
        .map(str::to_string);
        // The whole name is an import alias's target, reported where the
        // alias is declared.
        if named_type.span.is_some_and(|span| span.start == span.end) {
            return true;
        }
        let (diagnostic, span) = if let Some(suggestion) = suggestion {
            (
                Diagnostic::ts2724(&namespace, segment, suggestion, file_name),
                segment_span,
            )
        } else if is_last && meanings & MEANING_VALUE != 0 {
            // `canSuggestTypeof`: the qualified name reads a value.
            let whole = segments.join(".");
            let span = named_type.span.map(|span| TextSpan {
                start: span.start,
                end: span.start + whole.len(),
            });
            (Diagnostic::ts2749(whole, file_name), span)
        } else if !is_last && meanings & MEANING_TYPE != 0 {
            let next = segments[index + 1];
            let next_span = named_type.span.map(|span| {
                let start = span.start + offset + segment.len() + 1;
                TextSpan {
                    start,
                    end: start + next.len(),
                }
            });
            (Diagnostic::ts2713(segment, next, file_name), next_span)
        } else {
            (Diagnostic::ts2694(&namespace, segment, file_name), segment_span)
        };
        let diagnostic = match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };
        ctx.push_utility_diagnostic_once(diagnostic);
        return true;
    }
    false
}

/// `resolveQualifiedName` past a namespace import (`import * as ts from "m"`)
/// of a module whose whole export list surge saw: the first member is
/// neither an exported type or namespace nor, where tsc's `canSuggestTypeof`
/// looks, a value. The namespace is named by its module's quoted path.
fn emit_missing_module_namespace_member(
    named_type: &ParsedNamedType,
    head: &str,
    member: &str,
    ctx: &mut CheckerContext,
) -> bool {
    let qualified = format!("{head}.{member}");
    if ctx.lookup_type_declaration(&qualified).is_some() || is_namespace_like(&qualified, ctx) {
        return false;
    }
    let Some(surge_ts_types::Type::Object(namespace)) =
        ctx.symbols.get(head).map(|symbol| symbol.ty.clone())
    else {
        return false;
    };
    let Some(module_path) = namespace
        .alias_name
        .as_deref()
        .and_then(|alias| alias.strip_prefix("typeof import(\""))
        .and_then(|rest| rest.strip_suffix("\")"))
    else {
        return false;
    };
    let namespace_name = format!("\"{module_path}\"");
    let file_name = ctx.file_name.clone();
    let is_last = named_type.name.split('.').count() == 2;
    let member_span = named_type.span.map(|span| TextSpan {
        start: span.start + head.len() + 1,
        end: span.start + head.len() + 1 + member.len(),
    });
    let is_value = namespace.properties.get(member).is_some();
    // A value read past it (`core.published.Widget`) may be an exported
    // import alias of a namespace, whose members this table does not key —
    // not in a JavaScript module, which has no such alias.
    let javascript = ctx.is_javascript_namespace_import_binding(head);
    if is_value && !is_last && !javascript {
        return false;
    }
    // A JavaScript module's class-valued export declares the class, which is
    // a type as well.
    if javascript
        && is_last
        && namespace.properties.get(member).is_some_and(|property| {
            matches!(property.ty.peeled(), surge_ts_types::Type::Object(object) if object.construct_signature().is_some())
        })
    {
        return false;
    }
    if named_type.span.is_some_and(|span| span.start == span.end) {
        return true;
    }
    let (diagnostic, span) = if is_value && is_last {
        let span = named_type.span.map(|span| TextSpan {
            start: span.start,
            end: span.start + qualified.len(),
        });
        (Diagnostic::ts2749(&qualified, file_name), span)
    } else {
        let exported_names = module_namespace_export_names(head, &namespace, ctx);
        match crate::checks::expr::spelling_suggestion(
            member,
            exported_names.iter().map(String::as_str),
            0,
        ) {
            Some(suggestion) => (
                Diagnostic::ts2724(&namespace_name, member, suggestion, file_name),
                member_span,
            ),
            None => (
                Diagnostic::ts2694(&namespace_name, member, file_name),
                member_span,
            ),
        }
    };
    let diagnostic = match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };
    ctx.push_utility_diagnostic_once(diagnostic);
    true
}

/// A namespace import's exports as tsc's `getExportsOfModuleAsArray` lists
/// them for a spelling suggestion: the module object's values and the types
/// its alias layer keys under `head.`.
fn module_namespace_export_names(
    head: &str,
    namespace: &surge_ts_types::ObjectType,
    ctx: &CheckerContext,
) -> Vec<String> {
    let prefix = format!("{head}.");
    let mut names: Vec<String> = namespace
        .properties
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    if let Some(scope) = ctx.type_declaration_scope.as_ref() {
        for layer in scope.layers() {
            for (key, _) in layer.iter() {
                if let Some(name) = key.strip_prefix(prefix.as_str()) {
                    let name = name.split('.').next().unwrap_or(name);
                    if !names.iter().any(|existing| existing == name) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    names
}

/// Whether a qualified name is one written in a type position, where a
/// failure to resolve it through namespaces is the source's error. A name with
/// no span is surge's own lookup (the JSX namespace, a heritage re-resolution),
/// and a class's `extends` base is an expression, resolved and reported as a
/// value by the class check.
fn is_written_type_position(named_type: &ParsedNamedType, ctx: &CheckerContext) -> bool {
    named_type.span.is_some()
        && !(ctx.resolving_class_heritage
            && crate::program::current_dts_expansion_reason()
                == crate::program::DtsExpansionReason::InterfaceHeritageResolution)
}

/// A written qualified name (`N.Hidden`) that surge's type table resolves
/// although the namespace does not export the member: tsc reads only a
/// namespace's exports through a qualified name. Reports and answers whether
/// it did. Inside the namespace itself a qualified name still reads only its
/// exports (`namespace N { class C {} var c: N.C }`).
pub(crate) fn report_unexported_namespace_member(
    named_type: &ParsedNamedType,
    ctx: &mut CheckerContext,
) -> bool {
    if !named_type.name.contains('.')
        || ctx.suppress_unknown_type_name()
        || !ctx.knows_file_imports()
        || crate::modules::is_declaration_file_name(&ctx.file_name)
    {
        return false;
    }
    let head = named_type.name.split('.').next().unwrap_or_default();
    let Some((qualified_head, _)) = ctx.namespace_info(head) else {
        return false;
    };
    // A nested namespace its parent does not export is in scope only inside
    // the block that declares it, which the registry does not record.
    if let Some((parent, _)) = qualified_head.rsplit_once('.')
        && ctx.namespace_registry.info(&ctx.file_name, parent).is_none_or(|info| {
            info.exports.get(head).copied().unwrap_or(0) & crate::program::MEANING_NAMESPACE == 0
        })
    {
        return false;
    }
    emit_missing_namespace_member(named_type, &qualified_head, ctx)
}

/// Whether the declared type of `name` has a property `member`, as far as a
/// written object shape answers it; `None` for anything surge would have to
/// resolve to know.
fn declared_type_has_property(name: &str, member: &str, ctx: &CheckerContext) -> Option<bool> {
    // An unconstrained type parameter's apparent type is `{}`, whose members
    // are only `Object`'s.
    if ctx.type_parameter_scopes.iter().any(|scope| scope.contains_key(name)) {
        const OBJECT_MEMBERS: [&str; 7] = [
            "constructor",
            "toString",
            "toLocaleString",
            "valueOf",
            "hasOwnProperty",
            "isPrototypeOf",
            "propertyIsEnumerable",
        ];
        return (!ctx.type_parameter_has_constraint(name) && !OBJECT_MEMBERS.contains(&member)).then_some(false);
    }
    match ctx.lookup_type_declaration(name)? {
        TypeDeclarationInfo::Interface(info) => {
            if info.body.members.iter().any(|candidate| candidate.name == member) {
                Some(true)
            } else if info.body.extends.is_empty() {
                Some(false)
            } else {
                None
            }
        }
        TypeDeclarationInfo::Alias(alias) => match &alias.body.ty {
            surge_ts_syntax::ParsedType::Object(object) => {
                Some(object.properties.iter().any(|property| property.name == member))
            }
            _ => None,
        },
    }
}

/// The closest name heading a namespace, for TS2833.
fn suggested_namespace_name(name: &str, ctx: &CheckerContext) -> Option<String> {
    let head = |key: &Arc<str>| key.split_once('.').map(|(head, _)| head.to_string());
    let mut candidates: Vec<String> = ctx
        .type_declarations
        .iter()
        .chain(ctx.ambient_global_type_declarations.iter())
        .filter_map(|(key, _)| head(key))
        .filter(|candidate| !candidate.contains('\0'))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    crate::checks::expr::spelling_suggestion(name, candidates.iter().map(String::as_str), 0)
        .map(str::to_string)
}

/// The closest declared type name, as tsc's `resolveNameHelper` suggests for a
/// type reference. A qualified or file-keyed table entry is not a name a bare
/// reference could have meant.
fn suggested_type_name(name: &str, ctx: &CheckerContext) -> Option<String> {
    if name.contains('.') {
        return None;
    }
    // `resolveNameHelper` walks the enclosing namespaces' locals too; surge
    // keys their members `ns.Member`.
    let prefixes = &ctx.namespace_member_prefix_stack;
    // Screening before the sort leaves it only the names the suggestion could
    // pick; their order is unchanged.
    let screen = crate::checks::expr::SpellingScreen::new(name);
    let mut candidates: Vec<&str> = ctx
        .type_declarations
        .iter()
        .chain(ctx.ambient_global_type_declarations.iter())
        .map(|(candidate, _)| candidate.as_ref())
        .filter_map(|candidate| {
            if candidate.contains(['.', '\0']) { namespace_member(candidate, prefixes) } else { Some(candidate) }
        })
        .chain(
            ctx.type_parameter_scopes
                .iter()
                .flat_map(|scope| scope.keys().map(String::as_str)),
        )
        .filter(|candidate| screen.admits(candidate))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    crate::checks::expr::spelling_suggestion(name, candidates, 0).map(str::to_string)
}

fn namespace_member<'a>(candidate: &'a str, prefixes: &[String]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| {
        candidate
            .strip_prefix(prefix.as_str())?
            .strip_prefix('.')
            .filter(|member| !member.contains(['.', '\0']))
    })
}

pub(crate) fn emit_type_is_not_generic(
    name: &str,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let mut diagnostic = Diagnostic::ts2315(name, ctx.file_name.clone());
    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push_utility_diagnostic_once(diagnostic);
}

/// tsc's wrong-argument-count error on a generic type reference: TS2314 when
/// every type parameter is required, TS2707 when some have defaults. It is
/// reported on the reference; a lookup surge synthesized has none, and a
/// report there would land on whatever declaration it named.
pub(crate) fn emit_generic_arity(
    name: &str,
    min_type_argument_count: usize,
    type_parameter_count: usize,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let Some(span) = name_span else {
        return;
    };
    let name = &crate::checks::expr::bare_type_name(name);
    let file_name = ctx.file_name.clone();
    let diagnostic = if is_javascript_heritage_reference(ctx) {
        // `isJsImplicitAny`: a JavaScript reference fills what it lacks with
        // `any` and reports nothing without `noImplicitAny`.
        if !ctx.options.no_implicit_any {
            return;
        }
        if min_type_argument_count == type_parameter_count {
            Diagnostic::ts8026(name, file_name)
        } else {
            Diagnostic::ts8027(name, min_type_argument_count, file_name)
        }
    } else if min_type_argument_count == type_parameter_count {
        Diagnostic::ts2314(name, type_parameter_count, file_name)
    } else {
        Diagnostic::ts2707(name, min_type_argument_count, type_parameter_count, file_name)
    };
    ctx.push_utility_diagnostic_once(diagnostic.with_span(convert_span(span)));
}

/// `missingAugmentsTag`: the reference is a JavaScript class's `extends`
/// clause, whose type arguments only an `@extends` tag can supply. The
/// reparser moves the tag's arguments onto the clause, so the clause is the
/// reference either way.
pub(crate) fn is_javascript_heritage_reference(ctx: &CheckerContext) -> bool {
    ctx.resolving_class_heritage
        && crate::program::current_dts_expansion_reason()
            == crate::program::DtsExpansionReason::InterfaceHeritageResolution
        && surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
}

/// `Foo<Bad>` where `Foo`'s parameter is constrained. Only reported when both the
/// argument and the constraint resolved to real, settled shapes: the whole point
/// of the degradation sentinel is that surge cannot tell "violates the
/// constraint" from "I could not model this", and reporting on the latter turns
/// every modelling gap into a false positive.
///
/// `constraint_name` is the constraint *as written* where one can be rendered:
/// tsc names `keyof User` that way, not as the literal union it expands to, and
/// `Pick`'s own check in `utility.rs` already reports the written form. Two
/// spellings of one constraint are two dedup keys, so the same violation was
/// reported twice.
///
/// tsc checks the constraints of a type reference's written arguments
/// (`checkTypeReferenceNode`); an instantiation surge synthesized has no
/// reference, and tsc reports nothing for it.
pub(crate) fn emit_type_argument_constraint(
    argument_name: &str,
    constraint_name: &str,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let Some(span) = name_span else {
        return;
    };
    let diagnostic = Diagnostic::ts2344(argument_name, constraint_name, ctx.file_name.clone());
    ctx.push_utility_diagnostic_once(diagnostic.with_span(convert_span(span)));
}

pub(crate) fn emit_type_declaration_cycle(
    name: &str,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let mut diagnostic = Diagnostic::surge_type_declaration_cycle(name, ctx.file_name.clone());

    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push_utility_diagnostic_once(diagnostic);
}

pub(crate) fn emit_type_alias_cycle(
    name: &str,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    let mut diagnostic = Diagnostic::surge_type_alias_cycle(name, ctx.file_name.clone());

    if let Some(span) = name_span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }

    ctx.push_utility_diagnostic_once(diagnostic);
}
