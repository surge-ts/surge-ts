use std::sync::Arc;

use surge_ts_syntax::{ParsedNamedType, ParsedType};
use surge_ts_types::{FunctionType, ObjectProperty, Type};

use super::index::{FileSemanticIndex, record_file};
use super::retained::{RetainedProgram, RetainedState};
use crate::infer::TypeParameterSubstitution;

/// A property as a type exposes it.
#[derive(Debug, Clone)]
pub struct PropertyInfo {
    pub name: Arc<str>,
    pub ty: Type,
    pub optional: bool,
    pub readonly: bool,
    pub method: bool,
    /// Where an object literal declares the member.
    pub declaration: Option<Arc<surge_ts_types::PropertyDeclarationSite>>,
}

/// A type as a read of it sees it: a function's body-inferred return, an
/// inferred member or a lazily typed variable is the type it resolves to,
/// never the lazy reference standing in for it, as tsc's
/// `getReturnTypeOfSignature` only hands back a settled type.
pub fn settled_read(ty: Type) -> Type {
    crate::checks::function::settle_lazy_read(ty)
}

/// A retained program's queries, valid while [`RetainedProgram::query`] runs.
pub struct Query<'a> {
    program: &'a RetainedProgram,
    state: &'a mut RetainedState,
}

/// The global interface tsc's `getApparentType` gives a primitive.
fn apparent_interface_name(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::String | Type::StringLiteral(_) => Some("String"),
        Type::Number | Type::NumberLiteral(_) => Some("Number"),
        Type::Boolean | Type::BooleanLiteral(_) => Some("Boolean"),
        Type::BigInt => Some("BigInt"),
        Type::Symbol => Some("Symbol"),
        _ => None,
    }
}

impl<'a> Query<'a> {
    pub(crate) fn new(program: &'a RetainedProgram, state: &'a mut RetainedState) -> Self {
        Self { program, state }
    }

    /// The types checking `file_index` computed, recorded the first time a
    /// query asks. A declaration file's is empty: the checker never checks
    /// declaration bodies.
    pub fn semantic_index(&mut self, file_index: usize) -> Arc<FileSemanticIndex> {
        if let Some(index) = self.state.indexes.get(&file_index) {
            return index.clone();
        }
        let RetainedState { run, query_ctx, indexes, .. } = &mut *self.state;
        let index = match run.parsed_files.get(file_index) {
            Some(parsed_file) if !parsed_file.statements.is_empty() => {
                let file_name = parsed_file.file_name.clone();
                record_file(&file_name, || {
                    let _ = crate::program::check_program_file(
                        file_index,
                        parsed_file,
                        &run.shared_state,
                        query_ctx,
                        None,
                    );
                })
            }
            _ => FileSemanticIndex::default(),
        };
        query_ctx.diagnostics.clear();
        let index = Arc::new(index);
        indexes.insert(file_index, index.clone());
        index
    }

    /// tsc's `typeToString` as the checker renders types in its diagnostics.
    pub fn type_to_string(&self, ty: &Type) -> String {
        ty.name()
    }

    /// Under node16/nodenext resolution, whether `file_name`'s implied
    /// format is ESM (tsc's `impliedNodeFormat`).
    pub fn is_esm_under_node_resolution(&self, file_name: &str) -> bool {
        let options = &self.state.query_ctx.options;
        options.node_module_resolution && options.esm_module_files.contains(file_name)
    }

    /// `ty` as tsc shows it where the checker's own type erases a generic
    /// signature's type parameters: it resolves a member's signature with
    /// them standing for the sentinel and keeps the written one to
    /// instantiate per call. Resolved again with each standing for itself,
    /// under the bindings of the declaration it was read off, the signature
    /// is the one tsc prints. `None` when nothing changes.
    pub fn shown_type(&mut self, ty: &Type) -> Option<Type> {
        let Type::Function(function) = ty else { return None };
        let mut members = Vec::new();
        function.push_overload_members(&mut members);
        let mut changed = false;
        let shown: Vec<FunctionType> = members
            .into_iter()
            .map(|member| match self.shown_generic_signature(&member) {
                Some(shown) => {
                    changed = true;
                    shown
                }
                None => member,
            })
            .collect();
        if !changed {
            return None;
        }
        let first = shown.first()?.clone();
        Some(Type::Function(if shown.len() > 1 { first.with_overloads(shown) } else { first }))
    }

