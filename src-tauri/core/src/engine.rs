use crate::{model::*, process, store::*};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub fn discover() -> Option<PathBuf> {
    [
        "/usr/local/clamav/bin/clamscan",
        "/opt/homebrew/bin/clamscan",
        "/usr/local/bin/clamscan",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}
pub fn freshclam(scan: &Path) -> Option<PathBuf> {
    let bin = scan.parent()?;
    [bin.join("freshclam"), bin.parent()?.join("sbin/freshclam")]
        .into_iter()
        .find(|p| p.is_file())
}
pub async fn refresh(c: &Core) -> Result<Engine> {
    begin_inspection(c)?;
    let result = inspect(c).await;
    finish_inspection(c, result)
}
pub(crate) fn begin_inspection(c: &Core) -> Result<()> {
    {
        let mut document = c.document.lock().unwrap();
        if document.operation.is_some()
            || document
                .scan
                .as_ref()
                .is_some_and(|scan| scan.status == "running")
        {
            return Err("busy".into());
        }
        document.operation = Some(Operation {
            kind: "engine_inspection".into(),
            stage: "checkingEngine".into(),
            ..Default::default()
        });
    }
    c.publish();
    Ok(())
}
pub(crate) fn finish_inspection(c: &Core, result: Result<Engine>) -> Result<Engine> {
    c.document.lock().unwrap().operation = None;
    c.publish();
    result
}
pub async fn inspect(c: &Core) -> Result<Engine> {
    let result = inspect_inner(c).await;
    if result.is_err() {
        let mut d = c.document.lock().unwrap();
        d.engine.checked = true;
        d.engine.version = None;
        d.engine.path = discover().map(|p| p.to_string_lossy().into());
    }
    crate::diagnostics::record(c, "engine_inspection", &result);
    result
}
async fn inspect_inner(c: &Core) -> Result<Engine> {
    let path = discover();
    let mut version = None;
    if let Some(p) = &path {
        let mut output = String::new();
        let result = process::run(
            p,
            &["--version".into()],
            None,
            Duration::from_secs(15),
            |s| {
                if output.len() < 2048 {
                    output.push_str(s)
                }
            },
        )
        .await?;
        if result.code == Some(0) && output.starts_with("ClamAV ") {
            version = Some(output);
        } else {
            return Err(format!(
                "engine_probe_failed: exit={:?}; {output}",
                result.code
            ));
        }
    }
    let mut definitions_at = None;
    let mut database_path = None;
    let candidates = [
        c.root.join("database"),
        PathBuf::from("/opt/homebrew/var/lib/clamav"),
        PathBuf::from("/usr/local/var/lib/clamav"),
        PathBuf::from("/usr/local/clamav/share/clamav"),
    ];
    for dir in candidates {
        let daily = ["daily.cvd", "daily.cld"]
            .iter()
            .map(|n| dir.join(n))
            .find(|p| valid_header(p));
        let main = ["main.cvd", "main.cld"]
            .iter()
            .map(|n| dir.join(n))
            .any(|p| valid_header(&p));
        if let Some(daily) = daily.filter(|_| main) {
            definitions_at = daily
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            database_path = Some(dir.to_string_lossy().to_string());
            break;
        }
    }
    let definitions = database_path.is_some();
    let engine = Engine {
        path: path.map(|p| p.to_string_lossy().into()),
        version,
        definitions,
        database_path,
        definitions_at,
        checked: true,
    };
    c.document.lock().unwrap().engine = engine.clone();
    c.publish();
    Ok(engine)
}
pub async fn update(c: &Core) -> Result<()> {
    begin_update(c)?;
    complete_update(c).await
}

pub(crate) fn begin_update(c: &Core) -> Result<()> {
    if crate::diagnostics::retry_blocked(&c, "definitions_update") {
        return Err("diagnostic_retry_wait".into());
    }
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() || d.scan.as_ref().is_some_and(|s| s.status == "running") {
            return Err("busy".into());
        }
        c.definitions_cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        d.operation = Some(Operation {
            kind: "definitions".into(),
            stage: "updating".into(),
            ..Default::default()
        });
    }
    c.publish();
    Ok(())
}

