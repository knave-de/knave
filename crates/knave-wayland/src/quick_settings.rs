//! One cancellable D-Bus subscriber and bounded command queue per presentation host.
use crate::Waker;
use futures_util::{FutureExt, StreamExt};
use knave_desktop_api::quick_settings::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub struct QuickSettingsClient {
    authentication_pending: Arc<AtomicBool>,
    commands: mpsc::Sender<QuickSettingsCommand>,
    updates: Arc<Mutex<Option<Result<QuickSettingsSnapshot, String>>>>,
    errors: Arc<Mutex<Option<String>>>,
    charge_result: Arc<Mutex<Option<Result<u8, String>>>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl QuickSettingsClient {
    pub fn start(waker: Waker) -> Self {
        Self::start_inner(waker, false)
    }
    pub fn start_summary(waker: Waker) -> Self {
        Self::start_inner(waker, true)
    }
    fn start_inner(waker: Waker, summary: bool) -> Self {
        let authentication_pending = Arc::new(AtomicBool::new(false));
        let pending = authentication_pending.clone();
        let (commands, mut jobs) = mpsc::channel::<QuickSettingsCommand>(16);
        let updates = Arc::new(Mutex::new(None));
        let output = updates.clone();
        let errors = Arc::new(Mutex::new(None));
        let failures = errors.clone();
        let charge_result = Arc::new(Mutex::new(None));
        let completed_charge = charge_result.clone();
        let (stop, mut stopped) = oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    *output.lock().unwrap() = Some(Err(e.to_string()));
                    waker.wake();
                    return;
                }
            };
            runtime.block_on(async move {
                let mut backoff=Duration::from_millis(250);
                loop {
                    let run=async {
                        let connection=zbus::connection::Builder::session()?.method_timeout(Duration::from_secs(210)).build().await?;
                        let proxy=zbus::Proxy::new(&connection,BUS_NAME,OBJECT_PATH,INTERFACE).await?;
                        let mut changes=proxy.receive_signal("Changed").await?;
                        let mut owners=proxy.receive_owner_changed().await?;
                        async fn snapshot(proxy:&zbus::Proxy<'_>, _summary:bool)->Result<QuickSettingsSnapshot,String> {
                            let json:String=proxy.call("Snapshot",&()).await.map_err(|e|e.to_string())?;
                            if json.len()>MAX_SNAPSHOT_BYTES {return Err("quick-settings snapshot exceeds budget".into())}
                            serde_json::from_str(&json).map_err(|e|e.to_string())
                        }
                        let initial=snapshot(&proxy, summary).await.map_err(zbus::Error::Failure)?;
                        *output.lock().unwrap()=Some(Ok(initial));waker.wake();backoff=Duration::from_millis(250);
                        loop {
                            tokio::select!{
                                signal=changes.next()=>{
                                    if signal.is_none(){return Err(zbus::Error::Failure("quick-settings service disconnected".into()))}
                                    while matches!(changes.next().now_or_never(), Some(Some(_))) {}
                                    let state=snapshot(&proxy, summary).await.map_err(zbus::Error::Failure)?;
                                    *output.lock().unwrap()=Some(Ok(state));waker.wake();
                                },
                                owner=owners.next()=>{if owner.is_none_or(|o|o.is_none()){return Err(zbus::Error::Failure("quick-settings service stopped".into()))}},
                                job=jobs.recv()=>{
                                    let Some(command)=job else {return Ok(())};
                                    let charge=matches!(command,QuickSettingsCommand::SetChargeLimit{..}|QuickSettingsCommand::Suspend|QuickSettingsCommand::Reboot|QuickSettingsCommand::PowerOff);
                                    let requested = match &command { QuickSettingsCommand::SetChargeLimit { percent } => Some(*percent), _ => None };
                                    let result = proxy.call::<_,_,()>("Command",&(serde_json::to_string(&command).unwrap(),)).await.map_err(|e|match e { zbus::Error::MethodError(_,Some(detail),_) => detail, e => e.to_string() });
                                    if let Some(percent) = requested {
                                        let verified = match result {
                                            Ok(()) => match snapshot(&proxy,summary).await {
                                                Ok(state) => {
                                                    let actual = state.battery.as_ref().and_then(|b|b.driver_limit);
                                                    let preset_enabled = state.battery.as_ref().and_then(|b|b.charge_limit.as_ref()).is_some_and(|limit| limit.supported && limit.enabled);
                                                    *output.lock().unwrap() = Some(Ok(state));
                                                    if preset_enabled { Err("UPower's preset is still active; the custom charge limit was not confirmed".into()) } else if actual == Some(percent) { Ok(percent) } else { Err(format!("Charge limit was not accepted: requested {percent}%, driver reports {}", actual.map_or("unavailable".into(), |value| format!("{value}%")))) }
                                                }
                                                Err(e) => Err(format!("Could not verify charge limit: {e}")),
                                            },
                                            Err(e) => Err(e),
                                        };
                                        *completed_charge.lock().unwrap() = Some(verified);
                                        waker.wake();
                                    } else if let Err(e) = result {
                                        *failures.lock().unwrap()=Some(e);waker.wake();
                                    }
                                    if charge{pending.store(false,Ordering::SeqCst);waker.wake();}
                                }
                            }
                        }
                    };
                    let result:Result<(),zbus::Error>=tokio::select!{_=&mut stopped=>break,result=run=>result};
                    if let Err(error)=result {
                        while jobs.try_recv().is_ok(){}
                        pending.store(false,Ordering::SeqCst);
                        *output.lock().unwrap()=Some(Err(error.to_string()));waker.wake();
                    }
                    tokio::select!{_=&mut stopped=>break,_=tokio::time::sleep(backoff)=>{}}
                    backoff=(backoff*2).min(Duration::from_secs(5));
                }
            });
        });
        Self {
            authentication_pending,
            commands,
            updates,
            errors,
            charge_result,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    pub fn authentication_pending(&self) -> bool {
        self.authentication_pending.load(Ordering::SeqCst)
    }
    pub fn command(&self, command: QuickSettingsCommand) -> Result<(), String> {
        let charge = matches!(
            command,
            QuickSettingsCommand::SetChargeLimit { .. }
                | QuickSettingsCommand::Suspend
                | QuickSettingsCommand::Reboot
                | QuickSettingsCommand::PowerOff
        );
        if charge && self.authentication_pending.swap(true, Ordering::SeqCst) {
            return Err("An authenticated action is already in progress".into());
        }
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(e) => {
                if charge {
                    self.authentication_pending.store(false, Ordering::SeqCst)
                }
                Err(format!("quick-settings action queue unavailable: {e}"))
            }
        }
    }
    pub fn latest(&self) -> Option<Result<QuickSettingsSnapshot, String>> {
        self.updates.lock().unwrap().take()
    }
    pub fn take_charge_result(&self) -> Option<Result<u8, String>> {
        self.charge_result.lock().unwrap().take()
    }
    pub fn error(&self) -> Option<String> {
        self.errors.lock().unwrap().take()
    }
    pub fn has_update(&self) -> bool {
        self.updates.lock().unwrap().is_some()
            || self.errors.lock().unwrap().is_some()
            || self.charge_result.lock().unwrap().is_some()
    }
}
impl Drop for QuickSettingsClient {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
