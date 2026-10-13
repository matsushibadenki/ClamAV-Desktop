//! Real subprocess tests: neither a Mutex nor a same-process test proves ownership.
use clamav_desktop_core::{
    model::Scan,
    store::{now, Core, StateNotifier},
};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("clamav-ownership-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn scan() -> Scan {
    Scan {
        id: "ownership-test".into(),
        kind: "quick".into(),
        target: "/tmp".into(),
        started_at: now(),
        finished_at: None,
        status: "running".into(),
        scanned: 7,
        errors: 0,
        current_file: String::new(),
        findings: vec![],
        message: None,
    }
}
fn child(root: &std::path::Path, action: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "ownership_child", "--nocapture"])
        .env("CLAMAV_OWNERSHIP_TEST_ROOT", root)
        .env("CLAMAV_OWNERSHIP_TEST_ACTION", action);
    cmd
}
struct HeldChild(Child);
impl Drop for HeldChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn ownership_child() {
    let Some(root) = std::env::var_os("CLAMAV_OWNERSHIP_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    match std::env::var("CLAMAV_OWNERSHIP_TEST_ACTION")
        .unwrap()
        .as_str()
    {
        "blocked" => assert_eq!(Core::new(root).err().unwrap(), "data_in_use"),
        "hold" => {
            let c = Core::new(root.clone()).unwrap();
            c.save_scan(&scan()).unwrap();
            fs::write(root.join("ready"), b"ready").unwrap();
            let _ = std::io::stdin().read(&mut [0u8]);
            drop(c);
        }
        other => panic!("unknown child action: {other}"),
    }
}
#[test]
fn second_process_cannot_initialize_or_recover_live_state() {
    let fixture = Fixture::new();
    let c = Core::new(fixture.0.clone()).unwrap();
    c.save_scan(&scan()).unwrap();
    // A contender must not even parse a journal while another owner is active.
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO file_transfers(id,json) VALUES ('sentinel','invalid-json')",
            [],
        )
        .unwrap();
    let before: String =
        c.db.lock()
            .unwrap()
            .query_row("SELECT json FROM scans", [], |r| r.get(0))
            .unwrap();
    let output = child(&fixture.0, "blocked").output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let after: String =
        c.db.lock()
            .unwrap()
            .query_row("SELECT json FROM scans", [], |r| r.get(0))
            .unwrap();
    assert_eq!(before, after);
    let journal: String =
        c.db.lock()
            .unwrap()
            .query_row("SELECT json FROM file_transfers", [], |r| r.get(0))
            .unwrap();
    assert_eq!(journal, "invalid-json");
    c.db.lock()
        .unwrap()
        .execute("DELETE FROM file_transfers", [])
        .unwrap();
}
#[test]
fn crash_releases_ownership_and_recovery_runs_once() {
    let fixture = Fixture::new();
    let mut held = HeldChild(
        child(&fixture.0, "hold")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fixture.0.join("ready").exists() {
        assert!(
            held.0.try_wait().unwrap().is_none(),
            "owner exited before ready"
        );
        assert!(Instant::now() < deadline, "owner readiness timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(Core::new(fixture.0.clone()).err().unwrap(), "data_in_use");
    held.0.kill().unwrap();
    held.0.wait().unwrap();
    let c = Core::new(fixture.0.clone()).unwrap();
    let first = c.snapshot().history.remove(0);
    assert_eq!(first.status, "interrupted");
    assert_eq!(first.scanned, 7);
    let lock_inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(fixture.0.join("owner.lock")).unwrap().ino()
    };
    drop(c);
    let c = Core::new(fixture.0.clone()).unwrap();
    let second = c.snapshot().history.remove(0);
    assert_eq!(first.finished_at, second.finished_at);
    assert_eq!(first.message, second.message);
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        lock_inode,
        fs::metadata(fixture.0.join("owner.lock")).unwrap().ino()
    );
}
#[test]
fn failed_initialization_releases_ownership() {
    let fixture = Fixture::new();
    let c = Core::new(fixture.0.clone()).unwrap();
    c.db.lock()
        .unwrap()
        .execute(
            "INSERT INTO scans(id,json) VALUES ('bad','invalid-json')",
            [],
        )
        .unwrap();
    drop(c);
    assert_ne!(Core::new(fixture.0.clone()).err().unwrap(), "data_in_use");
    let db = rusqlite::Connection::open(fixture.0.join("state.sqlite")).unwrap();
    db.execute("DELETE FROM scans", []).unwrap();
    drop(db);
    let _c = Core::new(fixture.0.clone()).unwrap();
}
#[test]
fn aliases_share_ownership_and_unsafe_lock_files_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::new();
    fs::create_dir(&fixture.0).unwrap();
    let root = fixture.0.join("data");
    let c = Core::new(root.clone()).unwrap();
    let alias = fixture.0.join("alias");
    symlink(&root, &alias).unwrap();
    assert_eq!(Core::new(alias).err().unwrap(), "data_in_use");
    drop(c);
    let lock = root.join("owner.lock");
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        Core::new(root.clone()).err().unwrap(),
        "invalid_ownership_lock"
    );
    fs::remove_file(&lock).unwrap();
    let target = fixture.0.join("target");
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, &lock).unwrap();
    assert!(Core::new(root.clone())
        .err()
        .unwrap()
        .starts_with("ownership_lock_failed:"));
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    fs::remove_file(&lock).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&target, &lock).unwrap();
    assert_eq!(Core::new(root).err().unwrap(), "invalid_ownership_lock");
}
#[test]
fn notifications_work_without_tauri_and_allow_snapshot_reads() {
    use clamav_desktop_core::model::Snapshot;
    struct Recorder {
        values: Mutex<Vec<Snapshot>>,
        owner: Mutex<std::sync::Weak<Core>>,
    }
    impl StateNotifier for Recorder {
        fn publish(&self, snapshot: Snapshot) {
            // Callbacks may read the Core again; publish must not retain its document lock.
            let core = self.owner.lock().unwrap().upgrade().unwrap();
            assert_eq!(core.snapshot().activity.len(), snapshot.activity.len());
            self.values.lock().unwrap().push(snapshot);
        }
    }
    let fixture = Fixture::new();
    let recorder = Arc::new(Recorder {
        values: Mutex::new(vec![]),
        owner: Mutex::new(std::sync::Weak::new()),
    });
    let c = Arc::new(Core::with_notifier(fixture.0.clone(), recorder.clone()).unwrap());
    *recorder.owner.lock().unwrap() = Arc::downgrade(&c);
    c.log("test", "headless state notification");
    let values = recorder.values.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].activity[0].detail, "headless state notification");
    assert_eq!(c.snapshot().activity[0].code, "test");
}
