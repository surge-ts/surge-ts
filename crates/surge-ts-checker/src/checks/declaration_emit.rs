//! What declaration emit reports about the types the checker infers for a
//! module's exports. A class expression, or a class declared inside a
//! function, has no name a declaration file could write its type by, so emit
//! writes the type out as a literal (`WriteClassExpressionAsTypeLiteral`) —
//! and a type literal cannot keep a member private or protected (TS4094,
//! `createTypeNodesFromResolvedType`).

use std::collections::BTreeSet;
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedFunctionDeclaration, TextSpan};
use surge_ts_types::fx::FxHashSet;
use surge_ts_types::{FunctionType, ObjectProperty, ObjectType, Type};

use crate::context::{CheckerContext, convert_span};

const MAX_DEPTH: usize = 12;
const WORK_BUDGET: usize = 20_000;

/// Whether declaration emit writes the declarations this file exports from its
/// top level.
pub(crate) fn emits_declarations(ctx: &CheckerContext) -> bool {
    ctx.options.declaration
        && ctx.namespace_member_prefix_stack.is_empty()
        && !surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
        && !surge_ts_syntax::is_declaration_file_name(&ctx.file_name)
}

/// Where emit reports on the return type it infers for an exported function:
/// its name, when the type is the body's to infer.
pub(crate) fn inferred_return_span(function: &ParsedFunctionDeclaration, ctx: &CheckerContext) -> Option<TextSpan> {
    let infers_return = function.return_type.is_none()
        && function.full_signature.is_none()
        && function.has_body
        && !function.is_declare
        && !function.is_generator
        && !is_overloaded(&function.name, ctx);
    if !(infers_return && emits_declarations(ctx)) {
        return None;
    }
    function.name_span
}

/// An implementation behind overload signatures, which emit writes instead.
fn is_overloaded(name: &str, ctx: &CheckerContext) -> bool {
    ctx.symbols.get(name).is_some_and(|symbol| match &symbol.ty {
        Type::Function(function) => function.overloads().is_some_and(|overloads| !overloads.is_empty()),
        _ => false,
    })
}

/// The type emit writes for an expression it serializes (`export default
/// <expression>`, an `extends` clause that is no entity name), reported at
/// `span`.
pub(crate) fn report_expression_class_literal_members(
    expression: &ParsedExpression,
    span: Option<TextSpan>,
    ctx: &mut CheckerContext,
) {
    if is_entity_name_expression(expression) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if let crate::infer::InferredExpression::Known(ty) = inferred {
        report_class_literal_members(std::slice::from_ref(&ty), span, ctx);
    }
}

/// [`report_inaccessible_names`] for the type of `export default <expression>`.
pub(crate) fn report_default_export_names(expression: &ParsedExpression, span: Option<TextSpan>, ctx: &mut CheckerContext) {
    if is_entity_name_expression(expression) {
        return;
    }
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, &symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    if let crate::infer::InferredExpression::Known(ty) = inferred {
        report_inaccessible_names(std::slice::from_ref(&ty), EmitSite::DefaultExport, span, ctx);
    }
}

fn is_entity_name_expression(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::Identifier { .. } => true,
        ParsedExpression::PropertyAccess { object, is_bracketed: false, .. } => is_entity_name_expression(object),
        _ => false,
    }
}

