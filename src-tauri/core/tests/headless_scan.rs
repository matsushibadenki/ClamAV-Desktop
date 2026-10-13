use clamav_desktop_core::{
    details,
    model::Snapshot,
    quarantine, scanner,
    store::{Core, StateNotifier},
};
use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

#[tokio::test]
#[ignore = "requires installed ClamAV; harmless isolated custom-signature fixtures"]
async fn scan_quarantine_verify_restore_without_gui() {
    struct Recorder(Mutex<Vec<String>>);
    impl StateNotifier for Recorder {
        fn publish(&self, snapshot: Snapshot) {
            if let Some(activity) = snapshot.activity.first() {
                self.0.lock().unwrap().push(activity.code.clone());
            }
        }
    }
    let root = std::env::temp_dir().join(format!("clamav-headless-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let target = root.join("files");
    fs::create_dir(&target).unwrap();
    let payload = b"CLAMAV_DESKTOP_TEST_DETECTION_PAYLOAD";
    fs::write(
        target.join("clean: file.txt"),
        b"ordinary harmless document",
    )
    .unwrap();
    fs::write(target.join("detected.txt"), payload).unwrap();
    let signature = root.join("test.ndb");
    fs::write(
        &signature,
        format!("ClamAV-Desktop-Test:0:*:{}\n", hex::encode(payload)),
    )
    .unwrap();
    let recorder = Arc::new(Recorder(Mutex::new(vec![])));
    let c = Arc::new(Core::with_notifier(root.join("data"), recorder.clone()).unwrap());
    {
        let mut d = c.document.lock().unwrap();
        d.engine.version = Some("local fixture".into());
        d.engine.definitions = true;
        d.engine.path = Some(
            clamav_desktop_core::engine::discover()
                .expect("ClamAV required")
                .to_string_lossy()
                .into(),
        );
        d.engine.database_path = Some(signature.to_string_lossy().into());
    }
    let id = scanner::start(
        c.clone(),
        "custom".into(),
        Some(target.to_string_lossy().into()),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if c.snapshot().history.iter().any(|s| s.id == id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let scan = c.snapshot().history.remove(0);
    assert_eq!(scan.status, "completed");
    assert_eq!(scan.scanned, 2);
    assert_eq!(scan.errors, 0);
    assert_eq!(scan.findings.len(), 1);
    let finding = &scan.findings[0];
    quarantine::quarantine(&c, &finding.id).unwrap();
    assert!(!target.join("detected.txt").exists());
    details::verify(&c, &finding.id).unwrap();
    assert_eq!(
        details::get(&c, &finding.id)
            .unwrap()
            .integrity
            .unwrap()
            .state,
        "verified"
    );
    quarantine::restore(&c, &finding.id).unwrap();
    assert_eq!(fs::read(target.join("detected.txt")).unwrap(), payload);
    assert_eq!(c.snapshot().quarantine[0].status, "restored");
    let codes = recorder.0.lock().unwrap().clone();
    for code in [
        "scan_started",
        "scan_finished",
        "file_quarantined",
        "integrity_checked",
        "file_restored",
    ] {
        assert!(
            codes.iter().any(|s| s == code),
            "missing notification: {code}"
        );
    }
    drop(c);
    let reopened = Core::new(root.join("data")).unwrap();
    assert_eq!(reopened.snapshot().history[0].status, "completed");
    assert_eq!(reopened.snapshot().quarantine[0].status, "restored");
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}
