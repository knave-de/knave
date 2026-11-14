//! Official Knave SVG mark renders, with theme-specific palettes and transparency.
//!
//! The SVG paths in `assets/knave-color-{light,dark}.svg` are the source of truth.
//! PNGs are deterministic 128×128 Cairo renders for the shell's `UiImage` API.

use std::sync::OnceLock;

use knave_ui::{
    UiImage,
    toolkit::{Chroma, Contrast, Theme, ThemePolicy},
};

const MAX_BRAND_PIXELS: usize = 1024 * 1024;
const COLOR_DARK: &[u8] = include_bytes!("../assets/knave-color-dark.png");
const COLOR_LIGHT: &[u8] = include_bytes!("../assets/knave-color-light.png");
const MONOCHROME_DARK: &[u8] = include_bytes!("../assets/knave-monochrome-dark.png");
const MONOCHROME_LIGHT: &[u8] = include_bytes!("../assets/knave-monochrome-light.png");
const COLOR_LIGHT_HIGH_CONTRAST: &[u8] =
    include_bytes!("../assets/knave-color-light-high-contrast.png");

const VARIANTS: [(&[u8], ThemePolicy); 6] = [
    (
        COLOR_DARK,
        ThemePolicy::new(Theme::Dark, Contrast::Standard, Chroma::FullColor),
    ),
    (
        COLOR_LIGHT,
        ThemePolicy::new(Theme::Light, Contrast::Standard, Chroma::FullColor),
    ),
    (
        COLOR_DARK,
        ThemePolicy::new(Theme::Dark, Contrast::High, Chroma::FullColor),
    ),
    (
        COLOR_LIGHT_HIGH_CONTRAST,
        ThemePolicy::new(Theme::Light, Contrast::High, Chroma::FullColor),
    ),
    (
        MONOCHROME_DARK,
        ThemePolicy::new(Theme::Dark, Contrast::Standard, Chroma::Monochrome),
    ),
    (
        MONOCHROME_LIGHT,
        ThemePolicy::new(Theme::Light, Contrast::Standard, Chroma::Monochrome),
    ),
];

/// Official mark selected for the active scheme and accessibility palette.
pub fn image_for(policy: ThemePolicy) -> Option<UiImage> {
    let index = VARIANTS
        .iter()
        .position(|(_, candidate)| *candidate == policy)
        // Monochrome fills already meet high contrast against their scheme.
        .or(match policy {
            ThemePolicy {
                scheme: Theme::Dark,
                contrast: Contrast::High,
                chroma: Chroma::Monochrome,
            } => Some(4),
            ThemePolicy {
                scheme: Theme::Light,
                contrast: Contrast::High,
                chroma: Chroma::Monochrome,
            } => Some(5),
            _ => None,
        })?;
    images()[index]
        .get_or_init(|| decode(VARIANTS[index].0))
        .clone()
}

/// Dark full-color logo used by newly constructed shell scenes.
pub fn image() -> Option<UiImage> {
    image_for(ThemePolicy::default())
}

fn images() -> &'static [OnceLock<Option<UiImage>>; VARIANTS.len()] {
    static IMAGES: [OnceLock<Option<UiImage>>; VARIANTS.len()] =
        [const { OnceLock::new() }; VARIANTS.len()];
    &IMAGES
}

fn decode(bytes: &[u8]) -> Option<UiImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_BRAND_PIXELS,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let buffer_size = reader.output_buffer_size().filter(|size| {
        bounded_buffer_size(reader.info()).is_some_and(|bounded| *size <= bounded)
    })?;
    let mut buffer = vec![0; buffer_size];
    let info = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => data
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    UiImage::from_rgba(info.width, info.height, rgba)
}

