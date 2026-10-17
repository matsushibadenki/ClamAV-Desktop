//! Live ownership handoff. A lease prevents backend/Core teardown during any
//! command; process ownership remains the final authority before opening SQLite.
use crate::bridge::Backend;
use clamav_desktop_core::{
    ipc::{Client, Command, Payload},
    model::Snapshot,
    store::{Core, Result, StateNotifier},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub mode: &'static str,
    pub transitioning: bool,
}

pub struct Runtime {
    state: Mutex<Option<Arc<Backend>>>,
    transition: tokio::sync::Mutex<()>,
    root: PathBuf,
    notifier: Arc<dyn StateNotifier>,
}

impl Runtime {
    pub async fn open(root: PathBuf, notifier: Arc<dyn StateNotifier>) -> Result<Self> {
        let backend = Backend::open(root.clone(), notifier.clone()).await?;
        Ok(Self {
            state: Mutex::new(Some(Arc::new(backend))),
            transition: tokio::sync::Mutex::new(()),
            root,
            notifier,
        })
    }
    pub fn backend(&self) -> Result<Arc<Backend>> {
        self.state
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "busy".into())
    }
    pub fn status(&self) -> Status {
        let state = self.state.lock().unwrap();
        Status {
            mode: match state.as_deref() {
                Some(Backend::Local(_)) => "local",
                Some(Backend::Worker { .. }) => "worker",
                None => "switching",
            },
            transitioning: self.transition.try_lock().is_err(),
        }
    }
    fn take_idle(&self) -> Result<Backend> {
        let mut state = self.state.lock().unwrap();
        let backend = state.as_ref().ok_or("busy")?;
        if Arc::strong_count(backend) != 1 {
            return Err("busy".into());
        }
        if let Backend::Local(core) = backend.as_ref() {
            if core.snapshot().installer_ready {
                return Err("worker_prepared_installer".into());
            }
            // Includes detached scan tasks, installer, updater and maintenance.
            if Arc::strong_count(core) != 1 || active(&core.snapshot()) {
                return Err("busy".into());
            }
        }
        Arc::try_unwrap(state.take().unwrap()).map_err(|_| "busy".into())
    }
    fn put(&self, backend: Backend) {
        *self.state.lock().unwrap() = Some(Arc::new(backend));
    }
    fn local(&self) -> Result<Backend> {
        Core::with_notifier(self.root.clone(), self.notifier.clone())
            .map(|core| Backend::Local(Arc::new(core)))
    }
    fn remote(&self, snapshot: Snapshot) -> Backend {
        Backend::Worker {
            client: Client::new(self.root.clone()).expect("validated IPC path"),
            snapshot: Mutex::new(snapshot),
        }
    }
    /// Call in an owned task so closing a command receiver cannot cancel handoff.
    pub async fn switch(
        &self,
        worker: bool,
        executable: PathBuf,
        worker_arg: bool,
    ) -> Result<Status> {
        let _transition = self.transition.try_lock().map_err(|_| "busy")?;
        // Validate before releasing the local owner, including the socket path limit.
        let client = Client::new(self.root.clone())?;
        let current = self.take_idle()?;
        let already = matches!(&current, Backend::Worker { .. }) == worker;
        if already {
            self.put(current);
            return Ok(Status {
                mode: if worker { "worker" } else { "local" },
                transitioning: false,
            });
        }
        let snapshot = match current.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                if !worker {
                    if let Ok(local) = self.local() {
                        if let Backend::Worker { snapshot, .. } = &current {
                            let cached = snapshot.lock().unwrap();
                            let _ = local.set_panel(
                                cached.activity_floating,
                                Some(cached.activity_dock.clone()),
                            );
                        }
                        self.put(local);
                        return Ok(Status {
                            mode: "local",
                            transitioning: false,
                        });
                    }
                }
                self.put(current);
                return Err(reason);
            }
        };
        if active(&snapshot) {
            self.put(current);
            return Err("busy".into());
        }
        let result = if worker {
            drop(current); // DB/Document drop before releasing the OS ownership lock.
            self.start_worker(executable, worker_arg, &client, &snapshot)
                .await
        } else {
            self.stop_worker(current, &client, &snapshot).await
        };
        // Restore native panel arrangement in the newly selected backend.
        if let Ok(backend) = self.backend() {
            let _ = backend.set_panel(snapshot.activity_floating, Some(snapshot.activity_dock));
        }
        result?;
        Ok(Status {
            mode: if worker { "worker" } else { "local" },
            transitioning: false,
        })
    }
    async fn start_worker(
        &self,
        executable: PathBuf,
        worker_arg: bool,
        client: &Client,
        snapshot: &Snapshot,
    ) -> Result<()> {
        let mut command = tokio::process::Command::new(executable);
        if worker_arg {
            command.arg("--worker");
        }
        command
            .args(["serve", "--data-dir"])
            .arg(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The worker survives GUI termination and terminal process-group signals.
        command.process_group(0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(reason) => {
                self.put(
                    self.local()
                        .unwrap_or_else(|_| self.remote(snapshot.clone())),
                );
                return Err(format!("worker_start_failed: {reason}"));
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let result = loop {
            let exited = match child.try_wait() {
                Ok(status) => status.is_some(),
                Err(reason) => {
                    self.put(self.remote(snapshot.clone()));
                    break Err(format!("worker_start_failed: {reason}"));
                }
            };
            if exited {
                self.put(
                    self.local()
                        .unwrap_or_else(|_| self.remote(snapshot.clone())),
                );
                break Err("worker_start_failed".into());
            }
            if let Ok(Ok(Payload::Snapshot(next))) = tokio::time::timeout(
                Duration::from_millis(500),
                client.request(Command::GetSnapshot),
            )
            .await
            {
                self.put(self.remote(*next));
                break Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                // A live but unreachable child may still own DB/files. Keep the
                // proxy for reconnection, never kill it or open a second Core.
                self.put(self.remote(snapshot.clone()));
                break Err("worker_start_timeout".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        result
    }
    async fn stop_worker(
        &self,
        current: Backend,
        client: &Client,
        snapshot: &Snapshot,
    ) -> Result<()> {
        match client.request(Command::ShutdownIfIdle).await {
            Ok(Payload::Ack) => (),
            Ok(_) => {
                self.put(current);
                return Err("ipc_invalid_response".into());
            }
            Err(reason) => {
                // Explicit return-to-GUI may recover a crashed worker, but only
                // after the OS lock grants exclusive ownership. Live owners win.
                if let Ok(local) = self.local() {
                    self.put(local);
                    return Ok(());
                }
                self.put(current);
                return Err(reason);
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            match self.local() {
                Ok(local) => {
                    self.put(local);
                    return Ok(());
                }
                Err(reason)
                    if reason == "data_in_use" && tokio::time::Instant::now() < deadline =>
                {
                    ()
                }
                Err(reason) => {
                    self.put(self.remote(snapshot.clone()));
                    return Err(format!("worker_stop_failed: {reason}"));
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
fn active(snapshot: &Snapshot) -> bool {
    snapshot.operation.is_some()
        || snapshot
            .scan
            .as_ref()
            .is_some_and(|scan| scan.status == "running")
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Silent;
    impl StateNotifier for Silent {
        fn publish(&self, _: Snapshot) {}
    }
    fn root() -> PathBuf {
        PathBuf::from("/private/tmp").join(format!("clamav-handoff-{}", uuid::Uuid::new_v4()))
    }
    fn executable() -> PathBuf {
        // `cargo build --workspace --bins` builds both production entry points.
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("clamav-desktop");
        assert!(
            path.is_file(),
            "build workspace binaries before the handoff tests"
        );
        path
    }
    #[tokio::test]
    async fn live_round_trip_retains_settings_layout_and_exclusive_ownership() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        let settings = clamav_desktop_core::model::Settings {
            language: "zh-CN".into(),
            ..Default::default()
        };
        runtime
            .backend()
            .unwrap()
            .save_settings(settings)
            .await
            .unwrap();
        runtime
            .backend()
            .unwrap()
            .set_panel(true, Some("left".into()))
            .unwrap();
        assert_eq!(
            runtime.switch(true, executable(), true).await.unwrap().mode,
            "worker"
        );
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        let snapshot = runtime.backend().unwrap().snapshot().await.unwrap();
        assert_eq!(snapshot.settings.language, "zh-CN");
        assert!(snapshot.activity_floating);
        assert_eq!(snapshot.activity_dock, "left");
        // Shutdown waits for startup probing and releases the lock before local creation.
        assert_eq!(
            runtime
                .switch(false, executable(), true)
                .await
                .unwrap()
                .mode,
            "local"
        );
        assert!(!root.join("worker.sock").exists());
        let snapshot = runtime.backend().unwrap().snapshot().await.unwrap();
        assert_eq!(snapshot.settings.language, "zh-CN");
        assert!(snapshot.activity_floating);
        assert_eq!(snapshot.activity_dock, "left");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(runtime);
        drop(Core::new(root.clone()).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn leases_jobs_and_prepared_installers_block_handoff_without_releasing_owner() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        let lease = runtime.backend().unwrap();
        assert_eq!(
            runtime
                .switch(true, PathBuf::new(), true)
                .await
                .err()
                .unwrap(),
            "busy"
        );
        let core = match lease.as_ref() {
            Backend::Local(core) => core.clone(),
            _ => unreachable!(),
        };
        drop(lease);
        assert_eq!(
            runtime
                .switch(true, PathBuf::new(), true)
                .await
                .err()
                .unwrap(),
            "busy"
        );
        core.document.lock().unwrap().operation = Some(Default::default());
        drop(core);
        assert_eq!(
            runtime
                .switch(true, PathBuf::new(), true)
                .await
                .err()
                .unwrap(),
            "busy"
        );
        if let Backend::Local(core) = runtime.backend().unwrap().as_ref() {
            let mut document = core.document.lock().unwrap();
            document.operation = None;
            document.installer_ready = true;
        }
        assert_eq!(
            runtime
                .switch(true, PathBuf::new(), true)
                .await
                .err()
                .unwrap(),
            "worker_prepared_installer"
        );
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn spawn_and_early_exit_failures_restore_local_settings() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime
            .backend()
            .unwrap()
            .save_settings(clamav_desktop_core::model::Settings {
                language: "en".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        for executable in [root.join("missing-worker"), PathBuf::from("/usr/bin/false")] {
            assert!(runtime
                .switch(true, executable, true)
                .await
                .err()
                .unwrap()
                .starts_with("worker_start_failed"));
            assert_eq!(runtime.status().mode, "local");
            assert!(!runtime.status().transitioning);
            assert_eq!(
                runtime
                    .backend()
                    .unwrap()
                    .snapshot()
                    .await
                    .unwrap()
                    .settings
                    .language,
                "en"
            );
            assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        }
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn disconnected_owner_is_not_replaced_until_lock_is_released() {
        let root = root();
        let owner = Core::new(root.clone()).unwrap();
        let runtime = Runtime {
            state: Mutex::new(Some(Arc::new(Backend::Worker {
                client: Client::new(root.clone()).unwrap(),
                snapshot: Mutex::new(owner.snapshot()),
            }))),
            transition: tokio::sync::Mutex::new(()),
            root: root.clone(),
            notifier: Arc::new(Silent),
        };
        assert!(runtime.switch(false, PathBuf::new(), true).await.is_err());
        assert_eq!(runtime.status().mode, "worker");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(owner);
        assert_eq!(
            runtime
                .switch(false, PathBuf::new(), true)
                .await
                .unwrap()
                .mode,
            "local"
        );
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn closing_gui_leaves_worker_running_and_next_gui_can_return_to_local() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime.switch(true, executable(), true).await.unwrap();
        drop(runtime);
        assert!(matches!(
            Client::new(root.clone())
                .unwrap()
                .request(Command::GetSnapshot)
                .await
                .unwrap(),
            Payload::Snapshot(_)
        ));
        let reopened = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        assert_eq!(reopened.status().mode, "worker");
        reopened.switch(false, executable(), true).await.unwrap();
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
