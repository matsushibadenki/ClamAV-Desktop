use crate::{model::*, process, store::*};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;
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
pub async fn inspect(app: &AppHandle) -> Result<Engine> {
    let c = core(app);
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
    c.publish(app);
    Ok(engine)
}
pub async fn update(app: AppHandle) -> Result<()> {
    let c = core(&app);
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() || d.scan.as_ref().is_some_and(|s| s.status == "running") {
            return Err("busy".into());
        }
        d.operation = Some(Operation {
            kind: "definitions".into(),
            stage: "updating".into(),
            ..Default::default()
        });
    }
    c.publish(&app);
    let result = update_inner(&app).await;
    {
        let mut d = c.document.lock().unwrap();
        d.operation = None;
    }
    let _ = inspect(&app).await;
    match &result {
        Ok(_) => c.log(&app, "definitions_updated", ""),
        Err(e) => c.log(&app, "error", e),
    }
    result
}
async fn update_inner(app: &AppHandle) -> Result<()> {
    let c = core(app);
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
        None,
        Duration::from_secs(1800),
        |s| {
            tail = s.chars().take(2048).collect();
            if last.elapsed() > Duration::from_millis(300) {
                last = std::time::Instant::now();
                if let Some(op) = &mut c.document.lock().unwrap().operation {
                    op.message = Some(tail.clone())
                }
                c.publish(app);
            }
        },
    )
    .await?;
    if result.code != Some(0) {
        return Err(format!("freshclam_failed: {tail}"));
    }
    if inspect(app).await?.database_path.as_deref() != Some(db.to_string_lossy().as_ref()) {
        return Err("definitions_missing".into());
    }
    Ok(())
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
