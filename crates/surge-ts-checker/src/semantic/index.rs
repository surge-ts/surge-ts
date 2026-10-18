//! What checking one file computed, keyed by source span.
//!
//! A retained program builds a file's index by checking the file again with
//! the recorder on: the hooks below are the checker's own results at the
//! points it computes them, so a query reads what the checker decided rather
//! than a second implementation of it. With the recorder off (every normal
//! run) each hook is one relaxed atomic load.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use surge_ts_syntax::{ParsedExpression, ParsedStatement, TextSpan};
use surge_ts_types::Type;
use surge_ts_types::fx::FxHashMap;

use crate::context::CheckerContext;
use crate::program::FunctionDeclarationLocation;
use crate::symbols::{SymbolTable, TypeDeclarationScope, TypeDeclarationTable};

static RECORDING: AtomicBool = AtomicBool::new(false);
/// Set once the compiler API checks a program: from then on the checker
/// answers what only typescript@6 (the API's target) differs in from the
/// native compiler the diagnostics follow, and keeps what the API reads off
/// its types (where an object literal declares a member).
static API_PROGRAM: AtomicBool = AtomicBool::new(false);

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

struct Recorder {
    file_name: String,
    index: FileSemanticIndex,
}

type Span = (usize, usize);

/// The types the checker computed while checking one file, by byte span.
#[derive(Default)]
pub struct FileSemanticIndex {
    expressions: FxHashMap<Span, Type>,
    /// A call's result, by its callee's span: the checker keeps no span of
    /// the whole call for most call shapes.
    call_results: FxHashMap<Span, Type>,
    /// A declaration's type, by its name's span.
    declarations: FxHashMap<Span, Type>,
    /// An element access's (`o[k]`) type, by its object's start and its
    /// argument's end: the lowered tree keeps no span of the closing bracket.
    elements: FxHashMap<Span, Type>,
    /// The type scope a function body's local type declarations resolve in,
    /// by each one's name span.
    local_type_scopes: FxHashMap<Span, Arc<TypeDeclarationScope>>,
    /// The type an expression was checked against (tsc's contextual type),
    /// by its span.
    contextual: FxHashMap<Span, Type>,
    /// A generic call's argument's parameter in the signature the call was
    /// instantiated to, which tsc's contextual type is.
    instantiated_contextual: FxHashMap<Span, Type>,
    /// A function expression's own signature where the checker relates it as
    /// another (a contextual `void` return), by its span.
    own_functions: FxHashMap<Span, Type>,
    /// Spans whose recorded type is final: a literal's own type, which the
    /// contextual evaluation that follows reports as its target's.
    sealed: surge_ts_types::fx::FxHashSet<Span>,
    pub(crate) scope: Option<FileScope>,
}

/// The scope the file's statements were checked in, for resolving a type
/// name as the file sees it.
pub(crate) struct FileScope {
    pub(crate) type_declarations: TypeDeclarationTable,
    pub(crate) type_declaration_scope: Option<Arc<TypeDeclarationScope>>,
    pub(crate) symbols: SymbolTable,
}

impl FileSemanticIndex {
    pub fn expression_type(&self, start: usize, end: usize) -> Option<&Type> {
        self.expressions.get(&(start, end))
    }

    pub fn call_result_type(&self, callee_start: usize, callee_end: usize) -> Option<&Type> {
        self.call_results.get(&(callee_start, callee_end))
    }

    pub fn declaration_type(&self, name_start: usize, name_end: usize) -> Option<&Type> {
        self.declarations.get(&(name_start, name_end))
    }

    /// The declaration recorded at the first name of `name_len` bytes
    /// written at or after `member_start`: a class member's name, which
    /// follows its modifiers.
    pub(crate) fn member_declaration_type(&self, member_start: usize, name_len: usize) -> Option<&Type> {
        self.declarations
            .iter()
            .filter(|((start, end), _)| *start >= member_start && *start - member_start < 64 && end - start == name_len)
            .min_by_key(|((start, _), _)| *start)
            .map(|(_, ty)| ty)
    }

