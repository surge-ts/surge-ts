//! Class or enum used before its declaration (TS2449/TS2450), ported from
//! `checkResolvedBlockScopedVariable` / `isBlockScopedNameDeclaredBeforeUse` in
//! `tsc/internal/checker/checker.go`.
//!
//! A class binding is in its temporal dead zone until the declaration is
//! evaluated, so a reference that runs *before* it is an error — but only a
//! reference that actually runs then. tsc expresses that with
//! `isUsedInFunctionOrInstanceProperty`, which walks up from the use site and
//! quits at the first function-like ancestor: anything inside a function body
//! is deferred and legal however early it appears. The walk here mirrors that
//! by never descending into a function or arrow body.
//!
//! Type positions are exempt for the same reason (`isInAmbientOrTypeNode`), and
//! come for free: annotations are a separate tree from the expressions walked
//! here.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    EXPRESSION_HERITAGE_BASE, ParsedBindingName, ParsedClassDeclaration, ParsedClassMember,
    ParsedDecoratorTarget, ParsedExportDeclaration, ParsedExpression, ParsedFunctionBodyStatement,
    ParsedStatement, ParsedType, TextSpan,
};

use crate::context::{CheckerContext, convert_span};
use crate::symbols::TypeDeclarationInfo;
use surge_ts_types::fx::FxHashMap;

/// A declaration whose binding is in its temporal dead zone until evaluated.
#[derive(Clone, Copy)]
pub(crate) struct ForwardDeclaration {
    span: TextSpan,
    is_enum: bool,
}

/// Where each non-ambient class and regular enum in this file is declared. An
/// ambient declaration has no evaluation to be early of, and a `const enum`
/// no runtime binding at all (outside `isolatedModules`), which is why tsc
/// exempts both.
pub(crate) fn file_class_declarations(
    statements: &[ParsedStatement],
) -> FxHashMap<String, ForwardDeclaration> {
    let mut declarations = FxHashMap::default();
    collect(statements, &mut declarations);
    declarations
}

fn collect(statements: &[ParsedStatement], out: &mut FxHashMap<String, ForwardDeclaration>) {
    for statement in statements {
        match statement {
            ParsedStatement::ClassDeclaration(class) => {
                if class.is_declare {
                    continue;
                }
                if let Some(span) = class.name_span {
                    out.entry(class.name.clone())
                        .or_insert(ForwardDeclaration { span, is_enum: false });
                }
            }
            ParsedStatement::TypeAliasDeclaration(alias)
                if alias.enum_name.as_deref() == Some(alias.name.as_str())
                    && !alias.is_declare
                    && !alias.enum_is_const =>
            {
                if let Some(span) = alias.name_span {
                    out.entry(alias.name.clone())
                        .or_insert(ForwardDeclaration { span, is_enum: true });
                }
            }
            ParsedStatement::ExportDeclaration(export) => {
                if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                    collect(std::slice::from_ref(declaration), out);
                }
            }
            ParsedStatement::NamespaceDeclaration(namespace) if !namespace.is_declare => {
                collect_namespace_classes(&namespace.statements, &namespace.name, out);
            }
            _ => {}
        }
    }
}

/// A namespace's exported classes, keyed by their dotted path from the file
/// (`N.E`), which is how a qualified `extends N.E` reaches them
/// (`checkPropertyNotUsedBeforeDeclaration`).
fn collect_namespace_classes(
    statements: &[ParsedStatement],
    prefix: &str,
    out: &mut FxHashMap<String, ForwardDeclaration>,
) {
    for statement in statements {
        let (exported, inner) = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => (true, declaration.as_ref()),
                _ => continue,
            },
            other => (false, other),
        };
        match inner {
            ParsedStatement::ClassDeclaration(class) if exported && !class.is_declare => {
                if let Some(span) = class.name_span {
                    out.entry(format!("{prefix}.{}", class.name))
                        .or_insert(ForwardDeclaration { span, is_enum: false });
                }
            }
            ParsedStatement::NamespaceDeclaration(namespace) if !namespace.is_declare => {
                collect_namespace_classes(&namespace.statements, &format!("{prefix}.{}", namespace.name), out);
            }
            _ => {}
        }
    }
}

