use clamav_desktop_core::{
    ipc::{Client, Command, Payload, Request, Server, REQUEST_LIMIT},
    store::Core,
};
use std::{
    fs,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use tokio::{io::AsyncWriteExt, net::UnixStream};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(PathBuf::from("/private/tmp").join(format!("clamav-ipc-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
async fn start(core: Arc<Core>) -> tokio::task::JoinHandle<clamav_desktop_core::store::Result<()>> {
    let server = Server::bind(core).unwrap();
    tokio::spawn(server.run(std::future::pending()))
}
async fn shutdown(
    client: &Client,
    server: tokio::task::JoinHandle<clamav_desktop_core::store::Result<()>>,
) {
    client.request(Command::Shutdown).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(12), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn retries_are_persistent_and_do_not_repeat_mutations() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    let request = Request::new(Command::StopScan);
    client.send(&request).await.unwrap().into_result().unwrap();
    assert!(core.cancel.load(Ordering::Relaxed));
    core.cancel.store(false, Ordering::Relaxed);
    client.send(&request).await.unwrap().into_result().unwrap();
    assert!(
        !core.cancel.load(Ordering::Relaxed),
        "retry repeated a mutation"
    );
    let mut conflict = request.clone();
    conflict.body = Command::Shutdown;
    assert_eq!(
        client.send(&conflict).await.unwrap().error.as_deref(),
        Some("ipc_request_id_reused")
    );
    shutdown(&client, server).await;
    assert!(!fixture.0.join("worker.sock").exists());
    drop(core);
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    client.send(&request).await.unwrap().into_result().unwrap();
    assert!(
        !core.cancel.load(Ordering::Relaxed),
        "restart lost idempotency"
    );
    shutdown(&client, server).await;
}
#[tokio::test]
async fn ambiguous_claims_are_never_replayed_and_ledger_is_bounded() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    let request = Request::new(Command::StopScan);
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO ipc_requests(id,request,response,expires_at) VALUES (?1,?2,NULL,?3)",
            rusqlite::params![
                request.id,
                serde_json::to_string(&request).unwrap(),
                chrono::Utc::now().timestamp() + 3600
            ],
        )
        .unwrap();
    }
    assert_eq!(
        client.send(&request).await.unwrap().error.as_deref(),
        Some("ipc_outcome_unknown")
    );
    assert!(!core.cancel.load(Ordering::Relaxed));
    {
        let db = core.db.lock().unwrap();
        for i in 0..1023 {
            db.execute(
                "INSERT INTO ipc_requests(id,request,response,expires_at) VALUES (?1,'{}',NULL,?2)",
                rusqlite::params![
                    format!("fixture-{i}"),
                    chrono::Utc::now().timestamp() + 3600
                ],
            )
            .unwrap();
        }
    }
    assert_eq!(
        client.request(Command::StopScan).await.err().unwrap(),
        "ipc_request_limit"
    );
    assert!(matches!(
        client.request(Command::GetSnapshot).await.unwrap(),
        Payload::Snapshot(_)
    ));
    core.db
        .lock()
        .unwrap()
        .execute("DELETE FROM ipc_requests", [])
        .unwrap();
    shutdown(&client, server).await;
}
#[tokio::test]
async fn malformed_oversized_and_partial_frames_do_not_block_other_clients() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let path = fixture.0.join("worker.sock");
    for bytes in [
        vec![0xff],
        serde_json::to_vec(&serde_json::json!({"command":"run_shell"})).unwrap(),
    ] {
        let mut stream = UnixStream::connect(&path).await.unwrap();
        stream.write_u32(bytes.len() as u32).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();
    }
    let mut stream = UnixStream::connect(&path).await.unwrap();
    stream.write_u32(REQUEST_LIMIT as u32 + 1).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut partial = UnixStream::connect(&path).await.unwrap();
    partial.write_all(&[0, 0]).await.unwrap();
    let client = Client::new(fixture.0.clone()).unwrap();
    assert!(matches!(
        client.request(Command::GetSnapshot).await.unwrap(),
        Payload::Snapshot(_)
    ));
    assert!(!core.cancel.load(Ordering::Relaxed));
    drop(partial);
    shutdown(&client, server).await;
}
#[tokio::test]
async fn unsafe_socket_is_preserved_and_one_core_cannot_bind_twice() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let path = core.root.join("worker.sock");
    let target = core.root.join("unchanged");
    fs::write(&target, b"keep").unwrap();
    symlink(&target, &path).unwrap();
    assert_eq!(
        Server::bind(core.clone()).err().unwrap(),
        "ipc_invalid_socket"
    );
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::remove_file(&path).unwrap();
    let server = Server::bind(core.clone()).unwrap();
    assert_eq!(
        Server::bind(core.clone()).err().unwrap(),
        "worker_already_running"
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    let client = Client::new(core.root.clone()).unwrap();
    assert_eq!(
        client.request(Command::GetSnapshot).await.err().unwrap(),
        "ipc_invalid_socket"
    );
    drop(server);
    assert!(!path.exists());
    let server = Server::bind(core.clone()).unwrap();
    drop(server);
}