    fn shown_generic_signature(&mut self, function: &FunctionType) -> Option<FunctionType> {
        let declaration = function.declaration()?;
        let (signature, outer) =
            if let Some(member) = declaration.downcast_ref::<crate::checks::call::DeclaredMemberSignature>() {
                (member.signature.clone(), member.outer_type_arguments.clone())
            } else {
                (Arc::new(declaration.downcast_ref::<crate::symbols::FunctionSignatureInfo>()?.clone()), Vec::new())
            };
        if signature.type_parameters.is_empty()
            || signature.overloaded
            || signature.parameter_types.len() != function.parameters().len()
        {
            return None;
        }
        let file_index = signature
            .declaring_file
            .as_deref()
            .and_then(|file| self.program.file_names().iter().position(|name| name == file));
        match file_index {
            Some(file_index) => self.install_file_scope(file_index)?,
            None => {
                let RetainedState { run, query_ctx, .. } = &mut *self.state;
                query_ctx.begin_file_check(String::new());
                query_ctx.type_declarations = run.shared_state.script_type_declarations.clone();
                query_ctx.type_declaration_scope = None;
            }
        }
        let mut substitution = TypeParameterSubstitution::new();
        for (name, ty) in &outer {
            substitution.insert(name.clone(), ty.clone());
        }
        let mut bound = substitution.clone();
        for parameter in &signature.type_parameters {
            substitution.set(parameter.name.clone(), Type::type_parameter(&parameter.name), true);
        }
        let query_ctx = &mut self.state.query_ctx;
        let mut instantiated =
            crate::checks::call::instantiate_function_type_with_substitution(function, &signature, &substitution, query_ctx)
                .into_owned();
        // An unbound name merges out of an intersection as unmodelled, leaving
        // it open over nothing; bound as the body binds them, the parameters
        // stay its operands (`T & { … }`).
        let variables = holds_degraded_intersection(&Type::Function(instantiated.clone()), 4)
            .then(|| crate::checks::function::enter_body_type_variables(&signature.type_parameters, query_ctx))
            .flatten();
        if variables.is_some() {
            for parameter in &signature.type_parameters {
                let variable = parameter.name_span.and_then(|span| {
                    surge_ts_types::type_variable::variable_for_declaration(&query_ctx.file_name, span.start as u32)
                });
                bound.set(parameter.name.clone(), variable.unwrap_or_else(|| Type::type_parameter(&parameter.name)), true);
            }
            let rebound =
                crate::checks::call::instantiate_function_type_with_substitution(function, &signature, &bound, query_ctx)
                    .into_owned();
            if !holds_degraded_intersection(&Type::Function(rebound.clone()), 4) {
                instantiated = rebound;
                substitution = bound;
            }
        }
        // A list with nothing to instantiate keeps the head it was rendered
        // with (the lib's `filter` is resolved from a copy whose constraints
        // were taken off and rendered separately).
        let instantiates_head = signature
            .type_parameters
            .iter()
            .any(|parameter| parameter.constraint.is_some() || parameter.default_type.is_some() || parameter.is_const);
        let mut head = Vec::with_capacity(signature.type_parameters.len());
        for parameter in signature.type_parameters.iter().filter(|_| instantiates_head) {
            // `typeParameterToDeclaration` writes the list's modifiers.
            let mut text = String::new();
            for (written, keyword) in [(parameter.is_const, "const "), (parameter.is_in, "in "), (parameter.is_out, "out ")] {
                if written {
                    text.push_str(keyword);
                }
            }
            text.push_str(&parameter.name);
            for (keyword, written) in [(" extends ", &parameter.constraint), (" = ", &parameter.default_type)] {
                let Some(written) = written else { continue };
                let resolved = crate::infer::map_parsed_type_with_substitution(written.clone(), query_ctx, &substitution);
                if matches!(resolved, Type::Unknown) {
                    query_ctx.diagnostics.clear();
                    return None;
                }
                text.push_str(keyword);
                text.push_str(&resolved.name());
            }
            head.push(text);
        }
        // A written predicate's type, which the checker erased with the rest.
        let predicate = match signature.return_type.as_ref() {
            Some(ParsedType::Predicate(written)) => written.ty.as_ref().and_then(|ty| {
                use surge_ts_types::TypePredicateKind as Kind;
                let this = written.parameter_name == "this";
                let parameter_index = (!this)
                    .then(|| signature.parameter_names.iter().position(|name| name.as_deref() == Some(written.parameter_name.as_str())))
                    .flatten();
                if !this && parameter_index.is_none() {
                    return None;
                }
                let resolved = crate::infer::map_parsed_type_with_substitution(ty.clone(), query_ctx, &substitution);
                (!matches!(resolved, Type::Unknown)).then(|| surge_ts_types::TypePredicate {
                    kind: match (this, written.asserts) {
                        (true, false) => Kind::This,
                        (true, true) => Kind::AssertsThis,
                        (false, false) => Kind::Identifier,
                        (false, true) => Kind::AssertsIdentifier,
                    },
                    parameter_index,
                    ty: Some(resolved),
                })
            }),
            _ => None,
        };
        query_ctx.diagnostics.clear();
        // A position the written signature cannot resolve again keeps the
        // checker's.
        let pick = |shown: &Type, own: &Type| if matches!(shown, Type::Unknown) { own.clone() } else { shown.clone() };
        let parameters = instantiated
            .parameters()
            .iter()
            .zip(function.parameters())
            .map(|(shown, own)| pick(shown, own))
            .collect();
        let mut return_type = match signature.return_type {
            Some(ParsedType::Predicate(_)) => function.return_type().clone(),
            _ => pick(instantiated.return_type(), function.return_type()),
        };
        // A return read from the body per call is the one its own check
        // recorded over its type variables, where nothing outer is bound.
        if (matches!(return_type, Type::Unknown) || crate::checks::function::is_body_inferred_return(&return_type))
            && outer.is_empty()
            && let Some(file_index) = file_index
        {
            let index = self.semantic_index(file_index);
            let recorded = match (signature.body_return.as_ref(), signature.return_type.as_ref()) {
                (Some(source), _) => {
                    source.function.name_span.and_then(|span| index.declaration_type(span.start, span.end))
                }
                (None, Some(ParsedType::InferredMember(member)))
                    if matches!(member.source, surge_ts_syntax::ParsedInferredMemberSource::MethodBody(_)) =>
                {
                    index.member_declaration_type(member.member_start, member.member_name.len())
                }
                _ => None,
            };
            if let Some(Type::Function(recorded)) = recorded
                && recorded.type_parameter_head() == function.type_parameter_head()
                && !matches!(recorded.return_type(), Type::Unknown)
            {
                return_type = recorded.return_type().clone();
            }
        }
        let mut shown = function.with_signature_types(parameters, return_type);
        if predicate.is_some() {
            shown = shown.with_type_predicate(predicate);
        }
        Some(if instantiates_head { shown.with_type_parameter_head(Some(head.join(", "))) } else { shown })
    }

