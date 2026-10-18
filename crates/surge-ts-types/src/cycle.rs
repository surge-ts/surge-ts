//! Cycle markers, for the compiler API's `typeToString`.
//!
//! tsc's types are a graph: `const f = () => f` has a type whose return type
//! is itself, and its printer writes a type it meets again on the path it is
//! printing as `...`. surge's types are trees, so the checker cuts such a
//! cycle where it reads the type back — at `any`, or a copy from an earlier
//! pass. While serving the API it wraps that read in a marker: a one-member
//! union of what it read (so every type operation sees exactly that type)
//! named after the declaration it refers back to. Printed under tsc display
//! with that declaration's type at hand, a marker is `...` where that type is
//! already being printed, and that type, once, where it is not.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{Type, UnionType};

const PREFIX: &str = "...\u{1}";
const KEY_START: char = '\u{1}';
const KEY_END: char = '\u{2}';

thread_local! {
    static REFERENTS: RefCell<Option<Arc<HashMap<String, Type>>>> = const { RefCell::new(None) };
    /// The function and object types being printed, outermost first, with
    /// their canonical text once a marker compares against them.
    static ANCESTORS: RefCell<Vec<(Type, Option<String>)>> = const { RefCell::new(Vec::new()) };
    static EXPANDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CANONICAL: Cell<bool> = const { Cell::new(false) };
    static MARKERS_RENDERED: Cell<usize> = const { Cell::new(0) };
    static FRESH_ROOT: Cell<bool> = const { Cell::new(false) };
    static PROPERTY_VALUE: Cell<bool> = const { Cell::new(false) };
    static MISSING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `inner`, marked as a read of the declaration `key` names back into its
/// own type.
pub fn cycle_marker(inner: Type, key: &str) -> Type {
    if cycle_marker_key(&inner) == Some(key) {
        return inner;
    }
    Type::Union(UnionType::new(vec![inner]).with_alias_name(format!("{PREFIX}{key}")))
}

pub fn cycle_marker_key(ty: &Type) -> Option<&str> {
    match ty {
        Type::Union(union) => union.alias_name().and_then(key_of_alias),
        _ => None,
    }
}

pub(crate) fn key_of_alias(alias: &str) -> Option<&str> {
    alias.strip_prefix(PREFIX)
}

/// Runs `render` with the types `referents` gives each marker's declaration,
/// and answers what it printed with the declarations it met a marker of and
/// had no type for (printed as what the marker wraps).
pub fn with_cycle_referents<R>(referents: HashMap<String, Type>, render: impl FnOnce() -> R) -> (R, Vec<String>) {
    struct Restore(Option<Arc<HashMap<String, Type>>>, Vec<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REFERENTS.with(|referents| *referents.borrow_mut() = self.0.take());
            MISSING.with(|missing| *missing.borrow_mut() = std::mem::take(&mut self.1));
        }
    }
    let _restore = Restore(
        REFERENTS.with(|current| current.borrow_mut().replace(Arc::new(referents))),
        MISSING.with(|missing| std::mem::take(&mut *missing.borrow_mut())),
    );
    let rendered = render();
    (rendered, MISSING.with(|missing| std::mem::take(&mut *missing.borrow_mut())))
}

/// Runs `render` with the type it prints first standing apart from what its
/// markers read: an object literal expression's type, which tsc's fresh copy
/// of the literal is, where `this` and the binding read the regular one.
pub fn with_fresh_cycle_root<R>(fresh: bool, render: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            FRESH_ROOT.with(|fresh| fresh.set(self.0));
        }
    }
    let _restore = Restore(FRESH_ROOT.with(|current| current.replace(fresh)));
    render()
}

/// `ty` as printed with its markers named by their declarations, which two
/// copies of one cyclic type print alike however deep each was unrolled.
fn canonical_name(ty: &Type) -> String {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANONICAL.with(|canonical| canonical.set(self.0));
        }
    }
    let _restore = Restore(CANONICAL.with(|canonical| canonical.replace(true)));
    crate::with_tsc_display(|| crate::tsc_display::uncounted(|| crate::ty::without_structural_budget(|| ty.name())))
}