/// TS4094 at `span` for each private or protected member of a class emit
/// writes as a type literal while it writes `types`.
pub(crate) fn report_class_literal_members(types: &[Type], span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let names = {
        let mut walk = ClassLiteralWalk {
            ctx: &*ctx,
            names: BTreeSet::new(),
            expanded: FxHashSet::default(),
            work: 0,
        };
        for ty in types {
            walk.visit(ty, 0);
        }
        walk.names
    };
    for name in names {
        let diagnostic = Diagnostic::ts4094(name, ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// tsc's node builder over one inferred type, reduced to what decides TS4094:
/// a named type is written by its name (its type arguments with it), an
/// anonymous one by its members, and a class with no name to write is
/// expanded with every member it has, inherited ones included.
struct ClassLiteralWalk<'a> {
    ctx: &'a CheckerContext,
    names: BTreeSet<String>,
    expanded: FxHashSet<Arc<str>>,
    work: usize,
}

impl ClassLiteralWalk<'_> {
    fn visit(&mut self, ty: &Type, depth: usize) {
        self.work += 1;
        if depth > MAX_DEPTH || self.work > WORK_BUDGET {
            return;
        }
        let depth = depth + 1;
        match ty {
            Type::Union(union) => {
                for member in union.types() {
                    self.visit(member, depth);
                }
            }
            Type::Array(element) => self.visit(element, depth),
            Type::Tuple(elements) => {
                for element in elements {
                    self.visit(element, depth);
                }
            }
            Type::OpenTuple(tuple) => {
                for element in tuple.leading.iter().chain(std::iter::once(tuple.rest.as_ref())).chain(&tuple.trailing) {
                    self.visit(element, depth);
                }
            }
            Type::Function(function) => self.visit_signature(function, depth),
            Type::Reference(reference) if self.is_class_literal_instance(&reference.id) => {
                if let Type::Object(object) = reference.resolve() {
                    self.expand_instance(&reference.id, &object, depth);
                }
            }
            Type::Reference(reference) if reference.render_structurally => self.visit(&reference.resolve(), depth),
            Type::Reference(reference) => {
                for argument in reference.arguments.iter() {
                    self.visit(argument, depth);
                }
            }
            Type::Object(object) => self.visit_object(object, depth),
            _ => {}
        }
    }

    fn visit_signature(&mut self, function: &FunctionType, depth: usize) {
        let overloads = function.overloads().unwrap_or_default();
        for signature in std::iter::once(function).chain(overloads) {
            for parameter in signature.parameters() {
                self.visit(parameter, depth);
            }
            self.visit(signature.return_type(), depth);
        }
    }

    fn visit_object(&mut self, object: &ObjectType, depth: usize) {
        if let Some(signature) = object.construct_signature()
            && object.alias_name.as_deref().is_some_and(|name| name.starts_with("typeof "))
        {
            // `typeof C` is written by name unless C is a class with none.
            if self.instance_id(signature.return_type()).is_some_and(|id| self.is_class_literal_instance(&id)) {
                for (name, property) in object.properties.iter() {
                    if name.as_ref() == "prototype" {
                        continue;
                    }
                    self.note(name, property);
                    self.visit(&property.ty, depth);
                }
                self.visit_signature(signature, depth);
                for operand in object.intersection_operands.iter().flat_map(|operands| operands.iter()) {
                    self.visit(operand, depth);
                }
            }
            return;
        }
        if let Some(id) = object.alias_id.as_deref() {
            if self.is_class_literal_instance(id) {
                self.expand_instance(id, object, depth);
            }
            return;
        }
        if object.alias_name.is_some() {
            return;
        }
        for property in object.properties.values() {
            self.visit(&property.ty, depth);
        }
        for signature in object.call_signature().into_iter().chain(object.construct_signature()) {
            self.visit_signature(signature, depth);
        }
        for index in object.string_index_type.iter().chain(&object.number_index_type) {
            self.visit(index, depth);
        }
        for operand in object.intersection_operands.iter().flat_map(|operands| operands.iter()) {
            self.visit(operand, depth);
        }
    }

    fn expand_instance(&mut self, id: &str, object: &ObjectType, depth: usize) {
        if !self.expanded.insert(Arc::from(id)) {
            return;
        }
        for (name, property) in object.properties.iter() {
            self.note(name, property);
            self.visit(&property.ty, depth);
        }
    }

    fn note(&mut self, name: &str, property: &ObjectProperty) {
        if property.restriction.is_some() || name.starts_with('#') {
            self.names.insert(name.to_string());
        }
    }

    fn instance_id(&self, instance: &Type) -> Option<Arc<str>> {
        match instance {
            Type::Object(object) => object.alias_id.clone(),
            Type::Reference(reference) => Some(reference.id.clone()),
            _ => None,
        }
    }

    /// A class declaration emit cannot name: a class expression (bound by a
    /// `const` of this file, or anywhere in a body), or a class a function
    /// body declares. Body-local declarations are keyed `name@offset`.
    fn is_class_literal_instance(&self, id: &str) -> bool {
        let Some((file, name)) = id.split_once('\0') else {
            return false;
        };
        name.contains('@')
            || name == "(Anonymous class)"
            || (file == self.ctx.file_name.as_str() && self.ctx.file_const_class_names.contains(name))
    }
}

/// TS4023 at `span` for each late-bound member key (`[SYMBOL]`, `[NS.sym]`)
/// emit writes into an anonymous type of the exported variable `name` whose
/// leading name this file does not bind: a computed property name is an
/// expression, which no import type can stand in for.
pub(crate) fn report_unnameable_late_bound_keys(ty: &Type, name: &str, span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let mut heads = BTreeSet::new();
    let mut work = 0;
    collect_late_bound_key_heads(ty, 0, &mut work, &mut heads);
    for head in heads {
        if ctx.symbols.get(&head).is_some()
            || ctx.module_value_fallback.as_ref().is_some_and(|table| table.get(&head).is_some())
        {
            continue;
        }
        let diagnostic = Diagnostic::ts4023(name, &head, "\"\"", ctx.file_name.clone());
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// The names leading the late-bound keys of the anonymous types emit writes
/// out member by member; a named type is written by its name alone.
fn collect_late_bound_key_heads(ty: &Type, depth: usize, work: &mut usize, heads: &mut BTreeSet<String>) {
    *work += 1;
    if depth > MAX_DEPTH || *work > WORK_BUDGET {
        return;
    }
    let depth = depth + 1;
    match ty {
        Type::Union(union) => {
            for member in union.types() {
                collect_late_bound_key_heads(member, depth, work, heads);
            }
        }
        Type::Array(element) => collect_late_bound_key_heads(element, depth, work, heads),
        Type::Tuple(elements) => {
            for element in elements {
                collect_late_bound_key_heads(element, depth, work, heads);
            }
        }
        Type::Reference(reference) => {
            for argument in reference.arguments.iter() {
                collect_late_bound_key_heads(argument, depth, work, heads);
            }
        }
        Type::Object(object) if object.alias_name.is_none() && object.alias_id.is_none() => {
            for (key, property) in object.properties.iter() {
                if let Some(inner) = key.strip_prefix('[').and_then(|key| key.strip_suffix(']'))
                    && let Some(head) = inner.split('.').next()
                    && !head.is_empty()
                    && head.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
                {
                    heads.insert(head.to_string());
                }
                collect_late_bound_key_heads(&property.ty, depth, work, heads);
            }
        }
        _ => {}
    }
}

/// Publishes what [`crate::program::program_unexported_types`] reads.
pub(crate) fn publish_unexported_type_declarations(
    parsed_files: &[crate::program::ParsedProgramFile],
    module_export_tables: &[Option<crate::modules::ModuleExportTable>],
    local_type_declarations: &[Option<Arc<crate::symbols::TypeDeclarationTable>>],
    resolve_package_json_exports: bool,
) {
    use crate::infer::types::cache::canonical_declaration_file_name;
    use crate::symbols::TypeDeclarationInfo;
    let key = |info: &TypeDeclarationInfo, name: &str| {
        let file_name = match info {
            TypeDeclarationInfo::Alias(alias) => &alias.file_name,
            TypeDeclarationInfo::Interface(interface) => &interface.file_name,
        };
        Arc::<str>::from(format!("{}\0{name}", canonical_declaration_file_name(file_name)))
    };
    let mut exported = FxHashSet::default();
    for table in module_export_tables.iter().flatten() {
        for (bound, info) in table.type_declarations.iter() {
            exported.insert(key(info, bound));
            exported.insert(key(info, info.declared_name()));
        }
    }
    let mut unexported = surge_ts_types::fx::FxHashMap::default();
    for (file, table) in parsed_files.iter().zip(local_type_declarations) {
        let Some(table) = table.as_ref().filter(|_| file.is_module) else {
            continue;
        };
        for (name, info) in table.iter() {
            let id = key(info, name);
            if name.contains('.') || exported.contains(&id) {
                continue;
            }
            let kind = match info {
                TypeDeclarationInfo::Alias(_) => crate::program::UnexportedTypeKind::Alias,
                TypeDeclarationInfo::Interface(interface) if interface.is_class_instance => {
                    crate::program::UnexportedTypeKind::Class
                }
                TypeDeclarationInfo::Interface(_) => crate::program::UnexportedTypeKind::Interface,
            };
            unexported.insert(id, kind);
        }
    }
    // Without `resolvePackageJsonExports` a package's `exports` hides none of
    // its files from a module specifier.
    let unportable = if resolve_package_json_exports {
        unportable_package_declarations(parsed_files, module_export_tables, &key)
    } else {
        Default::default()
    };
    crate::program::publish_program_unexported_types(crate::program::UnexportedTypeDeclarations {
        unexported,
        unportable,
    });
}

/// The declarations of package files no portable module specifier reaches
/// (`tryGetModuleNameAsNodeModule`): a package nested in another package's
/// `node_modules`, or a file its package's `exports` does not expose. A
/// declaration an exposed file of the same package re-exports stays
/// reachable through it.
fn unportable_package_declarations(
    parsed_files: &[crate::program::ParsedProgramFile],
    module_export_tables: &[Option<crate::modules::ModuleExportTable>],
    key: &dyn Fn(&crate::symbols::TypeDeclarationInfo, &str) -> Arc<str>,
) -> surge_ts_types::fx::FxHashMap<Arc<str>, Arc<str>> {
    let mut exposures: surge_ts_types::fx::FxHashMap<String, Option<Vec<String>>> = Default::default();
    let mut portable_exports: FxHashSet<Arc<str>> = FxHashSet::default();
    let mut candidates: Vec<(Arc<str>, Arc<str>)> = Vec::new();
    for (index, file) in parsed_files.iter().enumerate() {
        let Some(package) = PackageFile::of(&file.file_name) else {
            continue;
        };
        let exposure = exposures
            .entry(package.root.clone())
            .or_insert_with(|| package_exposure(&package.root));
        let portable = !package.nested
            && exposure
                .as_ref()
                .is_none_or(|targets| targets.iter().any(|target| target_exposes(target, &package.relative)));
        if portable {
            if let Some(Some(table)) = module_export_tables.get(index) {
                for (bound, info) in table.type_declarations.iter() {
                    portable_exports.insert(key(info, bound));
                    portable_exports.insert(key(info, info.declared_name()));
                }
            }
            continue;
        }
        // Only what the file exports has a specifier to name it by; emit
        // writes out an alias it does not export, as for any other module.
        let Some(Some(table)) = module_export_tables.get(index) else {
            continue;
        };
        if !file.is_module {
            continue;
        }
        let module: Arc<str> = Arc::from(package.display());
        let own = format!("{}\0", crate::infer::types::cache::canonical_declaration_file_name(&file.file_name));
        for (_, info) in table.type_declarations.iter() {
            let id = key(info, info.declared_name());
            if id.starts_with(own.as_str()) {
                candidates.push((id, module.clone()));
            }
        }
    }
    candidates.into_iter().filter(|(id, _)| !portable_exports.contains(id)).collect()
}

struct PackageFile {
    /// The package directory, `…/node_modules/name` or `…/node_modules/@scope/name`.
    root: String,
    /// The file's path inside the package.
    relative: String,
    /// The package sits in another package's `node_modules`, which a module
    /// specifier from outside that package does not reach.
    nested: bool,
}

impl PackageFile {
    fn of(file_name: &str) -> Option<Self> {
        let marker = "/node_modules/";
        let at = file_name.rfind(marker)?;
        let after = &file_name[at + marker.len()..];
        let mut segments = after.splitn(3, '/');
        let first = segments.next()?;
        let name_length = if first.starts_with('@') {
            first.len() + 1 + segments.next()?.len()
        } else {
            first.len()
        };
        if first.starts_with('.') || after.len() <= name_length {
            return None;
        }
        let outer = &file_name[..at];
        // A package manager's store (`.pnpm`, `.store`) links every package
        // back to the top level, where it is reached by name.
        let nested = outer.contains(marker) && !file_name.contains("/node_modules/.");
        Some(Self {
            root: file_name[..at + marker.len() + name_length].to_string(),
            relative: after[name_length + 1..].to_string(),
            nested,
        })
    }

    fn display(&self) -> String {
        let without_extension = [".d.ts", ".d.mts", ".d.cts", ".ts", ".tsx", ".mts", ".cts"]
            .iter()
            .find_map(|extension| self.relative.strip_suffix(extension))
            .unwrap_or(&self.relative);
        let package = self.root.rsplit_once("/node_modules/").map_or(self.root.as_str(), |(_, name)| name);
        format!("{package}/{without_extension}")
    }
}

/// The paths a package's `exports` exposes, `None` when it has no `exports`
/// (every file is reachable by its path) or a `typesVersions` that may
/// redirect them.
fn package_exposure(root: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("{root}/package.json")).ok()?;
    let serde_json::Value::Object(json) = serde_json::from_str::<serde_json::Value>(&text).ok()? else {
        return None;
    };
    if json.contains_key("typesVersions") {
        return None;
    }
    let exports = json.get("exports").filter(|exports| !exports.is_null())?;
    fn collect(value: &serde_json::Value, targets: &mut Vec<String>) {
        match value {
            serde_json::Value::String(target) => targets.push(target.trim_start_matches("./").to_string()),
            serde_json::Value::Array(values) => values.iter().for_each(|value| collect(value, targets)),
            serde_json::Value::Object(map) => map.values().for_each(|value| collect(value, targets)),
            _ => {}
        }
    }
    let mut targets = Vec::new();
    collect(exports, &mut targets);
    Some(targets)
}

/// Whether an `exports` target (a path, a `*` pattern, or a folder ending in
/// `/`) maps to the declaration file at `relative`.
fn target_exposes(target: &str, relative: &str) -> bool {
    let declaration_of = |path: &str| -> Vec<String> {
        let mut paths = vec![path.to_string()];
        for (script, declaration) in [(".js", ".d.ts"), (".mjs", ".d.mts"), (".cjs", ".d.cts"), (".js", ".ts"), (".js", ".tsx")] {
            if let Some(stem) = path.strip_suffix(script) {
                paths.push(format!("{stem}{declaration}"));
            }
        }
        paths
    };
    if target.ends_with('/') {
        return relative.starts_with(target);
    }
    match target.split_once('*') {
        Some((prefix, suffix)) => declaration_of(suffix)
            .iter()
            .chain(std::iter::once(&suffix.to_string()))
            .any(|suffix| relative.starts_with(prefix) && relative.ends_with(suffix.as_str())),
        None => declaration_of(target).iter().any(|path| path == relative),
    }
}

thread_local! {
    /// What declaration emit writes of the file being checked beyond its
    /// exported declarations (see [`begin_file`]).
    static FILE_EMIT: std::cell::RefCell<FileEmit> = std::cell::RefCell::new(FileEmit::default());
}

#[derive(Default)]
struct FileEmit {
    /// Exported function declarations, whose expando members emit writes.
    exported_functions: FxHashSet<String>,
    /// Local variables emit declares because an export names them
    /// (`export { x }`, `export default x`, `export = x`, or a JavaScript
    /// `module.exports = x`).
    painted: FxHashSet<String>,
}

/// Records what declaration emit writes of the file about to be checked.
pub(crate) fn begin_file(statements: &[surge_ts_syntax::ParsedStatement], ctx: &CheckerContext) {
    use surge_ts_syntax::{ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedExpression, ParsedStatement};
    let mut emit = FileEmit::default();
    let javascript = surge_ts_syntax::is_javascript_file_name(&ctx.file_name);
    if ctx.options.declaration && ctx.namespace_member_prefix_stack.is_empty() {
        for statement in statements {
            match statement {
                ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                    ParsedExportDeclaration::Statement { declaration, .. } => {
                        if let ParsedStatement::FunctionDeclaration(function) = declaration.as_ref() {
                            emit.exported_functions.insert(function.name.clone());
                        }
                    }
                    ParsedExportDeclaration::Named { specifiers, module_specifier: None, .. } => {
                        emit.painted.extend(specifiers.iter().map(|specifier| specifier.local_name.clone()));
                    }
                    ParsedExportDeclaration::Default {
                        declaration: ParsedDefaultExportDeclaration::Expression(ParsedExpression::Identifier { name, .. }),
                        ..
                    } => {
                        emit.painted.insert(name.clone());
                    }
                    ParsedExportDeclaration::Equals { exported_name, .. } => {
                        emit.painted.insert(exported_name.clone());
                    }
                    _ => {}
                },
                ParsedStatement::MemberAssignment(assignment) if javascript => {
                    if let ParsedExpression::PropertyAccess { object, property_name, .. } = &assignment.target
                        && matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "module")
                        && property_name == "exports"
                        && let ParsedExpression::Identifier { name, .. } = &assignment.value
                    {
                        emit.painted.insert(name.clone());
                    }
                }
                _ => {}
            }
        }
    }
    FILE_EMIT.with(|state| *state.borrow_mut() = emit);
}

