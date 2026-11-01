//! Atomic loading and validation of Villain's effective configuration.

use std::{
    collections::BTreeMap,
    env, fmt, fs,
    path::{Path, PathBuf},
};

use knave_config::ConfigDocument;

use crate::keybinds::KeybindRegistry;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputConfig {
    pub tap_to_click: bool,
    pub natural_scroll: bool,
    pub focus_follows_mouse: bool,
    pub allow_virtual_keyboard: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scroll: true,
            focus_follows_mouse: true,
            allow_virtual_keyboard: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Output scale normalized to Wayland's 1/120 fractional-scale units.
    pub output_scale: f64,
    pub master_percent: u8,
    pub focus_cycle_order: knave_config::FocusCycleOrder,
    pub switch_to_new_window_workspace: bool,
    pub workspace: knave_config::CompositorWorkspaceConfig,
    pub appearance: knave_config::WindowAppearance,
    pub input: InputConfig,
    /// Ordered `[[compositor.device]]` rules; later matches override earlier ones.
    pub devices: Vec<knave_config::CompositorDeviceConfig>,
    pub window_rules: WindowRulesRuntime,
    pub keybinds: KeybindRegistry,
    pub gestures: Vec<knave_config::gesture::GestureBinding>,
    pub environment: BTreeMap<String, String>,
    path: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct WindowRulesRuntime {
    applications: Vec<knave_config::WindowApplicationRule>,
    rules: Vec<CompiledWindowRule>,
}

#[derive(Clone, Debug)]
struct CompiledWindowRule {
    matcher: knave_config::WindowRuleMatch,
    title_regex: Option<regex::Regex>,
    set: knave_config::WindowRuleActions,
}

impl WindowRulesRuntime {
    fn compile(config: &knave_config::WindowRulesConfig) -> Self {
        Self {
            applications: config.application.clone(),
            rules: config
                .rule
                .iter()
                .map(|rule| CompiledWindowRule {
                    matcher: rule.matcher.clone(),
                    title_regex: rule.matcher.title_regex.as_ref().map(|pattern| {
                        regex::RegexBuilder::new(pattern)
                            .size_limit(1 << 20)
                            .dfa_size_limit(1 << 20)
                            .build()
                            .expect("Knave config validates window rule regexes")
                    }),
                    set: rule.set.clone(),
                })
                .collect(),
        }
    }

    pub fn for_window(&self, app_id: &str, title: &str) -> knave_config::WindowRuleActions {
        let mut result = knave_config::WindowRuleActions::default();
        for rule in &self.applications {
            if rule.app_id == app_id {
                overlay_window_actions(&mut result, &rule.set);
            }
        }
        for rule in &self.rules {
            let matcher = &rule.matcher;
            let matches = matcher
                .app_id
                .as_deref()
                .is_none_or(|value| value == app_id)
                && matcher
                    .title_contains
                    .as_deref()
                    .is_none_or(|value| title.contains(value))
                && rule
                    .title_regex
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(title));
            if matches {
                overlay_window_actions(&mut result, &rule.set);
            }
        }
        result
    }
}

