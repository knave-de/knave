use knave_desktop_api::quick_settings::*;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

pub async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let task = async {
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("missing stdout")?.take(65537);
        let mut stderr = child.stderr.take().ok_or("missing stderr")?.take(8193);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (a, b) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        a.map_err(|e| e.to_string())?;
        b.map_err(|e| e.to_string())?;
        if out.len() > 65536 || err.len() > 8192 {
            return Err(format!("{program}: output exceeds budget"));
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "{program}: {}",
                String::from_utf8_lossy(&err).trim()
            ));
        }
        String::from_utf8(out).map_err(|e| e.to_string())
    };
    tokio::time::timeout(Duration::from_secs(8), task)
        .await
        .map_err(|_| format!("{program}: timed out"))?
}
pub fn fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            fields.last_mut().unwrap().push(c);
            escaped = false
        } else if c == '\\' {
            escaped = true
        } else if c == ':' {
            fields.push(String::new())
        } else {
            fields.last_mut().unwrap().push(c)
        }
    }
    fields
}
pub async fn network() -> Result<Network, String> {
    let (devices, radio, connections) = tokio::join!(
        run(
            "nmcli",
            &[
                "-t",
                "-f",
                "DEVICE,TYPE,STATE,CONNECTION",
                "device",
                "status"
            ]
        ),
        run("nmcli", &["radio", "wifi"]),
        run(
            "nmcli",
            &["-t", "-f", "UUID,NAME,TYPE", "connection", "show"]
        )
    );
    let mut n = Network {
        wifi: radio?.trim() == "enabled",
        ..Default::default()
    };
    for line in devices?.lines().take(64) {
        let f = fields(line);
        if f.len() == 4 && f[2] == "connected" {
            match f[1].as_str() {
                "ethernet" => n.ethernet = Some(f[0].clone()),
                "wifi" => n.wireless = Some(f[3].clone()),
                _ => {}
            }
        }
    }
    for line in connections?.lines().take(128) {
        let f = fields(line);
        if f.len() == 3 && matches!(f[2].as_str(), "802-11-wireless" | "802-3-ethernet") {
            n.connections.push(NetworkConnection {
                uuid: f[0].clone(),
                name: f[1].chars().take(120).collect(),
                wired: f[2] == "802-3-ethernet",
            })
        }
    }
    Ok(n)
}
pub fn parse_volume(text: &str) -> Result<Volume, String> {
    let value = text
        .split_whitespace()
        .nth(1)
        .ok_or("missing volume")?
        .parse::<f64>()
        .map_err(|e| e.to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("invalid volume".into());
    }
    Ok(Volume {
        percent: (value * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: text.contains("[MUTED]"),
    })
}
pub async fn volume(input: bool) -> Result<Volume, String> {
    parse_volume(
        &run(
            "wpctl",
            &[
                "get-volume",
                if input {
                    "@DEFAULT_AUDIO_SOURCE@"
                } else {
                    "@DEFAULT_AUDIO_SINK@"
                },
            ],
        )
        .await?,
    )
}
pub async fn brightness() -> Result<u8, String> {
    let text = run("brightnessctl", &["-m"]).await?;
    text.lines()
        .next()
        .and_then(|l| l.split(',').nth(3))
        .and_then(|v| v.trim_end_matches('%').parse().ok())
        .ok_or("brightness unavailable".into())
}
pub async fn bluetooth() -> Result<bool, String> {
    let text = run("bluetoothctl", &["show"]).await?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Powered: "))
        .map(|v| v == "yes")
        .ok_or("Bluetooth adapter unavailable".into())
}
pub async fn power() -> Result<Power, String> {
    let (active, available) = tokio::join!(
        run("powerprofilesctl", &["get"]),
        run("powerprofilesctl", &["list"])
    );
    let available = available?
        .lines()
        .filter_map(|l| {
            let l = l
                .trim()
                .trim_start_matches('*')
                .trim()
                .trim_end_matches(':');
            matches!(l, "power-saver" | "balanced" | "performance").then(|| l.to_owned())
        })
        .collect();
    Ok(Power {
        active: active?.trim().into(),
        available,
    })
}
pub fn battery_at(root: &Path) -> Result<Option<Battery>, String> {
    let mut entries = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|s| s.trim() == "Battery") {
            let read = |name: &str| {
                std::fs::read_to_string(p.join(name))
                    .map(|s| s.trim().to_owned())
                    .map_err(|e| e.to_string())
            };
            return Ok(Some(Battery {
                name: e.file_name().to_string_lossy().into_owned(),
                percent: read("capacity")?
                    .parse::<u8>()
                    .map_err(|e| e.to_string())?
                    .min(100),
                status: read("status")?,
                driver_limit: read("charge_control_end_threshold")
                    .ok()
                    .and_then(|s| s.parse().ok()),
                configured_limit: None,
                charge_limit: None,
            }));
        }
    }
    Ok(None)
}

