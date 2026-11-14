//! Small software raster adapter for compositor titlebars. No WGPU renderer is used.
use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache};
use knave_ui::toolkit::{Element, ElementId, Scene, Widget};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    desktop::Window,
    utils::Transform,
};
use std::collections::VecDeque;
use std::time::Instant;

const SLOW_TITLEBAR_RASTER_US: u128 = 1_000;

fn bgra(rgba: [u8; 4]) -> [u8; 4] {
    [rgba[2], rgba[1], rgba[0], rgba[3]]
}

fn stroke_segment(pixels: &mut [u8], width: i32, color: [u8; 4], a: (f32, f32), b: (f32, f32)) {
    // Four subpixel samples keep the small titlebar glyphs crisp without a
    // texture dependency or changing the titlebar's memory-buffer format.
    const SAMPLES: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let radius = 0.8f32;
    let min_x = (a.0.min(b.0) - radius - 1.0).floor().max(0.0) as i32;
    let max_x = (a.0.max(b.0) + radius + 1.0).ceil().min(width as f32) as i32;
    let min_y = (a.1.min(b.1) - radius - 1.0).floor().max(0.0) as i32;
    let max_y = (a.1.max(b.1) + radius + 1.0).ceil().min(34.0) as i32;
    for y in min_y..max_y {
        for x in min_x..max_x {
            let covered = SAMPLES
                .iter()
                .filter(|&&(sx, sy)| {
                    let px = x as f32 + sx;
                    let py = y as f32 + sy;
                    let t = if len2 == 0.0 {
                        0.0
                    } else {
                        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
                    };
                    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
                    (px - qx).powi(2) + (py - qy).powi(2) <= radius * radius
                })
                .count() as u32;
            if covered == 0 {
                continue;
            }
            let i = (y as usize * width as usize + x as usize) * 4;
            let alpha = covered * u32::from(color[3]) / 4;
            for (channel, value) in [(2, color[0]), (1, color[1]), (0, color[2])] {
                pixels[i + channel] = ((u32::from(value) * alpha
                    + u32::from(pixels[i + channel]) * (255 - alpha))
                    / 255) as u8;
            }
        }
    }
}

fn stroke_path(
    pixels: &mut [u8],
    width: i32,
    color: [u8; 4],
    origin: (f32, f32),
    points: &[(f32, f32)],
) {
    for pair in points.windows(2) {
        let map = |(x, y): (f32, f32)| (origin.0 + x * 0.55, origin.1 + y * 0.55);
        stroke_segment(pixels, width, color, map(pair[0]), map(pair[1]));
    }
}

fn control_origin(width: i32, index: i32) -> (f32, f32) {
    let slot_left = width - 24 - index * 24;
    // The paths use a 24-unit viewBox but scale to 13.2px. Center that
    // smaller glyph inside the 24px hit and hover slot.
    (slot_left as f32 + (24.0 - 24.0 * 0.55) / 2.0, 10.4)
}

fn paint_hover(pixels: &mut [u8], width: i32, index: u8) {
    let right = width - i32::from(index) * 24;
    let color = if index == 0 {
        [167, 91, 91, 255]
    } else {
        [70, 77, 85, 255]
    };
    for y in 5..29 {
        for x in (right - 23).max(0)..right.min(width) {
            let i = (y as usize * width as usize + x as usize) * 4;
            pixels[i..i + 4].copy_from_slice(&bgra(color));
        }
    }
}

