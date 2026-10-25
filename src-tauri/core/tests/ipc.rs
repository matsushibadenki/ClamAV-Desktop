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
async fn idle_shutdown_rejects_live_work_and_replays_the_busy_result() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    core.document.lock().unwrap().operation = Some(Default::default());
    let request = Request::new(Command::ShutdownIfIdle);
    assert_eq!(
        client.send(&request).await.unwrap().error.as_deref(),
        Some("busy")
    );
    assert!(!core.cancel.load(Ordering::Relaxed));
    core.document.lock().unwrap().operation = None;
    assert_eq!(
        client.send(&request).await.unwrap().error.as_deref(),
        Some("busy")
    );
    assert!(matches!(
        client.request(Command::GetSnapshot).await.unwrap(),
        Payload::Snapshot(_)
    ));
    client.request(Command::ShutdownIfIdle).await.unwrap();
    server.await.unwrap().unwrap();
    assert!(!fixture.0.join("worker.sock").exists());
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
    let finding_id = core.snapshot().history[0].findings[0].id.clone();
    let quarantine = Request::new(Command::QuarantineFile {
        id: finding_id.clone(),
    });
    client
        .send(&quarantine)
        .await
        .unwrap()
        .into_result()
        .unwrap();
    file_finished(&client, &quarantine).await.unwrap();
    assert!(!files.join("detected.txt").exists());
    let restore = Request::new(Command::RestoreFile {
        id: finding_id.clone(),
    });
    client.send(&restore).await.unwrap().into_result().unwrap();
    file_finished(&client, &restore).await.unwrap();
    assert_eq!(fs::read(files.join("detected.txt")).unwrap(), payload);
    // Retrying quarantine after restore must not remove the original again.
    client
        .send(&quarantine)
        .await
        .unwrap()
        .into_result()
        .unwrap();
    assert_eq!(fs::read(files.join("detected.txt")).unwrap(), payload);
    client.send(&restore).await.unwrap().into_result().unwrap();
    file_finished(&client, &restore).await.unwrap();
    assert_eq!(core.snapshot().quarantine[0].status, "restored");
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

#[tokio::test]
async fn definition_update_cooldown_is_enforced_and_replayed_without_network_access() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    clamav_desktop_core::diagnostics::record::<()>(
        &core,
        "definitions_update",
        &Err("HTTP 429 cool-down".into()),
    );
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    let request = Request::new(Command::UpdateDefinitions);
    assert_eq!(
        client
            .send(&request)
            .await
            .unwrap()
            .into_result()
            .err()
            .unwrap(),
        "diagnostic_retry_wait"
    );
    // Clear the cooldown to prove the retry uses its recorded rejection, not a new update.
    clamav_desktop_core::diagnostics::record(&core, "definitions_update", &Ok(()));
    assert_eq!(
        client
            .send(&request)
            .await
            .unwrap()
            .into_result()
            .err()
            .unwrap(),
        "diagnostic_retry_wait"
    );
    assert!(core.snapshot().operation.is_none());
    assert_eq!(
        core.snapshot()
            .diagnostics
            .iter()
            .find(|a| a.kind == "definitions_update")
            .unwrap()
            .status,
        "succeeded"
    );
    shutdown(&client, server).await;
}

async fn integrity_finished(
    client: &Client,
    request: &Request,
) -> clamav_desktop_core::store::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match client
                .request(Command::GetIntegrityResult {
                    request_id: request.id.clone(),
                })
                .await
                .unwrap()
            {
                Payload::IntegrityResult {
                    result: Some(result),
                } => return result,
                Payload::IntegrityResult { result: None } => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                _ => panic!("integrity result expected"),
            }
        }
    })
    .await
    .expect("integrity job did not finish")
}