pub(crate) fn check_statement_forward_references(
    statement: &ParsedStatement,
    classes: &FxHashMap<String, ForwardDeclaration>,
    ctx: &mut CheckerContext,
) {
    if classes.is_empty() {
        return;
    }
    let mut reported = Vec::new();
    for_each_statement_expression(statement, &mut |expression| {
        walk(expression, classes, &mut reported);
    });
    if let Some(class) = declared_class(statement) {
        walk_class_head(class, classes, ctx.options.experimental_decorators, &[], &mut reported);
    }
    let unwrapped = match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
            _ => statement,
        },
        other => other,
    };
    if let ParsedStatement::NamespaceDeclaration(namespace) = unwrapped
        && !namespace.is_declare
    {
        walk_namespace_class_heads(namespace, classes, ctx.options.experimental_decorators, &[], &mut reported);
    }
    for (name, span, is_enum) in reported {
        let diagnostic = if is_enum {
            Diagnostic::ts2450(&name, ctx.file_name.clone())
        } else {
            Diagnostic::ts2449(&name, ctx.file_name.clone())
        };
        ctx.push(diagnostic.with_span(convert_span(span)));
    }
}

/// The heads of the classes a namespace body declares, which run when the
/// body does, resolved under its scope chain.
fn walk_namespace_class_heads(
    namespace: &surge_ts_syntax::ParsedNamespaceDeclaration,
    classes: &FxHashMap<String, ForwardDeclaration>,
    legacy_decorators: bool,
    scope: &[String],
    reported: &mut Vec<(String, TextSpan, bool)>,
) {
    let mut inner_scope = scope.to_vec();
    inner_scope.push(namespace.name.clone());
    for statement in &namespace.statements {
        let statement = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                _ => continue,
            },
            other => other,
        };
        match statement {
            ParsedStatement::ClassDeclaration(class) => {
                walk_class_head(class, classes, legacy_decorators, &inner_scope, reported);
            }
            ParsedStatement::NamespaceDeclaration(nested) if !nested.is_declare => {
                walk_namespace_class_heads(nested, classes, legacy_decorators, &inner_scope, reported);
            }
            _ => {}
        }
    }
}

fn declared_class(statement: &ParsedStatement) -> Option<&ParsedClassDeclaration> {
    match statement {
        ParsedStatement::ClassDeclaration(class) => Some(class),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declared_class(declaration),
            _ => None,
        },
        _ => None,
    }
}

/// What a class declaration evaluates before its binding is initialized: its
/// members' computed names and the decorators tsc checks. Besides a class
/// declared later, `isBlockScopedNameDeclaredBeforeUse` rejects the class
/// itself there — in a decorator (other than a legacy one) only outside a
/// function it defers to.
fn walk_class_head(
    class: &ParsedClassDeclaration,
    classes: &FxHashMap<String, ForwardDeclaration>,
    legacy_decorators: bool,
    scope: &[String],
    reported: &mut Vec<(String, TextSpan, bool)>,
) {
    if class.is_declare {
        return;
    }
    // The `extends` expression is evaluated with the declaration.
    for base in &class.extends {
        let head = base.name.split('.').next().unwrap_or(&base.name);
        if scope.is_empty()
            && let Some(declared) = classes.get(head)
            && let Some(span) = base.span
            && span.start < declared.span.start
        {
            let use_span = TextSpan { start: span.start, end: span.start + head.len() };
            reported.push((head.to_string(), use_span, declared.is_enum));
            continue;
        }
        // A qualified base reads the class off its namespace, resolved from
        // the innermost enclosing namespace outward.
        if let Some(dot) = base.name.rfind('.')
            && let Some(span) = base.span
        {
            let declared = (0..=scope.len()).rev().find_map(|depth| {
                let key = if depth == 0 {
                    base.name.clone()
                } else {
                    format!("{}.{}", scope[..depth].join("."), base.name)
                };
                classes.get(&key)
            });
            if let Some(declared) = declared
                && span.start < declared.span.start
            {
                let name = &base.name[dot + 1..];
                let start = span.start + dot + 1;
                reported.push((name.to_string(), TextSpan { start, end: start + name.len() }, declared.is_enum));
            }
        }
    }
    if let Some(expression) = &class.heritage_expression {
        walk(expression, classes, reported);
    }
    for (key, _) in &class.computed_keys {
        walk(key, classes, reported);
        walk_self_reference(key, &class.name, reported);
    }
    for decorator in &class.decorators {
        if !decorator_is_checked(decorator.target, legacy_decorators) {
            continue;
        }
        walk(&decorator.expression, classes, reported);
        if !legacy_decorators {
            walk_self_reference(&decorator.expression, &class.name, reported);
        }
    }
}

