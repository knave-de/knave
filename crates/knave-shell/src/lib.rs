//! Desktop-facing composition on the retained Knave toolkit.
pub mod overview;

pub mod background_permission;

pub mod portal;
pub mod shortcut_assignment;

pub mod wallpaper;

pub mod notifications;
pub mod wallpaper_reload;

pub mod quick_settings;

pub mod polkit;

/// Resolve the toolkit stylesheet selection for Knave's desktop preference.
/// `system` has no authoritative system source yet, so toolkit scenes use dark.
pub fn toolkit_theme(preference: knave_config::Theme) -> knave_ui::toolkit::Theme {
    match preference {
        knave_config::Theme::Light => knave_ui::toolkit::Theme::Light,
        knave_config::Theme::System | knave_config::Theme::Dark => knave_ui::toolkit::Theme::Dark,
    }
}

#[cfg(test)]
mod theme_tests {
    #[test]
    fn system_preference_uses_the_documented_dark_toolkit_fallback() {
        assert_eq!(
            super::toolkit_theme(knave_config::Theme::System),
            knave_ui::toolkit::Theme::Dark
        );
    }

    #[test]
    fn explicit_light_preference_selects_light_toolkit_css() {
        assert_eq!(
            super::toolkit_theme(knave_config::Theme::Light),
            knave_ui::toolkit::Theme::Light
        );
    }
}

#[doc(hidden)]
pub mod entrypoint;