#[tokio::test]
async fn integrity_ipc_verifies_replays_and_stops_without_partial_hash() {
    use clamav_desktop_core::{model::QuarantineItem, scanner, store::now};
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let id = "harmless-record".to_string();
    let stored = fixture.0.join("quarantine").join(&id);
    fs::write(&stored, b"harmless integrity fixture").unwrap();
    core.document
        .lock()
        .unwrap()
        .quarantine
        .push(QuarantineItem {
            id: id.clone(),
            original_path: fixture.0.join("original").to_string_lossy().into(),
            signature: "fixture".into(),
            digest: scanner::digest(&stored).unwrap(),
            quarantined_at: now(),
            mode: 0o600,
            status: "active".into(),
        });
    let server = start(core.clone()).await;
    let client = Client::new(fixture.0.clone()).unwrap();
    let request = Request::new(Command::VerifyQuarantineIntegrity { id: id.clone() });
    assert!(matches!(
        client.send(&request).await.unwrap().into_result().unwrap(),
        Payload::Ack
    ));
    integrity_finished(&client, &request).await.unwrap();
    let checked_at = match client
        .request(Command::GetQuarantineDetail { id: id.clone() })
        .await
        .unwrap()
    {
        Payload::QuarantineDetail(detail) => {
            let result = detail.integrity.unwrap();
            assert_eq!(result.state, "verified");
            result.checked_at
        }
        _ => panic!("detail expected"),
    };
    fs::write(&stored, b"changed fixture").unwrap();
    client.send(&request).await.unwrap().into_result().unwrap();
    match client
        .request(Command::GetQuarantineDetail { id: id.clone() })
        .await
        .unwrap()
    {
        Payload::QuarantineDetail(detail) => {
            assert_eq!(detail.integrity.unwrap().checked_at, checked_at)
        }
        _ => panic!("detail expected"),
    }
    // A large sparse file keeps the job active without storing a large fixture.
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&stored)
        .unwrap();
    file.set_len(1024 * 1024 * 1024).unwrap();
    drop(file);
    let check = Request::new(Command::VerifyQuarantineIntegrity { id: id.clone() });
    client.send(&check).await.unwrap().into_result().unwrap();
    assert_eq!(
        client
            .request(Command::GetQuarantineDetail { id: id.clone() })
            .await
            .err()
            .unwrap(),
        "busy"
    );
    match client.request(Command::GetSnapshot).await.unwrap() {
        Payload::Snapshot(snapshot) => assert_eq!(
            snapshot.operation.unwrap().target_id.as_deref(),
            Some(id.as_str())
        ),
        _ => panic!("snapshot expected"),
    }
    client.request(Command::StopIntegrityCheck).await.unwrap();
    integrity_finished(&client, &check).await.unwrap();
    match client
        .request(Command::GetQuarantineDetail { id: id.clone() })
        .await
        .unwrap()
    {
        Payload::QuarantineDetail(detail) => {
            let result = detail.integrity.unwrap();
            assert_eq!(result.state, "cancelled");
            assert!(result.actual_digest.is_none());
            assert!(result.bytes_checked < result.total_bytes.unwrap());
        }
        _ => panic!("detail expected"),
    }
    assert_eq!(fs::metadata(&stored).unwrap().len(), 1024 * 1024 * 1024);
    let missing = Request::new(Command::VerifyQuarantineIntegrity {
        id: "missing".into(),
    });
    for _ in 0..2 {
        assert_eq!(
            client
                .send(&missing)
                .await
                .unwrap()
                .into_result()
                .err()
                .unwrap(),
            "finding_missing"
        );
    }
    let check_shutdown = Request::new(Command::VerifyQuarantineIntegrity { id: id.clone() });
    client
        .send(&check_shutdown)
        .await
        .unwrap()
        .into_result()
        .unwrap();
    shutdown(&client, server).await;
    assert!(core.snapshot().operation.is_none());
    assert_eq!(
        clamav_desktop_core::details::get(&core, &id)
            .unwrap()
            .integrity
            .unwrap()
            .state,
        "cancelled"
    );
}

async fn file_finished(
    client: &Client,
    request: &Request,
) -> clamav_desktop_core::store::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
                } => return result,
                Payload::FileOperationResult { result: None } => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                _ => panic!("file result expected"),
            }
        }
    })
    .await
    .expect("file operation did not finish")
}

