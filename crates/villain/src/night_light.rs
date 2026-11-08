//! Output-wide color temperature, shared by nested and DRM outputs.
use crate::{appearance::render::EffectError, overview_render::OverviewRenderElement};
use smithay::{
    backend::renderer::{
        element::{
            Element, Kind,
            texture::{TextureBuffer, TextureRenderElement},
        },
        gles::{
            GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType,
            element::TextureShaderElement,
        },
    },
    utils::{Logical, Scale, Size, Transform},
};

type Key = (
    smithay::backend::renderer::element::Id,
    smithay::backend::renderer::utils::CommitCounter,
    smithay::utils::Rectangle<i32, smithay::utils::Physical>,
    u32,
);
type NightCache = (
    Vec<Key>,
    u16,
    Size<i32, Logical>,
    TextureBuffer<smithay::backend::renderer::gles::GlesTexture>,
);
#[derive(Default)]
pub struct NightLight {
    pub temperature: Option<u16>,
    program: Option<GlesTexProgram>,
    cache: Option<NightCache>,
}
impl NightLight {
    pub fn set(&mut self, temperature: Option<u16>) -> Result<(), &'static str> {
        if temperature.is_some_and(|v| !(2500..=6500).contains(&v)) {
            return Err("Night Light temperature must be 2500–6500 K");
        }
        self.temperature = temperature;
        self.cache = None;
        Ok(())
    }
    pub fn apply(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Logical>,
        elements: Vec<OverviewRenderElement>,
    ) -> Result<Vec<OverviewRenderElement>, EffectError> {
        let Some(temperature) = self.temperature else {
            return Ok(elements);
        };
        let keys: Vec<_> = elements
            .iter()
            .map(|e| {
                (
                    e.id().clone(),
                    e.current_commit(),
                    e.geometry(Scale::from(1.0)),
                    e.alpha().to_bits(),
                )
            })
            .collect();
        let hit = self
            .cache
            .as_ref()
            .is_some_and(|(old, t, s, _)| *old == keys && *t == temperature && *s == size);
        if !hit {
            let texture =
                crate::appearance::render::render_night_texture(renderer, size, &elements)?;
            self.cache = Some((
                keys,
                temperature,
                size,
                TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None),
            ));
        }
        if self.program.is_none() {
            self.program = Some(
                renderer
                    .compile_custom_texture_shader(
                        include_str!("night_light.glsl"),
                        &[UniformName::new("gains", UniformType::_3f)],
                    )
                    .map_err(EffectError::Renderer)?,
            )
        }
        let buffer = &self.cache.as_ref().unwrap().3;
        let inner = TextureRenderElement::from_texture_buffer(
            (0.0, 0.0),
            buffer,
            Some(1.0),
            None,
            None,
            Kind::Unspecified,
        );
        let [r, g, b] = gains(temperature);
        let element = TextureShaderElement::new(
            inner,
            self.program.as_ref().unwrap().clone(),
            vec![Uniform::new("gains", (r, g, b))],
        );
        Ok(vec![OverviewRenderElement::Night(element)])
    }
}
pub fn gains(k: u16) -> [f32; 3] {
    fn raw(k: f32) -> [f32; 3] {
        let t = k / 100.0;
        [
            1.0,
            ((99.4708 * t.ln() - 161.11957) / 255.0).clamp(0.0, 1.0),
            if t <= 19.0 {
                0.0
            } else {
                ((138.51773 * (t - 10.0).ln() - 305.0448) / 255.0).clamp(0.0, 1.0)
            },
        ]
    }
    let base = raw(6500.0);
    let c = raw(f32::from(k.clamp(2500, 6500)));
    [1.0, (c[1] / base[1]).min(1.0), (c[2] / base[2]).min(1.0)]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_and_warm_are_bounded() {
        assert_eq!(gains(6500), [1.0; 3]);
        let warm = gains(2500);
        assert!(warm[0] > warm[1] && warm[1] > warm[2]);
        for k in 2500..=6500 {
            assert!(
                gains(k)
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            )
        }
        assert!(NightLight::default().set(Some(0)).is_err());
    }
}