fn walk_self_reference(expression: &ParsedExpression, class_name: &str, reported: &mut Vec<(String, TextSpan, bool)>) {
    if let ParsedExpression::Identifier { name, span: Some(span) } = expression
        && name == class_name
    {
        reported.push((name.clone(), *span, false));
        return;
    }
    for_each_child_expression(expression, &mut |child| walk_self_reference(child, class_name, reported));
}

/// tsc's `nodeCanBeDecorated` for a declaration of a class declaration: a
/// decorator anywhere else has its grammar error and is not checked.
pub(crate) fn decorator_is_checked(target: ParsedDecoratorTarget, legacy_decorators: bool) -> bool {
    match target {
        ParsedDecoratorTarget::Class => true,
        ParsedDecoratorTarget::Property { is_abstract, is_declare, private_name, .. } => {
            if legacy_decorators {
                !private_name
            } else {
                !is_abstract && !is_declare
            }
        }
        ParsedDecoratorTarget::Method { has_body, private_name } => has_body && !(legacy_decorators && private_name),
        ParsedDecoratorTarget::Parameter => legacy_decorators,
    }
}

/// The expressions a module-level statement evaluates as it runs. An
/// `export { X }` clause names a binding without reading it, which is why tsc
/// exempts an export specifier here.
fn for_each_statement_expression(
    statement: &ParsedStatement,
    visit: &mut impl FnMut(&ParsedExpression),
) {
    match statement {
        ParsedStatement::VariableDeclaration(variable) => {
            if let Some(initializer) = &variable.initializer {
                visit(initializer);
            }
        }
        ParsedStatement::Expression(expression) => visit(expression),
        ParsedStatement::Call(call) => {
            visit(&ParsedExpression::Identifier {
                name: call.callee_name.clone(),
                span: call.callee_span,
            });
            for argument in &call.arguments {
                visit(&argument.expression);
            }
        }
        ParsedStatement::Assignment(assignment) => visit(&assignment.value),
        ParsedStatement::MemberAssignment(assignment) => visit(&assignment.value),
        ParsedStatement::ExportDeclaration(export) => {
            if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                for_each_statement_expression(declaration, visit);
            }
        }
        // A namespace body runs where the namespace stands.
        ParsedStatement::NamespaceDeclaration(namespace) if !namespace.is_declare => {
            for statement in &namespace.statements {
                for_each_statement_expression(statement, visit);
            }
        }
        _ => {}
    }
}

fn walk(
    expression: &ParsedExpression,
    classes: &FxHashMap<String, ForwardDeclaration>,
    reported: &mut Vec<(String, TextSpan, bool)>,
) {
    if let ParsedExpression::Identifier { name, span } = expression
        && let Some(declared) = classes.get(name)
        && let Some(use_span) = span
        && use_span.start < declared.span.start
    {
        reported.push((name.clone(), *use_span, declared.is_enum));
        return;
    }
    if let ParsedExpression::ClassExpression(class_expression) = expression {
        walk_class_expression_head(class_expression, classes, reported);
        return;
    }

    for_each_child_expression(expression, &mut |child| walk(child, classes, reported));
}

/// What a class expression evaluates as it runs: its base, and its members'
/// computed names, where a name the class writes for itself is still in its
/// temporal dead zone (`isBlockScopedNameDeclaredBeforeUse`). A later class
/// named in a computed name is not reported: in a method's name it is
/// deferred (`isUsedInFunctionOrInstanceProperty`), which the name alone does
/// not tell apart.
fn walk_class_expression_head(
    class_expression: &surge_ts_syntax::ParsedClassExpression,
    classes: &FxHashMap<String, ForwardDeclaration>,
    reported: &mut Vec<(String, TextSpan, bool)>,
) {
    let class = &class_expression.class;
    for base in &class.extends {
        let head = base.name.split('.').next().unwrap_or(&base.name);
        if let Some(declared) = classes.get(head)
            && let Some(span) = base.span
            && span.start < declared.span.start
        {
            let use_span = TextSpan { start: span.start, end: span.start + head.len() };
            reported.push((head.to_string(), use_span, declared.is_enum));
        }
    }
    if let Some(expression) = &class.heritage_expression {
        walk(expression, classes, reported);
    }
    if class_expression.has_own_name {
        for (key, _) in &class.computed_keys {
            walk_self_reference(key, &class.name, reported);
        }
    }
}

