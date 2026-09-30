use super::*;
use crate::appearance::*;

#[test]
fn appearance_writes_preserve_nested_unknown_and_inline_tables() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(
        &path,
        r##"schema_version = 2
[compositor.appearance]
custom = "keep"
focused = { opacity = 80, future = 42, blur = { enabled = true, radius = 6, custom = 9 } }
[compositor.appearance.gaps.outer]
top = 12
future_edge = 7
"##,
    )
    .unwrap();
    let mut document = ConfigDocument::load(&path).unwrap();
    let mut config = document.config().clone();
    config.compositor.appearance.gaps.outer.left = 20;
    config.compositor.appearance.focused.opacity = 75;
    document.write(config.clone()).unwrap();
    assert_eq!(ConfigDocument::load(&path).unwrap().config(), &config);
    let source: toml::Value = toml::from_str(&document.source()).unwrap();
    let appearance = &source["compositor"]["appearance"];
    assert_eq!(appearance["custom"].as_str(), Some("keep"));
    assert_eq!(appearance["focused"]["future"].as_integer(), Some(42));
    assert_eq!(
        appearance["focused"]["blur"]["custom"].as_integer(),
        Some(9)
    );
    assert_eq!(
        appearance["gaps"]["outer"]["future_edge"].as_integer(),
        Some(7)
    );
}

#[test]
fn invalid_effects_reject_before_touching_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let mut document = ConfigDocument::write_default_at(&path).unwrap();
    let source = fs::read_to_string(&path).unwrap();
    for change in [
        |a: &mut WindowAppearance| a.focused.opacity = 101,
        |a: &mut WindowAppearance| a.unfocused.blur.passes = 0,
        |a: &mut WindowAppearance| a.focused.blur.radius = 65,
        |a: &mut WindowAppearance| a.gaps.inner.top = 4097,
        |a: &mut WindowAppearance| a.border.width.left = 129,
        |a: &mut WindowAppearance| a.focused.border_color.top = "#gg0000".into(),
        |a: &mut WindowAppearance| a.unfocused.shadows.right.offset_x = 257,
    ] {
        let mut config = document.config().clone();
        change(&mut config.compositor.appearance);
        assert!(document.write(config).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    assert_eq!(rgba("#ff000080"), Some([1.0, 0.0, 0.0, 128.0 / 255.0]));
    assert!(rgba("#💥💥💥").is_none());
}

#[test]
fn shipped_appearance_examples_validate() {
    let config: Config = toml::from_str(include_str!("../../../config.example.toml")).unwrap();
    config.validate().unwrap();
    let reference = include_str!("../../../docs/architecture/window-appearance.md");
    let sample = reference
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let config: Config = toml::from_str(sample).unwrap();
    config.validate().unwrap();
}
