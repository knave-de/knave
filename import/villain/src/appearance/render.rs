//! Event-driven effects composition. One bounded LRU owns finished scene textures.
pub(crate) use paint::normalized_radii;
#[path = "paint.rs"]
mod paint;
#[path = "titlebar.rs"]
mod titlebar;
use crate::{focus::KeyboardFocus, state::Villain};
use knave_config::{
    WindowAppearance,
    appearance::{Corners, Edges, ViewEffects, rgba},
};
use paint::*;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen, Texture,
            damage::OutputDamageTracker,
            element::{
                AsRenderElements, Element, Id, Kind,
                memory::MemoryRenderBufferRenderElement,
                solid::SolidColorRenderElement,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
                texture::{TextureBuffer, TextureRenderElement},
                utils::{CropRenderElement, Relocate, RelocateRenderElement},
            },
            gles::{
                GlesError, GlesPixelProgram, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
                UniformName, UniformType,
                element::{PixelShaderElement, TextureShaderElement},
            },
            utils::CommitCounter,
        },
    },
    desktop::{PopupManager, layer_map_for_output},
    utils::{Logical, Physical, Rectangle, Scale, Size, Transform},
    wayland::{seat::WaylandFocus, shell::wlr_layer::Layer},
};
use std::{collections::VecDeque, time::Instant};

smithay::backend::renderer::element::render_elements! {
    pub EffectElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    CroppedSurface=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Shape=PixelShaderElement,
    Texture=TextureRenderElement<GlesTexture>,
    Filter=TextureShaderElement,
}

#[derive(Debug)]
pub(crate) enum EffectError {
    TargetTooLarge,
    Renderer(GlesError),
    Damage(smithay::backend::renderer::damage::Error<GlesError>),
    OutsideOutput,
}
impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetTooLarge => f.write_str("effect target exceeds 36 Mi pixels"),
            Self::Renderer(error) => write!(f, "effect renderer failed: {error}"),
            Self::Damage(error) => write!(f, "effect composition failed: {error}"),
            Self::OutsideOutput => f.write_str("blur rectangle outside output"),
        }
    }
}
impl std::error::Error for EffectError {}