/// The expressions an immediately invoked body evaluates as it runs, nested
/// functions excluded.
fn for_each_body_statement_expression(
    statements: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    visit: &mut impl FnMut(&ParsedExpression),
) {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(variable) => {
                if let Some(initializer) = &variable.initializer {
                    visit(initializer);
                }
            }
            Statement::Return(statement) => {
                if let Some(expression) = &statement.expression {
                    visit(expression);
                }
            }
            Statement::Expression(expression) => visit(expression),
            Statement::Assignment(assignment) => visit(&assignment.value),
            Statement::MemberAssignment(assignment) => visit(&assignment.value),
            Statement::Block(statements) => for_each_body_statement_expression(statements, visit),
            _ => {}
        }
    }
}

/// Sub-expressions evaluated as part of this one. A function or arrow body is
/// deliberately not among them: its contents run later, which is exactly what
/// makes an early reference inside one legal.
fn for_each_child_expression(
    expression: &ParsedExpression,
    visit: &mut impl FnMut(&ParsedExpression),
) {
    match expression {
        ParsedExpression::New {
            callee, arguments, ..
        } => {
            visit(callee);
            for argument in arguments {
                visit(&argument.expression);
            }
        }
        ParsedExpression::Call { arguments, .. } => {
            for argument in arguments {
                visit(&argument.expression);
            }
        }
        ParsedExpression::ExpressionCall {
            callee, arguments, ..
        }
        | ParsedExpression::OptionalCall {
            callee, arguments, ..
        } => {
            visit(callee);
            for argument in arguments {
                visit(&argument.expression);
            }
            // An immediately invoked function's body runs as it is called
            // (`isUsedInFunctionOrInstanceProperty` walks past an IIFE).
            if let ParsedExpression::ArrowFunction(function) = callee.as_ref() {
                match &function.body {
                    surge_ts_syntax::ParsedArrowFunctionBody::Expression(body) => visit(body),
                    surge_ts_syntax::ParsedArrowFunctionBody::Block(statements) => {
                        for_each_body_statement_expression(statements, visit);
                    }
                }
            }
        }
        ParsedExpression::PropertyCall {
            object, arguments, ..
        }
        | ParsedExpression::OptionalPropertyCall {
            object, arguments, ..
        } => {
            visit(object);
            for argument in arguments {
                visit(&argument.expression);
            }
        }
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. } => visit(object),
        ParsedExpression::ElementAccess { object, index, .. } => {
            visit(object);
            visit(index);
        }
        ParsedExpression::IndexAccess { index, .. } => visit(index),
        ParsedExpression::Unary { operand, .. }
        | ParsedExpression::Update { operand, .. }
        | ParsedExpression::Await { operand, .. } => visit(operand),
        ParsedExpression::ObjectRest { source, .. }
        | ParsedExpression::EmptyBindingPattern { source, .. } => visit(source),
        ParsedExpression::ArrayPatternElement { read, .. } => visit(read),
        ParsedExpression::Sequence { expressions } => {
            for (expression, _) in expressions {
                visit(expression);
            }
        }
        ParsedExpression::Binary { left, right, .. }
        | ParsedExpression::Logical { left, right, .. }
        | ParsedExpression::NullishCoalescing { left, right, .. } => {
            visit(left);
            visit(right);
        }
        ParsedExpression::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => {
            visit(condition);
            visit(when_true);
            visit(when_false);
        }
        ParsedExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                visit(&element.expression);
            }
        }
        ParsedExpression::ObjectLiteral { properties, .. } => {
            for property in properties {
                // A method shorthand carries a body, which runs later.
                if !property.is_method && !property.is_accessor {
                    visit(&property.value);
                }
            }
        }
        ParsedExpression::TemplateLiteral { expressions, .. } => {
            for interpolation in expressions {
                visit(interpolation);
            }
        }
        ParsedExpression::TypeAssertion { expression, .. }
        | ParsedExpression::SatisfiesExpression { expression, .. }
        | ParsedExpression::ConstAssertion { expression, .. }
        | ParsedExpression::NonNullAssertion { expression, .. } => visit(expression),
        _ => {}
    }
}

