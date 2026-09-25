//! Versioned contracts for the Knave desktop environment.

use serde::{Deserialize, Serialize};

pub const API_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WorkspaceId(pub u32);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub active: bool,
    pub window_count: u32,
    pub visible_window_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowSummary {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub workspace: WorkspaceId,
    pub focused: bool,
    pub minimized: bool,
    pub floating: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopSnapshot {
    pub generation: u64,
    pub workspaces: Vec<WorkspaceSummary>,
    pub windows: Vec<WindowSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopRequest {
    GetSnapshot,
    Subscribe,
    FocusWorkspace { workspace: WorkspaceId },
    FocusWindow { window: WindowId },
    CloseWindow { window: WindowId },
    MinimizeWindow { window: WindowId },
    RestoreWindow { window: WindowId },
    ReloadConfiguration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopEvent {
    Snapshot(DesktopSnapshot),
    SnapshotChanged { generation: u64 },
    Ready { api: ProtocolVersion },
    ConfigurationReloaded,
    Error(DesktopError),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopErrorCode {
    InvalidRequest,
    NotFound,
    Unavailable,
    IncompatibleVersion,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopError {
    pub code: DesktopErrorCode,
    pub message: String,
    pub retryable: bool,
}
