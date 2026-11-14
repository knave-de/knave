use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::Command,
    thread,
};

fn knave() -> Command {
    Command::new(env!("CARGO_BIN_EXE_knave"))
}

#[test]
fn internal_dispatch_rejects_malformed_modes_without_starting_a_desktop() {
    for args in [
        vec!["--internal"],
        vec!["--internal", "unknown"],
        vec!["--internal", "compositor", "--bogus"],
        vec!["--internal", "quick-settings-service", "extra"],
        vec!["--internal", "charge-limit", "BAT0"],
        vec!["--internal", "charge-limit", "BAT0", "not-a-number"],
        vec!["session", "start", "extra"],
        vec!["config", "init", "extra"],
        vec!["version", "extra"],
    ] {
        let output = knave().args(&args).output().unwrap();
        assert!(!output.status.success(), "unexpected success: {args:?}");
    }
    for args in [
        vec!["--help"],
        vec!["version"],
        vec!["--internal", "compositor", "--help"],
        vec!["--internal", "shell", "--check", "bar"],
        vec!["--internal", "shell", "--check", "overview"],
    ] {
        let output = knave().args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn public_control_commands_adapt_to_private_transport() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("desktop.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(
            line.trim(),
            r#"{"type":"query","payload":{"query":"windows"}}"#
        );
        stream
            .write_all(b"{\"type\":\"windows\",\"payload\":[]}\n")
            .unwrap();
    });
    let output = knave()
        .arg("windows")
        .env("KNAVE_SOCKET", &socket)
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("windows"));
}