fn validate_charge_target(name: &str, percent: u8) -> Result<(), String> {
    if !(50..=100).contains(&percent)
        || name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid battery or charge limit".into());
    }
    Ok(())
}

fn charge_battery_path(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

fn write_charge_limit(attribute: &Path, percent: u8) -> std::io::Result<()> {
    write_charge_limit_with(
        attribute,
        percent,
        |path, value| {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)?
                .write_all(value.as_bytes())
        },
        |path| std::fs::read_to_string(path),
    )
}

fn write_charge_limit_with(
    attribute: &Path,
    percent: u8,
    write: impl FnOnce(&Path, &str) -> std::io::Result<()>,
    read: impl FnOnce(&Path) -> std::io::Result<String>,
) -> std::io::Result<()> {
    write(attribute, &percent.to_string())?;
    let actual = read(attribute)?;
    if actual.trim().parse::<u8>().ok() != Some(percent) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "charge threshold readback mismatch (requested {percent}%, got {})",
                actual.trim()
            ),
        ));
    }
    Ok(())
}

fn charge_error(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            "Battery charge thresholds are not supported by this device".into()
        }
        std::io::ErrorKind::PermissionDenied => {
            "Permission denied while applying the charge limit".into()
        }
        std::io::ErrorKind::InvalidData => error.to_string(),
        _ => format!("Could not apply the charge limit: {error}"),
    }
}

fn helper_failure(output: &[u8]) -> String {
    let detail = String::from_utf8_lossy(output).trim().to_owned();
    if detail.is_empty() {
        "Charge limit was not applied; authentication may have been cancelled or denied".into()
    } else {
        format!("Charge limit was not applied: {detail}")
    }
}

fn charge_helper_at(root: &Path, name: &str, percent: u8) -> Result<(), String> {
    validate_charge_target(name, percent)?;
    let path = charge_battery_path(root, name);
    if std::fs::read_to_string(path.join("type"))
        .map_err(|e| e.to_string())?
        .trim()
        != "Battery"
    {
        return Err("not a battery".into());
    }
    write_charge_limit(&path.join("charge_control_end_threshold"), percent)
        .map_err(|e| charge_error(&e))
}

/// pkexec entry point. It only accepts a battery name and percentage, never an arbitrary path.
pub fn charge_helper(name: &str, percent: u8) -> Result<(), String> {
    charge_helper_at(Path::new("/sys/class/power_supply"), name, percent)
}

#[cfg(test)]
pub(super) fn set_charge_at(root: &Path, name: &str, percent: u8) -> Result<u8, String> {
    validate_charge_target(name, percent)?;
    let battery = charge_battery_path(root, name);
    if std::fs::read_to_string(battery.join("type"))
        .map_err(|e| e.to_string())?
        .trim()
        != "Battery"
    {
        return Err("not a battery".into());
    }
    write_charge_limit(&battery.join("charge_control_end_threshold"), percent)
        .map_err(|e| charge_error(&e))?;
    verify_charge_readback(root, name, percent)
}

fn verify_charge_readback(root: &Path, name: &str, requested: u8) -> Result<u8, String> {
    let actual = read_charge_limit_at(root, name)?;
    if actual != requested {
        return Err(format!(
            "charge threshold readback mismatch (requested {requested}%, got {actual}%)"
        ));
    }
    Ok(actual)
}

pub(super) fn read_charge_limit_at(root: &Path, name: &str) -> Result<u8, String> {
    validate_charge_target(name, 50)?;
    let battery = charge_battery_path(root, name);
    if std::fs::read_to_string(battery.join("type"))
        .map_err(|e| e.to_string())?
        .trim()
        != "Battery"
    {
        return Err("not a battery".into());
    }
    std::fs::read_to_string(battery.join("charge_control_end_threshold"))
        .map_err(|e| charge_error(&e))?
        .trim()
        .parse::<u8>()
        .map_err(|e| format!("invalid battery charge threshold: {e}"))
}

