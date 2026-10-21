//! Reuse an agent where possible; an explicitly launched fallback belongs to this service.
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};
#[derive(Default)]
pub(super) struct Agent {
    child: Option<Child>,
}
const CANDIDATES: &[&str] = &[
    "/usr/lib/hyprpolkitagent/hyprpolkitagent",
    "/usr/lib/polkit-kde-authentication-agent-1",
    "/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1",
    "/usr/libexec/polkit-gnome-authentication-agent-1",
    "/usr/lib/mate-polkit/polkit-mate-authentication-agent-1",
    "/usr/bin/lxqt-policykit-agent",
];
fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn discover<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Option<PathBuf> {
    paths
        .into_iter()
        .find(|p| executable(p))
        .map(Path::to_owned)
}
async fn hypr_running() -> bool {
    let check = async {
        let connection = zbus::Connection::session().await?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        proxy
            .call::<_, _, bool>("NameHasOwner", &("org.hyprland.hyprpolkitagent",))
            .await
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(1), check).await,
        Ok(Ok(true))
    )
}
impl Agent {
    pub async fn ensure(&mut self) -> Result<(), String> {
        if let Some(child) = &mut self.child {
            if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Ok(());
            }
            self.child = None;
        }
        if hypr_running().await {
            return Ok(());
        }
        let Some(path) = discover(CANDIDATES.iter().map(Path::new)) else {
            return Ok(());
        };
        let hypr = path == Path::new(CANDIDATES[0]);
        self.child = Some(
            Command::new(&path)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| {
                    format!(
                        "Could not start authentication agent {}: {e}",
                        path.display()
                    )
                })?,
        );
        // Registration is asynchronous. Bound startup waiting to the explicit Apply action.
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_some()
            {
                self.child = None;
                break;
            }
            if hypr && hypr_running().await {
                break;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_executable_files_are_selected() {
        let d = tempfile::tempdir().unwrap();
        let no = d.path().join("not-executable");
        std::fs::write(&no, "agent").unwrap();
        let yes = d.path().join("agent");
        std::fs::write(&yes, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&yes, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(discover([no.as_path(), yes.as_path()]), Some(yes));
        assert!(discover([d.path()]).is_none());
    }
}
