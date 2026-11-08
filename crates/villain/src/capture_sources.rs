//! Standard window sources and immutable workspace restrictions for image capture.
use crate::{capture::SourceData, state::Villain};
use knave_capture_protocol::server::knave_workspace_capture_manager_v1 as workspace;
use knave_desktop_api::WindowId;
use smithay::{
    output::Output,
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
    wayland::foreign_toplevel_list::{
        ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
    },
};
use wayland_protocols::ext::image_capture_source::v1::server::ext_foreign_toplevel_image_capture_source_manager_v1 as windows;

#[derive(Clone)]
pub(crate) enum CaptureSource {
    Output(Output),
    Window(WindowId),
    Workspaces { output: Output, allowed: u32 },
    Unavailable,
}

pub(crate) fn valid_mask(mask: u32, count: usize) -> bool {
    count > 0 && count <= 32 && mask != 0 && (count == 32 || mask & !((1_u32 << count) - 1) == 0)
}

impl ForeignToplevelListHandler for Villain {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}
smithay::delegate_foreign_toplevel_list!(Villain);

pub(crate) fn update_windows(state: &mut Villain) {
    let info = state.window_info();
    state.capture.windows.retain(|id, handle| {
        if info.iter().any(|window| window.id.0 == *id) {
            return true;
        }
        state.foreign_toplevel_state.remove_toplevel(handle);
        false
    });
    for window in info {
        let handle = state.capture.windows.entry(window.id.0).or_insert_with(|| {
            let handle = state
                .foreign_toplevel_state
                .new_toplevel::<Villain>(&window.title, &window.app_id);
            handle.user_data().insert_if_missing(|| window.id);
            handle
        });
        if handle.title() != window.title || handle.app_id() != window.app_id {
            handle.send_title(&window.title);
            handle.send_app_id(&window.app_id);
            handle.send_done();
        }
    }
}

pub(crate) fn init(display: &DisplayHandle) {
    display
        .create_global::<Villain, windows::ExtForeignToplevelImageCaptureSourceManagerV1, _>(1, ());
    display.create_global::<Villain, workspace::KnaveWorkspaceCaptureManagerV1, _>(1, ());
}
impl GlobalDispatch<windows::ExtForeignToplevelImageCaptureSourceManagerV1, ()> for Villain {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<windows::ExtForeignToplevelImageCaptureSourceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl Dispatch<windows::ExtForeignToplevelImageCaptureSourceManagerV1, ()> for Villain {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &windows::ExtForeignToplevelImageCaptureSourceManagerV1,
        request: windows::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let windows::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            let target = ForeignToplevelHandle::from_resource(&toplevel_handle)
                .filter(|handle| !handle.is_closed())
                .and_then(|handle| handle.user_data().get::<WindowId>().copied())
                .filter(|id| state.capture_window(*id).is_some())
                .map_or(CaptureSource::Unavailable, CaptureSource::Window);
            init.init(source, SourceData(target));
        }
    }
}
impl GlobalDispatch<workspace::KnaveWorkspaceCaptureManagerV1, ()> for Villain {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<workspace::KnaveWorkspaceCaptureManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ())
            .workspace_count(state.workspaces.len() as u32);
    }
}
impl Dispatch<workspace::KnaveWorkspaceCaptureManagerV1, ()> for Villain {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &workspace::KnaveWorkspaceCaptureManagerV1,
        request: workspace::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let workspace::Request::CreateSource {
            source,
            output,
            allowed_workspaces,
        } = request
        {
            if !valid_mask(allowed_workspaces, state.workspaces.len()) {
                init.init(source, SourceData(CaptureSource::Unavailable));
                resource.post_error(
                    workspace::Error::InvalidMask,
                    "unsupported or empty workspace mask",
                );
                return;
            }
            let target = Output::from_resource(&output)
                .filter(|output| state.space.outputs().any(|candidate| candidate == output))
                .map_or(CaptureSource::Unavailable, |output| {
                    CaptureSource::Workspaces {
                        output,
                        allowed: allowed_workspaces,
                    }
                });
            init.init(source, SourceData(target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_contiguous_masks_are_valid_but_outside_workspaces_are_rejected() {
        assert!(valid_mask(0b1_0000_0111, 10));
        assert!(!valid_mask(0, 10));
        assert!(!valid_mask(1 << 10, 10));
        assert!(valid_mask(u32::MAX, 32));
        assert!(!valid_mask(1, 33));
    }
}
