use knave_desktop_api::{DesktopEvent, DesktopSubscription, SubscriptionCancel};
use knave_wayland::Waker;
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

struct State {
    pins: Option<Vec<String>>,
    stopped: bool,
    cancel: Option<SubscriptionCancel>,
}
pub(super) struct PinsReload {
    state: Arc<Mutex<State>>,
    retry: Arc<Condvar>,
    thread: Option<JoinHandle<()>>,
}

impl PinsReload {
    pub fn start(waker: Waker) -> Self {
        let state = Arc::new(Mutex::new(State {
            pins: None,
            stopped: false,
            cancel: None,
        }));
        let retry = Arc::new(Condvar::new());
        let worker_state = state.clone();
        let worker_retry = retry.clone();
        let thread = thread::Builder::new()
            .name("knave-pins-reload".into())
            .spawn(move || {
                match knave_config::ConfigDocument::at_default_path() {
                    Ok(document) => {
                        worker_state.lock().unwrap().pins =
                            Some(document.config().shell.pinned_apps.clone());
                        waker.wake();
                    }
                    Err(error) => eprintln!("knave-shell: pinned apps load failed: {error}"),
                }
                let mut backoff = Duration::from_millis(250);
                loop {
                    if worker_state.lock().unwrap().stopped {
                        break;
                    }
                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let mut subscription = DesktopSubscription::connect()?;
                        let cancel = subscription.cancel_handle()?;
                        {
                            let mut s = worker_state.lock().unwrap();
                            if s.stopped {
                                let _ = cancel.cancel();
                                return Ok(());
                            }
                            s.cancel = Some(cancel);
                        }
                        loop {
                            if matches!(
                                subscription.next_event()?,
                                DesktopEvent::ConfigurationReloaded
                            ) {
                                match knave_config::ConfigDocument::at_default_path() {
                                    Ok(document) => {
                                        let pins = document.config().shell.pinned_apps.clone();
                                        worker_state.lock().unwrap().pins = Some(pins);
                                        waker.wake();
                                    }
                                    Err(error) => {
                                        eprintln!("knave-shell: pinned apps reload failed: {error}")
                                    }
                                }
                            }
                        }
                    })();
                    let mut s = worker_state.lock().unwrap();
                    s.cancel = None;
                    if s.stopped {
                        break;
                    }
                    if let Err(error) = result {
                        eprintln!("knave-shell: pinned apps subscription unavailable: {error}");
                    }
                    let (s, _) = worker_retry.wait_timeout(s, backoff).unwrap();
                    if s.stopped {
                        break;
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            })
            .expect("pinned apps reload thread spawns");
        Self {
            state,
            retry,
            thread: Some(thread),
        }
    }
    pub fn take(&self) -> Option<Vec<String>> {
        self.state.lock().unwrap().pins.take()
    }
    pub fn has_update(&self) -> bool {
        self.state.lock().unwrap().pins.is_some()
    }
}
impl Drop for PinsReload {
    fn drop(&mut self) {
        let mut s = self.state.lock().unwrap();
        s.stopped = true;
        if let Some(cancel) = s.cancel.take() {
            let _ = cancel.cancel();
        }
        self.retry.notify_all();
        drop(s);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
