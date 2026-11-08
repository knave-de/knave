//! Matching and application of `[[compositor.device]]` rules to libinput
//! devices. Matching is pure and unit-tested; application is the thin libinput
//! boundary.

use knave_config::CompositorDeviceConfig;
use smithay::reexports::input::{
    AccelProfile, ClickMethod, Device, DeviceConfigResult, ScrollMethod, SendEventsMode,
};

/// Case-insensitive glob match supporting `*` (any run) and `?` (one character).
pub fn matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    glob(&pattern, &name)
}

fn glob(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| glob(rest, &name[skip..])),
        Some(('?', rest)) => matches!(name.split_first(), Some((_, tail)) if glob(rest, tail)),
        Some((head, rest)) => {
            matches!(name.split_first(), Some((ch, tail)) if ch == head && glob(rest, tail))
        }
    }
}

/// Resolve whether a device should receive events. Absence of an `enabled`
/// field means enabled, so removing a disable rule re-enables the device on the
/// next reload instead of leaving stale libinput state behind.
pub fn resolved_enabled(rules: &[CompositorDeviceConfig], name: &str) -> bool {
    let mut enabled = true;
    for rule in rules {
        if matches(&rule.name, name)
            && let Some(value) = rule.enabled
        {
            enabled = value;
        }
    }
    enabled
}

/// Set whether libinput delivers events from the device, without detaching it.
pub fn set_enabled(device: &mut Device, enabled: bool) {
    let name = device.name().to_owned();
    let mode = if enabled {
        SendEventsMode::ENABLED
    } else {
        SendEventsMode::DISABLED
    };
    warn(&name, "enabled", device.config_send_events_set_mode(mode));
}

/// Apply the tuning fields of one matched rule. Unset fields leave the libinput
/// value untouched, so rules are non-destructive and can be layered by name
/// specificity.
pub fn apply(device: &mut Device, rule: &CompositorDeviceConfig) {
    let name = device.name().to_owned();

    if let Some(enabled) = rule.tap_to_click
        && device.config_tap_finger_count() > 0
    {
        warn(
            &name,
            "tap-to-click",
            device.config_tap_set_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.tap_and_drag
        && device.config_tap_finger_count() > 0
    {
        warn(
            &name,
            "tap-and-drag",
            device.config_tap_set_drag_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.drag_lock
        && device.config_tap_finger_count() > 0
    {
        warn(
            &name,
            "drag-lock",
            device.config_tap_set_drag_lock_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.natural_scroll
        && device.config_scroll_has_natural_scroll()
    {
        warn(
            &name,
            "natural-scroll",
            device.config_scroll_set_natural_scroll_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.disable_while_typing
        && device.config_dwt_is_available()
    {
        warn(
            &name,
            "disable-while-typing",
            device.config_dwt_set_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.middle_emulation
        && device.config_middle_emulation_is_available()
    {
        warn(
            &name,
            "middle-emulation",
            device.config_middle_emulation_set_enabled(enabled),
        );
    }
    if let Some(enabled) = rule.left_handed
        && device.config_left_handed_is_available()
    {
        warn(&name, "left-handed", device.config_left_handed_set(enabled));
    }
    if let Some(speed) = rule.sensitivity
        && device.config_accel_is_available()
    {
        warn(&name, "sensitivity", device.config_accel_set_speed(speed));
    }
    if let Some(profile) = rule.accel_profile
        && device.config_accel_is_available()
    {
        warn(
            &name,
            "accel-profile",
            device.config_accel_set_profile(accel_profile(profile)),
        );
    }
    if let Some(method) = rule.scroll_method {
        warn(
            &name,
            "scroll-method",
            device.config_scroll_set_method(scroll_method(method)),
        );
    }
    if let Some(button) = rule.scroll_button {
        warn(
            &name,
            "scroll-button",
            device.config_scroll_set_button(button),
        );
    }
    if let Some(method) = rule.click_method {
        warn(
            &name,
            "click-method",
            device.config_click_set_method(click_method(method)),
        );
    }
}

fn warn(device: &str, setting: &str, result: DeviceConfigResult) {
    if result.is_err() {
        tracing::warn!(device, setting, "could not apply device setting");
    }
}

fn accel_profile(profile: knave_config::DeviceAccelProfile) -> AccelProfile {
    match profile {
        knave_config::DeviceAccelProfile::Adaptive => AccelProfile::Adaptive,
        knave_config::DeviceAccelProfile::Flat => AccelProfile::Flat,
    }
}

fn scroll_method(method: knave_config::DeviceScrollMethod) -> ScrollMethod {
    match method {
        knave_config::DeviceScrollMethod::NoScroll => ScrollMethod::NoScroll,
        knave_config::DeviceScrollMethod::TwoFinger => ScrollMethod::TwoFinger,
        knave_config::DeviceScrollMethod::Edge => ScrollMethod::Edge,
        knave_config::DeviceScrollMethod::OnButtonDown => ScrollMethod::OnButtonDown,
    }
}

fn click_method(method: knave_config::DeviceClickMethod) -> ClickMethod {
    match method {
        knave_config::DeviceClickMethod::ButtonAreas => ClickMethod::ButtonAreas,
        knave_config::DeviceClickMethod::Clickfinger => ClickMethod::Clickfinger,
    }
}

#[cfg(test)]
mod tests {
    use super::{matches, resolved_enabled};

    #[test]
    fn wildcards_and_case_are_insensitive() {
        assert!(matches(
            "*Receiver Mouse",
            "Example Wireless Receiver Mouse"
        ));
        assert!(matches("Example Touchpad", "example touchpad"));
        assert!(matches(
            "AT Translated Set 2 keyboard",
            "at translated set 2 keyboard"
        ));
        assert!(matches("Tou?hpad", "Touchpad"));
        assert!(matches("*", "anything"));
        assert!(matches("", ""));
    }

    #[test]
    fn non_matches_are_rejected() {
        assert!(!matches("Touchpad", "Touchpad Extra"));
        assert!(!matches("*Mouse", "AT Translated Set 2 keyboard"));
        assert!(!matches("", "not empty"));
        assert!(!matches("Tou?hpad", "Touhpad"));
    }

    #[test]
    fn enabled_defaults_to_true_and_the_last_matching_rule_wins() {
        use knave_config::CompositorDeviceConfig;

        let rule = |name: &str, enabled: Option<bool>| CompositorDeviceConfig {
            name: name.into(),
            enabled,
            ..Default::default()
        };

        assert!(resolved_enabled(&[], "Example Wireless Receiver Mouse"));
        assert!(resolved_enabled(
            &[rule("*", None)],
            "Example Wireless Receiver Mouse"
        ));
        assert!(!resolved_enabled(
            &[rule("*Receiver Mouse", Some(false))],
            "Example Wireless Receiver Mouse"
        ));
        assert!(resolved_enabled(
            &[
                rule("*Mouse", Some(false)),
                rule("Example Wireless*", Some(true)),
            ],
            "Example Wireless Receiver Mouse"
        ));
        // A rule for a mouse never affects the keyboard.
        assert!(resolved_enabled(
            &[rule("*Mouse", Some(false))],
            "AT Translated Set 2 keyboard"
        ));
    }
}
