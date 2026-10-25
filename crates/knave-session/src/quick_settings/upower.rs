use knave_desktop_api::quick_settings::{Battery, ChargeLimit};
use std::{future::Future, time::Duration};
use zbus::{Connection, zvariant::OwnedObjectPath};

const NAME: &str = "org.freedesktop.UPower";
const ROOT: &str = "/org/freedesktop/UPower";
const ROOT_IFACE: &str = "org.freedesktop.UPower";
const DEVICE_IFACE: &str = "org.freedesktop.UPower.Device";
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const ACTION_TIMEOUT: Duration = Duration::from_secs(90);

async fn enumerate(connection: &Connection) -> Result<Vec<OwnedObjectPath>, String> {
    let proxy = zbus::Proxy::new(connection, NAME, ROOT, ROOT_IFACE)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .call("EnumerateDevices", &())
        .await
        .map_err(|e| e.to_string())
}

async fn property<T>(
    connection: &Connection,
    path: &OwnedObjectPath,
    name: &str,
) -> Result<T, String>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    <T as TryFrom<zbus::zvariant::OwnedValue>>::Error: Into<zbus::Error>,
{
    let proxy = zbus::Proxy::new(connection, NAME, path, DEVICE_IFACE)
        .await
        .map_err(|e| e.to_string())?;
    proxy.get_property(name).await.map_err(|e| e.to_string())
}

