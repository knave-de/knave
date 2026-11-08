//! Central imperative command interface for compositor state changes.

use std::{fmt, io};

use knave_desktop_api::{DesktopCommand, ResizeEdge, WindowId, WindowTarget};

use crate::{state::Villain, workspace_navigation::Direction};

/// Walk direction for window focus cycling within a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusCycleStep {
    Next,
    Previous,
}

/// Spatial direction for directional focus within a workspace layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dispatch {
    SetNightLight(Option<u16>),
    ToggleFloatingFocused,
    MoveWindow(WindowTarget),
    ResizeWindow(WindowTarget, Option<ResizeEdge>),
    ReloadConfig,
    ResizeMaster(i16),
    ResetMaster,
    CloseFocused,
    MinimizeFocused,
    MaximizeFocused,
    UnmaximizeFocused,
    ToggleMaximizeFocused,
    RestoreLastMinimized,
    FocusWorkspace(usize),
    MoveFocusedToWorkspace(usize),
    PreviousWorkspace,
    LastWorkspace,
    NextWorkspace,
    FocusWindow(WindowId),
    RestoreWindow(WindowId),
    FocusCycle(FocusCycleStep),
    FocusDirection(FocusDirection),
    MoveDirection(FocusDirection),
    Spawn(Vec<String>),
    Quit,
    ToggleOverview,
    SetOverviewVisible(bool),
    Screenshot {
        mode: crate::screenshot::ScreenshotMode,
        clipboard: bool,
    },
    GlobalShortcutActivated {
        app_id: String,
        session_handle: String,
        shortcut_id: String,
    },
    GlobalShortcutDeactivated {
        app_id: String,
        session_handle: String,
        shortcut_id: String,
    },
}

impl From<DesktopCommand> for Dispatch {
    fn from(request: DesktopCommand) -> Self {
        match request {
            DesktopCommand::SetNightLight { temperature } => Self::SetNightLight(temperature),
            DesktopCommand::ToggleFloatingFocused => Self::ToggleFloatingFocused,
            DesktopCommand::MoveWindow { target } => Self::MoveWindow(target),
            DesktopCommand::ResizeWindow { target, edge } => Self::ResizeWindow(target, edge),
            DesktopCommand::ReloadConfiguration => Self::ReloadConfig,
            DesktopCommand::CloseFocused => Self::CloseFocused,
            DesktopCommand::MaximizeFocused => Self::MaximizeFocused,
            DesktopCommand::UnmaximizeFocused => Self::UnmaximizeFocused,
            DesktopCommand::ToggleMaximizeFocused => Self::ToggleMaximizeFocused,
            DesktopCommand::MinimizeFocused => Self::MinimizeFocused,
            DesktopCommand::RestoreLastMinimized => Self::RestoreLastMinimized,
            DesktopCommand::FocusWorkspace { workspace } => {
                Self::FocusWorkspace(workspace.0 as usize)
            }
            DesktopCommand::FocusWindow { window } => Self::FocusWindow(window),
            DesktopCommand::RestoreWindow { window } => Self::RestoreWindow(window),
            DesktopCommand::Spawn { argv } => Self::Spawn(argv),
            DesktopCommand::Quit => Self::Quit,
            DesktopCommand::SetOverviewVisible { visible } => Self::SetOverviewVisible(visible),
        }
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Interaction(&'static str),
    Config(String),
    NoFocusedWindow,
    NoMinimizedWindow,
    InvalidWorkspace(usize),
    UnknownWindow(WindowId),
    MinimizedWindow(WindowId),
    EmptyCommand,
    Spawn(io::Error),
    Screenshot(String),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interaction(message) => formatter.write_str(message),
            Self::Config(error) => write!(formatter, "configuration reload failed: {error}"),
            Self::NoFocusedWindow => write!(formatter, "no window is focused"),
            Self::NoMinimizedWindow => write!(formatter, "no minimized window to restore"),
            Self::InvalidWorkspace(workspace) => {
                write!(formatter, "workspace {workspace} does not exist")
            }
            Self::UnknownWindow(window) => write!(formatter, "window {} does not exist", window.0),
            Self::MinimizedWindow(window) => {
                write!(
                    formatter,
                    "window {} is minimized; restore it first",
                    window.0
                )
            }
            Self::EmptyCommand => write!(formatter, "spawn command is empty"),
            Self::Spawn(error) => write!(formatter, "could not spawn command: {error}"),
            Self::Screenshot(error) => write!(formatter, "screenshot failed: {error}"),
        }
    }
}

