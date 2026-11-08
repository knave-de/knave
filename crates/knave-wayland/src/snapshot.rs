//! Blocking state subscription; only disconnected clients use a retry timer.
use super::{RuntimeWake, WakeSender};
use knave_desktop_api::{DesktopSnapshot, DesktopSubscription, SubscriptionCancel};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
#[derive(Default)]
struct Control {
    stopped: bool,
    cancel: Option<SubscriptionCancel>,
}
pub(super) struct SnapshotWorker {
    control: Arc<(Mutex<Control>, Condvar)>,
    updates: Arc<Mutex<Option<Result<DesktopSnapshot, String>>>>,
    thread: Option<JoinHandle<()>>,
}
impl SnapshotWorker {
    pub(super) fn start(wake: WakeSender) -> Self {
        let control = Arc::new((Mutex::new(Control::default()), Condvar::new()));
        let updates = Arc::new(Mutex::new(None));
        let thread_control = control.clone();
        let thread_updates = updates.clone();
        let thread = thread::spawn(move || {
            let (control, retry) = &*thread_control;
            let mut backoff = INITIAL_BACKOFF;
            let mut last_error = None;
            loop {
                if control.lock().unwrap().stopped {
                    break;
                }
                let result: Result<(), knave_desktop_api::ClientError> = (|| {
                    let mut subscription = DesktopSubscription::connect()?;
                    let cancel = subscription.cancel_handle()?;
                    {
                        let mut state = control.lock().unwrap();
                        if state.stopped {
                            let _ = cancel.cancel();
                            return Ok(());
                        }
                        state.cancel = Some(cancel);
                    }
                    loop {
                        let snapshot = subscription.next_snapshot()?;
                        backoff = INITIAL_BACKOFF;
                        last_error = None;
                        *thread_updates.lock().unwrap() = Some(Ok(snapshot));
                        let _ = wake.try_send(RuntimeWake::Redraw);
                    }
                })();
                let mut state = control.lock().unwrap();
                state.cancel = None;
                if state.stopped {
                    break;
                }
                if let Err(error) = result {
                    let message = format!("Desktop subscription unavailable: {error}");
                    if last_error.as_ref() != Some(&message) {
                        eprintln!("knave-shell: {message}");
                        *thread_updates.lock().unwrap() = Some(Err(message.clone()));
                        let _ = wake.try_send(RuntimeWake::Redraw);
                        last_error = Some(message);
                    }
                }
                let (state, _) = retry
                    .wait_timeout_while(state, backoff, |s| !s.stopped)
                    .unwrap();
                if state.stopped {
                    break;
                }
                backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
            }
        });
        Self {
            control,
            updates,
            thread: Some(thread),
        }
    }
    pub(super) fn latest(&self) -> Option<Result<DesktopSnapshot, String>> {
        self.updates.lock().unwrap().take()
    }
}
impl Drop for SnapshotWorker {
    fn drop(&mut self) {
        let (control, retry) = &*self.control;
        {
            let mut state = control.lock().unwrap();
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
