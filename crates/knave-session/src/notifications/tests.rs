use super::*;
fn model() -> Model {
    let mut m = Model::new(Config::default(), Vec::new(), false, 1);
    m.snapshot.owned = true;
    m
}
fn incoming() -> Incoming {
    Incoming {
        sender: ":1.10".into(),
        replaces: 0,
        app: "Test".into(),
        icon: "dialog-information".into(),
        summary: "Hello".into(),
        body: "Body".into(),
        app_id: "org.test.App".into(),
        actions: vec![NotificationAction {
            key: "default".into(),
            label: "Open".into(),
        }],
        urgency: 1,
        transient: false,
        resident: false,
        progress: None,
        timeout: -1,
    }
}
#[test]
fn timeout_archives_without_losing_unread_or_retaining_actions() {
    let mut m = model();
    m.notify(incoming(), 100).unwrap();
    let s = m.tick(6100);
    assert_eq!(s.len(), 1);
    let n = &m.snapshot.notifications[0];
    assert!(!n.live);
    assert!(!n.banner);
    assert!(n.unread);
    assert!(n.actions.is_empty());
    assert!(
        m.command(NotificationCommand::Invoke {
            id: 1,
            action: "default".into()
        })
        .is_err()
    );
}
#[test]
fn replacement_is_in_place_and_sender_scoped() {
    let mut m = model();
    m.notify(incoming(), 100).unwrap();
    let mut n = incoming();
    n.replaces = 1;
    n.summary = "Updated".into();
    assert_eq!(m.notify(n, 200).unwrap().0, 1);
    assert_eq!(m.snapshot.notifications.len(), 1);
    assert_eq!(m.snapshot.notifications[0].summary, "Updated");
    let mut n = incoming();
    n.replaces = 1;
    n.sender = ":1.11".into();
    assert!(m.notify(n, 300).is_err());
    assert!(m.close(1, ":1.11").is_err());
}
#[test]
fn dnd_collects_history_without_replay_and_critical_is_explicit() {
    let mut m = model();
    m.command(NotificationCommand::SetDnd { enabled: true })
        .unwrap();
    let mut n = incoming();
    n.urgency = 2;
    m.notify(n, 100).unwrap();
    assert!(!m.snapshot.notifications[0].banner);
    assert!(m.snapshot.notifications[0].live);
    m.command(NotificationCommand::SetDnd { enabled: false })
        .unwrap();
    assert!(!m.snapshot.notifications[0].banner);
    m.command(NotificationCommand::SetDnd { enabled: true })
        .unwrap();
    m.config.notifications.critical_in_dnd = true;
    let mut n = incoming();
    n.urgency = 2;
    m.notify(n, 200).unwrap();
    assert!(m.snapshot.notifications[0].banner);
}
#[test]
fn transient_resident_actions_and_close_reasons() {
    let mut m = model();
    let mut n = incoming();
    n.transient = true;
    m.notify(n, 0).unwrap();
    assert!(m.durable().is_empty());
    m.tick(6000);
    assert!(m.snapshot.notifications.is_empty());
    let mut n = incoming();
    n.resident = true;
    n.timeout = 0;
    m.notify(n, 7000).unwrap();
    assert!(matches!(
        &m.command(NotificationCommand::Invoke {
            id: 2,
            action: "default".into()
        })
        .unwrap()[0],
        Signal::Action(2, _, _)
    ));
    assert!(m.snapshot.notifications[0].live);
    let s = m.close(2, ":1.10").unwrap();
    assert!(matches!(&s[0], Signal::Closed(2, 3, _)));
    assert!(m.snapshot.notifications.is_empty());
}
#[test]
fn retention_live_limits_deadlines_and_overflow() {
    let mut m = model();
    m.config.notifications.history_limit = 2;
    for now in 0..10 {
        m.notify(incoming(), now).unwrap();
    }
    assert_eq!(m.snapshot.notifications.len(), 2);
    assert_eq!(m.live.len(), 2);
    assert_eq!(m.snapshot.unread(), 2);
    assert_eq!(m.deadline(), Some(6008));
    m.tick(7 * 86400000 + 10);
    assert!(m.snapshot.notifications.is_empty());
    m.next = u32::MAX as u64 + 1;
    assert!(m.notify(incoming(), 0).is_err());
    let mut m = model();
    for _ in 0..MAX_LIVE {
        let mut n = incoming();
        n.transient = true;
        n.timeout = 0;
        m.notify(n, 0).unwrap();
    }
    assert!(m.notify(incoming(), 0).is_err());
}
#[test]
fn seen_is_separate_from_dismiss_and_center_suppresses_banners() {
    let mut m = model();
    m.notify(incoming(), 0).unwrap();
    m.command(NotificationCommand::ToggleCenter).unwrap();
    assert_eq!(m.snapshot.unread(), 1);
    assert!(!m.snapshot.notifications[0].banner);
    m.notify(incoming(), 1).unwrap();
    assert!(!m.snapshot.notifications[0].banner);
    m.command(NotificationCommand::Seen { ids: vec![1] })
        .unwrap();
    assert_eq!(m.snapshot.unread(), 1);
    assert_eq!(m.snapshot.notifications.len(), 2);
    m.command(NotificationCommand::Clear).unwrap();
    assert!(m.snapshot.notifications.is_empty());
}
#[test]
fn store_restores_text_unread_and_dnd_but_not_live_actions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notifications.sqlite3");
    let mut s = store::Store::open(&path).unwrap();
    let mut m = model();
    m.notify(incoming(), 100).unwrap();
    s.save(&m.durable(), true, m.next).unwrap();
    drop(s);
    let s = store::Store::open(&path).unwrap();
    let (entries, dnd, next) = s.load().unwrap();
    assert!(dnd);
    assert_eq!(next, 2);
    assert!(entries[0].unread);
    assert!(!entries[0].live);
    assert!(!entries[0].banner);
    assert!(entries[0].actions.is_empty());
}
#[test]
fn unicode_bounds_and_control_characters() {
    assert_eq!(bounded("ééé", 5), "éé");
    assert_eq!(bounded("a\0b\u{1b}c\n", 100), "abc\n");
}