/// A class member a property initializer or a static block can read through
/// `this` or the class name, as `checkPropertyNotUsedBeforeDeclaration` weighs
/// its declaration.
#[derive(Clone, Copy)]
enum InitializedMemberKind {
    /// A property declaration, auto-accessors included.
    Property {
        has_initializer: bool,
        /// `x!: T`.
        asserted: bool,
        /// Typed `any` or `unknown`, an unannotated and uninitialized property
        /// included: `undefined` never shows in its flow type, so any static
        /// block ahead of a read counts as initializing it
        /// (`isPropertyInitializedInStaticBlocks`).
        wide: bool,
    },
    /// A constructor parameter property.
    ParameterProperty,
    /// A `get`/`set` accessor.
    Accessor,
    /// Never reported: an optional property, whose `undefined` is what its type
    /// already says (`isOptionalPropertyDeclaration`), or a method, which is
    /// deferred from an instance initializer and exempt when static.
    Exempt,
}

#[derive(Clone, Copy)]
struct InitializedMember {
    kind: InitializedMemberKind,
    is_static: bool,
    /// The declaration's source range (tsc's `declaration.Pos()` and `End()`).
    start: usize,
    end: usize,
}

/// Where a read runs, which is what `isUsedInFunctionOrInstanceProperty` asks
/// of it.
#[derive(Clone, Copy)]
enum ReadSite {
    Initializer { is_static: bool, initializer_start: usize },
    StaticBlock,
}

/// Property reads that run before the property is initialized (TS2729), ported
/// from `checkPropertyNotUsedBeforeDeclaration`: `this.<name>` or
/// `<Class>.<name>` in a property initializer or a static block, outside any
/// function or arrow (`isInPropertyInitializerOrClassStaticBlock`), resolved
/// to the class's own member on the side `this` stands for there.
pub(crate) fn check_class_property_initializers(
    class: &ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    if surge_ts_syntax::is_declaration_file_name(&ctx.file_name) {
        return;
    }
    let members = initialized_members(class);
    if members.is_empty() {
        return;
    }
    let emit_standard_class_fields = ctx.options.emit_standard_class_fields();
    let initialized_in_static_blocks = |name: &str, wide: bool, before: usize| {
        class.members.iter().any(|member| {
            matches!(member, ParsedClassMember::StaticBlock(block)
                if block.span.is_some_and(|span| span.start <= before)
                    && (wide || super::property_initialization::body_assigns(&block.body, name)))
        })
    };

    let mut reported = Vec::new();
    let mut check_read = |name: &str, read: TextSpan, is_static: bool, through_this: bool, site: ReadSite| {
        let Some(member) = members.get(&(name, is_static)).copied() else {
            return;
        };
        let early = read_before_initialization(member, site, read, through_this, emit_standard_class_fields, || {
            match (member.kind, site) {
                (
                    InitializedMemberKind::Property { wide, .. },
                    ReadSite::Initializer { initializer_start, .. },
                ) => initialized_in_static_blocks(name, wide, initializer_start),
                _ => false,
            }
        });
        if early {
            reported.push((name.to_string(), read));
        }
    };
    for member in &class.members {
        match member {
            ParsedClassMember::Property(property) if property.this_assignments.is_none() => {
                let (Some(initializer), Some(initializer_span)) =
                    (&property.initializer, property.initializer_span)
                else {
                    continue;
                };
                let site = ReadSite::Initializer {
                    is_static: property.is_static,
                    initializer_start: initializer_span.start,
                };
                walk_member_reads(initializer, &class.name, &mut |name, read, through_this| {
                    check_read(name, read, property.is_static || !through_this, through_this, site)
                });
            }
            ParsedClassMember::StaticBlock(block) => {
                walk_statement_member_reads(&block.body, &class.name, &mut |name, read, through_this| {
                    check_read(name, read, true, through_this, ReadSite::StaticBlock)
                });
            }
            _ => {}
        }
    }

    let use_define_for_class_fields = ctx.options.use_define_for_class_fields;
    for (name, span) in reported {
        if !use_define_for_class_fields && declared_in_ancestor_class(class, &name, ctx) {
            continue;
        }
        let diagnostic = Diagnostic::ts2729(surge_ts_types::private_name::display(&name), ctx.file_name.clone());
        ctx.push(diagnostic.with_span(convert_span(span)));
    }
}

