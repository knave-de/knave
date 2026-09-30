//! Validated window appearance contract. Distances are logical pixels.
use crate::ConfigError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Edges {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Corners {
    pub top_left: u16,
    pub top_right: u16,
    pub bottom_right: u16,
    pub bottom_left: u16,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Gaps {
    pub outer: Edges,
    pub inner: Edges,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Border {
    pub width: Edges,
    pub radius: Corners,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct EdgeColors {
    pub top: String,
    pub right: String,
    pub bottom: String,
    pub left: String,
}
impl Default for EdgeColors {
    fn default() -> Self {
        Self {
            top: "#8090b0ff".into(),
            right: "#8090b0ff".into(),
            bottom: "#8090b0ff".into(),
            left: "#8090b0ff".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Shadow {
    pub enabled: bool,
    pub color: String,
    pub blur_radius: u16,
    pub spread: i16,
    pub offset_x: i16,
    pub offset_y: i16,
}
impl Default for Shadow {
    fn default() -> Self {
        Self {
            enabled: false,
            color: "#00000060".into(),
            blur_radius: 12,
            spread: 0,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Shadows {
    pub top: Shadow,
    pub right: Shadow,
    pub bottom: Shadow,
    pub left: Shadow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Blur {
    pub enabled: bool,
    pub radius: u16,
    pub passes: u8,
}
impl Default for Blur {
    fn default() -> Self {
        Self {
            enabled: false,
            radius: 8,
            passes: 1,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct FocusAppearance {
    pub opacity: u8,
    pub border_color: EdgeColors,
    pub shadows: Shadows,
    pub blur: Blur,
}
impl Default for FocusAppearance {
    fn default() -> Self {
        Self {
            opacity: 100,
            border_color: EdgeColors::default(),
            shadows: Shadows::default(),
            blur: Blur::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ViewEffects {
    pub gaps: bool,
    pub borders: bool,
    pub radius: bool,
    pub shadows: bool,
    pub opacity: bool,
    pub blur: bool,
}
impl Default for ViewEffects {
    fn default() -> Self {
        Self {
            gaps: true,
            borders: true,
            radius: true,
            shadows: true,
            opacity: true,
            blur: true,
        }
    }
}
impl ViewEffects {
    pub const NONE: Self = Self {
        gaps: false,
        borders: false,
        radius: false,
        shadows: false,
        opacity: false,
        blur: false,
    };
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Views {
    pub tiled: ViewEffects,
    pub floating: ViewEffects,
    pub maximized: ViewEffects,
}
impl Default for Views {
    fn default() -> Self {
        Self {
            tiled: ViewEffects::default(),
            floating: ViewEffects::default(),
            maximized: ViewEffects::NONE,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WindowAppearance {
    pub gaps: Gaps,
    pub border: Border,
    pub focused: FocusAppearance,
    pub unfocused: FocusAppearance,
    pub views: Views,
}

pub fn rgba(color: &str) -> Option<[f32; 4]> {
    let hex = color.strip_prefix('#')?;
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut result = [1.0; 4];
    for (index, part) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        result[index] =
            u8::from_str_radix(std::str::from_utf8(part).ok()?, 16).ok()? as f32 / 255.0;
    }
    Some(result)
}

impl WindowAppearance {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let bounded = |name: &str, value: i32, min: i32, max: i32| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(ConfigError::Invalid(format!(
                    "compositor.appearance.{name} must be between {min} and {max}"
                )))
            }
        };
        for (name, edges, max) in [
            ("gaps.outer", self.gaps.outer, 4096),
            ("gaps.inner", self.gaps.inner, 4096),
            ("border.width", self.border.width, 128),
        ] {
            for (edge, value) in [
                ("top", edges.top),
                ("right", edges.right),
                ("bottom", edges.bottom),
                ("left", edges.left),
            ] {
                bounded(&format!("{name}.{edge}"), value.into(), 0, max)?;
            }
        }
        for (corner, value) in [
            ("top_left", self.border.radius.top_left),
            ("top_right", self.border.radius.top_right),
            ("bottom_right", self.border.radius.bottom_right),
            ("bottom_left", self.border.radius.bottom_left),
        ] {
            bounded(&format!("border.radius.{corner}"), value.into(), 0, 4096)?;
        }
        for (state, style) in [("focused", &self.focused), ("unfocused", &self.unfocused)] {
            bounded(&format!("{state}.opacity"), style.opacity.into(), 0, 100)?;
            bounded(
                &format!("{state}.blur.radius"),
                style.blur.radius.into(),
                0,
                64,
            )?;
            bounded(
                &format!("{state}.blur.passes"),
                style.blur.passes.into(),
                1,
                4,
            )?;
            for (side, color, shadow) in [
                ("top", &style.border_color.top, &style.shadows.top),
                ("right", &style.border_color.right, &style.shadows.right),
                ("bottom", &style.border_color.bottom, &style.shadows.bottom),
                ("left", &style.border_color.left, &style.shadows.left),
            ] {
                for (name, color) in [
                    (format!("{state}.border_color.{side}"), color),
                    (format!("{state}.shadows.{side}.color"), &shadow.color),
                ] {
                    if rgba(color).is_none() {
                        return Err(ConfigError::Invalid(format!(
                            "compositor.appearance.{name} must be #RRGGBB or #RRGGBBAA"
                        )));
                    }
                }
                for (field, value, min, max) in [
                    ("blur_radius", i32::from(shadow.blur_radius), 0, 128),
                    ("spread", i32::from(shadow.spread), -128, 128),
                    ("offset_x", i32::from(shadow.offset_x), -256, 256),
                    ("offset_y", i32::from(shadow.offset_y), -256, 256),
                ] {
                    bounded(&format!("{state}.shadows.{side}.{field}"), value, min, max)?;
                }
            }
        }
        Ok(())
    }
}
