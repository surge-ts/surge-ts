use oxc_allocator::{Box, Vec};
use oxc_ast::ast::*;
use oxc_span::{FileExtension, GetSpan, Span};

use crate::{
    Context, ParserConfig as Config, ParserImpl, diagnostics,
    js::{FunctionKind, VariableDeclarationParent},
    lexer::Kind,
    modifiers::{ModifierKind, ModifierKinds, Modifiers},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum CallOrConstructorSignature {
    Call,
    Constructor,
}

impl<'a, C: Config> ParserImpl<'a, C> {
    /* ------------------- Enum ------------------ */
    /// `https://www.typescriptlang.org/docs/handbook/enums.html`
    pub(crate) fn parse_ts_enum_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Declaration<'a> {
        self.bump_any(); // bump `enum`
        let id = self.parse_binding_identifier();
        let body = self.parse_ts_enum_body();
        let span = self.end_span(span);
        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare, ModifierKind::Const]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );
        self.ast.declaration_ts_enum(
            span,
            id,
            body,
            modifiers.contains_const(),
            modifiers.contains_declare(),
        )
    }

    pub(crate) fn parse_ts_enum_body(&mut self) -> TSEnumBody<'a> {
        let span = self.start_span();
        let opening_span = self.cur_token().span();
        self.expect(Kind::LCurly);
        let (members, _) = self.parse_delimited_list(
            Kind::RCurly,
            Kind::Comma,
            opening_span,
            Self::parse_ts_enum_member,
        );
        self.expect(Kind::RCurly);
        self.ast.ts_enum_body(self.end_span(span), members)
    }

    pub(crate) fn parse_ts_enum_member(&mut self) -> TSEnumMember<'a> {
        let span = self.start_span();
        let id = self.parse_ts_enum_member_name();
        let initializer = if self.eat(Kind::Eq) {
            Some(self.parse_assignment_expression_or_higher())
        } else {
            None
        };
        self.ast.ts_enum_member(self.end_span(span), id, initializer)
    }

    fn parse_ts_enum_member_name(&mut self) -> TSEnumMemberName<'a> {
        let name_start = self.start_span();
        match self.cur_kind() {
            Kind::Str => {
                let literal = self.parse_literal_string();
                TSEnumMemberName::String(self.alloc(literal))
            }
            Kind::LBrack => match self.parse_computed_property_name() {
                Expression::StringLiteral(literal) => TSEnumMemberName::ComputedString(literal),
                Expression::TemplateLiteral(template) if template.is_no_substitution_template() => {
                    TSEnumMemberName::ComputedTemplateString(template)
                }
                // surge: TS2452 — tsc keeps a numeric name and its checker rejects it
                // (`computeEnumMemberValue`); the name is kept as its text.
                Expression::NumericLiteral(literal) => {
                    let text = self.ast.str(&literal.value.to_string());
                    TSEnumMemberName::ComputedString(
                        self.alloc(self.ast.string_literal(literal.span(), text, None)),
                    )
                }
                // surge: TS1164 — tsc reports every such member at its `[…]` name and keeps
                // parsing; the member keeps an empty placeholder name.
                _ => {
                    let span = self.end_span(name_start);
                    self.error(diagnostics::computed_property_names_not_allowed_in_enums(span));
                    TSEnumMemberName::Identifier(self.alloc(self.ast.identifier_name(span, "")))
                }
            },
            // surge: TS18024 — tsc keeps a private name and its checker rejects it; the member
            // keeps an empty placeholder name, as for TS1164.
            Kind::PrivateIdentifier => {
                let span = self.cur_token().span();
                self.bump_any();
                self.error(diagnostics::enum_member_private_name(span));
                TSEnumMemberName::Identifier(self.alloc(self.ast.identifier_name(span, "")))
            }
            Kind::NoSubstitutionTemplate | Kind::TemplateHead => {
                let error = diagnostics::computed_property_names_not_allowed_in_enums(
                    self.cur_token().span(),
                );
                self.fatal_error(error)
            }
            // surge: TS2452 — as for the computed form above.
            kind if kind.is_number()
                && !matches!(
                    kind,
                    Kind::DecimalBigInt | Kind::BinaryBigInt | Kind::OctalBigInt | Kind::HexBigInt
                ) =>
            {
                let literal = self.parse_literal_number();
                let text = self.ast.str(&literal.value.to_string());
                TSEnumMemberName::String(self.alloc(self.ast.string_literal(literal.span, text, None)))
            }
            kind if kind.is_number() => {
                let error =
                    diagnostics::enum_member_cannot_have_numeric_name(self.cur_token().span());
                self.fatal_error(error)
            }
            _ => {
                let ident_name = self.parse_identifier_name();
                TSEnumMemberName::Identifier(self.alloc(ident_name))
            }
        }
    }

    /* ------------------- Annotation ----------------- */

    pub(crate) fn parse_ts_type_annotation(&mut self) -> Option<Box<'a, TSTypeAnnotation<'a>>> {
        if !self.is_ts {
            return None;
        }
        if !self.at(Kind::Colon) {
            return None;
        }
        let span = self.start_span();
        self.bump_any(); // bump ':'
        let type_annotation = self.parse_ts_type();
        Some(self.ast.alloc_ts_type_annotation(self.end_span(span), type_annotation))
    }

    pub(crate) fn parse_ts_type_alias_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Declaration<'a> {
        self.expect(Kind::Type);

        let id = self.parse_binding_identifier();
        let params = self.parse_ts_type_parameters();
        self.expect(Kind::Eq);

        let intrinsic_token = self.cur_token();
        let ty = if self.at(Kind::Intrinsic) {
            self.bump_any();
            if self.at(Kind::Dot) {
                // `type something = intrinsic. ...`
                let left_name = self.ast.ts_type_name_identifier_reference(
                    intrinsic_token.span(),
                    self.token_source(&intrinsic_token),
                );
                let type_name =
                    self.parse_ts_qualified_type_name(intrinsic_token.start(), left_name);
                let type_parameters = self.parse_type_arguments_of_type_reference();
                self.ast.ts_type_type_reference(
                    self.end_span(intrinsic_token.start()),
                    type_name,
                    type_parameters,
                )
            } else {
                // `type something = intrinsic`
                self.ast.ts_type_intrinsic_keyword(intrinsic_token.span())
            }
        } else {
            // `type something = ...`
            self.parse_ts_type()
        };

        self.asi();
        let span = self.end_span(span);

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        self.ast.declaration_ts_type_alias(span, id, params, ty, modifiers.contains_declare())
    }

    /* ---------------------  Interface  ------------------------ */

    pub(crate) fn parse_ts_interface_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Declaration<'a> {
        let id = self.parse_binding_identifier();
        let type_parameters = self.parse_ts_type_parameters();
        let (extends, implements) = self.parse_heritage_clause();
        let body = self.parse_ts_interface_body();
        let extends = extends.unwrap_or_else(|| self.ast.vec());
        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );
        if let Some((implements_kw_span, _)) = implements {
            self.error(diagnostics::interface_implements(implements_kw_span));
        }
        for extend in &extends {
            if self.fatal_error.is_some() {
                break;
            }
            if !extend.expression.is_entity_name_expression() {
                self.error(diagnostics::interface_extend(extend.span));
            }
        }
        self.ast.declaration_ts_interface(
            self.end_span(span),
            id,
            type_parameters,
            extends,
            body,
            modifiers.contains_declare(),
        )
    }

    fn parse_ts_interface_body(&mut self) -> Box<'a, TSInterfaceBody<'a>> {
        let span = self.start_span();
        let body_list =
            self.parse_normal_list(Kind::LCurly, Kind::RCurly, Self::parse_ts_type_signature);
        self.ast.alloc_ts_interface_body(self.end_span(span), body_list)
    }

    pub(crate) fn parse_ts_type_signature(&mut self) -> TSSignature<'a> {
        let span = self.start_span();
        let kind = self.cur_kind();

        if matches!(kind, Kind::LParen | Kind::LAngle) {
            return self.parse_signature_member(CallOrConstructorSignature::Call);
        }

        if kind == Kind::New
            && matches!(self.lexer.peek_token().kind(), Kind::LParen | Kind::LAngle)
        {
            return self.parse_signature_member(CallOrConstructorSignature::Constructor);
        }

        let modifiers = self.parse_modifiers(
            /* permit_const_as_modifier */ true,
            /* stop_on_start_of_class_static_block */ false,
        );

        if self.is_index_signature() {
            self.verify_modifiers(
                &modifiers,
                ModifierKinds::new([ModifierKind::Readonly]),
                true,
                diagnostics::cannot_appear_on_an_index_signature,
            );
            return TSSignature::TSIndexSignature(
                self.parse_index_signature_declaration(span, &modifiers),
            );
        }

        self.verify_modifiers(
            &modifiers,
            ModifierKinds::new([ModifierKind::Readonly]),
            true,
            diagnostics::cannot_appear_on_a_type_member,
        );

        if self.parse_contextual_modifier(Kind::Get) {
            return self.parse_getter_setter_signature_member(span, TSMethodSignatureKind::Get);
        }

        if self.parse_contextual_modifier(Kind::Set) {
            return self.parse_getter_setter_signature_member(span, TSMethodSignatureKind::Set);
        }

        self.parse_property_or_method_signature(span, &modifiers)
    }

    pub(crate) fn is_index_signature(&mut self) -> bool {
        self.at(Kind::LBrack) && self.lookahead(Self::is_unambiguously_index_signature)
    }

    fn is_unambiguously_index_signature(&mut self) -> bool {
        self.bump_any();
        // surge: tsc's recovery set (`isUnambiguouslyIndexSignature`) has `[]`,
        // not `[[`, which opens a computed name such as `[[]]() {}`.
        if matches!(self.cur_kind(), Kind::Dot3 | Kind::RBrack) {
            return true;
        }
        if self.cur_kind().is_modifier_kind() {
            self.bump_any();
            if self.cur_kind().is_identifier() {
                return true;
            }
        } else if !self.cur_kind().is_identifier() {
            return false;
        } else {
            self.bump_any();
        }
        if matches!(self.cur_kind(), Kind::Colon | Kind::Comma) {
            return true;
        }
        if self.cur_kind() != Kind::Question {
            return false;
        }
        self.bump_any();
        matches!(self.cur_kind(), Kind::Colon | Kind::Comma | Kind::RBrack)
    }

    /* ----------------------- Namespace & Module ----------------------- */

    fn parse_ts_module_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Box<'a, TSModuleDeclaration<'a>> {
        let kind = if self.eat(Kind::Namespace) {
            TSModuleDeclarationKind::Namespace
        } else {
            self.expect(Kind::Module);
            if self.at(Kind::Str) {
                return self.parse_ambient_external_module_declaration(span, modifiers);
            }
            TSModuleDeclarationKind::Module
        };
        self.parse_module_or_namespace_declaration(span, kind, modifiers)
    }

    fn parse_ambient_external_module_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Box<'a, TSModuleDeclaration<'a>> {
        let id = TSModuleDeclarationName::StringLiteral(self.parse_literal_string());
        let body = if self.at(Kind::LCurly) {
            let block = self.parse_ts_module_block();
            Some(TSModuleDeclarationBody::TSModuleBlock(block))
        } else {
            self.asi();
            None
        };
        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );
        self.ast.alloc_ts_module_declaration(
            self.end_span(span),
            id,
            body,
            TSModuleDeclarationKind::Module,
            modifiers.contains_declare(),
        )
    }

    fn parse_ts_module_block(&mut self) -> Box<'a, TSModuleBlock<'a>> {
        let span = self.start_span();
        self.expect(Kind::LCurly);
        // Remove TopLevel context for module block
        let (directives, statements) =
            self.context_remove(Context::TopLevel, Self::parse_directives_and_statements);
        self.expect(Kind::RCurly);
        self.ast.alloc_ts_module_block(self.end_span(span), directives, statements)
    }

    fn parse_module_or_namespace_declaration(
        &mut self,
        span: u32,
        kind: TSModuleDeclarationKind,
        modifiers: &Modifiers,
    ) -> Box<'a, TSModuleDeclaration<'a>> {
        let id = TSModuleDeclarationName::Identifier(self.parse_binding_identifier());
        let body = if self.eat(Kind::Dot) {
            let span = self.start_span();
            let decl = self.parse_module_or_namespace_declaration(span, kind, &Modifiers::empty());
            TSModuleDeclarationBody::TSModuleDeclaration(decl)
        } else {
            let block = self.parse_ts_module_block();
            TSModuleDeclarationBody::TSModuleBlock(block)
        };
        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );
        self.ast.alloc_ts_module_declaration(
            self.end_span(span),
            id,
            Some(body),
            kind,
            modifiers.contains_declare(),
        )
    }

    fn parse_ts_global_declaration(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
    ) -> Box<'a, TSGlobalDeclaration<'a>> {
        let keyword_span_start = self.start_span();
        self.expect(Kind::Global);
        let keyword_span = self.end_span(keyword_span_start);

        let body = self.parse_ts_module_block().unbox();

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        self.ast.alloc_ts_global_declaration(
            self.end_span(span),
            keyword_span,
            body,
            modifiers.contains_declare(),
        )
    }

    /* ----------------------- declare --------------------- */

    pub(crate) fn parse_ts_declaration_statement(&mut self, start_span: u32) -> Statement<'a> {
        // surge: TS1275 — tsc parses `accessor` before an import or export as
        // a modifier and rejects it from its checker (`checkGrammarModifiers`);
        // the statement stands.
        if self.at(Kind::Accessor)
            && self.lookahead(|parser| {
                while parser.at(Kind::Accessor) {
                    parser.bump_any();
                }
                matches!(parser.cur_kind(), Kind::Import | Kind::Export)
            })
        {
            while self.at(Kind::Accessor) {
                let span = self.cur_token().span();
                self.bump_any();
                self.error(diagnostics::accessor_modifier_cannot_be_used_here(span));
            }
            return if self.at(Kind::Import) {
                self.parse_import_statement()
            } else {
                self.parse_export_declaration(self.start_span(), self.ast.vec())
            };
        }
        let reserved_ctx = self.ctx;
        let modifiers = self.eat_modifiers_before_declaration();
        self.ctx = self
            .ctx
            .union_ambient_if(modifiers.contains_declare())
            .and_await(modifiers.contains_async());
        let decl = self.parse_declaration(start_span, &modifiers, self.ast.vec());
        self.ctx = reserved_ctx;
        Statement::from(decl)
    }

    pub(crate) fn parse_declaration(
        &mut self,
        start_span: u32,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Declaration<'a> {
        let kind = self.cur_kind();
        if kind != Kind::Class {
            for decorator in &decorators {
                self.error(diagnostics::decorators_are_not_valid_here(decorator.span));
            }
        }
        match kind {
            Kind::Var | Kind::Let | Kind::Const => {
                let kind = self.get_variable_declaration_kind();
                self.bump_any();
                self.verify_modifiers(
                    modifiers,
                    ModifierKinds::new([ModifierKind::Declare]),
                    true,
                    diagnostics::modifier_cannot_be_used_here,
                );
                let decl = self.parse_variable_declaration(
                    start_span,
                    kind,
                    VariableDeclarationParent::Statement,
                    modifiers.contains_declare(),
                );
                Declaration::VariableDeclaration(decl)
            }
            // surge: TS1491/TS1495 — tsc parses the declaration and its checker rejects the
            // `export` or `declare` before it (`checkGrammarModifiers`).
            Kind::Using | Kind::Await
                if (kind == Kind::Using && self.is_using_declaration())
                    || (kind == Kind::Await && self.is_using_statement()) =>
            {
                let word_at = |at: u32| {
                    let len = self.source_text[at as usize..]
                        .bytes()
                        .take_while(u8::is_ascii_alphabetic)
                        .count() as u32;
                    Span::new(at, at + len)
                };
                let mut modifier_span = word_at(start_span);
                // `export` was consumed before this declaration's span began.
                if modifier_span.start == self.cur_token().span().start {
                    let before = self.source_text[..start_span as usize].trim_end();
                    let start = before
                        .bytes()
                        .rposition(|byte| !byte.is_ascii_alphabetic())
                        .map_or(0, |at| at + 1) as u32;
                    modifier_span = word_at(start);
                }
                let modifier =
                    &self.source_text[modifier_span.start as usize..modifier_span.end as usize];
                let await_using = kind == Kind::Await;
                self.error(diagnostics::modifier_on_using_declaration(
                    modifier,
                    await_using,
                    modifier_span,
                ));
                if await_using {
                    self.bump_any();
                }
                self.bump_any();
                let kind = if await_using {
                    VariableDeclarationKind::AwaitUsing
                } else {
                    VariableDeclarationKind::Using
                };
                let decl = self.parse_variable_declaration(
                    start_span,
                    kind,
                    VariableDeclarationParent::Statement,
                    modifiers.contains_declare(),
                );
                Declaration::VariableDeclaration(decl)
            }
            Kind::Class => {
                let decl = self.parse_class_declaration(start_span, modifiers, decorators);
                Declaration::ClassDeclaration(decl)
            }
            Kind::Import => {
                self.bump_any();
                let token = self.cur_token();
                let mut import_kind = ImportOrExportKind::Value;
                let mut identifier = self.parse_binding_identifier();
                if self.is_ts
                    && token.kind() == Kind::Type
                    && self.cur_kind().is_binding_identifier()
                {
                    // `import type something ...`
                    identifier = self.parse_binding_identifier();
                    import_kind = ImportOrExportKind::Type;
                }
                self.parse_ts_import_equals_declaration(import_kind, identifier, start_span)
            }
            Kind::Module | Kind::Namespace if self.is_ts => {
                let decl = self.parse_ts_module_declaration(start_span, modifiers);
                Declaration::TSModuleDeclaration(decl)
            }
            Kind::Global if self.is_ts => {
                let decl = self.parse_ts_global_declaration(start_span, modifiers);
                Declaration::TSGlobalDeclaration(decl)
            }
            Kind::Type if self.is_ts => self.parse_ts_type_alias_declaration(start_span, modifiers),
            Kind::Enum if self.is_ts => self.parse_ts_enum_declaration(start_span, modifiers),
            Kind::Interface if self.is_ts => {
                self.bump_any();
                self.parse_ts_interface_declaration(start_span, modifiers)
            }
            _ if self.at_function_with_async() => {
                let declare = modifiers.contains(ModifierKind::Declare);
                if declare {
                    let decl = self.parse_ts_declare_function(start_span, modifiers);
                    Declaration::FunctionDeclaration(decl)
                } else if self.is_ts {
                    let decl = self.parse_ts_function_impl(
                        start_span,
                        FunctionKind::Declaration,
                        modifiers,
                    );
                    Declaration::FunctionDeclaration(decl)
                } else {
                    let span = self.start_span();
                    let r#async = self.eat(Kind::Async);
                    let decl = self.parse_function_impl(span, r#async, FunctionKind::Declaration);
                    Declaration::FunctionDeclaration(decl)
                }
            }
            _ => self.unexpected(),
        }
    }

    pub(crate) fn parse_ts_declare_function(
        &mut self,
        start_span: u32,
        modifiers: &Modifiers,
    ) -> Box<'a, Function<'a>> {
        let r#async = modifiers.contains(ModifierKind::Async);
        self.expect(Kind::Function);
        let func_kind = FunctionKind::TSDeclaration;
        // surge: TS1221 — tsc parses `declare function*`; surge's grammar pass reports the `*`.
        let generator = self.eat(Kind::Star);
        let id = self.parse_function_id(func_kind, r#async, generator);
        self.parse_function(
            start_span,
            id,
            r#async,
            generator,
            func_kind,
            FormalParameterKind::FormalParameter,
            modifiers,
        )
    }

    pub(crate) fn parse_ts_type_assertion(&mut self) -> Expression<'a> {
        let span = self.start_span();
        self.expect(Kind::LAngle);
        let type_annotation = self.parse_ts_type();
        self.expect(Kind::RAngle);
        let lhs_span = self.start_span();
        let expression = self.parse_simple_unary_expression(lhs_span);
        let span = self.end_span(span);

        if matches!(self.source_type.extension(), Some(FileExtension::Mts | FileExtension::Cts)) {
            self.error(diagnostics::jsx_type_assertion_in_mts_cts(span));
        }

        self.ast.expression_ts_type_assertion(span, type_annotation, expression)
    }

    pub(crate) fn parse_ts_import_equals_declaration(
        &mut self,
        import_kind: ImportOrExportKind,
        identifier: BindingIdentifier<'a>,
        span: u32,
    ) -> Declaration<'a> {
        self.expect(Kind::Eq);

        let reference_span = self.start_span();
        let module_reference = if self.eat(Kind::Require) {
            self.expect(Kind::LParen);
            let expression = if self.at(Kind::Str) {
                self.parse_literal_string()
            } else {
                // surge: TS1141 — tsc parses any expression as the module
                // specifier (`parseModuleSpecifier`) and its checker requires a
                // string literal (`checkExternalImportOrExportDeclaration`).
                let mut span = self.parse_expr().span();
                if span.end <= span.start {
                    span = self.cur_token().span();
                }
                self.error(diagnostics::ts_string_literal_expected(span));
                self.ast.string_literal(span, "", None)
            };
            self.expect(Kind::RParen);
            self.ast.ts_module_reference_external_module_reference(
                self.end_span(reference_span),
                expression,
            )
        } else {
            self.parse_ts_module_reference(reference_span)
        };

        self.asi();

        let span = self.end_span(span);

        if !self.is_ts {
            self.error(diagnostics::import_equals_can_only_be_used_in_typescript_files(span));
        }

        self.ast.declaration_ts_import_equals(span, identifier, module_reference, import_kind)
    }

    /// Parse `TSModuleReference` for `import x = foo` or `import x = foo.bar`.
    ///
    /// Unlike `parse_ts_type_name`, this does not allow `this` as the identifier.
    fn parse_ts_module_reference(&mut self, span: u32) -> TSModuleReference<'a> {
        // Check for invalid `this` keyword
        if self.at(Kind::This) {
            let this_span = self.cur_token().span();
            self.error(diagnostics::identifier_reserved_word(this_span, "this"));
            self.bump_any();
            // Recover by creating a dummy identifier
            let ident = self.ast.alloc_identifier_reference(this_span, "this");
            return TSModuleReference::IdentifierReference(ident);
        }

        let ident = self.parse_identifier_name();
        let left = self.ast.ts_type_name_identifier_reference(ident.span, ident.name);

        // Parse qualified name: foo.bar.baz
        let type_name =
            if self.at(Kind::Dot) { self.parse_ts_qualified_type_name(span, left) } else { left };

        // Convert TSTypeName to TSModuleReference
        match type_name {
            TSTypeName::IdentifierReference(ident) => TSModuleReference::IdentifierReference(ident),
            TSTypeName::QualifiedName(qualified) => TSModuleReference::QualifiedName(qualified),
            TSTypeName::ThisExpression(_) => {
                // This shouldn't happen since we check for `this` above,
                // but handle it for completeness
                unreachable!("ThisExpression should have been caught earlier")
            }
        }
    }

    pub(crate) fn parse_ts_this_parameter(&mut self) -> TSThisParameter<'a> {
        let span = self.start_span();
        self.bump_any();
        let this_span = self.end_span(span);

        let type_annotation = self.parse_ts_type_annotation();
        self.ast.ts_this_parameter(self.end_span(span), this_span, type_annotation)
    }

    pub(crate) fn at_start_of_ts_declaration(&mut self) -> bool {
        self.lookahead(Self::at_start_of_ts_declaration_worker)
    }

    /// Check if the parser is at a start of a ts declaration
    fn at_start_of_ts_declaration_worker(&mut self) -> bool {
        loop {
            match self.cur_kind() {
                Kind::Var | Kind::Let | Kind::Const | Kind::Function | Kind::Class | Kind::Enum => {
                    return true;
                }
                Kind::Interface | Kind::Type => {
                    self.bump_any();
                    return self.cur_kind().is_binding_identifier()
                        && !self.cur_token().is_on_new_line();
                }
                Kind::Module | Kind::Namespace => {
                    self.bump_any();
                    return !self.cur_token().is_on_new_line()
                        && (self.cur_kind().is_binding_identifier()
                            || self.cur_kind() == Kind::Str);
                }
                Kind::Abstract
                | Kind::Accessor
                | Kind::Async
                | Kind::Declare
                | Kind::Private
                | Kind::Protected
                | Kind::Public
                | Kind::Readonly => {
                    self.bump_any();
                    if self.cur_token().is_on_new_line() {
                        return false;
                    }
                }
                // surge: TS1491/TS1495 — tsc parses `declare using x` as the declaration
                // the modifier is then rejected on.
                Kind::Using => {
                    self.bump_any();
                    return self.cur_kind().is_binding_identifier()
                        && !self.cur_token().is_on_new_line();
                }
                Kind::Await => {
                    self.bump_any();
                    if !self.at(Kind::Using) || self.cur_token().is_on_new_line() {
                        return false;
                    }
                    self.bump_any();
                    return self.cur_kind().is_binding_identifier()
                        && !self.cur_token().is_on_new_line();
                }
                Kind::Global => {
                    self.bump_any();
                    return matches!(self.cur_kind(), Kind::Ident | Kind::LCurly | Kind::Export);
                }
                Kind::Import => {
                    self.bump_any();
                    let kind = self.cur_kind();
                    return matches!(kind, Kind::Str | Kind::Star | Kind::LCurly)
                        || kind.is_identifier();
                }
                Kind::Export => {
                    self.bump_any();
                    self.bump(Kind::Type); // optional `type` after `export`
                    // This allows constructs like
                    // `export *`, `export default`, `export {}`, `export = {}` along with all
                    // export [declaration]
                    if matches!(
                        self.cur_kind(),
                        Kind::Eq | Kind::Star | Kind::Default | Kind::LCurly | Kind::At | Kind::As
                    ) {
                        return true;
                    }
                    // falls through to check next token
                }
                Kind::Static => {
                    self.bump_any();
                }
                _ => {
                    return false;
                }
            }
        }
    }
}
