//! Reload-driven wallpaper updates. One background thread subscribes to the
//! desktop socket, re-validates `[shell.wallpaper]` on
//! `ConfigurationReloaded`, decodes replacements off the render loop and
//! keeps at most one pending update: reloads are levels, so an unread event
//! is superseded by the next one instead of being queued.

use knave_config::{ConfigDocument, WallpaperFit};
use knave_desktop_api::{DesktopEvent, DesktopSubscription, SubscriptionCancel};
use knave_ui::{ImageFit, UiImage};
use knave_wayland::Waker;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// What the render loop currently presents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedWallpaper {
    pub path: Option<String>,
    pub fit: ImageFit,
}

enum Desired {
    Image { path: String, fit: ImageFit },
    Cleared,
}

/// A change the render loop has not applied yet.
#[derive(Debug)]
pub enum ReloadUpdate {
    /// Swap the image; `None` renders the black background.
    Image {
        fit: ImageFit,
        image: Option<UiImage>,
    },
    /// The configured path is unchanged; only the fit moved.
    FitChange { fit: ImageFit },
}

type LoadImage = fn(&Path) -> Result<UiImage, String>;

struct Shared {
    applied: AppliedWallpaper,
    pending: Option<ReloadUpdate>,
    stopped: bool,
    cancel: Option<SubscriptionCancel>,
}

/// Owns the subscription thread. Dropping it stops, disconnects and joins.
pub struct ReloadWorker {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl ReloadWorker {
    /// `applied` is the wallpaper surfaced from the startup arguments; later
    /// changes come only through reload events.
    pub fn start(waker: Waker, applied: AppliedWallpaper, load: LoadImage) -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                applied,
                pending: None,
                stopped: false,
                cancel: None,
            }),
            Condvar::new(),
        ));
        let thread_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("knave-wallpaper-reload".into())
            .spawn(move || {
                let (shared, retry) = &*thread_shared;
                let mut backoff = INITIAL_BACKOFF;
                let mut last_error = None;
                loop {
                    if shared.lock().unwrap().stopped {
                        return;
                    }
                    let result: Result<(), knave_desktop_api::ClientError> = (|| {
                        let mut subscription = DesktopSubscription::connect()?;
                        let cancel = subscription.cancel_handle()?;
                        {
                            let mut state = shared.lock().unwrap();
                            if state.stopped {
                                let _ = cancel.cancel();
                                return Ok(());
                            }
                            state.cancel = Some(cancel);
                        }
                        loop {
                            if matches!(
                                subscription.next_event()?,
                                DesktopEvent::ConfigurationReloaded
                            ) {
                                match default_config_path() {
                                    Ok(path) => {
                                        let notify = || waker.wake();
                                        evaluate(&path, thread_shared.as_ref(), load, &notify);
                                    }
                                    Err(error) => eprintln!(
                                        "knave-shell: wallpaper reload: {error}; keeping previous image"
                                    ),
                                }
                            }
                            backoff = INITIAL_BACKOFF;
                            last_error = None;
                        }
                    })();
                    let mut state = shared.lock().unwrap();
                    state.cancel = None;
                    if state.stopped {
                        return;
                    }
                    if let Err(error) = result {
                        let message = format!("Desktop subscription unavailable: {error}");
                        if last_error.as_ref() != Some(&message) {
                            eprintln!("knave-shell: {message}");
                            last_error = Some(message);
                        }
                    }
                    let (state, _) = retry
                        .wait_timeout_while(state, backoff, |s| !s.stopped)
                        .unwrap();
                    if state.stopped {
                        return;
                    }
                    backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                }
            })
            .expect("wallpaper reload thread spawns");
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Whether an update is waiting to be applied by `frame`.
    pub fn pending(&self) -> bool {
        self.shared.0.lock().unwrap().pending.is_some()
    }

    /// Takes the pending update for the render loop.
    pub fn take_update(&self) -> Option<ReloadUpdate> {
        self.shared.0.lock().unwrap().pending.take()
    }
}

