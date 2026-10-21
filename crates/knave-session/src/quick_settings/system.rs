use knave_desktop_api::quick_settings::*;
use std::{path::Path, process::Stdio, time::Duration};
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
                limit: read("charge_control_end_threshold")
                    .ok()
                    .and_then(|s| s.parse().ok()),
            }));
        }
    }
    Ok(None)
}
/// Only the kernel's battery threshold attribute is writable through this privileged entry point.
pub fn charge_helper(name: &str, limit: u8) -> Result<(), String> {
    if !(50..=100).contains(&limit)
        || name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid battery or charge limit".into());
    }
    let path = Path::new("/sys/class/power_supply").join(name);
    if std::fs::read_to_string(path.join("type"))
        .map_err(|e| e.to_string())?
        .trim()
        != "Battery"
    {
        return Err("not a battery".into());
    }
    std::fs::write(path.join("charge_control_end_threshold"), limit.to_string())
        .map_err(|e| e.to_string())
}
pub async fn set_charge(name: &str, limit: u8) -> Result<(), String> {
    let path = Path::new("/sys/class/power_supply")
        .join(name)
        .join("charge_control_end_threshold");
    match std::fs::write(path, limit.to_string()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // Authentication can take longer than ordinary system queries.
            let mut child = Command::new("pkexec")
                .arg(exe)
                .args(["charge-limit", name, &limit.to_string()])
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| e.to_string())?;
            let result = tokio::time::timeout(Duration::from_secs(90), child.wait())
                .await
                .map_err(|_| "charge-limit authentication timed out")?
                .map_err(|e| e.to_string())?;
            if result.success() {
                Ok(())
            } else {
                Err("Charge limit was not changed; authentication failed or no authentication agent is running".into())
            }
        }
        Err(e) => Err(e.to_string()),
    }
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
        assert!(charge_helper("../BAT1", 80).is_err());
        assert!(charge_helper("BAT1", 101).is_err());
    }
}
