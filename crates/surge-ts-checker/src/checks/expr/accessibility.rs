//! `private`/`protected` member access (tsc's `checkPropertyAccessibility`).

use std::cell::RefCell;
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedAccessorSide, ParsedClassDeclaration, ParsedExpression, ParsedMemberAccessibility,
    ParsedType, ParsedTypeParameter, TextSpan as SyntaxTextSpan,
};
use surge_ts_types::Type;

use crate::context::CheckerContext;
use crate::symbols::{InterfaceInfo, SymbolKind, SymbolTable, TypeDeclarationInfo};

use super::diagnostic_with_syntax_span;

/// A class declaration's identity: its file and the offset of its name. Names
/// alone are not enough — an import can rename a class, and two files can
/// declare the same one — so the name rides along for messages only.
#[derive(Clone, Debug)]
pub(crate) struct ClassIdentity {
    file: Arc<str>,
    start: usize,
    pub(crate) name: String,
}

impl PartialEq for ClassIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.start == other.start
    }
}

/// Inheritance chains are short; the bound only guards a malformed cycle.
const MAX_HERITAGE_DEPTH: usize = 32;

fn identity(info: &InterfaceInfo) -> Option<ClassIdentity> {
    Some(ClassIdentity {
        file: info.file_name.clone(),
        start: info.name_span?.start,
        name: display_name(info),
    })
}

fn display_name(info: &InterfaceInfo) -> String {
    info.declared_name
        .as_deref()
        .unwrap_or(&info.name)
        .to_string()
}

/// The class named by the `this` parameter of the function being checked
/// (tsc's `getEnclosingClassFromThisParameter`), with the classes it extends,
/// which it reaches the protected instance members of.
#[derive(Clone)]
pub(crate) struct ThisParameterClass {
    class: InterfaceInfo,
    lineage: Vec<ClassIdentity>,
}

thread_local! {
    static THIS_PARAMETER_CLASS: RefCell<Option<ThisParameterClass>> = const { RefCell::new(None) };
}

/// Makes `class` the `this`-parameter class until dropped. A function binds its
/// own `this`, so every function body enters one, `None` when it has no `this`
/// parameter; an arrow keeps its enclosing function's.
pub(crate) struct ThisParameterClassScope(Option<ThisParameterClass>);

impl ThisParameterClassScope {
    pub(crate) fn enter(class: Option<ThisParameterClass>) -> Self {
        Self(THIS_PARAMETER_CLASS.with(|current| current.replace(class)))
    }
}

impl Drop for ThisParameterClassScope {
    fn drop(&mut self) {
        let outer = self.0.take();
        THIS_PARAMETER_CLASS.with(|current| *current.borrow_mut() = outer);
    }
}

/// The class or interface a written `this` parameter names, the constraint's
/// when it names one of the function's type parameters.
pub(crate) fn this_parameter_class(
    written: Option<&ParsedType>,
    type_parameters: &[ParsedTypeParameter],
    ctx: &mut CheckerContext,
) -> Option<ThisParameterClass> {
    let mut written = written?;
    if let ParsedType::Named(named) = written
        && named.type_arguments.is_empty()
        && let Some(parameter) = type_parameters
            .iter()
            .find(|parameter| parameter.name == named.name)
    {
        written = parameter.constraint.as_ref()?;
    }
    // The annotation reports its own errors where the signature is resolved.
    let checkpoint = ctx.diagnostics().len();
    let ty = crate::checks::function::with_type_parameter_scope(type_parameters, ctx, |ctx| {
        crate::infer::map_parsed_type(written.clone(), ctx)
    });
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let class = instance_class(&ty, ctx)?;
    Some(ThisParameterClass {
        lineage: class_lineage(&class, ctx),
        class,
    })
}

pub(crate) fn base_interface(info: &InterfaceInfo, ctx: &CheckerContext) -> Option<InterfaceInfo> {
    base_interface_at(info, 0, ctx)
}

/// The `index`th base of an interface's `extends` list (a class has one).
fn base_interface_at(info: &InterfaceInfo, index: usize, ctx: &CheckerContext) -> Option<InterfaceInfo> {
    let base = info.body.extends.get(index)?;
    // The declaration's own scope answers first, but it carries only the layer
    // it was bound in: a base imported into the subclass's file is not in it,
    // and stopping there cut the lineage short — every `super.x` on a
    // `protected` member of a cross-module base read as out of reach.
    let declaration = info
        .resolution_scope
        .as_ref()
        .and_then(|scope| scope.get(&base.name))
        .or_else(|| ctx.lookup_type_declaration(&base.name));
    match declaration? {
        TypeDeclarationInfo::Interface(base) => Some(base.clone()),
        TypeDeclarationInfo::Alias(_) => None,
    }
}

