//! One cancellable D-Bus subscriber and bounded command queue per presentation host.
use crate::Waker;
use futures_util::{FutureExt, StreamExt};
use knave_desktop_api::notifications::*;
use std::{
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub struct NotificationClient {
    commands: mpsc::Sender<NotificationCommand>,
    updates: Arc<Mutex<Option<Result<NotificationSnapshot, String>>>>,
    errors: Arc<Mutex<Option<String>>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl NotificationClient {
    pub fn start(waker: Waker) -> Self {
        Self::start_inner(waker, false)
    }
    pub fn start_summary(waker: Waker) -> Self {
        Self::start_inner(waker, true)
    }
    fn start_inner(waker: Waker, summary: bool) -> Self {
        let (commands, mut jobs) = mpsc::channel::<NotificationCommand>(16);
        let updates = Arc::new(Mutex::new(None));
        let output = updates.clone();
        let errors = Arc::new(Mutex::new(None));
        let failures = errors.clone();
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
                        let connection=zbus::connection::Builder::session()?.method_timeout(Duration::from_secs(2)).build().await?;
                        let proxy=zbus::Proxy::new(&connection,BUS_NAME,OBJECT_PATH,INTERFACE).await?;
                        let mut changes=proxy.receive_signal("Changed").await?;
                        let mut owners=proxy.receive_owner_changed().await?;
                        async fn snapshot(proxy:&zbus::Proxy<'_>, summary:bool)->Result<NotificationSnapshot,String> {
                            let json:String=proxy.call(if summary { "Summary" } else { "Snapshot" },&()).await.map_err(|e|e.to_string())?;
                            if json.len()>MAX_SNAPSHOT_BYTES {return Err("notification snapshot exceeds budget".into())}
                            serde_json::from_str(&json).map_err(|e|e.to_string())
                        }
                        let initial=snapshot(&proxy, summary).await.map_err(zbus::Error::Failure)?;
                        *output.lock().unwrap()=Some(Ok(initial));waker.wake();backoff=Duration::from_millis(250);
                        loop {
                            tokio::select!{
                                signal=changes.next()=>{
                                    if signal.is_none(){return Err(zbus::Error::Failure("notification service disconnected".into()))}
                                    while matches!(changes.next().now_or_never(), Some(Some(_))) {}
                                    let state=snapshot(&proxy, summary).await.map_err(zbus::Error::Failure)?;
                                    *output.lock().unwrap()=Some(Ok(state));waker.wake();
                                },
                                owner=owners.next()=>{if owner.is_none_or(|o|o.is_none()){return Err(zbus::Error::Failure("notification service stopped".into()))}},
                                job=jobs.recv()=>{
                                    let Some(command)=job else {return Ok(())};
                                    match proxy.call::<_,_,()>("Command",&(serde_json::to_string(&command).unwrap(),)).await {
                                        Ok(())=>{},Err(e)=>{*failures.lock().unwrap()=Some(e.to_string());waker.wake();}
                                    }
                                }
                            }
                        }
                    };
                    let result:Result<(),zbus::Error>=tokio::select!{_=&mut stopped=>break,result=run=>result};
                    if let Err(error)=result {*output.lock().unwrap()=Some(Err(error.to_string()));waker.wake();}
                    tokio::select!{_=&mut stopped=>break,_=tokio::time::sleep(backoff)=>{}}
                    backoff=(backoff*2).min(Duration::from_secs(5));
                }
            });
        });
        Self {
            commands,
            updates,
            errors,
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    pub fn command(&self, command: NotificationCommand) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|e| format!("notification action queue unavailable: {e}"))
    }
    pub fn latest(&self) -> Option<Result<NotificationSnapshot, String>> {
        self.updates.lock().unwrap().take()
    }
    pub fn error(&self) -> Option<String> {
        self.errors.lock().unwrap().take()
    }
    pub fn has_update(&self) -> bool {
        self.updates.lock().unwrap().is_some() || self.errors.lock().unwrap().is_some()
    }
}
impl Drop for NotificationClient {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
