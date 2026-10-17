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

    /// tsc's `isTypeAssignableTo`: the assignable relation.
    pub fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        surge_ts_types::is_assignable_to(source, target)
    }

    /// The structural type behind any lazy reference.
    pub fn peel(&self, ty: &Type) -> Type {
        ty.peeled()
    }

    /// tsc's `!operand`: `false` or `true` when the operand is definitely
    /// truthy or falsy, else `boolean`.
    pub fn logical_not_type(&self, operand: &Type) -> Type {
        crate::infer::expression::logical_not_result_type(operand)
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
                query_ctx.type_declaration_scope = run
                    .shared_state
                    .module_resolution_scopes
                    .get(file_index)
                    .cloned()
                    .flatten();
                if let Some(analysis) = run.shared_state.module_analyses.get(file_index).and_then(Option::as_ref) {
                    query_ctx.type_declarations = analysis.local_type_declarations().as_ref().clone();
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
        self.resolve_type_text_in(file_index, text, type_parameters, namespaces, None)
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
        self.resolve_type_text_in(file_index, text, type_parameters, &[], Some(local_declaration))
    }

    fn resolve_type_text_in(
        &mut self,
        file_index: usize,
        text: &str,
        type_parameters: &[String],
        namespaces: &[String],
        local_declaration: Option<(usize, usize)>,
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
        let saved = std::mem::replace(&mut self.state.query_ctx.namespace_member_prefix_stack, namespaces.to_vec());
        let resolved = self.resolve_in_file_scope(ty, &substitution);
        self.state.query_ctx.namespace_member_prefix_stack = saved;
        Some(resolved)
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
        // instantiated for the element, where its overloads stay one group.
        if lib.as_ref().is_none_or(Type::is_degraded) {
            let element = match &receiver {
                Type::Array(element) => element.as_ref().clone(),
                Type::Tuple(elements) => surge_ts_types::tuple_element_union(elements),
                _ => unreachable!(),
            };
            let array = ParsedType::Named(Arc::new(ParsedNamedType {
                type_argument_spans: Vec::new(),
                name: "Array".to_string(),
                span: None,
                type_arguments: vec![ParsedType::Named(Arc::new(ParsedNamedType {
                    type_argument_spans: Vec::new(),
                    name: "__surge_api_element__".to_string(),
                    span: None,
                    type_arguments: Vec::new(),
                }))],
            }));
            let mut substitution = TypeParameterSubstitution::new();
            substitution.set("__surge_api_element__".to_string(), element, false);
            let interface = crate::infer::map_parsed_type_with_substitution(array, query_ctx, &substitution);
            query_ctx.diagnostics.clear();
            if let Type::Object(object) = interface.peeled()
                && let Some(member) = object.get_property(name)
            {
                lib = Some(member.ty.clone());
            }
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
                .cloned(),
            Type::Object(object) => object.string_index_type.as_deref().cloned(),
            Type::Array(element) if number => Some(*element),
            _ => None,
        }
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
            })
        })
        .collect()
}
