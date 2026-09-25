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

fn parse_request(mut args: impl Iterator<Item = String>) -> Result<DesktopRequest, String> {
    match args.next().as_deref() {
        Some("reload") => Ok(DesktopRequest::Dispatch(
            DesktopCommand::ReloadConfiguration,
        )),
        Some("snapshot") => Ok(DesktopRequest::Query(DesktopQuery::Snapshot)),
        Some("dispatch") => match args.next().as_deref() {
            Some("reload") => Ok(DesktopRequest::Dispatch(
                DesktopCommand::ReloadConfiguration,
            )),
            Some("close") => Ok(DesktopRequest::Dispatch(DesktopCommand::CloseFocused)),
            Some("minimize") => Ok(DesktopRequest::Dispatch(DesktopCommand::MinimizeFocused)),
            Some("restore-minimized") => Ok(DesktopRequest::Dispatch(
                DesktopCommand::RestoreLastMinimized,
            )),
            Some("workspace") => Ok(DesktopRequest::Dispatch(DesktopCommand::FocusWorkspace {
                workspace: WorkspaceId(parse_number(args.next(), "workspace")?),
            })),
            Some("focus-window") => Ok(DesktopRequest::Dispatch(DesktopCommand::FocusWindow {
                window: WindowId(parse_number(args.next(), "window id")?),
            })),
            Some("restore-window") => Ok(DesktopRequest::Dispatch(DesktopCommand::RestoreWindow {
                window: WindowId(parse_number(args.next(), "window id")?),
            })),
            Some("exec") => {
                let argv: Vec<_> = args.collect();
                if argv.is_empty() {
                    Err("dispatch exec requires a program".into())
                } else {
                    Ok(DesktopRequest::Dispatch(DesktopCommand::Spawn { argv }))
                }
            }
            Some("quit") => Ok(DesktopRequest::Dispatch(DesktopCommand::Quit)),
            Some(command) => Err(format!("unknown dispatch command: {command}")),
            None => Err("missing dispatch command".into()),
        },
        Some("windows") => Ok(DesktopRequest::Query(DesktopQuery::Windows)),
        Some("workspaces") => Ok(DesktopRequest::Query(DesktopQuery::Workspaces)),
        Some("active-window") => Ok(DesktopRequest::Query(DesktopQuery::ActiveWindow)),
        Some("active-workspace") => Ok(DesktopRequest::Query(DesktopQuery::ActiveWorkspace)),
        Some("version") => Ok(DesktopRequest::Query(DesktopQuery::Version)),
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
}
