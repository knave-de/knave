//! Session-owned system controls. Optional sections distinguish missing hardware/services.
use serde::{Deserialize, Serialize};
pub const BUS_NAME: &str = "org.knave.QuickSettings";
pub const OBJECT_PATH: &str = "/org/knave/QuickSettings";
pub const INTERFACE: &str = BUS_NAME;
pub const MAX_SNAPSHOT_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct QuickSettingsSnapshot {
    pub open: bool,
    pub light: bool,
    pub high_contrast: bool,
    pub network: Option<Network>,
    pub output: Option<Volume>,
    pub input: Option<Volume>,
    pub brightness: Option<u8>,
    pub bluetooth: Option<bool>,
    pub power: Option<Power>,
    pub battery: Option<Battery>,
    pub night: NightLight,
    pub errors: Vec<String>,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Network {
    pub ethernet: Option<String>,
    pub wifi: bool,
    pub wireless: Option<String>,
    pub connections: Vec<NetworkConnection>,
}
impl Network {
    pub fn summary(&self) -> String {
        if self.ethernet.is_some() {
            "Ethernet connected · Preferred".into()
        } else if let Some(name) = &self.wireless {
            format!("{name} · Wi-Fi")
        } else if self.wifi {
            "Wi-Fi on · Disconnected".into()
        } else {
            "Disconnected".into()
        }
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NetworkConnection {
    pub uuid: String,
    pub name: String,
    pub wired: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Volume {
    pub percent: u8,
    pub muted: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Power {
    pub active: String,
    pub available: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Battery {
    pub name: String,
    pub percent: u8,
    pub status: String,
    /// Read-only driver report; some drivers return a cached request.
    pub driver_limit: Option<u8>,
    /// None means UPower could not be queried; unsupported is explicit below.
    pub charge_limit: Option<ChargeLimit>,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ChargeLimit {
    pub supported: bool,
    pub enabled: bool,
    /// UPower's supported preset thresholds, not measured charging cutoffs.
    pub start_threshold: Option<u8>,
    pub end_threshold: Option<u8>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct NightLight {
    pub enabled: bool,
    pub active: bool,
    pub temperature: u16,
    pub start: Option<u16>,
    pub end: Option<u16>,
}
impl Default for NightLight {
    fn default() -> Self {
        Self {
            enabled: false,
            active: false,
            temperature: 4000,
            start: None,
            end: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum QuickSettingsCommand {
    Toggle,
    Close,
    Refresh,
    SetWifi {
        enabled: bool,
    },
    Connect {
        uuid: String,
    },
    DisconnectEthernet,
    SetVolume {
        input: bool,
        percent: u8,
    },
    SetMute {
        input: bool,
        muted: bool,
    },
    OpenAudio,
    OpenNetwork,
    SetBrightness {
        percent: u8,
    },
    SetBluetooth {
        enabled: bool,
    },
    SetPowerProfile {
        profile: String,
    },
    SetChargeLimit {
        enabled: bool,
    },
    SetNightLight {
        enabled: bool,
        temperature: u16,
        start: Option<u16>,
        end: Option<u16>,
    },
    Suspend,
    Reboot,
    PowerOff,
}
