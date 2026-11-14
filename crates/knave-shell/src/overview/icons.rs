//! Bounded cache of finished icons; decoding happens on the loader thread.
use knave_ui::UiImage;
use std::collections::{HashMap, HashSet, VecDeque};

/// Rasterized at twice the 32px row icon so it stays sharp at scale 2.
pub(crate) const RASTER_PX: u32 = 64;
const MAX_CACHED: usize = 128;
pub(crate) const FALLBACK: &str = "application-x-executable";

#[derive(Default)]
pub(crate) struct Icons {
    /// `None` records a name with no usable icon so it is not requested again.
    cache: HashMap<String, Option<UiImage>>,
    order: VecDeque<String>,
    in_flight: HashSet<String>,
}

impl Icons {
    pub fn get(&self, name: &str) -> Option<&UiImage> {
        self.cache.get(name)?.as_ref()
    }

    pub fn needs_request(&self, name: &str) -> bool {
        !self.cache.contains_key(name) && !self.in_flight.contains(name)
    }

    /// Record a request that the loader accepted, so it is not queued twice.
    pub fn requested(&mut self, name: &str) {
        self.in_flight.insert(name.to_owned());
    }

    pub fn finish(&mut self, name: String, image: Option<UiImage>) {
        self.in_flight.remove(&name);
        while self.order.len() >= MAX_CACHED {
            if let Some(oldest) = self.order.pop_front() {
                self.cache.remove(&oldest);
            }
        }
        if self.cache.insert(name.clone(), image).is_none() {
            self.order.push_back(name);
        }
    }

    pub fn is_loading(&self) -> bool {
        !self.in_flight.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cache.len()
    }
}
