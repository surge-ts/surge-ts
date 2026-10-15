//! Grammar-level diagnostics, collected in one walk of the oxc AST.
//!
//! These are the checks that need no types at all — a `const` with no
//! initializer, a duplicate object-literal key, an overload group with no
//! implementation, a member with no annotation. The lossy `Parsed*` tree drops
//! most of what they look at (an unannotated type member is not even kept), so
//! they run here, against the real AST, and the checker turns each entry into a
//! diagnostic.

use oxc_ast::ast::{
    AssignmentTarget, Class, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression,
    ForInStatement, ForOfStatement, FormalParameters, Function, FunctionBody, MethodDefinitionKind,
    MethodDefinitionType, ModuleExportName, ObjectExpression, ObjectPropertyKind, Program,
    PropertyDefinitionType, PropertyKey, PropertyKind, Statement, TSGlobalDeclaration,
    TSEnumMemberName, TSMethodSignature, TSMethodSignatureKind, TSModuleDeclaration,
    TSModuleDeclarationBody,
    TSPropertySignature, TSSignature, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};
use oxc_syntax::operator::UnaryOperator;

use super::spans::text_span_from_oxc_span;
use crate::{ParenthesizedExpressionSpan, ParsedGrammarDiagnostic, ParsedGrammarDiagnosticKind as Kind};

mod super_call;
mod this_before_super;

pub(crate) fn collect_grammar_diagnostics(
    program: &Program<'_>,
) -> (Vec<ParsedGrammarDiagnostic>, Vec<ParenthesizedExpressionSpan>) {
    let mut collector = GrammarCollector::default();
    collector.visit_program(program);
    super::grammar_context::collect_context_grammar_diagnostics(program, &mut collector.diagnostics);
    super::reachability::collect_unreachable_code(program, &mut collector.diagnostics);
    super::grammar_recovered::collect_recovered_grammar_diagnostics(program, false, &mut collector.diagnostics);
    collect_oversized_flow_containers(program, &mut collector.diagnostics);
    let mut parenthesized = collector.parenthesized_expressions;
    parenthesized.sort_unstable_by_key(|span| (span.inner.start, span.inner.end));
    (collector.diagnostics, parenthesized)
}

/// tsc's `getTypeAtFlowArrayMutation` recurses once per earlier mutation of
/// the same evolving array, and at a flow depth of 2000 gives up with TS2563
/// at the first statement of the containing function or module body. Only a
/// straight run of writes in the body that declares the array is counted.
/// The checker reports it under `noImplicitAny`, where `[]` evolves.
fn collect_oversized_flow_containers(program: &Program<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    const MAX_FLOW_DEPTH: usize = 2000;
    fn mutated_array<'a>(statement: &'a Statement<'_>) -> Option<&'a str> {
        let Statement::ExpressionStatement(statement) = statement else {
            return None;
        };
        match statement.expression.without_parentheses() {
            Expression::AssignmentExpression(assignment) => match &assignment.left {
                oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(member) => {
                    match member.object.without_parentheses() {
                        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                        _ => None,
                    }
                }
                _ => None,
            },
            Expression::CallExpression(call) => match call.callee.without_parentheses() {
                Expression::StaticMemberExpression(member)
                    if matches!(member.property.name.as_str(), "push" | "unshift") =>
                {
                    match member.object.without_parentheses() {
                        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }
    fn check(statements: &[Statement<'_>], out: &mut Vec<ParsedGrammarDiagnostic>) {
        let Some(first) = statements.first() else {
            return;
        };
        let mut evolving: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for statement in statements {
            if let Statement::VariableDeclaration(declaration) = statement {
                for declarator in &declaration.declarations {
                    if declarator.type_annotation.is_none()
                        && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &declarator.id
                        && matches!(&declarator.init, Some(Expression::ArrayExpression(array)) if array.elements.is_empty())
                    {
                        evolving.insert(identifier.name.as_str(), 0);
                    }
                }
                continue;
            }
            if let Some(name) = mutated_array(statement)
                && let Some(count) = evolving.get_mut(name)
            {
                *count += 1;
                if *count > MAX_FLOW_DEPTH {
                    let start = first.span().start;
                    out.push(ParsedGrammarDiagnostic {
                        kind: Kind::Ts(2563),
                        span: text_span_from_oxc_span(oxc_span::Span::new(start, start + 1)),
                        name: None,
                    });
                    return;
                }
            }
        }
    }
    struct Bodies<'o> {
        out: &'o mut Vec<ParsedGrammarDiagnostic>,
    }
    impl<'a> Visit<'a> for Bodies<'_> {
        fn visit_function_body(&mut self, body: &oxc_ast::ast::FunctionBody<'a>) {
            check(&body.statements, self.out);
            oxc_ast_visit::walk::walk_function_body(self, body);
        }
    }
    check(&program.body, out);
    Bodies { out }.visit_program(program);
}

/// tsc's `checkAmbientInitializer` over a declaration file, where everything
/// is ambient: a `const` (or `readonly` property) without a type may be
/// initialized only with a constant (TS1254). Any other initializer is oxc's
/// own TS1039.
pub(crate) fn collect_declaration_file_const_initializers(program: &Program<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    struct Collector<'o> {
        out: &'o mut Vec<ParsedGrammarDiagnostic>,
    }

    impl Collector<'_> {
        fn check(&mut self, initializer: &Expression<'_>, const_or_readonly: bool, has_type: bool) {
            if const_or_readonly && !has_type && !is_ambient_constant_initializer(initializer) {
                self.out.push(ParsedGrammarDiagnostic {
                    kind: Kind::AmbientConstInitializer,
                    span: text_span_from_oxc_span(initializer.span()),
                    name: None,
                });
            }
        }
    }

    impl<'a> Visit<'a> for Collector<'_> {
        fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
            let const_like = matches!(
                declaration.kind,
                VariableDeclarationKind::Const | VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
            );
            for declarator in &declaration.declarations {
                if let Some(initializer) = declarator.init.as_ref() {
                    self.check(initializer, const_like, declarator.type_annotation.is_some());
                }
            }
        }

        fn visit_property_definition(&mut self, property: &oxc_ast::ast::PropertyDefinition<'a>) {
            if let Some(value) = property.value.as_ref() {
                self.check(value, property.readonly, property.type_annotation.is_some());
            }
        }

        fn visit_function_body(&mut self, _body: &oxc_ast::ast::FunctionBody<'a>) {}
    }

    Collector { out }.visit_program(program);
}

/// The circularities tsc's checker finds among a declaration file's own
/// declarations: a variable whose annotation reaches itself through `typeof`
/// (TS2502) and an alias that references itself (TS2456).
pub(crate) fn collect_declaration_file_circularities(program: &Program<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    let mut collector = GrammarCollector::default();
    collector.source_text = program.source_text.to_string();
    collector.collect_top_level_enums(&program.body);
    collector.check_circular_type_aliases(&program.body);
    out.append(&mut collector.diagnostics);
}

/// The implicit `any`s tsc's checker reports in a declaration file: a member
/// without a type (TS7008) and a bodyless signature without a return type
/// (TS7010). Everything there is ambient, so a private member reports neither
/// (`isPrivateWithinAmbient`).
pub(crate) fn collect_declaration_file_implicit_any(
    program: &Program<'_>,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    struct Collector<'o> {
        out: &'o mut Vec<ParsedGrammarDiagnostic>,
    }

    impl Collector<'_> {
        fn push(&mut self, kind: Kind, span: Span, name: &str) {
            self.out.push(ParsedGrammarDiagnostic {
                kind,
                span: text_span_from_oxc_span(span),
                name: Some(name.to_string()),
            });
        }
    }

    fn is_private(accessibility: Option<oxc_ast::ast::TSAccessibility>, key: &PropertyKey<'_>) -> bool {
        accessibility == Some(oxc_ast::ast::TSAccessibility::Private) || matches!(key, PropertyKey::PrivateIdentifier(_))
    }

    impl<'a> Visit<'a> for Collector<'_> {
        fn visit_function(&mut self, function: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
            if function.body.is_none()
                && function.return_type.is_none()
                && let Some(id) = function.id.as_ref()
            {
                self.push(Kind::ImplicitAnyReturn, id.span, id.name.as_str());
            }
            oxc_ast_visit::walk::walk_function(self, function, flags);
        }

        fn visit_method_definition(&mut self, method: &oxc_ast::ast::MethodDefinition<'a>) {
            if method.kind == MethodDefinitionKind::Method
                && method.value.body.is_none()
                && method.value.return_type.is_none()
                && !method.computed
                && !is_private(method.accessibility, &method.key)
                && let Some(name) = property_key_name(&method.key)
            {
                self.push(Kind::ImplicitAnyReturn, method.key.span(), &name);
            }
            oxc_ast_visit::walk::walk_method_definition(self, method);
        }

        fn visit_property_definition(&mut self, property: &oxc_ast::ast::PropertyDefinition<'a>) {
            if property.type_annotation.is_none()
                && property.value.is_none()
                && !property.computed
                && !is_private(property.accessibility, &property.key)
                && let Some(name) = property_key_name(&property.key)
            {
                self.push(Kind::ImplicitAnyMember, property.key.span(), &name);
            }
            oxc_ast_visit::walk::walk_property_definition(self, property);
        }

        fn visit_ts_property_signature(&mut self, property: &TSPropertySignature<'a>) {
            if property.type_annotation.is_none()
                && !property.computed
                && let Some(name) = property_key_name(&property.key)
            {
                self.push(Kind::ImplicitAnyMember, property.key.span(), &name);
            }
            oxc_ast_visit::walk::walk_ts_property_signature(self, property);
        }

        fn visit_ts_method_signature(&mut self, method: &TSMethodSignature<'a>) {
            if method.kind == TSMethodSignatureKind::Method
                && method.return_type.is_none()
                && !method.computed
                && let Some(name) = property_key_name(&method.key)
            {
                self.push(Kind::ImplicitAnyReturn, method.key.span(), &name);
            }
            oxc_ast_visit::walk::walk_ts_method_signature(self, method);
        }
    }

    Collector { out }.visit_program(program);
}

#[derive(Default)]
struct GrammarCollector {
    diagnostics: Vec<ParsedGrammarDiagnostic>,
    parenthesized_expressions: Vec<ParenthesizedExpressionSpan>,
    /// Whether each enclosing function is `async`, innermost last. Empty at the
    /// top level, where a module may `await`.
    function_async: Vec<bool>,
    /// tsc's `NodeFlagsAwaitContext` for each enclosing function, class field
    /// initializer, enum body and static block, innermost last.
    await_context: Vec<bool>,
    /// tsc's `parseParameterWorker` parses a parameter's decorators in the
    /// await context outside its signature; set until they are visited.
    parameter_decorator_await: Option<bool>,
    /// An import or export, or an `import.meta`, makes the file a module
    /// (`IsFileProbablyExternalModule`); the checker adds the module
    /// detection options.
    external_module: bool,
    /// `declare namespace`/`declare module` nesting. An ambient container makes
    /// a bodyless declaration legal, so the implementation-missing checks stay
    /// quiet inside one.
    ambient_depth: usize,
    /// `declare class` nesting: its members are ambient as well.
    declare_class_depth: usize,
    /// The file's text, which modifier-order checks read: the AST keeps which
    /// modifiers a member has, not the order they were written in.
    source_text: String,
    /// Module-level enum declarations, whose duplicate members are checked
    /// across every declaration of the enum at once.
    top_level_enums: std::collections::HashSet<(u32, u32)>,
    /// Starts of the `(0, x.f)` sequences called indirectly; see
    /// [`GrammarCollector::note_indirect_call`].
    indirect_call_sequences: std::collections::HashSet<u32>,
    /// Block and namespace nesting; a computed type member name is answered
    /// only outside both, where the file's top-level scope is the one it reads.
    nested_scope_depth: usize,
    /// Class bodies and namespaces enclosing the node: with `function_async`,
    /// what tsc's `IsInTopLevelContext` looks past.
    this_container_depth: usize,
    /// Every name the file binds, with where; a computed type member name any
    /// of them could answer — a value, a type parameter — is left alone.
    binding_names: Vec<(String, Span)>,
    /// The names of type aliases and interfaces, the bindings that do not.
    type_declaration_name_spans: std::collections::HashSet<(u32, u32)>,
    /// Top-level `const`s of a `unique symbol` type, written or a `Symbol()`
    /// call's: a computed name reading one binds like a literal
    /// (`isLateBindableName`).
    unique_symbol_consts: std::collections::HashSet<String>,
}

/// One `get`/`set` accessor of a class, interface, type literal or object
/// literal, as [`GrammarCollector::check_untyped_setters`] reads it.
struct AccessorRecord {
    key: String,
    display: Option<String>,
    is_static: bool,
    is_getter: bool,
    /// A getter's return annotation, or a setter's parameter annotation.
    annotated: bool,
    has_body: bool,
    private: bool,
    span: Span,
}

fn accessor_display_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(format!("\"{}\"", literal.value)),
        _ => None,
    }
}

/// tsc's `getSetAccessorValueParameter` takes the first parameter whatever its
/// kind, so `set x(...v: T[])` is annotated through its rest parameter.
fn setter_parameter_annotated(parameters: &FormalParameters<'_>) -> bool {
    match parameters.items.first() {
        Some(parameter) => parameter.type_annotation.is_some(),
        None => parameters.rest.as_ref().is_some_and(|rest| rest.type_annotation.is_some()),
    }
}

impl GrammarCollector {
    /// tsc's `isIndirectCall`: `(0, x.f)(…)`, a tagged `(0, x.f)` or
    /// `(0, eval)(…)` discards the `0` to call without a `this`, so the
    /// unused-comma check leaves it alone.
    fn note_indirect_call(&mut self, callee: &Expression<'_>) {
        let Expression::ParenthesizedExpression(parenthesized) = callee else {
            return;
        };
        let Expression::SequenceExpression(sequence) = &parenthesized.expression else {
            return;
        };
        let [first, last] = &sequence.expressions[..] else {
            return;
        };
        let zero = matches!(first, Expression::NumericLiteral(literal) if literal.raw.as_deref() == Some("0"));
        let access = matches!(
            last,
            Expression::StaticMemberExpression(_)
                | Expression::ComputedMemberExpression(_)
                | Expression::PrivateFieldExpression(_)
        ) || matches!(last, Expression::Identifier(identifier) if identifier.name == "eval");
        if zero && access {
            self.indirect_call_sequences.insert(sequence.span.start);
        }
    }

