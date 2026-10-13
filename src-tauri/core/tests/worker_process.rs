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
