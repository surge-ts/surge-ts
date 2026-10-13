use std::sync::Arc;

use surge_ts_types::Type;

use super::{LazyAnnotationContent, LazySignatureComponent, MODULE_INSTANTIATION_MEMO_TAG};
use crate::context::CheckerContext;
use crate::infer::types::*;

/// Everything that identifies one deferred interface member: the declaring
/// interface, the member's position in the merged member list (same-named
/// members from declaration-merged fragments are distinct annotations), and
/// the substitution the enclosing expansion resolved under. `component` is
/// `None` for a whole property member and `Some` for one component of a
/// method signature.
#[derive(Clone, Copy)]
pub(crate) struct LazyMemberIdentity<'a> {
    pub(crate) interface_name: &'a str,
    pub(crate) declaration_start: usize,
    pub(crate) member_index: usize,
    pub(crate) member_name: &'a str,
    pub(crate) component: Option<LazySignatureComponent>,
    pub(crate) substitution_fingerprint: u64,
}

impl LazyMemberIdentity<'_> {
    pub(super) fn component_identity(&self) -> String {
        self.component.map_or_else(String::new, |component| {
            format!(":{}", component.identity())
        })
    }

    pub(super) fn key_name(&self, kind: &str) -> String {
        format!(
            "{kind} {}@{}#{}.{}{}",
            self.interface_name,
            self.declaration_start,
            self.member_index,
            self.member_name,
            self.component_identity()
        )
    }

    pub(super) fn reference_id(&self, tag: &str, file_name: &str) -> String {
        format!(
            "{file_name}\u{0}{tag}\u{0}{}\u{0}{}\u{0}{}\u{0}{}{}\u{0}{:016x}",
            self.interface_name,
            self.declaration_start,
            self.member_index,
            self.member_name,
            self.component_identity(),
            self.substitution_fingerprint
        )
    }

    pub(super) fn digest(&self, file_name: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = surge_ts_types::fx::FxHasher::default();
        hasher.write(file_name.as_bytes());
        hasher.write(self.interface_name.as_bytes());
        hasher.write_usize(self.declaration_start);
        hasher.write_usize(self.member_index);
        hasher.write(self.member_name.as_bytes());
        self.component.hash(&mut hasher);
        hasher.write_u64(self.substitution_fingerprint);
        hasher.finish()
    }

    pub(super) fn matches(&self, entry: &LazyMemberTemplateEntry, file_name: &str) -> bool {
        self.declaration_start == entry.declaration_start
            && self.member_index == entry.member_index
            && self.substitution_fingerprint == entry.substitution_fingerprint
            && self.component == entry.component
            && self.member_name == entry.member_name.as_ref()
            && self.interface_name == entry.interface_name.as_ref()
            && file_name == entry.file_name.as_ref()
    }
}

/// One interned member content plus the identity fields the bucket scan
/// verifies against — the map is keyed by a digest, so a colliding digest must
/// never hand back another member's annotation.
pub(crate) struct LazyMemberTemplateEntry {
    pub(super) file_name: Arc<str>,
    pub(super) interface_name: Box<str>,
    pub(super) declaration_start: usize,
    pub(super) member_index: usize,
    pub(super) member_name: Box<str>,
    pub(super) component: Option<LazySignatureComponent>,
    pub(super) substitution_fingerprint: u64,
    pub(super) content: Arc<LazyAnnotationContent>,
}

impl std::fmt::Debug for LazyMemberTemplateEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyMemberTemplateEntry")
            .field("id", &self.content.id)
            .finish_non_exhaustive()
    }
}

pub(crate) type LazyMemberTemplateTable =
    surge_ts_types::fx::FxHashMap<u64, Vec<LazyMemberTemplateEntry>>;

