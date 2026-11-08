//! Preserve the native keyboard target so XWayland also receives X11 focus changes.

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::Window,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::state::Villain;

#[derive(Clone, Debug, PartialEq)]
pub enum KeyboardFocus {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl KeyboardFocus {
    pub fn for_window(window: &Window) -> Option<Self> {
        if let Some(surface) = window.x11_surface() {
            Some(Self::X11(surface.clone()))
        } else {
            window
                .wl_surface()
                .map(|surface| Self::Wayland(surface.into_owned()))
        }
    }
}

impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(surface) => surface.alive(),
            Self::X11(surface) => surface.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(surface) => Some(Cow::Borrowed(surface)),
            Self::X11(surface) => surface.wl_surface().map(Cow::Owned),
        }
    }
}

impl KeyboardTarget<Villain> for KeyboardFocus {
    fn enter(
        &self,
        seat: &Seat<Villain>,
        data: &mut Villain,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
            Self::X11(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<Villain>, data: &mut Villain, serial: Serial) {
        match self {
            Self::Wayland(surface) => KeyboardTarget::leave(surface, seat, data, serial),
            Self::X11(surface) => KeyboardTarget::leave(surface, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<Villain>,
        data: &mut Villain,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::key(surface, seat, data, key, state, serial, time)
            }
            Self::X11(surface) => {
                KeyboardTarget::key(surface, seat, data, key, state, serial, time)
            }
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<Villain>,
        data: &mut Villain,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::modifiers(surface, seat, data, modifiers, serial)
            }
            Self::X11(surface) => KeyboardTarget::modifiers(surface, seat, data, modifiers, serial),
        }
    }
}

impl From<smithay::desktop::PopupKind> for KeyboardFocus {
    fn from(popup: smithay::desktop::PopupKind) -> Self {
        Self::Wayland(popup.wl_surface().clone())
    }
}

impl From<KeyboardFocus> for WlSurface {
    fn from(focus: KeyboardFocus) -> Self {
        match focus {
            KeyboardFocus::Wayland(surface) => surface,
            KeyboardFocus::X11(surface) => {
                surface.wl_surface().expect("popup grab has a mapped root")
            }
        }
    }
}
