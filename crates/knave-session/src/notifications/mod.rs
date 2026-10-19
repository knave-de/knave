//! One session-owned notification service. Name acquisition is strictly first-owner-wins.
mod model;
mod store;
#[cfg(test)]
mod tests;
use futures_util::StreamExt;
use knave_config::Config;
use knave_desktop_api::notifications::*;
use model::{Incoming, Model, Signal, bounded};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Notify, oneshot};
use zbus::{
    Connection, fdo, interface, message::Header, object_server::SignalEmitter, zvariant::OwnedValue,
};

struct Engine {
    model: Model,
    store: store::Store,
}
impl Engine {
    fn save(&mut self) -> Result<(), String> {
        self.store.save(
            &self.model.durable(),
            self.model.snapshot.dnd,
            self.model.next,
        )
    }
    fn change<T>(
        &mut self,
        f: impl FnOnce(&mut Model) -> Result<(T, Vec<Signal>), String>,
    ) -> Result<(T, Vec<Signal>), String> {
        let before = self.model.clone();
        match f(&mut self.model).and_then(|result| {
            self.model.snapshot.error = None;
            if self.model.durable() != before.durable()
                || self.model.snapshot.dnd != before.snapshot.dnd
                || self.model.next != before.next
            {
                self.save()?;
            }
            Ok(result)
        }) {
            Ok(result) => Ok(result),
            Err(e) => {
                self.model = before;
                Err(e)
            }
        }
    }
}
#[derive(Clone)]
struct Service {
    engine: Arc<Mutex<Engine>>,
    wake: Arc<Notify>,
}
fn failure(e: impl ToString) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}
fn sender(header: &Header<'_>) -> fdo::Result<String> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| failure("notification caller has no unique bus identity"))
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
async fn emit(connection: &Connection, signals: Vec<Signal>) -> zbus::Result<()> {
    for signal in signals {
        match signal {
            Signal::Closed(id, reason, destination) => {
                connection
                    .emit_signal(
                        Some(destination.as_str()),
                        STANDARD_PATH,
                        STANDARD_NAME,
                        "NotificationClosed",
                        &(id, reason),
                    )
                    .await?
            }
            Signal::Action(id, action, destination) => {
                connection
                    .emit_signal(
                        Some(destination.as_str()),
                        STANDARD_PATH,
                        STANDARD_NAME,
                        "ActionInvoked",
                        &(id, action),
                    )
                    .await?
            }
        }
    }
    let emitter = SignalEmitter::new(connection, OBJECT_PATH)?;
    Control::changed(&emitter).await
}
#[interface(name = "org.freedesktop.Notifications")]
impl Service {
    fn get_capabilities(&self) -> Vec<&str> {
        vec!["actions", "body", "icon-static"]
    }
    fn get_server_information(&self) -> (&str, &str, &str, &str) {
        (
            "Knave Notifications",
            "Knave",
            env!("CARGO_PKG_VERSION"),
            "1.3",
        )
    }
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<u32> {
        if app_name.len() > 4096
            || app_icon.len() > 4096
            || summary.len() > 16384
            || body.len() > 65536
            || actions.len() > MAX_ACTIONS * 2
            || !actions.len().is_multiple_of(2)
            || actions.iter().any(|s| s.len() > 4096)
            || actions
                .iter()
                .step_by(2)
                .any(|s| s.len() > 128 || s.chars().any(char::is_control))
            || hints.len() > 64
        {
            return Err(fdo::Error::InvalidArgs(
                "notification exceeds text/action/hint limits".into(),
            ));
        }
        let hint_bool = |key: &str| {
            hints
                .get(key)
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false)
        };
        let app_id = hints
            .get("desktop-entry")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or(app_name);
        let urgency = hints
            .get("urgency")
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(1)
            .min(2);
        let progress = hints
            .get("value")
            .and_then(|v| i32::try_from(v).ok())
            .map(|v| v.clamp(0, 100) as u8);
        let incoming = Incoming {
            sender: sender(&header)?,
            replaces: replaces_id,
            app: bounded(app_name, 128),
            icon: bounded(app_icon, 512),
            summary: bounded(summary, 512),
            body: bounded(body, 4096),
            app_id: bounded(app_id, 256),
            actions: actions
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| NotificationAction {
                    key: bounded(&pair[0], 128),
                    label: bounded(&pair[1], 256),
                })
                .collect(),
            urgency,
            progress,
            transient: hint_bool("transient"),
            resident: hint_bool("resident"),
            timeout: expire_timeout,
        };
        let mut keys = std::collections::HashSet::new();
        if incoming
            .actions
            .iter()
            .any(|a| a.key.is_empty() || !keys.insert(a.key.clone()))
        {
            return Err(fdo::Error::InvalidArgs(
                "action keys must be unique and nonempty".into(),
            ));
        }
        let mut engine = self.engine.lock().await;
        let (id, signals) = engine
            .change(|m| m.notify(incoming, now_ms()))
            .map_err(failure)?;
        emit(connection, signals).await.map_err(failure)?;
        self.wake.notify_one();
        Ok(id)
    }
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let mut engine = self.engine.lock().await;
        let (_, signals) = engine
            .change(|m| m.close(id, &caller).map(|s| ((), s)))
            .map_err(failure)?;
        emit(connection, signals).await.map_err(failure)?;
        self.wake.notify_one();
        Ok(())
    }
}
#[derive(Clone)]
struct Control(Service);
#[interface(name = "org.knave.Notifications")]
impl Control {
    async fn snapshot(&self) -> fdo::Result<String> {
        serde_json::to_string(&self.0.engine.lock().await.model.snapshot).map_err(failure)
    }
    async fn command(
        &self,
        json: &str,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        if json.len() > 16384 {
            return Err(fdo::Error::InvalidArgs("command exceeds budget".into()));
        }
        let command =
            serde_json::from_str(json).map_err(|e| fdo::Error::InvalidArgs(format!("{e}")))?;
        let mut engine = self.0.engine.lock().await;
        let (_, signals) = engine
            .change(|m| m.command(command).map(|s| ((), s)))
            .map_err(failure)?;
        emit(connection, signals).await.map_err(failure)?;
        self.0.wake.notify_one();
        Ok(())
    }
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Dropping the guard closes the service, releases bus ownership and joins its sole runtime thread.
pub struct NotificationService {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl NotificationService {
    pub fn start(config: Config) -> Result<Self, String> {
        let (stop, receive) = oneshot::channel();
        let (ready, result) = std::sync::mpsc::sync_channel(1);
        let thread=std::thread::Builder::new().name("knave-notifications".into()).spawn(move||{
            let runtime=match tokio::runtime::Builder::new_current_thread().enable_all().build(){Ok(r)=>r,Err(e)=>{let _=ready.send(Err(e.to_string()));return}};
            runtime.block_on(async move {
                let setup=setup(config).await;
                match setup {
                    Ok((service,connection))=>{
                        let proxy = match zbus::fdo::DBusProxy::new(&connection).await { Ok(p)=>p,Err(e)=>{let _=ready.send(Err(e.to_string()));return} };
                        let mut owners = match proxy.receive_name_owner_changed().await { Ok(s)=>s,Err(e)=>{let _=ready.send(Err(e.to_string()));return} };
                        let _=ready.send(Ok(()));
                        tokio::pin!(receive);
                        loop {
                            let deadline=service.engine.lock().await.model.deadline();
                            let delay=deadline.map(|d|Duration::from_millis(d.saturating_sub(now_ms())));
                            tokio::select! {
                                _=&mut receive=>break,
                                owner=owners.next()=>{
                                    let Some(owner)=owner else {break};
                                    if let Ok(args)=owner.args() && args.new_owner().is_none() && args.name().as_str().starts_with(':') {
                                        let mut e=service.engine.lock().await;
                                        let changed=e.model.sender_gone(args.name().as_str());
                                        if changed {let _=emit(&connection,Vec::new()).await;}

                                    }
                                },
                                _=service.wake.notified()=>{},
                                _=async {if let Some(d)=delay {tokio::time::sleep(d).await}else{std::future::pending::<()>().await}}=>{
                                    let mut e=service.engine.lock().await;let before=e.model.snapshot.clone();
                                    let result=e.change(|m|Ok(((),m.tick(now_ms()))));
                                    match result {Ok(((),signals))=>{if before!=e.model.snapshot || !signals.is_empty(){let _=emit(&connection,signals).await;}},Err(error)=>{
                                        // Retry storage failures with backoff rather than spin on the elapsed deadline.
                                        e.model.snapshot.error=Some(format!("Could not save notification history: {error}"));
                                        let _=emit(&connection,Vec::new()).await;drop(e);
                                        tokio::select!{_=&mut receive=>break,_=tokio::time::sleep(Duration::from_secs(5))=>{}}
                                    }}
                                }
                            }
                        }
                    },Err(error)=>{let _=ready.send(Err(error));}
                }
            });
        }).map_err(|e|e.to_string())?;
        match result.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop: Some(stop),
                thread: Some(thread),
            }),
            result => {
                let _ = stop.send(());
                if thread.is_finished() {
                    let _ = thread.join();
                }
                Err(match result {
                    Ok(Err(e)) => e,
                    _ => "notification service startup timed out".into(),
                })
            }
        }
    }
}
impl Drop for NotificationService {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
async fn setup(config: Config) -> Result<(Service, Connection), String> {
    let connection = zbus::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(Duration::from_secs(2))
        .build()
        .await
        .map_err(|e| e.to_string())?;
    setup_on(config, connection, store::Store::path()?).await
}
async fn setup_on(
    config: Config,
    connection: Connection,
    path: std::path::PathBuf,
) -> Result<(Service, Connection), String> {
    // Do not queue or permit replacement. Losing this race never causes a retry or takeover.
    connection
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
        .map_err(|e| e.to_string())?;

    let store = store::Store::open(&path)?;
    let (entries, dnd, next) = store.load()?;
    let mut model = Model::new(config, entries, dnd, next);
    model.tick(now_ms());
    let mut engine = Engine { model, store };
    engine.save()?;
    let service = Service {
        engine: Arc::new(Mutex::new(engine)),
        wake: Arc::new(Notify::new()),
    };
    connection
        .object_server()
        .at(OBJECT_PATH, Control(service.clone()))
        .await
        .map_err(|e| e.to_string())?;
    connection
        .object_server()
        .at(STANDARD_PATH, service.clone())
        .await
        .map_err(|e| e.to_string())?;
    {
        let mut e = service.engine.lock().await;
        e.model.snapshot.owned = connection
            .request_name_with_flags(
                STANDARD_NAME,
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await
            .is_ok();
    }
    Ok((service, connection))
}

/// Standalone entry point for isolated protocol tests and manually managed sessions.
pub fn run_default() -> Result<(), String> {
    let config = knave_config::ConfigDocument::at_default_path()
        .map_err(|e| e.to_string())?
        .config()
        .clone();
    let _guard = NotificationService::start(config)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {let mut term=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e|e.to_string())?;
        tokio::select!{result=tokio::signal::ctrl_c()=>result.map_err(|e|e.to_string()),_=term.recv()=>Ok(())}})
}
