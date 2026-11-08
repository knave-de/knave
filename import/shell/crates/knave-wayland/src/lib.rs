//! Knave-owned Wayland layer-shell and wgpu runtime.

mod application;
mod clipboard;
mod cursor;
mod focused_app;
mod harness_ready;
mod ime;
pub mod notifications;
pub mod quick_settings;
mod snapshot;
mod theme;
pub mod tray;
pub mod tray_menu;
pub use application::{
    Application, HostRequest, KeyboardMode, SurfaceLayer, SurfaceOptions, SurfacePosition, Waker,
    run_application, run_application_with_theme,
};
use knave_ui::toolkit::{Input, KeyModifiers};
use snapshot::SnapshotWorker;

use std::{
    io::Cursor,
    num::NonZeroU32,
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use base64::Engine;
use knave_desktop_api::{
    ClientError, DesktopClient, DesktopCommand, DesktopQuery, DesktopRequest, DesktopResponse,
    DesktopSnapshot, OverviewPane, WorkspaceId, WorkspacePreview,
};
use knave_renderer::{RenderList, WgpuPainter, WgpuRenderer};
use knave_shell_ui::{MAX_SEARCH_QUERY, UiAction, UiScene, WorkspacePreviewImage};
use knave_ui::UiImage;
use smithay_client_toolkit::reexports::{
    calloop::{EventLoop, channel},
    calloop_wayland_source::WaylandSource,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
        pointer::{
            BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
        },
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};
use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellRole {
    Bar,
    Overview,
}

impl ShellRole {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bar" => Some(Self::Bar),
            "overview" => Some(Self::Overview),
            _ => None,
        }
    }

    fn layer(self) -> Layer {
        match self {
            Self::Bar => Layer::Top,
            Self::Overview => Layer::Overlay,
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            Self::Bar => "knave-shell-bar",
            Self::Overview => "knave-shell-overview",
        }
    }

    fn keyboard_interactivity(self) -> KeyboardInteractivity {
        match self {
            Self::Bar => KeyboardInteractivity::None,
            Self::Overview => KeyboardInteractivity::Exclusive,
        }
    }

    fn requested_size(self) -> (u32, u32) {
        match self {
            Self::Bar => (0, knave_shell_ui::status_bar::STATUS_BAR_HEIGHT as u32),
            Self::Overview => (0, 0),
        }
    }

    fn anchors(self) -> Anchor {
        match self {
            Self::Bar => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            Self::Overview => Anchor::TOP | Anchor::RIGHT | Anchor::BOTTOM | Anchor::LEFT,
        }
    }

    fn exclusive_zone(self) -> i32 {
        match self {
            Self::Bar => knave_shell_ui::status_bar::STATUS_BAR_HEIGHT as i32,
            Self::Overview => -1,
        }
    }

    fn scene(self, revision: u64, width: f32, height: f32, input: SceneInput<'_>) -> UiScene {
        match self {
            Self::Bar => UiScene::bar_with_snapshot(revision, width, height, input.snapshot),
            Self::Overview => UiScene::overview_with_snapshot_and_search(
                revision,
                width,
                height,
                input.snapshot,
                input.query,
                input.selected,
                input.previews,
            ),
        }
    }
}

#[derive(Debug)]
struct SceneInput<'a> {
    snapshot: Option<&'a DesktopSnapshot>,
    query: &'a str,
    selected: usize,
    previews: &'a [WorkspacePreviewImage],
}