pub(crate) async fn complete_update(c: &Core) -> Result<()> {
    let result = update_inner(c).await;
    {
        let mut d = c.document.lock().unwrap();
        d.operation = None;
    }
    crate::diagnostics::record(c, "definitions_update", &result);
    let _ = inspect(c).await;
    match &result {
        Ok(_) => c.log("definitions_updated", ""),
        Err(e) => c.log("error", e),
    }
    result
}
async fn update_inner(c: &Core) -> Result<()> {
    if c.definitions_cancel
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Err("definitions_update_cancelled".into());
    }
    let scan = discover().ok_or("engine_missing")?;
    let binary = freshclam(&scan).ok_or("freshclam_missing")?;
    let db = c.root.join("database");
    let config = c.root.join("config/freshclam.conf");
    let content=format!("DatabaseDirectory {}\nDatabaseMirror database.clamav.net\nDNSDatabaseInfo current.cvd.clamav.net\nConnectTimeout 30\nReceiveTimeout 120\nChecks 4\n",db.display());
    if content.contains('\r') || db.to_string_lossy().contains('\n') {
        return Err("invalid_config_path".into());
    }
    std::fs::write(&config, content).map_err(err)?;
    let mut tail = String::new();
    let mut last = std::time::Instant::now();
    let result = process::run(
        &binary,
        &[
            format!("--config-file={}", config.display()),
            format!("--datadir={}", db.display()),
        ],
        Some(&c.definitions_cancel),
        Duration::from_secs(1800),
        |s| {
            tail.push_str(s);
            tail.push('\n');
            if tail.len() > 8192 {
                tail = tail
                    .chars()
                    .rev()
                    .take(4096)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
            }
            if last.elapsed() > Duration::from_millis(300) {
                last = std::time::Instant::now();
                if let Some(op) = &mut c.document.lock().unwrap().operation {
                    op.message = Some(tail.clone())
                }
                c.publish();
            }
        },
    )
    .await?;
    if result.cancelled {
        return Err("definitions_update_cancelled".into());
    }
    validate_update(result.code, &tail)?;
    if inspect(c).await?.database_path.as_deref() != Some(db.to_string_lossy().as_ref()) {
        return Err("definitions_missing".into());
    }
    Ok(())
}

fn validate_update(code: Option<i32>, tail: &str) -> Result<()> {
    if code != Some(0) || crate::diagnostics::classify(tail) == "rate_limit" {
        return Err(format!("freshclam_failed: exit={code:?}; {tail}"));
    }
    Ok(())
}
#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[tokio::test]
    async fn shutdown_before_launch_records_cancellation_and_releases_operation() {
        let root = PathBuf::from("/private/tmp")
            .join(format!("clamav-update-cancel-{}", uuid::Uuid::new_v4()));
        let core = Core::new(root.clone()).unwrap();
        begin_update(&core).unwrap();
        assert_eq!(begin_update(&core).unwrap_err(), "busy");
        core.definitions_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            complete_update(&core).await.unwrap_err(),
            "definitions_update_cancelled"
        );
        assert!(core.snapshot().operation.is_none());
        assert!(core
            .snapshot()
            .diagnostics
            .iter()
            .any(|a| a.kind == "definitions_update" && a.code == "cancelled"));
        drop(core);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cooldown_is_failure_even_when_freshclam_returns_zero() {
        let tail = "WARNING: FreshClam previously received error code 429
WARNING: You are on cool-down until after: 2099-01-01 12:00:00";
        assert!(validate_update(Some(0), tail)
            .unwrap_err()
            .contains("cool-down"));
        assert!(validate_update(Some(1), "network failed")
            .unwrap_err()
            .contains("network failed"));
        assert!(validate_update(Some(0), "daily database is up-to-date").is_ok());
    }
}

fn valid_header(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 11];
    file.read_exact(&mut header).is_ok()
        && &header == b"ClamAV-VDB:"
        && file.metadata().is_ok_and(|m| m.len() > 512)
}