struct Entry {
    window_id: knave_desktop_api::WindowId,
    title: String,
    width: i32,
    focused: bool,
    hovered: Option<u8>,
    maximized: bool,
    tiled: bool,
    buffer: MemoryRenderBuffer,
}
#[derive(Clone, Copy)]
pub(super) struct TitlebarState {
    pub(super) focused: bool,
    pub(super) hovered: Option<u8>,
    pub(super) maximized: bool,
    pub(super) tiled: bool,
}
pub(super) struct Titlebars {
    fonts: FontSystem,
    glyphs: SwashCache,
    entries: VecDeque<Entry>,
    background: [u8; 4],
    foreground: [u8; 4],
}
impl Default for Titlebars {
    fn default() -> Self {
        let profile_font_init = tracing::enabled!(target: "villain::perf", tracing::Level::DEBUG);
        let font_init_started = profile_font_init.then(Instant::now);
        let fonts = FontSystem::new();
        if let Some(started) = font_init_started {
            let elapsed_us = started.elapsed().as_micros();
            tracing::debug!(
                target: "villain::perf",
                event = "titlebar_font_system_initialized",
                elapsed_us,
                slow = elapsed_us >= SLOW_TITLEBAR_RASTER_US,
                "titlebar font system initialized"
            );
        }
        let mut scene = Scene::new(Element::new(1, Widget::Panel)).expect("valid titlebar scene");
        scene
            .set_stylesheet("panel { background-color: var(--surface-hover); color: var(--text); }")
            .expect("valid titlebar style");
        let style = scene
            .computed_style(ElementId(1))
            .expect("titlebar style")
            .style;
        let bg = style
            .background
            .and_then(|b| b.fill)
            .expect("titlebar background");
        let fg = style.text.color;
        Self {
            fonts,
            glyphs: SwashCache::new(),
            entries: VecDeque::new(),
            background: [bg.red, bg.green, bg.blue, 255],
            foreground: [fg.red, fg.green, fg.blue, 255],
        }
    }
}
impl Titlebars {
    pub(super) fn buffer(
        &mut self,
        window_id: knave_desktop_api::WindowId,
        window: &Window,
        width: i32,
        state: TitlebarState,
    ) -> Option<&MemoryRenderBuffer> {
        let width = width.clamp(1, 8192);
        let title = if let Some(surface) = window.toplevel() {
            smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
                states
                    .data_map
                    .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                    .and_then(|data| data.lock().ok().and_then(|a| a.title.clone()))
                    .unwrap_or_default()
            })
        } else {
            window
                .x11_surface()
                .map(smithay::xwayland::X11Surface::title)
                .unwrap_or_default()
        };
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.window_id == window_id
                && entry.title == title
                && entry.width == width
                && entry.focused == state.focused
                && entry.hovered == state.hovered
                && entry.maximized == state.maximized
                && entry.tiled == state.tiled
        }) {
            let entry = self.entries.remove(index)?;
            self.entries.push_front(entry);
            return self.entries.front().map(|e| &e.buffer);
        }
        self.entries.retain(|entry| entry.window_id != window_id);
        let profile_raster = tracing::enabled!(target: "villain::perf", tracing::Level::DEBUG);
        let raster_started = profile_raster.then(Instant::now);
        let mut bg = self.background;
        if !state.focused {
            for value in &mut bg[..3] {
                *value = (*value as f32 * 0.8) as u8;
            }
        }
        let mut pixels = vec![0u8; width as usize * 34 * 4];
        for px in pixels.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&bgra(bg));
        }
        if let Some(index) = state.hovered {
            paint_hover(&mut pixels, width, index);
        }
        let mut text = Buffer::new(&mut self.fonts, Metrics::new(13.0, 18.0));
        text.set_size(Some((width - 130).max(1) as f32), Some(20.0));
        text.set_text(&title, &Attrs::new(), Shaping::Advanced, None);
        let color = TextColor::rgba(
            self.foreground[0],
            self.foreground[1],
            self.foreground[2],
            255,
        );
        text.draw(&mut self.fonts, &mut self.glyphs, color, |x, y, w, h, c| {
            let [r, g, b, a] = c.as_rgba();
            for yy in y..y + h as i32 {
                for xx in x..x + w as i32 {
                    let (xx, yy) = (xx + 12, yy + 7);
                    if xx < 0 || xx >= width - 112 || !(0..34).contains(&yy) {
                        continue;
                    }
                    let i = (yy as usize * width as usize + xx as usize) * 4;
                    let alpha = u32::from(a);
                    for (n, v) in [(2, r), (1, g), (0, b)] {
                        pixels[i + n] = ((u32::from(v) * alpha
                            + u32::from(pixels[i + n]) * (255 - alpha))
                            / 255) as u8;
                    }
                }
            }
        });
        // Hit testing counts slots from the right: close, maximize, float,
        // minimize. Rendering places their matching glyphs in those slots.
        let glyph = self.foreground;
        let center = |control: crate::appearance::TitlebarControl| {
            control_origin(width, control.right_slot())
        };
        stroke_path(
            &mut pixels,
            width,
            glyph,
            center(crate::appearance::TitlebarControl::Minimize),
            &[(5.0, 12.0), (19.0, 12.0)],
        );
        if state.tiled {
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Float),
                &[
                    (3.0, 3.0),
                    (15.0, 3.0),
                    (15.0, 15.0),
                    (3.0, 15.0),
                    (3.0, 3.0),
                ],
            );
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Float),
                &[
                    (9.0, 9.0),
                    (21.0, 9.0),
                    (21.0, 21.0),
                    (9.0, 21.0),
                    (9.0, 9.0),
                ],
            );
        } else {
            // A four-pane outline reads as the action to tile this floating window.
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Float),
                &[
                    (4.0, 4.0),
                    (20.0, 4.0),
                    (20.0, 20.0),
                    (4.0, 20.0),
                    (4.0, 4.0),
                ],
            );
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Float),
                &[(12.0, 4.0), (12.0, 20.0)],
            );
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Float),
                &[(4.0, 12.0), (20.0, 12.0)],
            );
        }
        if state.maximized {
            // Restore: two offset windows, with the rear window's top edge visible.
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Maximize),
                &[
                    (8.0, 4.0),
                    (20.0, 4.0),
                    (20.0, 16.0),
                    (8.0, 16.0),
                    (8.0, 4.0),
                ],
            );
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Maximize),
                &[(4.0, 8.0), (4.0, 20.0), (16.0, 20.0), (16.0, 16.0)],
            );
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Maximize),
                &[(4.0, 8.0), (8.0, 8.0)],
            );
        } else {
            stroke_path(
                &mut pixels,
                width,
                glyph,
                center(crate::appearance::TitlebarControl::Maximize),
                &[
                    (5.0, 5.0),
                    (19.0, 5.0),
                    (19.0, 19.0),
                    (5.0, 19.0),
                    (5.0, 5.0),
                ],
            );
        }
        stroke_path(
            &mut pixels,
            width,
            glyph,
            center(crate::appearance::TitlebarControl::Close),
            &[(6.0, 6.0), (18.0, 18.0)],
        );
        stroke_path(
            &mut pixels,
            width,
            glyph,
            center(crate::appearance::TitlebarControl::Close),
            &[(6.0, 18.0), (18.0, 6.0)],
        );
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (width, 34),
            1,
            Transform::Normal,
            None,
        );
        self.entries.push_front(Entry {
            window_id,
            title,
            width,
            focused: state.focused,
            hovered: state.hovered,
            maximized: state.maximized,
            tiled: state.tiled,
            buffer,
        });
        if let Some(started) = raster_started {
            let raster_us = started.elapsed().as_micros();
            let slow = raster_us >= SLOW_TITLEBAR_RASTER_US;
            tracing::debug!(
                target: "villain::perf",
                event = "titlebar_cache_miss",
                raster_us,
                slow,
                width,
                title_chars = self.entries.front().map_or(0, |entry| entry.title.chars().count()),
                focused = state.focused,
                hovered = state.hovered.is_some(),
                maximized = state.maximized,
                tiled = state.tiled,
                "titlebar cache miss rasterized"
            );
        }
        self.entries.front().map(|entry| &entry.buffer)
    }

    pub(super) fn remove_window(&mut self, window_id: knave_desktop_api::WindowId) {
        self.entries.retain(|entry| entry.window_id != window_id);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rasterized_glyph_is_centered_on_its_hover_background() {
        let width = 800;
        let index = crate::appearance::TitlebarControl::Minimize.right_slot() as u8;
        let mut pixels = vec![0; width as usize * 34 * 4];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&super::bgra([24, 28, 32, 255]));
        }
        super::paint_hover(&mut pixels, width, index);
        let hovered_color = super::bgra([70, 77, 85, 255]);
        let hover_center = ((width - i32::from(index) * 24 - 23) as f32
            + (width - i32::from(index) * 24 - 1) as f32)
            / 2.0;

        let (origin, _) = super::control_origin(width, i32::from(index));
        super::stroke_path(
            &mut pixels,
            width,
            [240, 240, 240, 255],
            (origin, 10.4),
            &[(5.0, 12.0), (19.0, 12.0)],
        );

        let mut weighted_x = 0.0;
        let mut weight = 0.0;
        let hover_left = (width - i32::from(index) * 24 - 23).max(0) as usize;
        let hover_right = (width - i32::from(index) * 24).min(width) as usize;
        for y in 5..29 {
            for x in hover_left..hover_right {
                let pixel = &pixels[(y * width as usize + x) * 4..][..4];
                if pixel == hovered_color {
                    continue;
                }
                // Glyph pixels differ from the exact hover fill; measure their
                // raster coverage rather than inferring location from origin.
                let coverage = u32::from(pixel[0].abs_diff(hovered_color[0]))
                    + u32::from(pixel[1].abs_diff(hovered_color[1]))
                    + u32::from(pixel[2].abs_diff(hovered_color[2]));
                if coverage > 0 {
                    weighted_x += (x as f32 + 0.5) * coverage as f32;
                    weight += coverage as f32;
                }
            }
        }
        assert!(
            weight > 0.0,
            "minimize glyph must rasterize over hover fill"
        );
        assert!((weighted_x / weight - hover_center).abs() <= 0.5);

        use crate::appearance::TitlebarControl as Control;
        for (control, distance) in [
            (Control::Close, 12.0),
            (Control::Maximize, 36.0),
            (Control::Float, 60.0),
            (Control::Minimize, 84.0),
        ] {
            assert_eq!(Control::from_right_distance(distance), Some(control));
        }
        assert_eq!(Control::from_right_distance(24.0), Some(Control::Close));
        assert_eq!(Control::from_right_distance(48.0), Some(Control::Maximize));
        assert_eq!(Control::from_right_distance(72.0), Some(Control::Float));
        assert_eq!(Control::from_right_distance(96.0), Some(Control::Minimize));
    }

    #[test]
    fn argb8888_memory_bytes_are_bgra() {
        assert_eq!(
            super::bgra([0x12, 0x34, 0x56, 0x78]),
            [0x56, 0x34, 0x12, 0x78]
        );
    }

    #[test]
    fn bundled_theme_resolves_for_titlebar() {
        let titlebars = super::Titlebars::default();
        assert_ne!(titlebars.background, titlebars.foreground);
    }
}