struct Bus {
    child: std::process::Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        Self::with_rule("")
    }
    fn with_rule(rule: &str) -> Self {
        use std::io::BufRead;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bus.conf");
        std::fs::write(&path,format!("<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><policy context=\"default\"><allow send_destination=\"*\"/><allow receive_sender=\"*\"/><allow own=\"*\"/>{rule}</policy></busconfig>")).unwrap();
        let mut child = std::process::Command::new("dbus-daemon")
            .arg(format!("--config-file={}", path.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("dbus-daemon test dependency");
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_secs(2))
            .build()
            .await
            .unwrap()
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn real_bus_first_owner_wins_without_queue_or_replacement() {
    let bus = Bus::new();
    let dir = tempfile::tempdir().unwrap();
    runtime().block_on(async {
        let external = bus.connect().await;
        external
            .request_name_with_flags(
                STANDARD_NAME,
                zbus::fdo::RequestNameFlags::AllowReplacement
                    | zbus::fdo::RequestNameFlags::DoNotQueue,
            )
            .await
            .unwrap();
        let (service, _connection) = setup_on(
            Config::default(),
            bus.connect().await,
            dir.path().join("history.sqlite3"),
        )
        .await
        .unwrap();
        assert!(!service.engine.lock().await.model.snapshot.owned);
        external.release_name(STANDARD_NAME).await.unwrap();
        let inspector = zbus::fdo::DBusProxy::new(&external).await.unwrap();
        assert!(
            !inspector
                .name_has_owner(STANDARD_NAME.try_into().unwrap())
                .await
                .unwrap(),
            "losing Knave must not be queued"
        );
        assert!(
            !service.engine.lock().await.model.snapshot.owned,
            "losing service must not reclaim"
        );
    });
}
#[test]
fn real_bus_protocol_actions_validation_and_sender_identity() {
    let bus = Bus::new();
    let dir = tempfile::tempdir().unwrap();
    runtime().block_on(async {
        let (service, server) = setup_on(
            Config::default(),
            bus.connect().await,
            dir.path().join("history.sqlite3"),
        )
        .await
        .unwrap();
        assert!(service.engine.lock().await.model.snapshot.owned);
        let client = bus.connect().await;
        let other = bus.connect().await;
        assert!(
            other
                .request_name_with_flags(
                    STANDARD_NAME,
                    zbus::fdo::RequestNameFlags::ReplaceExisting
                        | zbus::fdo::RequestNameFlags::DoNotQueue
                )
                .await
                .is_err()
        );
        let proxy = zbus::Proxy::new(&client, STANDARD_NAME, STANDARD_PATH, STANDARD_NAME)
            .await
            .unwrap();
        let capabilities: Vec<String> = proxy.call("GetCapabilities", &()).await.unwrap();
        assert!(capabilities.contains(&"actions".into()));
        assert!(!capabilities.contains(&"persistence".into()));
        let mut hints: HashMap<String, OwnedValue> = HashMap::new();
        hints.insert("urgency".into(), OwnedValue::from(2u8));
        hints.insert("value".into(), OwnedValue::from(150i32));
        hints.insert("unknown-hint".into(), OwnedValue::from(true));
        let actions = vec!["default".to_owned(), "Open".to_owned()];
        let id: u32 = proxy
            .call(
                "Notify",
                &(
                    "App",
                    0u32,
                    "dialog-information",
                    "Summary",
                    "Body",
                    actions.clone(),
                    hints,
                    0i32,
                ),
            )
            .await
            .unwrap();
        assert_eq!(id, 1);
        {
            let engine = service.engine.lock().await;
            assert_eq!(engine.model.snapshot.notifications[0].urgency, 2);
            assert_eq!(engine.model.snapshot.notifications[0].progress, Some(100));
        }
        let id: u32 = proxy
            .call(
                "Notify",
                &(
                    "App",
                    id,
                    "",
                    "Replacement",
                    "",
                    actions,
                    HashMap::<String, OwnedValue>::new(),
                    0i32,
                ),
            )
            .await
            .unwrap();
        assert_eq!(id, 1);
        let other_proxy = zbus::Proxy::new(&other, STANDARD_NAME, STANDARD_PATH, STANDARD_NAME)
            .await
            .unwrap();
        assert!(
            other_proxy
                .call::<_, _, u32>(
                    "Notify",
                    &(
                        "Other",
                        id,
                        "",
                        "Spoof",
                        "",
                        Vec::<String>::new(),
                        HashMap::<String, OwnedValue>::new(),
                        0i32
                    )
                )
                .await
                .is_err()
        );
        assert!(
            other_proxy
                .call::<_, _, ()>("CloseNotification", &(id,))
                .await
                .is_err()
        );
        assert!(
            proxy
                .call::<_, _, u32>(
                    "Notify",
                    &(
                        "App",
                        0u32,
                        "",
                        "Invalid",
                        "",
                        vec!["odd".to_owned()],
                        HashMap::<String, OwnedValue>::new(),
                        -1i32
                    )
                )
                .await
                .is_err()
        );
        let mut action_signals = proxy.receive_signal("ActionInvoked").await.unwrap();
        let mut closed_signals = proxy.receive_signal("NotificationClosed").await.unwrap();
        let control = zbus::Proxy::new(&client, BUS_NAME, OBJECT_PATH, INTERFACE)
            .await
            .unwrap();
        control
            .call::<_, _, ()>(
                "Command",
                &(serde_json::to_string(&NotificationCommand::Invoke {
                    id: 1,
                    action: "default".into(),
                })
                .unwrap(),),
            )
            .await
            .unwrap();
        let action = tokio::time::timeout(Duration::from_secs(2), action_signals.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            action.body().deserialize::<(u32, String)>().unwrap(),
            (1, "default".into())
        );
        let closed = tokio::time::timeout(Duration::from_secs(2), closed_signals.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(closed.body().deserialize::<(u32, u32)>().unwrap(), (1, 2));
        assert!(
            service
                .engine
                .lock()
                .await
                .model
                .snapshot
                .notifications
                .is_empty()
        );
        assert!(
            control
                .call::<_, _, ()>(
                    "Command",
                    &("{\"op\":\"invoke\",\"id\":1,\"action\":\"default\"}",)
                )
                .await
                .is_err()
        );
        drop(server);
    });
}
#[test]
fn sender_exit_disables_actions_without_hiding_fire_and_forget_banners() {
    let mut m = model();
    m.notify(incoming(), 0).unwrap();
    assert!(m.sender_gone(":1.10"));
    assert!(m.snapshot.notifications[0].banner);
    assert!(m.snapshot.notifications[0].actions.is_empty());
    assert!(
        m.command(NotificationCommand::Invoke {
            id: 1,
            action: "default".into()
        })
        .is_err()
    );
}
#[test]
fn transient_never_contributes_to_history_badge() {
    let mut m = model();
    let mut n = incoming();
    n.transient = true;
    m.notify(n, 0).unwrap();
    assert_eq!(m.snapshot.unread(), 0);
    assert_eq!(m.snapshot.history().count(), 0);
    assert!(m.snapshot.notifications[0].banner);
}
#[test]
fn database_lock_corruption_and_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/history.sqlite3");
    let s = store::Store::open(&path).unwrap();
    assert!(store::Store::open(&path).is_err());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    drop(s);
    std::fs::write(&path, "corrupt history").unwrap();
    assert!(store::Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"corrupt history");
}

#[test]
fn application_policies_suppress_banners_or_all_delivery_independently() {
    use knave_config::notifications::AppPolicy;
    let mut m = model();
    m.config.notifications.app.insert(
        "org.test.App".into(),
        AppPolicy {
            enabled: true,
            banners: false,
        },
    );
    m.notify(incoming(), 0).unwrap();
    assert_eq!(m.snapshot.unread(), 1);
    assert!(!m.snapshot.notifications[0].banner);
    m.config.notifications.app.insert(
        "org.test.App".into(),
        AppPolicy {
            enabled: false,
            banners: true,
        },
    );
    let (id, signals) = m.notify(incoming(), 1).unwrap();
    assert_eq!(id, 2);
    assert!(signals.iter().any(|s| matches!(s, Signal::Closed(2, 2, _))));
    assert!(!m.snapshot.notifications.iter().any(|n| n.id == 2));
}

#[test]
fn bus_policy_failure_is_unavailable_instead_of_fabricating_an_external_owner() {
    let bus = Bus::with_rule("<deny own=\"org.freedesktop.Notifications\"/>");
    let directory = tempfile::tempdir().unwrap();
    runtime().block_on(async {
        let result = setup_on(
            Config::default(),
            bus.connect().await,
            directory.path().join("history.sqlite3"),
        )
        .await;
        assert!(matches!(result,Err(error) if error.contains("name acquisition failed")));
    });
}
