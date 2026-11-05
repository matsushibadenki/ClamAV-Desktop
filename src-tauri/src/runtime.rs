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
        if self.login_registration_present() {
            return Err("login_service_disable_required".into());
        }
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
    pub(crate) fn login_registration_present(&self) -> bool {
        if self
            .root
            .file_name()
            .is_none_or(|name| name != "app.clamav.desktop")
        {
            return false;
        }
        let Some(library) = self.root.parent().and_then(std::path::Path::parent) else {
            return false;
        };
        let path = library.join("LaunchAgents/app.clamav.desktop.worker.plist");
        !matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
    }
    pub(crate) fn owns_root(&self, root: &std::path::Path) -> bool {
        self.root == root
    }
    pub async fn set_login_service(&self, enable: bool) -> Result<Status> {
        use clamav_desktop_core::login_service::Operation;
        self.configure_login(if enable {
            Operation::Enable
        } else {
            Operation::Disable
        })
        .await
    }
    pub async fn recover_login_service(&self) -> Result<Status> {
        self.configure_login(clamav_desktop_core::login_service::Operation::Recover)
            .await
    }
    pub async fn repair_login_service(&self) -> Result<Status> {
        self.configure_login(clamav_desktop_core::login_service::Operation::Repair)
            .await
    }
    async fn configure_login(
        &self,
        operation: clamav_desktop_core::login_service::Operation,
    ) -> Result<Status> {
        use clamav_desktop_core::login_service::{
            current_user_preflight, manage_current, Operation, Registration,
        };
        let plan = current_user_preflight()?;
        if plan.data_directory != self.root {
            return Err("login_service_data_mismatch".into());
        }
        match operation {
            Operation::Recover if plan.registration != Registration::Matches => {
                return Err("login_service_registration_conflict".into())
            }
            Operation::Repair if plan.registration != Registration::Repairable => {
                return Err("login_service_registration_conflict".into())
            }
            Operation::Enable if plan.registration == Registration::Matches => {
                return Err("login_service_disable_required".into())
            }
            Operation::Enable if plan.registration != Registration::Absent => {
                return Err("login_service_registration_conflict".into())
            }
            Operation::Disable if plan.registration != Registration::Matches => {
                return Err("login_service_registration_conflict".into())
            }
            _ => (),
        }
        if operation == Operation::Recover {
            self.handoff_recovery(|| manage_current(operation)).await
        } else if operation == Operation::Repair {
            self.handoff_login_with_repair(true, true, || manage_current(operation))
                .await
        } else {
            self.handoff_login(operation != Operation::Disable, || {
                manage_current(operation)
            })
            .await
        }
    }
    async fn handoff_login<F, Fut>(&self, enable: bool, manage: F) -> Result<Status>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        self.handoff_login_with_repair(enable, false, manage).await
    }
    async fn handoff_login_with_repair<F, Fut>(
        &self,
        enable: bool,
        repair: bool,
        manage: F,
    ) -> Result<Status>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let _transition = self.transition.try_lock().map_err(|_| "busy")?;
        let client = Client::new(self.root.clone())?;
        let current = self.take_idle()?;
        let snapshot = match current.snapshot().await {
            Ok(snapshot) if !active(&snapshot) => snapshot,
            Ok(_) => {
                self.put(current);
                return Err("busy".into());
            }
            Err(reason) => {
                if !enable || repair {
                    if let Backend::Worker { snapshot, .. } = &current {
                        snapshot.lock().unwrap().clone()
                    } else {
                        self.put(current);
                        return Err(reason);
                    }
                } else {
                    self.put(current);
                    return Err(reason);
                }
            }
        };
        if enable && !repair && matches!(current, Backend::Worker { .. }) {
            // A manual worker must exit before launchd can become its owner.
            self.stop_worker(current, &client, &snapshot).await?;
            let local = self.take_idle()?;
            drop(local);
        } else {
            drop(current);
        }
        let result = manage().await;
        self.finish_login(enable, &client, snapshot, result).await
    }
    async fn handoff_recovery<F, Fut>(&self, manage: F) -> Result<Status>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let _transition = self.transition.try_lock().map_err(|_| "busy")?;
        let client = Client::new(self.root.clone())?;
        let current = self.take_idle()?;
        let snapshot = match current.snapshot().await {
            Ok(snapshot) => {
                if matches!(current, Backend::Worker { .. }) {
                    // Preserve a connected worker and its current jobs; adopting
                    // fresh state is enough. Never restart it just to reconnect.
                    self.put(self.remote(snapshot));
                    return Ok(Status {
                        mode: "worker",
                        transitioning: false,
                    });
                }
                if active(&snapshot) {
                    self.put(current);
                    return Err("busy".into());
                }
                snapshot
            }
            Err(reason) => match &current {
                Backend::Worker { snapshot, .. } => snapshot.lock().unwrap().clone(),
                _ => {
                    self.put(current);
                    return Err(reason);
                }
            },
        };
        drop(current);
        let result = manage().await;
        self.finish_login(true, &client, snapshot, result).await
    }
    async fn finish_login(
        &self,
        enable: bool,
        client: &Client,
        snapshot: Snapshot,
        result: Result<()>,
    ) -> Result<Status> {
        let next = if result.is_ok() && enable {
            match client.request(Command::GetSnapshot).await {
                Ok(Payload::Snapshot(next)) => Ok(self.remote(*next)),
                _ => Err("login_service_worker_start_timeout".into()),
            }
        } else {
            self.local()
        };
        let recovery_error = next.as_ref().err().cloned();
        self.put(next.unwrap_or_else(|_| self.remote(snapshot.clone())));
        if let Ok(backend) = self.backend() {
            let _ = backend.set_panel(snapshot.activity_floating, Some(snapshot.activity_dock));
        }
        result?;
        if let Some(reason) = recovery_error {
            return Err(reason);
        }
        Ok(Status {
            mode: if enable { "worker" } else { "local" },
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
    #[tokio::test]
    async fn login_handoff_releases_local_core_and_restores_on_management_failure() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime
            .backend()
            .unwrap()
            .save_settings(clamav_desktop_core::model::Settings {
                language: "zh-CN".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        runtime
            .backend()
            .unwrap()
            .set_panel(true, Some("left".into()))
            .unwrap();
        let error = runtime
            .handoff_login(true, || async {
                assert!(Core::new(root.clone()).is_ok());
                assert!(runtime.status().transitioning);
                assert!(runtime.backend().is_err());
                Err("login_service_bootstrap_failed".into())
            })
            .await
            .err()
            .unwrap();
        assert_eq!(error, "login_service_bootstrap_failed");
        assert_eq!(runtime.status().mode, "local");
        assert!(!runtime.status().transitioning);
        let snapshot = runtime.backend().unwrap().snapshot().await.unwrap();
        assert_eq!(snapshot.settings.language, "zh-CN");
        assert!(snapshot.activity_floating);
        assert_eq!(snapshot.activity_dock, "left");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn login_handoff_rejects_leases_and_jobs_before_calling_os_adapter() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        let lease = runtime.backend().unwrap();
        assert_eq!(
            runtime
                .handoff_login(true, || async { panic!("must not manage service") })
                .await
                .err()
                .unwrap(),
            "busy"
        );
        if let Backend::Local(core) = lease.as_ref() {
            core.document.lock().unwrap().operation = Some(Default::default());
        }
        drop(lease);
        assert_eq!(
            runtime
                .handoff_login(false, || async { panic!("must not manage service") })
                .await
                .err()
                .unwrap(),
            "busy"
        );
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn login_handoff_adopts_real_worker_and_returns_to_gui_after_removal() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime
            .backend()
            .unwrap()
            .set_panel(true, Some("left".into()))
            .unwrap();
        let mut child = None;
        runtime
            .handoff_login(true, || async {
                let mut process = tokio::process::Command::new(executable());
                process
                    .args(["--worker", "serve", "--data-dir"])
                    .arg(&root)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                child = Some(process.spawn().unwrap());
                let client = Client::new(root.clone()).unwrap();
                tokio::time::timeout(Duration::from_secs(15), async {
                    loop {
                        if let Ok(Payload::Snapshot(snapshot)) =
                            client.request(Command::GetSnapshot).await
                        {
                            if !active(&snapshot) {
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .unwrap();
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(runtime.status().mode, "worker");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        assert_eq!(
            runtime
                .handoff_login(false, || async {
                    Err("login_service_bootout_failed".into())
                })
                .await
                .err()
                .unwrap(),
            "login_service_bootout_failed"
        );
        assert_eq!(runtime.status().mode, "worker");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        runtime
            .handoff_login(false, || async {
                assert!(matches!(
                    Client::new(root.clone())
                        .unwrap()
                        .request(Command::ShutdownIfIdle)
                        .await?,
                    Payload::Ack
                ));
                child
                    .as_mut()
                    .unwrap()
                    .wait()
                    .await
                    .map_err(clamav_desktop_core::store::err)?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(runtime.status().mode, "local");
        let snapshot = runtime.backend().unwrap().snapshot().await.unwrap();
        assert!(snapshot.activity_floating);
        assert_eq!(snapshot.activity_dock, "left");
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn registered_service_blocks_manual_switch_even_when_gui_owns_core() {
        let fixture = root();
        let data = fixture.join("Library/Application Support/app.clamav.desktop");
        let runtime = Runtime::open(data.clone(), Arc::new(Silent)).await.unwrap();
        let agents = fixture.join("Library/LaunchAgents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("app.clamav.desktop.worker.plist"), b"conflict").unwrap();
        assert!(runtime.login_registration_present());
        assert_eq!(
            runtime
                .switch(false, executable(), true)
                .await
                .err()
                .unwrap(),
            "login_service_disable_required"
        );
        assert_eq!(runtime.status().mode, "local");
        assert_eq!(Core::new(data).err().unwrap(), "data_in_use");
        drop(runtime);
        std::fs::remove_dir_all(fixture).unwrap();
    }
    #[tokio::test]
    async fn enabling_login_from_manual_worker_stops_it_before_management_and_recovers() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime.switch(true, executable(), true).await.unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if !active(&runtime.backend().unwrap().snapshot().await.unwrap()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime
                .handoff_login(true, || async {
                    assert!(!root.join("worker.sock").exists());
                    assert!(Core::new(root.clone()).is_ok());
                    Err("login_service_bootstrap_failed".into())
                })
                .await
                .err()
                .unwrap(),
            "login_service_bootstrap_failed"
        );
        assert_eq!(runtime.status().mode, "local");
        assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn repairing_disconnected_worker_recovers_only_after_exclusive_ownership() {
        for live in [true, false] {
            let root = root();
            let owner = Core::new(root.clone()).unwrap();
            let snapshot = owner.snapshot();
            let mut owner = Some(owner);
            if !live {
                owner.take();
            }
            let runtime = Runtime {
                state: Mutex::new(Some(Arc::new(Backend::Worker {
                    client: Client::new(root.clone()).unwrap(),
                    snapshot: Mutex::new(snapshot),
                }))),
                transition: tokio::sync::Mutex::new(()),
                root: root.clone(),
                notifier: Arc::new(Silent),
            };
            let result = runtime
                .handoff_login_with_repair(true, true, || async {
                    assert_eq!(Core::new(root.clone()).is_err(), live);
                    Err("login_service_bootout_failed".into())
                })
                .await;
            assert_eq!(result.err().unwrap(), "login_service_bootout_failed");
            assert_eq!(runtime.status().mode, if live { "worker" } else { "local" });
            assert_eq!(Core::new(root.clone()).err().unwrap(), "data_in_use");
            drop(owner);
            drop(runtime);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    async fn recovery_adopts_busy_live_worker_without_calling_restart() {
        use clamav_desktop_core::ipc::Server;
        let root = root();
        let core = Arc::new(Core::new(root.clone()).unwrap());
        core.document.lock().unwrap().operation = Some(Default::default());
        let server = Server::bind(core.clone()).unwrap();
        let task = tokio::spawn(server.run(std::future::pending()));
        let runtime = Runtime {
            state: Mutex::new(Some(Arc::new(Backend::Worker {
                client: Client::new(root.clone()).unwrap(),
                snapshot: Mutex::new(core.snapshot()),
            }))),
            transition: tokio::sync::Mutex::new(()),
            root: root.clone(),
            notifier: Arc::new(Silent),
        };
        runtime
            .handoff_recovery(|| async { panic!("healthy worker must not restart") })
            .await
            .unwrap();
        assert_eq!(runtime.status().mode, "worker");
        assert!(runtime
            .backend()
            .unwrap()
            .snapshot()
            .await
            .unwrap()
            .operation
            .is_some());
        core.document.lock().unwrap().operation = None;
        Client::new(root.clone())
            .unwrap()
            .request(Command::ShutdownIfIdle)
            .await
            .unwrap();
        task.await.unwrap().unwrap();
        drop(runtime);
        drop(core);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn failed_recovery_restores_local_owner_and_saved_settings() {
        let root = root();
        let runtime = Runtime::open(root.clone(), Arc::new(Silent)).await.unwrap();
        runtime
            .backend()
            .unwrap()
            .set_panel(true, Some("left".into()))
            .unwrap();
        assert_eq!(
            runtime
                .handoff_recovery(|| async {
                    assert!(Core::new(root.clone()).is_ok());
                    Err("login_service_approval_required".into())
                })
                .await
                .err()
                .unwrap(),
            "login_service_approval_required"
        );
        assert_eq!(runtime.status().mode, "local");
        assert!(
            runtime
                .backend()
                .unwrap()
                .snapshot()
                .await
                .unwrap()
                .activity_floating
        );
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }
}
