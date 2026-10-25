//! One session process owns controls. Queries run off the compositor/shell frame threads.
mod system;
mod upower;
use knave_config::{
    ConfigDocument,
    quick_settings::{NightLightConfig, QuickSettingsConfig},
};
use knave_desktop_api::{DesktopCommand, DesktopRequest, DesktopResponse, quick_settings::*};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Notify, Semaphore},
};
use zbus::{Connection, fdo, interface, object_server::SignalEmitter};
struct Model {
    snapshot: QuickSettingsSnapshot,
    settings: QuickSettingsConfig,
}
struct Service {
    model: Arc<Mutex<Model>>,
    operation: Arc<Semaphore>,
    wake: Arc<Notify>,
}
fn failed(e: impl ToString) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}
fn minute() -> u16 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut out = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&now, out.as_mut_ptr()) }.is_null() {
        return 0;
    }
    let t = unsafe { out.assume_init() };
    (t.tm_hour * 60 + t.tm_min) as u16
}
async fn desktop(command: DesktopCommand) -> Result<(), String> {
    let work = async {
        let mut stream = tokio::net::UnixStream::connect(
            knave_desktop_api::socket_path().map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;
        let mut bytes =
            serde_json::to_vec(&DesktopRequest::Dispatch(command)).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        match serde_json::from_str::<DesktopResponse>(&line).map_err(|e| e.to_string())? {
            DesktopResponse::Ok => Ok(()),
            DesktopResponse::Error(e) => Err(e.message),
            _ => Err("unexpected compositor reply".into()),
        }
    };
    tokio::time::timeout(Duration::from_secs(3), work)
        .await
        .map_err(|_| "compositor request timed out".to_owned())?
}
impl Service {
    async fn changed(&self, c: &Connection) -> fdo::Result<()> {
        Self::changed_signal(&SignalEmitter::new(c, OBJECT_PATH).map_err(failed)?)
            .await
            .map_err(failed)
    }
    async fn refresh(&self) -> Result<(), String> {
        let (network, output, input, brightness, bluetooth, power) = tokio::join!(
            system::network(),
            system::volume(false),
            system::volume(true),
            system::brightness(),
            system::bluetooth(),
            system::power()
        );
        let battery = tokio::task::spawn_blocking(|| {
            system::battery_at(std::path::Path::new("/sys/class/power_supply"))
        })
        .await
        .map_err(|e| e.to_string())?;
        let (battery_charge, battery_error) = match battery {
            Ok(Some(mut battery)) => match upower::system_connection().await {
                Ok(connection) => match upower::query(&connection, &battery).await {
                    Ok(limit) => {
                        battery.charge_limit = Some(limit);
                        (Some(Some(battery)), None)
                    }
                    Err(error) => (Some(Some(battery)), Some(error)),
                },
                Err(error) => (Some(Some(battery)), Some(error)),
            },
            Ok(None) => (Some(None), None),
            Err(error) => (None, Some(error)),
        };
        let mut m = self.model.lock().await;
        m.snapshot.errors.clear();
        macro_rules! apply {
            ($field:ident,$result:expr) => {
                match $result {
                    Ok(v) => m.snapshot.$field = Some(v),
                    Err(e) => {
                        m.snapshot.$field = None;
                        m.snapshot.errors.push(e)
                    }
                }
            };
        }
        apply!(network, network);
        apply!(output, output);
        apply!(input, input);
        apply!(brightness, brightness);
        apply!(bluetooth, bluetooth);
        apply!(power, power);
        match battery_charge {
            Some(v) => m.snapshot.battery = v,
            None => m.snapshot.battery = None,
        }
        if let Some(error) = battery_error {
            m.snapshot.errors.push(format!("battery: {error}"));
        }
        Ok(())
    }
    async fn night(&self) -> Result<(), String> {
        let (settings, old) = {
            let m = self.model.lock().await;
            (m.settings.clone(), m.snapshot.night)
        };
        let n = &settings.night_light;
        let active = settings.night_active(minute());
        if active != old.active || n.temperature != old.temperature {
            desktop(DesktopCommand::SetNightLight {
                temperature: active.then_some(n.temperature),
            })
            .await?;
        }
        self.model.lock().await.snapshot.night = NightLight {
            enabled: n.enabled,
            active,
            temperature: n.temperature,
            start: n.start,
            end: n.end,
        };
        Ok(())
    }
    async fn save(&self, settings: QuickSettingsConfig) -> Result<(), String> {
        settings.validate().map_err(|e| e.to_string())?;
        let next = settings.clone();
        tokio::task::spawn_blocking(move || {
            let mut doc = ConfigDocument::at_default_path().map_err(|e| e.to_string())?;
            let mut config = doc.config().clone();
            config.quick_settings = next;
            doc.write(config).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        self.model.lock().await.settings = settings;
        Ok(())
    }
    async fn execute(&self, command: QuickSettingsCommand) -> Result<(), String> {
        use QuickSettingsCommand::*;
        match command {
            Toggle | Close | Refresh => {}
            SetWifi { enabled } => {
                system::run(
                    "nmcli",
                    &["radio", "wifi", if enabled { "on" } else { "off" }],
                )
                .await?;
            }
            Connect { uuid } => {
                let m = self.model.lock().await;
                let network = m
                    .snapshot
                    .network
                    .as_ref()
                    .ok_or("NetworkManager unavailable")?;
                if !network.connections.iter().any(|n| n.uuid == uuid) {
                    return Err("saved network no longer available".into());
                }
                drop(m);
                system::run("nmcli", &["--wait", "5", "connection", "up", "uuid", &uuid]).await?;
            }
            DisconnectEthernet => {
                let dev = self
                    .model
                    .lock()
                    .await
                    .snapshot
                    .network
                    .as_ref()
                    .and_then(|n| n.ethernet.clone())
                    .ok_or("Ethernet is not connected")?;
                system::run("nmcli", &["device", "disconnect", &dev]).await?;
            }
            SetVolume { input, percent } => {
                if percent > 100 {
                    return Err("volume must be 0–100%".into());
                }
                system::run(
                    "wpctl",
                    &[
                        "set-volume",
                        if input {
                            "@DEFAULT_AUDIO_SOURCE@"
                        } else {
                            "@DEFAULT_AUDIO_SINK@"
                        },
                        &format!("{percent}%"),
                    ],
                )
                .await?;
            }
            SetMute { input, muted } => {
                system::run(
                    "wpctl",
                    &[
                        "set-mute",
                        if input {
                            "@DEFAULT_AUDIO_SOURCE@"
                        } else {
                            "@DEFAULT_AUDIO_SINK@"
                        },
                        if muted { "1" } else { "0" },
                    ],
                )
                .await?;
            }
            OpenNetwork => {
                desktop(DesktopCommand::Spawn {
                    argv: vec!["nm-connection-editor".into()],
                })
                .await?;
                self.model.lock().await.snapshot.open = false;
            }
            OpenAudio => {
                desktop(DesktopCommand::Spawn {
                    argv: vec!["pavucontrol".into()],
                })
                .await?;
                self.model.lock().await.snapshot.open = false;
            }
            SetBrightness { percent } => {
                if !(1..=100).contains(&percent) {
                    return Err("brightness must be 1–100%".into());
                }
                system::run("brightnessctl", &["set", &format!("{percent}%")]).await?;
            }
            SetBluetooth { enabled } => {
                system::run(
                    "bluetoothctl",
                    &["power", if enabled { "on" } else { "off" }],
                )
                .await?;
            }
            SetPowerProfile { profile } => {
                if !self
                    .model
                    .lock()
                    .await
                    .snapshot
                    .power
                    .as_ref()
                    .is_some_and(|p| p.available.contains(&profile))
                {
                    return Err("power profile unavailable".into());
                }
                system::run("powerprofilesctl", &["set", &profile]).await?;
            }
            SetChargeLimit { enabled } => {
                let battery = self
                    .model
                    .lock()
                    .await
                    .snapshot
                    .battery
                    .clone()
                    .filter(|b| b.charge_limit.as_ref().is_some_and(|l| l.supported))
                    .ok_or("UPower battery charge limits are unavailable")?;
                let connection = upower::system_connection_for_action().await?;
                let charge_limit = upower::set_enabled(&connection, &battery, enabled).await?;
                if let Some(current) = self.model.lock().await.snapshot.battery.as_mut()
                    && current.name == battery.name
                {
                    current.charge_limit = Some(charge_limit);
                }
            }
            SetNightLight {
                enabled,
                temperature,
                start,
                end,
            } => {
                let mut settings = self.model.lock().await.settings.clone();
                settings.night_light = NightLightConfig {
                    enabled,
                    temperature,
                    start,
                    end,
                };
                settings.validate().map_err(|e| e.to_string())?;
                let active = settings.night_active(minute());
                desktop(DesktopCommand::SetNightLight {
                    temperature: active.then_some(temperature),
                })
                .await?;
                if let Err(error) = self.save(settings).await {
                    let old = self.model.lock().await.snapshot.night;
                    let _ = desktop(DesktopCommand::SetNightLight {
                        temperature: old.active.then_some(old.temperature),
                    })
                    .await;
                    return Err(error);
                }
                self.night().await?;
            }
            Suspend | Reboot | PowerOff => {
                let verb = match command {
                    Suspend => "suspend",
                    Reboot => "reboot",
                    _ => "poweroff",
                };
                let connection = zbus::connection::Builder::system()
                    .map_err(|e| e.to_string())?
                    .method_timeout(Duration::from_secs(8))
                    .build()
                    .await
                    .map_err(|e| e.to_string())?;
                let proxy = zbus::Proxy::new(
                    &connection,
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                )
                .await
                .map_err(|e| e.to_string())?;
                let method = match verb {
                    "suspend" => "Suspend",
                    "reboot" => "Reboot",
                    _ => "PowerOff",
                };
                proxy
                    .call::<_, _, ()>(method, &(true,))
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
#[interface(name = "org.knave.QuickSettings")]
impl Service {
    async fn snapshot(&self) -> fdo::Result<String> {
        serde_json::to_string(&self.model.lock().await.snapshot).map_err(failed)
    }
    async fn command(&self, json: &str, #[zbus(connection)] c: &Connection) -> fdo::Result<()> {
        if json.len() > 4096 {
            return Err(fdo::Error::InvalidArgs("command exceeds budget".into()));
        }
        let command: QuickSettingsCommand =
            serde_json::from_str(json).map_err(|e| fdo::Error::InvalidArgs(e.to_string()))?;
        if matches!(command, QuickSettingsCommand::Close) {
            self.model.lock().await.snapshot.open = false;
            self.wake.notify_one();
            return self.changed(c).await;
        }
        let _permit = self
            .operation
            .try_acquire()
            .map_err(|_| failed("A system action is still in progress"))?;
        if matches!(command, QuickSettingsCommand::Toggle) {
            let mut m = self.model.lock().await;
            m.snapshot.open = !m.snapshot.open;
            drop(m);
            self.changed(c).await?;
        }
        let result = self.execute(command).await;
        if self.model.lock().await.snapshot.open {
            self.refresh().await.map_err(failed)?;
        }
        self.changed(c).await?;
        self.wake.notify_one();
        result.map_err(failed)
    }
    #[zbus(signal, name = "Changed")]
    async fn changed_signal(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
pub fn run() -> Result<(), String> {
    let doc = ConfigDocument::at_default_path().map_err(|e| e.to_string())?;
    let config = doc.config();
    let settings = config.quick_settings.clone();
    let snapshot = QuickSettingsSnapshot {
        light: config.appearance.theme == knave_config::Theme::Light,
        high_contrast: config.portal.high_contrast,
        ..Default::default()
    };
    let service = Service {
        model: Arc::new(Mutex::new(Model { snapshot, settings })),
        operation: Arc::new(Semaphore::new(1)),
        wake: Arc::new(Notify::new()),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move{
    let connection=zbus::connection::Builder::session().map_err(|e|e.to_string())?.serve_at(OBJECT_PATH,service).map_err(|e|e.to_string())?.build().await.map_err(|e|e.to_string())?;
    connection.request_name_with_flags(BUS_NAME,zbus::fdo::RequestNameFlags::DoNotQueue.into()).await.map_err(|e|e.to_string())?;
    let interface=connection.object_server().interface::<_,Service>(OBJECT_PATH).await.map_err(|e|e.to_string())?;
    let s=interface.get().await;
    let mut term=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e|e.to_string())?;
    let initial={let m=s.model.lock().await; m.settings.night_active(minute()).then_some(m.settings.night_light.temperature)};
    if let Err(e)=desktop(DesktopCommand::SetNightLight{temperature:initial}).await{s.model.lock().await.snapshot.errors=vec![e]}
    if let Err(e)=s.night().await{s.model.lock().await.snapshot.errors=vec![e]}
    loop {
        let m=s.model.lock().await;let open=m.snapshot.open;let scheduled=m.settings.night_light.enabled&&m.settings.night_light.start.is_some();drop(m);
        // No hidden hardware polling. An open panel samples external CLI/sysfs changes.
        let delay=if open{Some(Duration::from_secs(3))}else if scheduled{Some(Duration::from_secs(60))}else{None};
        tokio::select!{_=term.recv()=>break,_=tokio::signal::ctrl_c()=>break,_=s.wake.notified()=>{},_=async{if let Some(d)=delay{tokio::time::sleep(d).await}else{std::future::pending::<()>().await}}=>{
            let work=async {
             if let Ok(_permit)=s.operation.try_acquire(){
                if open && let Err(e)=s.refresh().await{s.model.lock().await.snapshot.errors=vec![e]}
                let initial={let m=s.model.lock().await; m.settings.night_active(minute()).then_some(m.settings.night_light.temperature)};
    if let Err(e)=desktop(DesktopCommand::SetNightLight{temperature:initial}).await{s.model.lock().await.snapshot.errors=vec![e]}
    if let Err(e)=s.night().await{s.model.lock().await.snapshot.errors=vec![e]}
                let _=s.changed(&connection).await;
             }
            };
            tokio::select!{_=term.recv()=>break,_=work=>{}}
        }}
    }Ok(())
 })
}
