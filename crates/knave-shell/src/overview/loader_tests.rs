use super::*;

fn catalog(loader: &Loader) -> Catalog {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for done in loader.take() {
            if let Done::Catalog(catalog) = done {
                return catalog.unwrap();
            }
        }
        assert!(Instant::now() < deadline, "catalog was not delivered");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn watch_coalesces_bursts_ignores_reads_and_stops_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let apps = root.path().join("applications");
    std::fs::create_dir(&apps).unwrap();
    let mut loader = Loader::start(IconLookup::from_process(), None);
    assert!(loader.submit(Job::Catalog(Environment {
        data_dirs: vec![root.path().into()],
        ..Default::default()
    })));
    assert_eq!(catalog(&loader).len(), 0);
    for i in 0..200 {
        std::fs::write(
            apps.join(format!("app{i}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=App {i}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    assert_eq!(catalog(&loader).len(), 200);
    // Reads made by the scanner or another program must not trigger a rescan loop.
    for entry in std::fs::read_dir(&apps).unwrap() {
        std::fs::read_to_string(entry.unwrap().path()).unwrap();
    }
    std::fs::write(root.path().join("unrelated"), "ignored").unwrap();
    thread::sleep(Duration::from_millis(400));
    assert!(
        !loader.has_results(),
        "read events or unrelated writes triggered a scan"
    );
    // The same bounded slot holds only the latest catalog while the UI is hidden.
    for i in 0..3 {
        std::fs::remove_file(apps.join(format!("app{i}.desktop"))).unwrap();
        thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(loader.done.lock().unwrap().len(), 1);
    assert_eq!(catalog(&loader).len(), 197);
    let worker = loader.thread.take().unwrap();
    drop(loader);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(Instant::now() < deadline, "watch worker did not stop");
        thread::sleep(Duration::from_millis(10));
    }
    worker.join().unwrap();
}

/// Run alone so other test workers cannot contaminate process resource samples.
#[test]
#[ignore = "isolated resource measurement"]
fn application_watch_resource_sample() {
    fn sample(label: &str) {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        let mut switches = 0u64;
        let mut tasks = 0;
        for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
            if let Ok(status) = std::fs::read_to_string(task.path().join("status")) {
                tasks += 1;
                switches += status
                    .lines()
                    .filter(|s| s.contains("ctxt_switches:"))
                    .map(|s| s.split_whitespace().last().unwrap().parse::<u64>().unwrap())
                    .sum::<u64>();
            }
        }
        println!(
            "{label}: threads={tasks}, fds={}, {}, cpu_ticks={}+{}, context_switches={switches}",
            std::fs::read_dir("/proc/self/fd").unwrap().count(),
            status.lines().find(|s| s.starts_with("VmRSS:")).unwrap(),
            fields[11],
            fields[12]
        );
    }
    let root = tempfile::tempdir().unwrap();
    let apps = root.path().join("applications");
    std::fs::create_dir(&apps).unwrap();
    sample("baseline");
    let mut loader = Loader::start(IconLookup::from_process(), None);
    assert!(loader.submit(Job::Catalog(Environment {
        data_dirs: vec![root.path().into()],
        ..Default::default()
    })));
    assert!(catalog(&loader).is_empty());
    sample("watch ready");
    thread::sleep(Duration::from_secs(1));
    sample("watch idle 1s");
    assert!(!loader.has_results());
    let start = Instant::now();
    for i in 0..1000 {
        std::fs::write(
            apps.join(format!("app{i}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=App {i}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    assert_eq!(catalog(&loader).len(), 1000);
    println!("1000-entry burst completed in {:?}", start.elapsed());
    sample("after burst");
    thread::sleep(Duration::from_secs(1));
    sample("post-burst idle 1s");
    assert!(!loader.has_results());
    let worker = loader.thread.take().unwrap();
    drop(loader);
    worker.join().unwrap();
    sample("after drop");
}
