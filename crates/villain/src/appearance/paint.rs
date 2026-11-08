//! GLES effect elements and bounded offscreen targets.
use super::*;
pub(super) fn source_key(element: &impl Element) -> SourceKey {
    (
        element.id().clone(),
        element.current_commit(),
        element.geometry(Scale::from(1.0)),
        element.alpha().to_bits(),
        element.src(),
        element.transform(),
    )
}
pub(super) fn render_texture<
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
>(
    renderer: &mut GlesRenderer,
    size: Size<i32, Logical>,
    elements: &[E],
    clear: [f32; 4],
) -> Result<GlesTexture, EffectError> {
    if size.w <= 0 || size.h <= 0 || i64::from(size.w) * i64::from(size.h) > MAX_PIXELS {
        return Err(EffectError::TargetTooLarge);
    }
    let mut texture = Offscreen::<GlesTexture>::create_buffer(
        renderer,
        Fourcc::Abgr8888,
        (size.w, size.h).into(),
    )
    .map_err(EffectError::Renderer)?;
    let mut framebuffer = renderer.bind(&mut texture).map_err(EffectError::Renderer)?;
    let mut tracker = OutputDamageTracker::new((size.w, size.h), 1.0, Transform::Normal);
    // No CPU wait: this runs on the compositor event thread every effect
    // frame. Smithay fences the texture write and makes later samplers on this
    // context wait GPU-side, so ordering holds without blocking input or IPC.
    tracker
        .render_output(renderer, &mut framebuffer, 0, elements, clear)
        .map_err(EffectError::Damage)?;
    drop(framebuffer);
    Ok(texture)
}
pub(super) fn texture_element(
    buffer: &TextureBuffer<GlesTexture>,
    location: (i32, i32),
    opacity: f32,
) -> TextureRenderElement<GlesTexture> {
    TextureRenderElement::from_texture_buffer(
        (f64::from(location.0), f64::from(location.1)),
        buffer,
        Some(opacity),
        None,
        None,
        Kind::Unspecified,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn filtered(
    renderer: &GlesRenderer,
    texture: GlesTexture,
    program: &GlesTexProgram,
    loc: smithay::utils::Point<i32, Logical>,
    radii: [f32; 4],
    opacity: f32,
    direction: (f32, f32),
    radius: f32,
) -> EffectElement {
    let size = texture.size();
    let flip = if texture.is_y_inverted() { 1.0 } else { 0.0 };
    let buffer = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    EffectElement::Filter(TextureShaderElement::new(
        texture_element(&buffer, (loc.x, loc.y), opacity),
        program.clone(),
        vec![
            Uniform::new("extent", (size.w as f32, size.h as f32)),
            Uniform::new("radii", radii),
            Uniform::new("mask_rect", [0.0, 0.0, size.w as f32, size.h as f32]),
            Uniform::new("direction", direction),
            Uniform::new("blur_radius", radius),
            Uniform::new("flip_y", flip),
        ],
    ))
}
pub(super) fn filtered_region(
    renderer: &GlesRenderer,
    texture: GlesTexture,
    program: &GlesTexProgram,
    destination: Geometry,
    source: Geometry,
    radii: [f32; 4],
) -> EffectElement {
    let size = texture.size();
    let flip = if texture.is_y_inverted() { 1.0 } else { 0.0 };
    let buffer = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    let inner = TextureRenderElement::from_texture_buffer(
        destination.loc.to_f64().to_physical(1.0),
        &buffer,
        Some(1.0),
        Some(source.to_f64()),
        Some(destination.size),
        Kind::Unspecified,
    );
    EffectElement::Filter(TextureShaderElement::new(
        inner,
        program.clone(),
        vec![
            Uniform::new("extent", (size.w as f32, size.h as f32)),
            Uniform::new("radii", radii),
            Uniform::new("direction", (0.0_f32, 0.0_f32)),
            Uniform::new("blur_radius", 0.0_f32),
            Uniform::new("flip_y", flip),
            Uniform::new(
                "mask_rect",
                [
                    source.loc.x as f32,
                    source.loc.y as f32,
                    source.size.w as f32,
                    source.size.h as f32,
                ],
            ),
        ],
    ))
}
pub(super) fn expand(mut rect: Geometry, width: Edges) -> Geometry {
    rect.loc.x -= i32::from(width.left);
    rect.loc.y -= i32::from(width.top);
    rect.size.w += i32::from(width.left) + i32::from(width.right);
    rect.size.h += i32::from(width.top) + i32::from(width.bottom);
    rect
}
pub(crate) fn normalized_radii(r: Corners, size: Size<i32, Logical>) -> [f32; 4] {
    let mut values = [
        r.top_left as f32,
        r.top_right as f32,
        r.bottom_right as f32,
        r.bottom_left as f32,
    ];
    let scale = [
        (size.w, values[0] + values[1]),
        (size.w, values[3] + values[2]),
        (size.h, values[0] + values[3]),
        (size.h, values[1] + values[2]),
    ]
    .into_iter()
    .filter(|(_, sum)| *sum > 0.0)
    .map(|(length, sum)| length as f32 / sum)
    .fold(1.0_f32, f32::min);
    for value in &mut values {
        *value *= scale;
    }
    values
}
pub(super) fn border(
    program: &GlesPixelProgram,
    rect: Geometry,
    width: Edges,
    radii: [f32; 4],
    colors: &knave_config::appearance::EdgeColors,
) -> PixelShaderElement {
    shape(
        program,
        rect,
        rect,
        radii,
        [
            width.top as f32,
            width.right as f32,
            width.bottom as f32,
            width.left as f32,
        ],
        colors,
        -1.0,
        [0.0; 4],
        [0.0; 4],
    )
}
pub(super) fn shadow_element(
    program: &GlesPixelProgram,
    body: Geometry,
    radii: [f32; 4],
    side: usize,
    shadow: &knave_config::appearance::Shadow,
) -> PixelShaderElement {
    let extent = i32::from(shadow.blur_radius) * 4
        + i32::from(shadow.spread).max(0)
        + i32::from(shadow.offset_x)
            .abs()
            .max(i32::from(shadow.offset_y).abs())
        + 1;
    let area = Rectangle::new(
        (body.loc.x - extent, body.loc.y - extent).into(),
        (body.size.w + extent * 2, body.size.h + extent * 2).into(),
    );
    shape(
        program,
        area,
        body,
        radii,
        [0.0; 4],
        &knave_config::appearance::EdgeColors::default(),
        side as f32,
        [
            shadow.offset_x as f32,
            shadow.offset_y as f32,
            shadow.blur_radius as f32,
            shadow.spread as f32,
        ],
        rgba(&shadow.color).expect("validated shadow color"),
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn shape(
    program: &GlesPixelProgram,
    area: Geometry,
    body: Geometry,
    radii: [f32; 4],
    widths: [f32; 4],
    colors: &knave_config::appearance::EdgeColors,
    side: f32,
    shadow_data: [f32; 4],
    shadow_color: [f32; 4],
) -> PixelShaderElement {
    PixelShaderElement::new(
        program.clone(),
        area,
        None,
        1.0,
        vec![
            Uniform::new("radii", radii),
            Uniform::new("widths", widths),
            Uniform::new(
                "body",
                [
                    (body.loc.x - area.loc.x) as f32,
                    (body.loc.y - area.loc.y) as f32,
                    body.size.w as f32,
                    body.size.h as f32,
                ],
            ),
            Uniform::new(
                "top_color",
                rgba(&colors.top).expect("validated border color"),
            ),
            Uniform::new(
                "right_color",
                rgba(&colors.right).expect("validated border color"),
            ),
            Uniform::new(
                "bottom_color",
                rgba(&colors.bottom).expect("validated border color"),
            ),
            Uniform::new(
                "left_color",
                rgba(&colors.left).expect("validated border color"),
            ),
            Uniform::new("side", side),
            Uniform::new("shadow_data", shadow_data),
            Uniform::new("shadow_color", shadow_color),
        ],
        Kind::Unspecified,
    )
}
