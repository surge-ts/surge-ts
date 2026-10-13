//! Cached file-existence probing for the loader's resolution passes.
//!
//! Package entrypoint resolution fans each extensionless target out to many
//! candidate paths and re-probes the same candidates from every importer
//! directory that reaches the package. The filesystem is static for the
//! duration of a run, so one `metadata` syscall per unique path answers every
//! later probe. The cache is cleared at the start of `Project::check` so test
//! processes that rebuild fixture trees between checks never observe a stale
//! answer.
//!
//! Directories that keep absorbing distinct-path misses (icon barrels and
//! other flat re-export packages fan a single directory out to thousands of
//! candidate stats) switch to a one-shot `read_dir` listing: after
//! [`DIR_LISTING_THRESHOLD`] unique probes into the same parent, later probes
//! are answered from the listing without a syscall. The listing only answers
//! definitively when it can do so on both case-sensitive and case-insensitive
//! filesystems: an exact-name plain-file entry is `true`, no case-folded match
//! at all is `false`, and everything else (case-only matches, symlinks,
//! unknown d_types, non-ASCII names) falls back to `metadata`.

use std::cell::RefCell;
use std::path::Path;
use surge_ts_types::fx::{FxHashMap, FxHashSet};

const DIR_LISTING_THRESHOLD: u32 = 16;

#[derive(Default)]
struct DirListing {
    /// Exact-byte names of plain-file entries.
    files: FxHashSet<Box<[u8]>>,
    /// ASCII-lowercased names of every entry the listing cannot rule out:
    /// all files (a case-only match may still resolve on a case-insensitive
    /// filesystem) plus symlinks and unknown d_types (their follow-target
    /// file-ness is never guessed).
    maybe_lower: FxHashSet<Box<[u8]>>,
    /// `read_dir` failed (permissions, transient error): the listing proves
    /// nothing, every probe stays on the `metadata` path.
    unlisted: bool,
}

#[derive(Default)]
struct ProbeCache {
    paths: FxHashMap<Box<[u8]>, bool>,
    dir_probe_counts: FxHashMap<Box<[u8]>, u32>,
    dir_listings: FxHashMap<Box<[u8]>, DirListing>,
    /// Whether a directory exists, memoized with its ancestors: resolution
    /// walks probe deep candidate paths under ancestor `node_modules`
    /// directories that don't exist, and one cached negative answer high in
    /// the chain rules out every candidate below it without a syscall.
    dir_exists: FxHashMap<Box<[u8]>, bool>,
}

fn dir_exists(cache: &mut ProbeCache, dir: &Path) -> bool {
    let key = dir.as_os_str().as_encoded_bytes();
    if let Some(&hit) = cache.dir_exists.get(key) {
        return hit;
    }
    let exists = match dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() && !dir_exists(cache, parent) => false,
        _ => {
            let probe_start = std::time::Instant::now();
            let is_dir = crate::host_fs::is_dir(dir);
            crate::io_stats::record_existence_probe(probe_start.elapsed());
            is_dir
        }
    };
    cache
        .dir_exists
        .insert(dir.as_os_str().as_encoded_bytes().into(), exists);
    exists
}

thread_local! {
    static PROBE_CACHE: RefCell<ProbeCache> = RefCell::new(ProbeCache::default());
}

pub(crate) fn clear_probe_cache() {
    PROBE_CACHE.with(|cache| *cache.borrow_mut() = ProbeCache::default());
}

fn stat_is_file(path: &Path) -> bool {
    let probe_start = std::time::Instant::now();
    let is_file = crate::host_fs::is_file(path);
    crate::io_stats::record_existence_probe(probe_start.elapsed());
    crate::io_stats::record_probe_parent(path);
    is_file
}

fn build_dir_listing(dir: &Path) -> DirListing {
    let read_start = std::time::Instant::now();
    let mut listing = DirListing::default();
    // A host's listing cannot tell a missing file from one it does not list,
    // so every probe under a host asks it directly.
    if crate::host_fs::is_custom() {
        listing.unlisted = true;
        return listing;
    }
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let bytes = name.as_encoded_bytes();
                let lower: Box<[u8]> = bytes.to_ascii_lowercase().into();
                if matches!(entry.file_type(), Ok(file_type) if file_type.is_dir()) {
                    continue;
                }
                if matches!(entry.file_type(), Ok(file_type) if file_type.is_file()) {
                    listing.files.insert(bytes.into());
                }
                listing.maybe_lower.insert(lower);
            }
        }
        // A missing directory rules every child out, so only a directory that
        // exists but cannot be listed is inconclusive.
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            listing.unlisted = true;
        }
        Err(_) => {}
    }
    crate::io_stats::record_read_dir(read_start.elapsed());
    listing
}

impl DirListing {
    fn answer(&self, path: &Path, name: &[u8]) -> bool {
        if self.unlisted {
            return stat_is_file(path);
        }
        if self.files.contains(name) {
            return true;
        }
        if self.maybe_lower.contains(name.to_ascii_lowercase().as_slice()) {
            return stat_is_file(path);
        }
        false
    }
}

/// Whether `path` names an existing directory, memoized per thread with its
/// ancestors.
pub(crate) fn is_existing_dir(path: &Path) -> bool {
    PROBE_CACHE.with(|cache| dir_exists(&mut cache.borrow_mut(), path))
}

/// Whether `path` names an existing regular file, memoized per thread.
pub(crate) fn is_existing_file(path: &Path) -> bool {
    PROBE_CACHE.with(|cache| {
        let key = path.as_os_str().as_encoded_bytes();
        if let Some(&hit) = cache.borrow().paths.get(key) {
            return hit;
        }

        let mut cache = cache.borrow_mut();
        let cache = &mut *cache;
        let is_file = match path.parent().zip(path.file_name()) {
            // A host answers for the files it has, not for every directory
            // above them: tsc's resolver asks a host `fileExists` alone.
            _ if crate::host_fs::is_custom() => stat_is_file(path),
            Some((parent, name)) if name.as_encoded_bytes().is_ascii() => {
                if !dir_exists(cache, parent) {
                    false
                } else {
                    let parent_key = parent.as_os_str().as_encoded_bytes();
                    let name = name.as_encoded_bytes();
                    if let Some(listing) = cache.dir_listings.get(parent_key) {
                        listing.answer(path, name)
                    } else {
                        let count = cache
                            .dir_probe_counts
                            .entry(parent_key.into())
                            .and_modify(|count| *count += 1)
                            .or_insert(1);
                        if *count >= DIR_LISTING_THRESHOLD {
                            let listing = build_dir_listing(parent);
                            let answer = listing.answer(path, name);
                            cache.dir_listings.insert(parent_key.into(), listing);
                            answer
                        } else {
                            stat_is_file(path)
                        }
                    }
                }
            }
            _ => stat_is_file(path),
        };
        cache.paths.insert(key.into(), is_file);
        is_file
    })
}
