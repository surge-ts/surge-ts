//! Class declaration binding and member-body checking.
//!
//! A class binds two things: an instance *type* (fields + methods, registered as
//! an interface so it resolves in type position and on `new` results) and a
//! constructor/static *value* (a `Type::Object` carrying static members plus a
//! construct signature, registered as a value symbol so `new Class(...)`,
//! `Class.staticMember`, and `typeof Class` all work).

use surge_ts_syntax::{
    ParsedBindingName, ParsedClassAccessor, ParsedClassDeclaration, ParsedClassMember,
    ParsedClassMethod, ParsedClassProperty, ParsedFunctionParameter, ParsedFunctionType,
    ParsedFunctionTypeParameter, ParsedInterfaceMember, ParsedNamedType, ParsedType,
};
use surge_ts_types::{FunctionType, ObjectProperty, ObjectType, PropertyMap, Type};

use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;

use crate::checks::function::{
    check_function_body_with_signature_and_this, map_function_signature,
};
use crate::context::{CheckerContext, convert_span};
use crate::infer::map_parsed_type;
use crate::symbols::{
    ClassMemberSymbol, InterfaceInfo, SymbolInfo, SymbolKind, SymbolTable, TypeDeclarationInfo,
};

/// Builds the instance-side interface (fields + instance methods) for a class.
/// Static members and the constructor are excluded; they live on the value side.
pub(crate) fn class_instance_interface_info(
    class: &ParsedClassDeclaration,
    file_name: Arc<str>,
) -> InterfaceInfo {
    // A generic class's member is inferred with its type parameters as type
    // variables, and stays `any` when what it infers mentions them.
    let infer_members = crate::checks::function::lazy_body_returns(&file_name);
    let javascript = surge_ts_syntax::is_javascript_file_name(&file_name);
    let mut members: Vec<_> = class
        .members
        .iter()
        .filter(|member| !is_overload_implementation(class, member, false))
        .filter_map(|member| class_member_to_interface_member(class, member, infer_members, javascript))
        .collect();
    members.extend(constructor_parameter_property_members(class));

    let mut info = InterfaceInfo::new(
        class.name.clone(),
        file_name,
        class.name_span,
        class.type_parameters.clone(),
        class.extends.clone(),
        members,
        class.string_index_type.clone(),
        class.number_index_type.clone(),
        None,
        Vec::new(),
        Vec::new(),
        None,
    )
    .with_readonly_indexes(class.string_index_readonly, class.number_index_readonly);
    info.is_abstract_class = class.is_abstract;
    if let Some(constructor) = class.members.iter().find_map(|member| match member {
        ParsedClassMember::Constructor(constructor) => Some(constructor),
        _ => None,
    }) {
        info.declares_constructor = true;
        info.constructor_accessibility = constructor.accessibility;
    }
    info.is_class_instance = true;
    if !class.restricted_members.is_empty() {
        Arc::make_mut(&mut info.body).restricted_members = class.restricted_members.clone();
    }
    let class_members: Vec<ClassMemberSymbol> = class
        .members
        .iter()
        .filter_map(|member| {
            let (name, is_static, is_get_accessor, is_field) = match member {
                ParsedClassMember::Property(property) => {
                    (&property.name, property.is_static, false, !property.is_auto_accessor)
                }
                ParsedClassMember::Method(method) => (&method.name, method.is_static, false, false),
                ParsedClassMember::Accessor(accessor) => {
                    (&accessor.name, accessor.is_static, accessor.has_getter, false)
                }
                ParsedClassMember::Constructor(_) | ParsedClassMember::StaticBlock(_) => return None,
            };
            Some(ClassMemberSymbol {
                name: name.clone(),
                is_static,
                is_get_accessor,
                is_field,
            })
        })
        .collect();
    if !class_members.is_empty() {
        Arc::make_mut(&mut info.body).class_members = class_members;
    }
    info
}

/// A method body beside its own overload signatures: `getSignaturesOfSymbol`
/// drops the implementation, so callers see the overloads alone.
fn is_overload_implementation(class: &ParsedClassDeclaration, member: &ParsedClassMember, is_static: bool) -> bool {
    let ParsedClassMember::Method(method) = member else {
        return false;
    };
    method.is_static == is_static
        && method.has_body
        && class.members.iter().any(|other| {
            matches!(other, ParsedClassMember::Method(signature)
                if signature.is_static == is_static
                    && signature.name == method.name
                    && !signature.has_body
                    && !signature.is_abstract)
        })
}

fn class_member_to_interface_member(
    class: &ParsedClassDeclaration,
    member: &ParsedClassMember,
    infer_members: bool,
    javascript: bool,
) -> Option<ParsedInterfaceMember> {
    match member {
        ParsedClassMember::Property(property) if !property.is_static => {
            Some(ParsedInterfaceMember {
                name: property.name.clone(),
                name_span: property.name_span,
                optional: property.optional,
                is_abstract: property.is_abstract,
                is_method: false,
                ty: property
                    .declared_type
                    .clone()
                    .unwrap_or_else(|| inferred_property_type(class, property, infer_members, javascript)),
                readonly: property.readonly,
                write_ty: None,
            })
        }
        ParsedClassMember::Method(method) if !method.is_static => Some(ParsedInterfaceMember {
            name: method.name.clone(),
            name_span: method.name_span,
            optional: false,
            is_abstract: method.is_abstract,
            is_method: true,
            ty: method_function_type(class, method, infer_members),
            readonly: false,
            write_ty: None,
        }),
        ParsedClassMember::Accessor(accessor) if !accessor.is_static => {
            Some(ParsedInterfaceMember {
                name: accessor.name.clone(),
                name_span: accessor.name_span,
                optional: false,
                is_abstract: accessor.is_abstract,
                is_method: false,
                ty: inferred_accessor_type(class, accessor, infer_members)
                    .unwrap_or_else(|| accessor_property_type(accessor)),
                // A getter with no setter is read-only, exactly as
                // `isReadonlySymbol` has it.
                readonly: accessor.has_getter && !accessor.has_setter,
                write_ty: accessor_write_type(accessor),
            })
        }
        _ => None,
    }
}