#[tokio::test]
#[ignore = "requires installed ClamAV; isolated harmless custom signature via IPC"]
async fn actual_scan_is_not_repeated_when_start_reply_is_retried() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.0).unwrap();
    let files = fixture.0.join("files");
    fs::create_dir(&files).unwrap();
    fs::write(files.join("clean.txt"), b"ordinary harmless document").unwrap();
    let payload = b"CLAMAV_DESKTOP_TEST_DETECTION_PAYLOAD";
    fs::write(files.join("detected.txt"), payload).unwrap();
    let signature = fixture.0.join("test.ndb");
    fs::write(
        &signature,
        format!("ClamAV-Desktop-Test:0:*:{}\n", hex::encode(payload)),
    )
    .unwrap();
    let core = Arc::new(Core::new(fixture.0.join("data")).unwrap());
    {
        let mut doc = core.document.lock().unwrap();
        doc.engine.version = Some("local fixture".into());
        doc.engine.definitions = true;
        doc.engine.database_path = Some(signature.to_string_lossy().into());
    }
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    let request = Request::new(Command::StartScan {
        kind: "custom".into(),
        target: Some(files.to_string_lossy().into()),
    });
    let id = match client.send(&request).await.unwrap().into_result().unwrap() {
        Payload::ScanStarted { scan_id } => scan_id,
        _ => panic!("invalid start reply"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            match client.request(Command::GetSnapshot).await.unwrap() {
                Payload::Snapshot(snapshot) if !snapshot.history.is_empty() => {
                    assert_eq!(snapshot.history[0].status, "completed");
                    assert_eq!(snapshot.history[0].scanned, 2);
                    assert_eq!(snapshot.history[0].findings.len(), 1);
                    break;
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .unwrap();
    match client.send(&request).await.unwrap().into_result().unwrap() {
        Payload::ScanStarted { scan_id } => assert_eq!(id, scan_id),
        _ => panic!("invalid retry reply"),
    }
    assert_eq!(core.snapshot().history.len(), 1);
    shutdown(&client, server).await;
}

#[tokio::test]
async fn oversized_snapshot_returns_a_bounded_error_reply() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    core.document.lock().unwrap().engine.version =
        Some("x".repeat(clamav_desktop_core::ipc::RESPONSE_LIMIT));
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    assert_eq!(
        client.request(Command::GetSnapshot).await.err().unwrap(),
        "ipc_frame_too_large"
    );
    core.document.lock().unwrap().engine.version = None;
    shutdown(&client, server).await;
}

#[tokio::test]
async fn settings_save_is_validated_persistent_and_retry_does_not_overwrite_newer_values() {
    use clamav_desktop_core::model::Settings;
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    let first = Request::new(Command::SaveSettings {
        settings: Settings {
            language: "en".into(),
            download_retention_days: 1,
            download_cache_mb: 257,
            ..Settings::default()
        },
    });
    assert!(matches!(
        client.send(&first).await.unwrap().into_result().unwrap(),
        Payload::Ack
    ));
    let next = Settings {
        language: "zh-CN".into(),
        schedule_hour: 23,
        max_file_mb: 4096,
        download_retention_days: 365,
        download_cache_mb: 16384,
        ..Settings::default()
    };
    client
        .request(Command::SaveSettings {
            settings: next.clone(),
        })
        .await
        .unwrap();
    // A delayed retry of an older save must return its stored acknowledgement only.
    client.send(&first).await.unwrap().into_result().unwrap();
    assert_eq!(core.snapshot().settings.language, "zh-CN");
    let bad = Settings {
        language: "unsupported".into(),
        ..next.clone()
    };
    assert_eq!(
        client
            .request(Command::SaveSettings { settings: bad })
            .await
            .err()
            .unwrap(),
        "invalid_settings"
    );
    match client.request(Command::GetSnapshot).await.unwrap() {
        Payload::Snapshot(state) => assert_eq!(
            serde_json::to_value(state.settings).unwrap(),
            serde_json::to_value(&next).unwrap()
        ),
        _ => panic!("snapshot expected"),
    }
    shutdown(&client, server).await;
    drop(core);
    let reopened = Core::new(fixture.0.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.snapshot().settings).unwrap(),
        serde_json::to_value(next).unwrap()
    );
}
