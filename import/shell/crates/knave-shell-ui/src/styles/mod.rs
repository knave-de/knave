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
