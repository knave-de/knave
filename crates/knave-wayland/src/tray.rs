//! Session-bus StatusNotifier watcher and host. One cancellable worker owns all
//! transport, icon decoding and action calls; the frame thread sees snapshots.
use crate::Waker;
use crate::tray_menu::MenuRow;
use futures_util::StreamExt;
use knave_icons::IconLookup;
use knave_shell_ui::status_bar::Applet;
use knave_ui::UiImage;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};
use zbus::{Connection, MessageStream, Proxy, message::Header};

const PATH: &str = "/StatusNotifierWatcher";
const KDE: &str = "org.kde.StatusNotifierWatcher";
const FDO: &str = "org.freedesktop.StatusNotifierWatcher";
const MAX_ITEMS: usize = 32;
const TIMEOUT: Duration = Duration::from_secs(2);
const ITEM_REFRESH_TIMEOUT: Duration = Duration::from_millis(700);
const MENU_CACHE_TIMEOUT: Duration = Duration::from_millis(350);
const MAX_MENU_CACHE_ARG_BYTES: usize = 64 * 1024;
type Update = Result<Vec<Applet>, String>;
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Activate,
    ContextMenu,
    SecondaryActivate,
    Scroll(i32),
}
struct Command {
    id: u64,
    action: Action,
    x: i32,
    y: i32,
}
struct AbortTask<T>(tokio::task::JoinHandle<T>);
type RefreshResult = zbus::Result<(Vec<Endpoint>, u64)>;
type RefreshTask = AbortTask<RefreshResult>;
impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Clone)]
struct Endpoint {
    id: u64,
    owner: String,
    path: String,
    menu_only: bool,
    registration: String,
    item_interface: String,
    menu_path: Option<String>,
    menu_rows: Option<Vec<MenuRow>>,
    applet: Option<Applet>,
}
#[derive(Default)]
struct Registry {
    items: Vec<Endpoint>,
    next_id: u64,
}
#[derive(Clone, Default)]
struct Watcher {
    registry: Arc<Mutex<Registry>>,
    changed: Arc<Notify>,
}

impl Watcher {
    async fn register(
        &self,
        service: &str,
        header: Header<'_>,
        conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("missing sender".into()))?;
        let (name, path) = if service.starts_with('/') {
            (sender.as_str(), service)
        } else {
            (service, "/StatusNotifierItem")
        };
        zbus::zvariant::ObjectPath::try_from(path)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let bus = zbus::fdo::DBusProxy::new(conn).await?;
        let name = zbus::names::BusName::try_from(name)
            .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        let owner = bus.get_name_owner(name.clone()).await?.to_string();
        let key = format!("{owner}{path}");
        {
            let mut registry = self.registry.lock().unwrap();
            if registry
                .items
                .iter()
                .any(|i| i.owner == owner && i.path == path)
            {
                return Ok(());
            }
            if registry.items.len() >= MAX_ITEMS {
                return Err(zbus::fdo::Error::LimitsExceeded(
                    "tray item limit reached".into(),
                ));
            }
            registry.next_id += 1;
            let id = registry.next_id;
            registry.items.push(Endpoint {
                id,
                owner,
                path: path.into(),
                menu_only: false,
                registration: name.to_string(),
                item_interface: "org.kde.StatusNotifierItem".into(),
                menu_path: None,
                menu_rows: None,
                applet: None,
            });
        }
        emit(conn, "StatusNotifierItemRegistered", &(key,)).await;
        self.changed.notify_one();
        Ok(())
    }
    fn items(&self) -> Vec<String> {
        self.registry
            .lock()
            .unwrap()
            .items
            .iter()
            .map(|i| format!("{}{}", i.owner, i.path))
            .collect()
    }
}

