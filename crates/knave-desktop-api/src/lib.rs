//! Desktop contracts and transport client for the Knave desktop environment.

use std::{
    env,
    ffi::OsStr,
    io::{self, BufRead, BufReader, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SOCKET_ENVIRONMENT_VARIABLE: &str = "KNAVE_SOCKET";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WorkspaceId(pub u32);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceSummary {
    pub workspace: WorkspaceId,
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
    /// Saved maximization state; fullscreen takes precedence while active.
    pub maximized: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspacePreview {
    pub workspace: WorkspaceId,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

/// A logical-output rectangle where the compositor displays one workspace.
/// Window surfaces are visual only; the shell keeps all pointer and keyboard input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverviewPane {
    pub workspace: WorkspaceId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopSnapshot {
    pub generation: u64,
    /// Whether the session-owned Overview layer should currently be mapped.
    pub overview_visible: bool,
    pub workspaces: Vec<WorkspaceSummary>,
    pub windows: Vec<WindowSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum DesktopCommand {
    ReloadConfiguration,
    CloseFocused,
    MinimizeFocused,
    MaximizeFocused,
    UnmaximizeFocused,
    ToggleMaximizeFocused,
    RestoreLastMinimized,
    FocusWorkspace { workspace: WorkspaceId },
    FocusWindow { window: WindowId },
    RestoreWindow { window: WindowId },
    Spawn { argv: Vec<String> },
    Quit,
    SetOverviewVisible { visible: bool },
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
    /// Converts a dedicated connection to a stream of replacement snapshots.
    Subscribe,
    /// Replace the compositor overview panes; an empty list clears them.
    SetOverviewPanes {
        panes: Vec<OverviewPane>,
    },
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
    Version { component: String },
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
    #[error("unexpected desktop subscription response")]
    UnexpectedResponse,
    #[error("desktop snapshot exceeds the subscription frame limit")]
    FrameTooLarge,
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

/// Maximum UTF-8 bytes in one subscription snapshot, including its newline.
pub const MAX_SNAPSHOT_FRAME_BYTES: u64 = 16 * 1024 * 1024;

/// Dropping the stream disconnects; this handle also interrupts a blocked reader.
pub struct SubscriptionCancel(UnixStream);
impl SubscriptionCancel {
    pub fn cancel(&self) -> io::Result<()> {
        self.0.shutdown(Shutdown::Both)
    }
}

/// Snapshot stream. The first frame is a complete initial state;
/// subsequent frames replace it. Intermediate states may be coalesced.
/// Use a separate DesktopClient for commands and preview requests.
pub struct DesktopSubscription {
    reader: BufReader<UnixStream>,
    initial: Option<DesktopSnapshot>,
    poisoned: bool,
}
impl DesktopSubscription {
    pub fn connect() -> Result<Self, ClientError> {
        let mut stream = UnixStream::connect(socket_path()?)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        serde_json::to_writer(&mut stream, &DesktopRequest::Subscribe)?;
        stream.write_all(b"\n")?;
        let mut subscription = Self {
            reader: BufReader::new(stream),
            initial: None,
            poisoned: false,
        };
        subscription.initial = Some(subscription.read_snapshot()?);
        // A healthy idle desktop sends nothing. Cancellation shuts down this socket.
        subscription.reader.get_ref().set_read_timeout(None)?;
        Ok(subscription)
    }

    pub fn cancel_handle(&self) -> Result<SubscriptionCancel, ClientError> {
        Ok(SubscriptionCancel(self.reader.get_ref().try_clone()?))
    }

    pub fn next_snapshot(&mut self) -> Result<DesktopSnapshot, ClientError> {
        match self.initial.take() {
            Some(snapshot) => Ok(snapshot),
            None => self.read_snapshot(),
        }
    }

    fn read_snapshot(&mut self) -> Result<DesktopSnapshot, ClientError> {
        use std::io::Read;
        if self.poisoned {
            return Err(ClientError::FrameTooLarge);
        }
        let mut line = String::new();
        let bytes = self
            .reader
            .by_ref()
            .take(MAX_SNAPSHOT_FRAME_BYTES + 1)
            .read_line(&mut line)?;
        if bytes == 0 {
            return Err(ClientError::Disconnected);
        }
        if bytes as u64 > MAX_SNAPSHOT_FRAME_BYTES {
            // The unread tail still belongs to this frame; it cannot be parsed safely.
            self.poisoned = true;
            let _ = self.reader.get_ref().shutdown(Shutdown::Both);
            return Err(ClientError::FrameTooLarge);
        }
        if !line.ends_with('\n') {
            return Err(ClientError::Disconnected);
        }
        match serde_json::from_str(&line)? {
            DesktopResponse::Snapshot(snapshot) => Ok(snapshot),
            DesktopResponse::Error(error) => Err(ClientError::Remote(error)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_one_json_line() {
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
    fn subscription_request_has_no_api_version() {
        assert_eq!(
            serde_json::to_string(&DesktopRequest::Subscribe).unwrap(),
            r#"{"type":"subscribe"}"#
        );
    }

    #[test]
    fn version_response_reports_only_the_component_package() {
        let response = DesktopResponse::Version {
            component: "villain 0.1.0".into(),
        };
        let encoded = serde_json::to_string(&response).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"version","payload":{"component":"villain 0.1.0"}}"#
        );
        assert!(!encoded.contains("protocol"));
    }

    #[test]
    fn maximize_commands_have_distinct_wire_actions() {
        for (command, action) in [
            (DesktopCommand::MaximizeFocused, "maximize-focused"),
            (DesktopCommand::UnmaximizeFocused, "unmaximize-focused"),
            (
                DesktopCommand::ToggleMaximizeFocused,
                "toggle-maximize-focused",
            ),
        ] {
            let value = serde_json::to_value(&command).unwrap();
            assert_eq!(value["action"], action);
            assert_eq!(
                serde_json::from_value::<DesktopCommand>(value).unwrap(),
                command
            );
        }
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