/// TS7022 for an unannotated property whose type comes from an initializer
/// that reads, through `this` or the class and outside any function, a
/// property whose type depends back on it: resolving either type meets the
/// other mid-resolution, and both are `any`. A property that only reads into
/// such a cycle is not part of it.
pub(crate) fn check_circular_property_initializers(
    class: &ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    if !ctx.options.no_implicit_any || surge_ts_syntax::is_declaration_file_name(&ctx.file_name) {
        return;
    }
    let mut nodes: Vec<((&str, bool), TextSpan, Vec<(String, bool)>)> = Vec::new();
    for member in &class.members {
        let ParsedClassMember::Property(property) = member else {
            continue;
        };
        let (Some(initializer), Some(name_span), None, None) = (
            &property.initializer,
            property.name_span,
            &property.declared_type,
            &property.this_assignments,
        ) else {
            continue;
        };
        let mut reads = Vec::new();
        walk_member_reads(initializer, &class.name, &mut |name, _, through_this| {
            reads.push((name.to_string(), property.is_static || !through_this));
        });
        nodes.push(((property.name.as_str(), property.is_static), name_span, reads));
    }
    let index: FxHashMap<(&str, bool), usize> =
        nodes.iter().enumerate().map(|(position, (key, _, _))| (*key, position)).collect();
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|(_, _, reads)| {
            reads
                .iter()
                .filter_map(|(name, is_static)| index.get(&(name.as_str(), *is_static)).copied())
                .collect()
        })
        .collect();
    for (start, ((name, _), span, _)) in nodes.iter().enumerate() {
        let mut stack = edges[start].clone();
        let mut seen = vec![false; nodes.len()];
        let mut cyclic = false;
        while let Some(next) = stack.pop() {
            if next == start {
                cyclic = true;
                break;
            }
            if !std::mem::replace(&mut seen[next], true) {
                stack.extend(edges[next].iter().copied());
            }
        }
        if cyclic {
            let diagnostic =
                Diagnostic::ts7022(surge_ts_types::private_name::display(name), ctx.file_name.clone());
            ctx.push(diagnostic.with_span(convert_span(*span)));
        }
    }
}

/// The class's own members by name and side; the first declaration of a name
/// is its value declaration.
fn initialized_members(class: &ParsedClassDeclaration) -> FxHashMap<(&str, bool), InitializedMember> {
    fn declare<'a>(
        members: &mut FxHashMap<(&'a str, bool), InitializedMember>,
        name: &'a str,
        is_static: bool,
        span: Option<TextSpan>,
        kind: InitializedMemberKind,
    ) {
        if let Some(span) = span {
            members.entry((name, is_static)).or_insert(InitializedMember {
                kind,
                is_static,
                start: span.start,
                end: span.end,
            });
        }
    }
    let mut members = FxHashMap::default();
    for member in &class.members {
        match member {
            // A JavaScript `this.x = v` declares no class element.
            ParsedClassMember::Property(property) if property.this_assignments.is_none() => {
                let kind = if property.optional {
                    InitializedMemberKind::Exempt
                } else {
                    InitializedMemberKind::Property {
                        has_initializer: property.initializer.is_some(),
                        asserted: property.has_definite_assertion,
                        wide: match &property.declared_type {
                            Some(declared) => matches!(declared, ParsedType::Any | ParsedType::UnknownKeyword),
                            None => property.initializer.is_none(),
                        },
                    }
                };
                declare(&mut members, &property.name, property.is_static, property.span, kind);
            }
            ParsedClassMember::Property(_) => {}
            ParsedClassMember::Accessor(accessor) => {
                declare(
                    &mut members,
                    &accessor.name,
                    accessor.is_static,
                    accessor.span,
                    InitializedMemberKind::Accessor,
                );
            }
            ParsedClassMember::Method(method) => {
                declare(&mut members, &method.name, method.is_static, method.span, InitializedMemberKind::Exempt);
            }
            ParsedClassMember::Constructor(constructor) => {
                for parameter in &constructor.parameters {
                    if parameter.is_parameter_property
                        && let ParsedBindingName::Identifier { name, span } = &parameter.binding_name
                    {
                        declare(&mut members, name, false, *span, InitializedMemberKind::ParameterProperty);
                    }
                }
            }
            ParsedClassMember::StaticBlock(_) => {}
        }
    }
    members
}

