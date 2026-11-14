//! Screenshot notifications delivered on the session bus.
//!
//! Delivery stays off the compositor loop. One lazily started worker owns a
//! bounded request slot, and one signal watcher turns the default notification
//! action into `xdg-open`. Clipboard captures have no file to open, so their
//! notification is preview-only.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use futures_lite::StreamExt;
use zbus::{
    Connection, MatchRule, MessageStream,
    zvariant::{Array, Structure, Value},
};

const APP_NAME: &str = "Knave";
const DESKTOP_ENTRY: &str = "org.knave.Screenshot";
const NOTIFICATION_INTERFACE: &str = "org.freedesktop.Notifications";
const NOTIFICATION_PATH: &str = "/org/freedesktop/Notifications";
/// The notification service caps decoded previews at this edge.
const PREVIEW_EDGE: u32 = 320;
const STOP_POLL: Duration = Duration::from_millis(500);
/// Bounds paths retained for live notifications if a daemon never closes them.
const MAX_PENDING_ACTIONS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Preview {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub(crate) struct ScreenshotNotice {
    saved: Option<OpenTarget>,
    width: u32,
    height: u32,
    preview: Option<Preview>,
}

struct OpenTarget {
    path: PathBuf,
    environment: BTreeMap<String, String>,
}

impl ScreenshotNotice {
    pub(crate) fn saved(
        path: PathBuf,
        width: u32,
        height: u32,
        pixels: &[u8],
        environment: BTreeMap<String, String>,
    ) -> Self {
        Self {
            saved: Some(OpenTarget { path, environment }),
            width,
            height,
            preview: preview(width, height, pixels),
        }
    }

    pub(crate) fn copied(width: u32, height: u32, pixels: &[u8]) -> Self {
        Self {
            saved: None,
            width,
            height,
            preview: preview(width, height, pixels),
        }
    }

    fn summary(&self) -> &'static str {
        if self.saved.is_some() {
            "Screenshot saved"
        } else {
            "Screenshot copied"
        }
    }

    fn body(&self) -> String {
        match &self.saved {
            Some(target) => format!(
                "{}\n{} × {} pixels",
                target.path.display(),
                self.width,
                self.height
            ),
            None => format!(
                "{} × {} pixels copied to the clipboard",
                self.width, self.height
            ),
        }
    }
}

/// One bounded request slot keeps rapid shortcuts from queueing unbounded work.
#[derive(Default)]
pub(crate) struct ScreenshotNotifier {
    request: Option<SyncSender<ScreenshotNotice>>,
    stop: Arc<AtomicBool>,
    actions: Arc<Mutex<BTreeMap<u32, OpenTarget>>>,
    worker: Option<JoinHandle<()>>,
}

impl ScreenshotNotifier {
    pub(crate) fn submit(&mut self, notice: ScreenshotNotice) {
        self.start();
        let Some(sender) = &self.request else {
            return;
        };
        match sender.try_send(notice) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::debug!("screenshot notification queue is full; dropping request");
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("screenshot notification worker is unavailable");
            }
        }
    }

    fn start(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let stop = Arc::clone(&self.stop);
        let actions = Arc::clone(&self.actions);
        match thread::Builder::new()
            .name("villain-screenshot-notifications".into())
            .spawn(move || run(receiver, stop, actions))
        {
            Ok(worker) => {
                self.request = Some(sender);
                self.worker = Some(worker);
            }
            Err(error) => {
                tracing::warn!(%error, "could not start screenshot notification worker");
            }
        }
    }
}

