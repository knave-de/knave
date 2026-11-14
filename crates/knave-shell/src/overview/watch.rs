//! Bounded directory watches matching the catalog's scan depth and entry limit.
use knave_apps::Environment;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{collections::HashSet, path::PathBuf};

pub(super) struct CatalogWatch {
    watcher: RecommendedWatcher,
    paths: HashSet<PathBuf>,
}

impl CatalogWatch {
    pub fn new(
        env: &Environment,
        mut changed: impl FnMut() + Send + 'static,
    ) -> notify::Result<Self> {
        let roots: Vec<_> = env
            .data_dirs
            .iter()
            .map(|p| p.join("applications"))
            .collect();
        let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let relevant = match result {
                Ok(event) => {
                    !matches!(event.kind, EventKind::Access(_))
                        && (event.need_rescan()
                            || event.paths.iter().any(|path| {
                                roots
                                    .iter()
                                    .any(|root| path.starts_with(root) || root.starts_with(path))
                            }))
                }
                Err(error) => {
                    eprintln!("knave-shell: application watch failed: {error}");
                    true
                }
            };
            if relevant {
                changed();
            }
        })?;
        let mut this = Self {
            watcher,
            paths: HashSet::new(),
        };
        this.refresh(env);
        Ok(this)
    }

    pub fn refresh(&mut self, env: &Environment) {
        let mut paths = HashSet::new();
        let mut remaining = env.limits.entries;
        for dir in &env.data_dirs {
            let root = dir.join("applications");
            // Watching ancestors non-recursively catches missing roots and replacement
            // of a whole export directory without watching unrelated subtrees.
            for ancestor in root.ancestors().skip(1) {
                if ancestor.is_dir() {
                    paths.insert(ancestor.to_owned());
                }
            }
            directories(root, 0, &mut remaining, &mut paths);
        }
        for old in self.paths.difference(&paths) {
            let _ = self.watcher.unwatch(old);
        }
        // Re-register retained paths too: their inode may have been replaced.
        self.paths = paths
            .into_iter()
            .filter(
                |path| match self.watcher.watch(path, RecursiveMode::NonRecursive) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("knave-shell: cannot watch {}: {error}", path.display());
                        false
                    }
                },
            )
            .collect();
    }
}

fn directories(path: PathBuf, depth: usize, remaining: &mut usize, paths: &mut HashSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(&path) else {
        return;
    };
    if !paths.insert(path) {
        return;
    }
    for entry in entries {
        if *remaining == 0 {
            break;
        }
        *remaining -= 1;
        let Ok(entry) = entry else { continue };
        if depth < 3 && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            directories(entry.path(), depth + 1, remaining, paths);
        }
    }
}
