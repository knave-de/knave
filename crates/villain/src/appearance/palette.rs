//! Decoration palette resolved at configuration load, never during frame rendering.
use knave_config::{AppearanceConfig, ChromaPreference, ContrastPreference, Theme as Preference};
use knave_ui::toolkit::{
    Chroma, Contrast, Element, ElementId, Scene, Theme, ThemePolicy, Widget, theme_file,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecorationPalette {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub hover: [u8; 4],
    pub close_hover: [u8; 4],
}

impl Default for DecorationPalette {
    fn default() -> Self {
        Self::resolve(&AppearanceConfig::default(), 0, "").expect("bundled decoration palette")
    }
}

impl DecorationPalette {
    pub fn load(config: &knave_config::Config) -> Result<Self, String> {
        let appearance = &config.appearance;
        let theme = scheme(appearance.theme, config.portal.color_scheme);
        let data = theme_file::data_home().map_err(|error| error.to_string())?;
        let path = theme_file::theme_css_path(data, &appearance.theme_name, theme)
            .map_err(|error| error.to_string())?;
        let css = theme_file::load_css(&path)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        Self::resolve(appearance, config.portal.color_scheme, &css)
    }

    fn resolve(appearance: &AppearanceConfig, fallback: u8, css: &str) -> Result<Self, String> {
        let policy = ThemePolicy::new(
            scheme(appearance.theme, fallback),
            match appearance.contrast {
                ContrastPreference::Standard => Contrast::Standard,
                ContrastPreference::High => Contrast::High,
            },
            match appearance.chroma {
                ChromaPreference::FullColor => Chroma::FullColor,
                ChromaPreference::Monochrome => Chroma::Monochrome,
            },
        );
        let mut scene =
            Scene::new(Element::new(1, Widget::Panel)).map_err(|error| error.to_string())?;
        let mut colors = Vec::with_capacity(3);
        let mut foreground = [0; 4];
        for token in ["--surface-hover", "--surface-pressed", "--danger-surface"] {
            let stylesheet =
                format!("{css}\npanel {{ background-color: var({token}); color: var(--text); }}");
            scene
                .set_theme_policy_stylesheets(policy, "", &stylesheet)
                .map_err(|error| error.to_string())?;
            let style = scene
                .computed_style(ElementId(1))
                .ok_or("decoration style missing")?
                .style;
            let fill = style
                .background
                .and_then(|background| background.fill)
                .ok_or("decoration fill missing")?;
            colors.push([fill.red, fill.green, fill.blue, 255]);
            let text = style.text.color;
            foreground = [text.red, text.green, text.blue, 255];
        }
        Ok(Self {
            background: colors[0],
            foreground,
            hover: colors[1],
            close_hover: colors[2],
        })
    }
}

fn scheme(preference: Preference, fallback: u8) -> Theme {
    match preference {
        Preference::Light => Theme::Light,
        Preference::System if fallback == 2 => Theme::Light,
        Preference::Dark | Preference::System => Theme::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheme_custom_palette_and_accessibility_are_resolved() {
        let mut appearance = AppearanceConfig::default();
        let dark = DecorationPalette::resolve(&appearance, 0, "").unwrap();
        appearance.theme = Preference::Light;
        let light = DecorationPalette::resolve(&appearance, 0, "").unwrap();
        assert_ne!(dark.foreground, light.foreground);
        appearance.theme = Preference::System;
        assert_eq!(
            DecorationPalette::resolve(&appearance, 2, "").unwrap(),
            light
        );
        let custom =
            DecorationPalette::resolve(&appearance, 2, ":root { --text: #123456; }").unwrap();
        assert_eq!(custom.foreground, [0x12, 0x34, 0x56, 255]);
        appearance.contrast = ContrastPreference::High;
        appearance.chroma = ChromaPreference::Monochrome;
        let accessible = DecorationPalette::resolve(&appearance, 2, "").unwrap();
        assert_eq!(accessible.foreground, [0, 0, 0, 255]);
        assert!(DecorationPalette::resolve(&appearance, 0, "panel { invalid: value; }").is_err());
    }
}