#[tokio::test]
async fn recovery_ipc_replays_without_repeating_new_journals() {
    use clamav_desktop_core::{
        file_transfer,
        model::QuarantineItem,
        scanner,
        store::{now, save_row},
    };
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.join("data")).unwrap());
    let source = fixture.0.join("original");
    fs::write(&source, b"harmless transfer fixture").unwrap();
    let stored = core.root.join("quarantine/recovered");
    let digest = scanner::digest(&source).unwrap();
    let item = QuarantineItem {
        id: "recovered".into(),
        original_path: source.to_string_lossy().into(),
        signature: "fixture".into(),
        digest: digest.clone(),
        quarantined_at: now(),
        mode: 0o600,
        status: "active".into(),
    };
    {
        let db = core.db.lock().unwrap();
        file_transfer::prepare(
            &db,
            &item.id,
            "quarantine",
            &source,
            &stored,
            &digest,
            |_| Ok(()),
        )
        .unwrap();
        save_row(&db, "quarantine", &item.id, &item).unwrap();
    }
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    let recover = Request::new(Command::RecoverFileTransfers);
    client.send(&recover).await.unwrap().into_result().unwrap();
    file_finished(&client, &recover).await.unwrap();
    assert_eq!(fs::read(&stored).unwrap(), b"harmless transfer fixture");
    assert!(!source.exists());
    assert_eq!(core.snapshot().quarantine[0].status, "active");
    let second_source = fixture.0.join("second");
    let second_dest = core.root.join("quarantine/uncommitted");
    fs::write(&second_source, b"uncommitted source").unwrap();
    file_transfer::prepare(
        &core.db.lock().unwrap(),
        "uncommitted",
        "quarantine",
        &second_source,
        &second_dest,
        &scanner::digest(&second_source).unwrap(),
        |_| Ok(()),
    )
    .unwrap();
    client.send(&recover).await.unwrap().into_result().unwrap();
    assert!(second_dest.exists());
    assert_eq!(
        core.db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM file_transfers", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    let fresh_recovery = Request::new(Command::RecoverFileTransfers);
    client
        .send(&fresh_recovery)
        .await
        .unwrap()
        .into_result()
        .unwrap();
    file_finished(&client, &fresh_recovery).await.unwrap();
    assert_eq!(fs::read(&second_source).unwrap(), b"uncommitted source");
    assert!(!second_dest.exists());
    let restore = Request::new(Command::RestoreFile {
        id: item.id.clone(),
    });
    fs::write(&source, b"existing destination").unwrap();
    client.send(&restore).await.unwrap().into_result().unwrap();
    assert_eq!(
        file_finished(&client, &restore).await.unwrap_err(),
        "restore_destination_exists"
    );
    fs::remove_file(&source).unwrap();
    client.send(&restore).await.unwrap().into_result().unwrap();
    assert_eq!(
        file_finished(&client, &restore).await.unwrap_err(),
        "restore_destination_exists"
    );
    assert!(!source.exists());
    let restore = Request::new(Command::RestoreFile { id: item.id });
    client.send(&restore).await.unwrap().into_result().unwrap();
    file_finished(&client, &restore).await.unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"harmless transfer fixture");
    shutdown(&client, server).await;
    drop(core);
    let reopened = Core::new(fixture.0.join("data")).unwrap();
    assert_eq!(reopened.snapshot().quarantine[0].status, "restored");
}

#[tokio::test]
async fn shutdown_waits_for_accepted_restore_and_retains_final_record() {
    use clamav_desktop_core::{
        model::QuarantineItem,
        scanner,
        store::{now, save_row},
    };
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.join("data")).unwrap());
    let stored = core.root.join("quarantine/large-restore");
    let original = fixture.0.join("restored-file");
    let file = fs::File::create(&stored).unwrap();
    file.set_len(32 * 1024 * 1024).unwrap();
    drop(file);
    let digest = scanner::digest(&stored).unwrap();
    let item = QuarantineItem {
        id: "large-restore".into(),
        original_path: original.to_string_lossy().into(),
        signature: "fixture".into(),
        digest: digest.clone(),
        quarantined_at: now(),
        mode: 0o600,
        status: "active".into(),
    };
    save_row(&core.db.lock().unwrap(), "quarantine", &item.id, &item).unwrap();
    core.document.lock().unwrap().quarantine.push(item.clone());
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    let restore = Request::new(Command::RestoreFile { id: item.id });
    client.send(&restore).await.unwrap().into_result().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        client.request(Command::GetSnapshot).await.unwrap();
        client.request(Command::StopScan).await.unwrap();
    })
    .await
    .expect("restore blocked IPC control requests");
    shutdown(&client, server).await;
    assert_eq!(scanner::digest(&original).unwrap(), digest);
    assert!(!stored.exists());
    assert!(core.snapshot().operation.is_none());
    assert_eq!(core.snapshot().quarantine[0].status, "restored");
    drop(core);
    let reopened = Core::new(fixture.0.join("data")).unwrap();
    assert_eq!(reopened.snapshot().quarantine[0].status, "restored");
}

