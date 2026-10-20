//! The file system the loader reads through: the process's own, or a
//! compiler API host's while a program built from one loads.
//!
//! The host is installed for the loading thread and carried into the
//! loader's worker threads by [`current`] / [`with_current`]; with none
//! installed every function is the `std::fs` call it replaces.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::CompilerHost;

thread_local! {
    static ACTIVE_HOST: RefCell<Option<Arc<dyn CompilerHost>>> = const { RefCell::new(None) };
}

/// Runs `f` reading through `host`. The process's own file system is read
/// directly.
pub(crate) fn with_host<R>(host: Arc<dyn CompilerHost>, f: impl FnOnce() -> R) -> R {
    if host.is_system() {
        return f();
    }
    with_current(Some(host), f)
}

/// Whether a host other than the process's own file system is installed:
/// the loader then reads on the loading thread only, since a host need not
/// be callable from others.
pub(crate) fn is_custom() -> bool {
    ACTIVE_HOST.with(|active| active.borrow().is_some())
}

fn current() -> Option<Arc<dyn CompilerHost>> {
    ACTIVE_HOST.with(|active| active.borrow().clone())
}

/// Runs `f` with `host` (from [`current`]) installed on this thread.
pub(crate) fn with_current<R>(host: Option<Arc<dyn CompilerHost>>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<dyn CompilerHost>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            ACTIVE_HOST.with(|active| *active.borrow_mut() = previous);
        }
    }
    let previous = ACTIVE_HOST.with(|active| std::mem::replace(&mut *active.borrow_mut(), host));
    let _restore = Restore(previous);
    f()
}

fn host_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn read_to_string(path: &Path) -> io::Result<String> {
    match current() {
        Some(host) => host
            .read_file(&host_path(path))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} not found", path.display()))),
        None => std::fs::read(path).map(decode_source_bytes),
    }
}

/// tsc's `sys.readFile`: a UTF-16 byte order mark reads the rest as UTF-16,
/// big- or little-endian; a UTF-8 one is dropped, so no position counts it;
/// bytes that are no UTF-8 read as U+FFFD.
pub(crate) fn decode_source_bytes(mut bytes: Vec<u8>) -> String {
    let utf16 = |rest: &[u8], big_endian: bool| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| if big_endian { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_le_bytes([pair[0], pair[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes.as_slice() {
        [0xFE, 0xFF, rest @ ..] => return utf16(rest, true),
        [0xFF, 0xFE, rest @ ..] => return utf16(rest, false),
        [0xEF, 0xBB, 0xBF, ..] => {
            bytes.drain(..3);
        }
        _ => {}
    }
    String::from_utf8(bytes).unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

pub(crate) fn is_file(path: &Path) -> bool {
    match current() {
        Some(host) => host.file_exists(&host_path(path)),
        None => std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()),
    }
}

pub(crate) fn is_dir(path: &Path) -> bool {
    match current() {
        Some(host) => host.directory_exists(&host_path(path)),
        None => std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()),
    }
}

/// `std::fs::canonicalize`: an existing path with its links resolved.
pub(crate) fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    match current() {
        Some(host) => {
            let name = host_path(path);
            if host.file_exists(&name) || host.directory_exists(&name) {
                Ok(PathBuf::from(host.realpath(&name)))
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, format!("{} not found", path.display())))
            }
        }
        None => std::fs::canonicalize(path),
    }
}

/// A directory entry: its name, and whether it is a directory (following
/// links).
pub(crate) struct Entry {
    pub(crate) name: std::ffi::OsString,
    pub(crate) is_dir: bool,
    /// The entry is a link or of a type the listing could not tell.
    pub(crate) indirect: bool,
}

pub(crate) fn read_dir(path: &Path) -> io::Result<Vec<Entry>> {
    match current() {
        Some(host) => host
            .read_directory_entries(&host_path(path))
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| Entry {
                        name: entry.name.into(),
                        is_dir: entry.is_directory,
                        indirect: false,
                    })
                    .collect()
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} not found", path.display()))),
        None => Ok(std::fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|entry| {
                let file_type = entry.file_type().ok();
                Entry {
                    name: entry.file_name(),
                    is_dir: file_type.is_some_and(|file_type| file_type.is_dir()),
                    indirect: file_type.is_none_or(|file_type| file_type.is_symlink()),
                }
            })
            .collect()),
    }
}
