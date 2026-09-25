use std::process::ExitCode;

use knave_config::ConfigDocument;

fn usage() -> &'static str {
    "usage:
  knave-session start
  knave-session check"
}

fn run() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("--help") | Some("-h") => println!("{}", usage()),
        Some("start") => knave_session::run_default().map_err(|error| error.to_string())?,
        Some("check") => {
            let document = ConfigDocument::at_default_path().map_err(|error| error.to_string())?;
            let config = document.config();
            println!(
                "valid {} (schema {}, backend {:?}, compositor {}, shell {})",
                document.path().display(),
                config.schema_version,
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