// Both wire dialects are used by real applications; they share one registry.
macro_rules! watcher_interface {
    ($ty:ident, $interface:literal) => {
        struct $ty(Watcher);
        #[zbus::interface(name = $interface)]
        impl $ty {
            async fn register_status_notifier_item(
                &self,
                service: &str,
                #[zbus(header)] header: Header<'_>,
                #[zbus(connection)] conn: &Connection,
            ) -> zbus::fdo::Result<()> {
                self.0.register(service, header, conn).await
            }
            async fn register_status_notifier_host(
                &self,
                service: &str,
                #[zbus(connection)] conn: &Connection,
            ) -> zbus::fdo::Result<()> {
                let name = zbus::names::BusName::try_from(service)
                    .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
                zbus::fdo::DBusProxy::new(conn)
                    .await?
                    .get_name_owner(name)
                    .await?;
                emit(conn, "StatusNotifierHostRegistered", &()).await;
                Ok(())
            }
            #[zbus(property)]
            fn registered_status_notifier_items(&self) -> Vec<String> {
                self.0.items()
            }
            #[zbus(property)]
            fn is_status_notifier_host_registered(&self) -> bool {
                true
            }
            #[zbus(property)]
            fn protocol_version(&self) -> i32 {
                0
            }
            #[allow(dead_code)]
            #[zbus(signal)]
            async fn status_notifier_item_registered(
                emitter: &zbus::object_server::SignalEmitter<'_>,
                service: &str,
            ) -> zbus::Result<()>;
            #[allow(dead_code)]
            #[zbus(signal)]
            async fn status_notifier_item_unregistered(
                emitter: &zbus::object_server::SignalEmitter<'_>,
                service: &str,
            ) -> zbus::Result<()>;
            #[allow(dead_code)]
            #[zbus(signal)]
            async fn status_notifier_host_registered(
                emitter: &zbus::object_server::SignalEmitter<'_>,
            ) -> zbus::Result<()>;
        }
    };
}
watcher_interface!(KdeWatcher, "org.kde.StatusNotifierWatcher");
watcher_interface!(FdoWatcher, "org.freedesktop.StatusNotifierWatcher");

async fn emit<B: serde::Serialize + zbus::zvariant::DynamicType>(
    conn: &Connection,
    member: &str,
    body: &B,
) {
    for interface in [KDE, FDO] {
        if let Err(error) = conn
            .emit_signal(None::<&str>, PATH, interface, member, body)
            .await
        {
            eprintln!("knave-shell: tray signal: {error}");
        }
    }
}

