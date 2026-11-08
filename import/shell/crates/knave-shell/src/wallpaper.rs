//! Static wallpaper surface. Decodes once at startup and swaps images only
//! on desktop reload events, with no watcher or redraw timer.
use crate::wallpaper_reload::{AppliedWallpaper, ReloadUpdate, ReloadWorker};
use knave_ui::toolkit::{Input, TextMeasurer};
use knave_ui::{Color, DisplayList, DisplayListBuilder, ImageFit, ImageStyle, Rect, UiImage};
use knave_wayland::{Application, KeyboardMode, SurfaceLayer, SurfaceOptions, Waker};
use std::{fs::File, io::Cursor, io::Read as _, path::Path};

pub fn run(path: &str, fit: &str) -> Result<(), String> {
    let fit = image_fit(fit)?;
    let image = match load(Path::new(path)) {
        Ok(image) => Some(image),
        Err(error) => {
            eprintln!("knave-shell: wallpaper {path}: {error}; using black");
            None
        }
    };
    knave_wayland::run_application(
        SurfaceOptions {
            keyboard: KeyboardMode::None,
            layer: SurfaceLayer::Background,
            namespace: "knave-shell-wallpaper".into(),
            ..Default::default()
        },
        Wallpaper {
            path: Some(path.to_owned()),
            image,
            fit,
            list: DisplayList::new(0, Color::rgba(0, 0, 0, 255)),
            worker: None,
        },
    )
    .map_err(|e| e.to_string())
}
fn image_fit(fit: &str) -> Result<ImageFit, String> {
    match fit {
        "cover" => Ok(ImageFit::Cover),
        "contain" => Ok(ImageFit::Contain),
        "stretch" => Ok(ImageFit::Stretch),
        _ => Err("wallpaper fit must be cover, contain, or stretch".into()),
    }
}
pub(crate) fn load(path: &Path) -> Result<UiImage, String> {
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    if !path.is_absolute() || !path.is_file() {
        return Err("expected an absolute path to a regular PNG file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("image exceeds 64 MiB".into());
    }
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 128 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > 16384 || h > 16384 || u64::from(w) * u64::from(h) > 16 * 1024 * 1024
    {
        return Err("image exceeds 16384 per side or 16 Mi pixels".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("image is too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err("palette was not expanded".into()),
    };
    UiImage::from_rgba(w, h, pixels).ok_or_else(|| "invalid decoded image".into())
}
struct Wallpaper {
    path: Option<String>,
    image: Option<UiImage>,
    fit: ImageFit,
    list: DisplayList,
    worker: Option<ReloadWorker>,
}
impl Application for Wallpaper {
    fn uses_theme(&self) -> bool {
        false
    }

    fn set_waker(&mut self, waker: Waker) {
        // The reload thread's lifetime matches this application's.
        self.worker = Some(ReloadWorker::start(
            waker,
            AppliedWallpaper {
                path: self.path.clone(),
                fit: self.fit,
            },
            load,
        ));
    }
    fn input(&mut self, _: Input) {}
    fn needs_frame(&self) -> bool {
        self.worker.as_ref().is_some_and(ReloadWorker::pending)
    }
    fn frame(&mut self, size: [f32; 2], _: &mut dyn TextMeasurer) -> &DisplayList {
        match self.worker.as_ref().and_then(ReloadWorker::take_update) {
            Some(ReloadUpdate::Image { fit, image }) => {
                self.fit = fit;
                self.image = image;
            }
            Some(ReloadUpdate::FitChange { fit }) => self.fit = fit,
            None => {}
        }
        let mut builder = DisplayListBuilder::new(
            self.list.revision.wrapping_add(1),
            Color::rgba(0, 0, 0, 255),
        );
        if let Some(image) = &self.image {
            builder.image(
                Rect::new(0.0, 0.0, size[0], size[1]),
                image.clone(),
                ImageStyle {
                    fit: self.fit,
                    ..Default::default()
                },
            );
        }
        self.list = builder.finish().expect("balanced wallpaper display list");
        &self.list
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_decoding_preserves_pixels_and_rejects_bad_input() {
        assert!(load(Path::new("relative.png")).is_err());
        let path = std::env::temp_dir().join(format!("wallpaper-{}.png", std::process::id()));
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[10, 20, 30])
                .unwrap();
        }
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(load(&path).unwrap().pixels(), &[10, 20, 30, 255]);
        std::fs::write(&path, b"bad image").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