#[tokio::test]
async fn cache_ipc_preserves_prepared_unknown_and_replayed_new_attempts() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.join("data")).unwrap());
    fn old_attempt(core: &Core) -> PathBuf {
        let dir = core
            .root
            .join("downloads")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&dir).unwrap();
        let pkg = dir.join("clamav-1.4.0.macos.universal.pkg");
        fs::write(&pkg, b"harmless cached fixture").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(90 * 86400);
        let times = fs::FileTimes::new().set_modified(old);
        fs::File::open(&pkg).unwrap().set_times(times).unwrap();
        fs::File::open(&dir).unwrap().set_times(times).unwrap();
        dir
    }
    let removable = old_attempt(&core);
    let prepared = old_attempt(&core);
    *core.prepared.lock().unwrap() = Some((
        prepared.join("clamav-1.4.0.macos.universal.pkg"),
        "fixture".into(),
    ));
    let unknown = core.root.join("downloads/keep.txt");
    fs::write(&unknown, b"keep").unwrap();
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    match client.request(Command::GetDownloadCache).await.unwrap() {
        Payload::DownloadCache(report) => {
            assert_eq!(report.removable, 1);
            assert_eq!(report.attempts, 2);
            assert_eq!(report.skipped, 1);
        }
        _ => panic!("cache report expected"),
    }
    let cleanup = Request::new(Command::CleanupDownloadCache);
    client.send(&cleanup).await.unwrap().into_result().unwrap();
    file_finished(&client, &cleanup).await.unwrap();
    assert!(!removable.exists());
    assert!(prepared.exists());
    assert_eq!(fs::read(&unknown).unwrap(), b"keep");
    let later = old_attempt(&core);
    client.send(&cleanup).await.unwrap().into_result().unwrap();
    assert!(later.exists());
    let next = Request::new(Command::CleanupDownloadCache);
    client.send(&next).await.unwrap().into_result().unwrap();
    file_finished(&client, &next).await.unwrap();
    assert!(!later.exists());
    shutdown(&client, server).await;
}