/// Synthesizes instance members for constructor parameter properties — a
/// parameter carrying a `public`/`private`/`protected`/`readonly` modifier
/// declares a field of the same name and type. Only identifier-named parameters
/// can be parameter properties (TS rejects destructuring patterns here).
pub(crate) fn constructor_parameter_property_members(
    class: &ParsedClassDeclaration,
) -> Vec<ParsedInterfaceMember> {
    class
        .members
        .iter()
        .find_map(|member| match member {
            ParsedClassMember::Constructor(constructor) => Some(&constructor.parameters),
            _ => None,
        })
        .map(|parameters| {
            parameters
                .iter()
                .filter(|parameter| parameter.is_parameter_property)
                .filter_map(|parameter| {
                    let ParsedBindingName::Identifier { name, span } = &parameter.binding_name
                    else {
                        return None;
                    };
                    Some(ParsedInterfaceMember {
                        name: name.clone(),
                        name_span: *span,
                        // A parameter property written with a default
                        // (`readonly encoder: E = noopEncoder`) declares a
                        // *required* member: the parameter is optional at the
                        // call, the property never is. The parser folds a
                        // default into `optional` so arity accepts the omitted
                        // argument, so the initializer is what tells the two
                        // apart here.
                        optional: parameter.optional && parameter.initializer.is_none(),
                        is_abstract: false,
                        is_method: false,
                        ty: parameter.declared_type.clone().unwrap_or(ParsedType::Any),
                        readonly: parameter.is_readonly_parameter_property,
                        write_ty: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a write to an accessor is checked against, when that differs from what
/// a read of it produces: the setter's parameter type of a pair whose getter
/// declares something else. tsc keeps the two apart as `getTypeOfSymbol` and
/// `getWriteTypeOfSymbol`, and writes against the latter, so
/// `set value(next: number | string)` beside `get value(): number` accepts a
/// string. `None` when reading and writing share a type, which is every member
/// that is not such a pair.
fn accessor_write_type(accessor: &ParsedClassAccessor) -> Option<ParsedType> {
    let setter_param_type = accessor.setter_param_type.clone()?;
    if !accessor.has_getter || accessor.getter_return_type.is_none() {
        return None;
    }
    (accessor.getter_return_type.as_ref() != Some(&setter_param_type)).then_some(setter_param_type)
}

/// Lowers an accessor to the type of the property it presents. A getter's
/// return type wins (the read type); a setter-only accessor falls back to its
/// parameter type. Missing annotations degrade to `any`, matching the implicit
/// type tsc infers for an un-annotated accessor.
/// An unannotated property typed by its initializer: the syntactic lowering
/// when the initializer's type is evident from its syntax, otherwise read from
/// the expression on first demand.
fn inferred_property_type(
    class: &ParsedClassDeclaration,
    property: &ParsedClassProperty,
    infer_members: bool,
    javascript: bool,
) -> ParsedType {
    if let Some(assignments) = &property.this_assignments {
        return this_assigned_member_type(class, property, assignments, infer_members);
    }
    // A JavaScript initializer is inferred where it is written, so an object
    // literal in it is open-ended (`isJSLiteralType`) as the syntax alone
    // cannot say.
    let syntactic = property
        .initializer
        .as_ref()
        .filter(|_| !javascript)
        .and_then(|initializer| syntactic_initializer_type(initializer, property.readonly));
    match (syntactic, &property.initializer) {
        (Some(syntactic), _) => syntactic,
        (None, Some(initializer)) if infer_members => {
            ParsedType::InferredMember(Arc::new(surge_ts_syntax::ParsedInferredMember {
                class_name: class.name.clone(),
                class_type_parameters: class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect(),
                member_start: property.name_span.map_or(0, |span| span.start),
                member_name: property.name.clone(),
                keep_literal: property.readonly,
                source: surge_ts_syntax::ParsedInferredMemberSource::Initializer(
                    initializer.clone(),
                ),
            }))
        }
        _ => ParsedType::Any,
    }
}

/// A JavaScript member declared by `this.x = v`, typed from what is assigned
/// (`getWidenedTypeForAssignmentDeclaration`).
fn this_assigned_member_type(
    class: &ParsedClassDeclaration,
    property: &ParsedClassProperty,
    assignments: &surge_ts_syntax::ParsedThisAssignments,
    infer_members: bool,
) -> ParsedType {
    if !infer_members {
        return ParsedType::Any;
    }
    let mut assignments = assignments.clone();
    if assignments.in_constructor
        && let Some(constructor) = class.members.iter().find_map(|member| match member {
            ParsedClassMember::Constructor(constructor) => Some(constructor),
            _ => None,
        })
    {
        assignments.constructor_parameters = constructor
            .parameters
            .iter()
            .filter_map(|parameter| match &parameter.binding_name {
                surge_ts_syntax::ParsedBindingName::Identifier { name, .. } => {
                    Some((name.clone(), parameter.declared_type.clone()))
                }
                _ => None,
            })
            .collect();
    }
    ParsedType::InferredMember(Arc::new(surge_ts_syntax::ParsedInferredMember {
        class_name: class.name.clone(),
        class_type_parameters: class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect(),
        member_start: property.name_span.map_or(0, |span| span.start),
        member_name: property.name.clone(),
        keep_literal: false,
        source: surge_ts_syntax::ParsedInferredMemberSource::ThisAssignments(assignments),
    }))
}

/// A getter with no annotation on either half of the pair takes its body's
/// return type (tsc's `getTypeOfAccessors`).
fn inferred_accessor_type(
    class: &ParsedClassDeclaration,
    accessor: &ParsedClassAccessor,
    infer_members: bool,
) -> Option<ParsedType> {
    if !infer_members
        || accessor.getter_return_type.is_some()
        || accessor.setter_param_type.is_some()
    {
        return None;
    }
    let getter = accessor
        .declarations
        .iter()
        .find(|declaration| declaration.is_getter && declaration.has_body)?;
    Some(ParsedType::InferredMember(Arc::new(
        surge_ts_syntax::ParsedInferredMember {
            class_name: class.name.clone(),
            class_type_parameters: class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect(),
            member_start: accessor.name_span.map_or(0, |span| span.start),
            member_name: accessor.name.clone(),
            keep_literal: false,
            source: surge_ts_syntax::ParsedInferredMemberSource::GetterBody(getter.body.clone()),
        },
    )))
}

fn accessor_property_type(accessor: &ParsedClassAccessor) -> ParsedType {
    accessor
        .getter_return_type
        .clone()
        .or_else(|| accessor.setter_param_type.clone())
        .unwrap_or(ParsedType::Any)
}

/// A declaration's parameters as its callers see them. tsc's `addOptionality`,
/// as `build_function_type` applies it: a defaulted parameter a required one
/// follows is written `T | undefined`.
fn declared_signature_parameters(parameters: &[ParsedFunctionParameter]) -> Vec<ParsedFunctionTypeParameter> {
    let required = crate::checks::function::required_parameter_count(parameters);
    parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let mut lowered = parameter_to_type_parameter(parameter);
            if parameter.initializer.is_some() && index < required && !matches!(lowered.ty, ParsedType::Any) {
                lowered.ty = ParsedType::Union(std::sync::Arc::new(vec![lowered.ty, ParsedType::Undefined]));
                lowered.optional = false;
            }
            lowered
        })
        .collect()
}

fn method_function_type(class: &ParsedClassDeclaration, method: &ParsedClassMethod, infer_members: bool) -> ParsedType {
    let written_this = method.this_parameter_type.clone().map(|ty| ParsedFunctionTypeParameter {
        name: Some("this".to_string()),
        name_span: None,
        ty,
        optional: false,
        is_this: true,
        polymorphic_this: method.this_parameter_polymorphic,
        rest: false,
        bound_names: Vec::new(),
    });
    ParsedType::Function(std::sync::Arc::new(ParsedFunctionType {
        parameters: written_this
            .into_iter()
            .chain(declared_signature_parameters(&method.parameters))
            .collect(),
        // A generic class's syntactic return (`new MyList<T>(…)`) resolves
        // per instantiation, where the inferred one stands for all of them
        // and gives up on whatever names `T`, so it goes first there.
        return_type: Box::new(
            method
                .return_type
                .clone()
                .or_else(|| {
                    if class.type_parameters.is_empty() {
                        inferred_method_return_type(class, method, infer_members)
                            .or_else(|| syntactic_method_return_type(class, method))
                    } else {
                        syntactic_method_return_type(class, method)
                            .or_else(|| inferred_method_return_type(class, method, infer_members))
                    }
                })
                .unwrap_or(ParsedType::Any),
        ),
        type_parameters: method.type_parameters.clone(),
    }))
}

/// `getReturnTypeFromBody` for an instance method that ends returning `this`
/// and returns nothing else is the polymorphic `this` type.
fn returns_only_this(method: &ParsedClassMethod) -> bool {
    use surge_ts_syntax::{ParsedExpression, ParsedFunctionBodyStatement as Statement};

    fn returns_this(body: &[Statement]) -> bool {
        body.iter().all(|statement| match statement {
            Statement::Return(return_statement) => {
                matches!(return_statement.expression, Some(ParsedExpression::This { .. }))
            }
            Statement::Block(block) => returns_this(block),
            Statement::If(if_statement) => {
                returns_this(&if_statement.then_body) && returns_this(&if_statement.else_body)
            }
            Statement::While(while_statement) => returns_this(&while_statement.body),
            Statement::ForOf(for_of) => returns_this(&for_of.body),
            Statement::Switch(switch_statement) => {
                switch_statement.cases.iter().all(|case| returns_this(&case.consequent))
            }
            Statement::Try(try_statement) => {
                returns_this(&try_statement.block)
                    && try_statement.handler.as_ref().is_none_or(|handler| returns_this(&handler.body))
                    && returns_this(&try_statement.finalizer)
            }
            _ => true,
        })
    }

    if method.is_static || method.is_async || method.is_generator || method.this_parameter_type.is_some() {
        return false;
    }
    // A body that can complete without a `return` adds `undefined`.
    let ends_returning_this = matches!(
        method.body.last(),
        Some(Statement::Return(return_statement))
            if matches!(return_statement.expression, Some(ParsedExpression::This { .. }))
    );
    ends_returning_this && returns_this(&method.body)
}

/// An unannotated method's return as `getReturnTypeFromBody` reads it, for a
/// method that is not a generator (whose result is a `Generator`, not what its
/// body completes with). A generic method's is read over its own type
/// parameters as well as the class's, and again under each call's arguments.
fn inferred_method_return_type(
    class: &ParsedClassDeclaration,
    method: &ParsedClassMethod,
    infer_members: bool,
) -> Option<ParsedType> {
    if !infer_members
        || method.return_type.is_some()
        || !method.has_body
        || method.body.is_empty() && !method.is_generator
    {
        return None;
    }
    let parameter_types = declared_signature_parameters(&method.parameters)
        .into_iter()
        .map(|parameter| parameter.ty)
        .collect();
    Some(ParsedType::InferredMember(Arc::new(
        surge_ts_syntax::ParsedInferredMember {
            class_name: class.name.clone(),
            // A static member does not see the class's type parameters.
            class_type_parameters: if method.is_static {
                Vec::new()
            } else {
                class.type_parameters.iter().map(|parameter| parameter.name.clone()).collect()
            },
            member_start: method.name_span.map_or(0, |span| span.start),
            member_name: method.name.clone(),
            keep_literal: false,
            source: surge_ts_syntax::ParsedInferredMemberSource::MethodBody(
                surge_ts_syntax::ParsedInferredMethodBody {
                    parameters: method.parameters.clone(),
                    parameter_types,
                    body: method.body.clone(),
                    is_static: method.is_static,
                    is_async: method.is_async,
                    is_generator: method.is_generator,
                    this_parameter_type: method.this_parameter_type.clone(),
                    returns_only_this: returns_only_this(method),
                    type_parameters: method.type_parameters.clone(),
                },
            ),
        },
    )))
}

/// The return type an unannotated method takes from its body when every
/// `return` in it hands back a literal: the widened union of those literals'
/// primitives, as `getReturnTypeFromBody` gives it, `void` when no `return`
/// carries a value, and either wrapped in `Promise` for an `async` method.
/// A `return new C<…>()` of the enclosing class with its type arguments written
/// out is that instantiation, whatever the constructor takes — the builder
/// shape (`context<T>() { return new Builder<T, TMeta>() }`) whose result every
/// later link of the chain is typed from.
/// Anything else — another kind of returned value, a bare `return` beside a
/// valued one, a generator — is left for inference surge does not do here.
fn syntactic_method_return_type(
    class: &ParsedClassDeclaration,
    method: &ParsedClassMethod,
) -> Option<ParsedType> {
    use surge_ts_syntax::{ParsedExpression, ParsedFunctionBodyStatement as Statement};

    fn own_instantiation(
        class: &ParsedClassDeclaration,
        expression: &ParsedExpression,
    ) -> Option<ParsedType> {
        let ParsedExpression::New {
            callee,
            type_arguments,
            ..
        } = expression
        else {
            return None;
        };
        let ParsedExpression::Identifier { name, .. } = callee.as_ref() else {
            return None;
        };
        if *name != class.name || type_arguments.len() != class.type_parameters.len() {
            return None;
        }
        Some(ParsedType::Named(Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: name.clone(),
            span: None,
            type_arguments: type_arguments.clone(),
        })))
    }

    fn collect(
        class: &ParsedClassDeclaration,
        body: &[Statement],
        kinds: &mut Vec<ParsedType>,
        bare_return: &mut bool,
    ) -> Option<()> {
        for statement in body {
            match statement {
                Statement::Return(return_statement) => {
                    let Some(expression) = return_statement.expression.as_ref() else {
                        *bare_return = true;
                        continue;
                    };
                    let kind = match expression {
                        ParsedExpression::StringLiteral(_) => ParsedType::String,
                        ParsedExpression::TemplateLiteral { expressions, .. }
                            if expressions.is_empty() =>
                        {
                            ParsedType::String
                        }
                        ParsedExpression::NumberLiteral(_) => ParsedType::Number,
                        ParsedExpression::BooleanLiteral(_) => ParsedType::Boolean,
                        other => own_instantiation(class, other)?,
                    };
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
                Statement::Block(block) => collect(class, block, kinds, bare_return)?,
                Statement::If(if_statement) => {
                    collect(class, &if_statement.then_body, kinds, bare_return)?;
                    collect(class, &if_statement.else_body, kinds, bare_return)?;
                }
                Statement::While(while_statement) => {
                    collect(class, &while_statement.body, kinds, bare_return)?
                }
                Statement::ForOf(for_of) => collect(class, &for_of.body, kinds, bare_return)?,
                Statement::Switch(switch_statement) => {
                    for case in &switch_statement.cases {
                        collect(class, &case.consequent, kinds, bare_return)?;
                    }
                }
                Statement::Try(try_statement) => {
                    collect(class, &try_statement.block, kinds, bare_return)?;
                    if let Some(handler) = &try_statement.handler {
                        collect(class, &handler.body, kinds, bare_return)?;
                    }
                    collect(class, &try_statement.finalizer, kinds, bare_return)?;
                }
                _ => {}
            }
        }
        Some(())
    }

    if method.is_generator || !method.has_body {
        return None;
    }
    let mut kinds = Vec::new();
    let mut bare_return = false;
    collect(class, &method.body, &mut kinds, &mut bare_return)?;
    let returned = match kinds.len() {
        // No value is returned anywhere: the method is `void`.
        0 => ParsedType::Void,
        _ if bare_return => return None,
        1 => kinds.pop()?,
        _ => ParsedType::Union(Arc::new(kinds)),
    };
    Some(if method.is_async {
        ParsedType::Named(Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: "Promise".to_string(),
            span: None,
            type_arguments: vec![returned],
        }))
    } else {
        returned
    })
}

fn parameter_to_type_parameter(parameter: &ParsedFunctionParameter) -> ParsedFunctionTypeParameter {
    let (name, name_span) = match &parameter.binding_name {
        ParsedBindingName::Identifier { name, span } => (Some(name.clone()), *span),
        _ => (None, None),
    };

    ParsedFunctionTypeParameter {
        name,
        name_span,
        ty: parameter.declared_type.clone().unwrap_or(ParsedType::Any),
        optional: parameter.optional,
        is_this: false,
        polymorphic_this: false,
        rest: parameter.rest,
        bound_names: match &parameter.binding_name {
            ParsedBindingName::Identifier { .. } => Vec::new(),
            pattern => pattern.bound_names(),
        },
    }
}

/// The written constructor signature of a generic class, carried on its `any`
/// value side so `new C(arg)` can infer the class's type arguments from the
/// arguments instead of falling back to their declared defaults
/// (`MutationObserver<…, TVariables = void>` stayed at `void`). Only an explicit
/// constructor contributes: with none there is nothing to infer from, and the
/// signature carried declares no type parameters. It also carries the class's
/// construct signatures, which `new` checks its arguments against.
fn generic_class_constructor_signature(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &CheckerContext,
) -> Option<Arc<crate::symbols::FunctionSignatureInfo>> {
    let base = class.extends.first().and_then(|base| {
        scope
            .and_then(|scope| scope.get(&base.name))
            .or_else(|| ctx.symbols.get(&base.name))
            .or_else(|| {
                ctx.module_value_fallback
                    .as_ref()
                    .and_then(|fallback| fallback.get(&base.name))
            })
    });
    let construct_signatures =
        crate::checks::call::construct::generic_class_construct_signatures(class, base, &ctx.file_name);
    let constructor = class.members.iter().find_map(|member| match member {
        ParsedClassMember::Constructor(constructor) => Some(constructor),
        _ => None,
    });
    let signature = match constructor {
        Some(constructor) => crate::checks::function::function_signature_info(
            &class.type_parameters,
            &constructor.parameters,
            None,
            &ctx.file_name,
        ),
        None if !construct_signatures.is_empty() => {
            crate::checks::function::function_signature_info(&[], &[], None, &ctx.file_name)
        }
        None => return None,
    };
    let mut signature = Arc::unwrap_or_clone(signature);
    signature.construct_signatures =
        (!construct_signatures.is_empty()).then(|| Arc::new(construct_signatures));
    Some(Arc::new(signature))
}

/// Writes `class`'s own static properties, methods and accessors into
/// `properties`.
fn collect_static_members(
    class: &ParsedClassDeclaration,
    properties: &mut PropertyMap,
    ctx: &mut CheckerContext,
) {
    let mut own_methods: Vec<&str> = Vec::new();
    for member in &class.members {
        match member {
            ParsedClassMember::Property(property) if property.is_static => {
                let property_type = static_property_type(class, property, ctx);
                let object_property = if property.optional {
                    ObjectProperty::optional(property_type)
                } else {
                    ObjectProperty::required(property_type)
                };
                properties.insert(
                    property.name.as_str().into(),
                    object_property.with_readonly(property.readonly),
                );
            }
            ParsedClassMember::Method(method) if method.is_static && !is_overload_implementation(class, member, true) => {
                let infer_members = crate::checks::function::lazy_body_returns(&ctx.file_name);
                let inferred_return = inferred_method_return_type(class, method, infer_members)
                    .or_else(|| syntactic_method_return_type(class, method));
                let function_type = static_signature_type(
                    &method.type_parameters,
                    &method.parameters,
                    method.return_type.as_ref().or(inferred_return.as_ref()),
                    ctx,
                );
                // Ambient overloads (`static on(a: X): R; static on(a: Y): R;`)
                // reach here as separate members; together they are one group.
                let function_type = match properties.get(method.name.as_str()) {
                    Some(previous) if own_methods.contains(&method.name.as_str()) => match &previous.ty {
                        Type::Function(previous) => crate::checks::function::merge_overload_group_signatures(
                            previous,
                            &function_type,
                        ),
                        _ => function_type,
                    },
                    _ => function_type,
                };
                own_methods.push(method.name.as_str());
                properties.insert(
                    method.name.as_str().into(),
                    ObjectProperty::required(Type::Function(function_type)),
                );
            }
            ParsedClassMember::Accessor(accessor) if accessor.is_static => {
                let property_type = map_parsed_type(accessor_property_type(accessor), ctx);
                properties.insert(
                    accessor.name.as_str().into(),
                    ObjectProperty::required(property_type)
                        .with_readonly(accessor.has_getter && !accessor.has_setter),
                );
            }
            _ => {}
        }
    }
    let _ = crate::infer::types::late_bind_member_names(properties, ctx);
}

/// A generic class's static side: its statics (own and inherited), a
/// `prototype` and a construct signature taking the class's type parameters.
/// A `new` call does not read that signature: it infers the class's type
/// arguments through the written constructor carried on the symbol.
fn generic_class_value_symbol(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> SymbolInfo {
    // The instance is not resolved here: binding the class is too early to
    // expand its members, whose own diagnostics belong to the class check.
    // `new` and `instanceof` build `C<…>` by name where they need it.
    let instance = Type::Any;
    // A static naming the class's type parameters is TS2302 where the member
    // itself is checked; building the static side reports nothing of its own.
    let diagnostics_before = ctx.diagnostics().len();
    let mut properties = inherited_static_properties(class, scope, ctx);
    properties.insert("prototype".into(), ObjectProperty::required(instance.clone()));
    collect_static_members(class, &mut properties, ctx);
    let static_string_index = class.static_string_index_type.clone().map(|ty| map_parsed_type(ty, ctx));
    let static_number_index = class.static_number_index_type.clone().map(|ty| map_parsed_type(ty, ctx));
    ctx.truncate_diagnostics(diagnostics_before);

    // Only the arity is read off this head, a default by its `=`.
    let type_parameters = class
        .type_parameters
        .iter()
        .map(|parameter| match parameter.default_type {
            Some(_) => format!("{} = unknown", parameter.name),
            None => parameter.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let construct_signature = FunctionType::new(vec![Type::Any], instance, true, 0)
        .with_type_parameter_head(Some(type_parameters));
    let static_type = ObjectType::new(properties, static_string_index)
        .with_number_index_type(static_number_index)
        .with_readonly_indexes(class.static_string_index_readonly, class.static_number_index_readonly)
        .with_construct_signature(construct_signature)
        .with_alias_name(format!("typeof {}", class.name));

    SymbolInfo {
        ty: Type::Object(static_type),
        kind: SymbolKind::Const,
        function_signature: generic_class_constructor_signature(class, scope, ctx),
    }
}

/// The constructor type tsc's `checkAndReportErrorForMissingPrefix` looks a
/// static member up on. A generic class's value is modelled as `any` for its
/// uses ([`generic_class_value_symbol`]), which would answer every name.
fn generic_class_static_side(
    class: &ParsedClassDeclaration,
    instance_type: &Type,
    ctx: &mut CheckerContext,
) -> Type {
    let mut properties = inherited_static_properties(class, None, ctx);
    properties.insert(
        "prototype".into(),
        ObjectProperty::required(instance_type.clone()),
    );
    collect_static_members(class, &mut properties, ctx);
    let construct_signature = FunctionType::new(vec![Type::Any], instance_type.clone(), true, 0);
    Type::Object(ObjectType::new(properties, None).with_construct_signature(construct_signature))
}

/// Builds the constructor/static-side value symbol: a `Type::Object` whose
/// properties are the static members and whose construct signature yields the
/// instance type.
pub(crate) fn build_class_value_symbol(
    class: &ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) -> SymbolInfo {
    build_class_value_symbol_with_scope(class, None, ctx)
}

/// [`build_class_value_symbol`] with the in-progress value table the class is
/// being bound into, which is where a base class declared earlier in the same
/// scope lives — `ctx.symbols` does not have it yet at binding time.
pub(crate) fn build_class_value_symbol_with_scope(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> SymbolInfo {
    if !class.type_parameters.is_empty() {
        return generic_class_value_symbol(class, scope, ctx);
    }

    let instance_type = class_instance_type(class, ctx);
    let construct_signature = class_construct_signature(class, instance_type.clone(), scope, ctx);

    // Statics are inherited: `class D extends B {}` makes every static of `B`
    // reachable as `D.x`. The base's static side is a value, not part of the
    // instance-side interface classes are bound as, so it is read from the
    // value environment — which means an unbound base simply contributes
    // nothing rather than being wrong.
    let mut properties = inherited_static_properties(class, scope, ctx);
    // Every class value carries `prototype`, typed as the instance — the shape
    // `Object.setPrototypeOf(this, C.prototype)` reads. A static member named
    // `prototype` is illegal in TypeScript, so nothing below can overwrite it.
    // It is also the one inherited entry that must not survive: `D.prototype`
    // is a `D`, not a `B`.
    properties.insert(
        "prototype".into(),
        ObjectProperty::required(instance_type),
    );
    collect_static_members(class, &mut properties, ctx);

    // tsc's `resolveAnonymousTypeMembers`: the class symbol's static
    // `__index` member is the constructor type's index signatures.
    let static_string_index = class
        .static_string_index_type
        .clone()
        .map(|ty| map_parsed_type(ty, ctx));
    let static_number_index = class
        .static_number_index_type
        .clone()
        .map(|ty| map_parsed_type(ty, ctx));
    let static_type = ObjectType::new(properties, static_string_index)
        .with_number_index_type(static_number_index)
        .with_readonly_indexes(class.static_string_index_readonly, class.static_number_index_readonly)
        .with_construct_signature(construct_signature)
        .with_alias_name(format!("typeof {}", class.name));
    // tsc's `getBaseTypeVariableOfClass`: a class extending a value typed by a
    // type variable is `typeof C & T` — the mixin idiom returns it as `T`.
    let static_type = match base_type_variable(class, scope, ctx) {
        Some(variable) => static_type
            .with_intersection_marker()
            .with_intersection_operands(vec![variable]),
        None => static_type,
    };

    SymbolInfo {
        ty: Type::Object(static_type),
        kind: SymbolKind::Const,
        function_signature: None,
    }
}

fn base_type_variable(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &CheckerContext,
) -> Option<Type> {
    let base = class.extends.first()?;
    let symbol = scope
        .and_then(|scope| scope.get(&base.name))
        .or_else(|| ctx.symbols.get(&base.name))?;
    symbol.ty.is_type_variable().then(|| symbol.ty.clone())
}

/// The base class's static members, as the starting point for a derived class's
/// static side (tsc's `resolveAnonymousTypeMembers` adds the properties of the
/// base constructor type). Empty when the class has no base, when the base's
/// value is not in scope yet, or when the base's static side is not an object (a
/// generic class models its value as `any`).
fn inherited_static_properties(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> PropertyMap {
    let base_static = match class.heritage_expression.as_deref() {
        Some(expression) => expression_base_constructor_type(expression, scope, ctx),
        None => base_static_side(class, scope, ctx),
    };
    // `addInheritedMembers` leaves a static private name with its own class.
    base_static
        .map(|base_static| {
            base_static
                .properties
                .iter()
                .filter(|(name, _)| !surge_ts_types::private_name::is_static(name))
                .map(|(name, property)| (name.clone(), property.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// tsc's `getBaseConstructorTypeOfClass` for an `extends` expression that is
/// not a name (`extends Mixin(A, B)`): the expression's type where the class is
/// bound, an intersection's being the merged surface of its constituents. What
/// inferring it reports is dropped, as binding runs before every value the
/// expression may name is in scope.
fn expression_base_constructor_type(
    expression: &surge_ts_syntax::ParsedExpression,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> Option<ObjectType> {
    let enclosing;
    let symbols = match scope {
        Some(scope) => scope,
        None => {
            enclosing = ctx
                .symbols
                .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
            &enclosing
        }
    };
    let diagnostics_before = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    match inferred {
        crate::infer::InferredExpression::Known(ty) => match ty.peeled() {
            Type::Object(object) => Some(object),
            _ => None,
        },
        _ => None,
    }
}

fn base_static_side(
    class: &ParsedClassDeclaration,
    scope: Option<&SymbolTable>,
    ctx: &CheckerContext,
) -> Option<ObjectType> {
    let base = class.extends.first()?;
    // A base bound by an import is not in the scope being built: inside
    // `declare module "stream"`, `class Stream extends EventEmitter` names the
    // block's `import { EventEmitter } from "node:events"`, which the export
    // table build seeds as the value fallback.
    let base_symbol = scope
        .and_then(|scope| scope.get(&base.name))
        .or_else(|| ctx.symbols.get(&base.name))
        .or_else(|| {
            ctx.module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get(&base.name))
        })?;
    match base_symbol.ty.peeled() {
        Type::Object(base_static) => Some(base_static),
        _ => None,
    }
}

/// tsc's `getDefaultConstructSignatures`: a derived class that declares no
/// constructor takes the base's construct signatures, returning itself.
fn inherited_construct_signature(base: &FunctionType, instance_type: &Type) -> FunctionType {
    let retarget = |signature: &FunctionType| {
        let retargeted = FunctionType::new(
            signature.parameters().to_vec(),
            instance_type.clone(),
            signature.is_variadic(),
            signature.required_parameter_count(),
        );
        match signature.parameter_names() {
            Some(names) => retargeted.with_parameter_names(names.to_vec()),
            None => retargeted,
        }
    };
    let retargeted = retarget(base);
    match base.overloads() {
        Some(overloads) => retargeted.with_overloads(overloads.iter().map(retarget).collect()),
        None => retargeted,
    }
}

/// `getDefaultConstructSignatures` over a generic base class, whose value
/// carries its construct signatures written over its own type parameters
/// (`generic_class_value_symbol`): each one whose type parameters the `extends`
/// clause's argument count fits (`getMinTypeArgumentCount`), instantiated with
/// those arguments and the defaults of the rest (`fillMissingTypeArguments`),
/// returning the derived class. A JavaScript class, whose missing arguments tsc
/// fills with `any`, and a base written as an expression keep the permissive
/// signature.
fn generic_base_construct_signature(
    class: &ParsedClassDeclaration,
    instance_type: &Type,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> Option<FunctionType> {
    if class.heritage_expression.is_some() || surge_ts_syntax::is_javascript_file_name(&ctx.file_name) {
        return None;
    }
    let base_reference = class.extends.first()?;
    let base_signatures = scope
        .and_then(|scope| scope.get(&base_reference.name))
        .or_else(|| ctx.symbols.get(&base_reference.name))
        .or_else(|| {
            ctx.module_value_fallback
                .as_ref()
                .and_then(|fallback| fallback.get(&base_reference.name))
        })?
        .function_signature
        .as_ref()?
        .construct_signatures
        .clone()?;
    let written = base_reference.type_arguments.len();
    let mut signatures = Vec::with_capacity(base_signatures.len());
    for base in base_signatures.iter() {
        let type_parameters = &base.signature.type_parameters;
        let minimum = type_parameters
            .iter()
            .rposition(|parameter| parameter.default_type.is_none())
            .map_or(0, |index| index + 1);
        if written < minimum || written > type_parameters.len() {
            continue;
        }
        // The clause's arguments are reported where the class is checked.
        let checkpoint = ctx.diagnostics().len();
        let substitution = crate::checks::call::explicit_type_argument_substitution(
            &base.signature,
            &base_reference.type_arguments,
            ctx,
        );
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        let instantiated = crate::checks::call::instantiate_function_type_with_substitution(
            &base.template,
            &base.signature,
            &substitution,
            ctx,
        )
        .into_owned();
        let names: Vec<Option<Arc<str>>> = base
            .signature
            .parameter_names
            .iter()
            .map(|name| name.as_deref().map(Arc::from))
            .collect();
        signatures.push(inherited_construct_signature(
            &instantiated.with_parameter_names(names),
            instance_type,
        ));
    }
    let mut signatures = signatures.into_iter();
    let first = signatures.next()?;
    Some(signatures.fold(first, |group, signature| {
        crate::checks::function::merge_overload_group_signatures(&group, &signature)
    }))
}

fn static_property_type(
    class: &ParsedClassDeclaration,
    property: &ParsedClassProperty,
    ctx: &mut CheckerContext,
) -> Type {
    if let Some(assignments) = &property.this_assignments {
        let infer_members = class.type_parameters.is_empty()
            && crate::checks::function::lazy_body_returns(&ctx.file_name);
        return map_parsed_type(
            this_assigned_member_type(class, property, assignments, infer_members),
            ctx,
        );
    }
    match property.declared_type.clone() {
        Some(declared_type) => map_parsed_type(declared_type, ctx),
        // An open-ended JavaScript object literal (`isJSLiteralType`) has no
        // syntactic type; the static side infers nothing else either.
        None if surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
            && matches!(
                property.initializer,
                Some(surge_ts_syntax::ParsedExpression::ObjectLiteral { .. })
            ) =>
        {
            Type::Any
        }
        // tsc types an unannotated property by checking its initializer
        // (`getTypeForVariableLikeDeclaration`), and a function-valued one
        // checks to its own declaration's signature (`getSignatureFromDeclaration`)
        // — the same signature a static method with those parameters and
        // return type has.
        None => match property.initializer.as_ref() {
            Some(surge_ts_syntax::ParsedExpression::ArrowFunction(function)) => {
                Type::Function(static_signature_type(
                    &function.type_parameters,
                    &function.parameters,
                    function.return_type.as_ref(),
                    ctx,
                ))
            }
            Some(initializer) => {
                let syntactic = syntactic_initializer_type(initializer, property.readonly);
                let infer_members = class.type_parameters.is_empty()
                    && crate::checks::function::lazy_body_returns(&ctx.file_name);
                let property_type = match syntactic {
                    Some(syntactic) => syntactic,
                    None if infer_members => ParsedType::InferredMember(Arc::new(surge_ts_syntax::ParsedInferredMember {
                        class_name: class.name.clone(),
                        class_type_parameters: Vec::new(),
                        member_start: property.name_span.map_or(0, |span| span.start),
                        member_name: property.name.clone(),
                        keep_literal: property.readonly,
                        source: surge_ts_syntax::ParsedInferredMemberSource::StaticInitializer(initializer.clone()),
                    })),
                    None => ParsedType::Any,
                };
                map_parsed_type(property_type, ctx)
            }
            None => map_parsed_type(initializer_property_type(property), ctx),
        },
    }
}

/// A static member's call signature from its declaration. A generic or
/// predicate one (`static assert(v): asserts v is E`) keeps its written
/// signature on the handle, so a call can instantiate it and a guard can read
/// the predicate.
fn static_signature_type(
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    parameters: &[ParsedFunctionParameter],
    return_type: Option<&ParsedType>,
    ctx: &mut CheckerContext,
) -> FunctionType {
    let function_type =
        map_function_signature(parameters, return_type, type_parameters, None, ctx);
    if type_parameters.is_empty() && !matches!(return_type, Some(ParsedType::Predicate(_))) {
        return function_type;
    }
    function_type.with_declaration(Arc::new(crate::checks::call::DeclaredMemberSignature {
        signature: crate::checks::function::function_signature_info(
            type_parameters,
            parameters,
            return_type,
            &ctx.file_name,
        ),
        outer_type_arguments: Vec::new(),
        generic_shape: None,
    }))
}

/// The type an unannotated property takes from its initializer, as tsc's
/// `getWidenedTypeForVariableLikeDeclaration` gives it: widened, except that a
/// `readonly` property keeps its literal. The instance side is built from
/// written types before any expression is checked, so only initializers whose
/// type is evident from their syntax are lowered; the rest stay `any`.
fn initializer_property_type(property: &ParsedClassProperty) -> ParsedType {
    property
        .initializer
        .as_ref()
        .and_then(|initializer| syntactic_initializer_type(initializer, property.readonly))
        .unwrap_or(ParsedType::Any)
}

fn syntactic_initializer_type(initializer: &surge_ts_syntax::ParsedExpression, keep_literal: bool) -> Option<ParsedType> {
    use surge_ts_syntax::ParsedExpression;
    Some(match initializer {
        ParsedExpression::StringLiteral(value) if keep_literal => ParsedType::StringLiteral(value.clone()),
        ParsedExpression::NumberLiteral(value) if keep_literal => ParsedType::NumberLiteral(value.clone()),
        ParsedExpression::BooleanLiteral(value) if keep_literal => ParsedType::BooleanLiteral(*value),
        ParsedExpression::StringLiteral(_) => ParsedType::String,
        ParsedExpression::NumberLiteral(_) => ParsedType::Number,
        ParsedExpression::BooleanLiteral(_) => ParsedType::Boolean,
        ParsedExpression::TemplateLiteral { expressions, .. } if expressions.is_empty() => {
            ParsedType::String
        }
        // An asserted literal is regular, which widening leaves alone.
        ParsedExpression::ConstAssertion { expression, .. } => match expression.as_ref() {
            ParsedExpression::StringLiteral(value) => ParsedType::StringLiteral(value.clone()),
            ParsedExpression::NumberLiteral(value) => ParsedType::NumberLiteral(value.clone()),
            ParsedExpression::BooleanLiteral(value) => ParsedType::BooleanLiteral(*value),
            _ => return None,
        },
        // An empty array literal is `never[]` under strictNullChecks: nothing
        // widens it to an evolving array the way a `let` binding would be.
        // Without it the literal is `undefined[]`, which widens to `any[]`.
        ParsedExpression::ArrayLiteral { elements, .. } if elements.is_empty() => {
            if surge_ts_types::strict_null_checks() {
                ParsedType::Array(Arc::new(ParsedType::Never))
            } else {
                ParsedType::Array(Arc::new(ParsedType::Any))
            }
        }
        ParsedExpression::ArrayLiteral { elements, .. } => {
            let mut element_types = elements.iter().map(|element| match &element.expression {
                _ if element.spread => None,
                ParsedExpression::StringLiteral(_) => Some(ParsedType::String),
                ParsedExpression::NumberLiteral(_) => Some(ParsedType::Number),
                ParsedExpression::BooleanLiteral(_) => Some(ParsedType::Boolean),
                _ => None,
            });
            let first = element_types.next()??;
            if !element_types.all(|element| element.as_ref() == Some(&first)) {
                return None;
            }
            ParsedType::Array(Arc::new(first))
        }
        // A function whose return type is written has the signature it spells.
        // A class declaration's property initializer has no contextual type
        // (`getContextualTypeForVariableLikeDeclaration` gives one to a class
        // expression's static side alone), so an unannotated parameter is
        // `any` and a defaulted one takes its initializer's widened type. oxc
        // keeps a `this` parameter out of the list, so a function expression
        // that writes one is not lowered.
        ParsedExpression::ArrowFunction(function)
            if function.this_binding != surge_ts_syntax::ParsedThisBinding::Own =>
        {
            let return_type = function.return_type.clone()?;
            let mut parameters = function.parameters.clone();
            for parameter in &mut parameters {
                if parameter.declared_type.is_none()
                    && let Some(initializer) = &parameter.initializer
                {
                    parameter.declared_type = Some(syntactic_initializer_type(initializer, false)?);
                }
            }
            ParsedType::Function(Arc::new(ParsedFunctionType {
                parameters: declared_signature_parameters(&parameters),
                return_type: Box::new(return_type),
                type_parameters: function.type_parameters.clone(),
            }))
        }
        ParsedExpression::ObjectLiteral { properties, .. } => {
            let mut members = Vec::with_capacity(properties.len());
            for property in properties {
                if property.is_spread || property.is_accessor || property.computed_key.is_some() {
                    return None;
                }
                // A member is a mutable location, so its literal widens even
                // when the property holding the object is `readonly`. One the
                // syntax cannot type leaves the object to inference.
                if property.is_method {
                    return None;
                }
                let ty = syntactic_initializer_type(&property.value, false)?;
                members.push(surge_ts_syntax::ParsedObjectTypeProperty {
                    name: property.name.clone(),
                    name_span: property.name_span,
                    ty,
                    optional: false,
                    is_method: property.is_method,
                    readonly: false,
                    write_ty: None,
                });
            }
            ParsedType::Object(Arc::new(surge_ts_syntax::ParsedObjectType {
                properties: members,
                string_index_type: None,
                number_index_type: None,
                string_index_readonly: false,
                number_index_readonly: false,
                call_signature: None,
                call_signature_overloads: Vec::new(),
                construct_signature: None,
                construct_signature_overloads: Vec::new(),
                non_primitive: false,
                display_name: None,
                abstract_construct_signature: false,
                keyed_indexes: None,
                number_index_span: None,
            }))
        }
        _ => return None,
    })
}

/// Inside its own body a generic class's instance is `C<T, …>` over its type
/// variables when they are bound (tsc's `this` type is the class instantiated
/// with its own parameters); otherwise the bare reference, as before.
fn class_instance_type(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) -> Type {
    let own_arguments_bound = !class.type_parameters.is_empty()
        && class.type_parameters.iter().all(|parameter| {
            parameter.name_span.is_some_and(|span| {
                surge_ts_types::type_variable::variable_for_declaration(&ctx.file_name, span.start as u32).is_some()
            })
        });
    let type_arguments = if own_arguments_bound {
        class
            .type_parameters
            .iter()
            .map(|parameter| {
                ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
                    type_argument_spans: Vec::new(),
                    name: parameter.name.clone(),
                    span: None,
                    type_arguments: Vec::new(),
                }))
            })
            .collect()
    } else {
        Vec::new()
    };
    map_parsed_type(
        ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: class.name.clone(),
            span: class.name_span,
            type_arguments,
        })),
        ctx,
    )
}

fn class_construct_signature(
    class: &ParsedClassDeclaration,
    instance_type: Type,
    scope: Option<&SymbolTable>,
    ctx: &mut CheckerContext,
) -> FunctionType {
    let named_instance = ParsedType::Named(std::sync::Arc::new(ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: class.name.clone(),
        span: class.name_span,
        type_arguments: Vec::new(),
    }));

    let constructors = class
        .members
        .iter()
        .filter_map(|member| match member {
            ParsedClassMember::Constructor(constructor) => Some(constructor),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Overloaded constructors are what `new` resolves against; outside an
    // ambient class the last declaration is the implementation, which tsc
    // hides behind its overloads.
    let overloads = match constructors.as_slice() {
        [] => &[][..],
        [_] => &constructors[..],
        [overloads @ .., _] if !class.is_declare => overloads,
        _ => &constructors[..],
    };
    // `declarationBelongsToPrivateAmbientMember`: a private constructor of an
    // ambient class reports no implicit `any` for its parameters.
    let ambient = class.is_declare || crate::modules::is_declaration_file_name(&ctx.file_name);
    let declared_accessibility = overloads.first().map(|constructor| match constructor.accessibility {
        Some(surge_ts_syntax::ParsedMemberAccessibility::Private) => surge_ts_types::ConstructorAccessibility::Private,
        Some(surge_ts_syntax::ParsedMemberAccessibility::Protected) => {
            surge_ts_types::ConstructorAccessibility::Protected
        }
        None => surge_ts_types::ConstructorAccessibility::Public,
    });
    // checker.go `getDefaultConstructSignatures`: the signature is abstract
    // exactly when this class is, whatever it inherits.
    let modifiers = |accessibility| surge_ts_types::ConstructModifiers::new(class.is_abstract, accessibility);
    let mut signatures = overloads.iter().map(|constructor| {
        let private_ambient = ambient
            && matches!(constructor.accessibility, Some(surge_ts_syntax::ParsedMemberAccessibility::Private));
        let checkpoint = ctx.diagnostics().len();
        let signature = map_function_signature(
            &constructor.parameters,
            Some(&named_instance),
            &[],
            None,
            ctx,
        );
        if private_ambient {
            let reported: Vec<Diagnostic> = ctx.diagnostics()[checkpoint..].to_vec();
            ctx.truncate_diagnostics(checkpoint);
            for diagnostic in reported {
                if !matches!(
                    diagnostic.code,
                    surge_ts_diagnostics::DiagnosticCode::TypeScript(7006 | 7019 | 7031)
                ) {
                    ctx.push(diagnostic);
                }
            }
        }
        signature
    });
    if let Some(first) = signatures.next() {
        let group = signatures.fold(first, |group, signature| {
            crate::checks::function::merge_overload_group_signatures(&group, &signature)
        });
        let accessibility = declared_accessibility.unwrap_or(surge_ts_types::ConstructorAccessibility::Public);
        return group.with_construct_modifiers(modifiers(accessibility));
    }

    if !class.extends.is_empty()
        && !class.extends.iter().all(|base| base_value_is_no_constructor(base, scope, ctx))
    {
        if let Some(signature) = generic_base_construct_signature(class, &instance_type, scope, ctx) {
            return signature.with_construct_modifiers(modifiers(surge_ts_types::ConstructorAccessibility::Undeclared));
        }
        if let Some(base_signature) = base_static_side(class, scope, ctx)
            .and_then(|base_static| base_static.construct_signature().cloned())
        {
            let inherited = base_signature.construct_modifiers().with_abstract(class.is_abstract);
            return inherited_construct_signature(&base_signature, &instance_type).with_construct_modifiers(inherited);
        }
        // A base whose value is not an object here — a generic class with no
        // signature the clause's arguments fit, an expression, one not bound
        // yet — has no signature to inherit. Accept any argument list rather
        // than report the base's arity as zero: `new ZodString({ … })` on a
        // derived class was TS2554 "Expected 0 arguments".
        return FunctionType::new(vec![Type::Any], instance_type, true, 0)
            .with_construct_modifiers(modifiers(surge_ts_types::ConstructorAccessibility::Undeclared));
    }

    // A class with no explicit constructor is constructible with zero arguments.
    FunctionType::new(vec![], instance_type, false, 0)
        .with_construct_modifiers(modifiers(surge_ts_types::ConstructorAccessibility::Undeclared))
}

/// Type-checks a class's constructor and method bodies, binding `this` to the
/// instance type (instance members/constructor) or static side (static methods).
/// TS2515/TS2654/TS2655: a class that is not itself abstract has to implement
/// every abstract member it inherits. tsc picks the code by how many are
/// missing — one names it, two to five list them, six or more list the first
/// four and count the rest — and names the *direct* base class.
///
/// Base resolution is conservative: a base that does not resolve to a
/// source-declared class leaves the whole check quiet rather than risking a
/// false positive.
fn check_inherited_abstract_members(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    const LISTED_WHEN_TRUNCATED: usize = 4;
    const MAX_LISTED: usize = 5;

    if class.is_abstract {
        return;
    }
    let Some(base) = class.extends.first() else {
        return;
    };

    let mut satisfied: std::collections::HashSet<String> = class
        .members
        .iter()
        .filter_map(implemented_member_name)
        .collect();
    satisfied.extend(
        constructor_parameter_property_members(class)
            .into_iter()
            .map(|member| member.name),
    );
    satisfied.extend(merged_interface_member_names(class, &base.name, ctx));

    let mut missing: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next_base = Some(base.name.clone());
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    while let Some(base_name) = next_base.take() {
        if !visited.insert(base_name.clone()) {
            break;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&base_name)
        else {
            return;
        };
        // A base whose declaration surge resolved out of another declaration
        // file may not be the class the oracle sees; stay quiet rather than
        // guess.
        if info.file_name.ends_with(".d.ts") && *info.file_name != *ctx.file_name {
            return;
        }
        for member in &info.body.members {
            if satisfied.contains(&member.name) || !seen.insert(member.name.clone()) {
                continue;
            }
            if member.is_abstract {
                missing.push(member.name.clone());
            }
        }
        next_base = info.body.extends.first().map(|parent| parent.name.clone());
    }

    if missing.is_empty() {
        return;
    }

    // tsc names both classes by `typeToString` of their types: the base as the
    // heritage clause instantiates it, under the class's own type parameters
    // (`A<T>`), and a generic class as its declared type (`C<T>`). Resolving
    // the arguments is only for their names; the heritage check reports them.
    let base_display = if base.type_arguments.is_empty() {
        base.name.clone()
    } else {
        let checkpoint = ctx.diagnostics().len();
        let _type_variables =
            crate::checks::function::enter_body_type_variables(&class.type_parameters, ctx);
        let arguments: Vec<String> = crate::checks::function::with_type_parameter_scope(
            &class.type_parameters,
            ctx,
            |ctx| {
                base.type_arguments
                    .iter()
                    .map(|argument| map_parsed_type(argument.clone(), ctx).name())
                    .collect()
            },
        );
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        format!("{}<{}>", base.name, arguments.join(", "))
    };
    let class_display = if class.type_parameters.is_empty() {
        class.name.clone()
    } else {
        let parameters: Vec<&str> = class
            .type_parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect();
        format!("{}<{}>", class.name, parameters.join(", "))
    };
    let quoted = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    // A class expression is reported at itself and without a name
    // (`checkKindsOfPropertyMemberOverrides`, TS2653/TS2656/TS2650).
    if class.const_binding || class.name == "(Anonymous class)" {
        let diagnostic = match missing.len() {
            1 => Diagnostic::ts2653(&missing[0], &base_display, ctx.file_name.clone()),
            count if count <= MAX_LISTED => {
                Diagnostic::ts2656(&base_display, quoted(&missing), ctx.file_name.clone())
            }
            count => Diagnostic::ts2650(
                &base_display,
                quoted(&missing[..LISTED_WHEN_TRUNCATED]),
                count - LISTED_WHEN_TRUNCATED,
                ctx.file_name.clone(),
            ),
        };
        ctx.push(match class.span.or(class.name_span) {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
        return;
    }
    let diagnostic = match missing.len() {
        1 => Diagnostic::ts2515(
            &class_display,
            &missing[0],
            &base_display,
            ctx.file_name.clone(),
        ),
        count if count <= MAX_LISTED => Diagnostic::ts2654(
            &class_display,
            &base_display,
            quoted(&missing),
            ctx.file_name.clone(),
        ),
        count => Diagnostic::ts2655(
            &class_display,
            &base_display,
            quoted(&missing[..LISTED_WHEN_TRUNCATED]),
            count - LISTED_WHEN_TRUNCATED,
            ctx.file_name.clone(),
        ),
    };
    let diagnostic = match class.name_span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };
    ctx.push(diagnostic);
}

/// Members an `interface` merged with the class puts on its instance type,
/// its own and those its heritage brings: Go's
/// `checkKindsOfPropertyMemberOverrides` reads the derived member off that
/// type, so they implement an inherited abstract member too.
fn merged_interface_member_names(
    class: &ParsedClassDeclaration,
    base_name: &str,
    ctx: &CheckerContext,
) -> Vec<String> {
    let Some(TypeDeclarationInfo::Interface(own)) = ctx.lookup_type_declaration(&class.name) else {
        return Vec::new();
    };
    let mut names: Vec<String> = own
        .body
        .members
        .iter()
        .filter(|member| !member.is_abstract)
        .map(|member| member.name.clone())
        .collect();
    let mut pending: Vec<String> = own
        .body
        .extends
        .iter()
        .filter(|heritage| heritage.name != base_name)
        .map(|heritage| heritage.name.clone())
        .collect();
    let mut visited = std::collections::HashSet::new();
    while let Some(name) = pending.pop() {
        if visited.len() >= 32 || !visited.insert(name.clone()) {
            continue;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&name) else {
            continue;
        };
        names.extend(info.body.members.iter().map(|member| member.name.clone()));
        pending.extend(info.body.extends.iter().map(|heritage| heritage.name.clone()));
    }
    names
}

/// The name a class member declares when it is not itself abstract — what
/// implements an inherited abstract member of the same name.
fn implemented_member_name(member: &ParsedClassMember) -> Option<String> {
    match member {
        ParsedClassMember::Property(property) if !property.is_abstract && !property.is_static => {
            Some(property.name.clone())
        }
        ParsedClassMember::Method(method) if !method.is_abstract && !method.is_static => {
            Some(method.name.clone())
        }
        ParsedClassMember::Accessor(accessor) if !accessor.is_abstract && !accessor.is_static => {
            Some(accessor.name.clone())
        }
        _ => None,
    }
}

/// TS2420: a class has to declare every required member of each interface it
/// implements — an `implements` clause contributes nothing to the class, it
/// only constrains it.
///
/// The member-specific TS2416 walk runs first, then the missing members, then
/// the modifier rules of `propertyRelatedTo`. Interface resolution is
/// conservative in the same way the abstract-member check is: anything that
/// does not resolve to a source-declared interface leaves that clause
/// unchecked.
fn check_implemented_interfaces(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    if class.implements.is_empty() || class.is_declare {
        return;
    }

    let Some(declared) = declared_instance_member_names(class, ctx) else {
        // Part of the class's own surface is out of reach — a base class from a
        // declaration file, an unresolved heritage name — so a member that looks
        // missing may simply be inherited. Report nothing.
        return;
    };

    for implemented in &class.implements {
        // `implements string` is TS2864 alone: the clause's type is tsc's
        // error type, which no member check compares against.
        if is_primitive_type_name(&implemented.name) {
            continue;
        }
        // tsc reports the member-specific errors first and falls back to the
        // broad one only when that walk found nothing.
        if super::heritage::report_incompatible_heritage_members(class, implemented, ctx) {
            continue;
        }
        // tsc names a class it finds in the clause, merged with an interface or not.
        let implements_class = matches!(
            ctx.lookup_type_declaration(&implemented.name),
            Some(crate::symbols::TypeDeclarationInfo::Interface(info)) if info.is_class_instance
        );
        let Some(required) = unimplemented_interface_members(&implemented.name, &declared, ctx)
        else {
            continue;
        };
        if required.is_empty() && implements_weak_type_without_common_member(&implemented.name, &declared, ctx) {
            let diagnostic = Diagnostic::ts2559(&class.name, &implemented.name, ctx.file_name.clone());
            ctx.push(match class.name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
            continue;
        }
        if required.is_empty() && super::heritage::implements_unrelated_accessibility(class, implemented, ctx) {
            let diagnostic = if implements_class {
                Diagnostic::ts2720(&class.name, &implemented.name, ctx.file_name.clone())
            } else {
                Diagnostic::ts2420(&class.name, &implemented.name, ctx.file_name.clone())
            };
            ctx.push(match class.name_span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
            continue;
        }
        if required.is_empty() {
            super::heritage::report_incompatible_index_signatures(
                class,
                implemented,
                |own, base, file| {
                    if implements_class {
                        Diagnostic::ts2720(own, base, file)
                    } else {
                        Diagnostic::ts2420(own, base, file)
                    }
                },
                ctx,
            );
            continue;
        }
        let diagnostic = if implements_class {
            Diagnostic::ts2720(&class.name, &implemented.name, ctx.file_name.clone())
        } else {
            Diagnostic::ts2420(&class.name, &implemented.name, ctx.file_name.clone())
        };
        let diagnostic = match class.name_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };
        ctx.push(diagnostic);
    }
}

/// TS2416 for a member that overrides a base *class* member incompatibly.
///
/// Only the member-specific half of tsc's `extends` check runs here. The broad
/// TS2415 needs a whole-type relation against the base plus the missing-member
/// notion the `implements` path has, and surge's class instance surfaces are
/// not complete enough for that to be free of false positives.
fn check_extended_base_class(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    if class.is_declare {
        return;
    }
    for base in &class.extends {
        if base.name == surge_ts_syntax::EXPRESSION_HERITAGE_BASE {
            super::heritage::report_never_mixin_base(class, base, ctx);
            super::heritage::report_call_base_constructors(class, base, ctx);
            super::heritage::report_incompatible_heritage_members(class, base, ctx);
            continue;
        }
        // A base that names nothing is already reported, at its own span.
        if ctx.lookup_type_declaration(&base.name).is_none()
            && ctx.symbols.get(&base.name).is_none()
        {
            continue;
        }
        if super::heritage::report_incompatible_heritage_members(class, base, ctx)
            || super::heritage::report_incompatible_index_signatures(
                class,
                base,
                |own, base, file| Diagnostic::ts2415(own, base, file),
                ctx,
            )
        {
            continue;
        }
        if base.type_arguments.is_empty() {
            super::heritage::check_base_class_relation(class, &base.name, ctx);
        }
    }
}

/// Every instance member name the class has: what its body declares, what a
/// same-named interface merged into it declares, and what it inherits. `None`
/// when any part of that chain is not source-declared, which is what keeps an
/// inherited member surge cannot see from reading as a missing one.
fn declared_instance_member_names(
    class: &ParsedClassDeclaration,
    ctx: &CheckerContext,
) -> Option<std::collections::HashSet<String>> {
    let mut declared: std::collections::HashSet<String> = class
        .members
        .iter()
        .filter_map(declared_member_name)
        .collect();
    declared.extend(
        constructor_parameter_property_members(class)
            .into_iter()
            .map(|member| member.name),
    );

    // The instance-side declaration surge built for this class is where a
    // merged `interface C {}` of the same name lands, heritage included.
    let mut stack: Vec<String> = class
        .extends
        .iter()
        .map(|base| base.name.clone())
        .collect();
    if let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&class.name) {
        for member in &info.body.members {
            declared.insert(member.name.clone());
        }
        stack.extend(info.body.extends.iter().map(|base| base.name.clone()));
    }

    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(name) = stack.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&name) else {
            return None;
        };
        if info.file_name.ends_with(".d.ts") {
            return None;
        }
        for member in &info.body.members {
            declared.insert(member.name.clone());
        }
        stack.extend(info.body.extends.iter().map(|base| base.name.clone()));
    }

    Some(declared)
}

/// The required members of `interface_name` (its own and its bases') that
/// `declared` does not cover. `None` when the interface, or any interface it
/// extends, is not a source-declared object shape.
fn is_library_declaration_file(file_name: &str) -> bool {
    file_name.ends_with(".d.ts")
        && (file_name.starts_with("<surge-lib>") || file_name.contains("/node_modules/"))
}

fn unimplemented_interface_members(
    interface_name: &str,
    declared: &std::collections::HashSet<String>,
    ctx: &CheckerContext,
) -> Option<Vec<String>> {
    let mut required = Vec::new();
    let mut stack = vec![interface_name.to_string()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    while let Some(name) = stack.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        match ctx.lookup_type_declaration(&name) {
            Some(TypeDeclarationInfo::Interface(info)) => {
                // A library's interface may be merged or augmented where this
                // walk cannot see; the program's own declaration files are
                // read like any other source.
                if is_library_declaration_file(&info.file_name) {
                    return None;
                }
                for member in &info.body.members {
                    if !member.optional && !declared.contains(&member.name) {
                        required.push(member.name.clone());
                    }
                }
                stack.extend(info.body.extends.iter().map(|base| base.name.clone()));
            }
            // `implements SomeAlias` where the alias is written as an object
            // type is the same constraint; any other alias body (a union, a
            // conditional, a generic instantiation) is left alone.
            Some(TypeDeclarationInfo::Alias(alias))
                if alias.body.type_parameters.is_empty()
                    && !is_library_declaration_file(&alias.file_name) =>
            {
                let surge_ts_syntax::ParsedType::Object(object) = &alias.body.ty else {
                    return None;
                };
                for property in &object.properties {
                    if !property.optional && !declared.contains(&property.name) {
                        required.push(property.name.clone());
                    }
                }
            }
            _ => return None,
        }
    }

    Some(required)
}

/// tsc's weak-type check on `implements`: an interface (with its bases) whose
/// members are all optional and that has no signature shares none of them
/// with the class.
fn implements_weak_type_without_common_member(
    interface_name: &str,
    declared: &std::collections::HashSet<String>,
    ctx: &CheckerContext,
) -> bool {
    // `isRelatedTo` runs the common-property check only for a source with
    // properties or signatures: an empty class relates to any weak type.
    if declared.is_empty() {
        return false;
    }
    let mut members = Vec::new();
    let mut stack = vec![interface_name.to_string()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(name) = stack.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&name) else {
            return false;
        };
        let body = &info.body;
        if info.file_name.ends_with(".d.ts")
            || body.string_index_type.is_some()
            || body.number_index_type.is_some()
            || body.call_signature.is_some()
            || !body.construct_signatures.is_empty()
        {
            return false;
        }
        members.extend(body.members.iter().map(|member| (member.name.clone(), member.optional)));
        stack.extend(body.extends.iter().map(|base| base.name.clone()));
    }
    !members.is_empty()
        && members.iter().all(|(_, optional)| *optional)
        && !members.iter().any(|(name, _)| declared.contains(name))
}

/// Every instance member name a class body declares, `abstract` ones included:
/// an abstract member is a declaration, which is what an `implements` clause
/// asks for.
fn declared_member_name(member: &ParsedClassMember) -> Option<String> {
    match member {
        ParsedClassMember::Property(property) if !property.is_static => Some(property.name.clone()),
        ParsedClassMember::Method(method) if !method.is_static => Some(method.name.clone()),
        ParsedClassMember::Accessor(accessor) if !accessor.is_static => {
            Some(accessor.name.clone())
        }
        _ => None,
    }
}

/// `checkThisExpression` past a computed member name: `this` is the member's
/// own, the constructor for a static one and the instance otherwise.
fn computed_key_this_scope(
    class: &ParsedClassDeclaration,
    is_static: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> SymbolTable {
    let checkpoint = ctx.diagnostics().len();
    let this_type = if is_static {
        build_class_value_symbol(class, ctx).ty
    } else {
        class_instance_type(class, ctx)
    };
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let mut scope = symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let _ = scope.insert(
        "this".to_string(),
        SymbolInfo {
            ty: this_type,
            kind: SymbolKind::Const,
            function_signature: None,
        },
    );
    scope
}

/// What a class declaration evaluates outside its body: its members'
/// computed names and the decorators tsc checks.
pub(crate) fn check_class_head_expressions(
    class: &ParsedClassDeclaration,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    // The class's own type parameters are found from a computed name (the
    // binder then rejects the reference as TS2467, which the grammar pass
    // reports), so they resolve here instead of reading as unknown names.
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        let ambient = class.is_declare || crate::modules::is_declaration_file_name(&ctx.file_name);
        let mut member_this: [Option<SymbolTable>; 2] = [None, None];
        for (index, (key, span)) in class.computed_keys.iter().enumerate() {
            let non_emitting =
                ambient || class.non_emitting_computed_keys.get(index).copied().unwrap_or(false);
            let key_symbols = match class.this_computed_keys.get(index).copied().flatten() {
                Some(is_static) => &*member_this[usize::from(is_static)]
                    .get_or_insert_with(|| computed_key_this_scope(class, is_static, symbols, ctx)),
                None => symbols,
            };
            let saved = std::mem::replace(&mut ctx.type_only_alias_use_valid, non_emitting);
            let key_type = crate::checks::expr::evaluate_expression(key, *span, key_symbols, ctx);
            ctx.type_only_alias_use_valid = saved;
            crate::checks::expr::report_invalid_computed_key(&key_type, *span, ctx);
        }
        // tsc's `checkDecorators`: the expression of every decorator on a
        // declaration that can be decorated. How the decorator is then called
        // (TS1238, TS1270, ...) is not modelled, and neither is the contextual
        // type that call gives the expression (`getContextualTypeForDecorator`),
        // so a function written as the decorator is checked as one whose
        // contextual type surge does not know.
        let legacy_decorators = ctx.options.experimental_decorators;
        let class_type = symbols.get(&class.name).map(|symbol| symbol.ty.clone());
        for decorator in &class.decorators {
            if !super::forward_references::decorator_is_checked(decorator.target, legacy_decorators) {
                continue;
            }
            if decorator.needs_parentheses {
                ctx.push(crate::spans::diagnostic_with_syntax_span(
                    Diagnostic::ts1497(ctx.file_name.clone()),
                    decorator.span,
                ));
            }
            let contextually_typed = matches!(decorator.expression, surge_ts_syntax::ParsedExpression::ArrowFunction(_));
            if contextually_typed {
                ctx.degraded_expected_type_depth += 1;
            }
            let decorator_type = crate::checks::expr::evaluate_expression(&decorator.expression, decorator.span, symbols, ctx);
            if contextually_typed {
                ctx.degraded_expected_type_depth -= 1;
            }
            if let crate::infer::InferredExpression::Known(decorator_type) = &decorator_type {
                check_decorator_call(decorator_type, decorator, legacy_decorators, class_type.as_ref(), ctx);
            }
        }
    });
    // `checkClassLikeDeclaration` checks the `extends` expression like any
    // other, so a call there has its arguments checked. A method call is left
    // out: a static inherited through another expression base is one surge's
    // static side may lack.
    if let Some(expression) = class.heritage_expression.as_deref()
        && matches!(
            expression,
            surge_ts_syntax::ParsedExpression::Call { .. } | surge_ts_syntax::ParsedExpression::ExpressionCall { .. }
        )
    {
        let _ = crate::checks::expr::evaluate_expression(expression, None, symbols, ctx);
    }
}

/// tsc's `resolveDecorator`, for what surge models of a decorator call: one
/// that takes too few parameters to be applied uncalled
/// (`isPotentiallyUncalledDecorator`, TS1329), and one no signature of which
/// accepts the arguments the runtime passes (`hasCorrectArity`; TS1238 for
/// a class decorator, TS1239-TS1241 for a parameter, property or method). A
/// signature that fits the count but not the types is not checked.
fn check_decorator_call(
    decorator_type: &Type,
    decorator: &surge_ts_syntax::ParsedDecorator,
    legacy_decorators: bool,
    class_type: Option<&Type>,
    ctx: &mut CheckerContext,
) {
    use surge_ts_syntax::ParsedDecoratorTarget as Target;
    let head_diagnostic = |ctx: &CheckerContext| {
        let file_name = ctx.file_name.clone();
        match decorator.target {
            Target::Class => Diagnostic::ts1238(file_name),
            Target::Parameter => Diagnostic::ts1239(file_name),
            Target::Property { .. } => Diagnostic::ts1240(file_name),
            Target::Method { .. } => Diagnostic::ts1241(file_name),
        }
    };
    let peeled = decorator_type.peeled();
    // A decorator with no call signature (a class, a primitive) fails before
    // any arity is weighed; tsc reports it on the expression.
    let not_callable = match &peeled {
        Type::Object(object) => object.construct_signature().is_some() && object.call_signature().is_none(),
        Type::Number
        | Type::String
        | Type::Boolean
        | Type::BigInt
        | Type::NumberLiteral(_)
        | Type::StringLiteral(_)
        | Type::BooleanLiteral(_) => true,
        _ => false,
    };
    if not_callable {
        let diagnostic = head_diagnostic(ctx);
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, decorator.span));
        return;
    }
    let Type::Function(function) = peeled else {
        return;
    };
    let mut signatures = Vec::new();
    function.push_overload_members(&mut signatures);
    if signatures.is_empty() {
        return;
    }
    // `getDecoratorArgumentCount`.
    let argument_count = |signature: &surge_ts_types::FunctionType| {
        let parameters = signature.parameters().len();
        if !legacy_decorators {
            return parameters.clamp(1, 2);
        }
        match decorator.target {
            Target::Class => 1,
            Target::Property { is_auto_accessor, .. } => if is_auto_accessor { 3 } else { 2 },
            Target::Method { .. } => if parameters <= 2 { 2 } else { 3 },
            Target::Parameter => 3,
        }
    };
    let potentially_uncalled = signatures.iter().all(|signature| {
        surge_ts_types::min_argument_count(signature) == 0
            && !signature.is_variadic()
            && signature.parameters().len() < argument_count(signature)
    });
    if potentially_uncalled && !decorator.is_parenthesized {
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            Diagnostic::ts1329(&decorator.expression_text, ctx.file_name.clone()),
            decorator.decorator_span,
        ));
        return;
    }
    let too_many = |signature: &surge_ts_types::FunctionType| {
        !signature.is_variadic() && argument_count(signature) > signature.parameters().len()
    };
    let has_correct_arity = |signature: &surge_ts_types::FunctionType| {
        !too_many(signature) && argument_count(signature) >= surge_ts_types::min_argument_count(signature)
    };
    if signatures.iter().any(has_correct_arity) {
        if legacy_decorators
            && let [signature] = signatures.as_slice()
        {
            check_legacy_decorator_return(signature, decorator, class_type, ctx);
        }
        return;
    }
    let diagnostic = head_diagnostic(ctx);
    // `getArgumentArityError`: arguments beyond every signature are reported
    // where they would be, the decorator's expression; too few, on the call.
    let span = if signatures.iter().all(too_many) { decorator.span } else { decorator.decorator_span };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
}

/// tsc's `checkDecorator` return check under `experimentalDecorators`: what a
/// class decorator returns replaces the class (`typeof C | void`, TS1270), and
/// a property or parameter decorator returns nothing (`void`, TS1271). A
/// method decorator returns `TypedPropertyDescriptor<T> | void` for the
/// member's type `T`; a return that `TypedPropertyDescriptor<any>` rejects is
/// rejected for every `T`, which is all that is decided here. A return type
/// that depends on inference is not decided.
fn check_legacy_decorator_return(
    signature: &surge_ts_types::FunctionType,
    decorator: &surge_ts_syntax::ParsedDecorator,
    class_type: Option<&Type>,
    ctx: &mut CheckerContext,
) {
    use surge_ts_syntax::ParsedDecoratorTarget as Target;
    let return_type = signature.return_type();
    if matches!(return_type, Type::Any | Type::ErrorType)
        || signature.type_parameter_head().is_some()
        || crate::checks::function::type_contains_degradation(return_type)
    {
        return;
    }
    let file_name = ctx.file_name.clone();
    let diagnostic = match decorator.target {
        Target::Class => {
            let Some(class_type) = class_type else {
                return;
            };
            let expected = surge_ts_types::union_type(vec![Type::Void, class_type.clone()]);
            if surge_ts_types::is_assignable_to(return_type, &expected) {
                return;
            }
            Diagnostic::ts1270(return_type.name(), expected.name(), file_name)
        }
        Target::Property { .. } | Target::Parameter => {
            if surge_ts_types::is_assignable_to(return_type, &Type::Void) {
                return;
            }
            Diagnostic::ts1271(return_type.name(), file_name)
        }
        Target::Method { .. } => {
            let checkpoint = ctx.diagnostics().len();
            let descriptor = crate::infer::map_parsed_type(
                ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
                    name: "TypedPropertyDescriptor".to_string(),
                    span: None,
                    type_arguments: vec![ParsedType::Any],
                    type_argument_spans: Vec::new(),
                })),
                ctx,
            );
            ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
            if descriptor.is_unmodelled() {
                return;
            }
            let expected = surge_ts_types::union_type(vec![Type::Void, descriptor]);
            if surge_ts_types::is_assignable_to(return_type, &expected) {
                return;
            }
            Diagnostic::ts1270(return_type.name(), expected.name(), file_name)
        }
    };
    ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, decorator.span));
}

