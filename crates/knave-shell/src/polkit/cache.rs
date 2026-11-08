//! Read the authority's live cache; Knave does not maintain a second cache.
use std::time::{SystemTime, UNIX_EPOCH};

pub fn status() -> Result<(), String> {
    glib::MainContext::new()
        .with_thread_default(|| {
            let authority = polkit::Authority::sync(gio::Cancellable::NONE)
                .map_err(|e| format!("Could not read Polkit authorization cache: {e}"))?;
            let subject = polkit::UnixSession::new_for_process_sync(
                std::process::id() as i32,
                gio::Cancellable::NONE,
            )
            .map_err(|e| e.to_string())?
            .ok_or("Could not find the current login session")?;
            let entries = authority
                .enumerate_temporary_authorizations_sync(&subject, gio::Cancellable::NONE)
                .map_err(|e| format!("Could not read Polkit authorization cache: {e}"))?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs();
            let active: Vec<_> = entries
                .iter()
                .filter(|entry| entry.time_expires() > now)
                .collect();
            if active.is_empty() {
                println!("No cached authorizations.");
            } else {
                for entry in active {
                    println!(
                        "{} — expires in {} seconds (Unix time {})",
                        entry.action_id(),
                        entry.time_expires() - now,
                        entry.time_expires()
                    );
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?
}
