use std::path::{Path, PathBuf};

use surge_ts_checker::SourceFileInput;
use surge_ts_checker::lowlevel::resolution_candidates::{
    directory_index_candidates, mapped_target_candidates, relative_import_candidates,
    relative_specifier_names_directory,
};
use surge_ts_config::PathMapping;
use surge_ts_config::{
    canonicalize_if_exists, canonicalize_if_exists_string, normalize_path_string,
    select_path_mapping_targets,
};

use crate::specifier::is_relative_specifier;
use crate::specifier_scan::ModuleSpecifierScanner;

/// Import-graph BFS state that survives across loader fixpoint iterations, so
/// each source is scanned exactly once no matter how many times the loader
/// loop re-enters the expansion.
#[derive(Default)]
pub struct ImportGraphState {
    known_files: surge_ts_types::fx::FxHashSet<String>,
    probe_cache: surge_ts_types::fx::FxHashMap<String, bool>,
    next_source_index: usize,
    synced_inputs: usize,
}

pub fn expand_project_inputs(
    state: &mut ImportGraphState,
    scanner: &mut ModuleSpecifierScanner,
    inputs: &mut Vec<SourceFileInput>,
    sources: &mut Vec<(PathBuf, String, String)>,
    root_dir: &Path,
    base_url: Option<&Path>,
    paths: &[PathMapping],
    resolve_json_module: bool,
    javascript_modules: &mut Vec<(String, String, String)>,
) -> usize {
    // Other scanners (package declarations, reference directives) may have
    // appended inputs since the previous call; fold them into the known set so
    // their files are not re-added under a second path spelling.
    for input in inputs[state.synced_inputs..].iter() {
        state
            .known_files
            .insert(canonicalize_if_exists_string(Path::new(&input.file_name)));
    }

    let mut added = 0usize;

    // Frontier-at-a-time BFS. Resolution stays serial (it mutates the probe
    // cache and the known-file set), but a wave's file reads are independent,
    // so they run on a pool and the discovered sources are appended in the
    // exact order the file-at-a-time loop would have appended them.
    while state.next_source_index < sources.len() {
        let wave_end = sources.len();
        scanner.prefetch(sources, state.next_source_index);

        let mut discovered: Vec<PathBuf> = Vec::new();
        while state.next_source_index < wave_end {
            let index = state.next_source_index;
            state.next_source_index += 1;

            let (file_path, file_name, source_text) = {
                let (file_path, file_name, source_text) = &sources[index];
                (file_path.clone(), file_name.clone(), source_text.clone())
            };

            for module_specifier in scanner.specifiers(index, &file_name, &source_text).iter() {
                let candidate = if is_relative_specifier(module_specifier) {
                    let candidate = resolve_relative_candidate(
                        &file_path,
                        module_specifier,
                        &mut state.probe_cache,
                        resolve_json_module,
                    );
                    if candidate.is_none()
                        && let Some(javascript) = resolve_relative_javascript(
                            &file_path,
                            module_specifier,
                            &mut state.probe_cache,
                            resolve_json_module,
                        )
                    {
                        javascript_modules.push((
                            file_name.clone(),
                            module_specifier.to_string(),
                            canonicalize_if_exists_string(&javascript),
                        ));
                    }
                    candidate
                } else {
                    resolve_paths_alias_candidate(
                        module_specifier,
                        resolve_json_module,
                        paths,
                        base_url,
                        root_dir,
                        &mut state.probe_cache,
                    )
                };

                let Some(candidate) = candidate else {
                    continue;
                };

                if is_dependency_javascript_source_file(&candidate)
                    || !is_loadable_graph_file(&candidate, resolve_json_module)
                {
                    continue;
                }

                let canonical = canonicalize_if_exists(&candidate);
                let normalized = canonicalize_if_exists_string(&canonical);
                record_graph_edge(&file_name, &normalized);
                if !state.known_files.insert(normalized) {
                    continue;
                }

                discovered.push(canonical);
            }
        }

        for (canonical, source_text) in read_discovered_sources(discovered) {
            let Some(source_text) = source_text else {
                continue;
            };
            let file_name = canonical.to_string_lossy().into_owned();
            inputs.push(SourceFileInput {
                file_name: file_name.clone(),
                source_text: source_text.clone(),
            });
            sources.push((canonical, file_name, source_text));
            added += 1;
        }
    }

    state.synced_inputs = inputs.len();
    added
}

