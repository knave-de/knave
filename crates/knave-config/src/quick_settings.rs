use crate::ConfigError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct QuickSettingsConfig {
    pub night_light: NightLightConfig,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct NightLightConfig {
    pub enabled: bool,
    pub temperature: u16,
    /// Local minutes after midnight; both absent means manual mode.
    pub start: Option<u16>,
    pub end: Option<u16>,
}
impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 4000,
            start: None,
            end: None,
        }
    }
}
impl QuickSettingsConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let n = &self.night_light;
        if !(2500..=6500).contains(&n.temperature)
            || !matches!((n.start, n.end), (None, None))
                && !matches!((n.start,n.end),(Some(a),Some(b)) if a<1440 && b<1440 && a!=b)
        {
            return Err(ConfigError::Invalid(
                "invalid quick-settings temperature or schedule".into(),
            ));
        }
        Ok(())
    }
    pub fn night_active(&self, minute: u16) -> bool {
        let n = &self.night_light;
        n.enabled
            && match (n.start, n.end) {
                (Some(a), Some(b)) if a < b => (a..b).contains(&minute),
                (Some(a), Some(b)) => minute >= a || minute < b,
                _ => true,
            }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedules_cross_midnight_and_validate() {
        let mut c = QuickSettingsConfig::default();
        c.night_light.enabled = true;
        c.night_light.start = Some(1140);
        c.night_light.end = Some(420);
        assert!(c.night_active(1200));
        assert!(c.night_active(419));
        assert!(!c.night_active(420));
        assert!(!c.night_active(900));
        c.night_light.end = Some(1140);
        assert!(c.validate().is_err());
    }
}

#[cfg(test)]
mod document_tests {
    use crate::ConfigDocument;
    #[test]
    fn switching_schedule_to_manual_removes_only_known_optional_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path,"[quick_settings.night_light]\nenabled=true\ntemperature=4000\nstart=1140\nend=420\nfuture_setting='preserved'\n").unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        let mut config = document.config().clone();
        config.quick_settings.night_light.start = None;
        config.quick_settings.night_light.end = None;
        document.write(config).unwrap();
        let reloaded = ConfigDocument::load(&path).unwrap();
        assert!(reloaded.config().quick_settings.night_light.start.is_none());
        assert!(reloaded.config().quick_settings.night_light.end.is_none());
        assert!(reloaded.source().contains("future_setting='preserved'"));
    }
}

#[cfg(test)]
mod retired_charge_setting_tests {
    use crate::ConfigDocument;
    #[test]
    fn old_percentage_is_retired_without_losing_unknown_settings() {
        for input in [
            "[quick_settings]\ncharge_limit=95\nfuture_setting='keep'\n",
            "quick_settings = { charge_limit=95, future_setting='keep' }\n",
        ] {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("config.toml");
            std::fs::write(&p, input).unwrap();
            let mut doc = ConfigDocument::load(&p).unwrap();
            let config = doc.config().clone();
            doc.write(config).unwrap();
            let source = std::fs::read_to_string(p).unwrap();
            assert!(!source.contains("charge_limit"));
            let values: toml::Value = toml::from_str(&source).unwrap();
            assert_eq!(
                values["quick_settings"]["future_setting"].as_str(),
                Some("keep")
            );
        }
    }
}