/// Bound decoder growth instead of trusting checked-in assets implicitly.
fn bounded_buffer_size(info: &png::Info<'_>) -> Option<usize> {
    let pixel_count = u64::from(info.width) * u64::from(info.height);
    if pixel_count == 0 || pixel_count > u64::try_from(MAX_BRAND_PIXELS).ok()? {
        return None;
    }
    usize::try_from(pixel_count.checked_mul(4)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICIES: [ThemePolicy; 8] = [
        ThemePolicy::new(Theme::Dark, Contrast::Standard, Chroma::FullColor),
        ThemePolicy::new(Theme::Light, Contrast::Standard, Chroma::FullColor),
        ThemePolicy::new(Theme::Dark, Contrast::High, Chroma::FullColor),
        ThemePolicy::new(Theme::Light, Contrast::High, Chroma::FullColor),
        ThemePolicy::new(Theme::Dark, Contrast::Standard, Chroma::Monochrome),
        ThemePolicy::new(Theme::Light, Contrast::Standard, Chroma::Monochrome),
        ThemePolicy::new(Theme::Dark, Contrast::High, Chroma::Monochrome),
        ThemePolicy::new(Theme::Light, Contrast::High, Chroma::Monochrome),
    ];

    const SOURCE_SVGS: [&str; 2] = [
        include_str!("../assets/knave-color-light.svg"),
        include_str!("../assets/knave-color-dark.svg"),
    ];
    const VARIANT_SVGS: [&str; 5] = [
        include_str!("../assets/knave-color-light-high-contrast.svg"),
        include_str!("../assets/knave-monochrome-dark.svg"),
        include_str!("../assets/knave-monochrome-light.svg"),
        include_str!("../assets/knave-color-light.svg"),
        include_str!("../assets/knave-color-dark.svg"),
    ];

    #[test]
    fn policy_variants_keep_transparent_cutouts_and_identical_coverage() {
        let images: Vec<_> = POLICIES
            .iter()
            .map(|policy| image_for(*policy).expect("bundled themed mark decodes"))
            .collect();
        let reference = images[0].pixels();
        assert_eq!((images[0].width(), images[0].height()), (128, 128));
        assert_eq!(pixel(&images[0], 64, 5), [0, 0, 0, 0]);
        assert_eq!(pixel(&images[0], 50, 20)[3], 255);
        assert!(
            reference
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0 && pixel[3] < 255)
        );
        for image in &images[1..] {
            assert_eq!((image.width(), image.height()), (128, 128));
            assert_eq!(
                image
                    .pixels()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>(),
                reference
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>(),
            );
            assert_eq!(pixel(image, 64, 5), [0, 0, 0, 0]);
        }
        for policy in POLICIES
            .into_iter()
            .filter(|policy| policy.chroma == Chroma::Monochrome)
        {
            let image = image_for(policy).unwrap();
            assert!(
                image
                    .pixels()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| { pixel[3] == 0 || pixel[0] == pixel[1] && pixel[1] == pixel[2] })
            );
        }
    }

    #[test]
    fn appearance_colors_remain_visible_against_both_bar_surfaces() {
        for policy in POLICIES {
            let image = image_for(policy).unwrap();
            let background = match policy.scheme {
                // Shell bar follows the toolkit surface token in each scheme.
                Theme::Dark => [35, 39, 45],
                Theme::Light => [221, 229, 234],
            };
            let ratio =
                |x, y| contrast_ratio(pixel(&image, x, y)[..3].try_into().unwrap(), background);
            let minimum = if policy.contrast == Contrast::High {
                3.0
            } else {
                2.0
            };
            assert!(
                ratio(50, 20) >= minimum,
                "{policy:?} left half lacks contrast"
            );
            assert!(
                ratio(80, 20) >= minimum,
                "{policy:?} right half lacks contrast"
            );
            assert_eq!(image.cache_key(), image_for(policy).unwrap().cache_key());
        }
    }

    #[test]
    fn every_raster_variant_uses_the_official_svg_path_geometry() {
        let geometry = path_data(SOURCE_SVGS[0]);
        assert_eq!(geometry.len(), 8);
        assert_eq!(geometry, path_data(SOURCE_SVGS[1]));
        for svg in VARIANT_SVGS {
            assert!(svg.contains("viewBox=\"0 0 862 862\""));
            assert_eq!(path_data(svg), geometry);
        }
    }

    fn path_data(svg: &str) -> Vec<&str> {
        svg.lines()
            .filter_map(|line| line.split_once(" d=\"").map(|(_, path)| path))
            .filter_map(|path| path.split_once('"').map(|(path, _)| path))
            .collect()
    }

    fn pixel(image: &UiImage, x: usize, y: usize) -> [u8; 4] {
        let start = (y * image.width() as usize + x) * 4;
        image.pixels()[start..start + 4].try_into().unwrap()
    }

    fn contrast_ratio(first: [u8; 3], second: [u8; 3]) -> f64 {
        fn luminance(rgb: [u8; 3]) -> f64 {
            let linear = rgb.map(|value| {
                let channel = f64::from(value) / 255.0;
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
        }
        let first = luminance(first);
        let second = luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    #[test]
    fn bounded_buffer_size_caps_dimensional_growth() {
        let mut info = png::Info::default();
        info.width = u32::MAX;
        info.height = u32::MAX;
        assert_eq!(bounded_buffer_size(&info), None);
    }
}
