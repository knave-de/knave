//! Explicitly enabled, Winit-only synthetic input for the isolated desktop harness.
use crate::state::Villain;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::{
    os::unix::{fs::PermissionsExt, net::UnixDatagram},
    path::PathBuf,
};

pub(crate) struct TestSocket(PathBuf);
impl Drop for TestSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub(crate) fn install(
    state: &mut Villain,
) -> Result<Option<TestSocket>, Box<dyn std::error::Error>> {
    if std::env::var_os("VILLAIN_GESTURE_TEST").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Ok(None);
    }
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("missing runtime directory")?);
    let path = runtime.join("villain-gesture-test.sock");
    // Never replace an existing socket/file; the harness owns a fresh runtime directory.
    let socket = UnixDatagram::bind(&path)?;
    let cleanup = TestSocket(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    socket.set_nonblocking(true)?;
    state.loop_handle.insert_source(
        Generic::new(socket, Interest::READ, Mode::Level),
        |_, socket, state| {
            // One bounded datagram per dispatch keeps synthetic traffic from monopolizing the loop.
            let mut buffer = [0; 512];
            match socket.recv_from(&mut buffer) {
                Ok((size, address)) => {
                    let request = serde_json::from_slice::<serde_json::Value>(&buffer[..size]);
                    let response = request
                        .map_err(|error| error.to_string())
                        .and_then(|request| apply(state, &request));
                    let response = match response {
                        Ok(value) => value,
                        Err(error) => serde_json::json!({"error": error}),
                    };
                    if let Some(path) = address.as_pathname() {
                        let _ = socket.send_to(response.to_string().as_bytes(), path);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => tracing::warn!(%error, "gesture test socket receive failed"),
            }
            Ok(PostAction::Continue)
        },
    )?;
    Ok(Some(cleanup))
}
fn apply(state: &mut Villain, request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let device = request
        .get("device")
        .and_then(|value| value.as_str())
        .unwrap_or("harness");
    if device.len() > 64 {
        return Err("device ID too long".into());
    }
    match request.get("op").and_then(|value| value.as_str()) {
        Some("begin") => {
            let fingers = request
                .get("fingers")
                .and_then(|value| value.as_u64())
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or("invalid finger count")?;
            state.swipe_begin(device.into(), fingers, 0);
        }
        Some("update") => {
            let x = request
                .get("x")
                .and_then(|value| value.as_f64())
                .filter(|value| value.is_finite())
                .ok_or("invalid x")?;
            let y = request
                .get("y")
                .and_then(|value| value.as_f64())
                .filter(|value| value.is_finite())
                .ok_or("invalid y")?;
            state.swipe_update(device, x, y, 0);
        }
        Some("end") => state.swipe_end(
            device,
            request
                .get("cancelled")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            0,
        ),
        Some("remove") => state.swipe_device_removed(device),
        Some("query") => {}
        _ => return Err("expected begin/update/end/remove/query".into()),
    }
    Ok(serde_json::json!({
        "active_workspace": state.active_workspace + 1,
        "motion": state.workspace_motion.scenes(std::time::Instant::now()).into_iter()
            .map(|(workspace, x)| serde_json::json!({"workspace": workspace + 1, "x": x})).collect::<Vec<_>>(),
        "motion_timer": state.workspace_motion.timer.is_some(),
        "offset": state.gestures.transition.map(|transition| transition.offset),
        "source": state.gestures.transition.map(|transition| transition.source + 1),
        "target": state.gestures.transition.map(|transition| transition.target + 1),
    }))
}
