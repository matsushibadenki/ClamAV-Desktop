//! Tauri adapter and worker proxy. Windows never own business state or SQLite.
use clamav_desktop_core::{
    details,
    ipc::{Client, Command, Payload, Request},
    model::{Settings, Snapshot},
    store::{Core, Result, StateNotifier},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager};

pub struct TauriNotifier(pub AppHandle);
impl StateNotifier for TauriNotifier {
    fn publish(&self, snapshot: Snapshot) {
        let _ = self.0.emit("core:state", snapshot);
    }
}
pub enum Backend {
    Local(Arc<Core>),
    Worker {
        client: Client,
        snapshot: Mutex<Snapshot>,
    },
}
impl Backend {
    pub async fn refresh_engine(&self) -> Result<clamav_desktop_core::model::Engine> {
        match self {
            Self::Local(core) => clamav_desktop_core::engine::refresh(core).await,
            Self::Worker { client, .. } => {
                let request = Request::new(Command::RefreshEngine);
                match client.send(&request).await?.into_result()? {
                    Payload::Ack => (),
                    _ => return Err("ipc_invalid_response".into()),
                }
                loop {
                    match client
                        .request(Command::GetEngineInspectionResult {
                            request_id: request.id.clone(),
                        })
                        .await?
                    {
                        Payload::EngineInspectionResult {
                            result: Some(result),
                        } => return result,
                        Payload::EngineInspectionResult { result: None } => (),
                        _ => return Err("ipc_invalid_response".into()),
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
    pub async fn download_cache(&self) -> Result<clamav_desktop_core::downloads::CacheReport> {
        match self {
            Self::Local(core) => {
                let core = core.clone();
                tokio::task::spawn_blocking(move || {
                    let _gate = core.file_gate.try_lock().map_err(|_| "busy")?;
                    clamav_desktop_core::downloads::report(&core)
                })
                .await
                .map_err(clamav_desktop_core::store::err)?
            }
            Self::Worker { client, .. } => match client.request(Command::GetDownloadCache).await? {
                Payload::DownloadCache(report) => Ok(*report),
                _ => Err("ipc_invalid_response".into()),
            },
        }
    }
    pub async fn file_operation(
        &self,
        action: clamav_desktop_core::quarantine::FileAction,
    ) -> Result<()> {
        use clamav_desktop_core::quarantine::{self, FileAction};
        match self {
            Self::Local(core) => {
                let core = core.clone();
                tokio::task::spawn_blocking(move || {
                    quarantine::run_file_operation(&core, action, || {})
                })
                .await
                .map_err(clamav_desktop_core::store::err)?
            }
            Self::Worker { client, .. } => {
                let command = match action {
                    FileAction::Quarantine(id) => Command::QuarantineFile { id },
                    FileAction::Restore(id) => Command::RestoreFile { id },
                    FileAction::Recover => Command::RecoverFileTransfers,
                    FileAction::CleanupCache => Command::CleanupDownloadCache,
                    FileAction::Export {
                        kind,
                        destination,
                        app_update,
                    } => Command::ExportData {
                        kind,
                        destination,
                        app_update,
                    },
                };
                let request = Request::new(command);
                match client.send(&request).await?.into_result()? {
                    Payload::Ack => (),
                    _ => return Err("ipc_invalid_response".into()),
                }
                loop {
                    match client
                        .request(Command::GetFileOperationResult {
                            request_id: request.id.clone(),
                        })
                        .await?
                    {
                        Payload::FileOperationResult {
                            result: Some(result),
                        } => return result,
                        Payload::FileOperationResult { result: None } => (),
                        _ => return Err("ipc_invalid_response".into()),
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
    pub async fn quarantine_detail(&self, id: String) -> Result<details::Detail> {
        match self {
            Self::Local(core) => {
                let core = core.clone();
                tokio::task::spawn_blocking(move || details::get(&core, &id))
                    .await
                    .map_err(clamav_desktop_core::store::err)?
            }
            Self::Worker { client, .. } => {
                match client.request(Command::GetQuarantineDetail { id }).await? {
                    Payload::QuarantineDetail(detail) => Ok(*detail),
                    _ => Err("ipc_invalid_response".into()),
                }
            }
        }
    }
    pub async fn verify_integrity(&self, id: String) -> Result<details::Detail> {
        match self {
            Self::Local(core) => {
                let core = core.clone();
                tokio::task::spawn_blocking(move || {
                    details::verify(&core, &id)?;
                    details::get(&core, &id)
                })
                .await
                .map_err(clamav_desktop_core::store::err)?
            }
            Self::Worker { client, .. } => {
                let request = Request::new(Command::VerifyQuarantineIntegrity { id: id.clone() });
                match client.send(&request).await?.into_result()? {
                    Payload::Ack => (),
                    _ => return Err("ipc_invalid_response".into()),
                }
                loop {
                    match client
                        .request(Command::GetIntegrityResult {
                            request_id: request.id.clone(),
                        })
                        .await?
                    {
                        Payload::IntegrityResult {
                            result: Some(result),
                        } => {
                            result?;
                            match self.quarantine_detail(id.clone()).await {
                                Err(reason) if reason == "busy" => (),
                                result => return result,
                            }
                        }
                        Payload::IntegrityResult { result: None } => (),
                        _ => return Err("ipc_invalid_response".into()),
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
    pub async fn stop_integrity(&self) -> Result<()> {
        match self {
            Self::Local(core) => {
                details::stop(core);
                Ok(())
            }
            Self::Worker { client, .. } => match client.request(Command::StopIntegrityCheck).await?
            {
                Payload::Ack => Ok(()),
                _ => Err("ipc_invalid_response".into()),
            },
        }
    }
    pub async fn save_settings(&self, settings: Settings) -> Result<()> {
        match self {
            Self::Local(core) => clamav_desktop_core::settings::save(core, settings),
            Self::Worker { client, .. } => {
                match client.request(Command::SaveSettings { settings }).await? {
                    Payload::Ack => Ok(()),
                    _ => Err("ipc_invalid_response".into()),
                }
            }
        }
    }
    /// No fallback Core is created if the existing owner's IPC cannot be authenticated.
    pub async fn open(root: PathBuf, notifier: Arc<dyn StateNotifier>) -> Result<Self> {
        match Core::with_notifier(root.clone(), notifier) {
            Ok(core) => Ok(Self::Local(Arc::new(core))),
            Err(reason) if reason == "data_in_use" => {
                let client = Client::new(root).map_err(|e| format!("data_in_use: {e}"))?;
                match client
                    .request(Command::GetSnapshot)
                    .await
                    .map_err(|e| format!("data_in_use: {e}"))?
                {
                    Payload::Snapshot(snapshot) => Ok(Self::Worker {
                        client,
                        snapshot: Mutex::new(*snapshot),
                    }),
                    _ => Err("ipc_invalid_response".into()),
                }
            }
            Err(reason) => Err(reason),
        }
    }
    pub async fn snapshot(&self) -> Result<Snapshot> {
        match self {
            Self::Local(core) => Ok(core.snapshot()),
            Self::Worker { client, snapshot } => {
                match client.request(Command::GetSnapshot).await? {
                    Payload::Snapshot(next) => {
                        // Native panel arrangement is Renderer state shared by all GUI windows.
                        let mut cached = snapshot.lock().unwrap();
                        let mut next = *next;
                        next.activity_floating = cached.activity_floating;
                        next.activity_dock = cached.activity_dock.clone();
                        *cached = next.clone();
                        Ok(next)
                    }
                    _ => Err("ipc_invalid_response".into()),
                }
            }
        }
    }
    pub fn set_panel(&self, floating: bool, side: Option<String>) -> Result<()> {
        if side.as_ref().is_some_and(|s| s != "left" && s != "right") {
            return Err("invalid_dock".into());
        }
        let mut snapshot = match self {
            Self::Local(core) => core.document.lock().unwrap(),
            Self::Worker { snapshot, .. } => snapshot.lock().unwrap(),
        };
        snapshot.activity_floating = floating;
        if let Some(side) = side {
            snapshot.activity_dock = side;
        }
        Ok(())
    }
    pub fn publish_cached(&self, app: &AppHandle) {
        let snapshot = match self {
            Self::Local(core) => core.snapshot(),
            Self::Worker { snapshot, .. } => snapshot.lock().unwrap().clone(),
        };
        let _ = app.emit("core:state", snapshot);
    }
}
pub fn backend(app: &AppHandle) -> Result<Arc<Backend>> {
    app.state::<crate::runtime::Runtime>().backend()
}
pub fn core(app: &AppHandle) -> Result<Arc<Core>> {
    match backend(app)?.as_ref() {
        Backend::Local(core) => Ok(core.clone()),
        Backend::Worker { .. } => Err("worker_operation_unavailable".into()),
    }
}
pub async fn poll_worker(app: AppHandle) {
    let mut disconnected = false;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let runtime = app.state::<crate::runtime::Runtime>();
        let _ = app.emit("core:backend", runtime.status());
        let Ok(backend) = runtime.backend() else {
            continue;
        };
        if matches!(backend.as_ref(), Backend::Local(_)) {
            // Local Core already emits changes. Avoid cloning/sending all history
            // every second merely to observe a future ownership handoff.
            if disconnected {
                backend.publish_cached(&app);
                let _ = app.emit("core:connection", true);
                disconnected = false;
            }
            continue;
        }
        match backend.snapshot().await {
            Ok(snapshot) => {
                let _ = app.emit("core:state", snapshot);
                if disconnected {
                    let _ = app.emit("core:connection", true);
                }
                disconnected = false;
            }
            Err(_) => {
                if !disconnected {
                    let _ = app.emit("core:connection", false);
                }
                disconnected = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clamav_desktop_core::ipc::Server;
    struct Silent;
    impl StateNotifier for Silent {
        fn publish(&self, _: Snapshot) {}
    }
    #[tokio::test]
    async fn gui_proxy_never_owns_db_or_stops_worker_on_close() {
        let root =
            PathBuf::from("/private/tmp").join(format!("clamav-proxy-{}", uuid::Uuid::new_v4()));
        let owner = Arc::new(Core::new(root.clone()).unwrap());
        let server = Server::bind(owner.clone()).unwrap();
        let task = tokio::spawn(server.run(std::future::pending()));
        let proxy = Backend::open(root.clone(), Arc::new(Silent)).await.unwrap();
        assert!(matches!(&proxy, Backend::Worker { .. }));
        proxy
            .save_settings(Settings {
                language: "en".into(),
                ..Settings::default()
            })
            .await
            .unwrap();
        assert_eq!(owner.snapshot().settings.language, "en");
        assert_eq!(proxy.snapshot().await.unwrap().settings.language, "en");
        let id = "proxy-integrity".to_string();
        let stored = root.join("quarantine").join(&id);
        std::fs::write(&stored, b"harmless proxy fixture").unwrap();
        owner.document.lock().unwrap().quarantine.push(
            clamav_desktop_core::model::QuarantineItem {
                id: id.clone(),
                original_path: root.join("original").to_string_lossy().into(),
                signature: "fixture".into(),
                digest: clamav_desktop_core::scanner::digest(&stored).unwrap(),
                quarantined_at: clamav_desktop_core::store::now(),
                mode: 0o600,
                status: "active".into(),
            },
        );
        assert_eq!(
            proxy
                .quarantine_detail(id.clone())
                .await
                .unwrap()
                .stored
                .state,
            "present"
        );
        assert_eq!(
            proxy
                .verify_integrity(id.clone())
                .await
                .unwrap()
                .integrity
                .unwrap()
                .state,
            "verified"
        );
        owner.db.lock().unwrap().execute_batch("CREATE TRIGGER integrity_write_failure BEFORE INSERT ON runtime_kv WHEN NEW.key LIKE 'integrity:%' BEGIN SELECT RAISE(FAIL, 'fixture_write_failure'); END;").unwrap();
        assert!(proxy
            .verify_integrity(id)
            .await
            .err()
            .unwrap()
            .contains("fixture_write_failure"));
        owner
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER integrity_write_failure")
            .unwrap();
        assert_eq!(std::fs::read(&stored).unwrap(), b"harmless proxy fixture");
        assert_eq!(
            proxy
                .file_operation(clamav_desktop_core::quarantine::FileAction::Restore(
                    "missing".into()
                ))
                .await
                .unwrap_err(),
            "quarantine_missing"
        );
        proxy
            .file_operation(clamav_desktop_core::quarantine::FileAction::Recover)
            .await
            .unwrap();

        proxy.set_panel(true, Some("left".into())).unwrap();
        assert!(proxy.snapshot().await.unwrap().activity_floating);
        assert!(proxy.refresh_engine().await.unwrap().checked);
        assert!(owner.snapshot().engine.checked);
        assert_eq!(proxy.download_cache().await.unwrap().attempts, 0);
        proxy
            .file_operation(clamav_desktop_core::quarantine::FileAction::CleanupCache)
            .await
            .unwrap();
        let report_path = root.with_extension("diagnostics.json");
        proxy
            .file_operation(clamav_desktop_core::quarantine::FileAction::Export {
                kind: clamav_desktop_core::exports::Kind::Diagnostics,
                destination: report_path.to_string_lossy().into(),
                app_update: Some(serde_json::json!({"phase":"idle"})),
            })
            .await
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
        assert_eq!(report["appUpdate"]["phase"], "idle");
        std::fs::remove_file(report_path).unwrap();
        assert!(!owner.snapshot().activity_floating);
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(proxy);
        let client = Client::new(root.clone()).unwrap();
        assert!(matches!(
            client.request(Command::GetSnapshot).await.unwrap(),
            Payload::Snapshot(_)
        ));
        client.request(Command::Shutdown).await.unwrap();
        task.await.unwrap().unwrap();
        drop(owner);
        let local = Backend::open(root.clone(), Arc::new(Silent)).await.unwrap();
        assert!(matches!(&local, Backend::Local(_)));
        assert_eq!(local.snapshot().await.unwrap().settings.language, "en");
        local
            .save_settings(Settings {
                language: "ja".into(),
                ..Settings::default()
            })
            .await
            .unwrap();
        assert_eq!(local.snapshot().await.unwrap().settings.language, "ja");
        drop(local);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn unreachable_owner_does_not_trigger_local_fallback() {
        let root =
            PathBuf::from("/private/tmp").join(format!("clamav-noipc-{}", uuid::Uuid::new_v4()));
        let owner = Core::new(root.clone()).unwrap();
        assert!(Backend::open(root.clone(), Arc::new(Silent))
            .await
            .err()
            .unwrap()
            .starts_with("data_in_use:"));
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(owner);
        std::fs::remove_dir_all(root).unwrap();
    }
}
