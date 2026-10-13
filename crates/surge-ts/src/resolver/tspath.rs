//! tsgo's `tspath`: slash-normalized path strings, as the module resolver
//! reads and builds them.

pub(crate) const EXTENSION_TS: &str = ".ts";
pub(crate) const EXTENSION_TSX: &str = ".tsx";
pub(crate) const EXTENSION_DTS: &str = ".d.ts";
pub(crate) const EXTENSION_JS: &str = ".js";
pub(crate) const EXTENSION_JSX: &str = ".jsx";
pub(crate) const EXTENSION_JSON: &str = ".json";
pub(crate) const EXTENSION_MJS: &str = ".mjs";
pub(crate) const EXTENSION_MTS: &str = ".mts";
pub(crate) const EXTENSION_DMTS: &str = ".d.mts";
pub(crate) const EXTENSION_CJS: &str = ".cjs";
pub(crate) const EXTENSION_CTS: &str = ".cts";
pub(crate) const EXTENSION_DCTS: &str = ".d.cts";

const SUPPORTED_DECLARATION_EXTENSIONS: [&str; 3] = [EXTENSION_DTS, EXTENSION_DCTS, EXTENSION_DMTS];
const SUPPORTED_TS_IMPLEMENTATION_EXTENSIONS: [&str; 4] = [EXTENSION_TS, EXTENSION_TSX, EXTENSION_MTS, EXTENSION_CTS];
const TS_EXTENSIONS_FOR_EXTRACT: [&str; 7] =
    [EXTENSION_DTS, EXTENSION_DCTS, EXTENSION_DMTS, EXTENSION_TS, EXTENSION_TSX, EXTENSION_MTS, EXTENSION_CTS];
pub(crate) const SUPPORTED_TS_EXTENSIONS_WITH_JSON_FLAT: [&str; 8] = [
    EXTENSION_TS,
    EXTENSION_TSX,
    EXTENSION_DTS,
    EXTENSION_CTS,
    EXTENSION_DCTS,
    EXTENSION_MTS,
    EXTENSION_DMTS,
    EXTENSION_JSON,
];
const EXTENSIONS_TO_REMOVE: [&str; 12] = [
    EXTENSION_DTS,
    EXTENSION_DMTS,
    EXTENSION_DCTS,
    EXTENSION_MJS,
    EXTENSION_MTS,
    EXTENSION_CJS,
    EXTENSION_CTS,
    EXTENSION_TS,
    EXTENSION_JS,
    EXTENSION_TSX,
    EXTENSION_JSX,
    EXTENSION_JSON,
];

/// tsgo asks the host whether file names are case-sensitive; macOS and Windows
/// file systems are not by default.
pub(crate) fn use_case_sensitive_file_names() -> bool {
    !(cfg!(target_os = "macos") || cfg!(windows))
}

pub(crate) fn normalize_slashes(path: &str) -> String {
    if path.contains('\\') { path.replace('\\', "/") } else { path.to_string() }
}

fn is_volume_character(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

/// `GetRootLength` for the roots a resolver meets: `/`, `//server/`, `c:/`.
pub(crate) fn get_root_length(path: &str) -> usize {
    let bytes = path.as_bytes();
    match bytes {
        [] => 0,
        [b'/', b'/', ..] => match path[2..].find('/') {
            Some(index) => index + 3,
            None => path.len(),
        },
        [b'/', ..] => 1,
        [volume, b':', rest @ ..] if is_volume_character(*volume) => match rest.first() {
            Some(b'/') => 3,
            None => 2,
            _ => 0,
        },
        _ => 0,
    }
}

pub(crate) fn is_rooted_disk_path(path: &str) -> bool {
    get_root_length(path) > 0
}

pub(crate) fn has_trailing_directory_separator(path: &str) -> bool {
    path.ends_with('/') || path.ends_with('\\')
}

pub(crate) fn remove_trailing_directory_separator(path: &str) -> &str {
    if path.len() > 1 && has_trailing_directory_separator(path) { &path[..path.len() - 1] } else { path }
}

pub(crate) fn ensure_trailing_directory_separator(path: &str) -> String {
    if has_trailing_directory_separator(path) { path.to_string() } else { format!("{path}/") }
}

/// `CombinePaths`: a rooted segment replaces what came before; nothing is
/// simplified.
pub(crate) fn combine_paths(first: &str, rest: &[&str]) -> String {
    let mut result = normalize_slashes(first);
    for segment in rest {
        if segment.is_empty() {
            continue;
        }
        let segment = normalize_slashes(segment);
        if result.is_empty() || get_root_length(&segment) != 0 {
            result = segment;
        } else {
            if !has_trailing_directory_separator(&result) {
                result.push('/');
            }
            result.push_str(&segment);
        }
    }
    result
}

pub(crate) fn combine(first: &str, second: &str) -> String {
    combine_paths(first, &[second])
}

/// `GetPathComponents`: the root, then each non-empty-trailing segment.
pub(crate) fn get_path_components(path: &str) -> Vec<String> {
    let root_length = get_root_length(path);
    let mut components = vec![path[..root_length].to_string()];
    let mut rest: Vec<&str> = path[root_length..].split('/').collect();
    if rest.last().is_some_and(|last| last.is_empty()) {
        rest.pop();
    }
    components.extend(rest.into_iter().map(str::to_string));
    components
}

/// `reducePathComponents`: drop `.` and empty segments, fold `..`.
fn reduce_path_components(components: Vec<String>) -> Vec<String> {
    let mut reduced = vec![components[0].clone()];
    for component in components.into_iter().skip(1) {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            if reduced.len() > 1 {
                if reduced.last().is_some_and(|last| last != "..") {
                    reduced.pop();
                    continue;
                }
            } else if !reduced[0].is_empty() {
                continue;
            }
        }
        reduced.push(component);
    }
    reduced
}

