//! Owns Knave compositor and shell process lifecycle.

use std::{
    collections::HashSet,
    env,
    ffi::OsString,
    fs, io,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use knave_config::{Config, ConfigDocument, SessionBackend};
use thiserror::Error;

const SOCKET_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const MAX_RESTARTS: u32 = 3;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("could not locate XDG_RUNTIME_DIR")]
    MissingRuntimeDirectory,
    #[error("could not read runtime directory {path}: {source}")]
    RuntimeDirectory { path: PathBuf, source: io::Error },
    #[error("could not spawn {component}: {source}")]
    Spawn {
        component: String,
        source: io::Error,
    },
    #[error("{component} exited before the session was ready: {status}")]
    EarlyExit {
        component: String,
        status: ExitStatus,
    },
    #[error("timed out waiting for a new Wayland socket in {0}")]
    SocketTimeout(PathBuf),
    #[error("could not inspect {path}: {source}")]
    SocketMetadata { path: PathBuf, source: io::Error },
    #[error("session process failed: {0}")]
    Process(String),
}

/// Load the canonical configuration and run the desktop session.
pub fn run_default() -> Result<(), SessionError> {
    let document = ConfigDocument::at_default_path()
        .map_err(|error| SessionError::Process(error.to_string()))?;
    run(document.config().clone())
}

/// Start, supervise, and cleanly stop the compositor and shell processes.
pub fn run(config: Config) -> Result<(), SessionError> {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    install_signal_handlers();

    let runtime = runtime_directory()?;
    let mut restarts = 0;
    let mut before = wayland_sockets(&runtime)?;
    let (mut compositor, mut display, mut shells) = start_components(&config, &runtime, &before)?;

    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            terminate_children(&mut shells);
            terminate_child(&mut compositor);
            return Ok(());
        }

        if let Some(status) = compositor
            .try_wait()
            .map_err(|error| SessionError::Process(format!("villain wait failed: {error}")))?
        {
            terminate_children(&mut shells);
            if !config.session.restart_on_failure || restarts >= MAX_RESTARTS {
                return Err(SessionError::EarlyExit {
                    component: "villain".into(),
                    status,
                });
            }
            restarts += 1;
            thread::sleep(restart_delay(restarts));
            before = wayland_sockets(&runtime)?;
            let (new_compositor, new_display, new_shells) =
                start_components(&config, &runtime, &before)?;
            compositor = new_compositor;
            display = new_display;
            shells = new_shells;
        }

        let mut shell_exit = None;
        for managed in &mut shells {
            if let Some(status) = managed
                .child
                .try_wait()
                .map_err(|error| SessionError::Process(format!("shell wait failed: {error}")))?
            {
                shell_exit = Some((managed.role, status));
                break;
            }
        }
        if let Some((role, status)) = shell_exit {
            terminate_children(&mut shells);
            if !config.session.restart_on_failure || restarts >= MAX_RESTARTS {
                terminate_child(&mut compositor);
                return Err(SessionError::EarlyExit {
                    component: format!("knave-shell {role}"),
                    status,
                });
            }
            restarts += 1;
            thread::sleep(restart_delay(restarts));
            shells = match spawn_shells(&config, &display) {
                Ok(shells) => shells,
                Err(error) => {
                    terminate_child(&mut compositor);
                    return Err(error);
                }
            };
        }

        thread::sleep(POLL_INTERVAL);
    }
}

struct ManagedChild {
    role: &'static str,
    child: Child,
}

fn start_components(
    config: &Config,
    runtime: &Path,
    before: &HashSet<OsString>,
) -> Result<(Child, String, Vec<ManagedChild>), SessionError> {
    let mut compositor = spawn_compositor(config)?;
    let display = match wait_for_wayland_socket(runtime, before, &mut compositor) {
        Ok(display) => display,
        Err(error) => {
            terminate_child(&mut compositor);
            return Err(error);
        }
    };
    let shells = match spawn_shells(config, &display) {
        Ok(shells) => shells,
        Err(error) => {
            terminate_child(&mut compositor);
            return Err(error);
        }
    };
    Ok((compositor, display, shells))
}

fn spawn_compositor(config: &Config) -> Result<Child, SessionError> {
    let mut command = Command::new(&config.session.compositor_binary);
    match config.session.backend {
        SessionBackend::Auto => {}
        SessionBackend::Tty => {
            command.arg("--tty");
        }
        SessionBackend::Winit => {
            command.arg("--winit");
        }
    }
    command
        .env("KNAVE_SESSION", "1")
        .spawn()
        .map_err(|source| SessionError::Spawn {
            component: "villain".into(),
            source,
        })
}

