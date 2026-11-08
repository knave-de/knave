//! Session environment and desktop-portal activation.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::state::Villain;

const ACTIVATION_KEYS: &[&str] = &[
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_TYPE",
    "XDG_DATA_DIRS",
    "PATH",
    "XDP_KNAVE_SHELL_BINARY",
    "XDP_KNAVE_ENABLED",
];
/// Removes the Wayland socket owned by one Villain process on every exit path.
pub struct SocketCleanup {
    socket: PathBuf,
    lock: PathBuf,
}

impl SocketCleanup {
    pub fn new(socket_name: &OsStr) -> Option<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
        let socket = runtime.join(socket_name);
        let lock = PathBuf::from(format!("{}.lock", socket.display()));
        Some(Self { socket, lock })
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let removed = match fs::symlink_metadata(&self.socket) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                fs::remove_file(&self.socket).is_ok()
            }
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        if removed {
            let _ = fs::remove_file(&self.lock);
        }
    }
}

/// Complete the environment inherited by compositor-launched applications.
pub fn prepare_environment(state: &mut Villain) {
    if std::env::var_os("KNAVE_SESSION").as_deref() == Some(OsStr::new("1")) {
        state
            .config
            .environment
            .insert("XDG_CURRENT_DESKTOP".into(), "Knave:Villain".into());
        state
            .config
            .environment
            .insert("XDG_SESSION_DESKTOP".into(), "knave".into());
        for key in ["PATH", "XDP_KNAVE_SHELL_BINARY", "XDP_KNAVE_ENABLED"] {
            if let Ok(value) = std::env::var(key) {
                state.config.environment.insert(key.into(), value);
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let data = PathBuf::from(home).join(".local/share");
            let current = state
                .config
                .environment
                .get("XDG_DATA_DIRS")
                .cloned()
                .or_else(|| std::env::var("XDG_DATA_DIRS").ok())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            if !current.split(':').any(|entry| Path::new(entry) == data) {
                state.config.environment.insert(
                    "XDG_DATA_DIRS".into(),
                    format!("{}:{current}", data.display()),
                );
            }
        }
    }
    state.config.environment.insert(
        "WAYLAND_DISPLAY".into(),
        state.socket_name.to_string_lossy().into_owned(),
    );
    state.config.environment.remove("WAYLAND_SOCKET");
    if let Some(display) = state.xwayland_display {
        state
            .config
            .environment
            .insert("DISPLAY".into(), format!(":{display}"));
    } else {
        state.config.environment.remove("DISPLAY");
    }

    if let Some(data_dir) = villain_data_dir() {
        let current = state
            .config
            .environment
            .get("XDG_DATA_DIRS")
            .cloned()
            .or_else(|| std::env::var("XDG_DATA_DIRS").ok())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let data_dir = data_dir.to_string_lossy();
        if !current.split(':').any(|entry| entry == data_dir) {
            state
                .config
                .environment
                .insert("XDG_DATA_DIRS".into(), format!("{data_dir}:{current}"));
        }
    }
}

enum ActivationCommand {
    Run {
        environment: BTreeMap<String, String>,
        restart_portal: bool,
    },
    Stop,
}

/// Keeps session activation off the compositor loop with one bounded request
/// slot. The stop flag lets shutdown cancel a command child before joining.
pub(crate) struct ActivationWorker {
    sender: SyncSender<ActivationCommand>,
    receiver: Option<mpsc::Receiver<ActivationCommand>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ActivationWorker {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        Self {
            sender,
            receiver: Some(receiver),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }
        let Some(receiver) = self.receiver.take() else {
            return;
        };
        let worker_stop = Arc::clone(&self.stop);
        self.thread = thread::Builder::new()
            .name("villain-session-activation".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    match command {
                        ActivationCommand::Stop => break,
                        ActivationCommand::Run {
                            environment,
                            restart_portal,
                        } => {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            activate_in_background(environment, restart_portal, &worker_stop);
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|error| {
                tracing::warn!(%error, "could not start session activation worker");
            })
            .ok();
    }

    pub(crate) fn submit(&mut self, environment: BTreeMap<String, String>, restart_portal: bool) {
        self.start();
        match self.sender.try_send(ActivationCommand::Run {
            environment,
            restart_portal,
        }) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                tracing::debug!("session activation queue is full; coalescing request");
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                tracing::warn!("session activation worker is unavailable");
            }
        }
    }
}

impl Drop for ActivationWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.try_send(ActivationCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Publish Villain's environment to D-Bus/systemd activation.
///
/// This is only called for the TTY backend. A nested development compositor
/// must not replace the host desktop's activation environment.
pub fn activate(state: &mut Villain, restart_portal: bool) {
    state
        .activation
        .submit(activation_environment(state), restart_portal);
}

fn activate_in_background(
    environment: BTreeMap<String, String>,
    restart_portal: bool,
    stop: &AtomicBool,
) {
    if !run(
        Command::new("systemctl")
            .args(["--user", "import-environment"])
            .args(
                ACTIVATION_KEYS
                    .iter()
                    .filter(|key| environment.contains_key(**key)),
            )
            .envs(&environment),
        "import the Villain systemd activation environment",
        stop,
    ) {
        return;
    }

    let assignments = ACTIVATION_KEYS
        .iter()
        .filter_map(|key| environment.get(*key).map(|value| format!("{key}={value}")));
    if !run(
        Command::new("dbus-update-activation-environment")
            .arg("--systemd")
            .args(assignments)
            .envs(&environment),
        "import the Villain D-Bus activation environment",
        stop,
    ) {
        return;
    }

    if restart_portal {
        if !run(
            Command::new("systemctl")
                .args([
                    "--user",
                    "try-restart",
                    "--no-block",
                    "xdg-desktop-portal-gtk.service",
                    "xdg-desktop-portal-knave.service",
                ])
                .envs(&environment),
            "restart the GTK portal backend for Villain",
            stop,
        ) {
            return;
        }
        let _ = run(
            Command::new("systemctl")
                .args([
                    "--user",
                    "try-restart",
                    "--no-block",
                    "xdg-desktop-portal.service",
                ])
                .envs(&environment),
            "restart the desktop portal frontend for Villain",
            stop,
        );
    }
}

fn activation_environment(state: &Villain) -> BTreeMap<String, String> {
    let mut environment = state.config.environment.clone();
    if let Some(display) = state.xwayland_display {
        environment.insert("DISPLAY".into(), format!(":{display}"));
    } else {
        // An empty value replaces a stale host DISPLAY in both activation
        // environments until Villain's own XWayland is ready.
        environment.insert("DISPLAY".into(), String::new());
    }
    environment
}

fn run(command: &mut Command, purpose: &str, stop: &AtomicBool) -> bool {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(%purpose, "session integration command is not installed");
            return true;
        }
        Err(error) => {
            tracing::warn!(%error, %purpose, "could not run session integration command");
            return true;
        }
    };

    loop {
        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }

        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                tracing::debug!(%purpose, "session integration succeeded");
                return true;
            }
            Ok(Some(status)) => {
                tracing::warn!(%status, %purpose, "session integration command failed");
                return true;
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                tracing::warn!(%error, %purpose, "could not wait for session integration command");
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

fn villain_data_dir() -> Option<PathBuf> {
    if let Some(path) = option_env!("VILLAIN_DATA_DIR") {
        let path = PathBuf::from(path);
        if has_portal_config(&path) {
            return Some(path);
        }
    }

    if let Ok(executable) = std::env::current_exe()
        && let Some(prefix) = executable.parent().and_then(Path::parent)
    {
        let path = prefix.join("share");
        if has_portal_config(&path) {
            return Some(path);
        }
    }

    if cfg!(debug_assertions) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("share");
        if has_portal_config(&path) {
            return Some(path);
        }
    }
    None
}

fn has_portal_config(path: &Path) -> bool {
    path.join("xdg-desktop-portal/villain-portals.conf")
        .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_portal_configuration_selects_gtk() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("share");
        assert!(has_portal_config(&path));
        let contents =
            std::fs::read_to_string(path.join("xdg-desktop-portal/villain-portals.conf")).unwrap();
        assert!(contents.contains("[preferred]"));
        assert!(contents.contains("default=gtk"));
    }
}
