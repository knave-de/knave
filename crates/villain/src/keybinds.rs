//! Configurable key combinations resolve to central dispatcher actions.

use std::collections::HashSet;

use smithay::{
    backend::input::{InputBackend, KeyState, KeyboardKeyEvent},
    input::keyboard::{FilterResult, KeyboardTarget, Keycode, Keysym, keysyms, xkb},
    utils::SERIAL_COUNTER,
};

use crate::{
    config::BindSpec,
    dispatch::{Dispatch, FocusCycleStep, FocusDirection},
    state::Villain,
};

const MAX_GLOBAL_SHORTCUTS: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
enum KeyboardAction {
    Dispatch(Dispatch),
    Vt(i32),
    FinishInteraction(bool),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BindingKey {
    Keysym(u32),
    Mouse(u32),
    Modifier(Modkey),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct KeyCombination {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
    key: BindingKey,
}

#[derive(Clone, Debug)]
struct Keybind {
    keys: KeyCombination,
    dispatch: Dispatch,
}

#[derive(Clone, Debug)]
pub struct KeybindRegistry {
    bindings: Vec<Keybind>,
}

impl KeybindRegistry {
    pub fn defaults(modkey: &str) -> Result<Self, String> {
        Self::from_specs(modkey, &Self::default_specs())
    }

    pub(crate) fn empty() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    pub(crate) fn conflicts_with(
        &self,
        other: &Self,
        trigger: &str,
        modkey: &str,
    ) -> Result<bool, String> {
        let modkey = parse_modkey(modkey)?;
        let key = parse_portal_trigger(trigger, modkey)?;
        let reserved_session_chord = key == parse_portal_trigger("CTRL+ALT+BACKSPACE", modkey)?
            || (1..=12).any(|number| {
                parse_portal_trigger(&format!("CTRL+ALT+F{number}"), modkey)
                    .is_ok_and(|reserved| key == reserved)
            });
        Ok(reserved_session_chord
            || self
                .bindings
                .iter()
                .chain(&other.bindings)
                .any(|binding| binding.keys == key))
    }

    pub(crate) fn register_global(
        &mut self,
        app_id: String,
        session_handle: String,
        shortcut_id: String,
        trigger: &str,
        modkey: &str,
    ) -> Result<String, String> {
        if app_id.is_empty() || shortcut_id.is_empty() {
            return Err("app ID and shortcut ID must not be empty".into());
        }
        if self.bindings.iter().any(|binding| {
            matches!(&binding.dispatch, Dispatch::GlobalShortcutActivated { app_id: owner, session_handle: session, shortcut_id: id } if owner == &app_id && session == &session_handle && id == &shortcut_id)
        }) {
            return Err("shortcut ID is already registered for this app".into());
        }
        let keys = parse_portal_trigger(trigger, parse_modkey(modkey)?)?;
        if self.bindings.iter().any(|binding| binding.keys == keys) {
            return Err("shortcut trigger is already registered".into());
        }
        if self.bindings.len() >= MAX_GLOBAL_SHORTCUTS {
            return Err("global shortcut registration limit reached".into());
        }
        let trigger = canonical_portal_trigger(trigger);
        self.bindings.push(Keybind {
            keys,
            dispatch: Dispatch::GlobalShortcutActivated {
                app_id,
                session_handle,
                shortcut_id: shortcut_id.clone(),
            },
        });
        Ok(trigger)
    }

    pub(crate) fn replace_global(
        &mut self,
        configured: &Self,
        app_id: &str,
        session_handle: &str,
        shortcut_id: &str,
        trigger: &str,
        modkey: &str,
    ) -> Result<String, String> {
        let index = self.bindings.iter().position(|binding| {
            matches!(&binding.dispatch, Dispatch::GlobalShortcutActivated { app_id: owner, session_handle: session, shortcut_id: id } if owner == app_id && session == session_handle && id == shortcut_id)
        }).ok_or_else(|| "global shortcut is not registered".to_string())?;
        let modkey = parse_modkey(modkey)?;
        let key = parse_portal_trigger(trigger, modkey)?;
        let reserved = key == parse_portal_trigger("CTRL+ALT+BACKSPACE", modkey)?
            || (1..=12).any(|number| {
                parse_portal_trigger(&format!("CTRL+ALT+F{number}"), modkey)
                    .is_ok_and(|reserved| key == reserved)
            });
        if reserved
            || configured
                .bindings
                .iter()
                .any(|binding| binding.keys == key)
            || self
                .bindings
                .iter()
                .enumerate()
                .any(|(other_index, binding)| other_index != index && binding.keys == key)
        {
            return Err("global shortcut trigger conflicts with an existing binding".into());
        }
        self.bindings[index].keys = key;
        Ok(canonical_portal_trigger(trigger))
    }

    pub(crate) fn unregister_global(
        &mut self,
        app_id: &str,
        session_handle: &str,
        shortcut_id: &str,
    ) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|binding| {
            !matches!(&binding.dispatch, Dispatch::GlobalShortcutActivated { app_id: owner, session_handle: session, shortcut_id: id } if owner == app_id && session == session_handle && id == shortcut_id)
        });
        self.bindings.len() != before
    }

