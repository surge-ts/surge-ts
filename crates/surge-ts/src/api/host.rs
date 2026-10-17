//! TypeScript's `CompilerHost`: where a program reads its files.

use std::path::Path;

/// The file system a program is read from, kept to the operations surge's
/// loader performs: reading files, probing files and directories while
/// resolving modules, and listing directories while discovering type
/// packages.
///
/// Paths are absolute and use `/` separators.
pub trait CompilerHost: Send + Sync {
    fn read_file(&self, path: &str) -> Option<String>;
    fn file_exists(&self, path: &str) -> bool;
    fn directory_exists(&self, path: &str) -> bool;
    /// The names of `path`'s entries, files and directories alike.
    fn read_directory_entries(&self, path: &str) -> Option<Vec<DirectoryEntry>>;
    fn current_directory(&self) -> String;
    /// Resolves symbolic links; a host without them returns `path`.
    fn realpath(&self, path: &str) -> String {
        path.to_string()
    }
    fn use_case_sensitive_file_names(&self) -> bool {
        true
    }
    fn new_line(&self) -> String {
        "\n".to_string()
    }
    /// Whether this host is the process's own file system, which the loader
    /// reads directly.
    fn is_system(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_file: bool,
    pub is_directory: bool,
}

/// The process's file system: TypeScript's `sys`.
#[derive(Clone, Debug)]
pub struct SystemHost {
    current_directory: String,
}

impl SystemHost {
    pub fn new() -> Self {
        let current_directory = std::env::current_dir()
            .map(|dir| dir.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| "/".to_string());
        SystemHost { current_directory }
    }

    pub fn with_current_directory(current_directory: impl Into<String>) -> Self {
        SystemHost { current_directory: current_directory.into() }
    }
}

impl Default for SystemHost {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilerHost for SystemHost {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn file_exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn directory_exists(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn read_directory_entries(&self, path: &str) -> Option<Vec<DirectoryEntry>> {
        let entries = std::fs::read_dir(path).ok()?;
        Some(
            entries
                .filter_map(Result::ok)
                .map(|entry| {
                    let file_type = std::fs::metadata(entry.path()).ok();
                    DirectoryEntry {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        is_file: file_type.as_ref().is_some_and(std::fs::Metadata::is_file),
                        is_directory: file_type.as_ref().is_some_and(std::fs::Metadata::is_dir),
                    }
                })
                .collect(),
        )
    }

    fn current_directory(&self) -> String {
        self.current_directory.clone()
    }

    fn realpath(&self, path: &str) -> String {
        std::fs::canonicalize(path)
            .map(|path| surge_ts_config::normalize_path_string(&path.to_string_lossy()))
            .unwrap_or_else(|_| path.to_string())
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        !cfg!(any(target_os = "macos", target_os = "windows"))
    }

    fn new_line(&self) -> String {
        if cfg!(windows) { "\r\n".to_string() } else { "\n".to_string() }
    }

    fn is_system(&self) -> bool {
        true
    }
}