/// Content-addressed interning for deferred member annotations. The same
/// library member is deferred once per expansion of its interface — on tRPC
/// 52.8k creates carry only 1.7k distinct contents — and every create formats
/// two identity strings, walks the annotation for its display, and clones the
/// annotation and the substitution. Interning collapses all of that to a
/// digest lookup; the environment, creation scope and memo slots stay
/// per-capture, so a shared content never shares a resolution.
pub(super) fn intern_lazy_member_content(
    ctx: &mut CheckerContext,
    identity: LazyMemberIdentity<'_>,
    build: impl FnOnce(Arc<str>, &[String]) -> Arc<LazyAnnotationContent>,
) -> Arc<LazyAnnotationContent> {
    let file_name = ctx.canonical_file_name_arc();
    let digest = identity.digest(&file_name);
    let table = ctx.lazy_member_annotation_templates.clone();
    let Ok(mut templates) = table.lock() else {
        return build(file_name, &ctx.namespace_member_prefix_stack);
    };
    let bucket: &mut Vec<LazyMemberTemplateEntry> = templates.entry(digest).or_default();
    if let Some(entry) = bucket
        .iter()
        .find(|entry| identity.matches(entry, &file_name))
    {
        crate::program::record_program_counter(|c| c.lazy_member_template_hit_count += 1);
        return entry.content.clone();
    }
    crate::program::record_program_counter(|c| c.lazy_member_template_miss_count += 1);
    let content = build(file_name.clone(), &ctx.namespace_member_prefix_stack);
    bucket.push(LazyMemberTemplateEntry {
        file_name,
        interface_name: Box::from(identity.interface_name),
        declaration_start: identity.declaration_start,
        member_index: identity.member_index,
        member_name: Box::from(identity.member_name),
        component: identity.component,
        substitution_fingerprint: identity.substitution_fingerprint,
        content: content.clone(),
    });
    content
}

pub(crate) fn member_substitution_fingerprint(substitution: &TypeParameterSubstitution) -> u64 {
    use std::hash::Hasher;
    let mut entries = substitution.iter().peekable();
    if entries.peek().is_none() {
        return 0;
    }
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    for (name, ty) in entries {
        hasher.write(name.as_bytes());
        hasher.write_u8(u8::from(substitution.is_placeholder(name)));
        hasher.write_u64(crate::speculative::display_type_fingerprint(ty));
    }
    // High bit clear keeps the key disjoint from the module-memo tag.
    hasher.finish() & !MODULE_INSTANTIATION_MEMO_TAG
}

/// Syntactic twin of [`physical_interface_method_has_contextual_typing_dependency`]
/// for methods whose components defer: any real (non-`this`) parameter whose
/// ANNOTATION spells a callable. Matches the resolved-side classifier exactly,
/// because that classifier has no `Type::Reference` arm — a named annotation
/// resolving to a callable already classified as `false` there.
pub(crate) fn parsed_method_has_contextual_typing_dependency(
    function: &surge_ts_syntax::ParsedFunctionType,
) -> bool {
    fn contains_callable(annotation: &surge_ts_syntax::ParsedType, depth: usize) -> bool {
        use surge_ts_syntax::ParsedType;
        if depth >= 32 {
            return true;
        }
        match annotation {
            ParsedType::Function(_) => true,
            ParsedType::Object(object) => {
                object.call_signature.is_some()
                    || object
                        .properties
                        .iter()
                        .any(|property| contains_callable(&property.ty, depth + 1))
            }
            ParsedType::Array(element)
            | ParsedType::KeyOf(element)
            | ParsedType::Readonly(element) => contains_callable(element, depth + 1),
            ParsedType::Tuple(elements) | ParsedType::Union(elements) => {
                elements.iter().any(|e| contains_callable(e, depth + 1))
            }
            ParsedType::VariadicTuple(elements) => elements.iter().any(|element| {
                let (surge_ts_syntax::ParsedTupleElement::Fixed(ty)
                | surge_ts_syntax::ParsedTupleElement::Rest(ty, _)) = element;
                contains_callable(ty, depth + 1)
            }),
            _ => false,
        }
    }
    function
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .any(|parameter| contains_callable(&parameter.ty, 0))
}

