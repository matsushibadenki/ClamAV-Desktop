//! Application updates stay in Rust. The WebView receives metadata/progress only.
use crate::{
    bridge::core,
    store::{err, now, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub current_version: String,
    pub phase: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub checked_at: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error_code: Option<String>,
    pub detail: Option<String>,
}
pub struct Service {
    state: Mutex<Status>,
    update: Mutex<Option<Update>>,
    gate: tokio::sync::Mutex<()>,
}
impl Service {
    pub fn new(version: String) -> Self {
        Self {
            state: Mutex::new(Status {
                current_version: version,
                phase: "idle".into(),
                version: None,
                notes: None,
                checked_at: None,
                downloaded: 0,
                total: None,
                error_code: None,
                detail: None,
            }),
            update: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
        }
    }
    pub fn snapshot(&self) -> Status {
        self.state.lock().unwrap().clone()
    }
    pub fn blocks_operations(&self) -> bool {
        matches!(
            self.snapshot().phase.as_str(),
            "downloading" | "installing" | "installed"
        )
    }
    fn publish(&self, app: &AppHandle) {
        let _ = app.emit("app:update", self.snapshot());
    }
    fn fail(&self, app: &AppHandle, reason: &str) {
        let mut status = self.state.lock().unwrap();
        status.phase = "error".into();
        status.error_code = Some(classify(reason).into());
        status.detail = Some(reason.chars().take(2048).collect());
        drop(status);
        self.publish(app);
    }
}
fn classify(reason: &str) -> &'static str {
    let s = reason.to_lowercase();
    if s.contains("too_large") {
        "size"
    } else if s.contains("signature") || s.contains("minisign") || s.contains("signed for version")
    {
        "signature"
    } else if s.contains("worker_operation_unavailable") {
        "worker"
    } else if s.contains("valid release json") {
        "feed"
    } else if s.contains("platform") {
        "platform"
    } else if s.contains("busy") {
        "busy"
    } else if s.contains("timeout") || s.contains("timed out") {
        "timeout"
    } else if s.contains("untrusted_update_url") {
        "source"
    } else {
        "network_or_install"
    }
}
fn trusted_artifact(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url
            .path()
            .starts_with("/matsushibadenki/ClamAV-Desktop/releases/download/")
        && url.path().ends_with(".app.tar.gz")
        && url.query().is_none()
        && url.fragment().is_none()
}
pub async fn check(app: AppHandle) -> Result<Status> {
    let s = app.state::<Service>();
    let _gate = s.gate.try_lock().map_err(|_| "busy")?;
    if s.blocks_operations() {
        return Err("busy".into());
    }
    {
        let mut state = s.state.lock().unwrap();
        state.phase = "checking".into();
        state.error_code = None;
        state.detail = None;
        state.version = None;
        state.notes = None;
        state.downloaded = 0;
        state.total = None;
    }
    *s.update.lock().unwrap() = None;
    s.publish(&app);
    let result: Result<Option<Update>> = async {
        let update = app
            .updater_builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(err)?
            .check()
            .await
            .map_err(err)?;
        if let Some(update) = &update {
            if !trusted_artifact(&update.download_url) {
                return Err("untrusted_update_url".into());
            }
        }
        Ok(update)
    }
    .await;
    match result {
        Ok(update) => {
            let mut status = s.state.lock().unwrap();
            status.phase = if update.is_some() {
                "available"
            } else {
                "latest"
            }
            .into();
            status.version = update.as_ref().map(|u| u.version.clone());
            status.notes = update.as_ref().and_then(|u| {
                u.body
                    .as_ref()
                    .map(|text| text.chars().take(8192).collect())
            });
            status.checked_at = Some(now());
            *s.update.lock().unwrap() = update;
            drop(status);
            s.publish(&app);
        }
        Err(reason) => s.fail(&app, &reason),
    }
    Ok(s.snapshot())
}
pub async fn install(app: AppHandle) -> Result<Status> {
    let s = app.state::<Service>();
    let _gate = s.gate.try_lock().map_err(|_| "busy")?;
    if s.snapshot().phase != "available" {
        return Err("app_update_not_ready".into());
    }
    let update = s
        .update
        .lock()
        .unwrap()
        .clone()
        .ok_or("app_update_not_ready")?;
    let owned = match core(&app) {
        Ok(core) => core,
        Err(reason) => {
            s.fail(&app, &reason);
            return Ok(s.snapshot());
        }
    };
    let handle = app.clone();
    let failure_core = owned.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        // Keep file transfers out while replacing the application bundle.
        let _file_gate = owned.file_gate.try_lock().map_err(|_| "busy")?;
        {
            let mut doc = owned.document.lock().unwrap();
            if doc.operation.is_some() || doc.scan.as_ref().is_some_and(|s| s.status == "running") {
                return Err("busy".into());
            }
            doc.operation = Some(crate::model::Operation {
                kind: "app_update".into(),
                stage: "downloading".into(),
                ..Default::default()
            });
        }
        let service = handle.state::<Service>();
        service.state.lock().unwrap().phase = "downloading".into();
        service.publish(&handle);
        owned.publish();
        let mut last = Instant::now();
        let exceeded = tokio::sync::Notify::new();
        let result: Result<()> = tauri::async_runtime::block_on(async {
            let bytes = tokio::select! {
                biased;
                _ = exceeded.notified() => return Err("app_update_too_large".into()),
                result = update.download(
                    |chunk, total| {
                        let mut status = service.state.lock().unwrap();
                        status.downloaded = status.downloaded.saturating_add(chunk as u64);
                        status.total = total;
                        if status.downloaded > 128 * 1024 * 1024 || total.is_some_and(|t| t > 128 * 1024 * 1024) { exceeded.notify_one(); }
                        drop(status);
                        if last.elapsed() >= Duration::from_millis(150) {
                            let status = service.snapshot();
                            if let Some(op) = &mut owned.document.lock().unwrap().operation { op.downloaded = status.downloaded; op.total = status.total; }
                            owned.publish(); service.publish(&handle); last = Instant::now();
                        }
                    },
                    || {
                        service.state.lock().unwrap().phase = "installing".into();
                        let status = service.snapshot();
                        if let Some(op) = &mut owned.document.lock().unwrap().operation { op.stage = "appUpdate_installing".into(); op.downloaded = status.downloaded; op.total = status.total; }
                        owned.publish(); service.publish(&handle);
                    },
                ) => result.map_err(err)?,
            };
            if bytes.len() > 128 * 1024 * 1024 { return Err("app_update_too_large".into()); }
            update.install(bytes).map_err(err)
        });
        {
            let mut doc = owned.document.lock().unwrap();
            if result.is_ok() {
                if let Some(op) = &mut doc.operation {
                    op.stage = "appUpdate_installed".into();
                }
            } else {
                doc.operation = None;
            }
        }
        owned.publish();
        result
    })
    .await;
    let result = match joined {
        Ok(result) => result,
        Err(reason) => {
            if let Ok(mut doc) = failure_core.document.lock() {
                if doc
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.kind == "app_update")
                {
                    doc.operation = None;
                }
            }
            failure_core.publish();
            Err(format!("app_update_task_failed: {reason}"))
        }
    };
    match result {
        Ok(()) => {
            s.state.lock().unwrap().phase = "installed".into();
            *s.update.lock().unwrap() = None;
            s.publish(&app);
        }
        Err(reason) => s.fail(&app, &reason),
    }
    Ok(s.snapshot())
}
pub fn restart(app: &AppHandle) -> Result<()> {
    if app.state::<Service>().snapshot().phase != "installed" {
        return Err("app_update_not_ready".into());
    }
    app.restart()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a locally signed release updater archive"]
    fn release_signature_matches_public_key() {
        use base64::Engine;
        let archive = std::env::var("CLAMAV_UPDATE_TEST_ARCHIVE").expect("archive required");
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let decode = |value: &str| {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(value.trim())
                    .unwrap(),
            )
            .unwrap()
        };
        let public = minisign_verify::PublicKey::decode(&decode(
            config["plugins"]["updater"]["pubkey"].as_str().unwrap(),
        ))
        .unwrap();
        let signature = minisign_verify::Signature::decode(&decode(
            &std::fs::read_to_string(format!("{archive}.sig")).unwrap(),
        ))
        .unwrap();
        let mut bytes = std::fs::read(archive).unwrap();
        public.verify(&bytes, &signature, true).unwrap();
        let version = signature
            .trusted_comment()
            .split('\t')
            .find_map(|v| v.strip_prefix("version:"))
            .unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        bytes[0] ^= 1;
        assert!(
            public.verify(&bytes, &signature, true).is_err(),
            "tampered archive was accepted"
        );
    }
    #[test]
    fn permits_only_repository_https_archives() {
        assert!(trusted_artifact(&reqwest::Url::parse("https://github.com/matsushibadenki/ClamAV-Desktop/releases/download/v0.2.0/test.app.tar.gz").unwrap()));
        for url in ["http://github.com/matsushibadenki/ClamAV-Desktop/releases/download/v1/test.app.tar.gz", "https://github.com/other/repo/releases/download/v1/test.app.tar.gz", "https://github.com/matsushibadenki/ClamAV-Desktop/releases/download/v1/test.dmg", "https://github.com/matsushibadenki/ClamAV-Desktop/releases/download/v1/test.app.tar.gz?token=secret"] { assert!(!trusted_artifact(&reqwest::Url::parse(url).unwrap())); }
    }
    #[test]
    fn classifies_rejected_signatures_and_unavailable_feed() {
        assert_eq!(classify("signature verification failed"), "signature");
        assert_eq!(
            classify("The update was signed for version 1 but announced version 2"),
            "signature"
        );
        assert_eq!(
            classify("Could not fetch a valid release JSON from the remote"),
            "feed"
        );
        assert_eq!(classify("worker_operation_unavailable"), "worker");
    }
}