    /// Apply explicit bindings on top of the default set. An explicit binding
    /// with the same effective key combination replaces only the matching
    /// default, mirroring how later `[[compositor.device]]` rules layer over
    /// earlier ones; every other default stays active. Bindings that match no
    /// default are additions. Overriding the same combination twice fails.
    pub(crate) fn with_overrides(modkey: &str, overrides: &[BindSpec]) -> Result<Self, String> {
        let mut base = Self::from_specs(modkey, &Self::default_specs())?;
        let modkey = parse_modkey(modkey)?;
        let mut overridden = HashSet::new();
        for spec in overrides {
            let keys = parse_keys(&spec.keys, modkey)
                .map_err(|error| format!("binding {:?}: {error}", spec.keys))?;
            if !overridden.insert(keys) {
                return Err(format!("duplicate keybinding {:?}", spec.keys));
            }
            let dispatch = parse_dispatch(spec)
                .map_err(|error| format!("binding {:?}: {error}", spec.keys))?;
            if let Some(binding) = base
                .bindings
                .iter_mut()
                .find(|binding| binding.keys == keys)
            {
                binding.dispatch = dispatch;
            } else {
                base.bindings.push(Keybind { keys, dispatch });
            }
        }
        Ok(base)
    }

    fn default_specs() -> Vec<BindSpec> {
        let mut specs = vec![
            spec("MOD", "toggle-overview", &[]),
            spec("MOD+BUTTON_LEFT", "move-window", &["pointer"]),
            spec("MOD+BUTTON_RIGHT", "resize-window", &["pointer"]),
            spec("MOD+RETURN", "exec", &["kitty"]),
            spec("MOD+Q", "close", &[]),
            spec("MOD+V", "toggle-floating", &[]),
            spec("MOD+F", "toggle-maximize", &[]),
            spec("MOD+CTRL+LEFT", "resize-master", &["-5"]),
            spec("MOD+CTRL+RIGHT", "resize-master", &["5"]),
            spec("MOD+CTRL+R", "reset-master", &[]),
            spec("PRINT", "screenshot", &["region", "save"]),
            spec("SHIFT+PRINT", "screenshot", &["output", "save"]),
            spec("MOD+PRINT", "screenshot", &["window", "save"]),
            spec("CTRL+PRINT", "screenshot", &["region", "clipboard"]),
            spec("CTRL+SHIFT+PRINT", "screenshot", &["output", "clipboard"]),
            spec("MOD+SHIFT+PRINT", "screenshot", &["window", "clipboard"]),
            spec("MOD+M", "minimize", &[]),
            spec("MOD+SHIFT+M", "restore-minimized", &[]),
            spec("MOD+SHIFT+H", "move-left", &[]),
            spec("MOD+SHIFT+J", "move-down", &[]),
            spec("MOD+SHIFT+K", "move-up", &[]),
            spec("MOD+SHIFT+L", "move-right", &[]),
            spec("MOD+H", "focus-left", &[]),
            spec("MOD+L", "focus-right", &[]),
            spec("MOD+K", "focus-up", &[]),
            spec("MOD+J", "focus-down", &[]),
            spec("MOD+TAB", "focus-next", &[]),
            spec("MOD+SHIFT+TAB", "focus-previous", &[]),
            spec("MOD+LEFT", "previous-workspace", &[]),
            spec("MOD+GRAVE", "last-workspace", &[]),
            spec("MOD+RIGHT", "next-workspace", &[]),
        ];
        for workspace in 1..=10 {
            let key = workspace % 10;
            specs.push(BindSpec {
                keys: format!("MOD+{key}"),
                dispatch: "workspace".into(),
                args: vec![workspace.to_string()],
            });
            specs.push(BindSpec {
                keys: format!("MOD+SHIFT+{key}"),
                dispatch: "move-to-workspace".into(),
                args: vec![workspace.to_string()],
            });
        }
        specs
    }

    pub(crate) fn from_specs(modkey: &str, specs: &[BindSpec]) -> Result<Self, String> {
        let modkey = parse_modkey(modkey)?;
        let mut seen = HashSet::new();
        let mut bindings = Vec::with_capacity(specs.len());
        for spec in specs {
            let keys = parse_keys(&spec.keys, modkey)
                .map_err(|error| format!("binding {:?}: {error}", spec.keys))?;
            if !seen.insert(keys) {
                return Err(format!("duplicate keybinding {:?}", spec.keys));
            }
            let dispatch = parse_dispatch(spec)
                .map_err(|error| format!("binding {:?}: {error}", spec.keys))?;
            bindings.push(Keybind { keys, dispatch });
        }
        Ok(Self { bindings })
    }

    pub(crate) fn find(
        &self,
        raw_syms: &[Keysym],
        ctrl: bool,
        alt: bool,
        shift: bool,
        super_key: bool,
    ) -> Option<Dispatch> {
        self.find_binding(raw_syms, ctrl, alt, shift, super_key)
            .map(|binding| binding.dispatch.clone())
    }

    pub(crate) fn find_modifier_only(
        &self,
        raw_syms: &[Keysym],
        ctrl: bool,
        alt: bool,
        shift: bool,
        super_key: bool,
    ) -> Option<Dispatch> {
        self.find_binding(raw_syms, ctrl, alt, shift, super_key)
            .filter(|binding| matches!(binding.keys.key, BindingKey::Modifier(_)))
            .map(|binding| binding.dispatch.clone())
    }