/// `isBlockScopedNameDeclaredBeforeUse` for a read of a member of the class
/// being declared, with `isUsedInFunctionOrInstanceProperty` and
/// `isPropertyImmediatelyReferencedWithinDeclaration` answered for the two
/// sites such a read can run from.
///
/// A property **without an initializer** never passes the declared-before test
/// through `this`, even when it appears earlier in the class: there is nothing
/// to have run. Only a `!` exempts it. This is the `declaration.Initializer()
/// == nil` clause inside the `declaration.Pos() <= usage.Pos()` branch, and it
/// does not apply to a read through the class name.
fn read_before_initialization(
    member: InitializedMember,
    site: ReadSite,
    read: TextSpan,
    through_this: bool,
    emit_standard_class_fields: bool,
    initialized_in_static_blocks: impl FnOnce() -> bool,
) -> bool {
    let is_property = match member.kind {
        InitializedMemberKind::Exempt => return false,
        InitializedMemberKind::Property { .. } => true,
        InitializedMemberKind::ParameterProperty | InitializedMemberKind::Accessor => false,
    };
    let unassigned_this_read = through_this
        && matches!(
            member.kind,
            InitializedMemberKind::Property { has_initializer: false, asserted: false, .. }
        );
    if member.start <= read.start && !unassigned_this_read {
        return match member.kind {
            // Only a read inside the property's own initializer (`x = this.x`).
            InitializedMemberKind::Property { .. } => read.end <= member.end,
            // With [[Define]] fields a parameter property is assigned after
            // every field initializer has run.
            InitializedMemberKind::ParameterProperty => {
                emit_standard_class_fields && matches!(site, ReadSite::Initializer { is_static: false, .. })
            }
            InitializedMemberKind::Accessor | InitializedMemberKind::Exempt => false,
        };
    }
    let deferred = match site {
        ReadSite::StaticBlock => member.start < read.start,
        ReadSite::Initializer { is_static: false, .. } => !(is_property && !member.is_static),
        ReadSite::Initializer { is_static: true, .. } => is_property && initialized_in_static_blocks(),
    };
    if !deferred {
        return true;
    }
    emit_standard_class_fields
        && (is_property || matches!(member.kind, InitializedMemberKind::ParameterProperty))
        && read.end <= member.end
}

/// tsc's `isPropertyDeclaredInAncestorClass`: without [[Define]] fields, a
/// member the base class already declares was initialized by its constructor.
/// A base chain this file cannot name counts as declaring it.
fn declared_in_ancestor_class(class: &ParsedClassDeclaration, name: &str, ctx: &CheckerContext) -> bool {
    let Some(base) = class.extends.first() else {
        return false;
    };
    let mut next = Some(base.name.clone());
    let mut visited = Vec::new();
    while let Some(current) = next.take() {
        if current == EXPRESSION_HERITAGE_BASE || visited.contains(&current) {
            return true;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&current) else {
            return true;
        };
        if info.body.members.iter().any(|member| member.name == name) {
            return true;
        }
        next = info.body.extends.first().map(|parent| parent.name.clone());
        visited.push(current);
    }
    false
}

