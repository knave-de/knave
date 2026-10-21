//! Exercise the real supervisor in isolated processes with socket-owning children.
use std::{
    env, fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "lifecycle fixture timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

struct Session(Child);
impl Session {
    fn wait(&mut self) -> ExitStatus {
        let mut status = None;
        wait_until(|| {
            status = self.0.try_wait().unwrap();
            status.is_some()
        });
        status.unwrap()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn exercise(status: &str, restart: bool, launches: usize, success: bool) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let wrapper = root.join("component");
    // Shell quoting preserves paths containing spaces or apostrophes.
    let executable = env::current_exe().unwrap();
    let quoted = executable.to_str().unwrap().replace('\'', "'\\''");
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec '{quoted}' --ignored --exact component --nocapture\n"),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let config_dir = root.join("knave");
    fs::create_dir(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!(
            "[session]\ncompositor_binary = {binary}\nshell_binary = {binary}\nrestart_on_failure = {restart}\n[shell]\nstart_overview_service = false\n[notifications]\nenabled = false\n",
            binary = serde_json::to_string(wrapper.to_str().unwrap()).unwrap(),
        ),
    )
    .unwrap();
    // Every real session service stays on a private bus, including parallel fixtures.
    let mut bus = Session(
        Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut address = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(bus.0.stdout.take().unwrap()),
        &mut address,
    )
    .unwrap();
    let mut session = Session(
        Command::new(env!("CARGO_BIN_EXE_knave-session"))
            .arg("start")
            .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
            .env("XDG_RUNTIME_DIR", root)
            .env("XDG_CONFIG_HOME", root)
            .env("KNAVE_LIFECYCLE_TEST_ROOT", root)
            .env("KNAVE_LIFECYCLE_TEST_EXIT", status)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("KNAVE_CONFIG")
            .env_remove("KNAVE_SOCKET")
            .spawn()
            .unwrap(),
    );
    wait_until(|| root.join("shells").exists());
    fs::write(root.join("quit"), "").unwrap();
    assert_eq!(session.wait().success(), success);
    let compositors = fs::read_to_string(root.join("compositors")).unwrap();
    assert_eq!(compositors.lines().count(), launches);
    for pid in fs::read_to_string(root.join("shells")).unwrap().lines() {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "unreaped shell {pid}"
        );
    }
}

#[test]
fn clean_compositor_exit_ends_session_with_restarts_enabled() {
    exercise("0", true, 1, true);
}

#[test]
fn clean_compositor_exit_ends_session_with_restarts_disabled() {
    exercise("0", false, 1, true);
}

#[test]
fn compositor_failure_uses_bounded_restart_budget() {
    exercise("1", true, 4, false);
}

#[test]
fn compositor_failure_respects_disabled_restarts() {
    exercise("1", false, 1, false);
}

#[test]
fn compositor_signal_is_a_failure() {
    exercise("signal", true, 4, false);
}

#[test]
#[ignore = "subprocess fixture"]
fn component() {
    use std::io::Write;
    let root = env::var_os("KNAVE_LIFECYCLE_TEST_ROOT").unwrap();
    let root = Path::new(&root);
    let pid = std::process::id();
    let shell = env::var_os("WAYLAND_DISPLAY");
    let log = if shell.is_some() {
        "shells"
    } else {
        "compositors"
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(log))
        .unwrap();
    file.write_all(format!("{pid}\n").as_bytes()).unwrap();
    if let Some(display) = shell {
        fs::write(root.join(display).with_extension("ready"), "").unwrap();
        thread::sleep(Duration::from_secs(15));
    } else {
        let socket = root.join(format!("wayland-{pid}"));
        let _listener = UnixListener::bind(&socket).unwrap();
        wait_until(|| root.join("quit").exists() && socket.with_extension("ready").exists());
        fs::remove_file(socket).unwrap();
        let status = env::var("KNAVE_LIFECYCLE_TEST_EXIT").unwrap();
        if status == "signal" {
            unsafe {
                libc::raise(libc::SIGKILL);
            }
        }
        std::process::exit(status.parse().unwrap());
    }
}
