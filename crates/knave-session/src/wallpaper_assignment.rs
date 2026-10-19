//! Reload-driven wallpaper assignment. The session keeps at most one
//! wallpaper child and reconciles it with `[shell.wallpaper]` after each
//! successful desktop reload: a new assignment spawns the process, a
//! cleared one stops it, and image or fit changes on a running process
//! stay entirely the shell's own reload handling.

use std::{
    process::{Child, ExitStatus},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use knave_config::{Config, ConfigDocument, WallpaperFit};
use knave_desktop_api::{DesktopEvent, DesktopSubscription, SubscriptionCancel};

use crate::SessionError;

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// A wallpaper assignment freshly read from configuration; `None` unassigns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Assignment {
    image: String,
    fit: WallpaperFit,
}

/// The session-owned wallpaper child. Spawning and stopping serialize
/// through the slot so the supervisor loop, the assignment thread, and
/// shutdown always agree on the live process.
#[derive(Default)]
pub(super) struct AssignmentSlot {
    child: Mutex<Option<Child>>,
    cancel: Mutex<Option<SubscriptionCancel>>,
    stopped: AtomicBool,
}

impl AssignmentSlot {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Spawns per the startup or shell-respawn configuration; an unassigned
    /// configuration leaves the slot empty. Spawn failures abort the
    /// procedure, exactly like the other shell roles.
    pub(super) fn reset(&self, config: &Config, display: &str) -> Result<(), SessionError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.stop_child();
        if config.shell.wallpaper.image.is_some() {
            let child = crate::spawn_shell(config, "wallpaper", display)?;
            *self.child.lock().unwrap() = Some(child.child);
        }
        Ok(())
    }

    /// Applies a reconciliation result: spawn on a new assignment, stop on
    /// a cleared one, and neither when the assignment already matches.
    /// Spawn failures leave the slot empty so the next reload event retries.
    pub(super) fn apply(
        &self,
        config: &Config,
        display: &str,
        desired: Option<&Assignment>,
    ) -> Result<(), SessionError> {
        let mut assigned = self.lock_child();
        if self.stopped.load(Ordering::SeqCst) {
            return Ok(());
        }
        match desired {
            // Image and fit changes on a running process are the shell's
            // own reload handling; the process identity stays.
            Some(_) if assigned.is_some() => Ok(()),
            Some(request) => {
                // The fresh assignment carries the current image and fit as
                // arguments; the shell binary stays the startup choice.
                let mut spawn_config = config.clone();
                spawn_config.shell.wallpaper.image = Some(request.image.clone());
                spawn_config.shell.wallpaper.fit = request.fit;
                *assigned = Some(crate::spawn_shell(&spawn_config, "wallpaper", display)?.child);
                Ok(())
            }
            None if assigned.is_none() => Ok(()),
            None => {
                let taken = assigned.take();
                drop(assigned);
                if let Some(mut child) = taken {
                    crate::terminate_child(&mut child);
                }
                Ok(())
            }
        }
    }

    /// Reaps the wallpaper child once it exits, for the supervisor loop.
    pub(super) fn exited(&self) -> Result<Option<ExitStatus>, SessionError> {
        let mut assigned = self.lock_child();
        match assigned.as_mut() {
            Some(child) => child
                .try_wait()
                .map_err(|error| SessionError::Process(format!("wallpaper wait failed: {error}"))),
            None => Ok(None),
        }
    }

    /// Blocks further reconciliation, disconnects the subscription, and
    /// stops the live child. Safe to call repeatedly and alongside a
    /// concurrent reconcile, which then observes the stopped state.
    pub(super) fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(cancel) = self.cancel.lock().unwrap().take() {
            let _ = cancel.cancel();
        }
        self.stop_child();
    }

    /// Stops the child if it is still live; an already exited child was
    /// reaped by the exit check itself. Unlike shutdown, reconciliation
    /// stays enabled, so a restarted compositor can be re-attached to.
    pub(super) fn disown(&self) {
        self.stop_child();
    }

    /// Stops the child if it is still live; an already exited child was
    /// reaped by the exit check itself.
    fn stop_child(&self) {
        let Some(mut taken) = self.child.lock().unwrap().take() else {
            return;
        };
        match taken.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => crate::terminate_child(&mut taken),
        }
    }

    fn lock_child(&self) -> MutexGuard<'_, Option<Child>> {
        self.child.lock().unwrap()
    }

    #[cfg(test)]
    fn pid(&self) -> Option<u32> {
        self.child.lock().unwrap().as_ref().map(Child::id)
    }
}

