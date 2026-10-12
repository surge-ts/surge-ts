//! tsc's package deduplication (`packageIdToSourceFile`): a file in a
//! package whose package id (`package.json` name and version, the file's path
//! in the package, and the versions of its peer dependencies) an earlier file
//! already has is a redirect to that file. It is not parsed into the program,
//! and every resolution to it answers the earlier file, so both copies declare
//! one set of types.

use std::path::{Path, PathBuf};

use surge_ts_checker::SourceFileInput;
use surge_ts_types::fx::FxHashMap;

/// Redirected file name to the file that answers for it.
pub(crate) type PackageRedirects = FxHashMap<String, String>;

/// Every resolution that lands in a package gives the file its id, a relative
/// one inside the package as much as a package specifier
/// (`nodeLoadModuleByRelativeName`). tsc walks the program's files
/// depth-first; the loader's breadth-first `sources` order stands in for it,
/// which picks the same first copy unless one is reached deeper but sooner.
pub(crate) fn package_redirects(sources: &[(PathBuf, String, String)]) -> PackageRedirects {
    let mut redirects = PackageRedirects::default();
    let in_packages: Vec<&str> = sources
        .iter()
        .map(|(_, file_name, _)| file_name.as_str())
        .filter(|file_name| file_name.contains("/node_modules/"))
        .collect();
    if in_packages.len() < 2 {
        return redirects;
    }
    let mut manifests = FxHashMap::<PathBuf, Option<Manifest>>::default();
    let mut first_by_package_id = FxHashMap::<String, &str>::default();
    for file_name in in_packages {
        let Some(package_id) = package_id(file_name, &mut manifests) else {
            continue;
        };
        match first_by_package_id.get(&package_id) {
            Some(first) if *first != file_name => {
                redirects.insert(file_name.to_string(), (*first).to_string());
            }
            Some(_) => {}
            None => {
                first_by_package_id.insert(package_id, file_name);
            }
        }
    }
    redirects
}

/// Drops the redirected files from the program and points every resolution
/// to one at the file that answers for it.
pub(crate) fn apply_package_redirects(
    redirects: &PackageRedirects,
    inputs: &mut Vec<SourceFileInput>,
    sources: &mut Vec<(PathBuf, String, String)>,
    resolved_modules: &mut FxHashMap<String, String>,
    resolved_modules_by_importer: &mut FxHashMap<String, FxHashMap<String, String>>,
) {
    if redirects.is_empty() {
        return;
    }
    inputs.retain(|input| !redirects.contains_key(&input.file_name));
    sources.retain(|(_, file_name, _)| !redirects.contains_key(file_name));
    let redirect = |resolved: &mut String| {
        if let Some(target) = redirects.get(resolved.as_str()) {
            resolved.clone_from(target);
        }
    };
    resolved_modules.values_mut().for_each(redirect);
    for resolutions in resolved_modules_by_importer.values_mut() {
        resolutions.values_mut().for_each(redirect);
    }
}

struct Manifest {
    name: Option<String>,
    version: Option<String>,
    peer_dependencies: Vec<String>,
}

/// tsc's `getPackageId`: `name@version` with the file's path in its package,
/// and `+peer@version` for each peer dependency installed beside it.
fn package_id(file_name: &str, manifests: &mut FxHashMap<PathBuf, Option<Manifest>>) -> Option<String> {
    let (node_modules, package_directory, sub_module_name) = split_package_path(file_name)?;
    let (name, version, peers) = {
        let manifest = read_manifest(&package_directory, manifests)?;
        (manifest.name.clone()?, manifest.version.clone()?, manifest.peer_dependencies.clone())
    };
    let mut id = format!("{name}/{sub_module_name}@{version}");
    for peer in peers {
        if let Some(peer_manifest) = read_manifest(&Path::new(node_modules).join(&peer), manifests) {
            id.push_str(&format!("+{peer}@{}", peer_manifest.version.as_deref().unwrap_or_default()));
        }
    }
    Some(id)
}

/// The `node_modules` directory a file's package is installed in, the package
/// directory under it (a scoped package spans two segments), and the file's
/// path inside that.
fn split_package_path(file_name: &str) -> Option<(&str, PathBuf, &str)> {
    const NODE_MODULES: &str = "/node_modules/";
    let start = file_name.rfind(NODE_MODULES)? + NODE_MODULES.len();
    let rest = &file_name[start..];
    let mut segments = rest.splitn(3, '/');
    let first = segments.next()?;
    let name_length = if first.starts_with('@') {
        first.len() + 1 + segments.next()?.len()
    } else {
        first.len()
    };
    let sub_module_name = rest.get(name_length + 1..).filter(|sub| !sub.is_empty())?;
    Some((&file_name[..start], PathBuf::from(&file_name[..start + name_length]), sub_module_name))
}

fn read_manifest<'a>(
    package_directory: &Path,
    manifests: &'a mut FxHashMap<PathBuf, Option<Manifest>>,
) -> Option<&'a Manifest> {
    manifests
        .entry(package_directory.to_path_buf())
        .or_insert_with(|| {
            let text = crate::host_fs::read_to_string(&package_directory.join("package.json")).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            let field = |name: &str| json.get(name).and_then(serde_json::Value::as_str).map(str::to_string);
            let mut peer_dependencies: Vec<String> = json
                .get("peerDependencies")
                .and_then(serde_json::Value::as_object)
                .map(|peers| peers.keys().cloned().collect())
                .unwrap_or_default();
            peer_dependencies.sort();
            Some(Manifest {
                name: field("name"),
                version: field("version"),
                peer_dependencies,
            })
        })
        .as_ref()
}
