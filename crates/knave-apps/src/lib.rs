//! Installed-application catalog built from XDG `.desktop` files.
//!
//! The catalog is a value: loading reads files once, and searching never touches
//! the filesystem. Every scan is bounded so a hostile or broken data directory
//! cannot stall or exhaust the shell.
mod entry;

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_DEPTH: usize = 3;
const MAX_QUERY_TERMS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("no readable application directories under XDG_DATA_HOME or XDG_DATA_DIRS")]
    NoApplicationDirectories,
}

/// Upper bounds on one scan. Reaching either stops the scan and marks the catalog truncated.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Directory entries examined across all directories, whatever their type.
    pub entries: usize,
    pub apps: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 16_384,
            apps: 4096,
        }
    }
}

/// Environment inputs that decide which entries are visible. Injectable for tests.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    pub limits: Limits,
    pub data_dirs: Vec<PathBuf>,
    pub path: Vec<PathBuf>,
    pub locales: Vec<String>,
    pub desktops: Vec<String>,
}
impl Environment {
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from);
        let data_home = var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")));
        let system = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut data_dirs: Vec<PathBuf> = data_home.into_iter().collect();
        data_dirs.extend(
            system
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        );
        let locale = var("LC_ALL")
            .or_else(|| var("LC_MESSAGES"))
            .or_else(|| var("LANG"))
            .unwrap_or_default();
        Self {
            limits: Limits::default(),
            data_dirs,
            path: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
            locales: entry::locale_candidates(&locale),
            desktops: var("XDG_CURRENT_DESKTOP")
                .map(|d| d.split(':').map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct App {
    /// Desktop file ID, for example `org.example.App.desktop`.
    pub id: String,
    pub name: String,
    /// Generic name or comment, whichever describes the application.
    pub description: String,
    pub argv: Vec<String>,
    /// Icon theme name or absolute path from the `Icon` key.
    pub icon: Option<String>,
    fields: Fields,
}

/// Lowercased search text, prepared once at load time.
#[derive(Clone, Debug)]
struct Fields {
    name: String,
    extra: String,
    program: String,
    comment: String,
}

#[derive(Debug, Default)]
pub struct Catalog {
    apps: Vec<App>,
    truncated: bool,
}

/// Mutable state shared by one scan.
struct Scan<'a> {
    env: &'a Environment,
    seen: HashSet<String>,
    apps: Vec<App>,
    entries: usize,
    truncated: bool,
}

impl Catalog {
    pub fn load() -> Result<Self, CatalogError> {
        Self::load_with(&Environment::from_process())
    }

    pub fn load_with(env: &Environment) -> Result<Self, CatalogError> {
        let mut scan = Scan {
            env,
            seen: HashSet::new(),
            apps: Vec::new(),
            entries: 0,
            truncated: false,
        };
        let mut readable = false;
        let mut roots = HashSet::new();
        for dir in &env.data_dirs {
            let root = dir.join("applications");
            // Duplicate XDG_DATA_DIRS entries would otherwise be scanned repeatedly.
            if !roots.insert(root.clone()) {
                continue;
            }
            readable |= scan.dir(&root, "", 0);
        }
        if !readable {
            return Err(CatalogError::NoApplicationDirectories);
        }
        Ok(Self {
            apps: scan.apps,
            truncated: scan.truncated,
        })
    }

    /// True when a limit stopped the scan, so some installed applications are missing.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&App> {
        self.apps.get(index)
    }

    /// Indexes of matching applications, best first. Every query term must match.
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().take(MAX_QUERY_TERMS).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u32, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(index, app)| {
                terms
                    .iter()
                    .map(|term| app.fields.rank(term))
                    .sum::<Option<u32>>()
                    .map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|(a, i), (b, j)| {
            a.cmp(b)
                .then_with(|| self.apps[*i].fields.name.cmp(&self.apps[*j].fields.name))
                .then(i.cmp(j))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, index)| index)
            .collect()
    }
}

