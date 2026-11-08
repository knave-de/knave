use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "knave-icons-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn png_bytes(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = (0..width * height).flat_map(|_| rgba).collect();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    out
}
const SVG_RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="#ff0000"/></svg>"##;
const SVG_HALF: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
const INDEX: &str = "[Icon Theme]\nName=T\nDirectories=48x48/apps,scalable/apps,512x512/apps\n\
[48x48/apps]\nSize=48\nType=Fixed\n\
[512x512/apps]\nSize=512\nType=Fixed\n\
[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n";

fn lookup(root: &Dir) -> IconLookup {
    IconLookup::new(vec![root.0.clone()], vec![root.0.join("pixmaps")])
}
fn pixel(icon: &Icon, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * icon.width + x) * 4) as usize;
    icon.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn svg_is_rasterized_to_the_requested_size_with_straight_alpha() {
    let d = Dir::new();
    d.write("hicolor/index.theme", INDEX.as_bytes());
    d.write("hicolor/scalable/apps/solid.svg", SVG_RED.as_bytes());
    d.write("hicolor/scalable/apps/half.svg", SVG_HALF.as_bytes());
    let mut icons = lookup(&d);
    let solid = icons.load("solid", 64).unwrap().unwrap();
    assert_eq!(
        (solid.width, solid.height, solid.rgba.len()),
        (64, 64, 64 * 64 * 4)
    );
    assert_eq!(pixel(&solid, 32, 32), [255, 0, 0, 255]);
    let half = icons.load("half", 32).unwrap().unwrap();
    let [r, g, b, a] = pixel(&half, 16, 16);
    // Premultiplied output would report r≈128 here.
    assert_eq!((r, g, b), (255, 0, 0));
    assert!((126..=129).contains(&a), "alpha {a}");
}

#[test]
fn the_closest_directory_wins_and_scalable_wins_ties() {
    let d = Dir::new();
    d.write("hicolor/index.theme", INDEX.as_bytes());
    d.write(
        "hicolor/48x48/apps/app.png",
        &png_bytes(48, 48, [0, 255, 0, 255]),
    );
    d.write(
        "hicolor/512x512/apps/onlybig.png",
        &png_bytes(512, 512, [0, 0, 255, 255]),
    );
    d.write(
        "hicolor/48x48/apps/both.png",
        &png_bytes(48, 48, [0, 255, 0, 255]),
    );
    d.write("hicolor/scalable/apps/both.svg", SVG_RED.as_bytes());
    let mut icons = lookup(&d);
    let near = icons.load("app", 48).unwrap().unwrap();
    assert_eq!((near.width, pixel(&near, 10, 10)), (48, [0, 255, 0, 255]));
    // A 512px source is averaged down instead of being uploaded at full size.
    let big = icons.load("onlybig", 64).unwrap().unwrap();
    assert_eq!((big.width, big.height), (64, 64));
    assert_eq!(pixel(&big, 10, 10), [0, 0, 255, 255]);
    let tie = icons.load("both", 48).unwrap().unwrap();
    assert_eq!(pixel(&tie, 24, 24), [255, 0, 0, 255]);
}

#[test]
fn hicolor_is_tried_before_other_themes_then_pixmaps() {
    let d = Dir::new();
    for theme in ["hicolor", "Adwaita"] {
        d.write(&format!("{theme}/index.theme"), INDEX.as_bytes());
    }
    d.write(
        "Adwaita/48x48/apps/shared.png",
        &png_bytes(48, 48, [0, 0, 255, 255]),
    );
    d.write(
        "hicolor/48x48/apps/shared.png",
        &png_bytes(48, 48, [0, 255, 0, 255]),
    );
    d.write(
        "Adwaita/48x48/apps/only-adwaita.png",
        &png_bytes(48, 48, [0, 0, 255, 255]),
    );
    d.write("pixmaps/loose.png", &png_bytes(32, 32, [9, 9, 9, 255]));
    let mut icons = lookup(&d);
    assert_eq!(
        pixel(&icons.load("shared", 48).unwrap().unwrap(), 1, 1),
        [0, 255, 0, 255]
    );
    assert_eq!(
        pixel(&icons.load("only-adwaita", 48).unwrap().unwrap(), 1, 1),
        [0, 0, 255, 255]
    );
    assert_eq!(
        pixel(&icons.load("loose", 32).unwrap().unwrap(), 1, 1),
        [9, 9, 9, 255]
    );
    assert!(icons.load("missing", 48).unwrap().is_none());
}