/// Read a discovery wave's files, preserving input order. Unreadable files come
/// back as `None` so the caller skips them exactly as the serial read did.
fn read_discovered_sources(paths: Vec<PathBuf>) -> Vec<(PathBuf, Option<String>)> {
    let read_one = |path: &Path| {
        let read_start = std::time::Instant::now();
        let text = crate::host_fs::read_to_string(path).ok();
        if let Some(text) = &text {
            crate::io_stats::record_expansion_read(text.len(), read_start.elapsed());
        }
        text
    };

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(paths.len());
    if paths.len() < 8 || workers <= 1 || crate::host_fs::is_custom() {
        return paths
            .into_iter()
            .map(|path| {
                let text = read_one(&path);
                (path, text)
            })
            .collect();
    }

    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<(usize, Option<String>)> = std::thread::scope(|scope| {
        let paths = &paths;
        let next = &next;
        let read_one = &read_one;
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            handles.push(scope.spawn(move || {
                let mut out = Vec::new();
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= paths.len() {
                        break;
                    }
                    out.push((index, read_one(&paths[index])));
                }
                out
            }));
        }
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("expansion read worker panicked"))
            .collect()
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    paths
        .into_iter()
        .zip(results)
        .map(|(path, (_, text))| (path, text))
        .collect()
}

/// Opt-in module import-edge dump (`SURGE_MODULE_GRAPH_DUMP=<path>`): appends one
/// tab-separated `importer\timportee` line per resolved relative / `paths` edge,
/// including back-edges to already-discovered files, so the full cyclic import
/// graph (not just the discovery tree) can be reconstructed offline for SCC /
/// critical-path analysis. Off by default and zero-cost when unset.
fn record_graph_edge(importer: &str, importee: &str) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let sink = SINK.get_or_init(|| {
        let path = std::env::var_os("SURGE_MODULE_GRAPH_DUMP")?;
        std::fs::File::create(path).ok().map(Mutex::new)
    });
    if let Some(sink) = sink
        && let Ok(mut file) = sink.lock()
    {
        let _ = writeln!(file, "{importer}\t{importee}");
    }
}

fn resolve_relative_candidate(
    importer_file: &Path,
    specifier: &str,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
    resolve_json_module: bool,
) -> Option<PathBuf> {
    let importer_dir = importer_file.parent().unwrap_or_else(|| Path::new(""));
    let normalized_specifier = normalize_path_string(specifier);
    let joined = normalize_path_string(&importer_dir.join(&normalized_specifier).to_string_lossy());

    let candidate_paths = if relative_specifier_names_directory(specifier) {
        directory_index_candidates(&joined)
    } else {
        relative_import_candidates(&joined, &normalized_specifier)?
    };

    for candidate in candidate_paths {
        let candidate = PathBuf::from(candidate);
        if !candidate_is_existing_file(&candidate, probe_cache) {
            continue;
        }

        if is_dependency_javascript_source_file(&candidate)
            || !is_loadable_graph_file(&candidate, resolve_json_module)
        {
            continue;
        }

        return Some(candidate);
    }

    None
}

/// How the loader resolves one specifier from one importer, outside a
/// program build: a relative path among the files on disk, else through
/// `paths`. Bare package specifiers are the package resolver's.
pub(crate) fn resolve_specifier_on_disk(
    importer_file: &Path,
    specifier: &str,
    resolve_json_module: bool,
    paths: &[surge_ts_config::PathMapping],
    base_url: Option<&Path>,
    root_dir: &Path,
) -> Option<PathBuf> {
    let mut probe_cache = surge_ts_types::fx::FxHashMap::default();
    if is_relative_specifier(specifier) {
        return resolve_relative_candidate(importer_file, specifier, &mut probe_cache, resolve_json_module)
            .or_else(|| resolve_relative_javascript(importer_file, specifier, &mut probe_cache, resolve_json_module));
    }
    resolve_paths_alias_candidate(specifier, resolve_json_module, paths, base_url, root_dir, &mut probe_cache)
}

/// Where tsc's resolver lands a relative specifier no TypeScript or
/// declaration file answers, when that is a JavaScript file. The graph never
/// loads it, but the checker needs it to tell an untyped module (TS7016, or
/// nothing) from a missing one (TS2307).
fn resolve_relative_javascript(
    importer_file: &Path,
    specifier: &str,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
    resolve_json_module: bool,
) -> Option<PathBuf> {
    let importer_dir = importer_file.parent().unwrap_or_else(|| Path::new(""));
    let candidate = PathBuf::from(normalize_path_string(
        &importer_dir.join(normalize_path_string(specifier)).to_string_lossy(),
    ));
    let resolved = module_on_disk(&candidate, true, probe_cache, resolve_json_module)?;
    is_javascript_file(&resolved).then_some(resolved)
}

