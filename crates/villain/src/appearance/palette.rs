//! Decoration palette resolved at configuration load, never during frame rendering.
use knave_config::{AppearanceConfig, ChromaPreference, ContrastPreference, Theme as Preference};
use knave_ui::toolkit::{
    Chroma, Contrast, Element, ElementId, Scene, Theme, ThemePolicy, Widget, theme_file,
};

#[derive(Clone, Debug)]
pub(crate) struct ThemeWindowStyle {
    pub active_border: knave_config::appearance::EdgeColors,
    pub inactive_border: knave_config::appearance::EdgeColors,
    pub focused_shadows: Vec<knave_config::appearance::Shadow>,
    pub focused_outline: Option<knave_config::appearance::EdgeColors>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecorationPalette {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub hover: [u8; 4],
    pub close_hover: [u8; 4],
    pub close_foreground: [u8; 4],
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
        let mut close_foreground = [0; 4];
        for (token, text_token) in [
            ("--surface", "--text"),
            ("--surface-hover", "--text"),
            ("--danger", "--on-danger"),
        ] {
            let stylesheet = format!(
                "{css}\npanel {{ background-color: var({token}); color: var({text_token}); }}"
            );
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
            colors.push([fill.red, fill.green, fill.blue, fill.alpha]);
            let text = style.text.color;
            if token == "--surface" {
                foreground = [text.red, text.green, text.blue, text.alpha];
            }
            if token == "--danger" {
                close_foreground = [text.red, text.green, text.blue, text.alpha];
            }
        }
        Ok(Self {
            background: colors[0],
            foreground,
            hover: colors[1],
            close_hover: colors[2],
            close_foreground,
        })
    }
}

impl ThemeWindowStyle {
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
        scene
            .set_theme_policy_stylesheets(policy, "", css)
            .map_err(|error| error.to_string())?;
        let active = scene
            .theme_color("--window-border-color")
            .ok_or("window border theme token missing")?;
        let inactive = scene
            .theme_color("--border")
            .ok_or("inactive border theme token missing")?;
        let shadow = scene
            .theme_value("--shadow-active-window")
            .ok_or("focused shadow theme token missing")?;
        let focused_outline = if appearance.contrast == ContrastPreference::High {
            let color = scene
                .theme_color("--text")
                .ok_or("high-contrast focus outline token missing")?;
            Some(edge_colors(color))
        } else {
            None
        };
        let active_border = edge_colors(active);
        let inactive_border = edge_colors(inactive);
        let focused_shadows = parse_box_shadows(&shadow)?;
        Ok(Self {
            active_border,
            inactive_border,
            focused_shadows,
            focused_outline,
        })
    }
}

fn edge_colors(color: knave_ui::Color) -> knave_config::appearance::EdgeColors {
    let color = format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.red, color.green, color.blue, color.alpha
    );
    knave_config::appearance::EdgeColors {
        top: color.clone(),
        right: color.clone(),
        bottom: color.clone(),
        left: color,
    }
}