/// Opt-in probe filter (`SURGE_LAZY_VALUE_TRACE=<substr>`), read once — the
/// trace sites sit on resolution paths where per-call `getenv` is prohibited.
pub(crate) fn lazy_value_trace_filter() -> Option<&'static str> {
    static FILTER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FILTER
        .get_or_init(|| std::env::var("SURGE_LAZY_VALUE_TRACE").ok())
        .as_deref()
}

/// Debug shape for the `SURGE_LAZY_VALUE_TRACE` probe: variant + display +
/// (for objects / peeled references) the member-name list.
pub(crate) fn lazy_value_trace_shape(ty: &Type) -> String {
    fn describe(ty: &Type, force: bool) -> String {
        match ty {
            Type::Object(object) => {
                let mut names: Vec<&str> = object.properties.keys().map(|k| k.as_ref()).collect();
                names.sort_unstable();
                format!(
                    "Object{{{} props: {}}} call={}",
                    names.len(),
                    names.join(","),
                    object.call_signature.is_some(),
                )
            }
            Type::Reference(reference) => {
                if force {
                    let peeled = reference.resolve().peeled();
                    format!(
                        "Reference({}) -> {}",
                        reference.display,
                        describe(&peeled, false)
                    )
                } else {
                    format!("Reference({})", reference.display)
                }
            }
            other => format!("{other:?}").chars().take(120).collect(),
        }
    }
    describe(ty, true)
}

