//! tsc's resolved project references (`projectReferenceFileMapper`): the
//! projects a config references, those they reference in turn, and the
//! declaration file each of their sources is emitted as, which a program
//! reads in the source's place.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::{LoadedTsConfig, TsConfigLoadOptions, canonicalize_if_exists, load_tsconfig};

#[derive(Debug, Default)]
pub struct ProjectReferenceOutputs {
    /// Each referenced project's config, a parent before the projects it
    /// references.
    pub projects: Vec<LoadedTsConfig>,
    by_source: HashMap<PathBuf, Option<PathBuf>>,
}

impl ProjectReferenceOutputs {
    /// The projects `config_files` name, read in order. A file two projects
    /// include belongs to the later, as tsgo's `initMapperWorker` copies a
    /// referenced project's map over its parent's.
    pub fn load(config_files: &[PathBuf]) -> Self {
        let mut outputs = Self::default();
        for config_file in config_files {
            let config = load_tsconfig(TsConfigLoadOptions { project: canonicalize_if_exists(config_file) });
            for file in &config.files {
                outputs.by_source.insert(canonicalize_if_exists(file), output_declaration(file, &config));
            }
            outputs.projects.push(config);
        }
        outputs
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// What a program reads for `file`: `None` when no referenced project
    /// includes it; else the output declaration it is emitted as, or `None`
    /// inside for a declaration or JSON file, which is read as itself.
    pub fn redirect(&self, file: &Path) -> Option<Option<&Path>> {
        self.by_source
            .get(&canonicalize_if_exists(file))
            .map(|output| output.as_deref())
    }

    /// The referenced project that includes `file`.
    pub fn project_of(&self, file: &Path) -> Option<&LoadedTsConfig> {
        let file = canonicalize_if_exists(file);
        self.projects
            .iter()
            .rev()
            .find(|project| project.files.iter().any(|candidate| canonicalize_if_exists(candidate) == file))
    }
}

/// tsgo's `GetOutputDeclarationFileNameWorker`: the source's path under
/// `declarationDir` (else `outDir`) relative to the common source directory,
/// or beside the source when neither is set, with its declaration extension.
/// A declaration or JSON file emits none.
fn output_declaration(source: &Path, config: &LoadedTsConfig) -> Option<PathBuf> {
    let name = source.to_string_lossy();
    if is_declaration_file_name(&name) || name.ends_with(".json") {
        return None;
    }
    let options = &config.compiler_options;
    let located = match options.declaration_dir.as_ref().or(options.out_dir.as_ref()) {
        Some(directory) => normalize(&directory.join(relative_to(&common_source_directory(config), source))),
        None => source.to_path_buf(),
    };
    Some(with_declaration_extension(&located))
}

/// tsgo's `GetCommonSourceDirectory` for a project with a config file:
/// `rootDir`, else the config file's directory.
fn common_source_directory(config: &LoadedTsConfig) -> PathBuf {
    let directory = config
        .compiler_options
        .root_dir
        .clone()
        .unwrap_or_else(|| config.config_path.parent().map(Path::to_path_buf).unwrap_or_default());
    canonicalize_if_exists(&directory)
}

/// tsgo's `GetDeclarationEmitExtensionForPath` over the source extensions a
/// project emits.
fn with_declaration_extension(path: &Path) -> PathBuf {
    let name = path.to_string_lossy();
    for (source, declaration) in [
        (".mts", ".d.mts"),
        (".mjs", ".d.mts"),
        (".cts", ".d.cts"),
        (".cjs", ".d.cts"),
        (".tsx", ".d.ts"),
        (".jsx", ".d.ts"),
        (".ts", ".d.ts"),
        (".js", ".d.ts"),
    ] {
        if let Some(stem) = name.strip_suffix(source) {
            return PathBuf::from(format!("{stem}{declaration}"));
        }
    }
    PathBuf::from(format!("{name}.d.ts"))
}

fn is_declaration_file_name(name: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"].iter().any(|extension| name.ends_with(extension))
        || name.rsplit_once(".d.").is_some_and(|(_, rest)| rest.ends_with(".ts"))
}

/// `from` to `to` as a relative path, climbing with `..` where `to` is
/// outside `from` (tsgo's `GetRelativePathFromDirectory`).
fn relative_to(from: &Path, to: &Path) -> PathBuf {
    let to = canonicalize_if_exists(to);
    let from: Vec<Component> = from.components().collect();
    let to_components: Vec<Component> = to.components().collect();
    let shared = from.iter().zip(&to_components).take_while(|(left, right)| left == right).count();
    let mut relative = PathBuf::new();
    for _ in shared..from.len() {
        relative.push("..");
    }
    for component in &to_components[shared..] {
        relative.push(component.as_os_str());
    }
    relative
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