impl Fields {
    /// Lower is better; `None` means the term does not match this application.
    fn rank(&self, term: &str) -> Option<u32> {
        let name = &self.name;
        if name == term {
            Some(0)
        } else if name.starts_with(term) {
            Some(1)
        } else if name
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word.starts_with(term))
        {
            Some(2)
        } else if name.contains(term) {
            Some(3)
        } else if self.extra.contains(term) {
            Some(4)
        } else if self.program.contains(term) {
            Some(5)
        } else if self.comment.contains(term) {
            Some(6)
        } else {
            None
        }
    }
}

impl Scan<'_> {
    /// Returns whether `dir` could be read.
    fn dir(&mut self, dir: &Path, prefix: &str, depth: usize) -> bool {
        let Ok(read) = fs::read_dir(dir) else {
            return false;
        };
        // Count entries as they are read, so a huge directory is never fully collected.
        let mut entries = Vec::new();
        for item in read.filter_map(Result::ok) {
            if self.entries >= self.env.limits.entries {
                self.truncated = true;
                break;
            }
            self.entries += 1;
            entries.push(item);
        }
        // Stable order keeps duplicate resolution and ties deterministic.
        entries.sort_by_key(|e| e.file_name());
        for item in entries {
            if self.apps.len() >= self.env.limits.apps {
                self.truncated = true;
                break;
            }
            let file_name = item.file_name().to_string_lossy().into_owned();
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir() {
                // Real directories only; following symlinked directories could loop.
                if depth < MAX_DEPTH {
                    let nested = format!("{prefix}{file_name}-");
                    self.dir(&item.path(), &nested, depth + 1);
                }
                continue;
            }
            if !file_name.ends_with(".desktop") {
                continue;
            }
            let id = format!("{prefix}{file_name}");
            // Higher-priority directories win, including when they hide the entry.
            if !self.seen.insert(id.clone()) {
                continue;
            }
            let Some(text) = read_bounded(&item.path()) else {
                continue;
            };
            if let Some(app) = build(id, &text, self.env) {
                self.apps.push(app);
            }
        }
        true
    }
}

fn read_bounded(path: &Path) -> Option<String> {
    // Opening a FIFO or device would block the loader forever; follow symlinks, read files only.
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    // Symlinks (Flatpak exports) are followed for files; a longer file is malformed.
    fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_FILE_BYTES)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

fn build(id: String, text: &str, env: &Environment) -> Option<App> {
    let entry = entry::parse(text, &env.locales);
    let listed = |names: &[String]| names.iter().any(|n| env.desktops.contains(n));
    if !entry.is_application
        || entry.deleted
        || entry.no_display
        || entry.terminal
        || entry.name.is_empty()
        || (!entry.only_show_in.is_empty() && !listed(&entry.only_show_in))
        || listed(&entry.not_show_in)
        || entry
            .try_exec
            .as_deref()
            .is_some_and(|program| !executable_exists(program, &env.path))
    {
        return None;
    }
    let argv = entry::argv(&entry.exec, &entry.name)?;
    let program = Path::new(&argv[0])
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().to_lowercase());
    let extra = format!("{};{}", entry.generic_name, entry.keywords.join(";")).to_lowercase();
    Some(App {
        id,
        description: if entry.generic_name.is_empty() {
            entry.comment.clone()
        } else {
            entry.generic_name.clone()
        },
        fields: Fields {
            name: entry.name.to_lowercase(),
            extra,
            program,
            comment: entry.comment.to_lowercase(),
        },
        name: entry.name,
        icon: entry.icon,
        argv,
    })
}

fn executable_exists(program: &str, path: &[PathBuf]) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| {
        fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return is_exec(Path::new(program));
    }
    path.iter().any(|dir| is_exec(&dir.join(program)))
}

#[cfg(test)]
mod tests;
