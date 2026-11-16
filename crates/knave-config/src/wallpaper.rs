//! Effective wallpaper projection. An explicit `[shell.wallpaper].image`
//! wins; otherwise the active theme's default for the current mode, contrast
//! and chroma is looked up in the XDG data directories.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::{AppearanceConfig, ChromaPreference, Config, ContrastPreference, Theme, WallpaperFit};

/// Theme whose wallpapers are used when the selected theme ships none.
const FALLBACK_THEME: &str = "knave";
/// Installed and workspace-tree assets, relative to a data directory.
const THEMES_DIRECTORY: &str = "knave/themes";

/// The image the wallpaper surface should present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveWallpaper {
    pub image: String,
    pub fit: WallpaperFit,
    pub source: WallpaperSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallpaperSource {
    Configured,
    ThemeDefault,
}

/// Ordered data directories searched for theme wallpapers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataDirectories(pub Vec<PathBuf>);

impl DataDirectories {
    /// XDG data home, the running executable's install prefix, XDG data dirs,
    /// then the source tree for workspace builds.
    pub fn from_environment() -> Self {
        Self::from_parts(
            env::var_os("XDG_DATA_HOME"),
            env::var_os("HOME"),
            env::var_os("XDG_DATA_DIRS"),
            env::current_exe().ok().as_deref(),
        )
    }

    fn from_parts(
        data_home: Option<OsString>,
        home: Option<OsString>,
        data_dirs: Option<OsString>,
        executable: Option<&Path>,
    ) -> Self {
        let mut directories = Vec::new();
        match data_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            Some(path) => directories.push(path),
            None => directories.extend(
                home.map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|home| home.join(".local/share")),
            ),
        }
        // `--prefix` installs are not necessarily on XDG_DATA_DIRS.
        if let Some(bin) = executable.and_then(Path::parent)
            && bin.file_name().is_some_and(|name| name == "bin")
            && let Some(prefix) = bin.parent()
        {
            directories.push(prefix.join("share"));
        }
        let system = data_dirs
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        directories.extend(env::split_paths(&system).filter(|path| path.is_absolute()));
        // Uninstalled workspace binaries live in target/<profile>/ and use the
        // checked-in assets so development and harness runs match installs.
        if executable
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|profile| profile == "debug" || profile == "release")
        {
            directories.extend(workspace_share());
        }
        let mut unique = Vec::with_capacity(directories.len());
        for directory in directories {
            if !unique.contains(&directory) {
                unique.push(directory);
            }
        }
        Self(unique)
    }
}

/// Checked-in `share/` of the workspace this crate was built from.
fn workspace_share() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|root| root.join("share"))
}

/// File stem of the default wallpaper for an appearance. `system` follows the
/// documented dark fallback; monochrome already meets high contrast.
pub fn theme_variant(appearance: &AppearanceConfig) -> &'static str {
    let light = appearance.theme == Theme::Light;
    match (light, appearance.chroma, appearance.contrast) {
        (false, ChromaPreference::Monochrome, _) => "dark-monochrome",
        (true, ChromaPreference::Monochrome, _) => "light-monochrome",
        (false, ChromaPreference::FullColor, ContrastPreference::High) => "dark-high-contrast",
        (true, ChromaPreference::FullColor, ContrastPreference::High) => "light-high-contrast",
        (false, ChromaPreference::FullColor, ContrastPreference::Standard) => "dark-standard",
        (true, ChromaPreference::FullColor, ContrastPreference::Standard) => "light-standard",
    }
}

/// First installed default wallpaper for the selected theme, then for Knave's.
pub fn theme_default(appearance: &AppearanceConfig, data: &DataDirectories) -> Option<PathBuf> {
    let file = format!("{}.png", theme_variant(appearance));
    let mut themes = vec![appearance.theme_name.as_str()];
    if appearance.theme_name != FALLBACK_THEME {
        themes.push(FALLBACK_THEME);
    }
    themes.into_iter().find_map(|theme| {
        data.0
            .iter()
            .map(|root| {
                root.join(THEMES_DIRECTORY)
                    .join(theme)
                    .join("wallpapers")
                    .join(&file)
            })
            .find(|path| path.is_file())
    })
}

