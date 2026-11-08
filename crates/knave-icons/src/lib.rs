//! Freedesktop icon lookup and rasterization.
//!
//! Lookup follows the icon theme spec's size-distance rule. Knave has no
//! icon-theme setting and this crate reads no other desktop's settings, so it
//! searches `hicolor` first and then every other installed theme by name.
mod raster;
mod theme;

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use theme::{Dir, Kind};

const MAX_INDEX_BYTES: u64 = 256 * 1024;
const MAX_THEMES: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error("cannot decode icon {}: {reason}", path.display())]
    Decode { path: PathBuf, reason: String },
}

/// Straight-alpha RGBA8 pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Theme {
    dirs: Vec<Dir>,
    /// Every icon root that contains this theme; files may live in any of them.
    roots: Vec<PathBuf>,
}

pub struct IconLookup {
    roots: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
    themes: Option<Vec<Theme>>,
}

impl IconLookup {
    pub fn new(roots: Vec<PathBuf>, pixmaps: Vec<PathBuf>) -> Self {
        Self {
            roots,
            pixmaps,
            themes: None,
        }
    }

    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let mut roots = Vec::new();
        if let Some(home) = var("HOME") {
            roots.push(PathBuf::from(home).join(".icons"));
        }
        let data_home = var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        roots.extend(data_home.map(|d| d.join("icons")));
        let system = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        roots.extend(std::env::split_paths(&system).map(|d| d.join("icons")));
        let mut seen = BTreeSet::new();
        roots.retain(|root| seen.insert(root.clone()));
        Self::new(roots, vec![PathBuf::from("/usr/share/pixmaps")])
    }

    /// `Ok(None)` means no such icon exists; errors mean a file was found but unusable.
    pub fn load(&mut self, name: &str, size: u32) -> Result<Option<Icon>, IconError> {
        let size = size.clamp(8, 512);
        match self.find(name, size) {
            Some(path) => raster::decode(&path, size).map(Some),
            None => Ok(None),
        }
    }

    fn find(&mut self, name: &str, size: u32) -> Option<PathBuf> {
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        if name.starts_with('/') {
            let path = PathBuf::from(name);
            return matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("png" | "svg")
            )
            .then_some(path)
            .filter(|p| p.is_file());
        }
        // A name is one file stem, never a path.
        if name.contains('/') || name == ".." {
            return None;
        }
        for theme in self.themes() {
            let mut dirs: Vec<&Dir> = theme.dirs.iter().collect();
            // Closest size first; on ties scalable art beats bitmaps.
            dirs.sort_by_key(|d| (d.distance(size), d.kind != Kind::Scalable));
            for dir in dirs {
                for root in &theme.roots {
                    if let Some(found) = existing(&root.join(&dir.path), name) {
                        return Some(found);
                    }
                }
            }
        }
        self.pixmaps.iter().find_map(|dir| existing(dir, name))
    }

    fn themes(&mut self) -> &[Theme] {
        let roots = &self.roots;
        self.themes.get_or_insert_with(|| discover(roots))
    }
}

fn existing(dir: &Path, name: &str) -> Option<PathBuf> {
    ["svg", "png"]
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| p.is_file())
}

fn discover(roots: &[PathBuf]) -> Vec<Theme> {
    let mut names = BTreeSet::new();
    for root in roots {
        let Ok(read) = fs::read_dir(root) else {
            continue;
        };
        for entry in read.filter_map(Result::ok) {
            if entry.path().join("index.theme").is_file() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    // hicolor is the mandatory fallback theme, so it is always tried first.
    let mut ordered: Vec<String> = names.iter().filter(|n| *n == "hicolor").cloned().collect();
    ordered.extend(names.into_iter().filter(|n| n != "hicolor"));
    ordered.truncate(MAX_THEMES);
    ordered
        .into_iter()
        .filter_map(|name| {
            let holders: Vec<PathBuf> = roots
                .iter()
                .map(|r| r.join(&name))
                .filter(|p| p.is_dir())
                .collect();
            // The first index.theme found defines the theme's directories.
            let text = holders
                .iter()
                .find_map(|p| read_index(&p.join("index.theme")))?;
            let dirs = theme::parse_index(&text);
            (!dirs.is_empty()).then_some(Theme {
                dirs,
                roots: holders,
            })
        })
        .collect()
}

fn read_index(path: &Path) -> Option<String> {
    use std::io::Read;
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_INDEX_BYTES)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

#[cfg(test)]
mod tests;
