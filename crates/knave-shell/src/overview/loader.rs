//! One bounded worker for catalog and icon loading, off the Wayland input and frame path.
//!
//! The `Overview` owns the worker: dropping it closes the job queue, which ends the
//! thread after its current job. Jobs are capped by the queue size and deduplicated by the caller, so a
//! burst of typing cannot create more than `QUEUE` outstanding jobs.
use super::icons::{FALLBACK, RASTER_PX};
use knave_apps::{Catalog, CatalogError, Environment};
use knave_icons::IconLookup;
use knave_ui::UiImage;
use knave_wayland::Waker;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
};

const QUEUE: usize = 32;

pub(crate) enum Job {
    Catalog(Environment),
    Icon(String),
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
}

impl Loader {
    pub fn start(lookup: IconLookup, waker: Option<Waker>) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<Job>(QUEUE);
        let done = Arc::new(Mutex::new(VecDeque::new()));
        let results = done.clone();
        let thread = thread::spawn(move || {
            let mut lookup = lookup;
            let mut fallback: Option<Option<UiImage>> = None;
            while let Ok(job) = receiver.recv() {
                let finished = match job {
                    Job::Catalog(env) => Done::Catalog(Catalog::load_with(&env)),
                    Job::Icon(name) => {
                        let image = icon(&mut lookup, &mut fallback, &name);
                        Done::Icon(name, image)
                    }
                };
                results
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(finished);
                if let Some(waker) = &waker {
                    waker.wake();
                }
            }
        });
        Self {
            jobs: Some(jobs),
            done,
            thread: Some(thread),
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
        // Closing the queue ends the worker after its current job. Never join a
        // running worker: a stuck filesystem must not be able to hang shutdown.
        self.jobs.take();
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
