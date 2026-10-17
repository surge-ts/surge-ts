use std::cell::RefCell;
use std::path::{Path, PathBuf};

use surge_ts_types::fx::FxHashMap;

#[derive(Clone)]
struct CanonEntry {
    canonical: std::sync::Arc<str>,
    // Whether `canonical` came from a successful realpath resolution rather
    // than the textual fallback for nonexistent paths; only resolved parents
    // may seed leaf-probe resolution.
    resolved: bool,
}

thread_local! {
    // `std::fs::canonicalize` issues a `realpath()` syscall on every call, and
    // type/module resolution canonicalizes the same handful of paths over and
    // over within a single check. Profiling showed this syscall as the single
    // largest self-time cost. The filesystem and cwd are stable for the
    // duration of a check, so memoizing the result per thread is safe. Worker
    // threads are spawned fresh per run (via `thread::scope`), so their caches
    // never outlive a run; the main thread's cache is cleared at the start of
    // each program check.
    // Keyed by the path's raw bytes: `Path as Hash` normalizes separators
    // component-by-component on every lookup, which showed up in profiles at
    // ~6M lookups/run. Raw-byte identity is stricter than `Path` equality, so
    // at worst two spellings of one path each get their own (identical) entry.
    static CANONICALIZE_CACHE: RefCell<FxHashMap<Box<[u8]>, CanonEntry>> =
        RefCell::new(FxHashMap::default());
}

/// Clears the per-thread path canonicalization cache. Called at the start of a
/// program check so a long-lived process (e.g. a test binary running many
/// checks) never observes a stale canonicalization across runs.
pub(crate) fn clear_canonicalize_cache() {
    CANONICALIZE_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub(crate) fn canonicalize_if_exists_string(path: &Path) -> String {
    canonicalize_if_exists_arc(path).as_ref().to_string()
}

/// Shared-handle variant of [`canonicalize_if_exists_string`]: cache hits are
/// a refcount bump instead of a fresh `String`, which matters on the
/// resolution-key path (millions of lookups per run).
pub(crate) fn canonicalize_if_exists_arc(path: &Path) -> std::sync::Arc<str> {
    crate::program::record_string_path_lookup();
    crate::program::record_canonicalize_call();
    let key = path.as_os_str().as_encoded_bytes();
    let cached = CANONICALIZE_CACHE.with(|cache| cache.borrow().get(key).cloned());
    if let Some(cached) = cached {
        crate::program::record_canonicalize_cache_hit();
        return cached.canonical;
    }
    let entry = resolve_canonical_entry(path);
    CANONICALIZE_CACHE.with(|cache| cache.borrow_mut().insert(key.into(), entry.clone()));
    entry.canonical
}

/// Internal lookup for parent-chain resolution; bypasses the public-call
/// counters so `canonicalize_calls`/`cache_hits` keep meaning "resolution-key
/// requests".
fn canonical_entry(path: &Path) -> CanonEntry {
    let key = path.as_os_str().as_encoded_bytes();
    let cached = CANONICALIZE_CACHE.with(|cache| cache.borrow().get(key).cloned());
    if let Some(cached) = cached {
        return cached;
    }
    let entry = resolve_canonical_entry(path);
    CANONICALIZE_CACHE.with(|cache| cache.borrow_mut().insert(key.into(), entry.clone()));
    entry
}

fn resolve_canonical_entry(path: &Path) -> CanonEntry {
    if let Some(entry) = resolve_via_parent(path) {
        return entry;
    }
    full_realpath_entry(path)
}

fn entry_from_path(canonical: &Path, resolved: bool) -> CanonEntry {
    CanonEntry {
        canonical: std::sync::Arc::from(
            normalize_path_buf(canonical)
                .to_string_lossy()
                .replace('\\', "/"),
        ),
        resolved,
    }
}

fn full_realpath_entry(path: &Path) -> CanonEntry {
    let start = crate::program::counters_enabled().then(std::time::Instant::now);
    let canonical = std::fs::canonicalize(path);
    if let Some(start) = start {
        crate::program::record_canonicalize_syscall(start.elapsed());
    }
    match canonical {
        Ok(canonical) => entry_from_path(&canonical, true),
        Err(_) => entry_from_path(path, false),
    }
}

/// Resolves `path` as canonical-parent + one leaf probe instead of a full
/// `realpath` walk. Returns `None` whenever the answer could differ from
/// `std::fs::canonicalize` — the caller then takes the full walk.
fn resolve_via_parent(path: &Path) -> Option<CanonEntry> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    let name = path.file_name()?;
    // `file_name()` hides trailing separators (`a/b/` -> `b`), but realpath
    // fails those with ENOTDIR when `b` is a regular file; only take the
    // shortcut when the spelling really ends with the leaf name.
    if !path
        .as_os_str()
        .as_encoded_bytes()
        .ends_with(name.as_encoded_bytes())
    {
        return None;
    }
    let parent_entry = canonical_entry(parent);
    if !parent_entry.resolved {
        // realpath fails on the parent chain, so it would fail on `path` too.
        return Some(entry_from_path(path, false));
    }
    let parent_canonical = Path::new(parent_entry.canonical.as_ref());
    let start = crate::program::counters_enabled().then(std::time::Instant::now);
    let probe = surge_ts_types::leaf_probe::probe_leaf(&parent_canonical.join(name));
    if let Some(start) = start {
        crate::program::record_canonicalize_syscall(start.elapsed());
    }
    match probe {
        surge_ts_types::leaf_probe::LeafProbe::Entry {
            name: on_disk_name,
            is_symlink: false,
        } => Some(entry_from_path(&parent_canonical.join(on_disk_name), true)),
        // A symlink leaf still needs realpath to chase the target.
        surge_ts_types::leaf_probe::LeafProbe::Entry {
            is_symlink: true, ..
        } => None,
        surge_ts_types::leaf_probe::LeafProbe::Missing => Some(entry_from_path(path, false)),
        surge_ts_types::leaf_probe::LeafProbe::Unsupported => None,
    }
}

pub(crate) fn normalize_path_string(path: &str) -> String {
    crate::program::record_string_path_lookup();
    normalize_path_buf(Path::new(path))
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalize_path_buf(path: &Path) -> PathBuf {
    let path = path.to_string_lossy().replace('\\', "/");
    // Windows' `canonicalize` answers with verbatim paths (`\\?\C:\…`,
    // `\\?\UNC\server\share`), which no other spelling of the file matches.
    let path = match path.strip_prefix("//?/") {
        Some(rest) => match rest.strip_prefix("UNC/") {
            Some(share) => format!("//{share}"),
            None => rest.to_string(),
        },
        None => path,
    };
    let is_absolute = path.starts_with('/');
    let mut drive_letter = "";

    let path_to_split = if path.chars().nth(1) == Some(':') {
        drive_letter = &path[0..2];
        &path[2..]
    } else {
        &path
    };

    let mut segments = Vec::new();
    for segment in path_to_split.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }

        if segment == ".." {
            if let Some(last) = segments.last() {
                if last != ".." {
                    segments.pop();
                    continue;
                }
            }

            if !is_absolute && drive_letter.is_empty() {
                segments.push(segment.to_string());
            }

            continue;
        }

        segments.push(segment.to_string());
    }

    let mut result = String::new();
    if !drive_letter.is_empty() {
        result.push_str(drive_letter);
        if path_to_split.starts_with('/') {
            result.push('/');
        }
    } else if is_absolute {
        result.push('/');
    }

    result.push_str(&segments.join("/"));

    if result.is_empty() {
        if is_absolute {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        }
    } else {
        PathBuf::from(result)
    }
}