/// Visits every `this.<name>` and `<ClassName>.<name>` property access
/// `expression` evaluates as it runs, saying whether it is through `this`. No
/// function or arrow body is entered, not even an immediately invoked one:
/// `isInPropertyInitializerOrClassStaticBlock` stops at both. A `this["name"]`
/// element access is not a property access at all.
fn walk_member_reads(
    expression: &ParsedExpression,
    class_name: &str,
    visit: &mut impl FnMut(&str, TextSpan, bool),
) {
    if let ParsedExpression::PropertyAccess {
        object,
        property_name,
        property_span: Some(span),
        is_bracketed: false,
        binding_element: false,
        ..
    }
    | ParsedExpression::OptionalPropertyAccess {
        object,
        property_name,
        property_span: Some(span),
        is_bracketed: false,
        ..
    }
    | ParsedExpression::PropertyCall {
        object,
        property_name,
        property_span: Some(span),
        ..
    }
    | ParsedExpression::OptionalPropertyCall {
        object,
        property_name,
        property_span: Some(span),
        ..
    } = expression
    {
        match object.as_ref() {
            ParsedExpression::This { .. } => visit(property_name, *span, true),
            ParsedExpression::Identifier { name, .. } if name == class_name => {
                visit(property_name, *span, false)
            }
            _ => {}
        }
    }

    expression.for_each_child(&mut |child| walk_member_reads(child, class_name, visit));
    match expression {
        // A computed key runs with the literal; a method's or accessor's does
        // not reach here, being inside the function it names.
        ParsedExpression::ObjectLiteral { properties, .. } => {
            for property in properties {
                if let Some(key) = &property.computed_key {
                    walk_member_reads(key, class_name, visit);
                }
                if let Some(value) = &property.unnamed_key_value {
                    walk_member_reads(value, class_name, visit);
                }
            }
        }
        // The opening tag is read as a value; the closing tag is not checked
        // for this (`isInPropertyInitializerOrClassStaticBlock` quits at it).
        ParsedExpression::JsxElement { tag, .. } => {
            if let Some(tag_expression) = &tag.expression {
                walk_member_reads(tag_expression, class_name, visit);
            }
        }
        _ => {}
    }
}

/// [`walk_member_reads`] over a static block's statements. A nested function
/// or class declaration runs later, or reads its own `this`.
fn walk_statement_member_reads(
    statements: &[ParsedFunctionBodyStatement],
    class_name: &str,
    visit: &mut impl FnMut(&str, TextSpan, bool),
) {
    for statement in statements {
        match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                if let Some(initializer) = &variable.initializer {
                    walk_member_reads(initializer, class_name, visit);
                }
            }
            ParsedFunctionBodyStatement::Return(statement) => {
                if let Some(expression) = &statement.expression {
                    walk_member_reads(expression, class_name, visit);
                }
            }
            ParsedFunctionBodyStatement::Throw(statement) => {
                walk_member_reads(&statement.expression, class_name, visit);
            }
            ParsedFunctionBodyStatement::Assignment(assignment) => {
                walk_member_reads(&assignment.value, class_name, visit);
            }
            // The target is a property access tsc checks like any other; the
            // same lowering carries `this["x"] = v`, an element access, whose
            // written target ends past the key's closing quote.
            ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => {
                if let (Some(property_span), Some(target_span)) =
                    (assignment.property_span, assignment.target_span)
                    && target_span.end == property_span.end
                {
                    visit(&assignment.property_name, property_span, true);
                }
                walk_member_reads(&assignment.value, class_name, visit);
            }
            ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
                walk_member_reads(&assignment.target, class_name, visit);
                walk_member_reads(&assignment.value, class_name, visit);
            }
            ParsedFunctionBodyStatement::Expression(expression) => {
                walk_member_reads(expression, class_name, visit);
            }
            ParsedFunctionBodyStatement::Block(block) => {
                walk_statement_member_reads(block, class_name, visit);
            }
            ParsedFunctionBodyStatement::If(statement) => {
                walk_member_reads(&statement.condition, class_name, visit);
                walk_statement_member_reads(&statement.then_body, class_name, visit);
                walk_statement_member_reads(&statement.else_body, class_name, visit);
            }
            ParsedFunctionBodyStatement::While(statement) => {
                walk_member_reads(&statement.condition, class_name, visit);
                walk_statement_member_reads(&statement.body, class_name, visit);
            }
            ParsedFunctionBodyStatement::ForOf(statement) => {
                walk_member_reads(&statement.iterable, class_name, visit);
                if let Some((target, _)) = &statement.head_target {
                    walk_member_reads(target, class_name, visit);
                }
                walk_statement_member_reads(&statement.body, class_name, visit);
            }
            ParsedFunctionBodyStatement::Switch(statement) => {
                walk_member_reads(&statement.discriminant, class_name, visit);
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        walk_member_reads(test, class_name, visit);
                    }
                    walk_statement_member_reads(&case.consequent, class_name, visit);
                }
            }
            ParsedFunctionBodyStatement::Try(statement) => {
                walk_statement_member_reads(&statement.block, class_name, visit);
                if let Some(handler) = &statement.handler {
                    walk_statement_member_reads(&handler.body, class_name, visit);
                }
                walk_statement_member_reads(&statement.finalizer, class_name, visit);
            }
            _ => {}
        }
    }
}
