//! Versioned contracts and transport client for the Knave desktop environment.

use std::{
    env,
    ffi::OsStr,
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const API_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };
pub const SOCKET_ENVIRONMENT_VARIABLE: &str = "KNAVE_SOCKET";

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
    #[serde(default)]
    pub floating: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspacePreview {
    pub workspace: WorkspaceId,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopSnapshot {
    pub generation: u64,
    pub workspaces: Vec<WorkspaceSummary>,
    pub windows: Vec<WindowSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum DesktopCommand {
    ReloadConfiguration,
    CloseFocused,
    MinimizeFocused,
    RestoreLastMinimized,
    FocusWorkspace { workspace: WorkspaceId },
    FocusWindow { window: WindowId },
    RestoreWindow { window: WindowId },
    Spawn { argv: Vec<String> },
    Quit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "query", rename_all = "kebab-case")]
pub enum DesktopQuery {
    Snapshot,
    Windows,
    Workspaces,
    ActiveWindow,
    ActiveWorkspace,
    WorkspacePreview {
        workspace: WorkspaceId,
        width: u32,
        height: u32,
    },
    Version,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DesktopRequest {
    Dispatch(DesktopCommand),
    Query(DesktopQuery),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DesktopResponse {
    Ok,
    Snapshot(DesktopSnapshot),
    Windows(Vec<WindowSummary>),
    Workspaces(Vec<WorkspaceSummary>),
    ActiveWindow(Option<WindowSummary>),
    ActiveWorkspace(WorkspaceId),
    WorkspacePreview(WorkspacePreview),
    Version {
        protocol: ProtocolVersion,
        component: String,
    },
    Error(DesktopError),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum DesktopEvent {
    SnapshotChanged { generation: u64 },
    WindowOpened(WindowSummary),
    WindowClosed(WindowId),
    WindowFocused(Option<WindowId>),
    WindowMinimized(WindowId),
    WindowRestored(WindowId),
    WorkspaceChanged(WorkspaceId),
    ConfigurationReloaded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopError {
    pub code: DesktopErrorCode,
    pub message: String,
    pub retryable: bool,
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

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0} is not set")]
    MissingEnvironment(&'static str),
    #[error("desktop IPC I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("desktop IPC JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Knave closed the IPC connection")]
    Disconnected,
    #[error("Knave returned an error: {0:?}")]
    Remote(DesktopError),
}

pub fn socket_path() -> Result<PathBuf, ClientError> {
    if let Some(path) = env::var_os(SOCKET_ENVIRONMENT_VARIABLE) {
        return Ok(path.into());
    }
    let runtime =
        env::var_os("XDG_RUNTIME_DIR").ok_or(ClientError::MissingEnvironment("XDG_RUNTIME_DIR"))?;
    let display =
        env::var_os("WAYLAND_DISPLAY").ok_or(ClientError::MissingEnvironment("WAYLAND_DISPLAY"))?;
    Ok(socket_path_for_display(runtime.as_ref(), display.as_ref()))
}

pub fn socket_path_for_display(runtime: &OsStr, display: &OsStr) -> PathBuf {
    let display = display.to_string_lossy().replace(['/', '\\'], "_");
    PathBuf::from(runtime)
        .join("knave")
        .join(format!("desktop-{display}.sock"))
}

pub struct DesktopClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl DesktopClient {
    pub fn connect() -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket_path()?)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    pub fn request(&mut self, request: &DesktopRequest) -> Result<DesktopResponse, ClientError> {
        serde_json::to_writer(&mut self.writer, request)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;

        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(ClientError::Disconnected);
        }
        let response: DesktopResponse = serde_json::from_str(&line)?;
        match response {
            DesktopResponse::Error(error) => Err(ClientError::Remote(error)),
            response => Ok(response),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_one_versioned_json_line() {
        let request = DesktopRequest::Dispatch(DesktopCommand::FocusWorkspace {
            workspace: WorkspaceId(2),
        });
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"dispatch","payload":{"action":"focus-workspace","workspace":2}}"#
        );
        assert_eq!(
            serde_json::from_str::<DesktopRequest>(&encoded).unwrap(),
            request
        );
    }

    #[test]
    fn socket_is_scoped_to_knave_and_wayland_display() {
        assert_eq!(
            socket_path_for_display(OsStr::new("/run/user/1000"), OsStr::new("wayland-2")),
            PathBuf::from("/run/user/1000/knave/desktop-wayland-2.sock")
        );
    }

    #[test]
    fn display_name_cannot_escape_runtime_directory() {
        assert_eq!(
            socket_path_for_display(OsStr::new("/tmp/runtime"), OsStr::new("../wayland-2")),
            PathBuf::from("/tmp/runtime/knave/desktop-.._wayland-2.sock")
        );
    }
}