#[tokio::test]
async fn exports_ipc_write_private_json_without_large_response_or_replayed_writes() {
    use clamav_desktop_core::{
        exports::Kind,
        model::{Finding, QuarantineItem, Scan},
        scanner,
        store::now,
    };
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.join("data")).unwrap());
    let original = fixture.0.join("original");
    fs::write(&original, b"preserve original").unwrap();
    let item = QuarantineItem {
        id: "export-record".into(),
        original_path: original.to_string_lossy().into(),
        signature: "fixture".into(),
        digest: scanner::digest(&original).unwrap(),
        quarantined_at: now(),
        mode: 0o600,
        status: "active".into(),
    };
    core.document.lock().unwrap().quarantine.push(item);
    core.document.lock().unwrap().history.push(Scan {
        id: "scan".into(),
        kind: "custom".into(),
        target: original.to_string_lossy().into(),
        started_at: now(),
        finished_at: Some(now()),
        status: "completed".into(),
        scanned: 1,
        errors: 0,
        current_file: String::new(),
        findings: vec![Finding {
            id: "finding".into(),
            path: original.to_string_lossy().into(),
            signature: "fixture".into(),
            digest: None,
            quarantined: false,
        }],
        message: Some("x".repeat(clamav_desktop_core::ipc::RESPONSE_LIMIT + 1)),
    });
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    let history = fixture.0.join("history.json");
    let request = Request::new(Command::ExportData {
        kind: Kind::History,
        destination: history.to_string_lossy().into(),
        app_update: None,
    });
    assert!(matches!(
        client.send(&request).await.unwrap().into_result().unwrap(),
        Payload::Ack
    ));
    file_finished(&client, &request).await.unwrap();
    assert!(
        fs::metadata(&history).unwrap().len() > clamav_desktop_core::ipc::RESPONSE_LIMIT as u64
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    assert_eq!(json[0]["id"], "scan");
    assert_eq!(
        fs::metadata(&history).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&history, b"later user content").unwrap();
    client.send(&request).await.unwrap().into_result().unwrap();
    assert_eq!(fs::read(&history).unwrap(), b"later user content");
    core.document.lock().unwrap().history[0].message = None;
    for (kind, name) in [
        (Kind::Quarantine, "quarantine.json"),
        (Kind::Diagnostics, "diagnostics.json"),
    ] {
        let destination = fixture.0.join(name);
        let export = Request::new(Command::ExportData {
            kind,
            destination: destination.to_string_lossy().into(),
            app_update: Some(serde_json::json!({"phase":"idle", "currentVersion":"0.2.0"})),
        });
        client.send(&export).await.unwrap().into_result().unwrap();
        file_finished(&client, &export).await.unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        if name == "diagnostics.json" {
            assert_eq!(json["appUpdate"]["phase"], "idle");
            assert_eq!(json["state"]["history"][0]["id"], "scan");
        } else {
            assert_eq!(json["records"][0]["item"]["id"], "export-record");
        }
    }
    for destination in [original.clone(), core.root.join("state.db")] {
        let export = Request::new(Command::ExportData {
            kind: Kind::History,
            destination: destination.to_string_lossy().into(),
            app_update: None,
        });
        client.send(&export).await.unwrap().into_result().unwrap();
        assert_eq!(
            file_finished(&client, &export).await.unwrap_err(),
            "export_protected_path"
        );
    }
    let link = fixture.0.join("linked.json");
    std::os::unix::fs::symlink(&original, &link).unwrap();
    let linked = Request::new(Command::ExportData {
        kind: Kind::History,
        destination: link.to_string_lossy().into(),
        app_update: None,
    });
    client.send(&linked).await.unwrap().into_result().unwrap();
    assert_eq!(
        file_finished(&client, &linked).await.unwrap_err(),
        "export_protected_path"
    );
    let excessive = Request::new(Command::ExportData {
        kind: Kind::Diagnostics,
        destination: fixture.0.join("oversized.json").to_string_lossy().into(),
        app_update: Some(serde_json::json!({"notes":"x".repeat(50 * 1024)})),
    });
    assert_eq!(
        client.send(&excessive).await.err().unwrap(),
        "ipc_frame_too_large"
    );
    assert!(!fixture.0.join("oversized.json").exists());
    assert_eq!(fs::read(original).unwrap(), b"preserve original");
    let invalid = Request::new(Command::ExportData {
        kind: Kind::History,
        destination: "relative.json".into(),
        app_update: None,
    });
    assert_eq!(client.send(&invalid).await.err().unwrap(), "invalid_path");
    shutdown(&client, server).await;
}

#[tokio::test]
async fn engine_inspection_ipc_returns_metadata_and_does_not_reprobe_on_replay() {
    let fixture = Fixture::new();
    let core = Arc::new(Core::new(fixture.0.clone()).unwrap());
    let server = start(core.clone()).await;
    let client = Client::new(core.root.clone()).unwrap();
    let request = Request::new(Command::RefreshEngine);
    assert!(matches!(
        client.send(&request).await.unwrap().into_result().unwrap(),
        Payload::Ack
    ));
    let engine = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            match client
                .request(Command::GetEngineInspectionResult {
                    request_id: request.id.clone(),
                })
                .await
                .unwrap()
            {
                Payload::EngineInspectionResult {
                    result: Some(result),
                } => break result.unwrap(),
                Payload::EngineInspectionResult { result: None } => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                _ => panic!("engine result expected"),
            }
        }
    })
    .await
    .unwrap();
    assert!(engine.checked);
    assert!(core.snapshot().operation.is_none());
    assert!(core
        .snapshot()
        .diagnostics
        .iter()
        .any(|attempt| attempt.kind == "engine_inspection"));
    core.document.lock().unwrap().engine.checked = false;
    client.send(&request).await.unwrap().into_result().unwrap();
    assert!(
        !core.snapshot().engine.checked,
        "replayed request reran the probe"
    );
    match client
        .request(Command::GetEngineInspectionResult {
            request_id: request.id,
        })
        .await
        .unwrap()
    {
        Payload::EngineInspectionResult {
            result: Some(Ok(cached)),
        } => assert!(cached.checked),
        _ => panic!("cached result expected"),
    }
    shutdown(&client, server).await;
}
