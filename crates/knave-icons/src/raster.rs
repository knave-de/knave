//! Decode PNG and SVG icons to straight-alpha RGBA8.
use crate::{Icon, IconError};
use resvg::{tiny_skia, usvg};
use std::{fs::File, io::Read, path::Path};

const MAX_PNG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SVG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PNG_DIMENSION: u32 = 2048;

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, IconError> {
    let fail = |reason: String| IconError::Decode {
        path: path.to_path_buf(),
        reason,
    };
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(fail("not a regular file".into()));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|e| fail(e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(fail(format!("file exceeds {limit} bytes")));
    }
    Ok(bytes)
}

pub(crate) fn decode(path: &Path, size: u32) -> Result<Icon, IconError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => png(path, size),
        Some("svg") => svg(path, size),
        _ => Err(IconError::Decode {
            path: path.to_path_buf(),
            reason: "unsupported icon format".into(),
        }),
    }
}

fn svg(path: &Path, size: u32) -> Result<Icon, IconError> {
    let fail = |reason: String| IconError::Decode {
        path: path.to_path_buf(),
        reason,
    };
    let bytes = read_bounded(path, MAX_SVG_BYTES)?;
    let mut options = usvg::Options::default();
    // Icons are self-contained; never let an SVG read other files.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = usvg::Tree::from_data(&bytes, &options).map_err(|e| fail(e.to_string()))?;
    let source = tree.size();
    let scale = (size as f32 / source.width()).min(size as f32 / source.height());
    if !scale.is_finite() || scale <= 0.0 {
        return Err(fail("invalid SVG size".into()));
    }
    let (w, h) = (source.width() * scale, source.height() * scale);
    let transform =
        tiny_skia::Transform::from_translate((size as f32 - w) / 2.0, (size as f32 - h) / 2.0)
            .pre_scale(scale, scale);
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or_else(|| fail("empty icon".into()))?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(Icon {
        width: size,
        height: size,
        rgba: pixmap.take_demultiplied(),
    })
}

fn png(path: &Path, size: u32) -> Result<Icon, IconError> {
    let fail = |reason: String| IconError::Decode {
        path: path.to_path_buf(),
        reason,
    };
    let bytes = read_bounded(path, MAX_PNG_BYTES)?;
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: 16 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| fail(e.to_string()))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
        return Err(fail(format!("unsupported dimensions {width}x{height}")));
    }
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| fail("too large".into()))?
    ];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|e| fail(e.to_string()))?;
    let data = &buffer[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(fail("palette was not expanded".into())),
    };
    // Sources far above the target are averaged down, or they alias when scaled.
    let factor = (width.max(height) / size.max(1)).max(1);
    if factor >= 2 {
        let (w, h, pixels) = box_downscale(&rgba, width, height, factor);
        return Ok(Icon {
            width: w,
            height: h,
            rgba: pixels,
        });
    }
    Ok(Icon {
        width,
        height,
        rgba,
    })
}

/// Average `factor`×`factor` blocks, weighting color by alpha to avoid dark fringes.
fn box_downscale(rgba: &[u8], width: u32, height: u32, factor: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = ((width / factor).max(1), (height / factor).max(1));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for dy in 0..factor {
                for dx in 0..factor {
                    let (sx, sy) = (
                        (x * factor + dx).min(width - 1),
                        (y * factor + dy).min(height - 1),
                    );
                    let i = ((sy * width + sx) * 4) as usize;
                    let alpha = u64::from(rgba[i + 3]);
                    r += u64::from(rgba[i]) * alpha;
                    g += u64::from(rgba[i + 1]) * alpha;
                    b += u64::from(rgba[i + 2]) * alpha;
                    a += alpha;
                    n += 1;
                }
            }
            match (
                r.checked_div(a),
                g.checked_div(a),
                b.checked_div(a),
                a.checked_div(n),
            ) {
                (Some(r), Some(g), Some(b), Some(a)) => {
                    out.extend([r as u8, g as u8, b as u8, a as u8])
                }
                _ => out.extend([0, 0, 0, 0]),
            }
        }
    }
    (w, h, out)
}
