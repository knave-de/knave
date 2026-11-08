//! Render-only workspace surfaces inside shell-owned overview panes.

use smithay::{
    backend::renderer::{
        element::{
            Id, Kind,
            memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
            utils::{CropRenderElement, Relocate, RelocateRenderElement},
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    utils::{Physical, Rectangle, Scale, Size},
};

use crate::{cursor::CursorRenderElement, state::Villain};
use std::time::Instant;

const SLOW_OVERVIEW_STAGE_US: u128 = 8_000;
const SLOW_OVERVIEW_TOTAL_US: u128 = 16_000;

fn log_slow_stage(stage: &'static str, started: Option<Instant>, elements: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed_us = started.elapsed().as_micros();
    if elapsed_us >= SLOW_OVERVIEW_STAGE_US {
        tracing::debug!(
            target: "villain::perf",
            event = "slow_overview_stage",
            stage,
            elapsed_us,
            elements,
            "overview composition stage exceeded timing threshold"
        );
    }
}

fn log_slow_total(path: &'static str, started: Option<Instant>, elements: usize) {
    let Some(started) = started else {
        return;
    };
    let elapsed_us = started.elapsed().as_micros();
    if elapsed_us >= SLOW_OVERVIEW_TOTAL_US {
        tracing::debug!(
            target: "villain::perf",
            event = "slow_overview_elements",
            path,
            elapsed_us,
            elements,
            "overview element generation exceeded timing threshold"
        );
    }
}

smithay::backend::renderer::element::render_elements! {
    pub OverviewRenderElement<=GlesRenderer>;
    Night=smithay::backend::renderer::gles::element::TextureShaderElement,
    Cursor=CursorRenderElement,
    Overlay=smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>,
    Window=crate::capture_window::WindowElement,
    Scene=crate::appearance::render::EffectElement,
    Swipe=CropRenderElement<RelocateRenderElement<crate::appearance::render::EffectElement>>,
    Screenshot=MemoryRenderBufferRenderElement<GlesRenderer>,
    ScreenshotOverlay=SolidColorRenderElement,
}

pub fn elements(
    state: &mut Villain,
    renderer: &mut GlesRenderer,
    cursor: Vec<CursorRenderElement>,
) -> Result<Vec<OverviewRenderElement>, crate::appearance::render::EffectError> {
    let profile = tracing::enabled!(target: "villain::perf", tracing::Level::DEBUG);
    let total_started = profile.then(Instant::now);
    if let Some(background) = state.screenshot.background() {
        let stage_started = profile.then(Instant::now);
        let result = screenshot_elements(state, renderer, cursor, background);
        log_slow_stage(
            "screenshot",
            stage_started,
            result.as_ref().map_or(0, Vec::len),
        );
        log_slow_total(
            "screenshot",
            total_started,
            result.as_ref().map_or(0, Vec::len),
        );
        return result;
    }
    let stage_started = profile.then(Instant::now);
    let mut result: Vec<_> = cursor
        .into_iter()
        .map(OverviewRenderElement::Cursor)
        .collect();
    let cursor_len = result.len();
    log_slow_stage("cursor_elements", stage_started, cursor_len);
    let now = std::time::Instant::now();
    let scenes = if let Some(transition) = state.gestures.transition {
        let incoming = transition.offset
            + f64::from(transition.direction.incoming_offset(state.output_size.w));
        vec![
            (transition.source, transition.offset),
            (transition.target, incoming),
        ]
    } else {
        state.workspace_motion.scenes(now)
    };
    if !scenes.is_empty() {
        use smithay::{
            backend::renderer::element::AsRenderElements, desktop::layer_map_for_output,
        };
        let output_scale = state.config.output_scale;
        let stage_started = profile.then(Instant::now);
        // Keep shell UI stationary, above the two clipped workspace scenes.
        for output in state.space.outputs() {
            let map = layer_map_for_output(output);
            for layer in map
                .layers()
                .filter(|layer| layer.namespace() != "knave-shell-wallpaper")
            {
                if !state.layer_surface_visible(layer.wl_surface()) {
                    continue;
                }
                if let Some(geometry) = map.layer_geometry(layer) {
                    let elements: Vec<
                        smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
                            GlesRenderer,
                        >,
                    > = layer.render_elements(
                        renderer,
                        geometry.loc.to_physical_precise_round(output_scale),
                        Scale::from(output_scale),
                        1.0,
                    );
                    result.splice(
                        cursor_len..cursor_len,
                        elements.into_iter().map(OverviewRenderElement::Overlay),
                    );
                }
            }
        }
        log_slow_stage("transition_overlays", stage_started, result.len());
        let width = state.output_size.w;
        let bounds = Rectangle::from_size((width, state.output_size.h).into());
        let mut effects = std::mem::take(&mut state.appearance_renderer);
        let scenes = (|| {
            for (workspace, x) in scenes {
                let stage_started = profile.then(Instant::now);
                let x = x.round() as i32;
                let clip = Rectangle::new((x, 0).into(), (width, state.output_size.h).into());
                let Some(clip) = clip.intersection(bounds) else {
                    continue;
                };
                let scene = effects.scene(
                    state,
                    renderer,
                    workspace,
                    crate::appearance::render::SceneLayers::Wallpaper,
                    state.config.output_scale,
                );
                let scene = match scene {
                    Ok(scene) => scene,
                    Err(error) => {
                        log_slow_stage("transition_background_scene", stage_started, 0);
                        return Err(error);
                    }
                };
                let before = result.len();
                result.extend(scene.into_iter().filter_map(|element| {
                    let physical_origin =
                        smithay::utils::Point::<i32, smithay::utils::Logical>::from((x, 0))
                            .to_physical_precise_round(output_scale);
                    let physical_clip = clip.to_physical_precise_round(output_scale);
                    let shifted = RelocateRenderElement::from_element(
                        element,
                        physical_origin,
                        Relocate::Relative,
                    );
                    CropRenderElement::from_element(shifted, output_scale, physical_clip)
                        .map(OverviewRenderElement::Swipe)
                }));
                log_slow_stage(
                    "transition_background_scene",
                    stage_started,
                    result.len().saturating_sub(before),
                );
            }
            Ok(result)
        })();
        state.appearance_renderer = effects;
        log_slow_total(
            "transition",
            total_started,
            scenes.as_ref().map_or(0, Vec::len),
        );
        return scenes;
    }
    let output = state.output_size;
    let overview_mapped = state.overview_mapped();
    if overview_mapped {
        use smithay::{
            backend::renderer::element::AsRenderElements, desktop::layer_map_for_output,
        };
        let stage_started = profile.then(Instant::now);
        for output in state.space.outputs() {
            let map = layer_map_for_output(output);
            for layer in map
                .layers()
                .filter(|layer| layer.namespace() == "knave-shell-overview")
            {
                if !state.layer_surface_visible(layer.wl_surface()) {
                    continue;
                }
                if let Some(geometry) = map.layer_geometry(layer) {
                    let elements: Vec<
                        smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
                            GlesRenderer,
                        >,
                    > = layer.render_elements(
                        renderer,
                        geometry
                            .loc
                            .to_physical_precise_round(state.config.output_scale),
                        Scale::from(state.config.output_scale),
                        1.0,
                    );
                    result.extend(elements.into_iter().map(OverviewRenderElement::Overlay));
                }
            }
        }
        log_slow_stage("overview_shell_layers", stage_started, result.len());
    }
    for pane in state
        .overview_panes
        .clone()
        .iter()
        .rev()
        .filter(|_| overview_mapped)
    {
        if state
            .workspace_preview_scene(pane.workspace.0 as usize - 1)
            .is_none()
        {
            continue;
        }
        let Ok(width) = i32::try_from(pane.width) else {
            continue;
        };
        let Ok(height) = i32::try_from(pane.height) else {
            continue;
        };
        let mut effects = std::mem::take(&mut state.appearance_renderer);
        let stage_started = profile.then(Instant::now);
        let element = effects.overview_pane(
            state,
            renderer,
            pane.workspace.0 as usize - 1,
            Rectangle::new((pane.x, pane.y).into(), (width, height).into()),
        );
        state.appearance_renderer = effects;
        let element = match element {
            Ok(element) => element,
            Err(error) => {
                log_slow_stage("overview_pane", stage_started, result.len());
                return Err(error);
            }
        };
        log_slow_stage("overview_pane", stage_started, result.len() + 1);
        result.push(OverviewRenderElement::Scene(element));
    }
    if overview_mapped {
        result.push(OverviewRenderElement::ScreenshotOverlay(
            SolidColorRenderElement::new(
                state.overview_backdrop_id.clone(),
                Rectangle::from_size(
                    (
                        (f64::from(output.w) * state.config.output_scale).round() as i32,
                        (f64::from(output.h) * state.config.output_scale).round() as i32,
                    )
                        .into(),
                ),
                CommitCounter::default(),
                [0.0, 0.0, 0.0, 0.5],
                Kind::Unspecified,
            ),
        ));
    }
    let mut effects = std::mem::take(&mut state.appearance_renderer);
    let stage_started = profile.then(Instant::now);
    let scene = effects.scene(
        state,
        renderer,
        state.active_workspace,
        crate::appearance::render::SceneLayers::All,
        state.config.output_scale,
    );
    state.appearance_renderer = effects;
    let scene = match scene {
        Ok(scene) => scene,
        Err(error) => {
            log_slow_stage("active_workspace_scene", stage_started, result.len());
            return Err(error);
        }
    };
    let scene_len = scene.len();
    log_slow_stage("active_workspace_scene", stage_started, scene_len);
    result.extend(scene.into_iter().map(OverviewRenderElement::Scene));
    log_slow_total("normal", total_started, result.len());
    Ok(result)
}

fn screenshot_elements(
    state: &Villain,
    renderer: &mut GlesRenderer,
    cursor: Vec<CursorRenderElement>,
    background: &smithay::backend::renderer::element::memory::MemoryRenderBuffer,
) -> Result<Vec<OverviewRenderElement>, crate::appearance::render::EffectError> {
    // Render elements are front-to-back; keep the frozen image behind the tint and cursor.
    let mut result: Vec<_> = cursor
        .into_iter()
        .map(OverviewRenderElement::Cursor)
        .collect();
    let image = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        (0.0, 0.0),
        background,
        None,
        None,
        None,
        Kind::Unspecified,
    )
    .map_err(crate::appearance::render::EffectError::Renderer)?;
    let size = state
        .screenshot
        .frame_size()
        .unwrap_or_else(|| Size::from((state.output_size.w, state.output_size.h)));
    let overlay_ids = state
        .screenshot
        .overlay_ids()
        .expect("screenshot selection");
    let full = Rectangle::from_size((size.w, size.h).into());
    if let Some(selection) = state.screenshot.selection_rect() {
        let x0 = selection.loc.x.clamp(0, full.size.w);
        let y0 = selection.loc.y.clamp(0, full.size.h);
        let x1 = selection
            .loc
            .x
            .saturating_add(selection.size.w)
            .clamp(0, full.size.w);
        let y1 = selection
            .loc
            .y
            .saturating_add(selection.size.h)
            .clamp(0, full.size.h);
        let shade = [0.0, 0.0, 0.0, 0.38];
        for (index, rect) in [
            Rectangle::new((0, 0).into(), (full.size.w, y0).into()),
            Rectangle::new((0, y1).into(), (full.size.w, full.size.h - y1).into()),
            Rectangle::new((0, y0).into(), (x0, y1 - y0).into()),
            Rectangle::new((x1, y0).into(), (full.size.w - x1, y1 - y0).into()),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(element) = solid(overlay_ids[index].clone(), rect, shade) {
                result.push(OverviewRenderElement::ScreenshotOverlay(element));
            }
        }
        let border = [0.25, 0.78, 1.0, 1.0];
        let thickness = 2.min(selection.size.w).min(selection.size.h);
        for (index, rect) in [
            Rectangle::new(selection.loc, (selection.size.w, thickness).into()),
            Rectangle::new(
                (
                    selection.loc.x,
                    selection.loc.y + selection.size.h - thickness,
                )
                    .into(),
                (selection.size.w, thickness).into(),
            ),
            Rectangle::new(selection.loc, (thickness, selection.size.h).into()),
            Rectangle::new(
                (
                    selection.loc.x + selection.size.w - thickness,
                    selection.loc.y,
                )
                    .into(),
                (thickness, selection.size.h).into(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(element) = solid(overlay_ids[index + 4].clone(), rect, border) {
                result.push(OverviewRenderElement::ScreenshotOverlay(element));
            }
        }
    } else {
        if let Some(element) = solid(overlay_ids[0].clone(), full, [0.0, 0.0, 0.0, 0.12]) {
            result.push(OverviewRenderElement::ScreenshotOverlay(element));
        }
    }
    result.push(OverviewRenderElement::Screenshot(image));
    Ok(result)
}

fn solid(
    id: Id,
    rect: Rectangle<i32, Physical>,
    color: [f32; 4],
) -> Option<SolidColorRenderElement> {
    (rect.size.w > 0 && rect.size.h > 0).then(|| {
        SolidColorRenderElement::new(id, rect, CommitCounter::default(), color, Kind::Unspecified)
    })
}
