use std::process::ExitCode;

use knave_config::ConfigDocument;
use knave_desktop_api::API_VERSION;

fn usage() -> &'static str {
    "usage:
  knave version
  knave config path
  knave config init
  knave config check
  knave config print"
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("version") => {
            println!(
                "knave 0.1.0 (desktop-api {}.{})",
                API_VERSION.major, API_VERSION.minor
            );
        }
        Some("config") => match args.next().as_deref() {
            Some("path") => println!(
                "{}",
                ConfigDocument::default_path()
                    .map_err(|error| error.to_string())?
                    .display()
            ),
            Some("init") => {
                let path = ConfigDocument::default_path().map_err(|error| error.to_string())?;
                ConfigDocument::write_default_at(path.clone())
                    .map_err(|error| error.to_string())?;
                println!("created {}", path.display());
            }
            Some("check") => {
                let document =
                    ConfigDocument::at_default_path().map_err(|error| error.to_string())?;
                println!(
                    "valid {} (schema {})",
                    document.path().display(),
                    document.config().schema_version
                );
            }
            Some("print") => {
                let document =
                    ConfigDocument::at_default_path().map_err(|error| error.to_string())?;
                print!("{}", document.source());
            }
            _ => return Err(usage().into()),
        },
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("knave: {error}\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}