impl Drop for ScreenshotNotifier {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.request.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    receiver: Receiver<ScreenshotNotice>,
    stop: Arc<AtomicBool>,
    actions: Arc<Mutex<BTreeMap<u32, OpenTarget>>>,
) {
    let mut connection: Option<Connection> = None;
    let mut watcher: Option<JoinHandle<()>> = None;
    let mut watcher_stop: Option<Arc<AtomicBool>> = None;
    loop {
        if watcher.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(finished) = watcher.take()
        {
            let _ = finished.join();
            watcher_stop.take();
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let notice = match receiver.recv() {
            Ok(notice) => notice,
            Err(_) => break,
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        let reconnect = connection
            .as_ref()
            .is_none_or(|connection| connection.is_closed());
        if reconnect {
            connection = match zbus::block_on(Connection::session()) {
                Ok(connection) => Some(connection),
                Err(error) => {
                    tracing::warn!(%error, "could not connect to the session bus for screenshots");
                    continue;
                }
            };
        }
        let Some(connection) = connection.as_ref() else {
            continue;
        };
        if reconnect || watcher.is_none() {
            if let Some(previous_stop) = watcher_stop.take() {
                previous_stop.store(true, Ordering::Release);
            }
            if let Some(previous) = watcher.take() {
                let _ = previous.join();
            }
            let watcher_connection = connection.clone();
            let watcher_global_stop = Arc::clone(&stop);
            let watcher_own_stop = Arc::new(AtomicBool::new(false));
            let watcher_actions = Arc::clone(&actions);
            let own_stop = Arc::clone(&watcher_own_stop);
            match thread::Builder::new()
                .name("villain-screenshot-actions".into())
                .spawn(move || {
                    watch_actions(
                        watcher_connection,
                        watcher_global_stop,
                        own_stop,
                        watcher_actions,
                    )
                }) {
                Ok(started) => {
                    watcher = Some(started);
                    watcher_stop = Some(watcher_own_stop);
                }
                Err(error) => {
                    tracing::warn!(%error, "could not start the screenshot action watcher");
                }
            }
        }
        let id = match zbus::block_on(send(connection, &notice)) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(%error, "could not show the screenshot notification");
                continue;
            }
        };
        if let Some(target) = notice.saved
            && let Ok(mut pending) = actions.lock()
        {
            pending.insert(id, target);
            while pending.len() > MAX_PENDING_ACTIONS {
                pending.pop_first();
            }
        }
    }
    stop.store(true, Ordering::Release);
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
}

async fn send(connection: &Connection, notice: &ScreenshotNotice) -> zbus::Result<u32> {
    let body = notice.body();
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(DESKTOP_ENTRY));
    if let Some(target) = &notice.saved {
        hints.insert(
            "image-path",
            Value::from(target.path.to_string_lossy().into_owned()),
        );
    }
    // Raw pixels make the preview independent of file size and decode limits;
    // saved captures additionally point the daemon at the original file.
    if let Some(preview) = &notice.preview {
        hints.insert(
            "image-data",
            Value::from(Structure::from((
                preview.width as i32,
                preview.height as i32,
                (preview.width * 4) as i32,
                true,
                8,
                4,
                Array::from(preview.pixels.clone()),
            ))),
        );
    }
    let (actions, timeout) = if notice.saved.is_some() {
        // Keep the action available in history instead of expiring it with the
        // banner. "default" also lets daemons open on a banner click.
        (vec!["default", "Open"], 0i32)
    } else {
        (Vec::new(), -1i32)
    };
    let reply = connection
        .call_method(
            Some(NOTIFICATION_INTERFACE),
            NOTIFICATION_PATH,
            Some(NOTIFICATION_INTERFACE),
            "Notify",
            &(
                APP_NAME,
                0u32,
                "",
                notice.summary(),
                body.as_str(),
                actions,
                hints,
                timeout,
            ),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

fn watch_actions(
    connection: Connection,
    stop: Arc<AtomicBool>,
    own_stop: Arc<AtomicBool>,
    actions: Arc<Mutex<BTreeMap<u32, OpenTarget>>>,
) {
    zbus::block_on(async move {
        let Some(rule) = notification_rule() else {
            return;
        };
        let Ok(mut stream) = MessageStream::for_match_rule(rule, &connection, Some(32)).await
        else {
            return;
        };
        loop {
            if stop.load(Ordering::Acquire) || own_stop.load(Ordering::Acquire) {
                return;
            }
            let event = futures_lite::future::or(async { stream.next().await }, async {
                async_io::Timer::after(STOP_POLL).await;
                None
            })
            .await;
            match event {
                Some(Ok(message)) => {
                    let header = message.header();
                    let member = header.member().map(|name| name.as_str());
                    let Ok(mut pending) = actions.lock() else {
                        return;
                    };
                    if member == Some("NotificationClosed") {
                        if let Ok((id, _)) = message.body().deserialize::<(u32, u32)>() {
                            pending.remove(&id);
                        }
                    } else if member == Some("ActionInvoked")
                        && let Ok((id, action)) = message.body().deserialize::<(u32, String)>()
                        && (action == "default" || action == "open")
                        && let Some(target) = pending.remove(&id)
                    {
                        drop(pending);
                        open_path(target.path.as_path(), &target.environment);
                    }
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, "screenshot notification signal failed");
                    return;
                }
                None => {}
            }
        }
    });
}