    pub(crate) fn local_type_scope(&self, name_start: usize, name_end: usize) -> Option<&Arc<TypeDeclarationScope>> {
        self.local_type_scopes.get(&(name_start, name_end))
    }

    pub fn contextual_type(&self, start: usize, end: usize) -> Option<&Type> {
        self.instantiated_contextual.get(&(start, end)).or_else(|| self.contextual.get(&(start, end)))
    }

    pub fn element_access_type(&self, object_start: usize, argument_end: usize) -> Option<&Type> {
        self.elements.get(&(object_start, argument_end))
    }

    pub fn recorded_count(&self) -> usize {
        self.expressions.len() + self.call_results.len() + self.declarations.len() + self.elements.len()
    }

    fn insert(map: &mut FxHashMap<Span, Type>, span: Span, ty: &Type) {
        // A later degraded evaluation of the same span (a re-check without the
        // context the first one had) never replaces a modelled type.
        if ty.is_degraded() && map.get(&span).is_some_and(|existing| !existing.is_degraded()) {
            return;
        }
        map.insert(span, ty.clone());
    }

    /// [`Self::insert`] for an expression or parameter a callback's re-check
    /// without the signature it was passed to types `any`: that never
    /// replaces the contextual type the first check had.
    fn insert_contextual(map: &mut FxHashMap<Span, Type>, span: Span, ty: &Type) {
        if lost_context(ty) && map.get(&span).is_some_and(|existing| !lost_context(existing) && !existing.is_degraded()) {
            return;
        }
        Self::insert(map, span, ty);
    }
}

/// `any`, or a function whose every parameter is: what a contextually typed
/// callback or parameter checks as without its context.
fn lost_context(ty: &Type) -> bool {
    match ty {
        Type::Any => true,
        Type::Function(function) => {
            !function.parameters().is_empty() && function.parameters().iter().all(|parameter| matches!(parameter, Type::Any))
        }
        _ => false,
    }
}

#[inline]
pub(crate) fn recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

pub(crate) fn mark_api_program() {
    API_PROGRAM.store(true, Ordering::Relaxed);
}

/// The checker serves the compiler API (see [`API_PROGRAM`]).
pub(crate) fn serves_api() -> bool {
    API_PROGRAM.load(Ordering::Relaxed) || recording()
}

/// While serving the API, a read that cycles back into the type being
/// declared (`surge_ts_syntax::cycle_read`) is marked as such: the API prints
/// it as tsc's printer prints the cycle, and every type operation sees `ty`.
pub(crate) fn mark_cycle_read(span: Option<surge_ts_syntax::TextSpan>, ty: Type, ctx: &CheckerContext) -> Type {
    if !serves_api() {
        return ty;
    }
    let Some(span) = span else { return ty };
    match surge_ts_syntax::cycle_read(&ctx.file_name, span.start as u32) {
        Some((start, end)) => surge_ts_types::cycle_marker(ty, &format!("{}\u{3}{start}\u{3}{end}", ctx.file_name)),
        None => ty,
    }
}

fn with_index(ctx: &CheckerContext, f: impl FnOnce(&mut FileSemanticIndex)) {
    RECORDER.with(|recorder| {
        if let Ok(mut recorder) = recorder.try_borrow_mut()
            && let Some(recorder) = recorder.as_mut()
            && recorder.file_name == ctx.file_name
        {
            f(&mut recorder.index);
        }
    });
}