/// tsc's `nodeLoadModuleByRelativeName` over the disk, every extension family
/// in one pass: the file the path names, then the directory's package.json
/// entry, then its index.
fn module_on_disk(
    candidate: &Path,
    consider_package_json: bool,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
    resolve_json_module: bool,
) -> Option<PathBuf> {
    if let Some(file) = module_file_on_disk(candidate, probe_cache, resolve_json_module) {
        return Some(file);
    }
    if !crate::host_fs::is_dir(candidate) {
        return None;
    }
    if consider_package_json
        && let Some(entry) = package_json_entry(candidate)
        && let Some(file) = module_on_disk(&entry, false, probe_cache, resolve_json_module)
    {
        return Some(file);
    }
    module_file_on_disk(&candidate.join("index"), probe_cache, resolve_json_module)
}

/// tsc's `loadModuleFromFile`: a known extension is replaced first (`a.js`
/// tries `a.ts`, `a.tsx`, `a.d.ts`, `a.js`, `a.jsx`), then each extension is
/// appended to the whole name.
fn module_file_on_disk(
    candidate: &Path,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
    resolve_json_module: bool,
) -> Option<PathBuf> {
    let name = candidate.file_name()?.to_str()?;
    let mut candidates = Vec::new();
    if let Some(dot) = name.rfind('.') {
        let removable = [
            ".d.ts", ".d.mts", ".d.cts", ".mjs", ".mts", ".cjs", ".cts", ".ts", ".js", ".tsx",
            ".jsx", ".json",
        ];
        let extension = removable
            .into_iter()
            .find(|extension| name.ends_with(extension))
            .unwrap_or(&name[dot..]);
        let stem = &name[..name.len() - extension.len()];
        let replacements: Vec<String> = match extension {
            ".mjs" | ".mts" | ".d.mts" => vec![".mts".into(), ".d.mts".into(), ".mjs".into()],
            ".cjs" | ".cts" | ".d.cts" => vec![".cts".into(), ".d.cts".into(), ".cjs".into()],
            ".json" if resolve_json_module => vec![".d.json.ts".into(), ".json".into()],
            ".json" => vec![".d.json.ts".into()],
            ".tsx" | ".jsx" => [".tsx", ".ts", ".d.ts", ".jsx", ".js"].map(String::from).to_vec(),
            ".ts" | ".d.ts" | ".js" => {
                [".ts", ".tsx", ".d.ts", ".js", ".jsx"].map(String::from).to_vec()
            }
            other => vec![format!(".d{other}.ts")],
        };
        candidates.extend(
            replacements
                .into_iter()
                .map(|replacement| candidate.with_file_name(format!("{stem}{replacement}"))),
        );
    }
    candidates.extend(
        [".ts", ".tsx", ".d.ts", ".js", ".jsx"]
            .into_iter()
            .map(|extension| candidate.with_file_name(format!("{name}{extension}"))),
    );
    candidates
        .into_iter()
        .find(|candidate| candidate_is_existing_file(candidate, probe_cache))
}

/// `mapped_target_candidates` for a `paths` or `baseUrl` target, with a
/// directory's package.json entry tried before its index, as tsc's
/// `loadNodeModuleFromDirectory` does.
pub(crate) fn path_target_candidates(target: &str) -> Vec<String> {
    let mut candidates = mapped_target_candidates(target);
    let directory = Path::new(target);
    if !crate::host_fs::is_dir(directory) {
        return candidates;
    }
    let Some(entry) = package_json_entry(directory) else {
        return candidates;
    };
    let index_prefix = format!("{target}/index.");
    let index_at = candidates
        .iter()
        .position(|candidate| candidate.starts_with(&index_prefix))
        .unwrap_or(candidates.len());
    candidates.splice(index_at..index_at, mapped_target_candidates(&entry.to_string_lossy()));
    candidates
}

