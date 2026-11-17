//! Session-local mandatory GSettings values without modifying the user's database.
use crate::{SHUTDOWN_REQUESTED, SessionError};
use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

const PROFILE_LIMIT: u64 = 64 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(5);
const LAYOUT_KEY: &str = "/org/gnome/desktop/wm/preferences/button-layout";

pub(super) struct Policy {
    profile: PathBuf,
    directory: Option<tempfile::TempDir>,
    base: String,
    fallback_profile: PathBuf,
    original_backend: String,
    original_config_dirs: std::ffi::OsString,
    gtk_config_dirs: std::ffi::OsString,
}

impl Policy {
    pub(super) fn start(runtime: &Path) -> Result<Self, SessionError> {
        let data_dirs = env::var_os("XDG_DATA_DIRS")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let base = inherited_profile(
            env::var_os("DCONF_PROFILE").as_deref(),
            runtime,
            Path::new("/etc/dconf/profile"),
            &env::split_paths(&data_dirs).collect::<Vec<_>>(),
        )
        .map_err(|source| error("read inherited profile", source))?;
        Self::create(runtime, &base, Path::new("dconf"))
    }

    fn create(runtime: &Path, base: &str, compiler: &Path) -> Result<Self, SessionError> {
        if !runtime.is_absolute() {
            return Err(error(
                "prepare runtime",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "XDG_RUNTIME_DIR must be absolute",
                ),
            ));
        }
        let directory = tempfile::Builder::new()
            .prefix("knave-gtk-settings-")
            .tempdir_in(runtime)
            .map_err(|source| error("create runtime", source))?;
        let original_config_dirs = env::var_os("XDG_CONFIG_DIRS")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into());
        let gtk_config = directory.path().join("gtk-config");
        let gtk_config_dirs = append_config_dir(&original_config_dirs, &gtk_config)
            .map_err(|source| error("prepare GTK config path", source))?;
        let source = directory.path().join("source");
        fs::create_dir(&source).map_err(|source| error("create source", source))?;
        fs::create_dir(source.join("locks")).map_err(|source| error("create locks", source))?;
        fs::write(
            source.join("window-controls"),
            "[org/gnome/desktop/wm/preferences]\nbutton-layout=':'\n",
        )
        .map_err(|source| error("write settings", source))?;
        fs::write(
            source.join("locks/window-controls"),
            format!("{LAYOUT_KEY}\n"),
        )
        .map_err(|source| error("write lock", source))?;
        // GTK reads settings.ini when no Wayland settings portal is available.
        // Keep this session default after inherited system config directories;
        // XDG_CONFIG_HOME remains untouched and retains user precedence.
        for version in ["gtk-3.0", "gtk-4.0"] {
            let gtk_dir = gtk_config.join(version);
            fs::create_dir_all(&gtk_dir).map_err(|source| error("create GTK config", source))?;
            fs::write(
                gtk_dir.join("settings.ini"),
                "[Settings]\ngtk-decoration-layout=:\n",
            )
            .map_err(|source| error("write GTK settings", source))?;
        }
        let database = directory.path().join("policy.db");
        compile(compiler, &database, &source)?;
        let database_name = database
            .to_str()
            .filter(|value| !value.contains(['\n', '\r', '#']))
            .ok_or_else(|| {
                error(
                    "write profile",
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "runtime path must be UTF-8 without newlines or #",
                    ),
                )
            })?;
        let profile = directory.path().join("profile");
        // A lock in the last database overrides this key in every earlier
        // database while retaining the inherited profile for all other keys.
        fs::write(
            &profile,
            format!("{}\nfile-db:{database_name}\n", base.trim_end()),
        )
        .map_err(|source| error("write profile", source))?;
        Ok(Self {
            profile,
            directory: Some(directory),
            base: base.to_owned(),
            fallback_profile: runtime.join("knave-dconf-base-profile"),
            original_backend: env::var("GSETTINGS_BACKEND").unwrap_or_else(|_| "dconf".into()),
            original_config_dirs,
            gtk_config_dirs,
        })
    }

    pub(super) fn profile(&self) -> &Path {
        &self.profile
    }

    pub(super) fn gtk_config_dirs(&self) -> &std::ffi::OsStr {
        &self.gtk_config_dirs
    }
}