#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("invalid surface options: {0}")]
    InvalidOptions(&'static str),
    #[error("could not connect to the Knave Wayland compositor: {0}")]
    Connect(String),
    #[error("could not enumerate Wayland globals: {0}")]
    Globals(String),
    #[error("wl_compositor is unavailable: {0}")]
    Compositor(String),
    #[error("wl_shm is unavailable: {0}")]
    Shm(String),
    #[error("wlr-layer-shell is unavailable: {0}")]
    LayerShell(String),
    #[error("no compatible GPU adapter was found: {0}")]
    Adapter(String),
    #[error("could not create the GPU device: {0}")]
    Device(String),
    #[error("the Wayland surface has no supported configuration")]
    SurfaceConfiguration,
    #[error("Wayland dispatch failed: {0}")]
    Dispatch(String),
    #[error("could not signal desktop harness readiness: {0}")]
    HarnessReadiness(#[source] std::io::Error),
}

const MAX_PREVIEWS: usize = 3;
const MAX_PREVIEW_PIXELS: u64 = 36 * 1024 * 1024;
const MAX_PREVIEW_BASE64_LENGTH: usize = 192 * 1024 * 1024;

enum RuntimeWake {
    Redraw,
}

type WakeSender = channel::SyncSender<RuntimeWake>;

#[derive(Debug)]
struct PreviewUpdate {
    generation: u64,
    previews: Vec<WorkspacePreviewImage>,
}

enum PreviewCommand {
    Refresh {
        generation: u64,
        workspaces: Vec<WorkspaceId>,
        size: [u32; 2],
    },
}

struct PreviewWorker {
    commands: Option<SyncSender<PreviewCommand>>,
    updates: Arc<Mutex<Option<PreviewUpdate>>>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewWorker {
    fn start(wake: WakeSender) -> Self {
        let (commands, command_rx) = mpsc::sync_channel(1);
        let updates = Arc::new(Mutex::new(None));
        let update_slot = Arc::clone(&updates);
        let thread = thread::spawn(move || {
            let mut client = None;
            while let Ok(command) = command_rx.recv() {
                match command {
                    PreviewCommand::Refresh {
                        generation,
                        mut workspaces,
                        size,
                    } => {
                        workspaces.truncate(MAX_PREVIEWS);
                        let previews = query_previews(&mut client, &workspaces, size);
                        if let Ok(mut slot) = update_slot.lock() {
                            *slot = Some(PreviewUpdate {
                                generation,
                                previews,
                            });
                            let _ = wake.try_send(RuntimeWake::Redraw);
                        }
                    }
                }
            }
        });
        Self {
            commands: Some(commands),
            updates,
            thread: Some(thread),
        }
    }

    fn request_refresh(
        &self,
        generation: u64,
        workspaces: Vec<WorkspaceId>,
        size: [u32; 2],
    ) -> bool {
        self.commands
            .as_ref()
            .unwrap()
            .try_send(PreviewCommand::Refresh {
                generation,
                workspaces,
                size,
            })
            .is_ok()
    }

    fn latest(&self) -> Option<PreviewUpdate> {
        self.updates.lock().ok()?.take()
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn query_previews(
    client: &mut Option<DesktopClient>,
    workspaces: &[WorkspaceId],
    size: [u32; 2],
) -> Vec<WorkspacePreviewImage> {
    if workspaces.is_empty() {
        return Vec::new();
    }
    if client.is_none() {
        *client = DesktopClient::connect().ok();
    }
    let Some(_) = client.as_mut() else {
        return Vec::new();
    };

    let mut previews = Vec::with_capacity(workspaces.len().min(MAX_PREVIEWS));
    let mut failed = false;
    for workspace in workspaces.iter().take(MAX_PREVIEWS).copied() {
        let response = client
            .as_mut()
            .expect("preview client was initialized")
            .request(&DesktopRequest::Query(DesktopQuery::WorkspacePreview {
                workspace,
                width: size[0],
                height: size[1],
            }));
        match response {
            Ok(DesktopResponse::WorkspacePreview(preview)) => {
                if let Some(preview) = decode_preview(preview, size) {
                    previews.push(preview);
                }
            }
            Ok(_) => {}
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    if failed {
        *client = None;
    }
    previews
}

fn decode_preview(preview: WorkspacePreview, size: [u32; 2]) -> Option<WorkspacePreviewImage> {
    if [preview.width, preview.height] != size {
        return None;
    }
    if preview.width == 0
        || preview.height == 0
        || u64::from(preview.width) * u64::from(preview.height) > MAX_PREVIEW_PIXELS
        || preview.png_base64.len() > MAX_PREVIEW_BASE64_LENGTH
    {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD
        .decode(preview.png_base64)
        .ok()?;
    let mut decoder = png::Decoder::new(Cursor::new(encoded));
    decoder.set_limits(png::Limits {
        bytes: MAX_PREVIEW_BASE64_LENGTH,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let pixel_count =
        u64::from(reader.info().width).checked_mul(u64::from(reader.info().height))?;
    if [reader.info().width, reader.info().height] != size
        || pixel_count == 0
        || pixel_count > MAX_PREVIEW_PIXELS
    {
        return None;
    }
    let max_bytes = usize::try_from(pixel_count.checked_mul(4)?).ok()?;
    let output_size = reader.output_buffer_size()?;
    if output_size > max_bytes {
        return None;
    }
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.width != preview.width || info.height != preview.height {
        return None;
    }
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => {
            buffer.truncate(info.buffer_size());
            buffer
        }
        png::ColorType::Rgb => data
            .chunks(3)
            .filter(|pixel| pixel.len() == 3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => data
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .filter(|pixel| pixel.len() == 2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(WorkspacePreviewImage {
        workspace: preview.workspace,
        image: UiImage::from_rgba(info.width, info.height, rgba)?,
    })
}

/// One bounded IPC worker replaces the PNG preview worker for application overviews.
struct PaneWorker {
    commands: Option<SyncSender<Vec<OverviewPane>>>,
    errors: Receiver<String>,
    acknowledgements: Receiver<Vec<OverviewPane>>,
    thread: Option<JoinHandle<()>>,
}
impl PaneWorker {
    fn start(wake: WakeSender) -> Self {
        let (commands, receiver) = mpsc::sync_channel::<Vec<OverviewPane>>(1);
        let (errors_tx, errors) = mpsc::sync_channel(1);
        let (ack_tx, acknowledgements) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
            const MAX_BACKOFF: Duration = Duration::from_secs(5);
            let mut client: Option<DesktopClient> = None;
            while let Ok(mut panes) = receiver.recv() {
                let mut backoff = INITIAL_BACKOFF;
                loop {
                    // New geometry replaces a failed request; never replay stale panes.
                    while let Ok(newer) = receiver.try_recv() {
                        panes = newer;
                        backoff = INITIAL_BACKOFF;
                    }
                    let result = (|| {
                        if client.is_none() {
                            client = Some(DesktopClient::connect()?);
                        }
                        client.as_mut().expect("connected above").request(
                            &DesktopRequest::SetOverviewPanes {
                                panes: panes.clone(),
                            },
                        )
                    })();
                    let retry = match result {
                        Ok(DesktopResponse::Ok) => {
                            let _ = ack_tx.try_send(panes.clone());
                            false
                        }
                        Ok(_) => {
                            let _ =
                                errors_tx.try_send("Unexpected overview pane response".to_owned());
                            client = None;
                            false
                        }
                        Err(error) => {
                            let retryable =
                                !matches!(&error, ClientError::Remote(remote) if !remote.retryable);
                            if !retryable {
                                let _ = errors_tx.try_send(error.to_string());
                            }
                            client = None;
                            retryable
                        }
                    };
                    // Let the UI submit its newest geometry if the one-entry queue was full.
                    let _ = wake.try_send(RuntimeWake::Redraw);
                    if !retry {
                        break;
                    }
                    match receiver.recv_timeout(backoff) {
                        Ok(newer) => {
                            panes = newer;
                            backoff = INITIAL_BACKOFF;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            backoff = (backoff * 2).min(MAX_BACKOFF);
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        Self {
            commands: Some(commands),
            errors,
            acknowledgements,
            thread: Some(thread),
        }
    }
    fn update(&self, panes: Vec<OverviewPane>) -> bool {
        self.commands
            .as_ref()
            .is_some_and(|sender| sender.try_send(panes).is_ok())
    }
    fn latest_ack(&self) -> Option<Vec<OverviewPane>> {
        self.acknowledgements.try_iter().last()
    }
    fn latest_error(&self) -> Option<String> {
        self.errors.try_iter().last()
    }
}
impl Drop for PaneWorker {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum ActionCommand {
    Dispatch(DesktopCommand),
}

struct ActionWorker {
    results: Receiver<Result<(), String>>,
    commands: Option<SyncSender<ActionCommand>>,
    thread: Option<JoinHandle<()>>,
}

impl ActionWorker {
    fn start(wake: WakeSender) -> Self {
        let (commands, command_rx) = mpsc::sync_channel(1);
        let (results_tx, results) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let mut client = None;
            while let Ok(command) = command_rx.recv() {
                match command {
                    ActionCommand::Dispatch(command) => {
                        let result = dispatch_action(&mut client, command);
                        if let Err(error) = &result {
                            eprintln!("knave-shell: shell action failed: {error}");
                            client = None;
                        }
                        let _ = results_tx.try_send(result);
                        let _ = wake.try_send(RuntimeWake::Redraw);
                    }
                }
            }
        });
        Self {
            results,
            commands: Some(commands),
            thread: Some(thread),
        }
    }

    fn dispatch(&self, command: DesktopCommand) -> Result<(), String> {
        self.commands
            .as_ref()
            .unwrap()
            .try_send(ActionCommand::Dispatch(command))
            .map_err(|_| "Desktop action queue is busy".into())
    }
    fn latest(&self) -> Option<Result<(), String>> {
        self.results.try_iter().last()
    }
}

impl Drop for ActionWorker {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn dispatch_action(
    client: &mut Option<DesktopClient>,
    command: DesktopCommand,
) -> Result<(), String> {
    if client.is_none() {
        *client = Some(DesktopClient::connect().map_err(|error| error.to_string())?);
    }
    match client
        .as_mut()
        .expect("desktop client was initialized")
        .request(&DesktopRequest::Dispatch(command))
        .map_err(|error| error.to_string())?
    {
        DesktopResponse::Ok => Ok(()),
        _ => Err("Unexpected desktop action response".into()),
    }
}

pub fn run(role: ShellRole) -> Result<(), WaylandError> {
    run_with_theme(role, knave_ui::toolkit::Theme::Dark)
}

/// Run a shell surface with the canonical Knave appearance preference applied.
pub fn run_with_theme(
    role: ShellRole,
    theme: knave_ui::toolkit::Theme,
) -> Result<(), WaylandError> {
    run_internal(role, None, theme)
}

fn run_internal(
    role: ShellRole,
    application: Option<(SurfaceOptions, Box<dyn Application>)>,
    theme: knave_ui::toolkit::Theme,
) -> Result<(), WaylandError> {
    let (options, mut app) = match application {
        Some((options, app)) => (Some(options), Some(app)),
        None => (None, None),
    };

    let connection = Connection::connect_to_env().map_err(|error| {
        WaylandError::Connect(format!(
            "{error}; start the Knave compositor and use its WAYLAND_DISPLAY"
        ))
    })?;
    let (globals, event_queue) = registry_queue_init(&connection)
        .map_err(|error| WaylandError::Globals(error.to_string()))?;
    let queue_handle = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &queue_handle)
        .map_err(|error| WaylandError::Compositor(error.to_string()))?;
    let shm =
        Shm::bind(&globals, &queue_handle).map_err(|error| WaylandError::Shm(error.to_string()))?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)
        .map_err(|error| WaylandError::LayerShell(error.to_string()))?;

    let fractional_scale_manager = globals
        .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(
            &queue_handle,
            1..=1,
            (),
        )
        .ok();
    let viewporter_manager = globals
        .bind::<wp_viewporter::WpViewporter, _, _>(&queue_handle, 1..=1, ())
        .ok();

    // Bind seats before configuring the layer: capabilities must be dispatched
    // before configure can map the first buffer under an already-present pointer.
    let seat_state = SeatState::new(&globals, &queue_handle);

    let surface = compositor.create_surface(&queue_handle);
    let (fractional_scale, viewport) = match (
        fractional_scale_manager.as_ref(),
        viewporter_manager.as_ref(),
    ) {
        (Some(scale_manager), Some(viewport_manager)) => (
            Some(scale_manager.get_fractional_scale(&surface, &queue_handle, ())),
            Some(viewport_manager.get_viewport(&surface, &queue_handle, ())),
        ),
        _ => (None, None),
    };
    let layer = layer_shell.create_layer_surface(
        &queue_handle,
        surface,
        options
            .as_ref()
            .map_or_else(|| role.layer(), |options| options.layer.wayland()),
        Some(
            options
                .as_ref()
                .map_or(role.namespace(), |o| o.namespace.as_str()),
        ),
        None,
    );
    if let Some(options) = &options {
        layer.set_anchor(if options.size.is_none() {
            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT
        } else {
            options.position.anchors()
        });
        layer.set_keyboard_interactivity(match options.keyboard {
            KeyboardMode::None => KeyboardInteractivity::None,
            KeyboardMode::OnDemand => KeyboardInteractivity::OnDemand,
            KeyboardMode::Exclusive => KeyboardInteractivity::Exclusive,
        });
        layer.set_exclusive_zone(-1);
        let [w, h] = options.size.unwrap_or([0, 0]);
        layer.set_size(w, h);
    } else {
        layer.set_anchor(role.anchors());
        layer.set_keyboard_interactivity(role.keyboard_interactivity());
        layer.set_exclusive_zone(role.exclusive_zone());
        let (width, height) = role.requested_size();
        layer.set_size(width, height);
    }
    if fractional_scale.is_some() && viewport.is_some() {
        layer.wl_surface().set_buffer_scale(1);
    }
    layer.commit();

    let renderer = WgpuRenderer::new();
    let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
        NonNull::new(connection.backend().display_ptr() as *mut _)
            .expect("Wayland connection returned a null display pointer"),
    ));
    let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
        NonNull::new(layer.wl_surface().id().as_ptr() as *mut _)
            .expect("Wayland surface returned a null object pointer"),
    ));
    let surface = unsafe {
        renderer
            .instance()
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })
            .map_err(|error| WaylandError::Connect(error.to_string()))?
    };

    let adapter = pollster::block_on(renderer.instance().request_adapter(
        &wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        },
    ))
    .map_err(|error| WaylandError::Adapter(error.to_string()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .map_err(|error| WaylandError::Device(error.to_string()))?;

    let mut event_loop: EventLoop<Runtime> =
        EventLoop::try_new().map_err(|error| WaylandError::Dispatch(error.to_string()))?;
    WaylandSource::new(connection.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|error| WaylandError::Dispatch(error.to_string()))?;
    let (wake_sender, wake_channel) = channel::sync_channel(1);
    let wake_queue_handle = queue_handle.clone();
    event_loop
        .handle()
        .insert_source(wake_channel, move |event, _, state| match event {
            channel::Event::Msg(RuntimeWake::Redraw) => state.request_draw(&wake_queue_handle),
            channel::Event::Closed => state.exit = true,
        })
        .map_err(|error| WaylandError::Dispatch(error.to_string()))?;

    let uses_theme = app.as_ref().is_none_or(|app| app.uses_theme());
    let selected_theme = match if uses_theme {
        theme::Selection::load()
    } else {
        Ok(theme::Selection::fallback(theme))
    } {
        Ok(selected) => selected,
        Err(error) => {
            eprintln!("knave-shell: theme: {error}; using toolkit defaults");
            theme::Selection::fallback(theme)
        }
    };
    if let Some(app) = &mut app {
        app.theme_policy_changed(selected_theme.policy, &selected_theme.css)
            .map_err(|error| WaylandError::Dispatch(format!("theme: {error}")))?;
    }
    // Recheck after watch registration so an edit during startup cannot be missed.
    let theme_pending = Arc::new(AtomicBool::new(uses_theme));
    let pending = theme_pending.clone();
    let theme_wake = Waker::new(wake_sender.clone());
    let theme_watcher = uses_theme
        .then(|| {
            knave_ui::toolkit::theme_file::FileWatcher::start(theme::watch_paths, move || {
                pending.store(true, Ordering::Release);
                theme_wake.wake();
            })
            .map_err(|error| eprintln!("knave-shell: theme reload unavailable: {error}"))
            .ok()
        })
        .flatten();

    let desktop_role = options.is_none() || app.as_ref().is_some_and(|app| app.uses_desktop());
    let application_overview = app.is_some() && role == ShellRole::Overview;
    let clipboard_manager =
        smithay_client_toolkit::data_device_manager::DataDeviceManagerState::bind(
            &globals,
            &queue_handle,
        )
        .ok();
    let ime_manager=globals.bind::<wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3,_,_>(&queue_handle,1..=1,()).ok();
    let notification_bar = ((options.is_none() && role == ShellRole::Bar)
        || app.as_ref().is_some_and(|app| app.uses_status_bar()))
        && std::env::var_os("KNAVE_NOTIFICATIONS_ENABLED").as_deref()
            != Some(std::ffi::OsStr::new("0"));
    if let Some(app) = &mut app {
        app.notifications_enabled(notification_bar);
    }
    let tray_bar = (options.is_none() && role == ShellRole::Bar)
        || app.as_ref().is_some_and(|app| app.uses_status_bar());
    let mut state = Runtime {
        harness_ready: harness_ready::ReadySignal::from_env(
            options
                .as_ref()
                .map_or(role.namespace(), |options| options.namespace.as_str()),
        ),
        harness_error: None,
        wake_sender: wake_sender.clone(),
        connection: connection.clone(),
        compositor,
        shm,
        _fractional_scale_manager: fractional_scale_manager,
        _viewporter_manager: viewporter_manager,
        fractional_scale,
        viewport,
        cursor: cursor::CursorState::default(),
        clipboard: clipboard::Clipboard::new(clipboard_manager),
        ime: ime::Ime::new(ime_manager),
        loop_handle: event_loop.handle(),
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        seat_state,
        active_seat: None,
        keyboard: None,
        pointer: None,
        role,
        theme_policy: selected_theme.policy,
        user_theme_css: selected_theme.css,
        theme_pending,
        _theme_watcher: theme_watcher,
        app,
        options,
        notification_client: notification_bar.then(|| {
            notifications::NotificationClient::start_summary(Waker::new(wake_sender.clone()))
        }),
        notification_snapshot: None,
        quick_settings_client: tray_bar
            .then(|| quick_settings::QuickSettingsClient::start(Waker::new(wake_sender.clone()))),
        tray_client: tray_bar.then(|| tray::TrayClient::start(Waker::new(wake_sender.clone()))),
        applets: Vec::new(),
        app_input_region: None,
        app_keyboard_mode: None,
        integer_scale: 1,
        preferred_scale: 1.0,
        modifiers: KeyModifiers::default(),
        layer,
        surface,
        adapter,
        device,
        queue,
        width: 1,
        height: 1,
        revision: 0,
        frame_pending: false,
        snapshot_worker: desktop_role.then(|| SnapshotWorker::start(wake_sender.clone())),
        focused_icon_worker: tray_bar
            .then(|| focused_app::Worker::start(Waker::new(wake_sender.clone()))),
        focused_icon_key: None,
        focused_icon_token: 0,
        focused_icon_request: None,
        focused_app_icon: None,
        action_worker: desktop_role.then(|| ActionWorker::start(wake_sender.clone())),
        preview_worker: (desktop_role && role == ShellRole::Overview && !application_overview)
            .then(|| PreviewWorker::start(wake_sender.clone())),
        pane_worker: (desktop_role && application_overview)
            .then(|| PaneWorker::start(wake_sender.clone())),
        pane_requested: Vec::new(),
        pane_acknowledged: None,
        snapshot: None,
        previews: Vec::new(),
        preview_generation: 0,
        preview_requested: Vec::new(),
        preview_size: [0, 0],
        preview_refresh: true,
        search_query: String::new(),
        search_index: 0,
        scene: UiScene::new(0),
        render_list: RenderList::default(),
        scene_dirty: true,
        painter: None,
        painter_format: None,
        configured: false,
        surface_buffer_attached: false,
        exit: false,
    };

    if let Some(app) = &mut state.app {
        app.set_waker(application::Waker::new(wake_sender.clone()));
    }
    while !state.exit {
        event_loop
            .dispatch(None, &mut state)
            .map_err(|error| WaylandError::Dispatch(error.to_string()))?;
    }

    state.cancel_clipboard();
    if let Some(error) = state.harness_error.take() {
        return Err(WaylandError::HarnessReadiness(error));
    }
    Ok(())
}

struct Runtime {
    harness_ready: harness_ready::ReadySignal,
    harness_error: Option<std::io::Error>,
    // Keep the wake source alive even when no desktop workers are needed.
    wake_sender: WakeSender,
    connection: Connection,
    compositor: CompositorState,
    shm: Shm,
    _fractional_scale_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    _viewporter_manager: Option<wp_viewporter::WpViewporter>,
    fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    viewport: Option<wp_viewport::WpViewport>,
    cursor: cursor::CursorState,
    ime: ime::Ime,
    clipboard: clipboard::Clipboard,
    loop_handle: smithay_client_toolkit::reexports::calloop::LoopHandle<'static, Self>,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    active_seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    role: ShellRole,
    theme_policy: knave_ui::toolkit::ThemePolicy,
    user_theme_css: String,
    theme_pending: Arc<AtomicBool>,
    _theme_watcher: Option<knave_ui::toolkit::theme_file::FileWatcher>,
    app: Option<Box<dyn Application>>,
    options: Option<SurfaceOptions>,
    notification_client: Option<notifications::NotificationClient>,
    quick_settings_client: Option<quick_settings::QuickSettingsClient>,
    notification_snapshot: Option<knave_desktop_api::notifications::NotificationSnapshot>,
    tray_client: Option<tray::TrayClient>,
    applets: Vec<knave_shell_ui::status_bar::Applet>,
    app_input_region: Option<Vec<knave_ui::Rect>>,
    app_keyboard_mode: Option<KeyboardMode>,
    integer_scale: u32,
    preferred_scale: f64,
    modifiers: KeyModifiers,
    layer: LayerSurface,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    painter: Option<WgpuPainter>,
    snapshot_worker: Option<SnapshotWorker>,
    focused_icon_worker: Option<focused_app::Worker>,
    focused_icon_key: Option<focused_app::Identity>,
    focused_icon_token: u64,
    focused_icon_request: Option<focused_app::Request>,
    focused_app_icon: Option<UiImage>,
    action_worker: Option<ActionWorker>,
    preview_worker: Option<PreviewWorker>,
    pane_worker: Option<PaneWorker>,
    pane_requested: Vec<OverviewPane>,
    pane_acknowledged: Option<Vec<OverviewPane>>,
    snapshot: Option<DesktopSnapshot>,
    previews: Vec<WorkspacePreviewImage>,
    preview_generation: u64,
    preview_requested: Vec<WorkspaceId>,
    preview_size: [u32; 2],
    preview_refresh: bool,
    search_query: String,
    search_index: usize,
    scene: UiScene,
    render_list: RenderList,
    scene_dirty: bool,
    width: u32,
    height: u32,
    painter_format: Option<wgpu::TextureFormat>,
    revision: u64,
    frame_pending: bool,
    configured: bool,
    surface_buffer_attached: bool,
    exit: bool,
}

impl Runtime {
    fn fractional_scaling(&self) -> bool {
        self.fractional_scale.is_some() && self.viewport.is_some()
    }

    fn render_scale(&self) -> f64 {
        if self.fractional_scaling() {
            self.preferred_scale
        } else {
            f64::from(self.integer_scale)
        }
    }

    fn set_viewport_destination(&self) {
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(
                i32::try_from(self.width).unwrap_or(i32::MAX),
                i32::try_from(self.height).unwrap_or(i32::MAX),
            );
        }
    }

    fn preferred_scale_changed(&mut self, scale_120ths: u32, qh: &QueueHandle<Self>) {
        let scale = fractional_scale_from_protocol(scale_120ths);
        if self.preferred_scale == scale {
            return;
        }
        self.preferred_scale = scale;
        if !self.configured {
            return;
        }
        if let Some(config) = self.configure_surface(self.width, self.height) {
            self.surface.configure(&self.device, &config);
        }
        self.set_viewport_destination();
        self.scene_dirty = true;
        self.request_draw(qh);
    }

    fn set_focused_app(&mut self, snapshot: Option<&DesktopSnapshot>) {
        let next = snapshot.and_then(focused_app::identity);
        if next == self.focused_icon_key {
            return;
        }
        self.focused_icon_key = next;
        self.focused_icon_token = self.focused_icon_token.wrapping_add(1);
        if let Some(worker) = &self.focused_icon_worker {
            worker.cancel_before(self.focused_icon_token);
        }
        self.focused_app_icon = None;
        self.focused_icon_request =
            focused_app::request(self.focused_icon_key.as_ref(), self.focused_icon_token);
        if let Some(app) = &mut self.app {
            app.focused_app_icon(None, None);
        }
        self.scene_dirty = true;
    }

    fn configure_surface(&self, width: u32, height: u32) -> Option<wgpu::SurfaceConfiguration> {
        let scale = self.render_scale();
        let mut config = self.surface.get_default_config(
            &self.adapter,
            scaled_buffer_dimension(width.max(1), scale),
            scaled_buffer_dimension(height.max(1), scale),
        )?;
        let Some(format) = self
            .surface
            .get_capabilities(&self.adapter)
            .formats
            .into_iter()
            .find(|format| format.is_srgb())
        else {
            eprintln!("knave-shell: GPU surface does not support an sRGB output format");
            return None;
        };
        config.format = format;
        if self
            .surface
            .get_capabilities(&self.adapter)
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
        }
        Some(config)
    }

    fn request_draw(&mut self, qh: &QueueHandle<Self>) {
        if self.configured && !self.frame_pending {
            self.draw(qh);
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        if !self.configured {
            return;
        }
        if self.exit {
            return;
        }
        if self.theme_pending.swap(false, Ordering::AcqRel) {
            let result = theme::Selection::load().and_then(|selected| {
                if let Some(app) = &mut self.app {
                    app.theme_policy_changed(selected.policy, &selected.css)
                        .map_err(|error| error.to_string())?;
                }
                self.theme_policy = selected.policy;
                self.user_theme_css = selected.css;
                self.scene_dirty = true;
                Ok(())
            });
            if let Err(error) = result {
                eprintln!("knave-shell: theme reload: {error}; keeping previous theme");
            }
        }
        if let Some(error) = self.pane_worker.as_ref().and_then(PaneWorker::latest_error)
            && let Some(app) = &mut self.app
        {
            app.host_error(&format!("Live workspace preview unavailable: {error}"));
            self.pane_acknowledged = Some(self.pane_requested.clone());
            self.scene_dirty = true;
        }
        if let Some(ack) = self.pane_worker.as_ref().and_then(PaneWorker::latest_ack) {
            self.pane_acknowledged = Some(ack);
            self.scene_dirty = true;
        }
        if let Some(app) = &mut self.app {
            app.update();
        }
        if let Some(update) = self
            .notification_client
            .as_ref()
            .and_then(notifications::NotificationClient::latest)
        {
            self.notification_snapshot = update.ok();
            if let Some(app) = &mut self.app {
                app.notification_snapshot(self.notification_snapshot.as_ref());
            }
            self.scene_dirty = true;
        }
        if let Some(update) = self.tray_client.as_ref().and_then(tray::TrayClient::latest) {
            self.applets = match update {
                Ok(items) => items,
                Err(error) => {
                    eprintln!("knave-shell: applets unavailable: {error}");
                    Vec::new()
                }
            };
            if let Some(app) = &mut self.app {
                app.applets(&self.applets);
            }
            self.scene_dirty = true;
        }
        let mut should_render =
            self.scene_dirty || self.app.as_ref().is_some_and(|app| app.needs_frame());
        if let Some(update) = self
            .snapshot_worker
            .as_ref()
            .and_then(SnapshotWorker::latest)
        {
            match update {
                Ok(snapshot) => {
                    self.set_focused_app(Some(&snapshot));
                    if let Some(app) = &mut self.app {
                        app.desktop_snapshot(&snapshot);
                    }
                    // Every frame is a change or reconnect initialization. A restarted
                    // compositor may reuse identical metadata with different pixels.
                    self.preview_refresh = true;
                    self.pane_requested.clear();
                    self.pane_acknowledged = None;
                    self.snapshot = Some(snapshot);
                }
                Err(_) => {
                    self.set_focused_app(None);
                    self.snapshot = None;
                    self.preview_refresh = true;
                    self.pane_requested.clear();
                    self.pane_acknowledged = None;
                    if let Some(app) = &mut self.app {
                        app.desktop_unavailable();
                    }
                }
            }
            self.scene_dirty = true;
            should_render = true;
        }
        if let Some(result) = self
            .focused_icon_worker
            .as_ref()
            .and_then(focused_app::Worker::take)
            && focused_app::matches(
                self.focused_icon_key.as_ref(),
                self.focused_icon_token,
                &result,
            )
        {
            self.focused_app_icon = result.image;
            if let Some(app) = &mut self.app {
                app.focused_app_icon(Some(&result.app_id), self.focused_app_icon.as_ref());
            }
            self.scene_dirty = true;
            should_render = true;
        }
        if let Some(request) = self.focused_icon_request.take()
            && !self
                .focused_icon_worker
                .as_ref()
                .is_some_and(|worker| worker.submit(request.clone()))
        {
            self.focused_icon_request = Some(request);
        }
        if let Some(result) = self.action_worker.as_ref().and_then(ActionWorker::latest)
            && let Some(app) = &mut self.app
        {
            app.desktop_action_finished(result);
            self.exit |= app.should_close();
            self.scene_dirty = true;
            should_render = true;
        }
        if self.exit {
            return;
        }
        // Lifecycle requests must progress even when the application is unmapped.
        self.application_requests(qh);
        if self.app.as_ref().is_some_and(|app| !app.surface_visible()) {
            if self.surface_buffer_attached {
                self.layer.wl_surface().attach(None, 0, 0);
                self.layer.wl_surface().commit();
                self.surface_buffer_attached = false;
                self.frame_pending = false;
                if let Some(app) = &mut self.app {
                    app.surface_unmapped();
                }
            }
            self.scene_dirty = false;
            self.notify_harness_ready();
            return;
        }
        let mut desired = self
            .app
            .as_ref()
            .and_then(|app| app.preview_workspaces())
            .unwrap_or_else(|| {
                self.snapshot.as_ref().map_or_else(Vec::new, |s| {
                    s.workspaces
                        .iter()
                        .take(MAX_PREVIEWS)
                        .map(|w| w.workspace)
                        .collect()
                })
            });
        desired.truncate(MAX_PREVIEWS);
        let preview_scale = self.render_scale();
        let preview_size = [
            scaled_buffer_dimension(self.width, preview_scale),
            scaled_buffer_dimension(self.height, preview_scale),
        ];
        if (self.preview_refresh
            || desired != self.preview_requested
            || preview_size != self.preview_size)
            && let Some(worker) = &self.preview_worker
        {
            let generation = self.preview_generation.wrapping_add(1);
            // On a full queue retry when the existing job wakes us; never lose the newest selection.
            if worker.request_refresh(generation, desired.clone(), preview_size) {
                self.preview_generation = generation;
                self.preview_requested = desired;
                self.preview_size = preview_size;
                self.preview_refresh = false;
            }
        }
        if let Some(worker) = &self.preview_worker
            && let Some(update) = worker.latest()
            && self.preview_generation == update.generation
        {
            self.previews = update.previews;
            if let Some(app) = &mut self.app {
                app.workspace_previews(&self.previews);
            }
            self.scene_dirty = true;
            should_render = true;
        }
        if self.app.is_none() && self.scene_dirty {
            if self.role == ShellRole::Bar {
                let mut bar = knave_shell_ui::status_bar::StatusBar::with_focused_icon(
                    self.revision,
                    self.width as f32,
                    self.height as f32,
                    self.snapshot.as_ref(),
                    self.notification_client.is_some(),
                    self.notification_snapshot.as_ref(),
                    &self.applets,
                    self.focused_app_icon.as_ref(),
                );
                bar.set_user_policy(self.theme_policy, &self.user_theme_css)
                    .expect("bundled shell theme stylesheet is valid");
                self.scene = bar.scene;
                self.render_list = bar.display;
            } else {
                let mut scene = self.role.scene(
                    self.revision,
                    self.width as f32,
                    self.height as f32,
                    SceneInput {
                        snapshot: self.snapshot.as_ref(),
                        query: &self.search_query,
                        selected: self.search_index,
                        previews: &self.previews,
                    },
                );
                scene.set_theme_policy(self.theme_policy);
                self.scene = scene;
                self.render_list = self.scene.display_list();
            }
            self.scene_dirty = false;
        }
        if !should_render {
            if let Some(panes) = self.app.as_ref().and_then(|app| app.overview_panes())
                && panes != self.pane_requested
                && let Some(worker) = &self.pane_worker
                && worker.update(panes.clone())
            {
                self.pane_requested = panes;
            }
            return;
        }
        if let Some(app) = &mut self.app {
            if app.should_close() {
                self.exit = true;
                return;
            }
            if let Some(painter) = &mut self.painter {
                self.render_list = app
                    .frame([self.width as f32, self.height as f32], painter)
                    .clone();
            }
            self.scene_dirty = false;
        }
        if let Some(panes) = self.app.as_ref().and_then(|app| app.overview_panes())
            && let Some(worker) = &self.pane_worker
        {
            if (panes != self.pane_requested || self.pane_acknowledged.as_ref() != Some(&panes))
                && worker.update(panes.clone())
            {
                self.pane_requested = panes.clone();
            }
            // The compositor stages geometry; publish its matching shell frame only after acknowledgement.
            if self.pane_acknowledged.as_ref() != Some(&panes) {
                return;
            }
        }
        if let Some(app) = &self.app {
            let mode = app.keyboard_mode();
            if mode != self.app_keyboard_mode {
                if let Some(mode) = mode {
                    self.layer.set_keyboard_interactivity(match mode {
                        KeyboardMode::None => KeyboardInteractivity::None,
                        KeyboardMode::OnDemand => KeyboardInteractivity::OnDemand,
                        KeyboardMode::Exclusive => KeyboardInteractivity::Exclusive,
                    });
                }
                self.app_keyboard_mode = mode;
            }
            let input = app.input_region();
            if input != self.app_input_region {
                if let Some(rects) = &input {
                    match smithay_client_toolkit::compositor::Region::new(&self.compositor) {
                        Ok(region) => {
                            for r in rects {
                                region.add(
                                    r.x.floor() as i32,
                                    r.y.floor() as i32,
                                    r.width.ceil() as i32,
                                    r.height.ceil() as i32,
                                );
                            }
                            self.layer
                                .wl_surface()
                                .set_input_region(Some(region.wl_region()));
                        }
                        Err(error) => {
                            eprintln!("knave-shell: input region unavailable: {error}");
                            self.exit = true;
                            return;
                        }
                    }
                } else {
                    self.layer.wl_surface().set_input_region(None);
                }
                self.app_input_region = input;
            }
        }
        self.sync_ime();
        self.sync_cursor();
        self.application_requests(qh);
        let scale = self.render_scale();
        let scaled;
        let render_list = if scale > 1.0 {
            scaled = scale_list(&self.render_list, scale as f32);
            &scaled
        } else {
            &self.render_list
        };
        let [red, green, blue, alpha] = render_list.clear_color.to_linear_rgba();
        let clear_color = wgpu::Color {
            r: f64::from(red),
            g: f64::from(green),
            b: f64::from(blue),
            a: f64::from(alpha),
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.scene_dirty = true;
                if let Some(config) = self.configure_surface(self.width, self.height) {
                    self.surface.configure(&self.device, &config);
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.scene_dirty = true;
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("knave-shell: GPU surface validation failed");
                self.exit = true;
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("knave-shell-frame"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("knave-shell-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        if let Some(painter) = &mut self.painter
            && let Err(error) = painter.encode(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                (
                    scaled_buffer_dimension(self.width, scale),
                    scaled_buffer_dimension(self.height, scale),
                ),
                render_list,
            )
        {
            eprintln!("knave-shell: renderer failed to prepare frame: {error}");
        }
        self.frame_pending = true;
        self.layer
            .wl_surface()
            .frame(qh, FrameCallbackData(self.layer.wl_surface().clone()));
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        self.surface_buffer_attached = true;
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Runtime {
    fn notify_harness_ready(&mut self) {
        if self.configured
            && self.painter.is_some()
            && (self.snapshot.is_some() || self.app.as_ref().is_some_and(|app| !app.uses_desktop()))
            && let Err(error) = self.harness_ready.notify()
        {
            self.harness_error = Some(error);
            self.exit = true;
        }
    }

    fn handle_key(&mut self, qh: &QueueHandle<Self>, event: KeyEvent) {
        if self.role != ShellRole::Overview {
            return;
        }

        let raw_keysym = event.keysym.raw();
        match raw_keysym {
            0xff1b => {
                self.exit = true;
                return;
            }
            0xff0d => {
                if let Some(action) = self.scene.search_action(self.search_index) {
                    self.dispatch_ui_action(action);
                }
                return;
            }
            0xff51 | 0xff52 => {
                self.search_index = self.search_index.saturating_sub(1);
                self.scene_dirty = true;
                self.request_draw(qh);
                return;
            }
            0xff53 | 0xff54 => {
                let result_count = self.scene.search_result_count();
                if result_count > 0 {
                    self.search_index = (self.search_index + 1).min(result_count - 1);
                }
                self.scene_dirty = true;
                self.request_draw(qh);
                return;
            }
            0xff08 => {
                if self.search_query.pop().is_some() {
                    self.search_index = 0;
                    self.scene_dirty = true;
                    self.request_draw(qh);
                }
                return;
            }
            0x31..=0x39 if self.search_query.is_empty() => {
                self.dispatch_ui_action(UiAction::FocusWorkspace(WorkspaceId(raw_keysym - 0x30)));
                return;
            }
            0x30 if self.search_query.is_empty() => {
                self.dispatch_ui_action(UiAction::FocusWorkspace(WorkspaceId(10)));
                return;
            }
            _ => {}
        }

        if let Some(text) = event.utf8 {
            let mut changed = false;
            for character in text.chars().filter(|character| !character.is_control()) {
                if self.search_query.chars().count() >= MAX_SEARCH_QUERY {
                    break;
                }
                self.search_query.push(character);
                changed = true;
            }
            if changed {
                self.search_index = 0;
                self.scene_dirty = true;
                self.request_draw(qh);
            }
        }
    }
}

impl Runtime {
    fn dispatch_ui_action(&mut self, action: UiAction) {
        let command = match action {
            UiAction::Applet(id) => {
                let x = self
                    .scene
                    .hit_targets()
                    .find_map(|(bounds, action)| {
                        (action == UiAction::Applet(id))
                            .then_some((bounds.x + bounds.width / 2.0) as i32)
                    })
                    .unwrap_or(0);
                self.tray_action(id, tray::Action::Activate, x, 18);
                return;
            }
            UiAction::ToggleQuickSettings => {
                if let Some(client) = &self.quick_settings_client
                    && let Err(error) = client
                        .command(knave_desktop_api::quick_settings::QuickSettingsCommand::Toggle)
                {
                    eprintln!("knave-shell: {error}");
                }
                return;
            }
            UiAction::ToggleNotifications => {
                if let Some(client) = &self.notification_client
                    && let Err(error) = client.command(
                        knave_desktop_api::notifications::NotificationCommand::ToggleCenter,
                    )
                {
                    eprintln!("knave-shell: {error}");
                }
                return;
            }
            UiAction::CloseOverview => {
                self.exit = true;
                return;
            }
            UiAction::OpenOverview => DesktopCommand::SetOverviewVisible {
                visible: !self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.overview_visible),
            },
            UiAction::FocusWorkspace(workspace) => DesktopCommand::FocusWorkspace { workspace },
            UiAction::FocusWindow(window) => DesktopCommand::FocusWindow { window },
            UiAction::RestoreWindow(window) => DesktopCommand::RestoreWindow { window },
        };
        match self
            .action_worker
            .as_ref()
            .expect("desktop role owns an action worker")
            .dispatch(command)
        {
            Ok(()) => {
                if self.role == ShellRole::Overview {
                    self.exit = true;
                }
            }
            Err(error) => eprintln!("knave-shell: shell action rejected: {error}"),
        }
    }

    fn tray_action(&self, id: u64, action: tray::Action, x: i32, y: i32) {
        if let Some(client) = &self.tray_client
            && let Err(error) = client.command(id, action, x, y)
        {
            eprintln!("knave-shell: {error}");
        }
    }
    fn handle_pointer(&mut self, x: f32, y: f32) {
        if let Some(action) = self.scene.hit_test(x, y) {
            self.dispatch_ui_action(action);
        }
    }
}

impl SeatHandler for Runtime {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self
            .active_seat
            .as_ref()
            .is_some_and(|active| *active != seat)
        {
            return;
        }
        self.active_seat = Some(seat.clone());
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Some(manager) = &self.ime.manager {
                self.ime.proxy = Some(manager.get_text_input(&seat, qh, ()));
            }
            if let Some(manager) = &self.clipboard.manager {
                self.clipboard.device = Some(manager.get_data_device(qh, &seat));
            }
            let keyboard = if self.app.is_some() {
                let repeat_qh = qh.clone();
                self.seat_state.get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(move |state, _, event| state.app_key(&repeat_qh, event, true, true)),
                )
            } else {
                self.seat_state.get_keyboard(qh, &seat, None)
            };
            match keyboard {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(error) => eprintln!("knave-shell: could not acquire shell keyboard: {error}"),
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            match self.seat_state.get_pointer_with_theme::<_, ()>(
                qh,
                &seat,
                self.shm.wl_shm(),
                surface.clone(),
                ThemeSpec::System,
            ) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(error) => {
                    surface.destroy();
                    eprintln!("knave-shell: could not acquire shell pointer: {error}");
                }
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if self.active_seat.as_ref() != Some(&seat) {
            return;
        }
        self.app_input(Input::Cancel);
        if capability == Capability::Keyboard {
            self.app_input(Input::FocusLost);
            self.cancel_clipboard();
            self.clipboard.clear_seat();
            self.ime.clear_seat();
            self.modifiers = KeyModifiers::default();
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if capability == Capability::Pointer {
            self.pointer = None; // ThemedPointer owns pointer, shape-device and cursor-surface cleanup.
            self.cursor = cursor::CursorState::default();
        }
        self.request_draw(qh);
    }

    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.active_seat.as_ref() == Some(&seat) {
            self.remove_capability(conn, qh, seat.clone(), Capability::Keyboard);
            self.remove_capability(conn, qh, seat, Capability::Pointer);
            self.active_seat = None;
        }
    }
}

impl KeyboardHandler for Runtime {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
        self.app_input(Input::FocusGained);
        self.sync_ime();
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        self.app_input(Input::FocusLost);
        self.request_draw(qh);
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.clipboard.serial = Some(serial);
        if self.app.is_some() {
            self.app_key(qh, event, true, false);
        } else {
            self.handle_key(qh, event);
        }
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self.app.is_some() {
            self.app_key(qh, event, true, true);
        }
    }
    fn release_key(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self.app.is_some() {
            self.app_key(qh, event, false, false);
        }
    }
    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw: RawModifiers,
        _layout: u32,
    ) {
        self.modifiers = KeyModifiers {
            shift: modifiers.shift,
            control: modifiers.ctrl,
            alt: modifiers.alt,
            logo: modifiers.logo,
        };
    }
}

impl PointerHandler for Runtime {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let surface = self.layer.wl_surface().clone();
        for event in events {
            if event.surface != surface {
                continue;
            }
            self.cursor.pointer_event(
                &event.kind,
                [event.position.0 as f32, event.position.1 as f32],
            );
            if let PointerEventKind::Press { serial, .. } = &event.kind {
                self.clipboard.serial = Some(*serial);
            }
            let p = [event.position.0 as f32, event.position.1 as f32];
            let applet = self
                .scene
                .hit_test(p[0], p[1])
                .and_then(|action| match action {
                    UiAction::Applet(id) => Some(id),
                    _ => None,
                });
            if let Some(id) = applet {
                let action = match &event.kind {
                    PointerEventKind::Press { button: 272, .. } => Some(tray::Action::Activate),
                    PointerEventKind::Press { button: 273, .. } => Some(tray::Action::ContextMenu),
                    PointerEventKind::Press { button: 274, .. } => {
                        Some(tray::Action::SecondaryActivate)
                    }
                    PointerEventKind::Axis { vertical, .. } if !vertical.is_none() => {
                        let delta = if vertical.value120 != 0 {
                            vertical.value120
                        } else if vertical.discrete != 0 {
                            vertical.discrete.saturating_mul(120)
                        } else {
                            vertical.absolute.round() as i32
                        };
                        (delta != 0).then_some(tray::Action::Scroll(delta))
                    }
                    _ => None,
                };
                if let Some(action) = action {
                    self.tray_action(id, action, p[0] as i32, p[1] as i32);
                    continue;
                }
                if matches!(event.kind, PointerEventKind::Release { .. }) {
                    continue;
                }
            }
            if self.app.is_some() {
                let input = match &event.kind {
                    PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                        Some(Input::PointerMove(p))
                    }
                    PointerEventKind::Leave { .. } => Some(Input::PointerLeave),
                    PointerEventKind::Press { button, .. } if *button == BTN_LEFT => {
                        Some(Input::PointerDown(p))
                    }
                    PointerEventKind::Press { button, .. } if *button == 0x111 => {
                        Some(Input::ContextMenu(p))
                    }
                    PointerEventKind::Release { button, .. } if *button == BTN_LEFT => {
                        Some(Input::PointerUp(p))
                    }
                    PointerEventKind::Axis {
                        horizontal,
                        vertical,
                        ..
                    } => Some(Input::Scroll {
                        position: p,
                        delta: [horizontal.absolute as f32, vertical.absolute as f32],
                    }),
                    _ => None,
                };
                if let Some(input) = input {
                    self.app_input(input);
                }
            } else if let PointerEventKind::Press { button, .. } = &event.kind
                && *button == BTN_LEFT
            {
                self.handle_pointer(p[0], p[1]);
            }
        }
        self.sync_cursor();
        if self.app.is_some() {
            self.sync_ime();
            self.application_requests(qh);
            self.request_draw(qh);
        }
    }
}

impl CompositorHandler for Runtime {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if self
            .pointer
            .as_ref()
            .is_some_and(|pointer| pointer.surface() == surface)
        {
            self.cursor.invalidate();
            self.sync_cursor();
            return;
        }
        if surface != self.layer.wl_surface() {
            return;
        }
        let old_scale = self.render_scale();
        self.integer_scale = new_factor.clamp(1, 8) as u32;
        if self.fractional_scaling() {
            self.layer.wl_surface().set_buffer_scale(1);
            return;
        }
        self.layer
            .wl_surface()
            .set_buffer_scale(self.integer_scale as i32);
        if self.configured && old_scale != self.render_scale() {
            if let Some(config) = self.configure_surface(self.width, self.height) {
                self.surface.configure(&self.device, &config);
            }
            self.scene_dirty = true;
            self.request_draw(qh);
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if self.surface_buffer_attached && _surface == self.layer.wl_surface() {
            self.notify_harness_ready();
        }
        self.frame_pending = false;
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Runtime {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Runtime {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let requested = self
            .options
            .as_ref()
            .and_then(|o| o.size)
            .map(|s| (s[0], s[1]))
            .unwrap_or_else(|| self.role.requested_size());
        self.width =
            NonZeroU32::new(configure.new_size.0).map_or(requested.0.max(1), NonZeroU32::get);
        self.height =
            NonZeroU32::new(configure.new_size.1).map_or(requested.1.max(1), NonZeroU32::get);

        let Some(config) = self.configure_surface(self.width, self.height) else {
            self.exit = true;
            return;
        };
        self.surface.configure(&self.device, &config);
        self.set_viewport_destination();
        if self.painter_format != Some(config.format) {
            match WgpuPainter::new(&self.device, &self.queue, config.format) {
                Ok(painter) => self.painter = Some(painter),
                Err(error) => {
                    eprintln!("knave-shell: cannot create renderer: {error}");
                    self.exit = true;
                    return;
                }
            }
            self.painter_format = Some(config.format);
        }
        self.scene_dirty = true;
        self.configured = true;
        self.draw(qh);
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Runtime {
    fn event(
        state: &mut Self,
        _proxy: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.preferred_scale_changed(scale, qh);
        }
    }
}

delegate_noop!(Runtime: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Runtime: ignore wp_viewporter::WpViewporter);
delegate_noop!(Runtime: ignore wp_viewport::WpViewport);

delegate_registry!(Runtime);

impl ProvidesRegistryState for Runtime {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(Runtime);

impl Runtime {
    fn app_input(&mut self, event: Input) {
        if let Some(app) = &mut self.app {
            app.input(event);
            if app.should_close() {
                self.exit = true;
            }
        }
    }
    fn app_key(&mut self, qh: &QueueHandle<Self>, event: KeyEvent, pressed: bool, repeat: bool) {
        self.app_input(Input::Key {
            key: application::key(event.keysym.raw()),
            pressed,
            repeat,
            modifiers: self.modifiers,
        });
        if let Some(app) = &mut self.app
            && app.wants_raw_keys()
        {
            let keysym =
                xkbcommon::xkb::keysym_get_name(xkbcommon::xkb::Keysym::new(event.keysym.raw()));
            app.raw_key(&keysym, pressed, repeat, self.modifiers);
        }
        if pressed
            && !self.modifiers.control
            && !self.modifiers.alt
            && !self.modifiers.logo
            && let Some(text) = event.utf8
            && text.chars().any(|c| !c.is_control())
        {
            self.app_input(Input::Text(text));
        }
        self.sync_ime();
        self.sync_cursor();
        self.application_requests(qh);
        self.request_draw(qh);
    }
}
fn fractional_scale_from_protocol(scale_120ths: u32) -> f64 {
    f64::from(scale_120ths.clamp(120, 480)) / 120.0
}

fn scaled_buffer_dimension(logical: u32, scale: f64) -> u32 {
    let scale = if scale.is_finite() {
        scale.clamp(1.0, 8.0)
    } else {
        1.0
    };
    (f64::from(logical.max(1)) * scale)
        .round()
        .clamp(1.0, f64::from(u32::MAX)) as u32
}

fn scale_list(list: &RenderList, scale: f32) -> RenderList {
    use knave_ui::{DisplayCommand, Transform2D};
    let mut result = list.clone();
    for command in &mut result.commands {
        let (transform, clip) = match command {
            DisplayCommand::Shape {
                transform, clip, ..
            }
            | DisplayCommand::Text {
                transform, clip, ..
            }
            | DisplayCommand::RichText {
                transform, clip, ..
            }
            | DisplayCommand::Image {
                transform, clip, ..
            } => (transform, clip),
            // Marker commands such as opacity scopes have no geometry to scale.
            _ => continue,
        };
        *transform = Transform2D::scale(scale, scale).compose(*transform);
        *clip = clip.map(|r| Transform2D::scale(scale, scale).transform_rect_bounds(r));
    }
    result
}

impl ShmHandler for Runtime {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_scale_uses_protocol_units_and_logical_destination_rounding() {
        assert_eq!(fractional_scale_from_protocol(150), 1.25);
        assert_eq!(fractional_scale_from_protocol(180), 1.5);
        assert_eq!(fractional_scale_from_protocol(0), 1.0);
        assert_eq!(fractional_scale_from_protocol(u32::MAX), 4.0);

        assert_eq!(scaled_buffer_dimension(101, 1.25), 126);
        assert_eq!(scaled_buffer_dimension(102, 1.25), 128);
        assert_eq!(scaled_buffer_dimension(101, 1.5), 152);
        assert_eq!(scaled_buffer_dimension(0, 1.5), 2);
        assert_eq!(scaled_buffer_dimension(u32::MAX, 8.0), u32::MAX);
    }

    #[test]
    fn roles_have_independent_layer_contracts() {
        assert_ne!(ShellRole::Bar.layer(), ShellRole::Overview.layer());
        assert_eq!(ShellRole::Bar.exclusive_zone(), 36);
        assert_eq!(ShellRole::Overview.exclusive_zone(), -1);
        assert_eq!(ShellRole::parse("bar"), Some(ShellRole::Bar));
        assert_eq!(ShellRole::parse("unknown"), None);
    }

    #[test]
    fn preview_decoder_accepts_bounded_rgba_png() {
        const PREVIEW_WIDTH: u32 = 1920;
        const PREVIEW_HEIGHT: u32 = 1080;
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, PREVIEW_WIDTH, PREVIEW_HEIGHT);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![17; (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize])
                .unwrap();
        }
        let preview = decode_preview(
            WorkspacePreview {
                workspace: WorkspaceId(1),
                width: PREVIEW_WIDTH,
                height: PREVIEW_HEIGHT,
                png_base64: base64::engine::general_purpose::STANDARD.encode(encoded),
            },
            [PREVIEW_WIDTH, PREVIEW_HEIGHT],
        )
        .unwrap();

        assert_eq!(preview.workspace, WorkspaceId(1));
        assert_eq!(preview.image.width(), PREVIEW_WIDTH);
        assert_eq!(preview.image.height(), PREVIEW_HEIGHT);
        assert_eq!(
            preview.image.pixels().len(),
            (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize
        );
    }

    #[test]
    fn preview_decoder_rejects_unrequested_dimensions() {
        assert!(
            decode_preview(
                WorkspacePreview {
                    workspace: WorkspaceId(1),
                    width: 64,
                    height: 36,
                    png_base64: String::new(),
                },
                [1920, 1080]
            )
            .is_none()
        );
    }
}
