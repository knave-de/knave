//! Notification presentation contract. Live protocol IDs never identify archived actions.
use serde::{Deserialize, Serialize};

pub const BUS_NAME: &str = "org.knave.Notifications";
pub const OBJECT_PATH: &str = "/org/knave/Notifications";
pub const INTERFACE: &str = "org.knave.Notifications";
pub const STANDARD_NAME: &str = "org.freedesktop.Notifications";
pub const STANDARD_PATH: &str = "/org/freedesktop/Notifications";
/// Maximum decoded preview dimensions and aggregate retained pixel memory.
pub const MAX_IMAGE_EDGE: u32 = 320;
pub const MAX_IMAGE_BYTES: usize = MAX_IMAGE_EDGE as usize * MAX_IMAGE_EDGE as usize * 4;
pub const MAX_TOTAL_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_NOTIFICATION_RECORD_BYTES: usize = 1024 * 1024;

/// Straight-alpha sRGB RGBA8; encoded as base64 in snapshots/history.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationImage {
    width: u32,
    height: u32,
    pixels: std::sync::Arc<[u8]>,
}
impl NotificationImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        if width == 0
            || height == 0
            || width > MAX_IMAGE_EDGE
            || height > MAX_IMAGE_EDGE
            || pixels.len() != width as usize * height as usize * 4
        {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels: pixels.into(),
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
impl Serialize for NotificationImage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use base64::Engine;
        #[derive(Serialize)]
        struct Wire {
            width: u32,
            height: u32,
            rgba: String,
        }
        Wire {
            width: self.width,
            height: self.height,
            rgba: base64::engine::general_purpose::STANDARD.encode(&self.pixels),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for NotificationImage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use base64::Engine;
        use serde::de::Error;
        #[derive(Deserialize)]
        struct Wire {
            width: u32,
            height: u32,
            rgba: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.rgba.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
            return Err(D::Error::custom("notification image exceeds budget"));
        }
        let pixels = base64::engine::general_purpose::STANDARD
            .decode(wire.rgba)
            .map_err(D::Error::custom)?;
        Self::new(wire.width, wire.height, pixels)
            .ok_or_else(|| D::Error::custom("invalid notification image dimensions or pixels"))
    }
}

pub const MAX_HISTORY: usize = 500;
pub const MAX_LIVE: usize = 64;
pub const MAX_ACTIONS: usize = 8;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Notification {
    pub id: u64,
    pub app: String,
    pub app_id: String,
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<NotificationImage>,
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

#[cfg(test)]
mod image_tests {
    use super::*;
    #[test]
    fn image_wire_roundtrip_is_bounded_and_rejects_inconsistent_pixels() {
        let image = NotificationImage::new(1, 1, vec![88, 101, 242, 128]).unwrap();
        let json = serde_json::to_string(&image).unwrap();
        assert!(json.contains("rgba"));
        assert_eq!(
            serde_json::from_str::<NotificationImage>(&json).unwrap(),
            image
        );
        for json in [
            r#"{"width":0,"height":1,"rgba":"AAAAAA=="}"#,
            r#"{"width":321,"height":1,"rgba":"AAAAAA=="}"#,
            r#"{"width":2,"height":1,"rgba":"AAAAAA=="}"#,
            r#"{"width":1,"height":1,"rgba":"invalid"}"#,
        ] {
            assert!(serde_json::from_str::<NotificationImage>(json).is_err());
        }
        assert!(NotificationImage::new(u32::MAX, u32::MAX, vec![]).is_none());
        let notification = Notification::default();
        let json = serde_json::to_string(&notification).unwrap();
        assert!(!json.contains("image"));
        assert!(
            serde_json::from_str::<Notification>(&json)
                .unwrap()
                .image
                .is_none()
        );
    }
}
