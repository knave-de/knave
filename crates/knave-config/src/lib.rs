//! Canonical Knave configuration loading, validation, and atomic writing.

pub mod quick_settings;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

pub mod appearance;
pub mod gesture;
pub mod notifications;
pub use appearance::WindowAppearance;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum SessionBackend {
    #[default]
    Auto,
    Tty,
    Winit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub backend: SessionBackend,
    #[serde(default = "default_compositor_binary")]
    pub compositor_binary: String,
    #[serde(default = "default_shell_binary")]
    pub shell_binary: String,
    #[serde(default = "default_true")]
    pub restart_on_failure: bool,
    #[serde(default)]
    pub startup: Vec<StartupCommand>,
}

/// A process launched once the compositor and shell are ready.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StartupCommand {
    /// Executable followed by its arguments; no shell interpretation is applied.
    pub command: Vec<String>,
}

fn default_compositor_binary() -> String {
    "villain".into()
}

fn default_shell_binary() -> String {
    "knave-shell".into()
}

fn default_true() -> bool {
    true
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            backend: SessionBackend::default(),
            compositor_binary: default_compositor_binary(),
            shell_binary: default_shell_binary(),
            restart_on_failure: true,
            startup: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellConfig {
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    #[serde(default = "default_true")]
    pub start_bar: bool,
    #[serde(default = "default_true")]
    pub start_overview_service: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub fit: WallpaperFit,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperFit {
    #[default]
    Cover,
    Contain,
    Stretch,
}
impl WallpaperFit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Stretch => "stretch",
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            wallpaper: WallpaperConfig::default(),
            start_bar: true,
            start_overview_service: true,
        }
    }
}
/// Walk order that focus-switching cycle shortcuts use across windows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusCycleOrder {
    /// Master first, then stack windows top to bottom: the visible layout order.
    Layout,
    /// Most recently focused window first.
    Mru,
}

impl FocusCycleOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Mru => "mru",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositorInputConfig {
    #[serde(default = "default_true")]
    pub tap_to_click: bool,
    #[serde(default = "default_true")]
    pub natural_scroll: bool,
    /// Move keyboard focus to the window under the pointer while it moves.
    /// `false` keeps focus on click only.
    #[serde(default = "default_true")]
    pub focus_follows_mouse: bool,
}

impl Default for CompositorInputConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scroll: true,
            focus_follows_mouse: true,
        }
    }
}

/// Pointer acceleration profile applied to a matched input device.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAccelProfile {
    Adaptive,
    Flat,
}

impl DeviceAccelProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Flat => "flat",
        }
    }
}

/// Scroll method applied to a matched input device.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceScrollMethod {
    NoScroll,
    TwoFinger,
    Edge,
    OnButtonDown,
}

impl DeviceScrollMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoScroll => "no_scroll",
            Self::TwoFinger => "two_finger",
            Self::Edge => "edge",
            Self::OnButtonDown => "on_button_down",
        }
    }
}

/// Click method applied to a matched input device.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClickMethod {
    ButtonAreas,
    Clickfinger,
}

impl DeviceClickMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ButtonAreas => "button_areas",
            Self::Clickfinger => "clickfinger",
        }
    }
}

/// One `[[compositor.device]]` rule for a libinput device.
///
/// `name` is matched case-insensitively against the runtime device name and
/// supports the `*` and `?` wildcards. Every other field is optional; an unset
/// field leaves the underlying libinput value untouched. When several rules
/// match the same device they apply in file order, so a later rule overrides an
/// earlier one. `enabled = false` stops libinput delivering events from the
/// device without detaching it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CompositorDeviceConfig {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_to_click: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_and_drag: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag_lock: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural_scroll: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_while_typing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middle_emulation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_handed: Option<bool>,
    /// Pointer acceleration speed in -1.0..=1.0; `0.0` is the device default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_profile: Option<DeviceAccelProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_method: Option<DeviceScrollMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_button: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_method: Option<DeviceClickMethod>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositorBinding {
    pub keys: String,
    pub dispatch: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Policy for next/previous navigation; explicit workspace selection is unaffected.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompositorWorkspaceConfig {
    /// Number of fixed workspaces at the start of the workspace list.
    #[serde(default = "default_predefined_workspaces")]
    pub predefined: u32,
    #[serde(default)]
    pub skip_empty: bool,
}

/// Initial window behavior. Rules run once when a window first opens.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRuleApply {
    #[default]
    OnOpen,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPlacement {
    Tiled,
    Floating,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowRuleActions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<WindowPlacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stay_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<u8>,
}

/// Preferences applied to every window with this exact app_id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowApplicationRule {
    pub app_id: String,
    #[serde(flatten)]
    pub set: WindowRuleActions,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowRuleMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_regex: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "match")]
    pub matcher: WindowRuleMatch,
    pub set: WindowRuleActions,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WindowRulesConfig {
    #[serde(default)]
    pub apply: WindowRuleApply,
    #[serde(default)]
    pub application: Vec<WindowApplicationRule>,
    #[serde(default)]
    pub rule: Vec<WindowRule>,
}

pub const MAX_PREDEFINED_WORKSPACES: u32 = 4095;

fn default_predefined_workspaces() -> u32 {
    1
}

