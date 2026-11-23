//! `zwp_virtual_keyboard_v1` input: keymap activation and modifier requests.
//!
//! A virtual keyboard's keycodes and modifier masks only mean something under
//! the keymap its client uploaded (wtype, for example, maps `v` to keycode 1).
//! The seat switches to that keymap while the device sends input and switches
//! back on the next physical key or when the device goes away.

use std::collections::HashMap;

use smithay::{
    backend::input::{Event, InputEvent, KeyboardKeyEvent},
    input::keyboard::KeymapFile,
    wayland::virtual_keyboard::{
        VirtualKeyboardBackend, VirtualKeyboardDevice, VirtualKeyboardSpecialEvent,
    },
};

use crate::state::Villain;

#[derive(Default)]
pub(crate) struct VirtualKeymapState {
    /// Device whose keymap the seat uses, with the seat keymap to restore.
    active: Option<(VirtualKeyboardDevice, KeymapFile)>,
    /// Depressed/latched bits set by `no_keymap` devices, which share the
    /// seat's xkb state and are not reset when they disappear.
    shared_masks: HashMap<VirtualKeyboardDevice, (u32, u32)>,
}

pub(crate) fn process_event(state: &mut Villain, event: InputEvent<VirtualKeyboardBackend>) {
    match event {
        InputEvent::Keyboard { event } => {
            let device = event.device();
            activate(state, &device);
            crate::keybinds::handle_keyboard_key_from_source(
                state,
                device.source(),
                event.key_code(),
                event.state(),
                event.time().millis(),
            );
        }
        InputEvent::Special(VirtualKeyboardSpecialEvent::Modifiers {
            device,
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
        }) => {
            activate(state, &device);
            if device.keymap().is_none() {
                state
                    .virtual_keymap
                    .shared_masks
                    .insert(device, (mods_depressed, mods_latched));
            }
            let keyboard = state.keyboard.clone();
            keyboard.with_xkb_state(state, |mut context| {
                context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group)
            });
        }
        InputEvent::Special(VirtualKeyboardSpecialEvent::KeymapChanged { device }) => {
            // Inactive devices pick up their new keymap on their next input.
            if is_active(state, &device) {
                restore_seat_keymap(state);
                activate(state, &device);
            }
        }
        InputEvent::DeviceRemoved { device } => {
            if is_active(state, &device) {
                // Rebuilding the seat xkb state drops the device's modifiers.
                restore_seat_keymap(state);
            }
            if let Some(masks) = state.virtual_keymap.shared_masks.remove(&device) {
                clear_shared_mask(state, masks);
            }
        }
        _ => {}
    }
}

/// Restore the seat keymap before handling physical keyboard input.
pub(crate) fn restore_seat_keymap(state: &mut Villain) {
    let Some((_, seat_keymap)) = state.virtual_keymap.active.take() else {
        return;
    };
    let keyboard = state.keyboard.clone();
    if let Err(error) = keyboard.set_keymap_from_keymap_file(state, seat_keymap) {
        tracing::error!(
            ?error,
            "could not restore the seat keymap after virtual keyboard input"
        );
    }
}

fn is_active(state: &Villain, device: &VirtualKeyboardDevice) -> bool {
    state
        .virtual_keymap
        .active
        .as_ref()
        .is_some_and(|(owner, _)| owner == device)
}

fn activate(state: &mut Villain, device: &VirtualKeyboardDevice) {
    let Some(keymap) = device.keymap() else {
        // `no_keymap` devices send keycodes for the seat keymap.
        restore_seat_keymap(state);
        return;
    };
    if is_active(state, device) {
        return;
    }
    let keyboard = state.keyboard.clone();
    let seat_keymap = match state.virtual_keymap.active.take() {
        Some((_, seat_keymap)) => seat_keymap,
        None => keyboard.keymap_file(),
    };
    match keyboard.set_keymap_from_keymap_file(state, keymap) {
        Ok(()) => state.virtual_keymap.active = Some((device.clone(), seat_keymap)),
        Err(error) => {
            tracing::warn!(?error, "could not activate a virtual keyboard keymap");
            // A previous virtual keymap may still be loaded; return to the seat's.
            state.virtual_keymap.active = Some((device.clone(), seat_keymap));
            restore_seat_keymap(state);
        }
    }
}

fn clear_shared_mask(state: &mut Villain, (depressed, latched): (u32, u32)) {
    if depressed == 0 && latched == 0 {
        return;
    }
    let keyboard = state.keyboard.clone();
    let current = keyboard.modifier_state().serialized;
    keyboard.with_xkb_state(state, |mut context| {
        context.set_modifier_mask(
            current.depressed & !depressed,
            current.latched & !latched,
            current.locked,
            current.layout_effective,
        )
    });
}