pub async fn set_charge(name: &str, percent: u8) -> Result<u8, String> {
    validate_charge_target(name, percent)?;
    let battery = Path::new("/sys/class/power_supply").join(name);
    if std::fs::read_to_string(battery.join("type"))
        .map_err(|e| e.to_string())?
        .trim()
        != "Battery"
    {
        return Err("not a battery".into());
    }
    let path = Path::new("/sys/class/power_supply")
        .join(name)
        .join("charge_control_end_threshold");
    match write_charge_limit(&path, percent) {
        Ok(()) => {
            return verify_charge_readback(Path::new("/sys/class/power_supply"), name, percent);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        Err(error) => return Err(charge_error(&error)),
    }

    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new("pkexec")
        .arg(executable)
        .args(["charge-limit", name, &percent.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("missing helper stderr")?
        .take(8193);
    let work = async {
        let mut output = Vec::new();
        stderr
            .read_to_end(&mut output)
            .await
            .map_err(|e| e.to_string())?;
        if output.len() > 8192 {
            return Err("charge-limit helper output exceeds budget".to_owned());
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if status.success() {
            return verify_charge_readback(Path::new("/sys/class/power_supply"), name, percent);
        }
        Err(helper_failure(&output))
    };
    tokio::time::timeout(Duration::from_secs(90), work)
        .await
        .map_err(|_| "charge-limit authentication timed out".to_owned())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_nmcli_fields_and_invalid_audio() {
        assert_eq!(
            fields(r"uuid:Office\: Wi-Fi:802-11-wireless"),
            vec!["uuid", "Office: Wi-Fi", "802-11-wireless"]
        );
        assert!(parse_volume("Volume: NaN").is_err());
        assert!(parse_volume("Volume: -1").is_err());
        assert_eq!(
            parse_volume("Volume: 0.42 [MUTED]").unwrap(),
            Volume {
                percent: 42,
                muted: true
            }
        );
    }
    #[test]
    fn no_battery_is_distinct_from_failed_read() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(battery_at(d.path()).unwrap(), None);
        assert!(battery_at(&d.path().join("absent")).is_err());
    }

    #[test]
    fn battery_driver_threshold_is_read_only_cache_data() {
        let d = tempfile::tempdir().unwrap();
        let battery = d.path().join("BAT0");
        std::fs::create_dir(&battery).unwrap();
        std::fs::write(battery.join("type"), "Battery\n").unwrap();
        std::fs::write(battery.join("capacity"), "42\n").unwrap();
        std::fs::write(battery.join("status"), "Discharging\n").unwrap();
        let threshold = battery.join("charge_control_end_threshold");
        std::fs::write(&threshold, "95\n").unwrap();

        let report = battery_at(d.path()).unwrap().unwrap();
        assert_eq!(report.driver_limit, Some(95));
        assert_eq!(report.charge_limit, None);
        assert_eq!(std::fs::read_to_string(threshold).unwrap(), "95\n");
    }

    #[test]
    fn custom_thresholds_validate_and_verify_temporary_sysfs_values() {
        let d = tempfile::tempdir().unwrap();
        let battery = d.path().join("BAT0");
        std::fs::create_dir(&battery).unwrap();
        std::fs::write(battery.join("type"), "Battery\n").unwrap();
        let threshold = battery.join("charge_control_end_threshold");
        std::fs::write(&threshold, "100\n").unwrap();

        for percent in [85, 95, 100] {
            assert_eq!(set_charge_at(d.path(), "BAT0", percent).unwrap(), percent);
            assert_eq!(read_charge_limit_at(d.path(), "BAT0").unwrap(), percent);
        }
        assert!(set_charge_at(d.path(), "BAT0", 49).is_err());
        assert!(set_charge_at(d.path(), "../BAT0", 95).is_err());
        assert!(charge_helper_at(d.path(), "BAT0", 101).is_err());
        charge_helper_at(d.path(), "BAT0", 95).unwrap();
        assert_eq!(read_charge_limit_at(d.path(), "BAT0").unwrap(), 95);
        assert!(helper_failure(b"fixture rejected helper request").contains("fixture rejected"));
        assert!(helper_failure(b"").contains("cancelled or denied"));
    }

    #[test]
    fn helper_rejects_mismatch_and_failed_writes() {
        let d = tempfile::tempdir().unwrap();
        let attribute = d.path().join("threshold");
        let mismatch = write_charge_limit_with(
            &attribute,
            85,
            |path, _| std::fs::write(path, "86"),
            |path| std::fs::read_to_string(path),
        )
        .unwrap_err();
        assert_eq!(mismatch.kind(), std::io::ErrorKind::InvalidData);
        let denied = write_charge_limit_with(
            &attribute,
            95,
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            |_| unreachable!("failed write must not read back"),
        )
        .unwrap_err();
        assert_eq!(denied.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