fn spawn_shells(config: &Config, display: &str) -> Result<Vec<ManagedChild>, SessionError> {
    let mut shells = Vec::new();
    if config.shell.start_bar {
        match spawn_shell(config, "bar", display) {
            Ok(shell) => shells.push(shell),
            Err(error) => return Err(cleanup_shell_startup(shells, error)),
        }
    }
    if config.shell.start_overview_service {
        match spawn_shell(config, "overview", display) {
            Ok(shell) => shells.push(shell),
            Err(error) => return Err(cleanup_shell_startup(shells, error)),
        }
    }
    Ok(shells)
}

fn cleanup_shell_startup(mut shells: Vec<ManagedChild>, error: SessionError) -> SessionError {
    terminate_children(&mut shells);
    error
}

fn spawn_shell(
    config: &Config,
    role: &'static str,
    display: &str,
) -> Result<ManagedChild, SessionError> {
    let child = Command::new(&config.session.shell_binary)
        .arg(role)
        .env("WAYLAND_DISPLAY", display)
        .env("KNAVE_SESSION", "1")
        .spawn()
        .map_err(|source| SessionError::Spawn {
            component: format!("knave-shell {role}"),
            source,
        })?;
    Ok(ManagedChild { role, child })
}

fn runtime_directory() -> Result<PathBuf, SessionError> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or(SessionError::MissingRuntimeDirectory)
}

fn wayland_sockets(runtime: &Path) -> Result<HashSet<OsString>, SessionError> {
    let entries = fs::read_dir(runtime).map_err(|source| SessionError::RuntimeDirectory {
        path: runtime.to_path_buf(),
        source,
    })?;
    let mut sockets = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(|source| SessionError::RuntimeDirectory {
            path: runtime.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("wayland-") {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|source| SessionError::SocketMetadata { path, source })?;
        if metadata.file_type().is_socket() {
            sockets.insert(name);
        }
    }
    Ok(sockets)
}

fn wait_for_wayland_socket(
    runtime: &Path,
    before: &HashSet<OsString>,
    compositor: &mut Child,
) -> Result<String, SessionError> {
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if let Some(status) = compositor
            .try_wait()
            .map_err(|error| SessionError::Process(format!("villain wait failed: {error}")))?
        {
            return Err(SessionError::EarlyExit {
                component: "villain".into(),
                status,
            });
        }

        let current = wayland_sockets(runtime)?;
        let mut candidates = current.difference(before).cloned().collect::<Vec<_>>();
        candidates.sort();
        if let Some(display) = candidates.into_iter().next() {
            return Ok(display.to_string_lossy().into_owned());
        }
        if Instant::now() >= deadline {
            return Err(SessionError::SocketTimeout(runtime.to_path_buf()));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn terminate_children(children: &mut Vec<ManagedChild>) {
    for managed in children.iter_mut() {
        terminate_child(&mut managed.child);
    }
    children.clear();
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    }

    let deadline = Instant::now() + SHUTDOWN_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) | Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn restart_delay(count: u32) -> Duration {
    Duration::from_millis(100 * (1_u64 << count.saturating_sub(1).min(4)))
}

extern "C" fn request_shutdown(_: i32) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    #[cfg(unix)]
    unsafe {
        libc::signal(
            libc::SIGINT,
            request_shutdown as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            request_shutdown as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn restart_delay_is_bounded_exponential_backoff() {
        assert_eq!(restart_delay(1), Duration::from_millis(100));
        assert_eq!(restart_delay(3), Duration::from_millis(400));
        assert_eq!(restart_delay(99), Duration::from_millis(1_600));
    }

    #[test]
    fn only_wayland_sockets_are_discovered() {
        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("wayland-test");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        fs::write(directory.path().join("wayland-not-a-socket"), "x").unwrap();
        fs::write(directory.path().join("unrelated"), "x").unwrap();

        let sockets = wayland_sockets(directory.path()).unwrap();

        assert!(sockets.contains(&OsString::from("wayland-test")));
        assert!(!sockets.contains(&OsString::from("wayland-not-a-socket")));
        assert!(!sockets.contains(&OsString::from("unrelated")));
    }
}