impl std::error::Error for DispatchError {}

impl Villain {
    pub fn dispatch(&mut self, dispatch: Dispatch) -> Result<(), DispatchError> {
        match dispatch {
            Dispatch::SetNightLight(temperature) => {
                self.night_light
                    .set(temperature)
                    .map_err(DispatchError::Interaction)?;
                self.request_repaint();
                Ok(())
            }
            Dispatch::ToggleFloatingFocused => self.toggle_floating_focused(),
            Dispatch::MoveWindow(target) => self.begin_window_interaction(target, false, None),
            Dispatch::ResizeWindow(target, edge) => {
                self.begin_window_interaction(target, true, edge)
            }
            Dispatch::ReloadConfig => {
                let config = self
                    .config
                    .reload()
                    .map_err(|error| DispatchError::Config(error.to_string()))?;
                let output_scale_changed = self.config.output_scale != config.output_scale;
                self.cancel_swipe(0);
                self.cancel_workspace_motion();
                self.release_pointer_buttons();
                self.appearance_renderer.clear();
                self.config = config;
                self.trim_dynamic_workspaces();
                if output_scale_changed {
                    self.apply_output_scale(self.config.output_scale);
                } else {
                    self.relayout_active_workspace();
                }
                self.request_repaint();
                crate::session::prepare_environment(self);
                if self.owns_session {
                    crate::session::activate(self, false);
                }
                self.apply_input_config();
                self.publish_reload_event();
                tracing::info!("configuration reloaded");
                Ok(())
            }
            Dispatch::ResizeMaster(delta) => {
                self.resize_master(Some(delta));
                Ok(())
            }
            Dispatch::ResetMaster => {
                self.resize_master(None);
                Ok(())
            }
            Dispatch::CloseFocused => self
                .close_focused_window()
                .then_some(())
                .ok_or(DispatchError::NoFocusedWindow),
            Dispatch::MaximizeFocused => self
                .maximize_focused_window(Some(true))
                .then_some(())
                .ok_or(DispatchError::NoFocusedWindow),
            Dispatch::UnmaximizeFocused => self
                .maximize_focused_window(Some(false))
                .then_some(())
                .ok_or(DispatchError::NoFocusedWindow),
            Dispatch::ToggleMaximizeFocused => self
                .maximize_focused_window(None)
                .then_some(())
                .ok_or(DispatchError::NoFocusedWindow),
            Dispatch::MinimizeFocused => self
                .minimize_focused_window()
                .then_some(())
                .ok_or(DispatchError::NoFocusedWindow),
            Dispatch::RestoreLastMinimized => self
                .restore_last_minimized_window()
                .then_some(())
                .ok_or(DispatchError::NoMinimizedWindow),
            Dispatch::FocusWorkspace(workspace) => {
                if workspace == 0 || workspace > crate::workspace_navigation::MAX_WORKSPACES {
                    return Err(DispatchError::InvalidWorkspace(workspace));
                }
                let index = workspace - 1;
                // Number shortcuts may select fixed slots or the already
                // materialized dynamic tail, but must not skip ahead and
                // manufacture several empty workspaces at once.
                if index < self.workspaces.len() {
                    self.switch_workspace(index);
                }
                Ok(())
            }
            Dispatch::MoveFocusedToWorkspace(workspace) => {
                if workspace == 0 || workspace > crate::workspace_navigation::MAX_WORKSPACES {
                    return Err(DispatchError::InvalidWorkspace(workspace));
                }
                self.move_focused_to_workspace(workspace - 1)
                    .then_some(())
                    .ok_or(DispatchError::NoFocusedWindow)
            }
            Dispatch::PreviousWorkspace => {
                if let Some(target) =
                    self.workspace_target(self.active_workspace, Direction::Previous)
                {
                    self.switch_workspace_direction(target, Direction::Previous);
                }
                Ok(())
            }
            Dispatch::LastWorkspace => {
                if let Some(target) = self.last_workspace {
                    self.switch_workspace(target);
                }
                Ok(())
            }
            Dispatch::NextWorkspace => {
                if let Some(target) = self.workspace_target(self.active_workspace, Direction::Next)
                {
                    self.switch_workspace_direction(target, Direction::Next);
                }
                Ok(())
            }
            Dispatch::FocusCycle(step) => {
                self.cycle_focus(step);
                Ok(())
            }
            Dispatch::MoveDirection(direction) => {
                self.move_focused_directional(direction);
                Ok(())
            }
            Dispatch::FocusDirection(direction) => {
                self.focus_directional(direction);
                Ok(())
            }
            Dispatch::FocusWindow(window) => match self.focus_window(window) {
                Some(true) => Ok(()),
                Some(false) => Err(DispatchError::MinimizedWindow(window)),
                None => Err(DispatchError::UnknownWindow(window)),
            },
            Dispatch::RestoreWindow(window) => self
                .restore_window(window)
                .then_some(())
                .ok_or(DispatchError::UnknownWindow(window)),
            Dispatch::Spawn(argv) => {
                if argv.is_empty() {
                    Err(DispatchError::EmptyCommand)
                } else {
                    self.spawn(argv).map_err(DispatchError::Spawn)
                }
            }
            Dispatch::Quit => {
                self.loop_signal.stop();
                Ok(())
            }
            Dispatch::ToggleOverview => {
                self.set_overview_visible(!self.overview_visible);
                Ok(())
            }
            Dispatch::SetOverviewVisible(visible) => {
                self.set_overview_visible(visible);
                Ok(())
            }
            Dispatch::Screenshot { mode, clipboard } => {
                crate::screenshot::begin(self, mode, clipboard).map_err(DispatchError::Screenshot)
            }
            Dispatch::GlobalShortcutActivated {
                app_id,
                session_handle,
                shortcut_id,
            } => {
                self.ipc_server.publish_event(
                    knave_desktop_api::DesktopEvent::GlobalShortcutActivated {
                        app_id,
                        session_handle,
                        shortcut_id,
                    },
                );
                Ok(())
            }
            Dispatch::GlobalShortcutDeactivated {
                app_id,
                session_handle,
                shortcut_id,
            } => {
                self.ipc_server.publish_event(
                    knave_desktop_api::DesktopEvent::GlobalShortcutDeactivated {
                        app_id,
                        session_handle,
                        shortcut_id,
                    },
                );
                Ok(())
            }
        }
    }

    fn set_overview_visible(&mut self, visible: bool) {
        if self.overview_visible != visible {
            self.cancel_swipe(0);
            self.cancel_workspace_motion();
            self.release_pointer_buttons();
            if !visible {
                for entry in &mut self.shell_surfaces {
                    if entry.mapped && entry.layer.namespace() == "knave-shell-overview" {
                        entry.hide_pending = true;
                    }
                }
            }
            self.overview_visible = visible;
            self.desktop_state_dirty = true;
            self.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_dispatch_maps_to_internal_dispatch() {
        assert_eq!(
            Dispatch::from(DesktopCommand::FocusWorkspace {
                workspace: knave_desktop_api::WorkspaceId(2),
            }),
            Dispatch::FocusWorkspace(2)
        );
        assert_eq!(
            Dispatch::from(DesktopCommand::MinimizeFocused),
            Dispatch::MinimizeFocused
        );
    }
}
