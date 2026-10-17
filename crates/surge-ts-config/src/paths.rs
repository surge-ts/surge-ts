use std::{
    cell::RefCell,
    env,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use surge_ts_types::fx::FxHashMap;

static CANONICALIZE_MEMO_MISSES: AtomicU64 = AtomicU64::new(0);
static CANONICALIZE_FULL_REALPATHS: AtomicU64 = AtomicU64::new(0);
static CANONICALIZE_LEAF_PROBES: AtomicU64 = AtomicU64::new(0);
static CANONICALIZE_MISS_NANOS: AtomicU64 = AtomicU64::new(0);

/// Process-global canonicalization counters, mirroring the loader's
/// `io_stats` pattern: callers snapshot before and after a phase and report
/// the delta.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalizeIoSnapshot {
    pub memo_misses: u64,
    pub full_realpaths: u64,
    pub leaf_probes: u64,
    pub miss_io: Duration,
}

pub fn canonicalize_io_snapshot() -> CanonicalizeIoSnapshot {
    CanonicalizeIoSnapshot {
        memo_misses: CANONICALIZE_MEMO_MISSES.load(Ordering::Relaxed),
        full_realpaths: CANONICALIZE_FULL_REALPATHS.load(Ordering::Relaxed),
        leaf_probes: CANONICALIZE_LEAF_PROBES.load(Ordering::Relaxed),
        miss_io: Duration::from_nanos(CANONICALIZE_MISS_NANOS.load(Ordering::Relaxed)),
    }
}

#[derive(Clone)]
struct CanonEntry {
    canonical: PathBuf,
    // The forward-slash string form, computed once per unique path: the
    // loader's hottest callers want the string, and rebuilding it on every
    // cache hit costs an allocation plus a rescan.
    canonical_str: String,
    // Whether `canonical` came from a successful realpath resolution (as
    // opposed to the textual-normalization fallback for paths that do not
    // exist). Only resolved parents may seed leaf-probe resolution: a
    // fallback parent would join real-looking children onto a path realpath
    // never blessed.
    resolved: bool,
}

impl CanonEntry {
    fn new(canonical: PathBuf, resolved: bool) -> Self {
        let canonical_str = canonical.to_string_lossy().replace('\\', "/");
        Self {
            canonical,
            canonical_str,
            resolved,
        }
    }
}

thread_local! {
    // `std::fs::canonicalize` issues a `realpath()` syscall on every call.
    // Project discovery (config loading, package-entrypoint resolution, the
    // import-graph fixpoint) canonicalizes the same paths repeatedly, and
    // profiling showed the syscall as a top cost. The filesystem is stable for
    // the duration of a run, so memoizing per thread is safe.
    static CANONICALIZE_CACHE: RefCell<FxHashMap<PathBuf, CanonEntry>> =
        RefCell::new(FxHashMap::default());
}

/// Drops the calling thread's canonicalization memo. The cache is only valid
/// while the filesystem is stable, and without a reset a long-lived process
/// that loads many projects accumulates every path it ever canonicalized.
/// Called at the start of each project load.
pub fn clear_canonicalize_cache() {
    CANONICALIZE_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub fn resolve_project_path(project: &Path) -> (PathBuf, PathBuf) {
    let project = absolutize(project);

    if project.exists() && project.is_file() {
        let project = canonicalize_if_exists(&project);
        let root_dir = project
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project.clone());
        return (project, root_dir);
    }

    if project.exists() && project.is_dir() {
        let project = canonicalize_if_exists(&project);
        let config_path = project.join("tsconfig.json");
        return (config_path, project);
    }

    if project
        .file_name()
        .is_some_and(|name| name == "tsconfig.json")
        || project.extension().is_some_and(|ext| ext == "json")
    {
        let root_dir = project
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        (project, root_dir)
    } else {
        let config_path = project.join("tsconfig.json");
        (config_path, project)
    }
}

pub fn resolve_path(base_dir: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

pub fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    match env::current_dir() {
        Ok(current_dir) => current_dir.join(path),
        Err(_) => path.to_path_buf(),
    }
}

fn with_canonical_entry<R>(path: &Path, read: impl FnOnce(&CanonEntry) -> R) -> R {
    let cached = CANONICALIZE_CACHE.with(|cache| cache.borrow().get(path).cloned());
    if let Some(cached) = cached {
        return read(&cached);
    }
    CANONICALIZE_MEMO_MISSES.fetch_add(1, Ordering::Relaxed);
    let entry = resolve_canonical_entry(path);
    let value = read(&entry);
    CANONICALIZE_CACHE.with(|cache| cache.borrow_mut().insert(path.to_path_buf(), entry));
    value
}

fn resolve_canonical_entry(path: &Path) -> CanonEntry {
    if let Some(entry) = resolve_via_parent(path) {
        return entry;
    }
    full_realpath_entry(path)
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
    let (parent_canonical, parent_resolved) =
        with_canonical_entry(parent, |entry| (entry.canonical.clone(), entry.resolved));
    if !parent_resolved {
        // realpath fails on the parent chain, so it would fail on `path` too.
        return Some(CanonEntry::new(normalize_path_buf(path), false));
    }
    let probe_start = std::time::Instant::now();
    CANONICALIZE_LEAF_PROBES.fetch_add(1, Ordering::Relaxed);
    let probe = surge_ts_types::leaf_probe::probe_leaf(&parent_canonical.join(name));
    CANONICALIZE_MISS_NANOS.fetch_add(probe_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    match probe {
        surge_ts_types::leaf_probe::LeafProbe::Entry {
            name: on_disk_name,
            is_symlink: false,
        } => Some(CanonEntry::new(
            normalize_path_buf(&parent_canonical.join(on_disk_name)),
            true,
        )),
        // A symlink leaf still needs realpath to chase the target.
        surge_ts_types::leaf_probe::LeafProbe::Entry {
            is_symlink: true, ..
        } => None,
        surge_ts_types::leaf_probe::LeafProbe::Missing => {
            Some(CanonEntry::new(normalize_path_buf(path), false))
        }
        surge_ts_types::leaf_probe::LeafProbe::Unsupported => None,
    }
}

fn full_realpath_entry(path: &Path) -> CanonEntry {
    let syscall_start = std::time::Instant::now();
    CANONICALIZE_FULL_REALPATHS.fetch_add(1, Ordering::Relaxed);
    let result = std::fs::canonicalize(path);
    CANONICALIZE_MISS_NANOS.fetch_add(syscall_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    match result {
        Ok(canonical) => CanonEntry::new(normalize_path_buf(&canonical), true),
        Err(_) => CanonEntry::new(normalize_path_buf(path), false),
    }
}

pub fn canonicalize_if_exists(path: &Path) -> PathBuf {
    with_canonical_entry(path, |entry| entry.canonical.clone())
}

pub fn canonicalize_if_exists_string(path: &Path) -> String {
    with_canonical_entry(path, |entry| entry.canonical_str.clone())
}

pub fn cycle_key(path: &Path) -> PathBuf {
    canonicalize_if_exists(path)
}

pub fn normalize_path_string(path: &str) -> String {
    normalize_path_buf(Path::new(path))
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn normalize_path_buf(path: &Path) -> PathBuf {
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
