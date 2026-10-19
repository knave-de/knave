//! Gesture configuration only; Villain owns defaults and runtime input policy.
use crate::ConfigError;
use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, Item, Table, value};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GestureConfig {
    #[serde(default)]
    pub bind: Vec<GestureBinding>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GestureBinding {
    pub fingers: u32,
    pub direction: GestureDirection,
    pub action: GestureAction,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GestureDirection {
    Horizontal,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GestureAction {
    Workspace,
    None,
}

impl GestureConfig {
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if self.bind.iter().any(|binding| binding.fingers == 0) {
            return Err(ConfigError::Invalid(
                "compositor.gesture.bind.fingers must be at least 1".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn write(compositor: &mut Item, config: &GestureConfig) {
    if compositor.get("gesture").is_none() {
        compositor["gesture"] = Item::Table(Table::new());
    }
    let previous = compositor["gesture"]
        .get("bind")
        .and_then(Item::as_array_of_tables);
    let mut bindings = ArrayOfTables::new();
    for (index, binding) in config.bind.iter().enumerate() {
        // Match source entries by identity so unknown fields survive reordering.
        let mut table = previous
            .and_then(|tables| {
                tables
                    .get(index)
                    .filter(|table| {
                        table.get("fingers").and_then(Item::as_integer)
                            == Some(i64::from(binding.fingers))
                            && table.get("direction").and_then(Item::as_str) == Some("horizontal")
                    })
                    .or_else(|| {
                        tables
                            .iter()
                            .filter(|table| {
                                table.get("fingers").and_then(Item::as_integer)
                                    == Some(i64::from(binding.fingers))
                                    && table.get("direction").and_then(Item::as_str)
                                        == Some("horizontal")
                            })
                            .last()
                    })
            })
            .cloned()
            .unwrap_or_default();
        table["fingers"] = value(i64::from(binding.fingers));
        table["direction"] = value("horizontal");
        table["action"] = value(match binding.action {
            GestureAction::Workspace => "workspace",
            GestureAction::None => "none",
        });
        bindings.push(table);
    }
    compositor["gesture"]["bind"] = Item::ArrayOfTables(bindings);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConfigDocument;
    #[test]
    fn round_trip_preserves_unknown_gesture_fields() {
        let path = std::env::temp_dir().join(format!("knave-gesture-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            r#"schema_version = 2
[compositor.gesture]
future = true
[[compositor.gesture.bind]]
fingers = 3
direction = "horizontal"
action = "none"
future_option = 42
"#,
        )
        .unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        let mut config = document.config().clone();
        config.compositor.gesture.bind[0].action = GestureAction::Workspace;
        document.write(config.clone()).unwrap();
        assert!(document.source().contains("future = true"));
        assert!(document.source().contains("future_option = 42"));
        assert_eq!(ConfigDocument::load(&path).unwrap().config(), &config);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn validates_fingers_and_rejects_unknown_enums() {
        for field in [
            "fingers = 0\ndirection = 'horizontal'\naction = 'workspace'",
            "fingers = 3\ndirection = 'diagonal'\naction = 'workspace'",
            "fingers = 3\ndirection = 'horizontal'\naction = 'shell'",
        ] {
            let parsed = toml::from_str::<GestureConfig>(&format!("[[bind]]\n{field}"));
            assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
        }
        assert!(GestureConfig::default().bind.is_empty());
    }
}
