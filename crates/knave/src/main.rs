use std::process::ExitCode;

use knave_config::ConfigDocument;

fn usage() -> &'static str {
    "usage:
  knave version
  knave session start|check
  knave config path|init|check|print
  knave reload
  knave snapshot|windows|workspaces|devices|active-window|active-workspace|desktop-version
  knave dispatch <command> [args...]"
}

// These modes are private launch contracts between components of this build.
fn internal(args: &[String]) -> Result<(), String> {
    match args.split_first() {
        Some((mode, args)) => match mode.as_str() {
            "compositor" => villain::run(args.iter().cloned()).map_err(|e| e.to_string()),
            "shell" => knave_shell::entrypoint::run(args),
            "quick-settings-service" if args.is_empty() => knave_session::quick_settings::run(),
            "notifications-service" if args.is_empty() => {
                knave_session::notifications::run_default()
            }
            "charge-limit" if args.len() == 2 => knave_session::quick_settings::charge_helper(
                &args[0],
                args[1].parse().map_err(|_| "invalid charge limit")?,
            ),
            _ => Err("invalid internal process mode or arguments".into()),
        },
        None => Err("missing internal process mode".into()),
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(usage().into());
    };
    match (command.as_str(), rest) {
        ("--help" | "-h", []) => println!("{}", usage()),
        ("version", []) => println!("knave {}", env!("CARGO_PKG_VERSION")),
        ("--internal", args) => return internal(args),
        ("session", [action]) => match action.as_str() {
            "start" => knave_session::run_default().map_err(|error| error.to_string())?,
            "check" => {
                let document = ConfigDocument::at_default_path().map_err(|e| e.to_string())?;
                println!(
                    "valid {} (backend {:?}, executable {})",
                    document.path().display(),
                    document.config().session.backend,
                    std::env::current_exe()
                        .map_err(|e| e.to_string())?
                        .display()
                );
            }
            _ => return Err(usage().into()),
        },
        ("config", [action]) => match action.as_str() {
            "path" => println!(
                "{}",
                ConfigDocument::default_path()
                    .map_err(|e| e.to_string())?
                    .display()
            ),
            "init" => {
                let path = ConfigDocument::default_path().map_err(|e| e.to_string())?;
                ConfigDocument::write_default_at(path.clone()).map_err(|e| e.to_string())?;
                println!("created {}", path.display());
            }
            "check" => {
                let document = ConfigDocument::at_default_path().map_err(|e| e.to_string())?;
                println!("valid {}", document.path().display());
            }
            "print" => print!(
                "{}",
                ConfigDocument::at_default_path()
                    .map_err(|e| e.to_string())?
                    .source()
            ),
            _ => return Err(usage().into()),
        },
        (
            "reload" | "snapshot" | "windows" | "workspaces" | "devices" | "active-window"
            | "active-workspace" | "desktop-version" | "dispatch",
            _,
        ) => return knavectl::run(args),
        _ => return Err(usage().into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("knave: {error}");
            ExitCode::FAILURE
        }
    }
}
