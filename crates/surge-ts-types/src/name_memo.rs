use std::hash::Hasher;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::fx::FxHashMap;

/// tsc prints at most ~160 characters of a type in a diagnostic, so nothing
/// past this cap is ever compared; rendering it was most of the check time on
/// zod and MCP-SDK projects (an interface's methods re-spelled per instantiation).
pub(crate) const DISPLAY_CAP_BYTES: usize = 1024;

/// Cached rendered display name for an `Arc`-shared type payload. Type graphs
/// share subtrees, but `Type::name` walks the *tree* expansion — deeply nested
/// function/union compositions re-render the same shared payload exponentially
/// many times. Memoizing per payload collapses that walk to the DAG size.
///
/// Transparent to equality: two payloads that differ only in whether their name
/// has been rendered are the same payload, so `eq` is constant `true` (the
/// surrounding derived `PartialEq` still compares every other field).
#[derive(Default)]
pub(crate) struct NameMemo(OnceLock<Arc<str>>);

impl NameMemo {
    pub(crate) fn get_or_render(&self, render: impl FnOnce() -> String) -> Arc<str> {
        if let Some(name) = self.0.get() {
            return name.clone();
        }
        let rendered = render();
        // Names above the cap are abbreviated before caching: on tRPC ~113
        // pathological compositions render 64KB–5MB names totalling ~0.5GB,
        // which no consumer can meaningfully display. Abbreviating at the
        // memo layer means the full string is materialized exactly once per
        // payload, and every enclosing render reads the short form — so the
        // giant-name cascade never rebuilds. The full render's hash keeps the
        // abbreviation injective and deterministic.
        let stored = if rendered.len() > DISPLAY_CAP_BYTES {
            abbreviate(&rendered)
        } else {
            rendered
        };
        let interned = intern_name(stored);
        // A lost set race stores another thread's intern of the same payload's
        // render — deterministic, so the strings are identical.
        let _ = self.0.set(interned.clone());
        interned
    }
}

impl PartialEq for NameMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for NameMemo {}

impl Clone for NameMemo {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for NameMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NameMemo")
            .field(&self.0.get().map(|name| name.len()))
            .finish()
    }
}

/// A display string bounded as a memoized name is (see [`NameMemo`]): a
/// reference instantiated over schema-library shapes spells its arguments,
/// whose own displays spell theirs, and on zod compositions that reached tens
/// of megabytes per instantiation.
pub(crate) fn bounded_display(display: Arc<str>) -> Arc<str> {
    if display.len() > DISPLAY_CAP_BYTES {
        Arc::from(abbreviate(&display))
    } else {
        display
    }
}

/// [`bounded_display`] for a structural rendering, which no payload memoizes.
pub(crate) fn bounded_name(name: String) -> String {
    if name.len() > DISPLAY_CAP_BYTES { abbreviate(&name) } else { name }
}

fn abbreviate(rendered: &str) -> String {
    let mut hasher = crate::fx::FxHasher::default();
    hasher.write(rendered.as_bytes());
    let hash = hasher.finish();
    let mut end = 240;
    while !rendered.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...#{hash:016x}", &rendered[..end])
}

/// Rendered names are deduplicated through a weak intern table: distinct
/// payloads routinely render the same (sometimes multi-megabyte) name, and a
/// per-payload copy retained ~0.5GB on tRPC. Entries are `Weak`, so a name
/// dies with the last payload memo holding it; dead entries are pruned on the
/// bucket they collide with, and [`clear_name_intern_table`] drops the table
/// skeleton in the end-of-run teardown.
static NAME_INTERN_TABLE: OnceLock<Mutex<FxHashMap<u64, Vec<Weak<str>>>>> = OnceLock::new();

fn intern_table() -> &'static Mutex<FxHashMap<u64, Vec<Weak<str>>>> {
    NAME_INTERN_TABLE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

fn intern_name(rendered: String) -> Arc<str> {
    let mut hasher = crate::fx::FxHasher::default();
    hasher.write(rendered.as_bytes());
    let hash = hasher.finish();

    let Ok(mut table) = intern_table().lock() else {
        return Arc::from(rendered);
    };
    let bucket = table.entry(hash).or_default();
    for entry in bucket.iter() {
        if let Some(existing) = entry.upgrade()
            && *existing == *rendered
        {
            return existing;
        }
    }
    let name: Arc<str> = Arc::from(rendered);
    bucket.retain(|entry| entry.strong_count() > 0);
    bucket.push(Arc::downgrade(&name));
    record_intern_stats(name.len());
    name
}

static INTERN_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INTERN_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INTERN_MAX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INTERN_OVER_64K: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INTERN_OVER_1M: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn record_intern_stats(len: usize) {
    use std::sync::atomic::Ordering;
    INTERN_COUNT.fetch_add(1, Ordering::Relaxed);
    INTERN_BYTES.fetch_add(len as u64, Ordering::Relaxed);
    INTERN_MAX.fetch_max(len as u64, Ordering::Relaxed);
    if len > 64 * 1024 {
        INTERN_OVER_64K.fetch_add(1, Ordering::Relaxed);
    }
    if len > 1024 * 1024 {
        INTERN_OVER_1M.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn clear_name_intern_table() {
    use std::sync::atomic::Ordering;
    if std::env::var_os("SURGE_NAME_INTERN_STATS").is_some() {
        eprintln!(
            "NAME INTERN STATS distinct={} bytes={} max={} over_64k={} over_1m={}",
            INTERN_COUNT.load(Ordering::Relaxed),
            INTERN_BYTES.load(Ordering::Relaxed),
            INTERN_MAX.load(Ordering::Relaxed),
            INTERN_OVER_64K.load(Ordering::Relaxed),
            INTERN_OVER_1M.load(Ordering::Relaxed),
        );
    }
    if let Ok(mut table) = intern_table().lock() {
        table.clear();
        table.shrink_to_fit();
    }
}