impl Default for CompositorWorkspaceConfig {
    fn default() -> Self {
        Self {
            predefined: default_predefined_workspaces(),
            skip_empty: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompositorConfig {
    /// Logical-to-buffer output scale. Villain rounds to the nearest 1/120.
    #[serde(default = "default_output_scale")]
    pub scale: f64,
    #[serde(default)]
    pub workspace: CompositorWorkspaceConfig,
    #[serde(default)]
    pub window_rules: WindowRulesConfig,
    #[serde(default)]
    pub gesture: gesture::GestureConfig,
    #[serde(default)]
    pub appearance: WindowAppearance,
    /// Default master width as a percentage of usable workspace width (10..=90).
    #[serde(default = "default_master_percent")]
    pub master_percent: u8,
    #[serde(default = "default_modkey")]
    pub modkey: String,
    /// Order focus-switching cycle shortcuts walk a workspace's windows in.
    #[serde(default = "default_focus_cycle_order")]
    pub focus_cycle_order: FocusCycleOrder,
    #[serde(default)]
    pub environment_file: Option<String>,
    #[serde(default)]
    pub input: CompositorInputConfig,
    /// Per-device overrides, matched by name and applied in order.
    #[serde(default)]
    pub device: Vec<CompositorDeviceConfig>,
    #[serde(default)]
    pub bind: Vec<CompositorBinding>,
}

fn default_master_percent() -> u8 {
    50
}

fn default_output_scale() -> f64 {
    1.0
}

fn default_modkey() -> String {
    "Super".into()
}

fn default_focus_cycle_order() -> FocusCycleOrder {
    FocusCycleOrder::Layout
}

impl Default for CompositorConfig {
    fn default() -> Self {
        Self {
            scale: default_output_scale(),
            workspace: CompositorWorkspaceConfig::default(),
            window_rules: WindowRulesConfig::default(),
            gesture: gesture::GestureConfig::default(),
            appearance: WindowAppearance::default(),
            master_percent: default_master_percent(),
            modkey: default_modkey(),
            focus_cycle_order: default_focus_cycle_order(),
            environment_file: None,
            input: CompositorInputConfig::default(),
            device: Vec::new(),
            bind: Vec::new(),
        }
    }
}

/// Desktop dark/light preference shared by the shell, compositor and portal.
///
/// `system` reports no preference; `dark` and `light` force the matching
/// `org.freedesktop.appearance` `color-scheme`. This slice persists the
/// preference only; consumers adopt it as their theme layers land.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    /// Stable lowercase spelling used in configuration and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// `color-scheme` value for `org.freedesktop.appearance` (0/1/2).
    pub fn color_scheme(self) -> u8 {
        match self {
            Self::System => 0,
            Self::Dark => 1,
            Self::Light => 2,
        }
    }
}

/// Requested contrast strength for desktop UI chrome.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContrastPreference {
    #[default]
    Standard,
    High,
}
impl ContrastPreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::High => "high",
        }
    }
}

/// Whether desktop UI chrome uses a color accent or a neutral palette.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChromaPreference {
    #[default]
    FullColor,
    Monochrome,
}
impl ChromaPreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullColor => "full-color",
            Self::Monochrome => "monochrome",
        }
    }
}

/// Desktop appearance preference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppearanceConfig {
    pub theme: Theme,
    pub theme_name: String,
    pub contrast: ContrastPreference,
    pub chroma: ChromaPreference,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            theme_name: "knave".into(),
            contrast: ContrastPreference::Standard,
            chroma: ChromaPreference::FullColor,
        }
    }
}

impl Serialize for AppearanceConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Selection<'a> {
            theme: &'a str,
            mode: Theme,
            contrast: ContrastPreference,
            chroma: ChromaPreference,
        }
        #[derive(Serialize)]
        struct Output<'a> {
            theme: Selection<'a>,
        }
        Output {
            theme: Selection {
                theme: &self.theme_name,
                mode: self.theme,
                contrast: self.contrast,
                chroma: self.chroma,
            },
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AppearanceConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Selection {
            #[serde(default = "default_theme_name")]
            theme: String,
            #[serde(default)]
            mode: Theme,
            #[serde(default)]
            contrast: ContrastPreference,
            #[serde(default)]
            chroma: ChromaPreference,
        }
        #[derive(Deserialize, Default)]
        struct Input {
            theme: Option<Selection>,
        }
        let input = Input::deserialize(deserializer)?;
        Ok(match input.theme {
            Some(Selection {
                theme,
                mode,
                contrast,
                chroma,
            }) => Self {
                theme: mode,
                theme_name: theme,
                contrast,
                chroma,
            },
            None => Self::default(),
        })
    }
}
fn default_theme_name() -> String {
    "knave".into()
}

