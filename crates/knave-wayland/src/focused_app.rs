//! Bounded, event-driven focused-application icon resolution.
use super::application::Waker;
use knave_apps::Catalog;
use knave_desktop_api::{DesktopSnapshot, WindowId, WorkspaceId};
use knave_icons::IconLookup;
use knave_ui::UiImage;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

const QUEUE: usize = 1;
const CACHE: usize = 64;
const ICON_SIZE: u32 = 48;
const FALLBACK: &str = "application-x-executable";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Request {
    pub token: u64,
    pub app_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResultIcon {
    pub token: u64,
    pub app_id: String,
    pub image: Option<UiImage>,
}

pub(super) type Identity = (WindowId, WorkspaceId, String);

pub(super) fn identity(snapshot: &DesktopSnapshot) -> Option<Identity> {
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.active)?
        .workspace;
    snapshot
        .windows
        .iter()
        .find(|window| window.focused && !window.minimized && window.workspace == workspace)
        .map(|window| (window.id, workspace, window.app_id.clone()))
}

pub(super) fn request(identity: Option<&Identity>, token: u64) -> Option<Request> {
    identity.map(|(_, _, app_id)| Request {
        token,
        app_id: app_id.clone(),
    })
}

pub(super) fn matches(identity: Option<&Identity>, token: u64, result: &ResultIcon) -> bool {
    token == result.token && identity.is_some_and(|(_, _, app_id)| app_id == &result.app_id)
}

#[derive(Default)]
struct Completion {
    closed: bool,
    result: Option<ResultIcon>,
}

pub(super) struct Worker {
    jobs: Option<SyncSender<Request>>,
    result: Arc<Mutex<Completion>>,
    current_token: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(waker: Waker) -> Self {
        let mut catalog: Option<Option<Catalog>> = None;
        let mut lookup = None;
        Self::start_with(waker, move |app_id| {
            let catalog = catalog.get_or_insert_with(|| Catalog::load().ok());
            let icon = catalog
                .as_ref()
                .and_then(|catalog| app_icon(catalog, app_id));
            let lookup = lookup.get_or_insert_with(IconLookup::from_process);
            icon_candidates(icon.as_deref())
                .iter()
                .find_map(|name| load(lookup, name))
                .or_else(|| Some(generic_icon()))
        })
    }

    fn start_with(
        waker: Waker,
        mut resolve: impl FnMut(&str) -> Option<UiImage> + Send + 'static,
    ) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<Request>(QUEUE);
        let result = Arc::new(Mutex::new(Completion::default()));
        let results = result.clone();
        let current_token = Arc::new(AtomicU64::new(0));
        let active_token = current_token.clone();
        let thread = thread::spawn(move || {
            let mut cache = HashMap::<String, Option<UiImage>>::new();
            let mut order = VecDeque::<String>::new();
            while let Ok(request) = receiver.recv() {
                if active_token.load(Ordering::Acquire) != request.token {
                    let completion = results.lock().unwrap_or_else(PoisonError::into_inner);
                    if completion.closed {
                        break;
                    }
                    waker.wake();
                    continue;
                }
                let image = if let Some(image) = cache.get(&request.app_id) {
                    image.clone()
                } else {
                    let image = resolve(&request.app_id);
                    if active_token.load(Ordering::Acquire) != request.token {
                        let completion = results.lock().unwrap_or_else(PoisonError::into_inner);
                        if completion.closed {
                            break;
                        }
                        waker.wake();
                        continue;
                    }
                    while order.len() >= CACHE {
                        if let Some(oldest) = order.pop_front() {
                            cache.remove(&oldest);
                        }
                    }
                    order.push_back(request.app_id.clone());
                    cache.insert(request.app_id.clone(), image.clone());
                    image
                };
                if active_token.load(Ordering::Acquire) != request.token {
                    let completion = results.lock().unwrap_or_else(PoisonError::into_inner);
                    if completion.closed {
                        break;
                    }
                    waker.wake();
                    continue;
                }
                let mut completion = results.lock().unwrap_or_else(PoisonError::into_inner);
                if completion.closed {
                    break;
                }
                completion.result = Some(ResultIcon {
                    token: request.token,
                    app_id: request.app_id,
                    image,
                });
                // A single slot means consumers cannot accumulate completed images.
                waker.wake();
            }
        });
        Self {
            jobs: Some(jobs),
            result,
            current_token,
            thread: Some(thread),
        }
    }

    pub fn cancel_before(&self, token: u64) {
        self.current_token.store(token, Ordering::Release);
    }

    pub fn submit(&self, request: Request) -> bool {
        self.jobs
            .as_ref()
            .is_some_and(|jobs| jobs.try_send(request).is_ok())
    }

    pub fn take(&self) -> Option<ResultIcon> {
        self.result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .result
            .take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Serialize shutdown with publication/waking without waiting for I/O.
        {
            let mut completion = self.result.lock().unwrap_or_else(PoisonError::into_inner);
            completion.closed = true;
            completion.result = None;
            self.current_token.fetch_add(1, Ordering::AcqRel);
        }
        self.jobs.take();
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
    }
}

fn load(lookup: &mut IconLookup, name: &str) -> Option<UiImage> {
    match lookup.load(name, ICON_SIZE) {
        Ok(Some(icon)) => UiImage::from_rgba(icon.width, icon.height, icon.rgba),
        Ok(None) => None,
        Err(error) => {
            eprintln!("knave-shell: {error}");
            None
        }
    }
}

