//! Notification presentation contract. Live protocol IDs never identify archived actions.
use serde::{Deserialize, Serialize};

pub const BUS_NAME: &str = "org.knave.Notifications";
pub const OBJECT_PATH: &str = "/org/knave/Notifications";
pub const INTERFACE: &str = "org.knave.Notifications";
pub const STANDARD_NAME: &str = "org.freedesktop.Notifications";
pub const STANDARD_PATH: &str = "/org/freedesktop/Notifications";
pub const MAX_HISTORY: usize = 500;
pub const MAX_LIVE: usize = 64;
pub const MAX_ACTIONS: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Notification {
    pub id: u64,
    pub app: String,
    pub app_id: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub created_ms: u64,
    pub unread: bool,
    pub urgency: u8,
    pub progress: Option<u8>,
    pub live: bool,
    #[serde(default)]
    pub transient: bool,
    pub banner: bool,
    pub actions: Vec<NotificationAction>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NotificationAction {
    pub key: String,
    pub label: String,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NotificationSnapshot {
    pub owned: bool,
    pub dnd: bool,
    pub center_open: bool,
    pub light: bool,
    pub high_contrast: bool,
    pub history_days: u32,
    pub history_limit: usize,
    pub notifications: Vec<Notification>,
    pub app_banners: std::collections::BTreeMap<String, bool>,
    pub error: Option<String>,
}
impl NotificationSnapshot {
    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.notifications.iter().filter(|n| !n.transient)
    }
    pub fn unread(&self) -> usize {
        self.history().filter(|n| n.unread).count()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NotificationCommand {
    ToggleCenter,
    CloseCenter,
    SetDnd { enabled: bool },
    Seen { ids: Vec<u64> },
    Dismiss { id: u64 },
    Clear,
    Invoke { id: u64, action: String },
    SetAppBanners { app_id: String, enabled: bool },
}