/// The assignment thread is process-scoped: subscription breaks reconnect
/// with the same backoff as the shell's reload worker, while the slot's
/// stopped flag and revoke handle end the loop at shutdown.
pub(super) fn thread(config: Config, display: String, slot: Arc<AssignmentSlot>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("knave-wallpaper-assignment".into())
        .spawn(move || serve(config, display, slot))
        .expect("wallpaper assignment thread spawns")
}

fn serve(config: Config, display: String, slot: Arc<AssignmentSlot>) {
    let mut backoff = INITIAL_BACKOFF;
    let mut last_error: Option<String> = None;
    loop {
        if slot.stopped.load(Ordering::SeqCst) {
            return;
        }
        let connection: Result<(), String> = (|| {
            let mut subscription =
                DesktopSubscription::connect().map_err(|error| error.to_string())?;
            let cancel = subscription
                .cancel_handle()
                .map_err(|error| error.to_string())?;
            *slot.cancel.lock().unwrap() = Some(cancel);
            loop {
                if matches!(
                    subscription
                        .next_event()
                        .map_err(|error| error.to_string())?,
                    DesktopEvent::ConfigurationReloaded
                ) {
                    match read_assignment() {
                        Ok(desired) => {
                            if let Err(error) = slot.apply(&config, &display, desired.as_ref()) {
                                eprintln!(
                                    "knave-session: wallpaper assignment: {error}; retrying at the next reload"
                                );
                            }
                        }
                        Err(error) => eprintln!(
                            "knave-session: wallpaper assignment: {error}; keeping the current assignment"
                        ),
                    }
                }
                backoff = INITIAL_BACKOFF;
                last_error = None;
            }
        })();
        *slot.cancel.lock().unwrap() = None;
        if slot.stopped.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = connection {
            let message = format!("desktop subscription unavailable: {error}");
            if last_error.as_ref() != Some(&message) {
                eprintln!("knave-session: wallpaper assignment: {message}");
                last_error = Some(message);
            }
        }
        thread::sleep(backoff);
        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
    }
}

