use std::{
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
};

use knave_shell_ui::UiScene;
use knave_wayland::ShellRole;

fn configured_settings() -> Result<knave_config::Config, String> {
    let config = knave_config::ConfigDocument::at_default_path()
        .map_err(|error| format!("could not load Knave settings: {error}"))?;
    Ok(config.config().clone())
}

fn configured_theme() -> Result<knave_ui::toolkit::Theme, String> {
    Ok(crate::toolkit_theme(
        configured_settings()?.appearance.theme,
    ))
}

fn usage() -> &'static str {
    "usage: knave --internal shell [--check] [bar|overview|portal-picker|shortcut-assignment|background-permission|polkit-agent|polkit-cache]"
}

fn check(role: ShellRole) {
    if role == ShellRole::Overview {
        crate::overview::Overview::new()
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

pub fn run(args: &[String]) -> Result<(), String> {
    if args.iter().map(String::as_str).eq(["polkit-cache"]) {
        return crate::polkit::cache_status();
    }
    if args.iter().map(String::as_str).eq(["polkit-agent"]) {
        return crate::polkit::run_with_theme(configured_theme()?);
    }
    if args.iter().map(String::as_str).eq(["quick-settings"]) {
        return crate::quick_settings::run_with_theme(configured_theme()?);
    }
    if args.first().map(String::as_str) == Some("tray-menu") {
        let args: Vec<_> = args.iter().skip(1).cloned().collect();
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
    if args.iter().map(String::as_str).eq(["notifications"]) {
        return crate::notifications::run_with_theme(configured_theme()?);
    }
    if args.first().map(String::as_str) == Some("wallpaper") {
        let args: Vec<_> = args.iter().skip(1).cloned().collect();
        if args.len() != 2 {
            return Err(
                "usage: knave --internal shell wallpaper ABSOLUTE_PNG_PATH cover|contain|stretch"
                    .into(),
            );
        }
        return crate::wallpaper::run(&args[0], &args[1]);
    }
    if args.iter().map(String::as_str).eq(["portal-picker"]) {
        return crate::portal::run_with_theme(configured_theme()?);
    }
    if args.iter().map(String::as_str).eq(["shortcut-assignment"]) {
        return crate::shortcut_assignment::run_with_theme(configured_theme()?);
    }
    if args
        .iter()
        .map(String::as_str)
        .eq(["background-permission"])
    {
        return crate::background_permission::run_with_theme(configured_theme()?);
    }
    let mut check_only = false;
    let mut role = ShellRole::Bar;

    for argument in args {
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
            return Err("overview is managed by the Knave session supervisor".into());
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
            crate::overview::Overview::with_theme(crate::toolkit_theme(settings.appearance.theme))
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
