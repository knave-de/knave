//! A transient dbusmenu presentation process, owned and reaped by the tray host.
use crate::{Application, KeyboardMode, SurfaceLayer, SurfaceOptions, Waker};
use futures_util::StreamExt;
use knave_ui::{Color, DisplayList, Rect, ShapePaint, toolkit::*};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use zbus::{
    Proxy,
    zvariant::{OwnedValue, Value},
};

const MENU_LOAD_TIMEOUT: Duration = Duration::from_millis(900);
const MAX_CACHED_ROWS: usize = 128;
const MAX_CACHED_JSON_BYTES: usize = 64 * 1024;
pub(super) type WireLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct MenuRow {
    pub(crate) id: i32,
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) separator: bool,
    pub(crate) submenu: bool,
}
type Row = MenuRow;
#[derive(Clone, Default)]
struct Page {
    rows: Vec<Row>,
    parent: Option<i32>,
    error: Option<String>,
    loading: bool,
    close: bool,
}
enum Job {
    Open(i32),
    Click(i32),
}
struct Client {
    jobs: mpsc::Sender<Job>,
    update: Arc<Mutex<Option<Page>>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Client {
    fn start(
        owner: String,
        item_path: String,
        cached_menu_path: String,
        cached_rows: Vec<Row>,
        x: i32,
        y: i32,
        waker: Waker,
    ) -> Self {
        let (jobs, mut input) = mpsc::channel(4);
        let update = Arc::new(Mutex::new(None));
        let output = update.clone();
        let (stop, mut stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let publish = |page| {
                *output.lock().unwrap() = Some(page);
                waker.wake();
            };
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let result = match result {
                Err(error) => Err(error.to_string()),
                Ok(runtime) => runtime.block_on(async {
                    let work = async {
                        let conn = zbus::connection::Builder::session()?.method_timeout(Duration::from_secs(2)).build().await?;
                        let (item, menu_path) = if cached_menu_path != "/" {
                            let path = zbus::zvariant::OwnedObjectPath::try_from(cached_menu_path.as_str())
                                .map_err(|error| zbus::Error::Failure(error.to_string()))?;
                            (None, path)
                        } else {
                            match tokio::time::timeout(MENU_LOAD_TIMEOUT, item_menu(&conn, &owner, &item_path)).await {
                                Ok(Ok((item, path))) => (Some(item), path),
                                Ok(Err(error)) => return Err(error),
                                Err(_) => return Err(zbus::Error::Failure("timed out discovering the application menu".into())),
                            }
                        };
                        if menu_path.as_str() == "/" {
                            let item = item.expect("uncached menu path has an item proxy");
                            match tokio::time::timeout(MENU_LOAD_TIMEOUT, item.call::<_, _, ()>("ContextMenu", &(x, y))).await {
                                Ok(result) => result?,
                                Err(_) => return Err(zbus::Error::Failure("timed out opening the application menu".into())),
                            }
                            publish(Page { close: true, ..Default::default() });
                            return Ok::<_, zbus::Error>(());
                        }
                        let proxy = Proxy::new(&conn, owner.as_str(), menu_path.as_str(), "com.canonical.dbusmenu").await?;
                        let mut changes = proxy.receive_all_signals().await?;
                        let mut owners = proxy.receive_owner_changed().await?;
                        let mut stack = vec![0];
                        let mut dirty = true;
                        let mut cached_pages = HashMap::new();
                        if !cached_rows.is_empty() {
                            cached_pages.insert(0, cached_rows);
                        }
                        loop {
                            if dirty {
                                let parent = *stack.last().unwrap();
                                let fetched = tokio::time::timeout(
                                    MENU_LOAD_TIMEOUT,
                                    load_layout(&proxy, parent),
                                )
                                .await;
                                let parent_page = (stack.len() > 1).then_some(stack[stack.len().saturating_sub(2)]);
                                let fetched = match fetched {
                                    Ok(result) => result.map_err(|error| error.to_string()),
                                    Err(_) => Err("Timed out waiting for the application menu".into()),
                                };
                                let page = update_cached_page(&mut cached_pages, parent, parent_page, fetched);
                                publish(page);
                                dirty = false;
                            }
                            tokio::select! {
                                job = input.recv() => match job {
                                    Some(Job::Open(id)) => {
                                        if stack.len() > 1 && stack[stack.len()-2] == id { stack.pop(); }
                                        else if stack.len() < 16 { stack.push(id); }
                                        dirty = true;
                                    },
                                    Some(Job::Click(id)) => {
                                        proxy.call::<_,_,()>("Event", &(id, "clicked", Value::from(0_i32), 0_u32)).await?;
                                        publish(Page { close: true, ..Default::default() });
                                        return Ok::<_,zbus::Error>(());
                                    },
                                    None => return Ok(()),
                                },
                                signal = changes.next() => {
                                    if signal.is_none() { return Err(zbus::Error::Failure("menu disconnected".into())); }
                                    // Coalesce bursts; there is no timer while the menu is idle.
                                    tokio::time::sleep(Duration::from_millis(40)).await;
                                    dirty = true;
                                },
                                owner = owners.next() => if owner.is_none_or(|o| o.is_none()) { return Err(zbus::Error::Failure("application closed".into())); },
                            }
                        }
                    };
                    tokio::select! { _ = &mut stopped => Ok(()), result = work => result.map_err(|e| e.to_string()) }
                }),
            };
            if let Err(error) = result {
                publish(Page {
                    error: Some(error),
                    ..Default::default()
                });
            }
        });
        Self {
            jobs,
            update,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn update_cached_page(
    cache: &mut HashMap<i32, Vec<Row>>,
    parent_id: i32,
    parent: Option<i32>,
    fetched: Result<Vec<Row>, String>,
) -> Page {
    match fetched {
        Ok(rows) => {
            cache.insert(parent_id, rows.clone());
            Page {
                rows,
                parent,
                ..Default::default()
            }
        }
        Err(error) => Page {
            rows: cache.get(&parent_id).cloned().unwrap_or_default(),
            parent,
            error: Some(error),
            ..Default::default()
        },
    }
}
fn initial_page(rows: Vec<Row>) -> Page {
    Page {
        error: (!rows.is_empty()).then(|| "Showing last known menu".into()),
        loading: rows.is_empty(),
        rows,
        ..Default::default()
    }
}
fn decode_cached_rows(json: &str) -> Result<Vec<Row>, String> {
    if json.len() > MAX_CACHED_JSON_BYTES {
        return Err("cached tray menu exceeds size limit".into());
    }
    let mut rows: Vec<Row> =
        serde_json::from_str(json).map_err(|error| format!("invalid cached tray menu: {error}"))?;
    if rows.len() > MAX_CACHED_ROWS {
        return Err("cached tray menu exceeds row limit".into());
    }
    for row in &mut rows {
        row.label = row
            .label
            .chars()
            .filter(|character| !character.is_control())
            .take(160)
            .collect();
    }
    Ok(rows)
}
async fn load_layout(proxy: &Proxy<'_>, parent: i32) -> zbus::Result<Vec<Row>> {
    // Menus may populate their children only after AboutToShow.
    let _ = tokio::time::timeout(
        Duration::from_millis(100),
        proxy.call::<_, _, bool>("AboutToShow", &(parent,)),
    )
    .await;
    let (_, layout): (u32, WireLayout) = proxy
        .call("GetLayout", &(parent, 1_i32, Vec::<&str>::new()))
        .await?;
    rows(layout)
}
async fn item_menu<'a>(
    conn: &'a zbus::Connection,
    owner: &'a str,
    path: &'a str,
) -> zbus::Result<(Proxy<'a>, zbus::zvariant::OwnedObjectPath)> {
    let kde = Proxy::new(conn, owner, path, "org.kde.StatusNotifierItem").await?;
    if let Ok(menu) = kde
        .get_property::<zbus::zvariant::OwnedObjectPath>("Menu")
        .await
    {
        return Ok((kde, menu));
    }
    drop(kde);
    let fdo = Proxy::new(conn, owner, path, "org.freedesktop.StatusNotifierItem").await?;
    let menu = fdo
        .get_property::<zbus::zvariant::OwnedObjectPath>("Menu")
        .await?;
    Ok((fdo, menu))
}
fn text(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or("")
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect()
}
pub(super) fn rows(layout: WireLayout) -> zbus::Result<Vec<MenuRow>> {
    layout
        .2
        .into_iter()
        .take(128)
        .filter_map(|value| {
            let result = (|| {
                let (id, props, children): WireLayout = value.try_into()?;
                if props.get("visible").and_then(|v| bool::try_from(v).ok()) == Some(false) {
                    return Ok(None);
                }
                let toggle = text(&props, "toggle-type");
                let checked = props
                    .get("toggle-state")
                    .and_then(|v| i32::try_from(v).ok())
                    == Some(1);
                let label = text(&props, "label")
                    .replace("__", "\0")
                    .replace('_', "")
                    .replace('\0', "_");
                let label = if toggle.is_empty() {
                    label
                } else {
                    format!("{}  {label}", if checked { "✓" } else { "○" })
                };
                Ok(Some(Row {
                    id,
                    label,
                    enabled: props
                        .get("enabled")
                        .and_then(|v| bool::try_from(v).ok())
                        .unwrap_or(true),
                    separator: text(&props, "type") == "separator",
                    submenu: text(&props, "children-display") == "submenu" || !children.is_empty(),
                }))
            })();
            match result {
                Ok(Some(row)) => Some(Ok(row)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        })
        .collect()
}
struct Menu {
    owner: String,
    item_path: String,
    menu_path: String,
    x: f32,
    y: f32,
    client: Option<Client>,
    page: Page,
    scene: Scene,
    display: DisplayList,
    dirty: bool,
    close: bool,
    size: [f32; 2],
    panel: Rect,
}
impl Menu {
    fn new(
        owner: String,
        item_path: String,
        menu_path: String,
        initial_rows: Vec<Row>,
        x: f32,
        y: f32,
        theme: Theme,
    ) -> Self {
        let mut scene = Scene::new(Element::new(0, Widget::Panel)).unwrap();
        scene
            .set_desktop_theme(theme, knave_shell_ui::styles::STYLESHEET)
            .expect("shell stylesheet is valid");
        Self {
            owner,
            item_path,
            menu_path,
            x,
            y,
            client: None,
            page: initial_page(initial_rows),
            scene,
            display: DisplayList::default(),
            dirty: true,
            close: false,
            size: [0.0; 2],
            panel: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }
    fn send(&mut self, job: Job) {
        if self
            .client
            .as_ref()
            .is_none_or(|c| c.jobs.try_send(job).is_err())
        {
            self.page.error = Some("Menu busy; try again".into());
            self.dirty = true;
        }
    }
}
impl Application for Menu {
    fn theme_policy_changed(
        &mut self,
        policy: ThemePolicy,
        css: &str,
    ) -> Result<(), StylesheetError> {
        self.scene
            .set_theme_policy_stylesheets(policy, knave_shell_ui::styles::STYLESHEET, css)?;
        self.dirty = true;
        Ok(())
    }
    fn set_waker(&mut self, waker: Waker) {
        self.client = Some(Client::start(
            self.owner.clone(),
            self.item_path.clone(),
            self.menu_path.clone(),
            self.page.rows.clone(),
            self.x as i32,
            self.y as i32,
            waker,
        ));
    }
    fn update(&mut self) {
        if let Some(page) = self
            .client
            .as_ref()
            .and_then(|c| c.update.lock().unwrap().take())
        {
            self.close |= page.close;
            self.page = page;
            self.dirty = true;
        }
    }
    fn input(&mut self, event: Input) {
        if matches!(
            event,
            Input::FocusLost
                | Input::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                }
        ) {
            self.close = true;
            return;
        }
        if let Input::PointerDown(p) = event
            && !(p[0] >= self.panel.x
                && p[0] < self.panel.x + self.panel.width
                && p[1] >= self.panel.y
                && p[1] < self.panel.y + self.panel.height)
        {
            self.close = true;
            return;
        }
        if let Some(Action::Activated(id)) = self.scene.event(event).action {
            if id.0 == 3 {
                self.close = true;
            } else if id.0 == 4 {
                if let Some(parent) = self.page.parent {
                    self.send(Job::Open(parent));
                }
            } else if let Some(row) = self.page.rows.get(id.0.saturating_sub(100) as usize) {
                self.send(if row.submenu {
                    Job::Open(row.id)
                } else {
                    Job::Click(row.id)
                });
            }
        }
    }
    fn needs_frame(&self) -> bool {
        self.dirty || self.scene.needs_frame()
    }
    fn should_close(&self) -> bool {
        self.close
    }
    fn frame(&mut self, size: [f32; 2], text: &mut dyn TextMeasurer) -> &DisplayList {
        if self.dirty || size != self.size {
            self.size = size;
            let width = 280.0_f32.min(size[0]);
            let top = (self.y.max(knave_shell_ui::status_bar::STATUS_BAR_HEIGHT) + 8.0)
                .clamp(0.0, (size[1] - 96.0).max(0.0));
            let status_height =
                if self.page.error.is_some() || self.page.loading || self.page.rows.is_empty() {
                    60.0
                } else {
                    0.0
                };
            let height = (self
                .page
                .rows
                .iter()
                .map(|row| if row.separator { 10.0 } else { 32.0 })
                .sum::<f32>()
                + 64.0
                + status_height)
                .max(96.0)
                .min((size[1] - top).max(1.0));
            self.panel = Rect::new(
                (self.x - width / 2.0).clamp(0.0, (size[0] - width).max(0.0)),
                top,
                width,
                height,
            );
            let mut root = Element::new(0, Widget::Panel).layout(Layout {
                flow: Flow::Overlay,
                height: Length::Fill,
                gap: 0.0,
                ..Default::default()
            });
            root = root.without_surface();
            root.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
            let mut panel = Element::new(1, Widget::Panel).layout(Layout {
                width: Length::Px(width),
                height: Length::Px(height),
                offset: [self.panel.x, self.panel.y],
                padding: 8.0,
                gap: 4.0,
                ..Default::default()
            });
            panel.component = Some(ComponentKind::Card);
            panel
                .classes
                .extend(["shell-card".into(), "tray-menu-panel".into()]);
            let mut header = Element::new(2, Widget::Panel).layout(Layout {
                flow: Flow::Row,
                height: Length::Px(28.0),
                ..Default::default()
            });
            header = header.without_surface();
            header.style.background = Some(ShapePaint::fill(Color::rgba(0, 0, 0, 0)));
            header.layout.padding = 0.0;
            if self.page.parent.is_some() {
                header
                    .children
                    .push(Element::new(4, Widget::Button("Back".into())));
            }
            header
                .children
                .push(Element::new(3, Widget::Button("Close".into())));
            panel.children.push(header);
            let mut list = Element::new(5, Widget::Scroll).layout(Layout {
                height: Length::Fill,
                gap: 0.0,
                ..Default::default()
            });
            if let Some(error) = &self.page.error {
                let text = if error == "Showing last known menu" {
                    error.clone()
                } else {
                    format!("Menu unavailable: {error}")
                };
                let mut status = Element::new(6, Widget::Text(text)).class("tray-menu-status");
                status.style.text.wrap = knave_ui::TextWrap::WordOrGlyph;
                list.children.push(status);
            } else if self.page.loading {
                list.children
                    .push(Element::new(6, Widget::Text("Loading menu…".into())));
            } else if self.page.rows.is_empty() {
                list.children
                    .push(Element::new(6, Widget::Text("No menu actions".into())));
            }
            for (index, row) in self.page.rows.iter().enumerate() {
                let label = if row.submenu {
                    format!("{}  ›", row.label)
                } else {
                    row.label.clone()
                };
                let mut item = Element::new(
                    100 + index as u64,
                    if row.separator {
                        Widget::Text("────────────────".into())
                    } else {
                        Widget::Button(label)
                    },
                )
                .layout(Layout {
                    height: Length::Px(if row.separator { 10.0 } else { 32.0 }),
                    ..Default::default()
                });
                item.classes.push("tray-menu-action".into());
                item.enabled = row.enabled && !row.separator;
                list.children.push(item);
            }
            panel.children.push(list);
            root.children.push(panel);
            if let Err(error) = self.scene.replace(knave_shell_ui::styles::root(
                root.class("shell-overlay"),
                self.scene.theme(),
            )) {
                eprintln!("knave-shell: tray menu layout: {error}");
                self.close = true;
            }
            self.scene.layout(size, text);
            self.dirty = false;
        }
        self.display = self.scene.display_list(size, text).clone();
        self.display.clear_color = Color::rgba(0, 0, 0, 0);
        &self.display
    }
}
/// Private shell role; endpoints are passed by the tray worker, never interpreted as commands.
pub fn run(
    owner: String,
    item_path: String,
    menu_path: String,
    cached_rows_json: String,
    x: f32,
    y: f32,
    theme: Theme,
) -> Result<(), String> {
    zbus::names::BusName::try_from(owner.as_str()).map_err(|e| e.to_string())?;
    zbus::zvariant::ObjectPath::try_from(item_path.as_str()).map_err(|e| e.to_string())?;
    zbus::zvariant::ObjectPath::try_from(menu_path.as_str()).map_err(|e| e.to_string())?;
    let cached_rows = decode_cached_rows(&cached_rows_json)?;
    if !x.is_finite() || !y.is_finite() {
        return Err("invalid menu position".into());
    }
    crate::run_application_with_theme(
        SurfaceOptions {
            keyboard: KeyboardMode::Exclusive,
            layer: SurfaceLayer::Overlay,
            namespace: "knave-shell-tray-menu".into(),
            ..Default::default()
        },
        Menu::new(owner, item_path, menu_path, cached_rows, x, y, theme),
        theme,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicBool, Ordering},
    };

    struct Metrics;
    impl TextMeasurer for Metrics {
        fn measure(&mut self, text: &str, style: &knave_ui::TextStyle, width: f32) -> TextLayout {
            let glyph = style.font_size * 0.6;
            let columns = (width / glyph).floor().max(1.0) as usize;
            TextLayout {
                carets: Vec::new(),
                size: [
                    (text.chars().count() as f32 * glyph).min(width),
                    text.chars().count().max(1).div_ceil(columns) as f32 * style.line_height,
                ],
            }
        }
        fn measure_spans(
            &mut self,
            spans: &[knave_ui::TextSpan],
            style: &knave_ui::TextStyle,
            width: f32,
        ) -> TextLayout {
            self.measure(
                &spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>(),
                style,
                width,
            )
        }
    }

    #[test]
    fn cached_timeout_menu_keeps_backdrop_transparent_and_action_visible() {
        for theme in [Theme::Dark, Theme::Light] {
            let mut menu = Menu::new(
                ":1.1".into(),
                "/item".into(),
                "/menu".into(),
                vec![Row {
                    id: 7,
                    label: "Cached action".into(),
                    enabled: true,
                    separator: false,
                    submenu: false,
                }],
                1169.0,
                18.0,
                theme,
            );
            menu.page.error = Some("Timed out waiting for the application menu".into());
            menu.frame([1280.0, 800.0], &mut Metrics);
            let backdrop = menu.scene.computed_style(ElementId(0)).unwrap();
            assert_eq!(backdrop.style.background.unwrap().fill.unwrap().alpha, 0);
            let list = menu.scene.bounds(ElementId(5)).unwrap();
            let action = menu.scene.bounds(ElementId(100)).unwrap();
            assert!(action.height >= 32.0);
            assert!(action.y >= list.y);
            assert!(
                action.y + action.height <= list.y + list.height,
                "cached action must remain visible below timeout text: {action:?} in {list:?}"
            );
            menu.input(Input::PointerDown([400.0, 400.0]));
            assert!(menu.should_close());
        }
    }

    struct PrivateBus(Child);
    impl PrivateBus {
        fn start() -> Result<(Self, String), String> {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|error| error.to_string())?;
            let mut address = String::new();
            BufReader::new(
                child
                    .stdout
                    .take()
                    .ok_or_else(|| "dbus-daemon stdout unavailable".to_owned())?,
            )
            .read_line(&mut address)
            .map_err(|error| error.to_string())?;
            let address = address.trim();
            if address.is_empty() {
                let _ = child.kill();
                let _ = child.wait();
                return Err("dbus-daemon did not provide an address".into());
            }
            Ok((Self(child), address.into()))
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct FixtureMenu {
        delay_about: Arc<AtomicBool>,
        delay_layout: Arc<AtomicBool>,
    }
    struct FixtureItem(Arc<AtomicBool>);
    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl FixtureItem {
        #[zbus(property)]
        async fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
            if self.0.load(Ordering::Relaxed) {
                std::future::pending::<()>().await;
            }
            "/menu".try_into().unwrap()
        }
    }
    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl FixtureMenu {
        async fn about_to_show(&self, _id: i32) -> bool {
            if self.delay_about.load(Ordering::Relaxed) {
                std::future::pending::<()>().await;
            }
            false
        }
        async fn get_layout(
            &self,
            id: i32,
            _depth: i32,
            _properties: Vec<String>,
        ) -> (u32, WireLayout) {
            if self.delay_layout.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            let mut properties: HashMap<String, OwnedValue> = HashMap::new();
            properties.insert(
                "label".into(),
                Value::from("Cached action").try_into().unwrap(),
            );
            let item = Value::from((7_i32, properties, Vec::<OwnedValue>::new()))
                .try_into()
                .unwrap();
            (1, (id, HashMap::new(), vec![item]))
        }
    }

    #[tokio::test]
    async fn delayed_about_to_show_keeps_cached_actions_and_bounds_initial_loading() {
        let (_bus, address) = PrivateBus::start().expect("private D-Bus fixture must start");
        let delayed_about = Arc::new(AtomicBool::new(false));
        let delayed_layout = Arc::new(AtomicBool::new(false));
        let delayed_property = Arc::new(AtomicBool::new(false));
        let server = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.knave.TrayMenuFixture")
            .unwrap()
            .serve_at(
                "/menu",
                FixtureMenu {
                    delay_about: delayed_about.clone(),
                    delay_layout: delayed_layout.clone(),
                },
            )
            .unwrap()
            .serve_at("/item", FixtureItem(delayed_property.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap();
        let proxy = Proxy::new(
            &client,
            "org.knave.TrayMenuFixture",
            "/menu",
            "com.canonical.dbusmenu",
        )
        .await
        .unwrap();
        let (_, menu_path) = item_menu(&client, "org.knave.TrayMenuFixture", "/item")
            .await
            .unwrap();
        assert_eq!(menu_path.as_str(), "/menu");
        delayed_property.store(true, Ordering::Relaxed);
        let property_read = tokio::time::timeout(
            Duration::from_millis(30),
            item_menu(&client, "org.knave.TrayMenuFixture", "/item"),
        )
        .await;
        assert!(
            property_read.is_err(),
            "fixture should hold Menu property open"
        );
        delayed_property.store(false, Ordering::Relaxed);
        let mut cached = HashMap::new();
        let initial = load_layout(&proxy, 0).await.unwrap();
        assert_eq!(initial[0].label, "Cached action");
        let page = update_cached_page(&mut cached, 0, None, Ok(initial));
        assert_eq!(page.rows[0].id, 7);

        delayed_about.store(true, Ordering::Relaxed);
        let start = tokio::time::Instant::now();
        let fetched =
            tokio::time::timeout(Duration::from_millis(500), load_layout(&proxy, 0)).await;
        assert!(
            fetched.is_ok(),
            "GetLayout should proceed when AboutToShow hangs"
        );
        assert_eq!(fetched.unwrap().unwrap()[0].label, "Cached action");
        assert!(start.elapsed() < Duration::from_millis(300));

        delayed_layout.store(true, Ordering::Relaxed);
        let fetched = tokio::time::timeout(Duration::from_millis(30), load_layout(&proxy, 0)).await;
        assert!(fetched.is_err(), "fixture should hold GetLayout open");
        let page = update_cached_page(
            &mut cached,
            0,
            None,
            Err("Timed out waiting for the application menu".into()),
        );
        assert_eq!(page.rows[0].label, "Cached action");
        assert_eq!(
            page.error.as_deref(),
            Some("Timed out waiting for the application menu")
        );

        let unavailable = update_cached_page(
            &mut HashMap::new(),
            0,
            None,
            Err("Timed out waiting for the application menu".into()),
        );
        assert!(unavailable.rows.is_empty());
        assert!(unavailable.error.is_some());
        drop(server);
    }

    #[test]
    fn unavailable_refresh_keeps_cached_rows_and_exposes_error() {
        let mut cached = HashMap::new();
        let first = update_cached_page(
            &mut cached,
            0,
            None,
            Ok(vec![Row {
                id: 7,
                label: "Cached action".into(),
                enabled: true,
                separator: false,
                submenu: false,
            }]),
        );
        assert_eq!(first.rows[0].id, 7);
        let encoded = serde_json::to_string(&first.rows).unwrap();
        let restored = initial_page(decode_cached_rows(&encoded).unwrap());
        assert_eq!(restored.rows[0].label, "Cached action");
        assert!(!restored.loading);
        assert_eq!(restored.error.as_deref(), Some("Showing last known menu"));
        let mut reopened_cache = HashMap::from([(0, restored.rows.clone())]);
        let page = update_cached_page(
            &mut reopened_cache,
            0,
            None,
            Err("Timed out waiting for the application menu".into()),
        );
        assert_eq!(page.rows[0].label, "Cached action");
        assert!(page.error.is_some());
    }

    fn child(id: i32, properties: HashMap<String, OwnedValue>) -> OwnedValue {
        Value::from((id, properties, Vec::<OwnedValue>::new()))
            .try_into()
            .unwrap()
    }
    #[test]
    fn menu_preserves_disabled_checked_and_nested_actions_and_hides_invisible_rows() {
        let mut checked = HashMap::new();
        checked.insert(
            "label".into(),
            Value::from("_Keep __open").try_into().unwrap(),
        );
        checked.insert(
            "toggle-type".into(),
            Value::from("checkmark").try_into().unwrap(),
        );
        checked.insert("toggle-state".into(), OwnedValue::from(1_i32));
        checked.insert("enabled".into(), OwnedValue::from(false));
        let mut hidden = HashMap::new();
        hidden.insert("visible".into(), OwnedValue::from(false));
        let mut nested = HashMap::new();
        nested.insert(
            "children-display".into(),
            Value::from("submenu").try_into().unwrap(),
        );
        let parsed = rows((
            0,
            HashMap::new(),
            vec![child(7, checked), child(8, hidden), child(9, nested)],
        ))
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, 7);
        assert_eq!(parsed[0].label, "✓  Keep _open");
        assert!(!parsed[0].enabled);
        assert!(parsed[1].submenu);
    }
}