pub(crate) fn canonical() -> bool {
    CANONICAL.with(Cell::get)
}

/// How many markers printed through their referents so far: a rendering
/// that printed one depends on the path it was printed on.
pub(crate) fn markers_rendered() -> usize {
    MARKERS_RENDERED.with(Cell::get)
}

fn tracking() -> bool {
    !canonical() && REFERENTS.with(|referents| referents.borrow().is_some())
}

/// Renders a function or object type as an ancestor of what it contains.
pub(crate) fn as_ancestor(ty: &Type, render: impl FnOnce() -> String) -> String {
    if !tracking() || FRESH_ROOT.with(|fresh| fresh.replace(false)) {
        return render();
    }
    // Inside a marker's expansion a property's type can be one being printed
    // already, which tsc elides as it does any type node it revisits (a plain
    // tree never contains its own ancestor). A method member prints as a
    // signature, which tsc does not check so.
    let mut canonical = None;
    if PROPERTY_VALUE.with(|value| value.replace(false)) && EXPANDING.with(|expanding| !expanding.borrow().is_empty()) {
        let own = canonical_name(ty);
        if is_ancestor(&own) {
            MARKERS_RENDERED.with(|count| count.set(count.get() + 1));
            crate::tsc_display::add_length(3);
            return "...".to_string();
        }
        canonical = Some(own);
    }
    ANCESTORS.with(|ancestors| ancestors.borrow_mut().push((ty.clone(), canonical)));
    let name = render();
    ANCESTORS.with(|ancestors| ancestors.borrow_mut().pop());
    name
}

/// Renders a non-method property's type.
pub(crate) fn property_value(render: impl FnOnce() -> String) -> String {
    PROPERTY_VALUE.with(|value| value.set(true));
    let name = render();
    PROPERTY_VALUE.with(|value| value.set(false));
    name
}

/// Whether a type printing as `canonical` is being printed already.
fn is_ancestor(canonical: &str) -> bool {
    let pending: Vec<(usize, Type)> = ANCESTORS.with(|ancestors| {
        ancestors
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (_, canonical))| canonical.is_none())
            .map(|(index, (ty, _))| (index, ty.clone()))
            .collect()
    });
    for (index, ty) in pending {
        let text = canonical_name(&ty);
        ANCESTORS.with(|ancestors| ancestors.borrow_mut()[index].1 = Some(text));
    }
    ANCESTORS.with(|ancestors| ancestors.borrow().iter().any(|(_, ancestor)| ancestor.as_deref() == Some(canonical)))
}

pub(crate) fn render_marker(key: &str, inner: &Type) -> String {
    if canonical() {
        return format!("{KEY_START}{key}{KEY_END}");
    }
    let resolved = crate::tsc_display::active()
        .then(|| REFERENTS.with(|referents| referents.borrow().as_ref().map(|map| map.get(key).cloned())))
        .flatten();
    let resolved = match resolved {
        Some(Some(resolved)) => resolved,
        Some(None) => {
            MISSING.with(|missing| {
                let mut missing = missing.borrow_mut();
                if !missing.iter().any(|known| known == key) {
                    missing.push(key.to_string());
                }
            });
            return inner.name();
        }
        None => return inner.name(),
    };
    MARKERS_RENDERED.with(|count| count.set(count.get() + 1));
    let revisit = is_ancestor(&canonical_name(&resolved))
        || EXPANDING.with(|expanding| expanding.borrow().iter().any(|expanding| expanding == key));
    if revisit {
        crate::tsc_display::add_length(3);
        return "...".to_string();
    }
    EXPANDING.with(|expanding| expanding.borrow_mut().push(key.to_string()));
    let name = resolved.name();
    EXPANDING.with(|expanding| expanding.borrow_mut().pop());
    name
}