pub struct TrayClient {
    commands: mpsc::Sender<Command>,
    updates: Arc<Mutex<Option<Update>>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl TrayClient {
    pub fn start(waker: Waker) -> Self {
        let (commands, mut jobs) = mpsc::channel(16);
        let updates = Arc::new(Mutex::new(None));
        let output = updates.clone();
        let (stop, mut stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            let publish = |update| {
                *output.lock().unwrap() = Some(update);
                waker.wake();
            };
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    publish(Err(error.to_string()));
                    return;
                }
            };
            runtime.block_on(async {
                let mut backoff = Duration::from_millis(250);
                loop {
                    let result = tokio::select! {
                        _ = &mut stopped => break,
                        result = serve(&mut jobs, &publish) => result,
                    };
                    if let Err(error) = result { publish(Err(error.to_string())); }
                    tokio::select! { _ = &mut stopped => break, _ = tokio::time::sleep(backoff) => {} }
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            });
        });
        Self {
            commands,
            updates,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    pub fn latest(&self) -> Option<Update> {
        self.updates.lock().unwrap().take()
    }
    pub fn command(&self, id: u64, action: Action, x: i32, y: i32) -> Result<(), String> {
        self.commands
            .try_send(Command { id, action, x, y })
            .map_err(|e| format!("tray action queue unavailable: {e}"))
    }
}
impl Drop for TrayClient {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn serve(jobs: &mut mpsc::Receiver<Command>, publish: &impl Fn(Update)) -> zbus::Result<()> {
    let conn = zbus::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    // Subscribe before advertising the watcher, so no registration or exit is lost.
    let mut signals = futures_util::stream::SelectAll::new();
    for interface in [
        KDE,
        FDO,
        "org.kde.StatusNotifierItem",
        "org.freedesktop.StatusNotifierItem",
        "org.freedesktop.DBus.Properties",
        "org.freedesktop.DBus",
    ] {
        let mut rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(interface)?;
        if interface == "org.freedesktop.DBus" {
            rule = rule
                .sender("org.freedesktop.DBus")?
                .member("NameOwnerChanged")?;
        }
        signals.push(MessageStream::for_match_rule(rule.build(), &conn, Some(16)).await?);
    }
    let watcher = Watcher::default();
    conn.object_server()
        .at(PATH, KdeWatcher(watcher.clone()))
        .await?;
    conn.object_server()
        .at(PATH, FdoWatcher(watcher.clone()))
        .await?;
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    let own = match conn.request_name_with_flags(KDE, flags).await {
        Ok(_) => true,
        Err(zbus::Error::NameTaken) => false,
        Err(error) => return Err(error),
    };
    if own {
        match conn.request_name_with_flags(FDO, flags).await {
            Ok(_) | Err(zbus::Error::NameTaken) => {}
            Err(error) => return Err(error),
        }
    }
    let host_name = format!("org.kde.StatusNotifierHost.knave.p{}", std::process::id());
    conn.request_name_with_flags(host_name.as_str(), flags)
        .await?;
    let proxy = Proxy::new(&conn, KDE, PATH, KDE).await?;
    proxy
        .call::<_, _, ()>("RegisterStatusNotifierHost", &(host_name.as_str(),))
        .await?;
    let mut endpoints = Vec::new();
    let mut next_id = 0;
    let mut dirty = true;
    let mut refresh: Option<RefreshTask> = None;
    let mut actions = tokio::task::JoinSet::new();
    let mut menu: Option<tokio::process::Child> = None;
    loop {
        tokio::select! {
            _ = watcher.changed.notified() => dirty = true,
            _ = tokio::time::sleep(Duration::from_millis(40)), if dirty && refresh.is_none() => {
                dirty = false;
                refresh = Some(AbortTask(tokio::spawn(refresh_items(conn.clone(), endpoints.clone(), next_id))));
            }
            result = async {
                match refresh.as_mut() {
                    Some(task) => (&mut task.0).await,
                    None => std::future::pending().await,
                }
            }, if refresh.is_some() => {
                refresh = None;
                match result {
                    Ok(Ok((fresh, id))) => {
                        next_id = id;
                        endpoints = fresh;
                        publish(Ok(endpoints.iter().filter_map(|e| e.applet.clone()).collect()));
                    }
                    Ok(Err(error)) => {
                        eprintln!("knave-shell: tray refresh failed: {error}");
                        if dirty { continue; }
                    }
                    Err(error) => eprintln!("knave-shell: tray refresh task failed: {error}"),
                }
            }
            _ = actions.join_next(), if !actions.is_empty() => {}
            message = signals.next() => {
                let message = message.ok_or_else(|| zbus::Error::Failure("tray bus disconnected".into()))??;
                let header = message.header();
                let interface = header.interface().map(|i| i.as_str()).unwrap_or("");
                let member = header.member().map(|m| m.as_str()).unwrap_or("");
                if interface == "org.freedesktop.DBus" && member == "NameOwnerChanged" {
                    let (name, old, _new): (String, String, String) = message.body().deserialize()?;
                    if name == KDE && !old.is_empty() { return Err(zbus::Error::Failure("tray watcher changed; reconnecting".into())); }
                    if own && !old.is_empty() {
                        let removed = {
                            let mut registry = watcher.registry.lock().unwrap();
                            let removed: Vec<_> = registry.items.iter().filter(|i| i.owner == name || i.registration == name).map(|i| format!("{}{}", i.owner, i.path)).collect();
                            registry.items.retain(|i| i.owner != name && i.registration != name);
                            removed
                        };
                        for key in &removed { emit(&conn, "StatusNotifierItemUnregistered", &(key,)).await; }
                        dirty |= !removed.is_empty();
                    }
                } else if interface == KDE || interface == FDO
                    || ((interface == "org.kde.StatusNotifierItem" || interface == "org.freedesktop.StatusNotifierItem" || interface == "org.freedesktop.DBus.Properties")
                    && endpoints.iter().any(|e| header.sender().is_some_and(|s| s.as_str() == e.owner) && header.path().is_some_and(|p| p.as_str() == e.path))) {
                    dirty = true;
                }
            }
            _ = async { menu.as_mut().unwrap().wait().await }, if menu.is_some() => { menu = None; },
            job = jobs.recv() => {
                let Some(job) = job else { return Ok(()) };
                if let Some(endpoint) = endpoints.iter().find(|e| e.id == job.id) {
                    let opens_menu = matches!(job.action, Action::ContextMenu)
                        || (matches!(job.action, Action::Activate) && endpoint.menu_only);
                    if opens_menu {
                        if let Err(error) = activate(&conn, endpoint, job, &mut menu).await {
                            eprintln!("knave-shell: tray action failed: {error}");
                        }
                    } else if actions.len() < 16 {
                        let conn = conn.clone();
                        let endpoint = endpoint.clone();
                        actions.spawn(async move {
                            if let Err(error) = activate(&conn, &endpoint, job, &mut None).await {
                                eprintln!("knave-shell: tray action failed: {error}");
                            }
                        });
                    } else {
                        eprintln!("knave-shell: tray action queue is full");
                    }
                }
            }
        }
    }
}

async fn refresh_items(
    conn: Connection,
    old_endpoints: Vec<Endpoint>,
    mut next_id: u64,
) -> zbus::Result<(Vec<Endpoint>, u64)> {
    let fresh = Proxy::new(&conn, KDE, PATH, KDE).await?;
    let registered: Vec<String> = fresh.get_property("RegisteredStatusNotifierItems").await?;
    let old: HashMap<_, _> = old_endpoints
        .into_iter()
        .map(|endpoint| (endpoint.registration.clone(), endpoint))
        .collect();
    let bus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut lookup = IconLookup::from_process();
    let mut endpoints = Vec::new();
    for key in registered.into_iter().take(MAX_ITEMS) {
        let Some((name, tail)) = key.split_once('/') else {
            continue;
        };
        let path = format!("/{tail}");
        let previous = old.get(&key);
        let id = previous.map(|e| e.id).unwrap_or_else(|| {
            next_id += 1;
            next_id
        });
        let name = match zbus::names::BusName::try_from(name) {
            Ok(name) => name,
            Err(_) => continue,
        };
        let owner = match bus.get_name_owner(name).await {
            Ok(owner) => owner.to_string(),
            Err(_) => {
                if let Some(prior) = previous {
                    endpoints.push(prior.clone());
                }
                continue;
            }
        };
        let prior = previous.filter(|endpoint| endpoint.owner == owner);
        let mut endpoint = Endpoint {
            id,
            owner,
            path,
            menu_only: prior.is_some_and(|e| e.menu_only),
            registration: key.clone(),
            item_interface: prior
                .map(|e| e.item_interface.clone())
                .unwrap_or_else(|| "org.kde.StatusNotifierItem".into()),
            menu_path: prior.and_then(|e| e.menu_path.clone()),
            menu_rows: prior.and_then(|e| e.menu_rows.clone()),
            applet: prior.and_then(|e| e.applet.clone()),
        };
        match tokio::time::timeout(
            ITEM_REFRESH_TIMEOUT,
            read_item(&conn, &mut endpoint, &mut lookup),
        )
        .await
        {
            Ok(Ok(applet)) => endpoint.applet = applet,
            Ok(Err(error)) => eprintln!("knave-shell: tray item {key}: {error}"),
            Err(_) => eprintln!("knave-shell: tray item {key}: refresh timed out"),
        }
        if let Some(menu_path) = endpoint.menu_path.as_deref() {
            match tokio::time::timeout(
                MENU_CACHE_TIMEOUT,
                read_menu_rows(&conn, &endpoint, menu_path),
            )
            .await
            {
                Ok(Ok(rows)) => endpoint.menu_rows = Some(rows),
                Ok(Err(error)) => eprintln!("knave-shell: tray item {key} menu cache: {error}"),
                Err(_) => eprintln!("knave-shell: tray item {key} menu cache timed out"),
            }
        }
        endpoints.push(endpoint);
    }
    Ok((endpoints, next_id))
}

async fn read_menu_rows(
    conn: &Connection,
    endpoint: &Endpoint,
    menu_path: &str,
) -> zbus::Result<Vec<MenuRow>> {
    let proxy = Proxy::new(
        conn,
        endpoint.owner.as_str(),
        menu_path,
        "com.canonical.dbusmenu",
    )
    .await?;
    let (_, layout): (u32, crate::tray_menu::WireLayout) = proxy
        .call("GetLayout", &(0_i32, 1_i32, Vec::<&str>::new()))
        .await?;
    crate::tray_menu::rows(layout)
}

async fn item_proxy<'a>(conn: &'a Connection, endpoint: &'a Endpoint) -> zbus::Result<Proxy<'a>> {
    let proxy = Proxy::new(
        conn,
        endpoint.owner.as_str(),
        endpoint.path.as_str(),
        endpoint.item_interface.as_str(),
    )
    .await?;
    // The original freedesktop dialect is less common but has the same properties.
    if proxy.get_property::<String>("Status").await.is_ok() {
        return Ok(proxy);
    }
    Proxy::new(
        conn,
        endpoint.owner.as_str(),
        endpoint.path.as_str(),
        "org.freedesktop.StatusNotifierItem",
    )
    .await
}

async fn read_item(
    conn: &Connection,
    endpoint: &mut Endpoint,
    lookup: &mut IconLookup,
) -> zbus::Result<Option<Applet>> {
    let proxy = item_proxy(conn, endpoint).await?;
    let item_interface = proxy.interface().to_string();
    let cached_menu_path = endpoint.menu_path.clone();
    let menu_property = proxy
        .get_property::<zbus::zvariant::OwnedObjectPath>("Menu")
        .await
        .ok();
    let menu_path = match menu_property.as_ref() {
        Some(path) if path.as_str() != "/" => Some(path.to_string()),
        Some(_) => None,
        None => cached_menu_path.clone(),
    };
    let clear_menu_cache = menu_property.as_ref().is_some_and(|path| {
        path.as_str() == "/" || Some(path.as_str()) != cached_menu_path.as_deref()
    });
    let status: String = proxy.get_property("Status").await?;
    let menu_only = proxy
        .get_property::<bool>("ItemIsMenu")
        .await
        .unwrap_or(false);
    if status == "Passive" {
        drop(proxy);
        endpoint.item_interface = item_interface;
        endpoint.menu_path = menu_path;
        if clear_menu_cache {
            endpoint.menu_rows = None;
        }
        endpoint.menu_only = menu_only;
        return Ok(None);
    }
    let attention = matches!(status.as_str(), "NeedsAttention" | "RequestingAttention");
    let title: String = proxy
        .get_property("Title")
        .await
        .unwrap_or_else(|_| "Application".into());
    let prefix = if attention { "AttentionIcon" } else { "Icon" };
    let mut image = read_icon(&proxy, prefix, lookup).await;
    if image.is_none() && attention {
        image = read_icon(&proxy, "Icon", lookup).await;
    }
    drop(proxy);
    endpoint.item_interface = item_interface;
    endpoint.menu_path = menu_path;
    if clear_menu_cache {
        endpoint.menu_rows = None;
    }
    endpoint.menu_only = menu_only;
    Ok(Some(Applet {
        id: endpoint.id,
        title: title.chars().take(128).collect(),
        image,
        attention,
    }))
}
async fn read_icon(proxy: &Proxy<'_>, prefix: &str, lookup: &mut IconLookup) -> Option<UiImage> {
    if let Ok(name) = proxy.get_property::<String>(&format!("{prefix}Name")).await
        && let Ok(Some(icon)) = lookup.load(&name, 22)
    {
        return UiImage::from_rgba(icon.width, icon.height, icon.rgba);
    }
    let pixmaps = proxy
        .get_property::<Pixmaps>(&format!("{prefix}Pixmap"))
        .await
        .ok()?;
    decode_pixmap(pixmaps)
}
fn decode_pixmap(pixmaps: Pixmaps) -> Option<UiImage> {
    let (w, h, pixels) = pixmaps
        .into_iter()
        .filter(|(w, h, p)| {
            *w > 0 && *h > 0 && *w <= 512 && *h <= 512 && p.len() == *w as usize * *h as usize * 4
        })
        .min_by_key(|(w, h, _)| (w.abs_diff(22) + h.abs_diff(22), *w < 22))?;
    let rgba = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[1], p[2], p[3], p[0]])
        .collect();
    UiImage::from_rgba(w as u32, h as u32, rgba)
}
async fn activate(
    conn: &Connection,
    endpoint: &Endpoint,
    command: Command,
    menu: &mut Option<tokio::process::Child>,
) -> zbus::Result<()> {
    let method = match command.action {
        Action::Activate if endpoint.menu_only => "ContextMenu",
        Action::Activate => "Activate",
        Action::ContextMenu => "ContextMenu",
        Action::SecondaryActivate => "SecondaryActivate",
        Action::Scroll(delta) => {
            let proxy = item_proxy(conn, endpoint).await?;
            return proxy.call("Scroll", &(delta, "vertical")).await;
        }
    };
    if method == "ContextMenu" {
        // Replace and reap the previous popup; no detached children survive the host.
        *menu = None;
        let cached_rows = serialize_menu_rows(endpoint.menu_rows.as_deref().unwrap_or(&[]))?;
        let child = tokio::process::Command::new(
            std::env::current_exe().map_err(|e| zbus::Error::Failure(e.to_string()))?,
        )
        .args([
            "tray-menu",
            &endpoint.owner,
            &endpoint.path,
            endpoint.menu_path.as_deref().unwrap_or("/"),
            &cached_rows,
            &command.x.to_string(),
            &command.y.to_string(),
        ])
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        *menu = Some(child);
        return Ok(());
    }
    let proxy = item_proxy(conn, endpoint).await?;
    proxy.call(method, &(command.x, command.y)).await
}

fn serialize_menu_rows(rows: &[MenuRow]) -> zbus::Result<String> {
    let mut rows = rows.to_vec();
    loop {
        let encoded = serde_json::to_string(&rows)
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        if encoded.len() <= MAX_MENU_CACHE_ARG_BYTES {
            return Ok(encoded);
        }
        if rows.is_empty() {
            return Err(zbus::Error::Failure(
                "could not fit cached tray menu within argument limit".into(),
            ));
        }
        rows.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixmaps_validate_size_and_convert_network_argb() {
        assert!(
            decode_pixmap(vec![(513, 1, vec![]), (-1, 1, vec![]), (1, 1, vec![0; 3])]).is_none()
        );
        let image = decode_pixmap(vec![(1, 1, vec![128, 20, 40, 60])]).unwrap();
        assert_eq!(image.pixels(), &[20, 40, 60, 128]);
    }
}