/// Appearance projection for the portal backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortalConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub color_scheme: u8,
    #[serde(default = "default_accent")]
    pub accent_color: String,
    #[serde(default)]
    pub high_contrast: bool,
    #[serde(default)]
    pub reduced_motion: bool,
}
fn default_accent() -> String {
    "0.21,0.52,0.89".into()
}
impl Default for PortalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            color_scheme: 0,
            accent_color: default_accent(),
            high_contrast: false,
            reduced_motion: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Config {
    #[serde(default)]
    pub quick_settings: quick_settings::QuickSettingsConfig,
    #[serde(default)]
    pub notifications: notifications::NotificationConfig,
    #[serde(default)]
    pub portal: PortalConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub compositor: CompositorConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub shell: ShellConfig,
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.appearance.theme_name.is_empty()
            || !self
                .appearance
                .theme_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ConfigError::Invalid(
                "appearance.theme.theme must be a simple theme name".into(),
            ));
        }
        self.notifications.validate()?;
        self.quick_settings.validate()?;
        let accent: Vec<_> = self
            .portal
            .accent_color
            .split(',')
            .map(str::trim)
            .map(str::parse::<f64>)
            .collect();
        if self.portal.color_scheme > 2
            || accent.len() != 3
            || accent.iter().any(|n| {
                n.as_ref()
                    .map_or(true, |n| !n.is_finite() || !(0.0..=1.0).contains(n))
            })
        {
            return Err(ConfigError::Invalid("portal appearance requires color_scheme 0..2 and three finite accent components in 0..1".into()));
        }
        if self.session.compositor_binary.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "session.compositor_binary is empty".into(),
            ));
        }
        if self.session.shell_binary.trim().is_empty() {
            return Err(ConfigError::Invalid("session.shell_binary is empty".into()));
        }
        if self.session.startup.len() > 64 {
            return Err(ConfigError::Invalid(
                "session.startup must contain at most 64 commands".into(),
            ));
        }
        for (index, startup) in self.session.startup.iter().enumerate() {
            if startup.command.is_empty()
                || startup.command[0].trim().is_empty()
                || startup.command.iter().any(|part| part.contains('\0'))
            {
                return Err(ConfigError::Invalid(format!(
                    "session.startup[{index}].command must have a non-empty executable and contain no NUL characters"
                )));
            }
        }
        if !(10..=90).contains(&self.compositor.master_percent) {
            return Err(ConfigError::Invalid(
                "compositor.master_percent must be between 10 and 90".into(),
            ));
        }
        if !self.compositor.scale.is_finite() || !(1.0..=4.0).contains(&self.compositor.scale) {
            return Err(ConfigError::Invalid(
                "compositor.scale must be finite and between 1 and 4".into(),
            ));
        }
        if !(1..=MAX_PREDEFINED_WORKSPACES).contains(&self.compositor.workspace.predefined) {
            return Err(ConfigError::Invalid(
                "compositor.workspace.predefined must be between 1 and 4095".into(),
            ));
        }
        self.compositor.appearance.validate()?;
        self.compositor.gesture.validate()?;
        for (index, rule) in self.compositor.window_rules.rule.iter().enumerate() {
            let matcher = &rule.matcher;
            if matcher
                .app_id
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
                && matcher
                    .title_contains
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                && matcher
                    .title_regex
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
            {
                return Err(ConfigError::Invalid(format!(
                    "compositor.window_rules.rule[{index}].match needs at least one condition"
                )));
            }
            if let Some(pattern) = &matcher.title_regex {
                if pattern.len() > 1024 {
                    return Err(ConfigError::Invalid(format!(
                        "compositor.window_rules.rule[{index}].match.title_regex exceeds 1024 bytes"
                    )));
                }
                regex::RegexBuilder::new(pattern)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                    .map_err(|error| ConfigError::Invalid(format!(
                        "compositor.window_rules.rule[{index}].match.title_regex is invalid: {error}"
                    )))?;
            }
            validate_window_rule_actions(
                &rule.set,
                &format!("rule[{index}].set"),
                self.compositor.workspace.predefined,
            )?;
        }
        for (index, rule) in self.compositor.window_rules.application.iter().enumerate() {
            if rule.app_id.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "compositor.window_rules.application[{index}].app_id must not be empty"
                )));
            }
            validate_window_rule_actions(
                &rule.set,
                &format!("application[{index}]"),
                self.compositor.workspace.predefined,
            )?;
        }
        if let Some(image) = &self.shell.wallpaper.image
            && (!Path::new(image).is_absolute() || image.contains('\0'))
        {
            return Err(ConfigError::Invalid(
                "shell.wallpaper.image must be an absolute path".into(),
            ));
        }
        if self.compositor.modkey.trim().is_empty() {
            return Err(ConfigError::Invalid("compositor.modkey is empty".into()));
        }
        for device in &self.compositor.device {
            if device.name.trim().is_empty() {
                return Err(ConfigError::Invalid(
                    "compositor.device.name must not be empty".into(),
                ));
            }
            if let Some(sensitivity) = device.sensitivity
                && (!sensitivity.is_finite() || !(-1.0..=1.0).contains(&sensitivity))
            {
                return Err(ConfigError::Invalid(
                    "compositor.device.sensitivity must be between -1.0 and 1.0".into(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_window_rule_actions(
    actions: &WindowRuleActions,
    path: &str,
    predefined_workspaces: u32,
) -> Result<(), ConfigError> {
    let max_workspace = predefined_workspaces.min(10);
    if actions
        .workspace
        .is_some_and(|workspace| workspace == 0 || u32::from(workspace) > max_workspace)
    {
        return Err(ConfigError::Invalid(format!(
            "compositor.window_rules.{path}.workspace must be between 1 and {max_workspace} (the configured retained workspace range)"
        )));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine Knave configuration path: {0}")]
    Path(String),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not parse editable document {path}: {source}")]
    Document {
        path: PathBuf,
        source: toml_edit::TomlError,
    },
    #[error("invalid Knave configuration: {0}")]
    Invalid(String),
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}
pub struct ConfigDocument {
    path: PathBuf,
    document: DocumentMut,
    config: Config,
}

impl ConfigDocument {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Read { path, source });
            }
        };

        let document = if source.trim().is_empty() {
            DocumentMut::new()
        } else {
            source
                .parse::<DocumentMut>()
                .map_err(|source| ConfigError::Document {
                    path: path.clone(),
                    source,
                })?
        };
        let config = if source.trim().is_empty() {
            Config::default()
        } else {
            toml::from_str::<Config>(&source).map_err(|source| ConfigError::Parse {
                path: path.clone(),
                source,
            })?
        };
        config.validate()?;

        Ok(Self {
            path,
            document,
            config,
        })
    }

    pub fn at_default_path() -> Result<Self, ConfigError> {
        Self::load(config_path()?)
    }

    pub fn default_path() -> Result<PathBuf, ConfigError> {
        config_path()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn source(&self) -> String {
        self.document.to_string()
    }

    pub fn write(&mut self, config: Config) -> Result<(), ConfigError> {
        config.validate()?;
        apply_config(&mut self.document, &config);
        let source = self.document.to_string();
        atomic_write(&self.path, source.as_bytes())?;
        self.config = config;
        Ok(())
    }

    pub fn write_default_at(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        if path.exists() {
            return Err(ConfigError::Invalid(format!(
                "configuration already exists at {}",
                path.display()
            )));
        }
        let mut document = Self::load(path.clone())?;
        document.write(Config::default())?;
        Ok(document)
    }
}

fn apply_config(document: &mut DocumentMut, config: &Config) {
    if let Some(table) = document
        .get("quick_settings")
        .and_then(Item::as_inline_table)
        .cloned()
    {
        document["quick_settings"] = Item::Table(table.into_table());
    }
    let quick = toml::to_string(&config.quick_settings)
        .expect("validated settings serialize")
        .parse::<DocumentMut>()
        .expect("settings TOML");
    if let Some(table) = document
        .get_mut("quick_settings")
        .and_then(Item::as_table_mut)
    {
        if config.quick_settings.charge_limit.is_none() {
            table.remove("charge_limit");
        }
        if let Some(night) = table.get_mut("night_light").and_then(Item::as_table_mut) {
            if config.quick_settings.night_light.start.is_none() {
                night.remove("start");
            }
            if config.quick_settings.night_light.end.is_none() {
                night.remove("end");
            }
        }
    }
    merge_known_table(document.as_item_mut(), "quick_settings", quick.as_table());

    notifications::apply(document, &config.notifications);
    // Harness/config serializers may normalize the projection to an inline table.
    if let Some(table) = document
        .get("compositor")
        .and_then(Item::as_inline_table)
        .cloned()
    {
        document["compositor"] = Item::Table(table.into_table());
    }
    set_table_value(
        document,
        "shell",
        "wallpaper",
        Item::Table({
            let mut wallpaper = document
                .get("shell")
                .and_then(|s| s.get("wallpaper"))
                .cloned()
                .unwrap_or_else(|| Item::Table(Table::new()));
            wallpaper["fit"] = value(config.shell.wallpaper.fit.as_str());
            if let Some(image) = &config.shell.wallpaper.image {
                wallpaper["image"] = value(image.clone());
            } else {
                wallpaper
                    .as_table_like_mut()
                    .expect("validated wallpaper table")
                    .remove("image");
            }
            wallpaper.as_table().cloned().unwrap_or_else(|| {
                let mut table = Table::new();
                for (key, value) in wallpaper
                    .as_table_like()
                    .expect("validated wallpaper table")
                    .iter()
                {
                    table.insert(key, value.clone());
                }
                table
            })
        }),
    );
    if let Some(inline) = document
        .get("appearance")
        .and_then(Item::as_inline_table)
        .cloned()
    {
        document["appearance"] = Item::Table(inline.into_table());
    } else if !document.get("appearance").is_some_and(Item::is_table) {
        document["appearance"] = Item::Table(Table::new());
    }
    if !document["appearance"]
        .get("theme")
        .is_some_and(Item::is_table_like)
    {
        document["appearance"]["theme"] = Item::Table(Table::new());
    }
    document["appearance"]["theme"]["theme"] = value(config.appearance.theme_name.clone());
    document["appearance"]["theme"]["mode"] = value(config.appearance.theme.as_str());
    document["appearance"]["theme"]["contrast"] = value(config.appearance.contrast.as_str());
    document["appearance"]["theme"]["chroma"] = value(config.appearance.chroma.as_str());
    set_table_value(document, "portal", "enabled", value(config.portal.enabled));
    set_table_value(
        document,
        "portal",
        "color_scheme",
        value(i64::from(config.portal.color_scheme)),
    );
    set_table_value(
        document,
        "portal",
        "accent_color",
        value(config.portal.accent_color.clone()),
    );
    set_table_value(
        document,
        "portal",
        "high_contrast",
        value(config.portal.high_contrast),
    );
    set_table_value(
        document,
        "portal",
        "reduced_motion",
        value(config.portal.reduced_motion),
    );
    set_table_value(
        document,
        "compositor",
        "scale",
        value(config.compositor.scale),
    );
    set_table_value(
        document,
        "compositor",
        "master_percent",
        value(i64::from(config.compositor.master_percent)),
    );
    set_table_value(
        document,
        "compositor",
        "modkey",
        value(config.compositor.modkey.clone()),
    );
    set_table_value(
        document,
        "compositor",
        "focus_cycle_order",
        value(config.compositor.focus_cycle_order.as_str()),
    );
    if let Some(environment_file) = &config.compositor.environment_file {
        set_table_value(
            document,
            "compositor",
            "environment_file",
            value(environment_file.clone()),
        );
    } else if let Some(compositor) = document["compositor"].as_table_mut() {
        compositor.remove("environment_file");
    }
    set_table_value(
        document,
        "compositor",
        "input",
        Item::Table({
            let mut input = Table::new();
            input["tap_to_click"] = value(config.compositor.input.tap_to_click);
            input["natural_scroll"] = value(config.compositor.input.natural_scroll);
            input["focus_follows_mouse"] = value(config.compositor.input.focus_follows_mouse);
            input
        }),
    );
    let mut devices = ArrayOfTables::new();
    for device in &config.compositor.device {
        let mut table = Table::new();
        table["name"] = value(device.name.clone());
        if let Some(enabled) = device.enabled {
            table["enabled"] = value(enabled);
        }
        if let Some(tap_to_click) = device.tap_to_click {
            table["tap_to_click"] = value(tap_to_click);
        }
        if let Some(tap_and_drag) = device.tap_and_drag {
            table["tap_and_drag"] = value(tap_and_drag);
        }
        if let Some(drag_lock) = device.drag_lock {
            table["drag_lock"] = value(drag_lock);
        }
        if let Some(natural_scroll) = device.natural_scroll {
            table["natural_scroll"] = value(natural_scroll);
        }
        if let Some(disable_while_typing) = device.disable_while_typing {
            table["disable_while_typing"] = value(disable_while_typing);
        }
        if let Some(middle_emulation) = device.middle_emulation {
            table["middle_emulation"] = value(middle_emulation);
        }
        if let Some(left_handed) = device.left_handed {
            table["left_handed"] = value(left_handed);
        }
        if let Some(sensitivity) = device.sensitivity {
            table["sensitivity"] = value(sensitivity);
        }
        if let Some(accel_profile) = device.accel_profile {
            table["accel_profile"] = value(accel_profile.as_str());
        }
        if let Some(scroll_method) = device.scroll_method {
            table["scroll_method"] = value(scroll_method.as_str());
        }
        if let Some(scroll_button) = device.scroll_button {
            table["scroll_button"] = value(i64::from(scroll_button));
        }
        if let Some(click_method) = device.click_method {
            table["click_method"] = value(click_method.as_str());
        }
        devices.push(table);
    }
    document["compositor"]["device"] = Item::ArrayOfTables(devices);
    let appearance = toml::to_string(&config.compositor.appearance)
        .expect("validated appearance is serializable")
        .parse::<DocumentMut>()
        .expect("serialized appearance is TOML");
    merge_known_table(
        &mut document["compositor"],
        "appearance",
        appearance.as_table(),
    );
    let mut workspace = Table::new();
    workspace["skip_empty"] = value(config.compositor.workspace.skip_empty);
    workspace["predefined"] = value(i64::from(config.compositor.workspace.predefined));
    merge_known_table(&mut document["compositor"], "workspace", &workspace);
    let window_rules = toml::to_string(&config.compositor.window_rules)
        .expect("validated window rules are serializable")
        .parse::<DocumentMut>()
        .expect("serialized window rules are TOML");
    merge_known_table(
        &mut document["compositor"],
        "window_rules",
        window_rules.as_table(),
    );
    gesture::write(&mut document["compositor"], &config.compositor.gesture);
    let mut bindings = ArrayOfTables::new();
    for binding in &config.compositor.bind {
        let mut table = Table::new();
        table["keys"] = value(binding.keys.clone());
        table["dispatch"] = value(binding.dispatch.clone());
        let mut args = Array::new();
        for arg in &binding.args {
            args.push(arg.clone());
        }
        table["args"] = Item::Value(args.into());
        bindings.push(table);
    }
    document["compositor"]["bind"] = Item::ArrayOfTables(bindings);
    set_table_value(
        document,
        "session",
        "backend",
        value(format!("{:?}", config.session.backend).to_lowercase()),
    );
    set_table_value(
        document,
        "session",
        "compositor_binary",
        value(config.session.compositor_binary.clone()),
    );
    set_table_value(
        document,
        "session",
        "shell_binary",
        value(config.session.shell_binary.clone()),
    );
    set_table_value(
        document,
        "session",
        "restart_on_failure",
        value(config.session.restart_on_failure),
    );
    let mut startup = ArrayOfTables::new();
    for entry in &config.session.startup {
        let mut table = Table::new();
        let mut command = Array::new();
        for part in &entry.command {
            command.push(part.as_str());
        }
        table["command"] = Item::Value(command.into());
        startup.push(table);
    }
    document["session"]["startup"] = Item::ArrayOfTables(startup);
    set_table_value(
        document,
        "shell",
        "start_bar",
        value(config.shell.start_bar),
    );
    set_table_value(
        document,
        "shell",
        "start_overview_service",
        value(config.shell.start_overview_service),
    );
}

fn set_table_value(document: &mut DocumentMut, section: &str, key: &str, item: Item) {
    if !document.get(section).is_some_and(Item::is_table) {
        document[section] = Item::Table(Table::new());
    }
    document[section]
        .as_table_mut()
        .expect("section was initialized as a table")[key] = item;
}

// Merge leaves so unknown nested source data survives settings writes.
fn merge_known_table(parent: &mut Item, key: &str, source: &Table) {
    if parent
        .as_table_like()
        .and_then(|table| table.get(key))
        .is_none()
    {
        parent[key] = Item::Table(Table::new());
    }
    merge_table_leaves(&mut parent[key], source);
}

fn merge_table_leaves(destination: &mut Item, source: &Table) {
    let target = destination
        .as_table_like_mut()
        .expect("parsed settings table");
    for (key, value) in source.iter() {
        if let Some(table) = value.as_table() {
            if target.get(key).is_none() {
                target.insert(key, Item::Table(Table::new()));
            }
            merge_table_leaves(target.get_mut(key).expect("table initialized"), table);
        } else if let Some(tables) = value.as_array_of_tables() {
            if target.get(key).and_then(Item::as_array_of_tables).is_none() {
                target.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
            }
            merge_array_of_tables(
                target.get_mut(key).expect("array of tables initialized"),
                tables,
            );
        } else if value.as_array().is_some_and(Array::is_empty)
            && target
                .get(key)
                .is_some_and(|item| item.as_array_of_tables().is_some())
        {
            merge_array_of_tables(
                target.get_mut(key).expect("existing array of tables"),
                &ArrayOfTables::new(),
            );
        } else {
            if let Some(existing) = target.get_mut(key) {
                replace_value_preserving_decor(existing, value);
            } else {
                target.insert(key, value.clone());
            }
        }
    }
}

fn replace_value_preserving_decor(destination: &mut Item, source: &Item) {
    if let (Some(existing), Some(source)) = (destination.as_value_mut(), source.as_value()) {
        let decor = existing.decor().clone();
        let mut value = source.clone();
        *value.decor_mut() = decor;
        *existing = value;
    } else {
        *destination = source.clone();
    }
}

fn merge_array_of_tables(destination: &mut Item, source: &ArrayOfTables) {
    let target = destination
        .as_array_of_tables_mut()
        .expect("window-rule entries are an array of tables");
    while target.len() > source.len() {
        target.remove(target.len() - 1);
    }
    for (index, source_table) in source.iter().enumerate() {
        if let Some(destination_table) = target.get_mut(index) {
            let mut entry = Item::Table(destination_table.clone());
            merge_table_leaves(&mut entry, source_table);
            *destination_table = entry.as_table().expect("merged entry is a table").clone();
        } else {
            target.push(source_table.clone());
        }
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .as_nanos();
    let temporary = path.with_extension(format!("tmp.{}.{}", std::process::id(), stamp));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| ConfigError::Write {
                path: temporary.clone(),
                source,
            })?;
        file.write_all(contents)
            .map_err(|source| ConfigError::Write {
                path: temporary.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| ConfigError::Write {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os("KNAVE_CONFIG") {
        return Ok(path.into());
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("knave/config.toml"));
    }
    let home =
        std::env::var_os("HOME").ok_or_else(|| ConfigError::Path("HOME is not set".into()))?;
    Ok(PathBuf::from(home).join(".config/knave/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_policy_defaults_validate_and_preserve_unknown_options() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "[compositor]\nunknown_option = 7\n[compositor.input]\nunknown_input = true\n",
        )
        .unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(
            document.config().compositor.focus_cycle_order,
            FocusCycleOrder::Layout
        );
        assert!(document.config().compositor.input.focus_follows_mouse);
        let mut config = document.config().clone();
        config.compositor.focus_cycle_order = FocusCycleOrder::Mru;
        config.compositor.input.focus_follows_mouse = false;
        document.write(config).unwrap();
        let reloaded = ConfigDocument::load(&path).unwrap();
        assert_eq!(
            reloaded.config().compositor.focus_cycle_order,
            FocusCycleOrder::Mru
        );
        assert!(!reloaded.config().compositor.input.focus_follows_mouse);
        assert!(document.source().contains("unknown_option = 7"));
        assert!(document.source().contains("focus_follows_mouse = false"));
    }

    #[test]
    fn master_percentage_defaults_validates_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[compositor]\nunknown_layout_option = 42\n").unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().compositor.master_percent, 50);
        let mut config = document.config().clone();
        for invalid in [0, 9, 91, 255] {
            config.compositor.master_percent = invalid;
            assert!(document.write(config.clone()).is_err());
        }
        for valid in [10, 60, 90] {
            config.compositor.master_percent = valid;
            document.write(config.clone()).unwrap();
            assert_eq!(
                ConfigDocument::load(&path)
                    .unwrap()
                    .config()
                    .compositor
                    .master_percent,
                valid
            );
            assert!(document.source().contains("unknown_layout_option = 42"));
        }
    }

    #[test]
    fn compositor_scale_defaults_validates_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[compositor]\ncustom_output_option = 42\n").unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().compositor.scale, 1.0);
        let mut config = document.config().clone();
        for invalid in [f64::NAN, f64::INFINITY, 0.99, 4.01] {
            config.compositor.scale = invalid;
            assert!(document.write(config.clone()).is_err());
        }
        for valid in [1.0, 1.25, 1.5, 4.0] {
            config.compositor.scale = valid;
            document.write(config.clone()).unwrap();
            assert_eq!(
                ConfigDocument::load(&path)
                    .unwrap()
                    .config()
                    .compositor
                    .scale,
                valid
            );
            assert!(document.source().contains("custom_output_option = 42"));
        }
    }

    #[test]
    fn portal_defaults_validate_and_preserve_unknown_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[portal]\ncustom_option = 42\n").unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().portal, PortalConfig::default());
        let mut config = document.config().clone();
        config.portal.color_scheme = 1;
        document.write(config.clone()).unwrap();
        assert!(document.source().contains("custom_option = 42"));
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .portal
                .color_scheme,
            1
        );
        for invalid in ["NaN,0,1", "1.1,0,1", "0,1"] {
            config.portal.accent_color = invalid.into();
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn theme_defaults_to_system_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[appearance]\ncustom_option = 42\n").unwrap();
        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().appearance.theme, Theme::System);
        assert_eq!(
            document.config().appearance.contrast,
            ContrastPreference::Standard
        );
        assert_eq!(
            document.config().appearance.chroma,
            ChromaPreference::FullColor
        );

        let mut config = document.config().clone();
        config.appearance.theme = Theme::Dark;
        document.write(config.clone()).unwrap();
        assert!(document.source().contains("custom_option = 42"));
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .appearance
                .theme,
            Theme::Dark
        );

        config.appearance.theme = Theme::Light;
        document.write(config).unwrap();
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .appearance
                .theme,
            Theme::Light
        );
    }

    #[test]
    fn named_theme_round_trips_and_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[appearance.theme]\ntheme = 'dracula'\nmode = 'light'\ncontrast = 'high'\nchroma = 'monochrome'\nextra = 42\n",
        )
        .unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        assert_eq!(doc.config().appearance.theme_name, "dracula");
        assert_eq!(doc.config().appearance.theme, Theme::Light);
        assert_eq!(doc.config().appearance.contrast, ContrastPreference::High);
        assert_eq!(doc.config().appearance.chroma, ChromaPreference::Monochrome);
        doc.write(doc.config().clone()).unwrap();
        assert!(doc.source().contains("extra = 42"));
        assert_eq!(ConfigDocument::load(&path).unwrap().config(), doc.config());
        let mut invalid = doc.config().clone();
        invalid.appearance.theme_name = "../outside".into();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn inline_named_theme_keeps_unknown_fields_when_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[appearance]\ntheme = { theme = 'test', mode = 'dark', custom = 7 }\n",
        )
        .unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        let mut config = doc.config().clone();
        config.appearance.theme = Theme::Light;
        doc.write(config).unwrap();
        assert!(doc.source().contains("custom = 7"));
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .appearance
                .theme,
            Theme::Light
        );
    }

    #[test]
    fn inline_appearance_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "appearance = { theme = { theme = 'knave', mode = 'dark' }, custom = 7 }\n",
        )
        .unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        doc.write(doc.config().clone()).unwrap();
        assert!(doc.source().contains("custom = 7"));
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .appearance
                .theme,
            Theme::Dark
        );
    }

    #[test]
    fn theme_color_scheme_matches_portal_values() {
        assert_eq!(Theme::System.color_scheme(), 0);
        assert_eq!(Theme::Dark.color_scheme(), 1);
        assert_eq!(Theme::Light.color_scheme(), 2);
    }

    #[test]
    fn unknown_theme_value_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[appearance]\ntheme = \"midnight\"\n").unwrap();
        assert!(ConfigDocument::load(&path).is_err());
    }

    #[test]
    fn legacy_scalar_theme_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[appearance]\ntheme = \"dark\"\n").unwrap();
        assert!(ConfigDocument::load(&path).is_err());
    }

    #[test]
    fn wallpaper_round_trip_clear_and_validation_preserve_unknown_options() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[shell.wallpaper]\nfuture_option = 42\n").unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        let mut config = doc.config().clone();
        assert!(config.shell.wallpaper.image.is_none());
        config.shell.wallpaper.image = Some("relative.png".into());
        assert!(doc.write(config.clone()).is_err());
        config.shell.wallpaper.image = Some("/tmp/wallpaper.png".into());
        config.shell.wallpaper.fit = WallpaperFit::Contain;
        doc.write(config.clone()).unwrap();
        assert_eq!(
            ConfigDocument::load(&path)
                .unwrap()
                .config()
                .shell
                .wallpaper,
            config.shell.wallpaper
        );
        config.shell.wallpaper.image = None;
        doc.write(config).unwrap();
        assert!(doc.source().contains("future_option = 42"));
        assert!(
            ConfigDocument::load(path)
                .unwrap()
                .config()
                .shell
                .wallpaper
                .image
                .is_none()
        );
    }

    #[test]
    fn workspace_navigation_defaults_and_round_trip_preserve_source() {
        assert_eq!(Config::default().compositor.workspace.predefined, 1);
        assert!(!Config::default().compositor.workspace.skip_empty);
        for (name, source) in [
            (
                "table",
                "[compositor.workspace]\nskip_empty = true\nfuture_option = 42\n",
            ),
            (
                "inline",
                "compositor = { workspace = {skip_empty = true, future_option = 42} }\n",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(format!("{name}.toml"));
            fs::write(&path, source).unwrap();
            let mut document = ConfigDocument::load(&path).unwrap();
            assert!(document.config().compositor.workspace.skip_empty);
            let mut config = document.config().clone();
            config.compositor.workspace.skip_empty = false;
            document.write(config).unwrap();
            assert!(document.source().contains("future_option = 42"));
            assert!(
                !ConfigDocument::load(path)
                    .unwrap()
                    .config()
                    .compositor
                    .workspace
                    .skip_empty
            );
        }
        assert!(toml::from_str::<Config>("[compositor.workspace]\nskip_empty = 'true'\n").is_err());
        let mut config = Config::default();
        config.compositor.workspace.predefined = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn defaults_validate() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn startup_command_entries_round_trip_and_reject_empty_executables() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "[[session.startup]]\ncommand = [\"kitty\"]\n\
             [[session.startup]]\ncommand = [\"xterm\", \"-fa\", \"monospace\"]\n",
        )
        .unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().session.startup.len(), 2);
        document.write(document.config().clone()).unwrap();
        assert_eq!(
            ConfigDocument::load(path).unwrap().config().session.startup,
            vec![
                StartupCommand {
                    command: vec!["kitty".into()],
                },
                StartupCommand {
                    command: vec!["xterm".into(), "-fa".into(), "monospace".into()],
                },
            ]
        );

        let mut config = Config::default();
        config
            .session
            .startup
            .push(StartupCommand { command: vec![] });
        assert!(config.validate().is_err());
    }

    #[test]
    fn overview_service_can_be_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[shell]\nstart_overview_service = false\n").unwrap();

        let document = ConfigDocument::load(&path).unwrap();
        assert!(!document.config().shell.start_overview_service);
    }

    #[test]
    fn unknown_toml_is_preserved_when_known_values_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "custom_value = \"keep\"\n[session]\nbackend = \"auto\"\n",
        )
        .unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        let mut config = document.config().clone();
        config.session.restart_on_failure = false;
        config.compositor.bind.push(CompositorBinding {
            keys: "MOD+Q".into(),
            dispatch: "close".into(),
            args: vec!["--force".into()],
        });

        document.write(config).unwrap();

        let output = fs::read_to_string(path).unwrap();
        assert!(output.contains("custom_value = \"keep\""));
        assert!(output.contains("restart_on_failure = false"));
        assert!(output.contains("[compositor]"));
        assert!(output.contains("[[compositor.bind]]"));
        assert!(output.contains(r#"args = ["--force"]"#));
    }
    #[test]
    fn clearing_optional_environment_file_removes_known_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[compositor]\nenvironment_file = \"environment\"\n").unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        let mut config = document.config().clone();
        config.compositor.environment_file = None;
        document.write(config).unwrap();

        let output = fs::read_to_string(path).unwrap();
        assert!(!output.contains("environment_file"));
    }

    #[test]
    fn device_rules_round_trip_validate_and_preserve_unknown_options() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            r#"[compositor]