pub(super) fn configure(
    command: &mut Command,
    profile: Option<&Path>,
    gtk_config_dirs: Option<&std::ffi::OsStr>,
) {
    if let Some(profile) = profile {
        command
            .env("DCONF_PROFILE", profile)
            .env("GSETTINGS_BACKEND", "dconf")
            .env("KNAVE_GTK_SETTINGS_PROFILE", profile);
        if let Some(config_dirs) = gtk_config_dirs {
            command.env("XDG_CONFIG_DIRS", config_dirs);
        }
    }
}

fn inherited_profile(
    name: Option<&std::ffi::OsStr>,
    runtime: &Path,
    system: &Path,
    data_dirs: &[PathBuf],
) -> io::Result<String> {
    if let Some(name) = name.filter(|name| Path::new(name).is_absolute()) {
        return read_profile(Some(Path::new(name)), true);
    }
    if name.is_none() {
        match read_profile(Some(&runtime.join("dconf/profile")), true) {
            Ok(profile) => return Ok(profile),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let profile_name = name.unwrap_or_else(|| std::ffi::OsStr::new("user"));
    let paths = std::iter::once(system.join(profile_name)).chain(
        data_dirs
            .iter()
            .map(|data| data.join("dconf/profile").join(profile_name)),
    );
    for path in paths {
        match read_profile(Some(&path), true) {
            Ok(profile) => return Ok(profile),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if name.is_none() {
        Ok("user-db:user\n".into())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "named dconf profile was not found in /etc or XDG_DATA_DIRS",
        ))
    }
}

fn read_profile(path: Option<&Path>, required: bool) -> io::Result<String> {
    let Some(path) = path else {
        return Ok("user-db:user\n".into());
    };
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !required => {
            return Ok("user-db:user\n".into());
        }
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dconf profile must be a regular file",
        ));
    }
    let mut data = Vec::new();
    file.take(PROFILE_LIMIT + 1).read_to_end(&mut data)?;
    if data.len() as u64 > PROFILE_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dconf profile exceeds 64 KiB",
        ));
    }
    String::from_utf8(data).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn compile(compiler: &Path, database: &Path, source: &Path) -> Result<(), SessionError> {
    let mut command = Command::new(compiler);
    command.arg("compile").arg(database).arg(source);
    run_command(command, true).map_err(|source| error("compile policy (install dconf)", source))
}

fn run_command(mut command: Command, cancellable: bool) -> io::Result<()> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).spawn()?;
    let deadline = Instant::now() + COMPILE_TIMEOUT;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => {
                break Err(io::Error::other(format!("settings helper exited {status}")));
            }
            Err(error) => break Err(error),
            Ok(None) if cancellable && SHUTDOWN_REQUESTED.load(Ordering::SeqCst) => {
                break Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "session shutdown requested",
                ));
            }
            Ok(None) if Instant::now() >= deadline => {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "settings helper exceeded five seconds",
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
        }
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

impl Drop for Policy {
    fn drop(&mut self) {
        // Only direct compositor activation creates this marker. Nested sessions
        // must never change the enclosing desktop's activation environment.
        if !self.profile.with_extension("activated").exists() {
            return;
        }
        if let Err(error) = self.restore_activation() {
            eprintln!(
                "knave-session: could not restore GTK activation settings: {error}; retaining {}",
                self.profile.display()
            );
            if let Some(directory) = self.directory.take() {
                let _ = directory.keep();
            }
        }
    }
}

impl Policy {
    fn restore_activation(&self) -> io::Result<()> {
        self.restore_activation_with(Path::new("dbus-update-activation-environment"))
    }

