//! Bounded, on-demand workspace previews for trusted shell clients.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::utils::{Relocate, RelocateRenderElement, RescaleRenderElement},
            gles::{GlesRenderbuffer, GlesRenderer},
        },
    },
    utils::{Buffer, Physical, Point, Rectangle, Size, Transform},
};

use crate::{appearance::render::EffectElement, state::Villain};

const MIN_WIDTH: u32 = 64;
const MIN_HEIGHT: u32 = 36;
const MAX_WIDTH: u32 = 16384;
const MAX_HEIGHT: u32 = 16384;
const MAX_PIXELS: u64 = 36 * 1024 * 1024;

pub fn capture(
    state: &mut Villain,
    workspace: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&width)
        || !(MIN_HEIGHT..=MAX_HEIGHT).contains(&height)
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(format!(
            "preview dimensions must be within {MIN_WIDTH}x{MIN_HEIGHT} and {MAX_WIDTH}x{MAX_HEIGHT}, with at most {MAX_PIXELS} pixels"
        ));
    }
    let index = workspace
        .checked_sub(1)
        .filter(|index| *index < state.workspaces.len())
        .ok_or_else(|| format!("workspace {workspace} does not exist"))?;
    let output_size = state.output_size;
    let mut effects = std::mem::take(&mut state.appearance_renderer);
    let result = if let Some(mut tty) = state.tty.take() {
        let result = effects
            .scene(
                state,
                &mut tty.renderer,
                index,
                crate::appearance::render::SceneLayers::Wallpaper,
                1.0,
            )
            .map_err(|error| error.to_string())
            .and_then(|elements| render(&mut tty.renderer, elements, output_size, width, height));
        state.tty = Some(tty);
        result
    } else if let Some(mut winit) = state.winit.take() {
        let result = effects
            .scene(
                state,
                winit.renderer(),
                index,
                crate::appearance::render::SceneLayers::Wallpaper,
                1.0,
            )
            .map_err(|error| error.to_string())
            .and_then(|elements| render(winit.renderer(), elements, output_size, width, height));
        state.winit = Some(winit);
        result
    } else {
        Err("no renderer is available".into())
    };
    state.appearance_renderer = effects;
    result
}

fn render(
    renderer: &mut GlesRenderer,
    source_elements: Vec<EffectElement>,
    output_size: Size<i32, smithay::utils::Logical>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let buffer_size: Size<i32, Buffer> = (width as i32, height as i32).into();
    let physical_size: Size<i32, Physical> = (width as i32, height as i32).into();
    let source_width = output_size.w.max(1) as f64;
    let source_height = output_size.h.max(1) as f64;
    let scale = (f64::from(width) / source_width).min(f64::from(height) / source_height);
    let content_width = (source_width * scale).round() as i32;
    let content_height = (source_height * scale).round() as i32;
    let offset: Point<i32, Physical> = (
        (width as i32 - content_width) / 2,
        (height as i32 - content_height) / 2,
    )
        .into();

    let elements: Vec<_> = source_elements
        .into_iter()
        .map(|element| {
            RelocateRenderElement::from_element(
                RescaleRenderElement::from_element(element, (0, 0).into(), scale),
                offset,
                Relocate::Relative,
            )
        })
        .collect();

    let mut target =
        Offscreen::<GlesRenderbuffer>::create_buffer(renderer, Fourcc::Abgr8888, buffer_size)
            .map_err(|error| format!("could not allocate preview target: {error}"))?;
    let mut framebuffer = renderer
        .bind(&mut target)
        .map_err(|error| format!("could not bind preview target: {error}"))?;
    let mut damage = OutputDamageTracker::new(physical_size, 1.0, Transform::Normal);
    let result = damage
        .render_output(
            renderer,
            &mut framebuffer,
            0,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )
        .map_err(|error| format!("could not render preview: {error:?}"))?;
    result
        .sync
        .wait()
        .map_err(|error| format!("preview synchronization failed: {error}"))?;

    let mapping = renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(buffer_size),
            Fourcc::Abgr8888,
        )
        .map_err(|error| format!("could not read preview: {error}"))?;
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|error| format!("could not map preview: {error}"))?
        .to_vec();
    encode_png(&pixels, width, height)
}

fn encode_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("could not encode preview header: {error}"))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| format!("could not encode preview pixels: {error}"))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoder_produces_png_signature() {
        let png = encode_png(&[0; 4 * 4 * 4], 4, 4).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