fn get_path_from_path_components(components: &[String]) -> String {
    if components.is_empty() {
        return String::new();
    }
    let root = &components[0];
    let rest = components[1..].join("/");
    if root.is_empty() {
        rest
    } else if has_trailing_directory_separator(root) {
        format!("{root}{rest}")
    } else if rest.is_empty() {
        root.clone()
    } else {
        format!("{root}/{rest}")
    }
}

/// `NormalizePath`: `.`/`..` resolved, a trailing separator kept.
pub(crate) fn normalize_path(path: &str) -> String {
    let path = normalize_slashes(path);
    let reduced = reduce_path_components(get_path_components(&path));
    let mut normalized = get_path_from_path_components(&reduced);
    if !normalized.is_empty() && has_trailing_directory_separator(&path) {
        normalized = ensure_trailing_directory_separator(&normalized);
    }
    normalized
}

pub(crate) fn get_normalized_absolute_path(path: &str, current_directory: &str) -> String {
    normalize_path(&combine(current_directory, path))
}

pub(crate) fn get_directory_path(path: &str) -> String {
    let path = normalize_slashes(path);
    let root_length = get_root_length(&path);
    if root_length == path.len() {
        return path;
    }
    let path = remove_trailing_directory_separator(&path);
    let cut = path.rfind('/').map_or(0, |index| index).max(root_length);
    path[..cut].to_string()
}

pub(crate) fn get_base_file_name(path: &str) -> String {
    let path = normalize_slashes(path);
    let root_length = get_root_length(&path);
    if root_length == path.len() {
        return String::new();
    }
    let path = remove_trailing_directory_separator(&path);
    let start = path.rfind('/').map_or(0, |index| index + 1).max(get_root_length(path));
    path[start..].to_string()
}

pub(crate) fn path_is_relative(path: &str) -> bool {
    path == "."
        || path == ".."
        || path.starts_with("./")
        || path.starts_with(".\\")
        || path.starts_with("../")
        || path.starts_with("..\\")
}

pub(crate) fn is_external_module_name_relative(name: &str) -> bool {
    path_is_relative(name) || is_rooted_disk_path(name)
}

pub(crate) fn file_extension_is(path: &str, extension: &str) -> bool {
    path.len() > extension.len() && path.ends_with(extension)
}

pub(crate) fn file_extension_is_one_of(path: &str, extensions: &[&str]) -> bool {
    extensions.iter().any(|extension| file_extension_is(path, extension))
}

pub(crate) fn remove_file_extension(path: &str) -> &str {
    EXTENSIONS_TO_REMOVE
        .iter()
        .find(|extension| path.ends_with(*extension))
        .map_or(path, |extension| &path[..path.len() - extension.len()])
}

pub(crate) fn try_get_extension_from_path(path: &str) -> &'static str {
    EXTENSIONS_TO_REMOVE.iter().find(|extension| file_extension_is(path, extension)).copied().unwrap_or("")
}

pub(crate) fn try_extract_ts_extension(file_name: &str) -> &'static str {
    TS_EXTENSIONS_FOR_EXTRACT.iter().find(|extension| file_extension_is(file_name, extension)).copied().unwrap_or("")
}

pub(crate) fn get_declaration_file_extension(file_name: &str) -> String {
    let base = get_base_file_name(file_name);
    if let Some(extension) = SUPPORTED_DECLARATION_EXTENSIONS.iter().find(|extension| base.ends_with(*extension)) {
        return extension.to_string();
    }
    if base.ends_with(EXTENSION_TS)
        && let Some(index) = base.find(".d.")
    {
        return base[index..].to_string();
    }
    String::new()
}