async fn matching_device(
    connection: &Connection,
    native_name: &str,
) -> Result<Option<OwnedObjectPath>, String> {
    for path in enumerate(connection).await? {
        let native_path: String = property(connection, &path, "NativePath").await?;
        let kind: u32 = property(connection, &path, "Type").await?;
        let power_supply: bool = property(connection, &path, "PowerSupply").await?;
        let present: bool = property(connection, &path, "IsPresent").await?;
        let basename = std::path::Path::new(&native_path)
            .file_name()
            .and_then(|name| name.to_str());
        if basename == Some(native_name) && kind == 2 && power_supply && present {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn threshold(value: u32) -> Option<u8> {
    u8::try_from(value).ok().filter(|v| *v <= 100)
}

async fn settings(connection: &Connection, path: &OwnedObjectPath) -> Result<ChargeLimit, String> {
    let supported: bool = property(connection, path, "ChargeThresholdSupported").await?;
    let enabled: bool = property(connection, path, "ChargeThresholdEnabled").await?;
    if !supported {
        return Ok(ChargeLimit {
            supported: false,
            enabled,
            start_threshold: None,
            end_threshold: None,
        });
    }

    let mask = property::<u32>(connection, path, "ChargeThresholdSettingsSupported")
        .await
        .ok();
    let start = property::<u32>(connection, path, "ChargeStartThreshold")
        .await
        .ok()
        .and_then(threshold);
    let end = property::<u32>(connection, path, "ChargeEndThreshold")
        .await
        .ok()
        .and_then(threshold);
    let (start_threshold, end_threshold) = match mask {
        Some(bits) => (
            (bits & 1 != 0).then_some(start).flatten(),
            (bits & 2 != 0).then_some(end).flatten(),
        ),
        // Older UPower versions exposed threshold properties without a support mask.
        None => (start, end),
    };
    Ok(ChargeLimit {
        supported,
        enabled,
        start_threshold,
        end_threshold,
    })
}

pub async fn query(connection: &Connection, battery: &Battery) -> Result<ChargeLimit, String> {
    tokio::time::timeout(QUERY_TIMEOUT, async {
        let path = matching_device(connection, &battery.name)
            .await?
            .ok_or_else(|| {
                format!(
                    "UPower has no present power-supply battery named {}",
                    battery.name
                )
            })?;
        settings(connection, &path).await
    })
    .await
    .map_err(|_| "UPower battery query timed out".to_owned())?
}

pub async fn set_enabled(
    connection: &Connection,
    battery: &Battery,
    enabled: bool,
) -> Result<ChargeLimit, String> {
    let path = tokio::time::timeout(QUERY_TIMEOUT, async {
        let path = matching_device(connection, &battery.name)
            .await?
            .ok_or_else(|| {
                format!(
                    "UPower has no present power-supply battery named {}",
                    battery.name
                )
            })?;
        let before = settings(connection, &path).await?;
        if !before.supported {
            return Err("UPower reports that battery charge limits are unsupported".into());
        }
        Ok::<_, String>(path)
    })
    .await
    .map_err(|_| "UPower charge limit discovery timed out".to_owned())??;
    let proxy = zbus::Proxy::new(connection, NAME, &path, DEVICE_IFACE)
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(
        ACTION_TIMEOUT,
        proxy.call::<_, _, ()>("EnableChargeThreshold", &(enabled,)),
    )
    .await
    .map_err(|_| "UPower charge limit request timed out".to_owned())?
    .map_err(|error| match error {
        zbus::Error::MethodError(_, Some(detail), _) => detail,
        error => error.to_string(),
    })?;
    tokio::time::timeout(QUERY_TIMEOUT, async {
        let after = settings(connection, &path).await?;
        if after.enabled != enabled {
            return Err(format!(
                "UPower did not apply the charge-limit request: requested {}, reports {}",
                if enabled { "on" } else { "off" },
                if after.enabled { "on" } else { "off" }
            ));
        }
        Ok(after)
    })
    .await
    .map_err(|_| "UPower charge limit readback timed out".to_owned())?
}

pub async fn apply_custom_limit<T, F, Fut>(
    connection: Option<&Connection>,
    battery: &Battery,
    previously_active: bool,
    write: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    match connection {
        Some(connection) => match query(connection, battery).await {
            Ok(policy) if policy.supported && policy.enabled => {
                // UPower reapplies its preset on every enable call; disable it before writing.
                set_enabled(connection, battery, false).await?;
            }
            Ok(_) => {}
            Err(error) if previously_active => return Err(error),
            Err(_) => {}
        },
        None if previously_active => {
            return Err("Could not verify the active UPower charge preset".into());
        }
        None => {}
    }
    write().await
}

async fn system_connection_with_method_timeout(
    method_timeout: Duration,
) -> Result<Connection, String> {
    tokio::time::timeout(
        QUERY_TIMEOUT,
        zbus::connection::Builder::system()
            .map_err(|e| e.to_string())?
            .method_timeout(method_timeout)
            .build(),
    )
    .await
    .map_err(|_| "UPower system bus connection timed out".to_owned())?
    .map_err(|e| e.to_string())
}

pub async fn system_connection() -> Result<Connection, String> {
    system_connection_with_method_timeout(QUERY_TIMEOUT).await
}

pub async fn system_connection_for_action() -> Result<Connection, String> {
    system_connection_with_method_timeout(ACTION_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::BufRead,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    const BATTERY: &str = "/org/freedesktop/UPower/devices/battery_BAT0";
    const COMPOSITE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

    struct PrivateBus {
        child: std::process::Child,
        address: String,
        _directory: tempfile::TempDir,
    }
    impl PrivateBus {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("system-bus.conf");
            std::fs::write(&config, "<busconfig><type>system</type><listen>unix:tmpdir=/tmp</listen><policy context=\"default\"><allow send_destination=\"*\"/><allow receive_sender=\"*\"/><allow own=\"*\"/></policy></busconfig>").unwrap();
            let mut child = std::process::Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
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
                _directory: dir,
            }
        }
        async fn connect(&self) -> Connection {
            zbus::connection::Builder::address(self.address.as_str())
                .unwrap()
                .method_timeout(QUERY_TIMEOUT)
                .build()
                .await
                .unwrap()
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    struct FakeRoot;
    #[zbus::interface(name = "org.freedesktop.UPower")]
    impl FakeRoot {
        async fn enumerate_devices(&self) -> Vec<OwnedObjectPath> {
            vec![COMPOSITE.try_into().unwrap(), BATTERY.try_into().unwrap()]
        }
    }

    #[derive(Clone)]
    struct FakeControl {
        supported: std::sync::Arc<AtomicBool>,
        enabled: std::sync::Arc<AtomicBool>,
        calls: std::sync::Arc<AtomicUsize>,
        apply: std::sync::Arc<AtomicBool>,
        reject: std::sync::Arc<AtomicBool>,
    }
    struct FakeDevice {
        native_path: String,
        kind: u32,
        power_supply: bool,
        present: bool,
        control: FakeControl,
    }
    impl FakeDevice {
        fn new(
            native_path: &str,
            kind: u32,
            power_supply: bool,
            present: bool,
            supported: bool,
            enabled: bool,
        ) -> (Self, FakeControl) {
            let control = FakeControl {
                supported: std::sync::Arc::new(AtomicBool::new(supported)),
                enabled: std::sync::Arc::new(AtomicBool::new(enabled)),
                calls: std::sync::Arc::new(AtomicUsize::new(0)),
                apply: std::sync::Arc::new(AtomicBool::new(true)),
                reject: std::sync::Arc::new(AtomicBool::new(false)),
            };
            (
                Self {
                    native_path: native_path.into(),
                    kind,
                    power_supply,
                    present,
                    control: control.clone(),
                },
                control,
            )
        }
    }
    #[zbus::interface(name = "org.freedesktop.UPower.Device")]
    impl FakeDevice {
        #[zbus(property)]
        async fn native_path(&self) -> String {
            self.native_path.clone()
        }
        #[zbus(property, name = "Type")]
        async fn device_type(&self) -> u32 {
            self.kind
        }
        #[zbus(property)]
        async fn power_supply(&self) -> bool {
            self.power_supply
        }
        #[zbus(property)]
        async fn is_present(&self) -> bool {
            self.present
        }
        #[zbus(property)]
        async fn charge_threshold_supported(&self) -> bool {
            self.control.supported.load(Ordering::SeqCst)
        }
        #[zbus(property)]
        async fn charge_threshold_enabled(&self) -> bool {
            self.control.enabled.load(Ordering::SeqCst)
        }
        #[zbus(property)]
        async fn charge_threshold_settings_supported(&self) -> u32 {
            2
        }
        #[zbus(property)]
        async fn charge_start_threshold(&self) -> u32 {
            u32::MAX
        }
        #[zbus(property)]
        async fn charge_end_threshold(&self) -> u32 {
            80
        }
        async fn enable_charge_threshold(&self, enabled: bool) -> zbus::fdo::Result<()> {
            self.control.calls.fetch_add(1, Ordering::SeqCst);
            if self.control.reject.load(Ordering::SeqCst) {
                return Err(zbus::fdo::Error::Failed("fixture rejected request".into()));
            }
            if self.control.apply.load(Ordering::SeqCst) {
                self.control.enabled.store(enabled, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn battery(name: &str) -> Battery {
        Battery {
            name: name.into(),
            percent: 50,
            status: "Discharging".into(),
            driver_limit: Some(95),
            configured_limit: None,
            charge_limit: None,
        }
    }
    async fn fixture() -> (PrivateBus, Connection, Connection, FakeControl, FakeControl) {
        let bus = PrivateBus::new();
        let (actual, actual_control) = FakeDevice::new(
            "/sys/devices/test/power_supply/BAT0",
            2,
            true,
            true,
            true,
            true,
        );
        let (composite, composite_control) = FakeDevice::new("BAT0", 0, false, true, true, false);
        let server = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .serve_at(ROOT, FakeRoot)
            .unwrap()
            .serve_at(BATTERY, actual)
            .unwrap()
            .serve_at(COMPOSITE, composite)
            .unwrap()
            .build()
            .await
            .unwrap();
        server.request_name(NAME).await.unwrap();
        let client = bus.connect().await;
        (bus, server, client, actual_control, composite_control)
    }

    #[test]
    fn discovers_native_battery_and_reports_threshold_mask() {
        runtime().block_on(async {
            let (_bus, _server, client, _actual, composite) = fixture().await;
            let state = query(&client, &battery("BAT0")).await.unwrap();
            assert_eq!(
                state,
                ChargeLimit {
                    supported: true,
                    enabled: true,
                    start_threshold: None,
                    end_threshold: Some(80)
                }
            );
            assert_eq!(composite.calls.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn unsupported_and_missing_devices_are_explicit() {
        runtime().block_on(async {
            let (_bus, _server, client, actual, _composite) = fixture().await;
            // Exercise the unsupported policy through the injected device's exposed property.
            actual.supported.store(false, Ordering::SeqCst);
            let unsupported = query(&client, &battery("BAT0")).await.unwrap();
            assert!(!unsupported.supported);
            assert!(query(&client, &battery("BAT9")).await.is_err());
        });
    }

    #[test]
    fn enable_method_is_reapplied_even_when_value_is_unchanged_and_read_back() {
        runtime().block_on(async {
            let (_bus, _server, client, actual, _composite) = fixture().await;
            let state = set_enabled(&client, &battery("BAT0"), true).await.unwrap();
            assert!(state.enabled);
            let state = set_enabled(&client, &battery("BAT0"), true).await.unwrap();
            assert!(state.enabled);
            assert_eq!(actual.calls.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn method_rejection_and_readback_mismatch_are_errors() {
        runtime().block_on(async {
            let (_bus, _server, client, actual, _composite) = fixture().await;
            actual.reject.store(true, Ordering::SeqCst);
            assert!(
                set_enabled(&client, &battery("BAT0"), false)
                    .await
                    .unwrap_err()
                    .contains("fixture rejected request")
            );

            actual.reject.store(false, Ordering::SeqCst);
            actual.apply.store(false, Ordering::SeqCst);
            assert!(
                set_enabled(&client, &battery("BAT0"), false)
                    .await
                    .unwrap_err()
                    .contains("did not apply")
            );
        });
    }

    #[test]
    fn custom_limit_disables_upower_before_writing_and_keeps_custom_percent() {
        runtime().block_on(async {
            let (_bus, _server, client, actual, _composite) = fixture().await;
            let root = tempfile::tempdir().unwrap();
            let battery_dir = root.path().join("BAT0");
            std::fs::create_dir(&battery_dir).unwrap();
            std::fs::write(battery_dir.join("type"), "Battery\n").unwrap();
            std::fs::write(battery_dir.join("charge_control_end_threshold"), "80\n").unwrap();
            let battery = battery("BAT0");
            for percent in [85, 95, 100] {
                let result = apply_custom_limit(Some(&client), &battery, true, || async {
                    assert!(!actual.enabled.load(Ordering::SeqCst));
                    crate::quick_settings::system::set_charge_at(root.path(), "BAT0", percent)
                })
                .await
                .unwrap();
                assert_eq!(result, percent);
                assert_eq!(
                    std::fs::read_to_string(battery_dir.join("charge_control_end_threshold"))
                        .unwrap()
                        .trim(),
                    percent.to_string()
                );
            }
            assert_eq!(actual.calls.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn failed_upower_disable_prevents_custom_write() {
        runtime().block_on(async {
            let (_bus, _server, client, actual, _composite) = fixture().await;
            let root = tempfile::tempdir().unwrap();
            let battery_dir = root.path().join("BAT0");
            std::fs::create_dir(&battery_dir).unwrap();
            std::fs::write(battery_dir.join("type"), "Battery\n").unwrap();
            let threshold = battery_dir.join("charge_control_end_threshold");
            std::fs::write(&threshold, "80\n").unwrap();
            actual.reject.store(true, Ordering::SeqCst);
            let writes = AtomicUsize::new(0);
            assert!(
                apply_custom_limit(Some(&client), &battery("BAT0"), true, || async {
                    writes.fetch_add(1, Ordering::SeqCst);
                    crate::quick_settings::system::set_charge_at(root.path(), "BAT0", 95)
                })
                .await
                .is_err()
            );
            assert_eq!(writes.load(Ordering::SeqCst), 0);
            assert_eq!(std::fs::read_to_string(threshold).unwrap(), "80\n");
        });
    }
}
