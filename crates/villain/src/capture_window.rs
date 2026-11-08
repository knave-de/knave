//! Render one managed window into a stable canvas, never the desktop behind it.
use crate::{overview_render::OverviewRenderElement, state::Villain};
use knave_desktop_api::WindowId;
use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements,
            surface::WaylandSurfaceRenderElement,
            utils::{
                ConstrainAlign, ConstrainScaleBehavior, CropRenderElement, Relocate,
                RelocateRenderElement, RescaleRenderElement, constrain_render_elements,
            },
        },
        gles::GlesRenderer,
    },
    utils::{Buffer, Physical, Rectangle, Scale, Size},
};

smithay::backend::renderer::element::render_elements! {
    pub WindowSceneElement<=GlesRenderer>;
    Cursor=crate::cursor::CursorRenderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
}
pub(crate) type WindowElement = RelocateRenderElement<
    CropRenderElement<RelocateRenderElement<RescaleRenderElement<WindowSceneElement>>>,
>;

pub(crate) fn elements(
    state: &mut Villain,
    renderer: &mut GlesRenderer,
    id: WindowId,
    size: Size<i32, Buffer>,
    cursor: bool,
) -> Result<Vec<OverviewRenderElement>, String> {
    let window = state.capture_window(id).ok_or("selected window closed")?;
    let geometry = window.geometry();
    if geometry.size.w <= 0 || geometry.size.h <= 0 {
        return Err("selected window has no content".into());
    }
    let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = window.render_elements(
        renderer,
        (-geometry.loc.x, -geometry.loc.y).into(),
        Scale::from(1.0),
        1.0,
    );
    let mut scene = Vec::new();
    if cursor
        && let Some(info) = state.window_info().iter().find(|info| info.id == id)
        && info.workspace.0 as usize == state.active_workspace + 1
        && let Some((_, rect, _, _, _, visible)) = state
            .workspace_layout(state.active_workspace)
            .into_iter()
            .find(|entry| entry.0 == window)
        && visible
        && rect.to_f64().contains(state.pointer_location)
        && let Some((surface, _)) = window.surface_under(
            state.pointer_location - rect.loc.to_f64() + geometry.loc.to_f64(),
            smithay::desktop::WindowSurfaceType::ALL,
        )
        && state.pointer.current_focus().as_ref() == Some(&surface)
    {
        scene.extend(
            state
                .cursor
                .render_elements(
                    renderer,
                    state.pointer_location - rect.loc.to_f64(),
                    state.start_time.elapsed(),
                    state.config.output_scale,
                )
                .into_iter()
                .map(WindowSceneElement::Cursor),
        );
    }
    scene.extend(surfaces.into_iter().map(WindowSceneElement::Surface));
    // Resizing changes only the selected window's fit inside its original canvas.
    // It cannot reveal neighbouring windows, decorations or the output behind it.
    let crop = Rectangle::<i32, Physical>::from_size((size.w, size.h).into());
    let reference =
        Rectangle::<i32, Physical>::from_size((geometry.size.w, geometry.size.h).into());
    Ok(constrain_render_elements(
        scene,
        (0, 0),
        crop,
        reference,
        ConstrainScaleBehavior::Fit,
        ConstrainAlign::CENTER,
        Scale::from(1.0),
    )
    .map(|element| RelocateRenderElement::from_element(element, (0, 0), Relocate::Relative))
    .map(OverviewRenderElement::Window)
    .collect())
}