/// Reads the desired assignment from the canonical configuration. Read or
/// validation failures keep the current assignment.
fn read_assignment() -> Result<Option<Assignment>, String> {
    let path = ConfigDocument::default_path().map_err(|error| error.to_string())?;
    let document = ConfigDocument::load(path).map_err(|error| error.to_string())?;
    let wallpaper = &document.config().shell.wallpaper;
    Ok(wallpaper.image.as_ref().map(|image| Assignment {
        image: image.clone(),
        fit: wallpaper.fit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    fn long_running_stub(directory: &Path) -> String {
        // The stub records its arguments next to itself and lingers, so a
        // spawn can be verified without reading /proc.
        script(
            directory,
            "stub-shell",
            &format!(
                "printf '%s\\n' \"$@\" > \"{}/argv.txt\"\nwhile true; do sleep 1; done",
                directory.display()
            ),
        )
    }

    /// A recording stub started just before an assertion may not have
    /// reached the write yet; wait for the marker instead.
    fn recorded_arguments(directory: &Path) -> Vec<String> {
        let marker = directory.join("argv.txt");
        for _ in 0..100 {
            if let Ok(text) = fs::read_to_string(&marker) {
                return text
                    .lines()
                    .filter(|argument| !argument.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("stub did not record its arguments");
    }

    fn script(directory: &Path, name: &str, body: &str) -> String {
        let binary = directory.join(name);
        fs::write(&binary, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        binary.to_string_lossy().into_owned()
    }

    fn config_with(binary: &str, image: Option<String>) -> Config {
        let mut config = Config::default();
        config.session.shell_binary = binary.to_owned();
        config.shell.wallpaper.image = image;
        config
    }

    /// A reaped pid has no /proc entry; a live or zombie pid does.
    fn exited_and_cleared(pid: u32) -> bool {
        for _ in 0..40 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn an_assignment_spawns_the_configured_wallpaper() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("assigned.png");
        fs::write(&image, b"png").unwrap();
        let slot = AssignmentSlot::new();
        let config = config_with(
            &long_running_stub(directory.path()),
            Some(image.to_string_lossy().into_owned()),
        );

        slot.apply(
            &config,
            "unused",
            Some(&Assignment {
                image: image.to_string_lossy().into_owned(),
                fit: WallpaperFit::Contain,
            }),
        )
        .unwrap();

        let argv = recorded_arguments(directory.path());
        assert!(argv.contains(&image.to_str().unwrap().to_string()));
        assert!(argv.contains(&"contain".to_string()));

        slot.shutdown();
    }

    #[test]
    fn an_unchanged_assignment_is_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("assigned.png");
        fs::write(&image, b"png").unwrap();
        let slot = AssignmentSlot::new();
        let config = config_with(
            &long_running_stub(directory.path()),
            Some(image.to_string_lossy().into_owned()),
        );

        slot.apply(
            &config,
            "unused",
            Some(&Assignment {
                image: image.to_string_lossy().into_owned(),
                fit: WallpaperFit::Cover,
            }),
        )
        .unwrap();
        let first = slot.pid();
        slot.apply(
            &config,
            "unused",
            Some(&Assignment {
                image: image.to_string_lossy().into_owned(),
                fit: WallpaperFit::Cover,
            }),
        )
        .unwrap();
        assert_eq!(slot.pid(), first);

        slot.shutdown();
    }

    #[test]
    fn a_cleared_assignment_stops_and_reaps_the_wallpaper() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("assigned.png");
        fs::write(&image, b"png").unwrap();
        let slot = AssignmentSlot::new();
        let config = config_with(
            &long_running_stub(directory.path()),
            Some(image.to_string_lossy().into_owned()),
        );

        slot.apply(
            &config,
            "unused",
            Some(&Assignment {
                image: image.to_string_lossy().into_owned(),
                fit: WallpaperFit::Cover,
            }),
        )
        .unwrap();
        let pid = slot.pid().expect("assigned child");
        slot.apply(&config, "unused", None).unwrap();

        assert_eq!(slot.pid(), None);
        assert!(
            exited_and_cleared(pid),
            "child {pid} was stopped and reaped"
        );

        slot.shutdown();
    }

    #[test]
    fn a_spawn_failure_keeps_the_slot_empty_for_the_next_reload() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("assigned.png");
        fs::write(&image, b"png").unwrap();
        let slot = AssignmentSlot::new();
        let config = config_with("./definitely-not-a-shell-binary", None);

        let error = slot
            .apply(
                &config,
                "unused",
                Some(&Assignment {
                    image: image.to_string_lossy().into_owned(),
                    fit: WallpaperFit::Cover,
                }),
            )
            .expect_err("assignment spawn succeeds");
        assert!(matches!(error, SessionError::Spawn { .. }));
        assert_eq!(slot.pid(), None);

        slot.shutdown();
    }

    #[test]
    fn shutdown_blocks_late_reconcile_results() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("assigned.png");
        fs::write(&image, b"png").unwrap();
        let slot = AssignmentSlot::new();
        let config = config_with(
            &long_running_stub(directory.path()),
            Some(image.to_string_lossy().into_owned()),
        );

        slot.shutdown();
        slot.apply(
            &config,
            "unused",
            Some(&Assignment {
                image: image.to_string_lossy().into_owned(),
                fit: WallpaperFit::Cover,
            }),
        )
        .unwrap();
        assert_eq!(slot.pid(), None);
    }
}
