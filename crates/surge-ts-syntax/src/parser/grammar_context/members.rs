//! Member, parameter, and statement placement rules tsc answers from a node's
//! parent chain: `checkGrammarModifiers` (`abstract`/`accessor` on the wrong
//! declaration, modifiers on `this`), `checkGrammarAccessor`,
//! `checkParameter`, `checkTypePredicate`, the private-name and `arguments`
//! placement checks, `return` in setters and static blocks, a for-in
//! destructuring target, `super()` under `extends null`, and the parser's
//! "`super` must be followed by an argument list or member access".
//!
//! oxc drops the modifiers tsc rejects here (`abstract enum`, `accessor` on a
//! method, `public` on a rest parameter), so those are read back from the
//! source text around the node.

use oxc_ast::AstKind;
use oxc_ast::ast::{
    AccessorPropertyType, BindingPattern, Expression, ForStatementLeft, FormalParameter,
    FormalParameters, MethodDefinition, MethodDefinitionKind, MethodDefinitionType,
    PropertyDefinitionType, PropertyKey, PropertyKind, TSMethodSignatureKind, TSThisParameter,
    TSTypeName, TSTypePredicate, TSTypePredicateName,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};

use super::{ContextCollector, collect_binding_names, computed_key_contains, first_token_end};

const MODIFIER_WORDS: &[&str] = &[
    "public", "private", "protected", "readonly", "override", "abstract", "accessor", "static",
    "declare", "async", "export", "default", "const",
];

