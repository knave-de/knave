//! Optional, one-shot readiness acknowledgement for an isolated desktop harness.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct ReadySignal(Option<PathBuf>);

impl ReadySignal {
    pub(crate) fn from_env(namespace: &str) -> Self {
        Self::new(
            std::env::var_os("KNAVE_HARNESS_READY_DIR").map(PathBuf::from),
            namespace,
        )
    }

    fn new(directory: Option<PathBuf>, namespace: &str) -> Self {
        let role = match namespace {
            "knave-shell-bar" => "bar",
            "knave-shell-wallpaper" => "wallpaper",
            "knave-shell-overview" => "overview",
            "knave-shell-notifications" => "notifications",
            _ => return Self::default(),
        };
        Self(directory.map(|path| path.join(format!("{role}.ready"))))
    }

    pub(crate) fn notify(&mut self) -> io::Result<()> {
        if let Some(path) = &self.0 {
            match write_marker(path) {
                Ok(()) => {}
                // A shell respawned later in the same session sees the first
                // signal's marker; the harness startup wait only needs the
                // role signalled once, so treat the duplicate as success.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            self.0 = None;
        }
        Ok(())
    }
}

fn write_marker(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(b"ready\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_and_unrelated_surfaces_do_not_signal() {
        let mut disabled = ReadySignal::new(None, "knave-shell-bar");
        disabled.notify().unwrap();
        let unrelated = ReadySignal::new(Some(PathBuf::from("/unused")), "knave-shell-portal");
        assert!(unrelated.0.is_none());
    }

    #[test]
    fn roles_use_distinct_marker_paths() {
        let directory = Some(PathBuf::from("/private-run"));
        let bar = ReadySignal::new(directory.clone(), "knave-shell-bar");
        let overview = ReadySignal::new(directory, "knave-shell-overview");
        assert_eq!(bar.0, Some(PathBuf::from("/private-run/bar.ready")));
        assert_eq!(
            overview.0,
            Some(PathBuf::from("/private-run/overview.ready"))
        );
    }

    #[test]
    fn marker_is_one_shot_and_notification_is_idempotent() {
        let directory =
            std::env::temp_dir().join(format!("knave-ready-test-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let mut signal = ReadySignal::new(Some(directory.clone()), "knave-shell-bar");
        signal.notify().unwrap();
        let path = directory.join("bar.ready");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ready\n");
        // A re-spawned shell must not fail on the existing marker.
        signal.notify().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