impl Config {
    /// Explicit image, else the installed theme default; `None` when neither
    /// exists and the desktop has no wallpaper surface.
    pub fn effective_wallpaper(&self, data: &DataDirectories) -> Option<EffectiveWallpaper> {
        let fit = self.shell.wallpaper.fit;
        if let Some(image) = &self.shell.wallpaper.image {
            return Some(EffectiveWallpaper {
                image: image.clone(),
                fit,
                source: WallpaperSource::Configured,
            });
        }
        let path = theme_default(&self.appearance, data)?;
        // Data directories are absolute, so the joined path is too.
        Some(EffectiveWallpaper {
            image: path.to_string_lossy().into_owned(),
            fit,
            source: WallpaperSource::ThemeDefault,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn install(root: &Path, theme: &str, variant: &str) -> PathBuf {
        let directory = root.join(THEMES_DIRECTORY).join(theme).join("wallpapers");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("{variant}.png"));
        fs::write(&path, b"png").unwrap();
        path
    }

    fn appearance(
        theme: Theme,
        contrast: ContrastPreference,
        chroma: ChromaPreference,
    ) -> AppearanceConfig {
        AppearanceConfig {
            theme,
            contrast,
            chroma,
            ..AppearanceConfig::default()
        }
    }

    #[test]
    fn variants_follow_mode_contrast_and_chroma() {
        use ChromaPreference::*;
        use ContrastPreference::*;
        let cases = [
            (Theme::System, Standard, FullColor, "dark-standard"),
            (Theme::Dark, Standard, FullColor, "dark-standard"),
            (Theme::Light, Standard, FullColor, "light-standard"),
            (Theme::Dark, High, FullColor, "dark-high-contrast"),
            (Theme::Light, High, FullColor, "light-high-contrast"),
            (Theme::Dark, Standard, Monochrome, "dark-monochrome"),
            (Theme::Light, High, Monochrome, "light-monochrome"),
        ];
        for (theme, contrast, chroma, expected) in cases {
            assert_eq!(
                theme_variant(&appearance(theme, contrast, chroma)),
                expected
            );
        }
    }

    #[test]
    fn configured_image_wins_over_theme_default() {
        let root = tempfile::tempdir().unwrap();
        install(root.path(), "knave", "dark-standard");
        let mut config = Config::default();
        config.shell.wallpaper.image = Some("/chosen.png".into());
        let effective = config
            .effective_wallpaper(&DataDirectories(vec![root.path().into()]))
            .unwrap();
        assert_eq!(effective.image, "/chosen.png");
        assert_eq!(effective.source, WallpaperSource::Configured);
    }

    #[test]
    fn theme_default_prefers_earlier_directories_and_selected_theme() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let data = DataDirectories(vec![user.path().into(), system.path().into()]);
        let mut config = Config::default();
        config.appearance.theme = Theme::Light;

        assert_eq!(config.effective_wallpaper(&data), None);

        let system_knave = install(system.path(), "knave", "light-standard");
        let effective = config.effective_wallpaper(&data).unwrap();
        assert_eq!(Path::new(&effective.image), system_knave);
        assert_eq!(effective.source, WallpaperSource::ThemeDefault);

        let user_knave = install(user.path(), "knave", "light-standard");
        assert_eq!(theme_default(&config.appearance, &data), Some(user_knave));

        // A selected theme's own art beats Knave's, even from a later root.
        config.appearance.theme_name = "custom".into();
        let custom = install(system.path(), "custom", "light-standard");
        assert_eq!(theme_default(&config.appearance, &data), Some(custom));

        // A theme without that variant falls back to Knave's.
        config.appearance.contrast = ContrastPreference::High;
        let knave_high = install(system.path(), "knave", "light-high-contrast");
        assert_eq!(theme_default(&config.appearance, &data), Some(knave_high));
    }

    #[test]
    fn data_directories_follow_xdg_install_prefix_and_workspace_order() {
        let installed = DataDirectories::from_parts(
            None,
            Some("/home/user".into()),
            Some("/opt/share:relative:/usr/share".into()),
            Some(Path::new("/opt/knave/bin/knave")),
        );
        assert_eq!(
            installed.0,
            [
                PathBuf::from("/home/user/.local/share"),
                PathBuf::from("/opt/knave/share"),
                PathBuf::from("/opt/share"),
                PathBuf::from("/usr/share"),
            ]
        );

        let workspace = DataDirectories::from_parts(
            Some("/data".into()),
            None,
            None,
            Some(Path::new("/src/knave/target/debug/knave")),
        );
        assert_eq!(
            workspace.0[..3],
            [
                PathBuf::from("/data"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        );
        assert_eq!(workspace.0.get(3), workspace_share().as_ref());
    }

    #[test]
    fn checked_in_knave_theme_ships_every_variant() {
        let data = DataDirectories(workspace_share().into_iter().collect());
        for theme in [Theme::Dark, Theme::Light] {
            for contrast in [ContrastPreference::Standard, ContrastPreference::High] {
                for chroma in [ChromaPreference::FullColor, ChromaPreference::Monochrome] {
                    let appearance = appearance(theme, contrast, chroma);
                    assert!(
                        theme_default(&appearance, &data).is_some(),
                        "missing {}",
                        theme_variant(&appearance)
                    );
                }
            }
        }
    }
}
