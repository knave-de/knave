use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::{AtomicU32, Ordering},
};

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "knave-apps-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn desktop(name: &str, exec: &str, extra: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n{extra}")
}
fn env(dirs: &[&Dir]) -> Environment {
    Environment {
        data_dirs: dirs.iter().map(|d| d.0.clone()).collect(),
        ..Default::default()
    }
}
fn names(catalog: &Catalog, query: &str) -> Vec<String> {
    catalog
        .search(query, 50)
        .into_iter()
        .map(|i| catalog.get(i).unwrap().name.clone())
        .collect()
}

#[test]
fn only_visible_graphical_applications_are_listed() {
    let d = Dir::new();
    d.write("applications/a.desktop", &desktop("Alpha", "alpha", ""));
    d.write(
        "applications/i.desktop",
        &desktop("Iconic", "iconic", "Icon=my-icon\n"),
    );
    d.write(
        "applications/hidden.desktop",
        &desktop("Hid", "h", "NoDisplay=true\n"),
    );
    d.write(
        "applications/gone.desktop",
        &desktop("Gone", "g", "Hidden=true\n"),
    );
    d.write(
        "applications/term.desktop",
        &desktop("Term", "t", "Terminal=true\n"),
    );
    d.write(
        "applications/link.desktop",
        "[Desktop Entry]\nType=Link\nName=L\nURL=x\n",
    );
    d.write(
        "applications/noexec.desktop",
        "[Desktop Entry]\nType=Application\nName=N\n",
    );
    d.write("applications/readme.txt", "ignored");
    let catalog = Catalog::load_with(&env(&[&d])).unwrap();
    assert_eq!(names(&catalog, "a"), ["Alpha"]);
    assert_eq!(names(&catalog, "iconic"), ["Iconic"]);
    assert_eq!(
        catalog
            .get(catalog.search("iconic", 1)[0])
            .unwrap()
            .icon
            .as_deref(),
        Some("my-icon")
    );
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.get(0).unwrap().id, "a.desktop");
    assert_eq!(catalog.get(0).unwrap().icon, None);
}

#[test]
fn earlier_directories_override_and_can_delete_later_entries() {
    let user = Dir::new();
    let system = Dir::new();
    user.write("applications/x.desktop", &desktop("User X", "ux", ""));
    user.write(
        "applications/y.desktop",
        &desktop("User Y", "uy", "Hidden=true\n"),
    );
    system.write("applications/x.desktop", &desktop("System X", "sx", ""));
    system.write("applications/y.desktop", &desktop("System Y", "sy", ""));
    system.write("applications/z.desktop", &desktop("System Z", "sz", ""));
    let catalog = Catalog::load_with(&env(&[&user, &system, &user])).unwrap();
    let mut listed = names(&catalog, "system");
    listed.sort();
    assert_eq!(listed, ["System Z"]);
    assert_eq!(names(&catalog, "user"), ["User X"]);
    assert_eq!(catalog.len(), 2);
}

#[test]
fn subdirectories_form_dashed_ids_and_symlinked_files_are_followed() {
    let d = Dir::new();
    let target = Dir::new();
    d.write(
        "applications/vendor/tool.desktop",
        &desktop("Tool", "tool", ""),
    );
    target.write("real.desktop", &desktop("Linked", "linked", ""));
    fs::create_dir_all(d.0.join("applications")).unwrap();
    symlink(
        target.0.join("real.desktop"),
        d.0.join("applications/link.desktop"),
    )
    .unwrap();
    // A directory symlink pointing back at its parent must not recurse.
    symlink(d.0.join("applications"), d.0.join("applications/loop")).unwrap();
    let catalog = Catalog::load_with(&env(&[&d])).unwrap();
    let ids: Vec<_> = (0..catalog.len())
        .map(|i| catalog.get(i).unwrap().id.clone())
        .collect();
    assert!(ids.contains(&"vendor-tool.desktop".to_owned()));
    assert!(ids.contains(&"link.desktop".to_owned()));
    assert_eq!(ids.len(), 2);
}