type Geometry = Rectangle<i32, Logical>;
type SourceKey = (
    Id,
    CommitCounter,
    Rectangle<i32, Physical>,
    u32,
    Rectangle<f64, smithay::utils::Buffer>,
    Transform,
);
#[derive(PartialEq)]
struct SceneKey {
    workspace: usize,
    layers: SceneLayers,
    size: Size<i32, Logical>,
    appearance: WindowAppearance,
    sources: Vec<SourceKey>,
    windows: Vec<(Geometry, bool, bool, bool, bool)>,
}
struct CachedScene {
    key: SceneKey,
    texture: TextureBuffer<GlesTexture>,
}
#[derive(Default)]
pub(crate) struct EffectsRenderer {
    programs: Option<(GlesPixelProgram, GlesTexProgram)>,
    titlebars: Option<titlebar::Titlebars>,
    scenes: VecDeque<CachedScene>,
    panes: VecDeque<CachedPane>,
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SceneLayers {
    None,
    Wallpaper,
    All,
}
impl SceneLayers {
    fn includes(self, layer: Layer, namespace: &str) -> bool {
        self == Self::All
            || (self == Self::Wallpaper
                && layer == Layer::Background
                && namespace == "knave-shell-wallpaper")
    }
}
struct CachedPane {
    workspace: usize,
    geometry: Geometry,
    sources: Vec<SourceKey>,
    texture: TextureBuffer<GlesTexture>,
}
const MAX_PIXELS: i64 = 36 * 1024 * 1024;
const SLOW_SCENE_STAGE_US: u128 = 8_000;
const SLOW_SCENE_TOTAL_US: u128 = 16_000;

fn log_slow_scene_stage(stage: &'static str, started: Option<Instant>, windows: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed_us = started.elapsed().as_micros();
    if elapsed_us >= SLOW_SCENE_STAGE_US {
        tracing::debug!(
            target: "villain::perf",
            event = "slow_base_scene_stage",
            stage,
            elapsed_us,
            windows,
            "base scene stage exceeded timing threshold"
        );
    }
}

fn log_slow_scene_total(path: &'static str, started: Option<Instant>, windows: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed_us = started.elapsed().as_micros();
    if elapsed_us >= SLOW_SCENE_TOTAL_US {
        tracing::debug!(
            target: "villain::perf",
            event = "slow_base_scene",
            path,
            elapsed_us,
            windows,
            "base scene generation exceeded timing threshold"
        );
    }
}

fn scene_render_scale(target_scale: f64, has_effects: bool) -> f64 {
    if has_effects { 1.0 } else { target_scale }
}

fn physical_window_crop(rect: Geometry, scale: f64) -> Rectangle<i32, Physical> {
    rect.to_physical_precise_round(scale)
}

// Clip root/subsurface content even when visual effects are disabled. Resize
// configures are asynchronous, and clients may keep buffers larger than a tile.
fn window_elements(
    window: &smithay::desktop::Window,
    renderer: &mut GlesRenderer,
    rect: Geometry,
    clip: bool,
    scale: f64,
) -> Vec<EffectElement> {
    if !clip {
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = window.render_elements(
            renderer,
            (rect.loc - window.geometry().loc).to_physical_precise_round(scale),
            Scale::from(scale),
            1.0,
        );
        return elements.into_iter().map(EffectElement::Surface).collect();
    }
    let Some(root) = window.wl_surface() else {
        return Vec::new();
    };
    let mut elements = Vec::new();
    // Menus and tooltips are separate surface trees and keep their own bounds.
    for (popup, location) in PopupManager::popups_for_surface(&root) {
        let popup_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                (rect.loc + location - popup.geometry().loc).to_physical_precise_round(scale),
                Scale::from(scale),
                1.0,
                Kind::Unspecified,
            );
        elements.extend(popup_elements.into_iter().map(EffectElement::Surface));
    }
    let root_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(
            renderer,
            &root,
            (rect.loc - window.geometry().loc).to_physical_precise_round(scale),
            Scale::from(scale),
            1.0,
            Kind::Unspecified,
        );
    elements.extend(root_elements.into_iter().filter_map(|element| {
        CropRenderElement::from_element(element, scale, physical_window_crop(rect, scale))
            .map(EffectElement::CroppedSurface)
    }));
    elements
}

impl EffectsRenderer {
    pub(crate) fn with_warmed_titlebars() -> Self {
        Self {
            titlebars: Some(titlebar::Titlebars::default()),
            ..Self::default()
        }
    }

    pub(crate) fn clear(&mut self) {
        self.scenes.clear();
        self.panes.clear();
    }

