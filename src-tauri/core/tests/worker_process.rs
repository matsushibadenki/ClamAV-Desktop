use clamav_desktop_core::{
    ipc::{Client, Command, Payload},
    store::Core,
};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(PathBuf::from("/private/tmp").join(format!("clamav-worker-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn start(root: &PathBuf) -> (Worker, Client) {
    let mut worker = Worker(
        ProcessCommand::new(env!("CARGO_BIN_EXE_clamav-worker"))
            .args(["serve", "--data-dir"])
            .arg(root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let client = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(
                worker.0.try_wait().unwrap().is_none(),
                "worker exited during startup"
            );
            if let Ok(client) = Client::new(root.clone()) {
                if client.request(Command::GetSnapshot).await.is_ok() {
                    break client;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    (worker, client)
}
async fn wait(worker: &mut Worker) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = worker.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn standalone_worker_survives_clients_and_releases_ownership_on_shutdown() {
    let fixture = Fixture::new();
    let (mut worker, client) = start(&fixture.0).await;
    assert_eq!(Core::new(fixture.0.clone()).err().unwrap(), "data_in_use");
    drop(client);
    let client = Client::new(fixture.0.clone()).unwrap();
    assert!(matches!(
        client.request(Command::GetSnapshot).await.unwrap(),
        Payload::Snapshot(_)
    ));
    client.request(Command::StopScan).await.unwrap();
    client.request(Command::Shutdown).await.unwrap();
    wait(&mut worker).await;
    assert!(!fixture.0.join("worker.sock").exists());
    let _core = Core::new(fixture.0.clone()).unwrap();
}
#[tokio::test]
async fn killed_worker_leaves_recoverable_socket_and_can_restart() {
    let fixture = Fixture::new();
    let (mut worker, client) = start(&fixture.0).await;
    worker.0.kill().unwrap();
    worker.0.wait().unwrap();
    assert!(client.request(Command::GetSnapshot).await.is_err());
    assert!(fixture.0.join("worker.sock").exists());
    let (mut replacement, client) = start(&fixture.0).await;
    client.request(Command::Shutdown).await.unwrap();
    wait(&mut replacement).await;
    assert!(!fixture.0.join("worker.sock").exists());
}
#[test]
fn executable_rejects_relative_roots_and_environment_overrides() {
    let fixture = Fixture::new();
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_clamav-worker"))
        .args(["serve", "--data-dir", "relative-root"])
        .env("CLAMAV_DESKTOP_DEV_DATA_DIR", &fixture.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("absolute_data_directory_required"));
    assert!(!fixture.0.exists());
}

#[tokio::test]
async fn worker_evaluates_saved_schedule_without_a_gui() {
    use chrono::Timelike;
    let fixture = Fixture::new();
    let root = fixture.0.join("data");
    let core = Core::new(root.clone()).unwrap();
    clamav_desktop_core::settings::save(
        &core,
        clamav_desktop_core::model::Settings {
            schedule_enabled: true,
            schedule_hour: chrono::Local::now().hour(),
            ..Default::default()
        },
    )
    .unwrap();
    drop(core);
    // A dedicated HOME without Downloads prevents any access to real user files.
    let mut worker = Worker(
        ProcessCommand::new(env!("CARGO_BIN_EXE_clamav-worker"))
            .args(["serve", "--data-dir"])
            .arg(&root)
            .env("HOME", fixture.0.join("home"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = Client::new(root.clone()).unwrap();
    let record = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            assert!(worker.0.try_wait().unwrap().is_none());
            if let Ok(Payload::Snapshot(snapshot)) = client.request(Command::GetSnapshot).await {
                if let Some(record) = snapshot.scheduled_run {
                    break record;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(record.kind, "quick");
    assert_ne!(record.status, "completed");
    client.request(Command::Shutdown).await.unwrap();
    wait(&mut worker).await;
    let reopened = Core::new(root).unwrap();
    assert!(reopened.snapshot().scheduled_run.is_some());
    assert!(reopened.snapshot().history.is_empty());
}

#[tokio::test]
#[ignore = "requires local ClamAV and definitions; scans only a dedicated temporary HOME"]
async fn actual_worker_runs_scheduled_quick_scan_and_retains_the_daily_claim() {
    use chrono::Timelike;
    let fixture = Fixture::new();
    let root = fixture.0.join("data");
    let home = fixture.0.join("home");
    fs::create_dir_all(home.join("Downloads")).unwrap();
    fs::write(
        home.join("Downloads/harmless.txt"),
        b"An ordinary scheduler test document.",
    )
    .unwrap();
    let core = Core::new(root.clone()).unwrap();
    let engine = clamav_desktop_core::engine::inspect(&core).await.unwrap();
    assert!(
        engine.version.is_some() && engine.definitions,
        "installed engine and definitions required"
    );
    clamav_desktop_core::settings::save(
        &core,
        clamav_desktop_core::model::Settings {
            schedule_enabled: true,
            schedule_hour: chrono::Local::now().hour(),
            ..Default::default()
        },
    )
    .unwrap();
    drop(core);
    let mut worker = Worker(
        ProcessCommand::new(env!("CARGO_BIN_EXE_clamav-worker"))
            .args(["serve", "--data-dir"])
            .arg(&root)
            .env("HOME", &home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = Client::new(root.clone()).unwrap();
    let record = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            assert!(worker.0.try_wait().unwrap().is_none());
            if let Ok(Payload::Snapshot(snapshot)) = client.request(Command::GetSnapshot).await {
                if let Some(record) = snapshot.scheduled_run {
                    if matches!(
                        record.status.as_str(),
                        "completed" | "failed" | "missed" | "incomplete"
                    ) {
                        break record;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(record.status, "completed", "{record:?}");
    client.request(Command::ShutdownIfIdle).await.unwrap();
    wait(&mut worker).await;
    let reopened = Core::new(root).unwrap();
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.scheduled_run.unwrap().status, "completed");
    assert_eq!(snapshot.history.len(), 1);
    assert_eq!(snapshot.history[0].scanned, 1);
    assert_eq!(
        snapshot.history[0].target,
        home.join("Downloads").to_str().unwrap()
    );
    assert_eq!(snapshot.history[0].id, record.scan_id.unwrap());
    assert!(snapshot.quarantine.is_empty());
    assert!(home.join("Downloads/harmless.txt").exists());
}

#[tokio::test]
async fn diagnostics_identify_live_worker_and_preserve_instances_across_restart() {
    use clamav_desktop_core::{exports::Kind, ipc::Request};
    use std::os::unix::fs::PermissionsExt;
    async fn export(client: &Client, path: &std::path::Path) -> serde_json::Value {
        let request = Request::new(Command::ExportData {
            kind: Kind::Diagnostics,
            destination: path.to_string_lossy().into(),
            app_update: None,
        });
        assert!(matches!(
            client.send(&request).await.unwrap().into_result().unwrap(),
            Payload::Ack
        ));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match client
                    .request(Command::GetFileOperationResult {
                        request_id: request.id.clone(),
                    })
                    .await
                    .unwrap()
                {
                    Payload::FileOperationResult {
                        result: Some(result),
                    } => {
                        result.unwrap();
                        break;
                    }
                    Payload::FileOperationResult { result: None } => (),
                    _ => panic!("file operation result expected"),
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }
    let fixture = Fixture::new();
    let root = fixture.0.join("data");
    let (mut first, client) = start(&root).await;
    let one = export(&client, &fixture.0.join("first.json")).await;
    let instance = &one["workerRuntime"]["current"];
    assert_eq!(instance["pid"], first.0.id());
    assert_eq!(instance["parentPidAtReady"], std::process::id());
    assert_eq!(instance["journalPersisted"], true);
    assert!(chrono::DateTime::parse_from_rfc3339(instance["readyAt"].as_str().unwrap()).is_ok());
    assert_eq!(one["workerRuntime"]["recent"].as_array().unwrap().len(), 1);
    client.request(Command::Shutdown).await.unwrap();
    wait(&mut first).await;
    let (mut second, client) = start(&root).await;
    let two = export(&client, &fixture.0.join("second.json")).await;
    assert_eq!(two["workerRuntime"]["current"]["pid"], second.0.id());
    assert_ne!(
        instance["instanceId"],
        two["workerRuntime"]["current"]["instanceId"]
    );
    assert_eq!(two["workerRuntime"]["recent"].as_array().unwrap().len(), 2);
    assert_eq!(two["workerRuntime"]["recent"][1], *instance);
    client.request(Command::Shutdown).await.unwrap();
    wait(&mut second).await;
    let local = Core::new(root).unwrap();
    let local_report =
        serde_json::to_value(clamav_desktop_core::diagnostics::report(&local).unwrap()).unwrap();
    assert!(local_report["workerRuntime"]["current"].is_null());
    assert_eq!(
        local_report["workerRuntime"]["recent"],
        two["workerRuntime"]["recent"]
    );
}