/// The class being entered and every class it extends, pushed for the length
/// of its body so an access inside it can tell what it may reach.
pub(crate) fn enclosing_class_lineage(
    class: &ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) -> Vec<ClassIdentity> {
    let mut lineage = Vec::new();
    let Some(span) = class.name_span else {
        return lineage;
    };
    let own_identity = ClassIdentity {
        file: ctx.file_name_arc(),
        start: span.start,
        name: class.name.clone(),
    };
    let mut current = match ctx.lookup_type_declaration(&class.name) {
        Some(TypeDeclarationInfo::Interface(info)) => info.clone(),
        _ => {
            lineage.push(own_identity);
            return lineage;
        }
    };
    // A class merged with a same-named interface is one declaration, and a
    // member read resolves its owner to the merged record, which carries the
    // first fragment's name — the class's own span would never match it.
    lineage.push(
        identity(&current)
            .filter(|identity| *identity.file == *ctx.file_name)
            .unwrap_or(own_identity),
    );
    for _ in 0..MAX_HERITAGE_DEPTH {
        let Some(base) = base_interface(&current, ctx) else {
            break;
        };
        if let Some(identity) = identity(&base) {
            lineage.push(identity);
        }
        current = base;
    }
    lineage
}

/// The class that declares `member` restricted, found by walking up from the
/// receiver's class. A class along the way that declares the member without a
/// modifier makes it public from there on.
pub(crate) fn restricted_member_owner(
    receiver_class: &InterfaceInfo,
    member: &str,
    is_static: bool,
    is_write: bool,
    ctx: &CheckerContext,
) -> Option<(InterfaceInfo, ParsedMemberAccessibility)> {
    // The accessor side the access goes through; the other side's modifier
    // does not apply to it.
    let other_side = if is_write {
        ParsedAccessorSide::Get
    } else {
        ParsedAccessorSide::Set
    };
    // An interface reaches the member through any of its bases, in order.
    fn search(
        current: &InterfaceInfo,
        member: &str,
        is_static: bool,
        other_side: ParsedAccessorSide,
        depth: usize,
        ctx: &CheckerContext,
    ) -> Option<(InterfaceInfo, ParsedMemberAccessibility)> {
        if depth >= MAX_HERITAGE_DEPTH {
            return None;
        }
        if let Some(restricted) = current.body.restricted_members.iter().find(|restricted| {
            restricted.name == member
                && restricted.is_static == is_static
                && restricted.accessor_side != Some(other_side)
        }) {
            return Some((current.clone(), restricted.accessibility));
        }
        if !is_static && current.body.members.iter().any(|declared| declared.name == member) {
            return None;
        }
        (0..current.body.extends.len()).find_map(|index| {
            let base = base_interface_at(current, index, ctx)?;
            search(&base, member, is_static, other_side, depth + 1, ctx)
        })
    }
    search(receiver_class, member, is_static, other_side, 0, ctx)
}

/// tsc's `typeHasStaticProperty`: `receiver` is an instance of a class whose
/// constructor type has a property `name` declared `static`, by the class or
/// by one it extends. A namespace merged with the class adds properties to
/// that type too, but none of them static, and a private name is never one of
/// its keys. `None` when the receiver's class or a base of it cannot be looked
/// up.
pub(crate) fn class_has_static_member(
    receiver: &Type,
    name: &str,
    ctx: &CheckerContext,
) -> Option<bool> {
    if name.starts_with('#') {
        return Some(false);
    }
    let mut current = instance_class(receiver, ctx)?;
    for _ in 0..MAX_HERITAGE_DEPTH {
        if !current.is_class_instance {
            return Some(false);
        }
        if current
            .body
            .class_members
            .iter()
            .any(|member| member.is_static && member.name == name)
        {
            return Some(true);
        }
        if current.body.extends.is_empty() {
            return Some(false);
        }
        current = base_interface(&current, ctx)?;
    }
    None
}

