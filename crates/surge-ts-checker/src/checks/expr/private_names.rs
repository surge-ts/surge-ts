//! Member access through a private name (`o.#x`): the private-identifier
//! branch of tsc's `checkPropertyAccessExpressionOrQualifiedName`, with
//! `checkPrivateIdentifierPropertyAccess` and the part of
//! `reportNonexistentProperty` a private name reaches. The lowering has already
//! resolved the name against the enclosing class bodies
//! (`lookupSymbolForPrivateIdentifierDeclaration`); see
//! `surge_ts_types::private_name` for what the key records.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::TextSpan as SyntaxTextSpan;
use surge_ts_types::{Type, private_name};

use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::symbols::SymbolTable;

use super::diagnostic_with_syntax_span;

/// The receiver of a private-name access.
pub(crate) enum PrivateNameReceiver<'a> {
    Type(&'a Type),
    /// A receiver tsc reads as its error type: a name or member that did not
    /// resolve.
    Error,
}

impl<'a> PrivateNameReceiver<'a> {
    /// `None` for a receiver surge could not model, which reports nothing.
    pub(crate) fn of(receiver: &'a InferredExpression) -> Option<Self> {
        match receiver {
            InferredExpression::Known(ty) => Some(Self::Type(ty)),
            InferredExpression::UnresolvedIdentifier { .. } | InferredExpression::MissingProperty { .. } => {
                Some(Self::Error)
            }
            InferredExpression::Unknown => None,
        }
    }
}

/// Checks `receiver.<key>` for the private-name `key`. `None` when the access
/// goes on like any other — the receiver has the member, or is `any` and the
/// name is declared; otherwise the type the access has once reported: tsc's
/// error type, or `any` for a name outside every class body.
pub(crate) fn check_private_name_access(
    receiver: PrivateNameReceiver<'_>,
    key: &str,
    property_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let description = private_name::display(key);
    let lexical = private_name::declaring_class(key);
    let receiver = match receiver {
        PrivateNameReceiver::Type(ty) => Some(surge_ts_types::remove_nullish(ty)),
        PrivateNameReceiver::Error => None,
    };
    // `isAnyLike`: `any`, or the error type an unresolved receiver is read as —
    // which is also what `checkNonNullExpression` leaves of an `unknown` one
    // under `strictNullChecks`.
    let any_like = match &receiver {
        None | Some(Type::Any | Type::ErrorType) => true,
        Some(Type::GenuineUnknown) => surge_ts_types::strict_null_checks(),
        Some(_) => false,
    };
    if any_like {
        // A declared name reads off `any` like any other member does.
        if lexical.is_some() {
            return None;
        }
        if private_name::is_outside_class_body(key) {
            let diagnostic = Diagnostic::ts18016(ctx.file_name.clone());
            ctx.push(diagnostic_with_syntax_span(diagnostic, property_span));
            return Some(Type::Any);
        }
        let diagnostic = Diagnostic::ts2339(description, "any", ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, property_span));
        return Some(Type::ErrorType);
    }
    let receiver = receiver?;
    let peeled = receiver.peeled();
    // A type variable, or a shape surge could not rebuild, answers through the
    // ordinary lookup, which reports nothing it cannot see.
    if matches!(peeled, Type::Unknown | Type::TypeParameter(_)) {
        return None;
    }
    // A static private member is its own class's alone: a subclass
    // constructor inherits the property but not the brand (`Child.#bar`).
    let subclass_constructor = match (&lexical, &peeled) {
        (Some(lexical), Type::Object(object)) => object
            .alias_name
            .as_deref()
            .and_then(|alias| alias.strip_prefix("typeof "))
            .is_some_and(|class_name| !lexical.name.is_empty() && class_name != lexical.name),
        _ => false,
    };
    if subclass_constructor && receiver.get_property_access_type(key).is_some() {
        let diagnostic = super::missing_property_diagnostic(description, &receiver, symbols, ctx);
        ctx.push(diagnostic_with_syntax_span(diagnostic, property_span));
        return Some(Type::ErrorType);
    }
    if lexical.is_some() && receiver.get_property_access_type(key).is_some() {
        return None;
    }
    let object = match &peeled {
        Type::Object(object) => Some(object),
        _ => None,
    };
    // `checkPrivateIdentifierPropertyAccess`: a member of the receiver spelled
    // the same belongs to another class body.
    let same_spelling = object.and_then(|object| {
        object.properties.keys().find_map(|name| {
            let name: &str = name;
            (private_name::display(name) == description)
                .then(|| private_name::declaring_class(name))
                .flatten()
        })
    });
    if let Some(owner) = same_spelling {
        let diagnostic = match &lexical {
            // The class body that declares the name written here sits inside
            // the one the receiver's member comes from, and shadows it.
            Some(lexical) if owner.encloses(lexical) => {
                Diagnostic::ts18014(description, receiver.name(), ctx.file_name.clone())
            }
            _ => Diagnostic::ts18013(description, owner.name, ctx.file_name.clone()),
        };
        ctx.push(diagnostic_with_syntax_span(diagnostic, property_span));
        return Some(Type::ErrorType);
    }
    // An object surge could not enumerate may hold the member after all, and
    // a JavaScript literal (`isJSLiteralType`) reads any it lacks as `any`.
    let javascript_literal = |object: &surge_ts_types::ObjectType| {
        !ctx.options.no_implicit_any
            && surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
            && object.alias_name.is_none()
            && object.string_index_type.as_deref() == Some(&Type::Any)
    };
    if object.is_some_and(|object| object.synthetic_open_index || javascript_literal(object)) {
        return Some(Type::Any);
    }
    let diagnostic = super::missing_property_diagnostic(description, &receiver, symbols, ctx);
    ctx.push(diagnostic_with_syntax_span(diagnostic, property_span));
    Some(Type::ErrorType)
}