fn parse_box_shadows(value: &str) -> Result<Vec<knave_config::appearance::Shadow>, String> {
    use knave_config::appearance::{Shadow, rgba};

    let value = value.trim();
    if value == "none" {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|layer| {
            let mut color = None;
            let mut lengths = Vec::new();
            for token in layer.split_whitespace() {
                if token.starts_with('#') {
                    if rgba(token).is_none() || color.replace(token.to_owned()).is_some() {
                        return Err(format!("invalid theme window shadow: {value}"));
                    }
                } else if token == "0" {
                    lengths.push(0);
                } else if let Some(px) = token.strip_suffix("px") {
                    lengths.push(
                        px.parse::<i16>()
                            .map_err(|_| format!("invalid theme window shadow: {value}"))?,
                    );
                } else {
                    return Err(format!("unsupported theme window shadow: {value}"));
                }
            }
            if !(3..=4).contains(&lengths.len()) {
                return Err(format!("invalid theme window shadow: {value}"));
            }
            let color = color.ok_or_else(|| format!("invalid theme window shadow: {value}"))?;
            let blur_radius = u16::try_from(lengths[2])
                .map_err(|_| format!("invalid theme window shadow: {value}"))?;
            let spread = lengths.get(3).copied().unwrap_or_default();
            if blur_radius > 128
                || !(-128..=128).contains(&spread)
                || !(-256..=256).contains(&lengths[0])
                || !(-256..=256).contains(&lengths[1])
            {
                return Err(format!(
                    "theme window shadow exceeds supported bounds: {value}"
                ));
            }
            Ok(Shadow {
                enabled: true,
                color,
                blur_radius,
                spread,
                offset_x: lengths[0],
                offset_y: lengths[1],
            })
        })
        .collect()
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
        let translucent = DecorationPalette::resolve(
            &appearance,
            2,
            ":root { --danger: #00000010; --on-danger: #ffffff; }",
        )
        .unwrap();
        assert_eq!(translucent.close_hover, [0, 0, 0, 16]);
        assert_eq!(translucent.close_foreground, [255, 255, 255, 255]);
        let titlebar = DecorationPalette::resolve(
            &appearance,
            2,
            ":root { --surface: #111122; --surface-hover: #223344; --text: #123456; }",
        )
        .unwrap();
        assert_eq!(titlebar.background, [0x11, 0x11, 0x22, 255]);
        assert_eq!(titlebar.hover, [0x22, 0x33, 0x44, 255]);
        assert_eq!(titlebar.foreground, [0x12, 0x34, 0x56, 255]);
        appearance.contrast = ContrastPreference::High;
        appearance.chroma = ChromaPreference::Monochrome;
        let accessible = DecorationPalette::resolve(&appearance, 2, "").unwrap();
        assert_eq!(accessible.foreground, [0, 0, 0, 255]);
        assert!(DecorationPalette::resolve(&appearance, 0, "panel { invalid: value; }").is_err());
    }

    #[test]
    fn high_contrast_focus_edges_are_explicit_across_all_profiles() {
        let cases = [
            (
                Preference::Dark,
                ChromaPreference::FullColor,
                "#FFFFFFFF",
                "#00516BFF",
            ),
            (
                Preference::Light,
                ChromaPreference::FullColor,
                "#071820FF",
                "#8FE7FFFF",
            ),
            (
                Preference::Dark,
                ChromaPreference::Monochrome,
                "#FFFFFFFF",
                "#FFFFFFFF",
            ),
            (
                Preference::Light,
                ChromaPreference::Monochrome,
                "#000000FF",
                "#000000FF",
            ),
        ];
        for (theme, chroma, outline, active_border) in cases {
            let appearance = AppearanceConfig {
                theme,
                contrast: ContrastPreference::High,
                chroma,
                ..AppearanceConfig::default()
            };
            let style = ThemeWindowStyle::resolve(&appearance, 0, "").unwrap();
            assert_eq!(style.focused_outline.as_ref().unwrap().top, outline);
            assert_eq!(style.active_border.top, active_border);
            assert!(style.focused_shadows.is_empty());
        }
        let standard = ThemeWindowStyle::resolve(&AppearanceConfig::default(), 0, "").unwrap();
        assert!(standard.focused_outline.is_none());
    }

    #[test]
    fn focused_shadow_tokens_keep_each_box_shadow_layer() {
        let shadows = parse_box_shadows("0 0 14px #60CBED26, 0 4px 16px #00000055").unwrap();
        assert_eq!(shadows.len(), 2);
        assert_eq!(shadows[0].color, "#60CBED26");
        assert_eq!(shadows[0].blur_radius, 14);
        assert_eq!(shadows[1].offset_y, 4);
        assert_eq!(shadows[1].blur_radius, 16);
        assert!(parse_box_shadows("none").unwrap().is_empty());
    }
}