pub(crate) fn check_class_declaration(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    check_class_declaration_parts(class, true, None, ctx);
}

/// A class declared in a function body, checked once the body is bound, as a
/// nested function is: its members run later and see every enclosing binding
/// at its declared type. Its heritage and computed names were checked where
/// its statement is.
pub(crate) fn check_nested_class_declaration(
    class: &ParsedClassDeclaration,
    enclosing: &crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    if class.is_declare {
        return;
    }
    let symbols = enclosing.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let saved_symbols = std::mem::replace(&mut ctx.symbols, symbols);
    let scopes = ClassMemberScopes {
        narrowed: None,
        contextual_type: None,
    };
    check_class_declaration_parts(class, false, Some(&scopes), ctx);
    ctx.symbols = saved_symbols;
}

/// The scopes a class checked where it is written — a class expression, or a
/// class declared in a function body — gives its member bodies. Without them
/// a body is rooted at the module (`merged_function_body_root_symbols`) and
/// loses every binding of an enclosing function but the innermost's.
struct ClassMemberScopes<'a> {
    /// A class expression's scope, narrowing kept, for the members that
    /// continue the enclosing flow.
    narrowed: Option<&'a SymbolTable>,
    /// A class expression's contextual type.
    contextual_type: Option<&'a Type>,
}