pub(super) fn parsed_annotation_display(annotation: &surge_ts_syntax::ParsedType) -> String {
    use surge_ts_syntax::ParsedType;

    match annotation {
        ParsedType::String => "string".to_string(),
        ParsedType::Number => "number".to_string(),
        ParsedType::Boolean => "boolean".to_string(),
        ParsedType::BigInt => "bigint".to_string(),
        ParsedType::Symbol => "symbol".to_string(),
        ParsedType::Undefined => "undefined".to_string(),
        ParsedType::Null => "null".to_string(),
        ParsedType::Void => "void".to_string(),
        ParsedType::Any => "any".to_string(),
        ParsedType::ErrorType | ParsedType::Unknown | ParsedType::UnknownKeyword => {
            "unknown".to_string()
        }
        ParsedType::Never => "never".to_string(),
        ParsedType::StringLiteral(value) => format!("\"{value}\""),
        ParsedType::NumberLiteral(value) => value.clone(),
        ParsedType::BooleanLiteral(value) => value.to_string(),
        ParsedType::Named(named) => {
            if named.type_arguments.is_empty() {
                named.name.clone()
            } else {
                format!(
                    "{}<{}>",
                    named.name,
                    named
                        .type_arguments
                        .iter()
                        .map(parsed_annotation_display)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        ParsedType::Array(element) => {
            let display = parsed_annotation_display(element);
            if matches!(
                element.as_ref(),
                ParsedType::Union(_)
                    | ParsedType::Intersection(_)
                    | ParsedType::Function(_)
                    | ParsedType::Conditional(_)
            ) {
                format!("({display})[]")
            } else {
                format!("{display}[]")
            }
        }
        ParsedType::Tuple(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(parsed_annotation_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ParsedType::VariadicTuple(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(|element| match element {
                    surge_ts_syntax::ParsedTupleElement::Fixed(ty) => parsed_annotation_display(ty),
                    surge_ts_syntax::ParsedTupleElement::Rest(ty, _) =>
                        format!("...{}", parsed_annotation_display(ty)),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ParsedType::Readonly(inner) => format!("readonly {}", parsed_annotation_display(inner)),
        ParsedType::Union(members) => members
            .iter()
            .map(parsed_annotation_display)
            .collect::<Vec<_>>()
            .join(" | "),
        ParsedType::Intersection(members) => members
            .iter()
            .map(parsed_annotation_display)
            .collect::<Vec<_>>()
            .join(" & "),
        ParsedType::TypeOf(query) => {
            let suffix = query
                .members
                .iter()
                .map(|member| format!(".{member}"))
                .collect::<String>();
            format!("typeof {}{suffix}", query.name)
        }
        ParsedType::KeyOf(inner) => format!("keyof {}", parsed_annotation_display(inner)),
        ParsedType::IndexedAccess(indexed) => format!(
            "{}[{}]",
            parsed_annotation_display(&indexed.object_type),
            parsed_annotation_display(&indexed.index_type)
        ),
        ParsedType::Function(function) => {
            let type_parameters = if function.type_parameters.is_empty() {
                String::new()
            } else {
                format!(
                    "<{}>",
                    function
                        .type_parameters
                        .iter()
                        .map(|parameter| {
                            let mut display = parameter.name.clone();
                            if let Some(constraint) = &parameter.constraint {
                                display.push_str(" extends ");
                                display.push_str(&parsed_annotation_display(constraint));
                            }
                            if let Some(default_type) = &parameter.default_type {
                                display.push_str(" = ");
                                display.push_str(&parsed_annotation_display(default_type));
                            }
                            display
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let parameters = function
                .parameters
                .iter()
                .map(|parameter| {
                    let rest = if parameter.rest { "..." } else { "" };
                    let name = parameter.name.as_deref().unwrap_or("arg");
                    let optional = if parameter.optional { "?" } else { "" };
                    format!(
                        "{rest}{name}{optional}: {}",
                        parsed_annotation_display(&parameter.ty)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{type_parameters}({parameters}) => {}",
                parsed_annotation_display(&function.return_type)
            )
        }
        ParsedType::Object(object) => {
            let mut members = object
                .properties
                .iter()
                .map(|property| {
                    let optional = if property.optional { "?" } else { "" };
                    format!(
                        "{}{optional}: {}",
                        property.name,
                        parsed_annotation_display(&property.ty)
                    )
                })
                .collect::<Vec<_>>();
            if let Some(call) = &object.call_signature {
                members.push(parsed_annotation_display(&ParsedType::Function(
                    std::sync::Arc::new(call.as_ref().clone()),
                )));
            }
            format!("{{ {} }}", members.join("; "))
        }
        ParsedType::Mapped(mapped) => {
            let optional = match mapped.optional {
                surge_ts_syntax::MappedOptionality::Keep => "",
                surge_ts_syntax::MappedOptionality::Add => "?",
                surge_ts_syntax::MappedOptionality::Remove => "-?",
            };
            format!(
                "{{ [{} in {}]{optional}: {} }}",
                mapped.key_name,
                parsed_annotation_display(&mapped.constraint),
                parsed_annotation_display(&mapped.value_type)
            )
        }
        ParsedType::Conditional(conditional) => format!(
            "{} extends {} ? {} : {}",
            parsed_annotation_display(&conditional.check_type),
            parsed_annotation_display(&conditional.extends_type),
            parsed_annotation_display(&conditional.true_type),
            parsed_annotation_display(&conditional.false_type)
        ),
        ParsedType::TemplateLiteral(template) => {
            let mut display = String::from("`");
            for (index, quasi) in template.quasis.iter().enumerate() {
                display.push_str(quasi);
                if let Some(interpolation) = template.interpolations.get(index) {
                    display.push_str("${");
                    display.push_str(&parsed_annotation_display(interpolation));
                    display.push('}');
                }
            }
            display.push('`');
            display
        }
        ParsedType::Infer(infer) => match &infer.constraint {
            Some(constraint) => format!("infer {} extends {}", infer.name, parsed_annotation_display(constraint)),
            None => format!("infer {}", infer.name),
        },
        ParsedType::UniqueSymbol(name) if name.ends_with("\u{0}member") => "unique symbol".to_string(),
        ParsedType::UniqueSymbol(name) => format!("typeof {}", name.split('\u{0}').next().unwrap_or(name)),
        ParsedType::InferredMember(member) => {
            format!("{}[\"{}\"]", member.class_name, member.member_name)
        }
        ParsedType::Predicate(predicate) => match &predicate.ty {
            Some(ty) => format!(
                "{}{} is {}",
                if predicate.asserts { "asserts " } else { "" },
                predicate.parameter_name,
                parsed_annotation_display(ty)
            ),
            None => format!("asserts {}", predicate.parameter_name),
        },
    }
}