    /// tsc's `computeEnumMemberValues` (see `enum_values`): a member without
    /// an initializer takes the previous numeric value plus one, so it needs
    /// one after a string or computed member (TS1061); a `const enum`
    /// initializer must be a constant expression (TS2474) with a finite value
    /// (TS2477, TS2478), and so must a written initializer in an ambient enum
    /// (TS1066).
    fn check_enum_member_values(&mut self, declaration: &oxc_ast::ast::TSEnumDeclaration<'_>) {
        use super::enum_values::{EnumValue, Evaluated};
        let evaluation = super::enum_values::enum_evaluation(declaration);
        for (member, result) in declaration.body.members.iter().zip(&evaluation.members) {
            if result.needs_initializer {
                self.push(Kind::EnumMemberInitializerRequired, member.id.span(), None);
            }
            let Some(initializer) = &member.initializer else {
                continue;
            };
            match &result.value {
                Evaluated::Computed if declaration.r#const => {
                    self.push(Kind::ConstEnumInitializerNotConstant, initializer.span(), None);
                }
                Evaluated::Computed if evaluation.ambient => {
                    self.push(Kind::AmbientEnumInitializerNotConstant, initializer.span(), None);
                }
                Evaluated::Value(EnumValue::Number(number))
                    if declaration.r#const && !number.is_finite() =>
                {
                    let code = if number.is_nan() { 2478 } else { 2477 };
                    self.push(Kind::Ts(code), initializer.span(), None);
                }
                _ => {}
            }
        }
    }

    /// Every member name declared more than once across the declarations of
    /// one enum is TS2300 at each declaration, named as written.
    fn check_duplicate_enum_members(&mut self, declarations: &[&oxc_ast::ast::TSEnumDeclaration<'_>]) {
        let names: Vec<(String, Span)> = declarations
            .iter()
            .flat_map(|declaration| declaration.body.members.iter())
            .filter_map(|member| {
                property_key_name_of_enum_member(&member.id).map(|name| (name, member.id.span()))
            })
            .collect();
        for (name, span) in &names {
            if names.iter().filter(|(other, _)| other == name).count() < 2 {
                continue;
            }
            let written = self
                .source_text
                .get(span.start as usize..span.end as usize)
                .unwrap_or(name)
                .to_string();
            self.push(Kind::DuplicateMember, *span, Some(&written));
        }
    }

    fn check_top_level_enum_merges(&mut self, statements: &[Statement<'_>]) {
        let mut groups: Vec<(&str, Vec<&oxc_ast::ast::TSEnumDeclaration<'_>>)> = Vec::new();
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            let Some(Declaration::TSEnumDeclaration(enumeration)) = declaration else {
                continue;
            };
            let name = enumeration.id.name.as_str();
            match groups.iter_mut().find(|(group, _)| *group == name) {
                Some((_, declarations)) => declarations.push(enumeration),
                None => groups.push((name, vec![enumeration])),
            }
        }
        for (_, declarations) in groups {
            self.check_duplicate_enum_members(&declarations);
        }
    }

    /// tsc's `checkKindsOfPropertyMemberOverrides` for classes whose base is a
    /// class declared at the top of the same file: an instance member may not
    /// change kind between property, accessor and method. A private member on
    /// either side is not an override, and an abstract base property or
    /// accessor may be implemented as either. Each derived name is judged
    /// once, at its first declaration (the symbol's `valueDeclaration`); an
    /// auto-accessor binds as an accessor and a parameter property as a
    /// property.
    fn check_member_kind_overrides(&mut self, statements: &[Statement<'_>]) {
        #[derive(Clone, Copy, PartialEq)]
        enum MemberKind {
            Property,
            Accessor,
            Method,
        }
        struct Member {
            kind: MemberKind,
            is_private: bool,
            is_abstract: bool,
            span: Span,
        }
        fn instance_members(class: &Class<'_>) -> Vec<(String, Member)> {
            let mut members = Vec::new();
            for element in &class.body.body {
                let (key, is_static, member) = match element {
                    ClassElement::MethodDefinition(method) => {
                        let kind = match method.kind {
                            MethodDefinitionKind::Method => MemberKind::Method,
                            MethodDefinitionKind::Get | MethodDefinitionKind::Set => {
                                MemberKind::Accessor
                            }
                            MethodDefinitionKind::Constructor => {
                                for parameter in &method.value.params.items {
                                    if (parameter.accessibility.is_some() || parameter.readonly || parameter.r#override)
                                        && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &parameter.pattern
                                    {
                                        members.push((
                                            identifier.name.to_string(),
                                            Member {
                                                kind: MemberKind::Property,
                                                is_private: parameter.accessibility
                                                    == Some(oxc_ast::ast::TSAccessibility::Private),
                                                is_abstract: false,
                                                span: identifier.span,
                                            },
                                        ));
                                    }
                                }
                                continue;
                            }
                        };
                        (
                            &method.key,
                            method.r#static,
                            Member {
                                kind,
                                is_private: method.accessibility
                                    == Some(oxc_ast::ast::TSAccessibility::Private),
                                is_abstract: method.r#type
                                    == MethodDefinitionType::TSAbstractMethodDefinition,
                                span: method.key.span(),
                            },
                        )
                    }
                    ClassElement::PropertyDefinition(property) => (
                        &property.key,
                        property.r#static,
                        Member {
                            kind: MemberKind::Property,
                            is_private: property.accessibility
                                == Some(oxc_ast::ast::TSAccessibility::Private),
                            is_abstract: property.r#type
                                == PropertyDefinitionType::TSAbstractPropertyDefinition,
                            span: property.key.span(),
                        },
                    ),
                    ClassElement::AccessorProperty(property) => (
                        &property.key,
                        property.r#static,
                        Member {
                            kind: MemberKind::Accessor,
                            is_private: property.accessibility
                                == Some(oxc_ast::ast::TSAccessibility::Private),
                            is_abstract: property.r#type
                                == oxc_ast::ast::AccessorPropertyType::TSAbstractAccessorProperty,
                            span: property.key.span(),
                        },
                    ),
                    _ => continue,
                };
                if is_static || matches!(key, PropertyKey::PrivateIdentifier(_)) {
                    continue;
                }
                if let Some(name) = property_key_name(key) {
                    members.push((name, member));
                }
            }
            members
        }

        let classes: Vec<&Class<'_>> = statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::ClassDeclaration(class) => Some(class.as_ref()),
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                    _ => None,
                },
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // First declaration wins for both lookups, as the linear `find`s did.
        let mut class_slots: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for (slot, class) in classes.iter().enumerate() {
            if let Some(id) = class.id.as_ref() {
                class_slots.entry(id.name.as_str()).or_insert(slot);
            }
        }
        let base_of = |class: &Class<'_>| match &class.super_class {
            Some(Expression::Identifier(base)) if class.super_type_arguments.is_none() => {
                class_slots.get(base.name.as_str()).copied()
            }
            _ => None,
        };
        let member_lists: Vec<Vec<(String, Member)>> =
            classes.iter().map(|class| instance_members(class)).collect();
        let member_maps: Vec<std::collections::HashMap<&str, &Member>> = member_lists
            .iter()
            .map(|members| {
                let mut map = std::collections::HashMap::new();
                for (name, member) in members {
                    map.entry(name.as_str()).or_insert(member);
                }
                map
            })
            .collect();

        for (slot, class) in classes.iter().enumerate() {
            let (Some(derived_name), Some(base)) = (class.id.as_ref(), base_of(class)) else {
                continue;
            };
            let Some(base_name) = classes[base].id.as_ref() else {
                continue;
            };
            let mut judged: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for (name, derived) in &member_lists[slot] {
                if !judged.insert(name.as_str()) {
                    continue;
                }
                let mut ancestor = Some(base);
                let mut found = None;
                let mut depth = 0;
                while let Some(current) = ancestor
                    && depth < 32
                {
                    if let Some(&member) = member_maps[current].get(name.as_str()) {
                        found = Some(member);
                        break;
                    }
                    ancestor = base_of(classes[current]);
                    depth += 1;
                }
                let Some(inherited) = found else {
                    continue;
                };
                if inherited.is_private || derived.is_private {
                    continue;
                }
                let kind = match (inherited.kind, derived.kind) {
                    (MemberKind::Accessor, MemberKind::Property) if !inherited.is_abstract => {
                        Kind::PropertyAccessorOverride
                    }
                    (MemberKind::Property, MemberKind::Accessor) if !inherited.is_abstract => {
                        Kind::AccessorPropertyOverride
                    }
                    (MemberKind::Method, MemberKind::Accessor) => Kind::MethodAccessorOverride,
                    (MemberKind::Property, MemberKind::Method) => Kind::PropertyMethodOverride,
                    (MemberKind::Accessor, MemberKind::Method) => Kind::AccessorMethodOverride,
                    _ => continue,
                };
                let names = format!("{name}\u{0}{}\u{0}{}", base_name.name, derived_name.name);
                self.push(kind, derived.span, Some(&names));
            }
        }
    }

    /// tsc's `pushTypeResolution` cycle for module-level type aliases: an
    /// alias whose body reaches itself through positions resolved eagerly
    /// (union and intersection members, `keyof`, indexed access, a conditional's
    /// check and extends types, template literal spans, a mapped type's
    /// constraint, another alias and its type arguments) circularly references
    /// itself (TS2456). Object members, signatures, array and tuple elements,
    /// conditional branches and the type arguments of other references are
    /// deferred. A `typeof` query of an annotated variable resolves the
    /// annotation (`getTypeOfVariableOrParameterOrProperty`), where only object
    /// members, signatures and conditional branches are deferred, since
    /// `isDeferredTypeReferenceNode` defers the rest only under an alias; such a
    /// variable on the cycle is TS2502. Only the declarations on the cycle are
    /// reported, not those that merely lead into one.
    fn check_circular_type_aliases(&mut self, statements: &[Statement<'_>]) {
        use oxc_ast::ast::{
            BindingIdentifier, BindingPattern, TSTupleElement, TSType, TSTypeAliasDeclaration,
            TSTypeName, TSTypeOperatorOperator, TSTypeQueryExprName,
        };
        let mut aliases: Vec<&TSTypeAliasDeclaration<'_>> = Vec::new();
        let mut variables: Vec<(&BindingIdentifier<'_>, &TSType<'_>)> = Vec::new();
        // Functions whose return type is written, and unannotated variables
        // initialized by calling one (`var v = f()`, typed by `f`'s return).
        let mut functions: Vec<(&str, &oxc_ast::ast::TSTypeAnnotation<'_>, Vec<&str>)> = Vec::new();
        let mut call_variables: Vec<(&str, &str)> = Vec::new();
        let mut function_names: Vec<&str> = Vec::new();
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            match declaration {
                Some(Declaration::TSTypeAliasDeclaration(alias)) => aliases.push(alias.as_ref()),
                Some(Declaration::VariableDeclaration(variable)) => {
                    for declarator in &variable.declarations {
                        let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                            continue;
                        };
                        // A redeclared `var` is typed by its first declaration.
                        if variables.iter().any(|(known, _)| known.name == identifier.name)
                            || call_variables.iter().any(|(known, _)| *known == identifier.name.as_str())
                        {
                            continue;
                        }
                        if let Some(annotation) = &declarator.type_annotation {
                            variables.push((identifier.as_ref(), &annotation.type_annotation));
                        } else if let Some(oxc_ast::ast::Expression::CallExpression(call)) =
                            declarator.init.as_ref().map(|init| init.without_parentheses())
                            && let oxc_ast::ast::Expression::Identifier(callee) = &call.callee
                        {
                            call_variables.push((identifier.name.as_str(), callee.name.as_str()));
                        }
                    }
                }
                Some(Declaration::FunctionDeclaration(function)) => {
                    if let Some(id) = &function.id {
                        function_names.push(id.name.as_str());
                        if let Some(return_type) = &function.return_type
                            && function.body.is_some()
                        {
                            functions.push((
                                id.name.as_str(),
                                return_type.as_ref(),
                                type_parameter_names(function.type_parameters.as_deref()),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        // An overloaded function's type is its overloads', not one return.
        functions.retain(|(name, _, _)| function_names.iter().filter(|known| *known == name).count() == 1);
        // A type alias sharing an import's name merges with the import, which
        // its references resolve through (`resolveAlias`): `type E = E` names
        // the imported `E` (TS2440 is reported for the conflict instead).
        let imported: std::collections::HashSet<&str> = statements
            .iter()
            .flat_map(|statement| match statement {
                Statement::ImportDeclaration(import) => import
                    .specifiers
                    .iter()
                    .flatten()
                    .map(|specifier| specifier.local().name.as_str())
                    .collect::<Vec<_>>(),
                Statement::TSImportEqualsDeclaration(import) => vec![import.id.name.as_str()],
                _ => Vec::new(),
            })
            .collect();
        aliases.retain(|alias| !imported.contains(alias.id.name.as_str()));

        struct Declarations<'n> {
            aliases: Vec<&'n str>,
            variables: Vec<&'n str>,
            call_variables: Vec<&'n str>,
            functions: Vec<&'n str>,
        }
        let declarations = Declarations {
            aliases: aliases.iter().map(|alias| alias.id.name.as_str()).collect(),
            variables: variables.iter().map(|(identifier, _)| identifier.name.as_str()).collect(),
            call_variables: call_variables.iter().map(|(name, _)| *name).collect(),
            functions: functions.iter().map(|(name, _, _)| *name).collect(),
        };
        // Declarations are numbered aliases, annotated variables, call-initialized
        // variables, then functions.
        let call_variable_base = aliases.len() + variables.len();
        let function_base = call_variable_base + call_variables.len();

        fn tuple_element_type<'e, 'a>(element: &'e TSTupleElement<'a>) -> Option<&'e TSType<'a>> {
            match element {
                TSTupleElement::TSOptionalType(optional) => Some(&optional.type_annotation),
                TSTupleElement::TSRestType(rest) => Some(&rest.type_annotation),
                TSTupleElement::TSNamedTupleMember(member) => tuple_element_type(&member.element_type),
                other => other.as_ts_type(),
            }
        }

        fn type_parameter_names<'n>(
            parameters: Option<&'n oxc_ast::ast::TSTypeParameterDeclaration<'_>>,
        ) -> Vec<&'n str> {
            parameters
                .map(|parameters| parameters.params.iter().map(|parameter| parameter.name.name.as_str()).collect())
                .unwrap_or_default()
        }

        // `annotation`: the type is a variable's annotation rather than an
        // alias body. Declarations are numbered aliases first, then variables.
        // `shadowed`: the type parameters in scope, which no alias of the same
        // name is referenced through.
        fn eager_references(
            ty: &TSType<'_>,
            annotation: bool,
            declarations: &Declarations<'_>,
            shadowed: &[&str],
            out: &mut Vec<usize>,
        ) {
            match ty {
                TSType::TSTypeReference(reference) => {
                    let TSTypeName::IdentifierReference(identifier) = &reference.type_name else {
                        return;
                    };
                    if shadowed.contains(&identifier.name.as_str()) {
                        return;
                    }
                    let alias = declarations
                        .aliases
                        .iter()
                        .position(|name| *name == identifier.name.as_str());
                    if let Some(index) = alias {
                        out.push(index);
                    }
                    // `ReturnType<typeof f>` evaluates the conditional over the
                    // function's signature, which needs its return type.
                    if alias.is_none()
                        && identifier.name == "ReturnType"
                        && let Some(arguments) = &reference.type_arguments
                        && let [TSType::TSTypeQuery(query)] = arguments.params.as_slice()
                        && let TSTypeQueryExprName::IdentifierReference(function) = &query.expr_name
                        && let Some(index) = declarations.functions.iter().position(|name| *name == function.name.as_str())
                    {
                        out.push(
                            declarations.aliases.len()
                                + declarations.variables.len()
                                + declarations.call_variables.len()
                                + index,
                        );
                    }
                    if (alias.is_some() || annotation)
                        && let Some(arguments) = &reference.type_arguments
                    {
                        for argument in &arguments.params {
                            eager_references(argument, annotation, declarations, shadowed, out);
                        }
                    }
                }
                TSType::TSTypeQuery(query) => {
                    if let TSTypeQueryExprName::IdentifierReference(identifier) = &query.expr_name {
                        let name = identifier.name.as_str();
                        if let Some(index) = declarations.variables.iter().position(|known| *known == name) {
                            out.push(declarations.aliases.len() + index);
                        } else if let Some(index) = declarations.call_variables.iter().position(|known| *known == name) {
                            out.push(declarations.aliases.len() + declarations.variables.len() + index);
                        }
                    }
                }
                TSType::TSArrayType(array) if annotation => {
                    eager_references(&array.element_type, annotation, declarations, shadowed, out);
                }
                TSType::TSTupleType(tuple) if annotation => {
                    for element in &tuple.element_types {
                        if let Some(element) = tuple_element_type(element) {
                            eager_references(element, annotation, declarations, shadowed, out);
                        }
                    }
                }
                // `getTypeFromMappedTypeNode` resolves the constraint eagerly.
                TSType::TSMappedType(mapped) => {
                    eager_references(&mapped.constraint, annotation, declarations, shadowed, out);
                }
                TSType::TSUnionType(union) => {
                    for member in &union.types {
                        eager_references(member, annotation, declarations, shadowed, out);
                    }
                }
                TSType::TSIntersectionType(intersection) => {
                    for member in &intersection.types {
                        eager_references(member, annotation, declarations, shadowed, out);
                    }
                }
                TSType::TSParenthesizedType(inner) => {
                    eager_references(&inner.type_annotation, annotation, declarations, shadowed, out);
                }
                TSType::TSTypeOperatorType(operator)
                    if operator.operator == TSTypeOperatorOperator::Keyof
                        || (annotation && operator.operator == TSTypeOperatorOperator::Readonly) =>
                {
                    eager_references(&operator.type_annotation, annotation, declarations, shadowed, out);
                }
                TSType::TSIndexedAccessType(access) => {
                    eager_references(&access.object_type, annotation, declarations, shadowed, out);
                    eager_references(&access.index_type, annotation, declarations, shadowed, out);
                }
                TSType::TSConditionalType(conditional) => {
                    eager_references(&conditional.check_type, annotation, declarations, shadowed, out);
                    eager_references(&conditional.extends_type, annotation, declarations, shadowed, out);
                }
                TSType::TSTemplateLiteralType(template) => {
                    for span in &template.types {
                        eager_references(span, annotation, declarations, shadowed, out);
                    }
                }
                _ => {}
            }
        }

        let mut edges: Vec<Vec<usize>> = aliases
            .iter()
            .map(|alias| (&alias.type_annotation, false, type_parameter_names(alias.type_parameters.as_deref())))
            .chain(variables.iter().map(|(_, annotation)| (*annotation, true, Vec::new())))
            .map(|(ty, annotation, shadowed)| {
                let mut referenced = Vec::new();
                eager_references(ty, annotation, &declarations, &shadowed, &mut referenced);
                referenced
            })
            .collect();
        edges.extend(call_variables.iter().map(|(_, callee)| {
            declarations
                .functions
                .iter()
                .position(|name| name == callee)
                .map(|index| vec![function_base + index])
                .unwrap_or_default()
        }));
        edges.extend(functions.iter().map(|(_, return_type, shadowed)| {
            let mut referenced = Vec::new();
            eager_references(&return_type.type_annotation, true, &declarations, shadowed, &mut referenced);
            referenced
        }));
        let reaches = |from: Vec<usize>, target: usize| {
            let mut visited = vec![false; edges.len()];
            let mut stack = from;
            while let Some(next) = stack.pop() {
                if next == target {
                    return true;
                }
                if std::mem::replace(&mut visited[next], true) {
                    continue;
                }
                stack.extend(edges[next].iter().copied());
            }
            false
        };
        // A mapped type on the cycle whose constraint leads back into it: the
        // key's constraint is resolving when the cycle is detected (TS2313).
        fn eager_mapped<'t, 'a>(ty: &'t TSType<'a>, out: &mut Vec<&'t oxc_ast::ast::TSMappedType<'a>>) {
            match ty {
                TSType::TSMappedType(mapped) => out.push(mapped),
                TSType::TSUnionType(union) => union.types.iter().for_each(|member| eager_mapped(member, out)),
                TSType::TSIntersectionType(intersection) => {
                    intersection.types.iter().for_each(|member| eager_mapped(member, out))
                }
                TSType::TSParenthesizedType(inner) => eager_mapped(&inner.type_annotation, out),
                _ => {}
            }
        }
        for (index, reachable) in edges.iter().enumerate() {
            if !reaches(reachable.clone(), index) {
                continue;
            }
            match aliases.get(index) {
                Some(alias) => {
                    self.push(Kind::CircularTypeAlias, alias.id.span, Some(alias.id.name.as_str()));
                    let mut mapped_types = Vec::new();
                    eager_mapped(&alias.type_annotation, &mut mapped_types);
                    let shadowed = type_parameter_names(alias.type_parameters.as_deref());
                    for mapped in mapped_types {
                        let mut referenced = Vec::new();
                        eager_references(&mapped.constraint, false, &declarations, &shadowed, &mut referenced);
                        if reaches(referenced, index) {
                            self.push(Kind::Ts(2313), mapped.constraint.span(), Some(mapped.key.name.as_str()));
                        }
                    }
                }
                None if index < call_variable_base => {
                    let (identifier, _) = variables[index - aliases.len()];
                    self.push(Kind::Ts(2502), identifier.span, Some(identifier.name.as_str()));
                }
                // An unannotated variable's circularity is its own (TS7022).
                None if index < function_base => {}
                None => {
                    let (_, return_type, _) = &functions[index - function_base];
                    self.push(Kind::Ts(2577), return_type.type_annotation.span(), None);
                }
            }
        }
    }

    fn collect_top_level_enums(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            if let Some(Declaration::TSEnumDeclaration(enumeration)) = declaration {
                self.top_level_enums
                    .insert((enumeration.span.start, enumeration.span.end));
            }
        }
    }

    /// tsc's `checkReturnTypeAnnotation` for an `async` signature: the written
    /// return type must be a reference to the global `Promise`. A written type
    /// reference is left alone, since an alias may resolve to `Promise`.
    fn check_async_return_type(&mut self, return_type: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>) {
        let Some(annotation) = return_type else {
            return;
        };
        let written = &annotation.type_annotation;
        if let oxc_ast::ast::TSType::TSTypeReference(reference) = written {
            let name_span = reference.type_name.span();
            let name = &self.source_text[name_span.start as usize..name_span.end as usize];
            let argument = reference
                .type_arguments
                .as_ref()
                .and_then(|arguments| arguments.params.first())
                .map(|argument| {
                    let span = argument.span();
                    self.source_text[span.start as usize..span.end as usize].to_string()
                })
                .unwrap_or_default();
            let payload = format!("{name}\0{argument}");
            let span = written.span();
            self.push(Kind::AsyncReturnTypeReference, span, Some(&payload));
            return;
        }
        if matches!(
            written,
            oxc_ast::ast::TSType::TSTypeQuery(_)
                | oxc_ast::ast::TSType::TSImportType(_)
                | oxc_ast::ast::TSType::TSIndexedAccessType(_)
                | oxc_ast::ast::TSType::TSConditionalType(_)
                | oxc_ast::ast::TSType::TSParenthesizedType(_)
        ) {
            return;
        }
        let span = written.span();
        let text = self
            .source_text
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default()
            .to_string();
        self.push(Kind::AsyncReturnTypeNotPromise, span, Some(&text));
    }

    /// tsc's `IsInTopLevelContext`: no function, class body or namespace
    /// encloses the node.
    fn in_top_level_context(&self) -> bool {
        self.function_async.is_empty() && self.this_container_depth == 0
    }

    fn push(&mut self, kind: Kind, span: Span, name: Option<&str>) {
        self.diagnostics.push(ParsedGrammarDiagnostic {
            kind,
            span: text_span_from_oxc_span(span),
            name: name.map(str::to_string),
        });
    }

    /// tsc's `checkTruthinessOfType` diagnostic, which depends only on syntax.
    fn check_truthiness(&mut self, expression: &Expression<'_>) {
        match syntactic_truthiness(expression) {
            Some(true) => self.push(Kind::AlwaysTruthyExpression, expression.span(), None),
            Some(false) => self.push(Kind::AlwaysFalsyExpression, expression.span(), None),
            None => {}
        }
    }

    fn is_ambient(&self) -> bool {
        self.ambient_depth > 0
    }

    /// tsc's `checkExportDeclaration` inside a namespace body: `export … from
    /// "m"` is TS1194 on the module name in any namespace, and a local
    /// `export { … }` is TS1194 on the declaration unless the namespace is
    /// ambient.
    fn check_namespace_export_declarations(&mut self, statements: &[Statement<'_>], ambient: bool) {
        for statement in statements {
            match statement {
                Statement::ExportNamedDeclaration(export) if export.declaration.is_none() => {
                    match export.source.as_ref() {
                        Some(source) => {
                            self.push(Kind::ExportDeclarationInNamespace, source.span, None)
                        }
                        None if !ambient => {
                            self.push(Kind::ExportDeclarationInNamespace, export.span, None)
                        }
                        None => {}
                    }
                }
                Statement::ExportAllDeclaration(export) => {
                    self.push(Kind::ExportDeclarationInNamespace, export.source.span, None);
                }
                _ => {}
            }
        }
    }

    /// `export default` appearing more than once in the module: tsc reports
    /// every one of them, not just the later.
    fn check_default_exports(&mut self, statements: &[Statement<'_>]) {
        let mut defaults: Vec<(Span, bool)> = Vec::new();

        for statement in statements {
            match statement {
                Statement::ExportDefaultDeclaration(export) => {
                    let (span, is_declaration) = match &export.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                            (function.id.as_ref().map_or(export.span, |id| id.span), true)
                        }
                        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                            (class.id.as_ref().map_or(export.span, |id| id.span), true)
                        }
                        ExportDefaultDeclarationKind::Identifier(identifier) => {
                            (identifier.span, false)
                        }
                        _ => (export.span, false),
                    };
                    defaults.push((span, is_declaration));
                }
                Statement::ExportNamedDeclaration(export) => {
                    for specifier in &export.specifiers {
                        if export_name(&specifier.exported) == "default" {
                            defaults.push((specifier.exported.span(), false));
                        }
                    }
                }
                _ => {}
            }
        }

        if defaults.len() < 2 {
            return;
        }
        // Two default *declarations* are an attempted merge, which tsc reports
        // with the redeclaration diagnostics (TS2323 and friends) instead.
        if defaults
            .iter()
            .filter(|(_, is_declaration)| *is_declaration)
            .count()
            >= 2
        {
            return;
        }
        for (span, _) in defaults {
            self.push(Kind::MultipleDefaultExports, span, None);
        }
    }

    /// tsc's `checkFunctionOrConstructorSymbol` for the function declarations
    /// of one statement list.
    fn check_function_implementations(&mut self, statements: &[Statement<'_>]) {
        let siblings: Vec<OverloadSibling> = statements
            .iter()
            .map(|statement| {
                let (function, exported) = match statement {
                    Statement::FunctionDeclaration(function) => (Some(&**function), false),
                    Statement::ExportNamedDeclaration(export) => match &export.declaration {
                        Some(Declaration::FunctionDeclaration(function)) => (Some(&**function), true),
                        _ => (None, false),
                    },
                    _ => (None, false),
                };
                let Some(function) = function else {
                    return OverloadSibling::other();
                };
                let Some(id) = function.id.as_ref() else {
                    return OverloadSibling::other();
                };
                OverloadSibling {
                    kind: SiblingKind::Function,
                    name: Some(id.name.to_string()),
                    name_span: id.span,
                    is_static: false,
                    has_body: function.body.is_some(),
                    ambient: function.declare || self.is_ambient(),
                    exported,
                    accessibility: None,
                    is_abstract: false,
                    optional: false,
                }
            })
            .collect();
        self.check_overload_groups(&siblings);
    }

    /// `checkFunctionOrConstructorSymbol` for a block's or a function body's
    /// own function declarations (the top level is the checker's): a name
    /// implemented twice is TS2393 at each of its declarations.
    fn check_nested_duplicate_implementations(&mut self, statements: &[Statement<'_>]) {
        let mut groups: Vec<(&str, Vec<(Span, bool)>)> = Vec::new();
        for statement in statements {
            let function = match statement {
                Statement::FunctionDeclaration(function) => &**function,
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::FunctionDeclaration(function)) => &**function,
                    _ => continue,
                },
                _ => continue,
            };
            let Some(id) = function.id.as_ref() else {
                continue;
            };
            let entry = (id.span, function.body.is_some());
            match groups.iter_mut().find(|(name, _)| *name == id.name.as_str()) {
                Some((_, members)) => members.push(entry),
                None => groups.push((id.name.as_str(), vec![entry])),
            }
        }
        for (_, members) in groups {
            if members.iter().filter(|(_, has_body)| *has_body).count() > 1 {
                for (span, _) in members {
                    self.push(Kind::DuplicateImplementation, span, None);
                }
            }
        }
    }

    /// The overload half of tsc's `checkFunctionOrConstructorSymbolWorker`
    /// over one container's declarations, grouped the way the binder forms
    /// symbols: by name, and for class members by staticness.
    fn check_overload_groups(&mut self, siblings: &[OverloadSibling]) {
        let mut groups: Vec<(&str, bool, Vec<usize>)> = Vec::new();
        for (index, sibling) in siblings.iter().enumerate() {
            let Some(name) = sibling.name.as_deref() else {
                continue;
            };
            if sibling.kind == SiblingKind::Other {
                continue;
            }
            match groups
                .iter_mut()
                .find(|(group, is_static, _)| *group == name && *is_static == sibling.is_static)
            {
                Some((_, _, indices)) => indices.push(index),
                None => groups.push((name, sibling.is_static, vec![index])),
            }
        }
        for (_, _, indices) in groups {
            let mut previous: Option<usize> = None;
            let mut last_non_ambient: Option<usize> = None;
            let mut body_seen = false;
            for &index in &indices {
                let sibling = &siblings[index];
                if sibling.ambient {
                    previous = None;
                }
                if !(sibling.has_body && body_seen)
                    && let Some(previous) = previous
                    && previous + 1 != index
                {
                    self.report_implementation_expected(siblings, previous);
                }
                body_seen |= sibling.has_body;
                previous = Some(index);
                if !sibling.ambient {
                    last_non_ambient = Some(index);
                }
            }
            if let Some(last) = last_non_ambient
                && !siblings[last].has_body
                && !siblings[last].is_abstract
                && !siblings[last].optional
            {
                self.report_implementation_expected(siblings, last);
            }
            if indices.iter().any(|&index| !siblings[index].has_body) {
                self.check_overload_flag_agreement(siblings, &indices);
            }
        }
    }

    /// The `[ … ]` around a computed key, which is the name tsc reports at.
    fn bracketed_name_span(&self, key: Span) -> Span {
        let before = &self.source_text[..key.start as usize];
        let after = &self.source_text[key.end as usize..];
        match (before.rfind('['), after.find(']')) {
            (Some(open), Some(close)) => Span::new(open as u32, key.end + close as u32 + 1),
            _ => key,
        }
    }

    /// An accessor's property key, how `symbolToString` spells it (a computed
    /// name as written, brackets included) and where it is reported.
    fn accessor_name(&self, key: &PropertyKey<'_>, computed: bool) -> Option<(String, Option<String>, Span)> {
        if !computed {
            return Some((property_key_name(key)?, accessor_display_name(key), key.span()));
        }
        let span = self.bracketed_name_span(key.span());
        let written = self.source_text.get(span.start as usize..span.end as usize)?.to_string();
        Some((property_key_name(key).unwrap_or_else(|| written.clone()), Some(written), span))
    }

    fn is_bracketed_name(&self, span: Span) -> bool {
        self.source_text.as_bytes().get(span.start as usize) == Some(&b'[')
    }

    /// tsc's `reportImplementationExpectedError`: the declaration right after
    /// a bodyless one decides the message.
    fn report_implementation_expected(&mut self, siblings: &[OverloadSibling], index: usize) {
        let node = &siblings[index];
        if let Some(next) = siblings.get(index + 1)
            && next.kind == node.kind
        {
            // tsc matches a computed name only against another computed name.
            if next.name.is_some()
                && next.name == node.name
                && self.is_bracketed_name(next.name_span) == self.is_bracketed_name(node.name_span)
            {
                if node.kind == SiblingKind::Method && node.is_static != next.is_static {
                    let code = if node.is_static { 2387 } else { 2388 };
                    self.push(Kind::Ts(code), next.name_span, None);
                }
                return;
            }
            if next.has_body {
                let display = if self.is_bracketed_name(node.name_span) {
                    Some(self.source_text[node.name_span.start as usize..node.name_span.end as usize].to_string())
                } else {
                    node.name.clone()
                };
                self.push(Kind::Ts(2389), next.name_span, display.as_deref());
                return;
            }
        }
        if node.is_abstract {
            self.push(Kind::Ts(2516), node.name_span, None);
        } else {
            self.push(Kind::FunctionImplementationMissing, node.name_span, None);
        }
    }

    /// tsc's `checkFlagAgreementBetweenOverloads` and
    /// `checkQuestionTokenAgreementBetweenOverloads`, measured against the
    /// implementation when there is one and the first declaration otherwise.
    fn check_overload_flag_agreement(&mut self, siblings: &[OverloadSibling], indices: &[usize]) {
        let canonical = indices
            .iter()
            .copied()
            .find(|&index| siblings[index].has_body)
            .unwrap_or(indices[0]);
        let canonical = &siblings[canonical];
        for &index in indices {
            let sibling = &siblings[index];
            let code = if sibling.exported != canonical.exported {
                Some(2383)
            } else if sibling.ambient != canonical.ambient {
                Some(2384)
            } else if sibling.accessibility != canonical.accessibility {
                Some(2385)
            } else if sibling.is_abstract != canonical.is_abstract {
                Some(2512)
            } else {
                None
            };
            if let Some(code) = code {
                self.push(Kind::Ts(code), sibling.name_span, None);
            }
        }
        for &index in indices {
            let sibling = &siblings[index];
            if sibling.optional != canonical.optional {
                self.push(Kind::Ts(2386), sibling.name_span, None);
            }
        }
    }

    /// tsc's `checkAccessorDeclaration` for a `get`/`set` pair of one name: both
    /// or neither `abstract` (TS2676), and a getter no less accessible than its
    /// setter (TS2808). Each is reported on both accessors.
    fn check_accessor_pairs(&mut self, class: &Class<'_>) {
        use oxc_ast::ast::TSAccessibility;
        let accessors: Vec<(String, bool, bool, &oxc_ast::ast::MethodDefinition<'_>)> = class
            .body
            .body
            .iter()
            .filter_map(|element| match element {
                ClassElement::MethodDefinition(method)
                    if matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set) =>
                {
                    property_key_name(&method.key).map(|name| {
                        (name, method.r#static, method.kind == MethodDefinitionKind::Get, method.as_ref())
                    })
                }
                _ => None,
            })
            .collect();
        for (name, is_static, is_getter, getter) in &accessors {
            if !is_getter {
                continue;
            }
            let Some((_, _, _, setter)) = accessors
                .iter()
                .find(|(other, other_static, other_getter, _)| {
                    other == name && other_static == is_static && !other_getter
                })
            else {
                continue;
            };
            let abstract_of = |method: &oxc_ast::ast::MethodDefinition<'_>| {
                method.r#type == MethodDefinitionType::TSAbstractMethodDefinition
            };
            if abstract_of(getter) != abstract_of(setter) {
                self.push(Kind::AccessorAbstractMismatch, getter.key.span(), None);
                self.push(Kind::AccessorAbstractMismatch, setter.key.span(), None);
            }
            let less_accessible = match (getter.accessibility, setter.accessibility) {
                (Some(TSAccessibility::Protected), None | Some(TSAccessibility::Public)) => true,
                (Some(TSAccessibility::Private), setter) => setter != Some(TSAccessibility::Private),
                _ => false,
            };
            if less_accessible {
                self.push(Kind::GetAccessorLessAccessible, getter.key.span(), None);
                self.push(Kind::GetAccessorLessAccessible, setter.key.span(), None);
            }
        }
    }

    /// tsc's `getTypeOfAccessors` when neither accessor of a name is typed: no
    /// getter return annotation, no getter body to infer from and no setter
    /// parameter annotation leave the property an implicit `any`, reported on
    /// the setter (TS7032) unless it is private in an ambient context. Only
    /// names whose `symbolToString` surge can spell are reported.
    fn check_untyped_setters(&mut self, accessors: &[AccessorRecord], ambient: bool) {
        for setter in accessors.iter().filter(|accessor| !accessor.is_getter) {
            let Some(display) = setter.display.as_deref() else {
                continue;
            };
            if setter.annotated || (setter.private && ambient) {
                continue;
            }
            let getter = accessors.iter().find(|accessor| {
                accessor.is_getter && accessor.key == setter.key && accessor.is_static == setter.is_static
            });
            if getter.is_some_and(|getter| getter.annotated || getter.has_body) {
                continue;
            }
            // A private setter in a declaration file is ambient too, which
            // only the checker knows.
            let payload = if setter.private { format!("{display}\0private") } else { display.to_string() };
            self.push(Kind::Ts(7032), setter.span, Some(&payload));
        }
    }

    /// A member name `[Name]` in a module-level type literal or interface,
    /// which tsc resolves as a value; see [`Kind::ComputedTypeMemberName`].
    fn check_computed_type_member_names(&mut self, members: &[TSSignature<'_>], type_literal: bool) {
        if !self.function_async.is_empty() || self.nested_scope_depth > 0 {
            return;
        }
        for member in members {
            let (key, mapped) = match member {
                TSSignature::TSPropertySignature(property) if property.computed => {
                    (&property.key, type_literal && members.len() == 1)
                }
                TSSignature::TSMethodSignature(method) if method.computed => (&method.key, false),
                _ => continue,
            };
            if let PropertyKey::Identifier(identifier) = key {
                let payload = format!("{}\0{}", identifier.name, u8::from(mapped));
                self.push(Kind::ComputedTypeMemberName, identifier.span, Some(&payload));
            } else if let PropertyKey::AssignmentExpression(assignment) = key
                && let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left
            {
                // `[a = 0]`, once an index signature: the target is still read.
                let payload = format!("{}\00", identifier.name);
                self.push(Kind::ComputedTypeMemberName, identifier.span, Some(&payload));
            } else if let PropertyKey::StaticMemberExpression(member) = key {
                // `[E.A]`: the entity's root is resolved as a value.
                let mut object = &member.object;
                while let Expression::StaticMemberExpression(inner) = object {
                    object = &inner.object;
                }
                if let Expression::Identifier(root) = object {
                    let payload = format!("{}\0dotted", root.name);
                    self.push(Kind::ComputedTypeMemberName, root.span, Some(&payload));
                }
            }
            // An interface's keys are typed where its statement is checked.
            if type_literal
                && let Some(path) = entity_name_text(key)
            {
                let span = self.bracketed_key_span(key.span());
                self.push(Kind::ComputedTypeLiteralKey, span, Some(path.as_str()));
            }
        }
    }

    /// A computed key's span widened to its brackets, where tsc reports the
    /// name.
    fn bracketed_key_span(&self, key: Span) -> Span {
        let open = self.source_text[..key.start as usize].rfind('[').map_or(key.start, |open| open as u32);
        let close = self.source_text[key.end as usize..]
            .find(']')
            .map_or(key.end, |offset| key.end + offset as u32 + 1);
        Span::new(open, close)
    }

    /// tsc's `checkResolvedBlockScopedVariable` inside a function body: a
    /// class the body declares is in its temporal dead zone until evaluated, so
    /// a read written before it that runs with the body (not deferred in a
    /// nested function or class member) is TS2449. Type positions are exempt.
    /// The checker does the same for a file's top level.
    fn check_body_class_forward_references(&mut self, statements: &[Statement<'_>]) {
        struct EarlyReads<'n> {
            classes: &'n [(&'n str, u32)],
            found: Vec<(String, Span)>,
        }
        impl<'a> Visit<'a> for EarlyReads<'_> {
            fn visit_identifier_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'a>) {
                if self
                    .classes
                    .iter()
                    .any(|(name, start)| *name == identifier.name.as_str() && identifier.span.start < *start)
                {
                    self.found.push((identifier.name.to_string(), identifier.span));
                }
            }
            fn visit_function(&mut self, _: &Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
            fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
            fn visit_class(&mut self, class: &Class<'a>) {
                if let Some(base) = &class.super_class {
                    self.visit_expression(base);
                }
            }
            fn visit_ts_type(&mut self, _: &oxc_ast::ast::TSType<'a>) {}
            fn visit_ts_type_annotation(&mut self, _: &oxc_ast::ast::TSTypeAnnotation<'a>) {}
            fn visit_ts_type_parameter_instantiation(&mut self, _: &oxc_ast::ast::TSTypeParameterInstantiation<'a>) {}
            fn visit_ts_interface_declaration(&mut self, _: &oxc_ast::ast::TSInterfaceDeclaration<'a>) {}
            fn visit_ts_type_alias_declaration(&mut self, _: &oxc_ast::ast::TSTypeAliasDeclaration<'a>) {}
            fn visit_ts_module_declaration(&mut self, _: &TSModuleDeclaration<'a>) {}
        }
        let classes: Vec<(&str, u32)> = statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::ClassDeclaration(class) if !class.declare => {
                    class.id.as_ref().map(|id| (id.name.as_str(), id.span.start))
                }
                _ => None,
            })
            .collect();
        if classes.is_empty() {
            return;
        }
        let mut reads = EarlyReads { classes: &classes, found: Vec::new() };
        for statement in statements {
            reads.visit_statement(statement);
        }
        for (name, span) in reads.found {
            self.push(Kind::Ts(2449), span, Some(&name));
        }
    }

    /// TS2302: a static index signature (`static [x: string]: T`) may not name
    /// a type parameter of its class. The checker reports static properties
    /// and methods.
    fn check_static_index_signature_type_parameters(&mut self, class: &Class<'_>) {
        struct TypeParameterReads<'n> {
            names: &'n [&'n str],
            found: Vec<Span>,
        }
        impl<'a> Visit<'a> for TypeParameterReads<'_> {
            fn visit_ts_type_reference(&mut self, reference: &oxc_ast::ast::TSTypeReference<'a>) {
                if let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name
                    && self.names.contains(&name.name.as_str())
                {
                    self.found.push(name.span);
                }
                oxc_ast_visit::walk::walk_ts_type_reference(self, reference);
            }
        }
        let Some(type_parameters) = &class.type_parameters else {
            return;
        };
        let names: Vec<&str> = type_parameters.params.iter().map(|parameter| parameter.name.name.as_str()).collect();
        for element in &class.body.body {
            let ClassElement::TSIndexSignature(signature) = element else {
                continue;
            };
            if !signature.r#static {
                continue;
            }
            let mut reads = TypeParameterReads { names: &names, found: Vec::new() };
            reads.visit_ts_type_annotation(&signature.type_annotation);
            for span in reads.found {
                self.push(Kind::Ts(2302), span, None);
            }
        }
    }

    /// tsc's "A 'get' accessor must return a value" (TS2378): a body with no
    /// `return` at all whose end is reachable. Reachability is approximated by
    /// the body not ending in `throw`.
    fn check_getter_returns(&mut self, name_span: Span, body: Option<&FunctionBody<'_>>) {
        let Some(body) = body else {
            return;
        };
        if self.is_ambient()
            || matches!(body.statements.last(), Some(Statement::ThrowStatement(_)))
            || body_has_return(body)
        {
            return;
        }
        self.push(Kind::GetAccessorWithoutReturn, name_span, None);
    }

    /// The class-method overload checks. A name that is also a property or an
    /// accessor is a duplicate, not an overload set, and reports as one.
    fn check_method_overloads(&mut self, class: &Class<'_>) {
        let conflicting: Vec<(String, bool)> = class
            .body
            .body
            .iter()
            .filter_map(|element| match element {
                ClassElement::PropertyDefinition(property) => {
                    property_key_name(&property.key).map(|name| (name, property.r#static))
                }
                ClassElement::MethodDefinition(method)
                    if matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set) =>
                {
                    property_key_name(&method.key).map(|name| (name, method.r#static))
                }
                _ => None,
            })
            .collect();
        let siblings: Vec<OverloadSibling> = class
            .body
            .body
            .iter()
            .map(|element| {
                let ClassElement::MethodDefinition(method) = element else {
                    return OverloadSibling::other();
                };
                if method.kind != MethodDefinitionKind::Method {
                    return OverloadSibling::other();
                }
                // `hasBindableName`: a computed name joins the overload set
                // only when it is a literal, a well-known symbol or a unique
                // symbol.
                let unique_symbol = match &method.key {
                    PropertyKey::Identifier(identifier)
                        if method.computed && self.unique_symbol_consts.contains(identifier.name.as_str()) =>
                    {
                        Some(format!("[{}]", identifier.name))
                    }
                    _ => None,
                };
                if method.computed
                    && unique_symbol.is_none()
                    && !matches!(
                        method.key,
                        PropertyKey::StringLiteral(_)
                            | PropertyKey::NumericLiteral(_)
                            | PropertyKey::StaticMemberExpression(_)
                    )
                {
                    return OverloadSibling::other();
                }
                let Some(name) = unique_symbol.or_else(|| property_key_name(&method.key)) else {
                    return OverloadSibling::other();
                };
                let name_span = if method.computed {
                    self.bracketed_name_span(method.key.span())
                } else {
                    method.key.span()
                };
                if conflicting.iter().any(|(other, is_static)| *other == name && *is_static == method.r#static) {
                    return OverloadSibling::other();
                }
                OverloadSibling {
                    kind: SiblingKind::Method,
                    name: Some(name),
                    name_span,
                    is_static: method.r#static,
                    has_body: method.value.body.is_some(),
                    // A `declare` method (TS1031 itself) is ambient.
                    ambient: writes_declare(&self.source_text, method),
                    exported: false,
                    accessibility: method.accessibility.filter(|accessibility| {
                        *accessibility != oxc_ast::ast::TSAccessibility::Public
                    }),
                    is_abstract: method.r#type == MethodDefinitionType::TSAbstractMethodDefinition,
                    optional: method.optional,
                }
            })
            .collect();
        self.check_overload_groups(&siblings);
        self.check_overload_arity(class);
    }

    /// The arity half of tsc's `isImplementationCompatibleWithOverload` for a
    /// class's method and constructor overloads (TS2394): an implementation
    /// that requires more arguments than an overload declares can never be
    /// compatible, whatever the types say. Only the first overload is judged,
    /// since tsc reports the first incompatible one and a later overload's
    /// verdict would need the types of the ones before it.
    fn check_overload_arity(&mut self, class: &Class<'_>) {
        let mut groups: Vec<((Option<String>, bool, bool), Vec<&oxc_ast::ast::MethodDefinition<'_>>)> = Vec::new();
        for element in &class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            let key = match method.kind {
                MethodDefinitionKind::Constructor => (None, false, true),
                MethodDefinitionKind::Method if !method.computed => {
                    let Some(name) = property_key_name(&method.key) else {
                        continue;
                    };
                    (Some(name), method.r#static, false)
                }
                _ => continue,
            };
            match groups.iter_mut().find(|(group, _)| *group == key) {
                Some((_, members)) => members.push(method),
                None => groups.push((key, vec![method])),
            }
        }
        for (key, members) in groups {
            // A JavaScript constructor's overloads are its JSDoc `@overload`
            // signatures, judged in order the same way.
            if key.2
                && let [implementation] = members.as_slice()
                && implementation.value.body.is_some()
                && let Some(span) = jsdoc_overload_arity_mismatch(implementation)
            {
                self.push(Kind::Ts(2394), Span::new(span.start as u32, span.end as u32), None);
                continue;
            }
            let [first, .., implementation] = members.as_slice() else {
                continue;
            };
            if first.value.body.is_some()
                || implementation.value.body.is_none()
                || members[..members.len() - 1].iter().any(|member| member.value.body.is_some())
                || first.value.params.rest.is_some()
            {
                continue;
            }
            let Some(required) = required_argument_count(&implementation.value.params) else {
                continue;
            };
            if required > first.value.params.items.len() {
                self.push(Kind::Ts(2394), first.key.span(), None);
            }
        }
    }

    /// tsc's `getFlowTypeOfAccessExpression` for `this.p` in the constructor
    /// of `p`'s class, where `p` has no initializer: the flow starts with `p`
    /// unassigned, so a read before any assignment is TS2565 (kept by the
    /// checker only under `strictPropertyInitialization`). Without a flow
    /// graph a read counts as unassigned only when no write to `p` precedes
    /// it anywhere in the constructor, and only for a property whose written
    /// type cannot hold `undefined`.
    fn check_properties_used_before_assigned(&mut self, class: &Class<'_>) {
        let candidates: Vec<String> = class
            .body
            .body
            .iter()
            .filter_map(|element| {
                let ClassElement::PropertyDefinition(property) = element else {
                    return None;
                };
                if property.r#static
                    || property.value.is_some()
                    || property.declare
                    || property.optional
                    || property.definite
                    || property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
                    || !property
                        .type_annotation
                        .as_ref()
                        .is_some_and(|annotation| type_excludes_undefined(&annotation.type_annotation))
                {
                    return None;
                }
                match &property.key {
                    PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
                    PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
                    _ => None,
                }
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let Some(body) = class.body.body.iter().find_map(|element| match element {
            ClassElement::MethodDefinition(method) if method.kind == MethodDefinitionKind::Constructor => {
                method.value.body.as_ref()
            }
            _ => None,
        }) else {
            return;
        };
        let mut accesses = ThisPropertyAccesses::default();
        accesses.visit_function_body(body);
        for (name, span) in &accesses.reads {
            if candidates.contains(name)
                && !accesses
                    .writes
                    .iter()
                    .any(|(written, start)| written == name && *start < span.start)
            {
                self.push(Kind::Ts(2565), *span, Some(name));
            }
        }
    }

    /// TS2717 for a property a type literal declares twice with different
    /// written types, on the later one. A numeric name is its number's
    /// canonical text (`1.0` is `"1"`), as the property's symbol is keyed.
    fn check_conflicting_type_literal_properties(&mut self, members: &[TSSignature<'_>]) {
        let mut first: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for member in members {
            let TSSignature::TSPropertySignature(property) = member else {
                continue;
            };
            if property.computed {
                continue;
            }
            let Some(annotation) = &property.type_annotation else {
                continue;
            };
            let key = match &property.key {
                PropertyKey::StaticIdentifier(identifier) => identifier.name.to_string(),
                PropertyKey::StringLiteral(literal) => literal.value.to_string(),
                PropertyKey::NumericLiteral(literal) => super::number_text::js_number_to_string(literal.value),
                _ => continue,
            };
            let text = |span: Span| {
                self.source_text.get(span.start as usize..span.end as usize).unwrap_or_default().to_string()
            };
            let written = text(annotation.type_annotation.span());
            let normalized = |text: &str| text.chars().filter(|ch| !ch.is_whitespace()).collect::<String>();
            match first.get(&key) {
                Some(expected) if normalized(expected) != normalized(&written) => {
                    let payload = format!("{}\0{expected}\0{written}", text(property.key.span()));
                    self.push(Kind::Ts(2717), property.key.span(), Some(&payload));
                }
                Some(_) => {}
                None => {
                    first.insert(key, written);
                }
            }
        }
    }

    /// Everything one class body can say wrong about its own members: a
    /// constructor group with no implementation (TS2390), two implementations
    /// of one name (TS2393/TS2392), two members declaring the same name where
    /// neither is an overload of the other (TS2300), and the method overload
    /// checks of [`Self::check_method_overloads`].
    ///
    /// Static and instance members are separate names, and an `abstract` or
    /// ambient member has no implementation to miss.
    fn check_class_members(&mut self, class: &Class<'_>) {
        let ambient = self.is_ambient() || class.declare;
        let mut groups: Vec<MemberGroup> = Vec::new();
        let mut group_slots: std::collections::HashMap<(String, bool), usize> =
            std::collections::HashMap::new();
        let mut constructors: Vec<(Span, bool, bool)> = Vec::new();
        let mut constructor_accessibility: Vec<(Span, bool, Option<oxc_ast::ast::TSAccessibility>)> = Vec::new();
        // The first written annotation of each property, by name and side.
        let mut property_annotations: std::collections::HashMap<(String, bool), String> =
            std::collections::HashMap::new();

        for element in &class.body.body {
            let (key, is_static, member) = match element {
                ClassElement::MethodDefinition(method) => {
                    let is_abstract =
                        method.r#type == MethodDefinitionType::TSAbstractMethodDefinition;
                    // `abstract constructor` is TS1242, where tsc stops.
                    if is_abstract
                        && !class.r#abstract
                        && method.kind != MethodDefinitionKind::Constructor
                    {
                        self.push(Kind::AbstractMethodOutsideAbstractClass, method.span, None);
                    }
                    if is_abstract
                        && method.kind == MethodDefinitionKind::Method
                        && method.value.body.is_some()
                    {
                        let span = method.key.span();
                        let written = self
                            .source_text
                            .get(span.start as usize..span.end as usize)
                            .unwrap_or_default()
                            .to_string();
                        self.push(Kind::Ts(1245), span, Some(&written));
                    }
                    if method.kind == MethodDefinitionKind::Constructor {
                        if method.value.body.is_none() {
                            self.check_signature_parameters(&method.value.params);
                        }
                        // `checkGrammarConstructorTypeParameters` and
                        // `…TypeAnnotation` see a JSDoc `@template` and
                        // `@return` as the constructor's own.
                        if method.value.type_parameters.is_none()
                            && let Some(span) = super::jsdoc::type_parameters_at(method.value.span.start)
                                .and_then(|parameters| parameters.first().and_then(|parameter| parameter.name_span))
                        {
                            self.push(Kind::Ts(1092), Span::new(span.start as u32, span.end as u32), None);
                        }
                        if method.value.return_type.is_none()
                            && let Some((_, span)) = super::jsdoc::return_type_at(method.value.span.start)
                        {
                            self.push(Kind::Ts(1093), Span::new(span.start as u32, span.end as u32), None);
                        }
                        constructors.push((
                            method.key.span(),
                            method.value.body.is_some(),
                            is_abstract,
                        ));
                        constructor_accessibility.push((
                            Span::new(method.span.start, method.key.span().end),
                            method.value.body.is_some(),
                            method.accessibility.filter(|accessibility| {
                                *accessibility != oxc_ast::ast::TSAccessibility::Public
                            }),
                        ));
                        // A parameter property is an instance property
                        // declared where the constructor stands.
                        for parameter in &method.value.params.items {
                            if (parameter.accessibility.is_some() || parameter.readonly || parameter.r#override)
                                && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &parameter.pattern
                            {
                                add_group_member(
                                    &mut groups,
                                    &mut group_slots,
                                    identifier.name.to_string(),
                                    false,
                                    (identifier.span, MemberKind::Property),
                                );
                            }
                        }
                        continue;
                    }
                    if matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set)
                        && let PropertyKey::StaticIdentifier(name) = &method.key
                        && name.name.as_str() == "constructor"
                    {
                        self.push(Kind::Ts(1341), name.span, None);
                    }
                    let member = match method.kind {
                        MethodDefinitionKind::Get | MethodDefinitionKind::Set => {
                            MemberKind::Accessor
                        }
                        _ if method.r#type == MethodDefinitionType::TSAbstractMethodDefinition => {
                            MemberKind::AbstractMethod
                        }
                        _ => MemberKind::Method {
                            has_body: method.value.body.is_some(),
                        },
                    };
                    (&method.key, method.r#static, member)
                }
                ClassElement::PropertyDefinition(property) => {
                    if property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
                        && !class.r#abstract
                    {
                        // tsc reports on the modifier, which is where the
                        // member's own span starts.
                        self.push(
                            Kind::AbstractPropertyOutsideAbstractClass,
                            property.span,
                            None,
                        );
                    }
                    // A `declare` field is ambient itself. tsc checks the
                    // initializer in `checkGrammarProperty`, which a decorator on
                    // such a field never reaches (`checkGrammarModifiers` fails
                    // first).
                    if (ambient || property.declare)
                        && !(property.declare && !property.decorators.is_empty())
                        && let Some(value) = property.value.as_ref()
                    {
                        self.check_ambient_initializer(
                            value,
                            property.readonly,
                            property.type_annotation.is_some(),
                        );
                    }
                    (&property.key, property.r#static, MemberKind::Property)
                }
                ClassElement::AccessorProperty(property) => (&property.key, property.r#static, MemberKind::Accessor),
                _ => continue,
            };

            let Some(name) = property_key_name(key) else {
                continue;
            };
            let computed = match element {
                ClassElement::MethodDefinition(method) => method.computed,
                ClassElement::PropertyDefinition(property) => property.computed,
                ClassElement::AccessorProperty(property) => property.computed,
                _ => false,
            };
            let span = if computed { self.bracketed_name_span(key.span()) } else { key.span() };
            // `checkVariableLikeDeclaration` for a later declaration of a
            // property: its type must be the first one's (TS2717).
            if let ClassElement::PropertyDefinition(property) = element
                && !computed
                && let Some(annotation) = &property.type_annotation
            {
                let annotation_span = annotation.type_annotation.span();
                let written = self
                    .source_text
                    .get(annotation_span.start as usize..annotation_span.end as usize)
                    .unwrap_or_default()
                    .to_string();
                let normalized: String = written.chars().filter(|ch| !ch.is_whitespace()).collect();
                match property_annotations.get(&(name.clone(), is_static)) {
                    Some(first) if first.chars().filter(|ch| !ch.is_whitespace()).collect::<String>() != normalized => {
                        let payload = format!("{name}\0{first}\0{written}");
                        self.push(Kind::Ts(2717), span, Some(&payload));
                    }
                    Some(_) => {}
                    None => {
                        property_annotations.insert((name.clone(), is_static), written);
                    }
                }
            }
            add_group_member(&mut groups, &mut group_slots, name, is_static, (span, member));
        }

        for group in &groups {
            self.report_member_group(group);
        }
        if !ambient {
            self.check_method_overloads(class);
            self.check_properties_used_before_assigned(class);
        }
        self.check_accessor_pairs(class);
        let accessors: Vec<AccessorRecord> = class
            .body
            .body
            .iter()
            .filter_map(|element| match element {
                ClassElement::MethodDefinition(method)
                    if matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set) =>
                {
                    let is_getter = method.kind == MethodDefinitionKind::Get;
                    let (key, display, span) = self.accessor_name(&method.key, method.computed)?;
                    Some(AccessorRecord {
                        key,
                        display,
                        is_static: method.r#static,
                        is_getter,
                        annotated: if is_getter {
                            method.value.return_type.is_some()
                                || super::jsdoc::return_type_at(method.value.span.start).is_some()
                        } else {
                            setter_parameter_annotated(&method.value.params)
                                || method.value.params.items.first().is_some_and(|parameter| {
                                    super::jsdoc::parameter_at(parameter.span.start)
                                        .is_some_and(|jsdoc| jsdoc.ty.is_some())
                                })
                        },
                        has_body: method.value.body.is_some(),
                        private: method.accessibility == Some(oxc_ast::ast::TSAccessibility::Private)
                            || matches!(method.key, PropertyKey::PrivateIdentifier(_)),
                        span,
                    })
                }
                _ => None,
            })
            .collect();
        self.check_untyped_setters(&accessors, ambient);

        // A property with no annotation and no initializer has an implicit
        // `any` type *unless* the constructor assigns it: tsc infers the
        // declaration's type from that assignment.
        let (assigned_instance, assigned_static) = assigned_property_names(class);
        for element in &class.body.body {
            // An auto-accessor is declared like a property and judged like one.
            let (key, is_typed, is_static, accessibility) = match element {
                ClassElement::PropertyDefinition(property) => (
                    &property.key,
                    property.type_annotation.is_some()
                        || property.value.is_some()
                        || property.computed
                        || super::jsdoc::declared_type_at(property.span.start).is_some(),
                    property.r#static,
                    property.accessibility,
                ),
                ClassElement::AccessorProperty(property) => (
                    &property.key,
                    property.type_annotation.is_some() || property.value.is_some() || property.computed,
                    property.r#static,
                    property.accessibility,
                ),
                _ => continue,
            };
            if is_typed {
                continue;
            }
            // `declarationBelongsToPrivateAmbientMember`: an ambient class's
            // private member reports no implicit `any`.
            if ambient
                && (accessibility == Some(oxc_ast::ast::TSAccessibility::Private)
                    || matches!(key, PropertyKey::PrivateIdentifier(_)))
            {
                continue;
            }
            let Some(name) = property_key_name(key) else {
                continue;
            };
            let assigned = if is_static { &assigned_static } else { &assigned_instance };
            if assigned.contains(&name) {
                continue;
            }
            // The binder merges a static `prototype` into the class's built-in
            // `prototype` symbol, whose type is the instance type.
            if is_static && name == "prototype" {
                continue;
            }
            self.push(Kind::ImplicitAnyMember, key.span(), Some(&name));
        }

        if constructors.iter().filter(|(_, has_body, _)| *has_body).count() > 1 {
            for (span, _, _) in &constructors {
                self.push(Kind::MultipleConstructorImplementations, *span, None);
            }
        }
        // `checkFlagAgreementBetweenOverloads` for the constructor symbol,
        // measured against the implementation (the first declaration when
        // there is none).
        if constructor_accessibility.iter().any(|(_, has_body, _)| !has_body) {
            let canonical = constructor_accessibility
                .iter()
                .find(|(_, has_body, _)| *has_body)
                .unwrap_or(&constructor_accessibility[0])
                .2;
            for (span, _, accessibility) in &constructor_accessibility {
                if *accessibility != canonical {
                    self.push(Kind::Ts(2385), *span, None);
                }
            }
        }
        // tsc exempts an `abstract` last overload from needing an implementation.
        if !ambient
            && !constructors.is_empty()
            && !constructors.iter().any(|(_, has_body, _)| *has_body)
            && let Some((span, _, false)) = constructors.last() {
                self.push(Kind::ConstructorImplementationMissing, *span, None);
            }

        // A class extending `null` has no base constructor to call (TS17005
        // when it does), so tsc does not require the call.
        let extends_null = matches!(
            class.super_class.as_ref().map(Expression::without_parentheses),
            Some(Expression::NullLiteral(_))
        );
        if !ambient && class.super_class.is_some() && !extends_null {
            for element in &class.body.body {
                let ClassElement::MethodDefinition(method) = element else {
                    continue;
                };
                if method.kind != MethodDefinitionKind::Constructor {
                    continue;
                }
                let Some(body) = method.value.body.as_ref() else {
                    continue;
                };
                if !body_calls_super(body) {
                    self.push(Kind::MissingSuperCall, method.key.span(), None);
                } else {
                    self.report_this_before_super(&method.value.params, body);
                    self.check_super_call_placement(class, method.key.span(), &method.value.params, body);
                }
            }
        }
    }

    /// One name's worth of class or interface members. A group of method
    /// signatures is an overload set, so it reports only about implementations;
    /// as soon as a property is in the group nothing there is an overload of
    /// anything and every member is a duplicate.
    fn report_member_group(&mut self, group: &MemberGroup) {
        let has_property = group
            .members
            .iter()
            .any(|(_, member)| matches!(member, MemberKind::Property));
        let has_accessor = group
            .members
            .iter()
            .any(|(_, member)| matches!(member, MemberKind::Accessor));
        let has_method = group
            .members
            .iter()
            .any(|(_, member)| matches!(member, MemberKind::Method { .. } | MemberKind::AbstractMethod));

        // Properties and accessors merge into one symbol, so tsc's checker
        // finds these (`checkObjectTypeForDuplicateDeclarations`): a second
        // property, or a property with an accessor, is a duplicate at every
        // member of the name; a get/set pair is not. A method among them is
        // the binder's conflict instead.
        if !has_method {
            let mut first_kind: Option<MemberKind> = None;
            for (_, member) in &group.members {
                match first_kind {
                    None => first_kind = Some(*member),
                    Some(MemberKind::Accessor) if *member == MemberKind::Accessor => {}
                    Some(_) => {
                        let name = group.name.clone();
                        for (span, _) in &group.members {
                            self.push(Kind::DuplicateMember, *span, Some(&name));
                        }
                        return;
                    }
                }
            }
            return;
        }

        if group.members.len() > 1 && has_property && !has_accessor {
            let name = group.name.clone();
            for (span, _) in &group.members {
                self.push(Kind::DuplicateMember, *span, Some(&name));
            }
            return;
        }
        if has_property || has_accessor {
            return;
        }

        let implementations = group
            .members
            .iter()
            .filter(|(_, member)| matches!(member, MemberKind::Method { has_body: true }))
            .count();
        // `checkFunctionOrConstructorSymbol` reports at every declaration of
        // the symbol, overloads included.
        if implementations > 1 {
            for (span, _) in &group.members {
                self.push(Kind::DuplicateImplementation, *span, None);
            }
            return;
        }

    }

    /// An interface reports the duplicate half only: a bodyless method there is
    /// a signature, never a missing implementation.
    fn check_interface_members(&mut self, members: &[TSSignature<'_>]) {
        let accessors: Vec<AccessorRecord> = members
            .iter()
            .filter_map(|member| match member {
                TSSignature::TSMethodSignature(method) if method.kind != TSMethodSignatureKind::Method => {
                    let is_getter = method.kind == TSMethodSignatureKind::Get;
                    let (key, display, span) = self.accessor_name(&method.key, method.computed)?;
                    Some(AccessorRecord {
                        key,
                        display,
                        is_static: false,
                        is_getter,
                        annotated: if is_getter {
                            method.return_type.is_some()
                        } else {
                            setter_parameter_annotated(&method.params)
                        },
                        has_body: false,
                        private: false,
                        span,
                    })
                }
                _ => None,
            })
            .collect();
        self.check_untyped_setters(&accessors, false);
        let mut groups: Vec<MemberGroup> = Vec::new();
        let mut group_slots: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for member in members {
            let (key, kind) = match member {
                TSSignature::TSPropertySignature(property) => (&property.key, MemberKind::Property),
                TSSignature::TSMethodSignature(method) => match method.kind {
                    TSMethodSignatureKind::Method => {
                        (&method.key, MemberKind::Method { has_body: false })
                    }
                    _ => (&method.key, MemberKind::Accessor),
                },
                _ => continue,
            };
            let Some(name) = property_key_name(key) else {
                continue;
            };
            match group_slots.get(&name) {
                Some(&slot) => groups[slot].members.push((key.span(), kind)),
                None => {
                    group_slots.insert(name.clone(), groups.len());
                    groups.push(MemberGroup {
                        name,
                        members: vec![(key.span(), kind)],
                    });
                }
            }
        }

        for group in groups {
            let has_property = group
                .members
                .iter()
                .any(|(_, member)| matches!(member, MemberKind::Property));
            let has_accessor = group
                .members
                .iter()
                .any(|(_, member)| matches!(member, MemberKind::Accessor));
            if group.members.len() > 1 && has_property && !has_accessor {
                for (span, _) in &group.members {
                    self.push(Kind::DuplicateMember, *span, Some(&group.name));
                }
            }
        }
    }

    /// The parameter-list grammar, checked wherever a list appears (a
    /// function, a method, a constructor, an arrow, a signature): a required
    /// parameter after an optional one, a parameter written both `?` and with
    /// a default, and a parameter written after the rest.
    fn check_parameter_list(&mut self, parameters: &FormalParameters<'_>) {
        let mut seen_optional = false;
        let mut reported_required = false;
        // An optional `@param` gives a JavaScript parameter its `?`
        // (`makeQuestionIfOptional`), which `checkGrammarParameterList` then
        // reads like a written one.
        let jsdoc_question = |parameter_start: u32, annotated: bool| {
            (!annotated)
                .then(|| super::jsdoc::parameter_at(parameter_start))
                .flatten()
                .and_then(|jsdoc| jsdoc.question)
        };

        for parameter in &parameters.items {
            if parameter.optional && parameter.initializer.is_some() {
                self.push(Kind::OptionalParameterWithInitializer, parameter.span, None);
            }
            let optional = parameter.optional
                || jsdoc_question(parameter.span.start, parameter.type_annotation.is_some()).is_some();
            if optional {
                seen_optional = true;
            } else if seen_optional && parameter.initializer.is_none() {
                // A *defaulted* parameter does not make the next one illegal —
                // only a `?` one does.
                self.push(Kind::RequiredParameterAfterOptional, parameter.span, None);
                reported_required = true;
            }
        }
        if !reported_required
            && let Some(rest) = &parameters.rest
            && rest.type_annotation.is_none()
            && let Some(jsdoc) = super::jsdoc::parameter_at(rest.span.start)
            && let Some(question) = jsdoc.question
        {
            self.diagnostics.push(ParsedGrammarDiagnostic {
                kind: Kind::Ts(1047),
                span: question,
                name: None,
            });
            // `{...T=}` types the rest parameter `T[] | undefined`, which
            // `checkParameter` finds no array under strictNullChecks.
            if let Some((crate::ParsedType::Union(members), _)) = &jsdoc.ty
                && matches!(members.as_slice(), [crate::ParsedType::Array(_), crate::ParsedType::Undefined])
            {
                self.push(Kind::TsUnderStrictNullChecks(2370), rest.span, None);
            }
        }
    }

    /// tsc's `checkAmbientInitializer`: only a `const` (or `readonly` property)
    /// without a type annotation may be initialized in an ambient context, and
    /// only with a constant (TS1254); any other initializer is TS1039.
    fn check_ambient_initializer(&mut self, initializer: &Expression<'_>, const_or_readonly: bool, has_type: bool) {
        if !const_or_readonly || has_type {
            self.push(Kind::AmbientInitializer, initializer.span(), None);
        } else if !is_ambient_constant_initializer(initializer) {
            self.push(Kind::AmbientConstInitializer, initializer.span(), None);
        }
    }

    /// A signature has no body to run a default in.
    fn check_signature_parameters(&mut self, parameters: &FormalParameters<'_>) {
        for parameter in &parameters.items {
            self.check_signature_pattern_initializers(&parameter.pattern);
            if parameter.initializer.is_some() {
                self.push(
                    Kind::ParameterInitializerOutsideImplementation,
                    parameter.span,
                    None,
                );
            }
        }
    }

    /// The elements of a parameter's pattern are checked like the parameter
    /// (`checkVariableLikeDeclaration`), so an initializer there is TS2371 too,
    /// at the element's name — except on a renamed element (`{ key: local }`),
    /// which that check leaves at once.
    fn check_signature_pattern_initializers(&mut self, pattern: &oxc_ast::ast::BindingPattern<'_>) {
        let elements: Vec<(&oxc_ast::ast::BindingPattern<'_>, bool)> = match pattern {
            oxc_ast::ast::BindingPattern::ObjectPattern(object) => {
                object.properties.iter().map(|property| (&property.value, !property.shorthand)).collect()
            }
            oxc_ast::ast::BindingPattern::ArrayPattern(array) => {
                array.elements.iter().flatten().map(|element| (element, false)).collect()
            }
            _ => return,
        };
        for (element, renamed) in elements {
            match element {
                oxc_ast::ast::BindingPattern::AssignmentPattern(assignment) => {
                    if renamed && matches!(assignment.left, oxc_ast::ast::BindingPattern::BindingIdentifier(_)) {
                        continue;
                    }
                    self.check_signature_pattern_initializers(&assignment.left);
                    self.push(Kind::ParameterInitializerOutsideImplementation, assignment.left.span(), None);
                }
                oxc_ast::ast::BindingPattern::BindingIdentifier(_) => {}
                nested => self.check_signature_pattern_initializers(nested),
            }
        }
    }

    fn check_variable_initializers(&mut self, declaration: &VariableDeclaration<'_>) {
        if declaration.declare || self.is_ambient() {
            // An ambient declaration declares a value rather than producing
            // one, so its missing initializer is not the `const` grammar error.
            let const_like = matches!(
                declaration.kind,
                VariableDeclarationKind::Const
                    | VariableDeclarationKind::Using
                    | VariableDeclarationKind::AwaitUsing
            );
            for declarator in &declaration.declarations {
                if let Some(initializer) = declarator.init.as_ref() {
                    self.check_ambient_initializer(
                        initializer,
                        const_like,
                        declarator.type_annotation.is_some(),
                    );
                } else if declarator.type_annotation.is_none()
                    && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &declarator.id
                {
                    // `widenTypeForVariableLikeDeclaration`: nothing types an
                    // ambient variable written with neither, so it is an
                    // implicit `any`, reported under `noImplicitAny`.
                    let payload = format!("{}\0any", identifier.name.as_str());
                    self.push(Kind::Ts(7005), identifier.span, Some(&payload));
                }
            }
            return;
        }

        // tsc's `checkGrammarVariableDeclaration`: a `using` pattern is TS1492
        // before anything else; a pattern without an initializer is TS1182
        // whatever its keyword, ahead of the `const`/`using` rule (TS1155).
        let using_keyword = using_declaration_keyword(declaration.kind);
        for declarator in &declaration.declarations {
            let is_pattern = !matches!(declarator.id, oxc_ast::ast::BindingPattern::BindingIdentifier(_));
            if is_pattern && let Some(keyword) = using_keyword {
                self.push(Kind::Ts(1492), declarator.id.span(), Some(keyword));
                continue;
            }
            if declarator.init.is_some() {
                continue;
            }
            if is_pattern {
                self.push(Kind::Ts(1182), declarator.id.span(), None);
                // Nothing types the pattern, so each element without a default
                // is an implicit `any` (`getTypeForBindingElement`).
                if declarator.type_annotation.is_none() {
                    let mut leaves = Vec::new();
                    undefaulted_binding_leaves(&declarator.id, &mut leaves);
                    for identifier in leaves {
                        let payload = format!("{}\0any", identifier.name.as_str());
                        self.push(Kind::Ts(7031), identifier.span, Some(&payload));
                    }
                }
            } else if declaration.kind == VariableDeclarationKind::Const {
                self.push(Kind::ConstNotInitialized, declarator.id.span(), None);
                // Nothing types it either (`widenTypeForVariableLikeDeclaration`).
                if declarator.type_annotation.is_none()
                    && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &declarator.id
                {
                    let payload = format!("{}\0any", identifier.name.as_str());
                    self.push(Kind::Ts(7005), identifier.span, Some(&payload));
                }
            } else if let Some(keyword) = using_keyword {
                self.push(Kind::Ts(1155), declarator.id.span(), Some(keyword));
            }
        }
    }

    /// tsc's `checkGrammarVariableDeclaration` on a `for...of` declaration,
    /// which the loop initializes: only the `using` pattern rule applies.
    fn check_using_loop_bindings(&mut self, left: &oxc_ast::ast::ForStatementLeft<'_>) {
        let oxc_ast::ast::ForStatementLeft::VariableDeclaration(declaration) = left else {
            return;
        };
        let Some(keyword) = using_declaration_keyword(declaration.kind) else {
            return;
        };
        for declarator in &declaration.declarations {
            if !matches!(declarator.id, oxc_ast::ast::BindingPattern::BindingIdentifier(_)) {
                self.push(Kind::Ts(1492), declarator.id.span(), Some(keyword));
            }
        }
    }

    /// tsc reports every property whose name was already written, so
    /// `{ a: 1, a: 2, a: 3 }` reports twice. A duplicate that involves a method
    /// or an accessor is a *different* diagnostic there (TS2300 for methods, a
    /// legal get/set pair for accessors), so only plain properties count.
    fn check_duplicate_properties(&mut self, object: &ObjectExpression<'_>) {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        // Methods report in first-written order, so the spans live in a `Vec`
        // and the map only finds a name's slot.
        let mut methods: Vec<(String, Vec<Span>)> = Vec::new();
        let mut method_slots: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut accessors: std::collections::HashSet<String> = std::collections::HashSet::new();
        // The accessor kinds seen per name (1 = get, 2 = set): a second
        // accessor of a kind already seen is TS1118, after which tsc's
        // `checkGrammarObjectLiteralExpression` stops looking at the literal.
        let mut accessor_kinds: std::collections::HashMap<String, u8> =
            std::collections::HashMap::new();

        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if property.kind != PropertyKind::Init {
                if let Some(name) = property_key_name(&property.key) {
                    let kind = if property.kind == PropertyKind::Get { 1 } else { 2 };
                    match accessor_kinds.get_mut(&name) {
                        Some(seen_kinds) if *seen_kinds == 3 || *seen_kinds == kind => {
                            self.push(Kind::Ts(1118), property.key.span(), None);
                            self.report_duplicate_accessors(object, &name);
                            return;
                        }
                        Some(seen_kinds) => *seen_kinds |= kind,
                        None => {
                            accessor_kinds.insert(name.clone(), kind);
                        }
                    }
                    if seen.contains(&name) || method_slots.contains_key(&name) {
                        self.push(
                            Kind::ObjectLiteralPropertyAndAccessor,
                            property.key.span(),
                            None,
                        );
                    }
                    accessors.insert(name);
                }
                continue;
            }
            let Some(name) = property_key_name(&property.key) else {
                continue;
            };
            if name == "__proto__" {
                continue;
            }
            // A name written as both an accessor and something else is its own
            // diagnostic, reported on whichever came later.
            if accessors.contains(&name) {
                self.push(
                    Kind::ObjectLiteralPropertyAndAccessor,
                    property.key.span(),
                    None,
                );
            }
            if property.method {
                match method_slots.get(&name) {
                    Some(&slot) => methods[slot].1.push(property.span),
                    None => {
                        method_slots.insert(name.clone(), methods.len());
                        methods.push((name, vec![property.span]));
                    }
                }
                continue;
            }
            if seen.contains(&name) {
                self.push(Kind::DuplicateObjectLiteralProperty, property.span, None);
            } else {
                seen.insert(name);
            }
        }

        // A repeated *method* is a duplicate identifier to tsc, not a repeated
        // property, and it names both of them. A name written once as a method
        // and once as a property is a third diagnostic pair (TS1119 with
        // TS2300) that is deliberately left alone.
        for (name, spans) in methods {
            if spans.len() < 2 || seen.contains(&name) {
                continue;
            }
            for span in spans {
                self.push(Kind::DuplicateMember, span, Some(&name));
            }
        }
    }

    /// The binder's duplicate-symbol error for two accessors of one kind in an
    /// object literal: TS2300 at every accessor of that name and kind.
    fn report_duplicate_accessors(&mut self, object: &ObjectExpression<'_>, name: &str) {
        for kind in [PropertyKind::Get, PropertyKind::Set] {
            let spans: Vec<Span> = object
                .properties
                .iter()
                .filter_map(|property| match property {
                    ObjectPropertyKind::ObjectProperty(property)
                        if property.kind == kind
                            && property_key_name(&property.key).as_deref() == Some(name) =>
                    {
                        Some(property.key.span())
                    }
                    _ => None,
                })
                .collect();
            if spans.len() > 1 {
                for span in spans {
                    self.push(Kind::DuplicateMember, span, Some(name));
                }
            }
        }
    }

    /// A type-level signature has no body to infer a parameter from, so an
    /// unannotated one is `any` unless an (erroneous) initializer types it.
    /// `may_name_type` is tsc's `reportImplicitAny` test for a call signature,
    /// method signature or function type, whose bare parameter name may be a
    /// forgotten type (`(string) => void`); the checker resolves the name.
    fn check_implicit_any_signature_parameters(
        &mut self,
        parameters: &FormalParameters<'_>,
        may_name_type: bool,
    ) {
        for (index, parameter) in parameters.items.iter().enumerate() {
            if parameter.type_annotation.is_some() || parameter.initializer.is_some() {
                continue;
            }
            let binding = match &parameter.pattern {
                oxc_ast::ast::BindingPattern::BindingIdentifier(binding) => binding,
                pattern => {
                    self.report_implicit_any_binding_elements(pattern);
                    continue;
                }
            };
            if may_name_type {
                self.push(
                    Kind::NamedSignatureParameterWithoutType,
                    binding.span,
                    Some(&format!("{}\0arg{index}\0", binding.name)),
                );
            } else {
                self.push(
                    Kind::ImplicitAnySignatureParameter,
                    binding.span,
                    Some(binding.name.as_str()),
                );
            }
        }
        if may_name_type
            && let Some(rest) = parameters.rest.as_deref()
            && rest.type_annotation.is_none()
            && let oxc_ast::ast::BindingPattern::BindingIdentifier(binding) = &rest.rest.argument
        {
            self.push(
                Kind::NamedSignatureParameterWithoutType,
                rest.span,
                Some(&format!("{}\0arg{}\0[]", binding.name, parameters.items.len())),
            );
        }
    }

    /// tsc's `getTypeFromBindingPattern` with errors reported: an element
    /// with neither an initializer nor a pattern of its own is an implicit
    /// `any` (TS7031, which the checker keeps under `noImplicitAny`). An
    /// object rest, a computed name that is no literal, and an array pattern
    /// with nothing but a rest imply nothing to report.
    fn report_implicit_any_binding_elements(&mut self, pattern: &oxc_ast::ast::BindingPattern<'_>) {
        match pattern {
            oxc_ast::ast::BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    // A renamed element is left to the unused-renaming check.
                    let renamed = !property.shorthand
                        && matches!(property.value, oxc_ast::ast::BindingPattern::BindingIdentifier(_));
                    if renamed
                        || property.computed
                            && !matches!(property.key, PropertyKey::StringLiteral(_) | PropertyKey::NumericLiteral(_))
                    {
                        continue;
                    }
                    self.report_implicit_any_binding_element(&property.value);
                }
            }
            oxc_ast::ast::BindingPattern::ArrayPattern(array) => {
                if array.elements.is_empty() {
                    return;
                }
                for element in array.elements.iter().flatten() {
                    self.report_implicit_any_binding_element(element);
                }
                if let Some(rest) = &array.rest {
                    self.report_implicit_any_binding_element(&rest.argument);
                }
            }
            oxc_ast::ast::BindingPattern::BindingIdentifier(_) | oxc_ast::ast::BindingPattern::AssignmentPattern(_) => {}
        }
    }

    fn report_implicit_any_binding_element(&mut self, element: &oxc_ast::ast::BindingPattern<'_>) {
        match element {
            oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) => {
                self.push(Kind::Ts(7031), identifier.span, Some(&format!("{}\0any", identifier.name)));
            }
            oxc_ast::ast::BindingPattern::AssignmentPattern(_) => {}
            nested => self.report_implicit_any_binding_elements(nested),
        }
    }

    /// tsc's `reportErrorsFromWidening` for the elements of a destructuring
    /// pattern read off a literal: without `strictNullChecks` an element
    /// initialized by `null` or `undefined` widens to an implicit `any`
    /// (TS7031).
    fn report_widening_binding_elements(&mut self, pattern: &oxc_ast::ast::BindingPattern<'_>, init: &Expression<'_>) {
        use oxc_ast::ast::{ArrayExpressionElement, BindingPattern};
        let report = |this: &mut Self, element: &BindingPattern<'_>, value: &Expression<'_>| match element {
            BindingPattern::BindingIdentifier(identifier) if widens(value) => this.push(
                Kind::TsWithoutStrictNullChecks(7031),
                identifier.span,
                Some(&format!("{}\0any", identifier.name)),
            ),
            BindingPattern::ArrayPattern(_) | BindingPattern::ObjectPattern(_) => {
                this.report_widening_binding_elements(element, value);
            }
            _ => {}
        };
        match (pattern, init.without_parentheses()) {
            (BindingPattern::ArrayPattern(array), Expression::ArrayExpression(literal)) => {
                for (element, value) in array.elements.iter().zip(&literal.elements) {
                    let (Some(element), Some(value)) = (element, value.as_expression()) else {
                        if matches!(value, ArrayExpressionElement::SpreadElement(_)) {
                            return;
                        }
                        continue;
                    };
                    report(self, element, value);
                }
            }
            (BindingPattern::ObjectPattern(object), Expression::ObjectExpression(literal)) => {
                if literal.properties.iter().any(|property| !matches!(property, ObjectPropertyKind::ObjectProperty(_))) {
                    return;
                }
                for property in &object.properties {
                    if property.computed {
                        continue;
                    }
                    let Some(name) = property_key_name(&property.key) else {
                        continue;
                    };
                    let value = literal.properties.iter().rev().find_map(|candidate| match candidate {
                        ObjectPropertyKind::ObjectProperty(candidate)
                            if !candidate.computed
                                && candidate.kind == PropertyKind::Init
                                && property_key_name(&candidate.key).as_deref() == Some(name.as_str()) =>
                        {
                            Some(&candidate.value)
                        }
                        _ => None,
                    });
                    if let Some(value) = value {
                        report(self, &property.value, value);
                    }
                }
            }
            _ => {}
        }
    }

    /// tsc's `reportWideningErrorsInType` for a variable's literal
    /// initializer, which only exists without `strictNullChecks`: a property
    /// that widens is TS7018, and an array of nothing but `null` and
    /// `undefined` leaves the variable an implicit `any[]` (TS7005).
    fn report_widening_initializer(&mut self, identifier: &oxc_ast::ast::BindingIdentifier<'_>, init: &Expression<'_>) {
        match init.without_parentheses() {
            Expression::ObjectExpression(object) => {
                self.report_widening_properties(object);
            }
            Expression::ArrayExpression(array) if array_widens(array) => self.push(
                Kind::TsWithoutStrictNullChecks(7005),
                identifier.span,
                Some(&format!("{}\0any[]", identifier.name)),
            ),
            // `var b = a = [null]`: the assignment has its right side's type.
            Expression::AssignmentExpression(assignment)
                if assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign =>
            {
                self.report_widening_initializer(identifier, &assignment.right);
            }
            _ => {}
        }
    }

    /// Whether any property of the literal widened, reporting each.
    fn report_widening_properties(&mut self, object: &ObjectExpression<'_>) -> bool {
        let mut properties = Vec::new();
        let reported = widening_properties(object, &mut properties);
        self.push_widening_properties(properties);
        reported
    }

    fn push_widening_properties(&mut self, properties: Vec<(Span, String, &'static str)>) {
        for (span, name, widened) in properties {
            self.push(Kind::TsWithoutStrictNullChecks(7018), span, Some(&format!("{name}\0{widened}")));
        }
    }

    /// tsc's `reportErrorsFromWidening` for the return type a function no
    /// contextual signature types infers, which only exists without
    /// `strictNullChecks`: TS7010 names the function, TS7011 stands for one
    /// with no name, reported at `unnamed_span`.
    fn report_widening_return(
        &mut self,
        body: Option<&FunctionBody<'_>>,
        expression_body: bool,
        function_start: u32,
        name: Option<(Span, &str)>,
        unnamed_span: Span,
    ) {
        if super::jsdoc::return_type_at(function_start).is_some()
            || super::jsdoc::full_signature_at(function_start).is_some()
            || super::jsdoc::has_opaque_full_signature(function_start)
        {
            return;
        }
        let Some(widening) = body.and_then(|body| return_widening(body, expression_body)) else {
            return;
        };
        match (widening, name) {
            (ReturnWidening::Properties(properties), _) => self.push_widening_properties(properties),
            (ReturnWidening::Implicit(widened), Some((span, name))) => {
                self.push(Kind::TsWithoutStrictNullChecks(7010), span, Some(&format!("{name}\0{widened}")));
            }
            (ReturnWidening::Implicit(widened), None) => {
                self.push(Kind::TsWithoutStrictNullChecks(7011), unnamed_span, Some(widened));
            }
        }
    }

    /// A function written where nothing gives it a contextual signature — an
    /// unannotated variable's or class property's initializer, or a member of
    /// an object literal written there. An unnamed function expression is
    /// reported at the name it is assigned to, when it has one.
    fn report_widening_function_value(&mut self, value: &Expression<'_>, assigned_name: Option<Span>) {
        match value.without_parentheses() {
            Expression::ArrowFunctionExpression(arrow) if arrow.return_type.is_none() => {
                self.report_widening_return(Some(&arrow.body), arrow.expression, arrow.span.start, None, arrow.span);
            }
            Expression::FunctionExpression(function) if function.return_type.is_none() => {
                let name = function.id.as_ref().map(|id| (id.span, id.name.as_str()));
                self.report_widening_return(
                    function.body.as_deref(),
                    false,
                    function.span.start,
                    name,
                    assigned_name.unwrap_or(function.span),
                );
            }
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        continue;
                    };
                    match &property.value {
                        Expression::FunctionExpression(function)
                            if property.method || property.kind == PropertyKind::Get =>
                        {
                            if function.return_type.is_none()
                                && let Some((span, name)) =
                                    self.implicit_any_member_name(&property.key, property.computed)
                            {
                                self.report_widening_return(
                                    function.body.as_deref(),
                                    false,
                                    function.span.start,
                                    Some((span, &name)),
                                    span,
                                );
                            }
                        }
                        value if property.kind == PropertyKind::Init => {
                            self.report_widening_function_value(value, Some(property.key.span()));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// A class property's initializer, which types the member when nothing is
    /// written: a widening value is TS7008 on the member.
    fn report_widening_member(&mut self, property: &oxc_ast::ast::PropertyDefinition<'_>, value: &Expression<'_>) {
        let widened = match value.without_parentheses() {
            value if widens(value) => Some("any"),
            Expression::ArrayExpression(array) if array.elements.is_empty() || array_widens(array) => Some("any[]"),
            Expression::ObjectExpression(object) => {
                self.report_widening_properties(object);
                self.report_widening_function_value(value, None);
                None
            }
            value => {
                self.report_widening_function_value(value, None);
                None
            }
        };
        if let Some(widened) = widened
            && let Some((span, name)) = self.implicit_any_member_name(&property.key, property.computed)
        {
            self.push(Kind::TsWithoutStrictNullChecks(7008), span, Some(&format!("{name}\0{widened}")));
        }
    }

    /// The name an implicit-`any` report gives a member, and where: a computed
    /// one is its bracketed source text (`declarationNameToString`).
    fn implicit_any_member_name(&self, key: &PropertyKey<'_>, computed: bool) -> Option<(Span, String)> {
        if !computed {
            return property_key_name(key).map(|name| (key.span(), name));
        }
        let span = key.span();
        let open = self.source_text[..span.start as usize].rfind('[')?;
        let close = span.end as usize + self.source_text[span.end as usize..].find(']')?;
        let name = self.source_text.get(open..=close)?.to_string();
        Some((Span::new(open as u32, close as u32 + 1), name))
    }

    /// A signature with neither a body nor a written return type has an
    /// implicit `any` return — the overload signatures of a function that *does*
    /// have an implementation included, which is why this does not look at the
    /// group the way the missing-implementation check does.
    fn check_implicit_any_return(&mut self, function: &Function<'_>) {
        if function.body.is_some() || function.return_type.is_some() {
            return;
        }
        let Some(id) = function.id.as_ref() else {
            return;
        };
        self.push(Kind::ImplicitAnyReturn, id.span, Some(id.name.as_str()));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SiblingKind {
    Function,
    Method,
    Other,
}

/// One entry of a container's declaration list, as the overload checks see
/// it; everything that is not a function or method is `Other`, kept so that
/// adjacency is measured against the real next sibling.
struct OverloadSibling {
    kind: SiblingKind,
    name: Option<String>,
    name_span: Span,
    is_static: bool,
    has_body: bool,
    ambient: bool,
    exported: bool,
    accessibility: Option<oxc_ast::ast::TSAccessibility>,
    is_abstract: bool,
    optional: bool,
}

impl OverloadSibling {
    fn other() -> Self {
        Self {
            kind: SiblingKind::Other,
            name: None,
            name_span: Span::default(),
            is_static: false,
            has_body: false,
            ambient: false,
            exported: false,
            accessibility: None,
            is_abstract: false,
            optional: false,
        }
    }
}

struct MemberGroup {
    name: String,
    members: Vec<(Span, MemberKind)>,
}

fn add_group_member(
    groups: &mut Vec<MemberGroup>,
    slots: &mut std::collections::HashMap<(String, bool), usize>,
    name: String,
    is_static: bool,
    member: (Span, MemberKind),
) {
    match slots.get(&(name.clone(), is_static)) {
        Some(&slot) => groups[slot].members.push(member),
        None => {
            slots.insert((name.clone(), is_static), groups.len());
            groups.push(MemberGroup {
                name,
                members: vec![member],
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MemberKind {
    Property,
    Accessor,
    AbstractMethod,
    Method { has_body: bool },
}

/// The names a class constructor assigns through `this.<name> = …`, which is
/// where tsc gets the type of a property declared without one.
/// The members tsc infers a declared type for from assignments: `this.x = …`
/// (or `this["x"]`, `this[0]`) in the constructor for an instance member, and
/// the same inside a `static {}` block for a static one. Returned as
/// `(instance, static)`.
fn assigned_property_names(class: &Class<'_>) -> (Vec<String>, Vec<String>) {
    struct ThisAssignmentCollector {
        names: Vec<String>,
    }

    impl<'a> Visit<'a> for ThisAssignmentCollector {
        fn visit_assignment_expression(
            &mut self,
            assignment: &oxc_ast::ast::AssignmentExpression<'a>,
        ) {
            match &assignment.left {
                AssignmentTarget::StaticMemberExpression(member)
                    if matches!(member.object, Expression::ThisExpression(_)) =>
                {
                    self.names.push(member.property.name.to_string());
                }
                AssignmentTarget::PrivateFieldExpression(member)
                    if matches!(member.object, Expression::ThisExpression(_)) =>
                {
                    self.names.push(format!("#{}", member.field.name));
                }
                AssignmentTarget::ComputedMemberExpression(member)
                    if matches!(member.object, Expression::ThisExpression(_)) =>
                {
                    match &member.expression {
                        Expression::StringLiteral(literal) => {
                            self.names.push(literal.value.to_string());
                        }
                        Expression::NumericLiteral(literal) => {
                            self.names.push(super::number_text::js_number_to_string(literal.value));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            oxc_ast_visit::walk::walk_assignment_expression(self, assignment);
        }
    }

    let mut instance = ThisAssignmentCollector { names: Vec::new() };
    let mut statics = ThisAssignmentCollector { names: Vec::new() };
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor =>
            {
                if let Some(body) = method.value.body.as_ref() {
                    instance.visit_function_body(body);
                }
            }
            ClassElement::StaticBlock(block) => {
                for statement in &block.body {
                    statics.visit_statement(statement);
                }
            }
            _ => {}
        }
    }
    (instance.names, statics.names)
}

/// Whether a function body has a `return` of its own, outside any nested
/// function.
fn body_has_return(body: &FunctionBody<'_>) -> bool {
    struct ReturnFinder {
        found: bool,
    }

    impl<'a> Visit<'a> for ReturnFinder {
        fn visit_return_statement(&mut self, _: &oxc_ast::ast::ReturnStatement<'a>) {
            self.found = true;
        }
        fn visit_function(&mut self, _: &Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
        fn visit_class(&mut self, _: &Class<'a>) {}
    }

    let mut finder = ReturnFinder { found: false };
    finder.visit_function_body(body);
    finder.found
}

/// Whether a constructor body calls `super(...)` anywhere inside it, including
/// inside a branch or an arrow function — the same places the call counts for
/// tsc.
fn body_calls_super(body: &FunctionBody<'_>) -> bool {
    struct SuperCallFinder {
        found: bool,
    }

    // tsc's `containsSuperCall` stops at a nested function, arrow included.
    impl<'a> Visit<'a> for SuperCallFinder {
        fn visit_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
            if matches!(call.callee, Expression::Super(_)) {
                self.found = true;
            }
            oxc_ast_visit::walk::walk_call_expression(self, call);
        }

        fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}

        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
    }

    let mut finder = SuperCallFinder { found: false };
    finder.visit_function_body(body);
    finder.found
}

/// tsc's `isSideEffectFree`: the expressions whose value can be dropped without
/// anything happening. A property access is deliberately *not* one of them — a
/// getter can do anything.
/// tsc's `getSyntacticTruthySemantics`: `Some(true)` for syntax that is always
/// truthy, `Some(false)` for syntax always falsy, `None` when it can be either.
fn syntactic_truthiness(expression: &Expression<'_>) -> Option<bool> {
    match expression.get_inner_expression() {
        // `while (0)` and `while (1)` are idioms, not mistakes. tsc compares the
        // literal's normalized text, so `0.0` and `0x1` are exempt too.
        Expression::NumericLiteral(literal) => {
            (literal.value != 0.0 && literal.value != 1.0).then_some(true)
        }
        Expression::ArrayExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::BigIntLiteral(_)
        | Expression::ClassExpression(_)
        | Expression::FunctionExpression(_)
        | Expression::JSXElement(_)
        | Expression::ObjectExpression(_)
        | Expression::RegExpLiteral(_) => Some(true),
        Expression::NullLiteral(_) => Some(false),
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::Void => Some(false),
        Expression::StringLiteral(literal) => Some(!literal.value.is_empty()),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => Some(
            template
                .quasis
                .first()
                .is_some_and(|quasi| !quasi.value.raw.is_empty()),
        ),
        Expression::ConditionalExpression(conditional) => {
            let when_true = syntactic_truthiness(&conditional.consequent)?;
            (syntactic_truthiness(&conditional.alternate)? == when_true).then_some(when_true)
        }
        Expression::Identifier(identifier) if identifier.name == "undefined" => Some(false),
        _ => None,
    }
}

/// tsc's `getSyntacticNullishnessSemantics`: `Some(true)` for syntax that is
/// always nullish, `Some(false)` for syntax never nullish, `None` when it can be
/// either. A nested `??` follows its right operand, as TypeScript 7.0 does.
fn syntactic_nullishness(expression: &Expression<'_>) -> Option<bool> {
    match expression.get_inner_expression() {
        Expression::AwaitExpression(_)
        | Expression::CallExpression(_)
        | Expression::ChainExpression(_)
        | Expression::ImportExpression(_)
        | Expression::TaggedTemplateExpression(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::StaticMemberExpression(_)
        | Expression::PrivateFieldExpression(_)
        | Expression::MetaProperty(_)
        | Expression::NewExpression(_)
        | Expression::YieldExpression(_)
        | Expression::ThisExpression(_)
        | Expression::Super(_) => None,
        Expression::LogicalExpression(logical) => match logical.operator {
            oxc_syntax::operator::LogicalOperator::Coalesce => syntactic_nullishness(&logical.right),
            _ => None,
        },
        Expression::AssignmentExpression(assignment) => match assignment.operator {
            oxc_syntax::operator::AssignmentOperator::Assign
            | oxc_syntax::operator::AssignmentOperator::LogicalNullish => {
                syntactic_nullishness(&assignment.right)
            }
            oxc_syntax::operator::AssignmentOperator::LogicalOr
            | oxc_syntax::operator::AssignmentOperator::LogicalAnd => None,
            _ => Some(false),
        },
        Expression::SequenceExpression(sequence) => {
            sequence.expressions.last().and_then(syntactic_nullishness)
        }
        Expression::ConditionalExpression(conditional) => {
            let when_true = syntactic_nullishness(&conditional.consequent)?;
            (syntactic_nullishness(&conditional.alternate)? == when_true).then_some(when_true)
        }
        Expression::NullLiteral(_) => Some(true),
        Expression::Identifier(identifier) => (identifier.name == "undefined").then_some(true),
        _ => Some(false),
    }
}

/// tsc's `isSideEffectFree`, which looks through parentheses only: an `as`
/// or `satisfies` operand counts as having effects.
fn is_side_effect_free(expression: &Expression<'_>) -> bool {
    match expression.without_parentheses() {
        Expression::Identifier(_)
        | Expression::StringLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::TemplateLiteral(_)
        | Expression::TaggedTemplateExpression(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::FunctionExpression(_)
        | Expression::ClassExpression(_)
        | Expression::ArrowFunctionExpression(_)
        | Expression::ArrayExpression(_)
        | Expression::ObjectExpression(_)
        | Expression::JSXElement(_) => true,
        Expression::ConditionalExpression(conditional) => {
            is_side_effect_free(&conditional.consequent)
                && is_side_effect_free(&conditional.alternate)
        }
        // tsc's binary expressions include the logical and comma operators.
        Expression::BinaryExpression(binary) => {
            is_side_effect_free(&binary.left) && is_side_effect_free(&binary.right)
        }
        Expression::LogicalExpression(logical) => {
            is_side_effect_free(&logical.left) && is_side_effect_free(&logical.right)
        }
        Expression::SequenceExpression(sequence) => sequence.expressions.iter().all(is_side_effect_free),
        Expression::UnaryExpression(unary) => matches!(
            unary.operator,
            UnaryOperator::LogicalNot
                | UnaryOperator::UnaryPlus
                | UnaryOperator::UnaryNegation
                | UnaryOperator::BitwiseNot
                | UnaryOperator::Typeof
        ),
        Expression::TSNonNullExpression(_) => true,
        _ => false,
    }
}

fn export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
    }
}

impl GrammarCollector {
    fn collect_unique_symbol_consts(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            let declaration = match statement {
                Statement::VariableDeclaration(declaration) => declaration,
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::VariableDeclaration(declaration)) => declaration,
                    _ => continue,
                },
                _ => continue,
            };
            if declaration.kind != VariableDeclarationKind::Const {
                continue;
            }
            for declarator in &declaration.declarations {
                let Some(id) = declarator.id.get_binding_identifier() else {
                    continue;
                };
                let written = declarator.type_annotation.as_ref().is_some_and(|annotation| {
                    matches!(
                        &annotation.type_annotation,
                        oxc_ast::ast::TSType::TSTypeOperatorType(operator)
                            if operator.operator == oxc_ast::ast::TSTypeOperatorOperator::Unique
                    )
                });
                let symbol_call = declarator.type_annotation.is_none()
                    && matches!(
                        &declarator.init,
                        Some(Expression::CallExpression(call))
                            if matches!(&call.callee, Expression::Identifier(callee) if callee.name == "Symbol")
                    );
                if written || symbol_call {
                    self.unique_symbol_consts.insert(id.name.to_string());
                }
            }
        }
    }
}

fn property_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::NumericLiteral(literal) => Some(super::number_text::js_number_to_string(literal.value)),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        // A well-known symbol is a unique, statically known name
        // (`[Symbol.iterator]`), late-bound like a literal one.
        PropertyKey::StaticMemberExpression(member)
            if matches!(&member.object, Expression::Identifier(object) if object.name == "Symbol") =>
        {
            Some(format!("[Symbol.{}]", member.property.name))
        }
        _ => None,
    }
}

impl<'a> Visit<'a> for GrammarCollector {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.source_text = program.source_text.to_string();
        self.external_module = super::grammar_context::is_external_module(program)
            || program.source_text.contains("import.meta");
        self.collect_top_level_enums(&program.body);
        self.collect_unique_symbol_consts(&program.body);
        self.check_member_kind_overrides(&program.body);
        self.check_circular_type_aliases(&program.body);
        self.check_top_level_enum_merges(&program.body);
        self.check_default_exports(&program.body);
        self.check_function_implementations(&program.body);
        oxc_ast_visit::walk::walk_program(self, program);
        let bound: std::collections::HashSet<&str> = self
            .binding_names
            .iter()
            .filter(|(_, span)| {
                !self
                    .type_declaration_name_spans
                    .contains(&(span.start, span.end))
            })
            .map(|(name, _)| name.as_str())
            .collect();
        let findings = std::mem::take(&mut self.diagnostics);
        self.diagnostics = findings
            .into_iter()
            .filter(|finding| {
                finding.kind != Kind::ComputedTypeMemberName
                    || finding
                        .name
                        .as_deref()
                        .and_then(|payload| payload.split('\0').next())
                        .is_some_and(|name| !bound.contains(name))
            })
            .collect();
    }

    fn visit_binding_identifier(&mut self, identifier: &oxc_ast::ast::BindingIdentifier<'a>) {
        self.binding_names.push((identifier.name.to_string(), identifier.span));
    }

    fn visit_ts_type_alias_declaration(
        &mut self,
        declaration: &oxc_ast::ast::TSTypeAliasDeclaration<'a>,
    ) {
        self.type_declaration_name_spans
            .insert((declaration.id.span.start, declaration.id.span.end));
        oxc_ast_visit::walk::walk_ts_type_alias_declaration(self, declaration);
    }

    /// tsc's `checkTupleType` for a lone rest element (`[...string]`), which
    /// the lowering reads as its operand and so never checks: an operand that
    /// is plainly not array-like is TS2574.
    fn visit_ts_tuple_type(&mut self, tuple: &oxc_ast::ast::TSTupleType<'a>) {
        fn plainly_not_array_like(ty: &oxc_ast::ast::TSType<'_>) -> bool {
            use oxc_ast::ast::TSType;
            match ty {
                TSType::TSStringKeyword(_)
                | TSType::TSNumberKeyword(_)
                | TSType::TSBooleanKeyword(_)
                | TSType::TSBigIntKeyword(_)
                | TSType::TSSymbolKeyword(_)
                | TSType::TSUnknownKeyword(_)
                | TSType::TSLiteralType(_) => true,
                TSType::TSParenthesizedType(parenthesized) => plainly_not_array_like(&parenthesized.type_annotation),
                // `T?` is `T | null` and `T!` is `T`.
                TSType::JSDocNullableType(nullable) => plainly_not_array_like(&nullable.type_annotation),
                TSType::JSDocNonNullableType(non_nullable) => plainly_not_array_like(&non_nullable.type_annotation),
                TSType::TSUnionType(union) => union.types.iter().all(plainly_not_array_like),
                _ => false,
            }
        }
        if let [oxc_ast::ast::TSTupleElement::TSRestType(rest)] = tuple.element_types.as_slice()
            && plainly_not_array_like(&rest.type_annotation)
        {
            self.push(Kind::Ts(2574), rest.span, None);
        }
        oxc_ast_visit::walk::walk_ts_tuple_type(self, tuple);
    }

    fn visit_block_statement(&mut self, block: &oxc_ast::ast::BlockStatement<'a>) {
        self.check_function_implementations(&block.body);
        self.check_nested_duplicate_implementations(&block.body);
        self.nested_scope_depth += 1;
        oxc_ast_visit::walk::walk_block_statement(self, block);
        self.nested_scope_depth -= 1;
    }


    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.check_function_implementations(&body.statements);
        self.check_nested_duplicate_implementations(&body.statements);
        self.check_body_class_forward_references(&body.statements);
        oxc_ast_visit::walk::walk_function_body(self, body);
    }

    // An enum member initializer may only name members declared before it: the
    // members are evaluated in order, so a forward reference reads a binding
    // that does not have its value yet (tsc's `checkEnumDeclaration`, TS2651).
    fn visit_ts_enum_declaration(&mut self, declaration: &oxc_ast::ast::TSEnumDeclaration<'a>) {
        self.check_enum_member_values(declaration);
        if !self
            .top_level_enums
            .contains(&(declaration.span.start, declaration.span.end))
        {
            self.check_duplicate_enum_members(std::slice::from_ref(&declaration));
        }
        let mut declared_so_far: Vec<&str> = Vec::new();
        let member_names: Vec<Option<String>> = declaration
            .body
            .members
            .iter()
            .map(|member| property_key_name_of_enum_member(&member.id))
            .collect();

        for (index, member) in declaration.body.members.iter().enumerate() {
            if let Some(initializer) = &member.initializer {
                let later: Vec<&str> = member_names
                    .iter()
                    .skip(index)
                    .filter_map(|name| name.as_deref())
                    .collect();
                report_enum_forward_references(self, initializer, &later);
            }
            if let Some(name) = member_names[index].as_deref() {
                declared_so_far.push(name);
            }
        }

        oxc_ast_visit::walk::walk_ts_enum_declaration(self, declaration);
    }

    // `delete x` on a direct binding reference is a strict-mode syntax error
    // (tsc's `checkStrictModeDeleteExpression`, binder.go:1409), and tsgo
    // checks every file as strict-mode code. `delete o.p`,
    // the legal form, is checked by the checker's own operand rules.
    fn visit_unary_expression(&mut self, unary: &oxc_ast::ast::UnaryExpression<'a>) {
        if unary.operator == UnaryOperator::LogicalNot {
            self.check_truthiness(&unary.argument);
        }
        if unary.operator == oxc_syntax::operator::UnaryOperator::Delete
            && let Expression::Identifier(identifier) = &unary.argument
        {
            self.push(Kind::DeleteOnIdentifierInStrictMode, identifier.span, None);
        }
        oxc_ast_visit::walk::walk_unary_expression(self, unary);
    }

    // `declare global { … }` is its own node, and everything inside it is
    // ambient: a bodyless declaration there is a declaration, not a missing
    // implementation.
    fn visit_ts_global_declaration(&mut self, declaration: &TSGlobalDeclaration<'a>) {
        self.nested_scope_depth += 1;
        self.ambient_depth += 1;
        oxc_ast_visit::walk::walk_ts_global_declaration(self, declaration);
        self.ambient_depth -= 1;
        self.nested_scope_depth -= 1;
    }

    fn visit_ts_module_declaration(&mut self, declaration: &TSModuleDeclaration<'a>) {
        let ambient = declaration.declare || self.is_ambient();
        self.nested_scope_depth += 1;
        self.this_container_depth += 1;
        if ambient {
            self.ambient_depth += 1;
        }
        if let Some(TSModuleDeclarationBody::TSModuleBlock(block)) = declaration.body.as_ref() {
            self.check_function_implementations(&block.body);
            // An ambient external module (`declare module "x"`) may re-export.
            if !matches!(declaration.id, oxc_ast::ast::TSModuleDeclarationName::StringLiteral(_)) {
                self.check_namespace_export_declarations(&block.body, ambient);
            }
        }
        oxc_ast_visit::walk::walk_ts_module_declaration(self, declaration);
        if ambient {
            self.ambient_depth -= 1;
        }
        self.this_container_depth -= 1;
        self.nested_scope_depth -= 1;
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.check_class_members(class);
        self.check_static_index_signature_type_parameters(class);
        if class.declare {
            self.declare_class_depth += 1;
        }
        oxc_ast_visit::walk::walk_class(self, class);
        if class.declare {
            self.declare_class_depth -= 1;
        }
    }

    fn visit_ts_interface_declaration(
        &mut self,
        declaration: &oxc_ast::ast::TSInterfaceDeclaration<'a>,
    ) {
        self.type_declaration_name_spans
            .insert((declaration.id.span.start, declaration.id.span.end));
        self.check_interface_members(&declaration.body.body);
        self.check_computed_type_member_names(&declaration.body.body, false);
        oxc_ast_visit::walk::walk_ts_interface_declaration(self, declaration);
    }

    fn visit_ts_type_literal(&mut self, literal: &oxc_ast::ast::TSTypeLiteral<'a>) {
        self.check_interface_members(&literal.members);
        self.check_computed_type_member_names(&literal.members, true);
        self.check_conflicting_type_literal_properties(&literal.members);
        oxc_ast_visit::walk::walk_ts_type_literal(self, literal);
    }

    fn visit_if_statement(&mut self, statement: &oxc_ast::ast::IfStatement<'a>) {
        self.check_truthiness(&statement.test);
        oxc_ast_visit::walk::walk_if_statement(self, statement);
    }

    fn visit_while_statement(&mut self, statement: &oxc_ast::ast::WhileStatement<'a>) {
        self.check_truthiness(&statement.test);
        oxc_ast_visit::walk::walk_while_statement(self, statement);
    }

    fn visit_do_while_statement(&mut self, statement: &oxc_ast::ast::DoWhileStatement<'a>) {
        self.check_truthiness(&statement.test);
        oxc_ast_visit::walk::walk_do_while_statement(self, statement);
    }

    fn visit_for_statement(&mut self, statement: &oxc_ast::ast::ForStatement<'a>) {
        if let Some(test) = &statement.test {
            self.check_truthiness(test);
        }
        oxc_ast_visit::walk::walk_for_statement(self, statement);
    }

    fn visit_conditional_expression(
        &mut self,
        conditional: &oxc_ast::ast::ConditionalExpression<'a>,
    ) {
        self.check_truthiness(&conditional.test);
        oxc_ast_visit::walk::walk_conditional_expression(self, conditional);
    }

    fn visit_logical_expression(&mut self, logical: &oxc_ast::ast::LogicalExpression<'a>) {
        if logical.operator == oxc_syntax::operator::LogicalOperator::Coalesce {
            let left = logical.left.get_inner_expression();
            match syntactic_nullishness(left) {
                Some(true) => self.push(Kind::AlwaysNullishCoalesceOperand, left.span(), None),
                Some(false) => self.push(Kind::NeverNullishCoalesceOperand, left.span(), None),
                None => {}
            }
        } else {
            self.check_truthiness(&logical.left);
        }
        oxc_ast_visit::walk::walk_logical_expression(self, logical);
    }

    fn visit_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        self.note_indirect_call(&call.callee);
        oxc_ast_visit::walk::walk_call_expression(self, call);
    }

    fn visit_tagged_template_expression(
        &mut self,
        expression: &oxc_ast::ast::TaggedTemplateExpression<'a>,
    ) {
        self.note_indirect_call(&expression.tag);
        oxc_ast_visit::walk::walk_tagged_template_expression(self, expression);
    }

    fn visit_sequence_expression(&mut self, sequence: &oxc_ast::ast::SequenceExpression<'a>) {
        // tsc parses `a, b, c` as `(a, b), c`: each comma's discarded left
        // operand is the whole sequence before it, side-effect free only when
        // every operand in it is, and reported from the first operand on.
        let indirect_call = self.indirect_call_sequences.contains(&sequence.span.start);
        let start = sequence.expressions.first().map_or(sequence.span.start, |first| first.span().start);
        let mut prefix_side_effect_free = true;
        for expression in sequence
            .expressions
            .iter()
            .take(sequence.expressions.len().saturating_sub(1))
        {
            prefix_side_effect_free &= is_side_effect_free(expression);
            if !prefix_side_effect_free {
                break;
            }
            if !indirect_call {
                self.push(Kind::UnusedCommaOperand, Span::new(start, expression.span().end), None);
            }
        }
        oxc_ast_visit::walk::walk_sequence_expression(self, sequence);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.check_implicit_any_return(function);
        if function.is_function_declaration()
            && function.return_type.is_none()
            && let Some(id) = &function.id
        {
            self.report_widening_return(
                function.body.as_deref(),
                false,
                function.span.start,
                Some((id.span, id.name.as_str())),
                id.span,
            );
        }
        if function.r#async && !function.generator {
            self.check_async_return_type(function.return_type.as_deref());
        }
        if function.body.is_none() {
            self.check_signature_parameters(&function.params);
        }
        self.function_async.push(function.r#async);
        self.await_context.push(function.r#async);
        oxc_ast_visit::walk::walk_function(self, function, flags);
        self.await_context.pop();
        self.function_async.pop();
    }

    fn visit_formal_parameter(&mut self, parameter: &oxc_ast::ast::FormalParameter<'a>) {
        self.parameter_decorator_await =
            self.await_context.len().checked_sub(2).map(|outer| self.await_context[outer]);
        oxc_ast_visit::walk::walk_formal_parameter(self, parameter);
    }

    fn visit_decorators(&mut self, decorators: &oxc_allocator::Vec<'a, oxc_ast::ast::Decorator<'a>>) {
        match self.parameter_decorator_await.take() {
            Some(outer) => {
                self.await_context.push(outer);
                oxc_ast_visit::walk::walk_decorators(self, decorators);
                self.await_context.pop();
            }
            None => oxc_ast_visit::walk::walk_decorators(self, decorators),
        }
    }

    fn visit_arrow_function_expression(&mut self, arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) {
        if arrow.r#async {
            self.check_async_return_type(arrow.return_type.as_deref());
            // A declaration's inferred return type is resolved only where it is
            // read; an arrow's, wherever the arrow is.
            if arrow.return_type.is_none() {
                self.push(Kind::AsyncInferredReturn, arrow.span, None);
            }
        }
        // `checkGrammarArrowFunction`: `<T>() => …` reads as a JSX tag in a
        // `.mts`/`.cts` file; the checker keeps it for those extensions.
        if let Some(type_parameters) = &arrow.type_parameters
            && let [parameter] = type_parameters.params.as_slice()
            && parameter.constraint.is_none()
            && !self.source_text[parameter.span.end as usize..type_parameters.span.end as usize].contains(',')
        {
            self.push(Kind::Ts(7060), parameter.span, None);
        }
        self.function_async.push(arrow.r#async);
        self.await_context.push(arrow.r#async);
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.await_context.pop();
        self.function_async.pop();
    }

    /// tsc's `checkGrammarAwaitOrAwaitUsing`: `await` inside a function that
    /// is not `async` — TS1308.
    fn visit_await_expression(&mut self, expression: &oxc_ast::ast::AwaitExpression<'a>) {
        let keyword = Span::new(expression.span.start, expression.span.start + 5);
        // Outside an await context tsc's parser takes `await` for an operator
        // only before a name, keyword or literal on its line
        // (`isAwaitExpression`), and for a name otherwise.
        let parsed_as_await = await_operand_on_same_line(&self.source_text, keyword.end);
        if self.await_context.last() == Some(&false) {
            if parsed_as_await {
                self.push(Kind::AwaitOutsideAsyncFunction, keyword, None);
            }
        } else if self.in_top_level_context() && parsed_as_await {
            // `checkGrammarAwaitOrAwaitUsing`: the checker keeps TS1309 for a
            // CommonJS-format file under a node module kind, and TS1378 under
            // a module kind or target that cannot run top-level `await`.
            self.push(Kind::Ts(1309), keyword, None);
            if !self.external_module {
                self.push(Kind::Ts(1375), keyword, None);
            }
            self.push(Kind::Ts(1378), keyword, None);
        }
        oxc_ast_visit::walk::walk_await_expression(self, expression);
    }

    fn visit_class_body(&mut self, body: &oxc_ast::ast::ClassBody<'a>) {
        self.this_container_depth += 1;
        oxc_ast_visit::walk::walk_class_body(self, body);
        self.this_container_depth -= 1;
    }

    // tsc's parser clears the await context for a class field's initializer
    // (`parsePropertyDeclaration`) and an enum's members
    // (`parseEnumDeclaration`), and sets it for a static block's body
    // (`parseClassStaticBlockBody`), whatever function encloses them.
    fn visit_property_definition(&mut self, property: &oxc_ast::ast::PropertyDefinition<'a>) {
        self.visit_decorators(&property.decorators);
        self.visit_property_key(&property.key);
        if let Some(type_annotation) = &property.type_annotation {
            self.visit_ts_type_annotation(type_annotation);
        }
        if property.type_annotation.is_none()
            && !property.declare
            && super::jsdoc::declared_type_at(property.span.start).is_none()
            && let Some(value) = &property.value
        {
            self.report_widening_member(property, value);
        }
        if let Some(value) = &property.value {
            self.await_context.push(false);
            self.visit_expression(value);
            self.await_context.pop();
        }
    }

    fn visit_accessor_property(&mut self, property: &oxc_ast::ast::AccessorProperty<'a>) {
        self.visit_decorators(&property.decorators);
        self.visit_property_key(&property.key);
        if let Some(type_annotation) = &property.type_annotation {
            self.visit_ts_type_annotation(type_annotation);
        }
        if let Some(value) = &property.value {
            self.await_context.push(false);
            self.visit_expression(value);
            self.await_context.pop();
        }
    }

    fn visit_ts_enum_body(&mut self, body: &oxc_ast::ast::TSEnumBody<'a>) {
        self.await_context.push(false);
        oxc_ast_visit::walk::walk_ts_enum_body(self, body);
        self.await_context.pop();
    }

    fn visit_static_block(&mut self, block: &oxc_ast::ast::StaticBlock<'a>) {
        self.await_context.push(true);
        oxc_ast_visit::walk::walk_static_block(self, block);
        self.await_context.pop();
    }

    fn visit_ts_type_parameter_declaration(
        &mut self,
        declaration: &oxc_ast::ast::TSTypeParameterDeclaration<'a>,
    ) {
        let mut seen_default = false;
        for parameter in &declaration.params {
            if parameter.default.is_some() {
                seen_default = true;
            } else if seen_default {
                self.push(Kind::RequiredTypeParameterAfterOptional, parameter.name.span, None);
            }
        }
        oxc_ast_visit::walk::walk_ts_type_parameter_declaration(self, declaration);
    }

    fn visit_formal_parameters(&mut self, parameters: &FormalParameters<'a>) {
        self.check_parameter_list(parameters);
        oxc_ast_visit::walk::walk_formal_parameters(self, parameters);
    }

    fn visit_method_definition(&mut self, method: &oxc_ast::ast::MethodDefinition<'a>) {
        if method.kind == MethodDefinitionKind::Get {
            self.check_getter_returns(method.key.span(), method.value.body.as_deref());
        }
        if method.kind == MethodDefinitionKind::Set && method.value.return_type.is_some() {
            self.push(Kind::SetAccessorReturnType, method.key.span(), None);
        }
        // `isPrivateWithinAmbient`: a private method of an ambient class
        // reports no implicit `any` return.
        let private_within_ambient = (self.is_ambient()
            || self.declare_class_depth > 0
            || writes_declare(&self.source_text, method))
            && (method.accessibility == Some(oxc_ast::ast::TSAccessibility::Private)
                || matches!(method.key, PropertyKey::PrivateIdentifier(_)));
        if method.kind == MethodDefinitionKind::Method
            && method.value.body.is_none()
            && method.value.return_type.is_none()
            && !private_within_ambient
            && let Some((span, name)) = self.implicit_any_member_name(&method.key, method.computed)
        {
            self.push(Kind::ImplicitAnyReturn, span, Some(&name));
        }
        if matches!(method.kind, MethodDefinitionKind::Method | MethodDefinitionKind::Get)
            && method.value.body.is_some()
            && method.value.return_type.is_none()
            && let Some((span, name)) = self.implicit_any_member_name(&method.key, method.computed)
        {
            self.report_widening_return(
                method.value.body.as_deref(),
                false,
                method.value.span.start,
                Some((span, &name)),
                span,
            );
        }
        oxc_ast_visit::walk::walk_method_definition(self, method);
    }

    fn visit_ts_property_signature(&mut self, property: &TSPropertySignature<'a>) {
        if property.type_annotation.is_none() && !property.computed
            && let Some(name) = property_key_name(&property.key) {
                self.push(Kind::ImplicitAnyMember, property.key.span(), Some(&name));
            }
        oxc_ast_visit::walk::walk_ts_property_signature(self, property);
    }

    fn visit_ts_call_signature_declaration(
        &mut self,
        signature: &oxc_ast::ast::TSCallSignatureDeclaration<'a>,
    ) {
        if signature.return_type.is_none() {
            self.push(Kind::ImplicitAnyCallReturn, signature.span, None);
        }
        self.check_signature_parameters(&signature.params);
        self.check_implicit_any_signature_parameters(&signature.params, true);
        oxc_ast_visit::walk::walk_ts_call_signature_declaration(self, signature);
    }

    fn visit_ts_construct_signature_declaration(
        &mut self,
        signature: &oxc_ast::ast::TSConstructSignatureDeclaration<'a>,
    ) {
        if signature.return_type.is_none() {
            self.push(Kind::ImplicitAnyConstructReturn, signature.span, None);
        }
        self.check_signature_parameters(&signature.params);
        self.check_implicit_any_signature_parameters(&signature.params, false);
        oxc_ast_visit::walk::walk_ts_construct_signature_declaration(self, signature);
    }

    fn visit_ts_function_type(&mut self, function: &oxc_ast::ast::TSFunctionType<'a>) {
        self.check_implicit_any_signature_parameters(&function.params, true);
        oxc_ast_visit::walk::walk_ts_function_type(self, function);
    }

    fn visit_ts_constructor_type(&mut self, constructor: &oxc_ast::ast::TSConstructorType<'a>) {
        self.check_implicit_any_signature_parameters(&constructor.params, false);
        oxc_ast_visit::walk::walk_ts_constructor_type(self, constructor);
    }

    fn visit_ts_method_signature(&mut self, method: &TSMethodSignature<'a>) {
        self.check_signature_parameters(&method.params);
        self.check_implicit_any_signature_parameters(&method.params, true);
        if method.kind == TSMethodSignatureKind::Method
            && method.return_type.is_none()
            && let Some((span, name)) = self.implicit_any_member_name(&method.key, method.computed)
        {
            self.push(Kind::ImplicitAnyReturn, span, Some(&name));
        }
        oxc_ast_visit::walk::walk_ts_method_signature(self, method);
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        self.check_variable_initializers(declaration);
        for declarator in &declaration.declarations {
            if declarator.type_annotation.is_none()
                && let Some(init) = &declarator.init
            {
                self.report_widening_binding_elements(&declarator.id, init);
                if let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                    self.report_widening_initializer(identifier, init);
                }
                if super::jsdoc::declared_type_at(declarator.span.start).is_none() {
                    let assigned_name = match &declarator.id {
                        oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) => Some(identifier.span),
                        _ => None,
                    };
                    self.report_widening_function_value(init, assigned_name);
                }
            }
        }
        oxc_ast_visit::walk::walk_variable_declaration(self, declaration);
    }

    fn visit_object_expression(&mut self, object: &ObjectExpression<'a>) {
        self.check_duplicate_properties(object);
        let accessors: Vec<AccessorRecord> = object
            .properties
            .iter()
            .filter_map(|property| match property {
                ObjectPropertyKind::ObjectProperty(property)
                    if matches!(property.kind, PropertyKind::Get | PropertyKind::Set) =>
                {
                    let Expression::FunctionExpression(function) = &property.value else {
                        return None;
                    };
                    let is_getter = property.kind == PropertyKind::Get;
                    let (key, display, span) = self.accessor_name(&property.key, property.computed)?;
                    Some(AccessorRecord {
                        key,
                        display,
                        is_static: false,
                        is_getter,
                        annotated: if is_getter {
                            function.return_type.is_some()
                                || super::jsdoc::return_type_at(function.span.start).is_some()
                        } else {
                            setter_parameter_annotated(&function.params)
                                || function.params.items.first().is_some_and(|parameter| {
                                    super::jsdoc::parameter_at(parameter.span.start)
                                        .is_some_and(|jsdoc| jsdoc.ty.is_some())
                                })
                        },
                        has_body: function.body.is_some(),
                        private: false,
                        span,
                    })
                }
                _ => None,
            })
            .collect();
        self.check_untyped_setters(&accessors, false);
        for property in &object.properties {
            if let ObjectPropertyKind::ObjectProperty(property) = property
                && property.kind == PropertyKind::Get
                && let Expression::FunctionExpression(function) = &property.value
            {
                self.check_getter_returns(property.key.span(), function.body.as_deref());
            }
        }
        oxc_ast_visit::walk::walk_object_expression(self, object);
    }

    // A `for (const x of xs)` binding is initialized by the loop, not by an
    // initializer, so its declaration must not reach the const check.
    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        if statement.r#await
            && self.in_top_level_context()
            && let Some(offset) = self.source_text[statement.span.start as usize..].find("await")
        {
            let start = statement.span.start + offset as u32;
            self.push(Kind::Ts(1309), Span::new(start, start + 5), None);
            if !self.external_module {
                self.push(Kind::Ts(1431), Span::new(start, start + 5), None);
            }
            self.push(Kind::Ts(1432), Span::new(start, start + 5), None);
        }
        self.check_using_loop_bindings(&statement.left);
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
    }

    // Only the outermost of nested parentheses is recorded, keyed by the
    // expression they wrap: that is the span the lowered tree keeps.
    fn visit_parenthesized_expression(
        &mut self,
        parenthesized: &oxc_ast::ast::ParenthesizedExpression<'a>,
    ) {
        let mut inner = &parenthesized.expression;
        while let Expression::ParenthesizedExpression(nested) = inner {
            inner = &nested.expression;
        }
        self.parenthesized_expressions.push(ParenthesizedExpressionSpan {
            inner: text_span_from_oxc_span(inner.span()),
            outer: text_span_from_oxc_span(parenthesized.span),
        });
        self.visit_expression(inner);
    }
}

/// How tsc's messages name a `using` declaration's keyword.
fn using_declaration_keyword(kind: VariableDeclarationKind) -> Option<&'static str> {
    match kind {
        VariableDeclarationKind::Using => Some("using"),
        VariableDeclarationKind::AwaitUsing => Some("await using"),
        _ => None,
    }
}

/// The dotted text of a computed key written as an entity name (`a`, `a.b`).
fn entity_name_text(key: &PropertyKey<'_>) -> Option<String> {
    fn expression_text(expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::Identifier(identifier) => Some(identifier.name.to_string()),
            Expression::StaticMemberExpression(member) => {
                Some(format!("{}.{}", expression_text(&member.object)?, member.property.name))
            }
            _ => None,
        }
    }
    match key {
        PropertyKey::Identifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StaticMemberExpression(member) => {
            Some(format!("{}.{}", expression_text(&member.object)?, member.property.name))
        }
        _ => None,
    }
}

/// tsgo's `nextTokenIsIdentifierOrKeywordOrLiteralOnSameLine`, which
/// `isAwaitExpression` asks outside an await context: whether the token after
/// the `await` that ends at `from` is a name, a keyword, or a string or numeric
/// literal on the same line.
fn await_operand_on_same_line(source_text: &str, from: u32) -> bool {
    let is_line_break = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let mut rest = source_text.get(from as usize..).unwrap_or("");
    loop {
        let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() && !is_line_break(c));
        if let Some(comment) = trimmed.strip_prefix("/*") {
            let Some(end) = comment.find("*/") else {
                return false;
            };
            if comment[..end].contains(is_line_break) {
                return false;
            }
            rest = &comment[end + 2..];
            continue;
        }
        let mut chars = trimmed.chars();
        return match chars.next() {
            Some(c) if c.is_alphanumeric() || matches!(c, '_' | '$' | '\\' | '\'' | '"') => true,
            Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        };
    }
}

fn property_key_name_of_enum_member(name: &TSEnumMemberName<'_>) -> Option<String> {
    match name {
        // The parser's nameless placeholder for a computed name (TS1164).
        TSEnumMemberName::Identifier(identifier) if identifier.name.is_empty() => None,
        TSEnumMemberName::Identifier(identifier) => Some(identifier.name.to_string()),
        TSEnumMemberName::String(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

/// Reports every identifier in `expression` naming one of `later`. The walk
/// covers the expression forms an enum initializer may take — an enum member
/// must be a constant expression, so there are no function bodies to stop at.
fn report_enum_forward_references(
    collector: &mut GrammarCollector,
    expression: &Expression<'_>,
    later: &[&str],
) {
    match expression {
        Expression::Identifier(identifier) => {
            if later.contains(&identifier.name.as_str()) {
                collector.push(Kind::EnumForwardReference, identifier.span, None);
            }
        }
        Expression::BinaryExpression(binary) => {
            report_enum_forward_references(collector, &binary.left, later);
            report_enum_forward_references(collector, &binary.right, later);
        }
        Expression::UnaryExpression(unary) => {
            report_enum_forward_references(collector, &unary.argument, later);
        }
        Expression::ParenthesizedExpression(parenthesized) => {
            report_enum_forward_references(collector, &parenthesized.expression, later);
        }
        Expression::TemplateLiteral(template) => {
            for interpolation in &template.expressions {
                report_enum_forward_references(collector, interpolation, later);
            }
        }
        _ => {}
    }
}

/// The constants tsc's `checkAmbientInitializer` accepts: a string, numeric,
/// bigint or boolean literal (a numeric or bigint one possibly negated) or a
/// literal enum reference. Whether a member access is enum-typed is the
/// checker's to know (`isInitializerSimpleLiteralEnumReference`); the walker
/// accepts every access of the enum-reference shape.
fn is_ambient_constant_initializer(expression: &Expression<'_>) -> bool {
    fn is_string_or_number(expression: &Expression<'_>) -> bool {
        match expression {
            Expression::StringLiteral(_) | Expression::NumericLiteral(_) => true,
            Expression::TemplateLiteral(template) => template.expressions.is_empty(),
            _ => false,
        }
    }
    fn is_entity_name(expression: &Expression<'_>) -> bool {
        match expression {
            Expression::Identifier(_) => true,
            Expression::StaticMemberExpression(member) => is_entity_name(&member.object),
            _ => false,
        }
    }
    match expression {
        Expression::BooleanLiteral(_) | Expression::BigIntLiteral(_) => true,
        Expression::UnaryExpression(unary) => {
            unary.operator == UnaryOperator::UnaryNegation
                && matches!(unary.argument, Expression::NumericLiteral(_) | Expression::BigIntLiteral(_))
        }
        Expression::StaticMemberExpression(_) => true,
        Expression::ComputedMemberExpression(member) => {
            is_string_or_number(&member.expression) && is_entity_name(&member.object)
        }
        other => is_string_or_number(other),
    }
}

/// `getMinArgumentCount` of a written parameter list: one past its last
/// parameter without `?` or an initializer. `None` when that parameter's type
/// could accept `void`, which tsc would also count as optional.
/// Whether a `declare` modifier is written before the method's name, which
/// oxc keeps no flag for on a method.
fn writes_declare(source_text: &str, method: &oxc_ast::ast::MethodDefinition<'_>) -> bool {
    source_text
        .get(method.span.start as usize..method.key.span().start as usize)
        .is_some_and(|modifiers| modifiers.split_whitespace().any(|word| word == "declare"))
}

/// The first JSDoc `@overload` of a JavaScript method that declares fewer
/// parameters than the implementation requires, a `@param` written optional
/// (`[b]`, `{T=}`) included.
fn jsdoc_overload_arity_mismatch(implementation: &oxc_ast::ast::MethodDefinition<'_>) -> Option<crate::TextSpan> {
    let required = implementation
        .value
        .params
        .items
        .iter()
        .rposition(|parameter| {
            !parameter.optional
                && parameter.initializer.is_none()
                && !super::jsdoc::parameter_at(parameter.span.start).is_some_and(|tag| tag.optional)
        })
        .map_or(0, |last| last + 1);
    super::jsdoc::overloads_at(implementation.value.span.start)
        .into_iter()
        .find_map(|(signature, span)| match signature {
            crate::ParsedType::Function(signature) if !signature.parameters.iter().any(|parameter| parameter.rest) => {
                let declared = signature.parameters.iter().filter(|parameter| !parameter.is_this).count();
                (required > declared).then_some(span)
            }
            _ => None,
        })
}

fn required_argument_count(parameters: &oxc_ast::ast::FormalParameters<'_>) -> Option<usize> {
    let Some(last) = parameters
        .items
        .iter()
        .rposition(|parameter| !parameter.optional && parameter.initializer.is_none())
    else {
        return Some(0);
    };
    let annotation = parameters.items[last].type_annotation.as_ref();
    if annotation.is_some_and(|annotation| {
        !matches!(
            annotation.type_annotation,
            oxc_ast::ast::TSType::TSStringKeyword(_)
                | oxc_ast::ast::TSType::TSNumberKeyword(_)
                | oxc_ast::ast::TSType::TSBooleanKeyword(_)
                | oxc_ast::ast::TSType::TSAnyKeyword(_)
                | oxc_ast::ast::TSType::TSArrayType(_)
        )
    }) {
        return None;
    }
    Some(last + 1)
}

/// A `null`, `undefined` or `void` expression, whose type is the widening
/// one (`nullWideningType`, `undefinedWideningType`).
fn widens(expression: &Expression<'_>) -> bool {
    match expression.without_parentheses() {
        Expression::NullLiteral(_) => true,
        Expression::Identifier(identifier) => identifier.name == "undefined",
        Expression::UnaryExpression(unary) => unary.operator == UnaryOperator::Void,
        _ => false,
    }
}

/// A non-empty array literal of nothing but widening elements.
fn array_widens(array: &oxc_ast::ast::ArrayExpression<'_>) -> bool {
    !array.elements.is_empty()
        && array
            .elements
            .iter()
            .all(|element| element.as_expression().is_some_and(widens))
}

/// Each member of an object literal that widens, a nested literal's included,
/// with the type it widens to (`reportWideningErrorsInType`); whether there
/// was one. A literal with a spread or a repeated name is left alone.
fn widening_properties(object: &ObjectExpression<'_>, out: &mut Vec<(Span, String, &'static str)>) -> bool {
    let mut names = std::collections::HashSet::new();
    let simple = object.properties.iter().all(|property| match property {
        ObjectPropertyKind::ObjectProperty(property) => {
            property.computed || property_key_name(&property.key).is_some_and(|name| names.insert(name))
        }
        ObjectPropertyKind::SpreadProperty(_) => false,
    });
    if !simple {
        return false;
    }
    let mut reported = false;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };
        if property.kind != PropertyKind::Init || property.method || property.computed {
            continue;
        }
        let widened = match property.value.without_parentheses() {
            value if widens(value) => Some("any"),
            Expression::ArrayExpression(array) if array_widens(array) => Some("any[]"),
            Expression::ObjectExpression(inner) => {
                reported |= widening_properties(inner, out);
                None
            }
            _ => None,
        };
        if let Some(widened) = widened {
            out.push((property.key.span(), property_key_name(&property.key).unwrap_or_default(), widened));
            reported = true;
        }
    }
    reported
}

/// What a function's inferred return type widens to without
/// `strictNullChecks` (`getReturnTypeFromBody`, then `reportErrorsFromWidening`).
enum ReturnWidening {
    /// The implicit `any` (or `any[]`) the function itself reports.
    Implicit(&'static str),
    /// A lone returned object literal's widening members, reported instead.
    Properties(Vec<(Span, String, &'static str)>),
}

enum ReturnShape {
    Nullish,
    NullishArray,
    Object(Vec<(Span, String, &'static str)>),
    Other,
}

fn return_shape(expression: Option<&Expression<'_>>) -> ReturnShape {
    let Some(expression) = expression else {
        return ReturnShape::Nullish;
    };
    match expression.without_parentheses() {
        value if widens(value) => ReturnShape::Nullish,
        Expression::ArrayExpression(array) if array.elements.is_empty() || array_widens(array) => {
            ReturnShape::NullishArray
        }
        Expression::ObjectExpression(object) => {
            let mut properties = Vec::new();
            widening_properties(object, &mut properties);
            ReturnShape::Object(properties)
        }
        // `return x = null`: the assignment has its right side's type.
        Expression::AssignmentExpression(assignment)
            if assignment.operator == oxc_syntax::operator::AssignmentOperator::Assign =>
        {
            return_shape(Some(&assignment.right))
        }
        _ => ReturnShape::Other,
    }
}

/// Every return `null`, `undefined` or `void` — a bare `return` among them
/// being `undefined` — leaves an implicit `any`, and with an array of nothing
/// else among them an `any[]`: subtype reduction drops the rest. Any other
/// return absorbs them all.
fn return_widening(body: &FunctionBody<'_>, expression_body: bool) -> Option<ReturnWidening> {
    struct Returns {
        shapes: Vec<ReturnShape>,
        valued: bool,
    }

    impl<'a> Visit<'a> for Returns {
        fn visit_return_statement(&mut self, statement: &oxc_ast::ast::ReturnStatement<'a>) {
            self.valued |= statement.argument.is_some();
            self.shapes.push(return_shape(statement.argument.as_ref()));
        }
        fn visit_function(&mut self, _: &Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}
        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
        fn visit_class(&mut self, _: &Class<'a>) {}
    }

    let mut returns = Returns {
        shapes: Vec::new(),
        valued: false,
    };
    if expression_body {
        let Some(Statement::ExpressionStatement(statement)) = body.statements.first() else {
            return None;
        };
        returns.valued = true;
        returns.shapes.push(return_shape(Some(&statement.expression)));
    } else {
        returns.visit_function_body(body);
    }
    if !returns.valued || returns.shapes.iter().any(|shape| matches!(shape, ReturnShape::Other)) {
        return None;
    }
    let arrays = returns.shapes.iter().any(|shape| matches!(shape, ReturnShape::NullishArray));
    let mut objects: Vec<Vec<(Span, String, &'static str)>> = returns
        .shapes
        .into_iter()
        .filter_map(|shape| match shape {
            ReturnShape::Object(properties) => Some(properties),
            _ => None,
        })
        .collect();
    match objects.len() {
        0 => Some(ReturnWidening::Implicit(if arrays { "any[]" } else { "any" })),
        1 if !arrays => {
            let properties = objects.pop().expect("one object literal");
            (!properties.is_empty()).then_some(ReturnWidening::Properties(properties))
        }
        _ => None,
    }
}

/// A written property type that `undefined` is not assignable to, read off
/// the annotation alone: an alias or type parameter might admit it.
fn type_excludes_undefined(ty: &oxc_ast::ast::TSType<'_>) -> bool {
    use oxc_ast::ast::TSType;
    match ty {
        TSType::TSNumberKeyword(_)
        | TSType::TSStringKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSBigIntKeyword(_)
        | TSType::TSSymbolKeyword(_)
        | TSType::TSObjectKeyword(_)
        | TSType::TSArrayType(_)
        | TSType::TSTupleType(_)
        | TSType::TSLiteralType(_)
        | TSType::TSTypeLiteral(_)
        | TSType::TSFunctionType(_) => true,
        TSType::TSUnionType(union) => union
            .types
            .iter()
            .all(|member| matches!(member, TSType::TSNullKeyword(_)) || type_excludes_undefined(member)),
        TSType::TSParenthesizedType(inner) => type_excludes_undefined(&inner.type_annotation),
        _ => false,
    }
}

/// The `this.p` / `this.#p` accesses of a constructor body: reads made by
/// the constructor itself (a nested function is its own flow container), and
/// writes anywhere. Anything inside an assignment target counts as a write,
/// and a `for` update is skipped since it runs after the body it follows.
#[derive(Default)]
struct ThisPropertyAccesses {
    reads: Vec<(String, Span)>,
    writes: Vec<(String, u32)>,
    writing: usize,
    nested: usize,
}

impl ThisPropertyAccesses {
    fn record(&mut self, object: &Expression<'_>, name: String, span: Span) {
        if !matches!(object.without_parentheses(), Expression::ThisExpression(_)) {
            return;
        }
        if self.writing > 0 {
            self.writes.push((name, span.start));
        } else if self.nested == 0 {
            self.reads.push((name, span));
        }
    }
}

impl<'a> Visit<'a> for ThisPropertyAccesses {
    fn visit_static_member_expression(&mut self, member: &oxc_ast::ast::StaticMemberExpression<'a>) {
        self.record(&member.object, member.property.name.to_string(), member.property.span);
        oxc_ast_visit::walk::walk_static_member_expression(self, member);
    }

    fn visit_private_field_expression(&mut self, member: &oxc_ast::ast::PrivateFieldExpression<'a>) {
        self.record(&member.object, format!("#{}", member.field.name), member.field.span);
        oxc_ast_visit::walk::walk_private_field_expression(self, member);
    }

    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        self.writing += 1;
        oxc_ast_visit::walk::walk_assignment_target(self, target);
        self.writing -= 1;
    }

    fn visit_simple_assignment_target(&mut self, target: &oxc_ast::ast::SimpleAssignmentTarget<'a>) {
        self.writing += 1;
        oxc_ast_visit::walk::walk_simple_assignment_target(self, target);
        self.writing -= 1;
    }

    fn visit_for_statement(&mut self, statement: &oxc_ast::ast::ForStatement<'a>) {
        if let Some(init) = &statement.init {
            self.visit_for_statement_init(init);
        }
        if let Some(test) = &statement.test {
            self.visit_expression(test);
        }
        self.visit_statement(&statement.body);
        if let Some(update) = &statement.update {
            self.nested += 1;
            self.visit_expression(update);
            self.nested -= 1;
        }
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.nested += 1;
        oxc_ast_visit::walk::walk_function(self, function, flags);
        self.nested -= 1;
    }

    fn visit_arrow_function_expression(&mut self, arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) {
        self.nested += 1;
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.nested -= 1;
    }

    fn visit_class(&mut self, _class: &Class<'a>) {}
}

fn undefaulted_binding_leaves<'p, 'a>(
    pattern: &'p oxc_ast::ast::BindingPattern<'a>,
    leaves: &mut Vec<&'p oxc_ast::ast::BindingIdentifier<'a>>,
) {
    use oxc_ast::ast::BindingPattern;
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => leaves.push(identifier),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                undefaulted_binding_leaves(&property.value, leaves);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                undefaulted_binding_leaves(element, leaves);
            }
        }
        BindingPattern::AssignmentPattern(_) => {}
    }
}
