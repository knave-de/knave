use std::process::ExitCode;

use knave_config::ConfigDocument;
use knave_desktop_api::API_VERSION;
use knave_session::run_default;

fn usage() -> &'static str {
    "usage:
  knave version
  knave session start
  knave session check
  knave config path
  knave config init
  knave config migrate
  knave config check
  knave config print"
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if matches!(args.next().as_deref(), Some("--help" | "-h")) {
        println!("{}", usage());
        return Ok(());
    }
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("version") => {
            println!(
                "knave 0.1.0 (desktop-api {}.{})",
                API_VERSION.major, API_VERSION.minor
            );
        }
        Some("session") => match args.next().as_deref() {
            Some("start") => run_default().map_err(|error| error.to_string())?,
            Some("check") => {
                let document =
                    ConfigDocument::at_default_path().map_err(|error| error.to_string())?;
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
        },
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
            Some("migrate") => {
                let path = ConfigDocument::default_path().map_err(|error| error.to_string())?;
                if !path.exists() {
                    return Err(format!(
                        "configuration does not exist at {}; run `knave config init` first",
                        path.display()
                    ));
                }
                let mut document = ConfigDocument::load(path).map_err(|error| error.to_string())?;
                if document
                    .materialize_root_compositor()
                    .map_err(|error| error.to_string())?
                {
                    println!(
                        "materialized root compositor settings in {}",
                        document.path().display()
                    );
                } else {
                    println!("no root compositor settings require migration");
                }
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
