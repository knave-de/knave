//! `index.theme` parsing and the freedesktop directory-size distance rule.
use std::collections::HashMap;

const MAX_DIRS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone, Debug)]
pub(crate) struct Dir {
    pub path: String,
    size: u32,
    min: u32,
    max: u32,
    threshold: u32,
    scale: u32,
    pub kind: Kind,
}

impl Dir {
    /// Distance in pixels between this directory's supported sizes and `size`.
    pub fn distance(&self, size: u32) -> u32 {
        let scaled = |v: u32| v.saturating_mul(self.scale);
        let (low, high) = match self.kind {
            Kind::Fixed => (scaled(self.size), scaled(self.size)),
            Kind::Scalable => (scaled(self.min), scaled(self.max)),
            Kind::Threshold => (
                scaled(self.size.saturating_sub(self.threshold)),
                scaled(self.size.saturating_add(self.threshold)),
            ),
        };
        if size < low {
            low - size
        } else {
            size.saturating_sub(high)
        }
    }
}

pub(crate) fn parse_index(text: &str) -> Vec<Dir> {
    let mut groups: HashMap<&str, HashMap<&str, &str>> = HashMap::new();
    let mut current = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(name);
            groups.entry(name).or_default();
        } else if let (Some(group), Some((key, value))) = (current, line.split_once('=')) {
            groups
                .entry(group)
                .or_default()
                .insert(key.trim(), value.trim());
        }
    }
    let Some(theme) = groups.get("Icon Theme") else {
        return Vec::new();
    };
    let listed = |key: &str| -> Vec<&str> {
        theme
            .get(key)
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut dirs = Vec::new();
    for path in listed("Directories")
        .into_iter()
        .chain(listed("ScaledDirectories"))
    {
        if dirs.len() >= MAX_DIRS {
            break;
        }
        // Names become path components; never let an index escape its theme.
        if path.starts_with('/') || path.split('/').any(|part| part == "..") {
            continue;
        }
        let Some(group) = groups.get(path) else {
            continue;
        };
        let number = |key: &str| group.get(key).and_then(|v| v.parse::<u32>().ok());
        let Some(size) = number("Size") else { continue };
        dirs.push(Dir {
            path: path.to_owned(),
            size,
            min: number("MinSize").unwrap_or(size),
            max: number("MaxSize").unwrap_or(size),
            threshold: number("Threshold").unwrap_or(2),
            scale: number("Scale").unwrap_or(1).max(1),
            kind: match group.get("Type").copied() {
                Some("Fixed") => Kind::Fixed,
                Some("Scalable") => Kind::Scalable,
                _ => Kind::Threshold,
            },
        });
    }
    dirs
}
