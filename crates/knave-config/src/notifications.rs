//! Canonical notification preferences; history and DND runtime state live separately.
use crate::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub history_limit: usize,
    pub history_days: u32,
    pub banner_timeout_ms: u32,
    pub max_banners: usize,
    pub critical_in_dnd: bool,
    pub app: BTreeMap<String, AppPolicy>,
}
impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history_limit: 200,
            history_days: 7,
            banner_timeout_ms: 6000,
            max_banners: 3,
            critical_in_dnd: false,
            app: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppPolicy {
    pub enabled: bool,
    pub banners: bool,
}
impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            banners: true,
        }
    }
}
impl NotificationConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=500).contains(&self.history_limit)
            || !(1..=365).contains(&self.history_days)
            || !(1000..=60000).contains(&self.banner_timeout_ms)
            || !(1..=3).contains(&self.max_banners)
            || self.app.len() > 256
            || self
                .app
                .keys()
                .any(|k| k.is_empty() || k.len() > 256 || k.chars().any(char::is_control))
        {
            return Err(ConfigError::Invalid("notifications: history_limit 1..500, history_days 1..365, banner_timeout_ms 1000..60000, max_banners 1..3; at most 256 named app policies".into()));
        }
        Ok(())
    }
}
pub(super) fn apply(document: &mut DocumentMut, config: &NotificationConfig) {
    // Merge through the existing table so unknown keys and comments survive settings writes.
    if let Some(t) = document
        .get("notifications")
        .and_then(Item::as_inline_table)
        .cloned()
    {
        document["notifications"] = Item::Table(t.into_table());
    }
    if document.get("notifications").is_none() {
        document["notifications"] = Item::Table(Table::new());
    }
    let n = &mut document["notifications"];
    n["enabled"] = value(config.enabled);
    n["history_limit"] = value(config.history_limit as i64);
    n["history_days"] = value(i64::from(config.history_days));
    n["banner_timeout_ms"] = value(i64::from(config.banner_timeout_ms));
    n["max_banners"] = value(config.max_banners as i64);
    n["critical_in_dnd"] = value(config.critical_in_dnd);
    for (id, policy) in &config.app {
        if n.get("app").is_none() {
            n["app"] = Item::Table(Table::new());
        }
        if n["app"].get(id).is_none() {
            n["app"][id] = Item::Table(Table::new());
        }
        n["app"][id]["enabled"] = value(policy.enabled);
        n["app"][id]["banners"] = value(policy.banners);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_bounds_and_preserves_unknown_settings() {
        let mut c = NotificationConfig::default();
        assert!(c.validate().is_ok());
        c.max_banners = 4;
        assert!(c.validate().is_err());
        c.max_banners = 3;
        c.app.insert(
            "org.test.App".into(),
            AppPolicy {
                enabled: true,
                banners: false,
            },
        );
        let mut d="[notifications]\nfuture = 42 # keep\n[notifications.app.\"org.test.App\"]\nfuture = true\n".parse().unwrap();
        apply(&mut d, &c);
        let s = d.to_string();
        assert!(s.contains("42 # keep"));
        assert!(s.contains("future = true"));
        let parsed: crate::Config = toml::from_str(&s).unwrap();
        assert_eq!(parsed.notifications, c);
    }
}

#[cfg(test)]
mod bounds {
    #[test]
    fn every_notification_setting_rejects_invalid_bounds() {
        for field in [
            "history_limit",
            "history_days",
            "banner_timeout_ms",
            "max_banners",
        ] {
            for invalid in [0, 1000000] {
                let source = format!("[notifications]\n{field} = {invalid}\n");
                let config: crate::Config = toml::from_str(&source).unwrap();
                assert!(config.validate().is_err(), "{field}={invalid}");
            }
        }
        for source in [
            "history_limit = 1\nhistory_days = 1\nbanner_timeout_ms = 1000\nmax_banners = 1",
            "history_limit = 500\nhistory_days = 365\nbanner_timeout_ms = 60000\nmax_banners = 3",
        ] {
            let config: crate::Config =
                toml::from_str(&format!("[notifications]\n{source}\n")).unwrap();
            assert!(config.validate().is_ok());
        }
    }
}