    pub(crate) fn find_mouse(
        &self,
        button: u32,
        mods: smithay::input::keyboard::ModifiersState,
    ) -> Option<Dispatch> {
        self.bindings
            .iter()
            .find(|binding| {
                binding.keys.key == BindingKey::Mouse(button)
                    && binding.keys.ctrl == mods.ctrl
                    && binding.keys.alt == mods.alt
                    && binding.keys.shift == mods.shift
                    && binding.keys.super_key == mods.logo
            })
            .map(|binding| binding.dispatch.clone())
    }

    fn find_binding(
        &self,
        raw_syms: &[Keysym],
        ctrl: bool,
        alt: bool,
        shift: bool,
        super_key: bool,
    ) -> Option<&Keybind> {
        self.bindings.iter().find(|binding| {
            binding.keys.ctrl == ctrl
                && binding.keys.alt == alt
                && binding.keys.shift == shift
                && binding.keys.super_key == super_key
                && binding.keys.key.matches(raw_syms)
        })
    }
}

impl BindingKey {
    fn matches(self, raw_syms: &[Keysym]) -> bool {
        raw_syms.iter().any(|symbol| match self {
            Self::Mouse(_) => false,
            Self::Keysym(expected) => {
                symbol.raw() == expected
                    || (expected == keysyms::KEY_Tab && symbol.raw() == keysyms::KEY_ISO_Left_Tab)
            }
            Self::Modifier(Modkey::Alt) => {
                matches!(symbol.raw(), keysyms::KEY_Alt_L | keysyms::KEY_Alt_R)
            }
            Self::Modifier(Modkey::Super) => {
                matches!(symbol.raw(), keysyms::KEY_Super_L | keysyms::KEY_Super_R)
            }
        })
    }
}