#[test]
fn names_cannot_escape_and_absolute_paths_must_be_icons() {
    let d = Dir::new();
    d.write("hicolor/index.theme", INDEX.as_bytes());
    d.write("secret.png", &png_bytes(8, 8, [1, 2, 3, 255]));
    d.write(
        "hicolor/48x48/apps/x.png",
        &png_bytes(48, 48, [1, 1, 1, 255]),
    );
    let mut icons = lookup(&d);
    for name in ["../../secret", "a/b", "..", "", "x\0y"] {
        assert!(icons.load(name, 48).unwrap().is_none(), "{name:?}");
    }
    let absolute = d.0.join("secret.png");
    assert!(icons.load(absolute.to_str().unwrap(), 8).unwrap().is_some());
    d.write("notes.txt", b"text");
    let text = d.0.join("notes.txt");
    assert!(icons.load(text.to_str().unwrap(), 8).unwrap().is_none());
    // Index entries that climb out of the theme are ignored.
    let escaping = "[Icon Theme]\nDirectories=../../x\n[../../x]\nSize=48\n";
    assert!(theme::parse_index(escaping).is_empty());
}

#[test]
fn corrupt_and_oversized_files_are_typed_errors() {
    let d = Dir::new();
    d.write("hicolor/index.theme", INDEX.as_bytes());
    d.write("hicolor/48x48/apps/bad.png", b"not a png");
    d.write("hicolor/scalable/apps/bad.svg", b"<svg");
    d.write("hicolor/48x48/apps/huge.png", &vec![0u8; 5 * 1024 * 1024]);
    let mut icons = lookup(&d);
    for name in ["bad", "huge"] {
        assert!(
            matches!(icons.load(name, 48), Err(IconError::Decode { .. })),
            "{name}"
        );
    }
}

#[test]
fn directory_distance_follows_the_spec() {
    let dirs = theme::parse_index(
        "[Icon Theme]\nDirectories=a,b,c\n[a]\nSize=48\nType=Fixed\n\
         [b]\nSize=32\nMinSize=16\nMaxSize=64\nType=Scalable\n\
         [c]\nSize=32\nThreshold=4\nType=Threshold\n",
    );
    assert_eq!(dirs[0].distance(64), 16);
    assert_eq!(
        [
            dirs[1].distance(8),
            dirs[1].distance(40),
            dirs[1].distance(80)
        ],
        [8, 0, 16]
    );
    assert_eq!(
        [
            dirs[2].distance(30),
            dirs[2].distance(40),
            dirs[2].distance(20)
        ],
        [0, 4, 8]
    );
}

#[test]
fn large_real_world_indexes_keep_every_directory() {
    // Distribution hicolor themes list well over 600 directories.
    let names: Vec<String> = (0..700).map(|n| format!("ctx{n}")).collect();
    let mut index = format!("[Icon Theme]\nDirectories={}\n", names.join(","));
    for name in &names {
        index.push_str(&format!("[{name}]\nSize=48\nType=Fixed\n"));
    }
    assert_eq!(theme::parse_index(&index).len(), 700);
    let d = Dir::new();
    d.write("hicolor/index.theme", index.as_bytes());
    d.write(
        "hicolor/ctx699/late.png",
        &png_bytes(48, 48, [7, 7, 7, 255]),
    );
    assert!(lookup(&d).load("late", 48).unwrap().is_some());
}

#[test]
fn special_files_are_never_opened() {
    let d = Dir::new();
    d.write("hicolor/index.theme", INDEX.as_bytes());
    fs::create_dir_all(d.0.join("hicolor/48x48/apps")).unwrap();
    let fifo = d.0.join("hicolor/48x48/apps/pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("skipping: mkfifo unavailable");
        return;
    }
    assert!(lookup(&d).load("pipe", 48).unwrap().is_none());
    let absolute = fifo.to_str().unwrap().to_owned();
    assert!(lookup(&d).load(&absolute, 48).unwrap().is_none());
}
