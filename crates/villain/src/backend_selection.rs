//! Select from the controlling terminal, not the globally active console.
//!
//! A desktop terminal has a pseudoterminal even while its desktop owns a VT.
//! /proc/self/stat also works when stdin/stdout have been redirected to a file.

pub fn default_is_direct() -> bool {
    let console = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| console_number(&stat));
    let has_display = ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    select_direct(console, has_display)
}

fn select_direct(console: Option<u32>, has_display: bool) -> bool {
    console.is_some() || !has_display
}

fn console_number(stat: &str) -> Option<u32> {
    // Field 2 (comm) can contain spaces and parentheses. After its final ')',
    // fields start with state, ppid, pgrp, session, then tty_nr (field 7).
    let fields = stat.rsplit_once(')')?.1;
    let device = fields.split_whitespace().nth(4)?.parse::<i32>().ok()? as u32;
    let major = (device >> 8) & 0xff;
    let minor = (device & 0xff) | ((device >> 12) & 0xfff00);
    // Linux virtual consoles are /dev/tty1..tty63, major 4. Serial terminals
    // and /dev/pts/* must not be mistaken for a local virtual console.
    (major == 4 && (1..=63).contains(&minor)).then_some(minor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_wins_over_inherited_desktop_environment() {
        let console = console_number("42 (villain (test)) S 1 42 42 1027 42");
        assert_eq!(console, Some(3));
        assert!(select_direct(console, true));
    }

    #[test]
    fn desktop_pseudoterminal_stays_nested() {
        let console = console_number("42 (villain) S 1 42 42 34816 42");
        assert_eq!(console, None);
        assert!(!select_direct(console, true));
    }

    #[test]
    fn missing_or_other_terminal_uses_display_fallback() {
        for stat in [
            "invalid",
            "42 (villain) S 1 42 42 0",
            "42 (villain) S 1 42 42 1088",
        ] {
            assert_eq!(console_number(stat), None);
        }
        assert!(select_direct(None, false));
        assert!(!select_direct(None, true));
    }
}
