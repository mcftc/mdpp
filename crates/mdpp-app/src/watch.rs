//! File-change notifications (PLAN §3.3): the folders of open documents and the config folder are
//! watched non-recursively; changed paths arrive on a channel the workspace drains on the UI thread.
//! Whether a change matters (our own save, an external edit, a deletion) is decided by comparing
//! file stamps there — this module only reports paths.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub struct DiskWatcher {
    watcher: RecommendedWatcher,
    watched: HashSet<PathBuf>,
}

impl DiskWatcher {
    /// Changed paths are sent to `changes` from the watcher's own thread.
    pub fn new(changes: smol::channel::Sender<PathBuf>) -> notify::Result<Self> {
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let Ok(event) = result else {
                return;
            };
            if matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
            ) {
                for path in event.paths {
                    // The receiver lives as long as the workspace; a send after that is moot.
                    let _ = changes.try_send(path);
                }
            }
        })?;
        Ok(Self {
            watcher,
            watched: HashSet::new(),
        })
    }

    /// Watch exactly `folders` (non-recursive), adding and removing watches as needed.
    pub fn watch_folders(&mut self, folders: HashSet<PathBuf>) {
        for gone in self
            .watched
            .difference(&folders)
            .cloned()
            .collect::<Vec<_>>()
        {
            let _ = self.watcher.unwatch(&gone);
            self.watched.remove(&gone);
        }
        for folder in folders {
            if !self.watched.contains(&folder)
                && self
                    .watcher
                    .watch(&folder, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                self.watched.insert(folder);
            }
        }
    }
}

/// The folder to watch for a file.
pub fn folder_of(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}