impl ClassMemberScopes<'_> {
    /// The scope `member`'s body reads outer names from. tsc continues the
    /// enclosing flow into a class expression's method or accessor
    /// (`checkIdentifier` widens the flow container past
    /// `IsObjectLiteralOrClassExpressionMethodOrAccessor`) and evaluates a
    /// static block in place; any other member reads each binding at its
    /// declared type, which is the scope the class is checked in.
    fn body_scope(&self, member: &ParsedClassMember, ctx: &CheckerContext) -> Arc<SymbolTable> {
        let continues_flow = matches!(
            member,
            ParsedClassMember::Method(_) | ParsedClassMember::Accessor(_) | ParsedClassMember::StaticBlock(_)
        );
        let Some(narrowed) = self.narrowed.filter(|_| continues_flow) else {
            return Arc::new(ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext));
        };
        let mut scope = narrowed.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        match ctx.symbols.get_own_handle("super") {
            Some(base) => {
                let _ = scope.insert_handle("super", base);
            }
            None => {
                let _ = scope.remove("super");
            }
        }
        Arc::new(scope)
    }

    /// tsc's `getContextualTypeForStaticPropertyDeclaration`: an unannotated
    /// static property of a class expression takes its initializer's
    /// contextual type from the class's.
    fn static_property_context(&self, property: &ParsedClassProperty) -> Option<Type> {
        if !property.is_static || property.declared_type.is_some() {
            return None;
        }
        surge_ts_types::remove_nullish(self.contextual_type?).get_property_access_type(&property.name)
    }
}

