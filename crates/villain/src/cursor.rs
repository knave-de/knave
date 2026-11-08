//! Shared software cursor policy and rendering for every backend.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::GlesRenderer,
        },
    },
    desktop::utils::send_frames_surface_tree,
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    utils::{Point, Transform},
    wayland::compositor::with_states,
};
use std::{collections::HashMap, fs, time::Duration};

smithay::backend::renderer::element::render_elements! {
    pub CursorRenderElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

struct CursorFrame {
    buffer: MemoryRenderBuffer,
    hotspot: Point<i32, smithay::utils::Logical>,
    delay: Duration,
}

pub struct CursorState {
    image: CursorImageStatus,
    split_override: bool,
    interaction_override: Option<CursorIcon>,
    screenshot_override: bool,
    image_since: Duration,
    theme: xcursor::CursorTheme,
    size: u32,
    named: HashMap<CursorIcon, Vec<CursorFrame>>,
    fallback: CursorFrame,
}

impl CursorState {
    pub fn new() -> Self {
        let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|size| *size > 0)
            .unwrap_or(24);
        Self {
            image: CursorImageStatus::default_named(),
            split_override: false,
            interaction_override: None,
            screenshot_override: false,
            image_since: Duration::ZERO,
            theme: xcursor::CursorTheme::load(&theme_name),
            size,
            named: HashMap::new(),
            fallback: fallback_cursor(),
        }
    }

    pub(crate) fn set_interaction_override(&mut self, icon: Option<CursorIcon>) {
        self.interaction_override = icon;
    }
    pub fn set_split_override(&mut self, active: bool) {
        self.split_override = active;
    }

    pub fn set_screenshot_override(&mut self, active: bool) {
        self.screenshot_override = active;
    }

    fn effective_image(&self) -> CursorImageStatus {
        if self.screenshot_override {
            CursorImageStatus::Named(CursorIcon::Crosshair)
        } else if let Some(icon) = self.interaction_override {
            CursorImageStatus::Named(icon)
        } else if self.split_override {
            CursorImageStatus::Named(CursorIcon::ColResize)
        } else {
            self.image.clone()
        }
    }

    pub fn set_image(&mut self, image: CursorImageStatus, now: Duration) {
        if self.image != image {
            self.image = image;
            self.image_since = now;
        }
    }

    pub fn render_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        location: Point<f64, smithay::utils::Logical>,
        now: Duration,
        output_scale: f64,
    ) -> Vec<CursorRenderElement> {
        match self.effective_image() {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .and_then(|attributes| attributes.lock().ok().map(|value| value.hotspot))
                        .unwrap_or_default()
                });
                render_elements_from_surface_tree(
                    renderer,
                    &surface,
                    (location.to_i32_round() - hotspot).to_physical_precise_round(output_scale),
                    output_scale,
                    1.0,
                    Kind::Cursor,
                )
            }
            CursorImageStatus::Named(icon) => {
                let elapsed = now.saturating_sub(self.image_since);
                if !self.named.contains_key(&icon) {
                    let frames = load_named_cursor(&self.theme, icon, self.size);
                    self.named.insert(icon, frames);
                }
                let frames = self.named.get(&icon).unwrap();
                let frame = frame_at(frames, elapsed).unwrap_or(&self.fallback);
                let render_location: Point<i32, smithay::utils::Physical> =
                    (location.to_i32_round() - frame.hotspot)
                        .to_physical_precise_round(output_scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    render_location.to_f64(),
                    &frame.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(element) => vec![element.into()],
                    Err(error) => {
                        tracing::warn!(%error, "could not upload cursor image");
                        Vec::new()
                    }
                }
            }
        }
    }

    pub fn send_frame(&self, output: &Output, now: Duration) {
        if let CursorImageStatus::Surface(surface) = &self.image {
            send_frames_surface_tree(surface, output, now, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
    }

    pub fn uses_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> bool {
        matches!(&self.image, CursorImageStatus::Surface(cursor) if cursor == surface)
    }

    pub fn next_animation_delay(&self, now: Duration) -> Option<Duration> {
        let CursorImageStatus::Named(icon) = self.effective_image() else {
            return None;
        };
        let frames = self.named.get(&icon)?;
        if frames.len() < 2 {
            return None;
        }
        frame_delay_at(frames, now.saturating_sub(self.image_since))
    }
}

fn load_named_cursor(
    theme: &xcursor::CursorTheme,
    icon: CursorIcon,
    requested_size: u32,
) -> Vec<CursorFrame> {
    let path = std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .find_map(|name| theme.load_icon(name));
    let Some(path) = path else {
        tracing::warn!(cursor = %icon, "cursor shape is missing from theme");
        return Vec::new();
    };
    let Some(images) = fs::read(&path)
        .ok()
        .and_then(|contents| xcursor::parser::parse_xcursor(&contents))
    else {
        tracing::warn!(cursor = %icon, path = %path.display(), "could not parse cursor image");
        return Vec::new();
    };
    let Some(best_size) = images
        .iter()
        .map(|image| image.size)
        .min_by_key(|size| size.abs_diff(requested_size))
    else {
        return Vec::new();
    };
    images
        .into_iter()
        .filter(|image| image.size == best_size)
        .map(|image| CursorFrame {
            buffer: MemoryRenderBuffer::from_slice(
                &image.pixels_rgba,
                Fourcc::Argb8888,
                (image.width as i32, image.height as i32),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: (image.xhot as i32, image.yhot as i32).into(),
            delay: Duration::from_millis(u64::from(image.delay.max(1))),
        })
        .collect()
}

fn frame_at(frames: &[CursorFrame], elapsed: Duration) -> Option<&CursorFrame> {
    let total: Duration = frames.iter().map(|frame| frame.delay).sum();
    if total.is_zero() {
        return frames.first();
    }
    let mut offset = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
    frames.iter().find(|frame| {
        if offset < frame.delay {
            true
        } else {
            offset -= frame.delay;
            false
        }
    })
}

fn frame_delay_at(frames: &[CursorFrame], elapsed: Duration) -> Option<Duration> {
    let total: Duration = frames.iter().map(|frame| frame.delay).sum();
    if total.is_zero() {
        return None;
    }
    let mut offset = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
    for frame in frames {
        if offset < frame.delay {
            return Some(frame.delay - offset);
        }
        offset -= frame.delay;
    }
    None
}

fn fallback_cursor() -> CursorFrame {
    const WIDTH: usize = 16;
    const HEIGHT: usize = 24;
    let mut pixels = vec![0; WIDTH * HEIGHT * 4];
    for y in 0..18 {
        for x in 0..=y / 2 {
            let pixel = (y * WIDTH + x) * 4;
            let border = x == 0 || x == y / 2 || y == 17;
            pixels[pixel..pixel + 4].copy_from_slice(if border {
                &[0, 0, 0, 255]
            } else {
                &[255, 255, 255, 255]
            });
        }
    }
    CursorFrame {
        buffer: MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (WIDTH as i32, HEIGHT as i32),
            1,
            Transform::Normal,
            None,
        ),
        hotspot: (0, 0).into(),
        delay: Duration::from_millis(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_selects_frames_by_delay() {
        let frames = [fallback_cursor(), fallback_cursor()];
        assert!(std::ptr::eq(
            frame_at(&frames, Duration::ZERO).unwrap(),
            &frames[0]
        ));
        assert!(std::ptr::eq(
            frame_at(&frames, Duration::from_millis(1)).unwrap(),
            &frames[1]
        ));
        assert!(std::ptr::eq(
            frame_at(&frames, Duration::from_millis(2)).unwrap(),
            &frames[0]
        ));
        assert_eq!(
            frame_delay_at(&frames, Duration::ZERO),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            frame_delay_at(&frames, Duration::from_micros(1500)),
            Some(Duration::from_micros(500))
        );
    }
}
