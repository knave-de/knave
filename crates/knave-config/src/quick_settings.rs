use crate::ConfigError;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct QuickSettingsConfig {
    pub night_light: NightLightConfig,
    /// None leaves the firmware's existing limit alone.
    pub charge_limit: Option<u8>,
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
            || self.charge_limit.is_some_and(|v| !(50..=100).contains(&v))
            || !matches!((n.start, n.end), (None, None))
                && !matches!((n.start,n.end),(Some(a),Some(b)) if a<1440 && b<1440 && a!=b)
        {
            return Err(ConfigError::Invalid(
                "invalid quick-settings temperature, schedule or charge limit".into(),
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