/// Records an inferred expression under its own span, and a call's result
/// under its callee's.
pub(crate) fn record_expression_type(expression: &ParsedExpression, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    // tsc checks an object literal's members in a mutable location
    // (`checkExpressionForMutableLocation`), widening their literals, where
    // this checker keeps them for the declaration to widen.
    let widened;
    let ty = if matches!(expression, ParsedExpression::ObjectLiteral { .. }) {
        widened = crate::checks::expr::widen_type(ty);
        &widened
    } else {
        ty
    };
    with_index(ctx, |index| {
        if let Some(span) = own_span(expression)
            && !index.sealed.contains(&span)
        {
            let ty = match expression {
                ParsedExpression::ArrowFunction(_) => index.own_functions.get(&span).cloned().unwrap_or_else(|| ty.clone()),
                _ => ty.clone(),
            };
            FileSemanticIndex::insert_contextual(&mut index.expressions, span, &ty);
        }
        if let Some(span) = callee_span(expression) {
            FileSemanticIndex::insert_contextual(&mut index.call_results, span, ty);
        }
        if let Some(span) = element_span(expression) {
            FileSemanticIndex::insert_contextual(&mut index.elements, span, ty);
        }
        // A JSX element's attributes, lowered to an object literal with no
        // span of its own, are reached only through its properties' names.
        if let (ParsedExpression::ObjectLiteral { properties, span: None }, Type::Object(object)) = (expression, ty) {
            for property in properties {
                if property.is_spread || property.computed_key.is_some() {
                    continue;
                }
                if let (Some(span), Some(member)) = (span(&property.name_span), object.get_property(property.name.as_str())) {
                    index.declarations.entry(span).or_insert_with(|| member.ty.clone());
                }
            }
        }
    });
}

/// Records the type an expression is checked against, under its own span
/// (or `fallback_span`, the span its caller knows it by).
pub(crate) fn record_contextual_type(
    expression: &ParsedExpression,
    fallback_span: Option<TextSpan>,
    ty: &Type,
    ctx: &CheckerContext,
) {
    if !recording() {
        return;
    }
    with_index(ctx, |index| {
        if let Some(key) = own_span(expression).or_else(|| span(&fallback_span)) {
            FileSemanticIndex::insert(&mut index.contextual, key, ty);
        }
    });
}

/// Records each argument's parameter in the signature a generic call was
/// instantiated to.
pub(crate) fn record_instantiated_arguments(
    arguments: &[surge_ts_syntax::ParsedCallArgument],
    instantiated: &surge_ts_types::FunctionType,
    ctx: &CheckerContext,
) {
    if !recording() {
        return;
    }
    let parameters = instantiated.parameters();
    with_index(ctx, |index| {
        for (position, argument) in arguments.iter().enumerate() {
            if argument.spread {
                break;
            }
            let rest = instantiated.is_variadic() && position + 1 >= parameters.len();
            let Some(parameter) = parameters.get(position.min(parameters.len().saturating_sub(1))) else { break };
            let ty = if rest {
                match parameter {
                    Type::Array(element) => element.as_ref().clone(),
                    _ => break,
                }
            } else if position >= parameters.len() {
                break;
            } else if position >= instantiated.required_parameter_count()
                && !surge_ts_types::is_assignable_to(&Type::Undefined, parameter)
            {
                surge_ts_types::union_type(vec![parameter.clone(), Type::Undefined])
            } else {
                parameter.clone()
            };
            if ty.is_degraded() || crate::checks::expected::mentions_type_parameter(&ty) {
                continue;
            }
            if let Some(key) = own_span(&argument.expression).or_else(|| span(&argument.span)) {
                index.instantiated_contextual.insert(key, ty);
            }
        }
    });
}

/// Records the signature a function expression at `span` is checked
/// against, unless the expected type it was checked against (which may be
/// wider, `F | undefined`) is recorded already.
pub(crate) fn record_contextual_signature(span: Option<TextSpan>, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let Some(key) = self::span(&span) else { return };
    with_index(ctx, |index| {
        index.contextual.entry(key).or_insert_with(|| ty.clone());
    });
}

/// Records the type the checker computed for the expression written at
/// `span` where the expression itself is not at hand (a call checked by its
/// parts).
pub(crate) fn record_span_type(span: Option<TextSpan>, ty: Option<&Type>, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let (Some(span), Some(ty)) = (span, ty) else { return };
    with_index(ctx, |index| {
        let key = (span.start, span.end);
        if !index.sealed.contains(&key) {
            FileSemanticIndex::insert_contextual(&mut index.expressions, key, ty);
        }
    });
}

