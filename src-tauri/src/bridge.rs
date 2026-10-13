//! Tauri adapter and worker proxy. Windows never own business state or SQLite.
use clamav_desktop_core::{
    ipc::{Client, Command, Payload},
    model::Snapshot,
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
pub fn backend(app: &AppHandle) -> tauri::State<'_, Backend> {
    app.state::<Backend>()
}
pub fn core(app: &AppHandle) -> Result<Arc<Core>> {
    match backend(app).inner() {
        Backend::Local(core) => Ok(core.clone()),
        Backend::Worker { .. } => Err("worker_operation_unavailable".into()),
    }
}
pub async fn poll_worker(app: AppHandle) {
    let mut disconnected = false;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        match backend(&app).snapshot().await {
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
        proxy.set_panel(true, Some("left".into())).unwrap();
        assert!(proxy.snapshot().await.unwrap().activity_floating);
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
