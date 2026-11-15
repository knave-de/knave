//! Opt-in GTK shared-header experiment; no persistent setting or protocol contract.
use smithay::{
    desktop::Window,
    utils::{Logical, Rectangle},
};
use std::sync::OnceLock;

type Geometry = Rectangle<i32, Logical>;

fn selected_app() -> Option<&'static str> {
    static APP: OnceLock<Option<String>> = OnceLock::new();
    APP.get_or_init(|| std::env::var("VILLAIN_EXPERIMENTAL_SHARED_HEADER_APP_ID").ok())
        .as_deref()
        .filter(|id| !id.is_empty())
}

pub(crate) fn owns_pointer(state: &crate::state::Villain, window: &Window) -> bool {
    if selected_app().is_none() {
        return false;
    }
    state
        .workspace_layout(state.active_workspace)
        .into_iter()
        .find(|entry| entry.0 == *window && entry.5)
        .and_then(|(_, rect, fullscreen, _, _, _)| controls_rect(window, rect, fullscreen))
        .is_some_and(|rect| rect.to_f64().contains(state.pointer_location))
}

pub(crate) fn controls_rect(
    window: &Window,
    client: Geometry,
    fullscreen: bool,
) -> Option<Geometry> {
    let app = selected_app()?;
    let top = window.toplevel()?;
    if fullscreen || top.parent().is_some() || crate::workspaces::has_compositor_titlebar(window) {
        return None;
    }
    let matches = smithay::wayland::compositor::with_states(top.wl_surface(), |states| {
        states
            .data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            .and_then(|data| {
                data.lock()
                    .ok()
                    .map(|data| data.app_id.as_deref() == Some(app))
            })
            .unwrap_or(false)
    });
    if !matches {
        return None;
    }
    let controls = reserved_rect(client)?;
    // Only overlapping popups displace controls; a tooltip below the header
    // must not remove its buttons or make the reservation client-interactive.
    if smithay::desktop::PopupManager::popups_for_surface(top.wl_surface()).any(
        |(popup, location)| {
            Geometry::new(client.loc + location, popup.geometry().size)
                .intersection(controls)
                .is_some()
        },
    ) {
        return None;
    }
    Some(controls)
}

fn reserved_rect(client: Geometry) -> Option<Geometry> {
    // The isolated GTK adapter reserves this exact logical rectangle. Never
    // infer a header from pixels or cover content in a window too small for it.
    (client.size.w >= 96 && client.size.h >= 46).then(|| {
        Geometry::new(
            (client.loc.x + client.size.w - 96, client.loc.y + 6).into(),
            (96, 34).into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservation_stays_inside_client_and_adds_no_frame_height() {
        let client = Geometry::new((40, 80).into(), (640, 480).into());
        let controls = reserved_rect(client).unwrap();
        assert_eq!(controls, Geometry::new((584, 86).into(), (96, 34).into()));
        assert_eq!(controls.intersection(client), Some(controls));
        assert!(reserved_rect(Geometry::from_size((95, 480).into())).is_none());
        assert!(reserved_rect(Geometry::from_size((640, 45).into())).is_none());
    }
}