/// Records a literal's own type at its span, final whatever is recorded
/// there later.
pub(crate) fn record_literal_type(span: Option<TextSpan>, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let Some(span) = span else { return };
    with_index(ctx, |index| {
        index.expressions.insert((span.start, span.end), ty.clone());
        index.sealed.insert((span.start, span.end));
    });
}

/// Records the type an `as const` operand has in its const context, final
/// whatever the operand's own evaluation records there.
pub(crate) fn record_const_operand_type(operand: &ParsedExpression, ty: &Type, ctx: &CheckerContext) {
    if !recording() || !matches!(operand, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }) {
        return;
    }
    let Some(key) = own_span(operand) else { return };
    with_index(ctx, |index| {
        index.expressions.insert(key, ty.clone());
        index.sealed.insert(key);
    });
}

/// Records the type scope a function body's local type declaration named at
/// `name_span` resolves in.
pub(crate) fn record_local_type_scope(name_span: Option<TextSpan>, scope: &Arc<TypeDeclarationScope>, ctx: &CheckerContext) {
    let Some(span) = span(&name_span) else { return };
    with_index(ctx, |index| {
        index.local_type_scopes.insert(span, scope.clone());
    });
}

/// Records the signature a function expression at `span` has of its own,
/// where the checker relates it as another.
pub(crate) fn record_own_function_type(span: Option<TextSpan>, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let Some(span) = span else { return };
    with_index(ctx, |index| {
        index.own_functions.insert((span.start, span.end), ty.clone());
    });
}

/// A function's type as the compiler API shows it: with the type predicate
/// tsc infers from its body (`getTypePredicateFromBody`), which the checker
/// keeps on the signature it narrows by rather than on the type.
pub(crate) fn with_inferred_predicate(ty: &Type, signature: Option<&crate::symbols::FunctionSignatureInfo>) -> Type {
    let (Type::Function(function), Some(inferred)) = (ty, signature.and_then(|signature| signature.inferred_predicate.as_ref()))
    else {
        return ty.clone();
    };
    if function.type_predicate().is_some() {
        return ty.clone();
    }
    Type::Function(function.clone().with_type_predicate(Some(surge_ts_types::TypePredicate {
        kind: surge_ts_types::TypePredicateKind::Identifier,
        parameter_index: Some(inferred.parameter_index),
        ty: Some(inferred.target.clone()),
    })))
}

/// Records a parameter's type at its name, which a re-check of its function
/// without the contextual signature does not replace with `any`.
pub(crate) fn record_parameter_type(name_span: Option<TextSpan>, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let Some(span) = name_span else { return };
    with_index(ctx, |index| {
        FileSemanticIndex::insert_contextual(&mut index.declarations, (span.start, span.end), ty);
    });
}

/// Records the type a declaration gives the name written at `name_span`.
pub(crate) fn record_declaration_type(name_span: Option<TextSpan>, ty: &Type, ctx: &CheckerContext) {
    if !recording() {
        return;
    }
    let Some(span) = name_span else { return };
    with_index(ctx, |index| {
        FileSemanticIndex::insert(&mut index.declarations, (span.start, span.end), ty);
    });
}

/// Captures the scope the file's statements are about to be checked in, and
/// the signatures of its top-level functions.
pub(crate) fn capture_file_scope(
    ctx: &CheckerContext,
    file_index: usize,
    statements: &[ParsedStatement],
    function_signatures: &HashMap<FunctionDeclarationLocation, surge_ts_types::FunctionType>,
) {
    if !recording() {
        return;
    }
    let scope = FileScope {
        type_declarations: ctx.type_declarations.clone(),
        type_declaration_scope: ctx.type_declaration_scope.clone(),
        symbols: ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    };
    with_index(ctx, |index| {
        for (statement_index, statement) in statements.iter().enumerate() {
            let function = match statement {
                ParsedStatement::FunctionDeclaration(function) => function,
                _ => continue,
            };
            let location = FunctionDeclarationLocation { file_index, statement_index };
            if let (Some(signature), Some(span)) = (function_signatures.get(&location), function.name_span) {
                let inferred = ctx.symbols.get(&function.name).and_then(|symbol| symbol.function_signature.as_deref());
                FileSemanticIndex::insert(
                    &mut index.declarations,
                    (span.start, span.end),
                    &with_inferred_predicate(&Type::Function(signature.clone()), inferred),
                );
            }
        }
        index.scope = Some(scope);
    });
}

