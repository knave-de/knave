//! Snapshot caller-owned images before temporary files disappear. Invalid images never block text.
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbaImage};
use knave_desktop_api::notifications::{MAX_IMAGE_EDGE, NotificationImage};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};
use zbus::zvariant::{Array, OwnedValue, Structure};

const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_INPUT_EDGE: u32 = 4096;
const KEYS: [&str; 5] = [
    "image-data",
    "image_data",
    "image-path",
    "image_path",
    "icon_data",
];

pub(super) fn has_hint(hints: &HashMap<String, OwnedValue>) -> bool {
    KEYS.iter().any(|key| hints.contains_key(*key))
}
pub(super) fn decode(hints: &HashMap<String, OwnedValue>) -> Option<NotificationImage> {
    // The app icon remains separate; raw image hints take precedence over paths and legacy icons.
    for key in KEYS {
        let Some(value) = hints.get(key) else {
            continue;
        };
        let image = if key.contains("path") {
            <&str>::try_from(value).ok().and_then(file)
        } else {
            raw(value)
        };
        if image.is_some() {
            return image;
        }
    }
    None
}
fn preview(image: DynamicImage) -> Option<NotificationImage> {
    let image = if image.width() > MAX_IMAGE_EDGE || image.height() > MAX_IMAGE_EDGE {
        image.thumbnail(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    NotificationImage::new(rgba.width(), rgba.height(), rgba.into_raw())
}
fn raw(value: &OwnedValue) -> Option<NotificationImage> {
    let fields = <&Structure>::try_from(value).ok()?.fields();
    if fields.len() != 7 {
        return None;
    }
    let width = u32::try_from(i32::try_from(&fields[0]).ok()?).ok()?;
    let height = u32::try_from(i32::try_from(&fields[1]).ok()?).ok()?;
    let stride = usize::try_from(i32::try_from(&fields[2]).ok()?).ok()?;
    let alpha = bool::try_from(&fields[3]).ok()?;
    let bits = i32::try_from(&fields[4]).ok()?;
    let channels = usize::try_from(i32::try_from(&fields[5]).ok()?).ok()?;
    if width == 0
        || height == 0
        || width > MAX_INPUT_EDGE
        || height > MAX_INPUT_EDGE
        || bits != 8
        || channels != if alpha { 4 } else { 3 }
    {
        return None;
    }
    let row = (width as usize).checked_mul(channels)?;
    if stride < row {
        return None;
    }
    let required = (height as usize - 1)
        .checked_mul(stride)?
        .checked_add(row)?;
    let data = <&Array>::try_from(&fields[6]).ok()?.inner();
    if required > MAX_INPUT_BYTES || data.len() < required || data.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let start = y * stride + x * channels;
            for channel in 0..3 {
                rgba.push(u8::try_from(&data[start + channel]).ok()?);
            }
            rgba.push(if alpha {
                u8::try_from(&data[start + 3]).ok()?
            } else {
                255
            });
        }
    }
    preview(DynamicImage::ImageRgba8(RgbaImage::from_raw(
        width, height, rgba,
    )?))
}
fn file(value: &str) -> Option<NotificationImage> {
    if value.len() > 4096 || value.contains('\0') {
        return None;
    }
    let path = if let Some(uri) = value.strip_prefix("file://") {
        let path = uri.strip_prefix("localhost").unwrap_or(uri);
        if !path.starts_with('/') || path.contains(['?', '#']) {
            return None;
        }
        PathBuf::from(
            percent_encoding::percent_decode_str(path)
                .decode_utf8()
                .ok()?
                .as_ref(),
        )
    } else {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return None;
        }
        path
    };
    // A FIFO/device must not hold the service's bounded decoder workers open.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_EDGE);
    limits.max_image_height = Some(MAX_INPUT_EDGE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    preview(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hint(
        w: i32,
        h: i32,
        stride: i32,
        alpha: bool,
        bits: i32,
        channels: i32,
        bytes: Vec<u8>,
    ) -> OwnedValue {
        zbus::zvariant::Value::from(Structure::from((
            w, h, stride, alpha, bits, channels, bytes,
        )))
        .try_into()
        .unwrap()
    }
    #[test]
    fn raw_rgb_rgba_stride_and_legacy_hints_preserve_pixels() {
        for key in ["image-data", "image_data", "icon_data"] {
            let hints = HashMap::from([(
                key.into(),
                hint(1, 2, 5, false, 8, 3, vec![10, 20, 30, 0, 0, 40, 50, 60]),
            )]);
            let image = decode(&hints).unwrap();
            assert_eq!((image.width(), image.height()), (1, 2));
            assert_eq!(image.pixels(), [10, 20, 30, 255, 40, 50, 60, 255]);
        }
        let hints = HashMap::from([(
            "image-data".into(),
            hint(
                2,
                1,
                8,
                true,
                8,
                4,
                vec![88, 101, 242, 128, 255, 255, 255, 0],
            ),
        )]);
        assert_eq!(
            decode(&hints).unwrap().pixels(),
            [88, 101, 242, 128, 255, 255, 255, 0]
        );
    }
    #[test]
    fn malformed_raw_data_is_ignored() {
        for (w, h, stride, alpha, bits, channels, bytes) in [
            (0, 1, 4, true, 8, 4, vec![0; 4]),
            (-1, 1, 4, true, 8, 4, vec![0; 4]),
            (4097, 1, 16388, true, 8, 4, vec![0; 4]),
            (1, 1, -1, true, 8, 4, vec![0; 4]),
            (2, 1, 4, true, 8, 4, vec![0; 8]),
            (1, 1, 4, true, 16, 4, vec![0; 4]),
            (1, 1, 4, false, 8, 4, vec![0; 4]),
            (1, 1, 3, true, 8, 3, vec![0; 3]),
            (2, 2, 8, true, 8, 4, vec![0; 15]),
            (4096, 4096, 16384, true, 8, 4, vec![0; 4]),
        ] {
            assert!(
                decode(&HashMap::from([(
                    "image-data".into(),
                    hint(w, h, stride, alpha, bits, channels, bytes)
                )]))
                .is_none()
            );
        }
        assert!(
            decode(&HashMap::from([(
                "image-data".into(),
                OwnedValue::from(true)
            )]))
            .is_none()
        );
    }
    #[test]
    fn local_png_jpeg_webp_and_file_uri_are_captured_and_scaled() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["png", "jpg", "webp"] {
            let path = dir.path().join(format!("preview image.{extension}"));
            image::RgbImage::from_pixel(640, 320, image::Rgb([88, 101, 242]))
                .save(&path)
                .unwrap();
            let image = file(path.to_str().unwrap()).unwrap();
            assert_eq!((image.width(), image.height()), (320, 160));
            assert!((image.pixels()[0] as i16 - 88).abs() <= 3);
            let uri = format!("file://{}", path.display()).replace(' ', "%20");
            assert_eq!(image, file(&uri).unwrap());
            std::fs::remove_file(path).unwrap();
            assert_eq!(image.pixels().len(), 320 * 160 * 4);
        }
    }
    #[test]
    fn raw_precedes_path_invalid_raw_falls_back_and_unsafe_paths_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 128]))
            .save(&path)
            .unwrap();
        let path_hint =
            || OwnedValue::try_from(zbus::zvariant::Value::from(path.to_str().unwrap())).unwrap();
        let mut hints = HashMap::from([
            ("image-path".into(), path_hint()),
            (
                "image-data".into(),
                hint(1, 1, 4, true, 8, 4, vec![4, 5, 6, 255]),
            ),
        ]);
        assert_eq!(decode(&hints).unwrap().pixels(), [4, 5, 6, 255]);
        hints.insert("image-data".into(), OwnedValue::from(false));
        assert_eq!(decode(&hints).unwrap().pixels(), [1, 2, 3, 128]);
        for value in [
            "https://example.com/a.png",
            "file://remote/tmp/a.png",
            "relative.png",
            "/dev/zero",
            "/does-not-exist",
        ] {
            assert!(file(value).is_none(), "{value}");
        }
        let fifo = dir.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(file(fifo.to_str().unwrap()).is_none());
        let oversized = dir.path().join("large.png");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_INPUT_BYTES as u64 + 1)
            .unwrap();
        assert!(file(oversized.to_str().unwrap()).is_none());
    }
}