future_option = 7

[[compositor.device]]
name = "Example Touchpad"
sensitivity = 0.4

[[compositor.device]]
name = "*Receiver Mouse"
enabled = false
"#,
        )
        .unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(document.config().compositor.device.len(), 2);
        let touchpad = &document.config().compositor.device[0];
        assert_eq!(touchpad.name, "Example Touchpad");
        assert_eq!(touchpad.sensitivity, Some(0.4));
        assert_eq!(touchpad.enabled, None);

        let mut config = document.config().clone();
        for invalid in [f64::NAN, 1.5, -1.5] {
            config.compositor.device[0].sensitivity = Some(invalid);
            assert!(config.validate().is_err());
        }
        config.compositor.device[0].sensitivity = Some(-1.0);
        config.compositor.device[0].name = "   ".into();
        assert!(config.validate().is_err());

        config.compositor.device[0] = CompositorDeviceConfig {
            name: "AT Translated Set 2 keyboard".into(),
            enabled: Some(false),
            tap_to_click: None,
            tap_and_drag: None,
            drag_lock: None,
            natural_scroll: None,
            disable_while_typing: None,
            middle_emulation: None,
            left_handed: None,
            sensitivity: None,
            accel_profile: Some(DeviceAccelProfile::Flat),
            scroll_method: Some(DeviceScrollMethod::TwoFinger),
            scroll_button: Some(272),
            click_method: Some(DeviceClickMethod::Clickfinger),
        };
        document.write(config).unwrap();

        let reloaded = ConfigDocument::load(&path).unwrap();
        let device = &reloaded.config().compositor.device[0];
        assert_eq!(device.name, "AT Translated Set 2 keyboard");
        assert_eq!(device.enabled, Some(false));
        assert_eq!(device.accel_profile, Some(DeviceAccelProfile::Flat));
        assert_eq!(device.scroll_method, Some(DeviceScrollMethod::TwoFinger));
        assert_eq!(device.scroll_button, Some(272));
        assert_eq!(device.click_method, Some(DeviceClickMethod::Clickfinger));
        assert_eq!(reloaded.config().compositor.device.len(), 2);
        // Unknown keys outside a rewritten device rule survive.
        assert!(reloaded.source().contains("future_option = 7"));
    }

    #[test]
    fn window_rules_parse_validate_and_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            r#"
