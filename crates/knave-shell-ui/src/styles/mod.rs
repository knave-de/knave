//! Shell-specific presentation layered over Knave UI's widget styles.
use knave_ui::toolkit::{Element, Theme};

pub const STYLESHEET: &str = concat!(
    include_str!("common.css"),
    "\n",
    include_str!("status-bar.css"),
    "\n",
    include_str!("overview.css"),
    "\n",
    include_str!("notifications.css"),
    "\n",
    include_str!("quick-settings.css"),
    "\n",
    include_str!("dialogs.css"),
);

pub fn root(element: Element, theme: Theme) -> Element {
    element.class("knave-shell").class(match theme {
        Theme::Light => "shell-light",
        Theme::Dark => "shell-dark",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use knave_ui::toolkit::{Chroma, Contrast, Scene, ThemePolicy, Widget};

    #[test]
    fn shell_styles_parse_in_every_appearance_profile() {
        for theme in [Theme::Light, Theme::Dark] {
            for contrast in [Contrast::Standard, Contrast::High] {
                for chroma in [Chroma::FullColor, Chroma::Monochrome] {
                    let policy = ThemePolicy::new(theme, contrast, chroma);
                    let mut scene = Scene::new(Element::new(1, Widget::Panel)).unwrap();
                    scene
                        .set_theme_policy_stylesheets(policy, STYLESHEET, "")
                        .unwrap();
                }
            }
        }
    }
}
