//! Small bundled status symbols; decode once so bar rebuilds stay allocation-light.
use std::sync::OnceLock;

use knave_ui::{Color, UiImage};

const ICON_PIXELS: u64 = 48 * 48;
const ICON_BYTES: usize = (ICON_PIXELS * 4) as usize;

const WIFI_PNG: &[u8] = include_bytes!("../assets/status-wifi.png");
const VOLUME_PNG: &[u8] = include_bytes!("../assets/status-volume.png");
const BELL_PNG: &[u8] = include_bytes!("../assets/status-bell.png");

fn decode(bytes: &[u8]) -> Option<UiImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: ICON_BYTES });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.width != 48 || info.height != 48 {
        return None;
    }
    let buffer_size = reader
        .output_buffer_size()
        .filter(|size| *size <= ICON_BYTES)?;
    let mut buffer = vec![0; buffer_size];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    UiImage::from_rgba(
        info.width,
        info.height,
        buffer[..info.buffer_size()].to_vec(),
    )
}

pub(super) fn tint(image: &UiImage, color: Color) -> UiImage {
    let mut pixels = image.pixels().to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[0] = color.red;
        pixel[1] = color.green;
        pixel[2] = color.blue;
    }
    UiImage::from_rgba(image.width(), image.height(), pixels).unwrap_or_else(|| image.clone())
}

pub fn wifi() -> Option<UiImage> {
    static IMAGE: OnceLock<Option<UiImage>> = OnceLock::new();
    IMAGE.get_or_init(|| decode(WIFI_PNG)).clone()
}

pub fn volume() -> Option<UiImage> {
    static IMAGE: OnceLock<Option<UiImage>> = OnceLock::new();
    IMAGE.get_or_init(|| decode(VOLUME_PNG)).clone()
}

pub fn bell() -> Option<UiImage> {
    static IMAGE: OnceLock<Option<UiImage>> = OnceLock::new();
    IMAGE.get_or_init(|| decode(BELL_PNG)).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_icons_decode_once_at_bounded_resolution() {
        for image in [wifi(), volume(), bell()] {
            let image = image.expect("bundled icon decodes");
            assert_eq!((image.width(), image.height()), (48, 48));
            assert_eq!(image.pixels().len(), ICON_BYTES);
        }
        assert_eq!(wifi().unwrap().cache_key(), wifi().unwrap().cache_key());
    }
}