/// Whether emit declares the top-level variable `name` though the file does
/// not export it.
pub(crate) fn is_painted_variable(name: &str, ctx: &CheckerContext) -> bool {
    ctx.options.declaration
        && ctx.namespace_member_prefix_stack.is_empty()
        && !surge_ts_syntax::is_declaration_file_name(&ctx.file_name)
        && FILE_EMIT.with(|state| state.borrow().painted.contains(name))
}

/// Whether `name.member = …` writes an expando member of a function the file
/// exports, whose declaration emit writes each such member.
pub(crate) fn is_exported_function(name: &str) -> bool {
    FILE_EMIT.with(|state| state.borrow().exported_functions.contains(name))
}

/// Which declaration emit writes a type for, which picks what it reports.
pub(crate) enum EmitSite<'a> {
    Variable(&'a str),
    FunctionReturn,
    ExpandoProperty(&'a str),
    DefaultExport,
}

/// What declaration emit reports when the types it writes for `site` name a
/// type another module declares and no module exports (tsc's
/// `handleSymbolAccessibilityError`): an interface it cannot name, or, inside
/// an alias it has to write out instead, a property's unique symbol.
pub(crate) fn report_inaccessible_names(types: &[Type], site: EmitSite<'_>, span: Option<TextSpan>, ctx: &mut CheckerContext) {
    let Some(unexported) = crate::program::program_unexported_types() else {
        return;
    };
    let current = crate::infer::types::cache::canonical_declaration_file_name(&ctx.file_name);
    let mut walk = InaccessibleWalk {
        unexported: &unexported.unexported,
        unportable: &unexported.unportable,
        unportable_names: Vec::new(),
        current: &current,
        names: Vec::new(),
        unique_symbol: false,
        expanded: FxHashSet::default(),
        work: 0,
    };
    for ty in types {
        walk.visit(ty, 0, false);
    }
    let file_name = ctx.file_name.clone();
    let mut diagnostics = Vec::new();
    for (name, module) in &walk.names {
        diagnostics.push(match site {
            EmitSite::Variable(variable) => Diagnostic::ts4023(variable, name, format!("\"{module}\""), file_name.clone()),
            EmitSite::FunctionReturn => Diagnostic::ts4058(name, format!("\"{module}\""), file_name.clone()),
            EmitSite::ExpandoProperty(property) => {
                Diagnostic::ts4032(property, name, format!("\"{module}\""), file_name.clone())
            }
            EmitSite::DefaultExport => Diagnostic::ts4082(name, file_name.clone()),
        });
    }
    for (name, module) in &walk.unportable_names {
        let declaration = match site {
            EmitSite::Variable(variable) => variable,
            EmitSite::DefaultExport => "default",
            EmitSite::FunctionReturn | EmitSite::ExpandoProperty(_) => continue,
        };
        diagnostics.push(Diagnostic::ts2883(declaration, module, name, file_name.clone()));
    }
    if walk.unique_symbol
        && let EmitSite::Variable(variable) = site
    {
        diagnostics.push(Diagnostic::ts2527(variable, "unique symbol", file_name));
    }
    for diagnostic in diagnostics {
        ctx.push(match span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

/// tsc's node builder reduced to symbol accessibility: a named type is written
/// by name when emit can reach it, an inaccessible alias is written out, and
/// an inaccessible interface is reported.
struct InaccessibleWalk<'a> {
    unexported: &'a surge_ts_types::fx::FxHashMap<Arc<str>, crate::program::UnexportedTypeKind>,
    unportable: &'a surge_ts_types::fx::FxHashMap<Arc<str>, Arc<str>>,
    /// Named types reachable only through a non-portable package path, with
    /// that path (TS2883).
    unportable_names: Vec<(String, String)>,
    current: &'a str,
    names: Vec<(String, String)>,
    unique_symbol: bool,
    expanded: FxHashSet<Arc<str>>,
    work: usize,
}

impl InaccessibleWalk<'_> {
    fn visit(&mut self, ty: &Type, depth: usize, written_out: bool) {
        self.work += 1;
        if depth > MAX_DEPTH || self.work > WORK_BUDGET {
            return;
        }
        let depth = depth + 1;
        match ty {
            Type::Union(union) => {
                for member in union.types() {
                    self.visit(member, depth, written_out);
                }
            }
            Type::Array(element) => self.visit(element, depth, written_out),
            Type::Tuple(elements) => {
                for element in elements {
                    self.visit(element, depth, written_out);
                }
            }
            Type::Function(function) => self.visit_signature(function, depth, written_out),
            Type::Reference(reference) if reference.is_unique_symbol() => {
                if written_out && reference.display.as_ref() == "unique symbol" {
                    self.unique_symbol = true;
                }
            }
            // A lazily resolved annotation (`file\0signature-annotation\0…`)
            // names no declaration: emit writes what it resolves to.
            Type::Reference(reference)
                if reference.render_structurally
                    || reference.id.split_once('\0').is_some_and(|(_, name)| name.contains('\0')) =>
            {
                self.visit(&reference.resolve(), depth, written_out);
            }
            Type::Reference(reference) => {
                let nominal = reference.enum_owner.is_some() || reference.numeric_enum;
                let write_out = |walk: &mut Self| walk.write_out(&reference.resolve(), depth);
                if self.named(&reference.id, nominal, &write_out) {
                    for argument in reference.arguments.iter() {
                        self.visit(argument, depth, written_out);
                    }
                }
            }
            Type::Object(object) => {
                if let Some(id) = object.alias_id.as_deref() {
                    let structural = Type::Object(object.clone());
                    self.named(id, false, &|walk: &mut Self| walk.visit_members(&structural, depth, true));
                    return;
                }
                if object.alias_name.is_some() {
                    return;
                }
                self.visit_members(ty, depth, written_out);
            }
            _ => {}
        }
    }

    /// Whether `id` is written by name; an inaccessible alias is written out
    /// through `write_out`, and an inaccessible interface or enum (`nominal`)
    /// recorded.
    fn named(&mut self, id: &str, nominal: bool, write_out: &dyn Fn(&mut Self)) -> bool {
        let Some((file, name)) = id.split_once('\0') else {
            return true;
        };
        if file == self.current {
            return true;
        }
        if let Some(module) = self.unportable.get(id) {
            let member = name.rsplit_once('.').map_or(name, |(_, member)| member);
            let entry = (member.to_string(), module.to_string());
            if !self.unportable_names.contains(&entry) {
                self.unportable_names.push(entry);
            }
            return false;
        }
        let kind = self.unexported.get(id).map(|kind| match kind {
            crate::program::UnexportedTypeKind::Alias if nominal => crate::program::UnexportedTypeKind::Interface,
            kind => *kind,
        });
        match kind {
            Some(crate::program::UnexportedTypeKind::Interface) => {
                let module = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
                let module = module.strip_suffix(".d").unwrap_or(module);
                let entry = (name.to_string(), module.to_string());
                if !self.names.contains(&entry) {
                    self.names.push(entry);
                }
                false
            }
            Some(crate::program::UnexportedTypeKind::Alias) => {
                if self.expanded.insert(Arc::from(id)) {
                    write_out(self);
                }
                false
            }
            Some(crate::program::UnexportedTypeKind::Class) | None => true,
        }
    }

    /// An alias's resolution written in place of its name: its own members,
    /// not its name again.
    fn write_out(&mut self, ty: &Type, depth: usize) {
        match ty {
            Type::Object(_) => self.visit_members(ty, depth, true),
            other => self.visit(other, depth, true),
        }
    }

    fn visit_members(&mut self, ty: &Type, depth: usize, written_out: bool) {
        let Type::Object(object) = ty else {
            return;
        };
        for property in object.properties.values() {
            self.visit(&property.ty, depth, written_out);
        }
        for signature in object.call_signature().into_iter().chain(object.construct_signature()) {
            self.visit_signature(signature, depth, written_out);
        }
        for index in object.string_index_type.iter().chain(&object.number_index_type) {
            self.visit(index, depth, written_out);
        }
        // `getIntersectionType` flattens an operand that is itself an
        // intersection, so an alias naming one is not written by its name.
        for operand in object.intersection_operands.iter().flat_map(|operands| operands.iter()) {
            self.visit_operand(operand, depth, written_out);
        }
    }

    fn visit_operand(&mut self, operand: &Type, depth: usize, written_out: bool) {
        if let Type::Reference(reference) = operand
            && let Type::Object(resolved) = reference.resolve()
            && let Some(operands) = &resolved.intersection_operands
            && depth <= MAX_DEPTH
        {
            for operand in operands.iter() {
                self.visit_operand(operand, depth + 1, written_out);
            }
            return;
        }
        self.visit(operand, depth, written_out);
    }

    fn visit_signature(&mut self, function: &FunctionType, depth: usize, written_out: bool) {
        let overloads = function.overloads().unwrap_or_default();
        for signature in std::iter::once(function).chain(overloads) {
            for parameter in signature.parameters() {
                self.visit(parameter, depth, written_out);
            }
            self.visit(signature.return_type(), depth, written_out);
        }
    }
}