/// The class a member read goes through, and whether the read is of its static
/// side. An instance is a reference to the class's declaration; the static side
/// is the class binding itself, written by name.
fn receiver_class(
    object: &ParsedExpression,
    receiver_type: &Type,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<(InterfaceInfo, bool)> {
    if let Type::Reference(_) = receiver_type {
        return instance_class(receiver_type, ctx).map(|class| (class, false));
    }
    if let Some(class) = static_receiver_class(object, symbols, ctx) {
        return Some((class, true));
    }
    instance_class(receiver_type, ctx).map(|class| (class, false))
}

/// tsc's `SymbolFlagsGetAccessor` on the member a read of `name` through
/// `object` resolves to, when the receiver is a class instance or a class: the
/// nearest class up the lineage that declares the name decides.
pub(crate) fn member_is_get_accessor(
    object: &ParsedExpression,
    receiver_type: &Type,
    name: &str,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> bool {
    let Some((mut current, is_static)) = receiver_class(object, receiver_type, symbols, ctx) else {
        return false;
    };
    for _ in 0..MAX_HERITAGE_DEPTH {
        if let Some(member) = current
            .body
            .class_members
            .iter()
            .find(|member| member.is_static == is_static && member.name == name)
        {
            return member.is_get_accessor;
        }
        // A parameter property, or a member of an interface merged with the class.
        if !is_static && current.body.members.iter().any(|member| member.name == name) {
            return false;
        }
        let Some(base) = base_interface(&current, ctx) else {
            return false;
        };
        current = base;
    }
    false
}

/// tsc's `isSpreadableProperty`: an object spread does not copy the methods
/// and accessors a class declares, its bases' included, unless a nearer
/// declaration makes the member a property.
pub(crate) fn class_unspreadable_members(ty: &Type, ctx: &CheckerContext) -> Vec<String> {
    let mut names = Vec::new();
    let Some(mut current) = instance_class(ty, ctx) else {
        return names;
    };
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_HERITAGE_DEPTH {
        if !current.is_class_instance {
            break;
        }
        for member in current.body.class_members.iter().filter(|member| !member.is_static) {
            if seen.insert(member.name.clone()) && !member.is_field {
                names.push(member.name.clone());
            }
        }
        for member in &current.body.members {
            seen.insert(member.name.clone());
        }
        let Some(base) = base_interface(&current, ctx) else {
            break;
        };
        current = base;
    }
    names
}

/// The class (or interface) whose instance `ty` is.
fn instance_class(ty: &Type, ctx: &CheckerContext) -> Option<InterfaceInfo> {
    if let Type::Reference(reference) = ty {
        let mut parts = reference.id.split('\u{0}');
        let file = parts.next()?;
        let name = parts.next_back()?;
        let TypeDeclarationInfo::Interface(info) = ctx.lookup_type_declaration(name)? else {
            return None;
        };
        // A different declaration that merely shares the name is not the
        // receiver's class.
        return (info.file_name.as_ref() == file).then(|| info.clone());
    }
    // Narrowing one of an instance's members materializes the instance as an
    // object that keeps only the class's name.
    // An instantiation (a generic class's own `C<T>` inside its body) keeps
    // its arguments in the name.
    if let Type::Object(object) = ty
        && let Some(name) = object.alias_name.as_deref()
        && let Some(TypeDeclarationInfo::Interface(info)) =
            ctx.lookup_type_declaration(name.split('<').next().unwrap_or(name))
    {
        return Some(info.clone());
    }
    None
}

/// The class a static read goes through: the class binding itself, written by
/// name. Looked up by name over the type table, so a binding that shadows the
/// class (a parameter or a variable holding some other value) does not denote
/// it.
fn static_receiver_class(
    object: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &CheckerContext,
) -> Option<InterfaceInfo> {
    let ParsedExpression::Identifier { name, .. } = object else {
        return None;
    };
    if matches!(
        symbols.get(name).map(|symbol| symbol.kind),
        Some(SymbolKind::Parameter | SymbolKind::Let | SymbolKind::Var) | None
    ) {
        return None;
    }
    match ctx.lookup_type_declaration(name)? {
        TypeDeclarationInfo::Interface(info) => Some(info.clone()),
        TypeDeclarationInfo::Alias(_) => None,
    }
}

/// tsc's `checkPropertyAccessibilityAtLocation` for a named member read: a
/// `private` member is reachable only inside the class that declares it
/// (TS2341), a `protected` one from a class deriving from it (TS2445) and only
/// through an instance of that class (TS2446).
pub(crate) fn check_member_accessibility(
    object: &ParsedExpression,
    receiver_type: &Type,
    member: &str,
    member_span: Option<SyntaxTextSpan>,
    is_write: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    // Accessibility belongs to the declaration, so a receiver narrowed by an
    // earlier check on one of its members (`if (!c.x) return; c.x = …`) is
    // judged by its declared type. `maybe?.x` reads the member off what the
    // optional chain leaves defined.
    let declared = match object {
        ParsedExpression::Identifier { name, .. } => symbols.declared_type(name),
        _ => None,
    };
    let receiver_type =
        surge_ts_types::without_polymorphic_this(surge_ts_types::remove_nullish(declared.unwrap_or(receiver_type)));
    // `this` whose type names no class — a type parameter's, or a function
    // expression's own — is the class its written `this` parameter names.
    let this_parameter_receiver = || {
        matches!(object, ParsedExpression::This { .. })
            .then(|| THIS_PARAMETER_CLASS.with(|current| current.borrow().as_ref().map(|this| this.class.clone())))
            .flatten()
            .map(|class| (class, false))
    };
    let Some((class, is_static)) =
        receiver_class(object, &receiver_type, symbols, ctx).or_else(this_parameter_receiver)
    else {
        check_intersection_member_accessibility(&receiver_type, member, member_span, is_write, ctx);
        return;
    };
    // An abstract member has no implementation for `super` to reach
    // (`checkPropertyAccessibilityAtLocation`); the nearest declaration of the
    // name decides.
    if !is_static
        && matches!(object, ParsedExpression::Identifier { name, .. } if name == "super")
        && let Some(declaring) = abstract_member_owner(&class, member, ctx)
    {
        let diagnostic = Diagnostic::ts2513(member, display_name(&declaring), ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, member_span));
        return;
    }
    // A class field lives on the instance, not the prototype `super` reads.
    if !is_static
        && matches!(object, ParsedExpression::Identifier { name, .. } if name == "super")
        && nearest_instance_member_is_field(&class, member, ctx)
    {
        let diagnostic = Diagnostic::ts2855(member, ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, member_span));
        return;
    }
    let Some((declaring, accessibility)) =
        restricted_member_owner(&class, member, is_static, is_write, ctx)
    else {
        return;
    };
    let Some(declaring_identity) = identity(&declaring) else {
        return;
    };
    let diagnostic = match accessibility {
        ParsedMemberAccessibility::Private => {
            if ctx
                .enclosing_classes
                .iter()
                .any(|lineage| lineage.first() == Some(&declaring_identity))
            {
                return;
            }
            Diagnostic::ts2341(member, display_name(&declaring), ctx.file_name.clone())
        }
        ParsedMemberAccessibility::Protected => {
            // A `super` access reaches every protected member of the base.
            if matches!(object, ParsedExpression::Identifier { name, .. } if name == "super") {
                return;
            }
            let Some(enclosing) = protected_access_class(&declaring_identity, is_static, ctx)
            else {
                let diagnostic =
                    Diagnostic::ts2445(member, display_name(&declaring), ctx.file_name.clone());
                ctx.push(diagnostic_with_syntax_span(diagnostic, member_span));
                return;
            };
            if is_static || class_lineage(&class, ctx).contains(&enclosing) {
                return;
            }
            Diagnostic::ts2446(member, &enclosing.name, receiver_type.name(), ctx.file_name.clone())
        }
    };
    ctx.push(diagnostic_with_syntax_span(diagnostic, member_span));
}