[compositor.window_rules]
apply = "on_open"
[compositor.workspace]
predefined = 3
[[compositor.window_rules.application]]
app_id = "firefox"
workspace = 3
vendor_option = 7 # Vendor metadata is not part of the typed schema and must survive writes.
[[compositor.window_rules.rule]]
name = "Firefox picture-in-picture"
[compositor.window_rules.rule.match]
app_id = "firefox"
title_regex = '(?i)picture[\s-]*in[\s-]*picture'
[compositor.window_rules.rule.set]
placement = "floating"
center = true
stay_on_top = true
"#,
        )
        .unwrap();

        let mut document = ConfigDocument::load(&path).unwrap();
        assert_eq!(
            document.config().compositor.window_rules.application[0].app_id,
            "firefox"
        );
        assert_eq!(
            document.config().compositor.window_rules.application[0]
                .set
                .workspace,
            Some(3)
        );
        assert_eq!(
            document.config().compositor.window_rules.rule[0]
                .matcher
                .title_regex
                .as_deref(),
            Some(r"(?i)picture[\s-]*in[\s-]*picture")
        );
        document.write(document.config().clone()).unwrap();
        let reloaded = ConfigDocument::load(&path).unwrap();
        assert_eq!(
            reloaded.config().compositor.window_rules,
            document.config().compositor.window_rules
        );
        assert!(
            reloaded
                .source()
                .contains("Vendor metadata is not part of the typed schema")
        );
        assert!(reloaded.source().contains("vendor_option = 7"));

        let mut invalid = reloaded.config().clone();
        invalid.compositor.window_rules.rule[0].matcher.title_regex = Some("(".into());
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("title_regex is invalid")
        );
        invalid.compositor.window_rules.rule[0].matcher.title_regex = Some("x".repeat(1025));
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("exceeds 1024 bytes")
        );
        invalid.compositor.window_rules.rule[0].matcher.title_regex = Some("valid".into());
        invalid.compositor.window_rules.application[0].set.workspace = Some(4);
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("workspace must be between 1 and 3")
        );
    }

    #[test]
    fn absent_device_rules_write_no_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut document = ConfigDocument::load(&path).unwrap();
        document.write(Config::default()).unwrap();
        assert!(!document.source().contains("device"));
    }
}

#[cfg(test)]
mod appearance_tests;
