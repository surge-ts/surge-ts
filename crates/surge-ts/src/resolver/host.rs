//! The resolver's view of the disk (or of a compiler host), through the
//! loader's memoized probes.

use std::cell::RefCell;
use std::path::Path;

use surge_ts_types::fx::FxHashMap;

use super::ResolverHost;

#[derive(Default)]
pub(crate) struct DiskHost {
    realpaths: RefCell<FxHashMap<String, String>>,
}

impl ResolverHost for DiskHost {
    fn file_exists(&self, path: &str) -> bool {
        crate::probe::is_existing_file(Path::new(path))
    }

    fn directory_exists(&self, path: &str) -> bool {
        crate::probe::is_existing_dir(Path::new(path))
    }

    fn read_file(&self, path: &str) -> Option<String> {
        crate::host_fs::read_to_string(Path::new(path)).ok()
    }

    fn realpath(&self, path: &str) -> String {
        if let Some(real) = self.realpaths.borrow().get(path) {
            return real.clone();
        }
        let real = crate::host_fs::canonicalize(Path::new(path))
            .map(|real| real.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string());
        self.realpaths.borrow_mut().insert(path.to_string(), real.clone());
        real
    }

    fn directories(&self, path: &str) -> Vec<String> {
        crate::host_fs::read_dir(Path::new(path))
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.is_dir || (entry.indirect && Path::new(path).join(&entry.name).is_dir()))
                    .map(|entry| entry.name.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}
