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
    let user_gtk = root.join("gtk-4.0");
    fs::create_dir(&user_gtk).unwrap();
    let user_settings = "[Settings]\ngtk-font-name=Sans 13\n";
    fs::write(user_gtk.join("settings.ini"), user_settings).unwrap();
    let inherited_dirs = env::join_paths([root.join("system-a"), root.join("system-b")]).unwrap();
    let wrapper = root.join("component");
    // Shell quoting preserves paths containing spaces or apostrophes.
    let executable = env::current_exe().unwrap();
    let quoted = executable.to_str().unwrap().replace('\'', "'\\''");
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nshift; [ \"$1\" = shell ] && shift\nexport KNAVE_LIFECYCLE_TEST_ROLE=\"$1\"\nexec '{quoted}' --ignored --exact component --nocapture\n"),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let config_dir = root.join("knave");
    fs::create_dir(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        format!(
            "[session]\nbackend = \"tty\"\nrestart_on_failure = {restart}\n[shell]\nstart_overview_service = false\n[notifications]\nenabled = false\n",
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
        Command::new(env::current_exe().unwrap())
            .args(["--ignored", "--exact", "supervisor", "--nocapture"])
            .env("KNAVE_LIFECYCLE_TEST_EXECUTABLE", &wrapper)
            .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
            .env("XDG_RUNTIME_DIR", root)
            .env("XDG_CONFIG_HOME", root)
            .env("XDG_CONFIG_DIRS", inherited_dirs)
            .env("KNAVE_LIFECYCLE_TEST_ROOT", root)
            .env("KNAVE_LIFECYCLE_TEST_EXIT", status)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("KNAVE_CONFIG")
            .env_remove("KNAVE_SOCKET")
            .spawn()
            .unwrap(),
    );
    wait_until(|| root.join("shells").exists());
    wait_until(|| root.join("polkit-agent-started").exists());
    fs::write(root.join("quit"), "").unwrap();
    assert_eq!(session.wait().success(), success);
    let policy = fs::read_to_string(root.join("settings-profile")).unwrap();
    assert!(
        !Path::new(&policy).exists(),
        "session settings policy survived shutdown"
    );
    assert!(
        !Path::new(&policy)
            .parent()
            .unwrap()
            .join("gtk-config")
            .exists(),
        "session GTK fallback survived shutdown"
    );
    assert_eq!(
        fs::read_to_string(user_gtk.join("settings.ini")).unwrap(),
        user_settings
    );
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
    let policy =
        Path::new(&env::var_os("DCONF_PROFILE").expect("session settings policy")).to_owned();
    assert!(policy.is_file());
    assert_eq!(
        env::var_os("KNAVE_GTK_SETTINGS_PROFILE").as_deref(),
        Some(policy.as_os_str())
    );
    assert_eq!(env::var("GSETTINGS_BACKEND").as_deref(), Ok("dconf"));
    let gtk_config = policy.parent().unwrap().join("gtk-config");
    assert_eq!(
        env::split_paths(&env::var_os("XDG_CONFIG_DIRS").expect("session GTK config path"))
            .collect::<Vec<_>>(),
        vec![
            root.join("system-a"),
            root.join("system-b"),
            gtk_config.clone()
        ],
        "every desktop child must preserve inherited defaults and receive the session fallback"
    );
    assert_eq!(
        fs::read_to_string(gtk_config.join("gtk-4.0/settings.ini")).unwrap(),
        "[Settings]\ngtk-decoration-layout=:minimize,maximize,close\n"
    );
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
    if shell.is_none() {
        fs::write(
            root.join("settings-profile"),
            policy.to_string_lossy().as_bytes(),
        )
        .unwrap();
    }
    if let Some(display) = shell {
        if env::var("KNAVE_LIFECYCLE_TEST_ROLE").as_deref() == Ok("polkit-agent") {
            fs::write(root.join("polkit-agent-started"), pid.to_string()).unwrap();
        }
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

#[test]
#[ignore = "subprocess supervisor fixture"]
fn supervisor() {
    let config = knave_config::ConfigDocument::at_default_path().unwrap();
    let executable = env::var_os("KNAVE_LIFECYCLE_TEST_EXECUTABLE").unwrap();
    if let Err(error) =
        knave_session::run_with_executable(config.config().clone(), Path::new(&executable))
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