/// tsc's synthetic intersection property: protected only when every
/// constituent declaring it does so protected, and then reachable only from a
/// class derived from one of the declaring classes. The diagnostic names the
/// intersection, which declares the synthetic property.
fn check_intersection_member_accessibility(
    receiver_type: &Type,
    member: &str,
    member_span: Option<SyntaxTextSpan>,
    is_write: bool,
    ctx: &mut CheckerContext,
) {
    let Type::Object(object) = receiver_type.peeled() else {
        return;
    };
    let Some(operands) = object.intersection_operands.as_deref().filter(|_| object.is_intersection) else {
        return;
    };
    if !ctx.enclosing_classes.is_empty() || THIS_PARAMETER_CLASS.with(|current| current.borrow().is_some()) {
        return;
    }
    let mut protected = 0;
    for operand in operands {
        let declares = matches!(operand.peeled(), Type::Object(constituent) if constituent.get_property(member).is_some());
        if !declares {
            continue;
        }
        let Some(class) = instance_class(operand, ctx) else {
            return;
        };
        match restricted_member_owner(&class, member, false, is_write, ctx) {
            Some((_, ParsedMemberAccessibility::Protected)) => protected += 1,
            _ => return,
        }
    }
    if protected > 0 {
        let diagnostic = Diagnostic::ts2445(member, receiver_type.name(), ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(diagnostic, member_span));
    }
}

