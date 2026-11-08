use std::{
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    process::ExitCode,
};

use knave_shell_ui::UiScene;
use knave_wayland::ShellRole;

fn configured_settings() -> Result<knave_config::Config, String> {
    let config = knave_config::ConfigDocument::at_default_path()
        .map_err(|error| format!("could not load Knave settings: {error}"))?;
    Ok(config.config().clone())
}

fn configured_theme() -> Result<knave_ui::toolkit::Theme, String> {
    Ok(knave_shell::toolkit_theme(
        configured_settings()?.appearance.theme,
    ))
}

fn usage() -> &'static str {
    "usage: knave-shell [--check] [bar|overview|portal-picker|shortcut-assignment|background-permission|polkit-agent|polkit-cache]\n       knave-shell --supports-overview-service"
}

fn check(role: ShellRole) {
    if role == ShellRole::Overview {
        knave_shell::overview::Overview::new()
            .check([1920.0, 1080.0])
            .expect("valid overview composition");
        println!("knave-shell role=overview retained scene valid");
        return;
    }
    let scene = match role {
        ShellRole::Bar => UiScene::bar(1, 1920.0, knave_shell_ui::status_bar::STATUS_BAR_HEIGHT),
        ShellRole::Overview => UiScene::overview(1, 1920.0, 1080.0),
    };
    let render_list = scene.display_list();
    println!(
        "knave-shell role={} revision={} commands={}",
        match role {
            ShellRole::Bar => "bar",
            ShellRole::Overview => "overview",
        },
        render_list.revision,
        render_list.commands.len()
    );
}

fn run() -> Result<(), String> {
    if std::env::args().skip(1).eq(["polkit-cache"]) {
        return knave_shell::polkit::cache_status();
    }
    if std::env::args().skip(1).eq(["--supports-polkit-agent"]) {
        return Ok(());
    }
    if std::env::args().skip(1).eq(["polkit-agent"]) {
        return knave_shell::polkit::run_with_theme(configured_theme()?);
    }
    if std::env::args().skip(1).eq(["quick-settings"]) {
        return knave_shell::quick_settings::run_with_theme(configured_theme()?);
    }
    if std::env::args().nth(1).as_deref() == Some("tray-menu") {
        let args: Vec<_> = std::env::args().skip(2).collect();
        if args.len() != 6 {
            return Err("invalid internal tray menu arguments".into());
        }
        return knave_wayland::tray_menu::run(
            args[0].clone(),
            args[1].clone(),
            args[2].clone(),
            args[3].clone(),
            args[4].parse().map_err(|_| "invalid tray menu position")?,
            args[5].parse().map_err(|_| "invalid tray menu position")?,
            configured_theme()?,
        );
    }
    if std::env::args().skip(1).eq(["notifications"]) {
        return knave_shell::notifications::run_with_theme(configured_theme()?);
    }
    if std::env::args().nth(1).as_deref() == Some("wallpaper") {
        let args: Vec<_> = std::env::args().skip(2).collect();
        if args.len() != 2 {
            return Err(
                "usage: knave-shell wallpaper ABSOLUTE_PNG_PATH cover|contain|stretch".into(),
            );
        }
        return knave_shell::wallpaper::run(&args[0], &args[1]);
    }
    if std::env::args()
        .skip(1)
        .eq(["--supports-notifications-service"])
    {
        return Ok(());
    }
    if std::env::args().skip(1).eq(["portal-picker"]) {
        return knave_shell::portal::run_with_theme(configured_theme()?);
    }
    if std::env::args().skip(1).eq(["shortcut-assignment"]) {
        return knave_shell::shortcut_assignment::run_with_theme(configured_theme()?);
    }
    if std::env::args().skip(1).eq(["background-permission"]) {
        return knave_shell::background_permission::run_with_theme(configured_theme()?);
    }
    if std::env::args().skip(1).eq(["--supports-overview-service"]) {
        return Ok(());
    }
    let mut check_only = false;
    let mut role = ShellRole::Bar;

    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--check" => check_only = true,
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            value => role = ShellRole::parse(value).ok_or_else(|| usage().to_string())?,
        }
    }

    if check_only {
        check(role);
        return Ok(());
    }

    if role == ShellRole::Overview {
        if std::env::var_os("KNAVE_OVERVIEW_SERVICE").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return Err("overview is managed by knave-session".into());
        }
        let _overview_lock = overview_lock()?;
        let settings = configured_settings()?;
        knave_wayland::run_application(
            knave_wayland::SurfaceOptions {
                keyboard: knave_wayland::KeyboardMode::Exclusive,
                layer: knave_wayland::SurfaceLayer::Overlay,
                namespace: "knave-shell-overview".into(),
                ..Default::default()
            },
            knave_shell::overview::Overview::with_theme(knave_shell::toolkit_theme(
                settings.appearance.theme,
            ))
            .with_pinned_apps(settings.shell.pinned_apps),
        )
        .map_err(|error| error.to_string())
    } else {
        knave_wayland::run_with_theme(role, configured_theme()?).map_err(|error| error.to_string())
    }
}

fn overview_lock() -> Result<File, String> {
    let socket = knave_desktop_api::socket_path().map_err(|error| error.to_string())?;
    let name = socket
        .file_name()
        .ok_or_else(|| "desktop socket path has no file name".to_owned())?
        .to_string_lossy();
    let path = socket.with_file_name(format!("{name}.overview.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|error| {
            format!(
                "could not open overview session lock {}: {error}",
                path.display()
            )
        })?;
    // `flock` is released by the kernel on process exit, including crashes.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        Ok(file)
    } else {
        let error = std::io::Error::last_os_error();
        Err(format!(
            "another overview is active for this Wayland session: {error}"
        ))
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("knave-shell: {error}\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}