pub(crate) fn is_declaration_file_name(file_name: &str) -> bool {
    !get_declaration_file_extension(file_name).is_empty()
}

pub(crate) fn has_implementation_ts_file_extension(path: &str) -> bool {
    file_extension_is_one_of(path, &SUPPORTED_TS_IMPLEMENTATION_EXTENSIONS) && !is_declaration_file_name(path)
}

pub(crate) fn extension_is_ts(extension: &str) -> bool {
    matches!(
        extension,
        EXTENSION_TS | EXTENSION_TSX | EXTENSION_DTS | EXTENSION_MTS | EXTENSION_DMTS | EXTENSION_CTS | EXTENSION_DCTS
    ) || (extension.len() >= 7 && extension.starts_with(".d.") && extension.ends_with(".ts"))
}

pub(crate) fn change_extension(path: &str, new_extension: &str) -> String {
    let base = remove_file_extension(path);
    if base.len() == path.len() {
        match path.rfind('.') {
            Some(dot) if dot > path.rfind('/').map_or(0, |slash| slash + 1) => {
                format!("{}{new_extension}", &path[..dot])
            }
            _ => format!("{path}{new_extension}"),
        }
    } else {
        format!("{base}{new_extension}")
    }
}

/// `GetPossibleOriginalInputExtensionForExtension`.
pub(crate) fn possible_original_input_extensions(path: &str) -> Vec<String> {
    if file_extension_is_one_of(path, &[EXTENSION_DMTS, EXTENSION_MJS, EXTENSION_MTS]) {
        return vec![EXTENSION_MTS.into(), EXTENSION_MJS.into()];
    }
    if file_extension_is_one_of(path, &[EXTENSION_DCTS, EXTENSION_CJS, EXTENSION_CTS]) {
        return vec![EXTENSION_CTS.into(), EXTENSION_CJS.into()];
    }
    let declaration = get_declaration_file_extension(path);
    if !declaration.is_empty() && declaration != EXTENSION_DTS {
        let inner = &declaration[".d.".len()..declaration.len() - ".ts".len()];
        return vec![format!(".{inner}")];
    }
    vec![EXTENSION_TSX.into(), EXTENSION_TS.into(), EXTENSION_JSX.into(), EXTENSION_JS.into()]
}

fn components_equal(a: &str, b: &str, case_sensitive: bool) -> bool {
    if case_sensitive { a == b } else { a.eq_ignore_ascii_case(b) }
}

/// `ComparePaths(...) == 0` after normalizing both.
pub(crate) fn paths_equal(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_path(a), normalize_path(b));
    components_equal(&a, &b, use_case_sensitive_file_names())
}

/// `ContainsPath`.
pub(crate) fn contains_path(parent: &str, child: &str, current_directory: &str) -> bool {
    let parent = combine(current_directory, parent);
    let child = combine(current_directory, child);
    if parent.is_empty() || child.is_empty() {
        return false;
    }
    if parent == child {
        return true;
    }
    let parent_components = reduce_path_components(get_path_components(&parent));
    let child_components = reduce_path_components(get_path_components(&child));
    if child_components.len() < parent_components.len() {
        return false;
    }
    let case_sensitive = use_case_sensitive_file_names();
    parent_components.iter().enumerate().all(|(index, component)| {
        components_equal(component, &child_components[index], index != 0 && case_sensitive)
    })
}

/// `GetRelativePathFromDirectory` for two absolute paths.
pub(crate) fn get_relative_path_from_directory(from_directory: &str, to: &str) -> String {
    let from = reduce_path_components(get_path_components(from_directory));
    let to = reduce_path_components(get_path_components(to));
    let case_sensitive = use_case_sensitive_file_names();
    let mut start = 0;
    while start < from.len().min(to.len()) {
        let equal = if start == 0 {
            from[0].eq_ignore_ascii_case(&to[0])
        } else {
            components_equal(&from[start], &to[start], case_sensitive)
        };
        if !equal {
            break;
        }
        start += 1;
    }
    if start == 0 {
        return get_path_from_path_components(&to);
    }
    let mut components = vec![String::new()];
    components.extend(std::iter::repeat_n("..".to_string(), from.len() - start));
    components.extend(to[start..].iter().cloned());
    get_path_from_path_components(&components)
}

/// `ForEachAncestorDirectory`: `directory`, then each parent up to the root.
pub(crate) fn ancestor_directories(directory: &str) -> Vec<String> {
    let mut directories = Vec::new();
    let mut current = normalize_slashes(directory);
    loop {
        directories.push(current.clone());
        let parent = get_directory_path(&current);
        if parent == current {
            break;
        }
        current = parent;
    }
    directories
}
