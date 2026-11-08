//! Villain, a deliberately small Smithay compositor.
//!
//! Winit runs nested in a desktop; the TTY backend owns a Linux seat and output.
//! Both backends use the same Wayland protocol and workspace state.

mod appearance;
mod backend_selection;
mod capture;
mod capture_sources;
mod capture_window;
mod config;
mod cursor;
mod dispatch;
mod focus;
mod frame_clock;
mod gesture_test;
mod gestures;
mod handlers;
mod input_devices;
mod ipc;
mod keybinds;
mod layer_shell;
mod motion;
mod night_light;
mod overview_render;
mod preview;
mod render;
mod screenshot;
mod screenshot_notification;
mod session;
mod shutdown;
mod split_grab;
mod state;
mod tty;
mod window_grab;
mod workspace_motion;
mod workspace_navigation;
mod workspaces;
mod xwayland;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use state::Villain;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args: Vec<_> = std::env::args().skip(1).collect();
    let direct = match args.as_slice() {
        [] => backend_selection::default_is_direct(),
        [arg] if arg == "--tty" => true,
        [arg] if arg == "--winit" => false,
        [arg] if arg == "--help" => {
            println!(
                "villain [--tty | --winit]\nDefault: Winit inside a desktop, DRM/KMS from a TTY."
            );
            return Ok(());
        }
        _ => return Err("usage: villain [--tty | --winit]".into()),
    };
    tracing::info!(
        backend = if direct { "tty" } else { "winit" },
        "selected backend"
    );
    // Parse options before creating sockets so --help works without a session.
    let mut event_loop: EventLoop<'static, Villain> = EventLoop::try_new()?;
    shutdown::install(&mut event_loop)?;
    let display = Display::new()?;
    let config = config::RuntimeConfig::load()?;
    let mut state = Villain::new(&mut event_loop, display, config);
    let _socket_cleanup = session::SocketCleanup::new(&state.socket_name);
    state.owns_session = direct;
    session::prepare_environment(&mut state);
    xwayland::init(&mut event_loop, &mut state);
    if direct {
        tty::init(&mut event_loop, &mut state)?;
    } else {
        render::init_winit(&mut event_loop, &mut state)?;
    }
    if direct {
        session::activate(&mut state, true);
    }

    let _gesture_test = if !direct {
        gesture_test::install(&mut state)?
    } else {
        None
    };
    tracing::info!(socket = ?state.socket_name, "Villain is ready");
    if direct {
        tracing::info!("submitting initial direct TTY frame");
    }
    state.render_if_needed();
    if direct {
        tracing::info!("initial direct TTY frame request completed");
    }
    event_loop.run(None, &mut state, |state| {
        state.maintain_swipe();
        state.reap_children();
        state.space.refresh();
        state.popups.cleanup();
        state.publish_desktop_state();
        let _ = state.display_handle.flush_clients();
        state.render_if_needed();
    })?;

    state.cancel_swipe(0);
    state.cancel_workspace_motion();
    shutdown::reset();
    Ok(())
}

fn init_logging() {
    let subscriber = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "villain=info".into()),
    );
    subscriber.init();
}
