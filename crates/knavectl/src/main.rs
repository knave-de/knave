use std::process::ExitCode;

use knave_desktop_api::{
    DesktopClient, DesktopCommand, DesktopQuery, DesktopRequest, DesktopResponse, WindowId,
    WorkspaceId,
};

fn usage() -> &'static str {
    "usage:
  knavectl reload
  knavectl snapshot
  knavectl dispatch close
  knavectl dispatch minimize
  knavectl dispatch restore-minimized
  knavectl dispatch workspace <1-10>
  knavectl dispatch focus-window <id>
  knavectl dispatch restore-window <id>
  knavectl dispatch exec <program> [args...]
  knavectl dispatch quit
  knavectl windows
  knavectl workspaces
  knavectl active-window
  knavectl active-workspace
  knavectl version"
}

fn parse_number<T: std::str::FromStr>(value: Option<String>, name: &str) -> Result<T, String> {
    value
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

fn ensure_no_args(args: &mut impl Iterator<Item = String>, command: &str) -> Result<(), String> {
    match args.next() {
        Some(argument) => Err(format!("{command} does not accept argument: {argument}")),
        None => Ok(()),
    }
}

fn parse_workspace(value: Option<String>) -> Result<WorkspaceId, String> {
    let workspace: u32 = parse_number(value, "workspace")?;
    if (1..=10).contains(&workspace) {
        Ok(WorkspaceId(workspace))
    } else {
        Err(format!(
            "workspace must be between 1 and 10, got {workspace}"
        ))
    }
}

fn parse_request(mut args: impl Iterator<Item = String>) -> Result<DesktopRequest, String> {
    match args.next().as_deref() {
        Some("reload") => {
            ensure_no_args(&mut args, "reload")?;
            Ok(DesktopRequest::Dispatch(
                DesktopCommand::ReloadConfiguration,
            ))
        }
        Some("snapshot") => {
            ensure_no_args(&mut args, "snapshot")?;
            Ok(DesktopRequest::Query(DesktopQuery::Snapshot))
        }
        Some("dispatch") => match args.next().as_deref() {
            Some("reload") => {
                ensure_no_args(&mut args, "dispatch reload")?;
                Ok(DesktopRequest::Dispatch(
                    DesktopCommand::ReloadConfiguration,
                ))
            }
            Some("close") => {
                ensure_no_args(&mut args, "dispatch close")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::CloseFocused))
            }
            Some("minimize") => {
                ensure_no_args(&mut args, "dispatch minimize")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::MinimizeFocused))
            }
            Some("restore-minimized") => {
                ensure_no_args(&mut args, "dispatch restore-minimized")?;
                Ok(DesktopRequest::Dispatch(
                    DesktopCommand::RestoreLastMinimized,
                ))
            }
            Some("workspace") => {
                let workspace = parse_workspace(args.next())?;
                ensure_no_args(&mut args, "dispatch workspace")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::FocusWorkspace {
                    workspace,
                }))
            }
            Some("focus-window") => {
                let window = WindowId(parse_number(args.next(), "window id")?);
                ensure_no_args(&mut args, "dispatch focus-window")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::FocusWindow {
                    window,
                }))
            }
            Some("restore-window") => {
                let window = WindowId(parse_number(args.next(), "window id")?);
                ensure_no_args(&mut args, "dispatch restore-window")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::RestoreWindow {
                    window,
                }))
            }
            Some("exec") => {
                let argv: Vec<_> = args.collect();
                if argv.is_empty() {
                    Err("dispatch exec requires a program".into())
                } else {
                    Ok(DesktopRequest::Dispatch(DesktopCommand::Spawn { argv }))
                }
            }
            Some("quit") => {
                ensure_no_args(&mut args, "dispatch quit")?;
                Ok(DesktopRequest::Dispatch(DesktopCommand::Quit))
            }
            Some(command) => Err(format!("unknown dispatch command: {command}")),
            None => Err("missing dispatch command".into()),
        },
        Some("windows") => {
            ensure_no_args(&mut args, "windows")?;
            Ok(DesktopRequest::Query(DesktopQuery::Windows))
        }
        Some("workspaces") => {
            ensure_no_args(&mut args, "workspaces")?;
            Ok(DesktopRequest::Query(DesktopQuery::Workspaces))
        }
        Some("active-window") => {
            ensure_no_args(&mut args, "active-window")?;
            Ok(DesktopRequest::Query(DesktopQuery::ActiveWindow))
        }
        Some("active-workspace") => {
            ensure_no_args(&mut args, "active-workspace")?;
            Ok(DesktopRequest::Query(DesktopQuery::ActiveWorkspace))
        }
        Some("version") => {
            ensure_no_args(&mut args, "version")?;
            Ok(DesktopRequest::Query(DesktopQuery::Version))
        }
        Some(command) => Err(format!("unknown command: {command}")),
        None => Err("missing command".into()),
    }
}

fn print_response(response: &DesktopResponse) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(response).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if matches!(args.next().as_deref(), Some("--help" | "-h")) {
        println!("{}", usage());
        return Ok(());
    }

    let request = parse_request(std::env::args().skip(1))?;
    let mut client = DesktopClient::connect().map_err(|error| error.to_string())?;
    let response = client
        .request(&request)
        .map_err(|error| error.to_string())?;
    print_response(&response)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("knavectl: {error}\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_replacement_commands() {
        assert_eq!(
            parse_request(["reload"].map(str::to_owned).into_iter()).unwrap(),
            DesktopRequest::Dispatch(DesktopCommand::ReloadConfiguration)
        );
        assert_eq!(
            parse_request(
                ["dispatch", "workspace", "2"]
                    .map(str::to_owned)
                    .into_iter()
            )
            .unwrap(),
            DesktopRequest::Dispatch(DesktopCommand::FocusWorkspace {
                workspace: WorkspaceId(2)
            })
        );
        assert_eq!(
            parse_request(["snapshot"].map(str::to_owned).into_iter()).unwrap(),
            DesktopRequest::Query(DesktopQuery::Snapshot)
        );
    }

    #[test]
    fn rejects_missing_exec_program() {
        let error = parse_request(["dispatch", "exec"].map(str::to_owned).into_iter()).unwrap_err();
        assert_eq!(error, "dispatch exec requires a program");
    }

    #[test]
    fn rejects_invalid_workspace_and_trailing_arguments() {
        let error = parse_request(
            ["dispatch", "workspace", "0"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap_err();
        assert_eq!(error, "workspace must be between 1 and 10, got 0");

        let error =
            parse_request(["snapshot", "unexpected"].map(str::to_owned).into_iter()).unwrap_err();
        assert_eq!(error, "snapshot does not accept argument: unexpected");

        let error = parse_request(
            ["dispatch", "close", "unexpected"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap_err();
        assert_eq!(error, "dispatch close does not accept argument: unexpected");
    }
}