#[test]
fn desktop_filters_locale_and_tryexec_apply() {
    let d = Dir::new();
    let bin = Dir::new();
    let tool = bin.0.join("present");
    fs::write(&tool, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    d.write(
        "applications/only.desktop",
        &desktop("Only", "o", "OnlyShowIn=GNOME;\n"),
    );
    d.write(
        "applications/not.desktop",
        &desktop("Not", "n", "NotShowIn=Knave;\n"),
    );
    d.write(
        "applications/have.desktop",
        &desktop("Have", "h", "TryExec=present\n"),
    );
    d.write(
        "applications/lack.desktop",
        &desktop("Lack", "l", "TryExec=absent\n"),
    );
    d.write(
        "applications/loc.desktop",
        &desktop("Files", "f", "Name[de]=Dateien\n"),
    );
    let catalog = Catalog::load_with(&Environment {
        limits: Limits::default(),
        data_dirs: vec![d.0.clone()],
        path: vec![bin.0.clone()],
        locales: vec!["de".into()],
        desktops: vec!["Knave".into()],
    })
    .unwrap();
    let mut all: Vec<_> = (0..catalog.len())
        .map(|i| catalog.get(i).unwrap().name.clone())
        .collect();
    all.sort();
    assert_eq!(all, ["Dateien", "Have"]);
}

#[test]
fn search_ranks_name_before_keywords_before_comment() {
    let d = Dir::new();
    d.write(
        "applications/1.desktop",
        &desktop("Firefox", "firefox", "Keywords=browser;web\n"),
    );
    d.write(
        "applications/2.desktop",
        &desktop("Web Inspector", "inspector", ""),
    );
    d.write(
        "applications/3.desktop",
        &desktop("Notes", "notes", "Comment=Take web notes\n"),
    );
    d.write("applications/4.desktop", &desktop("Browser Kit", "kit", ""));
    d.write(
        "applications/5.desktop",
        &desktop("Editor", "code", "GenericName=Text Editor\n"),
    );
    let catalog = Catalog::load_with(&env(&[&d])).unwrap();
    assert_eq!(
        names(&catalog, "web"),
        ["Web Inspector", "Firefox", "Notes"]
    );
    assert_eq!(names(&catalog, "BROWSER"), ["Browser Kit", "Firefox"]);
    assert_eq!(names(&catalog, "fire"), ["Firefox"]);
    assert_eq!(names(&catalog, "code"), ["Editor"]);
    assert_eq!(names(&catalog, "text edit"), ["Editor"]);
    assert_eq!(names(&catalog, "web notes"), ["Notes"]);
    assert!(names(&catalog, "   ").is_empty());
    assert!(names(&catalog, "zzz").is_empty());
    assert_eq!(catalog.search("e", 2).len(), 2);
}

#[test]
fn oversized_files_and_missing_directories_are_handled() {
    let d = Dir::new();
    d.write(
        "applications/big.desktop",
        &desktop("Big", "big", &"#".repeat(70 * 1024)),
    );
    d.write("applications/ok.desktop", &desktop("Ok", "ok", ""));
    let catalog = Catalog::load_with(&env(&[&d])).unwrap();
    assert_eq!(names(&catalog, "o"), ["Ok"]);
    let missing = Environment {
        data_dirs: vec![d.0.join("nowhere")],
        ..Default::default()
    };
    assert!(matches!(
        Catalog::load_with(&missing),
        Err(CatalogError::NoApplicationDirectories)
    ));
    // An existing but empty applications directory is a valid, empty catalog.
    let empty = Dir::new();
    fs::create_dir_all(empty.0.join("applications")).unwrap();
    assert!(Catalog::load_with(&env(&[&empty])).unwrap().is_empty());
}

#[test]
fn hostile_directories_are_bounded_before_being_collected() {
    let d = Dir::new();
    d.write("applications/keep.desktop", &desktop("Keep", "keep", ""));
    // Thousands of irrelevant entries and nested directories must still count.
    for n in 0..300 {
        d.write(&format!("applications/junk{n}.txt"), "x");
    }
    for n in 0..60 {
        d.write(
            &format!("applications/sub{n}/deep.desktop"),
            &desktop("Deep", "deep", ""),
        );
    }
    let limited = Environment {
        limits: Limits {
            entries: 100,
            apps: 4096,
        },
        ..env(&[&d])
    };
    let catalog = Catalog::load_with(&limited).unwrap();
    assert!(catalog.truncated());
    assert!(catalog.len() < 60);
    let capped = Environment {
        limits: Limits {
            entries: 100_000,
            apps: 5,
        },
        ..env(&[&d])
    };
    let catalog = Catalog::load_with(&capped).unwrap();
    assert!(catalog.truncated());
    assert_eq!(catalog.len(), 5);
    assert!(!Catalog::load_with(&env(&[&d])).unwrap().truncated());
}

#[test]
fn special_files_are_skipped_instead_of_blocking_the_scan() {
    let d = Dir::new();
    d.write("applications/ok.desktop", &desktop("Ok", "ok", ""));
    fs::create_dir_all(d.0.join("applications")).unwrap();
    let fifo = d.0.join("applications/pipe.desktop");
    // A FIFO with no writer blocks forever on open.
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("skipping: mkfifo unavailable");
        return;
    }
    let catalog = Catalog::load_with(&env(&[&d])).unwrap();
    assert_eq!(names(&catalog, "o"), ["Ok"]);
}