fn notification_rule() -> Option<MatchRule<'static>> {
    let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal);
    let rule = rule.interface(NOTIFICATION_INTERFACE).ok()?;
    Some(rule.build())
}

fn open_path(path: &Path, environment: &BTreeMap<String, String>) {
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(path)
        // Match the environment Villain gives launched applications instead
        // of inheriting a host display or socket from the launching session.
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .envs(environment);
    // xdg-open only recognizes a fixed set of desktop names. Knave otherwise
    // falls through to its generic text-browser path; GNOME selects gio open.
    command.env("XDG_CURRENT_DESKTOP", "GNOME:Knave");
    match command.status() {
        Ok(status) if status.success() => {
            tracing::info!(path = %path.display(), "opened screenshot");
        }
        Ok(status) => {
            tracing::warn!(path = %path.display(), %status, "xdg-open exited unsuccessfully");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!("xdg-open is not installed");
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not run xdg-open");
        }
    }
}

/// Nearest-neighbour downscale is enough for a bounded notification preview.
fn preview(width: u32, height: u32, pixels: &[u8]) -> Option<Preview> {
    let expected = usize::try_from(width).ok()? * usize::try_from(height).ok()? * 4;
    if width == 0 || height == 0 || pixels.len() != expected {
        return None;
    }
    let longest = width.max(height);
    if longest <= PREVIEW_EDGE {
        return Some(Preview {
            width,
            height,
            pixels: pixels.to_vec(),
        });
    }
    let scale = f64::from(longest) / f64::from(PREVIEW_EDGE);
    let target_width = ((f64::from(width) / scale).round() as u32).clamp(1, PREVIEW_EDGE);
    let target_height = ((f64::from(height) / scale).round() as u32).clamp(1, PREVIEW_EDGE);
    let mut output =
        Vec::with_capacity(usize::try_from(target_width * target_height * 4).unwrap_or_default());
    for y in 0..target_height {
        let source_y = (u64::from(y) * u64::from(height) / u64::from(target_height)) as usize;
        for x in 0..target_width {
            let source_x = (u64::from(x) * u64::from(width) / u64::from(target_width)) as usize;
            let offset = (source_y * usize::try_from(width).ok()? + source_x) * 4;
            output.extend_from_slice(&pixels[offset..offset + 4]);
        }
    }
    Some(Preview {
        width: target_width,
        height: target_height,
        pixels: output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_small_pixels_and_bound_large_edges() {
        let small = preview(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        assert_eq!(small.width, 2);
        assert_eq!(small.height, 1);
        assert_eq!(small.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);

        let pixels = vec![9_u8; 4000 * 3000 * 4];
        let large = preview(4000, 3000, &pixels).unwrap();
        assert_eq!((large.width, large.height), (320, 240));
        assert_eq!(large.pixels.len(), 320 * 240 * 4);
        assert!(large.pixels.iter().all(|pixel| *pixel == 9));
    }

    #[test]
    fn previews_reject_inconsistent_buffers() {
        assert!(preview(0, 1, &[]).is_none());
        assert!(preview(2, 2, &[0; 4]).is_none());
    }

    #[test]
    fn saved_notices_carry_the_path_and_clipboard_notices_do_not() {
        let path = PathBuf::from("/home/test/Pictures/Screenshots/Screenshot-1.png");
        let saved = ScreenshotNotice::saved(path.clone(), 2, 1, &[0; 8], BTreeMap::new());
        assert_eq!(saved.summary(), "Screenshot saved");
        assert!(saved.body().contains(path.to_string_lossy().as_ref()));
        assert!(saved.body().contains("2 × 1 pixels"));

        let copied = ScreenshotNotice::copied(2, 1, &[0; 8]);
        assert_eq!(copied.summary(), "Screenshot copied");
        assert!(copied.body().contains("copied to the clipboard"));
    }
}