/// The class whose nearest declaration of an instance member is `abstract`.
fn nearest_instance_member_is_field(class: &InterfaceInfo, member: &str, ctx: &CheckerContext) -> bool {
    let mut current = class.clone();
    for _ in 0..MAX_HERITAGE_DEPTH {
        if let Some(declared) =
            current.body.class_members.iter().find(|declared| !declared.is_static && declared.name == member)
        {
            return declared.is_field;
        }
        if current.body.members.iter().any(|declared| declared.name == member) {
            return false;
        }
        let Some(base) = base_interface(&current, ctx) else {
            return false;
        };
        current = base;
    }
    false
}

fn abstract_member_owner(class: &InterfaceInfo, member: &str, ctx: &CheckerContext) -> Option<InterfaceInfo> {
    let mut current = class.clone();
    for _ in 0..MAX_HERITAGE_DEPTH {
        if let Some(declared) = current.body.members.iter().find(|declared| declared.name == member) {
            return declared.is_abstract.then_some(current);
        }
        current = base_interface(&current, ctx)?;
    }
    None
}

/// The class a protected member is reached from: the innermost enclosing class
/// that derives from the member's declaring class, or else — for an instance
/// member — the class the enclosing function's `this` parameter names.
fn protected_access_class(
    declaring: &ClassIdentity,
    is_static: bool,
    ctx: &CheckerContext,
) -> Option<ClassIdentity> {
    if let Some(lineage) = ctx
        .enclosing_classes
        .iter()
        .rev()
        .find(|lineage| lineage.contains(declaring))
    {
        return lineage.first().cloned();
    }
    if is_static {
        return None;
    }
    THIS_PARAMETER_CLASS.with(|current| {
        let current = current.borrow();
        let class = current.as_ref().filter(|class| class.lineage.contains(declaring))?;
        class.lineage.first().cloned()
    })
}

/// A class and every class it extends.
fn class_lineage(class: &InterfaceInfo, ctx: &CheckerContext) -> Vec<ClassIdentity> {
    let mut lineage = Vec::new();
    let mut current = class.clone();
    for _ in 0..MAX_HERITAGE_DEPTH {
        if let Some(identity) = identity(&current) {
            lineage.push(identity);
        }
        let Some(base) = base_interface(&current, ctx) else {
            break;
        };
        current = base;
    }
    lineage
}

/// The declaring class and modifier of the constructor `new C()` or
/// `class D extends C` would reach, when the current position may not: tsc's
/// `getConstructorAccessibilityError`. A class without a constructor inherits
/// its base's, so the walk goes up to the first class that writes one.
/// `protected_allowed_from_subclass` is the `new` rule; `extends` only refuses
/// a private constructor.
pub(crate) fn constructor_accessibility_error(
    class: &InterfaceInfo,
    protected_allowed_from_subclass: bool,
    ctx: &CheckerContext,
) -> Option<(InterfaceInfo, ParsedMemberAccessibility)> {
    let mut current = class.clone();
    for _ in 0..MAX_HERITAGE_DEPTH {
        if current.declares_constructor {
            break;
        }
        current = base_interface(&current, ctx)?;
    }
    let accessibility = current.constructor_accessibility?;
    let declaring_identity = identity(&current)?;
    let within_class = ctx
        .enclosing_classes
        .iter()
        .any(|lineage| lineage.first() == Some(&declaring_identity));
    if within_class {
        return None;
    }
    match accessibility {
        ParsedMemberAccessibility::Private => Some((current, accessibility)),
        ParsedMemberAccessibility::Protected => {
            if !protected_allowed_from_subclass {
                return None;
            }
            let from_subclass = ctx
                .enclosing_classes
                .last()
                .is_some_and(|lineage| lineage.contains(&declaring_identity));
            (!from_subclass).then_some((current, accessibility))
        }
    }
}