fn app_icon(catalog: &Catalog, app_id: &str) -> Option<String> {
    let id = if app_id.ends_with(".desktop") {
        app_id.to_owned()
    } else {
        format!("{app_id}.desktop")
    };
    (0..catalog.len())
        .filter_map(|index| catalog.get(index))
        .find(|app| app.id == id)
        .and_then(|app| app.icon.clone())
}

fn icon_candidates(name: Option<&str>) -> Vec<String> {
    match name.filter(|name| !name.is_empty()) {
        Some(name) if name != FALLBACK => vec![name.to_owned(), FALLBACK.to_owned()],
        _ => vec![FALLBACK.to_owned()],
    }
}

fn generic_icon() -> UiImage {
    const SIZE: u32 = 48;
    let mut pixels = vec![0; (SIZE * SIZE * 4) as usize];
    for y in 8..39 {
        for x in 7..41 {
            let border = x == 7 || x == 40 || y == 8 || y == 38;
            let top_bar = (12..18).contains(&y) && (11..37).contains(&x);
            let screen = (21..34).contains(&y) && (11..37).contains(&x);
            if border || top_bar || screen {
                let color = if border || top_bar {
                    [145, 174, 205, 255]
                } else {
                    [65, 83, 105, 255]
                };
                let offset = ((y * SIZE + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
    UiImage::from_rgba(SIZE, SIZE, pixels).expect("valid generic icon raster")
}

#[cfg(test)]
mod tests {
    use super::*;
    use knave_desktop_api::{WindowSummary, WorkspaceSummary};

    fn snapshot() -> DesktopSnapshot {
        DesktopSnapshot {
            generation: 1,
            overview_visible: false,
            workspaces: vec![WorkspaceSummary {
                workspace: WorkspaceId(1),
                active: true,
                window_count: 1,
                visible_window_count: 1,
            }],
            windows: vec![WindowSummary {
                id: WindowId(7),
                title: "Editor — 文書 📄".into(),
                app_id: "org.example.Editor".into(),
                workspace: WorkspaceId(1),
                focused: true,
                minimized: false,
                floating: false,
                fullscreen: false,
                maximized: false,
            }],
        }
    }

    #[test]
    fn identity_requires_a_focused_non_minimized_window_on_the_active_workspace() {
        let mut state = snapshot();
        let expected = Some((WindowId(7), WorkspaceId(1), "org.example.Editor".into()));
        assert_eq!(identity(&state), expected);
        state.windows[0].minimized = true;
        assert_eq!(identity(&state), None);
        state.windows[0].minimized = false;
        state.workspaces[0].active = false;
        assert_eq!(identity(&state), None);
        state.workspaces[0].active = true;
        state.windows[0].workspace = WorkspaceId(2);
        assert_eq!(identity(&state), None);
    }

    #[test]
    fn late_results_are_rejected_after_focus_change_or_disconnect() {
        let first = identity(&snapshot()).unwrap();
        let result = ResultIcon {
            token: 3,
            app_id: first.2.clone(),
            image: None,
        };
        assert!(matches(Some(&first), 3, &result));
        assert!(!matches(Some(&first), 4, &result));
        assert!(!matches(None, 3, &result));
        let mut next = snapshot();
        next.windows[0].id = WindowId(8);
        next.windows[0].app_id = "org.example.Other".into();
        assert!(!matches(identity(&next).as_ref(), 4, &result));
    }

    #[test]
    fn empty_app_id_keeps_focus_and_requests_fallback() {
        let mut state = snapshot();
        state.windows[0].app_id.clear();
        let key = identity(&state);
        assert_eq!(
            request(key.as_ref(), 1),
            Some(Request {
                token: 1,
                app_id: String::new()
            })
        );
        assert_eq!(request(None, 2), None);
        assert_eq!(icon_candidates(None), vec![FALLBACK.to_owned()]);
    }

    #[test]
    fn drop_cancels_queued_lookup_and_suppresses_late_results_and_wakes() {
        use std::time::Duration;
        let (wake, wakes) = super::super::channel::sync_channel(1);
        let (started, starts) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let mut worker = Worker::start_with(Waker::new(wake), move |app_id| {
            started.send(app_id.to_owned()).unwrap();
            blocked.recv().unwrap();
            Some(generic_icon())
        });
        worker.cancel_before(1);
        assert!(worker.submit(Request {
            token: 1,
            app_id: String::new()
        }));
        assert_eq!(starts.recv_timeout(Duration::from_secs(2)).unwrap(), "");
        assert!(worker.submit(Request {
            token: 1,
            app_id: "queued".into()
        }));
        assert!(!worker.submit(Request {
            token: 1,
            app_id: "overflow".into()
        }));
        let completion = worker.result.clone();
        // Retain the handle only to synchronize the test after nonblocking Drop.
        let thread = worker.thread.take().unwrap();
        drop(worker);
        assert!(completion.lock().unwrap().closed);
        release.send(()).unwrap();
        drop(release);
        thread.join().unwrap();
        assert!(starts.try_recv().is_err(), "queued lookup must not run");
        assert!(completion.lock().unwrap().result.is_none());
        assert!(
            wakes.try_recv().is_err(),
            "shutdown must not wake the old host"
        );
    }

    #[test]
    fn unknown_app_ids_resolve_to_the_generic_icon_candidate() {
        assert_eq!(icon_candidates(None), vec![FALLBACK.to_owned()]);
        assert_eq!(
            icon_candidates(Some("editor-icon")),
            vec!["editor-icon".to_owned(), FALLBACK.to_owned()]
        );
        assert_eq!(icon_candidates(Some("")), vec![FALLBACK.to_owned()]);
        let fallback = generic_icon();
        assert_eq!((fallback.width(), fallback.height()), (48, 48));
    }
}