/// tsc's `checkClassExpression`: the class is checked where it is evaluated,
/// as a declaration is, and evaluates to its constructor. `contextual_type`
/// is the expression's own contextual type.
pub(crate) fn check_class_expression(
    class_expression: &surge_ts_syntax::ParsedClassExpression,
    contextual_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let class = &class_expression.class;
    let saved_scope = crate::checks::function::install_class_expression_type_declaration(class, ctx);
    let value = class_expression_value(class, symbols, ctx);
    let class_type = value.ty.clone();
    // A name the class writes for itself is bound inside it alone.
    let mut narrowed = symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    if class_expression.has_own_name {
        let _ = narrowed.insert(class.name.clone(), value);
    }
    let declared = crate::checks::function::declared_type_view(&narrowed);
    let saved_symbols = std::mem::replace(&mut ctx.symbols, declared);
    // What the class reports is its own, not a verdict on the value it is.
    let saved_return_check = std::mem::replace(&mut ctx.in_contextual_return_check, false);
    // tsc's `nodeCanBeDecorated`: under legacy decorators nothing in a class
    // expression can be decorated, so no decorator in it is checked.
    let undecorated;
    let head = if ctx.options.experimental_decorators && !class.decorators.is_empty() {
        undecorated = ParsedClassDeclaration {
            decorators: Vec::new(),
            ..class.clone()
        };
        &undecorated
    } else {
        class
    };
    let head_symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    check_class_head_expressions(head, &head_symbols, ctx);
    crate::flow::check_class_member_flow(class, ctx);
    let scopes = ClassMemberScopes {
        narrowed: Some(&narrowed),
        contextual_type,
    };
    check_class_declaration_parts(class, false, Some(&scopes), ctx);
    ctx.in_contextual_return_check = saved_return_check;
    ctx.symbols = saved_symbols;
    ctx.type_declaration_scope = saved_scope;
    class_type
}