impl<'a> ContextCollector<'a, '_> {
    /// Runs before `kind` is pushed, so `self.stack` holds its ancestors.
    pub(super) fn check_member_placement(&mut self, kind: &AstKind<'a>) {
        match kind {
            AstKind::PrivateInExpression(expression) => {
                if !self.in_class() {
                    self.push(18016, expression.left.span, &[]);
                }
            }
            AstKind::ObjectProperty(property) => {
                if let PropertyKey::PrivateIdentifier(key) = &property.key {
                    self.push(18016, key.span, &[]);
                }
                if property.kind != PropertyKind::Init
                    && let Expression::FunctionExpression(function) = &property.value
                {
                    self.check_accessor_parameters(
                        property.kind == PropertyKind::Get,
                        self.member_name_span(&property.key, property.computed),
                        function.this_param.as_deref(),
                        &function.params,
                        function.return_type.is_some(),
                    );
                }
            }
            AstKind::TSPropertySignature(signature) => {
                if let PropertyKey::PrivateIdentifier(key) = &signature.key {
                    self.push(18016, key.span, &[]);
                }
            }
            AstKind::TSMethodSignature(signature) => {
                if let PropertyKey::PrivateIdentifier(key) = &signature.key
                    && !self.in_class()
                {
                    self.push(18016, key.span, &[]);
                }
                if signature.kind != TSMethodSignatureKind::Method {
                    self.check_accessor_parameters(
                        signature.kind == TSMethodSignatureKind::Get,
                        self.member_name_span(&signature.key, signature.computed),
                        signature.this_param.as_deref(),
                        &signature.params,
                        signature.return_type.is_some(),
                    );
                }
            }
            AstKind::MethodDefinition(method) => self.check_method_definition(method),
            AstKind::PropertyDefinition(property) => {
                self.check_constructor_field_name(&property.key, property.computed);
                if property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
                    && property.value.is_some()
                {
                    self.report_abstract_initializer(&property.key, property.computed);
                }
            }
            AstKind::AccessorProperty(property) => {
                self.check_constructor_field_name(&property.key, property.computed);
                if property.r#type == AccessorPropertyType::TSAbstractAccessorProperty
                    && property.value.is_some()
                {
                    self.report_abstract_initializer(&property.key, property.computed);
                }
            }
            AstKind::FormalParameter(parameter) => self.check_parameter(parameter),
            AstKind::FormalParameters(parameters) => self.check_rest_parameter_property(parameters),
            AstKind::BindingRestElement(rest) => self.check_rest_element_initializer(rest),
            AstKind::ForInStatement(statement) => match &statement.left {
                ForStatementLeft::VariableDeclaration(declaration) => {
                    // `checkGrammarForInOrForOfStatement`'s `using` rule.
                    match declaration.kind {
                        oxc_ast::ast::VariableDeclarationKind::Using => {
                            self.push(1493, declaration.span, &[]);
                        }
                        oxc_ast::ast::VariableDeclarationKind::AwaitUsing => {
                            self.push(1494, declaration.span, &[]);
                        }
                        _ => {}
                    }
                    if let Some(first) = declaration.declarations.first()
                        && !matches!(first.id, BindingPattern::BindingIdentifier(_))
                    {
                        self.push(2491, first.id.span(), &[]);
                    }
                }
                ForStatementLeft::ArrayAssignmentTarget(_)
                | ForStatementLeft::ObjectAssignmentTarget(_) => {
                    self.push(2491, statement.left.span(), &[]);
                }
                _ => {}
            },
            AstKind::ReturnStatement(statement) => {
                self.check_return_container(statement.span, statement.argument.is_some());
            }
            AstKind::TaggedTemplateExpression(template) => {
                if is_optional_chain(&template.tag)
                    || matches!(self.stack.last(), Some(AstKind::ChainExpression(_)))
                {
                    self.push(1358, template.quasi.span, &[]);
                }
            }
            AstKind::IdentifierReference(identifier) if identifier.name == "arguments" => {
                self.check_arguments_reference(identifier.span);
            }
            AstKind::TSTypeReference(reference) => {
                if let TSTypeName::IdentifierReference(identifier) = &reference.type_name {
                    self.check_computed_name_type_parameter(&identifier.name, identifier.span);
                }
            }
            AstKind::TSTypePredicate(predicate) => self.check_type_predicate(predicate),
            AstKind::Function(function)
                if matches!(
                    function.r#type,
                    oxc_ast::ast::FunctionType::FunctionDeclaration
                        | oxc_ast::ast::FunctionType::TSDeclareFunction
                ) =>
            {
                self.check_abstract_declaration(function.span.start);
            }
            AstKind::TSEnumDeclaration(declaration) => {
                self.check_abstract_declaration(declaration.span.start);
            }
            AstKind::TSInterfaceDeclaration(declaration) => {
                self.check_abstract_declaration(declaration.span.start);
            }
            AstKind::TSTypeAliasDeclaration(declaration) => {
                self.check_abstract_declaration(declaration.span.start);
            }
            AstKind::TSModuleDeclaration(declaration) => {
                self.check_abstract_declaration(declaration.span.start);
            }
            AstKind::VariableDeclaration(declaration) => {
                self.check_abstract_declaration(declaration.span.start);
            }
            _ => {}
        }
    }

    fn check_method_definition(&mut self, method: &MethodDefinition<'a>) {
        let modifiers =
            self.leading_modifiers(method.span.start, &method.decorators, method.key.span().start);
        if let Some(span) = modifier_span(&modifiers, "accessor") {
            self.push(1275, span, &[]);
        }
        if method.kind == MethodDefinitionKind::Constructor
            && method.r#type == MethodDefinitionType::TSAbstractMethodDefinition
            && let Some(span) = modifier_span(&modifiers, "abstract")
        {
            self.push(1242, span, &[]);
        }
        match method.kind {
            MethodDefinitionKind::Get | MethodDefinitionKind::Set => self.check_accessor_parameters(
                method.kind == MethodDefinitionKind::Get,
                self.member_name_span(&method.key, method.computed),
                method.value.this_param.as_deref(),
                &method.value.params,
                method.value.return_type.is_some(),
            ),
            MethodDefinitionKind::Constructor => {
                if let Some(body) = &method.value.body {
                    self.check_super_call_under_null_base(body);
                }
            }
            MethodDefinitionKind::Method => {}
        }
    }

    /// tsc's `checkGrammarAccessor` parameter rules, then `checkParameter`'s
    /// TS2784 for a `this` parameter, which runs even after a count error.
    fn check_accessor_parameters(
        &mut self,
        is_getter: bool,
        name_span: Span,
        this_param: Option<&TSThisParameter<'a>>,
        params: &FormalParameters<'a>,
        has_return_type: bool,
    ) {
        let expected = usize::from(!is_getter);
        let total = params.items.len()
            + usize::from(params.rest.is_some())
            + usize::from(this_param.is_some());
        let count_ok = total == expected || (this_param.is_some() && total == expected + 1);
        if !count_ok {
            self.push(if is_getter { 1054 } else { 1049 }, name_span, &[]);
        } else if !is_getter && !has_return_type {
            // A return type is TS1095, which tsc stops at.
            if let Some(rest) = &params.rest {
                let start = rest.span.start;
                self.push(1053, Span::new(start, start + 3), &[]);
            } else if let Some(parameter) = params.items.first() {
                let jsdoc_question = (!parameter.optional && parameter.type_annotation.is_none())
                    .then(|| super::super::jsdoc::parameter_at(parameter.span.start))
                    .flatten()
                    .and_then(|jsdoc| jsdoc.question);
                if parameter.optional {
                    if let Some(question) = self.question_token(parameter) {
                        self.push(1051, question, &[]);
                    }
                } else if let Some(question) = jsdoc_question {
                    self.push(1051, Span::new(question.start as u32, question.end as u32), &[]);
                } else if parameter.initializer.is_some() {
                    self.push(1052, name_span, &[]);
                }
            }
        }
        if let Some(this_param) = this_param {
            self.push(2784, this_param.this_span, &[]);
        }
    }

    fn question_token(&self, parameter: &FormalParameter<'a>) -> Option<Span> {
        let start = skip_trivia(self.source_text, parameter.pattern.span().end as usize);
        self.source_text[start..]
            .starts_with('?')
            .then(|| Span::new(start as u32, start as u32 + 1))
    }

    /// tsc's `checkParameter`, plus the `abstract`/`accessor` arms of
    /// `checkGrammarModifiers` and its `this`-parameter rule.
    fn check_parameter(&mut self, parameter: &FormalParameter<'a>) {
        let modifiers = self.leading_modifiers(
            parameter.span.start,
            &parameter.decorators,
            parameter.pattern.span().start,
        );
        let is_this = matches!(
            &parameter.pattern,
            BindingPattern::BindingIdentifier(identifier) if identifier.name == "this"
        );
        if is_this {
            if !parameter.decorators.is_empty()
                || parameter.accessibility.is_some()
                || parameter.readonly
                || parameter.r#override
                || !modifiers.is_empty()
            {
                let start = parameter.span.start;
                let end = first_token_end(self.source_text, start as usize) as u32;
                self.push(1433, Span::new(start, end), &[]);
            }
            let (owner, _) = self.parameter_owner();
            let is_first = matches!(
                self.stack.last(),
                Some(AstKind::FormalParameters(parameters))
                    if parameters.items.first().is_some_and(|first| first.span == parameter.span)
            );
            if !is_first {
                self.push(2680, parameter.pattern.span(), &["this"]);
            }
            if owner.is_some_and(|owner| self.signature_is_accessor(owner)) {
                self.push(2784, parameter.pattern.span(), &[]);
            }
        } else {
            for (word, span) in &modifiers {
                match *word {
                    "abstract" => self.push(1242, *span, &[]),
                    "accessor" => self.push(1275, *span, &[]),
                    _ => {}
                }
            }
        }
        // An optional `@param` makes a JavaScript parameter optional
        // (`isOptionalDeclaration`).
        let optional = parameter.optional
            || (parameter.type_annotation.is_none()
                && super::super::jsdoc::parameter_at(parameter.span.start)
                    .is_some_and(|jsdoc| jsdoc.question.is_some()));
        if parameter.initializer.is_none()
            && optional
            && matches!(
                parameter.pattern,
                BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
            )
            && self.parameter_owner().0.is_some_and(|owner| match owner {
                AstKind::Function(function) => function.body.is_some(),
                AstKind::ArrowFunctionExpression(_) => true,
                _ => false,
            })
        {
            self.push(2463, parameter.pattern.span(), &[]);
        }
    }

    /// The signature a parameter list belongs to and, for a `function` node,
    /// the member or property that owns the function.
    fn parameter_owner(&self) -> (Option<AstKind<'a>>, Option<AstKind<'a>>) {
        let len = self.stack.len();
        if !matches!(self.stack.last(), Some(AstKind::FormalParameters(_))) || len < 2 {
            return (None, None);
        }
        (
            Some(self.stack[len - 2]),
            len.checked_sub(3).map(|index| self.stack[index]),
        )
    }

    fn signature_is_accessor(&self, owner: AstKind<'a>) -> bool {
        match owner {
            AstKind::TSMethodSignature(signature) => {
                signature.kind != TSMethodSignatureKind::Method
            }
            AstKind::Function(_) => match self.parameter_owner().1 {
                Some(AstKind::MethodDefinition(method)) => {
                    matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set)
                }
                Some(AstKind::ObjectProperty(property)) => property.kind != PropertyKind::Init,
                _ => false,
            },
            _ => false,
        }
    }

    /// TS1317: a rest parameter carrying a parameter-property modifier. oxc
    /// keeps no modifier on a rest parameter, so it is read from the text.
    fn check_rest_parameter_property(&mut self, parameters: &FormalParameters<'a>) {
        let Some(rest) = &parameters.rest else {
            return;
        };
        let decorators_end = rest.decorators.iter().map(|decorator| decorator.span.end).max();
        let dots = self.source_text[rest.span.start as usize..]
            .find("...")
            .map_or(rest.span.start, |offset| rest.span.start + offset as u32);
        let mut modifiers = self.leading_modifiers(rest.span.start, &rest.decorators, dots);
        if modifiers.is_empty() {
            modifiers = self.preceding_modifiers(rest.span.start, decorators_end);
        }
        if let Some(first) = modifiers.iter().find(|(word, _)| {
            matches!(*word, "public" | "private" | "protected" | "readonly" | "override")
        }) {
            let start = first.1.start.min(rest.span.start);
            self.push(1317, Span::new(start, rest.span.end), &[]);
        }
    }

    /// TS1242 on a declaration statement written with `abstract`, which oxc
    /// parses and drops.
    fn check_abstract_declaration(&mut self, start: u32) {
        let text = &self.source_text[start as usize..];
        if let Some(rest) = text.strip_prefix("abstract")
            && rest.starts_with(|c: char| c.is_whitespace())
        {
            self.push(1242, Span::new(start, start + 8), &[]);
            return;
        }
        if let Some((_, span)) = self
            .preceding_modifiers(start, None)
            .into_iter()
            .find(|(word, _)| *word == "abstract")
        {
            self.push(1242, span, &[]);
        }
    }

    fn check_constructor_field_name(&mut self, key: &PropertyKey<'a>, computed: bool) {
        if !computed
            && let PropertyKey::StringLiteral(literal) = key
            && literal.value == "constructor"
            && matches!(self.stack.last(), Some(AstKind::ClassBody(_)))
        {
            self.push(18006, literal.span, &[]);
        }
    }

    fn report_abstract_initializer(&mut self, key: &PropertyKey<'a>, computed: bool) {
        let span = key.span();
        let text = &self.source_text[span.start as usize..span.end as usize];
        let name = if computed { format!("[{text}]") } else { text.to_string() };
        self.push(1267, span, &[name.as_str()]);
    }

    /// TS18041 for a `return` whose nearest container is a static block,
    /// otherwise TS2408 for a value returned from a `set` accessor.
    fn check_return_container(&mut self, span: Span, has_value: bool) {
        for index in (0..self.stack.len()).rev() {
            let kind = self.stack[index];
            match kind {
                AstKind::StaticBlock(_) => {
                    self.push(18041, Span::new(span.start, span.start + 6), &[]);
                    return;
                }
                AstKind::ArrowFunctionExpression(_) => return,
                AstKind::Function(_) => {
                    let is_setter = match index.checked_sub(1).map(|parent| self.stack[parent]) {
                        Some(AstKind::MethodDefinition(method)) => {
                            method.kind == MethodDefinitionKind::Set
                        }
                        Some(AstKind::ObjectProperty(property)) => {
                            property.kind == PropertyKind::Set
                        }
                        _ => false,
                    };
                    if is_setter && has_value && self.ambient_depth == 0 {
                        self.push(2408, span, &[]);
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    /// TS17005: the first `super()` in a constructor of a class whose
    /// `extends` clause is the `null` literal. tsc decides this from the base
    /// constructor type, so a base that is a variable of type `null` is
    /// missed here.
    fn check_super_call_under_null_base(&mut self, body: &oxc_ast::ast::FunctionBody<'a>) {
        let len = self.stack.len();
        let Some(AstKind::Class(class)) = len.checked_sub(2).map(|index| self.stack[index]) else {
            return;
        };
        if !matches!(
            class.super_class.as_ref().map(Expression::without_parentheses),
            Some(Expression::NullLiteral(_))
        ) {
            return;
        }
        let mut finder = FirstSuperCall { found: None };
        finder.visit_function_body(body);
        if let Some(span) = finder.found {
            self.push(17005, span, &[]);
        }
    }

    /// tsc's `checkIdentifier` for `arguments`: TS2815 when the reference sits
    /// in a property initializer or static block (arrows do not shield it)
    /// and still resolves to some enclosing function's `arguments`.
    fn check_arguments_reference(&mut self, span: Span) {
        let mut member = None;
        for (index, kind) in self.stack.iter().enumerate().rev() {
            match kind {
                AstKind::PropertyDefinition(_)
                | AstKind::AccessorProperty(_)
                | AstKind::StaticBlock(_) => {
                    member = Some(index);
                    break;
                }
                AstKind::TSTypeQuery(_) => return,
                AstKind::FunctionBody(_)
                    if matches!(
                        index.checked_sub(1).map(|parent| self.stack[parent]),
                        Some(AstKind::Function(_))
                    ) =>
                {
                    return;
                }
                _ => {}
            }
        }
        let Some(member) = member else {
            return;
        };
        if self.stack[..member]
            .iter()
            .any(|kind| matches!(kind, AstKind::Function(_)))
        {
            self.push(2815, span, &[]);
        }
    }

    /// The binder's TS2467: a type reference in a member's computed name
    /// resolving to a type parameter of the class or interface that owns the
    /// member. Anything inside the name that declares the same name first
    /// (a generic arrow, a nested class) shadows it.
    fn check_computed_name_type_parameter(&mut self, name: &str, span: Span) {
        let mut child_span = span;
        for index in (0..self.stack.len()).rev() {
            let kind = self.stack[index];
            let in_computed_key = computed_key_contains(&kind, child_span)
                || match kind {
                    AstKind::TSPropertySignature(member) if member.computed => {
                        span_contains(member.key.span(), child_span)
                    }
                    AstKind::TSMethodSignature(member) if member.computed => {
                        span_contains(member.key.span(), child_span)
                    }
                    _ => false,
                };
            if in_computed_key {
                let owner = match (
                    index.checked_sub(1).map(|body| self.stack[body]),
                    index.checked_sub(2).map(|owner| self.stack[owner]),
                ) {
                    (Some(AstKind::ClassBody(_)), Some(AstKind::Class(class))) => {
                        class.type_parameters.as_deref()
                    }
                    (
                        Some(AstKind::TSInterfaceBody(_)),
                        Some(AstKind::TSInterfaceDeclaration(interface)),
                    ) => interface.type_parameters.as_deref(),
                    _ => return,
                };
                if owner.is_some_and(|parameters| {
                    parameters.params.iter().any(|parameter| parameter.name.name == name)
                }) {
                    self.push(2467, span, &[]);
                }
                return;
            }
            if declares_type_parameter(&kind, name) {
                return;
            }
            child_span = kind.span();
        }
    }

    /// A rest element written with an initializer: on a parameter it is
    /// `checkGrammarParameterList`'s TS1048 at the name, elsewhere
    /// `checkGrammarBindingElement`'s TS1186 at the `=`.
    fn check_rest_element_initializer(&mut self, rest: &oxc_ast::ast::BindingRestElement<'a>) {
        let BindingPattern::AssignmentPattern(pattern) = &rest.argument else {
            return;
        };
        if matches!(self.stack.last(), Some(AstKind::FormalParameterRest(_))) {
            self.push(1048, pattern.left.span(), &[]);
            return;
        }
        let from = pattern.left.span().end as usize;
        let to = pattern.right.span().start as usize;
        if let Some(offset) = self.source_text.get(from..to).and_then(|text| text.find('=')) {
            let start = (from + offset) as u32;
            self.push(1186, Span::new(start, start + 1), &[]);
        }
    }

    /// tsc's `checkTypePredicate`. Its parser only accepts a predicate as a
    /// return type, so one anywhere else is a parse error there (and nothing
    /// here); a return position that is not a function, arrow, function type,
    /// call signature or method — an accessor, a constructor, a constructor
    /// type or construct signature — is TS1228. A predicate naming no
    /// parameter is TS1225, or TS1230 when the name is declared inside a
    /// binding-pattern parameter.
    fn check_type_predicate(&mut self, predicate: &TSTypePredicate<'a>) {
        let len = self.stack.len();
        let Some(AstKind::TSTypeAnnotation(annotation)) = self.stack.last() else {
            return;
        };
        let is_return = |return_type: Option<&oxc_ast::ast::TSTypeAnnotation<'a>>| {
            return_type.is_some_and(|candidate| candidate.span == annotation.span)
        };
        // `Ok(params)` for a signature that may carry a predicate, `Err(())`
        // for a return position that may not.
        let owner: Result<&FormalParameters<'a>, ()> =
            match len.checked_sub(2).map(|index| self.stack[index]) {
                Some(AstKind::Function(function)) if is_return(function.return_type.as_deref()) => {
                    match len.checked_sub(3).map(|index| self.stack[index]) {
                        Some(AstKind::MethodDefinition(method))
                            if method.kind != MethodDefinitionKind::Method =>
                        {
                            Err(())
                        }
                        Some(AstKind::ObjectProperty(property))
                            if property.kind != PropertyKind::Init =>
                        {
                            Err(())
                        }
                        _ => Ok(&*function.params),
                    }
                }
                Some(AstKind::ArrowFunctionExpression(arrow))
                    if is_return(arrow.return_type.as_deref()) =>
                {
                    Ok(&*arrow.params)
                }
                Some(AstKind::TSFunctionType(function))
                    if is_return(Some(&*function.return_type)) =>
                {
                    Ok(&*function.params)
                }
                Some(AstKind::TSMethodSignature(signature))
                    if is_return(signature.return_type.as_deref()) =>
                {
                    if signature.kind == TSMethodSignatureKind::Method {
                        Ok(&*signature.params)
                    } else {
                        Err(())
                    }
                }
                Some(AstKind::TSCallSignatureDeclaration(signature))
                    if is_return(signature.return_type.as_deref()) =>
                {
                    Ok(&*signature.params)
                }
                Some(AstKind::TSConstructorType(constructor))
                    if is_return(Some(&*constructor.return_type)) =>
                {
                    Err(())
                }
                Some(AstKind::TSConstructSignatureDeclaration(signature))
                    if is_return(signature.return_type.as_deref()) =>
                {
                    Err(())
                }
                // A `this` predicate parses wherever the `this` type does
                // (`var x: this is string`, a setter's parameter).
                _ if matches!(predicate.parameter_name, TSTypePredicateName::This(_)) => Err(()),
                _ => return,
            };
        let params = match owner {
            Ok(params) => params,
            Err(()) => {
                self.push(1228, predicate.span, &[]);
                return;
            }
        };
        let TSTypePredicateName::Identifier(name) = &predicate.parameter_name else {
            return;
        };
        let names_identifier = |pattern: &BindingPattern<'a>| {
            matches!(pattern, BindingPattern::BindingIdentifier(identifier) if identifier.name == name.name)
        };
        if params.items.iter().any(|parameter| names_identifier(&parameter.pattern))
            // A rest parameter is TS1229, which surge does not emit.
            || params.rest.as_ref().is_some_and(|rest| names_identifier(&rest.rest.argument))
        {
            return;
        }
        for parameter in &params.items {
            let mut declared = Vec::new();
            collect_binding_names(&parameter.pattern, &mut declared);
            if declared.iter().any(|(declared, _)| *declared == name.name.as_str()) {
                self.push(1230, name.span, &[name.name.as_str()]);
                return;
            }
        }
        self.push(1225, name.span, &[name.name.as_str()]);
    }

    /// The modifier keywords between a node's start (past its decorators) and
    /// `until`, each with its span.
    fn leading_modifiers(
        &self,
        start: u32,
        decorators: &[oxc_ast::ast::Decorator<'a>],
        until: u32,
    ) -> Vec<(&'a str, Span)> {
        let from = decorators
            .iter()
            .map(|decorator| decorator.span.end)
            .max()
            .unwrap_or(start)
            .max(start) as usize;
        let mut words = Vec::new();
        let mut position = from;
        let until = until as usize;
        while position < until {
            position = skip_trivia(self.source_text, position);
            if position >= until {
                break;
            }
            let end = first_token_end(self.source_text, position);
            let word = &self.source_text[position..end];
            if !word.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$') {
                break;
            }
            words.push((word, Span::new(position as u32, end as u32)));
            position = end;
        }
        words
    }

    /// The modifier keywords written on the same line right before `start`,
    /// for a node whose span oxc begins after them. `floor` bounds the scan
    /// (a decorator's end); a span that already covers its decorators, as a
    /// decorated rest parameter's does, has nothing before it to read.
    fn preceding_modifiers(&self, start: u32, floor: Option<u32>) -> Vec<(&'a str, Span)> {
        let floor = floor.unwrap_or(0).min(start) as usize;
        let mut words = Vec::new();
        let mut end = start as usize;
        loop {
            let before = &self.source_text[floor..end];
            let trimmed = before.trim_end_matches([' ', '\t']);
            let word_end = floor + trimmed.len();
            let word_start = floor
                + trimmed
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                    .map_or(0, |index| index + trimmed[index..].chars().next().map_or(1, char::len_utf8));
            let word = &self.source_text[word_start..word_end];
            if word.is_empty() || !MODIFIER_WORDS.contains(&word) {
                break;
            }
            words.insert(0, (word, Span::new(word_start as u32, word_end as u32)));
            end = word_start;
        }
        words
    }
}

fn modifier_span(modifiers: &[(&str, Span)], word: &str) -> Option<Span> {
    modifiers
        .iter()
        .find(|(candidate, _)| *candidate == word)
        .map(|(_, span)| *span)
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// The offset of the first character after whitespace and comments at
/// `position`.
pub(super) fn skip_trivia(text: &str, mut position: usize) -> usize {
    loop {
        let rest = &text[position..];
        let trimmed = rest.trim_start();
        position += rest.len() - trimmed.len();
        if let Some(comment) = trimmed.strip_prefix("//") {
            position += 2 + comment.find('\n').unwrap_or(comment.len());
        } else if let Some(comment) = trimmed.strip_prefix("/*") {
            position += 2 + comment.find("*/").map_or(comment.len(), |end| end + 2);
        } else {
            return position;
        }
    }
}

/// tsc's `NodeFlagsOptionalChain` on a tagged template's tag: a `?.` anywhere
/// in the unparenthesized chain leading to it.
fn is_optional_chain(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ChainExpression(_) => true,
        Expression::StaticMemberExpression(member) => {
            member.optional || is_optional_chain(&member.object)
        }
        Expression::ComputedMemberExpression(member) => {
            member.optional || is_optional_chain(&member.object)
        }
        Expression::PrivateFieldExpression(member) => {
            member.optional || is_optional_chain(&member.object)
        }
        Expression::CallExpression(call) => call.optional || is_optional_chain(&call.callee),
        Expression::TaggedTemplateExpression(template) => is_optional_chain(&template.tag),
        Expression::TSNonNullExpression(expression) => is_optional_chain(&expression.expression),
        _ => false,
    }
}

fn declares_type_parameter(kind: &AstKind<'_>, name: &str) -> bool {
    let declared = match kind {
        AstKind::Function(function) => function.type_parameters.as_deref(),
        AstKind::ArrowFunctionExpression(arrow) => arrow.type_parameters.as_deref(),
        AstKind::Class(class) => class.type_parameters.as_deref(),
        AstKind::TSFunctionType(function) => function.type_parameters.as_deref(),
        AstKind::TSConstructorType(constructor) => constructor.type_parameters.as_deref(),
        AstKind::TSMethodSignature(signature) => signature.type_parameters.as_deref(),
        AstKind::TSCallSignatureDeclaration(signature) => signature.type_parameters.as_deref(),
        AstKind::TSConstructSignatureDeclaration(signature) => {
            signature.type_parameters.as_deref()
        }
        AstKind::TSTypeAliasDeclaration(alias) => alias.type_parameters.as_deref(),
        AstKind::TSInterfaceDeclaration(interface) => interface.type_parameters.as_deref(),
        AstKind::TSMappedType(mapped) => return mapped.key.name == name,
        AstKind::TSInferType(infer) => return infer.type_parameter.name.name == name,
        _ => None,
    };
    declared.is_some_and(|parameters| {
        parameters.params.iter().any(|parameter| parameter.name.name == name)
    })
}

/// tsc's `findFirstSuperCall`: the first `super(...)` in a body, not looking
/// inside nested functions.
struct FirstSuperCall {
    found: Option<Span>,
}

impl<'a> Visit<'a> for FirstSuperCall {
    fn visit_call_expression(&mut self, call: &oxc_ast::ast::CallExpression<'a>) {
        if self.found.is_some() {
            return;
        }
        if matches!(call.callee, Expression::Super(_)) {
            self.found = Some(call.span);
            return;
        }
        oxc_ast_visit::walk::walk_call_expression(self, call);
    }

    fn visit_function(
        &mut self,
        _: &oxc_ast::ast::Function<'a>,
        _: oxc_syntax::scope::ScopeFlags,
    ) {
    }

    fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}
}