fn spec(keys: &str, dispatch: &str, args: &[&str]) -> BindSpec {
    BindSpec {
        keys: keys.into(),
        dispatch: dispatch.into(),
        args: args.iter().map(|argument| (*argument).into()).collect(),
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Modkey {
    Alt,
    Super,
}

fn parse_modkey(value: &str) -> Result<Modkey, String> {
    match value.to_ascii_lowercase().as_str() {
        "alt" => Ok(Modkey::Alt),
        "super" | "logo" => Ok(Modkey::Super),
        _ => Err(format!("modkey must be Alt or Super, got {value:?}")),
    }
}

fn parse_keys(value: &str, modkey: Modkey) -> Result<KeyCombination, String> {
    let tokens: Vec<_> = value
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect();
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_key = false;
    let mut mod_token = false;
    let mut key = None;
    for token in &tokens {
        match token.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => ctrl = true,
            "ALT" => alt = true,
            "SHIFT" => shift = true,
            "SUPER" | "LOGO" => super_key = true,
            "MOD" => match modkey {
                Modkey::Alt => {
                    alt = true;
                    mod_token = true;
                }
                Modkey::Super => {
                    super_key = true;
                    mod_token = true;
                }
            },
            _ if key.is_none() => key = Some(token),
            _ => return Err("a combination must contain exactly one non-modifier key".into()),
        }
    }
    let key = match key {
        Some(key) if key.eq_ignore_ascii_case("BUTTON_LEFT") => BindingKey::Mouse(0x110),
        Some(key) if key.eq_ignore_ascii_case("BUTTON_RIGHT") => BindingKey::Mouse(0x111),
        Some(key) if key.eq_ignore_ascii_case("BUTTON_MIDDLE") => BindingKey::Mouse(0x112),
        Some(key) => {
            let canonical = match key.to_ascii_uppercase().as_str() {
                "ENTER" => "Return",
                "ESC" => "Escape",
                "SPACE" => "space",
                "TILDE" | "~" => "grave",
                _ => key,
            };
            let sym = xkb::keysym_from_name(canonical, xkb::KEYSYM_CASE_INSENSITIVE).raw();
            if sym == keysyms::KEY_NoSymbol {
                return Err(format!("unknown key name {key:?}"));
            }
            BindingKey::Keysym(sym)
        }
        None if mod_token && tokens.len() == 1 => BindingKey::Modifier(modkey),
        None => return Err("combination has no key".into()),
    };
    Ok(KeyCombination {
        ctrl,
        alt,
        shift,
        super_key,
        key,
    })
}

fn parse_portal_trigger(value: &str, modkey: Modkey) -> Result<KeyCombination, String> {
    if value
        .split('+')
        .any(|token| token.trim().eq_ignore_ascii_case("MOD"))
    {
        return Err("global shortcut triggers must name modifiers explicitly".into());
    }
    let normalized = value
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            let upper = token.to_ascii_uppercase();
            match upper.as_str() {
                "CONTROL" => "CTRL".to_string(),
                "LOGO" => "SUPER".to_string(),
                _ => upper.strip_prefix("KEY_").unwrap_or(&upper).to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+");
    let combination = parse_keys(&normalized, modkey)?;
    if matches!(combination.key, BindingKey::Keysym(_)) {
        Ok(combination)
    } else {
        Err("global shortcuts require one keyboard key".into())
    }
}

fn canonical_portal_trigger(value: &str) -> String {
    value
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            let upper = token.to_ascii_uppercase();
            match upper.as_str() {
                "CONTROL" => "CTRL".to_string(),
                "LOGO" => "SUPER".to_string(),
                _ => upper.strip_prefix("KEY_").unwrap_or(&upper).to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn parse_dispatch(spec: &BindSpec) -> Result<Dispatch, String> {
    match spec.dispatch.as_str() {
        "move-window" | "resize-window" => {
            let target = match spec.args.as_slice() {
                [] => knave_desktop_api::WindowTarget::Focused,
                [value] if value == "pointer" => knave_desktop_api::WindowTarget::Pointer,
                _ => {
                    return Err(
                        "move-window/resize-window accepts only optional pointer target".into(),
                    );
                }
            };
            Ok(if spec.dispatch == "move-window" {
                Dispatch::MoveWindow(target)
            } else {
                Dispatch::ResizeWindow(target, None)
            })
        }
        "screenshot" if spec.args.len() == 2 => {
            let mode = match spec.args[0].as_str() {
                "region" => crate::screenshot::ScreenshotMode::Region,
                "output" => crate::screenshot::ScreenshotMode::Output,
                "window" => crate::screenshot::ScreenshotMode::Window,
                _ => return Err("screenshot mode must be region, output, or window".into()),
            };
            let clipboard = match spec.args[1].as_str() {
                "save" => false,
                "clipboard" => true,
                _ => return Err("screenshot destination must be save or clipboard".into()),
            };
            Ok(Dispatch::Screenshot { mode, clipboard })
        }
        "screenshot" => Err("screenshot requires a mode and destination".into()),
        "resize-master" if spec.args.len() == 1 => {
            let delta: i16 = spec.args[0]
                .parse()
                .map_err(|_| "resize-master requires an integer percentage change".to_string())?;
            if delta == 0 || !(-80..=80).contains(&delta) {
                return Err("resize-master change must be -80..-1 or 1..80".into());
            }
            Ok(Dispatch::ResizeMaster(delta))
        }
        "resize-master" => Err("resize-master requires exactly one argument".into()),
        "reset-master" if spec.args.is_empty() => Ok(Dispatch::ResetMaster),
        "reset-master" => Err("reset-master does not accept arguments".into()),
        "toggle-floating" if spec.args.is_empty() => Ok(Dispatch::ToggleFloatingFocused),
        "close" if spec.args.is_empty() => Ok(Dispatch::CloseFocused),
        "maximize" if spec.args.is_empty() => Ok(Dispatch::MaximizeFocused),
        "unmaximize" if spec.args.is_empty() => Ok(Dispatch::UnmaximizeFocused),
        "toggle-maximize" if spec.args.is_empty() => Ok(Dispatch::ToggleMaximizeFocused),
        "minimize" if spec.args.is_empty() => Ok(Dispatch::MinimizeFocused),
        "restore-minimized" if spec.args.is_empty() => Ok(Dispatch::RestoreLastMinimized),
        "previous-workspace" if spec.args.is_empty() => Ok(Dispatch::PreviousWorkspace),
        "last-workspace" if spec.args.is_empty() => Ok(Dispatch::LastWorkspace),
        "next-workspace" if spec.args.is_empty() => Ok(Dispatch::NextWorkspace),
        "focus-next" | "focus-previous" if spec.args.is_empty() => {
            Ok(Dispatch::FocusCycle(match spec.dispatch.as_str() {
                "focus-next" => FocusCycleStep::Next,
                _ => FocusCycleStep::Previous,
            }))
        }
        "focus-left" | "focus-right" | "focus-up" | "focus-down" if spec.args.is_empty() => {
            Ok(Dispatch::FocusDirection(match spec.dispatch.as_str() {
                "focus-left" => FocusDirection::Left,
                "focus-right" => FocusDirection::Right,
                "focus-up" => FocusDirection::Up,
                _ => FocusDirection::Down,
            }))
        }
        "move-left" | "move-right" | "move-up" | "move-down" if spec.args.is_empty() => {
            Ok(Dispatch::MoveDirection(match spec.dispatch.as_str() {
                "move-left" => FocusDirection::Left,
                "move-right" => FocusDirection::Right,
                "move-up" => FocusDirection::Up,
                _ => FocusDirection::Down,
            }))
        }
        "quit" if spec.args.is_empty() => Ok(Dispatch::Quit),
        "toggle-overview" if spec.args.is_empty() => Ok(Dispatch::ToggleOverview),
        // Keep user configurations that copied the former default from creating
        // transient Overview processes now that Knave Session owns the service.
        "exec" if spec.args == ["knave-shell", "overview"] => Ok(Dispatch::ToggleOverview),
        "exec" if !spec.args.is_empty() => Ok(Dispatch::Spawn(spec.args.clone())),
        "workspace" | "move-to-workspace" if spec.args.len() == 1 => {
            let workspace = spec.args[0]
                .parse()
                .map_err(|_| "workspace argument must be a number".to_string())?;
            if workspace == 0 {
                return Err("workspace argument must be at least 1".into());
            }
            Ok(if spec.dispatch == "move-to-workspace" {
                Dispatch::MoveFocusedToWorkspace(workspace)
            } else {
                Dispatch::FocusWorkspace(workspace)
            })
        }
        "toggle-floating" | "maximize" | "unmaximize" | "toggle-maximize" | "close"
        | "minimize" | "restore-minimized" | "previous-workspace" | "last-workspace"
        | "next-workspace" | "quit" | "toggle-overview" | "focus-next" | "focus-previous"
        | "focus-left" | "focus-right" | "focus-up" | "focus-down" | "move-left" | "move-right"
        | "move-up" | "move-down" => Err("dispatch does not accept arguments".into()),
        "exec" => Err("exec requires a program in args".into()),
        "workspace" | "move-to-workspace" => {
            Err(format!("{} requires exactly one argument", spec.dispatch))
        }
        dispatch => Err(format!("unknown dispatch {dispatch:?}")),
    }
}

pub fn handle_keyboard_event<B: InputBackend>(
    state: &mut Villain,
    event: impl KeyboardKeyEvent<B>,
) {
    handle_keyboard_key(state, event.key_code(), event.state(), event.time_msec());
}

/// True while the focused client holds an active keyboard-shortcuts
/// inhibitor: every key must reach it instead of the binding table, so
/// recorders, terminal emulators and remote clients never lose chords.
/// Inhibition applies to the surface keyboard focus, which is per seat.
pub(crate) fn shortcuts_inhibited(state: &Villain) -> bool {
    use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
    use smithay::wayland::seat::WaylandFocus;
    let Some(focus) = state.keyboard.current_focus() else {
        return false;
    };
    let Some(surface) = focus.wl_surface() else {
        return false;
    };
    state
        .seat
        .keyboard_shortcuts_inhibitor_for_surface(&surface)
        .is_some_and(|inhibitor| inhibitor.is_active())
}

pub(crate) fn handle_keyboard_key(
    state: &mut Villain,
    code: Keycode,
    key_state: KeyState,
    time: u32,
) {
    let pressed = key_state == KeyState::Pressed;
    let keyboard = state.keyboard.clone();
    let serial = SERIAL_COUNTER.next_serial();
    let (result, mods_changed) =
        keyboard.input_intercept(state, code, key_state, |state, mods, key| {
            let inhibited = shortcuts_inhibited(state);
            if !pressed
                && let Some((app_id, session_handle, shortcut_id)) =
                    state.active_global_shortcuts.remove(&code)
            {
                return FilterResult::Intercept(Some(KeyboardAction::Dispatch(
                    Dispatch::GlobalShortcutDeactivated {
                        app_id,
                        session_handle,
                        shortcut_id,
                    },
                )));
            }
            if !pressed
                && state
                    .pending_modifier
                    .as_ref()
                    .is_some_and(|(pending_code, _)| *pending_code == code)
                && let Some((_, dispatch)) = state.pending_modifier.take()
            {
                state.suppressed_keys.remove(&code);
                // The press was consumed before the inhibitor activated:
                // swallow the release instead of completing the binding, but
                // still keep the key from the client. Smithay's intercept path
                // below re-delivers the corrected modifier state either way.
                if inhibited {
                    return FilterResult::Intercept(None);
                }
                return FilterResult::Intercept(Some(KeyboardAction::Dispatch(dispatch)));
            }
            if !pressed && state.suppressed_keys.remove(&code) {
                return FilterResult::Intercept(None);
            }
            if state.suppressed_keys.contains(&code) {
                return FilterResult::Intercept(None);
            }
            if state.screenshot.selecting() {
                if pressed {
                    state.suppressed_keys.insert(code);
                    if key
                        .raw_syms()
                        .iter()
                        .any(|symbol| symbol.raw() == keysyms::KEY_Escape)
                    {
                        crate::screenshot::keyboard_key(state, keysyms::KEY_Escape, true);
                    }
                }
                return FilterResult::Intercept(None);
            }
            if state.interaction.is_some() {
                if pressed {
                    state.suppressed_keys.insert(code);
                    if key
                        .raw_syms()
                        .iter()
                        .any(|s| s.raw() == keysyms::KEY_Escape)
                    {
                        return FilterResult::Intercept(Some(KeyboardAction::FinishInteraction(
                            false,
                        )));
                    }
                    if key
                        .raw_syms()
                        .iter()
                        .any(|s| s.raw() == keysyms::KEY_Return)
                    {
                        return FilterResult::Intercept(Some(KeyboardAction::FinishInteraction(
                            true,
                        )));
                    }
                }
                return FilterResult::Intercept(None);
            }
            if pressed
                && state
                    .pending_modifier
                    .as_ref()
                    .is_some_and(|(pending_code, _)| *pending_code != code)
            {
                state.pending_modifier = None;
            }
            if pressed {
                tracing::debug!(?code, sym = ?key.modified_sym(), "key pressed");
            }
            if pressed && mods.ctrl && mods.alt {
                let sym = key.modified_sym().raw();
                let action = if key
                    .raw_syms()
                    .iter()
                    .any(|sym| sym.raw() == keysyms::KEY_BackSpace)
                {
                    Some(KeyboardAction::Dispatch(Dispatch::Quit))
                } else if state.tty.is_some()
                    && (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&sym)
                {
                    Some(KeyboardAction::Vt(
                        (sym - keysyms::KEY_XF86Switch_VT_1 + 1) as i32,
                    ))
                } else {
                    key.raw_syms().iter().find_map(|sym| {
                        (state.tty.is_some()
                            && (keysyms::KEY_F1..=keysyms::KEY_F12).contains(&sym.raw()))
                        .then(|| KeyboardAction::Vt((sym.raw() - keysyms::KEY_F1 + 1) as i32))
                    })
                };
                if let Some(action) = action {
                    state.suppressed_keys.insert(code);
                    return FilterResult::Intercept(Some(action));
                }
            }
            // Inhibition only removes the configurable bindings. Session
            // escape chords above run regardless, so an inhibited client can
            // never lock the user out of VT switching or session quit.
            if pressed
                && !inhibited
                && let Some(action) = state.config.keybinds.find_modifier_only(
                    &key.raw_syms(),
                    mods.ctrl,
                    mods.alt,
                    mods.shift,
                    mods.logo,
                )
            {
                state.pending_modifier = Some((code, action));
                state.suppressed_keys.insert(code);
                return FilterResult::Intercept(None);
            }
            if pressed
                && !inhibited
                && let Some(action) = state
                    .config
                    .keybinds
                    .find(&key.raw_syms(), mods.ctrl, mods.alt, mods.shift, mods.logo)
                    .or_else(|| {
                        state.global_keybinds.find(
                            &key.raw_syms(),
                            mods.ctrl,
                            mods.alt,
                            mods.shift,
                            mods.logo,
                        )
                    })
            {
                if let Dispatch::GlobalShortcutActivated {
                    app_id,
                    session_handle,
                    shortcut_id,
                } = &action
                {
                    state.active_global_shortcuts.insert(
                        code,
                        (app_id.clone(), session_handle.clone(), shortcut_id.clone()),
                    );
                }
                state.suppressed_keys.insert(code);
                return FilterResult::Intercept(Some(KeyboardAction::Dispatch(action)));
            }
            FilterResult::Forward
        });
    let action = match result {
        FilterResult::Forward => {
            keyboard.input_forward(state, code, key_state, serial, time, mods_changed);
            None
        }
        FilterResult::Intercept(action) => {
            // Intercepting a key also skips Smithay's modifier delivery. A
            // client entered while MOD is held must still see its release.
            if mods_changed && let Some(focus) = keyboard.current_focus() {
                focus.modifiers(
                    &state.seat.clone(),
                    state,
                    keyboard.modifier_state(),
                    serial,
                );
            }
            action
        }
    };
    match action {
        Some(KeyboardAction::FinishInteraction(commit)) => state.finish_interaction_key(commit),
        Some(KeyboardAction::Dispatch(dispatch)) => {
            if let Err(error) = state.dispatch(dispatch) {
                tracing::debug!(%error, "keybind dispatch had no effect");
            }
        }
        Some(KeyboardAction::Vt(vt)) => {
            use smithay::backend::session::Session;
            if let Some(tty) = state.tty.as_mut()
                && let Err(error) = tty.session.change_vt(vt)
            {
                tracing::warn!(%error, "VT switch failed");
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_shortcut_replacement_is_atomic_on_conflict() {
        let configured = KeybindRegistry::defaults("Super").unwrap();
        let mut globals = KeybindRegistry::empty();
        globals
            .register_global(
                "app.one".into(),
                "/session/one".into(),
                "toggle".into(),
                "CTRL+F9",
                "Super",
            )
            .unwrap();
        globals
            .register_global(
                "app.two".into(),
                "/session/two".into(),
                "toggle".into(),
                "CTRL+F10",
                "Super",
            )
            .unwrap();
        assert!(
            globals
                .replace_global(
                    &configured,
                    "app.one",
                    "/session/one",
                    "toggle",
                    "CTRL+F10",
                    "Super"
                )
                .is_err()
        );
        assert_eq!(
            globals.replace_global(
                &configured,
                "app.one",
                "/session/one",
                "toggle",
                "CTRL+F11",
                "Super"
            ),
            Ok("CTRL+F11".into())
        );
    }

    #[test]
    fn mouse_bindings_use_modkey_and_can_be_overridden() {
        let registry = KeybindRegistry::defaults("Alt").unwrap();
        let mut mods = smithay::input::keyboard::ModifiersState {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            registry.find_mouse(0x110, mods),
            Some(Dispatch::MoveWindow(
                knave_desktop_api::WindowTarget::Pointer
            ))
        );
        assert_eq!(
            registry.find_mouse(0x111, mods),
            Some(Dispatch::ResizeWindow(
                knave_desktop_api::WindowTarget::Pointer,
                None
            ))
        );
        mods.alt = false;
        assert!(registry.find_mouse(0x110, mods).is_none());
        let registry = KeybindRegistry::with_overrides(
            "Super",
            &[spec("MOD+BUTTON_LEFT", "resize-window", &["pointer"])],
        )
        .unwrap();
        mods.logo = true;
        assert_eq!(
            registry.find_mouse(0x110, mods),
            Some(Dispatch::ResizeWindow(
                knave_desktop_api::WindowTarget::Pointer,
                None
            ))
        );
        assert!(parse_keys("MOD+BUTTON_UNKNOWN", Modkey::Super).is_err());
    }

    #[test]
    fn resize_bindings_validate_delta_and_resolve_modkey() {
        let registry = KeybindRegistry::defaults("Super").unwrap();
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_Left)], true, false, false, true),
            Some(Dispatch::ResizeMaster(-5))
        );
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_Right)], true, false, false, true),
            Some(Dispatch::ResizeMaster(5))
        );
        assert_eq!(
            parse_dispatch(&spec("MOD+R", "reset-master", &[])).unwrap(),
            Dispatch::ResetMaster
        );
        assert_eq!(
            parse_dispatch(&spec("MOD+H", "resize-master", &["-2"])).unwrap(),
            Dispatch::ResizeMaster(-2)
        );
        for args in [
            vec![],
            vec!["0"],
            vec!["81"],
            vec!["-81"],
            vec!["NaN"],
            vec!["2", "3"],
        ] {
            assert!(parse_dispatch(&spec("MOD+H", "resize-master", &args)).is_err());
        }
    }

    #[test]
    fn modkey_changes_all_default_bindings() {
        let super_registry = KeybindRegistry::defaults("Super").unwrap();
        let q = [Keysym::new(keysyms::KEY_q)];
        assert_eq!(
            super_registry.find(&q, false, false, false, true),
            Some(Dispatch::CloseFocused)
        );
        assert_eq!(super_registry.find(&q, false, true, false, false), None);

        let alt_registry = KeybindRegistry::defaults("Alt").unwrap();
        assert_eq!(
            alt_registry.find(&q, false, true, false, false),
            Some(Dispatch::CloseFocused)
        );
    }

    #[test]
    fn default_modifier_release_toggles_session_overview() {
        let registry = KeybindRegistry::defaults("Super").unwrap();
        assert_eq!(
            registry.find_modifier_only(
                &[Keysym::new(keysyms::KEY_Super_L)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::ToggleOverview)
        );
        assert_eq!(
            parse_dispatch(&spec("MOD", "exec", &["knave-shell", "overview"])).unwrap(),
            Dispatch::ToggleOverview
        );
    }

    #[test]
    fn maximize_binding_and_explicit_actions() {
        let registry = KeybindRegistry::defaults("Super").unwrap();
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_f)], false, false, false, true),
            Some(Dispatch::ToggleMaximizeFocused)
        );
        for name in ["maximize", "unmaximize", "toggle-maximize"] {
            assert!(KeybindRegistry::from_specs("Super", &[spec("MOD+F", name, &[])]).is_ok());
            assert!(
                KeybindRegistry::from_specs("Super", &[spec("MOD+F", name, &["unexpected"])])
                    .is_err()
            );
        }
    }

    #[test]
    fn focus_switch_bindings_resolve_names_and_reject_arguments() {
        let registry = KeybindRegistry::defaults("Super").unwrap();
        let keys = |sym| [Keysym::new(sym)];
        assert_eq!(
            registry.find(&keys(keysyms::KEY_Tab), false, false, false, true),
            Some(Dispatch::FocusCycle(FocusCycleStep::Next))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_Tab), false, false, true, true),
            Some(Dispatch::FocusCycle(FocusCycleStep::Previous))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_ISO_Left_Tab), false, false, true, true),
            Some(Dispatch::FocusCycle(FocusCycleStep::Previous))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_h), false, false, false, true),
            Some(Dispatch::FocusDirection(FocusDirection::Left))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_j), false, false, false, true),
            Some(Dispatch::FocusDirection(FocusDirection::Down))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_k), false, false, false, true),
            Some(Dispatch::FocusDirection(FocusDirection::Up))
        );
        assert_eq!(
            registry.find(&keys(keysyms::KEY_l), false, false, false, true),
            Some(Dispatch::FocusDirection(FocusDirection::Right))
        );
        for (symbol, direction) in [
            (keysyms::KEY_h, FocusDirection::Left),
            (keysyms::KEY_j, FocusDirection::Down),
            (keysyms::KEY_k, FocusDirection::Up),
            (keysyms::KEY_l, FocusDirection::Right),
        ] {
            assert_eq!(
                registry.find(&keys(symbol), false, false, true, true),
                Some(Dispatch::MoveDirection(direction))
            );
        }
        for (keys, dispatch) in [
            ("MOD+TAB", "focus-next"),
            ("MOD+SHIFT+TAB", "focus-previous"),
            ("MOD+L", "focus-right"),
        ] {
            assert!(parse_dispatch(&spec(keys, dispatch, &["extra"])).is_err());
        }
    }

    #[test]
    fn numbered_move_defaults_and_overrides() {
        for (modkey, alt, logo) in [("Super", false, true), ("Alt", true, false)] {
            let registry = KeybindRegistry::defaults(modkey).unwrap();
            for workspace in 1..=10 {
                let key = Keysym::new(keysyms::KEY_0 + (workspace % 10) as u32);
                assert_eq!(
                    registry.find(&[key], false, alt, true, logo),
                    Some(Dispatch::MoveFocusedToWorkspace(workspace))
                );
                assert_eq!(
                    registry.find(&[key], false, alt, false, logo),
                    Some(Dispatch::FocusWorkspace(workspace))
                );
            }
        }
        let registry = KeybindRegistry::with_overrides(
            "Super",
            &[
                spec("MOD+SHIFT+2", "workspace", &["3"]),
                spec("MOD+B", "move-to-workspace", &["10"]),
            ],
        )
        .unwrap();
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_2)], false, false, true, true),
            Some(Dispatch::FocusWorkspace(3))
        );
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_b)], false, false, false, true),
            Some(Dispatch::MoveFocusedToWorkspace(10))
        );
        for args in [vec![], vec!["0"], vec!["x"], vec!["1", "2"]] {
            assert!(parse_dispatch(&spec("MOD+B", "move-to-workspace", &args)).is_err());
        }
        assert!(parse_dispatch(&spec("MOD+B", "workspace", &["11"])).is_ok());
    }

    #[test]
    fn last_workspace_binding_resolves_modkey_and_can_be_overridden() {
        let grave = [Keysym::new(keysyms::KEY_grave)];
        for (modkey, alt, logo) in [("Super", false, true), ("Alt", true, false)] {
            let registry = KeybindRegistry::defaults(modkey).unwrap();
            assert_eq!(
                registry.find(&grave, false, alt, false, logo),
                Some(Dispatch::LastWorkspace)
            );
        }
        for name in ["GRAVE", "TILDE", "~"] {
            let registry = KeybindRegistry::with_overrides(
                "Super",
                &[spec(&format!("MOD+{name}"), "workspace", &["4"])],
            )
            .unwrap();
            assert_eq!(
                registry.find(&grave, false, false, false, true),
                Some(Dispatch::FocusWorkspace(4))
            );
        }
        let registry =
            KeybindRegistry::with_overrides("Super", &[spec("MOD+B", "last-workspace", &[])])
                .unwrap();
        assert_eq!(
            registry.find(&[Keysym::new(keysyms::KEY_b)], false, false, false, true),
            Some(Dispatch::LastWorkspace)
        );
        assert!(parse_dispatch(&spec("MOD+GRAVE", "last-workspace", &["extra"])).is_err());
    }

    #[test]
    fn explicit_bindings_replace_only_the_matching_default() {
        let registry = KeybindRegistry::with_overrides(
            "Super",
            &[
                spec("MOD+Q", "exec", &["wofi"]),
                spec("MOD+LEFT", "exec", &["bemenu"]),
            ],
        )
        .unwrap();
        let q = [Keysym::new(keysyms::KEY_q)];
        let left = [Keysym::new(keysyms::KEY_Left)];
        assert_eq!(
            registry.find(&q, false, false, false, true),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );
        assert_eq!(
            registry.find(&left, false, false, false, true),
            Some(Dispatch::Spawn(vec!["bemenu".into()]))
        );
        // Unmentioned defaults keep working.
        assert_eq!(
            registry.find(
                &[Keysym::new(keysyms::KEY_Return)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["kitty".into()]))
        );
    }

    #[test]
    fn explicit_bindings_without_default_match_are_additive() {
        let registry =
            KeybindRegistry::with_overrides("Super", &[spec("MOD+H", "exec", &["bemenu"])])
                .unwrap();
        let h = [Keysym::new(keysyms::KEY_h)];
        assert_eq!(
            registry.find(&h, false, false, false, true),
            Some(Dispatch::Spawn(vec!["bemenu".into()]))
        );
        let q = [Keysym::new(keysyms::KEY_q)];
        assert_eq!(
            registry.find(&q, false, false, false, true),
            Some(Dispatch::CloseFocused)
        );
    }

    #[test]
    fn modifier_only_defaults_can_be_overridden() {
        let registry =
            KeybindRegistry::with_overrides("Super", &[spec("MOD", "exec", &["wofi"])]).unwrap();
        assert_eq!(
            registry.find_modifier_only(
                &[Keysym::new(keysyms::KEY_Super_L)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );
    }

    #[test]
    fn overriding_the_same_combination_twice_fails() {
        let overrides = vec![
            spec("MOD+Q", "minimize", &[]),
            spec("MOD+Q", "exec", &["bemenu"]),
        ];
        assert!(KeybindRegistry::with_overrides("Super", &overrides).is_err());
    }

    #[test]
    fn invalid_and_duplicate_bindings_are_rejected() {
        assert!(parse_keys("MOD+DOES_NOT_EXIST", Modkey::Super).is_err());
        let duplicate = vec![spec("MOD+Q", "close", &[]), spec("MOD+Q", "minimize", &[])];
        assert!(KeybindRegistry::from_specs("Super", &duplicate).is_err());
    }

    #[test]
    fn configured_exec_keeps_arguments() {
        let registry = KeybindRegistry::from_specs(
            "Super",
            &[spec("MOD+RETURN", "exec", &["kitty", "--single-instance"])],
        )
        .unwrap();
        assert_eq!(
            registry.find(
                &[Keysym::new(keysyms::KEY_Return)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec![
                "kitty".into(),
                "--single-instance".into()
            ]))
        );
    }

    #[test]
    fn modifier_only_binding_matches_both_sides_of_modkey() {
        let super_registry =
            KeybindRegistry::from_specs("Super", &[spec("MOD", "exec", &["wofi"])]).unwrap();
        assert_eq!(
            super_registry.find(
                &[Keysym::new(keysyms::KEY_Super_L)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );
        assert_eq!(
            super_registry.find(
                &[Keysym::new(keysyms::KEY_Super_R)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );

        let alt_registry =
            KeybindRegistry::from_specs("Alt", &[spec("MOD", "exec", &["wofi"])]).unwrap();
        assert_eq!(
            alt_registry.find(
                &[Keysym::new(keysyms::KEY_Alt_L)],
                false,
                true,
                false,
                false,
            ),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );
        assert_eq!(
            super_registry.find_modifier_only(
                &[Keysym::new(keysyms::KEY_Super_L)],
                false,
                false,
                false,
                true,
            ),
            Some(Dispatch::Spawn(vec!["wofi".into()]))
        );
        assert_eq!(
            super_registry.find_modifier_only(
                &[Keysym::new(keysyms::KEY_a)],
                false,
                false,
                false,
                true,
            ),
            None
        );
    }
}