/// The constructor a class expression evaluates to, read without checking
/// the class.
pub(crate) fn class_expression_type(
    class_expression: &surge_ts_syntax::ParsedClassExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let class = &class_expression.class;
    let saved_scope = crate::checks::function::install_class_expression_type_declaration(class, ctx);
    let class_type = class_expression_value(class, symbols, ctx).ty;
    ctx.type_declaration_scope = saved_scope;
    class_type
}

/// A class expression's value side. The scope it is evaluated in is the value
/// table a base in its `extends` clause is read from, for the static side and
/// the instance's heritage both; building it reports nothing.
fn class_expression_value(
    class: &ParsedClassDeclaration,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> SymbolInfo {
    let saved_symbols = std::mem::replace(
        &mut ctx.symbols,
        symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    let checkpoint = ctx.diagnostics().len();
    let mut value = build_class_value_symbol_with_scope(class, Some(symbols), ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    ctx.symbols = saved_symbols;
    // In JavaScript a class expression is an expando container (`C.x = 0`
    // declares a static member), which surge does not collect for it: its
    // static side stays open.
    if surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && let Type::Object(object) = &value.ty
    {
        let mut open = object.clone().with_open_index_marker();
        open.string_index_type = Some(std::sync::Arc::new(Type::Any));
        value.ty = Type::Object(open);
    }
    value
}

fn check_class_declaration_parts(
    class: &ParsedClassDeclaration,
    check_head: bool,
    scopes: Option<&ClassMemberScopes<'_>>,
    ctx: &mut CheckerContext,
) {
    crate::checks::function::check_type_parameter_declarations(&class.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        for member in &class.members {
            if let ParsedClassMember::Method(method) = member {
                crate::checks::function::check_type_parameter_declarations(
                    &method.type_parameters,
                    ctx,
                );
            }
        }
    });
    super::heritage::report_circular_base(&class.name, class.name_span, &class.type_parameters, true, ctx);
    if check_head {
        let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        check_class_head_expressions(class, &symbols, ctx);
    }
    // Everything checked from here on is lexically inside the class, which is
    // what decides whether its `private`/`protected` members are reachable.
    let lineage = crate::checks::expr::enclosing_class_lineage(class, ctx);
    ctx.enclosing_classes.push(lineage);
    check_class_declaration_inside(class, scopes, ctx);
    ctx.enclosing_classes.pop();
}

/// tsc's `isAssignmentToReadonlyEntity` lets a constructor write the `readonly`
/// members its own class declares — a property declaration or a parameter
/// property — and nothing inherited.
fn constructor_writable_members(
    class: &ParsedClassDeclaration,
    constructor: &surge_ts_syntax::ParsedClassConstructor,
) -> Vec<String> {
    let properties = class.members.iter().filter_map(|member| match member {
        ParsedClassMember::Property(property) if !property.is_static => Some(property.name.clone()),
        _ => None,
    });
    let parameter_properties =
        constructor
            .parameters
            .iter()
            .filter_map(|parameter| match &parameter.binding_name {
                surge_ts_syntax::ParsedBindingName::Identifier { name, .. }
                    if parameter.is_parameter_property =>
                {
                    Some(name.clone())
                }
                _ => None,
            });
    properties.chain(parameter_properties).collect()
}

/// A class base is an expression: tsc resolves `extends X` as a value, so the
/// name is unresolved only when it is neither a type surge bound the class to
/// nor a value in scope. Signature collection leaves this report to here,
/// where every value declared before the class is bound.
fn check_heritage_base_resolves(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    for base in &class.extends {
        if base.name == surge_ts_syntax::EXPRESSION_HERITAGE_BASE {
            if let Some(literal) = class.heritage_expression.as_deref().and_then(literal_base_type) {
                report_non_constructor_base(&literal, base, ctx);
            }
            continue;
        }
        // The base is a value read: a name bound through a type-only import
        // or export is TS1361/TS1362 (`checkExpressionWorker`'s alias check).
        let head = base.name.split('.').next().unwrap_or(&base.name);
        // An ambient class's base is no emitted read (`isValidTypeOnlyAliasUseSite`);
        // a namespace member's ambience is not recorded on the class itself.
        let ambient = class.is_declare
            || crate::modules::is_declaration_file_name(&ctx.file_name)
            || !ctx.namespace_member_prefix_stack.is_empty();
        if let Some(kind) = ctx
            .type_only_value_reference(head)
            .filter(|_| !ctx.type_only_alias_use_valid && !ambient)
        {
            let diagnostic = match kind {
                crate::modules::TypeOnlyAliasKind::Import => Diagnostic::ts1361(head, ctx.file_name.clone()),
                crate::modules::TypeOnlyAliasKind::Export => Diagnostic::ts1362(head, ctx.file_name.clone()),
            };
            let span = base.span.map(|span| surge_ts_syntax::TextSpan { start: span.start, end: span.start + head.len() });
            ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, span));
            continue;
        }
        if let Some((head, member)) = base.name.split_once('.') {
            if check_qualified_base_head_resolves(head, base.span, ctx) {
                check_namespace_base_member_exists(head, member, base.span, ctx);
            }
            continue;
        }
        if ctx.symbols.get(&base.name).is_some() {
            check_base_is_constructor_type(base, ctx);
            continue;
        }
        if base.name == "undefined" {
            report_non_constructor_base(&Type::Undefined, base, ctx);
            continue;
        }
        // tsc's `checkAndReportErrorForUsingTypeAsValue`: a primitive keyword
        // in an `extends` clause is TS2863 instead of an unresolved name.
        if is_primitive_type_name(&base.name) {
            let diagnostic = Diagnostic::ts2863(&base.name, ctx.file_name.clone());
            ctx.push(match base.span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
            continue;
        }
        match ctx.lookup_type_declaration(&base.name) {
            // tsc's `checkAndReportErrorForExtendingInterface`: the base
            // expression names an interface and no value.
            Some(TypeDeclarationInfo::Interface(info)) if !info.is_class_instance => {
                let diagnostic = Diagnostic::ts2689(&base.name, ctx.file_name.clone());
                let diagnostic = match base.span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                };
                ctx.push(diagnostic);
            }
            Some(_) => {}
            None => {
                let _ = crate::infer::types::emit_unknown_type_name(base, ctx);
            }
        }
    }
    // An `implements` clause is a type reference, resolved (and reported) as
    // one.
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        for implemented in &class.implements {
            if is_primitive_type_name(&implemented.name) {
                let diagnostic = Diagnostic::ts2864(&implemented.name, ctx.file_name.clone());
                ctx.push(match implemented.span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                });
                continue;
            }
            let _ = crate::infer::map_parsed_type(
                ParsedType::Named(Arc::new(implemented.clone())),
                ctx,
            );
        }
    });
}

/// The head of a qualified class base (`extends M.C`) is resolved as a value,
/// as tsc's `checkExpression` does: a namespace with no value is TS2708, and a
/// name that is no value at all fails like any unresolved value reference.
/// Answers whether the head resolved.
fn check_qualified_base_head_resolves(
    head: &str,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    let head_span = span.map(|span| surge_ts_syntax::TextSpan {
        start: span.start,
        end: span.start + head.len(),
    });
    if ctx.namespace_meaning(head) == Some(false) {
        let diagnostic = Diagnostic::ts2708(head, ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, head_span));
        return false;
    }
    if ctx.symbols.get(head).is_some()
        || ctx.namespace_meaning(head).is_some()
        || ctx.is_import_binding(head)
        || ctx.is_umd_global_value_reference(head)
        || ctx.ambient_global_symbols.get(head).is_some()
    {
        return true;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    crate::checks::expr::report_unresolved_value_name(
        head,
        head_span,
        crate::checks::expr::UnresolvedNameSite::Reference,
        &symbols,
        ctx,
    );
    false
}

/// `extends M.C` reads `C` off the namespace value: a member it does not
/// export is TS2339 on `typeof M`. Only a namespace head is looked at, and the
/// read is only asked whether the member is missing.
fn check_namespace_base_member_exists(
    head: &str,
    member: &str,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    if member.contains('.') || ctx.namespace_meaning(head) != Some(true) {
        return;
    }
    let head_span = span.map(|span| surge_ts_syntax::TextSpan { start: span.start, end: span.start + head.len() });
    let member_span = span.map(|span| {
        let start = span.start + head.len() + 1;
        surge_ts_syntax::TextSpan { start, end: start + member.len() }
    });
    let access = surge_ts_syntax::ParsedExpression::PropertyAccess {
        object: Box::new(surge_ts_syntax::ParsedExpression::Identifier { name: head.to_string(), span: head_span }),
        object_span: head_span,
        property_name: member.to_string(),
        property_span: member_span,
        is_bracketed: false,
        binding_element: false,
    };
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let _ = crate::checks::expr::evaluate_expression(&access, span, &symbols, ctx);
    let missing = ctx.diagnostics()[checkpoint..]
        .iter()
        .any(|diagnostic| matches!(diagnostic.code, surge_ts_diagnostics::DiagnosticCode::TypeScript(2339)));
    ctx.truncate_diagnostics(checkpoint);
    if missing {
        let diagnostic = Diagnostic::ts2339(member, format!("typeof {head}"), ctx.file_name.clone());
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, member_span));
    }
}

/// tsc's `isPrimitiveTypeName`: the keywords `resolveName` reports specially
/// when they are used as values.
fn is_primitive_type_name(name: &str) -> bool {
    matches!(name, "any" | "string" | "number" | "boolean" | "never" | "unknown")
}

/// tsc's `getBaseConstructorTypeOfClass`: a base that names a value must be
/// constructable (TS2507). A class is always one, so only a name with no type
/// declaration is looked at, through the value it binds; anything surge could
/// not type, a type variable (a mixin base) and an unexpanded reference are
/// left alone.
/// `resolveBaseTypesOfClass`: a base naming no type whose value has no
/// construct signature — a plain function (TS2507) — gives the class no base
/// at all, and so no inherited members or constructor.
pub(crate) fn base_value_is_no_constructor(
    base: &ParsedNamedType,
    scope: Option<&SymbolTable>,
    ctx: &CheckerContext,
) -> bool {
    if base.name.contains('.') || ctx.lookup_type_declaration(&base.name).is_some() {
        return false;
    }
    let Some(symbol) = scope.and_then(|scope| scope.get(&base.name)).or_else(|| ctx.symbols.get(&base.name)) else {
        return false;
    };
    match symbol.ty.peeled() {
        Type::Function(_) => true,
        Type::Object(object) => object.call_signature().is_some() && object.construct_signature().is_none(),
        _ => false,
    }
}

fn check_base_is_constructor_type(base: &ParsedNamedType, ctx: &mut CheckerContext) {
    let Some(symbol) = ctx.symbols.get(&base.name) else {
        return;
    };
    let ty = symbol.ty.peeled();
    // A value of primitive type can only be a nearer binding shadowing the
    // type declaration (`namespace M { var C = 1; class D extends C {} }`).
    let primitive = matches!(
        ty,
        Type::Number
            | Type::NumberLiteral(_)
            | Type::String
            | Type::StringLiteral(_)
            | Type::Boolean
            | Type::BooleanLiteral(_)
            | Type::BigInt
            | Type::Symbol
    );
    if !primitive && ctx.lookup_type_declaration(&base.name).is_some() {
        return;
    }
    let constructable = match &ty {
        Type::Any
        | Type::ErrorType
        | Type::Null
        | Type::TypeParameter(_)
        | Type::Reference(_)
        | Type::Union(_) => true,
        Type::Object(object) => object.construct_signature().is_some(),
        _ if ty.is_unknown() => true,
        _ => false,
    };
    if constructable {
        return;
    }
    report_non_constructor_base(&ty, base, ctx);
}

/// The type `checkExpression` gives a literal `extends` expression.
fn literal_base_type(expression: &surge_ts_syntax::ParsedExpression) -> Option<Type> {
    use surge_ts_syntax::ParsedExpression as Expression;
    match expression {
        Expression::StringLiteral(text) => Some(Type::StringLiteral(text.clone())),
        Expression::NumberLiteral(text) => Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType {
            value: text.clone(),
        })),
        Expression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
        _ => None,
    }
}

