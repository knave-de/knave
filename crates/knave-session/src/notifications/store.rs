use knave_desktop_api::notifications::Notification;
use rusqlite::{Connection, params};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct Store {
    db: Connection,
    _lock: fs::File,
}
impl Store {
    pub fn path() -> Result<PathBuf, String> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
            .ok_or("neither XDG_STATE_HOME nor HOME is available")?;
        Ok(base.join("knave/notifications.sqlite3"))
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("history path has no parent")?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| e.to_string())?;
        // Do not follow a substituted database symlink; SQLite sidecars inherit private directory access.
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| e.to_string())?;
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("notification history is already owned by another session".into());
        }
        lock.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(Duration::from_millis(100))
            .map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON; PRAGMA max_page_count=8192;
            CREATE TABLE IF NOT EXISTS history(id INTEGER PRIMARY KEY, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY, value INTEGER NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { db, _lock: lock })
    }
    pub fn load(&self) -> Result<(Vec<Notification>, bool, u64), String> {
        let mut statement = self
            .db
            .prepare("SELECT json FROM history ORDER BY id DESC LIMIT 500")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for row in rows {
            let json = row.map_err(|e| e.to_string())?;
            if json.len() > 16384 {
                return Err("oversized history record".into());
            }
            let mut n: Notification =
                serde_json::from_str(&json).map_err(|e| format!("invalid history: {e}"))?;
            n.live = false;
            n.banner = false;
            n.actions.clear();
            entries.push(n);
        }
        entries.sort_by_key(|n| std::cmp::Reverse((n.created_ms, n.id)));
        let read = |key: &str, default: u64| {
            self.db
                .query_row("SELECT value FROM state WHERE key=?", [key], |r| {
                    r.get::<_, u64>(0)
                })
                .or_else(|e| {
                    if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                        Ok(default)
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| e.to_string())
        };
        Ok((entries, read("dnd", 0)? != 0, read("next", 1)?))
    }
    pub fn save(&mut self, entries: &[Notification], dnd: bool, next: u64) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        let keep: Vec<_> = entries.iter().map(|n| n.id).collect();
        let placeholders = vec!["?"; keep.len()].join(",");
        tx.execute(
            &format!("DELETE FROM history WHERE id NOT IN ({placeholders})"),
            rusqlite::params_from_iter(keep),
        )
        .map_err(|e| e.to_string())?;
        for entry in entries {
            let mut archived = entry.clone();
            archived.actions.clear();
            archived.live = false;
            archived.banner = false;
            tx.execute(
                "INSERT INTO history(id,json) VALUES (?,?) ON CONFLICT(id) DO UPDATE SET json=excluded.json WHERE history.json != excluded.json",
                params![
                    archived.id,
                    serde_json::to_string(&archived).map_err(|e| e.to_string())?
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        for (key, value) in [("dnd", u64::from(dnd)), ("next", next)] {
            tx.execute(
                "INSERT INTO state(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE state.value != excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
}
