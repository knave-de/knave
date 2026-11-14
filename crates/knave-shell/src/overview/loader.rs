//! One bounded worker for catalog and icon loading, off the Wayland input and frame path.
//!
//! The `Overview` owns the worker: dropping it signals shutdown and wakes the
//! thread, which releases its filesystem watches after its current job. Jobs are
//! capped by the queue size and deduplicated by the caller, so a burst of typing
//! cannot create more than `QUEUE` outstanding jobs.
use super::{
    icons::{FALLBACK, RASTER_PX},
    watch::CatalogWatch,
};
use knave_apps::{Catalog, CatalogError, Environment};
use knave_icons::IconLookup;
use knave_ui::UiImage;
use knave_wayland::Waker;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const QUEUE: usize = 32;
const QUIET: Duration = Duration::from_millis(150);
const MAX_BURST: Duration = Duration::from_secs(1);

pub(crate) enum Job {
    Catalog(Environment),
    Icon(String),
    Changed,
}
pub(crate) enum Done {
    Catalog(Result<Catalog, CatalogError>),
    /// `None` means no icon, not even the fallback, could be produced.
    Icon(String, Option<UiImage>),
}

pub(crate) struct Loader {
    jobs: Option<SyncSender<Job>>,
    done: Arc<Mutex<VecDeque<Done>>>,
    thread: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
}

impl Loader {
    pub fn start(lookup: IconLookup, waker: Option<Waker>) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<Job>(QUEUE);
        let done = Arc::new(Mutex::new(VecDeque::new()));
        let results = done.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let changed = Arc::new(AtomicBool::new(false));
        let events = jobs.clone();
        let thread = thread::spawn(move || {
            let mut lookup = lookup;
            let mut fallback: Option<Option<UiImage>> = None;
            let mut environment = None;
            let mut watch = None;
            let mut deadline = None;
            let mut burst = None;
            while !stop.load(Ordering::Acquire) {
                if changed.swap(false, Ordering::AcqRel) {
                    let now = Instant::now();
                    let start = *burst.get_or_insert(now);
                    deadline = Some((now + QUIET).min(start + MAX_BURST));
                }
                let job = if let Some(until) = deadline {
                    if Instant::now() >= until {
                        Err(RecvTimeoutError::Timeout)
                    } else {
                        receiver.recv_timeout(until.saturating_duration_since(Instant::now()))
                    }
                } else {
                    receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
                };
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let finished = match job {
                    Ok(Job::Catalog(env)) => {
                        if watch.is_none() {
                            let dirty = changed.clone();
                            let events = events.clone();
                            match CatalogWatch::new(&env, move || {
                                if !dirty.swap(true, Ordering::AcqRel) {
                                    let _ = events.try_send(Job::Changed);
                                }
                            }) {
                                Ok(created) => watch = Some(created),
                                Err(error) => {
                                    eprintln!("knave-shell: cannot monitor applications: {error}")
                                }
                            }
                        }
                        let catalog = Catalog::load_with(&env);
                        environment = Some(env);
                        Done::Catalog(catalog)
                    }
                    Ok(Job::Icon(name)) => {
                        let image = icon(&mut lookup, &mut fallback, &name);
                        Done::Icon(name, image)
                    }
                    Ok(Job::Changed) => continue,
                    Err(RecvTimeoutError::Timeout) => {
                        deadline = None;
                        burst = None;
                        let Some(env) = &environment else { continue };
                        if let Some(watch) = &mut watch {
                            watch.refresh(env);
                        }
                        Done::Catalog(Catalog::load_with(env))
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let mut done = results.lock().unwrap_or_else(PoisonError::into_inner);
                // Hidden surfaces need only the newest catalog, however many installs occur.
                if matches!(&finished, Done::Catalog(_)) {
                    done.retain(|result| !matches!(result, Done::Catalog(_)));
                }
                done.push_back(finished);
                drop(done);
                if let Some(waker) = &waker {
                    waker.wake();
                }
            }
        });
        Self {
            jobs: Some(jobs),
            done,
            thread: Some(thread),
            stopping,
        }
    }

    /// Queue a job without blocking. False means the queue is full; retry after results arrive.
    pub fn submit(&self, job: Job) -> bool {
        self.jobs
            .as_ref()
            .is_some_and(|jobs| jobs.try_send(job).is_ok())
    }

    pub fn take(&self) -> Vec<Done> {
        self.done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    pub fn has_results(&self) -> bool {
        !self
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        // Signal and wake even though the watcher holds a queue sender. Never join a
        // running worker: a stuck filesystem must not be able to hang shutdown.
        self.stopping.store(true, Ordering::Release);
        if let Some(jobs) = self.jobs.take() {
            let _ = jobs.try_send(Job::Changed);
        }
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
    }
}

fn icon(
    lookup: &mut IconLookup,
    fallback: &mut Option<Option<UiImage>>,
    name: &str,
) -> Option<UiImage> {
    load(lookup, name).or_else(|| {
        if name == FALLBACK {
            return None;
        }
        fallback
            .get_or_insert_with(|| load(lookup, FALLBACK))
            .clone()
    })
}

fn load(lookup: &mut IconLookup, name: &str) -> Option<UiImage> {
    match lookup.load(name, RASTER_PX) {
        Ok(Some(icon)) => UiImage::from_rgba(icon.width, icon.height, icon.rgba),
        Ok(None) => None,
        Err(error) => {
            eprintln!("knave-shell: {error}");
            None
        }
    }
}

#[cfg(test)]
#[path = "loader_tests.rs"]
mod tests;
