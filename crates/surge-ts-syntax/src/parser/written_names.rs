//! Member names written as anything but an identifier, as their source text.
//! tsc names such a member in a diagnostic by its declaration's name as written
//! (`declarationNameToString`): `"1"`, `3.0`, `["get1"]`, where the member's
//! key is `1`, `3`, `get1`.

use oxc_ast::ast::{
    AccessorProperty, MethodDefinition, ObjectProperty, Program, PropertyDefinition, PropertyKey, TSMethodSignature,
    TSPropertySignature,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;

use crate::WrittenMemberName;

pub(crate) fn collect_written_member_names(program: &Program<'_>, source_text: &str) -> Vec<WrittenMemberName> {
    let mut collector = Collector { source_text, found: Vec::new() };
    collector.visit_program(program);
    collector.found.sort_by_key(|name| name.start);
    collector.found
}

struct Collector<'s> {
    source_text: &'s str,
    found: Vec<WrittenMemberName>,
}

impl Collector<'_> {
    fn record(&mut self, key: &PropertyKey<'_>, computed: bool) {
        if !computed && !matches!(key, PropertyKey::StringLiteral(_) | PropertyKey::NumericLiteral(_)) {
            return;
        }
        let span = key.span();
        let Some(text) = self.source_text.get(span.start as usize..span.end as usize) else {
            return;
        };
        let text = if computed { format!("[{text}]") } else { text.to_string() };
        // A computed name is looked up by its key or by its brackets.
        if computed
            && let Some(open) = self.source_text.get(..span.start as usize).and_then(|before| before.rfind('['))
        {
            self.found.push(WrittenMemberName { start: open as u32, text: text.clone().into() });
        }
        self.found.push(WrittenMemberName { start: span.start, text: text.into() });
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_property_definition(self, it);
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_method_definition(self, it);
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_accessor_property(self, it);
    }

    fn visit_ts_property_signature(&mut self, it: &TSPropertySignature<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_ts_property_signature(self, it);
    }

    fn visit_ts_method_signature(&mut self, it: &TSMethodSignature<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_ts_method_signature(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        self.record(&it.key, it.computed);
        walk::walk_object_property(self, it);
    }

    fn visit_function(&mut self, it: &oxc_ast::ast::Function<'a>, flags: ScopeFlags) {
        walk::walk_function(self, it, flags);
    }
}
