//! Selected desktop theme loading, shared by all shell surface hosts.
use knave_config::{ChromaPreference, ConfigDocument, ContrastPreference, Theme as Preference};
use knave_ui::toolkit::{Chroma, Contrast, Element, Scene, Theme, ThemePolicy, Widget, theme_file};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub policy: ThemePolicy,
    pub css: String,
}

impl Selection {
    pub fn fallback(theme: Theme) -> Self {
        Self {
            policy: ThemePolicy::new(theme, Contrast::Standard, Chroma::FullColor),
            css: String::new(),
        }
    }

    pub fn load() -> Result<Self, String> {
        let config = config_path()?;
        let data = theme_file::data_home().map_err(|error| error.to_string())?;
        Self::at(&config, &data)
    }

    fn at(config: &Path, data: &Path) -> Result<Self, String> {
        let document = ConfigDocument::load(config).map_err(|error| error.to_string())?;
        let appearance = &document.config().appearance;
        let theme = match appearance.theme {
            Preference::Light => Theme::Light,
            Preference::Dark | Preference::System => Theme::Dark,
        };
        let contrast = match appearance.contrast {
            ContrastPreference::Standard => Contrast::Standard,
            ContrastPreference::High => Contrast::High,
        };
        let chroma = match appearance.chroma {
            ChromaPreference::FullColor => Chroma::FullColor,
            ChromaPreference::Monochrome => Chroma::Monochrome,
        };
        let policy = ThemePolicy::new(theme, contrast, chroma);
        let path = theme_file::theme_css_path(data, &appearance.theme_name, theme)
            .map_err(|error| error.to_string())?;
        let css = theme_file::load_css(&path)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        // Validate before publishing to any surface, including the scene-less bar host.
        let mut scene =
            Scene::new(Element::new(0, Widget::Panel)).map_err(|error| error.to_string())?;
        scene
            .set_theme_policy_stylesheets(policy, knave_shell_ui::styles::STYLESHEET, &css)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self { policy, css })
    }
}

fn config_path() -> Result<PathBuf, String> {
    let path = ConfigDocument::default_path().map_err(|error| error.to_string())?;
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| error.to_string())
    }
}

pub(super) fn watch_paths() -> Vec<PathBuf> {
    let Ok(config) = config_path() else {
        return Vec::new();
    };
    let mut paths = vec![config.clone()];
    if let (Ok(document), Ok(data)) = (ConfigDocument::load(&config), theme_file::data_home()) {
        for variant in [Theme::Dark, Theme::Light] {
            if let Ok(path) =
                theme_file::theme_css_path(&data, &document.config().appearance.theme_name, variant)
            {
                paths.push(path);
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn selected_named_files_missing_invalid_and_mode_changes() {
        let base = std::env::temp_dir().join(format!("knave-shell-theme-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let config = base.join("config.toml");
        let theme = base.join("knave/themes/custom");
        fs::create_dir_all(&theme).unwrap();
        fs::write(
            &config,
            "[appearance.theme]\ntheme = \"custom\"\nmode = \"dark\"\n",
        )
        .unwrap();
        assert_eq!(
            Selection::at(&config, &base).unwrap(),
            Selection::fallback(Theme::Dark)
        );
        let css =
            ":root { --accent: #123456; } .quick-settings-panel { background-color: #234567; }";
        fs::write(theme.join("dark.css"), css).unwrap();
        assert_eq!(Selection::at(&config, &base).unwrap().css, css);
        fs::write(theme.join("light.css"), ":root { --accent: #abcdef; }").unwrap();
        fs::write(
            &config,
            "[appearance.theme]\ntheme = \"custom\"\nmode = \"light\"\ncontrast = \"high\"\nchroma = \"monochrome\"\n",
        )
        .unwrap();
        let selected = Selection::at(&config, &base).unwrap();
        assert_eq!(selected.policy.scheme, Theme::Light);
        assert_eq!(selected.policy.contrast, Contrast::High);
        assert_eq!(selected.policy.chroma, Chroma::Monochrome);
        assert!(selected.css.contains("#abcdef"));
        fs::write(theme.join("light.css"), "panel { unknown-property: nope; }").unwrap();
        assert!(Selection::at(&config, &base).is_err());
        fs::remove_dir_all(base).unwrap();
    }
}