fn report_non_constructor_base(ty: &Type, base: &ParsedNamedType, ctx: &mut CheckerContext) {
    let diagnostic = Diagnostic::ts2507(ty.name(), ctx.file_name.clone());
    ctx.push(match base.span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// tsc's `checkBaseTypeAccessibility`: a class whose constructor is private
/// can be extended only from inside its own body (TS2675).
fn check_base_constructor_accessibility(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    for base in &class.extends {
        let Some(TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(&base.name)
        else {
            continue;
        };
        if !info.is_class_instance {
            continue;
        }
        let info = info.clone();
        let Some((declaring, _)) =
            crate::checks::expr::constructor_accessibility_error(&info, false, ctx)
        else {
            continue;
        };
        let name = declaring
            .declared_name
            .as_deref()
            .unwrap_or(&declaring.name)
            .to_string();
        let diagnostic = Diagnostic::ts2675(name, ctx.file_name.clone());
        ctx.push(match base.span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

fn check_class_declaration_inside(
    class: &ParsedClassDeclaration,
    scopes: Option<&ClassMemberScopes<'_>>,
    ctx: &mut CheckerContext,
) {
    check_heritage_base_resolves(class, ctx);
    super::late_bound_members::check_late_bound_members(class, ctx);
    check_base_constructor_accessibility(class, ctx);
    check_inherited_abstract_members(class, ctx);
    check_extended_base_class(class, ctx);
    check_implemented_interfaces(class, ctx);
    super::property_initialization::check_property_initialization(class, ctx);
    super::forward_references::check_class_property_initializers(class, ctx);
    super::forward_references::check_circular_property_initializers(class, ctx);
    super::statements::check_self_indexed_member_annotations(
        &class.name,
        class.members.iter().filter_map(|member| match member {
            ParsedClassMember::Property(property) if !property.is_static => property
                .declared_type
                .as_ref()
                .map(|ty| (property.name.as_str(), property.name_span, ty)),
            _ => None,
        }),
        ctx,
    );
    super::override_modifiers::check_members_for_override_modifier(class, ctx);
    super::index_constraints::check_class_index_constraints(class, ctx);
    super::index_constraints::check_class_static_index_constraints(class, ctx);

    // Ambient classes have no bodies. Definite assignment needs no member
    // types, so they still get it.
    if class.is_declare {
        crate::flow::check_class_member_flow(class, ctx);
        report_ambient_member_implicit_any(class, ctx);
        return;
    }

    // A class type parameter is in scope throughout its members, so the bodies
    // are checked under it; without the scope every annotation naming one
    // resolves to nothing.
    let _type_variables = crate::checks::function::enter_body_type_variables(&class.type_parameters, ctx);
    crate::checks::function::with_type_parameter_scope(&class.type_parameters, ctx, |ctx| {
        check_property_redeclaring_accessor(class, ctx);
        check_class_member_bodies(class, scopes, ctx)
    });
}

/// `checkVariableLikeDeclaration` for a property declared after an accessor of
/// the same name: a property excludes nothing, so the binder merges it into
/// the accessor's symbol and it is a subsequent declaration, whose type must
/// be the accessor's (TS2717). Declared the other way round, the accessor gets
/// a symbol of its own.
fn check_property_redeclaring_accessor(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    let mut accessors: Vec<&ParsedClassAccessor> = Vec::new();
    for member in &class.members {
        match member {
            ParsedClassMember::Accessor(accessor) => accessors.push(accessor),
            ParsedClassMember::Property(property) if !property.is_auto_accessor => {
                let Some(accessor) = accessors
                    .iter()
                    .find(|accessor| accessor.name == property.name && accessor.is_static == property.is_static)
                else {
                    continue;
                };
                let Some(accessor_type) =
                    accessor.getter_return_type.clone().or_else(|| accessor.setter_param_type.clone())
                else {
                    continue;
                };
                let property_type = match (&property.declared_type, &property.initializer) {
                    (Some(declared), _) => declared.clone(),
                    (None, Some(initializer)) => match syntactic_initializer_type(initializer, property.readonly) {
                        Some(inferred) => inferred,
                        None => continue,
                    },
                    (None, None) => ParsedType::Any,
                };
                let accessor_type = crate::infer::map_parsed_type(accessor_type, ctx);
                let property_type = crate::infer::map_parsed_type(property_type, ctx);
                if accessor_type.is_unmodelled()
                    || property_type.is_unmodelled()
                    || surge_ts_types::is_type_identical_to(&accessor_type, &property_type)
                {
                    continue;
                }
                let diagnostic = Diagnostic::ts2717(
                    &property.name,
                    accessor_type.name(),
                    property_type.name(),
                    ctx.file_name.clone(),
                );
                ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, property.name_span));
            }
            _ => {}
        }
    }
}

/// tsc's implicit-any report on an ambient class's member parameters, which
/// spares only a private member's (`isPrivateWithinAmbient`). The signatures
/// are mapped for that report alone.
fn report_ambient_member_implicit_any(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    if !ctx.options.no_implicit_any {
        return;
    }
    let is_private = |name: &str, is_static: bool| {
        name.starts_with('#')
            || class.restricted_members.iter().any(|member| {
                member.name == name
                    && member.is_static == is_static
                    && matches!(member.accessibility, surge_ts_syntax::ParsedMemberAccessibility::Private)
            })
    };
    let checkpoint = ctx.diagnostics().len();
    for member in &class.members {
        match member {
            ParsedClassMember::Method(method) if !is_private(&method.name, method.is_static) => {
                let _ = crate::checks::function::map_function_signature(
                    &method.parameters,
                    method.return_type.as_ref(),
                    &method.type_parameters,
                    None,
                    ctx,
                );
            }
            ParsedClassMember::Constructor(constructor)
                if !matches!(constructor.accessibility, Some(surge_ts_syntax::ParsedMemberAccessibility::Private)) =>
            {
                let _ = crate::checks::function::map_function_signature(&constructor.parameters, None, &[], None, ctx);
            }
            _ => {}
        }
    }
    let reported: Vec<Diagnostic> = ctx.diagnostics()[checkpoint..].to_vec();
    ctx.truncate_diagnostics(checkpoint);
    for diagnostic in reported {
        if matches!(
            diagnostic.code,
            surge_ts_diagnostics::DiagnosticCode::TypeScript(7006 | 7019 | 7031)
        ) {
            ctx.push(diagnostic);
        }
    }
}

fn check_class_member_bodies(
    class: &ParsedClassDeclaration,
    scopes: Option<&ClassMemberScopes<'_>>,
    ctx: &mut CheckerContext,
) {
    // Building the class's own instance and static types is this check's
    // lookup, not a use of the class: a generic class referred to without
    // arguments here would report TS2314 against its own declaration.
    let checkpoint = ctx.diagnostics().len();
    let instance_type = class_instance_type(class, ctx);
    let static_value = build_class_value_symbol(class, ctx);
    let member_prefix_static_type = if class.type_parameters.is_empty() {
        static_value.ty.clone()
    } else {
        generic_class_static_side(class, &instance_type, ctx)
    };
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    // Signature collection built this class's value before its file's `const`s
    // were bound, so a base that is one of them left the instance open and the
    // constructor unknown. Here the base is bound: the value built now is the
    // one later statements should read.
    if class.extends.first().is_some_and(|base| {
        ctx.lookup_type_declaration(&base.name).is_none() && ctx.symbols.get(&base.name).is_some()
    }) {
        let _ = ctx.symbols.insert(class.name.clone(), static_value.clone());
    }
    let static_type = static_value.ty;

    // `super` in a member body reads the base class: its instance for an
    // instance member, its constructor for a static one.
    let super_types = class.extends.first().and_then(|base| {
        ctx.lookup_type_declaration(&base.name)?;
        let checkpoint = ctx.diagnostics().len();
        let instance = map_parsed_type(ParsedType::Named(Arc::new(base.clone())), ctx);
        ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
        let constructor = ctx.symbols.get(&base.name).map(|symbol| symbol.ty.clone())?;
        (!instance.is_unknown() && !matches!(instance, Type::Any))
            .then_some((instance, constructor))
    });
    let previous_super = ctx.symbols.remove("super");
    ctx.enclosing_class_members
        .push(crate::checks::expr::EnclosingClassMembers {
            class_name: class.name.clone(),
            instance_type: instance_type.clone(),
            static_type: member_prefix_static_type,
        });

    let class_type_parameter_depth = ctx
        .type_parameter_scopes
        .len()
        .saturating_sub(usize::from(!class.type_parameters.is_empty()));
    let class_type_parameter_names: Vec<String> = class
        .type_parameters
        .iter()
        .map(|type_parameter| type_parameter.name.clone())
        .collect();

    for member in &class.members {
        let _this_class = crate::checks::expr::ThisParameterClassScope::enter(match member {
            ParsedClassMember::Method(method) => crate::checks::expr::this_parameter_class(
                method.this_parameter_type.as_ref(),
                &method.type_parameters,
                ctx,
            ),
            _ => None,
        });
        let static_span = match member {
            ParsedClassMember::Method(method) => method.is_static.then_some(method.span),
            ParsedClassMember::Property(property) => property.is_static.then_some(property.span),
            ParsedClassMember::Accessor(accessor) => accessor.is_static.then_some(accessor.span),
            ParsedClassMember::StaticBlock(block) => Some(block.span),
            ParsedClassMember::Constructor(_) => None,
        };
        let is_static = static_span.is_some();
        ctx.static_member_type_parameters = static_span
            .filter(|_| !class_type_parameter_names.is_empty())
            .map(|span| {
                (
                    span,
                    class_type_parameter_depth,
                    class_type_parameter_names.clone(),
                )
            });
        if let Some((instance, constructor)) = &super_types {
            let _ = ctx.symbols.insert(
                "super",
                SymbolInfo {
                    ty: if is_static { constructor.clone() } else { instance.clone() },
                    kind: SymbolKind::Const,
                    function_signature: None,
                },
            );
        }
        let body_scope = scopes.map(|scopes| scopes.body_scope(member, ctx));
        match member {
            ParsedClassMember::Constructor(constructor) => {
                let function_type =
                    map_function_signature(&constructor.parameters, None, &[], None, ctx);
                ctx.constructor_writable_members =
                    Some(constructor_writable_members(class, constructor));
                let outer_super_constructor = std::mem::replace(
                    &mut ctx.super_constructor_type,
                    super_types
                        .as_ref()
                        .map(|(_, constructor)| constructor.clone()),
                );
                let outer_super_reference =
                    std::mem::replace(&mut ctx.super_class_reference, class.extends.first().cloned());
                if let Some(scope) = body_scope {
                    ctx.nested_function_scope = Some(scope);
                }
                check_function_body_with_signature_and_this(
                    None,
                    constructor.parameters.clone(),
                    constructor.body.clone(),
                    &function_type,
                    &[],
                    None,
                    false,
                    None,
                    Some(instance_type.clone()),
                    true,
                    false,
                    false,
                    false,
                    ctx,
                );
                ctx.constructor_writable_members = None;
                ctx.super_constructor_type = outer_super_constructor;
                ctx.super_class_reference = outer_super_reference;
            }
            ParsedClassMember::Method(method) => {
                let _type_variables =
                    crate::checks::function::enter_body_type_variables(&method.type_parameters, ctx);
                let function_type = map_function_signature(
                    &method.parameters,
                    method.return_type.as_ref(),
                    &method.type_parameters,
                    None,
                    ctx,
                );
                // A bodyless method is `abstract`, an overload signature, or a
                // member of an ambient class: there is no body to check, and
                // checking the absent one reported TS2355 for a non-void return
                // the implementation below actually satisfies. The same guard
                // has always been on the function-declaration path
                // (`check_function_declaration`); class members never had it.
                // The signature above is still mapped, so its annotations are
                // resolved and reported either way.
                if !method.has_body {
                    continue;
                }
                // A method's own `this` parameter is what `this` means in it.
                let this_type = match &method.this_parameter_type {
                    Some(written) => crate::checks::function::with_type_parameter_scope(
                        &method.type_parameters,
                        ctx,
                        |ctx| map_parsed_type(written.clone(), ctx),
                    ),
                    None if method.is_static => static_type.clone(),
                    None => instance_type.clone(),
                };
                if let Some(scope) = body_scope {
                    ctx.nested_function_scope = Some(scope);
                }
                check_function_body_with_signature_and_this(
                    None,
                    method.parameters.clone(),
                    method.body.clone(),
                    &function_type,
                    &method.type_parameters,
                    None,
                    method.return_type.is_some(),
                    method.return_type_span.or(method.name_span),
                    Some(this_type),
                    false,
                    method.is_generator,
                    method.is_async,
                    false,
                    ctx,
                );
            }
            ParsedClassMember::Property(property) => {
                let this_type = if property.is_static {
                    static_type.clone()
                } else {
                    instance_type.clone()
                };
                if let Some(declared) = &property.declared_type {
                    report_this_query_members(declared, &this_type, ctx);
                }
                // A static property's annotation is not resolved anywhere else
                // on this path; resolving it here is only for the type
                // parameters it may not name, so nothing else it reports is
                // kept (the annotation is checked with the class's own types).
                if property.is_static
                    && !class_type_parameter_names.is_empty()
                    && let Some(declared) = property.declared_type.clone()
                {
                    let checkpoint = ctx.diagnostics().len();
                    let _ = map_parsed_type(declared, ctx);
                    let static_parameter_errors: Vec<_> = ctx.diagnostics()[checkpoint..]
                        .iter()
                        .filter(|diagnostic| diagnostic.code.to_string() == "TS2302")
                        .cloned()
                        .collect();
                    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
                    for diagnostic in static_parameter_errors {
                        ctx.push(diagnostic);
                    }
                }
                let contextual_type = scopes.and_then(|scopes| scopes.static_property_context(property));
                check_class_property_initializer(property, this_type, contextual_type, ctx);
            }
            ParsedClassMember::Accessor(accessor) => {
                // A same-named method or property takes the member's symbol, so
                // the binder declares each accessor apart (TS2300) and the getter
                // has no setter to take its type from.
                let shares_symbol = class.members.iter().any(|other| match other {
                    ParsedClassMember::Method(method) => {
                        method.name == accessor.name && method.is_static == accessor.is_static
                    }
                    ParsedClassMember::Property(property) => {
                        property.name == accessor.name && property.is_static == accessor.is_static
                    }
                    _ => false,
                });
                check_class_accessor_body(accessor, !shares_symbol, &instance_type, &static_type, body_scope, ctx);
            }
            ParsedClassMember::StaticBlock(block) => {
                let function_type = FunctionType::new(Vec::new(), Type::Void, false, 0);
                if let Some(scope) = body_scope {
                    ctx.nested_function_scope = Some(scope);
                }
                check_function_body_with_signature_and_this(
                    None,
                    Vec::new(),
                    block.body.clone(),
                    &function_type,
                    &[],
                    None,
                    false,
                    None,
                    Some(static_type.clone()),
                    false,
                    false,
                    false,
                    false,
                    ctx,
                );
            }
        }
    }
    ctx.static_member_type_parameters = None;
    ctx.enclosing_class_members.pop();
    let _ = ctx.symbols.remove("super");
    if let Some(previous) = previous_super {
        let _ = ctx.symbols.insert_handle("super", previous);
    }
}

/// Each accessor body is checked like a method's. A setter parameter written
/// without a type takes the getter's annotation, as tsc infers it.
fn check_class_accessor_body(
    accessor: &surge_ts_syntax::ParsedClassAccessor,
    pairs_with_setter: bool,
    instance_type: &Type,
    static_type: &Type,
    body_scope: Option<Arc<SymbolTable>>,
    ctx: &mut CheckerContext,
) {
    for declaration in &accessor.declarations {
        if !declaration.has_body {
            continue;
        }
        let return_type = if declaration.is_getter {
            accessor
                .getter_return_type
                .as_ref()
                .or(accessor.setter_param_type.as_ref().filter(|_| pairs_with_setter))
        } else {
            None
        };
        let mut parameters = declaration.parameters.clone();
        // A getter without an annotation types the pair from its body, which
        // is not modelled here; the parameter reads `any` rather than implicit.
        if !declaration.is_getter
            && accessor.has_getter
            && let Some(parameter) = parameters.first_mut()
            && parameter.declared_type.is_none()
        {
            parameter.declared_type =
                Some(accessor.getter_return_type.clone().unwrap_or(ParsedType::Any));
        }
        let function_type = map_function_signature(&parameters, return_type, &[], None, ctx);
        let this_type = if accessor.is_static {
            static_type.clone()
        } else {
            instance_type.clone()
        };
        if let Some(scope) = &body_scope {
            ctx.nested_function_scope = Some(scope.clone());
        }
        check_function_body_with_signature_and_this(
            None,
            parameters,
            declaration.body.clone(),
            &function_type,
            &[],
            None,
            return_type.is_some(),
            declaration.return_type_span.or(accessor.name_span),
            Some(this_type),
            false,
            false,
            false,
            false,
            ctx,
        );
    }
}

/// tsc checks a property initializer as it checks a variable's
/// (`checkVariableLikeDeclaration`): against the annotation when there is one,
/// with `this` bound to the instance — or the constructor, for a static.
/// `typeof this.x` in a property's annotation is `this.x` checked as an
/// expression (`checkTypeQuery`), so a member a closed `this` lacks is TS2339 at
/// the member. Type resolution leaves such a query open, binding no `this`.
fn report_this_query_members(annotation: &ParsedType, this_type: &Type, ctx: &mut CheckerContext) {
    match annotation {
        ParsedType::TypeOf(query) if query.name == "this" && query.import_specifier.is_none() => {
            let mut ty = this_type.clone();
            for (index, member) in query.members.iter().enumerate() {
                if let Some(member_ty) = ty.get_property_access_type(member) {
                    ty = member_ty;
                    continue;
                }
                let closed = matches!(ty.peeled(), Type::Object(object)
                    if !object.synthetic_open_index && object.string_index_type.is_none());
                if closed && let Some(span) = query.member_spans.get(index) {
                    ctx.push(
                        Diagnostic::ts2339(member, ty.name(), ctx.file_name.clone()).with_span(convert_span(*span)),
                    );
                }
                return;
            }
        }
        ParsedType::Union(members) | ParsedType::Intersection(members) => {
            for member in members.iter() {
                report_this_query_members(member, this_type, ctx);
            }
        }
        ParsedType::Array(element) | ParsedType::Readonly(element) => report_this_query_members(element, this_type, ctx),
        _ => {}
    }
}

fn check_class_property_initializer(
    property: &ParsedClassProperty,
    this_type: Type,
    contextual_type: Option<Type>,
    ctx: &mut CheckerContext,
) {
    let Some(initializer) = &property.initializer else {
        return;
    };
    let mut symbols = ctx
        .symbols
        .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let _ = symbols.insert(
        "this".to_string(),
        SymbolInfo {
            ty: this_type,
            kind: SymbolKind::Const,
            function_signature: None,
        },
    );
    let outer_bindings = property
        .initializer_span
        .and_then(|span| {
            ctx.constructor_local_properties
                .get(ctx.file_name.as_str())?
                .iter()
                .filter(|candidate| {
                    candidate.span.start <= span.start && span.end <= candidate.span.end
                })
                .min_by_key(|candidate| candidate.span.start)
        })
        .map(|candidate| {
            candidate
                .constructor_locals
                .iter()
                .map(|local| (Arc::from(local.as_str()), symbols.get_handle(local)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let saved_outer_bindings =
        std::mem::replace(&mut ctx.constructor_local_outer_bindings, outer_bindings);
    let Some(declared_type) = property.declared_type.clone() else {
        match contextual_type {
            Some(contextual_type) => {
                let _ = crate::checks::expected::evaluate_expression_with_expected_type_anchored(
                    initializer,
                    property.initializer_span,
                    property.name_span,
                    Some(&contextual_type),
                    crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
                    &symbols,
                    ctx,
                );
            }
            None => {
                crate::checks::expr::evaluate_expression(
                    initializer,
                    property.initializer_span,
                    &symbols,
                    ctx,
                );
            }
        }
        ctx.constructor_local_outer_bindings = saved_outer_bindings;
        return;
    };
    let declared_type = map_parsed_type(declared_type, ctx);
    // `addOptionality`: an optional property's initializer is checked against
    // the declared type with `undefined` added.
    let declared_type = if property.optional && surge_ts_types::strict_null_checks() {
        surge_ts_types::union_type(vec![declared_type, Type::Undefined])
    } else {
        declared_type
    };
    let inferred = crate::checks::expected::evaluate_expression_with_expected_type_anchored(
        initializer,
        property.initializer_span,
        property.name_span,
        Some(&declared_type),
        crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
        &symbols,
        ctx,
    );
    ctx.constructor_local_outer_bindings = saved_outer_bindings;
    if let crate::infer::InferredExpression::Known(inferred_type) = inferred {
        crate::checks::var::report_initializer_mismatch(
            &inferred_type,
            &declared_type,
            property.name_span.or(property.initializer_span),
            ctx,
        );
    }
}

/// A non-static property of a class whose constructor implementation declares
/// locals. tsc's name resolver records such a property while climbing out of
/// it (`propertyWithInvalidInitializer` in `nameresolver.go`) when those locals
/// hold a value of the name being resolved, and unless class fields are
/// standard fields `checkAndReportErrorForInvalidInitializer` reports the
/// reference instead of resolving it.
#[derive(Debug, Clone)]
pub(crate) struct ConstructorLocalProperty {
    name: String,
    /// From the property's name to the end of the member: a decorator is
    /// resolved at the class, not inside the property.
    span: surge_ts_syntax::TextSpan,
    name_end: usize,
    initializer: Option<surge_ts_syntax::TextSpan>,
    constructor_locals: Arc<[String]>,
}

impl ConstructorLocalProperty {
    /// TS2844 for a reference in the property's type annotation, TS2301
    /// anywhere else in it.
    pub(crate) fn invalid_reference_diagnostic(
        &self,
        name: &str,
        reference: surge_ts_syntax::TextSpan,
        file_name: String,
    ) -> Diagnostic {
        let in_initializer = self.initializer.is_some_and(|initializer| {
            initializer.start <= reference.start && reference.end <= initializer.end
        });
        if !in_initializer && reference.start >= self.name_end {
            Diagnostic::ts2844(&self.name, name, file_name)
        } else {
            Diagnostic::ts2301(&self.name, name, file_name)
        }
    }
}

/// Every [`ConstructorLocalProperty`] of the classes `statements` declare, at
/// any statement depth.
pub(crate) fn file_constructor_local_properties(
    statements: &[surge_ts_syntax::ParsedStatement],
    options: &crate::context::CheckerOptions,
) -> Arc<[ConstructorLocalProperty]> {
    // tsgo's `GetEmitStandardClassFields`.
    if options.emit_standard_class_fields() {
        return Arc::from([]);
    }
    let mut properties = Vec::new();
    for statement in statements {
        collect_statement_constructor_locals(statement, &mut properties);
    }
    properties.into()
}

/// [`file_constructor_local_properties`] of every source file that has any.
pub(crate) fn constructor_local_properties_by_file(
    parsed_files: &[super::ParsedProgramFile],
    options: &crate::context::CheckerOptions,
) -> Arc<surge_ts_types::fx::FxHashMap<Arc<str>, Arc<[ConstructorLocalProperty]>>> {
    let mut by_file = surge_ts_types::fx::FxHashMap::default();
    for parsed_file in parsed_files {
        if parsed_file.file_kind.is_declaration() {
            continue;
        }
        let properties = file_constructor_local_properties(&parsed_file.statements, options);
        if !properties.is_empty() {
            by_file.insert(Arc::from(parsed_file.file_name.as_str()), properties);
        }
    }
    Arc::new(by_file)
}

/// tsc's `propertyWithInvalidInitializer` for a reference to `name`: of the
/// enclosing properties whose constructor declares the name, the outermost is
/// the last one the resolver's climb records.
pub(crate) fn property_with_invalid_initializer<'a>(
    name: &str,
    reference: surge_ts_syntax::TextSpan,
    ctx: &'a CheckerContext,
) -> Option<&'a ConstructorLocalProperty> {
    ctx.constructor_local_properties
        .get(ctx.file_name.as_str())?
        .iter()
        .filter(|property| {
            property.span.start <= reference.start
                && reference.end <= property.span.end
                && property.constructor_locals.iter().any(|local| local == name)
        })
        .min_by_key(|property| property.span.start)
}

fn collect_statement_constructor_locals(
    statement: &surge_ts_syntax::ParsedStatement,
    properties: &mut Vec<ConstructorLocalProperty>,
) {
    use surge_ts_syntax::{ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedStatement};
    match statement {
        ParsedStatement::ClassDeclaration(class) => {
            collect_class_constructor_locals(class, properties)
        }
        ParsedStatement::FunctionDeclaration(function) => {
            collect_body_constructor_locals(&function.body, properties)
        }
        // An ambient namespace's classes have no constructor bodies.
        ParsedStatement::NamespaceDeclaration(namespace) if !namespace.is_declare => {
            for statement in &namespace.statements {
                collect_statement_constructor_locals(statement, properties);
            }
        }
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => {
                collect_statement_constructor_locals(declaration, properties)
            }
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Class(class),
                ..
            } => collect_class_constructor_locals(class, properties),
            ParsedExportDeclaration::Default {
                declaration: ParsedDefaultExportDeclaration::Function(function),
                ..
            } => collect_body_constructor_locals(&function.body, properties),
            _ => {}
        },
        ParsedStatement::If(if_statement) => {
            collect_body_constructor_locals(&if_statement.then_body, properties);
            collect_body_constructor_locals(&if_statement.else_body, properties);
        }
        ParsedStatement::Block(body) => collect_body_constructor_locals(body, properties),
        _ => {}
    }
}

fn collect_body_constructor_locals(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    properties: &mut Vec<ConstructorLocalProperty>,
) {
    use surge_ts_syntax::ParsedFunctionBodyStatement;
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Class(class) => {
                collect_class_constructor_locals(class, properties)
            }
            ParsedFunctionBodyStatement::Function(function) => {
                collect_body_constructor_locals(&function.body, properties)
            }
            ParsedFunctionBodyStatement::Block(block) => {
                collect_body_constructor_locals(block, properties)
            }
            ParsedFunctionBodyStatement::If(if_statement) => {
                collect_body_constructor_locals(&if_statement.then_body, properties);
                collect_body_constructor_locals(&if_statement.else_body, properties);
            }
            ParsedFunctionBodyStatement::While(while_statement) => {
                collect_body_constructor_locals(&while_statement.body, properties)
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                collect_body_constructor_locals(&for_of_statement.body, properties)
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    collect_body_constructor_locals(&case.consequent, properties);
                }
            }
            ParsedFunctionBodyStatement::Try(try_statement) => {
                collect_body_constructor_locals(&try_statement.block, properties);
                if let Some(handler) = &try_statement.handler {
                    collect_body_constructor_locals(&handler.body, properties);
                }
                collect_body_constructor_locals(&try_statement.finalizer, properties);
            }
            _ => {}
        }
    }
}

fn collect_class_constructor_locals(
    class: &ParsedClassDeclaration,
    properties: &mut Vec<ConstructorLocalProperty>,
) {
    for member in &class.members {
        match member {
            ParsedClassMember::Method(method) => {
                collect_body_constructor_locals(&method.body, properties)
            }
            ParsedClassMember::Constructor(constructor) => {
                collect_body_constructor_locals(&constructor.body, properties)
            }
            ParsedClassMember::Accessor(accessor) => {
                for declaration in &accessor.declarations {
                    collect_body_constructor_locals(&declaration.body, properties);
                }
            }
            ParsedClassMember::StaticBlock(block) => {
                collect_body_constructor_locals(&block.body, properties)
            }
            ParsedClassMember::Property(_) => {}
        }
    }
    if class.is_declare {
        return;
    }
    // `FindConstructorDeclaration` takes the constructor with a body, which
    // follows its overloads.
    let Some(constructor) = class.members.iter().rev().find_map(|member| match member {
        ParsedClassMember::Constructor(constructor) => Some(constructor),
        _ => None,
    }) else {
        return;
    };
    let locals = constructor_local_value_names(constructor);
    if locals.is_empty() {
        return;
    }
    let locals: Arc<[String]> = locals.into();
    for member in &class.members {
        let ParsedClassMember::Property(property) = member else {
            continue;
        };
        let (false, Some(name_span), Some(span)) =
            (property.is_static, property.name_span, property.span)
        else {
            continue;
        };
        properties.push(ConstructorLocalProperty {
            name: property.name.clone(),
            span: surge_ts_syntax::TextSpan {
                start: name_span.start,
                end: span.end,
            },
            name_end: name_span.end,
            initializer: property.initializer_span,
            constructor_locals: Arc::clone(&locals),
        });
    }
}

/// The names `ctor.Locals()` answers with a value: the parameters, what the
/// body declares at its top level, and every `var` it hoists.
fn constructor_local_value_names(
    constructor: &surge_ts_syntax::ParsedClassConstructor,
) -> Vec<String> {
    use surge_ts_syntax::ParsedFunctionBodyStatement;
    let mut names: Vec<String> = constructor
        .parameters
        .iter()
        .flat_map(|parameter| parameter.binding_name.bound_names())
        .map(|bound| bound.name)
        .collect();
    for statement in &constructor.body {
        match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                names.push(variable.name.clone())
            }
            ParsedFunctionBodyStatement::Function(function) => names.push(function.name.clone()),
            ParsedFunctionBodyStatement::Class(class) => names.push(class.name.clone()),
            _ => {}
        }
    }
    crate::flow::collect_var_names(&constructor.body, &mut names);
    names
}

/// Inserts a class's instance-side interface into the current type-declaration
/// table. Mirrors `collect_interface` for first-wins / duplicate behaviour.
pub(crate) fn collect_class(class: &ParsedClassDeclaration, ctx: &mut CheckerContext) {
    let info = class_instance_interface_info(class, ctx.file_name_arc());
    // A class declaration-merges with a same-named interface of its file
    // (`declare interface Emitter<T> { on(…): this }` + `class Emitter<T>
    // extends EventEmitter {}`): the interface's members and the class's
    // heritage both belong to the instance. The class-first order already
    // merges through `collect_interface`; this is the interface-first order.
    let merged = match ctx.type_declarations.get(&class.name) {
        Some(TypeDeclarationInfo::Interface(existing)) if existing.file_name == info.file_name => {
            Some(crate::symbols::merge_interface_infos(existing, &info))
        }
        _ => None,
    };
    match merged {
        Some(merged) => ctx
            .type_declarations
            .upsert(&class.name, TypeDeclarationInfo::Interface(merged)),
        None => {
            let _ = ctx
                .type_declarations
                .insert(class.name.clone(), TypeDeclarationInfo::Interface(info));
        }
    }
}