/// The file a directory's package.json points at, read the way tsc's
/// `getPackageFile` reads it: `typings`, then `types`, then `main`.
fn package_json_entry(directory: &Path) -> Option<PathBuf> {
    let text = crate::host_fs::read_to_string(&directory.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    ["typings", "types", "main"]
        .into_iter()
        .find_map(|field| json.get(field)?.as_str())
        .map(|entry| PathBuf::from(normalize_path_string(&directory.join(entry).to_string_lossy())))
}

fn is_javascript_file(path: &Path) -> bool {
    [".js", ".jsx", ".mjs", ".cjs"]
        .into_iter()
        .any(|extension| path_ends_with_ignore_ascii_case(path, extension))
}

fn resolve_paths_alias_candidate(
    specifier: &str,
    resolve_json_module: bool,
    paths: &[PathMapping],
    base_url: Option<&Path>,
    root_dir: &Path,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
) -> Option<PathBuf> {
    // `paths` substitutions and the bare-import fallback resolve against
    // `baseUrl` when set, else the config directory (tsc ≥4.4 allows `paths`
    // without `baseUrl`).
    let mapping_base = base_url.unwrap_or(root_dir);

    if let Some(targets) = select_path_mapping_targets(specifier, paths) {
        for target in targets {
            let joined = normalize_path_string(&mapping_base.join(&target).to_string_lossy());
            if let Some(candidate) = probe_loadable_candidates(&joined, probe_cache, resolve_json_module) {
                return Some(candidate);
            }
        }
        return None;
    }

    // No pattern matched: tsc falls back to resolving the bare specifier
    // directly against `baseUrl`.
    if let Some(base_url) = base_url {
        let joined = normalize_path_string(&base_url.join(specifier).to_string_lossy());
        return probe_loadable_candidates(&joined, probe_cache, resolve_json_module);
    }

    None
}

fn probe_loadable_candidates(
    target: &str,
    probe_cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
    resolve_json_module: bool,
) -> Option<PathBuf> {
    for candidate in path_target_candidates(target) {
        let candidate = PathBuf::from(candidate);
        if !candidate_is_existing_file(&candidate, probe_cache) {
            continue;
        }

        if is_dependency_javascript_source_file(&candidate)
            || !is_loadable_graph_file(&candidate, resolve_json_module)
        {
            continue;
        }

        return Some(candidate);
    }

    None
}

// Most extensionless specifiers fan out to ~15 candidate paths and modules are
// imported from many files, so hits are memoized by path; misses go through
// the shared probe layer, which upgrades hot directories to a one-shot
// `read_dir` listing.
fn candidate_is_existing_file(
    candidate: &Path,
    cache: &mut surge_ts_types::fx::FxHashMap<String, bool>,
) -> bool {
    let key = candidate.to_string_lossy();
    if let Some(&hit) = cache.get(key.as_ref()) {
        return hit;
    }
    let is_file = crate::probe::is_existing_file(candidate);
    cache.insert(key.into_owned(), is_file);
    is_file
}

// Byte-level ASCII-case-insensitive matching: these checks run per resolved
// candidate, and the previous `to_string_lossy().to_ascii_lowercase()` paid
// two allocations each. Lossy conversion only replaces invalid UTF-8 with
// U+FFFD (non-ASCII), so ASCII needles match the lossy string iff they match
// the raw bytes.
fn path_ends_with_ignore_ascii_case(path: &Path, suffix: &str) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    bytes.len() >= suffix.len()
        && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn path_contains_ignore_ascii_case(path: &Path, needle: &str) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn is_loadable_graph_file(path: &Path, resolve_json_module: bool) -> bool {
    // A `.json` file is not a TypeScript source; it joins the graph only so its
    // value type can back a `resolveJsonModule` import, and never otherwise.
    if path_ends_with_ignore_ascii_case(path, ".json") {
        return resolve_json_module;
    }

    // TypeScript sources are loadable even under `node_modules`: tsc applies the
    // same extension priority (`.ts` before `.d.ts`) to relative imports inside
    // dependencies, so a package that ships sources (or is resolved through a
    // source `exports` condition) gets its source graph checked, not its
    // declarations.
    path_ends_with_ignore_ascii_case(path, ".ts")
        || path_ends_with_ignore_ascii_case(path, ".tsx")
        || path_ends_with_ignore_ascii_case(path, ".mts")
        || path_ends_with_ignore_ascii_case(path, ".cts")
}

fn is_dependency_javascript_source_file(path: &Path) -> bool {
    let is_node_modules = path_contains_ignore_ascii_case(path, "/node_modules/")
        || path_contains_ignore_ascii_case(path, "\\node_modules\\");
    let is_javascript_source = path_ends_with_ignore_ascii_case(path, ".js")
        || path_ends_with_ignore_ascii_case(path, ".jsx")
        || path_ends_with_ignore_ascii_case(path, ".mjs")
        || path_ends_with_ignore_ascii_case(path, ".cjs");

    is_node_modules && is_javascript_source
}