/// Runs `check` with the recorder on for `file_name`, returning what it
/// recorded. The caller holds the engine lock.
pub(crate) fn record_file(file_name: &str, check: impl FnOnce()) -> FileSemanticIndex {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            RECORDING.store(false, Ordering::Relaxed);
        }
    }
    RECORDER.with(|recorder| {
        *recorder.borrow_mut() = Some(Recorder {
            file_name: file_name.to_string(),
            index: FileSemanticIndex::default(),
        });
    });
    {
        let _stop = Stop;
        RECORDING.store(true, Ordering::Relaxed);
        check();
    }
    RECORDER
        .with(|recorder| recorder.borrow_mut().take())
        .map(|recorder| recorder.index)
        .unwrap_or_default()
}

fn span(span: &Option<TextSpan>) -> Option<Span> {
    span.map(|span| (span.start, span.end))
}

fn join(first: &Option<TextSpan>, last: &Option<TextSpan>) -> Option<Span> {
    Some((first.as_ref()?.start, last.as_ref()?.end))
}

/// The span tsc's node for `expression` covers, where the lowered tree keeps
/// or implies one. Expressions the lowering synthesizes (a destructuring
/// element's read, an object rest) have none.
fn own_span(expression: &ParsedExpression) -> Option<Span> {
    use ParsedExpression as E;
    match expression {
        E::Identifier { span: s, .. }
        | E::This { span: s }
        | E::ObjectLiteral { span: s, .. }
        | E::ArrayLiteral { span: s, .. }
        | E::TemplateLiteral { span: s, .. }
        | E::Yield { span: s, .. }
        | E::New { span: s, .. }
        | E::SatisfiesExpression { span: s, .. }
        | E::NonNullAssertion { span: s, .. }
        | E::ConstAssertion { span: s, .. }
        | E::ImportCall { span: s, .. }
        | E::JsxFragment { span: s, .. } => span(s),
        E::ArrowFunction(function) => span(&function.span),
        E::Binary { left_span, right_span, .. }
        | E::Logical { left_span, right_span, .. }
        | E::NullishCoalescing { left_span, right_span, .. } => join(left_span, right_span),
        E::Conditional { condition_span, when_false_span, .. } => join(condition_span, when_false_span),
        E::PropertyAccess { object_span, property_span, is_bracketed: false, binding_element: false, .. }
        | E::OptionalPropertyAccess { object_span, property_span, is_bracketed: false, .. } => {
            join(object_span, property_span)
        }
        E::PropertyCall { call_span, .. } | E::OptionalPropertyCall { call_span, .. } => span(call_span),
        E::Unary { operator_span, operand_span, .. } => join(operator_span, operand_span),
        E::Assignment { target_span, value_span, .. } => join(target_span, value_span),
        _ => None,
    }
}

/// An element access's object start and argument end.
fn element_span(expression: &ParsedExpression) -> Option<Span> {
    use ParsedExpression as E;
    match expression {
        E::PropertyAccess { object_span, property_span, is_bracketed: true, binding_element: false, .. } => {
            join(object_span, property_span)
        }
        E::IndexAccess { object_span, index_span, .. } | E::ElementAccess { object_span, index_span, .. } => {
            join(object_span, index_span)
        }
        _ => None,
    }
}

fn callee_span(expression: &ParsedExpression) -> Option<Span> {
    use ParsedExpression as E;
    match expression {
        E::Call { callee_span, .. } | E::OptionalCall { callee_span, .. } | E::ExpressionCall { callee_span, .. } => {
            span(callee_span)
        }
        _ => None,
    }
}