    pub(crate) fn remove_titlebar(&mut self, window_id: knave_desktop_api::WindowId) {
        if let Some(titlebars) = &mut self.titlebars {
            titlebars.remove_window(window_id);
        }
    }
    pub(crate) fn scene(
        &mut self,
        state: &Villain,
        renderer: &mut GlesRenderer,
        workspace: usize,
        layers: SceneLayers,
        target_scale: f64,
    ) -> Result<Vec<EffectElement>, EffectError> {
        let mut elements = self.base_scene(state, renderer, workspace, layers, target_scale)?;
        if workspace == state.active_workspace
            && layers == SceneLayers::All
            && let Some(drag) = &state.interaction
            && drag.moved
            && let crate::window_grab::Operation::Tile {
                geometry,
                destination,
                ..
            } = &drag.operation
        {
            let target = destination.as_ref().and_then(|target| {
                state
                    .tiled_slots()
                    .into_iter()
                    .find(|(w, _)| w == target)
                    .map(|(_, rect)| rect)
            });
            for (index, rect) in [Some(*geometry), target].into_iter().enumerate() {
                if let Some(rect) = rect {
                    elements.insert(
                        0,
                        EffectElement::Solid(SolidColorRenderElement::new(
                            state.drag_marker_ids[index].clone(),
                            rect.to_physical_precise_round(target_scale),
                            CommitCounter::default(),
                            if index == 0 {
                                [0.04, 0.07, 0.12, 0.18]
                            } else {
                                [0.06, 0.20, 0.32, 0.32]
                            },
                            Kind::Unspecified,
                        )),
                    );
                }
            }
        }
        Ok(elements)
    }
    fn base_scene(
        &mut self,
        state: &Villain,
        renderer: &mut GlesRenderer,
        workspace: usize,
        layers: SceneLayers,
        target_scale: f64,
    ) -> Result<Vec<EffectElement>, EffectError> {
        let profile = tracing::enabled!(target: "villain::perf", tracing::Level::DEBUG);
        let total_started = profile.then(Instant::now);
        let stage_started = profile.then(Instant::now);
        let size = state.output_size;
        let focused = state.keyboard.current_focus();
        let mut windows: Vec<_> = state
            .workspace_layout(workspace)
            .into_iter()
            .filter(|entry| entry.5 && !(layers == SceneLayers::All && state.overview_mapped()))
            .collect();
        let window_count = windows.len();
        if workspace == state.active_workspace
            && layers == SceneLayers::All
            && let Some(drag) = &state.interaction
            && drag.moved
            && let crate::window_grab::Operation::Tile {
                window, geometry, ..
            } = &drag.operation
            && let Some(index) = windows.iter().position(|entry| entry.0 == *window)
        {
            let mut lifted = windows.remove(index);
            lifted.1 = Rectangle::new(geometry.loc + drag.delta, geometry.size);
            windows.push(lifted);
        }
        let any_effect = windows
            .iter()
            .any(|(_, _, fullscreen, maximized, tiled, _)| {
                let a = &state.config.appearance;
                let view = if *fullscreen {
                    ViewEffects::NONE
                } else if *maximized {
                    a.views.maximized
                } else if *tiled {
                    a.views.tiled
                } else {
                    a.views.floating
                };
                (view.borders && a.border.width != Edges::default())
                    || (view.radius && a.border.radius != Corners::default())
                    || [&a.focused, &a.unfocused].into_iter().any(|style| {
                        (view.opacity && style.opacity != 100)
                            || (view.blur && style.blur.enabled && style.blur.radius > 0)
                            || (view.shadows
                                && [
                                    &style.shadows.top,
                                    &style.shadows.right,
                                    &style.shadows.bottom,
                                    &style.shadows.left,
                                ]
                                .iter()
                                .any(|s| s.enabled))
                    })
            });
        log_slow_scene_stage("workspace_layout", stage_started, window_count);
        // Effects are composed into a logical-size offscreen target. The fast
        // path returns elements directly to its caller, so use that caller's
        // target scale for both surface placement and clipping.
        let render_scale = scene_render_scale(target_scale, any_effect);
        let mut key = SceneKey {
            workspace,
            layers,
            size,
            appearance: state.config.appearance.clone(),
            sources: Vec::new(),
            windows: Vec::new(),
        };
        let mut lower = Vec::<EffectElement>::new();
        let mut upper = Vec::<EffectElement>::new();
        let layers_started = profile.then(Instant::now);
        {
            for output in state.space.outputs() {
                let map = layer_map_for_output(output);
                for layer in map.layers() {
                    if (state.overview_mapped()
                        && layers == SceneLayers::All
                        && matches!(
                            layer.namespace(),
                            "knave-shell-overview" | "knave-shell-bar"
                        ))
                        || !layers.includes(layer.layer(), layer.namespace())
                        || !state.layer_surface_visible(layer.wl_surface())
                    {
                        continue;
                    }
                    let Some(geometry) = map.layer_geometry(layer) else {
                        continue;
                    };
                    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = layer
                        .render_elements(
                            renderer,
                            geometry.loc.to_physical_precise_round(render_scale),
                            Scale::from(render_scale),
                            1.0,
                        );
                    let target = if matches!(layer.layer(), Layer::Top | Layer::Overlay) {
                        &mut upper
                    } else {
                        &mut lower
                    };
                    key.sources.extend(elements.iter().map(source_key));
                    target.splice(0..0, elements.into_iter().map(EffectElement::Surface));
                }
            }
        }
        log_slow_scene_stage("layer_elements", layers_started, window_count);
        let mut sources = Vec::new();
        let windows_started = profile.then(Instant::now);
        for (window, rect, fullscreen, maximized, tiled, _) in &windows {
            let is_focused = KeyboardFocus::for_window(window) == focused;
            key.windows
                .push((*rect, *fullscreen, *maximized, *tiled, is_focused));
            let elements = window_elements(
                window,
                renderer,
                *rect,
                *tiled || *fullscreen || *maximized,
                render_scale,
            );
            key.sources.extend(elements.iter().map(source_key));
            let bar = if let (Some(bar_rect), Some(window_id)) = (
                crate::workspaces::titlebar_rect(window, *rect, *fullscreen),
                state.window_id(window),
            ) {
                let hovered = if workspace == state.active_workspace
                    && bar_rect.to_f64().contains(state.pointer_location)
                {
                    let distance =
                        f64::from(bar_rect.loc.x + bar_rect.size.w) - state.pointer_location.x;
                    crate::appearance::TitlebarControl::from_right_distance(distance)
                        .map(|control| control.right_slot() as u8)
                } else {
                    None
                };
                if let Some(buffer) = self
                    .titlebars
                    .get_or_insert_with(titlebar::Titlebars::default)
                    .buffer(
                        window_id,
                        window,
                        bar_rect.size.w,
                        titlebar::TitlebarState {
                            focused: is_focused,
                            hovered,
                            maximized: *maximized,
                            tiled: *tiled,
                        },
                    )
                {
                    let element = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (
                            f64::from(bar_rect.loc.x) * render_scale,
                            f64::from(bar_rect.loc.y) * render_scale,
                        ),
                        buffer,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    )
                    .map_err(EffectError::Renderer)?;
                    key.sources.push(source_key(&element));
                    Some(EffectElement::Memory(element))
                } else {
                    None
                }
            } else {
                None
            };
            sources.push((elements, bar));
        }
        log_slow_scene_stage("window_elements", windows_started, window_count);
        // Include override-redirect X11 surfaces without adding decorations.
        let mut unmanaged = Vec::new();
        if layers == SceneLayers::All && !state.overview_mapped() {
            for window in state.space.elements() {
                if windows.iter().any(|entry| entry.0 == *window) {
                    continue;
                }
                let Some(loc) = state.space.element_location(window) else {
                    continue;
                };
                let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = window
                    .render_elements(
                        renderer,
                        (loc - window.geometry().loc).to_physical_precise_round(render_scale),
                        Scale::from(render_scale),
                        1.0,
                    );
                key.sources.extend(elements.iter().map(source_key));
                unmanaged.splice(0..0, elements.into_iter().map(EffectElement::Surface));
            }
        }
        if !any_effect {
            self.scenes
                .retain(|entry| entry.key.workspace != workspace || entry.key.layers != layers);
            for (mut source, bar) in sources {
                if let Some(bar) = bar {
                    source.insert(0, bar);
                }
                lower.splice(0..0, source);
            }
            unmanaged.extend(lower);
            upper.extend(unmanaged);
            log_slow_scene_total("direct", total_started, window_count);
            return Ok(upper);
        }
        if size.w <= 0 || size.h <= 0 || i64::from(size.w) * i64::from(size.h) > MAX_PIXELS {
            return Err(EffectError::TargetTooLarge);
        }
        if let Some(index) = self.scenes.iter().position(|entry| entry.key == key) {
            let entry = self.scenes.remove(index).expect("cache index exists");
            let element = texture_element(&entry.texture, (0, 0), 1.0);
            self.scenes.push_back(entry);
            return Ok(vec![EffectElement::Texture(element)]);
        }
        let effects_started = profile.then(Instant::now);
        let compile_started = profile.then(Instant::now);
        self.ensure_programs(renderer)
            .map_err(EffectError::Renderer)?;
        log_slow_scene_stage("effect_programs", compile_started, window_count);
        let (shape_program, texture_program) = self.programs.as_ref().expect("programs compiled");
        let appearance = &state.config.appearance;
        let background = [0.0, 0.0, 0.0, 1.0];
        let mut elements = lower;
        let mut live_pixels = 0_i64;
        for ((window, rect, fullscreen, maximized, tiled, _), (original, bar)) in
            windows.into_iter().zip(sources)
        {
            let view = if fullscreen {
                ViewEffects::NONE
            } else if maximized {
                appearance.views.maximized
            } else if tiled {
                appearance.views.tiled
            } else {
                appearance.views.floating
            };
            let style = if KeyboardFocus::for_window(&window) == focused {
                &appearance.focused
            } else {
                &appearance.unfocused
            };
            let opacity = if view.opacity {
                f32::from(style.opacity) / 100.0
            } else {
                1.0
            };
            let widths = if view.borders {
                appearance.border.width
            } else {
                Edges::default()
            };
            let outer = expand(rect, widths);
            let radii = normalized_radii(
                if view.radius {
                    appearance.border.radius
                } else {
                    Corners::default()
                },
                outer.size,
            );
            let inner_radii = [
                (radii[0] - f32::from(widths.left.max(widths.top))).max(0.0),
                (radii[1] - f32::from(widths.right.max(widths.top))).max(0.0),
                (radii[2] - f32::from(widths.right.max(widths.bottom))).max(0.0),
                (radii[3] - f32::from(widths.left.max(widths.bottom))).max(0.0),
            ];
            // Flatten retained lower content before another effect window can exceed
            // the scene pixel budget. Transient targets remain bounded independently.
            let window_pixels = i64::from(rect.size.w) * i64::from(rect.size.h) * 2;
            if live_pixels + window_pixels > MAX_PIXELS && live_pixels > 0 {
                let lower_texture = render_texture(renderer, size, &elements, background)?;
                let lower_buffer = TextureBuffer::from_texture(
                    renderer,
                    lower_texture,
                    1,
                    Transform::Normal,
                    None,
                );
                elements = vec![EffectElement::Texture(texture_element(
                    &lower_buffer,
                    (0, 0),
                    1.0,
                ))];
                live_pixels = i64::from(size.w) * i64::from(size.h);
            }
            live_pixels += window_pixels;
            let mut window_elements = Vec::new();
            // Root/subsurfaces are clipped as a group. Popups remain outside the window mask.
            if radii.iter().any(|r| *r > 0.0) || opacity != 1.0 {
                if let Some(root) = window.wl_surface() {
                    let root_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        render_elements_from_surface_tree(
                            renderer,
                            &root,
                            smithay::utils::Point::<i32, Logical>::from((
                                -window.geometry().loc.x,
                                -window.geometry().loc.y,
                            ))
                            .to_physical_precise_round(1.0),
                            Scale::from(1.0),
                            1.0,
                            Kind::Unspecified,
                        );
                    let content = render_texture(renderer, rect.size, &root_elements, [0.0; 4])?;
                    window_elements.push(filtered(
                        renderer,
                        content,
                        texture_program,
                        rect.loc,
                        inner_radii,
                        opacity,
                        (0.0, 0.0),
                        0.0,
                    ));
                    for (popup, location) in PopupManager::popups_for_surface(&root) {
                        let location = rect.loc + location - popup.geometry().loc;
                        let popup_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            render_elements_from_surface_tree(
                                renderer,
                                popup.wl_surface(),
                                location.to_physical_precise_round(1.0),
                                Scale::from(1.0),
                                opacity,
                                Kind::Unspecified,
                            );
                        window_elements
                            .splice(0..0, popup_elements.into_iter().map(EffectElement::Surface));
                    }
                }
            } else {
                window_elements.extend(original);
            }
            if let Some(bar) = bar {
                window_elements.insert(0, bar);
            }
            if widths != Edges::default() {
                window_elements.push(EffectElement::Shape(border(
                    shape_program,
                    outer,
                    widths,
                    radii,
                    &style.border_color,
                )));
            }
            if view.blur && style.blur.enabled && style.blur.radius > 0 {
                // Capture only lower stacking content; cursor, this window and upper layers are excluded.
                let padding = i32::from(style.blur.radius) * i32::from(style.blur.passes);
                let capture = Rectangle::new(
                    (rect.loc.x - padding, rect.loc.y - padding).into(),
                    (rect.size.w + padding * 2, rect.size.h + padding * 2).into(),
                )
                .intersection(Rectangle::from_size(size))
                .ok_or(EffectError::OutsideOutput)?;
                let relocated: Vec<_> = elements
                    .iter()
                    .map(|element| {
                        RelocateRenderElement::from_element(
                            element,
                            (-capture.loc.x, -capture.loc.y),
                            Relocate::Relative,
                        )
                    })
                    .collect();
                let mut backdrop = render_texture(renderer, capture.size, &relocated, background)?;
                for _ in 0..style.blur.passes {
                    for direction in [(1.0, 0.0), (0.0, 1.0)] {
                        let element = filtered(
                            renderer,
                            backdrop,
                            texture_program,
                            (0, 0).into(),
                            [0.0; 4],
                            1.0,
                            direction,
                            f32::from(style.blur.radius),
                        );
                        backdrop = render_texture(renderer, capture.size, &[element], [0.0; 4])?;
                    }
                }
                let source_rect = Rectangle::new(rect.loc - capture.loc, rect.size);
                window_elements.push(filtered_region(
                    renderer,
                    backdrop,
                    texture_program,
                    rect,
                    source_rect,
                    inner_radii,
                ));
            }
            if view.shadows {
                for (side, shadow) in [
                    &style.shadows.top,
                    &style.shadows.right,
                    &style.shadows.bottom,
                    &style.shadows.left,
                ]
                .into_iter()
                .enumerate()
                {
                    if shadow.enabled {
                        window_elements.push(EffectElement::Shape(shadow_element(
                            shape_program,
                            outer,
                            radii,
                            side,
                            shadow,
                        )));
                    }
                }
            }
            window_elements.extend(elements);
            elements = window_elements;
        }
        unmanaged.extend(elements);
        upper.extend(unmanaged);
        let texture = render_texture(renderer, size, &upper, background)?;
        let texture = TextureBuffer::from_texture(
            renderer,
            texture,
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size((size.w, size.h).into())]),
        );
        let element = texture_element(&texture, (0, 0), 1.0);
        // Replace stale scene for the same workspace; LRU eviction bounds resident pixels.
        self.scenes
            .retain(|entry| entry.key.workspace != workspace || entry.key.layers != layers);
        let pixels =
            |entry: &CachedScene| i64::from(entry.key.size.w) * i64::from(entry.key.size.h);
        while self.scenes.len() >= 10
            || self.scenes.iter().map(pixels).sum::<i64>() + i64::from(size.w) * i64::from(size.h)
                > MAX_PIXELS
        {
            self.scenes.pop_front();
        }
        self.scenes.push_back(CachedScene { key, texture });
        log_slow_scene_stage("effects_composition", effects_started, window_count);
        log_slow_scene_total("effects", total_started, window_count);
        Ok(vec![EffectElement::Texture(element)])
    }
    pub(crate) fn overview_pane(
        &mut self,
        state: &Villain,
        renderer: &mut GlesRenderer,
        workspace: usize,
        geometry: Geometry,
    ) -> Result<EffectElement, EffectError> {
        use smithay::backend::renderer::element::utils::{
            ConstrainAlign, ConstrainScaleBehavior, constrain_render_elements,
        };
        let scene = self.scene(state, renderer, workspace, SceneLayers::Wallpaper, 1.0)?;
        let sources: Vec<_> = scene.iter().map(source_key).collect();
        if let Some(index) = self.panes.iter().position(|p| {
            p.workspace == workspace && p.geometry.size == geometry.size && p.sources == sources
        }) {
            let cached = self.panes.remove(index).expect("pane cache index");
            let location = geometry
                .loc
                .to_physical_precise_round(state.config.output_scale);
            let element = EffectElement::Texture(texture_element(
                &cached.texture,
                (location.x, location.y),
                1.0,
            ));
            self.panes.push_back(cached);
            return Ok(element);
        }
        let pixels = i64::from(geometry.size.w) * i64::from(geometry.size.h);
        if pixels <= 0 || pixels > MAX_PIXELS {
            return Err(EffectError::TargetTooLarge);
        }
        self.panes
            .retain(|p| p.workspace != workspace || p.geometry.size != geometry.size);
        while self.panes.len() >= 6
            || self
                .panes
                .iter()
                .map(|p| i64::from(p.geometry.size.w) * i64::from(p.geometry.size.h))
                .sum::<i64>()
                + pixels
                > MAX_PIXELS
        {
            self.panes.pop_front();
        }
        let elements: Vec<_> = constrain_render_elements(
            scene,
            (0, 0),
            Rectangle::from_size((geometry.size.w, geometry.size.h).into()),
            Rectangle::from_size((state.output_size.w, state.output_size.h).into()),
            ConstrainScaleBehavior::Zoom,
            ConstrainAlign::CENTER,
            Scale::from(1.0),
        )
        .collect();
        let texture = render_texture(renderer, geometry.size, &elements, [0.0, 0.0, 0.0, 1.0])?;
        self.ensure_programs(renderer)
            .map_err(EffectError::Renderer)?;
        let radius = 22.0_f32
            .min(geometry.size.w as f32 / 2.0)
            .min(geometry.size.h as f32 / 2.0);
        let element = filtered(
            renderer,
            texture,
            &self.programs.as_ref().expect("compiled pane shader").1,
            (0, 0).into(),
            [radius; 4],
            1.0,
            (0.0, 0.0),
            0.0,
        );
        let texture = render_texture(renderer, geometry.size, &[element], [0.0; 4])?;
        let texture = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
        let location = geometry
            .loc
            .to_physical_precise_round(state.config.output_scale);
        let element =
            EffectElement::Texture(texture_element(&texture, (location.x, location.y), 1.0));
        self.panes.push_back(CachedPane {
            workspace,
            geometry,
            sources,
            texture,
        });
        Ok(element)
    }
    fn ensure_programs(&mut self, renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        if self.programs.is_none() {
            let shape = renderer.compile_custom_pixel_shader(
                include_str!("shape.glsl"),
                &[
                    UniformName::new("radii", UniformType::_4f),
                    UniformName::new("widths", UniformType::_4f),
                    UniformName::new("top_color", UniformType::_4f),
                    UniformName::new("right_color", UniformType::_4f),
                    UniformName::new("bottom_color", UniformType::_4f),
                    UniformName::new("left_color", UniformType::_4f),
                    UniformName::new("body", UniformType::_4f),
                    UniformName::new("shadow_data", UniformType::_4f),
                    UniformName::new("shadow_color", UniformType::_4f),
                    UniformName::new("side", UniformType::_1f),
                ],
            )?;
            let texture = renderer.compile_custom_texture_shader(
                include_str!("texture.glsl"),
                &[
                    UniformName::new("extent", UniformType::_2f),
                    UniformName::new("mask_rect", UniformType::_4f),
                    UniformName::new("radii", UniformType::_4f),
                    UniformName::new("direction", UniformType::_2f),
                    UniformName::new("blur_radius", UniformType::_1f),
                    UniformName::new("flip_y", UniformType::_1f),
                ],
            )?;
            self.programs = Some((shape, texture));
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;

pub(crate) fn render_night_texture<
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
>(
    renderer: &mut GlesRenderer,
    size: Size<i32, Logical>,
    elements: &[E],
) -> Result<GlesTexture, EffectError> {
    paint::render_texture(renderer, size, elements, [0.0, 0.0, 0.0, 1.0])
}
