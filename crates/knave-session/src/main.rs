use std::process::ExitCode;

use knave_config::ConfigDocument;

fn usage() -> &'static str {
    "usage:
  knave-session start
  knave-session check
  knave-session notifications"
}

fn run() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("--help") | Some("-h") => println!("{}", usage()),
        Some("quick-settings-service") => knave_session::quick_settings::run()?,
        Some("notifications") => knave_session::notifications::run_default()?,
        Some("start") => knave_session::run_default().map_err(|error| error.to_string())?,
        Some("check") => {
            let document = ConfigDocument::at_default_path().map_err(|error| error.to_string())?;
            let config = document.config();
            println!(
                "valid {} (backend {:?}, compositor {}, shell {})",
                document.path().display(),
                config.session.backend,
                config.session.compositor_binary,
                config.session.shell_binary
            );
        }
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("knave-session: {error}\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}