fn overlay_window_actions(
    target: &mut knave_config::WindowRuleActions,
    source: &knave_config::WindowRuleActions,
) {
    if source.placement.is_some() {
        target.placement = source.placement;
    }
    if source.center.is_some() {
        target.center = source.center;
    }
    if source.stay_on_top.is_some() {
        target.stay_on_top = source.stay_on_top;
    }
    if source.workspace.is_some() {
        target.workspace = source.workspace;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BindSpec {
    pub keys: String,
    pub dispatch: String,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ConfigError {}

impl RuntimeConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(
            ConfigDocument::default_path().map_err(|error| ConfigError(error.to_string()))?,
        )
    }

    pub fn reload(&self) -> Result<Self, ConfigError> {
        Self::load_from(self.path.clone())
    }

    fn load_from(path: PathBuf) -> Result<Self, ConfigError> {
        let document =
            ConfigDocument::load(&path).map_err(|error| ConfigError(error.to_string()))?;
        let compositor = &document.config().compositor;
        let output_scale = (compositor.scale * 120.0).round() / 120.0;
        let bindings: Vec<BindSpec> = compositor
            .bind
            .iter()
            .map(|binding| BindSpec {
                keys: binding.keys.clone(),
                dispatch: binding.dispatch.clone(),
                args: binding.args.clone(),
            })
            .collect();
        // Explicit bindings override matching defaults in place; other
        // defaults stay active so one custom binding keeps the rest of the set.
        let keybinds =
            KeybindRegistry::with_overrides(&compositor.modkey, &bindings).map_err(ConfigError)?;

        let mut environment = default_environment();
        let default_environment_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("environment");
        let (environment_path, required) = match compositor.environment_file.as_deref() {
            Some(value) => (resolve_path(value, path.parent())?, true),
            None => (default_environment_path, false),
        };
        match fs::read_to_string(&environment_path) {
            Ok(contents) => parse_environment(&contents, &environment_path, &mut environment)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {}
            Err(error) => {
                return Err(ConfigError(format!(
                    "could not read {}: {error}",
                    environment_path.display()
                )));
            }
        }

        Ok(Self {
            output_scale,
            master_percent: compositor.master_percent,
            focus_cycle_order: compositor.focus_cycle_order,
            switch_to_new_window_workspace: compositor.switch_to_new_window_workspace,
            workspace: compositor.workspace,
            appearance: compositor.appearance.clone(),
            input: InputConfig {
                tap_to_click: compositor.input.tap_to_click,
                natural_scroll: compositor.input.natural_scroll,
                focus_follows_mouse: compositor.input.focus_follows_mouse,
                allow_virtual_keyboard: compositor.input.allow_virtual_keyboard,
            },
            devices: compositor.device.clone(),
            window_rules: WindowRulesRuntime::compile(&compositor.window_rules),
            gestures: crate::gestures::bindings(&compositor.gesture),
            keybinds,
            environment,
            path,
        })
    }
}

fn resolve_path(value: &str, base: Option<&Path>) -> Result<PathBuf, ConfigError> {
    if value == "~" || value.starts_with("~/") {
        let home = env::var_os("HOME")
            .ok_or_else(|| ConfigError("HOME is not set; cannot expand ~".into()))?;
        return Ok(if value == "~" {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(&value[2..])
        });
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(base.unwrap_or_else(|| Path::new(".")).join(path))
    }
}

fn default_environment() -> BTreeMap<String, String> {
    [
        ("XDG_SESSION_TYPE", "wayland"),
        ("XDG_CURRENT_DESKTOP", "Villain"),
        ("XDG_SESSION_DESKTOP", "villain"),
        ("ELECTRON_OZONE_PLATFORM_HINT", "wayland"),
        ("MOZ_ENABLE_WAYLAND", "1"),
        ("QT_QPA_PLATFORM", "wayland"),
        ("GDK_BACKEND", "wayland"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn parse_environment(
    contents: &str,
    path: &Path,
    output: &mut BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    for (index, original) in contents.lines().enumerate() {
        let line = original.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(ConfigError(format!(
                "{}:{}: expected KEY=VALUE",
                path.display(),
                index + 1
            )));
        };
        let key = key.trim();
        if !valid_environment_key(key) {
            return Err(ConfigError(format!(
                "{}:{}: invalid environment variable name {key:?}",
                path.display(),
                index + 1
            )));
        }
        let value = strip_matching_quotes(value.trim()).ok_or_else(|| {
            ConfigError(format!(
                "{}:{}: environment value has unmatched quotes",
                path.display(),
                index + 1
            ))
        })?;
        output.insert(key.into(), value.into());
    }
    Ok(())
}

fn valid_environment_key(key: &str) -> bool {
    let mut characters = key.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn strip_matching_quotes(value: &str) -> Option<&str> {
    let quoted = (value.starts_with('"'), value.ends_with('"'));
    let single_quoted = (value.starts_with('\''), value.ends_with('\''));
    match (quoted, single_quoted) {
        ((true, true), _) | (_, (true, true)) if value.len() >= 2 => {
            Some(&value[1..value.len() - 1])
        }
        ((false, false), (false, false)) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_wayland_native_and_touchpad_friendly() {
        assert!(InputConfig::default().tap_to_click);
        assert!(InputConfig::default().natural_scroll);
        assert!(InputConfig::default().focus_follows_mouse);
        assert!(InputConfig::default().allow_virtual_keyboard);
        assert_eq!(default_environment()["XDG_SESSION_TYPE"], "wayland");
        assert_eq!(
            default_environment()["ELECTRON_OZONE_PLATFORM_HINT"],
            "wayland"
        );
    }

    #[test]
    fn environment_parser_is_data_not_shell() {
        let mut values = BTreeMap::new();
        parse_environment(
            "# comment\nexport NAME=Villain\nLITERAL='$HOME and spaces'\n",
            Path::new("environment"),
            &mut values,
        )
        .unwrap();
        assert_eq!(values["NAME"], "Villain");
        assert_eq!(values["LITERAL"], "$HOME and spaces");
    }

    #[test]
    fn invalid_environment_line_rejects_complete_load() {
        let mut values = BTreeMap::new();
        assert!(
            parse_environment("NOT VALID=value", Path::new("environment"), &mut values).is_err()
        );
        assert!(values.is_empty());
    }

    #[test]
    fn shipped_examples_are_valid() {
        let config: knave_config::Config =
            toml::from_str(include_str!("../config.example.toml")).unwrap();
        config.validate().unwrap();
        let specs: Vec<BindSpec> = config
            .compositor
            .bind
            .iter()
            .map(|binding| BindSpec {
                keys: binding.keys.clone(),
                dispatch: binding.dispatch.clone(),
                args: binding.args.clone(),
            })
            .collect();
        KeybindRegistry::from_specs(&config.compositor.modkey, &specs).unwrap();
        let mut environment = BTreeMap::new();
        parse_environment(
            include_str!("../environment.example"),
            Path::new("environment.example"),
            &mut environment,
        )
        .unwrap();

        assert_eq!(environment["XDG_SESSION_TYPE"], "wayland");
    }

    #[test]
    fn reload_validates_before_returning_replacement() {
        let directory =
            std::env::temp_dir().join(format!("villain-config-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            "[compositor]\nmodkey = \"Alt\"\nfocus_cycle_order = \"mru\"\n[compositor.input]\ntap_to_click = false\nnatural_scroll = false\nfocus_follows_mouse = false\nallow_virtual_keyboard = false\n",
        )
        .unwrap();
        let current = RuntimeConfig::load_from(path.clone()).unwrap();
        assert!(!current.input.tap_to_click);
        assert!(!current.input.natural_scroll);
        assert!(!current.input.focus_follows_mouse);
        assert!(!current.input.allow_virtual_keyboard);
        assert_eq!(
            current.focus_cycle_order,
            knave_config::FocusCycleOrder::Mru
        );

        fs::write(&path, "[compositor]\nmodkey = \"Invalid\"\n").unwrap();
        assert!(current.reload().is_err());
        assert!(!current.input.tap_to_click);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_scale_is_projected_as_wayland_fractional_units() {
        let directory =
            std::env::temp_dir().join(format!("villain-output-scale-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, "[compositor]\nscale = 1.5\n").unwrap();

        let config = RuntimeConfig::load_from(path.clone()).unwrap();
        assert_eq!(config.output_scale, 1.5);
        fs::write(&path, "[compositor]\nscale = 1.333\n").unwrap();
        let config = RuntimeConfig::load_from(path).unwrap();
        assert_eq!(config.output_scale, 160.0 / 120.0);

        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn compositor_settings_are_projected_by_knave() {
        let directory =
            std::env::temp_dir().join(format!("villain-compositor-config-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            r#"[compositor]
modkey = "Alt"
[compositor.input]
tap_to_click = false
natural_scroll = false
[[compositor.bind]]
keys = "MOD+Q"
dispatch = "close"
"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_from(path).unwrap();
        assert!(!config.input.tap_to_click);
        assert!(!config.input.natural_scroll);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_bindings_override_matches_and_keep_other_defaults() {
        use crate::dispatch::Dispatch;
        use smithay::input::keyboard::{Keysym, keysyms};

        let directory =
            std::env::temp_dir().join(format!("villain-keybind-config-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            r#"[compositor]
modkey = "Super"

[[compositor.bind]]
keys = "MOD+Q"
dispatch = "exec"
args = ["bemenu"]
"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_from(path).unwrap();
        let q = [Keysym::new(keysyms::KEY_q)];
        assert_eq!(
            config.keybinds.find(&q, false, false, false, true),
            Some(Dispatch::Spawn(vec!["bemenu".into()]))
        );
        // An unmentioned default binding survives the override.
        assert_eq!(
            config.keybinds.find(
                &[Keysym::new(keysyms::KEY_Return)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["kitty".into()]))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn window_rules_apply_in_layers_and_later_rules_override_only_set_fields() {
        let directory =
            std::env::temp_dir().join(format!("villain-window-rules-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            r#"
[compositor.workspace]
predefined = 3
[[compositor.window_rules.application]]
app_id = "firefox"
workspace = 3
placement = "tiled"
[[compositor.window_rules.rule]]
[compositor.window_rules.rule.match]
app_id = "firefox"
title_contains = "Picture"
[compositor.window_rules.rule.set]
placement = "floating"
center = true
[[compositor.window_rules.rule]]
[compositor.window_rules.rule.match]
title_regex = '(?i)^picture.*in.picture$'
[compositor.window_rules.rule.set]
stay_on_top = true
"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_from(path).unwrap();
        let matched = config
            .window_rules
            .for_window("firefox", "Picture-in-Picture");
        assert_eq!(
            matched.placement,
            Some(knave_config::WindowPlacement::Floating)
        );
        assert_eq!(matched.center, Some(true));
        assert_eq!(matched.workspace, Some(3));
        assert_eq!(matched.stay_on_top, Some(true));
        let unrelated = config.window_rules.for_window("firefox", "Settings");
        assert_eq!(
            unrelated.placement,
            Some(knave_config::WindowPlacement::Tiled)
        );
        assert_eq!(unrelated.workspace, Some(3));
        assert_eq!(unrelated.center, None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn device_rules_are_projected_from_the_knave_configuration() {
        let directory =
            std::env::temp_dir().join(format!("villain-device-config-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            r#"[compositor]
modkey = "Super"

[[compositor.device]]
name = "*Touchpad"
enabled = false
sensitivity = 0.5
scroll_method = "two_finger"
"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_from(path).unwrap();
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].name, "*Touchpad");
        assert_eq!(config.devices[0].enabled, Some(false));
        assert_eq!(config.devices[0].sensitivity, Some(0.5));
        assert_eq!(
            config.devices[0].scroll_method,
            Some(knave_config::DeviceScrollMethod::TwoFinger)
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
