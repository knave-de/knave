//! Owns Knave compositor and shell process lifecycle.

pub mod notifications;
pub mod quick_settings;

use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use knave_config::{Config, ConfigDocument, SessionBackend, StartupCommand};
use knave_desktop_api::{
    DesktopClient, DesktopQuery, DesktopRequest, DesktopResponse, XWaylandStatus,
    socket_path_for_display,
};
use thiserror::Error;

use crate::wallpaper_assignment::AssignmentSlot;

mod wallpaper_assignment;

const SOCKET_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const MAX_RESTARTS: u32 = 3;
const CAPABILITY_WAIT: Duration = Duration::from_secs(2);
const XWAYLAND_WAIT: Duration = Duration::from_secs(5);

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("{0} does not support Overview service mode; install a matching Knave Shell")]
    UnsupportedOverviewService(String),
    #[error("{0} does not support notification service mode; install a matching Knave Shell")]
    UnsupportedNotificationService(String),
    #[error("{0} does not support Polkit agent mode; install a matching Knave Shell")]
    UnsupportedPolkitAgent(String),
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
    let _notifications = if config.notifications.enabled {
        match notifications::NotificationService::start(config.clone()) {
            Ok(service) => Some(service),
            Err(error) => {
                eprintln!("knave-session: notifications unavailable: {error}");
                None
            }
        }
    } else {
        None
    };

    let runtime = runtime_directory()?;
    let mut restarts = 0;
    let mut before = wayland_sockets(&runtime)?;
    let slot = Arc::new(AssignmentSlot::new());
    let (mut compositor, mut display, mut shells) = start_components(&config, &runtime, &before)?;
    if let Err(error) = slot.reset(&config, &display) {
        terminate_children(&mut shells);
        terminate_child(&mut compositor);
        return Err(error);
    }
    slot.retarget(&display);
    let _assignment = wallpaper_assignment::thread(config.clone(), runtime.clone(), slot.clone());
    let xwayland_display = if config.session.startup.is_empty() {
        None
    } else {
        wait_for_xwayland_display(&runtime, &display)
    };
    let mut startup_commands = start_startup_commands(
        &config.session.startup,
        &display,
        xwayland_display.as_deref(),
    );

    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            slot.shutdown();
            terminate_startup_commands(&mut startup_commands);
            terminate_children(&mut shells);
            terminate_child(&mut compositor);
            return Ok(());
        }

        if let Some(status) = compositor
            .try_wait()
            .map_err(|error| SessionError::Process(format!("villain wait failed: {error}")))?
        {
            // A clean compositor exit is the session's logout path, including
            // `knavectl dispatch quit`; only failures consume the restart budget.
            if status.success() {
                slot.shutdown();
                terminate_startup_commands(&mut startup_commands);
                terminate_children(&mut shells);
                return Ok(());
            }
            // The wallpaper surface belongs to the dead compositor; hand it
            // back to the reaper before any restart attempt.
            slot.disown();
            terminate_startup_commands(&mut startup_commands);
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
            // The assignment subscription follows the possibly new display.
            slot.retarget(&display);
            if let Err(error) = slot.reset(&config, &display) {
                terminate_startup_commands(&mut startup_commands);
                terminate_children(&mut shells);
                terminate_child(&mut compositor);
                return Err(error);
            }
            let xwayland_display = if config.session.startup.is_empty() {
                None
            } else {
                wait_for_xwayland_display(&runtime, &display)
            };
            startup_commands = start_startup_commands(
                &config.session.startup,
                &display,
                xwayland_display.as_deref(),
            );
        }

        reap_startup_commands(&mut startup_commands);

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
        if shell_exit.is_none() {
            shell_exit = slot.exited()?.map(|status| ("wallpaper", status));
        }
        if let Some((role, status)) = shell_exit {
            terminate_children(&mut shells);
            if !config.session.restart_on_failure || restarts >= MAX_RESTARTS {
                terminate_startup_commands(&mut startup_commands);
                terminate_child(&mut compositor);
                return Err(SessionError::EarlyExit {
                    component: format!("knave-shell {role}"),
                    status,
                });
            }
            restarts += 1;
            thread::sleep(restart_delay(restarts));
            // Shells may observe Wayland disconnection before the compositor
            // exits. Let its exit status decide logout versus recovery.
            if compositor
                .try_wait()
                .map_err(|error| SessionError::Process(format!("villain wait failed: {error}")))?
                .is_some()
            {
                continue;
            }
            shells = match spawn_shells(&config, &display) {
                Ok(shells) => shells,
                Err(error) => {
                    terminate_startup_commands(&mut startup_commands);
                    terminate_child(&mut compositor);
                    return Err(error);
                }
            };
            if let Err(error) = slot.reset(&config, &display) {
                terminate_children(&mut shells);
                terminate_child(&mut compositor);
                return Err(error);
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

pub(crate) struct ManagedChild {
    pub(crate) role: &'static str,
    pub(crate) child: Child,
}

fn wait_for_xwayland_display(runtime: &Path, wayland_display: &str) -> Option<String> {
    let deadline = Instant::now() + XWAYLAND_WAIT;
    let socket = socket_path_for_display(runtime.as_os_str(), OsStr::new(wayland_display));
    loop {
        if let Ok(mut client) = DesktopClient::connect_to_path(&socket) {
            match client.request(&DesktopRequest::Query(DesktopQuery::XWaylandStatus)) {
                Ok(DesktopResponse::XWaylandStatus(XWaylandStatus::Available { display })) => {
                    return Some(display);
                }
                Ok(DesktopResponse::XWaylandStatus(XWaylandStatus::Unavailable)) => return None,
                Ok(DesktopResponse::XWaylandStatus(XWaylandStatus::Starting)) => {}
                Err(_) if Instant::now() < deadline => {}
                Ok(response) => {
                    eprintln!("knave-session: unexpected XWayland status response: {response:?}");
                    return None;
                }
                Err(error) => {
                    eprintln!("knave-session: XWayland status query failed: {error}");
                    return None;
                }
            }
        }
        if Instant::now() >= deadline {
            eprintln!(
                "knave-session: timed out waiting for XWayland; startup commands will use Wayland only"
            );
            return None;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn start_startup_commands(
    commands: &[StartupCommand],
    display: &str,
    xwayland_display: Option<&str>,
) -> Vec<Child> {
    let mut children = Vec::with_capacity(commands.len());
    for (index, startup) in commands.iter().enumerate() {
        let mut command = Command::new(&startup.command[0]);
        command
            .args(&startup.command[1..])
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("KNAVE_SOCKET")
            .env("WAYLAND_DISPLAY", display)
            .env("KNAVE_SESSION", "1")
            .env("XDG_CURRENT_DESKTOP", "Knave:Villain")
            .env("XDG_SESSION_DESKTOP", "knave");
        if let Some(xwayland_display) = xwayland_display {
            command.env("DISPLAY", xwayland_display);
        }
        match spawn_with_brief_retry(command) {
            Ok(child) => children.push(child),
            Err(error) => {
                eprintln!("knave-session: startup command {index} could not be started: {error}")
            }
        }
    }
    children
}

fn reap_startup_commands(children: &mut Vec<Child>) {
    let mut index = 0;
    while index < children.len() {
        match children[index].try_wait() {
            Ok(Some(_)) => {
                let mut child = children.swap_remove(index);
                let _ = child.wait();
            }
            Ok(None) => index += 1,
            Err(error) => {
                eprintln!("knave-session: startup process wait failed: {error}");
                let mut child = children.swap_remove(index);
                terminate_child(&mut child);
            }
        }
    }
}

fn terminate_startup_commands(children: &mut Vec<Child>) {
    for child in children.iter_mut() {
        terminate_child(child);
    }
    children.clear();
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

/// A just-replaced binary can still be pinned briefly (ETXTBSY) by the
/// previous instance's dying descriptors; wait that out before failing.
fn spawn_with_brief_retry(mut command: Command) -> io::Result<Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempt < 10 => {
                attempt += 1;
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
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
        .env("XDG_CURRENT_DESKTOP", "Knave:Villain")
        .env("XDG_SESSION_DESKTOP", "knave")
        .env(
            "XDP_KNAVE_ENABLED",
            if config.portal.enabled { "1" } else { "0" },
        )
        .env(
            "XDP_KNAVE_SHELL_BINARY",
            resolved_binary(&config.session.shell_binary),
        );
    spawn_with_brief_retry(command).map_err(|source| SessionError::Spawn {
        component: "villain".into(),
        source,
    })
}

fn resolved_binary(binary: &str) -> String {
    if Path::new(binary).components().count() > 1 {
        return fs::canonicalize(binary).map_or_else(
            |_| binary.into(),
            |path| path.to_string_lossy().into_owned(),
        );
    }
    env::var_os("PATH")
        .and_then(|path| {
            env::split_paths(&path)
                .map(|directory| directory.join(binary))
                .find(|path| path.is_file())
        })
        .map_or_else(|| binary.into(), |path| path.to_string_lossy().into_owned())
}

fn spawn_shells(config: &Config, display: &str) -> Result<Vec<ManagedChild>, SessionError> {
    if config.shell.start_overview_service {
        check_overview_service(&config.session.shell_binary, CAPABILITY_WAIT)?;
    }
    if config.notifications.enabled {
        check_notification_service(&config.session.shell_binary, CAPABILITY_WAIT)?;
    }
    let inherited_display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|key| env::var_os(key).is_some_and(|value| !value.is_empty()));
    let start_polkit = polkit_session(&config.session.backend, inherited_display);
    if start_polkit
        && !check_capability(
            &config.session.shell_binary,
            "--supports-polkit-agent",
            CAPABILITY_WAIT,
        )?
    {
        return Err(SessionError::UnsupportedPolkitAgent(
            config.session.shell_binary.clone(),
        ));
    }
    let mut shells = Vec::new();
    if start_polkit {
        match spawn_shell(config, "polkit-agent", display) {
            Ok(shell) => shells.push(shell),
            Err(error) => return Err(cleanup_shell_startup(shells, error)),
        }
    }
    if config.shell.start_bar || config.shell.start_overview_service {
        for role in ["quick-settings-service", "quick-settings"] {
            match spawn_shell(config, role, display) {
                Ok(shell) => shells.push(shell),
                Err(error) => return Err(cleanup_shell_startup(shells, error)),
            }
        }
    }
    if config.notifications.enabled {
        match spawn_shell(config, "notifications", display) {
            Ok(shell) => shells.push(shell),
            Err(error) => return Err(cleanup_shell_startup(shells, error)),
        }
    }
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
    // The wallpaper child lives in the assignment slot; the assignment
    // thread and reload events manage it after startup.
    Ok(shells)
}

// Nested desktops must not register an agent for their enclosing login session.
fn polkit_session(backend: &SessionBackend, inherited_display: bool) -> bool {
    match backend {
        SessionBackend::Tty => true,
        SessionBackend::Winit => false,
        SessionBackend::Auto => !inherited_display,
    }
}

fn check_overview_service(binary: &str, timeout: Duration) -> Result<(), SessionError> {
    if check_capability(binary, "--supports-overview-service", timeout)? {
        Ok(())
    } else {
        Err(SessionError::UnsupportedOverviewService(binary.into()))
    }
}
fn check_notification_service(binary: &str, timeout: Duration) -> Result<(), SessionError> {
    if check_capability(binary, "--supports-notifications-service", timeout)? {
        Ok(())
    } else {
        Err(SessionError::UnsupportedNotificationService(binary.into()))
    }
}
fn check_capability(binary: &str, flag: &str, timeout: Duration) -> Result<bool, SessionError> {
    let mut command = Command::new(binary);
    command
        .arg(flag)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = spawn_with_brief_retry(command).map_err(|source| SessionError::Spawn {
        component: "knave-shell capability check".into(),
        source,
    })?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(true),
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SessionError::Process(format!(
                    "shell capability wait failed: {error}"
                )));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    Ok(false)
}

fn cleanup_shell_startup(mut shells: Vec<ManagedChild>, error: SessionError) -> SessionError {
    terminate_children(&mut shells);
    error
}

pub(crate) fn spawn_shell(
    config: &Config,
    role: &'static str,
    display: &str,
) -> Result<ManagedChild, SessionError> {
    let mut command = if role == "quick-settings-service" {
        Command::new(
            env::current_exe()
                .map_err(|e| SessionError::Process(e.to_string()))?
                .with_file_name("knave-session"),
        )
    } else {
        Command::new(&config.session.shell_binary)
    };
    command
        .arg(role)
        .env("WAYLAND_DISPLAY", display)
        .env("KNAVE_SESSION", "1")
        .env(
            "KNAVE_NOTIFICATIONS_ENABLED",
            if config.notifications.enabled {
                "1"
            } else {
                "0"
            },
        )
        .env("XDG_CURRENT_DESKTOP", "Knave:Villain")
        .env("XDG_SESSION_DESKTOP", "knave");
    if role == "wallpaper" {
        command
            .arg(
                config
                    .shell
                    .wallpaper
                    .image
                    .as_deref()
                    .expect("configured wallpaper"),
            )
            .arg(config.shell.wallpaper.fit.as_str());
    }
    if role == "overview" {
        command.env("KNAVE_OVERVIEW_SERVICE", "1");
    }
    let child = spawn_with_brief_retry(command).map_err(|source| SessionError::Spawn {
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

pub(crate) fn terminate_child(child: &mut Child) {
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
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    #[test]
    fn nested_sessions_cannot_claim_the_host_polkit_agent() {
        assert!(!polkit_session(&SessionBackend::Winit, false));
        assert!(!polkit_session(&SessionBackend::Winit, true));
        assert!(!polkit_session(&SessionBackend::Auto, true));
        assert!(polkit_session(&SessionBackend::Auto, false));
        assert!(polkit_session(&SessionBackend::Tty, true));
        assert!(polkit_session(&SessionBackend::Tty, false));
    }

    #[test]
    fn overview_capability_rejects_old_and_hung_shells_before_starting_roles() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("shell");
        for (script, accepted) in [
            (
                "#!/bin/sh\n[ \"$1\" = --supports-overview-service ]\n",
                true,
            ),
            ("#!/bin/sh\nexit 1\n", false),
            ("#!/bin/sh\nexec sleep 10\n", false),
        ] {
            fs::write(&binary, script).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let started = Instant::now();
            assert_eq!(
                check_overview_service(binary.to_str().unwrap(), Duration::from_millis(100))
                    .is_ok(),
                accepted
            );
            assert!(started.elapsed() < Duration::from_secs(1));
        }
        fs::write(&binary, "#!/bin/sh\nexit 1\n").unwrap();
        let mut config = Config::default();
        config.session.shell_binary = binary.to_str().unwrap().into();
        assert!(matches!(
            spawn_shells(&config, "unused"),
            Err(SessionError::UnsupportedOverviewService(_))
        ));
        config.session.backend = SessionBackend::Tty;
        config.shell.start_overview_service = false;
        config.notifications.enabled = false;
        assert!(matches!(
            spawn_shells(&config, "unused"),
            Err(SessionError::UnsupportedPolkitAgent(_))
        ));
    }

    #[test]
    fn restart_delay_is_bounded_exponential_backoff() {
        assert_eq!(restart_delay(1), Duration::from_millis(100));
        assert_eq!(restart_delay(3), Duration::from_millis(400));
        assert_eq!(restart_delay(99), Duration::from_millis(1_600));
    }

    #[test]
    fn startup_commands_receive_the_session_xwayland_display() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("display");
        let command = StartupCommand {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf '%s\\n' \"$DISPLAY\" > \"$1\"".into(),
                "sh".into(),
                output.to_string_lossy().into_owned(),
            ],
        };
        let mut children = start_startup_commands(&[command], "wayland-test", Some(":42"));
        assert_eq!(children.len(), 1);
        assert!(children[0].wait().unwrap().success());
        assert_eq!(fs::read_to_string(output).unwrap().trim(), ":42");
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