    fn restore_activation_with(&self, updater: &Path) -> io::Result<()> {
        // D-Bus cannot unset activation variables. Keep one bounded runtime
        // fallback profile containing the original databases without our lock.
        let mut fallback = tempfile::NamedTempFile::new_in(
            self.fallback_profile
                .parent()
                .ok_or_else(|| io::Error::other("fallback profile parent missing"))?,
        )?;
        fallback.write_all(self.base.as_bytes())?;
        fallback
            .persist(&self.fallback_profile)
            .map_err(|error| error.error)?;
        let mut command = Command::new(updater);
        command
            .arg("--systemd")
            .arg(format!("DCONF_PROFILE={}", self.fallback_profile.display()))
            .arg(format!("GSETTINGS_BACKEND={}", self.original_backend))
            .arg({
                let mut assignment = std::ffi::OsString::from("XDG_CONFIG_DIRS=");
                assignment.push(&self.original_config_dirs);
                assignment
            });
        run_command(command, false)
    }
}

fn append_config_dir(original: &std::ffi::OsStr, policy: &Path) -> io::Result<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = env::split_paths(original).collect();
    dirs.push(policy.to_owned());
    env::join_paths(dirs).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

fn error(step: &'static str, source: io::Error) -> SessionError {
    SessionError::SettingsPolicy { step, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_locks_only_layout_and_removes_runtime_on_drop() {
        let runtime = tempfile::tempdir().unwrap();
        let policy = Policy::create(runtime.path(), "user-db:user\n", Path::new("dconf")).unwrap();
        let path = policy.profile().to_owned();
        for version in ["gtk-3.0", "gtk-4.0"] {
            assert_eq!(
                fs::read_to_string(
                    path.parent()
                        .unwrap()
                        .join("gtk-config")
                        .join(version)
                        .join("settings.ini")
                )
                .unwrap(),
                "[Settings]\ngtk-decoration-layout=:\n"
            );
        }
        let config = tempfile::tempdir().unwrap();
        let user_source = config.path().join("source");
        fs::create_dir(&user_source).unwrap();
        fs::create_dir(config.path().join("dconf")).unwrap();
        fs::write(user_source.join("user-settings"), "[org/gnome/desktop/wm/preferences]\nbutton-layout=':close'\n[org/gnome/desktop/interface]\nfont-name='Knave Test 12'\n").unwrap();
        compile(
            Path::new("dconf"),
            &config.path().join("dconf/user"),
            &user_source,
        )
        .unwrap();
        let query = |args: &[&str], profile: Option<&Path>| {
            let mut command = Command::new("gsettings");
            command
                .args(args)
                .env("XDG_CONFIG_HOME", config.path())
                .env_remove("DCONF_PROFILE");
            configure(&mut command, profile, Some(policy.gtk_config_dirs()));
            command.output().unwrap()
        };
        let args = ["get", "org.gnome.desktop.wm.preferences", "button-layout"];
        let original = query(&args, None).stdout;
        assert_eq!(original, b"':close'\n");
        assert_eq!(
            query(
                &["get", "org.gnome.desktop.interface", "font-name"],
                Some(&path)
            )
            .stdout,
            b"'Knave Test 12'\n"
        );
        assert_eq!(query(&args, Some(&path)).stdout, b"':'\n");
        assert_eq!(
            query(
                &[
                    "writable",
                    "org.gnome.desktop.wm.preferences",
                    "button-layout"
                ],
                Some(&path)
            )
            .stdout,
            b"false\n"
        );
        assert!(
            !query(
                &[
                    "set",
                    "org.gnome.desktop.wm.preferences",
                    "button-layout",
                    ":close"
                ],
                Some(&path)
            )
            .status
            .success()
        );
        assert_eq!(
            query(
                &["writable", "org.gnome.desktop.interface", "font-name"],
                Some(&path)
            )
            .stdout,
            b"true\n"
        );
        assert_eq!(query(&args, None).stdout, original);
        drop(policy);
        assert!(!path.exists());
    }
    #[cfg(unix)]
    #[test]
    fn invalid_runtime_config_path_is_a_typed_error_and_cleans_up() {
        let runtime = tempfile::tempdir().unwrap();
        let bad_runtime = runtime.path().join("bad:runtime");
        fs::create_dir(&bad_runtime).unwrap();
        let error = match Policy::create(&bad_runtime, "user-db:user\n", Path::new("dconf")) {
            Ok(_) => panic!("a colon in XDG_CONFIG_DIRS must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            SessionError::SettingsPolicy {
                step: "prepare GTK config path",
                ..
            }
        ));
        assert_eq!(fs::read_dir(bad_runtime).unwrap().count(), 0);
    }

    #[test]
    fn activation_fallback_retains_only_the_original_profile() {
        let runtime = tempfile::tempdir().unwrap();
        let base = "user-db:user\nsystem-db:site\n";
        let policy = Policy::create(runtime.path(), base, Path::new("dconf")).unwrap();
        policy
            .restore_activation_with(Path::new("/bin/true"))
            .unwrap();
        assert_eq!(fs::read_to_string(&policy.fallback_profile).unwrap(), base);
        let active_profile = policy.profile().to_owned();
        let fallback = policy.fallback_profile.clone();
        drop(policy);
        assert!(!active_profile.exists());
        assert_eq!(fs::read_to_string(fallback).unwrap(), base);
    }

    #[test]
    fn named_and_default_profiles_follow_dconf_search_order() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("etc");
        let data = root.path().join("share");
        let runtime = root.path().join("runtime");
        fs::create_dir_all(&system).unwrap();
        fs::create_dir_all(data.join("dconf/profile")).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        let dirs = [data.clone()];
        fs::write(data.join("dconf/profile/site"), "user-db:xdg\n").unwrap();
        assert_eq!(
            inherited_profile(Some(std::ffi::OsStr::new("site")), &runtime, &system, &dirs)
                .unwrap(),
            "user-db:xdg\n"
        );
        fs::write(system.join("site"), "user-db:etc\n").unwrap();
        assert_eq!(
            inherited_profile(Some(std::ffi::OsStr::new("site")), &runtime, &system, &dirs)
                .unwrap(),
            "user-db:etc\n"
        );
        assert_eq!(
            inherited_profile(None, &runtime, &system, &dirs).unwrap(),
            "user-db:user\n"
        );
        fs::write(data.join("dconf/profile/user"), "user-db:default\n").unwrap();
        assert_eq!(
            inherited_profile(None, &runtime, &system, &dirs).unwrap(),
            "user-db:default\n"
        );
        fs::create_dir(runtime.join("dconf")).unwrap();
        fs::write(runtime.join("dconf/profile"), "user-db:runtime\n").unwrap();
        assert_eq!(
            inherited_profile(None, &runtime, &system, &dirs).unwrap(),
            "user-db:runtime\n"
        );
    }

    #[test]
    fn runtime_comment_delimiter_cannot_disable_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime#comment");
        fs::create_dir(&runtime).unwrap();
        assert!(Policy::create(&runtime, "user-db:user\n", Path::new("dconf")).is_err());
        assert_eq!(fs::read_dir(runtime).unwrap().count(), 0);
    }

    #[test]
    fn inherited_profile_is_bounded_and_compiler_errors_clean_up() {
        let runtime = tempfile::tempdir().unwrap();
        let path = runtime.path().join("base");
        fs::write(&path, "user-db:custom\nsystem-db:site\n").unwrap();
        assert_eq!(
            read_profile(Some(&path), true).unwrap(),
            "user-db:custom\nsystem-db:site\n"
        );
        fs::write(&path, vec![b'x'; PROFILE_LIMIT as usize + 1]).unwrap();
        assert!(read_profile(Some(&path), true).is_err());
        assert!(read_profile(Some(&runtime.path().join("missing")), true).is_err());
        assert!(Policy::create(runtime.path(), "user-db:user", Path::new("/bin/false")).is_err());
        assert_eq!(fs::read_dir(runtime.path()).unwrap().count(), 1);
    }
}