impl Drop for ReloadWorker {
    fn drop(&mut self) {
        let (shared, retry) = &*self.shared;
        {
            let mut state = shared.lock().unwrap();
            state.stopped = true;
            if let Some(cancel) = state.cancel.take() {
                let _ = cancel.cancel();
            }
            retry.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn default_config_path() -> Result<PathBuf, String> {
    ConfigDocument::default_path().map_err(|error| error.to_string())
}

fn image_fit(value: WallpaperFit) -> Option<ImageFit> {
    match value.as_str() {
        "cover" => Some(ImageFit::Cover),
        "contain" => Some(ImageFit::Contain),
        "stretch" => Some(ImageFit::Stretch),
        _ => None,
    }
}

/// Reads the wallpaper projection from an already validated configuration.
fn desired_at(path: &Path) -> Result<Desired, String> {
    let document = ConfigDocument::load(path).map_err(|error| error.to_string())?;
    let config = document.config();
    match &config.shell.wallpaper.image {
        Some(image) => Ok(Desired::Image {
            path: image.clone(),
            fit: image_fit(config.shell.wallpaper.fit)
                .ok_or_else(|| "wallpaper fit must be cover, contain, or stretch".to_string())?,
        }),
        None => Ok(Desired::Cleared),
    }
}

/// Compares the configured wallpaper with what is rendered and stages one
/// update. Decode failures keep the previous image; the next reload retries.
fn evaluate(config: &Path, shared: &(Mutex<Shared>, Condvar), load: LoadImage, notify: &dyn Fn()) {
    let desired = match desired_at(config) {
        Ok(desired) => desired,
        Err(error) => {
            eprintln!("knave-shell: wallpaper reload: {error}; keeping previous image");
            return;
        }
    };
    let mut state = shared.0.lock().unwrap();
    if state.stopped {
        return;
    }
    let changed = match desired {
        Desired::Image { path, fit } => {
            if state.applied.path.as_deref() == Some(path.as_str()) {
                if state.applied.fit == fit {
                    false
                } else {
                    state.applied.fit = fit;
                    state.pending = Some(ReloadUpdate::FitChange { fit });
                    true
                }
            } else {
                match load(Path::new(&path)) {
                    Ok(image) => {
                        state.applied = AppliedWallpaper {
                            path: Some(path.clone()),
                            fit,
                        };
                        state.pending = Some(ReloadUpdate::Image {
                            fit,
                            image: Some(image),
                        });
                        true
                    }
                    Err(error) => {
                        eprintln!(
                            "knave-shell: wallpaper reload {path}: {error}; keeping previous image"
                        );
                        false
                    }
                }
            }
        }
        Desired::Cleared => {
            if state.applied.path.is_none() {
                false
            } else {
                state.applied = AppliedWallpaper {
                    path: None,
                    fit: ImageFit::Cover,
                };
                state.pending = Some(ReloadUpdate::Image {
                    fit: ImageFit::Cover,
                    image: None,
                });
                true
            }
        }
    };
    drop(state);
    if changed {
        notify();
    }
}

#[cfg(test)]
fn must_not_load(_: &Path) -> Result<UiImage, String> {
    panic!("fit-only changes and unchanged wallpaper must not decode")
}

#[cfg(test)]
fn failing_load(_: &Path) -> Result<UiImage, String> {
    Err("invalid image".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper;
    use std::sync::mpsc::{self, TryRecvError};

    fn shared_with(applied: AppliedWallpaper) -> Arc<(Mutex<Shared>, Condvar)> {
        Arc::new((
            Mutex::new(Shared {
                applied,
                pending: None,
                stopped: false,
                cancel: None,
            }),
            Condvar::new(),
        ))
    }

    fn encode_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[1, 2, 3])
                .unwrap();
        }
        bytes
    }

    fn toml_string(value: &str) -> String {
        format!("\"{}\"", value.replace('\\', "\\\\"))
    }

    fn config_document(directory: &tempfile::TempDir, image: Option<&Path>, fit: &str) -> PathBuf {
        let path = directory.path().join("config.toml");
        let wallpaper = match image {
            Some(image) => {
                format!(
                    "image = {}\nfit = \"{}\"",
                    toml_string(&image.to_string_lossy()),
                    fit
                )
            }
            None => String::new(),
        };
        std::fs::write(&path, format!("[shell.wallpaper]\n{wallpaper}")).unwrap();
        path
    }

    #[test]
    fn image_fit_maps_only_supported_values() {
        assert_eq!(image_fit(WallpaperFit::Cover), Some(ImageFit::Cover));
        assert_eq!(image_fit(WallpaperFit::Contain), Some(ImageFit::Contain));
        assert_eq!(image_fit(WallpaperFit::Stretch), Some(ImageFit::Stretch));
    }

    #[test]
    fn a_missing_configuration_renders_black() {
        let applied = AppliedWallpaper {
            path: Some("current.png".into()),
            fit: ImageFit::Cover,
        };
        let shared = shared_with(applied);
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        evaluate(
            Path::new("/definitely/missing/config.toml"),
            &shared,
            must_not_load,
            &wake,
        );
        let mut state = shared.0.lock().unwrap();
        assert!(state.applied.path.is_none());
        match state.pending.take() {
            Some(ReloadUpdate::Image { image, .. }) => assert!(image.is_none()),
            other => panic!("expected a clear update: {other:?}"),
        }
        drop(state);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn an_invalid_configuration_keeps_the_previous_image() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        std::fs::write(&config, "not toml at all[[[").unwrap();
        let applied = AppliedWallpaper {
            path: Some("current.png".into()),
            fit: ImageFit::Cover,
        };
        let shared = shared_with(applied.clone());
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        evaluate(&config, &shared, must_not_load, &wake);
        let state = shared.0.lock().unwrap();
        assert_eq!(state.applied, applied);
        assert!(state.pending.is_none());
        drop(state);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn a_new_image_updates_and_the_same_configuration_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("image.png");
        std::fs::write(&image, encode_png()).unwrap();
        let config = config_document(&directory, Some(&image), "contain");
        let applied = AppliedWallpaper {
            path: None,
            fit: ImageFit::Cover,
        };
        let shared = shared_with(applied);
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        evaluate(&config, &shared, wallpaper::load, &wake);
        let mut state = shared.0.lock().unwrap();
        assert_eq!(state.applied.path.as_deref(), Some(image.to_str().unwrap()));
        assert!(matches!(state.applied.fit, ImageFit::Contain));
        match state.pending.take() {
            Some(ReloadUpdate::Image { fit, image }) => {
                assert!(matches!(fit, ImageFit::Contain));
                assert!(image.is_some());
            }
            other => panic!("expected an image update: {other:?}"),
        }
        drop(state);
        assert!(receiver.try_recv().is_ok());
        // Evaluating the unchanged configuration again stages nothing.
        evaluate(&config, &shared, wallpaper::load, &wake);
        let state = shared.0.lock().unwrap();
        assert!(state.pending.is_none());
        drop(state);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn a_fit_change_does_not_decode() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("image.png");
        std::fs::write(&image, encode_png()).unwrap();
        let config = config_document(&directory, Some(&image), "stretch");
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        let shared = shared_with(AppliedWallpaper {
            path: Some(image.to_string_lossy().into_owned()),
            fit: ImageFit::Cover,
        });
        evaluate(&config, &shared, must_not_load, &wake);
        let state = shared.0.lock().unwrap();
        assert!(matches!(
            state.pending,
            Some(ReloadUpdate::FitChange { .. })
        ));
        assert!(matches!(state.applied.fit, ImageFit::Stretch));
        drop(state);
        assert!(receiver.try_recv().is_ok());
    }

    #[test]
    fn a_broken_image_keeps_the_previous_image() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("broken.png");
        std::fs::write(&image, b"broken").unwrap();
        let config = config_document(&directory, Some(&image), "cover");
        let applied = AppliedWallpaper {
            path: Some("kept.png".into()),
            fit: ImageFit::Stretch,
        };
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        let shared = shared_with(applied.clone());
        evaluate(&config, &shared, failing_load, &wake);
        let state = shared.0.lock().unwrap();
        assert_eq!(state.applied, applied);
        assert!(state.pending.is_none());
        drop(state);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn clearing_renders_black_once() {
        let directory = tempfile::tempdir().unwrap();
        let config = config_document(&directory, None, "cover");
        let (notify, receiver) = mpsc::sync_channel::<()>(1);
        let wake = || {
            let _ = notify.try_send(());
        };
        let shared = shared_with(AppliedWallpaper {
            path: Some("current.png".into()),
            fit: ImageFit::Cover,
        });
        evaluate(&config, &shared, must_not_load, &wake);
        let mut state = shared.0.lock().unwrap();
        assert!(state.applied.path.is_none());
        match state.pending.take() {
            Some(ReloadUpdate::Image { image, .. }) => assert!(image.is_none()),
            other => panic!("expected a clear update: {other:?}"),
        }
        drop(state);
        assert!(receiver.try_recv().is_ok());
        // Clearing again is a no-op.
        evaluate(&config, &shared, must_not_load, &wake);
        assert!(shared.0.lock().unwrap().pending.is_none());
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }
}