    /// tsc's `isTypeAssignableTo`: the assignable relation.
    pub fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        surge_ts_types::is_assignable_to(source, target)
    }

    /// The structural type behind any lazy reference.
    pub fn peel(&self, ty: &Type) -> Type {
        ty.peeled()
    }

    /// `getIntersectionType` of `members`.
    pub fn intersection_type(&self, members: Vec<Type>) -> Type {
        crate::infer::types::merge_intersection_members(members)
    }

    /// tsc's `!operand`: `false` or `true` when the operand is definitely
    /// truthy or falsy, else `boolean`.
    pub fn logical_not_type(&self, operand: &Type) -> Type {
        crate::infer::expression::logical_not_result_type(operand)
    }

    /// tsc's `checkBinaryLikeExpression` of `left || right` (`or`) or
    /// `left && right`.
    pub fn logical_type(&self, or: bool, left: &Type, right: &Type) -> Option<Type> {
        use crate::infer::InferredExpression;
        let operator = if or { surge_ts_syntax::ParsedLogicalOperator::Or } else { surge_ts_syntax::ParsedLogicalOperator::And };
        match crate::checks::ops::evaluate_logical_expression(
            operator,
            InferredExpression::Known(left.clone()),
            InferredExpression::Known(right.clone()),
        ) {
            InferredExpression::Known(ty) => Some(ty),
            _ => None,
        }
    }

    /// tsc's `checkBinaryLikeExpression` of `left ?? right`.
    pub fn nullish_coalescing_type(&self, left: &Type, right: &Type) -> Type {
        crate::infer::expression::nullish_coalescing_result(left.clone(), right.clone())
    }

    /// tsc's `getWidenedLiteralType`: a literal as its primitive.
    pub fn widened_literal(&self, ty: &Type) -> Type {
        crate::checks::expr::widen_type(ty)
    }

    /// tsc's `getAwaitedType`: what `await` of a value of `ty` yields.
    pub fn awaited_type(&self, ty: &Type) -> Type {
        crate::checks::call::awaited_type(ty)
    }

    /// tsc's `typeofType`, what a `typeof` expression yields.
    pub fn typeof_type(&self) -> Type {
        crate::infer::expression::typeof_result_type()
    }

    /// Installs the scope `file_index`'s statements were checked in on the
    /// query context: the file's own when it was checked, else the scope its
    /// declarations bind in.
    fn install_file_scope(&mut self, file_index: usize) -> Option<()> {
        let index = self.semantic_index(file_index);
        let file_name = self.program.file_names().get(file_index)?.clone();
        let RetainedState { run, query_ctx, .. } = &mut *self.state;
        query_ctx.begin_file_check(file_name);
        match &index.scope {
            Some(scope) => {
                query_ctx.type_declarations = scope.type_declarations.clone();
                query_ctx.type_declaration_scope = scope.type_declaration_scope.clone();
                query_ctx.set_symbols(scope.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext));
            }
            None => {
                query_ctx.type_declarations = run.shared_state.script_type_declarations.clone();
                query_ctx.type_declaration_scope = None;
                // A package's script declaration file has a module analysis too
                // (binding analyzes every dependency declaration), but its
                // declarations bind in the global scope: its own table alone
                // would resolve a global interface it reopens (`String`) from
                // this file's members only.
                let is_module = run.parsed_files.get(file_index).is_some_and(|file| file.is_module);
                if is_module {
                    query_ctx.type_declaration_scope =
                        run.shared_state.module_resolution_scopes.get(file_index).cloned().flatten();
                    if let Some(analysis) = run.shared_state.module_analyses.get(file_index).and_then(Option::as_ref) {
                        query_ctx.type_declarations = analysis.local_type_declarations().as_ref().clone();
                    }
                }
            }
        }
        Some(())
    }

    fn resolve_in_file_scope(&mut self, ty: ParsedType, substitution: &TypeParameterSubstitution) -> Type {
        let query_ctx = &mut self.state.query_ctx;
        let resolved = crate::infer::map_parsed_type_with_substitution(ty, query_ctx, substitution);
        query_ctx.diagnostics.clear();
        resolved
    }

    /// Resolves type syntax written in `file_index` (a declaration's
    /// annotation, as its source spells it) the way the checker resolves an
    /// annotation there: parsed by the checker's parser, resolved in the
    /// file's scope, with `type_parameters` (those of the declarations it is
    /// written inside) standing for themselves, and bare names inside
    /// `namespaces` (the dotted paths of the namespaces it is written in,
    /// outermost first) reaching their members as the namespace body's do.
    /// `None` when the text does not parse as a type.
    pub fn resolve_type_text(
        &mut self,
        file_index: usize,
        text: &str,
        type_parameters: &[String],
        namespaces: &[String],
    ) -> Option<Type> {
        self.resolve_type_text_in(file_index, text, type_parameters, namespaces, None, &[])
    }

    /// [`Self::resolve_type_text`] inside the function body that declares the
    /// local type named at `local_declaration` (its name's byte span), where
    /// that body's local types are in scope.
    pub fn resolve_local_type_text(
        &mut self,
        file_index: usize,
        text: &str,
        type_parameters: &[String],
        local_declaration: (usize, usize),
    ) -> Option<Type> {
        self.resolve_type_text_in(file_index, text, type_parameters, &[], Some(local_declaration), &[])
    }

    /// [`Self::resolve_type_text`] with some type parameters in scope bound
    /// to types (an alias's to the arguments a reference instantiated it
    /// with), optionally inside a function body as for
    /// [`Self::resolve_local_type_text`]. With `for_constraint`, a conditional
    /// the text writes is resolved as tsc's
    /// `getConstraintOfDistributiveConditionalType` instantiates one.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_bound_type_text(
        &mut self,
        file_index: usize,
        text: &str,
        type_parameters: &[String],
        namespaces: &[String],
        local_declaration: Option<(usize, usize)>,
        bindings: &[(String, Type)],
        for_constraint: bool,
    ) -> Option<Type> {
        if for_constraint {
            crate::infer::types::resolving_for_constraint(|| {
                self.resolve_type_text_in(file_index, text, type_parameters, namespaces, local_declaration, bindings)
            })
        } else {
            self.resolve_type_text_in(file_index, text, type_parameters, namespaces, local_declaration, bindings)
        }
    }

    fn resolve_type_text_in(
        &mut self,
        file_index: usize,
        text: &str,
        type_parameters: &[String],
        namespaces: &[String],
        local_declaration: Option<(usize, usize)>,
        bindings: &[(String, Type)],
    ) -> Option<Type> {
        let source = format!("type __surge_api_query__ = {text};");
        let parsed = surge_ts_syntax::parse_source(&source, "__surge_api_query__.ts");
        if !parsed.parser_errors.is_empty() || parsed.parse_aborted {
            return None;
        }
        let ty = parsed.statements.into_iter().find_map(|statement| match statement {
            surge_ts_syntax::ParsedStatement::TypeAliasDeclaration(alias) => Some(alias.ty),
            _ => None,
        })?;
        self.install_file_scope(file_index)?;
        if let Some((start, end)) = local_declaration {
            let scope = self.semantic_index(file_index).local_type_scope(start, end)?.clone();
            self.state.query_ctx.type_declaration_scope = Some(scope);
        }
        let mut substitution = TypeParameterSubstitution::new();
        for name in type_parameters {
            substitution.set(name.clone(), Type::type_parameter(name), true);
        }
        for (name, ty) in bindings {
            substitution.set(name.clone(), ty.clone(), false);
        }
        let saved = std::mem::replace(&mut self.state.query_ctx.namespace_member_prefix_stack, namespaces.to_vec());
        let resolved = self.resolve_in_file_scope(ty, &substitution);
        self.state.query_ctx.namespace_member_prefix_stack = saved;
        Some(resolved)
    }

    /// A member a `noLib` program's own `Array<element>` declares or inherits
    /// (`ArrayLike`'s `size()` in roblox-ts's lib) for an array or tuple
    /// `receiver`, which surge's array model lacks. `None` outside `noLib`, or
    /// for a member the lib lacks too.
    pub fn no_lib_array_member(&mut self, file_index: usize, receiver: &Type, name: &str) -> Option<Type> {
        let element = crate::infer::types::no_lib_array_element(receiver)?;
        self.install_file_scope(file_index)?;
        if !self.state.query_ctx.options.no_lib {
            return None;
        }
        crate::infer::types::no_lib_array_member_type(&element, name, &mut self.state.query_ctx)
    }

    /// The type of a global value (a script's top-level value, a `declare
    /// global` member, or a default library's).
    pub fn global_value_type(&self, name: &str) -> Option<Type> {
        let state = &*self.state;
        state
            .run
            .ctx
            .ambient_global_symbols
            .get(name)
            .or_else(|| state.run.shared_state.global_symbols.get(name))
            .map(|symbol| symbol.ty.clone())
    }

    /// The type of `name` among the values `file_index`'s statements were
    /// checked with: its own and those it imports.
    pub fn file_value_type(&mut self, file_index: usize, name: &str) -> Option<Type> {
        let index = self.semantic_index(file_index);
        index.scope.as_ref()?.symbols.get(name).map(|symbol| symbol.ty.clone())
    }

    /// The type of `name` among a module file's own top-level values.
    pub fn module_value_type(&self, file_index: usize, name: &str) -> Option<Type> {
        let analysis = self.state.run.shared_state.module_analyses.get(file_index)?.as_ref()?;
        analysis
            .local_symbols()
            .get(name)
            .or_else(|| analysis.local_export_table().symbols.get(name))
            .map(|symbol| symbol.ty.clone())
    }

    /// The type of a module's export named `name` (`"default"` for its default
    /// export), as the module itself declares it.
    pub fn module_export_type(&self, file_index: usize, name: &str) -> Option<Type> {
        let analysis = self.state.run.shared_state.module_analyses.get(file_index)?.as_ref()?;
        let exports = analysis.local_export_table();
        if name == "default"
            && let Some(symbol) = &exports.default_symbol
        {
            return Some(symbol.ty.clone());
        }
        exports.symbols.get(name).map(|symbol| symbol.ty.clone())
    }

    /// The value a module assigns as a whole (`export =`, a JavaScript
    /// module's `module.exports =`).
    pub fn module_export_assignment_type(&self, file_index: usize) -> Option<Type> {
        let analysis = self.state.run.shared_state.module_analyses.get(file_index)?.as_ref()?;
        analysis.local_export_table().export_assignment_symbol.as_ref().map(|symbol| symbol.ty.clone())
    }

    /// The file `specifier` imported from `importer_index` resolved to, as
    /// the checker resolved it: relative specifiers among the program's
    /// files, the rest through the loader's resolution.
    pub fn resolved_module(&mut self, importer_index: usize, specifier: &str) -> Option<String> {
        let run = &self.state.run;
        let importer = &run.parsed_files.get(importer_index)?.file_name;
        if crate::modules::is_relative_specifier(specifier) {
            let resolution = crate::modules::resolve_relative_module(
                importer,
                specifier,
                &run.parsed_files,
                &run.ctx.module_file_index_by_identity,
            )?;
            return Some(run.parsed_files.get(resolution.resolved_file_index)?.file_name.clone());
        }
        crate::modules::resolved_module_in_mode(&run.ctx, importer, specifier, None)
            .filter(|resolved| !resolved.is_empty())
            .cloned()
    }

    /// The type of a module's namespace object (`typeof import("m")`), as a
    /// namespace import of it would see it.
    pub fn module_namespace_type(&self, file_index: usize) -> Option<Type> {
        let analysis = self.state.run.shared_state.module_analyses.get(file_index)?.as_ref()?;
        Some(crate::modules::namespace_export_object_type(analysis.local_export_table()))
    }

    /// tsc's `getApparentType`: a primitive's global interface; any other
    /// type as it is.
    pub fn apparent_type(&mut self, ty: &Type) -> Type {
        let peeled = ty.peeled();
        if let Some(name) = apparent_interface_name(&peeled)
            && let Some(apparent) = self.resolve_global_type(name)
        {
            return apparent;
        }
        peeled
    }

    fn resolve_global_type(&mut self, name: &'static str) -> Option<Type> {
        if let Some(ty) = self.state.apparent_types.get(name) {
            return Some(ty.clone());
        }
        let RetainedState { run, query_ctx, apparent_types, .. } = &mut *self.state;
        query_ctx.begin_file_check(String::new());
        query_ctx.type_declarations = run.shared_state.script_type_declarations.clone();
        query_ctx.type_declaration_scope = None;
        query_ctx.lookup_type_declaration(name)?;
        let named = ParsedType::Named(Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.to_string(),
            span: None,
            type_arguments: Vec::new(),
        }));
        let resolved = crate::infer::map_parsed_type(named, query_ctx);
        query_ctx.diagnostics.clear();
        apparent_types.insert(name, resolved.clone());
        Some(resolved)
    }

    /// tsc's `getPropertiesOfType`: an object's members, a primitive's
    /// apparent members, and the members every constituent of a union has.
    pub fn properties(&mut self, ty: &Type) -> Vec<PropertyInfo> {
        let peeled = ty.peeled();
        match &peeled {
            Type::Object(object) => object
                .properties
                .iter()
                .map(|(name, property)| property_info(name.clone(), property))
                .collect(),
            Type::Union(union) => {
                let members: Vec<Vec<PropertyInfo>> =
                    union.types().iter().map(|member| self.properties(member)).collect();
                let Some((first, rest)) = members.split_first() else { return Vec::new() };
                first
                    .iter()
                    .filter_map(|property| {
                        let mut types = vec![property.ty.clone()];
                        let mut optional = property.optional;
                        let mut readonly = property.readonly;
                        for other in rest {
                            let found = other.iter().find(|candidate| candidate.name == property.name)?;
                            types.push(found.ty.clone());
                            optional |= found.optional;
                            readonly |= found.readonly;
                        }
                        Some(PropertyInfo {
                            name: property.name.clone(),
                            ty: surge_ts_types::union_type(types),
                            optional,
                            readonly,
                            method: false,
                            declaration: None,
                        })
                    })
                    .collect()
            }
            Type::Array(element) => array_properties(element),
            Type::Tuple(elements) => {
                let mut properties: Vec<PropertyInfo> = elements
                    .iter()
                    .enumerate()
                    .map(|(position, element)| PropertyInfo {
                        name: Arc::from(position.to_string()),
                        ty: element.clone(),
                        optional: false,
                        readonly: false,
                        method: false,
                        declaration: None,
                    })
                    .collect();
                properties.extend(array_properties(&surge_ts_types::tuple_element_union(elements)));
                properties
            }
            _ => match apparent_interface_name(&peeled) {
                Some(name) => match self.resolve_global_type(name) {
                    Some(apparent) if !matches!(apparent.peeled(), Type::String | Type::Number) => {
                        self.properties(&apparent)
                    }
                    _ => Vec::new(),
                },
                None => Vec::new(),
            },
        }
    }

    pub fn property(&mut self, ty: &Type, name: &str) -> Option<PropertyInfo> {
        let property = self.properties(ty).into_iter().find(|property| &*property.name == name);
        // An array's or tuple's method is the lib `Array<T>`'s, which surge's
        // own array table only approximates.
        let receiver = match ty.peeled() {
            receiver @ Type::Array(_) => receiver,
            Type::Tuple(elements) if name.parse::<usize>().is_err() => Type::Tuple(elements),
            _ => return property,
        };
        let RetainedState { run, query_ctx, .. } = &mut *self.state;
        query_ctx.begin_file_check(String::new());
        query_ctx.type_declarations = run.shared_state.script_type_declarations.clone();
        query_ctx.type_declaration_scope = None;
        let mut lib = crate::infer::expression::lib_builtin_member_type(&receiver, name, query_ctx);
        query_ctx.diagnostics.clear();
        // An overloaded member (`reduce`, `find`) is read off the interface
        // instantiated for the element, where its overloads stay one group;
        // the builtin table answers those `any`.
        if lib.as_ref().is_none_or(|ty| ty.is_degraded() || matches!(ty, Type::Any)) {
            let element = match &receiver {
                Type::Array(element) => element.as_ref().clone(),
                Type::Tuple(elements) => surge_ts_types::tuple_element_union(elements),
                _ => unreachable!(),
            };
            if let Some(member) = crate::infer::expression::lib_array_overloaded_member(element, name, query_ctx) {
                lib = Some(member);
            }
            query_ctx.diagnostics.clear();
        }
        match lib {
            Some(ty) if !ty.is_degraded() => Some(PropertyInfo {
                name: Arc::from(name),
                // The lib declares every callable `Array<T>` member as a method.
                method: property.as_ref().is_some_and(|property| property.method)
                    || match ty.peeled() {
                        Type::Function(_) => true,
                        Type::Object(object) => object.properties.is_empty() && object.call_signature().is_some(),
                        _ => false,
                    },
                ty,
                optional: false,
                readonly: false,
                declaration: None,
            }),
            _ => property,
        }
    }

    /// The type an index signature of the given key kind gives, where the
    /// (peeled) type declares one.
    pub fn index_type(&self, ty: &Type, number: bool) -> Option<Type> {
        match ty.peeled() {
            Type::Object(object) if number => object
                .number_index_type
                .as_deref()
                .or(object.string_index_type.as_deref())
                .cloned()
                .or_else(|| self.index_type(object.intersected_array()?, number)),
            Type::Object(object) => object.string_index_type.as_deref().cloned(),
            Type::Array(element) if number => Some(*element),
            // A tuple's is its `Array<E>`'s, `E` the union of its elements.
            Type::Tuple(elements) if number => Some(surge_ts_types::union_type(elements.to_vec())),
            Type::OpenTuple(tuple) if number => Some(tuple.element_union()),
            _ => None,
        }
    }

    /// What element `index` of an array binding pattern reads from a source
    /// that is not array-like (`checkIteratedTypeOrElementType`): the iterated
    /// type, or for a rest element an array of it.
    pub fn iterated_pattern_element(&self, source: &Type, index: usize, rest: bool) -> Option<Type> {
        crate::checks::expr::array_pattern_element_type(
            source,
            index,
            rest,
            self.state.query_ctx.options.strict_builtin_iterator_return,
        )
        .filter(|element| !element.is_unmodelled())
    }

    /// What a `for…of` over `iterable` binds each iteration
    /// (`checkIteratedTypeOrElementType`): an array-like's element, else the
    /// iterated type.
    pub fn for_of_element_type(&self, iterable: &Type) -> Option<Type> {
        let element = crate::checks::expr::array_pattern_element_type(
            iterable,
            0,
            false,
            self.state.query_ctx.options.strict_builtin_iterator_return,
        )
        .unwrap_or_else(|| crate::checks::function::for_of_element_type(iterable));
        (!element.is_unmodelled()).then_some(element)
    }

    /// tsc's `getSignaturesOfType`: call or construct signatures, one per
    /// overload.
    pub fn signatures(&self, ty: &Type, construct: bool) -> Vec<FunctionType> {
        let expand = |signature: &FunctionType| {
            let mut out = Vec::new();
            match signature.overloads() {
                Some(overloads) => out.extend(overloads.iter().cloned()),
                None => out.push(signature.clone()),
            }
            out
        };
        match ty.peeled() {
            Type::Function(function) if !construct => expand(&function),
            Type::Object(object) => {
                let signature = if construct { object.construct_signature() } else { object.call_signature() };
                signature.map(expand).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

fn property_info(name: Arc<str>, property: &ObjectProperty) -> PropertyInfo {
    // A private name's key carries its declaring class; it is named as written.
    let name = if surge_ts_types::private_name::is_private_name_key(&name) {
        Arc::from(surge_ts_types::private_name::display(&name))
    } else {
        name
    };
    PropertyInfo {
        name,
        ty: property.ty.clone(),
        optional: property.optional,
        readonly: property.readonly,
        method: property.method,
        declaration: property.declaration.clone(),
    }
}

fn array_properties(element: &Type) -> Vec<PropertyInfo> {
    surge_ts_types::array_property_names()
        .iter()
        .filter_map(|name| {
            let ty = surge_ts_types::array_member_type(name, element)?;
            Some(PropertyInfo {
                name: Arc::from(*name),
                method: matches!(ty, Type::Function(_)),
                ty,
                optional: false,
                readonly: false,
                declaration: None,
            })
        })
        .collect()
}

/// Whether `ty` holds an intersection the checker opened over operands it
/// merged away (`{ [x: string]: any; … }` with no operand kept).
fn holds_degraded_intersection(ty: &Type, depth: u8) -> bool {
    if depth == 0 {
        return false;
    }
    match ty {
        Type::Object(object) => {
            (object.synthetic_open_index
                && matches!(object.string_index_type.as_deref(), Some(Type::Any))
                && object.intersection_operands.is_none()
                && !object.properties.is_empty())
                || object.properties.values().any(|property| holds_degraded_intersection(&property.ty, depth - 1))
        }
        Type::Function(function) => {
            function.parameters().iter().any(|parameter| holds_degraded_intersection(parameter, depth - 1))
                || holds_degraded_intersection(function.return_type(), depth - 1)
        }
        Type::Union(union) => union.types().iter().any(|member| holds_degraded_intersection(member, depth - 1)),
        _ => false,
    }
}
